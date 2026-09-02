//! Integration Smoke Tests for Fusion
//!
//! Verifies end-to-end functionality of:
//! 1. Tool execution (read, write, edit, bash, grep, glob, and tool registry)
//! 2. Configuration loading, serialization, and provider URL/key resolution
//! 3. Conversational session management, serialization, and persistence

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde_json::json;
use uuid::Uuid;

use fusion::agent::session::{Session, SessionSummary};
use fusion::config::Config;
use fusion::provider::types::{Message, Role, ToolCall};
use fusion::tools::{
    default_registry, BashTool, EditFileTool, GlobTool, GrepTool, ReadFileTool, Tool, ToolContext,
    ToolRegistry, WriteFileTool,
};

// ---------------------------------------------------------------------------
// RAII Temp Directory Helper
// ---------------------------------------------------------------------------

struct TempDir {
    path: PathBuf,
}

impl TempDir {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!("fusion_smoke_test_{}", Uuid::new_v4()));
        fs::create_dir_all(&path).expect("Failed to create temporary directory for smoke test");
        Self { path }
    }

    fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

// ---------------------------------------------------------------------------
// 1. Tool Execution Tests
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_tool_registry_and_definitions() {
    let registry = default_registry();

    // Verify all primary tools and aliases exist in the registry
    assert!(registry.get("bash").is_some(), "bash tool missing");
    assert!(registry.get("read").is_some(), "read tool missing");
    assert!(registry.get("read_file").is_some(), "read_file alias missing");
    assert!(registry.get("write").is_some(), "write tool missing");
    assert!(registry.get("write_file").is_some(), "write_file alias missing");
    assert!(registry.get("edit").is_some(), "edit tool missing");
    assert!(registry.get("edit_file").is_some(), "edit_file alias missing");
    assert!(registry.get("grep").is_some(), "grep tool missing");
    assert!(registry.get("glob").is_some(), "glob tool missing");

    // Definitions list should contain the 6 registered tools
    let defs = registry.definitions();
    assert_eq!(defs.len(), 6, "Expected 6 tool definitions in default registry");

    let tool_names: Vec<String> = defs.into_iter().map(|d| d.name).collect();
    assert!(tool_names.contains(&"bash".to_string()));
    assert!(tool_names.contains(&"read".to_string()));
    assert!(tool_names.contains(&"write".to_string()));
    assert!(tool_names.contains(&"edit".to_string()));
    assert!(tool_names.contains(&"grep".to_string()));
    assert!(tool_names.contains(&"glob".to_string()));
}

#[tokio::test]
async fn test_tool_file_read_write_edit_cycle() {
    let temp = TempDir::new();
    let ctx = ToolContext {
        cwd: temp.path().to_path_buf(),
        env: HashMap::new(),
    };

    let registry = default_registry();

    // 1. Write a new file in a nested directory via Registry
    let write_res = registry
        .execute(
            "write",
            json!({
                "path": "nested/dir/sample.txt",
                "content": "Line 1: Alpha\nLine 2: Beta\nLine 3: Gamma\nLine 4: Delta"
            }),
            &ctx,
        )
        .await;

    assert!(write_res.is_ok(), "Write tool execution failed: {:?}", write_res);
    let write_output = write_res.unwrap();
    assert!(write_output.contains("Successfully wrote"));

    // Verify file exists on disk
    let file_path = temp.path().join("nested/dir/sample.txt");
    assert!(file_path.exists(), "File was not created on disk");

    // 2. Read full file with line numbers
    let read_res = registry
        .execute(
            "read",
            json!({
                "path": "nested/dir/sample.txt",
                "line_numbers": true
            }),
            &ctx,
        )
        .await;

    assert!(read_res.is_ok(), "Read tool execution failed: {:?}", read_res);
    let read_output = read_res.unwrap();
    assert!(read_output.contains("1 | Line 1: Alpha"));
    assert!(read_output.contains("2 | Line 2: Beta"));
    assert!(read_output.contains("3 | Line 3: Gamma"));
    assert!(read_output.contains("4 | Line 4: Delta"));

    // 3. Read with offset and limit
    let read_slice_res = registry
        .execute(
            "read_file",
            json!({
                "path": "nested/dir/sample.txt",
                "offset": 2,
                "limit": 2,
                "line_numbers": false
            }),
            &ctx,
        )
        .await;

    assert!(read_slice_res.is_ok());
    let slice_output = read_slice_res.unwrap();
    assert!(!slice_output.contains("Line 1: Alpha"));
    assert!(slice_output.contains("Line 2: Beta"));
    assert!(slice_output.contains("Line 3: Gamma"));
    assert!(!slice_output.contains("Line 4: Delta"));

    // 4. Edit file: replace 'Line 2: Beta' with 'Line 2: Beta_Modified'
    let edit_res = registry
        .execute(
            "edit",
            json!({
                "path": "nested/dir/sample.txt",
                "old_text": "Line 2: Beta",
                "new_text": "Line 2: Beta_Modified"
            }),
            &ctx,
        )
        .await;

    assert!(edit_res.is_ok(), "Edit tool failed: {:?}", edit_res);
    let edit_output = edit_res.unwrap();
    assert!(edit_output.contains("Successfully edited"));
    assert!(edit_output.contains("-Line 2: Beta"));
    assert!(edit_output.contains("+Line 2: Beta_Modified"));

    // 5. Verify edited content with Read tool
    let read_after_edit = registry
        .execute(
            "read",
            json!({
                "path": "nested/dir/sample.txt",
                "line_numbers": false
            }),
            &ctx,
        )
        .await
        .unwrap();

    assert!(read_after_edit.contains("Line 2: Beta_Modified"));
    assert!(!read_after_edit.contains("Line 2: Beta\n"));

    // 6. Test Edit tool failure on missing text
    let edit_fail_missing = registry
        .execute(
            "edit",
            json!({
                "path": "nested/dir/sample.txt",
                "old_text": "Non-existent string",
                "new_text": "Replacement"
            }),
            &ctx,
        )
        .await;

    assert!(edit_fail_missing.is_err());

    // 7. Test Read tool error on non-existent file
    let read_fail = registry
        .execute(
            "read",
            json!({
                "path": "non_existent_file.txt"
            }),
            &ctx,
        )
        .await;

    assert!(read_fail.is_err());
}

#[tokio::test]
async fn test_edit_tool_ambiguous_match_error() {
    let temp = TempDir::new();
    let ctx = ToolContext {
        cwd: temp.path().to_path_buf(),
        env: HashMap::new(),
    };

    let write_tool = WriteFileTool::new();
    let edit_tool = EditFileTool::new();

    write_tool
        .execute(
            json!({
                "path": "duplicate.txt",
                "content": "repeated string\nsome middle line\nrepeated string\n"
            }),
            &ctx,
        )
        .await
        .unwrap();

    let edit_res = edit_tool
        .execute(
            json!({
                "path": "duplicate.txt",
                "old_text": "repeated string",
                "new_text": "new string"
            }),
            &ctx,
        )
        .await;

    assert!(edit_res.is_err(), "Expected edit to fail when old_text is ambiguous");
    let err_msg = edit_res.unwrap_err().to_string();
    assert!(err_msg.contains("occurs 2 times"));
}

#[tokio::test]
async fn test_bash_tool_execution() {
    let temp = TempDir::new();
    let mut env = HashMap::new();
    env.insert("TEST_VAR".to_string(), "FUSION_SMOKE_OK".to_string());

    let ctx = ToolContext {
        cwd: temp.path().to_path_buf(),
        env,
    };

    let bash_tool = BashTool::new();

    // 1. Simple echo command
    #[cfg(not(windows))]
    let cmd = "echo \"Hello from $TEST_VAR\"";
    #[cfg(windows)]
    let cmd = "echo Hello from %TEST_VAR%";

    let result = bash_tool
        .execute(
            json!({
                "command": cmd,
                "timeout_secs": 10
            }),
            &ctx,
        )
        .await;

    assert!(result.is_ok(), "Bash tool execution failed: {:?}", result);
    let output = result.unwrap();
    assert!(output.contains("FUSION_SMOKE_OK"), "Output did not contain expected env var: {}", output);

    // 2. Empty command should fail validation
    let empty_res = bash_tool
        .execute(
            json!({
                "command": "   "
            }),
            &ctx,
        )
        .await;

    assert!(empty_res.is_err());

    // 3. Failing command should return non-zero exit code error
    #[cfg(not(windows))]
    let fail_cmd = "exit 42";
    #[cfg(windows)]
    let fail_cmd = "exit /b 42";

    let fail_res = bash_tool
        .execute(
            json!({
                "command": fail_cmd
            }),
            &ctx,
        )
        .await;

    assert!(fail_res.is_err());
    let err_msg = fail_res.unwrap_err().to_string();
    assert!(err_msg.contains("42"));
}

#[tokio::test]
async fn test_search_tools_grep_and_glob() {
    let temp = TempDir::new();
    let ctx = ToolContext {
        cwd: temp.path().to_path_buf(),
        env: HashMap::new(),
    };

    let write_tool = WriteFileTool::new();
    let grep_tool = GrepTool::new();
    let glob_tool = GlobTool::new();

    // Create a directory tree for searching
    write_tool
        .execute(
            json!({
                "path": "src/main.rs",
                "content": "fn main() {\n    println!(\"Hello Fusion Engine!\");\n}\n"
            }),
            &ctx,
        )
        .await
        .unwrap();

    write_tool
        .execute(
            json!({
                "path": "src/lib.rs",
                "content": "pub fn fusion_init() -> bool {\n    true\n}\n"
            }),
            &ctx,
        )
        .await
        .unwrap();

    write_tool
        .execute(
            json!({
                "path": "docs/readme.txt",
                "content": "Fusion Documentation\nVersion 2.0\n"
            }),
            &ctx,
        )
        .await
        .unwrap();

    // 1. Grep for "fusion_init"
    let grep_res = grep_tool
        .execute(
            json!({
                "pattern": "fusion_init"
            }),
            &ctx,
        )
        .await;

    assert!(grep_res.is_ok(), "Grep failed: {:?}", grep_res);
    let grep_out = grep_res.unwrap();
    assert!(grep_out.contains("src/lib.rs"));
    assert!(grep_out.contains("pub fn fusion_init"));

    // 2. Grep case-insensitive
    let grep_ci = grep_tool
        .execute(
            json!({
                "pattern": "DOCUMENTATION",
                "case_sensitive": false
            }),
            &ctx,
        )
        .await
        .unwrap();

    assert!(grep_ci.contains("docs/readme.txt"));
    assert!(grep_ci.contains("Fusion Documentation"));

    // 3. Glob for Rust files: `**/*.rs`
    let glob_rs = glob_tool
        .execute(
            json!({
                "pattern": "**/*.rs"
            }),
            &ctx,
        )
        .await;

    assert!(glob_rs.is_ok(), "Glob failed: {:?}", glob_rs);
    let glob_out = glob_rs.unwrap();
    assert!(glob_out.contains("src/main.rs"));
    assert!(glob_out.contains("src/lib.rs"));
    assert!(!glob_out.contains("docs/readme.txt"));

    // 4. Glob for text files: `docs/*.txt`
    let glob_txt = glob_tool
        .execute(
            json!({
                "pattern": "docs/*.txt"
            }),
            &ctx,
        )
        .await
        .unwrap();

    assert!(glob_txt.contains("docs/readme.txt"));
    assert!(!glob_txt.contains("src/main.rs"));

    // 5. Grep for non-existent pattern
    let grep_none = grep_tool
        .execute(
            json!({
                "pattern": "PatternNotFoundInAnyFileXYZ"
            }),
            &ctx,
        )
        .await
        .unwrap();

    assert!(grep_none.contains("No matches found"));
}

// ---------------------------------------------------------------------------
// 2. Config Loading and Provider Resolution Tests
// ---------------------------------------------------------------------------

#[test]
fn test_config_defaults_and_serialization() {
    let cfg = Config::default();

    // Verify baseline defaults
    assert_eq!(cfg.default_provider, "deepseek");
    assert_eq!(cfg.default_model, "deepseek-chat");
    assert_eq!(cfg.default_temperature, Some(0.2));
    assert_eq!(cfg.max_tokens, Some(8192));
    assert!(cfg.advisors_enabled);

    // Test JSON Serialization & Deserialization
    let json_str = serde_json::to_string_pretty(&cfg).expect("Failed to serialize Config to JSON");
    assert!(json_str.contains("\"default_provider\": \"deepseek\""));
    assert!(json_str.contains("\"default_model\": \"deepseek-chat\""));

    let deserialized: Config = serde_json::from_str(&json_str).expect("Failed to deserialize Config");
    assert_eq!(deserialized.default_provider, cfg.default_provider);
    assert_eq!(deserialized.default_model, cfg.default_model);
    assert_eq!(deserialized.advisors_enabled, cfg.advisors_enabled);
    assert_eq!(deserialized.max_tokens, cfg.max_tokens);
}

#[test]
fn test_config_provider_url_and_key_resolution() {
    let mut cfg = Config::default();
    cfg.openai_api_key = Some("sk-openai-test-key".to_string());
    cfg.anthropic_api_key = Some("sk-ant-test-key".to_string());
    cfg.deepseek_api_key = Some("sk-ds-test-key".to_string());
    cfg.xai_api_key = Some("xai-test-key".to_string());
    cfg.openrouter_api_key = Some("sk-or-test-key".to_string());

    // 1. OpenAI
    let (key, url) = cfg.get_key_and_url("openai");
    assert_eq!(key, Some("sk-openai-test-key".to_string()));
    assert_eq!(url, "https://api.openai.com/v1");

    // 2. Anthropic
    let (key, url) = cfg.get_key_and_url("anthropic");
    assert_eq!(key, Some("sk-ant-test-key".to_string()));
    assert_eq!(url, "https://api.anthropic.com/v1");

    // 3. DeepSeek
    let (key, url) = cfg.get_key_and_url("deepseek");
    assert_eq!(key, Some("sk-ds-test-key".to_string()));
    assert_eq!(url, "https://api.deepseek.com");

    // 4. xAI / Grok
    let (key, url) = cfg.get_key_and_url("xai");
    assert_eq!(key, Some("xai-test-key".to_string()));
    assert_eq!(url, "https://api.x.ai/v1");

    let (key, url) = cfg.get_key_and_url("grok");
    assert_eq!(key, Some("xai-test-key".to_string()));
    assert_eq!(url, "https://api.x.ai/v1");

    // 5. OpenRouter
    let (key, url) = cfg.get_key_and_url("openrouter");
    assert_eq!(key, Some("sk-or-test-key".to_string()));
    assert_eq!(url, "https://openrouter.ai/api/v1");

    // 6. Ollama
    let (key, url) = cfg.get_key_and_url("ollama");
    assert_eq!(key, None);
    assert_eq!(url, "http://localhost:11434");

    // 7. Custom base URL overrides
    cfg.openai_base_url = Some("https://custom.openai.proxy/v1".to_string());
    let (_, custom_url) = cfg.get_key_and_url("openai");
    assert_eq!(custom_url, "https://custom.openai.proxy/v1");
}

// ---------------------------------------------------------------------------
// 3. Session Management and Serialization Tests
// ---------------------------------------------------------------------------

#[test]
fn test_session_lifecycle_and_messages() {
    let mut session = Session::new("claude-3-5-sonnet");

    assert_eq!(session.active_model(), "claude-3-5-sonnet");
    assert_eq!(session.total_messages(), 0);
    assert!(session.title.is_none());

    // 1. Add System Message
    session.add_system_message("You are an expert Rust software architect.");
    assert_eq!(session.total_messages(), 1);
    assert_eq!(session.messages()[0].role, Role::System);

    // 2. Add User Message (should auto-generate title)
    session.add_user_message("Please help me refactor the database module to use connection pooling.");
    assert_eq!(session.total_messages(), 2);
    assert_eq!(session.messages()[1].role, Role::User);
    assert!(session.title.is_some());
    let title = session.title.as_ref().unwrap();
    assert!(title.starts_with("Please help me refactor"));

    // 3. Add Assistant Message with Tool Calls
    let tool_call = ToolCall {
        id: "call_read_123".to_string(),
        name: "read".to_string(),
        arguments: "{\"path\": \"src/db.rs\"}".to_string(),
    };
    session.add_assistant_with_tools("Checking the current DB implementation...", vec![tool_call]);
    assert_eq!(session.total_messages(), 3);
    assert_eq!(session.messages()[2].role, Role::Assistant);
    assert!(session.messages()[2].tool_calls.is_some());

    // 4. Add Tool Result
    session.add_tool_result("call_read_123", "struct Database { connection: String }");
    assert_eq!(session.total_messages(), 4);
    assert_eq!(session.messages()[3].role, Role::Tool);
    assert_eq!(session.messages()[3].tool_call_id, Some("call_read_123".to_string()));

    // 5. Add Final Assistant Message
    session.add_assistant_message("I suggest introducing `r2d2` or `deadpool` for connection pooling.");
    assert_eq!(session.total_messages(), 5);

    // 6. Test Truncation
    session.truncate(3);
    assert_eq!(session.total_messages(), 3);

    // 7. Test Clear
    session.clear();
    assert_eq!(session.total_messages(), 0);
}

#[test]
fn test_session_json_serialization_and_persistence() {
    let temp = TempDir::new();
    let session_id = Uuid::new_v4();
    let mut session = Session::with_id(session_id, "gpt-4o");

    session.add_user_message("Write a smoke test for Fusion");
    session.add_assistant_message("Here is the smoke test implementation...");

    // Serialize to JSON string
    let json_str = serde_json::to_string_pretty(&session).expect("Failed to serialize session");
    assert!(json_str.contains("\"active_model\": \"gpt-4o\""));
    assert!(json_str.contains("Write a smoke test for Fusion"));

    // Write to a temporary file path
    let file_path = temp.path().join("test_session.json");
    fs::write(&file_path, &json_str).expect("Failed to write session file");

    // Load back via `load_from_path`
    let loaded_session = Session::load_from_path(&file_path).expect("Failed to load session from path");

    assert_eq!(loaded_session.id(), session_id);
    assert_eq!(loaded_session.active_model(), "gpt-4o");
    assert_eq!(loaded_session.total_messages(), 2);
    assert_eq!(loaded_session.messages()[0].content, "Write a smoke test for Fusion");
    assert_eq!(loaded_session.messages()[1].content, "Here is the smoke test implementation...");
    assert_eq!(loaded_session.title, session.title);
}

#[test]
fn test_session_summary_representation() {
    let mut session = Session::new("deepseek-coder");
    session.add_user_message("How do I build a cross-platform TUI in Rust?");
    session.add_assistant_message("You can use Ratatui with Crossterm backend in inline mode.");

    let summary = SessionSummary {
        id: session.id,
        created_at: session.created_at.clone(),
        updated_at: session.updated_at.clone(),
        active_model: session.active_model.clone(),
        title: session.title.clone(),
        message_count: session.messages.len(),
        preview: "You can use Ratatui with Crossterm backend in inline mode.".to_string(),
    };

    assert_eq!(summary.message_count, 2);
    assert_eq!(summary.active_model, "deepseek-coder");
    assert!(summary.preview.contains("Ratatui"));

    // Verify summary JSON roundtrip
    let json_summary = serde_json::to_string(&summary).unwrap();
    let deser_summary: SessionSummary = serde_json::from_str(&json_summary).unwrap();
    assert_eq!(deser_summary.id, summary.id);
    assert_eq!(deser_summary.message_count, 2);
}
