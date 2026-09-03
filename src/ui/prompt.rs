use crossterm::{
    cursor,
    event::{self, Event, KeyEventKind},
    execute,
    terminal::{self, ClearType},
};
use std::io::{stdout, Write};
use unicode_width::UnicodeWidthStr;

use crate::ui::keys::{KeyHandler, KeyResult, KeybindingProfile, PromptState, ViMode};

/// Maximum number of history entries retained by the prompt.
const HISTORY_CAP: usize = 512;

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
    key_handler: KeyHandler,
    show_mode_indicator: bool,
    /// Highlighted index inside the slash autocomplete dialog.
    slash_selection: usize,
    /// Highlighted index inside the model picker dialog.
    model_selection: usize,
    /// Whether the model picker dialog is active (opened from `/model`).
    model_picker_active: bool,
    /// Whether Ctrl+C was pressed on an empty buffer (double-Ctrl+C exits).
    cancel_pressed: bool,
    /// Available models as `(id, display_name)` for the `/model` picker dialog.
    models: Vec<(String, String)>,
    /// Active model name shown in the prompt title bar.
    active_model: String,
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
            prompt_symbol: "\x1b[38;5;75m┃\x1b[0m ".to_string(),
            multiline_symbol: "\x1b[38;5;75m┃\x1b[0m ".to_string(),
            placeholder: None,
            key_handler: KeyHandler::new(KeybindingProfile::Default),
            show_mode_indicator: false,
            slash_selection: 0,
            model_selection: 0,
            model_picker_active: false,
            cancel_pressed: false,
            models: Vec::new(),
            active_model: String::new(),
        }
    }

    /// Set initial history entries.
    pub fn with_history(mut self, history: Vec<String>) -> Self {
        self.history = history;
        self
    }

    /// Provide available models `(id, display_name)` for the `/model` picker dialog.
    pub fn with_models(mut self, models: Vec<(String, String)>) -> Self {
        self.models = models;
        self
    }

    /// Set active model displayed in the prompt box title.
    pub fn with_model(mut self, model: impl Into<String>) -> Self {
        self.active_model = model.into();
        self
    }

    /// Update the active model displayed in the prompt box title.
    pub fn set_model(&mut self, model: impl Into<String>) {
        self.active_model = model.into();
    }
    /// Get active model displayed in the prompt.
    pub fn active_model(&self) -> &str {
        &self.active_model
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

    /// Get current prompt symbol.
    pub fn prompt_symbol(&self) -> &str {
        &self.prompt_symbol
    }

    /// Get current multiline symbol.
    pub fn multiline_symbol(&self) -> &str {
        &self.multiline_symbol
    }
    /// Set placeholder text shown when buffer is empty.
    pub fn with_placeholder(mut self, placeholder: impl Into<String>) -> Self {
        self.placeholder = Some(placeholder.into());
        self
    }
    /// Set keybinding profile (Default, Emacs, Vi).
    pub fn with_keybinding_profile(mut self, profile: KeybindingProfile) -> Self {
        self.key_handler.set_profile(profile);
        self
    }

    /// Return currently active keybinding profile.
    pub fn keybinding_profile(&self) -> KeybindingProfile {
        self.key_handler.profile()
    }

    /// Switch active keybinding profile.
    pub fn set_keybinding_profile(&mut self, profile: KeybindingProfile) {
        self.key_handler.set_profile(profile);
    }

    /// Attach a custom keymap configuration.
    pub fn with_keymap(mut self, config: crate::ui::keymap_config::KeymapConfig) -> Self {
        self.key_handler.set_keymap(config);
        self
    }

    /// Set a custom keymap configuration.
    pub fn set_keymap(&mut self, config: crate::ui::keymap_config::KeymapConfig) {
        self.key_handler.set_keymap(config);
    }

    /// Toggle showing modal indicators (e.g. `[INS]` / `[NOR]` in Vi mode).
    pub fn with_mode_indicator(mut self, show: bool) -> Self {
        self.show_mode_indicator = show;
        self
    }

    /// Access underlying key handler.
    pub fn key_handler(&self) -> &KeyHandler {
        &self.key_handler
    }

    /// Mutable access to underlying key handler.
    pub fn key_handler_mut(&mut self) -> &mut KeyHandler {
        &mut self.key_handler
    }


    /// Returns a slice of recorded history entries.
    pub fn history(&self) -> &[String] {
        &self.history
    }

    /// Append a completed line to history.
    /// Caps retained entries at [`HISTORY_CAP`], dropping the oldest first.
    pub fn add_history(&mut self, entry: impl Into<String>) {
        let entry = entry.into();
        if !entry.trim().is_empty() {
            // Avoid duplicate consecutive entries
            if self.history.last().map(|s| s.as_str()) != Some(&entry) {
                self.history.push(entry);
                if self.history.len() > HISTORY_CAP {
                    self.history.remove(0);
                }
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

        // Reset Vi mode to Insert at the beginning of each user turn
        if self.key_handler.profile() == KeybindingProfile::Vi {
            self.key_handler.set_vi_mode(ViMode::Insert);
        }
        self.key_handler.clear_pending();

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

                    // Model picker dialog mode (FX-style): opened by selecting
                    // `/model` from the command dialog. Buffer holds the filter
                    // query (no `/model` prefix). Enter picks, Esc closes back
                    // to normal input.
                    if self.model_picker_active {
                        use crossterm::event::KeyCode;
                        let query: String = buffer.iter().collect::<String>().to_lowercase();
                        let filtered: Vec<&(String, String)> = if query.is_empty() {
                            self.models.iter().collect()
                        } else {
                            self.models
                                .iter()
                                .filter(|(id, name)| {
                                    id.to_lowercase().contains(&query)
                                        || name.to_lowercase().contains(&query)
                                })
                                .collect()
                        };
                        match key.code {
                            KeyCode::Tab | KeyCode::Down => {
                                if !filtered.is_empty() {
                                    self.model_selection = (self.model_selection + 1) % filtered.len();
                                }
                                self.render(
                                    &buffer,
                                    cursor_pos,
                                    &mut last_rendered_lines,
                                    &mut last_cursor_row,
                                )?;
                                continue;
                            }
                            KeyCode::BackTab | KeyCode::Up => {
                                if !filtered.is_empty() {
                                    self.model_selection = if self.model_selection == 0 {
                                        filtered.len() - 1
                                    } else {
                                        self.model_selection - 1
                                    };
                                }
                                self.render(
                                    &buffer,
                                    cursor_pos,
                                    &mut last_rendered_lines,
                                    &mut last_cursor_row,
                                )?;
                                continue;
                            }
                            KeyCode::Enter => {
                                if let Some(sel) =
                                    filtered.get(self.model_selection.min(filtered.len().saturating_sub(1)))
                                {
                                    let text = format!("/model {}", sel.0);
                                    let mut out = stdout();
                                    if last_cursor_row > 0 {
                                        let _ = execute!(out, cursor::MoveUp(last_cursor_row as u16));
                                    }
                                    let _ = execute!(out, cursor::MoveToColumn(0), terminal::Clear(ClearType::FromCursorDown));
                                    let _ = write!(out, "\x1b[1m┃ {}\x1b[0m\r\n\r\n", text);
                                    let _ = out.flush();

                                    self.model_picker_active = false;
                                    self.model_selection = 0;
                                    self.add_history(text.clone());
                                    return Ok(PromptResult::Submit(text));
                                }
                            }
                            KeyCode::Esc => {
                                // Close the picker, return to a blank prompt.
                                buffer.clear();
                                cursor_pos = 0;
                                self.model_picker_active = false;
                                self.model_selection = 0;
                                self.render(
                                    &buffer,
                                    cursor_pos,
                                    &mut last_rendered_lines,
                                    &mut last_cursor_row,
                                )?;
                                continue;
                            }
                            KeyCode::Backspace => {
                                if cursor_pos > 0 {
                                    buffer.remove(cursor_pos - 1);
                                    cursor_pos -= 1;
                                }
                                self.model_selection = 0;
                                self.render(
                                    &buffer,
                                    cursor_pos,
                                    &mut last_rendered_lines,
                                    &mut last_cursor_row,
                                )?;
                                continue;
                            }
                            KeyCode::Char(c) => {
                                buffer.insert(cursor_pos, c);
                                cursor_pos += 1;
                                self.model_selection = 0;
                                self.render(
                                    &buffer,
                                    cursor_pos,
                                    &mut last_rendered_lines,
                                    &mut last_cursor_row,
                                )?;
                                continue;
                            }
                            _ => {}
                        }
                    }

                    // Slash autocomplete dialog navigation (FX-style):
                    // active whenever the first line starts with '/'.
                    let text_so_far: String = buffer.iter().collect();
                    let first_line = text_so_far.split('\n').next().unwrap_or("");
                    if first_line.starts_with('/') {
                        let matches = slash_matches(first_line);
                        if !matches.is_empty() {
                            use crossterm::event::KeyCode;
                            match key.code {
                                KeyCode::Tab | KeyCode::Down => {
                                    self.slash_selection =
                                        (self.slash_selection + 1) % matches.len();
                                    self.render(
                                        &buffer,
                                        cursor_pos,
                                        &mut last_rendered_lines,
                                        &mut last_cursor_row,
                                    )?;
                                    continue;
                                }
                                KeyCode::BackTab | KeyCode::Up => {
                                    self.slash_selection = if self.slash_selection == 0 {
                                        matches.len() - 1
                                    } else {
                                        self.slash_selection - 1
                                    };
                                    self.render(
                                        &buffer,
                                        cursor_pos,
                                        &mut last_rendered_lines,
                                        &mut last_cursor_row,
                                    )?;
                                    continue;
                                }
                                KeyCode::Enter => {
                                    // Accept the highlighted command: replace the
                                    // first line with the selected command name.
                                    if let Some(sel) =
                                        matches.get(self.slash_selection.min(matches.len() - 1))
                                    {
                                        // Selecting `/model` opens the model picker
                                        // dialog directly (no prefix text).
                                        if sel.name == "/model" || sel.aliases.contains(&"/model") {
                                            buffer.clear();
                                            cursor_pos = 0;
                                            self.model_picker_active = true;
                                            self.model_selection = 0;
                                            self.render(
                                                &buffer,
                                                cursor_pos,
                                                &mut last_rendered_lines,
                                                &mut last_cursor_row,
                                            )?;
                                            continue;
                                        }
                                        let cmd = sel.name.to_string();
                                        let rest: String = text_so_far
                                            .split_once('\n')
                                            .map(|(_, r)| r.to_string())
                                            .unwrap_or_default();
                                        let new_text = format!("{} {}", cmd, rest);
                                        buffer.clear();
                                        buffer.extend(new_text.chars());
                                        cursor_pos = cmd.len() + 1;
                                        self.slash_selection = 0;
                                        self.render(
                                            &buffer,
                                            cursor_pos,
                                            &mut last_rendered_lines,
                                            &mut last_cursor_row,
                                        )?;
                                        continue;
                                    }
                                }
                                _ => {}
                            }
                        }
                    }

                    let mut state = PromptState::new(
                        &mut buffer,
                        &mut cursor_pos,
                        &self.history,
                        &mut self.history_idx,
                        &mut saved_current,
                    );

                    match self.key_handler.handle_key(key, &mut state) {
                        KeyResult::Continue => {}
                        KeyResult::Noop => continue,
                        KeyResult::Submit(text) => {
                            let mut out = stdout();
                            if last_cursor_row > 0 {
                                let _ = execute!(out, cursor::MoveUp(last_cursor_row as u16));
                            }
                            let _ = execute!(out, cursor::MoveToColumn(0), terminal::Clear(ClearType::FromCursorDown));

                            let lines: Vec<&str> = text.split('\n').collect();
                            for line in &lines {
                                let _ = write!(out, "\x1b[1m┃ {}\x1b[0m\r\n", line);
                            }
                            let _ = write!(out, "\r\n");
                            let _ = out.flush();
                            if !text.trim().is_empty() {
                                self.add_history(text.clone());
                            }
                            return Ok(PromptResult::Submit(text));
                        }
                        KeyResult::Cancel => {
                            let has_text: bool = buffer.iter().any(|c| !c.is_whitespace());
                            let mut out = stdout();
                            if last_cursor_row > 0 {
                                let _ = execute!(out, cursor::MoveUp(last_cursor_row as u16));
                            }
                            let _ = execute!(out, cursor::MoveToColumn(0), terminal::Clear(ClearType::FromCursorDown));
                            let _ = out.flush();
                            if !has_text && self.cancel_pressed {
                                return Ok(PromptResult::Exit);
                            }
                            self.cancel_pressed = !has_text;
                            if has_text {
                                self.cancel_pressed = false;
                            }
                            return Ok(PromptResult::Cancel);
                        }
                        KeyResult::Exit => {
                            let mut out = stdout();
                            if last_cursor_row > 0 {
                                let _ = execute!(out, cursor::MoveUp(last_cursor_row as u16));
                            }
                            let _ = execute!(out, cursor::MoveToColumn(0), terminal::Clear(ClearType::FromCursorDown));
                            let _ = out.flush();
                            return Ok(PromptResult::Exit);
                        }
                        KeyResult::ClearScreen => {
                            let _ = execute!(stdout(), terminal::Clear(ClearType::All), cursor::MoveTo(0, 0));
                            last_rendered_lines = 0;
                            last_cursor_row = 0;
                        }
                    }
                }

                // Handle bracketed paste / multi-character paste stream.
                // CRLF and lone CR are normalized to LF so pasted Windows text
                // renders and submits consistently.
                Event::Paste(text) => {
                    self.key_handler.snapshot_undo(&buffer, cursor_pos);
                    let normalized = if text.contains('\r') {
                        text.replace("\r\n", "\n").replace('\r', "\n")
                    } else {
                        text
                    };
                    for c in normalized.chars() {
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

    /// Render the prompt buffer with a border line above/below the input and
    /// an FX-style slash autocomplete dialog below it when typing a command.
    /// Render the prompt buffer as a rounded input box docked at the bottom of the terminal screen.
    fn render(
        &self,
        buffer: &[char],
        cursor_pos: usize,
        last_rendered_lines: &mut usize,
        last_cursor_row: &mut usize,
    ) -> std::io::Result<()> {
        self.render_to(
            &mut stdout(),
            buffer,
            cursor_pos,
            last_rendered_lines,
            last_cursor_row,
        )
    }

    /// Render the prompt buffer into a generic writer.
    pub fn render_to<W: std::io::Write>(
        &self,
        out: &mut W,
        buffer: &[char],
        cursor_pos: usize,
        last_rendered_lines: &mut usize,
        last_cursor_row: &mut usize,
    ) -> std::io::Result<()> {
        let text: String = buffer.iter().collect();
        let lines: Vec<&str> = text.split('\n').collect();

        // Compute cursor row & column
        let (target_row, target_col, _) = get_line_info(buffer, cursor_pos);

        // Filter models if model picker is active
        let mut filtered_models = Vec::new();
        if self.model_picker_active && !self.models.is_empty() {
            let query = text.to_lowercase();
            filtered_models = if query.is_empty() {
                self.models.iter().collect()
            } else {
                self.models
                    .iter()
                    .filter(|(id, name)| {
                        id.to_lowercase().contains(&query)
                            || name.to_lowercase().contains(&query)
                    })
                    .collect()
            };
        }

        // Check for slash suggestions
        let first_line = lines.first().copied().unwrap_or("");
        let slash_suggestions = if !self.model_picker_active && first_line.starts_with('/') {
            slash_matches(first_line)
        } else {
            Vec::new()
        };

        // Clear previous frame using exact relative cursor movement
        if *last_cursor_row > 0 {
            execute!(out, cursor::MoveUp(*last_cursor_row as u16))?;
        }
        execute!(out, cursor::MoveToColumn(0), terminal::Clear(ClearType::FromCursorDown))?;

        let mut total_lines = 0;

        // 1. Clean suggestions list (if active)
        if self.model_picker_active && !filtered_models.is_empty() {
            let sel = self.model_selection.min(filtered_models.len().saturating_sub(1));
            let window_start = if sel >= 6 { sel - 5 } else { 0 };
            write!(out, "  \x1b[2;37mModels\x1b[0m\r\n")?;
            total_lines += 1;
            for (idx, (id, name)) in filtered_models.iter().enumerate().skip(window_start).take(6) {
                if idx == sel {
                    write!(
                        out,
                        "  \x1b[1;36m▶\x1b[0m \x1b[1;37m{:<32}\x1b[0m \x1b[2;37m{}\x1b[0m\r\n",
                        id, name
                    )?;
                } else {
                    write!(
                        out,
                        "    \x1b[2;37m{:<32} {}\x1b[0m\r\n",
                        id, name
                    )?;
                }
                total_lines += 1;
            }
            write!(out, "\r\n")?;
            total_lines += 1;
        } else if !slash_suggestions.is_empty() {
            let sel = self.slash_selection.min(slash_suggestions.len().saturating_sub(1));
            let window_start = if sel >= 6 { sel - 5 } else { 0 };
            for (idx, item) in slash_suggestions.iter().enumerate().skip(window_start).take(6) {
                if idx == sel {
                    write!(
                        out,
                        "  \x1b[1;36m▶\x1b[0m \x1b[1;37m{:<16}\x1b[0m \x1b[2;37m{}\x1b[0m\r\n",
                        item.name, item.description
                    )?;
                } else {
                    write!(
                        out,
                        "    \x1b[2;37m{:<16} {}\x1b[0m\r\n",
                        item.name, item.description
                    )?;
                }
                total_lines += 1;
            }
            write!(out, "\r\n")?;
            total_lines += 1;
        }

        let dialog_lines = total_lines;

        // 2. Input lines with clean vertical rail symbol (┃ )
        for (idx, line) in lines.iter().enumerate() {
            let prefix = if idx == 0 {
                &self.prompt_symbol
            } else {
                &self.multiline_symbol
            };

            write!(out, "{}", prefix)?;
            if idx == 0 && line.is_empty() && lines.len() == 1 {
                if let Some(ph) = &self.placeholder {
                    write!(out, "\x1b[2;37m{}\x1b[0m", ph)?;
                }
            } else {
                write!(out, "{}", line)?;
            }
            write!(out, "\r\n")?;
            total_lines += 1;
        }

        // 3. Blank line between input and status
        write!(out, "\r\n")?;
        total_lines += 1;

        // 4. Status line at the bottom: mode and model name
        let model_label = match self.active_model.as_str() {
            "deepseek-ai/DeepSeek-V4-Flash-0731" | "flash" | "v4" => "DeepSeek V4 Flash",
            "MiniMaxAI/MiniMax-M2.7" | "minimax" => "MiniMax M2.7",
            "moonshotai/Kimi-K2.6" | "kimi" => "Kimi K2.6",
            other => {
                if let Some((_, name)) = other.split_once('/') {
                    name
                } else if other.is_empty() {
                    "auto"
                } else {
                    other
                }
            }
        };

        write!(out, "\x1b[2;37mauto · {}\x1b[0m", model_label)?;
        total_lines += 1;

        // Reposition cursor inside input box on active input row
        let lines_up = (lines.len() - 1 - target_row) + 2;
        execute!(out, cursor::MoveUp(lines_up as u16))?;

        let prefix_col = 2; // "┃ " occupies 2 terminal cells
        let target_x = (prefix_col + target_col) as u16;
        execute!(out, cursor::MoveToColumn(target_x))?;

        *last_rendered_lines = total_lines;
        *last_cursor_row = dialog_lines + target_row;

        out.flush()?;
        Ok(())
    }

    /// Render a submitted user input prompt to a generic writer.
    pub fn render_submitted_prompt_to<W: std::io::Write>(out: &mut W, text: &str) -> std::io::Result<()> {
        let lines: Vec<&str> = text.split('\n').collect();
        for line in &lines {
            write!(out, "\x1b[1m┃ {}\x1b[0m\r\n", line)?;
        }
        write!(out, "\r\n")?;
        out.flush()?;
        Ok(())
    }

    /// Render a submitted user input prompt to stdout.
    pub fn render_submitted_prompt(text: &str) {
        let _ = Self::render_submitted_prompt_to(&mut stdout(), text);
    }
}

/// Match slash commands whose name or aliases start with the typed prefix.
fn slash_matches(typed: &str) -> Vec<&'static crate::ui::slash::CommandDescriptor> {
    let query = typed.trim_start().to_lowercase();
    crate::ui::slash::COMMAND_PALETTE
        .iter()
        .filter(|d| {
            d.name.to_lowercase().starts_with(&query)
                || d.aliases.iter().any(|a| a.to_lowercase().starts_with(&query))
        })
        .take(8)
        .collect()
}

/// Calculate visible character width by ignoring ANSI escape codes.
/// Wide (CJK) characters count 2 columns via `unicode-width`.
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
            let mut buf = [0u8; 4];
            width += UnicodeWidthStr::width(c.encode_utf8(&mut buf));
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

    #[test]
    fn test_prompt_keybinding_profiles() {
        let p_default = Prompt::new();
        assert_eq!(p_default.keybinding_profile(), KeybindingProfile::Default);

        let p_emacs = Prompt::new().with_keybinding_profile(KeybindingProfile::Emacs);
        assert_eq!(p_emacs.keybinding_profile(), KeybindingProfile::Emacs);

        let mut p_vi = Prompt::new().with_keybinding_profile(KeybindingProfile::Vi);
        assert_eq!(p_vi.keybinding_profile(), KeybindingProfile::Vi);
        assert_eq!(p_vi.key_handler().vi_mode(), ViMode::Insert);

        p_vi.set_keybinding_profile(KeybindingProfile::Default);
        assert_eq!(p_vi.keybinding_profile(), KeybindingProfile::Default);
    }

    #[test]
    fn test_prompt_custom_keymap() {
        let mut keymap = crate::ui::keymap_config::KeymapConfig::default();
        keymap
            .bind("ctrl+s", crate::ui::keymap_config::KeyAction::Submit)
            .unwrap();
        let prompt = Prompt::new().with_keymap(keymap);
        assert!(prompt.key_handler().keymap().is_some());
    }

    #[test]
    fn test_slash_matches_filters_by_prefix() {
        // "/hel" should match "/help"
        let m = slash_matches("/hel");
        assert!(!m.is_empty());
        assert!(m.iter().any(|d| d.name == "/help"));

        // "/mo" should match "/model"
        let m = slash_matches("/mo");
        assert!(m.iter().any(|d| d.name == "/model"));

        // Unknown prefix -> no matches
        assert!(slash_matches("/zzz-nope").is_empty());
    }

    #[test]
    fn test_slash_matches_respects_alias() {
        // "/pal" is an alias of "/palette"
        let m = slash_matches("/pal");
        assert!(m.iter().any(|d| d.name == "/palette"));
    }

    #[test]
    fn test_slash_matches_case_insensitive() {
        assert!(slash_matches("/HEL").iter().any(|d| d.name == "/help"));
    }

    #[test]
    fn test_truncate_str() {
        assert_eq!(truncate_str("short", 10), "short");
        let long = "a".repeat(50);
        let t = truncate_str(&long, 10);
        assert_eq!(t.chars().count(), 10);
        assert!(t.ends_with('…'));
    }

    /// Test-local UTF-8 truncator mirroring the production helper.
    fn truncate_str(s: &str, max_chars: usize) -> String {
        let count = s.chars().count();
        if count <= max_chars {
            s.to_string()
        } else {
            let keep: String = s.chars().take(max_chars.saturating_sub(1)).collect();
            format!("{}…", keep)
        }
    }
}
