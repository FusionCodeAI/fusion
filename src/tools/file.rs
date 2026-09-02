use async_trait::async_trait;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

use crate::tools::types::{Tool, ToolContext};

pub fn resolve_path(path_str: &str, cwd: &Path) -> PathBuf {
    let p = Path::new(path_str);
    if p.is_absolute() {
        p.to_path_buf()
    } else {
        cwd.join(p)
    }
}

// ---------------------------------------------------------------------------
// ReadFileTool
// ---------------------------------------------------------------------------

#[derive(Default, Debug, Clone)]
pub struct ReadFileTool;

impl ReadFileTool {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl Tool for ReadFileTool {
    fn name(&self) -> &str {
        "read"
    }

    fn description(&self) -> &str {
        "Read the text content of a file with optional line offset, limit, and line numbering."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "Path to the file to read (relative to workspace or absolute)."
                },
                "offset": {
                    "type": "integer",
                    "description": "1-based line number to start reading from (optional, default: 1)."
                },
                "limit": {
                    "type": "integer",
                    "description": "Maximum number of lines to read (optional)."
                },
                "line_numbers": {
                    "type": "boolean",
                    "description": "Whether to prefix each line with its 1-based line number (optional, default: true)."
                }
            },
            "required": ["path"]
        })
    }

    async fn execute(&self, args: Value, ctx: &ToolContext) -> anyhow::Result<String> {
        let path_str = args
            .get("path")
            .and_then(|v| v.as_str())
            .or_else(|| args.get("file_path").and_then(|v| v.as_str()))
            .ok_or_else(|| anyhow::anyhow!("Missing required parameter: path"))?;

        let full_path = resolve_path(path_str, &ctx.cwd);

        if !full_path.exists() {
            anyhow::bail!("File not found: '{}'", full_path.display());
        }

        if full_path.is_dir() {
            anyhow::bail!("Path is a directory, not a file: '{}'", full_path.display());
        }

        let bytes = tokio::fs::read(&full_path)
            .await
            .map_err(|e| anyhow::anyhow!("Failed to read file '{}': {e}", full_path.display()))?;

        // Simple binary check
        if bytes.iter().take(8192).any(|&b| b == 0) {
            anyhow::bail!("Cannot read binary file '{}'", full_path.display());
        }

        let content = String::from_utf8(bytes)
            .map_err(|e| anyhow::anyhow!("File '{}' is not valid UTF-8: {e}", full_path.display()))?;

        if content.is_empty() {
            return Ok("(empty file)".to_string());
        }

        let offset = args
            .get("offset")
            .and_then(|v| v.as_u64())
            .unwrap_or(1)
            .max(1) as usize;

        let limit = args
            .get("limit")
            .and_then(|v| v.as_u64())
            .map(|l| l as usize);

        let show_line_numbers = args
            .get("line_numbers")
            .and_then(|v| v.as_bool())
            .unwrap_or(true);

        let lines: Vec<&str> = content.lines().collect();
        let total_lines = lines.len();

        if offset > total_lines && total_lines > 0 {
            anyhow::bail!(
                "Offset {} is beyond total lines ({}) in '{}'",
                offset,
                total_lines,
                path_str
            );
        }

        let start_idx = offset.saturating_sub(1);
        let max_take = limit.unwrap_or(usize::MAX);
        let selected_lines = lines.iter().skip(start_idx).take(max_take);

        let mut output = String::new();
        for (idx, line) in selected_lines.enumerate() {
            let line_num = offset + idx;
            if show_line_numbers {
                output.push_str(&format!("{:6} | {}\n", line_num, line));
            } else {
                output.push_str(line);
                output.push('\n');
            }
        }

        if let Some(lim) = limit {
            let displayed = lines.len().saturating_sub(start_idx).min(lim);
            if start_idx + displayed < total_lines {
                let remaining = total_lines - (start_idx + displayed);
                output.push_str(&format!("\n... [{} more lines in file (total: {})]\n", remaining, total_lines));
            }
        }

        Ok(output)
    }
}

// ---------------------------------------------------------------------------
// WriteFileTool
// ---------------------------------------------------------------------------

#[derive(Default, Debug, Clone)]
pub struct WriteFileTool;

impl WriteFileTool {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl Tool for WriteFileTool {
    fn name(&self) -> &str {
        "write"
    }

    fn description(&self) -> &str {
        "Write content to a file. Automatically creates parent directories if they do not exist."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "Path to the file to write (relative to workspace or absolute)."
                },
                "content": {
                    "type": "string",
                    "description": "Content to write into the file."
                }
            },
            "required": ["path", "content"]
        })
    }

    async fn execute(&self, args: Value, ctx: &ToolContext) -> anyhow::Result<String> {
        let path_str = args
            .get("path")
            .and_then(|v| v.as_str())
            .or_else(|| args.get("file_path").and_then(|v| v.as_str()))
            .ok_or_else(|| anyhow::anyhow!("Missing required parameter: path"))?;

        let content = args
            .get("content")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("Missing required parameter: content"))?;

        let full_path = resolve_path(path_str, &ctx.cwd);

        if let Some(parent) = full_path.parent() {
            tokio::fs::create_dir_all(parent)
                .await
                .map_err(|e| anyhow::anyhow!("Failed to create parent directory for '{}': {e}", full_path.display()))?;
        }

        tokio::fs::write(&full_path, content.as_bytes())
            .await
            .map_err(|e| anyhow::anyhow!("Failed to write file '{}': {e}", full_path.display()))?;

        let lines_count = content.lines().count();
        let bytes_count = content.len();

        Ok(format!(
            "Successfully wrote {} bytes ({} lines) to '{}'",
            bytes_count, lines_count, path_str
        ))
    }
}

// Re-export EditFileTool from the dedicated edit module
pub use crate::tools::edit::EditFileTool;

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    static COUNTER: AtomicUsize = AtomicUsize::new(0);

    fn temp_test_dir() -> PathBuf {
        let count = COUNTER.fetch_add(1, Ordering::SeqCst);
        let dir = std::env::temp_dir().join(format!("fusion_file_test_{}_{}_{}", std::process::id(), chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0), count));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[tokio::test]
    async fn test_resolve_path() {
        let cwd = PathBuf::from("/workspace/fusion");
        assert_eq!(resolve_path("src/main.rs", &cwd), PathBuf::from("/workspace/fusion/src/main.rs"));
        #[cfg(unix)]
        assert_eq!(resolve_path("/etc/hosts", &cwd), PathBuf::from("/etc/hosts"));
    }

    #[tokio::test]
    async fn test_write_and_read_file_tool() {
        let dir = temp_test_dir();
        let ctx = ToolContext {
            cwd: dir.clone(),
            env: std::collections::HashMap::new(),
        };

        let write_tool = WriteFileTool::new();
        let read_tool = ReadFileTool::new();

        // 1. Write file with parent dir creation
        let write_res = write_tool.execute(json!({
            "path": "nested/sub/dir/test.txt",
            "content": "Line 1\nLine 2\nLine 3\nLine 4\nLine 5\n"
        }), &ctx).await;
        assert!(write_res.is_ok());
        assert!(write_res.unwrap().contains("Successfully wrote"));

        // Verify file was written
        let file_path = dir.join("nested/sub/dir/test.txt");
        assert!(file_path.exists());

        // 2. Read whole file with line numbers
        let read_res = read_tool.execute(json!({
            "path": "nested/sub/dir/test.txt"
        }), &ctx).await.unwrap();
        assert!(read_res.contains("     1 | Line 1"));
        assert!(read_res.contains("     5 | Line 5"));

        // 3. Read without line numbers
        let raw_read = read_tool.execute(json!({
            "path": "nested/sub/dir/test.txt",
            "line_numbers": false
        }), &ctx).await.unwrap();
        assert_eq!(raw_read, "Line 1\nLine 2\nLine 3\nLine 4\nLine 5\n");

        // 4. Read with offset and limit
        let slice_read = read_tool.execute(json!({
            "path": "nested/sub/dir/test.txt",
            "offset": 2,
            "limit": 2,
            "line_numbers": true
        }), &ctx).await.unwrap();
        assert!(slice_read.contains("     2 | Line 2"));
        assert!(slice_read.contains("     3 | Line 3"));
        assert!(!slice_read.contains("     1 | Line 1"));
        assert!(!slice_read.contains("     4 | Line 4"));
        assert!(slice_read.contains("more lines in file"));

        // Cleanup
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn test_read_errors() {
        let dir = temp_test_dir();
        let ctx = ToolContext {
            cwd: dir.clone(),
            env: std::collections::HashMap::new(),
        };
        let read_tool = ReadFileTool::new();

        // File not found
        let err = read_tool.execute(json!({ "path": "nonexistent.txt" }), &ctx).await.unwrap_err();
        assert!(err.to_string().contains("File not found"));

        // Path is a directory
        let sub_dir = dir.join("some_dir");
        std::fs::create_dir_all(&sub_dir).unwrap();
        let err = read_tool.execute(json!({ "path": "some_dir" }), &ctx).await.unwrap_err();
        assert!(err.to_string().contains("is a directory"));

        // Binary file
        let bin_path = dir.join("binary.bin");
        std::fs::write(&bin_path, &[0x00, 0x01, 0x02, 0xFF]).unwrap();
        let err = read_tool.execute(json!({ "path": "binary.bin" }), &ctx).await.unwrap_err();
        assert!(err.to_string().contains("Cannot read binary file"));

        // Invalid UTF-8 (without null bytes)
        let non_utf8_path = dir.join("invalid_utf8.txt");
        std::fs::write(&non_utf8_path, &[0x80, 0x81, 0x82, 0x83]).unwrap();
        let err = read_tool.execute(json!({ "path": "invalid_utf8.txt" }), &ctx).await.unwrap_err();
        assert!(err.to_string().contains("not valid UTF-8"));

        // Offset beyond total lines
        let text_path = dir.join("short.txt");
        std::fs::write(&text_path, "one\ntwo\n").unwrap();
        let err = read_tool.execute(json!({ "path": "short.txt", "offset": 10 }), &ctx).await.unwrap_err();
        assert!(err.to_string().contains("beyond total lines"));

        // Empty file
        let empty_path = dir.join("empty.txt");
        std::fs::write(&empty_path, "").unwrap();
        let empty_res = read_tool.execute(json!({ "path": "empty.txt" }), &ctx).await.unwrap();
        assert_eq!(empty_res, "(empty file)");

        // Cleanup
        let _ = std::fs::remove_dir_all(&dir);
    }
}
