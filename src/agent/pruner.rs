//! Context-saving conversation pruner for Fusion.
//!
//! Optimizes context window token usage by:
//! 1. Stripping or collapsing redundant intermediate thinking blocks (`<think>`, `<thought>`,
//!    `<reasoning>`, etc.) from older assistant turns.
//! 2. Truncating verbose tool results (file dumps, command logs, compiler output, search matches)
//!    while preserving head/tail context, diff headers, and error messages.
//! 3. Deduplicating identical repeated tool outputs across turns.
//! 4. Progressive budget-aware multi-stage pruning to fit within context limits.
use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::agent::session::Session;
use crate::agent::tokens::{
    estimate_message_tokens, estimate_messages_tokens, estimate_text_tokens,
};
use crate::provider::types::{Message, Role, ToolCall};

// ============================================================================
// Thinking Block Policies & Parsing
// ============================================================================

/// Policy governing how thinking / reasoning blocks (`<think>`, `<thought>`, etc.) are pruned.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ThinkingPrunePolicy {
    /// Completely strip thinking blocks from all assistant messages except the most recent N turns.
    KeepRecentTurns(usize),
    /// Completely strip thinking blocks from all assistant messages without exception.
    StripAll,
    /// Collapse thinking blocks into a concise placeholder notice (e.g. `[thinking elided: ~120 tokens]`).
    CollapseToPlaceholder {
        /// Custom placeholder template. If empty or None, a default notice is used.
        #[serde(default)]
        placeholder: Option<String>,
        /// Number of recent assistant turns to keep uncollapsed.
        #[serde(default = "default_recent_turns")]
        preserve_recent_turns: usize,
    },
    /// Keep all thinking blocks intact (no stripping).
    KeepAll,
}

fn default_recent_turns() -> usize {
    1
}

impl Default for ThinkingPrunePolicy {
    fn default() -> Self {
        // By default, keep thinking only in the most recent assistant response (1 turn),
        // stripping intermediate thinking from older turns.
        Self::KeepRecentTurns(1)
    }
}

// ============================================================================
// Tool Output Pruning Policies
// ============================================================================

/// Policy governing how tool results are truncated and compressed.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum ToolPrunePolicy {
    /// Smart truncation: adapts strategy based on tool type (bash, file read, grep, diff, json)
    /// and preserves critical error diagnostics.
    Smart {
        /// Maximum allowed tokens per older tool output before truncation.
        max_tokens: usize,
        /// Number of recent turns whose tool outputs remain uncompressed.
        preserve_recent_turns: usize,
        /// When true, error messages and failure indicators are always preserved.
        preserve_errors: bool,
    },
    /// Head-and-tail line truncation: keeps first N lines and last M lines.
    HeadTail {
        head_lines: usize,
        tail_lines: usize,
        max_tokens: usize,
        preserve_recent_turns: usize,
    },
    /// Simple token cap truncation: truncates outputs exceeding `max_tokens`.
    MaxTokens {
        max_tokens: usize,
        preserve_recent_turns: usize,
    },
    /// Keep all tool outputs intact (no truncation).
    KeepAll,
}

impl Default for ToolPrunePolicy {
    fn default() -> Self {
        Self::Smart {
            max_tokens: 300,
            preserve_recent_turns: 2,
            preserve_errors: true,
        }
    }
}

// ============================================================================
// Pruner Configuration
// ============================================================================

/// Comprehensive configuration for conversation pruning.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PrunerConfig {
    /// Policy for stripping or collapsing reasoning/thinking blocks.
    pub thinking_policy: ThinkingPrunePolicy,
    /// Policy for truncating and compressing tool outputs.
    pub tool_policy: ToolPrunePolicy,
    /// Number of recent conversational turns to preserve untouched.
    pub preserve_recent_turns: usize,
    /// Whether to preserve the initial user prompt/goal at the start of conversation.
    pub preserve_initial_goal: bool,
    /// Whether to preserve system prompt messages intact.
    pub preserve_system_messages: bool,
    /// Whether to deduplicate identical tool outputs across turns.
    pub deduplicate_tools: bool,
    /// Whether to drop empty assistant messages that resulted from stripping thinking blocks.
    pub drop_empty_assistant_messages: bool,
    /// Optional target token budget. If set, pruner applies progressive tiered pruning
    /// to bring total estimated tokens below this limit.
    pub target_token_budget: Option<usize>,
    /// Custom thinking block tag pairs to recognize in addition to standard ones.
    /// Each pair is `(open_tag, close_tag)` (e.g. `("<reasoning>", "</reasoning>")`).
    pub custom_thinking_tags: Vec<(String, String)>,
}

impl Default for PrunerConfig {
    fn default() -> Self {
        Self {
            thinking_policy: ThinkingPrunePolicy::default(),
            tool_policy: ToolPrunePolicy::default(),
            preserve_recent_turns: 2,
            preserve_initial_goal: true,
            preserve_system_messages: true,
            deduplicate_tools: true,
            drop_empty_assistant_messages: false,
            target_token_budget: None,
            custom_thinking_tags: Vec::new(),
        }
    }
}

impl PrunerConfig {
    /// Creates a new default pruning configuration.
    pub fn new() -> Self {
        Self::default()
    }

    /// Creates an aggressive pruning configuration for tight context windows.
    pub fn aggressive() -> Self {
        Self {
            thinking_policy: ThinkingPrunePolicy::StripAll,
            tool_policy: ToolPrunePolicy::Smart {
                max_tokens: 150,
                preserve_recent_turns: 1,
                preserve_errors: true,
            },
            preserve_recent_turns: 1,
            preserve_initial_goal: true,
            preserve_system_messages: true,
            deduplicate_tools: true,
            drop_empty_assistant_messages: true,
            target_token_budget: None,
            custom_thinking_tags: Vec::new(),
        }
    }

    /// Creates a conservative pruning configuration that only trims very large tool outputs
    /// and preserves more recent context.
    pub fn conservative() -> Self {
        Self {
            thinking_policy: ThinkingPrunePolicy::KeepRecentTurns(3),
            tool_policy: ToolPrunePolicy::Smart {
                max_tokens: 600,
                preserve_recent_turns: 4,
                preserve_errors: true,
            },
            preserve_recent_turns: 4,
            preserve_initial_goal: true,
            preserve_system_messages: true,
            deduplicate_tools: true,
            drop_empty_assistant_messages: false,
            target_token_budget: None,
            custom_thinking_tags: Vec::new(),
        }
    }

    /// Sets the thinking block pruning policy.
    pub fn with_thinking_policy(mut self, policy: ThinkingPrunePolicy) -> Self {
        self.thinking_policy = policy;
        self
    }

    /// Sets the tool output pruning policy.
    pub fn with_tool_policy(mut self, policy: ToolPrunePolicy) -> Self {
        self.tool_policy = policy;
        self
    }

    /// Sets the number of recent turns preserved untouched.
    pub fn with_preserve_recent_turns(mut self, turns: usize) -> Self {
        self.preserve_recent_turns = turns;
        self
    }

    /// Sets whether to preserve the initial user message.
    pub fn with_preserve_initial_goal(mut self, preserve: bool) -> Self {
        self.preserve_initial_goal = preserve;
        self
    }

    /// Sets whether to deduplicate identical tool outputs.
    pub fn with_deduplicate_tools(mut self, deduplicate: bool) -> Self {
        self.deduplicate_tools = deduplicate;
        self
    }

    /// Sets a target token budget for progressive pruning.
    pub fn with_target_token_budget(mut self, budget: usize) -> Self {
        self.target_token_budget = Some(budget);
        self
    }

    /// Adds a custom pair of thinking tags to recognize.
    pub fn with_custom_thinking_tag(
        mut self,
        open: impl Into<String>,
        close: impl Into<String>,
    ) -> Self {
        self.custom_thinking_tags.push((open.into(), close.into()));
        self
    }
}

// ============================================================================
// Pruning Audit Actions & Results
// ============================================================================

/// Types of pruning actions recorded during conversation pruning.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum PruneActionType {
    /// Stripped reasoning / thinking block from an assistant message.
    StripThinking,
    /// Collapsed thinking block into a placeholder notice.
    CollapseThinking,
    /// Truncated verbose tool result output.
    TruncateToolOutput,
    /// Deduplicated identical tool output with a reference pointer.
    DeduplicateToolOutput,
    /// Dropped empty assistant message.
    DropEmptyMessage,
}

/// Audit record for a specific pruning mutation on a message.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PruneAction {
    /// Index of the message in the conversation.
    pub message_index: usize,
    /// Role of the message.
    pub role: Role,
    /// Type of action taken.
    pub action_type: PruneActionType,
    /// Estimated tokens before this action.
    pub tokens_before: usize,
    /// Estimated tokens after this action.
    pub tokens_after: usize,
    /// Human-readable description of the pruning change.
    pub description: String,
}

impl PruneAction {
    /// Number of tokens saved by this specific action.
    pub fn tokens_saved(&self) -> usize {
        self.tokens_before.saturating_sub(self.tokens_after)
    }
}

/// Comprehensive outcome of a conversation pruning operation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PruneResult {
    /// The pruned messages.
    pub messages: Vec<Message>,
    /// Total estimated tokens before pruning.
    pub original_tokens: usize,
    /// Total estimated tokens after pruning.
    pub pruned_tokens: usize,
    /// Number of tokens saved.
    pub tokens_saved: usize,
    /// Compression ratio (`pruned_tokens as f32 / original_tokens as f32`).
    pub compression_ratio: f32,
    /// Total thinking blocks stripped or collapsed.
    pub thinking_blocks_pruned: usize,
    /// Total tokens saved specifically from thinking block removal.
    pub thinking_tokens_saved: usize,
    /// Total tool result messages truncated.
    pub tool_outputs_truncated: usize,
    /// Total tokens saved specifically from tool output truncation.
    pub tool_tokens_saved: usize,
    /// Total duplicate tool outputs collapsed.
    pub duplicate_tools_collapsed: usize,
    /// Total empty messages dropped.
    pub empty_messages_dropped: usize,
    /// Chronological log of all pruning actions performed.
    pub actions: Vec<PruneAction>,
}

impl PruneResult {
    /// Returns true if any pruning mutations were applied.
    pub fn is_pruned(&self) -> bool {
        self.tokens_saved > 0 || self.empty_messages_dropped > 0
    }

    /// Returns the percentage of tokens saved (0.0 to 100.0).
    pub fn savings_percentage(&self) -> f32 {
        if self.original_tokens == 0 {
            0.0
        } else {
            (self.tokens_saved as f32 / self.original_tokens as f32) * 100.0
        }
    }

    /// Generates a human-readable summary of the pruning results.
    pub fn summary(&self) -> String {
        format!(
            "Pruned conversation: {} -> {} tokens (saved {} tokens, {:.1}%), pruned {} thinking blocks ({} tokens), truncated {} tool outputs ({} tokens), collapsed {} duplicates",
            self.original_tokens,
            self.pruned_tokens,
            self.tokens_saved,
            self.savings_percentage(),
            self.thinking_blocks_pruned,
            self.thinking_tokens_saved,
            self.tool_outputs_truncated,
            self.tool_tokens_saved,
            self.duplicate_tools_collapsed,
        )
    }
}

// ============================================================================
// Turn Grouping Representation
// ============================================================================

/// Represents a logical turn in a conversational exchange.
#[derive(Debug, Clone)]
pub struct PruneTurn {
    /// Turn index (0-based).
    pub turn_index: usize,
    /// Indices of messages in the flat message list that belong to this turn.
    pub message_indices: Vec<usize>,
    /// Whether this turn contains tool calls or tool results.
    pub has_tools: bool,
    /// Number of assistant messages in this turn.
    pub assistant_count: usize,
}

/// Groups flat messages into logical turns (user prompt -> assistant response / tool cycles).
pub fn group_messages_into_turns(messages: &[Message]) -> Vec<PruneTurn> {
    let mut turns: Vec<PruneTurn> = Vec::new();
    let mut current_indices: Vec<usize> = Vec::new();
    let mut current_has_tools = false;
    let mut current_assistant_count = 0;

    for (idx, msg) in messages.iter().enumerate() {
        if msg.role == Role::System {
            // System prompt belongs to its own special turn (or turn 0)
            if !current_indices.is_empty() {
                turns.push(PruneTurn {
                    turn_index: turns.len(),
                    message_indices: std::mem::take(&mut current_indices),
                    has_tools: current_has_tools,
                    assistant_count: current_assistant_count,
                });
                current_has_tools = false;
                current_assistant_count = 0;
            }
            turns.push(PruneTurn {
                turn_index: turns.len(),
                message_indices: vec![idx],
                has_tools: false,
                assistant_count: 0,
            });
            continue;
        }

        if msg.role == Role::User {
            // A new user message starts a new conversational turn
            if !current_indices.is_empty() {
                turns.push(PruneTurn {
                    turn_index: turns.len(),
                    message_indices: std::mem::take(&mut current_indices),
                    has_tools: current_has_tools,
                    assistant_count: current_assistant_count,
                });
                current_has_tools = false;
                current_assistant_count = 0;
            }
            current_indices.push(idx);
            continue;
        }

        if msg.role == Role::Assistant {
            current_assistant_count += 1;
            if msg.tool_calls.as_ref().map_or(false, |tc| !tc.is_empty()) {
                current_has_tools = true;
            }
            current_indices.push(idx);
            continue;
        }

        if msg.role == Role::Tool {
            current_has_tools = true;
            current_indices.push(idx);
            continue;
        }

        current_indices.push(idx);
    }

    if !current_indices.is_empty() {
        turns.push(PruneTurn {
            turn_index: turns.len(),
            message_indices: current_indices,
            has_tools: current_has_tools,
            assistant_count: current_assistant_count,
        });
    }

    turns
}

// ============================================================================
// Thinking Block Extraction & Stripping Engine
// ============================================================================

/// Default recognized tag pairs for reasoning/thinking blocks.
pub const STANDARD_THINKING_TAGS: &[(&str, &str)] = &[
    ("<think>", "</think>"),
    ("<thought>", "</thought>"),
    ("<thinking>", "</thinking>"),
    ("<reasoning>", "</reasoning>"),
    ("```thought\n", "\n```"),
    ("```think\n", "\n```"),
    ("[THINKING]", "[/THINKING]"),
];

/// Extracts all thinking blocks from a string, returning a list of extracted thoughts
/// and the remaining content with thinking blocks removed.
pub fn extract_all_thinking_blocks(content: &str) -> (Vec<String>, String) {
    extract_thinking_blocks_with_custom_tags(content, &[])
}

/// Extracts thinking blocks using both standard tags and custom user-provided tag pairs.
pub fn extract_thinking_blocks_with_custom_tags(
    content: &str,
    custom_tags: &[(String, String)],
) -> (Vec<String>, String) {
    if content.is_empty() {
        return (Vec::new(), String::new());
    }

    let mut all_tags: Vec<(&str, &str)> = STANDARD_THINKING_TAGS.to_vec();
    for (o, c) in custom_tags {
        all_tags.push((o.as_str(), c.as_str()));
    }

    let mut thoughts: Vec<String> = Vec::new();
    let mut result = String::with_capacity(content.len());
    let mut cursor = 0;

    while cursor < content.len() {
        let remaining = &content[cursor..];

        // Find the earliest matching open tag
        let mut earliest_match: Option<(usize, usize, &str, &str)> = None;

        for &(open_tag, close_tag) in &all_tags {
            // Case-insensitive search for open tag
            if let Some(open_rel_idx) = find_case_insensitive(remaining, open_tag) {
                let open_abs_idx = cursor + open_rel_idx;
                match earliest_match {
                    None => {
                        earliest_match = Some((open_abs_idx, open_tag.len(), open_tag, close_tag));
                    }
                    Some((earliest_pos, _, _, _)) if open_abs_idx < earliest_pos => {
                        earliest_match = Some((open_abs_idx, open_tag.len(), open_tag, close_tag));
                    }
                    _ => {}
                }
            }
        }

        if let Some((open_pos, open_len, _open_tag, close_tag)) = earliest_match {
            // Append content prior to the open tag
            result.push_str(&content[cursor..open_pos]);

            let after_open = &content[open_pos + open_len..];
            if let Some(close_rel_idx) = find_case_insensitive(after_open, close_tag) {
                let thought_text = &after_open[..close_rel_idx];
                thoughts.push(thought_text.trim().to_string());
                cursor = open_pos + open_len + close_rel_idx + close_tag.len();
            } else {
                // Unclosed thinking tag: treat the rest of the text as thinking block
                let thought_text = after_open.trim();
                if !thought_text.is_empty() {
                    thoughts.push(thought_text.to_string());
                }
                cursor = content.len();
            }
        } else {
            // No more thinking tags
            result.push_str(remaining);
            break;
        }
    }

    let cleaned = clean_excess_whitespace(&result);
    (thoughts, cleaned)
}

/// Case-insensitive substring search helper.
fn find_case_insensitive(haystack: &str, needle: &str) -> Option<usize> {
    if needle.is_empty() {
        return Some(0);
    }
    if haystack.len() < needle.len() {
        return None;
    }

    let needle_lower = needle.to_lowercase();
    let haystack_lower = haystack.to_lowercase();
    haystack_lower.find(&needle_lower)
}

/// Strips reasoning / thinking blocks from text, replacing them optionally with a placeholder.
/// Returns `(cleaned_text, blocks_stripped, estimated_tokens_saved)`.
pub fn strip_thinking_with_stats(
    content: &str,
    placeholder: Option<&str>,
    custom_tags: &[(String, String)],
) -> (String, usize, usize) {
    let (thoughts, remaining) = extract_thinking_blocks_with_custom_tags(content, custom_tags);
    if thoughts.is_empty() {
        return (content.to_string(), 0, 0);
    }

    let initial_tokens = estimate_text_tokens(content);
    let blocks_count = thoughts.len();

    let final_text = if let Some(ph) = placeholder {
        if remaining.is_empty() {
            ph.to_string()
        } else {
            format!("{}\n\n{}", ph, remaining)
        }
    } else {
        remaining
    };

    let final_tokens = estimate_text_tokens(&final_text);
    let tokens_saved = initial_tokens.saturating_sub(final_tokens);

    (final_text, blocks_count, tokens_saved)
}

/// Quick helper to strip all thinking tags and return just the cleaned text.
pub fn strip_thinking_tags(content: &str) -> String {
    let (_, cleaned) = extract_all_thinking_blocks(content);
    cleaned
}

/// Checks whether a text contains any recognized thinking / reasoning blocks.
pub fn has_thinking_blocks(content: &str) -> bool {
    for &(open_tag, _) in STANDARD_THINKING_TAGS {
        if find_case_insensitive(content, open_tag).is_some() {
            return true;
        }
    }
    false
}

/// Cleans excessive newlines and trailing/leading whitespace left behind after removing blocks.
fn clean_excess_whitespace(s: &str) -> String {
    let trimmed = s.trim();
    if trimmed.is_empty() {
        return String::new();
    }

    let mut result = String::with_capacity(trimmed.len());
    let mut newline_count = 0;

    for ch in trimmed.chars() {
        if ch == '\n' {
            newline_count += 1;
            if newline_count <= 2 {
                result.push(ch);
            }
        } else {
            newline_count = 0;
            result.push(ch);
        }
    }

    result
}

// ============================================================================
// Tool Output Truncation & Optimization Engine
// ============================================================================

/// Tests whether tool output appears to be an error, panic, or failure report.
pub fn is_error_tool_output(content: &str) -> bool {
    let lower = content.to_lowercase();
    lower.contains("error:")
        || lower.contains("error[e")
        || lower.contains("fatal:")
        || lower.contains("failed:")
        || lower.contains("panic:")
        || lower.contains("exception:")
        || lower.contains("traceback (most recent call last)")
        || lower.contains("exit status: 1")
        || lower.contains("exit status: 2")
        || lower.contains("command failed")
        || lower.contains("compilation error")
        || lower.contains("syntaxerror")
}

/// Minifies or compacts JSON output if it represents valid JSON and reduces token count.
pub fn minify_json_tool_output(content: &str) -> Option<String> {
    let trimmed = content.trim();
    if (trimmed.starts_with('{') && trimmed.ends_with('}'))
        || (trimmed.starts_with('[') && trimmed.ends_with(']'))
    {
        if let Ok(val) = serde_json::from_str::<serde_json::Value>(trimmed) {
            if let Ok(minified) = serde_json::to_string(&val) {
                if minified.len() < trimmed.len() {
                    return Some(minified);
                }
            }
        }
    }
    None
}

/// Truncates a git diff, preserving diff headers and eliding middle hunk lines.
pub fn truncate_git_diff_output(content: &str, max_tokens: usize) -> String {
    let lines: Vec<&str> = content.lines().collect();
    if lines.len() <= 20 {
        return content.to_string();
    }

    let mut header_lines: Vec<&str> = Vec::new();
    let mut hunk_lines: Vec<&str> = Vec::new();

    for line in &lines {
        if line.starts_with("diff --git")
            || line.starts_with("index ")
            || line.starts_with("--- ")
            || line.starts_with("+++ ")
            || line.starts_with("@@ ")
        {
            header_lines.push(line);
        } else {
            hunk_lines.push(line);
        }
    }

    let keep_hunks = max_tokens / 10;
    if hunk_lines.len() <= keep_hunks {
        return content.to_string();
    }

    let head_count = keep_hunks / 2;
    let tail_count = keep_hunks / 2;
    let elided = hunk_lines.len().saturating_sub(head_count + tail_count);

    let mut out = String::new();
    for hl in &header_lines {
        out.push_str(hl);
        out.push('\n');
    }
    out.push_str("\n... [git diff hunks truncated: ");
    out.push_str(&elided.to_string());
    out.push_str(" lines elided for context window] ...\n\n");

    if tail_count > 0 && hunk_lines.len() >= tail_count {
        for l in &hunk_lines[hunk_lines.len() - tail_count..] {
            out.push_str(l);
            out.push('\n');
        }
    }

    out
}

/// Smartly truncates tool result content based on tool type, content heuristics,
/// and error preservation rules.
pub fn truncate_tool_output_smart(
    content: &str,
    tool_name: Option<&str>,
    max_tokens: usize,
    preserve_errors: bool,
) -> String {
    let current_tokens = estimate_text_tokens(content);
    if current_tokens <= max_tokens {
        return content.to_string();
    }

    // 1. Check if error preservation applies
    if preserve_errors && is_error_tool_output(content) {
        // Keep error lines intact, truncate peripheral normal lines
        let lines: Vec<&str> = content.lines().collect();
        if lines.len() > 30 {
            // Extract error lines and last few lines
            let mut error_sections: Vec<&str> = Vec::new();
            for line in &lines {
                let l = line.to_lowercase();
                if l.contains("error")
                    || l.contains("fatal")
                    || l.contains("panic")
                    || l.contains("failed")
                    || l.contains("exception")
                {
                    error_sections.push(line);
                }
            }

            let tail_lines = lines.len().saturating_sub(10);
            let tail = &lines[tail_lines..];

            let mut out = String::new();
            out.push_str("... [Initial output elided; preserved error diagnostics] ...\n");
            for err in error_sections.iter().take(15) {
                out.push_str(err);
                out.push('\n');
            }
            out.push_str("\n... [Output tail] ...\n");
            for tl in tail {
                out.push_str(tl);
                out.push('\n');
            }
            let out_tokens = estimate_text_tokens(&out);
            if out_tokens < current_tokens {
                return out;
            }
        }
    }

    // 2. Try JSON minification
    if let Some(minified) = minify_json_tool_output(content) {
        let min_tokens = estimate_text_tokens(&minified);
        if min_tokens <= max_tokens {
            return minified;
        }
    }

    // 3. Check tool-specific patterns
    let name_str = tool_name.unwrap_or("").to_lowercase();

    if name_str.contains("git") || content.starts_with("diff --git") {
        let diff_truncated = truncate_git_diff_output(content, max_tokens);
        if estimate_text_tokens(&diff_truncated) < current_tokens {
            return diff_truncated;
        }
    }

    // 4. Default Head-Tail line truncation
    let lines: Vec<&str> = content.lines().collect();
    let total_lines = lines.len();

    if total_lines <= 8 {
        // Single or few huge lines (e.g. minified output, long base64, big json)
        let max_chars = max_tokens * 4;
        if content.len() > max_chars {
            let half = max_chars / 2;
            let head = &content[..half];
            let tail = &content[content.len() - half..];
            return format!(
                "{}\n\n... [{} characters elided for context] ...\n\n{}",
                head,
                content.len() - max_chars,
                tail
            );
        }
        return content.to_string();
    }

    // Target ~max_tokens: roughly 4 chars per token, ~80 chars per line => ~20 tokens/line
    let target_lines = (max_tokens / 15).clamp(4, 40);
    let head_lines = target_lines / 2;
    let tail_lines = target_lines / 2;

    if head_lines + tail_lines >= total_lines {
        return content.to_string();
    }

    let elided_count = total_lines - head_lines - tail_lines;
    let head = lines[..head_lines].join("\n");
    let tail = lines[total_lines - tail_lines..].join("\n");

    let tool_label = if let Some(n) = tool_name {
        format!(" `{}`", n)
    } else {
        String::new()
    };

    format!(
        "{}\n\n... [{} lines elided from{} output for context window] ...\n\n{}",
        head, elided_count, tool_label, tail
    )
}

// ============================================================================
// Tool Output Deduplication
// ============================================================================

/// Deduplicates identical tool outputs across previous turns.
/// Replaces identical repetitive results with a concise reference pointer.
pub fn deduplicate_tool_results_in_place(
    messages: &mut [Message],
    turns: &[PruneTurn],
    preserve_recent_turns: usize,
    actions: &mut Vec<PruneAction>,
) -> usize {
    let mut seen_outputs: HashMap<String, (String, usize)> = HashMap::new(); // hash -> (first_tool_call_id, first_msg_idx)
    let total_turns = turns.len();
    let cutoff_turn = total_turns.saturating_sub(preserve_recent_turns);
    let mut tokens_saved = 0usize;

    for (t_idx, turn) in turns.iter().enumerate() {
        // Only deduplicate in older turns (before cutoff)
        let is_older = t_idx < cutoff_turn;

        for &msg_idx in &turn.message_indices {
            let msg = &messages[msg_idx];
            if msg.role != Role::Tool {
                continue;
            }

            let content = &msg.content;
            // Ignore very short tool results (<= 80 chars) for deduplication
            if content.len() <= 80 {
                continue;
            }

            let tool_id = msg
                .tool_call_id
                .clone()
                .unwrap_or_else(|| "call".to_string());

            if let Some((first_id, _first_idx)) = seen_outputs.get(content) {
                if is_older {
                    let old_tokens = estimate_text_tokens(content);
                    let replacement = format!(
                        "[Duplicate tool output identical to call `{}` (~{} tokens omitted)]",
                        first_id, old_tokens
                    );
                    let new_tokens = estimate_text_tokens(&replacement);
                    let saved = old_tokens.saturating_sub(new_tokens);

                    if saved > 0 {
                        actions.push(PruneAction {
                            message_index: msg_idx,
                            role: Role::Tool,
                            action_type: PruneActionType::DeduplicateToolOutput,
                            tokens_before: old_tokens,
                            tokens_after: new_tokens,
                            description: format!(
                                "Deduplicated repeated tool output with reference to `{}` (saved {} tokens)",
                                first_id, saved
                            ),
                        });

                        messages[msg_idx].content = replacement;
                        tokens_saved += saved;
                    }
                }
            } else {
                seen_outputs.insert(content.clone(), (tool_id, msg_idx));
            }
        }
    }

    tokens_saved
}

// ============================================================================
// Core Conversation Pruner Engine
// ============================================================================

/// The primary engine for conversation context pruning.
#[derive(Debug, Clone, Default)]
pub struct ConversationPruner {
    config: PrunerConfig,
}

impl ConversationPruner {
    /// Creates a new conversation pruner with default settings.
    pub fn new() -> Self {
        Self::default()
    }

    /// Creates a new conversation pruner with custom configuration.
    pub fn with_config(config: PrunerConfig) -> Self {
        Self { config }
    }

    /// Gets a reference to the active configuration.
    pub fn config(&self) -> &PrunerConfig {
        &self.config
    }

    /// Gets a mutable reference to the active configuration.
    pub fn config_mut(&mut self) -> &mut PrunerConfig {
        &mut self.config
    }

    /// Prunes a slice of messages, returning a detailed `PruneResult`.
    pub fn prune(&self, messages: &[Message]) -> PruneResult {
        let mut cloned = messages.to_vec();
        self.prune_mut(&mut cloned)
    }

    /// Prunes a mutable vector of messages in place.
    pub fn prune_mut(&self, messages: &mut Vec<Message>) -> PruneResult {
        let original_tokens = estimate_messages_tokens(messages);
        let mut actions: Vec<PruneAction> = Vec::new();

        let mut thinking_blocks_pruned = 0usize;
        let mut thinking_tokens_saved = 0usize;
        let mut tool_outputs_truncated = 0usize;
        let mut tool_tokens_saved = 0usize;
        let mut duplicate_tools_collapsed = 0usize;
        let mut empty_messages_dropped = 0usize;

        if messages.is_empty() {
            return PruneResult {
                messages: Vec::new(),
                original_tokens: 0,
                pruned_tokens: 0,
                tokens_saved: 0,
                compression_ratio: 1.0,
                thinking_blocks_pruned: 0,
                thinking_tokens_saved: 0,
                tool_outputs_truncated: 0,
                tool_tokens_saved: 0,
                duplicate_tools_collapsed: 0,
                empty_messages_dropped: 0,
                actions: Vec::new(),
            };
        }

        // 1. Group into logical turns
        let turns = group_messages_into_turns(messages);
        let total_turns = turns.len();

        // 2. Determine turn cutoff boundaries
        let recent_turns_cutoff = total_turns.saturating_sub(self.config.preserve_recent_turns);

        // Find initial user message index if preserve_initial_goal is true
        let _initial_user_msg_idx = if self.config.preserve_initial_goal {
            messages.iter().position(|m| m.role == Role::User)
        } else {
            None
        };

        // 3. Map tool_call_id to tool name for smarter truncation
        let mut tool_id_to_name: HashMap<String, String> = HashMap::new();
        for msg in messages.iter() {
            if let Some(calls) = &msg.tool_calls {
                for tc in calls {
                    tool_id_to_name.insert(tc.id.clone(), tc.name.clone());
                }
            }
        }

        // 4. Pass 1: Prune Thinking Blocks from Assistant Messages
        for (t_idx, turn) in turns.iter().enumerate() {
            let _is_older_turn = t_idx < recent_turns_cutoff;

            for &msg_idx in &turn.message_indices {
                let msg = &messages[msg_idx];
                if msg.role != Role::Assistant {
                    continue;
                }

                // Check thinking prune policy
                let should_prune_thinking = match &self.config.thinking_policy {
                    ThinkingPrunePolicy::StripAll => true,
                    ThinkingPrunePolicy::KeepRecentTurns(keep_n) => {
                        let cutoff = total_turns.saturating_sub(*keep_n);
                        t_idx < cutoff
                    }
                    ThinkingPrunePolicy::CollapseToPlaceholder {
                        preserve_recent_turns,
                        ..
                    } => {
                        let cutoff = total_turns.saturating_sub(*preserve_recent_turns);
                        t_idx < cutoff
                    }
                    ThinkingPrunePolicy::KeepAll => false,
                };

                if should_prune_thinking && has_thinking_blocks(&msg.content) {
                    let old_tokens = estimate_message_tokens(msg);
                    let placeholder = match &self.config.thinking_policy {
                        ThinkingPrunePolicy::CollapseToPlaceholder { placeholder, .. } => {
                            placeholder.as_deref().or(Some("[thinking elided]"))
                        }
                        _ => None,
                    };

                    let (cleaned, count, _) = strip_thinking_with_stats(
                        &msg.content,
                        placeholder,
                        &self.config.custom_thinking_tags,
                    );

                    if count > 0 {
                        messages[msg_idx].content = cleaned;
                        let new_tokens = estimate_message_tokens(&messages[msg_idx]);
                        let actual_saved = old_tokens.saturating_sub(new_tokens);

                        thinking_blocks_pruned += count;
                        thinking_tokens_saved += actual_saved;

                        let action_type = if placeholder.is_some() {
                            PruneActionType::CollapseThinking
                        } else {
                            PruneActionType::StripThinking
                        };

                        actions.push(PruneAction {
                            message_index: msg_idx,
                            role: Role::Assistant,
                            action_type,
                            tokens_before: old_tokens,
                            tokens_after: new_tokens,
                            description: format!(
                                "Pruned {} thinking block(s) from turn {} (saved {} tokens)",
                                count, turn.turn_index, actual_saved
                            ),
                        });
                    }
                }
            }
        }

        // 5. Pass 2: Deduplicate identical tool outputs
        if self.config.deduplicate_tools {
            let dedup_saved = deduplicate_tool_results_in_place(
                messages,
                &turns,
                self.config.preserve_recent_turns,
                &mut actions,
            );
            duplicate_tools_collapsed += actions
                .iter()
                .filter(|a| a.action_type == PruneActionType::DeduplicateToolOutput)
                .count();
            tool_tokens_saved += dedup_saved;
        }

        // 6. Pass 3: Truncate Tool Outputs
        for (t_idx, turn) in turns.iter().enumerate() {
            let _is_older_turn = t_idx < recent_turns_cutoff;

            for &msg_idx in &turn.message_indices {
                let msg = &messages[msg_idx];
                if msg.role != Role::Tool {
                    continue;
                }

                // Check tool prune policy
                let (should_prune, max_tokens, preserve_errors, is_smart) =
                    match &self.config.tool_policy {
                        ToolPrunePolicy::Smart {
                            max_tokens,
                            preserve_recent_turns,
                            preserve_errors,
                        } => {
                            let cutoff = total_turns.saturating_sub(*preserve_recent_turns);
                            (t_idx < cutoff, *max_tokens, *preserve_errors, true)
                        }
                        ToolPrunePolicy::HeadTail {
                            max_tokens,
                            preserve_recent_turns,
                            ..
                        } => {
                            let cutoff = total_turns.saturating_sub(*preserve_recent_turns);
                            (t_idx < cutoff, *max_tokens, false, false)
                        }
                        ToolPrunePolicy::MaxTokens {
                            max_tokens,
                            preserve_recent_turns,
                        } => {
                            let cutoff = total_turns.saturating_sub(*preserve_recent_turns);
                            (t_idx < cutoff, *max_tokens, false, false)
                        }
                        ToolPrunePolicy::KeepAll => (false, 0, false, false),
                    };

                if should_prune {
                    let old_tokens = estimate_text_tokens(&msg.content);
                    if old_tokens > max_tokens {
                        let tool_name = msg
                            .tool_call_id
                            .as_ref()
                            .and_then(|id| tool_id_to_name.get(id))
                            .map(|s| s.as_str());

                        let truncated = if is_smart {
                            truncate_tool_output_smart(
                                &msg.content,
                                tool_name,
                                max_tokens,
                                preserve_errors,
                            )
                        } else if let ToolPrunePolicy::HeadTail {
                            head_lines,
                            tail_lines,
                            ..
                        } = &self.config.tool_policy
                        {
                            let lines: Vec<&str> = msg.content.lines().collect();
                            if lines.len() > head_lines + tail_lines {
                                let head = lines[..*head_lines].join("\n");
                                let tail = lines[lines.len() - *tail_lines..].join("\n");
                                let elided = lines.len() - head_lines - tail_lines;
                                format!("{}\n\n... [{} lines elided] ...\n\n{}", head, elided, tail)
                            } else {
                                msg.content.clone()
                            }
                        } else {
                            truncate_tool_output_smart(&msg.content, tool_name, max_tokens, false)
                        };

                        let new_tokens = estimate_text_tokens(&truncated);
                        if new_tokens < old_tokens {
                            let saved = old_tokens - new_tokens;
                            messages[msg_idx].content = truncated;
                            tool_outputs_truncated += 1;
                            tool_tokens_saved += saved;

                            actions.push(PruneAction {
                                message_index: msg_idx,
                                role: Role::Tool,
                                action_type: PruneActionType::TruncateToolOutput,
                                tokens_before: old_tokens,
                                tokens_after: new_tokens,
                                description: format!(
                                    "Truncated tool result in turn {} (saved {} tokens)",
                                    turn.turn_index, saved
                                ),
                            });
                        }
                    }
                }
            }
        }

        // 7. Pass 4: Progressive Multi-Tier Budget Pruning (if budget is specified and exceeded)
        if let Some(budget) = self.config.target_token_budget {
            let mut current_total = estimate_messages_tokens(messages);
            if current_total > budget {
                // Tier 1: Strip thinking blocks from ALL assistant messages (even recent ones)
                for (idx, msg) in messages.iter_mut().enumerate() {
                    if msg.role == Role::Assistant && has_thinking_blocks(&msg.content) {
                        let old_tok = estimate_message_tokens(msg);
                        let (cleaned, cnt, _) = strip_thinking_with_stats(
                            &msg.content,
                            None,
                            &self.config.custom_thinking_tags,
                        );
                        if cnt > 0 {
                            msg.content = cleaned;
                            let new_tok = estimate_message_tokens(msg);
                            let saved = old_tok.saturating_sub(new_tok);
                            thinking_blocks_pruned += cnt;
                            thinking_tokens_saved += saved;
                            actions.push(PruneAction {
                                message_index: idx,
                                role: Role::Assistant,
                                action_type: PruneActionType::StripThinking,
                                tokens_before: old_tok,
                                tokens_after: new_tok,
                                description: format!(
                                    "Budget enforcement: stripped thinking from recent assistant message (saved {} tokens)",
                                    saved
                                ),
                            });
                        }
                    }
                }

                current_total = estimate_messages_tokens(messages);

                // Tier 2: Aggressive tool truncation (down to 100 tokens per tool output across all turns)
                if current_total > budget {
                    for (idx, msg) in messages.iter_mut().enumerate() {
                        if msg.role == Role::Tool {
                            let old_tok = estimate_text_tokens(&msg.content);
                            if old_tok > 100 {
                                let truncated =
                                    truncate_tool_output_smart(&msg.content, None, 100, true);
                                let new_tok = estimate_text_tokens(&truncated);
                                if new_tok < old_tok {
                                    let saved = old_tok - new_tok;
                                    msg.content = truncated;
                                    tool_outputs_truncated += 1;
                                    tool_tokens_saved += saved;
                                    actions.push(PruneAction {
                                        message_index: idx,
                                        role: Role::Tool,
                                        action_type: PruneActionType::TruncateToolOutput,
                                        tokens_before: old_tok,
                                        tokens_after: new_tok,
                                        description: format!(
                                            "Budget enforcement: aggressive tool truncation (saved {} tokens)",
                                            saved
                                        ),
                                    });
                                }
                            }
                        }
                    }
                }
            }
        }

        // 8. Pass 5: Drop empty assistant messages if requested
        if self.config.drop_empty_assistant_messages {
            let before_count = messages.len();
            messages.retain(|m| {
                if m.role == Role::Assistant
                    && m.content.trim().is_empty()
                    && m.tool_calls.as_ref().map_or(true, |tc| tc.is_empty())
                {
                    false
                } else {
                    true
                }
            });
            empty_messages_dropped = before_count.saturating_sub(messages.len());
        }

        let pruned_tokens = estimate_messages_tokens(messages);
        let tokens_saved = original_tokens.saturating_sub(pruned_tokens);
        let compression_ratio = if original_tokens == 0 {
            1.0
        } else {
            pruned_tokens as f32 / original_tokens as f32
        };

        PruneResult {
            messages: messages.clone(),
            original_tokens,
            pruned_tokens,
            tokens_saved,
            compression_ratio,
            thinking_blocks_pruned,
            thinking_tokens_saved,
            tool_outputs_truncated,
            tool_tokens_saved,
            duplicate_tools_collapsed,
            empty_messages_dropped,
            actions,
        }
    }

    /// Prunes an active `Session` in-place, updating its messages and returning `PruneResult`.
    pub fn prune_session(&self, session: &mut Session) -> PruneResult {
        self.prune_mut(&mut session.messages)
    }
}

// ============================================================================
// Convenience Functions
// ============================================================================

/// Quick one-liner to prune a conversation using default context-saving settings.
/// Returns `(pruned_messages, tokens_saved)`.
pub fn prune_conversation(messages: &[Message]) -> (Vec<Message>, usize) {
    let pruner = ConversationPruner::new();
    let result = pruner.prune(messages);
    (result.messages, result.tokens_saved)
}

/// Prunes a conversation with custom configuration.
pub fn prune_conversation_with_config(messages: &[Message], config: &PrunerConfig) -> PruneResult {
    let pruner = ConversationPruner::with_config(config.clone());
    pruner.prune(messages)
}

/// Strips all thinking blocks in place across all messages in a slice.
/// Returns total tokens saved.
pub fn strip_all_thinking_blocks(messages: &mut [Message]) -> usize {
    let mut total_saved = 0usize;
    for msg in messages.iter_mut() {
        if msg.role == Role::Assistant && has_thinking_blocks(&msg.content) {
            let old_tok = estimate_message_tokens(msg);
            let (thoughts, cleaned) = extract_all_thinking_blocks(&msg.content);
            if !thoughts.is_empty() {
                msg.content = cleaned;
                let new_tok = estimate_message_tokens(msg);
                total_saved += old_tok.saturating_sub(new_tok);
            }
        }
    }
    total_saved
}

/// Truncates tool outputs in older turns (keeping the last `keep_recent_turns` turns intact).
/// Returns total tokens saved.
pub fn prune_tool_outputs_older_than(
    messages: &mut [Message],
    keep_recent_turns: usize,
    max_tokens: usize,
) -> usize {
    let config = PrunerConfig {
        thinking_policy: ThinkingPrunePolicy::KeepAll,
        tool_policy: ToolPrunePolicy::Smart {
            max_tokens,
            preserve_recent_turns: keep_recent_turns,
            preserve_errors: true,
        },
        preserve_recent_turns: keep_recent_turns,
        deduplicate_tools: false,
        ..Default::default()
    };

    let pruner = ConversationPruner::with_config(config);
    let mut vec_msg = messages.to_vec();
    let res = pruner.prune_mut(&mut vec_msg);
    messages.clone_from_slice(&vec_msg[..messages.len()]);
    res.tokens_saved
}

/// Prunes a conversation message list against an explicit token budget.
///
/// Strips redundant `<think>` / `<thought>` / `<reasoning>` blocks from older assistant turns
/// and elides verbose tool outputs that exceed `tool_line_threshold` lines, applying progressive
/// budget enforcement until the conversation fits within `budget` tokens.
///
/// Returns `(pruned_messages, tokens_saved)`.
///
/// # Parameters
/// - `messages` – owned conversation history.
/// - `budget` – maximum token count for the returned conversation.  Set to `usize::MAX`
///   for default context-saving behavior without a hard ceiling.
pub fn prune_messages(messages: Vec<Message>, budget: usize) -> (Vec<Message>, usize) {
    prune_messages_with_line_threshold(messages, budget, 80)
}

/// Like [`prune_messages`] but exposes the per-tool-output line truncation threshold.
///
/// Tool results whose raw line count exceeds `tool_line_threshold` are elided to a head+tail
/// window with a `[N lines elided]` banner, preserving error lines in full when detected.
///
/// Returns `(pruned_messages, tokens_saved)`.
pub fn prune_messages_with_line_threshold(
    messages: Vec<Message>,
    budget: usize,
    tool_line_threshold: usize,
) -> (Vec<Message>, usize) {
    // Translate the line threshold into a rough token cap (4 chars/token, ~60 chars/line).
    let tokens_per_line: usize = 15; // conservative: 60 chars / 4 chars-per-token
    let tool_max_tokens = tool_line_threshold.saturating_mul(tokens_per_line).max(50);

    let config = PrunerConfig {
        thinking_policy: ThinkingPrunePolicy::KeepRecentTurns(1),
        tool_policy: ToolPrunePolicy::Smart {
            max_tokens: tool_max_tokens,
            preserve_recent_turns: 1,
            preserve_errors: true,
        },
        preserve_recent_turns: 1,
        preserve_initial_goal: true,
        preserve_system_messages: true,
        deduplicate_tools: true,
        drop_empty_assistant_messages: true,
        target_token_budget: if budget == usize::MAX {
            None
        } else {
            Some(budget)
        },
        custom_thinking_tags: Vec::new(),
    };

    let pruner = ConversationPruner::with_config(config);
    let result = pruner.prune(&messages);
    (result.messages, result.tokens_saved)
}

// ============================================================================
// Unit Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::provider::types::{Role, ToolCall};

    #[test]
    fn test_extract_all_thinking_blocks_standard() {
        let text = "<think>Let me calculate 2+2.\nIt is 4.</think>The answer is 4.";
        let (thoughts, remaining) = extract_all_thinking_blocks(text);
        assert_eq!(thoughts.len(), 1);
        assert_eq!(thoughts[0], "Let me calculate 2+2.\nIt is 4.");
        assert_eq!(remaining, "The answer is 4.");
    }

    #[test]
    fn test_extract_multiple_thinking_blocks() {
        let text = "<think>First thought</think>\nMiddle text\n<thought>Second thought</thought>\nEnd text.";
        let (thoughts, remaining) = extract_all_thinking_blocks(text);
        assert_eq!(thoughts.len(), 2);
        assert_eq!(thoughts[0], "First thought");
        assert_eq!(thoughts[1], "Second thought");
        assert_eq!(remaining, "Middle text\nEnd text.");
    }

    #[test]
    fn test_extract_unclosed_thinking_block() {
        let text = "Start text\n<think>I am still thinking and stream cut off...";
        let (thoughts, remaining) = extract_all_thinking_blocks(text);
        assert_eq!(thoughts.len(), 1);
        assert_eq!(thoughts[0], "I am still thinking and stream cut off...");
        assert_eq!(remaining, "Start text");
    }

    #[test]
    fn test_extract_reasoning_and_markdown_fences() {
        let text = "```thought\nAnalyzing performance profile\n```\nHere is the optimization plan.";
        let (thoughts, remaining) = extract_all_thinking_blocks(text);
        assert_eq!(thoughts.len(), 1);
        assert_eq!(thoughts[0], "Analyzing performance profile");
        assert_eq!(remaining, "Here is the optimization plan.");
    }

    #[test]
    fn test_case_insensitive_thinking_tags() {
        let text = "<THINK>Thinking in uppercase</THINK>Final answer.";
        let (thoughts, remaining) = extract_all_thinking_blocks(text);
        assert_eq!(thoughts.len(), 1);
        assert_eq!(thoughts[0], "Thinking in uppercase");
        assert_eq!(remaining, "Final answer.");
    }

    #[test]
    fn test_custom_thinking_tags() {
        let text = "<my_reasoning>Step 1</my_reasoning>Result.";
        let custom = vec![("<my_reasoning>".to_string(), "</my_reasoning>".to_string())];
        let (thoughts, remaining) = extract_thinking_blocks_with_custom_tags(text, &custom);
        assert_eq!(thoughts.len(), 1);
        assert_eq!(thoughts[0], "Step 1");
        assert_eq!(remaining, "Result.");
    }

    #[test]
    fn test_strip_thinking_with_placeholder() {
        let text = "<think>Long complex chain of thought</think>Here is the plan.";
        let (cleaned, count, saved) =
            strip_thinking_with_stats(text, Some("[reasoning omitted]"), &[]);
        assert_eq!(count, 1);
        assert!(cleaned.starts_with("[reasoning omitted]"));
        assert!(cleaned.contains("Here is the plan."));
        assert!(saved > 0);
    }

    #[test]
    fn test_is_error_tool_output() {
        assert!(is_error_tool_output(
            "error[E0432]: unresolved import `foo`"
        ));
        assert!(is_error_tool_output("FATAL: database connection refused"));
        assert!(is_error_tool_output("Process exited with exit status: 1"));
        assert!(!is_error_tool_output(
            "File successfully saved to disk. 120 lines written."
        ));
    }

    #[test]
    fn test_minify_json_tool_output() {
        let json_pretty = "{\n  \"status\": \"success\",\n  \"count\": 42\n}";
        let minified = minify_json_tool_output(json_pretty);
        assert!(minified.is_some());
        assert_eq!(minified.unwrap(), "{\"count\":42,\"status\":\"success\"}");
    }

    #[test]
    fn test_truncate_tool_output_smart_long_text() {
        let long_text = (0..200)
            .map(|i| {
                format!(
                    "Line {}: Some detailed logging output from server process",
                    i
                )
            })
            .collect::<Vec<_>>()
            .join("\n");

        let truncated = truncate_tool_output_smart(&long_text, Some("bash"), 50, true);
        assert!(truncated.contains("lines elided"));
        assert!(truncated.contains("Line 0:"));
        assert!(truncated.contains("Line 199:"));
        assert!(estimate_text_tokens(&truncated) < estimate_text_tokens(&long_text));
    }

    #[test]
    fn test_truncate_tool_output_preserves_errors() {
        let mut lines = Vec::new();
        for i in 0..50 {
            lines.push(format!("Normal log line {}", i));
        }
        lines.push("ERROR: Segmentation fault in core module".to_string());
        for i in 51..100 {
            lines.push(format!("Normal log line {}", i));
        }
        let full_output = lines.join("\n");

        let truncated = truncate_tool_output_smart(&full_output, Some("bash"), 40, true);
        assert!(truncated.contains("preserved error diagnostics"));
        assert!(truncated.contains("ERROR: Segmentation fault in core module"));
    }

    #[test]
    fn test_conversation_pruner_end_to_end() {
        let messages = vec![
            Message::system("You are an expert Rust assistant."),
            // Turn 1 (Old turn - should have tool truncated & thinking stripped)
            Message::user("Please inspect the codebase."),
            Message::assistant_with_tools(
                "<think>I will run glob to find files.</think>Running glob...",
                vec![ToolCall {
                    id: "call_1".to_string(),
                    name: "glob".to_string(),
                    arguments: "{}".to_string(),
                }],
            ),
            Message::tool_result(
                "call_1",
                (0..150)
                    .map(|i| format!("src/file_{}.rs", i))
                    .collect::<Vec<_>>()
                    .join("\n"),
            ),
            Message::assistant("<think>I analyzed all 150 files.</think>Found 150 files."),
            // Turn 2 (Intermediate turn)
            Message::user("Read file_0.rs."),
            Message::assistant("<think>Thinking about file 0</think>File 0 is good."),
            // Turn 3 (Recent turn - should preserve thinking and tools)
            Message::user("Now optimize file_0.rs."),
            Message::assistant(
                "<think>Thinking about optimization for file 0</think>Here is the optimized code.",
            ),
        ];

        let pruner = ConversationPruner::new();
        let res = pruner.prune(&messages);

        assert!(res.is_pruned());
        assert!(res.tokens_saved > 0);
        assert!(res.thinking_blocks_pruned >= 2); // Old turn thinking stripped
        assert!(res.tool_outputs_truncated >= 1); // Large tool result truncated

        // Verify turn 1 assistant message has no <think>
        assert!(!res.messages[2].content.contains("<think>"));
        assert_eq!(res.messages[2].content, "Running glob...");

        // Verify recent turn 3 assistant message PRESERVED its <think>
        assert!(res.messages[8]
            .content
            .contains("<think>Thinking about optimization for file 0</think>"));
    }

    #[test]
    fn test_deduplicate_identical_tool_results() {
        let long_output = "Identical status output: branch main is up to date with origin/main.\nNothing to commit, working tree clean.".repeat(3);

        let messages = vec![
            // Turn 1
            Message::user("Check git status"),
            Message::assistant_with_tools(
                "Checking status",
                vec![ToolCall {
                    id: "call_a".to_string(),
                    name: "git".to_string(),
                    arguments: "{}".to_string(),
                }],
            ),
            Message::tool_result("call_a", long_output.clone()),
            Message::assistant("Status is clean."),
            // Turn 2 (Duplicate tool output in older turn)
            Message::user("Check git status again"),
            Message::assistant_with_tools(
                "Checking status again",
                vec![ToolCall {
                    id: "call_b".to_string(),
                    name: "git".to_string(),
                    arguments: "{}".to_string(),
                }],
            ),
            Message::tool_result("call_b", long_output.clone()),
            Message::assistant("Status is still clean."),
            // Turn 3 (Recent turn)
            Message::user("What should we do next?"),
            Message::assistant("We can start building."),
        ];

        let config = PrunerConfig {
            preserve_recent_turns: 1,
            deduplicate_tools: true,
            ..Default::default()
        };

        let pruner = ConversationPruner::with_config(config);
        let res = pruner.prune(&messages);

        assert!(res.duplicate_tools_collapsed >= 1);
        assert!(res.messages[6]
            .content
            .contains("Duplicate tool output identical to call `call_a`"));
    }

    #[test]
    fn test_budget_aware_progressive_pruning() {
        let messages = vec![
            Message::system("System prompt"),
            Message::user("Prompt 1"),
            Message::assistant("<think>Long detailed chain of thought step 1 2 3</think>Answer 1"),
            Message::user("Prompt 2"),
            Message::assistant("<think>Long detailed chain of thought step 4 5 6</think>Answer 2"),
            Message::user("Prompt 3"),
            Message::assistant("<think>Long detailed chain of thought step 7 8 9</think>Answer 3"),
        ];

        // Target a budget that requires stripping all thinking blocks
        let config = PrunerConfig::new().with_target_token_budget(50);
        let pruner = ConversationPruner::with_config(config);
        let res = pruner.prune(&messages);

        assert!(res.pruned_tokens <= 50);
        // All thinking blocks stripped to meet budget
        for msg in &res.messages {
            if msg.role == Role::Assistant {
                assert!(!msg.content.contains("<think>"));
            }
        }
    }

    #[test]
    fn test_prune_session_in_place() {
        let mut session = Session::new("test-model");
        session.messages.push(Message::user("Hello"));
        session
            .messages
            .push(Message::assistant("<think>Pondering</think>Hi there!"));

        let pruner = ConversationPruner::with_config(PrunerConfig {
            thinking_policy: ThinkingPrunePolicy::StripAll,
            ..Default::default()
        });

        let res = pruner.prune_session(&mut session);
        assert!(res.is_pruned());
        assert_eq!(session.messages[1].content, "Hi there!");
    }
}
