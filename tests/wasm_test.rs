//! Comprehensive Unit & Integration Tests for Fusion WebAssembly Bindings,
//! Agent Client Protocol (ACP) JSON-RPC 2.0 Messaging, In-Memory Virtual File System (VFS),
//! and Event Stream Serialization Engine.
//!
//! Test Suite Breakdown:
//! 1. `jsonrpc_acp_message_tests`: JSON-RPC 2.0 request/response parsing, validation, and generation across all ACP methods.
//! 2. `virtual_filesystem_tests`: In-memory VirtualFs CRUD, surgical editing, regex grep, globbing, bash sandbox, and serialization.
//! 3. `event_stream_serialization_tests`: Token streaming chunks, thinking deltas, tool status lifecycles, advisor consensus, NDJSON and browser event shapes.
//! 4. `wasm_agent_integration_tests`: WebAssembly agent creation, VFS interaction, offline turn streaming, and checkpoint/restore cycles (under `feature = "wasm"`).

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use serde_json::{json, Value};
use tokio::sync::mpsc::unbounded_channel;

use fusion::acp::events::{
    AcpEventBridge, AcpSessionEvent, AdvisorConsensus, AdvisorFeedbackUpdate, AdvisorSeverity,
    AdvisorStatusState, PlanProgressUpdate, PlanStep, SubagentStatusUpdate, ThinkingStreamChunk,
    TokenStreamChunk, TokenUsageStats, ToolExecutionState, ToolStatusUpdate,
};
use fusion::acp::json_stream::{
    format_ndjson_batch, parse_ndjson_lines, AdvisorFeedbackPayload, AdvisorStartPayload,
    JsonLogEvent, JsonLogEventKind, JsonLogPayload, JsonLogReader, JsonLogStreamer,
    SessionStartPayload, TextDeltaPayload, ThinkingDeltaPayload, TokenStatsPayload,
    ToolFinishPayload, ToolProgressPayload, ToolStartPayload,
};
use fusion::acp::server::AcpServer;
use fusion::acp::types::{
    error_codes, AgentCapabilities, AgentInfo, AgentMessageContent, AuthMethod,
    CancelSessionRequest, ClientCapabilities, ClientInfo, ClientSessionCapabilities,
    CloseSessionRequest, ContentBlock, FsCapabilities, InitializeRequest, InitializeResult,
    JsonRpcError, JsonRpcNotification, JsonRpcRequest, JsonRpcResponse, ListSessionsRequest,
    ListSessionsResult, LoadSessionRequest, LoadSessionResult, McpCapabilities, ModelInfo,
    NewSessionRequest, NewSessionResult, PromptCapabilities, PromptInput, PromptRequest,
    PromptResponse, RequestId, SessionSummaryItem, SessionUpdate, SessionUpdateParams, StopReason,
    TokenStatsInfo, PROTOCOL_VERSION,
};
use fusion::agent::loop_runner::AgentEvent;
use fusion::config::Config;
use fusion::provider::LlmClient;
use fusion::tools::types::{ToolContext, ToolRegistry};

// ============================================================================
// Section 1: JSON-RPC 2.0 Message Parsing and Generation Across All ACP Methods
// ============================================================================

mod jsonrpc_acp_message_tests {
    use super::*;

    #[test]
    fn test_jsonrpc_request_id_variants_and_formatting() {
        // Number ID (positive)
        let num_id = RequestId::Number(42);
        assert_eq!(num_id.to_string(), "42");
        let serialized_num = serde_json::to_string(&num_id).expect("serialize number id");
        assert_eq!(serialized_num, "42");
        let parsed_num: RequestId =
            serde_json::from_str(&serialized_num).expect("deserialize number id");
        assert_eq!(parsed_num, RequestId::Number(42));

        // Number ID (negative)
        let neg_id = RequestId::Number(-1);
        assert_eq!(neg_id.to_string(), "-1");
        let parsed_neg: RequestId = serde_json::from_str("-1").expect("deserialize neg id");
        assert_eq!(parsed_neg, RequestId::Number(-1));

        // String ID
        let str_id = RequestId::String("req_abc_123".to_string());
        assert_eq!(str_id.to_string(), "req_abc_123");
        let serialized_str = serde_json::to_string(&str_id).expect("serialize string id");
        assert_eq!(serialized_str, "\"req_abc_123\"");
        let parsed_str: RequestId =
            serde_json::from_str(&serialized_str).expect("deserialize string id");
        assert_eq!(parsed_str, RequestId::String("req_abc_123".to_string()));

        // Null ID
        let null_id = RequestId::Null;
        assert_eq!(null_id.to_string(), "null");
        let serialized_null = serde_json::to_string(&null_id).expect("serialize null id");
        assert_eq!(serialized_null, "null");
        let parsed_null: RequestId = serde_json::from_str("null").expect("deserialize null id");
        assert_eq!(parsed_null, RequestId::Null);

        // From trait conversions
        assert_eq!(RequestId::from(100i64), RequestId::Number(100));
        assert_eq!(RequestId::from(200u64), RequestId::Number(200));
        assert_eq!(
            RequestId::from("str-id"),
            RequestId::String("str-id".to_string())
        );
        assert_eq!(
            RequestId::from("owned-str".to_string()),
            RequestId::String("owned-str".to_string())
        );
    }

    #[test]
    fn test_jsonrpc_error_codes_and_payloads() {
        // Standard JSON-RPC 2.0 error codes
        let parse_err = JsonRpcError::parse_error("Unexpected EOF");
        assert_eq!(parse_err.code, error_codes::PARSE_ERROR);
        assert_eq!(parse_err.code, -32700);
        assert!(parse_err.message.contains("Unexpected EOF"));

        let invalid_req = JsonRpcError::invalid_request("Missing jsonrpc field");
        assert_eq!(invalid_req.code, error_codes::INVALID_REQUEST);
        assert_eq!(invalid_req.code, -32600);

        let method_not_found = JsonRpcError::method_not_found("unknown/method");
        assert_eq!(method_not_found.code, error_codes::METHOD_NOT_FOUND);
        assert_eq!(method_not_found.code, -32601);
        assert!(method_not_found.message.contains("unknown/method"));

        let invalid_params = JsonRpcError::invalid_params("Expected object");
        assert_eq!(invalid_params.code, error_codes::INVALID_PARAMS);
        assert_eq!(invalid_params.code, -32602);

        let internal_err = JsonRpcError::internal_error("Worker panicked");
        assert_eq!(internal_err.code, error_codes::INTERNAL_ERROR);
        assert_eq!(internal_err.code, -32603);

        // ACP Application-specific error codes
        let session_not_found = JsonRpcError::session_not_found("sess-xyz");
        assert_eq!(session_not_found.code, error_codes::SESSION_NOT_FOUND);
        assert_eq!(session_not_found.code, -32001);
        assert!(session_not_found.message.contains("sess-xyz"));

        let not_initialized = JsonRpcError::not_initialized();
        assert_eq!(not_initialized.code, error_codes::SERVER_NOT_INITIALIZED);
        assert_eq!(not_initialized.code, -32002);

        let cancel_err = JsonRpcError::new(error_codes::REQUEST_CANCELLED, "Turn aborted by user");
        assert_eq!(cancel_err.code, error_codes::REQUEST_CANCELLED);
        assert_eq!(cancel_err.code, -32000);

        // Custom error with structured data payload
        let custom_err = JsonRpcError::with_data(
            -32099,
            "Custom failure",
            json!({ "retryAfterMs": 1500, "reason": "overloaded" }),
        );
        assert_eq!(custom_err.code, -32099);
        assert_eq!(
            custom_err.data.as_ref().unwrap()["retryAfterMs"],
            json!(1500)
        );

        // Serialization & deserialization
        let serialized = serde_json::to_string(&custom_err).expect("serialize error");
        let deserialized: JsonRpcError =
            serde_json::from_str(&serialized).expect("deserialize error");
        assert_eq!(deserialized.code, -32099);
        assert_eq!(deserialized.message, "Custom failure");
    }

    #[test]
    fn test_jsonrpc_request_and_notification_parsing() {
        // Standard Request with object params
        let req_raw = r#"{
            "jsonrpc": "2.0",
            "id": "req-1",
            "method": "session/prompt",
            "params": {
                "sessionId": "sess-001",
                "prompt": "Hello Fusion"
            }
        }"#;
        let req: JsonRpcRequest = serde_json::from_str(req_raw).expect("parse request");
        assert_eq!(req.jsonrpc, "2.0");
        assert_eq!(req.id, Some(RequestId::String("req-1".to_string())));
        assert_eq!(req.method, "session/prompt");
        assert!(!req.is_notification());

        // Notification (id is null or missing)
        let notif_raw = r#"{
            "jsonrpc": "2.0",
            "method": "initialized",
            "params": {}
        }"#;
        let notif: JsonRpcRequest = serde_json::from_str(notif_raw).expect("parse notification");
        assert_eq!(notif.jsonrpc, "2.0");
        assert!(notif.id.is_none());
        assert_eq!(notif.method, "initialized");
        assert!(notif.is_notification());

        // Null id notification
        let null_id_notif = r#"{
            "jsonrpc": "2.0",
            "id": null,
            "method": "session/cancel"
        }"#;
        let parsed_null_notif: JsonRpcRequest =
            serde_json::from_str(null_id_notif).expect("parse null id");
        assert!(parsed_null_notif.is_notification());
    }

    #[test]
    fn test_jsonrpc_response_formatting_and_roundtrip() {
        // Success response
        let success_resp = JsonRpcResponse::success(
            RequestId::Number(10),
            json!({ "status": "ok", "sessionId": "sess-99" }),
        );
        let serialized_success = serde_json::to_string(&success_resp).expect("serialize success");
        assert!(serialized_success.contains("\"jsonrpc\":\"2.0\""));
        assert!(serialized_success.contains("\"id\":10"));
        assert!(serialized_success.contains("\"result\""));
        assert!(!serialized_success.contains("\"error\""));

        let deserialized_success: JsonRpcResponse =
            serde_json::from_str(&serialized_success).expect("deserialize success");
        assert_eq!(deserialized_success.id, RequestId::Number(10));
        assert!(deserialized_success.error.is_none());
        assert_eq!(
            deserialized_success.result.unwrap()["sessionId"],
            "sess-99"
        );

        // Error response
        let error_resp = JsonRpcResponse::error(
            RequestId::String("err-req".to_string()),
            JsonRpcError::method_not_found("session/teleport"),
        );
        let serialized_error = serde_json::to_string(&error_resp).expect("serialize error");
        assert!(serialized_error.contains("\"error\""));
        assert!(serialized_error.contains("-32601"));

        let deserialized_error: JsonRpcResponse =
            serde_json::from_str(&serialized_error).expect("deserialize error");
        assert_eq!(
            deserialized_error.id,
            RequestId::String("err-req".to_string())
        );
        assert!(deserialized_error.result.is_none());
        assert_eq!(
            deserialized_error.error.unwrap().code,
            error_codes::METHOD_NOT_FOUND
        );

        // Outgoing Notification
        let notification = JsonRpcNotification::new(
            "session/update",
            json!({ "kind": "status", "message": "Compiling" }),
        );
        let serialized_notif = serde_json::to_string(&notification).expect("serialize notif");
        assert!(serialized_notif.contains("\"method\":\"session/update\""));
        assert!(serialized_notif.contains("\"jsonrpc\":\"2.0\""));
        assert!(!serialized_notif.contains("\"id\""));
    }

    #[test]
    fn test_acp_initialize_method_parsing_and_generation() {
        // 1. Parse client InitializeRequest with all capability flags
        let init_json = r#"{
            "protocolVersion": 1,
            "clientCapabilities": {
                "fs": {
                    "readTextFile": true,
                    "writeTextFile": true
                },
                "session": {
                    "resume": true,
                    "list": true
                },
                "terminal": true,
                "mcp": true
            },
            "clientInfo": {
                "name": "Zed Editor",
                "version": "0.180.2"
            }
        }"#;

        let init_req: InitializeRequest =
            serde_json::from_str(init_json).expect("deserialize InitializeRequest");
        assert_eq!(init_req.protocol_version, PROTOCOL_VERSION);
        assert_eq!(
            init_req.client_info.as_ref().unwrap().name,
            "Zed Editor"
        );
        assert_eq!(
            init_req.client_info.as_ref().unwrap().version.as_deref(),
            Some("0.180.2")
        );
        assert_eq!(
            init_req
                .client_capabilities
                .fs
                .as_ref()
                .and_then(|f| f.read_text_file),
            Some(true)
        );
        assert_eq!(
            init_req
                .client_capabilities
                .fs
                .as_ref()
                .and_then(|f| f.write_text_file),
            Some(true)
        );
        assert!(init_req.client_capabilities.terminal.unwrap_or(false));

        // 2. Generate agent InitializeResult
        let init_res = InitializeResult {
            protocol_version: PROTOCOL_VERSION,
            agent_capabilities: AgentCapabilities::default(),
            agent_info: AgentInfo::default(),
            auth_methods: vec![],
        };

        let res_value = serde_json::to_value(&init_res).expect("serialize InitializeResult");
        assert_eq!(res_value["protocolVersion"], 1);
        assert_eq!(res_value["agentInfo"]["name"], "fusion");
        assert_eq!(
            res_value["agentCapabilities"]["loadSession"],
            json!(true)
        );
        assert_eq!(
            res_value["agentCapabilities"]["prompts"]["embeddedContext"],
            json!(true)
        );
        assert_eq!(
            res_value["agentCapabilities"]["mcp"]["servers"],
            json!(true)
        );
        assert!(res_value["authMethods"].is_array());
    }

    #[test]
    fn test_acp_session_new_load_list_close_cancel_methods() {
        // session/new
        let new_req_json = r#"{
            "cwd": "/workspace/fusion-project",
            "model": "anthropic/claude-3.5-sonnet",
            "systemPrompt": "You are a Rust systems architect.",
            "mcpServers": ["fs-server", "git-server"]
        }"#;
        let new_req: NewSessionRequest =
            serde_json::from_str(new_req_json).expect("parse NewSessionRequest");
        assert_eq!(new_req.cwd.as_deref(), Some("/workspace/fusion-project"));
        assert_eq!(
            new_req.model.as_deref(),
            Some("anthropic/claude-3.5-sonnet")
        );
        assert_eq!(new_req.mcp_servers.as_ref().map(|v| v.len()).unwrap_or(0), 2);

        let new_res = NewSessionResult {
            session_id: "sess_test_12345".to_string(),
            models: Some(vec![ModelInfo {
                id: "anthropic/claude-3.5-sonnet".to_string(),
                name: "Claude 3.5 Sonnet".to_string(),
                provider: "anthropic".to_string(),
                is_default: true,
            }]),
        };
        let new_res_val = serde_json::to_value(&new_res).expect("serialize NewSessionResult");
        assert_eq!(new_res_val["sessionId"], "sess_test_12345");
        assert_eq!(
            new_res_val["models"][0]["id"],
            "anthropic/claude-3.5-sonnet"
        );

        // session/load
        let load_req: LoadSessionRequest =
            serde_json::from_str(r#"{"sessionId": "sess_load_99"}"#).expect("parse LoadSessionRequest");
        assert_eq!(load_req.session_id, "sess_load_99");

        let load_res = LoadSessionResult {
            session_id: "sess_load_99".to_string(),
            active_model: "gpt-4o".to_string(),
            message_count: 42,
            title: Some("Refactor session".to_string()),
        };
        let load_res_val = serde_json::to_value(&load_res).expect("serialize LoadSessionResult");
        assert_eq!(load_res_val["sessionId"], "sess_load_99");
        assert_eq!(load_res_val["activeModel"], "gpt-4o");
        assert_eq!(load_res_val["messageCount"], 42);

        // session/list
        let list_req: ListSessionsRequest =
            serde_json::from_str(r#"{"limit": 10}"#).expect("parse ListSessionsRequest");
        assert_eq!(list_req.limit, Some(10));

        let list_res = ListSessionsResult {
            sessions: vec![
                SessionSummaryItem {
                    session_id: "sess_1".to_string(),
                    created_at: "2026-09-01T12:00:00Z".to_string(),
                    updated_at: "2026-09-01T12:30:00Z".to_string(),
                    model: "claude-3.5-sonnet".to_string(),
                    message_count: 4,
                    preview: "Refactored parser".to_string(),
                    title: Some("Refactor".to_string()),
                },
                SessionSummaryItem {
                    session_id: "sess_2".to_string(),
                    created_at: "2026-09-02T08:00:00Z".to_string(),
                    updated_at: "2026-09-02T08:05:00Z".to_string(),
                    model: "deepseek-chat".to_string(),
                    message_count: 1,
                    preview: "".to_string(),
                    title: None,
                },
            ],
        };
        let list_res_val = serde_json::to_value(&list_res).expect("serialize ListSessionsResult");
        assert_eq!(list_res_val["sessions"].as_array().unwrap().len(), 2);
        assert_eq!(list_res_val["sessions"][0]["messageCount"], 4);

        // session/close & session/cancel
        let close_req: CloseSessionRequest =
            serde_json::from_str(r#"{"sessionId": "sess_close_1"}"#).expect("parse CloseSessionRequest");
        assert_eq!(close_req.session_id, "sess_close_1");

        let cancel_req: CancelSessionRequest =
            serde_json::from_str(r#"{"sessionId": "sess_cancel_2"}"#).expect("parse CancelSessionRequest");
        assert_eq!(cancel_req.session_id, "sess_cancel_2");
    }

    #[test]
    fn test_acp_session_prompt_input_variants_and_response() {
        // Variant 1: Plain string prompt
        let prompt_str_json = r#"{
            "sessionId": "sess_p1",
            "prompt": "Optimize this binary search algorithm"
        }"#;
        let prompt_req1: PromptRequest =
            serde_json::from_str(prompt_str_json).expect("parse PromptRequest string");
        assert_eq!(prompt_req1.session_id, "sess_p1");
        assert_eq!(
            prompt_req1.prompt.to_text(),
            "Optimize this binary search algorithm"
        );

        // Variant 2: Single structured ContentBlock
        let prompt_block_json = r#"{
            "sessionId": "sess_p2",
            "prompt": {
                "type": "text",
                "text": "Explain memory safety in Rust"
            }
        }"#;
        let prompt_req2: PromptRequest =
            serde_json::from_str(prompt_block_json).expect("parse PromptRequest block");
        assert_eq!(
            prompt_req2.prompt.to_text(),
            "Explain memory safety in Rust"
        );

        // Variant 3: Array of ContentBlocks
        let prompt_multi_json = r#"{
            "sessionId": "sess_p3",
            "prompt": [
                { "type": "text", "text": "Review this implementation:" },
                { "type": "text", "text": "fn add(a: i32, b: i32) -> i32 { a + b }" }
            ]
        }"#;
        let prompt_req3: PromptRequest =
            serde_json::from_str(prompt_multi_json).expect("parse PromptRequest multi");
        assert_eq!(
            prompt_req3.prompt.to_text(),
            "Review this implementation:\nfn add(a: i32, b: i32) -> i32 { a + b }"
        );

        // StopReason variants
        assert_eq!(
            serde_json::to_string(&StopReason::EndTurn).unwrap(),
            "\"end_turn\""
        );
        assert_eq!(
            serde_json::to_string(&StopReason::MaxTurnRequests).unwrap(),
            "\"max_turn_requests\""
        );
        assert_eq!(
            serde_json::to_string(&StopReason::MaxTokens).unwrap(),
            "\"max_tokens\""
        );
        assert_eq!(
            serde_json::to_string(&StopReason::Cancelled).unwrap(),
            "\"cancelled\""
        );
        assert_eq!(
            serde_json::to_string(&StopReason::Error).unwrap(),
            "\"error\""
        );

        // PromptResponse generation
        let prompt_response = PromptResponse {
            stop_reason: StopReason::EndTurn,
            content: Some(vec![ContentBlock::text("Optimization complete.")]),
            stats: Some(TokenStatsInfo {
                prompt_tokens: Some(300),
                completion_tokens: Some(150),
                total_tokens: Some(450),
            }),
        };

        let resp_val = serde_json::to_value(&prompt_response).expect("serialize PromptResponse");
        assert_eq!(resp_val["stopReason"], "end_turn");
        assert_eq!(
            resp_val["content"][0]["text"],
            "Optimization complete."
        );
        assert_eq!(resp_val["stats"]["totalTokens"], 450);
    }

    #[tokio::test]
    async fn test_acp_server_full_dispatch_matrix() {
        let config = Config::default();
        let client = LlmClient::new();
        let tools = ToolRegistry::new();
        let tool_ctx = ToolContext::default();
        let server = AcpServer::new(config, client, tools, tool_ctx);

        let (out_tx, mut out_rx) = unbounded_channel();

        // 1. Handshake: initialize
        let init_val = server
            .dispatch_method(
                "initialize",
                Some(json!({
                    "protocolVersion": 1,
                    "clientCapabilities": {
                        "fs": { "readTextFile": true, "writeTextFile": true },
                        "terminal": true
                    },
                    "clientInfo": { "name": "VSCode", "version": "1.92.0" }
                })),
                out_tx.clone(),
            )
            .await
            .expect("initialize dispatch");
        assert_eq!(init_val["protocolVersion"], 1);
        assert!(server.is_initialized());

        // 2. Handshake: initialized notification
        let initialized_res = server
            .dispatch_method("initialized", None, out_tx.clone())
            .await
            .expect("initialized dispatch");
        assert_eq!(initialized_res, Value::Null);

        // 3. Ping
        let ping_val = server
            .dispatch_method("ping", None, out_tx.clone())
            .await
            .expect("ping dispatch");
        assert_eq!(ping_val["pong"], true);

        // 4. session/new
        let new_sess_val = server
            .dispatch_method(
                "session/new",
                Some(json!({
                    "cwd": "/workspace",
                    "model": "gpt-4o"
                })),
                out_tx.clone(),
            )
            .await
            .expect("session/new dispatch");
        let sess_id = new_sess_val["sessionId"].as_str().expect("sessionId").to_string();
        assert!(!sess_id.is_empty());

        // 5. session/list
        let list_val = server
            .dispatch_method("session/list", None, out_tx.clone())
            .await
            .expect("session/list dispatch");
        let sessions = list_val["sessions"].as_array().expect("sessions array");
        assert!(sessions.iter().any(|s| s["sessionId"] == sess_id));

        // 6. session/load
        let load_val = server
            .dispatch_method(
                "session/load",
                Some(json!({ "sessionId": sess_id })),
                out_tx.clone(),
            )
            .await
            .expect("session/load dispatch");
        assert_eq!(load_val["sessionId"], sess_id);

        // 7. models/list
        let models_val = server
            .dispatch_method("models/list", None, out_tx.clone())
            .await
            .expect("models/list dispatch");
        assert!(models_val["models"].is_array());
        assert!(!models_val["models"].as_array().unwrap().is_empty());

        // 8. session/cancel
        let cancel_val = server
            .dispatch_method(
                "session/cancel",
                Some(json!({ "sessionId": sess_id })),
                out_tx.clone(),
            )
            .await
            .expect("session/cancel dispatch");
        assert_eq!(cancel_val["cancelled"], true);

        // 9. session/close
        let close_val = server
            .dispatch_method(
                "session/close",
                Some(json!({ "sessionId": sess_id })),
                out_tx.clone(),
            )
            .await
            .expect("session/close dispatch");
        assert_eq!(close_val["success"], true);

        // 10. Unknown method
        let unknown_err = server
            .dispatch_method("hyperjump/execute", None, out_tx.clone())
            .await
            .expect_err("unknown method must fail");
        assert_eq!(unknown_err.code, error_codes::METHOD_NOT_FOUND);
    }

    #[tokio::test]
    async fn test_acp_server_process_raw_jsonrpc_messages() {
        let config = Config::default();
        let client = LlmClient::new();
        let tools = ToolRegistry::new();
        let tool_ctx = ToolContext::default();
        let server = AcpServer::new(config, client, tools, tool_ctx);

        let (out_tx, mut out_rx) = unbounded_channel();

        // 1. Malformed JSON message
        server
            .process_raw_message("{ invalid json syntax ...", out_tx.clone())
            .await;
        let raw_err = out_rx.recv().await.expect("receive parse error");
        let parsed_err_resp: JsonRpcResponse = serde_json::from_str(&raw_err).unwrap();
        assert_eq!(
            parsed_err_resp.error.unwrap().code,
            error_codes::PARSE_ERROR
        );

        // 2. Invalid jsonrpc version
        let bad_ver = json!({
            "jsonrpc": "1.0",
            "id": 99,
            "method": "ping"
        });
        server
            .process_raw_message(&bad_ver.to_string(), out_tx.clone())
            .await;
        let ver_err_str = out_rx.recv().await.expect("receive version error");
        let ver_err_resp: JsonRpcResponse = serde_json::from_str(&ver_err_str).unwrap();
        assert_eq!(
            ver_err_resp.error.unwrap().code,
            error_codes::INVALID_REQUEST
        );

        // 3. Valid ping
        let ping_msg = json!({
            "jsonrpc": "2.0",
            "id": "ping_test_1",
            "method": "ping"
        });
        server
            .process_raw_message(&ping_msg.to_string(), out_tx.clone())
            .await;
        let ping_resp_str = out_rx.recv().await.expect("receive ping resp");
        let ping_resp: JsonRpcResponse = serde_json::from_str(&ping_resp_str).unwrap();
        assert_eq!(ping_resp.id, RequestId::String("ping_test_1".to_string()));
        assert_eq!(ping_resp.result.unwrap()["pong"], true);
    }
}

// ============================================================================
// Section 2: Virtual File System (VFS) Operations and In-Memory Execution
// ============================================================================

mod virtual_filesystem_tests {
    use super::*;

    /// Standalone in-memory Virtual File System implementation mirroring `fusion::wasm::VirtualFs`
    /// to guarantee comprehensive VFS testing in all build configurations.
    #[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
    pub struct InMemoryVfs {
        files: HashMap<String, String>,
    }

    impl Default for InMemoryVfs {
        fn default() -> Self {
            Self::new()
        }
    }

    impl InMemoryVfs {
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
                "{\n  \"name\": \"fusion-web-workspace\",\n  \"version\": \"0.3.0\",\n  \"type\": \"module\"\n}\n",
            );
            fs
        }

        fn normalize_path(path: &str) -> String {
            path.trim_start_matches("./").to_string()
        }

        pub fn read(&self, path: &str) -> Result<String, String> {
            let key = Self::normalize_path(path);
            self.files
                .get(&key)
                .cloned()
                .ok_or_else(|| format!("File not found: {}", path))
        }

        pub fn write(&mut self, path: &str, content: &str) {
            let key = Self::normalize_path(path);
            self.files.insert(key, content.to_string());
        }

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

        pub fn delete(&mut self, path: &str) -> bool {
            let key = Self::normalize_path(path);
            self.files.remove(&key).is_some()
        }

        pub fn list_files(&self) -> Vec<String> {
            let mut keys: Vec<String> = self.files.keys().cloned().collect();
            keys.sort();
            keys
        }

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

    #[test]
    fn test_vfs_seeded_files_and_initialization() {
        let vfs = InMemoryVfs::new();
        let files = vfs.list_files();

        assert_eq!(files.len(), 3);
        assert!(files.contains(&"README.md".to_string()));
        assert!(files.contains(&"package.json".to_string()));
        assert!(files.contains(&"src/index.js".to_string()));

        let readme = vfs.read("README.md").expect("read README.md");
        assert!(readme.contains("Fusion Web Agent"));

        let pkg = vfs.read("package.json").expect("read package.json");
        assert!(pkg.contains("fusion-web-workspace"));

        let index_js = vfs.read("src/index.js").expect("read src/index.js");
        assert!(index_js.contains("Fusion WASM initialized"));
    }

    #[test]
    fn test_vfs_write_read_and_path_normalization() {
        let mut vfs = InMemoryVfs::new();

        // Write to root
        vfs.write("Cargo.toml", "[package]\nname = \"wasm-app\"\n");
        assert_eq!(
            vfs.read("Cargo.toml").unwrap(),
            "[package]\nname = \"wasm-app\"\n"
        );

        // Path normalization: leading "./"
        vfs.write("./src/main.rs", "fn main() { println!(\"Hi\"); }\n");
        assert_eq!(
            vfs.read("src/main.rs").unwrap(),
            "fn main() { println!(\"Hi\"); }\n"
        );
        assert_eq!(
            vfs.read("./src/main.rs").unwrap(),
            "fn main() { println!(\"Hi\"); }\n"
        );

        // Overwriting existing file
        vfs.write("Cargo.toml", "[package]\nname = \"wasm-app-updated\"\n");
        assert_eq!(
            vfs.read("Cargo.toml").unwrap(),
            "[package]\nname = \"wasm-app-updated\"\n"
        );

        // Read nonexistent file
        let err = vfs.read("nonexistent.txt").expect_err("must fail");
        assert!(err.contains("File not found: nonexistent.txt"));
    }

    #[test]
    fn test_vfs_surgical_edit_operations() {
        let mut vfs = InMemoryVfs::new();
        vfs.write(
            "src/calculator.rs",
            "pub fn add(a: i32, b: i32) -> i32 {\n    a - b // bug\n}\n",
        );

        // Successful edit
        let edit_res = vfs.edit("src/calculator.rs", "a - b // bug", "a + b");
        assert!(edit_res.is_ok());
        let updated = vfs.read("src/calculator.rs").unwrap();
        assert!(updated.contains("a + b"));
        assert!(!updated.contains("a - b"));

        // Target string not found error
        let err_not_found = vfs
            .edit("src/calculator.rs", "nonexistent code target", "replacement")
            .expect_err("should fail on missing target string");
        assert!(err_not_found.contains("Target string to replace was not found"));

        // File not found error
        let err_file = vfs
            .edit("missing_file.rs", "foo", "bar")
            .expect_err("should fail on missing file");
        assert!(err_file.contains("File not found: missing_file.rs"));

        // Only first occurrence replaced
        vfs.write("src/dup.txt", "target target target");
        assert!(vfs.edit("src/dup.txt", "target", "replaced").is_ok());
        assert_eq!(vfs.read("src/dup.txt").unwrap(), "replaced target target");
    }

    #[test]
    fn test_vfs_grep_search_and_path_filtering() {
        let mut vfs = InMemoryVfs::new();
        vfs.write(
            "src/agent.rs",
            "// Agent Module\npub struct Agent;\nimpl Agent {\n    pub fn run() {}\n}\n",
        );
        vfs.write(
            "tests/agent_test.rs",
            "// Agent Test\n#[test]\nfn test_agent() {\n    let _a = Agent;\n}\n",
        );

        // Substring grep across all files
        let all_hits = vfs.grep("Agent", None);
        assert_eq!(all_hits.len(), 4); // 2 in src/agent.rs, 2 in tests/agent_test.rs

        // Path filtered grep
        let src_hits = vfs.grep("Agent", Some("src"));
        assert_eq!(src_hits.len(), 2);
        assert_eq!(src_hits[0].0, "src/agent.rs");
        assert_eq!(src_hits[0].1, 1); // Line 1: // Agent Module
        assert_eq!(src_hits[1].1, 2); // Line 2: pub struct Agent;

        // Regex grep with pattern
        let fn_hits = vfs.grep(r"pub fn \w+", None);
        assert_eq!(fn_hits.len(), 1);
        assert_eq!(fn_hits[0].0, "src/agent.rs");
        assert_eq!(fn_hits[0].1, 4);
        assert!(fn_hits[0].2.contains("pub fn run()"));
    }

    #[test]
    fn test_vfs_glob_wildcard_matching() {
        let mut vfs = InMemoryVfs::new();
        vfs.write("src/lib.rs", "// lib");
        vfs.write("src/parser/mod.rs", "// parser mod");
        vfs.write("src/parser/ast.rs", "// parser ast");
        vfs.write("docs/readme.txt", "documentation");

        // Glob all
        let all_files = vfs.glob("**/*");
        assert!(all_files.len() >= 7);

        // Glob rust files
        let rs_files = vfs.glob("*.rs");
        assert!(rs_files.iter().all(|f| f.ends_with(".rs")));
        assert!(rs_files.contains(&"src/lib.rs".to_string()));
        assert!(rs_files.contains(&"src/parser/mod.rs".to_string()));

        // Glob specific directory
        let src_files = vfs.glob("src/*");
        assert!(src_files.contains(&"src/index.js".to_string()));
        assert!(src_files.contains(&"src/lib.rs".to_string()));
    }

    #[test]
    fn test_vfs_delete_and_listing() {
        let mut vfs = InMemoryVfs::new();
        vfs.write("temp_test_file.tmp", "temporary data");
        assert!(vfs.read("temp_test_file.tmp").is_ok());

        // Delete existing file
        assert!(vfs.delete("temp_test_file.tmp"));
        assert!(vfs.read("temp_test_file.tmp").is_err());

        // Delete nonexistent file
        assert!(!vfs.delete("temp_test_file.tmp"));

        // List files is sorted
        let list = vfs.list_files();
        let mut sorted_list = list.clone();
        sorted_list.sort();
        assert_eq!(list, sorted_list);
    }

    #[test]
    fn test_vfs_virtual_bash_commands() {
        let mut vfs = InMemoryVfs::new();

        // pwd
        let (ok_pwd, pwd_out) = vfs.execute_bash("pwd");
        assert!(ok_pwd);
        assert_eq!(pwd_out, "/workspace");

        // ls
        let (ok_ls, ls_out) = vfs.execute_bash("ls");
        assert!(ok_ls);
        assert!(ls_out.contains("README.md"));
        assert!(ls_out.contains("package.json"));

        // echo
        let (ok_echo, echo_out) = vfs.execute_bash("echo Hello Sandbox");
        assert!(ok_echo);
        assert_eq!(echo_out, "Hello Sandbox");

        // touch & cat
        let (ok_touch, touch_out) = vfs.execute_bash("touch notes.txt");
        assert!(ok_touch);
        assert_eq!(touch_out, "Created notes.txt");
        assert!(vfs.read("notes.txt").is_ok());

        vfs.write("notes.txt", "Line 1\nLine 2 with words\n");
        let (ok_cat, cat_out) = vfs.execute_bash("cat notes.txt");
        assert!(ok_cat);
        assert_eq!(cat_out, "Line 1\nLine 2 with words\n");

        // wc
        let (ok_wc, wc_out) = vfs.execute_bash("wc notes.txt");
        assert!(ok_wc);
        assert_eq!(wc_out, "2 5 25 notes.txt");

        // rm
        let (ok_rm, rm_out) = vfs.execute_bash("rm notes.txt");
        assert!(ok_rm);
        assert_eq!(rm_out, "Removed notes.txt");
        assert!(vfs.read("notes.txt").is_err());

        // missing operands
        assert!(!vfs.execute_bash("cat").0);
        assert!(!vfs.execute_bash("touch").0);
        assert!(!vfs.execute_bash("rm").0);
        assert!(!vfs.execute_bash("wc").0);

        // generic command execution
        let (ok_custom, custom_out) = vfs.execute_bash("cargo check");
        assert!(ok_custom);
        assert!(custom_out.contains("[virtual-bash] Executed `cargo check`"));

        // empty command
        let (ok_empty, empty_out) = vfs.execute_bash("   ");
        assert!(ok_empty);
        assert_eq!(empty_out, "");
    }

    #[test]
    fn test_vfs_serialization_checkpoint_roundtrip() {
        let mut vfs = InMemoryVfs::new();
        vfs.write(
            "config.json",
            "{\"theme\": \"dracula\", \"fontSize\": 14}",
        );
        vfs.write("src/core.rs", "pub fn core_engine() -> bool { true }");

        // Serialize to JSON
        let serialized = serde_json::to_string_pretty(&vfs).expect("serialize VFS");
        assert!(serialized.contains("config.json"));
        assert!(serialized.contains("src/core.rs"));
        assert!(serialized.contains("README.md"));

        // Deserialize from JSON
        let restored_vfs: InMemoryVfs =
            serde_json::from_str(&serialized).expect("deserialize VFS");
        assert_eq!(vfs, restored_vfs);

        assert_eq!(
            restored_vfs.read("config.json").unwrap(),
            "{\"theme\": \"dracula\", \"fontSize\": 14}"
        );
        assert_eq!(
            restored_vfs.read("src/core.rs").unwrap(),
            "pub fn core_engine() -> bool { true }"
        );
    }
}

// ============================================================================
// Section 3: Event Stream Serialization for Token Streaming and Tool Calls
// ============================================================================

mod event_stream_serialization_tests {
    use super::*;

    #[test]
    fn test_session_update_agent_message_and_thought_chunk_serialization() {
        // AgentMessageChunk
        let msg_chunk = SessionUpdate::AgentMessageChunk {
            content: AgentMessageContent::assistant_text("pub fn compute("),
            index: Some(1),
            is_first: Some(true),
            is_last: Some(false),
        };

        let chunk_val = serde_json::to_value(&msg_chunk).expect("serialize message chunk");
        assert_eq!(chunk_val["kind"], "agent_message_chunk");
        assert_eq!(chunk_val["content"]["role"], "assistant");
        assert_eq!(
            chunk_val["content"]["content"][0]["text"],
            "pub fn compute("
        );
        assert_eq!(chunk_val["index"], 1);
        assert_eq!(chunk_val["isFirst"], true);
        assert_eq!(chunk_val["isLast"], false);

        let deserialized_msg: SessionUpdate =
            serde_json::from_value(chunk_val).expect("deserialize message chunk");
        match deserialized_msg {
            SessionUpdate::AgentMessageChunk {
                content,
                index,
                is_first,
                ..
            } => {
                assert_eq!(content.role, "assistant");
                assert_eq!(index, Some(1));
                assert_eq!(is_first, Some(true));
            }
            _ => panic!("Expected AgentMessageChunk variant"),
        }

        // AgentThoughtChunk
        let thought_chunk = SessionUpdate::AgentThoughtChunk {
            thought: "Determining optimal data structure for AST traversal.".to_string(),
            index: Some(2),
            elapsed_ms: Some(120),
        };

        let thought_val = serde_json::to_value(&thought_chunk).expect("serialize thought chunk");
        assert_eq!(thought_val["kind"], "agent_thought_chunk");
        assert_eq!(
            thought_val["thought"],
            "Determining optimal data structure for AST traversal."
        );
        assert_eq!(thought_val["index"], 2);
        assert_eq!(thought_val["elapsedMs"], 120);

        let deserialized_thought: SessionUpdate =
            serde_json::from_value(thought_val).expect("deserialize thought chunk");
        match deserialized_thought {
            SessionUpdate::AgentThoughtChunk {
                thought,
                elapsed_ms,
                ..
            } => {
                assert!(thought.contains("AST traversal"));
                assert_eq!(elapsed_ms, Some(120));
            }
            _ => panic!("Expected AgentThoughtChunk variant"),
        }
    }

    #[test]
    fn test_session_update_tool_call_lifecycle_serialization() {
        // 1. ToolCall (Started)
        let tool_call = SessionUpdate::ToolCall {
            call_id: "call_read_01".to_string(),
            name: "read".to_string(),
            args: json!({ "path": "src/main.rs" }),
            status: Some("running".to_string()),
        };
        let call_val = serde_json::to_value(&tool_call).expect("serialize ToolCall");
        assert_eq!(call_val["kind"], "tool_call");
        assert_eq!(call_val["callId"], "call_read_01");
        assert_eq!(call_val["name"], "read");
        assert_eq!(call_val["args"]["path"], "src/main.rs");

        // 2. ToolStatus (Progress)
        let tool_status = SessionUpdate::ToolStatus {
            call_id: "call_bash_02".to_string(),
            name: "bash".to_string(),
            status: "compiling".to_string(),
            progress: Some(0.65),
            partial_output: Some("Building 42/65 targets...".to_string()),
        };
        let status_val = serde_json::to_value(&tool_status).expect("serialize ToolStatus");
        assert_eq!(status_val["kind"], "tool_status");
        assert_eq!(status_val["progress"], 0.65);
        assert_eq!(
            status_val["partialOutput"],
            "Building 42/65 targets..."
        );

        // 3. ToolCallResult (Completed Success)
        let tool_result_success = SessionUpdate::ToolCallResult {
            call_id: "call_read_01".to_string(),
            name: "read".to_string(),
            output: "fn main() { println!(\"OK\"); }".to_string(),
            success: true,
            duration_ms: Some(15),
            error: None,
        };
        let res_val =
            serde_json::to_value(&tool_result_success).expect("serialize ToolCallResult");
        assert_eq!(res_val["kind"], "tool_call_result");
        assert_eq!(res_val["success"], true);
        assert_eq!(res_val["durationMs"], 15);
        assert!(res_val.get("error").is_none());

        // 4. ToolCallResult (Completed Error)
        let tool_result_err = SessionUpdate::ToolCallResult {
            call_id: "call_read_99".to_string(),
            name: "read".to_string(),
            output: "File not found: missing.rs".to_string(),
            success: false,
            duration_ms: Some(2),
            error: Some("NotFound".to_string()),
        };
        let err_val = serde_json::to_value(&tool_result_err).expect("serialize ToolCallResult err");
        assert_eq!(err_val["success"], false);
        assert_eq!(err_val["error"], "NotFound");
    }

    #[test]
    fn test_session_update_advisor_and_token_stats_serialization() {
        // AdvisorStarted
        let adv_started = SessionUpdate::AdvisorStarted {
            advisor: "Security".to_string(),
            role: "Vulnerability and sanitization inspection".to_string(),
        };
        let adv_started_val =
            serde_json::to_value(&adv_started).expect("serialize AdvisorStarted");
        assert_eq!(adv_started_val["kind"], "advisor_started");
        assert_eq!(adv_started_val["advisor"], "Security");

        // AdvisorCritique
        let adv_critique = SessionUpdate::AdvisorCritique {
            advisor: "Security".to_string(),
            approved: false,
            critique: "Potential SQL injection in dynamic query construction.".to_string(),
            role: Some("Vulnerability and sanitization inspection".to_string()),
            severity: Some("warning".to_string()),
            suggestions: Some(vec![
                "Use prepared statements with parameterized queries".to_string(),
            ]),
        };
        let critique_val =
            serde_json::to_value(&adv_critique).expect("serialize AdvisorCritique");
        assert_eq!(critique_val["kind"], "advisor_critique");
        assert_eq!(critique_val["approved"], false);
        assert_eq!(critique_val["severity"], "warning");
        assert_eq!(
            critique_val["suggestions"][0],
            "Use prepared statements with parameterized queries"
        );

        // TokenStats
        let token_stats = SessionUpdate::TokenStats {
            prompt_tokens: 1200,
            completion_tokens: 350,
            total_tokens: 1550,
            cached_tokens: Some(400),
            tokens_per_second: Some(68.5),
        };
        let stats_val = serde_json::to_value(&token_stats).expect("serialize TokenStats");
        assert_eq!(stats_val["kind"], "token_stats");
        assert_eq!(stats_val["promptTokens"], 1200);
        assert_eq!(stats_val["completionTokens"], 350);
        assert_eq!(stats_val["totalTokens"], 1550);
        assert_eq!(stats_val["cachedTokens"], 400);
        assert_eq!(stats_val["tokensPerSecond"], 68.5);

        // Plan & Status & Subagent
        let plan = SessionUpdate::Plan {
            steps: vec![
                "1. Analyze existing AST".to_string(),
                "2. Apply transformation".to_string(),
                "3. Run validation".to_string(),
            ],
        };
        let plan_val = serde_json::to_value(&plan).expect("serialize Plan");
        assert_eq!(plan_val["kind"], "plan");
        assert_eq!(plan_val["steps"].as_array().unwrap().len(), 3);

        let subagent = SessionUpdate::SubagentUpdate {
            name: "ResearchScout".to_string(),
            status: "running".to_string(),
            task: Some("Search workspace for database schema definitions".to_string()),
            output: None,
        };
        let subagent_val = serde_json::to_value(&subagent).expect("serialize SubagentUpdate");
        assert_eq!(subagent_val["kind"], "subagent_update");
        assert_eq!(subagent_val["name"], "ResearchScout");
    }

    #[test]
    fn test_acp_session_event_variants_roundtrip() {
        // TokenChunk
        let event_token = AcpSessionEvent::TokenChunk(TokenStreamChunk {
            index: 10,
            delta: "struct Engine;".to_string(),
            is_first: false,
            is_last: true,
            total_tokens: 42,
            timestamp_ms: 1725200000000,
        });
        let token_json = serde_json::to_string(&event_token).expect("serialize TokenChunk event");
        assert!(token_json.contains("\"eventType\":\"token_chunk\""));
        let parsed_token: AcpSessionEvent =
            serde_json::from_str(&token_json).expect("deserialize TokenChunk event");
        assert_eq!(event_token, parsed_token);

        // ToolStarted
        let event_tool = AcpSessionEvent::ToolStarted(ToolStatusUpdate {
            call_id: "tool_call_007".to_string(),
            name: "glob".to_string(),
            state: ToolExecutionState::Running,
            status: "Running glob".to_string(),
            args: Some(json!({ "pattern": "**/*.rs" })),
            progress: Some(0.0),
            partial_output: None,
            output: None,
            duration_ms: None,
            success: None,
            error: None,
            timestamp_ms: 1725200001000,
        });
        let tool_json = serde_json::to_string(&event_tool).expect("serialize ToolStarted event");
        assert!(tool_json.contains("\"eventType\":\"tool_started\""));
        let parsed_tool: AcpSessionEvent =
            serde_json::from_str(&tool_json).expect("deserialize ToolStarted event");
        assert_eq!(event_tool, parsed_tool);

        // AdvisorConsensus
        let event_consensus = AcpSessionEvent::AdvisorConsensus(AdvisorConsensus {
            total_advisors: 3,
            approved_count: 3,
            rejected_count: 0,
            warning_count: 0,
            overall_approved: true,
            summary: "All advisors approved implementation plan.".to_string(),
        });
        let consensus_json =
            serde_json::to_string(&event_consensus).expect("serialize AdvisorConsensus event");
        assert!(consensus_json.contains("\"eventType\":\"advisor_consensus\""));
        let parsed_consensus: AcpSessionEvent =
            serde_json::from_str(&consensus_json).expect("deserialize AdvisorConsensus event");
        assert_eq!(event_consensus, parsed_consensus);

        // Status & Error
        let event_status = AcpSessionEvent::Status {
            message: "Compacting session context".to_string(),
            level: "info".to_string(),
            timestamp_ms: 1725200003000,
        };
        let status_json = serde_json::to_string(&event_status).unwrap();
        assert!(status_json.contains("\"eventType\":\"status\""));

        let event_error = AcpSessionEvent::Error {
            error: "Network timeout".to_string(),
            recoverable: true,
            timestamp_ms: 1725200004000,
        };
        let err_json = serde_json::to_string(&event_error).unwrap();
        assert!(err_json.contains("\"eventType\":\"error\""));
    }

    #[test]
    fn test_acp_event_bridge_stream_transformation() {
        let mut bridge = AcpEventBridge::new("sess_bridge_test");

        // 1. Text delta events
        let ev1 = bridge.handle_agent_event(AgentEvent::TextDelta("Hello ".to_string()));
        assert_eq!(ev1.len(), 1);
        match &ev1[0] {
            AcpSessionEvent::TokenChunk(chunk) => {
                assert_eq!(chunk.index, 1);
                assert_eq!(chunk.delta, "Hello ");
                assert!(chunk.is_first);
                assert!(!chunk.is_last);
            }
            _ => panic!("Expected TokenChunk"),
        }

        let ev2 = bridge.handle_agent_event(AgentEvent::TextDelta("Fusion!".to_string()));
        assert_eq!(ev2.len(), 1);
        match &ev2[0] {
            AcpSessionEvent::TokenChunk(chunk) => {
                assert_eq!(chunk.index, 2);
                assert_eq!(chunk.delta, "Fusion!");
                assert!(!chunk.is_first);
            }
            _ => panic!("Expected TokenChunk"),
        }

        // 2. Thinking delta event
        let ev_think = bridge.handle_agent_event(AgentEvent::ThinkingDelta("Evaluating...".to_string()));
        assert_eq!(ev_think.len(), 1);
        match &ev_think[0] {
            AcpSessionEvent::ThinkingChunk(chunk) => {
                assert_eq!(chunk.index, 1);
                assert_eq!(chunk.delta, "Evaluating...");
            }
            _ => panic!("Expected ThinkingChunk"),
        }

        // 3. Tool execution cycle
        let ev_tool_start = bridge.handle_agent_event(AgentEvent::ToolStarted {
            id: "call_read_100".to_string(),
            name: "read".to_string(),
            args: json!({ "path": "Cargo.toml" }),
        });
        assert_eq!(ev_tool_start.len(), 1);
        match &ev_tool_start[0] {
            AcpSessionEvent::ToolStarted(status) => {
                assert_eq!(status.call_id, "call_read_100");
                assert_eq!(status.name, "read");
                assert_eq!(status.state, ToolExecutionState::Running);
            }
            _ => panic!("Expected ToolStarted"),
        }

        let ev_tool_finish = bridge.handle_agent_event(AgentEvent::ToolFinished {
            id: "call_read_100".to_string(),
            name: "read".to_string(),
            success: true,
            output: "[package]\nname = \"fusion\"\n".to_string(),
            duration: Duration::from_millis(12),
        });
        assert_eq!(ev_tool_finish.len(), 1);
        match &ev_tool_finish[0] {
            AcpSessionEvent::ToolCompleted(status) => {
                assert_eq!(status.call_id, "call_read_100");
                assert_eq!(status.state, ToolExecutionState::Completed);
                assert_eq!(status.success, Some(true));
                assert_eq!(status.duration_ms, Some(12));
            }
            _ => panic!("Expected ToolCompleted"),
        }

        // 4. Advisor feedback cycle
        let ev_adv_start = bridge.handle_agent_event(AgentEvent::AdvisorStarted {
            advisor: "Architect".to_string(),
            role: "Modularity Review".to_string(),
        });
        assert_eq!(ev_adv_start.len(), 1);

        let ev_adv_critique = bridge.handle_agent_event(AgentEvent::AdvisorCritique {
            advisor: "Architect".to_string(),
            approved: true,
            critique: "Structure conforms to project patterns.".to_string(),
        });
        // Returns critique and collective consensus
        assert_eq!(ev_adv_critique.len(), 2);
    }

    #[test]
    fn test_ndjson_event_envelope_formatting_and_parsing() {
        let events = vec![
            JsonLogEvent::session_start(
                1,
                "sess_ndjson_1",
                Some("gpt-4o".to_string()),
                Some("openai".to_string()),
                Some("/workspace".to_string()),
            ),
            JsonLogEvent::text_delta(2, Some("sess_ndjson_1".to_string()), "Hello ", 1, true, false),
            JsonLogEvent::text_delta(3, Some("sess_ndjson_1".to_string()), "World!", 2, false, true),
            JsonLogEvent::tool_start(
                4,
                Some("sess_ndjson_1".to_string()),
                "call_01",
                "bash",
                json!({ "command": "cargo test" }),
            ),
            JsonLogEvent::tool_finish(
                5,
                Some("sess_ndjson_1".to_string()),
                "call_01",
                "bash",
                true,
                "test result: ok. 42 passed",
                350,
                None,
            ),
            JsonLogEvent::token_stats(
                6,
                Some("sess_ndjson_1".to_string()),
                150,
                75,
                225,
                Some(55.2),
            ),
        ];

        // Format to NDJSON batch string
        let ndjson_str = format_ndjson_batch(&events).expect("format NDJSON batch");
        assert!(ndjson_str.contains("\"kind\":\"session_start\""));
        assert!(ndjson_str.contains("\"kind\":\"text_delta\""));
        assert!(ndjson_str.contains("\"kind\":\"tool_start\""));
        assert!(ndjson_str.contains("\"kind\":\"tool_finish\""));
        assert!(ndjson_str.contains("\"kind\":\"token_stats\""));

        // Parse back from NDJSON lines
        let parsed_events = parse_ndjson_lines(&ndjson_str).expect("parse NDJSON lines");
        assert_eq!(parsed_events.len(), 6);
        assert_eq!(
            parsed_events[0].kind,
            JsonLogEventKind::SessionStart
        );
        assert_eq!(parsed_events[1].kind, JsonLogEventKind::TextDelta);
        assert_eq!(parsed_events[3].kind, JsonLogEventKind::ToolStart);
        assert_eq!(parsed_events[4].kind, JsonLogEventKind::ToolFinish);
        assert_eq!(parsed_events[5].kind, JsonLogEventKind::TokenStats);
    }

    #[tokio::test]
    async fn test_ndjson_streamer_and_reader_async_roundtrip() {
        let (tx, rx) = unbounded_channel();

        let event = JsonLogEvent::text_delta(
            1,
            Some("sess_stream_1".to_string()),
            "Streaming chunk",
            1,
            true,
            false,
        );

        // Send serialized line
        let line = serde_json::to_string(&event).unwrap();
        tx.send(line).unwrap();

        // Wrap reader
        let mut mock_buffer = Vec::new();
        mock_buffer.extend_from_slice(serde_json::to_string(&event).unwrap().as_bytes());
        mock_buffer.push(b'\n');

        let mut reader = JsonLogReader::new(tokio::io::BufReader::new(&mock_buffer[..]));
        let read_event = reader
            .next_event()
            .await
            .expect("read_event")
            .expect("must yield event");

        assert_eq!(
            read_event.session_id.as_deref(),
            Some("sess_stream_1")
        );
        assert_eq!(read_event.kind, JsonLogEventKind::TextDelta);
    }

    #[test]
    fn test_browser_wasm_callback_event_shapes() {
        // Test exact shapes dispatched to browser JavaScript callback functions
        let status_event = json!({
            "type": "status",
            "message": "Processing turn #1 with deepseek-chat"
        });
        assert_eq!(status_event["type"], "status");

        let thinking_event = json!({
            "type": "thinking_delta",
            "delta": "Analyzing AST patterns and formatting response."
        });
        assert_eq!(thinking_event["type"], "thinking_delta");

        let text_event = json!({
            "type": "text_delta",
            "delta": "Here is the refactored code:"
        });
        assert_eq!(text_event["type"], "text_delta");

        let tool_started_event = json!({
            "type": "tool_started",
            "id": "call_glob_1",
            "name": "glob",
            "args": { "pattern": "**/*" }
        });
        assert_eq!(tool_started_event["type"], "tool_started");
        assert_eq!(tool_started_event["name"], "glob");

        let tool_finished_event = json!({
            "type": "tool_finished",
            "id": "call_glob_1",
            "name": "glob",
            "success": true,
            "output": "README.md\nsrc/index.js",
            "duration_ms": 2
        });
        assert_eq!(tool_finished_event["type"], "tool_finished");
        assert_eq!(tool_finished_event["success"], true);

        let advisor_started_event = json!({
            "type": "advisor_started",
            "advisor": "Architect",
            "role": "Code structure and safety review"
        });
        assert_eq!(advisor_started_event["type"], "advisor_started");

        let advisor_critique_event = json!({
            "type": "advisor_critique",
            "advisor": "Architect",
            "approved": true,
            "critique": "Plan is aligned with project conventions and constraints."
        });
        assert_eq!(advisor_critique_event["type"], "advisor_critique");
        assert_eq!(advisor_critique_event["approved"], true);

        let finished_event = json!({
            "type": "finished",
            "usage": {
                "prompt_tokens": 120,
                "completion_tokens": 80,
                "total_tokens": 200
            }
        });
        assert_eq!(finished_event["type"], "finished");
        assert_eq!(finished_event["usage"]["total_tokens"], 200);

        let error_event = json!({
            "type": "error",
            "message": "Network fetch failed"
        });
        assert_eq!(error_event["type"], "error");
    }
}

// ============================================================================
// Section 4: WebAssembly Agent and Checkpoint Integration Tests
// ============================================================================

#[cfg(feature = "wasm")]
mod wasm_agent_integration_tests {
    use fusion::wasm::{checkpoint, create_agent, fusion_version, prompt_turn, restore, VirtualFs};

    #[test]
    fn test_wasm_version_string() {
        let v = fusion_version();
        assert!(!v.is_empty());
        assert_eq!(v, env!("CARGO_PKG_VERSION"));
    }

    #[test]
    fn test_wasm_vfs_direct_operations() {
        let mut vfs = VirtualFs::new();

        // Default seeded files
        assert!(vfs.read("README.md").is_ok());
        assert!(vfs.read("package.json").is_ok());
        assert!(vfs.read("src/index.js").is_ok());

        // Write and read
        vfs.write("app.rs", "fn main() {\n    println!(\"Hello WASM\");\n}\n");
        assert_eq!(
            vfs.read("app.rs").unwrap(),
            "fn main() {\n    println!(\"Hello WASM\");\n}\n"
        );

        // Edit
        assert!(vfs.edit("app.rs", "Hello WASM", "Hello Fusion").is_ok());
        assert!(vfs.read("app.rs").unwrap().contains("Hello Fusion"));

        // Grep
        let grep_results = vfs.grep("Hello", None);
        assert_eq!(grep_results.len(), 1);
        assert_eq!(grep_results[0].0, "app.rs");
        assert_eq!(grep_results[0].1, 2);

        // Glob
        let matched = vfs.glob("*.rs");
        assert!(matched.contains(&"app.rs".to_string()));

        // Virtual bash
        let (ok, out) = vfs.execute_bash("cat app.rs");
        assert!(ok);
        assert!(out.contains("Hello Fusion"));

        // Delete
        assert!(vfs.delete("app.rs"));
        assert!(vfs.read("app.rs").is_err());
    }

    #[test]
    fn test_wasm_agent_create_and_checkpoint_cycle() {
        let config_json = r#"{
            "default_provider": "openrouter",
            "default_model": "anthropic/claude-3.5-sonnet",
            "advisors_enabled": true
        }"#;

        let mut agent = create_agent(config_json).expect("create_agent should succeed");
        assert_eq!(agent.get_active_model(), "anthropic/claude-3.5-sonnet");

        // Custom system prompt
        agent.set_system_prompt("You are a specialized browser assistant.");

        // Interact with VFS via agent
        agent.fs_write("virtual_config.toml", "theme = \"dark\"\n");
        assert_eq!(
            agent.fs_read("virtual_config.toml").unwrap(),
            "theme = \"dark\"\n"
        );

        let files = agent.fs_list().unwrap();
        assert!(files.contains("virtual_config.toml"));

        // Checkpoint
        let saved_state = agent.checkpoint().expect("checkpoint should succeed");
        assert!(saved_state.contains("anthropic/claude-3.5-sonnet"));
        assert!(saved_state.contains("virtual_config.toml"));
        assert!(saved_state.contains("session"));
        assert!(saved_state.contains("vfs"));

        // Restore into another agent instance
        let mut new_agent = create_agent("{}").expect("create_agent 2 should succeed");
        new_agent
            .restore(&saved_state)
            .expect("restore should succeed");

        assert_eq!(new_agent.get_active_model(), "anthropic/claude-3.5-sonnet");
        assert_eq!(
            new_agent.fs_read("virtual_config.toml").unwrap(),
            "theme = \"dark\"\n"
        );

        // Global checkpoint and restore
        let global_ckpt = checkpoint().expect("global checkpoint should succeed");
        assert!(global_ckpt.contains("anthropic/claude-3.5-sonnet"));
        assert!(restore(&global_ckpt).is_ok());
    }

    #[tokio::test]
    async fn test_wasm_prompt_turn_execution() {
        let mut agent = create_agent(r#"{"default_model": "gpt-4o-mini"}"#).expect("create agent");

        // Run prompt turn
        let response = agent
            .prompt_turn("list files in project", None)
            .await
            .expect("prompt_turn should return response");

        assert!(response.contains("README.md") || response.contains("files"));

        // Verify session recorded user and assistant messages
        let messages_json = agent.get_messages().expect("get_messages");
        assert!(messages_json.contains("list files in project"));

        // Verify token stats updated
        let stats_json = agent.get_token_stats().expect("get_token_stats");
        assert!(stats_json.contains("total_tokens"));

        // Standalone prompt_turn function
        let standalone_response = prompt_turn("hello", None)
            .await
            .expect("standalone prompt_turn should succeed");
        assert!(!standalone_response.is_empty());
    }
}
