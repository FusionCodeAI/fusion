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
                },
                "depth": {
                    "type": "integer",
                    "description": "Maximum directory depth to descend (optional, 0 or omitted = unlimited)."
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

        let depth = match args.get("depth") {
            Some(v) => match v.as_u64() {
                Some(d) if d <= u16::MAX as u64 => Some(d as usize),
                Some(_) => anyhow::bail!("depth must be at most {}", u16::MAX),
                None => anyhow::bail!("depth must be a non-negative integer"),
            },
            None => None,
        };

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
                // Honor .gitignore even outside a git repository (e.g. extracted
                // archives, CI workspaces) — matches the behavior of ripgrep's
                // default file-type filtering in editors.
                .require_git(false)
                .git_ignore(true)
                .git_global(true)
                .git_exclude(true)
                .parents(true);

            if let Some(max_depth) = depth {
                builder.max_depth(Some(max_depth));
            }

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

                // Never descend into .git internals, even when hidden files are
                // included — the ignore crate only auto-filters .git when the
                // walk is inside a real repository.
                if path
                    .strip_prefix(&target_path)
                    .unwrap_or(path)
                    .components()
                    .any(|c| c.as_os_str() == ".git")
                {
                    tracing::debug!("Glob walk skipping .git path: {}", path.display());
                    continue;
                }

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

    #[tokio::test]
    async fn test_glob_tool_missing_pattern() {
        let temp = TempDir::new();
        let ctx = ToolContext {
            cwd: temp.path().to_path_buf(),
            env: std::collections::HashMap::new(),
        };

        let tool = GlobTool::new();
        let res = tool.execute(json!({}), &ctx).await;
        assert!(res.is_err());
        let err = format!("{}", res.unwrap_err());
        assert!(err.contains("pattern"), "unexpected error: {err}");
    }

    #[tokio::test]
    async fn test_glob_tool_invalid_pattern() {
        let temp = TempDir::new();
        let ctx = ToolContext {
            cwd: temp.path().to_path_buf(),
            env: std::collections::HashMap::new(),
        };

        // An unclosed character class is rejected by globset.
        let tool = GlobTool::new();
        let res = tool.execute(json!({ "pattern": "[abc" }), &ctx).await;
        assert!(res.is_err());
        let err = format!("{}", res.unwrap_err());
        assert!(err.contains("Invalid glob pattern"), "unexpected error: {err}");
    }

    #[tokio::test]
    async fn test_glob_tool_gitignored_files_excluded() {
        let temp = TempDir::new();
        let ctx = ToolContext {
            cwd: temp.path().to_path_buf(),
            env: std::collections::HashMap::new(),
        };

        fs::write(temp.path().join("Cargo.toml"), "[package]\n").unwrap();
        fs::write(temp.path().join("debug.bin"), "\0\0\0").unwrap();
        let target = temp.path().join("target");
        fs::create_dir_all(&target).unwrap();
        fs::write(target.join("out.o"), "\0\0\0").unwrap();
        fs::write(
            temp.path().join(".gitignore"),
            "target/\n*.bin\n",
        )
        .unwrap();

        let tool = GlobTool::new();
        let res = tool
            .execute(json!({ "pattern": "**/*" }), &ctx)
            .await
            .unwrap();

        assert!(res.contains("Cargo.toml"));
        // .gitignore entries are honored even outside a real git repository.
        assert!(!res.contains("target/"), "gitignored dir leaked: {res}");
        assert!(!res.contains("debug.bin"), "gitignored file leaked: {res}");
    }

    #[tokio::test]
    async fn test_glob_tool_hidden_files_excluded_by_default() {
        let temp = TempDir::new();
        let ctx = ToolContext {
            cwd: temp.path().to_path_buf(),
            env: std::collections::HashMap::new(),
        };

        fs::write(temp.path().join("visible.txt"), "v\n").unwrap();
        fs::write(temp.path().join(".hidden.txt"), "h\n").unwrap();
        let nested = temp.path().join(".config");
        fs::create_dir_all(&nested).unwrap();
        fs::write(nested.join("settings.conf"), "s\n").unwrap();

        let tool = GlobTool::new();

        // Default: hidden files and anything beneath hidden directories is skipped.
        let res = tool
            .execute(json!({ "pattern": "**/*" }), &ctx)
            .await
            .unwrap();
        assert!(res.contains("visible.txt"));
        assert!(!res.contains(".hidden.txt"), "hidden file leaked: {res}");
        assert!(!res.contains("settings.conf"), "nested hidden file leaked: {res}");

        // With hidden: true, dotfiles appear but .git internals never do.
        let git_dir = temp.path().join(".git");
        fs::create_dir_all(git_dir.join("objects")).unwrap();
        fs::write(git_dir.join("HEAD"), "ref: refs/heads/main\n").unwrap();
        fs::write(git_dir.join("objects").join("abc123"), "\0").unwrap();

        let res = tool
            .execute(json!({ "pattern": "**/*", "hidden": true }), &ctx)
            .await
            .unwrap();
        assert!(res.contains(".hidden.txt"));
        assert!(res.contains(".config/"));
        assert!(!res.contains(".git/"), ".git internals leaked: {res}");
        assert!(!res.contains("HEAD"), ".git contents leaked: {res}");
    }

    #[tokio::test]
    async fn test_glob_tool_depth_limit() {
        let temp = TempDir::new();
        let ctx = ToolContext {
            cwd: temp.path().to_path_buf(),
            env: std::collections::HashMap::new(),
        };

        let deep = temp.path().join("a").join("b").join("c");
        fs::create_dir_all(&deep).unwrap();
        fs::write(temp.path().join("top.txt"), "t\n").unwrap();
        fs::write(temp.path().join("a").join("mid.txt"), "m\n").unwrap();
        fs::write(deep.join("deep.txt"), "d\n").unwrap();

        let tool = GlobTool::new();

        // depth 2 covers the search root (1) + a/mid.txt (2).
        let res = tool
            .execute(json!({ "pattern": "**/*.txt", "depth": 2 }), &ctx)
            .await
            .unwrap();
        assert!(res.contains("top.txt"));
        assert!(res.contains("a/mid.txt"));
        assert!(!res.contains("deep.txt"), "depth limit ignored: {res}");

        // Without a limit, everything is found.
        let res = tool
            .execute(json!({ "pattern": "**/*.txt" }), &ctx)
            .await
            .unwrap();
        assert!(res.contains("top.txt"));
        assert!(res.contains("a/mid.txt"));
        assert!(res.contains("deep.txt"));
    }

    #[tokio::test]
    async fn test_glob_tool_depth_validation() {
        let temp = TempDir::new();
        let ctx = ToolContext {
            cwd: temp.path().to_path_buf(),
            env: std::collections::HashMap::new(),
        };

        let tool = GlobTool::new();

        // Negative / non-integer depth is rejected rather than silently ignored.
        let res = tool
            .execute(json!({ "pattern": "*.txt", "depth": -1 }), &ctx)
            .await;
        assert!(res.is_err());

        let res = tool
            .execute(json!({ "pattern": "*.txt", "depth": "2" }), &ctx)
            .await;
        assert!(res.is_err());
    }

    #[tokio::test]
    async fn test_glob_tool_case_insensitive() {
        let temp = TempDir::new();
        let ctx = ToolContext {
            cwd: temp.path().to_path_buf(),
            env: std::collections::HashMap::new(),
        };

        fs::write(temp.path().join("README.MD"), "# x\n").unwrap();

        let tool = GlobTool::new();

        // Case-sensitive (default): *.md does not match README.MD.
        let res = tool
            .execute(json!({ "pattern": "*.md" }), &ctx)
            .await
            .unwrap();
        assert!(res.contains("No files found"));

        // case_sensitive: false matches regardless of case.
        let res = tool
            .execute(json!({ "pattern": "*.md", "case_sensitive": false }), &ctx)
            .await
            .unwrap();
        assert!(res.contains("README.MD"), "case-insensitive match failed: {res}");
    }

    #[tokio::test]
    async fn test_glob_tool_scoped_path() {
        let temp = TempDir::new();
        let ctx = ToolContext {
            cwd: temp.path().to_path_buf(),
            env: std::collections::HashMap::new(),
        };

        let src = temp.path().join("src");
        let docs = temp.path().join("docs");
        fs::create_dir_all(&src).unwrap();
        fs::create_dir_all(&docs).unwrap();
        fs::write(src.join("main.rs"), "fn main() {}\n").unwrap();
        fs::write(docs.join("guide.md"), "# guide\n").unwrap();

        let tool = GlobTool::new();

        // Search restricted to docs/: src/ contents must not leak in.
        let res = tool
            .execute(
                json!({ "pattern": "**/*", "path": docs.to_string_lossy() }),
                &ctx,
            )
            .await
            .unwrap();
        assert!(res.contains("guide.md"));
        assert!(!res.contains("main.rs"), "path scoping ignored: {res}");
    }

    #[tokio::test]
    async fn test_glob_tool_nonexistent_path() {
        let temp = TempDir::new();
        let ctx = ToolContext {
            cwd: temp.path().to_path_buf(),
            env: std::collections::HashMap::new(),
        };

        let tool = GlobTool::new();
        let res = tool
            .execute(json!({ "pattern": "*.txt", "path": "does/not/exist" }), &ctx)
            .await;
        assert!(res.is_err());
    }

    #[tokio::test]
    async fn test_glob_tool_output_truncation() {
        let temp = TempDir::new();
        let ctx = ToolContext {
            cwd: temp.path().to_path_buf(),
            env: std::collections::HashMap::new(),
        };

        for i in 0..550 {
            fs::write(temp.path().join(format!("file_{i:04}.txt")), "x\n").unwrap();
        }

        let tool = GlobTool::new();
        let res = tool
            .execute(json!({ "pattern": "file_*.txt" }), &ctx)
            .await
            .unwrap();

        assert!(res.contains("additional files truncated"), "no truncation: {res}");
        let lines: Vec<&str> = res.lines().collect();
        assert_eq!(lines.len(), 501); // 500 results + 1 truncation notice
    }

    #[tokio::test]
    async fn test_glob_tool_directories_flagged_with_slash() {
        let temp = TempDir::new();
        let ctx = ToolContext {
            cwd: temp.path().to_path_buf(),
            env: std::collections::HashMap::new(),
        };

        let sub = temp.path().join("subdir");
        fs::create_dir_all(&sub).unwrap();

        let tool = GlobTool::new();
        let res = tool
            .execute(json!({ "pattern": "subdir" }), &ctx)
            .await
            .unwrap();

        assert_eq!(res, "subdir/");
    }
}
