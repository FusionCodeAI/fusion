use async_trait::async_trait;
use globset::GlobBuilder;
use ignore::WalkBuilder;
use serde_json::{json, Value};

use crate::tools::file::resolve_path;
use crate::tools::types::{Tool, ToolContext};

/// Tool for recursively finding files matching wildcard/glob patterns.
#[derive(Default, Debug, Clone)]
pub struct GlobTool;

impl GlobTool {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl Tool for GlobTool {
    fn name(&self) -> &str {
        "glob"
    }

    fn description(&self) -> &str {
        "Recursively match files and directories using a glob pattern, respecting .gitignore."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "pattern": {
                    "type": "string",
                    "description": "Glob pattern to match against (e.g. '**/*.rs', 'src/**/*.ts', '*.json', 'Cargo.*')."
                },
                "path": {
                    "type": "string",
                    "description": "Root directory to search within (optional, defaults to workspace root)."
                },
                "hidden": {
                    "type": "boolean",
                    "description": "Whether to include hidden files (dotfiles) in search (optional, default: false)."
                },
                "case_sensitive": {
                    "type": "boolean",
                    "description": "Whether pattern matching is case-sensitive (optional, default: true)."
                }
            },
            "required": ["pattern"]
        })
    }

    async fn execute(&self, args: Value, ctx: &ToolContext) -> anyhow::Result<String> {
        let pattern = args
            .get("pattern")
            .and_then(|v| v.as_str())
            .or_else(|| args.get("glob").and_then(|v| v.as_str()))
            .ok_or_else(|| anyhow::anyhow!("Missing required parameter: pattern"))?;

        if pattern.is_empty() {
            anyhow::bail!("Glob pattern cannot be empty");
        }

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

        let hidden = args
            .get("hidden")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        let case_sensitive = args
            .get("case_sensitive")
            .and_then(|v| v.as_bool())
            .unwrap_or(true);

        let glob_matcher = GlobBuilder::new(pattern)
            .case_insensitive(!case_sensitive)
            .literal_separator(false)
            .build()
            .map_err(|e| anyhow::anyhow!("Invalid glob pattern '{}': {e}", pattern))?
            .compile_matcher();

        let strict_matcher = GlobBuilder::new(pattern)
            .case_insensitive(!case_sensitive)
            .literal_separator(true)
            .build()
            .ok()
            .map(|g| g.compile_matcher());

        let cwd = ctx.cwd.clone();
        let target_path = search_path.clone();

        let matched = tokio::task::spawn_blocking(move || -> anyhow::Result<(Vec<String>, usize)> {
            let mut builder = WalkBuilder::new(&target_path);
            builder
                .hidden(!hidden)
                .git_ignore(true)
                .git_global(true)
                .git_exclude(true);

            let mut results = Vec::new();
            let mut total_count = 0;
            let max_results = 500;

            for result in builder.build() {
                let entry = match result {
                    Ok(entry) => entry,
                    Err(e) => {
                        tracing::debug!("Glob walk error: {e}");
                        continue;
                    }
                };

                // Skip the search root itself
                if entry.depth() == 0 {
                    continue;
                }

                let path = entry.path();
                let rel_path = path.strip_prefix(&cwd).unwrap_or(path);
                let rel_target = path.strip_prefix(&target_path).unwrap_or(path);
                let file_name = path.file_name().unwrap_or_default();

                // Normalize paths for consistent cross-platform matching (replace '\' with '/')
                let rel_path_str = rel_path.to_string_lossy().replace('\\', "/");
                let rel_target_str = rel_target.to_string_lossy().replace('\\', "/");
                let full_path_str = path.to_string_lossy().replace('\\', "/");
                let file_name_str = file_name.to_string_lossy();

                let is_match = glob_matcher.is_match(&rel_path_str)
                    || glob_matcher.is_match(&rel_target_str)
                    || glob_matcher.is_match(&full_path_str)
                    || glob_matcher.is_match(&*file_name_str)
                    || strict_matcher.as_ref().map_or(false, |m| {
                        m.is_match(&rel_path_str)
                            || m.is_match(&rel_target_str)
                            || m.is_match(&full_path_str)
                    });

                if is_match {
                    total_count += 1;
                    if results.len() < max_results {
                        let mut s = rel_path_str.clone();
                        if entry.file_type().map(|ft| ft.is_dir()).unwrap_or(false) && !s.ends_with('/') {
                            s.push('/');
                        }
                        results.push(s);
                    }
                }
            }

            results.sort();
            Ok((results, total_count))
        })
        .await
        .map_err(|e| anyhow::anyhow!("Glob task failed: {e}"))??;

        let (mut results, total_count) = matched;

        if results.is_empty() {
            return Ok(format!(
                "No files found matching pattern '{}' in '{}'",
                pattern,
                search_path.display()
            ));
        }

        if total_count > results.len() {
            results.push(format!(
                "\n... [{} additional files truncated]",
                total_count - results.len()
            ));
        }

        Ok(results.join("\n"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile_helper::TempDir;

    mod tempfile_helper {
        use std::path::{Path, PathBuf};
        use uuid::Uuid;

        pub struct TempDir {
            path: PathBuf,
        }

        impl TempDir {
            pub fn new() -> Self {
                let path = std::env::temp_dir().join(format!("fusion_glob_test_{}", Uuid::new_v4()));
                std::fs::create_dir_all(&path).expect("failed to create temp dir");
                Self { path }
            }

            pub fn path(&self) -> &Path {
                &self.path
            }
        }

        impl Drop for TempDir {
            fn drop(&mut self) {
                let _ = std::fs::remove_dir_all(&self.path);
            }
        }
    }

    #[tokio::test]
    async fn test_glob_tool_basic() {
        let temp = TempDir::new();
        let ctx = ToolContext {
            cwd: temp.path().to_path_buf(),
            env: std::collections::HashMap::new(),
        };

        fs::write(temp.path().join("a.rs"), "fn a() {}\n").unwrap();
        fs::write(temp.path().join("b.rs"), "fn b() {}\n").unwrap();
        fs::write(temp.path().join("c.txt"), "hello\n").unwrap();

        let tool = GlobTool::new();
        let res = tool
            .execute(json!({ "pattern": "*.rs" }), &ctx)
            .await
            .unwrap();

        assert!(res.contains("a.rs"));
        assert!(res.contains("b.rs"));
        assert!(!res.contains("c.txt"));

        // Check sorted output
        let lines: Vec<&str> = res.lines().collect();
        assert_eq!(lines, vec!["a.rs", "b.rs"]);
    }

    #[tokio::test]
    async fn test_glob_tool_nested() {
        let temp = TempDir::new();
        let ctx = ToolContext {
            cwd: temp.path().to_path_buf(),
            env: std::collections::HashMap::new(),
        };

        let sub = temp.path().join("src").join("tools");
        fs::create_dir_all(&sub).unwrap();
        fs::write(sub.join("glob.rs"), "// glob\n").unwrap();
        fs::write(sub.join("grep.rs"), "// grep\n").unwrap();

        let tool = GlobTool::new();
        let res = tool
            .execute(json!({ "pattern": "src/**/*.rs" }), &ctx)
            .await
            .unwrap();

        assert!(res.contains("src/tools/glob.rs"));
        assert!(res.contains("src/tools/grep.rs"));
    }

    #[tokio::test]
    async fn test_glob_tool_no_matches() {
        let temp = TempDir::new();
        let ctx = ToolContext {
            cwd: temp.path().to_path_buf(),
            env: std::collections::HashMap::new(),
        };

        let tool = GlobTool::new();
        let res = tool
            .execute(json!({ "pattern": "*.xyz" }), &ctx)
            .await
            .unwrap();

        assert!(res.contains("No files found"));
    }

    #[tokio::test]
    async fn test_glob_tool_empty_pattern() {
        let temp = TempDir::new();
        let ctx = ToolContext {
            cwd: temp.path().to_path_buf(),
            env: std::collections::HashMap::new(),
        };

        let tool = GlobTool::new();
        let res = tool.execute(json!({ "pattern": "" }), &ctx).await;
        assert!(res.is_err());
    }
}
