//! Rich Agent Client Protocol (ACP) Event Streaming Engine.
//!
//! Provides granular session update events, token-by-token streaming, tool status tracking,
//! advisor feedback lifecycles, and bidirectional JSON-RPC 2.0 notification bridging for IDEs
//! and external editor clients (e.g. Zed, JetBrains, Neovim, VS Code).

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use tokio::sync::broadcast;
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender};

use crate::acp::types::{
    ContentBlock, JsonRpcNotification, SessionUpdate, SessionUpdateParams,
};
use crate::agent::loop_runner::AgentEvent;

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

// ============================================================================
// Token Streaming Payloads
// ============================================================================

/// Granular token chunk emitted during token-by-token LLM generation.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct TokenStreamChunk {
    /// Monotonically increasing sequence index within the turn.
    pub index: u64,
    /// Incremental token or text slice.
    pub delta: String,
    /// Whether this is the first token of the turn.
    pub is_first: bool,
    /// Whether this is the final token of the turn.
    pub is_last: bool,
    /// Running total of tokens emitted so far in this turn.
    pub total_tokens: u64,
    /// Milliseconds since Unix epoch.
    pub timestamp_ms: u64,
}

/// Incremental model thinking / reasoning chunk.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ThinkingStreamChunk {
    /// Monotonically increasing sequence index for reasoning.
    pub index: u64,
    /// Incremental reasoning thought slice.
    pub delta: String,
    /// Elapsed reasoning time in milliseconds.
    pub elapsed_ms: u64,
    /// Milliseconds since Unix epoch.
    pub timestamp_ms: u64,
}

/// Token usage and throughput statistics.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct TokenUsageStats {
    /// Tokens consumed by input prompt and system context.
    pub prompt_tokens: u64,
    /// Tokens produced by assistant completion.
    pub completion_tokens: u64,
    /// Total tokens (prompt + completion).
    pub total_tokens: u64,
    /// Prompt tokens retrieved from provider KV cache if supported.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cached_tokens: Option<u64>,
    /// Generation throughput in tokens per second.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tokens_per_second: Option<f64>,
    /// Elapsed turn duration in milliseconds.
    pub duration_ms: u64,
}

// ============================================================================
// Tool Status Payloads
// ============================================================================

/// Execution phase for an invoked tool.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ToolExecutionState {
    /// Tool execution scheduled or queued.
    Pending,
    /// Tool is actively executing.
    Running,
    /// Tool is streaming partial output chunks.
    Streaming,
    /// Tool execution finished successfully.
    Completed,
    /// Tool execution failed with an error.
    Failed,
    /// Tool execution was cancelled or aborted.
    Aborted,
}

impl std::fmt::Display for ToolExecutionState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Pending => write!(f, "pending"),
            Self::Running => write!(f, "running"),
            Self::Streaming => write!(f, "streaming"),
            Self::Completed => write!(f, "completed"),
            Self::Failed => write!(f, "failed"),
            Self::Aborted => write!(f, "aborted"),
        }
    }
}

/// Detailed status update for a tool call.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ToolStatusUpdate {
    /// Unique identifier matching across all events for this tool call.
    pub call_id: String,
    /// Name of the tool (e.g. `bash`, `read_file`, `git_diff`).
    pub name: String,
    /// Current execution state.
    pub state: ToolExecutionState,
    /// Human-readable status line (e.g. "Executing `cargo check` in /src").
    pub status: String,
    /// Tool call input arguments (when starting).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub args: Option<serde_json::Value>,
    /// Progress fraction between 0.0 and 1.0 if measurable.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub progress: Option<f64>,
    /// Streaming partial output chunk if active.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub partial_output: Option<String>,
    /// Final output content upon completion.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output: Option<String>,
    /// Execution time in milliseconds.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub duration_ms: Option<u64>,
    /// Success indicator upon completion.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub success: Option<bool>,
    /// Detailed error message if failed.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    /// Milliseconds since Unix epoch.
    pub timestamp_ms: u64,
}

// ============================================================================
// Advisor Feedback Payloads
// ============================================================================

/// Severity classification for advisor reviews.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AdvisorSeverity {
    /// Purely informational observations or style tips.
    Info,
    /// Noticeable suggestion that does not block execution.
    Notice,
    /// Cautionary warning (e.g. performance regression, deprecation).
    Warning,
    /// Blocking issue (e.g. compilation failure, syntax bug).
    Error,
    /// Critical hazard (e.g. credential leak, dangerous command, security hole).
    Critical,
}

impl std::fmt::Display for AdvisorSeverity {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Info => write!(f, "info"),
            Self::Notice => write!(f, "notice"),
            Self::Warning => write!(f, "warning"),
            Self::Error => write!(f, "error"),
            Self::Critical => write!(f, "critical"),
        }
    }
}

/// Lifecycle state for an advisor review.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AdvisorStatusState {
    /// Review has started.
    Started,
    /// Advisor is actively evaluating proposed actions.
    Evaluating,
    /// Review completed and proposed action is approved.
    Approved,
    /// Review completed and proposed action is rejected.
    Rejected,
}

/// Rich feedback emitted by an automated advisor.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct AdvisorFeedbackUpdate {
    /// Identifier or name of the advisor (e.g. `security`, `architecture`, `git`).
    pub advisor: String,
    /// Advisor role description (e.g. "Security Auditor", "Code Architect").
    pub role: String,
    /// Lifecycle state of this review.
    pub state: AdvisorStatusState,
    /// Whether the proposed action is approved.
    pub approved: bool,
    /// Severity classification.
    pub severity: AdvisorSeverity,
    /// Full critique feedback text.
    pub critique: String,
    /// Structured list of actionable recommendations parsed from the critique.
    pub suggestions: Vec<String>,
    /// Advisor confidence score between 0.0 and 1.0 if provided.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub confidence: Option<f32>,
    /// Milliseconds since Unix epoch.
    pub timestamp_ms: u64,
}

/// Aggregate consensus across multiple automated advisors.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AdvisorConsensus {
    /// Total advisors that contributed a critique.
    pub total_advisors: usize,
    /// Number of approving reviews.
    pub approved_count: usize,
    /// Number of rejecting reviews.
    pub rejected_count: usize,
    /// Number of cautionary warnings.
    pub warning_count: usize,
    /// Final consensus determination (true if approved by all or policy passed).
    pub overall_approved: bool,
    /// Summary explanation of the collective decision.
    pub summary: String,
}

// ============================================================================
// Subagent and Plan Payloads
// ============================================================================

/// Subagent execution status update.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SubagentStatusUpdate {
    /// Name or identifier of the subagent.
    pub name: String,
    /// Task assigned to the subagent.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub task: Option<String>,
    /// Current execution status (`started`, `running`, `finished`, `error`).
    pub status: String,
    /// Output produced by the subagent if finished.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output: Option<String>,
    /// Success indicator if finished.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub success: Option<bool>,
    /// Milliseconds since Unix epoch.
    pub timestamp_ms: u64,
}

/// Individual step within a multi-step execution plan.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct PlanStep {
    /// 0-based step index.
    pub index: usize,
    /// Description of the planned step.
    pub description: String,
    /// Step status: `pending`, `in_progress`, `completed`, `failed`, `skipped`.
    pub status: String,
}

/// Multi-step execution plan update.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct PlanProgressUpdate {
    /// All planned steps and their current states.
    pub steps: Vec<PlanStep>,
    /// Currently active step index if one is in progress.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub current_step_index: Option<usize>,
    /// Number of completed steps.
    pub completed_count: usize,
    /// Total steps in the plan.
    pub total_count: usize,
}

// ============================================================================
// Rich ACP Session Event Enumeration
// ============================================================================

/// Strongly-typed top-level ACP event encompassing all stream updates.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "eventType", content = "data", rename_all = "snake_case")]
pub enum AcpSessionEvent {
    /// Incremental token chunk during assistant answer generation.
    TokenChunk(TokenStreamChunk),
    /// Incremental reasoning thought chunk.
    ThinkingChunk(ThinkingStreamChunk),
    /// Tool execution scheduled or started.
    ToolStarted(ToolStatusUpdate),
    /// Tool execution progress or partial output.
    ToolProgress(ToolStatusUpdate),
    /// Tool execution finished.
    ToolCompleted(ToolStatusUpdate),
    /// Advisor review initialized.
    AdvisorStarted(AdvisorFeedbackUpdate),
    /// Advisor critique received.
    AdvisorFeedback(AdvisorFeedbackUpdate),
    /// Collective advisor consensus evaluation.
    AdvisorConsensus(AdvisorConsensus),
    /// Real-time token usage and throughput update.
    TokenStats(TokenUsageStats),
    /// Subagent execution lifecycle event.
    Subagent(SubagentStatusUpdate),
    /// Execution plan progress.
    Plan(PlanProgressUpdate),
    /// Informational or diagnostic status notification.
    Status {
        message: String,
        level: String,
        timestamp_ms: u64,
    },
    /// Execution error or warning.
    Error {
        error: String,
        recoverable: bool,
        timestamp_ms: u64,
    },
}

impl AcpSessionEvent {
    /// Convenience constructor for a token chunk.
    pub fn token(
        index: u64,
        delta: impl Into<String>,
        is_first: bool,
        is_last: bool,
        total_tokens: u64,
    ) -> Self {
        Self::TokenChunk(TokenStreamChunk {
            index,
            delta: delta.into(),
            is_first,
            is_last,
            total_tokens,
            timestamp_ms: now_ms(),
        })
    }

    /// Convenience constructor for a reasoning thought chunk.
    pub fn thought(index: u64, delta: impl Into<String>, elapsed_ms: u64) -> Self {
        Self::ThinkingChunk(ThinkingStreamChunk {
            index,
            delta: delta.into(),
            elapsed_ms,
            timestamp_ms: now_ms(),
        })
    }

    /// Convenience constructor for a tool start event.
    pub fn tool_started(
        call_id: impl Into<String>,
        name: impl Into<String>,
        args: serde_json::Value,
    ) -> Self {
        let name_str = name.into();
        let status = format!("Starting tool `{}`", name_str);
        Self::ToolStarted(ToolStatusUpdate {
            call_id: call_id.into(),
            name: name_str,
            state: ToolExecutionState::Running,
            status,
            args: Some(args),
            progress: Some(0.0),
            partial_output: None,
            output: None,
            duration_ms: None,
            success: None,
            error: None,
            timestamp_ms: now_ms(),
        })
    }

    /// Convenience constructor for a tool progress event.
    pub fn tool_progress(
        call_id: impl Into<String>,
        name: impl Into<String>,
        status: impl Into<String>,
        progress: Option<f64>,
        partial_output: Option<String>,
    ) -> Self {
        Self::ToolProgress(ToolStatusUpdate {
            call_id: call_id.into(),
            name: name.into(),
            state: ToolExecutionState::Streaming,
            status: status.into(),
            args: None,
            progress,
            partial_output,
            output: None,
            duration_ms: None,
            success: None,
            error: None,
            timestamp_ms: now_ms(),
        })
    }

    /// Convenience constructor for a tool completion event.
    pub fn tool_completed(
        call_id: impl Into<String>,
        name: impl Into<String>,
        success: bool,
        output: impl Into<String>,
        duration_ms: u64,
    ) -> Self {
        let name_str = name.into();
        let output_str = output.into();
        let status = if success {
            format!("Tool `{}` completed in {}ms", name_str, duration_ms)
        } else {
            format!("Tool `{}` failed after {}ms", name_str, duration_ms)
        };
        Self::ToolCompleted(ToolStatusUpdate {
            call_id: call_id.into(),
            name: name_str,
            state: if success {
                ToolExecutionState::Completed
            } else {
                ToolExecutionState::Failed
            },
            status,
            args: None,
            progress: Some(1.0),
            partial_output: None,
            output: Some(output_str),
            duration_ms: Some(duration_ms),
            success: Some(success),
            error: if success {
                None
            } else {
                Some("Tool execution failed".to_string())
            },
            timestamp_ms: now_ms(),
        })
    }

    /// Convenience constructor for an advisor start event.
    pub fn advisor_started(advisor: impl Into<String>, role: impl Into<String>) -> Self {
        let adv = advisor.into();
        let r = role.into();
        Self::AdvisorStarted(AdvisorFeedbackUpdate {
            advisor: adv,
            role: r,
            state: AdvisorStatusState::Started,
            approved: false,
            severity: AdvisorSeverity::Info,
            critique: String::new(),
            suggestions: Vec::new(),
            confidence: None,
            timestamp_ms: now_ms(),
        })
    }

    /// Convenience constructor for an advisor critique event.
    pub fn advisor_feedback(
        advisor: impl Into<String>,
        role: impl Into<String>,
        approved: bool,
        severity: AdvisorSeverity,
        critique: impl Into<String>,
        suggestions: Vec<String>,
    ) -> Self {
        let critique_str = critique.into();
        Self::AdvisorFeedback(AdvisorFeedbackUpdate {
            advisor: advisor.into(),
            role: role.into(),
            state: if approved {
                AdvisorStatusState::Approved
            } else {
                AdvisorStatusState::Rejected
            },
            approved,
            severity,
            critique: critique_str,
            suggestions,
            confidence: Some(1.0),
            timestamp_ms: now_ms(),
        })
    }

    /// Converts this rich event into standard ACP `SessionUpdate` compatible with all ACP clients.
    pub fn to_session_update(&self) -> SessionUpdate {
        match self {
            Self::TokenChunk(chunk) => SessionUpdate::AgentMessageChunk {
                content: ContentBlock::text(&chunk.delta),
                index: Some(chunk.index),
                is_first: Some(chunk.is_first),
                is_last: Some(chunk.is_last),
            },
            Self::ThinkingChunk(thought) => SessionUpdate::AgentThoughtChunk {
                content: ContentBlock::text(&thought.delta),
                thought: Some(thought.delta.clone()),
                index: Some(thought.index),
                elapsed_ms: Some(thought.elapsed_ms),
            },
            Self::ToolStarted(tool) => SessionUpdate::ToolCall {
                call_id: tool.call_id.clone(),
                name: tool.name.clone(),
                args: tool.args.clone().unwrap_or(serde_json::json!({})),
                status: Some(tool.status.clone()),
            },
            Self::ToolProgress(tool) => SessionUpdate::ToolStatus {
                call_id: tool.call_id.clone(),
                name: tool.name.clone(),
                status: tool.status.clone(),
                progress: tool.progress,
                partial_output: tool.partial_output.clone(),
            },
            Self::ToolCompleted(tool) => SessionUpdate::ToolCallResult {
                call_id: tool.call_id.clone(),
                name: tool.name.clone(),
                output: tool.output.clone().unwrap_or_default(),
                success: tool.success.unwrap_or(true),
                duration_ms: tool.duration_ms,
                error: tool.error.clone(),
            },
            Self::AdvisorStarted(adv) => SessionUpdate::AdvisorStarted {
                advisor: adv.advisor.clone(),
                role: adv.role.clone(),
            },
            Self::AdvisorFeedback(adv) => SessionUpdate::AdvisorCritique {
                advisor: adv.advisor.clone(),
                approved: adv.approved,
                critique: adv.critique.clone(),
                role: Some(adv.role.clone()),
                severity: Some(adv.severity.to_string()),
                suggestions: if adv.suggestions.is_empty() {
                    None
                } else {
                    Some(adv.suggestions.clone())
                },
            },
            Self::AdvisorConsensus(consensus) => SessionUpdate::Status {
                message: format!(
                    "Advisor Consensus: {} ({}/{} approved)",
                    consensus.summary, consensus.approved_count, consensus.total_advisors
                ),
                level: Some(if consensus.overall_approved {
                    "info".to_string()
                } else {
                    "warning".to_string()
                }),
            },
            Self::TokenStats(stats) => SessionUpdate::TokenStats {
                prompt_tokens: stats.prompt_tokens,
                completion_tokens: stats.completion_tokens,
                total_tokens: stats.total_tokens,
                cached_tokens: stats.cached_tokens,
                tokens_per_second: stats.tokens_per_second,
            },
            Self::Subagent(sub) => SessionUpdate::SubagentUpdate {
                name: sub.name.clone(),
                status: sub.status.clone(),
                task: sub.task.clone(),
                output: sub.output.clone(),
            },
            Self::Plan(plan) => {
                let steps = plan.steps.iter().map(|s| s.description.clone()).collect();
                SessionUpdate::Plan { steps }
            }
            Self::Status { message, level, .. } => SessionUpdate::Status {
                message: message.clone(),
                level: Some(level.clone()),
            },
            Self::Error { error, .. } => SessionUpdate::Status {
                message: format!("Error: {}", error),
                level: Some("error".to_string()),
            },
        }
    }

    /// Packages this event into an ACP JSON-RPC 2.0 `session/update` notification.
    pub fn to_jsonrpc_notification(&self, session_id: &str) -> JsonRpcNotification {
        let update = self.to_session_update();
        let mut update_val = serde_json::to_value(&update).unwrap_or(serde_json::Value::Null);
        // Ensure both "sessionUpdate" and "kind" are populated for maximum client compatibility
        if let serde_json::Value::Object(map) = &mut update_val {
            if let Some(tag) = map.get("sessionUpdate").cloned() {
                map.insert("kind".to_string(), tag);
            } else if let Some(tag) = map.get("kind").cloned() {
                map.insert("sessionUpdate".to_string(), tag);
            }
        }
        let params = serde_json::json!({
            "sessionId": session_id,
            "update": update_val,
        });

        JsonRpcNotification::new("session/update", params)
    }

    /// Serializes this event into a JSON-RPC notification payload string.
    pub fn to_jsonrpc_line(&self, session_id: &str) -> Option<String> {
        let notif = self.to_jsonrpc_notification(session_id);
        serde_json::to_string(&notif).ok()
    }
}

// ============================================================================
// Advisor Feedback Aggregator
// ============================================================================

/// Analyzes and aggregates advisor critiques across multiple advisory agents.
#[derive(Debug, Clone, Default)]
pub struct AdvisorFeedbackAggregator {
    reviews: Vec<AdvisorFeedbackUpdate>,
    active_advisors: HashMap<String, String>, // advisor -> role
}

impl AdvisorFeedbackAggregator {
    /// Creates a new aggregator.
    pub fn new() -> Self {
        Self::default()
    }

    /// Records that an advisor review has initiated.
    pub fn record_started(&mut self, advisor: &str, role: &str) -> AdvisorFeedbackUpdate {
        self.active_advisors
            .insert(advisor.to_string(), role.to_string());
        AdvisorFeedbackUpdate {
            advisor: advisor.to_string(),
            role: role.to_string(),
            state: AdvisorStatusState::Started,
            approved: false,
            severity: AdvisorSeverity::Info,
            critique: String::new(),
            suggestions: Vec::new(),
            confidence: None,
            timestamp_ms: now_ms(),
        }
    }

    /// Records an advisor critique, parsing recommendations and classifying severity.
    pub fn record_critique(
        &mut self,
        advisor: &str,
        approved: bool,
        critique: &str,
    ) -> AdvisorFeedbackUpdate {
        let role = self
            .active_advisors
            .get(advisor)
            .cloned()
            .unwrap_or_else(|| "Advisor".to_string());

        let suggestions = Self::extract_suggestions(critique);
        let severity = Self::infer_severity(approved, critique);

        let update = AdvisorFeedbackUpdate {
            advisor: advisor.to_string(),
            role,
            state: if approved {
                AdvisorStatusState::Approved
            } else {
                AdvisorStatusState::Rejected
            },
            approved,
            severity,
            critique: critique.to_string(),
            suggestions,
            confidence: Some(1.0),
            timestamp_ms: now_ms(),
        };

        self.reviews.push(update.clone());
        update
    }

    /// Extracts bulleted or enumerated recommendations from raw critique text.
    pub fn extract_suggestions(critique: &str) -> Vec<String> {
        let mut suggestions = Vec::new();
        for line in critique.lines() {
            let trimmed = line.trim();
            if trimmed.starts_with("- ") || trimmed.starts_with("* ") || trimmed.starts_with("+ ") {
                let mut text = trimmed[2..].trim();
                if text.to_lowercase().starts_with("suggestion:") {
                    text = text["suggestion:".len()..].trim();
                } else if text.to_lowercase().starts_with("recommendation:") {
                    text = text["recommendation:".len()..].trim();
                }
                if !text.is_empty() {
                    suggestions.push(text.to_string());
                }
            } else if let Some(idx) = trimmed.find(". ") {
                let prefix = &trimmed[..idx];
                if prefix.chars().all(|c| c.is_ascii_digit()) && !prefix.is_empty() {
                    let text = trimmed[idx + 2..].trim();
                    if !text.is_empty() {
                        suggestions.push(text.to_string());
                    }
                }
            } else if trimmed.to_lowercase().starts_with("suggestion:") {
                let text = trimmed["suggestion:".len()..].trim();
                if !text.is_empty() {
                    suggestions.push(text.to_string());
                }
            } else if trimmed.to_lowercase().starts_with("recommendation:") {
                let text = trimmed["recommendation:".len()..].trim();
                if !text.is_empty() {
                    suggestions.push(text.to_string());
                }
            }
        }
        suggestions
    }

    /// Infers critique severity based on approval status and keywords.
    pub fn infer_severity(approved: bool, critique: &str) -> AdvisorSeverity {
        let lower = critique.to_lowercase();
        if lower.contains("critical")
            || lower.contains("security hazard")
            || lower.contains("vulnerability")
            || lower.contains("fatal")
        {
            AdvisorSeverity::Critical
        } else if !approved {
            if lower.contains("warning") || lower.contains("caution") {
                AdvisorSeverity::Warning
            } else {
                AdvisorSeverity::Error
            }
        } else if lower.contains("warning")
            || lower.contains("caution")
            || lower.contains("perf")
            || lower.contains("deprecated")
        {
            AdvisorSeverity::Warning
        } else if lower.contains("note:") || lower.contains("notice:") {
            AdvisorSeverity::Notice
        } else {
            AdvisorSeverity::Info
        }
    }

    /// Computes consensus summary if any reviews have been gathered.
    pub fn compute_consensus(&self) -> Option<AdvisorConsensus> {
        if self.reviews.is_empty() {
            return None;
        }

        let total = self.reviews.len();
        let approved_count = self.reviews.iter().filter(|r| r.approved).count();
        let rejected_count = total - approved_count;
        let warning_count = self
            .reviews
            .iter()
            .filter(|r| {
                matches!(
                    r.severity,
                    AdvisorSeverity::Warning | AdvisorSeverity::Critical
                )
            })
            .count();

        let overall_approved = rejected_count == 0;
        let summary = if overall_approved {
            if warning_count > 0 {
                format!("Approved with {} warning(s)", warning_count)
            } else {
                "All advisors approved without objections".to_string()
            }
        } else {
            format!(
                "{}/{} advisors rejected proposed actions",
                rejected_count, total
            )
        };

        Some(AdvisorConsensus {
            total_advisors: total,
            approved_count,
            rejected_count,
            warning_count,
            overall_approved,
            summary,
        })
    }

    /// Returns recorded feedback reviews.
    pub fn reviews(&self) -> &[AdvisorFeedbackUpdate] {
        &self.reviews
    }
}

// ============================================================================
// Event Broadcaster
// ============================================================================

/// Pub/sub event broadcaster allowing multiple consumers (IDEs, UI monitors, loggers)
/// to receive a cloned stream of live ACP session events.
#[derive(Clone)]
pub struct AcpEventBroadcaster {
    sender: Arc<broadcast::Sender<AcpSessionEvent>>,
}

impl Default for AcpEventBroadcaster {
    fn default() -> Self {
        Self::new(256)
    }
}

impl AcpEventBroadcaster {
    /// Creates a broadcaster with the specified channel capacity.
    pub fn new(capacity: usize) -> Self {
        let (tx, _rx) = broadcast::channel(capacity.max(16));
        Self {
            sender: Arc::new(tx),
        }
    }

    /// Subscribes to the broadcast stream.
    pub fn subscribe(&self) -> broadcast::Receiver<AcpSessionEvent> {
        self.sender.subscribe()
    }

    /// Emits an event to all active subscribers. Returns subscriber count.
    pub fn send(&self, event: AcpSessionEvent) -> usize {
        self.sender.send(event).unwrap_or(0)
    }
}

// ============================================================================
// ACP Stream Bridge Engine
// ============================================================================

/// Execution summary returned upon completion of an ACP prompt streaming turn.
#[derive(Debug, Clone, Default)]
pub struct AcpBridgeSummary {
    /// Complete accumulated assistant text.
    pub full_assistant_text: String,
    /// Complete accumulated thinking/reasoning text.
    pub full_thought_text: String,
    /// Prompt tokens consumed.
    pub prompt_tokens: u64,
    /// Completion tokens produced.
    pub completion_tokens: u64,
    /// Total tokens (prompt + completion).
    pub total_tokens: u64,
    /// Total stream turn duration in milliseconds.
    pub duration_ms: u64,
    /// Output generation speed (tokens per second).
    pub tokens_per_second: f64,
    /// Milliseconds elapsed before the first assistant token was received.
    pub time_to_first_token_ms: Option<u64>,
    /// History of all tool calls executed during the turn.
    pub tool_calls: Vec<ToolStatusUpdate>,
    /// History of all advisor feedback updates gathered during the turn.
    pub advisor_reviews: Vec<AdvisorFeedbackUpdate>,
    /// Collective advisor consensus if advisors participated.
    pub advisor_consensus: Option<AdvisorConsensus>,
}

/// The core ACP streaming engine bridging Fusion `AgentEvent`s to ACP session updates.
///
/// Manages:
/// - Token-by-token emission with monotonic sequence indices and throughput tracking
/// - Thinking/reasoning stream chunking with elapsed time
/// - Tool execution lifecycle tracking (started -> progress -> finished) with consistent call IDs
/// - Multi-advisor feedback ingestion, suggestion extraction, and consensus calculation
/// - Direct formatting and dispatch into ACP JSON-RPC 2.0 stdio/socket sender
pub struct AcpEventBridge {
    session_id: String,
    out_tx: Option<UnboundedSender<String>>,
    broadcaster: Option<AcpEventBroadcaster>,
    advisor_aggregator: AdvisorFeedbackAggregator,
    active_tools: HashMap<String, (String, Instant)>, // call_id -> (name, start_instant)
    recorded_tools: Vec<ToolStatusUpdate>,
    full_assistant_text: String,
    full_thought_text: String,
    token_index: u64,
    thought_index: u64,
    start_time: Instant,
    first_token_time: Option<Instant>,
    last_token_time: Option<Instant>,
    prompt_tokens: u64,
    completion_tokens: u64,
}

impl AcpEventBridge {
    /// Creates a new bridge for the specified session ID.
    pub fn new(session_id: impl Into<String>) -> Self {
        Self {
            session_id: session_id.into(),
            out_tx: None,
            broadcaster: None,
            advisor_aggregator: AdvisorFeedbackAggregator::new(),
            active_tools: HashMap::new(),
            recorded_tools: Vec::new(),
            full_assistant_text: String::new(),
            full_thought_text: String::new(),
            token_index: 0,
            thought_index: 0,
            start_time: Instant::now(),
            first_token_time: None,
            last_token_time: None,
            prompt_tokens: 0,
            completion_tokens: 0,
        }
    }

    /// Sets the JSON-RPC string transmitter to stream directly to editor client.
    pub fn with_out_sender(mut self, tx: UnboundedSender<String>) -> Self {
        self.out_tx = Some(tx);
        self
    }

    /// Sets an optional event broadcaster for external telemetry or debugging.
    pub fn with_broadcaster(mut self, broadcaster: AcpEventBroadcaster) -> Self {
        self.broadcaster = Some(broadcaster);
        self
    }

    /// Transforms a Fusion `AgentEvent` into zero or more rich `AcpSessionEvent`s.
    pub fn handle_agent_event(&mut self, event: AgentEvent) -> Vec<AcpSessionEvent> {
        let mut events = Vec::new();

        match event {
            AgentEvent::TextDelta(delta) => {
                if !delta.is_empty() {
                    let now = Instant::now();
                    if self.first_token_time.is_none() {
                        self.first_token_time = Some(now);
                    }
                    self.last_token_time = Some(now);

                    let is_first = self.token_index == 0;
                    self.token_index += 1;
                    self.completion_tokens += 1;
                    self.full_assistant_text.push_str(&delta);

                    events.push(AcpSessionEvent::TokenChunk(TokenStreamChunk {
                        index: self.token_index,
                        delta,
                        is_first,
                        is_last: false,
                        total_tokens: self.completion_tokens,
                        timestamp_ms: now_ms(),
                    }));
                }
            }
            AgentEvent::ThinkingDelta(thought) => {
                if !thought.is_empty() {
                    self.thought_index += 1;
                    self.full_thought_text.push_str(&thought);
                    let elapsed_ms = self.start_time.elapsed().as_millis() as u64;

                    events.push(AcpSessionEvent::ThinkingChunk(ThinkingStreamChunk {
                        index: self.thought_index,
                        delta: thought,
                        elapsed_ms,
                        timestamp_ms: now_ms(),
                    }));
                }
            }
            AgentEvent::ToolStarted { id, name, args } => {
                let call_id = if id.is_empty() {
                    format!("tool_{}_{}", name, uuid::Uuid::new_v4().simple())
                } else {
                    id
                };

                self.active_tools
                    .insert(call_id.clone(), (name.clone(), Instant::now()));

                let status = format!("Running tool `{}`", name);
                let update = ToolStatusUpdate {
                    call_id,
                    name,
                    state: ToolExecutionState::Running,
                    status,
                    args: Some(args),
                    progress: Some(0.0),
                    partial_output: None,
                    output: None,
                    duration_ms: None,
                    success: None,
                    error: None,
                    timestamp_ms: now_ms(),
                };
                events.push(AcpSessionEvent::ToolStarted(update));
            }
            AgentEvent::ToolFinished {
                id,
                name,
                success,
                output,
                duration,
            } => {
                let call_id = if id.is_empty() {
                    // Try to match by tool name in active tools
                    self.active_tools
                        .iter()
                        .find(|(_, (n, _))| n == &name)
                        .map(|(k, _)| k.clone())
                        .unwrap_or_else(|| format!("tool_{}", name))
                } else {
                    id
                };

                let explicit_dur = duration.as_millis() as u64;
                let computed_duration_ms =
                    if let Some((_, start)) = self.active_tools.remove(&call_id) {
                        if explicit_dur > 0 {
                            explicit_dur
                        } else {
                            start.elapsed().as_millis() as u64
                        }
                    } else {
                        explicit_dur
                    };

                let status = if success {
                    format!("Tool `{}` completed in {}ms", name, computed_duration_ms)
                } else {
                    format!("Tool `{}` failed after {}ms", name, computed_duration_ms)
                };

                let update = ToolStatusUpdate {
                    call_id,
                    name,
                    state: if success {
                        ToolExecutionState::Completed
                    } else {
                        ToolExecutionState::Failed
                    },
                    status,
                    args: None,
                    progress: Some(1.0),
                    partial_output: None,
                    output: Some(output),
                    duration_ms: Some(computed_duration_ms),
                    success: Some(success),
                    error: if success {
                        None
                    } else {
                        Some("Tool execution failed".to_string())
                    },
                    timestamp_ms: now_ms(),
                };

                self.recorded_tools.push(update.clone());
                events.push(AcpSessionEvent::ToolCompleted(update));
            }
            AgentEvent::AdvisorStarted { advisor, role } => {
                let update = self.advisor_aggregator.record_started(&advisor, &role);
                events.push(AcpSessionEvent::AdvisorStarted(update));
            }
            AgentEvent::AdvisorCritique {
                advisor,
                approved,
                critique,
            } => {
                let update = self
                    .advisor_aggregator
                    .record_critique(&advisor, approved, &critique);
                events.push(AcpSessionEvent::AdvisorFeedback(update));

                // If multiple advisors reviewed, update consensus
                if let Some(consensus) = self.advisor_aggregator.compute_consensus() {
                    events.push(AcpSessionEvent::AdvisorConsensus(consensus));
                }
            }
            AgentEvent::SubagentStarted { name, task } => {
                events.push(AcpSessionEvent::Subagent(SubagentStatusUpdate {
                    name,
                    task: Some(task),
                    status: "started".to_string(),
                    output: None,
                    success: None,
                    timestamp_ms: now_ms(),
                }));
            }
            AgentEvent::SubagentFinished {
                name,
                success,
                output,
            } => {
                events.push(AcpSessionEvent::Subagent(SubagentStatusUpdate {
                    name,
                    task: None,
                    status: if success {
                        "completed".to_string()
                    } else {
                        "failed".to_string()
                    },
                    output: Some(output),
                    success: Some(success),
                    timestamp_ms: now_ms(),
                }));
            }
            AgentEvent::Status(message) => {
                events.push(AcpSessionEvent::Status {
                    message,
                    level: "info".to_string(),
                    timestamp_ms: now_ms(),
                });
            }
            AgentEvent::Error(err) => {
                events.push(AcpSessionEvent::Error {
                    error: err,
                    recoverable: true,
                    timestamp_ms: now_ms(),
                });
            }
            AgentEvent::Finished { usage } => {
                // Parse token usage if available from model
                if let Some(val) = usage {
                    if let Some(p) = val.get("prompt_tokens").and_then(|v| v.as_u64()) {
                        self.prompt_tokens = p;
                    }
                    if let Some(c) = val.get("completion_tokens").and_then(|v| v.as_u64()) {
                        self.completion_tokens = c;
                    }
                }

                let total = self.prompt_tokens + self.completion_tokens;
                let duration_ms = self.start_time.elapsed().as_millis() as u64;
                let tps = if duration_ms > 0 {
                    Some((self.completion_tokens as f64) / (duration_ms as f64 / 1000.0))
                } else {
                    None
                };

                events.push(AcpSessionEvent::TokenStats(TokenUsageStats {
                    prompt_tokens: self.prompt_tokens,
                    completion_tokens: self.completion_tokens,
                    total_tokens: total,
                    cached_tokens: None,
                    tokens_per_second: tps,
                    duration_ms,
                }));
            }
        }

        events
    }

    /// Emits an event by sending it to the client output stream and broadcasting it.
    pub fn emit(&mut self, event: &AcpSessionEvent) {
        if let Some(broadcaster) = &self.broadcaster {
            broadcaster.send(event.clone());
        }

        if let Some(tx) = &self.out_tx {
            if let Some(line) = event.to_jsonrpc_line(&self.session_id) {
                let _ = tx.send(line);
            }
        }
    }

    /// Consumes the entire `AgentEvent` stream, converting events to ACP notifications,
    /// and produces an `AcpBridgeSummary` when the stream is exhausted.
    pub async fn run(mut self, mut rx: UnboundedReceiver<AgentEvent>) -> AcpBridgeSummary {
        while let Some(agent_event) = rx.recv().await {
            let acp_events = self.handle_agent_event(agent_event);
            for event in acp_events {
                self.emit(&event);
            }
        }

        // Finalize summary
        let duration_ms = self.start_time.elapsed().as_millis() as u64;
        let ttft_ms = self
            .first_token_time
            .map(|t| t.duration_since(self.start_time).as_millis() as u64);

        let tps = if duration_ms > 0 && self.completion_tokens > 0 {
            (self.completion_tokens as f64) / (duration_ms as f64 / 1000.0)
        } else {
            0.0
        };

        let total_tokens = self.prompt_tokens + self.completion_tokens;
        let consensus = self.advisor_aggregator.compute_consensus();

        AcpBridgeSummary {
            full_assistant_text: self.full_assistant_text,
            full_thought_text: self.full_thought_text,
            prompt_tokens: self.prompt_tokens,
            completion_tokens: self.completion_tokens,
            total_tokens,
            duration_ms,
            tokens_per_second: tps,
            time_to_first_token_ms: ttft_ms,
            tool_calls: self.recorded_tools,
            advisor_reviews: self.advisor_aggregator.reviews().to_vec(),
            advisor_consensus: consensus,
        }
    }

    /// Access the current accumulated assistant message.
    pub fn accumulated_text(&self) -> &str {
        &self.full_assistant_text
    }

    /// Access the current accumulated reasoning thoughts.
    pub fn accumulated_thought(&self) -> &str {
        &self.full_thought_text
    }

    /// Count of tokens emitted so far.
    pub fn token_count(&self) -> u64 {
        self.token_index
    }
}

// ============================================================================
// Unit Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::sync::mpsc::unbounded_channel;

    #[test]
    fn test_token_stream_chunk_serialization() {
        let chunk = TokenStreamChunk {
            index: 1,
            delta: "Hello".to_string(),
            is_first: true,
            is_last: false,
            total_tokens: 1,
            timestamp_ms: 1700000000000,
        };

        let event = AcpSessionEvent::TokenChunk(chunk.clone());
        let update = event.to_session_update();

        match update {
            SessionUpdate::AgentMessageChunk {
                content,
                index,
                is_first,
                is_last,
            } => {
                assert_eq!(content.text, Some("Hello".to_string()));
                assert_eq!(index, Some(1));
                assert_eq!(is_first, Some(true));
                assert_eq!(is_last, Some(false));
            }
            _ => panic!("Expected AgentMessageChunk"),
        }

        let notif = event.to_jsonrpc_notification("sess-123");
        assert_eq!(notif.method, "session/update");
        assert_eq!(notif.jsonrpc, "2.0");

        let json_val = serde_json::to_value(&notif).unwrap();
        assert_eq!(json_val["params"]["sessionId"], "sess-123");
        assert_eq!(json_val["params"]["update"]["sessionUpdate"], "agent_message_chunk");
        assert_eq!(json_val["params"]["update"]["kind"], "agent_message_chunk");
        assert_eq!(
            json_val["params"]["update"]["content"]["text"],
            "Hello"
        );
    }

    #[test]
    fn test_thinking_stream_chunk_conversion() {
        let event = AcpSessionEvent::thought(42, "Analyzing problem...", 120);
        let update = event.to_session_update();

        match update {
            SessionUpdate::AgentThoughtChunk {
                thought,
                content,
                index,
                elapsed_ms,
            } => {
                assert_eq!(thought.as_deref(), Some("Analyzing problem..."));
                assert_eq!(content.text.as_deref(), Some("Analyzing problem..."));
                assert_eq!(index, Some(42));
                assert_eq!(elapsed_ms, Some(120));
            }
            _ => panic!("Expected AgentThoughtChunk"),
        }
    }

    #[test]
    fn test_tool_lifecycle_status_and_id_tracking() {
        let mut bridge = AcpEventBridge::new("sess-tools");

        // 1. Tool started
        let start_ev = AgentEvent::ToolStarted {
            id: "call_git_status".to_string(),
            name: "git".to_string(),
            args: serde_json::json!({ "command": "status" }),
        };

        let res = bridge.handle_agent_event(start_ev);
        assert_eq!(res.len(), 1);
        match &res[0] {
            AcpSessionEvent::ToolStarted(tool) => {
                assert_eq!(tool.call_id, "call_git_status");
                assert_eq!(tool.name, "git");
                assert_eq!(tool.state, ToolExecutionState::Running);
                assert!(tool.status.contains("git"));
            }
            _ => panic!("Expected ToolStarted"),
        }

        // 2. Tool finished
        let finish_ev = AgentEvent::ToolFinished {
            id: "call_git_status".to_string(),
            name: "git".to_string(),
            success: true,
            output: "On branch main\nnothing to commit".to_string(),
            duration: Duration::from_millis(35),
        };

        let res = bridge.handle_agent_event(finish_ev);
        assert_eq!(res.len(), 1);
        match &res[0] {
            AcpSessionEvent::ToolCompleted(tool) => {
                assert_eq!(tool.call_id, "call_git_status");
                assert_eq!(tool.name, "git");
                assert_eq!(tool.state, ToolExecutionState::Completed);
                assert_eq!(tool.success, Some(true));
                assert!(tool.duration_ms.is_some());
                assert_eq!(
                    tool.output.as_deref(),
                    Some("On branch main\nnothing to commit")
                );
            }
            _ => panic!("Expected ToolCompleted"),
        }
    }

    #[test]
    fn test_advisor_feedback_lifecycle_and_consensus() {
        let mut bridge = AcpEventBridge::new("sess-advisors");

        // 1. Advisor started
        let start_ev = AgentEvent::AdvisorStarted {
            advisor: "security".to_string(),
            role: "Security Auditor".to_string(),
        };
        let res = bridge.handle_agent_event(start_ev);
        assert_eq!(res.len(), 1);
        match &res[0] {
            AcpSessionEvent::AdvisorStarted(adv) => {
                assert_eq!(adv.advisor, "security");
                assert_eq!(adv.role, "Security Auditor");
                assert_eq!(adv.state, AdvisorStatusState::Started);
            }
            _ => panic!("Expected AdvisorStarted"),
        }

        // 2. Advisor critique with suggestions
        let critique_ev = AgentEvent::AdvisorCritique {
            advisor: "security".to_string(),
            approved: true,
            critique: "LGTM. Safe code execution.\n- Suggestion: Sanitize environment variables\n- Use read-only filesystem where possible".to_string(),
        };
        let res = bridge.handle_agent_event(critique_ev);
        assert!(res.len() >= 2); // AdvisorFeedback + AdvisorConsensus

        match &res[0] {
            AcpSessionEvent::AdvisorFeedback(adv) => {
                assert_eq!(adv.advisor, "security");
                assert_eq!(adv.role, "Security Auditor");
                assert!(adv.approved);
                assert_eq!(adv.suggestions.len(), 2);
                assert_eq!(adv.suggestions[0], "Sanitize environment variables");
                assert_eq!(
                    adv.suggestions[1],
                    "Use read-only filesystem where possible"
                );
            }
            _ => panic!("Expected AdvisorFeedback"),
        }

        match &res[1] {
            AcpSessionEvent::AdvisorConsensus(consensus) => {
                assert_eq!(consensus.total_advisors, 1);
                assert_eq!(consensus.approved_count, 1);
                assert!(consensus.overall_approved);
            }
            _ => panic!("Expected AdvisorConsensus"),
        }
    }

    #[test]
    fn test_advisor_suggestion_and_severity_inference() {
        let critique_critical = "CRITICAL: Detected hardcoded private key.\n1. Remove private key immediately\n2. Rotate credentials";
        let suggestions = AdvisorFeedbackAggregator::extract_suggestions(critique_critical);
        assert_eq!(suggestions.len(), 2);
        assert_eq!(suggestions[0], "Remove private key immediately");
        assert_eq!(suggestions[1], "Rotate credentials");

        let severity = AdvisorFeedbackAggregator::infer_severity(false, critique_critical);
        assert_eq!(severity, AdvisorSeverity::Critical);

        let critique_clean = "Looks good to me.\n* Keep methods small";
        let clean_sugg = AdvisorFeedbackAggregator::extract_suggestions(critique_clean);
        assert_eq!(clean_sugg.len(), 1);
        assert_eq!(clean_sugg[0], "Keep methods small");
        let clean_sev = AdvisorFeedbackAggregator::infer_severity(true, critique_clean);
        assert_eq!(clean_sev, AdvisorSeverity::Info);
    }

    #[tokio::test]
    async fn test_full_acp_event_bridge_stream_run() {
        let (out_tx, mut out_rx) = unbounded_channel::<String>();
        let (event_tx, event_rx) = unbounded_channel::<AgentEvent>();

        let broadcaster = AcpEventBroadcaster::new(64);
        let mut bcast_rx = broadcaster.subscribe();

        let bridge = AcpEventBridge::new("session-integration")
            .with_out_sender(out_tx)
            .with_broadcaster(broadcaster);

        let bridge_handle = tokio::spawn(bridge.run(event_rx));

        // Stream a turn:
        // 1. Thinking
        event_tx
            .send(AgentEvent::ThinkingDelta(
                "Let's look into the file.".to_string(),
            ))
            .unwrap();

        // 2. Token chunks
        event_tx
            .send(AgentEvent::TextDelta("Here ".to_string()))
            .unwrap();
        event_tx
            .send(AgentEvent::TextDelta("is the result.".to_string()))
            .unwrap();

        // 3. Tool execution
        event_tx
            .send(AgentEvent::ToolStarted {
                id: "tool-1".to_string(),
                name: "read_file".to_string(),
                args: serde_json::json!({ "path": "src/main.rs" }),
            })
            .unwrap();
        event_tx
            .send(AgentEvent::ToolFinished {
                id: "tool-1".to_string(),
                name: "read_file".to_string(),
                success: true,
                output: "fn main() {}".to_string(),
                duration: Duration::from_millis(15),
            })
            .unwrap();

        // 4. Finished
        event_tx
            .send(AgentEvent::Finished {
                usage: Some(serde_json::json!({
                    "prompt_tokens": 50,
                    "completion_tokens": 12
                })),
            })
            .unwrap();

        drop(event_tx); // close stream

        let summary = bridge_handle.await.unwrap();

        assert_eq!(summary.full_assistant_text, "Here is the result.");
        assert_eq!(summary.full_thought_text, "Let's look into the file.");
        assert_eq!(summary.prompt_tokens, 50);
        assert_eq!(summary.completion_tokens, 12);
        assert_eq!(summary.total_tokens, 62);
        assert_eq!(summary.tool_calls.len(), 1);
        assert_eq!(summary.tool_calls[0].name, "read_file");

        // Verify JSON-RPC output messages
        let mut notifications = Vec::new();
        while let Ok(line) = out_rx.try_recv() {
            let notif: JsonRpcNotification = serde_json::from_str(&line).unwrap();
            assert_eq!(notif.method, "session/update");
            notifications.push(notif);
        }

        assert!(
            notifications.len() >= 5,
            "Expected at least 5 notifications (thought, 2x text, tool started, tool completed, stats), got {}",
            notifications.len()
        );

        // Verify broadcast receiver got events
        let mut bcast_count = 0;
        while let Ok(_ev) = bcast_rx.try_recv() {
            bcast_count += 1;
        }
        assert_eq!(bcast_count, notifications.len());
    }
}
