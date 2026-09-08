//! Structural syntax-aware code rewriting using tree-sitter AST patterns with metavariables.
//!
//! Provides the `ast_edit` tool, powered by `ast-grep` and `tree-sitter` grammars:
//! - Structural pattern matching using AST nodes rather than raw regex/text.
//! - Metavariables (`$A`, `$NAME`) capture single AST nodes.
//! - Spread metavariables (`$$$ARGS`, `$$$BODY`) capture zero-or-more sequential AST nodes.
//! - Multi-language support (Rust, JavaScript, TypeScript, Python, Go, C, C++, JSON, and more).
//! - Clean integration with `fusion` tool execution pipeline and unified diff generation.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use thiserror::Error;

use fusion_ast::ops::{
    compile_rewrite_rules, resolve_language, resolve_supported_lang, rewrite_source,
    CompiledRewrite,
};
use fusion_ast::SupportLang;

// In integration tests, `fusion::tools` references the library crate.
// In the library itself, `crate::tools` references the internal module hierarchy.
use crate::tools::edit::{compute_diff_stats, generate_unified_diff};
use crate::tools::file::{atomic_write, resolve_path};
use crate::tools::types::{Tool, ToolContext};
/// Errors that can occur during AST pattern compilation or rewriting.
#[derive(Debug, Error)]
pub enum AstEditError {
    #[error("Unsupported language: '{0}'. Supported: rust, javascript, typescript, python, go, c, cpp, json, etc.")]
    UnsupportedLanguage(String),

    #[error("Pattern compilation failed for '{pattern}': {message}")]
    PatternError { pattern: String, message: String },

    #[error("Rewrite failed: {0}")]
    RewriteError(String),

    #[error("File not found: '{0}'")]
    FileNotFound(String),

    #[error("Path is a directory: '{0}'")]
    IsDirectory(String),

    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Invalid argument: {0}")]
    InvalidArgument(String),
}

/// A single rewrite operation pairing a pattern with a replacement template.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RewriteOp {
    /// AST pattern to search for (e.g. `foo($A, $B)`, `console.log($$$)`).
    pub pat: String,
    /// Replacement template using captured metavariables (e.g. `bar($B, $A)`, `console.info($$$)`).
    pub out: String,
}

/// Structural AST code rewrite tool.
#[derive(Debug, Default, Clone)]
pub struct AstEditTool;

impl AstEditTool {
    /// Create a new `AstEditTool` instance.
    pub fn new() -> Self {
        Self
    }

    /// Rewrites `source` code in the specified `lang` using a single `pat` and `template`.
    ///
    /// # Arguments
    /// * `source` - Raw source code string.
    /// * `lang` - Language name or file extension (e.g. "rust", "rs", "js", "ts", "python").
    /// * `pat` - AST pattern with metavariables (`$A`, `$$$ARGS`).
    /// * `template` - Replacement template string with metavariables substituted.
    pub fn apply_rewrite(
        source: &str,
        lang: &str,
        pat: &str,
        template: &str,
    ) -> Result<String, AstEditError> {
        apply_rewrite(source, lang, pat, template)
    }

    /// Rewrites `source` code in the specified `lang` using multiple rewrite operations.
    pub fn apply_rewrites(
        source: &str,
        lang: &str,
        ops: &[RewriteOp],
    ) -> Result<(String, u32), AstEditError> {
        apply_rewrites(source, lang, ops)
    }
}

/// Public function to apply an AST rewrite to `source` code.
///
/// Implements structural syntax-aware code rewriting using tree-sitter AST patterns
/// with metavariables (`$A`, `$$$ARGS`).
pub fn apply_rewrite(
    source: &str,
    lang: &str,
    pat: &str,
    template: &str,
) -> Result<String, AstEditError> {
    let ops = [RewriteOp {
        pat: pat.to_string(),
        out: template.to_string(),
    }];
    let (rewritten, _) = apply_rewrites(source, lang, &ops)?;
    Ok(rewritten)
}

/// Public function to apply multiple AST rewrite operations to `source` code.
pub fn apply_rewrites(
    source: &str,
    lang: &str,
    ops: &[RewriteOp],
) -> Result<(String, u32), AstEditError> {
    let support_lang = resolve_supported_lang(lang)
        .or_else(|_| SupportLang::from_alias(lang).ok_or_else(|| ()))
        .map_err(|_| AstEditError::UnsupportedLanguage(lang.to_string()))?;

    let compiled_ops = compile_ops(ops, support_lang)?;
    execute_ast_rewrite(source, support_lang, &compiled_ops)
}

/// Compiles a slice of `RewriteOp`s for a target `SupportLang`.
fn compile_ops(
    ops: &[RewriteOp],
    language: SupportLang,
) -> Result<Vec<CompiledRewrite>, AstEditError> {
    let raw_rules: Vec<(String, String)> = ops
        .iter()
        .map(|op| (op.pat.clone(), op.out.clone()))
        .collect();

    compile_rewrite_rules(&raw_rules, language).map_err(|(idx, err)| {
        let pattern = ops.get(idx).map_or_else(String::new, |o| o.pat.clone());
        AstEditError::PatternError {
            pattern,
            message: format!("{err:?}"),
        }
    })
}

/// Execute AST rewrites on a source string, returning the updated string and total replacement count.
fn execute_ast_rewrite(
    source: &str,
    language: SupportLang,
    ops: &[CompiledRewrite],
) -> Result<(String, u32), AstEditError> {
    let (rewritten, count) = rewrite_source(source, language, ops)
        .map_err(|e| AstEditError::RewriteError(e.to_string()))?;
    Ok((rewritten, count))
}

#[async_trait]
impl Tool for AstEditTool {
    fn name(&self) -> &str {
        "ast_edit"
    }

    fn description(&self) -> &str {
        "Structural syntax-aware code rewriting using tree-sitter AST patterns with metavariables ($A, $$$ARGS)."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "paths": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "Files, directories, or globs to rewrite."
                },
                "ops": {
                    "type": "array",
                    "items": {
                        "type": "object",
                        "properties": {
                            "pat": {
                                "type": "string",
                                "description": "AST pattern to match (e.g. '$A + $B' or 'fn $NAME($$$ARGS)')."
                            },
                            "out": {
                                "type": "string",
                                "description": "Replacement template using captured metavariables (e.g. '$B + $A' or 'pub fn $NAME($$$ARGS)')."
                            }
                        },
                        "required": ["pat", "out"]
                    },
                    "description": "Rewrite operations to apply to matching files."
                },
                "lang": {
                    "type": "string",
                    "description": "Optional explicit language override (e.g. 'rust', 'javascript', 'typescript', 'python')."
                }
            },
            "required": ["paths", "ops"]
        })
    }

    async fn execute(&self, args: Value, ctx: &ToolContext) -> anyhow::Result<String> {
        // Extract paths (support "paths" array or "path" single string)
        let mut target_paths: Vec<String> = Vec::new();
        if let Some(paths_arr) = args.get("paths").and_then(|v| v.as_array()) {
            for item in paths_arr {
                if let Some(s) = item.as_str() {
                    target_paths.push(s.to_string());
                }
            }
        } else if let Some(single_path) = args.get("path").and_then(|v| v.as_str()) {
            target_paths.push(single_path.to_string());
        }

        if target_paths.is_empty() {
            anyhow::bail!("Missing required parameter: 'paths' (must be a non-empty array of file paths or globs)");
        }

        // Extract ops
        let ops_val = args
            .get("ops")
            .and_then(|v| v.as_array())
            .ok_or_else(|| anyhow::anyhow!("Missing required parameter: 'ops'"))?;

        if ops_val.is_empty() {
            anyhow::bail!("'ops' parameter cannot be empty");
        }

        let mut ops = Vec::with_capacity(ops_val.len());
        for (i, op_item) in ops_val.iter().enumerate() {
            let pat = op_item
                .get("pat")
                .or_else(|| op_item.get("pattern"))
                .and_then(|v| v.as_str())
                .ok_or_else(|| anyhow::anyhow!("Missing 'pat' in ops[{i}]"))?;

            let out = op_item
                .get("out")
                .or_else(|| op_item.get("template"))
                .or_else(|| op_item.get("replacement"))
                .and_then(|v| v.as_str())
                .ok_or_else(|| anyhow::anyhow!("Missing 'out' in ops[{i}]"))?;

            ops.push(RewriteOp {
                pat: pat.to_string(),
                out: out.to_string(),
            });
        }

        let explicit_lang = args.get("lang").and_then(|v| v.as_str());

        // Resolve files from paths/globs
        let mut files_to_process = Vec::new();
        for target in &target_paths {
            let resolved = resolve_path(target, &ctx.cwd);
            if resolved.is_file() {
                files_to_process.push(resolved);
            } else if resolved.is_dir() {
                // Collect files in directory
                if let Ok(entries) = std::fs::read_dir(&resolved) {
                    for entry in entries.flatten() {
                        let path = entry.path();
                        if path.is_file() {
                            files_to_process.push(path);
                        }
                    }
                }
            } else if fusion_ast::ops::has_glob_syntax(target) {
                if let Ok(matched_files) =
                    fusion_ast::ops::collect_matched_files(&ctx.cwd, &[target.clone()])
                {
                    for mf in matched_files {
                        files_to_process.push(mf.absolute_path);
                    }
                }
            } else if resolved.exists() {
                files_to_process.push(resolved);
            } else {
                // File does not exist
                if target_paths.len() == 1 {
                    anyhow::bail!("File not found: '{}'", resolved.display());
                }
            }
        }

        if files_to_process.is_empty() {
            anyhow::bail!("No files found matching specified paths");
        }

        // Deduplicate files
        files_to_process.sort();
        files_to_process.dedup();

        let mut modified_files = 0;
        let mut total_replacements = 0;
        let mut output_messages = Vec::new();

        for file_path in &files_to_process {
            // Determine language for file
            let lang = match resolve_language(explicit_lang, &file_path) {
                Ok(l) => l,
                Err(e) => {
                    // If multiple files, skip unsupported files gracefully
                    if target_paths.len() > 1 || files_to_process.len() > 1 {
                        continue;
                    } else {
                        anyhow::bail!("{e}");
                    }
                }
            };

            // Read file content
            let content = match tokio::fs::read_to_string(&file_path).await {
                Ok(c) => c,
                Err(e) => {
                    if target_paths.len() == 1 {
                        anyhow::bail!("Failed to read '{}': {e}", file_path.display());
                    } else {
                        continue;
                    }
                }
            };

            // Compile ops for this language
            let compiled = match compile_ops(&ops, lang) {
                Ok(c) => c,
                Err(e) => {
                    // If multi-file, skip language that cannot parse this pattern
                    if target_paths.len() > 1 || files_to_process.len() > 1 {
                        continue;
                    } else {
                        anyhow::bail!("{e}");
                    }
                }
            };

            // Execute rewrite
            match execute_ast_rewrite(&content, lang, &compiled) {
                Ok((updated, count)) => {
                    if count > 0 && updated != content {
                        // Write changes atomically
                        atomic_write(file_path, updated.as_bytes())
                            .await
                            .map_err(|e| {
                                anyhow::anyhow!("Failed to write '{}': {e}", file_path.display())
                            })?;

                        let rel_display = file_path
                            .strip_prefix(&ctx.cwd)
                            .unwrap_or(file_path)
                            .display()
                            .to_string();

                        let diff = generate_unified_diff(&content, &updated, &rel_display, 2);
                        let stats = compute_diff_stats(&content, &updated);

                        output_messages.push(format!(
                            "### {}\nApplied {} replacement(s) (+{} -{} lines):\n\n```diff\n{}\n```",
                            rel_display, count, stats.additions, stats.deletions, diff.trim_end()
                        ));

                        modified_files += 1;
                        total_replacements += count;
                    }
                }
                Err(e) => {
                    if target_paths.len() == 1 && files_to_process.len() == 1 {
                        anyhow::bail!("{e}");
                    }
                }
            }
        }

        if modified_files == 0 {
            Ok("No matching AST patterns found. No files modified.".to_string())
        } else {
            Ok(format!(
                "Successfully applied {} AST replacement(s) across {} file(s):\n\n{}",
                total_replacements,
                modified_files,
                output_messages.join("\n\n")
            ))
        }
    }
}
