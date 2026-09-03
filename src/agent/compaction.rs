use serde::{Deserialize, Serialize};
use std::collections::HashSet;

use crate::agent::session::Session;
use crate::config::Config;
use crate::provider::types::{Message, Role, ToolCall};
use crate::provider::LlmClient;

// ============================================================================
// Re-export token estimators from tokens module
// ============================================================================
pub use crate::agent::tokens::{
    estimate_message_tokens, estimate_messages_tokens, estimate_text_tokens,
    estimate_tool_call_tokens, is_context_overflow, model_context_limit,
};

// ============================================================================
// Context Budget Status & Enums
// ============================================================================

/// Qualitative assessment of context window token budget utilization.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ContextBudgetStatus {
    /// Token usage is healthy (< 60% of context window).
    Healthy,
    /// Token usage is moderately elevated (60% - 80%).
    ApproachingThreshold,
    /// Token usage has crossed the trigger threshold (>= 80%); compaction recommended.
    CompactionRequired,
    /// Token usage is dangerously close to limit (>= 95%); immediate truncation required.
    Critical,
}

impl ContextBudgetStatus {
    /// Returns true if compaction is required or critical.
    pub fn should_compact(&self) -> bool {
        matches!(self, Self::CompactionRequired | Self::Critical)
    }
}

/// Strategy applied during history compaction.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CompactionStrategy {
    /// Multi-tiered intelligent compaction:
    /// 1. Prunes oversized older tool outputs (preserving recent turns).
    /// 2. If still over budget, summarizes older turns into a structured summary.
    Auto,
    /// Summarizes older turns into a single structured summary message.
    Summarize,
    /// Only compresses large tool result messages, leaving all turn boundaries intact.
    PruneToolsOnly,
    /// Drops older turns, keeping only the most recent N turns intact (pure sliding window).
    SlidingWindow,
}

/// Turn classification type within conversation groups.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TurnType {
    /// System initialization prompt.
    System,
    /// User query or command.
    User,
    /// Plain assistant response or explanation.
    Assistant,
    /// Tool invocation request and corresponding tool results.
    ToolExecution,
}

// ============================================================================
// Compaction Configuration
// ============================================================================

/// Configuration settings governing conversation history compaction.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompactionConfig {
    /// Explicit context limit in tokens. If None, auto-detected from model name.
    pub context_limit: Option<usize>,
    /// Fraction of context limit that triggers compaction (e.g. 0.80 for 80%).
    pub threshold: f32,
    /// Target fraction of context limit to reach after compaction (e.g. 0.50 for 50%).
    pub target_ratio: f32,
    /// Number of recent conversation turns to keep completely uncompressed (default: 4).
    pub preserve_recent_turns: usize,
    /// Whether to preserve the initial user message/goal at the start (default: true).
    pub preserve_initial_goal: bool,
    /// Maximum allowed tokens in an older tool result before truncation (default: 400).
    pub max_tool_output_tokens: usize,
    /// Compaction strategy to apply.
    pub strategy: CompactionStrategy,
}

impl Default for CompactionConfig {
    fn default() -> Self {
        Self {
            context_limit: None,
            threshold: 0.80,
            target_ratio: 0.50,
            preserve_recent_turns: 4,
            preserve_initial_goal: true,
            max_tool_output_tokens: 400,
            strategy: CompactionStrategy::Auto,
        }
    }
}

impl CompactionConfig {
    /// Creates a new configuration with the specified context limit in tokens.
    pub fn new(context_limit: usize) -> Self {
        Self {
            context_limit: Some(context_limit),
            ..Default::default()
        }
    }

    /// Sets the overflow trigger threshold (e.g. 0.80 for 80%).
    pub fn with_threshold(mut self, threshold: f32) -> Self {
        self.threshold = threshold.clamp(0.1, 0.99);
        self
    }

    /// Sets the target post-compaction ratio (e.g. 0.50 for 50%).
    pub fn with_target_ratio(mut self, target_ratio: f32) -> Self {
        self.target_ratio = target_ratio.clamp(0.05, self.threshold);
        self
    }

    /// Sets how many recent turns to preserve intact.
    pub fn with_preserve_recent_turns(mut self, turns: usize) -> Self {
        self.preserve_recent_turns = turns.max(1);
        self
    }

    /// Sets whether to preserve the initial user message.
    pub fn with_preserve_initial_goal(mut self, preserve: bool) -> Self {
        self.preserve_initial_goal = preserve;
        self
    }

    /// Sets the maximum tokens allowed per older tool output before truncation.
    pub fn with_max_tool_output_tokens(mut self, max_tokens: usize) -> Self {
        self.max_tool_output_tokens = max_tokens.max(50);
        self
    }

    /// Sets the compaction strategy.
    pub fn with_strategy(mut self, strategy: CompactionStrategy) -> Self {
        self.strategy = strategy;
        self
    }
}

// ============================================================================
// Compaction Plan & Results
// ============================================================================

/// Detailed pre-compaction analysis and plan.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompactionPlan {
    /// Current estimated tokens in the message history.
    pub current_tokens: usize,
    /// Effective context limit for the model in tokens.
    pub limit_tokens: usize,
    /// Current context utilization ratio (0.0 to 1.0+).
    pub utilization: f32,
    /// Configured trigger threshold (e.g. 0.80).
    pub trigger_threshold: f32,
    /// Target tokens after compaction.
    pub target_tokens: usize,
    /// Whether compaction will be triggered.
    pub will_compact: bool,
    /// Current budget status.
    pub budget_status: ContextBudgetStatus,
    /// Total atomic turns detected.
    pub total_turns: usize,
    /// Number of recent turns configured to be preserved intact.
    pub turns_to_preserve: usize,
    /// Number of older turns subject to compaction/summarization.
    pub turns_to_compact: usize,
    /// Strategy recommended/configured for this plan.
    pub strategy: CompactionStrategy,
}

/// Metrics and metadata describing the outcome of a compaction operation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompactionResult {
    /// Whether any compaction was performed.
    pub compacted: bool,
    /// Estimated tokens before compaction.
    pub original_tokens: usize,
    /// Estimated tokens after compaction.
    pub compacted_tokens: usize,
    /// Tokens saved by compaction.
    pub tokens_saved: usize,
    /// Number of messages before compaction.
    pub original_messages: usize,
    /// Number of messages after compaction.
    pub compacted_messages: usize,
    /// Number of messages pruned or summarized away.
    pub messages_removed: usize,
    /// The generated summary text, if summarization occurred.
    pub summary: Option<String>,
    /// The compaction strategy that was executed.
    pub strategy_used: CompactionStrategy,
}

impl CompactionResult {
    /// Creates an empty result indicating no compaction was needed.
    pub fn uncompacted(tokens: usize, messages_count: usize) -> Self {
        Self {
            compacted: false,
            original_tokens: tokens,
            compacted_tokens: tokens,
            tokens_saved: 0,
            original_messages: messages_count,
            compacted_messages: messages_count,
            messages_removed: 0,
            summary: None,
            strategy_used: CompactionStrategy::Auto,
        }
    }

    /// Returns a concise, human-readable summary of the compaction result.
    pub fn format_summary(&self) -> String {
        if !self.compacted {
            return format!(
                "No compaction needed ({} tokens in {} messages).",
                self.original_tokens, self.original_messages
            );
        }

        let pct_saved = if self.original_tokens > 0 {
            (self.tokens_saved as f64 / self.original_tokens as f64) * 100.0
        } else {
            0.0
        };

        format!(
            "Compacted {} messages -> {} ({} -> {} tokens, saved {} tokens / {:.1}%)",
            self.original_messages,
            self.compacted_messages,
            self.original_tokens,
            self.compacted_tokens,
            self.tokens_saved,
            pct_saved
        )
    }
}

// ============================================================================
// Safe Unicode Text Slicing Helpers
// ============================================================================

/// Safely extracts up to `max_chars` characters from the start of a UTF-8 string without panicking.
fn safe_char_head(s: &str, max_chars: usize) -> &str {
    match s.char_indices().nth(max_chars) {
        Some((idx, _)) => &s[..idx],
        None => s,
    }
}

/// Safely extracts up to `max_chars` characters from the end of a UTF-8 string without panicking.
fn safe_char_tail(s: &str, max_chars: usize) -> &str {
    let total_chars = s.chars().count();
    if total_chars <= max_chars {
        return s;
    }
    let skip = total_chars - max_chars;
    match s.char_indices().nth(skip) {
        Some((idx, _)) => &s[idx..],
        None => s,
    }
}

// ============================================================================
// Atomic Turn Grouping & Structure
// ============================================================================

/// Represents an atomic conversation turn that should not be split arbitrarily.
/// For instance, an Assistant message with `tool_calls` and its following `Tool`
/// response messages MUST remain together to maintain valid LLM protocol state.
#[derive(Debug, Clone)]
pub struct TurnGroup {
    /// The messages comprising this turn.
    pub messages: Vec<Message>,
    /// Whether this turn contains tool calls.
    pub is_tool_call: bool,
    /// Estimated tokens for this turn group.
    pub estimated_tokens: usize,
}

impl TurnGroup {
    /// Returns the semantic turn type of this group.
    pub fn turn_type(&self) -> TurnType {
        if self.is_tool_call {
            return TurnType::ToolExecution;
        }
        if let Some(first) = self.messages.first() {
            match first.role {
                Role::System => TurnType::System,
                Role::User => TurnType::User,
                Role::Assistant => TurnType::Assistant,
                Role::Tool => TurnType::ToolExecution,
            }
        } else {
            TurnType::User
        }
    }

    /// Returns true if this turn contains an error or failure indicator.
    pub fn has_error_or_failure(&self) -> bool {
        for m in &self.messages {
            let lower = m.content.to_lowercase();
            if lower.contains("error:")
                || lower.contains("failed:")
                || lower.contains("fatal:")
                || lower.contains("panicked at")
                || lower.contains("exit code: 1")
                || lower.contains("command not found")
            {
                return true;
            }
        }
        false
    }

    /// Extracts referenced file paths from tool calls and text within this turn.
    pub fn extracted_file_paths(&self) -> Vec<String> {
        let mut paths = Vec::new();
        for m in &self.messages {
            if let Some(tool_calls) = &m.tool_calls {
                for tc in tool_calls {
                    if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(&tc.arguments) {
                        if let Some(p) = parsed.get("path").and_then(|v| v.as_str()) {
                            paths.push(p.to_string());
                        }
                    }
                }
            }
        }
        paths
    }

    /// Returns the primary text summary / first line of the turn.
    pub fn primary_text(&self) -> String {
        for m in &self.messages {
            let trimmed = m.content.trim();
            if !trimmed.is_empty() {
                let first_line = trimmed.lines().next().unwrap_or(trimmed);
                return safe_char_head(first_line, 120).to_string();
            }
        }
        String::new()
    }
}

/// Groups a slice of messages into atomic turn groups, strictly respecting
/// tool call / tool result pairings.
pub fn group_into_turns(messages: &[Message]) -> Vec<TurnGroup> {
    let mut groups: Vec<TurnGroup> = Vec::new();
    let mut i = 0;

    while i < messages.len() {
        let msg = &messages[i];

        // System messages form their own independent groups
        if msg.role == Role::System {
            groups.push(TurnGroup {
                messages: vec![msg.clone()],
                is_tool_call: false,
                estimated_tokens: estimate_message_tokens(msg),
            });
            i += 1;
            continue;
        }

        // Assistant message with tool calls: gather it and ALL following Tool messages matching it
        if msg.role == Role::Assistant && msg.tool_calls.as_ref().map_or(false, |tc| !tc.is_empty())
        {
            let mut group_msgs = vec![msg.clone()];
            let mut turn_tokens = estimate_message_tokens(msg);
            let expected_ids: HashSet<String> = msg
                .tool_calls
                .as_ref()
                .map(|tcs| tcs.iter().map(|tc| tc.id.clone()).collect())
                .unwrap_or_default();

            i += 1;

            // Collect corresponding Tool results matching the expected call IDs
            while i < messages.len() {
                let next_msg = &messages[i];
                if next_msg.role == Role::Tool {
                    if let Some(call_id) = &next_msg.tool_call_id {
                        if expected_ids.contains(call_id) {
                            turn_tokens += estimate_message_tokens(next_msg);
                            group_msgs.push(next_msg.clone());
                            i += 1;
                            continue;
                        }
                    }
                }
                break;
            }

            groups.push(TurnGroup {
                messages: group_msgs,
                is_tool_call: true,
                estimated_tokens: turn_tokens,
            });
            continue;
        }

        // Standard user or plain assistant message
        let token_count = estimate_message_tokens(msg);
        groups.push(TurnGroup {
            messages: vec![msg.clone()],
            is_tool_call: false,
            estimated_tokens: token_count,
        });
        i += 1;
    }

    groups
}

// ============================================================================
// Tool Output Pruning
// ============================================================================

/// Truncates a lengthy tool result string, preserving head and tail lines with
/// an informative elision notice. Operates 100% UTF-8 safe without panics.
pub fn truncate_tool_output(content: &str, max_tokens: usize) -> String {
    let current_tokens = estimate_text_tokens(content);
    if current_tokens <= max_tokens {
        return content.to_string();
    }

    let lines: Vec<&str> = content.lines().collect();
    if lines.len() <= 10 {
        // Line-count is small but character count is large (e.g. minified single line / JSON blob)
        let max_chars = max_tokens * 4;
        let char_count = content.chars().count();
        if char_count > max_chars {
            let half = max_chars / 2;
            let head = safe_char_head(content, half);
            let tail = safe_char_tail(content, half);
            return format!(
                "{}\n... [elided for context compaction] ...\n{}",
                head, tail
            );
        }
        return content.to_string();
    }

    // Keep top 10-25 lines and bottom 10-25 lines
    let total_lines = lines.len();
    let keep_head_lines = (total_lines / 10).clamp(4, 25);
    let keep_tail_lines = (total_lines / 10).clamp(4, 25);

    if keep_head_lines + keep_tail_lines >= total_lines {
        return content.to_string();
    }

    let elided_count = total_lines - keep_head_lines - keep_tail_lines;
    let head = lines[..keep_head_lines].join("\n");
    let tail = lines[total_lines - keep_tail_lines..].join("\n");

    format!(
        "{}\n\n... [{} lines elided for context compaction] ...\n\n{}",
        head, elided_count, tail
    )
}

/// Prunes older tool result messages in-place across the message list, leaving
/// the most recent `preserve_recent_turns` turns untouched.
pub fn prune_older_tool_outputs(
    messages: &mut [Message],
    preserve_recent_turns: usize,
    max_tool_tokens: usize,
) -> usize {
    let turn_groups = group_into_turns(messages);
    let total_groups = turn_groups.len();

    // Determine the boundary index for older turns
    let older_group_limit = if total_groups > preserve_recent_turns {
        total_groups - preserve_recent_turns
    } else {
        0
    };

    let mut msg_idx = 0;
    let mut tokens_saved = 0usize;

    for (group_idx, group) in turn_groups.iter().enumerate() {
        for msg in &group.messages {
            if group_idx < older_group_limit && msg.role == Role::Tool {
                let old_tokens = estimate_text_tokens(&messages[msg_idx].content);
                if old_tokens > max_tool_tokens {
                    let truncated =
                        truncate_tool_output(&messages[msg_idx].content, max_tool_tokens);
                    let new_tokens = estimate_text_tokens(&truncated);
                    if new_tokens < old_tokens {
                        tokens_saved += old_tokens - new_tokens;
                        messages[msg_idx].content = truncated;
                    }
                }
            }
            msg_idx += 1;
        }
    }

    tokens_saved
}

// ============================================================================
// Heuristic Structured Summarization
// ============================================================================

/// Generates an intelligent, structured markdown summary of older messages
/// without requiring an LLM call. Runs in sub-millisecond time.
pub fn generate_heuristic_summary(older_messages: &[Message]) -> String {
    let mut user_requests: Vec<String> = Vec::new();
    let mut actions_taken: Vec<String> = Vec::new();
    let mut assistant_notes: Vec<String> = Vec::new();
    let mut files_referenced: HashSet<String> = HashSet::new();
    let mut critical_findings: Vec<String> = Vec::new();

    for msg in older_messages {
        match msg.role {
            Role::User => {
                let trimmed = msg.content.trim();
                if !trimmed.is_empty() {
                    let first_line = trimmed.lines().next().unwrap_or(trimmed);
                    let snippet = safe_char_head(first_line, 140);
                    if snippet.len() < first_line.len() {
                        user_requests.push(format!("{}...", snippet));
                    } else {
                        user_requests.push(snippet.to_string());
                    }
                }
            }
            Role::Assistant => {
                if let Some(tool_calls) = &msg.tool_calls {
                    for tc in tool_calls {
                        let name = &tc.name;
                        let arg_snippet = if let Ok(parsed) =
                            serde_json::from_str::<serde_json::Value>(&tc.arguments)
                        {
                            if let Some(path) = parsed.get("path").and_then(|p| p.as_str()) {
                                files_referenced.insert(path.to_string());
                                format!("path: `{}`", path)
                            } else if let Some(cmd) = parsed.get("command").and_then(|c| c.as_str())
                            {
                                let first_cmd = cmd.lines().next().unwrap_or(cmd);
                                format!("cmd: `{}`", safe_char_head(first_cmd, 60))
                            } else if let Some(pat) = parsed.get("pattern").and_then(|p| p.as_str())
                            {
                                format!("pattern: `{}`", safe_char_head(pat, 40))
                            } else {
                                safe_char_head(&tc.arguments, 40).to_string()
                            }
                        } else {
                            safe_char_head(&tc.arguments, 40).to_string()
                        };

                        actions_taken.push(format!("Tool `{}` ({})", name, arg_snippet));
                    }
                }

                let trimmed = msg.content.trim();
                if !trimmed.is_empty() && msg.tool_calls.is_none() {
                    let first_line = trimmed.lines().next().unwrap_or(trimmed);
                    if !first_line.is_empty() {
                        let snippet = safe_char_head(first_line, 120);
                        if snippet.len() < first_line.len() {
                            assistant_notes.push(format!("{}...", snippet));
                        } else {
                            assistant_notes.push(snippet.to_string());
                        }
                    }
                }
            }
            Role::Tool => {
                let lower = msg.content.to_lowercase();
                if lower.contains("error") || lower.contains("failed") || lower.contains("fatal") {
                    let first_line = msg
                        .content
                        .trim()
                        .lines()
                        .next()
                        .unwrap_or("Error in tool output");
                    critical_findings
                        .push(format!("Tool failure: {}", safe_char_head(first_line, 100)));
                }
            }
            Role::System => {}
        }
    }

    let mut summary = String::from("### Compacted Conversation Context\n");
    summary.push_str("*(The preceding older conversation history was compacted to stay within context window limits.)*\n\n");

    if !user_requests.is_empty() {
        summary.push_str("#### User Inquiries & Goals:\n");
        for req in user_requests.iter().take(6) {
            summary.push_str(&format!("- {}\n", req));
        }
        if user_requests.len() > 6 {
            summary.push_str(&format!(
                "- *(... plus {} earlier inquiries)*\n",
                user_requests.len() - 6
            ));
        }
        summary.push('\n');
    }

    if !files_referenced.is_empty() {
        summary.push_str("#### Files Inspected / Modified:\n");
        let mut sorted_files: Vec<_> = files_referenced.into_iter().collect();
        sorted_files.sort();
        for file in sorted_files.iter().take(10) {
            summary.push_str(&format!("- `{}`\n", file));
        }
        if sorted_files.len() > 10 {
            summary.push_str(&format!(
                "- *(... plus {} additional files)*\n",
                sorted_files.len() - 10
            ));
        }
        summary.push('\n');
    }

    if !actions_taken.is_empty() {
        summary.push_str("#### Key Actions Taken:\n");
        for action in actions_taken.iter().take(8) {
            summary.push_str(&format!("- {}\n", action));
        }
        if actions_taken.len() > 8 {
            summary.push_str(&format!(
                "- *(... plus {} additional operations)*\n",
                actions_taken.len() - 8
            ));
        }
        summary.push('\n');
    }

    if !critical_findings.is_empty() {
        summary.push_str("#### Critical Notices & Tool Failures:\n");
        for crit in critical_findings.iter().take(4) {
            summary.push_str(&format!("- {}\n", crit));
        }
        summary.push('\n');
    }

    if !assistant_notes.is_empty() {
        summary.push_str("#### Assistant Findings & Decisions:\n");
        for note in assistant_notes.iter().take(5) {
            summary.push_str(&format!("- {}\n", note));
        }
        summary.push('\n');
    }

    summary.push_str(
        "*(Resume conversation maintaining continuity with these previous actions and findings.)*",
    );
    summary
}

// ============================================================================
// Compactor Engine
// ============================================================================

/// The primary context compaction engine responsible for evaluating token
/// budgets, grouping atomic turns, pruning bloated tool outputs, and generating
/// concise summaries of older conversation turns.
#[derive(Debug, Clone)]
pub struct Compactor {
    pub config: CompactionConfig,
}

impl Default for Compactor {
    fn default() -> Self {
        Self {
            config: CompactionConfig::default(),
        }
    }
}

impl Compactor {
    /// Creates a new Compactor with an explicit token limit.
    pub fn new(context_limit: usize) -> Self {
        Self {
            config: CompactionConfig::new(context_limit),
        }
    }

    /// Creates a Compactor with full custom configuration.
    pub fn with_config(config: CompactionConfig) -> Self {
        Self { config }
    }

    /// Sets the context overflow threshold (e.g. 0.80).
    pub fn with_threshold(mut self, threshold: f32) -> Self {
        self.config = self.config.with_threshold(threshold);
        self
    }

    /// Sets the post-compaction target ratio (e.g. 0.50).
    pub fn with_target_ratio(mut self, target_ratio: f32) -> Self {
        self.config = self.config.with_target_ratio(target_ratio);
        self
    }

    /// Sets the number of recent turns to preserve intact.
    pub fn with_preserve_turns(mut self, turns: usize) -> Self {
        self.config = self.config.with_preserve_recent_turns(turns);
        self
    }

    /// Sets whether to preserve the initial user message.
    pub fn with_preserve_initial_goal(mut self, preserve: bool) -> Self {
        self.config = self.config.with_preserve_initial_goal(preserve);
        self
    }

    /// Sets the compaction strategy.
    pub fn with_strategy(mut self, strategy: CompactionStrategy) -> Self {
        self.config = self.config.with_strategy(strategy);
        self
    }

    /// Resolves the effective context limit, using the configured limit or auto-detecting from the model name.
    pub fn resolve_context_limit(&self, model: &str) -> usize {
        self.config
            .context_limit
            .unwrap_or_else(|| model_context_limit(model))
    }

    /// Computes the context utilization ratio (0.0 to 1.0+) of the message list.
    pub fn context_utilization(&self, messages: &[Message], model: &str) -> f32 {
        let limit = self.resolve_context_limit(model);
        if limit == 0 {
            return 0.0;
        }
        let current_tokens = estimate_messages_tokens(messages);
        current_tokens as f32 / limit as f32
    }

    /// Evaluates the qualitative budget status of the current conversation history.
    pub fn budget_status(&self, messages: &[Message], model: &str) -> ContextBudgetStatus {
        let util = self.context_utilization(messages, model);
        if util >= 0.95 {
            ContextBudgetStatus::Critical
        } else if util >= self.config.threshold {
            ContextBudgetStatus::CompactionRequired
        } else if util >= 0.60 {
            ContextBudgetStatus::ApproachingThreshold
        } else {
            ContextBudgetStatus::Healthy
        }
    }

    /// Determines whether the given conversation messages require compaction.
    pub fn needs_compaction(&self, messages: &[Message], model: &str) -> bool {
        let limit = self.resolve_context_limit(model);
        let current_tokens = estimate_messages_tokens(messages);
        let budget = (limit as f32 * self.config.threshold) as usize;
        current_tokens >= budget
    }

    /// Calculates a pre-compaction plan and analysis without modifying the messages.
    pub fn calculate_compaction_plan(&self, messages: &[Message], model: &str) -> CompactionPlan {
        let limit = self.resolve_context_limit(model);
        let current_tokens = estimate_messages_tokens(messages);
        let utilization = if limit > 0 {
            current_tokens as f32 / limit as f32
        } else {
            0.0
        };
        let target_tokens = (limit as f32 * self.config.target_ratio) as usize;
        let will_compact = current_tokens >= (limit as f32 * self.config.threshold) as usize;
        let budget_status = self.budget_status(messages, model);

        let turn_groups = group_into_turns(messages);
        let total_turns = turn_groups.len();
        let turns_to_preserve = self.config.preserve_recent_turns.min(total_turns);
        let turns_to_compact = total_turns.saturating_sub(turns_to_preserve);

        CompactionPlan {
            current_tokens,
            limit_tokens: limit,
            utilization,
            trigger_threshold: self.config.threshold,
            target_tokens,
            will_compact,
            budget_status,
            total_turns,
            turns_to_preserve,
            turns_to_compact,
            strategy: self.config.strategy,
        }
    }

    /// Performs synchronous compaction on a slice of messages.
    /// Returns the new compacted messages vector and the detailed `CompactionResult`.
    pub fn compact(&self, messages: &[Message], model: &str) -> (Vec<Message>, CompactionResult) {
        let original_tokens = estimate_messages_tokens(messages);
        let original_count = messages.len();

        let limit = self.resolve_context_limit(model);
        let budget = (limit as f32 * self.config.threshold) as usize;
        let target_tokens = (limit as f32 * self.config.target_ratio) as usize;

        // If below budget, no compaction required
        if original_tokens < budget {
            return (
                messages.to_vec(),
                CompactionResult::uncompacted(original_tokens, original_count),
            );
        }

        let mut working_messages = messages.to_vec();

        // --------------------------------------------------------------------
        // Stage 1: Tool Output Pruning (if Auto or PruneToolsOnly)
        // --------------------------------------------------------------------
        if matches!(
            self.config.strategy,
            CompactionStrategy::Auto | CompactionStrategy::PruneToolsOnly
        ) {
            prune_older_tool_outputs(
                &mut working_messages,
                self.config.preserve_recent_turns,
                self.config.max_tool_output_tokens,
            );

            let pruned_tokens = estimate_messages_tokens(&working_messages);

            // If pruning tool outputs brought us under target, we can return early!
            if self.config.strategy == CompactionStrategy::PruneToolsOnly
                || pruned_tokens <= target_tokens
            {
                let saved = original_tokens.saturating_sub(pruned_tokens);
                return (
                    working_messages,
                    CompactionResult {
                        compacted: true,
                        original_tokens,
                        compacted_tokens: pruned_tokens,
                        tokens_saved: saved,
                        original_messages: original_count,
                        compacted_messages: original_count,
                        messages_removed: 0,
                        summary: Some("Pruned verbose tool outputs from older turns.".to_string()),
                        strategy_used: CompactionStrategy::PruneToolsOnly,
                    },
                );
            }
        }

        // --------------------------------------------------------------------
        // Stage 2: Turn-level Compaction (Summarize / SlidingWindow / Auto)
        // --------------------------------------------------------------------
        let turn_groups = group_into_turns(&working_messages);
        let total_turns = turn_groups.len();

        if total_turns <= self.config.preserve_recent_turns + 1 {
            // Not enough turns to safely summarize/drop
            let final_tokens = estimate_messages_tokens(&working_messages);
            let final_count = working_messages.len();
            return (
                working_messages,
                CompactionResult {
                    compacted: original_tokens > final_tokens,
                    original_tokens,
                    compacted_tokens: final_tokens,
                    tokens_saved: original_tokens.saturating_sub(final_tokens),
                    original_messages: original_count,
                    compacted_messages: final_count,
                    messages_removed: original_count.saturating_sub(final_count),
                    summary: None,
                    strategy_used: self.config.strategy,
                },
            );
        }

        // Identify system messages at the start
        let mut system_messages = Vec::new();
        for msg in &working_messages {
            if msg.role == Role::System {
                system_messages.push(msg.clone());
            } else {
                break;
            }
        }

        // Determine how many older turns to compact
        let split_idx = total_turns.saturating_sub(self.config.preserve_recent_turns);
        let older_turn_groups = &turn_groups[..split_idx];
        let recent_turn_groups = &turn_groups[split_idx..];

        let mut older_messages = Vec::new();
        for group in older_turn_groups {
            for m in &group.messages {
                // Do not include initial system messages in summarization pool
                if m.role != Role::System {
                    older_messages.push(m.clone());
                }
            }
        }

        // Build compacted output
        let mut final_messages: Vec<Message> = Vec::new();

        // 1. Keep system prompt(s) at top
        final_messages.extend(system_messages);

        // 2. Optionally keep the first user message for initial goal grounding
        if self.config.preserve_initial_goal {
            if let Some(first_user) = older_messages.iter().find(|m| m.role == Role::User) {
                final_messages.push(first_user.clone());
            }
        }

        // 3. Insert Summary (if Summarize or Auto)
        let summary_text = if self.config.strategy != CompactionStrategy::SlidingWindow {
            let summary = generate_heuristic_summary(&older_messages);
            final_messages.push(Message::user(format!(
                "[Conversation Summary - Prior History]\n{}\n[End of Prior History Summary. Recent turns follow.]",
                summary
            )));
            final_messages.push(Message::assistant(
                "Understood. I have absorbed the context, prior actions, and findings from the conversation history summary. Continuing with your requests.",
            ));
            Some(summary)
        } else {
            None
        };

        // 4. Attach the preserved recent turns
        for group in recent_turn_groups {
            for m in &group.messages {
                final_messages.push(m.clone());
            }
        }

        let compacted_tokens = estimate_messages_tokens(&final_messages);
        let tokens_saved = original_tokens.saturating_sub(compacted_tokens);
        let compacted_count = final_messages.len();

        (
            final_messages,
            CompactionResult {
                compacted: true,
                original_tokens,
                compacted_tokens,
                tokens_saved,
                original_messages: original_count,
                compacted_messages: compacted_count,
                messages_removed: original_count.saturating_sub(compacted_count),
                summary: summary_text,
                strategy_used: self.config.strategy,
            },
        )
    }

    /// Performs compaction while preserving specified turn indices (e.g. pinned turns or bookmarks).
    pub fn compact_with_pinned(
        &self,
        messages: &[Message],
        model: &str,
        pinned_turn_indices: &[usize],
    ) -> (Vec<Message>, CompactionResult) {
        let turn_groups = group_into_turns(messages);
        let total_turns = turn_groups.len();

        if total_turns <= self.config.preserve_recent_turns + 1 {
            return self.compact(messages, model);
        }

        let pinned_set: HashSet<usize> = pinned_turn_indices.iter().copied().collect();
        let split_idx = total_turns.saturating_sub(self.config.preserve_recent_turns);

        let mut older_messages = Vec::new();
        let mut pinned_groups = Vec::new();

        for (idx, group) in turn_groups[..split_idx].iter().enumerate() {
            if pinned_set.contains(&idx) {
                pinned_groups.push(group.clone());
            } else {
                for m in &group.messages {
                    if m.role != Role::System {
                        older_messages.push(m.clone());
                    }
                }
            }
        }

        let mut final_messages: Vec<Message> = Vec::new();

        // 1. Keep system message
        if let Some(first) = messages.first() {
            if first.role == Role::System {
                final_messages.push(first.clone());
            }
        }

        // 2. Initial goal
        if self.config.preserve_initial_goal {
            if let Some(first_user) = older_messages.iter().find(|m| m.role == Role::User) {
                final_messages.push(first_user.clone());
            }
        }

        // 3. Pinned turns preserved in older history
        for pg in pinned_groups {
            final_messages.extend(pg.messages);
        }

        // 4. Summarize unpinned older turns
        let summary_text = if !older_messages.is_empty()
            && self.config.strategy != CompactionStrategy::SlidingWindow
        {
            let summary = generate_heuristic_summary(&older_messages);
            final_messages.push(Message::user(format!(
                "[Conversation Summary - Prior History]\n{}\n[End of Prior History Summary. Recent turns follow.]",
                summary
            )));
            final_messages.push(Message::assistant(
                "Understood. I have absorbed the context from the conversation summary. Continuing with your requests.",
            ));
            Some(summary)
        } else {
            None
        };

        // 5. Preserved recent turns
        for group in &turn_groups[split_idx..] {
            final_messages.extend(group.messages.clone());
        }

        let original_tokens = estimate_messages_tokens(messages);
        let compacted_tokens = estimate_messages_tokens(&final_messages);
        let original_count = messages.len();
        let compacted_count = final_messages.len();

        (
            final_messages,
            CompactionResult {
                compacted: true,
                original_tokens,
                compacted_tokens,
                tokens_saved: original_tokens.saturating_sub(compacted_tokens),
                original_messages: original_count,
                compacted_messages: compacted_count,
                messages_removed: original_count.saturating_sub(compacted_count),
                summary: summary_text,
                strategy_used: self.config.strategy,
            },
        )
    }

    /// Compacts a `Session` in-place, updating its messages and touching its timestamp.
    pub fn compact_session(&self, session: &mut Session) -> CompactionResult {
        let (new_messages, result) = self.compact(session.messages(), session.active_model());
        if result.compacted {
            *session.messages_mut() = new_messages;
        }
        result
    }

    /// Asynchronously compacts a `Session` using an LLM to generate a high-fidelity summary
    /// of older turns, with seamless fallback to heuristic summarization if the LLM call fails.
    pub async fn compact_session_with_llm(
        &self,
        session: &mut Session,
        client: &LlmClient,
        config: &Config,
    ) -> anyhow::Result<CompactionResult> {
        let original_tokens = estimate_messages_tokens(session.messages());
        let original_count = session.messages().len();
        let model = session.active_model();

        let limit = self.resolve_context_limit(model);
        let budget = (limit as f32 * self.config.threshold) as usize;

        if original_tokens < budget {
            return Ok(CompactionResult::uncompacted(
                original_tokens,
                original_count,
            ));
        }

        let turn_groups = group_into_turns(session.messages());
        let total_turns = turn_groups.len();

        if total_turns <= self.config.preserve_recent_turns + 1 {
            // Insufficient turns to summarize; fall back to synchronous compaction
            return Ok(self.compact_session(session));
        }

        let split_idx = total_turns.saturating_sub(self.config.preserve_recent_turns);
        let older_turn_groups = &turn_groups[..split_idx];
        let recent_turn_groups = &turn_groups[split_idx..];

        let mut older_messages = Vec::new();
        for group in older_turn_groups {
            for m in &group.messages {
                if m.role != Role::System {
                    older_messages.push(m.clone());
                }
            }
        }

        // Format prompt for LLM summarizer
        let mut transcript = String::new();
        for m in &older_messages {
            let role_name = match m.role {
                Role::User => "User",
                Role::Assistant => "Assistant",
                Role::Tool => "Tool Result",
                Role::System => "System",
            };
            let content_preview = if m.content.len() > 300 {
                format!("{}...", safe_char_head(&m.content, 297))
            } else {
                m.content.clone()
            };
            transcript.push_str(&format!("{}: {}\n", role_name, content_preview));
        }

        let summarizer_prompt = format!(
            "Summarize the following conversation history for an AI coding assistant. \
             Highlight: 1) The user's main objectives, 2) Files read, edited, or created, \
             3) Key technical decisions made, and 4) Current state of the work. \
             Keep the summary factual, concise, and under 300 words.\n\nTranscript:\n{}",
            transcript
        );

        let summary_req = vec![
            Message::system("You are an expert technical summarizer for a coding assistant. Be concise and precise."),
            Message::user(summarizer_prompt),
        ];

        // Attempt LLM summarization
        let llm_summary = match client.complete(config, &summary_req, &[]).await {
            Ok((content, _, _)) if !content.trim().is_empty() => content.trim().to_string(),
            _ => {
                // Fallback to heuristic summary if LLM call fails
                generate_heuristic_summary(&older_messages)
            }
        };

        // Construct compacted message list
        let mut final_messages: Vec<Message> = Vec::new();

        // Preserve system message if present
        if let Some(first) = session.messages().first() {
            if first.role == Role::System {
                final_messages.push(first.clone());
            }
        }

        // Preserve initial user goal if enabled
        if self.config.preserve_initial_goal {
            if let Some(first_user) = older_messages.iter().find(|m| m.role == Role::User) {
                final_messages.push(first_user.clone());
            }
        }

        // Inject LLM summary
        final_messages.push(Message::user(format!(
            "[Conversation Summary - Prior History]\n{}\n[End of Summary. Recent conversation follows.]",
            llm_summary
        )));
        final_messages.push(Message::assistant(
            "I have absorbed the context from the conversation summary. Continuing with your requests.",
        ));

        // Append recent intact turns
        for group in recent_turn_groups {
            for m in &group.messages {
                final_messages.push(m.clone());
            }
        }

        let compacted_tokens = estimate_messages_tokens(&final_messages);
        let tokens_saved = original_tokens.saturating_sub(compacted_tokens);
        let compacted_count = final_messages.len();

        *session.messages_mut() = final_messages;

        Ok(CompactionResult {
            compacted: true,
            original_tokens,
            compacted_tokens,
            tokens_saved,
            original_messages: original_count,
            compacted_messages: compacted_count,
            messages_removed: original_count.saturating_sub(compacted_count),
            summary: Some(llm_summary),
            strategy_used: CompactionStrategy::Summarize,
        })
    }
}

// ============================================================================
// Convenience Free Functions
// ============================================================================

/// Compacts a session using default configuration or the provided compactor.
pub fn compact_session(session: &mut Session, compactor: Option<&Compactor>) -> CompactionResult {
    if let Some(c) = compactor {
        c.compact_session(session)
    } else {
        Compactor::default().compact_session(session)
    }
}

/// Asynchronously compacts a session using LLM summarization with heuristic fallback.
pub async fn compact_session_with_llm(
    session: &mut Session,
    client: &LlmClient,
    config: &Config,
) -> anyhow::Result<CompactionResult> {
    Compactor::default()
        .compact_session_with_llm(session, client, config)
        .await
}

// ============================================================================
// Comprehensive Unit Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::provider::types::{Message, Role, ToolCall};

    #[test]
    fn test_estimate_text_tokens_basic() {
        assert_eq!(estimate_text_tokens(""), 0);
        let tokens = estimate_text_tokens("Hello world");
        assert!(tokens >= 2 && tokens <= 4);

        let code_tokens = estimate_text_tokens("fn main() { println!(\"Hello, world!\"); }");
        assert!(code_tokens > 5 && code_tokens < 25);
    }

    #[test]
    fn test_model_context_limit_known() {
        assert_eq!(model_context_limit("gpt-4o"), 128_000);
        assert_eq!(model_context_limit("claude-3-7-sonnet"), 200_000);
        assert!(model_context_limit("gemini-1.5-pro") >= 1_000_000);
        assert_eq!(model_context_limit("deepseek-r1"), 128_000);
        assert_eq!(model_context_limit("llama-3-8b"), 8_192);
        assert_eq!(model_context_limit("unknown-model-xyz"), 128_000);
    }

    #[test]
    fn test_is_context_overflow() {
        assert!(!is_context_overflow(100_000, "gpt-4o", 0.80));
        assert!(is_context_overflow(102_400, "gpt-4o", 0.80));
        assert!(is_context_overflow(120_000, "gpt-4o", 0.80));
    }

    #[test]
    fn test_budget_status_and_utilization() {
        let compactor = Compactor::new(1000).with_threshold(0.80);

        let msg_small = vec![Message::user("Hello")];
        assert_eq!(
            compactor.budget_status(&msg_small, "test"),
            ContextBudgetStatus::Healthy
        );
        assert!(compactor.context_utilization(&msg_small, "test") < 0.10);

        // Fill up to ~70%
        let mut msg_med = Vec::new();
        for _ in 0..35 {
            msg_med.push(Message::user("Medium turn request with some tokens"));
            msg_med.push(Message::assistant("Medium turn response with tokens"));
        }
        let status = compactor.budget_status(&msg_med, "test");
        assert!(matches!(
            status,
            ContextBudgetStatus::ApproachingThreshold | ContextBudgetStatus::CompactionRequired
        ));

        // Plan calculation
        let plan = compactor.calculate_compaction_plan(&msg_med, "test");
        assert_eq!(plan.limit_tokens, 1000);
        assert_eq!(plan.trigger_threshold, 0.80);
        assert_eq!(plan.target_tokens, 500);
    }

    #[test]
    fn test_truncate_tool_output_multiline() {
        let short = "Short output\nunder limit";
        assert_eq!(truncate_tool_output(short, 100), short);

        let long_output = (0..100)
            .map(|i| format!("Line {}: some log output with details and tokens here", i))
            .collect::<Vec<_>>()
            .join("\n");

        let truncated = truncate_tool_output(&long_output, 50);
        assert!(truncated.contains("lines elided for context compaction"));
        assert!(estimate_text_tokens(&truncated) < estimate_text_tokens(&long_output));
    }

    #[test]
    fn test_truncate_tool_output_utf8_safety() {
        // Multi-byte unicode characters & emojis
        let emoji_blob =
            "🚀 Rust Crab 🦀 — Special Unicode Characters: こんにちは世界 / 💻 🔥 ".repeat(50);
        let truncated = truncate_tool_output(&emoji_blob, 20);
        assert!(truncated.contains("elided for context compaction"));
        assert!(estimate_text_tokens(&truncated) < estimate_text_tokens(&emoji_blob));

        // Single line very long UTF-8 string
        let long_single_line = "🔥".repeat(1000);
        let truncated_single = truncate_tool_output(&long_single_line, 20);
        assert!(truncated_single.contains("elided for context compaction"));
    }

    #[test]
    fn test_group_into_turns_preserves_tool_pairs() {
        let messages = vec![
            Message::system("System prompt initialized"),
            Message::user("Please read main.rs"),
            Message::assistant_with_tools(
                "",
                vec![ToolCall {
                    id: "call_1".into(),
                    name: "read".into(),
                    arguments: "{\"path\":\"src/main.rs\"}".into(),
                }],
            ),
            Message::tool_result("call_1", "fn main() {}"),
            Message::assistant("I finished reading the file."),
            Message::user("Now edit it"),
        ];

        let turns = group_into_turns(&messages);
        assert_eq!(turns.len(), 5);
        assert_eq!(turns[0].turn_type(), TurnType::System);
        assert_eq!(turns[1].turn_type(), TurnType::User);
        assert_eq!(turns[2].turn_type(), TurnType::ToolExecution);
        assert!(turns[2].is_tool_call);
        assert_eq!(turns[2].messages.len(), 2); // Tool call assistant + Tool response
        assert_eq!(turns[2].extracted_file_paths(), vec!["src/main.rs"]);
        assert_eq!(turns[3].turn_type(), TurnType::Assistant);
        assert_eq!(turns[4].turn_type(), TurnType::User);
    }

    #[test]
    fn test_turn_group_error_detection() {
        let fail_turn = TurnGroup {
            messages: vec![
                Message::assistant_with_tools(
                    "Running build",
                    vec![ToolCall {
                        id: "tc_fail".into(),
                        name: "bash".into(),
                        arguments: "{\"command\":\"cargo build\"}".into(),
                    }],
                ),
                Message::tool_result(
                    "tc_fail",
                    "error: could not compile `fusion` due to 1 previous error",
                ),
            ],
            is_tool_call: true,
            estimated_tokens: 30,
        };

        assert!(fail_turn.has_error_or_failure());
        assert_eq!(fail_turn.turn_type(), TurnType::ToolExecution);
    }

    #[test]
    fn test_compaction_below_threshold_no_op() {
        let messages = vec![Message::user("Hello"), Message::assistant("Hi there!")];
        let compactor = Compactor::new(100_000).with_threshold(0.80);
        let (compacted_msgs, res) = compactor.compact(&messages, "gpt-4o");

        assert!(!res.compacted);
        assert_eq!(compacted_msgs.len(), 2);
        assert_eq!(res.tokens_saved, 0);
    }

    #[test]
    fn test_compaction_exceeding_threshold() {
        let mut messages = Vec::new();
        messages.push(Message::system("System prompt instructions"));
        messages.push(Message::user("Initial goal: build a web server"));
        messages.push(Message::assistant("Starting now"));

        for i in 1..=10 {
            messages.push(Message::user(format!("Step {}: do something important", i)));
            messages.push(Message::assistant_with_tools(
                format!("Executing step {}", i),
                vec![ToolCall {
                    id: format!("call_{}", i),
                    name: "bash".into(),
                    arguments: "{\"command\":\"cargo build\"}".into(),
                }],
            ));
            messages.push(Message::tool_result(
                format!("call_{}", i),
                format!("Output for command {}: Finished dev [unoptimized + debuginfo] target(s) in 0.42s", i),
            ));
            messages.push(Message::assistant(format!("Step {} is complete.", i)));
        }

        // Limit of 300 tokens: this conversation will definitely exceed 80% (240 tokens)
        let compactor = Compactor::new(300)
            .with_threshold(0.80)
            .with_preserve_turns(2);

        let (compacted_msgs, res) = compactor.compact(&messages, "test-model");

        assert!(res.compacted);
        assert!(res.compacted_tokens < res.original_tokens);
        assert!(res.tokens_saved > 0);
        assert!(compacted_msgs.len() < messages.len());

        // System prompt preserved at start
        assert_eq!(compacted_msgs[0].role, Role::System);
        // Initial goal preserved
        assert!(compacted_msgs
            .iter()
            .any(|m| m.content.contains("Initial goal")));
        // Summary should be generated
        assert!(res.summary.is_some());
    }

    #[test]
    fn test_compact_session_integration() {
        let mut session = Session::new("gpt-4o");
        session.add_user_message("First user message");
        session.add_assistant_message("Assistant reply 1");

        for i in 2..=8 {
            session.add_user_message(format!("Turn {}", i));
            session.add_assistant_message(format!("Response {}", i));
        }

        let compactor = Compactor::new(100)
            .with_threshold(0.80)
            .with_preserve_turns(2);
        let res = compactor.compact_session(&mut session);

        assert!(res.compacted);
        assert!(session.messages().len() < 16);
    }

    #[test]
    fn test_prune_older_tool_outputs_strategy() {
        let mut messages = vec![
            Message::user("Inspect large file"),
            Message::assistant_with_tools(
                "Reading...",
                vec![ToolCall {
                    id: "c1".into(),
                    name: "read".into(),
                    arguments: "{\"path\":\"big.txt\"}".into(),
                }],
            ),
            Message::tool_result(
                "c1",
                (0..80)
                    .map(|i| format!("Line {} content for testing tool pruning", i))
                    .collect::<Vec<_>>()
                    .join("\n"),
            ),
            Message::assistant("Done reading."),
            Message::user("Recent turn 1"),
            Message::assistant("Recent reply 1"),
            Message::user("Recent turn 2"),
            Message::assistant("Recent reply 2"),
        ];

        let initial_tokens = estimate_messages_tokens(&messages);
        let compactor = Compactor::new(initial_tokens / 2)
            .with_strategy(CompactionStrategy::PruneToolsOnly)
            .with_preserve_turns(2);

        let (compacted_msgs, res) = compactor.compact(&messages, "test-model");
        assert!(res.compacted);
        assert!(res.compacted_tokens < initial_tokens);
        assert_eq!(compacted_msgs.len(), messages.len()); // PruneToolsOnly keeps message count
        assert!(compacted_msgs[2]
            .content
            .contains("lines elided for context compaction"));
    }

    #[test]
    fn test_sliding_window_strategy() {
        let mut messages = Vec::new();
        messages.push(Message::system("System instructions"));
        for i in 1..=10 {
            messages.push(Message::user(format!("Question {}", i)));
            messages.push(Message::assistant(format!("Answer {}", i)));
        }

        let compactor = Compactor::new(100)
            .with_strategy(CompactionStrategy::SlidingWindow)
            .with_preserve_turns(3);

        let (compacted_msgs, res) = compactor.compact(&messages, "test-model");
        assert!(res.compacted);
        // System prompt (1) + Initial User (1) + Preserved recent turns (6)
        assert!(compacted_msgs.len() <= 8);
        assert_eq!(compacted_msgs[0].role, Role::System);
        assert_eq!(res.strategy_used, CompactionStrategy::SlidingWindow);
    }

    #[test]
    fn test_compact_with_pinned_turns() {
        let mut messages = Vec::new();
        messages.push(Message::system("System prompt"));
        messages.push(Message::user("Goal: build compiler")); // turn 1
        messages.push(Message::assistant("Starting compiler"));

        for i in 2..=8 {
            messages.push(Message::user(format!("Query {}", i)));
            messages.push(Message::assistant(format!("Answer {}", i)));
        }

        let compactor = Compactor::new(80).with_preserve_turns(2);
        // Pin turn 1 (initial goal) and turn 3
        let (compacted_msgs, res) = compactor.compact_with_pinned(&messages, "test-model", &[1, 3]);

        assert!(res.compacted);
        assert!(compacted_msgs.iter().any(|m| m.content.contains("Query 3")));
    }

    #[test]
    fn test_heuristic_summary_generation() {
        let older_msgs = vec![
            Message::user("Please build a REST API in Rust"),
            Message::assistant_with_tools(
                "Checking files",
                vec![ToolCall {
                    id: "tc1".into(),
                    name: "read".into(),
                    arguments: "{\"path\":\"src/server.rs\"}".into(),
                }],
            ),
            Message::tool_result("tc1", "fn serve() {}"),
            Message::assistant("I reviewed the server module."),
            Message::assistant_with_tools(
                "Compiling",
                vec![ToolCall {
                    id: "tc2".into(),
                    name: "bash".into(),
                    arguments: "{\"command\":\"cargo check\"}".into(),
                }],
            ),
            Message::tool_result("tc2", "error: could not compile due to unresolved import"),
        ];

        let summary = generate_heuristic_summary(&older_msgs);
        assert!(summary.contains("REST API in Rust"));
        assert!(summary.contains("src/server.rs"));
        assert!(summary.contains("Tool `read`"));
        assert!(summary.contains("Critical Notices & Tool Failures"));
    }

    #[test]
    fn test_compaction_result_formatting() {
        let uncompacted = CompactionResult::uncompacted(1500, 5);
        assert!(!uncompacted.compacted);
        assert!(uncompacted
            .format_summary()
            .contains("No compaction needed"));

        let mut session = Session::new("gpt-4o");
        assert_eq!(session.estimate_tokens(), 0);
        session.add_user_message("Hello world");
        assert!(session.estimate_tokens() > 0);
        assert!(!session.needs_compaction(None));
    }
}
