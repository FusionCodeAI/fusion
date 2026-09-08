//! Cross-platform desktop and terminal notifications for Fusion.
//!
//! Provides non-blocking notifications on task completion, subagent execution events, turn completion,
//! and notable system occurrences.
//! Supports native desktop notifications across operating systems:
//! - **macOS**: Native notifications via `osascript` (AppleScript).
//! - **Linux / BSD**: Desktop notifications via `notify-send` (or `kdialog` fallback).
//! - **Windows**: Windows 10/11 Toast notifications and balloon tooltips via `powershell` (`ToastNotificationManager`).
//! - **Android / Termux**: Native Android notifications via `termux-notification`.
//! - **Terminal Emulators**: Inline ANSI/OSC notifications via OSC 777, OSC 9, and OSC 99 escape sequences.
//!
//! Notifications are dispatched asynchronously in background threads by default to ensure
//! zero latency impact on interactive REPL and agent execution pipelines.

use serde::{Deserialize, Serialize};
use std::fmt;
use std::io::{stderr, stdout, IsTerminal, Write};
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// Default application name shown in notification headers.
pub const DEFAULT_APP_NAME: &str = "Fusion";

/// Default notification expiration / display timeout in milliseconds.
pub const DEFAULT_TIMEOUT_MS: u32 = 5000;

/// Standard ASCII bell control character (`BEL`, `\x07`).
pub const TERMINAL_BELL: &str = "\x07";

// ---------------------------------------------------------------------------
// Notification Priority, Urgency & Triggers
// ---------------------------------------------------------------------------

/// Explicit event triggers for notifications matching OMP terminal notification triggers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NotificationTrigger {
    /// Agent finished turn normally.
    Completion,
    /// Agent stopped with an unrecoverable error.
    Error,
    /// Agent waiting for user input / decision.
    Ask,
}

impl NotificationTrigger {
    /// Returns the string identifier for this trigger (e.g. "completion", "error", "ask").
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Completion => "completion",
            Self::Error => "error",
            Self::Ask => "ask",
        }
    }
}

impl fmt::Display for NotificationTrigger {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

/// Priority level for notifications (Info, Success, Warning, Error).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum NotificationPriority {
    /// Informational notification (default / low urgency).
    #[default]
    Info,
    /// Successful operation, turn or subagent completion.
    Success,
    /// Warning or non-fatal anomaly.
    Warning,
    /// Error, failure, or actionable issue requiring immediate attention.
    Error,
}

impl NotificationPriority {
    /// Returns the standard lowercase string representation.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Info => "info",
            Self::Success => "success",
            Self::Warning => "warning",
            Self::Error => "error",
        }
    }

    /// Converts this priority level to the underlying `NotificationUrgency`.
    pub fn to_urgency(&self) -> NotificationUrgency {
        match self {
            Self::Info => NotificationUrgency::Low,
            Self::Success => NotificationUrgency::Normal,
            Self::Warning => NotificationUrgency::Normal,
            Self::Error => NotificationUrgency::Critical,
        }
    }

    /// Returns the default system icon name for this priority.
    pub fn default_icon(&self) -> &'static str {
        match self {
            Self::Info => "dialog-information",
            Self::Success => "dialog-ok",
            Self::Warning => "dialog-warning",
            Self::Error => "dialog-error",
        }
    }
}

impl From<NotificationPriority> for NotificationUrgency {
    fn from(prio: NotificationPriority) -> Self {
        prio.to_urgency()
    }
}

impl From<NotificationUrgency> for NotificationPriority {
    fn from(urgency: NotificationUrgency) -> Self {
        match urgency {
            NotificationUrgency::Low => NotificationPriority::Info,
            NotificationUrgency::Normal => NotificationPriority::Success,
            NotificationUrgency::Critical => NotificationPriority::Error,
        }
    }
}

impl fmt::Display for NotificationPriority {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

/// Urgency / priority level for notifications matching standard desktop specs (FreeDesktop).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum NotificationUrgency {
    /// Low priority / background info.
    Low,
    /// Standard priority (default for normal task/turn completions).
    #[default]
    Normal,
    /// Critical priority (errors, required human intervention).
    Critical,
}

impl NotificationUrgency {
    /// Returns the standard lowercase string representation.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Low => "low",
            Self::Normal => "normal",
            Self::Critical => "critical",
        }
    }

    /// Returns the urgency string for Linux `notify-send -u <urgency>`.
    pub fn notify_send_urgency(&self) -> &'static str {
        match self {
            Self::Low => "low",
            Self::Normal => "normal",
            Self::Critical => "critical",
        }
    }

    /// Returns the priority string for Android `termux-notification --priority <prio>`.
    pub fn termux_priority(&self) -> &'static str {
        match self {
            Self::Low => "low",
            Self::Normal => "default",
            Self::Critical => "high",
        }
    }
}

impl fmt::Display for NotificationUrgency {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

// ---------------------------------------------------------------------------
// Notification Backend
// ---------------------------------------------------------------------------

/// Available notification delivery backends.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum NotificationBackend {
    /// Automatically detect the best available backend based on OS and environment.
    #[default]
    Auto,
    /// macOS AppleScript notification (`osascript`).
    MacOS,
    /// Linux/Unix desktop notification (`notify-send` / `kdialog`).
    Linux,
    /// Windows PowerShell Toast / Balloon notification (`powershell.exe`).
    Windows,
    /// Android / Termux native notification (`termux-notification`).
    Termux,
    /// Terminal ANSI/OSC escape sequence notification (OSC 777 / OSC 9 / OSC 99).
    TerminalOsc,
    /// Custom command template (e.g. `"my-notifier {title} {body}"`).
    Custom(String),
    /// Notifications disabled / no-op.
    Disabled,
}

impl NotificationBackend {
    /// Detects the most appropriate notification backend for the current platform and environment.
    pub fn detect() -> Self {
        if is_termux() {
            return Self::Termux;
        }

        #[cfg(target_os = "macos")]
        {
            return Self::TerminalOsc;
        }

        #[cfg(target_os = "windows")]
        {
            return Self::Windows;
        }

        #[cfg(any(
            target_os = "linux",
            target_os = "freebsd",
            target_os = "openbsd",
            target_os = "netbsd"
        ))]
        {
            if is_executable_in_path("notify-send") || is_executable_in_path("kdialog") {
                return Self::Linux;
            }
            // Check for WSL with Windows PowerShell or wsl-notify-send available
            if is_wsl() {
                if is_executable_in_path("wsl-notify-send.exe")
                    || is_executable_in_path("powershell.exe")
                {
                    return Self::Windows;
                }
            }
            return Self::TerminalOsc;
        }

        #[cfg(target_os = "android")]
        {
            return Self::Termux;
        }

        #[allow(unreachable_code)]
        Self::TerminalOsc
    }

    /// Human-readable backend name.
    pub fn name(&self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::MacOS => "macos (osascript)",
            Self::Linux => "linux (notify-send)",
            Self::Windows => "windows (powershell)",
            Self::Termux => "termux (termux-notification)",
            Self::TerminalOsc => "terminal (osc)",
            Self::Custom(_) => "custom",
            Self::Disabled => "disabled",
        }
    }

    /// Returns whether this backend is available on the current system.
    pub fn is_available(&self) -> bool {
        match self {
            Self::Auto => true,
            Self::MacOS => cfg!(target_os = "macos") || is_executable_in_path("osascript"),
            Self::Linux => is_executable_in_path("notify-send") || is_executable_in_path("kdialog"),
            Self::Windows => {
                cfg!(target_os = "windows")
                    || is_executable_in_path("powershell.exe")
                    || is_executable_in_path("powershell")
            }
            Self::Termux => is_termux() || is_executable_in_path("termux-notification"),
            Self::TerminalOsc => true,
            Self::Custom(cmd) => {
                let bin = cmd.split_whitespace().next().unwrap_or("");
                !bin.is_empty() && is_executable_in_path(bin)
            }
            Self::Disabled => false,
        }
    }
}

impl fmt::Display for NotificationBackend {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.name())
    }
}

// ---------------------------------------------------------------------------
// Terminal OSC Protocol
// ---------------------------------------------------------------------------

/// Terminal notification escape sequence protocol variants.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum TerminalOscProtocol {
    /// OSC 777 (`\x1b]777;notify;{title};{body}\x07`) - supported by Kitty, Foot, WezTerm, Ghostty.
    Osc777,
    /// OSC 9 (`\x1b]9;{title}: {body}\x07`) - supported by iTerm2, ConEmu, Windows Terminal.
    Osc9,
    /// OSC 99 (`\x1b]99;i=1:d=0;{title}\x1b\\...`) - Kitty desktop notification protocol.
    Osc99,
    /// Terminal Bell (`\x07`) - minimalist audible or visual bell for multiplexers and VTE terminals.
    Bell,
    /// Emits combined multi-protocol escape sequences for maximum terminal emulator compatibility.
    #[default]
    All,
}

/// Auto-detect the best terminal notification escape protocol for the current environment.
pub fn detect_terminal_osc_protocol() -> TerminalOscProtocol {
    let term_prog = std::env::var("TERM_PROGRAM").unwrap_or_default();
    let term = std::env::var("TERM").unwrap_or_default();

    if term_prog.contains("Warp") || term_prog.contains("iTerm") || term_prog.contains("WezTerm") {
        TerminalOscProtocol::Osc9
    } else if term.contains("kitty") || std::env::var("KITTY_PID").is_ok() {
        TerminalOscProtocol::Osc99
    } else if term_prog.contains("ghostty") || term.contains("ghostty") || term.contains("foot") {
        TerminalOscProtocol::Osc777
    } else {
        TerminalOscProtocol::Osc9
    }
}

// ---------------------------------------------------------------------------
// Configuration
// ---------------------------------------------------------------------------

/// Configuration settings for desktop and terminal notifications.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NotificationConfig {
    /// Master toggle for all notifications.
    #[serde(default = "default_true")]
    pub enabled: bool,

    /// Whether desktop OS notifications are enabled.
    #[serde(default = "default_true")]
    pub desktop_enabled: bool,

    /// Whether terminal OSC notifications are enabled.
    #[serde(default = "default_true")]
    pub terminal_enabled: bool,

    /// Whether sound / audio cue accompanies the notification.
    #[serde(default = "default_false")]
    pub sound: bool,

    /// Minimum task/turn duration in seconds required to trigger a notification.
    /// If `Some(secs)`, tasks finishing faster than `secs` are silenced.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min_duration_secs: Option<f64>,

    /// Default notification urgency.
    #[serde(default)]
    pub urgency: NotificationUrgency,

    /// Application name shown in notification headers.
    #[serde(default = "default_app_name")]
    pub app_name: String,

    /// Optional default icon path or named system icon.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,

    /// Notification expiration timeout in milliseconds.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout_ms: Option<u32>,

    /// Explicitly selected notification backend (None for auto-detection).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub backend: Option<NotificationBackend>,

    /// If true, terminal escape sequences are only emitted when stderr is a TTY.
    #[serde(default = "default_true")]
    pub tty_only: bool,
}

fn default_true() -> bool {
    true
}

fn default_false() -> bool {
    false
}

fn default_app_name() -> String {
    DEFAULT_APP_NAME.to_string()
}

impl Default for NotificationConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            desktop_enabled: true,
            terminal_enabled: true,
            sound: false,
            min_duration_secs: None,
            urgency: NotificationUrgency::Normal,
            app_name: DEFAULT_APP_NAME.to_string(),
            icon: None,
            timeout_ms: Some(DEFAULT_TIMEOUT_MS),
            backend: None,
            tty_only: true,
        }
    }
}

impl NotificationConfig {
    /// Creates a new `NotificationConfig` with the master enabled flag.
    pub fn new(enabled: bool) -> Self {
        Self {
            enabled,
            ..Default::default()
        }
    }

    /// Loads notification configuration from environment variables with sensible fallbacks.
    ///
    /// Recognized variables:
    /// - `FUSION_NOTIFY` / `FUSION_NOTIFICATIONS`: "1"/"true" or "0"/"false"
    /// - `NO_NOTIFY` / `FUSION_NO_NOTIFY`: if set and not "0", disables notifications
    /// - `FUSION_DESKTOP_NOTIFY`: enable/disable OS desktop notifications
    /// - `FUSION_TERMINAL_NOTIFY`: enable/disable terminal OSC notifications
    /// - `FUSION_NOTIFY_SOUND`: enable/disable notification sound
    /// - `FUSION_NOTIFY_MIN_DURATION`: minimum duration float in seconds (e.g. "5.0")
    /// - `FUSION_NOTIFY_BACKEND`: "macos", "linux", "windows", "termux", "osc", "disabled"
    pub fn from_env() -> Self {
        let mut cfg = Self::default();

        if let Some(val) =
            first_non_empty_env(&["FUSION_NOTIFY", "FUSION_NOTIFICATIONS", "NOTIFY_ENABLED"])
        {
            let lower = val.to_lowercase();
            if lower == "0" || lower == "false" || lower == "no" || lower == "off" {
                cfg.enabled = false;
            } else if lower == "1" || lower == "true" || lower == "yes" || lower == "on" {
                cfg.enabled = true;
            }
        }

        if let Some(val) = first_non_empty_env(&["NO_NOTIFY", "FUSION_NO_NOTIFY"]) {
            let lower = val.to_lowercase();
            if lower != "0" && lower != "false" {
                cfg.enabled = false;
            }
        }

        if let Some(val) = first_non_empty_env(&["FUSION_DESKTOP_NOTIFY", "DESKTOP_NOTIFY"]) {
            let lower = val.to_lowercase();
            if lower == "0" || lower == "false" || lower == "no" || lower == "off" {
                cfg.desktop_enabled = false;
            } else if lower == "1" || lower == "true" || lower == "yes" || lower == "on" {
                cfg.desktop_enabled = true;
            }
        }

        if let Some(val) = first_non_empty_env(&["FUSION_TERMINAL_NOTIFY", "TERMINAL_NOTIFY"]) {
            let lower = val.to_lowercase();
            if lower == "0" || lower == "false" || lower == "no" || lower == "off" {
                cfg.terminal_enabled = false;
            } else if lower == "1" || lower == "true" || lower == "yes" || lower == "on" {
                cfg.terminal_enabled = true;
            }
        }

        if let Some(val) = first_non_empty_env(&["FUSION_NOTIFY_SOUND", "NOTIFY_SOUND"]) {
            let lower = val.to_lowercase();
            if lower == "1" || lower == "true" || lower == "yes" || lower == "on" {
                cfg.sound = true;
            } else if lower == "0" || lower == "false" || lower == "no" || lower == "off" {
                cfg.sound = false;
            }
        }

        if let Some(val) =
            first_non_empty_env(&["FUSION_NOTIFY_MIN_DURATION", "NOTIFY_MIN_DURATION"])
        {
            if let Ok(secs) = val.parse::<f64>() {
                if secs >= 0.0 {
                    cfg.min_duration_secs = Some(secs);
                }
            }
        }

        if let Some(val) = first_non_empty_env(&["FUSION_NOTIFY_BACKEND", "NOTIFY_BACKEND"]) {
            match val.to_lowercase().as_str() {
                "macos" | "apple" | "osascript" => cfg.backend = Some(NotificationBackend::MacOS),
                "linux" | "notify-send" | "kdialog" => {
                    cfg.backend = Some(NotificationBackend::Linux)
                }
                "windows" | "powershell" => cfg.backend = Some(NotificationBackend::Windows),
                "termux" | "android" => cfg.backend = Some(NotificationBackend::Termux),
                "osc" | "terminal" => cfg.backend = Some(NotificationBackend::TerminalOsc),
                "disabled" | "none" | "off" => cfg.backend = Some(NotificationBackend::Disabled),
                "auto" => cfg.backend = Some(NotificationBackend::Auto),
                custom => {
                    if !custom.is_empty() {
                        cfg.backend = Some(NotificationBackend::Custom(custom.to_string()));
                    }
                }
            }
        }

        cfg
    }

    /// Sets the minimum task duration required before a notification will be shown.
    pub fn with_min_duration(mut self, secs: f64) -> Self {
        self.min_duration_secs = Some(secs);
        self
    }

    /// Sets whether sound is enabled.
    pub fn with_sound(mut self, sound: bool) -> Self {
        self.sound = sound;
        self
    }

    /// Sets whether desktop OS notifications are enabled.
    pub fn with_desktop(mut self, desktop: bool) -> Self {
        self.desktop_enabled = desktop;
        self
    }

    /// Sets whether terminal OSC notifications are enabled.
    pub fn with_terminal(mut self, terminal: bool) -> Self {
        self.terminal_enabled = terminal;
        self
    }

    /// Sets an explicit notification backend.
    pub fn with_backend(mut self, backend: NotificationBackend) -> Self {
        self.backend = Some(backend);
        self
    }

    /// Sets the application name.
    pub fn with_app_name(mut self, name: impl Into<String>) -> Self {
        self.app_name = name.into();
        self
    }

    /// Sets default icon.
    pub fn with_icon(mut self, icon: impl Into<String>) -> Self {
        self.icon = Some(icon.into());
        self
    }

    /// Checks if a given elapsed duration satisfies the minimum threshold configured.
    pub fn is_duration_met(&self, duration_secs: Option<f64>) -> bool {
        match (self.min_duration_secs, duration_secs) {
            (Some(threshold), Some(actual)) => actual >= threshold,
            (Some(threshold), None) => threshold <= 0.0,
            (None, _) => true,
        }
    }

    /// Enables all notifications globally.
    pub fn enable(&mut self) {
        self.enabled = true;
    }

    /// Disables all notifications globally.
    pub fn disable(&mut self) {
        self.enabled = false;
    }

    /// Toggles the master enabled state and returns the new value.
    pub fn toggle(&mut self) -> bool {
        self.enabled = !self.enabled;
        self.enabled
    }
}

// ---------------------------------------------------------------------------
// Notification Struct
// ---------------------------------------------------------------------------

/// Represents a discrete notification payload to be delivered across desktop and terminal channels.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Notification {
    /// Notification title / summary header.
    pub title: String,
    /// Main notification body text.
    pub body: String,
    /// Optional subtitle (macOS / Termux support).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subtitle: Option<String>,
    /// Whether to play a sound cue with this notification.
    #[serde(default)]
    pub sound: bool,
    /// Notification urgency / priority level.
    #[serde(default)]
    pub urgency: NotificationUrgency,
    /// Optional structured priority level (Info, Success, Warning, Error).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub priority: Option<NotificationPriority>,
    /// Explicit event trigger (completion, error, ask).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub trigger: Option<NotificationTrigger>,
    /// Originating application name.
    #[serde(default = "default_app_name")]
    pub app_name: String,
    /// Optional icon path or theme icon name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,
    /// Expiration timeout in milliseconds.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout_ms: Option<u32>,
    /// Optional category identifier for deduplication or routing.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub category: Option<String>,
    /// Optional elapsed duration in seconds for task completion context.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duration_secs: Option<f64>,
}

impl Notification {
    /// Creates a new notification with the given title and body.
    pub fn new(title: impl Into<String>, body: impl Into<String>) -> Self {
        Self {
            title: title.into(),
            body: body.into(),
            subtitle: None,
            sound: false,
            urgency: NotificationUrgency::Normal,
            priority: None,
            trigger: None,
            app_name: DEFAULT_APP_NAME.to_string(),
            icon: None,
            timeout_ms: Some(DEFAULT_TIMEOUT_MS),
            category: None,
            duration_secs: None,
        }
    }

    /// Creates a new notification with an explicit `NotificationPriority`.
    pub fn with_priority(
        title: impl Into<String>,
        body: impl Into<String>,
        priority: NotificationPriority,
    ) -> Self {
        let notif = Self::new(title, body).priority(priority);
        let icon = priority.default_icon();
        notif.icon(icon)
    }

    /// Convenience constructor for informational notifications (Priority::Info).
    pub fn info(title: impl Into<String>, info_message: impl Into<String>) -> Self {
        Self::with_priority(title, info_message, NotificationPriority::Info)
            .subtitle("Info")
            .category("info")
    }

    /// Convenience constructor for success notifications (Priority::Success).
    pub fn success(title: impl Into<String>, success_message: impl Into<String>) -> Self {
        Self::with_priority(title, success_message, NotificationPriority::Success)
            .subtitle("Success")
            .category("success")
    }

    /// Convenience constructor for warning notifications (Priority::Warning).
    pub fn warning(title: impl Into<String>, warning_message: impl Into<String>) -> Self {
        Self::with_priority(title, warning_message, NotificationPriority::Warning)
            .subtitle("Warning")
            .category("warning")
    }

    /// Convenience constructor for error / failure notifications (Priority::Error).
    pub fn error(title: impl Into<String>, error_message: impl Into<String>) -> Self {
        Self::with_priority(title, error_message, NotificationPriority::Error)
            .subtitle("Error")
            .sound(true)
            .category("error")
            .trigger(NotificationTrigger::Error)
    }

    /// Convenience constructor for completion notifications (agent finished turn normally).
    pub fn completion(title: impl Into<String>, body: impl Into<String>) -> Self {
        Self::with_priority(title, body, NotificationPriority::Success)
            .subtitle("Completed")
            .category("completion")
            .trigger(NotificationTrigger::Completion)
    }

    /// Convenience constructor for ask notifications (agent waiting for user input / decision).
    pub fn ask(title: impl Into<String>, prompt: impl Into<String>) -> Self {
        Self::with_priority(title, prompt, NotificationPriority::Warning)
            .subtitle("Input Required")
            .sound(true)
            .category("ask")
            .trigger(NotificationTrigger::Ask)
    }
    /// Convenience constructor for general task completion notifications.
    pub fn task_complete(task_name: &str, duration_secs: Option<f64>) -> Self {
        let body = match duration_secs {
            Some(secs) => format!("Task completed in {}", format_duration_secs(secs)),
            None => "Task completed successfully".to_string(),
        };

        Self::new(task_name, body)
            .subtitle("Fusion Task")
            .priority(NotificationPriority::Success)
            .category("task_complete")
            .trigger(NotificationTrigger::Completion)
            .set_duration(duration_secs)
    }

    /// Convenience constructor for long-running task completion notifications.
    pub fn long_running_task_complete(task_name: &str, duration_secs: f64) -> Self {
        let body = format!(
            "Long-running task completed in {}",
            format_duration_secs(duration_secs)
        );

        Self::new(format!("Task Completed: {task_name}"), body)
            .subtitle("Long-running Task")
            .priority(NotificationPriority::Success)
            .category("long_task_complete")
            .trigger(NotificationTrigger::Completion)
            .duration_secs(duration_secs)
    }

    /// Convenience constructor for subagent execution completion notifications.
    pub fn subagent_complete(
        agent_name: &str,
        task_desc: &str,
        duration_secs: Option<f64>,
    ) -> Self {
        let body = match duration_secs {
            Some(secs) => format!(
                "Subagent '{agent_name}' finished task: {task_desc} ({})",
                format_duration_secs(secs)
            ),
            None => format!("Subagent '{agent_name}' finished task: {task_desc}"),
        };

        Self::new(format!("Subagent Complete: {agent_name}"), body)
            .subtitle(format!("Agent: {agent_name}"))
            .priority(NotificationPriority::Success)
            .category("subagent_complete")
            .icon("dialog-ok")
            .trigger(NotificationTrigger::Completion)
            .set_duration(duration_secs)
    }

    /// Convenience constructor for subagent execution failure notifications.
    pub fn subagent_failed(agent_name: &str, error_msg: &str, duration_secs: Option<f64>) -> Self {
        let body = match duration_secs {
            Some(secs) => format!(
                "Subagent '{agent_name}' failed after {}: {error_msg}",
                format_duration_secs(secs)
            ),
            None => format!("Subagent '{agent_name}' failed: {error_msg}"),
        };

        Self::new(format!("Subagent Failed: {agent_name}"), body)
            .subtitle(format!("Agent: {agent_name}"))
            .priority(NotificationPriority::Error)
            .sound(true)
            .category("subagent_failed")
            .icon("dialog-error")
            .trigger(NotificationTrigger::Error)
            .set_duration(duration_secs)
    }

    /// Convenience constructor for agent turn completion.
    pub fn turn_complete(task_name: &str, model: &str, duration_secs: Option<f64>) -> Self {
        let first_line = task_name.lines().next().unwrap_or("").trim();
        let title = if first_line.len() > 60 {
            format!("{}...", &first_line[..57])
        } else if !first_line.is_empty() {
            first_line.to_string()
        } else {
            "Turn Complete".to_string()
        };

        let body = match duration_secs {
            Some(secs) => format!(
                "Turn completed via {model} in {}",
                format_duration_secs(secs)
            ),
            None => format!("Turn completed via {model}"),
        };

        Self::new(title, body)
            .subtitle(format!("Model: {model}"))
            .priority(NotificationPriority::Success)
            .category("turn_complete")
            .trigger(NotificationTrigger::Completion)
            .set_duration(duration_secs)
    }

    // Builder methods

    /// Sets the notification priority level and adjusts urgency accordingly.
    pub fn priority(mut self, priority: NotificationPriority) -> Self {
        self.urgency = priority.to_urgency();
        self.priority = Some(priority);
        self
    }
    /// Sets the explicit notification event trigger.
    pub fn trigger(mut self, trigger: NotificationTrigger) -> Self {
        self.trigger = Some(trigger);
        self
    }

    /// Returns the explicit notification event trigger if set.
    pub fn get_trigger(&self) -> Option<NotificationTrigger> {
        self.trigger
    }

    /// Returns the active notification priority level.
    pub fn get_priority(&self) -> NotificationPriority {
        self.priority.unwrap_or_else(|| match self.urgency {
            NotificationUrgency::Low => NotificationPriority::Info,
            NotificationUrgency::Normal => NotificationPriority::Success,
            NotificationUrgency::Critical => NotificationPriority::Error,
        })
    }

    /// Sets the notification subtitle.
    pub fn subtitle(mut self, subtitle: impl Into<String>) -> Self {
        self.subtitle = Some(subtitle.into());
        self
    }

    /// Sets whether to play a sound cue.
    pub fn sound(mut self, sound: bool) -> Self {
        self.sound = sound;
        self
    }

    /// Sets the notification urgency.
    pub fn urgency(mut self, urgency: NotificationUrgency) -> Self {
        self.urgency = urgency;
        self
    }

    /// Sets the application name.
    pub fn app_name(mut self, name: impl Into<String>) -> Self {
        self.app_name = name.into();
        self
    }

    /// Sets the notification icon.
    pub fn icon(mut self, icon: impl Into<String>) -> Self {
        self.icon = Some(icon.into());
        self
    }

    /// Sets the notification timeout in milliseconds.
    pub fn timeout_ms(mut self, ms: u32) -> Self {
        self.timeout_ms = Some(ms);
        self
    }

    /// Sets the category identifier.
    pub fn category(mut self, category: impl Into<String>) -> Self {
        self.category = Some(category.into());
        self
    }

    /// Sets the duration in seconds.
    pub fn duration_secs(mut self, secs: f64) -> Self {
        self.duration_secs = Some(secs);
        self
    }

    fn set_duration(mut self, duration: Option<f64>) -> Self {
        self.duration_secs = duration;
        self
    }

    // -----------------------------------------------------------------------
    // Command and Escape Sequence Generation
    // -----------------------------------------------------------------------

    /// Renders the platform-specific command and argument list for a given backend.
    ///
    /// Returns `Some((executable, args))` or `None` if notifications are disabled/unsupported.
    pub fn build_command(&self, backend: NotificationBackend) -> Option<(String, Vec<String>)> {
        let active_backend = match backend {
            NotificationBackend::Auto => NotificationBackend::detect(),
            other => other,
        };

        match active_backend {
            NotificationBackend::MacOS => Some(self.render_macos_command()),
            NotificationBackend::Linux => Some(self.render_linux_command()),
            NotificationBackend::Windows => Some(self.render_windows_command()),
            NotificationBackend::Termux => Some(self.render_termux_command()),
            NotificationBackend::Custom(cmd) => Some(self.render_custom_command(&cmd)),
            NotificationBackend::TerminalOsc
            | NotificationBackend::Disabled
            | NotificationBackend::Auto => None,
        }
    }

    /// Renders macOS `osascript` command and arguments.
    /// Uses AppleScript `on run argv` handlers to safely pass strings without escaping injection risks.
    pub fn render_macos_command(&self) -> (String, Vec<String>) {
        let script = r#"on run argv
set notifTitle to item 1 of argv
set notifBody to item 2 of argv
set notifSub to item 3 of argv
set notifSound to item 4 of argv
if notifSub is not "" and notifSound is "true" then
    display notification notifBody with title notifTitle subtitle notifSub sound name "default"
else if notifSub is not "" then
    display notification notifBody with title notifTitle subtitle notifSub
else if notifSound is "true" then
    display notification notifBody with title notifTitle sound name "default"
else
    display notification notifBody with title notifTitle
end if
end run"#;

        let args = vec![
            "-e".to_string(),
            script.to_string(),
            self.title.clone(),
            self.body.clone(),
            self.subtitle.clone().unwrap_or_default(),
            if self.sound {
                "true".to_string()
            } else {
                "false".to_string()
            },
        ];

        ("osascript".to_string(), args)
    }

    /// Renders Linux `notify-send` command and arguments.
    pub fn render_linux_command(&self) -> (String, Vec<String>) {
        let mut args = vec![
            "-a".to_string(),
            self.app_name.clone(),
            "-u".to_string(),
            self.urgency.notify_send_urgency().to_string(),
        ];

        if let Some(icon) = &self.icon {
            args.push("-i".to_string());
            args.push(icon.clone());
        }

        if let Some(timeout) = self.timeout_ms {
            args.push("-t".to_string());
            args.push(timeout.to_string());
        }

        args.push(self.title.clone());

        let body_text = match &self.subtitle {
            Some(sub) if !sub.is_empty() => format!("{sub}\n{}", self.body),
            _ => self.body.clone(),
        };
        args.push(body_text);

        ("notify-send".to_string(), args)
    }

    /// Renders Linux `kdialog` command and arguments as a fallback when `notify-send` is absent.
    pub fn render_kdialog_command(&self) -> (String, Vec<String>) {
        let body_text = match &self.subtitle {
            Some(sub) if !sub.is_empty() => format!("{sub}\n{}", self.body),
            _ => self.body.clone(),
        };

        let timeout_secs = self.timeout_ms.unwrap_or(DEFAULT_TIMEOUT_MS) / 1000;
        let timeout_str = timeout_secs.max(1).to_string();

        let args = vec![
            "--title".to_string(),
            self.title.clone(),
            "--passivepopup".to_string(),
            body_text,
            timeout_str,
        ];

        ("kdialog".to_string(), args)
    }

    /// Renders Android / Termux `termux-notification` command and arguments.
    pub fn render_termux_command(&self) -> (String, Vec<String>) {
        let mut args = vec!["-t".to_string(), self.title.clone()];

        let content = match &self.subtitle {
            Some(sub) if !sub.is_empty() => format!("{sub}: {}", self.body),
            _ => self.body.clone(),
        };
        args.push("-c".to_string());
        args.push(content);

        let id = format!("fusion_{}", self.category.as_deref().unwrap_or("general"));
        args.push("--id".to_string());
        args.push(id);

        args.push("--priority".to_string());
        args.push(self.urgency.termux_priority().to_string());

        if self.sound {
            args.push("--sound".to_string());
        }

        if self.urgency == NotificationUrgency::Critical {
            args.push("--vibrate".to_string());
            args.push("100,100".to_string());
        }

        if let Some(icon) = &self.icon {
            args.push("--icon".to_string());
            args.push(icon.clone());
        }

        ("termux-notification".to_string(), args)
    }

    /// Renders Windows PowerShell script and arguments.
    /// Safely handles PowerShell single-quote string escaping and invokes `ToastNotificationManager`.
    pub fn render_windows_command(&self) -> (String, Vec<String>) {
        let safe_title = escape_powershell(&self.title);
        let safe_body = escape_powershell(&self.body);
        let safe_app_name = escape_powershell(&self.app_name);

        let ps_script = format!(
            "$title = '{safe_title}'; $body = '{safe_body}'; $app = '{safe_app_name}'; \
             try {{ \
                 [Windows.UI.Notifications.ToastNotificationManager, Windows.UI.Notifications, ContentType = WindowsRuntime] > $null; \
                 $template = [Windows.UI.Notifications.ToastNotificationManager]::GetTemplateContent([Windows.UI.Notifications.ToastTemplateType]::ToastText02); \
                 $textNodes = $template.GetElementsByTagName('text'); \
                 $textNodes.Item(0).AppendChild($template.CreateTextNode($title)) > $null; \
                 $textNodes.Item(1).AppendChild($template.CreateTextNode($body)) > $null; \
                 $toast = [Windows.UI.Notifications.ToastNotification]::new($template); \
                 [Windows.UI.Notifications.ToastNotificationManager]::CreateToastNotifier($app).Show($toast); \
             }} catch {{ \
                 Add-Type -AssemblyName System.Windows.Forms; \
                 $balloon = New-Object System.Windows.Forms.NotifyIcon; \
                 $balloon.Icon = [System.Drawing.SystemIcons]::Information; \
                 $balloon.Visible = $true; \
                 $balloon.ShowBalloonTip(4000, $title, $body, [System.Windows.Forms.ToolTipIcon]::Info); \
             }}"
        );

        let args = vec![
            "-NoProfile".to_string(),
            "-NonInteractive".to_string(),
            "-WindowStyle".to_string(),
            "Hidden".to_string(),
            "-Command".to_string(),
            ps_script,
        ];

        ("powershell".to_string(), args)
    }

    /// Renders a custom command template by substituting `{title}`, `{body}`, `{subtitle}`, `{urgency}`, `{priority}`, `{app}`.
    pub fn render_custom_command(&self, template: &str) -> (String, Vec<String>) {
        let sub = self.subtitle.as_deref().unwrap_or("");
        let rendered = template
            .replace("{title}", &self.title)
            .replace("{body}", &self.body)
            .replace("{subtitle}", sub)
            .replace("{urgency}", self.urgency.as_str())
            .replace("{priority}", self.get_priority().as_str())
            .replace("{app}", &self.app_name);

        let mut parts = rendered.split_whitespace();
        let bin = parts.next().unwrap_or("").to_string();
        let args = parts.map(|s| s.to_string()).collect();

        (bin, args)
    }

    /// Renders inline terminal notification escape sequences.
    ///
    /// Generates multi-protocol sequences (OSC 777, OSC 9, OSC 99) for broad terminal compatibility.
    pub fn render_terminal_osc(&self) -> String {
        self.render_terminal_osc_protocol(TerminalOscProtocol::All)
    }

    /// Auto-detects the active terminal's supported OSC protocol.
    pub fn detect_protocol() -> TerminalOscProtocol {
        detect_terminal_osc_protocol()
    }

    /// Renders rich Kitty OSC 99 desktop notification escape sequence with title and body parameters.
    pub fn render_osc99(&self) -> String {
        let id = self.category.as_deref().unwrap_or("1");

        let mut meta = vec![
            format!("i={id}"),
            format!("f={}", base64_encode(self.app_name.as_bytes())),
        ];

        meta.push("a=focus".to_string());

        let urgency_num = match self.urgency {
            NotificationUrgency::Low => "0",
            NotificationUrgency::Normal => "1",
            NotificationUrgency::Critical => "2",
        };
        meta.push(format!("u={urgency_num}"));

        if let Some(trigger) = self.trigger {
            meta.push(format!("t={}", base64_encode(trigger.as_str().as_bytes())));
        } else if let Some(cat) = &self.category {
            meta.push(format!("t={}", base64_encode(cat.as_bytes())));
        }

        if let Some(icon) = &self.icon {
            meta.push(format!("n={}", base64_encode(icon.as_bytes())));
        }

        if self.sound {
            meta.push(format!("s={}", base64_encode(b"default")));
        }

        if let Some(timeout) = self.timeout_ms {
            meta.push(format!("w={timeout}"));
        }

        if !self.body.is_empty() {
            // First chunk: title with d=0 (more chunks follow)
            let mut title_meta = meta;
            title_meta.push("d=0".to_string());
            let title_chunk = osc99_chunk(&title_meta, &self.title);

            // Second chunk: body with p=body
            let body_meta = vec![format!("i={id}"), "p=body".to_string()];
            let body_chunk = osc99_chunk(&body_meta, &self.body);

            format!("{title_chunk}{body_chunk}")
        } else {
            // Single chunk: title only
            osc99_chunk(&meta, &self.title)
        }
    }

    /// Renders inline terminal notification escape sequences for a specific protocol.
    pub fn render_terminal_osc_protocol(&self, protocol: TerminalOscProtocol) -> String {
        let clean_title = sanitize_terminal_text(&self.title);
        let clean_body = sanitize_terminal_text(&self.body);

        match protocol {
            TerminalOscProtocol::Osc777 => {
                format!("\x1b]777;notify;{clean_title};{clean_body}\x07")
            }
            TerminalOscProtocol::Osc9 => {
                format!("\x1b]9;{clean_title}: {clean_body}\x07")
            }
            TerminalOscProtocol::Osc99 => self.render_osc99(),
            TerminalOscProtocol::Bell => TERMINAL_BELL.to_string(),
            TerminalOscProtocol::All => {
                let proto = detect_terminal_osc_protocol();
                self.render_terminal_osc_protocol(proto)
            }
        }
    }

    /// Renders inline terminal notification escape sequences with multiplexer wrapping (tmux / Zellij).
    pub fn render_terminal_osc_with_multiplexer(&self) -> String {
        format_multiplexer_terminal_sequence(&self.render_terminal_osc())
    }

    /// Renders inline terminal notification escape sequences for a specific protocol with multiplexer wrapping.
    pub fn render_terminal_osc_protocol_with_multiplexer(
        &self,
        protocol: TerminalOscProtocol,
    ) -> String {
        format_multiplexer_terminal_sequence(&self.render_terminal_osc_protocol(protocol))
    }

    // -----------------------------------------------------------------------
    // Dispatch Methods
    // -----------------------------------------------------------------------

    /// Dispatches the notification non-blockingly in a background thread using environment configuration.
    pub fn send(&self) {
        let config = NotificationConfig::from_env();
        self.send_with_config(&config);
    }

    /// Dispatches the notification non-blockingly in a background thread using the provided configuration.
    pub fn send_with_config(&self, config: &NotificationConfig) {
        if !config.enabled {
            return;
        }

        if !config.is_duration_met(self.duration_secs) {
            return;
        }

        // Emit terminal OSC immediately to stdout for terminal emulators (Warp, iTerm, WezTerm)
        if config.terminal_enabled {
            let tty_ok = !config.tty_only || stdout().is_terminal() || stderr().is_terminal();
            if tty_ok {
                let mut out = stdout();
                let _ = self.send_terminal_osc(&mut out);
            }
        }

        let notification = self.clone();
        let cfg = config.clone();

        let _ = std::thread::Builder::new()
            .name("fusion-notify".to_string())
            .spawn(move || {
                if cfg.desktop_enabled {
                    let backend = cfg
                        .backend
                        .clone()
                        .unwrap_or_else(NotificationBackend::detect);
                    if backend != NotificationBackend::TerminalOsc
                        && backend != NotificationBackend::Disabled
                    {
                        let _ = notification.send_desktop(backend);
                    }
                }
                if cfg.sound || notification.sound {
                    let mut out = stdout();
                    let _ = out.write_all(TERMINAL_BELL.as_bytes());
                    let _ = out.flush();
                }
            });
    }

    /// Dispatches the notification synchronously and returns the delivery outcome.
    pub fn send_sync(&self, config: &NotificationConfig) -> NotificationOutcome {
        let mut outcome = NotificationOutcome::default();

        if !config.enabled {
            outcome.suppressed = true;
            return outcome;
        }

        if !config.is_duration_met(self.duration_secs) {
            outcome.suppressed = true;
            return outcome;
        }

        // 1. Terminal OSC notification
        if config.terminal_enabled {
            let tty_ok = !config.tty_only || stdout().is_terminal() || stderr().is_terminal();
            if tty_ok {
                let mut out = stdout();
                if self.send_terminal_osc(&mut out).is_ok() {
                    outcome.terminal_sent = true;
                }
            }
        }

        // 2. Desktop OS notification
        if config.desktop_enabled {
            let backend = config
                .backend
                .clone()
                .unwrap_or_else(NotificationBackend::detect);
            if backend != NotificationBackend::TerminalOsc
                && backend != NotificationBackend::Disabled
            {
                match self.send_desktop(backend) {
                    Ok(()) => {
                        outcome.desktop_sent = true;
                    }
                    Err(e) => {
                        outcome.error = Some(e.to_string());
                    }
                }
            }
        }

        // 3. Audio cue / terminal bell if requested
        if config.sound || self.sound {
            let mut err = stderr();
            let _ = err.write_all(TERMINAL_BELL.as_bytes());
            let _ = err.flush();
        }

        outcome
    }

    /// Sends terminal OSC escape sequences to the provided writer, applying tmux DCS passthrough or Zellij BEL wrapping if inside a multiplexer.
    pub fn send_terminal_osc<W: Write>(&self, writer: &mut W) -> std::io::Result<bool> {
        let protocol = detect_terminal_osc_protocol();
        let osc_seq = self.render_terminal_osc_protocol(protocol);
        let final_seq = format_multiplexer_terminal_sequence(&osc_seq);
        writer.write_all(final_seq.as_bytes())?;
        writer.flush()?;
        Ok(true)
    }

    /// Executes the desktop notification backend command with a bounded execution deadline.
    pub fn send_desktop(&self, backend: NotificationBackend) -> Result<(), NotificationError> {
        #[cfg(target_os = "macos")]
        {
            if let Some(tn_path) = find_terminal_notifier() {
                let mut args = vec![
                    "-title".to_string(),
                    self.title.clone(),
                    "-message".to_string(),
                    self.body.clone(),
                ];
                if let Some(sub) = &self.subtitle {
                    if !sub.is_empty() {
                        args.push("-subtitle".to_string());
                        args.push(sub.clone());
                    }
                }
                if self.sound {
                    args.push("-sound".to_string());
                    args.push("default".to_string());
                }
                if let Ok(()) = execute_process_with_timeout(&tn_path, &args, self.timeout_ms) {
                    return Ok(());
                }
            }
        }

        let cmd_spec = match self.build_command(backend) {
            Some(spec) => spec,
            None => return Err(NotificationError::Disabled),
        };

        let (bin, args) = cmd_spec;
        // Check if executable exists
        if !is_executable_in_path(&bin) {
            // Check for Linux kdialog fallback if notify-send is missing
            if bin == "notify-send" && is_executable_in_path("kdialog") {
                let (kdialog_bin, kdialog_args) = self.render_kdialog_command();
                return execute_process_with_timeout(&kdialog_bin, &kdialog_args, self.timeout_ms);
            }
            // Check for Termux fallback if termux-notification is missing
            if bin == "termux-notification" && is_executable_in_path("termux-toast") {
                let toast_args = vec!["-s".to_string(), format!("{}: {}", self.title, self.body)];
                return execute_process_with_timeout("termux-toast", &toast_args, self.timeout_ms);
            }
            return Err(NotificationError::BackendUnavailable(format!(
                "Executable '{bin}' not found in PATH"
            )));
        }

        execute_process_with_timeout(&bin, &args, self.timeout_ms)
    }
}

// ---------------------------------------------------------------------------
// Process Execution with Deadline Timeout
// ---------------------------------------------------------------------------

fn execute_process_with_timeout(
    bin: &str,
    args: &[String],
    timeout_ms: Option<u32>,
) -> Result<(), NotificationError> {
    let mut cmd = Command::new(bin);
    cmd.args(args);
    cmd.stdin(Stdio::null());
    cmd.stdout(Stdio::null());
    cmd.stderr(Stdio::null());

    let mut child = cmd
        .spawn()
        .map_err(|e| NotificationError::SpawnError(format!("Failed to spawn '{bin}': {e}")))?;

    let timeout = Duration::from_millis(timeout_ms.unwrap_or(DEFAULT_TIMEOUT_MS) as u64);
    let start = Instant::now();

    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                if status.success() {
                    return Ok(());
                } else {
                    return Err(NotificationError::CommandFailed(format!(
                        "Process '{bin}' exited with status {:?}",
                        status.code()
                    )));
                }
            }
            Ok(None) => {
                if start.elapsed() > timeout {
                    let _ = child.kill();
                    return Err(NotificationError::Timeout);
                }
                std::thread::sleep(Duration::from_millis(25));
            }
            Err(e) => {
                return Err(NotificationError::Io(e.to_string()));
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Notification Outcome & Errors
// ---------------------------------------------------------------------------

/// Outcome summary of a notification dispatch attempt.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct NotificationOutcome {
    /// True if an OS desktop notification was successfully sent.
    pub desktop_sent: bool,
    /// True if an inline terminal OSC sequence was successfully emitted.
    pub terminal_sent: bool,
    /// True if notification was suppressed due to disabled config or duration threshold.
    pub suppressed: bool,
    /// Error message if desktop notification execution failed.
    pub error: Option<String>,
}

impl NotificationOutcome {
    /// Returns true if at least one notification channel successfully delivered.
    pub fn is_delivered(&self) -> bool {
        self.desktop_sent || self.terminal_sent
    }
}

/// Errors that can occur during notification dispatch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NotificationError {
    /// Notification backend command exited with a non-zero status code.
    CommandFailed(String),
    /// The required notification executable is missing from system PATH.
    BackendUnavailable(String),
    /// Spawning the notification process failed.
    SpawnError(String),
    /// Process execution exceeded the allowed timeout deadline.
    Timeout,
    /// Underlying I/O error.
    Io(String),
    /// Notifications are explicitly disabled.
    Disabled,
    /// Notification was suppressed by configuration.
    Suppressed,
}

impl fmt::Display for NotificationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::CommandFailed(msg) => write!(f, "Notification command failed: {msg}"),
            Self::BackendUnavailable(msg) => write!(f, "Notification backend unavailable: {msg}"),
            Self::SpawnError(msg) => write!(f, "Failed to spawn notification: {msg}"),
            Self::Timeout => write!(f, "Notification process timed out"),
            Self::Io(msg) => write!(f, "Notification I/O error: {msg}"),
            Self::Disabled => write!(f, "Notifications are disabled"),
            Self::Suppressed => write!(f, "Notification suppressed"),
        }
    }
}

impl std::error::Error for NotificationError {}

// ---------------------------------------------------------------------------
// System Environment Detection Helpers
// ---------------------------------------------------------------------------

/// Checks whether Fusion is executing inside an Android Termux environment.
pub fn is_termux() -> bool {
    if std::env::var("TERMUX_VERSION").is_ok() {
        return true;
    }

    if let Ok(prefix) = std::env::var("PREFIX") {
        if prefix.contains("com.termux") {
            return true;
        }
    }

    if cfg!(target_os = "android") {
        return true;
    }

    false
}

/// Checks whether Fusion is executing inside Windows Subsystem for Linux (WSL).
pub fn is_wsl() -> bool {
    if std::env::var("WSL_DISTRO_NAME").is_ok() || std::env::var("WSL_INTEROP").is_ok() {
        return true;
    }

    if Path::new("/proc/sys/fs/binfmt_misc/WSLInterop").exists() {
        return true;
    }

    false
}

/// Checks whether the current process is running inside a tmux session.
/// Reads `TMUX` from the environment on each invocation.
pub fn is_inside_tmux() -> bool {
    match std::env::var("TMUX") {
        Ok(val) => !val.is_empty(),
        Err(_) => false,
    }
}

/// Checks whether the current process is running inside a Zellij session.
/// Reads `ZELLIJ` from the environment on each invocation.
pub fn is_inside_zellij() -> bool {
    match std::env::var("ZELLIJ") {
        Ok(val) => !val.is_empty(),
        Err(_) => false,
    }
}

/// Wraps a terminal escape sequence in tmux's DCS passthrough envelope with doubled ESCs.
///
/// tmux requires nested control sequences to be passed through DCS (`\x1bPtmux;\x1b<OSC>\x1b\\`),
/// with any inner `\x1b` (ESC) characters doubled (`\x1b\x1b`).
pub fn wrap_tmux_passthrough(payload: &str) -> String {
    format!("\x1bPtmux;{}\x1b\\", payload.replace('\x1b', "\x1b\x1b"))
}

/// Formats a terminal escape sequence for the active multiplexer environment.
///
/// - If inside tmux (`is_inside_tmux()`), wraps the sequence in DCS passthrough (`wrap_tmux_passthrough`)
///   followed by `\x07` (BEL) so notifications escape tmux and trigger `monitor-bell` / `monitor-activity`.
/// - If inside Zellij (`is_inside_zellij()`), appends `\x07` (BEL) after the OSC sequence.
/// - If the sequence is already a bare `BEL` (`\x07`), returns it unchanged.
pub fn format_multiplexer_terminal_sequence(payload: &str) -> String {
    if payload == TERMINAL_BELL {
        return payload.to_string();
    }
    if is_inside_tmux() {
        format!("{}\x07", wrap_tmux_passthrough(payload))
    } else if is_inside_zellij() {
        format!("{payload}\x07")
    } else {
        payload.to_string()
    }
}

/// Pure-Rust PATH lookup to check if an executable exists and is runnable.
pub fn is_executable_in_path(cmd: &str) -> bool {
    if let Ok(path_var) = std::env::var("PATH") {
        for dir in std::env::split_paths(&path_var) {
            let full_path = dir.join(cmd);
            if full_path.is_file() {
                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt;
                    if let Ok(meta) = full_path.metadata() {
                        if meta.permissions().mode() & 0o111 != 0 {
                            return true;
                        }
                    }
                }
                #[cfg(windows)]
                {
                    return true;
                }
                #[cfg(not(any(unix, windows)))]
                {
                    return true;
                }
            }

            #[cfg(windows)]
            {
                if !cmd.ends_with(".exe") && !cmd.ends_with(".cmd") && !cmd.ends_with(".bat") {
                    for ext in &[".exe", ".cmd", ".bat"] {
                        let with_ext = dir.join(format!("{cmd}{ext}"));
                        if with_ext.is_file() {
                            return true;
                        }
                    }
                }
            }
        }
    }
    false
}

/// Locates `terminal-notifier` executable on macOS if installed.
pub fn find_terminal_notifier() -> Option<String> {
    if is_executable_in_path("terminal-notifier") {
        return Some("terminal-notifier".to_string());
    }
    for candidate in &[
        "/opt/homebrew/bin/terminal-notifier",
        "/usr/local/bin/terminal-notifier",
    ] {
        if std::path::Path::new(candidate).exists() {
            return Some((*candidate).to_string());
        }
    }
    None
}

// ---------------------------------------------------------------------------
// String Escaping & Formatting Helpers
// ---------------------------------------------------------------------------

/// Escapes single quotes for PowerShell single-quoted string literals (`'` -> `''`).
pub fn escape_powershell(s: &str) -> String {
    s.replace('\'', "''").replace('\0', "")
}

/// Sanitizes text for terminal OSC escape sequences by stripping control characters.
pub fn sanitize_terminal_text(s: &str) -> String {
    s.chars()
        .filter(|c| !c.is_control() || *c == ' ' || *c == '\t')
        .collect()
}

/// Minimal RFC 4648 base64 encoder without extra external dependencies.
pub fn base64_encode(input: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity((input.len() + 2) / 3 * 4);
    for chunk in input.chunks(3) {
        let b0 = chunk[0];
        let b1 = chunk.get(1).copied().unwrap_or(0);
        let b2 = chunk.get(2).copied().unwrap_or(0);

        out.push(TABLE[(b0 >> 2) as usize] as char);
        out.push(TABLE[(((b0 & 0x03) << 4) | (b1 >> 4)) as usize] as char);
        if chunk.len() > 1 {
            out.push(TABLE[(((b1 & 0x0f) << 2) | (b2 >> 6)) as usize] as char);
        } else {
            out.push('=');
        }
        if chunk.len() > 2 {
            out.push(TABLE[(b2 & 0x3f) as usize] as char);
        } else {
            out.push('=');
        }
    }
    out
}

/// Checks if a string contains C0/C1 control characters that are unsafe inside an OSC payload.
pub fn is_osc99_unsafe(s: &str) -> bool {
    s.chars()
        .any(|c| (c as u32) <= 0x1f || ((c as u32) >= 0x7f && (c as u32) <= 0x9f))
}

/// Emits an OSC 99 chunk, using base64 encoding (`e=1`) if payload contains unsafe control characters.
pub fn osc99_chunk(meta: &[String], payload: &str) -> String {
    let mut chunk_meta = meta.to_vec();
    if is_osc99_unsafe(payload) {
        chunk_meta.push("e=1".to_string());
        format!(
            "\x1b]99;{};{}\x1b\\",
            chunk_meta.join(":"),
            base64_encode(payload.as_bytes())
        )
    } else {
        format!("\x1b]99;{};{}\x1b\\", chunk_meta.join(":"), payload)
    }
}

/// Formats duration in seconds into a clean human-readable representation.
pub fn format_duration_secs(secs: f64) -> String {
    if secs < 1.0 {
        format!("{:.0}ms", secs * 1000.0)
    } else if secs < 60.0 {
        format!("{:.1}s", secs)
    } else {
        let mins = (secs / 60.0).floor() as u64;
        let rem_secs = (secs % 60.0).round() as u64;
        format!("{mins}m {rem_secs:02}s")
    }
}

fn first_non_empty_env(keys: &[&str]) -> Option<String> {
    for &k in keys {
        if let Ok(val) = std::env::var(k) {
            let trimmed = val.trim();
            if !trimmed.is_empty() {
                return Some(trimmed.to_string());
            }
        }
    }
    None
}

// ---------------------------------------------------------------------------
// Top-Level Convenience Functions
// ---------------------------------------------------------------------------

/// Dispatches a non-blocking notification with the given title and body text.
pub fn notify(title: impl AsRef<str>, body: impl AsRef<str>) {
    Notification::new(title.as_ref(), body.as_ref()).send();
}

/// Dispatches a non-blocking informational notification.
pub fn notify_info(title: impl AsRef<str>, body: impl AsRef<str>) {
    Notification::info(title.as_ref(), body.as_ref()).send();
}

/// Dispatches a non-blocking success notification.
pub fn notify_success(title: impl AsRef<str>, body: impl AsRef<str>) {
    Notification::success(title.as_ref(), body.as_ref()).send();
}

/// Dispatches a non-blocking warning notification.
pub fn notify_warning(title: impl AsRef<str>, body: impl AsRef<str>) {
    Notification::warning(title.as_ref(), body.as_ref()).send();
}

/// Dispatches a non-blocking error notification.
pub fn notify_err(title: &str, error_message: &str) {
    Notification::error(title, error_message).send();
}

/// Dispatches a non-blocking task completion notification.
pub fn notify_task(task_name: &str, duration_secs: Option<f64>) {
    Notification::task_complete(task_name, duration_secs).send();
}

/// Dispatches a non-blocking notification when a long-running task completes.
pub fn notify_long_running_task(task_name: &str, duration_secs: f64) {
    Notification::long_running_task_complete(task_name, duration_secs).send();
}

/// Dispatches a non-blocking notification when a subagent finishes work successfully.
pub fn notify_subagent_complete(agent_name: &str, task_desc: &str, duration_secs: Option<f64>) {
    Notification::subagent_complete(agent_name, task_desc, duration_secs).send();
}

/// Dispatches a non-blocking notification when a subagent fails.
pub fn notify_subagent_failed(agent_name: &str, error_msg: &str, duration_secs: Option<f64>) {
    Notification::subagent_failed(agent_name, error_msg, duration_secs).send();
}

/// Convenience helper for task completion integrating application `Config`.
pub fn notify_task_complete(
    config: &crate::config::Config,
    task_name: &str,
    duration_secs: Option<f64>,
) -> bool {
    if !config.notify_enabled || !config.notify_on_completion {
        return false;
    }
    let notif_cfg = config.notification_config();
    Notification::task_complete(task_name, duration_secs).send_with_config(&notif_cfg);
    true
}

/// Convenience helper for subagent completion integrating application `Config`.
pub fn notify_subagent_complete_with_config(
    config: &crate::config::Config,
    agent_name: &str,
    task_desc: &str,
    duration_secs: Option<f64>,
) -> bool {
    if !config.notify_enabled || !config.notify_on_completion {
        return false;
    }
    let notif_cfg = config.notification_config();
    Notification::subagent_complete(agent_name, task_desc, duration_secs)
        .send_with_config(&notif_cfg);
    true
}

/// Convenience helper for subagent failure integrating application `Config`.
pub fn notify_subagent_failed_with_config(
    config: &crate::config::Config,
    agent_name: &str,
    error_msg: &str,
    duration_secs: Option<f64>,
) -> bool {
    if !config.notify_enabled || !config.notify_on_error {
        return false;
    }
    let notif_cfg = config.notification_config();
    Notification::subagent_failed(agent_name, error_msg, duration_secs)
        .send_with_config(&notif_cfg);
    true
}

/// Convenience helper for turn completion integrating application `Config`.
pub fn notify_turn_complete(
    config: &crate::config::Config,
    task_name: &str,
    model: &str,
    duration_secs: Option<f64>,
) -> bool {
    if !config.notify_enabled || !config.notify_on_completion {
        return false;
    }
    let notif_cfg = config.notification_config();
    Notification::turn_complete(task_name, model, duration_secs).send_with_config(&notif_cfg);
    true
}

/// Convenience helper for error notifications integrating application `Config`.
pub fn notify_error(config: &crate::config::Config, title: &str, error_message: &str) -> bool {
    if !config.notify_enabled || !config.notify_on_error {
        return false;
    }
    let notif_cfg = config.notification_config();
    Notification::error(title, error_message).send_with_config(&notif_cfg);
    true
}

/// Dispatches a non-blocking completion notification (agent finished turn normally).
pub fn notify_completion(title: impl AsRef<str>, body: impl AsRef<str>) {
    Notification::completion(title.as_ref(), body.as_ref()).send();
}

/// Dispatches a non-blocking ask notification (agent waiting for user input / decision).
pub fn notify_ask(title: impl AsRef<str>, prompt: impl AsRef<str>) {
    Notification::ask(title.as_ref(), prompt.as_ref()).send();
}

/// Convenience helper for completion notifications integrating application `Config`.
pub fn notify_completion_with_config(
    config: &crate::config::Config,
    title: &str,
    body: &str,
) -> bool {
    if !config.notify_enabled || !config.notify_on_completion {
        return false;
    }
    let notif_cfg = config.notification_config();
    Notification::completion(title, body).send_with_config(&notif_cfg);
    true
}

/// Convenience helper for ask notifications integrating application `Config`.
pub fn notify_ask_with_config(config: &crate::config::Config, title: &str, prompt: &str) -> bool {
    if !config.notify_enabled {
        return false;
    }
    let notif_cfg = config.notification_config();
    Notification::ask(title, prompt).send_with_config(&notif_cfg);
    true
}

/// Formats a rich Kitty OSC 99 desktop notification with title and body parameters.
pub fn format_osc99_notification(title: &str, body: &str) -> String {
    Notification::new(title, body).render_osc99()
}

/// Formats a rich Kitty OSC 99 desktop notification with title, body, and explicit trigger.
pub fn format_osc99_with_trigger(title: &str, body: &str, trigger: NotificationTrigger) -> String {
    Notification::new(title, body)
        .trigger(trigger)
        .render_osc99()
}

/// Emits an inline terminal OSC notification directly to standard error.
pub fn emit_terminal_notification(title: &str, body: &str) -> bool {
    let notif = Notification::new(title, body);
    let mut err = stderr();
    notif.send_terminal_osc(&mut err).is_ok()
}

/// Emits an inline terminal OSC notification to an arbitrary writer.
pub fn emit_terminal_osc_to<W: Write>(
    title: &str,
    body: &str,
    writer: &mut W,
) -> std::io::Result<()> {
    let notif = Notification::new(title, body);
    notif.send_terminal_osc(writer)?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Unit Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_notification_urgency() {
        assert_eq!(NotificationUrgency::Low.as_str(), "low");
        assert_eq!(NotificationUrgency::Normal.as_str(), "normal");
        assert_eq!(NotificationUrgency::Critical.as_str(), "critical");

        assert_eq!(NotificationUrgency::Low.notify_send_urgency(), "low");
        assert_eq!(NotificationUrgency::Normal.notify_send_urgency(), "normal");
        assert_eq!(
            NotificationUrgency::Critical.notify_send_urgency(),
            "critical"
        );

        assert_eq!(NotificationUrgency::Low.termux_priority(), "low");
        assert_eq!(NotificationUrgency::Normal.termux_priority(), "default");
        assert_eq!(NotificationUrgency::Critical.termux_priority(), "high");
    }

    #[test]
    fn test_notification_priority_and_conversions() {
        assert_eq!(NotificationPriority::Info.as_str(), "info");
        assert_eq!(NotificationPriority::Success.as_str(), "success");
        assert_eq!(NotificationPriority::Warning.as_str(), "warning");
        assert_eq!(NotificationPriority::Error.as_str(), "error");

        assert_eq!(
            NotificationPriority::Info.to_urgency(),
            NotificationUrgency::Low
        );
        assert_eq!(
            NotificationPriority::Success.to_urgency(),
            NotificationUrgency::Normal
        );
        assert_eq!(
            NotificationPriority::Warning.to_urgency(),
            NotificationUrgency::Normal
        );
        assert_eq!(
            NotificationPriority::Error.to_urgency(),
            NotificationUrgency::Critical
        );

        assert_eq!(
            NotificationUrgency::from(NotificationPriority::Info),
            NotificationUrgency::Low
        );
        assert_eq!(
            NotificationUrgency::from(NotificationPriority::Success),
            NotificationUrgency::Normal
        );
        assert_eq!(
            NotificationUrgency::from(NotificationPriority::Warning),
            NotificationUrgency::Normal
        );
        assert_eq!(
            NotificationUrgency::from(NotificationPriority::Error),
            NotificationUrgency::Critical
        );

        assert_eq!(
            NotificationPriority::from(NotificationUrgency::Low),
            NotificationPriority::Info
        );
        assert_eq!(
            NotificationPriority::from(NotificationUrgency::Normal),
            NotificationPriority::Success
        );
        assert_eq!(
            NotificationPriority::from(NotificationUrgency::Critical),
            NotificationPriority::Error
        );
    }

    #[test]
    fn test_notification_builders() {
        let notif = Notification::new("My Title", "My Body")
            .subtitle("Sub")
            .sound(true)
            .priority(NotificationPriority::Error)
            .app_name("CustomFusion")
            .icon("custom-icon")
            .timeout_ms(3000)
            .category("build")
            .duration_secs(12.5);

        assert_eq!(notif.title, "My Title");
        assert_eq!(notif.body, "My Body");
        assert_eq!(notif.subtitle.as_deref(), Some("Sub"));
        assert!(notif.sound);
        assert_eq!(notif.urgency, NotificationUrgency::Critical);
        assert_eq!(notif.get_priority(), NotificationPriority::Error);
        assert_eq!(notif.app_name, "CustomFusion");
        assert_eq!(notif.icon.as_deref(), Some("custom-icon"));
        assert_eq!(notif.timeout_ms, Some(3000));
        assert_eq!(notif.category.as_deref(), Some("build"));
        assert_eq!(notif.duration_secs, Some(12.5));
    }

    #[test]
    fn test_priority_constructors() {
        let info = Notification::info("Notice", "System is up to date");
        assert_eq!(info.get_priority(), NotificationPriority::Info);
        assert_eq!(info.urgency, NotificationUrgency::Low);
        assert_eq!(info.icon.as_deref(), Some("dialog-information"));

        let success = Notification::success("Deployed", "Service deployed to prod");
        assert_eq!(success.get_priority(), NotificationPriority::Success);
        assert_eq!(success.urgency, NotificationUrgency::Normal);
        assert_eq!(success.icon.as_deref(), Some("dialog-ok"));

        let warn = Notification::warning("Disk Usage", "Disk is 85% full");
        assert_eq!(warn.get_priority(), NotificationPriority::Warning);
        assert_eq!(warn.urgency, NotificationUrgency::Normal);
        assert_eq!(warn.icon.as_deref(), Some("dialog-warning"));

        let err = Notification::error("Fatal Error", "Out of memory");
        assert_eq!(err.get_priority(), NotificationPriority::Error);
        assert_eq!(err.urgency, NotificationUrgency::Critical);
        assert!(err.sound);
        assert_eq!(err.icon.as_deref(), Some("dialog-error"));
    }

    #[test]
    fn test_task_complete_builder() {
        let notif = Notification::task_complete("Refactor auth", Some(4.2));
        assert_eq!(notif.title, "Refactor auth");
        assert!(notif.body.contains("4.2s"));
        assert_eq!(notif.subtitle.as_deref(), Some("Fusion Task"));
        assert_eq!(notif.urgency, NotificationUrgency::Normal);
        assert_eq!(notif.category.as_deref(), Some("task_complete"));
    }

    #[test]
    fn test_long_running_task_complete_builder() {
        let notif = Notification::long_running_task_complete("Data Migration", 124.5);
        assert_eq!(notif.title, "Task Completed: Data Migration");
        assert!(notif.body.contains("2m 05s"));
        assert_eq!(notif.subtitle.as_deref(), Some("Long-running Task"));
        assert_eq!(notif.category.as_deref(), Some("long_task_complete"));
        assert_eq!(notif.duration_secs, Some(124.5));
    }

    #[test]
    fn test_subagent_complete_builder() {
        let notif = Notification::subagent_complete("ScoutAgent", "Mapped 45 files", Some(3.8));
        assert_eq!(notif.title, "Subagent Complete: ScoutAgent");
        assert!(notif.body.contains("Mapped 45 files"));
        assert!(notif.body.contains("3.8s"));
        assert_eq!(notif.subtitle.as_deref(), Some("Agent: ScoutAgent"));
        assert_eq!(notif.get_priority(), NotificationPriority::Success);
        assert_eq!(notif.category.as_deref(), Some("subagent_complete"));
    }

    #[test]
    fn test_subagent_failed_builder() {
        let notif =
            Notification::subagent_failed("CodeReviewer", "Syntax validation failed", Some(1.2));
        assert_eq!(notif.title, "Subagent Failed: CodeReviewer");
        assert!(notif.body.contains("Syntax validation failed"));
        assert!(notif.body.contains("1.2s"));
        assert_eq!(notif.subtitle.as_deref(), Some("Agent: CodeReviewer"));
        assert_eq!(notif.get_priority(), NotificationPriority::Error);
        assert!(notif.sound);
        assert_eq!(notif.category.as_deref(), Some("subagent_failed"));
    }

    #[test]
    fn test_turn_complete_builder() {
        let notif = Notification::turn_complete("Optimize DB", "claude-3-5-sonnet", Some(1.5));
        assert_eq!(notif.title, "Optimize DB");
        assert!(notif.body.contains("claude-3-5-sonnet"));
        assert!(notif.body.contains("1.5s"));
        assert_eq!(notif.subtitle.as_deref(), Some("Model: claude-3-5-sonnet"));
    }

    #[test]
    fn test_error_builder() {
        let notif = Notification::error("Build Failed", "Missing dependency 'foo'");
        assert_eq!(notif.title, "Build Failed");
        assert_eq!(notif.body, "Missing dependency 'foo'");
        assert_eq!(notif.urgency, NotificationUrgency::Critical);
        assert!(notif.sound);
        assert_eq!(notif.icon.as_deref(), Some("dialog-error"));
    }

    #[test]
    fn test_config_min_duration() {
        let mut cfg = NotificationConfig::default();
        cfg.min_duration_secs = Some(5.0);

        assert!(!cfg.is_duration_met(Some(4.9)));
        assert!(cfg.is_duration_met(Some(5.0)));
        assert!(cfg.is_duration_met(Some(10.0)));
        assert!(!cfg.is_duration_met(None));

        cfg.min_duration_secs = None;
        assert!(cfg.is_duration_met(Some(0.1)));
        assert!(cfg.is_duration_met(None));
    }

    #[test]
    fn test_config_enable_disable_toggle() {
        let mut cfg = NotificationConfig::new(true);
        assert!(cfg.enabled);

        cfg.disable();
        assert!(!cfg.enabled);

        cfg.enable();
        assert!(cfg.enabled);

        let toggled = cfg.toggle();
        assert!(!toggled);
        assert!(!cfg.enabled);
    }

    #[test]
    fn test_render_macos_command() {
        let notif = Notification::new("Fusion", "Task done")
            .subtitle("Agent")
            .sound(true);

        let (bin, args) = notif.render_macos_command();
        assert_eq!(bin, "osascript");
        assert_eq!(args[0], "-e");
        assert!(args[1].contains("on run argv"));
        assert_eq!(args[2], "Fusion");
        assert_eq!(args[3], "Task done");
        assert_eq!(args[4], "Agent");
        assert_eq!(args[5], "true");
    }

    #[test]
    fn test_render_linux_command() {
        let notif = Notification::new("Fusion Title", "Main body")
            .subtitle("Sub info")
            .urgency(NotificationUrgency::Critical)
            .app_name("FusionApp")
            .icon("custom-icon")
            .timeout_ms(4000);

        let (bin, args) = notif.render_linux_command();
        assert_eq!(bin, "notify-send");
        assert!(args.contains(&"-a".to_string()));
        assert!(args.contains(&"FusionApp".to_string()));
        assert!(args.contains(&"-u".to_string()));
        assert!(args.contains(&"critical".to_string()));
        assert!(args.contains(&"-i".to_string()));
        assert!(args.contains(&"custom-icon".to_string()));
        assert!(args.contains(&"-t".to_string()));
        assert!(args.contains(&"4000".to_string()));
        assert!(args.contains(&"Fusion Title".to_string()));
        assert!(args.contains(&"Sub info\nMain body".to_string()));
    }

    #[test]
    fn test_render_kdialog_command() {
        let notif = Notification::new("KDialog Title", "KDialog Body")
            .subtitle("KDialog Sub")
            .timeout_ms(6000);

        let (bin, args) = notif.render_kdialog_command();
        assert_eq!(bin, "kdialog");
        assert_eq!(args[0], "--title");
        assert_eq!(args[1], "KDialog Title");
        assert_eq!(args[2], "--passivepopup");
        assert_eq!(args[3], "KDialog Sub\nKDialog Body");
        assert_eq!(args[4], "6");
    }

    #[test]
    fn test_render_termux_command() {
        let notif = Notification::new("Termux Title", "Termux Content")
            .subtitle("Sub")
            .sound(true)
            .urgency(NotificationUrgency::Critical)
            .category("task_done");

        let (bin, args) = notif.render_termux_command();
        assert_eq!(bin, "termux-notification");
        assert!(args.contains(&"-t".to_string()));
        assert!(args.contains(&"Termux Title".to_string()));
        assert!(args.contains(&"-c".to_string()));
        assert!(args.contains(&"Sub: Termux Content".to_string()));
        assert!(args.contains(&"--id".to_string()));
        assert!(args.contains(&"fusion_task_done".to_string()));
        assert!(args.contains(&"--priority".to_string()));
        assert!(args.contains(&"high".to_string()));
        assert!(args.contains(&"--sound".to_string()));
        assert!(args.contains(&"--vibrate".to_string()));
    }

    #[test]
    fn test_render_windows_command() {
        let notif = Notification::new("Win'Title", "Win'Body");
        let (bin, args) = notif.render_windows_command();
        assert_eq!(bin, "powershell");
        assert!(args.contains(&"-NoProfile".to_string()));
        assert!(args.contains(&"-Command".to_string()));
        let cmd = args.last().unwrap();
        assert!(cmd.contains("Win''Title"));
        assert!(cmd.contains("Win''Body"));
        assert!(cmd.contains("ToastNotificationManager"));
        assert!(cmd.contains("ToastTemplateType"));
    }

    #[test]
    fn test_render_custom_command() {
        let notif = Notification::new("Title1", "Body1")
            .subtitle("Sub1")
            .priority(NotificationPriority::Warning)
            .app_name("MyApp");

        let (bin, args) = notif.render_custom_command(
            "notify-tool --title {title} --msg {body} --sub {subtitle} --urg {urgency} --prio {priority} --app {app}",
        );
        assert_eq!(bin, "notify-tool");
        assert_eq!(
            args,
            vec![
                "--title", "Title1", "--msg", "Body1", "--sub", "Sub1", "--urg", "normal",
                "--prio", "warning", "--app", "MyApp",
            ]
        );
    }

    #[test]
    fn test_render_terminal_osc_protocols() {
        let notif = Notification::new("Terminal Title", "Terminal Body");

        let osc777 = notif.render_terminal_osc_protocol(TerminalOscProtocol::Osc777);
        assert!(osc777.contains("777;notify;Terminal Title;Terminal Body"));

        let osc9 = notif.render_terminal_osc_protocol(TerminalOscProtocol::Osc9);
        assert!(osc9.contains("9;Terminal Title: Terminal Body"));

        let osc99 = notif.render_terminal_osc_protocol(TerminalOscProtocol::Osc99);
        assert!(osc99.contains("99;i=1"));
        assert!(osc99.contains("d=0;Terminal Title"));
        assert!(osc99.contains("p=body;Terminal Body"));
        let osc_all = notif.render_terminal_osc();
        assert!(!osc_all.is_empty());
    }
    #[test]
    fn test_send_terminal_osc_to_writer() {
        let notif = Notification::new("Test Title", "Test Body");
        let mut buffer = Vec::new();

        let res = notif.send_terminal_osc(&mut buffer);
        assert!(res.is_ok());

        let output_str = String::from_utf8_lossy(&buffer);
        assert!(output_str.contains("Test Title"));
        assert!(output_str.contains("Test Body"));
    }

    #[test]
    fn test_emit_terminal_osc_to_helper() {
        let mut buffer = Vec::new();
        let res = emit_terminal_osc_to("Stream Title", "Stream Body", &mut buffer);
        assert!(res.is_ok());

        let output_str = String::from_utf8_lossy(&buffer);
        assert!(output_str.contains("Stream Title"));
        assert!(output_str.contains("Stream Body"));
    }

    #[test]
    fn test_send_sync_suppressed_when_disabled() {
        let notif = Notification::new("Disabled Title", "Disabled Body");
        let cfg = NotificationConfig::new(false);

        let outcome = notif.send_sync(&cfg);
        assert!(outcome.suppressed);
        assert!(!outcome.is_delivered());
    }

    #[test]
    fn test_send_sync_suppressed_duration_threshold() {
        let notif = Notification::new("Fast Task", "Done in 0.1s").duration_secs(0.1);
        let mut cfg = NotificationConfig::default();
        cfg.min_duration_secs = Some(3.0);

        let outcome = notif.send_sync(&cfg);
        assert!(outcome.suppressed);
        assert!(!outcome.is_delivered());
    }

    #[test]
    fn test_format_duration_secs() {
        assert_eq!(format_duration_secs(0.05), "50ms");
        assert_eq!(format_duration_secs(0.85), "850ms");
        assert_eq!(format_duration_secs(1.23), "1.2s");
        assert_eq!(format_duration_secs(45.0), "45.0s");
        assert_eq!(format_duration_secs(65.0), "1m 05s");
        assert_eq!(format_duration_secs(125.0), "2m 05s");
    }

    #[test]
    fn test_escape_powershell() {
        assert_eq!(escape_powershell("normal text"), "normal text");
        assert_eq!(escape_powershell("user's input"), "user''s input");
        assert_eq!(escape_powershell("'''"), "''''''");
    }

    #[test]
    fn test_sanitize_terminal_text() {
        assert_eq!(sanitize_terminal_text("Clean text"), "Clean text");
        assert_eq!(sanitize_terminal_text("Text\x1b[31mRed\x07"), "Text[31mRed");
    }

    #[test]
    fn test_notification_serde_roundtrip() {
        let notif = Notification::new("Serde Title", "Serde Body")
            .subtitle("Serde Sub")
            .sound(true)
            .priority(NotificationPriority::Error)
            .category("serde_test")
            .duration_secs(3.25);

        let json = serde_json::to_string(&notif).unwrap();
        let parsed: Notification = serde_json::from_str(&json).unwrap();

        assert_eq!(notif, parsed);
    }

    #[test]
    fn test_config_serde_roundtrip() {
        let mut cfg = NotificationConfig::default();
        cfg.enabled = true;
        cfg.sound = true;
        cfg.min_duration_secs = Some(2.5);
        cfg.urgency = NotificationUrgency::Low;

        let json = serde_json::to_string(&cfg).unwrap();
        let parsed: NotificationConfig = serde_json::from_str(&json).unwrap();

        assert_eq!(cfg, parsed);
    }

    #[test]
    fn test_priority_serde_roundtrip() {
        for prio in &[
            NotificationPriority::Info,
            NotificationPriority::Success,
            NotificationPriority::Warning,
            NotificationPriority::Error,
        ] {
            let json = serde_json::to_string(prio).unwrap();
            let parsed: NotificationPriority = serde_json::from_str(&json).unwrap();
            assert_eq!(*prio, parsed);
        }
    }

    #[test]
    fn test_backend_availability_and_names() {
        assert_eq!(NotificationBackend::Auto.name(), "auto");
        assert_eq!(NotificationBackend::MacOS.name(), "macos (osascript)");
        assert_eq!(NotificationBackend::Linux.name(), "linux (notify-send)");
        assert_eq!(NotificationBackend::Windows.name(), "windows (powershell)");
        assert_eq!(
            NotificationBackend::Termux.name(),
            "termux (termux-notification)"
        );
        assert_eq!(NotificationBackend::TerminalOsc.name(), "terminal (osc)");
        assert_eq!(NotificationBackend::Disabled.name(), "disabled");

        assert!(NotificationBackend::Auto.is_available());
        assert!(NotificationBackend::TerminalOsc.is_available());
        assert!(!NotificationBackend::Disabled.is_available());
    }

    #[test]
    fn test_wrap_tmux_passthrough() {
        let payload = "\x1b]99;;Hello\x1b\\";
        let wrapped = wrap_tmux_passthrough(payload);
        assert_eq!(wrapped, "\x1bPtmux;\x1b\x1b]99;;Hello\x1b\x1b\\\x1b\\");
    }

    #[test]
    fn test_format_multiplexer_terminal_sequence() {
        let orig_tmux = std::env::var("TMUX").ok();
        let orig_zellij = std::env::var("ZELLIJ").ok();

        let payload = "\x1b]9;Test Title: Test Body\x07";
        // When TMUX and ZELLIJ are not set, sequence is unchanged
        std::env::remove_var("TMUX");
        std::env::remove_var("ZELLIJ");
        assert_eq!(format_multiplexer_terminal_sequence(payload), payload);

        // When TMUX is set, wraps in DCS passthrough followed by BEL
        std::env::set_var("TMUX", "/tmp/tmux-1000/default,1234,0");
        let tmux_formatted = format_multiplexer_terminal_sequence(payload);
        assert!(tmux_formatted.starts_with("\x1bPtmux;\x1b\x1b]9;"));
        assert!(tmux_formatted.ends_with("\x1b\\\x07"));
        std::env::remove_var("TMUX");

        // When ZELLIJ is set, appends BEL
        std::env::set_var("ZELLIJ", "1");
        let zellij_formatted = format_multiplexer_terminal_sequence(payload);
        assert_eq!(zellij_formatted, format!("{payload}\x07"));
        std::env::remove_var("ZELLIJ");

        // Plain Bell is never wrapped
        std::env::set_var("TMUX", "/tmp/tmux-1000/default,1234,0");
        assert_eq!(
            format_multiplexer_terminal_sequence(TERMINAL_BELL),
            TERMINAL_BELL
        );

        // Restore env
        if let Some(val) = orig_tmux {
            std::env::set_var("TMUX", val);
        } else {
            std::env::remove_var("TMUX");
        }
        if let Some(val) = orig_zellij {
            std::env::set_var("ZELLIJ", val);
        } else {
            std::env::remove_var("ZELLIJ");
        }
    }
    #[test]
    fn test_notification_triggers_and_constructors() {
        assert_eq!(NotificationTrigger::Completion.as_str(), "completion");
        assert_eq!(NotificationTrigger::Error.as_str(), "error");
        assert_eq!(NotificationTrigger::Ask.as_str(), "ask");

        let comp = Notification::completion("Done", "Task completed");
        assert_eq!(comp.get_trigger(), Some(NotificationTrigger::Completion));
        assert_eq!(comp.get_priority(), NotificationPriority::Success);

        let err = Notification::error("Failed", "Network timeout");
        assert_eq!(err.get_trigger(), Some(NotificationTrigger::Error));
        assert_eq!(err.get_priority(), NotificationPriority::Error);
        assert!(err.sound);

        let ask = Notification::ask("Question", "Allow bash command execution?");
        assert_eq!(ask.get_trigger(), Some(NotificationTrigger::Ask));
        assert_eq!(ask.get_priority(), NotificationPriority::Warning);
        assert!(ask.sound);
    }

    #[test]
    fn test_rich_osc99_notification_protocol() {
        let notif = Notification::new("Session", "Complete")
            .category("complete-1")
            .app_name("Oh My Pi")
            .trigger(NotificationTrigger::Completion)
            .priority(NotificationPriority::Success)
            .timeout_ms(5000);

        let osc = notif.render_osc99();
        assert!(osc.contains("99;i=complete-1:f="));
        assert!(osc.contains("a=focus"));
        assert!(osc.contains("u=1"));
        assert!(osc.contains("t=")); // base64 encoded trigger
        assert!(osc.contains("w=5000"));
        assert!(osc.contains("d=0;Session\x1b\\"));
        assert!(osc.contains("i=complete-1:p=body;Complete\x1b\\"));

        // Test base64 encoding of unsafe control characters in payload
        let unsafe_notif = Notification::new("Line 1\nLine 2", "").category("unsafe");
        let unsafe_osc = unsafe_notif.render_osc99();
        assert!(unsafe_osc.contains("e=1"));
        assert!(unsafe_osc.contains("TGluZSAxCkxpbmUgMg=="));
    }

    #[test]
    fn test_trigger_serde_roundtrip() {
        for trigger in &[
            NotificationTrigger::Completion,
            NotificationTrigger::Error,
            NotificationTrigger::Ask,
        ] {
            let json = serde_json::to_string(trigger).unwrap();
            let parsed: NotificationTrigger = serde_json::from_str(&json).unwrap();
            assert_eq!(*trigger, parsed);
        }
    }
}
