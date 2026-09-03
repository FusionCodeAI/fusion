//! Context Window Budget & Visual Warning Banners
//!
//! Provides visual warning banners and progress meters when a conversation
//! approaches model context window limits:
//! - **80% Warning Threshold**: Amber/Yellow warning banner alerting that the context
//!   is filling up, recommending compaction (`/compact`) or pruning.
//! - **95% Critical Danger Threshold**: Bold Red/Crimson danger banner alerting that the
//!   model is critically close to context exhaustion, where truncation or failures occur.
//! - **100%+ Overflow Threshold**: Urgent overflow notice indicating immediate compaction needed.
//!
//! Features:
//! - Pure Rust implementation with zero external C/C++ dependencies.
//! - High-polish ANSI terminal rendering with box-drawing borders and progress meters.
//! - Ratatui widget support (`BudgetBannerWidget`, `render_budget_banner_widget`) for inline/TUI integration.
//! - `BudgetAlertTracker` for stateful deduplication so users are notified on threshold crossings
//!   without turn-by-turn spam.
//! - Adaptive color and theme support (`TokyoNight`, `Monokai`, `Dracula`, `HighContrast`, `Adaptive`).

use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Paragraph, Widget, Wrap},
    Frame,
};
use serde::{Deserialize, Serialize};
use std::fmt;

use crate::agent::session::Session;
use crate::agent::tokens::{format_token_count, model_context_limit, ContextBreakdown};
use crate::provider::types::{Message, ToolDefinition};
use crate::ui::theme::Theme;

// ---------------------------------------------------------------------------
// 1. Constants & Thresholds
// ---------------------------------------------------------------------------

/// Warning threshold ratio (80% of model context window).
pub const WARNING_THRESHOLD: f32 = 0.80;

/// Critical danger threshold ratio (95% of model context window).
pub const CRITICAL_THRESHOLD: f32 = 0.95;

/// Overflow threshold ratio (100% of model context window).
pub const OVERFLOW_THRESHOLD: f32 = 1.00;

/// Default width for ASCII/Unicode progress bar meters.
pub const DEFAULT_METER_WIDTH: usize = 28;

/// Minimum safe banner width in characters.
pub const MIN_BANNER_WIDTH: usize = 48;

/// Default standard banner width in characters.
pub const DEFAULT_BANNER_WIDTH: usize = 72;

// ANSI escape sequences
const ANSI_RESET: &str = "\x1b[0m";
const ANSI_BOLD: &str = "\x1b[1m";
const ANSI_DIM: &str = "\x1b[2m";
const ANSI_YELLOW: &str = "\x1b[33m";
const ANSI_BOLD_YELLOW: &str = "\x1b[1;33m";
const ANSI_RED: &str = "\x1b[31m";
const ANSI_BOLD_RED: &str = "\x1b[1;31m";
const ANSI_BG_RED: &str = "\x1b[41;1;37m";
const ANSI_BG_YELLOW: &str = "\x1b[43;1;30m";
const ANSI_GREEN: &str = "\x1b[32m";
const ANSI_CYAN: &str = "\x1b[36m";
const ANSI_BOLD_CYAN: &str = "\x1b[1;36m";
const ANSI_GRAY: &str = "\x1b[90m";
const ANSI_WHITE: &str = "\x1b[37m";
const ANSI_BOLD_WHITE: &str = "\x1b[1;37m";

// ---------------------------------------------------------------------------
// 2. Alert Levels & Classification
// ---------------------------------------------------------------------------

/// Severity level of the context window consumption.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum ContextAlertLevel {
    /// Token usage is within safe operating margins (< 80%).
    Safe,
    /// Token usage has reached or exceeded 80% of the context window.
    Warning,
    /// Token usage has reached or exceeded 95% of the context window.
    Critical,
    /// Token usage has reached or exceeded 100% of the context window.
    Overflow,
}

impl Default for ContextAlertLevel {
    fn default() -> Self {
        Self::Safe
    }
}

impl ContextAlertLevel {
    /// Classifies context utilization ratio (e.g. 0.82 -> `Warning`).
    pub fn from_utilization(ratio: f32) -> Self {
        if ratio >= OVERFLOW_THRESHOLD {
            Self::Overflow
        } else if ratio >= CRITICAL_THRESHOLD {
            Self::Critical
        } else if ratio >= WARNING_THRESHOLD {
            Self::Warning
        } else {
            Self::Safe
        }
    }

    /// Returns `true` if this level warrants showing a visual alert (Warning, Critical, Overflow).
    #[inline]
    pub fn is_alert(&self) -> bool {
        matches!(self, Self::Warning | Self::Critical | Self::Overflow)
    }

    /// Returns `true` if at Warning level (80% - 94.9%).
    #[inline]
    pub fn is_warning(&self) -> bool {
        matches!(self, Self::Warning)
    }

    /// Returns `true` if at Critical level (95% - 99.9%).
    #[inline]
    pub fn is_critical(&self) -> bool {
        matches!(self, Self::Critical)
    }

    /// Returns `true` if at Overflow level (>= 100%).
    #[inline]
    pub fn is_overflow(&self) -> bool {
        matches!(self, Self::Overflow)
    }

    /// Short label name for the level.
    pub fn name(&self) -> &'static str {
        match self {
            Self::Safe => "Safe",
            Self::Warning => "Warning",
            Self::Critical => "Critical",
            Self::Overflow => "Overflow",
        }
    }

    /// Prominent banner headline title.
    pub fn banner_title(&self) -> &'static str {
        match self {
            Self::Safe => "CONTEXT WINDOW OK",
            Self::Warning => "CONTEXT WINDOW WARNING (80%+ CAPACITY)",
            Self::Critical => "CONTEXT WINDOW CRITICAL (95%+ CAPACITY)",
            Self::Overflow => "CONTEXT WINDOW LIMIT EXCEEDED (100%+)",
        }
    }

    /// Associated status icon symbol.
    pub fn icon(&self) -> &'static str {
        match self {
            Self::Safe => "✓",
            Self::Warning => "⚠",
            Self::Critical => "⛔",
            Self::Overflow => "🚨",
        }
    }

    /// Primary ANSI color code for this level.
    pub fn ansi_color(&self) -> &'static str {
        match self {
            Self::Safe => ANSI_GREEN,
            Self::Warning => ANSI_BOLD_YELLOW,
            Self::Critical => ANSI_BOLD_RED,
            Self::Overflow => ANSI_BG_RED,
        }
    }

    /// Border ANSI color code.
    pub fn ansi_border_color(&self) -> &'static str {
        match self {
            Self::Safe => ANSI_GREEN,
            Self::Warning => ANSI_YELLOW,
            Self::Critical => ANSI_RED,
            Self::Overflow => ANSI_BOLD_RED,
        }
    }

    /// Badge string with background color for terminal rendering.
    pub fn ansi_badge(&self) -> String {
        match self {
            Self::Safe => format!("{}[ OK ]{}", ANSI_GREEN, ANSI_RESET),
            Self::Warning => format!(
                "{}{} WARN 80% {}{}",
                ANSI_BG_YELLOW, ANSI_BOLD, ANSI_RESET, ANSI_RESET
            ),
            Self::Critical => format!(
                "{}{} CRITICAL 95% {}{}",
                ANSI_BG_RED, ANSI_BOLD, ANSI_RESET, ANSI_RESET
            ),
            Self::Overflow => format!(
                "{}{} OVERFLOW 100%+ {}{}",
                ANSI_BG_RED, ANSI_BOLD, ANSI_RESET, ANSI_RESET
            ),
        }
    }

    /// Resolves the corresponding Ratatui Color based on Theme.
    pub fn ratatui_color(&self, theme: &Theme) -> Color {
        match self {
            Self::Safe => theme.success,
            Self::Warning => theme.warning,
            Self::Critical => theme.error,
            Self::Overflow => Color::LightRed,
        }
    }
}

impl fmt::Display for ContextAlertLevel {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.name())
    }
}

// ---------------------------------------------------------------------------
// 3. Context Alert Struct
// ---------------------------------------------------------------------------

/// Context window metrics and alert payload for a model / session.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ContextAlert {
    /// Active model name or identifier.
    pub model: String,
    /// Estimated or exact current token count.
    pub current_tokens: usize,
    /// Total context window capacity for the model.
    pub max_context_tokens: usize,
    /// Context utilization ratio (0.0 to 1.0+).
    pub utilization: f32,
    /// Current alert level classification.
    pub level: ContextAlertLevel,
    /// Remaining tokens before context window is fully exhausted.
    pub remaining_tokens: isize,
    /// Optional system prompt token consumption.
    pub system_tokens: Option<usize>,
    /// Optional conversation messages token consumption.
    pub messages_tokens: Option<usize>,
    /// Optional tool definitions token consumption.
    pub tools_tokens: Option<usize>,
}

impl ContextAlert {
    /// Creates a new `ContextAlert` from current token count and max context limit.
    pub fn new(model: impl Into<String>, current_tokens: usize, max_context_tokens: usize) -> Self {
        let model_str = model.into();
        let max_tokens = if max_context_tokens == 0 {
            model_context_limit(&model_str)
        } else {
            max_context_tokens
        };

        let utilization = if max_tokens == 0 {
            1.0
        } else {
            current_tokens as f32 / max_tokens as f32
        };

        let remaining = max_tokens as isize - current_tokens as isize;
        let level = ContextAlertLevel::from_utilization(utilization);

        Self {
            model: model_str,
            current_tokens,
            max_context_tokens: max_tokens,
            utilization,
            level,
            remaining_tokens: remaining,
            system_tokens: None,
            messages_tokens: None,
            tools_tokens: None,
        }
    }

    /// Creates a `ContextAlert` from a `Session` instance.
    pub fn from_session(session: &Session) -> Self {
        let model = session.active_model();
        let max_context = model_context_limit(model);
        let tokens = session.estimate_tokens();
        Self::new(model, tokens, max_context)
    }

    /// Creates a `ContextAlert` from a `ContextBreakdown`.
    pub fn from_breakdown(model: impl Into<String>, breakdown: &ContextBreakdown) -> Self {
        let mut alert = Self::new(model, breakdown.total_tokens, breakdown.max_context);
        alert.system_tokens = Some(breakdown.system_tokens);
        alert.messages_tokens = Some(breakdown.messages_tokens);
        alert.tools_tokens = Some(breakdown.tools_tokens);
        alert
    }

    /// Creates a `ContextAlert` from conversation messages, system prompt, and tools.
    pub fn from_messages(
        model: impl Into<String>,
        messages: &[Message],
        system_prompt: Option<&str>,
        tools: &[ToolDefinition],
    ) -> Self {
        let model_str = model.into();
        let budget = crate::agent::tokens::ContextBudget::new(&model_str);
        let breakdown = budget.calculate_breakdown(messages, system_prompt, tools);
        Self::from_breakdown(model_str, &breakdown)
    }

    /// Attaches granular token breakdown stats.
    pub fn with_breakdown(mut self, system: usize, messages: usize, tools: usize) -> Self {
        self.system_tokens = Some(system);
        self.messages_tokens = Some(messages);
        self.tools_tokens = Some(tools);
        self
    }

    /// Returns context utilization percentage (0.0 to 100.0+).
    #[inline]
    pub fn utilization_pct(&self) -> f32 {
        self.utilization * 100.0
    }

    /// Formats utilization as a clean string (e.g. `"82.4%"`).
    pub fn percentage_str(&self) -> String {
        format!("{:.1}%", self.utilization_pct())
    }

    /// Formatted current tokens count (e.g. `"165K"`).
    pub fn formatted_current(&self) -> String {
        format_token_count(self.current_tokens)
    }

    /// Formatted maximum context capacity (e.g. `"200K"`).
    pub fn formatted_max(&self) -> String {
        format_token_count(self.max_context_tokens)
    }

    /// Formatted remaining token count (e.g. `"35K"` or `"-5K"`).
    pub fn formatted_remaining(&self) -> String {
        if self.remaining_tokens >= 0 {
            format!(
                "{} tokens remaining",
                format_token_count(self.remaining_tokens as usize)
            )
        } else {
            format!(
                "{} tokens over limit",
                format_token_count((-self.remaining_tokens) as usize)
            )
        }
    }

    /// Returns `true` if current state requires a visual alert (Warning, Critical, Overflow).
    #[inline]
    pub fn is_alert(&self) -> bool {
        self.level.is_alert()
    }

    /// Returns `true` if at 80%+ Warning level.
    #[inline]
    pub fn is_warning(&self) -> bool {
        self.level.is_warning()
    }

    /// Returns `true` if at 95%+ Critical Danger level.
    #[inline]
    pub fn is_critical(&self) -> bool {
        self.level.is_critical()
    }

    /// Returns `true` if at 100%+ Overflow level.
    #[inline]
    pub fn is_overflow(&self) -> bool {
        self.level.is_overflow()
    }

    /// Short primary recommendation action string.
    pub fn primary_action(&self) -> &'static str {
        match self.level {
            ContextAlertLevel::Safe => "Normal operation.",
            ContextAlertLevel::Warning => "Run /compact to summarize conversation history.",
            ContextAlertLevel::Critical => {
                "Urgent: Run /compact now or prune messages to prevent truncation."
            }
            ContextAlertLevel::Overflow => {
                "Critical overflow: Conversation exceeds window. Compact immediately."
            }
        }
    }

    /// List of actionable user tips tailored to the current context state.
    pub fn recommendations(&self) -> Vec<String> {
        match self.level {
            ContextAlertLevel::Safe => vec![
                "Context budget is within comfortable limits.".to_string(),
            ],
            ContextAlertLevel::Warning => vec![
                "Run \x1b[1;36m/compact\x1b[0m to summarize older turns into a concise context block.".to_string(),
                "Use \x1b[1;36m/model\x1b[0m to switch to a larger context window model (e.g. Claude 3.7 / Gemini).".to_string(),
                "Clear temporary scratchpad or bloated tool outputs if no longer required.".to_string(),
            ],
            ContextAlertLevel::Critical => vec![
                "Run \x1b[1;36m/compact\x1b[0m immediately — upcoming turns risk truncation or API errors.".to_string(),
                "Save session with \x1b[1;36m/export\x1b[0m and start a fresh session with \x1b[1;36m/clear\x1b[0m.".to_string(),
                "Switch to an ultra-large context model (1M+ tokens) via \x1b[1;36m/model\x1b[0m.".to_string(),
            ],
            ContextAlertLevel::Overflow => vec![
                "Context limit exceeded. API requests will fail without compaction.".to_string(),
                "Run \x1b[1;36m/compact\x1b[0m or delete recent large tool inputs/outputs.".to_string(),
            ],
        }
    }

    /// Renders a full boxed visual warning banner in ANSI format.
    pub fn render_banner(&self) -> String {
        render_budget_banner_ansi(self, BannerBoxStyle::Rounded, DEFAULT_BANNER_WIDTH)
    }

    /// Renders a compact 2-line warning banner in ANSI format.
    pub fn render_compact(&self) -> String {
        render_budget_banner_compact_ansi(self)
    }

    /// Renders a single-line status pill in ANSI format.
    pub fn render_one_line(&self) -> String {
        render_budget_status_pill_ansi(self)
    }
}

// ---------------------------------------------------------------------------
// 4. Progress Meter & Visual Bar Styles
// ---------------------------------------------------------------------------

/// Visual styling for the progress meter bar.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ProgressBarStyle {
    /// Unicode solid block style: `[████████░░░░]`
    Block,
    /// Unicode smooth sub-character fraction style
    Smooth,
    /// Plain ASCII style: `[########----]`
    Ascii,
    /// Unicode shaded gradient style: `[████▓▓▒▒░░]`
    Shaded,
}

impl Default for ProgressBarStyle {
    fn default() -> Self {
        Self::Block
    }
}

/// Configuration options for rendering a progress bar meter.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProgressBarConfig {
    /// Width of the bar itself in characters (excluding labels).
    pub width: usize,
    /// Style of characters used in the bar.
    pub style: ProgressBarStyle,
    /// Whether to include ANSI color styling.
    pub colored: bool,
    /// Whether to show percentage label.
    pub show_percentage: bool,
    /// Whether to show numerical token counts `(160K / 200K)`.
    pub show_counts: bool,
}

impl Default for ProgressBarConfig {
    fn default() -> Self {
        Self {
            width: DEFAULT_METER_WIDTH,
            style: ProgressBarStyle::Block,
            colored: true,
            show_percentage: true,
            show_counts: true,
        }
    }
}

impl ProgressBarConfig {
    /// Creates a new progress bar configuration with specified width.
    pub fn new(width: usize) -> Self {
        Self {
            width,
            ..Default::default()
        }
    }

    /// Disables ANSI colors.
    pub fn plain(mut self) -> Self {
        self.colored = false;
        self
    }

    /// Sets progress bar style.
    pub fn with_style(mut self, style: ProgressBarStyle) -> Self {
        self.style = style;
        self
    }
}

/// Formats a visual token usage progress meter string.
pub fn format_token_progress_bar(
    current: usize,
    max: usize,
    width: usize,
    colored: bool,
) -> String {
    let config = ProgressBarConfig {
        width,
        colored,
        show_percentage: true,
        show_counts: true,
        ..Default::default()
    };
    format_progress_bar(current, max, &config)
}

/// Formats a visual progress meter based on `ProgressBarConfig`.
pub fn format_progress_bar(current: usize, max: usize, config: &ProgressBarConfig) -> String {
    let safe_max = max.max(1);
    let ratio = (current as f32 / safe_max as f32).max(0.0);
    let pct = ratio * 100.0;
    let width = config.width.max(4);

    let filled_exact = (ratio * width as f32).min(width as f32);
    let filled_chars = filled_exact.floor() as usize;
    let remainder = filled_exact - filled_chars as f32;

    let (fill_char, empty_char) = match config.style {
        ProgressBarStyle::Block => ('█', '░'),
        ProgressBarStyle::Ascii => ('#', '-'),
        ProgressBarStyle::Shaded => ('█', '░'),
        ProgressBarStyle::Smooth => ('█', '░'),
    };

    let bar_str = if config.style == ProgressBarStyle::Smooth && filled_chars < width {
        let partial_char = if remainder >= 0.875 {
            '▉'
        } else if remainder >= 0.75 {
            '▊'
        } else if remainder >= 0.625 {
            '▋'
        } else if remainder >= 0.50 {
            '▌'
        } else if remainder >= 0.375 {
            '▍'
        } else if remainder >= 0.25 {
            '▎'
        } else if remainder >= 0.125 {
            '▏'
        } else {
            empty_char
        };

        let mut s = String::with_capacity(width * 4);
        for _ in 0..filled_chars {
            s.push(fill_char);
        }
        if filled_chars < width {
            s.push(partial_char);
            for _ in (filled_chars + 1)..width {
                s.push(empty_char);
            }
        }
        s
    } else {
        let mut s = String::with_capacity(width * 4);
        for i in 0..width {
            if i < filled_chars {
                s.push(fill_char);
            } else {
                s.push(empty_char);
            }
        }
        s
    };

    let level = ContextAlertLevel::from_utilization(ratio);

    let colored_bar = if config.colored {
        match level {
            ContextAlertLevel::Safe => format!("{}{}{}", ANSI_GREEN, bar_str, ANSI_RESET),
            ContextAlertLevel::Warning => format!("{}{}{}", ANSI_BOLD_YELLOW, bar_str, ANSI_RESET),
            ContextAlertLevel::Critical => format!("{}{}{}", ANSI_BOLD_RED, bar_str, ANSI_RESET),
            ContextAlertLevel::Overflow => format!("{}{}{}", ANSI_BG_RED, bar_str, ANSI_RESET),
        }
    } else {
        bar_str
    };

    let mut result = format!("[{}]", colored_bar);

    if config.show_percentage {
        let pct_str = if config.colored {
            match level {
                ContextAlertLevel::Safe => format!(" {}{:.1}%{}", ANSI_GREEN, pct, ANSI_RESET),
                ContextAlertLevel::Warning => {
                    format!(" {}{:.1}%{}", ANSI_BOLD_YELLOW, pct, ANSI_RESET)
                }
                ContextAlertLevel::Critical => {
                    format!(" {}{:.1}%{}", ANSI_BOLD_RED, pct, ANSI_RESET)
                }
                ContextAlertLevel::Overflow => format!(" {}{:.1}%{}", ANSI_BG_RED, pct, ANSI_RESET),
            }
        } else {
            format!(" {:.1}%", pct)
        };
        result.push_str(&pct_str);
    }

    if config.show_counts {
        let counts_str = if config.colored {
            format!(
                " {}({} / {}){}",
                ANSI_GRAY,
                format_token_count(current),
                format_token_count(max),
                ANSI_RESET
            )
        } else {
            format!(
                " ({} / {})",
                format_token_count(current),
                format_token_count(max)
            )
        };
        result.push_str(&counts_str);
    }

    result
}

// ---------------------------------------------------------------------------
// 5. Box Border Styles & ANSI Banner Rendering
// ---------------------------------------------------------------------------

/// Terminal box border style characters.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BannerBoxStyle {
    /// Rounded Unicode corners: `╭─╮`, `│`, `╰─╯`
    Rounded,
    /// Heavy/Bold Unicode borders: `┏━┓`, `┃`, `┗━┛`
    Heavy,
    /// Double-line Unicode borders: `╔═╗`, `║`, `╚═╝`
    Double,
    /// Plain ASCII corners: `+--+`, `|`, `+--+`
    Ascii,
}

impl BannerBoxStyle {
    #[inline]
    pub fn top_left(&self) -> char {
        match self {
            Self::Rounded => '╭',
            Self::Heavy => '┏',
            Self::Double => '╔',
            Self::Ascii => '+',
        }
    }

    #[inline]
    pub fn top_right(&self) -> char {
        match self {
            Self::Rounded => '╮',
            Self::Heavy => '┓',
            Self::Double => '╗',
            Self::Ascii => '+',
        }
    }

    #[inline]
    pub fn bottom_left(&self) -> char {
        match self {
            Self::Rounded => '╰',
            Self::Heavy => '┗',
            Self::Double => '╚',
            Self::Ascii => '+',
        }
    }

    #[inline]
    pub fn bottom_right(&self) -> char {
        match self {
            Self::Rounded => '╯',
            Self::Heavy => '┛',
            Self::Double => '╝',
            Self::Ascii => '+',
        }
    }

    #[inline]
    pub fn horizontal(&self) -> char {
        match self {
            Self::Rounded => '─',
            Self::Heavy => '━',
            Self::Double => '═',
            Self::Ascii => '-',
        }
    }

    #[inline]
    pub fn vertical(&self) -> char {
        match self {
            Self::Rounded => '│',
            Self::Heavy => '┃',
            Self::Double => '║',
            Self::Ascii => '|',
        }
    }
}

/// Renders a full, styled visual context warning banner with borders, progress meter,
/// metrics breakdown, and actionable suggestions.
pub fn render_budget_banner_ansi(
    alert: &ContextAlert,
    box_style: BannerBoxStyle,
    banner_width: usize,
) -> String {
    let width = banner_width.max(MIN_BANNER_WIDTH);
    let border_color = alert.level.ansi_border_color();
    let horiz = box_style.horizontal();
    let vert = box_style.vertical();

    let title_badge = format!(" {} {} ", alert.level.icon(), alert.level.banner_title());
    let title_len = visible_width(&title_badge);

    // Top border with embedded title
    let top_horiz_total = width.saturating_sub(title_len + 4);
    let top_left_count = 2;
    let top_right_count = top_horiz_total.saturating_sub(top_left_count);

    let mut out = String::with_capacity(1024);

    // Header row
    out.push_str(border_color);
    out.push(box_style.top_left());
    for _ in 0..top_left_count {
        out.push(horiz);
    }
    out.push_str(ANSI_RESET);

    // Title styling based on severity
    let styled_title = match alert.level {
        ContextAlertLevel::Safe => {
            format!("{}{}{}{}", ANSI_BOLD, ANSI_GREEN, title_badge, ANSI_RESET)
        }
        ContextAlertLevel::Warning => {
            format!("{}{}{}{}", ANSI_BOLD, ANSI_YELLOW, title_badge, ANSI_RESET)
        }
        ContextAlertLevel::Critical => {
            format!("{}{}{}{}", ANSI_BOLD, ANSI_RED, title_badge, ANSI_RESET)
        }
        ContextAlertLevel::Overflow => {
            format!("{}{}{}{}", ANSI_BOLD, ANSI_BG_RED, title_badge, ANSI_RESET)
        }
    };
    out.push_str(&styled_title);

    out.push_str(border_color);
    for _ in 0..top_right_count {
        out.push(horiz);
    }
    out.push(box_style.top_right());
    out.push_str(ANSI_RESET);
    out.push('\n');

    // Helper closure to format a padded inner row
    let render_row = |content: &str| -> String {
        let content_vis_len = visible_width(content);
        let padding = width.saturating_sub(content_vis_len + 4);
        let mut row = String::new();
        row.push_str(border_color);
        row.push(vert);
        row.push(' ');
        row.push_str(ANSI_RESET);
        row.push_str(content);
        for _ in 0..padding {
            row.push(' ');
        }
        row.push_str(border_color);
        row.push(' ');
        row.push(vert);
        row.push_str(ANSI_RESET);
        row.push('\n');
        row
    };

    // Blank row
    out.push_str(&render_row(""));

    // Model & Status line
    let model_line = format!(
        "Model: {}{}{}  │  Usage: {}{}{} ({})",
        ANSI_BOLD_CYAN,
        alert.model,
        ANSI_RESET,
        alert.level.ansi_color(),
        alert.formatted_current(),
        ANSI_RESET,
        alert.percentage_str()
    );
    out.push_str(&render_row(&model_line));

    // Progress Bar Meter line
    let meter_width = (width.saturating_sub(32)).clamp(12, 40);
    let progress_bar = format_token_progress_bar(
        alert.current_tokens,
        alert.max_context_tokens,
        meter_width,
        true,
    );
    let meter_line = format!("Capacity: {}", progress_bar);
    out.push_str(&render_row(&meter_line));

    // Remaining tokens line
    let remaining_styled = if alert.remaining_tokens >= 0 {
        format!(
            "Available: {}{}{} before window full",
            ANSI_BOLD_WHITE,
            format_token_count(alert.remaining_tokens as usize),
            ANSI_RESET
        )
    } else {
        format!(
            "Over Limit: {}{}{} tokens exceed window",
            ANSI_BOLD_RED,
            format_token_count((-alert.remaining_tokens) as usize),
            ANSI_RESET
        )
    };
    out.push_str(&render_row(&remaining_styled));

    // Granular breakdown if available
    if let (Some(sys), Some(msg), Some(tools)) = (
        alert.system_tokens,
        alert.messages_tokens,
        alert.tools_tokens,
    ) {
        let breakdown_line = format!(
            "Breakdown: {}Sys: {}  │  Msgs: {}  │  Tools: {}{}",
            ANSI_GRAY,
            format_token_count(sys),
            format_token_count(msg),
            format_token_count(tools),
            ANSI_RESET
        );
        out.push_str(&render_row(&breakdown_line));
    }

    // Blank row before recommendations
    out.push_str(&render_row(""));

    // Recommendations section
    let rec_header = format!("{}Suggested Actions:{}", ANSI_BOLD, ANSI_RESET);
    out.push_str(&render_row(&rec_header));

    for rec in alert.recommendations() {
        let rec_line = format!("  • {}", rec);
        out.push_str(&render_row(&rec_line));
    }

    // Blank row before bottom border
    out.push_str(&render_row(""));

    // Bottom border
    out.push_str(border_color);
    out.push(box_style.bottom_left());
    for _ in 0..width.saturating_sub(2) {
        out.push(horiz);
    }
    out.push(box_style.bottom_right());
    out.push_str(ANSI_RESET);

    out
}

/// Renders a compact 2-line visual banner suitable for mobile/Termux or narrow viewports.
pub fn render_budget_banner_compact_ansi(alert: &ContextAlert) -> String {
    let icon = alert.level.icon();
    let badge = alert.level.ansi_badge();
    let pct = alert.percentage_str();
    let model = &alert.model;
    let curr = alert.formatted_current();
    let max = alert.formatted_max();

    let line1 = format!(
        "{} {} [{}] {} / {} tokens ({})",
        badge, icon, model, curr, max, pct
    );

    let action = alert.primary_action();
    let line2 = format!("   {}→ Tip: {}{}", ANSI_GRAY, action, ANSI_RESET);

    format!("{}\n{}", line1, line2)
}

/// Renders a single-line status pill for the inline status bar or prompt.
pub fn render_budget_status_pill_ansi(alert: &ContextAlert) -> String {
    let icon = alert.level.icon();
    let color = alert.level.ansi_color();
    let pct = alert.percentage_str();
    let curr = alert.formatted_current();
    let max = alert.formatted_max();

    format!(
        "{}{} {} [{} / {}]{}",
        color, icon, pct, curr, max, ANSI_RESET
    )
}

// ---------------------------------------------------------------------------
// 6. Ratatui Widget & Frame Renderers
// ---------------------------------------------------------------------------

/// Ratatui Widget for rendering context budget alerts into terminal frames.
pub struct BudgetBannerWidget<'a> {
    alert: &'a ContextAlert,
    theme: Theme,
    border_type: BorderType,
}

impl<'a> BudgetBannerWidget<'a> {
    /// Creates a new `BudgetBannerWidget` for the specified alert.
    pub fn new(alert: &'a ContextAlert) -> Self {
        Self {
            alert,
            theme: Theme::auto(),
            border_type: BorderType::Rounded,
        }
    }

    /// Sets custom Theme for rendering.
    pub fn with_theme(mut self, theme: Theme) -> Self {
        self.theme = theme;
        self
    }

    /// Sets custom Ratatui BorderType.
    pub fn with_border_type(mut self, border_type: BorderType) -> Self {
        self.border_type = border_type;
        self
    }
}

impl<'a> Widget for BudgetBannerWidget<'a> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        if area.width < 10 || area.height < 3 {
            return;
        }

        let alert_color = self.alert.level.ratatui_color(&self.theme);
        let title = format!(
            " {} {} ",
            self.alert.level.icon(),
            self.alert.level.banner_title()
        );

        let block = Block::default()
            .title(Span::styled(
                title,
                Style::default()
                    .fg(alert_color)
                    .add_modifier(Modifier::BOLD),
            ))
            .borders(Borders::ALL)
            .border_type(self.border_type)
            .border_style(Style::default().fg(alert_color));

        let mut lines = Vec::new();

        // Line 1: Model & Percentage info
        lines.push(Line::from(vec![
            Span::raw("Model: "),
            Span::styled(
                &self.alert.model,
                Style::default()
                    .fg(self.theme.primary)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw("  |  Usage: "),
            Span::styled(
                format!(
                    "{} / {} tokens",
                    self.alert.formatted_current(),
                    self.alert.formatted_max()
                ),
                Style::default().fg(self.theme.foreground),
            ),
            Span::styled(
                format!(" ({})", self.alert.percentage_str()),
                Style::default()
                    .fg(alert_color)
                    .add_modifier(Modifier::BOLD),
            ),
        ]));

        // Line 2: Visual ASCII/Unicode Progress Meter
        let bar_width = (area.width.saturating_sub(18) as usize).clamp(8, 36);
        let plain_bar = format_progress_bar(
            self.alert.current_tokens,
            self.alert.max_context_tokens,
            &ProgressBarConfig {
                width: bar_width,
                colored: false,
                show_percentage: true,
                show_counts: false,
                ..Default::default()
            },
        );

        lines.push(Line::from(vec![
            Span::raw("Meter: "),
            Span::styled(plain_bar, Style::default().fg(alert_color)),
            Span::raw(format!("  |  {}", self.alert.formatted_remaining())),
        ]));

        // Line 3: Granular Breakdown or Recommendation
        if let (Some(sys), Some(msg), Some(tools)) = (
            self.alert.system_tokens,
            self.alert.messages_tokens,
            self.alert.tools_tokens,
        ) {
            lines.push(Line::from(vec![
                Span::styled("Tokens: ", Style::default().fg(self.theme.muted)),
                Span::styled(
                    format!(
                        "Sys: {}  Msgs: {}  Tools: {}",
                        format_token_count(sys),
                        format_token_count(msg),
                        format_token_count(tools)
                    ),
                    Style::default().fg(self.theme.muted),
                ),
            ]));
        }

        // Line 4+: Action advice
        if area.height >= 5 {
            lines.push(Line::from(vec![
                Span::styled(
                    "Action: ",
                    Style::default()
                        .fg(alert_color)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    self.alert.primary_action(),
                    Style::default().fg(self.theme.foreground),
                ),
            ]));
        }

        let paragraph = Paragraph::new(lines).block(block).wrap(Wrap { trim: true });
        paragraph.render(area, buf);
    }
}

/// Renders a full context warning banner inside a Ratatui Frame area.
pub fn render_budget_banner_widget(
    f: &mut Frame,
    area: Rect,
    alert: &ContextAlert,
    theme: Option<&Theme>,
) {
    let t = theme.cloned().unwrap_or_else(Theme::auto);
    let widget = BudgetBannerWidget::new(alert).with_theme(t);
    f.render_widget(widget, area);
}

/// Renders a compact context warning banner inside a Ratatui Frame area.
pub fn render_budget_banner_compact_widget(
    f: &mut Frame,
    area: Rect,
    alert: &ContextAlert,
    theme: Option<&Theme>,
) {
    let t = theme.cloned().unwrap_or_else(Theme::auto);
    let alert_color = alert.level.ratatui_color(&t);

    let title = format!(
        " {} Context {} ",
        alert.level.icon(),
        alert.percentage_str()
    );
    let block = Block::default()
        .title(Span::styled(
            title,
            Style::default()
                .fg(alert_color)
                .add_modifier(Modifier::BOLD),
        ))
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(alert_color));

    let content = format!(
        "{} / {} tokens. {}",
        alert.formatted_current(),
        alert.formatted_max(),
        alert.primary_action()
    );

    let paragraph = Paragraph::new(content)
        .block(block)
        .wrap(Wrap { trim: true });

    f.render_widget(paragraph, area);
}

// ---------------------------------------------------------------------------
// 7. Stateful Alert Tracker & Deduplication
// ---------------------------------------------------------------------------

/// Stateful tracker that monitors conversation context growth and manages
/// alert notifications, ensuring alerts are surfaced when crossing 80% and 95%
/// thresholds without spamming on every consecutive turn.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BudgetAlertTracker {
    /// Last alert level that was surfaced to the user.
    pub last_alert_level: ContextAlertLevel,
    /// Token count at the time of the last surfaced alert.
    pub last_alert_tokens: usize,
    /// Percentage utilization at the time of the last surfaced alert.
    pub last_alert_pct: f32,
    /// Minimum percentage delta required to trigger a repeated alert within the same level.
    pub min_pct_delta: f32,
    /// Whether an alert has already been surfaced in this session.
    pub alerted_count: usize,
}

impl Default for BudgetAlertTracker {
    fn default() -> Self {
        Self::new()
    }
}

impl BudgetAlertTracker {
    /// Creates a new `BudgetAlertTracker` starting in `Safe` state.
    pub fn new() -> Self {
        Self {
            last_alert_level: ContextAlertLevel::Safe,
            last_alert_tokens: 0,
            last_alert_pct: 0.0,
            min_pct_delta: 5.0, // Alert on each +5% growth after warning threshold
            alerted_count: 0,
        }
    }

    /// Sets the minimum percentage delta to trigger repeated alerts.
    pub fn with_min_pct_delta(mut self, delta: f32) -> Self {
        self.min_pct_delta = delta.max(1.0);
        self
    }

    /// Determines whether the given `ContextAlert` should trigger a visual notification.
    ///
    /// Triggers when:
    /// 1. Level transitions from `Safe` -> `Warning` (crossed 80%).
    /// 2. Level transitions from `Warning` -> `Critical` (crossed 95%).
    /// 3. Level transitions to `Overflow` (crossed 100%).
    /// 4. Within the same warning or critical level, context grew by at least `min_pct_delta`%.
    pub fn should_notify(&self, alert: &ContextAlert) -> bool {
        if !alert.is_alert() {
            return false;
        }

        // Always notify on upward severity transition
        if alert.level > self.last_alert_level {
            return true;
        }

        // If at the same alert level, only notify if token usage grew significantly
        if alert.level == self.last_alert_level {
            let pct_diff = alert.utilization_pct() - self.last_alert_pct;
            if pct_diff >= self.min_pct_delta {
                return true;
            }
        }

        false
    }

    /// Records that an alert was displayed, updating internal state.
    pub fn record_notification(&mut self, alert: &ContextAlert) {
        self.last_alert_level = alert.level;
        self.last_alert_tokens = alert.current_tokens;
        self.last_alert_pct = alert.utilization_pct();
        self.alerted_count = self.alerted_count.saturating_add(1);
    }

    /// Checks the given alert and returns a formatted ANSI banner string if notification is due.
    pub fn check_and_notify(&mut self, alert: &ContextAlert) -> Option<String> {
        if self.should_notify(alert) {
            self.record_notification(alert);
            Some(alert.render_banner())
        } else {
            None
        }
    }

    /// Checks the given alert and returns a compact 2-line ANSI banner if notification is due.
    pub fn check_and_notify_compact(&mut self, alert: &ContextAlert) -> Option<String> {
        if self.should_notify(alert) {
            self.record_notification(alert);
            Some(alert.render_compact())
        } else {
            None
        }
    }

    /// Resets the tracker back to clean `Safe` state (e.g. after conversation compaction or session clear).
    pub fn reset(&mut self) {
        self.last_alert_level = ContextAlertLevel::Safe;
        self.last_alert_tokens = 0;
        self.last_alert_pct = 0.0;
        self.alerted_count = 0;
    }
}

// ---------------------------------------------------------------------------
// 8. Convenience Evaluation Helpers
// ---------------------------------------------------------------------------

/// Evaluates a model's token usage against its context limits and returns a `ContextAlert`.
pub fn evaluate_context_budget(
    model: &str,
    current_tokens: usize,
    max_context: Option<usize>,
) -> ContextAlert {
    let max = max_context.unwrap_or_else(|| model_context_limit(model));
    ContextAlert::new(model, current_tokens, max)
}

/// Evaluates a `Session`'s current conversation size and returns a `ContextAlert`.
pub fn evaluate_session_budget(session: &Session) -> ContextAlert {
    ContextAlert::from_session(session)
}

/// Helper function to check if token usage has reached the 80% warning threshold.
#[inline]
pub fn check_warning_80(current_tokens: usize, max_context: usize) -> bool {
    if max_context == 0 {
        return true;
    }
    (current_tokens as f32 / max_context as f32) >= WARNING_THRESHOLD
}

/// Helper function to check if token usage has reached the 95% critical danger threshold.
#[inline]
pub fn check_critical_95(current_tokens: usize, max_context: usize) -> bool {
    if max_context == 0 {
        return true;
    }
    (current_tokens as f32 / max_context as f32) >= CRITICAL_THRESHOLD
}

/// Helper to strip ANSI escape codes and compute visible printable character count.
fn visible_width(s: &str) -> usize {
    let mut width = 0;
    let mut in_escape = false;

    for c in s.chars() {
        if c == '\x1b' {
            in_escape = true;
        } else if in_escape {
            if c == 'm' || c == 'K' || c == 'H' || c == 'J' {
                in_escape = false;
            }
        } else {
            width += 1;
        }
    }
    width
}

// ---------------------------------------------------------------------------
// 9. Unit Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;

    #[test]
    fn test_alert_level_classification() {
        // Safe: < 80%
        assert_eq!(
            ContextAlertLevel::from_utilization(0.0),
            ContextAlertLevel::Safe
        );
        assert_eq!(
            ContextAlertLevel::from_utilization(0.50),
            ContextAlertLevel::Safe
        );
        assert_eq!(
            ContextAlertLevel::from_utilization(0.799),
            ContextAlertLevel::Safe
        );
        assert!(!ContextAlertLevel::Safe.is_alert());

        // Warning: >= 80% and < 95%
        assert_eq!(
            ContextAlertLevel::from_utilization(0.80),
            ContextAlertLevel::Warning
        );
        assert_eq!(
            ContextAlertLevel::from_utilization(0.85),
            ContextAlertLevel::Warning
        );
        assert_eq!(
            ContextAlertLevel::from_utilization(0.949),
            ContextAlertLevel::Warning
        );
        assert!(ContextAlertLevel::Warning.is_alert());
        assert!(ContextAlertLevel::Warning.is_warning());
        assert!(!ContextAlertLevel::Warning.is_critical());

        // Critical: >= 95% and < 100%
        assert_eq!(
            ContextAlertLevel::from_utilization(0.95),
            ContextAlertLevel::Critical
        );
        assert_eq!(
            ContextAlertLevel::from_utilization(0.98),
            ContextAlertLevel::Critical
        );
        assert_eq!(
            ContextAlertLevel::from_utilization(0.999),
            ContextAlertLevel::Critical
        );
        assert!(ContextAlertLevel::Critical.is_alert());
        assert!(ContextAlertLevel::Critical.is_critical());
        assert!(!ContextAlertLevel::Critical.is_warning());

        // Overflow: >= 100%
        assert_eq!(
            ContextAlertLevel::from_utilization(1.0),
            ContextAlertLevel::Overflow
        );
        assert_eq!(
            ContextAlertLevel::from_utilization(1.2),
            ContextAlertLevel::Overflow
        );
        assert!(ContextAlertLevel::Overflow.is_alert());
        assert!(ContextAlertLevel::Overflow.is_overflow());
    }

    #[test]
    fn test_context_alert_80_percent_warning() {
        // 160,000 / 200,000 = 80.0%
        let alert = ContextAlert::new("claude-3-5-sonnet", 160_000, 200_000);
        assert_eq!(alert.level, ContextAlertLevel::Warning);
        assert!(alert.is_warning());
        assert!(!alert.is_critical());
        assert!(!alert.is_overflow());
        assert_eq!(alert.remaining_tokens, 40_000);
        assert_eq!(alert.percentage_str(), "80.0%");

        let banner = alert.render_banner();
        assert!(banner.contains("WARNING"));
        assert!(banner.contains("80%"));
        assert!(banner.contains("claude-3-5-sonnet"));
        assert!(banner.contains("/compact"));

        let compact = alert.render_compact();
        assert!(compact.contains("WARN 80%"));
        assert!(compact.contains("160K"));
    }

    #[test]
    fn test_context_alert_95_percent_critical() {
        // 190,000 / 200,000 = 95.0%
        let alert = ContextAlert::new("gpt-4o", 190_000, 200_000);
        assert_eq!(alert.level, ContextAlertLevel::Critical);
        assert!(alert.is_critical());
        assert!(!alert.is_warning());
        assert_eq!(alert.remaining_tokens, 10_000);
        assert_eq!(alert.percentage_str(), "95.0%");

        let banner = alert.render_banner();
        assert!(banner.contains("CRITICAL"));
        assert!(banner.contains("95%"));
        assert!(banner.contains("gpt-4o"));

        let compact = alert.render_compact();
        assert!(compact.contains("CRITICAL 95%"));
    }

    #[test]
    fn test_check_helpers() {
        assert!(!check_warning_80(79, 100));
        assert!(check_warning_80(80, 100));
        assert!(check_warning_80(95, 100));

        assert!(!check_critical_95(94, 100));
        assert!(check_critical_95(95, 100));
        assert!(check_critical_95(99, 100));
    }

    #[test]
    fn test_progress_bar_rendering() {
        let bar_plain = format_progress_bar(
            80,
            100,
            &ProgressBarConfig {
                width: 10,
                colored: false,
                show_percentage: true,
                show_counts: true,
                style: ProgressBarStyle::Block,
            },
        );
        assert!(bar_plain.contains("████████░░"));
        assert!(bar_plain.contains("80.0%"));

        let bar_smooth = format_progress_bar(
            95,
            100,
            &ProgressBarConfig {
                width: 10,
                colored: false,
                show_percentage: true,
                show_counts: false,
                style: ProgressBarStyle::Smooth,
            },
        );
        assert!(bar_smooth.contains("█████████▌"));
    }

    #[test]
    fn test_budget_alert_tracker_state_transitions() {
        let mut tracker = BudgetAlertTracker::new();

        // 1. Safe (50%) -> should NOT notify
        let safe_alert = ContextAlert::new("deepseek-chat", 50_000, 100_000);
        assert!(!tracker.should_notify(&safe_alert));
        assert!(tracker.check_and_notify(&safe_alert).is_none());

        // 2. Crossed 80% Warning -> MUST notify
        let warn_80 = ContextAlert::new("deepseek-chat", 80_000, 100_000);
        assert!(tracker.should_notify(&warn_80));
        let notice = tracker.check_and_notify(&warn_80);
        assert!(notice.is_some());
        assert_eq!(tracker.last_alert_level, ContextAlertLevel::Warning);

        // 3. Small token change at 82% (< 5% delta) -> should NOT duplicate spam
        let warn_82 = ContextAlert::new("deepseek-chat", 82_000, 100_000);
        assert!(!tracker.should_notify(&warn_82));

        // 4. Token change at 86% (+6% delta) -> should notify
        let warn_86 = ContextAlert::new("deepseek-chat", 86_000, 100_000);
        assert!(tracker.should_notify(&warn_86));
        tracker.record_notification(&warn_86);

        // 5. Crossed 95% Critical -> MUST notify (level increase)
        let crit_95 = ContextAlert::new("deepseek-chat", 95_000, 100_000);
        assert!(tracker.should_notify(&crit_95));
        let crit_notice = tracker.check_and_notify(&crit_95);
        assert!(crit_notice.is_some());
        assert_eq!(tracker.last_alert_level, ContextAlertLevel::Critical);

        // 6. Reset tracker
        tracker.reset();
        assert_eq!(tracker.last_alert_level, ContextAlertLevel::Safe);
    }

    #[test]
    fn test_ratatui_widget_rendering() {
        let backend = TestBackend::new(80, 8);
        let mut terminal = Terminal::new(backend).unwrap();

        let alert = ContextAlert::new("claude-3-7-sonnet", 165_000, 200_000)
            .with_breakdown(1000, 160_000, 4000);

        terminal
            .draw(|f| {
                let area = f.area();
                render_budget_banner_widget(f, area, &alert, None);
            })
            .unwrap();

        let buffer = terminal.backend().buffer();
        let content_str: String = buffer.content().iter().map(|c| c.symbol()).collect();
        assert!(content_str.contains("WARNING") || content_str.contains("claude-3-7-sonnet"));
    }

    #[test]
    fn test_ratatui_compact_widget_rendering() {
        let backend = TestBackend::new(80, 4);
        let mut terminal = Terminal::new(backend).unwrap();

        let alert = ContextAlert::new("o3-mini", 192_000, 200_000);

        terminal
            .draw(|f| {
                let area = f.area();
                render_budget_banner_compact_widget(f, area, &alert, None);
            })
            .unwrap();

        let buffer = terminal.backend().buffer();
        let content_str: String = buffer.content().iter().map(|c| c.symbol()).collect();
        assert!(content_str.contains("Context") && content_str.contains("96.0%"));
    }
}
