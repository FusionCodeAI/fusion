use crossterm::{
    cursor,
    event::{self, Event, KeyCode, KeyEventKind, KeyModifiers},
    execute,
    terminal::{self, ClearType},
};
use std::borrow::Cow;
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

/// Position and query extracted from an `@file` trigger in the buffer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AtFileTrigger {
    /// Index in `buffer` where `@` starts.
    pub at_index: usize,
    /// The query typed after `@` up to the cursor.
    pub query: String,
}

/// Represents an image attached to the pending prompt.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingImageAttachment {
    pub index: usize,
    pub path: std::path::PathBuf,
    pub width: u32,
    pub height: u32,
    pub tag: String,
}

/// Represents a large block of pasted text collapsed into an inline placeholder.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingPastedText {
    pub index: usize,
    pub line_count: usize,
    pub char_count: usize,
    pub content: String,
    pub tag: String,
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
    /// Cached workspace files for @file autocomplete.
    pub(crate) file_cache: Vec<String>,
    /// Cache timestamp for file list invalidation.
    pub(crate) file_cache_time: Option<std::time::Instant>,
    /// Highlighted index inside the @file autocomplete dialog.
    pub at_file_selection: usize,
    /// Whether the @file autocomplete dropdown was dismissed via Esc.
    pub at_file_dismissed: bool,
    /// Attached images pending turn submission.
    pub pending_images: Vec<PendingImageAttachment>,
    /// Attached large text pastes pending turn submission.
    pub pending_pastes: Vec<PendingPastedText>,
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
            file_cache: Vec::new(),
            file_cache_time: None,
            at_file_selection: 0,
            at_file_dismissed: false,
            pending_images: Vec::new(),
            pending_pastes: Vec::new(),
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

    /// like `[Image #1: 1280x720]` at the current cursor position.
    pub fn attach_image(&mut self, path: std::path::PathBuf, width: u32, height: u32) {
        let index = self.pending_images.len() + 1;
        let tag = crate::ui::clipboard_image::format_image_placeholder(index, width, height);
        for c in tag.chars() {
            self.buffer.insert(self.cursor_pos, c);
            self.cursor_pos += 1;
        }
        self.pending_images.push(PendingImageAttachment {
            index,
            path,
            width,
            height,
            tag,
        });
    }

    /// Reconcile attached images against current buffer text. If the user deleted the
    /// `[Image #N: ...]` placeholder tag with Backspace, the image is automatically detached.
    pub fn reconcile_attached_images(&self) -> Vec<PendingImageAttachment> {
        let current_text: String = self.buffer.iter().collect();
        self.pending_images
            .iter()
            .filter(|img| {
                current_text.contains(&img.tag)
                    || current_text.contains(&format!("[Image #{},", img.index))
                    || current_text.contains(&format!("[Image #{}:", img.index))
                    || current_text.contains(&format!("[Image #{}", img.index))
            })
            .cloned()
            .collect()
    }

    /// Paste text into the prompt buffer, collapsing multi-line blocks
    /// (>= 4 lines) into a clean placeholder like `[Pasted text #1, 28 lines]`.
    pub fn paste_text(&mut self, text: &str) {
        let normalized = text.replace("\r\n", "\n").replace('\r', "\n");
        let line_count = normalized.lines().count();

        if line_count >= 4 || (line_count >= 3 && normalized.len() > 180) {
            let index = self.pending_pastes.len() + 1;
            let tag = format!("[Pasted text #{}, {} lines]", index, line_count);
            for c in tag.chars() {
                self.buffer.insert(self.cursor_pos, c);
                self.cursor_pos += 1;
            }
            self.pending_pastes.push(PendingPastedText {
                index,
                line_count,
                char_count: normalized.chars().count(),
                content: normalized,
                tag,
            });
        } else {
            for c in normalized.chars() {
                self.buffer.insert(self.cursor_pos, c);
                self.cursor_pos += 1;
            }
        }
    }

    /// Reconciles attached pasted text against current buffer content.
    pub fn reconcile_attached_pastes(&self) -> Vec<PendingPastedText> {
        let current_text: String = self.buffer.iter().collect();
        self.pending_pastes
            .iter()
            .filter(|p| {
                current_text.contains(&p.tag)
                    || current_text.contains(&format!("[Pasted text #{},", p.index))
                    || current_text.contains(&format!("[Pasted text #{}", p.index))
                    || current_text.contains(&format!("[Pasted text {}", p.index))
            })
            .cloned()
            .collect()
    }

    /// Handle paste action (e.g. from Ctrl+V or explicit paste key):
    /// 1. First check if system clipboard contains an image (screenshot).
    /// 2. If no image, read clipboard text and insert it (or check if text is an image path).
    /// 3. If clipboard text is unavailable, fall back to internal kill ring.
    pub fn handle_paste_action(&mut self) -> std::io::Result<()> {
        self.key_handler
            .snapshot_undo(&self.buffer, self.cursor_pos);
        // 1. Check for image in system clipboard
        if let Some(pasted) = crate::ui::clipboard_image::read_clipboard_image() {
            self.attach_image(pasted.path, pasted.width, pasted.height);
            return Ok(());
        }

        // 2. Check for text in system clipboard
        #[cfg(not(target_arch = "wasm32"))]
        {
            if let Ok(mut clipboard) = arboard::Clipboard::new() {
                if let Ok(text) = clipboard.get_text() {
                    if !text.is_empty() {
                        let trimmed = text.trim();
                        let path_cand = std::path::Path::new(trimmed);
                        let ext = path_cand
                            .extension()
                            .and_then(|e| e.to_str())
                            .unwrap_or("")
                            .to_lowercase();
                        if matches!(
                            ext.as_str(),
                            "png" | "jpg" | "jpeg" | "webp" | "gif" | "bmp"
                        ) && path_cand.exists()
                        {
                            if let Ok(img) =
                                crate::ui::clipboard_image::load_and_cache_image_file(path_cand)
                            {
                                self.attach_image(img.path, img.width, img.height);
                                return Ok(());
                            }
                        }

                        self.paste_text(&text);
                        return Ok(());
                    }
                }
            }
        }

        // 3. Fallback to kill_ring
        if let Some(text) = self.key_handler.kill_ring().last() {
            let text = text.clone();
            self.paste_text(&text);
        }

        Ok(())
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

    /// Set the selected index for the @file autocomplete dialog.
    pub fn with_at_file_selection(mut self, sel: usize) -> Self {
        self.at_file_selection = sel;
        self
    }

    /// Set the selected index for the @file autocomplete dialog.
    pub fn set_at_file_selection(&mut self, sel: usize) {
        self.at_file_selection = sel;
    }

    /// Get the selected index for the @file autocomplete dialog.
    pub fn at_file_selection(&self) -> usize {
        self.at_file_selection
    }

    /// Set a custom file cache (e.g. for testing).
    pub fn set_file_cache(&mut self, files: Vec<String>) {
        self.file_cache = files;
        self.file_cache_time =
            Some(std::time::Instant::now() + std::time::Duration::from_secs(3600));
    }

    /// Read the currently cached files, if any.
    pub fn file_cache(&self) -> Option<&[String]> {
        if self.file_cache.is_empty() {
            None
        } else {
            Some(&self.file_cache)
        }
    }

    /// Builder method to pre-populate file cache.
    pub fn with_file_cache(mut self, files: Vec<String>) -> Self {
        self.set_file_cache(files);
        self
    }

    /// Return the current prompt buffer as a String.
    pub fn buffer_text(&self) -> String {
        self.buffer.iter().collect()
    }

    /// Scan workspace files using `fusion_walker`.
    pub fn scan_workspace_files() -> Vec<String> {
        let cwd = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
        if let Ok(entries) = fusion_walker::WalkRequest::new(&cwd)
            .hidden(false)
            .gitignore(true)
            .skip_git(true)
            .skip_node_modules(true)
            .cache(true)
            .collect_files()
        {
            let mut paths: Vec<String> = entries.into_iter().map(|e| e.path).collect();
            paths.sort();
            paths
        } else {
            Vec::new()
        }
    }

    /// Extract active @file trigger at the current cursor position in buffer.
    pub fn at_file_trigger(&self) -> Option<AtFileTrigger> {
        extract_at_trigger(&self.buffer, self.cursor_pos)
    }

    /// Get matching files for the active @file trigger.
    pub fn at_file_matches(&mut self) -> Vec<String> {
        let trigger = match self.at_file_trigger() {
            Some(t) => t,
            None => return Vec::new(),
        };
        if self.file_cache.is_empty()
            || self
                .file_cache_time
                .map_or(true, |t| t.elapsed() > std::time::Duration::from_secs(5))
        {
            self.file_cache = Self::scan_workspace_files();
            self.file_cache_time = Some(std::time::Instant::now());
        }
        fuzzy_match_files(&trigger.query, &self.file_cache)
    }

    /// Replace active @query with given path.
    pub fn apply_at_file_completion(&mut self, path: &str) -> bool {
        let trigger = match self.at_file_trigger() {
            Some(t) => t,
            None => return false,
        };
        let at_idx = trigger.at_index;
        // Find end of token: scan forward while characters are valid query chars
        let mut end = at_idx + 1;
        while end < self.buffer.len() && is_query_char(self.buffer[end]) {
            end += 1;
        }
        // Replace buffer[at_idx..end] with path
        let path_chars: Vec<char> = path.chars().collect();
        let path_len = path_chars.len();
        self.buffer.splice(at_idx..end, path_chars);
        self.cursor_pos = at_idx + path_len;
        self.at_file_selection = 0;
        self.at_file_dismissed = false;
        true
    }

    /// Complete with the currently selected matching file.
    pub fn select_at_file_completion(&mut self) -> bool {
        let matches = self.at_file_matches();
        if matches.is_empty() {
            return false;
        }
        let sel = self.at_file_selection.min(matches.len().saturating_sub(1));
        let path = matches[sel].clone();
        self.apply_at_file_completion(&path)
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
        self.at_file_selection = 0;
        self.at_file_dismissed = false;
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
                    if !self.at_file_dismissed && !self.at_file_matches().is_empty() {
                        self.at_file_dismissed = true;
                        self.render_current()?;
                        return Ok(None);
                    }
                    if !self.buffer.is_empty() {
                        self.buffer.clear();
                        self.cursor_pos = 0;
                        self.render_current()?;
                        return Ok(None);
                    }
                    return Ok(None);
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

                // @file autocomplete dialog navigation and insertion
                if !self.at_file_dismissed && !first_line.starts_with('/') {
                    let at_matches = self.at_file_matches();
                    if !at_matches.is_empty() {
                        match key.code {
                            KeyCode::Down => {
                                self.at_file_selection =
                                    (self.at_file_selection + 1) % at_matches.len();
                                self.render_current()?;
                                return Ok(None);
                            }
                            KeyCode::BackTab | KeyCode::Up => {
                                self.at_file_selection = if self.at_file_selection == 0 {
                                    at_matches.len() - 1
                                } else {
                                    self.at_file_selection - 1
                                };
                                self.render_current()?;
                                return Ok(None);
                            }
                            KeyCode::Tab | KeyCode::Enter => {
                                if self.select_at_file_completion() {
                                    self.render_current()?;
                                    return Ok(None);
                                }
                            }
                            KeyCode::Esc => {
                                self.at_file_dismissed = true;
                                self.render_current()?;
                                return Ok(None);
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
                        self.at_file_dismissed = false;
                        self.render_current()?;
                        Ok(None)
                    }
                    KeyResult::Paste => {
                        self.handle_paste_action()?;
                        self.at_file_dismissed = false;
                        self.render_current()?;
                        Ok(None)
                    }
                    KeyResult::Reload => {
                        if crate::agent::updater::has_staged_update() {
                            self.clear_frame()?;
                            println!("\x1b[1;36mReloading Fusion with staged update...\x1b[0m\r\n");
                            crate::agent::updater::reload_process();
                        }
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
                            let (term_cols, _) = terminal::size()
                                .map(|(w, h)| (w as usize, h as usize))
                                .unwrap_or((80, 24));
                            let buf_chars: Vec<char> = text.chars().collect();
                            let (v_lines, _, _) = wrap_prompt_lines(&buf_chars, 0, term_cols, 0);
                            let mut out = stdout();
                            for line in &v_lines {
                                let formatted =
                                    format_prompt_line_with_colored_placeholders(&line.text);
                                let _ = write!(out, "\x1b[1m┃ {}\x1b[0m\r\n", formatted);
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
                if text.trim().is_empty() {
                    if let Some(pasted) = crate::ui::clipboard_image::read_clipboard_image() {
                        self.attach_image(pasted.path, pasted.width, pasted.height);
                        self.render_current()?;
                        return Ok(None);
                    }
                }
                let trimmed = text.trim();
                let path_cand = std::path::Path::new(trimmed);
                let ext = path_cand
                    .extension()
                    .and_then(|e| e.to_str())
                    .unwrap_or("")
                    .to_lowercase();
                if matches!(
                    ext.as_str(),
                    "png" | "jpg" | "jpeg" | "webp" | "gif" | "bmp"
                ) && path_cand.exists()
                {
                    if let Ok(img) =
                        crate::ui::clipboard_image::load_and_cache_image_file(path_cand)
                    {
                        self.attach_image(img.path, img.width, img.height);
                        self.render_current()?;
                        return Ok(None);
                    }
                }
                // If system clipboard has an image right now, attach it
                if let Some(pasted) = crate::ui::clipboard_image::read_clipboard_image() {
                    self.attach_image(pasted.path, pasted.width, pasted.height);
                    self.render_current()?;
                    return Ok(None);
                }
                self.paste_text(&text);
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
        let (term_cols, term_rows) = terminal::size()
            .map(|(w, h)| (w as usize, h as usize))
            .unwrap_or((80, 24));
        let max_up = term_rows.saturating_sub(1).min(50);
        let chip_w = self.skill_chip_width();

        let (visual_lines, target_row, target_col) =
            wrap_prompt_lines(buffer, cursor_pos, term_cols, chip_w);
        let lines: Vec<&str> = text.split('\n').collect();
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

        // Check for @file suggestions
        let at_file_matches = if !self.model_picker_active
            && !self.effort_picker_active
            && !self.skill_picker_active
            && !self.at_file_dismissed
            && slash_suggestions.is_empty()
        {
            if let Some(trigger) = extract_at_trigger(buffer, cursor_pos) {
                let files: Cow<[String]> = if self.file_cache.is_empty() {
                    Cow::Owned(Self::scan_workspace_files())
                } else {
                    Cow::Borrowed(&self.file_cache)
                };
                fuzzy_match_files(&trigger.query, &files)
            } else {
                Vec::new()
            }
        } else {
            Vec::new()
        };

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
            let clean_status = status
                .trim_start_matches('\r')
                .trim_start_matches("\x1b[2K");
            let display_status = if max_w > 0 {
                truncate_fit(clean_status, max_w)
            } else {
                clean_status.to_string()
            };
            write!(
                out,
                "\r\x1b[2K\r\n\r\x1b[2K  \x1b[2;37m{}\x1b[0m\r\n\r\n",
                display_status
            )?;
            3
        } else {
            0
        };

        let queue_banner_lines = if self.queued_count > 0 {
            let banner = if self.queued_count == 1 {
                "1 queued message · ↑ to edit".to_string()
            } else {
                format!("{} queued messages · ↑ to edit", self.queued_count)
            };
            write!(out, "\r\x1b[2K  \x1b[2;37m{}\x1b[0m\r\n\r\n", banner)?;
            2
        } else {
            0
        };
        let header_lines = running_lines + queue_banner_lines;
        total_lines += header_lines;
        // 1. Input lines with clean vertical rail symbol (┃ )
        for (v_idx, v_line) in visual_lines.iter().enumerate() {
            let prefix = if v_idx == 0 {
                &self.prompt_symbol
            } else {
                &self.multiline_symbol
            };

            write!(out, "{}", prefix)?;
            if v_idx == 0 {
                // Active skill chip prefix (fx-style): skill name · source
                if let Some((skill_name, source)) = &self.active_skill {
                    write!(
                        out,
                        "\x1b[1;36m{}\x1b[0m \x1b[2;37m· {}\x1b[0m  ",
                        skill_name, source
                    )?;
                }
            }
            if v_idx == 0 && v_line.text.is_empty() && visual_lines.len() == 1 && !self.is_running {
                if let Some(ph) = &self.placeholder {
                    if !ph.is_empty() {
                        write!(out, "\x1b[2;37m{}\x1b[0m", ph)?;
                    }
                }
            } else {
                let formatted = format_prompt_line_with_colored_placeholders(&v_line.text);
                write!(out, "{}", formatted)?;
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
        } else if !at_file_matches.is_empty() {
            let sel = self
                .at_file_selection
                .min(at_file_matches.len().saturating_sub(1));
            let window_start = if sel >= 6 { sel - 5 } else { 0 };
            let visible_items: Vec<_> = at_file_matches
                .iter()
                .enumerate()
                .skip(window_start)
                .take(6)
                .collect();
            let visible_count = visible_items.len();
            let window_end = window_start + visible_count;

            write!(out, "\x1b[38;5;240m{}\x1b[0m\r\n", divider)?;
            total_lines += 1;

            let left = format!("Files {} · Type to filter", at_file_matches.len());
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

            let max_path_w = term_cols.saturating_sub(6);

            for (idx, path) in visible_items {
                let is_selected = idx == sel;
                let display_path = truncate_fit(path, max_path_w);

                if is_selected {
                    write!(out, "\x1b[1;37m📄 {}\x1b[0m\r\n", display_path)?;
                } else {
                    write!(out, "\x1b[2;37m📄 {}\x1b[0m\r\n", display_path)?;
                }
                total_lines += 1;
            }

            write!(out, "\x1b[38;5;240m{}\x1b[0m\r\n", divider)?;
            total_lines += 1;

            write!(
                out,
                "\x1b[2;37m↑↓ Navigate     Tab/Enter Insert     Esc Close\x1b[0m"
            )?;
            total_lines += 1;
            let lines_up = ((visual_lines.len() - 1 - target_row) + visible_count + 5).min(max_up);
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
            let right_text = if crate::agent::updater::has_staged_update() {
                "update ready: ctrl+g to reload"
            } else {
                ""
            };

            if !right_text.is_empty() && term_cols > status_text.len() + right_text.len() + 6 {
                let gap = term_cols.saturating_sub(status_text.len() + right_text.len() + 4);
                write!(
                    out,
                    "  \x1b[2;37m{}\x1b[0m{:gap$}\x1b[2;37m{}\x1b[0m\r\n",
                    status_text, "", right_text
                )?;
            } else {
                write!(out, "  \x1b[2;37m{}\x1b[0m\r\n", status_text)?;
            }
            total_lines += 1;

            // Reposition cursor inside input box on active input row
            let lines_up = ((visual_lines.len() - 1 - target_row) + 3).min(max_up);
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

    /// Highlight any `[Image #...]` placeholder tags with bold cyan color.
    pub fn format_with_colored_placeholders(line: &str) -> String {
        format_prompt_line_with_colored_placeholders(line)
    }
    /// Render a submitted user input prompt to a generic writer.
    pub fn render_submitted_prompt_to<W: std::io::Write>(
        out: &mut W,
        text: &str,
    ) -> std::io::Result<()> {
        let (term_cols, _) = terminal::size()
            .map(|(w, h)| (w as usize, h as usize))
            .unwrap_or((80, 24));
        let buf_chars: Vec<char> = text.chars().collect();
        let (v_lines, _, _) = wrap_prompt_lines(&buf_chars, 0, term_cols, 0);
        for line in &v_lines {
            let formatted = format_prompt_line_with_colored_placeholders(&line.text);
            write!(out, "\x1b[1m┃ {}\x1b[0m\r\n", formatted)?;
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

/// Highlights any `[Image #...]` or `[Pasted text #...]` placeholder tags within a rendered line with bold cyan styling.
pub fn format_prompt_line_with_colored_placeholders(line: &str) -> String {
    if !line.contains("[Image") && !line.contains("[Pasted text") {
        return line.to_string();
    }

    let mut result = String::with_capacity(line.len() + 32);
    let mut remainder = line;

    while !remainder.is_empty() {
        let next_image = remainder.find("[Image");
        let next_paste = remainder.find("[Pasted text");

        let next_match = match (next_image, next_paste) {
            (Some(i), Some(p)) => Some((i.min(p), if i < p { "[Image" } else { "[Pasted text" })),
            (Some(i), None) => Some((i, "[Image")),
            (None, Some(p)) => Some((p, "[Pasted text")),
            (None, None) => None,
        };

        if let Some((start, _prefix)) = next_match {
            result.push_str(&remainder[..start]);
            let after_start = &remainder[start..];
            if let Some(end) = after_start.find(']') {
                let tag = &after_start[..=end];
                // High-contrast bold electric cyan (\x1b[1;36;38;5;39m...\x1b[0m)
                result.push_str("\x1b[1;36;38;5;39m");
                result.push_str(tag);
                result.push_str("\x1b[0m");
                remainder = &after_start[end + 1..];
                continue;
            } else {
                result.push_str(after_start);
                remainder = "";
                break;
            }
        } else {
            result.push_str(remainder);
            break;
        }
    }
    result
}

/// Expands any `[Pasted text #N, ...]` placeholders in a user prompt into their full content.
pub fn expand_pasted_text_placeholders(prompt_text: &str, pastes: &[PendingPastedText]) -> String {
    let mut expanded = prompt_text.to_string();
    for p in pastes {
        if expanded.contains(&p.tag) {
            expanded = expanded.replace(&p.tag, &p.content);
        } else {
            let alt_prefix = format!("[Pasted text #{}", p.index);
            if let Some(start) = expanded.find(&alt_prefix) {
                if let Some(close_rel) = expanded[start..].find(']') {
                    let full_tag = &expanded[start..=start + close_rel];
                    expanded = expanded.replace(full_tag, &p.content);
                }
            }
        }
    }
    expanded
}

/// Check if a character can be part of an `@file` path query.
pub fn is_query_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_' || c == '-' || c == '.' || c == '/' || c == '\\'
}

/// Check if character preceding `@` is valid (start of word/token).
pub fn is_valid_at_prefix(prev: char) -> bool {
    !prev.is_alphanumeric() && prev != '_'
}

/// Extract `@` trigger and query from buffer at cursor position.
pub fn extract_at_trigger(buffer: &[char], cursor_pos: usize) -> Option<AtFileTrigger> {
    if cursor_pos == 0 || buffer.is_empty() {
        return None;
    }
    let effective_cursor = cursor_pos.min(buffer.len());
    let slice = &buffer[..effective_cursor];

    let mut at_idx = None;
    for (i, &c) in slice.iter().enumerate().rev() {
        if c == '@' {
            at_idx = Some(i);
            break;
        } else if !is_query_char(c) {
            return None;
        }
    }

    let at_idx = at_idx?;

    if at_idx > 0 {
        let prev = buffer[at_idx - 1];
        if !is_valid_at_prefix(prev) {
            return None;
        }
    }

    let query: String = slice[at_idx + 1..].iter().collect();
    Some(AtFileTrigger {
        at_index: at_idx,
        query,
    })
}

/// Fuzzy match and rank workspace file paths against a query.
pub fn fuzzy_match_files(query: &str, files: &[String]) -> Vec<String> {
    let q = query.trim().to_lowercase();
    if q.is_empty() {
        return files.iter().take(50).cloned().collect();
    }

    let mut scored: Vec<(i64, &String)> = Vec::new();

    for file in files {
        let path_lower = file.to_lowercase();
        let file_name = file.rsplit(['/', '\\']).next().unwrap_or(file);
        let file_name_lower = file_name.to_lowercase();

        let mut score: Option<i64> = None;

        if file_name_lower == q {
            score = Some(10_000 - file.len() as i64);
        } else if file_name_lower.starts_with(&q) {
            score = Some(8_000 - file.len() as i64);
        } else if path_lower.starts_with(&q) {
            score = Some(7_000 - file.len() as i64);
        } else if file_name_lower.contains(&q) {
            score = Some(5_000 - file.len() as i64);
        } else if path_lower.contains(&q) {
            score = Some(3_000 - file.len() as i64);
        } else {
            // Check fuzzy subsequence match
            let mut q_chars = q.chars().peekable();
            let mut boundary_matches = 0;
            let mut prev_char = '/';
            for c in path_lower.chars() {
                if let Some(&qc) = q_chars.peek() {
                    if c == qc {
                        q_chars.next();
                        if prev_char == '/'
                            || prev_char == '_'
                            || prev_char == '-'
                            || prev_char == '.'
                        {
                            boundary_matches += 1;
                        }
                    }
                }
                prev_char = c;
            }
            if q_chars.peek().is_none() {
                score = Some(1_000 + (boundary_matches * 50) - file.len() as i64);
            }
        }

        if let Some(s) = score {
            scored.push((s, file));
        }
    }

    scored.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.cmp(b.1)));
    scored
        .into_iter()
        .take(50)
        .map(|(_, f)| f.clone())
        .collect()
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
    } else if lower.contains("reason") || lower.contains("minimax") {
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

/// A visual line displayed in the prompt after soft-wrapping at terminal boundaries.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VisualLine {
    pub text: String,
    pub is_logical_start: bool,
    pub char_start: usize,
    pub char_end: usize,
}

/// Breaks the prompt buffer into visual lines wrapped to `term_cols`,
/// and computes the cursor's visual row and visual column.
pub fn wrap_prompt_lines(
    buffer: &[char],
    cursor_pos: usize,
    term_cols: usize,
    chip_w: usize,
) -> (Vec<VisualLine>, usize, usize) {
    if buffer.is_empty() {
        return (
            vec![VisualLine {
                text: String::new(),
                is_logical_start: true,
                char_start: 0,
                char_end: 0,
            }],
            0,
            0,
        );
    }

    let mut visual_lines = Vec::new();
    let mut logical_start = 0;
    let mut cur_char_idx = 0;

    while cur_char_idx <= buffer.len() {
        if cur_char_idx == buffer.len() || buffer[cur_char_idx] == '\n' {
            let logical_slice = &buffer[logical_start..cur_char_idx];
            let is_first_logical = visual_lines.is_empty();

            if logical_slice.is_empty() {
                visual_lines.push(VisualLine {
                    text: String::new(),
                    is_logical_start: true,
                    char_start: logical_start,
                    char_end: cur_char_idx,
                });
            } else {
                let mut seg_start = 0;
                let mut is_first_visual_of_logical = true;

                while seg_start < logical_slice.len() {
                    let max_w = if is_first_logical && is_first_visual_of_logical {
                        term_cols.saturating_sub(2 + chip_w).max(15)
                    } else {
                        term_cols.saturating_sub(2).max(15)
                    };

                    let mut seg_end = seg_start;
                    let mut current_w = 0;
                    let mut last_space_idx = None;
                    let mut last_tag_close_idx = None;

                    while seg_end < logical_slice.len() {
                        let c = logical_slice[seg_end];
                        let c_w = unicode_width::UnicodeWidthChar::width(c).unwrap_or(1);
                        if current_w + c_w > max_w && seg_end > seg_start {
                            break;
                        }
                        if c == ' ' {
                            last_space_idx = Some(seg_end);
                        } else if c == ']' {
                            last_tag_close_idx = Some(seg_end);
                        }
                        current_w += c_w;
                        seg_end += 1;
                    }

                    // Prevent chopping tags ([Image...] or [Pasted text...]) in half
                    let actual_end = if seg_end < logical_slice.len() {
                        let mut tag_break = None;
                        let sub_chars = &logical_slice[seg_start..seg_end];
                        if let Some(open_rel) = sub_chars.iter().rposition(|&c| c == '[') {
                            let open_idx = seg_start + open_rel;
                            let has_close = sub_chars[open_rel..].iter().any(|&c| c == ']');
                            if !has_close {
                                let tag_prefix: String = logical_slice
                                    [open_idx..seg_end.min(open_idx + 8)]
                                    .iter()
                                    .collect();
                                if tag_prefix.starts_with("[Image")
                                    || tag_prefix.starts_with("[Pasted")
                                {
                                    if open_idx > seg_start {
                                        tag_break = Some(open_idx);
                                    }
                                }
                            }
                        }

                        if let Some(tb) = tag_break {
                            tb
                        } else {
                            let mut boundary = None;
                            if let Some(space_idx) = last_space_idx {
                                if space_idx >= seg_start {
                                    boundary = Some(space_idx + 1);
                                }
                            }
                            if let Some(close_idx) = last_tag_close_idx {
                                if close_idx >= seg_start {
                                    let candidate = close_idx + 1;
                                    boundary =
                                        Some(boundary.map_or(candidate, |b| b.max(candidate)));
                                }
                            }
                            boundary.unwrap_or(seg_end)
                        }
                    } else {
                        seg_end
                    };

                    let line_chars = &logical_slice[seg_start..actual_end];
                    let line_str: String = line_chars.iter().collect();

                    visual_lines.push(VisualLine {
                        text: line_str,
                        is_logical_start: is_first_visual_of_logical,
                        char_start: logical_start + seg_start,
                        char_end: logical_start + actual_end,
                    });

                    is_first_visual_of_logical = false;
                    seg_start = actual_end;
                }
            }

            logical_start = cur_char_idx + 1;
        }
        cur_char_idx += 1;
    }

    if visual_lines.is_empty() {
        visual_lines.push(VisualLine {
            text: String::new(),
            is_logical_start: true,
            char_start: 0,
            char_end: 0,
        });
    }
    // Determine visual cursor row & column
    let mut target_row = 0;
    let mut target_col = 0;
    for (v_idx, v_line) in visual_lines.iter().enumerate() {
        if cursor_pos >= v_line.char_start
            && (cursor_pos <= v_line.char_end || v_idx == visual_lines.len() - 1)
        {
            target_row = v_idx;
            let clamped_cur = cursor_pos.min(v_line.char_end);
            let sub_slice: String = buffer[v_line.char_start..clamped_cur].iter().collect();
            target_col = crate::ui::table::visible_width(&sub_slice);
            break;
        }
    }

    (visual_lines, target_row, target_col)
}

/// Calculate visible character width by ignoring ANSI escape codes.
/// Wide (CJK) characters count 2 columns via `unicode-width`.
fn visible_width(s: &str) -> usize {
    crate::ui::table::visible_width(s)
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
            let slice: String = buffer[start..cursor_pos].iter().collect();
            cur_col = crate::ui::table::visible_width(&slice);
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
        let _ = execute!(stdout(), cursor::Show, event::EnableBracketedPaste);
        Ok(Self)
    }
}

impl Drop for RawModeGuard {
    fn drop(&mut self) {
        let _ = execute!(stdout(), event::DisableBracketedPaste, cursor::Show);
        let _ = terminal::disable_raw_mode();
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
            raw.contains("Models 2 · Type to filter"),
            "Missing header in:\n{}",
            raw
        );
        assert!(raw.contains("1-2"), "Missing range indicator in:\n{}", raw);
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
            .with_pending_model_id("MiniMaxAI/MiniMax-M2.7")
            .with_effort_picker_active(true)
            .with_effort_selection(1); // xhigh

        let mut buf = Vec::new();
        let buffer: Vec<char> = "/model MiniMaxAI/MiniMax-M2.7 ".chars().collect();
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
            raw.contains("auto · MiniMax M2.7 · xhigh"),
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
            last_cursor, 5,
            "last_cursor_row should be 5 (3 running + 2 queue banner + 0 target_row)"
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
            last_cursor, 3,
            "last_cursor_row should be 3 (3 running + 0 queue banner + 0 target_row)"
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
            last_cursor, 3,
            "last_cursor_row should be 3 (line above + status line + line below)"
        );
    }

    #[test]
    fn test_burmese_unicode_cursor_column_and_width() {
        // "မင်္ဂလာပါ" has 9 Unicode code points and 7 visual columns on monospace terminal
        // (မ:1, င:1, ်:0, ္:0, ဂ:1, လ:1, ာ:1, ပ:1, ာ:1 = 7 visual cols vs naive char count 9)
        let text = "မင်္ဂလာပါ";
        assert_eq!(crate::ui::table::visible_width(text), 7);

        let buf: Vec<char> = text.chars().collect();
        assert_eq!(buf.len(), 9);

        // Cursor at character position 9 (end of word)
        let (line, col, _) = get_line_info(&buf, 9);
        assert_eq!(line, 0);
        assert_eq!(col, 7, "Visual column must be 7 (not naive char count 9)");
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

    #[test]
    fn test_extract_at_trigger() {
        // Simple @ at start of buffer
        let buf: Vec<char> = "@".chars().collect();
        assert_eq!(
            extract_at_trigger(&buf, 1),
            Some(AtFileTrigger {
                at_index: 0,
                query: String::new()
            })
        );

        // Simple @word at start of buffer
        let buf: Vec<char> = "@main".chars().collect();
        assert_eq!(
            extract_at_trigger(&buf, 5),
            Some(AtFileTrigger {
                at_index: 0,
                query: "main".to_string()
            })
        );

        // Word preceded by space
        let buf: Vec<char> = "hello @main".chars().collect();
        assert_eq!(
            extract_at_trigger(&buf, 11),
            Some(AtFileTrigger {
                at_index: 6,
                query: "main".to_string()
            })
        );

        // Word with path characters
        let buf: Vec<char> = "see @src/ui/prompt.rs".chars().collect();
        assert_eq!(
            extract_at_trigger(&buf, 21),
            Some(AtFileTrigger {
                at_index: 4,
                query: "src/ui/prompt.rs".to_string()
            })
        );

        // Trailing space after word should not trigger
        let buf: Vec<char> = "@main ".chars().collect();
        assert_eq!(extract_at_trigger(&buf, 6), None);

        // Email / identifier should not trigger
        let buf: Vec<char> = "user@example.com".chars().collect();
        assert_eq!(extract_at_trigger(&buf, 16), None);

        // Cursor at 0
        let buf: Vec<char> = "@main".chars().collect();
        assert_eq!(extract_at_trigger(&buf, 0), None);
    }

    #[test]
    fn test_fuzzy_match_files() {
        let files = vec![
            "src/main.rs".to_string(),
            "crates/fusion-shell/src/main.rs".to_string(),
            "src/ui/prompt.rs".to_string(),
            "src/auth/login.rs".to_string(),
            "Cargo.toml".to_string(),
        ];

        // Empty query returns all files
        let all = fuzzy_match_files("", &files);
        assert_eq!(all.len(), 5);

        // Exact / prefix match on filename ranks highest
        let matches = fuzzy_match_files("main", &files);
        assert_eq!(matches[0], "src/main.rs");
        assert_eq!(matches[1], "crates/fusion-shell/src/main.rs");

        // Subsequence match
        let prompt_matches = fuzzy_match_files("prompt", &files);
        assert_eq!(prompt_matches.len(), 1);
        assert_eq!(prompt_matches[0], "src/ui/prompt.rs");

        let auth_matches = fuzzy_match_files("auth", &files);
        assert_eq!(auth_matches.len(), 1);
        assert_eq!(auth_matches[0], "src/auth/login.rs");
    }

    #[test]
    fn test_apply_at_file_completion() {
        let mut prompt = Prompt::new();
        prompt.buffer = "look at @main and test".chars().collect();
        prompt.cursor_pos = 13; // after @main

        assert!(prompt.apply_at_file_completion("src/main.rs"));
        assert_eq!(prompt.buffer_text(), "look at src/main.rs and test");
        assert_eq!(prompt.cursor_pos, 8 + 11); // 8 is start of path, 11 is len
    }

    #[test]
    fn test_render_at_file_dropdown() {
        let mut prompt = Prompt::new();
        prompt.set_file_cache(vec![
            "src/main.rs".to_string(),
            "src/ui/prompt.rs".to_string(),
        ]);
        let buffer: Vec<char> = "@main".chars().collect();
        let mut buf = Vec::new();
        let mut last_lines = 0;
        let mut last_cursor = 0;

        prompt
            .render_to(&mut buf, &buffer, 5, &mut last_lines, &mut last_cursor)
            .expect("render_to failed");

        let rendered = String::from_utf8_lossy(&buf);
        assert!(
            rendered.contains("📄 src/main.rs"),
            "Dropdown must contain document icon and path: {}",
            rendered
        );
        assert!(
            rendered.contains("Tab/Enter Insert"),
            "Dropdown must contain keybinding hint: {}",
            rendered
        );
    }

    #[test]
    fn test_handle_event_at_file_navigation_and_insertion() {
        let mut prompt = Prompt::new();
        prompt.set_file_cache(vec![
            "src/main.rs".to_string(),
            "crates/fusion-shell/src/main.rs".to_string(),
        ]);
        prompt.buffer = "@main".chars().collect();
        prompt.cursor_pos = 5;

        // Down arrow changes selection
        let down_event = Event::Key(crossterm::event::KeyEvent::new(
            KeyCode::Down,
            KeyModifiers::NONE,
        ));
        let res = prompt
            .handle_event(down_event)
            .expect("handle_event failed");
        assert_eq!(res, None);
        assert_eq!(prompt.at_file_selection(), 1);

        // Tab key inserts selected file
        let tab_event = Event::Key(crossterm::event::KeyEvent::new(
            KeyCode::Tab,
            KeyModifiers::NONE,
        ));
        let res = prompt.handle_event(tab_event).expect("handle_event failed");
        assert_eq!(res, None);
        assert_eq!(prompt.buffer_text(), "crates/fusion-shell/src/main.rs");
    }

    #[test]
    fn test_handle_event_at_file_esc_dismissal() {
        let mut prompt = Prompt::new();
        prompt.set_file_cache(vec!["src/main.rs".to_string()]);
        prompt.buffer = "@main".chars().collect();
        prompt.cursor_pos = 5;

        // Esc dismisses the @file dropdown without canceling prompt
        let esc_event = Event::Key(crossterm::event::KeyEvent::new(
            KeyCode::Esc,
            KeyModifiers::NONE,
        ));
        let res = prompt.handle_event(esc_event).expect("handle_event failed");
        assert_eq!(res, None);
        assert!(prompt.at_file_dismissed);
        assert_eq!(prompt.buffer_text(), "@main");

        // Second Esc clears the buffer without canceling or exiting
        let esc_event2 = Event::Key(crossterm::event::KeyEvent::new(
            KeyCode::Esc,
            KeyModifiers::NONE,
        ));
        let res2 = prompt
            .handle_event(esc_event2.clone())
            .expect("handle_event failed");
        assert_eq!(res2, None);
        assert_eq!(prompt.buffer_text(), "");

        // Third Esc on empty buffer is a safe no-op (never quits)
        let res3 = prompt
            .handle_event(esc_event2)
            .expect("handle_event failed");
        assert_eq!(res3, None);
    }

    #[test]
    fn test_esc_never_exits_prompt() {
        let mut prompt = Prompt::new();
        prompt.buffer = "some input text".chars().collect();
        prompt.cursor_pos = prompt.buffer.len();

        let esc = Event::Key(crossterm::event::KeyEvent::new(
            KeyCode::Esc,
            KeyModifiers::NONE,
        ));

        // Press 1: Clears text buffer
        let res1 = prompt.handle_event(esc.clone()).unwrap();
        assert_eq!(res1, None);
        assert!(prompt.buffer.is_empty());

        // Press 2: Empty buffer, remains in prompt
        let res2 = prompt.handle_event(esc.clone()).unwrap();
        assert_eq!(res2, None);

        // Press 3: Still in prompt
        let res3 = prompt.handle_event(esc.clone()).unwrap();
        assert_eq!(res3, None);

        // Ctrl+C produces Cancel
        let ctrl_c = Event::Key(crossterm::event::KeyEvent::new(
            KeyCode::Char('c'),
            KeyModifiers::CONTROL,
        ));
        let res_c = prompt.handle_event(ctrl_c).unwrap();
        assert_eq!(res_c, Some(PromptResult::Cancel));
    }

    #[test]
    fn test_prompt_attach_image() {
        let mut prompt = Prompt::new();
        prompt.buffer = "Look at this ".chars().collect();
        prompt.cursor_pos = prompt.buffer.len();

        let path = std::path::PathBuf::from(".fusion/cache/images/test.png");
        prompt.attach_image(path, 800, 600);

        let text: String = prompt.buffer.iter().collect();
        assert_eq!(text, "Look at this [Image #1, 800x600]");
        assert_eq!(prompt.pending_images.len(), 1);
        assert_eq!(prompt.pending_images[0].width, 800);
        assert_eq!(prompt.pending_images[0].height, 600);
    }

    #[test]
    fn test_prompt_reconcile_attached_images() {
        let mut prompt = Prompt::new();
        let path = std::path::PathBuf::from(".fusion/cache/images/test.png");
        prompt.attach_image(path, 800, 600);

        // If placeholder tag is present, it reconciles
        let remaining = prompt.reconcile_attached_images();
        assert_eq!(remaining.len(), 1);

        // If user deleted the placeholder tag with Backspace, it reconciles to empty
        prompt.buffer.clear();
        let remaining_after_delete = prompt.reconcile_attached_images();
        assert!(remaining_after_delete.is_empty());
    }

    #[test]
    fn test_format_prompt_line_with_colored_placeholders() {
        let line = "Explain [Image #1, 1568x1037] and [Image #2, 800x600] please";
        let formatted = format_prompt_line_with_colored_placeholders(line);
        assert!(formatted.contains("\x1b[1;36;38;5;39m[Image #1, 1568x1037]\x1b[0m"));
        assert!(formatted.contains("\x1b[1;36;38;5;39m[Image #2, 800x600]\x1b[0m"));
        assert!(formatted.ends_with(" please"));

        // Regular line without image tags is unchanged
        let plain = "Hello world";
        assert_eq!(
            format_prompt_line_with_colored_placeholders(plain),
            "Hello world"
        );
    }

    #[test]
    fn test_wrap_prompt_lines_multiline() {
        let text = "[Image 1] hi sucker what will yo do mother fucking kdjfoawejfowa roawjrowjrowe roeq jroejorjawor joewjroew roewjorjw orjowejrjoewroewjorewroewjroewjorweo";
        let buf: Vec<char> = text.chars().collect();
        let term_cols = 80;
        let (visual_lines, target_row, target_col) =
            wrap_prompt_lines(&buf, buf.len(), term_cols, 0);
        assert!(
            visual_lines.len() >= 2,
            "Long line must wrap into at least 2 visual lines"
        );
        assert_eq!(
            target_row,
            visual_lines.len() - 1,
            "Cursor at end must be on the last visual row"
        );
        assert!(target_col > 0, "Cursor col must be positive on last line");
        assert!(visual_lines[0].text.starts_with("[Image 1]"));
    }

    #[test]
    fn test_format_image_1_bracket_without_hash() {
        let line = "Look at [Image 1] and [Image #2] right now";
        let formatted = format_prompt_line_with_colored_placeholders(line);
        assert!(formatted.contains("\x1b[1;36;38;5;39m[Image 1]\x1b[0m"));
        assert!(formatted.contains("\x1b[1;36;38;5;39m[Image #2]\x1b[0m"));
    }

    #[test]
    fn test_paste_text_collapsing() {
        let mut prompt = Prompt::new();
        let long_paste = "line 1\nline 2\nline 3\nline 4\nline 5\nline 6";
        prompt.paste_text(long_paste);

        let text: String = prompt.buffer.iter().collect();
        assert_eq!(text, "[Pasted text #1, 6 lines]");
        assert_eq!(prompt.pending_pastes.len(), 1);
        assert_eq!(prompt.pending_pastes[0].line_count, 6);

        // Highlight check
        let formatted = format_prompt_line_with_colored_placeholders(&text);
        assert!(formatted.contains("\x1b[1;36;38;5;39m[Pasted text #1, 6 lines]\x1b[0m"));
        // Expansion check
        let expanded = expand_pasted_text_placeholders(&text, &prompt.pending_pastes);
        assert_eq!(expanded, long_paste);
    }

    #[test]
    fn test_short_paste_not_collapsed() {
        let mut prompt = Prompt::new();
        let short_paste = "cargo build --bin fusion";
        prompt.paste_text(short_paste);

        let text: String = prompt.buffer.iter().collect();
        assert_eq!(text, "cargo build --bin fusion");
        assert!(prompt.pending_pastes.is_empty());
    }

    #[test]
    fn test_wrap_adjacent_image_tags_never_split() {
        let text = "[Image #1, 1568x1037][Image #2, 1568x1037][Image #3, 1568x1037][Image #4, 1568x1037][Image #5, 1568x1037][Image #6, 1568x1037][Image #7, 1568x1037]";
        let buf: Vec<char> = text.chars().collect();
        let term_cols = 80;
        let (visual_lines, _, _) = wrap_prompt_lines(&buf, buf.len(), term_cols, 0);

        assert!(visual_lines.len() >= 2);
        for line in &visual_lines {
            if line.text.contains('[') {
                assert!(
                    line.text.contains(']'),
                    "A tag must never be chopped in half across lines: {:?}",
                    line.text
                );
            }
            let colored = format_prompt_line_with_colored_placeholders(&line.text);
            assert!(colored.contains("\x1b[1;36;38;5;39m"));
        }
    }
}
