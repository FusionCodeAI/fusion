//! Atomic turn auto-save, crash recovery, and fault tolerance engine for Fusion.
//!
//! # Architecture & Capabilities
//!
//! Fusion incorporates a comprehensive write-ahead auto-save, crash recovery, and
//! self-healing fault tolerance protocol to protect user conversations, tool execution
//! histories, and agent loops against crashes, terminal disconnections, model errors,
//! rate limits, context overflow, and network failures.
//!
//! ## Core Subsystems
//!
//! 1. **Write-Ahead Turn Auto-Save**: Atomically persists the complete turn context
//!    (user prompt, active session snapshot, turn index, active model, and tool queue)
//!    to `.fusion/recovery.json` immediately before and after every turn.
//! 2. **In-Flight Tool Execution Tracking**: Incremental atomic checkpoints updated as
//!    tools begin and complete execution, capturing partial tool outputs and execution durations.
//! 3. **Crash Detection & Resumption**: Proactively detects uncompleted or interrupted turns upon REPL
//!    startup or CLI invocation by inspecting `.fusion/recovery.json`.
//! 4. **Interactive Recovery Prompts & Commands**: Rich interactive dialog and `/recover`
//!    commands offering multiple resume strategies (`ReplayPrompt`, `ContinueTurn`, `RestoreSessionOnly`, `Discard`).
//! 5. **Error Classification Engine**: Classifies runtime errors into discrete fault categories
//!    (`TransientNetwork`, `RateLimit`, `ContextLengthExceeded`, `ToolExecutionFailure`, `InvalidModelOutput`,
//!    `AuthenticationOrQuota`, `InternalServerError`).
//! 6. **Automated Remediation Strategies**: Self-healing loop strategies including exponential
//!    backoff with jitter, automated fallback model routing, intelligent multi-level context pruning,
//!    malformed output repairs, and provider failover.
//! 7. **Circuit Breakers**: Multi-state circuit breakers (`Closed`, `Open`, `HalfOpen`) tracking
//!    service health per model and provider to prevent cascading retry storms.
//! 8. **Recovery History & Metrics**: Bounded audit log of all fault occurrences and remediation
//!    attempts, computing MTTR (Mean Time to Recovery), success rates, and fault distributions.

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use chrono::Utc;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::agent::session::Session;
use crate::config::Config;
use crate::provider::types::{Message, Role, ToolCall};

/// Standard recovery file name stored within the workspace directory.
pub const RECOVERY_FILE_NAME: &str = "recovery.json";

/// Standard workspace directory name.
pub const FUSION_DIR_NAME: &str = ".fusion";

/// Current recovery schema version for forward/backward compatibility.
pub const RECOVERY_SCHEMA_VERSION: u32 = 1;

// ============================================================================
// Core Enums & Data Structures for Crash Recovery
// ============================================================================

/// Identifies the discrete execution phase of an agent turn.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TurnPhase {
    /// Turn initialized; user prompt accepted and saved before LLM invocation.
    TurnStarted,
    /// Multi-agent advisor engine is actively reviewing the user request.
    AdvisorConsultation,
    /// Model reasoning / chain-of-thought phase in progress.
    Thinking,
    /// Streaming assistant tokens from the LLM provider.
    ModelStreaming,
    /// A tool is currently executing in the environment.
    ToolExecuting {
        /// Name of the active tool (e.g. `edit`, `bash`, `write`).
        tool_name: String,
        /// Tool call identifier assigned by the LLM provider.
        call_id: String,
        /// Short preview of tool arguments.
        args_preview: String,
        /// Timestamp when tool execution commenced.
        started_at: String,
    },
    /// A tool has completed execution.
    ToolCompleted {
        /// Name of the completed tool.
        tool_name: String,
        /// Tool call identifier.
        call_id: String,
        /// Whether tool execution succeeded.
        success: bool,
    },
    /// History compaction is being performed to conserve context budget.
    Compaction,
    /// Turn completed cleanly and successfully.
    TurnCompleted,
    /// Turn was interrupted by user signal (e.g. SIGINT / Ctrl+C).
    Interrupted {
        /// Reason or description of interruption.
        reason: String,
    },
    /// Process crashed or terminated unexpectedly during turn execution.
    Crashed {
        /// Optional error message or panic payload.
        error: Option<String>,
    },
}

impl TurnPhase {
    /// Returns a concise human-readable description of the execution phase.
    pub fn display_name(&self) -> &'static str {
        match self {
            Self::TurnStarted => "Turn Started (Pre-execution)",
            Self::AdvisorConsultation => "Advisor Consultation",
            Self::Thinking => "Reasoning / Chain of Thought",
            Self::ModelStreaming => "Model Generation",
            Self::ToolExecuting { .. } => "Tool Executing",
            Self::ToolCompleted { .. } => "Tool Completed",
            Self::Compaction => "History Compaction",
            Self::TurnCompleted => "Completed",
            Self::Interrupted { .. } => "Interrupted by User",
            Self::Crashed { .. } => "Process Terminated / Crashed",
        }
    }

    /// Returns `true` if the turn is in an active, uncompleted state.
    pub fn is_active(&self) -> bool {
        !matches!(self, Self::TurnCompleted | Self::Interrupted { .. })
    }
}

/// Recorded result of a tool execution completed within an active turn.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CompletedToolResult {
    /// Name of the executed tool.
    pub tool_name: String,
    /// Unique tool call ID assigned by LLM.
    pub call_id: String,
    /// Serialized arguments JSON.
    pub arguments: String,
    /// Whether the tool returned success.
    pub success: bool,
    /// Output produced by the tool (truncated if excessively large).
    pub output_preview: String,
    /// Duration of tool execution in milliseconds.
    pub duration_ms: u64,
    /// ISO 8601 completion timestamp.
    pub completed_at: String,
}

/// Complete persisted crash recovery snapshot stored in `.fusion/recovery.json`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecoveryState {
    /// Recovery schema version.
    #[serde(default = "default_schema_version")]
    pub schema_version: u32,
    /// Unique session UUID.
    pub session_id: Uuid,
    /// Active turn sequence index (1-based).
    pub turn_index: usize,
    /// User input prompt for the current turn.
    pub user_input: String,
    /// Current execution phase.
    pub phase: TurnPhase,
    /// Active model name used for this turn.
    pub active_model: String,
    /// Active working directory.
    pub working_dir: PathBuf,
    /// Timestamp when this turn began (RFC 3339).
    pub started_at: String,
    /// Timestamp when this recovery state was last updated (RFC 3339).
    pub updated_at: String,
    /// Operating system process ID that wrote this recovery state.
    pub process_id: u32,
    /// Optional machine hostname.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hostname: Option<String>,
    /// Snapshot of the session state prior to the current turn.
    pub session_snapshot: Session,
    /// Partial assistant text received before crash/interruption (if any).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub partial_assistant_response: Option<String>,
    /// Tools that completed execution during the current turn before interruption.
    #[serde(default)]
    pub completed_tools: Vec<CompletedToolResult>,
    /// True if the turn completed normally and no recovery is required.
    pub completed: bool,
    /// Optional error diagnostic message.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error_diagnostic: Option<String>,
}

fn default_schema_version() -> u32 {
    RECOVERY_SCHEMA_VERSION
}

impl RecoveryState {
    /// Creates a fresh `RecoveryState` initialized for the start of a turn.
    pub fn new_for_turn(
        session: &Session,
        user_input: &str,
        turn_index: usize,
        working_dir: &Path,
    ) -> Self {
        let now = Utc::now().to_rfc3339();
        Self {
            schema_version: RECOVERY_SCHEMA_VERSION,
            session_id: session.id,
            turn_index,
            user_input: user_input.to_string(),
            phase: TurnPhase::TurnStarted,
            active_model: session.active_model.clone(),
            working_dir: working_dir.to_path_buf(),
            started_at: now.clone(),
            updated_at: now,
            process_id: std::process::id(),
            hostname: get_hostname(),
            session_snapshot: session.clone(),
            partial_assistant_response: None,
            completed_tools: Vec::new(),
            completed: false,
            error_diagnostic: None,
        }
    }

    /// Updates the current turn execution phase and refreshes timestamp.
    pub fn update_phase(&mut self, phase: TurnPhase) {
        self.phase = phase;
        self.updated_at = Utc::now().to_rfc3339();
    }

    /// Records the commencement of a tool call.
    pub fn record_tool_start(&mut self, tool_name: &str, call_id: &str, args: &serde_json::Value) {
        let args_preview = args_preview_string(args);
        self.update_phase(TurnPhase::ToolExecuting {
            tool_name: tool_name.to_string(),
            call_id: call_id.to_string(),
            args_preview,
            started_at: Utc::now().to_rfc3339(),
        });
    }

    /// Records that a tool finished execution and appends to `completed_tools`.
    pub fn record_tool_finish(
        &mut self,
        tool_name: &str,
        call_id: &str,
        arguments: &str,
        success: bool,
        output: &str,
        duration: Duration,
    ) {
        let preview = truncate_output(output, 2048);
        self.completed_tools.push(CompletedToolResult {
            tool_name: tool_name.to_string(),
            call_id: call_id.to_string(),
            arguments: arguments.to_string(),
            success,
            output_preview: preview,
            duration_ms: duration.as_millis() as u64,
            completed_at: Utc::now().to_rfc3339(),
        });

        self.update_phase(TurnPhase::ToolCompleted {
            tool_name: tool_name.to_string(),
            call_id: call_id.to_string(),
            success,
        });
    }

    /// Appends partial assistant response streaming tokens.
    pub fn set_partial_response(&mut self, text: &str) {
        self.partial_assistant_response = Some(text.to_string());
        self.update_phase(TurnPhase::ModelStreaming);
    }

    /// Marks the turn as cleanly completed.
    pub fn mark_completed(&mut self) {
        self.completed = true;
        self.phase = TurnPhase::TurnCompleted;
        self.updated_at = Utc::now().to_rfc3339();
    }

    /// Marks the turn as crashed with diagnostic details.
    pub fn mark_crashed(&mut self, error: Option<String>) {
        self.completed = false;
        self.error_diagnostic = error.clone();
        self.phase = TurnPhase::Crashed { error };
        self.updated_at = Utc::now().to_rfc3339();
    }

    /// Marks the turn as interrupted by user signal.
    pub fn mark_interrupted(&mut self, reason: &str) {
        self.completed = false;
        self.phase = TurnPhase::Interrupted {
            reason: reason.to_string(),
        };
        self.updated_at = Utc::now().to_rfc3339();
    }

    /// Returns `true` if an uncompleted crash or abnormal termination occurred.
    pub fn is_uncompleted_crash(&self) -> bool {
        !self.completed && !matches!(self.phase, TurnPhase::TurnCompleted)
    }

    /// Generates a diagnostic `CrashReport` from this recovery state.
    pub fn to_crash_report(&self) -> CrashReport {
        let completed_names = self
            .completed_tools
            .iter()
            .map(|t| t.tool_name.clone())
            .collect();

        CrashReport {
            session_id: self.session_id,
            turn_index: self.turn_index,
            user_input: self.user_input.clone(),
            phase: self.phase.clone(),
            active_model: self.active_model.clone(),
            working_dir: self.working_dir.clone(),
            started_at: self.started_at.clone(),
            updated_at: self.updated_at.clone(),
            process_id: self.process_id,
            completed_tool_names: completed_names,
            completed_tool_count: self.completed_tools.len(),
            has_partial_response: self.partial_assistant_response.is_some(),
            partial_response_preview: self
                .partial_assistant_response
                .as_deref()
                .map(|s| truncate_output(s, 256)),
            error_diagnostic: self.error_diagnostic.clone(),
        }
    }
}

// ============================================================================
// Crash Diagnostics & Reporting
// ============================================================================

/// Diagnostic report summarizing an interrupted or crashed turn.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CrashReport {
    /// Session UUID.
    pub session_id: Uuid,
    /// Turn sequence index.
    pub turn_index: usize,
    /// User prompt that was running when interrupted.
    pub user_input: String,
    /// Phase where interruption or crash occurred.
    pub phase: TurnPhase,
    /// Active model name.
    pub active_model: String,
    /// Working directory.
    pub working_dir: PathBuf,
    /// Timestamp when turn started.
    pub started_at: String,
    /// Timestamp when state was last updated.
    pub updated_at: String,
    /// Process ID of the aborted instance.
    pub process_id: u32,
    /// List of tool names that completed execution before interruption.
    pub completed_tool_names: Vec<String>,
    /// Number of completed tool calls.
    pub completed_tool_count: usize,
    /// Whether any assistant response tokens were captured.
    pub has_partial_response: bool,
    /// Truncated preview of partial assistant text.
    pub partial_response_preview: Option<String>,
    /// Error diagnostic message if recorded.
    pub error_diagnostic: Option<String>,
}

impl CrashReport {
    /// Formats a clean, colorized CLI prompt explaining crash status and recovery options.
    pub fn format_prompt_message(&self) -> String {
        let mut msg = String::new();
        msg.push_str("\n\x1b[1;33m⚠ Interrupted Turn Detected\x1b[0m\n");
        msg.push_str(&format!(
            "  Session:     \x1b[36m{}\x1b[0m (Turn {})\n",
            self.session_id, self.turn_index
        ));
        msg.push_str(&format!(
            "  Model:       \x1b[35m{}\x1b[0m\n",
            self.active_model
        ));
        msg.push_str(&format!(
            "  Phase:       \x1b[1;31m{}\x1b[0m\n",
            self.phase.display_name()
        ));
        msg.push_str(&format!(
            "  Last Active: {}\n",
            self.updated_at
        ));

        let prompt_preview = truncate_output(&self.user_input, 120);
        msg.push_str(&format!("  Prompt:      \"{}\"\n", prompt_preview));

        if self.completed_tool_count > 0 {
            msg.push_str(&format!(
                "  Completed Tools ({}): \x1b[32m{}\x1b[0m\n",
                self.completed_tool_count,
                self.completed_tool_names.join(", ")
            ));
        }

        if let Some(diag) = &self.error_diagnostic {
            msg.push_str(&format!("  Error Detail: \x1b[31m{}\x1b[0m\n", diag));
        }

        msg.push_str("\n\x1b[1mRecovery Options:\x1b[0m\n");
        msg.push_str("  \x1b[1;32m[c]ontinue\x1b[0m - Resume turn preserving completed tool outputs (default)\n");
        msg.push_str("  \x1b[1;34m[r]eplay\x1b[0m   - Re-run prompt from scratch against clean session\n");
        msg.push_str("  \x1b[1;35m[s]ession\x1b[0m  - Restore session state without executing prompt\n");
        msg.push_str("  \x1b[1;31m[d]iscard\x1b[0m  - Discard recovery state and start fresh\n\n");
        msg.push_str("Choose recovery action [c/r/s/d] (Enter = continue): ");

        msg
    }

    /// Formats a detailed diagnostic string for `/recover status`.
    pub fn format_diagnostic(&self) -> String {
        let mut msg = String::new();
        msg.push_str("=== Crash Recovery Diagnostic Report ===\n");
        msg.push_str(&format!("Session ID:    {}\n", self.session_id));
        msg.push_str(&format!("Turn Index:    {}\n", self.turn_index));
        msg.push_str(&format!("Active Model:  {}\n", self.active_model));
        msg.push_str(&format!("Phase:         {}\n", self.phase.display_name()));
        msg.push_str(&format!("Started At:    {}\n", self.started_at));
        msg.push_str(&format!("Interrupted:   {}\n", self.updated_at));
        msg.push_str(&format!("Process ID:    {}\n", self.process_id));
        msg.push_str(&format!("Working Dir:   {}\n", self.working_dir.display()));
        msg.push_str(&format!("User Prompt:   {}\n", self.user_input));
        msg.push_str(&format!(
            "Completed Tools ({}): {}\n",
            self.completed_tool_count,
            if self.completed_tool_names.is_empty() {
                "none".to_string()
            } else {
                self.completed_tool_names.join(", ")
            }
        ));

        if let Some(part) = &self.partial_response_preview {
            msg.push_str(&format!("Partial Response: {}\n", part));
        }

        if let Some(diag) = &self.error_diagnostic {
            msg.push_str(&format!("Diagnostic:    {}\n", diag));
        }

        msg
    }
}

// ============================================================================
// Resume Strategies & Recovery Execution
// ============================================================================

/// Strategy for restoring state from an interrupted turn.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ResumeStrategy {
    /// Resume turn retaining completed tool results.
    ContinueTurn,
    /// Re-execute the original prompt from scratch against the pre-turn snapshot.
    ReplayPrompt,
    /// Restore conversation history up to failure without re-running the turn.
    RestoreSessionOnly,
    /// Discards the recovery file completely without restoring.
    Discard,
}

impl ResumeStrategy {
    /// Parses a string token into a `ResumeStrategy`.
    pub fn from_str_loose(s: &str) -> Option<Self> {
        let lower = s.trim().to_lowercase();
        match lower.as_str() {
            "c" | "continue" | "cont" | "continue_turn" | "yes" | "y" | "" => {
                Some(Self::ContinueTurn)
            }
            "r" | "replay" | "replay_prompt" | "retry" | "rerun" => Some(Self::ReplayPrompt),
            "s" | "session" | "restore" | "restore_session" | "restore_session_only" => {
                Some(Self::RestoreSessionOnly)
            }
            "d" | "discard" | "delete" | "clear" | "drop" | "no" | "n" => Some(Self::Discard),
            _ => None,
        }
    }
}

/// Outcome of a recovery resume operation.
#[derive(Debug, Clone)]
pub struct ResumeResult {
    /// The restored conversational session.
    pub session: Session,
    /// Optional user prompt string to execute (if `ReplayPrompt` or `ContinueTurn`).
    pub prompt_to_run: Option<String>,
    /// The recovery strategy that was executed.
    pub strategy: ResumeStrategy,
    /// Human-readable explanation of actions taken.
    pub summary: String,
}

// ============================================================================
// Error Types
// ============================================================================

/// Errors encountered during turn auto-save or crash recovery operations.
#[derive(Debug, thiserror::Error)]
pub enum RecoveryError {
    /// Standard I/O failure.
    #[error("I/O error during recovery operation: {0}")]
    Io(#[from] std::io::Error),

    /// JSON serialization or deserialization failure.
    #[error("Recovery serialization error: {0}")]
    Serialization(#[from] serde_json::Error),

    /// Recovery file was not found at specified path.
    #[error("Recovery file not found at '{0}'")]
    NotFound(PathBuf),

    /// Recovery file contains corrupted or invalid schema data.
    #[error("Recovery file corrupted: {0}")]
    Corrupted(String),

    /// Recovery state is in an invalid state for the requested operation.
    #[error("Invalid recovery state: {0}")]
    InvalidState(String),

    /// Remediation failure or unrecoverable condition.
    #[error("Remediation failure: {0}")]
    RemediationFailed(String),

    /// Circuit breaker is open and blocking requests.
    #[error("Circuit breaker is open: {0}")]
    CircuitOpen(String),
}

// ============================================================================
// File Path Utilities & Atomic Persistence
// ============================================================================

/// Resolves the default recovery file path: `<working_dir>/.fusion/recovery.json`.
pub fn recovery_file_path(working_dir: Option<&Path>) -> PathBuf {
    match working_dir {
        Some(wd) => wd.join(FUSION_DIR_NAME).join(RECOVERY_FILE_NAME),
        None => PathBuf::from(FUSION_DIR_NAME).join(RECOVERY_FILE_NAME),
    }
}

/// Resolves user-global fallback recovery path: `~/.fusion/recovery.json`.
pub fn global_recovery_path() -> PathBuf {
    Config::config_dir().join(RECOVERY_FILE_NAME)
}

/// Atomically persists `RecoveryState` to disk using a unique sibling temporary file.
pub fn save_recovery_state_atomic(
    path: &Path,
    state: &RecoveryState,
) -> Result<PathBuf, RecoveryError> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }

    let json_bytes = serde_json::to_vec_pretty(state)?;

    let temp_name = format!("{}.tmp.{}", RECOVERY_FILE_NAME, Uuid::new_v4());
    let temp_path = match path.parent() {
        Some(p) => p.join(temp_name),
        None => PathBuf::from(temp_name),
    };

    fs::write(&temp_path, &json_bytes)?;

    if let Err(e) = fs::rename(&temp_path, path) {
        let _ = fs::remove_file(&temp_path);
        return Err(RecoveryError::Io(e));
    }

    Ok(path.to_path_buf())
}

/// Loads and validates a `RecoveryState` from disk.
pub fn load_recovery_state(path: &Path) -> Result<RecoveryState, RecoveryError> {
    if !path.exists() {
        return Err(RecoveryError::NotFound(path.to_path_buf()));
    }

    let bytes = fs::read(path)?;
    match serde_json::from_slice::<RecoveryState>(&bytes) {
        Ok(state) => Ok(state),
        Err(e) => Err(RecoveryError::Corrupted(format!(
            "Failed to parse recovery JSON at '{}': {}",
            path.display(),
            e
        ))),
    }
}

/// Safely removes the recovery file if it exists. Returns `true` if a file was deleted.
pub fn clear_recovery_file(path: &Path) -> Result<bool, RecoveryError> {
    if path.exists() {
        fs::remove_file(path)?;
        Ok(true)
    } else {
        Ok(false)
    }
}

/// Checks if an interrupted turn or crash exists at the specified workspace location.
pub fn check_for_crash(working_dir: Option<&Path>) -> Result<Option<CrashReport>, RecoveryError> {
    let rec_path = recovery_file_path(working_dir);
    check_for_crash_at_path(&rec_path)
}

/// Checks if an uncompleted turn exists at the exact recovery file path.
pub fn check_for_crash_at_path(path: &Path) -> Result<Option<CrashReport>, RecoveryError> {
    if !path.exists() {
        return Ok(None);
    }

    match load_recovery_state(path) {
        Ok(state) => {
            if state.is_uncompleted_crash() {
                Ok(Some(state.to_crash_report()))
            } else {
                Ok(None)
            }
        }
        Err(RecoveryError::Corrupted(e)) => {
            let fallback_report = CrashReport {
                session_id: Uuid::nil(),
                turn_index: 0,
                user_input: "<corrupted recovery file>".to_string(),
                phase: TurnPhase::Crashed {
                    error: Some(format!("Corrupted recovery JSON: {}", e)),
                },
                active_model: "unknown".to_string(),
                working_dir: path.parent().unwrap_or_else(|| Path::new(".")).to_path_buf(),
                started_at: Utc::now().to_rfc3339(),
                updated_at: Utc::now().to_rfc3339(),
                process_id: 0,
                completed_tool_names: Vec::new(),
                completed_tool_count: 0,
                has_partial_response: false,
                partial_response_preview: None,
                error_diagnostic: Some(e),
            };
            Ok(Some(fallback_report))
        }
        Err(e) => Err(e),
    }
}

/// Restores a session from a `RecoveryState` according to the chosen `ResumeStrategy`.
pub fn resume_session_from_recovery(
    state: &RecoveryState,
    strategy: ResumeStrategy,
) -> Result<ResumeResult, RecoveryError> {
    let mut session = state.session_snapshot.clone();

    match strategy {
        ResumeStrategy::ContinueTurn => {
            if !state.completed_tools.is_empty() {
                let tool_calls: Vec<ToolCall> = state
                    .completed_tools
                    .iter()
                    .map(|t| ToolCall {
                        id: t.call_id.clone(),
                        name: t.tool_name.clone(),
                        arguments: t.arguments.clone(),
                    })
                    .collect();

                let assistant_text = state
                    .partial_assistant_response
                    .clone()
                    .unwrap_or_default();

                session.messages.push(Message {
                    role: Role::Assistant,
                    content: assistant_text,
                    name: None,
                    tool_calls: Some(tool_calls),
                    tool_call_id: None,
                });

                for tool in &state.completed_tools {
                    session.messages.push(Message {
                        role: Role::Tool,
                        content: tool.output_preview.clone(),
                        name: Some(tool.tool_name.clone()),
                        tool_calls: None,
                        tool_call_id: Some(tool.call_id.clone()),
                    });
                }
            }

            let summary = format!(
                "Restored session to turn {} with {} completed tool executions preserved.",
                state.turn_index,
                state.completed_tools.len()
            );

            Ok(ResumeResult {
                session,
                prompt_to_run: Some(state.user_input.clone()),
                strategy,
                summary,
            })
        }
        ResumeStrategy::ReplayPrompt => {
            let summary = format!(
                "Restored clean session prior to turn {}. Re-queued original user prompt.",
                state.turn_index
            );

            Ok(ResumeResult {
                session,
                prompt_to_run: Some(state.user_input.clone()),
                strategy,
                summary,
            })
        }
        ResumeStrategy::RestoreSessionOnly => {
            let summary = format!(
                "Restored session state up to turn {} without executing prompt.",
                state.turn_index
            );

            Ok(ResumeResult {
                session,
                prompt_to_run: None,
                strategy,
                summary,
            })
        }
        ResumeStrategy::Discard => {
            Ok(ResumeResult {
                session,
                prompt_to_run: None,
                strategy,
                summary: "Discarded recovery state and initialized fresh session".to_string(),
            })
        }
    }
}

// ============================================================================
// Turn Recovery Manager
// ============================================================================

/// Coordinates turn auto-save lifecycle, crash inspection, and state restoration.
#[derive(Debug, Clone)]
pub struct RecoveryManager {
    /// Active recovery file path.
    recovery_path: PathBuf,
    /// Active working directory.
    working_dir: PathBuf,
    /// Whether turn auto-saving is enabled.
    enabled: bool,
}

impl RecoveryManager {
    /// Creates a new `RecoveryManager` for a workspace working directory.
    pub fn new(working_dir: impl Into<PathBuf>) -> Self {
        let wd = working_dir.into();
        let rec_path = recovery_file_path(Some(&wd));
        Self {
            recovery_path: rec_path,
            working_dir: wd,
            enabled: true,
        }
    }

    /// Creates a `RecoveryManager` with a custom recovery file path.
    pub fn with_path(
        recovery_path: impl Into<PathBuf>,
        working_dir: impl Into<PathBuf>,
    ) -> Self {
        Self {
            recovery_path: recovery_path.into(),
            working_dir: working_dir.into(),
            enabled: true,
        }
    }

    /// Creates a manager configured for the current process working directory.
    pub fn default_for_cwd() -> Self {
        let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
        Self::new(cwd)
    }

    /// Returns a reference to the active recovery file path.
    pub fn recovery_path(&self) -> &Path {
        &self.recovery_path
    }

    /// Returns a reference to the active working directory.
    pub fn working_dir(&self) -> &Path {
        &self.working_dir
    }

    /// Enables or disables auto-save functionality.
    pub fn set_enabled(&mut self, enabled: bool) {
        self.enabled = enabled;
    }

    /// Returns true if auto-save is enabled.
    pub fn is_enabled(&self) -> bool {
        self.enabled
    }

    /// Checks if an interrupted turn or crash exists in the recovery file.
    pub fn detect_crash(&self) -> Option<CrashReport> {
        check_for_crash_at_path(&self.recovery_path).ok().flatten()
    }

    /// Loads the active recovery state from disk if present.
    pub fn load_active_state(&self) -> Result<Option<RecoveryState>, RecoveryError> {
        if !self.recovery_path.exists() {
            return Ok(None);
        }
        load_recovery_state(&self.recovery_path).map(Some)
    }

    /// Auto-saves state immediately before starting a conversation turn.
    pub fn on_turn_start(
        &self,
        session: &Session,
        user_input: &str,
        turn_index: usize,
    ) -> Result<PathBuf, RecoveryError> {
        if !self.enabled {
            return Ok(self.recovery_path.clone());
        }

        let state = RecoveryState::new_for_turn(
            session,
            user_input,
            turn_index,
            &self.working_dir,
        );

        save_recovery_state_atomic(&self.recovery_path, &state)
    }

    /// Updates recovery state during advisor consultation.
    pub fn on_advisor_phase(&self) -> Result<PathBuf, RecoveryError> {
        self.mutate_active_state(|state| {
            state.update_phase(TurnPhase::AdvisorConsultation);
        })
    }

    /// Updates recovery state during model streaming with partial assistant tokens.
    pub fn on_model_streaming(&self, partial_content: &str) -> Result<PathBuf, RecoveryError> {
        self.mutate_active_state(|state| {
            state.set_partial_response(partial_content);
        })
    }

    /// Updates recovery state when a tool starts execution.
    pub fn on_tool_start(
        &self,
        tool_name: &str,
        call_id: &str,
        args: &serde_json::Value,
    ) -> Result<PathBuf, RecoveryError> {
        self.mutate_active_state(|state| {
            state.record_tool_start(tool_name, call_id, args);
        })
    }

    /// Updates recovery state when a tool completes execution.
    pub fn on_tool_finish(
        &self,
        tool_name: &str,
        call_id: &str,
        arguments: &str,
        success: bool,
        output: &str,
        duration: Duration,
    ) -> Result<PathBuf, RecoveryError> {
        self.mutate_active_state(|state| {
            state.record_tool_finish(tool_name, call_id, arguments, success, output, duration);
        })
    }

    /// Auto-saves state immediately upon clean completion of a conversation turn.
    pub fn on_turn_completed(&self, session: &Session) -> Result<PathBuf, RecoveryError> {
        if !self.enabled {
            return Ok(self.recovery_path.clone());
        }

        let mut state = self
            .load_active_state()?
            .unwrap_or_else(|| RecoveryState::new_for_turn(session, "", 1, &self.working_dir));

        state.session_snapshot = session.clone();
        state.mark_completed();

        save_recovery_state_atomic(&self.recovery_path, &state)
    }

    /// Records that the turn aborted due to an error.
    pub fn on_turn_error(&self, error: &str) -> Result<PathBuf, RecoveryError> {
        self.mutate_active_state(|state| {
            state.mark_crashed(Some(error.to_string()));
        })
    }

    /// Records that the turn was interrupted by user signal.
    pub fn on_turn_interrupted(&self, reason: &str) -> Result<PathBuf, RecoveryError> {
        self.mutate_active_state(|state| {
            state.mark_interrupted(reason);
        })
    }

    /// Clears the recovery file.
    pub fn clear(&self) -> Result<bool, RecoveryError> {
        clear_recovery_file(&self.recovery_path)
    }

    /// Resumes an interrupted session using the specified strategy.
    pub fn resume(&self, strategy: ResumeStrategy) -> Result<ResumeResult, RecoveryError> {
        let state = self.load_active_state()?.ok_or_else(|| {
            RecoveryError::NotFound(self.recovery_path.clone())
        })?;

        let result = resume_session_from_recovery(&state, strategy)?;

        let _ = self.clear();

        Ok(result)
    }

    /// Creates an RAII `TurnRecoveryGuard` to protect turn execution against panics.
    pub fn create_guard<'a>(
        &'a self,
        session: &'a Session,
        user_input: &'a str,
        turn_index: usize,
    ) -> Result<TurnRecoveryGuard<'a>, RecoveryError> {
        self.on_turn_start(session, user_input, turn_index)?;
        Ok(TurnRecoveryGuard::new(self, session))
    }

    fn mutate_active_state<F>(&self, mutator: F) -> Result<PathBuf, RecoveryError>
    where
        F: FnOnce(&mut RecoveryState),
    {
        if !self.enabled {
            return Ok(self.recovery_path.clone());
        }

        if let Some(mut state) = self.load_active_state()? {
            mutator(&mut state);
            save_recovery_state_atomic(&self.recovery_path, &state)
        } else {
            Ok(self.recovery_path.clone())
        }
    }
}

// ============================================================================
// RAII Turn Recovery Guard
// ============================================================================

/// RAII Drop guard that guarantees crash persistence if turn execution drops unexpectedly.
pub struct TurnRecoveryGuard<'a> {
    manager: &'a RecoveryManager,
    session: &'a Session,
    completed: bool,
}

impl<'a> TurnRecoveryGuard<'a> {
    /// Creates a new active guard.
    pub fn new(manager: &'a RecoveryManager, session: &'a Session) -> Self {
        Self {
            manager,
            session,
            completed: false,
        }
    }

    /// Marks the turn as cleanly completed, preventing drop crash recording.
    pub fn mark_completed(mut self, session: &Session) -> Result<PathBuf, RecoveryError> {
        self.completed = true;
        self.manager.on_turn_completed(session)
    }

    /// Records partial assistant text.
    pub fn record_partial_response(&self, text: &str) {
        let _ = self.manager.on_model_streaming(text);
    }

    /// Records tool start.
    pub fn record_tool_start(&self, name: &str, id: &str, args: &serde_json::Value) {
        let _ = self.manager.on_tool_start(name, id, args);
    }

    /// Records tool completion.
    pub fn record_tool_finish(
        &self,
        name: &str,
        id: &str,
        arguments: &str,
        success: bool,
        output: &str,
        duration: Duration,
    ) {
        let _ = self
            .manager
            .on_tool_finish(name, id, arguments, success, output, duration);
    }
}

impl<'a> Drop for TurnRecoveryGuard<'a> {
    fn drop(&mut self) {
        if !self.completed {
            let is_panicking = std::thread::panicking();
            let reason = if is_panicking {
                "Turn panicked unexpectedly"
            } else {
                "Turn dropped before clean completion"
            };
            let _ = self.manager.on_turn_error(reason);
        }
    }
}

// ============================================================================
// Interactive Prompt & Slash Command Integration
// ============================================================================

/// Formats a complete interactive recovery dialog header.
pub fn format_crash_banner(report: &CrashReport) -> String {
    report.format_prompt_message()
}

/// Dispatches `/recover` slash commands.
pub fn handle_recovery_command(
    args: &str,
    working_dir: &Path,
    session: &mut Session,
) -> String {
    let mgr = RecoveryManager::new(working_dir.to_path_buf());
    let trimmed = args.trim();

    let tokens: Vec<&str> = trimmed.split_whitespace().collect();
    let subcommand = tokens.first().copied().unwrap_or("status");

    match subcommand {
        "status" | "info" | "show" => match mgr.detect_crash() {
            Some(report) => report.format_diagnostic(),
            None => {
                let rec_path = mgr.recovery_path();
                if rec_path.exists() {
                    match mgr.load_active_state() {
                        Ok(Some(st)) => {
                            if st.completed {
                                format!(
                                    "Recovery status: Clean (Last turn completed normally at {})\nRecovery file: `{}`",
                                    st.updated_at,
                                    rec_path.display()
                                )
                            } else {
                                st.to_crash_report().format_diagnostic()
                            }
                        }
                        Ok(None) => "No active recovery state recorded.".to_string(),
                        Err(e) => format!("Error reading recovery state: {}", e),
                    }
                } else {
                    format!(
                        "No crash recovery file found at `{}`. Environment clean.",
                        rec_path.display()
                    )
                }
            }
        },
        "resume" => {
            let strategy_str = tokens.get(1).copied().unwrap_or("continue");
            let strategy = ResumeStrategy::from_str_loose(strategy_str)
                .unwrap_or(ResumeStrategy::ContinueTurn);

            match mgr.resume(strategy) {
                Ok(res) => {
                    *session = res.session;
                    let mut out = format!("✓ Crash recovery executed successfully.\n{}", res.summary);
                    if let Some(p) = res.prompt_to_run {
                        out.push_str(&format!("\nRe-queued prompt: \"{}\"", p));
                    }
                    out
                }
                Err(e) => format!("❌ Failed to resume from recovery state: {}", e),
            }
        }
        "discard" | "clear" | "delete" => match mgr.clear() {
            Ok(true) => "✓ Crash recovery file safely deleted.".to_string(),
            Ok(false) => "No crash recovery file was present to delete.".to_string(),
            Err(e) => format!("❌ Error deleting recovery file: {}", e),
        },
        "diff" => match mgr.load_active_state() {
            Ok(Some(st)) => {
                let mut out = format!(
                    "=== Recovery State Turn {} Diff ===\nPrompt: {}\nPhase: {}\nCompleted Tools: {}\n",
                    st.turn_index,
                    st.user_input,
                    st.phase.display_name(),
                    st.completed_tools.len()
                );
                for (i, tool) in st.completed_tools.iter().enumerate() {
                    out.push_str(&format!(
                        "\n[{}] Tool: {} (status: {})\nArguments: {}\nOutput Preview:\n{}\n",
                        i + 1,
                        tool.tool_name,
                        if tool.success { "success" } else { "failed" },
                        tool.arguments,
                        tool.output_preview
                    ));
                }
                out
            }
            Ok(None) => "No recovery state available for diff inspection.".to_string(),
            Err(e) => format!("Error loading recovery state: {}", e),
        },
        "help" | _ => {
            r#"### `/recover` Crash Recovery Commands
- `/recover` or `/recover status` - Display active recovery status and diagnostics.
- `/recover resume [continue|replay|restore]` - Resume interrupted session.
- `/recover diff` - Inspect completed tool outputs and turn modifications.
- `/recover discard` - Delete recovery state and clear recovery file.
"#
            .to_string()
        }
    }
}

// ============================================================================
// FAULT TOLERANCE: Error Classification & Diagnostics
// ============================================================================

/// Severity level associated with an error condition.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorSeverity {
    /// Low severity: transient glitch, quickly retryable.
    Low,
    /// Medium severity: rate limit or moderate context pressure.
    Medium,
    /// High severity: malformed output or model exhaustion, requires model switch or pruning.
    High,
    /// Critical severity: persistent auth failure or broken invariant, requires user escalation.
    Critical,
}

/// Comprehensive classification of runtime agent and LLM errors.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "category", rename_all = "snake_case")]
pub enum ErrorClass {
    /// Transient network interruption, socket drop, DNS resolution failure, or HTTP 502/503/504.
    TransientNetwork {
        reason: String,
        status_code: Option<u16>,
        retryable: bool,
    },
    /// Rate limit reached (HTTP 429), quota exhausted, or token bucket depleted.
    RateLimit {
        retry_after: Option<Duration>,
        limit_type: Option<String>,
        status_code: Option<u16>,
    },
    /// Context window limit reached or maximum prompt token length exceeded.
    ContextLengthExceeded {
        requested_tokens: Option<usize>,
        max_context_tokens: Option<usize>,
        model: Option<String>,
    },
    /// Tool execution returned a nonzero exit code, crashed, or was terminated.
    ToolExecutionFailure {
        tool_name: String,
        error_message: String,
        is_recoverable: bool,
    },
    /// Model output could not be parsed as valid JSON, hallucinated invalid tool schema, or returned empty tokens.
    InvalidModelOutput {
        reason: String,
        raw_snippet: Option<String>,
        expected_format: Option<String>,
    },
    /// Authentication failure, invalid API key, or permission denied.
    AuthenticationOrQuota {
        reason: String,
        status_code: Option<u16>,
    },
    /// Upstream provider internal error (HTTP 500, model server overload, etc.).
    InternalServerError {
        message: String,
        status_code: Option<u16>,
    },
    /// Unclassified or unknown error string.
    Unknown {
        raw: String,
    },
}

impl ErrorClass {
    /// Classifies an arbitrary error string or message into a structured `ErrorClass`.
    pub fn classify(err: &str) -> Self {
        let trimmed = err.trim();
        let lower = trimmed.to_lowercase();

        // 1. Check for JSON structured error response bodies
        if let Ok(val) = serde_json::from_str::<serde_json::Value>(trimmed) {
            if let Some(error_obj) = val.get("error") {
                let code = error_obj.get("code").and_then(|c| c.as_str()).unwrap_or("");
                let msg = error_obj.get("message").and_then(|m| m.as_str()).unwrap_or("");
                let error_type = error_obj.get("type").and_then(|t| t.as_str()).unwrap_or("");

                if code.contains("context_length_exceeded")
                    || code.contains("max_tokens")
                    || msg.contains("context length")
                    || msg.contains("maximum context")
                {
                    return Self::ContextLengthExceeded {
                        requested_tokens: None,
                        max_context_tokens: None,
                        model: None,
                    };
                }

                if code.contains("rate_limit")
                    || error_type.contains("rate_limit")
                    || msg.contains("rate limit")
                    || msg.contains("quota")
                {
                    return Self::RateLimit {
                        retry_after: None,
                        limit_type: Some(code.to_string()),
                        status_code: Some(429),
                    };
                }

                if code.contains("invalid_api_key")
                    || code.contains("unauthorized")
                    || msg.contains("api key")
                    || msg.contains("authentication")
                {
                    return Self::AuthenticationOrQuota {
                        reason: msg.to_string(),
                        status_code: Some(401),
                    };
                }
            }
        }

        // 2. Context length exceeded checks
        if lower.contains("context_length_exceeded")
            || lower.contains("maximum context length")
            || lower.contains("context window")
            || lower.contains("tokens exceeded")
            || lower.contains("prompt is too long")
            || lower.contains("too many tokens")
            || lower.contains("exceeds the limit of")
            || lower.contains("max_tokens")
            || lower.contains("input is too long")
            || lower.contains("token limit")
            || lower.contains("kv cache capacity")
            || lower.contains("reduce the length of the messages")
        {
            return Self::ContextLengthExceeded {
                requested_tokens: None,
                max_context_tokens: None,
                model: None,
            };
        }

        // 3. Rate limiting checks
        if lower.contains("rate limit")
            || lower.contains("rate_limit")
            || lower.contains("429")
            || lower.contains("too many requests")
            || lower.contains("quota exceeded")
            || lower.contains("tokens per minute")
            || lower.contains("requests per minute")
            || lower.contains("tpm limit")
            || lower.contains("rpm limit")
            || lower.contains("insufficient credits")
            || lower.contains("credit balance is too low")
            || lower.contains("exceeded your current quota")
            || lower.contains("retry-after")
            || lower.contains("resource exhausted")
            || lower.contains("resource_exhausted")
            || lower.contains("capacity limit")
        {
            let mut retry_after = None;
            // Attempt to extract numeric seconds if present, e.g. "retry after 5s"
            if let Some(idx) = lower.find("retry after") {
                let rest = &lower[idx + 11..];
                let num_str: String = rest.chars().skip_while(|c| !c.is_numeric()).take_while(|c| c.is_numeric()).collect();
                if let Ok(secs) = num_str.parse::<u64>() {
                    retry_after = Some(Duration::from_secs(secs));
                }
            }

            return Self::RateLimit {
                retry_after,
                limit_type: None,
                status_code: Some(429),
            };
        }

        // 4. Transient network issues & Gateway errors
        if lower.contains("timeout")
            || lower.contains("timed out")
            || lower.contains("connection reset")
            || lower.contains("connection refused")
            || lower.contains("connection closed")
            || lower.contains("broken pipe")
            || lower.contains("network error")
            || lower.contains("dns error")
            || lower.contains("failed to lookup address")
            || lower.contains("tcp reset")
            || lower.contains("eof while parsing")
            || lower.contains("transport error")
            || lower.contains("connection aborted")
            || lower.contains("socket hang up")
            || lower.contains("gateway timeout")
            || lower.contains("bad gateway")
            || lower.contains("service unavailable")
            || lower.contains("502")
            || lower.contains("503")
            || lower.contains("504")
            || lower.contains("500 internal server error")
        {
            let status = if lower.contains("504") || lower.contains("gateway timeout") {
                Some(504)
            } else if lower.contains("503") || lower.contains("service unavailable") {
                Some(503)
            } else if lower.contains("502") || lower.contains("bad gateway") {
                Some(502)
            } else {
                None
            };

            return Self::TransientNetwork {
                reason: trimmed.to_string(),
                status_code: status,
                retryable: true,
            };
        }

        // 5. Invalid model output & JSON parser errors
        if lower.contains("invalid json")
            || lower.contains("failed to parse tool call")
            || lower.contains("malformed json")
            || lower.contains("missing field")
            || lower.contains("schema validation")
            || lower.contains("unexpected token")
            || lower.contains("json parse error")
            || lower.contains("invalid tool format")
            || lower.contains("unknown tool")
            || lower.contains("empty response")
            || lower.contains("missing required parameter")
            || lower.contains("unclosed quote")
            || lower.contains("expected comma")
        {
            return Self::InvalidModelOutput {
                reason: trimmed.to_string(),
                raw_snippet: Some(truncate_output(trimmed, 256)),
                expected_format: None,
            };
        }

        // 6. Tool execution failure
        if lower.contains("tool execution")
            || lower.contains("tool error")
            || lower.contains("tool returned error")
            || lower.contains("command failed")
            || lower.contains("exit code")
            || lower.contains("file not found")
            || lower.contains("no such file or directory")
            || lower.contains("permission denied")
            || lower.contains("invalid argument")
            || lower.contains("tool crashed")
        {
            return Self::ToolExecutionFailure {
                tool_name: "generic".to_string(),
                error_message: trimmed.to_string(),
                is_recoverable: true,
            };
        }

        // 7. Authentication or quota
        if lower.contains("invalid api key")
            || lower.contains("unauthorized")
            || lower.contains("401")
            || lower.contains("403 forbidden")
            || lower.contains("authentication failed")
            || lower.contains("billing")
        {
            return Self::AuthenticationOrQuota {
                reason: trimmed.to_string(),
                status_code: Some(401),
            };
        }

        // 8. Internal Server Error
        if lower.contains("internal server error")
            || lower.contains("server error")
            || lower.contains("500")
            || lower.contains("model overloaded")
            || lower.contains("upstream error")
        {
            return Self::InternalServerError {
                message: trimmed.to_string(),
                status_code: Some(500),
            };
        }

        Self::Unknown {
            raw: trimmed.to_string(),
        }
    }

    /// Classifies an HTTP response status code and optional body.
    pub fn classify_http(status: u16, body: Option<&str>) -> Self {
        match status {
            429 => {
                let mut retry_after = None;
                if let Some(b) = body {
                    let b_lower = b.to_lowercase();
                    if let Some(idx) = b_lower.find("retry-after") {
                        let rest = &b_lower[idx + 11..];
                        let num: String = rest.chars().skip_while(|c| !c.is_numeric()).take_while(|c| c.is_numeric()).collect();
                        if let Ok(s) = num.parse::<u64>() {
                            retry_after = Some(Duration::from_secs(s));
                        }
                    }
                }
                Self::RateLimit {
                    retry_after,
                    limit_type: None,
                    status_code: Some(429),
                }
            }
            400 => {
                if let Some(b) = body {
                    let b_lower = b.to_lowercase();
                    if b_lower.contains("context_length_exceeded")
                        || b_lower.contains("maximum context length")
                        || b_lower.contains("token limit")
                    {
                        return Self::ContextLengthExceeded {
                            requested_tokens: None,
                            max_context_tokens: None,
                            model: None,
                        };
                    }
                }
                Self::InvalidModelOutput {
                    reason: body.unwrap_or("HTTP 400 Bad Request").to_string(),
                    raw_snippet: body.map(|b| truncate_output(b, 256)),
                    expected_format: None,
                }
            }
            401 | 403 => Self::AuthenticationOrQuota {
                reason: body.unwrap_or("HTTP 401/403 Unauthorized").to_string(),
                status_code: Some(status),
            },
            500 => Self::InternalServerError {
                message: body.unwrap_or("HTTP 500 Internal Server Error").to_string(),
                status_code: Some(500),
            },
            502 | 503 | 504 => Self::TransientNetwork {
                reason: body.unwrap_or("HTTP Gateway/Service Unavailable").to_string(),
                status_code: Some(status),
                retryable: true,
            },
            _ => {
                if let Some(b) = body {
                    Self::classify(b)
                } else {
                    Self::Unknown {
                        raw: format!("HTTP Status {}", status),
                    }
                }
            }
        }
    }

    /// Returns `true` if this error category is generally retryable without structural modification.
    pub fn is_retryable(&self) -> bool {
        match self {
            Self::TransientNetwork { retryable, .. } => *retryable,
            Self::RateLimit { .. } => true,
            Self::InternalServerError { .. } => true,
            Self::ContextLengthExceeded { .. } => false,
            Self::ToolExecutionFailure { is_recoverable, .. } => *is_recoverable,
            Self::InvalidModelOutput { .. } => true,
            Self::AuthenticationOrQuota { .. } => false,
            Self::Unknown { .. } => false,
        }
    }

    /// Returns `true` if switching to a fallback model is recommended.
    pub fn is_model_switch_recommended(&self) -> bool {
        matches!(
            self,
            Self::RateLimit { .. }
                | Self::ContextLengthExceeded { .. }
                | Self::InternalServerError { .. }
                | Self::InvalidModelOutput { .. }
        )
    }

    /// Returns `true` if context window reduction/pruning is recommended.
    pub fn is_context_pruning_recommended(&self) -> bool {
        matches!(self, Self::ContextLengthExceeded { .. })
    }

    /// Returns the default suggested initial backoff duration for this error class.
    pub fn default_backoff(&self) -> Duration {
        match self {
            Self::TransientNetwork { .. } => Duration::from_millis(500),
            Self::RateLimit { retry_after, .. } => {
                retry_after.unwrap_or_else(|| Duration::from_secs(2))
            }
            Self::InternalServerError { .. } => Duration::from_secs(1),
            Self::ToolExecutionFailure { .. } => Duration::from_millis(200),
            Self::InvalidModelOutput { .. } => Duration::from_millis(100),
            Self::ContextLengthExceeded { .. } => Duration::ZERO,
            Self::AuthenticationOrQuota { .. } => Duration::ZERO,
            Self::Unknown { .. } => Duration::from_millis(500),
        }
    }

    /// Returns the severity rating of this error.
    pub fn severity(&self) -> ErrorSeverity {
        match self {
            Self::TransientNetwork { .. } => ErrorSeverity::Low,
            Self::RateLimit { .. } => ErrorSeverity::Medium,
            Self::ToolExecutionFailure { .. } => ErrorSeverity::Medium,
            Self::InvalidModelOutput { .. } => ErrorSeverity::Medium,
            Self::InternalServerError { .. } => ErrorSeverity::High,
            Self::ContextLengthExceeded { .. } => ErrorSeverity::High,
            Self::AuthenticationOrQuota { .. } => ErrorSeverity::Critical,
            Self::Unknown { .. } => ErrorSeverity::High,
        }
    }

    /// Returns a short label for the error classification.
    pub fn name(&self) -> &'static str {
        match self {
            Self::TransientNetwork { .. } => "TransientNetwork",
            Self::RateLimit { .. } => "RateLimit",
            Self::ContextLengthExceeded { .. } => "ContextLengthExceeded",
            Self::ToolExecutionFailure { .. } => "ToolExecutionFailure",
            Self::InvalidModelOutput { .. } => "InvalidModelOutput",
            Self::AuthenticationOrQuota { .. } => "AuthenticationOrQuota",
            Self::InternalServerError { .. } => "InternalServerError",
            Self::Unknown { .. } => "Unknown",
        }
    }

    /// Formats a diagnostic description of the error class.
    pub fn description(&self) -> String {
        match self {
            Self::TransientNetwork { reason, status_code, .. } => {
                format!("Transient network glitch (status: {:?}): {}", status_code, reason)
            }
            Self::RateLimit { retry_after, limit_type, .. } => {
                format!(
                    "Rate limit reached (type: {:?}, retry_after: {:?})",
                    limit_type, retry_after
                )
            }
            Self::ContextLengthExceeded { requested_tokens, max_context_tokens, model } => {
                format!(
                    "Context length exceeded for model {:?} (requested: {:?}, max: {:?})",
                    model, requested_tokens, max_context_tokens
                )
            }
            Self::ToolExecutionFailure { tool_name, error_message, .. } => {
                format!("Tool '{}' execution failed: {}", tool_name, error_message)
            }
            Self::InvalidModelOutput { reason, raw_snippet, .. } => {
                format!("Malformed model output: {} (snippet: {:?})", reason, raw_snippet)
            }
            Self::AuthenticationOrQuota { reason, status_code } => {
                format!("Auth/Quota error (status: {:?}): {}", status_code, reason)
            }
            Self::InternalServerError { message, status_code } => {
                format!("Internal upstream error (status: {:?}): {}", status_code, message)
            }
            Self::Unknown { raw } => format!("Unknown fault: {}", raw),
        }
    }
}

// ============================================================================
// FAULT TOLERANCE: Automated Remediation Strategies & Policies
// ============================================================================

/// Strategy for pruning session conversation history to satisfy context budgets.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ContextPruningStrategy {
    /// Drop intermediate tool outputs while keeping the most recent tool results.
    DropIntermediateToolOutputs,
    /// Truncate large tool result payloads preserving prefixes and suffixes.
    TruncateLargeToolOutputs { max_output_chars: usize },
    /// Summarize older conversational turns into a compact summary block.
    SummarizeOldestTurns { keep_recent_turns: usize },
    /// Drop oldest user/assistant turns, preserving system prompt.
    DropOldestUserTurns { keep_recent_turns: usize },
    /// Sliding window retaining only the most recent N messages.
    SlidingWindow { max_messages: usize },
    /// Composite strategy applying truncation, tool dropping, and turn summarization sequentially.
    AdaptiveComposite,
}

/// Report summarizing context pruning results.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PruningReport {
    /// Estimated tokens freed by the pruning operation.
    pub tokens_freed_est: usize,
    /// Number of messages completely removed.
    pub messages_removed: usize,
    /// Number of tool output payloads truncated.
    pub tool_outputs_truncated: usize,
    /// Human-readable explanation of actions taken.
    pub summary: String,
}

/// Automated corrective remediation action decided by the fault tolerance engine.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum RemediationAction {
    /// Retry the current request after an exponential backoff delay with jitter.
    RetryWithBackoff {
        delay: Duration,
        attempt: usize,
        max_retries: usize,
        jitter: bool,
    },
    /// Switch from the failing model to a suitable fallback model in the fallback chain.
    SwitchFallbackModel {
        primary_model: String,
        fallback_model: String,
        reason: String,
    },
    /// Prune session history to fit within model context length bounds.
    PruneContext {
        target_reduction_tokens: usize,
        strategy: ContextPruningStrategy,
        preserve_system_prompt: bool,
    },
    /// Injects corrective formatting instructions into the session to fix malformed output.
    FixMalformedOutput {
        error_details: String,
        repair_instructions: String,
    },
    /// Failover to a secondary provider (e.g. Anthropic -> OpenAI).
    FailoverProvider {
        from_provider: String,
        to_provider: String,
    },
    /// Disable a failing tool and suggest an alternative or continue without it.
    DisableTool {
        tool_name: String,
        alternative_tool: Option<String>,
    },
    /// Escalate directly to user intervention when automatic remediation is impossible.
    EscalateToUser {
        reason: String,
        diagnostic_report: String,
    },
    /// No corrective action required.
    NoAction,
}

/// Type alias for ergonomic equivalence.
pub type RemediationStrategy = RemediationAction;

// ============================================================================
// Context Pruning Engine
// ============================================================================

/// Estimates the token count of a single message (approx 1 token per 4 chars + overhead).
pub fn estimate_message_tokens(msg: &Message) -> usize {
    let mut chars = msg.content.len();
    if let Some(name) = &msg.name {
        chars += name.len();
    }
    if let Some(calls) = &msg.tool_calls {
        for call in calls {
            chars += call.name.len() + call.arguments.len() + 16;
        }
    }
    (chars / 4).max(1) + 4
}

/// Estimates the total token count of all messages in a session.
pub fn estimate_session_tokens(session: &Session) -> usize {
    let mut total = 0;
    if let Some(sys) = &session.system_prompt {
        total += (sys.len() / 4) + 4;
    }
    for msg in &session.messages {
        total += estimate_message_tokens(msg);
    }
    total
}

/// Prunes session context according to the specified strategy to free up token budget.
pub fn prune_session_context(
    session: &mut Session,
    target_reduction: usize,
    strategy: ContextPruningStrategy,
) -> PruningReport {
    let initial_tokens = estimate_session_tokens(session);
    let mut messages_removed = 0;
    let mut tool_outputs_truncated = 0;

    match strategy {
        ContextPruningStrategy::TruncateLargeToolOutputs { max_output_chars } => {
            for msg in &mut session.messages {
                if msg.role == Role::Tool && msg.content.len() > max_output_chars {
                    let half = max_output_chars / 2;
                    let prefix = &msg.content[..half];
                    let suffix = &msg.content[msg.content.len() - half..];
                    msg.content = format!(
                        "{}\n\n... [{} characters omitted for context limit] ...\n\n{}",
                        prefix,
                        msg.content.len() - max_output_chars,
                        suffix
                    );
                    tool_outputs_truncated += 1;
                }
            }
        }
        ContextPruningStrategy::DropIntermediateToolOutputs => {
            let total_msgs = session.messages.len();
            // Preserve tool outputs in the most recent 2 messages, prune earlier ones
            for (i, msg) in session.messages.iter_mut().enumerate() {
                if i + 2 < total_msgs && msg.role == Role::Tool {
                    if msg.content.len() > 120 {
                        msg.content = "[Tool output pruned to fit context window]".to_string();
                        tool_outputs_truncated += 1;
                    }
                }
            }
        }
        ContextPruningStrategy::SummarizeOldestTurns { keep_recent_turns } => {
            let keep_msg_count = (keep_recent_turns * 2).max(2);
            if session.messages.len() > keep_msg_count + 1 {
                let to_prune = session.messages.len() - keep_msg_count;
                let pruned_msgs: Vec<_> = session.messages.drain(0..to_prune).collect();
                messages_removed += pruned_msgs.len();

                let summary_text = format!(
                    "[Summary: {} historical messages summarized and pruned to conserve context window]",
                    pruned_msgs.len()
                );
                session.messages.insert(0, Message::user(summary_text));
            }
        }
        ContextPruningStrategy::DropOldestUserTurns { keep_recent_turns } => {
            let keep_msg_count = (keep_recent_turns * 2).max(2);
            if session.messages.len() > keep_msg_count {
                let to_drop = session.messages.len() - keep_msg_count;
                session.messages.drain(0..to_drop);
                messages_removed += to_drop;
            }
        }
        ContextPruningStrategy::SlidingWindow { max_messages } => {
            if session.messages.len() > max_messages {
                let to_remove = session.messages.len() - max_messages;
                session.messages.drain(0..to_remove);
                messages_removed += to_remove;
            }
        }
        ContextPruningStrategy::AdaptiveComposite => {
            // Stage 1: Truncate large tool outputs (over 1000 chars)
            for msg in &mut session.messages {
                if msg.role == Role::Tool && msg.content.len() > 1000 {
                    let prefix = &msg.content[..500];
                    let suffix = &msg.content[msg.content.len() - 500..];
                    msg.content = format!(
                        "{}\n\n... [{} characters truncated] ...\n\n{}",
                        prefix,
                        msg.content.len() - 1000,
                        suffix
                    );
                    tool_outputs_truncated += 1;
                }
            }

            let current_tokens = estimate_session_tokens(session);
            let freed_so_far = initial_tokens.saturating_sub(current_tokens);

            // Stage 2: Drop intermediate tool outputs if target not reached
            if freed_so_far < target_reduction {
                let total_msgs = session.messages.len();
                for (i, msg) in session.messages.iter_mut().enumerate() {
                    if i + 3 < total_msgs && msg.role == Role::Tool {
                        msg.content = "[Tool output pruned for context limit]".to_string();
                        tool_outputs_truncated += 1;
                    }
                }
            }

            let current_tokens_2 = estimate_session_tokens(session);
            let freed_so_far_2 = initial_tokens.saturating_sub(current_tokens_2);

            // Stage 3: Summarize oldest turns if still above budget
            if freed_so_far_2 < target_reduction && session.messages.len() > 6 {
                let to_prune = session.messages.len() - 6;
                let pruned = session.messages.drain(0..to_prune).count();
                messages_removed += pruned;
                session.messages.insert(
                    0,
                    Message::user(format!(
                        "[Prior conversation history ({} messages) summarized]",
                        pruned
                    )),
                );
            }
        }
    }

    let final_tokens = estimate_session_tokens(session);
    let tokens_freed = initial_tokens.saturating_sub(final_tokens);

    PruningReport {
        tokens_freed_est: tokens_freed,
        messages_removed,
        tool_outputs_truncated,
        summary: format!(
            "Context pruned: estimated {} tokens freed ({} messages removed, {} outputs truncated)",
            tokens_freed, messages_removed, tool_outputs_truncated
        ),
    }
}

// ============================================================================
// Exponential Backoff & Jitter Policy
// ============================================================================

/// Configuration for exponential backoff retry calculations with jitter.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BackoffPolicy {
    /// Initial retry delay.
    pub initial_delay: Duration,
    /// Maximum backoff ceiling.
    pub max_delay: Duration,
    /// Multiplier per subsequent attempt.
    pub multiplier: f64,
    /// Jitter variance factor (0.0 to 1.0, e.g. 0.2 = +/- 20% variance).
    pub jitter_factor: f64,
    /// Maximum retry attempts before giving up.
    pub max_retries: usize,
}

impl Default for BackoffPolicy {
    fn default() -> Self {
        Self {
            initial_delay: Duration::from_millis(500),
            max_delay: Duration::from_secs(30),
            multiplier: 2.0,
            jitter_factor: 0.2,
            max_retries: 3,
        }
    }
}

impl BackoffPolicy {
    /// Creates a custom `BackoffPolicy`.
    pub fn new(
        initial_delay: Duration,
        max_delay: Duration,
        multiplier: f64,
        jitter_factor: f64,
        max_retries: usize,
    ) -> Self {
        Self {
            initial_delay,
            max_delay,
            multiplier: multiplier.max(1.0),
            jitter_factor: jitter_factor.clamp(0.0, 1.0),
            max_retries,
        }
    }

    /// Calculates backoff delay for the given 1-based attempt index with pseudo-random jitter.
    pub fn calculate_delay(&self, attempt: usize) -> Duration {
        let seed = Uuid::new_v4().as_u128() as u64;
        self.calculate_delay_with_seed(attempt, seed)
    }

    /// Calculates backoff delay deterministically for testing with a given seed.
    pub fn calculate_delay_with_seed(&self, attempt: usize, seed: u64) -> Duration {
        let attempt_idx = (attempt.max(1) - 1) as i32;
        let factor = self.multiplier.powi(attempt_idx);
        let base_millis = (self.initial_delay.as_millis() as f64) * factor;
        let clamped_millis = base_millis.min(self.max_delay.as_millis() as f64);

        if self.jitter_factor > 0.0 {
            // Compute deterministic pseudo-random jitter in range [1.0 - jitter_factor, 1.0 + jitter_factor]
            let normalized_rand = ((seed % 1000) as f64) / 1000.0;
            let jitter_multiplier = (1.0 - self.jitter_factor) + (2.0 * self.jitter_factor * normalized_rand);
            let final_millis = (clamped_millis * jitter_multiplier).max(1.0);
            Duration::from_millis(final_millis as u64)
        } else {
            Duration::from_millis(clamped_millis as u64)
        }
    }
}

// ============================================================================
// Model Fallback Router
// ============================================================================

/// Intelligent fallback router for switching models during outages or rate limits.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FallbackRouter {
    /// Model to ordered fallback candidates map.
    routes: HashMap<String, Vec<String>>,
    /// Global default fallback chain when model has no specific mapping.
    default_chain: Vec<String>,
}

impl Default for FallbackRouter {
    fn default() -> Self {
        Self::with_default_routes()
    }
}

impl FallbackRouter {
    /// Creates an empty `FallbackRouter`.
    pub fn new() -> Self {
        Self {
            routes: HashMap::new(),
            default_chain: vec![
                "gpt-4o".to_string(),
                "claude-3-5-sonnet".to_string(),
                "deepseek-chat".to_string(),
                "gpt-4o-mini".to_string(),
            ],
        }
    }

    /// Creates a `FallbackRouter` populated with standard production fallback routes.
    pub fn with_default_routes() -> Self {
        let mut routes = HashMap::new();

        routes.insert(
            "claude-3-5-sonnet".to_string(),
            vec![
                "gpt-4o".to_string(),
                "deepseek-chat".to_string(),
                "gemini-1.5-pro".to_string(),
                "claude-3-5-haiku".to_string(),
            ],
        );

        routes.insert(
            "gpt-4o".to_string(),
            vec![
                "claude-3-5-sonnet".to_string(),
                "gpt-4o-mini".to_string(),
                "deepseek-chat".to_string(),
                "gemini-1.5-pro".to_string(),
            ],
        );

        routes.insert(
            "deepseek-chat".to_string(),
            vec![
                "deepseek-reasoner".to_string(),
                "gpt-4o-mini".to_string(),
                "claude-3-5-haiku".to_string(),
                "gemini-1.5-flash".to_string(),
            ],
        );

        routes.insert(
            "gemini-1.5-pro".to_string(),
            vec![
                "gpt-4o".to_string(),
                "claude-3-5-sonnet".to_string(),
                "gemini-1.5-flash".to_string(),
            ],
        );

        Self {
            routes,
            default_chain: vec![
                "gpt-4o".to_string(),
                "claude-3-5-sonnet".to_string(),
                "deepseek-chat".to_string(),
                "gpt-4o-mini".to_string(),
            ],
        }
    }

    /// Registers a fallback chain for a primary model.
    pub fn add_route(&mut self, primary: impl Into<String>, fallbacks: Vec<String>) {
        self.routes.insert(primary.into(), fallbacks);
    }

    /// Resolves the next candidate model that has not yet been attempted.
    pub fn get_fallback(&self, current_model: &str, tried_models: &[String]) -> Option<String> {
        if let Some(candidates) = self.routes.get(current_model) {
            for cand in candidates {
                if cand != current_model && !tried_models.contains(cand) {
                    return Some(cand.clone());
                }
            }
        }

        for cand in &self.default_chain {
            if cand != current_model && !tried_models.contains(cand) {
                return Some(cand.clone());
            }
        }

        None
    }
}

// ============================================================================
// FAULT TOLERANCE: Circuit Breaker Subsystem
// ============================================================================

/// Operational state of a circuit breaker.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CircuitState {
    /// Normal operation: requests pass through.
    Closed,
    /// Tripped / Open: requests fail fast to prevent cascading load.
    Open,
    /// Trial mode: probing with a limited number of test requests.
    HalfOpen,
}

/// Configuration thresholds for a `CircuitBreaker`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CircuitBreakerConfig {
    /// Number of consecutive failures to trip circuit from Closed to Open.
    pub failure_threshold: usize,
    /// Number of consecutive successes in HalfOpen to transition back to Closed.
    pub success_threshold: usize,
    /// Duration circuit remains Open before transitioning to HalfOpen.
    pub cooldown_duration: Duration,
}

impl Default for CircuitBreakerConfig {
    fn default() -> Self {
        Self {
            failure_threshold: 3,
            success_threshold: 2,
            cooldown_duration: Duration::from_secs(30),
        }
    }
}

/// Metrics counters for circuit breaker monitoring.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CircuitBreakerMetrics {
    pub total_calls: u64,
    pub successful_calls: u64,
    pub failed_calls: u64,
    pub trips_count: u64,
    pub consecutive_failures: usize,
    pub consecutive_successes: usize,
}

/// Robust state machine preventing cascading failures against troubled upstream services.
#[derive(Debug, Clone)]
pub struct CircuitBreaker {
    config: CircuitBreakerConfig,
    state: CircuitState,
    metrics: CircuitBreakerMetrics,
    last_failure_time: Option<Instant>,
    opened_at: Option<Instant>,
}

impl CircuitBreaker {
    /// Creates a new `CircuitBreaker` with the given configuration.
    pub fn new(config: CircuitBreakerConfig) -> Self {
        Self {
            config,
            state: CircuitState::Closed,
            metrics: CircuitBreakerMetrics::default(),
            last_failure_time: None,
            opened_at: None,
        }
    }

    /// Evaluates current state and checks if an execution is permitted.
    pub fn can_execute(&mut self) -> bool {
        self.update_state();
        match self.state {
            CircuitState::Closed | CircuitState::HalfOpen => true,
            CircuitState::Open => false,
        }
    }

    /// Returns the current state after checking cooldown expiration.
    pub fn state(&mut self) -> CircuitState {
        self.update_state();
        self.state
    }

    /// Records a successful execution.
    pub fn record_success(&mut self) {
        self.metrics.total_calls += 1;
        self.metrics.successful_calls += 1;
        self.metrics.consecutive_failures = 0;

        self.update_state();

        if self.state == CircuitState::HalfOpen {
            self.metrics.consecutive_successes += 1;
            if self.metrics.consecutive_successes >= self.config.success_threshold {
                self.state = CircuitState::Closed;
                self.metrics.consecutive_successes = 0;
                self.opened_at = None;
            }
        }
    }

    /// Records a failed execution.
    pub fn record_failure(&mut self) {
        self.metrics.total_calls += 1;
        self.metrics.failed_calls += 1;
        self.metrics.consecutive_failures += 1;
        self.metrics.consecutive_successes = 0;
        let now = Instant::now();
        self.last_failure_time = Some(now);

        self.update_state();

        match self.state {
            CircuitState::Closed => {
                if self.metrics.consecutive_failures >= self.config.failure_threshold {
                    self.state = CircuitState::Open;
                    self.opened_at = Some(now);
                    self.metrics.trips_count += 1;
                }
            }
            CircuitState::HalfOpen => {
                // Immediate trip back to Open on failure in HalfOpen
                self.state = CircuitState::Open;
                self.opened_at = Some(now);
                self.metrics.trips_count += 1;
            }
            CircuitState::Open => {}
        }
    }

    /// Returns remaining cooldown duration if currently in Open state.
    pub fn time_until_retry(&self) -> Option<Duration> {
        if self.state == CircuitState::Open {
            if let Some(opened) = self.opened_at {
                let elapsed = opened.elapsed();
                if elapsed < self.config.cooldown_duration {
                    return Some(self.config.cooldown_duration - elapsed);
                }
            }
        }
        None
    }

    /// Resets the circuit breaker to closed clean state.
    pub fn reset(&mut self) {
        self.state = CircuitState::Closed;
        self.metrics.consecutive_failures = 0;
        self.metrics.consecutive_successes = 0;
        self.opened_at = None;
        self.last_failure_time = None;
    }

    /// Returns metrics counters.
    pub fn metrics(&self) -> &CircuitBreakerMetrics {
        &self.metrics
    }

    fn update_state(&mut self) {
        if self.state == CircuitState::Open {
            if let Some(opened) = self.opened_at {
                if opened.elapsed() >= self.config.cooldown_duration {
                    self.state = CircuitState::HalfOpen;
                    self.metrics.consecutive_successes = 0;
                }
            }
        }
    }
}

/// Registry managing per-model and per-provider circuit breakers.
#[derive(Debug, Clone)]
pub struct CircuitBreakerRegistry {
    breakers: HashMap<String, CircuitBreaker>,
    default_config: CircuitBreakerConfig,
}

impl Default for CircuitBreakerRegistry {
    fn default() -> Self {
        Self::new(CircuitBreakerConfig::default())
    }
}

impl CircuitBreakerRegistry {
    /// Creates a new `CircuitBreakerRegistry`.
    pub fn new(default_config: CircuitBreakerConfig) -> Self {
        Self {
            breakers: HashMap::new(),
            default_config,
        }
    }

    /// Retrieves or instantiates a circuit breaker for the given key.
    pub fn get_or_create(&mut self, key: &str) -> &mut CircuitBreaker {
        let config = self.default_config.clone();
        self.breakers
            .entry(key.to_string())
            .or_insert_with(|| CircuitBreaker::new(config))
    }

    /// Checks if the target service/model is permitted to execute.
    pub fn can_execute(&mut self, key: &str) -> bool {
        self.get_or_create(key).can_execute()
    }

    /// Records success for key.
    pub fn record_success(&mut self, key: &str) {
        self.get_or_create(key).record_success();
    }

    /// Records failure for key.
    pub fn record_failure(&mut self, key: &str) {
        self.get_or_create(key).record_failure();
    }

    /// Resets all registered circuit breakers.
    pub fn reset_all(&mut self) {
        for cb in self.breakers.values_mut() {
            cb.reset();
        }
    }
}

// ============================================================================
// FAULT TOLERANCE: Recovery History Logging & Metrics
// ============================================================================

/// Terminal status of a recovery remediation attempt.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RecoveryStatus {
    /// Recovery in progress.
    Pending,
    /// Recovery successfully resolved the fault.
    Succeeded,
    /// Recovery failed to resolve the fault.
    Failed,
    /// Recovery was aborted or cancelled.
    Aborted,
    /// Escalated to user intervention.
    Escalated,
}

/// Audit log record for a single fault recovery attempt.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecoveryRecord {
    /// Unique attempt UUID.
    pub id: Uuid,
    /// Timestamp when fault occurred (RFC 3339).
    pub timestamp: String,
    /// Session identifier.
    pub session_id: Option<String>,
    /// Turn index.
    pub turn_index: usize,
    /// Active model during fault.
    pub model: String,
    /// Classified error category.
    pub error_class: ErrorClass,
    /// Raw error diagnostic string.
    pub raw_error: String,
    /// Remediation action executed.
    pub remediation: RemediationAction,
    /// Recovery outcome status.
    pub status: RecoveryStatus,
    /// Time spent resolving the fault in milliseconds.
    pub duration_ms: u64,
    /// Arbitrary metadata key-value tags.
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub metadata: HashMap<String, String>,
}

/// Aggregated recovery and fault tolerance performance statistics.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct RecoveryStats {
    /// Total fault occurrences recorded.
    pub total_faults: usize,
    /// Total completed recoveries.
    pub total_recoveries: usize,
    /// Total successful recoveries.
    pub successful_recoveries: usize,
    /// Total failed recovery attempts.
    pub failed_recoveries: usize,
    /// Recovery success rate (0.0 to 1.0).
    pub success_rate: f64,
    /// Distribution of faults by error class name.
    pub fault_distribution: HashMap<String, usize>,
    /// Distribution of remediation actions taken.
    pub remediation_distribution: HashMap<String, usize>,
    /// Mean time to recovery in milliseconds (MTTR).
    pub mean_time_to_recovery_ms: f64,
    /// Number of circuit breaker trip events.
    pub circuit_breaker_trips: usize,
}

/// Bounded in-memory history log of fault recovery attempts.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecoveryHistory {
    records: Vec<RecoveryRecord>,
    max_entries: usize,
}

impl Default for RecoveryHistory {
    fn default() -> Self {
        Self::new(1000)
    }
}

impl RecoveryHistory {
    /// Creates a new `RecoveryHistory` with maximum capacity bounds.
    pub fn new(max_entries: usize) -> Self {
        Self {
            records: Vec::new(),
            max_entries: max_entries.max(10),
        }
    }

    /// Logs the commencement of a fault recovery attempt.
    pub fn log_start(
        &mut self,
        session_id: Option<&str>,
        turn_index: usize,
        model: &str,
        error_class: ErrorClass,
        raw_error: &str,
        remediation: RemediationAction,
    ) -> Uuid {
        let id = Uuid::new_v4();
        let record = RecoveryRecord {
            id,
            timestamp: Utc::now().to_rfc3339(),
            session_id: session_id.map(|s| s.to_string()),
            turn_index,
            model: model.to_string(),
            error_class,
            raw_error: raw_error.to_string(),
            remediation,
            status: RecoveryStatus::Pending,
            duration_ms: 0,
            metadata: HashMap::new(),
        };

        if self.records.len() >= self.max_entries {
            self.records.remove(0);
        }
        self.records.push(record);
        id
    }

    /// Updates the terminal status and duration of a recovery record.
    pub fn record_outcome(&mut self, id: Uuid, status: RecoveryStatus, duration_ms: u64) {
        if let Some(record) = self.records.iter_mut().find(|r| r.id == id) {
            record.status = status;
            record.duration_ms = duration_ms;
        }
    }

    /// Calculates aggregated recovery statistics.
    pub fn get_stats(&self) -> RecoveryStats {
        let total_faults = self.records.len();
        let mut successful = 0;
        let mut failed = 0;
        let mut total_duration = 0u64;
        let mut fault_dist = HashMap::new();
        let mut rem_dist = HashMap::new();

        for rec in &self.records {
            *fault_dist.entry(rec.error_class.name().to_string()).or_insert(0) += 1;

            let action_name = match rec.remediation {
                RemediationAction::RetryWithBackoff { .. } => "RetryWithBackoff",
                RemediationAction::SwitchFallbackModel { .. } => "SwitchFallbackModel",
                RemediationAction::PruneContext { .. } => "PruneContext",
                RemediationAction::FixMalformedOutput { .. } => "FixMalformedOutput",
                RemediationAction::FailoverProvider { .. } => "FailoverProvider",
                RemediationAction::DisableTool { .. } => "DisableTool",
                RemediationAction::EscalateToUser { .. } => "EscalateToUser",
                RemediationAction::NoAction => "NoAction",
            };
            *rem_dist.entry(action_name.to_string()).or_insert(0) += 1;

            match rec.status {
                RecoveryStatus::Succeeded => {
                    successful += 1;
                    total_duration += rec.duration_ms;
                }
                RecoveryStatus::Failed | RecoveryStatus::Escalated => {
                    failed += 1;
                }
                _ => {}
            }
        }

        let total_finished = successful + failed;
        let success_rate = if total_finished > 0 {
            (successful as f64) / (total_finished as f64)
        } else {
            0.0
        };

        let mttr = if successful > 0 {
            (total_duration as f64) / (successful as f64)
        } else {
            0.0
        };

        RecoveryStats {
            total_faults,
            total_recoveries: total_finished,
            successful_recoveries: successful,
            failed_recoveries: failed,
            success_rate,
            fault_distribution: fault_dist,
            remediation_distribution: rem_dist,
            mean_time_to_recovery_ms: mttr,
            circuit_breaker_trips: 0,
        }
    }

    /// Returns a slice of all recorded recovery attempts.
    pub fn entries(&self) -> &[RecoveryRecord] {
        &self.records
    }

    /// Filters recovery records by error class name.
    pub fn filter_by_class(&self, class_name: &str) -> Vec<&RecoveryRecord> {
        self.records
            .iter()
            .filter(|r| r.error_class.name() == class_name)
            .collect()
    }

    /// Exports recovery history as formatted JSON.
    pub fn export_json(&self) -> Result<String, RecoveryError> {
        serde_json::to_string_pretty(&self.records).map_err(RecoveryError::Serialization)
    }

    /// Clears all recorded entries.
    pub fn clear(&mut self) {
        self.records.clear();
    }
}

// ============================================================================
// FAULT TOLERANCE: Self-Healing Agent Engine
// ============================================================================

/// Configuration parameters for the `FaultToleranceEngine`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FaultToleranceConfig {
    /// Maximum remediation attempts before escalating to user.
    pub max_remediation_retries: usize,
    /// Exponential backoff policy.
    pub backoff_policy: BackoffPolicy,
    /// Fallback model router.
    pub fallback_router: FallbackRouter,
    /// Circuit breaker parameters.
    pub circuit_breaker_config: CircuitBreakerConfig,
    /// Whether automatic context pruning is enabled.
    pub auto_prune_context: bool,
    /// Target token reduction when pruning context.
    pub target_prune_tokens: usize,
}

impl Default for FaultToleranceConfig {
    fn default() -> Self {
        Self {
            max_remediation_retries: 3,
            backoff_policy: BackoffPolicy::default(),
            fallback_router: FallbackRouter::default(),
            circuit_breaker_config: CircuitBreakerConfig::default(),
            auto_prune_context: true,
            target_prune_tokens: 4000,
        }
    }
}

/// Remediation decision output produced by `FaultToleranceEngine::evaluate_fault`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecoveryDecision {
    /// Unique identifier for this recovery attempt.
    pub attempt_id: Uuid,
    /// Classified error class.
    pub error_class: ErrorClass,
    /// Selected remediation action.
    pub remediation: RemediationAction,
    /// Whether this error represents a terminal condition requiring escalation.
    pub is_terminal: bool,
    /// Diagnostic explanation of the remediation decision.
    pub explanation: String,
}

/// Result of applying a remediation action to an active session.
#[derive(Debug, Clone)]
pub struct RemediationExecutionResult {
    /// Action description.
    pub action_taken: String,
    /// Model changed from -> to (if any).
    pub model_changed: Option<(String, String)>,
    /// Context pruning report (if any).
    pub context_pruned: Option<PruningReport>,
    /// Retry delay required (if any).
    pub retry_delay: Option<Duration>,
    /// Corrective instructions injected (if any).
    pub instructions_injected: Option<String>,
}

/// Unified self-healing agent engine managing error classification, remediation,
/// circuit breaking, model fallback, and history logging.
#[derive(Debug, Clone)]
pub struct FaultToleranceEngine {
    config: FaultToleranceConfig,
    history: RecoveryHistory,
    circuit_breakers: CircuitBreakerRegistry,
}

impl Default for FaultToleranceEngine {
    fn default() -> Self {
        Self::new(FaultToleranceConfig::default())
    }
}

impl FaultToleranceEngine {
    /// Creates a new `FaultToleranceEngine`.
    pub fn new(config: FaultToleranceConfig) -> Self {
        let cb_reg = CircuitBreakerRegistry::new(config.circuit_breaker_config.clone());
        Self {
            config,
            history: RecoveryHistory::default(),
            circuit_breakers: cb_reg,
        }
    }

    /// Evaluates a runtime error and determines the optimal remediation strategy.
    pub fn evaluate_fault(
        &mut self,
        error_msg: &str,
        current_model: &str,
        turn_index: usize,
        session: Option<&Session>,
        attempt_count: usize,
        tried_models: &[String],
    ) -> RecoveryDecision {
        let error_class = ErrorClass::classify(error_msg);
        let session_id_str = session.map(|s| s.id.to_string());

        // Update circuit breaker for current model on failure
        self.circuit_breakers.record_failure(current_model);

        // Select remediation action based on error classification and attempt count
        let (remediation, is_terminal, explanation) = match &error_class {
            ErrorClass::ContextLengthExceeded { .. } => {
                if self.config.auto_prune_context {
                    (
                        RemediationAction::PruneContext {
                            target_reduction_tokens: self.config.target_prune_tokens,
                            strategy: ContextPruningStrategy::AdaptiveComposite,
                            preserve_system_prompt: true,
                        },
                        false,
                        "Pruning session context window using adaptive composite pruning.".to_string(),
                    )
                } else if let Some(fallback) = self.config.fallback_router.get_fallback(current_model, tried_models) {
                    (
                        RemediationAction::SwitchFallbackModel {
                            primary_model: current_model.to_string(),
                            fallback_model: fallback.clone(),
                            reason: "Context window exceeded; switching to high-capacity fallback model.".to_string(),
                        },
                        false,
                        format!("Switching to fallback model '{}' for larger context window.", fallback),
                    )
                } else {
                    (
                        RemediationAction::EscalateToUser {
                            reason: "Context length exceeded and auto-pruning disabled.".to_string(),
                            diagnostic_report: error_msg.to_string(),
                        },
                        true,
                        "Context window exceeded with no available remediation.".to_string(),
                    )
                }
            }
            ErrorClass::RateLimit { retry_after, .. } => {
                if attempt_count < self.config.max_remediation_retries {
                    let delay = retry_after.unwrap_or_else(|| {
                        self.config.backoff_policy.calculate_delay(attempt_count + 1)
                    });
                    (
                        RemediationAction::RetryWithBackoff {
                            delay,
                            attempt: attempt_count + 1,
                            max_retries: self.config.max_remediation_retries,
                            jitter: true,
                        },
                        false,
                        format!("Rate limit encountered. Retrying in {:?} with backoff.", delay),
                    )
                } else if let Some(fallback) = self.config.fallback_router.get_fallback(current_model, tried_models) {
                    (
                        RemediationAction::SwitchFallbackModel {
                            primary_model: current_model.to_string(),
                            fallback_model: fallback.clone(),
                            reason: "Rate limit retry ceiling reached; falling back to alternative model.".to_string(),
                        },
                        false,
                        format!("Rate limit ceiling reached. Switching to fallback model '{}'.", fallback),
                    )
                } else {
                    (
                        RemediationAction::EscalateToUser {
                            reason: "Rate limit exhausted and all fallback models attempted.".to_string(),
                            diagnostic_report: error_msg.to_string(),
                        },
                        true,
                        "Persistent rate limit exhausted all retries and fallback models.".to_string(),
                    )
                }
            }
            ErrorClass::TransientNetwork { .. } => {
                if attempt_count < self.config.max_remediation_retries {
                    let delay = self.config.backoff_policy.calculate_delay(attempt_count + 1);
                    (
                        RemediationAction::RetryWithBackoff {
                            delay,
                            attempt: attempt_count + 1,
                            max_retries: self.config.max_remediation_retries,
                            jitter: true,
                        },
                        false,
                        format!("Transient network glitch. Retrying in {:?} (attempt {}/{}).", delay, attempt_count + 1, self.config.max_remediation_retries),
                    )
                } else if let Some(fallback) = self.config.fallback_router.get_fallback(current_model, tried_models) {
                    (
                        RemediationAction::SwitchFallbackModel {
                            primary_model: current_model.to_string(),
                            fallback_model: fallback.clone(),
                            reason: "Network connection persistently failing for primary model.".to_string(),
                        },
                        false,
                        format!("Network retries exhausted. Switching to fallback model '{}'.", fallback),
                    )
                } else {
                    (
                        RemediationAction::EscalateToUser {
                            reason: "Network connection persistently failed across all retry attempts.".to_string(),
                            diagnostic_report: error_msg.to_string(),
                        },
                        true,
                        "Network failure exhausted all retries.".to_string(),
                    )
                }
            }
            ErrorClass::InvalidModelOutput { reason, raw_snippet, .. } => {
                if attempt_count < 2 {
                    let repair_instructions = format!(
                        "Your previous response had formatting issues: {}. Please return strictly valid JSON conforming to the requested schema.",
                        reason
                    );
                    (
                        RemediationAction::FixMalformedOutput {
                            error_details: reason.clone(),
                            repair_instructions,
                        },
                        false,
                        "Injecting formatting correction prompt for malformed output self-repair.".to_string(),
                    )
                } else if let Some(fallback) = self.config.fallback_router.get_fallback(current_model, tried_models) {
                    (
                        RemediationAction::SwitchFallbackModel {
                            primary_model: current_model.to_string(),
                            fallback_model: fallback.clone(),
                            reason: "Model repeatedly produced invalid output schemas.".to_string(),
                        },
                        false,
                        format!("Model produced unparseable output. Switching to fallback model '{}'.", fallback),
                    )
                } else {
                    (
                        RemediationAction::EscalateToUser {
                            reason: format!("Model generated unfixable malformed output: {}", reason),
                            diagnostic_report: raw_snippet.clone().unwrap_or_else(|| error_msg.to_string()),
                        },
                        true,
                        "Malformed output recovery attempts exhausted.".to_string(),
                    )
                }
            }
            ErrorClass::ToolExecutionFailure { tool_name, error_message, is_recoverable } => {
                if *is_recoverable && attempt_count < self.config.max_remediation_retries {
                    (
                        RemediationAction::RetryWithBackoff {
                            delay: Duration::from_millis(200),
                            attempt: attempt_count + 1,
                            max_retries: self.config.max_remediation_retries,
                            jitter: false,
                        },
                        false,
                        format!("Tool '{}' failed. Retrying with short backoff.", tool_name),
                    )
                } else {
                    (
                        RemediationAction::DisableTool {
                            tool_name: tool_name.clone(),
                            alternative_tool: None,
                        },
                        false,
                        format!("Tool '{}' failed persistently: {}. Disabling tool.", tool_name, error_message),
                    )
                }
            }
            ErrorClass::AuthenticationOrQuota { reason, .. } => {
                if let Some(fallback) = self.config.fallback_router.get_fallback(current_model, tried_models) {
                    (
                        RemediationAction::SwitchFallbackModel {
                            primary_model: current_model.to_string(),
                            fallback_model: fallback.clone(),
                            reason: format!("Authentication/quota error: {}", reason),
                        },
                        false,
                        format!("Auth/quota error on primary model. Switching to fallback '{}'.", fallback),
                    )
                } else {
                    (
                        RemediationAction::EscalateToUser {
                            reason: "Authentication failure or quota exhausted.".to_string(),
                            diagnostic_report: reason.clone(),
                        },
                        true,
                        "Authentication or quota failure requires user intervention.".to_string(),
                    )
                }
            }
            ErrorClass::InternalServerError { message, .. } => {
                if attempt_count < 2 {
                    let delay = Duration::from_secs(1);
                    (
                        RemediationAction::RetryWithBackoff {
                            delay,
                            attempt: attempt_count + 1,
                            max_retries: 2,
                            jitter: true,
                        },
                        false,
                        format!("Upstream 500 error. Retrying in {:?}.", delay),
                    )
                } else if let Some(fallback) = self.config.fallback_router.get_fallback(current_model, tried_models) {
                    (
                        RemediationAction::SwitchFallbackModel {
                            primary_model: current_model.to_string(),
                            fallback_model: fallback.clone(),
                            reason: format!("Internal server error: {}", message),
                        },
                        false,
                        format!("Upstream error persists. Switching to fallback model '{}'.", fallback),
                    )
                } else {
                    (
                        RemediationAction::EscalateToUser {
                            reason: "Internal server error from upstream provider.".to_string(),
                            diagnostic_report: message.clone(),
                        },
                        true,
                        "Upstream provider failure exhausted retries.".to_string(),
                    )
                }
            }
            ErrorClass::Unknown { raw } => {
                if attempt_count < 1 {
                    (
                        RemediationAction::RetryWithBackoff {
                            delay: Duration::from_millis(500),
                            attempt: 1,
                            max_retries: 1,
                            jitter: true,
                        },
                        false,
                        "Unknown transient glitch. Attempting single retry.".to_string(),
                    )
                } else {
                    (
                        RemediationAction::EscalateToUser {
                            reason: "Unclassified error condition.".to_string(),
                            diagnostic_report: raw.clone(),
                        },
                        true,
                        "Unclassified error requires user inspection.".to_string(),
                    )
                }
            }
        };

        let attempt_id = self.history.log_start(
            session_id_str.as_deref(),
            turn_index,
            current_model,
            error_class.clone(),
            error_msg,
            remediation.clone(),
        );

        RecoveryDecision {
            attempt_id,
            error_class,
            remediation,
            is_terminal,
            explanation,
        }
    }

    /// Applies a remediation decision to the active session state.
    pub fn apply_remediation(
        &mut self,
        decision: &RecoveryDecision,
        session: &mut Session,
    ) -> Result<RemediationExecutionResult, RecoveryError> {
        match &decision.remediation {
            RemediationAction::RetryWithBackoff { delay, .. } => {
                Ok(RemediationExecutionResult {
                    action_taken: format!("Backoff pause for {:?}", delay),
                    model_changed: None,
                    context_pruned: None,
                    retry_delay: Some(*delay),
                    instructions_injected: None,
                })
            }
            RemediationAction::SwitchFallbackModel { primary_model, fallback_model, .. } => {
                session.active_model = fallback_model.clone();
                Ok(RemediationExecutionResult {
                    action_taken: format!("Switched model from {} to {}", primary_model, fallback_model),
                    model_changed: Some((primary_model.clone(), fallback_model.clone())),
                    context_pruned: None,
                    retry_delay: None,
                    instructions_injected: None,
                })
            }
            RemediationAction::PruneContext { target_reduction_tokens, strategy, .. } => {
                let report = prune_session_context(session, *target_reduction_tokens, strategy.clone());
                Ok(RemediationExecutionResult {
                    action_taken: report.summary.clone(),
                    model_changed: None,
                    context_pruned: Some(report),
                    retry_delay: None,
                    instructions_injected: None,
                })
            }
            RemediationAction::FixMalformedOutput { repair_instructions, .. } => {
                session.messages.push(Message::user(repair_instructions.clone()));
                Ok(RemediationExecutionResult {
                    action_taken: "Injected repair prompt into session messages.".to_string(),
                    model_changed: None,
                    context_pruned: None,
                    retry_delay: None,
                    instructions_injected: Some(repair_instructions.clone()),
                })
            }
            RemediationAction::FailoverProvider { from_provider, to_provider } => {
                Ok(RemediationExecutionResult {
                    action_taken: format!("Failover from provider {} to {}", from_provider, to_provider),
                    model_changed: None,
                    context_pruned: None,
                    retry_delay: None,
                    instructions_injected: None,
                })
            }
            RemediationAction::DisableTool { tool_name, .. } => {
                Ok(RemediationExecutionResult {
                    action_taken: format!("Disabled tool '{}' for subsequent turn attempts.", tool_name),
                    model_changed: None,
                    context_pruned: None,
                    retry_delay: None,
                    instructions_injected: None,
                })
            }
            RemediationAction::EscalateToUser { reason, diagnostic_report } => {
                Ok(RemediationExecutionResult {
                    action_taken: format!("Escalated to user: {} (diagnostic: {})", reason, diagnostic_report),
                    model_changed: None,
                    context_pruned: None,
                    retry_delay: None,
                    instructions_injected: None,
                })
            }
            RemediationAction::NoAction => {
                Ok(RemediationExecutionResult {
                    action_taken: "No action performed.".to_string(),
                    model_changed: None,
                    context_pruned: None,
                    retry_delay: None,
                    instructions_injected: None,
                })
            }
        }
    }

    /// Records the final outcome of a recovery remediation attempt.
    pub fn record_recovery_outcome(&mut self, attempt_id: Uuid, success: bool, duration_ms: u64) {
        let status = if success {
            RecoveryStatus::Succeeded
        } else {
            RecoveryStatus::Failed
        };
        self.history.record_outcome(attempt_id, status, duration_ms);
    }

    /// Checks if a model's circuit breaker allows execution.
    pub fn can_call_model(&mut self, model: &str) -> bool {
        self.circuit_breakers.can_execute(model)
    }

    /// Records model call success with the circuit breaker registry.
    pub fn record_model_success(&mut self, model: &str) {
        self.circuit_breakers.record_success(model);
    }

    /// Returns a reference to the recovery history.
    pub fn history(&self) -> &RecoveryHistory {
        &self.history
    }

    /// Returns a mutable reference to the recovery history.
    pub fn history_mut(&mut self) -> &mut RecoveryHistory {
        &mut self.history
    }

    /// Returns a reference to the circuit breaker registry.
    pub fn circuit_breakers(&self) -> &CircuitBreakerRegistry {
        &self.circuit_breakers
    }

    /// Returns a mutable reference to the circuit breaker registry.
    pub fn circuit_breakers_mut(&mut self) -> &mut CircuitBreakerRegistry {
        &mut self.circuit_breakers
    }

    /// Returns performance statistics.
    pub fn stats(&self) -> RecoveryStats {
        self.history.get_stats()
    }
}

// ============================================================================
// Internal Helpers
// ============================================================================

fn truncate_output(s: &str, max_chars: usize) -> String {
    if s.len() <= max_chars {
        s.to_string()
    } else {
        format!("{}... [truncated {} chars]", &s[..max_chars], s.len() - max_chars)
    }
}

fn args_preview_string(args: &serde_json::Value) -> String {
    match args {
        serde_json::Value::Object(map) => {
            if let Some(serde_json::Value::String(path)) = map.get("path") {
                format!("path: {}", path)
            } else if let Some(serde_json::Value::String(cmd)) = map.get("command") {
                format!("cmd: {}", cmd)
            } else {
                let s = args.to_string();
                truncate_output(&s, 64)
            }
        }
        _ => {
            let s = args.to_string();
            truncate_output(&s, 64)
        }
    }
}

fn get_hostname() -> Option<String> {
    std::env::var("HOSTNAME")
        .or_else(|_| std::env::var("HOST"))
        .ok()
}

// ============================================================================
// Unit & Integration Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn test_recovery_state_creation() {
        let session = Session::new("gpt-4o");
        let wd = PathBuf::from("/tmp/test_workspace");
        let state = RecoveryState::new_for_turn(&session, "Fix compiler error", 1, &wd);

        assert_eq!(state.user_input, "Fix compiler error");
        assert_eq!(state.turn_index, 1);
        assert_eq!(state.phase, TurnPhase::TurnStarted);
        assert!(!state.completed);
        assert_eq!(state.active_model, "gpt-4o");
        assert_eq!(state.working_dir, wd);
    }

    #[test]
    fn test_atomic_save_and_load() {
        let temp = tempdir().unwrap();
        let recovery_path = temp.path().join(".fusion").join("recovery.json");

        let mut session = Session::new("claude-3-5-sonnet");
        session.add_user_message("Hello world");
        let wd = temp.path().to_path_buf();

        let state = RecoveryState::new_for_turn(&session, "Refactor auth module", 2, &wd);

        let saved_path = save_recovery_state_atomic(&recovery_path, &state).unwrap();
        assert_eq!(saved_path, recovery_path);
        assert!(recovery_path.exists());

        let loaded = load_recovery_state(&recovery_path).unwrap();
        assert_eq!(loaded.session_id, session.id);
        assert_eq!(loaded.user_input, "Refactor auth module");
        assert_eq!(loaded.turn_index, 2);
        assert_eq!(loaded.phase, TurnPhase::TurnStarted);
        assert!(!loaded.completed);
    }

    #[test]
    fn test_crash_detection_when_uncompleted() {
        let temp = tempdir().unwrap();
        let wd = temp.path().to_path_buf();
        let mgr = RecoveryManager::new(&wd);

        let session = Session::new("deepseek-chat");
        mgr.on_turn_start(&session, "Deploy cloudflare worker", 1).unwrap();

        let crash = mgr.detect_crash();
        assert!(crash.is_some());
        let report = crash.unwrap();
        assert_eq!(report.user_input, "Deploy cloudflare worker");
        assert_eq!(report.turn_index, 1);
        assert_eq!(report.phase, TurnPhase::TurnStarted);

        mgr.on_turn_completed(&session).unwrap();
        let crash_after = mgr.detect_crash();
        assert!(crash_after.is_none());
    }

    #[test]
    fn test_tool_execution_tracking() {
        let temp = tempdir().unwrap();
        let wd = temp.path().to_path_buf();
        let mgr = RecoveryManager::new(&wd);

        let session = Session::new("gpt-4o");
        mgr.on_turn_start(&session, "Run unit tests", 1).unwrap();

        let args = serde_json::json!({ "command": "cargo test" });
        mgr.on_tool_start("bash", "call_123", &args).unwrap();

        let state = mgr.load_active_state().unwrap().unwrap();
        match state.phase {
            TurnPhase::ToolExecuting { tool_name, call_id, .. } => {
                assert_eq!(tool_name, "bash");
                assert_eq!(call_id, "call_123");
            }
            _ => panic!("Expected ToolExecuting phase"),
        }

        mgr.on_tool_finish(
            "bash",
            "call_123",
            "{\"command\": \"cargo test\"}",
            true,
            "test result: ok. 5 passed",
            Duration::from_millis(350),
        )
        .unwrap();

        let state_after = mgr.load_active_state().unwrap().unwrap();
        assert_eq!(state_after.completed_tools.len(), 1);
        assert_eq!(state_after.completed_tools[0].tool_name, "bash");
        assert!(state_after.completed_tools[0].success);
        assert_eq!(state_after.completed_tools[0].duration_ms, 350);
    }

    #[test]
    fn test_resume_continue_turn() {
        let temp = tempdir().unwrap();
        let wd = temp.path().to_path_buf();
        let mgr = RecoveryManager::new(&wd);

        let mut session = Session::new("gpt-4o");
        session.add_user_message("Initial task");
        mgr.on_turn_start(&session, "Continue task", 1).unwrap();

        mgr.on_tool_finish(
            "edit",
            "call_abc",
            "{\"path\": \"src/main.rs\"}",
            true,
            "File patched cleanly",
            Duration::from_millis(50),
        )
        .unwrap();

        let resume_res = mgr.resume(ResumeStrategy::ContinueTurn).unwrap();
        assert_eq!(resume_res.prompt_to_run, Some("Continue task".to_string()));
        assert_eq!(resume_res.strategy, ResumeStrategy::ContinueTurn);
        assert!(resume_res.session.messages.len() >= 2);
    }

    #[test]
    fn test_resume_replay_prompt() {
        let temp = tempdir().unwrap();
        let wd = temp.path().to_path_buf();
        let mgr = RecoveryManager::new(&wd);

        let mut session = Session::new("gpt-4o");
        session.add_user_message("Replay me");
        mgr.on_turn_start(&session, "Replay me", 1).unwrap();

        let resume_res = mgr.resume(ResumeStrategy::ReplayPrompt).unwrap();
        assert_eq!(resume_res.prompt_to_run, Some("Replay me".to_string()));
        assert_eq!(resume_res.strategy, ResumeStrategy::ReplayPrompt);
    }

    #[test]
    fn test_resume_restore_session_only() {
        let temp = tempdir().unwrap();
        let wd = temp.path().to_path_buf();
        let mgr = RecoveryManager::new(&wd);

        let mut session = Session::new("gpt-4o");
        session.add_user_message("Restore only");
        mgr.on_turn_start(&session, "Restore only", 1).unwrap();

        let resume_res = mgr.resume(ResumeStrategy::RestoreSessionOnly).unwrap();
        assert_eq!(resume_res.prompt_to_run, None);
        assert_eq!(resume_res.strategy, ResumeStrategy::RestoreSessionOnly);
    }

    #[test]
    fn test_resume_discard() {
        let temp = tempdir().unwrap();
        let wd = temp.path().to_path_buf();
        let mgr = RecoveryManager::new(&wd);

        let session = Session::new("gpt-4o");
        mgr.on_turn_start(&session, "Discard me", 1).unwrap();

        let resume_res = mgr.resume(ResumeStrategy::Discard).unwrap();
        assert_eq!(resume_res.strategy, ResumeStrategy::Discard);
        assert!(mgr.detect_crash().is_none());
    }

    #[test]
    fn test_turn_recovery_guard_drop_on_panic() {
        let temp = tempdir().unwrap();
        let wd = temp.path().to_path_buf();
        let mgr = RecoveryManager::new(&wd);

        let session = Session::new("gpt-4o");
        {
            let _guard = mgr.create_guard(&session, "Panic test", 1).unwrap();
            // Simulating drop before completion
        }

        let crash = mgr.detect_crash();
        assert!(crash.is_some());
        let report = crash.unwrap();
        assert_eq!(report.user_input, "Panic test");
    }

    #[test]
    fn test_turn_recovery_guard_mark_completed() {
        let temp = tempdir().unwrap();
        let wd = temp.path().to_path_buf();
        let mgr = RecoveryManager::new(&wd);

        let session = Session::new("gpt-4o");
        {
            let guard = mgr.create_guard(&session, "Complete test", 1).unwrap();
            guard.mark_completed(&session).unwrap();
        }

        let crash = mgr.detect_crash();
        assert!(crash.is_none());
    }

    #[test]
    fn test_slash_recover_commands() {
        let temp = tempdir().unwrap();
        let wd = temp.path().to_path_buf();
        let mut session = Session::new("gpt-4o");

        let status_out = handle_recovery_command("status", &wd, &mut session);
        assert!(status_out.contains("No crash recovery file") || status_out.contains("clean"));

        let help_out = handle_recovery_command("help", &wd, &mut session);
        assert!(help_out.contains("/recover"));
    }

    // ------------------------------------------------------------------------
    // Error Classification Tests
    // ------------------------------------------------------------------------

    #[test]
    fn test_error_classification_transient_network() {
        let errs = [
            "Connection reset by peer",
            "request timed out after 30000ms",
            "failed to lookup address information: nodename nor servname provided",
            "HTTP 502 Bad Gateway from Cloudflare",
            "503 Service Unavailable: upstream server busy",
            "504 Gateway Timeout",
            "broken pipe (os error 32)",
        ];

        for err in errs {
            let class = ErrorClass::classify(err);
            assert!(
                matches!(class, ErrorClass::TransientNetwork { .. }),
                "Expected TransientNetwork for '{}', got {:?}",
                err,
                class
            );
            assert!(class.is_retryable());
            assert_eq!(class.name(), "TransientNetwork");
        }
    }

    #[test]
    fn test_error_classification_rate_limit() {
        let errs = [
            "HTTP 429 Too Many Requests: Rate limit reached for TPM",
            "Rate limit exceeded: please retry after 5s",
            "You have exceeded your current quota, please check your plan and billing details",
            "tokens per minute limit reached",
            "RESOURCE_EXHAUSTED: quota exceeded for model",
        ];

        for err in errs {
            let class = ErrorClass::classify(err);
            assert!(
                matches!(class, ErrorClass::RateLimit { .. }),
                "Expected RateLimit for '{}', got {:?}",
                err,
                class
            );
            assert!(class.is_retryable());
            assert!(class.is_model_switch_recommended());
            assert_eq!(class.name(), "RateLimit");
        }

        // Test retry-after duration extraction
        let class_with_seconds = ErrorClass::classify("rate limit reached, retry after 15s");
        if let ErrorClass::RateLimit { retry_after, .. } = class_with_seconds {
            assert_eq!(retry_after, Some(Duration::from_secs(15)));
        } else {
            panic!("Expected RateLimit with duration");
        }
    }

    #[test]
    fn test_error_classification_context_length() {
        let errs = [
            "This model's maximum context length is 128000 tokens. However, your messages resulted in 135000 tokens.",
            "context_length_exceeded: prompt exceeds max_tokens limit",
            "KV cache capacity limit reached: please reduce the length of the messages",
            "input is too long for requested model context window",
        ];

        for err in errs {
            let class = ErrorClass::classify(err);
            assert!(
                matches!(class, ErrorClass::ContextLengthExceeded { .. }),
                "Expected ContextLengthExceeded for '{}', got {:?}",
                err,
                class
            );
            assert!(!class.is_retryable());
            assert!(class.is_context_pruning_recommended());
            assert_eq!(class.name(), "ContextLengthExceeded");
        }
    }

    #[test]
    fn test_error_classification_tool_failure() {
        let errs = [
            "Tool execution failed: cargo test exited with status code 101",
            "command failed: Permission denied (os error 13)",
            "Tool error: File not found at path 'src/foo.rs'",
            "Tool crashed unexpectedly",
        ];

        for err in errs {
            let class = ErrorClass::classify(err);
            assert!(
                matches!(class, ErrorClass::ToolExecutionFailure { .. }),
                "Expected ToolExecutionFailure for '{}', got {:?}",
                err,
                class
            );
            assert_eq!(class.name(), "ToolExecutionFailure");
        }
    }

    #[test]
    fn test_error_classification_invalid_model_output() {
        let errs = [
            "Failed to parse tool call: invalid json syntax at line 1 column 45",
            "Malformed JSON response from model: unexpected token",
            "Schema validation error: missing required parameter 'path'",
            "Empty response tokens received from model",
        ];

        for err in errs {
            let class = ErrorClass::classify(err);
            assert!(
                matches!(class, ErrorClass::InvalidModelOutput { .. }),
                "Expected InvalidModelOutput for '{}', got {:?}",
                err,
                class
            );
            assert_eq!(class.name(), "InvalidModelOutput");
        }
    }

    #[test]
    fn test_error_classification_http_codes() {
        assert_eq!(
            ErrorClass::classify_http(429, Some("rate limit")).name(),
            "RateLimit"
        );
        assert_eq!(
            ErrorClass::classify_http(502, Some("bad gateway")).name(),
            "TransientNetwork"
        );
        assert_eq!(
            ErrorClass::classify_http(500, Some("internal error")).name(),
            "InternalServerError"
        );
        assert_eq!(
            ErrorClass::classify_http(401, Some("invalid api key")).name(),
            "AuthenticationOrQuota"
        );
        assert_eq!(
            ErrorClass::classify_http(400, Some("context_length_exceeded")).name(),
            "ContextLengthExceeded"
        );
    }

    #[test]
    fn test_error_classification_json_payloads() {
        let openai_json = r#"{"error": {"code": "context_length_exceeded", "message": "Maximum context length exceeded"}}"#;
        assert_eq!(ErrorClass::classify(openai_json).name(), "ContextLengthExceeded");

        let rate_json = r#"{"error": {"code": "rate_limit_exceeded", "type": "rate_limit_error", "message": "RPM exceeded"}}"#;
        assert_eq!(ErrorClass::classify(rate_json).name(), "RateLimit");

        let auth_json = r#"{"error": {"code": "invalid_api_key", "message": "Incorrect API key provided"}}"#;
        assert_eq!(ErrorClass::classify(auth_json).name(), "AuthenticationOrQuota");
    }

    // ------------------------------------------------------------------------
    // Remediation and Backoff Tests
    // ------------------------------------------------------------------------

    #[test]
    fn test_backoff_policy_calculation_and_jitter() {
        let policy = BackoffPolicy::new(
            Duration::from_millis(100),
            Duration::from_secs(5),
            2.0,
            0.0, // No jitter for deterministic checks
            5,
        );

        assert_eq!(policy.calculate_delay_with_seed(1, 0), Duration::from_millis(100));
        assert_eq!(policy.calculate_delay_with_seed(2, 0), Duration::from_millis(200));
        assert_eq!(policy.calculate_delay_with_seed(3, 0), Duration::from_millis(400));
        assert_eq!(policy.calculate_delay_with_seed(4, 0), Duration::from_millis(800));

        // Test jitter variance within bounds
        let jitter_policy = BackoffPolicy::new(
            Duration::from_millis(1000),
            Duration::from_secs(10),
            2.0,
            0.2, // +/- 20%
            3,
        );

        for seed in [0, 250, 500, 750, 999] {
            let d = jitter_policy.calculate_delay_with_seed(1, seed);
            assert!(
                d >= Duration::from_millis(800) && d <= Duration::from_millis(1200),
                "Delay {:?} out of jitter bounds [800ms, 1200ms]",
                d
            );
        }
    }

    #[test]
    fn test_fallback_router() {
        let mut router = FallbackRouter::default();

        let fallback_1 = router.get_fallback("claude-3-5-sonnet", &[]);
        assert_eq!(fallback_1, Some("gpt-4o".to_string()));

        let fallback_2 = router.get_fallback("claude-3-5-sonnet", &["gpt-4o".to_string()]);
        assert_eq!(fallback_2, Some("deepseek-chat".to_string()));

        // Add custom route
        router.add_route("custom-llm", vec!["fallback-a".into(), "fallback-b".into()]);
        assert_eq!(router.get_fallback("custom-llm", &[]), Some("fallback-a".to_string()));
        assert_eq!(
            router.get_fallback("custom-llm", &["fallback-a".into()]),
            Some("fallback-b".to_string())
        );
    }

    // ------------------------------------------------------------------------
    // Context Pruning Tests
    // ------------------------------------------------------------------------

    #[test]
    fn test_context_pruning_token_estimation() {
        let msg = Message::user("A".repeat(400));
        let est = estimate_message_tokens(&msg);
        assert!(est >= 100 && est <= 110);
    }

    #[test]
    fn test_context_pruning_truncate_tool_outputs() {
        let mut session = Session::new("gpt-4o");
        session.messages.push(Message::user("Run command"));
        session.messages.push(Message::tool_result("call_1", "X".repeat(5000)));

        let initial = estimate_session_tokens(&session);
        let report = prune_session_context(
            &mut session,
            500,
            ContextPruningStrategy::TruncateLargeToolOutputs { max_output_chars: 500 },
        );

        let after = estimate_session_tokens(&session);
        assert!(after < initial);
        assert_eq!(report.tool_outputs_truncated, 1);
        assert!(session.messages[1].content.contains("characters omitted"));
    }

    #[test]
    fn test_context_pruning_drop_intermediate_tools() {
        let mut session = Session::new("gpt-4o");
        session.messages.push(Message::user("Task 1"));
        session.messages.push(Message::tool_result("call_1", "Output 1: ".to_string() + &"A".repeat(500)));
        session.messages.push(Message::user("Task 2"));
        session.messages.push(Message::tool_result("call_2", "Output 2: ".to_string() + &"B".repeat(500)));

        let report = prune_session_context(
            &mut session,
            100,
            ContextPruningStrategy::DropIntermediateToolOutputs,
        );

        assert!(report.tool_outputs_truncated >= 1);
        assert!(session.messages[1].content.contains("pruned to fit context"));
    }

    #[test]
    fn test_context_pruning_summarize_turns() {
        let mut session = Session::new("gpt-4o");
        for i in 1..=10 {
            session.messages.push(Message::user(format!("Question {}", i)));
            session.messages.push(Message::assistant(format!("Answer {}", i)));
        }

        let report = prune_session_context(
            &mut session,
            500,
            ContextPruningStrategy::SummarizeOldestTurns { keep_recent_turns: 2 },
        );

        assert!(report.messages_removed > 0);
        assert!(session.messages[0].content.contains("Summary:"));
        assert!(session.messages.len() <= 5);
    }

    #[test]
    fn test_context_pruning_adaptive_composite() {
        let mut session = Session::new("gpt-4o");
        session.messages.push(Message::user("Initial task"));
        for i in 1..=8 {
            session.messages.push(Message::tool_result(
                format!("call_{}", i),
                format!("Tool output {} with long content: {}", i, "Z".repeat(2000)),
            ));
        }

        let report = prune_session_context(
            &mut session,
            2000,
            ContextPruningStrategy::AdaptiveComposite,
        );

        assert!(report.tokens_freed_est > 0);
        assert!(report.tool_outputs_truncated > 0 || report.messages_removed > 0);
    }

    // ------------------------------------------------------------------------
    // Circuit Breaker Tests
    // ------------------------------------------------------------------------

    #[test]
    fn test_circuit_breaker_transitions() {
        let config = CircuitBreakerConfig {
            failure_threshold: 3,
            success_threshold: 2,
            cooldown_duration: Duration::from_millis(50),
        };

        let mut cb = CircuitBreaker::new(config);
        assert_eq!(cb.state(), CircuitState::Closed);
        assert!(cb.can_execute());

        // 1. Record 2 failures -> remains Closed
        cb.record_failure();
        cb.record_failure();
        assert_eq!(cb.state(), CircuitState::Closed);
        assert!(cb.can_execute());

        // 2. Record 3rd failure -> trips to Open
        cb.record_failure();
        assert_eq!(cb.state(), CircuitState::Open);
        assert!(!cb.can_execute());
        assert!(cb.time_until_retry().is_some());

        // 3. Wait for cooldown expiration -> transitions to HalfOpen
        std::thread::sleep(Duration::from_millis(60));
        assert_eq!(cb.state(), CircuitState::HalfOpen);
        assert!(cb.can_execute());

        // 4. Record 1 success -> still HalfOpen
        cb.record_success();
        assert_eq!(cb.state(), CircuitState::HalfOpen);

        // 5. Record 2nd success -> transitions back to Closed
        cb.record_success();
        assert_eq!(cb.state(), CircuitState::Closed);
        assert!(cb.can_execute());
    }

    #[test]
    fn test_circuit_breaker_half_open_failure() {
        let config = CircuitBreakerConfig {
            failure_threshold: 2,
            success_threshold: 2,
            cooldown_duration: Duration::from_millis(20),
        };

        let mut cb = CircuitBreaker::new(config);
        cb.record_failure();
        cb.record_failure();
        assert_eq!(cb.state(), CircuitState::Open);

        std::thread::sleep(Duration::from_millis(25));
        assert_eq!(cb.state(), CircuitState::HalfOpen);

        // Failure in HalfOpen trips immediately back to Open
        cb.record_failure();
        assert_eq!(cb.state(), CircuitState::Open);
        assert!(!cb.can_execute());
    }

    #[test]
    fn test_circuit_breaker_registry() {
        let mut registry = CircuitBreakerRegistry::default();

        assert!(registry.can_execute("claude-3-5-sonnet"));
        registry.record_failure("claude-3-5-sonnet");
        registry.record_failure("claude-3-5-sonnet");
        registry.record_failure("claude-3-5-sonnet");

        assert!(!registry.can_execute("claude-3-5-sonnet"));
        // Sibling model remains unaffected
        assert!(registry.can_execute("gpt-4o"));

        registry.reset_all();
        assert!(registry.can_execute("claude-3-5-sonnet"));
    }

    // ------------------------------------------------------------------------
    // Recovery History & Stats Tests
    // ------------------------------------------------------------------------

    #[test]
    fn test_recovery_history_logging_and_stats() {
        let mut history = RecoveryHistory::new(100);

        let id1 = history.log_start(
            Some("session-1"),
            1,
            "gpt-4o",
            ErrorClass::classify("Rate limit 429"),
            "Rate limit 429",
            RemediationAction::RetryWithBackoff {
                delay: Duration::from_secs(1),
                attempt: 1,
                max_retries: 3,
                jitter: true,
            },
        );

        history.record_outcome(id1, RecoveryStatus::Succeeded, 1200);

        let id2 = history.log_start(
            Some("session-1"),
            2,
            "gpt-4o",
            ErrorClass::classify("context_length_exceeded"),
            "context_length_exceeded",
            RemediationAction::PruneContext {
                target_reduction_tokens: 2000,
                strategy: ContextPruningStrategy::AdaptiveComposite,
                preserve_system_prompt: true,
            },
        );

        history.record_outcome(id2, RecoveryStatus::Succeeded, 800);

        let stats = history.get_stats();
        assert_eq!(stats.total_faults, 2);
        assert_eq!(stats.successful_recoveries, 2);
        assert_eq!(stats.failed_recoveries, 0);
        assert_eq!(stats.success_rate, 1.0);
        assert_eq!(stats.mean_time_to_recovery_ms, 1000.0);
        assert_eq!(history.filter_by_class("RateLimit").len(), 1);
        assert_eq!(history.filter_by_class("ContextLengthExceeded").len(), 1);

        let json = history.export_json().unwrap();
        assert!(json.contains("session-1"));
        assert!(json.contains("gpt-4o"));
    }

    // ------------------------------------------------------------------------
    // Fault Tolerance Engine End-to-End Tests
    // ------------------------------------------------------------------------

    #[test]
    fn test_fault_tolerance_engine_transient_retry() {
        let mut engine = FaultToleranceEngine::default();
        let mut session = Session::new("gpt-4o");

        let decision = engine.evaluate_fault(
            "HTTP 502 Bad Gateway",
            "gpt-4o",
            1,
            Some(&session),
            0,
            &[],
        );

        assert_eq!(decision.error_class.name(), "TransientNetwork");
        assert!(!decision.is_terminal);
        assert!(matches!(
            decision.remediation,
            RemediationAction::RetryWithBackoff { attempt: 1, .. }
        ));

        let res = engine.apply_remediation(&decision, &mut session).unwrap();
        assert!(res.retry_delay.is_some());
    }

    #[test]
    fn test_fault_tolerance_engine_context_pruning() {
        let mut engine = FaultToleranceEngine::default();
        let mut session = Session::new("gpt-4o");
        session.messages.push(Message::user("Hello"));
        session.messages.push(Message::tool_result("call_1", "L".repeat(5000)));

        let decision = engine.evaluate_fault(
            "maximum context length exceeded: please reduce tokens",
            "gpt-4o",
            1,
            Some(&session),
            0,
            &[],
        );

        assert_eq!(decision.error_class.name(), "ContextLengthExceeded");
        assert!(matches!(
            decision.remediation,
            RemediationAction::PruneContext { .. }
        ));

        let res = engine.apply_remediation(&decision, &mut session).unwrap();
        assert!(res.context_pruned.is_some());
    }

    #[test]
    fn test_fault_tolerance_engine_fallback_model_switch() {
        let mut engine = FaultToleranceEngine::default();
        let mut session = Session::new("claude-3-5-sonnet");

        // Retries exhausted (attempt = 3) for rate limit
        let decision = engine.evaluate_fault(
            "429 rate limit exceeded",
            "claude-3-5-sonnet",
            1,
            Some(&session),
            3,
            &["claude-3-5-sonnet".to_string()],
        );

        assert_eq!(decision.error_class.name(), "RateLimit");
        match &decision.remediation {
            RemediationAction::SwitchFallbackModel { fallback_model, .. } => {
                assert_eq!(fallback_model, "gpt-4o");
            }
            _ => panic!("Expected SwitchFallbackModel remediation"),
        }

        let res = engine.apply_remediation(&decision, &mut session).unwrap();
        assert_eq!(session.active_model, "gpt-4o");
        assert_eq!(
            res.model_changed,
            Some(("claude-3-5-sonnet".to_string(), "gpt-4o".to_string()))
        );
    }

    #[test]
    fn test_fault_tolerance_engine_malformed_output_repair() {
        let mut engine = FaultToleranceEngine::default();
        let mut session = Session::new("gpt-4o");

        let decision = engine.evaluate_fault(
            "failed to parse tool call: invalid JSON syntax at line 1",
            "gpt-4o",
            1,
            Some(&session),
            0,
            &[],
        );

        assert_eq!(decision.error_class.name(), "InvalidModelOutput");
        assert!(matches!(
            decision.remediation,
            RemediationAction::FixMalformedOutput { .. }
        ));

        let res = engine.apply_remediation(&decision, &mut session).unwrap();
        assert!(res.instructions_injected.is_some());
        assert!(session.messages.last().unwrap().content.contains("formatting issues"));
    }

    #[test]
    fn test_fault_tolerance_engine_circuit_trip_and_escalate() {
        let mut engine = FaultToleranceEngine::default();
        let mut session = Session::new("gpt-4o");

        // Auth failure with no fallback available
        let decision = engine.evaluate_fault(
            "HTTP 401 Unauthorized: Invalid API key",
            "gpt-4o",
            1,
            Some(&session),
            0,
            &["gpt-4o".into(), "claude-3-5-sonnet".into(), "deepseek-chat".into(), "gpt-4o-mini".into()],
        );

        assert_eq!(decision.error_class.name(), "AuthenticationOrQuota");
        assert!(decision.is_terminal);
        assert!(matches!(
            decision.remediation,
            RemediationAction::EscalateToUser { .. }
        ));
    }
}
