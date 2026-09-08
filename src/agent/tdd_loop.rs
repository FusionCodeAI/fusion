//! Autonomous Test-Driven Development (TDD) Fix & Verify Loop Engine.
//!
//! Provides automated feedback extraction, structured test failure parsing across
//! multiple language ecosystems (Rust `cargo test`, Python `pytest`, Node.js/Bun `vitest`/`jest`),
//! targeted correction guidance generation, and multi-phase TDD state tracking (Red -> Green -> Refactor -> Completed).

use std::fmt;
use std::path::Path;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// TDD Phase & Core State Types
// ---------------------------------------------------------------------------

/// Phases in the autonomous Test-Driven Development (TDD) lifecycle.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TddPhase {
    /// Initial phase where the test fails, capturing the bug or missing capability (Red).
    Red,
    /// Minimal implementation to satisfy the failing test and achieve passing status (Green).
    Green,
    /// Code quality, architecture, and maintainability improvement while keeping tests passing.
    Refactor,
    /// TDD cycle successfully completed with passing tests and verified refactoring.
    Completed,
}

impl TddPhase {
    /// Compute the next phase given whether tests passed during the current iteration.
    pub fn next_phase(&self, passed: bool) -> Self {
        match self {
            Self::Red => {
                if passed {
                    Self::Green
                } else {
                    Self::Red
                }
            }
            Self::Green => {
                if passed {
                    Self::Refactor
                } else {
                    Self::Red
                }
            }
            Self::Refactor => {
                if passed {
                    Self::Completed
                } else {
                    Self::Red
                }
            }
            Self::Completed => Self::Completed,
        }
    }

    /// Whether this phase represents completion of the TDD cycle.
    pub fn is_terminal(&self) -> bool {
        matches!(self, Self::Completed)
    }

    /// Human-readable label for the phase.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Red => "Red",
            Self::Green => "Green",
            Self::Refactor => "Refactor",
            Self::Completed => "Completed",
        }
    }
}

impl fmt::Display for TddPhase {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

/// A structured representation of a test failure extracted from runner output.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TestFailure {
    /// Name or identifier of the failing test or test case.
    pub test_name: String,
    /// Source file path where failure was triggered or located.
    pub file: Option<String>,
    /// Line number where the failure or panic occurred.
    pub line: Option<usize>,
    /// Primary error message, assertion failure, or exception details.
    pub error_message: String,
    /// Formatted stack trace or execution traceback, if available.
    pub stack_trace: Option<String>,
}

impl TestFailure {
    /// Create a new test failure with the given test name and error message.
    pub fn new(test_name: impl Into<String>, error_message: impl Into<String>) -> Self {
        Self {
            test_name: test_name.into(),
            file: None,
            line: None,
            error_message: error_message.into(),
            stack_trace: None,
        }
    }

    /// Attach source file location details.
    pub fn with_location(mut self, file: impl Into<String>, line: usize) -> Self {
        self.file = Some(file.into());
        self.line = Some(line);
        self
    }

    /// Attach formatted stack trace.
    pub fn with_stack_trace(mut self, trace: impl Into<String>) -> Self {
        self.stack_trace = Some(trace.into());
        self
    }
}

/// Single execution iteration within the autonomous TDD loop.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TddIteration {
    /// 1-based iteration counter.
    pub iteration: usize,
    /// Active phase during this iteration.
    pub phase: TddPhase,
    /// Command executed to verify or test the code.
    pub command: String,
    /// Structured test failures detected during execution.
    pub failures: Vec<TestFailure>,
    /// Whether all tests passed.
    pub passed: bool,
}

impl TddIteration {
    /// Create a new TddIteration record.
    pub fn new(
        iteration: usize,
        phase: TddPhase,
        command: impl Into<String>,
        failures: Vec<TestFailure>,
        passed: bool,
    ) -> Self {
        Self {
            iteration,
            phase,
            command: command.into(),
            failures,
            passed,
        }
    }

    /// Total number of detected test failures.
    pub fn failure_count(&self) -> usize {
        self.failures.len()
    }

    /// Whether this iteration passed without failures.
    pub fn is_passed(&self) -> bool {
        self.passed
    }
}

// ---------------------------------------------------------------------------
// TDD Engine
// ---------------------------------------------------------------------------

/// Autonomous TDD fix/verify loop engine with compiler and test runner feedback.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TddEngine {
    /// Current TDD lifecycle phase.
    pub current_phase: TddPhase,
    /// Current iteration count.
    pub current_iteration: usize,
    /// Maximum allowed iterations before aborting the loop.
    pub max_iterations: usize,
    /// Execution history of iterations.
    pub history: Vec<TddIteration>,
}

impl Default for TddEngine {
    fn default() -> Self {
        Self::new()
    }
}

impl TddEngine {
    /// Create a new TddEngine initialized to the `Red` phase.
    pub fn new() -> Self {
        Self {
            current_phase: TddPhase::Red,
            current_iteration: 0,
            max_iterations: 10,
            history: Vec::new(),
        }
    }

    /// Set a custom maximum iteration limit.
    pub fn with_max_iterations(mut self, max: usize) -> Self {
        self.max_iterations = max;
        self
    }

    /// Reset engine state back to initial `Red` phase.
    pub fn reset(&mut self) {
        self.current_phase = TddPhase::Red;
        self.current_iteration = 0;
        self.history.clear();
    }

    /// Advance the engine with a completed iteration record, transitioning phase accordingly.
    pub fn record_iteration(&mut self, iteration: TddIteration) -> TddPhase {
        self.current_iteration += 1;
        self.current_phase = self.current_phase.next_phase(iteration.passed);
        self.history.push(iteration);
        self.current_phase
    }

    /// Parse test output from raw runner output and command exit code.
    ///
    /// Supports:
    /// - Rust `cargo test` failures (`panicked at ...`, `assertion failed: left == right`, compiler errors).
    /// - Python `pytest` failures (traceback blocks, `E   ` assertions, summary lines).
    /// - Node.js / Bun `vitest` and `jest` failures (`FAIL`, `●`, `❯`, `at ...`).
    pub fn parse_test_output(raw_output: &str, exit_code: i32) -> (bool, Vec<TestFailure>) {
        let clean = strip_ansi(raw_output);

        // If exit code is 0 and there are no failure keywords, report success
        if exit_code == 0
            && !clean.contains("FAILED")
            && !clean.contains("FAIL ")
            && !clean.contains("FAIL\t")
            && !clean.contains("FAILURES")
            && !clean.contains("test result: FAILED")
        {
            return (true, Vec::new());
        }

        let mut failures = Vec::new();

        // 1. Rust `cargo test` signature detection
        if (clean.contains("running ") && clean.contains("tests"))
            || clean.contains("stdout ----")
            || clean.contains("test result: FAILED")
            || clean.contains("error[E")
        {
            failures = parse_cargo_test(&clean);
        }

        // 2. Python `pytest` signature detection
        if failures.is_empty()
            && (clean.contains("=== FAILURES ===")
                || clean.contains(" short test summary info ")
                || (clean.contains("FAILED ") && clean.contains("::"))
                || clean.contains("pytest"))
        {
            failures = parse_pytest(&clean);
        }

        // 3. Node.js / Bun `vitest` / `jest` signature detection
        if failures.is_empty()
            && (clean.contains("  ● ")
                || clean.contains("FAIL ")
                || clean.contains("FAIL\t")
                || clean.contains("Tests ")
                || clean.contains("Test Suites:")
                || clean.contains("vitest")
                || clean.contains("jest"))
        {
            failures = parse_vitest_jest(&clean);
        }

        // 4. Fallback if exit code is non-zero but no specific runner matched
        if failures.is_empty() && exit_code != 0 {
            let non_empty: Vec<&str> = clean
                .lines()
                .map(|l| l.trim())
                .filter(|l| !l.is_empty())
                .collect();
            let error_message = if non_empty.is_empty() {
                format!("Command exited with non-zero exit code: {}", exit_code)
            } else {
                let start = non_empty.len().saturating_sub(6);
                non_empty[start..].join("\n")
            };

            failures.push(TestFailure {
                test_name: "test_execution".to_string(),
                file: None,
                line: None,
                error_message,
                stack_trace: None,
            });
        }

        let passed = exit_code == 0 && failures.is_empty();
        (passed, failures)
    }
    /// Generate concise, targeted diagnosis and fix constraints for the given test failures.
    pub fn generate_correction_guidance(failures: &[TestFailure]) -> String {
        if failures.is_empty() {
            return "All tests passed. No test failures detected.".to_string();
        }

        let mut guidance = String::new();
        guidance.push_str(&format!(
            "## TDD Loop Diagnostic & Guidance\n\nDetected {} failing test(s):\n\n",
            failures.len()
        ));

        for (idx, failure) in failures.iter().enumerate() {
            guidance.push_str(&format!(
                "### Failure {}: `{}`\n",
                idx + 1,
                failure.test_name
            ));

            if let Some(file) = &failure.file {
                if let Some(line) = failure.line {
                    guidance.push_str(&format!("- **Location**: `{}:{}`\n", file, line));
                } else {
                    guidance.push_str(&format!("- **Location**: `{}`\n", file));
                }
            }

            let trimmed_err = failure.error_message.trim();
            guidance.push_str(&format!("- **Error**: {}\n", trimmed_err));

            // Targeted Diagnosis
            let diagnosis = diagnose_error(trimmed_err);
            guidance.push_str(&format!("- **Diagnosis**: {}\n", diagnosis));

            if let Some(trace) = &failure.stack_trace {
                let trace_lines: Vec<&str> = trace.lines().take(5).collect();
                if !trace_lines.is_empty() {
                    guidance.push_str(&format!(
                        "- **Trace Context**:\n```\n{}\n```\n",
                        trace_lines.join("\n")
                    ));
                }
            }

            guidance.push('\n');
        }

        guidance.push_str("### Target Fix Constraints:\n");
        guidance.push_str("1. **Minimal Change Principle**: Modify only the minimal code required to satisfy the failing test(s). Do not introduce speculative features.\n");
        guidance.push_str("2. **Test Invariance**: Never modify, comment out, or delete existing tests or assertions. The test represents the ground truth contract.\n");
        guidance.push_str(
            "3. **Regression Prevention**: Ensure all previously passing tests continue to pass.\n",
        );
        guidance.push_str("4. **Type & Signature Stability**: Preserve existing public API signatures, types, and error variants.\n");
        guidance.push_str("5. **Phase Progression**: Verify fix with the test runner command to advance from Red to Green phase.\n");

        guidance
    }

    /// Execute verification command synchronously and return the resulting iteration.
    pub fn verify_fix(command: &str, cwd: &Path) -> Result<TddIteration> {
        #[cfg(target_os = "windows")]
        let mut cmd = std::process::Command::new("cmd");
        #[cfg(target_os = "windows")]
        cmd.args(["/C", command]);

        #[cfg(not(target_os = "windows"))]
        let mut cmd = std::process::Command::new("sh");
        #[cfg(not(target_os = "windows"))]
        cmd.args(["-c", command]);

        cmd.current_dir(cwd);
        let output = cmd.output().with_context(|| {
            format!(
                "Failed to execute verification command: `{}` in `{}`",
                command,
                cwd.display()
            )
        })?;

        let exit_code = output.status.code().unwrap_or(-1);
        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);
        let combined = format!("{}\n{}", stdout, stderr);

        let (passed, failures) = Self::parse_test_output(&combined, exit_code);
        let phase = if passed {
            TddPhase::Green
        } else {
            TddPhase::Red
        };

        Ok(TddIteration {
            iteration: 1,
            phase,
            command: command.to_string(),
            failures,
            passed,
        })
    }

    /// Execute verification command asynchronously and return the resulting iteration.
    pub async fn verify_fix_async(command: &str, cwd: &Path) -> Result<TddIteration> {
        #[cfg(target_os = "windows")]
        let mut cmd = tokio::process::Command::new("cmd");
        #[cfg(target_os = "windows")]
        cmd.args(["/C", command]);

        #[cfg(not(target_os = "windows"))]
        let mut cmd = tokio::process::Command::new("sh");
        #[cfg(not(target_os = "windows"))]
        cmd.args(["-c", command]);

        cmd.current_dir(cwd);
        let output = cmd.output().await.with_context(|| {
            format!(
                "Failed to execute verification command: `{}` in `{}`",
                command,
                cwd.display()
            )
        })?;

        let exit_code = output.status.code().unwrap_or(-1);
        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);
        let combined = format!("{}\n{}", stdout, stderr);

        let (passed, failures) = Self::parse_test_output(&combined, exit_code);
        let phase = if passed {
            TddPhase::Green
        } else {
            TddPhase::Red
        };

        Ok(TddIteration {
            iteration: 1,
            phase,
            command: command.to_string(),
            failures,
            passed,
        })
    }

    /// Run a synchronous verification step, updating the engine's internal iteration history and phase.
    pub fn step(&mut self, command: &str, cwd: &Path) -> Result<TddIteration> {
        let mut iter = Self::verify_fix(command, cwd)?;
        iter.iteration = self.current_iteration + 1;
        iter.phase = self.current_phase;
        self.record_iteration(iter.clone());
        Ok(iter)
    }

    /// Run an asynchronous verification step, updating the engine's internal iteration history and phase.
    pub async fn step_async(&mut self, command: &str, cwd: &Path) -> Result<TddIteration> {
        let mut iter = Self::verify_fix_async(command, cwd).await?;
        iter.iteration = self.current_iteration + 1;
        iter.phase = self.current_phase;
        self.record_iteration(iter.clone());
        Ok(iter)
    }
}

// ---------------------------------------------------------------------------
// Runner Parsers
// ---------------------------------------------------------------------------

/// Strip ANSI escape sequences from terminal output.
pub fn strip_ansi(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut chars = input.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\x1b' {
            match chars.peek() {
                Some(&'[') => {
                    chars.next(); // consume '['
                                  // consume parameters and intermediate bytes until final byte (0x40-0x7E)
                    while let Some(&b) = chars.peek() {
                        chars.next();
                        if ('@'..='~').contains(&b) {
                            break;
                        }
                    }
                }
                Some(&'(') | Some(&')') => {
                    chars.next();
                    chars.next(); // consume charset designator
                }
                _ => {}
            }
        } else {
            out.push(c);
        }
    }
    out
}

/// Parse source file path and line number from strings like `path/to/file.rs:42:5` or `path/to/file.rs:42`.
pub fn parse_file_line(s: &str) -> Option<(String, usize)> {
    let s = s.trim();
    let parts: Vec<&str> = s.rsplitn(3, ':').collect();
    if parts.len() == 3 {
        // parts[0] is col, parts[1] is line, parts[2] is file
        if let Ok(line) = parts[1].trim().parse::<usize>() {
            return Some((parts[2].trim().to_string(), line));
        }
    } else if parts.len() == 2 {
        // parts[0] is line, parts[1] is file
        if let Ok(line) = parts[0].trim().parse::<usize>() {
            return Some((parts[1].trim().to_string(), line));
        }
    }
    None
}

/// Parse Rust `cargo test` failures from cleaned text.
fn parse_cargo_test(clean: &str) -> Vec<TestFailure> {
    let mut failures = Vec::new();

    // 1. Parse `---- <test_name> stdout ----` sections
    let stdout_marker = " stdout ----";
    let mut remaining = clean;
    while let Some(pos) = remaining.find(stdout_marker) {
        let prefix = &remaining[..pos];
        let line_start = prefix.rfind("---- ").map(|i| i + 5).unwrap_or(0);
        let test_name = prefix[line_start..].trim().to_string();

        let after_marker = &remaining[pos + stdout_marker.len()..];
        let block_end = after_marker
            .find("\n---- ")
            .or_else(|| after_marker.find("\nfailures:"))
            .unwrap_or(after_marker.len());
        let block = &after_marker[..block_end];

        let mut file = None;
        let mut line = None;
        let mut error_lines = Vec::new();
        let mut stack_trace_lines = Vec::new();
        let mut in_stack_trace = false;
        let mut found_panic = false;

        for raw_l in block.lines() {
            let l = raw_l.trim();
            if l.starts_with("stack backtrace:") {
                in_stack_trace = true;
                continue;
            }
            if l.starts_with("note: run with `RUST_BACKTRACE") {
                in_stack_trace = false;
                continue;
            }

            if in_stack_trace {
                if !l.is_empty() {
                    stack_trace_lines.push(raw_l.to_string());
                }
                continue;
            }

            if l.contains("panicked at") {
                found_panic = true;
                // Format: panicked at 'msg', file:line:col
                if let Some(panic_idx) = l.find("panicked at '") {
                    let after_quote = &l[panic_idx + 13..];
                    if let Some(close_quote) = after_quote.find("',") {
                        let msg = &after_quote[..close_quote];
                        error_lines.push(msg.to_string());
                        let rest = after_quote[close_quote + 2..].trim();
                        if let Some((f, l_num)) = parse_file_line(rest) {
                            file = Some(f);
                            line = Some(l_num);
                        }
                    }
                } else if let Some(panic_idx) = l.find("panicked at ") {
                    // Format: panicked at file:line:col:
                    let rest = &l[panic_idx + 12..].trim_end_matches(':');
                    if let Some((f, l_num)) = parse_file_line(rest) {
                        file = Some(f);
                        line = Some(l_num);
                    }
                }
                continue;
            }

            if found_panic {
                if l.starts_with("note:") {
                    found_panic = false;
                    continue;
                }
                if !l.is_empty() {
                    error_lines.push(raw_l.trim().to_string());
                } else if !error_lines.is_empty() {
                    found_panic = false;
                }
            }
        }

        let error_message = if error_lines.is_empty() {
            "Test panicked without explicit assertion message".to_string()
        } else {
            error_lines.join("\n")
        };

        let stack_trace = if stack_trace_lines.is_empty() {
            None
        } else {
            Some(stack_trace_lines.join("\n"))
        };

        failures.push(TestFailure {
            test_name,
            file,
            line,
            error_message,
            stack_trace,
        });

        remaining = &after_marker[block_end..];
    }

    // 2. Compiler errors during cargo test (e.g. error[E0425]: ...)
    if failures.is_empty()
        && (clean.contains("error[E") || clean.contains("error: aborting due to"))
    {
        for line_str in clean.lines() {
            if line_str.starts_with("error[E") || line_str.starts_with("error:") {
                let err_msg = line_str.to_string();
                let mut loc_file = None;
                let mut loc_line = None;

                if let Some(arrow_pos) = clean.find("  --> ") {
                    let after_arrow = &clean[arrow_pos + 6..];
                    if let Some(end_line) = after_arrow.lines().next() {
                        if let Some((f, l)) = parse_file_line(end_line.trim()) {
                            loc_file = Some(f);
                            loc_line = Some(l);
                        }
                    }
                }

                failures.push(TestFailure {
                    test_name: "compiler::error".to_string(),
                    file: loc_file,
                    line: loc_line,
                    error_message: err_msg,
                    stack_trace: None,
                });
                break;
            }
        }
    }

    // 3. Fallback: check failures summary if no stdout blocks were captured
    if failures.is_empty() {
        if let Some(failures_pos) = clean.find("\nfailures:\n") {
            let after_fail = &clean[failures_pos + 11..];
            let list_end = after_fail
                .find("\ntest result:")
                .unwrap_or(after_fail.len());
            for line_str in after_fail[..list_end].lines() {
                let t = line_str.trim();
                if !t.is_empty() && !t.starts_with("----") {
                    failures.push(TestFailure {
                        test_name: t.to_string(),
                        file: None,
                        line: None,
                        error_message: format!("Test `{}` failed", t),
                        stack_trace: None,
                    });
                }
            }
        }
    }

    failures
}

/// Parse Python `pytest` failures from cleaned text.
fn parse_pytest(clean: &str) -> Vec<TestFailure> {
    let mut failures = Vec::new();

    let lines: Vec<&str> = clean.lines().collect();
    let mut i = 0;
    while i < lines.len() {
        let line = lines[i].trim();
        if line.starts_with("___") && line.ends_with("___") {
            let test_name = line.trim_matches('_').trim().to_string();
            let mut error_lines = Vec::new();
            let mut stack_lines = Vec::new();
            let mut file = None;
            let mut line_no = None;

            i += 1;
            while i < lines.len() {
                let cur = lines[i];
                let cur_trim = cur.trim();

                if (cur_trim.starts_with("___") && cur_trim.ends_with("___"))
                    || cur_trim.starts_with("=== short test summary")
                    || (cur_trim.starts_with("===") && cur_trim.contains("failed"))
                {
                    break;
                }

                if cur.starts_with("E   ") || cur.starts_with("E ") {
                    let msg = cur[2..].trim();
                    error_lines.push(msg.to_string());
                } else if cur_trim.starts_with('>')
                    || cur_trim.starts_with("def ")
                    || cur_trim.starts_with("_ _")
                {
                    stack_lines.push(cur.to_string());
                } else if cur_trim.contains(".py:") && file.is_none() {
                    if let Some(colon_pos) = cur_trim.find(".py:") {
                        let file_str = &cur_trim[..colon_pos + 3];
                        let rest = &cur_trim[colon_pos + 4..];
                        let num_str: String =
                            rest.chars().take_while(|c| c.is_ascii_digit()).collect();
                        if let Ok(num) = num_str.parse::<usize>() {
                            file = Some(file_str.to_string());
                            line_no = Some(num);
                        }
                    }
                }

                i += 1;
            }

            let error_message = if error_lines.is_empty() {
                "Test failed (AssertionError or unhandled exception)".to_string()
            } else {
                error_lines.join("\n")
            };

            let stack_trace = if stack_lines.is_empty() {
                None
            } else {
                Some(stack_lines.join("\n"))
            };

            failures.push(TestFailure {
                test_name,
                file,
                line: line_no,
                error_message,
                stack_trace,
            });
            continue;
        }
        i += 1;
    }

    // Fallback: parse `short test summary info`: `FAILED tests/test_math.py::test_add - assert 3 == 4`
    if failures.is_empty() {
        for line in clean.lines() {
            let trim = line.trim();
            if let Some(rest) = trim.strip_prefix("FAILED ") {
                if let Some((loc, err)) = rest.split_once(" - ") {
                    let loc = loc.trim();
                    let err = err.trim();
                    let (f, test_name) = if let Some((f_path, t_name)) = loc.split_once("::") {
                        (Some(f_path.to_string()), t_name.to_string())
                    } else {
                        (None, loc.to_string())
                    };

                    failures.push(TestFailure {
                        test_name,
                        file: f,
                        line: None,
                        error_message: err.to_string(),
                        stack_trace: None,
                    });
                }
            }
        }
    }

    failures
}

/// Parse Node.js / Bun `vitest` and `jest` failures from cleaned text.
fn parse_vitest_jest(clean: &str) -> Vec<TestFailure> {
    let mut failures = Vec::new();

    // 1. Jest format with `  ● ` test header markers
    if clean.contains("  ● ") || clean.contains("\n● ") {
        let marker = "● ";
        let mut parts = clean.split(marker);
        let _ = parts.next(); // skip before first marker
        for part in parts {
            let lines: Vec<&str> = part.lines().collect();
            if lines.is_empty() {
                continue;
            }
            let test_name = lines[0].trim().to_string();
            let mut error_lines = Vec::new();
            let mut stack_lines = Vec::new();
            let mut file = None;
            let mut line_no = None;

            for raw_l in &lines[1..] {
                let l = raw_l.trim();
                if l.is_empty() {
                    continue;
                }
                if l.starts_with("at ") {
                    stack_lines.push(l.to_string());
                    if file.is_none() {
                        if let Some((f, ln)) = parse_js_stack_location(l) {
                            file = Some(f);
                            line_no = Some(ln);
                        }
                    }
                } else if l.contains('|')
                    && l.chars()
                        .take_while(|c| c.is_whitespace() || c.is_ascii_digit())
                        .any(|c| c.is_ascii_digit())
                {
                    stack_lines.push(raw_l.to_string());
                } else if !stack_lines.is_empty() {
                    stack_lines.push(l.to_string());
                } else {
                    error_lines.push(l.to_string());
                }
            }

            let error_message = if error_lines.is_empty() {
                "Test failed assertion".to_string()
            } else {
                error_lines.join("\n")
            };

            let stack_trace = if stack_lines.is_empty() {
                None
            } else {
                Some(stack_lines.join("\n"))
            };

            failures.push(TestFailure {
                test_name,
                file,
                line: line_no,
                error_message,
                stack_trace,
            });
        }
        return failures;
    }

    // 2. Vitest format: `FAIL src/sum.test.ts > sum of two numbers` or `× tests/...`
    let lines: Vec<&str> = clean.lines().collect();
    let mut i = 0;
    while i < lines.len() {
        let line = lines[i].trim();
        let is_vitest_fail =
            line.starts_with("FAIL ") || line.starts_with("FAIL\t") || line.starts_with("× ");
        if is_vitest_fail {
            let header = line
                .trim_start_matches("FAIL")
                .trim_start_matches('×')
                .trim();
            let (file_hint, test_name) = if let Some((f, t)) = header.split_once(" > ") {
                (Some(f.trim().to_string()), t.trim().to_string())
            } else {
                (None, header.to_string())
            };

            let mut error_lines = Vec::new();
            let mut stack_lines = Vec::new();
            let mut file = file_hint;
            let mut line_no = None;

            i += 1;
            while i < lines.len() {
                let cur = lines[i];
                let cur_trim = cur.trim();
                if cur_trim.starts_with("FAIL ")
                    || cur_trim.starts_with("× ")
                    || cur_trim.starts_with("Tests ")
                {
                    break;
                }

                if cur_trim.starts_with('❯') || cur_trim.starts_with("at ") {
                    stack_lines.push(cur_trim.to_string());
                    if line_no.is_none() {
                        if let Some((f, ln)) = parse_js_stack_location(cur_trim) {
                            file = Some(f);
                            line_no = Some(ln);
                        }
                    }
                } else if cur_trim.contains('|')
                    && cur_trim
                        .chars()
                        .take_while(|c| c.is_whitespace() || c.is_ascii_digit())
                        .any(|c| c.is_ascii_digit())
                {
                    stack_lines.push(cur.to_string());
                } else if !cur_trim.is_empty() && !cur_trim.starts_with('⎯') {
                    error_lines.push(cur_trim.to_string());
                }
                i += 1;
            }

            let error_message = if error_lines.is_empty() {
                "Assertion failed".to_string()
            } else {
                error_lines.join("\n")
            };

            let stack_trace = if stack_lines.is_empty() {
                None
            } else {
                Some(stack_lines.join("\n"))
            };

            failures.push(TestFailure {
                test_name,
                file,
                line: line_no,
                error_message,
                stack_trace,
            });
            continue;
        }
        i += 1;
    }

    failures
}

/// Extract file and line from JS/TS stack frame line.
fn parse_js_stack_location(line: &str) -> Option<(String, usize)> {
    let line = line.trim();
    let candidate = if let Some(open) = line.rfind('(') {
        if let Some(close) = line[open..].find(')') {
            &line[open + 1..open + close]
        } else {
            line
        }
    } else {
        line.trim_start_matches('❯')
            .trim_start_matches("at ")
            .trim()
    };

    parse_file_line(candidate)
}

/// Generate targeted diagnosis from error message content.
fn diagnose_error(error_msg: &str) -> &'static str {
    let lower = error_msg.to_lowercase();
    if lower.contains("assertion `left == right` failed")
        || lower.contains("assertion failed: left == right")
        || lower.contains("assert ")
        || lower.contains("expect(")
        || lower.contains("expected ")
    {
        "Assertion mismatch: Returned value diverges from the expected test contract. Compare actual vs expected value."
    } else if lower.contains("divide by zero") || lower.contains("zerodivisionerror") {
        "Zero division error: Unchecked division operator. Add input zero check or validation."
    } else if lower.contains("typeerror") || lower.contains("mismatched types") {
        "Type inconsistency: Value type does not match expected parameter or return type."
    } else if lower.contains("not found")
        || lower.contains("cannot find")
        || lower.contains("undefined")
    {
        "Missing symbol: Referenced variable, function, or module is not defined in current scope."
    } else if lower.contains("panicked at") || lower.contains("panic") {
        "Runtime panic: Explicit panic or unwrap on None/Err encountered during execution."
    } else {
        "Test execution failed. Inspect failure details and stack trace above."
    }
}

// ---------------------------------------------------------------------------
// Unit Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_tdd_phase_transitions() {
        let phase = TddPhase::Red;
        assert_eq!(phase.next_phase(false), TddPhase::Red);
        assert_eq!(phase.next_phase(true), TddPhase::Green);

        let green = TddPhase::Green;
        assert_eq!(green.next_phase(true), TddPhase::Refactor);
        assert_eq!(green.next_phase(false), TddPhase::Red);

        let refactor = TddPhase::Refactor;
        assert_eq!(refactor.next_phase(true), TddPhase::Completed);
        assert_eq!(refactor.next_phase(false), TddPhase::Red);

        let completed = TddPhase::Completed;
        assert!(completed.is_terminal());
        assert_eq!(completed.next_phase(true), TddPhase::Completed);
    }

    #[test]
    fn test_cargo_test_panic_parsing() {
        let output = r#"
running 3 tests
test tests::test_add ... ok
test tests::test_sub ... FAILED
test tests::test_mul ... ok

failures:

---- tests::test_sub stdout ----
thread 'tests::test_sub' panicked at src/lib.rs:42:5:
assertion `left == right` failed
  left: 3
 right: 5
note: run with `RUST_BACKTRACE=1` environment variable to display a backtrace

failures:
    tests::test_sub

test result: FAILED. 2 passed; 1 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.01s
"#;
        let (passed, failures) = TddEngine::parse_test_output(output, 101);
        assert!(!passed);
        assert_eq!(failures.len(), 1);
        let f = &failures[0];
        assert_eq!(f.test_name, "tests::test_sub");
        assert_eq!(f.file.as_deref(), Some("src/lib.rs"));
        assert_eq!(f.line, Some(42));
        assert!(f.error_message.contains("assertion `left == right` failed"));
    }

    #[test]
    fn test_pytest_failure_parsing() {
        let output = r#"
============================= test session starts ==============================
collected 2 items

tests/test_math.py .F                                                    [100%]

=================================== FAILURES ===================================
___________________________________ test_add ___________________________________

    def test_add():
>       assert add(1, 2) == 4
E       assert 3 == 4
E        +  where 3 = add(1, 2)

tests/test_math.py:15: AssertionError
=========================== short test summary info ============================
FAILED tests/test_math.py::test_add - assert 3 == 4
========================= 1 failed, 1 passed in 0.05s ==========================
"#;
        let (passed, failures) = TddEngine::parse_test_output(output, 1);
        assert!(!passed);
        assert_eq!(failures.len(), 1);
        let f = &failures[0];
        assert_eq!(f.test_name, "test_add");
        assert_eq!(f.file.as_deref(), Some("tests/test_math.py"));
        assert_eq!(f.line, Some(15));
        assert!(f.error_message.contains("assert 3 == 4"));
    }

    #[test]
    fn test_vitest_failure_parsing() {
        let output = r#"
FAIL  src/sum.test.ts > sum of two numbers
AssertionError: expected 3 to be 4 // Object.is equality

- Expected
+ Received

- 4
+ 3

 ❯ src/sum.test.ts:7:14
      5| test('sum of two numbers', () => {
      6|   expect(sum(1, 2)).toBe(4)
      7| })
 ❯ runTest node_modules/vitest/dist/runner.js:123:45

Tests  1 failed | 2 passed (3)
"#;
        let (passed, failures) = TddEngine::parse_test_output(output, 1);
        assert!(!passed);
        assert_eq!(failures.len(), 1);
        let f = &failures[0];
        assert_eq!(f.test_name, "sum of two numbers");
        assert_eq!(f.file.as_deref(), Some("src/sum.test.ts"));
        assert_eq!(f.line, Some(7));
        assert!(f.error_message.contains("expected 3 to be 4"));
    }

    #[test]
    fn test_passing_test_output() {
        let output = "test result: ok. 5 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out";
        let (passed, failures) = TddEngine::parse_test_output(output, 0);
        assert!(passed);
        assert!(failures.is_empty());
    }

    #[test]
    fn test_guidance_generation() {
        let failure = TestFailure::new("tests::test_divide", "attempt to divide by zero")
            .with_location("src/calc.rs", 10);
        let guidance = TddEngine::generate_correction_guidance(&[failure]);
        assert!(guidance.contains("tests::test_divide"));
        assert!(guidance.contains("src/calc.rs:10"));
        assert!(guidance.contains("Target Fix Constraints"));
    }
}
