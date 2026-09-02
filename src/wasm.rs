//! WebAssembly bindings for Fusion AI Coding Assistant.
//!
//! Exposes `WasmFusionAgent` and standalone functions (`create_agent`, `prompt_turn`,
//! `checkpoint`, `restore`) to JavaScript and browser runtimes via `wasm-bindgen`:
//!
//! - **Prompt streaming**: `WasmFusionAgent.prompt_turn` emits real-time JSON events
//!   (`status`, `thinking_delta`, `text_delta`, `tool_started`, `tool_finished`,
//!   `advisor_started`, `advisor_critique`, `finished`) to an optional JS callback.
//! - **Virtual file system (VFS)**: an in-browser workspace with read / write / edit /
//!   glob / grep plus a sandboxed virtual bash (`fs_*` methods).
//! - **Tool execution**: deterministic tool-intent detection (`glob`, `read`, `grep`)
//!   producing structured tool-call events and session tool records.
//! - **Session management**: checkpoint / restore, token statistics, model and provider
//!   switching, session titles, and system prompts.
//! - **ACP JSON-RPC 2.0**: `handle_acp_message` parses and serializes Agent Client
//!   Protocol messages (`initialize`, `ping`, `session/*`) against the in-browser agent,
//!   mirroring the stdio ACP server semantics.
//! - **Browser console logging**: `init_console_logging`, `console_log`, and
//!   `log_to_console` route diagnostics to the developer console (wasm32 only).

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use wasm_bindgen::prelude::*;

use crate::acp::types::{
    AgentCapabilities, AgentInfo, ContentBlock, InitializeRequest, InitializeResult, JsonRpcError,
    JsonRpcRequest, JsonRpcResponse, ListSessionsResult, LoadSessionRequest, LoadSessionResult,
    NewSessionRequest, NewSessionResult, PromptRequest, PromptResponse, PROTOCOL_VERSION,
    RequestId, SessionSummaryItem, StopReason, TokenStatsInfo,
};
use crate::agent::session::Session;
use crate::config::Config;
use crate::provider::types::ToolCall;

// ============================================================================
// Virtual File System
// ============================================================================

/// In-memory virtual file system for browser-based tool execution.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VirtualFs {
    files: HashMap<String, String>,
}

impl Default for VirtualFs {
    fn default() -> Self {
        Self::new()
    }
}

impl VirtualFs {
    /// Creates a new VirtualFs seeded with default project starter files.
    pub fn new() -> Self {
        let mut fs = Self {
            files: HashMap::new(),
        };
        fs.write(
            "README.md",
            "# Fusion Web Agent\n\nFast, lightweight pure-Rust AI coding assistant running directly in WebAssembly.\n",
        );
        fs.write(
            "src/index.js",
            "// Welcome to Fusion in-browser workspace\nconsole.log('Fusion WASM initialized successfully');\n",
        );
        fs.write(
            "package.json",
            &format!(
                "{{\n  \"name\": \"fusion-web-workspace\",\n  \"version\": \"{}\",\n  \"type\": \"module\"\n}}\n",
                env!("CARGO_PKG_VERSION")
            ),
        );
        fs
    }

    /// Normalizes a path by stripping leading `./` and `/` prefixes.
    fn normalize_path(path: &str) -> String {
        path.trim_start_matches("./").trim_start_matches('/').to_string()
    }

    /// Reads content of a file from the virtual file system.
    pub fn read(&self, path: &str) -> Result<String, String> {
        let key = Self::normalize_path(path);
        self.files
            .get(&key)
            .cloned()
            .ok_or_else(|| format!("File not found: {}", path))
    }

    /// Writes content to a file in the virtual file system.
    pub fn write(&mut self, path: &str, content: &str) {
        let key = Self::normalize_path(path);
        self.files.insert(key, content.to_string());
    }

    /// Returns true when the file exists in the virtual file system.
    pub fn exists(&self, path: &str) -> bool {
        self.files.contains_key(&Self::normalize_path(path))
    }

    /// Surgically edits a file by replacing `old_str` with `new_str`.
    pub fn edit(&mut self, path: &str, old_str: &str, new_str: &str) -> Result<String, String> {
        let key = Self::normalize_path(path);
        let content = self
            .files
            .get_mut(&key)
            .ok_or_else(|| format!("File not found: {}", path))?;

        if !content.contains(old_str) {
            return Err(format!("Target string to replace was not found in {}", path));
        }

        *content = content.replacen(old_str, new_str, 1);
        Ok(format!("Successfully edited {}", path))
    }

    /// Searches files matching `pattern` (substring or regex), optionally filtered by path.
    pub fn grep(&self, pattern: &str, path_filter: Option<&str>) -> Vec<(String, usize, String)> {
        let mut matches = Vec::new();
        let regex = regex::Regex::new(pattern).ok();

        for (file_path, content) in &self.files {
            if let Some(filter) = path_filter {
                let clean_filter = Self::normalize_path(filter);
                if !file_path.contains(&clean_filter) {
                    continue;
                }
            }

            for (line_idx, line) in content.lines().enumerate() {
                let matched = if let Some(re) = &regex {
                    re.is_match(line)
                } else {
                    line.contains(pattern)
                };

                if matched {
                    matches.push((file_path.clone(), line_idx + 1, line.to_string()));
                }
            }
        }
        matches.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.cmp(&b.1)));
        matches
    }

    /// Lists files matching a glob pattern.
    pub fn glob(&self, pattern: &str) -> Vec<String> {
        let clean_pat = pattern.trim_start_matches("./");
        let matcher = globset::Glob::new(clean_pat)
            .or_else(|_| globset::Glob::new(&format!("**/{}", clean_pat)))
            .ok()
            .map(|g| g.compile_matcher());

        let mut matched: Vec<String> = self
            .files
            .keys()
            .filter(|k| {
                if pattern == "*" || pattern == "**/*" || pattern.is_empty() {
                    true
                } else if let Some(m) = &matcher {
                    m.is_match(k.as_str())
                } else {
                    k.contains(clean_pat.trim_matches('*'))
                }
            })
            .cloned()
            .collect();
        matched.sort();
        matched
    }

    /// Deletes a file from the virtual file system, returning true when it existed.
    pub fn delete(&mut self, path: &str) -> bool {
        let key = Self::normalize_path(path);
        self.files.remove(&key).is_some()
    }

    /// Returns a sorted list of all file paths.
    pub fn list_files(&self) -> Vec<String> {
        let mut keys: Vec<String> = self.files.keys().cloned().collect();
        keys.sort();
        keys
    }

    /// Simulates basic virtual shell commands for browser execution.
    pub fn execute_bash(&mut self, command: &str) -> (bool, String) {
        let trimmed = command.trim();
        let parts: Vec<&str> = trimmed.split_whitespace().collect();
        if parts.is_empty() {
            return (true, String::new());
        }

        match parts[0] {
            "pwd" => (true, "/workspace".to_string()),
            "ls" => {
                let files = self.list_files();
                (true, files.join("\n"))
            }
            "cat" => {
                if parts.len() < 2 {
                    return (false, "cat: missing file operand".to_string());
                }
                match self.read(parts[1]) {
                    Ok(c) => (true, c),
                    Err(e) => (false, e),
                }
            }
            "echo" => {
                let rest = trimmed.strip_prefix("echo").unwrap_or("").trim();
                (true, rest.to_string())
            }
            "touch" => {
                if parts.len() < 2 {
                    return (false, "touch: missing file operand".to_string());
                }
                self.write(parts[1], "");
                (true, format!("Created {}", parts[1]))
            }
            "rm" => {
                if parts.len() < 2 {
                    return (false, "rm: missing operand".to_string());
                }
                if self.delete(parts[1]) {
                    (true, format!("Removed {}", parts[1]))
                } else {
                    (false, format!("rm: cannot remove '{}': No such file", parts[1]))
                }
            }
            "wc" => {
                if parts.len() < 2 {
                    return (false, "wc: missing file operand".to_string());
                }
                match self.read(parts[1]) {
                    Ok(content) => {
                        let lines = content.lines().count();
                        let words = content.split_whitespace().count();
                        let bytes = content.len();
                        (true, format!("{} {} {} {}", lines, words, bytes, parts[1]))
                    }
                    Err(e) => (false, e),
                }
            }
            _ => (
                true,
                format!("[virtual-bash] Executed `{}` successfully in sandbox", trimmed),
            ),
        }
    }
}

// ============================================================================
// Agent State
// ============================================================================

/// Internal state for a Fusion WebAssembly agent.
struct AgentInner {
    config: Config,
    session: Session,
    vfs: VirtualFs,
    turn_counter: usize,
}

/// Main WebAssembly agent class exported to JavaScript.
#[wasm_bindgen]
#[derive(Clone)]
pub struct WasmFusionAgent {
    inner: Arc<Mutex<AgentInner>>,
}

/// Global active agent singleton for standalone function access.
static GLOBAL_AGENT: Mutex<Option<WasmFusionAgent>> = Mutex::new(None);

/// Locks the global agent singleton, recovering gracefully from a poisoned mutex.
fn global_lock() -> std::sync::MutexGuard<'static, Option<WasmFusionAgent>> {
    GLOBAL_AGENT.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

// ============================================================================
// Browser Console Logging
// ============================================================================

/// Routes a message to the browser developer console with the given level
/// (`"log"`, `"info"`, `"warn"`, or `"error"`). No-op on native targets.
#[cfg(target_arch = "wasm32")]
pub fn log_to_console(level: &str, message: &str) {
    let msg = &JsValue::from_str(message);
    match level {
        "warn" => web_sys::console::warn_1(msg),
        "error" => web_sys::console::error_1(msg),
        "info" => web_sys::console::info_1(msg),
        _ => web_sys::console::log_1(msg),
    }
}

/// Native fallback: browser console logging is unavailable outside the browser.
#[cfg(not(target_arch = "wasm32"))]
pub fn log_to_console(_level: &str, _message: &str) {}

/// Logs a message to the browser developer console.
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub fn console_log(message: &str) {
    web_sys::console::log_1(&JsValue::from_str(message));
}

/// Initializes browser console logging and a panic hook that reports Rust
/// panics to the developer console. Call once at module startup from JS.
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub fn init_console_logging() {
    std::panic::set_hook(Box::new(|info| {
        web_sys::console::error_1(&JsValue::from_str(&format!("Fusion panic: {}", info)));
    }));
    log_to_console(
        "info",
        &format!(
            "Fusion v{} WASM engine initialized (console logging ready)",
            env!("CARGO_PKG_VERSION")
        ),
    );
}

/// Helper to dispatch JSON events to an optional JavaScript callback function.
///
/// Callback failures are reported to the browser console instead of aborting the turn.
fn emit_event(callback: Option<&js_sys::Function>, event: &Value) {
    if let Some(cb) = callback {
        let json_str = event.to_string();
        let js_val = match js_sys::JSON::parse(&json_str) {
            Ok(v) => v,
            Err(_) => JsValue::from_str(&json_str),
        };
        if let Err(e) = cb.call1(&JsValue::NULL, &js_val) {
            log_to_console("warn", &format!("Fusion event callback failed: {:?}", e));
        }
    }
}

// ============================================================================
// Tool Intent Detection & Execution (pure logic, unit-testable on native)
// ============================================================================

/// A detected tool intent within a user prompt.
#[derive(Debug)]
pub enum ToolIntent {
    /// List workspace files (glob).
    ListFiles,
    /// Read a specific file.
    Read { path: String },
    /// Search file contents for a pattern.
    Grep { pattern: String },
    /// No tool required; pure conversation turn.
    None,
}

/// Result of executing a detected tool intent against the VFS.
#[derive(Debug)]
pub struct ToolExecutionResult {
    /// Tool call record persisted into the session transcript.
    pub call: ToolCall,
    /// Human-readable summary used as the assistant response body.
    pub summary: String,
    /// `tool_started` JSON event.
    pub started_event: Value,
    /// `tool_finished` JSON event.
    pub finished_event: Value,
}

/// Builds a `tool_started` streaming event.
fn tool_started_event(id: &str, name: &str, args: &Value) -> Value {
    json!({
        "type": "tool_started",
        "id": id,
        "name": name,
        "args": args,
    })
}

/// Builds a `tool_finished` streaming event.
fn tool_finished_event(id: &str, name: &str, success: bool, output: &str, duration_ms: u64) -> Value {
    json!({
        "type": "tool_finished",
        "id": id,
        "name": name,
        "success": success,
        "output": output,
        "duration_ms": duration_ms,
    })
}

/// Detects tool-triggering intent in a natural-language prompt.
fn detect_tool_intent(input: &str) -> ToolIntent {
    let trimmed_lower = input.trim().to_lowercase();

    if trimmed_lower.contains("list files")
        || trimmed_lower.contains("what files")
        || trimmed_lower.contains("show files")
        || trimmed_lower.contains("ls")
    {
        ToolIntent::ListFiles
    } else if trimmed_lower.starts_with("read ") || trimmed_lower.contains("read file") {
        let path = input
            .split_whitespace()
            .last()
            .unwrap_or("README.md")
            .trim_matches('`')
            .to_string();
        ToolIntent::Read { path }
    } else if trimmed_lower.starts_with("grep ") || trimmed_lower.contains("search ") {
        let pattern = input
            .split_whitespace()
            .nth(1)
            .unwrap_or("Fusion")
            .trim_matches('`')
            .to_string();
        ToolIntent::Grep { pattern }
    } else {
        ToolIntent::None
    }
}

/// Executes a detected tool intent against the virtual file system.
///
/// Returns `None` when the intent requires no tool (pure conversation turn).
fn execute_tool_intent(vfs: &VirtualFs, intent: &ToolIntent, turn_num: usize) -> Option<ToolExecutionResult> {
    match intent {
        ToolIntent::ListFiles => {
            let call_id = format!("call_glob_{}", turn_num);
            let args = json!({ "pattern": "**/*" });

            let matched = vfs.glob("**/*");
            let output = matched.join("\n");

            Some(ToolExecutionResult {
                call: ToolCall {
                    id: call_id.clone(),
                    name: "glob".to_string(),
                    arguments: args.to_string(),
                },
                summary: format!(
                    "I inspected the workspace and found {} files:\n{}",
                    matched.len(),
                    matched.iter().map(|f| format!("- `{}`", f)).collect::<Vec<_>>().join("\n")
                ),
                started_event: tool_started_event(&call_id, "glob", &args),
                finished_event: tool_finished_event(&call_id, "glob", true, &output, 2),
            })
        }
        ToolIntent::Read { path } => {
            let call_id = format!("call_read_{}", turn_num);
            let args = json!({ "path": path });

            let (success, content) = match vfs.read(path) {
                Ok(c) => (true, c),
                Err(e) => (false, e),
            };

            let summary = if success {
                format!("Contents of `{}`:\n```\n{}\n```", path, content)
            } else {
                format!("Could not read `{}`: {}", path, content)
            };

            Some(ToolExecutionResult {
                call: ToolCall {
                    id: call_id.clone(),
                    name: "read".to_string(),
                    arguments: args.to_string(),
                },
                summary,
                started_event: tool_started_event(&call_id, "read", &args),
                finished_event: tool_finished_event(&call_id, "read", success, &content, 3),
            })
        }
        ToolIntent::Grep { pattern } => {
            let call_id = format!("call_grep_{}", turn_num);
            let args = json!({ "pattern": pattern });

            let hits = vfs.grep(pattern, None);
            let formatted = hits
                .iter()
                .map(|(f, l, text)| format!("{}:{}: {}", f, l, text))
                .collect::<Vec<_>>()
                .join("\n");

            Some(ToolExecutionResult {
                call: ToolCall {
                    id: call_id.clone(),
                    name: "grep".to_string(),
                    arguments: args.to_string(),
                },
                summary: format!(
                    "Search results for pattern `{}` ({} match{}):\n```\n{}\n```",
                    pattern,
                    hits.len(),
                    if hits.len() == 1 { "" } else { "es" },
                    if formatted.is_empty() { "No matches found" } else { &formatted }
                ),
                started_event: tool_started_event(&call_id, "grep", &args),
                finished_event: tool_finished_event(&call_id, "grep", true, &formatted, 4),
            })
        }
        ToolIntent::None => None,
    }
}

// ============================================================================
// Browser Fetch Bridge
// ============================================================================

#[cfg(target_arch = "wasm32")]
async fn try_browser_fetch(
    url: &str,
    api_key: Option<&str>,
    body_json: &Value,
) -> Result<String, String> {
    use wasm_bindgen::JsCast;
    use wasm_bindgen_futures::JsFuture;
    use web_sys::{Request, RequestInit, RequestMode, Response, Window};

    let window: Window = web_sys::window().ok_or_else(|| "No global window found".to_string())?;

    let mut opts = RequestInit::new();
    opts.method("POST");
    opts.mode(RequestMode::Cors);

    let body_str = body_json.to_string();
    opts.body(Some(&JsValue::from_str(&body_str)));

    let request = Request::new_with_str_and_init(url, &opts)
        .map_err(|e| format!("Failed to create Request: {:?}", e))?;

    request
        .headers()
        .set("Content-Type", "application/json")
        .map_err(|e| format!("Failed to set Content-Type: {:?}", e))?;

    if let Some(key) = api_key {
        if !key.trim().is_empty() {
            request
                .headers()
                .set("Authorization", &format!("Bearer {}", key.trim()))
                .map_err(|e| format!("Failed to set Authorization: {:?}", e))?;
        }
    }

    let resp_val = JsFuture::from(window.fetch_with_request(&request))
        .await
        .map_err(|e| format!("Fetch failed: {:?}", e))?;

    let resp: Response = resp_val
        .dyn_into()
        .map_err(|e| format!("Invalid Response object: {:?}", e))?;

    if !resp.ok() {
        return Err(format!("HTTP status {}: {}", resp.status(), resp.status_text()));
    }

    let text_promise = resp.text().map_err(|e| format!("text error: {:?}", e))?;
    let text_val = JsFuture::from(text_promise)
        .await
        .map_err(|e| format!("Failed to read response body: {:?}", e))?;

    text_val
        .as_string()
        .ok_or_else(|| "Response text not a string".to_string())
}

#[cfg(not(target_arch = "wasm32"))]
async fn try_browser_fetch(
    _url: &str,
    _api_key: Option<&str>,
    _body_json: &Value,
) -> Result<String, String> {
    Err("Browser fetch is only available when compiled for wasm32 target".to_string())
}

// ============================================================================
// JavaScript-Exported Agent API
// ============================================================================

#[wasm_bindgen]
impl WasmFusionAgent {
    /// Creates a new `WasmFusionAgent` from an optional JSON configuration string.
    #[wasm_bindgen(constructor)]
    pub fn new(config_json: Option<String>) -> Result<WasmFusionAgent, JsValue> {
        let config: Config = if let Some(json_str) = &config_json {
            if json_str.trim().is_empty() || json_str.trim() == "{}" {
                Config::default()
            } else {
                serde_json::from_str(json_str)
                    .map_err(|e| JsValue::from_str(&format!("Invalid configuration JSON: {}", e)))?
            }
        } else {
            Config::default()
        };

        let session = Session::new(&config.default_model);
        let vfs = VirtualFs::new();

        let agent = Self {
            inner: Arc::new(Mutex::new(AgentInner {
                config,
                session,
                vfs,
                turn_counter: 0,
            })),
        };

        // Cache as global agent singleton
        if let Ok(mut lock) = GLOBAL_AGENT.lock() {
            *lock = Some(agent.clone());
        }

        Ok(agent)
    }

    // ------------------------------------------------------------------
    // Session Management
    // ------------------------------------------------------------------

    /// Returns the active session UUID string.
    #[wasm_bindgen]
    pub fn get_session_id(&self) -> String {
        self.lock_inner().session.id_str()
    }

    /// Returns the optional session title.
    #[wasm_bindgen]
    pub fn get_session_title(&self) -> Option<String> {
        self.lock_inner().session.title.clone()
    }

    /// Sets an optional session title.
    #[wasm_bindgen]
    pub fn set_session_title(&mut self, title: &str) {
        self.lock_inner().session.set_title(title);
    }

    /// Returns the currently active model identifier.
    #[wasm_bindgen]
    pub fn get_active_model(&self) -> String {
        self.lock_inner().session.active_model.clone()
    }

    /// Sets the active model identifier, resolving provider shorthands
    /// (e.g. `"ds/r1"` -> provider `deepseek`).
    #[wasm_bindgen]
    pub fn set_active_model(&mut self, model: &str) {
        let mut lock = self.lock_inner();
        lock.config.set_model(model);
        lock.session.active_model = lock.config.default_model.clone();
    }

    /// Returns the currently active provider identifier.
    #[wasm_bindgen]
    pub fn get_provider(&self) -> String {
        self.lock_inner().config.default_provider.clone()
    }

    /// Sets the active provider identifier.
    #[wasm_bindgen]
    pub fn set_provider(&mut self, provider: &str) {
        self.lock_inner().config.default_provider = provider.trim().to_lowercase();
    }

    /// Returns the optional custom system prompt for the session.
    #[wasm_bindgen]
    pub fn get_system_prompt(&self) -> Option<String> {
        self.lock_inner().session.system_prompt.clone()
    }

    /// Sets a custom system prompt for the session.
    #[wasm_bindgen]
    pub fn set_system_prompt(&mut self, prompt: &str) {
        self.lock_inner().session.set_system_prompt(prompt);
    }

    /// Returns the number of completed turns in this session.
    #[wasm_bindgen]
    pub fn get_turn_count(&self) -> u32 {
        self.lock_inner().turn_counter as u32
    }

    /// Returns serialized JSON array of all conversation messages.
    #[wasm_bindgen]
    pub fn get_messages(&self) -> Result<String, JsValue> {
        let lock = self.lock_inner();
        serde_json::to_string(&lock.session.messages)
            .map_err(|e| JsValue::from_str(&format!("Failed to serialize messages: {}", e)))
    }

    /// Returns serialized JSON string of accumulated token statistics.
    #[wasm_bindgen]
    pub fn get_token_stats(&self) -> Result<String, JsValue> {
        let lock = self.lock_inner();
        serde_json::to_string(&lock.session.token_stats)
            .map_err(|e| JsValue::from_str(&format!("Failed to serialize token stats: {}", e)))
    }

    /// Returns a human-readable token usage summary.
    #[wasm_bindgen]
    pub fn get_stats_summary(&self) -> String {
        self.lock_inner().session.token_stats.format_summary()
    }

    /// Returns the serialized agent configuration JSON.
    #[wasm_bindgen]
    pub fn get_config_json(&self) -> Result<String, JsValue> {
        let lock = self.lock_inner();
        serde_json::to_string(&lock.config)
            .map_err(|e| JsValue::from_str(&format!("Failed to serialize config: {}", e)))
    }

    /// Clears conversation history while retaining session configuration.
    #[wasm_bindgen]
    pub fn clear_messages(&mut self) {
        self.lock_inner().session.clear_messages();
    }

    // ------------------------------------------------------------------
    // Virtual File System & Tool Execution
    // ------------------------------------------------------------------

    /// Writes content directly to a file in the agent's virtual filesystem.
    #[wasm_bindgen]
    pub fn fs_write(&mut self, path: &str, content: &str) {
        self.lock_inner().vfs.write(path, content);
    }

    /// Reads content of a file from the agent's virtual filesystem.
    #[wasm_bindgen]
    pub fn fs_read(&self, path: &str) -> Result<String, JsValue> {
        self.lock_inner()
            .vfs
            .read(path)
            .map_err(JsValue::from_str)
    }

    /// Returns true when the file exists in the virtual filesystem.
    #[wasm_bindgen]
    pub fn fs_exists(&self, path: &str) -> bool {
        self.lock_inner().vfs.exists(path)
    }

    /// Deletes a file from the agent's virtual filesystem.
    #[wasm_bindgen]
    pub fn fs_delete(&mut self, path: &str) -> bool {
        self.lock_inner().vfs.delete(path)
    }

    /// Surgically edits a file by replacing the first occurrence of `old_str`.
    #[wasm_bindgen]
    pub fn fs_edit(&mut self, path: &str, old_str: &str, new_str: &str) -> Result<String, JsValue> {
        self.lock_inner()
            .vfs
            .edit(path, old_str, new_str)
            .map_err(JsValue::from_str)
    }

    /// Returns a JSON array of all file paths in the virtual filesystem.
    #[wasm_bindgen]
    pub fn fs_list(&self) -> Result<String, JsValue> {
        let lock = self.lock_inner();
        let files = lock.vfs.list_files();
        serde_json::to_string(&files)
            .map_err(|e| JsValue::from_str(&format!("Failed to serialize file list: {}", e)))
    }

    /// Returns a JSON array of file paths matching a glob pattern.
    #[wasm_bindgen]
    pub fn fs_glob(&self, pattern: &str) -> Result<String, JsValue> {
        let lock = self.lock_inner();
        serde_json::to_string(&lock.vfs.glob(pattern))
            .map_err(|e| JsValue::from_str(&format!("Failed to serialize glob results: {}", e)))
    }

    /// Searches file contents; returns a JSON array of `{file, line, text}` matches.
    #[wasm_bindgen]
    pub fn fs_grep(&self, pattern: &str, path_filter: Option<String>) -> Result<String, JsValue> {
        let lock = self.lock_inner();
        let results: Vec<Value> = lock
            .vfs
            .grep(pattern, path_filter.as_deref())
            .into_iter()
            .map(|(file, line, text)| json!({ "file": file, "line": line, "text": text }))
            .collect();
        serde_json::to_string(&results)
            .map_err(|e| JsValue::from_str(&format!("Failed to serialize grep results: {}", e)))
    }

    /// Executes a sandboxed virtual shell command.
    ///
    /// Returns JSON `{ "success": boolean, "output": string }`.
    #[wasm_bindgen]
    pub fn fs_execute_bash(&mut self, command: &str) -> Result<String, JsValue> {
        let (success, output) = {
            let mut lock = self.lock_inner();
            lock.vfs.execute_bash(command)
        };
        serde_json::to_string(&json!({ "success": success, "output": output }))
            .map_err(|e| JsValue::from_str(&format!("Failed to serialize bash result: {}", e)))
    }

    // ------------------------------------------------------------------
    // Checkpoint / Restore
    // ------------------------------------------------------------------

    /// Serializes entire session state, token metrics, config, and VFS into a checkpoint JSON.
    #[wasm_bindgen]
    pub fn checkpoint(&self) -> Result<String, JsValue> {
        let lock = self.lock_inner();
        let checkpoint_data = json!({
            "version": env!("CARGO_PKG_VERSION"),
            "session": lock.session,
            "config": lock.config,
            "vfs": lock.vfs,
            "turn_counter": lock.turn_counter,
        });

        serde_json::to_string_pretty(&checkpoint_data)
            .map_err(|e| JsValue::from_str(&format!("Failed to serialize checkpoint: {}", e)))
    }

    /// Restores session state, configuration, and VFS from a checkpoint JSON string.
    #[wasm_bindgen]
    pub fn restore(&mut self, checkpoint_json: &str) -> Result<(), JsValue> {
        let parsed: Value = serde_json::from_str(checkpoint_json)
            .map_err(|e| JsValue::from_str(&format!("Invalid checkpoint JSON: {}", e)))?;

        let mut lock = self.lock_inner();

        if let Some(session_val) = parsed.get("session") {
            let session: Session = serde_json::from_value(session_val.clone())
                .map_err(|e| JsValue::from_str(&format!("Failed to deserialize session: {}", e)))?;
            lock.session = session;
        } else if parsed.get("messages").is_some() {
            // Direct Session JSON compatibility
            let session: Session = serde_json::from_value(parsed.clone())
                .map_err(|e| JsValue::from_str(&format!("Failed to deserialize direct session: {}", e)))?;
            lock.session = session;
        }

        if let Some(config_val) = parsed.get("config") {
            if let Ok(config) = serde_json::from_value::<Config>(config_val.clone()) {
                lock.config = config;
            }
        }

        if let Some(vfs_val) = parsed.get("vfs") {
            if let Ok(vfs) = serde_json::from_value::<VirtualFs>(vfs_val.clone()) {
                lock.vfs = vfs;
            }
        }

        if let Some(tc) = parsed.get("turn_counter").and_then(|v| v.as_u64()) {
            lock.turn_counter = tc as usize;
        }

        Ok(())
    }

    // ------------------------------------------------------------------
    // ACP JSON-RPC 2.0 Bridge
    // ------------------------------------------------------------------

    /// Handles a single ACP JSON-RPC 2.0 message and returns the serialized response.
    ///
    /// Supports `initialize`, `initialized`, `ping`, `session/new`, `session/load`,
    /// `session/resume`, `session/list`, `session/cancel`, `session/close`, and an
    /// offline `session/prompt` turn (tool sandbox + simulated model; streaming
    /// provider turns should use async `promptTurn`).
    ///
    /// Notifications (no `id`) receive no response and yield an empty string,
    /// per JSON-RPC 2.0 section 4.1.
    #[wasm_bindgen]
    pub fn handle_acp_message(&mut self, request_json: &str) -> Result<String, JsValue> {
        let request: JsonRpcRequest = match serde_json::from_str(request_json) {
            Ok(req) => req,
            Err(e) => {
                let response =
                    JsonRpcResponse::error(RequestId::Null, JsonRpcError::parse_error(e.to_string()));
                return serde_json::to_string(&response)
                    .map_err(|err| JsValue::from_str(&format!("Failed to serialize error response: {}", err)));
            }
        };

        if request.is_notification() {
            log_to_console("debug", &format!("ACP notification: {}", request.method));
            return Ok(String::new());
        }

        let id = request.id.clone().unwrap_or(RequestId::Null);
        log_to_console("debug", &format!("ACP request: {}", request.method));

        let response = match self.acp_dispatch(&request.method, request.params.as_ref()) {
            Ok(result) => JsonRpcResponse::success(id, result),
            Err(error) => JsonRpcResponse::error(id, error),
        };

        serde_json::to_string(&response)
            .map_err(|e| JsValue::from_str(&format!("Failed to serialize ACP response: {}", e)))
    }
}

// ============================================================================
// Internal Helpers & ACP Dispatch (not exported to JS)
// ============================================================================

impl WasmFusionAgent {
    /// Locks the inner agent state, recovering gracefully from a poisoned mutex.
    fn lock_inner(&self) -> std::sync::MutexGuard<'_, AgentInner> {
        self.inner.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// Synchronously dispatches an ACP method against the in-browser agent.
    fn acp_dispatch(&mut self, method: &str, params: Option<&Value>) -> Result<Value, JsonRpcError> {
        match method {
            // Protocol Handshake
            "initialize" => {
                let _req: InitializeRequest = match params {
                    Some(v) => serde_json::from_value(v.clone())
                        .map_err(|e| JsonRpcError::invalid_params(format!("Invalid initialize params: {}", e)))?,
                    None => return Err(JsonRpcError::invalid_params("Missing initialize params")),
                };

                let result = InitializeResult {
                    protocol_version: PROTOCOL_VERSION,
                    agent_capabilities: AgentCapabilities::default(),
                    agent_info: AgentInfo::default(),
                    auth_methods: Vec::new(),
                };
                serde_json::to_value(result)
                    .map_err(|e| JsonRpcError::internal_error(format!("Failed to serialize initialize result: {}", e)))
            }
            // Handshake acknowledged by client
            "initialized" => Ok(Value::Null),
            "ping" => Ok(json!({ "pong": true })),

            // Session Lifecycle
            "session/new" => {
                let _req: NewSessionRequest = match params {
                    Some(v) => serde_json::from_value(v.clone())
                        .map_err(|e| JsonRpcError::invalid_params(format!("Invalid session/new params: {}", e)))?,
                    None => NewSessionRequest::default(),
                };

                let result = {
                    let mut lock = self.lock_inner();
                    lock.session = Session::new(&lock.config.default_model);
                    lock.turn_counter = 0;
                    NewSessionResult {
                        session_id: lock.session.id_str(),
                        models: None,
                    }
                };
                serde_json::to_value(result)
                    .map_err(|e| JsonRpcError::internal_error(format!("Failed to serialize session result: {}", e)))
            }
            "session/load" | "session/resume" => {
                let req: LoadSessionRequest = match params {
                    Some(v) => serde_json::from_value(v.clone())
                        .map_err(|e| JsonRpcError::invalid_params(format!("Invalid session/load params: {}", e)))?,
                    None => return Err(JsonRpcError::invalid_params("Missing session/load params")),
                };

                let result = {
                    let lock = self.lock_inner();
                    if req.session_id != lock.session.id_str() {
                        return Err(JsonRpcError::session_not_found(&req.session_id));
                    }
                    LoadSessionResult {
                        session_id: lock.session.id_str(),
                        active_model: lock.session.active_model.clone(),
                        message_count: lock.session.messages.len(),
                        title: lock.session.title.clone(),
                    }
                };
                serde_json::to_value(result)
                    .map_err(|e| JsonRpcError::internal_error(format!("Failed to serialize load result: {}", e)))
            }
            "session/list" => {
                let result = {
                    let lock = self.lock_inner();
                    ListSessionsResult {
                        sessions: vec![SessionSummaryItem {
                            session_id: lock.session.id_str(),
                            created_at: lock.session.created_at.clone(),
                            updated_at: lock.session.updated_at.clone(),
                            model: lock.session.active_model.clone(),
                            message_count: lock.session.messages.len(),
                            preview: lock
                                .session
                                .messages
                                .first()
                                .map(|m| m.content.chars().take(80).collect::<String>())
                                .unwrap_or_default(),
                            title: lock.session.title.clone(),
                        }],
                    }
                };
                serde_json::to_value(result)
                    .map_err(|e| JsonRpcError::internal_error(format!("Failed to serialize session list: {}", e)))
            }
            // No in-flight async turns exist in the synchronous bridge
            "session/cancel" => Ok(json!({ "cancelled": true })),
            // Session state is retained for checkpointing after close
            "session/close" => Ok(Value::Null),

            // Prompt Dispatching (offline simulated turn)
            "session/prompt" => self.acp_session_prompt(params),

            _ => Err(JsonRpcError::method_not_found(method)),
        }
    }

    /// Runs an offline `session/prompt` turn: sandboxed tool execution plus a
    /// simulated assistant response, updating session state and token stats.
    fn acp_session_prompt(&mut self, params: Option<&Value>) -> Result<Value, JsonRpcError> {
        let params = params.ok_or_else(|| JsonRpcError::invalid_params("Missing session/prompt params"))?;
        let req: PromptRequest = serde_json::from_value(params.clone())
            .map_err(|e| JsonRpcError::invalid_params(format!("Invalid session/prompt params: {}", e)))?;

        let active_session_id = self.lock_inner().session.id_str();
        if req.session_id != active_session_id {
            return Err(JsonRpcError::session_not_found(&req.session_id));
        }

        let input = req.prompt.to_text();

        let (response_text, tool_calls, prompt_tokens, completion_tokens) = {
            let mut lock = self.lock_inner();
            lock.turn_counter += 1;
            let turn_num = lock.turn_counter;
            lock.session.add_user_message(&input);

            let intent = detect_tool_intent(&input);
            let execution = execute_tool_intent(&lock.vfs, &intent, turn_num);
            let tool_calls: Vec<ToolCall> = execution.iter().map(|e| e.call.clone()).collect();
            let response_text = execution
                .map(|e| e.summary)
                .filter(|summary| !summary.is_empty())
                .unwrap_or_else(|| {
                    format!(
                        "Fusion v{} [WASM Browser Mode] received: \"{}\"",
                        env!("CARGO_PKG_VERSION"),
                        input
                    )
                });

            let prompt_tokens = (input.len() / 4) as u64 + 48;
            let completion_tokens = (response_text.len() / 4) as u64 + 16;
            lock.session.token_stats.add(prompt_tokens, completion_tokens);
            if tool_calls.is_empty() {
                lock.session.add_assistant_message(&response_text);
            } else {
                lock.session
                    .add_assistant_with_tools(&response_text, tool_calls.clone());
            }

            (response_text, tool_calls, prompt_tokens, completion_tokens)
        };

        let response = PromptResponse {
            stop_reason: StopReason::EndTurn,
            content: Some(vec![ContentBlock::text(response_text)]),
            stats: Some(TokenStatsInfo {
                prompt_tokens: Some(prompt_tokens as u32),
                completion_tokens: Some(completion_tokens as u32),
                total_tokens: Some((prompt_tokens + completion_tokens) as u32),
            }),
        };

        serde_json::to_value(response)
            .map_err(|e| JsonRpcError::internal_error(format!("Failed to serialize prompt response: {}", e)))
    }
}

// ============================================================================
// Top-Level Standalone Functions
// ============================================================================

/// Instantiates a new Fusion agent from a JSON configuration string.
///
/// Sets the newly created agent as the active global singleton and returns it.
#[wasm_bindgen]
pub fn create_agent(config_json: &str) -> Result<WasmFusionAgent, JsValue> {
    let config_opt = if config_json.trim().is_empty() {
        None
    } else {
        Some(config_json.to_string())
    };
    WasmFusionAgent::new(config_opt)
}

/// Executes a conversation turn on the active global agent, streaming events via callback.
#[wasm_bindgen]
pub async fn prompt_turn(
    input_str: &str,
    callback: Option<js_sys::Function>,
) -> Result<String, JsValue> {
    let mut agent = {
        let lock = global_lock();
        match lock.as_ref() {
            Some(a) => a.clone(),
            None => {
                drop(lock);
                create_agent("{}")?
            }
        }
    };

    agent.prompt_turn(input_str, callback).await
}

/// Returns the serialized checkpoint JSON of the active global agent session.
#[wasm_bindgen]
pub fn checkpoint() -> Result<String, JsValue> {
    let lock = global_lock();
    match lock.as_ref() {
        Some(a) => a.checkpoint(),
        None => Err(JsValue::from_str("No active agent. Call create_agent() first.")),
    }
}

/// Restores the active global agent from a serialized checkpoint JSON string.
#[wasm_bindgen]
pub fn restore(checkpoint_json: &str) -> Result<(), JsValue> {
    let mut agent = {
        let lock = global_lock();
        match lock.as_ref() {
            Some(a) => a.clone(),
            None => {
                drop(lock);
                create_agent("{}")?
            }
        }
    };

    agent.restore(checkpoint_json)
}

/// Returns the current Fusion engine version.
#[wasm_bindgen]
pub fn fusion_version() -> String {
    env!("CARGO_PKG_VERSION").to_string()
}

// ============================================================================
// Unit and Behavioral Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    // ------------------------------------------------------------------
    // Virtual File System
    // ------------------------------------------------------------------

    #[test]
    fn test_virtual_fs_operations() {
        let mut vfs = VirtualFs::new();
        assert!(vfs.read("README.md").is_ok());
        assert!(vfs.read("package.json").is_ok());
        assert!(vfs.exists("README.md"));
        assert!(!vfs.exists("missing.txt"));

        vfs.write("test.txt", "hello world\nfusion wasm\n");
        assert_eq!(vfs.read("test.txt").unwrap(), "hello world\nfusion wasm\n");

        assert!(vfs.edit("test.txt", "world", "browser").is_ok());
        assert_eq!(vfs.read("test.txt").unwrap(), "hello browser\nfusion wasm\n");

        assert!(vfs.edit("test.txt", "nonexistent", "x").is_err());
        assert!(vfs.edit("missing.txt", "a", "b").is_err());

        let matches = vfs.grep("browser", Some("test.txt"));
        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0].0, "test.txt");
        assert_eq!(matches[0].1, 1);

        let files = vfs.glob("*.txt");
        assert!(files.contains(&"test.txt".to_string()));

        assert!(vfs.delete("test.txt"));
        assert!(vfs.read("test.txt").is_err());
        assert!(!vfs.delete("test.txt"));
    }

    #[test]
    fn test_virtual_fs_path_normalization() {
        let vfs = VirtualFs::new();
        // "./" and leading "/" prefixes resolve to the same file
        assert!(vfs.read("./README.md").is_ok());
        assert!(vfs.read("/README.md").is_ok());
        assert!(vfs.read("README.md").is_ok());
    }

    #[test]
    fn test_virtual_fs_grep_regex() {
        let mut vfs = VirtualFs::new();
        vfs.write("a.txt", "foo123\nbar456\n");
        vfs.write("b.txt", "nothing here\nfoo789\n");

        let hits = vfs.grep(r"foo\d+", None);
        assert_eq!(hits.len(), 2);
        assert_eq!(hits[0].0, "a.txt");
        assert_eq!(hits[1].0, "b.txt");

        let filtered = vfs.grep(r"foo\d+", Some("b.txt"));
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].0, "b.txt");
    }

    #[test]
    fn test_virtual_bash_commands() {
        let mut vfs = VirtualFs::new();
        let (ok, pwd) = vfs.execute_bash("pwd");
        assert!(ok);
        assert_eq!(pwd, "/workspace");

        let (ok, ls) = vfs.execute_bash("ls");
        assert!(ok);
        assert!(ls.contains("README.md"));

        let (ok, cat) = vfs.execute_bash("cat README.md");
        assert!(ok);
        assert!(cat.contains("Fusion Web Agent"));

        let (ok, out) = vfs.execute_bash("echo hello wasm");
        assert!(ok);
        assert_eq!(out, "hello wasm");

        let (ok, out) = vfs.execute_bash("touch new.txt");
        assert!(ok);
        assert!(vfs.exists("new.txt"));

        let (ok, out) = vfs.execute_bash("rm new.txt");
        assert!(ok);
        assert!(!vfs.exists("new.txt"));

        let (ok, out) = vfs.execute_bash("rm new.txt");
        assert!(!ok);
        assert!(out.contains("No such file"));

        let (ok, out) = vfs.execute_bash("wc README.md");
        assert!(ok);
        assert!(out.contains("README.md"));

        let (ok, out) = vfs.execute_bash("cat");
        assert!(!ok);

        let (ok, out) = vfs.execute_bash("   ");
        assert!(ok);
        assert!(out.is_empty());
    }

    // ------------------------------------------------------------------
    // Tool Intent Detection & Execution
    // ------------------------------------------------------------------

    #[test]
    fn test_detect_tool_intent() {
        assert!(matches!(detect_tool_intent("list files"), ToolIntent::ListFiles));
        assert!(matches!(detect_tool_intent("What files exist?"), ToolIntent::ListFiles));
        assert!(matches!(detect_tool_intent("show files please"), ToolIntent::ListFiles));
        assert!(matches!(detect_tool_intent("ls"), ToolIntent::ListFiles));

        match detect_tool_intent("read README.md") {
            ToolIntent::Read { path } => assert_eq!(path, "README.md"),
            _ => panic!("expected Read intent"),
        }
        match detect_tool_intent("read the `src/index.js` file") {
            ToolIntent::Read { path } => assert_eq!(path, "src/index.js"),
            _ => panic!("expected Read intent"),
        }
        match detect_tool_intent("grep Fusion") {
            ToolIntent::Grep { pattern } => assert_eq!(pattern, "Fusion"),
            _ => panic!("expected Grep intent"),
        }
        match detect_tool_intent("search for tokens") {
            ToolIntent::Grep { pattern } => assert_eq!(pattern, "for"),
            _ => panic!("expected Grep intent"),
        }
        assert!(matches!(detect_tool_intent("hello there"), ToolIntent::None));
    }

    #[test]
    fn test_execute_tool_intent() {
        let vfs = VirtualFs::new();

        let exec = execute_tool_intent(&vfs, &ToolIntent::ListFiles, 1).expect("glob result");
        assert_eq!(exec.call.name, "glob");
        assert!(exec.summary.contains("README.md"));
        assert_eq!(exec.started_event["type"], "tool_started");
        assert_eq!(exec.finished_event["type"], "tool_finished");
        assert_eq!(exec.finished_event["success"], true);

        let exec = execute_tool_intent(&vfs, &ToolIntent::Read { path: "README.md".into() }, 2)
            .expect("read result");
        assert_eq!(exec.call.name, "read");
        assert!(exec.summary.contains("Fusion Web Agent"));
        assert_eq!(exec.finished_event["success"], true);

        let exec =
            execute_tool_intent(&vfs, &ToolIntent::Read { path: "missing.txt".into() }, 3)
                .expect("read result");
        assert_eq!(exec.finished_event["success"], false);
        assert!(exec.summary.contains("Could not read"));

        let exec = execute_tool_intent(&vfs, &ToolIntent::Grep { pattern: "Fusion".into() }, 4)
            .expect("grep result");
        assert_eq!(exec.call.name, "grep");
        assert!(exec.summary.contains("match"));

        assert!(execute_tool_intent(&vfs, &ToolIntent::None, 5).is_none());
    }

    // ------------------------------------------------------------------
    // Session Management
    // ------------------------------------------------------------------

    #[test]
    fn test_wasm_agent_creation_and_checkpoint() {
        let agent = create_agent(
            r#"{"default_provider": "fusion", "default_model": "fusion-coder"}"#,
        )
        .expect("Failed to create agent");

        assert_eq!(agent.get_active_model(), "fusion-coder");
        assert_eq!(agent.get_provider(), "fusion");
        assert_eq!(agent.get_turn_count(), 0);
        assert!(!agent.get_session_id().is_empty());

        let checkpoint_json = agent.checkpoint().expect("Failed to checkpoint");
        assert!(checkpoint_json.contains("fusion-coder"));
        assert!(checkpoint_json.contains("session"));
        assert!(checkpoint_json.contains("vfs"));

        let mut agent2 = create_agent("{}").expect("Failed to create agent 2");
        agent2
            .restore(&checkpoint_json)
            .expect("Failed to restore checkpoint");
        assert_eq!(agent2.get_active_model(), "fusion-coder");
    }

    #[test]
    fn test_session_metadata_and_config_api() {
        let mut agent = create_agent("{}").expect("Failed to create agent");

        agent.set_session_title("Browser Session");
        assert_eq!(agent.get_session_title().as_deref(), Some("Browser Session"));

        agent.set_system_prompt("You are a browser coding assistant.");
        assert_eq!(
            agent.get_system_prompt().as_deref(),
            Some("You are a browser coding assistant.")
        );

        agent.set_provider("fusion");
        assert_eq!(agent.get_provider(), "fusion");

        let config_json = agent.get_config_json().expect("config json");
        assert!(config_json.contains("default_provider"));

        let summary = agent.get_stats_summary();
        assert!(summary.contains("Tokens"));

        agent.clear_messages();
        let msgs = agent.get_messages().expect("messages");
        assert_eq!(msgs, "[]");
    }

    #[test]
    fn test_restore_rejects_invalid_json() {
        let mut agent = create_agent("{}").expect("Failed to create agent");
        assert!(agent.restore("not json at all").is_err());
    }

    // ------------------------------------------------------------------
    // VFS API surface
    // ------------------------------------------------------------------

    #[test]
    fn test_agent_vfs_api() {
        let mut agent = create_agent("{}").expect("Failed to create agent");

        agent.fs_write("notes.txt", "alpha beta");
        assert!(agent.fs_exists("notes.txt"));
        assert_eq!(agent.fs_read("notes.txt").expect("read"), "alpha beta");

        let edited = agent
            .fs_edit("notes.txt", "alpha", "gamma")
            .expect("edit");
        assert!(edited.contains("Successfully"));
        assert_eq!(agent.fs_read("notes.txt").expect("read"), "gamma beta");

        let globbed = agent.fs_glob("*.txt").expect("glob");
        assert!(globbed.contains("notes.txt"));

        let grepped = agent.fs_grep("gamma", None).expect("grep");
        assert!(grepped.contains("notes.txt"));

        assert!(agent.fs_delete("notes.txt"));
        assert!(agent.fs_read("notes.txt").is_err());

        let bash_result = agent.fs_execute_bash("pwd").expect("bash");
        let parsed: Value = serde_json::from_str(&bash_result).expect("parse bash result");
        assert_eq!(parsed["success"], true);
        assert_eq!(parsed["output"], "/workspace");
    }

    // ------------------------------------------------------------------
    // ACP JSON-RPC Bridge
    // ------------------------------------------------------------------

    #[test]
    fn test_acp_initialize_and_ping() {
        let mut agent = create_agent("{}").expect("Failed to create agent");

        let resp = agent
            .handle_acp_message(r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":1}}"#)
            .expect("initialize response");
        let parsed: Value = serde_json::from_str(&resp).expect("parse");
        assert_eq!(parsed["jsonrpc"], "2.0");
        assert_eq!(parsed["id"], 1);
        assert_eq!(parsed["result"]["protocolVersion"], PROTOCOL_VERSION);
        assert_eq!(parsed["result"]["agentInfo"]["name"], "fusion");
        assert!(parsed["error"].is_null());

        let resp = agent
            .handle_acp_message(r#"{"jsonrpc":"2.0","id":2,"method":"ping"}"#)
            .expect("ping response");
        let parsed: Value = serde_json::from_str(&resp).expect("parse");
        assert_eq!(parsed["result"]["pong"], true);
    }

    #[test]
    fn test_acp_parse_error() {
        let mut agent = create_agent("{}").expect("Failed to create agent");
        let resp = agent
            .handle_acp_message("{invalid json")
            .expect("error response envelope");
        let parsed: Value = serde_json::from_str(&resp).expect("parse");
        assert_eq!(parsed["error"]["code"], -32700);
        assert!(parsed["result"].is_null());
    }

    #[test]
    fn test_acp_method_not_found() {
        let mut agent = create_agent("{}").expect("Failed to create agent");
        let resp = agent
            .handle_acp_message(r#"{"jsonrpc":"2.0","id":7,"method":"no/such/method"}"#)
            .expect("response");
        let parsed: Value = serde_json::from_str(&resp).expect("parse");
        assert_eq!(parsed["error"]["code"], -32601);
    }

    #[test]
    fn test_acp_notification_yields_empty_response() {
        let mut agent = create_agent("{}").expect("Failed to create agent");
        let resp = agent
            .handle_acp_message(r#"{"jsonrpc":"2.0","method":"initialized"}"#)
            .expect("notification handled");
        assert_eq!(resp, "");
    }

    #[test]
    fn test_acp_session_lifecycle() {
        let mut agent = create_agent("{}").expect("Failed to create agent");

        // Capture the current session id via checkpoint JSON
        let cp = agent.checkpoint().expect("checkpoint");
        let cp_val: Value = serde_json::from_str(&cp).expect("parse checkpoint");
        let session_id = cp_val["session"]["id"].as_str().expect("session id").to_string();

        // session/list includes the active session
        let resp = agent
            .handle_acp_message(r#"{"jsonrpc":"2.0","id":10,"method":"session/list"}"#)
            .expect("list response");
        let parsed: Value = serde_json::from_str(&resp).expect("parse");
        assert_eq!(parsed["result"]["sessions"].as_array().expect("sessions").len(), 1);
        assert_eq!(parsed["result"]["sessions"][0]["sessionId"], session_id.as_str());

        // session/load with matching id
        let load_req = format!(
            r#"{{"jsonrpc":"2.0","id":11,"method":"session/load","params":{{"sessionId":"{}"}}}}"#,
            session_id
        );
        let resp = agent.handle_acp_message(&load_req).expect("load response");
        let parsed: Value = serde_json::from_str(&resp).expect("parse");
        assert_eq!(parsed["result"]["sessionId"], session_id.as_str());
        assert!(parsed["result"]["messageCount"].is_u64());

        // session/load with unknown id -> session_not_found
        let bad_req = r#"{"jsonrpc":"2.0","id":12,"method":"session/load","params":{"sessionId":"00000000-0000-0000-0000-000000000000"}}"#;
        let resp = agent.handle_acp_message(bad_req).expect("load response");
        let parsed: Value = serde_json::from_str(&resp).expect("parse");
        assert_eq!(parsed["error"]["code"], -32001);

        // session/new resets the session
        let resp = agent
            .handle_acp_message(r#"{"jsonrpc":"2.0","id":13,"method":"session/new"}"#)
            .expect("new response");
        let parsed: Value = serde_json::from_str(&resp).expect("parse");
        let new_id = parsed["result"]["sessionId"].as_str().expect("new id");
        assert_ne!(new_id, session_id.as_str());
        assert_eq!(agent.get_turn_count(), 0);

        // session/cancel and session/close succeed
        let resp = agent
            .handle_acp_message(r#"{"jsonrpc":"2.0","id":14,"method":"session/cancel"}"#)
            .expect("cancel response");
        let parsed: Value = serde_json::from_str(&resp).expect("parse");
        assert_eq!(parsed["result"]["cancelled"], true);

        let resp = agent
            .handle_acp_message(r#"{"jsonrpc":"2.0","id":15,"method":"session/close"}"#)
            .expect("close response");
        let parsed: Value = serde_json::from_str(&resp).expect("parse");
        assert!(parsed["result"].is_null());
    }

    #[test]
    fn test_acp_session_prompt_turn() {
        let mut agent = create_agent("{}").expect("Failed to create agent");

        // Establish session id
        let cp = agent.checkpoint().expect("checkpoint");
        let cp_val: Value = serde_json::from_str(&cp).expect("parse checkpoint");
        let session_id = cp_val["session"]["id"].as_str().expect("session id").to_string();

        let prompt_req = format!(
            r#"{{"jsonrpc":"2.0","id":20,"method":"session/prompt","params":{{"sessionId":"{}","prompt":"list files"}}}}"#,
            session_id
        );
        let resp = agent.handle_acp_message(&prompt_req).expect("prompt response");
        let parsed: Value = serde_json::from_str(&resp).expect("parse");

        assert_eq!(parsed["result"]["stopReason"], "end_turn");
        let content = parsed["result"]["content"].as_array().expect("content");
        assert_eq!(content[0]["type"], "text");
        assert!(content[0]["text"].as_str().expect("text").contains("README.md"));
        assert!(parsed["result"]["stats"]["totalTokens"].as_u64().expect("tokens") > 0);
        assert_eq!(agent.get_turn_count(), 1);

        // Unknown session id -> session_not_found
        let bad_req = format!(
            r#"{{"jsonrpc":"2.0","id":21,"method":"session/prompt","params":{{"sessionId":"deadbeef","prompt":"hi"}}}}"#
        );
        let resp = agent.handle_acp_message(&bad_req).expect("prompt response");
        let parsed: Value = serde_json::from_str(&resp).expect("parse");
        assert_eq!(parsed["error"]["code"], -32001);
    }

    // ------------------------------------------------------------------
    // Prompt Streaming (offline mode, no API key configured)
    // ------------------------------------------------------------------

    #[tokio::test]
    async fn test_wasm_prompt_turn() {
        let mut agent = create_agent("{}").expect("Failed to create agent");
        let response = agent
            .prompt_turn("list files", None)
            .await
            .expect("Prompt turn failed");

        assert!(response.contains("README.md"));
        assert_eq!(agent.get_turn_count(), 1);

        let msgs = agent.get_messages().expect("Failed to get messages");
        assert!(msgs.contains("list files"));

        let stats = agent.get_token_stats().expect("Failed to get token stats");
        assert!(stats.contains("total_tokens"));
    }

    #[tokio::test]
    async fn test_wasm_prompt_turn_conversation() {
        let mut agent = create_agent("{}").expect("Failed to create agent");
        let response = agent
            .prompt_turn("hello, who are you?", None)
            .await
            .expect("Prompt turn failed");

        assert!(response.contains("WASM Browser Mode"));
        assert!(response.contains("hello, who are you?"));

        let stats = agent.get_token_stats().expect("stats");
        let parsed: Value = serde_json::from_str(&stats).expect("parse stats");
        assert!(parsed["total_tokens"].as_u64().expect("total") > 0);
        assert_eq!(parsed["total_turns"].as_u64().expect("turns"), 1);
    }
}
