use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tokio::io::{AsyncBufRead, AsyncBufReadExt, AsyncWrite, AsyncWriteExt};
use tokio::sync::mpsc::{unbounded_channel, UnboundedReceiver, UnboundedSender};
use tokio::sync::{watch, Mutex, RwLock};

use crate::acp::events::AcpEventBridge;
use crate::acp::types::*;
use crate::agent::advisor::AdvisorRegistry;
use crate::agent::loop_runner::{AgentEvent, AgentRunner};
use crate::agent::session::Session;
use crate::config::Config;
use crate::provider::LlmClient;
use crate::tools::types::{ToolContext, ToolRegistry};

/// The ACP (Agent Client Protocol) JSON-RPC 2.0 Server.
#[derive(Clone)]
pub struct AcpServer {
    config: Arc<RwLock<Config>>,
    client: LlmClient,
    tools: ToolRegistry,
    tool_ctx: ToolContext,
    advisors: AdvisorRegistry,
    sessions: Arc<RwLock<HashMap<String, Arc<Mutex<Session>>>>>,
    cancellations: Arc<RwLock<HashMap<String, watch::Sender<bool>>>>,
    initialized: Arc<AtomicBool>,
    client_capabilities: Arc<RwLock<Option<ClientCapabilities>>>,
    client_info: Arc<RwLock<Option<ClientInfo>>>,
}

impl AcpServer {
    /// Creates a new ACP server instance with the given configuration and tools.
    pub fn new(
        config: Config,
        client: LlmClient,
        tools: ToolRegistry,
        tool_ctx: ToolContext,
    ) -> Self {
        Self {
            config: Arc::new(RwLock::new(config)),
            client,
            tools,
            tool_ctx,
            advisors: AdvisorRegistry::default_advisors(),
            sessions: Arc::new(RwLock::new(HashMap::new())),
            cancellations: Arc::new(RwLock::new(HashMap::new())),
            initialized: Arc::new(AtomicBool::new(false)),
            client_capabilities: Arc::new(RwLock::new(None)),
            client_info: Arc::new(RwLock::new(None)),
        }
    }

    /// Creates a new ACP server instance from an existing `AgentRunner`.
    pub fn from_runner(runner: &AgentRunner) -> Self {
        Self {
            config: Arc::new(RwLock::new(runner.config().clone())),
            client: runner.client().clone(),
            tools: runner.tools().clone(),
            tool_ctx: runner.tool_ctx().clone(),
            advisors: runner.advisors().clone(),
            sessions: Arc::new(RwLock::new(HashMap::new())),
            cancellations: Arc::new(RwLock::new(HashMap::new())),
            initialized: Arc::new(AtomicBool::new(false)),
            client_capabilities: Arc::new(RwLock::new(None)),
            client_info: Arc::new(RwLock::new(None)),
        }
    }

    /// Configures custom advisors for the ACP server.
    pub fn with_advisors(mut self, advisors: AdvisorRegistry) -> Self {
        self.advisors = advisors;
        self
    }

    /// Returns whether the client has completed the `initialize` handshake.
    pub fn is_initialized(&self) -> bool {
        self.initialized.load(Ordering::SeqCst)
    }

    /// Runs the ACP server over standard I/O (stdin/stdout).
    pub async fn run_stdio(&self) -> anyhow::Result<()> {
        let stdin = tokio::io::BufReader::new(tokio::io::stdin());
        let stdout = tokio::io::stdout();
        self.run_stream(stdin, stdout).await
    }

    /// Runs the ACP server over any generic asynchronous reader and writer streams.
    pub async fn run_stream<R, W>(&self, mut reader: R, mut writer: W) -> anyhow::Result<()>
    where
        R: AsyncBufRead + Unpin + Send,
        W: AsyncWrite + Unpin + Send + 'static,
    {
        let (out_tx, mut out_rx): (UnboundedSender<String>, UnboundedReceiver<String>) =
            unbounded_channel();

        // Writer task to serialize all outgoing JSON-RPC lines
        let writer_handle = tokio::spawn(async move {
            while let Some(line) = out_rx.recv().await {
                if let Err(e) = writer.write_all(line.as_bytes()).await {
                    tracing::error!("Failed to write ACP response: {}", e);
                    break;
                }
                if let Err(e) = writer.write_all(b"\n").await {
                    tracing::error!("Failed to write ACP newline: {}", e);
                    break;
                }
                let _ = writer.flush().await;
            }
        });

        let mut line_buf = String::new();
        loop {
            line_buf.clear();
            let bytes_read = match reader.read_line(&mut line_buf).await {
                Ok(n) => n,
                Err(e) => {
                    tracing::error!("ACP read error: {}", e);
                    break;
                }
            };

            if bytes_read == 0 {
                // EOF reached
                break;
            }

            let trimmed = line_buf.trim();
            if trimmed.is_empty() {
                continue;
            }

            let server = self.clone();
            let out_tx_clone = out_tx.clone();
            let raw_msg = trimmed.to_string();

            tokio::spawn(async move {
                server.process_raw_message(&raw_msg, out_tx_clone).await;
            });
        }

        drop(out_tx);
        let _ = writer_handle.await;
        Ok(())
    }

    /// Parses and processes a single raw JSON-RPC string message.
    ///
    /// Accepts either a single request/notification object or a JSON-RPC 2.0 batch array.
    /// Malformed JSON yields a single `parse_error` response with a null id.
    pub async fn process_raw_message(&self, raw: &str, out_tx: UnboundedSender<String>) {
        let value: serde_json::Value = match serde_json::from_str(raw) {
            Ok(v) => v,
            Err(e) => {
                let error_resp = JsonRpcResponse::error(
                    RequestId::Null,
                    JsonRpcError::parse_error(e.to_string()),
                );
                if let Ok(json_str) = serde_json::to_string(&error_resp) {
                    let _ = out_tx.send(json_str);
                }
                return;
            }
        };

        match value {
            serde_json::Value::Array(items) => {
                if items.is_empty() {
                    let error_resp = JsonRpcResponse::error(
                        RequestId::Null,
                        JsonRpcError::invalid_request("Empty JSON-RPC batch"),
                    );
                    if let Ok(json_str) = serde_json::to_string(&error_resp) {
                        let _ = out_tx.send(json_str);
                    }
                } else {
                    for item in items {
                        self.process_json_message(item, &out_tx).await;
                    }
                }
            }
            other => self.process_json_message(other, &out_tx).await,
        }
    }

    /// Processes a single parsed JSON-RPC message value (request or notification).
    ///
    /// Valid JSON that fails structural validation responds with `invalid_request`,
    /// echoing the original request id whenever it can be recovered.
    async fn process_json_message(
        &self,
        value: serde_json::Value,
        out_tx: &UnboundedSender<String>,
    ) {
        // Recover the request id so clients can correlate error responses.
        let echoed_id = value
            .get("id")
            .filter(|v| !v.is_null())
            .and_then(|v| serde_json::from_value::<RequestId>(v.clone()).ok());

        let request: JsonRpcRequest = match serde_json::from_value(value) {
            Ok(req) => req,
            Err(e) => {
                let error_resp = JsonRpcResponse::error(
                    echoed_id.unwrap_or(RequestId::Null),
                    JsonRpcError::invalid_request(e.to_string()),
                );
                if let Ok(json_str) = serde_json::to_string(&error_resp) {
                    let _ = out_tx.send(json_str);
                }
                return;
            }
        };

        if request.jsonrpc != "2.0" {
            if let Some(id) = request.id {
                let error_resp = JsonRpcResponse::error(
                    id,
                    JsonRpcError::invalid_request(
                        "Missing or invalid 'jsonrpc' version; must be '2.0'",
                    ),
                );
                if let Ok(json_str) = serde_json::to_string(&error_resp) {
                    let _ = out_tx.send(json_str);
                }
            }
            return;
        }

        let is_notification = request.is_notification();
        let maybe_id = request.id.clone();
        let method = request.method.as_str();

        match self
            .dispatch_method(method, request.params, out_tx.clone())
            .await
        {
            Ok(maybe_result) => {
                if !is_notification {
                    if let Some(id) = maybe_id {
                        let response = JsonRpcResponse::success(id, maybe_result);
                        if let Ok(json_str) = serde_json::to_string(&response) {
                            let _ = out_tx.send(json_str);
                        }
                    }
                }
            }
            Err(rpc_err) => {
                if !is_notification {
                    if let Some(id) = maybe_id {
                        let response = JsonRpcResponse::error(id, rpc_err);
                        if let Ok(json_str) = serde_json::to_string(&response) {
                            let _ = out_tx.send(json_str);
                        }
                    }
                } else {
                    tracing::warn!("Notification {} failed: {:?}", method, rpc_err);
                }
            }
        }
    }

    /// Dispatches an ACP JSON-RPC method to the appropriate handler.
    pub async fn dispatch_method(
        &self,
        method: &str,
        params: Option<serde_json::Value>,
        out_tx: UnboundedSender<String>,
    ) -> Result<serde_json::Value, JsonRpcError> {
        match method {
            // Protocol Handshake
            "initialize" => self.handle_initialize(params).await,
            "initialized" => {
                // Handshake acknowledged by client
                Ok(serde_json::Value::Null)
            }
            "ping" => Ok(serde_json::json!({ "pong": true })),

            // Session Lifecycle
            "session/new" => self.handle_session_new(params).await,
            "session/load" => self.handle_session_load(params).await,
            "session/list" => self.handle_session_list(params).await,
            "session/close" => self.handle_session_close(params).await,
            "session/cancel" => self.handle_session_cancel(params).await,

            // Prompt Dispatching
            "session/prompt" => self.handle_session_prompt(params, out_tx).await,
            "models/list" => self.handle_models_list().await,

            _ => Err(JsonRpcError::method_not_found(method)),
        }
    }

    // ========================================================================
    // Handlers
    // ========================================================================

    async fn handle_initialize(
        &self,
        params: Option<serde_json::Value>,
    ) -> Result<serde_json::Value, JsonRpcError> {
        let req: InitializeRequest = match params {
            Some(v) => serde_json::from_value(v).map_err(|e| {
                JsonRpcError::invalid_params(format!("Invalid initialize params: {}", e))
            })?,
            None => {
                return Err(JsonRpcError::invalid_params("Missing initialize params"));
            }
        };

        // Record client capabilities & info
        *self.client_capabilities.write().await = Some(req.client_capabilities);
        *self.client_info.write().await = req.client_info;
        self.initialized.store(true, Ordering::SeqCst);

        let result = InitializeResult {
            protocol_version: PROTOCOL_VERSION,
            agent_capabilities: AgentCapabilities::default(),
            agent_info: AgentInfo::default(),
            auth_methods: Vec::new(),
        };

        serde_json::to_value(result).map_err(|e| JsonRpcError::internal_error(e.to_string()))
    }

    async fn handle_session_new(
        &self,
        params: Option<serde_json::Value>,
    ) -> Result<serde_json::Value, JsonRpcError> {
        let req: NewSessionRequest = if let Some(v) = params {
            serde_json::from_value(v).unwrap_or_default()
        } else {
            NewSessionRequest::default()
        };

        let cfg = self.config.read().await;
        let active_model = req.model.unwrap_or_else(|| cfg.default_model.clone());

        let session = Session::new(&active_model);
        let session_id = session.id.to_string();

        let models = vec![ModelInfo {
            id: cfg.default_model.clone(),
            name: cfg.default_model.clone(),
            provider: cfg.default_provider.clone(),
            is_default: true,
        }];

        self.sessions
            .write()
            .await
            .insert(session_id.clone(), Arc::new(Mutex::new(session)));

        let result = NewSessionResult {
            session_id,
            models: Some(models),
        };

        serde_json::to_value(result).map_err(|e| JsonRpcError::internal_error(e.to_string()))
    }

    async fn handle_session_load(
        &self,
        params: Option<serde_json::Value>,
    ) -> Result<serde_json::Value, JsonRpcError> {
        let req: LoadSessionRequest = match params {
            Some(v) => serde_json::from_value(v).map_err(|e| {
                JsonRpcError::invalid_params(format!("Invalid load session params: {}", e))
            })?,
            None => return Err(JsonRpcError::invalid_params("Missing session_id")),
        };

        // Check in-memory first
        {
            let sessions = self.sessions.read().await;
            if let Some(s) = sessions.get(&req.session_id) {
                let guard = s.lock().await;
                let result = LoadSessionResult {
                    session_id: guard.id.to_string(),
                    active_model: guard.active_model.clone(),
                    message_count: guard.messages.len(),
                    title: guard.title.clone(),
                };
                return serde_json::to_value(result)
                    .map_err(|e| JsonRpcError::internal_error(e.to_string()));
            }
        }

        // Attempt loading from storage
        let uuid = uuid::Uuid::parse_str(&req.session_id)
            .map_err(|_| JsonRpcError::session_not_found(&req.session_id))?;

        let loaded =
            Session::load(uuid).map_err(|_| JsonRpcError::session_not_found(&req.session_id))?;

        let result = LoadSessionResult {
            session_id: loaded.id.to_string(),
            active_model: loaded.active_model.clone(),
            message_count: loaded.messages.len(),
            title: loaded.title.clone(),
        };

        self.sessions
            .write()
            .await
            .insert(req.session_id.clone(), Arc::new(Mutex::new(loaded)));

        serde_json::to_value(result).map_err(|e| JsonRpcError::internal_error(e.to_string()))
    }

    async fn handle_session_list(
        &self,
        _params: Option<serde_json::Value>,
    ) -> Result<serde_json::Value, JsonRpcError> {
        let mut list = Vec::new();

        // Collect in-memory sessions
        {
            let sessions = self.sessions.read().await;
            for (id, session_arc) in sessions.iter() {
                let guard = session_arc.lock().await;
                let preview = guard
                    .messages
                    .last()
                    .map(|m| m.content.chars().take(80).collect::<String>())
                    .unwrap_or_default();

                list.push(SessionSummaryItem {
                    session_id: id.clone(),
                    created_at: guard.created_at.clone(),
                    updated_at: guard.updated_at.clone(),
                    model: guard.active_model.clone(),
                    message_count: guard.messages.len(),
                    preview,
                    title: guard.title.clone(),
                });
            }
        }

        // Merge disk sessions if not already present
        if let Ok(disk_summaries) = Session::list_sessions() {
            for ds in disk_summaries {
                let sid = ds.id.to_string();
                if !list.iter().any(|item| item.session_id == sid) {
                    list.push(SessionSummaryItem {
                        session_id: sid,
                        created_at: ds.created_at,
                        updated_at: ds.updated_at,
                        model: ds.active_model,
                        message_count: ds.message_count,
                        preview: ds.preview,
                        title: ds.title,
                    });
                }
            }
        }

        let result = ListSessionsResult { sessions: list };
        serde_json::to_value(result).map_err(|e| JsonRpcError::internal_error(e.to_string()))
    }

    async fn handle_session_close(
        &self,
        params: Option<serde_json::Value>,
    ) -> Result<serde_json::Value, JsonRpcError> {
        let req: CloseSessionRequest = match params {
            Some(v) => serde_json::from_value(v).map_err(|e| {
                JsonRpcError::invalid_params(format!("Invalid close session params: {}", e))
            })?,
            None => return Err(JsonRpcError::invalid_params("Missing session_id")),
        };

        self.sessions.write().await.remove(&req.session_id);
        self.cancellations.write().await.remove(&req.session_id);

        Ok(serde_json::json!({ "success": true }))
    }

    async fn handle_session_cancel(
        &self,
        params: Option<serde_json::Value>,
    ) -> Result<serde_json::Value, JsonRpcError> {
        let req: CancelSessionRequest = match params {
            Some(v) => serde_json::from_value(v).map_err(|e| {
                JsonRpcError::invalid_params(format!("Invalid cancel session params: {}", e))
            })?,
            None => return Err(JsonRpcError::invalid_params("Missing session_id")),
        };

        let cancellations = self.cancellations.read().await;
        if let Some(tx) = cancellations.get(&req.session_id) {
            let _ = tx.send(true);
        }

        Ok(serde_json::json!({
            "cancelled": true,
            "sessionId": req.session_id
        }))
    }

    async fn handle_models_list(&self) -> Result<serde_json::Value, JsonRpcError> {
        let cfg = self.config.read().await;
        let models = vec![
            ModelInfo {
                id: cfg.default_model.clone(),
                name: cfg.default_model.clone(),
                provider: cfg.default_provider.clone(),
                is_default: true,
            },
            ModelInfo {
                id: "deepseek-chat".to_string(),
                name: "DeepSeek V3".to_string(),
                provider: "deepseek".to_string(),
                is_default: cfg.default_model == "deepseek-chat",
            },
            ModelInfo {
                id: "claude-3-5-sonnet-20241022".to_string(),
                name: "Claude 3.5 Sonnet".to_string(),
                provider: "anthropic".to_string(),
                is_default: cfg.default_model == "claude-3-5-sonnet-20241022",
            },
            ModelInfo {
                id: "gpt-4o".to_string(),
                name: "GPT-4o".to_string(),
                provider: "openai".to_string(),
                is_default: cfg.default_model == "gpt-4o",
            },
        ];

        Ok(serde_json::json!({ "models": models }))
    }

    async fn handle_session_prompt(
        &self,
        params: Option<serde_json::Value>,
        out_tx: UnboundedSender<String>,
    ) -> Result<serde_json::Value, JsonRpcError> {
        let req: PromptRequest = match params {
            Some(v) => serde_json::from_value(v).map_err(|e| {
                JsonRpcError::invalid_params(format!("Invalid prompt params: {}", e))
            })?,
            None => return Err(JsonRpcError::invalid_params("Missing prompt params")),
        };

        let user_prompt = req.prompt.to_text();
        if user_prompt.trim().is_empty() {
            return Err(JsonRpcError::invalid_params(
                "Prompt content cannot be empty",
            ));
        }

        // Get or create session
        let session_arc = {
            let default_model = self.config.read().await.default_model.clone();
            let mut sessions = self.sessions.write().await;
            sessions
                .entry(req.session_id.clone())
                .or_insert_with(|| Arc::new(Mutex::new(Session::new(&default_model))))
                .clone()
        };

        // Setup cancellation token
        let (cancel_tx, mut cancel_rx) = watch::channel(false);
        {
            self.cancellations
                .write()
                .await
                .insert(req.session_id.clone(), cancel_tx);
        }

        let (event_tx, mut event_rx) = unbounded_channel::<AgentEvent>();

        // Build runner
        let runner = {
            let cfg = self.config.read().await.clone();
            AgentRunner::new(
                self.client.clone(),
                cfg,
                self.tools.clone(),
                self.tool_ctx.clone(),
            )
            .with_advisors(self.advisors.clone())
        };

        // Stream bridge task: converts AgentEvent to rich ACP session/update notifications
        let bridge = AcpEventBridge::new(&req.session_id).with_out_sender(out_tx.clone());
        let stream_bridge = tokio::spawn(bridge.run(event_rx));

        // Execute runner with cancellation watch
        let mut session_guard = session_arc.lock().await;
        let mut stop_reason = StopReason::EndTurn;

        tokio::select! {
            result = runner.run_turn_stream(&mut *session_guard, &user_prompt, event_tx) => {
                match result {
                    Ok(_) => {
                        stop_reason = StopReason::EndTurn;
                    }
                    Err(e) => {
                        tracing::error!("ACP prompt execution error: {}", e);
                        stop_reason = StopReason::Error;
                    }
                }
            }
            _ = cancel_rx.changed() => {
                if *cancel_rx.borrow() {
                    stop_reason = StopReason::Cancelled;
                }
            }
        }

        // Clean up cancellation token
        {
            self.cancellations.write().await.remove(&req.session_id);
        }

        let summary = stream_bridge.await.unwrap_or_default();

        let response = PromptResponse {
            stop_reason,
            content: Some(vec![ContentBlock::text(summary.full_assistant_text)]),
            stats: Some(TokenStatsInfo {
                prompt_tokens: Some(summary.prompt_tokens as u32),
                completion_tokens: Some(summary.completion_tokens as u32),
                total_tokens: Some(summary.total_tokens as u32),
            }),
        };

        serde_json::to_value(response).map_err(|e| JsonRpcError::internal_error(e.to_string()))
    }
}

// ============================================================================
// Unit Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use tokio::sync::mpsc::unbounded_channel;

    fn test_server() -> AcpServer {
        AcpServer::new(
            Config::default(),
            LlmClient::new(),
            ToolRegistry::new(),
            ToolContext::default(),
        )
    }

    async fn next_response(out_rx: &mut UnboundedReceiver<String>) -> JsonRpcResponse {
        let line = out_rx.recv().await.expect("Expected a JSON-RPC line");
        serde_json::from_str(&line).expect("Valid JSON-RPC response")
    }

    #[tokio::test]
    async fn test_ping_roundtrip() {
        let server = test_server();
        let (out_tx, mut out_rx) = unbounded_channel();

        server
            .process_raw_message(
                &json!({ "jsonrpc": "2.0", "id": 7, "method": "ping" }).to_string(),
                out_tx,
            )
            .await;

        let resp = next_response(&mut out_rx).await;
        assert_eq!(resp.jsonrpc, "2.0");
        assert_eq!(resp.id, RequestId::Number(7));
        assert_eq!(resp.result.unwrap(), json!({ "pong": true }));
    }

    #[tokio::test]
    async fn test_malformed_json_yields_parse_error() {
        let server = test_server();
        let (out_tx, mut out_rx) = unbounded_channel();

        server.process_raw_message("not valid json", out_tx).await;

        let resp = next_response(&mut out_rx).await;
        assert_eq!(resp.id, RequestId::Null);
        let err = resp.error.expect("Expected parse error");
        assert_eq!(err.code, error_codes::PARSE_ERROR);
    }

    #[tokio::test]
    async fn test_missing_method_is_invalid_request() {
        let server = test_server();
        let (out_tx, mut out_rx) = unbounded_channel();

        server
            .process_raw_message(&json!({ "jsonrpc": "2.0", "id": 5 }).to_string(), out_tx)
            .await;

        let resp = next_response(&mut out_rx).await;
        assert_eq!(resp.id, RequestId::Number(5));
        assert_eq!(resp.error.unwrap().code, error_codes::INVALID_REQUEST);
    }

    #[tokio::test]
    async fn test_wrong_jsonrpc_version_responds_invalid_request() {
        let server = test_server();
        let (out_tx, mut out_rx) = unbounded_channel();

        server
            .process_raw_message(
                &json!({ "jsonrpc": "1.0", "id": 3, "method": "ping" }).to_string(),
                out_tx,
            )
            .await;

        let resp = next_response(&mut out_rx).await;
        assert_eq!(resp.id, RequestId::Number(3));
        assert_eq!(resp.error.unwrap().code, error_codes::INVALID_REQUEST);
    }

    #[tokio::test]
    async fn test_unknown_method_responds_method_not_found() {
        let server = test_server();
        let (out_tx, mut out_rx) = unbounded_channel();

        server
            .process_raw_message(
                &json!({ "jsonrpc": "2.0", "id": 99, "method": "no/such" }).to_string(),
                out_tx,
            )
            .await;

        let resp = next_response(&mut out_rx).await;
        assert_eq!(resp.error.unwrap().code, error_codes::METHOD_NOT_FOUND);
    }

    #[tokio::test]
    async fn test_notification_ping_produces_no_response() {
        let server = test_server();
        let (out_tx, mut out_rx) = unbounded_channel();

        server
            .process_raw_message(
                &json!({ "jsonrpc": "2.0", "method": "ping" }).to_string(),
                out_tx,
            )
            .await;

        assert!(
            out_rx.try_recv().is_err(),
            "Notifications must not yield a JSON-RPC response"
        );
    }

    #[tokio::test]
    async fn test_batch_dispatch_processes_every_request() {
        let server = test_server();
        let (out_tx, mut out_rx) = unbounded_channel();

        let batch = json!([
            { "jsonrpc": "2.0", "id": 1, "method": "ping" },
            { "jsonrpc": "2.0", "id": 2, "method": "no/such" }
        ]);
        server.process_raw_message(&batch.to_string(), out_tx).await;

        let first = next_response(&mut out_rx).await;
        assert_eq!(first.id, RequestId::Number(1));
        assert!(first.error.is_none());

        let second = next_response(&mut out_rx).await;
        assert_eq!(second.id, RequestId::Number(2));
        assert_eq!(second.error.unwrap().code, error_codes::METHOD_NOT_FOUND);
    }

    #[tokio::test]
    async fn test_empty_batch_yields_invalid_request() {
        let server = test_server();
        let (out_tx, mut out_rx) = unbounded_channel();

        server
            .process_raw_message(&json!([]).to_string(), out_tx)
            .await;

        let resp = next_response(&mut out_rx).await;
        assert_eq!(resp.id, RequestId::Null);
        assert_eq!(resp.error.unwrap().code, error_codes::INVALID_REQUEST);
    }

    #[tokio::test]
    async fn test_invalid_id_type_echoed_in_error_response() {
        let server = test_server();
        let (out_tx, mut out_rx) = unbounded_channel();

        // `method` missing so structural validation fails, but `id` is recoverable.
        let raw = json!({ "jsonrpc": "2.0", "id": "echo-1", "extra": true }).to_string();
        server.process_raw_message(&raw, out_tx).await;

        let resp = next_response(&mut out_rx).await;
        assert_eq!(resp.id, RequestId::String("echo-1".to_string()));
        assert_eq!(resp.error.unwrap().code, error_codes::INVALID_REQUEST);
    }

    #[tokio::test]
    async fn test_initialize_params_missing_rejected() {
        let server = test_server();
        let (out_tx, mut out_rx) = unbounded_channel();

        server
            .process_raw_message(
                &json!({ "jsonrpc": "2.0", "id": 1, "method": "initialize" }).to_string(),
                out_tx,
            )
            .await;

        let resp = next_response(&mut out_rx).await;
        assert_eq!(resp.error.unwrap().code, error_codes::INVALID_PARAMS);
    }

    #[tokio::test]
    async fn test_session_cancel_response() {
        let server = test_server();
        let (out_tx, mut out_rx) = unbounded_channel();

        server
            .process_raw_message(
                &json!({
                    "jsonrpc": "2.0",
                    "id": 10,
                    "method": "session/cancel",
                    "params": { "sessionId": "sess-123" }
                })
                .to_string(),
                out_tx,
            )
            .await;

        let resp = next_response(&mut out_rx).await;
        assert!(resp.error.is_none());
        assert_eq!(
            resp.result.unwrap(),
            json!({
                "cancelled": true,
                "sessionId": "sess-123"
            })
        );
    }

    #[tokio::test]
    async fn test_session_close_response() {
        let server = test_server();
        let (out_tx, mut out_rx) = unbounded_channel();

        server
            .process_raw_message(
                &json!({
                    "jsonrpc": "2.0",
                    "id": 11,
                    "method": "session/close",
                    "params": { "sessionId": "sess-123" }
                })
                .to_string(),
                out_tx,
            )
            .await;

        let resp = next_response(&mut out_rx).await;
        assert!(resp.error.is_none());
        assert_eq!(
            resp.result.unwrap(),
            json!({
                "success": true
            })
        );
    }
}
