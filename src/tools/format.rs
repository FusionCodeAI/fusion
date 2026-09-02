use async_trait::async_trait;
use serde_json::{json, Value};
use std::path::Path;
use std::process::Stdio;
use std::time::Duration;

use crate::tools::file::resolve_path;
use crate::tools::types::{Tool, ToolContext};

/// Formatter configuration: binary name, how to pass the file path, and whether the
/// formatter writes in-place (true) or emits formatted text to stdout (false).
struct FormatterSpec {
    /// Executable name looked up via PATH.
    binary: &'static str,
    /// Arguments that come *before* the file path. For in-place formatters these
    /// typically include the in-place flag; for stdout formatters just the mode flag.
    args_before: &'static [&'static str],
    /// Whether the formatter rewrites the file in place.
    /// `false` → formatter writes to stdout and we capture + overwrite the file.
    in_place: bool,
}

/// Map a file extension to its formatter spec.
fn formatter_for_ext(ext: &str) -> Option<FormatterSpec> {
    match ext {
        // Rust
        "rs" => Some(FormatterSpec {
            binary: "rustfmt",
            args_before: &["--edition", "2021"],
            in_place: true,
        }),
        // JavaScript / TypeScript / JSX / TSX / JSON / CSS / HTML / Markdown / YAML
        "js" | "jsx" | "ts" | "tsx" | "json" | "css" | "html" | "md" | "yaml" | "yml" => {
            Some(FormatterSpec {
                binary: "prettier",
                args_before: &["--write"],
                in_place: true,
            })
        }
        // Python
        "py" => Some(FormatterSpec {
            binary: "black",
            args_before: &[],
            in_place: true,
        }),
        // Go
        "go" => Some(FormatterSpec {
            binary: "gofmt",
            // gofmt -w writes in place
            args_before: &["-w"],
            in_place: true,
        }),
        // C / C++ / Objective-C / CUDA
        "c" | "cc" | "cpp" | "cxx" | "h" | "hh" | "hpp" | "hxx" | "m" | "mm" | "cu" => {
            Some(FormatterSpec {
                binary: "clang-format",
                args_before: &["-i"],
                in_place: true,
            })
        }
        _ => None,
    }
}

/// Resolve the absolute path of a binary on PATH, returning `None` if not found.
fn which(binary: &str) -> Option<std::path::PathBuf> {
    std::env::var_os("PATH").and_then(|path_var| {
        std::env::split_paths(&path_var).find_map(|dir| {
            let candidate = dir.join(binary);
            if candidate.is_file() {
                Some(candidate)
            } else {
                // On Windows executables need the .exe suffix
                #[cfg(windows)]
                {
                    let exe = dir.join(format!("{binary}.exe"));
                    if exe.is_file() {
                        return Some(exe);
                    }
                }
                None
            }
        })
    })
}

/// Auto-format tool: detects the language from the file extension and runs the
/// appropriate formatter (rustfmt, prettier, black, gofmt, clang-format) via a
/// child process with a configurable timeout.
#[derive(Default, Debug, Clone)]
pub struct AutoFormatTool;

impl AutoFormatTool {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl Tool for AutoFormatTool {
    fn name(&self) -> &str {
        "format_file"
    }

    fn description(&self) -> &str {
        "Auto-format a source file using the appropriate formatter for its language. \
         Supports Rust (rustfmt), JavaScript/TypeScript/JSON/CSS/HTML/Markdown/YAML (prettier), \
         Python (black), Go (gofmt), and C/C++ (clang-format). \
         Returns a summary of the formatter output."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "Path to the file to format (relative to workspace or absolute)."
                },
                "timeout_secs": {
                    "type": "integer",
                    "description": "Formatter timeout in seconds (default: 30)."
                }
            },
            "required": ["path"]
        })
    }

    async fn execute(&self, args: Value, ctx: &ToolContext) -> anyhow::Result<String> {
        // --- resolve path ---
        let path_str = args
            .get("path")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("Missing required parameter: path"))?;

        if path_str.trim().is_empty() {
            anyhow::bail!("path cannot be empty");
        }

        let abs_path = resolve_path(path_str, &ctx.cwd);

        if !abs_path.exists() {
            anyhow::bail!("File does not exist: {}", abs_path.display());
        }
        if !abs_path.is_file() {
            anyhow::bail!("Path is not a file: {}", abs_path.display());
        }

        // --- detect language ---
        let ext = abs_path
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_lowercase();

        let spec = formatter_for_ext(&ext).ok_or_else(|| {
            anyhow::anyhow!(
                "Unsupported language: no formatter configured for '.{}' files. \
                 Supported extensions: rs, js, jsx, ts, tsx, json, css, html, md, yaml, yml, py, go, \
                 c, cc, cpp, cxx, h, hh, hpp, hxx, m, mm, cu",
                ext
            )
        })?;

        // --- locate binary ---
        let binary_path = which(spec.binary).ok_or_else(|| {
            anyhow::anyhow!(
                "Formatter '{}' not found on PATH. Please install it and ensure it is on your PATH.",
                spec.binary
            )
        })?;

        let timeout_secs = args
            .get("timeout_secs")
            .and_then(|v| v.as_u64())
            .unwrap_or(30)
            .max(1);

        // --- build command ---
        let mut cmd = tokio::process::Command::new(&binary_path);
        for arg in spec.args_before {
            cmd.arg(arg);
        }
        cmd.arg(&abs_path);

        // Inherit a clean environment; use the file's directory as cwd
        if let Some(parent) = abs_path.parent() {
            cmd.current_dir(parent);
        } else {
            cmd.current_dir(&ctx.cwd);
        }

        // Sanitize env: strip secrets
        let cleaner = crate::tools::env_cleaner::EnvCleaner::default();
        cleaner.apply_to_tokio_command(&mut cmd, Some(&ctx.env));

        cmd.stdin(Stdio::null());

        if spec.in_place {
            // Formatter modifies the file; capture stdout/stderr for diagnostics only
            cmd.stdout(Stdio::piped());
            cmd.stderr(Stdio::piped());
            cmd.kill_on_drop(true);

            let child = cmd.spawn().map_err(|e| {
                anyhow::anyhow!("Failed to spawn formatter '{}': {e}", spec.binary)
            })?;

            let output =
                match tokio::time::timeout(Duration::from_secs(timeout_secs), child.wait_with_output()).await {
                    Ok(Ok(out)) => out,
                    Ok(Err(e)) => {
                        anyhow::bail!("Formatter '{}' I/O error: {e}", spec.binary);
                    }
                    Err(_) => {
                        anyhow::bail!(
                            "Formatter '{}' timed out after {} second{}",
                            spec.binary,
                            timeout_secs,
                            if timeout_secs == 1 { "" } else { "s" }
                        );
                    }
                };

            if !output.status.success() {
                let stderr = String::from_utf8_lossy(&output.stderr);
                let stdout = String::from_utf8_lossy(&output.stdout);
                let detail = [stderr.trim(), stdout.trim()]
                    .iter()
                    .filter(|s| !s.is_empty())
                    .cloned()
                    .collect::<Vec<_>>()
                    .join("\n");
                if detail.is_empty() {
                    anyhow::bail!(
                        "Formatter '{}' exited with code {:?}",
                        spec.binary,
                        output.status.code()
                    );
                } else {
                    anyhow::bail!(
                        "Formatter '{}' failed (exit {:?}):\n{}",
                        spec.binary,
                        output.status.code(),
                        detail
                    );
                }
            }

            let stderr_msg = String::from_utf8_lossy(&output.stderr).trim().to_string();
            let note = if stderr_msg.is_empty() {
                String::new()
            } else {
                format!("\n{stderr_msg}")
            };

            Ok(format!(
                "Formatted '{}' with {}.{note}",
                Path::new(path_str).display(),
                spec.binary
            ))
        } else {
            // Formatter writes to stdout; capture and write back
            cmd.stdout(Stdio::piped());
            cmd.stderr(Stdio::piped());
            cmd.kill_on_drop(true);

            let child = cmd.spawn().map_err(|e| {
                anyhow::anyhow!("Failed to spawn formatter '{}': {e}", spec.binary)
            })?;

            let output =
                match tokio::time::timeout(Duration::from_secs(timeout_secs), child.wait_with_output()).await {
                    Ok(Ok(out)) => out,
                    Ok(Err(e)) => {
                        anyhow::bail!("Formatter '{}' I/O error: {e}", spec.binary);
                    }
                    Err(_) => {
                        anyhow::bail!(
                            "Formatter '{}' timed out after {} second{}",
                            spec.binary,
                            timeout_secs,
                            if timeout_secs == 1 { "" } else { "s" }
                        );
                    }
                };

            if !output.status.success() {
                let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
                anyhow::bail!(
                    "Formatter '{}' failed (exit {:?}):\n{}",
                    spec.binary,
                    output.status.code(),
                    stderr
                );
            }

            tokio::fs::write(&abs_path, &output.stdout)
                .await
                .map_err(|e| anyhow::anyhow!("Failed to write formatted output to '{}': {e}", abs_path.display()))?;

            Ok(format!(
                "Formatted '{}' with {} (stdout → file).",
                Path::new(path_str).display(),
                spec.binary
            ))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::NamedTempFile;

    fn ctx() -> ToolContext {
        ToolContext {
            cwd: std::env::temp_dir(),
            env: std::env::vars().collect(),
        }
    }

    #[test]
    fn unsupported_extension_returns_error() {
        assert!(formatter_for_ext("xyz").is_none());
        assert!(formatter_for_ext("").is_none());
    }

    #[test]
    fn known_extensions_map_correctly() {
        assert_eq!(formatter_for_ext("rs").map(|s| s.binary), Some("rustfmt"));
        assert_eq!(formatter_for_ext("py").map(|s| s.binary), Some("black"));
        assert_eq!(formatter_for_ext("go").map(|s| s.binary), Some("gofmt"));
        assert_eq!(formatter_for_ext("ts").map(|s| s.binary), Some("prettier"));
        assert_eq!(formatter_for_ext("cpp").map(|s| s.binary), Some("clang-format"));
        assert_eq!(formatter_for_ext("c").map(|s| s.binary), Some("clang-format"));
    }

    #[tokio::test]
    async fn missing_path_param_errors() {
        let tool = AutoFormatTool::new();
        let err = tool.execute(json!({}), &ctx()).await.unwrap_err();
        assert!(err.to_string().contains("Missing required parameter: path"));
    }

    #[tokio::test]
    async fn nonexistent_file_errors() {
        let tool = AutoFormatTool::new();
        let err = tool
            .execute(json!({"path": "/nonexistent/no_such_file.rs"}), &ctx())
            .await
            .unwrap_err();
        assert!(err.to_string().contains("does not exist"));
    }

    #[tokio::test]
    async fn unsupported_lang_errors_with_helpful_message() {
        let tool = AutoFormatTool::new();
        let mut f = NamedTempFile::with_suffix(".xyz").unwrap();
        writeln!(f, "hello").unwrap();
        let path = f.path().to_string_lossy().to_string();
        let err = tool
            .execute(json!({"path": path}), &ctx())
            .await
            .unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("Unsupported language"), "got: {msg}");
        assert!(msg.contains(".xyz"), "got: {msg}");
    }

    #[tokio::test]
    async fn rustfmt_path_verified() {
        // Ensure rustfmt is locatable on PATH in the dev environment
        let path = which("rustfmt");
        assert!(
            path.is_some(),
            "rustfmt not found on PATH; install via `rustup component add rustfmt`"
        );
    }

    #[tokio::test]
    async fn format_rust_file_roundtrip() {
        // Only run if rustfmt is available
        let Some(_) = which("rustfmt") else { return };

        let tool = AutoFormatTool::new();
        let mut f = NamedTempFile::with_suffix(".rs").unwrap();
        // Poorly formatted Rust
        writeln!(f, "fn main(){{println!(\"hi\");}}").unwrap();
        let path = f.path().to_string_lossy().to_string();

        let result = tool.execute(json!({"path": path}), &ctx()).await;
        assert!(result.is_ok(), "rustfmt failed: {:?}", result);
        let msg = result.unwrap();
        assert!(msg.contains("rustfmt"), "unexpected message: {msg}");

        // File should now have been reformatted (contain a newline after fn main)
        let contents = std::fs::read_to_string(f.path()).unwrap();
        assert!(contents.contains('\n'));
    }
}
