use async_trait::async_trait;
use serde_json::{json, Value};
use similar::{ChangeTag, TextDiff};
use std::path::{Path, PathBuf};

use crate::tools::types::{Tool, ToolContext};

/// Resolve a relative or absolute path against a working directory.
/// Also expands leading `~` to user home dir if available.
pub fn resolve_path(path_str: &str, cwd: &Path) -> PathBuf {
    if let Some(stripped) = path_str.strip_prefix("~/") {
        if let Some(home) = dirs::home_dir() {
            return home.join(stripped);
        }
    } else if path_str == "~" {
        if let Some(home) = dirs::home_dir() {
            return home;
        }
    }

    let p = Path::new(path_str);
    if p.is_absolute() {
        p.to_path_buf()
    } else {
        cwd.join(p)
    }
}

/// Statistics on the diff between old and new text.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct DiffStats {
    pub additions: usize,
    pub deletions: usize,
}

impl DiffStats {
    pub fn is_empty(&self) -> bool {
        self.additions == 0 && self.deletions == 0
    }
}

/// Compute diff stats (lines added and deleted) between two strings.
pub fn compute_diff_stats(old_content: &str, new_content: &str) -> DiffStats {
    let diff = TextDiff::from_lines(old_content, new_content);
    let mut stats = DiffStats::default();

    for change in diff.iter_all_changes() {
        match change.tag() {
            ChangeTag::Delete => stats.deletions += 1,
            ChangeTag::Insert => stats.additions += 1,
            ChangeTag::Equal => {}
        }
    }

    stats
}

/// Generate a standard unified diff string between old and new content.
pub fn generate_unified_diff(
    old_content: &str,
    new_content: &str,
    file_path: &str,
    context_radius: usize,
) -> String {
    let diff = TextDiff::from_lines(old_content, new_content);
    diff.unified_diff()
        .context_radius(context_radius)
        .header(&format!("a/{}", file_path), &format!("b/{}", file_path))
        .to_string()
}

/// Generate an ANSI-colorized unified diff for terminal rendering.
/// - Red (`\x1b[31m`) for deleted lines
/// - Green (`\x1b[32m`) for inserted lines
/// - Cyan (`\x1b[36m`) for hunk headers
/// - Bold (`\x1b[1m`) for file headers
pub fn generate_colorized_diff(
    old_content: &str,
    new_content: &str,
    file_path: &str,
    context_radius: usize,
) -> String {
    let diff = TextDiff::from_lines(old_content, new_content);
    let mut output = String::new();
    let mut header_printed = false;

    let header_a = format!("a/{}", file_path);
    let header_b = format!("b/{}", file_path);
    let mut unified_builder = diff.unified_diff();
    let udiff = unified_builder
        .context_radius(context_radius)
        .header(&header_a, &header_b);
    for hunk in udiff.iter_hunks() {
        if !header_printed {
            output.push_str(&format!("\x1b[1m--- a/{}\x1b[0m\n", file_path));
            output.push_str(&format!("\x1b[1m+++ b/{}\x1b[0m\n", file_path));
            header_printed = true;
        }

        output.push_str(&format!("\x1b[36m{}\x1b[0m\n", hunk.header()));

        for change in hunk.iter_changes() {
            match change.tag() {
                ChangeTag::Delete => {
                    output.push_str("\x1b[31m-");
                    output.push_str(change.value());
                    output.push_str("\x1b[0m");
                    if !change.value().ends_with('\n') {
                        output.push('\n');
                    }
                }
                ChangeTag::Insert => {
                    output.push_str("\x1b[32m+");
                    output.push_str(change.value());
                    output.push_str("\x1b[0m");
                    if !change.value().ends_with('\n') {
                        output.push('\n');
                    }
                }
                ChangeTag::Equal => {
                    output.push(' ');
                    output.push_str(change.value());
                    if !change.value().ends_with('\n') {
                        output.push('\n');
                    }
                }
            }
        }
    }

    output
}

/// Apply exact search-and-replace to content, verifying uniqueness of `old_text`.
pub fn apply_exact_edit(
    current_content: &str,
    old_text: &str,
    new_text: &str,
    path_display: &str,
) -> anyhow::Result<String> {
    if old_text.is_empty() {
        anyhow::bail!("old_text cannot be empty. Please specify the exact snippet to replace.");
    }

    let matches_count = current_content.matches(old_text).count();

    if matches_count == 0 {
        // Diagnostic check: test if mismatch is due to CRLF / LF differences
        let normalized_content = current_content.replace("\r\n", "\n");
        let normalized_old = old_text.replace("\r\n", "\n");
        let normalized_matches = normalized_content.matches(&normalized_old).count();

        if normalized_matches > 0 {
            anyhow::bail!(
                "old_text not found in '{}' due to line ending differences (CRLF vs LF). \
                The file uses different newline conventions than old_text. \
                Please ensure newlines match the file exactly.",
                path_display
            );
        }

        anyhow::bail!(
            "old_text not found in '{}'. Ensure old_text matches the file content exactly, \
            including whitespace, indentation, and line breaks.",
            path_display
        );
    }

    if matches_count > 1 {
        anyhow::bail!(
            "old_text occurs {} times in '{}'. It must appear exactly once to ensure unambiguous replacement. \
            Add surrounding context lines to make old_text unique.",
            matches_count,
            path_display
        );
    }

    Ok(current_content.replacen(old_text, new_text, 1))
}

// ---------------------------------------------------------------------------
// EditFileTool
// ---------------------------------------------------------------------------

#[derive(Default, Debug, Clone)]
pub struct EditFileTool;

impl EditFileTool {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl Tool for EditFileTool {
    fn name(&self) -> &str {
        "edit"
    }

    fn description(&self) -> &str {
        "Edit an existing file by replacing an exact, unique occurrence of old_text with new_text. Generates a unified diff of changes."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "Path to the file to edit (relative to workspace or absolute)."
                },
                "old_text": {
                    "type": "string",
                    "description": "Exact text to be replaced (must appear uniquely in the file)."
                },
                "new_text": {
                    "type": "string",
                    "description": "New replacement text."
                }
            },
            "required": ["path", "old_text", "new_text"]
        })
    }

    async fn execute(&self, args: Value, ctx: &ToolContext) -> anyhow::Result<String> {
        let path_str = args
            .get("path")
            .and_then(|v| v.as_str())
            .or_else(|| args.get("file_path").and_then(|v| v.as_str()))
            .or_else(|| args.get("file").and_then(|v| v.as_str()))
            .ok_or_else(|| anyhow::anyhow!("Missing required parameter: path"))?;

        let old_text = args
            .get("old_text")
            .and_then(|v| v.as_str())
            .or_else(|| args.get("old_string").and_then(|v| v.as_str()))
            .or_else(|| args.get("old_content").and_then(|v| v.as_str()))
            .ok_or_else(|| anyhow::anyhow!("Missing required parameter: old_text"))?;

        let new_text = args
            .get("new_text")
            .and_then(|v| v.as_str())
            .or_else(|| args.get("new_string").and_then(|v| v.as_str()))
            .or_else(|| args.get("new_content").and_then(|v| v.as_str()))
            .ok_or_else(|| anyhow::anyhow!("Missing required parameter: new_text"))?;

        let full_path = resolve_path(path_str, &ctx.cwd);

        if !full_path.exists() {
            anyhow::bail!("File not found: '{}'", full_path.display());
        }

        if full_path.is_dir() {
            anyhow::bail!("Path is a directory, not a file: '{}'", full_path.display());
        }

        let current_content = tokio::fs::read_to_string(&full_path)
            .await
            .map_err(|e| anyhow::anyhow!("Failed to read file '{}': {e}", full_path.display()))?;

        if old_text == new_text {
            return Ok(format!(
                "No changes made to '{}' (old_text is identical to new_text).",
                path_str
            ));
        }

        let updated_content = apply_exact_edit(&current_content, old_text, new_text, path_str)?;

        tokio::fs::write(&full_path, updated_content.as_bytes())
            .await
            .map_err(|e| anyhow::anyhow!("Failed to write updated file '{}': {e}", full_path.display()))?;

        let stats = compute_diff_stats(&current_content, &updated_content);
        let unified_diff = generate_unified_diff(&current_content, &updated_content, path_str, 3);

        if unified_diff.trim().is_empty() {
            Ok(format!(
                "File '{}' updated successfully (no line differences).",
                path_str
            ))
        } else {
            Ok(format!(
                "Successfully edited '{}' (+{} -{} lines):\n\n```diff\n{}```",
                path_str,
                stats.additions,
                stats.deletions,
                unified_diff
            ))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_apply_exact_edit_success() {
        let content = "fn main() {\n    println!(\"hello\");\n}\n";
        let old = "println!(\"hello\");";
        let new = "println!(\"world\");";

        let result = apply_exact_edit(content, old, new, "main.rs").unwrap();
        assert_eq!(result, "fn main() {\n    println!(\"world\");\n}\n");
    }

    #[test]
    fn test_apply_exact_edit_not_found() {
        let content = "fn main() {\n    println!(\"hello\");\n}\n";
        let old = "println!(\"missing\");";
        let new = "println!(\"world\");";

        let err = apply_exact_edit(content, old, new, "main.rs").unwrap_err();
        assert!(err.to_string().contains("old_text not found in 'main.rs'"));
    }

    #[test]
    fn test_apply_exact_edit_multiple_occurrences() {
        let content = "let x = 1;\nlet x = 1;\n";
        let old = "let x = 1;";
        let new = "let x = 2;";

        let err = apply_exact_edit(content, old, new, "test.rs").unwrap_err();
        assert!(err.to_string().contains("occurs 2 times in 'test.rs'"));
        assert!(err.to_string().contains("unambiguous"));
    }

    #[test]
    fn test_apply_exact_edit_empty_old() {
        let content = "hello";
        let err = apply_exact_edit(content, "", "world", "test.txt").unwrap_err();
        assert!(err.to_string().contains("old_text cannot be empty"));
    }

    #[test]
    fn test_compute_diff_stats() {
        let old = "line1\nline2\nline3\n";
        let new = "line1\nline2_modified\nline3\nline4\n";

        let stats = compute_diff_stats(old, new);
        assert_eq!(stats.deletions, 1);
        assert_eq!(stats.additions, 2);
    }

    #[test]
    fn test_generate_unified_diff() {
        let old = "fn foo() {\n    1\n}\n";
        let new = "fn foo() {\n    2\n}\n";

        let diff = generate_unified_diff(old, new, "foo.rs", 3);
        assert!(diff.contains("--- a/foo.rs"));
        assert!(diff.contains("+++ b/foo.rs"));
        assert!(diff.contains("-    1"));
        assert!(diff.contains("+    2"));
    }

    #[test]
    fn test_generate_colorized_diff() {
        let old = "line 1\nold line\nline 3\n";
        let new = "line 1\nnew line\nline 3\n";

        let color_diff = generate_colorized_diff(old, new, "sample.txt", 3);
        assert!(color_diff.contains("--- a/sample.txt"));
        assert!(color_diff.contains("+++ b/sample.txt"));
        assert!(color_diff.contains("\x1b[31m-old line"));
        assert!(color_diff.contains("\x1b[32m+new line"));
    }

    #[tokio::test]
    async fn test_edit_tool_execute() {
        let temp_dir = std::env::temp_dir().join(format!("fusion_edit_test_{}", uuid::Uuid::new_v4()));
        tokio::fs::create_dir_all(&temp_dir).await.unwrap();

        let file_path = temp_dir.join("test.txt");
        tokio::fs::write(&file_path, "apple\nbanana\ncherry\n").await.unwrap();

        let tool = EditFileTool::new();
        let ctx = ToolContext {
            cwd: temp_dir.clone(),
            env: std::collections::HashMap::new(),
        };

        // Test editing
        let args = json!({
            "path": "test.txt",
            "old_text": "banana",
            "new_text": "blueberry"
        });

        let output = tool.execute(args, &ctx).await.unwrap();
        assert!(output.contains("Successfully edited 'test.txt'"));
        assert!(output.contains("-banana"));
        assert!(output.contains("+blueberry"));

        let updated = tokio::fs::read_to_string(&file_path).await.unwrap();
        assert_eq!(updated, "apple\nblueberry\ncherry\n");

        // Clean up
        let _ = tokio::fs::remove_dir_all(&temp_dir).await;
    }
}
