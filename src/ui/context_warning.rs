//! Context Window Warning Alert Widget
//!
//! Provides a high-polish visual warning and alert widget when a conversation
//! approaches or exceeds model context window limits.
//!
//! # Features
//! - **Real-time Remaining Tokens & Capacity**:
//!   Calculates exact and effective remaining tokens before context exhaustion,
//!   taking into account safety margins and reserved completion tokens.
//! - **High-Resolution Percentage Bar**:
//!   Supports sub-character Unicode block rendering (`█`, `▉`, `▊`, `▋`, `▌`, `▍`, `▎`, `▏`),
//!   standard blocks, shaded blocks, ASCII, and Braille glyph styles with dynamic color thresholds.
//! - **Multi-Tier Severity Levels**:
//!   - `Safe`: `< 70%` context utilization (Normal operating margin).
//!   - `Notice`: `70% - 79.9%` context utilization (Headroom narrowing).
//!   - `Warning`: `80% - 89.9%` context utilization (Compaction recommended).
//!   - `Critical`: `90% - 99.9%` context utilization (Immediate compaction required to avoid truncation).
//!   - `Overflow`: `>= 100%` context utilization (Context window exceeded).
//! - **Multiple Display Modes**:
//!   - `Banner`: Multi-line bordered box alert with title, metrics grid, progress meter, and action recommendations.
//!   - `Compact`: Sleek 1-2 line inline alert banner for prompt footers or terminal logs.
//!   - `Pill`: Ultra-compact badge format (e.g. `[⚠ 88.4% | 14.8k rem]`).
//!   - `Card`: Floating modal-style card with detailed category breakdown (Sys, History, Tools) and turn estimates.
//!   - `MiniBar`: Standalone progress meter with token count indicators.
//! - **Ratatui Widget Support**:
//!   Full `ratatui::widgets::Widget` implementations (`ContextWarningWidget`, `CompactWarningWidget`, `ContextProgressBarWidget`)
//!   supporting theme integration, custom borders, titles, and layout constraints.
//! - **Stateful Alert Tracker (`ContextWarningTracker`)**:
//!   Provides hysteresis and event deduplication so alerts only trigger upon crossing thresholds
//!   or significant token escalations without turn-by-turn spam.
//! - **Pure ANSI Output**:
//!   Zero-dependency ANSI string formatters for standard terminal output, CLI commands, and REPLs.

use std::fmt;
use ratatui::{
    buffer::Buffer,
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Clear, Paragraph, Widget, Wrap},
    Frame,
};
use serde::{Deserialize, Serialize};

use crate::agent::session::Session;
use crate::agent::tokens::{
    format_token_count, model_context_limit, ContextBudget,
    DEFAULT_RESERVED_COMPLETION, DEFAULT_SAFETY_MARGIN,
};
use crate::ui::theme::Theme;

// ---------------------------------------------------------------------------
// 1. Constants & Default Thresholds
// ---------------------------------------------------------------------------

/// Default threshold for Notice alert (70% utilization).
pub const DEFAULT_NOTICE_THRESHOLD: f32 = 0.70;

/// Default threshold for Warning alert (80% utilization).
pub const DEFAULT_WARNING_THRESHOLD: f32 = 0.80;

/// Default threshold for Critical alert (90% utilization).
pub const DEFAULT_CRITICAL_THRESHOLD: f32 = 0.90;

/// Default threshold for Overflow alert (100% utilization).
pub const DEFAULT_OVERFLOW_THRESHOLD: f32 = 1.00;

/// Default meter width for ASCII/ANSI progress bars.
pub const DEFAULT_PROGRESS_BAR_WIDTH: usize = 28;

/// Default banner width in terminal columns.
pub const DEFAULT_BANNER_WIDTH: usize = 68;

/// Minimum safe width for rendering alert banners.
pub const MIN_BANNER_WIDTH: usize = 40;

/// Sub-character block characters for high-resolution progress meters.
pub const SUB_BLOCK_CHARS: &[&str] = &[" ", "▏", "▎", "▍", "▌", "▋", "▊", "▉", "█"];

// ANSI escape color sequences
const ANSI_RESET: &str = "\x1b[0m";
const ANSI_BOLD: &str = "\x1b[1m";
const ANSI_DIM: &str = "\x1b[2m";
const ANSI_ITALIC: &str = "\x1b[3m";
const ANSI_UNDERLINE: &str = "\x1b[4m";

const ANSI_RED: &str = "\x1b[31m";
const ANSI_BOLD_RED: &str = "\x1b[1;31m";
const ANSI_GREEN: &str = "\x1b[32m";
const ANSI_YELLOW: &str = "\x1b[33m";
const ANSI_BOLD_YELLOW: &str = "\x1b[1;33m";
const ANSI_BLUE: &str = "\x1b[34m";
const ANSI_MAGENTA: &str = "\x1b[35m";
const ANSI_CYAN: &str = "\x1b[36m";
const ANSI_BOLD_CYAN: &str = "\x1b[1;36m";
const ANSI_WHITE: &str = "\x1b[37m";
const ANSI_BOLD_WHITE: &str = "\x1b[1;37m";
const ANSI_GRAY: &str = "\x1b[90m";
const ANSI_LIGHT_GRAY: &str = "\x1b[37m";

const ANSI_BG_RED: &str = "\x1b[41;1;37m";
const ANSI_BG_YELLOW: &str = "\x1b[43;1;30m";
const ANSI_BG_MAGENTA: &str = "\x1b[45;1;37m";
const ANSI_BG_DARK_GRAY: &str = "\x1b[100;1;37m";

// ---------------------------------------------------------------------------
// 2. Alert Severity Classification
// ---------------------------------------------------------------------------

/// Severity level of the context window consumption.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum ContextAlertSeverity {
    /// Token usage is within safe operating margins (< 70%).
    Safe,
    /// Token usage is entering notice zone (70% - 79.9%).
    Notice,
    /// Token usage has reached or exceeded 80% (Compaction recommended).
    Warning,
    /// Token usage has reached or exceeded 90% (Truncation imminent).
    Critical,
    /// Token usage has reached or exceeded 100% (Context overflow).
    Overflow,
}

impl Default for ContextAlertSeverity {
    fn default() -> Self {
        Self::Safe
    }
}

impl ContextAlertSeverity {
    /// Classifies a utilization ratio according to provided thresholds.
    pub fn from_ratio(ratio: f32, notice: f32, warning: f32, critical: f32, overflow: f32) -> Self {
        if ratio >= overflow {
            Self::Overflow
        } else if ratio >= critical {
            Self::Critical
        } else if ratio >= warning {
            Self::Warning
        } else if ratio >= notice {
            Self::Notice
        } else {
            Self::Safe
        }
    }

    /// Classifies a utilization ratio using standard default thresholds.
    pub fn from_utilization(ratio: f32) -> Self {
        Self::from_ratio(
            ratio,
            DEFAULT_NOTICE_THRESHOLD,
            DEFAULT_WARNING_THRESHOLD,
            DEFAULT_CRITICAL_THRESHOLD,
            DEFAULT_OVERFLOW_THRESHOLD,
        )
    }

    /// Returns `true` if this severity warrants showing a visible warning alert.
    #[inline]
    pub fn is_alert(&self) -> bool {
        matches!(self, Self::Notice | Self::Warning | Self::Critical | Self::Overflow)
    }

    /// Returns `true` if at warning level or higher (>= 80%).
    #[inline]
    pub fn is_warning_or_worse(&self) -> bool {
        matches!(self, Self::Warning | Self::Critical | Self::Overflow)
    }

    /// Returns `true` if at critical level or higher (>= 90%).
    #[inline]
    pub fn is_critical_or_worse(&self) -> bool {
        matches!(self, Self::Critical | Self::Overflow)
    }

    /// Returns `true` if context has strictly overflowed (>= 100%).
    #[inline]
    pub fn is_overflow(&self) -> bool {
        matches!(self, Self::Overflow)
    }

    /// Human-readable label for the severity level.
    pub fn label(&self) -> &'static str {
        match self {
            Self::Safe => "SAFE",
            Self::Notice => "NOTICE",
            Self::Warning => "WARNING",
            Self::Critical => "CRITICAL",
            Self::Overflow => "OVERFLOW",
        }
    }

    /// Unicode icon representing the severity level.
    pub fn icon(&self) -> &'static str {
        match self {
            Self::Safe => "✓",
            Self::Notice => "ℹ",
            Self::Warning => "⚠",
            Self::Critical => "🚨",
            Self::Overflow => "⛔",
        }
    }

    /// ASCII-safe icon for non-Unicode environments.
    pub fn ascii_icon(&self) -> &'static str {
        match self {
            Self::Safe => "[OK]",
            Self::Notice => "[i]",
            Self::Warning => "[!]",
            Self::Critical => "[!!]",
            Self::Overflow => "[XX]",
        }
    }

    /// ANSI color escape sequence associated with this severity.
    pub fn ansi_color(&self) -> &'static str {
        match self {
            Self::Safe => ANSI_GREEN,
            Self::Notice => ANSI_CYAN,
            Self::Warning => ANSI_BOLD_YELLOW,
            Self::Critical => ANSI_BOLD_RED,
            Self::Overflow => ANSI_BG_RED,
        }
    }

    /// ANSI styled badge string (e.g. `[⚠ WARNING]`).
    pub fn ansi_badge(&self) -> String {
        match self {
            Self::Safe => format!("{}✓ SAFE{}", ANSI_GREEN, ANSI_RESET),
            Self::Notice => format!("{}ℹ NOTICE{}", ANSI_CYAN, ANSI_RESET),
            Self::Warning => format!("{}⚠ WARNING{}", ANSI_BOLD_YELLOW, ANSI_RESET),
            Self::Critical => format!("{}🚨 CRITICAL{}", ANSI_BOLD_RED, ANSI_RESET),
            Self::Overflow => format!("{}⛔ OVERFLOW{}", ANSI_BG_RED, ANSI_RESET),
        }
    }

    /// Ratatui [`Color`] representing this severity.
    pub fn ratatui_color(&self) -> Color {
        match self {
            Self::Safe => Color::Green,
            Self::Notice => Color::Cyan,
            Self::Warning => Color::Yellow,
            Self::Critical => Color::Red,
            Self::Overflow => Color::LightRed,
        }
    }

    /// Ratatui [`Style`] representing this severity.
    pub fn ratatui_style(&self) -> Style {
        match self {
            Self::Safe => Style::default().fg(Color::Green),
            Self::Notice => Style::default().fg(Color::Cyan),
            Self::Warning => Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD),
            Self::Critical => Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
            Self::Overflow => Style::default()
                .fg(Color::White)
                .bg(Color::Red)
                .add_modifier(Modifier::BOLD),
        }
    }

    /// Numeric priority for sorting (higher = more urgent).
    pub fn priority(&self) -> u8 {
        match self {
            Self::Safe => 0,
            Self::Notice => 1,
            Self::Warning => 2,
            Self::Critical => 3,
            Self::Overflow => 4,
        }
    }
}

impl fmt::Display for ContextAlertSeverity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.label())
    }
}

// ---------------------------------------------------------------------------
// 3. Progress Bar Styles & Unicode Block Engines
// ---------------------------------------------------------------------------

/// Visual style configuration for percentage and progress meters.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ProgressBarStyle {
    /// High-resolution Unicode sub-blocks (`█`, `▉`, `▊`, `▋`, `▌`, `▍`, `▎`, `▏`, empty ` `).
    UnicodeSubBlock,
    /// Solid full blocks with light shade background (`█` / `░`).
    UnicodeBlock,
    /// Shaded intensity gradient (`▓` / `▒` / `░`).
    Shaded,
    /// Standard ASCII characters (`#` / `-`).
    Ascii,
    /// Braille dots matrix (`⣿`, `⣷`, `⣶`, `⣦`, `⣤`, `⣄`, `⣀`, ` `).
    Braille,
    /// Double-line bar (`═` / `─`).
    DoubleLine,
}

impl Default for ProgressBarStyle {
    fn default() -> Self {
        Self::UnicodeSubBlock
    }
}

/// Renders a plain progress bar string at a given utilization (0.0 to 1.0+) and width.
pub fn render_progress_bar_text(utilization: f32, width: usize, style: ProgressBarStyle) -> String {
    if width == 0 {
        return String::new();
    }

    let clamped_util = utilization.max(0.0);

    match style {
        ProgressBarStyle::UnicodeSubBlock => {
            let total_steps = width * 8;
            let active_steps = ((clamped_util * total_steps as f32).round() as usize).min(total_steps);
            let full_blocks = active_steps / 8;
            let remainder = active_steps % 8;

            let mut out = String::with_capacity(width * 4);
            for _ in 0..full_blocks {
                out.push('█');
            }

            if full_blocks < width {
                if remainder > 0 {
                    out.push_str(SUB_BLOCK_CHARS[remainder]);
                    for _ in (full_blocks + 1)..width {
                        out.push('░');
                    }
                } else {
                    for _ in full_blocks..width {
                        out.push('░');
                    }
                }
            }
            out
        }
        ProgressBarStyle::UnicodeBlock => {
            let filled = ((clamped_util * width as f32).round() as usize).min(width);
            let empty = width.saturating_sub(filled);
            let mut out = String::with_capacity(width * 3);
            for _ in 0..filled {
                out.push('█');
            }
            for _ in 0..empty {
                out.push('░');
            }
            out
        }
        ProgressBarStyle::Shaded => {
            let filled = ((clamped_util * width as f32).round() as usize).min(width);
            let empty = width.saturating_sub(filled);
            let mut out = String::with_capacity(width * 3);
            for _ in 0..filled {
                out.push('▓');
            }
            for _ in 0..empty {
                out.push('░');
            }
            out
        }
        ProgressBarStyle::Ascii => {
            let filled = ((clamped_util * width as f32).round() as usize).min(width);
            let empty = width.saturating_sub(filled);
            let mut out = String::with_capacity(width);
            for _ in 0..filled {
                out.push('#');
            }
            for _ in 0..empty {
                out.push('-');
            }
            out
        }
        ProgressBarStyle::Braille => {
            let filled = ((clamped_util * width as f32).round() as usize).min(width);
            let empty = width.saturating_sub(filled);
            let mut out = String::with_capacity(width * 3);
            for _ in 0..filled {
                out.push('⣿');
            }
            for _ in 0..empty {
                out.push('⣀');
            }
            out
        }
        ProgressBarStyle::DoubleLine => {
            let filled = ((clamped_util * width as f32).round() as usize).min(width);
            let empty = width.saturating_sub(filled);
            let mut out = String::with_capacity(width * 3);
            for _ in 0..filled {
                out.push('═');
            }
            for _ in 0..empty {
                out.push('─');
            }
            out
        }
    }
}

/// Renders a colorized ANSI progress bar with percentage and optional bounding brackets.
pub fn render_progress_bar_ansi(
    utilization: f32,
    width: usize,
    severity: ContextAlertSeverity,
    style: ProgressBarStyle,
    show_percentage: bool,
) -> String {
    let bar_str = render_progress_bar_text(utilization, width, style);
    let color = severity.ansi_color();
    let pct_str = format!("{:.1}%", (utilization * 100.0).max(0.0));

    if show_percentage {
        format!(
            "{}{}[{}]{}{} {}{}{}",
            ANSI_GRAY,
            color,
            bar_str,
            ANSI_RESET,
            ANSI_GRAY,
            color,
            pct_str,
            ANSI_RESET
        )
    } else {
        format!("{}{}[{}]{}", color, ANSI_BOLD, bar_str, ANSI_RESET)
    }
}

// ---------------------------------------------------------------------------
// 4. Border Styles & Box Drawing
// ---------------------------------------------------------------------------

/// Box-drawing border glyphs for alert banners and cards.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum WarningBorderStyle {
    /// Smooth rounded Unicode corners (`╭─╮`, `│ │`, `╰─╯`).
    Rounded,
    /// Thick heavy Unicode borders (`┏━┓`, `┃ ┃`, `┗━┛`).
    Heavy,
    /// Double-line Unicode borders (`╔═╗`, `║ ║`, `╚═╝`).
    Double,
    /// Standard single-line Unicode borders (`┌─┐`, `│ │`, `└─┘`).
    Single,
    /// Pure ASCII borders (`+-+`, `| |`, `+-+`).
    Ascii,
}

impl Default for WarningBorderStyle {
    fn default() -> Self {
        Self::Rounded
    }
}

impl WarningBorderStyle {
    pub fn top_left(&self) -> char {
        match self {
            Self::Rounded => '╭',
            Self::Heavy => '┏',
            Self::Double => '╔',
            Self::Single => '┌',
            Self::Ascii => '+',
        }
    }

    pub fn top_right(&self) -> char {
        match self {
            Self::Rounded => '╮',
            Self::Heavy => '┓',
            Self::Double => '╗',
            Self::Single => '┐',
            Self::Ascii => '+',
        }
    }

    pub fn bottom_left(&self) -> char {
        match self {
            Self::Rounded => '╰',
            Self::Heavy => '┗',
            Self::Double => '╚',
            Self::Single => '└',
            Self::Ascii => '+',
        }
    }

    pub fn bottom_right(&self) -> char {
        match self {
            Self::Rounded => '╯',
            Self::Heavy => '┛',
            Self::Double => '╝',
            Self::Single => '┘',
            Self::Ascii => '+',
        }
    }

    pub fn horizontal(&self) -> char {
        match self {
            Self::Rounded | Self::Single => '─',
            Self::Heavy => '━',
            Self::Double => '═',
            Self::Ascii => '-',
        }
    }

    pub fn vertical(&self) -> char {
        match self {
            Self::Rounded | Self::Single => '│',
            Self::Heavy => '┃',
            Self::Double => '║',
            Self::Ascii => '|',
        }
    }

    pub fn divider_left(&self) -> char {
        match self {
            Self::Rounded | Self::Single => '├',
            Self::Heavy => '┣',
            Self::Double => '╠',
            Self::Ascii => '+',
        }
    }

    pub fn divider_right(&self) -> char {
        match self {
            Self::Rounded | Self::Single => '┤',
            Self::Heavy => '┫',
            Self::Double => '╣',
            Self::Ascii => '+',
        }
    }

    pub fn to_ratatui_border_type(&self) -> BorderType {
        match self {
            Self::Rounded => BorderType::Rounded,
            Self::Heavy => BorderType::Thick,
            Self::Double => BorderType::Double,
            Self::Single => BorderType::Plain,
            Self::Ascii => BorderType::Plain,
        }
    }
}

// ---------------------------------------------------------------------------
// 5. Display Modes & Visual Styles
// ---------------------------------------------------------------------------

/// Visual display format for the context warning widget.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ContextWarningStyle {
    /// Full multi-line bordered alert banner with title, progress meter, and suggestions.
    Banner,
    /// Compact 1-2 line inline alert banner for prompts or status bars.
    Compact,
    /// Minimal badge / pill for prompt lines (e.g. `[⚠ 88.4% | 14.8k rem]`).
    Pill,
    /// Detailed modal-style card with breakdown and turns estimation.
    Card,
    /// Standalone progress bar meter with percentage and remaining token counts.
    MiniBar,
}

impl Default for ContextWarningStyle {
    fn default() -> Self {
        Self::Banner
    }
}

// ---------------------------------------------------------------------------
// 6. Context Limit Alert Data Structure
// ---------------------------------------------------------------------------

/// Complete context limit evaluation result containing token counts, remaining capacity,
/// utilization ratio, and contextual recommendations.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ContextLimitAlert {
    /// Total tokens currently consumed in the active conversation context.
    pub used_tokens: usize,
    /// Maximum context window limit of the active LLM (e.g. 128,000 or 200,000).
    pub max_context_tokens: usize,
    /// Name or identifier of the active model (e.g. "claude-3-7-sonnet", "gpt-4o").
    pub model: String,
    /// Reserved token headroom for LLM response completions (default ~4,096).
    pub reserved_completion: usize,
    /// Additional safety margin tokens (default ~1,024).
    pub safety_margin: usize,
    /// Optional token breakdown for system prompts.
    pub system_tokens: Option<usize>,
    /// Optional token breakdown for conversation message history.
    pub messages_tokens: Option<usize>,
    /// Optional token breakdown for registered tool schemas.
    pub tools_tokens: Option<usize>,
    /// Estimated turns remaining before context is exhausted at current rate.
    pub estimated_remaining_turns: Option<usize>,
    /// Estimated session cost in USD if available.
    pub cost_estimate_usd: Option<f64>,
    /// Evaluated severity classification.
    pub severity: ContextAlertSeverity,
    /// Context utilization ratio (`0.0` to `1.0+`).
    pub utilization: f32,
    /// Remaining prompt tokens before reaching available capacity.
    /// Can be negative if overflowing.
    pub remaining_tokens: isize,
    /// Effective remaining tokens clamped at 0.
    pub effective_remaining_tokens: usize,
}

impl ContextLimitAlert {
    /// Creates a new `ContextLimitAlert` evaluated against the given token counts and model.
    pub fn new(used_tokens: usize, max_context_tokens: usize, model: impl Into<String>) -> Self {
        let model_str = model.into();
        let max = if max_context_tokens == 0 {
            model_context_limit(&model_str)
        } else {
            max_context_tokens
        };

        let reserved = DEFAULT_RESERVED_COMPLETION;
        let safety = DEFAULT_SAFETY_MARGIN;

        let available_prompt_tokens = max.saturating_sub(reserved).saturating_sub(safety);
        let remaining_tokens = available_prompt_tokens as isize - used_tokens as isize;
        let effective_remaining = remaining_tokens.max(0) as usize;

        let utilization = if max > 0 {
            used_tokens as f32 / max as f32
        } else {
            1.0
        };

        let severity = ContextAlertSeverity::from_utilization(utilization);

        let estimated_turns = if remaining_tokens > 0 {
            // Assume average turn cost is ~1,500 tokens (user query + assistant response)
            let avg_turn_cost = 1500;
            Some((effective_remaining / avg_turn_cost).max(1))
        } else {
            Some(0)
        };

        Self {
            used_tokens,
            max_context_tokens: max,
            model: model_str,
            reserved_completion: reserved,
            safety_margin: safety,
            system_tokens: None,
            messages_tokens: None,
            tools_tokens: None,
            estimated_remaining_turns: estimated_turns,
            cost_estimate_usd: None,
            severity,
            utilization,
            remaining_tokens,
            effective_remaining_tokens: effective_remaining,
        }
    }

    /// Evaluates context limits from raw token numbers with optional model name.
    pub fn from_tokens(used_tokens: usize, limit: usize, model: Option<&str>) -> Self {
        let model_name = model.unwrap_or("default");
        Self::new(used_tokens, limit, model_name)
    }

    /// Evaluates context limits directly from an active `Session` and model name.
    pub fn from_session(session: &Session, model: &str) -> Self {
        let max = model_context_limit(model);
        let used = session.estimate_tokens();

        let mut alert = Self::new(used, max, model);

        // Populate breakdown if message history is accessible
        if !session.messages.is_empty() {
            let msg_tokens: usize = session
                .messages
                .iter()
                .map(|m| crate::agent::tokens::estimate_message_tokens(m))
                .sum();
            alert.messages_tokens = Some(msg_tokens);
        }

        alert
    }

    /// Evaluates context limits from a `ContextBudget` and current token count.
    pub fn from_budget(budget: &ContextBudget, current_tokens: usize) -> Self {
        let max = budget.max_context_tokens;
        let reserved = budget.reserved_completion_tokens;
        let safety = budget.safety_margin_tokens;

        let available = max.saturating_sub(reserved).saturating_sub(safety);
        let remaining = available as isize - current_tokens as isize;
        let effective = remaining.max(0) as usize;

        let utilization = if max > 0 {
            current_tokens as f32 / max as f32
        } else {
            1.0
        };

        let severity = ContextAlertSeverity::from_ratio(
            utilization,
            DEFAULT_NOTICE_THRESHOLD,
            budget.warning_threshold,
            budget.danger_threshold,
            DEFAULT_OVERFLOW_THRESHOLD,
        );

        Self {
            used_tokens: current_tokens,
            max_context_tokens: max,
            model: budget.model.clone(),
            reserved_completion: reserved,
            safety_margin: safety,
            system_tokens: None,
            messages_tokens: None,
            tools_tokens: None,
            estimated_remaining_turns: Some((effective / 1500).max(if remaining > 0 { 1 } else { 0 })),
            cost_estimate_usd: None,
            severity,
            utilization,
            remaining_tokens: remaining,
            effective_remaining_tokens: effective,
        }
    }

    // Builder pattern modifiers

    pub fn with_reserved_completion(mut self, reserved: usize) -> Self {
        self.reserved_completion = reserved;
        self.recompute();
        self
    }

    pub fn with_safety_margin(mut self, margin: usize) -> Self {
        self.safety_margin = margin;
        self.recompute();
        self
    }

    pub fn with_system_tokens(mut self, tokens: usize) -> Self {
        self.system_tokens = Some(tokens);
        self
    }

    pub fn with_messages_tokens(mut self, tokens: usize) -> Self {
        self.messages_tokens = Some(tokens);
        self
    }

    pub fn with_tools_tokens(mut self, tokens: usize) -> Self {
        self.tools_tokens = Some(tokens);
        self
    }

    pub fn with_estimated_turns(mut self, turns: usize) -> Self {
        self.estimated_remaining_turns = Some(turns);
        self
    }

    pub fn with_cost_estimate(mut self, cost_usd: f64) -> Self {
        self.cost_estimate_usd = Some(cost_usd);
        self
    }

    fn recompute(&mut self) {
        let available = self
            .max_context_tokens
            .saturating_sub(self.reserved_completion)
            .saturating_sub(self.safety_margin);
        self.remaining_tokens = available as isize - self.used_tokens as isize;
        self.effective_remaining_tokens = self.remaining_tokens.max(0) as usize;
    }

    // Inspection & Formatting Helpers

    #[inline]
    pub fn is_alert(&self) -> bool {
        self.severity.is_alert()
    }

    #[inline]
    pub fn is_warning(&self) -> bool {
        self.severity.is_warning_or_worse()
    }

    #[inline]
    pub fn is_critical(&self) -> bool {
        self.severity.is_critical_or_worse()
    }

    #[inline]
    pub fn is_overflow(&self) -> bool {
        self.severity.is_overflow()
    }

    #[inline]
    pub fn is_safe(&self) -> bool {
        self.severity == ContextAlertSeverity::Safe
    }

    /// Formatted percentage string (e.g. `"85.4%"`).
    pub fn percentage_str(&self) -> String {
        format!("{:.1}%", (self.utilization * 100.0).max(0.0))
    }

    /// Formatted current token count (e.g. `"108.8k"`).
    pub fn formatted_used(&self) -> String {
        format_token_count(self.used_tokens)
    }

    /// Formatted maximum context window limit (e.g. `"128k"`).
    pub fn formatted_max(&self) -> String {
        format_token_count(self.max_context_tokens)
    }

    /// Formatted remaining token headroom (e.g. `"19.2k"`).
    pub fn formatted_remaining(&self) -> String {
        if self.remaining_tokens >= 0 {
            format_token_count(self.remaining_tokens as usize)
        } else {
            format!("-{}", format_token_count((-self.remaining_tokens) as usize))
        }
    }

    /// Primary recommended action based on severity.
    pub fn primary_action(&self) -> &'static str {
        match self.severity {
            ContextAlertSeverity::Safe => "Normal operation",
            ContextAlertSeverity::Notice => "Monitor context growth",
            ContextAlertSeverity::Warning => "Run `/compact` or summarize history",
            ContextAlertSeverity::Critical => "Immediate `/compact` required",
            ContextAlertSeverity::Overflow => "Context exceeded: `/compact` or `/clear` now",
        }
    }

    /// Compact keyboard shortcut hint (e.g. `"/compact"`).
    pub fn action_hint(&self) -> &'static str {
        match self.severity {
            ContextAlertSeverity::Safe => "",
            ContextAlertSeverity::Notice => "tip: /compact",
            ContextAlertSeverity::Warning => "press /compact",
            ContextAlertSeverity::Critical => "URGENT: /compact",
            ContextAlertSeverity::Overflow => "CRITICAL: /compact or /clear",
        }
    }

    /// List of actionable recommendations for the user.
    pub fn recommendations(&self) -> Vec<&'static str> {
        match self.severity {
            ContextAlertSeverity::Safe => vec![
                "Context budget is healthy (< 70% capacity).",
                "No pruning or compaction necessary.",
            ],
            ContextAlertSeverity::Notice => vec![
                "Context headroom is narrowing (70% - 79%).",
                "Consider `/compact` after completing the current task.",
                "Review active tool definitions if tokens grow rapidly.",
            ],
            ContextAlertSeverity::Warning => vec![
                "Run `/compact` to summarize conversation history and reclaim 60%+ headroom.",
                "Clear temporary tool outputs or unnecessary file attachments.",
                "Switch to a larger context model (e.g. Claude 3.7 or Gemini 2.5) if needed.",
            ],
            ContextAlertSeverity::Critical => vec![
                "IMMEDIATE ACTION: Run `/compact` to avoid prompt truncation or turn failures.",
                "Drop older turns with `/drop <n>` or prune history.",
                "Save session state with `/export` before continuing.",
            ],
            ContextAlertSeverity::Overflow => vec![
                "CONTEXT EXCEEDED: The prompt has overflowed the model's physical window.",
                "Execute `/compact` immediately to restore session operation.",
                "Alternatively run `/clear` or `/new` to start a clean session context.",
            ],
        }
    }

    /// Header title for the alert widget.
    pub fn headline(&self) -> String {
        match self.severity {
            ContextAlertSeverity::Safe => "Context Window: Optimal Margin".to_string(),
            ContextAlertSeverity::Notice => "Context Window: Headroom Notice".to_string(),
            ContextAlertSeverity::Warning => {
                format!("Context Window Warning ({} Capacity)", self.percentage_str())
            }
            ContextAlertSeverity::Critical => {
                format!("🚨 Critical Context Limit Approaching ({})", self.percentage_str())
            }
            ContextAlertSeverity::Overflow => {
                format!("⛔ Context Window Overflow ({})", self.percentage_str())
            }
        }
    }

    /// Formatted badge string suitable for terminal status bars.
    pub fn badge_text(&self) -> String {
        format!(
            "{} {} / {} ({} | {} rem)",
            self.severity.icon(),
            self.formatted_used(),
            self.formatted_max(),
            self.percentage_str(),
            self.formatted_remaining(),
        )
    }

    /// Formats a single-line summary string.
    pub fn summary_line(&self) -> String {
        format!(
            "Context: {} / {} tokens ({}) │ Remaining: {} │ Model: {}",
            self.formatted_used(),
            self.formatted_max(),
            self.percentage_str(),
            self.formatted_remaining(),
            self.model
        )
    }
}

// ---------------------------------------------------------------------------
// 7. Context Warning Tracker (Hysteresis & Deduplication)
// ---------------------------------------------------------------------------

/// Event record of an alert triggered during a session.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ContextAlertEvent {
    pub turn_index: usize,
    pub used_tokens: usize,
    pub severity: ContextAlertSeverity,
    pub timestamp_epoch_secs: u64,
}

/// Stateful tracker managing context alerts across conversation turns,
/// implementing threshold hysteresis to prevent oscillating alert spam.
#[derive(Debug, Clone)]
pub struct ContextWarningTracker {
    /// Highest severity level seen in the current escalation cycle.
    pub last_severity: ContextAlertSeverity,
    /// Token count at which the last alert was emitted.
    pub last_warned_tokens: usize,
    /// Turn index when the last alert was emitted.
    pub last_warned_turn: usize,
    /// Hysteresis ratio to prevent alert flap (e.g. 0.03 = 3% delta required to re-trigger).
    pub hysteresis_ratio: f32,
    /// Whether user has explicitly dismissed alerts for the current severity tier.
    pub dismissed: bool,
    /// History log of alerts fired.
    pub alert_history: Vec<ContextAlertEvent>,
    /// Configured thresholds.
    pub notice_threshold: f32,
    pub warning_threshold: f32,
    pub critical_threshold: f32,
    pub overflow_threshold: f32,
}

impl Default for ContextWarningTracker {
    fn default() -> Self {
        Self {
            last_severity: ContextAlertSeverity::Safe,
            last_warned_tokens: 0,
            last_warned_turn: 0,
            hysteresis_ratio: 0.03,
            dismissed: false,
            alert_history: Vec::new(),
            notice_threshold: DEFAULT_NOTICE_THRESHOLD,
            warning_threshold: DEFAULT_WARNING_THRESHOLD,
            critical_threshold: DEFAULT_CRITICAL_THRESHOLD,
            overflow_threshold: DEFAULT_OVERFLOW_THRESHOLD,
        }
    }
}

impl ContextWarningTracker {
    pub fn new() -> Self {
        Self::default()
    }

    /// Evaluates current token usage and returns `Some(ContextLimitAlert)` if a new alert
    /// condition is met or severity escalates.
    pub fn evaluate(
        &mut self,
        used_tokens: usize,
        max_tokens: usize,
        model: &str,
        turn_index: usize,
    ) -> Option<ContextLimitAlert> {
        let max = if max_tokens == 0 {
            model_context_limit(model)
        } else {
            max_tokens
        };

        let utilization = if max > 0 {
            used_tokens as f32 / max as f32
        } else {
            1.0
        };

        let current_severity = ContextAlertSeverity::from_ratio(
            utilization,
            self.notice_threshold,
            self.warning_threshold,
            self.critical_threshold,
            self.overflow_threshold,
        );

        let mut alert = ContextLimitAlert::new(used_tokens, max, model);
        alert.severity = current_severity;

        // If context dropped back down to Safe (e.g. after /compact or /clear), reset state
        if current_severity == ContextAlertSeverity::Safe {
            self.last_severity = ContextAlertSeverity::Safe;
            self.dismissed = false;
            return None;
        }

        // Escalation condition: current severity is strictly higher than last warned
        let is_escalation = current_severity > self.last_severity;

        // Growth condition: tokens increased significantly (> hysteresis threshold)
        let token_delta = used_tokens.saturating_sub(self.last_warned_tokens);
        let delta_ratio = if max > 0 {
            token_delta as f32 / max as f32
        } else {
            0.0
        };
        let is_significant_growth = delta_ratio >= self.hysteresis_ratio;

        let should_trigger = (is_escalation || is_significant_growth)
            && (!self.dismissed || is_escalation);

        if should_trigger {
            self.last_severity = current_severity;
            self.last_warned_tokens = used_tokens;
            self.last_warned_turn = turn_index;
            self.dismissed = false;

            self.alert_history.push(ContextAlertEvent {
                turn_index,
                used_tokens,
                severity: current_severity,
                timestamp_epoch_secs: std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_secs(),
            });

            Some(alert)
        } else {
            None
        }
    }

    /// Evaluates without mutating tracker state.
    pub fn check_peek(&self, used_tokens: usize, max_tokens: usize, model: &str) -> ContextLimitAlert {
        ContextLimitAlert::new(used_tokens, max_tokens, model)
    }

    /// Dismisses the active alert for the current severity level.
    pub fn dismiss(&mut self) {
        self.dismissed = true;
    }

    /// Resets the tracker to initial safe state.
    pub fn reset(&mut self) {
        self.last_severity = ContextAlertSeverity::Safe;
        self.last_warned_tokens = 0;
        self.last_warned_turn = 0;
        self.dismissed = false;
        self.alert_history.clear();
    }
}

// ---------------------------------------------------------------------------
// 8. Standalone ANSI String Formatters
// ---------------------------------------------------------------------------

/// Renders a full multi-line ANSI alert banner with border box, meter, and actions.
pub fn render_warning_banner_ansi(
    alert: &ContextLimitAlert,
    banner_width: usize,
    border_style: WarningBorderStyle,
) -> String {
    let width = banner_width.max(MIN_BANNER_WIDTH);
    let inner_width = width.saturating_sub(4); // 2 borders + 2 padding spaces
    let border_color = alert.severity.ansi_color();
    let horiz = border_style.horizontal();
    let vert = border_style.vertical();

    let mut out = String::with_capacity(1024);

    let render_row = |content: &str| -> String {
        let stripped = strip_ansi(content);
        let visible_len = stripped.chars().count();
        let pad = inner_width.saturating_sub(visible_len);
        format!(
            "{}{}{} {} {}{}{}{}\n",
            border_color,
            vert,
            ANSI_RESET,
            content,
            " ".repeat(pad),
            border_color,
            vert,
            ANSI_RESET
        )
    };

    // 1. Top border with title badge
    let badge_content = format!(" {} {} ", alert.severity.icon(), alert.severity.label());
    let badge_len = badge_content.chars().count();
    let rem_border = width.saturating_sub(2).saturating_sub(badge_len);
    let left_border = 3;
    let right_border = rem_border.saturating_sub(left_border);

    out.push_str(border_color);
    out.push(border_style.top_left());
    for _ in 0..left_border {
        out.push(horiz);
    }
    out.push_str(ANSI_BOLD);
    out.push_str(&badge_content);
    out.push_str(ANSI_RESET);
    out.push_str(border_color);
    for _ in 0..right_border {
        out.push(horiz);
    }
    out.push(border_style.top_right());
    out.push_str(ANSI_RESET);
    out.push('\n');

    // 2. Headline & Model Line
    let model_tag = format!("{}Model: {}{}{}", ANSI_GRAY, ANSI_BOLD_CYAN, alert.model, ANSI_RESET);
    let headline_str = format!("{}{}{}", ANSI_BOLD, alert.headline(), ANSI_RESET);
    out.push_str(&render_row(&headline_str));
    out.push_str(&render_row(&model_tag));
    out.push_str(&render_row(""));

    // 3. High-resolution Progress Meter Bar
    let meter_bar_width = inner_width.saturating_sub(24).clamp(12, 36);
    let progress_bar_str = render_progress_bar_ansi(
        alert.utilization,
        meter_bar_width,
        alert.severity,
        ProgressBarStyle::UnicodeSubBlock,
        true,
    );
    let meter_line = format!("Capacity: {}", progress_bar_str);
    out.push_str(&render_row(&meter_line));

    // 4. Token Metrics Grid
    let metrics_line = format!(
        "Used: {}{}{} / {}  │  Remaining: {}{}{}",
        alert.severity.ansi_color(),
        alert.formatted_used(),
        ANSI_RESET,
        alert.formatted_max(),
        if alert.remaining_tokens >= 0 {
            ANSI_BOLD_WHITE
        } else {
            ANSI_BOLD_RED
        },
        alert.formatted_remaining(),
        ANSI_RESET
    );
    out.push_str(&render_row(&metrics_line));

    // 5. Granular Breakdown if available
    if let (Some(sys), Some(msgs), Some(tools)) = (alert.system_tokens, alert.messages_tokens, alert.tools_tokens) {
        let breakdown_line = format!(
            "{}Breakdown: Sys: {}  │  Msgs: {}  │  Tools: {}{}",
            ANSI_GRAY,
            format_token_count(sys),
            format_token_count(msgs),
            format_token_count(tools),
            ANSI_RESET
        );
        out.push_str(&render_row(&breakdown_line));
    }

    // 6. Turns remaining estimate
    if let Some(turns) = alert.estimated_remaining_turns {
        let turns_line = if turns > 0 {
            format!("{}Est. Turns Left: ~{} turns before limit{}", ANSI_GRAY, turns, ANSI_RESET)
        } else {
            format!("{}Est. Turns Left: 0 turns (Limit exceeded){}", ANSI_BOLD_RED, ANSI_RESET)
        };
        out.push_str(&render_row(&turns_line));
    }

    // 7. Divider before suggestions
    out.push_str(border_color);
    out.push(border_style.divider_left());
    for _ in 0..width.saturating_sub(2) {
        out.push(horiz);
    }
    out.push(border_style.divider_right());
    out.push_str(ANSI_RESET);
    out.push('\n');

    // 8. Actionable recommendations
    let rec_header = format!("{}Suggested Actions:{}", ANSI_BOLD, ANSI_RESET);
    out.push_str(&render_row(&rec_header));
    for rec in alert.recommendations() {
        let rec_line = format!("  • {}", rec);
        out.push_str(&render_row(&rec_line));
    }

    // 9. Bottom border
    out.push_str(border_color);
    out.push(border_style.bottom_left());
    for _ in 0..width.saturating_sub(2) {
        out.push(horiz);
    }
    out.push(border_style.bottom_right());
    out.push_str(ANSI_RESET);
    out.push('\n');

    out
}

/// Renders a compact 2-line ANSI alert banner suitable for message streams or footers.
pub fn render_warning_compact_ansi(alert: &ContextLimitAlert) -> String {
    let icon = alert.severity.icon();
    let badge = alert.severity.ansi_badge();
    let pct = alert.percentage_str();
    let used = alert.formatted_used();
    let max = alert.formatted_max();
    let remaining = alert.formatted_remaining();
    let model = &alert.model;
    let hint = alert.action_hint();

    let bar = render_progress_bar_ansi(
        alert.utilization,
        18,
        alert.severity,
        ProgressBarStyle::UnicodeSubBlock,
        false,
    );

    let line1 = format!(
        "{} {} [{}] {} / {} ({}) │ Rem: {} │ {}",
        icon, badge, model, used, max, pct, remaining, bar
    );

    let line2 = if !hint.is_empty() {
        format!("  {}Action: {}{}", ANSI_DIM, hint, ANSI_RESET)
    } else {
        String::new()
    };

    if line2.is_empty() {
        line1
    } else {
        format!("{}\n{}", line1, line2)
    }
}

/// Renders an ultra-compact inline pill / badge string (e.g. `[⚠ 88.4% | 14.8k rem]`).
pub fn render_warning_pill_ansi(alert: &ContextLimitAlert) -> String {
    let color = alert.severity.ansi_color();
    let icon = alert.severity.icon();
    let pct = alert.percentage_str();
    let remaining = alert.formatted_remaining();

    format!(
        "{}[{} {} | {} rem]{}",
        color, icon, pct, remaining, ANSI_RESET
    )
}

/// Strips all ANSI escape codes from a string for length calculation.
pub fn strip_ansi(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\x1b' {
            if chars.peek() == Some(&'[') {
                chars.next(); // consume '['
                while let Some(&next) = chars.peek() {
                    chars.next();
                    if next.is_ascii_alphabetic() || next == 'm' || next == 'K' || next == 'H' || next == 'J' {
                        break;
                    }
                }
            }
        } else {
            out.push(c);
        }
    }
    out
}

/// Renders a floating modal card ANSI representation.
pub fn render_warning_card_ansi(
    alert: &ContextLimitAlert,
    width: usize,
    border_style: WarningBorderStyle,
) -> String {
    render_warning_banner_ansi(alert, width, border_style)
}

// ---------------------------------------------------------------------------
// 9. Ratatui Widgets
// ---------------------------------------------------------------------------

/// Ratatui widget displaying a full context limit warning alert dialog,
/// banner, pill, or compact card.
#[derive(Debug, Clone)]
pub struct ContextWarningWidget<'a> {
    alert: &'a ContextLimitAlert,
    style_mode: ContextWarningStyle,
    border_style: WarningBorderStyle,
    theme: Option<&'a Theme>,
    custom_title: Option<String>,
    show_recommendations: bool,
    show_breakdown: bool,
    show_turns_estimate: bool,
    show_actions_hint: bool,
}

impl<'a> ContextWarningWidget<'a> {
    /// Creates a new `ContextWarningWidget` for the specified alert data.
    pub fn new(alert: &'a ContextLimitAlert) -> Self {
        Self {
            alert,
            style_mode: ContextWarningStyle::Banner,
            border_style: WarningBorderStyle::Rounded,
            theme: None,
            custom_title: None,
            show_recommendations: true,
            show_breakdown: true,
            show_turns_estimate: true,
            show_actions_hint: true,
        }
    }

    /// Sets the display style (Banner, Compact, Pill, Card, MiniBar).
    pub fn style_mode(mut self, mode: ContextWarningStyle) -> Self {
        self.style_mode = mode;
        self
    }

    /// Sets the border glyph style.
    pub fn border_style(mut self, border: WarningBorderStyle) -> Self {
        self.border_style = border;
        self
    }

    /// Attaches an active `Theme` for color matching.
    pub fn theme(mut self, theme: &'a Theme) -> Self {
        self.theme = Some(theme);
        self
    }

    /// Overrides the alert widget title.
    pub fn title(mut self, title: impl Into<String>) -> Self {
        self.custom_title = Some(title.into());
        self
    }

    /// Toggles showing action recommendations.
    pub fn show_recommendations(mut self, show: bool) -> Self {
        self.show_recommendations = show;
        self
    }

    /// Toggles showing breakdown lines.
    pub fn show_breakdown(mut self, show: bool) -> Self {
        self.show_breakdown = show;
        self
    }

    /// Toggles showing turn estimates.
    pub fn show_turns_estimate(mut self, show: bool) -> Self {
        self.show_turns_estimate = show;
        self
    }

    /// Toggles showing action hints.
    pub fn show_actions_hint(mut self, show: bool) -> Self {
        self.show_actions_hint = show;
        self
    }

    // Internal Renderers for Ratatui

    fn render_banner(&self, area: Rect, buf: &mut Buffer) {
        if area.width < 4 || area.height < 3 {
            return;
        }

        let alert_color = self.alert.severity.ratatui_color();
        let border_type = self.border_style.to_ratatui_border_type();

        let title_text = self
            .custom_title
            .clone()
            .unwrap_or_else(|| format!(" {} {} ", self.alert.severity.icon(), self.alert.headline()));

        let block = Block::default()
            .borders(Borders::ALL)
            .border_type(border_type)
            .border_style(Style::default().fg(alert_color))
            .title(Span::styled(
                title_text,
                Style::default().fg(alert_color).add_modifier(Modifier::BOLD),
            ));

        let inner = block.inner(area);
        block.render(area, buf);

        if inner.height == 0 || inner.width == 0 {
            return;
        }

        let mut lines = Vec::new();

        // 1. Model and Status row
        lines.push(Line::from(vec![
            Span::styled("Model: ", Style::default().fg(Color::DarkGray)),
            Span::styled(&self.alert.model, Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)),
            Span::raw("  │  "),
            Span::styled("Capacity: ", Style::default().fg(Color::DarkGray)),
            Span::styled(
                self.alert.percentage_str(),
                Style::default().fg(alert_color).add_modifier(Modifier::BOLD),
            ),
        ]));

        // 2. Visual Progress Bar Row
        let bar_width = (inner.width.saturating_sub(18) as usize).clamp(8, 40);
        let bar_str = render_progress_bar_text(
            self.alert.utilization,
            bar_width,
            ProgressBarStyle::UnicodeSubBlock,
        );

        lines.push(Line::from(vec![
            Span::styled("Usage: [", Style::default().fg(Color::DarkGray)),
            Span::styled(bar_str, Style::default().fg(alert_color)),
            Span::styled("] ", Style::default().fg(Color::DarkGray)),
            Span::styled(
                format!("{}/{}", self.alert.formatted_used(), self.alert.formatted_max()),
                Style::default().fg(Color::White),
            ),
        ]));

        // 3. Remaining Tokens & Turns Row
        let remaining_style = if self.alert.remaining_tokens >= 0 {
            Style::default().fg(Color::Green).add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(Color::Red).add_modifier(Modifier::BOLD)
        };

        let mut rem_spans = vec![
            Span::styled("Remaining: ", Style::default().fg(Color::DarkGray)),
            Span::styled(
                format!("{} tokens", self.alert.formatted_remaining()),
                remaining_style,
            ),
        ];

        if self.show_turns_estimate {
            if let Some(turns) = self.alert.estimated_remaining_turns {
                rem_spans.push(Span::raw("  │  "));
                rem_spans.push(Span::styled("Est. Turns: ", Style::default().fg(Color::DarkGray)));
                rem_spans.push(Span::styled(
                    format!("~{}", turns),
                    Style::default().fg(Color::Yellow),
                ));
            }
        }
        lines.push(Line::from(rem_spans));

        // 4. Breakdown if requested and space permits
        if self.show_breakdown && inner.height >= 6 {
            if let (Some(sys), Some(msgs), Some(tools)) = (
                self.alert.system_tokens,
                self.alert.messages_tokens,
                self.alert.tools_tokens,
            ) {
                lines.push(Line::from(vec![
                    Span::styled("Sys: ", Style::default().fg(Color::DarkGray)),
                    Span::styled(format_token_count(sys), Style::default().fg(Color::Blue)),
                    Span::raw(" │ "),
                    Span::styled("Msgs: ", Style::default().fg(Color::DarkGray)),
                    Span::styled(format_token_count(msgs), Style::default().fg(Color::Cyan)),
                    Span::raw(" │ "),
                    Span::styled("Tools: ", Style::default().fg(Color::DarkGray)),
                    Span::styled(format_token_count(tools), Style::default().fg(Color::Magenta)),
                ]));
            }
        }

        // 5. Action recommendation row if space permits
        if self.show_recommendations && inner.height >= 5 {
            lines.push(Line::from(vec![
                Span::styled("Action: ", Style::default().fg(Color::DarkGray)),
                Span::styled(
                    self.alert.primary_action(),
                    Style::default().fg(Color::White).add_modifier(Modifier::ITALIC),
                ),
            ]));
        }

        let paragraph = Paragraph::new(lines).wrap(Wrap { trim: true });
        paragraph.render(inner, buf);
    }

    fn render_compact(&self, area: Rect, buf: &mut Buffer) {
        if area.width < 4 || area.height == 0 {
            return;
        }

        let alert_color = self.alert.severity.ratatui_color();
        let icon = self.alert.severity.icon();
        let pct = self.alert.percentage_str();
        let used = self.alert.formatted_used();
        let max = self.alert.formatted_max();
        let remaining = self.alert.formatted_remaining();

        let bar_width = (area.width.saturating_sub(36) as usize).clamp(6, 20);
        let bar_str = render_progress_bar_text(
            self.alert.utilization,
            bar_width,
            ProgressBarStyle::UnicodeSubBlock,
        );

        let line = Line::from(vec![
            Span::styled(format!("{} ", icon), Style::default().fg(alert_color).add_modifier(Modifier::BOLD)),
            Span::styled(&self.alert.model, Style::default().fg(Color::Cyan)),
            Span::raw(" ["),
            Span::styled(bar_str, Style::default().fg(alert_color)),
            Span::raw("] "),
            Span::styled(format!("{}/{} ({})", used, max, pct), Style::default().fg(alert_color)),
            Span::raw(" │ Rem: "),
            Span::styled(remaining, Style::default().fg(Color::White).add_modifier(Modifier::BOLD)),
        ]);

        let paragraph = Paragraph::new(line);
        paragraph.render(area, buf);
    }

    fn render_pill(&self, area: Rect, buf: &mut Buffer) {
        if area.width < 4 || area.height == 0 {
            return;
        }

        let alert_color = self.alert.severity.ratatui_color();
        let icon = self.alert.severity.icon();
        let pct = self.alert.percentage_str();
        let remaining = self.alert.formatted_remaining();

        let line = Line::from(vec![
            Span::styled("[", Style::default().fg(Color::DarkGray)),
            Span::styled(format!("{} ", icon), Style::default().fg(alert_color).add_modifier(Modifier::BOLD)),
            Span::styled(pct, Style::default().fg(alert_color).add_modifier(Modifier::BOLD)),
            Span::styled(" │ ", Style::default().fg(Color::DarkGray)),
            Span::styled(format!("{} rem", remaining), Style::default().fg(Color::White)),
            Span::styled("]", Style::default().fg(Color::DarkGray)),
        ]);

        let paragraph = Paragraph::new(line);
        paragraph.render(area, buf);
    }
}

impl<'a> Widget for ContextWarningWidget<'a> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        match self.style_mode {
            ContextWarningStyle::Banner | ContextWarningStyle::Card => self.render_banner(area, buf),
            ContextWarningStyle::Compact | ContextWarningStyle::MiniBar => self.render_compact(area, buf),
            ContextWarningStyle::Pill => self.render_pill(area, buf),
        }
    }
}

impl<'a> Widget for &ContextWarningWidget<'a> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        match self.style_mode {
            ContextWarningStyle::Banner | ContextWarningStyle::Card => self.render_banner(area, buf),
            ContextWarningStyle::Compact | ContextWarningStyle::MiniBar => self.render_compact(area, buf),
            ContextWarningStyle::Pill => self.render_pill(area, buf),
        }
    }
}

/// Standalone Ratatui progress bar widget for context window utilization.
#[derive(Debug, Clone)]
pub struct ContextProgressBarWidget {
    pub utilization: f32,
    pub severity: ContextAlertSeverity,
    pub style: ProgressBarStyle,
    pub label: Option<String>,
}

impl ContextProgressBarWidget {
    pub fn new(utilization: f32) -> Self {
        let severity = ContextAlertSeverity::from_utilization(utilization);
        Self {
            utilization,
            severity,
            style: ProgressBarStyle::UnicodeSubBlock,
            label: None,
        }
    }

    pub fn with_severity(mut self, severity: ContextAlertSeverity) -> Self {
        self.severity = severity;
        self
    }

    pub fn with_style(mut self, style: ProgressBarStyle) -> Self {
        self.style = style;
        self
    }

    pub fn with_label(mut self, label: impl Into<String>) -> Self {
        self.label = Some(label.into());
        self
    }
}

impl Widget for ContextProgressBarWidget {
    fn render(self, area: Rect, buf: &mut Buffer) {
        if area.width == 0 || area.height == 0 {
            return;
        }

        let alert_color = self.severity.ratatui_color();
        let bar_width = (area.width as usize).saturating_sub(
            self.label.as_ref().map(|l| l.chars().count() + 1).unwrap_or(0),
        );

        let bar_str = render_progress_bar_text(self.utilization, bar_width, self.style);

        let mut spans = Vec::new();
        if let Some(lbl) = self.label {
            spans.push(Span::styled(format!("{} ", lbl), Style::default().fg(Color::DarkGray)));
        }
        spans.push(Span::styled(bar_str, Style::default().fg(alert_color)));

        let line = Line::from(spans);
        let paragraph = Paragraph::new(line);
        paragraph.render(area, buf);
    }
}

// ---------------------------------------------------------------------------
// 10. Frame Convenience Functions
// ---------------------------------------------------------------------------

/// Convenience helper to render the context warning widget directly into a Ratatui Frame area.
pub fn render_warning_widget(alert: &ContextLimitAlert, frame: &mut Frame, area: Rect) {
    let widget = ContextWarningWidget::new(alert);
    frame.render_widget(widget, area);
}

/// Convenience helper to render a compact inline warning widget directly into a Ratatui Frame area.
pub fn render_compact_warning_widget(alert: &ContextLimitAlert, frame: &mut Frame, area: Rect) {
    let widget = ContextWarningWidget::new(alert).style_mode(ContextWarningStyle::Compact);
    frame.render_widget(widget, area);
}

// ---------------------------------------------------------------------------
// 11. Unit & Regression Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_severity_classification() {
        assert_eq!(ContextAlertSeverity::from_utilization(0.50), ContextAlertSeverity::Safe);
        assert_eq!(ContextAlertSeverity::from_utilization(0.69), ContextAlertSeverity::Safe);
        assert_eq!(ContextAlertSeverity::from_utilization(0.70), ContextAlertSeverity::Notice);
        assert_eq!(ContextAlertSeverity::from_utilization(0.79), ContextAlertSeverity::Notice);
        assert_eq!(ContextAlertSeverity::from_utilization(0.80), ContextAlertSeverity::Warning);
        assert_eq!(ContextAlertSeverity::from_utilization(0.89), ContextAlertSeverity::Warning);
        assert_eq!(ContextAlertSeverity::from_utilization(0.90), ContextAlertSeverity::Critical);
        assert_eq!(ContextAlertSeverity::from_utilization(0.99), ContextAlertSeverity::Critical);
        assert_eq!(ContextAlertSeverity::from_utilization(1.00), ContextAlertSeverity::Overflow);
        assert_eq!(ContextAlertSeverity::from_utilization(1.20), ContextAlertSeverity::Overflow);
    }

    #[test]
    fn test_severity_properties() {
        let safe = ContextAlertSeverity::Safe;
        assert!(!safe.is_alert());
        assert!(!safe.is_warning_or_worse());

        let notice = ContextAlertSeverity::Notice;
        assert!(notice.is_alert());
        assert!(!notice.is_warning_or_worse());

        let warning = ContextAlertSeverity::Warning;
        assert!(warning.is_alert());
        assert!(warning.is_warning_or_worse());
        assert!(!warning.is_critical_or_worse());

        let critical = ContextAlertSeverity::Critical;
        assert!(critical.is_critical_or_worse());
        assert!(!critical.is_overflow());

        let overflow = ContextAlertSeverity::Overflow;
        assert!(overflow.is_overflow());
    }

    #[test]
    fn test_context_limit_alert_construction() {
        let alert = ContextLimitAlert::new(80_000, 100_000, "claude-3-7-sonnet");
        assert_eq!(alert.used_tokens, 80_000);
        assert_eq!(alert.max_context_tokens, 100_000);
        assert_eq!(alert.severity, ContextAlertSeverity::Warning);
        assert_eq!(alert.percentage_str(), "80.0%");
        assert!(alert.is_warning());
        assert!(!alert.is_critical());
        assert!(!alert.is_overflow());

        // Available = 100_000 - 4096 - 1024 = 94_880
        // Remaining = 94_880 - 80_000 = 14_880
        assert_eq!(alert.remaining_tokens, 14_880);
        assert_eq!(alert.effective_remaining_tokens, 14_880);
    }

    #[test]
    fn test_context_limit_overflow() {
        let alert = ContextLimitAlert::new(105_000, 100_000, "gpt-4o");
        assert_eq!(alert.severity, ContextAlertSeverity::Overflow);
        assert!(alert.is_overflow());
        assert!(alert.remaining_tokens < 0);
        assert_eq!(alert.effective_remaining_tokens, 0);
        assert_eq!(alert.estimated_remaining_turns, Some(0));
    }

    #[test]
    fn test_progress_bar_rendering() {
        let bar_sub = render_progress_bar_text(0.50, 10, ProgressBarStyle::UnicodeSubBlock);
        assert_eq!(bar_sub.chars().count(), 10);

        let bar_ascii = render_progress_bar_text(0.50, 10, ProgressBarStyle::Ascii);
        assert_eq!(bar_ascii, "#####-----");

        let bar_block = render_progress_bar_text(1.00, 10, ProgressBarStyle::UnicodeBlock);
        assert_eq!(bar_block, "██████████");

        let bar_empty = render_progress_bar_text(0.00, 10, ProgressBarStyle::UnicodeBlock);
        assert_eq!(bar_empty, "░░░░░░░░░░");
    }

    #[test]
    fn test_ansi_rendering() {
        let alert = ContextLimitAlert::new(85_000, 100_000, "claude-3-7-sonnet");
        let banner = render_warning_banner_ansi(&alert, 60, WarningBorderStyle::Rounded);
        assert!(banner.contains("Context Window Warning"));
        assert!(banner.contains("85.0%"));
        assert!(banner.contains("claude-3-7-sonnet"));

        let compact = render_warning_compact_ansi(&alert);
        assert!(compact.contains("85.0%"));
        assert!(compact.contains("claude-3-7-sonnet"));

        let pill = render_warning_pill_ansi(&alert);
        assert!(pill.contains("85.0%"));
    }

    #[test]
    fn test_warning_tracker_lifecycle() {
        let mut tracker = ContextWarningTracker::new();

        // 1. Safe usage (< 70%) -> no alert
        let res1 = tracker.evaluate(50_000, 100_000, "claude-3-7-sonnet", 1);
        assert!(res1.is_none());
        assert_eq!(tracker.last_severity, ContextAlertSeverity::Safe);

        // 2. Crosses into Warning (82%) -> triggers alert
        let res2 = tracker.evaluate(82_000, 100_000, "claude-3-7-sonnet", 2);
        assert!(res2.is_some());
        let alert2 = res2.unwrap();
        assert_eq!(alert2.severity, ContextAlertSeverity::Warning);
        assert_eq!(tracker.last_severity, ContextAlertSeverity::Warning);

        // 3. Tiny change in same tier (82.5%) -> suppressed due to hysteresis
        let res3 = tracker.evaluate(82_500, 100_000, "claude-3-7-sonnet", 3);
        assert!(res3.is_none());

        // 4. Escalates to Critical (92%) -> triggers escalation alert
        let res4 = tracker.evaluate(92_000, 100_000, "claude-3-7-sonnet", 4);
        assert!(res4.is_some());
        let alert4 = res4.unwrap();
        assert_eq!(alert4.severity, ContextAlertSeverity::Critical);

        // 5. Dismiss alert
        tracker.dismiss();
        let res5 = tracker.evaluate(93_000, 100_000, "claude-3-7-sonnet", 5);
        assert!(res5.is_none());

        // 6. Escalates to Overflow (102%) -> overrides dismissal
        let res6 = tracker.evaluate(102_000, 100_000, "claude-3-7-sonnet", 6);
        assert!(res6.is_some());
        assert_eq!(res6.unwrap().severity, ContextAlertSeverity::Overflow);

        // 7. Reset after /compact
        tracker.reset();
        assert_eq!(tracker.last_severity, ContextAlertSeverity::Safe);
    }

    #[test]
    fn test_ratatui_widget_buffer_rendering() {
        let alert = ContextLimitAlert::new(88_000, 100_000, "claude-3-7-sonnet")
            .with_system_tokens(10_000)
            .with_messages_tokens(70_000)
            .with_tools_tokens(8_000);

        let area = Rect::new(0, 0, 60, 8);
        let mut buffer = Buffer::empty(area);

        let widget = ContextWarningWidget::new(&alert);
        widget.render(area, &mut buffer);

        // Verify buffer contains expected content characters
        let mut buffer_text = String::new();
        for y in 0..area.height {
            for x in 0..area.width {
                let cell = buffer.get(x, y);
                buffer_text.push_str(cell.symbol());
            }
            buffer_text.push('\n');
        }

        assert!(buffer_text.contains("claude-3-7-sonnet"));
        assert!(buffer_text.contains("88.0%"));
        assert!(buffer_text.contains("Remaining"));
    }

    #[test]
    fn test_compact_widget_buffer_rendering() {
        let alert = ContextLimitAlert::new(92_000, 100_000, "gpt-4o");
        let area = Rect::new(0, 0, 60, 1);
        let mut buffer = Buffer::empty(area);

        let widget = ContextWarningWidget::new(&alert).style_mode(ContextWarningStyle::Compact);
        widget.render(area, &mut buffer);

        let mut buffer_text = String::new();
        for x in 0..area.width {
            let cell = buffer.get(x, 0);
            buffer_text.push_str(cell.symbol());
        }

        assert!(buffer_text.contains("gpt-4o"));
        assert!(buffer_text.contains("92.0%"));
    }

    #[test]
    fn test_pill_widget_buffer_rendering() {
        let alert = ContextLimitAlert::new(85_000, 100_000, "gemini-2.5-pro");
        let area = Rect::new(0, 0, 40, 1);
        let mut buffer = Buffer::empty(area);

        let widget = ContextWarningWidget::new(&alert).style_mode(ContextWarningStyle::Pill);
        widget.render(area, &mut buffer);

        let mut buffer_text = String::new();
        for x in 0..area.width {
            let cell = buffer.get(x, 0);
            buffer_text.push_str(cell.symbol());
        }

        assert!(buffer_text.contains("85.0%"));
    }
}
