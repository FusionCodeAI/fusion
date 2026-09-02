use crossterm::{cursor, execute};
use std::io::{stdout, IsTerminal, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::watch;
use tokio::task::JoinHandle;

/// Standard Braille animation frames for smooth terminal progress indicators.
pub const BRAILLE_FRAMES: &[&str] = &["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];

/// Dot animation frames.
pub const DOTS_FRAMES: &[&str] = &["⣾", "⣽", "⣻", "⢿", "⡿", "⣟", "⣯", "⣷"];

/// Arc animation frames.
pub const ARC_FRAMES: &[&str] = &["◜", "◠", "◝", "◞", "◟", "◡"];

/// Pulse block frames.
pub const PULSE_FRAMES: &[&str] = &["█", "▓", "▒", "░", "▒", "▓"];

/// Plain ASCII fallback animation frames.
pub const ASCII_FRAMES: &[&str] = &["-", "\\", "|", "/"];

/// Spinner animation style presets.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SpinnerStyle {
    /// Braille dots (default, high aesthetic quality)
    #[default]
    Braille,
    /// Rotating dots
    Dots,
    /// Rotating arc
    Arc,
    /// Pulsing block
    Pulse,
    /// 7-bit ASCII spinner for terminals without UTF-8 support
    Ascii,
}

impl SpinnerStyle {
    /// Returns the animation frames for this spinner style.
    pub fn frames(&self) -> &'static [&'static str] {
        match self {
            Self::Braille => BRAILLE_FRAMES,
            Self::Dots => DOTS_FRAMES,
            Self::Arc => ARC_FRAMES,
            Self::Pulse => PULSE_FRAMES,
            Self::Ascii => ASCII_FRAMES,
        }
    }

    /// Returns recommended frame interval duration.
    pub fn frame_interval(&self) -> Duration {
        match self {
            Self::Braille | Self::Dots => Duration::from_millis(80),
            Self::Arc => Duration::from_millis(100),
            Self::Pulse | Self::Ascii => Duration::from_millis(120),
        }
    }
}

/// Internal dynamic state of the spinner.
#[derive(Debug, Clone)]
struct SpinnerState {
    prefix: Option<String>,
    message: String,
    details: Option<String>,
}

/// Dynamic single-line spinner for active tool execution, advisor reviews, and subagents.
pub struct Spinner;

impl Spinner {
    /// Start a new dynamic single-line spinner with a given message.
    pub fn start(message: impl Into<String>) -> SpinnerHandle {
        SpinnerHandle::new(SpinnerStyle::Braille, None, message.into(), None)
    }

    /// Start a new spinner with a specific style and initial message.
    pub fn with_style(style: SpinnerStyle, message: impl Into<String>) -> SpinnerHandle {
        SpinnerHandle::new(style, None, message.into(), None)
    }

    /// Convenience helper for starting a spinner for a tool execution.
    /// Example: `[bash] cargo build... (1.2s)`
    pub fn for_tool(tool_name: &str, target: &str) -> SpinnerHandle {
        let prefix = if is_color_enabled() {
            format!("\x1b[1;34m[{}]\x1b[0m", tool_name)
        } else {
            format!("[{}]", tool_name)
        };

        let message = if target.is_empty() {
            "running...".to_string()
        } else {
            format!("{}...", target)
        };

        SpinnerHandle::new(SpinnerStyle::Braille, Some(prefix), message, None)
    }

    /// Convenience helper for starting a spinner for an advisor review.
    /// Example: `[Advisor: Security] inspecting diffs... (0.5s)`
    pub fn for_advisor(advisor_name: &str, action: &str) -> SpinnerHandle {
        let prefix = if is_color_enabled() {
            format!("\x1b[1;35m[Advisor: {}]\x1b[0m", advisor_name)
        } else {
            format!("[Advisor: {}]", advisor_name)
        };

        let message = if action.is_empty() {
            "reviewing...".to_string()
        } else {
            action.to_string()
        };

        SpinnerHandle::new(SpinnerStyle::Braille, Some(prefix), message, None)
    }

    /// Convenience helper for starting a spinner for a subagent.
    /// Example: `[Agent: Scout] searching workspace... (2.1s)`
    pub fn for_subagent(agent_name: &str, task: &str) -> SpinnerHandle {
        let prefix = if is_color_enabled() {
            format!("\x1b[1;36m[Agent: {}]\x1b[0m", agent_name)
        } else {
            format!("[Agent: {}]", agent_name)
        };

        let message = if task.is_empty() {
            "working...".to_string()
        } else {
            format!("{}...", task)
        };

        SpinnerHandle::new(SpinnerStyle::Braille, Some(prefix), message, None)
    }

    /// Convenience helper for model text generation.
    /// Example: `[claude-3-7-sonnet] Generating response... (3.4s)`
    pub fn for_model(model_name: &str) -> SpinnerHandle {
        let prefix = if is_color_enabled() {
            format!("\x1b[1;32m[{}]\x1b[0m", model_name)
        } else {
            format!("[{}]", model_name)
        };

        SpinnerHandle::new(
            SpinnerStyle::Braille,
            Some(prefix),
            "Generating response...".to_string(),
            None,
        )
    }
}

/// Handle controlling an active dynamic spinner.
/// Automatically cleans up on `Drop` if not explicitly finished.
pub struct SpinnerHandle {
    state_tx: watch::Sender<SpinnerState>,
    start_time: Instant,
    stopped: Arc<AtomicBool>,
    task: Option<JoinHandle<()>>,
}

impl SpinnerHandle {
    fn new(
        style: SpinnerStyle,
        prefix: Option<String>,
        message: String,
        details: Option<String>,
    ) -> Self {
        let initial_state = SpinnerState {
            prefix,
            message,
            details,
        };

        let (state_tx, mut state_rx) = watch::channel(initial_state);
        let stopped = Arc::new(AtomicBool::new(false));
        let is_stopped = stopped.clone();
        let start_time = Instant::now();
        let frames = style.frames();
        let interval = style.frame_interval();
        let color_enabled = is_color_enabled();

        // Hide terminal cursor to prevent flicker
        if stdout().is_terminal() {
            let _ = execute!(stdout(), cursor::Hide);
        }

        let task = tokio::spawn(async move {
            let mut frame_idx = 0;
            let mut current_state = state_rx.borrow().clone();

            while !is_stopped.load(Ordering::Relaxed) {
                if state_rx.has_changed().unwrap_or(false) {
                    current_state = state_rx.borrow_and_update().clone();
                }

                let frame = frames[frame_idx % frames.len()];
                let elapsed = format_duration(start_time.elapsed());

                let mut line = String::with_capacity(128);
                if color_enabled {
                    line.push_str("\x1b[36m");
                    line.push_str(frame);
                    line.push_str("\x1b[0m ");
                } else {
                    line.push_str(frame);
                    line.push(' ');
                }
                // Optional prefix
                if let Some(pfx) = &current_state.prefix {
                    line.push_str(pfx);
                    line.push(' ');
                }

                // Main message
                line.push_str(&current_state.message);

                // Optional details
                if let Some(det) = &current_state.details {
                    line.push(' ');
                    if color_enabled {
                        line.push_str("\x1b[2;37m");
                        line.push_str(det);
                        line.push_str("\x1b[0m");
                    } else {
                        line.push_str(det);
                    }
                }

                // Elapsed time
                line.push(' ');
                if color_enabled {
                    line.push_str("\x1b[2;37m(");
                    line.push_str(&elapsed);
                    line.push_str(")\x1b[0m");
                } else {
                    line.push('(');
                    line.push_str(&elapsed);
                    line.push(')');
                }

                // Guard against terminal line wrap causing visual flicker
                let terminal_cols = get_terminal_width();
                let rendered_line = truncate_to_width(&line, terminal_cols.saturating_sub(1));

                // Single atomic write to prevent frame tearing / flickering:
                // \r (carriage return) + \x1b[2K (clear current line) + content
                let mut out = stdout();
                let _ = out.write_all(b"\r\x1b[2K");
                let _ = out.write_all(rendered_line.as_bytes());
                let _ = out.flush();

                frame_idx = (frame_idx + 1) % frames.len();
                tokio::time::sleep(interval).await;
            }
        });

        Self {
            state_tx,
            start_time,
            stopped,
            task: Some(task),
        }
    }

    /// Update the spinner message dynamically while running.
    pub fn set_message(&self, message: impl Into<String>) {
        let msg = message.into();
        let _ = self.state_tx.send_modify(|s| s.message = msg);
    }

    /// Update the prefix label (e.g. `[tool_name]`).
    pub fn set_prefix(&self, prefix: impl Into<String>) {
        let pfx = prefix.into();
        let _ = self.state_tx.send_modify(|s| s.prefix = Some(pfx));
    }

    /// Update auxiliary detail text.
    pub fn set_details(&self, details: impl Into<String>) {
        let det = details.into();
        let _ = self.state_tx.send_modify(|s| s.details = Some(det));
    }

    /// Clear auxiliary detail text.
    pub fn clear_details(&self) {
        let _ = self.state_tx.send_modify(|s| s.details = None);
    }

    /// Returns the elapsed duration since the spinner started.
    pub fn elapsed(&self) -> Duration {
        self.start_time.elapsed()
    }

    /// Returns true if the spinner is still active.
    pub fn is_running(&self) -> bool {
        !self.stopped.load(Ordering::Relaxed)
    }

    /// Mark as successful, collapse the spinner line into a permanent `✓ summary (elapsed)` line.
    pub fn success(mut self, message: &str) {
        let elapsed = format_duration(self.start_time.elapsed());
        self.cleanup();

        let formatted = if is_color_enabled() {
            format!(
                "\r\x1b[2K\x1b[1;32m✓\x1b[0m {} \x1b[2;37m({})\x1b[0m\n",
                message, elapsed
            )
        } else {
            format!("\r\x1b[2K✓ {} ({})\n", message, elapsed)
        };

        let mut out = stdout();
        let _ = out.write_all(formatted.as_bytes());
        let _ = out.flush();
    }

    /// Mark as failed, collapse the spinner line into a permanent `✗ summary (elapsed)` line.
    pub fn error(mut self, message: &str) {
        let elapsed = format_duration(self.start_time.elapsed());
        self.cleanup();

        let formatted = if is_color_enabled() {
            format!(
                "\r\x1b[2K\x1b[1;31m✗\x1b[0m {} \x1b[2;37m({})\x1b[0m\n",
                message, elapsed
            )
        } else {
            format!("\r\x1b[2K✗ {} ({})\n", message, elapsed)
        };

        let mut out = stdout();
        let _ = out.write_all(formatted.as_bytes());
        let _ = out.flush();
    }

    /// Mark with warning indicator: `! summary (elapsed)`.
    pub fn warn(mut self, message: &str) {
        let elapsed = format_duration(self.start_time.elapsed());
        self.cleanup();

        let formatted = if is_color_enabled() {
            format!(
                "\r\x1b[2K\x1b[1;33m!\x1b[0m {} \x1b[2;37m({})\x1b[0m\n",
                message, elapsed
            )
        } else {
            format!("\r\x1b[2K! {} ({})\n", message, elapsed)
        };

        let mut out = stdout();
        let _ = out.write_all(formatted.as_bytes());
        let _ = out.flush();
    }

    /// Mark with info indicator: `ℹ summary (elapsed)`.
    pub fn info(mut self, message: &str) {
        let elapsed = format_duration(self.start_time.elapsed());
        self.cleanup();

        let formatted = if is_color_enabled() {
            format!(
                "\r\x1b[2K\x1b[1;36mℹ\x1b[0m {} \x1b[2;37m({})\x1b[0m\n",
                message, elapsed
            )
        } else {
            format!("\r\x1b[2Kℹ {} ({})\n", message, elapsed)
        };

        let mut out = stdout();
        let _ = out.write_all(formatted.as_bytes());
        let _ = out.flush();
    }

    /// Finish with a custom symbol and color.
    pub fn finish_with_symbol(mut self, symbol: &str, ansi_color_code: &str, message: &str) {
        let elapsed = format_duration(self.start_time.elapsed());
        self.cleanup();

        let formatted = if is_color_enabled() {
            format!(
                "\r\x1b[2K{}{}\x1b[0m {} \x1b[2;37m({})\x1b[0m\n",
                ansi_color_code, symbol, message, elapsed
            )
        } else {
            format!("\r\x1b[2K{} {} ({})\n", symbol, message, elapsed)
        };

        let mut out = stdout();
        let _ = out.write_all(formatted.as_bytes());
        let _ = out.flush();
    }

    /// Stop spinner and clear the line without leaving any permanent output.
    pub fn stop(mut self) {
        self.cleanup();
        let mut out = stdout();
        let _ = out.write_all(b"\r\x1b[2K");
        let _ = out.flush();
    }

    fn cleanup(&mut self) {
        self.stopped.store(true, Ordering::Relaxed);
        if let Some(task) = self.task.take() {
            task.abort();
        }
        if stdout().is_terminal() {
            let _ = execute!(stdout(), cursor::Show);
        }
    }
}

impl Drop for SpinnerHandle {
    fn drop(&mut self) {
        if !self.stopped.load(Ordering::Relaxed) {
            self.cleanup();
            // Clear unfinished line on unexpected drop
            let mut out = stdout();
            let _ = out.write_all(b"\r\x1b[2K");
            let _ = out.flush();
        }
    }
}

/// Helper to format duration nicely: e.g. "450ms", "1.2s", "1m 12s", "1h 05m"
pub fn format_duration(d: Duration) -> String {
    let total_secs = d.as_secs();
    let millis = d.subsec_millis();

    if total_secs == 0 {
        format!("{}ms", millis)
    } else if total_secs < 60 {
        let tenths = millis / 100;
        format!("{}.{}s", total_secs, tenths)
    } else if total_secs < 3600 {
        let mins = total_secs / 60;
        let rem_secs = total_secs % 60;
        format!("{}m {:02}s", mins, rem_secs)
    } else {
        let hours = total_secs / 3600;
        let rem_mins = (total_secs % 3600) / 60;
        format!("{}h {:02}m", hours, rem_mins)
    }
}

/// Check if ANSI colors should be emitted.
fn is_color_enabled() -> bool {
    if std::env::var_os("NO_COLOR").is_some() {
        return false;
    }
    if let Ok(term) = std::env::var("TERM") {
        if term == "dumb" {
            return false;
        }
    }
    stdout().is_terminal()
}

/// Get current terminal column width safely.
fn get_terminal_width() -> usize {
    crossterm::terminal::size()
        .map(|(w, _)| w as usize)
        .unwrap_or(80)
}

/// Calculate visible printable column width of a string, ignoring ANSI escape sequences.
pub fn visible_width(s: &str) -> usize {
    let mut in_escape = false;
    let mut width = 0;

    for ch in s.chars() {
        if in_escape {
            if ch == 'm' || ch == 'K' || ch == 'H' || ch == 'J' {
                in_escape = false;
            }
        } else if ch == '\x1b' {
            in_escape = true;
        } else if !ch.is_control() {
            width += 1;
        }
    }

    width
}

/// Truncate a formatted string to `max_width` visible characters, preserving ANSI escape codes
/// and cleanly appending `…\x1b[0m` if truncated.
pub fn truncate_to_width(s: &str, max_width: usize) -> String {
    if max_width == 0 {
        return String::new();
    }

    let current_visible = visible_width(s);
    if current_visible <= max_width {
        return s.to_string();
    }

    let target_width = max_width.saturating_sub(1); // reserve 1 column for '…'
    let mut result = String::with_capacity(s.len());
    let mut visible_count = 0;
    let mut in_escape = false;

    for ch in s.chars() {
        if in_escape {
            result.push(ch);
            if ch == 'm' || ch == 'K' || ch == 'H' || ch == 'J' {
                in_escape = false;
            }
        } else if ch == '\x1b' {
            in_escape = true;
            result.push(ch);
        } else if !ch.is_control() {
            if visible_count < target_width {
                result.push(ch);
                visible_count += 1;
            } else {
                break;
            }
        }
    }

    result.push('…');
    result.push_str("\x1b[0m");
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_format_duration_millis() {
        assert_eq!(format_duration(Duration::from_millis(0)), "0ms");
        assert_eq!(format_duration(Duration::from_millis(350)), "350ms");
        assert_eq!(format_duration(Duration::from_millis(999)), "999ms");
    }

    #[test]
    fn test_format_duration_secs() {
        assert_eq!(format_duration(Duration::from_millis(1000)), "1.0s");
        assert_eq!(format_duration(Duration::from_millis(1250)), "1.2s");
        assert_eq!(format_duration(Duration::from_secs(59)), "59.0s");
    }

    #[test]
    fn test_format_duration_mins() {
        assert_eq!(format_duration(Duration::from_secs(60)), "1m 00s");
        assert_eq!(format_duration(Duration::from_secs(75)), "1m 15s");
        assert_eq!(format_duration(Duration::from_secs(3599)), "59m 59s");
    }

    #[test]
    fn test_format_duration_hours() {
        assert_eq!(format_duration(Duration::from_secs(3600)), "1h 00m");
        assert_eq!(format_duration(Duration::from_secs(3665)), "1h 01m");
        assert_eq!(format_duration(Duration::from_secs(7320)), "2h 02m");
    }

    #[test]
    fn test_visible_width() {
        assert_eq!(visible_width("hello world"), 11);
        assert_eq!(
            visible_width("\x1b[1;34m[bash]\x1b[0m cargo build..."),
            21
        );
        assert_eq!(
            visible_width("\x1b[1;32m✓\x1b[0m success \x1b[2;37m(1.2s)\x1b[0m"),
            16
        );
    }

    #[test]
    fn test_truncate_to_width() {
        let plain = "This is a very long message that needs truncation";
        let truncated = truncate_to_width(plain, 20);
        assert!(visible_width(&truncated) <= 20);
        assert!(truncated.ends_with("…\x1b[0m"));

        let styled = "\x1b[1;34m[tool]\x1b[0m doing something long";
        let trunc_styled = truncate_to_width(styled, 15);
        assert!(visible_width(&trunc_styled) <= 15);
    }

    #[test]
    fn test_spinner_styles() {
        assert_eq!(SpinnerStyle::Braille.frames().len(), 10);
        assert_eq!(SpinnerStyle::Dots.frames().len(), 8);
        assert_eq!(SpinnerStyle::Arc.frames().len(), 6);
        assert_eq!(SpinnerStyle::Pulse.frames().len(), 6);
        assert_eq!(SpinnerStyle::Ascii.frames().len(), 4);
    }
}
