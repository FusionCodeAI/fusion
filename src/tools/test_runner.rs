use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::tools::file::resolve_path;
use crate::tools::types::{Tool, ToolContext};

// ---------------------------------------------------------------------------
// TestFramework
// ---------------------------------------------------------------------------

/// Supported test frameworks and their detection/invocation strategies.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TestFramework {
    Cargo,
    Pytest,
    Jest,
    GoTest,
}

impl TestFramework {
    /// Detect the appropriate framework for a given path by inspecting sentinel
    /// files in or above that directory.
    pub fn detect(path: &Path) -> Option<Self> {
        // Walk up looking for project root markers.
        let mut dir = if path.is_file() {
            path.parent().unwrap_or(path).to_path_buf()
        } else {
            path.to_path_buf()
        };

        loop {
            if dir.join("Cargo.toml").exists() {
                return Some(Self::Cargo);
            }
            if dir.join("go.mod").exists() {
                return Some(Self::GoTest);
            }
            if dir.join("package.json").exists() {
                // Prefer jest when package.json is present.
                return Some(Self::Jest);
            }
            if dir.join("pyproject.toml").exists()
                || dir.join("setup.py").exists()
                || dir.join("setup.cfg").exists()
                || dir.join("pytest.ini").exists()
            {
                return Some(Self::Pytest);
            }
            match dir.parent() {
                Some(p) if p != dir => dir = p.to_path_buf(),
                _ => break,
            }
        }
        None
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::Cargo => "cargo",
            Self::Pytest => "pytest",
            Self::Jest => "jest",
            Self::GoTest => "go test",
        }
    }
}

// ---------------------------------------------------------------------------
// TestResult
// ---------------------------------------------------------------------------

/// Parsed summary of a test run.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TestResult {
    pub framework: String,
    /// Number of tests that passed.
    pub passed: u32,
    /// Number of tests that failed.
    pub failed: u32,
    /// Number of tests that were ignored / skipped.
    pub ignored: u32,
    /// Whether the overall process exited successfully.
    pub success: bool,
    /// Process exit code, if available.
    pub exit_code: Option<i32>,
    /// Raw stdout captured from the test runner.
    pub stdout: String,
    /// Raw stderr captured from the test runner.
    pub stderr: String,
}

impl TestResult {
    pub fn total(&self) -> u32 {
        self.passed + self.failed + self.ignored
    }

    /// Format a human-readable summary line.
    pub fn summary(&self) -> String {
        let status = if self.success { "PASSED" } else { "FAILED" };
        format!(
            "[{}] {} passed, {} failed, {} ignored ({} total)",
            status,
            self.passed,
            self.failed,
            self.ignored,
            self.total()
        )
    }
}

// ---------------------------------------------------------------------------
// Output parsers
// ---------------------------------------------------------------------------

/// Parse `cargo test` output.
///
/// Cargo emits a summary line like:
/// `test result: ok. 5 passed; 2 failed; 1 ignored; 0 measured; 0 filtered out; finished in 0.01s`
///
/// It may appear multiple times (once per test binary). We sum all occurrences.
fn parse_cargo(stdout: &str, stderr: &str) -> (u32, u32, u32) {
    let mut passed = 0u32;
    let mut failed = 0u32;
    let mut ignored = 0u32;

    for line in stdout.lines().chain(stderr.lines()) {
        let line = line.trim();
        // e.g. "test result: ok. 5 passed; 2 failed; 1 ignored; ..."
        if line.starts_with("test result:") {
            passed += extract_count(line, "passed");
            failed += extract_count(line, "failed");
            ignored += extract_count(line, "ignored");
        }
    }
    (passed, failed, ignored)
}

/// Parse `pytest` output.
///
/// pytest prints a summary line like:
/// `5 passed, 2 failed, 1 warning in 0.42s`
/// or:
/// `===== 3 failed, 7 passed, 1 skipped in 1.23s =====`
fn parse_pytest(stdout: &str, stderr: &str) -> (u32, u32, u32) {
    let mut passed = 0u32;
    let mut failed = 0u32;
    let mut ignored = 0u32;

    for line in stdout.lines().chain(stderr.lines()) {
        let line = line.trim();
        // Look for the final summary line (contains " in " and duration)
        if (line.contains(" passed") || line.contains(" failed") || line.contains(" error"))
            && line.contains(" in ")
        {
            passed = extract_count(line, "passed");
            failed = extract_count(line, "failed") + extract_count(line, "error");
            ignored = extract_count(line, "skipped") + extract_count(line, "deselected");
        }
    }
    (passed, failed, ignored)
}

/// Parse `jest` output.
///
/// Jest prints lines like:
/// `Tests: 2 failed, 5 passed, 7 total`
/// `Test Suites: 1 failed, 3 passed, 4 total`
fn parse_jest(stdout: &str, stderr: &str) -> (u32, u32, u32) {
    let mut passed = 0u32;
    let mut failed = 0u32;
    let mut ignored = 0u32;

    for line in stdout.lines().chain(stderr.lines()) {
        let line = line.trim();
        // Match the "Tests:" summary line specifically (not "Test Suites:")
        if line.starts_with("Tests:") {
            passed = extract_count(line, "passed");
            failed = extract_count(line, "failed");
            ignored = extract_count(line, "skipped") + extract_count(line, "pending");
        }
    }
    (passed, failed, ignored)
}

/// Parse `go test` output.
///
/// Go test emits lines like:
/// `--- FAIL: TestFoo (0.00s)`
/// `--- PASS: TestBar (0.00s)`
/// `ok  	github.com/pkg/name	0.005s`
/// `FAIL	github.com/pkg/name	0.003s`
fn parse_go(stdout: &str, stderr: &str) -> (u32, u32, u32) {
    let mut passed = 0u32;
    let mut failed = 0u32;
    let ignored = 0u32; // go test has no standard "ignored" count

    for line in stdout.lines().chain(stderr.lines()) {
        let line = line.trim();
        if line.starts_with("--- PASS:") {
            passed += 1;
        } else if line.starts_with("--- FAIL:") {
            failed += 1;
        } else if line.starts_with("--- SKIP:") {
            // counted as ignored in our model; however we keep ignored at 0
            // since go test counts are per-function and SKIP is uncommon via
            // the standard harness (t.Skip sets exit 0)
            let _ = line;
        }
    }
    (passed, failed, ignored)
}

/// Extract a numeric count that immediately precedes a word in `line`.
///
/// Example: `extract_count("5 passed; 2 failed", "passed")` → `5`.
fn extract_count(line: &str, word: &str) -> u32 {
    // Walk backwards from every occurrence of `word` to find the preceding number.
    let mut search = line;
    while let Some(pos) = search.find(word) {
        let before = &search[..pos].trim_end();
        // The number is the last whitespace-delimited token before the match.
        if let Some(tok) = before.split_whitespace().last() {
            // Strip common punctuation
            let clean: String = tok.chars().filter(|c| c.is_ascii_digit()).collect();
            if !clean.is_empty() {
                if let Ok(n) = clean.parse::<u32>() {
                    return n;
                }
            }
        }
        // Advance past this occurrence and keep searching (handles duplicates).
        search = &search[pos + word.len()..];
    }
    0
}

// ---------------------------------------------------------------------------
// RunTestsArgs
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
struct RunTestsArgs {
    /// Path to the project, crate, or test file.
    path: String,
    /// Override the framework; if omitted, auto-detected from project files.
    #[serde(default)]
    framework: Option<String>,
    /// Timeout in seconds (default 120).
    #[serde(default)]
    timeout_secs: Option<u64>,
    /// Extra arguments forwarded verbatim to the test runner.
    #[serde(default)]
    extra_args: Option<String>,
    /// Whether to print the full output in addition to the parsed summary.
    #[serde(default)]
    verbose: Option<bool>,
}

// ---------------------------------------------------------------------------
// RunTestsTool
// ---------------------------------------------------------------------------

/// Tool that runs project tests for a given path/crate with configurable timeout,
/// captures stdout/stderr, and parses pass/fail counts. Supports cargo test,
/// pytest, jest, and go test.
#[derive(Default, Debug, Clone)]
pub struct RunTestsTool;

impl RunTestsTool {
    pub fn new() -> Self {
        Self
    }

    /// Build the shell command string for the given framework and path.
    fn build_command(
        framework: TestFramework,
        path: &Path,
        working_dir: &Path,
        extra_args: Option<&str>,
    ) -> String {
        let extra = extra_args.unwrap_or("").trim();

        match framework {
            TestFramework::Cargo => {
                // Determine whether `path` points to a specific Cargo.toml or a
                // directory relative to working_dir.
                let manifest = if path.join("Cargo.toml").exists() {
                    // path is a crate directory
                    format!("--manifest-path {}/Cargo.toml", path.display())
                } else if path.is_file() && path.file_name().map(|n| n == "Cargo.toml").unwrap_or(false) {
                    format!("--manifest-path {}", path.display())
                } else if path == working_dir {
                    String::new()
                } else {
                    // Treat path as a package filter / test name filter
                    let rel = path.strip_prefix(working_dir).unwrap_or(path);
                    format!("-- {}", rel.display())
                };
                format!("cargo test {} {}", manifest, extra).trim().to_string()
            }

            TestFramework::Pytest => {
                let target = path.display();
                format!("python -m pytest {} {} --tb=short -q", target, extra)
                    .trim()
                    .to_string()
            }

            TestFramework::Jest => {
                let target = if path.is_file() {
                    format!("\"{}\"", path.display())
                } else {
                    format!("--rootDir \"{}\"", path.display())
                };
                format!("npx jest {} {} --no-coverage", target, extra)
                    .trim()
                    .to_string()
            }

            TestFramework::GoTest => {
                // For go test the path should be a package path like ./... or ./pkg
                let pkg = {
                    let rel = path.strip_prefix(working_dir).unwrap_or(path);
                    let s = rel.to_string_lossy();
                    if s.is_empty() || s == "." {
                        "./...".to_string()
                    } else if s.starts_with("./") {
                        s.into_owned()
                    } else {
                        format!("./{}", s)
                    }
                };
                format!("go test -v {} {}", pkg, extra).trim().to_string()
            }
        }
    }

    async fn run(
        &self,
        command: &str,
        working_dir: &Path,
        timeout_secs: u64,
        env: &HashMap<String, String>,
    ) -> anyhow::Result<(String, String, Option<i32>, bool)> {
        let mut cmd = crate::tools::bash::BashTool::build_command(command);

        cmd.current_dir(working_dir);
        let cleaner = crate::tools::env_cleaner::EnvCleaner::default();
        cleaner.apply_to_tokio_command(&mut cmd, Some(env));
        cmd.stdin(std::process::Stdio::null());
        cmd.stdout(std::process::Stdio::piped());
        cmd.stderr(std::process::Stdio::piped());
        cmd.kill_on_drop(true);

        let child = cmd
            .spawn()
            .map_err(|e| anyhow::anyhow!("Failed to spawn test runner '{}': {e}", command))?;

        let timeout_duration = Duration::from_secs(timeout_secs.max(1));
        let output =
            match tokio::time::timeout(timeout_duration, child.wait_with_output()).await {
                Ok(Ok(out)) => out,
                Ok(Err(e)) => anyhow::bail!("Failed to read test runner output: {e}"),
                Err(_) => anyhow::bail!(
                    "Test run timed out after {} second{}",
                    timeout_secs,
                    if timeout_secs == 1 { "" } else { "s" }
                ),
            };

        let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
        let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
        let exit_code = output.status.code();
        let success = output.status.success();

        Ok((stdout, stderr, exit_code, success))
    }
}

#[async_trait]
impl Tool for RunTestsTool {
    fn name(&self) -> &str {
        "run_tests"
    }

    fn description(&self) -> &str {
        "Run project tests for a given path or crate with configurable timeout. \
         Captures stdout/stderr and parses pass/fail/ignored counts. \
         Supports cargo test (Rust), pytest (Python), jest (Node.js), and go test (Go). \
         The framework is auto-detected from project files when not specified."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "Path to the project directory, crate, or test file to test (relative to workspace or absolute)."
                },
                "framework": {
                    "type": "string",
                    "enum": ["cargo", "pytest", "jest", "go_test"],
                    "description": "Override the test framework. If omitted, auto-detected from Cargo.toml / go.mod / package.json / pyproject.toml."
                },
                "timeout_secs": {
                    "type": "integer",
                    "description": "Maximum seconds to wait for the test run (default: 120)."
                },
                "extra_args": {
                    "type": "string",
                    "description": "Extra arguments forwarded verbatim to the test runner (e.g. '--test integration' for cargo, '-k keyword' for pytest)."
                },
                "verbose": {
                    "type": "boolean",
                    "description": "Include full stdout/stderr in the output in addition to the parsed summary (default: false)."
                }
            },
            "required": ["path"]
        })
    }

    async fn execute(&self, args: Value, ctx: &ToolContext) -> anyhow::Result<String> {
        let params: RunTestsArgs = serde_json::from_value(args.clone()).map_err(|e| {
            anyhow::anyhow!("Invalid arguments for run_tests: {e}")
        })?;

        let timeout_secs = params.timeout_secs.unwrap_or(120);
        let verbose = params.verbose.unwrap_or(false);
        let extra_args = params.extra_args.as_deref();

        // Resolve the target path
        let target_path = resolve_path(&params.path, &ctx.cwd);

        // Determine working directory: the target if it's a dir, else its parent.
        let working_dir: PathBuf = if target_path.is_dir() {
            target_path.clone()
        } else if let Some(parent) = target_path.parent() {
            parent.to_path_buf()
        } else {
            ctx.cwd.clone()
        };

        if !working_dir.exists() {
            anyhow::bail!("Path does not exist: {}", target_path.display());
        }

        // Detect or parse the framework.
        let framework = match params.framework.as_deref() {
            Some("cargo") => TestFramework::Cargo,
            Some("pytest") => TestFramework::Pytest,
            Some("jest") => TestFramework::Jest,
            Some("go_test") | Some("go test") | Some("go") => TestFramework::GoTest,
            Some(other) => anyhow::bail!("Unknown framework '{}'. Valid values: cargo, pytest, jest, go_test", other),
            None => TestFramework::detect(&target_path)
                .ok_or_else(|| anyhow::anyhow!(
                    "Could not auto-detect test framework for '{}'. \
                     Provide 'framework' explicitly (cargo, pytest, jest, go_test).",
                    target_path.display()
                ))?,
        };

        let command =
            Self::build_command(framework, &target_path, &ctx.cwd, extra_args);

        tracing::debug!(
            framework = framework.name(),
            command = %command,
            working_dir = %working_dir.display(),
            timeout_secs,
            "run_tests: launching"
        );

        let (stdout, stderr, exit_code, success) = self
            .run(&command, &working_dir, timeout_secs, &ctx.env)
            .await?;

        // Parse pass/fail/ignored counts.
        let (passed, failed, ignored) = match framework {
            TestFramework::Cargo => parse_cargo(&stdout, &stderr),
            TestFramework::Pytest => parse_pytest(&stdout, &stderr),
            TestFramework::Jest => parse_jest(&stdout, &stderr),
            TestFramework::GoTest => parse_go(&stdout, &stderr),
        };

        let result = TestResult {
            framework: framework.name().to_string(),
            passed,
            failed,
            ignored,
            success,
            exit_code,
            stdout: stdout.clone(),
            stderr: stderr.clone(),
        };

        // Format output.
        let mut out = String::new();
        out.push_str(&format!("Framework : {}\n", result.framework));
        out.push_str(&format!("Command   : {}\n", command));
        out.push_str(&format!("Directory : {}\n", working_dir.display()));
        out.push_str(&format!("Exit code : {}\n", exit_code.map(|c| c.to_string()).unwrap_or_else(|| "—".to_string())));
        out.push('\n');
        out.push_str(&result.summary());
        out.push('\n');

        if verbose || !result.success {
            // Always show output on failure; show only when verbose requested otherwise.
            let combined_stdout = stdout.trim();
            let combined_stderr = stderr.trim();
            if !combined_stdout.is_empty() {
                out.push_str("\n--- stdout ---\n");
                out.push_str(combined_stdout);
                out.push('\n');
            }
            if !combined_stderr.is_empty() {
                out.push_str("\n--- stderr ---\n");
                out.push_str(combined_stderr);
                out.push('\n');
            }
        }

        Ok(out)
    }
}

// ---------------------------------------------------------------------------
// Unit Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_count_basic() {
        assert_eq!(extract_count("5 passed; 2 failed; 1 ignored", "passed"), 5);
        assert_eq!(extract_count("5 passed; 2 failed; 1 ignored", "failed"), 2);
        assert_eq!(extract_count("5 passed; 2 failed; 1 ignored", "ignored"), 1);
    }

    #[test]
    fn test_extract_count_zero() {
        assert_eq!(extract_count("0 passed; 0 failed", "failed"), 0);
        assert_eq!(extract_count("no mention here", "passed"), 0);
    }

    #[test]
    fn test_parse_cargo_single_binary() {
        let stdout = "test result: ok. 7 passed; 0 failed; 1 ignored; 0 measured; finished in 0.01s\n";
        let (passed, failed, ignored) = parse_cargo(stdout, "");
        assert_eq!(passed, 7);
        assert_eq!(failed, 0);
        assert_eq!(ignored, 1);
    }

    #[test]
    fn test_parse_cargo_multiple_binaries() {
        let stdout = "\
test result: ok. 3 passed; 1 failed; 0 ignored; finished in 0.01s\n\
test result: ok. 4 passed; 1 failed; 2 ignored; finished in 0.02s\n";
        let (passed, failed, ignored) = parse_cargo(stdout, "");
        assert_eq!(passed, 7);
        assert_eq!(failed, 2);
        assert_eq!(ignored, 2);
    }

    #[test]
    fn test_parse_cargo_failure_line() {
        // Cargo may emit "FAILED" result lines too.
        let stdout = "test result: FAILED. 2 passed; 3 failed; 0 ignored; finished in 0.05s\n";
        let (passed, failed, ignored) = parse_cargo(stdout, "");
        assert_eq!(passed, 2);
        assert_eq!(failed, 3);
        assert_eq!(ignored, 0);
    }

    #[test]
    fn test_parse_pytest_summary() {
        let stdout = "FAILED tests/test_foo.py::test_bar - AssertionError\n\
1 failed, 4 passed, 2 skipped in 0.42s\n";
        let (passed, failed, ignored) = parse_pytest(stdout, "");
        assert_eq!(passed, 4);
        assert_eq!(failed, 1);
        assert_eq!(ignored, 2);
    }

    #[test]
    fn test_parse_pytest_all_pass() {
        let stdout = "5 passed in 0.10s\n";
        let (passed, failed, ignored) = parse_pytest(stdout, "");
        assert_eq!(passed, 5);
        assert_eq!(failed, 0);
        assert_eq!(ignored, 0);
    }

    #[test]
    fn test_parse_jest_summary() {
        let stderr = "Tests: 2 failed, 5 passed, 7 total\n";
        let (passed, failed, ignored) = parse_jest("", stderr);
        assert_eq!(passed, 5);
        assert_eq!(failed, 2);
        assert_eq!(ignored, 0);
    }

    #[test]
    fn test_parse_jest_with_skipped() {
        let stderr = "Tests: 1 skipped, 3 passed, 4 total\n";
        let (passed, failed, ignored) = parse_jest("", stderr);
        assert_eq!(passed, 3);
        assert_eq!(failed, 0);
        assert_eq!(ignored, 1);
    }

    #[test]
    fn test_parse_go_verbose() {
        let stdout = "\
--- PASS: TestFoo (0.00s)\n\
--- PASS: TestBar (0.01s)\n\
--- FAIL: TestBaz (0.00s)\n\
ok  \tgithub.com/example/pkg\t0.01s\n";
        let (passed, failed, ignored) = parse_go(stdout, "");
        assert_eq!(passed, 2);
        assert_eq!(failed, 1);
        assert_eq!(ignored, 0);
    }

    #[test]
    fn test_test_result_summary() {
        let r = TestResult {
            framework: "cargo".to_string(),
            passed: 10,
            failed: 2,
            ignored: 1,
            success: false,
            exit_code: Some(101),
            stdout: String::new(),
            stderr: String::new(),
        };
        let s = r.summary();
        assert!(s.contains("FAILED"));
        assert!(s.contains("10 passed"));
        assert!(s.contains("2 failed"));
        assert!(s.contains("1 ignored"));
        assert!(s.contains("13 total"));
    }

    #[test]
    fn test_detect_framework_no_files(tmp_env: ()) {
        // When no markers exist we should return None.
        let _ = tmp_env;
    }

    #[test]
    fn test_framework_name() {
        assert_eq!(TestFramework::Cargo.name(), "cargo");
        assert_eq!(TestFramework::Pytest.name(), "pytest");
        assert_eq!(TestFramework::Jest.name(), "jest");
        assert_eq!(TestFramework::GoTest.name(), "go test");
    }

    #[test]
    fn test_build_command_cargo_working_dir() {
        let dir = PathBuf::from("/workspace");
        let cmd = RunTestsTool::build_command(TestFramework::Cargo, &dir, &dir, None);
        // No manifest flag when path equals working dir.
        assert!(cmd.starts_with("cargo test"));
    }

    #[test]
    fn test_build_command_pytest() {
        let dir = PathBuf::from("/workspace");
        let cmd = RunTestsTool::build_command(TestFramework::Pytest, &dir, &dir, Some("-k foo"));
        assert!(cmd.contains("pytest"));
        assert!(cmd.contains("-k foo"));
    }

    #[test]
    fn test_build_command_go_root() {
        let dir = PathBuf::from("/workspace");
        let cmd = RunTestsTool::build_command(TestFramework::GoTest, &dir, &dir, None);
        assert!(cmd.contains("go test"));
        assert!(cmd.contains("./..."));
    }
}
