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
pub const EFFORT_OPTIONS: &[&str] = &["default", "xhigh", "high", "medium", "low"];

/// Interactive terminal prompt supporting line editing, multiline input,
/// An owned suggestion entry for the slash autocomplete dropdown.
/// Merges static command palette entries with dynamic skill entries.
#[derive(Debug, Clone)]
pub struct SlashSuggestion {
    pub name: String,
    pub description: String,
    pub category: String,
    /// Whether this entry is a skill (vs a built-in command).
    pub is_skill: bool,
    /// Source label for skills (e.g. "Claude", "Fusion", "Global").
    pub source: String,
}

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
    pub queued_count: usize,
    pub is_running: bool,
    pub buffer: Vec<char>,
    pub cursor_pos: usize,
    saved_current: String,
    pub last_rendered_lines: usize,
    pub last_cursor_row: usize,
    pub effort_picker_active: bool,
    pub effort_selection: usize,
    pub pending_model_id: String,
    pub selected_effort: Option<String>,
    /// Dynamic skill entries for the slash autocomplete dropdown.
    skill_suggestions: Vec<SlashSuggestion>,
    /// Whether the skill picker panel is active (opened from `/skills`).
    pub skill_picker_active: bool,
    /// Highlighted index inside the skill picker.
    pub skill_picker_selection: usize,
    /// Active source filter tab index (0 = All).
    pub skill_picker_source: usize,
    /// Currently active skill: (name, source_label).
    pub active_skill: Option<(String, String)>,
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
            queued_count: 0,
            is_running: false,
            buffer: Vec::new(),
            cursor_pos: 0,
            saved_current: String::new(),
            last_rendered_lines: 0,
            last_cursor_row: 0,
            effort_picker_active: false,
            effort_selection: 0,
            pending_model_id: String::new(),
            selected_effort: None,
            skill_suggestions: Vec::new(),
            skill_picker_active: false,
            skill_picker_selection: 0,
            skill_picker_source: 0,
            active_skill: None,
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
    pub fn with_skill_suggestions(mut self, suggestions: Vec<SlashSuggestion>) -> Self {
        self.skill_suggestions = suggestions;
        self
    }

    pub fn set_skill_suggestions(&mut self, suggestions: Vec<SlashSuggestion>) {
        self.skill_suggestions = suggestions;
    }

    /// Set the active skill (name, source label) picked by the user.
    pub fn set_active_skill(&mut self, skill: Option<(String, String)>) {
        self.active_skill = skill;
    }

    /// Take and clear the active skill; used by the REPL after submit.
    pub fn take_active_skill(&mut self) -> Option<(String, String)> {
        self.active_skill.take()
    }

    /// Current active skill, if any.
    pub fn active_skill(&self) -> Option<&(String, String)> {
        self.active_skill.as_ref()
    }

    /// Filtered skill list for the picker: source tab filter + buffer text query.
    fn skill_picker_filtered(&self) -> Vec<&SlashSuggestion> {
        let query: String = self.buffer.iter().collect::<String>().to_lowercase();
        self.skill_suggestions
            .iter()
            .filter(|s| {
                if !s.is_skill {
                    return false;
                }
                let src_ok = match self.skill_picker_source {
                    0 => true,
                    1 => s.source == "Fusion",
                    2 => s.source == "Claude",
                    3 => s.source == "Global",
                    _ => s.source == "Custom" || s.source == "Builtin",
                };
                if !src_ok {
                    return false;
                }
                if query.is_empty() {
                    return true;
                }
                let name = s.name.strip_prefix("skill:").unwrap_or(&s.name);
                name.to_lowercase().contains(&query)
            })
            .collect()
    }

    /// Visible width of the active-skill chip rendered on the input row.
    fn skill_chip_width(&self) -> usize {
        if let Some((name, source)) = &self.active_skill {
            UnicodeWidthStr::width(name.as_str())
                + 3 // " · "
                + UnicodeWidthStr::width(source.as_str())
                + 2 // trailing "  "
        } else {
            0
        }
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

    /// Toggle whether the effort picker dialog is active.
    pub fn with_effort_picker_active(mut self, active: bool) -> Self {
        self.effort_picker_active = active;
        self
    }

    /// Set whether the effort picker dialog is active.
    pub fn set_effort_picker_active(&mut self, active: bool) {
        self.effort_picker_active = active;
    }

    /// Whether the effort picker dialog is active.
    pub fn effort_picker_active(&self) -> bool {
        self.effort_picker_active
    }

    /// Set the selected index for the effort picker dialog.
    pub fn with_effort_selection(mut self, sel: usize) -> Self {
        self.effort_selection = sel;
        self
    }

    /// Set the selected index for the effort picker dialog.
    pub fn set_effort_selection(&mut self, sel: usize) {
        self.effort_selection = sel;
    }

    /// Get the selected index for the effort picker dialog.
    pub fn effort_selection(&self) -> usize {
        self.effort_selection
    }

    /// Set pending model ID for effort picker dialog.
    pub fn with_pending_model_id(mut self, model: impl Into<String>) -> Self {
        self.pending_model_id = model.into();
        self
    }

    /// Set pending model ID for effort picker dialog.
    pub fn set_pending_model_id(&mut self, model: impl Into<String>) {
        self.pending_model_id = model.into();
    }

    /// Get pending model ID for effort picker dialog.
    pub fn pending_model_id(&self) -> &str {
        &self.pending_model_id
    }

    /// Set selected reasoning effort.
    pub fn with_selected_effort(mut self, effort: Option<String>) -> Self {
        self.selected_effort = effort;
        self
    }

    /// Set selected reasoning effort.
    pub fn set_selected_effort(&mut self, effort: Option<String>) {
        self.selected_effort = effort;
    }

    /// Get selected reasoning effort.
    pub fn selected_effort(&self) -> Option<&str> {
        self.selected_effort.as_deref()
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
        self.effort_picker_active = false;
        self.effort_selection = 0;
        self.pending_model_id.clear();
        self.reset_render_state();
        self.running_status = None;
        self.queued_count = 0;
        self.is_running = false;
        self.cancel_pressed = false;
        if self.key_handler.profile() == KeybindingProfile::Vi {
            self.key_handler.set_vi_mode(ViMode::Insert);
        }
        self.key_handler.clear_pending();
    }

    /// Reset rendered lines and cursor row tracking.
    /// Call this whenever external output has been printed to stdout (e.g. streaming markdown deltas,
    /// tool execution tree, turn stats) causing the terminal to scroll and invalidating prior cursor offsets.
    pub fn reset_render_state(&mut self) {
        self.last_rendered_lines = 0;
        self.last_cursor_row = 0;
    }

    /// Update active running/thinking status banner displayed above the prompt.
    pub fn set_running_status(&mut self, status: Option<String>) {
        self.running_status = status;
    }

    /// Set number of queued messages displayed in banner and status line.
    pub fn with_queued_count(mut self, count: usize) -> Self {
        self.queued_count = count;
        self
    }

    /// Set number of queued messages displayed in banner and status line.
    pub fn set_queued_count(&mut self, count: usize) {
        self.queued_count = count;
    }

    /// Get number of queued messages.
    pub fn queued_count(&self) -> usize {
        self.queued_count
    }
    /// Update active running state of the prompt.
    pub fn set_running(&mut self, running: bool) {
        self.is_running = running;
        if !running {
            self.reset_render_state();
        }
    }

    /// Builder method to set active running state.
    pub fn with_running(mut self, running: bool) -> Self {
        self.is_running = running;
        self
    }

    /// Check whether the prompt is currently in an active running state.
    pub fn is_running(&self) -> bool {
        self.is_running
    }
    /// Render current prompt state to stdout.
    pub fn render_current(&mut self) -> std::io::Result<()> {
        let mut out = stdout();
        let buffer = self.buffer.clone();
        let cursor_pos = self.cursor_pos;
        let mut last_lines = self.last_rendered_lines;
        let mut last_row = self.last_cursor_row;
        self.render_to(
            &mut out,
            &buffer,
            cursor_pos,
            &mut last_lines,
            &mut last_row,
        )?;
        self.last_rendered_lines = last_lines;
        self.last_cursor_row = last_row;
        Ok(())
    }

    /// Erase the rendered prompt frame from screen.
    pub fn clear_frame(&mut self) -> std::io::Result<()> {
        if self.last_rendered_lines == 0
            || self.last_cursor_row > 50
            || self.last_rendered_lines > 50
        {
            self.reset_render_state();
            return Ok(());
        }
        let term_rows = terminal::size().map(|(_, h)| h as usize).unwrap_or(24);
        let max_up = term_rows.saturating_sub(1).min(50);
        let up = self.last_cursor_row.min(max_up);
        let mut out = stdout();
        if up > 0 {
            execute!(out, cursor::MoveUp(up as u16))?;
        }
        execute!(
            out,
            cursor::MoveToColumn(0),
            terminal::Clear(ClearType::FromCursorDown)
        )?;
        out.flush()?;
        self.reset_render_state();
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
                    if self.effort_picker_active {
                        self.effort_picker_active = false;
                        self.effort_selection = 0;
                        self.pending_model_id.clear();
                        self.buffer.clear();
                        self.cursor_pos = 0;
                        self.render_current()?;
                        return Ok(None);
                    }
                    if self.model_picker_active {
                        self.model_picker_active = false;
                        self.buffer.clear();
                        self.cursor_pos = 0;
                        self.render_current()?;
                        return Ok(None);
                    }
                    if self.skill_picker_active {
                        self.skill_picker_active = false;
                        self.skill_picker_selection = 0;
                        self.skill_picker_source = 0;
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
                    self.effort_picker_active = false;
                    self.effort_selection = 0;
                    self.pending_model_id.clear();
                    self.model_picker_active = false;
                    self.skill_picker_active = false;
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

                // Effort picker dialog mode
                if self.effort_picker_active {
                    match key.code {
                        KeyCode::Tab | KeyCode::Down => {
                            self.effort_selection =
                                (self.effort_selection + 1) % EFFORT_OPTIONS.len();
                            self.render_current()?;
                            return Ok(None);
                        }
                        KeyCode::BackTab | KeyCode::Up => {
                            self.effort_selection = if self.effort_selection == 0 {
                                EFFORT_OPTIONS.len() - 1
                            } else {
                                self.effort_selection - 1
                            };
                            self.render_current()?;
                            return Ok(None);
                        }
                        KeyCode::Enter => {
                            let effort =
                                EFFORT_OPTIONS[self.effort_selection.min(EFFORT_OPTIONS.len() - 1)];
                            let cmd = if effort == "default" {
                                format!("/model {}", self.pending_model_id)
                            } else {
                                format!("/model {} {}", self.pending_model_id, effort)
                            };
                            self.selected_effort = if effort == "default" {
                                None
                            } else {
                                Some(effort.to_string())
                            };
                            self.clear_frame()?; // Do NOT print ┃ /model ...
                            self.effort_picker_active = false;
                            self.effort_selection = 0;
                            self.pending_model_id.clear();
                            self.buffer.clear();
                            self.cursor_pos = 0;
                            self.add_history(cmd.clone());
                            return Ok(Some(PromptResult::Submit(cmd)));
                        }
                        _ => return Ok(None),
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
                                self.model_selection = (self.model_selection + 1) % filtered.len();
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
                                let model_id = sel.0.clone();
                                self.pending_model_id = model_id.clone();
                                self.model_picker_active = false;
                                self.model_selection = 0;
                                self.effort_picker_active = true;
                                self.effort_selection = 0;
                                let prompt_text = format!("/model {} ", model_id);
                                self.buffer = prompt_text.chars().collect();
                                self.cursor_pos = self.buffer.len();
                                self.render_current()?;
                                return Ok(None);
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

                // Skill picker dialog mode
                if self.skill_picker_active {
                    let filtered = self.skill_picker_filtered();
                    match key.code {
                        KeyCode::Down => {
                            if !filtered.is_empty() {
                                self.skill_picker_selection =
                                    (self.skill_picker_selection + 1) % filtered.len();
                            }
                            self.render_current()?;
                            return Ok(None);
                        }
                        KeyCode::Up => {
                            if !filtered.is_empty() {
                                self.skill_picker_selection = if self.skill_picker_selection == 0 {
                                    filtered.len() - 1
                                } else {
                                    self.skill_picker_selection - 1
                                };
                            }
                            self.render_current()?;
                            return Ok(None);
                        }
                        KeyCode::Tab | KeyCode::BackTab => {
                            // Tab cycles source filter (fx-style)
                            self.skill_picker_source = (self.skill_picker_source + 1) % 5;
                            self.skill_picker_selection = 0;
                            self.render_current()?;
                            return Ok(None);
                        }
                        KeyCode::Enter => {
                            if let Some(sel) = filtered.get(
                                self.skill_picker_selection
                                    .min(filtered.len().saturating_sub(1)),
                            ) {
                                let skill_name =
                                    sel.name.strip_prefix("skill:").unwrap_or(&sel.name);
                                // Store active skill; clear picker; user keeps typing
                                self.active_skill =
                                    Some((skill_name.to_string(), sel.source.clone()));
                                self.skill_picker_active = false;
                                self.skill_picker_selection = 0;
                                self.skill_picker_source = 0;
                                self.buffer.clear();
                                self.cursor_pos = 0;
                                self.render_current()?;
                                return Ok(None);
                            }
                            return Ok(None);
                        }
                        KeyCode::Esc => {
                            self.skill_picker_active = false;
                            self.skill_picker_selection = 0;
                            self.skill_picker_source = 0;
                            self.buffer.clear();
                            self.cursor_pos = 0;
                            self.render_current()?;
                            return Ok(None);
                        }
                        KeyCode::Backspace => {
                            if self.cursor_pos > 0 {
                                self.buffer.remove(self.cursor_pos - 1);
                                self.cursor_pos -= 1;
                            }
                            self.skill_picker_selection = 0;
                            self.render_current()?;
                            return Ok(None);
                        }
                        KeyCode::Char(c) => {
                            self.buffer.insert(self.cursor_pos, c);
                            self.cursor_pos += 1;
                            self.skill_picker_selection = 0;
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
                    let matches = slash_matches(first_line, &self.skill_suggestions);
                    if !matches.is_empty() {
                        match key.code {
                            KeyCode::Tab | KeyCode::Down => {
                                self.slash_selection = (self.slash_selection + 1) % matches.len();
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
                                    if !sel.is_skill && sel.name == "/model" {
                                        self.buffer.clear();
                                        self.cursor_pos = 0;
                                        self.model_picker_active = true;
                                        self.model_selection = 0;
                                        self.render_current()?;
                                        return Ok(None);
                                    }
                                    if !sel.is_skill
                                        && (sel.name == "/skills" || sel.name == "/skill")
                                    {
                                        self.buffer.clear();
                                        self.cursor_pos = 0;
                                        self.skill_picker_active = true;
                                        self.skill_picker_selection = 0;
                                        self.skill_picker_source = 0;
                                        self.render_current()?;
                                        return Ok(None);
                                    }
                                    if sel.is_skill {
                                        let skill_name =
                                            sel.name.strip_prefix("skill:").unwrap_or(&sel.name);
                                        let cmd = format!("/skill {}", skill_name);
                                        self.clear_frame()?;
                                        self.add_history(cmd.clone());
                                        self.buffer.clear();
                                        self.cursor_pos = 0;
                                        self.slash_selection = 0;
                                        return Ok(Some(PromptResult::Submit(cmd)));
                                    }
                                    let cmd = sel.name.clone();
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
                        if !self.is_running
                            && self.running_status.is_none()
                            && self.queued_count == 0
                        {
                            let mut out = stdout();
                            let lines: Vec<&str> = text.split('\n').collect();
                            for line in &lines {
                                let _ = write!(out, "\x1b[1m┃ {}\x1b[0m\r\n", line);
                            }
                            let _ = write!(out, "\r\n");
                            let _ = out.flush();
                        }
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
                        self.reset_render_state();
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
                self.key_handler
                    .snapshot_undo(&self.buffer, self.cursor_pos);
                let normalized = text.replace("\r\n", "\n").replace('\r', "\n");
                for c in normalized.chars() {
                    self.buffer.insert(self.cursor_pos, c);
                    self.cursor_pos += 1;
                }
                self.render_current()?;
                Ok(None)
            }
            Event::Resize(_, _) => {
                self.reset_render_state();
                self.render_current()?;
                Ok(None)
            }
            _ => Ok(None),
        }
    }

    /// Read an interactive line / multiline input from user.
    pub fn read_input(&mut self) -> std::io::Result<PromptResult> {
        let _raw_guard = RawModeGuard::enter()?;
        if self.last_rendered_lines > 0 {
            self.clear_frame()?;
        }
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
                        id.to_lowercase().contains(&query) || name.to_lowercase().contains(&query)
                    })
                    .collect()
            };
        }

        // Check for slash suggestions
        let first_line = lines.first().copied().unwrap_or("");
        let slash_suggestions = if !self.model_picker_active
            && !self.effort_picker_active
            && first_line.starts_with('/')
        {
            slash_matches(first_line, &self.skill_suggestions)
        } else {
            Vec::new()
        };

        let (term_cols, term_rows) = terminal::size()
            .map(|(w, h)| (w as usize, h as usize))
            .unwrap_or((80, 24));
        let max_up = term_rows.saturating_sub(1).min(50);

        // Clear previous frame using exact relative cursor movement
        // Guard against out-of-bounds relative cursor jumps that erase streamed terminal content
        if *last_rendered_lines > 0
            && *last_cursor_row > 0
            && *last_cursor_row <= 50
            && *last_rendered_lines <= 50
        {
            let up = (*last_cursor_row).min(max_up);
            if up > 0 {
                execute!(out, cursor::MoveUp(up as u16))?;
            }
        }
        execute!(
            out,
            cursor::MoveToColumn(0),
            terminal::Clear(ClearType::FromCursorDown)
        )?;

        let mut total_lines = 0;

        let running_lines = if let Some(status) = &self.running_status {
            let max_w = term_cols.saturating_sub(4);
            let display_status = if max_w > 0 {
                truncate_fit(status, max_w)
            } else {
                status.clone()
            };
            write!(out, "\x1b[2K  \x1b[2;37m{}\x1b[0m\r\n\r\n", display_status)?;
            2
        } else {
            0
        };

        let queue_banner_lines = if self.queued_count > 0 {
            let banner = if self.queued_count == 1 {
                "1 queued message · ↑ to edit".to_string()
            } else {
                format!("{} queued messages · ↑ to edit", self.queued_count)
            };
            write!(out, "\x1b[2;37m{}\x1b[0m\r\n\r\n", banner)?;
            2
        } else {
            0
        };
        let header_lines = running_lines + queue_banner_lines;
        total_lines += header_lines;
        // 1. Input lines with clean vertical rail symbol (┃ )
        for (idx, line) in lines.iter().enumerate() {
            let prefix = if idx == 0 {
                &self.prompt_symbol
            } else {
                &self.multiline_symbol
            };

            write!(out, "{}", prefix)?;
            if idx == 0 {
                // Active skill chip prefix (fx-style): skill name · source
                if let Some((skill_name, source)) = &self.active_skill {
                    write!(
                        out,
                        "\x1b[1;36m{}\x1b[0m \x1b[2;37m· {}\x1b[0m  ",
                        skill_name, source
                    )?;
                }
            }
            if idx == 0 && line.is_empty() && lines.len() == 1 && !self.is_running {
                if let Some(ph) = &self.placeholder {
                    if !ph.is_empty() {
                        write!(out, "\x1b[2;37m{}\x1b[0m", ph)?;
                    }
                }
            } else {
                write!(out, "{}", line)?;
            }
            write!(out, "\r\n")?;
            total_lines += 1;
        }

        let divider = "─".repeat(term_cols);

        // 2. Dropdown menu below the input line (matching fx)
        if self.effort_picker_active {
            // Top divider
            write!(out, "\x1b[38;5;240m{}\x1b[0m\r\n", divider)?;
            total_lines += 1;

            // 5 effort options
            for (idx, &opt) in EFFORT_OPTIONS.iter().enumerate() {
                let is_selected = idx == self.effort_selection;
                if is_selected {
                    write!(out, "\x1b[1;37m{}\x1b[0m\r\n", opt)?;
                } else {
                    write!(out, "\x1b[2;37m{}\x1b[0m\r\n", opt)?;
                }
                total_lines += 1;
            }

            // Bottom divider
            write!(out, "\x1b[38;5;240m{}\x1b[0m\r\n", divider)?;
            total_lines += 1;

            // Status line dynamically updates to auto · <model> or auto · <model> · <effort>
            let model_label = if !self.pending_model_id.is_empty() {
                crate::ui::repl::format_model_label(&self.pending_model_id)
            } else {
                crate::ui::repl::format_model_label(&self.active_model)
            };
            let current_effort =
                EFFORT_OPTIONS[self.effort_selection.min(EFFORT_OPTIONS.len() - 1)];
            let mut status_body = format!("auto · {}", model_label);
            if current_effort != "default" {
                status_body.push_str(&format!(" · {}", current_effort));
            }
            let status_text = if self.queued_count > 0 {
                format!(
                    "queued {} · enter queue · {}",
                    self.queued_count, status_body
                )
            } else if self.running_status.is_some() || self.is_running {
                format!("enter queue · {}", status_body)
            } else {
                status_body
            };
            write!(out, "\x1b[2;37m{}\x1b[0m", status_text)?;
            total_lines += 1;

            let lines_up = ((lines.len() - 1 - target_row) + EFFORT_OPTIONS.len() + 3).min(max_up);
            execute!(out, cursor::MoveUp(lines_up as u16))?;
            let prefix = if target_row == 0 {
                &self.prompt_symbol
            } else {
                &self.multiline_symbol
            };
            let prefix_col = visible_width(prefix);
            let chip_w = if target_row == 0 {
                self.skill_chip_width()
            } else {
                0
            };
            let target_x = (prefix_col + chip_w + target_col) as u16;
            execute!(out, cursor::MoveToColumn(target_x))?;

            *last_rendered_lines = total_lines;
            *last_cursor_row = header_lines + target_row;
        } else if self.model_picker_active {
            let sel = if filtered_models.is_empty() {
                0
            } else {
                self.model_selection
                    .min(filtered_models.len().saturating_sub(1))
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
            write!(
                out,
                "\x1b[2;37m{}{}{}\x1b[0m\r\n",
                left,
                " ".repeat(gap),
                right
            )?;
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
                        col1,
                        col2,
                        cat,
                        c1 = col1_w,
                        c2 = col2_w,
                        c3 = col3_w
                    )?;
                }
                total_lines += 1;
            }

            write!(out, "\x1b[38;5;240m{}\x1b[0m\r\n", divider)?;
            total_lines += 1;

            write!(
                out,
                "\x1b[2;37m↑↓ Navigate     Enter Use     Esc Close\x1b[0m"
            )?;
            total_lines += 1;

            let lines_up = ((lines.len() - 1 - target_row) + visible_count + 5).min(max_up);
            execute!(out, cursor::MoveUp(lines_up as u16))?;
            let prefix = if target_row == 0 {
                &self.prompt_symbol
            } else {
                &self.multiline_symbol
            };
            let prefix_col = visible_width(prefix);
            let chip_w = if target_row == 0 {
                self.skill_chip_width()
            } else {
                0
            };
            let target_x = (prefix_col + chip_w + target_col) as u16;
            execute!(out, cursor::MoveToColumn(target_x))?;

            *last_rendered_lines = total_lines;
            *last_cursor_row = header_lines + target_row;
        } else if self.skill_picker_active {
            // Filter by source tab (0=All, 1=Fusion, 2=Claude, 3=Global, 4=Custom/Builtin)
            let filtered = self.skill_picker_filtered();
            let sel = if filtered.is_empty() {
                0
            } else {
                self.skill_picker_selection
                    .min(filtered.len().saturating_sub(1))
            };
            let window_start = if sel >= 8 { sel - 7 } else { 0 };
            let visible_items: Vec<_> = filtered
                .iter()
                .enumerate()
                .skip(window_start)
                .take(8)
                .collect();
            let visible_count = visible_items.len();

            write!(out, "\x1b[38;5;240m{}\x1b[0m\r\n", divider)?;
            total_lines += 1;

            // Header: Skills <count> [tabs]
            let tabs = ["All", "Fusion", "Claude", "Global", "Other"];
            let mut header = format!("Skills {}", filtered.len());
            for (i, tab) in tabs.iter().enumerate() {
                if i == self.skill_picker_source {
                    header.push_str(&format!(" \x1b[1;37m[{}]\x1b[0m", tab));
                } else {
                    header.push_str(&format!(" \x1b[2;37m{}\x1b[0m", tab));
                }
            }
            write!(out, "\x1b[2;37m{}\x1b[0m\r\n", header)?;
            total_lines += 1;

            write!(out, "\r\n")?;
            total_lines += 1;

            for (idx, item) in &visible_items {
                let is_selected = *idx == sel;
                let col1_w = visible_items
                    .iter()
                    .map(|(_, s)| UnicodeWidthStr::width(s.name.as_str()))
                    .max()
                    .unwrap_or(20)
                    .max(20)
                    .min(term_cols.saturating_sub(30));
                let col3_w = 12;
                let col2_w = term_cols.saturating_sub(col1_w + col3_w + 4);

                let col1 = truncate_fit(&item.name, col1_w);
                let col2 = truncate_fit(&item.description, col2_w);
                let src = &item.source;

                if is_selected {
                    write!(
                        out,
                        "\x1b[1;37m{:<c1$}\x1b[0m \x1b[37m{:<c2$}\x1b[0m \x1b[2;37m{:>c3$}\x1b[0m\r\n",
                        col1, col2, src, c1 = col1_w, c2 = col2_w, c3 = col3_w
                    )?;
                } else {
                    write!(
                        out,
                        "\x1b[2;37m{:<c1$} {:<c2$} {:>c3$}\x1b[0m\r\n",
                        col1,
                        col2,
                        src,
                        c1 = col1_w,
                        c2 = col2_w,
                        c3 = col3_w
                    )?;
                }
                total_lines += 1;
            }

            write!(out, "\x1b[38;5;240m{}\x1b[0m\r\n", divider)?;
            total_lines += 1;

            write!(
                out,
                "\x1b[2;37m↑↓ Navigate     Tab Source     Enter Use     Esc Close\x1b[0m"
            )?;
            total_lines += 1;

            let lines_up = ((lines.len() - 1 - target_row) + visible_count + 5).min(max_up);
            execute!(out, cursor::MoveUp(lines_up as u16))?;
            let prefix = if target_row == 0 {
                &self.prompt_symbol
            } else {
                &self.multiline_symbol
            };
            let prefix_col = visible_width(prefix);
            let chip_w = if target_row == 0 {
                self.skill_chip_width()
            } else {
                0
            };
            let target_x = (prefix_col + chip_w + target_col) as u16;
            execute!(out, cursor::MoveToColumn(target_x))?;

            *last_rendered_lines = total_lines;
            *last_cursor_row = header_lines + target_row;
        } else if !slash_suggestions.is_empty() {
            let sel = self
                .slash_selection
                .min(slash_suggestions.len().saturating_sub(1));
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

            let has_skills = slash_suggestions.iter().any(|s| s.is_skill);
            let noun = if first_line == "/" {
                if has_skills {
                    "Commands & Skills"
                } else {
                    "Commands"
                }
            } else {
                "Results"
            };
            let left = if first_line == "/" {
                format!("{} {} · Type to filter", noun, slash_suggestions.len())
            } else {
                format!("{} {}", noun, slash_suggestions.len())
            };
            let right = format!("{}-{}", window_start + 1, window_end);
            let gap = term_cols.saturating_sub(left.len() + right.len());
            write!(
                out,
                "\x1b[2;37m{}{}{}\x1b[0m\r\n",
                left,
                " ".repeat(gap),
                right
            )?;
            total_lines += 1;

            write!(out, "\r\n")?;
            total_lines += 1;

            // Compute col1 width dynamically based on longest visible name
            let col1_w = visible_items
                .iter()
                .map(|(_, item)| UnicodeWidthStr::width(item.name.as_str()))
                .max()
                .unwrap_or(14)
                .max(14)
                .min(term_cols.saturating_sub(20));
            let col3_w = 10;
            let col2_w = term_cols.saturating_sub(col1_w + col3_w + 4);

            for (idx, item) in visible_items {
                let is_selected = idx == sel;
                let cat = &item.category;

                let col1 = truncate_fit(&item.name, col1_w);
                let col2 = truncate_fit(&item.description, col2_w);

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
                        col1,
                        col2,
                        cat,
                        c1 = col1_w,
                        c2 = col2_w,
                        c3 = col3_w
                    )?;
                }
                total_lines += 1;
            }

            write!(out, "\x1b[38;5;240m{}\x1b[0m\r\n", divider)?;
            total_lines += 1;

            write!(
                out,
                "\x1b[2;37m↑↓ Navigate     Enter Use     Esc Close\x1b[0m"
            )?;
            total_lines += 1;

            let lines_up = ((lines.len() - 1 - target_row) + visible_count + 5).min(max_up);
            execute!(out, cursor::MoveUp(lines_up as u16))?;
            let prefix = if target_row == 0 {
                &self.prompt_symbol
            } else {
                &self.multiline_symbol
            };
            let prefix_col = visible_width(prefix);
            let chip_w = if target_row == 0 {
                self.skill_chip_width()
            } else {
                0
            };
            let target_x = (prefix_col + chip_w + target_col) as u16;
            execute!(out, cursor::MoveToColumn(target_x))?;

            *last_rendered_lines = total_lines;
            *last_cursor_row = header_lines + target_row;
        } else {
            // 3. Blank line between input and status
            write!(out, "\r\n")?;
            total_lines += 1;
            let model_label = crate::ui::repl::format_model_label(&self.active_model);
            let mode_prefix = if self.show_mode_indicator {
                if let Some(indicator) = self.key_handler.mode_indicator() {
                    format!("{} ", indicator)
                } else {
                    String::new()
                }
            } else {
                String::new()
            };
            let mut status_body = format!("{}auto · {}", mode_prefix, model_label);
            if let Some(effort) = &self.selected_effort {
                status_body.push_str(&format!(" · {}", effort));
            }
            let status_text = if self.queued_count > 0 {
                format!(
                    "queued {} · enter queue · {}",
                    self.queued_count, status_body
                )
            } else if self.running_status.is_some() || self.is_running {
                format!("enter queue · {}", status_body)
            } else {
                status_body
            };
            write!(out, "\x1b[2;37m{}\x1b[0m\r\n", status_text)?;
            total_lines += 1;

            // Reposition cursor inside input box on active input row
            let lines_up = ((lines.len() - 1 - target_row) + 3).min(max_up);
            execute!(out, cursor::MoveUp(lines_up as u16))?;

            let prefix = if target_row == 0 {
                &self.prompt_symbol
            } else {
                &self.multiline_symbol
            };
            let prefix_col = visible_width(prefix);
            let chip_w = if target_row == 0 {
                self.skill_chip_width()
            } else {
                0
            };
            let target_x = (prefix_col + chip_w + target_col) as u16;
            execute!(out, cursor::MoveToColumn(target_x))?;

            *last_rendered_lines = total_lines;
            *last_cursor_row = header_lines + target_row;
        }

        out.flush()?;
        Ok(())
    }

    /// Render a submitted user input prompt to a generic writer.
    pub fn render_submitted_prompt_to<W: std::io::Write>(
        out: &mut W,
        text: &str,
    ) -> std::io::Result<()> {
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

fn slash_category_label_static(cat: crate::ui::slash::CommandCategory) -> String {
    match cat {
        crate::ui::slash::CommandCategory::Core => "General".to_string(),
        crate::ui::slash::CommandCategory::Session => "Session".to_string(),
        crate::ui::slash::CommandCategory::Model => "Model".to_string(),
        crate::ui::slash::CommandCategory::Config => "Config".to_string(),
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
fn slash_matches(typed: &str, skills: &[SlashSuggestion]) -> Vec<SlashSuggestion> {
    let query = typed.trim_start().to_lowercase();
    let skill_query = query.strip_prefix('/').unwrap_or(&query);

    // Check if user is specifically looking for skills (typed /skill: or /sk)
    let wants_skills = skill_query.starts_with("sk") || skill_query.starts_with("skill");

    let mut cmd_results: Vec<SlashSuggestion> = crate::ui::slash::COMMAND_PALETTE
        .iter()
        .filter(|d| {
            d.name.to_lowercase().starts_with(&query)
                || d.aliases
                    .iter()
                    .any(|a| a.to_lowercase().starts_with(&query))
        })
        .map(|d| SlashSuggestion {
            name: d.name.to_string(),
            description: d.description.to_string(),
            category: slash_category_label_static(d.category),
            is_skill: false,
            source: String::new(),
        })
        .collect();

    // Build skill entries with skill: prefix
    let mut skill_results: Vec<SlashSuggestion> = Vec::new();
    for s in skills {
        let prefixed = format!("skill:{}", s.name);
        let prefixed_lower = prefixed.to_lowercase();
        if skill_query.is_empty()
            || prefixed_lower.starts_with(skill_query)
            || prefixed_lower.contains(skill_query)
            || s.name.to_lowercase().contains(skill_query)
        {
            skill_results.push(SlashSuggestion {
                name: prefixed,
                description: s.description.clone(),
                category: s.category.clone(),
                is_skill: true,
                source: s.source.clone(),
            });
        }
    }

    // When user specifically types /skill:, show only skills
    if wants_skills && !skill_results.is_empty() {
        // Put /skill command first (opens the picker on Enter), then all matching skills
        let mut results = Vec::new();
        if let Some(d) = crate::ui::slash::COMMAND_PALETTE
            .iter()
            .find(|d| d.name == "/skill")
        {
            results.push(SlashSuggestion {
                name: d.name.to_string(),
                description: d.description.to_string(),
                category: slash_category_label_static(d.category),
                is_skill: false,
                source: String::new(),
            });
        }
        results.extend(skill_results);
        results.truncate(12);
        return results;
    }

    // Default: commands first, then skills, with room for both
    let skill_count = skill_results.len().min(3);
    let cmd_limit = 8usize.saturating_sub(skill_count);
    cmd_results.truncate(cmd_limit);
    cmd_results.extend(skill_results.into_iter().take(3));
    cmd_results
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
        let m = slash_matches("/hel", &[]);
        assert!(!m.is_empty());
        assert!(m.iter().any(|d| d.name == "/help"));

        // "/mo" should match "/model"
        let m = slash_matches("/mo", &[]);
        assert!(m.iter().any(|d| d.name == "/model"));

        // Unknown prefix -> no matches
        assert!(slash_matches("/zzz-nope", &[]).is_empty());
    }

    #[test]
    fn test_slash_matches_respects_alias() {
        // "/pal" is an alias of "/palette"
        let m = slash_matches("/pal", &[]);
        assert!(m.iter().any(|d| d.name == "/palette"));
    }

    #[test]
    fn test_slash_matches_case_insensitive() {
        assert!(slash_matches("/HEL", &[]).iter().any(|d| d.name == "/help"));
    }

    #[test]
    fn test_slash_matches_with_skills() {
        let skills = vec![
            SlashSuggestion {
                name: "commit".to_string(),
                description: "Generate Git commit message".to_string(),
                category: "Skill".to_string(),
                is_skill: true,
                source: "Fusion".to_string(),
            },
            SlashSuggestion {
                name: "review".to_string(),
                description: "Code review".to_string(),
                category: "Skill".to_string(),
                is_skill: true,
                source: "Claude".to_string(),
            },
        ];
        let m = slash_matches("/com", &skills);
        assert!(m.iter().any(|s| s.name == "skill:commit" && s.is_skill));

        let m2 = slash_matches("/rev", &skills);
        assert!(m2.iter().any(|s| s.name == "skill:review" && s.is_skill));
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
            (
                "deepseek-ai/DeepSeek-V4-Flash-0731".to_string(),
                "DeepSeek V4 Flash".to_string(),
            ),
            (
                "MiniMaxAI/MiniMax-M2.7".to_string(),
                "MiniMax M2.7".to_string(),
            ),
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
        assert!(
            raw.contains("Models 3 · Type to filter"),
            "Missing header in:\n{}",
            raw
        );
        assert!(raw.contains("1-3"), "Missing range indicator in:\n{}", raw);
        // Top and bottom divider color
        assert!(
            raw.contains("\x1b[38;5;240m"),
            "Missing divider color in:\n{}",
            raw
        );
        // Footer hints
        assert!(
            raw.contains("↑↓ Navigate     Enter Use     Esc Close"),
            "Missing footer in:\n{}",
            raw
        );
        // Selected item bold
        assert!(
            raw.contains("\x1b[1;37m"),
            "Missing selected bold item in:\n{}",
            raw
        );
        // Categories
        assert!(raw.contains("Fast"), "Missing Fast category in:\n{}", raw);
        assert!(
            raw.contains("Reasoning"),
            "Missing Reasoning category in:\n{}",
            raw
        );
    }

    #[test]
    fn test_render_model_picker_with_filter() {
        let models = vec![
            (
                "deepseek-ai/DeepSeek-V4-Flash-0731".to_string(),
                "DeepSeek V4 Flash".to_string(),
            ),
            (
                "MiniMaxAI/MiniMax-M2.7".to_string(),
                "MiniMax M2.7".to_string(),
            ),
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
        assert!(
            raw.contains("Commands"),
            "Missing Commands header in:\n{}",
            raw
        );
        assert!(
            raw.contains("Type to filter"),
            "Missing filter hint in:\n{}",
            raw
        );
        assert!(raw.contains("↑↓ Navigate     Enter Use     Esc Close"));
        assert!(raw.contains("/help") || raw.contains("/model") || raw.contains("/clear"));
    }

    #[test]
    fn test_model_category_labels() {
        assert_eq!(model_category_label("gpt-4o-fast", "Fast GPT"), "Fast");
        assert_eq!(
            model_category_label("deepseek-ai/DeepSeek-V4-Flash-0731", "DeepSeek V4 Flash"),
            "Fast"
        );
        assert_eq!(
            model_category_label("moonshotai/Kimi-K2.6", "Kimi K2.6"),
            "Reasoning"
        );
        assert_eq!(
            model_category_label("MiniMaxAI/MiniMax-M2.7", "MiniMax M2.7"),
            "Reasoning"
        );
        assert_eq!(
            model_category_label("qwen/qwen-coder-32b", "Qwen Coder"),
            "Coding"
        );
        assert_eq!(
            model_category_label("custom-model", "Custom Model"),
            "Model"
        );
    }

    #[test]
    fn test_render_effort_picker_menu_layout() {
        let prompt = Prompt::new()
            .with_model("deepseek-ai/DeepSeek-V4-Flash-0731")
            .with_pending_model_id("deepseek-ai/DeepSeek-V4-Flash-0731")
            .with_effort_picker_active(true)
            .with_effort_selection(0);

        let mut buf = Vec::new();
        let buffer: Vec<char> = "/model deepseek-ai/DeepSeek-V4-Flash-0731 "
            .chars()
            .collect();
        let mut last_lines = 0;
        let mut last_cursor = 0;

        prompt
            .render_to(
                &mut buf,
                &buffer,
                buffer.len(),
                &mut last_lines,
                &mut last_cursor,
            )
            .expect("render_to effort picker failed");

        let raw = String::from_utf8_lossy(&buf);
        // Dividers
        assert!(
            raw.contains("\x1b[38;5;240m"),
            "Missing divider color in:\n{}",
            raw
        );
        // 5 options
        for opt in EFFORT_OPTIONS {
            assert!(
                raw.contains(opt),
                "Missing effort option {} in:\n{}",
                opt,
                raw
            );
        }
        // Selected item 0 (default) bold white
        assert!(
            raw.contains("\x1b[1;37mdefault\x1b[0m"),
            "Selected item not bold white in:\n{}",
            raw
        );
        // Unselected item (xhigh) dim
        assert!(
            raw.contains("\x1b[2;37mxhigh\x1b[0m"),
            "Unselected item not dim in:\n{}",
            raw
        );
        // Status line for default effort
        assert!(
            raw.contains("auto · DeepSeek V4 Flash"),
            "Missing status line in:\n{}",
            raw
        );
    }

    #[test]
    fn test_render_effort_picker_menu_with_effort_selected() {
        let prompt = Prompt::new()
            .with_pending_model_id("moonshotai/Kimi-K2.6")
            .with_effort_picker_active(true)
            .with_effort_selection(1); // xhigh

        let mut buf = Vec::new();
        let buffer: Vec<char> = "/model moonshotai/Kimi-K2.6 ".chars().collect();
        let mut last_lines = 0;
        let mut last_cursor = 0;

        prompt
            .render_to(
                &mut buf,
                &buffer,
                buffer.len(),
                &mut last_lines,
                &mut last_cursor,
            )
            .expect("render_to effort picker failed");

        let raw = String::from_utf8_lossy(&buf);
        // Selected item 1 (xhigh) bold white
        assert!(
            raw.contains("\x1b[1;37mxhigh\x1b[0m"),
            "Selected xhigh not bold white in:\n{}",
            raw
        );
        // Unselected default dim
        assert!(
            raw.contains("\x1b[2;37mdefault\x1b[0m"),
            "Unselected default not dim in:\n{}",
            raw
        );
        // Status line dynamically shows effort
        assert!(
            raw.contains("auto · Kimi K2.6 · xhigh"),
            "Missing dynamic status line in:\n{}",
            raw
        );
    }

    #[test]
    fn test_status_line_with_selected_effort_when_not_in_picker() {
        let prompt = Prompt::new()
            .with_model("MiniMaxAI/MiniMax-M2.7")
            .with_selected_effort(Some("high".to_string()));

        let mut buf = Vec::new();
        let buffer: Vec<char> = Vec::new();
        let mut last_lines = 0;
        let mut last_cursor = 0;

        prompt
            .render_to(&mut buf, &buffer, 0, &mut last_lines, &mut last_cursor)
            .expect("render_to status line with effort failed");

        let raw = String::from_utf8_lossy(&buf);
        assert!(
            raw.contains("auto · MiniMax M2.7 · high"),
            "Status line missing effort in:\n{}",
            raw
        );
    }

    #[test]
    fn test_effort_picker_handle_event_navigation_and_submit() {
        use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

        let models = vec![(
            "deepseek-ai/DeepSeek-V4-Flash-0731".to_string(),
            "DeepSeek V4 Flash".to_string(),
        )];
        let mut prompt = Prompt::new()
            .with_models(models)
            .with_model_picker_active(true);

        // 1. Enter on model picker -> enters effort picker
        let res = prompt
            .handle_event(Event::Key(KeyEvent::new(
                KeyCode::Enter,
                KeyModifiers::NONE,
            )))
            .unwrap();
        assert_eq!(res, None);
        assert!(!prompt.model_picker_active());
        assert!(prompt.effort_picker_active());
        assert_eq!(
            prompt.pending_model_id(),
            "deepseek-ai/DeepSeek-V4-Flash-0731"
        );
        assert_eq!(prompt.effort_selection(), 0);
        let buf_str: String = prompt.buffer.iter().collect();
        assert_eq!(buf_str, "/model deepseek-ai/DeepSeek-V4-Flash-0731 ");

        // 2. Down -> selects xhigh (idx 1)
        prompt
            .handle_event(Event::Key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE)))
            .unwrap();
        assert_eq!(prompt.effort_selection(), 1);

        // 3. Down -> selects high (idx 2)
        prompt
            .handle_event(Event::Key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE)))
            .unwrap();
        assert_eq!(prompt.effort_selection(), 2);

        // 4. Up -> selects xhigh (idx 1)
        prompt
            .handle_event(Event::Key(KeyEvent::new(KeyCode::Up, KeyModifiers::NONE)))
            .unwrap();
        assert_eq!(prompt.effort_selection(), 1);

        // 5. Enter -> submits command with effort
        let res = prompt
            .handle_event(Event::Key(KeyEvent::new(
                KeyCode::Enter,
                KeyModifiers::NONE,
            )))
            .unwrap();
        assert_eq!(
            res,
            Some(PromptResult::Submit(
                "/model deepseek-ai/DeepSeek-V4-Flash-0731 xhigh".to_string()
            ))
        );
        assert!(!prompt.effort_picker_active());
        assert_eq!(prompt.selected_effort(), Some("xhigh"));
    }

    #[test]
    fn test_effort_picker_handle_event_default_effort_submit() {
        use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

        let mut prompt = Prompt::new()
            .with_pending_model_id("gpt-4o")
            .with_effort_picker_active(true)
            .with_effort_selection(0);

        let res = prompt
            .handle_event(Event::Key(KeyEvent::new(
                KeyCode::Enter,
                KeyModifiers::NONE,
            )))
            .unwrap();
        assert_eq!(res, Some(PromptResult::Submit("/model gpt-4o".to_string())));
        assert!(!prompt.effort_picker_active());
        assert_eq!(prompt.selected_effort(), None);
    }

    #[test]
    fn test_effort_picker_handle_event_esc() {
        use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

        let mut prompt = Prompt::new()
            .with_pending_model_id("gpt-4o")
            .with_effort_picker_active(true)
            .with_effort_selection(2);
        prompt.buffer = "/model gpt-4o ".chars().collect();
        prompt.cursor_pos = prompt.buffer.len();

        let res = prompt
            .handle_event(Event::Key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)))
            .unwrap();
        assert_eq!(res, None);
        assert!(!prompt.effort_picker_active());
        assert_eq!(prompt.effort_selection(), 0);
        assert!(prompt.pending_model_id().is_empty());
        assert!(prompt.buffer.is_empty());
    }

    #[test]
    fn test_queued_count_lifecycle() {
        let mut prompt = Prompt::new();
        assert_eq!(prompt.queued_count(), 0);
        assert_eq!(prompt.queued_count, 0);

        prompt.set_queued_count(3);
        assert_eq!(prompt.queued_count(), 3);
        assert_eq!(prompt.queued_count, 3);

        let prompt2 = Prompt::new().with_queued_count(5);
        assert_eq!(prompt2.queued_count(), 5);

        prompt.reset_input();
        assert_eq!(prompt.queued_count(), 0);
    }

    #[test]
    fn test_render_single_queued_message_banner() {
        let prompt = Prompt::new().with_model("grok-4.6").with_queued_count(1);

        let mut buf = Vec::new();
        let buffer: Vec<char> = Vec::new();
        let mut last_lines = 0;
        let mut last_cursor = 0;

        prompt
            .render_to(&mut buf, &buffer, 0, &mut last_lines, &mut last_cursor)
            .expect("render_to single queue banner failed");

        let raw = String::from_utf8_lossy(&buf);
        assert!(
            raw.contains("1 queued message · ↑ to edit"),
            "Missing single queue banner in:\n{}",
            raw
        );
        assert!(
            raw.contains("queued 1 · enter queue · auto · grok-4.6"),
            "Missing queued status line in:\n{}",
            raw
        );
        assert_eq!(
            last_cursor, 2,
            "last_cursor_row should be 2 (0 running + 2 queue banner + 0 target_row)"
        );
    }

    #[test]
    fn test_render_multiple_queued_messages_banner_and_running_status() {
        let mut prompt = Prompt::new()
            .with_model("xai/grok-4.6")
            .with_queued_count(2);
        prompt.set_running_status(Some("Thinking (3s) (↑1 ↓0)".to_string()));

        let mut buf = Vec::new();
        let buffer: Vec<char> = Vec::new();
        let mut last_lines = 0;
        let mut last_cursor = 0;

        prompt
            .render_to(&mut buf, &buffer, 0, &mut last_lines, &mut last_cursor)
            .expect("render_to multi queue banner + running status failed");

        let raw = String::from_utf8_lossy(&buf);
        assert!(
            raw.contains("Thinking (3s) (↑1 ↓0)"),
            "Missing thinking status in:\n{}",
            raw
        );
        assert!(
            raw.contains("2 queued messages · ↑ to edit"),
            "Missing plural queue banner in:\n{}",
            raw
        );
        assert!(
            raw.contains("queued 2 · enter queue · auto · grok-4.6"),
            "Missing queued 2 status line in:\n{}",
            raw
        );
        assert_eq!(
            last_cursor, 4,
            "last_cursor_row should be 4 (2 running + 2 queue banner + 0 target_row)"
        );
    }

    #[test]
    fn test_render_running_status_without_queue() {
        let mut prompt = Prompt::new().with_model("xai/grok-4.6");
        prompt.set_running_status(Some("Thinking (1s)".to_string()));

        let mut buf = Vec::new();
        let buffer: Vec<char> = Vec::new();
        let mut last_lines = 0;
        let mut last_cursor = 0;

        prompt
            .render_to(&mut buf, &buffer, 0, &mut last_lines, &mut last_cursor)
            .expect("render_to running status failed");

        let raw = String::from_utf8_lossy(&buf);
        assert!(
            raw.contains("Thinking (1s)"),
            "Missing thinking status in:\n{}",
            raw
        );
        assert!(
            !raw.contains("queued message"),
            "Should not have queue banner"
        );
        assert!(
            raw.contains("enter queue · auto · grok-4.6"),
            "Missing enter queue status in:\n{}",
            raw
        );
        assert_eq!(
            last_cursor, 2,
            "last_cursor_row should be 2 (2 running + 0 queue banner + 0 target_row)"
        );
    }

    #[test]
    fn test_render_long_running_status_truncated_and_cleared() {
        let mut prompt = Prompt::new().with_model("xai/grok-4.6");
        let long_status = "Running cd /Users/aungmyatmoe/Workshop && for d in react-js-project-very-long-path-name-exceeding-columns; do echo $d; done (22s) (↑100 ↓200)";
        prompt.set_running_status(Some(long_status.to_string()));

        let mut buf = Vec::new();
        let buffer: Vec<char> = Vec::new();
        let mut last_lines = 0;
        let mut last_cursor = 0;

        prompt
            .render_to(&mut buf, &buffer, 0, &mut last_lines, &mut last_cursor)
            .expect("render_to long running status failed");

        let raw = String::from_utf8_lossy(&buf);
        assert!(
            raw.contains("\x1b[2K  \x1b[2;37m"),
            "Missing line clear sequence in:\n{}",
            raw
        );
        assert!(
            raw.contains("…\x1b[0m"),
            "Long status should be truncated with ellipsis in:\n{}",
            raw
        );
        assert_eq!(
            last_cursor, 2,
            "last_cursor_row should still be 2 (single unwrapped line + 1 blank line)"
        );
    }

    #[test]
    fn test_render_idle_prompt_status_line() {
        let prompt = Prompt::new().with_model("xai/grok-4.6");

        let mut buf = Vec::new();
        let buffer: Vec<char> = Vec::new();
        let mut last_lines = 0;
        let mut last_cursor = 0;

        prompt
            .render_to(&mut buf, &buffer, 0, &mut last_lines, &mut last_cursor)
            .expect("render_to idle failed");

        let raw = String::from_utf8_lossy(&buf);
        assert!(
            !raw.contains("enter queue"),
            "Idle status should not contain enter queue"
        );
        assert!(
            raw.contains("auto · grok-4.6"),
            "Missing auto status in:\n{}",
            raw
        );
        assert_eq!(
            last_cursor, 0,
            "last_cursor_row should be 0 (0 running + 0 queue banner + 0 target_row)"
        );
    }

    #[test]
    fn test_running_state_lifecycle_and_render() {
        let mut prompt = Prompt::new().with_model("xai/grok-4.6");
        assert!(!prompt.is_running());

        prompt.set_running(true);
        assert!(prompt.is_running());

        let mut buf = Vec::new();
        let buffer: Vec<char> = Vec::new();
        let mut last_lines = 0;
        let mut last_cursor = 0;

        prompt
            .render_to(&mut buf, &buffer, 0, &mut last_lines, &mut last_cursor)
            .expect("render_to running flag status failed");

        let raw = String::from_utf8_lossy(&buf);
        assert!(
            raw.contains("enter queue · auto · grok-4.6"),
            "Missing enter queue status in:\n{}",
            raw
        );
        assert!(
            !raw.contains("queued message"),
            "Should not have queue banner"
        );

        prompt.reset_input();
        assert!(!prompt.is_running());
    }

    #[test]
    fn test_submit_while_running_does_not_print_to_scrollback() {
        use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

        let mut prompt = Prompt::new().with_running(true);
        assert!(prompt.is_running());
        prompt.buffer = "queued question".chars().collect();
        prompt.cursor_pos = prompt.buffer.len();

        let res = prompt
            .handle_event(Event::Key(KeyEvent::new(
                KeyCode::Enter,
                KeyModifiers::NONE,
            )))
            .unwrap();
        assert_eq!(
            res,
            Some(PromptResult::Submit("queued question".to_string()))
        );
        assert!(prompt.buffer.is_empty());
        assert_eq!(prompt.cursor_pos, 0);
    }

    #[test]
    fn test_reset_render_state() {
        let mut prompt = Prompt::new();
        prompt.last_rendered_lines = 10;
        prompt.last_cursor_row = 4;
        prompt.reset_render_state();
        assert_eq!(prompt.last_rendered_lines, 0);
        assert_eq!(prompt.last_cursor_row, 0);
    }

    #[test]
    fn test_clear_frame_noop_when_not_rendered() {
        let mut prompt = Prompt::new();
        prompt.reset_render_state();
        assert_eq!(prompt.last_rendered_lines, 0);
        assert_eq!(prompt.last_cursor_row, 0);
        assert!(prompt.clear_frame().is_ok());
        assert_eq!(prompt.last_rendered_lines, 0);
        assert_eq!(prompt.last_cursor_row, 0);
    }

    #[test]
    fn test_render_to_empty_buffer_renders_full_box_and_placeholder() {
        let prompt = Prompt::new().with_model("deepseek-ai/DeepSeek-V4-Flash-0731");
        let mut buf = Vec::new();
        let buffer: Vec<char> = Vec::new();
        let mut last_lines = 0;
        let mut last_cursor = 0;

        prompt
            .render_to(&mut buf, &buffer, 0, &mut last_lines, &mut last_cursor)
            .expect("render_to should succeed");

        let raw = String::from_utf8_lossy(&buf);

        // 1. Input line has rail ┃
        assert!(raw.contains('┃'), "Must contain rail symbol '┃':\n{}", raw);
        // 3. Status footer is present
        assert!(
            raw.contains("auto · DeepSeek V4 Flash"),
            "Status footer must be rendered:\n{}",
            raw
        );
        // 4. Cursor tracking: 0 running lines, target_row 0 => last_cursor = 0
        assert_eq!(last_cursor, 0);
        // 5. Total lines rendered: 1 input + 1 spacer + 1 status line = 3
        assert_eq!(last_lines, 3);
    }

    #[test]
    fn test_render_to_does_not_move_up_when_last_rendered_lines_is_zero() {
        let prompt = Prompt::new().with_model("MiniMaxAI/MiniMax-M2.7");
        let mut buf = Vec::new();
        let buffer: Vec<char> = "hello".chars().collect();
        let mut last_lines = 0;
        let mut last_cursor = 10; // Stale cursor row from before terminal scroll

        prompt
            .render_to(&mut buf, &buffer, 5, &mut last_lines, &mut last_cursor)
            .expect("render_to should succeed");

        let raw = String::from_utf8_lossy(&buf);
        // Must NOT contain MoveUp escape sequence before Clear
        assert!(
            !raw.contains("\x1b[10A"),
            "Must not attempt MoveUp when last_rendered_lines is 0:\n{:?}",
            raw
        );
    }

    #[test]
    fn test_render_to_multiline_input_rails() {
        let prompt = Prompt::new().with_model("xai/grok-4.6");
        let mut buf = Vec::new();
        let text = "line 1\nline 2\nline 3";
        let buffer: Vec<char> = text.chars().collect();
        let mut last_lines = 0;
        let mut last_cursor = 0;

        prompt
            .render_to(
                &mut buf,
                &buffer,
                text.len(),
                &mut last_lines,
                &mut last_cursor,
            )
            .expect("render_to multiline should succeed");

        let raw = String::from_utf8_lossy(&buf);
        assert!(raw.contains("line 1"));
        assert!(raw.contains("line 2"));
        assert!(raw.contains("line 3"));
        assert!(raw.contains('┃'));
        assert!(raw.contains("auto · grok-4.6"));
    }

    #[test]
    fn test_clear_frame_guards_against_out_of_bounds_cursor_row() {
        let mut prompt = Prompt::new();
        prompt.last_rendered_lines = 100;
        prompt.last_cursor_row = 75;
        assert!(prompt.clear_frame().is_ok());
        assert_eq!(prompt.last_rendered_lines, 0);
        assert_eq!(prompt.last_cursor_row, 0);
    }

    #[test]
    fn test_render_to_does_not_move_up_when_last_cursor_exceeds_threshold() {
        let prompt = Prompt::new().with_model("MiniMaxAI/MiniMax-M2.7");
        let mut buf = Vec::new();
        let buffer: Vec<char> = "hello".chars().collect();
        let mut last_lines = 60;
        let mut last_cursor = 55; // Corrupted / stale cursor row beyond 50

        prompt
            .render_to(&mut buf, &buffer, 5, &mut last_lines, &mut last_cursor)
            .expect("render_to should succeed");

        let raw = String::from_utf8_lossy(&buf);
        assert!(
            !raw.contains("\x1b[50A") && !raw.contains("\x1b[55A"),
            "Must not attempt MoveUp when last_cursor_row > 50:\n{:?}",
            raw
        );
    }

    #[test]
    fn test_set_running_false_resets_render_state() {
        let mut prompt = Prompt::new();
        prompt.set_running(true);
        prompt.last_rendered_lines = 10;
        prompt.last_cursor_row = 5;
        prompt.set_running(false);
        assert_eq!(prompt.last_rendered_lines, 0);
        assert_eq!(prompt.last_cursor_row, 0);
    }

    #[test]
    fn test_clear_frame_when_last_rendered_lines_zero_even_with_cursor_row() {
        let mut prompt = Prompt::new();
        prompt.last_rendered_lines = 0;
        prompt.last_cursor_row = 15;
        assert!(prompt.clear_frame().is_ok());
        assert_eq!(prompt.last_rendered_lines, 0);
        assert_eq!(prompt.last_cursor_row, 0);
    }

    #[test]
    fn test_render_to_status_line_ends_with_newline() {
        let prompt = Prompt::new().with_model("xai/grok-4.6");
        let mut buf = Vec::new();
        let buffer: Vec<char> = Vec::new();
        let mut last_lines = 0;
        let mut last_cursor = 0;

        prompt
            .render_to(&mut buf, &buffer, 0, &mut last_lines, &mut last_cursor)
            .expect("render_to failed");

        let raw = String::from_utf8_lossy(&buf);
        assert!(
            raw.contains("\x1b[2;37mauto · grok-4.6\x1b[0m\r\n"),
            "Status line must end with \\r\\n in:\n{}",
            raw
        );
        assert!(
            raw.contains("\x1b[3A"),
            "Cursor must be moved up 3 lines to active input line in:\n{}",
            raw
        );
    }
}
