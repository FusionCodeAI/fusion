//! Integration Tests for Fusion Multi-Agent Subsystems, Coordination Mesh & Advisory Committee
//!
//! Verifies end-to-end functionality of:
//! 1. Concurrent subagent execution, lifecycle management, role-based tool isolation, and cancellation
//! 2. Pub-sub broadcast message delivery across mesh topics (discovery, progress, status, alerts, coordination)
//! 3. Direct peer-to-peer query-response RPC with timeouts, mailboxes, and conversation threading
//! 4. Advisory committee evaluation flow (Architecture, Security, Code Quality) and risk assessment consensus
//! 5. Multi-agent coordination mesh primitives (file resource locks, shared blackboard memory, sync barriers)
//! 6. Session persistence, resumption, token accumulation, and Markdown export
//! 7. End-to-end multi-agent collaborative workflows combining orchestration, mesh communication, and advisors

use std::collections::HashMap;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use futures::future::join_all;
use serde_json::{json, Value};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{broadcast, oneshot, RwLock};
use uuid::Uuid;

use fusion::agent::advisor::{
    consult_advisors, format_critiques_for_system_prompt, format_critiques_summary, Advisor,
    AdvisorCritique, AdvisorEngine, AdvisorRegistry, RiskLevel,
};
use fusion::agent::consensus::{
    resolve_consensus, resolve_consensus_with_policy, resolve_majority, resolve_risk_weighted,
    resolve_security_veto, resolve_unanimous, AdvisorVote, ConsensusEngine, ConsensusPolicy,
    ConsensusResolution, ConsensusStrategy,
};
use fusion::agent::mesh::{
    topics, AdvisorReviewHandler, AdvisorReviewRequest, AdvisorReviewResponse, AgentInfo,
    AgentMesh, AgentRole, AgentStatus, BroadcastMessage, BroadcastPayload, DirectMessage,
    MeshBroadcastTool, MeshClaimResourceTool, MeshError, MeshListPeersTool, MeshPeerChannel,
    MeshQueryPeerTool, MeshRequestReviewTool, PeerQuery, PeerQueryEnvelope, PeerResponse,
    ResourceClaim, SharedFact,
};
use fusion::agent::session::{Session, SessionSummary, TokenStats};
use fusion::agent::subagent::{
    SpawnBatchSubagentsTool, SpawnSubagentTool, SubagentHandle, SubagentInfo, SubagentManager,
    SubagentProgress, SubagentResult, SubagentRole, SubagentStatus, SubagentTask,
};
use fusion::config::Config;
use fusion::provider::types::{Message, Role, ToolCall};
use fusion::provider::LlmClient;
use fusion::tools::file::{ReadFileTool, WriteFileTool};
use fusion::tools::grep::GrepTool;
use fusion::tools::types::{Tool, ToolContext, ToolRegistry};

// ===========================================================================
// Test Helper: RAII Temp Directory
// ===========================================================================

struct TempDir {
    path: PathBuf,
}

impl TempDir {
    fn new(prefix: &str) -> Self {
        let unique = format!("fusion_multiagent_{}_{}", prefix, Uuid::new_v4());
        let path = std::env::temp_dir().join(unique);
        std::fs::create_dir_all(&path).expect("failed to create temp dir");
        Self { path }
    }

    fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

// ===========================================================================
// Test Helper: Mock LLM HTTP Server (OpenAI-compatible SSE streaming)
// ===========================================================================

#[derive(Clone)]
enum MockResponse {
    /// Streams standard assistant text delta chunks.
    Text(String),
    /// Streams an advisor JSON critique response.
    Json(Value),
    /// Streams a tool call execution request.
    ToolCall {
        id: String,
        name: String,
        arguments: String,
    },
    /// Returns an HTTP error response.
    Error(u16, String),
    /// Delayed text response to test cancellation.
    DelayedText(String, Duration),
}

type HandlerFn = Arc<dyn Fn(&Value) -> MockResponse + Send + Sync>;

struct MockLlmServer {
    addr: SocketAddr,
    shutdown_tx: Option<oneshot::Sender<()>>,
    handler: Arc<RwLock<HandlerFn>>,
    request_count: Arc<AtomicUsize>,
}

impl MockLlmServer {
    async fn start() -> Self {
        Self::start_with_handler(Arc::new(|payload| Self::default_handle_request(payload))).await
    }

    async fn start_with_handler(initial_handler: HandlerFn) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("failed to bind tcp listener");
        let addr = listener.local_addr().expect("failed to get local addr");

        let (shutdown_tx, mut shutdown_rx) = oneshot::channel();
        let handler = Arc::new(RwLock::new(initial_handler));
        let handler_clone = Arc::clone(&handler);
        let request_count = Arc::new(AtomicUsize::new(0));
        let count_clone = Arc::clone(&request_count);

        tokio::spawn(async move {
            loop {
                tokio::select! {
                    _ = &mut shutdown_rx => {
                        break;
                    }
                    accept_res = listener.accept() => {
                        if let Ok((socket, _)) = accept_res {
                            let current_handler = {
                                let guard = handler_clone.read().await;
                                guard.clone()
                            };
                            let req_count = Arc::clone(&count_clone);
                            tokio::spawn(async move {
                                req_count.fetch_add(1, Ordering::SeqCst);
                                Self::handle_connection(socket, current_handler).await;
                            });
                        }
                    }
                }
            }
        });

        Self {
            addr,
            shutdown_tx: Some(shutdown_tx),
            handler,
            request_count,
        }
    }

    fn base_url(&self) -> String {
        format!("http://{}", self.addr)
    }

    fn client(&self) -> Arc<LlmClient> {
        Arc::new(LlmClient::new())
    }

    fn config(&self) -> Config {
        let mut config = Config::default();
        config.default_provider = "openai".to_string();
        config.openai_base_url = Some(self.base_url());
        config.openai_api_key = Some("mock-api-key-test".to_string());
        config.advisors_enabled = true;
        config
    }

    async fn set_handler<F>(&self, handler: F)
    where
        F: Fn(&Value) -> MockResponse + Send + Sync + 'static,
    {
        let mut guard = self.handler.write().await;
        *guard = Arc::new(handler);
    }

    fn request_count(&self) -> usize {
        self.request_count.load(Ordering::SeqCst)
    }

    fn default_handle_request(payload: &Value) -> MockResponse {
        let messages = payload
            .get("messages")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();

        let all_text = messages
            .iter()
            .filter_map(|m| m.get("content").and_then(|c| c.as_str()))
            .collect::<Vec<_>>()
            .join("\n");

        let system_text = messages
            .iter()
            .find(|m| m.get("role").and_then(|r| r.as_str()) == Some("system"))
            .and_then(|m| m.get("content").and_then(|c| c.as_str()))
            .unwrap_or("");

        // 1. Advisor query check
        if system_text.contains("Advisor") || all_text.contains("evaluate this user request") {
            if all_text.contains("dangerous")
                || all_text.contains("rm -rf")
                || all_text.contains("delete database")
            {
                return MockResponse::Json(json!({
                    "approved": false,
                    "risk_level": "critical",
                    "critique": "Security Advisor detected highly destructive operations.",
                    "suggestions": ["Block execution immediately", "Verify user intent"]
                }));
            }

            if all_text.contains("cache") || system_text.contains("Architecture") {
                return MockResponse::Json(json!({
                    "approved": true,
                    "risk_level": "low",
                    "critique": "Architecture design is modular, stateless, and follows clean boundaries.",
                    "suggestions": ["Add integration tests for concurrent access"]
                }));
            }

            return MockResponse::Json(json!({
                "approved": true,
                "risk_level": "low",
                "critique": "Evaluation completed successfully with zero identified risks.",
                "suggestions": []
            }));
        }

        // 2. Subagent tool calling check
        if let Some(last_msg) = messages.last() {
            if last_msg.get("role").and_then(|r| r.as_str()) == Some("tool") {
                let tool_content = last_msg
                    .get("content")
                    .and_then(|c| c.as_str())
                    .unwrap_or("");
                return MockResponse::Text(format!(
                    "Subagent execution complete. Tool result verified: {}",
                    tool_content.trim()
                ));
            }
        }

        if all_text.contains("WRITE_FILE:") {
            if let Some(path_idx) = all_text.find("WRITE_FILE:") {
                let remainder = &all_text[path_idx + 11..];
                let path = remainder.lines().next().unwrap_or("output.txt").trim();
                return MockResponse::ToolCall {
                    id: "call_subagent_write_1".to_string(),
                    name: "write".to_string(),
                    arguments: json!({
                        "path": path,
                        "content": "Artifact content produced by worker subagent in multi-agent test."
                    })
                    .to_string(),
                };
            }
        }

        MockResponse::Text("Subagent task accomplished successfully.".to_string())
    }

    async fn handle_connection(mut socket: TcpStream, handler: HandlerFn) {
        let mut buffer = Vec::new();
        let mut temp = [0u8; 4096];

        let mut header_end = None;
        let mut content_length = 0;

        loop {
            match socket.read(&mut temp).await {
                Ok(0) => return,
                Ok(n) => {
                    buffer.extend_from_slice(&temp[..n]);
                    if header_end.is_none() {
                        if let Some(pos) = buffer.windows(4).position(|w| w == b"\r\n\r\n") {
                            header_end = Some(pos + 4);
                            let headers_str = String::from_utf8_lossy(&buffer[..pos]);
                            for line in headers_str.lines() {
                                if let Some(stripped) =
                                    line.to_lowercase().strip_prefix("content-length:")
                                {
                                    content_length = stripped.trim().parse::<usize>().unwrap_or(0);
                                }
                            }
                        }
                    }

                    if let Some(end) = header_end {
                        if buffer.len() >= end + content_length {
                            break;
                        }
                    }
                }
                Err(_) => return,
            }
        }

        let body_bytes = if let Some(end) = header_end {
            &buffer[end..end + content_length]
        } else {
            &[]
        };

        let payload: Value = serde_json::from_slice(body_bytes).unwrap_or(json!({}));
        let response = handler(&payload);

        match response {
            MockResponse::Text(text) => {
                let headers = "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nCache-Control: no-cache\r\nConnection: close\r\n\r\n";
                let _ = socket.write_all(headers.as_bytes()).await;

                let chunks = vec![text];
                for chunk in chunks {
                    let event_json = json!({
                        "id": "chatcmpl-mock",
                        "object": "chat.completion.chunk",
                        "choices": [{
                            "index": 0,
                            "delta": { "content": chunk },
                            "finish_reason": Value::Null
                        }]
                    });
                    let sse = format!("data: {}\n\n", serde_json::to_string(&event_json).unwrap());
                    let _ = socket.write_all(sse.as_bytes()).await;
                }

                let final_chunk = json!({
                    "id": "chatcmpl-mock",
                    "object": "chat.completion.chunk",
                    "choices": [{
                        "index": 0,
                        "delta": {},
                        "finish_reason": "stop"
                    }]
                });
                let _ = socket
                    .write_all(
                        format!("data: {}\n\n", serde_json::to_string(&final_chunk).unwrap())
                            .as_bytes(),
                    )
                    .await;
                let _ = socket.write_all(b"data: [DONE]\n\n").await;
            }
            MockResponse::Json(val) => {
                let json_str = serde_json::to_string(&val).unwrap();
                let headers = "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nCache-Control: no-cache\r\nConnection: close\r\n\r\n";
                let _ = socket.write_all(headers.as_bytes()).await;

                let event_json = json!({
                    "id": "chatcmpl-mock-json",
                    "object": "chat.completion.chunk",
                    "choices": [{
                        "index": 0,
                        "delta": { "content": json_str },
                        "finish_reason": Value::Null
                    }]
                });
                let sse = format!("data: {}\n\n", serde_json::to_string(&event_json).unwrap());
                let _ = socket.write_all(sse.as_bytes()).await;

                let final_chunk = json!({
                    "id": "chatcmpl-mock-json",
                    "object": "chat.completion.chunk",
                    "choices": [{
                        "index": 0,
                        "delta": {},
                        "finish_reason": "stop"
                    }]
                });
                let _ = socket
                    .write_all(
                        format!("data: {}\n\n", serde_json::to_string(&final_chunk).unwrap())
                            .as_bytes(),
                    )
                    .await;
                let _ = socket.write_all(b"data: [DONE]\n\n").await;
            }
            MockResponse::ToolCall {
                id,
                name,
                arguments,
            } => {
                let headers = "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nCache-Control: no-cache\r\nConnection: close\r\n\r\n";
                let _ = socket.write_all(headers.as_bytes()).await;

                let tool_chunk = json!({
                    "id": "chatcmpl-mock-tool",
                    "object": "chat.completion.chunk",
                    "choices": [{
                        "index": 0,
                        "delta": {
                            "tool_calls": [{
                                "index": 0,
                                "id": id,
                                "type": "function",
                                "function": {
                                    "name": name,
                                    "arguments": arguments
                                }
                            }]
                        },
                        "finish_reason": "tool_calls"
                    }]
                });
                let sse = format!("data: {}\n\n", serde_json::to_string(&tool_chunk).unwrap());
                let _ = socket.write_all(sse.as_bytes()).await;
                let _ = socket.write_all(b"data: [DONE]\n\n").await;
            }
            MockResponse::DelayedText(text, delay) => {
                tokio::time::sleep(delay).await;
                let headers = "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nCache-Control: no-cache\r\nConnection: close\r\n\r\n";
                let _ = socket.write_all(headers.as_bytes()).await;

                let event_json = json!({
                    "id": "chatcmpl-mock-delayed",
                    "object": "chat.completion.chunk",
                    "choices": [{
                        "index": 0,
                        "delta": { "content": text },
                        "finish_reason": "stop"
                    }]
                });
                let sse = format!("data: {}\n\n", serde_json::to_string(&event_json).unwrap());
                let _ = socket.write_all(sse.as_bytes()).await;
                let _ = socket.write_all(b"data: [DONE]\n\n").await;
            }
            MockResponse::Error(status, message) => {
                let body = json!({ "error": { "message": message } }).to_string();
                let resp = format!(
                    "HTTP/1.1 {} Error\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    status,
                    body.len(),
                    body
                );
                let _ = socket.write_all(resp.as_bytes()).await;
            }
        }
    }
}

impl Drop for MockLlmServer {
    fn drop(&mut self) {
        if let Some(tx) = self.shutdown_tx.take() {
            let _ = tx.send(());
        }
    }
}

fn create_test_tools() -> ToolRegistry {
    use std::sync::Arc;
    let mut registry = ToolRegistry::new();
    registry.register(Arc::new(ReadFileTool::new()));
    registry.register(Arc::new(WriteFileTool::new()));
    registry.register(Arc::new(GrepTool::new()));
    registry
}

// ===========================================================================
// Part 1: Concurrent Subagent Execution & Lifecycle Tests (SubagentManager)
// ===========================================================================

#[tokio::test]
async fn test_concurrent_subagents_execution() {
    let server = MockLlmServer::start().await;
    let client = server.client();
    let config = server.config();
    let tools = create_test_tools();

    let manager = SubagentManager::new(client, config, tools).with_max_concurrent(8);

    let tasks = vec![
        SubagentTask::scout("Inspect workspace architecture").with_name("Scout-1"),
        SubagentTask::coder("Refactor async channel buffers").with_name("Coder-1"),
        SubagentTask::tester("Verify error boundary conditions").with_name("Tester-1"),
        SubagentTask::reviewer("Audit token accounting math").with_name("Reviewer-1"),
        SubagentTask::general("Organize build artifacts").with_name("Worker-1"),
        SubagentTask::custom(
            "DatabaseWorker",
            "Optimize query indexes",
            "Analyze SQL plans",
        ),
    ];

    // Spawn all 6 tasks concurrently using spawn_batch and wait via join_all
    let handles: Vec<SubagentHandle> = manager.spawn_batch(tasks);
    let mut wait_futures = Vec::new();
    for handle in handles {
        wait_futures.push(handle.wait());
    }
    let results: Vec<anyhow::Result<SubagentResult>> = join_all(wait_futures).await;
    assert_eq!(results.len(), 6, "Expected 6 subagent execution results");

    for res in results {
        let sub_res: SubagentResult = res.expect("Subagent execution should succeed");
        assert!(
            !sub_res.output.is_empty(),
            "Subagent output should not be empty"
        );
        assert!(
            sub_res.turns >= 1,
            "Subagent should have taken at least 1 turn"
        );
    }

    // Verify all subagents are registered and marked Completed in active_agents list
    let active_list: Vec<SubagentInfo> = manager.list_subagents().await;
    assert_eq!(active_list.len(), 6);
    for info in &active_list {
        match &info.status {
            SubagentStatus::Completed { output, turns } => {
                assert!(!output.is_empty());
                assert!(*turns >= 1);
            }
            other => panic!("Expected completed status, found {:?}", other),
        }
    }
}

#[tokio::test]
async fn test_subagent_concurrency_limiting_semaphore() {
    let server = MockLlmServer::start().await;
    let client = server.client();
    let config = server.config();
    let tools = create_test_tools();

    // Limit concurrency to 2 concurrent workers
    let manager = SubagentManager::new(client, config, tools).with_max_concurrent(2);

    let tasks = (0..6)
        .map(|i| SubagentTask::scout(format!("Parallel investigation slice {}", i)))
        .collect::<Vec<_>>();

    let results = manager.run_concurrent(tasks).await;
    assert_eq!(results.len(), 6);

    for res in results {
        let sub_res = res.expect("All queued subagents should complete successfully");
        assert!(sub_res.success);
    }

    assert_eq!(manager.list_subagents().await.len(), 6);
}

#[tokio::test]
async fn test_subagent_progress_event_streaming() {
    let server = MockLlmServer::start().await;
    let client = server.client();
    let config = server.config();
    let tools = create_test_tools();

    let manager = SubagentManager::new(client, config, tools);
    let mut global_rx: broadcast::Receiver<SubagentProgress> = manager.subscribe();

    let task = SubagentTask::coder("Streamed implementation task")
        .with_id("test-stream-agent")
        .with_name("StreamCoder");

    let mut handle = manager.spawn(task);

    // Collect events from handle-specific receiver
    let mut handle_events = Vec::new();
    while let Some(event) = handle.recv_progress().await {
        handle_events.push(event.clone());
        if matches!(event, SubagentProgress::Completed { .. }) {
            break;
        }
    }

    assert!(!handle_events.is_empty());
    assert!(matches!(handle_events[0], SubagentProgress::Started { .. }));
    assert!(handle_events
        .iter()
        .any(|e| matches!(e, SubagentProgress::TurnStarted { .. })));
    assert!(handle_events
        .iter()
        .any(|e| matches!(e, SubagentProgress::Completed { .. })));

    // Verify global broadcast receiver also received events
    let mut global_started = false;
    while let Ok(event) = global_rx.try_recv() {
        if event.id() == "test-stream-agent" && matches!(event, SubagentProgress::Started { .. }) {
            global_started = true;
            break;
        }
    }
    assert!(
        global_started,
        "Global subscriber should receive agent started event"
    );

    let result = handle.wait().await.expect("Wait should succeed");
    assert!(result.success);
    assert_eq!(result.id, "test-stream-agent");
}

#[tokio::test]
async fn test_subagent_cancellation_lifecycle() {
    let server = MockLlmServer::start().await;
    // Set mock server to delay responses by 500ms
    server
        .set_handler(|_| {
            MockResponse::DelayedText(
                "Delayed response should be cancelled".to_string(),
                Duration::from_millis(500),
            )
        })
        .await;

    let client = server.client();
    let config = server.config();
    let tools = create_test_tools();

    let manager = SubagentManager::new(client, config, tools);

    let task1 = SubagentTask::scout("Long running exploration 1").with_id("cancel-task-1");
    let task2 = SubagentTask::coder("Long running exploration 2").with_id("cancel-task-2");

    let handle1 = manager.spawn(task1);
    let handle2 = manager.spawn(task2);

    // Give tasks a brief moment to transition to running
    tokio::time::sleep(Duration::from_millis(20)).await;

    // Cancel task 1 via handle
    handle1.cancel();

    // Cancel task 2 via manager
    let cancelled_2 = manager.cancel("cancel-task-2").await;
    assert!(
        cancelled_2,
        "manager.cancel should return true for active task"
    );

    let res1 = handle1.wait().await.expect("Task 1 should resolve");
    let res2 = handle2.wait().await.expect("Task 2 should resolve");

    assert!(
        !res1.success,
        "Cancelled task 1 should report success = false"
    );
    assert!(
        !res2.success,
        "Cancelled task 2 should report success = false"
    );
    assert_eq!(res1.output, "Subagent cancelled.");
    assert_eq!(res2.output, "Subagent cancelled.");

    // Verify statuses in manager
    assert_eq!(
        manager.get_status("cancel-task-1").await,
        Some(SubagentStatus::Cancelled)
    );
    assert_eq!(
        manager.get_status("cancel-task-2").await,
        Some(SubagentStatus::Cancelled)
    );
}

#[tokio::test]
async fn test_subagent_tool_execution_e2e() {
    let temp_dir = TempDir::new("subagent_tools");
    let artifact_file = temp_dir.path().join("subagent_created.txt");
    let artifact_str = artifact_file.to_string_lossy().to_string();

    let server = MockLlmServer::start().await;
    let client = server.client();
    let config = server.config();
    let tools = create_test_tools();

    let manager = SubagentManager::new(client, config, tools);

    // The subagent task instruction includes WRITE_FILE:<path>, which the mock server recognizes
    let task_instruction = format!("Perform file generation: WRITE_FILE:{}", artifact_str);
    let task = SubagentTask::coder(task_instruction).with_name("FileCreator");

    let handle = manager.spawn(task);
    let result = handle.wait().await.expect("Subagent should succeed");

    assert!(result.success);
    assert!(
        result.output.contains("Subagent execution complete"),
        "Output should confirm execution: {}",
        result.output
    );

    // Verify that the WriteFileTool was actually executed and the artifact exists on disk
    assert!(artifact_file.exists(), "Target file must exist on disk");
    let content = std::fs::read_to_string(&artifact_file).expect("Must read written file");
    assert!(
        content.contains("Artifact content produced by worker subagent"),
        "Content mismatch: {}",
        content
    );
}

#[tokio::test]
async fn test_subagent_role_tool_filtering() {
    let full = create_test_tools();

    // Scout has read, grep
    let scout_tools = SubagentRole::Scout.filter_tools(&full);
    assert!(scout_tools.get("read").is_some());
    assert!(scout_tools.get("grep").is_some());
    assert!(scout_tools.get("write").is_none());

    // Coder has read, write, grep
    let coder_tools = SubagentRole::Coder.filter_tools(&full);
    assert!(coder_tools.get("read").is_some());
    assert!(coder_tools.get("write").is_some());
    assert!(coder_tools.get("grep").is_some());

    // General inherits all registered tools
    let general_tools = SubagentRole::General.filter_tools(&full);
    assert!(general_tools.get("read").is_some());
    assert!(general_tools.get("write").is_some());
    assert!(general_tools.get("grep").is_some());

    // Role from string and display representation
    assert_eq!(
        SubagentRole::from_str("scout").unwrap(),
        SubagentRole::Scout
    );
    assert_eq!(
        SubagentRole::from_str("CODER").unwrap(),
        SubagentRole::Coder
    );
    assert_eq!(
        SubagentRole::from_str("tester").unwrap(),
        SubagentRole::Tester
    );
    assert_eq!(
        SubagentRole::from_str("reviewer").unwrap(),
        SubagentRole::Reviewer
    );
    assert_eq!(
        SubagentRole::from_str("unknown").unwrap(),
        SubagentRole::General
    );
    assert_eq!(format!("{}", SubagentRole::Scout), "Scout");
    assert_eq!(format!("{}", SubagentRole::Coder), "Coder");

    // Custom role system prompt
    let custom = SubagentRole::Custom {
        name: "SecurityAuditor".to_string(),
        prompt: "Verify no API keys are exposed".to_string(),
    };
    assert_eq!(
        custom.system_prompt("SecurityAuditor"),
        "Verify no API keys are exposed"
    );
}

#[tokio::test]
async fn test_spawn_subagent_tool_and_batch_tool_execution() {
    let server = MockLlmServer::start().await;
    let client = server.client();
    let config = server.config();
    let tools = create_test_tools();

    let tool_ctx = ToolContext::default();

    // 1. Single subagent tool
    let single_tool = SpawnSubagentTool::new(client.clone(), config.clone(), tools.clone());
    assert_eq!(single_tool.name(), "spawn_subagent");

    let single_args = json!({
        "role": "scout",
        "name": "SingleScout",
        "task": "Investigate network protocols"
    });
    let single_output = single_tool
        .execute(single_args, &tool_ctx)
        .await
        .expect("Single subagent tool execute should succeed");
    assert!(!single_output.is_empty());

    // 2. Batch subagent tool
    let batch_tool = SpawnBatchSubagentsTool::new(client, config, tools);
    assert_eq!(batch_tool.name(), "spawn_subagents_batch");

    let batch_args = json!({
        "tasks": [
            { "role": "coder", "name": "BatchCoder1", "task": "Module A refactoring" },
            { "role": "tester", "name": "BatchTester2", "task": "Module B test suite" }
        ]
    });
    let batch_output = batch_tool
        .execute(batch_args, &tool_ctx)
        .await
        .expect("Batch subagents tool execute should succeed");

    assert!(batch_output.contains("Batch subagents completed"));
    assert!(batch_output.contains("BatchCoder1"));
    assert!(batch_output.contains("BatchTester2"));
}

// ===========================================================================
// Part 2: Pub-Sub Broadcast Message Delivery across Topics (AgentMesh)
// ===========================================================================

#[tokio::test]
async fn test_mesh_pub_sub_discovery_and_status_broadcasts() {
    let mesh = AgentMesh::new();

    let scout = mesh
        .register("Scout-Alpha", AgentRole::Scout, "Exploration lead")
        .await
        .expect("register scout");
    let mut coder = mesh
        .register("Coder-Beta", AgentRole::Coder, "Feature engineer")
        .await
        .expect("register coder");
    let mut tester = mesh
        .register("Tester-Gamma", AgentRole::Tester, "QA engineer")
        .await
        .expect("register tester");

    // 1. Broadcast Discovery from Scout
    let file_refs = vec![
        "src/agent/mesh.rs".to_string(),
        "src/agent/subagent.rs".to_string(),
    ];
    mesh.broadcast_discovery(
        "Scout-Alpha",
        "ast_analysis",
        "Identified decentralized mesh channel architecture",
        file_refs.clone(),
    )
    .await
    .expect("broadcast discovery");

    // Both Coder and Tester should receive the discovery broadcast
    let coder_msg = coder.recv_broadcast().await.expect("coder recv broadcast");
    assert_eq!(coder_msg.sender, "Scout-Alpha");
    assert_eq!(coder_msg.topic, topics::DISCOVERY);
    if let BroadcastPayload::Discovery {
        findings,
        file_references,
        ..
    } = coder_msg.payload
    {
        assert!(findings.contains("mesh channel architecture"));
        assert_eq!(file_references, file_refs);
    } else {
        panic!("Expected discovery payload for coder");
    }

    let tester_msg = tester
        .recv_broadcast()
        .await
        .expect("tester recv broadcast");
    assert_eq!(tester_msg.sender, "Scout-Alpha");
    assert_eq!(tester_msg.topic, topics::DISCOVERY);

    // 2. Broadcast Status updates across different AgentStatus states
    scout
        .broadcast_status(AgentStatus::Active {
            task: "Indexing AST symbols".to_string(),
        })
        .await
        .expect("broadcast active status");

    let status_msg = coder.recv_broadcast().await.expect("coder recv status");
    assert_eq!(status_msg.sender, "Scout-Alpha");
    assert_eq!(status_msg.topic, topics::STATUS);
    assert_eq!(
        status_msg.payload,
        BroadcastPayload::Status {
            status: AgentStatus::Active {
                task: "Indexing AST symbols".to_string()
            }
        }
    );

    // Progress status update
    coder
        .broadcast_status(AgentStatus::Progress {
            step: 3,
            total: Some(10),
            message: "Refactored message envelopes".to_string(),
        })
        .await
        .expect("broadcast progress status");

    let progress_msg = tester.recv_broadcast().await.expect("tester recv progress");
    assert_eq!(progress_msg.sender, "Coder-Beta");
    if let BroadcastPayload::Status {
        status:
            AgentStatus::Progress {
                step,
                total,
                message,
            },
    } = progress_msg.payload
    {
        assert_eq!(step, 3);
        assert_eq!(total, Some(10));
        assert!(message.contains("Refactored message envelopes"));
    } else {
        panic!("Expected progress status payload");
    }

    // Completed status update
    coder
        .broadcast_status(AgentStatus::Completed {
            result: Some("All 10 steps finalized with tests".to_string()),
        })
        .await
        .expect("broadcast completed status");

    let completed_msg = tester
        .recv_broadcast()
        .await
        .expect("tester recv completed");
    assert_eq!(completed_msg.sender, "Coder-Beta");
    if let BroadcastPayload::Status {
        status: AgentStatus::Completed { result },
    } = completed_msg.payload
    {
        assert!(result.unwrap().contains("finalized"));
    } else {
        panic!("Expected completed status payload");
    }
}

#[tokio::test]
async fn test_mesh_pub_sub_multi_peer_fanout() {
    let mesh = AgentMesh::new();

    // Register 5 distinct peers across various roles
    let coordinator = mesh
        .register("Coordinator", AgentRole::Orchestrator, "Lead")
        .await
        .unwrap();
    let mut peer_a = mesh
        .register("Worker-A", AgentRole::Coder, "Worker A")
        .await
        .unwrap();
    let mut peer_b = mesh
        .register("Worker-B", AgentRole::Coder, "Worker B")
        .await
        .unwrap();
    let mut peer_c = mesh
        .register("Worker-C", AgentRole::Tester, "Worker C")
        .await
        .unwrap();
    let mut peer_d = mesh
        .register("Worker-D", AgentRole::Reviewer, "Worker D")
        .await
        .unwrap();

    let test_payload = BroadcastPayload::Alert {
        severity: "warning".to_string(),
        message: "High memory utilization in buffer cache".to_string(),
    };

    coordinator
        .broadcast(topics::ALERT, test_payload.clone())
        .await
        .expect("broadcast alert to all");

    // Verify all 4 listening workers receive the exact same alert broadcast
    let msg_a = peer_a.recv_broadcast().await.unwrap();
    let msg_b = peer_b.recv_broadcast().await.unwrap();
    let msg_c = peer_c.recv_broadcast().await.unwrap();
    let msg_d = peer_d.recv_broadcast().await.unwrap();

    for msg in &[msg_a, msg_b, msg_c, msg_d] {
        assert_eq!(msg.sender, "Coordinator");
        assert_eq!(msg.topic, topics::ALERT);
        assert_eq!(msg.payload, test_payload);
    }

    // Verify recent broadcast buffer has recorded the broadcast
    let recent = mesh.recent_broadcasts().await;
    assert!(recent
        .iter()
        .any(|m| m.sender == "Coordinator" && m.topic == topics::ALERT));
}

#[tokio::test]
async fn test_mesh_pub_sub_alerts_facts_and_custom_topics() {
    let mesh = AgentMesh::new();

    let publisher = mesh
        .register("Publisher", AgentRole::General, "Pub")
        .await
        .unwrap();
    let mut subscriber = mesh
        .register("Subscriber", AgentRole::General, "Sub")
        .await
        .unwrap();

    // 1. Alert payload
    publisher
        .broadcast(
            topics::ALERT,
            BroadcastPayload::Alert {
                severity: "critical".to_string(),
                message: "Security violation detected".to_string(),
            },
        )
        .await
        .unwrap();

    let alert_msg = subscriber.recv_broadcast().await.unwrap();
    assert_eq!(alert_msg.topic, topics::ALERT);
    if let BroadcastPayload::Alert { severity, message } = alert_msg.payload {
        assert_eq!(severity, "critical");
        assert!(message.contains("Security violation"));
    } else {
        panic!("Expected alert payload");
    }

    // 2. FactUpdate payload
    publisher
        .broadcast(
            topics::COORDINATION,
            BroadcastPayload::FactUpdate {
                key: "database_port".to_string(),
                value: json!(5432),
            },
        )
        .await
        .unwrap();

    let fact_msg = subscriber.recv_broadcast().await.unwrap();
    assert_eq!(fact_msg.topic, topics::COORDINATION);
    if let BroadcastPayload::FactUpdate { key, value } = fact_msg.payload {
        assert_eq!(key, "database_port");
        assert_eq!(value, json!(5432));
    } else {
        panic!("Expected FactUpdate payload");
    }

    // 3. Custom payload
    publisher
        .broadcast(
            "metrics",
            BroadcastPayload::Custom {
                kind: "cpu_usage".to_string(),
                data: json!({ "percent": 42.5 }),
            },
        )
        .await
        .unwrap();

    let custom_msg = subscriber.recv_broadcast().await.unwrap();
    assert_eq!(custom_msg.topic, "metrics");
    if let BroadcastPayload::Custom { kind, data } = custom_msg.payload {
        assert_eq!(kind, "cpu_usage");
        assert_eq!(data["percent"], 42.5);
    } else {
        panic!("Expected Custom payload");
    }
}

// ===========================================================================
// Part 3: Direct Peer-to-Peer Query-Response RPC & Mailbox Delivery
// ===========================================================================

#[tokio::test]
async fn test_mesh_direct_peer_query_response_rpc() {
    let mesh = AgentMesh::new();

    let coder = mesh
        .register("Coder-1", AgentRole::Coder, "Primary implementer")
        .await
        .unwrap();
    let mut scout = mesh
        .register("Scout-1", AgentRole::Scout, "Code locator")
        .await
        .unwrap();

    // Spawn async query responder on Scout
    let scout_handle = tokio::spawn(async move {
        if let Some(envelope) = scout.recv_query().await {
            let query = envelope.query();
            assert_eq!(query.from, "Coder-1");
            assert_eq!(query.to, "Scout-1");
            assert_eq!(query.query, "Locate database migration entrypoint");
            assert_eq!(query.context.as_deref(), Some("Schema v2 upgrade"));

            envelope
                .respond("Database migration defined in src/db/migrations/v2.rs:18")
                .expect("respond to query");
        } else {
            panic!("Scout failed to receive query envelope");
        }
    });

    // Coder asks Scout via RPC
    let response: PeerResponse = coder
        .ask(
            "Scout-1",
            "Locate database migration entrypoint",
            Some("Schema v2 upgrade".to_string()),
            Some(Duration::from_secs(5)),
        )
        .await
        .expect("RPC query response should succeed");

    assert!(response.success);
    assert_eq!(response.from, "Scout-1");
    assert_eq!(response.to, "Coder-1");
    assert!(response.answer.contains("src/db/migrations/v2.rs:18"));

    scout_handle.await.unwrap();
}

#[tokio::test]
async fn test_mesh_direct_query_timeout_handling() {
    let mesh = AgentMesh::new();

    let caller = mesh
        .register("Caller-Agent", AgentRole::Coder, "Caller")
        .await
        .unwrap();
    let _unresponsive = mesh
        .register("Unresponsive-Agent", AgentRole::Scout, "Silent")
        .await
        .unwrap();

    // Query with an aggressive 30ms timeout where recipient never answers
    let start = std::time::Instant::now();
    let result = caller
        .ask(
            "Unresponsive-Agent",
            "Are you responsive?",
            None,
            Some(Duration::from_millis(30)),
        )
        .await;

    let elapsed = start.elapsed();
    assert!(elapsed >= Duration::from_millis(25));
    assert!(
        matches!(&result, Err(MeshError::QueryTimeout { to, .. }) if to == "Unresponsive-Agent"),
        "Expected QueryTimeout error, got: {:?}",
        result
    );
}

#[tokio::test]
async fn test_mesh_direct_query_peer_not_found_and_disconnected() {
    let mesh = AgentMesh::new();

    let caller = mesh
        .register("Caller-Peer", AgentRole::Coder, "Active caller")
        .await
        .unwrap();

    // Query non-existent peer
    let not_found = caller
        .ask(
            "NonExistentAgent",
            "Hello?",
            None,
            Some(Duration::from_secs(1)),
        )
        .await;
    assert!(
        matches!(&not_found, Err(MeshError::PeerNotFound(id)) if id == "NonExistentAgent"),
        "Expected PeerNotFound, got: {:?}",
        not_found
    );

    // Direct message to non-existent peer
    let msg_err = caller
        .send_direct("NonExistentAgent", "Ping", "Are you there?")
        .await;
    assert!(matches!(msg_err, Err(MeshError::PeerNotFound(id)) if id == "NonExistentAgent"));
}

#[tokio::test]
async fn test_mesh_direct_point_to_point_messaging_and_threading() {
    let mesh = AgentMesh::new();

    let peer_a = mesh
        .register("Agent-A", AgentRole::Coder, "Initiator")
        .await
        .unwrap();
    let mut peer_b = mesh
        .register("Agent-B", AgentRole::Tester, "Receiver")
        .await
        .unwrap();

    // Send direct message with custom structured payload
    let direct_msg = DirectMessage::new(
        "Agent-A",
        "Agent-B",
        "Test Plan Handoff",
        "Refactoring of authentication module complete. Please run integration suite.",
    )
    .with_payload(json!({
        "test_target": "tests/auth_test.rs",
        "coverage_required": 0.95
    }));
    mesh.send_direct(direct_msg.clone())
        .await
        .expect("send direct message");

    let received = peer_b.recv_direct().await.expect("receive direct message");
    assert_eq!(received.from, "Agent-A");
    assert_eq!(received.to, "Agent-B");
    assert_eq!(received.subject, "Test Plan Handoff");
    assert!(received.content.contains("authentication module"));
    assert_eq!(received.payload["test_target"], "tests/auth_test.rs");

    // Reply from Peer B with conversation threading (reply_to)
    let reply_msg = DirectMessage::new(
        "Agent-B",
        "Agent-A",
        "Test Plan Received",
        "Starting test runner now with 95% threshold.",
    )
    .with_reply_to(received.id.clone());
    assert_eq!(reply_msg.reply_to, Some(received.id));
    assert_eq!(reply_msg.to, "Agent-A");
    mesh.send_direct(reply_msg)
        .await
        .expect("send reply message");
}

#[tokio::test]
async fn test_mesh_mailbox_fifo_ordering_and_capacity() {
    let mesh = AgentMesh::new();

    let sender = mesh
        .register("BurstSender", AgentRole::General, "Sender")
        .await
        .unwrap();
    let mut receiver = mesh
        .register("BurstReceiver", AgentRole::General, "Receiver")
        .await
        .unwrap();

    // Send 10 sequential messages
    for i in 0..10 {
        sender
            .send_direct(
                "BurstReceiver",
                &format!("Message-{}", i),
                &format!("Sequential payload content {}", i),
            )
            .await
            .expect("send sequential message");
    }

    // Verify strict FIFO order on receiver mailbox
    for i in 0..10 {
        let msg = receiver.recv_direct().await.expect("recv msg");
        assert_eq!(msg.subject, format!("Message-{}", i));
        assert_eq!(msg.content, format!("Sequential payload content {}", i));
    }
}

// ===========================================================================
// Part 4: Advisory Committee Evaluation Flow & Risk Assessment Consensus
// ===========================================================================

#[tokio::test]
async fn test_advisory_committee_evaluation_flow() {
    let server = MockLlmServer::start().await;
    let client = server.client();
    let config = server.config();

    // 1. Standard Advisory Committee setup (Architecture, Security, Code Quality)
    let committee = AdvisorRegistry::default_advisors();
    assert_eq!(committee.len(), 3);
    assert!(committee.get("ArchitectureAdvisor").is_some());
    assert!(committee.get("SecurityAdvisor").is_some());
    assert!(committee.get("CodeReviewAdvisor").is_some());

    let engine = AdvisorEngine::from_registry((*client).clone(), config.clone(), &committee);

    let critiques = engine
        .consult(
            "Refactor inter-agent channel buffer architecture",
            "Use bounded MPSC mailboxes with explicit backpressure and zero global locks",
        )
        .await;

    assert_eq!(
        critiques.len(),
        3,
        "All 3 committee advisors must participate"
    );
    for c in &critiques {
        assert!(!c.advisor.is_empty());
        assert!(!c.focus.is_empty());
        assert!(!c.critique.is_empty());
    }

    assert!(AdvisorEngine::is_all_approved(&critiques));
    assert_eq!(AdvisorEngine::highest_risk(&critiques), RiskLevel::Low);
    assert!(!AdvisorEngine::has_critical_risk(&critiques));
}

#[tokio::test]
async fn test_advisory_committee_custom_mesh_handler() {
    let mesh = AgentMesh::new();

    // Custom review handler simulating high-security committee
    struct StrictSecurityCommittee;

    #[async_trait]
    impl AdvisorReviewHandler for StrictSecurityCommittee {
        async fn handle_review(
            &self,
            req: &AdvisorReviewRequest,
        ) -> Result<AdvisorReviewResponse, MeshError> {
            let approved = !req.diff_or_plan.contains("unsafe");
            let risk_level = if approved {
                RiskLevel::Low
            } else {
                RiskLevel::Critical
            };
            let critique = if approved {
                "Memory safety and clean bounds verified.".to_string()
            } else {
                "Unsafe block detected. Zero unsafe allowed under policy.".to_string()
            };

            let critiques = vec![AdvisorCritique {
                advisor: "StrictSecurityCommittee".to_string(),
                focus: "Memory Safety".to_string(),
                approved,
                risk_level,
                critique,
                suggestions: if approved {
                    vec![]
                } else {
                    vec!["Rewrite without unsafe".to_string()]
                },
            }];

            Ok(AdvisorReviewResponse::from_critiques(
                req.request_id.clone(),
                critiques,
            ))
        }
    }

    mesh.set_advisor_handler(Arc::new(StrictSecurityCommittee))
        .await;

    let peer = mesh
        .register("DevAgent", AgentRole::Coder, "Dev")
        .await
        .unwrap();

    // 1. Safe review request
    let safe_resp = peer
        .request_review(
            "Add safe parser",
            "pub fn parse(s: &str) -> Option<i32> { s.parse().ok() }",
            None,
        )
        .await
        .expect("safe review");
    assert!(safe_resp.approved);
    assert_eq!(safe_resp.highest_risk, RiskLevel::Low);

    // 2. Unsafe review request
    let unsafe_resp = peer
        .request_review("Add pointer deref", "unsafe { *ptr = 42; }", None)
        .await
        .expect("unsafe review");
    assert!(!unsafe_resp.approved);
    assert_eq!(unsafe_resp.highest_risk, RiskLevel::Critical);
    assert!(unsafe_resp.summary.contains("Unsafe block detected"));
    assert!(unsafe_resp
        .critiques
        .iter()
        .any(|c| c.suggestions.iter().any(|s| s.contains("unsafe"))));
}

#[tokio::test]
async fn test_advisory_committee_heuristic_evaluations() {
    let mesh = AgentMesh::new();
    let peer = mesh
        .register("Worker", AgentRole::Coder, "Worker")
        .await
        .unwrap();

    // 1. Safe change
    let safe_resp = peer
        .request_review(
            "Add helper function",
            "pub fn add(a: i32, b: i32) -> i32 { a + b }",
            None,
        )
        .await
        .expect("safe review");
    assert!(safe_resp.approved);
    assert_eq!(safe_resp.highest_risk, RiskLevel::Low);

    // 2. Destructive command violation
    let dangerous_resp = peer
        .request_review(
            "Cleanup disk",
            "bash::execute: rm -rf /",
            Some("SecurityAdvisor"),
        )
        .await
        .expect("dangerous review");
    assert!(!dangerous_resp.approved);
    assert_eq!(dangerous_resp.highest_risk, RiskLevel::Critical);
    assert!(dangerous_resp.summary.contains("Catastrophic command"));

    // 3. Secret credential leak violation
    let secret_resp = peer
        .request_review(
            "Hardcode credentials",
            "const API_KEY: &str = \"sk-ant-api03-abcdef123456\";",
            Some("SecurityAdvisor"),
        )
        .await
        .expect("secret review");
    assert!(!secret_resp.approved);
    assert_eq!(secret_resp.highest_risk, RiskLevel::High);
    assert!(secret_resp
        .summary
        .contains("secret or private key leakage"));
}

#[tokio::test]
async fn test_advisor_risk_level_evaluation_and_blocking() {
    let server = MockLlmServer::start().await;
    let client = server.client();
    let config = server.config();

    let engine = AdvisorEngine::new((*client).clone(), config);

    // Dangerous command triggers Critical risk on mock server
    let dangerous_critiques = engine
        .consult(
            "Execute maintenance cleanup",
            "Run destructive script: rm -rf /var/data and delete database records",
        )
        .await;

    assert_eq!(dangerous_critiques.len(), 3);
    assert!(
        AdvisorEngine::has_critical_risk(&dangerous_critiques),
        "Must flag critical risk for dangerous command"
    );
    assert!(
        AdvisorEngine::has_high_or_critical_risk(&dangerous_critiques),
        "Must flag high or critical risk"
    );
    assert!(
        !AdvisorEngine::is_all_approved(&dangerous_critiques),
        "Dangerous critiques should NOT all be approved"
    );
    assert_eq!(
        AdvisorEngine::highest_risk(&dangerous_critiques),
        RiskLevel::Critical
    );

    // Verify RiskLevel ordering and methods
    assert!(RiskLevel::Low < RiskLevel::Medium);
    assert!(RiskLevel::Medium < RiskLevel::High);
    assert!(RiskLevel::High < RiskLevel::Critical);
    assert!(RiskLevel::Critical.is_critical());
    assert!(RiskLevel::Critical.is_high_or_critical());
    assert!(RiskLevel::High.is_high_or_critical());
    assert!(!RiskLevel::Medium.is_high_or_critical());
}

#[tokio::test]
async fn test_advisor_consensus_resolution_algorithms() {
    let critiques = vec![
        AdvisorCritique {
            advisor: "ArchitectureAdvisor".to_string(),
            focus: "Architecture".to_string(),
            approved: true,
            risk_level: RiskLevel::Low,
            critique: "Well modularized design".to_string(),
            suggestions: vec!["Keep interfaces lean".to_string()],
        },
        AdvisorCritique {
            advisor: "SecurityAdvisor".to_string(),
            focus: "Security".to_string(),
            approved: true,
            risk_level: RiskLevel::Medium,
            critique: "Network port opened; verify TLS certificates".to_string(),
            suggestions: vec!["Enforce TLS 1.3".to_string()],
        },
        AdvisorCritique {
            advisor: "CodeReviewAdvisor".to_string(),
            focus: "Code Quality".to_string(),
            approved: false,
            risk_level: RiskLevel::Low,
            critique: "Missing unit test for error edge case".to_string(),
            suggestions: vec!["Add test_invalid_input".to_string()],
        },
    ];

    // 1. Majority Voting (2 approved, 1 disapproved -> Approved)
    let majority_res = resolve_majority(&critiques);
    assert!(majority_res.is_approved());
    assert_eq!(majority_res.approved_count, 2);
    assert_eq!(majority_res.rejected_count, 1);
    assert_eq!(majority_res.total_advisors, 3);
    assert_eq!(majority_res.highest_risk, RiskLevel::Medium);
    assert_eq!(majority_res.recommendations.len(), 3);

    // 2. Unanimous Voting (1 disapproved -> Rejected)
    let unanimous_res = resolve_unanimous(&critiques);
    assert!(!unanimous_res.is_approved());
    assert!(unanimous_res.has_veto());
    assert!(unanimous_res.veto_reasons[0].contains("CodeReviewAdvisor"));

    // 3. Risk-Weighted Voting (2 approvals with Low/Medium vs 1 Low disapproval -> Approved)
    let risk_res = resolve_risk_weighted(&critiques);
    assert!(risk_res.is_approved());
    assert!(risk_res.confidence >= 0.5);

    // 4. Security-First Veto Test
    let security_critiques = vec![
        AdvisorCritique {
            advisor: "ArchitectureAdvisor".to_string(),
            focus: "Architecture".to_string(),
            approved: true,
            risk_level: RiskLevel::Low,
            critique: "Architecture is clean".to_string(),
            suggestions: vec![],
        },
        AdvisorCritique {
            advisor: "SecurityAdvisor".to_string(),
            focus: "Security".to_string(),
            approved: false,
            risk_level: RiskLevel::High,
            critique: "Insecure cipher suite detected".to_string(),
            suggestions: vec!["Use ChaCha20-Poly1305".to_string()],
        },
        AdvisorCritique {
            advisor: "CodeReviewAdvisor".to_string(),
            focus: "Code Quality".to_string(),
            approved: true,
            risk_level: RiskLevel::Low,
            critique: "Clean code structure".to_string(),
            suggestions: vec![],
        },
    ];

    let sec_veto_res = resolve_security_veto(&security_critiques);
    assert!(
        !sec_veto_res.is_approved(),
        "Security rejection must exercise veto"
    );
    assert!(sec_veto_res.has_veto());

    // 5. Critical Risk Auto-Veto Test
    let critical_critiques = vec![
        AdvisorCritique {
            advisor: "ArchitectureAdvisor".to_string(),
            focus: "Architecture".to_string(),
            approved: true,
            risk_level: RiskLevel::Low,
            critique: "Structure is okay".to_string(),
            suggestions: vec![],
        },
        AdvisorCritique {
            advisor: "SecurityAdvisor".to_string(),
            focus: "Security".to_string(),
            approved: false,
            risk_level: RiskLevel::Critical,
            critique: "Detected raw root command execution".to_string(),
            suggestions: vec!["Block execution".to_string()],
        },
    ];

    let critical_res = resolve_majority(&critical_critiques);
    assert!(
        !critical_res.is_approved(),
        "Critical risk must auto-veto even majority"
    );
    assert!(critical_res.is_critical());
    assert!(critical_res.has_veto());

    // 6. AdvisorEngine Integration
    let engine_res = AdvisorEngine::resolve_consensus(&critiques, ConsensusStrategy::Majority);
    assert!(engine_res.is_approved());
    assert!(engine_res.status_badge().contains("APPROVED"));
}

#[tokio::test]
async fn test_advisor_consensus_policy_and_engine() {
    let critiques = vec![
        AdvisorCritique {
            advisor: "ArchitectureAdvisor".to_string(),
            focus: "Architecture".to_string(),
            approved: true,
            risk_level: RiskLevel::Low,
            critique: "Clean structure".to_string(),
            suggestions: vec![],
        },
        AdvisorCritique {
            advisor: "SecurityAdvisor".to_string(),
            focus: "Security".to_string(),
            approved: true,
            risk_level: RiskLevel::Low,
            critique: "No vulnerabilities".to_string(),
            suggestions: vec![],
        },
        AdvisorCritique {
            advisor: "CodeReviewAdvisor".to_string(),
            focus: "Code Quality".to_string(),
            approved: false,
            risk_level: RiskLevel::Low,
            critique: "Need additional comments".to_string(),
            suggestions: vec![],
        },
    ];

    // Supermajority 2/3 (66.7%) threshold: 2/3 approved = 66.7% -> Passes
    let policy_super = ConsensusPolicy::supermajority(0.66);
    let engine = ConsensusEngine::new(policy_super);
    let res = engine.resolve(&critiques);
    assert!(res.is_approved());

    // Supermajority 3/4 (75%) threshold: 2/3 approved = 66.7% < 75% -> Fails
    let policy_strict_super = ConsensusPolicy::supermajority(0.75);
    let strict_engine = ConsensusEngine::new(policy_strict_super);
    let strict_res = strict_engine.resolve(&critiques);
    assert!(!strict_res.is_approved());
}

#[tokio::test]
async fn test_advisor_markdown_and_summary_formatting() {
    let critiques = vec![
        AdvisorCritique {
            advisor: "SecurityAdvisor".to_string(),
            focus: "Command safety".to_string(),
            approved: false,
            risk_level: RiskLevel::Critical,
            critique: "Arbitrary command injection vulnerability detected.".to_string(),
            suggestions: vec!["Sanitize shell arguments".to_string()],
        },
        AdvisorCritique {
            advisor: "ArchitectureAdvisor".to_string(),
            focus: "Modularity".to_string(),
            approved: true,
            risk_level: RiskLevel::Low,
            critique: "Layer separation is acceptable.".to_string(),
            suggestions: vec![],
        },
    ];

    let prompt_md = format_critiques_for_system_prompt(&critiques);
    assert!(prompt_md.contains("### Advisor Critiques & Safety Notes:"));
    assert!(prompt_md.contains("[FLAGGED/WARNING]"));
    assert!(prompt_md.contains("[APPROVED]"));
    assert!(prompt_md.contains("Suggestion: Sanitize shell arguments"));

    let summary = format_critiques_summary(&critiques);
    assert!(summary.contains("SecurityAdvisor: ⚠ (CRITICAL)"));
    assert!(summary.contains("ArchitectureAdvisor: ✓ (LOW)"));
}

#[tokio::test]
async fn test_advisor_fallback_and_fault_tolerance() {
    let server = MockLlmServer::start().await;
    server
        .set_handler(|_| {
            MockResponse::Text(
                "CAUTION: This implementation raises a medium risk warning.".to_string(),
            )
        })
        .await;

    let client = server.client();
    let config = server.config();
    let engine = AdvisorEngine::new((*client).clone(), config);

    let critiques = engine
        .consult(
            "Perform edge-case operation",
            "Action with plain text fallback",
        )
        .await;

    assert_eq!(critiques.len(), 3);
    for c in &critiques {
        assert_eq!(c.risk_level, RiskLevel::Medium);
    }

    // Test server error response (500)
    server
        .set_handler(|_| MockResponse::Error(500, "Internal Server Outage".to_string()))
        .await;

    let error_critiques = engine
        .consult("Check resilience", "Action during server outage")
        .await;

    assert_eq!(error_critiques.len(), 3);
    for c in &error_critiques {
        assert!(c.approved, "Advisor fallback on error defaults to approved");
        assert_eq!(c.risk_level, RiskLevel::Low);
        assert!(c.critique.contains("Advisor consultation unavailable"));
    }
}

#[tokio::test]
async fn test_advisor_disabled_configuration() {
    let server = MockLlmServer::start().await;
    let client = server.client();
    let mut config = server.config();
    config.advisors_enabled = false;
    let advisors = AdvisorRegistry::default_advisors();
    let direct_critiques = consult_advisors(
        advisors.all(),
        "User query",
        "Proposed action",
        &client,
        &config,
    )
    .await;
    assert!(
        direct_critiques.is_empty(),
        "Disabled advisors must return empty results immediately"
    );

    let engine = AdvisorEngine::new((*client).clone(), config);
    let critiques = engine.consult("User query", "Proposed action").await;

    assert!(
        critiques.is_empty(),
        "Disabled advisors must return empty results immediately"
    );
    assert_eq!(
        server.request_count(),
        0,
        "No HTTP requests should be sent when advisors disabled"
    );
}

// ===========================================================================
// Part 5: Multi-Agent Coordination Mesh Primitives & LLM Tools
// ===========================================================================

#[tokio::test]
async fn test_mesh_resource_claims_and_conflict_resolution() {
    let mesh = AgentMesh::new();

    let peer_a = mesh
        .register("Coder-A", AgentRole::Coder, "Coder A")
        .await
        .unwrap();
    let peer_b = mesh
        .register("Coder-B", AgentRole::Coder, "Coder B")
        .await
        .unwrap();

    let target_file = "src/agent/mesh.rs";

    // Peer A claims target file
    peer_a
        .claim_resource(target_file, None)
        .await
        .expect("claim file");

    // Peer B attempts to claim the same file -> rejected with ResourceAlreadyClaimed
    let conflict = peer_b.claim_resource(target_file, None).await;
    assert!(matches!(
        conflict,
        Err(MeshError::ResourceAlreadyClaimed { claimed_by, .. }) if claimed_by == "Coder-A"
    ));

    // Peer A releases the file lock
    let released = peer_a
        .release_resource(target_file)
        .await
        .expect("release file");
    assert!(released);

    // Peer B can now claim the file
    peer_b
        .claim_resource(target_file, None)
        .await
        .expect("peer b claim");

    // Auto-release on unregister: When peer B unregisters, its lock is cleared
    peer_b.unregister().await.expect("unregister peer b");
    let claims = mesh.get_resource_claims().await;
    assert!(!claims.contains_key(target_file));
}

#[tokio::test]
async fn test_mesh_shared_blackboard_facts() {
    let mesh = AgentMesh::new();

    let peer_a = mesh
        .register("Scout-1", AgentRole::Scout, "Scout")
        .await
        .unwrap();

    // Set fact on blackboard
    let rev1 = peer_a
        .set_fact("build_target", json!({ "arch": "arm64", "os": "darwin" }))
        .await
        .expect("set fact");
    assert_eq!(rev1, 1);

    let fact = peer_a.get_fact("build_target").await.expect("get fact");
    assert_eq!(fact.author, "Scout-1");
    assert_eq!(fact.revision, 1);
    assert_eq!(fact.value["arch"], "arm64");

    // Overwrite fact with incremented revision
    let rev2 = peer_a
        .set_fact("build_target", json!({ "arch": "wasm32", "os": "unknown" }))
        .await
        .expect("update fact");
    assert_eq!(rev2, 2);

    let updated = peer_a
        .get_fact("build_target")
        .await
        .expect("get updated fact");
    assert_eq!(updated.revision, 2);
    assert_eq!(updated.value["arch"], "wasm32");
}

#[tokio::test]
async fn test_mesh_synchronization_barrier() {
    let mesh = AgentMesh::new();

    mesh.create_barrier("phase_1_complete", 3).await;

    let m1 = mesh.clone();
    let m2 = mesh.clone();
    let m3 = mesh.clone();

    let h1 = tokio::spawn(async move {
        m1.wait_barrier("phase_1_complete", "Worker-1", Duration::from_secs(2))
            .await
    });
    let h2 = tokio::spawn(async move {
        m2.wait_barrier("phase_1_complete", "Worker-2", Duration::from_secs(2))
            .await
    });
    let h3 = tokio::spawn(async move {
        m3.wait_barrier("phase_1_complete", "Worker-3", Duration::from_secs(2))
            .await
    });

    let (r1, r2, r3) = tokio::join!(h1, h2, h3);
    assert!(r1.unwrap().is_ok());
    assert!(r2.unwrap().is_ok());
    assert!(r3.unwrap().is_ok());
}

#[tokio::test]
async fn test_mesh_llm_tools_execution() {
    let mesh = AgentMesh::new();
    let ctx = ToolContext::default();

    let _scout = mesh
        .register("ScoutAgent", AgentRole::Scout, "Scout")
        .await
        .unwrap();

    // 1. MeshListPeersTool
    let list_tool = MeshListPeersTool::new(mesh.clone());
    let list_out = list_tool.execute(json!({}), &ctx).await.unwrap();
    assert!(list_out.contains("ScoutAgent"));

    // 2. MeshBroadcastTool
    let broadcast_tool = MeshBroadcastTool::new(mesh.clone(), "ScoutAgent");
    let bcast_out = broadcast_tool
        .execute(
            json!({
                "topic": "discovery",
                "message": "Discovered 14 source modules in src/agent/",
                "file_references": ["src/agent/mesh.rs"]
            }),
            &ctx,
        )
        .await
        .unwrap();
    assert!(bcast_out.contains("discovery"));

    // 3. MeshClaimResourceTool
    let claim_tool = MeshClaimResourceTool::new(mesh.clone(), "ScoutAgent");
    let claim_out = claim_tool
        .execute(
            json!({
                "action": "claim",
                "resource": "src/agent/mod.rs"
            }),
            &ctx,
        )
        .await
        .unwrap();
    assert!(claim_out.contains("successfully claimed"));

    // 4. MeshRequestReviewTool
    let review_tool = MeshRequestReviewTool::new(mesh.clone(), "ScoutAgent");
    let review_out = review_tool
        .execute(
            json!({
                "subject": "Add utility function",
                "diff_or_plan": "pub fn util() {}"
            }),
            &ctx,
        )
        .await
        .unwrap();
    assert!(review_out.contains("REVIEW APPROVED"));
}

// ===========================================================================
// Part 6: Session Persistence & Resumption Tests
// ===========================================================================

#[test]
fn test_session_lifecycle_and_state_tracking() {
    let mut session = Session::new("claude-3-7-sonnet");
    assert_eq!(session.active_model(), "claude-3-7-sonnet");
    assert_eq!(session.total_messages(), 0);

    // Configure system prompt and metadata
    session.set_system_prompt("You are Fusion in strict test mode.");
    let mut meta = HashMap::new();
    meta.insert("env".to_string(), "integration_test".to_string());
    meta.insert("user_id".to_string(), "test_user_42".to_string());
    for (k, v) in meta {
        session.set_metadata(k, v);
    }

    // Add conversation history
    session.add_message(Message::user("Implement binary tree inversion"));
    assert_eq!(session.total_messages(), 1);
    assert_eq!(session.title(), Some("Implement binary tree inversion"));
    let tool_call = ToolCall {
        id: "call_invert_tree_1".to_string(),
        name: "write".to_string(),
        arguments: "{\"path\":\"tree.rs\"}".to_string(),
    };
    session.add_assistant_with_tools("Writing implementation...", vec![tool_call]);
    session.add_tool_result("call_invert_tree_1", "Successfully written tree.rs");
    session.add_assistant_message("Tree inversion implemented cleanly.");

    assert_eq!(session.total_messages(), 4);

    // Record token statistics
    session.record_tokens(Some(120), Some(45));
    session.record_usage(80, 30);
    session.token_stats_mut().record_cache(500, 100);

    let stats: &TokenStats = session.token_stats();
    assert_eq!(stats.prompt_tokens, 200);
    assert_eq!(stats.completion_tokens, 75);
    assert_eq!(stats.cache_read_tokens, 500);
    assert_eq!(stats.cache_write_tokens, 100);
    assert_eq!(stats.total_turns, 2);

    assert!(stats.format_summary().contains("275 total"));
}

#[test]
fn test_session_save_and_load_roundtrip() {
    let temp_dir = TempDir::new("session_persist");
    let session_file = temp_dir.path().join("test_session_roundtrip.json");

    let mut original = Session::with_system_prompt("gpt-4o", "Act as a senior Rust architect.");
    original.set_title("Architecture Review Session");
    original.set_metadata("project", "fusion_v2");
    original.set_working_dir(temp_dir.path());

    original.add_user_message("Evaluate our channel buffer topology.");
    original.add_assistant_message("Bounded MPSC with backpressure is recommended.");
    original.record_tokens(Some(150), Some(60));

    // Save to explicit path
    let saved_path = original
        .save_to_path(&session_file)
        .expect("save_to_path should succeed");
    assert_eq!(saved_path, session_file);
    assert!(session_file.exists());

    // Load from explicit path
    let loaded = Session::load_from_path(&session_file).expect("load_from_path should succeed");

    assert_eq!(loaded.id(), original.id());
    assert_eq!(loaded.active_model(), original.active_model());
    assert_eq!(loaded.title(), original.title());
    assert_eq!(loaded.system_prompt(), original.system_prompt());
    assert_eq!(loaded.metadata(), original.metadata());
    assert_eq!(loaded.total_messages(), 2);
    assert_eq!(
        loaded.messages()[0].content,
        "Evaluate our channel buffer topology."
    );
    assert_eq!(
        loaded.messages()[1].content,
        "Bounded MPSC with backpressure is recommended."
    );
    assert_eq!(loaded.token_stats().total_tokens, 210);
}

#[test]
fn test_session_resumption_and_continuation() {
    let temp_dir = TempDir::new("session_resume");
    let session_file = temp_dir.path().join("resumption_test.json");

    // Phase 1: Initial conversation
    let mut session = Session::new("claude-3-7-sonnet");
    session.add_user_message("Turn 1: Initialize database pool");
    session.add_assistant_message("Pool initialized with 10 connections.");
    session.record_tokens(Some(100), Some(50));
    session.save_to_path(&session_file).expect("Save phase 1");

    let initial_updated_at = session.updated_at().to_string();

    std::thread::sleep(Duration::from_millis(15));

    // Phase 2: Resume session from disk
    let mut resumed = Session::load_from_path(&session_file).expect("Load resumed");
    resumed.touch();
    assert_ne!(resumed.updated_at(), initial_updated_at);

    // Continue conversation on resumed session
    resumed.add_user_message("Turn 2: Run migration v2");
    resumed.add_assistant_message("Migration v2 applied successfully.");
    resumed.record_tokens(Some(120), Some(40));
    resumed.save_to_path(&session_file).expect("Save phase 2");

    // Phase 3: Final verification
    let final_session = Session::load_from_path(&session_file).expect("Load final");
    assert_eq!(final_session.total_messages(), 4);
    assert_eq!(final_session.token_stats().prompt_tokens, 220);
    assert_eq!(final_session.token_stats().completion_tokens, 90);
    assert_eq!(final_session.token_stats().total_tokens, 310);
    assert_eq!(final_session.token_stats().total_turns, 2);
}

#[test]
fn test_session_prefix_lookup_simulation() {
    let temp_dir = TempDir::new("session_prefix");

    let session1 = Session::with_id(
        Uuid::parse_str("aaaaaaaa-1111-2222-3333-444444444444").unwrap(),
        "model-a",
    );
    let session2 = Session::with_id(
        Uuid::parse_str("bbbbbbbb-1111-2222-3333-444444444444").unwrap(),
        "model-b",
    );
    let session3 = Session::with_id(
        Uuid::parse_str("bbbbcccc-1111-2222-3333-444444444444").unwrap(),
        "model-c",
    );

    session1
        .save_to_path(temp_dir.path().join(format!("{}.json", session1.id())))
        .unwrap();
    session2
        .save_to_path(temp_dir.path().join(format!("{}.json", session2.id())))
        .unwrap();
    session3
        .save_to_path(temp_dir.path().join(format!("{}.json", session3.id())))
        .unwrap();

    let find_in_dir = |prefix: &str| -> anyhow::Result<Option<Session>> {
        let clean = prefix.trim().to_lowercase();
        let mut matches = Vec::new();
        for entry in std::fs::read_dir(temp_dir.path())? {
            let entry = entry?;
            let path = entry.path();
            if path.extension().and_then(|s| s.to_str()) == Some("json") {
                if let Some(stem) = path.file_stem().and_then(|s| s.to_str()) {
                    if stem.to_lowercase().starts_with(&clean) {
                        matches.push(path);
                    }
                }
            }
        }
        if matches.is_empty() {
            Ok(None)
        } else if matches.len() == 1 {
            let s = Session::load_from_path(&matches[0])?;
            Ok(Some(s))
        } else {
            anyhow::bail!("Ambiguous prefix");
        }
    };

    let found1 = find_in_dir("aaaa").unwrap();
    assert!(found1.is_some());
    assert_eq!(found1.unwrap().id(), session1.id());

    let found3 = find_in_dir("bbbbcccc").unwrap();
    assert!(found3.is_some());
    assert_eq!(found3.unwrap().id(), session3.id());

    let amb = find_in_dir("bbbb");
    assert!(amb.is_err(), "Expected ambiguous error for 'bbbb'");

    let none = find_in_dir("zzzz").unwrap();
    assert!(none.is_none());
}

#[test]
fn test_session_listing_and_summary_generation() {
    let temp_dir = TempDir::new("session_summaries");

    let mut s1 = Session::new("model-1");
    s1.set_title("First Conversation");
    s1.add_user_message("How do I structure a CLI in Rust?");
    s1.add_assistant_message("Use clap with derive macros for clean subcommands.");
    s1.save_to_path(temp_dir.path().join(format!("{}.json", s1.id())))
        .unwrap();

    std::thread::sleep(Duration::from_millis(15));

    let mut s2 = Session::new("model-2");
    s2.set_title("Second Conversation");
    s2.add_user_message("Write a high-performance HTTP server");
    s2.add_assistant_message("Use axum or hyper with pure Rust TLS.");
    s2.save_to_path(temp_dir.path().join(format!("{}.json", s2.id())))
        .unwrap();

    let mut summaries = Vec::new();
    for entry in std::fs::read_dir(temp_dir.path()).unwrap() {
        let entry = entry.unwrap();
        let path = entry.path();
        if path.extension().and_then(|s| s.to_str()) == Some("json") {
            let session = Session::load_from_path(&path).unwrap();
            let preview = session
                .messages()
                .iter()
                .rev()
                .find(|m| m.role == Role::User || m.role == Role::Assistant)
                .map(|m| m.content.chars().take(40).collect::<String>())
                .unwrap_or_else(|| "Empty".to_string());

            summaries.push(SessionSummary {
                id: session.id(),
                created_at: session.created_at().to_string(),
                updated_at: session.updated_at().to_string(),
                active_model: session.active_model().to_string(),
                title: session.title().map(|s| s.to_string()),
                message_count: session.total_messages(),
                preview,
            });
        }
    }

    summaries.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));

    assert_eq!(summaries.len(), 2);
    assert_eq!(
        summaries[0].id,
        s2.id(),
        "Most recent session should be first"
    );
    assert_eq!(summaries[0].title.as_deref(), Some("Second Conversation"));
    assert_eq!(summaries[0].message_count, 2);
    assert_eq!(summaries[1].id, s1.id());
}

#[test]
fn test_session_export_markdown_formatting() {
    let mut session = Session::with_system_prompt("deepseek-chat", "You are an AI assistant.");
    session.set_title("Markdown Export Test");

    session.add_user_message("Please write a config parser.");

    let tool_call = ToolCall {
        id: "call_write_cfg".to_string(),
        name: "write".to_string(),
        arguments: "{\"path\":\"config.toml\",\"content\":\"port = 8080\"}".to_string(),
    };
    session.add_assistant_with_tools("Generating configuration...", vec![tool_call]);
    session.add_tool_result("call_write_cfg", "File written: 12 bytes");
    session.add_assistant_message("Config parser implementation complete.");

    session.record_tokens(Some(90), Some(40));

    let md = session.export_markdown();

    assert!(md.contains("# Markdown Export Test"));
    assert!(md.contains(&format!("Session ID:** `{}`", session.id())));
    assert!(md.contains("Model:** `deepseek-chat`"));
    assert!(md.contains("Tokens:** Tokens: 130 total"));
    assert!(md.contains("### 👤 User\n\nPlease write a config parser."));
    assert!(md.contains("### 🤖 Assistant\n\nGenerating configuration..."));
    assert!(md.contains("🛠️ **Tool Call:** `write` (`call_write_cfg`)"));
    assert!(md.contains("{\"path\":\"config.toml\",\"content\":\"port = 8080\"}"));
    assert!(
        md.contains("### 🔧 Tool Output (`call_write_cfg`)\n\n```\nFile written: 12 bytes\n```")
    );
    assert!(md.contains("Config parser implementation complete."));
}

#[test]
fn test_session_truncation_and_clearing() {
    let temp_dir = TempDir::new("session_trunc");
    let file = temp_dir.path().join("trunc_test.json");

    let mut session = Session::new("test-model");
    for i in 0..10 {
        session.add_user_message(format!("Message {}", i));
    }
    assert_eq!(session.total_messages(), 10);

    // Truncate to 4 messages
    session.truncate(4);
    assert_eq!(session.total_messages(), 4);
    assert_eq!(session.messages().last().unwrap().content, "Message 3");

    // Clear all messages
    session.clear();
    assert_eq!(session.total_messages(), 0);

    // Save and delete
    session.save_to_path(&file).unwrap();
    assert!(file.exists());
    Session::delete_by_path(&file).unwrap();
    assert!(!file.exists());
}

// ===========================================================================
// Part 7: End-to-End Multi-Agent & Advisory Collaboration Workflow Tests
// ===========================================================================

#[tokio::test]
async fn test_e2e_multi_agent_workflow() {
    let temp_dir = TempDir::new("e2e_workflow");
    let source_file = temp_dir.path().join("generated_lib.rs");
    let source_str = source_file.to_string_lossy().to_string();

    let server = MockLlmServer::start().await;
    let client = server.client();
    let config = server.config();
    let tools = create_test_tools();

    // 1. Primary session initialization
    let mut primary_session = Session::with_system_prompt(
        "claude-3-7-sonnet",
        "You are Fusion Master Agent orchestrating parallel subagents.",
    );
    primary_session.set_title("E2E Multi-Agent Orchestration");
    primary_session.set_working_dir(temp_dir.path());
    primary_session.add_user_message("Build and verify a high-performance logging module");

    // 2. Parallel Advisory Consultation Phase
    let advisor_engine = AdvisorEngine::new((*client).clone(), config.clone());
    let critiques = advisor_engine
        .consult(
            "Build and verify a high-performance logging module",
            "1. Scout architectural requirements\n2. Coder generates non-blocking ringbuffer logger\n3. Tester audits deadlock safety",
        )
        .await;

    assert_eq!(
        critiques.len(),
        3,
        "All 3 standard advisors must participate"
    );
    assert!(AdvisorEngine::is_all_approved(&critiques));
    let advisor_system_notes = format_critiques_for_system_prompt(&critiques);
    primary_session.add_system_message(advisor_system_notes);

    // 3. Subagent Orchestration Phase
    let manager =
        SubagentManager::new(client.clone(), config.clone(), tools.clone()).with_max_concurrent(4);

    let scout_task =
        SubagentTask::scout("Scout existing logger interfaces").with_name("LoggerScout");
    let coder_task =
        SubagentTask::coder(format!("WRITE_FILE:{}", source_str)).with_name("LoggerCoder");

    // Execute subagents concurrently
    let results = manager.run_concurrent(vec![scout_task, coder_task]).await;
    assert_eq!(results.len(), 2);

    for res in results {
        let sub_res = res.expect("Subagent execution must succeed");
        assert!(sub_res.success);
        primary_session.add_assistant_message(format!(
            "Subagent [{}] ({}) reported: {}",
            sub_res.name, sub_res.role, sub_res.output
        ));
        primary_session.record_tokens(Some(100), Some(40));
    }

    // Verify file written by Coder subagent
    assert!(
        source_file.exists(),
        "Coder subagent must have created generated_lib.rs"
    );

    // 4. Session Persistence and Verification
    let session_path = temp_dir.path().join("final_e2e_session.json");
    primary_session
        .save_to_path(&session_path)
        .expect("Must persist primary session");
    assert!(session_path.exists());

    let resumed_primary =
        Session::load_from_path(&session_path).expect("Must load primary session");
    assert_eq!(resumed_primary.id(), primary_session.id());
    assert!(resumed_primary.total_messages() >= 3);
    assert_eq!(resumed_primary.token_stats().total_turns, 2);
    assert_eq!(resumed_primary.token_stats().total_tokens, 280);

    // 5. Markdown Export Verification
    let md = resumed_primary.export_markdown();
    assert!(md.contains("# E2E Multi-Agent Orchestration"));
    assert!(md.contains("LoggerScout"));
    assert!(md.contains("LoggerCoder"));
}

#[tokio::test]
async fn test_e2e_mesh_coordinated_multi_agent_pipeline() {
    let mesh = AgentMesh::new();

    // 1. Register specialized peer agents in the Mesh
    let orchestrator = mesh
        .register("Orchestrator", AgentRole::Orchestrator, "Task Coordinator")
        .await
        .expect("register orchestrator");
    let scout = mesh
        .register("Scout", AgentRole::Scout, "Code Explorer")
        .await
        .expect("register scout");
    let coder = mesh
        .register("Coder", AgentRole::Coder, "Implementer")
        .await
        .expect("register coder");
    let tester = mesh
        .register("Tester", AgentRole::Tester, "QA Validator")
        .await
        .expect("register tester");

    // 2. Peer Discovery via Mesh API
    let peers = mesh.list_peers().await;
    assert_eq!(peers.len(), 4);
    assert!(peers
        .iter()
        .any(|p| p.id == "Scout" && p.role == AgentRole::Scout));
    assert!(peers
        .iter()
        .any(|p| p.id == "Coder" && p.role == AgentRole::Coder));

    // 3. Resource Locking: Coder claims exclusive lock on target source file
    let target_module = "src/agent/mesh.rs";
    coder
        .claim_resource(target_module, None)
        .await
        .expect("Coder claims mesh.rs");

    // Tester verifies resource is locked
    let claims = mesh.get_resource_claims().await;
    assert!(claims.contains_key(target_module));
    assert_eq!(claims[target_module].owner, "Coder");

    // 4. Shared Blackboard Facts: Scout posts findings to blackboard
    scout
        .set_fact(
            "module_architecture",
            json!({
                "type": "decentralized_mesh",
                "protocols": ["pub-sub", "direct-rpc", "blackboard", "barriers"]
            }),
        )
        .await
        .expect("scout records architectural findings");

    let fact = tester
        .get_fact("module_architecture")
        .await
        .expect("tester reads fact");
    assert_eq!(fact.author, "Scout");
    assert_eq!(fact.value["type"], "decentralized_mesh");

    // 5. Broadcast Discovery Event
    scout
        .broadcast_discovery(
            "mesh_primitives",
            "Discovered 4 coordination primitives ready for testing",
            vec![target_module.to_string()],
        )
        .await
        .expect("broadcast discovery");

    // 6. Advisor Review before releasing lock and finalizing
    let review_resp = coder
        .request_review(
            "Finalize mesh implementation",
            "pub async fn send_direct(&self) -> Result<(), MeshError> { Ok(()) }",
            Some("SecurityAdvisor"),
        )
        .await
        .expect("advisor review");
    assert!(review_resp.approved);
    assert_eq!(review_resp.highest_risk, RiskLevel::Low);

    // 7. Release Resource Lock and update status to completed
    coder
        .release_resource(target_module)
        .await
        .expect("release lock");
    coder
        .broadcast_status(AgentStatus::Completed {
            result: Some("Mesh refactoring fully verified".to_string()),
        })
        .await
        .expect("broadcast completed status");

    // Clean unregistration
    orchestrator.unregister().await.unwrap();
    scout.unregister().await.unwrap();
    coder.unregister().await.unwrap();
    tester.unregister().await.unwrap();

    assert_eq!(mesh.list_peers().await.len(), 0);
}
