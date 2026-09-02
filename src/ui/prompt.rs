use crossterm::{
    cursor,
    event::{self, Event, KeyCode, KeyEventKind, KeyModifiers},
    execute,
    terminal::{self, ClearType},
};
use std::io::{stdout, Write};

/// Result returned from reading interactive user input.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PromptResult {
    /// User submitted input text.
    Submit(String),
    /// User canceled input (Ctrl+C).
    Cancel,
    /// User requested exit / EOF (Ctrl+D on empty input).
    Exit,
}

/// Interactive terminal prompt supporting line editing, multiline input,
/// history navigation, and ANSI indicators.
pub struct Prompt {
    history: Vec<String>,
    history_idx: Option<usize>,
    prompt_symbol: String,
    multiline_symbol: String,
    placeholder: Option<String>,
}

impl Default for Prompt {
    fn default() -> Self {
        Self::new()
    }
}

impl Prompt {
    /// Create a new interactive Prompt.
    pub fn new() -> Self {
        Self {
            history: Vec::new(),
            history_idx: None,
            prompt_symbol: "\x1b[1;36m❯\x1b[0m ".to_string(),
            multiline_symbol: "\x1b[2;37m···\x1b[0m ".to_string(),
            placeholder: Some("Type a message or /help...".to_string()),
        }
    }

    /// Set initial history entries.
    pub fn with_history(mut self, history: Vec<String>) -> Self {
        self.history = history;
        self
    }

    /// Set a custom prompt symbol for the first line.
    pub fn with_prompt_symbol(mut self, symbol: impl Into<String>) -> Self {
        self.prompt_symbol = symbol.into();
        self
    }

    /// Set a custom multiline symbol for subsequent lines.
    pub fn with_multiline_symbol(mut self, symbol: impl Into<String>) -> Self {
        self.multiline_symbol = symbol.into();
        self
    }

    /// Set placeholder text shown when buffer is empty.
    pub fn with_placeholder(mut self, placeholder: impl Into<String>) -> Self {
        self.placeholder = Some(placeholder.into());
        self
    }

    /// Returns a slice of recorded history entries.
    pub fn history(&self) -> &[String] {
        &self.history
    }

    /// Append a completed line to history.
    pub fn add_history(&mut self, entry: impl Into<String>) {
        let entry = entry.into();
        if !entry.trim().is_empty() {
            // Avoid duplicate consecutive entries
            if self.history.last().map(|s| s.as_str()) != Some(&entry) {
                self.history.push(entry);
            }
        }
    }

    /// Read an interactive line / multiline input from user.
    pub fn read_input(&mut self) -> std::io::Result<PromptResult> {
        let _raw_guard = RawModeGuard::enter()?;
        let mut buffer: Vec<char> = Vec::new();
        let mut cursor_pos: usize = 0;
        let mut saved_current = String::new();
        self.history_idx = None;

        let mut last_rendered_lines = 0;
        let mut last_cursor_row = 0;

        // Render initial state
        self.render(
            &buffer,
            cursor_pos,
            &mut last_rendered_lines,
            &mut last_cursor_row,
        )?;

        loop {
            match event::read()? {
                Event::Key(key) => {
                    // In crossterm, ignore Release events
                    if key.kind == KeyEventKind::Release {
                        continue;
                    }

                    match (key.code, key.modifiers) {
                        // Enter: Submit or Multiline if ending with backslash
                        (KeyCode::Enter, KeyModifiers::NONE) => {
                            if !buffer.is_empty() && buffer[buffer.len() - 1] == '\\' {
                                buffer.pop();
                                if cursor_pos > buffer.len() {
                                    cursor_pos = buffer.len();
                                }
                                buffer.insert(cursor_pos, '\n');
                                cursor_pos += 1;
                            } else {
                                let text: String = buffer.iter().collect();
                                let rows_down = last_rendered_lines.saturating_sub(1 + last_cursor_row);
                                if rows_down > 0 {
                                    let _ = execute!(stdout(), cursor::MoveDown(rows_down as u16));
                                }
                                println!();
                                let _ = stdout().flush();

                                if !text.trim().is_empty() {
                                    self.add_history(text.clone());
                                }
                                return Ok(PromptResult::Submit(text));
                            }
                        }

                        // Multiline newline (Ctrl+J, Ctrl+Enter, Alt+Enter, Shift+Enter)
                        (KeyCode::Char('j'), KeyModifiers::CONTROL)
                        | (KeyCode::Enter, KeyModifiers::CONTROL)
                        | (KeyCode::Enter, KeyModifiers::ALT)
                        | (KeyCode::Enter, KeyModifiers::SHIFT) => {
                            buffer.insert(cursor_pos, '\n');
                            cursor_pos += 1;
                        }

                        // Cancel current turn (Ctrl+C)
                        (KeyCode::Char('c'), KeyModifiers::CONTROL) => {
                            let rows_down = last_rendered_lines.saturating_sub(1 + last_cursor_row);
                            if rows_down > 0 {
                                let _ = execute!(stdout(), cursor::MoveDown(rows_down as u16));
                            }
                            println!();
                            let _ = stdout().flush();
                            return Ok(PromptResult::Cancel);
                        }

                        // Exit on empty or delete character under cursor (Ctrl+D)
                        (KeyCode::Char('d'), KeyModifiers::CONTROL) => {
                            if buffer.is_empty() {
                                println!();
                                let _ = stdout().flush();
                                return Ok(PromptResult::Exit);
                            } else if cursor_pos < buffer.len() {
                                buffer.remove(cursor_pos);
                            }
                        }

                        // Word delete backward (Ctrl+W or Alt+Backspace)
                        (KeyCode::Char('w'), KeyModifiers::CONTROL)
                        | (KeyCode::Backspace, KeyModifiers::ALT) => {
                            let prev = prev_word_pos(&buffer, cursor_pos);
                            if prev < cursor_pos {
                                buffer.drain(prev..cursor_pos);
                                cursor_pos = prev;
                            }
                        }

                        // Backspace
                        (KeyCode::Backspace, _) | (KeyCode::Char('h'), KeyModifiers::CONTROL) => {
                            if cursor_pos > 0 {
                                buffer.remove(cursor_pos - 1);
                                cursor_pos -= 1;
                            }
                        }

                        // Word delete forward (Alt+D or Alt+Delete or Ctrl+Delete)
                        (KeyCode::Char('d'), KeyModifiers::ALT)
                        | (KeyCode::Delete, KeyModifiers::ALT)
                        | (KeyCode::Delete, KeyModifiers::CONTROL) => {
                            let next = next_word_pos(&buffer, cursor_pos);
                            if next > cursor_pos {
                                buffer.drain(cursor_pos..next);
                            }
                        }

                        // Delete
                        (KeyCode::Delete, _) => {
                            if cursor_pos < buffer.len() {
                                buffer.remove(cursor_pos);
                            }
                        }

                        // Clear line to start (Ctrl+U)
                        (KeyCode::Char('u'), KeyModifiers::CONTROL) => {
                            let (cur_line, cur_col, line_ranges) = get_line_info(&buffer, cursor_pos);
                            let (start, _) = line_ranges[cur_line];
                            if cur_col > 0 {
                                buffer.drain(start..cursor_pos);
                                cursor_pos = start;
                            } else if cur_line > 0 {
                                buffer.remove(start - 1);
                                cursor_pos = start - 1;
                            }
                        }

                        // Clear line to end (Ctrl+K)
                        (KeyCode::Char('k'), KeyModifiers::CONTROL) => {
                            let (cur_line, cur_col, line_ranges) = get_line_info(&buffer, cursor_pos);
                            let (start, len) = line_ranges[cur_line];
                            if cur_col < len {
                                buffer.drain(cursor_pos..start + len);
                            } else if cur_line + 1 < line_ranges.len() {
                                buffer.remove(start + len);
                            }
                        }

                        // Home / Ctrl+A
                        (KeyCode::Home, _) | (KeyCode::Char('a'), KeyModifiers::CONTROL) => {
                            let (cur_line, cur_col, line_ranges) = get_line_info(&buffer, cursor_pos);
                            let (start, _) = line_ranges[cur_line];
                            if cur_col > 0 {
                                cursor_pos = start;
                            } else {
                                cursor_pos = 0;
                            }
                        }

                        // End / Ctrl+E
                        (KeyCode::End, _) | (KeyCode::Char('e'), KeyModifiers::CONTROL) => {
                            let (cur_line, cur_col, line_ranges) = get_line_info(&buffer, cursor_pos);
                            let (start, len) = line_ranges[cur_line];
                            if cur_col < len {
                                cursor_pos = start + len;
                            } else {
                                cursor_pos = buffer.len();
                            }
                        }

                        // Word left: Ctrl+Left / Alt+Left / Alt+B
                        (KeyCode::Left, KeyModifiers::CONTROL)
                        | (KeyCode::Left, KeyModifiers::ALT)
                        | (KeyCode::Char('b'), KeyModifiers::ALT) => {
                            cursor_pos = prev_word_pos(&buffer, cursor_pos);
                        }

                        // Word right: Ctrl+Right / Alt+Right / Alt+F
                        (KeyCode::Right, KeyModifiers::CONTROL)
                        | (KeyCode::Right, KeyModifiers::ALT)
                        | (KeyCode::Char('f'), KeyModifiers::ALT) => {
                            cursor_pos = next_word_pos(&buffer, cursor_pos);
                        }

                        // Left arrow
                        (KeyCode::Left, _) => {
                            if cursor_pos > 0 {
                                cursor_pos -= 1;
                            }
                        }

                        // Right arrow
                        (KeyCode::Right, _) => {
                            if cursor_pos < buffer.len() {
                                cursor_pos += 1;
                            }
                        }

                        // Up arrow: multiline line-up or history recall
                        (KeyCode::Up, _) => {
                            let (cur_line, cur_col, line_ranges) = get_line_info(&buffer, cursor_pos);
                            if cur_line > 0 {
                                let (prev_start, prev_len) = line_ranges[cur_line - 1];
                                let new_col = cur_col.min(prev_len);
                                cursor_pos = prev_start + new_col;
                            } else if !self.history.is_empty() {
                                if self.history_idx.is_none() {
                                    saved_current = buffer.iter().collect();
                                    self.history_idx = Some(self.history.len() - 1);
                                } else if let Some(idx) = self.history_idx {
                                    if idx > 0 {
                                        self.history_idx = Some(idx - 1);
                                    }
                                }

                                if let Some(idx) = self.history_idx {
                                    if let Some(entry) = self.history.get(idx) {
                                        buffer = entry.chars().collect();
                                        cursor_pos = buffer.len();
                                    }
                                }
                            }
                        }

                        // Down arrow: multiline line-down or history next
                        (KeyCode::Down, _) => {
                            let (cur_line, cur_col, line_ranges) = get_line_info(&buffer, cursor_pos);
                            if cur_line + 1 < line_ranges.len() {
                                let (next_start, next_len) = line_ranges[cur_line + 1];
                                let new_col = cur_col.min(next_len);
                                cursor_pos = next_start + new_col;
                            } else if let Some(idx) = self.history_idx {
                                if idx + 1 < self.history.len() {
                                    self.history_idx = Some(idx + 1);
                                    if let Some(entry) = self.history.get(idx + 1) {
                                        buffer = entry.chars().collect();
                                        cursor_pos = buffer.len();
                                    }
                                } else {
                                    self.history_idx = None;
                                    buffer = saved_current.chars().collect();
                                    cursor_pos = buffer.len();
                                }
                            }
                        }

                        // Tab key: insert 2 spaces
                        (KeyCode::Tab, _) => {
                            buffer.insert(cursor_pos, ' ');
                            buffer.insert(cursor_pos + 1, ' ');
                            cursor_pos += 2;
                        }

                        // Ctrl+L: Clear screen & re-render
                        (KeyCode::Char('l'), KeyModifiers::CONTROL) => {
                            let _ = execute!(stdout(), terminal::Clear(ClearType::All), cursor::MoveTo(0, 0));
                            last_rendered_lines = 0;
                            last_cursor_row = 0;
                        }

                        // Printable character
                        (KeyCode::Char(c), KeyModifiers::NONE | KeyModifiers::SHIFT) => {
                            buffer.insert(cursor_pos, c);
                            cursor_pos += 1;
                        }

                        _ => {}
                    }
                }

                // Handle bracketed paste / multi-character paste stream
                Event::Paste(text) => {
                    for c in text.chars() {
                        if c == '\r' {
                            continue;
                        }
                        buffer.insert(cursor_pos, c);
                        cursor_pos += 1;
                    }
                }

                // Window resize event
                Event::Resize(_, _) => {}

                _ => {}
            }

            // Re-render after input event
            self.render(
                &buffer,
                cursor_pos,
                &mut last_rendered_lines,
                &mut last_cursor_row,
            )?;
        }
    }

    /// Render the prompt buffer and place cursor at correct row/col.
    fn render(
        &self,
        buffer: &[char],
        cursor_pos: usize,
        last_rendered_lines: &mut usize,
        last_cursor_row: &mut usize,
    ) -> std::io::Result<()> {
        let mut out = stdout();

        // Move cursor back to the top of the prompt rendering area
        if *last_cursor_row > 0 {
            execute!(out, cursor::MoveUp(*last_cursor_row as u16))?;
        }
        execute!(out, cursor::MoveToColumn(0), terminal::Clear(ClearType::FromCursorDown))?;

        let text: String = buffer.iter().collect();
        let lines: Vec<&str> = text.split('\n').collect();

        // Compute cursor row & column
        let (target_row, target_col, _) = get_line_info(buffer, cursor_pos);

        // Draw lines
        for (idx, line) in lines.iter().enumerate() {
            if idx > 0 {
                write!(out, "\r\n")?;
            }
            if idx == 0 {
                write!(out, "{}", self.prompt_symbol)?;
                if line.is_empty() && lines.len() == 1 {
                    if let Some(placeholder) = &self.placeholder {
                        write!(out, "\x1b[2;37m{}\x1b[0m", placeholder)?;
                    }
                } else {
                    write!(out, "{}", line)?;
                }
            } else {
                write!(out, "{}{}", self.multiline_symbol, line)?;
            }
        }

        let total_lines = lines.len();
        *last_rendered_lines = total_lines;
        *last_cursor_row = target_row;

        // Reposition cursor to target row/col
        let rows_up = total_lines.saturating_sub(1 + target_row);
        if rows_up > 0 {
            execute!(out, cursor::MoveUp(rows_up as u16))?;
        }

        let prefix_len = if target_row == 0 {
            visible_width(&self.prompt_symbol)
        } else {
            visible_width(&self.multiline_symbol)
        };
        let target_x = (prefix_len + target_col) as u16;
        execute!(out, cursor::MoveToColumn(target_x))?;

        out.flush()?;
        Ok(())
    }
}

/// Calculate visible character width by ignoring ANSI escape codes.
fn visible_width(s: &str) -> usize {
    let mut in_escape = false;
    let mut width = 0;
    for c in s.chars() {
        if c == '\x1b' {
            in_escape = true;
        } else if in_escape {
            if c == 'm' || (c.is_ascii_alphabetic() && c != '[') {
                in_escape = false;
            }
        } else {
            width += 1;
        }
    }
    width
}

/// Extract line ranges and cursor coordinates from buffer.
fn get_line_info(buffer: &[char], cursor_pos: usize) -> (usize, usize, Vec<(usize, usize)>) {
    let mut ranges = Vec::new();
    let mut line_start = 0;

    for (idx, &c) in buffer.iter().enumerate() {
        if c == '\n' {
            ranges.push((line_start, idx - line_start));
            line_start = idx + 1;
        }
    }
    ranges.push((line_start, buffer.len() - line_start));

    let mut cur_line = 0;
    let mut cur_col = 0;

    for (line_idx, &(start, len)) in ranges.iter().enumerate() {
        if cursor_pos >= start && cursor_pos <= start + len {
            cur_line = line_idx;
            cur_col = cursor_pos - start;
            break;
        }
    }

    (cur_line, cur_col, ranges)
}

/// Calculate previous word start position before cursor.
fn prev_word_pos(buffer: &[char], cursor_pos: usize) -> usize {
    if cursor_pos == 0 {
        return 0;
    }
    let mut pos = cursor_pos;
    while pos > 0 && buffer[pos - 1].is_whitespace() {
        pos -= 1;
    }
    while pos > 0 && !buffer[pos - 1].is_whitespace() {
        pos -= 1;
    }
    pos
}

/// Calculate next word start/end position after cursor.
fn next_word_pos(buffer: &[char], cursor_pos: usize) -> usize {
    let len = buffer.len();
    if cursor_pos >= len {
        return len;
    }
    let mut pos = cursor_pos;
    while pos < len && !buffer[pos].is_whitespace() {
        pos += 1;
    }
    while pos < len && buffer[pos].is_whitespace() {
        pos += 1;
    }
    pos
}

/// RAII Guard that enables raw mode on creation and disables on drop.
pub struct RawModeGuard;

impl RawModeGuard {
    pub fn enter() -> std::io::Result<Self> {
        terminal::enable_raw_mode()?;
        let _ = execute!(stdout(), cursor::Show);
        Ok(Self)
    }
}

impl Drop for RawModeGuard {
    fn drop(&mut self) {
        let _ = terminal::disable_raw_mode();
        let _ = execute!(stdout(), cursor::Show);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_visible_width() {
        assert_eq!(visible_width("hello"), 5);
        assert_eq!(visible_width("\x1b[1;36m❯\x1b[0m "), 2);
        assert_eq!(visible_width("\x1b[2;37m···\x1b[0m "), 4);
        assert_eq!(visible_width("\x1b[31;1mRed Bold\x1b[0m"), 8);
    }

    #[test]
    fn test_get_line_info() {
        let buf: Vec<char> = "hello\nworld".chars().collect();
        let (line, col, ranges) = get_line_info(&buf, 0);
        assert_eq!(line, 0);
        assert_eq!(col, 0);
        assert_eq!(ranges, vec![(0, 5), (6, 5)]);

        let (line, col, _) = get_line_info(&buf, 5);
        assert_eq!(line, 0);
        assert_eq!(col, 5);

        let (line, col, _) = get_line_info(&buf, 6);
        assert_eq!(line, 1);
        assert_eq!(col, 0);

        let (line, col, _) = get_line_info(&buf, 11);
        assert_eq!(line, 1);
        assert_eq!(col, 5);
    }

    #[test]
    fn test_word_pos() {
        let buf: Vec<char> = "hello world foo".chars().collect();
        assert_eq!(prev_word_pos(&buf, 15), 12);
        assert_eq!(prev_word_pos(&buf, 12), 6);
        assert_eq!(prev_word_pos(&buf, 6), 0);

        assert_eq!(next_word_pos(&buf, 0), 6);
        assert_eq!(next_word_pos(&buf, 6), 12);
        assert_eq!(next_word_pos(&buf, 12), 15);
    }

    #[test]
    fn test_history_dedup() {
        let mut p = Prompt::new();
        p.add_history("first");
        p.add_history("first");
        p.add_history("second");
        assert_eq!(p.history, vec!["first", "second"]);
    }
}
