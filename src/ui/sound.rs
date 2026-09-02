//! Subtle terminal audio cues for Fusion.
//!
//! Emits terminal bell characters (`\x07` / `ASCII 0x07` / BEL) on notable agent
//! events such as turn completion or errors. These cues can be globally or
//! granularly enabled/disabled via configuration, environment variables, or REPL commands.

use std::fmt;
use std::io::{stderr, IsTerminal, Write};
use serde::{Deserialize, Serialize};

/// Standard ASCII bell control character (`BEL`, `\x07`).
pub const TERMINAL_BELL: &str = "\x07";

/// ASCII bell byte (`0x07`).
pub const TERMINAL_BELL_BYTE: u8 = 0x07;

/// Discrete events that can trigger an audio cue.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SoundCue {
    /// Subtle cue played when an agent conversation turn completes successfully.
    TurnComplete,
    /// Subtle cue played when an agent turn or command encounters an error.
    Error,
    /// Generic direct bell signal.
    Bell,
}

impl SoundCue {
    /// Returns the canonical name of this cue.
    pub fn name(&self) -> &'static str {
        match self {
            SoundCue::TurnComplete => "turn_complete",
            SoundCue::Error => "error",
            SoundCue::Bell => "bell",
        }
    }

    /// Returns the byte sequence sent to the terminal to produce this cue.
    pub fn bell_sequence(&self) -> &'static [u8] {
        match self {
            SoundCue::TurnComplete => &[TERMINAL_BELL_BYTE],
            SoundCue::Error => &[TERMINAL_BELL_BYTE],
            SoundCue::Bell => &[TERMINAL_BELL_BYTE],
        }
    }
}

impl fmt::Display for SoundCue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.name())
    }
}

/// Configuration settings for terminal audio cues.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SoundConfig {
    /// Master switch for terminal audio cues.
    #[serde(default = "default_false")]
    pub enabled: bool,

    /// Emit terminal bell when an agent turn successfully finishes.
    #[serde(default = "default_true")]
    pub bell_on_completion: bool,

    /// Emit terminal bell when an error occurs during execution.
    #[serde(default = "default_true")]
    pub bell_on_error: bool,

    /// Only emit audio cues if standard error is connected to an interactive terminal (TTY).
    #[serde(default = "default_true")]
    pub tty_only: bool,
}

fn default_false() -> bool {
    false
}

fn default_true() -> bool {
    true
}

impl Default for SoundConfig {
    fn default() -> Self {
        Self::from_env()
    }
}

impl SoundConfig {
    /// Creates a new `SoundConfig` with the master enabled switch set to `enabled`.
    pub fn new(enabled: bool) -> Self {
        Self {
            enabled,
            bell_on_completion: true,
            bell_on_error: true,
            tty_only: true,
        }
    }

    /// Loads sound configuration inspecting environment variables:
    /// - `FUSION_SOUND` or `FUSION_BELL`: `1`/`true`/`on`/`yes` enables, `0`/`false`/`off`/`no` disables.
    /// - `NO_BELL`: if set, disables audio cues.
    /// - `FUSION_BELL_COMPLETION`: `0`/`false` disables completion bell.
    /// - `FUSION_BELL_ERROR`: `0`/`false` disables error bell.
    /// - `TERM=dumb`: disables audio cues by default.
    pub fn from_env() -> Self {
        let mut enabled = false;

        // Check master sound / bell env vars
        for var in &["FUSION_SOUND", "FUSION_BELL", "FUSION_AUDIO_CUES", "SOUND_ENABLED"] {
            if let Ok(val) = std::env::var(var) {
                let trimmed = val.trim().to_lowercase();
                if trimmed == "1" || trimmed == "true" || trimmed == "on" || trimmed == "yes" {
                    enabled = true;
                    break;
                } else if trimmed == "0" || trimmed == "false" || trimmed == "off" || trimmed == "no" {
                    enabled = false;
                    break;
                }
            }
        }

        // NO_BELL environment variable disables audio cues
        if let Ok(val) = std::env::var("NO_BELL") {
            let trimmed = val.trim().to_lowercase();
            if trimmed != "0" && trimmed != "false" {
                enabled = false;
            }
        }

        // Dumb terminal defaults to disabled
        if let Ok(term) = std::env::var("TERM") {
            if term.trim().eq_ignore_ascii_case("dumb") && !enabled {
                enabled = false;
            }
        }

        let mut bell_on_completion = true;
        if let Ok(val) = std::env::var("FUSION_BELL_COMPLETION") {
            let trimmed = val.trim().to_lowercase();
            if trimmed == "0" || trimmed == "false" || trimmed == "off" || trimmed == "no" {
                bell_on_completion = false;
            }
        }

        let mut bell_on_error = true;
        if let Ok(val) = std::env::var("FUSION_BELL_ERROR") {
            let trimmed = val.trim().to_lowercase();
            if trimmed == "0" || trimmed == "false" || trimmed == "off" || trimmed == "no" {
                bell_on_error = false;
            }
        }

        Self {
            enabled,
            bell_on_completion,
            bell_on_error,
            tty_only: true,
        }
    }

    /// Builder method to configure bell on completion.
    pub fn with_bell_on_completion(mut self, value: bool) -> Self {
        self.bell_on_completion = value;
        self
    }

    /// Builder method to configure bell on error.
    pub fn with_bell_on_error(mut self, value: bool) -> Self {
        self.bell_on_error = value;
        self
    }

    /// Builder method to configure TTY check requirement.
    pub fn with_tty_only(mut self, value: bool) -> Self {
        self.tty_only = value;
        self
    }

    /// Returns whether the given cue is currently enabled under this configuration.
    pub fn is_cue_enabled(&self, cue: SoundCue) -> bool {
        if !self.enabled {
            return false;
        }
        match cue {
            SoundCue::TurnComplete => self.bell_on_completion,
            SoundCue::Error => self.bell_on_error,
            SoundCue::Bell => true,
        }
    }

    /// Globally enables audio cues.
    pub fn enable(&mut self) {
        self.enabled = true;
    }

    /// Globally disables audio cues.
    pub fn disable(&mut self) {
        self.enabled = false;
    }

    /// Toggles the global sound state and returns the new value.
    pub fn toggle(&mut self) -> bool {
        self.enabled = !self.enabled;
        self.enabled
    }
}

/// Plays audio cues by outputting the terminal bell character.
#[derive(Debug, Clone)]
pub struct SoundPlayer {
    config: SoundConfig,
}

impl Default for SoundPlayer {
    fn default() -> Self {
        Self::new(SoundConfig::default())
    }
}

impl SoundPlayer {
    /// Creates a new `SoundPlayer` with the given configuration.
    pub fn new(config: SoundConfig) -> Self {
        Self { config }
    }

    /// Creates a new `SoundPlayer` using environment-derived configuration.
    pub fn from_env() -> Self {
        Self::new(SoundConfig::from_env())
    }

    /// Returns a reference to the active `SoundConfig`.
    pub fn config(&self) -> &SoundConfig {
        &self.config
    }

    /// Returns a mutable reference to the active `SoundConfig`.
    pub fn config_mut(&mut self) -> &mut SoundConfig {
        &mut self.config
    }

    /// Sets whether audio cues are globally enabled.
    pub fn set_enabled(&mut self, enabled: bool) {
        self.config.enabled = enabled;
    }

    /// Emits an audio cue to standard error if enabled.
    ///
    /// Returns `true` if the terminal bell was sent, `false` if suppressed.
    pub fn play(&self, cue: SoundCue) -> bool {
        if !self.config.is_cue_enabled(cue) {
            return false;
        }

        if self.config.tty_only && !stderr().is_terminal() {
            return false;
        }

        let mut err = stderr();
        if err.write_all(cue.bell_sequence()).is_ok() {
            let _ = err.flush();
            true
        } else {
            false
        }
    }

    /// Emits an audio cue into any writable sink.
    ///
    /// Ideal for unit testing and deterministic verification without relying on TTY state.
    pub fn play_to<W: Write>(&self, cue: SoundCue, writer: &mut W) -> std::io::Result<bool> {
        if !self.config.is_cue_enabled(cue) {
            return Ok(false);
        }

        writer.write_all(cue.bell_sequence())?;
        writer.flush()?;
        Ok(true)
    }

    /// Emits the turn completion audio cue if enabled.
    pub fn play_turn_complete(&self) -> bool {
        self.play(SoundCue::TurnComplete)
    }

    /// Emits the error audio cue if enabled.
    pub fn play_error(&self) -> bool {
        self.play(SoundCue::Error)
    }

    /// Emits the unconditional terminal bell to standard error.
    pub fn ring_bell(&self) -> bool {
        self.play(SoundCue::Bell)
    }
}

/// Convenience function: plays turn completion audio cue using application config settings.
pub fn play_turn_complete(config: &crate::config::Config) -> bool {
    let player = SoundPlayer::new(config.sound_config());
    player.play_turn_complete()
}

/// Convenience function: plays error audio cue using application config settings.
pub fn play_error(config: &crate::config::Config) -> bool {
    let player = SoundPlayer::new(config.sound_config());
    player.play_error()
}

/// Convenience function: plays a given cue using application config settings.
pub fn play_cue(cue: SoundCue, config: &crate::config::Config) -> bool {
    let player = SoundPlayer::new(config.sound_config());
    player.play(cue)
}

/// Emits the terminal bell byte (`\x07`) unconditionally to the given writer and flushes.
pub fn ring_bell_to<W: Write>(writer: &mut W) -> std::io::Result<()> {
    writer.write_all(&[TERMINAL_BELL_BYTE])?;
    writer.flush()
}

/// Emits the terminal bell byte (`\x07`) unconditionally to standard error.
pub fn ring_bell() -> bool {
    let mut err = stderr();
    if err.write_all(&[TERMINAL_BELL_BYTE]).is_ok() {
        let _ = err.flush();
        true
    } else {
        false
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_bell_constants() {
        assert_eq!(TERMINAL_BELL, "\x07");
        assert_eq!(TERMINAL_BELL_BYTE, 0x07);
        assert_eq!(TERMINAL_BELL.as_bytes(), &[0x07]);
    }

    #[test]
    fn test_sound_cue_properties() {
        assert_eq!(SoundCue::TurnComplete.name(), "turn_complete");
        assert_eq!(SoundCue::Error.name(), "error");
        assert_eq!(SoundCue::Bell.name(), "bell");

        assert_eq!(SoundCue::TurnComplete.bell_sequence(), &[0x07]);
        assert_eq!(SoundCue::Error.bell_sequence(), &[0x07]);
        assert_eq!(SoundCue::Bell.bell_sequence(), &[0x07]);

        assert_eq!(format!("{}", SoundCue::TurnComplete), "turn_complete");
        assert_eq!(format!("{}", SoundCue::Error), "error");
    }

    #[test]
    fn test_sound_config_new_and_defaults() {
        let mut cfg = SoundConfig::new(false);
        assert!(!cfg.enabled);
        assert!(cfg.bell_on_completion);
        assert!(cfg.bell_on_error);
        assert!(!cfg.is_cue_enabled(SoundCue::TurnComplete));
        assert!(!cfg.is_cue_enabled(SoundCue::Error));

        cfg.enable();
        assert!(cfg.enabled);
        assert!(cfg.is_cue_enabled(SoundCue::TurnComplete));
        assert!(cfg.is_cue_enabled(SoundCue::Error));

        cfg.toggle();
        assert!(!cfg.enabled);

        cfg.toggle();
        assert!(cfg.enabled);
    }

    #[test]
    fn test_sound_config_granular_controls() {
        let mut cfg = SoundConfig::new(true);

        cfg.bell_on_completion = false;
        assert!(!cfg.is_cue_enabled(SoundCue::TurnComplete));
        assert!(cfg.is_cue_enabled(SoundCue::Error));

        cfg.bell_on_completion = true;
        cfg.bell_on_error = false;
        assert!(cfg.is_cue_enabled(SoundCue::TurnComplete));
        assert!(!cfg.is_cue_enabled(SoundCue::Error));
    }

    #[test]
    fn test_sound_player_play_to() {
        let cfg = SoundConfig::new(true).with_tty_only(false);
        let player = SoundPlayer::new(cfg);

        // Turn complete
        let mut buf = Vec::new();
        let sounded = player.play_to(SoundCue::TurnComplete, &mut buf).unwrap();
        assert!(sounded);
        assert_eq!(buf, vec![0x07]);

        // Error
        let mut buf_err = Vec::new();
        let sounded = player.play_to(SoundCue::Error, &mut buf_err).unwrap();
        assert!(sounded);
        assert_eq!(buf_err, vec![0x07]);
    }

    #[test]
    fn test_sound_player_disabled_suppresses_audio() {
        let cfg = SoundConfig::new(false).with_tty_only(false);
        let player = SoundPlayer::new(cfg);

        let mut buf = Vec::new();
        let sounded = player.play_to(SoundCue::TurnComplete, &mut buf).unwrap();
        assert!(!sounded);
        assert!(buf.is_empty());

        let mut buf_err = Vec::new();
        let sounded = player.play_to(SoundCue::Error, &mut buf_err).unwrap();
        assert!(!sounded);
        assert!(buf_err.is_empty());
    }

    #[test]
    fn test_ring_bell_to() {
        let mut buf = Vec::new();
        ring_bell_to(&mut buf).unwrap();
        assert_eq!(buf, vec![0x07]);
    }

    #[test]
    fn test_serde_json_roundtrip() {
        let cfg = SoundConfig {
            enabled: true,
            bell_on_completion: true,
            bell_on_error: false,
            tty_only: true,
        };

        let json = serde_json::to_string(&cfg).unwrap();
        let deserialized: SoundConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(cfg, deserialized);
    }
}
