//! Native Termux Android integration for Fusion.
//!
//! Provides first-class support for Termux on Android, including:
//! - **Clipboard Sync**: Synchronous and asynchronous clipboard access via `termux-clipboard-get` and `termux-clipboard-set`.
//! - **Haptic Feedback**: Configurable terminal vibration and tactical haptic cues via `termux-vibrate`.
//! - **Extra Keys Bar Configuration**: Management, parsing, formatting, and live reloading of `~/.termux/termux.properties` extra keys layouts.
//! - **Termux API Helpers**: Detection, toast popups, notifications, and battery status monitoring.

use crate::ui::sound::SoundCue;
use serde::{Deserialize, Serialize};
use std::fmt;
use std::fs;
use std::io::Write as IoWrite;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

// ---------------------------------------------------------------------------
// Error Handling
// ---------------------------------------------------------------------------

/// Errors that can occur during Termux Android operations.
#[derive(Debug, thiserror::Error)]
pub enum TermuxError {
    /// The current process is not running within a Termux Android environment.
    #[error("Not running in a Termux Android environment")]
    NotTermuxEnvironment,

    /// A required Termux API tool executable was not found.
    #[error("Termux tool '{tool}' not found. Ensure 'termux-api' package is installed ('pkg install termux-api') and Termux:API addon app is installed. {suggestion}")]
    ToolNotFound {
        /// Name of the missing tool (e.g. `termux-clipboard-get`).
        tool: String,
        /// Helpful actionable suggestion for the user.
        suggestion: String,
    },

    /// A Termux command executed but returned a non-zero exit code or error output.
    #[error("Termux command '{tool}' failed (exit code: {status:?}): {stderr}")]
    CommandFailed {
        /// Tool name.
        tool: String,
        /// Process exit status if available.
        status: Option<i32>,
        /// Standard error message output.
        stderr: String,
    },

    /// File or process I/O error.
    #[error("Termux I/O error: {0}")]
    Io(#[from] std::io::Error),

    /// JSON serialization or deserialization failure.
    #[error("Termux JSON error: {0}")]
    Json(#[from] serde_json::Error),

    /// General configuration or formatting error.
    #[error("Termux configuration error: {0}")]
    ConfigError(String),
}

// ---------------------------------------------------------------------------
// Environment & Tool Detection
// ---------------------------------------------------------------------------

/// Returns `true` if the current process is running inside Android Termux.
///
/// Checks environment variables (`TERMUX_VERSION`, `PREFIX`, `TERMUX_MAIN_PACKAGE_FORMAT`, `TERMUX_APP_PID`)
/// and canonical filesystem paths.
pub fn is_termux() -> bool {
    if std::env::var_os("TERMUX_VERSION").is_some() {
        return true;
    }
    if let Ok(prefix) = std::env::var("PREFIX") {
        if prefix.contains("com.termux") {
            return true;
        }
    }
    if std::env::var_os("TERMUX_MAIN_PACKAGE_FORMAT").is_some() {
        return true;
    }
    if std::env::var_os("TERMUX_APP_PID").is_some() {
        return true;
    }
    Path::new("/data/data/com.termux/files/usr").exists()
}

/// Returns the Termux prefix directory (`$PREFIX` or `/data/data/com.termux/files/usr`).
pub fn termux_prefix() -> PathBuf {
    std::env::var_os("PREFIX")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/data/data/com.termux/files/usr"))
}

/// Returns the Termux user home directory (`$HOME` or `/data/data/com.termux/files/home`).
pub fn termux_home() -> PathBuf {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/data/data/com.termux/files/home"))
}

/// Returns the Termux configuration directory (`~/.termux` or `$TERMUX_CONFIG_DIR`).
pub fn termux_config_dir() -> PathBuf {
    if let Some(dir) = std::env::var_os("TERMUX_CONFIG_DIR") {
        PathBuf::from(dir)
    } else {
        termux_home().join(".termux")
    }
}

/// Returns the canonical path to `~/.termux/termux.properties`.
pub fn termux_properties_path() -> PathBuf {
    termux_config_dir().join("termux.properties")
}

/// Locates a Termux executable (e.g. `termux-clipboard-get`, `termux-vibrate`).
///
/// Checks `$PREFIX/bin/<tool>`, standard `PATH`, and `/data/data/com.termux/files/usr/bin/<tool>`.
pub fn find_termux_tool(tool: &str) -> Option<PathBuf> {
    // 1. Check $PREFIX/bin/<tool>
    let prefix_bin = termux_prefix().join("bin").join(tool);
    if prefix_bin.is_file() {
        return Some(prefix_bin);
    }

    // 2. Check canonical fallback path
    let canonical = Path::new("/data/data/com.termux/files/usr/bin").join(tool);
    if canonical.is_file() {
        return Some(canonical);
    }

    // 3. Search in system PATH
    if let Some(paths) = std::env::var_os("PATH") {
        for dir in std::env::split_paths(&paths) {
            let candidate = dir.join(tool);
            if candidate.is_file() {
                return Some(candidate);
            }
        }
    }

    None
}

/// Returns `true` if the Termux:API package and tools are installed and accessible.
pub fn has_termux_api() -> bool {
    find_termux_tool("termux-clipboard-get").is_some()
        || find_termux_tool("termux-vibrate").is_some()
        || find_termux_tool("termux-toast").is_some()
}

// ---------------------------------------------------------------------------
// Clipboard Sync
// ---------------------------------------------------------------------------

/// Synchronously reads the text content from the Android system clipboard via Termux API.
///
/// Uses `termux-clipboard-get`.
pub fn get_clipboard() -> Result<String, TermuxError> {
    let tool_path = find_termux_tool("termux-clipboard-get").ok_or_else(|| {
        TermuxError::ToolNotFound {
            tool: "termux-clipboard-get".to_string(),
            suggestion: "Run 'pkg install termux-api' in Termux and ensure Termux:API app is installed.".to_string(),
        }
    })?;

    let output = Command::new(tool_path)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).to_string();
        return Err(TermuxError::CommandFailed {
            tool: "termux-clipboard-get".to_string(),
            status: output.status.code(),
            stderr,
        });
    }

    let text = String::from_utf8_lossy(&output.stdout).to_string();
    Ok(text)
}

/// Synchronously writes text to the Android system clipboard via Termux API.
///
/// Uses `termux-clipboard-set`.
pub fn set_clipboard(text: &str) -> Result<(), TermuxError> {
    let tool_path = find_termux_tool("termux-clipboard-set").ok_or_else(|| {
        TermuxError::ToolNotFound {
            tool: "termux-clipboard-set".to_string(),
            suggestion: "Run 'pkg install termux-api' in Termux and ensure Termux:API app is installed.".to_string(),
        }
    })?;

    let mut child = Command::new(tool_path)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()?;

    if let Some(mut stdin) = child.stdin.take() {
        stdin.write_all(text.as_bytes())?;
        // flush and drop to signal EOF
        stdin.flush()?;
    }

    let output = child.wait_with_output()?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).to_string();
        return Err(TermuxError::CommandFailed {
            tool: "termux-clipboard-set".to_string(),
            status: output.status.code(),
            stderr,
        });
    }

    Ok(())
}

/// Asynchronously reads the text content from the Android system clipboard via Termux API.
pub async fn get_clipboard_async() -> Result<String, TermuxError> {
    let tool_path = find_termux_tool("termux-clipboard-get").ok_or_else(|| {
        TermuxError::ToolNotFound {
            tool: "termux-clipboard-get".to_string(),
            suggestion: "Run 'pkg install termux-api' in Termux and ensure Termux:API app is installed.".to_string(),
        }
    })?;

    let output = tokio::process::Command::new(tool_path)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .await?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).to_string();
        return Err(TermuxError::CommandFailed {
            tool: "termux-clipboard-get".to_string(),
            status: output.status.code(),
            stderr,
        });
    }

    let text = String::from_utf8_lossy(&output.stdout).to_string();
    Ok(text)
}

/// Asynchronously writes text to the Android system clipboard via Termux API.
pub async fn set_clipboard_async(text: &str) -> Result<(), TermuxError> {
    let tool_path = find_termux_tool("termux-clipboard-set").ok_or_else(|| {
        TermuxError::ToolNotFound {
            tool: "termux-clipboard-set".to_string(),
            suggestion: "Run 'pkg install termux-api' in Termux and ensure Termux:API app is installed.".to_string(),
        }
    })?;

    let mut child = tokio::process::Command::new(tool_path)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()?;

    if let Some(mut stdin) = child.stdin.take() {
        use tokio::io::AsyncWriteExt;
        stdin.write_all(text.as_bytes()).await?;
        stdin.flush().await?;
    }

    let output = child.wait_with_output().await?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).to_string();
        return Err(TermuxError::CommandFailed {
            tool: "termux-clipboard-set".to_string(),
            status: output.status.code(),
            stderr,
        });
    }

    Ok(())
}

/// High-level Termux clipboard manager.
#[derive(Debug, Clone, Copy, Default)]
pub struct TermuxClipboard;

impl TermuxClipboard {
    /// Returns `true` if Termux clipboard tools are available on the current system.
    pub fn is_available() -> bool {
        find_termux_tool("termux-clipboard-get").is_some()
            && find_termux_tool("termux-clipboard-set").is_some()
    }

    /// Read text from Android clipboard synchronously.
    pub fn get() -> Result<String, TermuxError> {
        get_clipboard()
    }

    /// Write text to Android clipboard synchronously.
    pub fn set(text: &str) -> Result<(), TermuxError> {
        set_clipboard(text)
    }

    /// Clears Android clipboard synchronously.
    pub fn clear() -> Result<(), TermuxError> {
        set_clipboard("")
    }

    /// Read text from Android clipboard asynchronously.
    pub async fn get_async() -> Result<String, TermuxError> {
        get_clipboard_async().await
    }

    /// Write text to Android clipboard asynchronously.
    pub async fn set_async(text: &str) -> Result<(), TermuxError> {
        set_clipboard_async(text).await
    }

    /// Clears Android clipboard asynchronously.
    pub async fn clear_async() -> Result<(), TermuxError> {
        set_clipboard_async("").await
    }
}

// ---------------------------------------------------------------------------
// Haptic Feedback (`termux-vibrate`)
// ---------------------------------------------------------------------------

/// Intensity level and duration preset for Termux haptic feedback.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HapticIntensity {
    /// Subtle, quick click for keypresses or fast user feedback (~40ms).
    Light,
    /// Standard feedback for selection or action confirmation (~100ms).
    Medium,
    /// Strong, prominent vibration for significant events (~250ms).
    Heavy,
    /// Positive completion vibration pattern (~150ms).
    Success,
    /// Alert / error vibration pattern (~350ms).
    Error,
    /// Custom millisecond duration and force override.
    Custom {
        /// Duration of the vibration in milliseconds.
        duration_ms: u32,
        /// Whether to force vibration even if device is in silent/DND mode.
        force: bool,
    },
}

impl Default for HapticIntensity {
    fn default() -> Self {
        Self::Medium
    }
}

impl HapticIntensity {
    /// Returns the vibration duration in milliseconds.
    pub fn duration_ms(&self) -> u32 {
        match self {
            Self::Light => 40,
            Self::Medium => 100,
            Self::Heavy => 250,
            Self::Success => 150,
            Self::Error => 350,
            Self::Custom { duration_ms, .. } => *duration_ms,
        }
    }

    /// Returns whether the vibration should be forced even in silent mode.
    pub fn force(&self) -> bool {
        match self {
            Self::Custom { force, .. } => *force,
            _ => false,
        }
    }
}

impl From<SoundCue> for HapticIntensity {
    fn from(cue: SoundCue) -> Self {
        match cue {
            SoundCue::TurnComplete => HapticIntensity::Success,
            SoundCue::Error => HapticIntensity::Error,
            SoundCue::Bell => HapticIntensity::Medium,
        }
    }
}

/// Configuration settings for Termux haptic feedback.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HapticConfig {
    /// Master switch for Termux haptic vibrations.
    #[serde(default = "default_true")]
    pub enabled: bool,

    /// Vibrate on successful turn completion.
    #[serde(default = "default_true")]
    pub on_turn_complete: bool,

    /// Vibrate on error or failure.
    #[serde(default = "default_true")]
    pub on_error: bool,

    /// Vibrate on interactive prompt keypresses / shortcuts.
    #[serde(default = "default_false")]
    pub on_keypress: bool,

    /// Force vibration even in silent/DND mode.
    #[serde(default = "default_false")]
    pub force: bool,

    /// Default intensity level.
    #[serde(default)]
    pub default_intensity: HapticIntensity,
}

const fn default_true() -> bool {
    true
}

const fn default_false() -> bool {
    false
}

impl Default for HapticConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            on_turn_complete: true,
            on_error: true,
            on_keypress: false,
            force: false,
            default_intensity: HapticIntensity::Medium,
        }
    }
}

impl HapticConfig {
    /// Creates a new `HapticConfig` with the master enabled switch set.
    pub fn new(enabled: bool) -> Self {
        Self {
            enabled,
            ..Default::default()
        }
    }

    /// Enables haptic feedback.
    pub fn enable(&mut self) {
        self.enabled = true;
    }

    /// Disables haptic feedback.
    pub fn disable(&mut self) {
        self.enabled = false;
    }

    /// Toggles haptic feedback.
    pub fn toggle(&mut self) -> bool {
        self.enabled = !self.enabled;
        self.enabled
    }

    /// Returns `true` if haptics should trigger for the specified cue.
    pub fn is_cue_enabled(&self, cue: SoundCue) -> bool {
        if !self.enabled {
            return false;
        }
        match cue {
            SoundCue::TurnComplete => self.on_turn_complete,
            SoundCue::Error => self.on_error,
            SoundCue::Bell => true,
        }
    }
}

/// Synchronously triggers a Termux vibration with the given duration and force flag.
pub fn vibrate(duration_ms: u32, force: bool) -> Result<(), TermuxError> {
    let tool_path = find_termux_tool("termux-vibrate").ok_or_else(|| {
        TermuxError::ToolNotFound {
            tool: "termux-vibrate".to_string(),
            suggestion: "Run 'pkg install termux-api' in Termux and ensure Termux:API app is installed.".to_string(),
        }
    })?;

    let mut cmd = Command::new(tool_path);
    cmd.arg("-d").arg(duration_ms.to_string());
    if force {
        cmd.arg("-f");
    }

    let output = cmd
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .output()?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).to_string();
        return Err(TermuxError::CommandFailed {
            tool: "termux-vibrate".to_string(),
            status: output.status.code(),
            stderr,
        });
    }

    Ok(())
}

/// Asynchronously triggers a Termux vibration in the background without blocking terminal execution.
pub fn vibrate_async(duration_ms: u32, force: bool) {
    std::thread::spawn(move || {
        let _ = vibrate(duration_ms, force);
    });
}

/// Controller for Termux haptic feedback cues.
#[derive(Debug, Clone)]
pub struct TermuxHaptics {
    config: HapticConfig,
}

impl Default for TermuxHaptics {
    fn default() -> Self {
        Self::new(HapticConfig::default())
    }
}

impl TermuxHaptics {
    /// Creates a new `TermuxHaptics` instance with the given configuration.
    pub fn new(config: HapticConfig) -> Self {
        Self { config }
    }

    /// Returns a reference to the active haptics configuration.
    pub fn config(&self) -> &HapticConfig {
        &self.config
    }

    /// Returns a mutable reference to the haptics configuration.
    pub fn config_mut(&mut self) -> &mut HapticConfig {
        &mut self.config
    }

    /// Returns `true` if `termux-vibrate` is present.
    pub fn is_available() -> bool {
        find_termux_tool("termux-vibrate").is_some()
    }

    /// Triggers a vibration for the given intensity level synchronously.
    ///
    /// Returns `Ok(true)` if vibration was executed, `Ok(false)` if suppressed by config.
    pub fn trigger(&self, intensity: HapticIntensity) -> Result<bool, TermuxError> {
        if !self.config.enabled {
            return Ok(false);
        }

        let duration_ms = intensity.duration_ms();
        let force = self.config.force || intensity.force();
        vibrate(duration_ms, force)?;
        Ok(true)
    }

    /// Triggers vibration for a `SoundCue` (e.g. TurnComplete, Error).
    pub fn trigger_for_cue(&self, cue: SoundCue) -> Result<bool, TermuxError> {
        if !self.config.is_cue_enabled(cue) {
            return Ok(false);
        }
        let intensity = HapticIntensity::from(cue);
        self.trigger(intensity)
    }

    /// Triggers a non-blocking background vibration for the given intensity.
    pub fn trigger_async(&self, intensity: HapticIntensity) {
        if !self.config.enabled {
            return;
        }
        let duration_ms = intensity.duration_ms();
        let force = self.config.force || intensity.force();
        vibrate_async(duration_ms, force);
    }

    /// Triggers a non-blocking background vibration for a `SoundCue`.
    pub fn trigger_for_cue_async(&self, cue: SoundCue) {
        if !self.config.is_cue_enabled(cue) {
            return;
        }
        let intensity = HapticIntensity::from(cue);
        self.trigger_async(intensity);
    }
}

// ---------------------------------------------------------------------------
// Extra Keys Bar Configuration
// ---------------------------------------------------------------------------

/// Representation of a single key button in the Termux extra keys row.
///
/// In Termux, keys can be a simple string (e.g. `"ESC"`, `"TAB"`, `"CTRL"`, `"ALT"`, `"/"`)
/// or a detailed object with popup (swipe-up) action, custom display text, or macro sequence.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ExtraKey {
    /// Simple key definition by name or character (e.g. `"ESC"`, `"TAB"`, `"/"`).
    Simple(String),
    /// Detailed key with optional popup action on swipe up, display label, or macro.
    Detailed {
        /// Primary key character or control name pressed on tap.
        key: String,
        /// Secondary character or control name typed when swiping up on the key.
        #[serde(skip_serializing_if = "Option::is_none")]
        popup: Option<String>,
        /// Display text shown on the keycap instead of `key`.
        #[serde(skip_serializing_if = "Option::is_none")]
        display: Option<String>,
        /// Macro string sent to terminal when pressed.
        #[serde(skip_serializing_if = "Option::is_none")]
        #[serde(rename = "macro")]
        macro_seq: Option<String>,
    },
}

impl ExtraKey {
    /// Creates a simple single-action extra key.
    pub fn simple(key: impl Into<String>) -> Self {
        Self::Simple(key.into())
    }

    /// Creates an extra key with a primary tap key and a swipe-up popup key.
    pub fn with_popup(key: impl Into<String>, popup: impl Into<String>) -> Self {
        Self::Detailed {
            key: key.into(),
            popup: Some(popup.into()),
            display: None,
            macro_seq: None,
        }
    }

    /// Creates a detailed extra key with full customization.
    pub fn detailed(
        key: impl Into<String>,
        popup: Option<String>,
        display: Option<String>,
        macro_seq: Option<String>,
    ) -> Self {
        Self::Detailed {
            key: key.into(),
            popup,
            display,
            macro_seq,
        }
    }

    /// Returns the primary key name or character.
    pub fn key_name(&self) -> &str {
        match self {
            Self::Simple(k) => k.as_str(),
            Self::Detailed { key, .. } => key.as_str(),
        }
    }

    /// Returns the popup key name if defined.
    pub fn popup_name(&self) -> Option<&str> {
        match self {
            Self::Simple(_) => None,
            Self::Detailed { popup, .. } => popup.as_deref(),
        }
    }

    /// Returns the custom display label if defined.
    pub fn display_label(&self) -> Option<&str> {
        match self {
            Self::Simple(_) => None,
            Self::Detailed { display, .. } => display.as_deref(),
        }
    }
}

impl From<&str> for ExtraKey {
    fn from(s: &str) -> Self {
        Self::Simple(s.to_string())
    }
}

impl From<String> for ExtraKey {
    fn from(s: String) -> Self {
        Self::Simple(s)
    }
}

impl From<(&str, &str)> for ExtraKey {
    fn from((key, popup): (&str, &str)) -> Self {
        Self::with_popup(key, popup)
    }
}

/// A complete 2D layout of Termux extra keys (rows of buttons).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExtraKeysLayout {
    /// Rows of extra key buttons.
    pub rows: Vec<Vec<ExtraKey>>,
}

impl ExtraKeysLayout {
    /// Creates a layout with the given rows.
    pub fn new(rows: Vec<Vec<ExtraKey>>) -> Self {
        Self { rows }
    }

    /// Returns the standard minimal 1-row layout:
    /// `[ESC, TAB, CTRL, ALT, LEFT, DOWN, UP, RIGHT]`
    pub fn default_minimal() -> Self {
        Self {
            rows: vec![vec![
                ExtraKey::simple("ESC"),
                ExtraKey::simple("TAB"),
                ExtraKey::simple("CTRL"),
                ExtraKey::simple("ALT"),
                ExtraKey::simple("LEFT"),
                ExtraKey::simple("DOWN"),
                ExtraKey::simple("UP"),
                ExtraKey::simple("RIGHT"),
            ]],
        }
    }

    /// Returns an optimized 2-row layout tailored for Fusion AI coding and rapid mobile navigation:
    ///
    /// Row 1: ESC (popup TAB), CTRL (popup ALT), `/` (popup `\`), `-` (popup `_`), `'` (popup `"`), `` ` `` (popup `~`), UP, END
    /// Row 2: `|` (popup `&`), `$` (popup `#`), `(` (popup `)`), `[` (popup `]`), `{` (popup `}`), LEFT, DOWN, RIGHT, ENTER
    pub fn fusion_coding() -> Self {
        Self {
            rows: vec![
                vec![
                    ExtraKey::with_popup("ESC", "TAB"),
                    ExtraKey::with_popup("CTRL", "ALT"),
                    ExtraKey::with_popup("/", "\\"),
                    ExtraKey::with_popup("-", "_"),
                    ExtraKey::with_popup("'", "\""),
                    ExtraKey::with_popup("`", "~"),
                    ExtraKey::simple("UP"),
                    ExtraKey::simple("END"),
                ],
                vec![
                    ExtraKey::with_popup("|", "&"),
                    ExtraKey::with_popup("$", "#"),
                    ExtraKey::with_popup("(", ")"),
                    ExtraKey::with_popup("[", "]"),
                    ExtraKey::with_popup("{", "}"),
                    ExtraKey::simple("LEFT"),
                    ExtraKey::simple("DOWN"),
                    ExtraKey::simple("RIGHT"),
                    ExtraKey::simple("ENTER"),
                ],
            ],
        }
    }

    /// Returns a compact 1-row layout with swipe-up popups for all essential coding symbols:
    ///
    /// ESC (popup TAB), CTRL (popup ALT), `/` (popup `\`), `-` (popup `_`), `|` (popup `&`),
    /// LEFT (popup HOME), UP (popup PGUP), DOWN (popup PGDN), RIGHT (popup END)
    pub fn compact() -> Self {
        Self {
            rows: vec![vec![
                ExtraKey::with_popup("ESC", "TAB"),
                ExtraKey::with_popup("CTRL", "ALT"),
                ExtraKey::with_popup("/", "\\"),
                ExtraKey::with_popup("-", "_"),
                ExtraKey::with_popup("|", "&"),
                ExtraKey::with_popup("LEFT", "HOME"),
                ExtraKey::with_popup("UP", "PGUP"),
                ExtraKey::with_popup("DOWN", "PGDN"),
                ExtraKey::with_popup("RIGHT", "END"),
            ]],
        }
    }

    /// Returns a 2-row layout optimized for Vi/Vim modal editing:
    ///
    /// Row 1: ESC, `:`, `/`, `?`, `-`, `_`, `~`, `^`, UP, PGUP
    /// Row 2: TAB, CTRL, ALT, `[`, `]`, `{`, `}`, LEFT, DOWN, RIGHT, PGDN
    pub fn vim() -> Self {
        Self {
            rows: vec![
                vec![
                    ExtraKey::simple("ESC"),
                    ExtraKey::simple(":"),
                    ExtraKey::simple("/"),
                    ExtraKey::simple("?"),
                    ExtraKey::simple("-"),
                    ExtraKey::simple("_"),
                    ExtraKey::simple("~"),
                    ExtraKey::simple("^"),
                    ExtraKey::simple("UP"),
                    ExtraKey::simple("PGUP"),
                ],
                vec![
                    ExtraKey::simple("TAB"),
                    ExtraKey::simple("CTRL"),
                    ExtraKey::simple("ALT"),
                    ExtraKey::simple("["),
                    ExtraKey::simple("]"),
                    ExtraKey::simple("{"),
                    ExtraKey::simple("}"),
                    ExtraKey::simple("LEFT"),
                    ExtraKey::simple("DOWN"),
                    ExtraKey::simple("RIGHT"),
                    ExtraKey::simple("PGDN"),
                ],
            ],
        }
    }

    /// Converts the layout to a JSON value representation.
    pub fn to_json_value(&self) -> serde_json::Value {
        serde_json::to_value(&self.rows).unwrap_or(serde_json::Value::Array(Vec::new()))
    }

    /// Formats the layout as a Termux properties value string.
    ///
    /// Multi-row layouts are formatted with clean backslash-newline indentation
    /// as expected in `~/.termux/termux.properties`.
    pub fn to_properties_value(&self) -> String {
        if self.rows.is_empty() {
            return "[]".to_string();
        }

        if self.rows.len() == 1 {
            let row_json = serde_json::to_string(&self.rows[0]).unwrap_or_else(|_| "[]".to_string());
            return format!("[{}]", row_json);
        }

        let mut lines = Vec::new();
        lines.push("[ \\".to_string());
        for (i, row) in self.rows.iter().enumerate() {
            let row_json = serde_json::to_string(row).unwrap_or_else(|_| "[]".to_string());
            let comma = if i + 1 < self.rows.len() { "," } else { "" };
            lines.push(format!("  {}{cmd} \\", row_json, cmd = comma));
        }
        lines.push("]".to_string());
        lines.join("\n")
    }

    /// Parses an `ExtraKeysLayout` from a JSON or properties value string.
    pub fn parse_properties_value(s: &str) -> Result<Self, TermuxError> {
        // Remove line continuations and normalize
        let cleaned: String = s
            .lines()
            .map(|l| {
                let trimmed = l.trim();
                trimmed.strip_suffix('\\').unwrap_or(trimmed).trim()
            })
            .collect::<Vec<_>>()
            .join(" ");

        let cleaned = cleaned.trim();
        if cleaned.is_empty() {
            return Ok(Self { rows: Vec::new() });
        }

        // Parse JSON array of arrays
        let rows: Vec<Vec<ExtraKey>> = serde_json::from_str(cleaned).map_err(|e| {
            TermuxError::ConfigError(format!(
                "Failed to parse extra-keys JSON: {}. Input: '{}'",
                e, cleaned
            ))
        })?;

        Ok(Self { rows })
    }
}

// ---------------------------------------------------------------------------
// Termux Properties Manager (`~/.termux/termux.properties`)
// ---------------------------------------------------------------------------

/// Single line entry in a `termux.properties` file, preserving comments, blanks, and formatting.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PropertyEntry {
    /// A comment line starting with `#` or `!`.
    Comment(String),
    /// A blank or whitespace-only line.
    Blank,
    /// A key-value configuration entry.
    KeyValue {
        /// Property key (e.g. `extra-keys`, `bell-character`, `use-black-ui`).
        key: String,
        /// Raw property value.
        value: String,
    },
}

/// Manager for Termux settings stored in `~/.termux/termux.properties`.
///
/// Preserves comments, ordering, and existing custom properties while providing
/// structured methods to update extra keys, bell behavior, and live-reload settings.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TermuxProperties {
    /// In-order entries of the properties file.
    pub entries: Vec<PropertyEntry>,
    /// File path this configuration was loaded from or will be saved to.
    pub path: PathBuf,
}

impl Default for TermuxProperties {
    fn default() -> Self {
        Self {
            entries: Vec::new(),
            path: termux_properties_path(),
        }
    }
}

impl TermuxProperties {
    /// Creates an empty `TermuxProperties` targeting the canonical path `~/.termux/termux.properties`.
    pub fn new() -> Self {
        Self::default()
    }

    /// Loads and parses the `termux.properties` from the canonical path.
    ///
    /// If the file does not exist, returns a new empty `TermuxProperties` with the canonical path.
    pub fn load() -> Result<Self, TermuxError> {
        let path = termux_properties_path();
        Self::load_from(&path)
    }

    /// Loads and parses `termux.properties` from a specific filesystem path.
    pub fn load_from(path: &Path) -> Result<Self, TermuxError> {
        if !path.exists() {
            return Ok(Self {
                entries: Vec::new(),
                path: path.to_path_buf(),
            });
        }

        let content = fs::read_to_string(path)?;
        let mut props = Self::parse_str(&content);
        props.path = path.to_path_buf();
        Ok(props)
    }

    /// Parses a properties file content string into `TermuxProperties`.
    pub fn parse_str(content: &str) -> Self {
        let mut entries = Vec::new();
        let lines: Vec<&str> = content.lines().collect();
        let mut idx = 0;

        while idx < lines.len() {
            let line = lines[idx];
            let trimmed = line.trim();

            if trimmed.is_empty() {
                entries.push(PropertyEntry::Blank);
                idx += 1;
                continue;
            }

            if trimmed.starts_with('#') || trimmed.starts_with('!') {
                entries.push(PropertyEntry::Comment(line.to_string()));
                idx += 1;
                continue;
            }

            // Find key-value delimiter ('=' or ':')
            let delim_pos = line.find('=').or_else(|| line.find(':'));
            if let Some(pos) = delim_pos {
                let key = line[..pos].trim().to_string();
                let mut value_part = line[pos + 1..].trim().to_string();

                // Check for multiline continuation with backslash
                while value_part.ends_with('\\') && idx + 1 < lines.len() {
                    idx += 1;
                    let next_line = lines[idx];
                    value_part.push('\n');
                    value_part.push_str(next_line.trim());
                }

                entries.push(PropertyEntry::KeyValue {
                    key,
                    value: value_part,
                });
            } else {
                // Unrecognized line format, preserve as raw comment/line
                entries.push(PropertyEntry::Comment(line.to_string()));
            }

            idx += 1;
        }

        Self {
            entries,
            path: termux_properties_path(),
        }
    }

    /// Returns the raw value of a property key if present.
    pub fn get_property(&self, key: &str) -> Option<&str> {
        for entry in &self.entries {
            if let PropertyEntry::KeyValue {
                key: k,
                value: v,
            } = entry
            {
                if k == key {
                    return Some(v.as_str());
                }
            }
        }
        None
    }

    /// Sets or updates a key-value property.
    ///
    /// If the key exists, its value is updated in place. If not, it is appended to the end.
    pub fn set_property(&mut self, key: &str, value: &str) {
        for entry in &mut self.entries {
            if let PropertyEntry::KeyValue {
                key: k,
                value: v,
            } = entry
            {
                if k == key {
                    *v = value.to_string();
                    return;
                }
            }
        }

        self.entries.push(PropertyEntry::KeyValue {
            key: key.to_string(),
            value: value.to_string(),
        });
    }

    /// Removes a property key if present.
    pub fn remove_property(&mut self, key: &str) -> bool {
        let initial_len = self.entries.len();
        self.entries.retain(|entry| match entry {
            PropertyEntry::KeyValue { key: k, .. } => k != key,
            _ => true,
        });
        self.entries.len() < initial_len
    }

    /// Gets and parses the configured `extra-keys` layout if present.
    pub fn get_extra_keys(&self) -> Option<ExtraKeysLayout> {
        let raw = self.get_property("extra-keys")?;
        ExtraKeysLayout::parse_properties_value(raw).ok()
    }

    /// Sets the `extra-keys` layout.
    pub fn set_extra_keys(&mut self, layout: &ExtraKeysLayout) {
        let formatted = layout.to_properties_value();
        self.set_property("extra-keys", &formatted);
    }

    /// Sets the `extra-keys-style` property (e.g. `"default"`, `"arrows-only"`, `"reload-settings"`).
    pub fn set_extra_keys_style(&mut self, style: &str) {
        self.set_property("extra-keys-style", style);
    }

    /// Sets the `bell-character` property (e.g. `"vibrate"`, `"beep"`, `"ignore"`).
    pub fn set_bell_character(&mut self, bell: &str) {
        self.set_property("bell-character", bell);
    }

    /// Formats the complete properties file content as a string.
    pub fn format_properties(&self) -> String {
        let mut out = String::new();
        for (i, entry) in self.entries.iter().enumerate() {
            match entry {
                PropertyEntry::Comment(c) => {
                    out.push_str(c);
                    out.push('\n');
                }
                PropertyEntry::Blank => {
                    out.push('\n');
                }
                PropertyEntry::KeyValue { key, value } => {
                    out.push_str(key);
                    out.push_str(" = ");
                    out.push_str(value);
                    out.push('\n');
                }
            }
            if i + 1 == self.entries.len() && !out.ends_with('\n') {
                out.push('\n');
            }
        }
        out
    }

    /// Saves the properties to the current path (`self.path`).
    pub fn save(&self) -> Result<(), TermuxError> {
        self.save_to(&self.path)
    }

    /// Saves the properties to a specified filesystem path, creating directories if needed.
    pub fn save_to(&self, path: &Path) -> Result<(), TermuxError> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let content = self.format_properties();
        fs::write(path, content)?;
        Ok(())
    }

    /// Creates a timestamped or `.bak` backup copy of `~/.termux/termux.properties` before modification.
    pub fn backup(&self) -> Result<Option<PathBuf>, TermuxError> {
        if !self.path.exists() {
            return Ok(None);
        }

        let backup_path = self.path.with_extension("properties.bak");
        fs::copy(&self.path, &backup_path)?;
        Ok(Some(backup_path))
    }

    /// Writes properties to disk and invokes `termux-reload-settings` to apply immediately in Termux.
    pub fn apply(&self) -> Result<(), TermuxError> {
        self.save()?;
        termux_reload_settings()?;
        Ok(())
    }

    /// Configures Termux properties with Fusion's recommended mobile coding settings:
    /// - Creates a backup of existing `~/.termux/termux.properties`
    /// - Applies the optimized `fusion_coding()` extra-keys bar layout
    /// - Configures terminal bell vibration (`bell-character = vibrate`) if not already set
    /// - Live reloads Termux UI via `termux-reload-settings`
    pub fn configure_for_fusion() -> Result<Self, TermuxError> {
        let mut props = Self::load()?;
        props.backup()?;

        // Apply fusion extra keys
        props.set_extra_keys(&ExtraKeysLayout::fusion_coding());

        // Configure bell vibration if not already explicitly customized
        if props.get_property("bell-character").is_none() {
            props.set_bell_character("vibrate");
        }

        props.apply()?;
        Ok(props)
    }
}

impl fmt::Display for TermuxProperties {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.format_properties())
    }
}

// ---------------------------------------------------------------------------
// Native Termux Helpers (Reload, Toast, Notifications, Battery)
// ---------------------------------------------------------------------------

/// Invokes `termux-reload-settings` to refresh Termux configuration and extra keys bar.
pub fn termux_reload_settings() -> Result<(), TermuxError> {
    if let Some(tool_path) = find_termux_tool("termux-reload-settings") {
        let output = Command::new(tool_path)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .output()?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr).to_string();
            return Err(TermuxError::CommandFailed {
                tool: "termux-reload-settings".to_string(),
                status: output.status.code(),
                stderr,
            });
        }
    }
    Ok(())
}

/// Displays an Android toast message via `termux-toast`.
pub fn termux_toast(
    message: &str,
    short: bool,
    background_color: Option<&str>,
    text_color: Option<&str>,
) -> Result<(), TermuxError> {
    let tool_path = find_termux_tool("termux-toast").ok_or_else(|| {
        TermuxError::ToolNotFound {
            tool: "termux-toast".to_string(),
            suggestion: "Run 'pkg install termux-api' in Termux and ensure Termux:API app is installed.".to_string(),
        }
    })?;

    let mut cmd = Command::new(tool_path);
    if short {
        cmd.arg("-s");
    }
    if let Some(bg) = background_color {
        cmd.arg("-b").arg(bg);
    }
    if let Some(tc) = text_color {
        cmd.arg("-c").arg(tc);
    }
    cmd.arg(message);

    let output = cmd
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .output()?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).to_string();
        return Err(TermuxError::CommandFailed {
            tool: "termux-toast".to_string(),
            status: output.status.code(),
            stderr,
        });
    }

    Ok(())
}

/// Sends an Android status bar notification via `termux-notification`.
pub fn termux_notification(
    title: &str,
    content: &str,
    id: Option<&str>,
    priority: Option<&str>,
) -> Result<(), TermuxError> {
    let tool_path = find_termux_tool("termux-notification").ok_or_else(|| {
        TermuxError::ToolNotFound {
            tool: "termux-notification".to_string(),
            suggestion: "Run 'pkg install termux-api' in Termux and ensure Termux:API app is installed.".to_string(),
        }
    })?;

    let mut cmd = Command::new(tool_path);
    cmd.arg("--title").arg(title);
    cmd.arg("--content").arg(content);
    if let Some(notification_id) = id {
        cmd.arg("--id").arg(notification_id);
    }
    if let Some(prio) = priority {
        cmd.arg("--priority").arg(prio);
    }

    let output = cmd
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .output()?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).to_string();
        return Err(TermuxError::CommandFailed {
            tool: "termux-notification".to_string(),
            status: output.status.code(),
            stderr,
        });
    }

    Ok(())
}

/// Android battery information returned by `termux-battery-status`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TermuxBatteryInfo {
    /// Battery charge level percentage (0 - 100).
    #[serde(default)]
    pub percentage: u8,
    /// Battery charging status (`"CHARGING"`, `"DISCHARGING"`, `"FULL"`, `"NOT_CHARGING"`).
    #[serde(default)]
    pub status: String,
    /// Battery health (`"GOOD"`, `"OVERHEAT"`, `"DEAD"`, etc.).
    #[serde(default)]
    pub health: String,
    /// Battery temperature in degrees Celsius.
    #[serde(default)]
    pub temperature: f64,
    /// Power connection type (`"AC"`, `"USB"`, `"WIRELESS"`, `"UNPLUGGED"`).
    #[serde(default)]
    pub plugged: String,
    /// Battery current in microamperes if available.
    #[serde(default)]
    pub current: Option<i64>,
}

impl TermuxBatteryInfo {
    /// Returns `true` if battery level is low (<= 15%) and the device is discharging.
    pub fn is_low_battery(&self) -> bool {
        self.percentage <= 15 && !self.is_charging()
    }

    /// Returns `true` if the battery is actively receiving power or full.
    pub fn is_charging(&self) -> bool {
        self.status.eq_ignore_ascii_case("CHARGING") || self.status.eq_ignore_ascii_case("FULL")
    }
}

/// Synchronously queries Android battery state using `termux-battery-status`.
pub fn termux_battery_status() -> Result<TermuxBatteryInfo, TermuxError> {
    let tool_path = find_termux_tool("termux-battery-status").ok_or_else(|| {
        TermuxError::ToolNotFound {
            tool: "termux-battery-status".to_string(),
            suggestion: "Run 'pkg install termux-api' in Termux and ensure Termux:API app is installed.".to_string(),
        }
    })?;

    let output = Command::new(tool_path)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).to_string();
        return Err(TermuxError::CommandFailed {
            tool: "termux-battery-status".to_string(),
            status: output.status.code(),
            stderr,
        });
    }

    let info: TermuxBatteryInfo = serde_json::from_slice(&output.stdout)?;
    Ok(info)
}

// ---------------------------------------------------------------------------
// Unit Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_termux_paths_and_env() {
        let prefix = termux_prefix();
        assert!(!prefix.as_os_str().is_empty());

        let home = termux_home();
        assert!(!home.as_os_str().is_empty());

        let cfg_dir = termux_config_dir();
        assert!(cfg_dir.ends_with(".termux"));

        let props_path = termux_properties_path();
        assert!(props_path.ends_with("termux.properties"));
    }

    #[test]
    fn test_extra_key_constructors() {
        let simple = ExtraKey::simple("ESC");
        assert_eq!(simple.key_name(), "ESC");
        assert_eq!(simple.popup_name(), None);
        assert_eq!(simple.display_label(), None);

        let popup = ExtraKey::with_popup("/", "\\");
        assert_eq!(popup.key_name(), "/");
        assert_eq!(popup.popup_name(), Some("\\"));
        assert_eq!(popup.display_label(), None);

        let detailed = ExtraKey::detailed(
            "TAB",
            Some("BACKTAB".to_string()),
            Some("⇥".to_string()),
            Some("\t".to_string()),
        );
        assert_eq!(detailed.key_name(), "TAB");
        assert_eq!(detailed.popup_name(), Some("BACKTAB"));
        assert_eq!(detailed.display_label(), Some("⇥"));
    }

    #[test]
    fn test_extra_key_serde_json() {
        let key1 = ExtraKey::simple("ESC");
        let json1 = serde_json::to_string(&key1).unwrap();
        assert_eq!(json1, "\"ESC\"");

        let key2 = ExtraKey::with_popup("/", "\\");
        let json2 = serde_json::to_string(&key2).unwrap();
        assert!(json2.contains("\"key\":\"/\""));
        assert!(json2.contains("\"popup\":\"\\\\\""));

        let parsed1: ExtraKey = serde_json::from_str(&json1).unwrap();
        assert_eq!(parsed1, key1);

        let parsed2: ExtraKey = serde_json::from_str(&json2).unwrap();
        assert_eq!(parsed2, key2);
    }

    #[test]
    fn test_extra_keys_layout_presets() {
        let minimal = ExtraKeysLayout::default_minimal();
        assert_eq!(minimal.rows.len(), 1);
        assert_eq!(minimal.rows[0].len(), 8);

        let fusion = ExtraKeysLayout::fusion_coding();
        assert_eq!(fusion.rows.len(), 2);
        assert_eq!(fusion.rows[0].len(), 8);
        assert_eq!(fusion.rows[1].len(), 9);

        let compact = ExtraKeysLayout::compact();
        assert_eq!(compact.rows.len(), 1);
        assert_eq!(compact.rows[0].len(), 9);

        let vim = ExtraKeysLayout::vim();
        assert_eq!(vim.rows.len(), 2);
    }

    #[test]
    fn test_extra_keys_layout_properties_format_roundtrip() {
        let layout = ExtraKeysLayout::fusion_coding();
        let formatted = layout.to_properties_value();

        assert!(formatted.starts_with("[ \\"));
        assert!(formatted.ends_with(']'));

        let parsed = ExtraKeysLayout::parse_properties_value(&formatted).unwrap();
        assert_eq!(parsed, layout);
    }

    #[test]
    fn test_termux_properties_parse_and_format() {
        let content = r#"# Termux properties configuration
# Optimized for mobile
extra-keys-style = default
bell-character = vibrate

# Extra keys definition
extra-keys = [ \
  ["ESC","TAB","CTRL","ALT","LEFT","DOWN","UP","RIGHT"] \
]

use-black-ui = true
"#;

        let props = TermuxProperties::parse_str(content);
        assert_eq!(props.get_property("extra-keys-style"), Some("default"));
        assert_eq!(props.get_property("bell-character"), Some("vibrate"));
        assert_eq!(props.get_property("use-black-ui"), Some("true"));

        let extra_keys = props.get_extra_keys().expect("parsed extra keys");
        assert_eq!(extra_keys.rows.len(), 1);
        assert_eq!(extra_keys.rows[0].len(), 8);

        let formatted = props.format_properties();
        assert!(formatted.contains("bell-character = vibrate"));
        assert!(formatted.contains("use-black-ui = true"));
    }

    #[test]
    fn test_termux_properties_mutation() {
        let mut props = TermuxProperties::new();
        props.set_property("bell-character", "vibrate");
        props.set_property("terminal-transcript-rows", "2000");

        assert_eq!(props.get_property("bell-character"), Some("vibrate"));
        assert_eq!(
            props.get_property("terminal-transcript-rows"),
            Some("2000")
        );

        props.set_extra_keys(&ExtraKeysLayout::compact());
        let layout = props.get_extra_keys().unwrap();
        assert_eq!(layout, ExtraKeysLayout::compact());

        let removed = props.remove_property("terminal-transcript-rows");
        assert!(removed);
        assert_eq!(props.get_property("terminal-transcript-rows"), None);
    }

    #[test]
    fn test_haptic_intensity_durations_and_cue_mapping() {
        assert_eq!(HapticIntensity::Light.duration_ms(), 40);
        assert_eq!(HapticIntensity::Medium.duration_ms(), 100);
        assert_eq!(HapticIntensity::Heavy.duration_ms(), 250);
        assert_eq!(HapticIntensity::Success.duration_ms(), 150);
        assert_eq!(HapticIntensity::Error.duration_ms(), 350);

        let custom = HapticIntensity::Custom {
            duration_ms: 500,
            force: true,
        };
        assert_eq!(custom.duration_ms(), 500);
        assert!(custom.force());

        assert_eq!(
            HapticIntensity::from(SoundCue::TurnComplete),
            HapticIntensity::Success
        );
        assert_eq!(
            HapticIntensity::from(SoundCue::Error),
            HapticIntensity::Error
        );
        assert_eq!(
            HapticIntensity::from(SoundCue::Bell),
            HapticIntensity::Medium
        );
    }

    #[test]
    fn test_haptic_config_controls() {
        let mut cfg = HapticConfig::default();
        assert!(cfg.enabled);
        assert!(cfg.is_cue_enabled(SoundCue::TurnComplete));
        assert!(cfg.is_cue_enabled(SoundCue::Error));

        cfg.disable();
        assert!(!cfg.enabled);
        assert!(!cfg.is_cue_enabled(SoundCue::TurnComplete));

        cfg.toggle();
        assert!(cfg.enabled);

        cfg.on_error = false;
        assert!(cfg.is_cue_enabled(SoundCue::TurnComplete));
        assert!(!cfg.is_cue_enabled(SoundCue::Error));
    }

    #[test]
    fn test_battery_info_helpers() {
        let low_battery = TermuxBatteryInfo {
            percentage: 12,
            status: "DISCHARGING".to_string(),
            health: "GOOD".to_string(),
            temperature: 28.5,
            plugged: "UNPLUGGED".to_string(),
            current: None,
        };
        assert!(low_battery.is_low_battery());
        assert!(!low_battery.is_charging());

        let charging_battery = TermuxBatteryInfo {
            percentage: 10,
            status: "CHARGING".to_string(),
            health: "GOOD".to_string(),
            temperature: 31.0,
            plugged: "AC".to_string(),
            current: Some(1500000),
        };
        assert!(!charging_battery.is_low_battery());
        assert!(charging_battery.is_charging());
    }

    #[test]
    fn test_termux_properties_save_load_tempfile() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("termux.properties");

        let mut props = TermuxProperties::new();
        props.set_property("bell-character", "vibrate");
        props.set_extra_keys(&ExtraKeysLayout::default_minimal());
        props.save_to(&path).unwrap();

        assert!(path.exists());

        let loaded = TermuxProperties::load_from(&path).unwrap();
        assert_eq!(loaded.get_property("bell-character"), Some("vibrate"));
        let extra_keys = loaded.get_extra_keys().unwrap();
        assert_eq!(extra_keys, ExtraKeysLayout::default_minimal());
    }
}
