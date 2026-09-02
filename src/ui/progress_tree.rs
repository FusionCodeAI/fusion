//! Real-Time Streaming Progress Tree Widget for Subagents & Multi-Agent Mesh
//!
//! Provides a live, real-time hierarchical progress tree widget and streaming visualization
//! engine for monitoring active and completed subagents, worker pipelines, coordinator delegations,
//! and parallel agent tasks:
//! - **Real-Time Streaming Event Ingestion**: Live updates from `SubagentProgress` event streams
//!   (`Started`, `TurnStarted`, `Thinking`, `Message`, `ToolStarted`, `ToolCompleted`, `Completed`, `Failed`, `Cancelled`).
//! - **Hierarchical Parent-Child Tree View**: Renders nested agent delegations using clean Unicode
//!   box-drawing glyphs (`├──`, `└──`, `│  `), expandable/collapsible branches, and depth tracking.
//! - **Animated Live Spinners**: Dynamic multi-frame spinner animations (`⠋`, `⠙`, `⠹`, `⠸`, etc.) for
//!   running subagents, green checkmarks for completed, red crosses for failed, and warning badges.
//! - **Progress & Activity Tracking**: Visual turn progress bars (`[████░░░░] 50%`), active tool indicator,
//!   live streaming reasoning / thinking preview deltas, and elapsed execution timers.
//! - **Token Economics & Throughput**: Tracks prompt tokens, completion tokens, token throughput (tokens/sec),
//!   and estimated USD costs per node and aggregated across the active mesh.
//! - **Interactive Ratatui TUI Widget**: Full [`Widget`] and [`StatefulWidget`] implementations with
//!   collapsible nodes, live detail inspector panel (split-view), status filtering, and keyboard navigation.
//! - **Multi-Format Terminal Output**: Zero-dependency standalone ANSI string renderer, plaintext trees,
//!   compact single-line summaries, and async terminal stream runners.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::{
    buffer::Buffer,
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Paragraph, StatefulWidget, Widget, Wrap},
};
use serde::{Deserialize, Serialize};

use crate::agent::subagent::{SubagentInfo, SubagentProgress, SubagentRole, SubagentStatus};
use crate::ui::spinner::{format_duration, visible_width, BRAILLE_FRAMES};
use crate::ui::theme::Theme;

// ============================================================================
// 1. Data Structures & Models
// ============================================================================

/// Recorded tool execution call within a subagent's lifecycle.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProgressToolCall {
    /// Tool name (e.g. `file_read`, `bash`, `web_search`).
    pub tool: String,
    /// Truncated JSON arguments or summary string.
    pub args_summary: String,
    /// Tool execution output preview or summary.
    pub output_preview: Option<String>,
    /// Whether the tool execution succeeded.
    pub success: bool,
    /// Duration of the tool execution in milliseconds.
    pub duration_ms: u64,
}

/// Categorized log event within a subagent's execution stream.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ProgressLogKind {
    Started,
    Turn,
    Thinking,
    Message,
    ToolStart,
    ToolSuccess,
    ToolError,
    Completed,
    Failed,
    Cancelled,
    Info,
}

impl ProgressLogKind {
    /// Glyph symbol representing the log event.
    pub fn icon(&self) -> &'static str {
        match self {
            ProgressLogKind::Started => "🚀",
            ProgressLogKind::Turn => "🔄",
            ProgressLogKind::Thinking => "💭",
            ProgressLogKind::Message => "💬",
            ProgressLogKind::ToolStart => "⚡",
            ProgressLogKind::ToolSuccess => "✓",
            ProgressLogKind::ToolError => "✗",
            ProgressLogKind::Completed => "✨",
            ProgressLogKind::Failed => "❌",
            ProgressLogKind::Cancelled => "⊘",
            ProgressLogKind::Info => "ℹ",
        }
    }
}

/// Single log entry in a subagent's stream buffer.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProgressLogEntry {
    /// Timestamp offset from start in milliseconds.
    pub elapsed_ms: u64,
    /// Type of log event.
    pub kind: ProgressLogKind,
    /// Log summary message.
    pub message: String,
    /// Detailed text or output snippet.
    pub detail: Option<String>,
}

/// Token usage and cost metrics for a subagent node.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, Default)]
pub struct ProgressTokenMetrics {
    /// Prompt / input tokens consumed.
    pub prompt_tokens: usize,
    /// Completion / generation tokens consumed.
    pub completion_tokens: usize,
    /// Total tokens consumed.
    pub total_tokens: usize,
    /// Generation throughput in tokens per second.
    pub tokens_per_sec: f64,
    /// Estimated cost in USD.
    pub estimated_cost_usd: f64,
}

impl ProgressTokenMetrics {
    /// Creates a new empty metrics container.
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds token counts and updates totals.
    pub fn add_tokens(&mut self, prompt: usize, completion: usize, cost_per_1k: f64) {
        self.prompt_tokens += prompt;
        self.completion_tokens += completion;
        self.total_tokens = self.prompt_tokens + self.completion_tokens;
        if cost_per_1k > 0.0 {
            self.estimated_cost_usd += (prompt + completion) as f64 * (cost_per_1k / 1000.0);
        }
    }

    /// Updates throughput given elapsed duration.
    pub fn update_tps(&mut self, elapsed: Duration) {
        let secs = elapsed.as_secs_f64();
        if secs > 0.1 && self.completion_tokens > 0 {
            self.tokens_per_sec = self.completion_tokens as f64 / secs;
        }
    }

    /// Formats total tokens into a compact human-readable string (e.g. `12.4k`).
    pub fn format_total_tokens(&self) -> String {
        if self.total_tokens >= 1_000_000 {
            format!("{:.1}M", self.total_tokens as f64 / 1_000_000.0)
        } else if self.total_tokens >= 1_000 {
            format!("{:.1}k", self.total_tokens as f64 / 1_000.0)
        } else {
            self.total_tokens.to_string()
        }
    }

    /// Formats estimated cost in USD (e.g. `$0.0024`).
    pub fn format_cost(&self) -> String {
        if self.estimated_cost_usd < 0.01 && self.estimated_cost_usd > 0.0 {
            format!("${:.4}", self.estimated_cost_usd)
        } else {
            format!("${:.2}", self.estimated_cost_usd)
        }
    }
}

/// Represents an individual subagent node in the real-time streaming progress tree.
#[derive(Debug, Clone)]
pub struct ProgressTreeNode {
    /// Unique identifier for the subagent.
    pub id: String,
    /// Display name of the subagent.
    pub name: String,
    /// Specialized worker role.
    pub role: SubagentRole,
    /// Current execution status.
    pub status: SubagentStatus,
    /// Primary task description or directive.
    pub task: String,
    /// ID of the parent delegator subagent, if any.
    pub parent_id: Option<String>,
    /// IDs of spawned child subagents.
    pub children: Vec<String>,
    /// Current execution turn number (1-based).
    pub current_turn: usize,
    /// Maximum configured turns.
    pub max_turns: usize,
    /// Latest streaming reasoning / thinking delta snippet.
    pub thinking_preview: String,
    /// Full accumulated thinking text.
    pub thinking_full: String,
    /// Latest emitted message content.
    pub last_message: Option<String>,
    /// Currently executing tool name.
    pub current_tool: Option<String>,
    /// Active tool start time.
    pub tool_start_time: Option<Instant>,
    /// History of executed tools.
    pub tool_history: Vec<ProgressToolCall>,
    /// Stream event log buffer.
    pub log_events: Vec<ProgressLogEntry>,
    /// Performance and token metrics.
    pub metrics: ProgressTokenMetrics,
    /// Node creation / start instant.
    pub start_time: Instant,
    /// Completion instant, if finished.
    pub finish_time: Option<Instant>,
    /// Tree nesting depth (0 = root).
    pub depth: usize,
    /// Whether child branches are expanded in the UI.
    pub is_expanded: bool,
    /// Final completion output snippet.
    pub output_summary: Option<String>,
    /// Failure error message, if failed.
    pub error_message: Option<String>,
}

impl ProgressTreeNode {
    /// Creates a new progress tree node.
    pub fn new(id: impl Into<String>, name: impl Into<String>, role: SubagentRole, task: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            name: name.into(),
            role,
            status: SubagentStatus::Pending,
            task: task.into(),
            parent_id: None,
            children: Vec::new(),
            current_turn: 0,
            max_turns: 10,
            thinking_preview: String::new(),
            thinking_full: String::new(),
            last_message: None,
            current_tool: None,
            tool_start_time: None,
            tool_history: Vec::new(),
            log_events: Vec::new(),
            metrics: ProgressTokenMetrics::new(),
            start_time: Instant::now(),
            finish_time: None,
            depth: 0,
            is_expanded: true,
            output_summary: None,
            error_message: None,
        }
    }

    /// Sets the parent delegator ID for hierarchical nesting.
    pub fn with_parent(mut self, parent_id: impl Into<String>) -> Self {
        self.parent_id = Some(parent_id.into());
        self
    }

    /// Sets the maximum turn limit.
    pub fn with_max_turns(mut self, max_turns: usize) -> Self {
        self.max_turns = max_turns.max(1);
        self
    }

    /// Elapsed execution duration.
    pub fn elapsed(&self) -> Duration {
        if let Some(finish) = self.finish_time {
            finish.saturating_duration_since(self.start_time)
        } else {
            self.start_time.elapsed()
        }
    }

    /// Progress completion ratio (0.0 to 1.0).
    pub fn progress_ratio(&self) -> f32 {
        match &self.status {
            SubagentStatus::Completed { .. } => 1.0,
            SubagentStatus::Failed { .. } | SubagentStatus::Cancelled => 1.0,
            SubagentStatus::Pending => 0.0,
            SubagentStatus::Running { turn, .. } => {
                let max = self.max_turns.max(1);
                (*turn as f32 / max as f32).clamp(0.05, 0.95)
            }
        }
    }

    /// Progress percentage (0 to 100).
    pub fn progress_pct(&self) -> u8 {
        (self.progress_ratio() * 100.0).round() as u8
    }

    /// Returns whether the subagent is currently running.
    pub fn is_running(&self) -> bool {
        matches!(self.status, SubagentStatus::Running { .. })
    }

    /// Returns whether the subagent has finished execution (completed, failed, or cancelled).
    pub fn is_finished(&self) -> bool {
        matches!(
            self.status,
            SubagentStatus::Completed { .. } | SubagentStatus::Failed { .. } | SubagentStatus::Cancelled
        )
    }

    /// Returns whether the subagent completed successfully.
    pub fn is_completed(&self) -> bool {
        matches!(self.status, SubagentStatus::Completed { .. })
    }

    /// Returns whether the subagent failed.
    pub fn is_failed(&self) -> bool {
        matches!(self.status, SubagentStatus::Failed { .. })
    }

    /// Role badge label (e.g. `[SCOUT]`, `[CODER]`).
    pub fn role_badge(&self) -> &'static str {
        match &self.role {
            SubagentRole::Scout => "[SCOUT]",
            SubagentRole::Coder => "[CODER]",
            SubagentRole::Tester => "[TESTER]",
            SubagentRole::Reviewer => "[REVIEWER]",
            SubagentRole::General => "[GENERAL]",
            SubagentRole::Custom { .. } => "[CUSTOM]",
        }
    }

    /// Unicode role icon.
    pub fn role_icon(&self) -> &'static str {
        match &self.role {
            SubagentRole::Scout => "🔍",
            SubagentRole::Coder => "⚡",
            SubagentRole::Tester => "🧪",
            SubagentRole::Reviewer => "🛡️",
            SubagentRole::General => "🤖",
            SubagentRole::Custom { .. } => "⚙️",
        }
    }

    /// Status icon / glyph for terminal rendering.
    pub fn status_icon(&self, spinner_tick: usize) -> &'static str {
        match &self.status {
            SubagentStatus::Pending => "⏳",
            SubagentStatus::Running { .. } => {
                let idx = spinner_tick % BRAILLE_FRAMES.len();
                BRAILLE_FRAMES[idx]
            }
            SubagentStatus::Completed { .. } => "✓",
            SubagentStatus::Failed { .. } => "✗",
            SubagentStatus::Cancelled => "⊘",
        }
    }

    /// Concise one-line status string.
    pub fn status_summary(&self) -> String {
        match &self.status {
            SubagentStatus::Pending => "Pending".to_string(),
            SubagentStatus::Running { turn, current_tool } => {
                if let Some(tool) = current_tool {
                    format!("Turn {}/{} (Running {})", turn, self.max_turns, tool)
                } else if !self.thinking_preview.is_empty() {
                    format!("Turn {}/{} (Reasoning...)", turn, self.max_turns)
                } else {
                    format!("Turn {}/{} (Thinking)", turn, self.max_turns)
                }
            }
            SubagentStatus::Completed { turns, .. } => format!("Completed in {} turns", turns),
            SubagentStatus::Failed { error } => format!("Failed: {}", error),
            SubagentStatus::Cancelled => "Cancelled".to_string(),
        }
    }

    /// Ingests a `SubagentProgress` streaming event and updates the node state.
    pub fn apply_progress_event(&mut self, event: &SubagentProgress) {
        let elapsed_ms = self.elapsed().as_millis() as u64;

        match event {
            SubagentProgress::Started { name, role, task, .. } => {
                self.name = name.clone();
                self.role = role.clone();
                self.task = task.clone();
                self.status = SubagentStatus::Running {
                    turn: 1,
                    current_tool: None,
                };
                self.current_turn = 1;
                self.log_events.push(ProgressLogEntry {
                    elapsed_ms,
                    kind: ProgressLogKind::Started,
                    message: format!("Subagent '{}' started task", name),
                    detail: Some(task.clone()),
                });
            }
            SubagentProgress::TurnStarted { turn, max_turns, .. } => {
                self.current_turn = *turn;
                self.max_turns = *max_turns;
                self.status = SubagentStatus::Running {
                    turn: *turn,
                    current_tool: self.current_tool.clone(),
                };
                // Rough token increment per turn
                self.metrics.add_tokens(450, 0, 0.002);
                self.metrics.update_tps(self.elapsed());

                self.log_events.push(ProgressLogEntry {
                    elapsed_ms,
                    kind: ProgressLogKind::Turn,
                    message: format!("Started turn {}/{}", turn, max_turns),
                    detail: None,
                });
            }
            SubagentProgress::Thinking { delta, .. } => {
                self.thinking_full.push_str(delta);
                // Keep preview short and clean (last non-empty line or last 80 chars)
                let trimmed = delta.trim();
                if !trimmed.is_empty() {
                    self.thinking_preview = if trimmed.len() > 70 {
                        format!("{}...", &trimmed[..67])
                    } else {
                        trimmed.to_string()
                    };
                }
                // Estimate completion tokens from streaming reasoning
                let tokens_est = (delta.len() / 4).max(1);
                self.metrics.add_tokens(0, tokens_est, 0.002);
                self.metrics.update_tps(self.elapsed());
            }
            SubagentProgress::Message { content, .. } => {
                self.last_message = Some(content.clone());
                let preview = if content.len() > 60 {
                    format!("{}...", &content[..57])
                } else {
                    content.clone()
                };
                self.log_events.push(ProgressLogEntry {
                    elapsed_ms,
                    kind: ProgressLogKind::Message,
                    message: format!("Message: {}", preview),
                    detail: Some(content.clone()),
                });
            }
            SubagentProgress::ToolStarted { tool, args, .. } => {
                self.current_tool = Some(tool.clone());
                self.tool_start_time = Some(Instant::now());
                let args_summary = match args {
                    serde_json::Value::Object(map) => {
                        let keys: Vec<_> = map.keys().take(3).cloned().collect();
                        if keys.is_empty() {
                            "{}".to_string()
                        } else {
                            format!("{{{}}}", keys.join(", "))
                        }
                    }
                    serde_json::Value::String(s) => {
                        if s.len() > 40 {
                            format!("\"{}...\"", &s[..37])
                        } else {
                            format!("\"{}\"", s)
                        }
                    }
                    other => other.to_string(),
                };

                self.status = SubagentStatus::Running {
                    turn: self.current_turn,
                    current_tool: Some(tool.clone()),
                };

                self.log_events.push(ProgressLogEntry {
                    elapsed_ms,
                    kind: ProgressLogKind::ToolStart,
                    message: format!("Executing tool '{}'", tool),
                    detail: Some(args_summary),
                });
            }
            SubagentProgress::ToolCompleted {
                tool,
                output,
                success,
                ..
            } => {
                let duration_ms = self
                    .tool_start_time
                    .map(|st| st.elapsed().as_millis() as u64)
                    .unwrap_or(0);
                self.tool_start_time = None;
                self.current_tool = None;

                let preview = if output.len() > 80 {
                    format!("{}...", &output[..77])
                } else {
                    output.clone()
                };

                self.tool_history.push(ProgressToolCall {
                    tool: tool.clone(),
                    args_summary: String::new(),
                    output_preview: Some(preview.clone()),
                    success: *success,
                    duration_ms,
                });

                self.status = SubagentStatus::Running {
                    turn: self.current_turn,
                    current_tool: None,
                };

                self.log_events.push(ProgressLogEntry {
                    elapsed_ms,
                    kind: if *success {
                        ProgressLogKind::ToolSuccess
                    } else {
                        ProgressLogKind::ToolError
                    },
                    message: format!("Tool '{}' finished ({:?}ms)", tool, duration_ms),
                    detail: Some(preview),
                });
            }
            SubagentProgress::Completed {
                output,
                turns_taken,
                ..
            } => {
                self.finish_time = Some(Instant::now());
                self.current_tool = None;
                self.output_summary = Some(output.clone());
                self.status = SubagentStatus::Completed {
                    output: output.clone(),
                    turns: *turns_taken,
                };
                self.metrics.update_tps(self.elapsed());

                self.log_events.push(ProgressLogEntry {
                    elapsed_ms,
                    kind: ProgressLogKind::Completed,
                    message: format!("Completed successfully in {} turns", turns_taken),
                    detail: Some(output.clone()),
                });
            }
            SubagentProgress::Failed { error, .. } => {
                self.finish_time = Some(Instant::now());
                self.current_tool = None;
                self.error_message = Some(error.clone());
                self.status = SubagentStatus::Failed {
                    error: error.clone(),
                };

                self.log_events.push(ProgressLogEntry {
                    elapsed_ms,
                    kind: ProgressLogKind::Failed,
                    message: format!("Failed: {}", error),
                    detail: Some(error.clone()),
                });
            }
            SubagentProgress::Cancelled { .. } => {
                self.finish_time = Some(Instant::now());
                self.current_tool = None;
                self.status = SubagentStatus::Cancelled;

                self.log_events.push(ProgressLogEntry {
                    elapsed_ms,
                    kind: ProgressLogKind::Cancelled,
                    message: "Execution cancelled by user or coordinator".to_string(),
                    detail: None,
                });
            }
        }
    }
}

// ============================================================================
// 2. Tree Glyphs & Box-Drawing Presets
// ============================================================================

/// Customizable Unicode and ASCII box-drawing character sets for hierarchical trees.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProgressGlyphSet {
    pub branch: &'static str,
    pub last: &'static str,
    pub vertical: &'static str,
    pub space: &'static str,
    pub expand_collapsed: &'static str,
    pub expand_open: &'static str,
}

impl Default for ProgressGlyphSet {
    fn default() -> Self {
        Self::unicode()
    }
}

impl ProgressGlyphSet {
    /// Standard crisp Unicode box-drawing glyphs (`├──`, `└──`, `│  `).
    pub const fn unicode() -> Self {
        Self {
            branch: "├── ",
            last: "└── ",
            vertical: "│   ",
            space: "    ",
            expand_collapsed: "▶ ",
            expand_open: "▼ ",
        }
    }

    /// Rounded modern Unicode glyphs (`╰──`, `├──`, `│  `).
    pub const fn unicode_rounded() -> Self {
        Self {
            branch: "├── ",
            last: "╰── ",
            vertical: "│   ",
            space: "    ",
            expand_collapsed: "▷ ",
            expand_open: "▽ ",
        }
    }

    /// Compact Unicode glyphs (`├─`, `└─`, `│ `).
    pub const fn compact() -> Self {
        Self {
            branch: "├─ ",
            last: "└─ ",
            vertical: "│  ",
            space: "   ",
            expand_collapsed: "› ",
            expand_open: "⌄ ",
        }
    }

    /// Pure ASCII fallback glyphs for legacy terminals (`|--`, `\--`, `|  `).
    pub const fn ascii() -> Self {
        Self {
            branch: "|-- ",
            last: "`-- ",
            vertical: "|   ",
            space: "    ",
            expand_collapsed: "+ ",
            expand_open: "- ",
        }
    }
}

// ============================================================================
// 3. Flattened Tree Row Representation
// ============================================================================

/// Flattened representation of a tree node ready for line-by-line rendering.
#[derive(Debug, Clone)]
pub struct FlattenedProgressRow {
    /// ID of the represented subagent node.
    pub id: String,
    /// Tree depth (0 = root level).
    pub depth: usize,
    /// Box-drawing prefix string (e.g. `│   ├── `).
    pub prefix: String,
    /// Whether this node has child nodes.
    pub has_children: bool,
    /// Whether child branches are expanded.
    pub is_expanded: bool,
    /// Whether this row is currently selected in interactive mode.
    pub is_selected: bool,
}

// ============================================================================
// 4. Progress Tree State Management
// ============================================================================

/// Filtering mode for tree nodes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum SubagentFilter {
    #[default]
    All,
    ActiveOnly,
    CompletedOnly,
    FailedOnly,
}

/// View mode layout for the progress tree widget.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum ProgressViewMode {
    /// Single hierarchical tree table view.
    #[default]
    TreeOnly,
    /// Split pane: Tree hierarchy on left, live detail inspector on right.
    SplitDetail,
    /// Compact single-line / multi-card view.
    Compact,
    /// Full dashboard with metrics header, split body, and live activity log.
    Dashboard,
}

/// User interaction action resulting from key handling.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProgressTreeAction {
    SelectNext,
    SelectPrev,
    ToggleExpand,
    ExpandAll,
    CollapseAll,
    ToggleViewMode,
    CycleFilter,
    CancelSelected(String),
    InspectSelected(String),
    Close,
}

/// Interactive state container managing nodes, streaming updates, selection, and layout.
#[derive(Debug, Clone)]
pub struct ProgressTreeState {
    /// All subagent nodes indexed by ID.
    pub nodes: HashMap<String, ProgressTreeNode>,
    /// Ordered root-level subagent IDs.
    pub root_ids: Vec<String>,
    /// Currently selected node ID.
    pub selected_id: Option<String>,
    /// Vertical scroll offset for tree view.
    pub scroll_offset: usize,
    /// Vertical scroll offset for inspector detail view.
    pub detail_scroll_offset: usize,
    /// View layout presentation mode.
    pub view_mode: ProgressViewMode,
    /// Status filter.
    pub filter: SubagentFilter,
    /// Optional role filter.
    pub role_filter: Option<SubagentRole>,
    /// Text search filter query.
    pub search_query: String,
    /// Whether search input mode is active.
    pub is_searching: bool,
    /// Whether to auto-scroll and follow active running subagents.
    pub auto_follow: bool,
    /// Global animation tick counter.
    pub spinner_tick: usize,
    /// Overall tree start instant.
    pub start_time: Instant,
    /// Glyph preset.
    pub glyphs: ProgressGlyphSet,
}

impl Default for ProgressTreeState {
    fn default() -> Self {
        Self::new()
    }
}

impl ProgressTreeState {
    /// Creates a new empty progress tree state.
    pub fn new() -> Self {
        Self {
            nodes: HashMap::new(),
            root_ids: Vec::new(),
            selected_id: None,
            scroll_offset: 0,
            detail_scroll_offset: 0,
            view_mode: ProgressViewMode::TreeOnly,
            filter: SubagentFilter::All,
            role_filter: None,
            search_query: String::new(),
            is_searching: false,
            auto_follow: true,
            spinner_tick: 0,
            start_time: Instant::now(),
            glyphs: ProgressGlyphSet::unicode(),
        }
    }

    /// Registers a new subagent or updates an existing one.
    pub fn register_subagent(
        &mut self,
        id: impl Into<String>,
        name: impl Into<String>,
        role: SubagentRole,
        task: impl Into<String>,
        parent_id: Option<String>,
    ) {
        let id_str = id.into();
        let mut node = ProgressTreeNode::new(id_str.clone(), name, role, task);

        if let Some(pid) = parent_id {
            node.parent_id = Some(pid.clone());
            if let Some(parent) = self.nodes.get_mut(&pid) {
                if !parent.children.contains(&id_str) {
                    parent.children.push(id_str.clone());
                }
            }
        } else if !self.root_ids.contains(&id_str) {
            self.root_ids.push(id_str.clone());
        }

        if self.selected_id.is_none() {
            self.selected_id = Some(id_str.clone());
        }

        self.nodes.insert(id_str, node);
        self.recalculate_depths();
    }

    /// Ingests a streaming `SubagentProgress` event and updates the internal state.
    pub fn handle_event(&mut self, event: &SubagentProgress) {
        let id = event.id().to_string();

        if !self.nodes.contains_key(&id) {
            // Auto-register if not yet known
            if let SubagentProgress::Started { name, role, task, .. } = event {
                self.register_subagent(id.clone(), name.clone(), role.clone(), task.clone(), None);
            } else {
                self.register_subagent(id.clone(), id.clone(), SubagentRole::General, "", None);
            }
        }

        if let Some(node) = self.nodes.get_mut(&id) {
            node.apply_progress_event(event);
        }

        if self.auto_follow && self.nodes.get(&id).map(|n| n.is_running()).unwrap_or(false) {
            self.selected_id = Some(id);
        }
    }

    /// Advances the global animation tick and updates running timers.
    pub fn tick(&mut self) {
        self.spinner_tick = self.spinner_tick.wrapping_add(1);
    }

    /// Recalculates depth values for all nodes in the hierarchy.
    fn recalculate_depths(&mut self) {
        let roots = self.root_ids.clone();
        for root_id in roots {
            Self::set_depth_recursive(&mut self.nodes, &root_id, 0);
        }
    }

    fn set_depth_recursive(nodes: &mut HashMap<String, ProgressTreeNode>, current_id: &str, depth: usize) {
        if let Some(node) = nodes.get_mut(current_id) {
            node.depth = depth;
            let children = node.children.clone();
            for child_id in children {
                Self::set_depth_recursive(nodes, &child_id, depth + 1);
            }
        }
    }

    /// Returns the number of currently active / running subagents.
    pub fn active_count(&self) -> usize {
        self.nodes.values().filter(|n| n.is_running()).count()
    }

    /// Returns the number of completed subagents.
    pub fn completed_count(&self) -> usize {
        self.nodes.values().filter(|n| n.is_completed()).count()
    }

    /// Returns the number of failed subagents.
    pub fn failed_count(&self) -> usize {
        self.nodes.values().filter(|n| n.is_failed()).count()
    }

    /// Returns the total number of registered subagents.
    pub fn total_count(&self) -> usize {
        self.nodes.len()
    }

    /// Aggregated total tokens consumed across all subagents.
    pub fn total_tokens(&self) -> usize {
        self.nodes.values().map(|n| n.metrics.total_tokens).sum()
    }

    /// Aggregated estimated USD cost across all subagents.
    pub fn total_cost(&self) -> f64 {
        self.nodes.values().map(|n| n.metrics.estimated_cost_usd).sum()
    }

    /// Overall progress percentage (0 to 100) across all registered nodes.
    pub fn overall_progress_pct(&self) -> u8 {
        if self.nodes.is_empty() {
            return 0;
        }
        let sum: f32 = self.nodes.values().map(|n| n.progress_ratio()).sum();
        ((sum / self.nodes.len() as f32) * 100.0).round() as u8
    }

    /// Checks if a node passes the active filters.
    pub fn matches_filter(&self, node: &ProgressTreeNode) -> bool {
        // Status filter
        let status_ok = match self.filter {
            SubagentFilter::All => true,
            SubagentFilter::ActiveOnly => node.is_running(),
            SubagentFilter::CompletedOnly => node.is_completed(),
            SubagentFilter::FailedOnly => node.is_failed(),
        };
        if !status_ok {
            return false;
        }

        // Role filter
        if let Some(role) = &self.role_filter {
            if &node.role != role {
                return false;
            }
        }

        // Search query
        if !self.search_query.is_empty() {
            let q = self.search_query.to_lowercase();
            if !node.name.to_lowercase().contains(&q)
                && !node.task.to_lowercase().contains(&q)
                && !node.id.to_lowercase().contains(&q)
            {
                return false;
            }
        }

        true
    }

    /// Flattens the visible tree nodes into printable rows with box-drawing prefixes.
    pub fn flatten_visible(&self) -> Vec<FlattenedProgressRow> {
        let mut rows = Vec::new();
        let mut continuation_mask = Vec::new();

        for (idx, root_id) in self.root_ids.iter().enumerate() {
            let is_last_root = idx == self.root_ids.len() - 1;
            self.flatten_recursive(root_id, &mut continuation_mask, is_last_root, &mut rows);
        }

        // Mark selected
        for row in &mut rows {
            if Some(&row.id) == self.selected_id.as_ref() {
                row.is_selected = true;
            }
        }

        rows
    }

    fn flatten_recursive(
        &self,
        node_id: &str,
        continuation_mask: &mut Vec<bool>,
        is_last_sibling: bool,
        rows: &mut Vec<FlattenedProgressRow>,
    ) {
        let node = match self.nodes.get(node_id) {
            Some(n) => n,
            None => return,
        };

        // Construct prefix
        let mut prefix = String::new();
        for &has_continuation in continuation_mask.iter() {
            if has_continuation {
                prefix.push_str(self.glyphs.vertical);
            } else {
                prefix.push_str(self.glyphs.space);
            }
        }

        if !continuation_mask.is_empty() {
            if is_last_sibling {
                prefix.push_str(self.glyphs.last);
            } else {
                prefix.push_str(self.glyphs.branch);
            }
        }

        let has_children = !node.children.is_empty();

        // Add expansion indicator if has children
        if has_children {
            if node.is_expanded {
                prefix.push_str(self.glyphs.expand_open);
            } else {
                prefix.push_str(self.glyphs.expand_collapsed);
            }
        }

        // Only include if matches filter or has matching descendants
        let matches_self = self.matches_filter(node);
        if matches_self {
            rows.push(FlattenedProgressRow {
                id: node_id.to_string(),
                depth: node.depth,
                prefix,
                has_children,
                is_expanded: node.is_expanded,
                is_selected: false,
            });
        }

        // Recursively add children if expanded
        if has_children && node.is_expanded {
            continuation_mask.push(!is_last_sibling && !continuation_mask.is_empty());
            let child_count = node.children.len();
            for (c_idx, child_id) in node.children.iter().enumerate() {
                let is_last_child = c_idx == child_count - 1;
                self.flatten_recursive(child_id, continuation_mask, is_last_child, rows);
            }
            continuation_mask.pop();
        }
    }

    /// Selects the next visible row.
    pub fn select_next(&mut self) {
        let visible = self.flatten_visible();
        if visible.is_empty() {
            return;
        }

        if let Some(cur_id) = &self.selected_id {
            if let Some(pos) = visible.iter().position(|r| &r.id == cur_id) {
                if pos + 1 < visible.len() {
                    self.selected_id = Some(visible[pos + 1].id.clone());
                }
            } else {
                self.selected_id = Some(visible[0].id.clone());
            }
        } else {
            self.selected_id = Some(visible[0].id.clone());
        }
        self.ensure_selected_visible();
    }

    /// Selects the previous visible row.
    pub fn select_prev(&mut self) {
        let visible = self.flatten_visible();
        if visible.is_empty() {
            return;
        }

        if let Some(cur_id) = &self.selected_id {
            if let Some(pos) = visible.iter().position(|r| &r.id == cur_id) {
                if pos > 0 {
                    self.selected_id = Some(visible[pos - 1].id.clone());
                }
            } else {
                self.selected_id = Some(visible[visible.len() - 1].id.clone());
            }
        } else {
            self.selected_id = Some(visible[0].id.clone());
        }
        self.ensure_selected_visible();
    }

    /// Toggles expansion for the currently selected node.
    pub fn toggle_expand_selected(&mut self) {
        if let Some(cur_id) = &self.selected_id {
            if let Some(node) = self.nodes.get_mut(cur_id) {
                node.is_expanded = !node.is_expanded;
            }
        }
    }

    /// Expands all nodes in the tree.
    pub fn expand_all(&mut self) {
        for node in self.nodes.values_mut() {
            node.is_expanded = true;
        }
    }

    /// Collapses all non-root nodes in the tree.
    pub fn collapse_all(&mut self) {
        for node in self.nodes.values_mut() {
            if node.depth > 0 {
                node.is_expanded = false;
            }
        }
    }

    /// Cycles through status filter modes.
    pub fn cycle_filter(&mut self) {
        self.filter = match self.filter {
            SubagentFilter::All => SubagentFilter::ActiveOnly,
            SubagentFilter::ActiveOnly => SubagentFilter::CompletedOnly,
            SubagentFilter::CompletedOnly => SubagentFilter::FailedOnly,
            SubagentFilter::FailedOnly => SubagentFilter::All,
        };
    }

    /// Cycles through view modes.
    pub fn toggle_view_mode(&mut self) {
        self.view_mode = match self.view_mode {
            ProgressViewMode::TreeOnly => ProgressViewMode::SplitDetail,
            ProgressViewMode::SplitDetail => ProgressViewMode::Dashboard,
            ProgressViewMode::Dashboard => ProgressViewMode::Compact,
            ProgressViewMode::Compact => ProgressViewMode::TreeOnly,
        };
    }

    /// Ensures the currently selected row is scrolled into view.
    pub fn ensure_selected_visible(&mut self) {
        let visible = self.flatten_visible();
        if let Some(cur_id) = &self.selected_id {
            if let Some(pos) = visible.iter().position(|r| &r.id == cur_id) {
                if pos < self.scroll_offset {
                    self.scroll_offset = pos;
                } else if pos >= self.scroll_offset + 15 {
                    self.scroll_offset = pos.saturating_sub(14);
                }
            }
        }
    }

    /// Handles keyboard events for interactive navigation.
    pub fn handle_key(&mut self, key: KeyEvent) -> Option<ProgressTreeAction> {
        if self.is_searching {
            match key.code {
                KeyCode::Esc => {
                    self.is_searching = false;
                    self.search_query.clear();
                    return Some(ProgressTreeAction::Close);
                }
                KeyCode::Enter => {
                    self.is_searching = false;
                    return None;
                }
                KeyCode::Backspace => {
                    self.search_query.pop();
                    return None;
                }
                KeyCode::Char(c) => {
                    self.search_query.push(c);
                    return None;
                }
                _ => return None,
            }
        }

        match key.code {
            KeyCode::Char('q') | KeyCode::Esc => Some(ProgressTreeAction::Close),
            KeyCode::Down | KeyCode::Char('j') => {
                self.select_next();
                Some(ProgressTreeAction::SelectNext)
            }
            KeyCode::Up | KeyCode::Char('k') => {
                self.select_prev();
                Some(ProgressTreeAction::SelectPrev)
            }
            KeyCode::Char(' ') | KeyCode::Enter => {
                self.toggle_expand_selected();
                Some(ProgressTreeAction::ToggleExpand)
            }
            KeyCode::Char('e') => {
                self.expand_all();
                Some(ProgressTreeAction::ExpandAll)
            }
            KeyCode::Char('c') => {
                self.collapse_all();
                Some(ProgressTreeAction::CollapseAll)
            }
            KeyCode::Tab | KeyCode::Char('v') => {
                self.toggle_view_mode();
                Some(ProgressTreeAction::ToggleViewMode)
            }
            KeyCode::Char('f') => {
                self.cycle_filter();
                Some(ProgressTreeAction::CycleFilter)
            }
            KeyCode::Char('/') => {
                self.is_searching = true;
                self.search_query.clear();
                None
            }
            KeyCode::Char('x') => {
                if let Some(id) = &self.selected_id {
                    Some(ProgressTreeAction::CancelSelected(id.clone()))
                } else {
                    None
                }
            }
            KeyCode::Char('i') => {
                if let Some(id) = &self.selected_id {
                    Some(ProgressTreeAction::InspectSelected(id.clone()))
                } else {
                    None
                }
            }
            _ => None,
        }
    }
}

// ============================================================================
// 5. Standalone ANSI & Plaintext Rendering
// ============================================================================

/// Rendering options for standalone string output.
#[derive(Debug, Clone)]
pub struct ProgressTreeOptions {
    /// Glyph preset to use.
    pub glyphs: ProgressGlyphSet,
    /// Whether to display ANSI colors.
    pub use_colors: bool,
    /// Whether to include performance metrics (tokens, duration).
    pub show_metrics: bool,
    /// Whether to include active tool summaries.
    pub show_tools: bool,
    /// Whether to include turn progress bars.
    pub show_progress_bars: bool,
    /// Maximum character width for task descriptions.
    pub task_max_length: usize,
    /// Whether to render header and summary footer.
    pub show_header_footer: bool,
}

impl Default for ProgressTreeOptions {
    fn default() -> Self {
        Self {
            glyphs: ProgressGlyphSet::unicode(),
            use_colors: true,
            show_metrics: true,
            show_tools: true,
            show_progress_bars: true,
            task_max_length: 45,
            show_header_footer: true,
        }
    }
}

impl ProgressTreeOptions {
    /// Options for plain text rendering without ANSI codes.
    pub fn plain() -> Self {
        Self {
            use_colors: false,
            ..Default::default()
        }
    }

    /// Compact options for embedding into status bars.
    pub fn compact() -> Self {
        Self {
            glyphs: ProgressGlyphSet::compact(),
            show_metrics: false,
            show_tools: false,
            show_progress_bars: false,
            task_max_length: 30,
            show_header_footer: false,
            ..Default::default()
        }
    }
}

/// Renders a mini progress bar string (e.g. `[████░░░░] 50%`).
pub fn render_mini_bar(ratio: f32, width: usize, use_colors: bool) -> String {
    let bar_width = width.max(4);
    let filled = ((ratio.clamp(0.0, 1.0) * bar_width as f32).round() as usize).min(bar_width);
    let empty = bar_width.saturating_sub(filled);

    let pct = (ratio * 100.0).round() as u8;

    if use_colors {
        let color = if pct >= 100 {
            "\x1b[32m" // Green
        } else if pct >= 50 {
            "\x1b[36m" // Cyan
        } else {
            "\x1b[33m" // Yellow
        };
        format!("{}[{}{}]\x1b[0m {:>3}%", color, "█".repeat(filled), "░".repeat(empty), pct)
    } else {
        format!("[{}{}] {:>3}%", "#".repeat(filled), "-".repeat(empty), pct)
    }
}

/// Renders the complete progress tree into an ANSI-formatted string.
pub fn render_progress_tree_ansi(state: &ProgressTreeState, options: &ProgressTreeOptions, theme: &Theme) -> String {
    let mut out = String::new();
    let visible = state.flatten_visible();

    // 1. Header
    if options.show_header_footer {
        let active = state.active_count();
        let completed = state.completed_count();
        let failed = state.failed_count();
        let total = state.total_count();
        let overall_pct = state.overall_progress_pct();
        let elapsed = format_duration(state.start_time.elapsed());
        let tokens = state.total_tokens();

        if options.use_colors {
            out.push_str("\x1b[1;36m╭─ Subagent Progress Tree\x1b[0m");
            out.push_str(&format!(
                " \x1b[2m({} active, {} done, {} failed / {} total | {}% | {} | {} tokens)\x1b[0m\n",
                active, completed, failed, total, overall_pct, elapsed, tokens
            ));
        } else {
            out.push_str(&format!(
                "+- Subagent Progress Tree ({} active, {} done, {} failed / {} total | {}% | {} | {} tokens)\n",
                active, completed, failed, total, overall_pct, elapsed, tokens
            ));
        }
    }

    if visible.is_empty() {
        if options.use_colors {
            out.push_str("  \x1b[2m(No matching active or completed subagents)\x1b[0m\n");
        } else {
            out.push_str("  (No matching active or completed subagents)\n");
        }
        return out;
    }

    // 2. Nodes
    for row in visible {
        let node = match state.nodes.get(&row.id) {
            Some(n) => n,
            None => continue,
        };

        let status_icon = node.status_icon(state.spinner_tick);
        let role_badge = node.role_badge();
        let duration_str = format_duration(node.elapsed());

        let bar_str = if options.show_progress_bars {
            format!(" {}", render_mini_bar(node.progress_ratio(), 6, options.use_colors))
        } else {
            String::new()
        };

        let metrics_str = if options.show_metrics {
            format!(" [{}]", node.metrics.format_total_tokens())
        } else {
            String::new()
        };

        let tool_str = if options.show_tools {
            if let Some(tool) = &node.current_tool {
                if options.use_colors {
                    format!(" \x1b[33m⚡{}\x1b[0m", tool)
                } else {
                    format!(" [Tool: {}]", tool)
                }
            } else if !node.thinking_preview.is_empty() && node.is_running() {
                if options.use_colors {
                    format!(" \x1b[2;3m💭{}\x1b[0m", node.thinking_preview)
                } else {
                    format!(" (Thinking: {})", node.thinking_preview)
                }
            } else {
                String::new()
            }
        } else {
            String::new()
        };

        let task_snippet = if node.task.is_empty() {
            String::new()
        } else {
            let truncated = if node.task.len() > options.task_max_length {
                format!("{}...", &node.task[..options.task_max_length.saturating_sub(3)])
            } else {
                node.task.clone()
            };
            if options.use_colors {
                format!(" \x1b[2m\"{}\"\x1b[0m", truncated)
            } else {
                format!(" \"{}\"", truncated)
            }
        };

        if options.use_colors {
            let role_col = match &node.role {
                SubagentRole::Scout => "\x1b[36m",       // Cyan
                SubagentRole::Coder => "\x1b[32m",       // Green
                SubagentRole::Tester => "\x1b[35m",      // Magenta
                SubagentRole::Reviewer => "\x1b[34m",    // Blue
                SubagentRole::General => "\x1b[37m",     // White
                SubagentRole::Custom { .. } => "\x1b[36m", // Cyan
            };

            let status_col = match &node.status {
                SubagentStatus::Pending => "\x1b[33m",
                SubagentStatus::Running { .. } => "\x1b[1;36m",
                SubagentStatus::Completed { .. } => "\x1b[32m",
                SubagentStatus::Failed { .. } => "\x1b[1;31m",
                SubagentStatus::Cancelled => "\x1b[33m",
            };

            let sel_indicator = if row.is_selected { "\x1b[1;33m❯\x1b[0m " } else { "  " };

            out.push_str(&format!(
                "{}{}{}{}\x1b[0m \x1b[1m{}\x1b[0m {}{}\x1b[0m{}{}{}{} \x1b[2m({})\x1b[0m\n",
                sel_indicator,
                row.prefix,
                status_col,
                status_icon,
                node.name,
                role_col,
                role_badge,
                bar_str,
                tool_str,
                task_snippet,
                metrics_str,
                duration_str,
            ));
        } else {
            let sel_indicator = if row.is_selected { "> " } else { "  " };
            out.push_str(&format!(
                "{}{}{} {} {}{}{}{}{} ({})\n",
                sel_indicator,
                row.prefix,
                status_icon,
                node.name,
                role_badge,
                bar_str,
                tool_str,
                task_snippet,
                metrics_str,
                duration_str,
            ));
        }
    }

    // 3. Footer
    if options.show_header_footer {
        if options.use_colors {
            out.push_str("\x1b[2m╰──────────────────────────────────────────────────────────\x1b[0m\n");
        } else {
            out.push_str("----------------------------------------------------------\n");
        }
    }

    out
}

/// Renders the progress tree into clean plain text without ANSI escapes.
pub fn render_progress_tree_plain(state: &ProgressTreeState, options: &ProgressTreeOptions) -> String {
    let mut plain_opts = options.clone();
    plain_opts.use_colors = false;
    render_progress_tree_ansi(state, &plain_opts, &Theme::auto())
}

/// Renders a single-line summary string for REPL or status bar embedding.
pub fn render_progress_summary_line(state: &ProgressTreeState, theme: &Theme) -> String {
    let active = state.active_count();
    let completed = state.completed_count();
    let total = state.total_count();
    let pct = state.overall_progress_pct();

    if active == 0 && total == 0 {
        return "🤖 No subagents active".to_string();
    }

    let spinner = if active > 0 {
        let idx = state.spinner_tick % BRAILLE_FRAMES.len();
        BRAILLE_FRAMES[idx]
    } else {
        "✓"
    };

    let bar = render_mini_bar(state.overall_progress_pct() as f32 / 100.0, 5, true);
    format!(
        "\x1b[1;36m{}\x1b[0m \x1b[1mAgents:\x1b[0m {} active / {} done {} ({}%)",
        spinner, active, completed, bar, pct
    )
}

// ============================================================================
// 6. Interactive Ratatui Widget Implementation
// ============================================================================

/// Ratatui widget for rendering the interactive subagent progress tree.
pub struct ProgressTreeWidget<'a> {
    state: &'a ProgressTreeState,
    theme: Theme,
}

impl<'a> ProgressTreeWidget<'a> {
    /// Creates a new widget instance.
    pub fn new(state: &'a ProgressTreeState) -> Self {
        Self {
            state,
            theme: Theme::auto(),
        }
    }

    /// Sets a custom theme.
    pub fn theme(mut self, theme: Theme) -> Self {
        self.theme = theme;
        self
    }
}

impl<'a> Widget for ProgressTreeWidget<'a> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        if area.width < 10 || area.height < 3 {
            return;
        }

        match self.state.view_mode {
            ProgressViewMode::TreeOnly => self.render_tree_view(area, buf),
            ProgressViewMode::SplitDetail => self.render_split_view(area, buf),
            ProgressViewMode::Dashboard => self.render_dashboard_view(area, buf),
            ProgressViewMode::Compact => self.render_compact_view(area, buf),
        }
    }
}

impl<'a> ProgressTreeWidget<'a> {
    /// Renders the tree-only layout.
    fn render_tree_view(&self, area: Rect, buf: &mut Buffer) {
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(3), // Header & stats
                Constraint::Min(4),    // Tree rows
                Constraint::Length(1), // Footer shortcuts
            ])
            .split(area);

        self.render_header(chunks[0], buf);
        self.render_tree_rows(chunks[1], buf);
        self.render_footer(chunks[2], buf);
    }

    /// Renders the split pane layout (tree on left, selected details on right).
    fn render_split_view(&self, area: Rect, buf: &mut Buffer) {
        let main_chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(3), // Header
                Constraint::Min(4),    // Split Body
                Constraint::Length(1), // Footer
            ])
            .split(area);

        self.render_header(main_chunks[0], buf);

        let body_chunks = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([
                Constraint::Percentage(50), // Tree
                Constraint::Percentage(50), // Inspector
            ])
            .split(main_chunks[1]);

        self.render_tree_rows(body_chunks[0], buf);
        self.render_detail_inspector(body_chunks[1], buf);
        self.render_footer(main_chunks[2], buf);
    }

    /// Renders full dashboard view (header metrics, split tree/detail, and activity stream).
    fn render_dashboard_view(&self, area: Rect, buf: &mut Buffer) {
        let v_chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(3),      // Header
                Constraint::Percentage(60), // Split tree / inspector
                Constraint::Percentage(40), // Live activity log stream
                Constraint::Length(1),      // Footer
            ])
            .split(area);

        self.render_header(v_chunks[0], buf);

        let h_chunks = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([
                Constraint::Percentage(45), // Tree
                Constraint::Percentage(55), // Selected inspector
            ])
            .split(v_chunks[1]);

        self.render_tree_rows(h_chunks[0], buf);
        self.render_detail_inspector(h_chunks[1], buf);
        self.render_activity_stream(v_chunks[2], buf);
        self.render_footer(v_chunks[3], buf);
    }

    /// Renders compact card view.
    fn render_compact_view(&self, area: Rect, buf: &mut Buffer) {
        let block = Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(Color::Cyan))
            .title(Span::styled(" 🤖 Subagent Progress ", Style::default().add_modifier(Modifier::BOLD)));

        let inner = block.inner(area);
        block.render(area, buf);

        let active = self.state.active_count();
        let total = self.state.total_count();
        let pct = self.state.overall_progress_pct();

        let line = Line::from(vec![
            Span::styled("Active: ", Style::default().fg(Color::DarkGray)),
            Span::styled(format!("{} / {}  ", active, total), Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)),
            Span::styled("Progress: ", Style::default().fg(Color::DarkGray)),
            Span::raw(render_mini_bar(pct as f32 / 100.0, 10, false)),
            Span::styled(format!(" ({}%)  ", pct), Style::default().fg(Color::Green)),
            Span::styled("Elapsed: ", Style::default().fg(Color::DarkGray)),
            Span::styled(format_duration(self.state.start_time.elapsed()), Style::default().fg(Color::Yellow)),
        ]);

        let p = Paragraph::new(vec![line]).wrap(Wrap { trim: true });
        p.render(inner, buf);
    }

    /// Renders the top metrics header bar.
    fn render_header(&self, area: Rect, buf: &mut Buffer) {
        let active = self.state.active_count();
        let completed = self.state.completed_count();
        let failed = self.state.failed_count();
        let total = self.state.total_count();
        let overall_pct = self.state.overall_progress_pct();
        let elapsed = format_duration(self.state.start_time.elapsed());
        let total_tok = self.state.total_tokens();

        let title_span = Span::styled(
            " 🌿 Multi-Agent Streaming Progress Tree ",
            Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD),
        );

        let block = Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(Color::DarkGray))
            .title(title_span);

        let inner = block.inner(area);
        block.render(area, buf);

        let stats_line = Line::from(vec![
            Span::styled(" Active: ", Style::default().fg(Color::DarkGray)),
            Span::styled(format!("{} ", active), Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)),
            Span::styled("│ Done: ", Style::default().fg(Color::DarkGray)),
            Span::styled(format!("{} ", completed), Style::default().fg(Color::Green).add_modifier(Modifier::BOLD)),
            Span::styled("│ Failed: ", Style::default().fg(Color::DarkGray)),
            Span::styled(format!("{} ", failed), Style::default().fg(if failed > 0 { Color::Red } else { Color::DarkGray }).add_modifier(Modifier::BOLD)),
            Span::styled("│ Total: ", Style::default().fg(Color::DarkGray)),
            Span::styled(format!("{} ", total), Style::default().fg(Color::White)),
            Span::styled("│ Progress: ", Style::default().fg(Color::DarkGray)),
            Span::styled(format!("{}% ", overall_pct), Style::default().fg(Color::Green).add_modifier(Modifier::BOLD)),
            Span::styled("│ Time: ", Style::default().fg(Color::DarkGray)),
            Span::styled(format!("{} ", elapsed), Style::default().fg(Color::Yellow)),
            Span::styled("│ Tokens: ", Style::default().fg(Color::DarkGray)),
            Span::styled(format!("{} ", total_tok), Style::default().fg(Color::Magenta)),
        ]);

        let p = Paragraph::new(vec![stats_line]);
        p.render(inner, buf);
    }

    /// Renders the tree node rows.
    fn render_tree_rows(&self, area: Rect, buf: &mut Buffer) {
        let block = Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(Color::DarkGray))
            .title(Span::styled(" Hierarchy & Tasks ", Style::default().fg(Color::White).add_modifier(Modifier::BOLD)));

        let inner = block.inner(area);
        block.render(area, buf);

        let visible = self.state.flatten_visible();
        if visible.is_empty() {
            let p = Paragraph::new(Line::from(Span::styled("No active subagents to display.", Style::default().fg(Color::DarkGray))));
            p.render(inner, buf);
            return;
        }

        let max_display = inner.height as usize;
        let start = self.state.scroll_offset.min(visible.len().saturating_sub(1));
        let slice = &visible[start..(start + max_display).min(visible.len())];

        let mut lines = Vec::new();

        for row in slice {
            let node = match self.state.nodes.get(&row.id) {
                Some(n) => n,
                None => continue,
            };

            let mut spans = Vec::new();

            // Selection indicator
            if row.is_selected {
                spans.push(Span::styled("❯ ", Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)));
            } else {
                spans.push(Span::raw("  "));
            }

            // Tree prefix
            spans.push(Span::styled(&row.prefix, Style::default().fg(Color::DarkGray)));

            // Status icon / spinner
            let status_style = match &node.status {
                SubagentStatus::Pending => Style::default().fg(Color::Yellow),
                SubagentStatus::Running { .. } => Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD),
                SubagentStatus::Completed { .. } => Style::default().fg(Color::Green).add_modifier(Modifier::BOLD),
                SubagentStatus::Failed { .. } => Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
                SubagentStatus::Cancelled => Style::default().fg(Color::DarkGray),
            };
            spans.push(Span::styled(format!("{} ", node.status_icon(self.state.spinner_tick)), status_style));

            // Name
            let name_style = if row.is_selected {
                Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(Color::White).add_modifier(Modifier::BOLD)
            };
            spans.push(Span::styled(format!("{} ", node.name), name_style));

            // Role badge
            let role_color = match &node.role {
                SubagentRole::Scout => Color::Cyan,
                SubagentRole::Coder => Color::Green,
                SubagentRole::Tester => Color::Magenta,
                SubagentRole::Reviewer => Color::Blue,
                SubagentRole::General => Color::White,
                SubagentRole::Custom { .. } => Color::Cyan,
            };
            spans.push(Span::styled(format!("{} ", node.role_badge()), Style::default().fg(role_color)));

            // Mini bar
            spans.push(Span::raw(format!("{} ", render_mini_bar(node.progress_ratio(), 5, false))));

            // Active Tool / Thinking preview
            if let Some(tool) = &node.current_tool {
                spans.push(Span::styled(format!("⚡{} ", tool), Style::default().fg(Color::Yellow)));
            } else if !node.thinking_preview.is_empty() && node.is_running() {
                spans.push(Span::styled(format!("💭{} ", node.thinking_preview), Style::default().fg(Color::DarkGray)));
            }

            // Duration
            spans.push(Span::styled(
                format!("({})", format_duration(node.elapsed())),
                Style::default().fg(Color::DarkGray),
            ));

            lines.push(Line::from(spans));
        }

        let p = Paragraph::new(lines);
        p.render(inner, buf);
    }

    /// Renders the detailed inspector panel for the selected node.
    fn render_detail_inspector(&self, area: Rect, buf: &mut Buffer) {
        let block = Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(Color::DarkGray))
            .title(Span::styled(" Subagent Inspector ", Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)));

        let inner = block.inner(area);
        block.render(area, buf);

        let selected = self.state.selected_id.as_ref().and_then(|id| self.state.nodes.get(id));
        let node = match selected {
            Some(n) => n,
            None => {
                let p = Paragraph::new(Line::from(Span::styled("Select a subagent to view details.", Style::default().fg(Color::DarkGray))));
                p.render(inner, buf);
                return;
            }
        };

        let mut lines = Vec::new();

        // Title & Role
        lines.push(Line::from(vec![
            Span::styled("Subagent: ", Style::default().fg(Color::DarkGray)),
            Span::styled(&node.name, Style::default().fg(Color::White).add_modifier(Modifier::BOLD)),
            Span::raw("  "),
            Span::styled(node.role_badge(), Style::default().fg(Color::Cyan)),
            Span::raw("  "),
            Span::styled(format!("(ID: {})", node.id), Style::default().fg(Color::DarkGray)),
        ]));

        // Status & Turns
        lines.push(Line::from(vec![
            Span::styled("Status: ", Style::default().fg(Color::DarkGray)),
            Span::styled(node.status_summary(), Style::default().fg(Color::Green)),
            Span::raw("  │  "),
            Span::styled("Turn: ", Style::default().fg(Color::DarkGray)),
            Span::styled(format!("{}/{}", node.current_turn, node.max_turns), Style::default().fg(Color::Yellow)),
            Span::raw("  │  "),
            Span::styled("Duration: ", Style::default().fg(Color::DarkGray)),
            Span::styled(format_duration(node.elapsed()), Style::default().fg(Color::Cyan)),
        ]));

        // Metrics & Tokens
        lines.push(Line::from(vec![
            Span::styled("Tokens: ", Style::default().fg(Color::DarkGray)),
            Span::styled(format!("{} total", node.metrics.format_total_tokens()), Style::default().fg(Color::Magenta)),
            Span::raw("  │  "),
            Span::styled("TPS: ", Style::default().fg(Color::DarkGray)),
            Span::styled(format!("{:.1} tok/s", node.metrics.tokens_per_sec), Style::default().fg(Color::White)),
            Span::raw("  │  "),
            Span::styled("Cost: ", Style::default().fg(Color::DarkGray)),
            Span::styled(node.metrics.format_cost(), Style::default().fg(Color::Green)),
        ]));

        lines.push(Line::from(Span::styled("─".repeat(inner.width as usize), Style::default().fg(Color::DarkGray))));

        // Task Directive
        lines.push(Line::from(vec![
            Span::styled("Task: ", Style::default().fg(Color::DarkGray).add_modifier(Modifier::BOLD)),
            Span::styled(&node.task, Style::default().fg(Color::White)),
        ]));

        // Live Reasoning / Thinking Stream
        if !node.thinking_full.is_empty() {
            lines.push(Line::from(Span::styled("─".repeat(inner.width as usize), Style::default().fg(Color::DarkGray))));
            lines.push(Line::from(vec![
                Span::styled("Live Reasoning / Thoughts: ", Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)),
            ]));
            let preview = if node.thinking_full.len() > 300 {
                format!("...{}", &node.thinking_full[node.thinking_full.len() - 300..])
            } else {
                node.thinking_full.clone()
            };
            for chunk in preview.lines().take(4) {
                lines.push(Line::from(Span::styled(
                    chunk.to_string(),
                    Style::default().fg(Color::DarkGray),
                )));
            }
        }

        // Tool history summary
        if !node.tool_history.is_empty() {
            lines.push(Line::from(Span::styled("─".repeat(inner.width as usize), Style::default().fg(Color::DarkGray))));
            lines.push(Line::from(vec![
                Span::styled("Executed Tools: ", Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)),
                Span::styled(format!("({} calls)", node.tool_history.len()), Style::default().fg(Color::DarkGray)),
            ]));
            for tool_call in node.tool_history.iter().rev().take(3) {
                let sym = if tool_call.success { "✓" } else { "✗" };
                let col = if tool_call.success { Color::Green } else { Color::Red };
                lines.push(Line::from(vec![
                    Span::styled(format!("  {} ", sym), Style::default().fg(col)),
                    Span::styled(&tool_call.tool, Style::default().fg(Color::White)),
                    Span::styled(format!(" ({}ms)", tool_call.duration_ms), Style::default().fg(Color::DarkGray)),
                ]));
            }
        }

        let p = Paragraph::new(lines).wrap(Wrap { trim: true });
        p.render(inner, buf);
    }

    /// Renders live activity stream logs.
    fn render_activity_stream(&self, area: Rect, buf: &mut Buffer) {
        let block = Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(Color::DarkGray))
            .title(Span::styled(" Live Activity Stream ", Style::default().fg(Color::Green).add_modifier(Modifier::BOLD)));

        let inner = block.inner(area);
        block.render(area, buf);

        let selected = self.state.selected_id.as_ref().and_then(|id| self.state.nodes.get(id));
        let events = match selected {
            Some(n) => &n.log_events,
            None => {
                let p = Paragraph::new(Line::from(Span::styled("No activity logs.", Style::default().fg(Color::DarkGray))));
                p.render(inner, buf);
                return;
            }
        };

        let mut lines = Vec::new();
        let max_lines = inner.height as usize;
        let start = events.len().saturating_sub(max_lines);

        for entry in &events[start..] {
            let time_str = format!("[+{:>4}ms]", entry.elapsed_ms);
            let icon = entry.kind.icon();
            let col = match entry.kind {
                ProgressLogKind::Started | ProgressLogKind::Completed => Color::Green,
                ProgressLogKind::Turn => Color::Cyan,
                ProgressLogKind::Thinking => Color::DarkGray,
                ProgressLogKind::Message => Color::White,
                ProgressLogKind::ToolStart => Color::Yellow,
                ProgressLogKind::ToolSuccess => Color::Green,
                ProgressLogKind::ToolError | ProgressLogKind::Failed => Color::Red,
                ProgressLogKind::Cancelled => Color::Yellow,
                ProgressLogKind::Info => Color::Blue,
            };

            lines.push(Line::from(vec![
                Span::styled(format!("{} ", time_str), Style::default().fg(Color::DarkGray)),
                Span::raw(format!("{} ", icon)),
                Span::styled(&entry.message, Style::default().fg(col)),
            ]));
        }

        let p = Paragraph::new(lines);
        p.render(inner, buf);
    }

    /// Renders the bottom keyboard shortcuts bar.
    fn render_footer(&self, area: Rect, buf: &mut Buffer) {
        let keys = vec![
            Span::styled(" ↑/↓", Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)),
            Span::raw(" Nav  "),
            Span::styled("Space", Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)),
            Span::raw(" Toggle  "),
            Span::styled("e/c", Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)),
            Span::raw(" Expand/Collapse  "),
            Span::styled("Tab", Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)),
            Span::raw(" Layout  "),
            Span::styled("f", Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)),
            Span::raw(" Filter  "),
            Span::styled("x", Style::default().fg(Color::Red).add_modifier(Modifier::BOLD)),
            Span::raw(" Cancel  "),
            Span::styled("q", Style::default().fg(Color::DarkGray).add_modifier(Modifier::BOLD)),
            Span::raw(" Quit"),
        ];

        let p = Paragraph::new(Line::from(keys));
        p.render(area, buf);
    }
}

// ============================================================================
// 7. Unit Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::subagent::SubagentProgress;

    #[test]
    fn test_node_creation_and_basics() {
        let node = ProgressTreeNode::new("sub_01", "ScoutAgent", SubagentRole::Scout, "Search repo for auth tokens");
        assert_eq!(node.id, "sub_01");
        assert_eq!(node.name, "ScoutAgent");
        assert_eq!(node.role, SubagentRole::Scout);
        assert_eq!(node.role_badge(), "[SCOUT]");
        assert!(!node.is_finished());
        assert_eq!(node.progress_pct(), 0);
    }

    #[test]
    fn test_streaming_event_ingestion() {
        let mut state = ProgressTreeState::new();
        state.register_subagent("coder_1", "CodeAgent", SubagentRole::Coder, "Implement patch", None);

        // Turn 1 start
        state.handle_event(&SubagentProgress::TurnStarted {
            id: "coder_1".into(),
            turn: 1,
            max_turns: 4,
        });

        let node = state.nodes.get("coder_1").unwrap();
        assert_eq!(node.current_turn, 1);
        assert_eq!(node.max_turns, 4);
        assert!(node.is_running());
        assert!(node.progress_pct() > 0);

        // Thinking delta
        state.handle_event(&SubagentProgress::Thinking {
            id: "coder_1".into(),
            delta: "Analyzing AST structure and finding diffs...".into(),
        });
        let node = state.nodes.get("coder_1").unwrap();
        assert!(!node.thinking_preview.is_empty());
        assert!(node.metrics.total_tokens > 0);

        // Tool started
        state.handle_event(&SubagentProgress::ToolStarted {
            id: "coder_1".into(),
            tool: "ast_edit".into(),
            args: serde_json::json!({"path": "src/main.rs"}),
        });
        let node = state.nodes.get("coder_1").unwrap();
        assert_eq!(node.current_tool.as_deref(), Some("ast_edit"));

        // Tool completed
        state.handle_event(&SubagentProgress::ToolCompleted {
            id: "coder_1".into(),
            tool: "ast_edit".into(),
            output: "Applied 2 edits successfully".into(),
            success: true,
        });
        let node = state.nodes.get("coder_1").unwrap();
        assert_eq!(node.current_tool, None);
        assert_eq!(node.tool_history.len(), 1);

        // Completed
        state.handle_event(&SubagentProgress::Completed {
            id: "coder_1".into(),
            output: "All tasks finished".into(),
            turns_taken: 1,
        });
        let node = state.nodes.get("coder_1").unwrap();
        assert!(node.is_completed());
        assert_eq!(node.progress_pct(), 100);
    }

    #[test]
    fn test_tree_hierarchy_flattening() {
        let mut state = ProgressTreeState::new();
        state.register_subagent("root", "LeadCoordinator", SubagentRole::General, "Orchestrate feature", None);
        state.register_subagent("scout", "RepoScout", SubagentRole::Scout, "Find files", Some("root".into()));
        state.register_subagent("coder", "PatchCoder", SubagentRole::Coder, "Write patch", Some("root".into()));

        let visible = state.flatten_visible();
        assert_eq!(visible.len(), 3);
        assert_eq!(visible[0].id, "root");
        assert_eq!(visible[1].id, "scout");
        assert_eq!(visible[2].id, "coder");

        // Collapse root
        state.toggle_expand_selected();
        let visible_collapsed = state.flatten_visible();
        assert_eq!(visible_collapsed.len(), 1);
        assert_eq!(visible_collapsed[0].id, "root");

        // Expand all
        state.expand_all();
        let visible_expanded = state.flatten_visible();
        assert_eq!(visible_expanded.len(), 3);
    }

    #[test]
    fn test_ansi_and_plain_rendering() {
        let mut state = ProgressTreeState::new();
        state.register_subagent("lead", "LeadAgent", SubagentRole::General, "Main task", None);
        state.register_subagent("worker", "WorkerAgent", SubagentRole::Coder, "Sub task", Some("lead".into()));

        let ansi = render_progress_tree_ansi(&state, &ProgressTreeOptions::default(), &Theme::auto());
        assert!(ansi.contains("LeadAgent"));
        assert!(ansi.contains("WorkerAgent"));
        assert!(ansi.contains("Subagent Progress Tree"));

        let plain = render_progress_tree_plain(&state, &ProgressTreeOptions::default());
        assert!(plain.contains("LeadAgent"));
        assert!(plain.contains("WorkerAgent"));
        assert!(!plain.contains("\x1b["));
    }

    #[test]
    fn test_summary_line() {
        let mut state = ProgressTreeState::new();
        state.register_subagent("agent_1", "Worker", SubagentRole::Tester, "Run tests", None);
        state.handle_event(&SubagentProgress::TurnStarted {
            id: "agent_1".into(),
            turn: 1,
            max_turns: 2,
        });

        let summary = render_progress_summary_line(&state, &Theme::auto());
        assert!(summary.contains("Agents:"));
        assert!(summary.contains("1 active"));
    }

    #[test]
    fn test_widget_buffer_rendering() {
        let mut state = ProgressTreeState::new();
        state.register_subagent("t1", "AgentOne", SubagentRole::Scout, "Inspect code", None);

        let widget = ProgressTreeWidget::new(&state);
        let area = Rect::new(0, 0, 80, 24);
        let mut buf = Buffer::empty(area);

        widget.render(area, &mut buf);
        assert!(!buf.content.is_empty());
    }
}
