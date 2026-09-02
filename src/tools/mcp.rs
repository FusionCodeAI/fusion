//! Model Context Protocol (MCP) Client Adapter.
//!
//! Provides a standards-compliant client implementation for the Model Context Protocol (MCP),
//! enabling Fusion to connect to external stdio MCP servers (Node.js, Python, Rust, Go, etc.)
//! and dynamically discover, inspect, and register their tools into Fusion's [`ToolRegistry`].
//!
//! # Protocol Compliance
//!
//! Implements JSON-RPC 2.0 stdio transport per the MCP specification (2024-11-05 & 2024-10-07):
//! - Lifecycle handshake: `initialize` request, capabilities negotiation, `notifications/initialized`.
//! - Tool discovery: `tools/list` with cursor-based pagination handling.
//! - Tool invocation: `tools/call` with argument passing and structured content parsing (text, image, resource).
//! - Diagnostics: JSON-RPC error mapping, server notifications, process lifecycle tracking, and graceful shutdown.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, Command};
use tokio::sync::{mpsc, oneshot, Mutex, RwLock};
use tracing::{debug, error, info, warn};
use crate::tools::types::{DynTool, Tool, ToolContext, ToolRegistry};

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// Latest supported MCP protocol specification version.
pub const LATEST_PROTOCOL_VERSION: &str = "2024-11-05";

/// Supported MCP protocol versions in order of preference.
pub const SUPPORTED_PROTOCOL_VERSIONS: &[&str] = &["2024-11-05", "2024-10-07"];

/// Default client name advertised in MCP handshake.
pub const DEFAULT_CLIENT_NAME: &str = "fusion";

/// Default client version advertised in MCP handshake.
pub const DEFAULT_CLIENT_VERSION: &str = env!("CARGO_PKG_VERSION");

/// Default tool execution timeout in seconds.
pub const DEFAULT_TIMEOUT_SECS: u64 = 60;

// Standard JSON-RPC 2.0 Error Codes
pub const PARSE_ERROR: i64 = -32700;
pub const INVALID_REQUEST: i64 = -32600;
pub const METHOD_NOT_FOUND: i64 = -32601;
pub const INVALID_PARAMS: i64 = -32602;
pub const INTERNAL_ERROR: i64 = -32603;

// ---------------------------------------------------------------------------
// JSON-RPC 2.0 Core Types
// ---------------------------------------------------------------------------

/// A JSON-RPC 2.0 request.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct JsonRpcRequest {
    pub jsonrpc: String,
    pub id: Value,
    pub method: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub params: Option<Value>,
}

impl JsonRpcRequest {
    pub fn new(id: Value, method: impl Into<String>, params: Option<Value>) -> Self {
        Self {
            jsonrpc: "2.0".to_string(),
            id,
            method: method.into(),
            params,
        }
    }
}

/// A JSON-RPC 2.0 notification (request without id).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct JsonRpcNotification {
    pub jsonrpc: String,
    pub method: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub params: Option<Value>,
}

impl JsonRpcNotification {
    pub fn new(method: impl Into<String>, params: Option<Value>) -> Self {
        Self {
            jsonrpc: "2.0".to_string(),
            method: method.into(),
            params,
        }
    }
}

/// A JSON-RPC 2.0 response.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct JsonRpcResponse {
    pub jsonrpc: String,
    #[serde(default)]
    pub id: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<JsonRpcError>,
}

/// A JSON-RPC 2.0 error object.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct JsonRpcError {
    pub code: i64,
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,
}

impl std::fmt::Display for JsonRpcError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "[code {}] {}", self.code, self.message)?;
        if let Some(data) = &self.data {
            write!(f, " (data: {})", data)?;
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// MCP Protocol Types
// ---------------------------------------------------------------------------

/// Server or client implementation metadata.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Implementation {
    pub name: String,
    pub version: String,
}

/// Capabilities declared by the MCP client during initialization.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ClientCapabilities {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub experimental: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub roots: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sampling: Option<Value>,
}

/// Parameters sent by the client in the `initialize` request.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct McpInitializeParams {
    pub protocol_version: String,
    pub capabilities: ClientCapabilities,
    pub client_info: Implementation,
}

/// Tools capability descriptor from server.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ToolsCapability {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub list_changed: Option<bool>,
}

/// Capabilities declared by the MCP server.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ServerCapabilities {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tools: Option<ToolsCapability>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub logging: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prompts: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resources: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub experimental: Option<Value>,
}

/// Result returned by the MCP server in response to `initialize`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct McpInitializeResult {
    pub protocol_version: String,
    pub capabilities: ServerCapabilities,
    pub server_info: Implementation,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub instructions: Option<String>,
}

/// MCP tool definition advertised by `tools/list`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct McpToolDefinition {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(alias = "input_schema")]
    pub input_schema: Value,
}

/// Result of `tools/list` request.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct McpListToolsResult {
    pub tools: Vec<McpToolDefinition>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_cursor: Option<String>,
}

/// Parameters for calling an MCP tool (`tools/call`).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct McpCallToolParams {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub arguments: Option<Value>,
}

/// Content items returned by `tools/call`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum McpContent {
    #[serde(rename = "text")]
    Text { text: String },
    #[serde(rename = "image")]
    Image {
        data: String,
        #[serde(rename = "mimeType", alias = "mime_type")]
        mime_type: String,
    },
    #[serde(rename = "resource")]
    Resource { resource: Value },
    #[serde(other)]
    Unknown,
}

/// Result returned by `tools/call`.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct McpCallToolResult {
    #[serde(default)]
    pub content: Vec<McpContent>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub is_error: Option<bool>,
}

// ---------------------------------------------------------------------------
// Error Types
// ---------------------------------------------------------------------------

/// MCP Client Error.
#[derive(Debug, thiserror::Error)]
pub enum McpError {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[error("JSON serialization/deserialization error: {0}")]
    Json(#[from] serde_json::Error),

    #[error("JSON-RPC error {code}: {message}")]
    JsonRpc {
        code: i64,
        message: String,
        data: Option<Value>,
    },

    #[error("MCP server timeout: {0}")]
    Timeout(String),

    #[error("MCP server process exited: {0}")]
    ProcessExited(String),

    #[error("MCP client is not initialized")]
    NotInitialized,

    #[error("MCP connection is closed")]
    ConnectionClosed,

    #[error("MCP tool execution failed: {0}")]
    ToolExecutionFailed(String),

    #[error("Invalid MCP response: {0}")]
    InvalidResponse(String),

    #[error("Configuration error: {0}")]
    ConfigError(String),
}

// ---------------------------------------------------------------------------
// Server Configuration
// ---------------------------------------------------------------------------

/// Configuration for an external stdio MCP server.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct McpServerConfig {
    /// Identifier for the server (e.g. "filesystem", "github", "memory").
    #[serde(default)]
    pub name: String,

    /// Executable binary or script command (e.g. "npx", "python", "node").
    pub command: String,

    /// Command-line arguments.
    #[serde(default)]
    pub args: Vec<String>,

    /// Extra environment variables to pass to the spawned process.
    #[serde(default)]
    pub env: HashMap<String, String>,

    /// Working directory for the process.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cwd: Option<PathBuf>,

    /// Whether this server is disabled.
    #[serde(default)]
    pub disabled: bool,

    /// Timeout in seconds for tool executions and requests.
    #[serde(default, skip_serializing_if = "Option::is_none", alias = "timeout")]
    pub timeout_secs: Option<u64>,

    /// Optional prefix added to tool names to prevent collisions (e.g. "fs_").
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prefix: Option<String>,

    /// Tools allowed to run automatically without interactive prompts.
    #[serde(default, alias = "autoApprove")]
    pub auto_approve: Vec<String>,
}

impl McpServerConfig {
    /// Creates a new MCP server configuration.
    pub fn new(name: impl Into<String>, command: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            command: command.into(),
            args: Vec::new(),
            env: HashMap::new(),
            cwd: None,
            disabled: false,
            timeout_secs: Some(DEFAULT_TIMEOUT_SECS),
            prefix: None,
            auto_approve: Vec::new(),
        }
    }

    /// Adds a single command-line argument.
    pub fn with_arg(mut self, arg: impl Into<String>) -> Self {
        self.args.push(arg.into());
        self
    }

    /// Appends multiple command-line arguments.
    pub fn with_args<I, S>(mut self, args: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.args.extend(args.into_iter().map(Into::into));
        self
    }

    /// Sets an environment variable for the server process.
    pub fn with_env(mut self, key: impl Into<String>, val: impl Into<String>) -> Self {
        self.env.insert(key.into(), val.into());
        self
    }

    /// Sets the working directory for the server process.
    pub fn with_cwd(mut self, cwd: impl Into<PathBuf>) -> Self {
        self.cwd = Some(cwd.into());
        self
    }

    /// Sets the request timeout in seconds.
    pub fn with_timeout(mut self, timeout_secs: u64) -> Self {
        self.timeout_secs = Some(timeout_secs);
        self
    }

    /// Sets the tool name prefix.
    pub fn with_prefix(mut self, prefix: impl Into<String>) -> Self {
        self.prefix = Some(prefix.into());
        self
    }

    /// Sets whether the server is disabled.
    pub fn with_disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    /// Adds an auto-approved tool name.
    pub fn with_auto_approve(mut self, tool: impl Into<String>) -> Self {
        self.auto_approve.push(tool.into());
        self
    }
}

/// Helper container for parsing MCP server configurations from various JSON layouts.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct McpServersConfig {
    #[serde(
        default,
        alias = "mcpServers",
        alias = "mcp_servers",
        alias = "servers"
    )]
    pub mcp_servers: HashMap<String, McpServerConfig>,
}

impl McpServersConfig {
    /// Parse server configurations from a JSON string.
    ///
    /// Supports:
    /// - Claude Desktop / VS Code standard: `{"mcpServers": { "name": { "command": "...", "args": [...] } }}`
    /// - Map format: `{"name": { "command": "..." }}`
    /// - List format: `[ { "name": "...", "command": "..." } ]`
    /// - Single server format: `{ "name": "...", "command": "..." }`
    pub fn from_json_str(json_str: &str) -> Result<Vec<McpServerConfig>, McpError> {
        let val: Value = serde_json::from_str(json_str)?;
        Self::from_value(&val)
    }

    /// Parse server configurations from a `serde_json::Value`.
    pub fn from_value(val: &Value) -> Result<Vec<McpServerConfig>, McpError> {
        // Case 1: Array of server objects
        if let Some(arr) = val.as_array() {
            let mut configs = Vec::new();
            for (idx, item) in arr.iter().enumerate() {
                let mut cfg: McpServerConfig = serde_json::from_value(item.clone())?;
                if cfg.name.is_empty() {
                    cfg.name = format!("mcp_server_{}", idx + 1);
                }
                configs.push(cfg);
            }
            return Ok(configs);
        }

        // Case 2: Object containing "mcpServers", "mcp_servers", or "servers"
        if let Some(obj) = val.as_object() {
            let nested_key = obj
                .get("mcpServers")
                .or_else(|| obj.get("mcp_servers"))
                .or_else(|| obj.get("servers"));

            if let Some(nested) = nested_key {
                if let Some(nested_obj) = nested.as_object() {
                    let mut configs = Vec::new();
                    for (name, cfg_val) in nested_obj {
                        let mut cfg: McpServerConfig = serde_json::from_value(cfg_val.clone())?;
                        if cfg.name.is_empty() {
                            cfg.name = name.clone();
                        }
                        configs.push(cfg);
                    }
                    return Ok(configs);
                } else if let Some(nested_arr) = nested.as_array() {
                    let mut configs = Vec::new();
                    for (idx, item) in nested_arr.iter().enumerate() {
                        let mut cfg: McpServerConfig = serde_json::from_value(item.clone())?;
                        if cfg.name.is_empty() {
                            cfg.name = format!("mcp_server_{}", idx + 1);
                        }
                        configs.push(cfg);
                    }
                    return Ok(configs);
                }
            }

            // Case 3: Direct map of server name -> server config
            // Check if values have "command" field
            let is_server_map = obj.values().any(|v| v.get("command").is_some());
            if is_server_map {
                let mut configs = Vec::new();
                for (name, cfg_val) in obj {
                    if cfg_val.get("command").is_some() {
                        let mut cfg: McpServerConfig = serde_json::from_value(cfg_val.clone())?;
                        if cfg.name.is_empty() {
                            cfg.name = name.clone();
                        }
                        configs.push(cfg);
                    }
                }
                if !configs.is_empty() {
                    return Ok(configs);
                }
            }

            // Case 4: Single server config object
            if obj.contains_key("command") {
                let mut cfg: McpServerConfig = serde_json::from_value(val.clone())?;
                if cfg.name.is_empty() {
                    cfg.name = "default".to_string();
                }
                return Ok(vec![cfg]);
            }
        }

        Err(McpError::ConfigError(
            "Could not parse valid MCP server configuration from provided JSON".to_string(),
        ))
    }

    /// Loads MCP server configurations from a JSON file.
    pub fn from_file(path: impl AsRef<Path>) -> Result<Vec<McpServerConfig>, McpError> {
        let content = std::fs::read_to_string(path)?;
        Self::from_json_str(&content)
    }

    /// Returns the standard path for MCP servers config: `~/.fusion/mcp_servers.json`.
    pub fn default_config_path() -> PathBuf {
        dirs::home_dir()
            .map(|h| h.join(".fusion").join("mcp_servers.json"))
            .unwrap_or_else(|| PathBuf::from(".fusion/mcp_servers.json"))
    }
}

// ---------------------------------------------------------------------------
// Formatter helper for tool results
// ---------------------------------------------------------------------------

/// Formats an [`McpCallToolResult`] into a string output or error message.
pub fn format_tool_call_result(result: &McpCallToolResult) -> anyhow::Result<String> {
    let mut output = String::new();
    let is_err = result.is_error.unwrap_or(false);

    for content in &result.content {
        match content {
            McpContent::Text { text } => {
                if !output.is_empty() && !output.ends_with('\n') {
                    output.push('\n');
                }
                output.push_str(text);
            }
            McpContent::Image { data, mime_type } => {
                if !output.is_empty() && !output.ends_with('\n') {
                    output.push('\n');
                }
                output.push_str(&format!("[Image: {}, {} bytes base64]", mime_type, data.len()));
            }
            McpContent::Resource { resource } => {
                if !output.is_empty() && !output.ends_with('\n') {
                    output.push('\n');
                }
                if let Ok(formatted) = serde_json::to_string_pretty(resource) {
                    output.push_str(&formatted);
                } else {
                    output.push_str(&resource.to_string());
                }
            }
            McpContent::Unknown => {}
        }
    }

    if is_err {
        if output.trim().is_empty() {
            anyhow::bail!("MCP tool execution failed (isError = true)");
        } else {
            anyhow::bail!("MCP tool error: {}", output.trim());
        }
    }

    if output.is_empty() {
        Ok("(empty response)".to_string())
    } else {
        Ok(output)
    }
}

// ---------------------------------------------------------------------------
// MCP Client (Stdio & Mock)
// ---------------------------------------------------------------------------

type PendingMap = Arc<Mutex<HashMap<u64, oneshot::Sender<Result<Value, McpError>>>>>;
type MockHandler = Arc<dyn Fn(String, Option<Value>) -> Result<Value, McpError> + Send + Sync>;

enum Transport {
    Process {
        stdin_tx: mpsc::Sender<String>,
        child: Arc<Mutex<Option<Child>>>,
    },
    Mock {
        handler: MockHandler,
    },
}

/// An active client connection to an external stdio MCP server.
pub struct McpClient {
    server_name: String,
    config: McpServerConfig,
    transport: Transport,
    pending: PendingMap,
    next_request_id: AtomicU64,
    server_info: Arc<RwLock<Option<Implementation>>>,
    server_capabilities: Arc<RwLock<Option<ServerCapabilities>>>,
    instructions: Arc<RwLock<Option<String>>>,
    is_initialized: Arc<AtomicBool>,
    is_closed: Arc<AtomicBool>,
}

impl McpClient {
    /// Starts and connects to an external stdio MCP server process.
    pub async fn start(config: McpServerConfig) -> Result<Self, McpError> {
        if config.disabled {
            return Err(McpError::ConfigError(format!(
                "MCP server '{}' is disabled",
                config.name
            )));
        }

        let server_name = config.name.clone();
        let mut cmd = Command::new(&config.command);
        cmd.args(&config.args);

        // Configure environment
        if !config.env.is_empty() {
            cmd.envs(&config.env);
        }

        // Configure working directory
        if let Some(cwd) = &config.cwd {
            cmd.current_dir(cwd);
        }

        // Setup stdio redirection
        cmd.stdin(std::process::Stdio::piped());
        cmd.stdout(std::process::Stdio::piped());
        cmd.stderr(std::process::Stdio::piped());

        debug!(
            "Spawning MCP server '{}': {} {:?}",
            server_name, config.command, config.args
        );

        let mut child = cmd.spawn().map_err(|e| {
            McpError::ProcessExited(format!(
                "Failed to spawn MCP server process '{}' (command: '{}'): {}",
                server_name, config.command, e
            ))
        })?;

        let child_stdin = child
            .stdin
            .take()
            .ok_or_else(|| McpError::ProcessExited("Failed to capture child stdin".to_string()))?;
        let child_stdout = child
            .stdout
            .take()
            .ok_or_else(|| McpError::ProcessExited("Failed to capture child stdout".to_string()))?;
        let child_stderr = child
            .stderr
            .take()
            .ok_or_else(|| McpError::ProcessExited("Failed to capture child stderr".to_string()))?;

        let pending: PendingMap = Arc::new(Mutex::new(HashMap::new()));
        let (stdin_tx, mut stdin_rx) = mpsc::channel::<String>(128);

        // Stdin writer task
        tokio::spawn(async move {
            let mut writer = child_stdin;
            while let Some(msg) = stdin_rx.recv().await {
                if let Err(e) = writer.write_all(msg.as_bytes()).await {
                    debug!("Failed to write to MCP stdin: {}", e);
                    break;
                }
                if let Err(e) = writer.flush().await {
                    debug!("Failed to flush MCP stdin: {}", e);
                    break;
                }
            }
        });

        // Stdout reader task
        let pending_clone = Arc::clone(&pending);
        let s_name = server_name.clone();
        tokio::spawn(async move {
            let mut reader = BufReader::new(child_stdout).lines();
            while let Ok(Some(line)) = reader.next_line().await {
                let line_str = line.trim();
                if line_str.is_empty() {
                    continue;
                }

                // Attempt to parse line as JSON-RPC response or notification
                match serde_json::from_str::<Value>(line_str) {
                    Ok(val) => {
                        // Check if this is a response to a pending request
                        if let Some(id_val) = val.get("id") {
                            let id_num = id_val.as_u64().or_else(|| {
                                id_val
                                    .as_str()
                                    .and_then(|s| s.parse::<u64>().ok())
                            });

                            if let Some(id) = id_num {
                                let mut map = pending_clone.lock().await;
                                if let Some(sender) = map.remove(&id) {
                                    if let Some(err_obj) = val.get("error") {
                                        if let Ok(rpc_err) =
                                            serde_json::from_value::<JsonRpcError>(err_obj.clone())
                                        {
                                            let _ = sender.send(Err(McpError::JsonRpc {
                                                code: rpc_err.code,
                                                message: rpc_err.message,
                                                data: rpc_err.data,
                                            }));
                                        } else {
                                            let _ = sender.send(Err(McpError::JsonRpc {
                                                code: INTERNAL_ERROR,
                                                message: err_obj.to_string(),
                                                data: None,
                                            }));
                                        }
                                    } else {
                                        let result = val
                                            .get("result")
                                            .cloned()
                                            .unwrap_or(Value::Null);
                                        let _ = sender.send(Ok(result));
                                    }
                                }
                            }
                        } else if let Some(method) = val.get("method").and_then(|m| m.as_str()) {
                            // Server notification (e.g. logging or tool list changed)
                            debug!(
                                target: "mcp_notification",
                                server = %s_name,
                                method = %method,
                                "Received MCP server notification"
                            );
                        }
                    }
                    Err(e) => {
                        warn!(
                            target: "mcp_parser",
                            server = %s_name,
                            "Failed to parse JSON-RPC line from MCP server: {}. Line: {}",
                            e,
                            line_str
                        );
                    }
                }
            }

            // Clean up any remaining pending requests upon EOF
            let mut map = pending_clone.lock().await;
            for (_, sender) in map.drain() {
                let _ = sender.send(Err(McpError::ProcessExited(
                    "MCP server process stdout closed".to_string(),
                )));
            }
        });

        // Stderr logger task
        let s_name_err = server_name.clone();
        tokio::spawn(async move {
            let mut err_reader = BufReader::new(child_stderr).lines();
            while let Ok(Some(line)) = err_reader.next_line().await {
                if !line.trim().is_empty() {
                    debug!(
                        target: "mcp_stderr",
                        server = %s_name_err,
                        "{}",
                        line
                    );
                }
            }
        });

        Ok(Self {
            server_name,
            config,
            transport: Transport::Process {
                stdin_tx,
                child: Arc::new(Mutex::new(Some(child))),
            },
            pending,
            next_request_id: AtomicU64::new(1),
            server_info: Arc::new(RwLock::new(None)),
            server_capabilities: Arc::new(RwLock::new(None)),
            instructions: Arc::new(RwLock::new(None)),
            is_initialized: Arc::new(AtomicBool::new(false)),
            is_closed: Arc::new(AtomicBool::new(false)),
        })
    }

    /// Creates an in-memory mock client for tests.
    pub fn mock(
        config: McpServerConfig,
        handler: Arc<dyn Fn(String, Option<Value>) -> Result<Value, McpError> + Send + Sync>,
    ) -> Self {
        let server_name = config.name.clone();
        Self {
            server_name,
            config,
            transport: Transport::Mock { handler },
            pending: Arc::new(Mutex::new(HashMap::new())),
            next_request_id: AtomicU64::new(1),
            server_info: Arc::new(RwLock::new(None)),
            server_capabilities: Arc::new(RwLock::new(None)),
            instructions: Arc::new(RwLock::new(None)),
            is_initialized: Arc::new(AtomicBool::new(false)),
            is_closed: Arc::new(AtomicBool::new(false)),
        }
    }

    /// Returns the server name.
    pub fn server_name(&self) -> &str {
        &self.server_name
    }

    /// Returns the server configuration.
    pub fn config(&self) -> &McpServerConfig {
        &self.config
    }

    /// Returns true if client has completed the initialization handshake.
    pub fn is_initialized(&self) -> bool {
        self.is_initialized.load(Ordering::SeqCst)
    }

    /// Returns true if client connection is closed.
    pub fn is_closed(&self) -> bool {
        self.is_closed.load(Ordering::SeqCst)
    }

    /// Returns the server metadata received during initialization.
    pub async fn server_info(&self) -> Option<Implementation> {
        self.server_info.read().await.clone()
    }

    /// Returns the server capabilities received during initialization.
    pub async fn server_capabilities(&self) -> Option<ServerCapabilities> {
        self.server_capabilities.read().await.clone()
    }

    /// Returns optional server instructions provided during initialization.
    pub async fn instructions(&self) -> Option<String> {
        self.instructions.read().await.clone()
    }

    /// Performs the full MCP lifecycle handshake:
    /// 1. Sends `initialize` request with client info and capabilities.
    /// 2. Records server capabilities and metadata.
    /// 3. Sends `notifications/initialized` notification.
    pub async fn initialize(&self) -> Result<McpInitializeResult, McpError> {
        if self.is_closed() {
            return Err(McpError::ConnectionClosed);
        }

        let init_params = McpInitializeParams {
            protocol_version: LATEST_PROTOCOL_VERSION.to_string(),
            capabilities: ClientCapabilities::default(),
            client_info: Implementation {
                name: DEFAULT_CLIENT_NAME.to_string(),
                version: DEFAULT_CLIENT_VERSION.to_string(),
            },
        };

        let res_val = self
            .send_request("initialize", Some(serde_json::to_value(init_params)?))
            .await?;

        let init_result: McpInitializeResult = serde_json::from_value(res_val)?;

        // Store server capabilities and metadata
        *self.server_info.write().await = Some(init_result.server_info.clone());
        *self.server_capabilities.write().await = Some(init_result.capabilities.clone());
        *self.instructions.write().await = init_result.instructions.clone();

        // Send initialized notification
        self.send_notification("notifications/initialized", None)
            .await?;

        self.is_initialized.store(true, Ordering::SeqCst);
        debug!(
            "MCP server '{}' initialized successfully: {:?}",
            self.server_name, init_result.server_info
        );

        Ok(init_result)
    }

    /// Discovers all tools supported by the MCP server (`tools/list`), handling pagination.
    pub async fn list_tools(&self) -> Result<Vec<McpToolDefinition>, McpError> {
        if !self.is_initialized() {
            return Err(McpError::NotInitialized);
        }

        let mut all_tools = Vec::new();
        let mut cursor: Option<String> = None;

        loop {
            let params = cursor.as_ref().map(|c| json!({ "cursor": c }));
            let res_val = self.send_request("tools/list", params).await?;

            let list_res: McpListToolsResult = serde_json::from_value(res_val)?;
            all_tools.extend(list_res.tools);

            if let Some(next) = list_res.next_cursor {
                if !next.is_empty() && Some(&next) != cursor.as_ref() {
                    cursor = Some(next);
                    continue;
                }
            }
            break;
        }

        debug!(
            "Discovered {} tools from MCP server '{}'",
            all_tools.len(),
            self.server_name
        );
        Ok(all_tools)
    }

    /// Executes a tool on the MCP server (`tools/call`).
    pub async fn call_tool(
        &self,
        name: &str,
        arguments: Value,
    ) -> Result<McpCallToolResult, McpError> {
        if !self.is_initialized() {
            return Err(McpError::NotInitialized);
        }

        let params = json!({
            "name": name,
            "arguments": arguments,
        });

        let res_val = self.send_request("tools/call", Some(params)).await?;
        let tool_result: McpCallToolResult = serde_json::from_value(res_val)?;
        Ok(tool_result)
    }

    /// Sends a `ping` request to verify connection liveness.
    pub async fn ping(&self) -> Result<(), McpError> {
        self.send_request("ping", None).await?;
        Ok(())
    }

    /// Sends a JSON-RPC request and awaits the response with timeout.
    pub async fn send_request(
        &self,
        method: &str,
        params: Option<Value>,
    ) -> Result<Value, McpError> {
        if self.is_closed() {
            return Err(McpError::ConnectionClosed);
        }

        match &self.transport {
            Transport::Mock { handler } => handler(method.to_string(), params),
            Transport::Process { stdin_tx, .. } => {
                let req_id = self.next_request_id.fetch_add(1, Ordering::SeqCst);
                let (tx, rx) = oneshot::channel();

                {
                    let mut map = self.pending.lock().await;
                    map.insert(req_id, tx);
                }

                let req = JsonRpcRequest::new(json!(req_id), method, params);
                let mut json_line = serde_json::to_string(&req)?;
                json_line.push('\n');

                if let Err(_) = stdin_tx.send(json_line).await {
                    let mut map = self.pending.lock().await;
                    map.remove(&req_id);
                    return Err(McpError::ConnectionClosed);
                }

                let timeout_secs = self
                    .config
                    .timeout_secs
                    .unwrap_or(DEFAULT_TIMEOUT_SECS);
                let timeout_dur = Duration::from_secs(timeout_secs);

                match tokio::time::timeout(timeout_dur, rx).await {
                    Ok(Ok(result)) => result,
                    Ok(Err(_)) => {
                        Err(McpError::ProcessExited(
                            "Response channel closed unexpectedly".to_string(),
                        ))
                    }
                    Err(_) => {
                        let mut map = self.pending.lock().await;
                        map.remove(&req_id);
                        Err(McpError::Timeout(format!(
                            "Request '{}' (id={}) to MCP server '{}' timed out after {:?}",
                            method, req_id, self.server_name, timeout_dur
                        )))
                    }
                }
            }
        }
    }

    /// Sends a JSON-RPC notification (no response expected).
    pub async fn send_notification(
        &self,
        method: &str,
        params: Option<Value>,
    ) -> Result<(), McpError> {
        if self.is_closed() {
            return Err(McpError::ConnectionClosed);
        }

        match &self.transport {
            Transport::Mock { .. } => Ok(()),
            Transport::Process { stdin_tx, .. } => {
                let notif = JsonRpcNotification::new(method, params);
                let mut json_line = serde_json::to_string(&notif)?;
                json_line.push('\n');

                stdin_tx
                    .send(json_line)
                    .await
                    .map_err(|_| McpError::ConnectionClosed)?;
                Ok(())
            }
        }
    }

    /// Closes connection and terminates the child process.
    pub async fn close(&self) -> Result<(), McpError> {
        if self.is_closed.swap(true, Ordering::SeqCst) {
            return Ok(());
        }

        if let Transport::Process { child, .. } = &self.transport {
            let mut child_guard = child.lock().await;
            if let Some(mut proc) = child_guard.take() {
                debug!("Terminating MCP server process '{}'", self.server_name);
                let _ = proc.kill().await;
            }
        }

        let mut map = self.pending.lock().await;
        for (_, sender) in map.drain() {
            let _ = sender.send(Err(McpError::ConnectionClosed));
        }

        Ok(())
    }
}

impl Drop for McpClient {
    fn drop(&mut self) {
        self.is_closed.store(true, Ordering::SeqCst);
    }
}

// ---------------------------------------------------------------------------
// Dynamic MCP Tool Adapter (implements Fusion `Tool` trait)
// ---------------------------------------------------------------------------

/// Adapter making an external MCP tool callable through Fusion's [`Tool`] trait.
pub struct McpTool {
    client: Arc<McpClient>,
    raw_name: String,
    exposed_name: String,
    description: String,
    parameters: Value,
    server_name: String,
}

impl McpTool {
    /// Creates a new `McpTool` wrapping an `McpClient` and an `McpToolDefinition`.
    pub fn new(
        client: Arc<McpClient>,
        definition: McpToolDefinition,
        prefix: Option<&str>,
    ) -> Self {
        let raw_name = definition.name.clone();
        let exposed_name = match prefix {
            Some(p) if !p.is_empty() => format!("{}{}", p, raw_name),
            _ => raw_name.clone(),
        };

        let server_name = client.server_name().to_string();
        let description = definition.description.unwrap_or_else(|| {
            format!("MCP tool '{}' provided by server '{}'", raw_name, server_name)
        });

        let mut parameters = definition.input_schema;
        if parameters.is_null() || !parameters.is_object() {
            parameters = json!({
                "type": "object",
                "properties": {}
            });
        }

        Self {
            client,
            raw_name,
            exposed_name,
            description,
            parameters,
            server_name,
        }
    }

    /// The original tool name on the MCP server without prefix.
    pub fn raw_name(&self) -> &str {
        &self.raw_name
    }

    /// The name of the MCP server providing this tool.
    pub fn server_name(&self) -> &str {
        &self.server_name
    }

    /// Reference to the underlying MCP client.
    pub fn client(&self) -> &Arc<McpClient> {
        &self.client
    }
}

#[async_trait]
impl Tool for McpTool {
    fn name(&self) -> &str {
        &self.exposed_name
    }

    fn description(&self) -> &str {
        &self.description
    }

    fn parameters(&self) -> Value {
        self.parameters.clone()
    }

    async fn execute(&self, args: Value, _ctx: &ToolContext) -> anyhow::Result<String> {
        let result = self.client.call_tool(&self.raw_name, args).await?;
        format_tool_call_result(&result)
    }
}

// ---------------------------------------------------------------------------
// MCP Server Status & Manager
// ---------------------------------------------------------------------------

/// Runtime status of a registered MCP server.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpServerStatus {
    pub name: String,
    pub command: String,
    pub args: Vec<String>,
    pub is_connected: bool,
    pub is_initialized: bool,
    pub tool_count: usize,
    pub server_info: Option<Implementation>,
}

/// Multi-server manager for connecting, lifecycle-tracking, and dynamically
/// registering external MCP tools into Fusion.
#[derive(Default, Clone)]
pub struct McpManager {
    clients: Arc<RwLock<HashMap<String, Arc<McpClient>>>>,
    tools: Arc<RwLock<HashMap<String, (String, DynTool)>>>,
}

impl McpManager {
    /// Creates a new, empty MCP manager.
    pub fn new() -> Self {
        Self {
            clients: Arc::new(RwLock::new(HashMap::new())),
            tools: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Connects to a single MCP server, runs initialization, discovers tools,
    /// and registers them internally.
    pub async fn connect_server(&self, config: McpServerConfig) -> anyhow::Result<Vec<DynTool>> {
        let server_name = config.name.clone();
        if config.disabled {
            return Ok(Vec::new());
        }

        // Disconnect existing client if already connected with this name
        let _ = self.disconnect_server(&server_name).await;

        let prefix = config.prefix.clone();
        let client = Arc::new(McpClient::start(config).await?);
        client.initialize().await?;

        let tool_defs = client.list_tools().await?;
        let mut created_tools: Vec<DynTool> = Vec::new();

        for def in tool_defs {
            let mcp_tool = McpTool::new(Arc::clone(&client), def, prefix.as_deref());
            let dyn_tool: DynTool = Arc::new(mcp_tool);
            created_tools.push(dyn_tool);
        }

        // Register tools internally
        {
            let mut clients_guard = self.clients.write().await;
            clients_guard.insert(server_name.clone(), Arc::clone(&client));

            let mut tools_guard = self.tools.write().await;
            for tool in &created_tools {
                tools_guard.insert(tool.name().to_string(), (server_name.clone(), Arc::clone(tool)));
            }
        }

        info!(
            "MCP server '{}' connected and registered {} tools",
            client.server_name(),
            created_tools.len()
        );

        Ok(created_tools)
    }

    /// Connects to a list of MCP servers, returning all discovered tools.
    pub async fn connect_servers(
        &self,
        configs: Vec<McpServerConfig>,
    ) -> Vec<anyhow::Result<Vec<DynTool>>> {
        let mut results = Vec::new();
        for config in configs {
            results.push(self.connect_server(config).await);
        }
        results
    }

    /// Loads configurations from a JSON string and connects to all active servers.
    pub async fn load_from_json(&self, json_str: &str) -> anyhow::Result<Vec<DynTool>> {
        let configs = McpServersConfig::from_json_str(json_str)?;
        let mut all_tools = Vec::new();

        for config in configs {
            if !config.disabled {
                match self.connect_server(config).await {
                    Ok(tools) => all_tools.extend(tools),
                    Err(e) => warn!("Failed to connect MCP server: {}", e),
                }
            }
        }

        Ok(all_tools)
    }

    /// Loads configurations from a file and connects to all active servers.
    pub async fn load_from_config_file(&self, path: impl AsRef<Path>) -> anyhow::Result<Vec<DynTool>> {
        let configs = McpServersConfig::from_file(path)?;
        let mut all_tools = Vec::new();

        for config in configs {
            if !config.disabled {
                match self.connect_server(config).await {
                    Ok(tools) => all_tools.extend(tools),
                    Err(e) => warn!("Failed to connect MCP server: {}", e),
                }
            }
        }

        Ok(all_tools)
    }

    /// Automatically checks and loads from `~/.fusion/mcp_servers.json` if it exists.
    pub async fn load_default_if_exists(&self) -> anyhow::Result<Vec<DynTool>> {
        let path = McpServersConfig::default_config_path();
        if path.exists() {
            self.load_from_config_file(&path).await
        } else {
            Ok(Vec::new())
        }
    }

    /// Copies all active MCP tools into the destination [`ToolRegistry`].
    pub async fn register_into(&self, registry: &mut ToolRegistry) {
        let tools_guard = self.tools.read().await;
        for (_, (_, tool)) in tools_guard.iter() {
            registry.register(Arc::clone(tool));
        }
    }

    /// Creates a new [`ToolRegistry`] populated with all active MCP tools.
    pub async fn create_registry(&self) -> ToolRegistry {
        let mut registry = ToolRegistry::new();
        self.register_into(&mut registry).await;
        registry
    }

    /// Returns the names of all registered MCP servers.
    pub async fn list_server_names(&self) -> Vec<String> {
        self.clients.read().await.keys().cloned().collect()
    }

    /// Returns runtime statuses for all connected MCP servers.
    pub async fn list_server_statuses(&self) -> Vec<McpServerStatus> {
        let clients = self.clients.read().await;
        let tools = self.tools.read().await;
        let mut statuses = Vec::new();

        for (name, client) in clients.iter() {
            let tool_count = tools
                .values()
                .filter(|(s_name, _)| s_name == name)
                .count();

            statuses.push(McpServerStatus {
                name: name.clone(),
                command: client.config().command.clone(),
                args: client.config().args.clone(),
                is_connected: !client.is_closed(),
                is_initialized: client.is_initialized(),
                tool_count,
                server_info: client.server_info().await,
            });
        }

        statuses
    }

    /// Returns a specific MCP client by server name.
    pub async fn get_client(&self, name: &str) -> Option<Arc<McpClient>> {
        self.clients.read().await.get(name).cloned()
    }

    /// Returns all registered MCP tools.
    pub async fn list_tools(&self) -> Vec<DynTool> {
        self.tools
            .read()
            .await
            .values()
            .map(|(_, tool)| Arc::clone(tool))
            .collect()
    }

    /// Disconnects and removes a specific MCP server and its tools.
    pub async fn disconnect_server(&self, name: &str) -> anyhow::Result<()> {
        let client = {
            let mut clients_guard = self.clients.write().await;
            clients_guard.remove(name)
        };

        if let Some(c) = client {
            let _ = c.close().await;

            let mut tools_guard = self.tools.write().await;
            tools_guard.retain(|_, (s_name, _)| s_name != name);
        }

        Ok(())
    }

    /// Disconnects all connected MCP servers.
    pub async fn disconnect_all(&self) {
        let clients = {
            let mut clients_guard = self.clients.write().await;
            let list: Vec<Arc<McpClient>> = clients_guard.values().cloned().collect();
            clients_guard.clear();
            list
        };

        for client in clients {
            let _ = client.close().await;
        }

        self.tools.write().await.clear();
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_jsonrpc_request_serialization() {
        let req = JsonRpcRequest::new(json!(1), "initialize", Some(json!({"foo": "bar"})));
        let serialized = serde_json::to_string(&req).unwrap();
        assert!(serialized.contains("\"jsonrpc\":\"2.0\""));
        assert!(serialized.contains("\"id\":1"));
        assert!(serialized.contains("\"method\":\"initialize\""));
        assert!(serialized.contains("\"params\":{\"foo\":\"bar\"}"));
    }

    #[test]
    fn test_jsonrpc_response_deserialization() {
        let json_str = r#"{"jsonrpc":"2.0","id":42,"result":{"protocolVersion":"2024-11-05"}}"#;
        let resp: JsonRpcResponse = serde_json::from_str(json_str).unwrap();
        assert_eq!(resp.id, Some(json!(42)));
        assert!(resp.result.is_some());
        assert!(resp.error.is_none());

        let err_json = r#"{"jsonrpc":"2.0","id":42,"error":{"code":-32601,"message":"Method not found"}}"#;
        let err_resp: JsonRpcResponse = serde_json::from_str(err_json).unwrap();
        assert_eq!(err_resp.error.unwrap().code, -32601);
    }

    #[test]
    fn test_mcp_server_config_parsing_claude_desktop_format() {
        let json_str = r#"{
            "mcpServers": {
                "filesystem": {
                    "command": "npx",
                    "args": ["-y", "@modelcontextprotocol/server-filesystem", "/tmp"],
                    "env": {"NODE_ENV": "production"}
                },
                "memory": {
                    "command": "python",
                    "args": ["-m", "mcp_server_memory"],
                    "disabled": true
                }
            }
        }"#;

        let configs = McpServersConfig::from_json_str(json_str).unwrap();
        assert_eq!(configs.len(), 2);

        let fs_cfg = configs.iter().find(|c| c.name == "filesystem").unwrap();
        assert_eq!(fs_cfg.command, "npx");
        assert_eq!(fs_cfg.args.len(), 3);
        assert_eq!(fs_cfg.env.get("NODE_ENV").unwrap(), "production");
        assert!(!fs_cfg.disabled);

        let mem_cfg = configs.iter().find(|c| c.name == "memory").unwrap();
        assert_eq!(mem_cfg.command, "python");
        assert!(mem_cfg.disabled);
    }

    #[test]
    fn test_mcp_server_config_builder() {
        let cfg = McpServerConfig::new("test_srv", "node")
            .with_arg("index.js")
            .with_arg("--port=8080")
            .with_env("KEY", "VALUE")
            .with_prefix("mcp_")
            .with_timeout(30);

        assert_eq!(cfg.name, "test_srv");
        assert_eq!(cfg.command, "node");
        assert_eq!(cfg.args, vec!["index.js", "--port=8080"]);
        assert_eq!(cfg.env.get("KEY").unwrap(), "VALUE");
        assert_eq!(cfg.prefix, Some("mcp_".to_string()));
        assert_eq!(cfg.timeout_secs, Some(30));
    }

    #[test]
    fn test_format_tool_call_result_text() {
        let res = McpCallToolResult {
            content: vec![
                McpContent::Text {
                    text: "Hello World".to_string(),
                },
                McpContent::Text {
                    text: "Second Line".to_string(),
                },
            ],
            is_error: Some(false),
        };

        let formatted = format_tool_call_result(&res).unwrap();
        assert_eq!(formatted, "Hello World\nSecond Line");
    }

    #[test]
    fn test_format_tool_call_result_image() {
        let res = McpCallToolResult {
            content: vec![McpContent::Image {
                data: "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mNk+A8AAQUBAScY42YAAAAASUVORK5CYII=".to_string(),
                mime_type: "image/png".to_string(),
            }],
            is_error: None,
        };

        let formatted = format_tool_call_result(&res).unwrap();
        assert!(formatted.contains("[Image: image/png"));
    }

    #[test]
    fn test_format_tool_call_result_error() {
        let res = McpCallToolResult {
            content: vec![McpContent::Text {
                text: "Permission denied".to_string(),
            }],
            is_error: Some(true),
        };

        let err = format_tool_call_result(&res).unwrap_err();
        assert!(err.to_string().contains("Permission denied"));
    }

    #[tokio::test]
    async fn test_mock_mcp_client_handshake_and_tool_call() {
        let config = McpServerConfig::new("mock_server", "mock_cmd");

        let handler: MockHandler = Arc::new(|method, params| match method.as_str() {
            "initialize" => Ok(json!({
                "protocolVersion": "2024-11-05",
                "capabilities": {
                    "tools": { "listChanged": true }
                },
                "serverInfo": {
                    "name": "mock-mcp-server",
                    "version": "1.0.0"
                }
            })),
            "tools/list" => Ok(json!({
                "tools": [
                    {
                        "name": "calculate",
                        "description": "Perform calculation",
                        "inputSchema": {
                            "type": "object",
                            "properties": {
                                "expr": {"type": "string"}
                            },
                            "required": ["expr"]
                        }
                    }
                ]
            })),
            "tools/call" => {
                let p = params.unwrap_or_default();
                let tool_name = p.get("name").and_then(|v| v.as_str()).unwrap_or("");
                let args = p.get("arguments").cloned().unwrap_or_default();

                if tool_name == "calculate" {
                    let expr = args.get("expr").and_then(|e| e.as_str()).unwrap_or("");
                    if expr == "2 + 2" {
                        Ok(json!({
                            "content": [
                                {"type": "text", "text": "4"}
                            ],
                            "isError": false
                        }))
                    } else {
                        Ok(json!({
                            "content": [
                                {"type": "text", "text": "Unknown expr"}
                            ],
                            "isError": true
                        }))
                    }
                } else {
                    Err(McpError::JsonRpc {
                        code: METHOD_NOT_FOUND,
                        message: "Tool not found".to_string(),
                        data: None,
                    })
                }
            }
            _ => Err(McpError::JsonRpc {
                code: METHOD_NOT_FOUND,
                message: "Method not found".to_string(),
                data: None,
            }),
        });

        let client = Arc::new(McpClient::mock(config, handler));

        // Test handshake
        assert!(!client.is_initialized());
        let init_res = client.initialize().await.unwrap();
        assert!(client.is_initialized());
        assert_eq!(init_res.server_info.name, "mock-mcp-server");

        // Test tool discovery
        let tools = client.list_tools().await.unwrap();
        assert_eq!(tools.len(), 1);
        assert_eq!(tools[0].name, "calculate");

        // Test tool wrapping
        let mcp_tool = McpTool::new(Arc::clone(&client), tools[0].clone(), Some("math_"));
        assert_eq!(mcp_tool.name(), "math_calculate");
        assert_eq!(mcp_tool.raw_name(), "calculate");
        assert_eq!(mcp_tool.description(), "Perform calculation");

        // Test tool execution via Tool trait
        let ctx = ToolContext::default();
        let res = mcp_tool
            .execute(json!({"expr": "2 + 2"}), &ctx)
            .await
            .unwrap();
        assert_eq!(res, "4");

        // Test error execution
        let err_res = mcp_tool.execute(json!({"expr": "invalid"}), &ctx).await;
        assert!(err_res.is_err());
    }

    #[tokio::test]
    async fn test_dynamic_registry_integration() {
        let config = McpServerConfig::new("echo_server", "echo");
        let handler: MockHandler = Arc::new(|method, _params| match method.as_str() {
            "initialize" => Ok(json!({
                "protocolVersion": "2024-11-05",
                "capabilities": { "tools": {} },
                "serverInfo": { "name": "echo", "version": "1.0" }
            })),
            "tools/list" => Ok(json!({
                "tools": [
                    {
                        "name": "echo_text",
                        "description": "Echo input text",
                        "inputSchema": { "type": "object" }
                    }
                ]
            })),
            "tools/call" => Ok(json!({
                "content": [ { "type": "text", "text": "echoed" } ]
            })),
            _ => Ok(json!({})),
        });

        let client = Arc::new(McpClient::mock(config, handler));
        client.initialize().await.unwrap();

        let tool_defs = client.list_tools().await.unwrap();
        let tool: DynTool = Arc::new(McpTool::new(client, tool_defs[0].clone(), None));

        let mut registry = ToolRegistry::new();
        registry.register(tool);

        assert!(registry.contains("echo_text"));
        let ctx = ToolContext::default();
        let output = registry.execute("echo_text", json!({}), &ctx).await.unwrap();
        assert_eq!(output, "echoed");
    }

    #[tokio::test]
    async fn test_tools_list_pagination() {
        let config = McpServerConfig::new("paginated_server", "mock_cmd");
        let handler: MockHandler = Arc::new(|method, params| match method.as_str() {
            "initialize" => Ok(json!({
                "protocolVersion": "2024-11-05",
                "capabilities": { "tools": {} },
                "serverInfo": { "name": "paginated", "version": "1.0" }
            })),
            "tools/list" => {
                let cursor = params
                    .as_ref()
                    .and_then(|p| p.get("cursor"))
                    .and_then(|c| c.as_str());
                if cursor.is_none() {
                    Ok(json!({
                        "tools": [
                            {
                                "name": "tool_page_1",
                                "description": "First page tool",
                                "inputSchema": { "type": "object" }
                            }
                        ],
                        "nextCursor": "page_2"
                    }))
                } else if cursor == Some("page_2") {
                    Ok(json!({
                        "tools": [
                            {
                                "name": "tool_page_2",
                                "description": "Second page tool",
                                "inputSchema": { "type": "object" }
                            }
                        ],
                        "nextCursor": null
                    }))
                } else {
                    Err(McpError::JsonRpc {
                        code: INVALID_PARAMS,
                        message: "Invalid cursor".to_string(),
                        data: None,
                    })
                }
            }
            _ => Ok(json!({})),
        });

        let client = Arc::new(McpClient::mock(config, handler));
        client.initialize().await.unwrap();

        let tools = client.list_tools().await.unwrap();
        assert_eq!(tools.len(), 2);
        assert_eq!(tools[0].name, "tool_page_1");
        assert_eq!(tools[1].name, "tool_page_2");
    }

    #[test]
    fn test_format_tool_call_result_resource() {
        let res = McpCallToolResult {
            content: vec![McpContent::Resource {
                resource: json!({
                    "uri": "file:///workspace/README.md",
                    "mimeType": "text/markdown",
                    "text": "# Project Readme"
                }),
            }],
            is_error: None,
        };

        let formatted = format_tool_call_result(&res).unwrap();
        assert!(formatted.contains("file:///workspace/README.md"));
        assert!(formatted.contains("text/markdown"));
    }

    #[tokio::test]
    async fn test_mcp_client_ping_and_notifications() {
        let config = McpServerConfig::new("ping_server", "mock_cmd");
        let handler: MockHandler = Arc::new(|method, _params| match method.as_str() {
            "ping" => Ok(json!({})),
            _ => Ok(json!({})),
        });

        let client = McpClient::mock(config, handler);
        assert!(client.ping().await.is_ok());
        assert!(client
            .send_notification("notifications/message", Some(json!({"level": "info"})))
            .await
            .is_ok());
    }
}
