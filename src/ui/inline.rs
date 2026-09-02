use crossterm::{cursor, execute, terminal};
use ratatui::{
    backend::CrosstermBackend,
    buffer::Buffer,
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span, Text},
    widgets::{Block, BorderType, Borders, Paragraph, Widget, Wrap},
    Frame, Terminal, TerminalOptions, Viewport,
};
use std::io::{stdout, Stdout, Write};

/// Information displayed in the inline status bar.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct StatusInfo {
    /// Active provider name (e.g. "deepseek", "anthropic", "openai", "ollama")
    pub provider: String,
    /// Active model name (e.g. "deepseek-chat", "claude-3-5-sonnet", "gpt-4o")
    pub model: String,
    /// Active subagent name if currently executing a subagent task
    pub active_agent: Option<String>,
    /// Active advisor critique name if currently reviewing
    pub active_advisor: Option<String>,
    /// Token usage summary string (e.g. "4.2k tokens / $0.012")
    pub token_usage: Option<String>,
    /// Current activity status message (e.g. "Thinking...", "Editing src/main.rs")
    pub status: String,
}

impl StatusInfo {
    /// Create a new StatusInfo with given provider and model.
    pub fn new(provider: impl Into<String>, model: impl Into<String>) -> Self {
        Self {
            provider: provider.into(),
            model: model.into(),
            active_agent: None,
            active_advisor: None,
            token_usage: None,
            status: String::new(),
        }
    }

    /// Set the active subagent name.
    pub fn with_agent(mut self, agent: impl Into<String>) -> Self {
        self.active_agent = Some(agent.into());
        self
    }

    /// Set the active advisor critique name.
    pub fn with_advisor(mut self, advisor: impl Into<String>) -> Self {
        self.active_advisor = Some(advisor.into());
        self
    }

    /// Set token usage summary.
    pub fn with_tokens(mut self, tokens: impl Into<String>) -> Self {
        self.token_usage = Some(tokens.into());
        self
    }

    /// Set current status message.
    pub fn with_status(mut self, status: impl Into<String>) -> Self {
        self.status = status.into();
        self
    }
}

/// Default inline viewport height in rows when not specified.
pub const DEFAULT_INLINE_HEIGHT: u16 = 4;

/// Minimum safe inline viewport height.
pub const MIN_INLINE_HEIGHT: u16 = 2;

/// Inline terminal wrapper using Ratatui `Viewport::Inline(height)`.
///
/// Renders an interactive status view / prompt at the bottom of the terminal
/// while allowing standard stdout scrollback above it. Completed output can be
/// inserted before the viewport into the terminal scrollback via `insert_before`.
///
/// Tailored for cross-platform support including Android (Termux), macOS, Linux, and Windows.
pub struct InlineTerminal {
    terminal: Terminal<CrosstermBackend<Stdout>>,
    height: u16,
    last_cols: u16,
    last_rows: u16,
    is_active: bool,
}

impl InlineTerminal {
    /// Create a new inline terminal with the given height in rows.
    /// Clamps height safely against current terminal size to prevent overflow.
    pub fn new(height: u16) -> std::io::Result<Self> {
        let (cols, rows) = Self::terminal_size();
        let safe_height = Self::clamp_height(height, rows);

        let backend = CrosstermBackend::new(stdout());
        let terminal = Terminal::with_options(
            backend,
            TerminalOptions {
                viewport: Viewport::Inline(safe_height),
            },
        )?;

        Ok(Self {
            terminal,
            height: safe_height,
            last_cols: cols,
            last_rows: rows,
            is_active: true,
        })
    }

    /// Create an inline terminal with the default height (4 rows).
    pub fn try_default() -> std::io::Result<Self> {
        Self::new(DEFAULT_INLINE_HEIGHT)
    }

    /// Clamp viewport height so it fits safely in current terminal rows.
    pub fn clamp_height(requested_height: u16, terminal_rows: u16) -> u16 {
        if terminal_rows <= 3 {
            1
        } else {
            let max_allowed = terminal_rows.saturating_sub(1);
            requested_height.clamp(1, max_allowed)
        }
    }

    /// Returns the current inline viewport height in rows.
    pub fn height(&self) -> u16 {
        self.height
    }

    /// Returns current terminal size `(columns, rows)`.
    pub fn terminal_size() -> (u16, u16) {
        terminal::size().unwrap_or((80, 24))
    }

    /// Check if terminal is narrow (e.g. Mobile / Termux portrait mode < 60 cols).
    pub fn is_narrow() -> bool {
        let (cols, _) = Self::terminal_size();
        cols < 60
    }

    /// Detect if running in Android Termux environment.
    pub fn is_termux() -> bool {
        std::env::var("TERMUX_VERSION").is_ok()
            || std::env::var("PREFIX")
                .map(|p| p.contains("com.termux"))
                .unwrap_or(false)
    }

    /// Calculates Termux-adaptive viewport height based on screen dimensions.
    /// On small mobile screens (e.g. keyboard up, 10-15 rows), reduces height to 2-3 rows.
    pub fn termux_adaptive_height(preferred: u16) -> u16 {
        let (_, rows) = Self::terminal_size();
        if rows < 14 {
            2
        } else if rows < 20 {
            3.min(preferred)
        } else {
            preferred
        }
    }

    /// Check if the terminal window size has changed, and autoresize if needed.
    /// Returns `Ok(true)` if a resize occurred.
    pub fn check_resize(&mut self) -> std::io::Result<bool> {
        let (cols, rows) = Self::terminal_size();
        if cols != self.last_cols || rows != self.last_rows {
            self.last_cols = cols;
            self.last_rows = rows;
            let safe_height = Self::clamp_height(self.height, rows);
            if safe_height != self.height {
                self.resize_viewport(safe_height)?;
            } else {
                self.terminal.autoresize()?;
            }
            Ok(true)
        } else {
            Ok(false)
        }
    }

    /// Explicitly handle a resize event with known `cols` and `rows`.
    pub fn handle_resize(&mut self, cols: u16, rows: u16) -> std::io::Result<()> {
        self.last_cols = cols;
        self.last_rows = rows;
        let safe_height = Self::clamp_height(self.height, rows);
        if safe_height != self.height {
            self.resize_viewport(safe_height)?;
        } else {
            self.terminal.autoresize()?;
        }
        Ok(())
    }

    /// Draw a frame using the given rendering closure.
    /// Checks for terminal resize before rendering.
    pub fn draw<F>(&mut self, f: F) -> std::io::Result<()>
    where
        F: FnOnce(&mut Frame),
    {
        let _ = self.check_resize();
        self.terminal.draw(f)?;
        Ok(())
    }

    /// Clear the inline terminal viewport area.
    pub fn clear(&mut self) -> std::io::Result<()> {
        self.terminal.clear()?;
        Ok(())
    }

    /// Recreate or resize the inline viewport to a new height in rows.
    pub fn resize_viewport(&mut self, new_height: u16) -> std::io::Result<()> {
        let (_, rows) = Self::terminal_size();
        let safe_height = Self::clamp_height(new_height, rows);

        if self.height != safe_height {
            let _ = self.terminal.clear();
            let backend = CrosstermBackend::new(stdout());
            self.terminal = Terminal::with_options(
                backend,
                TerminalOptions {
                    viewport: Viewport::Inline(safe_height),
                },
            )?;
            self.height = safe_height;
        }
        Ok(())
    }

    /// Legacy alias for `resize_viewport`.
    pub fn resize(&mut self, new_height: u16) -> std::io::Result<()> {
        self.resize_viewport(new_height)
    }

    /// Insert content above the inline viewport into the terminal scrollback buffer.
    ///
    /// The closure receives a mutable `&mut Buffer` with dimensions `(terminal_cols, height)`.
    /// The lines drawn into this buffer are pushed cleanly into scrollback above the viewport.
    pub fn insert_before<F>(&mut self, height: u16, draw_fn: F) -> std::io::Result<()>
    where
        F: FnOnce(&mut Buffer),
    {
        if height == 0 {
            return Ok(());
        }
        self.terminal.insert_before(height, draw_fn)
    }

    /// Insert any Ratatui `Widget` before the viewport into scrollback.
    pub fn insert_before_widget<W: Widget>(&mut self, height: u16, widget: W) -> std::io::Result<()> {
        self.insert_before(height, |buf| {
            let area = buf.area;
            widget.render(area, buf);
        })
    }

    /// Insert text lines before the viewport into scrollback.
    pub fn insert_before_lines<'a, I>(&mut self, lines: I) -> std::io::Result<()>
    where
        I: IntoIterator<Item = Line<'a>>,
    {
        let line_vec: Vec<Line<'a>> = lines.into_iter().collect();
        let count = line_vec.len() as u16;
        if count == 0 {
            return Ok(());
        }

        let paragraph = Paragraph::new(line_vec);
        self.insert_before_widget(count, paragraph)
    }

    /// Insert plain or formatted text string before the viewport into scrollback.
    /// Computes wrapped height based on terminal column width.
    pub fn insert_before_text(&mut self, text: &str) -> std::io::Result<()> {
        let (cols, _) = Self::terminal_size();
        let height = calculate_text_height(text, cols);
        if height == 0 {
            return Ok(());
        }

        let paragraph = Paragraph::new(text.to_string()).wrap(Wrap { trim: false });
        self.insert_before_widget(height, paragraph)
    }

    /// Insert a stylized card before the viewport into scrollback.
    pub fn insert_before_card(
        &mut self,
        title: &str,
        content: &str,
        border_color: Color,
    ) -> std::io::Result<()> {
        let (cols, _) = Self::terminal_size();
        // Width available inside card borders (minus left/right borders)
        let inner_width = cols.saturating_sub(4).max(10);
        let inner_height = calculate_text_height(content, inner_width);
        let total_height = inner_height.saturating_add(2); // Top and bottom borders

        self.insert_before(total_height, |buf| {
            let area = buf.area;
            let block = Block::default()
                .title(format!(" {} ", title))
                .borders(Borders::ALL)
                .border_type(BorderType::Rounded)
                .border_style(Style::default().fg(border_color));

            let paragraph = Paragraph::new(content.to_string())
                .block(block)
                .wrap(Wrap { trim: true });

            paragraph.render(area, buf);
        })
    }

    /// Insert a status line snapshot into scrollback before moving on.
    pub fn insert_before_status(&mut self, info: &StatusInfo) -> std::io::Result<()> {
        let (cols, _) = Self::terminal_size();
        let is_compact = cols < 60;

        let mut spans = Vec::new();
        spans.push(Span::styled("✦ ", Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)));
        spans.push(Span::styled(&info.provider, Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)));
        spans.push(Span::raw("/"));
        spans.push(Span::styled(&info.model, Style::default().fg(Color::White).add_modifier(Modifier::BOLD)));

        if let Some(agent) = &info.active_agent {
            spans.push(Span::raw(" │ "));
            spans.push(Span::styled(format!("Agent:{}", agent), Style::default().fg(Color::Magenta)));
        }

        if let Some(advisor) = &info.active_advisor {
            spans.push(Span::raw(" │ "));
            spans.push(Span::styled(format!("Advisor:{}", advisor), Style::default().fg(Color::Yellow)));
        }

        if !info.status.is_empty() {
            spans.push(Span::raw(" │ "));
            spans.push(Span::styled(&info.status, Style::default().fg(Color::Green)));
        }

        if !is_compact {
            if let Some(tokens) = &info.token_usage {
                spans.push(Span::raw(" │ "));
                spans.push(Span::styled(tokens, Style::default().fg(Color::DarkGray)));
            }
        }

        let paragraph = Paragraph::new(Line::from(spans));
        self.insert_before_widget(1, paragraph)
    }

    /// Render a standard status bar in the inline area.
    pub fn render_status(&mut self, info: &StatusInfo) -> std::io::Result<()> {
        self.draw(|f| {
            let size = f.area();
            render_status_bar(f, size, info);
        })
    }

    /// Reset and flush the inline terminal, restoring normal cursor visibility.
    pub fn finish(&mut self) -> std::io::Result<()> {
        if self.is_active {
            let _ = self.terminal.clear();
            let _ = execute!(stdout(), cursor::Show);
            let _ = stdout().flush();
            self.is_active = false;
        }
        Ok(())
    }
}

impl Drop for InlineTerminal {
    fn drop(&mut self) {
        if self.is_active {
            let _ = self.terminal.clear();
            let _ = execute!(stdout(), cursor::Show);
            let _ = stdout().flush();
            self.is_active = false;
        }
    }
}

/// Calculate required height in rows for text wrapped to a given column width.
pub fn calculate_text_height(text: &str, width: u16) -> u16 {
    if text.is_empty() {
        return 0;
    }
    let effective_width = width.max(1) as usize;
    let mut total_lines: u16 = 0;

    for line in text.lines() {
        let line_len = line.chars().count();
        if line_len == 0 {
            total_lines = total_lines.saturating_add(1);
        } else {
            let wrapped = (line_len + effective_width - 1) / effective_width;
            total_lines = total_lines.saturating_add(wrapped.max(1) as u16);
        }
    }

    // If string ends with trailing newline, count empty line
    if text.ends_with('\n') {
        total_lines = total_lines.saturating_add(1);
    }

    total_lines.max(1)
}

/// Helper function to render a clean, compact, Termux-friendly status bar inside a Frame area.
pub fn render_status_bar(f: &mut Frame, area: Rect, info: &StatusInfo) {
    let cols = area.width;
    let is_compact = cols < 60; // Mobile / Termux portrait friendly

    let mut spans = Vec::new();

    // Provider & Model tag
    spans.push(Span::styled(
        " ✦ ",
        Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD),
    ));
    spans.push(Span::styled(
        &info.provider,
        Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD),
    ));
    spans.push(Span::raw("/"));
    spans.push(Span::styled(
        &info.model,
        Style::default().fg(Color::White).add_modifier(Modifier::BOLD),
    ));

    // Agent tag if active
    if let Some(agent) = &info.active_agent {
        spans.push(Span::raw(" │ "));
        spans.push(Span::styled(
            if is_compact {
                format!("A:{}", agent)
            } else {
                format!("Agent: {}", agent)
            },
            Style::default().fg(Color::Magenta).add_modifier(Modifier::BOLD),
        ));
    }

    // Advisor tag if active
    if let Some(advisor) = &info.active_advisor {
        spans.push(Span::raw(" │ "));
        spans.push(Span::styled(
            if is_compact {
                format!("Adv:{}", advisor)
            } else {
                format!("Advisor: {}", advisor)
            },
            Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD),
        ));
    }

    // Status message
    if !info.status.is_empty() {
        spans.push(Span::raw(" │ "));
        spans.push(Span::styled(
            &info.status,
            Style::default().fg(Color::Green),
        ));
    }

    // Token usage if space permits
    if !is_compact {
        if let Some(tokens) = &info.token_usage {
            spans.push(Span::raw(" │ "));
            spans.push(Span::styled(
                tokens,
                Style::default().fg(Color::DarkGray),
            ));
        }
    }

    let paragraph = Paragraph::new(Line::from(spans))
        .block(
            Block::default()
                .borders(Borders::TOP)
                .border_type(BorderType::Plain)
                .border_style(Style::default().fg(Color::DarkGray)),
        )
        .wrap(Wrap { trim: true });

    f.render_widget(paragraph, area);
}

/// Helper to render an informational card with a colored border.
pub fn render_card(f: &mut Frame, area: Rect, title: &str, content: &str, border_color: Color) {
    let block = Block::default()
        .title(format!(" {} ", title))
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(border_color));

    let paragraph = Paragraph::new(content)
        .block(block)
        .wrap(Wrap { trim: true });

    f.render_widget(paragraph, area);
}

/// Helper to render an advisor critique card inside an inline Frame.
pub fn render_critique_card(
    f: &mut Frame,
    area: Rect,
    advisor_name: &str,
    approved: bool,
    critique: &str,
) {
    let (border_color, status_icon, status_text) = if approved {
        (Color::Green, "✓", "APPROVED")
    } else {
        (Color::Yellow, "!", "CRITIQUE")
    };

    let title = format!(" [Advisor: {}] {} {} ", advisor_name, status_icon, status_text);

    let block = Block::default()
        .title(title)
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(border_color));

    let paragraph = Paragraph::new(critique)
        .block(block)
        .wrap(Wrap { trim: true });

    f.render_widget(paragraph, area);
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::backend::TestBackend;

    #[test]
    fn test_status_info_builder() {
        let info = StatusInfo::new("deepseek", "deepseek-chat")
            .with_agent("Coder")
            .with_advisor("SecAdvisor")
            .with_tokens("1.2k tokens")
            .with_status("Running tool...");

        assert_eq!(info.provider, "deepseek");
        assert_eq!(info.model, "deepseek-chat");
        assert_eq!(info.active_agent.as_deref(), Some("Coder"));
        assert_eq!(info.active_advisor.as_deref(), Some("SecAdvisor"));
        assert_eq!(info.token_usage.as_deref(), Some("1.2k tokens"));
        assert_eq!(info.status, "Running tool...");
    }

    #[test]
    fn test_clamp_height() {
        assert_eq!(InlineTerminal::clamp_height(4, 24), 4);
        assert_eq!(InlineTerminal::clamp_height(10, 8), 7);
        assert_eq!(InlineTerminal::clamp_height(5, 2), 1);
        assert_eq!(InlineTerminal::clamp_height(0, 24), 1);
    }

    #[test]
    fn test_calculate_text_height() {
        assert_eq!(calculate_text_height("", 80), 0);
        assert_eq!(calculate_text_height("Hello", 80), 1);
        assert_eq!(calculate_text_height("Line 1\nLine 2\nLine 3", 80), 3);
        // Wrapping: 25 chars in 10-width column = 3 lines
        let text_25 = "1234567890123456789012345";
        assert_eq!(calculate_text_height(text_25, 10), 3);
    }

    #[test]
    fn test_render_status_bar_widget() {
        let backend = TestBackend::new(80, 2);
        let mut terminal = Terminal::new(backend).unwrap();

        let info = StatusInfo::new("deepseek", "deepseek-chat")
            .with_status("Ready");

        terminal.draw(|f| {
            let area = f.area();
            render_status_bar(f, area, &info);
        }).unwrap();

        let buffer = terminal.backend().buffer();
        assert!(buffer.content().iter().any(|cell| cell.symbol() == "✦"));
    }

    #[test]
    fn test_render_card_widget() {
        let backend = TestBackend::new(60, 4);
        let mut terminal = Terminal::new(backend).unwrap();

        terminal.draw(|f| {
            let area = f.area();
            render_card(f, area, "Advisor Notice", "Looks good to proceed.", Color::Cyan);
        }).unwrap();

        let buffer = terminal.backend().buffer();
        assert!(buffer.content().iter().any(|cell| cell.symbol() == "A" || cell.symbol() == "Notice"));
    }

    #[test]
    fn test_render_critique_card_widget() {
        let backend = TestBackend::new(70, 5);
        let mut terminal = Terminal::new(backend).unwrap();

        terminal.draw(|f| {
            let area = f.area();
            render_critique_card(f, area, "SecurityAdvisor", false, "Avoid hardcoded credentials.");
        }).unwrap();

        let buffer = terminal.backend().buffer();
        assert!(buffer.content().iter().any(|cell| cell.symbol() == "!" || cell.symbol() == "SecurityAdvisor"));
    }

    #[test]
    fn test_termux_adaptive_height() {
        // Test that clamp and adaptive height never produce 0
        let h = InlineTerminal::termux_adaptive_height(4);
        assert!(h >= 1);
    }
}
