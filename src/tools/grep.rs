use async_trait::async_trait;
use ignore::WalkBuilder;
use regex::RegexBuilder;
use serde_json::{json, Value};
use std::path::Path;

use crate::tools::file::resolve_path;
use crate::tools::types::{Tool, ToolContext};

// ---------------------------------------------------------------------------
// Helper functions
// ---------------------------------------------------------------------------

/// Check if a byte slice appears to be binary data (contains null bytes in initial probe).
fn is_binary(bytes: &[u8]) -> bool {
    bytes.iter().take(4096).any(|&b| b == 0)
}

/// Safely truncate a long line at a UTF-8 character boundary.
fn truncate_line(line: &str, max_len: usize) -> String {
    if line.len() <= max_len {
        line.to_string()
    } else {
        let end = line
            .char_indices()
            .map(|(idx, _)| idx)
            .take_while(|&idx| idx <= max_len)
            .last()
            .unwrap_or(0);
        format!("{}...", &line[..end])
    }
}

// ---------------------------------------------------------------------------
// GrepTool
// ---------------------------------------------------------------------------

#[derive(Default, Debug, Clone)]
pub struct GrepTool;

impl GrepTool {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl Tool for GrepTool {
    fn name(&self) -> &str {
        "grep"
    }

    fn description(&self) -> &str {
        "Recursively search file contents for a regular expression pattern, respecting .gitignore and skipping binary files."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "pattern": {
                    "type": "string",
                    "description": "Regular expression pattern to search for."
                },
                "path": {
                    "type": "string",
                    "description": "Directory or file path to search within (optional, defaults to workspace root)."
                },
                "case_sensitive": {
                    "type": "boolean",
                    "description": "Whether the regex search is case-sensitive (optional, default: true)."
                },
                "hidden": {
                    "type": "boolean",
                    "description": "Whether to include hidden files in search (optional, default: false)."
                },
                "max_results": {
                    "type": "integer",
                    "description": "Maximum number of matching lines to return (optional, default: 200)."
                }
            },
            "required": ["pattern"]
        })
    }

    async fn execute(&self, args: Value, ctx: &ToolContext) -> anyhow::Result<String> {
        let pattern = args
            .get("pattern")
            .and_then(|v| v.as_str())
            .or_else(|| args.get("regex").and_then(|v| v.as_str()))
            .ok_or_else(|| anyhow::anyhow!("Missing required parameter: pattern"))?;

        if pattern.is_empty() {
            anyhow::bail!("Search pattern cannot be empty");
        }

        let case_sensitive = args
            .get("case_sensitive")
            .and_then(|v| v.as_bool())
            .unwrap_or(true);

        let include_hidden = args
            .get("hidden")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        let max_results = args
            .get("max_results")
            .and_then(|v| v.as_u64())
            .map(|n| n as usize)
            .unwrap_or(200);

        let path_str = args
            .get("path")
            .and_then(|v| v.as_str())
            .or_else(|| args.get("dir").and_then(|v| v.as_str()));

        let search_path = match path_str {
            Some(p) => resolve_path(p, &ctx.cwd),
            None => ctx.cwd.clone(),
        };

        if !search_path.exists() {
            anyhow::bail!("Path not found: '{}'", search_path.display());
        }

        let regex = RegexBuilder::new(pattern)
            .case_insensitive(!case_sensitive)
            .build()
            .map_err(|e| anyhow::anyhow!("Invalid regular expression '{}': {e}", pattern))?;

        let cwd = ctx.cwd.clone();
        let target_path = search_path.clone();

        // Run file traversal and regex matching in a blocking threadpool task
        let (results, total_count) = tokio::task::spawn_blocking(move || -> anyhow::Result<(Vec<String>, usize)> {
            let mut results = Vec::new();
            let mut total_count = 0;

            if target_path.is_file() {
                // Search single file directly
                search_single_file(&target_path, &cwd, &regex, max_results, &mut results, &mut total_count);
            } else {
                // Walk directory tree respecting gitignore
                let mut builder = WalkBuilder::new(&target_path);
                builder
                    .hidden(!include_hidden)
                    .git_ignore(true)
                    .git_global(true)
                    .git_exclude(true)
                    .require_git(false)
                    .parents(true);

                for entry_result in builder.build() {
                    let entry = match entry_result {
                        Ok(e) => e,
                        Err(e) => {
                            tracing::debug!("Grep walk error: {e}");
                            continue;
                        }
                    };

                    let path = entry.path();
                    if !path.is_file() {
                        continue;
                    }

                    // Skip unusually large files (> 20 MB) to prevent OOM
                    if let Ok(metadata) = entry.metadata() {
                        if metadata.len() > 20 * 1024 * 1024 {
                            continue;
                        }
                    }

                    search_single_file(path, &cwd, &regex, max_results, &mut results, &mut total_count);
                }
            }

            Ok((results, total_count))
        })
        .await
        .map_err(|e| anyhow::anyhow!("Grep task execution failed: {e}"))??;

        if results.is_empty() {
            return Ok(format!(
                "No matches found for regex '{}' in '{}'",
                pattern,
                search_path.display()
            ));
        }

        let mut output = results.join("\n");
        if total_count > results.len() {
            output.push_str(&format!(
                "\n\n... [{} additional matches truncated; max_results={}]",
                total_count - results.len(),
                max_results
            ));
        }

        Ok(output)
    }
}

fn search_single_file(
    file_path: &Path,
    cwd: &Path,
    regex: &regex::Regex,
    max_results: usize,
    results: &mut Vec<String>,
    total_count: &mut usize,
) {
    let bytes = match std::fs::read(file_path) {
        Ok(b) => b,
        Err(_) => return,
    };

    if is_binary(&bytes) {
        return;
    }

    let text = match std::str::from_utf8(&bytes) {
        Ok(s) => s,
        Err(_) => return,
    };

    let rel_path = file_path.strip_prefix(cwd).unwrap_or(file_path);
    let rel_path_str = rel_path.to_string_lossy();

    for (idx, line) in text.lines().enumerate() {
        if regex.is_match(line) {
            *total_count += 1;
            if results.len() < max_results {
                let formatted_line = truncate_line(line, 400);
                results.push(format!("{}:{}: {}", rel_path_str, idx + 1, formatted_line));
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs::{self, File};
    use std::io::Write;
    use std::path::PathBuf;

    struct TestDir {
        path: PathBuf,
    }

    impl TestDir {
        fn new(prefix: &str) -> Self {
            let unique = uuid::Uuid::new_v4();
            let path = std::env::temp_dir().join(format!("fusion_grep_test_{}_{}", prefix, unique));
            fs::create_dir_all(&path).expect("Failed to create test directory");
            Self { path }
        }

        fn write_file(&self, rel_path: &str, content: &[u8]) -> PathBuf {
            let full_path = self.path.join(rel_path);
            if let Some(parent) = full_path.parent() {
                fs::create_dir_all(parent).expect("Failed to create parent dirs");
            }
            let mut file = File::create(&full_path).expect("Failed to create file");
            file.write_all(content).expect("Failed to write test file");
            full_path
        }
    }

    impl Drop for TestDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.path);
        }
    }

    #[tokio::test]
    async fn test_grep_basic_matching() {
        let temp = TestDir::new("basic");
        temp.write_file("src/main.rs", b"fn main() {\n    println!(\"Hello World!\");\n}\n");
        temp.write_file("src/lib.rs", b"pub fn helper() -> bool {\n    true\n}\n");

        let tool = GrepTool::new();
        let ctx = ToolContext {
            cwd: temp.path.clone(),
            env: std::collections::HashMap::new(),
        };

        let result = tool
            .execute(
                json!({
                    "pattern": "println!",
                }),
                &ctx,
            )
            .await
            .expect("grep execution failed");

        assert!(result.contains("src/main.rs:2:     println!(\"Hello World!\");"));
        assert!(!result.contains("src/lib.rs"));
    }

    #[tokio::test]
    async fn test_grep_case_sensitivity() {
        let temp = TestDir::new("case");
        temp.write_file("test.txt", b"Alpha\nbeta\nALPHA\n");

        let tool = GrepTool::new();
        let ctx = ToolContext {
            cwd: temp.path.clone(),
            env: std::collections::HashMap::new(),
        };

        // Case sensitive (default)
        let res_case = tool
            .execute(
                json!({
                    "pattern": "ALPHA",
                    "case_sensitive": true,
                }),
                &ctx,
            )
            .await
            .unwrap();

        assert_eq!(res_case.lines().count(), 1);
        assert!(res_case.contains("test.txt:3: ALPHA"));

        // Case insensitive
        let res_nocase = tool
            .execute(
                json!({
                    "pattern": "alpha",
                    "case_sensitive": false,
                }),
                &ctx,
            )
            .await
            .unwrap();

        assert_eq!(res_nocase.lines().count(), 2);
    }

    #[tokio::test]
    async fn test_grep_skips_binary_and_respects_gitignore() {
        let temp = TestDir::new("ignore_binary");
        temp.write_file(".gitignore", b"ignored.txt\n");
        temp.write_file("ignored.txt", b"MATCH_SECRET in ignored file\n");
        temp.write_file("visible.txt", b"MATCH_SECRET in visible file\n");

        // Binary file with null bytes
        let mut binary_content = Vec::new();
        binary_content.extend_from_slice(b"MATCH_SECRET in binary ");
        binary_content.push(0);
        binary_content.extend_from_slice(b" rest of binary");
        temp.write_file("binary.bin", &binary_content);

        let tool = GrepTool::new();
        let ctx = ToolContext {
            cwd: temp.path.clone(),
            env: std::collections::HashMap::new(),
        };

        let res = tool
            .execute(
                json!({
                    "pattern": "MATCH_SECRET",
                }),
                &ctx,
            )
            .await
            .unwrap();

        assert!(res.contains("visible.txt:1: MATCH_SECRET in visible file"));
        assert!(!res.contains("ignored.txt"));
        assert!(!res.contains("binary.bin"));
    }

    #[tokio::test]
    async fn test_grep_single_file_search() {
        let temp = TestDir::new("single");
        temp.write_file("doc.md", b"# Title\nLine 2 content\nLine 3 query match\n");

        let tool = GrepTool::new();
        let ctx = ToolContext {
            cwd: temp.path.clone(),
            env: std::collections::HashMap::new(),
        };

        let res = tool
            .execute(
                json!({
                    "pattern": "query match",
                    "path": "doc.md",
                }),
                &ctx,
            )
            .await
            .unwrap();

        assert!(res.contains("doc.md:3: Line 3 query match"));
    }

    #[tokio::test]
    async fn test_grep_invalid_regex() {
        let tool = GrepTool::new();
        let ctx = ToolContext::default();

        let err = tool
            .execute(
                json!({
                    "pattern": "[unclosed_regex",
                }),
                &ctx,
            )
            .await;

        assert!(err.is_err());
        assert!(err.unwrap_err().to_string().contains("Invalid regular expression"));
    }
}
