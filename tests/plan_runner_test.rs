//! Integration tests for PlanRunner (Phase 2 / Phase 3 Multi-Agent Execution Engine).
//!
//! Tests:
//! 1. ASCII progress visualization rendering (`render_progress_ascii`).
//! 2. Error handling (out-of-bounds stages, unmet dependencies, empty DAGs).
//! 3. Single-stage execution via `execute_stage` with concurrent batch dispatch.
//! 4. Full autonomous execution via `run_autonomous` across multi-stage DAGs.
//! 5. Error propagation and fail-fast termination on task failure.
//! 6. Summary generation and Markdown reporting.

mod agent {
    pub use fusion::agent::*;
}

#[path = "../src/agent/plan_runner.rs"]
pub mod plan_runner;

use std::net::SocketAddr;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use fusion::agent::planner_dag::{
    DagOverallStatus, DagTask, DagTaskStatus, SubagentDag,
};
use fusion::agent::subagent::{SubagentManager, SubagentRole};
use plan_runner::{PlanRunner, PlanRunnerError, PlanSummary, StageExecutionResult};
use fusion::config::Config;
use fusion::provider::LlmClient;
use fusion::tools::file::{ReadFileTool, WriteFileTool};
use fusion::tools::grep::GrepTool;
use fusion::tools::types::ToolRegistry;
use serde_json::{json, Value};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{oneshot, RwLock};

// ===========================================================================
// Test Helper: Lightweight Mock LLM HTTP Server
// ===========================================================================

#[derive(Clone)]
enum MockResponse {
    Text(String),
    Error(u16, String),
}

type HandlerFn = Arc<dyn Fn(&Value) -> MockResponse + Send + Sync>;
#[allow(dead_code)]
struct MockLlmServer {
    addr: SocketAddr,
    shutdown_tx: Option<oneshot::Sender<()>>,
    handler: Arc<RwLock<HandlerFn>>,
    request_count: Arc<AtomicUsize>,
}

impl MockLlmServer {
    async fn start() -> Self {
        Self::start_with_handler(Arc::new(|payload| Self::default_handler(payload))).await
    }

    async fn start_with_handler(initial_handler: HandlerFn) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind tcp listener");
        let addr = listener.local_addr().expect("local addr");

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
        config.openai_api_key = Some("mock-api-key".to_string());
        config
    }
    #[allow(dead_code)]
    async fn set_handler<F>(&self, handler: F)
    where
        F: Fn(&Value) -> MockResponse + Send + Sync + 'static,
    {
        let mut guard = self.handler.write().await;
        *guard = Arc::new(handler);
    }

    fn default_handler(payload: &Value) -> MockResponse {
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

        if all_text.contains("FAIL_TASK") {
            return MockResponse::Error(500, "Simulated subagent failure".to_string());
        }

        MockResponse::Text("Subagent task accomplished successfully.".to_string())
    }

    async fn handle_connection(mut socket: TcpStream, handler: HandlerFn) {
        let mut buffer = Vec::new();
        let mut temp = [0u8; 4096];

        let mut header_end = None;
        loop {
            match socket.read(&mut temp).await {
                Ok(0) => break,
                Ok(n) => {
                    buffer.extend_from_slice(&temp[..n]);
                    if let Some(pos) = buffer.windows(4).position(|w| w == b"\r\n\r\n") {
                        header_end = Some(pos + 4);
                        break;
                    }
                }
                Err(_) => return,
            }
        }

        let header_pos = match header_end {
            Some(p) => p,
            None => return,
        };

        let headers_str = String::from_utf8_lossy(&buffer[..header_pos]);
        let mut content_length = 0;
        for line in headers_str.lines() {
            if line.to_lowercase().starts_with("content-length:") {
                if let Some(val) = line.split(':').nth(1) {
                    content_length = val.trim().parse::<usize>().unwrap_or(0);
                }
            }
        }

        while buffer.len() < header_pos + content_length {
            match socket.read(&mut temp).await {
                Ok(0) => break,
                Ok(n) => buffer.extend_from_slice(&temp[..n]),
                Err(_) => return,
            }
        }

        let body_bytes = &buffer[header_pos..header_pos + content_length];
        let payload: Value = serde_json::from_slice(body_bytes).unwrap_or(Value::Null);

        let response = handler(&payload);

        match response {
            MockResponse::Text(text) => {
                let id = format!("chatcmpl-{}", uuid::Uuid::new_v4());
                let sse_chunk = json!({
                    "id": id,
                    "object": "chat.completion.chunk",
                    "created": 1700000000,
                    "model": "mock-model",
                    "choices": [{
                        "index": 0,
                        "delta": {
                            "role": "assistant",
                            "content": text
                        },
                        "finish_reason": "stop"
                    }]
                });

                let sse_data = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nCache-Control: no-cache\r\nConnection: close\r\n\r\ndata: {}\n\ndata: [DONE]\n\n",
                    sse_chunk
                );
                let _ = socket.write_all(sse_data.as_bytes()).await;
            }
            MockResponse::Error(code, msg) => {
                let err_body = json!({ "error": { "message": msg, "code": code } }).to_string();
                let resp = format!(
                    "HTTP/1.1 {} Internal Server Error\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    code,
                    err_body.len(),
                    err_body
                );
                let _ = socket.write_all(resp.as_bytes()).await;
            }
        }
        let _ = socket.flush().await;
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
    let mut registry = ToolRegistry::new();
    registry.register(Arc::new(ReadFileTool::new()));
    registry.register(Arc::new(WriteFileTool::new()));
    registry.register(Arc::new(GrepTool::new()));
    registry
}

// ===========================================================================
// Test 1: ASCII Progress Visualization
// ===========================================================================

#[test]
fn test_render_progress_ascii_empty_plan() {
    let dag = SubagentDag::new("Empty Plan", "Do nothing");
    let client = Arc::new(LlmClient::new());
    let manager = SubagentManager::new(client, Config::default(), ToolRegistry::new());
    let runner = PlanRunner::new(dag, manager);

    let progress = runner.render_progress_ascii();
    assert_eq!(progress, "[ ] (empty plan)");
}

#[test]
fn test_render_progress_ascii_linear_pipeline() {
    let mut dag = SubagentDag::new("Pipeline Plan", "Deliver feature");

    let t1 = DagTask::new("scout", "Scout Codebase", SubagentRole::Scout, "Explore repo");
    let t2 = DagTask::new("coder", "Implement Logic", SubagentRole::Coder, "Write code")
        .with_dependency("scout");
    let t3 = DagTask::new("tester", "Run Tests", SubagentRole::Tester, "Execute tests")
        .with_dependency("coder");

    dag.add_task(t1).unwrap();
    dag.add_task(t2).unwrap();
    dag.add_task(t3).unwrap();
    dag.compute_stages().unwrap();

    let client = Arc::new(LlmClient::new());
    let manager = SubagentManager::new(client, Config::default(), ToolRegistry::new());
    let mut runner = PlanRunner::new(dag, manager);

    // Initially: all pending
    assert_eq!(
        runner.render_progress_ascii(),
        "[ ] scout -> [ ] coder -> [ ] tester"
    );

    // Mark scout completed
    runner
        .dag_mut()
        .mark_task_completed("scout", "Found architecture".into(), 150);
    assert_eq!(
        runner.render_progress_ascii(),
        "[✓] scout -> [ ] coder -> [ ] tester"
    );

    // Mark coder running
    if let Some(t) = runner.dag_mut().get_task_mut("coder") {
        t.status = DagTaskStatus::Running {
            agent_id: "agent-1".into(),
            started_at: "now".into(),
        };
    }
    // Verifies requested pattern: [✓] scout -> [•] coder -> [ ] tester
    assert_eq!(
        runner.render_progress_ascii(),
        "[✓] scout -> [•] coder -> [ ] tester"
    );

    // Mark coder completed, tester running
    runner
        .dag_mut()
        .mark_task_completed("coder", "Code written".into(), 300);
    if let Some(t) = runner.dag_mut().get_task_mut("tester") {
        t.status = DagTaskStatus::Running {
            agent_id: "agent-2".into(),
            started_at: "now".into(),
        };
    }
    assert_eq!(
        runner.render_progress_ascii(),
        "[✓] scout -> [✓] coder -> [•] tester"
    );

    // Mark tester completed
    runner
        .dag_mut()
        .mark_task_completed("tester", "All passed".into(), 100);
    assert_eq!(
        runner.render_progress_ascii(),
        "[✓] scout -> [✓] coder -> [✓] tester"
    );
}

#[test]
fn test_render_progress_ascii_parallel_stages() {
    let mut dag = SubagentDag::new("Parallel Plan", "Multi-agent speedup");

    let s1 = DagTask::new("scout_ui", "Scout UI", SubagentRole::Scout, "UI audit");
    let s2 = DagTask::new("scout_db", "Scout DB", SubagentRole::Scout, "DB audit");
    let c1 = DagTask::new("coder", "Implement", SubagentRole::Coder, "Code")
        .with_dependencies(["scout_ui", "scout_db"]);

    dag.add_task(s1).unwrap();
    dag.add_task(s2).unwrap();
    dag.add_task(c1).unwrap();
    dag.compute_stages().unwrap();

    let client = Arc::new(LlmClient::new());
    let manager = SubagentManager::new(client, Config::default(), ToolRegistry::new());
    let mut runner = PlanRunner::new(dag, manager);

    runner
        .dag_mut()
        .mark_task_completed("scout_ui", "UI done".into(), 100);
    runner
        .dag_mut()
        .mark_task_completed("scout_db", "DB done".into(), 120);

    if let Some(t) = runner.dag_mut().get_task_mut("coder") {
        t.status = DagTaskStatus::Running {
            agent_id: "coder-1".into(),
            started_at: "now".into(),
        };
    }

    let progress = runner.render_progress_ascii();
    assert!(
        progress.contains("[✓] scout_ui") && progress.contains("[✓] scout_db"),
        "Expected both scouts completed in parallel group: {}",
        progress
    );
    assert!(
        progress.contains("[•] coder"),
        "Expected coder running: {}",
        progress
    );
    assert!(progress.contains(" -> "));
}

// ===========================================================================
// Test 2: Error Handling & Invariant Enforcement
// ===========================================================================

#[tokio::test]
async fn test_error_out_of_bounds_stage() {
    let mut dag = SubagentDag::new("Small Plan", "Goal");
    dag.add_task(DagTask::new("task1", "Task 1", SubagentRole::General, "Work")).unwrap();
    dag.compute_stages().unwrap();

    let client = Arc::new(LlmClient::new());
    let manager = SubagentManager::new(client, Config::default(), ToolRegistry::new());
    let mut runner = PlanRunner::new(dag, manager);

    let err = runner.execute_stage(99).await.unwrap_err();
    assert!(matches!(err, PlanRunnerError::StageNotFound { stage_idx: 99, total_stages: 1 }));
}

#[tokio::test]
async fn test_error_unmet_dependency() {
    let mut dag = SubagentDag::new("Dep Plan", "Goal");
    let t1 = DagTask::new("step1", "Step 1", SubagentRole::General, "First");
    let t2 = DagTask::new("step2", "Step 2", SubagentRole::General, "Second").with_dependency("step1");
    dag.add_task(t1).unwrap();
    dag.add_task(t2).unwrap();
    dag.compute_stages().unwrap();

    let client = Arc::new(LlmClient::new());
    let manager = SubagentManager::new(client, Config::default(), ToolRegistry::new());
    let mut runner = PlanRunner::new(dag, manager);

    // Attempt to run stage 1 before stage 0 has completed
    let err = runner.execute_stage(1).await.unwrap_err();
    match err {
        PlanRunnerError::DependencyNotMet { task_id, dep_id, .. } => {
            assert_eq!(task_id, "step2");
            assert_eq!(dep_id, "step1");
        }
        other => panic!("Unexpected error: {:?}", other),
    }
}

// ===========================================================================
// Test 3: Stage-by-Stage Execution with Mock Subagent Dispatch
// ===========================================================================

#[tokio::test]
async fn test_stage_by_stage_execution() {
    let server = MockLlmServer::start().await;
    let client = server.client();
    let config = server.config();
    let tools = create_test_tools();
    let manager = SubagentManager::new(client, config, tools).with_max_concurrent(4);

    let mut dag = SubagentDag::new("Stage Test Plan", "Test execution stages");
    let t1 = DagTask::new("scout", "Explore Code", SubagentRole::Scout, "Map directories");
    let t2 = DagTask::new("coder", "Add Feature", SubagentRole::Coder, "Implement feature")
        .with_dependency("scout");
    let t3 = DagTask::new("tester", "Verify", SubagentRole::Tester, "Run checks")
        .with_dependency("coder");

    dag.add_task(t1).unwrap();
    dag.add_task(t2).unwrap();
    dag.add_task(t3).unwrap();

    let mut runner = PlanRunner::new(dag, manager);
    runner.ensure_stages().unwrap();
    assert_eq!(runner.stage_count(), 3);

    // Execute Stage 0: Scout
    let stage0_res = runner.execute_stage(0).await.expect("stage 0 success");
    assert!(stage0_res.is_success());
    assert_eq!(stage0_res.completed_count(), 1);
    assert_eq!(stage0_res.failed_count(), 0);
    assert_eq!(stage0_res.task_ids, vec!["scout"]);
    assert!(runner.dag().get_task("scout").unwrap().status.is_completed());

    let ascii = runner.render_progress_ascii();
    assert!(ascii.contains("[✓] scout"));
    assert!(ascii.contains("[ ] coder"));

    // Execute Stage 1: Coder
    let stage1_res = runner.execute_stage(1).await.expect("stage 1 success");
    assert!(stage1_res.is_success());
    assert_eq!(stage1_res.completed_count(), 1);
    assert_eq!(stage1_res.task_ids, vec!["coder"]);
    assert!(runner.dag().get_task("coder").unwrap().status.is_completed());

    // Execute Stage 2: Tester
    let stage2_res = runner.execute_stage(2).await.expect("stage 2 success");
    assert!(stage2_res.is_success());
    assert_eq!(stage2_res.completed_count(), 1);
    assert_eq!(stage2_res.task_ids, vec!["tester"]);
    assert!(runner.dag().get_task("tester").unwrap().status.is_completed());

    assert_eq!(
        runner.render_progress_ascii(),
        "[✓] scout -> [✓] coder -> [✓] tester"
    );
    assert_eq!(runner.overall_status(), DagOverallStatus::Completed);
}

// ===========================================================================
// Test 4: End-to-End Autonomous Execution with Parallel Stages
// ===========================================================================

#[tokio::test]
async fn test_run_autonomous_parallel_workflow() {
    let server = MockLlmServer::start().await;
    let client = server.client();
    let config = server.config();
    let tools = create_test_tools();
    let manager = SubagentManager::new(client, config, tools).with_max_concurrent(8);

    let mut dag = SubagentDag::new("Autonomous Parallel Workflow", "Complete full workflow");

    // Stage 0: 2 parallel scouts
    let s1 = DagTask::new("scout_auth", "Inspect Auth", SubagentRole::Scout, "Check token code");
    let s2 = DagTask::new("scout_db", "Inspect DB", SubagentRole::Scout, "Check schema code");

    // Stage 1: 2 parallel coders
    let c1 = DagTask::new("coder_auth", "Implement Auth Fix", SubagentRole::Coder, "Fix auth")
        .with_dependency("scout_auth");
    let c2 = DagTask::new("coder_db", "Implement DB Migration", SubagentRole::Coder, "Migrate DB")
        .with_dependency("scout_db");

    // Stage 2: 1 tester verifying both
    let t1 = DagTask::new("tester_e2e", "Run E2E Suite", SubagentRole::Tester, "E2E verification")
        .with_dependencies(["coder_auth", "coder_db"]);

    dag.add_task(s1).unwrap();
    dag.add_task(s2).unwrap();
    dag.add_task(c1).unwrap();
    dag.add_task(c2).unwrap();
    dag.add_task(t1).unwrap();

    let mut runner = PlanRunner::new(dag, manager);

    let summary: PlanSummary = runner.run_autonomous().await.expect("autonomous execution");

    assert!(summary.is_success());
    assert_eq!(summary.total_tasks, 5);
    assert_eq!(summary.completed_tasks, 5);
    assert_eq!(summary.failed_tasks, 0);
    assert_eq!(summary.total_stages, 3);
    assert_eq!(summary.completed_stages, 3);
    assert_eq!(summary.task_results.len(), 5);

    // Check markdown formatting
    let markdown = summary.format_markdown();
    assert!(markdown.contains("# Plan Execution Report: Autonomous Parallel Workflow"));
    assert!(markdown.contains("- **Tasks:** 5 completed, 0 failed"));
    assert!(markdown.contains("### Task `tester_e2e`"));

    // Check clean formatted report
    let report = summary.format_report();
    assert!(report.contains("Plan Execution Report: Autonomous Parallel Workflow"));
    assert!(report.contains("[✓] Completed in 3 stages"));
    assert!(report.contains("Stages Executed:     3/3 completed"));
    assert!(report.contains("Subagent Tasks Run:  5 completed, 0 failed, 0 skipped (5 total)"));
    assert!(report.contains("Tokens Spent:"));
    assert!(summary.tokens_spent > 0);

    // Check final ASCII progress
    let ascii = runner.render_progress_ascii();
    assert!(ascii.contains("[✓] scout_auth"));
    assert!(ascii.contains("[✓] scout_db"));
    assert!(ascii.contains("[✓] coder_auth"));
    assert!(ascii.contains("[✓] coder_db"));
    assert!(ascii.contains("[✓] tester_e2e"));
}

// ===========================================================================
// Test 5: Failure Propagation & Fail-Fast Stop
// ===========================================================================

#[tokio::test]
async fn test_run_autonomous_fail_fast_on_error() {
    let server = MockLlmServer::start().await;
    let client = server.client();
    let config = server.config();
    let tools = create_test_tools();
    let manager = SubagentManager::new(client, config, tools).with_max_concurrent(4);

    let mut dag = SubagentDag::new("Failing Plan", "Demonstrate fail-fast");

    // Task 1 will fail because it contains FAIL_TASK
    let t1 = DagTask::new("scout", "Failing Scout", SubagentRole::Scout, "Analyze: FAIL_TASK");
    let t2 = DagTask::new("coder", "Never Executed", SubagentRole::Coder, "Implement code")
        .with_dependency("scout");

    dag.add_task(t1).unwrap();
    dag.add_task(t2).unwrap();

    let mut runner = PlanRunner::new(dag, manager);

    let err = runner.run_autonomous().await.unwrap_err();
    match err {
        PlanRunnerError::TaskExecutionFailed { task_id, error } => {
            assert_eq!(task_id, "scout");
            assert!(error.contains("500") || error.contains("failure") || error.contains("failed"));
        }
        other => panic!("Expected TaskExecutionFailed, got: {:?}", other),
    }

    // Task 2 should still be Pending (fail-fast stopped execution)
    let coder_task = runner.dag().get_task("coder").unwrap();
    assert!(coder_task.status.is_pending());

    let ascii = runner.render_progress_ascii();
    assert!(ascii.contains("[✗] scout"));
    assert!(ascii.contains("[ ] coder"));
}

// ===========================================================================
// Test 6: execute_active_plan Autonomous Execution Flow
// ===========================================================================

#[tokio::test]
async fn test_execute_active_plan_flow() {
    let server = MockLlmServer::start().await;
    let client = server.client();
    let config = server.config();
    let tools = create_test_tools();
    let manager = SubagentManager::new(client, config, tools).with_max_concurrent(4);

    let mut dag = SubagentDag::new("Active Autonomous Run", "Test execute_active_plan integration");
    let t1 = DagTask::new("research", "Research Auth", SubagentRole::Scout, "Investigate auth patterns");
    let t2 = DagTask::new("implement", "Implement Auth", SubagentRole::Coder, "Write auth handler")
        .with_dependency("research");
    dag.add_task(t1).unwrap();
    dag.add_task(t2).unwrap();

    let summary = fusion::ui::slash_plan::execute_active_plan(&mut dag, &manager)
        .await
        .expect("execute_active_plan autonomous run");

    assert!(summary.is_success());
    assert_eq!(summary.total_tasks, 2);
    assert_eq!(summary.completed_tasks, 2);
    assert_eq!(summary.failed_tasks, 0);
    assert_eq!(summary.total_stages, 2);
    assert_eq!(summary.completed_stages, 2);
    assert!(summary.tokens_spent > 0);

    let report = summary.format_report();
    assert!(report.contains("Plan Execution Report: Active Autonomous Run"));
    assert!(report.contains("[✓] Completed in 2 stages"));
    assert!(report.contains("Stages Executed:     2/2 completed"));
    assert!(report.contains("Subagent Tasks Run:  2 completed"));
    assert!(report.contains("Tokens Spent:"));

    // Verify the in-place mutated DAG
    assert_eq!(dag.overall_status(), DagOverallStatus::Completed);
    assert!(dag.get_task("research").unwrap().status.is_completed());
    assert!(dag.get_task("implement").unwrap().status.is_completed());
}
