//! High-contrast rounded bordered tool output card renderer for Fusion.
//!
//! Renders tool execution output into formatted terminal cards:
//! ```text
//! ╭─ bash: cargo test (15ms) ───────────────────────────────────╮
//! │ 15 passed in 0.02s                                          │
//! ╰─────────────────────────────────────────────────────────────╯
//! ```
//!
//! Features:
//! - Rounded box-drawing characters (`╭`, `─`, `╮`, `│`, `╰`, `╯`).
//! - Status-colored header chips (green for success, red for failure, cyan for read-only tools).
//! - Automatic text wrapping to `terminal_width - 4` using [`wrap_ansi`].
//! - Tail-preserving truncation for outputs exceeding 30 lines with `… N earlier lines elided …`.
//! - Clean multi-card stacking via [`render_tool_cards`].

use std::time::Duration;

use crate::ui::table::{get_terminal_width, strip_ansi, truncate_ansi, visible_width, wrap_ansi};

// ============================================================================
// ANSI Color Constants & Box Glyphs
// ============================================================================

pub const ANSI_RESET: &str = "\x1b[0m";
pub const ANSI_BOLD: &str = "\x1b[1m";
pub const ANSI_DIM: &str = "\x1b[2m";
pub const ANSI_GREEN: &str = "\x1b[32m";
pub const ANSI_RED: &str = "\x1b[31m";
pub const ANSI_CYAN: &str = "\x1b[36m";
pub const ANSI_GRAY: &str = "\x1b[90m";

// Box drawing characters
pub const BOX_TOP_LEFT: &str = "╭";
pub const BOX_TOP_RIGHT: &str = "╮";
pub const BOX_BOTTOM_LEFT: &str = "╰";
pub const BOX_BOTTOM_RIGHT: &str = "╯";
pub const BOX_HORIZONTAL: &str = "─";
pub const BOX_VERTICAL: &str = "│";

/// Default card width in columns if terminal width is 0 or unconstrained.
pub const DEFAULT_CARD_WIDTH: usize = 80;

/// Minimum safe card width in terminal columns.
pub const MIN_CARD_WIDTH: usize = 12;

/// Default maximum lines of tool output displayed before earlier lines are elided.
pub const DEFAULT_MAX_LINES: usize = 30;

// ============================================================================
// Tool Status Enum
// ============================================================================

/// Execution outcome status of a tool invocation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolStatus {
    /// The tool invocation succeeded.
    Success,
    /// The tool invocation failed or produced an error.
    Failed,
}

// ============================================================================
// ToolOutputCard Struct
// ============================================================================

/// Represents a completed or failed tool invocation and its captured output.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolOutputCard {
    /// Name of the tool (e.g. `bash`, `read`, `grep`, `edit`).
    pub tool_name: String,
    /// Title or command summary of the tool invocation (e.g. `cargo test`, `src/lib.rs`).
    pub title: String,
    /// Alias for title/summary.
    pub summary: String,
    /// Output content (stdout, stderr, returned text, or error message).
    pub content: String,
    /// Whether the tool execution succeeded.
    pub success: bool,
    /// Optional elapsed duration of the execution.
    pub duration: Option<Duration>,
    /// Explicit flag marking the tool as read-only.
    pub is_read_only: bool,
    /// Maximum number of content lines displayed before truncation (defaults to 30).
    pub max_lines: Option<usize>,
}

impl ToolOutputCard {
    /// Creates a new `ToolOutputCard`.
    pub fn new(
        tool_name: impl Into<String>,
        title: impl Into<String>,
        content: impl Into<String>,
        success: bool,
        duration: Option<Duration>,
    ) -> Self {
        let tool_str = tool_name.into();
        let title_str = title.into();
        let auto_ro = is_read_only_tool(&tool_str);
        Self {
            tool_name: tool_str,
            title: title_str.clone(),
            summary: title_str,
            content: content.into(),
            success,
            duration,
            is_read_only: auto_ro,
            max_lines: None,
        }
    }

    /// Creates a successful tool execution card.
    pub fn success(
        tool_name: impl Into<String>,
        title: impl Into<String>,
        content: impl Into<String>,
        duration: Option<Duration>,
    ) -> Self {
        Self::new(tool_name, title, content, true, duration)
    }

    /// Creates a failed tool execution card.
    pub fn failure(
        tool_name: impl Into<String>,
        title: impl Into<String>,
        content: impl Into<String>,
        duration: Option<Duration>,
    ) -> Self {
        Self::new(tool_name, title, content, false, duration)
    }

    /// Sets title and summary.
    pub fn with_title(mut self, title: impl Into<String>) -> Self {
        let t = title.into();
        self.title = t.clone();
        self.summary = t;
        self
    }

    /// Sets title and summary.
    pub fn with_summary(mut self, summary: impl Into<String>) -> Self {
        let s = summary.into();
        self.title = s.clone();
        self.summary = s;
        self
    }

    /// Sets the tool name.
    pub fn with_tool_name(mut self, name: impl Into<String>) -> Self {
        self.tool_name = name.into();
        self
    }

    /// Sets output content.
    pub fn with_content(mut self, content: impl Into<String>) -> Self {
        self.content = content.into();
        self
    }

    /// Sets success status.
    pub fn with_success(mut self, success: bool) -> Self {
        self.success = success;
        self
    }

    /// Sets execution duration.
    pub fn with_duration(mut self, duration: impl Into<Option<Duration>>) -> Self {
        self.duration = duration.into();
        self
    }

    /// Explicitly flags whether the tool is read-only.
    pub fn with_read_only(mut self, read_only: bool) -> Self {
        self.is_read_only = read_only;
        self
    }

    /// Configures the maximum lines threshold before earlier lines are elided.
    pub fn with_max_lines(mut self, max_lines: usize) -> Self {
        self.max_lines = Some(max_lines);
        self
    }

    /// Returns the status enum.
    pub fn status(&self) -> ToolStatus {
        if self.success {
            ToolStatus::Success
        } else {
            ToolStatus::Failed
        }
    }

    /// Returns true if the tool execution succeeded.
    pub fn is_success(&self) -> bool {
        self.success
    }

    /// Returns true if the tool execution failed.
    pub fn is_failed(&self) -> bool {
        !self.success
    }

    /// Returns whether this card should be treated as a read-only tool.
    pub fn is_effective_read_only(&self) -> bool {
        self.is_read_only || is_read_only_tool(&self.tool_name)
    }

    /// Formats the header chip text (e.g. `bash: cargo test (15ms)`).
    pub fn header_chip_text(&self) -> String {
        let mut text = String::new();
        let tool = self.tool_name.trim();
        let title_or_summary = if !self.title.trim().is_empty() {
            self.title.trim()
        } else {
            self.summary.trim()
        };

        if !tool.is_empty() && !title_or_summary.is_empty() {
            // Avoid duplicate prefix like "bash: bash: cargo test" or "bash: bash"
            if title_or_summary.eq_ignore_ascii_case(tool) {
                text.push_str(tool);
            } else if title_or_summary.starts_with(&format!("{}:", tool))
                || title_or_summary.starts_with(&format!("{} ", tool))
            {
                text.push_str(title_or_summary);
            } else {
                text.push_str(&format!("{}: {}", tool, title_or_summary));
            }
        } else if !tool.is_empty() {
            text.push_str(tool);
        } else if !title_or_summary.is_empty() {
            text.push_str(title_or_summary);
        } else {
            text.push_str("tool");
        }

        if let Some(dur) = self.duration {
            let dur_str = format_duration(dur);
            text.push_str(&format!(" ({})", dur_str));
        }

        text
    }

    /// Renders a high-contrast rounded bordered card constrained to `terminal_width`.
    pub fn render(&self, terminal_width: usize) -> String {
        let term_width = if terminal_width == 0 {
            get_terminal_width().max(DEFAULT_CARD_WIDTH)
        } else {
            terminal_width.max(MIN_CARD_WIDTH)
        };

        let mut out = String::new();

        // 1. Determine header chip color:
        // - Red for failure
        // - Cyan for read-only tools (when successful)
        // - Green for success (mutations / non-read-only)
        let chip_color = if !self.success {
            ANSI_RED
        } else if self.is_effective_read_only() {
            ANSI_CYAN
        } else {
            ANSI_GREEN
        };

        let raw_chip_text = self.header_chip_text();
        // Top border structure:
        // "╭─ " (3 cols) + chip + " " (1 col) + dashes (at least 1 col) + "╮" (1 col)
        // Fixed overhead: 6 cols.
        let max_chip_width = term_width.saturating_sub(6).max(1);
        let chip_text = if visible_width(&raw_chip_text) > max_chip_width {
            truncate_ansi(&raw_chip_text, max_chip_width, "…")
        } else {
            raw_chip_text
        };

        let chip_vis_w = visible_width(&chip_text);
        let right_dash_count = term_width.saturating_sub(chip_vis_w + 5).max(1);

        // Top line: ╭─ <chip> <dashes>╮
        out.push_str(ANSI_GRAY);
        out.push_str(BOX_TOP_LEFT);
        out.push_str(BOX_HORIZONTAL);
        out.push(' ');
        out.push_str(ANSI_RESET);

        out.push_str(chip_color);
        out.push_str(&chip_text);
        out.push_str(ANSI_RESET);

        out.push(' ');
        out.push_str(ANSI_GRAY);
        out.push_str(&BOX_HORIZONTAL.repeat(right_dash_count));
        out.push_str(BOX_TOP_RIGHT);
        out.push_str(ANSI_RESET);
        out.push('\n');

        // 2. Prepare inner content lines
        // Inner width is terminal_width - 4 (account for "│ " on left and " │" on right)
        let inner_width = term_width.saturating_sub(4).max(1);

        let mut wrapped_lines = Vec::new();
        if !self.content.is_empty() {
            for raw_line in self.content.lines() {
                if raw_line.is_empty() {
                    wrapped_lines.push(String::new());
                } else {
                    let wrapped = wrap_ansi(raw_line, inner_width);
                    if wrapped.is_empty() {
                        wrapped_lines.push(String::new());
                    } else {
                        wrapped_lines.extend(wrapped);
                    }
                }
            }
        }

        // 3. Truncation: truncate giant outputs (>30 lines) with "… N earlier lines elided …"
        let max_lines = self.max_lines.unwrap_or(DEFAULT_MAX_LINES);
        if wrapped_lines.len() > max_lines {
            let elided_count = wrapped_lines.len() - max_lines;
            let elided_msg = format!("… {} earlier lines elided …", elided_count);
            let mut truncated = Vec::with_capacity(max_lines + 1);
            truncated.push(format!("{}{}{}", ANSI_DIM, elided_msg, ANSI_RESET));
            truncated.extend_from_slice(&wrapped_lines[elided_count..]);
            wrapped_lines = truncated;
        }

        // 4. Render content lines with vertical borders
        for line in wrapped_lines {
            let vis_w = visible_width(&line);
            let pad = inner_width.saturating_sub(vis_w);

            out.push_str(ANSI_GRAY);
            out.push_str(BOX_VERTICAL);
            out.push_str(ANSI_RESET);
            out.push(' ');

            out.push_str(&line);
            out.push_str(ANSI_RESET);
            if pad > 0 {
                out.push_str(&" ".repeat(pad));
            }

            out.push(' ');
            out.push_str(ANSI_GRAY);
            out.push_str(BOX_VERTICAL);
            out.push_str(ANSI_RESET);
            out.push('\n');
        }

        // 5. Bottom line: ╰<dashes>╯
        let bottom_dash_count = term_width.saturating_sub(2).max(1);
        out.push_str(ANSI_GRAY);
        out.push_str(BOX_BOTTOM_LEFT);
        out.push_str(&BOX_HORIZONTAL.repeat(bottom_dash_count));
        out.push_str(BOX_BOTTOM_RIGHT);
        out.push_str(ANSI_RESET);
        out.push('\n');

        out
    }

    /// Renders the card as plain text without ANSI color escape sequences.
    pub fn render_plain(&self, terminal_width: usize) -> String {
        strip_ansi(&self.render(terminal_width))
    }
}

// ============================================================================
// Multi-card Renderer
// ============================================================================

/// Renders a slice of tool cards into a combined string.
pub fn render_tool_cards(cards: &[ToolOutputCard], terminal_width: usize) -> String {
    if cards.is_empty() {
        return String::new();
    }
    let mut out = String::new();
    for (i, card) in cards.iter().enumerate() {
        if i > 0 {
            out.push('\n');
        }
        out.push_str(&card.render(terminal_width));
    }
    out
}

// ============================================================================
// Helper Functions
// ============================================================================

/// Checks whether a tool name is recognized as a read-only inspection tool.
pub fn is_read_only_tool(tool: &str) -> bool {
    let lower = tool.trim().to_ascii_lowercase();
    matches!(
        lower.as_str(),
        "read"
            | "file_read"
            | "read_file"
            | "view"
            | "cat"
            | "glob"
            | "file_glob"
            | "glob_files"
            | "grep"
            | "file_grep"
            | "grep_files"
            | "search"
            | "web_search"
            | "fetch"
            | "locate"
            | "find"
            | "show"
            | "inspect"
            | "list"
            | "tree"
            | "docgen"
            | "crate_docs"
            | "dep_graph"
            | "scout"
            | "ask"
            | "ask_user"
            | "memory"
            | "memory_read"
    )
}

/// Formats execution duration into human-readable text (e.g. `15ms`, `1.4s`, `2m 15s`).
pub fn format_duration(duration: Duration) -> String {
    let millis = duration.as_millis();
    if millis < 1000 {
        format!("{}ms", millis)
    } else if duration.as_secs() < 60 {
        let secs = duration.as_secs();
        let sub_ms = duration.subsec_millis();
        if sub_ms == 0 {
            format!("{}s", secs)
        } else {
            let tenths = sub_ms / 100;
            format!("{}.{}s", secs, tenths)
        }
    } else {
        let total_secs = duration.as_secs();
        let mins = total_secs / 60;
        let secs = total_secs % 60;
        if secs == 0 {
            format!("{}m", mins)
        } else {
            format!("{}m {}s", mins, secs)
        }
    }
}
