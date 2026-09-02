//! End-to-End CLI Integration Tests for Fusion
//!
//! Verifies end-to-end functionality of the Fusion CLI binary and REPL commands:
//! 1. `--help` / `-h` flag and command line usage output
//! 2. `--version` / `-V` flag and version string output
//! 3. `/tools` slash command execution and registered tool listing
//! 4. REPL slash commands via CLI (`/help`, `/palette`, `/status`, `/config`, unknown commands)
//! 5. Shell completion generation (`--generate-completion` for bash, zsh, fish, powershell, elvish)
//! 6. Single prompt non-interactive execution with mock LLM server streaming
//! 7. Single prompt with tool execution cycle (e.g. file writing/reading)
//! 8. CLI option overrides (`--model`, `--provider`, `--no-advisors`, `-C`/`--cwd`, presets)
//! 9. Error handling for invalid flags and unrecognized options
//! 10. Model shorthands (`/model sonnet`, `/model r1`, `/model 4o`, `/model haiku`, etc.)
//! 11. Provider switches (`/provider deepseek`, `/provider anthropic`, `/provider ollama`, etc.)
//! 12. Advisor toggle commands (`/advisors on`, `/advisors off`, `/advisors status`, etc.)
//! 13. Multi-turn prompt history navigation, duplicate filtering, and session tracking
//! 14. Slash command tokenization, full grammar parsing, and execution dispatch

use std::collections::HashMap;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use clap::Parser;
use serde_json::{json, Value};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{oneshot, RwLock};

use fusion::agent::loop_runner::AgentRunner;
use fusion::agent::session::{Session, SessionSummary, TokenStats, TokenUsage};
use fusion::cli::Cli;
use fusion::config::{Config, ConfigPreset, MODEL_SHORTHANDS, SUPPORTED_PROVIDERS};
use fusion::provider::types::{Message, Role, ToolCall};
use fusion::provider::LlmClient;
use fusion::tools::{default_registry, ToolContext};
use fusion::ui::keys::KeybindingProfile;
use fusion::ui::prompt::{Prompt, PromptResult};
use fusion::ui::repl::handle_command;
use fusion::ui::slash::{
    execute_slash_command, get_command_palette, handle_slash_command, tokenize_command,
    CommandCategory, CommandDescriptor, CommandResult, ConfigCommand, ExportFormat,
    PromptCommand, SessionCommand, SkillsCommand, SlashCommand, COMMAND_PALETTE,
};

// ===========================================================================
// Test Helper: RAII Temp Directory
// ===========================================================================

struct TempDir {
    path: PathBuf,
}

impl TempDir {
    fn new(prefix: &str) -> Self {
        let unique = format!(
            "fusion_cli_test_{}_{}_{}",
            prefix,
            std::process::id(),
            uuid::Uuid::new_v4().simple()
        );
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
// Test Helper: Binary Command Resolver
// ===========================================================================

/// Creates a `std::process::Command` configured to execute the `fusion` binary.
fn fusion_cmd() -> Command {
    let bin_path = env!("CARGO_BIN_EXE_fusion");
    let mut cmd = Command::new(bin_path);
    // Isolate from user home config to ensure clean test environment
    cmd.env("HOME", std::env::temp_dir());
    cmd
}

// ===========================================================================
// Test Helper: In-Memory Runner & Session Factory
// ===========================================================================

fn create_test_runner_and_session() -> (AgentRunner, Session) {
    let config = Config::default();
    let tools = default_registry();
    let tool_ctx = ToolContext {
        cwd: std::env::temp_dir(),
        env: HashMap::new(),
    };
    let client = LlmClient::new();
    let model = config.default_model.clone();
    let runner = AgentRunner::new(client, config, tools, tool_ctx);
    let session = Session::new(&model);
    (runner, session)
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
            .expect("failed to bind tcp listener for mock llm");
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

        // Advisor query check
        if system_text.contains("Advisor") || all_text.contains("evaluate this user request") {
            return MockResponse::Json(json!({
                "approved": true,
                "risk_level": "low",
                "critique": "Action is verified safe.",
                "suggestions": []
            }));
        }

        // Single prompt query responses
        if all_text.contains("Explain quantum computing") {
            MockResponse::Text("Quantum computing leverages qubits and superposition for ultra-fast parallel calculations.".to_string())
        } else if all_text.contains("What is 2 + 2?") {
            MockResponse::Text("2 + 2 is equal to 4.".to_string())
        } else if all_text.contains("Echo this text:") {
            MockResponse::Text("Echo: Hello from Fusion CLI!".to_string())
        } else {
            MockResponse::Text("Default response from mock LLM server.".to_string())
        }
    }

    async fn handle_connection(mut socket: TcpStream, handler: HandlerFn) {
        let mut buf = Vec::with_capacity(4096);
        let mut temp = [0u8; 1024];

        let header_end;
        loop {
            let n = match socket.read(&mut temp).await {
                Ok(0) => return,
                Ok(n) => n,
                Err(_) => return,
            };
            buf.extend_from_slice(&temp[..n]);
            if let Some(pos) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
                header_end = pos + 4;
                break;
            }
            if buf.len() > 131_072 {
                return;
            }
        }

        let headers_str = String::from_utf8_lossy(&buf[..header_end]);
        let mut content_length = 0;
        for line in headers_str.lines() {
            let lower = line.to_lowercase();
            if lower.starts_with("content-length:") {
                if let Some(val) = line.split(':').nth(1) {
                    content_length = val.trim().parse::<usize>().unwrap_or(0);
                }
            }
        }

        while buf.len() < header_end + content_length {
            let n = match socket.read(&mut temp).await {
                Ok(0) => break,
                Ok(n) => n,
                Err(_) => return,
            };
            buf.extend_from_slice(&temp[..n]);
        }

        let body_bytes = &buf[header_end..header_end + content_length];
        let body_json: Value =
            serde_json::from_slice(body_bytes).unwrap_or(Value::Null);

        let response = handler(&body_json);

        match response {
            MockResponse::Text(content) => {
                let delta = json!({
                    "id": "chatcmpl-cli-mock",
                    "choices": [{
                        "index": 0,
                        "delta": { "content": content },
                        "finish_reason": null
                    }]
                });
                let stop = json!({
                    "id": "chatcmpl-cli-mock",
                    "choices": [{
                        "index": 0,
                        "delta": {},
                        "finish_reason": "stop"
                    }],
                    "usage": { "prompt_tokens": 20, "completion_tokens": 15 }
                });

                let sse_body = format!(
                    "data: {}\n\ndata: {}\n\ndata: [DONE]\n\n",
                    delta, stop
                );
                let http_resp = format!(
                    "HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\ncache-control: no-cache\r\nconnection: close\r\n\r\n{}",
                    sse_body
                );
                let _ = socket.write_all(http_resp.as_bytes()).await;
                let _ = socket.flush().await;
            }
            MockResponse::Json(val) => {
                let text = val.to_string();
                let delta = json!({
                    "id": "chatcmpl-cli-json",
                    "choices": [{
                        "index": 0,
                        "delta": { "content": text },
                        "finish_reason": null
                    }]
                });
                let stop = json!({
                    "id": "chatcmpl-cli-json",
                    "choices": [{
                        "index": 0,
                        "delta": {},
                        "finish_reason": "stop"
                    }],
                    "usage": { "prompt_tokens": 30, "completion_tokens": 25 }
                });

                let sse_body = format!(
                    "data: {}\n\ndata: {}\n\ndata: [DONE]\n\n",
                    delta, stop
                );
                let http_resp = format!(
                    "HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\ncache-control: no-cache\r\nconnection: close\r\n\r\n{}",
                    sse_body
                );
                let _ = socket.write_all(http_resp.as_bytes()).await;
                let _ = socket.flush().await;
            }
            MockResponse::ToolCall { id, name, arguments } => {
                let call_delta = json!({
                    "id": "chatcmpl-cli-tc",
                    "choices": [{
                        "index": 0,
                        "delta": {
                            "tool_calls": [{
                                "index": 0,
                                "id": id,
                                "function": {
                                    "name": name,
                                    "arguments": arguments
                                }
                            }]
                        },
                        "finish_reason": null
                    }]
                });
                let stop = json!({
                    "id": "chatcmpl-cli-tc",
                    "choices": [{
                        "index": 0,
                        "delta": {},
                        "finish_reason": "tool_calls"
                    }]
                });

                let sse_body = format!(
                    "data: {}\n\ndata: {}\n\ndata: [DONE]\n\n",
                    call_delta, stop
                );
                let http_resp = format!(
                    "HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\ncache-control: no-cache\r\nconnection: close\r\n\r\n{}",
                    sse_body
                );
                let _ = socket.write_all(http_resp.as_bytes()).await;
                let _ = socket.flush().await;
            }
            MockResponse::Error(status, msg) => {
                let err_body = json!({
                    "error": {
                        "message": msg,
                        "type": "invalid_request_error",
                        "code": status
                    }
                })
                .to_string();
                let http_resp = format!(
                    "HTTP/1.1 {} {}\r\ncontent-type: application/json\r\nconnection: close\r\n\r\n{}",
                    status, msg, err_body
                );
                let _ = socket.write_all(http_resp.as_bytes()).await;
                let _ = socket.flush().await;
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

// ===========================================================================
// Part 1: CLI Flags Verification (--help, -h, --version, -V)
// ===========================================================================

#[test]
fn test_cli_help_flag_long() {
    let output = fusion_cmd()
        .arg("--help")
        .output()
        .expect("failed to execute fusion --help");

    assert!(
        output.status.success(),
        "fusion --help failed with status: {:?}",
        output.status
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("Fast, lightweight") || stdout.contains("fusion"),
        "Help output should contain description: {}",
        stdout
    );
    assert!(
        stdout.contains("--model") || stdout.contains("-m"),
        "Help output should contain --model option: {}",
        stdout
    );
    assert!(
        stdout.contains("--provider") || stdout.contains("-p"),
        "Help output should contain --provider option: {}",
        stdout
    );
    assert!(
        stdout.contains("--no-advisors"),
        "Help output should contain --no-advisors option: {}",
        stdout
    );
    assert!(
        stdout.contains("--cwd") || stdout.contains("-C"),
        "Help output should contain --cwd option: {}",
        stdout
    );
    assert!(
        stdout.contains("--acp"),
        "Help output should contain --acp option: {}",
        stdout
    );
    assert!(
        stdout.contains("--generate-completion"),
        "Help output should contain --generate-completion option: {}",
        stdout
    );
}

#[test]
fn test_cli_help_flag_short() {
    let output = fusion_cmd()
        .arg("-h")
        .output()
        .expect("failed to execute fusion -h");

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("Usage:") || stdout.contains("fusion"),
        "Short help should contain usage info: {}",
        stdout
    );
}

#[test]
fn test_cli_version_flag_long() {
    let output = fusion_cmd()
        .arg("--version")
        .output()
        .expect("failed to execute fusion --version");

    assert!(
        output.status.success(),
        "fusion --version failed with status: {:?}",
        output.status
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    let expected_version = env!("CARGO_PKG_VERSION");
    assert!(
        stdout.contains("fusion") && stdout.contains(expected_version),
        "Version output should contain 'fusion {}', got: {}",
        expected_version,
        stdout
    );
}

#[test]
fn test_cli_version_flag_short() {
    let output = fusion_cmd()
        .arg("-V")
        .output()
        .expect("failed to execute fusion -V");

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    let expected_version = env!("CARGO_PKG_VERSION");
    assert!(
        stdout.contains("fusion") && stdout.contains(expected_version),
        "Short version output should contain 'fusion {}', got: {}",
        expected_version,
        stdout
    );
}

// ===========================================================================
// Part 2: Slash Commands Output via CLI (/tools, /help, /palette, /status, /config)
// ===========================================================================

#[test]
fn test_cli_slash_tools_output() {
    let output = fusion_cmd()
        .arg("/tools")
        .output()
        .expect("failed to execute fusion /tools");

    assert!(
        output.status.success(),
        "fusion /tools failed with status: {:?}",
        output.status
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("Registered Tools:"),
        "Output must contain 'Registered Tools:' header. Got:\n{}",
        stdout
    );

    let expected_tools = [
        "bash",
        "read",
        "write",
        "edit",
        "grep",
        "glob",
        "git_status",
        "git_diff",
        "patch",
        "watch",
        "web_search",
    ];

    for tool in &expected_tools {
        assert!(
            stdout.contains(tool),
            "Expected tool '{}' to be listed in /tools output. Got:\n{}",
            tool,
            stdout
        );
    }
}

#[test]
fn test_cli_slash_help_output() {
    let output = fusion_cmd()
        .arg("/help")
        .output()
        .expect("failed to execute fusion /help");

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("Fusion REPL Commands") || stdout.contains("/tools") || stdout.contains("/session"),
        "Expected /help to print command list. Got:\n{}",
        stdout
    );
}

#[test]
fn test_cli_slash_palette_output() {
    let output = fusion_cmd()
        .arg("/palette")
        .output()
        .expect("failed to execute fusion /palette");

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("Command Palette") || stdout.contains("Session Management") || stdout.contains("Tools & Environment"),
        "Expected /palette to print command palette. Got:\n{}",
        stdout
    );
}

#[test]
fn test_cli_slash_status_output() {
    let output = fusion_cmd()
        .arg("/status")
        .output()
        .expect("failed to execute fusion /status");

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("Fusion Status") || stdout.contains("Model") || stdout.contains("Provider"),
        "Expected /status to print current status. Got:\n{}",
        stdout
    );
}

#[test]
fn test_cli_slash_config_output() {
    let output = fusion_cmd()
        .arg("/config")
        .output()
        .expect("failed to execute fusion /config");

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("Configuration") || stdout.contains("Default Model") || stdout.contains("advisors_enabled"),
        "Expected /config to print active configuration. Got:\n{}",
        stdout
    );
}

#[test]
fn test_cli_slash_unknown_command() {
    let output = fusion_cmd()
        .arg("/nonexistentcommand")
        .output()
        .expect("failed to execute fusion /nonexistentcommand");

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("Unknown command:") && stdout.contains("/nonexistentcommand"),
        "Expected unknown command notice. Got:\n{}",
        stdout
    );
}

// ===========================================================================
// Part 3: Shell Completion Generation
// ===========================================================================

#[test]
fn test_cli_generate_completion_bash() {
    let output = fusion_cmd()
        .args(["--generate-completion", "bash"])
        .output()
        .expect("failed to execute fusion --generate-completion bash");

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("_fusion") || stdout.contains("complete -F"),
        "Bash completion script should contain completion functions: {}",
        stdout
    );
}

#[test]
fn test_cli_generate_completion_zsh() {
    let output = fusion_cmd()
        .args(["--generate-completion", "zsh"])
        .output()
        .expect("failed to execute fusion --generate-completion zsh");

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("#compdef fusion") || stdout.contains("_fusion"),
        "Zsh completion script should contain compdef: {}",
        stdout
    );
}

#[test]
fn test_cli_generate_completion_fish() {
    let output = fusion_cmd()
        .args(["--generate-completion", "fish"])
        .output()
        .expect("failed to execute fusion --generate-completion fish");

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("complete -c fusion"),
        "Fish completion script should contain complete command: {}",
        stdout
    );
}

#[test]
fn test_cli_generate_completion_powershell() {
    let output = fusion_cmd()
        .args(["--generate-completion", "powershell"])
        .output()
        .expect("failed to execute fusion --generate-completion powershell");

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("Register-ArgumentCompleter") || stdout.contains("fusion"),
        "PowerShell completion script should contain argument completer: {}",
        stdout
    );
}

// ===========================================================================
// Part 4: Single Prompt Non-Interactive Execution (E2E with Mock LLM Server)
// ===========================================================================

#[tokio::test]
async fn test_cli_single_prompt_execution_e2e() {
    let server = MockLlmServer::start().await;

    let output = fusion_cmd()
        .env("OPENAI_BASE_URL", server.base_url())
        .env("OPENAI_API_KEY", "mock-api-key-test")
        .args([
            "--model",
            "gpt-4o",
            "--provider",
            "openai",
            "--no-advisors",
            "Explain quantum computing in one sentence.",
        ])
        .output()
        .expect("failed to execute fusion single prompt");

    assert!(
        output.status.success(),
        "Single prompt failed with status: {:?}, stderr: {}",
        output.status,
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("Quantum computing leverages qubits and superposition"),
        "Stdout should contain streamed response from mock LLM server. Got:\n{}",
        stdout
    );

    assert!(
        server.request_count() >= 1,
        "Mock LLM server should have received at least 1 request"
    );
}

#[tokio::test]
async fn test_cli_single_prompt_with_advisors() {
    let server = MockLlmServer::start().await;

    let output = fusion_cmd()
        .env("OPENAI_BASE_URL", server.base_url())
        .env("OPENAI_API_KEY", "mock-api-key-test")
        .args([
            "--model",
            "gpt-4o",
            "--provider",
            "openai",
            "What is 2 + 2?",
        ])
        .output()
        .expect("failed to execute fusion single prompt with advisors");

    assert!(
        output.status.success(),
        "Single prompt with advisors failed: {:?}",
        output.status
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("2 + 2 is equal to 4"),
        "Stdout should contain LLM answer. Got:\n{}",
        stdout
    );
}

#[tokio::test]
async fn test_cli_single_prompt_with_custom_working_dir() {
    let temp = TempDir::new("custom_cwd");
    let server = MockLlmServer::start().await;

    let output = fusion_cmd()
        .env("OPENAI_BASE_URL", server.base_url())
        .env("OPENAI_API_KEY", "mock-api-key-test")
        .args([
            "--model",
            "gpt-4o",
            "--provider",
            "openai",
            "--no-advisors",
            "-C",
            temp.path().to_str().unwrap(),
            "Echo this text: Hello from Fusion CLI!",
        ])
        .output()
        .expect("failed to execute fusion with -C");

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("Echo: Hello from Fusion CLI!"),
        "Stdout should contain LLM answer. Got:\n{}",
        stdout
    );
}

#[tokio::test]
async fn test_cli_single_prompt_tool_call_cycle() {
    let temp = TempDir::new("tool_cycle");
    let test_file = temp.path().join("cli_greeting.txt");

    let server = MockLlmServer::start().await;
    let file_path_str = test_file.to_str().unwrap().to_string();

    let phase = Arc::new(AtomicUsize::new(0));
    let phase_clone = Arc::clone(&phase);
    let target_file = file_path_str.clone();

    server
        .set_handler(move |_payload| {
            let p = phase_clone.fetch_add(1, Ordering::SeqCst);
            if p == 0 {
                MockResponse::ToolCall {
                    id: "call_write_1".to_string(),
                    name: "write".to_string(),
                    arguments: json!({
                        "path": target_file,
                        "content": "Created via Fusion CLI single prompt tool call!\n"
                    })
                    .to_string(),
                }
            } else {
                MockResponse::Text("Successfully wrote greeting file.".to_string())
            }
        })
        .await;

    let output = fusion_cmd()
        .env("OPENAI_BASE_URL", server.base_url())
        .env("OPENAI_API_KEY", "mock-api-key-test")
        .args([
            "--model",
            "gpt-4o",
            "--provider",
            "openai",
            "--no-advisors",
            "-C",
            temp.path().to_str().unwrap(),
            "Write greeting file",
        ])
        .output()
        .expect("failed to execute single prompt tool call");

    assert!(output.status.success());

    assert!(
        test_file.exists(),
        "Expected file {} to be created by tool execution",
        test_file.display()
    );
    let content = std::fs::read_to_string(&test_file).expect("failed to read test file");
    assert_eq!(content, "Created via Fusion CLI single prompt tool call!\n");

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("Successfully wrote greeting file"),
        "Stdout should contain completion text. Got:\n{}",
        stdout
    );
}

// ===========================================================================
// Part 5: Error Handling & Argument Parsing Edge Cases
// ===========================================================================

#[test]
fn test_cli_unrecognized_argument_fails() {
    let output = fusion_cmd()
        .arg("--unrecognized-nonexistent-flag-xyz")
        .output()
        .expect("failed to execute fusion with invalid flag");

    assert!(
        !output.status.success(),
        "Unrecognized argument should result in non-zero exit code"
    );

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("unexpected argument") || stderr.contains("error:"),
        "Stderr should indicate argument parsing error. Got:\n{}",
        stderr
    );
}

#[test]
fn test_cli_clap_parser_unit_tests() {
    let cli_help = Cli::try_parse_from(["fusion", "/help"]).unwrap();
    assert_eq!(cli_help.prompt.as_deref(), Some("/help"));

    let cli_tools = Cli::try_parse_from(["fusion", "/tools"]).unwrap();
    assert_eq!(cli_tools.prompt.as_deref(), Some("/tools"));

    let cli_opts = Cli::try_parse_from([
        "fusion",
        "--model",
        "deepseek-chat",
        "--provider",
        "deepseek",
        "--no-advisors",
        "-C",
        "/custom/dir",
        "Single prompt task",
    ])
    .unwrap();

    assert_eq!(cli_opts.model.as_deref(), Some("deepseek-chat"));
    assert_eq!(cli_opts.provider.as_deref(), Some("deepseek"));
    assert!(cli_opts.no_advisors);
    assert_eq!(cli_opts.cwd, Some(PathBuf::from("/custom/dir")));
    assert_eq!(cli_opts.prompt.as_deref(), Some("Single prompt task"));
}

#[test]
fn test_cli_preset_parsing_integration() {
    let presets = [
        ("coding-fast", "anthropic", "claude-3-5-sonnet-20241022"),
        ("deep-reasoning", "deepseek", "deepseek-reasoner"),
        ("cheap", "deepseek", "deepseek-chat"),
        ("offline-ollama", "ollama", "qwen2.5-coder"),
        ("termux-mobile", "deepseek", "deepseek-chat"),
    ];

    for (preset_name, expected_prov, expected_model) in &presets {
        let cli = Cli::try_parse_from(["fusion", "--preset", preset_name]).unwrap();
        assert_eq!(cli.preset.as_deref(), Some(*preset_name));

        let mut config = Config::default();
        let preset = ConfigPreset::from_str_loose(preset_name)
            .unwrap_or_else(|| panic!("failed to parse preset '{}'", preset_name));
        config.apply_preset(preset);
        assert_eq!(config.default_provider, *expected_prov);
        assert_eq!(config.default_model, *expected_model);
    }
}

// ===========================================================================
// Part 6: Model Shorthands Integration Tests (/model sonnet, /model r1, /model 4o, /model haiku)
// ===========================================================================

#[test]
fn test_cli_slash_model_shorthands_binary_execution() {
    // 1. /model sonnet
    let out_sonnet = fusion_cmd()
        .arg("/model sonnet")
        .output()
        .expect("failed to execute fusion /model sonnet");
    assert!(out_sonnet.status.success());
    let stdout_sonnet = String::from_utf8_lossy(&out_sonnet.stdout);
    assert!(
        stdout_sonnet.contains("Switched active model to") && stdout_sonnet.contains("sonnet"),
        "Expected output confirming switch to 'sonnet'. Got:\n{}",
        stdout_sonnet
    );

    // 2. /model r1
    let out_r1 = fusion_cmd()
        .arg("/model r1")
        .output()
        .expect("failed to execute fusion /model r1");
    assert!(out_r1.status.success());
    let stdout_r1 = String::from_utf8_lossy(&out_r1.stdout);
    assert!(
        stdout_r1.contains("Switched active model to") && stdout_r1.contains("r1"),
        "Expected output confirming switch to 'r1'. Got:\n{}",
        stdout_r1
    );

    // 3. /model 4o
    let out_4o = fusion_cmd()
        .arg("/model 4o")
        .output()
        .expect("failed to execute fusion /model 4o");
    assert!(out_4o.status.success());
    let stdout_4o = String::from_utf8_lossy(&out_4o.stdout);
    assert!(
        stdout_4o.contains("Switched active model to") && stdout_4o.contains("4o"),
        "Expected output confirming switch to '4o'. Got:\n{}",
        stdout_4o
    );

    // 4. /model haiku
    let out_haiku = fusion_cmd()
        .arg("/model haiku")
        .output()
        .expect("failed to execute fusion /model haiku");
    assert!(out_haiku.status.success());
    let stdout_haiku = String::from_utf8_lossy(&out_haiku.stdout);
    assert!(
        stdout_haiku.contains("Switched active model to") && stdout_haiku.contains("haiku"),
        "Expected output confirming switch to 'haiku'. Got:\n{}",
        stdout_haiku
    );

    // 5. /model without arguments (query info)
    let out_info = fusion_cmd()
        .arg("/model")
        .output()
        .expect("failed to execute fusion /model");
    assert!(out_info.status.success());
    let stdout_info = String::from_utf8_lossy(&out_info.stdout);
    assert!(
        stdout_info.contains("Active Model:") && stdout_info.contains("Suggested Models per Provider:"),
        "Expected model overview table. Got:\n{}",
        stdout_info
    );

    // 6. Short alias /m sonnet
    let out_m_sonnet = fusion_cmd()
        .arg("/m sonnet")
        .output()
        .expect("failed to execute fusion /m sonnet");
    assert!(out_m_sonnet.status.success());
    let stdout_m_sonnet = String::from_utf8_lossy(&out_m_sonnet.stdout);
    assert!(
        stdout_m_sonnet.contains("Switched active model to") && stdout_m_sonnet.contains("sonnet"),
        "Expected /m alias to switch model. Got:\n{}",
        stdout_m_sonnet
    );
}

#[test]
fn test_slash_command_model_parsing_and_resolution() {
    // Verify parsing of /model variants
    let cmd_sonnet = SlashCommand::parse("/model sonnet").unwrap();
    assert_eq!(cmd_sonnet, SlashCommand::Model { name: Some("sonnet".to_string()) });

    let cmd_r1 = SlashCommand::parse("/model r1").unwrap();
    assert_eq!(cmd_r1, SlashCommand::Model { name: Some("r1".to_string()) });

    let cmd_4o = SlashCommand::parse("/model 4o").unwrap();
    assert_eq!(cmd_4o, SlashCommand::Model { name: Some("4o".to_string()) });

    let cmd_haiku = SlashCommand::parse("/model haiku").unwrap();
    assert_eq!(cmd_haiku, SlashCommand::Model { name: Some("haiku".to_string()) });

    let cmd_m_alias = SlashCommand::parse("/m claude-3-5-sonnet").unwrap();
    assert_eq!(cmd_m_alias, SlashCommand::Model { name: Some("claude-3-5-sonnet".to_string()) });

    let cmd_empty = SlashCommand::parse("/model").unwrap();
    assert_eq!(cmd_empty, SlashCommand::Model { name: None });

    // Verify resolve_model_shorthand canonical mappings
    assert_eq!(
        Config::resolve_model_shorthand("sonnet"),
        Some(("anthropic", "claude-3-5-sonnet-20241022"))
    );
    assert_eq!(
        Config::resolve_model_shorthand("r1"),
        Some(("deepseek", "deepseek-reasoner"))
    );
    assert_eq!(
        Config::resolve_model_shorthand("4o"),
        Some(("openai", "gpt-4o"))
    );
    assert_eq!(
        Config::resolve_model_shorthand("haiku"),
        Some(("anthropic", "claude-3-5-haiku-20241022"))
    );
    assert_eq!(
        Config::resolve_model_shorthand("opus"),
        Some(("anthropic", "claude-3-opus-20240229"))
    );
    assert_eq!(
        Config::resolve_model_shorthand("3.7-sonnet"),
        Some(("anthropic", "claude-3-7-sonnet-20250219"))
    );
    assert_eq!(
        Config::resolve_model_shorthand("v3"),
        Some(("deepseek", "deepseek-chat"))
    );
    assert_eq!(
        Config::resolve_model_shorthand("4o-mini"),
        Some(("openai", "gpt-4o-mini"))
    );
    assert_eq!(
        Config::resolve_model_shorthand("o1"),
        Some(("openai", "o1"))
    );
    assert_eq!(
        Config::resolve_model_shorthand("o3-mini"),
        Some(("openai", "o3-mini"))
    );
    assert_eq!(
        Config::resolve_model_shorthand("grok"),
        Some(("xai", "grok-2-latest"))
    );
    assert_eq!(
        Config::resolve_model_shorthand("llama3.3"),
        Some(("ollama", "llama3.3"))
    );
    assert_eq!(
        Config::resolve_model_shorthand("qwen"),
        Some(("ollama", "qwen2.5-coder"))
    );
}

#[test]
fn test_config_model_resolution_and_set_model() {
    let mut config = Config::default();

    // Test Config::resolve_model with explicit provider prefix
    let (p1, m1) = Config::resolve_model("openai:gpt-4o", None);
    assert_eq!(p1, "openai");
    assert_eq!(m1, "gpt-4o");

    let (p2, m2) = Config::resolve_model("anthropic/claude-3-5-sonnet", None);
    assert_eq!(p2, "anthropic");
    assert_eq!(m2, "claude-3-5-sonnet-20241022");

    // Test Config::resolve_model with shorthands and whitespace
    let (p_sonnet, m_sonnet) = Config::resolve_model("  sonnet  ", None);
    assert_eq!(p_sonnet, "anthropic");
    assert_eq!(m_sonnet, "claude-3-5-sonnet-20241022");

    let (p_r1, m_r1) = Config::resolve_model("  r1 ", None);
    assert_eq!(p_r1, "deepseek");
    assert_eq!(m_r1, "deepseek-reasoner");

    let (p_4o, m_4o) = Config::resolve_model(" 4o ", None);
    assert_eq!(p_4o, "openai");
    assert_eq!(m_4o, "gpt-4o");

    let (p_haiku, m_haiku) = Config::resolve_model(" HAIKU ", None);
    assert_eq!(p_haiku, "anthropic");
    assert_eq!(m_haiku, "claude-3-5-haiku-20241022");

    // Test Config::set_model updating both provider and model
    config.set_model("sonnet");
    assert_eq!(config.default_provider, "anthropic");
    assert_eq!(config.default_model, "claude-3-5-sonnet-20241022");

    config.set_model("r1");
    assert_eq!(config.default_provider, "deepseek");
    assert_eq!(config.default_model, "deepseek-reasoner");

    config.set_model("4o");
    assert_eq!(config.default_provider, "openai");
    assert_eq!(config.default_model, "gpt-4o");

    config.set_model("haiku");
    assert_eq!(config.default_provider, "anthropic");
    assert_eq!(config.default_model, "claude-3-5-haiku-20241022");
}

#[test]
fn test_slash_command_model_execution_dispatch() {
    let (mut runner, mut session) = create_test_runner_and_session();

    // Execute /model sonnet
    let res = handle_slash_command("/model sonnet", &mut runner, &mut session);
    assert!(matches!(res, Some(CommandResult::Continue)));
    assert_eq!(session.active_model(), "sonnet");
    assert_eq!(runner.config().default_model, "sonnet");

    // Execute /model r1
    let res = handle_slash_command("/model r1", &mut runner, &mut session);
    assert!(matches!(res, Some(CommandResult::Continue)));
    assert_eq!(session.active_model(), "r1");
    assert_eq!(runner.config().default_model, "r1");

    // Execute /model 4o
    let res = handle_slash_command("/model 4o", &mut runner, &mut session);
    assert!(matches!(res, Some(CommandResult::Continue)));
    assert_eq!(session.active_model(), "4o");
    assert_eq!(runner.config().default_model, "4o");

    // Execute /model haiku
    let res = handle_slash_command("/model haiku", &mut runner, &mut session);
    assert!(matches!(res, Some(CommandResult::Continue)));
    assert_eq!(session.active_model(), "haiku");
    assert_eq!(runner.config().default_model, "haiku");
}

// ===========================================================================
// Part 7: Provider Switches Integration Tests (/provider deepseek, /provider anthropic, /provider ollama)
// ===========================================================================

#[test]
fn test_cli_slash_provider_switches_binary_execution() {
    // 1. /provider deepseek
    let out_deepseek = fusion_cmd()
        .arg("/provider deepseek")
        .output()
        .expect("failed to execute fusion /provider deepseek");
    assert!(out_deepseek.status.success());
    let stdout_deepseek = String::from_utf8_lossy(&out_deepseek.stdout);
    assert!(
        stdout_deepseek.contains("Switched active provider to") && stdout_deepseek.contains("deepseek"),
        "Expected confirmation for deepseek switch. Got:\n{}",
        stdout_deepseek
    );

    // 2. /provider anthropic
    let out_anthropic = fusion_cmd()
        .arg("/provider anthropic")
        .output()
        .expect("failed to execute fusion /provider anthropic");
    assert!(out_anthropic.status.success());
    let stdout_anthropic = String::from_utf8_lossy(&out_anthropic.stdout);
    assert!(
        stdout_anthropic.contains("Switched active provider to") && stdout_anthropic.contains("anthropic"),
        "Expected confirmation for anthropic switch. Got:\n{}",
        stdout_anthropic
    );

    // 3. /provider ollama
    let out_ollama = fusion_cmd()
        .arg("/provider ollama")
        .output()
        .expect("failed to execute fusion /provider ollama");
    assert!(out_ollama.status.success());
    let stdout_ollama = String::from_utf8_lossy(&out_ollama.stdout);
    assert!(
        stdout_ollama.contains("Switched active provider to") && stdout_ollama.contains("ollama"),
        "Expected confirmation for ollama switch. Got:\n{}",
        stdout_ollama
    );

    // 4. /provider openai
    let out_openai = fusion_cmd()
        .arg("/provider openai")
        .output()
        .expect("failed to execute fusion /provider openai");
    assert!(out_openai.status.success());
    let stdout_openai = String::from_utf8_lossy(&out_openai.stdout);
    assert!(
        stdout_openai.contains("Switched active provider to") && stdout_openai.contains("openai"),
        "Expected confirmation for openai switch. Got:\n{}",
        stdout_openai
    );

    // 5. /provider without arguments (query status)
    let out_query = fusion_cmd()
        .arg("/provider")
        .output()
        .expect("failed to execute fusion /provider");
    assert!(out_query.status.success());
    let stdout_query = String::from_utf8_lossy(&out_query.stdout);
    assert!(
        stdout_query.contains("Active Provider:") && stdout_query.contains("Configured Providers:"),
        "Expected active provider and configured list. Got:\n{}",
        stdout_query
    );

    // 6. /provider invalid_provider (error notice)
    let out_invalid = fusion_cmd()
        .arg("/provider invalid_provider_xyz")
        .output()
        .expect("failed to execute fusion /provider invalid_provider_xyz");
    assert!(out_invalid.status.success());
    let stdout_invalid = String::from_utf8_lossy(&out_invalid.stdout);
    assert!(
        stdout_invalid.contains("Unknown provider:") && stdout_invalid.contains("Supported providers:"),
        "Expected unknown provider notice. Got:\n{}",
        stdout_invalid
    );

    // 7. Short alias /p deepseek
    let out_p_deepseek = fusion_cmd()
        .arg("/p deepseek")
        .output()
        .expect("failed to execute fusion /p deepseek");
    assert!(out_p_deepseek.status.success());
    let stdout_p_deepseek = String::from_utf8_lossy(&out_p_deepseek.stdout);
    assert!(
        stdout_p_deepseek.contains("Switched active provider to") && stdout_p_deepseek.contains("deepseek"),
        "Expected /p alias to switch provider. Got:\n{}",
        stdout_p_deepseek
    );
}

#[test]
fn test_slash_command_provider_parsing_and_dispatch() {
    let cmd_deepseek = SlashCommand::parse("/provider deepseek").unwrap();
    assert_eq!(cmd_deepseek, SlashCommand::Provider { name: Some("deepseek".to_string()) });

    let cmd_anthropic = SlashCommand::parse("/provider anthropic").unwrap();
    assert_eq!(cmd_anthropic, SlashCommand::Provider { name: Some("anthropic".to_string()) });

    let cmd_ollama = SlashCommand::parse("/provider ollama").unwrap();
    assert_eq!(cmd_ollama, SlashCommand::Provider { name: Some("ollama".to_string()) });

    let cmd_p_alias = SlashCommand::parse("/p openai").unwrap();
    assert_eq!(cmd_p_alias, SlashCommand::Provider { name: Some("openai".to_string()) });

    let cmd_empty = SlashCommand::parse("/provider").unwrap();
    assert_eq!(cmd_empty, SlashCommand::Provider { name: None });

    // Test dispatch against AgentRunner
    let (mut runner, mut session) = create_test_runner_and_session();

    handle_slash_command("/provider deepseek", &mut runner, &mut session);
    assert_eq!(runner.config().default_provider, "deepseek");

    handle_slash_command("/provider anthropic", &mut runner, &mut session);
    assert_eq!(runner.config().default_provider, "anthropic");

    handle_slash_command("/provider ollama", &mut runner, &mut session);
    assert_eq!(runner.config().default_provider, "ollama");

    handle_slash_command("/provider openai", &mut runner, &mut session);
    assert_eq!(runner.config().default_provider, "openai");

    handle_slash_command("/provider xai", &mut runner, &mut session);
    assert_eq!(runner.config().default_provider, "xai");

    handle_slash_command("/provider openrouter", &mut runner, &mut session);
    assert_eq!(runner.config().default_provider, "openrouter");

    // Invalid provider should not change the existing active provider
    handle_slash_command("/provider invalid_prov", &mut runner, &mut session);
    assert_eq!(runner.config().default_provider, "openrouter");
}

// ===========================================================================
// Part 8: Advisor Toggle Commands Integration Tests (/advisors on, /advisors off, /advisors status)
// ===========================================================================

#[test]
fn test_cli_slash_advisors_binary_execution() {
    // 1. /advisors on
    let out_on = fusion_cmd()
        .arg("/advisors on")
        .output()
        .expect("failed to execute fusion /advisors on");
    assert!(out_on.status.success());
    let stdout_on = String::from_utf8_lossy(&out_on.stdout);
    assert!(
        stdout_on.contains("Multi-domain advisors") && stdout_on.contains("ENABLED"),
        "Expected advisors ENABLED message. Got:\n{}",
        stdout_on
    );

    // 2. /advisors off
    let out_off = fusion_cmd()
        .arg("/advisors off")
        .output()
        .expect("failed to execute fusion /advisors off");
    assert!(out_off.status.success());
    let stdout_off = String::from_utf8_lossy(&out_off.stdout);
    assert!(
        stdout_off.contains("Multi-domain advisors") && stdout_off.contains("DISABLED"),
        "Expected advisors DISABLED message. Got:\n{}",
        stdout_off
    );

    // 3. /advisors status
    let out_status = fusion_cmd()
        .arg("/advisors status")
        .output()
        .expect("failed to execute fusion /advisors status");
    assert!(out_status.status.success());
    let stdout_status = String::from_utf8_lossy(&out_status.stdout);
    assert!(
        stdout_status.contains("Advisor Critique Subsystem:") && stdout_status.contains("Active Domains:"),
        "Expected advisor status and domains listing. Got:\n{}",
        stdout_status
    );
    assert!(
        stdout_status.contains("Security Advisor") && stdout_status.contains("Architecture Advisor"),
        "Expected core advisors listed. Got:\n{}",
        stdout_status
    );

    // 4. /advisors toggle
    let out_toggle = fusion_cmd()
        .arg("/advisors toggle")
        .output()
        .expect("failed to execute fusion /advisors toggle");
    assert!(out_toggle.status.success());
    let stdout_toggle = String::from_utf8_lossy(&out_toggle.stdout);
    assert!(
        stdout_toggle.contains("Multi-domain advisors"),
        "Expected advisors toggle message. Got:\n{}",
        stdout_toggle
    );

    // 5. Short aliases /adv on and /advisor off
    let out_adv = fusion_cmd()
        .arg("/adv on")
        .output()
        .expect("failed to execute fusion /adv on");
    assert!(out_adv.status.success());
    let stdout_adv = String::from_utf8_lossy(&out_adv.stdout);
    assert!(
        stdout_adv.contains("ENABLED"),
        "Expected /adv on to enable advisors. Got:\n{}",
        stdout_adv
    );
}

#[test]
fn test_slash_command_advisors_parsing_and_state_transitions() {
    let cmd_on = SlashCommand::parse("/advisors on").unwrap();
    assert_eq!(cmd_on, SlashCommand::Advisors { state: Some("on".to_string()) });

    let cmd_off = SlashCommand::parse("/advisors off").unwrap();
    assert_eq!(cmd_off, SlashCommand::Advisors { state: Some("off".to_string()) });

    let cmd_status = SlashCommand::parse("/advisors status").unwrap();
    assert_eq!(cmd_status, SlashCommand::Advisors { state: Some("status".to_string()) });

    let cmd_toggle = SlashCommand::parse("/advisors toggle").unwrap();
    assert_eq!(cmd_toggle, SlashCommand::Advisors { state: Some("toggle".to_string()) });

    let cmd_adv = SlashCommand::parse("/adv enable").unwrap();
    assert_eq!(cmd_adv, SlashCommand::Advisors { state: Some("enable".to_string()) });

    let cmd_empty = SlashCommand::parse("/advisors").unwrap();
    assert_eq!(cmd_empty, SlashCommand::Advisors { state: None });

    // Test state transitions via execute_slash_command
    let (mut runner, mut session) = create_test_runner_and_session();

    // 1. Explicit enable ("on", "enable", "true", "1")
    runner.config_mut().advisors_enabled = false;
    handle_slash_command("/advisors on", &mut runner, &mut session);
    assert!(runner.config().advisors_enabled);

    // 2. Explicit disable ("off", "disable", "false", "0")
    handle_slash_command("/advisors off", &mut runner, &mut session);
    assert!(!runner.config().advisors_enabled);

    handle_slash_command("/advisors enable", &mut runner, &mut session);
    assert!(runner.config().advisors_enabled);

    handle_slash_command("/advisors disable", &mut runner, &mut session);
    assert!(!runner.config().advisors_enabled);

    // 3. Toggle ("toggle", "t")
    handle_slash_command("/advisors toggle", &mut runner, &mut session);
    assert!(runner.config().advisors_enabled);

    handle_slash_command("/advisors t", &mut runner, &mut session);
    assert!(!runner.config().advisors_enabled);

    // 4. Status query should leave state unchanged
    handle_slash_command("/advisors status", &mut runner, &mut session);
    assert!(!runner.config().advisors_enabled);

    handle_slash_command("/advisors info", &mut runner, &mut session);
    assert!(!runner.config().advisors_enabled);

    // 5. Unknown argument should leave state unchanged
    handle_slash_command("/advisors unknown_state_xyz", &mut runner, &mut session);
    assert!(!runner.config().advisors_enabled);
}

// ===========================================================================
// Part 9: Multi-Turn Prompt History & Slash Command Dispatch
// ===========================================================================

#[test]
fn test_prompt_history_navigation_and_duplicate_suppression() {
    let mut prompt = Prompt::new();
    assert!(prompt.history().is_empty());

    // Add first prompt
    prompt.add_history("Explain Rust ownership model");
    assert_eq!(prompt.history(), &["Explain Rust ownership model"]);

    // Add second distinct prompt
    prompt.add_history("Write a thread-safe LRU cache in Rust");
    assert_eq!(
        prompt.history(),
        &[
            "Explain Rust ownership model",
            "Write a thread-safe LRU cache in Rust"
        ]
    );

    // Add duplicate consecutive entry - should be suppressed
    prompt.add_history("Write a thread-safe LRU cache in Rust");
    assert_eq!(
        prompt.history().len(),
        2,
        "Consecutive duplicate prompt should not be appended"
    );

    // Add third prompt
    prompt.add_history("Run unit tests with cargo test");
    assert_eq!(prompt.history().len(), 3);

    // Add previous entry non-consecutively - should be appended
    prompt.add_history("Explain Rust ownership model");
    assert_eq!(prompt.history().len(), 4);
    assert_eq!(prompt.history().last().unwrap(), "Explain Rust ownership model");

    // Empty and whitespace-only entries should be ignored
    prompt.add_history("");
    prompt.add_history("   ");
    prompt.add_history("\n\t");
    assert_eq!(prompt.history().len(), 4);
}

#[test]
fn test_prompt_builder_and_custom_options() {
    let prompt = Prompt::new()
        .with_history(vec!["history item 1".to_string(), "history item 2".to_string()])
        .with_prompt_symbol(">>> ")
        .with_multiline_symbol("... ")
        .with_placeholder("Enter instructions...")
        .with_keybinding_profile(KeybindingProfile::Emacs)
        .with_mode_indicator(true);

    assert_eq!(prompt.history().len(), 2);
    assert_eq!(prompt.keybinding_profile(), KeybindingProfile::Emacs);
}

#[test]
fn test_multi_turn_session_message_history_and_token_accumulation() {
    let mut session = Session::new("deepseek-chat");
    assert_eq!(session.total_messages(), 0);
    assert_eq!(session.token_stats().total_turns, 0);
    assert_eq!(session.token_stats().total_tokens, 0);

    // Turn 1: User prompt & Assistant reply
    session.add_message(Message::user("Hello, can you help me write Rust code?"));
    assert_eq!(
        session.title(),
        Some("Hello, can you help me write Rust code?")
    );
    session.add_message(Message::assistant("Certainly! What would you like to build?"));
    session.record_tokens(Some(15), Some(12));

    assert_eq!(session.total_messages(), 2);
    assert_eq!(session.token_stats().total_turns, 1);
    assert_eq!(session.token_stats().prompt_tokens, 15);
    assert_eq!(session.token_stats().completion_tokens, 12);
    assert_eq!(session.token_stats().total_tokens, 27);

    // Turn 2: Follow-up question & Assistant reply with tool calls
    session.add_message(Message::user("Please read src/main.rs"));
    session.add_message(Message::assistant_with_tools(
        "I'll inspect src/main.rs for you.",
        vec![ToolCall {
            id: "call_read_1".to_string(),
            name: "read".to_string(),
            arguments: "{\"path\": \"src/main.rs\"}".to_string(),
        }],
    ));
    session.add_message(Message::tool_result("call_read_1", "fn main() { ... }"));
    session.add_message(Message::assistant("The main function starts the CLI loop."));
    session.record_tokens(Some(45), Some(20));

    assert_eq!(session.total_messages(), 6);
    assert_eq!(session.token_stats().total_turns, 2);
    assert_eq!(session.token_stats().prompt_tokens, 60);
    assert_eq!(session.token_stats().completion_tokens, 32);
    assert_eq!(session.token_stats().total_tokens, 92);

    // Session title was preserved from first turn
    assert_eq!(
        session.title(),
        Some("Hello, can you help me write Rust code?")
    );

    // Turn 3: Record cache tokens and format summary
    session.token_stats_mut().record_cache(100, 20);
    assert_eq!(session.token_stats().cache_read_tokens, 100);
    assert_eq!(session.token_stats().cache_write_tokens, 20);

    let summary = session.token_stats().format_summary();
    assert!(summary.contains("Tokens: 92 total"));
    assert!(summary.contains("across 2 turns"));
}

#[test]
fn test_slash_command_tokenization() {
    // Simple command
    let tokens = tokenize_command("/model deepseek-chat");
    assert_eq!(tokens, vec!["/model", "deepseek-chat"]);

    // Quoted strings with spaces
    let tokens_quotes = tokenize_command("/prompt save \"Unit Test Generator\" \"Write tests for the given Rust code.\"");
    assert_eq!(
        tokens_quotes,
        vec![
            "/prompt",
            "save",
            "Unit Test Generator",
            "Write tests for the given Rust code."
        ]
    );

    // Single quotes
    let tokens_single = tokenize_command("/bookmark add 'Refactor target: auth module'");
    assert_eq!(
        tokens_single,
        vec!["/bookmark", "add", "Refactor target: auth module"]
    );

    // Escaped characters
    let tokens_escaped = tokenize_command("/file \"path\\ with\\ spaces.rs\"");
    assert_eq!(tokens_escaped, vec!["/file", "path with spaces.rs"]);

    // Multiple whitespace separators
    let tokens_spaces = tokenize_command("   /fork    feature-branch    5   ");
    assert_eq!(tokens_spaces, vec!["/fork", "feature-branch", "5"]);

    // Empty input
    let tokens_empty = tokenize_command("   ");
    assert!(tokens_empty.is_empty());
}

#[test]
fn test_slash_command_full_catalog_parsing() {
    // Core & Navigation
    assert_eq!(
        SlashCommand::parse("/help model"),
        Some(SlashCommand::Help { command: Some("model".to_string()) })
    );
    assert_eq!(
        SlashCommand::parse("/?"),
        Some(SlashCommand::Help { command: None })
    );
    assert_eq!(
        SlashCommand::parse("/palette cost"),
        Some(SlashCommand::Palette { filter: Some("cost".to_string()) })
    );
    assert_eq!(SlashCommand::parse("/clear"), Some(SlashCommand::Clear));
    assert_eq!(SlashCommand::parse("/cls"), Some(SlashCommand::Clear));
    assert_eq!(SlashCommand::parse("/c"), Some(SlashCommand::Clear));
    assert_eq!(SlashCommand::parse("/quit"), Some(SlashCommand::Quit));
    assert_eq!(SlashCommand::parse("/exit"), Some(SlashCommand::Quit));
    assert_eq!(SlashCommand::parse("/q"), Some(SlashCommand::Quit));
    assert_eq!(SlashCommand::parse("/status"), Some(SlashCommand::Status));
    assert_eq!(SlashCommand::parse("/st"), Some(SlashCommand::Status));
    assert_eq!(
        SlashCommand::parse("/file main.rs"),
        Some(SlashCommand::File { query: Some("main.rs".to_string()) })
    );

    // Session Management & History
    assert_eq!(
        SlashCommand::parse("/session info"),
        Some(SlashCommand::Session(SessionCommand::Info))
    );
    assert_eq!(
        SlashCommand::parse("/session list"),
        Some(SlashCommand::Session(SessionCommand::List))
    );
    assert_eq!(
        SlashCommand::parse("/session new gpt-4o"),
        Some(SlashCommand::Session(SessionCommand::New { model: Some("gpt-4o".to_string()) }))
    );
    assert_eq!(
        SlashCommand::parse("/session load session-123"),
        Some(SlashCommand::Session(SessionCommand::Load { id_or_prefix: "session-123".to_string() }))
    );
    assert_eq!(
        SlashCommand::parse("/session save"),
        Some(SlashCommand::Session(SessionCommand::Save))
    );
    assert_eq!(
        SlashCommand::parse("/session delete session-456"),
        Some(SlashCommand::Session(SessionCommand::Delete { id_or_prefix: "session-456".to_string() }))
    );
    assert_eq!(
        SlashCommand::parse("/session clear"),
        Some(SlashCommand::Session(SessionCommand::Clear))
    );
    assert_eq!(
        SlashCommand::parse("/session search query_term"),
        Some(SlashCommand::Session(SessionCommand::Search { query: "query_term".to_string() }))
    );
    assert_eq!(
        SlashCommand::parse("/fork my-branch 3"),
        Some(SlashCommand::Fork { title: Some("my-branch".to_string()), turn: Some(3) })
    );
    assert_eq!(
        SlashCommand::parse("/rewind 2"),
        Some(SlashCommand::Rewind { turns: Some(2) })
    );
    assert_eq!(SlashCommand::parse("/compact"), Some(SlashCommand::Compact));
    assert_eq!(SlashCommand::parse("/stats"), Some(SlashCommand::Stats));
    assert_eq!(
        SlashCommand::parse("/export html output.html"),
        Some(SlashCommand::Export {
            format: Some(ExportFormat::Html),
            path: Some("output.html".to_string())
        })
    );
    assert_eq!(
        SlashCommand::parse("/trace trace.md"),
        Some(SlashCommand::Trace { path: Some("trace.md".to_string()) })
    );

    // Config, Tools & Customization
    assert_eq!(
        SlashCommand::parse("/config show"),
        Some(SlashCommand::Config(ConfigCommand::Show))
    );
    assert_eq!(
        SlashCommand::parse("/config path"),
        Some(SlashCommand::Config(ConfigCommand::Path))
    );
    assert_eq!(
        SlashCommand::parse("/config set default_model gpt-4o"),
        Some(SlashCommand::Config(ConfigCommand::Set {
            key: "default_model".to_string(),
            value: "gpt-4o".to_string(),
        }))
    );
    assert_eq!(
        SlashCommand::parse("/preset coding-fast"),
        Some(SlashCommand::Preset { name: Some("coding-fast".to_string()) })
    );
    assert_eq!(SlashCommand::parse("/tools"), Some(SlashCommand::Tools));
    assert_eq!(
        SlashCommand::parse("/skills list"),
        Some(SlashCommand::Skills(SkillsCommand::List))
    );
    assert_eq!(
        SlashCommand::parse("/skills enable git-helper"),
        Some(SlashCommand::Skills(SkillsCommand::Enable { name: "git-helper".to_string() }))
    );
    assert_eq!(
        SlashCommand::parse("/snippet list"),
        Some(SlashCommand::Snippet { args: vec!["list".to_string()] })
    );
    assert_eq!(
        SlashCommand::parse("/tag add important"),
        Some(SlashCommand::Tag { args: vec!["add".to_string(), "important".to_string()] })
    );
    assert_eq!(
        SlashCommand::parse("/benchmark deepseek"),
        Some(SlashCommand::Benchmark { args: vec!["deepseek".to_string()] })
    );

    // Unknown command
    assert_eq!(
        SlashCommand::parse("/custom_unknown arg1 arg2"),
        Some(SlashCommand::Unknown {
            name: "/custom_unknown".to_string(),
            args: vec!["arg1".to_string(), "arg2".to_string()],
        })
    );

    // Non-slash inputs should return None
    assert_eq!(SlashCommand::parse("What is 2 + 2?"), None);
    assert_eq!(SlashCommand::parse("Explain main.rs"), None);
    assert_eq!(SlashCommand::parse("   "), None);
}

#[test]
fn test_slash_command_dispatch_and_command_results() {
    let (mut runner, mut session) = create_test_runner_and_session();

    // 1. Regular prompt returns None
    assert!(handle_slash_command("regular user query", &mut runner, &mut session).is_none());

    // 2. /quit returns CommandResult::Exit
    let res_quit = handle_slash_command("/quit", &mut runner, &mut session);
    assert!(matches!(res_quit, Some(CommandResult::Exit)));
    assert!(res_quit.as_ref().unwrap().is_exit());
    assert!(res_quit.as_ref().unwrap().should_exit());

    // 3. /exit returns CommandResult::Exit
    let res_exit = handle_slash_command("/exit", &mut runner, &mut session);
    assert!(matches!(res_exit, Some(CommandResult::Exit)));

    // 4. /clear returns CommandResult::ScreenCleared
    let res_clear = handle_slash_command("/clear", &mut runner, &mut session);
    assert!(matches!(res_clear, Some(CommandResult::ScreenCleared)));

    // 5. /session clear returns CommandResult::SessionCleared
    let res_sess_clear = handle_slash_command("/session clear", &mut runner, &mut session);
    assert!(matches!(res_sess_clear, Some(CommandResult::SessionCleared)));

    // 6. /session new returns CommandResult::SessionSwitched
    let res_sess_new = handle_slash_command("/session new deepseek-chat", &mut runner, &mut session);
    assert!(matches!(res_sess_new, Some(CommandResult::SessionSwitched(_))));

    // 7. General slash commands return CommandResult::Continue
    let res_help = handle_slash_command("/help", &mut runner, &mut session);
    assert!(matches!(res_help, Some(CommandResult::Continue)));

    let res_status = handle_slash_command("/status", &mut runner, &mut session);
    assert!(matches!(res_status, Some(CommandResult::Continue)));

    let res_palette = handle_slash_command("/palette", &mut runner, &mut session);
    assert!(matches!(res_palette, Some(CommandResult::Continue)));

    // 8. REPL handle_command integration test
    assert!(!handle_command("/help", &mut runner, &mut session));
    assert!(!handle_command("/status", &mut runner, &mut session));
    assert!(handle_command("/quit", &mut runner, &mut session));
}

#[test]
fn test_command_palette_registry_coverage() {
    let palette = get_command_palette();
    assert!(!palette.is_empty());

    let required_commands = [
        "/help",
        "/palette",
        "/clear",
        "/file",
        "/model",
        "/provider",
        "/advisors",
        "/session",
        "/bookmark",
        "/fork",
        "/rewind",
        "/compact",
        "/stats",
        "/benchmark",
        "/export",
        "/trace",
        "/config",
        "/preset",
        "/tools",
        "/skills",
        "/snippet",
        "/tag",
        "/prompt",
        "/recover",
        "/quit",
    ];

    for req in &required_commands {
        let found = palette.iter().any(|desc| desc.name == *req || desc.aliases.contains(req));
        assert!(
            found,
            "Expected command '{}' or its alias to be present in COMMAND_PALETTE",
            req
        );
    }
}
