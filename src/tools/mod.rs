pub mod bash;
pub mod clipboard;
pub mod edit;
pub mod env_cleaner;
pub mod fetch;
pub mod web_search;
pub mod file;
pub mod glob;
pub mod grep;
pub mod grep_filter;
pub mod search;
pub mod git;
pub mod git_branch;
pub mod guardrails;
pub mod patch;
pub mod watch;
pub mod system;
pub mod syntax;
pub mod mcp;
pub mod symbols;
pub mod process;
pub mod compat;
pub mod types;
pub mod sqlite;
pub mod mock_server;
pub mod regex_test;
pub mod deps;
pub mod tree;
pub mod docgen;
pub mod ports;
pub mod git_log;
pub mod hex;
pub mod diff_stats;
pub mod json_schema;
pub mod github;
pub use bash::BashTool;
pub use clipboard::{
    ClipboardBackendKind, ClipboardManager, ClipboardStatus, ClipboardTool, ReadClipboardTool,
    WriteClipboardTool, DEFAULT_CLIPBOARD_TIMEOUT,
};
pub use edit::EditFileTool;
pub use env_cleaner::{
    is_sensitive, is_sensitive_key, is_sensitive_value, mask_value, sanitize_env, EnvCleaner,
    EnvSanitizer, SanitizationPolicy, SanitizationReason, SanitizationReport, SanitizationResult,
};
pub use file::{ReadFileTool, WriteFileTool};
pub use fetch::{FetchFormat, FetchOptions, FetchResult, HttpFetchTool};
pub use grep::GrepTool;
pub use grep_filter::{
    FileTypeRegistry, FilterableGrepEngine, GrepFilter, GrepMatch, GrepOptions, GrepPathFilter,
    GrepSearchResult, PathFilter, PathFilterBuilder,
};
pub use glob::GlobTool;
pub use git::{GitDiffTool, GitStatusTool};
pub use git_branch::{
    format_branch_list, list_branches, parse_upstream_tracking, validate_branch_name,
    BranchInfo, BranchListReport, BranchOpResult, GitBranchTool,
};
pub use guardrails::*;
pub use patch::PatchTool;
pub use watch::{
    global_watcher_manager, ChangeKind, FileChange, FileRecord, FileSnapshot, WatchConfig,
    WatchTool, WatcherInfo, WatcherManager, WorkspaceWatcher,
};
pub use web_search::WebSearchTool;
pub use system::*;
pub use syntax::*;
pub use mcp::*;
pub use symbols::{
    scan_workspace, Language, Symbol, SymbolKind, SymbolQuery, SymbolScanner, SymbolsTool,
};
pub use sqlite::{
    parse_columns_from_ddl, ColumnDef, MasterEntry, QueryResult, Row, SqlValue, SqliteHeader,
    SqliteReader, SqliteTool, TableSchema,
};
pub use process::{
    global_process_manager, LogOutput, ManagedProcess, OutputBuffer, OutputLine, OutputStream,
    ProcessConfig, ProcessInfo, ProcessManager, ProcessStatus, ProcessTool, GLOBAL_PROCESS_MANAGER,
};
pub use types::*;
pub use compat::*;
pub use regex_test::*;
pub use mock_server::*;
pub use deps::*;
pub use tree::*;
pub use docgen::*;
pub use ports::*;
pub use git_log::*;
pub use hex::*;
pub use diff_stats::*;
pub use json_schema::*;
pub use github::*;

use std::sync::Arc;

/// Create and return the default tool registry populated with standard cross-platform tools:
/// `bash`, `read`, `write`, `edit`, `grep`, and `glob`.
pub fn default_registry() -> ToolRegistry {
    let mut registry = ToolRegistry::new();
    registry.register(Arc::new(BashTool::new()));
    registry.register(Arc::new(ReadFileTool::new()));
    registry.register(Arc::new(WriteFileTool::new()));
    registry.register(Arc::new(EditFileTool::new()));
    registry.register(Arc::new(GrepTool::new()));
    registry.register(Arc::new(GlobTool::new()));
    registry.register(Arc::new(GitStatusTool::new()));
    registry.register(Arc::new(GitDiffTool::new()));
    registry.register(Arc::new(GitBranchTool::new()));
    registry.register(Arc::new(PatchTool::new()));
    registry.register(Arc::new(WatchTool::new()));
    registry.register(Arc::new(WebSearchTool::new()));
    registry.register(Arc::new(ClipboardTool::new()));
    registry.register(Arc::new(SystemInfoTool::new()));
    registry.register(Arc::new(crate::agent::memory::MemoryTool::new()));
    registry.register(Arc::new(SyntaxCheckTool::new()));
    registry.register(Arc::new(HttpFetchTool::new()));
    registry.register(Arc::new(SymbolsTool::new()));
    registry.register(Arc::new(SqliteTool::new()));
    registry.register(Arc::new(RegexTestTool::new()));
    registry.register(Arc::new(ProcessTool::new()));
    registry.register(Arc::new(MockServerTool::new()));
    registry.register(Arc::new(DepsTool::new()));
    registry.register(Arc::new(TreeTool::new()));
    registry.register(Arc::new(DocgenTool::new()));
    registry.register(Arc::new(PortScannerTool::new()));
    registry.register(Arc::new(GitLogTool::new()));
    registry.register(Arc::new(crate::agent::planner_dag::PlanSubagentDagTool));
    registry.register(Arc::new(HexViewerTool::new()));
    registry.register(Arc::new(DiffStatsTool::new()));
    registry.register(Arc::new(JsonSchemaTool::new()));
    registry.register(Arc::new(GitHubTool::new()));
    compat::register_compat_tools(&mut registry);
    registry
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::fs;
    use tempfile_helper::TempDir;

    // A small RAII temp dir helper for isolated tool tests
    mod tempfile_helper {
        use std::path::{Path, PathBuf};
        pub struct TempDir(pub PathBuf);
        impl TempDir {
            pub fn new() -> Self {
                let p = std::env::temp_dir().join(format!("fusion_test_{}", uuid::Uuid::new_v4()));
                std::fs::create_dir_all(&p).unwrap();
                Self(p)
            }
            pub fn path(&self) -> &Path {
                &self.0
            }
        }
        impl Drop for TempDir {
            fn drop(&mut self) {
                let _ = std::fs::remove_dir_all(&self.0);
            }
        }
    }

    #[tokio::test]
    async fn test_default_registry() {
        let reg = default_registry();
        assert!(reg.get("bash").is_some());
        assert!(reg.get("read").is_some());
        assert!(reg.get("read_file").is_some());
        assert!(reg.get("write").is_some());
        assert!(reg.get("write_file").is_some());
        assert!(reg.get("edit").is_some());
        assert!(reg.get("edit_file").is_some());
        assert!(reg.get("grep").is_some());
        assert!(reg.get("glob").is_some());
        assert!(reg.get("git_status").is_some());
        assert!(reg.get("status").is_some());
        assert!(reg.get("git_diff").is_some());
        assert!(reg.get("diff").is_some());
        assert!(reg.get("git_log").is_some());
        assert!(reg.get("patch").is_some());
        assert!(reg.get("apply_patch").is_some());
        assert!(reg.get("web_search").is_some());
        assert!(reg.get("watch").is_some());
        assert!(reg.get("file_watch").is_some());
        assert!(reg.get("clipboard").is_some());
        assert!(reg.get("pbcopy").is_some());
        assert!(reg.get("pbpaste").is_some());
        assert!(reg.get("clip").is_some());
        assert!(reg.get("xclip").is_some());
        assert!(reg.get("wl-copy").is_some());
        assert!(reg.get("system_info").is_some());
        assert!(reg.get("system").is_some());
        assert!(reg.get("sys_info").is_some());
        assert!(reg.get("sysinfo").is_some());
        assert!(reg.get("host_info").is_some());
        assert!(reg.get("syntax_check").is_some());
        assert!(reg.get("syntax").is_some());
        assert!(reg.get("fetch").is_some());
        assert!(reg.get("http_fetch").is_some());
        assert!(reg.get("web_fetch").is_some());
        assert!(reg.get("curl").is_some());
        assert!(reg.get("symbols").is_some());
        assert!(reg.get("symbol").is_some());
        assert!(reg.get("workspace_symbols").is_some());
        assert!(reg.get("process").is_some());
        assert!(reg.get("bg_process").is_some());
        assert!(reg.get("proc").is_some());
        assert!(reg.definitions().len() >= 13);
    }
    #[tokio::test]
    async fn test_file_tools_roundtrip() {
        let temp = TempDir::new();
        let ctx = ToolContext {
            cwd: temp.path().to_path_buf(),
            env: std::collections::HashMap::new(),
        };

        let write_tool = WriteFileTool::new();
        let read_tool = ReadFileTool::new();
        let edit_tool = EditFileTool::new();

        // 1. Write file
        let write_res = write_tool
            .execute(
                json!({
                    "path": "test.txt",
                    "content": "Line 1\nLine 2\nLine 3"
                }),
                &ctx,
            )
            .await;
        assert!(write_res.is_ok());

        // 2. Read file with line numbers
        let read_res = read_tool
            .execute(
                json!({
                    "path": "test.txt"
                }),
                &ctx,
            )
            .await
            .unwrap();
        assert!(read_res.contains("Line 1"));
        assert!(read_res.contains("Line 2"));
        assert!(read_res.contains("Line 3"));

        // 3. Read with offset and limit
        let read_slice = read_tool
            .execute(
                json!({
                    "path": "test.txt",
                    "offset": 2,
                    "limit": 1
                }),
                &ctx,
            )
            .await
            .unwrap();
        assert!(!read_slice.contains("Line 1"));
        assert!(read_slice.contains("Line 2"));
        assert!(!read_slice.contains("Line 3"));

        // 4. Edit file
        let edit_res = edit_tool
            .execute(
                json!({
                    "path": "test.txt",
                    "old_text": "Line 2",
                    "new_text": "Line Two (edited)"
                }),
                &ctx,
            )
            .await
            .unwrap();
        assert!(edit_res.contains("Successfully edited"));

        // 5. Verify edited content
        let read_after_edit = read_tool
            .execute(
                json!({
                    "path": "test.txt",
                    "line_numbers": false
                }),
                &ctx,
            )
            .await
            .unwrap();
        assert_eq!(read_after_edit.trim(), "Line 1\nLine Two (edited)\nLine 3");
    }

    #[tokio::test]
    async fn test_search_tools() {
        let temp = TempDir::new();
        let ctx = ToolContext {
            cwd: temp.path().to_path_buf(),
            env: std::collections::HashMap::new(),
        };

        fs::write(temp.path().join("file_a.rs"), "fn hello_world() {}\n").unwrap();
        fs::write(temp.path().join("file_b.rs"), "fn goodbye_world() {}\n").unwrap();

        let grep_tool = GrepTool::new();
        let glob_tool = GlobTool::new();

        // Test grep
        let grep_res = grep_tool
            .execute(
                json!({
                    "pattern": "hello_\\w+"
                }),
                &ctx,
            )
            .await
            .unwrap();
        assert!(grep_res.contains("file_a.rs"));
        assert!(grep_res.contains("hello_world"));

        // Test glob
        let glob_res = glob_tool
            .execute(
                json!({
                    "pattern": "*.rs"
                }),
                &ctx,
            )
            .await
            .unwrap();
        assert!(glob_res.contains("file_a.rs"));
        assert!(glob_res.contains("file_b.rs"));
    }

    #[tokio::test]
    async fn test_bash_tool() {
        let temp = TempDir::new();
        let ctx = ToolContext {
            cwd: temp.path().to_path_buf(),
            env: std::collections::HashMap::new(),
        };

        let bash_tool = BashTool::new();
        let res = bash_tool
            .execute(
                json!({
                    "command": "echo 'Hello Fusion'"
                }),
                &ctx,
            )
            .await
            .unwrap();
        assert!(res.contains("Hello Fusion"));
    }

    #[tokio::test]
    async fn test_syntax_check_tool() {
        let temp = TempDir::new();
        let ctx = ToolContext {
            cwd: temp.path().to_path_buf(),
            env: std::collections::HashMap::new(),
        };

        let syntax_tool = SyntaxCheckTool::new();

        // Direct content test
        let res = syntax_tool
            .execute(
                json!({
                    "content": "fn foo() { let x = (1 + 2); }",
                    "language": "rust"
                }),
                &ctx,
            )
            .await
            .unwrap();
        assert!(res.contains("Syntax validation passed"));

        // Direct content test with syntax error
        let err_res = syntax_tool
            .execute(
                json!({
                    "content": "fn foo() { let x = (1 + 2; }",
                    "language": "rust"
                }),
                &ctx,
            )
            .await
            .unwrap();
        assert!(err_res.contains("Syntax validation failed"));
        assert!(err_res.contains("mismatched-delimiter") || err_res.contains("unclosed-delimiter"));

        // JSON format output
        let json_res = syntax_tool
            .execute(
                json!({
                    "content": "{\"key\": \"val\"}",
                    "language": "json",
                    "format": "json"
                }),
                &ctx,
            )
            .await
            .unwrap();
        assert!(json_res.contains("\"valid\": true"));
    }

    #[tokio::test]
    async fn test_regex_test_tool_in_registry() {
        let registry = default_registry();
        let ctx = ToolContext::default();

        assert!(registry.contains("regex_test"));
        assert!(registry.contains("regex"));

        let res = registry
            .execute(
                "regex_test",
                json!({
                    "pattern": r"(?P<word>\w+)\s+(?P<num>\d+)",
                    "input": "item 42, count 100",
                    "replacement": "${word}=${num}"
                }),
                &ctx,
            )
            .await
            .unwrap();

        assert!(res.contains("REGEX EVALUATION REPORT"));
        assert!(res.contains("2 matches found"));
        assert!(res.contains("item=42"));
    }

    #[tokio::test]
    async fn test_github_tool_in_registry() {
        let registry = default_registry();
        assert!(registry.contains("github"));
        assert!(registry.contains("gh"));
        assert!(registry.contains("gh_pr"));
        assert!(registry.contains("github_pr"));
        assert!(registry.contains("pull_request"));
        let tool = registry.get("github").expect("github tool should be in registry");
        assert_eq!(tool.name(), "github");
    }
}
