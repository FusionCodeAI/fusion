//! Self-correcting retry loop and error recovery for agent tool execution.
//!
//! When a tool call fails or returns an error, the `CorrectionEngine` analyzes the root
//! cause, categorizes the error, and formulates an immediate corrective action:
//! - Auto-adjusting tool arguments (e.g. argument name aliases, stringified JSON, path prefixes).
//! - Path normalization and fuzzy file path matching.
//! - Executing precursor actions (e.g. creating missing parent directories, unlocking `.git/index.lock`).
//! - Falling back to alternative equivalent tools (e.g. `rg` -> `grep`, `search` -> `file_search`/`grep`).
//! - Applying exponential backoff for transient rate-limiting and network timeouts.
//! - Formulating structured diagnostic prompts for unrecoverable or compiler errors so the LLM
//!   can self-correct on its next turn.

use std::collections::{HashMap, HashSet};
use std::future::Future;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tokio::sync::RwLock;
use tracing::{debug, info};

use crate::agent::session::Session;
use crate::agent::strace::CausalAttribution;
use crate::tools::types::{ToolContext, ToolRegistry};
// ---------------------------------------------------------------------------
// Error Categories & Diagnostics
// ---------------------------------------------------------------------------

/// Broad classification of tool execution errors for targeted recovery.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", content = "details", rename_all = "snake_case")]
pub enum ErrorCategory {
    /// A required file was not found at the specified path.
    FileNotFound {
        path: String,
        suggested_paths: Vec<String>,
    },
    /// A required directory does not exist (e.g. when creating or writing files).
    DirectoryNotFound { path: String, parent_dir: String },
    /// Permission denied on filesystem or resource.
    PermissionDenied {
        path: Option<String>,
        operation: String,
    },
    /// Executable or binary command not found in PATH.
    CommandNotFound {
        command: String,
        suggested_alternatives: Vec<String>,
    },
    /// Compiler, interpreter, or syntax error in executed code or command.
    SyntaxOrCompileError {
        language: Option<String>,
        primary_message: String,
        file_locations: Vec<String>,
    },
    /// Tool parameter validation failed (missing required param, wrong type, unknown key).
    ToolArgumentError {
        missing_param: Option<String>,
        invalid_param: Option<String>,
        suggested_fix: String,
    },
    /// Exact search text or patch chunk was not found in the target file.
    PatchOrEditMismatch {
        file: String,
        has_crlf_mismatch: bool,
        multiple_matches: bool,
    },
    /// Git operation failed due to conflict markers or locked index.
    GitError {
        is_index_locked: bool,
        is_conflict: bool,
        details: String,
    },
    /// Subprocess or command exited with non-zero exit code.
    ProcessNonZeroExit {
        exit_code: Option<i32>,
        command: String,
        stderr_snippet: String,
    },
    /// Transient network failure, HTTP 429 rate limit, or timeout.
    RateLimitOrTimeout {
        is_rate_limited: bool,
        retry_after_secs: Option<u64>,
        details: String,
    },
    /// Tool output or context window exceeded size limit.
    OutputTooLarge { approx_bytes: usize },
    /// Uncategorized or unknown error.
    Unknown { raw_error: String },
}

impl ErrorCategory {
    /// Returns a human-friendly name for this category.
    pub fn name(&self) -> &'static str {
        match self {
            Self::FileNotFound { .. } => "File Not Found",
            Self::DirectoryNotFound { .. } => "Directory Not Found",
            Self::PermissionDenied { .. } => "Permission Denied",
            Self::CommandNotFound { .. } => "Command Not Found",
            Self::SyntaxOrCompileError { .. } => "Syntax or Compilation Error",
            Self::ToolArgumentError { .. } => "Tool Parameter Error",
            Self::PatchOrEditMismatch { .. } => "Edit/Patch Mismatch",
            Self::GitError { .. } => "Git Operation Error",
            Self::ProcessNonZeroExit { .. } => "Process Non-Zero Exit",
            Self::RateLimitOrTimeout { .. } => "Rate Limit or Timeout",
            Self::OutputTooLarge { .. } => "Output Too Large",
            Self::Unknown { .. } => "Unknown Error",
        }
    }

    /// Whether this error is likely auto-recoverable without needing LLM-level code generation.
    pub fn is_auto_recoverable(&self) -> bool {
        match self {
            Self::FileNotFound {
                suggested_paths, ..
            } => !suggested_paths.is_empty(),
            Self::DirectoryNotFound { .. } => true,
            Self::CommandNotFound {
                suggested_alternatives,
                ..
            } => !suggested_alternatives.is_empty(),
            Self::ToolArgumentError { .. } => true,
            Self::PatchOrEditMismatch {
                has_crlf_mismatch, ..
            } => *has_crlf_mismatch,
            Self::GitError {
                is_index_locked, ..
            } => *is_index_locked,
            Self::RateLimitOrTimeout { .. } => true,
            Self::SyntaxOrCompileError { .. } => false,
            Self::ProcessNonZeroExit { .. } => false,
            Self::PermissionDenied { .. } => false,
            Self::OutputTooLarge { .. } => false,
            Self::Unknown { .. } => false,
        }
    }
}

/// Comprehensive diagnosis of a tool execution failure.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ErrorDiagnosis {
    /// Categorized error.
    pub category: ErrorCategory,
    /// The tool that triggered the error.
    pub tool_name: String,
    /// The original error message.
    pub raw_error: String,
    /// Root cause analysis explanation.
    pub root_cause: String,
    /// Confidence score of diagnosis (0.0 to 1.0).
    pub confidence: f32,
    /// Whether automated recovery can be attempted immediately.
    pub is_recoverable: bool,
    /// Brief suggested action.
    pub suggested_action: String,
    /// Additional context key-values for diagnostic inspection.
    pub details: HashMap<String, String>,
}

// ---------------------------------------------------------------------------
// Corrective Actions
// ---------------------------------------------------------------------------

/// Action to perform to recover from an error.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "action_type", rename_all = "snake_case")]
pub enum CorrectiveAction {
    /// Retry the tool with modified arguments (e.g. corrected path, aliased param keys).
    RetryWithModifiedArgs {
        new_tool: String,
        new_args: Value,
        reason: String,
    },
    /// Execute a preparatory precursor action first, then optionally retry original.
    ExecutePrecursorAction {
        precursor_tool: String,
        precursor_args: Value,
        then_retry_original: bool,
        reason: String,
    },
    /// Substitute an alternative equivalent tool.
    FallbackAlternativeTool {
        fallback_tool: String,
        fallback_args: Value,
        reason: String,
    },
    /// Pause execution with exponential backoff, then retry.
    ExponentialBackoff {
        delay_ms: u64,
        max_retries: u32,
        current_retry: u32,
        reason: String,
    },
    /// Format structured diagnostic guidance and inject it into the agent's turn response.
    FormatDiagnosticGuidanceForAgent {
        diagnosis_summary: String,
        actionable_hints: Vec<String>,
        suggested_followup_tools: Vec<String>,
    },
    /// Abort unrecoverable error.
    AbortUnrecoverable {
        reason: String,
        suggested_user_action: String,
    },
}

impl CorrectiveAction {
    pub fn description(&self) -> String {
        match self {
            Self::RetryWithModifiedArgs {
                new_tool, reason, ..
            } => {
                format!("Retry '{new_tool}' with modified arguments: {reason}")
            }
            Self::ExecutePrecursorAction {
                precursor_tool,
                reason,
                ..
            } => {
                format!("Execute precursor '{precursor_tool}' first: {reason}")
            }
            Self::FallbackAlternativeTool {
                fallback_tool,
                reason,
                ..
            } => {
                format!("Fallback to '{fallback_tool}': {reason}")
            }
            Self::ExponentialBackoff {
                delay_ms,
                current_retry,
                max_retries,
                reason,
            } => {
                format!("Backoff {delay_ms}ms (retry {current_retry}/{max_retries}): {reason}")
            }
            Self::FormatDiagnosticGuidanceForAgent {
                diagnosis_summary, ..
            } => {
                format!("Provide diagnostic guidance to agent: {diagnosis_summary}")
            }
            Self::AbortUnrecoverable { reason, .. } => {
                format!("Abort unrecoverable failure: {reason}")
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Configuration & History
// ---------------------------------------------------------------------------

/// Configuration for the self-correcting retry engine.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CorrectionConfig {
    /// Maximum number of automatic retries per tool call (default: 3).
    pub max_retries: usize,
    /// Whether automatic silent recovery is enabled (default: true).
    pub enable_auto_retry: bool,
    /// Whether fuzzy path matching is enabled when files are not found (default: true).
    pub enable_fuzzy_path_correction: bool,
    /// Whether precursor actions (e.g. creating missing dirs) are enabled (default: true).
    pub enable_precursor_actions: bool,
    /// Whether fallback tool alternatives (e.g. rg -> grep) are enabled (default: true).
    pub enable_tool_fallbacks: bool,
    /// Whether exponential backoff is enabled for rate limits and network errors (default: true).
    pub enable_backoff: bool,
    /// Base delay for exponential backoff in milliseconds (default: 100ms).
    pub initial_backoff_ms: u64,
    /// Maximum delay for exponential backoff in milliseconds (default: 5000ms).
    pub max_backoff_ms: u64,
    /// Exponential factor for backoff (default: 2.0).
    pub backoff_factor: f64,
    /// Maximum cascade depth to prevent infinite precursor loops (default: 4).
    pub max_cascade_depth: usize,
}

impl Default for CorrectionConfig {
    fn default() -> Self {
        Self {
            max_retries: 3,
            enable_auto_retry: true,
            enable_fuzzy_path_correction: true,
            enable_precursor_actions: true,
            enable_tool_fallbacks: true,
            enable_backoff: true,
            initial_backoff_ms: 100,
            max_backoff_ms: 5000,
            backoff_factor: 2.0,
            max_cascade_depth: 4,
        }
    }
}

/// A record of a single correction attempt during error recovery.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CorrectionAttempt {
    pub attempt_number: usize,
    pub tool_name: String,
    pub args: Value,
    pub action_taken: String,
    pub result: Result<String, String>,
    pub duration_ms: u64,
}

/// Tracking history of corrections within a turn or session to detect cycles.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CorrectionHistory {
    pub attempts: Vec<CorrectionAttempt>,
    attempted_keys: HashSet<String>,
}

impl CorrectionHistory {
    pub fn new() -> Self {
        Self::default()
    }

    /// Generates a fingerprint key for a tool call to prevent repeating identical failed actions.
    fn make_key(tool: &str, args: &Value, action: &str) -> String {
        format!("{tool}:{}:{}", args, action)
    }

    /// Records an attempt.
    pub fn record(
        &mut self,
        tool_name: String,
        args: Value,
        action_taken: String,
        result: Result<String, String>,
        duration_ms: u64,
    ) {
        let key = Self::make_key(&tool_name, &args, &action_taken);
        self.attempted_keys.insert(key);
        self.attempts.push(CorrectionAttempt {
            attempt_number: self.attempts.len() + 1,
            tool_name,
            args,
            action_taken,
            result,
            duration_ms,
        });
    }

    /// Checks if an exact same action was already attempted to avoid cycles.
    pub fn has_attempted(&self, tool: &str, args: &Value, action: &str) -> bool {
        let key = Self::make_key(tool, args, action);
        self.attempted_keys.contains(&key)
    }

    /// Total number of attempts.
    pub fn count(&self) -> usize {
        self.attempts.len()
    }

    /// Number of successful recoveries.
    pub fn successful_count(&self) -> usize {
        self.attempts.iter().filter(|a| a.result.is_ok()).count()
    }

    /// Clears history for a new turn.
    pub fn clear(&mut self) {
        self.attempts.clear();
        self.attempted_keys.clear();
    }
}

// ---------------------------------------------------------------------------
// Correction Outcome
// ---------------------------------------------------------------------------

/// The final outcome of executing a tool with self-correcting error recovery.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum CorrectionOutcome {
    /// Execution succeeded (either on first try or after automatic recovery).
    Success {
        output: String,
        attempts: Vec<CorrectionAttempt>,
        total_corrections: usize,
        was_corrected: bool,
    },
    /// Execution failed after exhaustion of retries or unrecoverable error.
    Failed {
        error: String,
        enriched_diagnostic: String,
        attempts: Vec<CorrectionAttempt>,
        diagnosis: ErrorDiagnosis,
    },
}

impl CorrectionOutcome {
    /// Returns the final output on success, or the enriched diagnostic message on failure.
    pub fn output_or_error(&self) -> &str {
        match self {
            Self::Success { output, .. } => output,
            Self::Failed {
                enriched_diagnostic,
                ..
            } => enriched_diagnostic,
        }
    }

    /// Returns true if execution succeeded.
    pub fn is_success(&self) -> bool {
        matches!(self, Self::Success { .. })
    }

    /// Returns true if self-correction successfully altered the outcome.
    pub fn was_corrected(&self) -> bool {
        match self {
            Self::Success { was_corrected, .. } => *was_corrected,
            Self::Failed { .. } => false,
        }
    }
}

// ---------------------------------------------------------------------------
// Compiler & Language Error Diagnostics
// ---------------------------------------------------------------------------

/// Parsed Rust compiler error diagnostic.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RustCompilerDiagnostic {
    pub code: Option<String>,
    pub file: String,
    pub line: usize,
    pub column: usize,
    pub message: String,
    pub suggestion: Option<String>,
}

/// Parsed TypeScript / JavaScript compiler diagnostic.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TsCompilerDiagnostic {
    pub code: Option<String>,
    pub file: String,
    pub line: usize,
    pub column: usize,
    pub message: String,
}

/// Parsed Python traceback diagnostic.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PythonTracebackDiagnostic {
    pub exception_type: String,
    pub message: String,
    pub file: String,
    pub line: usize,
    pub code_snippet: Option<String>,
}

/// Parses Rust compiler errors (`error[E0...]: ... --> file:line:col`).
pub fn parse_rust_compiler_errors(stderr: &str) -> Vec<RustCompilerDiagnostic> {
    let mut diagnostics = Vec::new();
    let lines: Vec<&str> = stderr.lines().collect();

    let mut i = 0;
    while i < lines.len() {
        let line = lines[i];
        if line.contains("error[") || line.starts_with("error:") {
            let code = if let Some(start) = line.find("error[") {
                if let Some(end) = line[start..].find(']') {
                    Some(line[start + 6..start + end].to_string())
                } else {
                    None
                }
            } else {
                None
            };

            let message = if let Some(colon) = line.find(": ") {
                line[colon + 2..].trim().to_string()
            } else {
                line.trim().to_string()
            };

            // Look ahead for " --> file:line:col"
            let mut file = String::new();
            let mut line_num = 1;
            let mut col_num = 1;
            let mut suggestion = None;

            for j in (i + 1)..std::cmp::min(i + 15, lines.len()) {
                let next_line = lines[j].trim();
                if next_line.starts_with("--> ") {
                    let loc_part = &next_line[4..];
                    let parts: Vec<&str> = loc_part.split(':').collect();
                    if parts.len() >= 3 {
                        file = parts[0].trim().to_string();
                        line_num = parts[1].trim().parse().unwrap_or(1);
                        col_num = parts[2].trim().parse().unwrap_or(1);
                    } else if parts.len() == 2 {
                        file = parts[0].trim().to_string();
                        line_num = parts[1].trim().parse().unwrap_or(1);
                    }
                } else if next_line.starts_with("help: ") {
                    suggestion = Some(next_line[6..].trim().to_string());
                } else if next_line.contains('+') && suggestion.is_some() {
                    if let Some(s) = &mut suggestion {
                        if let Some(plus_idx) = next_line.find('+') {
                            s.push(' ');
                            s.push_str(next_line[plus_idx + 1..].trim());
                        }
                    }
                } else if next_line.starts_with("error[") || next_line.starts_with("error:") {
                    break;
                }
            }

            if !file.is_empty() || !message.is_empty() {
                diagnostics.push(RustCompilerDiagnostic {
                    code,
                    file,
                    line: line_num,
                    column: col_num,
                    message,
                    suggestion,
                });
            }
        }
        i += 1;
    }

    diagnostics
}

/// Parses TypeScript compiler errors (`file(line,col): error TS...: message`).
pub fn parse_ts_compiler_errors(stderr: &str) -> Vec<TsCompilerDiagnostic> {
    let mut diagnostics = Vec::new();

    for line in stderr.lines() {
        let trimmed = line.trim();
        if trimmed.contains("error TS") {
            // e.g. "src/index.ts(14,5): error TS2322: Type 'string' is not assignable to type 'number'."
            let mut file = String::new();
            let mut line_num = 1;
            let mut col_num = 1;
            let mut code = None;
            let mut message = trimmed.to_string();

            if let Some(paren_open) = trimmed.find('(') {
                if let Some(paren_close) = trimmed.find("):") {
                    file = trimmed[..paren_open].trim().to_string();
                    let loc_str = &trimmed[paren_open + 1..paren_close];
                    let loc_parts: Vec<&str> = loc_str.split(',').collect();
                    if loc_parts.len() >= 2 {
                        line_num = loc_parts[0].trim().parse().unwrap_or(1);
                        col_num = loc_parts[1].trim().parse().unwrap_or(1);
                    }
                }
            }

            if let Some(ts_idx) = trimmed.find("error TS") {
                if let Some(colon) = trimmed[ts_idx..].find(':') {
                    code = Some(trimmed[ts_idx + 6..ts_idx + colon].trim().to_string());
                    message = trimmed[ts_idx + colon + 1..].trim().to_string();
                }
            }

            diagnostics.push(TsCompilerDiagnostic {
                code,
                file,
                line: line_num,
                column: col_num,
                message,
            });
        }
    }

    diagnostics
}

/// Parses Python tracebacks (`File "...", line ..., in ... \n Exception: ...`).
pub fn parse_python_traceback(stderr: &str) -> Option<PythonTracebackDiagnostic> {
    if !stderr.contains("Traceback (most recent call last):") {
        return None;
    }

    let lines: Vec<&str> = stderr.lines().collect();
    let mut file = String::new();
    let mut line_num = 1;
    let mut code_snippet = None;
    let mut exception_type = "Error".to_string();
    let mut message = String::new();

    for i in 0..lines.len() {
        let line = lines[i].trim();
        if line.starts_with("File \"") {
            if let Some(quote_end) = line[6..].find('\"') {
                file = line[6..6 + quote_end].to_string();
                if let Some(line_idx) = line.find(", line ") {
                    let rest = &line[line_idx + 7..];
                    let end = rest
                        .find(',')
                        .or_else(|| rest.find(' '))
                        .unwrap_or(rest.len());
                    line_num = rest[..end].trim().parse().unwrap_or(1);
                }
                if i + 1 < lines.len() {
                    code_snippet = Some(lines[i + 1].trim().to_string());
                }
            }
        }
    }

    if let Some(last_line) = lines.last() {
        let trimmed = last_line.trim();
        if let Some(colon) = trimmed.find(':') {
            exception_type = trimmed[..colon].trim().to_string();
            message = trimmed[colon + 1..].trim().to_string();
        } else {
            message = trimmed.to_string();
        }
    }

    if !file.is_empty() || !message.is_empty() {
        Some(PythonTracebackDiagnostic {
            exception_type,
            message,
            file,
            line: line_num,
            code_snippet,
        })
    } else {
        None
    }
}

// ---------------------------------------------------------------------------
// Path & Fuzzy Search Utilities
// ---------------------------------------------------------------------------

/// Normalizes a path string by stripping surrounding quotes, line-number suffixes, and trailing colons.
pub fn clean_path_string(raw: &str) -> String {
    let mut s = raw.trim().trim_matches('\'').trim_matches('\"').trim();
    // Remove :line:col or :line suffix (e.g. "src/main.rs:42:10" -> "src/main.rs")
    if let Some(idx) = s.rfind(':') {
        let after = &s[idx + 1..];
        if after.chars().all(|c| c.is_ascii_digit()) {
            s = &s[..idx];
            if let Some(idx2) = s.rfind(':') {
                let after2 = &s[idx2 + 1..];
                if after2.chars().all(|c| c.is_ascii_digit()) {
                    s = &s[..idx2];
                }
            }
        }
    }
    s.to_string()
}

/// Finds fuzzy matches for a missing file path within the current working directory.
pub fn find_fuzzy_file_matches(target_str: &str, cwd: &Path) -> Vec<String> {
    let clean = clean_path_string(target_str);
    let target_path = Path::new(&clean);
    let target_filename = target_path
        .file_name()
        .and_then(|f| f.to_str())
        .unwrap_or("")
        .to_lowercase();

    if target_filename.is_empty() {
        return Vec::new();
    }

    let mut suggestions = Vec::new();

    // Strategy 1: Check common missing extensions (.rs, .ts, .js, .json, .toml, .py, .md)
    let common_exts = [
        "rs", "ts", "js", "json", "toml", "py", "md", "sh", "yaml", "yml",
    ];
    for ext in common_exts {
        let with_ext = format!("{clean}.{ext}");
        if cwd.join(&with_ext).exists() {
            suggestions.push(with_ext);
        }
    }

    // Strategy 2: Check standard source prefix mappings (e.g. "agent/mod.rs" -> "src/agent/mod.rs")
    let common_prefixes = ["src/", "tests/", "src/tools/", "src/agent/", "crates/"];
    for prefix in common_prefixes {
        let prefixed = format!("{prefix}{clean}");
        if cwd.join(&prefixed).exists() {
            suggestions.push(prefixed);
        }
    }

    // Strategy 3: Walk shallow workspace (up to depth 4) to find files with matching name or similar name
    if suggestions.is_empty() {
        let mut matching_paths = Vec::new();
        walk_shallow_dir(cwd, 0, 4, &mut |path| {
            if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                let name_lower = name.to_lowercase();
                if name_lower == target_filename {
                    if let Ok(rel) = path.strip_prefix(cwd) {
                        matching_paths.push(rel.to_string_lossy().to_string());
                    }
                } else if target_filename.len() >= 4
                    && (name_lower.contains(&target_filename)
                        || target_filename.contains(&name_lower))
                {
                    if let Ok(rel) = path.strip_prefix(cwd) {
                        matching_paths.push(rel.to_string_lossy().to_string());
                    }
                }
            }
        });
        suggestions.extend(matching_paths);
    }

    suggestions.truncate(5);
    suggestions
}

/// Helper recursive shallow directory walker.
fn walk_shallow_dir<F>(dir: &Path, current_depth: usize, max_depth: usize, callback: &mut F)
where
    F: FnMut(&Path),
{
    if current_depth >= max_depth {
        return;
    }

    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if let Some(file_name) = path.file_name().and_then(|n| n.to_str()) {
                // Ignore heavy or hidden directories
                if file_name.starts_with('.')
                    || file_name == "target"
                    || file_name == "node_modules"
                {
                    continue;
                }
            }

            if path.is_file() {
                callback(&path);
            } else if path.is_dir() {
                walk_shallow_dir(&path, current_depth + 1, max_depth, callback);
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Error Analyzer & Diagnosis Engine
// ---------------------------------------------------------------------------

/// Analyzes errors and diagnoses root causes.
pub struct ErrorAnalyzer;

impl ErrorAnalyzer {
    /// Diagnoses a tool execution failure into a structured `ErrorDiagnosis`.
    pub fn diagnose(
        tool_name: &str,
        args: &Value,
        error_str: &str,
        ctx: &ToolContext,
    ) -> ErrorDiagnosis {
        let err_lower = error_str.to_lowercase();
        let mut details = HashMap::new();

        // 1. Check for Directory Not Found (e.g. writing to missing folder)
        if (err_lower.contains("no such file or directory")
            && (tool_name == "write" || tool_name == "write_file"))
            || err_lower.contains("parent directory does not exist")
        {
            let target_path = extract_path_from_args(args).unwrap_or_default();
            let parent_dir = Path::new(&target_path)
                .parent()
                .map(|p| p.to_string_lossy().to_string())
                .unwrap_or_default();

            details.insert("target_path".to_string(), target_path.clone());
            details.insert("parent_dir".to_string(), parent_dir.clone());

            return ErrorDiagnosis {
                category: ErrorCategory::DirectoryNotFound {
                    path: target_path,
                    parent_dir: parent_dir.clone(),
                },
                tool_name: tool_name.to_string(),
                raw_error: error_str.to_string(),
                root_cause: format!("Parent directory '{parent_dir}' does not exist yet."),
                confidence: 0.95,
                is_recoverable: !parent_dir.is_empty(),
                suggested_action: format!("Create directory '{parent_dir}' before writing file."),
                details,
            };
        }

        // 2. Check for File Not Found
        if err_lower.contains("file not found")
            || err_lower.contains("no such file or directory")
            || err_lower.contains("cannot find the path specified")
            || err_lower.contains("failed to read file")
        {
            let extracted_path = extract_path_from_error(error_str)
                .or_else(|| extract_path_from_args(args))
                .unwrap_or_default();

            let suggestions = if !extracted_path.is_empty() {
                find_fuzzy_file_matches(&extracted_path, &ctx.cwd)
            } else {
                Vec::new()
            };

            details.insert("extracted_path".to_string(), extracted_path.clone());
            if !suggestions.is_empty() {
                details.insert("suggestions".to_string(), suggestions.join(", "));
            }

            let is_rec = !suggestions.is_empty();
            let suggested_action = if is_rec {
                format!("Retry using resolved path: '{}'", suggestions[0])
            } else {
                "Verify file path with search or glob tool".to_string()
            };

            return ErrorDiagnosis {
                category: ErrorCategory::FileNotFound {
                    path: extracted_path,
                    suggested_paths: suggestions,
                },
                tool_name: tool_name.to_string(),
                raw_error: error_str.to_string(),
                root_cause: "Target file does not exist at specified path.".to_string(),
                confidence: 0.95,
                is_recoverable: is_rec,
                suggested_action,
                details,
            };
        }

        // 3. Check for Permission Denied
        if err_lower.contains("permission denied")
            || err_lower.contains("access is denied")
            || err_lower.contains("operation not permitted")
        {
            let path = extract_path_from_args(args);
            return ErrorDiagnosis {
                category: ErrorCategory::PermissionDenied {
                    path: path.clone(),
                    operation: tool_name.to_string(),
                },
                tool_name: tool_name.to_string(),
                raw_error: error_str.to_string(),
                root_cause: "Filesystem or OS permissions denied the operation.".to_string(),
                confidence: 0.9,
                is_recoverable: false,
                suggested_action:
                    "Check file permissions or target an authorized writable directory.".to_string(),
                details,
            };
        }

        // 4. Check for Command Not Found
        if err_lower.contains("command not found")
            || err_lower.contains("not recognized as an internal or external command")
            || err_lower.contains("no such file or directory") && tool_name == "bash"
        {
            let cmd = extract_command_from_args(args).unwrap_or_else(|| "command".to_string());
            let first_word = cmd.split_whitespace().next().unwrap_or(&cmd).to_string();

            let alternatives = suggest_command_alternatives(&first_word);
            let is_rec = !alternatives.is_empty();

            details.insert("command".to_string(), first_word.clone());
            if is_rec {
                details.insert("alternatives".to_string(), alternatives.join(", "));
            }

            let suggested_action = if is_rec {
                format!(
                    "Replace '{}' with alternative: '{}'",
                    first_word, alternatives[0]
                )
            } else {
                format!(
                    "Ensure binary '{}' is installed or use standard POSIX tools.",
                    first_word
                )
            };

            return ErrorDiagnosis {
                category: ErrorCategory::CommandNotFound {
                    command: first_word,
                    suggested_alternatives: alternatives,
                },
                tool_name: tool_name.to_string(),
                raw_error: error_str.to_string(),
                root_cause: "Requested executable binary is not available in PATH.".to_string(),
                confidence: 0.9,
                is_recoverable: is_rec,
                suggested_action,
                details,
            };
        }

        // 5. Check for Edit / Patch text mismatch
        if err_lower.contains("old_text not found")
            || err_lower.contains("patch failed")
            || err_lower.contains("could not find text to replace")
            || err_lower.contains("line ending differences (crlf vs lf)")
        {
            let has_crlf =
                err_lower.contains("crlf") || err_lower.contains("line ending differences");
            let multiple_matches = err_lower.contains("occurs") && err_lower.contains("times");
            let file = extract_path_from_args(args).unwrap_or_default();

            details.insert("file".to_string(), file.clone());
            details.insert("has_crlf".to_string(), has_crlf.to_string());

            let suggested_action = if has_crlf {
                "Normalize line endings to LF before replacing".to_string()
            } else if multiple_matches {
                "Add more surrounding context lines to disambiguate the target block".to_string()
            } else {
                "Re-read file to verify current content and exact indentation before editing"
                    .to_string()
            };

            return ErrorDiagnosis {
                category: ErrorCategory::PatchOrEditMismatch {
                    file,
                    has_crlf_mismatch: has_crlf,
                    multiple_matches,
                },
                tool_name: tool_name.to_string(),
                raw_error: error_str.to_string(),
                root_cause:
                    "Target search text in edit/patch operation does not match file content."
                        .to_string(),
                confidence: 0.95,
                is_recoverable: has_crlf,
                suggested_action,
                details,
            };
        }

        // 6. Check for Git Errors (index.lock or conflicts)
        if err_lower.contains("another git process seems to be running")
            || err_lower.contains(".git/index.lock")
            || err_lower.contains("conflict (content)")
            || err_lower.contains("merge conflict")
        {
            let is_locked =
                err_lower.contains("index.lock") || err_lower.contains("another git process");
            let is_conflict = err_lower.contains("conflict");

            let suggested_action = if is_locked {
                "Remove stale .git/index.lock file and retry git operation".to_string()
            } else {
                "Resolve git merge conflicts in affected files".to_string()
            };

            return ErrorDiagnosis {
                category: ErrorCategory::GitError {
                    is_index_locked: is_locked,
                    is_conflict,
                    details: error_str.to_string(),
                },
                tool_name: tool_name.to_string(),
                raw_error: error_str.to_string(),
                root_cause: if is_locked {
                    "Git index is locked by an earlier or active process.".to_string()
                } else {
                    "Git merge conflict detected.".to_string()
                },
                confidence: 0.95,
                is_recoverable: is_locked,
                suggested_action,
                details,
            };
        }

        // 7. Check for Tool Parameter / Argument Validation Errors
        if err_lower.contains("missing required parameter")
            || err_lower.contains("invalid type")
            || err_lower.contains("unknown parameter")
            || err_lower.contains("invalid json")
            || err_lower.contains("failed to parse arguments")
        {
            let missing_param = extract_missing_param(error_str);
            let fix = if let Some(p) = &missing_param {
                format!("Provide required parameter '{p}'")
            } else {
                "Correct parameter names and types to match tool definition".to_string()
            };

            return ErrorDiagnosis {
                category: ErrorCategory::ToolArgumentError {
                    missing_param,
                    invalid_param: None,
                    suggested_fix: fix.clone(),
                },
                tool_name: tool_name.to_string(),
                raw_error: error_str.to_string(),
                root_cause: "Tool call arguments do not match expected schema.".to_string(),
                confidence: 0.9,
                is_recoverable: true,
                suggested_action: fix,
                details,
            };
        }

        // 8. Check for Rate Limit or Network Timeouts
        if err_lower.contains("rate limit")
            || err_lower.contains("429")
            || err_lower.contains("too many requests")
            || err_lower.contains("connection timed out")
            || err_lower.contains("network unreachable")
            || err_lower.contains("timed out")
        {
            let is_rate_limit = err_lower.contains("429") || err_lower.contains("rate limit");
            let retry_after = extract_retry_after_secs(error_str);

            return ErrorDiagnosis {
                category: ErrorCategory::RateLimitOrTimeout {
                    is_rate_limited: is_rate_limit,
                    retry_after_secs: retry_after,
                    details: error_str.to_string(),
                },
                tool_name: tool_name.to_string(),
                raw_error: error_str.to_string(),
                root_cause: "Remote service rate limited request or network connection timed out."
                    .to_string(),
                confidence: 0.9,
                is_recoverable: true,
                suggested_action: "Retry with exponential backoff.".to_string(),
                details,
            };
        }

        // 9. Check for Syntax or Compilation Errors (Rust / TypeScript / Python)
        if error_str.contains("error[E")
            || error_str.contains("error TS")
            || error_str.contains("Traceback (most recent call last)")
        {
            let language = if error_str.contains("error[E") {
                Some("Rust".to_string())
            } else if error_str.contains("error TS") {
                Some("TypeScript".to_string())
            } else {
                Some("Python".to_string())
            };

            let primary_message = error_str.lines().next().unwrap_or(error_str).to_string();

            return ErrorDiagnosis {
                category: ErrorCategory::SyntaxOrCompileError {
                    language,
                    primary_message,
                    file_locations: Vec::new(),
                },
                tool_name: tool_name.to_string(),
                raw_error: error_str.to_string(),
                root_cause: "Source code failed compilation or syntax validation.".to_string(),
                confidence: 0.95,
                is_recoverable: false,
                suggested_action: "Formulate code fix addressing compiler error diagnostics."
                    .to_string(),
                details,
            };
        }

        // 10. Check for Process Non-Zero Exit
        if err_lower.contains("exit status")
            || err_lower.contains("exit code")
            || err_lower.contains("command failed with")
        {
            let code = extract_exit_code(error_str);
            let cmd = extract_command_from_args(args).unwrap_or_else(|| "bash".to_string());

            return ErrorDiagnosis {
                category: ErrorCategory::ProcessNonZeroExit {
                    exit_code: code,
                    command: cmd,
                    stderr_snippet: error_str.chars().take(200).collect(),
                },
                tool_name: tool_name.to_string(),
                raw_error: error_str.to_string(),
                root_cause: "Command exited with non-zero status code.".to_string(),
                confidence: 0.85,
                is_recoverable: false,
                suggested_action: "Inspect stderr output and adjust command or source code."
                    .to_string(),
                details,
            };
        }

        // 11. Fallback Unknown
        ErrorDiagnosis {
            category: ErrorCategory::Unknown {
                raw_error: error_str.to_string(),
            },
            tool_name: tool_name.to_string(),
            raw_error: error_str.to_string(),
            root_cause: "Uncategorized tool execution failure.".to_string(),
            confidence: 0.5,
            is_recoverable: false,
            suggested_action: "Review error output and formulate alternative tool strategy."
                .to_string(),
            details,
        }
    }
}

// ---------------------------------------------------------------------------
// Helper Extraction Functions
// ---------------------------------------------------------------------------

fn extract_path_from_args(args: &Value) -> Option<String> {
    if let Value::Object(map) = args {
        for key in [
            "path",
            "file_path",
            "file",
            "filename",
            "filepath",
            "target",
            "dest",
        ] {
            if let Some(Value::String(s)) = map.get(key) {
                return Some(s.clone());
            }
        }
    }
    None
}

fn extract_command_from_args(args: &Value) -> Option<String> {
    if let Value::Object(map) = args {
        for key in ["command", "cmd", "script", "input"] {
            if let Some(Value::String(s)) = map.get(key) {
                return Some(s.clone());
            }
        }
    }
    None
}

fn extract_path_from_error(error_str: &str) -> Option<String> {
    // Try matching single or double quoted paths, e.g. "File not found: 'src/main.rs'"
    if let Some(start) = error_str.find('\'') {
        if let Some(end) = error_str[start + 1..].find('\'') {
            return Some(error_str[start + 1..start + 1 + end].to_string());
        }
    }
    if let Some(start) = error_str.find('\"') {
        if let Some(end) = error_str[start + 1..].find('\"') {
            return Some(error_str[start + 1..start + 1 + end].to_string());
        }
    }
    None
}

fn extract_missing_param(error_str: &str) -> Option<String> {
    if let Some(idx) = error_str.find("Missing required parameter:") {
        let after = &error_str[idx + 27..].trim();
        let name = after
            .split_whitespace()
            .next()
            .unwrap_or("")
            .trim_matches(':')
            .trim_matches('\'')
            .trim_matches('\"');
        if !name.is_empty() {
            return Some(name.to_string());
        }
    }
    None
}

fn extract_retry_after_secs(error_str: &str) -> Option<u64> {
    if let Some(idx) = error_str.to_lowercase().find("retry-after") {
        let rest = &error_str[idx + 11..];
        for word in rest.split(|c: char| !c.is_ascii_digit()) {
            if let Ok(n) = word.parse::<u64>() {
                if n > 0 && n < 3600 {
                    return Some(n);
                }
            }
        }
    }
    None
}

fn extract_exit_code(error_str: &str) -> Option<i32> {
    for pattern in ["exit status ", "exit code ", "code "] {
        if let Some(idx) = error_str.to_lowercase().find(pattern) {
            let rest = &error_str[idx + pattern.len()..];
            let num_str: String = rest
                .chars()
                .take_while(|c| c.is_ascii_digit() || *c == '-')
                .collect();
            if let Ok(code) = num_str.parse::<i32>() {
                return Some(code);
            }
        }
    }
    None
}

fn suggest_command_alternatives(cmd: &str) -> Vec<String> {
    match cmd {
        "rg" | "ripgrep" => vec!["grep -rn".to_string()],
        "fd" => vec!["find . -name".to_string()],
        "bat" => vec!["cat".to_string()],
        "python3" => vec!["python".to_string()],
        "python" => vec!["python3".to_string()],
        "node" => vec!["bun".to_string(), "deno".to_string()],
        "bun" => vec!["node".to_string()],
        "trash" => vec!["rm".to_string()],
        _ => Vec::new(),
    }
}

// ---------------------------------------------------------------------------
// Self-Correcting Engine
// ---------------------------------------------------------------------------

/// Diagnoses session trajectory using STRACE to isolate root-cause attribution for turn failures.
pub fn causal_diagnose_turn(session: &Session) -> Option<CausalAttribution> {
    crate::agent::strace::RootCauseAttributor::diagnose_session(session).ok()
}

/// Constructs a structured correction prompt for repeated turn failures,
/// incorporating STRACE root-cause causal attribution if available from the session trajectory.
pub fn construct_correction_prompt(
    session: &Session,
    failed_tool: &str,
    error_message: &str,
) -> String {
    let mut prompt = format!("❌ Repeated Failure in `{failed_tool}`: {error_message}\n\n");
    if let Some(attr) = causal_diagnose_turn(session) {
        prompt.push_str("🔍 STRACE Root-Cause Attribution:\n");
        prompt.push_str(&format!(
            "• Manifestation: `{}` (step {})\n",
            attr.manifestation_node, attr.manifestation_pos
        ));
        prompt.push_str(&format!(
            "• Root Cause: `{}` (step {})\n",
            attr.root_cause_node, attr.root_cause_pos
        ));
        prompt.push_str(&format!("• Reason: {}\n", attr.reason));
        prompt.push_str(&format!(
            "• Recommended Heuristic: {}\n",
            attr.suggested_heuristic
        ));
        if !attr.causal_chain.is_empty() {
            prompt.push_str(&format!(
                "• Causal Dependency Chain: {:?}\n",
                attr.causal_chain
            ));
        }
        prompt.push_str("\nPlease update your execution strategy to avoid repeating this root-cause fault pattern.\n");
    } else {
        prompt.push_str(
            "Please review previous failed attempts and choose an alternative strategy.\n",
        );
    }
    prompt
}

/// The core self-correcting retry engine.
#[derive(Debug, Clone)]
pub struct CorrectionEngine {
    pub config: CorrectionConfig,
    pub history: Arc<RwLock<CorrectionHistory>>,
}

impl Default for CorrectionEngine {
    fn default() -> Self {
        Self::new(CorrectionConfig::default())
    }
}

impl CorrectionEngine {
    /// Creates a new `CorrectionEngine` with the given configuration.
    pub fn new(config: CorrectionConfig) -> Self {
        Self {
            config,
            history: Arc::new(RwLock::new(CorrectionHistory::new())),
        }
    }

    /// Diagnoses a tool execution failure into an `ErrorDiagnosis`.
    pub fn diagnose(
        &self,
        tool_name: &str,
        args: &Value,
        error_str: &str,
        ctx: &ToolContext,
    ) -> ErrorDiagnosis {
        ErrorAnalyzer::diagnose(tool_name, args, error_str, ctx)
    }

    /// Formulates an immediate corrective action based on error diagnosis and history.
    pub fn formulate_action(
        &self,
        diagnosis: &ErrorDiagnosis,
        original_tool: &str,
        original_args: &Value,
        history: &CorrectionHistory,
        _ctx: &ToolContext,
    ) -> CorrectiveAction {
        if !self.config.enable_auto_retry || history.count() >= self.config.max_retries {
            return self.formulate_diagnostic_guidance(diagnosis, original_tool, original_args);
        }

        match &diagnosis.category {
            // Recovery 1: File not found with suggested fuzzy path match
            ErrorCategory::FileNotFound {
                suggested_paths, ..
            } if self.config.enable_fuzzy_path_correction => {
                if let Some(best_match) = suggested_paths.first() {
                    let mut new_args = original_args.clone();
                    if let Value::Object(map) = &mut new_args {
                        for key in ["path", "file_path", "file", "filename"] {
                            if map.contains_key(key) {
                                map.insert(key.to_string(), json!(best_match));
                            }
                        }
                    }

                    let action = CorrectiveAction::RetryWithModifiedArgs {
                        new_tool: original_tool.to_string(),
                        new_args: new_args.clone(),
                        reason: format!(
                            "Corrected missing file path to closest match: '{}'",
                            best_match
                        ),
                    };

                    if !history.has_attempted(original_tool, &new_args, &action.description()) {
                        return action;
                    }
                }
            }

            // Recovery 2: Directory not found -> Precursor mkdir -p
            ErrorCategory::DirectoryNotFound { parent_dir, .. }
                if self.config.enable_precursor_actions && !parent_dir.is_empty() =>
            {
                let precursor_tool = "bash".to_string();
                let precursor_args = json!({
                    "command": format!("mkdir -p '{}'", parent_dir)
                });

                let action = CorrectiveAction::ExecutePrecursorAction {
                    precursor_tool: precursor_tool.clone(),
                    precursor_args: precursor_args.clone(),
                    then_retry_original: true,
                    reason: format!(
                        "Create missing parent directory '{parent_dir}' before creating file"
                    ),
                };

                if !history.has_attempted(&precursor_tool, &precursor_args, &action.description()) {
                    return action;
                }
            }

            // Recovery 3: Tool argument aliasing (e.g. "file" -> "path", "cmd" -> "command")
            ErrorCategory::ToolArgumentError { .. } => {
                let mut mutated_args = original_args.clone();
                let mut mutated = false;

                if let Value::Object(map) = &mut mutated_args {
                    // Normalize path parameter aliases
                    if (map.contains_key("file")
                        || map.contains_key("file_path")
                        || map.contains_key("filename"))
                        && !map.contains_key("path")
                    {
                        let val = map
                            .remove("file")
                            .or_else(|| map.remove("file_path"))
                            .or_else(|| map.remove("filename"))
                            .unwrap();
                        map.insert("path".to_string(), val);
                        mutated = true;
                    }

                    // Normalize command parameter aliases
                    if (map.contains_key("cmd") || map.contains_key("script"))
                        && !map.contains_key("command")
                    {
                        let val = map.remove("cmd").or_else(|| map.remove("script")).unwrap();
                        map.insert("command".to_string(), val);
                        mutated = true;
                    }

                    // Normalize content parameter aliases
                    if (map.contains_key("text") || map.contains_key("data"))
                        && !map.contains_key("content")
                    {
                        let val = map.remove("text").or_else(|| map.remove("data")).unwrap();
                        map.insert("content".to_string(), val);
                        mutated = true;
                    }

                    // Normalize edit tool parameter aliases
                    if (map.contains_key("old_string") || map.contains_key("old_content"))
                        && !map.contains_key("old_text")
                    {
                        let val = map
                            .remove("old_string")
                            .or_else(|| map.remove("old_content"))
                            .unwrap();
                        map.insert("old_text".to_string(), val);
                        mutated = true;
                    }
                    if (map.contains_key("new_string") || map.contains_key("new_content"))
                        && !map.contains_key("new_text")
                    {
                        let val = map
                            .remove("new_string")
                            .or_else(|| map.remove("new_content"))
                            .unwrap();
                        map.insert("new_text".to_string(), val);
                        mutated = true;
                    }
                }

                if mutated {
                    let action = CorrectiveAction::RetryWithModifiedArgs {
                        new_tool: original_tool.to_string(),
                        new_args: mutated_args.clone(),
                        reason: "Normalized parameter names to match tool schema.".to_string(),
                    };
                    if !history.has_attempted(original_tool, &mutated_args, &action.description()) {
                        return action;
                    }
                }
            }

            // Recovery 4: Command not found -> Fallback alternative CLI command
            ErrorCategory::CommandNotFound {
                suggested_alternatives,
                ..
            } if self.config.enable_tool_fallbacks => {
                if let Some(alt_cmd) = suggested_alternatives.first() {
                    let mut new_args = original_args.clone();
                    if let Value::Object(map) = &mut new_args {
                        if let Some(Value::String(orig_cmd_str)) = map.get("command") {
                            let words: Vec<&str> = orig_cmd_str.split_whitespace().collect();
                            if !words.is_empty() {
                                let rest = words[1..].join(" ");
                                let replaced_cmd = format!("{alt_cmd} {rest}").trim().to_string();
                                map.insert("command".to_string(), json!(replaced_cmd));

                                let action = CorrectiveAction::RetryWithModifiedArgs {
                                    new_tool: original_tool.to_string(),
                                    new_args: new_args.clone(),
                                    reason: format!("Substituted alternative command '{alt_cmd}'"),
                                };

                                if !history.has_attempted(
                                    original_tool,
                                    &new_args,
                                    &action.description(),
                                ) {
                                    return action;
                                }
                            }
                        }
                    }
                }
            }

            // Recovery 5: Edit CRLF mismatch -> Normalize line endings
            ErrorCategory::PatchOrEditMismatch {
                has_crlf_mismatch: true,
                ..
            } => {
                let mut new_args = original_args.clone();
                if let Value::Object(map) = &mut new_args {
                    if let Some(Value::String(old_t)) = map.get("old_text") {
                        let normalized = old_t.replace("\r\n", "\n");
                        map.insert("old_text".to_string(), json!(normalized));
                    }
                    if let Some(Value::String(new_t)) = map.get("new_text") {
                        let normalized = new_t.replace("\r\n", "\n");
                        map.insert("new_text".to_string(), json!(normalized));
                    }
                    let action = CorrectiveAction::RetryWithModifiedArgs {
                        new_tool: original_tool.to_string(),
                        new_args: new_args.clone(),
                        reason: "Normalized line endings from CRLF to LF in edit parameters."
                            .to_string(),
                    };
                    if !history.has_attempted(original_tool, &new_args, &action.description()) {
                        return action;
                    }
                }
            }

            // Recovery 6: Git index locked -> Precursor remove .git/index.lock
            ErrorCategory::GitError {
                is_index_locked: true,
                ..
            } if self.config.enable_precursor_actions => {
                let precursor_tool = "bash".to_string();
                let precursor_args = json!({
                    "command": "rm -f .git/index.lock"
                });

                let action = CorrectiveAction::ExecutePrecursorAction {
                    precursor_tool: precursor_tool.clone(),
                    precursor_args: precursor_args.clone(),
                    then_retry_original: true,
                    reason: "Removed stale .git/index.lock before retrying git command".to_string(),
                };

                if !history.has_attempted(&precursor_tool, &precursor_args, &action.description()) {
                    return action;
                }
            }

            // Recovery 7: Rate limit or timeout -> Exponential backoff
            ErrorCategory::RateLimitOrTimeout {
                retry_after_secs, ..
            } if self.config.enable_backoff => {
                let retry_count = (history.count() + 1) as u32;
                let calculated_delay = if let Some(secs) = retry_after_secs {
                    (*secs * 1000).min(self.config.max_backoff_ms)
                } else {
                    let factor = self.config.backoff_factor.powi((retry_count - 1) as i32);
                    ((self.config.initial_backoff_ms as f64) * factor) as u64
                };
                let delay = calculated_delay.min(self.config.max_backoff_ms);

                let action = CorrectiveAction::ExponentialBackoff {
                    delay_ms: delay,
                    max_retries: self.config.max_retries as u32,
                    current_retry: retry_count,
                    reason: format!(
                        "Transient rate limit or network timeout; backing off {delay}ms"
                    ),
                };

                if !history.has_attempted(original_tool, original_args, &action.description()) {
                    return action;
                }
            }

            _ => {}
        }

        // Default: Fallback to structured diagnostic guidance for the agent turn
        self.formulate_diagnostic_guidance(diagnosis, original_tool, original_args)
    }

    /// Formulates clear markdown diagnostic guidance to feed back to the LLM agent.
    fn formulate_diagnostic_guidance(
        &self,
        diagnosis: &ErrorDiagnosis,
        tool_name: &str,
        _args: &Value,
    ) -> CorrectiveAction {
        let mut hints = Vec::new();
        let mut followup_tools = Vec::new();

        match &diagnosis.category {
            ErrorCategory::FileNotFound {
                path,
                suggested_paths,
            } => {
                hints.push(format!("File '{path}' does not exist in workspace."));
                if !suggested_paths.is_empty() {
                    hints.push(format!(
                        "Possible matching files: {}",
                        suggested_paths.join(", ")
                    ));
                } else {
                    hints.push(
                        "Use `glob` or `search` to verify directory structure and locate the file."
                            .to_string(),
                    );
                }
                followup_tools.push("glob".to_string());
                followup_tools.push("search".to_string());
            }
            ErrorCategory::SyntaxOrCompileError {
                language,
                primary_message,
                ..
            } => {
                let lang = language.as_deref().unwrap_or("Code");
                hints.push(format!("{lang} validation failed: {primary_message}"));
                hints.push("Read affected source files around the reported error line numbers and apply surgical edits.".to_string());
                followup_tools.push("read".to_string());
                followup_tools.push("edit".to_string());
            }
            ErrorCategory::PatchOrEditMismatch {
                file,
                multiple_matches,
                ..
            } => {
                hints.push(format!(
                    "Search block in '{file}' could not be matched uniquely."
                ));
                if *multiple_matches {
                    hints.push(
                        "Include 3-5 lines of surrounding context to make `old_text` unique."
                            .to_string(),
                    );
                } else {
                    hints.push("Re-read the file with `read` tool to copy the exact lines including indentation.".to_string());
                }
                followup_tools.push("read".to_string());
                followup_tools.push("edit".to_string());
            }
            ErrorCategory::ProcessNonZeroExit {
                exit_code,
                stderr_snippet,
                ..
            } => {
                if let Some(code) = exit_code {
                    hints.push(format!("Process failed with exit code {code}."));
                }
                hints.push(format!("Error snippet: {stderr_snippet}"));
                followup_tools.push("bash".to_string());
            }
            _ => {
                hints.push(diagnosis.suggested_action.clone());
            }
        }

        CorrectiveAction::FormatDiagnosticGuidanceForAgent {
            diagnosis_summary: format!("Tool '{tool_name}' failed: {}", diagnosis.root_cause),
            actionable_hints: hints,
            suggested_followup_tools: followup_tools,
        }
    }

    /// Formats an error into a structured markdown diagnostic response for the LLM.
    pub fn format_agent_feedback(
        &self,
        tool_name: &str,
        _args: &Value,
        diagnosis: &ErrorDiagnosis,
        history: &CorrectionHistory,
    ) -> String {
        let mut out = String::new();
        out.push_str(&format!("❌ Tool Execution Failed: `{tool_name}`\n"));
        out.push_str(&format!("• Category: {}\n", diagnosis.category.name()));
        out.push_str(&format!("• Cause: {}\n", diagnosis.root_cause));
        out.push_str(&format!("• Raw Error: {}\n\n", diagnosis.raw_error));

        if !history.attempts.is_empty() {
            out.push_str("⚠️ Automated Recovery History:\n");
            for attempt in &history.attempts {
                let status = match &attempt.result {
                    Ok(_) => "✅ Success",
                    Err(_) => "❌ Failed",
                };
                out.push_str(&format!(
                    "  {}. [{}] Action: {} (took {}ms)\n",
                    attempt.attempt_number, status, attempt.action_taken, attempt.duration_ms
                ));
            }
            out.push('\n');
        }

        out.push_str("💡 Recommended Next Steps:\n");
        out.push_str(&format!("1. {}\n", diagnosis.suggested_action));

        if let Some(details_path) = diagnosis.details.get("suggestions") {
            out.push_str(&format!("2. Check possible matches: {details_path}\n"));
        }

        out
    }

    /// Diagnoses session trajectory using STRACE to isolate root-cause attribution for turn failures.
    pub fn causal_diagnose_turn(&self, session: &Session) -> Option<CausalAttribution> {
        causal_diagnose_turn(session)
    }

    /// Formats an error into a structured markdown diagnostic response for the LLM,
    /// including root-cause attribution from STRACE if available for repeated turn failures.
    pub fn format_agent_feedback_with_session(
        &self,
        tool_name: &str,
        args: &Value,
        diagnosis: &ErrorDiagnosis,
        history: &CorrectionHistory,
        session: Option<&Session>,
    ) -> String {
        let mut out = self.format_agent_feedback(tool_name, args, diagnosis, history);
        if let Some(session) = session {
            if let Some(attr) = causal_diagnose_turn(session) {
                out.push_str("\n🔍 STRACE Root-Cause Attribution:\n");
                out.push_str(&format!(
                    "• Manifestation: `{}` (step {})\n",
                    attr.manifestation_node, attr.manifestation_pos
                ));
                out.push_str(&format!(
                    "• Root Cause: `{}` (step {})\n",
                    attr.root_cause_node, attr.root_cause_pos
                ));
                out.push_str(&format!("• Reason: {}\n", attr.reason));
                out.push_str(&format!(
                    "• Recommended Heuristic: {}\n",
                    attr.suggested_heuristic
                ));
                if !attr.causal_chain.is_empty() {
                    out.push_str(&format!(
                        "• Causal Dependency Chain: {:?}\n",
                        attr.causal_chain
                    ));
                }
            }
        }
        out
    }

    /// Constructs a structured correction prompt for repeated turn failures,
    /// including root-cause attribution from STRACE if available.
    pub fn construct_correction_prompt(
        &self,
        session: &Session,
        tool_name: &str,
        args: &Value,
        diagnosis: &ErrorDiagnosis,
        history: &CorrectionHistory,
    ) -> String {
        self.format_agent_feedback_with_session(tool_name, args, diagnosis, history, Some(session))
    }

    /// Formulates clear markdown diagnostic guidance to feed back to the LLM agent,
    /// enriched with root-cause attribution when session trajectory is provided.
    pub fn formulate_diagnostic_guidance_with_session(
        &self,
        diagnosis: &ErrorDiagnosis,
        tool_name: &str,
        args: &Value,
        session: Option<&Session>,
    ) -> CorrectiveAction {
        let mut action = self.formulate_diagnostic_guidance(diagnosis, tool_name, args);
        if let Some(session) = session {
            if let Some(attr) = causal_diagnose_turn(session) {
                if let CorrectiveAction::FormatDiagnosticGuidanceForAgent {
                    ref mut diagnosis_summary,
                    ref mut actionable_hints,
                    ..
                } = action
                {
                    diagnosis_summary
                        .push_str(&format!(" [STRACE Root Cause: {}]", attr.root_cause_node));
                    actionable_hints.push(format!(
                        "STRACE Root Cause: {} (step {}) - {}",
                        attr.root_cause_node, attr.root_cause_pos, attr.reason
                    ));
                    actionable_hints.push(format!(
                        "STRACE Transferable Heuristic: {}",
                        attr.suggested_heuristic
                    ));
                }
            }
        }
        action
    }

    /// Executes a tool with the self-correcting retry loop.
    ///
    /// Executes the primary tool call. If it fails, diagnoses the error, formulates
    /// an immediate corrective action (modifying arguments, executing precursors, or
    /// falling back), and retries until success or retry exhaustion.
    pub async fn execute_with_auto_correction<F, Fut>(
        &self,
        tool_name: &str,
        args: Value,
        ctx: &ToolContext,
        mut executor: F,
    ) -> CorrectionOutcome
    where
        F: FnMut(String, Value) -> Fut,
        Fut: Future<Output = anyhow::Result<String>>,
    {
        let mut current_tool = tool_name.to_string();
        let mut current_args = args;
        let mut history = self.history.write().await;
        history.clear();

        let mut turn_corrections = 0;
        loop {
            let attempt_start = Instant::now();
            let result = executor(current_tool.clone(), current_args.clone()).await;
            let duration_ms = attempt_start.elapsed().as_millis() as u64;

            match result {
                Ok(output) => {
                    let was_corrected = turn_corrections > 0;
                    history.record(
                        current_tool,
                        current_args,
                        if was_corrected {
                            "Self-correcting retry succeeded".to_string()
                        } else {
                            "Initial execution".to_string()
                        },
                        Ok(output.clone()),
                        duration_ms,
                    );

                    return CorrectionOutcome::Success {
                        output,
                        attempts: history.attempts.clone(),
                        total_corrections: turn_corrections,
                        was_corrected,
                    };
                }
                Err(err) => {
                    let raw_err_msg = err.to_string();
                    let diagnosis = self.diagnose(&current_tool, &current_args, &raw_err_msg, ctx);

                    history.record(
                        current_tool.clone(),
                        current_args.clone(),
                        format!("Attempt failed: {}", diagnosis.category.name()),
                        Err(raw_err_msg.clone()),
                        duration_ms,
                    );

                    let action = self.formulate_action(
                        &diagnosis,
                        &current_tool,
                        &current_args,
                        &history,
                        ctx,
                    );
                    debug!(target: "correction", "Formulated corrective action: {}", action.description());

                    match action {
                        CorrectiveAction::RetryWithModifiedArgs {
                            new_tool,
                            new_args,
                            reason,
                        } => {
                            info!(target: "correction", "Applying corrective argument modification: {reason}");
                            current_tool = new_tool;
                            current_args = new_args;
                            turn_corrections += 1;
                        }
                        CorrectiveAction::ExecutePrecursorAction {
                            precursor_tool,
                            precursor_args,
                            reason,
                            ..
                        } => {
                            info!(target: "correction", "Executing precursor action: {reason}");
                            let prec_start = Instant::now();
                            let prec_res =
                                executor(precursor_tool.clone(), precursor_args.clone()).await;
                            let prec_duration = prec_start.elapsed().as_millis() as u64;

                            match prec_res {
                                Ok(prec_out) => {
                                    history.record(
                                        precursor_tool,
                                        precursor_args,
                                        format!("Precursor succeeded: {reason}"),
                                        Ok(prec_out),
                                        prec_duration,
                                    );
                                    turn_corrections += 1;
                                    // Retry the original tool on next iteration
                                }
                                Err(prec_err) => {
                                    history.record(
                                        precursor_tool,
                                        precursor_args,
                                        format!("Precursor failed: {reason}"),
                                        Err(prec_err.to_string()),
                                        prec_duration,
                                    );
                                    // Precursor failed, return structured feedback
                                    let feedback = self.format_agent_feedback(
                                        &current_tool,
                                        &current_args,
                                        &diagnosis,
                                        &history,
                                    );
                                    return CorrectionOutcome::Failed {
                                        error: raw_err_msg,
                                        enriched_diagnostic: feedback,
                                        attempts: history.attempts.clone(),
                                        diagnosis,
                                    };
                                }
                            }
                        }
                        CorrectiveAction::FallbackAlternativeTool {
                            fallback_tool,
                            fallback_args,
                            reason,
                        } => {
                            info!(target: "correction", "Falling back to alternative tool: {reason}");
                            current_tool = fallback_tool;
                            current_args = fallback_args;
                            turn_corrections += 1;
                        }
                        CorrectiveAction::ExponentialBackoff { delay_ms, .. } => {
                            info!(target: "correction", "Backing off for {delay_ms}ms before retrying...");
                            tokio::time::sleep(Duration::from_millis(delay_ms)).await;
                            turn_corrections += 1;
                        }
                        CorrectiveAction::FormatDiagnosticGuidanceForAgent { .. }
                        | CorrectiveAction::AbortUnrecoverable { .. } => {
                            let feedback = self.format_agent_feedback(
                                &current_tool,
                                &current_args,
                                &diagnosis,
                                &history,
                            );
                            return CorrectionOutcome::Failed {
                                error: raw_err_msg,
                                enriched_diagnostic: feedback,
                                attempts: history.attempts.clone(),
                                diagnosis,
                            };
                        }
                    }

                    if turn_corrections > self.config.max_cascade_depth {
                        let feedback = self.format_agent_feedback(
                            &current_tool,
                            &current_args,
                            &diagnosis,
                            &history,
                        );
                        return CorrectionOutcome::Failed {
                            error: raw_err_msg,
                            enriched_diagnostic: feedback,
                            attempts: history.attempts.clone(),
                            diagnosis,
                        };
                    }
                }
            }
        }
    }

    /// Executes tool using the standard `ToolRegistry`.
    pub async fn execute_with_registry(
        &self,
        tool_name: &str,
        args: Value,
        ctx: &ToolContext,
        registry: &ToolRegistry,
    ) -> CorrectionOutcome {
        let registry_clone = registry.clone();
        let ctx_clone = ctx.clone();

        self.execute_with_auto_correction(tool_name, args, ctx, |t_name, t_args| {
            let reg = registry_clone.clone();
            let c = ctx_clone.clone();
            async move { reg.execute(&t_name, t_args, &c).await }
        })
        .await
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[test]
    fn test_error_diagnosis_file_not_found() {
        let ctx = ToolContext::default();
        let args = json!({ "path": "src/agent/corection.rs" });
        let err = "File not found: 'src/agent/corection.rs'";

        let diagnosis = ErrorAnalyzer::diagnose("read", &args, err, &ctx);
        assert_eq!(diagnosis.category.name(), "File Not Found");
        assert!(matches!(
            diagnosis.category,
            ErrorCategory::FileNotFound { .. }
        ));
    }

    #[test]
    fn test_error_diagnosis_directory_not_found() {
        let ctx = ToolContext::default();
        let args = json!({ "path": "nonexistent/subdir/output.txt", "content": "hello" });
        let err = "No such file or directory (os error 2)";

        let diagnosis = ErrorAnalyzer::diagnose("write", &args, err, &ctx);
        assert_eq!(diagnosis.category.name(), "Directory Not Found");
        assert!(diagnosis.is_recoverable);
    }

    #[test]
    fn test_error_diagnosis_command_not_found() {
        let ctx = ToolContext::default();
        let args = json!({ "command": "rg 'foo' ." });
        let err = "bash: rg: command not found";

        let diagnosis = ErrorAnalyzer::diagnose("bash", &args, err, &ctx);
        assert_eq!(diagnosis.category.name(), "Command Not Found");
        assert!(diagnosis.is_recoverable);
        if let ErrorCategory::CommandNotFound {
            suggested_alternatives,
            ..
        } = diagnosis.category
        {
            assert!(suggested_alternatives.contains(&"grep -rn".to_string()));
        } else {
            panic!("Expected CommandNotFound category");
        }
    }

    #[test]
    fn test_error_diagnosis_tool_argument_error() {
        let ctx = ToolContext::default();
        let args = json!({ "file_path": "foo.txt" });
        let err = "Missing required parameter: path";

        let diagnosis = ErrorAnalyzer::diagnose("read", &args, err, &ctx);
        assert_eq!(diagnosis.category.name(), "Tool Parameter Error");
        assert!(diagnosis.is_recoverable);
    }

    #[test]
    fn test_error_diagnosis_edit_crlf_mismatch() {
        let ctx = ToolContext::default();
        let args = json!({ "path": "foo.rs", "old_text": "a\r\nb", "new_text": "a\nb" });
        let err = "old_text not found in 'foo.rs' due to line ending differences (CRLF vs LF).";

        let diagnosis = ErrorAnalyzer::diagnose("edit", &args, err, &ctx);
        assert_eq!(diagnosis.category.name(), "Edit/Patch Mismatch");
        assert!(diagnosis.is_recoverable);
    }

    #[test]
    fn test_error_diagnosis_git_index_locked() {
        let ctx = ToolContext::default();
        let args = json!({ "command": "git commit -m 'test'" });
        let err = "fatal: Unable to create '.git/index.lock': File exists.";

        let diagnosis = ErrorAnalyzer::diagnose("bash", &args, err, &ctx);
        assert_eq!(diagnosis.category.name(), "Git Operation Error");
        assert!(diagnosis.is_recoverable);
    }

    #[test]
    fn test_error_diagnosis_rate_limit() {
        let ctx = ToolContext::default();
        let args = json!({ "query": "rust async" });
        let err = "HTTP 429 Too Many Requests: Rate limit exceeded. Retry-After: 5";

        let diagnosis = ErrorAnalyzer::diagnose("web_search", &args, err, &ctx);
        assert_eq!(diagnosis.category.name(), "Rate Limit or Timeout");
        if let ErrorCategory::RateLimitOrTimeout {
            is_rate_limited,
            retry_after_secs,
            ..
        } = diagnosis.category
        {
            assert!(is_rate_limited);
            assert_eq!(retry_after_secs, Some(5));
        } else {
            panic!("Expected RateLimitOrTimeout category");
        }
    }

    #[test]
    fn test_formulate_action_argument_normalization() {
        let engine = CorrectionEngine::default();
        let ctx = ToolContext::default();
        let args = json!({ "file": "src/main.rs" });
        let diagnosis = ErrorDiagnosis {
            category: ErrorCategory::ToolArgumentError {
                missing_param: Some("path".to_string()),
                invalid_param: None,
                suggested_fix: "Provide parameter path".to_string(),
            },
            tool_name: "read".to_string(),
            raw_error: "Missing required parameter: path".to_string(),
            root_cause: "Missing required parameter: path".to_string(),
            confidence: 0.9,
            is_recoverable: true,
            suggested_action: "Provide parameter path".to_string(),
            details: HashMap::new(),
        };

        let history = CorrectionHistory::new();
        let action = engine.formulate_action(&diagnosis, "read", &args, &history, &ctx);

        if let CorrectiveAction::RetryWithModifiedArgs {
            new_tool, new_args, ..
        } = action
        {
            assert_eq!(new_tool, "read");
            assert_eq!(
                new_args.get("path").and_then(|v| v.as_str()),
                Some("src/main.rs")
            );
        } else {
            panic!("Expected RetryWithModifiedArgs action");
        }
    }

    #[test]
    fn test_formulate_action_crlf_normalization() {
        let engine = CorrectionEngine::default();
        let ctx = ToolContext::default();
        let args =
            json!({ "path": "test.txt", "old_text": "line1\r\nline2", "new_text": "new1\r\nnew2" });
        let diagnosis = ErrorDiagnosis {
            category: ErrorCategory::PatchOrEditMismatch {
                file: "test.txt".to_string(),
                has_crlf_mismatch: true,
                multiple_matches: false,
            },
            tool_name: "edit".to_string(),
            raw_error: "CRLF line ending mismatch".to_string(),
            root_cause: "CRLF mismatch".to_string(),
            confidence: 0.95,
            is_recoverable: true,
            suggested_action: "Normalize line endings".to_string(),
            details: HashMap::new(),
        };

        let history = CorrectionHistory::new();
        let action = engine.formulate_action(&diagnosis, "edit", &args, &history, &ctx);

        if let CorrectiveAction::RetryWithModifiedArgs { new_args, .. } = action {
            assert_eq!(
                new_args.get("old_text").and_then(|v| v.as_str()),
                Some("line1\nline2")
            );
            assert_eq!(
                new_args.get("new_text").and_then(|v| v.as_str()),
                Some("new1\nnew2")
            );
        } else {
            panic!("Expected RetryWithModifiedArgs action");
        }
    }

    #[tokio::test]
    async fn test_execute_with_auto_correction_success_after_argument_fix() {
        let engine = CorrectionEngine::default();
        let ctx = ToolContext::default();

        let call_count = Arc::new(AtomicUsize::new(0));
        let call_count_clone = call_count.clone();

        let outcome = engine
            .execute_with_auto_correction(
                "read",
                json!({ "file": "src/lib.rs" }),
                &ctx,
                |tool, args| {
                    let count = call_count_clone.fetch_add(1, Ordering::SeqCst);
                    async move {
                        if count == 0 {
                            // First call fails because "path" is missing
                            if args.get("path").is_none() {
                                anyhow::bail!("Missing required parameter: path");
                            }
                        }
                        // Second call with corrected args succeeds
                        assert_eq!(tool, "read");
                        assert_eq!(
                            args.get("path").and_then(|v| v.as_str()),
                            Some("src/lib.rs")
                        );
                        Ok("file content here".to_string())
                    }
                },
            )
            .await;

        assert!(outcome.is_success());
        assert_eq!(outcome.output_or_error(), "file content here");
        assert!(outcome.was_corrected());
        assert_eq!(call_count.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn test_execute_with_auto_correction_precursor_action() {
        let engine = CorrectionEngine::default();
        let ctx = ToolContext::default();

        let call_count = Arc::new(AtomicUsize::new(0));
        let call_count_clone = call_count.clone();

        let outcome = engine
            .execute_with_auto_correction(
                "write",
                json!({ "path": "subdir/output.txt", "content": "data" }),
                &ctx,
                |tool, args| {
                    let count = call_count_clone.fetch_add(1, Ordering::SeqCst);
                    async move {
                        if count == 0 {
                            anyhow::bail!("No such file or directory (os error 2)");
                        } else if count == 1 {
                            // Precursor mkdir -p succeeds
                            assert_eq!(tool, "bash");
                            assert!(args
                                .get("command")
                                .unwrap()
                                .as_str()
                                .unwrap()
                                .contains("mkdir -p"));
                            Ok("Directory created".to_string())
                        } else {
                            // Retried original write succeeds
                            assert_eq!(tool, "write");
                            Ok("Wrote 4 bytes".to_string())
                        }
                    }
                },
            )
            .await;

        assert!(outcome.is_success());
        assert_eq!(outcome.output_or_error(), "Wrote 4 bytes");
        assert!(outcome.was_corrected());
        assert_eq!(call_count.load(Ordering::SeqCst), 3);
    }

    #[test]
    fn test_parse_rust_compiler_errors() {
        let stderr = r#"
error[E0425]: cannot find value `foo` in this scope
  --> src/agent/mod.rs:42:15
   |
42 |     let x = foo + 1;
   |             ^^^ not found in this scope
   |
help: consider importing this constant
   |
1  + use crate::agent::foo;
   |

error[E0308]: mismatched types
  --> src/agent/loop_runner.rs:100:5
   |
100 |     5
   |     ^ expected `String`, found integer
"#;

        let diags = parse_rust_compiler_errors(stderr);
        assert_eq!(diags.len(), 2);
        assert_eq!(diags[0].code.as_deref(), Some("E0425"));
        assert_eq!(diags[0].file, "src/agent/mod.rs");
        assert_eq!(diags[0].line, 42);
        assert_eq!(diags[0].column, 15);
        assert!(diags[0]
            .suggestion
            .as_ref()
            .unwrap()
            .contains("use crate::agent::foo"));

        assert_eq!(diags[1].code.as_deref(), Some("E0308"));
        assert_eq!(diags[1].file, "src/agent/loop_runner.rs");
        assert_eq!(diags[1].line, 100);
        assert_eq!(diags[1].column, 5);
    }

    #[test]
    fn test_parse_ts_compiler_errors() {
        let stderr = "src/index.ts(14,5): error TS2322: Type 'string' is not assignable to type 'number'.\nsrc/app.ts(2,1): error TS2304: Cannot find name 'React'.";
        let diags = parse_ts_compiler_errors(stderr);
        assert_eq!(diags.len(), 2);
        assert_eq!(diags[0].code.as_deref(), Some("TS2322"));
        assert_eq!(diags[0].file, "src/index.ts");
        assert_eq!(diags[0].line, 14);
        assert_eq!(diags[0].column, 5);
        assert_eq!(
            diags[0].message,
            "Type 'string' is not assignable to type 'number'."
        );

        assert_eq!(diags[1].code.as_deref(), Some("TS2304"));
        assert_eq!(diags[1].file, "src/app.ts");
        assert_eq!(diags[1].line, 2);
        assert_eq!(diags[1].column, 1);
    }

    #[test]
    fn test_parse_python_traceback() {
        let stderr = r#"
Traceback (most recent call last):
  File "main.py", line 12, in <module>
    compute_total()
  File "calc.py", line 45, in compute_total
    return 10 / 0
ZeroDivisionError: division by zero
"#;

        let diag = parse_python_traceback(stderr);
        assert!(diag.is_some());
        let d = diag.unwrap();
        assert_eq!(d.exception_type, "ZeroDivisionError");
        assert_eq!(d.message, "division by zero");
        assert_eq!(d.file, "calc.py");
        assert_eq!(d.line, 45);
        assert_eq!(d.code_snippet.as_deref(), Some("return 10 / 0"));
    }

    #[test]
    fn test_clean_path_string() {
        assert_eq!(clean_path_string("'src/main.rs'"), "src/main.rs");
        assert_eq!(clean_path_string("\"src/main.rs:42:10\""), "src/main.rs");
        assert_eq!(clean_path_string("src/main.rs:100"), "src/main.rs");
    }

    #[test]
    fn test_find_fuzzy_file_matches() {
        let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
        // "correction" without .rs -> should find "src/agent/correction.rs" or similar
        let matches = find_fuzzy_file_matches("src/agent/correction", &cwd);
        assert!(matches.iter().any(|m| m.ends_with("correction.rs")));
    }

    #[tokio::test]
    async fn test_fallback_command_execution() {
        let engine = CorrectionEngine::default();
        let ctx = ToolContext::default();
        let call_count = Arc::new(AtomicUsize::new(0));
        let call_count_clone = call_count.clone();

        let outcome = engine
            .execute_with_auto_correction(
                "bash",
                json!({ "command": "rg 'pattern' src" }),
                &ctx,
                |tool, args| {
                    let count = call_count_clone.fetch_add(1, Ordering::SeqCst);
                    async move {
                        if count == 0 {
                            anyhow::bail!("bash: rg: command not found");
                        } else {
                            assert_eq!(tool, "bash");
                            assert_eq!(
                                args.get("command").unwrap().as_str().unwrap(),
                                "grep -rn 'pattern' src"
                            );
                            Ok("matched line".to_string())
                        }
                    }
                },
            )
            .await;

        assert!(outcome.is_success());
        assert_eq!(outcome.output_or_error(), "matched line");
        assert!(outcome.was_corrected());
        assert_eq!(call_count.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn test_rate_limit_backoff() {
        let mut config = CorrectionConfig::default();
        config.initial_backoff_ms = 10;
        config.max_backoff_ms = 50;
        let engine = CorrectionEngine::new(config);
        let ctx = ToolContext::default();
        let call_count = Arc::new(AtomicUsize::new(0));
        let call_count_clone = call_count.clone();

        let outcome = engine
            .execute_with_auto_correction(
                "web_search",
                json!({ "query": "fusion ai" }),
                &ctx,
                |_tool, _args| {
                    let count = call_count_clone.fetch_add(1, Ordering::SeqCst);
                    async move {
                        if count == 0 {
                            anyhow::bail!("429 Too Many Requests");
                        } else {
                            Ok("search results".to_string())
                        }
                    }
                },
            )
            .await;

        assert!(outcome.is_success());
        assert_eq!(outcome.output_or_error(), "search results");
        assert_eq!(call_count.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn test_history_cycle_prevention() {
        let mut history = CorrectionHistory::new();
        let tool = "read";
        let args = json!({ "path": "missing.rs" });
        let action = "retry with missing.rs";

        assert!(!history.has_attempted(tool, &args, action));
        history.record(
            tool.to_string(),
            args.clone(),
            action.to_string(),
            Err("failed".to_string()),
            10,
        );
        assert!(history.has_attempted(tool, &args, action));
        assert_eq!(history.count(), 1);
        assert_eq!(history.successful_count(), 0);
    }

    #[test]
    fn test_format_agent_feedback_output() {
        let engine = CorrectionEngine::default();
        let ctx = ToolContext::default();
        let args = json!({ "path": "src/missing_file.rs" });
        let diagnosis =
            engine.diagnose("read", &args, "File not found: 'src/missing_file.rs'", &ctx);
        let mut history = CorrectionHistory::new();
        history.record(
            "read".to_string(),
            args.clone(),
            "Attempted file read".to_string(),
            Err("File not found".to_string()),
            5,
        );

        let feedback = engine.format_agent_feedback("read", &args, &diagnosis, &history);
        assert!(feedback.contains("Tool Execution Failed: `read`"));
        assert!(feedback.contains("File Not Found"));
        assert!(feedback.contains("Automated Recovery History"));
        assert!(feedback.contains("Recommended Next Steps"));
    }

    #[test]
    fn test_causal_diagnose_turn_and_correction_prompt() {
        use crate::provider::types::ToolCall;

        let engine = CorrectionEngine::default();
        let mut session = Session::new("claude-3-7-sonnet");
        session.add_user_message("Please refactor the authentication schema.");
        session.add_assistant_with_tools(
            "I will inspect the migration file.",
            vec![ToolCall {
                id: "call_read_migration_01".to_string(),
                name: "read".to_string(),
                arguments: r#"{"path": "migrations/auth.sql"}"#.to_string(),
            }],
        );
        session.add_tool_result(
            "call_read_migration_01",
            "Error: file not found: migrations/auth.sql",
        );

        // Verify causal_diagnose_turn
        let attr = causal_diagnose_turn(&session);
        assert!(attr.is_some());
        let attribution = attr.unwrap();
        assert_eq!(attribution.manifestation_node, "ToolResult");
        assert_eq!(attribution.root_cause_node, "User");

        // Verify engine method
        let engine_attr = engine.causal_diagnose_turn(&session);
        assert!(engine_attr.is_some());

        // Verify construct_correction_prompt standalone
        let prompt = construct_correction_prompt(&session, "read", "File not found");
        assert!(prompt.contains("Repeated Failure in `read`"));
        assert!(prompt.contains("STRACE Root-Cause Attribution"));
        assert!(prompt.contains("Manifestation: `ToolResult`"));
        assert!(prompt.contains("Root Cause: `User`"));

        // Verify construct_correction_prompt on engine
        let ctx = ToolContext::default();
        let args = json!({ "path": "migrations/auth.sql" });
        let diagnosis = engine.diagnose("read", &args, "File not found: migrations/auth.sql", &ctx);
        let mut history = CorrectionHistory::new();
        history.record(
            "read".to_string(),
            args.clone(),
            "retry read".to_string(),
            Err("File not found".to_string()),
            15,
        );
        history.record(
            "read".to_string(),
            args.clone(),
            "second retry read".to_string(),
            Err("File not found".to_string()),
            18,
        );

        let engine_prompt =
            engine.construct_correction_prompt(&session, "read", &args, &diagnosis, &history);
        assert!(engine_prompt.contains("STRACE Root-Cause Attribution"));
        assert!(engine_prompt.contains("Manifestation: `ToolResult`"));
        assert!(engine_prompt.contains("Root Cause: `User`"));
    }
}
