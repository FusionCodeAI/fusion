pub mod bash;
pub mod edit;
pub mod file;
pub mod glob;
pub mod grep;
pub mod search;
pub mod types;

pub use bash::BashTool;
pub use edit::EditFileTool;
pub use file::{ReadFileTool, WriteFileTool};
pub use grep::GrepTool;
pub use glob::GlobTool;
pub use types::*;

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
        assert_eq!(reg.definitions().len(), 6);
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
}
