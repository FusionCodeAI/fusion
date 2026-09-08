use async_trait::async_trait;
use serde_json::{json, Value};
use similar::{ChangeTag, TextDiff};
use std::path::{Path, PathBuf};

use fusion_edit::modes::hashline::{
    apply::{apply_edits, ApplyOptions, EmptyPaste},
    block::{has_block_edit, resolve_block_edits, Unresolved},
    input::{contains_recognizable_hashline_operations, Patch, SplitOptions},
};

use crate::tools::file::atomic_write;
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

/// Compute diff stats (lines added and deleted) between two strings using fusion_diff.
pub fn compute_diff_stats(old_content: &str, new_content: &str) -> DiffStats {
    let changes = fusion_diff::line_changes_str(old_content, new_content);
    let mut stats = DiffStats::default();

    for change in changes {
        if change.added {
            stats.additions += change.count as usize;
        } else if change.removed {
            stats.deletions += change.count as usize;
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
        // Diagnostic check: is the mismatch caused by CRLF / LF differences?
        // Also suggest the closest matching line so the model can self-correct.
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
            including whitespace, indentation, and line breaks.{}",
            path_display,
            suggest_closest_line(current_content, old_text).unwrap_or_default(),
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

/// Build a short hint pointing at the closest matching line when `old_text`
/// cannot be found, to help diagnose near-miss edits (e.g. wrong indentation).
/// Returns `None` when no line is remotely similar.
fn suggest_closest_line(content: &str, old_text: &str) -> Option<String> {
    let needle: Vec<char> = old_text.chars().take(200).collect();
    if needle.is_empty() {
        return None;
    }

    let mut best: Option<(f64, usize, &str)> = None;
    for (idx, line) in content.lines().enumerate() {
        let haystack: Vec<char> = line.chars().collect();
        let score = trigram_similarity(&needle, &haystack);
        if best.map_or(true, |(b, _, _)| score > b) {
            best = Some((score, idx + 1, line));
        }
    }

    let (score, line_no, line) = best?;
    if score < 0.35 {
        return None;
    }

    let truncated: String = line.chars().take(120).collect();
    Some(format!(
        " Closest matching line is {} (similarity {:.0}%): `{}`",
        line_no,
        score * 100.0,
        truncated
    ))
}

/// Cheap trigram-overlap similarity between two char sequences (0.0..=1.0).
fn trigram_similarity(a: &[char], b: &[char]) -> f64 {
    if a.len() < 3 || b.len() < 3 {
        return 0.0;
    }

    let trigrams = |s: &[char]| -> std::collections::HashMap<[char; 3], usize> {
        let mut m = std::collections::HashMap::new();
        for w in s.windows(3) {
            *m.entry([w[0], w[1], w[2]]).or_insert(0usize) += 1;
        }
        m
    };

    let (ta, tb) = (trigrams(a), trigrams(b));
    let common: usize = ta
        .iter()
        .map(|(g, ca)| std::cmp::min(*ca, *tb.get(g).unwrap_or(&0)))
        .sum();
    let total: usize = ta.values().sum::<usize>().max(tb.values().sum::<usize>());
    if total == 0 {
        0.0
    } else {
        common as f64 / total as f64
    }
}

/// Check if an input string looks like a line-anchored hashline patch.
/// True if it starts with `[` or contains hashline operators (`PUT`, `CUT`).
pub fn is_hashline_input(input: &str) -> bool {
    let trimmed = input.trim_start();
    trimmed.starts_with('[')
        || contains_recognizable_hashline_operations(input)
        || input.lines().any(|l| {
            let t = l.trim_start();
            t.starts_with("PUT") || t.starts_with("CUT")
        })
}

/// Apply line-anchored edits from a hashline patch to one or more files.
pub async fn apply_hashline_patch(
    patch_text: &str,
    default_path: Option<&str>,
    cwd: &Path,
) -> anyhow::Result<String> {
    let patch = Patch::parse(
        patch_text,
        &SplitOptions {
            cwd: Some(cwd),
            path: default_path,
        },
    )
    .map_err(|e| anyhow::anyhow!("Failed to parse hashline patch: {e}"))?;

    if patch.sections.is_empty() {
        anyhow::bail!("No hashline sections found in input.");
    }

    let mut sections = Vec::with_capacity(patch.sections.len());
    for section in &patch.sections {
        let parsed = section
            .parse()
            .map_err(|e| anyhow::anyhow!("Failed to parse patch for '{}': {e}", section.path))?;
        sections.push((section.path.clone(), parsed.edits.clone()));
    }
    drop(patch);

    let mut results = Vec::new();

    for (section_path, edits) in sections {
        let full_path = resolve_path(&section_path, cwd);

        if !full_path.exists() {
            anyhow::bail!("File not found: '{}'", full_path.display());
        }

        if full_path.is_dir() {
            anyhow::bail!("Path is a directory, not a file: '{}'", full_path.display());
        }

        let current_content = tokio::fs::read_to_string(&full_path)
            .await
            .map_err(|e| anyhow::anyhow!("Failed to read file '{}': {e}", full_path.display()))?;

        let mut block_resolutions = Vec::new();
        let mut resolve_warnings = Vec::new();
        let resolved_edits = if has_block_edit(&edits) {
            resolve_block_edits(
                &edits,
                &current_content,
                &section_path,
                Unresolved::Throw,
                &mut |item| block_resolutions.push(item),
                &mut |warning| resolve_warnings.push(warning),
            )
            .map_err(|e| anyhow::anyhow!("Block resolution error in '{}': {e}", section_path))?
        } else {
            edits
        };

        let apply_result = apply_edits(
            &current_content,
            &resolved_edits,
            ApplyOptions {
                clipboard: None,
                path: Some(&section_path),
                on_empty_paste: EmptyPaste::Throw,
            },
        )
        .map_err(|e| {
            anyhow::anyhow!("Failed to apply hashline edits to '{}': {e}", section_path)
        })?;

        let updated_content = apply_result.text;

        if updated_content == current_content {
            results.push(format!(
                "No changes made to '{}' (edits produced identical content).",
                section_path
            ));
            continue;
        }

        atomic_write(&full_path, updated_content.as_bytes())
            .await
            .map_err(|e| anyhow::anyhow!("Failed to update file '{}': {e}", full_path.display()))?;

        let stats = compute_diff_stats(&current_content, &updated_content);
        let unified_diff =
            generate_unified_diff(&current_content, &updated_content, &section_path, 3);

        if unified_diff.trim().is_empty() {
            results.push(format!(
                "File '{}' updated successfully (no line differences).",
                section_path
            ));
        } else {
            results.push(format!(
                "Successfully edited '{}' (+{} -{} lines):\n\n```diff\n{}```",
                section_path, stats.additions, stats.deletions, unified_diff
            ));
        }
    }

    Ok(results.join("\n\n"))
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
        "Edit an existing file by replacing an exact, unique occurrence of old_text with new_text, or by applying line-anchored hashline patches."
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
                },
                "input": {
                    "type": "string",
                    "description": "Line-anchored hashline patch input (starting with [ or containing PUT/CUT)."
                }
            }
        })
    }

    async fn execute(&self, args: Value, ctx: &ToolContext) -> anyhow::Result<String> {
        let path_opt = args
            .get("path")
            .and_then(|v| v.as_str())
            .or_else(|| args.get("file_path").and_then(|v| v.as_str()))
            .or_else(|| args.get("file").and_then(|v| v.as_str()));

        // Check if hashline patch is provided via "input", "patch", "edits", or "content"
        let hashline_candidate = args
            .get("input")
            .and_then(|v| v.as_str())
            .or_else(|| args.get("patch").and_then(|v| v.as_str()))
            .or_else(|| args.get("edits").and_then(|v| v.as_str()))
            .or_else(|| args.get("content").and_then(|v| v.as_str()));

        if let Some(input_str) = hashline_candidate {
            if is_hashline_input(input_str) {
                return apply_hashline_patch(input_str, path_opt, &ctx.cwd).await;
            }
        }

        // Also check if old_text or new_text contains hashline patch syntax
        let old_text_opt = args
            .get("old_text")
            .and_then(|v| v.as_str())
            .or_else(|| args.get("old_string").and_then(|v| v.as_str()))
            .or_else(|| args.get("old_content").and_then(|v| v.as_str()));

        let new_text_opt = args
            .get("new_text")
            .and_then(|v| v.as_str())
            .or_else(|| args.get("new_string").and_then(|v| v.as_str()))
            .or_else(|| args.get("new_content").and_then(|v| v.as_str()));

        // If old_text looks like hashline patch and new_text is empty or not provided
        if let Some(old) = old_text_opt {
            if is_hashline_input(old) && new_text_opt.map_or(true, |n| n.is_empty()) {
                return apply_hashline_patch(old, path_opt, &ctx.cwd).await;
            }
        }

        // Standard exact search-and-replace mode
        let path_str =
            path_opt.ok_or_else(|| anyhow::anyhow!("Missing required parameter: path"))?;
        let old_text =
            old_text_opt.ok_or_else(|| anyhow::anyhow!("Missing required parameter: old_text"))?;
        let new_text =
            new_text_opt.ok_or_else(|| anyhow::anyhow!("Missing required parameter: new_text"))?;

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

        atomic_write(&full_path, updated_content.as_bytes())
            .await
            .map_err(|e| anyhow::anyhow!("Failed to update file '{}': {e}", full_path.display()))?;

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
                path_str, stats.additions, stats.deletions, unified_diff
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
    fn test_apply_exact_edit_not_found_suggests_closest_line() {
        let content = "fn main() {\n    println!(\"hello\");\n}\n";
        // Near-miss: wrong indentation only.
        let old = "    println!(\"hello\");";
        let content_indented = "fn main() {\n  println!(\"hello\");\n}\n";

        let err = apply_exact_edit(content_indented, old, "x", "t.rs").unwrap_err();
        assert!(
            err.to_string().contains("Closest matching line is 2"),
            "missing closest-line hint: {err}"
        );
        // Sanity: the unmodified content still errors plainly.
        let err2 = apply_exact_edit(content, "totally absent text!!", "x", "t.rs").unwrap_err();
        assert!(err2.to_string().contains("old_text not found in 't.rs'"));
    }

    #[test]
    fn test_trigram_similarity_bounds() {
        let a: Vec<char> = "hello world".chars().collect();
        let identical: Vec<char> = "hello world".chars().collect();
        let unrelated: Vec<char> = "zzz".chars().collect();

        assert_eq!(trigram_similarity(&a, &identical), 1.0);
        assert_eq!(trigram_similarity(&a, &unrelated), 0.0);
        // Too-short inputs score 0.
        let tiny: Vec<char> = "ab".chars().collect();
        assert_eq!(trigram_similarity(&tiny, &a), 0.0);
    }

    #[tokio::test]
    async fn test_edit_tool_uses_atomic_write() {
        let temp_dir =
            std::env::temp_dir().join(format!("fusion_edit_atomic_test_{}", uuid::Uuid::new_v4()));
        tokio::fs::create_dir_all(&temp_dir).await.unwrap();

        let file_path = temp_dir.join("atomic.txt");
        tokio::fs::write(&file_path, "alpha\nbeta\ngamma\n")
            .await
            .unwrap();

        let tool = EditFileTool::new();
        let ctx = ToolContext {
            cwd: temp_dir.clone(),
            env: std::collections::HashMap::new(),
        };

        let args = json!({
            "path": "atomic.txt",
            "old_text": "beta",
            "new_text": "BETA"
        });

        tool.execute(args, &ctx).await.unwrap();
        let updated = tokio::fs::read_to_string(&file_path).await.unwrap();
        assert_eq!(updated, "alpha\nBETA\ngamma\n");

        // No .tmp.* siblings leaked next to the file.
        let siblings: Vec<_> = std::fs::read_dir(&temp_dir)
            .unwrap()
            .filter_map(|e| e.ok())
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|n| n.contains(".tmp."))
            .collect();
        assert!(siblings.is_empty(), "temp files leaked: {siblings:?}");

        let _ = tokio::fs::remove_dir_all(&temp_dir).await;
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
        let temp_dir =
            std::env::temp_dir().join(format!("fusion_edit_test_{}", uuid::Uuid::new_v4()));
        tokio::fs::create_dir_all(&temp_dir).await.unwrap();

        let file_path = temp_dir.join("test.txt");
        tokio::fs::write(&file_path, "apple\nbanana\ncherry\n")
            .await
            .unwrap();

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

    #[tokio::test]
    async fn test_hashline_edit_execution() {
        let temp_dir =
            std::env::temp_dir().join(format!("fusion_edit_hl_test_{}", uuid::Uuid::new_v4()));
        tokio::fs::create_dir_all(&temp_dir).await.unwrap();

        let file_path = temp_dir.join("sample.rs");
        tokio::fs::write(&file_path, "fn main() {\n    println!(\"hello\");\n}\n")
            .await
            .unwrap();

        let tool = EditFileTool::new();
        let ctx = ToolContext {
            cwd: temp_dir.clone(),
            env: std::collections::HashMap::new(),
        };

        let args = json!({
            "input": "[sample.rs]\nPUT 2.=2:\n+    println!(\"world\");"
        });

        let output = tool.execute(args, &ctx).await.unwrap();
        assert!(output.contains("Successfully edited 'sample.rs'"));
        assert!(output.contains("+    println!(\"world\");"));

        let updated = tokio::fs::read_to_string(&file_path).await.unwrap();
        assert_eq!(updated, "fn main() {\n    println!(\"world\");\n}\n");

        let _ = tokio::fs::remove_dir_all(&temp_dir).await;
    }

    #[tokio::test]
    async fn test_hashline_headerless_with_path() {
        let temp_dir =
            std::env::temp_dir().join(format!("fusion_edit_hl_hl_test_{}", uuid::Uuid::new_v4()));
        tokio::fs::create_dir_all(&temp_dir).await.unwrap();

        let file_path = temp_dir.join("data.txt");
        tokio::fs::write(&file_path, "line 1\nline 2\nline 3\n")
            .await
            .unwrap();

        let tool = EditFileTool::new();
        let ctx = ToolContext {
            cwd: temp_dir.clone(),
            env: std::collections::HashMap::new(),
        };

        let args = json!({
            "path": "data.txt",
            "input": "PUT 2.=2:\n+line 2 updated"
        });

        let output = tool.execute(args, &ctx).await.unwrap();
        assert!(output.contains("Successfully edited 'data.txt'"));

        let updated = tokio::fs::read_to_string(&file_path).await.unwrap();
        assert_eq!(updated, "line 1\nline 2 updated\nline 3\n");

        let _ = tokio::fs::remove_dir_all(&temp_dir).await;
    }

    #[tokio::test]
    async fn test_hashline_cut_and_put() {
        let temp_dir =
            std::env::temp_dir().join(format!("fusion_edit_hl_cut_{}", uuid::Uuid::new_v4()));
        tokio::fs::create_dir_all(&temp_dir).await.unwrap();

        let file_path = temp_dir.join("cut_test.txt");
        tokio::fs::write(&file_path, "keep 1\ndelete me\nkeep 2\n")
            .await
            .unwrap();

        let tool = EditFileTool::new();
        let ctx = ToolContext {
            cwd: temp_dir.clone(),
            env: std::collections::HashMap::new(),
        };

        let args = json!({
            "input": "[cut_test.txt]\nCUT 2.=2"
        });

        let output = tool.execute(args, &ctx).await.unwrap();
        assert!(output.contains("Successfully edited 'cut_test.txt'"));

        let updated = tokio::fs::read_to_string(&file_path).await.unwrap();
        assert_eq!(updated, "keep 1\nkeep 2\n");

        let _ = tokio::fs::remove_dir_all(&temp_dir).await;
    }
}
