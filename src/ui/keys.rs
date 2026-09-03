use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use serde::{Deserialize, Serialize};
use std::fmt;
use std::str::FromStr;

/// Configurable keybinding profile for interactive terminal input.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum KeybindingProfile {
    /// Standard modern terminal line editing (arrows, home/end, backspace, common Ctrl shortcuts).
    #[default]
    Default,
    /// Full Emacs / Readline keybindings (Ctrl+A/E/F/B/P/N/K/U/W/Y/T/D, Alt+F/B/D/U/L/C/Y, etc.).
    Emacs,
    /// Modal Vi editing with Insert mode and Normal (command) mode.
    Vi,
}

impl fmt::Display for KeybindingProfile {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            KeybindingProfile::Default => write!(f, "default"),
            KeybindingProfile::Emacs => write!(f, "emacs"),
            KeybindingProfile::Vi => write!(f, "vi"),
        }
    }
}

impl FromStr for KeybindingProfile {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.trim().to_lowercase().as_str() {
            "default" | "standard" | "normal" => Ok(KeybindingProfile::Default),
            "emacs" | "readline" => Ok(KeybindingProfile::Emacs),
            "vi" | "vim" => Ok(KeybindingProfile::Vi),
            other => Err(format!(
                "Unknown keybinding profile '{}'. Available profiles: default, emacs, vi",
                other
            )),
        }
    }
}

/// Operational mode when running in Vi keybinding profile.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
pub enum ViMode {
    /// Standard typing mode inserting characters at cursor.
    #[default]
    Insert,
    /// Modal command navigation and manipulation mode.
    Normal,
}

impl fmt::Display for ViMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ViMode::Insert => write!(f, "INSERT"),
            ViMode::Normal => write!(f, "NORMAL"),
        }
    }
}

impl ViMode {
    /// Short indicator string for status bar or prompt indicator.
    pub fn indicator(&self) -> &'static str {
        match self {
            ViMode::Insert => "[INS]",
            ViMode::Normal => "[NOR]",
        }
    }

    /// ANSI styled indicator string.
    pub fn styled_indicator(&self) -> &'static str {
        match self {
            ViMode::Insert => "\x1b[1;32m[INS]\x1b[0m",
            ViMode::Normal => "\x1b[1;33m[NOR]\x1b[0m",
        }
    }

    /// True if currently in Insert mode.
    pub fn is_insert(&self) -> bool {
        matches!(self, ViMode::Insert)
    }

    /// True if currently in Normal mode.
    pub fn is_normal(&self) -> bool {
        matches!(self, ViMode::Normal)
    }
}

/// Multi-key pending operator state for Vi Normal mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ViPendingOp {
    /// Waiting for second key of delete operator (`d`).
    Delete,
    /// Waiting for second key of change operator (`c`).
    Change,
    /// Waiting for second key of yank operator (`y`).
    Yank,
    /// Waiting for replacement char (`r`).
    Replace,
    /// Waiting for second `g` in `gg` goto command.
    Goto,
}

/// Outcome returned after processing a keyboard event.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KeyResult {
    /// Input handled, continuing loop (cursor or buffer may have updated).
    Continue,
    /// User submitted the input text.
    Submit(String),
    /// User canceled input (Ctrl+C).
    Cancel,
    /// User requested EOF / exit (Ctrl+D on empty input).
    Exit,
    /// Clear screen requested (Ctrl+L).
    ClearScreen,
    /// Key event was ignored or had no effect.
    Noop,
}

/// Mutable view of the editor state during an active prompt session.
pub struct PromptState<'a> {
    pub buffer: &'a mut Vec<char>,
    pub cursor_pos: &'a mut usize,
    pub history: &'a [String],
    pub history_idx: &'a mut Option<usize>,
    pub saved_current: &'a mut String,
}

impl<'a> PromptState<'a> {
    /// Create a new `PromptState` view.
    pub fn new(
        buffer: &'a mut Vec<char>,
        cursor_pos: &'a mut usize,
        history: &'a [String],
        history_idx: &'a mut Option<usize>,
        saved_current: &'a mut String,
    ) -> Self {
        Self {
            buffer,
            cursor_pos,
            history,
            history_idx,
            saved_current,
        }
    }

    /// Get current buffer as a String.
    pub fn text(&self) -> String {
        self.buffer.iter().collect()
    }

    /// Total characters in buffer.
    pub fn len(&self) -> usize {
        self.buffer.len()
    }

    /// Whether buffer is empty.
    pub fn is_empty(&self) -> bool {
        self.buffer.is_empty()
    }

    /// Current cursor position.
    pub fn cursor(&self) -> usize {
        *self.cursor_pos
    }

    /// Clamp cursor position to valid range [0, buffer.len()].
    pub fn clamp_cursor(&mut self) {
        if *self.cursor_pos > self.buffer.len() {
            *self.cursor_pos = self.buffer.len();
        }
    }

    /// Insert a character at current cursor position and advance cursor.
    pub fn insert_char(&mut self, c: char) {
        self.clamp_cursor();
        self.buffer.insert(*self.cursor_pos, c);
        *self.cursor_pos += 1;
    }

    /// Insert a string slice at current cursor position and advance cursor.
    pub fn insert_str(&mut self, s: &str) {
        self.clamp_cursor();
        for c in s.chars() {
            self.buffer.insert(*self.cursor_pos, c);
            *self.cursor_pos += 1;
        }
    }

    /// Remove character under cursor if within bounds.
    pub fn remove_char_at_cursor(&mut self) -> Option<char> {
        self.clamp_cursor();
        if *self.cursor_pos < self.buffer.len() {
            Some(self.buffer.remove(*self.cursor_pos))
        } else {
            None
        }
    }

    /// Remove character immediately before cursor and decrement cursor.
    pub fn remove_char_before_cursor(&mut self) -> Option<char> {
        self.clamp_cursor();
        if *self.cursor_pos > 0 {
            *self.cursor_pos -= 1;
            Some(self.buffer.remove(*self.cursor_pos))
        } else {
            None
        }
    }

    /// Compute line information: (current_line_idx, current_col, line_ranges).
    /// Each line range is `(start_index, length_excluding_newline)`.
    pub fn line_info(&self) -> (usize, usize, Vec<(usize, usize)>) {
        let mut ranges = Vec::new();
        let mut line_start = 0;

        for (idx, &c) in self.buffer.iter().enumerate() {
            if c == '\n' {
                ranges.push((line_start, idx - line_start));
                line_start = idx + 1;
            }
        }
        ranges.push((line_start, self.buffer.len() - line_start));

        let mut cur_line = 0;
        let mut cur_col = 0;
        let pos = *self.cursor_pos;

        for (line_idx, &(start, len)) in ranges.iter().enumerate() {
            if pos >= start && pos <= start + len {
                cur_line = line_idx;
                cur_col = pos - start;
                break;
            }
        }

        (cur_line, cur_col, ranges)
    }

    /// Return `(start_idx, length)` of the line containing cursor.
    pub fn current_line_range(&self) -> (usize, usize) {
        let (cur_line, _, ranges) = self.line_info();
        ranges[cur_line]
    }

    /// Move cursor to beginning of current line.
    pub fn move_to_bol(&mut self) {
        let (start, _) = self.current_line_range();
        *self.cursor_pos = start;
    }

    /// Move cursor to end of current line.
    pub fn move_to_eol(&mut self) {
        let (start, len) = self.current_line_range();
        *self.cursor_pos = start + len;
    }

    /// Move cursor to first non-whitespace character on current line.
    pub fn move_to_first_non_whitespace(&mut self) {
        let (start, len) = self.current_line_range();
        let mut idx = start;
        while idx < start + len && self.buffer[idx].is_whitespace() {
            idx += 1;
        }
        *self.cursor_pos = idx;
    }

    /// Find previous word start position before cursor.
    pub fn prev_word_pos(&self) -> usize {
        let pos = *self.cursor_pos;
        if pos == 0 {
            return 0;
        }
        let mut p = pos;
        while p > 0 && self.buffer[p - 1].is_whitespace() {
            p -= 1;
        }
        while p > 0 && !self.buffer[p - 1].is_whitespace() {
            p -= 1;
        }
        p
    }

    /// Find next word start position after cursor.
    pub fn next_word_pos(&self) -> usize {
        let len = self.buffer.len();
        let pos = *self.cursor_pos;
        if pos >= len {
            return len;
        }
        let mut p = pos;
        while p < len && !self.buffer[p].is_whitespace() {
            p += 1;
        }
        while p < len && self.buffer[p].is_whitespace() {
            p += 1;
        }
        p
    }

    /// Find next word end position (for Vi `e`).
    pub fn next_word_end_pos(&self) -> usize {
        let len = self.buffer.len();
        let pos = *self.cursor_pos;
        if pos >= len {
            return len;
        }
        let mut p = pos;
        if p + 1 < len {
            p += 1;
        }
        while p < len && self.buffer[p].is_whitespace() {
            p += 1;
        }
        while p + 1 < len && !self.buffer[p + 1].is_whitespace() && self.buffer[p + 1] != '\n' {
            p += 1;
        }
        p.min(len)
    }

    /// Move cursor up one line if multiline, otherwise navigate history backward.
    pub fn move_up_or_history(&mut self) {
        let (cur_line, cur_col, ranges) = self.line_info();
        if cur_line > 0 {
            let (prev_start, prev_len) = ranges[cur_line - 1];
            let new_col = cur_col.min(prev_len);
            *self.cursor_pos = prev_start + new_col;
        } else if !self.history.is_empty() {
            if self.history_idx.is_none() {
                *self.saved_current = self.buffer.iter().collect();
                *self.history_idx = Some(self.history.len() - 1);
            } else if let Some(idx) = *self.history_idx {
                if idx > 0 {
                    *self.history_idx = Some(idx - 1);
                }
            }

            if let Some(idx) = *self.history_idx {
                if let Some(entry) = self.history.get(idx) {
                    *self.buffer = entry.chars().collect();
                    *self.cursor_pos = self.buffer.len();
                }
            }
        }
    }

    /// Move cursor down one line if multiline, otherwise navigate history forward.
    pub fn move_down_or_history(&mut self) {
        let (cur_line, cur_col, ranges) = self.line_info();
        if cur_line + 1 < ranges.len() {
            let (next_start, next_len) = ranges[cur_line + 1];
            let new_col = cur_col.min(next_len);
            *self.cursor_pos = next_start + new_col;
        } else if let Some(idx) = *self.history_idx {
            if idx + 1 < self.history.len() {
                *self.history_idx = Some(idx + 1);
                if let Some(entry) = self.history.get(idx + 1) {
                    *self.buffer = entry.chars().collect();
                    *self.cursor_pos = self.buffer.len();
                }
            } else {
                *self.history_idx = None;
                *self.buffer = self.saved_current.chars().collect();
                *self.cursor_pos = self.buffer.len();
            }
        }
    }

    /// Drain a range of characters and return as a String.
    pub fn drain_range(&mut self, start: usize, end: usize) -> String {
        let s = start.min(self.buffer.len());
        let e = end.min(self.buffer.len());
        if s < e {
            let drained: String = self.buffer.drain(s..e).collect();
            if *self.cursor_pos > s {
                *self.cursor_pos = s;
            }
            drained
        } else {
            String::new()
        }
    }
}

/// Keybinding handler and state manager supporting Default, Emacs, and Vi profiles.
#[derive(Debug, Clone)]
pub struct KeyHandler {
    profile: KeybindingProfile,
    vi_mode: ViMode,
    vi_pending: Option<ViPendingOp>,
    kill_ring: Vec<String>,
    last_yank_len: usize,
    last_action_was_yank: bool,
    undo_stack: Vec<(Vec<char>, usize)>,
    redo_stack: Vec<(Vec<char>, usize)>,
    keymap: Option<crate::ui::keymap_config::KeymapManager>,
}

impl Default for KeyHandler {
    fn default() -> Self {
        Self::new(KeybindingProfile::Default)
    }
}

impl KeyHandler {
    /// Create a new key handler with specified profile.
    pub fn new(profile: KeybindingProfile) -> Self {
        Self {
            profile,
            vi_mode: ViMode::Insert,
            vi_pending: None,
            kill_ring: Vec::new(),
            last_yank_len: 0,
            last_action_was_yank: false,
            undo_stack: Vec::new(),
            redo_stack: Vec::new(),
            keymap: None,
        }
    }

    /// Return active keybinding profile.
    pub fn profile(&self) -> KeybindingProfile {
        self.profile
    }

    /// Switch active keybinding profile.
    pub fn set_profile(&mut self, profile: KeybindingProfile) {
        self.profile = profile;
        if profile == KeybindingProfile::Vi {
            self.vi_mode = ViMode::Insert;
        }
        self.vi_pending = None;
    }

    /// Current Vi mode (Insert or Normal).
    pub fn vi_mode(&self) -> ViMode {
        self.vi_mode
    }

    /// Switch Vi mode explicitly.
    pub fn set_vi_mode(&mut self, mode: ViMode) {
        self.vi_mode = mode;
        self.vi_pending = None;
    }

    /// Clear any pending multi-key operator.
    pub fn clear_pending(&mut self) {
        self.vi_pending = None;
    }

    /// Check if in Vi mode and return the styled indicator, if any.
    pub fn mode_indicator(&self) -> Option<&'static str> {
        if self.profile == KeybindingProfile::Vi {
            Some(self.vi_mode.styled_indicator())
        } else {
            None
        }
    }
    /// Attach a custom keymap configuration.
    pub fn with_keymap(mut self, config: crate::ui::keymap_config::KeymapConfig) -> Self {
        self.keymap = Some(crate::ui::keymap_config::KeymapManager::from_config(config));
        self
    }

    /// Set custom keymap configuration.
    pub fn set_keymap(&mut self, config: crate::ui::keymap_config::KeymapConfig) {
        self.keymap = Some(crate::ui::keymap_config::KeymapManager::from_config(config));
    }

    /// Access custom keymap manager, if configured.
    pub fn keymap(&self) -> Option<&crate::ui::keymap_config::KeymapManager> {
        self.keymap.as_ref()
    }

    /// Mutable access to custom keymap manager, if configured.
    pub fn keymap_mut(&mut self) -> Option<&mut crate::ui::keymap_config::KeymapManager> {
        self.keymap.as_mut()
    }

    /// Load custom keymap from `~/.fusion/keymap.json` if the file exists on disk.
    pub fn load_custom_keymap(&mut self) {
        let path = crate::ui::keymap_config::KeymapConfig::default_path();
        if path.exists() {
            self.keymap = Some(crate::ui::keymap_config::KeymapManager::load());
        }
    }

    /// Save an undo snapshot of current buffer and cursor position.
    pub fn snapshot_undo(&mut self, buffer: &[char], cursor_pos: usize) {
        if self.undo_stack.len() >= 128 {
            self.undo_stack.remove(0);
        }
        self.undo_stack.push((buffer.to_vec(), cursor_pos));
        self.redo_stack.clear();
    }

    /// Perform Undo.
    pub fn undo(&mut self, state: &mut PromptState) -> bool {
        if let Some((prev_buf, prev_cur)) = self.undo_stack.pop() {
            self.redo_stack
                .push((state.buffer.clone(), *state.cursor_pos));
            *state.buffer = prev_buf;
            *state.cursor_pos = prev_cur.min(state.buffer.len());
            true
        } else {
            false
        }
    }

    /// Perform Redo.
    pub fn redo(&mut self, state: &mut PromptState) -> bool {
        if let Some((next_buf, next_cur)) = self.redo_stack.pop() {
            self.undo_stack
                .push((state.buffer.clone(), *state.cursor_pos));
            *state.buffer = next_buf;
            *state.cursor_pos = next_cur.min(state.buffer.len());
            true
        } else {
            false
        }
    }

    /// Push text into the kill ring (clipboard).
    pub fn push_kill(&mut self, text: String) {
        if !text.is_empty() {
            if self.kill_ring.len() >= 64 {
                self.kill_ring.remove(0);
            }
            self.kill_ring.push(text);
        }
    }

    /// Primary entry point: process a keyboard event against current state.
    pub fn handle_key(&mut self, key: KeyEvent, state: &mut PromptState) -> KeyResult {
        // Ignore Release events on platforms reporting them
        if key.kind == KeyEventKind::Release {
            return KeyResult::Noop;
        }

        // Check custom keymap overrides first
        if let Some(mut km) = self.keymap.take() {
            let action = km.resolve(
                &key,
                self.profile,
                if self.profile == KeybindingProfile::Vi {
                    Some(self.vi_mode)
                } else {
                    None
                },
            );
            self.keymap = Some(km);
            if let Some(act) = action {
                return act.execute(state, self);
            }
        }

        let was_yank = self.last_action_was_yank;
        self.last_action_was_yank = false;

        match self.profile {
            KeybindingProfile::Default => self.handle_default(key, state),
            KeybindingProfile::Emacs => self.handle_emacs(key, state, was_yank),
            KeybindingProfile::Vi => self.handle_vi(key, state),
        }
    }

    // ========================================================================
    // Default Mode Handler
    // ========================================================================

    fn handle_default(&mut self, key: KeyEvent, state: &mut PromptState) -> KeyResult {
        match (key.code, key.modifiers) {
            // Enter: submit or multiline backslash
            (KeyCode::Enter, KeyModifiers::NONE) => {
                if !state.is_empty() && state.buffer[state.buffer.len() - 1] == '\\' {
                    self.snapshot_undo(state.buffer, *state.cursor_pos);
                    state.buffer.pop();
                    state.clamp_cursor();
                    state.insert_char('\n');
                    KeyResult::Continue
                } else {
                    let text = state.text();
                    KeyResult::Submit(text)
                }
            }

            // Multiline newline (Ctrl+J, Ctrl+Enter, Alt+Enter, Shift+Enter)
            (KeyCode::Char('j'), KeyModifiers::CONTROL)
            | (KeyCode::Enter, KeyModifiers::CONTROL)
            | (KeyCode::Enter, KeyModifiers::ALT)
            | (KeyCode::Enter, KeyModifiers::SHIFT) => {
                self.snapshot_undo(state.buffer, *state.cursor_pos);
                state.insert_char('\n');
                KeyResult::Continue
            }

            // Cancel current turn (Ctrl+C)
            (KeyCode::Char('c'), KeyModifiers::CONTROL) => KeyResult::Cancel,

            // Exit on empty, or delete character under cursor (Ctrl+D)
            (KeyCode::Char('d'), KeyModifiers::CONTROL) => {
                if state.is_empty() {
                    KeyResult::Exit
                } else {
                    self.snapshot_undo(state.buffer, *state.cursor_pos);
                    state.remove_char_at_cursor();
                    KeyResult::Continue
                }
            }

            // Word delete backward (Ctrl+W or Alt+Backspace)
            (KeyCode::Char('w'), KeyModifiers::CONTROL)
            | (KeyCode::Backspace, KeyModifiers::ALT) => {
                let prev = state.prev_word_pos();
                if prev < *state.cursor_pos {
                    self.snapshot_undo(state.buffer, *state.cursor_pos);
                    let drained = state.drain_range(prev, *state.cursor_pos);
                    self.push_kill(drained);
                }
                KeyResult::Continue
            }

            // Backspace / Ctrl+H
            (KeyCode::Backspace, _) | (KeyCode::Char('h'), KeyModifiers::CONTROL) => {
                if *state.cursor_pos > 0 {
                    self.snapshot_undo(state.buffer, *state.cursor_pos);
                    state.remove_char_before_cursor();
                }
                KeyResult::Continue
            }

            // Word delete forward (Alt+D or Alt+Delete or Ctrl+Delete)
            (KeyCode::Char('d'), KeyModifiers::ALT)
            | (KeyCode::Delete, KeyModifiers::ALT)
            | (KeyCode::Delete, KeyModifiers::CONTROL) => {
                let next = state.next_word_pos();
                if next > *state.cursor_pos {
                    self.snapshot_undo(state.buffer, *state.cursor_pos);
                    let drained = state.drain_range(*state.cursor_pos, next);
                    self.push_kill(drained);
                }
                KeyResult::Continue
            }

            // Delete
            (KeyCode::Delete, _) => {
                self.snapshot_undo(state.buffer, *state.cursor_pos);
                state.remove_char_at_cursor();
                KeyResult::Continue
            }

            // Clear line to start (Ctrl+U)
            (KeyCode::Char('u'), KeyModifiers::CONTROL) => {
                let (cur_line, cur_col, line_ranges) = state.line_info();
                let (start, _) = line_ranges[cur_line];
                self.snapshot_undo(state.buffer, *state.cursor_pos);
                if cur_col > 0 {
                    let drained = state.drain_range(start, *state.cursor_pos);
                    self.push_kill(drained);
                } else if cur_line > 0 {
                    state.buffer.remove(start - 1);
                    *state.cursor_pos = start - 1;
                }
                KeyResult::Continue
            }

            // Clear line to end (Ctrl+K)
            (KeyCode::Char('k'), KeyModifiers::CONTROL) => {
                let (cur_line, cur_col, line_ranges) = state.line_info();
                let (start, len) = line_ranges[cur_line];
                self.snapshot_undo(state.buffer, *state.cursor_pos);
                if cur_col < len {
                    let drained = state.drain_range(*state.cursor_pos, start + len);
                    self.push_kill(drained);
                } else if cur_line + 1 < line_ranges.len() {
                    state.buffer.remove(start + len);
                }
                KeyResult::Continue
            }

            // Home / Ctrl+A
            (KeyCode::Home, _) | (KeyCode::Char('a'), KeyModifiers::CONTROL) => {
                state.move_to_bol();
                KeyResult::Continue
            }

            // End / Ctrl+E
            (KeyCode::End, _) | (KeyCode::Char('e'), KeyModifiers::CONTROL) => {
                state.move_to_eol();
                KeyResult::Continue
            }

            // Word left: Ctrl+Left / Alt+Left / Alt+B
            (KeyCode::Left, KeyModifiers::CONTROL)
            | (KeyCode::Left, KeyModifiers::ALT)
            | (KeyCode::Char('b'), KeyModifiers::ALT) => {
                *state.cursor_pos = state.prev_word_pos();
                KeyResult::Continue
            }

            // Word right: Ctrl+Right / Alt+Right / Alt+F
            (KeyCode::Right, KeyModifiers::CONTROL)
            | (KeyCode::Right, KeyModifiers::ALT)
            | (KeyCode::Char('f'), KeyModifiers::ALT) => {
                *state.cursor_pos = state.next_word_pos();
                KeyResult::Continue
            }

            // Left arrow
            (KeyCode::Left, _) => {
                if *state.cursor_pos > 0 {
                    *state.cursor_pos -= 1;
                }
                KeyResult::Continue
            }

            // Right arrow
            (KeyCode::Right, _) => {
                if *state.cursor_pos < state.buffer.len() {
                    *state.cursor_pos += 1;
                }
                KeyResult::Continue
            }

            // Up arrow
            (KeyCode::Up, _) => {
                state.move_up_or_history();
                KeyResult::Continue
            }

            // Down arrow
            (KeyCode::Down, _) => {
                state.move_down_or_history();
                KeyResult::Continue
            }

            // Tab: insert 2 spaces
            (KeyCode::Tab, _) => {
                self.snapshot_undo(state.buffer, *state.cursor_pos);
                state.insert_str("  ");
                KeyResult::Continue
            }

            // Ctrl+L: Clear screen
            (KeyCode::Char('l'), KeyModifiers::CONTROL) => KeyResult::ClearScreen,

            // Undo: Ctrl+Z
            (KeyCode::Char('z'), KeyModifiers::CONTROL) => {
                self.undo(state);
                KeyResult::Continue
            }

            // Printable character
            (KeyCode::Char(c), KeyModifiers::NONE | KeyModifiers::SHIFT) => {
                if state.is_empty() || c == ' ' {
                    self.snapshot_undo(state.buffer, *state.cursor_pos);
                }
                state.insert_char(c);
                KeyResult::Continue
            }

            _ => KeyResult::Noop,
        }
    }

    // ========================================================================
    // Emacs Mode Handler
    // ========================================================================

    fn handle_emacs(
        &mut self,
        key: KeyEvent,
        state: &mut PromptState,
        was_yank: bool,
    ) -> KeyResult {
        match (key.code, key.modifiers) {
            // Enter: Submit or Multiline if ending with backslash
            (KeyCode::Enter, KeyModifiers::NONE) => {
                if !state.is_empty() && state.buffer[state.buffer.len() - 1] == '\\' {
                    self.snapshot_undo(state.buffer, *state.cursor_pos);
                    state.buffer.pop();
                    state.clamp_cursor();
                    state.insert_char('\n');
                    KeyResult::Continue
                } else {
                    let text = state.text();
                    KeyResult::Submit(text)
                }
            }

            // Ctrl+J / Ctrl+Enter: Insert newline
            (KeyCode::Char('j'), KeyModifiers::CONTROL)
            | (KeyCode::Enter, KeyModifiers::CONTROL) => {
                self.snapshot_undo(state.buffer, *state.cursor_pos);
                state.insert_char('\n');
                KeyResult::Continue
            }

            // Ctrl+A: Beginning of line
            (KeyCode::Char('a'), KeyModifiers::CONTROL) | (KeyCode::Home, _) => {
                state.move_to_bol();
                KeyResult::Continue
            }

            // Ctrl+E: End of line
            (KeyCode::Char('e'), KeyModifiers::CONTROL) | (KeyCode::End, _) => {
                state.move_to_eol();
                KeyResult::Continue
            }

            // Ctrl+B: Backward one character
            (KeyCode::Char('b'), KeyModifiers::CONTROL) | (KeyCode::Left, KeyModifiers::NONE) => {
                if *state.cursor_pos > 0 {
                    *state.cursor_pos -= 1;
                }
                KeyResult::Continue
            }

            // Ctrl+F: Forward one character
            (KeyCode::Char('f'), KeyModifiers::CONTROL) | (KeyCode::Right, KeyModifiers::NONE) => {
                if *state.cursor_pos < state.buffer.len() {
                    *state.cursor_pos += 1;
                }
                KeyResult::Continue
            }

            // Alt+B: Backward one word
            (KeyCode::Char('b'), KeyModifiers::ALT) | (KeyCode::Left, KeyModifiers::ALT) => {
                *state.cursor_pos = state.prev_word_pos();
                KeyResult::Continue
            }

            // Alt+F: Forward one word
            (KeyCode::Char('f'), KeyModifiers::ALT) | (KeyCode::Right, KeyModifiers::ALT) => {
                *state.cursor_pos = state.next_word_pos();
                KeyResult::Continue
            }

            // Ctrl+P: Previous line / history up
            (KeyCode::Char('p'), KeyModifiers::CONTROL) | (KeyCode::Up, _) => {
                state.move_up_or_history();
                KeyResult::Continue
            }

            // Ctrl+N: Next line / history down
            (KeyCode::Char('n'), KeyModifiers::CONTROL) | (KeyCode::Down, _) => {
                state.move_down_or_history();
                KeyResult::Continue
            }

            // Alt+<: Beginning of buffer
            (KeyCode::Char('<'), KeyModifiers::ALT) => {
                *state.cursor_pos = 0;
                KeyResult::Continue
            }

            // Alt+>: End of buffer
            (KeyCode::Char('>'), KeyModifiers::ALT) => {
                *state.cursor_pos = state.buffer.len();
                KeyResult::Continue
            }

            // Ctrl+D: Delete character forward or exit if empty
            (KeyCode::Char('d'), KeyModifiers::CONTROL) => {
                if state.is_empty() {
                    KeyResult::Exit
                } else {
                    self.snapshot_undo(state.buffer, *state.cursor_pos);
                    state.remove_char_at_cursor();
                    KeyResult::Continue
                }
            }

            // Alt+Backspace / Ctrl+W: Kill word backward
            (KeyCode::Backspace, KeyModifiers::ALT)
            | (KeyCode::Char('w'), KeyModifiers::CONTROL) => {
                let prev = state.prev_word_pos();
                if prev < *state.cursor_pos {
                    self.snapshot_undo(state.buffer, *state.cursor_pos);
                    let drained = state.drain_range(prev, *state.cursor_pos);
                    self.push_kill(drained);
                }
                KeyResult::Continue
            }

            // Backspace / Ctrl+H: Delete character backward
            (KeyCode::Backspace, _) | (KeyCode::Char('h'), KeyModifiers::CONTROL) => {
                if *state.cursor_pos > 0 {
                    self.snapshot_undo(state.buffer, *state.cursor_pos);
                    state.remove_char_before_cursor();
                }
                KeyResult::Continue
            }

            // Delete: Delete character forward
            (KeyCode::Delete, _) => {
                self.snapshot_undo(state.buffer, *state.cursor_pos);
                state.remove_char_at_cursor();
                KeyResult::Continue
            }

            // Alt+D: Kill word forward
            (KeyCode::Char('d'), KeyModifiers::ALT) => {
                let next = state.next_word_pos();
                if next > *state.cursor_pos {
                    self.snapshot_undo(state.buffer, *state.cursor_pos);
                    let drained = state.drain_range(*state.cursor_pos, next);
                    self.push_kill(drained);
                }
                KeyResult::Continue
            }

            // Ctrl+K: Kill line to end
            (KeyCode::Char('k'), KeyModifiers::CONTROL) => {
                let (cur_line, cur_col, line_ranges) = state.line_info();
                let (start, len) = line_ranges[cur_line];
                self.snapshot_undo(state.buffer, *state.cursor_pos);
                if cur_col < len {
                    let drained = state.drain_range(*state.cursor_pos, start + len);
                    self.push_kill(drained);
                } else if cur_line + 1 < line_ranges.len() {
                    state.buffer.remove(start + len);
                }
                KeyResult::Continue
            }

            // Ctrl+U: Kill line to beginning
            (KeyCode::Char('u'), KeyModifiers::CONTROL) => {
                let (cur_line, cur_col, line_ranges) = state.line_info();
                let (start, _) = line_ranges[cur_line];
                self.snapshot_undo(state.buffer, *state.cursor_pos);
                if cur_col > 0 {
                    let drained = state.drain_range(start, *state.cursor_pos);
                    self.push_kill(drained);
                } else if cur_line > 0 {
                    state.buffer.remove(start - 1);
                    *state.cursor_pos = start - 1;
                }
                KeyResult::Continue
            }

            // Ctrl+Y: Yank (paste from kill ring)
            (KeyCode::Char('y'), KeyModifiers::CONTROL) => {
                if let Some(text) = self.kill_ring.last() {
                    let text = text.clone();
                    self.snapshot_undo(state.buffer, *state.cursor_pos);
                    self.last_yank_len = text.chars().count();
                    self.last_action_was_yank = true;
                    state.insert_str(&text);
                }
                KeyResult::Continue
            }

            // Alt+Y: Yank-pop (cycle kill ring if previous action was yank)
            (KeyCode::Char('y'), KeyModifiers::ALT) => {
                if was_yank && self.kill_ring.len() > 1 {
                    // Remove last yanked text
                    let pos = *state.cursor_pos;
                    let remove_start = pos.saturating_sub(self.last_yank_len);
                    state.drain_range(remove_start, pos);

                    // Cycle kill ring
                    let item = self.kill_ring.pop().unwrap();
                    self.kill_ring.insert(0, item);

                    if let Some(prev_item) = self.kill_ring.last() {
                        let text = prev_item.clone();
                        self.last_yank_len = text.chars().count();
                        self.last_action_was_yank = true;
                        state.insert_str(&text);
                    }
                }
                KeyResult::Continue
            }

            // Ctrl+T: Transpose characters
            (KeyCode::Char('t'), KeyModifiers::CONTROL) => {
                let len = state.buffer.len();
                if len >= 2 && *state.cursor_pos > 0 {
                    self.snapshot_undo(state.buffer, *state.cursor_pos);
                    let pos = *state.cursor_pos;
                    if pos == len {
                        state.buffer.swap(pos - 1, pos - 2);
                    } else {
                        state.buffer.swap(pos, pos - 1);
                        *state.cursor_pos += 1;
                    }
                }
                KeyResult::Continue
            }

            // Alt+T: Transpose words
            (KeyCode::Char('t'), KeyModifiers::ALT) => {
                let prev = state.prev_word_pos();
                let next = state.next_word_pos();
                if prev < *state.cursor_pos && next > *state.cursor_pos {
                    self.snapshot_undo(state.buffer, *state.cursor_pos);
                    let pos = *state.cursor_pos;
                    let first_word: String = state.buffer[prev..pos].iter().collect();
                    let second_word: String = state.buffer[pos..next].iter().collect();
                    state
                        .buffer
                        .splice(prev..next, second_word.chars().chain(first_word.chars()));
                    *state.cursor_pos = next;
                }
                KeyResult::Continue
            }

            // Alt+U: Uppercase word forward
            (KeyCode::Char('u'), KeyModifiers::ALT) => {
                let next = state.next_word_pos();
                if next > *state.cursor_pos {
                    self.snapshot_undo(state.buffer, *state.cursor_pos);
                    for i in *state.cursor_pos..next {
                        state.buffer[i] = state.buffer[i].to_ascii_uppercase();
                    }
                    *state.cursor_pos = next;
                }
                KeyResult::Continue
            }

            // Alt+L: Lowercase word forward
            (KeyCode::Char('l'), KeyModifiers::ALT) => {
                let next = state.next_word_pos();
                if next > *state.cursor_pos {
                    self.snapshot_undo(state.buffer, *state.cursor_pos);
                    for i in *state.cursor_pos..next {
                        state.buffer[i] = state.buffer[i].to_ascii_lowercase();
                    }
                    *state.cursor_pos = next;
                }
                KeyResult::Continue
            }

            // Alt+C: Capitalize word forward
            (KeyCode::Char('c'), KeyModifiers::ALT) => {
                let next = state.next_word_pos();
                if next > *state.cursor_pos {
                    self.snapshot_undo(state.buffer, *state.cursor_pos);
                    let mut first = true;
                    for i in *state.cursor_pos..next {
                        if first && state.buffer[i].is_alphabetic() {
                            state.buffer[i] = state.buffer[i].to_ascii_uppercase();
                            first = false;
                        } else if !state.buffer[i].is_alphabetic() {
                            // punctuation or space
                        } else {
                            state.buffer[i] = state.buffer[i].to_ascii_lowercase();
                        }
                    }
                    *state.cursor_pos = next;
                }
                KeyResult::Continue
            }

            // Ctrl+_ or Ctrl+/ : Undo
            (KeyCode::Char('_'), KeyModifiers::CONTROL)
            | (KeyCode::Char('/'), KeyModifiers::CONTROL) => {
                self.undo(state);
                KeyResult::Continue
            }

            // Ctrl+L: Clear screen
            (KeyCode::Char('l'), KeyModifiers::CONTROL) => KeyResult::ClearScreen,

            // Ctrl+C: Cancel
            (KeyCode::Char('c'), KeyModifiers::CONTROL) => KeyResult::Cancel,

            // Tab: insert 2 spaces
            (KeyCode::Tab, _) => {
                self.snapshot_undo(state.buffer, *state.cursor_pos);
                state.insert_str("  ");
                KeyResult::Continue
            }

            // Printable character
            (KeyCode::Char(c), KeyModifiers::NONE | KeyModifiers::SHIFT) => {
                if state.is_empty() || c == ' ' {
                    self.snapshot_undo(state.buffer, *state.cursor_pos);
                }
                state.insert_char(c);
                KeyResult::Continue
            }

            _ => KeyResult::Noop,
        }
    }

    // ========================================================================
    // Vi Mode Handler
    // ========================================================================

    fn handle_vi(&mut self, key: KeyEvent, state: &mut PromptState) -> KeyResult {
        match self.vi_mode {
            ViMode::Insert => self.handle_vi_insert(key, state),
            ViMode::Normal => self.handle_vi_normal(key, state),
        }
    }

    /// Vi Insert Mode: type normally, Esc switches to Normal mode.
    fn handle_vi_insert(&mut self, key: KeyEvent, state: &mut PromptState) -> KeyResult {
        match (key.code, key.modifiers) {
            // Esc or Ctrl+[: switch to Normal mode
            (KeyCode::Esc, _) | (KeyCode::Char('['), KeyModifiers::CONTROL) => {
                self.vi_mode = ViMode::Normal;
                self.vi_pending = None;
                // In Vi, transitioning to Normal mode moves cursor one char left if col > 0
                let (_, cur_col, _) = state.line_info();
                if cur_col > 0 && *state.cursor_pos > 0 {
                    *state.cursor_pos -= 1;
                }
                KeyResult::Continue
            }

            // Enter: submit or multiline backslash
            (KeyCode::Enter, KeyModifiers::NONE) => {
                if !state.is_empty() && state.buffer[state.buffer.len() - 1] == '\\' {
                    self.snapshot_undo(state.buffer, *state.cursor_pos);
                    state.buffer.pop();
                    state.clamp_cursor();
                    state.insert_char('\n');
                    KeyResult::Continue
                } else {
                    let text = state.text();
                    KeyResult::Submit(text)
                }
            }

            // Multiline newline (Ctrl+J, Ctrl+Enter, Alt+Enter)
            (KeyCode::Char('j'), KeyModifiers::CONTROL)
            | (KeyCode::Enter, KeyModifiers::CONTROL)
            | (KeyCode::Enter, KeyModifiers::ALT) => {
                self.snapshot_undo(state.buffer, *state.cursor_pos);
                state.insert_char('\n');
                KeyResult::Continue
            }

            // Cancel (Ctrl+C)
            (KeyCode::Char('c'), KeyModifiers::CONTROL) => KeyResult::Cancel,

            // Exit on empty, or delete forward (Ctrl+D)
            (KeyCode::Char('d'), KeyModifiers::CONTROL) => {
                if state.is_empty() {
                    KeyResult::Exit
                } else {
                    self.snapshot_undo(state.buffer, *state.cursor_pos);
                    state.remove_char_at_cursor();
                    KeyResult::Continue
                }
            }

            // Word delete backward (Ctrl+W)
            (KeyCode::Char('w'), KeyModifiers::CONTROL) => {
                let prev = state.prev_word_pos();
                if prev < *state.cursor_pos {
                    self.snapshot_undo(state.buffer, *state.cursor_pos);
                    let drained = state.drain_range(prev, *state.cursor_pos);
                    self.push_kill(drained);
                }
                KeyResult::Continue
            }

            // Delete line backward (Ctrl+U)
            (KeyCode::Char('u'), KeyModifiers::CONTROL) => {
                let (cur_line, cur_col, line_ranges) = state.line_info();
                let (start, _) = line_ranges[cur_line];
                self.snapshot_undo(state.buffer, *state.cursor_pos);
                if cur_col > 0 {
                    let drained = state.drain_range(start, *state.cursor_pos);
                    self.push_kill(drained);
                }
                KeyResult::Continue
            }

            // Backspace / Ctrl+H
            (KeyCode::Backspace, _) | (KeyCode::Char('h'), KeyModifiers::CONTROL) => {
                if *state.cursor_pos > 0 {
                    self.snapshot_undo(state.buffer, *state.cursor_pos);
                    state.remove_char_before_cursor();
                }
                KeyResult::Continue
            }

            // Delete
            (KeyCode::Delete, _) => {
                self.snapshot_undo(state.buffer, *state.cursor_pos);
                state.remove_char_at_cursor();
                KeyResult::Continue
            }

            // Arrow keys & Home / End in Insert mode
            (KeyCode::Left, _) => {
                if *state.cursor_pos > 0 {
                    *state.cursor_pos -= 1;
                }
                KeyResult::Continue
            }
            (KeyCode::Right, _) => {
                if *state.cursor_pos < state.buffer.len() {
                    *state.cursor_pos += 1;
                }
                KeyResult::Continue
            }
            (KeyCode::Up, _) => {
                state.move_up_or_history();
                KeyResult::Continue
            }
            (KeyCode::Down, _) => {
                state.move_down_or_history();
                KeyResult::Continue
            }
            (KeyCode::Home, _) => {
                state.move_to_bol();
                KeyResult::Continue
            }
            (KeyCode::End, _) => {
                state.move_to_eol();
                KeyResult::Continue
            }

            // Tab: insert 2 spaces
            (KeyCode::Tab, _) => {
                self.snapshot_undo(state.buffer, *state.cursor_pos);
                state.insert_str("  ");
                KeyResult::Continue
            }

            // Printable character
            (KeyCode::Char(c), KeyModifiers::NONE | KeyModifiers::SHIFT) => {
                if state.is_empty() || c == ' ' {
                    self.snapshot_undo(state.buffer, *state.cursor_pos);
                }
                state.insert_char(c);
                KeyResult::Continue
            }

            _ => KeyResult::Noop,
        }
    }

    /// Vi Normal Mode: modal navigation, operators, deletion, substitution, and paste.
    fn handle_vi_normal(&mut self, key: KeyEvent, state: &mut PromptState) -> KeyResult {
        // Handle pending multi-key operations (e.g. d, c, y, r, g)
        if let Some(op) = self.vi_pending {
            return self.handle_vi_pending_op(op, key, state);
        }

        match (key.code, key.modifiers) {
            // Esc: clear state / stay in Normal mode
            (KeyCode::Esc, _) => {
                self.vi_pending = None;
                KeyResult::Continue
            }

            // Enter: submit
            (KeyCode::Enter, KeyModifiers::NONE) => {
                let text = state.text();
                KeyResult::Submit(text)
            }

            // Cancel (Ctrl+C)
            (KeyCode::Char('c'), KeyModifiers::CONTROL) => KeyResult::Cancel,

            // Clear screen (Ctrl+L)
            (KeyCode::Char('l'), KeyModifiers::CONTROL) => KeyResult::ClearScreen,

            // ---- Mode transitions to Insert ----
            // 'i': insert before cursor
            (KeyCode::Char('i'), KeyModifiers::NONE) => {
                self.vi_mode = ViMode::Insert;
                KeyResult::Continue
            }

            // 'I': insert at first non-blank char of current line
            (KeyCode::Char('I'), KeyModifiers::SHIFT) => {
                state.move_to_first_non_whitespace();
                self.vi_mode = ViMode::Insert;
                KeyResult::Continue
            }

            // 'a': append after cursor
            (KeyCode::Char('a'), KeyModifiers::NONE) => {
                let (_, cur_col, ranges) = state.line_info();
                let (cur_line, _, _) = state.line_info();
                let (_, len) = ranges[cur_line];
                if cur_col < len {
                    *state.cursor_pos += 1;
                }
                self.vi_mode = ViMode::Insert;
                KeyResult::Continue
            }

            // 'A': append at end of current line
            (KeyCode::Char('A'), KeyModifiers::SHIFT) => {
                state.move_to_eol();
                self.vi_mode = ViMode::Insert;
                KeyResult::Continue
            }

            // 'o': open newline below and enter Insert
            (KeyCode::Char('o'), KeyModifiers::NONE) => {
                self.snapshot_undo(state.buffer, *state.cursor_pos);
                state.move_to_eol();
                state.insert_char('\n');
                self.vi_mode = ViMode::Insert;
                KeyResult::Continue
            }

            // 'O': open newline above and enter Insert
            (KeyCode::Char('O'), KeyModifiers::SHIFT) => {
                self.snapshot_undo(state.buffer, *state.cursor_pos);
                state.move_to_bol();
                state.insert_char('\n');
                // Move cursor back to the empty line above
                if *state.cursor_pos > 0 {
                    *state.cursor_pos -= 1;
                }
                self.vi_mode = ViMode::Insert;
                KeyResult::Continue
            }

            // 's': substitute char under cursor
            (KeyCode::Char('s'), KeyModifiers::NONE) => {
                if !state.is_empty() {
                    self.snapshot_undo(state.buffer, *state.cursor_pos);
                    state.remove_char_at_cursor();
                }
                self.vi_mode = ViMode::Insert;
                KeyResult::Continue
            }

            // 'S': substitute entire line (clear line and enter Insert)
            (KeyCode::Char('S'), KeyModifiers::SHIFT) => {
                self.snapshot_undo(state.buffer, *state.cursor_pos);
                let (start, len) = state.current_line_range();
                let drained = state.drain_range(start, start + len);
                self.push_kill(drained);
                self.vi_mode = ViMode::Insert;
                KeyResult::Continue
            }

            // 'C': change to end of line
            (KeyCode::Char('C'), KeyModifiers::SHIFT) => {
                self.snapshot_undo(state.buffer, *state.cursor_pos);
                let (start, len) = state.current_line_range();
                let drained = state.drain_range(*state.cursor_pos, start + len);
                self.push_kill(drained);
                self.vi_mode = ViMode::Insert;
                KeyResult::Continue
            }

            // 'D': delete to end of line
            (KeyCode::Char('D'), KeyModifiers::SHIFT) => {
                self.snapshot_undo(state.buffer, *state.cursor_pos);
                let (start, len) = state.current_line_range();
                let drained = state.drain_range(*state.cursor_pos, start + len);
                self.push_kill(drained);
                // Clamp cursor to last char of line
                let (_, new_col, new_ranges) = state.line_info();
                let (cur_line, _, _) = state.line_info();
                let (s, l) = new_ranges[cur_line];
                if l > 0 && new_col >= l {
                    *state.cursor_pos = s + l - 1;
                }
                KeyResult::Continue
            }

            // ---- Single-key editing ----
            // 'x': delete character under cursor into kill ring
            (KeyCode::Char('x'), KeyModifiers::NONE) => {
                if *state.cursor_pos < state.buffer.len() {
                    self.snapshot_undo(state.buffer, *state.cursor_pos);
                    if let Some(c) = state.remove_char_at_cursor() {
                        self.push_kill(c.to_string());
                    }
                    // Keep cursor on valid character if at EOL
                    let (cur_line, cur_col, ranges) = state.line_info();
                    let (start, len) = ranges[cur_line];
                    if len > 0 && cur_col >= len {
                        *state.cursor_pos = start + len - 1;
                    }
                }
                KeyResult::Continue
            }

            // 'X': delete character before cursor
            (KeyCode::Char('X'), KeyModifiers::SHIFT) => {
                let (_, cur_col, _) = state.line_info();
                if cur_col > 0 && *state.cursor_pos > 0 {
                    self.snapshot_undo(state.buffer, *state.cursor_pos);
                    if let Some(c) = state.remove_char_before_cursor() {
                        self.push_kill(c.to_string());
                    }
                }
                KeyResult::Continue
            }

            // '~': toggle case of char under cursor and advance cursor
            (KeyCode::Char('~'), KeyModifiers::NONE | KeyModifiers::SHIFT) => {
                if *state.cursor_pos < state.buffer.len() {
                    let c = state.buffer[*state.cursor_pos];
                    if c != '\n' {
                        self.snapshot_undo(state.buffer, *state.cursor_pos);
                        let toggled = if c.is_ascii_uppercase() {
                            c.to_ascii_lowercase()
                        } else if c.is_ascii_lowercase() {
                            c.to_ascii_uppercase()
                        } else {
                            c
                        };
                        state.buffer[*state.cursor_pos] = toggled;
                        let (_, cur_col, ranges) = state.line_info();
                        let (cur_line, _, _) = state.line_info();
                        let (_, len) = ranges[cur_line];
                        if cur_col + 1 < len {
                            *state.cursor_pos += 1;
                        }
                    }
                }
                KeyResult::Continue
            }

            // 'p': paste after cursor
            (KeyCode::Char('p'), KeyModifiers::NONE) => {
                if let Some(text) = self.kill_ring.last() {
                    let text = text.clone();
                    self.snapshot_undo(state.buffer, *state.cursor_pos);
                    if text.ends_with('\n') {
                        // Linewise paste below
                        state.move_to_eol();
                        state.insert_char('\n');
                        state.insert_str(text.trim_end_matches('\n'));
                    } else {
                        // Inline paste after current character
                        let (_, cur_col, ranges) = state.line_info();
                        let (cur_line, _, _) = state.line_info();
                        let (_, len) = ranges[cur_line];
                        if cur_col < len {
                            *state.cursor_pos += 1;
                        }
                        state.insert_str(&text);
                    }
                }
                KeyResult::Continue
            }

            // 'P': paste before cursor
            (KeyCode::Char('P'), KeyModifiers::SHIFT) => {
                if let Some(text) = self.kill_ring.last() {
                    let text = text.clone();
                    self.snapshot_undo(state.buffer, *state.cursor_pos);
                    if text.ends_with('\n') {
                        // Linewise paste above
                        state.move_to_bol();
                        state.insert_str(&text);
                    } else {
                        // Inline paste before cursor
                        state.insert_str(&text);
                    }
                }
                KeyResult::Continue
            }

            // 'u': undo
            (KeyCode::Char('u'), KeyModifiers::NONE) => {
                self.undo(state);
                KeyResult::Continue
            }

            // Ctrl+R: redo
            (KeyCode::Char('r'), KeyModifiers::CONTROL) => {
                self.redo(state);
                KeyResult::Continue
            }

            // ---- Operator prefixes ----
            // 'd': delete operator prefix
            (KeyCode::Char('d'), KeyModifiers::NONE) => {
                self.vi_pending = Some(ViPendingOp::Delete);
                KeyResult::Continue
            }

            // 'c': change operator prefix
            (KeyCode::Char('c'), KeyModifiers::NONE) => {
                self.vi_pending = Some(ViPendingOp::Change);
                KeyResult::Continue
            }

            // 'y': yank operator prefix
            (KeyCode::Char('y'), KeyModifiers::NONE) => {
                self.vi_pending = Some(ViPendingOp::Yank);
                KeyResult::Continue
            }

            // 'r': replace single char prefix
            (KeyCode::Char('r'), KeyModifiers::NONE) => {
                self.vi_pending = Some(ViPendingOp::Replace);
                KeyResult::Continue
            }

            // 'g': goto prefix (for gg)
            (KeyCode::Char('g'), KeyModifiers::NONE) => {
                self.vi_pending = Some(ViPendingOp::Goto);
                KeyResult::Continue
            }

            // 'G': go to end of buffer
            (KeyCode::Char('G'), KeyModifiers::SHIFT) => {
                *state.cursor_pos = state.buffer.len();
                let (_, cur_col, ranges) = state.line_info();
                let (cur_line, _, _) = state.line_info();
                let (start, len) = ranges[cur_line];
                if len > 0 && cur_col >= len {
                    *state.cursor_pos = start + len - 1;
                }
                KeyResult::Continue
            }

            // ---- Motions ----
            // 'h' / Left: move left within line
            (KeyCode::Char('h'), KeyModifiers::NONE) | (KeyCode::Left, _) => {
                let (_, cur_col, _) = state.line_info();
                if cur_col > 0 && *state.cursor_pos > 0 {
                    *state.cursor_pos -= 1;
                }
                KeyResult::Continue
            }

            // 'l' / Right: move right within line
            (KeyCode::Char('l'), KeyModifiers::NONE) | (KeyCode::Right, _) => {
                let (cur_line, cur_col, ranges) = state.line_info();
                let (_, len) = ranges[cur_line];
                if len > 0 && cur_col + 1 < len {
                    *state.cursor_pos += 1;
                }
                KeyResult::Continue
            }

            // 'k' / Up: line up / history up
            (KeyCode::Char('k'), KeyModifiers::NONE) | (KeyCode::Up, _) => {
                state.move_up_or_history();
                // Clamp within line length
                let (cur_line, cur_col, ranges) = state.line_info();
                let (start, len) = ranges[cur_line];
                if len > 0 && cur_col >= len {
                    *state.cursor_pos = start + len - 1;
                }
                KeyResult::Continue
            }

            // 'j' / Down: line down / history down
            (KeyCode::Char('j'), KeyModifiers::NONE) | (KeyCode::Down, _) => {
                state.move_down_or_history();
                let (cur_line, cur_col, ranges) = state.line_info();
                let (start, len) = ranges[cur_line];
                if len > 0 && cur_col >= len {
                    *state.cursor_pos = start + len - 1;
                }
                KeyResult::Continue
            }

            // '0' / Home: beginning of line
            (KeyCode::Char('0'), KeyModifiers::NONE) | (KeyCode::Home, _) => {
                state.move_to_bol();
                KeyResult::Continue
            }

            // '^': first non-whitespace character of line
            (KeyCode::Char('^'), KeyModifiers::NONE | KeyModifiers::SHIFT) => {
                state.move_to_first_non_whitespace();
                KeyResult::Continue
            }

            // '$' / End: end of line
            (KeyCode::Char('$'), KeyModifiers::NONE | KeyModifiers::SHIFT) | (KeyCode::End, _) => {
                let (cur_line, _, ranges) = state.line_info();
                let (start, len) = ranges[cur_line];
                if len > 0 {
                    *state.cursor_pos = start + len - 1;
                } else {
                    *state.cursor_pos = start;
                }
                KeyResult::Continue
            }

            // 'w': next word start
            (KeyCode::Char('w'), KeyModifiers::NONE) => {
                *state.cursor_pos = state.next_word_pos();
                KeyResult::Continue
            }

            // 'b': previous word start
            (KeyCode::Char('b'), KeyModifiers::NONE) => {
                *state.cursor_pos = state.prev_word_pos();
                KeyResult::Continue
            }

            // 'e': next word end
            (KeyCode::Char('e'), KeyModifiers::NONE) => {
                *state.cursor_pos = state.next_word_end_pos();
                KeyResult::Continue
            }

            _ => KeyResult::Noop,
        }
    }

    /// Handle multi-key operations (dd, dw, cc, cw, yy, yw, r<char>, gg).
    fn handle_vi_pending_op(
        &mut self,
        op: ViPendingOp,
        key: KeyEvent,
        state: &mut PromptState,
    ) -> KeyResult {
        self.vi_pending = None;

        match op {
            ViPendingOp::Delete => match (key.code, key.modifiers) {
                // 'dd': delete entire line
                (KeyCode::Char('d'), KeyModifiers::NONE) => {
                    self.snapshot_undo(state.buffer, *state.cursor_pos);
                    let (cur_line, _, ranges) = state.line_info();
                    let (start, len) = ranges[cur_line];
                    let delete_end = if cur_line + 1 < ranges.len() {
                        start + len + 1 // include newline
                    } else {
                        start + len
                    };
                    let mut drained = state.drain_range(start, delete_end);
                    if !drained.ends_with('\n') {
                        drained.push('\n');
                    }
                    self.push_kill(drained);
                    // Adjust cursor
                    let (new_line, _, new_ranges) = state.line_info();
                    let (s, l) = new_ranges[new_line];
                    if l > 0 {
                        *state.cursor_pos = s;
                    }
                    KeyResult::Continue
                }

                // 'dw': delete word forward
                (KeyCode::Char('w'), KeyModifiers::NONE) => {
                    let next = state.next_word_pos();
                    if next > *state.cursor_pos {
                        self.snapshot_undo(state.buffer, *state.cursor_pos);
                        let drained = state.drain_range(*state.cursor_pos, next);
                        self.push_kill(drained);
                    }
                    KeyResult::Continue
                }

                // 'db': delete word backward
                (KeyCode::Char('b'), KeyModifiers::NONE) => {
                    let prev = state.prev_word_pos();
                    if prev < *state.cursor_pos {
                        self.snapshot_undo(state.buffer, *state.cursor_pos);
                        let drained = state.drain_range(prev, *state.cursor_pos);
                        self.push_kill(drained);
                    }
                    KeyResult::Continue
                }

                // 'd$': delete to end of line
                (KeyCode::Char('$'), KeyModifiers::NONE | KeyModifiers::SHIFT) => {
                    self.snapshot_undo(state.buffer, *state.cursor_pos);
                    let (start, len) = state.current_line_range();
                    let drained = state.drain_range(*state.cursor_pos, start + len);
                    self.push_kill(drained);
                    KeyResult::Continue
                }

                // 'd0': delete to start of line
                (KeyCode::Char('0'), KeyModifiers::NONE) => {
                    self.snapshot_undo(state.buffer, *state.cursor_pos);
                    let (start, _) = state.current_line_range();
                    let drained = state.drain_range(start, *state.cursor_pos);
                    self.push_kill(drained);
                    KeyResult::Continue
                }

                _ => KeyResult::Noop,
            },

            ViPendingOp::Change => match (key.code, key.modifiers) {
                // 'cc': clear entire line and enter Insert mode
                (KeyCode::Char('c'), KeyModifiers::NONE) => {
                    self.snapshot_undo(state.buffer, *state.cursor_pos);
                    let (start, len) = state.current_line_range();
                    let drained = state.drain_range(start, start + len);
                    self.push_kill(drained);
                    self.vi_mode = ViMode::Insert;
                    KeyResult::Continue
                }

                // 'cw': change word forward
                (KeyCode::Char('w'), KeyModifiers::NONE) => {
                    let next = state.next_word_end_pos();
                    let target = if next > *state.cursor_pos {
                        next + 1
                    } else {
                        state.next_word_pos()
                    };
                    if target > *state.cursor_pos {
                        self.snapshot_undo(state.buffer, *state.cursor_pos);
                        let drained = state.drain_range(*state.cursor_pos, target);
                        self.push_kill(drained);
                    }
                    self.vi_mode = ViMode::Insert;
                    KeyResult::Continue
                }

                // 'cb': change word backward
                (KeyCode::Char('b'), KeyModifiers::NONE) => {
                    let prev = state.prev_word_pos();
                    if prev < *state.cursor_pos {
                        self.snapshot_undo(state.buffer, *state.cursor_pos);
                        let drained = state.drain_range(prev, *state.cursor_pos);
                        self.push_kill(drained);
                    }
                    self.vi_mode = ViMode::Insert;
                    KeyResult::Continue
                }

                // 'c$': change to end of line
                (KeyCode::Char('$'), KeyModifiers::NONE | KeyModifiers::SHIFT) => {
                    self.snapshot_undo(state.buffer, *state.cursor_pos);
                    let (start, len) = state.current_line_range();
                    let drained = state.drain_range(*state.cursor_pos, start + len);
                    self.push_kill(drained);
                    self.vi_mode = ViMode::Insert;
                    KeyResult::Continue
                }

                _ => KeyResult::Noop,
            },

            ViPendingOp::Yank => match (key.code, key.modifiers) {
                // 'yy': yank current line
                (KeyCode::Char('y'), KeyModifiers::NONE) => {
                    let (start, len) = state.current_line_range();
                    let line_text: String = state.buffer[start..start + len].iter().collect();
                    self.push_kill(format!("{}\n", line_text));
                    KeyResult::Continue
                }

                // 'yw': yank word forward
                (KeyCode::Char('w'), KeyModifiers::NONE) => {
                    let next = state.next_word_pos();
                    if next > *state.cursor_pos {
                        let word: String = state.buffer[*state.cursor_pos..next].iter().collect();
                        self.push_kill(word);
                    }
                    KeyResult::Continue
                }

                // 'yb': yank word backward
                (KeyCode::Char('b'), KeyModifiers::NONE) => {
                    let prev = state.prev_word_pos();
                    if prev < *state.cursor_pos {
                        let word: String = state.buffer[prev..*state.cursor_pos].iter().collect();
                        self.push_kill(word);
                    }
                    KeyResult::Continue
                }

                // 'y$': yank to end of line
                (KeyCode::Char('$'), KeyModifiers::NONE | KeyModifiers::SHIFT) => {
                    let (start, len) = state.current_line_range();
                    let text: String = state.buffer[*state.cursor_pos..start + len]
                        .iter()
                        .collect();
                    self.push_kill(text);
                    KeyResult::Continue
                }

                _ => KeyResult::Noop,
            },

            ViPendingOp::Replace => {
                if let KeyCode::Char(c) = key.code {
                    if *state.cursor_pos < state.buffer.len()
                        && state.buffer[*state.cursor_pos] != '\n'
                    {
                        self.snapshot_undo(state.buffer, *state.cursor_pos);
                        state.buffer[*state.cursor_pos] = c;
                    }
                }
                KeyResult::Continue
            }

            ViPendingOp::Goto => match (key.code, key.modifiers) {
                // 'gg': go to beginning of buffer
                (KeyCode::Char('g'), KeyModifiers::NONE) => {
                    *state.cursor_pos = 0;
                    KeyResult::Continue
                }
                _ => KeyResult::Noop,
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_test_state<'a>(
        buffer: &'a mut Vec<char>,
        cursor_pos: &'a mut usize,
        history: &'a [String],
        history_idx: &'a mut Option<usize>,
        saved_current: &'a mut String,
    ) -> PromptState<'a> {
        PromptState::new(buffer, cursor_pos, history, history_idx, saved_current)
    }

    #[test]
    fn test_profile_from_str() {
        assert_eq!(
            KeybindingProfile::from_str("default").unwrap(),
            KeybindingProfile::Default
        );
        assert_eq!(
            KeybindingProfile::from_str("emacs").unwrap(),
            KeybindingProfile::Emacs
        );
        assert_eq!(
            KeybindingProfile::from_str("vi").unwrap(),
            KeybindingProfile::Vi
        );
        assert_eq!(
            KeybindingProfile::from_str("vim").unwrap(),
            KeybindingProfile::Vi
        );
        assert!(KeybindingProfile::from_str("unknown").is_err());
    }

    #[test]
    fn test_default_typing_and_editing() {
        let mut handler = KeyHandler::new(KeybindingProfile::Default);
        let mut buf = Vec::new();
        let mut cur = 0;
        let hist = Vec::new();
        let mut hist_idx = None;
        let mut saved = String::new();

        {
            let mut st = make_test_state(&mut buf, &mut cur, &hist, &mut hist_idx, &mut saved);
            // Type "hi"
            handler.handle_key(
                KeyEvent::new(KeyCode::Char('h'), KeyModifiers::NONE),
                &mut st,
            );
            handler.handle_key(
                KeyEvent::new(KeyCode::Char('i'), KeyModifiers::NONE),
                &mut st,
            );
        }
        assert_eq!(buf, vec!['h', 'i']);
        assert_eq!(cur, 2);

        {
            let mut st = make_test_state(&mut buf, &mut cur, &hist, &mut hist_idx, &mut saved);
            // Backspace
            handler.handle_key(
                KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE),
                &mut st,
            );
        }
        assert_eq!(buf, vec!['h']);
        assert_eq!(cur, 1);

        {
            let mut st = make_test_state(&mut buf, &mut cur, &hist, &mut hist_idx, &mut saved);
            // Submit
            let res =
                handler.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE), &mut st);
            assert_eq!(res, KeyResult::Submit("h".to_string()));
        }
    }

    #[test]
    fn test_emacs_keybindings() {
        let mut handler = KeyHandler::new(KeybindingProfile::Emacs);
        let mut buf: Vec<char> = "hello world".chars().collect();
        let mut cur = 11;
        let hist = Vec::new();
        let mut hist_idx = None;
        let mut saved = String::new();

        {
            let mut st = make_test_state(&mut buf, &mut cur, &hist, &mut hist_idx, &mut saved);
            // Ctrl+A -> beginning of line
            handler.handle_key(
                KeyEvent::new(KeyCode::Char('a'), KeyModifiers::CONTROL),
                &mut st,
            );
            assert_eq!(*st.cursor_pos, 0);

            // Alt+F -> forward one word
            handler.handle_key(
                KeyEvent::new(KeyCode::Char('f'), KeyModifiers::ALT),
                &mut st,
            );
            assert_eq!(*st.cursor_pos, 6);

            // Alt+B -> backward one word
            handler.handle_key(
                KeyEvent::new(KeyCode::Char('b'), KeyModifiers::ALT),
                &mut st,
            );
            assert_eq!(*st.cursor_pos, 0);

            // Ctrl+K -> kill line to end
            handler.handle_key(
                KeyEvent::new(KeyCode::Char('k'), KeyModifiers::CONTROL),
                &mut st,
            );
            assert_eq!(st.text(), "");

            // Ctrl+Y -> yank (paste killed line)
            handler.handle_key(
                KeyEvent::new(KeyCode::Char('y'), KeyModifiers::CONTROL),
                &mut st,
            );
            assert_eq!(st.text(), "hello world");
        }
    }

    #[test]
    fn test_emacs_case_transforms() {
        let mut handler = KeyHandler::new(KeybindingProfile::Emacs);
        let mut buf: Vec<char> = "foo bar baz".chars().collect();
        let mut cur = 0;
        let hist = Vec::new();
        let mut hist_idx = None;
        let mut saved = String::new();

        {
            let mut st = make_test_state(&mut buf, &mut cur, &hist, &mut hist_idx, &mut saved);
            // Alt+U -> uppercase word
            handler.handle_key(
                KeyEvent::new(KeyCode::Char('u'), KeyModifiers::ALT),
                &mut st,
            );
            assert_eq!(st.text(), "FOO bar baz");

            // Alt+C -> capitalize word
            handler.handle_key(
                KeyEvent::new(KeyCode::Char('c'), KeyModifiers::ALT),
                &mut st,
            );
            assert_eq!(st.text(), "FOO Bar baz");
        }
    }

    #[test]
    fn test_vi_mode_switch_and_navigation() {
        let mut handler = KeyHandler::new(KeybindingProfile::Vi);
        assert_eq!(handler.vi_mode(), ViMode::Insert);

        let mut buf: Vec<char> = "hello world".chars().collect();
        let mut cur = 11;
        let hist = Vec::new();
        let mut hist_idx = None;
        let mut saved = String::new();

        {
            let mut st = make_test_state(&mut buf, &mut cur, &hist, &mut hist_idx, &mut saved);
            // Press Esc to switch to Normal mode
            handler.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE), &mut st);
            assert_eq!(handler.vi_mode(), ViMode::Normal);
            // Cursor should step back one to char 'd' (pos 10)
            assert_eq!(*st.cursor_pos, 10);

            // '0' -> start of line
            handler.handle_key(
                KeyEvent::new(KeyCode::Char('0'), KeyModifiers::NONE),
                &mut st,
            );
            assert_eq!(*st.cursor_pos, 0);

            // 'w' -> next word
            handler.handle_key(
                KeyEvent::new(KeyCode::Char('w'), KeyModifiers::NONE),
                &mut st,
            );
            assert_eq!(*st.cursor_pos, 6);

            // 'b' -> previous word
            handler.handle_key(
                KeyEvent::new(KeyCode::Char('b'), KeyModifiers::NONE),
                &mut st,
            );
            assert_eq!(*st.cursor_pos, 0);

            // '$' -> end of line (last char pos = 10)
            handler.handle_key(
                KeyEvent::new(KeyCode::Char('$'), KeyModifiers::NONE),
                &mut st,
            );
            assert_eq!(*st.cursor_pos, 10);

            // 'i' -> enter Insert mode
            handler.handle_key(
                KeyEvent::new(KeyCode::Char('i'), KeyModifiers::NONE),
                &mut st,
            );
            assert_eq!(handler.vi_mode(), ViMode::Insert);
        }
    }

    #[test]
    fn test_vi_delete_and_yank() {
        let mut handler = KeyHandler::new(KeybindingProfile::Vi);
        handler.set_vi_mode(ViMode::Normal);

        let mut buf: Vec<char> = "hello world".chars().collect();
        let mut cur = 0;
        let hist = Vec::new();
        let mut hist_idx = None;
        let mut saved = String::new();

        {
            let mut st = make_test_state(&mut buf, &mut cur, &hist, &mut hist_idx, &mut saved);
            // 'dw' -> delete word forward
            handler.handle_key(
                KeyEvent::new(KeyCode::Char('d'), KeyModifiers::NONE),
                &mut st,
            );
            handler.handle_key(
                KeyEvent::new(KeyCode::Char('w'), KeyModifiers::NONE),
                &mut st,
            );
            assert_eq!(st.text(), "world");

            // 'u' -> undo
            handler.handle_key(
                KeyEvent::new(KeyCode::Char('u'), KeyModifiers::NONE),
                &mut st,
            );
            assert_eq!(st.text(), "hello world");

            // 'x' -> delete character 'h'
            handler.handle_key(
                KeyEvent::new(KeyCode::Char('x'), KeyModifiers::NONE),
                &mut st,
            );
            assert_eq!(st.text(), "ello world");

            // 'r' followed by 'H' -> replace char under cursor
            handler.handle_key(
                KeyEvent::new(KeyCode::Char('r'), KeyModifiers::NONE),
                &mut st,
            );
            handler.handle_key(
                KeyEvent::new(KeyCode::Char('H'), KeyModifiers::NONE),
                &mut st,
            );
            assert_eq!(st.text(), "Hllo world");
        }
    }

    #[test]
    fn test_vi_line_deletion_and_paste() {
        let mut handler = KeyHandler::new(KeybindingProfile::Vi);
        handler.set_vi_mode(ViMode::Normal);

        let mut buf: Vec<char> = "first\nsecond".chars().collect();
        let mut cur = 0;
        let hist = Vec::new();
        let mut hist_idx = None;
        let mut saved = String::new();

        {
            let mut st = make_test_state(&mut buf, &mut cur, &hist, &mut hist_idx, &mut saved);
            // 'dd' -> delete line
            handler.handle_key(
                KeyEvent::new(KeyCode::Char('d'), KeyModifiers::NONE),
                &mut st,
            );
            handler.handle_key(
                KeyEvent::new(KeyCode::Char('d'), KeyModifiers::NONE),
                &mut st,
            );
            assert_eq!(st.text(), "second");

            // 'p' -> paste linewise below
            handler.handle_key(
                KeyEvent::new(KeyCode::Char('p'), KeyModifiers::NONE),
                &mut st,
            );
            assert_eq!(st.text(), "second\nfirst");
        }
    }

    #[test]
    fn test_emacs_transposition() {
        let mut handler = KeyHandler::new(KeybindingProfile::Emacs);
        let mut buf: Vec<char> = "ab".chars().collect();
        let mut cur = 2;
        let hist = Vec::new();
        let mut hist_idx = None;
        let mut saved = String::new();

        {
            let mut st = make_test_state(&mut buf, &mut cur, &hist, &mut hist_idx, &mut saved);
            // Ctrl+T at EOL swaps 'a' and 'b'
            handler.handle_key(
                KeyEvent::new(KeyCode::Char('t'), KeyModifiers::CONTROL),
                &mut st,
            );
            assert_eq!(st.text(), "ba");
        }
    }

    #[test]
    fn test_vi_insert_mode_variants() {
        let mut handler = KeyHandler::new(KeybindingProfile::Vi);
        handler.set_vi_mode(ViMode::Normal);

        let mut buf: Vec<char> = "hello".chars().collect();
        let mut cur = 0;
        let hist = Vec::new();
        let mut hist_idx = None;
        let mut saved = String::new();

        {
            let mut st = make_test_state(&mut buf, &mut cur, &hist, &mut hist_idx, &mut saved);
            // 'A' -> append at EOL
            handler.handle_key(
                KeyEvent::new(KeyCode::Char('A'), KeyModifiers::SHIFT),
                &mut st,
            );
            assert_eq!(handler.vi_mode(), ViMode::Insert);
            assert_eq!(*st.cursor_pos, 5);

            // Type " world"
            handler.handle_key(
                KeyEvent::new(KeyCode::Char(' '), KeyModifiers::NONE),
                &mut st,
            );
            handler.handle_key(
                KeyEvent::new(KeyCode::Char('w'), KeyModifiers::NONE),
                &mut st,
            );
            assert_eq!(st.text(), "hello w");
        }
    }

    #[test]
    fn test_vi_open_line_and_substitute() {
        let mut handler = KeyHandler::new(KeybindingProfile::Vi);
        handler.set_vi_mode(ViMode::Normal);

        let mut buf: Vec<char> = "line1".chars().collect();
        let mut cur = 0;
        let hist = Vec::new();
        let mut hist_idx = None;
        let mut saved = String::new();

        {
            let mut st = make_test_state(&mut buf, &mut cur, &hist, &mut hist_idx, &mut saved);
            // 'o' -> open line below and enter insert
            handler.handle_key(
                KeyEvent::new(KeyCode::Char('o'), KeyModifiers::NONE),
                &mut st,
            );
            assert_eq!(handler.vi_mode(), ViMode::Insert);
            assert_eq!(st.text(), "line1\n");
            assert_eq!(*st.cursor_pos, 6);
        }
    }

    #[test]
    fn test_vi_change_operations() {
        let mut handler = KeyHandler::new(KeybindingProfile::Vi);
        handler.set_vi_mode(ViMode::Normal);

        let mut buf: Vec<char> = "foo bar".chars().collect();
        let mut cur = 0;
        let hist = Vec::new();
        let mut hist_idx = None;
        let mut saved = String::new();

        {
            let mut st = make_test_state(&mut buf, &mut cur, &hist, &mut hist_idx, &mut saved);
            // 'cw' -> change word
            handler.handle_key(
                KeyEvent::new(KeyCode::Char('c'), KeyModifiers::NONE),
                &mut st,
            );
            handler.handle_key(
                KeyEvent::new(KeyCode::Char('w'), KeyModifiers::NONE),
                &mut st,
            );
            assert_eq!(handler.vi_mode(), ViMode::Insert);
            assert_eq!(st.text(), " bar");
            assert_eq!(*st.cursor_pos, 0);
        }
    }

    #[test]
    fn test_vi_case_toggle_and_word_end() {
        let mut handler = KeyHandler::new(KeybindingProfile::Vi);
        handler.set_vi_mode(ViMode::Normal);

        let mut buf: Vec<char> = "abc def".chars().collect();
        let mut cur = 0;
        let hist = Vec::new();
        let mut hist_idx = None;
        let mut saved = String::new();

        {
            let mut st = make_test_state(&mut buf, &mut cur, &hist, &mut hist_idx, &mut saved);
            // '~' -> toggle case 'a' -> 'A'
            handler.handle_key(
                KeyEvent::new(KeyCode::Char('~'), KeyModifiers::NONE),
                &mut st,
            );
            assert_eq!(st.text(), "Abc def");
            assert_eq!(*st.cursor_pos, 1);

            // 'e' -> jump to next word end
            handler.handle_key(
                KeyEvent::new(KeyCode::Char('e'), KeyModifiers::NONE),
                &mut st,
            );
            assert_eq!(*st.cursor_pos, 2);
        }
    }

    #[test]
    fn test_vi_goto_and_indicators() {
        let mut handler = KeyHandler::new(KeybindingProfile::Vi);
        assert_eq!(handler.mode_indicator(), Some("\x1b[1;32m[INS]\x1b[0m"));

        handler.set_vi_mode(ViMode::Normal);
        assert_eq!(handler.mode_indicator(), Some("\x1b[1;33m[NOR]\x1b[0m"));
        assert_eq!(ViMode::Normal.indicator(), "[NOR]");
        assert_eq!(ViMode::Insert.indicator(), "[INS]");

        let mut buf: Vec<char> = "line1\nline2\nline3".chars().collect();
        let mut cur = 12;
        let hist = Vec::new();
        let mut hist_idx = None;
        let mut saved = String::new();

        {
            let mut st = make_test_state(&mut buf, &mut cur, &hist, &mut hist_idx, &mut saved);
            // 'gg' -> jump to start
            handler.handle_key(
                KeyEvent::new(KeyCode::Char('g'), KeyModifiers::NONE),
                &mut st,
            );
            handler.handle_key(
                KeyEvent::new(KeyCode::Char('g'), KeyModifiers::NONE),
                &mut st,
            );
            assert_eq!(*st.cursor_pos, 0);

            // 'G' -> jump to end
            handler.handle_key(
                KeyEvent::new(KeyCode::Char('G'), KeyModifiers::SHIFT),
                &mut st,
            );
            assert_eq!(*st.cursor_pos, 16);
        }
    }

    #[test]
    fn test_default_multiline_backslash() {
        let mut handler = KeyHandler::new(KeybindingProfile::Default);
        let mut buf: Vec<char> = "first line\\".chars().collect();
        let mut cur = 11;
        let hist = Vec::new();
        let mut hist_idx = None;
        let mut saved = String::new();

        {
            let mut st = make_test_state(&mut buf, &mut cur, &hist, &mut hist_idx, &mut saved);
            // Enter when trailing backslash -> turns into newline
            let res =
                handler.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE), &mut st);
            assert_eq!(res, KeyResult::Continue);
            assert_eq!(st.text(), "first line\n");
        }
    }

    #[test]
    fn test_default_ctrl_u_and_ctrl_k() {
        let mut handler = KeyHandler::new(KeybindingProfile::Default);
        let mut buf: Vec<char> = "hello world".chars().collect();
        let mut cur = 5;
        let hist = Vec::new();
        let mut hist_idx = None;
        let mut saved = String::new();

        {
            let mut st = make_test_state(&mut buf, &mut cur, &hist, &mut hist_idx, &mut saved);
            // Ctrl+K -> kills from cursor to EOL
            handler.handle_key(
                KeyEvent::new(KeyCode::Char('k'), KeyModifiers::CONTROL),
                &mut st,
            );
            assert_eq!(st.text(), "hello");

            // Ctrl+U -> kills from cursor to BOL
            handler.handle_key(
                KeyEvent::new(KeyCode::Char('u'), KeyModifiers::CONTROL),
                &mut st,
            );
            assert_eq!(st.text(), "");
        }
    }

    #[test]
    fn test_history_navigation_multiline() {
        let mut handler = KeyHandler::new(KeybindingProfile::Default);
        let mut buf: Vec<char> = "first\nsecond".chars().collect();
        let mut cur = 8; // on "second" at 'c' (col 2)
        let hist = vec!["prev command".to_string()];
        let mut hist_idx = None;
        let mut saved = String::new();

        {
            let mut st = make_test_state(&mut buf, &mut cur, &hist, &mut hist_idx, &mut saved);
            // Up arrow moves from second line to first line
            handler.handle_key(KeyEvent::new(KeyCode::Up, KeyModifiers::NONE), &mut st);
            assert_eq!(*st.cursor_pos, 2); // on "first" at 'r' (col 2)

            // Up arrow again (now on line 0) recalls history
            handler.handle_key(KeyEvent::new(KeyCode::Up, KeyModifiers::NONE), &mut st);
            assert_eq!(st.text(), "prev command");

            // Down arrow restores saved buffer
            handler.handle_key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE), &mut st);
            assert_eq!(st.text(), "first\nsecond");
        }
    }

    #[test]
    fn test_custom_keymap_override() {
        let mut keymap = crate::ui::keymap_config::KeymapConfig::default();
        keymap
            .bind("ctrl+s", crate::ui::keymap_config::KeyAction::Submit)
            .unwrap();
        let mut handler = KeyHandler::new(KeybindingProfile::Default).with_keymap(keymap);
        let mut buf: Vec<char> = "hello".chars().collect();
        let mut cur = 5;
        let hist = Vec::new();
        let mut hist_idx = None;
        let mut saved = String::new();

        let mut st = make_test_state(&mut buf, &mut cur, &hist, &mut hist_idx, &mut saved);
        let res = handler.handle_key(
            KeyEvent::new(KeyCode::Char('s'), KeyModifiers::CONTROL),
            &mut st,
        );
        assert_eq!(res, KeyResult::Submit("hello".to_string()));
    }
}
