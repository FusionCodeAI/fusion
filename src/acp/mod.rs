//! Agent Client Protocol (ACP) JSON-RPC 2.0 Server Adapter.
//!
//! Enables IDEs, editors (such as Zed, JetBrains, Neovim), and external tools
//! to drive Fusion as an ACP coding agent over standard I/O (stdio) or generic async streams.

pub mod events;
pub mod json_stream;
pub mod server;
pub mod types;

pub use events::*;
pub use json_stream::*;
pub use server::AcpServer;
pub use types::*;

use crate::agent::loop_runner::AgentRunner;
#[cfg(test)]
use crate::config::Config;
#[cfg(test)]
use crate::provider::LlmClient;
#[cfg(test)]
use crate::tools::types::{ToolContext, ToolRegistry};
/// Starts and runs the ACP JSON-RPC stdio server with the specified AgentRunner.
pub async fn run_stdio_server(runner: AgentRunner) -> anyhow::Result<()> {
    let server = AcpServer::from_runner(&runner);
    server.run_stdio().await
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

    fn test_server() -> AcpServer {
        let config = Config::default();
        let client = LlmClient::new();
        let tools = ToolRegistry::new();
        let tool_ctx = ToolContext::default();
        AcpServer::new(config, client, tools, tool_ctx)
    }

    #[tokio::test]
    async fn test_acp_handshake() {
        let server = test_server();
        assert!(!server.is_initialized());

        let init_req = json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "initialize",
            "params": {
                "protocolVersion": 1,
                "clientCapabilities": {
                    "fs": { "readTextFile": true, "writeTextFile": true },
                    "terminal": true
                },
                "clientInfo": {
                    "name": "zed",
                    "version": "0.180.0"
                }
            }
        });

        let (out_tx, mut out_rx) = tokio::sync::mpsc::unbounded_channel();
        server.process_raw_message(&init_req.to_string(), out_tx).await;

        let response_str = out_rx.recv().await.expect("Expected handshake response");
        let resp: JsonRpcResponse = serde_json::from_str(&response_str).expect("Valid JSON-RPC response");

        assert_eq!(resp.id, RequestId::Number(1));
        assert!(resp.error.is_none());

        let result: InitializeResult = serde_json::from_value(resp.result.unwrap()).unwrap();
        assert_eq!(result.protocol_version, 1);
        assert_eq!(result.agent_info.name, "fusion");
        assert!(result.agent_capabilities.load_session);
        assert!(server.is_initialized());
    }

    #[tokio::test]
    async fn test_acp_ping() {
        let server = test_server();
        let ping_req = json!({
            "jsonrpc": "2.0",
            "id": "ping-42",
            "method": "ping"
        });

        let (out_tx, mut out_rx) = tokio::sync::mpsc::unbounded_channel();
        server.process_raw_message(&ping_req.to_string(), out_tx).await;

        let response_str = out_rx.recv().await.expect("Expected ping response");
        let resp: JsonRpcResponse = serde_json::from_str(&response_str).unwrap();

        assert_eq!(resp.id, RequestId::String("ping-42".to_string()));
        assert_eq!(resp.result.unwrap(), json!({ "pong": true }));
    }

    #[tokio::test]
    async fn test_session_lifecycle() {
        let server = test_server();

        // 1. Create session
        let new_session_req = json!({
            "jsonrpc": "2.0",
            "id": 2,
            "method": "session/new",
            "params": {
                "cwd": "/workspace",
                "model": "deepseek-chat"
            }
        });

        let (out_tx, mut out_rx) = tokio::sync::mpsc::unbounded_channel();
        server.process_raw_message(&new_session_req.to_string(), out_tx.clone()).await;

        let response_str = out_rx.recv().await.expect("Expected new session response");
        let resp: JsonRpcResponse = serde_json::from_str(&response_str).unwrap();
        let new_session_res: NewSessionResult = serde_json::from_value(resp.result.unwrap()).unwrap();
        let session_id = new_session_res.session_id;
        assert!(!session_id.is_empty());

        // 2. List sessions
        let list_req = json!({
            "jsonrpc": "2.0",
            "id": 3,
            "method": "session/list"
        });
        server.process_raw_message(&list_req.to_string(), out_tx.clone()).await;
        let list_str = out_rx.recv().await.expect("Expected list response");
        let list_resp: JsonRpcResponse = serde_json::from_str(&list_str).unwrap();
        let list_res: ListSessionsResult = serde_json::from_value(list_resp.result.unwrap()).unwrap();
        assert!(list_res.sessions.iter().any(|s| s.session_id == session_id));

        // 3. Close session
        let close_req = json!({
            "jsonrpc": "2.0",
            "id": 4,
            "method": "session/close",
            "params": {
                "sessionId": session_id
            }
        });
        server.process_raw_message(&close_req.to_string(), out_tx.clone()).await;
        let close_str = out_rx.recv().await.expect("Expected close response");
        let close_resp: JsonRpcResponse = serde_json::from_str(&close_str).unwrap();
        assert_eq!(close_resp.result.unwrap(), json!({ "success": true }));
    }

    #[tokio::test]
    async fn test_invalid_json_and_method_not_found() {
        let server = test_server();
        let (out_tx, mut out_rx) = tokio::sync::mpsc::unbounded_channel();

        // Malformed JSON
        server.process_raw_message("not valid json", out_tx.clone()).await;
        let err_str = out_rx.recv().await.unwrap();
        let err_resp: JsonRpcResponse = serde_json::from_str(&err_str).unwrap();
        assert!(err_resp.error.is_some());
        assert_eq!(err_resp.error.unwrap().code, error_codes::PARSE_ERROR);

        // Unknown method
        let unknown_req = json!({
            "jsonrpc": "2.0",
            "id": 99,
            "method": "unknown/method"
        });
        server.process_raw_message(&unknown_req.to_string(), out_tx.clone()).await;
        let unknown_str = out_rx.recv().await.unwrap();
        let unknown_resp: JsonRpcResponse = serde_json::from_str(&unknown_str).unwrap();
        assert!(unknown_resp.error.is_some());
        assert_eq!(unknown_resp.error.unwrap().code, error_codes::METHOD_NOT_FOUND);
    }

    #[tokio::test]
    async fn test_prompt_input_parsing() {
        // Plain string
        let p1: PromptInput = serde_json::from_str(r#""Write hello world""#).unwrap();
        assert_eq!(p1.to_text(), "Write hello world");

        // Array of content blocks
        let p2: PromptInput = serde_json::from_str(r#"[{"type": "text", "text": "First line"}, {"type": "text", "text": "Second line"}]"#).unwrap();
        assert_eq!(p2.to_text(), "First line\nSecond line");

        // Single object content block
        let p3: PromptInput = serde_json::from_str(r#"{"type": "text", "text": "Single block text"}"#).unwrap();
        assert_eq!(p3.to_text(), "Single block text");
    }

    #[tokio::test]
    async fn test_duplex_stream_handshake() {
        let server = test_server();
        let (client_io, server_io) = tokio::io::duplex(4096);
        let (server_read, server_write) = tokio::io::split(server_io);
        let (client_read, mut client_write) = tokio::io::split(client_io);

        // Spawn server loop
        let server_handle = tokio::spawn(async move {
            let buf_read = BufReader::new(server_read);
            server.run_stream(buf_read, server_write).await
        });

        // Send initialize request from client
        let init_req = json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "initialize",
            "params": {
                "protocolVersion": 1,
                "clientCapabilities": {},
                "clientInfo": { "name": "test-client" }
            }
        });

        client_write.write_all(init_req.to_string().as_bytes()).await.unwrap();
        client_write.write_all(b"\n").await.unwrap();
        client_write.flush().await.unwrap();

        let mut client_buf_read = BufReader::new(client_read);
        let mut response_line = String::new();
        client_buf_read.read_line(&mut response_line).await.unwrap();

        let resp: JsonRpcResponse = serde_json::from_str(response_line.trim()).unwrap();
        assert_eq!(resp.id, RequestId::Number(1));
        let result: InitializeResult = serde_json::from_value(resp.result.unwrap()).unwrap();
        assert_eq!(result.protocol_version, 1);
        assert_eq!(result.agent_info.name, "fusion");

        // Clean close
        drop(client_buf_read);
        drop(client_write);
        let _ = server_handle.await;
    }
}
