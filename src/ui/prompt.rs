use crossterm::{
    cursor,
    event::{self, Event, KeyCode, KeyEventKind, KeyModifiers},
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
    active_model: String,
    pub running_status: Option<String>,
    pub buffer: Vec<char>,
    pub cursor_pos: usize,
    saved_current: String,
    pub last_rendered_lines: usize,
    pub last_cursor_row: usize,
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
            prompt_symbol: "\x1b[1m┃\x1b[0m ".to_string(),
            multiline_symbol: "\x1b[1m┃\x1b[0m ".to_string(),
            placeholder: None,
            key_handler: KeyHandler::new(KeybindingProfile::Default),
            show_mode_indicator: false,
            slash_selection: 0,
            model_selection: 0,
            model_picker_active: false,
            cancel_pressed: false,
            models: Vec::new(),
            active_model: String::new(),
            running_status: None,
            buffer: Vec::new(),
            cursor_pos: 0,
            saved_current: String::new(),
            last_rendered_lines: 0,
            last_cursor_row: 0,
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

    /// Toggle whether the model picker dialog is active.
    pub fn with_model_picker_active(mut self, active: bool) -> Self {
        self.model_picker_active = active;
        self
    }

    /// Set whether the model picker dialog is active.
    pub fn set_model_picker_active(&mut self, active: bool) {
        self.model_picker_active = active;
    }

    /// Whether the model picker dialog is active.
    pub fn model_picker_active(&self) -> bool {
        self.model_picker_active
    }

    /// Set the selected index for the model picker dialog.
    pub fn with_model_selection(mut self, sel: usize) -> Self {
        self.model_selection = sel;
        self
    }

    /// Set the selected index for the model picker dialog.
    pub fn set_model_selection(&mut self, sel: usize) {
        self.model_selection = sel;
    }

    /// Get the selected index for the model picker dialog.
    pub fn model_selection(&self) -> usize {
        self.model_selection
    }

    /// Set the selected index for the slash command dialog.
    pub fn with_slash_selection(mut self, sel: usize) -> Self {
        self.slash_selection = sel;
        self
    }

    /// Set the selected index for the slash command dialog.
    pub fn set_slash_selection(&mut self, sel: usize) {
        self.slash_selection = sel;
    }

    /// Get the selected index for the slash command dialog.
    pub fn slash_selection(&self) -> usize {
        self.slash_selection
    }

    /// Available models for the model picker dialog.
    pub fn models(&self) -> &[(String, String)] {
        &self.models
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

    /// Reset internal state for a fresh input line.
    pub fn reset_input(&mut self) {
        self.buffer.clear();
        self.cursor_pos = 0;
        self.saved_current.clear();
        self.history_idx = None;
        self.slash_selection = 0;
        self.model_selection = 0;
        self.model_picker_active = false;
        self.last_rendered_lines = 0;
        self.last_cursor_row = 0;
        self.running_status = None;
        if self.key_handler.profile() == KeybindingProfile::Vi {
            self.key_handler.set_vi_mode(ViMode::Insert);
        }
        self.key_handler.clear_pending();
    }

    /// Update active running/thinking status banner displayed above the prompt.
    pub fn set_running_status(&mut self, status: Option<String>) {
        self.running_status = status;
    }

    /// Render current prompt state to stdout.
    pub fn render_current(&mut self) -> std::io::Result<()> {
        let mut out = stdout();
        let buffer = self.buffer.clone();
        let cursor_pos = self.cursor_pos;
        let mut last_lines = self.last_rendered_lines;
        let mut last_row = self.last_cursor_row;
        self.render_to(&mut out, &buffer, cursor_pos, &mut last_lines, &mut last_row)?;
        self.last_rendered_lines = last_lines;
        self.last_cursor_row = last_row;
        Ok(())
    }

    /// Erase the rendered prompt frame from screen.
    pub fn clear_frame(&mut self) -> std::io::Result<()> {
        let mut out = stdout();
        if self.last_cursor_row > 0 {
            execute!(out, cursor::MoveUp(self.last_cursor_row as u16))?;
        }
        execute!(out, cursor::MoveToColumn(0), terminal::Clear(ClearType::FromCursorDown))?;
        out.flush()?;
        self.last_rendered_lines = 0;
        self.last_cursor_row = 0;
        Ok(())
    }

    /// Handle a single crossterm event, updating input state and re-rendering.
    pub fn handle_event(&mut self, event: Event) -> std::io::Result<Option<PromptResult>> {
        match event {
            Event::Key(key) => {
                if key.kind == KeyEventKind::Release {
                    return Ok(None);
                }

                // Esc or Ctrl+C handling
                if key.code == KeyCode::Esc {
                    if self.model_picker_active {
                        self.model_picker_active = false;
                        self.buffer.clear();
                        self.cursor_pos = 0;
                        self.render_current()?;
                        return Ok(None);
                    }
                    let text_so_far: String = self.buffer.iter().collect();
                    if text_so_far.starts_with('/') {
                        self.buffer.clear();
                        self.cursor_pos = 0;
                        self.render_current()?;
                        return Ok(None);
                    }
                    self.clear_frame()?;
                    return Ok(Some(PromptResult::Cancel));
                }

                if key.modifiers.contains(KeyModifiers::CONTROL)
                    && (key.code == KeyCode::Char('c') || key.code == KeyCode::Char('C'))
                {
                    self.clear_frame()?;
                    return Ok(Some(PromptResult::Cancel));
                }

                if key.modifiers.contains(KeyModifiers::CONTROL)
                    && (key.code == KeyCode::Char('d') || key.code == KeyCode::Char('D'))
                {
                    if self.buffer.is_empty() {
                        self.clear_frame()?;
                        return Ok(Some(PromptResult::Exit));
                    }
                }

                // Model picker dialog mode
                if self.model_picker_active {
                    let query: String = self.buffer.iter().collect::<String>().to_lowercase();
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
                                self.model_selection =
                                    (self.model_selection + 1) % filtered.len();
                            }
                            self.render_current()?;
                            return Ok(None);
                        }
                        KeyCode::BackTab | KeyCode::Up => {
                            if !filtered.is_empty() {
                                self.model_selection = if self.model_selection == 0 {
                                    filtered.len() - 1
                                 } else {
                                    self.model_selection - 1
                                };
                            }
                            self.render_current()?;
                            return Ok(None);
                        }
                        KeyCode::Enter => {
                            if let Some(sel) = filtered
                                .get(self.model_selection.min(filtered.len().saturating_sub(1)))
                            {
                                let text = format!("/model {}", sel.0);
                                self.clear_frame()?;
                                let mut out = stdout();
                                let _ = write!(out, "\x1b[1m┃ {}\x1b[0m\r\n\r\n", text);
                                let _ = out.flush();

                                self.model_picker_active = false;
                                self.model_selection = 0;
                                self.buffer.clear();
                                self.cursor_pos = 0;
                                self.add_history(text.clone());
                                return Ok(Some(PromptResult::Submit(text)));
                            }
                            return Ok(None);
                        }
                        KeyCode::Backspace => {
                            if self.cursor_pos > 0 {
                                self.buffer.remove(self.cursor_pos - 1);
                                self.cursor_pos -= 1;
                            }
                            self.model_selection = 0;
                            self.render_current()?;
                            return Ok(None);
                        }
                        KeyCode::Char(c) => {
                            self.buffer.insert(self.cursor_pos, c);
                            self.cursor_pos += 1;
                            self.model_selection = 0;
                            self.render_current()?;
                            return Ok(None);
                        }
                        _ => return Ok(None),
                    }
                }

                // Slash autocomplete dialog navigation
                let text_so_far: String = self.buffer.iter().collect();
                let first_line = text_so_far.split('\n').next().unwrap_or("");
                if first_line.starts_with('/') {
                    let matches = slash_matches(first_line);
                    if !matches.is_empty() {
                        match key.code {
                            KeyCode::Tab | KeyCode::Down => {
                                self.slash_selection =
                                    (self.slash_selection + 1) % matches.len();
                                self.render_current()?;
                                return Ok(None);
                            }
                            KeyCode::BackTab | KeyCode::Up => {
                                self.slash_selection = if self.slash_selection == 0 {
                                    matches.len() - 1
                                } else {
                                    self.slash_selection - 1
                                };
                                self.render_current()?;
                                return Ok(None);
                            }
                            KeyCode::Enter => {
                                if let Some(sel) =
                                    matches.get(self.slash_selection.min(matches.len() - 1))
                                {
                                    if sel.name == "/model" || sel.aliases.contains(&"/model") {
                                        self.buffer.clear();
                                        self.cursor_pos = 0;
                                        self.model_picker_active = true;
                                        self.model_selection = 0;
                                        self.render_current()?;
                                        return Ok(None);
                                    }
                                    let cmd = sel.name.to_string();
                                    let rest: String = text_so_far
                                        .split_once('\n')
                                        .map(|(_, r)| r.to_string())
                                        .unwrap_or_default();
                                    let new_text = format!("{} {}", cmd, rest);
                                    self.buffer.clear();
                                    self.buffer.extend(new_text.chars());
                                    self.cursor_pos = cmd.len() + 1;
                                    self.slash_selection = 0;
                                    self.render_current()?;
                                    return Ok(None);
                                }
                            }
                            _ => {}
                        }
                    }
                }

                // Standard input editing
                let mut state = PromptState::new(
                    &mut self.buffer,
                    &mut self.cursor_pos,
                    &self.history,
                    &mut self.history_idx,
                    &mut self.saved_current,
                );

                match self.key_handler.handle_key(key, &mut state) {
                    KeyResult::Continue => {
                        self.render_current()?;
                        Ok(None)
                    }
                    KeyResult::Submit(text) => {
                        let trimmed = text.trim();
                        if trimmed.is_empty() {
                            return Ok(None);
                        }
                        self.clear_frame()?;
                        let mut out = stdout();
                        let lines: Vec<&str> = text.split('\n').collect();
                        for line in &lines {
                            let _ = write!(out, "\x1b[1m┃ {}\x1b[0m\r\n", line);
                        }
                        let _ = write!(out, "\r\n");
                        let _ = out.flush();
                        self.add_history(text.clone());
                        self.buffer.clear();
                        self.cursor_pos = 0;
                        Ok(Some(PromptResult::Submit(text)))
                    }
                    KeyResult::Cancel => {
                        self.clear_frame()?;
                        Ok(Some(PromptResult::Cancel))
                    }
                    KeyResult::Exit => {
                        self.clear_frame()?;
                        Ok(Some(PromptResult::Exit))
                    }
                    KeyResult::ClearScreen => {
                        let _ = execute!(
                            stdout(),
                            terminal::Clear(ClearType::All),
                            cursor::MoveTo(0, 0)
                        );
                        self.last_rendered_lines = 0;
                        self.last_cursor_row = 0;
                        self.render_current()?;
                        Ok(None)
                    }
                    _ => {
                        self.render_current()?;
                        Ok(None)
                    }
                }
            }
            Event::Paste(text) => {
                self.key_handler.snapshot_undo(&self.buffer, self.cursor_pos);
                let normalized = text.replace("\r\n", "\n").replace('\r', "\n");
                for c in normalized.chars() {
                    self.buffer.insert(self.cursor_pos, c);
                    self.cursor_pos += 1;
                }
                self.render_current()?;
                Ok(None)
            }
            Event::Resize(_, _) => {
                self.render_current()?;
                Ok(None)
            }
            _ => Ok(None),
        }
    }

    /// Read an interactive line / multiline input from user.
    pub fn read_input(&mut self) -> std::io::Result<PromptResult> {
        let _raw_guard = RawModeGuard::enter()?;
        self.reset_input();
        self.render_current()?;

        loop {
            let ev = event::read()?;
            if let Some(res) = self.handle_event(ev)? {
                return Ok(res);
            }
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

        let running_lines = if let Some(status) = &self.running_status {
            write!(out, "  \x1b[2;37m{}\x1b[0m\r\n\r\n", status)?;
            2
        } else {
            0
        };
        total_lines += running_lines;

        // 1. Input lines with clean vertical rail symbol (┃ )
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

        let term_cols = terminal::size().map(|(w, _)| w as usize).unwrap_or(80);
        let divider = "─".repeat(term_cols);

        // 2. Dropdown menu below the input line (matching fx)
        if self.model_picker_active {
            let sel = if filtered_models.is_empty() {
                0
            } else {
                self.model_selection.min(filtered_models.len().saturating_sub(1))
            };
            let window_start = if sel >= 6 { sel - 5 } else { 0 };
            let visible_models: Vec<_> = filtered_models
                .iter()
                .enumerate()
                .skip(window_start)
                .take(6)
                .collect();
            let visible_count = visible_models.len();
            let window_end = if filtered_models.is_empty() {
                0
            } else {
                window_start + visible_count
            };

            write!(out, "\x1b[38;5;240m{}\x1b[0m\r\n", divider)?;
            total_lines += 1;

            let left = format!("Models {} · Type to filter", filtered_models.len());
            let right = if filtered_models.is_empty() {
                "0-0".to_string()
            } else {
                format!("{}-{}", window_start + 1, window_end)
            };
            let gap = term_cols.saturating_sub(left.len() + right.len());
            write!(out, "\x1b[2;37m{}{}{}\x1b[0m\r\n", left, " ".repeat(gap), right)?;
            total_lines += 1;

            write!(out, "\r\n")?;
            total_lines += 1;

            for (idx, (id, name)) in visible_models {
                let is_selected = idx == sel;
                let cat = model_category_label(id, name);
                let col1_w = 34.min(term_cols.saturating_sub(30));
                let col3_w = 12;
                let col2_w = term_cols.saturating_sub(col1_w + col3_w + 4);

                let col1 = truncate_fit(id, col1_w);
                let col2 = truncate_fit(name, col2_w);

                if is_selected {
                    write!(
                        out,
                        "\x1b[1;37m{:<c1$}\x1b[0m \x1b[37m{:<c2$}\x1b[0m \x1b[2;37m{:>c3$}\x1b[0m\r\n",
                        col1, col2, cat, c1 = col1_w, c2 = col2_w, c3 = col3_w
                    )?;
                } else {
                    write!(
                        out,
                        "\x1b[2;37m{:<c1$} {:<c2$} {:>c3$}\x1b[0m\r\n",
                        col1, col2, cat, c1 = col1_w, c2 = col2_w, c3 = col3_w
                    )?;
                }
                total_lines += 1;
            }

            write!(out, "\x1b[38;5;240m{}\x1b[0m\r\n", divider)?;
            total_lines += 1;

            write!(out, "\x1b[2;37m↑↓ Navigate     Enter Use     Esc Close\x1b[0m")?;
            total_lines += 1;

            let lines_up = (lines.len() - 1 - target_row) + visible_count + 5;
            execute!(out, cursor::MoveUp(lines_up as u16))?;
            let prefix = if target_row == 0 {
                &self.prompt_symbol
            } else {
                &self.multiline_symbol
            };
            let prefix_col = visible_width(prefix);
            let target_x = (prefix_col + target_col) as u16;
            execute!(out, cursor::MoveToColumn(target_x))?;

            *last_rendered_lines = total_lines;
            *last_cursor_row = running_lines + target_row;
        } else if !slash_suggestions.is_empty() {
            let sel = self.slash_selection.min(slash_suggestions.len().saturating_sub(1));
            let window_start = if sel >= 6 { sel - 5 } else { 0 };
            let visible_items: Vec<_> = slash_suggestions
                .iter()
                .enumerate()
                .skip(window_start)
                .take(6)
                .collect();
            let visible_count = visible_items.len();
            let window_end = window_start + visible_count;

            write!(out, "\x1b[38;5;240m{}\x1b[0m\r\n", divider)?;
            total_lines += 1;

            let noun = if first_line == "/" { "Commands" } else { "Results" };
            let left = if first_line == "/" {
                format!("{} {} · Type to filter", noun, slash_suggestions.len())
            } else {
                format!("{} {}", noun, slash_suggestions.len())
            };
            let right = format!("{}-{}", window_start + 1, window_end);
            let gap = term_cols.saturating_sub(left.len() + right.len());
            write!(out, "\x1b[2;37m{}{}{}\x1b[0m\r\n", left, " ".repeat(gap), right)?;
            total_lines += 1;

            write!(out, "\r\n")?;
            total_lines += 1;

            for (idx, item) in visible_items {
                let is_selected = idx == sel;
                let cat = slash_category_label(item.category);
                let col1_w = 14;
                let col3_w = 10;
                let col2_w = term_cols.saturating_sub(col1_w + col3_w + 4);

                let col1 = truncate_fit(item.name, col1_w);
                let col2 = truncate_fit(item.description, col2_w);

                if is_selected {
                    write!(
                        out,
                        "\x1b[1;37m{:<c1$}\x1b[0m \x1b[37m{:<c2$}\x1b[0m \x1b[2;37m{:>c3$}\x1b[0m\r\n",
                        col1, col2, cat, c1 = col1_w, c2 = col2_w, c3 = col3_w
                    )?;
                } else {
                    write!(
                        out,
                        "\x1b[2;37m{:<c1$} {:<c2$} {:>c3$}\x1b[0m\r\n",
                        col1, col2, cat, c1 = col1_w, c2 = col2_w, c3 = col3_w
                    )?;
                }
                total_lines += 1;
            }

            write!(out, "\x1b[38;5;240m{}\x1b[0m\r\n", divider)?;
            total_lines += 1;

            write!(out, "\x1b[2;37m↑↓ Navigate     Enter Use     Esc Close\x1b[0m")?;
            total_lines += 1;

            let lines_up = (lines.len() - 1 - target_row) + visible_count + 5;
            execute!(out, cursor::MoveUp(lines_up as u16))?;
            let prefix = if target_row == 0 {
                &self.prompt_symbol
            } else {
                &self.multiline_symbol
            };
            let prefix_col = visible_width(prefix);
            let target_x = (prefix_col + target_col) as u16;
            execute!(out, cursor::MoveToColumn(target_x))?;

            *last_rendered_lines = total_lines;
            *last_cursor_row = running_lines + target_row;
        } else {
            // 3. Blank line between input and status
            write!(out, "\r\n")?;
            total_lines += 1;
            let model_label = crate::ui::repl::format_model_label(&self.active_model);
            let status_text = if self.running_status.is_some() {
                format!("enter queue · auto · {}", model_label)
            } else {
                format!("auto · {}", model_label)
            };
            write!(out, "\x1b[2;37m{}\x1b[0m", status_text)?;
            total_lines += 1;

            // Reposition cursor inside input box on active input row
            let lines_up = (lines.len() - 1 - target_row) + 2;
            execute!(out, cursor::MoveUp(lines_up as u16))?;

            let prefix = if target_row == 0 {
                &self.prompt_symbol
            } else {
                &self.multiline_symbol
            };
            let prefix_col = visible_width(prefix);
            let target_x = (prefix_col + target_col) as u16;
            execute!(out, cursor::MoveToColumn(target_x))?;

            *last_rendered_lines = total_lines;
            *last_cursor_row = running_lines + target_row;
        }

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
fn truncate_fit(s: &str, max_len: usize) -> String {
    if max_len == 0 {
        return String::new();
    }
    if s.chars().count() <= max_len {
        s.to_string()
    } else {
        let keep: String = s.chars().take(max_len.saturating_sub(1)).collect();
        format!("{}…", keep)
    }
}

fn slash_category_label(cat: crate::ui::slash::CommandCategory) -> &'static str {
    match cat {
        crate::ui::slash::CommandCategory::Core => "General",
        crate::ui::slash::CommandCategory::Session => "Session",
        crate::ui::slash::CommandCategory::Model => "Model",
        crate::ui::slash::CommandCategory::Config => "Config",
    }
}

fn model_category_label(id: &str, name: &str) -> &'static str {
    let lower = format!("{} {}", id, name).to_lowercase();
    if lower.contains("flash") || lower.contains("fast") {
        "Fast"
    } else if lower.contains("reason") || lower.contains("kimi") || lower.contains("minimax") {
        "Reasoning"
    } else if lower.contains("code") || lower.contains("coding") {
        "Coding"
    } else {
        "Model"
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

    #[test]
    fn test_render_model_picker_menu() {
        let models = vec![
            ("deepseek-ai/DeepSeek-V4-Flash-0731".to_string(), "DeepSeek V4 Flash".to_string()),
            ("MiniMaxAI/MiniMax-M2.7".to_string(), "MiniMax M2.7".to_string()),
            ("moonshotai/Kimi-K2.6".to_string(), "Kimi K2.6".to_string()),
        ];
        let prompt = Prompt::new()
            .with_models(models)
            .with_model_picker_active(true);

        let mut buf = Vec::new();
        let buffer: Vec<char> = Vec::new();
        let mut last_lines = 0;
        let mut last_cursor = 0;

        prompt
            .render_to(&mut buf, &buffer, 0, &mut last_lines, &mut last_cursor)
            .expect("render_to model picker failed");

        let raw = String::from_utf8_lossy(&buf);
        // Header row
        assert!(raw.contains("Models 3 · Type to filter"), "Missing header in:\n{}", raw);
        assert!(raw.contains("1-3"), "Missing range indicator in:\n{}", raw);
        // Top and bottom divider color
        assert!(raw.contains("\x1b[38;5;240m"), "Missing divider color in:\n{}", raw);
        // Footer hints
        assert!(
            raw.contains("↑↓ Navigate     Enter Use     Esc Close"),
            "Missing footer in:\n{}",
            raw
        );
        // Selected item bold
        assert!(raw.contains("\x1b[1;37m"), "Missing selected bold item in:\n{}", raw);
        // Categories
        assert!(raw.contains("Fast"), "Missing Fast category in:\n{}", raw);
        assert!(raw.contains("Reasoning"), "Missing Reasoning category in:\n{}", raw);
    }

    #[test]
    fn test_render_model_picker_with_filter() {
        let models = vec![
            ("deepseek-ai/DeepSeek-V4-Flash-0731".to_string(), "DeepSeek V4 Flash".to_string()),
            ("MiniMaxAI/MiniMax-M2.7".to_string(), "MiniMax M2.7".to_string()),
        ];
        let prompt = Prompt::new()
            .with_models(models)
            .with_model_picker_active(true);

        let mut buf = Vec::new();
        let buffer: Vec<char> = "flash".chars().collect();
        let mut last_lines = 0;
        let mut last_cursor = 0;

        prompt
            .render_to(&mut buf, &buffer, 5, &mut last_lines, &mut last_cursor)
            .expect("render_to filtered model picker failed");

        let raw = String::from_utf8_lossy(&buf);
        assert!(raw.contains("Models 1 · Type to filter"));
        assert!(raw.contains("deepseek-ai/DeepSeek-V4-Flash"));
        assert!(!raw.contains("MiniMax-M2.7"));
    }

    #[test]
    fn test_render_slash_suggestions_menu() {
        let prompt = Prompt::new();
        let mut buf = Vec::new();
        let buffer: Vec<char> = "/".chars().collect();
        let mut last_lines = 0;
        let mut last_cursor = 0;

        prompt
            .render_to(&mut buf, &buffer, 1, &mut last_lines, &mut last_cursor)
            .expect("render_to slash suggestions failed");

        let raw = String::from_utf8_lossy(&buf);
        assert!(raw.contains("Commands"), "Missing Commands header in:\n{}", raw);
        assert!(raw.contains("Type to filter"), "Missing filter hint in:\n{}", raw);
        assert!(raw.contains("↑↓ Navigate     Enter Use     Esc Close"));
        assert!(raw.contains("/help") || raw.contains("/model") || raw.contains("/clear"));
    }

    #[test]
    fn test_model_category_labels() {
        assert_eq!(model_category_label("gpt-4o-fast", "Fast GPT"), "Fast");
        assert_eq!(model_category_label("deepseek-ai/DeepSeek-V4-Flash-0731", "DeepSeek V4 Flash"), "Fast");
        assert_eq!(model_category_label("moonshotai/Kimi-K2.6", "Kimi K2.6"), "Reasoning");
        assert_eq!(model_category_label("MiniMaxAI/MiniMax-M2.7", "MiniMax M2.7"), "Reasoning");
        assert_eq!(model_category_label("qwen/qwen-coder-32b", "Qwen Coder"), "Coding");
        assert_eq!(model_category_label("custom-model", "Custom Model"), "Model");
    }
}
