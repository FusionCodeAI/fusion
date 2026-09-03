use async_trait::async_trait;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

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
// Shared file helpers
// ---------------------------------------------------------------------------

/// Default maximum number of lines returned by the `read` tool when no
/// explicit `limit` is provided. Prevents unbounded output on very large
/// files; the truncation footer reports the remaining lines.
pub const DEFAULT_READ_LIMIT: usize = 2000;

/// Number of leading bytes sniffed for NUL bytes to detect binary files.
pub const BINARY_SNIFF_BYTES: usize = 8192;

/// Parameters describing a windowed, streaming read of a text file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReadWindow {
    /// 1-based line number to start reading from (minimum 1).
    pub offset: usize,
    /// Maximum number of lines to return; `None` means `DEFAULT_READ_LIMIT`.
    pub limit: Option<usize>,
    /// Whether to prefix each output line with `"{n:>6} | "`.
    pub line_numbers: bool,
}

/// Parse `offset` / `limit` / `line_numbers` tool arguments into a
/// [`ReadWindow`]. `offset` is clamped to a minimum of 1.
pub fn parse_read_window(args: &Value) -> ReadWindow {
    let offset = args
        .get("offset")
        .and_then(Value::as_u64)
        .unwrap_or(1)
        .max(1) as usize;

    let limit = args
        .get("limit")
        .and_then(Value::as_u64)
        .map(|l| l as usize);

    let line_numbers = args
        .get("line_numbers")
        .and_then(Value::as_bool)
        .unwrap_or(true);

    ReadWindow {
        offset,
        limit,
        line_numbers,
    }
}

/// Format a selected window of lines for output.
///
/// * `selected` — the lines inside the window, in order.
/// * `start_line` — 1-based line number of `selected[0]` in the source file.
/// * `total_lines` — total line count of the source file.
///
/// A truncation footer is appended when the window does not reach the end
/// of the file.
pub fn format_read_output(
    window: &ReadWindow,
    selected: &[String],
    start_line: usize,
    total_lines: usize,
) -> String {
    use std::fmt::Write as _;

    let mut output = String::new();
    for (idx, line) in selected.iter().enumerate() {
        if window.line_numbers {
            let _ = write!(output, "{:6} | {}\n", start_line + idx, line);
        } else {
            output.push_str(line);
            output.push('\n');
        }
    }

    let end = start_line.saturating_sub(1) + selected.len();
    if end < total_lines {
        let remaining = total_lines - end;
        output.push_str(&format!(
            "\n... [{} more lines in file (total: {})]\n",
            remaining, total_lines
        ));
    }

    output
}

/// Atomically write `contents` to `path`.
///
/// The bytes are written to a unique sibling temporary file, flushed to
/// stable storage, then renamed over `path`, so readers never observe a
/// partially written file. Parent directories are created as needed. On
/// failure the temporary file is removed and any pre-existing file at
/// `path` is left untouched.
pub async fn atomic_write(path: &Path, contents: &[u8]) -> anyhow::Result<()> {
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            tokio::fs::create_dir_all(parent).await.map_err(|e| {
                anyhow::anyhow!(
                    "Failed to create parent directory for '{}': {e}",
                    path.display()
                )
            })?;
        }
    }

    let mut tmp_name = path.as_os_str().to_os_string();
    tmp_name.push(format!(".tmp.{}", uuid::Uuid::new_v4()).as_str());
    let tmp_path = PathBuf::from(tmp_name);

    let result = write_tmp_and_rename(&tmp_path, path, contents).await;

    if result.is_err() {
        // Best-effort cleanup; the temp file may already have been renamed.
        let _ = tokio::fs::remove_file(&tmp_path).await;
    }

    result
}

async fn write_tmp_and_rename(tmp_path: &Path, dest: &Path, contents: &[u8]) -> anyhow::Result<()> {
    let mut file = tokio::fs::File::create(tmp_path)
        .await
        .map_err(|e| anyhow::anyhow!("Failed to create temp file '{}': {e}", tmp_path.display()))?;
    file.write_all(contents)
        .await
        .map_err(|e| anyhow::anyhow!("Failed to write temp file '{}': {e}", tmp_path.display()))?;
    file.sync_all()
        .await
        .map_err(|e| anyhow::anyhow!("Failed to flush temp file '{}': {e}", tmp_path.display()))?;
    drop(file);

    tokio::fs::rename(tmp_path, dest)
        .await
        .map_err(|e| anyhow::anyhow!("Failed to atomically replace '{}': {e}", dest.display()))
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

        let window = parse_read_window(&args);

        // Streaming read: decode the file line-by-line instead of buffering
        // the whole file into memory, and keep only the requested window.
        let file = tokio::fs::File::open(&full_path)
            .await
            .map_err(|e| anyhow::anyhow!("Failed to read file '{}': {e}", full_path.display()))?;
        let mut reader = BufReader::with_capacity(64 * 1024, file);

        let mut buf = Vec::with_capacity(256);
        let mut selected: Vec<String> = Vec::new();
        let mut total_lines = 0usize;
        let start_idx = window.offset.saturating_sub(1);
        let max_take = window.limit.unwrap_or(DEFAULT_READ_LIMIT);
        let mut binary = false;
        let mut empty = true;

        loop {
            buf.clear();
            let n = reader.read_until(b'\n', &mut buf).await.map_err(|e| {
                anyhow::anyhow!("Failed to read file '{}': {e}", full_path.display())
            })?;
            if n == 0 {
                break;
            }
            empty = false;

            if total_lines < BINARY_SNIFF_BYTES && buf.contains(&0) {
                binary = true;
            }

            total_lines += 1;

            if total_lines > start_idx && selected.len() < max_take {
                // Trim trailing newline; strip a lone trailing \r from CRLF
                // files while preserving interior carriage returns.
                if buf.last() == Some(&b'\n') {
                    buf.pop();
                    if buf.last() == Some(&b'\r') {
                        buf.pop();
                    }
                }
                match String::from_utf8(buf.clone()) {
                    Ok(line) => selected.push(line),
                    Err(_) => anyhow::bail!(
                        "File '{}' is not valid UTF-8 (line {})",
                        full_path.display(),
                        total_lines
                    ),
                }
            }

            // Stop early once the window is full AND we have counted one more
            // line to know whether more content remains.
            if selected.len() >= max_take && total_lines > start_idx + max_take {
                break;
            }
        }

        if binary {
            anyhow::bail!("Cannot read binary file '{}'", full_path.display());
        }

        if empty {
            return Ok("(empty file)".to_string());
        }

        // total_lines was only counted up to the early-exit point; when we
        // broke out early the file has more lines than counted, which the
        // footer below reports correctly via `more_remaining`.
        if window.offset > total_lines {
            anyhow::bail!(
                "Offset {} is beyond total lines ({}) in '{}'",
                window.offset,
                total_lines,
                path_str
            );
        }

        let start_line = start_idx + 1;
        let mut output = format_read_output(&window, &selected, start_line, total_lines);

        // When we broke out early (window full + one lookahead line), the
        // counted total is a lower bound; rewrite the footer so remaining
        // counts are never understated.
        if selected.len() >= max_take && total_lines == start_idx + max_take + 1 {
            let more_marker = "\n... [";
            if let Some(pos) = output.rfind(more_marker) {
                output.truncate(pos);
                output.push_str(&format!(
                    "\n... [more lines in file (read lines {}-{}, stopped early; total: {}+)]\n",
                    start_line,
                    start_line + selected.len().saturating_sub(1),
                    total_lines
                ));
            }
        }

        Ok(output)
    }
}

/// Stream-read a text file line-by-line, returning the requested window
/// plus the total number of lines scanned. Exposed for reuse and testing.
pub async fn read_window_lines(
    path: &Path,
    window: &ReadWindow,
) -> anyhow::Result<(Vec<String>, usize)> {
    let file = tokio::fs::File::open(path)
        .await
        .map_err(|e| anyhow::anyhow!("Failed to read file '{}': {e}", path.display()))?;
    let mut reader = BufReader::with_capacity(64 * 1024, file);

    let mut buf = Vec::with_capacity(256);
    let mut selected: Vec<String> = Vec::new();
    let mut total = 0usize;
    let start_idx = window.offset.saturating_sub(1);
    let max_take = window.limit.unwrap_or(DEFAULT_READ_LIMIT);

    while total <= start_idx + max_take {
        buf.clear();
        let n = reader
            .read_until(b'\n', &mut buf)
            .await
            .map_err(|e| anyhow::anyhow!("Failed to read file '{}': {e}", path.display()))?;
        if n == 0 {
            break;
        }

        if buf.contains(&0) {
            anyhow::bail!("Cannot read binary file '{}'", path.display());
        }

        total += 1;
        if total > start_idx && selected.len() < max_take {
            if buf.last() == Some(&b'\n') {
                buf.pop();
                if buf.last() == Some(&b'\r') {
                    buf.pop();
                }
            }
            let line = String::from_utf8(buf.clone()).map_err(|e| {
                anyhow::anyhow!("File '{}' is not valid UTF-8: {e}", path.display())
            })?;
            selected.push(line);
        }
    }

    Ok((selected, total))
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

        atomic_write(&full_path, content.as_bytes())
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
        let dir = std::env::temp_dir().join(format!(
            "fusion_file_test_{}_{}_{}",
            std::process::id(),
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0),
            count
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[tokio::test]
    async fn test_resolve_path() {
        let cwd = PathBuf::from("/workspace/fusion");
        assert_eq!(
            resolve_path("src/main.rs", &cwd),
            PathBuf::from("/workspace/fusion/src/main.rs")
        );
        #[cfg(unix)]
        assert_eq!(
            resolve_path("/etc/hosts", &cwd),
            PathBuf::from("/etc/hosts")
        );
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
        let write_res = write_tool
            .execute(
                json!({
                    "path": "nested/sub/dir/test.txt",
                    "content": "Line 1\nLine 2\nLine 3\nLine 4\nLine 5\n"
                }),
                &ctx,
            )
            .await;
        assert!(write_res.is_ok());
        assert!(write_res.unwrap().contains("Successfully wrote"));

        // Verify file was written
        let file_path = dir.join("nested/sub/dir/test.txt");
        assert!(file_path.exists());

        // 2. Read whole file with line numbers
        let read_res = read_tool
            .execute(
                json!({
                    "path": "nested/sub/dir/test.txt"
                }),
                &ctx,
            )
            .await
            .unwrap();
        assert!(read_res.contains("     1 | Line 1"));
        assert!(read_res.contains("     5 | Line 5"));

        // 3. Read without line numbers
        let raw_read = read_tool
            .execute(
                json!({
                    "path": "nested/sub/dir/test.txt",
                    "line_numbers": false
                }),
                &ctx,
            )
            .await
            .unwrap();
        assert_eq!(raw_read, "Line 1\nLine 2\nLine 3\nLine 4\nLine 5\n");

        // 4. Read with offset and limit
        let slice_read = read_tool
            .execute(
                json!({
                    "path": "nested/sub/dir/test.txt",
                    "offset": 2,
                    "limit": 2,
                    "line_numbers": true
                }),
                &ctx,
            )
            .await
            .unwrap();
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
        let err = read_tool
            .execute(json!({ "path": "nonexistent.txt" }), &ctx)
            .await
            .unwrap_err();
        assert!(err.to_string().contains("File not found"));

        // Path is a directory
        let sub_dir = dir.join("some_dir");
        std::fs::create_dir_all(&sub_dir).unwrap();
        let err = read_tool
            .execute(json!({ "path": "some_dir" }), &ctx)
            .await
            .unwrap_err();
        assert!(err.to_string().contains("is a directory"));

        // Binary file
        let bin_path = dir.join("binary.bin");
        std::fs::write(&bin_path, &[0x00, 0x01, 0x02, 0xFF]).unwrap();
        let err = read_tool
            .execute(json!({ "path": "binary.bin" }), &ctx)
            .await
            .unwrap_err();
        assert!(err.to_string().contains("Cannot read binary file"));

        // Invalid UTF-8 (without null bytes)
        let non_utf8_path = dir.join("invalid_utf8.txt");
        std::fs::write(&non_utf8_path, &[0x80, 0x81, 0x82, 0x83]).unwrap();
        let err = read_tool
            .execute(json!({ "path": "invalid_utf8.txt" }), &ctx)
            .await
            .unwrap_err();
        assert!(err.to_string().contains("not valid UTF-8"));

        // Offset beyond total lines
        let text_path = dir.join("short.txt");
        std::fs::write(&text_path, "one\ntwo\n").unwrap();
        let err = read_tool
            .execute(json!({ "path": "short.txt", "offset": 10 }), &ctx)
            .await
            .unwrap_err();
        assert!(err.to_string().contains("beyond total lines"));

        // Empty file
        let empty_path = dir.join("empty.txt");
        std::fs::write(&empty_path, "").unwrap();
        let empty_res = read_tool
            .execute(json!({ "path": "empty.txt" }), &ctx)
            .await
            .unwrap();
        assert_eq!(empty_res, "(empty file)");

        // Cleanup
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn test_atomic_write_creates_parents_and_overwrites() {
        let dir = temp_test_dir();
        let target = dir.join("a/b/c/target.txt");

        // 1. Write with parent creation
        atomic_write(&target, b"first\n").await.unwrap();
        assert_eq!(std::fs::read(&target).unwrap(), b"first\n");

        // 2. Overwrite leaves no temp siblings behind
        atomic_write(&target, b"second content\n").await.unwrap();
        assert_eq!(std::fs::read(&target).unwrap(), b"second content\n");

        let entries: Vec<_> = std::fs::read_dir(target.parent().unwrap())
            .unwrap()
            .filter_map(|e| e.ok())
            .collect();
        assert_eq!(entries.len(), 1, "temp file leaked: {:?}", entries);

        // 3. Empty write is valid
        atomic_write(&target, b"").await.unwrap();
        assert_eq!(std::fs::read(&target).unwrap(), b"");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn test_atomic_write_failure_preserves_existing() {
        let dir = temp_test_dir();
        let target = dir.join("keep.txt");
        atomic_write(&target, b"original\n").await.unwrap();

        // Writing over a directory path must fail without clobbering the
        // original file content.
        let sub_dir = dir.join("sub");
        std::fs::create_dir_all(&sub_dir).unwrap();
        assert!(atomic_write(&sub_dir, b"nope\n").await.is_err());

        assert_eq!(std::fs::read(&target).unwrap(), b"original\n");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_parse_read_window_defaults_and_clamps() {
        let w = parse_read_window(&json!({}));
        assert_eq!(w.offset, 1);
        assert_eq!(w.limit, None);
        assert!(w.line_numbers);

        let w = parse_read_window(&json!({ "offset": 0, "limit": 10, "line_numbers": false }));
        assert_eq!(w.offset, 1, "offset 0 must clamp to 1");
        assert_eq!(w.limit, Some(10));
        assert!(!w.line_numbers);
    }

    #[test]
    fn test_format_read_output_numbering_and_footer() {
        let window = ReadWindow {
            offset: 3,
            limit: Some(2),
            line_numbers: true,
        };
        let selected = vec!["three".to_string(), "four".to_string()];
        let out = format_read_output(&window, &selected, 3, 10);

        assert!(out.contains("     3 | three\n"));
        assert!(out.contains("     4 | four\n"));
        assert!(!out.contains("     5 | "));
        assert!(out.contains("[6 more lines in file (total: 10)]"));
    }

    #[test]
    fn test_format_read_output_no_footer_at_eof() {
        let window = ReadWindow {
            offset: 1,
            limit: Some(5),
            line_numbers: false,
        };
        let selected = vec!["a".to_string(), "b".to_string()];
        let out = format_read_output(&window, &selected, 1, 2);

        assert_eq!(out, "a\nb\n", "no footer expected at EOF, got: {out:?}");
    }

    #[tokio::test]
    async fn test_read_window_lines_streaming() {
        let dir = temp_test_dir();
        let path = dir.join("stream.txt");
        let content = (1..=500).map(|i| format!("line {i}\n")).collect::<String>();
        std::fs::write(&path, &content).unwrap();

        // Window in the middle
        let window = ReadWindow {
            offset: 100,
            limit: Some(3),
            line_numbers: true,
        };
        let (selected, total) = read_window_lines(&path, &window).await.unwrap();
        assert_eq!(selected.len(), 3);
        assert_eq!(selected[0], "line 100");
        assert_eq!(selected[2], "line 102");
        assert_eq!(total, 103);

        // Window past EOF
        let window = ReadWindow {
            offset: 600,
            limit: Some(5),
            line_numbers: false,
        };
        let (selected, total) = read_window_lines(&path, &window).await.unwrap();
        assert!(selected.is_empty());
        assert_eq!(total, 500);

        // Binary file must bail
        let bin_path = dir.join("binary.bin");
        std::fs::write(&bin_path, &[0x00, 0x01, 0x02]).unwrap();
        let window = ReadWindow {
            offset: 1,
            limit: None,
            line_numbers: false,
        };
        let err = read_window_lines(&bin_path, &window).await.unwrap_err();
        assert!(err.to_string().contains("binary"));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn test_read_tool_default_limit_truncates() {
        let dir = temp_test_dir();
        let ctx = ToolContext {
            cwd: dir.clone(),
            env: std::collections::HashMap::new(),
        };
        let read_tool = ReadFileTool::new();

        // More lines than DEFAULT_READ_LIMIT
        let path = dir.join("big.txt");
        let content = (1..=(DEFAULT_READ_LIMIT + 50))
            .map(|i| format!("L{i}\n"))
            .collect::<String>();
        std::fs::write(&path, &content).unwrap();

        let res = read_tool
            .execute(json!({ "path": "big.txt" }), &ctx)
            .await
            .unwrap();
        assert!(res.contains(&format!("{:6} | L1\n", 1)));
        assert!(res.contains("more lines in file"));
        assert!(!res.contains(&format!(
            "{:6} | L{}\n",
            DEFAULT_READ_LIMIT + 1,
            DEFAULT_READ_LIMIT + 1
        )));

        let _ = std::fs::remove_dir_all(&dir);
    }
}
