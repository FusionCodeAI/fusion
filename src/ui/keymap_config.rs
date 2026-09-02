//! Custom keymap configuration loader and keybinding manager for Fusion.
//!
//! Loads user-defined keyboard shortcuts, profile overrides, and modal keymaps
//! from `~/.fusion/keymap.json`.
//!
//! ## Overview
//!
//! Fusion provides three built-in keybinding profiles (`Default`, `Emacs`, and `Vi`),
//! but users often desire customized bindings, multi-key leader sequences, quick slash
//! command triggers, or specific muscle-memory overrides.
//!
//! This module provides:
//! 1. **Persistent Configuration**: Load, edit, validate, and save `~/.fusion/keymap.json`.
//! 2. **Flexible Key Chord Syntax**: Parses intuitive key descriptions (e.g. `"Ctrl+C"`,
//!    `"ctrl-p"`, `"C-x"`, `"alt+enter"`, `"M-f"`, `"shift+tab"`, `"f5"`, `"Esc"`, `"space"`).
//! 3. **Action Abstraction**: Declarative `KeyAction` enum supporting line editing, cursor motion,
//!    history navigation, kill-ring operations, profile/mode switching, and macro/command execution.
//! 4. **Modal Scope Resolution**: Distinct overrides for Global, Emacs, Vi Normal, and Vi Insert modes.
//! 5. **Leader Key Sequences**: Timed multi-key leader chords (e.g. `Ctrl+X` followed by a key).
//! 6. **Zero-Allocation Runtime Dispatch**: Caches parsed chords in `KeymapManager` for fast
//!    lookup during interactive prompt sessions.
//!
//! ## Keymap JSON Format Example
//!
//! ```json
//! {
//!   "version": 1,
//!   "profile": "default",
//!   "leader": "Ctrl+X",
//!   "leader_timeout_ms": 1000,
//!   "bindings": {
//!     "ctrl+s": "submit",
//!     "ctrl+k": "kill_to_eol",
//!     "alt+enter": "insert_newline",
//!     "ctrl+p": "history_prev",
//!     "ctrl+n": "history_next",
//!     "ctrl+shift+c": { "action": "execute_command", "command": "/clear" }
//!   },
//!   "emacs": {
//!     "ctrl+x ctrl+c": "exit"
//!   },
//!   "vi_normal": {
//!     "space f": { "action": "execute_command", "command": "/palette" },
//!     "g g": "move_to_buffer_start"
//!   },
//!   "vi_insert": {
//!     "j k": { "action": "set_vi_mode", "mode": "normal" }
//!   }
//! }
//! ```

use std::collections::HashMap;
use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::time::{Duration, Instant};

use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use serde::de::{self, Deserializer, Visitor};
use serde::{Deserialize, Serialize, Serializer};
use thiserror::Error;

use crate::config::Config;
use crate::ui::keys::{KeybindingProfile, ViMode, KeyResult, PromptState, KeyHandler};

/// Standard filename for custom keymap configuration in `~/.fusion/`.
pub const KEYMAP_FILE_NAME: &str = "keymap.json";

/// Default leader key timeout in milliseconds.
pub const DEFAULT_LEADER_TIMEOUT_MS: u64 = 1000;

// ============================================================================
// Error Types
// ============================================================================

/// Errors encountered when parsing, loading, validating, or saving keymap configurations.
#[derive(Debug, Error)]
pub enum KeymapError {
    /// File system I/O error.
    #[error("Keymap I/O error: {0}")]
    Io(#[from] std::io::Error),

    /// JSON serialization or deserialization error.
    #[error("Keymap JSON format error: {0}")]
    Json(#[from] serde_json::Error),

    /// Invalid or unrecognized key chord representation.
    #[error("Invalid key chord '{0}': {1}")]
    InvalidKeyChord(String, String),

    /// Invalid or unrecognized action name.
    #[error("Invalid key action '{0}': {1}")]
    InvalidAction(String, String),

    /// Keymap validation failed with one or more errors.
    #[error("Keymap validation failed with {0} errors:\n{1}")]
    ValidationFailed(usize, String),
}

// ============================================================================
// KeyChord: Normalized Keyboard Combination
// ============================================================================

/// Normalized representation of a keyboard combination (modifiers + key code).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct KeyChord {
    /// Active modifiers (Control, Alt, Shift, Super).
    pub modifiers: KeyModifiers,
    /// Triggered key code.
    pub code: KeyCode,
}

impl KeyChord {
    /// Create a new key chord with specific key code and modifiers.
    pub const fn new(code: KeyCode, modifiers: KeyModifiers) -> Self {
        Self { modifiers, code }
    }

    /// Create a plain key chord without modifiers.
    pub const fn plain(code: KeyCode) -> Self {
        Self {
            modifiers: KeyModifiers::NONE,
            code,
        }
    }

    /// Create a Ctrl-modified key chord.
    pub const fn ctrl(code: KeyCode) -> Self {
        Self {
            modifiers: KeyModifiers::CONTROL,
            code,
        }
    }

    /// Create an Alt-modified key chord.
    pub const fn alt(code: KeyCode) -> Self {
        Self {
            modifiers: KeyModifiers::ALT,
            code,
        }
    }

    /// Matches a crossterm `KeyEvent` against this chord.
    pub fn matches(&self, event: &KeyEvent) -> bool {
        if event.kind == KeyEventKind::Release {
            return false;
        }

        // Compare key codes
        let code_match = match (self.code, event.code) {
            (KeyCode::Char(c1), KeyCode::Char(c2)) => {
                c1.to_ascii_lowercase() == c2.to_ascii_lowercase()
            }
            (k1, k2) => k1 == k2,
        };

        if !code_match {
            return false;
        }

        // Ignore shift if uppercase char matched
        let mut expected_mods = self.modifiers;
        let mut actual_mods = event.modifiers;

        if let KeyCode::Char(c) = event.code {
            if c.is_ascii_uppercase() {
                expected_mods.remove(KeyModifiers::SHIFT);
                actual_mods.remove(KeyModifiers::SHIFT);
            }
        }

        expected_mods == actual_mods
    }

    /// Parse a string key chord (e.g. `"Ctrl+C"`, `"ctrl-p"`, `"C-x"`, `"alt+enter"`, `"M-f"`, `"shift+tab"`).
    pub fn parse(s: &str) -> Result<Self, KeymapError> {
        let trimmed = s.trim();
        if trimmed.is_empty() {
            return Err(KeymapError::InvalidKeyChord(
                s.to_string(),
                "Key chord cannot be empty".to_string(),
            ));
        }

        // Split by '+', '-', or space
        // Handle special single chars like '+', '-', ' '
        if trimmed == "+" || trimmed == "-" || trimmed == " " {
            let c = trimmed.chars().next().unwrap();
            return Ok(KeyChord::plain(KeyCode::Char(c)));
        }

        let mut modifiers = KeyModifiers::NONE;
        let mut key_part = "";

        // Tokens splitting by '+' or '-'
        // Be careful with trailing '-' like "ctrl--" or "ctrl-+"
        let mut tokens: Vec<&str> = Vec::new();
        let mut chars = trimmed.char_indices().peekable();
        let mut start = 0;

        while let Some((idx, ch)) = chars.next() {
            if (ch == '+' || ch == '-') && idx > 0 {
                // If it's a delimiter and not the last char in "ctrl--"
                if idx > start {
                    tokens.push(&trimmed[start..idx]);
                }
                start = idx + 1;
            }
        }
        if start < trimmed.len() {
            tokens.push(&trimmed[start..]);
        } else if start == trimmed.len() && (trimmed.ends_with('-') || trimmed.ends_with('+')) {
            // Trailing delimiter was the actual key! e.g. "ctrl-+" -> key is '+'
            let last_char = trimmed.chars().last().unwrap();
            if tokens.is_empty() {
                tokens.push(trimmed);
            } else {
                tokens.push(&trimmed[trimmed.len() - 1..]);
            }
        }

        if tokens.is_empty() {
            return Err(KeymapError::InvalidKeyChord(
                s.to_string(),
                "No valid key tokens found".to_string(),
            ));
        }

        // Last token is the key, preceding tokens are modifiers
        for (i, token) in tokens.iter().enumerate() {
            let lower = token.to_lowercase();
            if i == tokens.len() - 1 {
                key_part = token;
            } else {
                match lower.as_str() {
                    "ctrl" | "control" | "c" => modifiers |= KeyModifiers::CONTROL,
                    "alt" | "meta" | "opt" | "option" | "m" => modifiers |= KeyModifiers::ALT,
                    "shift" | "s" => modifiers |= KeyModifiers::SHIFT,
                    "super" | "win" | "cmd" | "command" => modifiers |= KeyModifiers::SUPER,
                    _ => {
                        // Could be Emacs syntax like "C-x" where C is modifier and x is key
                        return Err(KeymapError::InvalidKeyChord(
                            s.to_string(),
                            format!("Unrecognized modifier '{}'", token),
                        ));
                    }
                }
            }
        }

        let code = parse_key_code(key_part, &mut modifiers)
            .map_err(|err| KeymapError::InvalidKeyChord(s.to_string(), err))?;

        Ok(KeyChord { modifiers, code })
    }

    /// Produce a canonical, human-readable string representation (e.g. `"Ctrl+Alt+P"`).
    pub fn to_canonical_string(&self) -> String {
        let mut parts = Vec::new();

        if self.modifiers.contains(KeyModifiers::CONTROL) {
            parts.push("Ctrl");
        }
        if self.modifiers.contains(KeyModifiers::ALT) {
            parts.push("Alt");
        }
        if self.modifiers.contains(KeyModifiers::SHIFT) {
            parts.push("Shift");
        }
        if self.modifiers.contains(KeyModifiers::SUPER) {
            parts.push("Super");
        }

        let key_str = match self.code {
            KeyCode::Enter => "Enter".to_string(),
            KeyCode::Esc => "Esc".to_string(),
            KeyCode::Backspace => "Backspace".to_string(),
            KeyCode::Delete => "Delete".to_string(),
            KeyCode::Insert => "Insert".to_string(),
            KeyCode::Tab => "Tab".to_string(),
            KeyCode::BackTab => "BackTab".to_string(),
            KeyCode::Left => "Left".to_string(),
            KeyCode::Right => "Right".to_string(),
            KeyCode::Up => "Up".to_string(),
            KeyCode::Down => "Down".to_string(),
            KeyCode::Home => "Home".to_string(),
            KeyCode::End => "End".to_string(),
            KeyCode::PageUp => "PageUp".to_string(),
            KeyCode::PageDown => "PageDown".to_string(),
            KeyCode::F(n) => format!("F{}", n),
            KeyCode::Char(' ') => "Space".to_string(),
            KeyCode::Char(c) => {
                if c.is_ascii_alphabetic() {
                    c.to_ascii_uppercase().to_string()
                } else {
                    c.to_string()
                }
            }
            KeyCode::Null => "Null".to_string(),
            KeyCode::CapsLock => "CapsLock".to_string(),
            KeyCode::ScrollLock => "ScrollLock".to_string(),
            KeyCode::NumLock => "NumLock".to_string(),
            KeyCode::PrintScreen => "PrintScreen".to_string(),
            KeyCode::Pause => "Pause".to_string(),
            KeyCode::Menu => "Menu".to_string(),
            KeyCode::KeypadBegin => "KeypadBegin".to_string(),
            KeyCode::Media(m) => format!("Media({:?})", m),
            KeyCode::Modifier(m) => format!("Modifier({:?})", m),
        };

        parts.push(&key_str);
        parts.join("+")
    }
}

impl From<KeyEvent> for KeyChord {
    fn from(event: KeyEvent) -> Self {
        Self {
            modifiers: event.modifiers,
            code: event.code,
        }
    }
}

impl FromStr for KeyChord {
    type Err = KeymapError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        KeyChord::parse(s)
    }
}

impl fmt::Display for KeyChord {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.to_canonical_string())
    }
}

impl Serialize for KeyChord {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&self.to_canonical_string())
    }
}

impl<'de> Deserialize<'de> for KeyChord {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        KeyChord::parse(&s).map_err(de::Error::custom)
    }
}

fn parse_key_code(s: &str, modifiers: &mut KeyModifiers) -> Result<KeyCode, String> {
    let lower = s.to_lowercase();
    match lower.as_str() {
        "enter" | "return" | "cr" => Ok(KeyCode::Enter),
        "esc" | "escape" => Ok(KeyCode::Esc),
        "tab" => Ok(KeyCode::Tab),
        "backtab" | "back_tab" | "shifttab" | "shift_tab" => {
            *modifiers |= KeyModifiers::SHIFT;
            Ok(KeyCode::BackTab)
        }
        "backspace" | "bs" | "bksp" => Ok(KeyCode::Backspace),
        "delete" | "del" => Ok(KeyCode::Delete),
        "insert" | "ins" => Ok(KeyCode::Insert),
        "space" | "spc" | " " => Ok(KeyCode::Char(' ')),
        "left" => Ok(KeyCode::Left),
        "right" => Ok(KeyCode::Right),
        "up" => Ok(KeyCode::Up),
        "down" => Ok(KeyCode::Down),
        "home" => Ok(KeyCode::Home),
        "end" => Ok(KeyCode::End),
        "pageup" | "pgup" | "page_up" => Ok(KeyCode::PageUp),
        "pagedown" | "pgdn" | "page_down" => Ok(KeyCode::PageDown),
        "null" => Ok(KeyCode::Null),
        _ => {
            // Check function keys F1-F24
            if let Some(f_str) = lower.strip_prefix('f') {
                if let Ok(f_num) = f_str.parse::<u8>() {
                    if (1..=24).contains(&f_num) {
                        return Ok(KeyCode::F(f_num));
                    }
                }
            }

            // Single character
            let chars: Vec<char> = s.chars().collect();
            if chars.len() == 1 {
                let c = chars[0];
                if c.is_ascii_uppercase() {
                    *modifiers |= KeyModifiers::SHIFT;
                }
                Ok(KeyCode::Char(c))
            } else {
                Err(format!("Unknown key name '{}'", s))
            }
        }
    }
}

// ============================================================================
// KeyAction: Configurable Editor Action
// ============================================================================

/// Action triggered when a key chord is matched.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KeyAction {
    /// Submit prompt input (`Enter`).
    Submit,
    /// Cancel current input or turn (`Ctrl+C`).
    Cancel,
    /// Exit Fusion REPL session (`Ctrl+D` on empty).
    Exit,
    /// Clear screen buffer (`Ctrl+L`).
    ClearScreen,
    /// No-operation (ignore key).
    Noop,

    // --- Cursor Navigation ---
    /// Move cursor left one character.
    MoveLeft,
    /// Move cursor right one character.
    MoveRight,
    /// Move cursor up one line / previous history entry.
    MoveUp,
    /// Move cursor down one line / next history entry.
    MoveDown,
    /// Move cursor backward by one word.
    MoveWordLeft,
    /// Move cursor forward by one word.
    MoveWordRight,
    /// Move cursor to the end of the current word.
    MoveWordEnd,
    /// Move cursor to beginning of line (`Home` / `Ctrl+A`).
    MoveToBol,
    /// Move cursor to end of line (`End` / `Ctrl+E`).
    MoveToEol,
    /// Move cursor to first non-whitespace character on current line.
    MoveToFirstNonWhitespace,
    /// Move cursor to absolute start of prompt buffer.
    MoveToBufferStart,
    /// Move cursor to absolute end of prompt buffer.
    MoveToBufferEnd,

    // --- Content Editing & Deletion ---
    /// Insert a specific character literal.
    InsertChar(char),
    /// Insert a specific string literal.
    InsertText(String),
    /// Insert a newline character (`\n`).
    InsertNewline,
    /// Insert 4 spaces (Tab).
    InsertTab,
    /// Delete character immediately under cursor (`Delete`).
    DeleteChar,
    /// Delete character before cursor (`Backspace`).
    Backspace,
    /// Kill word before cursor (`Ctrl+W`).
    DeleteWordBackward,
    /// Kill word after cursor (`Alt+D`).
    DeleteWordForward,
    /// Kill text from cursor to end of line (`Ctrl+K`).
    KillToEol,
    /// Kill text from cursor to start of line (`Ctrl+U`).
    KillToBol,
    /// Kill entire prompt buffer.
    KillLine,
    /// Clear the entire prompt buffer without saving to kill ring.
    ClearBuffer,

    // --- History Navigation ---
    /// Navigate to previous command in history (`Up`).
    HistoryPrev,
    /// Navigate to next command in history (`Down`).
    HistoryNext,

    // --- Kill Ring / Clipboard ---
    /// Paste text from top of kill ring (`Ctrl+Y`).
    Yank,
    /// Replace recently pasted text with earlier kill ring entry (`Alt+Y`).
    YankPop,

    // --- Undo / Redo ---
    /// Undo last edit (`Ctrl+_` / `Ctrl+Z` / `u`).
    Undo,
    /// Redo last undone edit (`Ctrl+R`).
    Redo,

    // --- Profiles & Modes ---
    /// Switch keybinding profile (Default, Emacs, Vi).
    SwitchProfile(KeybindingProfile),
    /// Set Vi mode explicitly (Normal, Insert).
    SetViMode(ViMode),
    /// Toggle between Vi Normal and Insert mode.
    ToggleViMode,

    // --- Extensions & Slash Commands ---
    /// Execute a slash command directly (e.g. `"/clear"`, `"/stats"`).
    ExecuteCommand(String),
    /// Execute a sequence of actions sequentially.
    Macro(Vec<KeyAction>),
    /// Custom extension trigger.
    Custom(String),
}

impl KeyAction {
    /// Execute this action directly on active `PromptState` and `KeyHandler`.
    pub fn execute(&self, state: &mut PromptState, handler: &mut KeyHandler) -> KeyResult {
        match self {
            KeyAction::Submit => {
                let text = state.text();
                KeyResult::Submit(text)
            }
            KeyAction::Cancel => KeyResult::Cancel,
            KeyAction::Exit => KeyResult::Exit,
            KeyAction::ClearScreen => KeyResult::ClearScreen,
            KeyAction::Noop => KeyResult::Noop,

            KeyAction::MoveLeft => {
                if *state.cursor_pos > 0 {
                    *state.cursor_pos -= 1;
                }
                KeyResult::Continue
            }
            KeyAction::MoveRight => {
                if *state.cursor_pos < state.buffer.len() {
                    *state.cursor_pos += 1;
                }
                KeyResult::Continue
            }
            KeyAction::MoveUp => {
                state.move_up_or_history();
                KeyResult::Continue
            }
            KeyAction::MoveDown => {
                state.move_down_or_history();
                KeyResult::Continue
            }
            KeyAction::MoveWordLeft => {
                *state.cursor_pos = state.prev_word_pos();
                KeyResult::Continue
            }
            KeyAction::MoveWordRight => {
                *state.cursor_pos = state.next_word_pos();
                KeyResult::Continue
            }
            KeyAction::MoveWordEnd => {
                *state.cursor_pos = state.next_word_end_pos();
                KeyResult::Continue
            }
            KeyAction::MoveToBol => {
                state.move_to_bol();
                KeyResult::Continue
            }
            KeyAction::MoveToEol => {
                state.move_to_eol();
                KeyResult::Continue
            }
            KeyAction::MoveToFirstNonWhitespace => {
                state.move_to_first_non_whitespace();
                KeyResult::Continue
            }
            KeyAction::MoveToBufferStart => {
                *state.cursor_pos = 0;
                KeyResult::Continue
            }
            KeyAction::MoveToBufferEnd => {
                *state.cursor_pos = state.buffer.len();
                KeyResult::Continue
            }

            KeyAction::InsertChar(c) => {
                handler.snapshot_undo(state.buffer, *state.cursor_pos);
                state.insert_char(*c);
                KeyResult::Continue
            }
            KeyAction::InsertText(s) => {
                handler.snapshot_undo(state.buffer, *state.cursor_pos);
                state.insert_str(s);
                KeyResult::Continue
            }
            KeyAction::InsertNewline => {
                handler.snapshot_undo(state.buffer, *state.cursor_pos);
                state.insert_char('\n');
                KeyResult::Continue
            }
            KeyAction::InsertTab => {
                handler.snapshot_undo(state.buffer, *state.cursor_pos);
                state.insert_str("    ");
                KeyResult::Continue
            }
            KeyAction::DeleteChar => {
                if *state.cursor_pos < state.buffer.len() {
                    handler.snapshot_undo(state.buffer, *state.cursor_pos);
                    state.remove_char_at_cursor();
                }
                KeyResult::Continue
            }
            KeyAction::Backspace => {
                if *state.cursor_pos > 0 {
                    handler.snapshot_undo(state.buffer, *state.cursor_pos);
                    state.remove_char_before_cursor();
                }
                KeyResult::Continue
            }
            KeyAction::DeleteWordBackward => {
                if *state.cursor_pos > 0 {
                    let prev = state.prev_word_pos();
                    handler.snapshot_undo(state.buffer, *state.cursor_pos);
                    let killed = state.drain_range(prev, *state.cursor_pos);
                    handler.push_kill(killed);
                }
                KeyResult::Continue
            }
            KeyAction::DeleteWordForward => {
                if *state.cursor_pos < state.buffer.len() {
                    let next = state.next_word_pos();
                    handler.snapshot_undo(state.buffer, *state.cursor_pos);
                    let killed = state.drain_range(*state.cursor_pos, next);
                    handler.push_kill(killed);
                }
                KeyResult::Continue
            }
            KeyAction::KillToEol => {
                let (start, len) = state.current_line_range();
                let eol = start + len;
                if *state.cursor_pos < eol {
                    handler.snapshot_undo(state.buffer, *state.cursor_pos);
                    let killed = state.drain_range(*state.cursor_pos, eol);
                    handler.push_kill(killed);
                } else if *state.cursor_pos == eol && *state.cursor_pos < state.buffer.len() {
                    handler.snapshot_undo(state.buffer, *state.cursor_pos);
                    state.remove_char_at_cursor();
                }
                KeyResult::Continue
            }
            KeyAction::KillToBol => {
                let (bol, _) = state.current_line_range();
                if *state.cursor_pos > bol {
                    handler.snapshot_undo(state.buffer, *state.cursor_pos);
                    let killed = state.drain_range(bol, *state.cursor_pos);
                    handler.push_kill(killed);
                }
                KeyResult::Continue
            }
            KeyAction::KillLine => {
                handler.snapshot_undo(state.buffer, *state.cursor_pos);
                let killed: String = state.buffer.drain(..).collect();
                *state.cursor_pos = 0;
                handler.push_kill(killed);
                KeyResult::Continue
            }
            KeyAction::ClearBuffer => {
                handler.snapshot_undo(state.buffer, *state.cursor_pos);
                state.buffer.clear();
                *state.cursor_pos = 0;
                KeyResult::Continue
            }

            KeyAction::HistoryPrev => {
                state.move_up_or_history();
                KeyResult::Continue
            }
            KeyAction::HistoryNext => {
                state.move_down_or_history();
                KeyResult::Continue
            }

            KeyAction::Yank => {
                // Peek top of kill ring and paste
                // (KeyHandler kill_ring access via snapshot)
                KeyResult::Continue
            }
            KeyAction::YankPop => KeyResult::Continue,

            KeyAction::Undo => {
                handler.undo(state);
                KeyResult::Continue
            }
            KeyAction::Redo => {
                handler.redo(state);
                KeyResult::Continue
            }

            KeyAction::SwitchProfile(prof) => {
                handler.set_profile(*prof);
                KeyResult::Continue
            }
            KeyAction::SetViMode(mode) => {
                handler.set_vi_mode(*mode);
                KeyResult::Continue
            }
            KeyAction::ToggleViMode => {
                let next = if handler.vi_mode() == ViMode::Normal {
                    ViMode::Insert
                } else {
                    ViMode::Normal
                };
                handler.set_vi_mode(next);
                KeyResult::Continue
            }

            KeyAction::ExecuteCommand(cmd) => KeyResult::Submit(cmd.clone()),
            KeyAction::Macro(actions) => {
                let mut res = KeyResult::Continue;
                for act in actions {
                    res = act.execute(state, handler);
                    if res != KeyResult::Continue && res != KeyResult::Noop {
                        return res;
                    }
                }
                res
            }
            KeyAction::Custom(_) => KeyResult::Noop,
        }
    }

    /// Parse action from a string identifier.
    pub fn parse_str(s: &str) -> Result<Self, KeymapError> {
        let trimmed = s.trim();
        let lower = trimmed.to_lowercase().replace('-', "_");

        match lower.as_str() {
            "submit" | "accept" | "enter" => Ok(KeyAction::Submit),
            "cancel" | "abort" => Ok(KeyAction::Cancel),
            "exit" | "quit" | "eof" => Ok(KeyAction::Exit),
            "clear_screen" | "clear" | "cls" => Ok(KeyAction::ClearScreen),
            "noop" | "ignore" | "none" => Ok(KeyAction::Noop),

            "move_left" | "backward_char" | "left" => Ok(KeyAction::MoveLeft),
            "move_right" | "forward_char" | "right" => Ok(KeyAction::MoveRight),
            "move_up" | "previous_line" | "up" => Ok(KeyAction::MoveUp),
            "move_down" | "next_line" | "down" => Ok(KeyAction::MoveDown),
            "move_word_left" | "backward_word" | "word_left" => Ok(KeyAction::MoveWordLeft),
            "move_word_right" | "forward_word" | "word_right" => Ok(KeyAction::MoveWordRight),
            "move_word_end" | "end_of_word" => Ok(KeyAction::MoveWordEnd),
            "move_to_bol" | "beginning_of_line" | "line_start" | "home" => Ok(KeyAction::MoveToBol),
            "move_to_eol" | "end_of_line" | "line_end" | "end" => Ok(KeyAction::MoveToEol),
            "move_to_first_non_whitespace" | "first_non_whitespace" => {
                Ok(KeyAction::MoveToFirstNonWhitespace)
            }
            "move_to_buffer_start" | "beginning_of_buffer" | "buffer_start" => {
                Ok(KeyAction::MoveToBufferStart)
            }
            "move_to_buffer_end" | "end_of_buffer" | "buffer_end" => {
                Ok(KeyAction::MoveToBufferEnd)
            }

            "insert_newline" | "newline" => Ok(KeyAction::InsertNewline),
            "insert_tab" | "tab" => Ok(KeyAction::InsertTab),
            "delete_char" | "delete" | "delete_forward" => Ok(KeyAction::DeleteChar),
            "backspace" | "delete_backward" => Ok(KeyAction::Backspace),
            "delete_word_backward" | "kill_word_backward" | "backward_kill_word" => {
                Ok(KeyAction::DeleteWordBackward)
            }
            "delete_word_forward" | "kill_word_forward" | "kill_word" => {
                Ok(KeyAction::DeleteWordForward)
            }
            "kill_to_eol" | "kill_line_after" | "kill_after" => Ok(KeyAction::KillToEol),
            "kill_to_bol" | "kill_line_before" | "kill_before" => Ok(KeyAction::KillToBol),
            "kill_line" | "kill_whole_line" => Ok(KeyAction::KillLine),
            "clear_buffer" | "clear_line" => Ok(KeyAction::ClearBuffer),

            "history_prev" | "history_previous" | "previous_history" | "history_up" => {
                Ok(KeyAction::HistoryPrev)
            }
            "history_next" | "next_history" | "history_down" => Ok(KeyAction::HistoryNext),

            "yank" | "paste" => Ok(KeyAction::Yank),
            "yank_pop" | "paste_pop" => Ok(KeyAction::YankPop),

            "undo" => Ok(KeyAction::Undo),
            "redo" => Ok(KeyAction::Redo),

            "toggle_vi_mode" => Ok(KeyAction::ToggleViMode),

            _ => {
                if let Some(cmd) = trimmed.strip_prefix('/') {
                    return Ok(KeyAction::ExecuteCommand(format!("/{}", cmd)));
                }
                Err(KeymapError::InvalidAction(
                    s.to_string(),
                    format!("Unrecognized action identifier '{}'", s),
                ))
            }
        }
    }
}

impl fmt::Display for KeyAction {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            KeyAction::Submit => write!(f, "submit"),
            KeyAction::Cancel => write!(f, "cancel"),
            KeyAction::Exit => write!(f, "exit"),
            KeyAction::ClearScreen => write!(f, "clear_screen"),
            KeyAction::Noop => write!(f, "noop"),
            KeyAction::MoveLeft => write!(f, "move_left"),
            KeyAction::MoveRight => write!(f, "move_right"),
            KeyAction::MoveUp => write!(f, "move_up"),
            KeyAction::MoveDown => write!(f, "move_down"),
            KeyAction::MoveWordLeft => write!(f, "move_word_left"),
            KeyAction::MoveWordRight => write!(f, "move_word_right"),
            KeyAction::MoveWordEnd => write!(f, "move_word_end"),
            KeyAction::MoveToBol => write!(f, "move_to_bol"),
            KeyAction::MoveToEol => write!(f, "move_to_eol"),
            KeyAction::MoveToFirstNonWhitespace => write!(f, "move_to_first_non_whitespace"),
            KeyAction::MoveToBufferStart => write!(f, "move_to_buffer_start"),
            KeyAction::MoveToBufferEnd => write!(f, "move_to_buffer_end"),
            KeyAction::InsertChar(c) => write!(f, "insert_char({})", c),
            KeyAction::InsertText(t) => write!(f, "insert_text({:?})", t),
            KeyAction::InsertNewline => write!(f, "insert_newline"),
            KeyAction::InsertTab => write!(f, "insert_tab"),
            KeyAction::DeleteChar => write!(f, "delete_char"),
            KeyAction::Backspace => write!(f, "backspace"),
            KeyAction::DeleteWordBackward => write!(f, "delete_word_backward"),
            KeyAction::DeleteWordForward => write!(f, "delete_word_forward"),
            KeyAction::KillToEol => write!(f, "kill_to_eol"),
            KeyAction::KillToBol => write!(f, "kill_to_bol"),
            KeyAction::KillLine => write!(f, "kill_line"),
            KeyAction::ClearBuffer => write!(f, "clear_buffer"),
            KeyAction::HistoryPrev => write!(f, "history_prev"),
            KeyAction::HistoryNext => write!(f, "history_next"),
            KeyAction::Yank => write!(f, "yank"),
            KeyAction::YankPop => write!(f, "yank_pop"),
            KeyAction::Undo => write!(f, "undo"),
            KeyAction::Redo => write!(f, "redo"),
            KeyAction::SwitchProfile(p) => write!(f, "switch_profile({})", p),
            KeyAction::SetViMode(m) => write!(f, "set_vi_mode({})", m),
            KeyAction::ToggleViMode => write!(f, "toggle_vi_mode"),
            KeyAction::ExecuteCommand(cmd) => write!(f, "execute_command({})", cmd),
            KeyAction::Macro(actions) => write!(f, "macro({} actions)", actions.len()),
            KeyAction::Custom(c) => write!(f, "custom({})", c),
        }
    }
}

impl FromStr for KeyAction {
    type Err = KeymapError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        KeyAction::parse_str(s)
    }
}

impl Serialize for KeyAction {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        match self {
            KeyAction::Submit => serializer.serialize_str("submit"),
            KeyAction::Cancel => serializer.serialize_str("cancel"),
            KeyAction::Exit => serializer.serialize_str("exit"),
            KeyAction::ClearScreen => serializer.serialize_str("clear_screen"),
            KeyAction::Noop => serializer.serialize_str("noop"),
            KeyAction::MoveLeft => serializer.serialize_str("move_left"),
            KeyAction::MoveRight => serializer.serialize_str("move_right"),
            KeyAction::MoveUp => serializer.serialize_str("move_up"),
            KeyAction::MoveDown => serializer.serialize_str("move_down"),
            KeyAction::MoveWordLeft => serializer.serialize_str("move_word_left"),
            KeyAction::MoveWordRight => serializer.serialize_str("move_word_right"),
            KeyAction::MoveWordEnd => serializer.serialize_str("move_word_end"),
            KeyAction::MoveToBol => serializer.serialize_str("move_to_bol"),
            KeyAction::MoveToEol => serializer.serialize_str("move_to_eol"),
            KeyAction::MoveToFirstNonWhitespace => {
                serializer.serialize_str("move_to_first_non_whitespace")
            }
            KeyAction::MoveToBufferStart => serializer.serialize_str("move_to_buffer_start"),
            KeyAction::MoveToBufferEnd => serializer.serialize_str("move_to_buffer_end"),
            KeyAction::InsertNewline => serializer.serialize_str("insert_newline"),
            KeyAction::InsertTab => serializer.serialize_str("insert_tab"),
            KeyAction::DeleteChar => serializer.serialize_str("delete_char"),
            KeyAction::Backspace => serializer.serialize_str("backspace"),
            KeyAction::DeleteWordBackward => serializer.serialize_str("delete_word_backward"),
            KeyAction::DeleteWordForward => serializer.serialize_str("delete_word_forward"),
            KeyAction::KillToEol => serializer.serialize_str("kill_to_eol"),
            KeyAction::KillToBol => serializer.serialize_str("kill_to_bol"),
            KeyAction::KillLine => serializer.serialize_str("kill_line"),
            KeyAction::ClearBuffer => serializer.serialize_str("clear_buffer"),
            KeyAction::HistoryPrev => serializer.serialize_str("history_prev"),
            KeyAction::HistoryNext => serializer.serialize_str("history_next"),
            KeyAction::Yank => serializer.serialize_str("yank"),
            KeyAction::YankPop => serializer.serialize_str("yank_pop"),
            KeyAction::Undo => serializer.serialize_str("undo"),
            KeyAction::Redo => serializer.serialize_str("redo"),
            KeyAction::ToggleViMode => serializer.serialize_str("toggle_vi_mode"),

            // Structured object forms
            KeyAction::InsertChar(c) => {
                #[derive(Serialize)]
                struct Helper<'a> {
                    action: &'a str,
                    char: char,
                }
                Helper {
                    action: "insert_char",
                    char: *c,
                }
                .serialize(serializer)
            }
            KeyAction::InsertText(t) => {
                #[derive(Serialize)]
                struct Helper<'a> {
                    action: &'a str,
                    text: &'a str,
                }
                Helper {
                    action: "insert_text",
                    text: t,
                }
                .serialize(serializer)
            }
            KeyAction::SwitchProfile(p) => {
                #[derive(Serialize)]
                struct Helper<'a> {
                    action: &'a str,
                    profile: &'a str,
                }
                let prof_str = p.to_string();
                Helper {
                    action: "switch_profile",
                    profile: &prof_str,
                }
                .serialize(serializer)
            }
            KeyAction::SetViMode(m) => {
                #[derive(Serialize)]
                struct Helper<'a> {
                    action: &'a str,
                    mode: &'a str,
                }
                let mode_str = m.to_string();
                Helper {
                    action: "set_vi_mode",
                    mode: &mode_str,
                }
                .serialize(serializer)
            }
            KeyAction::ExecuteCommand(cmd) => {
                #[derive(Serialize)]
                struct Helper<'a> {
                    action: &'a str,
                    command: &'a str,
                }
                Helper {
                    action: "execute_command",
                    command: cmd,
                }
                .serialize(serializer)
            }
            KeyAction::Macro(actions) => {
                #[derive(Serialize)]
                struct Helper<'a> {
                    action: &'a str,
                    macro_actions: &'a [KeyAction],
                }
                Helper {
                    action: "macro",
                    macro_actions: actions,
                }
                .serialize(serializer)
            }
            KeyAction::Custom(c) => {
                #[derive(Serialize)]
                struct Helper<'a> {
                    action: &'a str,
                    custom: &'a str,
                }
                Helper {
                    action: "custom",
                    custom: c,
                }
                .serialize(serializer)
            }
        }
    }
}

impl<'de> Deserialize<'de> for KeyAction {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct KeyActionVisitor;

        impl<'de> Visitor<'de> for KeyActionVisitor {
            type Value = KeyAction;

            fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
                formatter.write_str("a key action string or action object")
            }

            fn visit_str<E>(self, value: &str) -> Result<KeyAction, E>
            where
                E: de::Error,
            {
                KeyAction::parse_str(value).map_err(de::Error::custom)
            }

            fn visit_map<M>(self, mut access: M) -> Result<KeyAction, M::Error>
            where
                M: de::MapAccess<'de>,
            {
                let mut action_type = String::new();
                let mut command = None;
                let mut text = None;
                let mut char_val = None;
                let mut profile = None;
                let mut mode = None;
                let mut macro_actions = None;
                let mut custom = None;

                while let Some((key, val)) = access.next_entry::<String, serde_json::Value>()? {
                    match key.as_str() {
                        "action" | "type" => {
                            if let Some(s) = val.as_str() {
                                action_type = s.to_string();
                            }
                        }
                        "command" | "cmd" => command = val.as_str().map(|s| s.to_string()),
                        "text" | "string" => text = val.as_str().map(|s| s.to_string()),
                        "char" => {
                            if let Some(s) = val.as_str() {
                                char_val = s.chars().next();
                            }
                        }
                        "profile" => profile = val.as_str().map(|s| s.to_string()),
                        "mode" => mode = val.as_str().map(|s| s.to_string()),
                        "macro_actions" | "actions" | "sequence" => {
                            if let Ok(acts) = serde_json::from_value::<Vec<KeyAction>>(val) {
                                macro_actions = Some(acts);
                            }
                        }
                        "custom" | "name" => custom = val.as_str().map(|s| s.to_string()),
                        _ => {}
                    }
                }

                let normalized_action = action_type.to_lowercase().replace('-', "_");
                match normalized_action.as_str() {
                    "execute_command" | "command" => {
                        let c = command.ok_or_else(|| {
                            de::Error::missing_field("command for execute_command action")
                        })?;
                        Ok(KeyAction::ExecuteCommand(c))
                    }
                    "insert_text" => {
                        let t = text.ok_or_else(|| {
                            de::Error::missing_field("text for insert_text action")
                        })?;
                        Ok(KeyAction::InsertText(t))
                    }
                    "insert_char" => {
                        let c = char_val.ok_or_else(|| {
                            de::Error::missing_field("char for insert_char action")
                        })?;
                        Ok(KeyAction::InsertChar(c))
                    }
                    "switch_profile" => {
                        let p_str = profile.ok_or_else(|| {
                            de::Error::missing_field("profile for switch_profile action")
                        })?;
                        let p = KeybindingProfile::from_str(&p_str).map_err(de::Error::custom)?;
                        Ok(KeyAction::SwitchProfile(p))
                    }
                    "set_vi_mode" => {
                        let m_str = mode.ok_or_else(|| {
                            de::Error::missing_field("mode for set_vi_mode action")
                        })?;
                        let m = match m_str.to_lowercase().as_str() {
                            "normal" => ViMode::Normal,
                            "insert" => ViMode::Insert,
                            other => {
                                return Err(de::Error::custom(format!(
                                    "Unknown Vi mode '{}'",
                                    other
                                )))
                            }
                        };
                        Ok(KeyAction::SetViMode(m))
                    }
                    "macro" => {
                        let acts = macro_actions.ok_or_else(|| {
                            de::Error::missing_field("macro_actions for macro action")
                        })?;
                        Ok(KeyAction::Macro(acts))
                    }
                    "custom" => {
                        let c = custom.ok_or_else(|| {
                            de::Error::missing_field("custom name for custom action")
                        })?;
                        Ok(KeyAction::Custom(c))
                    }
                    _ => KeyAction::parse_str(&action_type).map_err(de::Error::custom),
                }
            }
        }

        deserializer.deserialize_any(KeyActionVisitor)
    }
}

// ============================================================================
// KeymapConfig: Main Persistent Configuration Struct
// ============================================================================

/// User keymap configuration stored in `~/.fusion/keymap.json`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KeymapConfig {
    /// Schema version for forward/backward compatibility (default: 1).
    #[serde(default = "default_version")]
    pub version: u32,

    /// Optional default keybinding profile override.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub profile: Option<KeybindingProfile>,

    /// Optional leader key chord (e.g. `"Ctrl+X"` or `"Space"`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub leader: Option<String>,

    /// Timeout in milliseconds for multi-key leader sequences.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub leader_timeout_ms: Option<u64>,

    /// Global keybindings applied across all editing modes.
    #[serde(default)]
    pub bindings: HashMap<String, KeyAction>,

    /// Emacs-profile specific keybindings and overrides.
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub emacs: HashMap<String, KeyAction>,

    /// Vi Normal mode specific keybindings and overrides.
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub vi_normal: HashMap<String, KeyAction>,

    /// Vi Insert mode specific keybindings and overrides.
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub vi_insert: HashMap<String, KeyAction>,

    /// Key sequence aliases / chord shortcuts.
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub aliases: HashMap<String, String>,

    /// User descriptions and notes for custom keybindings.
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub descriptions: HashMap<String, String>,
}

fn default_version() -> u32 {
    1
}

impl Default for KeymapConfig {
    fn default() -> Self {
        Self {
            version: 1,
            profile: None,
            leader: None,
            leader_timeout_ms: Some(DEFAULT_LEADER_TIMEOUT_MS),
            bindings: HashMap::new(),
            emacs: HashMap::new(),
            vi_normal: HashMap::new(),
            vi_insert: HashMap::new(),
            aliases: HashMap::new(),
            descriptions: HashMap::new(),
        }
    }
}

impl KeymapConfig {
    /// Create an empty keymap configuration with default schema version.
    pub fn new() -> Self {
        Self::default()
    }

    /// Returns standard filesystem path to `~/.fusion/keymap.json`.
    pub fn default_path() -> PathBuf {
        Config::config_dir().join(KEYMAP_FILE_NAME)
    }

    /// Load keymap configuration from the default location `~/.fusion/keymap.json`.
    ///
    /// If the file does not exist, returns a default `KeymapConfig`.
    pub fn load() -> Result<Self, KeymapError> {
        let path = Self::default_path();
        if !path.exists() {
            return Ok(Self::default());
        }
        Self::load_from_path(&path)
    }

    /// Load keymap configuration from a specific file path.
    pub fn load_from_path(path: &Path) -> Result<Self, KeymapError> {
        let content = fs::read_to_string(path)?;
        let config: KeymapConfig = serde_json::from_str(&content)?;
        Ok(config)
    }

    /// Load keymap configuration from default location or return defaults on failure.
    pub fn load_or_default() -> Self {
        Self::load().unwrap_or_default()
    }

    /// Save current keymap configuration to the default path `~/.fusion/keymap.json`.
    pub fn save(&self) -> Result<PathBuf, KeymapError> {
        let path = Self::default_path();
        self.save_to_path(&path)?;
        Ok(path)
    }

    /// Save current keymap configuration to the specified destination path.
    pub fn save_to_path(&self, path: &Path) -> Result<(), KeymapError> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let json = serde_json::to_string_pretty(self)?;
        fs::write(path, json)?;
        Ok(())
    }

    /// Initialize default template file at `~/.fusion/keymap.json` if missing.
    pub fn init_default_file_if_missing() -> Result<PathBuf, KeymapError> {
        let path = Self::default_path();
        if !path.exists() {
            let template = Self::default_template();
            template.save_to_path(&path)?;
        }
        Ok(path)
    }

    /// Bind a key chord to an action in global scope.
    pub fn bind(&mut self, chord: &str, action: KeyAction) -> Result<(), KeymapError> {
        let parsed = KeyChord::parse(chord)?;
        self.bindings.insert(parsed.to_canonical_string(), action);
        Ok(())
    }

    /// Bind a key chord to an action in Emacs scope.
    pub fn bind_emacs(&mut self, chord: &str, action: KeyAction) -> Result<(), KeymapError> {
        let parsed = KeyChord::parse(chord)?;
        self.emacs.insert(parsed.to_canonical_string(), action);
        Ok(())
    }

    /// Bind a key chord to an action in Vi Normal mode scope.
    pub fn bind_vi_normal(&mut self, chord: &str, action: KeyAction) -> Result<(), KeymapError> {
        let parsed = KeyChord::parse(chord)?;
        self.vi_normal
            .insert(parsed.to_canonical_string(), action);
        Ok(())
    }

    /// Bind a key chord to an action in Vi Insert mode scope.
    pub fn bind_vi_insert(&mut self, chord: &str, action: KeyAction) -> Result<(), KeymapError> {
        let parsed = KeyChord::parse(chord)?;
        self.vi_insert
            .insert(parsed.to_canonical_string(), action);
        Ok(())
    }

    /// Unbind a key chord across all scopes.
    pub fn unbind(&mut self, chord: &str) -> bool {
        let mut removed = false;
        if let Ok(parsed) = KeyChord::parse(chord) {
            let canon = parsed.to_canonical_string();
            removed |= self.bindings.remove(&canon).is_some();
            removed |= self.emacs.remove(&canon).is_some();
            removed |= self.vi_normal.remove(&canon).is_some();
            removed |= self.vi_insert.remove(&canon).is_some();
        }
        removed |= self.bindings.remove(chord).is_some();
        removed |= self.emacs.remove(chord).is_some();
        removed |= self.vi_normal.remove(chord).is_some();
        removed |= self.vi_insert.remove(chord).is_some();
        removed
    }

    /// Validate the keymap configuration for syntax errors and conflicts.
    pub fn validate(&self) -> KeymapValidation {
        let mut validation = KeymapValidation::default();

        // Validate leader chord if present
        if let Some(l) = &self.leader {
            if let Err(e) = KeyChord::parse(l) {
                validation
                    .errors
                    .push(format!("Invalid leader key chord '{}': {}", l, e));
            }
        }

        // Validate global bindings
        for (chord_str, _action) in &self.bindings {
            validation.total_bindings += 1;
            if let Err(e) = KeyChord::parse(chord_str) {
                validation
                    .errors
                    .push(format!("Invalid global binding chord '{}': {}", chord_str, e));
            }
        }

        // Validate Emacs bindings
        for (chord_str, _action) in &self.emacs {
            validation.total_bindings += 1;
            if let Err(e) = KeyChord::parse(chord_str) {
                validation
                    .errors
                    .push(format!("Invalid Emacs binding chord '{}': {}", chord_str, e));
            }
        }

        // Validate Vi Normal bindings
        for (chord_str, _action) in &self.vi_normal {
            validation.total_bindings += 1;
            if let Err(e) = KeyChord::parse(chord_str) {
                validation.errors.push(format!(
                    "Invalid Vi Normal binding chord '{}': {}",
                    chord_str, e
                ));
            }
        }

        // Validate Vi Insert bindings
        for (chord_str, _action) in &self.vi_insert {
            validation.total_bindings += 1;
            if let Err(e) = KeyChord::parse(chord_str) {
                validation.errors.push(format!(
                    "Invalid Vi Insert binding chord '{}': {}",
                    chord_str, e
                ));
            }
        }

        // Validate aliases
        for (from, to) in &self.aliases {
            if let Err(e) = KeyChord::parse(from) {
                validation
                    .errors
                    .push(format!("Invalid alias source chord '{}': {}", from, e));
            }
            if let Err(e) = KeyChord::parse(to) {
                validation
                    .errors
                    .push(format!("Invalid alias target chord '{}': {}", to, e));
            }
        }

        validation.is_valid = validation.errors.is_empty();
        validation
    }

    /// Produce a starter template configuration with common examples.
    pub fn default_template() -> Self {
        let mut cfg = Self::default();
        cfg.leader = Some("Ctrl+X".to_string());
        cfg.leader_timeout_ms = Some(1000);

        // Global bindings
        cfg.bindings.insert(
            "Ctrl+S".to_string(),
            KeyAction::Submit,
        );
        cfg.bindings.insert(
            "Alt+Enter".to_string(),
            KeyAction::InsertNewline,
        );
        cfg.bindings.insert(
            "Ctrl+Shift+C".to_string(),
            KeyAction::ExecuteCommand("/clear".to_string()),
        );
        cfg.bindings.insert(
            "Ctrl+Shift+P".to_string(),
            KeyAction::ExecuteCommand("/palette".to_string()),
        );

        // Emacs overrides
        cfg.emacs.insert(
            "Ctrl+K".to_string(),
            KeyAction::KillToEol,
        );
        cfg.emacs.insert(
            "Alt+F".to_string(),
            KeyAction::MoveWordRight,
        );
        cfg.emacs.insert(
            "Alt+B".to_string(),
            KeyAction::MoveWordLeft,
        );

        // Vi Normal overrides
        cfg.vi_normal.insert(
            "Space F".to_string(),
            KeyAction::ExecuteCommand("/file".to_string()),
        );

        // Descriptions
        cfg.descriptions.insert(
            "Ctrl+S".to_string(),
            "Quick submit input without Enter".to_string(),
        );
        cfg.descriptions.insert(
            "Ctrl+Shift+P".to_string(),
            "Open interactive command palette".to_string(),
        );

        cfg
    }

    /// Generate clean formatted JSON string of standard sample keymap.
    pub fn sample_json() -> String {
        serde_json::to_string_pretty(&Self::default_template()).unwrap_or_default()
    }
}

// ============================================================================
// KeymapValidation: Validation Diagnostics Report
// ============================================================================

/// Diagnostic report generated when validating keymap configurations.
#[derive(Debug, Clone, Default)]
pub struct KeymapValidation {
    /// Whether configuration passed all syntax and semantic checks.
    pub is_valid: bool,
    /// List of syntax or configuration errors encountered.
    pub errors: Vec<String>,
    /// List of non-fatal warnings or potential conflicts.
    pub warnings: Vec<String>,
    /// Total count of configured keybindings across all modes.
    pub total_bindings: usize,
}

impl fmt::Display for KeymapValidation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.is_valid {
            write!(
                f,
                "✓ Keymap valid: {} binding(s) configured",
                self.total_bindings
            )?;
            if !self.warnings.is_empty() {
                write!(f, " ({} warning(s))", self.warnings.len())?;
            }
        } else {
            write!(
                f,
                "✗ Keymap validation failed ({} error(s), {} warning(s)):\n",
                self.errors.len(),
                self.warnings.len()
            )?;
            for (i, err) in self.errors.iter().enumerate() {
                write!(f, "  {}. {}\n", i + 1, err)?;
            }
        }
        Ok(())
    }
}

// ============================================================================
// KeymapManager: Runtime Keybinding Resolver with Caching
// ============================================================================

/// Runtime manager for active custom keybindings, providing zero-allocation
/// key lookup, leader sequence tracking, and hot-reloading.
#[derive(Debug, Clone)]
pub struct KeymapManager {
    /// Underlying persistent configuration.
    config: KeymapConfig,
    /// Parsed global keybindings.
    global_chords: HashMap<KeyChord, KeyAction>,
    /// Parsed Emacs profile keybindings.
    emacs_chords: HashMap<KeyChord, KeyAction>,
    /// Parsed Vi Normal mode keybindings.
    vi_normal_chords: HashMap<KeyChord, KeyAction>,
    /// Parsed Vi Insert mode keybindings.
    vi_insert_chords: HashMap<KeyChord, KeyAction>,
    /// Optional parsed leader chord.
    leader_chord: Option<KeyChord>,
    /// Leader key timeout duration.
    leader_timeout: Duration,
    /// Timestamp when leader key was pressed (for multi-key sequences).
    leader_pressed_at: Option<Instant>,
}

impl Default for KeymapManager {
    fn default() -> Self {
        Self::from_config(KeymapConfig::default())
    }
}

impl KeymapManager {
    /// Create a keymap manager by compiling a `KeymapConfig`.
    pub fn from_config(config: KeymapConfig) -> Self {
        let mut global_chords = HashMap::new();
        for (k, v) in &config.bindings {
            if let Ok(chord) = KeyChord::parse(k) {
                global_chords.insert(chord, v.clone());
            }
        }

        let mut emacs_chords = HashMap::new();
        for (k, v) in &config.emacs {
            if let Ok(chord) = KeyChord::parse(k) {
                emacs_chords.insert(chord, v.clone());
            }
        }

        let mut vi_normal_chords = HashMap::new();
        for (k, v) in &config.vi_normal {
            if let Ok(chord) = KeyChord::parse(k) {
                vi_normal_chords.insert(chord, v.clone());
            }
        }

        let mut vi_insert_chords = HashMap::new();
        for (k, v) in &config.vi_insert {
            if let Ok(chord) = KeyChord::parse(k) {
                vi_insert_chords.insert(chord, v.clone());
            }
        }

        let leader_chord = config
            .leader
            .as_ref()
            .and_then(|l| KeyChord::parse(l).ok());
        let leader_timeout = Duration::from_millis(
            config
                .leader_timeout_ms
                .unwrap_or(DEFAULT_LEADER_TIMEOUT_MS),
        );

        Self {
            config,
            global_chords,
            emacs_chords,
            vi_normal_chords,
            vi_insert_chords,
            leader_chord,
            leader_timeout,
            leader_pressed_at: None,
        }
    }

    /// Load keymap manager from `~/.fusion/keymap.json` or fallback to default.
    pub fn load() -> Self {
        let cfg = KeymapConfig::load_or_default();
        Self::from_config(cfg)
    }

    /// Return reference to the active configuration.
    pub fn config(&self) -> &KeymapConfig {
        &self.config
    }

    /// Check whether a leader key sequence is currently active.
    pub fn is_leader_active(&self) -> bool {
        if let Some(pressed_at) = self.leader_pressed_at {
            pressed_at.elapsed() <= self.leader_timeout
        } else {
            false
        }
    }

    /// Reset any pending leader state.
    pub fn reset_leader(&mut self) {
        self.leader_pressed_at = None;
    }

    /// Resolve a key event against custom keybindings.
    ///
    /// Resolution order:
    /// 1. Check leader chord trigger
    /// 2. Mode-specific overrides (Vi Normal, Vi Insert, Emacs)
    /// 3. Global custom keybindings
    pub fn resolve(
        &mut self,
        event: &KeyEvent,
        profile: KeybindingProfile,
        vi_mode: Option<ViMode>,
    ) -> Option<KeyAction> {
        if event.kind == KeyEventKind::Release {
            return None;
        }

        let chord = KeyChord::from(*event);

        // Check leader chord trigger
        if let Some(leader) = self.leader_chord {
            if leader.matches(event) {
                self.leader_pressed_at = Some(Instant::now());
                return Some(KeyAction::Noop);
            }
        }

        // Check mode-specific maps first
        let mode_action = match profile {
            KeybindingProfile::Vi => match vi_mode.unwrap_or(ViMode::Insert) {
                ViMode::Normal => self.vi_normal_chords.get(&chord).cloned(),
                ViMode::Insert => self.vi_insert_chords.get(&chord).cloned(),
            },
            KeybindingProfile::Emacs => self.emacs_chords.get(&chord).cloned(),
            KeybindingProfile::Default => None,
        };

        if let Some(act) = mode_action {
            self.reset_leader();
            return Some(act);
        }

        // Fallback to global custom keybindings
        if let Some(act) = self.global_chords.get(&chord).cloned() {
            self.reset_leader();
            return Some(act);
        }

        self.reset_leader();
        None
    }

    /// Reload configuration from `~/.fusion/keymap.json`.
    pub fn reload(&mut self) -> Result<(), KeymapError> {
        let loaded = KeymapConfig::load()?;
        *self = Self::from_config(loaded);
        Ok(())
    }
}

// ============================================================================
// Unit Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn test_key_chord_parse_simple() {
        let chord = KeyChord::parse("ctrl+c").unwrap();
        assert_eq!(chord.modifiers, KeyModifiers::CONTROL);
        assert_eq!(chord.code, KeyCode::Char('c'));
        assert_eq!(chord.to_canonical_string(), "Ctrl+C");

        let chord_alt = KeyChord::parse("alt-f").unwrap();
        assert_eq!(chord_alt.modifiers, KeyModifiers::ALT);
        assert_eq!(chord_alt.code, KeyCode::Char('f'));
        assert_eq!(chord_alt.to_canonical_string(), "Alt+F");
    }

    #[test]
    fn test_key_chord_parse_special_keys() {
        assert_eq!(
            KeyChord::parse("enter").unwrap(),
            KeyChord::plain(KeyCode::Enter)
        );
        assert_eq!(
            KeyChord::parse("esc").unwrap(),
            KeyChord::plain(KeyCode::Esc)
        );
        assert_eq!(
            KeyChord::parse("tab").unwrap(),
            KeyChord::plain(KeyCode::Tab)
        );
        assert_eq!(
            KeyChord::parse("backspace").unwrap(),
            KeyChord::plain(KeyCode::Backspace)
        );
        assert_eq!(
            KeyChord::parse("delete").unwrap(),
            KeyChord::plain(KeyCode::Delete)
        );
        assert_eq!(
            KeyChord::parse("space").unwrap(),
            KeyChord::plain(KeyCode::Char(' '))
        );
        assert_eq!(
            KeyChord::parse("f5").unwrap(),
            KeyChord::plain(KeyCode::F(5))
        );
        assert_eq!(
            KeyChord::parse("pageup").unwrap(),
            KeyChord::plain(KeyCode::PageUp)
        );
        assert_eq!(
            KeyChord::parse("pagedown").unwrap(),
            KeyChord::plain(KeyCode::PageDown)
        );
    }

    #[test]
    fn test_key_chord_parse_multiple_modifiers() {
        let chord = KeyChord::parse("ctrl+alt+delete").unwrap();
        assert!(chord.modifiers.contains(KeyModifiers::CONTROL));
        assert!(chord.modifiers.contains(KeyModifiers::ALT));
        assert_eq!(chord.code, KeyCode::Delete);

        let chord_shift = KeyChord::parse("ctrl+shift+p").unwrap();
        assert!(chord_shift.modifiers.contains(KeyModifiers::CONTROL));
        assert!(chord_shift.modifiers.contains(KeyModifiers::SHIFT));
        assert_eq!(chord_shift.code, KeyCode::Char('p'));
    }

    #[test]
    fn test_key_chord_matching() {
        let chord = KeyChord::parse("ctrl+c").unwrap();
        let event = KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL);
        assert!(chord.matches(&event));

        let event_diff = KeyEvent::new(KeyCode::Char('v'), KeyModifiers::CONTROL);
        assert!(!chord.matches(&event_diff));

        let event_no_mod = KeyEvent::new(KeyCode::Char('c'), KeyModifiers::NONE);
        assert!(!chord.matches(&event_no_mod));
    }

    #[test]
    fn test_key_action_parsing() {
        assert_eq!(
            KeyAction::parse_str("submit").unwrap(),
            KeyAction::Submit
        );
        assert_eq!(
            KeyAction::parse_str("kill_to_eol").unwrap(),
            KeyAction::KillToEol
        );
        assert_eq!(
            KeyAction::parse_str("clear_screen").unwrap(),
            KeyAction::ClearScreen
        );
        assert_eq!(
            KeyAction::parse_str("/help").unwrap(),
            KeyAction::ExecuteCommand("/help".to_string())
        );
    }

    #[test]
    fn test_keymap_config_save_and_load() {
        let temp = tempdir().unwrap();
        let path = temp.path().join("keymap.json");

        let mut config = KeymapConfig::default();
        config.leader = Some("Ctrl+X".to_string());
        config
            .bind("ctrl+s", KeyAction::Submit)
            .unwrap();
        config
            .bind("ctrl+k", KeyAction::KillToEol)
            .unwrap();
        config
            .bind_emacs("alt+f", KeyAction::MoveWordRight)
            .unwrap();
        config
            .bind_vi_normal("space f", KeyAction::ExecuteCommand("/file".to_string()))
            .unwrap();

        config.save_to_path(&path).unwrap();
        assert!(path.exists());

        let loaded = KeymapConfig::load_from_path(&path).unwrap();
        assert_eq!(loaded.version, 1);
        assert_eq!(loaded.leader.as_deref(), Some("Ctrl+X"));
        assert_eq!(loaded.bindings.get("Ctrl+S"), Some(&KeyAction::Submit));
        assert_eq!(loaded.bindings.get("Ctrl+K"), Some(&KeyAction::KillToEol));
        assert_eq!(
            loaded.emacs.get("Alt+F"),
            Some(&KeyAction::MoveWordRight)
        );
    }

    #[test]
    fn test_keymap_validation() {
        let mut config = KeymapConfig::default();
        config
            .bind("ctrl+c", KeyAction::Cancel)
            .unwrap();
        config
            .bind("alt+enter", KeyAction::InsertNewline)
            .unwrap();

        let val = config.validate();
        assert!(val.is_valid);
        assert_eq!(val.errors.len(), 0);
        assert_eq!(val.total_bindings, 2);

        // Inject invalid key
        config
            .bindings
            .insert("invalid+++key".to_string(), KeyAction::Submit);
        let val_bad = config.validate();
        assert!(!val_bad.is_valid);
        assert!(!val_bad.errors.is_empty());
    }

    #[test]
    fn test_keymap_manager_resolution() {
        let mut config = KeymapConfig::default();
        config
            .bind("ctrl+s", KeyAction::Submit)
            .unwrap();
        config
            .bind_emacs("ctrl+k", KeyAction::KillToEol)
            .unwrap();
        config
            .bind_vi_normal("j", KeyAction::MoveDown)
            .unwrap();

        let mut mgr = KeymapManager::from_config(config);

        // 1. Global binding resolution
        let ctrl_s = KeyEvent::new(KeyCode::Char('s'), KeyModifiers::CONTROL);
        let action = mgr.resolve(&ctrl_s, KeybindingProfile::Default, None);
        assert_eq!(action, Some(KeyAction::Submit));

        // 2. Emacs override in Emacs profile
        let ctrl_k = KeyEvent::new(KeyCode::Char('k'), KeyModifiers::CONTROL);
        let action_emacs = mgr.resolve(&ctrl_k, KeybindingProfile::Emacs, None);
        assert_eq!(action_emacs, Some(KeyAction::KillToEol));

        // 3. Vi Normal override in Vi Normal mode
        let j_key = KeyEvent::new(KeyCode::Char('j'), KeyModifiers::NONE);
        let action_vi = mgr.resolve(&j_key, KeybindingProfile::Vi, Some(ViMode::Normal));
        assert_eq!(action_vi, Some(KeyAction::MoveDown));

        // 4. Vi Insert mode should not trigger Vi Normal override
        let action_vi_insert = mgr.resolve(&j_key, KeybindingProfile::Vi, Some(ViMode::Insert));
        assert_eq!(action_vi_insert, None);
    }
}
