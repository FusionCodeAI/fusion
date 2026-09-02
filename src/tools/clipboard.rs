//! Cross-platform clipboard tool with support for Termux, macOS (`pbcopy`/`pbpaste`),
//! Linux (`wl-copy`/`wl-paste`, `xclip`, `xsel`), Windows (`clip.exe`, PowerShell),
//! and graceful in-memory fallback.

use std::path::Path;
use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tokio::io::AsyncWriteExt;
use tokio::sync::RwLock;
use tokio::time::timeout;

use crate::tools::types::{Tool, ToolContext};

/// Default timeout for clipboard sub-process invocations to prevent hanging on headless or frozen display servers.
pub const DEFAULT_CLIPBOARD_TIMEOUT: Duration = Duration::from_secs(4);

/// Supported clipboard backend kinds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ClipboardBackendKind {
    /// Android / Termux API (`termux-clipboard-get`, `termux-clipboard-set`).
    Termux,
    /// macOS native CLI (`pbcopy`, `pbpaste`).
    MacOS,
    /// Wayland compositor CLI (`wl-copy`, `wl-paste`).
    Wayland,
    /// X11 CLI via `xclip`.
    XClip,
    /// X11 CLI via `xsel`.
    XSel,
    /// Windows native CLI (`clip.exe` for write, PowerShell `Get-Clipboard` for read).
    Windows,
    /// In-memory clipboard buffer (used in headless, CI, Docker, or fallback environments).
    InMemory,
    /// Custom user-specified commands.
    Custom,
}

impl ClipboardBackendKind {
    /// Returns the human-readable display name for this backend.
    pub fn display_name(&self) -> &'static str {
        match self {
            Self::Termux => "Termux (termux-clipboard-get/set)",
            Self::MacOS => "macOS (pbcopy/pbpaste)",
            Self::Wayland => "Wayland (wl-copy/wl-paste)",
            Self::XClip => "X11 (xclip)",
            Self::XSel => "X11 (xsel)",
            Self::Windows => "Windows (clip.exe / PowerShell)",
            Self::InMemory => "In-Memory Fallback",
            Self::Custom => "Custom Command",
        }
    }

    /// Whether this backend interacts with the OS graphical/system clipboard.
    pub fn is_system_backend(&self) -> bool {
        !matches!(self, Self::InMemory)
    }
}

/// Information describing the active clipboard backend and its status.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClipboardStatus {
    pub backend: ClipboardBackendKind,
    pub backend_name: String,
    pub is_system_available: bool,
    pub fallback_active: bool,
    pub content_len_chars: usize,
    pub content_len_bytes: usize,
    pub is_empty: bool,
}

/// Cross-platform clipboard backend manager.
#[derive(Debug, Clone)]
pub struct ClipboardManager {
    backend: ClipboardBackendKind,
    in_memory_buffer: Arc<RwLock<String>>,
    custom_read_cmd: Option<(String, Vec<String>)>,
    custom_write_cmd: Option<(String, Vec<String>)>,
    timeout_duration: Duration,
}

impl Default for ClipboardManager {
    fn default() -> Self {
        Self::new()
    }
}

impl ClipboardManager {
    /// Creates a new `ClipboardManager` with automatic platform detection.
    pub fn new() -> Self {
        let backend = Self::detect_backend();
        Self {
            backend,
            in_memory_buffer: Arc::new(RwLock::new(String::new())),
            custom_read_cmd: None,
            custom_write_cmd: None,
            timeout_duration: DEFAULT_CLIPBOARD_TIMEOUT,
        }
    }

    /// Creates an explicitly in-memory clipboard manager (useful for testing or headless runs).
    pub fn in_memory() -> Self {
        Self {
            backend: ClipboardBackendKind::InMemory,
            in_memory_buffer: Arc::new(RwLock::new(String::new())),
            custom_read_cmd: None,
            custom_write_cmd: None,
            timeout_duration: DEFAULT_CLIPBOARD_TIMEOUT,
        }
    }

    /// Creates a clipboard manager with a specific backend kind.
    pub fn with_backend(backend: ClipboardBackendKind) -> Self {
        Self {
            backend,
            in_memory_buffer: Arc::new(RwLock::new(String::new())),
            custom_read_cmd: None,
            custom_write_cmd: None,
            timeout_duration: DEFAULT_CLIPBOARD_TIMEOUT,
        }
    }

    /// Creates a clipboard manager with custom read and write commands.
    pub fn with_custom_commands(
        read_cmd: (String, Vec<String>),
        write_cmd: (String, Vec<String>),
    ) -> Self {
        Self {
            backend: ClipboardBackendKind::Custom,
            in_memory_buffer: Arc::new(RwLock::new(String::new())),
            custom_read_cmd: Some(read_cmd),
            custom_write_cmd: Some(write_cmd),
            timeout_duration: DEFAULT_CLIPBOARD_TIMEOUT,
        }
    }

    /// Sets the command timeout duration.
    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout_duration = timeout;
        self
    }

    /// Returns the currently active backend kind.
    pub fn backend_kind(&self) -> ClipboardBackendKind {
        self.backend
    }

    /// Detects the most appropriate clipboard backend available on the current OS/environment.
    pub fn detect_backend() -> ClipboardBackendKind {
        // 1. Android / Termux environment check
        if is_termux_environment() {
            if has_binary_or_path("termux-clipboard-get", &["/data/data/com.termux/files/usr/bin/termux-clipboard-get"]) {
                return ClipboardBackendKind::Termux;
            }
        }

        // 2. macOS check
        #[cfg(target_os = "macos")]
        {
            if has_binary("pbcopy") && has_binary("pbpaste") {
                return ClipboardBackendKind::MacOS;
            }
        }

        // 3. Windows check
        #[cfg(target_os = "windows")]
        {
            return ClipboardBackendKind::Windows;
        }

        // 4. Linux / BSD / Unix environments
        #[cfg(not(any(target_os = "macos", target_os = "windows")))]
        {
            // Wayland priority when WAYLAND_DISPLAY is set
            if std::env::var_os("WAYLAND_DISPLAY").is_some() && has_binary("wl-copy") && has_binary("wl-paste") {
                return ClipboardBackendKind::Wayland;
            }

            // X11 with xclip
            if (std::env::var_os("DISPLAY").is_some() || has_binary("xclip")) && has_binary("xclip") {
                return ClipboardBackendKind::XClip;
            }

            // X11 with xsel
            if (std::env::var_os("DISPLAY").is_some() || has_binary("xsel")) && has_binary("xsel") {
                return ClipboardBackendKind::XSel;
            }

            // Wayland fallback even if WAYLAND_DISPLAY not explicitly exported
            if has_binary("wl-copy") && has_binary("wl-paste") {
                return ClipboardBackendKind::Wayland;
            }

            // Termux check for non-standard path
            if has_binary("termux-clipboard-get") {
                return ClipboardBackendKind::Termux;
            }
        }

        // 5. Fallback to in-memory clipboard buffer
        ClipboardBackendKind::InMemory
    }

    /// Reads text from the clipboard.
    ///
    /// If the primary system backend fails or is unavailable, falls back to the in-memory buffer.
    pub async fn read_text(&self) -> anyhow::Result<String> {
        match self.backend {
            ClipboardBackendKind::InMemory => {
                let buf = self.in_memory_buffer.read().await;
                Ok(buf.clone())
            }
            ClipboardBackendKind::MacOS => {
                match self.run_read_command("pbpaste", &[]).await {
                    Ok(text) => {
                        // Sync to in-memory buffer for consistency
                        *self.in_memory_buffer.write().await = text.clone();
                        Ok(text)
                    }
                    Err(e) => {
                        // Fallback to in-memory
                        let buf = self.in_memory_buffer.read().await;
                        if !buf.is_empty() {
                            Ok(buf.clone())
                        } else {
                            Err(anyhow::anyhow!("macOS pbpaste failed: {}. In-memory clipboard is empty.", e))
                        }
                    }
                }
            }
            ClipboardBackendKind::Termux => {
                let bin = get_termux_path("termux-clipboard-get");
                match self.run_read_command(&bin, &[]).await {
                    Ok(text) => {
                        *self.in_memory_buffer.write().await = text.clone();
                        Ok(text)
                    }
                    Err(e) => {
                        let buf = self.in_memory_buffer.read().await;
                        if !buf.is_empty() {
                            Ok(buf.clone())
                        } else {
                            Err(anyhow::anyhow!("Termux clipboard get failed: {}. In-memory clipboard is empty.", e))
                        }
                    }
                }
            }
            ClipboardBackendKind::Wayland => {
                match self.run_read_command("wl-paste", &["--no-newline"]).await {
                    Ok(text) => {
                        *self.in_memory_buffer.write().await = text.clone();
                        Ok(text)
                    }
                    Err(_) => {
                        // Retry without --no-newline flag in case of older wl-paste
                        match self.run_read_command("wl-paste", &[]).await {
                            Ok(text) => {
                                *self.in_memory_buffer.write().await = text.clone();
                                Ok(text)
                            }
                            Err(e) => {
                                let buf = self.in_memory_buffer.read().await;
                                if !buf.is_empty() {
                                    Ok(buf.clone())
                                } else {
                                    Err(anyhow::anyhow!("Wayland wl-paste failed: {}. In-memory clipboard is empty.", e))
                                }
                            }
                        }
                    }
                }
            }
            ClipboardBackendKind::XClip => {
                match self.run_read_command("xclip", &["-selection", "clipboard", "-out"]).await {
                    Ok(text) => {
                        *self.in_memory_buffer.write().await = text.clone();
                        Ok(text)
                    }
                    Err(e) => {
                        let buf = self.in_memory_buffer.read().await;
                        if !buf.is_empty() {
                            Ok(buf.clone())
                        } else {
                            Err(anyhow::anyhow!("xclip read failed: {}. In-memory clipboard is empty.", e))
                        }
                    }
                }
            }
            ClipboardBackendKind::XSel => {
                match self.run_read_command("xsel", &["--clipboard", "--output"]).await {
                    Ok(text) => {
                        *self.in_memory_buffer.write().await = text.clone();
                        Ok(text)
                    }
                    Err(e) => {
                        let buf = self.in_memory_buffer.read().await;
                        if !buf.is_empty() {
                            Ok(buf.clone())
                        } else {
                            Err(anyhow::anyhow!("xsel read failed: {}. In-memory clipboard is empty.", e))
                        }
                    }
                }
            }
            ClipboardBackendKind::Windows => {
                // Try PowerShell Get-Clipboard
                let ps_args = [
                    "-NoProfile",
                    "-NonInteractive",
                    "-Command",
                    "[Console]::OutputEncoding = [System.Text.Encoding]::UTF8; Get-Clipboard",
                ];
                let bin = if has_binary("powershell.exe") {
                    "powershell.exe"
                } else if has_binary("powershell") {
                    "powershell"
                } else if has_binary("pwsh.exe") {
                    "pwsh.exe"
                } else if has_binary("pwsh") {
                    "pwsh"
                } else {
                    "powershell"
                };

                match self.run_read_command(bin, &ps_args).await {
                    Ok(mut text) => {
                        // Standardize Windows line endings (\r\n -> \n)
                        if text.contains("\r\n") {
                            text = text.replace("\r\n", "\n");
                        }
                        *self.in_memory_buffer.write().await = text.clone();
                        Ok(text)
                    }
                    Err(e) => {
                        let buf = self.in_memory_buffer.read().await;
                        if !buf.is_empty() {
                            Ok(buf.clone())
                        } else {
                            Err(anyhow::anyhow!("Windows clipboard read failed: {}. In-memory clipboard is empty.", e))
                        }
                    }
                }
            }
            ClipboardBackendKind::Custom => {
                if let Some((cmd, args)) = &self.custom_read_cmd {
                    let str_args: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
                    match self.run_read_command(cmd, &str_args).await {
                        Ok(text) => {
                            *self.in_memory_buffer.write().await = text.clone();
                            Ok(text)
                        }
                        Err(e) => Err(anyhow::anyhow!("Custom read command '{}' failed: {}", cmd, e)),
                    }
                } else {
                    let buf = self.in_memory_buffer.read().await;
                    Ok(buf.clone())
                }
            }
        }
    }

    /// Writes text to the clipboard.
    ///
    /// Updates the in-memory buffer as well as calling the appropriate OS utility.
    pub async fn write_text(&self, text: &str) -> anyhow::Result<()> {
        // Always mirror to in-memory buffer
        *self.in_memory_buffer.write().await = text.to_string();

        match self.backend {
            ClipboardBackendKind::InMemory => Ok(()),
            ClipboardBackendKind::MacOS => {
                self.run_write_command("pbcopy", &[], text).await
            }
            ClipboardBackendKind::Termux => {
                let bin = get_termux_path("termux-clipboard-set");
                self.run_write_command(&bin, &[], text).await
            }
            ClipboardBackendKind::Wayland => {
                self.run_write_command("wl-copy", &[], text).await
            }
            ClipboardBackendKind::XClip => {
                self.run_write_command("xclip", &["-selection", "clipboard", "-in"], text).await
            }
            ClipboardBackendKind::XSel => {
                self.run_write_command("xsel", &["--clipboard", "--input"], text).await
            }
            ClipboardBackendKind::Windows => {
                // Try clip.exe first (fastest and built into all Windows versions)
                let clip_bin = if has_binary("clip.exe") {
                    "clip.exe"
                } else if has_binary("clip") {
                    "clip"
                } else {
                    "clip.exe"
                };

                match self.run_write_command(clip_bin, &[], text).await {
                    Ok(()) => Ok(()),
                    Err(_) => {
                        // Fallback to PowerShell Set-Clipboard
                        let bin = if has_binary("powershell.exe") {
                            "powershell.exe"
                        } else if has_binary("pwsh.exe") {
                            "pwsh.exe"
                        } else {
                            "powershell"
                        };
                        let ps_args = [
                            "-NoProfile",
                            "-NonInteractive",
                            "-Command",
                            "$input | Set-Clipboard",
                        ];
                        self.run_write_command(bin, &ps_args, text).await
                    }
                }
            }
            ClipboardBackendKind::Custom => {
                if let Some((cmd, args)) = &self.custom_write_cmd {
                    let str_args: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
                    self.run_write_command(cmd, &str_args, text).await
                } else {
                    Ok(())
                }
            }
        }
    }

    /// Clears the clipboard content by writing an empty string.
    pub async fn clear(&self) -> anyhow::Result<()> {
        self.write_text("").await
    }

    /// Gets current clipboard status.
    pub async fn status(&self) -> ClipboardStatus {
        let content = self.read_text().await.unwrap_or_default();
        let chars = content.chars().count();
        let bytes = content.len();
        let is_empty = content.is_empty();

        ClipboardStatus {
            backend: self.backend,
            backend_name: self.backend.display_name().to_string(),
            is_system_available: self.backend.is_system_backend(),
            fallback_active: matches!(self.backend, ClipboardBackendKind::InMemory),
            content_len_chars: chars,
            content_len_bytes: bytes,
            is_empty,
        }
    }

    /// Helper to execute an external CLI tool that outputs clipboard content to stdout.
    async fn run_read_command(&self, prog: &str, args: &[&str]) -> anyhow::Result<String> {
        let mut cmd = tokio::process::Command::new(prog);
        cmd.args(args);
        cmd.stdout(Stdio::piped());
        cmd.stderr(Stdio::piped());
        cmd.stdin(Stdio::null());

        #[cfg(windows)]
        {
            // Avoid creating a console window on Windows
            const CREATE_NO_WINDOW: u32 = 0x08000000;
            cmd.creation_flags(CREATE_NO_WINDOW);
        }

        let fut = async {
            let child = cmd.spawn().map_err(|e| anyhow::anyhow!("Failed to spawn '{}': {}", prog, e))?;
            let output = child.wait_with_output().await.map_err(|e| anyhow::anyhow!("Failed waiting for '{}': {}", prog, e))?;
            
            if !output.status.success() {
                let err_msg = String::from_utf8_lossy(&output.stderr);
                return Err(anyhow::anyhow!("'{}' exited with error ({}): {}", prog, output.status, err_msg.trim()));
            }

            let text = String::from_utf8(output.stdout)
                .map_err(|e| anyhow::anyhow!("Non-UTF-8 clipboard content from '{}': {}", prog, e))?;
            Ok(text)
        };

        match timeout(self.timeout_duration, fut).await {
            Ok(res) => res,
            Err(_) => Err(anyhow::anyhow!("Clipboard read command '{}' timed out after {:?}", prog, self.timeout_duration)),
        }
    }

    /// Helper to execute an external CLI tool that receives clipboard content via stdin.
    async fn run_write_command(&self, prog: &str, args: &[&str], text: &str) -> anyhow::Result<()> {
        let mut cmd = tokio::process::Command::new(prog);
        cmd.args(args);
        cmd.stdin(Stdio::piped());
        cmd.stdout(Stdio::piped());
        cmd.stderr(Stdio::piped());

        #[cfg(windows)]
        {
            const CREATE_NO_WINDOW: u32 = 0x08000000;
            cmd.creation_flags(CREATE_NO_WINDOW);
        }

        let bytes = text.as_bytes().to_vec();
        let prog_str = prog.to_string();

        let fut = async {
            let mut child = cmd.spawn().map_err(|e| anyhow::anyhow!("Failed to spawn '{}': {}", prog_str, e))?;
            
            if let Some(mut stdin) = child.stdin.take() {
                stdin.write_all(&bytes).await.map_err(|e| anyhow::anyhow!("Failed writing to '{}' stdin: {}", prog_str, e))?;
                stdin.flush().await.map_err(|e| anyhow::anyhow!("Failed flushing '{}' stdin: {}", prog_str, e))?;
                drop(stdin); // Signal EOF
            }

            let output = child.wait_with_output().await.map_err(|e| anyhow::anyhow!("Failed waiting for '{}': {}", prog_str, e))?;
            
            if !output.status.success() {
                let err_msg = String::from_utf8_lossy(&output.stderr);
                return Err(anyhow::anyhow!("'{}' write exited with error ({}): {}", prog_str, output.status, err_msg.trim()));
            }

            Ok(())
        };

        match timeout(self.timeout_duration, fut).await {
            Ok(res) => res,
            Err(_) => Err(anyhow::anyhow!("Clipboard write command '{}' timed out after {:?}", prog, self.timeout_duration)),
        }
    }
}

// ---------------------------------------------------------------------------
// Helpers for environment & binary detection
// ---------------------------------------------------------------------------

/// Checks if running in Android / Termux.
fn is_termux_environment() -> bool {
    if std::env::var_os("TERMUX_VERSION").is_some() {
        return true;
    }
    if let Ok(prefix) = std::env::var("PREFIX") {
        if prefix.contains("com.termux") {
            return true;
        }
    }
    if Path::new("/data/data/com.termux/files/usr").exists() {
        return true;
    }
    false
}

/// Resolves Termux utility paths.
fn get_termux_path(bin_name: &str) -> String {
    let termux_path = format!("/data/data/com.termux/files/usr/bin/{}", bin_name);
    if Path::new(&termux_path).exists() {
        termux_path
    } else {
        bin_name.to_string()
    }
}

/// Checks if a binary is present in system PATH or specific candidate absolute paths.
fn has_binary_or_path(bin_name: &str, extra_paths: &[&str]) -> bool {
    for p in extra_paths {
        if Path::new(p).is_file() {
            return true;
        }
    }
    has_binary(bin_name)
}

/// Checks if a command is executable / in PATH.
fn has_binary(bin_name: &str) -> bool {
    #[cfg(windows)]
    {
        let name_with_ext = if bin_name.ends_with(".exe") || bin_name.ends_with(".cmd") || bin_name.ends_with(".bat") {
            bin_name.to_string()
        } else {
            format!("{}.exe", bin_name)
        };

        if let Ok(paths) = std::env::var("PATH") {
            for dir in std::env::split_paths(&paths) {
                if dir.join(&name_with_ext).is_file() || dir.join(bin_name).is_file() {
                    return true;
                }
            }
        }
        false
    }

    #[cfg(not(windows))]
    {
        if bin_name.starts_with('/') || bin_name.starts_with("./") {
            return Path::new(bin_name).is_file();
        }

        if let Ok(paths) = std::env::var("PATH") {
            for dir in std::env::split_paths(&paths) {
                let full = dir.join(bin_name);
                if full.is_file() {
                    return true;
                }
            }
        }
        false
    }
}

// ---------------------------------------------------------------------------
// Tool Implementations
// ---------------------------------------------------------------------------

/// Unified cross-platform clipboard tool implementing `Tool`.
///
/// Supports reading from, writing to, clearing, and querying the clipboard.
#[derive(Debug, Clone)]
pub struct ClipboardTool {
    manager: ClipboardManager,
}

impl Default for ClipboardTool {
    fn default() -> Self {
        Self::new()
    }
}

impl ClipboardTool {
    /// Creates a new `ClipboardTool` with auto-detected platform backend.
    pub fn new() -> Self {
        Self {
            manager: ClipboardManager::new(),
        }
    }

    /// Creates an in-memory `ClipboardTool` for isolated tests or headless environments.
    pub fn in_memory() -> Self {
        Self {
            manager: ClipboardManager::in_memory(),
        }
    }

    /// Creates a `ClipboardTool` with a specific backend kind.
    pub fn with_backend(backend: ClipboardBackendKind) -> Self {
        Self {
            manager: ClipboardManager::with_backend(backend),
        }
    }

    /// Creates a `ClipboardTool` using an existing `ClipboardManager`.
    pub fn with_manager(manager: ClipboardManager) -> Self {
        Self { manager }
    }

    /// Reads clipboard contents directly.
    pub async fn read(&self) -> anyhow::Result<String> {
        self.manager.read_text().await
    }

    /// Writes text to clipboard directly.
    pub async fn write(&self, text: &str) -> anyhow::Result<()> {
        self.manager.write_text(text).await
    }

    /// Clears clipboard directly.
    pub async fn clear(&self) -> anyhow::Result<()> {
        self.manager.clear().await
    }

    /// Returns backend status information.
    pub async fn status(&self) -> ClipboardStatus {
        self.manager.status().await
    }
}

#[async_trait]
impl Tool for ClipboardTool {
    fn name(&self) -> &str {
        "clipboard"
    }

    fn description(&self) -> &str {
        "Read from, write to, or clear the system clipboard with cross-platform support for Termux, macOS (pbcopy/pbpaste), Linux (wl-copy/xclip/xsel), and Windows (clip.exe/powershell)."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "action": {
                    "type": "string",
                    "description": "Action to perform: 'read' (or 'paste'/'get'), 'write' (or 'copy'/'set'), 'clear', or 'status'/'info'. Defaults to 'read' if content is omitted, or 'write' if content is provided.",
                    "enum": ["read", "write", "paste", "copy", "get", "set", "clear", "status", "info"]
                },
                "content": {
                    "type": "string",
                    "description": "Text content to copy/write to clipboard (required for write/set/copy action)."
                },
                "text": {
                    "type": "string",
                    "description": "Alias for content."
                },
                "trim": {
                    "type": "boolean",
                    "description": "Whether to trim leading and trailing whitespace from clipboard content (optional, default: false)."
                },
                "limit": {
                    "type": "integer",
                    "description": "Maximum number of characters to return when reading (optional)."
                }
            }
        })
    }

    async fn execute(&self, args: Value, _ctx: &ToolContext) -> anyhow::Result<String> {
        // Extract content / text if supplied
        let content = args
            .get("content")
            .and_then(|v| v.as_str())
            .or_else(|| args.get("text").and_then(|v| v.as_str()));

        // Infer action: if content is provided and action is not specified, default to "write"
        let action_str = args
            .get("action")
            .and_then(|v| v.as_str())
            .map(|s| s.to_lowercase())
            .unwrap_or_else(|| {
                if content.is_some() {
                    "write".to_string()
                } else {
                    "read".to_string()
                }
            });

        let trim = args.get("trim").and_then(|v| v.as_bool()).unwrap_or(false);
        let limit = args.get("limit").and_then(|v| v.as_u64()).map(|n| n as usize);

        match action_str.as_str() {
            "read" | "paste" | "get" => {
                let mut text = self.manager.read_text().await?;
                if trim {
                    text = text.trim().to_string();
                }
                if let Some(max_chars) = limit {
                    if text.chars().count() > max_chars {
                        let truncated: String = text.chars().take(max_chars).collect();
                        return Ok(format!("{}\n[... truncated (exceeds {} characters) ...]", truncated, max_chars));
                    }
                }
                if text.is_empty() {
                    Ok("(clipboard is empty)".to_string())
                } else {
                    Ok(text)
                }
            }
            "write" | "copy" | "set" => {
                let text_to_write = content.ok_or_else(|| {
                    anyhow::anyhow!("Missing required parameter: 'content' or 'text' for clipboard write action")
                })?;
                let text = if trim { text_to_write.trim() } else { text_to_write };
                self.manager.write_text(text).await?;
                
                let char_count = text.chars().count();
                let line_count = text.lines().count();
                let backend_name = self.manager.backend_kind().display_name();

                Ok(format!(
                    "Successfully copied {} characters ({} lines) to clipboard via {}.",
                    char_count, line_count, backend_name
                ))
            }
            "clear" => {
                self.manager.clear().await?;
                Ok("Clipboard cleared successfully.".to_string())
            }
            "status" | "info" => {
                let status = self.manager.status().await;
                serde_json::to_string_pretty(&status)
                    .map_err(|e| anyhow::anyhow!("Failed serializing clipboard status: {}", e))
            }
            unknown => {
                Err(anyhow::anyhow!(
                    "Unknown clipboard action: '{}'. Valid actions are: 'read', 'write', 'clear', 'status'.",
                    unknown
                ))
            }
        }
    }
}

/// Dedicated clipboard read tool for agents or workflows preferring distinct read/write tools.
#[derive(Debug, Clone, Default)]
pub struct ReadClipboardTool {
    clipboard: ClipboardTool,
}

impl ReadClipboardTool {
    pub fn new() -> Self {
        Self {
            clipboard: ClipboardTool::new(),
        }
    }

    pub fn in_memory() -> Self {
        Self {
            clipboard: ClipboardTool::in_memory(),
        }
    }
}

#[async_trait]
impl Tool for ReadClipboardTool {
    fn name(&self) -> &str {
        "read_clipboard"
    }

    fn description(&self) -> &str {
        "Read the current text content of the system clipboard."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "trim": {
                    "type": "boolean",
                    "description": "Whether to trim leading and trailing whitespace (optional, default: false)."
                },
                "limit": {
                    "type": "integer",
                    "description": "Maximum number of characters to return (optional)."
                }
            }
        })
    }

    async fn execute(&self, args: Value, ctx: &ToolContext) -> anyhow::Result<String> {
        let mut forwarded = args;
        if let Some(obj) = forwarded.as_object_mut() {
            obj.insert("action".to_string(), Value::String("read".to_string()));
        } else {
            forwarded = json!({ "action": "read" });
        }
        self.clipboard.execute(forwarded, ctx).await
    }
}

/// Dedicated clipboard write tool for agents or workflows preferring distinct read/write tools.
#[derive(Debug, Clone, Default)]
pub struct WriteClipboardTool {
    clipboard: ClipboardTool,
}

impl WriteClipboardTool {
    pub fn new() -> Self {
        Self {
            clipboard: ClipboardTool::new(),
        }
    }

    pub fn in_memory() -> Self {
        Self {
            clipboard: ClipboardTool::in_memory(),
        }
    }
}

#[async_trait]
impl Tool for WriteClipboardTool {
    fn name(&self) -> &str {
        "write_clipboard"
    }

    fn description(&self) -> &str {
        "Write text content to the system clipboard."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "content": {
                    "type": "string",
                    "description": "Text content to copy to the clipboard."
                },
                "text": {
                    "type": "string",
                    "description": "Alias for content."
                },
                "trim": {
                    "type": "boolean",
                    "description": "Whether to trim leading and trailing whitespace before copying (optional, default: false)."
                }
            },
            "required": ["content"]
        })
    }

    async fn execute(&self, args: Value, ctx: &ToolContext) -> anyhow::Result<String> {
        let mut forwarded = args;
        if let Some(obj) = forwarded.as_object_mut() {
            obj.insert("action".to_string(), Value::String("write".to_string()));
        } else {
            forwarded = json!({ "action": "write" });
        }
        self.clipboard.execute(forwarded, ctx).await
    }
}

// ---------------------------------------------------------------------------
// Unit Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_in_memory_read_write_clear() {
        let tool = ClipboardTool::in_memory();
        let ctx = ToolContext::default();

        // 1. Initial empty read
        let initial = tool.execute(json!({ "action": "read" }), &ctx).await.unwrap();
        assert_eq!(initial, "(clipboard is empty)");

        // 2. Write text
        let write_res = tool
            .execute(
                json!({
                    "action": "write",
                    "content": "Hello World from Fusion!"
                }),
                &ctx,
            )
            .await
            .unwrap();
        assert!(write_res.contains("Successfully copied"));
        assert!(write_res.contains("In-Memory Fallback"));

        // 3. Read back text
        let read_res = tool.execute(json!({ "action": "read" }), &ctx).await.unwrap();
        assert_eq!(read_res, "Hello World from Fusion!");

        // 4. Read with trimming
        tool.write("   padded text   ").await.unwrap();
        let trimmed_res = tool
            .execute(json!({ "action": "read", "trim": true }), &ctx)
            .await
            .unwrap();
        assert_eq!(trimmed_res, "padded text");

        // 5. Read with limit
        let limit_res = tool
            .execute(json!({ "action": "read", "limit": 6 }), &ctx)
            .await
            .unwrap();
        assert!(limit_res.contains("   pad"));
        assert!(limit_res.contains("truncated"));

        // 6. Clear clipboard
        let clear_res = tool.execute(json!({ "action": "clear" }), &ctx).await.unwrap();
        assert!(clear_res.contains("cleared successfully"));

        let after_clear = tool.execute(json!({ "action": "read" }), &ctx).await.unwrap();
        assert_eq!(after_clear, "(clipboard is empty)");
    }

    #[tokio::test]
    async fn test_implicit_write_when_content_provided() {
        let tool = ClipboardTool::in_memory();
        let ctx = ToolContext::default();

        // When content is provided without action, it should default to "write"
        let res = tool
            .execute(json!({ "content": "Automatic write detection" }), &ctx)
            .await
            .unwrap();
        assert!(res.contains("Successfully copied"));

        let read = tool.read().await.unwrap();
        assert_eq!(read, "Automatic write detection");
    }

    #[tokio::test]
    async fn test_action_aliases_and_text_alias() {
        let tool = ClipboardTool::in_memory();
        let ctx = ToolContext::default();

        // Action "copy" with "text" property
        tool.execute(json!({ "action": "copy", "text": "Aliased copy" }), &ctx)
            .await
            .unwrap();
        assert_eq!(tool.read().await.unwrap(), "Aliased copy");

        // Action "paste"
        let pasted = tool.execute(json!({ "action": "paste" }), &ctx).await.unwrap();
        assert_eq!(pasted, "Aliased copy");

        // Action "set"
        tool.execute(json!({ "action": "set", "content": "Set value" }), &ctx)
            .await
            .unwrap();
        assert_eq!(tool.read().await.unwrap(), "Set value");

        // Action "get"
        let got = tool.execute(json!({ "action": "get" }), &ctx).await.unwrap();
        assert_eq!(got, "Set value");
    }

    #[tokio::test]
    async fn test_status_action() {
        let tool = ClipboardTool::in_memory();
        let ctx = ToolContext::default();

        tool.write("Status test string").await.unwrap();

        let status_res = tool.execute(json!({ "action": "status" }), &ctx).await.unwrap();
        let parsed: Value = serde_json::from_str(&status_res).unwrap();

        assert_eq!(parsed["backend"], "in_memory");
        assert_eq!(parsed["fallback_active"], true);
        assert_eq!(parsed["content_len_chars"], 18);
        assert_eq!(parsed["is_empty"], false);
    }

    #[tokio::test]
    async fn test_dedicated_read_and_write_tools() {
        let read_tool = ReadClipboardTool::in_memory();
        let write_tool = WriteClipboardTool::in_memory();
        let ctx = ToolContext::default();

        // ReadTool parameters & definition
        assert_eq!(read_tool.name(), "read_clipboard");
        assert!(read_tool.description().contains("Read"));

        // WriteTool parameters & definition
        assert_eq!(write_tool.name(), "write_clipboard");
        assert!(write_tool.description().contains("Write"));

        // Execute write
        let write_res = write_tool
            .execute(json!({ "content": "Dedicated tool test" }), &ctx)
            .await
            .unwrap();
        assert!(write_res.contains("Successfully copied"));

        // Execute read via dedicated tool
        let read_res = read_tool.execute(json!({}), &ctx).await.unwrap();
        assert_eq!(read_res, "Dedicated tool test");
    }

    #[tokio::test]
    async fn test_missing_content_error() {
        let tool = ClipboardTool::in_memory();
        let ctx = ToolContext::default();

        let err = tool.execute(json!({ "action": "write" }), &ctx).await;
        assert!(err.is_err());
        let err_str = err.unwrap_err().to_string();
        assert!(err_str.contains("Missing required parameter"));
    }

    #[tokio::test]
    async fn test_unknown_action_error() {
        let tool = ClipboardTool::in_memory();
        let ctx = ToolContext::default();

        let err = tool.execute(json!({ "action": "dance" }), &ctx).await;
        assert!(err.is_err());
        let err_str = err.unwrap_err().to_string();
        assert!(err_str.contains("Unknown clipboard action"));
    }

    #[tokio::test]
    async fn test_backend_kind_properties() {
        assert_eq!(ClipboardBackendKind::Termux.display_name(), "Termux (termux-clipboard-get/set)");
        assert_eq!(ClipboardBackendKind::MacOS.display_name(), "macOS (pbcopy/pbpaste)");
        assert_eq!(ClipboardBackendKind::Wayland.display_name(), "Wayland (wl-copy/wl-paste)");
        assert_eq!(ClipboardBackendKind::XClip.display_name(), "X11 (xclip)");
        assert_eq!(ClipboardBackendKind::XSel.display_name(), "X11 (xsel)");
        assert_eq!(ClipboardBackendKind::Windows.display_name(), "Windows (clip.exe / PowerShell)");
        assert_eq!(ClipboardBackendKind::InMemory.display_name(), "In-Memory Fallback");

        assert!(ClipboardBackendKind::Termux.is_system_backend());
        assert!(ClipboardBackendKind::MacOS.is_system_backend());
        assert!(ClipboardBackendKind::Wayland.is_system_backend());
        assert!(ClipboardBackendKind::XClip.is_system_backend());
        assert!(ClipboardBackendKind::XSel.is_system_backend());
        assert!(ClipboardBackendKind::Windows.is_system_backend());
        assert!(!ClipboardBackendKind::InMemory.is_system_backend());
    }

    #[tokio::test]
    async fn test_custom_command_manager() {
        // Echo test for custom command
        #[cfg(not(windows))]
        {
            let manager = ClipboardManager::with_custom_commands(
                ("sh".to_string(), vec!["-c".to_string(), "echo 'from custom'".to_string()]),
                ("sh".to_string(), vec!["-c".to_string(), "cat > /dev/null".to_string()]),
            );

            let tool = ClipboardTool::with_manager(manager);
            let text = tool.read().await.unwrap();
            assert_eq!(text.trim(), "from custom");
        }
    }
}
