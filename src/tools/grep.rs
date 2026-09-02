use async_trait::async_trait;
use serde_json::{json, Value};

use crate::tools::grep_filter::{FilterableGrepEngine, GrepOptions};
use crate::tools::types::{Tool, ToolContext};
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
        "Recursively search file contents for a regular expression pattern, respecting .gitignore, skipping binary files, and supporting advanced path include/exclude glob filters."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "pattern": {
                    "type": "string",
                    "description": "Regular expression pattern or text to search for."
                },
                "path": {
                    "type": "string",
                    "description": "Directory or file path to search within (optional, defaults to workspace root)."
                },
                "include": {
                    "description": "Glob pattern or array of glob patterns to include (e.g. '*.rs', ['src/**/*.ts', 'tests/**/*.ts']).",
                    "oneOf": [
                        { "type": "string" },
                        { "type": "array", "items": { "type": "string" } }
                    ]
                },
                "exclude": {
                    "description": "Glob pattern or array of glob patterns to exclude (e.g. 'target/**', ['*.min.js', 'vendor/**']).",
                    "oneOf": [
                        { "type": "string" },
                        { "type": "array", "items": { "type": "string" } }
                    ]
                },
                "type": {
                    "description": "File type shortcut or array of types to filter (e.g. 'rust', 'python', 'typescript', 'json', 'toml').",
                    "oneOf": [
                        { "type": "string" },
                        { "type": "array", "items": { "type": "string" } }
                    ]
                },
                "case_sensitive": {
                    "type": "boolean",
                    "description": "Whether the search is case-sensitive (optional, default: true)."
                },
                "hidden": {
                    "type": "boolean",
                    "description": "Whether to include hidden files in search (optional, default: false)."
                },
                "max_results": {
                    "type": "integer",
                    "description": "Maximum number of matching lines to return (optional, default: 200)."
                },
                "context_before": {
                    "type": "integer",
                    "description": "Number of lines of context before each match (optional, default: 0)."
                },
                "context_after": {
                    "type": "integer",
                    "description": "Number of lines of context after each match (optional, default: 0)."
                },
                "context": {
                    "type": "integer",
                    "description": "Number of lines of context before and after each match (optional, default: 0)."
                },
                "invert_match": {
                    "type": "boolean",
                    "description": "Invert match: select non-matching lines (optional, default: false)."
                },
                "fixed_strings": {
                    "type": "boolean",
                    "description": "Treat pattern as a literal fixed string instead of a regular expression (optional, default: false)."
                },
                "max_depth": {
                    "type": "integer",
                    "description": "Maximum directory traversal depth (optional)."
                },
                "max_file_size": {
                    "type": "integer",
                    "description": "Maximum file size in bytes to search (optional, default: 20MB)."
                },
                "files_with_matches": {
                    "type": "boolean",
                    "description": "Only output names of files containing matches (optional, default: false)."
                },
                "count_only": {
                    "type": "boolean",
                    "description": "Only output total match count (optional, default: false)."
                }
            },
            "required": ["pattern"]
        })
    }

    async fn execute(&self, args: Value, ctx: &ToolContext) -> anyhow::Result<String> {
        let options = GrepOptions::from_json(&args, &ctx.cwd)?;
        let search_path_display = options.search_path.display().to_string();
        let pattern = options.pattern.clone();
        let options_clone = options.clone();

        // Execute search on threadpool
        let result = tokio::task::spawn_blocking(move || -> anyhow::Result<String> {
            let engine = FilterableGrepEngine::new(options_clone.clone())?;
            let search_res = engine.search()?;
            Ok(search_res.format_output(&search_path_display, &pattern, &options_clone))
        })
        .await
        .map_err(|e| anyhow::anyhow!("Grep task execution failed: {e}"))??;

        Ok(result)
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

    #[tokio::test]
    async fn test_grep_include_glob_filter() {
        let temp = TestDir::new("include_glob");
        temp.write_file("src/app.rs", b"let target = 42;\n");
        temp.write_file("src/style.css", b"/* target */\n");
        temp.write_file("src/util.ts", b"const target = 100;\n");

        let tool = GrepTool::new();
        let ctx = ToolContext {
            cwd: temp.path.clone(),
            env: std::collections::HashMap::new(),
        };

        let res = tool
            .execute(
                json!({
                    "pattern": "target",
                    "include": "*.rs",
                }),
                &ctx,
            )
            .await
            .unwrap();

        assert!(res.contains("src/app.rs:1: let target = 42;"));
        assert!(!res.contains("src/style.css"));
        assert!(!res.contains("src/util.ts"));

        // Array of includes
        let res_multi = tool
            .execute(
                json!({
                    "pattern": "target",
                    "include": ["*.rs", "*.ts"],
                }),
                &ctx,
            )
            .await
            .unwrap();

        assert!(res_multi.contains("src/app.rs"));
        assert!(res_multi.contains("src/util.ts"));
        assert!(!res_multi.contains("src/style.css"));
    }

    #[tokio::test]
    async fn test_grep_exclude_glob_filter() {
        let temp = TestDir::new("exclude_glob");
        temp.write_file("src/app.rs", b"const MATCH_VAR: i32 = 1;\n");
        temp.write_file("tests/app_test.rs", b"assert_eq!(MATCH_VAR, 1);\n");
        temp.write_file("vendor/lib.rs", b"const MATCH_VAR: i32 = 2;\n");

        let tool = GrepTool::new();
        let ctx = ToolContext {
            cwd: temp.path.clone(),
            env: std::collections::HashMap::new(),
        };

        let res = tool
            .execute(
                json!({
                    "pattern": "MATCH_VAR",
                    "exclude": ["tests/**", "vendor/**"],
                }),
                &ctx,
            )
            .await
            .unwrap();

        assert!(res.contains("src/app.rs:1: const MATCH_VAR: i32 = 1;"));
        assert!(!res.contains("tests/app_test.rs"));
        assert!(!res.contains("vendor/lib.rs"));
    }

    #[tokio::test]
    async fn test_grep_file_type_filter() {
        let temp = TestDir::new("file_type");
        temp.write_file("src/lib.rs", b"pub fn match_func() {}\n");
        temp.write_file("scripts/run.py", b"def match_func(): pass\n");
        temp.write_file("config.json", b"{\"match_func\": true}\n");

        let tool = GrepTool::new();
        let ctx = ToolContext {
            cwd: temp.path.clone(),
            env: std::collections::HashMap::new(),
        };

        let res_rust = tool
            .execute(
                json!({
                    "pattern": "match_func",
                    "type": "rust",
                }),
                &ctx,
            )
            .await
            .unwrap();

        assert!(res_rust.contains("src/lib.rs"));
        assert!(!res_rust.contains("scripts/run.py"));
        assert!(!res_rust.contains("config.json"));
    }

    #[tokio::test]
    async fn test_grep_context_lines() {
        let temp = TestDir::new("context");
        temp.write_file("src/main.rs", b"line 1\nline 2\nKEYWORD_LINE\nline 4\nline 5\n");

        let tool = GrepTool::new();
        let ctx = ToolContext {
            cwd: temp.path.clone(),
            env: std::collections::HashMap::new(),
        };

        let res = tool
            .execute(
                json!({
                    "pattern": "KEYWORD_LINE",
                    "context_before": 1,
                    "context_after": 1,
                }),
                &ctx,
            )
            .await
            .unwrap();

        assert!(res.contains("src/main.rs-2: line 2"));
        assert!(res.contains("src/main.rs:3: KEYWORD_LINE"));
        assert!(res.contains("src/main.rs-4: line 4"));
    }

    #[tokio::test]
    async fn test_grep_invert_match() {
        let temp = TestDir::new("invert");
        temp.write_file("data.txt", b"skip\nkeep 1\nskip\nkeep 2\n");

        let tool = GrepTool::new();
        let ctx = ToolContext {
            cwd: temp.path.clone(),
            env: std::collections::HashMap::new(),
        };

        let res = tool
            .execute(
                json!({
                    "pattern": "skip",
                    "invert_match": true,
                }),
                &ctx,
            )
            .await
            .unwrap();

        assert!(res.contains("data.txt:2: keep 1"));
        assert!(res.contains("data.txt:4: keep 2"));
        assert!(!res.contains("skip"));
    }

    #[tokio::test]
    async fn test_grep_fixed_strings() {
        let temp = TestDir::new("fixed");
        temp.write_file("regex.txt", b"foo(bar)\nfoo.*bar\n");

        let tool = GrepTool::new();
        let ctx = ToolContext {
            cwd: temp.path.clone(),
            env: std::collections::HashMap::new(),
        };

        let res = tool
            .execute(
                json!({
                    "pattern": "foo(bar)",
                    "fixed_strings": true,
                }),
                &ctx,
            )
            .await
            .unwrap();

        assert!(res.contains("regex.txt:1: foo(bar)"));
        assert_eq!(res.lines().count(), 1);
    }

    #[tokio::test]
    async fn test_grep_files_with_matches() {
        let temp = TestDir::new("files_only");
        temp.write_file("a.txt", b"FINDME here\n");
        temp.write_file("b.txt", b"FINDME also here\n");
        temp.write_file("c.txt", b"nothing here\n");

        let tool = GrepTool::new();
        let ctx = ToolContext {
            cwd: temp.path.clone(),
            env: std::collections::HashMap::new(),
        };

        let res = tool
            .execute(
                json!({
                    "pattern": "FINDME",
                    "files_with_matches": true,
                }),
                &ctx,
            )
            .await
            .unwrap();

        assert!(res.contains("a.txt"));
        assert!(res.contains("b.txt"));
        assert!(!res.contains("c.txt"));
        assert!(!res.contains("FINDME")); // Only filenames
    }
}
