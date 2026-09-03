//! High-density session statistics card widget for Fusion.
//!
//! Provides a comprehensive session performance telemetry and metrics summary card:
//! - **Turn Count**: Total conversational turns, user messages, assistant responses,
//!   and average tokens/cost/duration per turn.
//! - **Total Tokens**: Aggregate token consumption with detailed breakdown for
//!   prompt tokens, completion tokens, cache read/write tokens, and cache hit rates.
//! - **Financial Cost**: Total estimated USD cost, uncached input cost, generated
//!   output cost, context cache write/read costs, and savings from caching.
//! - **Session Duration**: Elapsed conversation lifetime, active execution time,
//!   formatted timestamps, and per-turn latency averages.
//! - **Tool Execution Statistics**: Total tool invocations, unique tools used,
//!   per-tool call frequency counts, success/failure rates, and visual usage distribution meters.
//!
//! Supports multi-format rendering:
//! - **Ratatui Widget**: [`SessionStatsCardWidget`] implementing [`ratatui::widgets::Widget`]
//!   and [`ratatui::widgets::StatefulWidget`] for inline TUI and full-screen viewports.
//! - **ANSI Terminal String**: [`render_stats_card_ansi`] for pure Rust, zero-dependency ANSI box rendering.
//! - **Plain Text**: [`render_stats_card_plain`] for clean ASCII output on monochrome or log outputs.
//! - **Markdown**: [`render_stats_card_markdown`] for export to markdown reports and docs.
//! - **Compact Line**: [`render_stats_compact_ansi`] for status bars and single-line headers.

use std::collections::HashMap;
use std::time::Duration;

use chrono::DateTime;
use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, StatefulWidget, Widget},
};
use serde::{Deserialize, Serialize};

use crate::agent::cost::{format_usd, CostBreakdown};
use crate::agent::session::{Session, TokenStats};
use crate::agent::tokens::format_token_count;
use crate::provider::types::{Message, Role};
use crate::ui::table::{strip_ansi, visible_width};
use crate::ui::theme::Theme;
// ============================================================================
// 1. Constants & Unicode Glyphs
// ============================================================================

/// Default card width in terminal columns when not constrained.
pub const DEFAULT_CARD_WIDTH: usize = 68;

/// Minimum safe card width in columns.
pub const MIN_CARD_WIDTH: usize = 42;

/// Default maximum number of individual tool rows displayed before summarizing remainder.
pub const DEFAULT_MAX_TOOLS_DISPLAYED: usize = 8;

// Progress meter block characters
const METER_FULL: &str = "█";
const METER_SEVEN_EIGHTHS: &str = "▉";
const METER_THREE_QUARTERS: &str = "▊";
const METER_FIVE_EIGHTHS: &str = "▋";
const METER_HALF: &str = "▌";
const METER_THREE_EIGHTHS: &str = "▍";
const METER_ONE_QUARTER: &str = "▎";
const METER_ONE_EIGHTH: &str = "▏";
const METER_EMPTY: &str = "░";

// ANSI escape sequences
const ANSI_RESET: &str = "\x1b[0m";
const ANSI_BOLD: &str = "\x1b[1m";
const ANSI_DIM: &str = "\x1b[2m";
const ANSI_ITALIC: &str = "\x1b[3m";
const ANSI_CYAN: &str = "\x1b[36m";
const ANSI_BOLD_CYAN: &str = "\x1b[1;36m";
const ANSI_GREEN: &str = "\x1b[32m";
const ANSI_BOLD_GREEN: &str = "\x1b[1;32m";
const ANSI_YELLOW: &str = "\x1b[33m";
const ANSI_BOLD_YELLOW: &str = "\x1b[1;33m";
const ANSI_BLUE: &str = "\x1b[34m";
const ANSI_BOLD_BLUE: &str = "\x1b[1;34m";
const ANSI_MAGENTA: &str = "\x1b[35m";
const ANSI_BOLD_MAGENTA: &str = "\x1b[1;35m";
const ANSI_RED: &str = "\x1b[31m";
const ANSI_BOLD_RED: &str = "\x1b[1;31m";
const ANSI_GRAY: &str = "\x1b[90m";
const ANSI_WHITE: &str = "\x1b[37m";
const ANSI_BOLD_WHITE: &str = "\x1b[1;37m";

// ============================================================================
// 2. Data Models & Statistics Aggregation
// ============================================================================

/// Execution statistics for an individual tool across a session.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolExecutionStat {
    /// Name of the tool (e.g. `bash`, `read`, `edit`, `grep`, `glob`).
    pub name: String,
    /// Total number of invocations in the session.
    pub call_count: usize,
    /// Number of successful invocations.
    pub success_count: usize,
    /// Number of failed or error invocations.
    pub failure_count: usize,
    /// Cumulative execution duration if recorded.
    pub total_duration: Option<Duration>,
}

impl ToolExecutionStat {
    /// Creates a new tool execution record.
    pub fn new(name: impl Into<String>, call_count: usize) -> Self {
        Self {
            name: name.into(),
            call_count,
            success_count: call_count,
            failure_count: 0,
            total_duration: None,
        }
    }

    /// Success rate as a percentage between 0.0 and 100.0%.
    pub fn success_rate(&self) -> f64 {
        if self.call_count == 0 {
            100.0
        } else {
            (self.success_count as f64 / self.call_count as f64) * 100.0
        }
    }

    /// Returns `true` if all invocations succeeded with zero failures.
    pub fn is_perfect(&self) -> bool {
        self.failure_count == 0
    }

    /// Average duration per invocation if duration tracking was active.
    pub fn avg_duration(&self) -> Option<Duration> {
        self.total_duration.and_then(|dur| {
            if self.call_count > 0 {
                Some(dur / (self.call_count as u32))
            } else {
                None
            }
        })
    }
}

/// Comprehensive telemetry and metrics summary for a conversational session.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SessionStats {
    /// Unique session identifier (e.g. UUID string).
    pub session_id: Option<String>,
    /// Optional human-readable session title or goal.
    pub session_title: Option<String>,
    /// Active LLM model identifier (e.g. `claude-3-7-sonnet`, `deepseek-chat`).
    pub model_name: String,
    /// Total number of conversation turns.
    pub turn_count: usize,
    /// Total number of messages stored in the session buffer.
    pub message_count: usize,
    /// Accumulated token usage statistics.
    pub token_stats: TokenStats,
    /// Estimated financial cost breakdown.
    pub cost_breakdown: CostBreakdown,
    /// Total session duration or execution lifetime.
    pub duration: Duration,
    /// Timestamp when session was initiated (RFC 3339).
    pub started_at: Option<String>,
    /// Timestamp when session was last active (RFC 3339).
    pub updated_at: Option<String>,
    /// Aggregated tool execution statistics sorted descending by call count.
    pub tool_stats: Vec<ToolExecutionStat>,
    /// Total number of tool calls executed across all tools.
    pub total_tool_calls: usize,
    /// Whether the session is currently active or finalized.
    pub is_active: bool,
    /// Optional metadata tags.
    pub tags: Vec<String>,
}

impl Default for SessionStats {
    fn default() -> Self {
        Self::new()
    }
}

impl SessionStats {
    /// Creates a new empty `SessionStats` with default values.
    pub fn new() -> Self {
        Self {
            session_id: None,
            session_title: None,
            model_name: "default".to_string(),
            turn_count: 0,
            message_count: 0,
            token_stats: TokenStats::new(),
            cost_breakdown: CostBreakdown::zero(),
            duration: Duration::ZERO,
            started_at: None,
            updated_at: None,
            tool_stats: Vec::new(),
            total_tool_calls: 0,
            is_active: false,
            tags: Vec::new(),
        }
    }

    /// Extracts and aggregates complete statistics from a [`Session`].
    pub fn from_session(session: &Session) -> Self {
        let duration = compute_session_duration(session);
        let cost = crate::agent::cost::estimate_session_cost(session, None);
        Self::from_session_with_cost(session, cost, duration)
    }

    /// Extracts statistics from a [`Session`] with explicit duration.
    pub fn from_session_with_duration(session: &Session, duration: Duration) -> Self {
        let cost = crate::agent::cost::estimate_session_cost(session, None);
        Self::from_session_with_cost(session, cost, duration)
    }

    /// Extracts statistics from a [`Session`] with explicit cost breakdown and duration.
    pub fn from_session_with_cost(
        session: &Session,
        cost_breakdown: CostBreakdown,
        duration: Duration,
    ) -> Self {
        let (tool_stats, total_tool_calls) = extract_tool_stats(&session.messages);

        // Compute turn count: use token_stats.total_turns if > 0, otherwise count User messages
        let turn_count = if session.token_stats.total_turns > 0 {
            session.token_stats.total_turns as usize
        } else {
            let user_msgs = session
                .messages
                .iter()
                .filter(|m| m.role == Role::User)
                .count();
            if user_msgs > 0 {
                user_msgs
            } else if !session.messages.is_empty() {
                // Fallback heuristic: 1 turn for non-empty sessions
                1
            } else {
                0
            }
        };

        Self {
            session_id: Some(session.id.to_string()),
            session_title: session.title.clone(),
            model_name: session.active_model.clone(),
            turn_count,
            message_count: session.messages.len(),
            token_stats: session.token_stats,
            cost_breakdown,
            duration,
            started_at: Some(session.created_at.clone()),
            updated_at: Some(session.updated_at.clone()),
            tool_stats,
            total_tool_calls,
            is_active: false,
            tags: Vec::new(),
        }
    }

    /// Creates a builder for custom programmatic construction.
    pub fn builder() -> SessionStatsBuilder {
        SessionStatsBuilder::default()
    }

    // --- Metric Accessors & Computations ---

    /// Total tokens consumed (prompt + completion).
    pub fn total_tokens(&self) -> u64 {
        self.token_stats.total_tokens
    }

    /// Prompt / input tokens sent to the LLM.
    pub fn prompt_tokens(&self) -> u64 {
        self.token_stats.prompt_tokens
    }

    /// Completion / output tokens generated by the LLM.
    pub fn completion_tokens(&self) -> u64 {
        self.token_stats.completion_tokens
    }

    /// Cached tokens read from provider context cache.
    pub fn cache_read_tokens(&self) -> u64 {
        self.token_stats.cache_read_tokens
    }

    /// Cached tokens written to provider context cache.
    pub fn cache_write_tokens(&self) -> u64 {
        self.token_stats.cache_write_tokens
    }

    /// Context cache hit ratio (0.0% to 100.0%).
    pub fn cache_hit_ratio(&self) -> f64 {
        let total_prompt = self.prompt_tokens() + self.cache_read_tokens();
        if total_prompt > 0 {
            (self.cache_read_tokens() as f64 / total_prompt as f64) * 100.0
        } else {
            0.0
        }
    }

    /// Total accumulated cost in USD.
    pub fn total_cost(&self) -> f64 {
        self.cost_breakdown.total_cost
    }

    /// Number of distinct tool types executed.
    pub fn unique_tools_count(&self) -> usize {
        self.tool_stats.len()
    }

    /// Formats total tokens as a compact human-readable string (e.g. `14.2k`, `1.5M`).
    pub fn format_tokens(&self) -> String {
        format_token_count(self.total_tokens() as usize)
    }

    /// Formats total cost as standard USD string (e.g. `$0.0421`, `$1.25`).
    pub fn format_cost(&self) -> String {
        self.cost_breakdown.format_usd()
    }

    /// Formats session duration into clean, human-readable format.
    pub fn format_duration(&self) -> String {
        format_duration_pretty(self.duration)
    }

    /// Average tokens consumed per conversation turn.
    pub fn avg_tokens_per_turn(&self) -> u64 {
        if self.turn_count > 0 {
            self.total_tokens() / (self.turn_count as u64)
        } else {
            0
        }
    }

    /// Average USD cost incurred per conversation turn.
    pub fn avg_cost_per_turn(&self) -> f64 {
        if self.turn_count > 0 {
            self.total_cost() / (self.turn_count as f64)
        } else {
            0.0
        }
    }

    /// Average duration per conversation turn.
    pub fn avg_duration_per_turn(&self) -> Duration {
        if self.turn_count > 0 {
            self.duration / (self.turn_count as u32)
        } else {
            Duration::ZERO
        }
    }

    /// Average number of tool executions per turn.
    pub fn avg_tools_per_turn(&self) -> f64 {
        if self.turn_count > 0 {
            self.total_tool_calls as f64 / self.turn_count as f64
        } else {
            0.0
        }
    }

    /// Slices top tools up to `limit`.
    pub fn top_tools(&self, limit: usize) -> &[ToolExecutionStat] {
        let count = self.tool_stats.len().min(limit);
        &self.tool_stats[..count]
    }

    /// Tool call percentage distribution `(tool_name, call_count, percentage)`.
    pub fn tool_distribution(&self) -> Vec<(&str, usize, f64)> {
        if self.total_tool_calls == 0 {
            return Vec::new();
        }
        self.tool_stats
            .iter()
            .map(|t| {
                let pct = (t.call_count as f64 / self.total_tool_calls as f64) * 100.0;
                (t.name.as_str(), t.call_count, pct)
            })
            .collect()
    }
}

/// Builder pattern for fluent [`SessionStats`] construction.
#[derive(Debug, Default, Clone)]
pub struct SessionStatsBuilder {
    stats: SessionStats,
}

impl SessionStatsBuilder {
    /// Sets the session ID.
    pub fn session_id(mut self, id: impl Into<String>) -> Self {
        self.stats.session_id = Some(id.into());
        self
    }

    /// Sets the session title.
    pub fn session_title(mut self, title: impl Into<String>) -> Self {
        self.stats.session_title = Some(title.into());
        self
    }

    /// Sets the model name.
    pub fn model_name(mut self, model: impl Into<String>) -> Self {
        self.stats.model_name = model.into();
        self
    }

    /// Sets the turn count.
    pub fn turn_count(mut self, turns: usize) -> Self {
        self.stats.turn_count = turns;
        self
    }

    /// Sets message count.
    pub fn message_count(mut self, count: usize) -> Self {
        self.stats.message_count = count;
        self
    }

    /// Sets token stats.
    pub fn token_stats(mut self, token_stats: TokenStats) -> Self {
        self.stats.token_stats = token_stats;
        self
    }

    /// Sets tokens with prompt and completion counts.
    pub fn tokens(mut self, prompt: u64, completion: u64) -> Self {
        self.stats.token_stats.prompt_tokens = prompt;
        self.stats.token_stats.completion_tokens = completion;
        self.stats.token_stats.total_tokens = prompt + completion;
        self
    }

    /// Sets cached token counts.
    pub fn cache_tokens(mut self, read: u64, write: u64) -> Self {
        self.stats.token_stats.cache_read_tokens = read;
        self.stats.token_stats.cache_write_tokens = write;
        self
    }

    /// Sets cost breakdown.
    pub fn cost_breakdown(mut self, cost: CostBreakdown) -> Self {
        self.stats.cost_breakdown = cost;
        self
    }

    /// Sets total cost in USD directly.
    pub fn total_cost(mut self, total_cost: f64) -> Self {
        self.stats.cost_breakdown.total_cost = total_cost;
        self
    }

    /// Sets duration.
    pub fn duration(mut self, duration: Duration) -> Self {
        self.stats.duration = duration;
        self
    }

    /// Adds a tool execution record.
    pub fn add_tool(mut self, name: impl Into<String>, call_count: usize) -> Self {
        let stat = ToolExecutionStat::new(name, call_count);
        self.stats.total_tool_calls += call_count;
        self.stats.tool_stats.push(stat);
        self.stats
            .tool_stats
            .sort_by(|a, b| b.call_count.cmp(&a.call_count));
        self
    }

    /// Adds a detailed tool execution record.
    pub fn add_tool_stat(mut self, stat: ToolExecutionStat) -> Self {
        self.stats.total_tool_calls += stat.call_count;
        self.stats.tool_stats.push(stat);
        self.stats
            .tool_stats
            .sort_by(|a, b| b.call_count.cmp(&a.call_count));
        self
    }

    /// Sets whether the session is currently active.
    pub fn is_active(mut self, active: bool) -> Self {
        self.stats.is_active = active;
        self
    }

    /// Builds the configured [`SessionStats`].
    pub fn build(self) -> SessionStats {
        self.stats
    }
}

// ============================================================================
// 3. Extraction & Formatting Helpers
// ============================================================================

/// Extracts tool usage counts from conversation messages.
fn extract_tool_stats(messages: &[Message]) -> (Vec<ToolExecutionStat>, usize) {
    let mut call_counts: HashMap<String, usize> = HashMap::new();
    let mut success_counts: HashMap<String, usize> = HashMap::new();
    let mut failure_counts: HashMap<String, usize> = HashMap::new();
    let mut total_calls = 0;

    // Track tool_call_id -> tool_name mapping to correlate Tool responses with call names
    let mut id_to_tool: HashMap<String, String> = HashMap::new();

    for msg in messages {
        // Assistant messages with tool calls
        if let Some(calls) = &msg.tool_calls {
            for call in calls {
                let name = if call.name.is_empty() {
                    "tool".to_string()
                } else {
                    call.name.clone()
                };

                *call_counts.entry(name.clone()).or_insert(0) += 1;
                total_calls += 1;

                if !call.id.is_empty() {
                    id_to_tool.insert(call.id.clone(), name);
                }
            }
        }

        // Tool result messages (check for success/failure)
        if msg.role == Role::Tool {
            if let Some(call_id) = &msg.tool_call_id {
                if let Some(tool_name) = id_to_tool.get(call_id) {
                    let is_error = msg.content.contains("Error:")
                        || msg.content.contains("FAILED")
                        || msg.content.contains("exit code 1")
                        || msg.content.contains("command not found");

                    if is_error {
                        *failure_counts.entry(tool_name.clone()).or_insert(0) += 1;
                    } else {
                        *success_counts.entry(tool_name.clone()).or_insert(0) += 1;
                    }
                }
            }
        }
    }

    let mut stats: Vec<ToolExecutionStat> = call_counts
        .into_iter()
        .map(|(name, call_count)| {
            let failures = *failure_counts.get(&name).unwrap_or(&0);
            let successes = *success_counts
                .get(&name)
                .unwrap_or(&call_count.saturating_sub(failures));
            ToolExecutionStat {
                name,
                call_count,
                success_count: successes.min(call_count),
                failure_count: failures.min(call_count),
                total_duration: None,
            }
        })
        .collect();

    // Sort descending by call count, then alphabetically
    stats.sort_by(|a, b| {
        b.call_count
            .cmp(&a.call_count)
            .then_with(|| a.name.cmp(&b.name))
    });

    (stats, total_calls)
}

/// Computes session duration by comparing `created_at` and `updated_at`.
fn compute_session_duration(session: &Session) -> Duration {
    if let (Ok(start), Ok(end)) = (
        DateTime::parse_from_rfc3339(&session.created_at),
        DateTime::parse_from_rfc3339(&session.updated_at),
    ) {
        let diff = end.signed_duration_since(start);
        if diff.num_milliseconds() > 0 {
            return Duration::from_millis(diff.num_milliseconds() as u64);
        }
    }
    Duration::ZERO
}

/// Formats a duration into standard human-friendly format (e.g. `350ms`, `45s`, `2m 14s`, `1h 05m`).
pub fn format_duration_pretty(d: Duration) -> String {
    let total_secs = d.as_secs();
    let millis = d.subsec_millis();

    if total_secs == 0 {
        if millis == 0 {
            "0s".to_string()
        } else {
            format!("{}ms", millis)
        }
    } else if total_secs < 60 {
        if millis > 0 && total_secs < 5 {
            format!("{}.{:01}s", total_secs, millis / 100)
        } else {
            format!("{}s", total_secs)
        }
    } else if total_secs < 3600 {
        let mins = total_secs / 60;
        let secs = total_secs % 60;
        if secs == 0 {
            format!("{}m", mins)
        } else {
            format!("{}m {:02}s", mins, secs)
        }
    } else {
        let hours = total_secs / 3600;
        let mins = (total_secs % 3600) / 60;
        let secs = total_secs % 60;
        if mins == 0 && secs == 0 {
            format!("{}h", hours)
        } else if secs == 0 {
            format!("{}h {:02}m", hours, mins)
        } else {
            format!("{}h {:02}m {:02}s", hours, mins, secs)
        }
    }
}

/// Builds a horizontal Unicode meter bar representing a fraction between 0.0 and 1.0.
pub fn render_meter_bar(ratio: f64, width: usize) -> String {
    if width == 0 {
        return String::new();
    }

    let clamped = ratio.clamp(0.0, 1.0);
    let full_units = (clamped * width as f64).floor() as usize;
    let remainder = (clamped * width as f64) - full_units as f64;

    let mut bar = String::with_capacity(width * 4);

    for _ in 0..full_units {
        bar.push_str(METER_FULL);
    }

    if full_units < width {
        let partial = if remainder >= 0.875 {
            METER_SEVEN_EIGHTHS
        } else if remainder >= 0.75 {
            METER_THREE_QUARTERS
        } else if remainder >= 0.625 {
            METER_FIVE_EIGHTHS
        } else if remainder >= 0.5 {
            METER_HALF
        } else if remainder >= 0.375 {
            METER_THREE_EIGHTHS
        } else if remainder >= 0.25 {
            METER_ONE_QUARTER
        } else if remainder >= 0.125 {
            METER_ONE_EIGHTH
        } else {
            METER_EMPTY
        };

        bar.push_str(partial);

        let empty_units = width.saturating_sub(full_units + 1);
        for _ in 0..empty_units {
            bar.push_str(METER_EMPTY);
        }
    }

    bar
}

// ============================================================================
// 4. Configuration & Visual Styling Options
// ============================================================================

/// Border style presets for the statistics card.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum StatsCardBorderStyle {
    /// Modern rounded Unicode corners (`╭─╮`, `╰─╯`).
    #[default]
    Rounded,
    /// Square single-line borders (`┌─┐`, `└─┘`).
    Plain,
    /// High-emphasis double line borders (`╔═╗`, `╚═╝`).
    Double,
    /// Bold/thick Unicode borders (`┏━┓`, `┗━┛`).
    Thick,
    /// Standard ASCII characters for legacy terminal compatibility (`+--+`, `|  |`).
    Ascii,
    /// Borderless card.
    None,
}

impl StatsCardBorderStyle {
    /// Converts to Ratatui [`BorderType`].
    pub fn to_border_type(self) -> BorderType {
        match self {
            Self::Rounded => BorderType::Rounded,
            Self::Plain => BorderType::Plain,
            Self::Double => BorderType::Double,
            Self::Thick => BorderType::Thick,
            Self::Ascii => BorderType::Plain,
            Self::None => BorderType::Plain,
        }
    }
}

/// Visual layout modes for rendering.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum StatsCardLayout {
    /// Full rich card with all metrics, token breakdown, and tool table.
    #[default]
    Full,
    /// Compact 4-5 line summary box.
    Compact,
    /// Minimal key-value metrics only.
    MetricsOnly,
    /// Tool executions table and distribution only.
    ToolsOnly,
}

/// Rendering and display options for [`SessionStatsCardWidget`].
#[derive(Debug, Clone, PartialEq)]
pub struct StatsCardConfig {
    /// Active UI theme.
    pub theme: Theme,
    /// Border style.
    pub border_style: StatsCardBorderStyle,
    /// Layout density mode.
    pub layout: StatsCardLayout,
    /// Maximum number of individual tool rows displayed before summarizing remainder.
    pub max_tools: usize,
    /// Whether to render horizontal progress bars for tool distribution.
    pub show_tool_progress_bars: bool,
    /// Whether to display token breakdown (prompt, completion, cache).
    pub show_token_breakdown: bool,
    /// Whether to display cost breakdown (input, output, cache savings).
    pub show_cost_breakdown: bool,
    /// Whether to show per-turn averages.
    pub show_averages: bool,
    /// Optional custom title override.
    pub title: Option<String>,
    /// Explicit card width override in columns.
    pub width: Option<usize>,
}

impl Default for StatsCardConfig {
    fn default() -> Self {
        Self {
            theme: Theme::tokyo_night(),
            border_style: StatsCardBorderStyle::Rounded,
            layout: StatsCardLayout::Full,
            max_tools: DEFAULT_MAX_TOOLS_DISPLAYED,
            show_tool_progress_bars: true,
            show_token_breakdown: true,
            show_cost_breakdown: true,
            show_averages: true,
            title: None,
            width: None,
        }
    }
}

impl StatsCardConfig {
    /// Creates a new config with the specified theme.
    pub fn with_theme(theme: Theme) -> Self {
        Self {
            theme,
            ..Default::default()
        }
    }

    /// Sets the border style.
    pub fn border(mut self, border: StatsCardBorderStyle) -> Self {
        self.border_style = border;
        self
    }

    /// Sets the layout density mode.
    pub fn layout(mut self, layout: StatsCardLayout) -> Self {
        self.layout = layout;
        self
    }

    /// Sets maximum tools displayed.
    pub fn max_tools(mut self, max: usize) -> Self {
        self.max_tools = max;
        self
    }

    /// Sets custom card title.
    pub fn title(mut self, title: impl Into<String>) -> Self {
        self.title = Some(title.into());
        self
    }

    /// Sets explicit card width.
    pub fn width(mut self, width: usize) -> Self {
        self.width = Some(width);
        self
    }
}

// ============================================================================
// 5. Ratatui Widget Implementation
// ============================================================================

/// Interactive state for [`SessionStatsCardWidget`].
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SessionStatsCardState {
    /// Scroll offset for scrollable tool table.
    pub scroll_offset: usize,
    /// Currently highlighted or selected tool index.
    pub selected_tool: Option<usize>,
    /// Whether the card details are expanded.
    pub is_expanded: bool,
}

impl SessionStatsCardState {
    /// Creates a new initial state.
    pub fn new() -> Self {
        Self {
            scroll_offset: 0,
            selected_tool: None,
            is_expanded: true,
        }
    }

    /// Selects the next tool down.
    pub fn select_next(&mut self, total_tools: usize) {
        if total_tools == 0 {
            self.selected_tool = None;
            return;
        }
        self.selected_tool = match self.selected_tool {
            Some(curr) if curr + 1 < total_tools => Some(curr + 1),
            Some(_) => Some(0),
            None => Some(0),
        };
    }

    /// Selects the previous tool up.
    pub fn select_prev(&mut self, total_tools: usize) {
        if total_tools == 0 {
            self.selected_tool = None;
            return;
        }
        self.selected_tool = match self.selected_tool {
            Some(curr) if curr > 0 => Some(curr - 1),
            Some(_) => Some(total_tools.saturating_sub(1)),
            None => Some(total_tools.saturating_sub(1)),
        };
    }
}

/// High-polish Ratatui card widget displaying complete session statistics.
#[derive(Debug, Clone)]
pub struct SessionStatsCardWidget<'a> {
    stats: &'a SessionStats,
    config: StatsCardConfig,
}

impl<'a> SessionStatsCardWidget<'a> {
    /// Creates a new widget referencing the given session statistics.
    pub fn new(stats: &'a SessionStats) -> Self {
        Self {
            stats,
            config: StatsCardConfig::default(),
        }
    }

    /// Configures the widget with a custom config.
    pub fn with_config(mut self, config: StatsCardConfig) -> Self {
        self.config = config;
        self
    }

    /// Configures the widget with a specific theme.
    pub fn with_theme(mut self, theme: Theme) -> Self {
        self.config.theme = theme;
        self
    }

    /// Configures the border style.
    pub fn with_border(mut self, border: StatsCardBorderStyle) -> Self {
        self.config.border_style = border;
        self
    }

    /// Configures the layout density.
    pub fn with_layout(mut self, layout: StatsCardLayout) -> Self {
        self.config.layout = layout;
        self
    }
}

impl<'a> Widget for SessionStatsCardWidget<'a> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let mut state = SessionStatsCardState::new();
        StatefulWidget::render(self, area, buf, &mut state);
    }
}

impl<'a> StatefulWidget for SessionStatsCardWidget<'a> {
    type State = SessionStatsCardState;

    fn render(self, area: Rect, buf: &mut Buffer, _state: &mut Self::State) {
        if area.width < 4 || area.height < 3 {
            return;
        }

        let theme = &self.config.theme;

        // Title text
        let title_text = self
            .config
            .title
            .clone()
            .unwrap_or_else(|| " 📊 Session Statistics ".to_string());

        // Outer block
        let mut block = Block::default()
            .borders(if self.config.border_style == StatsCardBorderStyle::None {
                Borders::NONE
            } else {
                Borders::ALL
            })
            .border_type(self.config.border_style.to_border_type())
            .border_style(Style::default().fg(theme.border))
            .title(Span::styled(
                title_text,
                Style::default()
                    .fg(theme.primary)
                    .add_modifier(Modifier::BOLD),
            ));

        if let Some(bg) = theme.background {
            block = block.style(Style::default().bg(bg));
        }

        let inner_area = block.inner(area);
        block.render(area, buf);

        if inner_area.width == 0 || inner_area.height == 0 {
            return;
        }

        // Render contents based on layout mode
        match self.config.layout {
            StatsCardLayout::Compact => {
                render_compact_ratatui(self.stats, theme, inner_area, buf);
            }
            StatsCardLayout::MetricsOnly => {
                render_metrics_only_ratatui(self.stats, theme, inner_area, buf);
            }
            StatsCardLayout::ToolsOnly => {
                render_tools_only_ratatui(self.stats, &self.config, inner_area, buf);
            }
            StatsCardLayout::Full => {
                render_full_ratatui(self.stats, &self.config, inner_area, buf);
            }
        }
    }
}

/// Renders the full rich view into a Ratatui buffer.
fn render_full_ratatui(
    stats: &SessionStats,
    config: &StatsCardConfig,
    area: Rect,
    buf: &mut Buffer,
) {
    let theme = &config.theme;
    let mut current_y = area.y;
    let max_y = area.y + area.height;

    // 1. Session & Model Header Line
    if current_y < max_y {
        let model_span = Span::styled(
            format!(" 🤖 Model: {} ", stats.model_name),
            Style::default()
                .fg(theme.secondary)
                .add_modifier(Modifier::BOLD),
        );

        let turn_badge = Span::styled(
            format!(
                " [{} Turn{}] ",
                stats.turn_count,
                if stats.turn_count == 1 { "" } else { "s" }
            ),
            Style::default().fg(theme.info),
        );

        let msg_badge = Span::styled(
            format!("({} msgs) ", stats.message_count),
            Style::default().fg(theme.muted),
        );

        let mut header_spans = vec![model_span, turn_badge, msg_badge];

        if stats.is_active {
            header_spans.push(Span::styled(
                "● ACTIVE",
                Style::default()
                    .fg(theme.success)
                    .add_modifier(Modifier::BOLD),
            ));
        }

        let header_line = Line::from(header_spans);
        buf.set_line(area.x, current_y, &header_line, area.width);
        current_y += 1;
    }

    // Divider
    if current_y < max_y {
        let div = "─".repeat(area.width as usize);
        buf.set_string(area.x, current_y, div, Style::default().fg(theme.muted));
        current_y += 1;
    }

    // 2. Core Metrics 4-Box Grid
    // [💬 Turns]   [🔤 Tokens]   [💰 Cost]   [⏱️ Duration]
    if current_y < max_y {
        let turns_str = format!("Turns: {}", stats.turn_count);
        let tokens_str = format!("Tokens: {}", stats.format_tokens());
        let cost_str = format!("Cost: {}", stats.format_cost());
        let dur_str = format!("Duration: {}", stats.format_duration());

        let col_width = (area.width / 4).max(1);

        // Line 1: Values
        let x0 = area.x;
        let x1 = area.x + col_width;
        let x2 = area.x + col_width * 2;
        let x3 = area.x + col_width * 3;

        buf.set_string(
            x0,
            current_y,
            &turns_str,
            Style::default().fg(theme.info).add_modifier(Modifier::BOLD),
        );
        buf.set_string(
            x1,
            current_y,
            &tokens_str,
            Style::default()
                .fg(theme.primary)
                .add_modifier(Modifier::BOLD),
        );
        buf.set_string(
            x2,
            current_y,
            &cost_str,
            Style::default()
                .fg(theme.warning)
                .add_modifier(Modifier::BOLD),
        );
        buf.set_string(
            x3,
            current_y,
            &dur_str,
            Style::default()
                .fg(theme.accent)
                .add_modifier(Modifier::BOLD),
        );
        current_y += 1;

        // Line 2: Secondary subtitles / breakdowns
        if current_y < max_y && config.show_averages {
            let avg_tok = format!("~{}/turn", stats.avg_tokens_per_turn());
            let tok_split = format!(
                "P:{} C:{}",
                format_token_count(stats.prompt_tokens() as usize),
                format_token_count(stats.completion_tokens() as usize)
            );
            let avg_cost = format!("~{}/t", format_usd(stats.avg_cost_per_turn()));
            let avg_dur = format!(
                "~{}/t",
                format_duration_pretty(stats.avg_duration_per_turn())
            );

            buf.set_string(x0, current_y, avg_tok, Style::default().fg(theme.muted));
            buf.set_string(x1, current_y, tok_split, Style::default().fg(theme.muted));
            buf.set_string(x2, current_y, avg_cost, Style::default().fg(theme.muted));
            buf.set_string(x3, current_y, avg_dur, Style::default().fg(theme.muted));
            current_y += 1;
        }
    }

    // 3. Token & Cache Details Line (if space permits)
    if config.show_token_breakdown && current_y < max_y {
        let mut breakdown_parts = Vec::new();
        breakdown_parts.push(format!("Prompt: {}", stats.prompt_tokens()));
        breakdown_parts.push(format!("Completion: {}", stats.completion_tokens()));

        if stats.cache_read_tokens() > 0 || stats.cache_write_tokens() > 0 {
            breakdown_parts.push(format!(
                "Cache: [Read: {}, Write: {} | Hit: {:.1}%]",
                stats.cache_read_tokens(),
                stats.cache_write_tokens(),
                stats.cache_hit_ratio()
            ));
        }

        if stats.cost_breakdown.cache_savings > 1e-5 {
            breakdown_parts.push(format!(
                "Saved: {}",
                format_usd(stats.cost_breakdown.cache_savings)
            ));
        }

        let breakdown_str = format!("  ↳ {}", breakdown_parts.join("  |  "));
        buf.set_string(
            area.x,
            current_y,
            breakdown_str,
            Style::default().fg(theme.muted),
        );
        current_y += 1;
    }

    // Divider before Tools
    if current_y < max_y {
        let div = "─".repeat(area.width as usize);
        buf.set_string(area.x, current_y, div, Style::default().fg(theme.muted));
        current_y += 1;
    }

    // 4. Tool Execution Section
    if current_y < max_y {
        let tools_header = format!(
            " ⚙️ Tool Executions ({} total, {} unique tools)",
            stats.total_tool_calls,
            stats.unique_tools_count()
        );
        buf.set_string(
            area.x,
            current_y,
            tools_header,
            Style::default()
                .fg(theme.secondary)
                .add_modifier(Modifier::BOLD),
        );
        current_y += 1;
    }

    if stats.tool_stats.is_empty() {
        if current_y < max_y {
            buf.set_string(
                area.x + 2,
                current_y,
                "No tool executions recorded in this session.",
                Style::default().fg(theme.muted),
            );
        }
        return;
    }

    // Tool Rows
    let max_tools = config.max_tools.min(stats.tool_stats.len());
    let bar_width = 16.min(area.width.saturating_sub(36) as usize);

    for (idx, tool) in stats.tool_stats.iter().take(max_tools).enumerate() {
        if current_y >= max_y {
            break;
        }

        let pct = if stats.total_tool_calls > 0 {
            (tool.call_count as f64 / stats.total_tool_calls as f64) * 100.0
        } else {
            0.0
        };

        // Tool name (padded)
        let name_str = format!(" {:<12}", tool.name);
        buf.set_string(
            area.x,
            current_y,
            &name_str,
            Style::default()
                .fg(theme.primary)
                .add_modifier(Modifier::BOLD),
        );

        // Call count & Percentage
        let count_str = format!("{:>3} calls ({:>4.1}%)", tool.call_count, pct);
        buf.set_string(
            area.x + 14,
            current_y,
            &count_str,
            Style::default().fg(theme.foreground),
        );

        // Visual Progress Bar
        if config.show_tool_progress_bars && bar_width > 4 {
            let meter_str = render_meter_bar(pct / 100.0, bar_width);
            let bar_x = area.x + 32;
            buf.set_string(
                bar_x,
                current_y,
                format!(" [{}]", meter_str),
                Style::default().fg(theme.secondary),
            );
        }

        // Status badge (Success/Failure)
        let status_x = area.x + area.width.saturating_sub(14);
        if tool.failure_count > 0 {
            let fail_str = format!("✓{} ✗{}", tool.success_count, tool.failure_count);
            buf.set_string(
                status_x,
                current_y,
                fail_str,
                Style::default().fg(theme.error),
            );
        } else {
            let ok_str = format!("✓ {} ok", tool.success_count);
            buf.set_string(
                status_x,
                current_y,
                ok_str,
                Style::default().fg(theme.success),
            );
        }

        current_y += 1;
    }

    // Remainder note if tools exceeded max_tools
    if stats.tool_stats.len() > max_tools && current_y < max_y {
        let remainder = stats.tool_stats.len() - max_tools;
        let rem_str = format!(
            "   ... and {} more tool type{}",
            remainder,
            if remainder == 1 { "" } else { "s" }
        );
        buf.set_string(area.x, current_y, rem_str, Style::default().fg(theme.muted));
    }
}

/// Renders compact layout into Ratatui buffer.
fn render_compact_ratatui(stats: &SessionStats, theme: &Theme, area: Rect, buf: &mut Buffer) {
    let mut current_y = area.y;
    let max_y = area.y + area.height;

    if current_y < max_y {
        let line1 = format!(
            "Model: {} | Turns: {} | msgs: {}",
            stats.model_name, stats.turn_count, stats.message_count
        );
        buf.set_string(
            area.x,
            current_y,
            line1,
            Style::default()
                .fg(theme.primary)
                .add_modifier(Modifier::BOLD),
        );
        current_y += 1;
    }

    if current_y < max_y {
        let line2 = format!(
            "Tokens: {} (P:{} C:{}) | Cost: {} | Dur: {}",
            stats.format_tokens(),
            stats.prompt_tokens(),
            stats.completion_tokens(),
            stats.format_cost(),
            stats.format_duration()
        );
        buf.set_string(
            area.x,
            current_y,
            line2,
            Style::default().fg(theme.foreground),
        );
        current_y += 1;
    }

    if current_y < max_y && stats.total_tool_calls > 0 {
        let top_tools_summary: Vec<String> = stats
            .top_tools(4)
            .iter()
            .map(|t| format!("{}:{}", t.name, t.call_count))
            .collect();
        let line3 = format!(
            "Tools ({}/{}): {}",
            stats.total_tool_calls,
            stats.unique_tools_count(),
            top_tools_summary.join(", ")
        );
        buf.set_string(
            area.x,
            current_y,
            line3,
            Style::default().fg(theme.secondary),
        );
    }
}

/// Renders metrics-only layout into Ratatui buffer.
fn render_metrics_only_ratatui(stats: &SessionStats, theme: &Theme, area: Rect, buf: &mut Buffer) {
    let mut current_y = area.y;
    let max_y = area.y + area.height;

    let items = [
        ("Turns", stats.turn_count.to_string(), theme.info),
        ("Total Tokens", stats.format_tokens(), theme.primary),
        (
            "Prompt Tokens",
            stats.prompt_tokens().to_string(),
            theme.muted,
        ),
        (
            "Completion Tokens",
            stats.completion_tokens().to_string(),
            theme.muted,
        ),
        ("Cost (USD)", stats.format_cost(), theme.warning),
        ("Duration", stats.format_duration(), theme.accent),
        (
            "Tool Calls",
            stats.total_tool_calls.to_string(),
            theme.secondary,
        ),
    ];

    for (label, val, color) in items {
        if current_y >= max_y {
            break;
        }
        let line = format!("  {:<18} : {}", label, val);
        buf.set_string(area.x, current_y, line, Style::default().fg(color));
        current_y += 1;
    }
}

/// Renders tools-only layout into Ratatui buffer.
fn render_tools_only_ratatui(
    stats: &SessionStats,
    config: &StatsCardConfig,
    area: Rect,
    buf: &mut Buffer,
) {
    let theme = &config.theme;
    let mut current_y = area.y;
    let max_y = area.y + area.height;

    if current_y < max_y {
        let title = format!(
            "Tools: {} total calls across {} types",
            stats.total_tool_calls,
            stats.unique_tools_count()
        );
        buf.set_string(
            area.x,
            current_y,
            title,
            Style::default()
                .fg(theme.secondary)
                .add_modifier(Modifier::BOLD),
        );
        current_y += 1;
    }

    for tool in stats.top_tools(config.max_tools) {
        if current_y >= max_y {
            break;
        }
        let pct = if stats.total_tool_calls > 0 {
            (tool.call_count as f64 / stats.total_tool_calls as f64) * 100.0
        } else {
            0.0
        };
        let line = format!(
            "  {:<12} {:>4} calls ({:>4.1}%) - ✓{} ✗{}",
            tool.name, tool.call_count, pct, tool.success_count, tool.failure_count
        );
        buf.set_string(
            area.x,
            current_y,
            line,
            Style::default().fg(theme.foreground),
        );
        current_y += 1;
    }
}

// ============================================================================
// 6. Standalone ANSI, Plain Text, and Markdown String Renderers
// ============================================================================

/// Renders a full statistics card into a beautifully styled ANSI terminal string.
pub fn render_stats_card_ansi(stats: &SessionStats, config: &StatsCardConfig) -> String {
    let target_width = config
        .width
        .unwrap_or(DEFAULT_CARD_WIDTH)
        .max(MIN_CARD_WIDTH);
    let inner_width = target_width.saturating_sub(2);

    let (tl, tr, bl, br, h, v, sep_l, sep_r) = match config.border_style {
        StatsCardBorderStyle::Rounded => ("╭", "╮", "╰", "╯", "─", "│", "├", "┤"),
        StatsCardBorderStyle::Plain => ("┌", "┐", "└", "┘", "─", "│", "├", "┤"),
        StatsCardBorderStyle::Double => ("╔", "╗", "╚", "╝", "═", "║", "╠", "╣"),
        StatsCardBorderStyle::Thick => ("┏", "┓", "┗", "┛", "━", "┃", "┣", "┫"),
        StatsCardBorderStyle::Ascii => ("+", "+", "+", "+", "-", "|", "+", "+"),
        StatsCardBorderStyle::None => ("", "", "", "", " ", " ", "", ""),
    };

    let mut out = String::new();

    // 1. Top border with title
    let raw_title = config.title.as_deref().unwrap_or("📊 Session Statistics");
    let title_styled = format!(
        " {}{}{}{} ",
        ANSI_BOLD_CYAN, ANSI_BOLD, raw_title, ANSI_RESET
    );
    let title_vis_len = visible_width(raw_title) + 2;

    let right_border_len = inner_width.saturating_sub(title_vis_len + 1);
    let top_line = format!(
        "{}{}{}{}{}\n",
        tl,
        h,
        title_styled,
        h.repeat(right_border_len),
        tr
    );
    out.push_str(&top_line);

    // Helper closure for boxing a line
    let box_line = |content: &str| -> String {
        let vis_len = visible_width(content);
        let pad = inner_width.saturating_sub(vis_len);
        format!("{}{}{}{}\n", v, content, " ".repeat(pad), v)
    };

    // Helper closure for divider
    let div_line = || -> String { format!("{}{}{}\n", sep_l, h.repeat(inner_width), sep_r) };

    // 2. Model & Session Info Line
    let model_tag = format!(
        " 🤖 {}Model:{} {}{}{} ",
        ANSI_BOLD_BLUE, ANSI_RESET, ANSI_BOLD_WHITE, stats.model_name, ANSI_RESET
    );
    let turn_tag = format!(
        "[{}Turn{}{}: {}{}{}] ",
        ANSI_CYAN,
        if stats.turn_count == 1 { "" } else { "s" },
        ANSI_RESET,
        ANSI_BOLD_CYAN,
        stats.turn_count,
        ANSI_RESET
    );
    let msg_tag = format!("({} msgs)", stats.message_count);
    out.push_str(&box_line(&format!("{}{}{}", model_tag, turn_tag, msg_tag)));

    out.push_str(&div_line());

    // 3. Core 4-Metric Grid
    // Line 1: Main Metric Values
    let turns_fmt = format!(
        "💬 {}Turns:{} {}{}{}",
        ANSI_DIM, ANSI_RESET, ANSI_BOLD_CYAN, stats.turn_count, ANSI_RESET
    );
    let tokens_fmt = format!(
        "🔤 {}Tokens:{} {}{}{}",
        ANSI_DIM,
        ANSI_RESET,
        ANSI_BOLD_GREEN,
        stats.format_tokens(),
        ANSI_RESET
    );
    let cost_fmt = format!(
        "💰 {}Cost:{} {}{}{}",
        ANSI_DIM,
        ANSI_RESET,
        ANSI_BOLD_YELLOW,
        stats.format_cost(),
        ANSI_RESET
    );
    let dur_fmt = format!(
        "⏱️ {}Duration:{} {}{}{}",
        ANSI_DIM,
        ANSI_RESET,
        ANSI_BOLD_MAGENTA,
        stats.format_duration(),
        ANSI_RESET
    );

    let quarter_width = inner_width / 4;
    let col0 = pad_ansi_right(&turns_fmt, quarter_width);
    let col1 = pad_ansi_right(&tokens_fmt, quarter_width);
    let col2 = pad_ansi_right(&cost_fmt, quarter_width);
    let col3 = dur_fmt;
    out.push_str(&box_line(&format!(" {}{}{}{}", col0, col1, col2, col3)));

    // Line 2: Averages & Breakdown
    if config.show_averages {
        let avg_tok = format!("~{}/turn", stats.avg_tokens_per_turn());
        let tok_split = format!(
            "P:{} C:{}",
            format_token_count(stats.prompt_tokens() as usize),
            format_token_count(stats.completion_tokens() as usize)
        );
        let avg_cost = format!("~{}/t", format_usd(stats.avg_cost_per_turn()));
        let avg_dur = format!(
            "~{}/t",
            format_duration_pretty(stats.avg_duration_per_turn())
        );

        let s0 = pad_ansi_right(
            &format!("  {}{}{}", ANSI_GRAY, avg_tok, ANSI_RESET),
            quarter_width,
        );
        let s1 = pad_ansi_right(
            &format!("{}{}{}", ANSI_GRAY, tok_split, ANSI_RESET),
            quarter_width,
        );
        let s2 = pad_ansi_right(
            &format!("{}{}{}", ANSI_GRAY, avg_cost, ANSI_RESET),
            quarter_width,
        );
        let s3 = format!("{}{}{}", ANSI_GRAY, avg_dur, ANSI_RESET);
        out.push_str(&box_line(&format!("{}{}{}{}", s0, s1, s2, s3)));
    }

    // Line 3: Cache / Cost Breakdown
    if config.show_token_breakdown
        && (stats.cache_read_tokens() > 0
            || stats.cache_write_tokens() > 0
            || stats.cost_breakdown.cache_savings > 1e-5)
    {
        let mut cache_parts = Vec::new();
        if stats.cache_read_tokens() > 0 || stats.cache_write_tokens() > 0 {
            cache_parts.push(format!(
                "Cache: [Read: {}, Write: {} | Hit: {:.1}%]",
                stats.cache_read_tokens(),
                stats.cache_write_tokens(),
                stats.cache_hit_ratio()
            ));
        }
        if stats.cost_breakdown.cache_savings > 1e-5 {
            cache_parts.push(format!(
                "Saved: {}",
                format_usd(stats.cost_breakdown.cache_savings)
            ));
        }
        let cache_str = format!(
            "  ↳ {}{}{}",
            ANSI_GRAY,
            cache_parts.join("  |  "),
            ANSI_RESET
        );
        out.push_str(&box_line(&cache_str));
    }

    out.push_str(&div_line());

    // 4. Tool Execution Section
    let tool_hdr = format!(
        " ⚙️ {}Tool Executions{} ({} total, {} unique tools)",
        ANSI_BOLD_MAGENTA,
        ANSI_RESET,
        stats.total_tool_calls,
        stats.unique_tools_count()
    );
    out.push_str(&box_line(&tool_hdr));

    if stats.tool_stats.is_empty() {
        out.push_str(&box_line(&format!(
            "   {}No tool executions recorded.{}",
            ANSI_GRAY, ANSI_RESET
        )));
    } else {
        let max_tools = config.max_tools.min(stats.tool_stats.len());
        let bar_width = 16.min(inner_width.saturating_sub(36));

        for tool in stats.tool_stats.iter().take(max_tools) {
            let pct = if stats.total_tool_calls > 0 {
                (tool.call_count as f64 / stats.total_tool_calls as f64) * 100.0
            } else {
                0.0
            };

            let name_colored = format!(" {}{:<12}{}", ANSI_BOLD_CYAN, tool.name, ANSI_RESET);
            let count_colored = format!("{:>3} calls ({:>4.1}%)", tool.call_count, pct);

            let bar_str = if config.show_tool_progress_bars && bar_width > 4 {
                let bar = render_meter_bar(pct / 100.0, bar_width);
                format!(" {}{}[{}]{}", ANSI_CYAN, ANSI_DIM, bar, ANSI_RESET)
            } else {
                String::new()
            };

            let status_str = if tool.failure_count > 0 {
                format!(
                    " {}{}✓{} ✗{}{}",
                    ANSI_RED, ANSI_BOLD, tool.success_count, tool.failure_count, ANSI_RESET
                )
            } else {
                format!(
                    " {}{}✓ {} ok{}",
                    ANSI_GREEN, ANSI_DIM, tool.success_count, ANSI_RESET
                )
            };

            let line_content = format!(
                "{} {} {}{}",
                name_colored, count_colored, bar_str, status_str
            );
            out.push_str(&box_line(&line_content));
        }

        if stats.tool_stats.len() > max_tools {
            let rem = stats.tool_stats.len() - max_tools;
            out.push_str(&box_line(&format!(
                "   {}... and {} more tool type{}{}",
                ANSI_GRAY,
                rem,
                if rem == 1 { "" } else { "s" },
                ANSI_RESET
            )));
        }
    }

    // 5. Bottom border
    let bot_line = format!("{}{}{}\n", bl, h.repeat(inner_width), br);
    out.push_str(&bot_line);

    out
}

/// Helper to pad ANSI string with spaces to align columns.
fn pad_ansi_right(s: &str, width: usize) -> String {
    let vis = visible_width(s);
    if vis < width {
        format!("{}{}", s, " ".repeat(width - vis))
    } else {
        s.to_string()
    }
}

/// Renders a plain text statistics card with zero ANSI escape codes.
pub fn render_stats_card_plain(stats: &SessionStats, width: usize) -> String {
    let mut cfg = StatsCardConfig::default();
    cfg.width = Some(width.max(MIN_CARD_WIDTH));
    cfg.border_style = StatsCardBorderStyle::Ascii;
    let ansi_rendered = render_stats_card_ansi(stats, &cfg);
    strip_ansi(&ansi_rendered)
}

/// Renders a Markdown summary of the session statistics.
pub fn render_stats_card_markdown(stats: &SessionStats) -> String {
    let mut md = String::new();

    md.push_str("### 📊 Session Statistics\n\n");
    md.push_str(&format!(
        "- **Model**: `{}`\n\
         - **Turns**: {} ({} messages)\n\
         - **Total Tokens**: {} (Prompt: {}, Completion: {})\n\
         - **Estimated Cost**: {}\n\
         - **Duration**: {}\n\
         - **Tool Calls**: {} total across {} unique tools\n\n",
        stats.model_name,
        stats.turn_count,
        stats.message_count,
        stats.format_tokens(),
        stats.prompt_tokens(),
        stats.completion_tokens(),
        stats.format_cost(),
        stats.format_duration(),
        stats.total_tool_calls,
        stats.unique_tools_count()
    ));

    if !stats.tool_stats.is_empty() {
        md.push_str("#### Tool Executions\n\n");
        md.push_str("| Tool | Invocations | Share | Success Rate |\n");
        md.push_str("| :--- | :---: | :---: | :---: |\n");

        for tool in &stats.tool_stats {
            let pct = if stats.total_tool_calls > 0 {
                (tool.call_count as f64 / stats.total_tool_calls as f64) * 100.0
            } else {
                0.0
            };
            md.push_str(&format!(
                "| `{}` | {} | {:.1}% | {:.1}% (✓{} / ✗{}) |\n",
                tool.name,
                tool.call_count,
                pct,
                tool.success_rate(),
                tool.success_count,
                tool.failure_count
            ));
        }
        md.push('\n');
    }

    md
}

/// Renders a single-line high-density ANSI status string.
pub fn render_stats_compact_ansi(stats: &SessionStats, theme: &Theme) -> String {
    let tools_part = if stats.total_tool_calls > 0 {
        format!(
            " | Tools: {}{}{}",
            ANSI_BOLD_MAGENTA, stats.total_tool_calls, ANSI_RESET
        )
    } else {
        String::new()
    };

    format!(
        "{}Turns:{} {}{}{} | {}Tokens:{} {}{}{} | {}Cost:{} {}{}{} | {}Dur:{} {}{}{}{}",
        ANSI_DIM,
        ANSI_RESET,
        ANSI_BOLD_CYAN,
        stats.turn_count,
        ANSI_RESET,
        ANSI_DIM,
        ANSI_RESET,
        ANSI_BOLD_GREEN,
        stats.format_tokens(),
        ANSI_RESET,
        ANSI_DIM,
        ANSI_RESET,
        ANSI_BOLD_YELLOW,
        stats.format_cost(),
        ANSI_RESET,
        ANSI_DIM,
        ANSI_RESET,
        ANSI_BOLD_WHITE,
        stats.format_duration(),
        ANSI_RESET,
        tools_part
    )
}

// ============================================================================
// 7. Unit Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::provider::types::{Message, Role, ToolCall};
    #[test]
    fn test_session_stats_default() {
        let stats = SessionStats::new();
        assert_eq!(stats.turn_count, 0);
        assert_eq!(stats.total_tokens(), 0);
        assert_eq!(stats.total_cost(), 0.0);
        assert_eq!(stats.duration, Duration::ZERO);
        assert_eq!(stats.total_tool_calls, 0);
        assert!(stats.tool_stats.is_empty());
    }

    #[test]
    fn test_session_stats_builder() {
        let stats = SessionStats::builder()
            .session_id("sess-1234")
            .session_title("Refactor parser")
            .model_name("claude-3-7-sonnet")
            .turn_count(8)
            .message_count(16)
            .tokens(12000, 4500)
            .cache_tokens(3000, 1000)
            .total_cost(0.042)
            .duration(Duration::from_secs(145))
            .add_tool("bash", 10)
            .add_tool("read", 6)
            .add_tool("edit", 4)
            .build();

        assert_eq!(stats.session_id.as_deref(), Some("sess-1234"));
        assert_eq!(stats.turn_count, 8);
        assert_eq!(stats.message_count, 16);
        assert_eq!(stats.prompt_tokens(), 12000);
        assert_eq!(stats.completion_tokens(), 4500);
        assert_eq!(stats.total_tokens(), 16500);
        assert_eq!(stats.cache_read_tokens(), 3000);
        assert_eq!(stats.total_cost(), 0.042);
        assert_eq!(stats.duration, Duration::from_secs(145));
        assert_eq!(stats.total_tool_calls, 20);
        assert_eq!(stats.unique_tools_count(), 3);
        assert_eq!(stats.avg_tokens_per_turn(), 16500 / 8);
    }

    #[test]
    fn test_tool_execution_stat_success_rate() {
        let mut stat = ToolExecutionStat::new("bash", 10);
        assert_eq!(stat.success_rate(), 100.0);
        assert!(stat.is_perfect());

        stat.failure_count = 2;
        stat.success_count = 8;
        assert_eq!(stat.success_rate(), 80.0);
        assert!(!stat.is_perfect());
    }

    #[test]
    fn test_extract_tool_stats_from_messages() {
        let mut messages = Vec::new();

        // Turn 1
        messages.push(Message::user("Please list files and check git status."));
        messages.push(Message {
            role: Role::Assistant,
            content: "I will check the directory and git status.".to_string(),
            name: None,
            tool_calls: Some(vec![
                ToolCall {
                    id: "call_1".to_string(),
                    name: "bash".to_string(),
                    arguments: "{\"command\":\"ls\"}".to_string(),
                },
                ToolCall {
                    id: "call_2".to_string(),
                    name: "read".to_string(),
                    arguments: "{\"path\":\"Cargo.toml\"}".to_string(),
                },
            ]),
            tool_call_id: None,
        });

        messages.push(Message::tool_result("call_1", "src Cargo.toml target"));
        messages.push(Message::tool_result(
            "call_2",
            "[package]\nname = \"fusion\"",
        ));

        // Turn 2
        messages.push(Message::user("Now search for config."));
        messages.push(Message {
            role: Role::Assistant,
            content: "Searching...".to_string(),
            name: None,
            tool_calls: Some(vec![
                ToolCall {
                    id: "call_3".to_string(),
                    name: "grep".to_string(),
                    arguments: "{\"pattern\":\"Config\"}".to_string(),
                },
                ToolCall {
                    id: "call_4".to_string(),
                    name: "bash".to_string(),
                    arguments: "{\"command\":\"cargo check\"}".to_string(),
                },
            ]),
            tool_call_id: None,
        });

        messages.push(Message::tool_result("call_3", "src/config.rs:10"));
        messages.push(Message::tool_result("call_4", "Error: failed to compile"));

        let (stats, total_calls) = extract_tool_stats(&messages);
        assert_eq!(total_calls, 4);
        assert_eq!(stats.len(), 3);

        // bash has 2 calls (1 success, 1 failure)
        let bash_stat = stats.iter().find(|s| s.name == "bash").unwrap();
        assert_eq!(bash_stat.call_count, 2);
        assert_eq!(bash_stat.failure_count, 1);
        assert_eq!(bash_stat.success_count, 1);

        // read has 1 call (1 success)
        let read_stat = stats.iter().find(|s| s.name == "read").unwrap();
        assert_eq!(read_stat.call_count, 1);
        assert_eq!(read_stat.success_count, 1);
        assert_eq!(read_stat.failure_count, 0);

        // grep has 1 call (1 success)
        let grep_stat = stats.iter().find(|s| s.name == "grep").unwrap();
        assert_eq!(grep_stat.call_count, 1);
    }

    #[test]
    fn test_session_stats_from_session() {
        let mut session = Session::new("deepseek-chat");
        session.set_title("Test Session");
        session.add_user_message("Hello");
        session.add_assistant_message("Hi there!");
        session.record_usage(500, 150);

        let stats = SessionStats::from_session(&session);
        assert_eq!(stats.model_name, "deepseek-chat");
        assert_eq!(stats.turn_count, 1);
        assert_eq!(stats.prompt_tokens(), 500);
        assert_eq!(stats.completion_tokens(), 150);
        assert_eq!(stats.total_tokens(), 650);
    }

    #[test]
    fn test_render_meter_bar() {
        let bar_0 = render_meter_bar(0.0, 10);
        assert_eq!(visible_width(&bar_0), 10);

        let bar_50 = render_meter_bar(0.5, 10);
        assert_eq!(visible_width(&bar_50), 10);
        assert!(bar_50.contains(METER_FULL));

        let bar_100 = render_meter_bar(1.0, 10);
        assert_eq!(visible_width(&bar_100), 10);
        assert_eq!(bar_100, METER_FULL.repeat(10));
    }

    #[test]
    fn test_format_duration_pretty() {
        assert_eq!(format_duration_pretty(Duration::from_millis(0)), "0s");
        assert_eq!(format_duration_pretty(Duration::from_millis(450)), "450ms");
        assert_eq!(format_duration_pretty(Duration::from_secs(45)), "45s");
        assert_eq!(format_duration_pretty(Duration::from_secs(134)), "2m 14s");
        assert_eq!(format_duration_pretty(Duration::from_secs(3600)), "1h");
        assert_eq!(
            format_duration_pretty(Duration::from_secs(3665)),
            "1h 01m 05s"
        );
    }

    #[test]
    fn test_render_stats_card_plain() {
        let stats = SessionStats::builder()
            .model_name("gpt-4o")
            .turn_count(5)
            .message_count(10)
            .tokens(8000, 2000)
            .total_cost(0.05)
            .duration(Duration::from_secs(80))
            .add_tool("bash", 4)
            .add_tool("edit", 2)
            .build();

        let plain = render_stats_card_plain(&stats, 64);
        assert!(plain.contains("Session Statistics"));
        assert!(plain.contains("gpt-4o"));
        assert!(plain.contains("Turns: 5"));
        assert!(plain.contains("Tokens: 10.0k"));
        assert!(plain.contains("Cost: $0.050"));
        assert!(plain.contains("bash"));
        assert!(plain.contains("edit"));
        assert!(!plain.contains("\x1b["));
    }

    #[test]
    fn test_render_stats_card_markdown() {
        let stats = SessionStats::builder()
            .model_name("claude-3-7-sonnet")
            .turn_count(3)
            .message_count(6)
            .tokens(5000, 1200)
            .total_cost(0.033)
            .duration(Duration::from_secs(45))
            .add_tool("read", 3)
            .build();

        let md = render_stats_card_markdown(&stats);
        assert!(md.contains("### 📊 Session Statistics"));
        assert!(md.contains("claude-3-7-sonnet"));
        assert!(md.contains("| `read` | 3 |"));
    }

    #[test]
    fn test_render_stats_compact_ansi() {
        let stats = SessionStats::builder()
            .turn_count(4)
            .tokens(3000, 1000)
            .total_cost(0.012)
            .duration(Duration::from_secs(30))
            .add_tool("bash", 2)
            .build();

        let theme = Theme::tokyo_night();
        let compact = render_stats_compact_ansi(&stats, &theme);
        assert!(compact.contains("Turns:"));
        assert!(compact.contains("Tokens:"));
        assert!(compact.contains("Cost:"));
        assert!(compact.contains("Dur:"));
        assert!(compact.contains("Tools:"));
    }

    #[test]
    fn test_ratatui_widget_rendering() {
        let stats = SessionStats::builder()
            .model_name("deepseek-chat")
            .turn_count(6)
            .message_count(12)
            .tokens(15000, 4000)
            .total_cost(0.008)
            .duration(Duration::from_secs(90))
            .add_tool("grep", 5)
            .add_tool("read", 3)
            .build();

        let widget = SessionStatsCardWidget::new(&stats)
            .with_border(StatsCardBorderStyle::Rounded)
            .with_layout(StatsCardLayout::Full);

        let mut buffer = Buffer::empty(Rect::new(0, 0, 70, 20));
        ratatui::widgets::Widget::render(widget, Rect::new(0, 0, 70, 20), &mut buffer);

        // Convert buffer lines to strings
        let mut buffer_text = String::new();
        for y in 0..20 {
            for x in 0..70 {
                let cell = buffer.get(x, y);
                buffer_text.push_str(cell.symbol());
            }
            buffer_text.push('\n');
        }

        assert!(buffer_text.contains("Session Statistics"));
        assert!(buffer_text.contains("deepseek-chat"));
        assert!(buffer_text.contains("Turns: 6"));
        assert!(buffer_text.contains("grep"));
        assert!(buffer_text.contains("read"));
    }
}
