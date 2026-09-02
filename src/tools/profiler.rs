//! Profiler Tool — flamegraph-based CPU profiling for Rust binaries.
//!
//! Wraps `cargo flamegraph` (or falls back to `perf record` + `perf script` on Linux)
//! to profile a target binary, writing the resulting SVG flamegraph to a temporary file
//! and returning its absolute path.
//!
//! # Usage
//! ```json
//! { "binary": "my_binary", "args": ["--release"], "duration_secs": 10, "cwd": "/path/to/project" }
//! ```
//!
//! # Requirements
//! - `cargo flamegraph` must be installed (`cargo install flamegraph`) **or**
//! - On Linux: `perf` must be available in PATH as a fallback.
//!
//! # Parameters
//! - `binary` (required): The binary name to profile (used as `cargo flamegraph --bin <binary>`).
//! - `args`: Extra arguments forwarded after `--` to the profiled binary.
//! - `cargo_args`: Extra flags passed to `cargo flamegraph` itself (e.g. `["--release"]`).
//! - `duration_secs`: How long to let the binary run before stopping profiling (default: 10s).
//! - `output`: Override for the SVG output path (default: a temp file).
//! - `cwd`: Working directory for the cargo project.
//! - `mode`: `"flamegraph"` (default) or `"perf"` to force a backend.

use async_trait::async_trait;
use serde_json::{json, Value};
use std::path::PathBuf;
use std::time::Duration;

use crate::tools::file::resolve_path;
use crate::tools::types::{Tool, ToolContext};

// ---------------------------------------------------------------------------
// ProfilerTool
// ---------------------------------------------------------------------------

/// CPU profiler tool that runs a binary under `cargo flamegraph` (or `perf`)
/// and returns the path to the generated SVG flamegraph.
#[derive(Debug, Clone, Default)]
pub struct ProfilerTool;

impl ProfilerTool {
    pub fn new() -> Self {
        Self
    }

    /// Detect whether `cargo flamegraph` is available.
    async fn has_flamegraph() -> bool {
        tokio::process::Command::new("cargo")
            .args(["flamegraph", "--version"])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .await
            .map(|s| s.success())
            .unwrap_or(false)
    }

    /// Detect whether `perf` is available (Linux fallback).
    async fn has_perf() -> bool {
        tokio::process::Command::new("perf")
            .arg("--version")
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .await
            .map(|s| s.success())
            .unwrap_or(false)
    }

    /// Run profiling using `cargo flamegraph`.
    async fn run_flamegraph(
        binary: &str,
        cargo_args: &[String],
        bin_args: &[String],
        svg_path: &PathBuf,
        duration_secs: u64,
        cwd: &PathBuf,
    ) -> anyhow::Result<()> {
        let mut cmd = tokio::process::Command::new("cargo");
        cmd.arg("flamegraph");

        // Output path
        cmd.args(["--output", svg_path.to_str().unwrap_or("flamegraph.svg")]);

        // User-supplied cargo-flamegraph flags (e.g. --release, --features foo)
        cmd.args(cargo_args);

        // Target binary
        cmd.args(["--bin", binary]);

        // Forward binary arguments after --
        if !bin_args.is_empty() {
            cmd.arg("--");
            cmd.args(bin_args);
        }

        cmd.current_dir(cwd);
        cmd.stdout(std::process::Stdio::piped());
        cmd.stderr(std::process::Stdio::piped());

        let timeout = Duration::from_secs(duration_secs + 30); // buffer for compile time

        let child = cmd
            .spawn()
            .map_err(|e| anyhow::anyhow!("Failed to spawn cargo flamegraph: {e}"))?;

        let result = tokio::time::timeout(timeout, child.wait_with_output())
            .await
            .map_err(|_| {
                anyhow::anyhow!("cargo flamegraph timed out after {}s", timeout.as_secs())
            })?
            .map_err(|e| anyhow::anyhow!("cargo flamegraph I/O error: {e}"))?;

        if !result.status.success() {
            let stderr = String::from_utf8_lossy(&result.stderr);
            let stdout = String::from_utf8_lossy(&result.stdout);
            let combined = format!("{stdout}{stderr}");
            anyhow::bail!(
                "cargo flamegraph failed (exit {:?}):\n{}",
                result.status.code(),
                combined.trim()
            );
        }

        Ok(())
    }

    /// Run profiling using `perf record` + `flamegraph.pl` (Linux fallback).
    /// Requires `perf` and `flamegraph.pl` (from the Brendan Gregg toolkit) in PATH.
    async fn run_perf_flamegraph(
        binary: &str,
        bin_args: &[String],
        svg_path: &PathBuf,
        duration_secs: u64,
        cwd: &PathBuf,
    ) -> anyhow::Result<()> {
        // Locate the binary (try target/release first, then target/debug, then PATH)
        let bin_path = Self::locate_binary(binary, cwd)?;

        let perf_data = cwd.join("perf.data");
        let folded_path = cwd.join("perf.folded");

        // Step 1: perf record
        {
            let mut cmd = tokio::process::Command::new("perf");
            cmd.args([
                "record",
                "--call-graph",
                "dwarf",
                "-g",
                "-o",
                perf_data.to_str().unwrap_or("perf.data"),
                "--",
                bin_path.to_str().unwrap_or(binary),
            ]);
            cmd.args(bin_args);
            cmd.current_dir(cwd);
            cmd.stdout(std::process::Stdio::null());
            cmd.stderr(std::process::Stdio::null());

            let timeout = Duration::from_secs(duration_secs + 10);

            tokio::time::timeout(timeout, cmd.status())
                .await
                .map_err(|_| anyhow::anyhow!("perf record timed out"))?
                .map_err(|e| anyhow::anyhow!("perf record I/O error: {e}"))?;
            // perf record may exit non-zero when the tracee exits normally; ignore exit code.
        }

        // Step 2: perf script -> folded stacks
        {
            let script_out = tokio::process::Command::new("perf")
                .args([
                    "script",
                    "-i",
                    perf_data.to_str().unwrap_or("perf.data"),
                ])
                .current_dir(cwd)
                .stdout(std::process::Stdio::piped())
                .stderr(std::process::Stdio::null())
                .output()
                .await
                .map_err(|e| anyhow::anyhow!("perf script failed: {e}"))?;

            // pipe through stackcollapse-perf.pl or inferno-collapse-perf
            let folded = Self::collapse_stacks(&script_out.stdout, cwd).await?;
            tokio::fs::write(&folded_path, &folded)
                .await
                .map_err(|e| anyhow::anyhow!("Failed to write folded stacks: {e}"))?;
        }

        // Step 3: flamegraph -> SVG
        {
            let folded_bytes = tokio::fs::read(&folded_path)
                .await
                .map_err(|e| anyhow::anyhow!("Failed to read folded stacks: {e}"))?;

            let svg_bytes = Self::render_flamegraph_svg(&folded_bytes).await?;
            tokio::fs::write(svg_path, svg_bytes)
                .await
                .map_err(|e| anyhow::anyhow!("Failed to write SVG: {e}"))?;
        }

        // Cleanup temp files
        let _ = tokio::fs::remove_file(&perf_data).await;
        let _ = tokio::fs::remove_file(&folded_path).await;

        Ok(())
    }

    /// Locate a binary in target/release, target/debug, or PATH.
    fn locate_binary(name: &str, cwd: &PathBuf) -> anyhow::Result<PathBuf> {
        let candidates = [
            cwd.join("target/release").join(name),
            cwd.join("target/debug").join(name),
            PathBuf::from(name),
        ];
        for c in &candidates {
            if c.exists() {
                return Ok(c.clone());
            }
        }
        // Check PATH
        which_in_path(name).ok_or_else(|| {
            anyhow::anyhow!(
                "Binary '{name}' not found in target/release, target/debug, or PATH. \
                 Build it first."
            )
        })
    }

    /// Collapse stacks using `inferno-collapse-perf` or `stackcollapse-perf.pl`.
    async fn collapse_stacks(perf_script_output: &[u8], cwd: &PathBuf) -> anyhow::Result<Vec<u8>> {
        // Try inferno-collapse-perf first (installable via cargo)
        if which_in_path("inferno-collapse-perf").is_some() {
            let mut child = tokio::process::Command::new("inferno-collapse-perf")
                .current_dir(cwd)
                .stdin(std::process::Stdio::piped())
                .stdout(std::process::Stdio::piped())
                .stderr(std::process::Stdio::null())
                .spawn()
                .map_err(|e| anyhow::anyhow!("inferno-collapse-perf spawn failed: {e}"))?;

            if let Some(stdin) = child.stdin.take() {
                use tokio::io::AsyncWriteExt;
                let mut stdin = stdin;
                stdin.write_all(perf_script_output).await?;
                // drop closes stdin, signalling EOF
            }

            let out = child
                .wait_with_output()
                .await
                .map_err(|e| anyhow::anyhow!("inferno-collapse-perf I/O: {e}"))?;
            return Ok(out.stdout);
        }

        // Try stackcollapse-perf.pl (FlameGraph repo)
        if which_in_path("stackcollapse-perf.pl").is_some() {
            let mut child = tokio::process::Command::new("stackcollapse-perf.pl")
                .current_dir(cwd)
                .stdin(std::process::Stdio::piped())
                .stdout(std::process::Stdio::piped())
                .stderr(std::process::Stdio::null())
                .spawn()
                .map_err(|e| anyhow::anyhow!("stackcollapse-perf.pl spawn failed: {e}"))?;

            if let Some(stdin) = child.stdin.take() {
                use tokio::io::AsyncWriteExt;
                let mut stdin = stdin;
                stdin.write_all(perf_script_output).await?;
            }

            let out = child
                .wait_with_output()
                .await
                .map_err(|e| anyhow::anyhow!("stackcollapse-perf.pl I/O: {e}"))?;
            return Ok(out.stdout);
        }

        anyhow::bail!(
            "No stack collapser found. Install inferno: `cargo install inferno`, \
             or put stackcollapse-perf.pl (from FlameGraph) in PATH."
        );
    }

    /// Render folded stacks into an SVG using `inferno-flamegraph` or `flamegraph.pl`.
    async fn render_flamegraph_svg(folded: &[u8]) -> anyhow::Result<Vec<u8>> {
        if which_in_path("inferno-flamegraph").is_some() {
            let mut child = tokio::process::Command::new("inferno-flamegraph")
                .stdin(std::process::Stdio::piped())
                .stdout(std::process::Stdio::piped())
                .stderr(std::process::Stdio::null())
                .spawn()
                .map_err(|e| anyhow::anyhow!("inferno-flamegraph spawn failed: {e}"))?;

            if let Some(stdin) = child.stdin.take() {
                use tokio::io::AsyncWriteExt;
                let mut stdin = stdin;
                stdin.write_all(folded).await?;
            }

            let out = child
                .wait_with_output()
                .await
                .map_err(|e| anyhow::anyhow!("inferno-flamegraph I/O: {e}"))?;
            return Ok(out.stdout);
        }

        if which_in_path("flamegraph.pl").is_some() {
            let mut child = tokio::process::Command::new("flamegraph.pl")
                .stdin(std::process::Stdio::piped())
                .stdout(std::process::Stdio::piped())
                .stderr(std::process::Stdio::null())
                .spawn()
                .map_err(|e| anyhow::anyhow!("flamegraph.pl spawn failed: {e}"))?;

            if let Some(stdin) = child.stdin.take() {
                use tokio::io::AsyncWriteExt;
                let mut stdin = stdin;
                stdin.write_all(folded).await?;
            }

            let out = child
                .wait_with_output()
                .await
                .map_err(|e| anyhow::anyhow!("flamegraph.pl I/O: {e}"))?;
            return Ok(out.stdout);
        }

        anyhow::bail!(
            "No flamegraph renderer found. Install inferno: `cargo install inferno`, \
             or put flamegraph.pl (from FlameGraph) in PATH."
        );
    }
}

// ---------------------------------------------------------------------------
// Tool impl
// ---------------------------------------------------------------------------

#[async_trait]
impl Tool for ProfilerTool {
    fn name(&self) -> &str {
        "profile"
    }

    fn description(&self) -> &str {
        "Profile a Rust binary using cargo flamegraph (or perf on Linux), \
         producing an interactive SVG flamegraph. Returns the absolute path \
         to the generated SVG file. Requires `cargo install flamegraph` or \
         `perf` + `inferno` to be available."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "binary": {
                    "type": "string",
                    "description": "Name of the binary to profile (passed to --bin). Required."
                },
                "args": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "Arguments forwarded to the profiled binary after '--'."
                },
                "cargo_args": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "Extra flags for cargo flamegraph itself, e.g. ['--release']."
                },
                "duration_secs": {
                    "type": "integer",
                    "description": "Seconds to let the binary run before stopping (default: 10). Add extra time for long workloads."
                },
                "output": {
                    "type": "string",
                    "description": "Override the SVG output path. Defaults to a system temp file."
                },
                "cwd": {
                    "type": "string",
                    "description": "Working directory containing the Cargo.toml (default: tool context cwd)."
                },
                "mode": {
                    "type": "string",
                    "enum": ["flamegraph", "perf"],
                    "description": "Profiling backend: 'flamegraph' uses cargo-flamegraph (default); 'perf' forces the perf+inferno fallback."
                }
            },
            "required": ["binary"]
        })
    }

    async fn execute(&self, args: Value, ctx: &ToolContext) -> anyhow::Result<String> {
        // --- required params ---
        let binary = args
            .get("binary")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("Missing required parameter: binary"))?;

        if binary.trim().is_empty() {
            anyhow::bail!("Parameter 'binary' must not be empty");
        }

        // --- optional params ---
        let bin_args: Vec<String> = args
            .get("args")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(str::to_owned))
                    .collect()
            })
            .unwrap_or_default();

        let cargo_args: Vec<String> = args
            .get("cargo_args")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(str::to_owned))
                    .collect()
            })
            .unwrap_or_default();

        let duration_secs = args
            .get("duration_secs")
            .and_then(|v| v.as_u64())
            .unwrap_or(10);

        let mode = args
            .get("mode")
            .and_then(|v| v.as_str())
            .unwrap_or("flamegraph");

        // --- resolve cwd ---
        let cwd = if let Some(cwd_str) = args.get("cwd").and_then(|v| v.as_str()) {
            resolve_path(cwd_str, &ctx.cwd)
        } else {
            ctx.cwd.clone()
        };

        // --- resolve svg output path ---
        let svg_path: PathBuf = if let Some(out) = args.get("output").and_then(|v| v.as_str()) {
            resolve_path(out, &ctx.cwd)
        } else {
            // Use std::env::temp_dir for a deterministic temp location
            let tmp = std::env::temp_dir();
            let filename = format!(
                "fusion-flamegraph-{}-{}.svg",
                binary,
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_secs()
            );
            tmp.join(filename)
        };

        // --- choose backend ---
        let use_flamegraph = match mode {
            "perf" => false,
            "flamegraph" => true,
            _ => {
                // Auto-detect: prefer cargo flamegraph, fall back to perf
                Self::has_flamegraph().await
            }
        };

        if use_flamegraph {
            if !Self::has_flamegraph().await {
                anyhow::bail!(
                    "cargo flamegraph is not installed. Run: cargo install flamegraph\n\
                     Alternatively set mode='perf' and ensure `perf` + `inferno` are in PATH."
                );
            }
            Self::run_flamegraph(
                binary,
                &cargo_args,
                &bin_args,
                &svg_path,
                duration_secs,
                &cwd,
            )
            .await?;
        } else {
            if !Self::has_perf().await {
                anyhow::bail!(
                    "Neither cargo flamegraph nor perf is available.\n\
                     Install flamegraph: cargo install flamegraph\n\
                     Install perf: apt install linux-perf (Linux only)"
                );
            }
            Self::run_perf_flamegraph(binary, &bin_args, &svg_path, duration_secs, &cwd).await?;
        }

        // Confirm the file was actually written
        if !svg_path.exists() {
            anyhow::bail!(
                "Profiling completed but SVG was not written to: {}",
                svg_path.display()
            );
        }

        let size = std::fs::metadata(&svg_path)
            .map(|m| m.len())
            .unwrap_or(0);

        Ok(format!(
            "Flamegraph saved to: {}\nSize: {} bytes\nOpen the SVG in a browser for an interactive flamegraph.",
            svg_path.display(),
            size
        ))
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Check whether a program name is available in the system PATH.
fn which_in_path(name: &str) -> Option<PathBuf> {
    std::env::var_os("PATH").and_then(|path_var| {
        std::env::split_paths(&path_var).find_map(|dir| {
            let candidate = dir.join(name);
            if candidate.is_file() {
                Some(candidate)
            } else {
                None
            }
        })
    })
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn make_ctx() -> ToolContext {
        ToolContext {
            cwd: std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")),
            env: Default::default(),
        }
    }

    #[test]
    fn name_is_profile() {
        assert_eq!(ProfilerTool::new().name(), "profile");
    }

    #[test]
    fn parameters_requires_binary() {
        let params = ProfilerTool::new().parameters();
        let required = params.get("required").and_then(|v| v.as_array()).unwrap();
        assert!(required.iter().any(|v| v.as_str() == Some("binary")));
    }

    #[tokio::test]
    async fn missing_binary_errors() {
        let tool = ProfilerTool::new();
        let ctx = make_ctx();
        let err = tool.execute(json!({}), &ctx).await.unwrap_err();
        assert!(err.to_string().contains("binary"));
    }

    #[tokio::test]
    async fn empty_binary_errors() {
        let tool = ProfilerTool::new();
        let ctx = make_ctx();
        let err = tool.execute(json!({ "binary": "" }), &ctx).await.unwrap_err();
        assert!(err.to_string().contains("empty"));
    }

    #[test]
    fn which_in_path_finds_real_binary() {
        // `sh` is virtually always in PATH on Unix
        #[cfg(unix)]
        assert!(which_in_path("sh").is_some());
    }

    #[test]
    fn which_in_path_misses_nonexistent() {
        assert!(which_in_path("__fusion_nonexistent_tool_xyz__").is_none());
    }
}
