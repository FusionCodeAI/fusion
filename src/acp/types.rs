use serde::{Deserialize, Serialize};

/// The standard Agent Client Protocol version implemented by Fusion.
pub const PROTOCOL_VERSION: u32 = 1;

// ============================================================================
// JSON-RPC 2.0 Base Types
// ============================================================================

/// JSON-RPC 2.0 Request Identifier (number, string, or null).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(untagged)]
pub enum RequestId {
    Number(i64),
    String(String),
    Null,
}

impl std::fmt::Display for RequestId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RequestId::Number(n) => write!(f, "{}", n),
            RequestId::String(s) => write!(f, "{}", s),
            RequestId::Null => write!(f, "null"),
        }
    }
}

impl From<i64> for RequestId {
    fn from(n: i64) -> Self {
        RequestId::Number(n)
    }
}

impl From<u64> for RequestId {
    fn from(n: u64) -> Self {
        RequestId::Number(n as i64)
    }
}

impl From<String> for RequestId {
    fn from(s: String) -> Self {
        RequestId::String(s)
    }
}

impl From<&str> for RequestId {
    fn from(s: &str) -> Self {
        RequestId::String(s.to_string())
    }
}

/// Standard JSON-RPC 2.0 Error Codes.
pub mod error_codes {
    pub const PARSE_ERROR: i64 = -32700;
    pub const INVALID_REQUEST: i64 = -32600;
    pub const METHOD_NOT_FOUND: i64 = -32601;
    pub const INVALID_PARAMS: i64 = -32602;
    pub const INTERNAL_ERROR: i64 = -32603;
    pub const SERVER_NOT_INITIALIZED: i64 = -32002;
    pub const SESSION_NOT_FOUND: i64 = -32001;
    pub const REQUEST_CANCELLED: i64 = -32000;
}

/// JSON-RPC 2.0 Error object.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct JsonRpcError {
    pub code: i64,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<serde_json::Value>,
}

impl JsonRpcError {
    pub fn new(code: i64, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            data: None,
        }
    }

    pub fn with_data(code: i64, message: impl Into<String>, data: serde_json::Value) -> Self {
        Self {
            code,
            message: message.into(),
            data: Some(data),
        }
    }

    pub fn parse_error(details: impl Into<String>) -> Self {
        Self::new(error_codes::PARSE_ERROR, format!("Parse error: {}", details.into()))
    }

    pub fn invalid_request(details: impl Into<String>) -> Self {
        Self::new(error_codes::INVALID_REQUEST, format!("Invalid request: {}", details.into()))
    }

    pub fn method_not_found(method: impl Into<String>) -> Self {
        Self::new(error_codes::METHOD_NOT_FOUND, format!("Method not found: {}", method.into()))
    }

    pub fn invalid_params(details: impl Into<String>) -> Self {
        Self::new(error_codes::INVALID_PARAMS, format!("Invalid params: {}", details.into()))
    }

    pub fn internal_error(details: impl Into<String>) -> Self {
        Self::new(error_codes::INTERNAL_ERROR, format!("Internal error: {}", details.into()))
    }

    pub fn session_not_found(session_id: &str) -> Self {
        Self::new(error_codes::SESSION_NOT_FOUND, format!("Session not found: {}", session_id))
    }

    pub fn not_initialized() -> Self {
        Self::new(error_codes::SERVER_NOT_INITIALIZED, "Server not initialized. Call 'initialize' first.")
    }
}

/// Incoming JSON-RPC 2.0 Message (Request or Notification).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JsonRpcRequest {
    pub jsonrpc: String,
    #[serde(default = "default_null_id")]
    pub id: Option<RequestId>,
    pub method: String,
    #[serde(default)]
    pub params: Option<serde_json::Value>,
}

fn default_null_id() -> Option<RequestId> {
    None
}

impl JsonRpcRequest {
    pub fn is_notification(&self) -> bool {
        self.id.is_none()
    }
}

/// Outgoing JSON-RPC 2.0 Response.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JsonRpcResponse {
    pub jsonrpc: String,
    pub id: RequestId,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<JsonRpcError>,
}

impl JsonRpcResponse {
    pub fn success(id: RequestId, result: serde_json::Value) -> Self {
        Self {
            jsonrpc: "2.0".to_string(),
            id,
            result: Some(result),
            error: None,
        }
    }

    pub fn error(id: RequestId, error: JsonRpcError) -> Self {
        Self {
            jsonrpc: "2.0".to_string(),
            id,
            result: None,
            error: Some(error),
        }
    }
}

/// Outgoing JSON-RPC 2.0 Notification (e.g. streaming update).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JsonRpcNotification {
    pub jsonrpc: String,
    pub method: String,
    pub params: serde_json::Value,
}

impl JsonRpcNotification {
    pub fn new(method: impl Into<String>, params: serde_json::Value) -> Self {
        Self {
            jsonrpc: "2.0".to_string(),
            method: method.into(),
            params,
        }
    }
}

// ============================================================================
// ACP Handshake Types (`initialize`, `initialized`)
// ============================================================================

/// Parameters for the `initialize` method.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InitializeRequest {
    pub protocol_version: u32,
    #[serde(default)]
    pub client_capabilities: ClientCapabilities,
    #[serde(default)]
    pub client_info: Option<ClientInfo>,
}

/// Capabilities advertised by the client/editor.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ClientCapabilities {
    #[serde(default)]
    pub fs: Option<FsCapabilities>,
    #[serde(default)]
    pub terminal: Option<bool>,
    #[serde(default)]
    pub session: Option<ClientSessionCapabilities>,
}

/// File system capabilities supported by the client.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FsCapabilities {
    #[serde(default)]
    pub read_text_file: Option<bool>,
    #[serde(default)]
    pub write_text_file: Option<bool>,
}

/// Session management capabilities supported by the client.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ClientSessionCapabilities {
    #[serde(default)]
    pub streaming: Option<bool>,
}

/// Information about the connecting client (e.g. Zed, JetBrains, Neovim).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ClientInfo {
    pub name: String,
    #[serde(default)]
    pub version: Option<String>,
}

/// Result returned by the agent for the `initialize` method.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InitializeResult {
    pub protocol_version: u32,
    pub agent_capabilities: AgentCapabilities,
    pub agent_info: AgentInfo,
    #[serde(default)]
    pub auth_methods: Vec<AuthMethod>,
}

/// Capabilities supported by the Fusion agent.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentCapabilities {
    pub load_session: bool,
    pub prompt_capabilities: PromptCapabilities,
    #[serde(default)]
    pub mcp_capabilities: Option<McpCapabilities>,
    #[serde(default)]
    pub terminal: Option<bool>,
}

impl Default for AgentCapabilities {
    fn default() -> Self {
        Self {
            load_session: true,
            prompt_capabilities: PromptCapabilities::default(),
            mcp_capabilities: Some(McpCapabilities { servers: true }),
            terminal: Some(true),
        }
    }
}

/// Prompt and modality capabilities supported by the agent.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PromptCapabilities {
    pub image: bool,
    pub audio: bool,
    pub embedded_resources: bool,
}

impl Default for PromptCapabilities {
    fn default() -> Self {
        Self {
            image: false,
            audio: false,
            embedded_resources: true,
        }
    }
}

/// MCP support capabilities.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct McpCapabilities {
    pub servers: bool,
}

/// Information identifying the Fusion agent.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentInfo {
    pub name: String,
    pub version: String,
    #[serde(default)]
    pub description: Option<String>,
}

impl Default for AgentInfo {
    fn default() -> Self {
        Self {
            name: "fusion".to_string(),
            version: env!("CARGO_PKG_VERSION").to_string(),
            description: Some("Fast, lightweight, pure-Rust AI coding assistant with subagents and advisors".to_string()),
        }
    }
}

/// Authentication methods supported by the agent (none required for local stdio).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AuthMethod {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub description: Option<String>,
}

// ============================================================================
// Session Lifecycle Types (`session/new`, `session/load`, `session/list`, etc.)
// ============================================================================

/// Parameters for creating a new session (`session/new`).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NewSessionRequest {
    #[serde(default)]
    pub cwd: Option<String>,
    #[serde(default)]
    pub mcp_servers: Option<Vec<serde_json::Value>>,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub provider: Option<String>,
}

/// Result returned when a new session is created.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NewSessionResult {
    pub session_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub models: Option<Vec<ModelInfo>>,
}

/// Model descriptor advertised to the client.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelInfo {
    pub id: String,
    pub name: String,
    pub provider: String,
    #[serde(default)]
    pub is_default: bool,
}

/// Parameters for loading/resuming an existing session (`session/load` or `session/resume`).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LoadSessionRequest {
    pub session_id: String,
}

/// Result for loading a session.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LoadSessionResult {
    pub session_id: String,
    pub active_model: String,
    pub message_count: usize,
    #[serde(default)]
    pub title: Option<String>,
}

/// Parameters for listing sessions (`session/list`).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ListSessionsRequest {
    #[serde(default)]
    pub limit: Option<usize>,
}

/// Result for listing sessions.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ListSessionsResult {
    pub sessions: Vec<SessionSummaryItem>,
}

/// Summary item in session list.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionSummaryItem {
    pub session_id: String,
    pub created_at: String,
    pub updated_at: String,
    pub model: String,
    pub message_count: usize,
    pub preview: String,
    #[serde(default)]
    pub title: Option<String>,
}

/// Parameters for closing a session (`session/close`).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CloseSessionRequest {
    pub session_id: String,
}

/// Parameters for cancelling an ongoing turn (`session/cancel`).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CancelSessionRequest {
    pub session_id: String,
}

// ============================================================================
// Prompt Dispatching Types (`session/prompt`, `session/update`)
// ============================================================================

/// Flexible prompt representation accepting plain string, structured content block, or array of blocks.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum PromptInput {
    String(String),
    Blocks(Vec<ContentBlock>),
    Object(ContentBlock),
}

impl PromptInput {
    /// Extracts the full text content from the prompt input.
    pub fn to_text(&self) -> String {
        match self {
            PromptInput::String(s) => s.clone(),
            PromptInput::Blocks(blocks) => blocks
                .iter()
                .filter_map(|b| b.text.as_deref())
                .collect::<Vec<_>>()
                .join("\n"),
            PromptInput::Object(block) => block.text.clone().unwrap_or_default(),
        }
    }
}

/// Content block within an ACP prompt or response.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ContentBlock {
    #[serde(rename = "type", default = "default_content_type")]
    pub content_type: String,
    #[serde(default)]
    pub text: Option<String>,
    #[serde(default)]
    pub data: Option<String>,
    #[serde(default)]
    pub mime_type: Option<String>,
    #[serde(default)]
    pub uri: Option<String>,
}

fn default_content_type() -> String {
    "text".to_string()
}

impl ContentBlock {
    pub fn text(text: impl Into<String>) -> Self {
        Self {
            content_type: "text".to_string(),
            text: Some(text.into()),
            data: None,
            mime_type: None,
            uri: None,
        }
    }
}

/// Request parameters for dispatching a prompt (`session/prompt`).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PromptRequest {
    pub session_id: String,
    pub prompt: PromptInput,
}

/// Reason the agent stopped processing a prompt turn.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StopReason {
    EndTurn,
    MaxTokens,
    MaxTurnRequests,
    Refusal,
    Cancelled,
    Error,
}

/// Token usage metadata returned in prompt responses.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TokenStatsInfo {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prompt_tokens: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub completion_tokens: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub total_tokens: Option<u32>,
}

/// Final response returned after processing a `session/prompt` turn.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PromptResponse {
    pub stop_reason: StopReason,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content: Option<Vec<ContentBlock>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stats: Option<TokenStatsInfo>,
}

// ============================================================================
// Streaming Notifications (`session/update`)
// ============================================================================

/// Parameters for `session/update` notifications.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionUpdateParams {
    pub session_id: String,
    pub update: SessionUpdate,
}

/// Detailed update payload streamed during an agent turn.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum SessionUpdate {
    /// Incremental assistant text chunk.
    AgentMessageChunk {
        content: AgentMessageContent,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        index: Option<u64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        is_first: Option<bool>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        is_last: Option<bool>,
    },
    /// Incremental thinking/reasoning chunk.
    AgentThoughtChunk {
        thought: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        index: Option<u64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        elapsed_ms: Option<u64>,
    },
    /// Tool execution started.
    ToolCall {
        call_id: String,
        name: String,
        args: serde_json::Value,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        status: Option<String>,
    },
    /// Tool execution progress or intermediate status update.
    ToolStatus {
        call_id: String,
        name: String,
        status: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        progress: Option<f32>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        partial_output: Option<String>,
    },
    /// Tool execution completed.
    ToolCallResult {
        call_id: String,
        name: String,
        output: String,
        success: bool,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        duration_ms: Option<u64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        error: Option<String>,
    },
    /// Advisor review started.
    AdvisorStarted {
        advisor: String,
        role: String,
    },
    /// Advisor review feedback.
    AdvisorCritique {
        advisor: String,
        approved: bool,
        critique: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        role: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        severity: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        suggestions: Option<Vec<String>>,
    },
    /// Real-time token usage and speed statistics.
    TokenStats {
        prompt_tokens: u64,
        completion_tokens: u64,
        total_tokens: u64,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        cached_tokens: Option<u64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        tokens_per_second: Option<f64>,
    },
    /// Status message or progress indicator.
    Status {
        message: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        level: Option<String>,
    },
    /// Execution plan update.
    Plan {
        steps: Vec<String>,
    },
    /// Subagent execution status update.
    SubagentUpdate {
        name: String,
        status: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        task: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        output: Option<String>,
    },
}

/// Structured content wrapper for agent message chunks.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentMessageContent {
    pub role: String,
    pub content: Vec<ContentBlock>,
}

impl AgentMessageContent {
    pub fn assistant_text(text: impl Into<String>) -> Self {
        Self {
            role: "assistant".to_string(),
            content: vec![ContentBlock::text(text)],
        }
    }
}
