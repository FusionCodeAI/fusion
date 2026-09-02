//! Dynamic terminal tab and window title management via ANSI/OSC escape sequences.
//!
//! Updates terminal emulator window titles and tab labels via standard OSC 0 and OSC 2 escape sequences:
//! - **OSC 0**: `\x1b]0;{title}\x07` (sets both window title and icon/tab title across xterm, iTerm2, Kitty, WezTerm, Alacritty, Windows Terminal, Foot, Ghostty, etc.)
//! - **OSC 2**: `\x1b]2;{title}\x07` (sets window title specifically)
//! - **OSC 1**: `\x1b]1;{title}\x07` (sets icon/tab name specifically)
//!
//! Provides:
//! - Flexible title templates and styling (Default, Compact, Detailed, ModelFirst, StatusPrefix, Custom).
//! - Clean contextual components: Session name/ID, active model, provider, git branch, working directory, status (Thinking, Idle, Executing), tokens, cost.
//! - Security sanitization (stripping control chars, nested OSC/CSI escapes, BEL `\x07`, ST `\x1b\\`, newlines, nulls) and length capping to prevent terminal injection.
//! - Smart deduplication and caching in `TitleUpdater` to eliminate redundant terminal I/O.
//! - Environment variable awareness (`FUSION_TERMINAL_TITLE`, `NO_COLOR`, `CI`, `TERM=dumb`).
//! - RAII `TitleGuard` for automatic restoration on exit / drop.

use std::fmt;
use std::io::{stdout, IsTerminal, Write};
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::agent::session::Session;

// ---------------------------------------------------------------------------
// 1. Constants & Defaults
// ---------------------------------------------------------------------------

/// Default application name prefix for terminal titles.
pub const DEFAULT_APP_NAME: &str = "fusion";

/// Default maximum title length in characters to prevent terminal buffer overflow.
pub const DEFAULT_MAX_TITLE_LENGTH: usize = 128;

/// Fallback default title when resetting terminal window title.
pub const DEFAULT_FALLBACK_TITLE: &str = "fusion";

// ---------------------------------------------------------------------------
// 2. OSC Protocol & Terminator Types
// ---------------------------------------------------------------------------

/// Terminal Operating System Command (OSC) code for window/tab title manipulation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum OscType {
    /// OSC 0 (`\x1b]0;{title}\x07`) - Sets both window title and icon/tab title.
    /// Most widely supported across xterm, iTerm2, Kitty, WezTerm, Alacritty, Ghostty, Windows Terminal.
    #[default]
    Osc0,
    /// OSC 2 (`\x1b]2;{title}\x07`) - Sets window title only.
    Osc2,
    /// OSC 1 (`\x1b]1;{title}\x07`) - Sets icon / tab name only.
    Osc1,
    /// Emits both OSC 0 and OSC 2 for maximum terminal emulator compatibility.
    Both0And2,
}

impl OscType {
    /// Returns the primary numeric OSC code string (e.g. `"0"`, `"2"`, `"1"`).
    pub fn code_str(&self) -> &'static str {
        match self {
            Self::Osc0 | Self::Both0And2 => "0",
            Self::Osc2 => "2",
            Self::Osc1 => "1",
        }
    }
}

/// String terminator for terminal OSC sequences.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum OscTerminator {
    /// ASCII Bell character (`\x07` / `\a`). Standard across xterm, iTerm2, macOS Terminal, Windows Terminal.
    #[default]
    Bel,
    /// ANSI 7-bit String Terminator (`\x1b\\` / `ESC \`).
    St,
    /// Emits sequences terminated by both BEL and ST for broad terminal compatibility.
    Both,
}

impl OscTerminator {
    /// Returns the escape sequence string for this terminator.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Bel => "\x07",
            Self::St => "\x1b\\",
            Self::Both => "\x07",
        }
    }
}

// ---------------------------------------------------------------------------
// 3. Title Formatting Styles
// ---------------------------------------------------------------------------

/// Visual style preset for formatting the terminal title string.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum TitleFormatStyle {
    /// Standard format: `fusion: {session} ({model})` or `fusion ({model})`
    #[default]
    Default,
    /// Compact format: `{session} | {model}` or `{model}`
    Compact,
    /// Detailed format: `fusion: {session} [{branch}] ({model}) - {status}`
    Detailed,
    /// Model-first format: `({model}) {session} - fusion`
    ModelFirst,
    /// Status-prefix format: `● {status} | {session} ({model})`
    StatusPrefix,
    /// Minimal format: `{session}` or `{app}`
    Minimal,
    /// Custom template with placeholders: `{app}`, `{session}`, `{model}`, `{short_model}`, `{provider}`, `{branch}`, `{cwd}`, `{status}`, `{status_icon}`, `{tokens}`, `{cost}`
    Custom(String),
}

// ---------------------------------------------------------------------------
// 4. Sanitization & Escaping
// ---------------------------------------------------------------------------

/// Sanitizes a string for safe inclusion in terminal OSC escape sequences.
///
/// Strips:
/// - Control characters (`0x00..=0x1F`, `0x7F..=0x9F`, except spaces)
/// - ASCII Bell (`\x07`) and ESC (`\x1b`) to prevent terminal injection
/// - ANSI CSI (`\x1b[...]`) and OSC (`\x1b]...`) sequences
/// - Newlines, carriage returns, and tabs (converted to single space)
/// - Consecutive whitespace collapsed to a single space
/// - Truncates to `max_len` graphemes/characters preserving UTF-8 boundary
pub fn sanitize_title(input: &str, max_len: usize) -> String {
    let mut cleaned = String::with_capacity(input.len());
    let mut in_escape = false;
    let mut in_csi = false;
    let mut in_osc = false;
    let mut last_was_space = false;

    let chars: Vec<char> = input.chars().collect();
    let mut i = 0;

    while i < chars.len() {
        let c = chars[i];

        // Detect ANSI ESC sequence
        if c == '\x1b' {
            in_escape = true;
            i += 1;
            if i < chars.len() {
                let next = chars[i];
                if next == '[' {
                    in_csi = true;
                    i += 1;
                    continue;
                } else if next == ']' {
                    in_osc = true;
                    i += 1;
                    continue;
                }
            }
            continue;
        }

        if in_csi {
            // CSI ends at 0x40..=0x7E (@..=~)
            if ('@'..='~').contains(&c) {
                in_csi = false;
                in_escape = false;
            }
            i += 1;
            continue;
        }

        if in_osc {
            // OSC ends at BEL (\x07) or ST (\x1b\)
            if c == '\x07' || c == '\x1b' {
                in_osc = false;
                in_escape = false;
            }
            i += 1;
            continue;
        }

        if in_escape {
            // Skip 2-character escape sequences
            in_escape = false;
            i += 1;
            continue;
        }

        // Control characters & whitespace normalization
        if c == '\r' || c == '\n' || c == '\t' || c == ' ' {
            if !last_was_space && !cleaned.is_empty() {
                cleaned.push(' ');
                last_was_space = true;
            }
        } else if !c.is_control() && c != '\x07' && c != '\x1b' {
            cleaned.push(c);
            last_was_space = false;
        }

        i += 1;
    }

    let trimmed = cleaned.trim();
    if trimmed.chars().count() <= max_len {
        trimmed.to_string()
    } else {
        let mut truncated: String = trimmed.chars().take(max_len.saturating_sub(1)).collect();
        truncated.push('…');
        truncated
    }
}

/// Shortens a raw model ID into a concise tab-friendly name.
///
/// Examples:
/// - `anthropic/claude-3-7-sonnet-20250219` -> `claude-3.7-sonnet`
/// - `claude-3-5-sonnet-latest` -> `claude-3.5-sonnet`
/// - `openai/gpt-4o-2024-08-06` -> `gpt-4o`
/// - `deepseek/deepseek-chat` -> `deepseek-chat`
/// - `deepseek/deepseek-reasoner` -> `deepseek-r1`
/// - `google/gemini-2.0-flash-exp` -> `gemini-2.0-flash`
/// - `meta-llama/llama-3.3-70b-instruct` -> `llama-3.3-70b`
pub fn shorten_model_name(model: &str) -> String {
    let clean = model.trim();
    // Strip provider prefix if present (e.g. `anthropic/claude-...` -> `claude-...`)
    let without_provider = clean.split('/').last().unwrap_or(clean);

    // Exact matches
    match without_provider {
        "deepseek-reasoner" => return "deepseek-r1".to_string(),
        "deepseek-chat" => return "deepseek-v3".to_string(),
        "gpt-4o-mini" => return "gpt-4o-mini".to_string(),
        "gpt-4o" => return "gpt-4o".to_string(),
        "o1" => return "o1".to_string(),
        "o1-mini" => return "o1-mini".to_string(),
        "o3-mini" => return "o3-mini".to_string(),
        _ => {}
    }

    // Strip date suffixes like -20250219, -20241022, -2024-08-06, -latest
    let mut result = without_provider;
    if let Some(pos) = result.rfind("-202") {
        result = &result[..pos];
    }
    if let Some(pos) = result.rfind("-latest") {
        result = &result[..pos];
    }
    if let Some(pos) = result.rfind("-instruct") {
        result = &result[..pos];
    }

    // Convert claude-3-7 to claude-3.7 for compactness
    if result.starts_with("claude-3-7") {
        return result.replacen("claude-3-7", "claude-3.7", 1);
    }
    if result.starts_with("claude-3-5") {
        return result.replacen("claude-3-5", "claude-3.5", 1);
    }

    result.to_string()
}

// ---------------------------------------------------------------------------
// 5. TerminalTitle Context Builder
// ---------------------------------------------------------------------------

/// Builder for constructing context-rich terminal window/tab titles.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TerminalTitle {
    /// Application name prefix (e.g. `"fusion"`).
    pub app_name: String,
    /// Session name or title summary (e.g. `"Refactor Auth"`).
    pub session_name: Option<String>,
    /// Active LLM model identifier (e.g. `"claude-3.7-sonnet"`, `"gpt-4o"`).
    pub active_model: Option<String>,
    /// Provider name (e.g. `"anthropic"`, `"openai"`, `"deepseek"`).
    pub provider: Option<String>,
    /// Current git branch name (e.g. `"main"`, `"feat/terminal-title"`).
    pub git_branch: Option<String>,
    /// Working directory basename or path (e.g. `"fusion"`).
    pub working_dir: Option<String>,
    /// Operational status description (e.g. `"Thinking..."`, `"Idle"`, `"Tool: bash"`).
    pub status: Option<String>,
    /// Operational status icon (e.g. `"●"`, `"⚡"`, `"⏳"`, `"✓"`).
    pub status_icon: Option<String>,
    /// Active subagent name (e.g. `"scout"`, `"coder"`).
    pub subagent: Option<String>,
    /// Accumulated token usage count.
    pub tokens: Option<usize>,
    /// Estimated session cost in USD.
    pub cost: Option<f64>,
    /// Title format style preset.
    pub style: TitleFormatStyle,
    /// Maximum title length in characters.
    pub max_length: usize,
}

impl Default for TerminalTitle {
    fn default() -> Self {
        Self {
            app_name: DEFAULT_APP_NAME.to_string(),
            session_name: None,
            active_model: None,
            provider: None,
            git_branch: None,
            working_dir: None,
            status: None,
            status_icon: None,
            subagent: None,
            tokens: None,
            cost: None,
            style: TitleFormatStyle::Default,
            max_length: DEFAULT_MAX_TITLE_LENGTH,
        }
    }
}

impl TerminalTitle {
    /// Creates a new empty `TerminalTitle` builder.
    pub fn new() -> Self {
        Self::default()
    }

    /// Sets the application name.
    pub fn with_app_name(mut self, app_name: impl Into<String>) -> Self {
        self.app_name = app_name.into();
        self
    }

    /// Sets the session name or summary.
    pub fn with_session(mut self, session_name: impl Into<String>) -> Self {
        let s = session_name.into();
        self.session_name = if s.trim().is_empty() { None } else { Some(s) };
        self
    }

    /// Sets the session name from an optional string.
    pub fn with_opt_session(mut self, session_name: Option<impl Into<String>>) -> Self {
        self.session_name = session_name.map(|s| s.into()).filter(|s| !s.trim().is_empty());
        self
    }

    /// Sets the active model identifier.
    pub fn with_model(mut self, model: impl Into<String>) -> Self {
        let m = model.into();
        self.active_model = if m.trim().is_empty() { None } else { Some(m) };
        self
    }

    /// Sets the active model from an optional string.
    pub fn with_opt_model(mut self, model: Option<impl Into<String>>) -> Self {
        self.active_model = model.map(|m| m.into()).filter(|m| !m.trim().is_empty());
        self
    }

    /// Sets the provider name.
    pub fn with_provider(mut self, provider: impl Into<String>) -> Self {
        let p = provider.into();
        self.provider = if p.trim().is_empty() { None } else { Some(p) };
        self
    }

    /// Sets the Git branch name.
    pub fn with_branch(mut self, branch: impl Into<String>) -> Self {
        let b = branch.into();
        self.git_branch = if b.trim().is_empty() { None } else { Some(b) };
        self
    }

    /// Sets the working directory.
    pub fn with_working_dir(mut self, working_dir: impl Into<String>) -> Self {
        let d = working_dir.into();
        self.working_dir = if d.trim().is_empty() { None } else { Some(d) };
        self
    }

    /// Auto-detects working directory from current directory.
    pub fn with_current_dir(mut self) -> Self {
        if let Ok(cwd) = std::env::current_dir() {
            if let Some(name) = cwd.file_name().and_then(|n| n.to_str()) {
                self.working_dir = Some(name.to_string());
            }
        }
        self
    }

    /// Sets the operational status description (e.g. `"Thinking..."`, `"Executing"`).
    pub fn with_status(mut self, status: impl Into<String>) -> Self {
        let s = status.into();
        self.status = if s.trim().is_empty() { None } else { Some(s) };
        self
    }

    /// Sets the operational status icon (e.g. `"●"`, `"⚡"`, `"⏳"`).
    pub fn with_status_icon(mut self, icon: impl Into<String>) -> Self {
        let i = icon.into();
        self.status_icon = if i.trim().is_empty() { None } else { Some(i) };
        self
    }

    /// Sets the active subagent name.
    pub fn with_subagent(mut self, subagent: impl Into<String>) -> Self {
        let sa = subagent.into();
        self.subagent = if sa.trim().is_empty() { None } else { Some(sa) };
        self
    }

    /// Sets the accumulated token count.
    pub fn with_tokens(mut self, tokens: usize) -> Self {
        self.tokens = Some(tokens);
        self
    }

    /// Sets the estimated session cost in USD.
    pub fn with_cost(mut self, cost: f64) -> Self {
        self.cost = Some(cost);
        self
    }

    /// Sets the title formatting style preset.
    pub fn with_style(mut self, style: TitleFormatStyle) -> Self {
        self.style = style;
        self
    }

    /// Sets the maximum length limit.
    pub fn with_max_length(mut self, max_length: usize) -> Self {
        self.max_length = max_length;
        self
    }

    /// Constructs a `TerminalTitle` populated from a `Session`.
    pub fn from_session(session: &Session) -> Self {
        let session_title = session.title().map(|t| t.to_string()).or_else(|| {
            let id_str = session.id().to_string();
            Some(format!("Session {}", &id_str[..id_str.len().min(8)]))
        });

        let working_dir = session.working_dir().and_then(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .map(|s| s.to_string())
        });

        let mut title = Self::new()
            .with_opt_session(session_title)
            .with_model(session.active_model());

        if let Some(wd) = working_dir {
            title.working_dir = Some(wd);
        } else {
            title = title.with_current_dir();
        }

        let tokens = session.estimate_tokens();
        if tokens > 0 {
            title.tokens = Some(tokens);
        }

        title
    }

    /// Constructs a `TerminalTitle` populated from a `Session` with an explicit status.
    pub fn from_session_and_status(session: &Session, status: &str) -> Self {
        Self::from_session(session).with_status(status)
    }

    /// Formats the components into a single clean human-readable title string.
    pub fn format(&self) -> String {
        let model_display = self.active_model.as_deref().map(shorten_model_name);
        let raw_title = match &self.style {
            TitleFormatStyle::Default => self.format_default(model_display.as_deref()),
            TitleFormatStyle::Compact => self.format_compact(model_display.as_deref()),
            TitleFormatStyle::Detailed => self.format_detailed(model_display.as_deref()),
            TitleFormatStyle::ModelFirst => self.format_model_first(model_display.as_deref()),
            TitleFormatStyle::StatusPrefix => self.format_status_prefix(model_display.as_deref()),
            TitleFormatStyle::Minimal => self.format_minimal(),
            TitleFormatStyle::Custom(template) => {
                self.format_custom(template, model_display.as_deref())
            }
        };

        sanitize_title(&raw_title, self.max_length)
    }

    /// Renders this title as an ANSI OSC escape sequence.
    pub fn render_osc(&self, osc_type: OscType, terminator: OscTerminator) -> String {
        let title_str = self.format();
        render_osc(&title_str, osc_type, terminator)
    }

    /// Renders this title with default OSC 0 and BEL terminator (`\x1b]0;{title}\x07`).
    pub fn render_default_osc(&self) -> String {
        self.render_osc(OscType::Osc0, OscTerminator::Bel)
    }

    // --- Private formatting helpers ---

    fn format_default(&self, model: Option<&str>) -> String {
        let app = &self.app_name;
        match (&self.session_name, model) {
            (Some(session), Some(m)) => format!("{}: {} ({})", app, session, m),
            (Some(session), None) => format!("{}: {}", app, session),
            (None, Some(m)) => format!("{} ({})", app, m),
            (None, None) => app.to_string(),
        }
    }

    fn format_compact(&self, model: Option<&str>) -> String {
        match (&self.session_name, model) {
            (Some(session), Some(m)) => format!("{} | {}", session, m),
            (Some(session), None) => session.clone(),
            (None, Some(m)) => format!("{} | {}", self.app_name, m),
            (None, None) => self.app_name.clone(),
        }
    }

    fn format_detailed(&self, model: Option<&str>) -> String {
        let mut parts = Vec::new();
        parts.push(self.app_name.clone());

        if let Some(session) = &self.session_name {
            parts.push(format!(": {}", session));
        }

        if let Some(branch) = &self.git_branch {
            parts.push(format!(" [{}]", branch));
        }

        if let Some(m) = model {
            parts.push(format!(" ({})", m));
        }

        if let Some(subagent) = &self.subagent {
            parts.push(format!(" <{}>", subagent));
        }

        if let Some(status) = &self.status {
            parts.push(format!(" - {}", status));
        }

        parts.concat()
    }

    fn format_model_first(&self, model: Option<&str>) -> String {
        match (model, &self.session_name) {
            (Some(m), Some(session)) => format!("({}) {} - {}", m, session, self.app_name),
            (Some(m), None) => format!("({}) {}", m, self.app_name),
            (None, Some(session)) => format!("{} - {}", session, self.app_name),
            (None, None) => self.app_name.clone(),
        }
    }

    fn format_status_prefix(&self, model: Option<&str>) -> String {
        let icon = self.status_icon.as_deref().unwrap_or("●");
        let status = self.status.as_deref().unwrap_or("Active");

        match (&self.session_name, model) {
            (Some(session), Some(m)) => format!("{} {} | {} ({})", icon, status, session, m),
            (Some(session), None) => format!("{} {} | {}", icon, status, session),
            (None, Some(m)) => format!("{} {} | {} ({})", icon, status, self.app_name, m),
            (None, None) => format!("{} {} | {}", icon, status, self.app_name),
        }
    }

    fn format_minimal(&self) -> String {
        self.session_name
            .clone()
            .unwrap_or_else(|| self.app_name.clone())
    }

    fn format_custom(&self, template: &str, short_model: Option<&str>) -> String {
        let raw_model = self.active_model.as_deref().unwrap_or("");
        let short_m = short_model.unwrap_or(raw_model);
        let session = self.session_name.as_deref().unwrap_or("");
        let provider = self.provider.as_deref().unwrap_or("");
        let branch = self.git_branch.as_deref().unwrap_or("");
        let cwd = self.working_dir.as_deref().unwrap_or("");
        let status = self.status.as_deref().unwrap_or("");
        let status_icon = self.status_icon.as_deref().unwrap_or("");
        let subagent = self.subagent.as_deref().unwrap_or("");
        let tokens_str = self
            .tokens
            .map(|t| format_tokens_compact(t))
            .unwrap_or_default();
        let cost_str = self
            .cost
            .map(|c| format!("${:.2}", c))
            .unwrap_or_default();

        template
            .replace("{app}", &self.app_name)
            .replace("{session}", session)
            .replace("{model}", raw_model)
            .replace("{short_model}", short_m)
            .replace("{provider}", provider)
            .replace("{branch}", branch)
            .replace("{cwd}", cwd)
            .replace("{status}", status)
            .replace("{status_icon}", status_icon)
            .replace("{subagent}", subagent)
            .replace("{tokens}", &tokens_str)
            .replace("{cost}", &cost_str)
    }
}

impl fmt::Display for TerminalTitle {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.format())
    }
}

// ---------------------------------------------------------------------------
// 6. Configuration
// ---------------------------------------------------------------------------

/// Configuration options for dynamic terminal title updating.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TitleConfig {
    /// Whether terminal title updates are enabled.
    #[serde(default = "default_true")]
    pub enabled: bool,

    /// OSC protocol code variant to emit.
    #[serde(default)]
    pub osc_type: OscType,

    /// String terminator character/sequence to use.
    #[serde(default)]
    pub terminator: OscTerminator,

    /// Application name prefix.
    #[serde(default = "default_app_name")]
    pub app_name: String,

    /// Visual title format style preset.
    #[serde(default)]
    pub style: TitleFormatStyle,

    /// Maximum title length in characters.
    #[serde(default = "default_max_len")]
    pub max_length: usize,

    /// Whether to restore default terminal title when the updater is dropped or REPL exits.
    #[serde(default = "default_true")]
    pub restore_on_exit: bool,

    /// Default / fallback title string used when resetting.
    #[serde(default)]
    pub default_title: Option<String>,

    /// Whether to automatically detect git branch if not specified.
    #[serde(default = "default_true")]
    pub auto_detect_branch: bool,

    /// Whether to shorten long model IDs into compact names in titles.
    #[serde(default = "default_true")]
    pub shorten_model: bool,
}

fn default_true() -> bool {
    true
}

fn default_app_name() -> String {
    DEFAULT_APP_NAME.to_string()
}

fn default_max_len() -> usize {
    DEFAULT_MAX_TITLE_LENGTH
}

impl Default for TitleConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            osc_type: OscType::Osc0,
            terminator: OscTerminator::Bel,
            app_name: DEFAULT_APP_NAME.to_string(),
            style: TitleFormatStyle::Default,
            max_length: DEFAULT_MAX_TITLE_LENGTH,
            restore_on_exit: true,
            default_title: None,
            auto_detect_branch: true,
            shorten_model: true,
        }
    }
}

impl TitleConfig {
    /// Reads terminal title configuration from environment variables.
    ///
    /// Recognizes:
    /// - `FUSION_TERMINAL_TITLE`: `"0"`/`"false"`/`"off"` disables, `"1"`/`"true"` enables.
    /// - `FUSION_TITLE_STYLE`: `"default"`, `"compact"`, `"detailed"`, `"model_first"`, `"status_prefix"`, `"minimal"`, or custom template.
    /// - `FUSION_TITLE_OSC`: `"osc0"`, `"osc2"`, `"osc1"`, `"both"`.
    /// - `NO_COLOR`: if present and non-empty, title updates are disabled by default.
    /// - `TERM=dumb`: disables title updates.
    /// - `CI`: disables title updates.
    pub fn from_env() -> Self {
        let mut config = Self::default();

        // 1. Check CI / dumb terminal / NO_COLOR
        if std::env::var("CI").is_ok() {
            config.enabled = false;
        }
        if let Ok(term) = std::env::var("TERM") {
            if term == "dumb" {
                config.enabled = false;
            }
        }
        if let Ok(no_color) = std::env::var("NO_COLOR") {
            if !no_color.is_empty() {
                config.enabled = false;
            }
        }

        // 2. Explicit toggle: FUSION_TERMINAL_TITLE
        if let Ok(val) = std::env::var("FUSION_TERMINAL_TITLE") {
            let v = val.trim().to_lowercase();
            if v == "0" || v == "false" || v == "off" || v == "no" || v == "disable" {
                config.enabled = false;
            } else if v == "1" || v == "true" || v == "on" || v == "yes" || v == "enable" {
                config.enabled = true;
            }
        }

        // 3. Style: FUSION_TITLE_STYLE
        if let Ok(style_str) = std::env::var("FUSION_TITLE_STYLE") {
            let s = style_str.trim().to_lowercase();
            match s.as_str() {
                "default" => config.style = TitleFormatStyle::Default,
                "compact" => config.style = TitleFormatStyle::Compact,
                "detailed" => config.style = TitleFormatStyle::Detailed,
                "model_first" | "modelfirst" => config.style = TitleFormatStyle::ModelFirst,
                "status_prefix" | "statusprefix" => config.style = TitleFormatStyle::StatusPrefix,
                "minimal" => config.style = TitleFormatStyle::Minimal,
                _ => config.style = TitleFormatStyle::Custom(style_str),
            }
        }

        // 4. OSC Code: FUSION_TITLE_OSC
        if let Ok(osc_str) = std::env::var("FUSION_TITLE_OSC") {
            let o = osc_str.trim().to_lowercase();
            match o.as_str() {
                "0" | "osc0" => config.osc_type = OscType::Osc0,
                "2" | "osc2" => config.osc_type = OscType::Osc2,
                "1" | "osc1" => config.osc_type = OscType::Osc1,
                "both" | "0+2" | "all" => config.osc_type = OscType::Both0And2,
                _ => {}
            }
        }

        config
    }
}

// ---------------------------------------------------------------------------
// 7. Title Updater & Manager
// ---------------------------------------------------------------------------

/// Stateful manager that updates terminal window/tab titles with caching and deduplication.
///
/// Prevents redundant terminal I/O escape sequence emissions when the title content has not changed.
#[derive(Debug, Clone)]
pub struct TitleUpdater {
    /// Active configuration.
    config: TitleConfig,
    /// Last formatted title string emitted to terminal.
    last_title: Option<String>,
    /// Initial / original title to restore upon exit.
    original_title: Option<String>,
    /// Cached detected git branch.
    cached_branch: Option<String>,
}

impl Default for TitleUpdater {
    fn default() -> Self {
        Self::new(TitleConfig::default())
    }
}

impl TitleUpdater {
    /// Creates a new `TitleUpdater` with the given configuration.
    pub fn new(config: TitleConfig) -> Self {
        let branch = if config.auto_detect_branch {
            detect_git_branch_fast()
        } else {
            None
        };

        Self {
            config,
            last_title: None,
            original_title: None,
            cached_branch: branch,
        }
    }

    /// Creates a `TitleUpdater` configured from environment variables.
    pub fn from_env() -> Self {
        Self::new(TitleConfig::from_env())
    }

    /// Returns a reference to the active configuration.
    pub fn config(&self) -> &TitleConfig {
        &self.config
    }

    /// Returns a mutable reference to the active configuration.
    pub fn config_mut(&mut self) -> &mut TitleConfig {
        &mut self.config
    }

    /// Returns whether terminal title updating is currently enabled.
    pub fn is_enabled(&self) -> bool {
        self.config.enabled
    }

    /// Sets whether terminal title updating is enabled.
    pub fn set_enabled(&mut self, enabled: bool) {
        self.config.enabled = enabled;
    }

    /// Returns the last title string that was emitted.
    pub fn last_title(&self) -> Option<&str> {
        self.last_title.as_deref()
    }

    /// Sets a custom fallback / original title to restore on exit.
    pub fn set_default_title(&mut self, title: impl Into<String>) {
        self.original_title = Some(title.into());
    }

    /// Updates the terminal window and tab title directly with a raw string.
    ///
    /// Returns `Ok(true)` if escape sequences were emitted, `Ok(false)` if suppressed/cached.
    pub fn set_title(&mut self, title: &str) -> std::io::Result<bool> {
        let mut out = stdout();
        self.set_title_to(title, &mut out)
    }

    /// Updates the terminal title writing to an arbitrary destination writer.
    pub fn set_title_to<W: Write>(&mut self, title: &str, writer: &mut W) -> std::io::Result<bool> {
        if !self.config.enabled {
            return Ok(false);
        }

        let sanitized = sanitize_title(title, self.config.max_length);

        // Deduplication: check if unchanged
        if let Some(last) = &self.last_title {
            if last == &sanitized {
                return Ok(false);
            }
        }

        let seq = render_osc(&sanitized, self.config.osc_type, self.config.terminator);
        writer.write_all(seq.as_bytes())?;
        writer.flush()?;

        self.last_title = Some(sanitized);
        Ok(true)
    }

    /// Updates the terminal title using a `TerminalTitle` builder.
    pub fn update(&mut self, builder: &TerminalTitle) -> std::io::Result<bool> {
        let mut out = stdout();
        self.update_to(builder, &mut out)
    }

    /// Updates the terminal title using a `TerminalTitle` builder to a custom writer.
    pub fn update_to<W: Write>(
        &mut self,
        builder: &TerminalTitle,
        writer: &mut W,
    ) -> std::io::Result<bool> {
        if !self.config.enabled {
            return Ok(false);
        }

        let mut enriched = builder.clone();
        if enriched.git_branch.is_none() && self.cached_branch.is_some() {
            enriched.git_branch = self.cached_branch.clone();
        }
        if enriched.style == TitleFormatStyle::Default && self.config.style != TitleFormatStyle::Default {
            enriched.style = self.config.style.clone();
        }
        enriched.max_length = self.config.max_length;

        let title_str = enriched.format();
        self.set_title_to(&title_str, writer)
    }

    /// Updates the terminal title for a session name and active model.
    pub fn update_session_and_model(
        &mut self,
        session: Option<&str>,
        model: Option<&str>,
    ) -> std::io::Result<bool> {
        let mut builder = TerminalTitle::new()
            .with_app_name(&self.config.app_name)
            .with_style(self.config.style.clone());

        if let Some(s) = session {
            builder = builder.with_session(s);
        }
        if let Some(m) = model {
            builder = builder.with_model(m);
        }

        self.update(&builder)
    }

    /// Updates the terminal title from a `Session`.
    pub fn update_from_session(&mut self, session: &Session) -> std::io::Result<bool> {
        let mut builder = TerminalTitle::from_session(session)
            .with_app_name(&self.config.app_name)
            .with_style(self.config.style.clone());

        if let Some(branch) = &self.cached_branch {
            builder.git_branch = Some(branch.clone());
        }

        self.update(&builder)
    }

    /// Updates the terminal title from a `Session` with an activity status (e.g. `"Thinking..."`).
    pub fn update_from_session_and_status(
        &mut self,
        session: &Session,
        status: &str,
    ) -> std::io::Result<bool> {
        let mut builder = TerminalTitle::from_session_and_status(session, status)
            .with_app_name(&self.config.app_name)
            .with_style(self.config.style.clone());

        if let Some(branch) = &self.cached_branch {
            builder.git_branch = Some(branch.clone());
        }

        self.update(&builder)
    }

    /// Updates only the status of the current terminal title while keeping session/model.
    pub fn update_status(&mut self, status: Option<&str>) -> std::io::Result<bool> {
        let mut builder = TerminalTitle::new()
            .with_app_name(&self.config.app_name)
            .with_style(self.config.style.clone());

        if let Some(s) = status {
            builder = builder.with_status(s);
        }

        self.update(&builder)
    }

    /// Resets the terminal title to the default or fallback title.
    pub fn reset(&mut self) -> std::io::Result<bool> {
        let mut out = stdout();
        self.reset_to(&mut out)
    }

    /// Resets the terminal title writing to an arbitrary destination writer.
    pub fn reset_to<W: Write>(&mut self, writer: &mut W) -> std::io::Result<bool> {
        let target = self
            .original_title
            .clone()
            .or_else(|| self.config.default_title.clone())
            .unwrap_or_else(|| DEFAULT_FALLBACK_TITLE.to_string());

        self.set_title_to(&target, writer)
    }

    /// Clears the terminal window title (sets to empty string).
    pub fn clear(&mut self) -> std::io::Result<bool> {
        let mut out = stdout();
        self.clear_to(&mut out)
    }

    /// Clears the terminal window title to a writer.
    pub fn clear_to<W: Write>(&mut self, writer: &mut W) -> std::io::Result<bool> {
        self.set_title_to("", writer)
    }

    /// Creates an RAII `TitleGuard` that restores the terminal title when dropped.
    pub fn guard(&mut self) -> TitleGuard<'_> {
        TitleGuard { updater: self }
    }
}

// ---------------------------------------------------------------------------
// 8. RAII Title Guard
// ---------------------------------------------------------------------------

/// RAII guard that restores the terminal window/tab title to its original or default value upon drop.
#[derive(Debug)]
pub struct TitleGuard<'a> {
    updater: &'a mut TitleUpdater,
}

impl<'a> Drop for TitleGuard<'a> {
    fn drop(&mut self) {
        if self.updater.config.restore_on_exit {
            let _ = self.updater.reset();
        }
    }
}

// ---------------------------------------------------------------------------
// 9. Low-level OSC Rendering Helpers
// ---------------------------------------------------------------------------

/// Renders an OSC escape sequence string for setting the terminal title.
pub fn render_osc(title: &str, osc_type: OscType, terminator: OscTerminator) -> String {
    let clean = sanitize_title(title, DEFAULT_MAX_TITLE_LENGTH);
    let term_str = terminator.as_str();

    match osc_type {
        OscType::Osc0 => format!("\x1b]0;{}\x07", clean),
        OscType::Osc2 => format!("\x1b]2;{}\x07", clean),
        OscType::Osc1 => format!("\x1b]1;{}\x07", clean),
        OscType::Both0And2 => {
            format!("\x1b]0;{clean}{term_str}\x1b]2;{clean}{term_str}")
        }
    }
}

/// Renders standard OSC 0 sequence (`\x1b]0;{title}\x07`).
pub fn render_osc0(title: &str) -> String {
    render_osc(title, OscType::Osc0, OscTerminator::Bel)
}

/// Renders standard OSC 2 sequence (`\x1b]2;{title}\x07`).
pub fn render_osc2(title: &str) -> String {
    render_osc(title, OscType::Osc2, OscTerminator::Bel)
}

// ---------------------------------------------------------------------------
// 10. Global & Free Convenience Functions
// ---------------------------------------------------------------------------

/// Checks whether terminal window title updates are supported and enabled in current environment.
pub fn is_terminal_title_supported() -> bool {
    if std::env::var("CI").is_ok() {
        return false;
    }
    if let Ok(term) = std::env::var("TERM") {
        if term == "dumb" {
            return false;
        }
    }
    if let Ok(no_color) = std::env::var("NO_COLOR") {
        if !no_color.is_empty() {
            return false;
        }
    }
    if let Ok(val) = std::env::var("FUSION_TERMINAL_TITLE") {
        let v = val.trim().to_lowercase();
        if v == "0" || v == "false" || v == "off" || v == "no" {
            return false;
        }
    }
    stdout().is_terminal()
}

/// Directly sets the terminal window and tab title via standard OSC 0 (`\x1b]0;{title}\x07`).
///
/// Flushes stdout automatically. Returns `true` if emission succeeded.
pub fn set_terminal_title(title: &str) -> bool {
    let mut out = stdout();
    set_terminal_title_to(title, &mut out).is_ok()
}

/// Sets the terminal window/tab title writing to the provided writer.
pub fn set_terminal_title_to<W: Write>(title: &str, writer: &mut W) -> std::io::Result<()> {
    let osc = render_osc0(title);
    writer.write_all(osc.as_bytes())?;
    writer.flush()
}

/// Formats a standard title from optional session and model identifiers.
pub fn format_terminal_title(session: Option<&str>, model: Option<&str>) -> String {
    TerminalTitle::new()
        .with_opt_session(session)
        .with_opt_model(model)
        .format()
}

/// Directly updates the terminal title showing session name and active model.
pub fn set_session_model_title(session: Option<&str>, model: Option<&str>) -> bool {
    let formatted = format_terminal_title(session, model);
    set_terminal_title(&formatted)
}

/// Directly updates the terminal title showing session name, active model, and status.
pub fn set_session_model_status_title(
    session: Option<&str>,
    model: Option<&str>,
    status: Option<&str>,
) -> bool {
    let mut title = TerminalTitle::new()
        .with_opt_session(session)
        .with_opt_model(model);

    if let Some(s) = status {
        title = title.with_status(s);
    }

    set_terminal_title(&title.format())
}

/// Resets the terminal title to `"fusion"`.
pub fn reset_terminal_title() -> bool {
    set_terminal_title(DEFAULT_FALLBACK_TITLE)
}

/// Clears the terminal window title.
pub fn clear_terminal_title() -> bool {
    set_terminal_title("")
}

// ---------------------------------------------------------------------------
// 11. Helper Utilities
// ---------------------------------------------------------------------------

fn format_tokens_compact(tokens: usize) -> String {
    if tokens < 1_000 {
        format!("{} tok", tokens)
    } else if tokens < 1_000_000 {
        format!("{:.1}k tok", tokens as f64 / 1_000.0)
    } else {
        format!("{:.2}M tok", tokens as f64 / 1_000_000.0)
    }
}

/// Fast non-blocking Git branch detection inspecting `.git/HEAD` directly.
fn detect_git_branch_fast() -> Option<String> {
    let cwd = std::env::current_dir().ok()?;
    detect_git_branch_in_dir(&cwd)
}

fn detect_git_branch_in_dir(start_path: &Path) -> Option<String> {
    let mut current = if start_path.is_file() {
        start_path.parent()?.to_path_buf()
    } else {
        start_path.to_path_buf()
    };

    let mut git_marker_path = None;
    loop {
        let candidate = current.join(".git");
        if candidate.exists() {
            git_marker_path = Some(candidate);
            break;
        }
        if !current.pop() {
            break;
        }
    }

    let git_marker = git_marker_path?;
    let git_dir = if git_marker.is_file() {
        let content = std::fs::read_to_string(&git_marker).ok()?;
        let trimmed = content.trim();
        let target = trimmed.strip_prefix("gitdir:")?.trim();
        if Path::new(target).is_absolute() {
            std::path::PathBuf::from(target)
        } else {
            git_marker.parent()?.join(target)
        }
    } else {
        git_marker
    };

    let head_content = std::fs::read_to_string(git_dir.join("HEAD")).ok()?;
    let head_str = head_content.trim();

    if let Some(ref_path) = head_str.strip_prefix("ref:") {
        let ref_clean = ref_path.trim();
        if let Some(branch_name) = ref_clean.strip_prefix("refs/heads/") {
            return Some(branch_name.to_string());
        }
        return Some(ref_clean.to_string());
    }

    if head_str.len() >= 7 && head_str.chars().all(|c| c.is_ascii_hexdigit()) {
        return Some(format!("detached:{}", &head_str[..7]));
    }

    None
}

// ---------------------------------------------------------------------------
// 12. Unit Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sanitize_title() {
        assert_eq!(sanitize_title("Hello World", 50), "Hello World");
        assert_eq!(sanitize_title("Hello\nWorld\r\nTest", 50), "Hello World Test");
        assert_eq!(sanitize_title("Hello\x07World\x1b[31mRed\x1b[0m", 50), "HelloWorldRed");
        assert_eq!(sanitize_title("Multiple    Spaces", 50), "Multiple Spaces");
        assert_eq!(sanitize_title("   Trim Spaces   ", 50), "Trim Spaces");

        // Truncation
        let long = "a".repeat(200);
        let truncated = sanitize_title(&long, 10);
        assert_eq!(truncated.chars().count(), 10);
        assert!(truncated.ends_with('…'));
    }

    #[test]
    fn test_shorten_model_name() {
        assert_eq!(
            shorten_model_name("anthropic/claude-3-7-sonnet-20250219"),
            "claude-3.7-sonnet"
        );
        assert_eq!(
            shorten_model_name("claude-3-5-sonnet-20241022"),
            "claude-3.5-sonnet"
        );
        assert_eq!(
            shorten_model_name("openai/gpt-4o-2024-08-06"),
            "gpt-4o"
        );
        assert_eq!(
            shorten_model_name("deepseek/deepseek-reasoner"),
            "deepseek-r1"
        );
        assert_eq!(
            shorten_model_name("deepseek/deepseek-chat"),
            "deepseek-v3"
        );
        assert_eq!(
            shorten_model_name("meta-llama/llama-3.3-70b-instruct"),
            "llama-3.3-70b"
        );
    }

    #[test]
    fn test_terminal_title_default_format() {
        let title = TerminalTitle::new()
            .with_session("Refactor Parser")
            .with_model("claude-3-7-sonnet-20250219");

        assert_eq!(title.format(), "fusion: Refactor Parser (claude-3.7-sonnet)");

        // Session only
        let title_sess = TerminalTitle::new().with_session("My Session");
        assert_eq!(title_sess.format(), "fusion: My Session");

        // Model only
        let title_mod = TerminalTitle::new().with_model("gpt-4o");
        assert_eq!(title_mod.format(), "fusion (gpt-4o)");

        // Empty
        let title_empty = TerminalTitle::new();
        assert_eq!(title_empty.format(), "fusion");
    }

    #[test]
    fn test_terminal_title_styles() {
        let title = TerminalTitle::new()
            .with_session("Web App")
            .with_model("gpt-4o")
            .with_branch("feat/ui")
            .with_status("Thinking...");

        // Compact
        let compact = title.clone().with_style(TitleFormatStyle::Compact);
        assert_eq!(compact.format(), "Web App | gpt-4o");

        // Detailed
        let detailed = title.clone().with_style(TitleFormatStyle::Detailed);
        assert_eq!(
            detailed.format(),
            "fusion: Web App [feat/ui] (gpt-4o) - Thinking..."
        );

        // ModelFirst
        let model_first = title.clone().with_style(TitleFormatStyle::ModelFirst);
        assert_eq!(model_first.format(), "(gpt-4o) Web App - fusion");

        // StatusPrefix
        let status_prefix = title
            .clone()
            .with_status_icon("⚡")
            .with_style(TitleFormatStyle::StatusPrefix);
        assert_eq!(status_prefix.format(), "⚡ Thinking... | Web App (gpt-4o)");

        // Minimal
        let minimal = title.clone().with_style(TitleFormatStyle::Minimal);
        assert_eq!(minimal.format(), "Web App");

        // Custom Template
        let custom = title.clone().with_style(TitleFormatStyle::Custom(
            "[{branch}] {session} ({short_model}) -> {status}".to_string(),
        ));
        assert_eq!(custom.format(), "[feat/ui] Web App (gpt-4o) -> Thinking...");
    }

    #[test]
    fn test_render_osc_sequences() {
        let title = "fusion: My Session (gpt-4o)";

        // OSC 0 with BEL
        let osc0 = render_osc(title, OscType::Osc0, OscTerminator::Bel);
        assert_eq!(osc0, "\x1b]0;fusion: My Session (gpt-4o)\x07");

        // OSC 2 with BEL
        let osc2 = render_osc(title, OscType::Osc2, OscTerminator::Bel);
        assert_eq!(osc2, "\x1b]2;fusion: My Session (gpt-4o)\x07");

        // OSC 1 with BEL
        let osc1 = render_osc(title, OscType::Osc1, OscTerminator::Bel);
        assert_eq!(osc1, "\x1b]1;fusion: My Session (gpt-4o)\x07");

        // Both 0 and 2
        let both = render_osc(title, OscType::Both0And2, OscTerminator::Bel);
        assert_eq!(
            both,
            "\x1b]0;fusion: My Session (gpt-4o)\x07\x1b]2;fusion: My Session (gpt-4o)\x07"
        );
    }

    #[test]
    fn test_title_updater_caching_and_deduplication() {
        let mut updater = TitleUpdater::new(TitleConfig::default());
        let mut buffer = Vec::new();

        // 1. Initial write
        let emitted = updater
            .set_title_to("fusion: Session 1 (gpt-4o)", &mut buffer)
            .unwrap();
        assert!(emitted);
        assert_eq!(
            String::from_utf8(buffer.clone()).unwrap(),
            "\x1b]0;fusion: Session 1 (gpt-4o)\x07"
        );
        assert_eq!(updater.last_title(), Some("fusion: Session 1 (gpt-4o)"));

        // 2. Duplicate write (same title) -> should be suppressed
        buffer.clear();
        let emitted_dup = updater
            .set_title_to("fusion: Session 1 (gpt-4o)", &mut buffer)
            .unwrap();
        assert!(!emitted_dup);
        assert!(buffer.is_empty());

        // 3. New title -> should emit
        buffer.clear();
        let emitted_new = updater
            .set_title_to("fusion: Session 2 (claude-3.7-sonnet)", &mut buffer)
            .unwrap();
        assert!(emitted_new);
        assert_eq!(
            String::from_utf8(buffer.clone()).unwrap(),
            "\x1b]0;fusion: Session 2 (claude-3.7-sonnet)\x07"
        );

        // 4. Disabled updater -> suppresses everything
        updater.set_enabled(false);
        buffer.clear();
        let emitted_disabled = updater.set_title_to("fusion: Disabled", &mut buffer).unwrap();
        assert!(!emitted_disabled);
        assert!(buffer.is_empty());
    }

    #[test]
    fn test_title_from_session() {
        let mut session = Session::new("anthropic/claude-3-7-sonnet-20250219");
        session.set_title("Fix Database Deadlock");

        let title = TerminalTitle::from_session(&session);
        assert_eq!(
            title.format(),
            "fusion: Fix Database Deadlock (claude-3.7-sonnet)"
        );

        // With status
        let with_status = TerminalTitle::from_session_and_status(&session, "Compiling...");
        let detailed = with_status.with_style(TitleFormatStyle::Detailed);
        assert!(detailed.format().contains("Fix Database Deadlock"));
        assert!(detailed.format().contains("claude-3.7-sonnet"));
        assert!(detailed.format().contains("Compiling..."));
    }

    #[test]
    fn test_title_guard_resets_on_drop() {
        let mut config = TitleConfig::default();
        config.default_title = Some("Default Terminal".to_string());
        let mut updater = TitleUpdater::new(config);

        let mut buffer = Vec::new();
        updater.set_title_to("Temporary Task", &mut buffer).unwrap();

        // Scope with guard
        {
            let _guard = updater.guard();
        }

        // After guard drop, reset() has been called on updater
        assert_eq!(updater.last_title(), Some("Default Terminal"));
    }

    #[test]
    fn test_config_from_env() {
        let config = TitleConfig::from_env();
        assert_eq!(config.app_name, "fusion");
    }

    #[test]
    fn test_tokens_and_cost_formatting() {
        let title = TerminalTitle::new()
            .with_session("Test")
            .with_tokens(14500)
            .with_cost(0.042)
            .with_style(TitleFormatStyle::Custom(
                "{session} | {tokens} | {cost}".to_string(),
            ));

        assert_eq!(title.format(), "Test | 14.5k tok | $0.04");
    }
}
