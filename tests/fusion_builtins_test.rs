//! Integration tests for `fusion-builtins`.
//!
//! Comprehensive test suite verifying:
//! 1. Registry verification: `fusion_builtins::utility_builtins` contains `wc`, `cut`, and `head`
//! 2. Pure-Rust `wc` utility: line counting, word counting, byte/char counting, max line length, default triple counts, file/stdin operands
//! 3. Pure-Rust `cut` utility: byte/char slicing, field extraction, custom/default delimiters, whitespace extension (-w), complement, output delimiters
//! 4. Pure-Rust `head` utility: default lines, -n count, negative count, -c bytes, obsolete -N syntax, multi-file headers, -v verbose, -q quiet

use std::fs;
use std::path::Path;

use bytes::Bytes;
use flume::unbounded;
use fusion_shell::{
    cancel::CancelToken,
    execute_shell_streams,
    shell::{ShellExecuteOptions, StreamSinks},
};

/// Helper to execute a command string through the pure-Rust shell with captured stdout/stderr.
async fn run_sh(cmd: &str) -> (i32, String, String) {
    let (stdout_tx, stdout_rx) = unbounded::<Bytes>();
    let (stderr_tx, stderr_rx) = unbounded::<Bytes>();

    let result = execute_shell_streams(
        ShellExecuteOptions {
            command: cmd.to_string(),
            timeout_ms: Some(15_000),
            ..Default::default()
        },
        StreamSinks {
            stdout: Some(stdout_tx),
            stderr: Some(stderr_tx),
        },
        CancelToken::default(),
    )
    .await
    .expect("shell execution failed");

    let mut stdout_buf = Vec::new();
    while let Ok(chunk) = stdout_rx.try_recv() {
        stdout_buf.extend_from_slice(&chunk);
    }
    let mut stderr_buf = Vec::new();
    while let Ok(chunk) = stderr_rx.try_recv() {
        stderr_buf.extend_from_slice(&chunk);
    }

    (
        result.exit_code.unwrap_or(-1),
        String::from_utf8_lossy(&stdout_buf).to_string(),
        String::from_utf8_lossy(&stderr_buf).to_string(),
    )
}

/// Helper to execute a command in a specific working directory.
async fn run_sh_in(dir: &Path, cmd: &str) -> (i32, String, String) {
    let (stdout_tx, stdout_rx) = unbounded::<Bytes>();
    let (stderr_tx, stderr_rx) = unbounded::<Bytes>();

    let result = execute_shell_streams(
        ShellExecuteOptions {
            command: cmd.to_string(),
            cwd: Some(dir.to_string_lossy().to_string()),
            timeout_ms: Some(15_000),
            ..Default::default()
        },
        StreamSinks {
            stdout: Some(stdout_tx),
            stderr: Some(stderr_tx),
        },
        CancelToken::default(),
    )
    .await
    .expect("shell execution failed");

    let mut stdout_buf = Vec::new();
    while let Ok(chunk) = stdout_rx.try_recv() {
        stdout_buf.extend_from_slice(&chunk);
    }
    let mut stderr_buf = Vec::new();
    while let Ok(chunk) = stderr_rx.try_recv() {
        stderr_buf.extend_from_slice(&chunk);
    }

    (
        result.exit_code.unwrap_or(-1),
        String::from_utf8_lossy(&stdout_buf).to_string(),
        String::from_utf8_lossy(&stderr_buf).to_string(),
    )
}

// ============================================================================
// 1. Registry Verification
// ============================================================================

#[test]
fn test_utility_builtins_registry_contains_posix_commands() {
    let builtins =
        fusion_builtins::utility_builtins::<brush_core::extensions::DefaultShellExtensions>();
    let names: Vec<&str> = builtins.into_iter().map(|(name, _)| name).collect();

    assert!(
        names.contains(&"wc"),
        "utility_builtins must register pure-Rust 'wc'"
    );
    assert!(
        names.contains(&"cut"),
        "utility_builtins must register pure-Rust 'cut'"
    );
    assert!(
        names.contains(&"head"),
        "utility_builtins must register pure-Rust 'head'"
    );
}

// ============================================================================
// 2. Pure-Rust `wc` Utility Tests
// ============================================================================

#[tokio::test]
async fn test_wc_line_count_stdin() {
    let (code, stdout, stderr) = run_sh("printf 'line1\\nline2\\nline3\\n' | wc -l").await;
    assert_eq!(code, 0, "stderr: {stderr}");
    assert_eq!(stdout.trim(), "3");
}

#[tokio::test]
async fn test_wc_line_count_empty_input() {
    let (code, stdout, stderr) = run_sh("printf '' | wc -l").await;
    assert_eq!(code, 0, "stderr: {stderr}");
    assert_eq!(stdout.trim(), "0");
}

#[tokio::test]
async fn test_wc_line_count_no_trailing_newline() {
    let (code, stdout, stderr) = run_sh("printf 'alpha\\nbeta' | wc -l").await;
    assert_eq!(code, 0, "stderr: {stderr}");
    // Only 1 newline character present
    assert_eq!(stdout.trim(), "1");
}

#[tokio::test]
async fn test_wc_word_count() {
    let (code, stdout, stderr) =
        run_sh("printf 'The quick brown fox jumps over the lazy dog' | wc -w").await;
    assert_eq!(code, 0, "stderr: {stderr}");
    assert_eq!(stdout.trim(), "9");
}

#[tokio::test]
async fn test_wc_word_count_multiline_and_mixed_whitespace() {
    let (code, stdout, stderr) =
        run_sh("printf '  word1\\tword2   \\n\\n  word3\\nword4 word5  ' | wc -w").await;
    assert_eq!(code, 0, "stderr: {stderr}");
    assert_eq!(stdout.trim(), "5");
}

#[tokio::test]
async fn test_wc_byte_count() {
    let (code, stdout, stderr) = run_sh("printf '1234567890' | wc -c").await;
    assert_eq!(code, 0, "stderr: {stderr}");
    assert_eq!(stdout.trim(), "10");
}

#[tokio::test]
async fn test_wc_char_count_vs_byte_count_unicode() {
    // "hello 🦀" has 6 ASCII bytes + 4 UTF-8 bytes for crab emoji = 10 bytes, 7 chars
    let (code_c, stdout_c, _) = run_sh("printf 'hello 🦀' | wc -c").await;
    assert_eq!(code_c, 0);
    assert_eq!(stdout_c.trim(), "10");

    let (code_m, stdout_m, _) = run_sh("printf 'hello 🦀' | wc -m").await;
    assert_eq!(code_m, 0);
    assert_eq!(stdout_m.trim(), "7");
}

#[tokio::test]
async fn test_wc_max_line_length() {
    let (code, stdout, stderr) =
        run_sh("printf 'short\\nvery long line here\\nmid\\n' | wc -L").await;
    assert_eq!(code, 0, "stderr: {stderr}");
    assert_eq!(stdout.trim(), "19");
}

#[tokio::test]
async fn test_wc_default_counts_lines_words_bytes() {
    let (code, stdout, stderr) = run_sh("printf 'hello world\\nsecond line\\n' | wc").await;
    assert_eq!(code, 0, "stderr: {stderr}");
    let parts: Vec<&str> = stdout.split_whitespace().collect();
    assert_eq!(parts.len(), 3, "expected 3 count columns: {stdout}");
    assert_eq!(parts[0], "2", "lines");
    assert_eq!(parts[1], "4", "words");
    assert_eq!(parts[2], "24", "bytes");
}

#[tokio::test]
async fn test_wc_file_operand() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let file_path = tmp.path().join("sample.txt");
    fs::write(&file_path, "one two three\nfour five\n").expect("write sample.txt");

    let (code, stdout, stderr) = run_sh_in(tmp.path(), "wc -l sample.txt").await;
    assert_eq!(code, 0, "stderr: {stderr}");
    assert!(
        stdout.contains('2') && stdout.contains("sample.txt"),
        "expected 2 lines and filename in output: {stdout}"
    );

    let (code, stdout, stderr) = run_sh_in(tmp.path(), "wc -w sample.txt").await;
    assert_eq!(code, 0, "stderr: {stderr}");
    assert!(
        stdout.contains('5') && stdout.contains("sample.txt"),
        "expected 5 words and filename in output: {stdout}"
    );
}

#[tokio::test]
async fn test_wc_multiple_files_and_total() {
    let tmp = tempfile::tempdir().expect("tempdir");
    fs::write(tmp.path().join("a.txt"), "foo bar\n").expect("write a");
    fs::write(tmp.path().join("b.txt"), "baz qux quux\n").expect("write b");

    let (code, stdout, stderr) = run_sh_in(tmp.path(), "wc -w a.txt b.txt").await;
    assert_eq!(code, 0, "stderr: {stderr}");
    assert!(stdout.contains("a.txt"), "should mention a.txt: {stdout}");
    assert!(stdout.contains("b.txt"), "should mention b.txt: {stdout}");
    assert!(
        stdout.contains("total") || stdout.contains("5"),
        "should display total: {stdout}"
    );
}

// ============================================================================
// 3. Pure-Rust `cut` Utility Tests
// ============================================================================

#[tokio::test]
async fn test_cut_delimiter_and_fields() {
    let (code, stdout, stderr) =
        run_sh("printf 'root:x:0:0:root:/root:/bin/bash\\n' | cut -d ':' -f 1,7").await;
    assert_eq!(code, 0, "stderr: {stderr}");
    assert_eq!(stdout.trim(), "root:/bin/bash");
}

#[tokio::test]
async fn test_cut_field_range() {
    let (code, stdout, stderr) = run_sh("printf 'a,b,c,d,e\\n' | cut -d ',' -f 2-4").await;
    assert_eq!(code, 0, "stderr: {stderr}");
    assert_eq!(stdout.trim(), "b,c,d");
}

#[tokio::test]
async fn test_cut_open_ended_fields() {
    let (code, stdout, stderr) = run_sh("printf '1:2:3:4:5\\n' | cut -d ':' -f 3-").await;
    assert_eq!(code, 0, "stderr: {stderr}");
    assert_eq!(stdout.trim(), "3:4:5");

    let (code, stdout, stderr) = run_sh("printf '1:2:3:4:5\\n' | cut -d ':' -f -2").await;
    assert_eq!(code, 0, "stderr: {stderr}");
    assert_eq!(stdout.trim(), "1:2");
}

#[tokio::test]
async fn test_cut_byte_selection() {
    let (code, stdout, stderr) = run_sh("printf 'abcdefgh\\n' | cut -b 2,4,6").await;
    assert_eq!(code, 0, "stderr: {stderr}");
    assert_eq!(stdout.trim(), "bdf");

    let (code, stdout, stderr) = run_sh("printf 'abcdefgh\\n' | cut -b 3-5").await;
    assert_eq!(code, 0, "stderr: {stderr}");
    assert_eq!(stdout.trim(), "cde");
}

#[tokio::test]
async fn test_cut_character_selection() {
    let (code, stdout, stderr) = run_sh("printf 'hello world\\n' | cut -c 1-5").await;
    assert_eq!(code, 0, "stderr: {stderr}");
    assert_eq!(stdout.trim(), "hello");

    let (code, stdout, stderr) = run_sh("printf 'hello world\\n' | cut -c 7-11").await;
    assert_eq!(code, 0, "stderr: {stderr}");
    assert_eq!(stdout.trim(), "world");
}

#[tokio::test]
async fn test_cut_default_tab_delimiter() {
    let (code, stdout, stderr) =
        run_sh("printf 'col1\\tcol2\\tcol3\\ncolA\\tcolB\\tcolC\\n' | cut -f 2").await;
    assert_eq!(code, 0, "stderr: {stderr}");
    let lines: Vec<&str> = stdout.trim().lines().collect();
    assert_eq!(lines, vec!["col2", "colB"]);
}

#[tokio::test]
async fn test_cut_whitespace_delimited_extension() {
    let (code, stdout, stderr) =
        run_sh("printf 'a    b    c\\n1   2   3\\n' | cut -w -f 2").await;
    assert_eq!(code, 0, "stderr: {stderr}");
    let lines: Vec<&str> = stdout.trim().lines().collect();
    assert_eq!(lines, vec!["b", "2"]);
}

#[tokio::test]
async fn test_cut_complement() {
    let (code, stdout, stderr) =
        run_sh("printf 'one:two:three:four:five\\n' | cut -d ':' --complement -f 2,4").await;
    assert_eq!(code, 0, "stderr: {stderr}");
    assert_eq!(stdout.trim(), "one:three:five");
}

#[tokio::test]
async fn test_cut_output_delimiter() {
    let (code, stdout, stderr) =
        run_sh("printf 'alpha:beta:gamma\\n' | cut -d ':' -f 1,3 --output-delimiter='--'").await;
    assert_eq!(code, 0, "stderr: {stderr}");
    assert_eq!(stdout.trim(), "alpha--gamma");
}

#[tokio::test]
async fn test_cut_suppress_lines_without_delimiter() {
    let (code, stdout, stderr) =
        run_sh("printf 'has:delimiter\\nno_delimiter\\nalso:has\\n' | cut -d ':' -f 1 -s").await;
    assert_eq!(code, 0, "stderr: {stderr}");
    let lines: Vec<&str> = stdout.trim().lines().collect();
    assert_eq!(lines, vec!["has", "also"]);
}

#[tokio::test]
async fn test_cut_file_operand() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let csv = tmp.path().join("data.csv");
    fs::write(&csv, "id,name,role\n1,alice,admin\n2,bob,user\n").expect("write csv");

    let (code, stdout, stderr) = run_sh_in(tmp.path(), "cut -d ',' -f 2 data.csv").await;
    assert_eq!(code, 0, "stderr: {stderr}");
    let lines: Vec<&str> = stdout.trim().lines().collect();
    assert_eq!(lines, vec!["name", "alice", "bob"]);
}

// ============================================================================
// 4. Pure-Rust `head` Utility Tests
// ============================================================================

#[tokio::test]
async fn test_head_default_ten_lines() {
    let (code, stdout, stderr) = run_sh("seq 1 20 | head").await;
    assert_eq!(code, 0, "stderr: {stderr}");
    let lines: Vec<&str> = stdout.trim().lines().collect();
    assert_eq!(lines.len(), 10);
    assert_eq!(lines[0], "1");
    assert_eq!(lines[9], "10");
}

#[tokio::test]
async fn test_head_n_lines() {
    let (code, stdout, stderr) = run_sh("seq 1 20 | head -n 4").await;
    assert_eq!(code, 0, "stderr: {stderr}");
    let lines: Vec<&str> = stdout.trim().lines().collect();
    assert_eq!(lines, vec!["1", "2", "3", "4"]);
}

#[tokio::test]
async fn test_head_zero_lines() {
    let (code, stdout, stderr) = run_sh("seq 1 10 | head -n 0").await;
    assert_eq!(code, 0, "stderr: {stderr}");
    assert!(stdout.trim().is_empty());
}

#[tokio::test]
async fn test_head_negative_lines() {
    // -n -N prints all but the last N lines
    let (code, stdout, stderr) = run_sh("seq 1 10 | head -n -3").await;
    assert_eq!(code, 0, "stderr: {stderr}");
    let lines: Vec<&str> = stdout.trim().lines().collect();
    assert_eq!(lines, vec!["1", "2", "3", "4", "5", "6", "7"]);
}

#[tokio::test]
async fn test_head_bytes() {
    let (code, stdout, stderr) = run_sh("printf 'hello world from fusion' | head -c 11").await;
    assert_eq!(code, 0, "stderr: {stderr}");
    assert_eq!(stdout, "hello world");
}

#[tokio::test]
async fn test_head_negative_bytes() {
    // -c -N prints all but the last N bytes
    let (code, stdout, stderr) = run_sh("printf 'hello world!' | head -c -1").await;
    assert_eq!(code, 0, "stderr: {stderr}");
    assert_eq!(stdout, "hello world");
}

#[tokio::test]
async fn test_head_obsolete_num_syntax() {
    // POSIX obsolete syntax `head -5` should rewrite to `head -n 5`
    let (code, stdout, stderr) = run_sh("seq 1 20 | head -5").await;
    assert_eq!(code, 0, "stderr: {stderr}");
    let lines: Vec<&str> = stdout.trim().lines().collect();
    assert_eq!(lines, vec!["1", "2", "3", "4", "5"]);
}

#[tokio::test]
async fn test_head_file_operand_and_verbose() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let file_path = tmp.path().join("data.txt");
    fs::write(&file_path, "first\nsecond\nthird\nfourth\n").expect("write data.txt");

    // Standard single file has no header
    let (code, stdout, stderr) = run_sh_in(tmp.path(), "head -n 2 data.txt").await;
    assert_eq!(code, 0, "stderr: {stderr}");
    let lines: Vec<&str> = stdout.trim().lines().collect();
    assert_eq!(lines, vec!["first", "second"]);

    // With -v, header is always shown even for a single file
    let (code_v, stdout_v, stderr_v) = run_sh_in(tmp.path(), "head -v -n 2 data.txt").await;
    assert_eq!(code_v, 0, "stderr: {stderr_v}");
    assert!(
        stdout_v.contains("==> data.txt <=="),
        "expected verbose header: {stdout_v}"
    );
}

#[tokio::test]
async fn test_head_multiple_files_with_headers_and_quiet() {
    let tmp = tempfile::tempdir().expect("tempdir");
    fs::write(tmp.path().join("f1.txt"), "line1_1\nline1_2\n").expect("write f1");
    fs::write(tmp.path().join("f2.txt"), "line2_1\nline2_2\n").expect("write f2");

    // Without -q, multiple files include ==> filename <== headers
    let (code, stdout, stderr) = run_sh_in(tmp.path(), "head -n 1 f1.txt f2.txt").await;
    assert_eq!(code, 0, "stderr: {stderr}");
    assert!(
        stdout.contains("==> f1.txt <=="),
        "expected header for f1.txt: {stdout}"
    );
    assert!(
        stdout.contains("==> f2.txt <=="),
        "expected header for f2.txt: {stdout}"
    );
    assert!(stdout.contains("line1_1"));
    assert!(stdout.contains("line2_1"));

    // With -q, headers are suppressed
    let (code_q, stdout_q, stderr_q) =
        run_sh_in(tmp.path(), "head -q -n 1 f1.txt f2.txt").await;
    assert_eq!(code_q, 0, "stderr: {stderr_q}");
    assert!(
        !stdout_q.contains("==>"),
        "expected no header with -q: {stdout_q}"
    );
    let lines_q: Vec<&str> = stdout_q.trim().lines().collect();
    assert_eq!(lines_q, vec!["line1_1", "line2_1"]);
}

// ============================================================================
// 5. Combined Pipeline Invariants
// ============================================================================

#[tokio::test]
async fn test_combined_wc_cut_head_pipeline() {
    // Pipeline chaining: generate lines, filter fields with cut, truncate with head, count with wc
    let cmd = "printf 'alice:dev:100\\nbob:qa:200\\ncharlie:dev:300\\ndave:ops:400\\n' \
               | cut -d ':' -f 1,2 \
               | head -n 3 \
               | wc -l";
    let (code, stdout, stderr) = run_sh(cmd).await;
    assert_eq!(code, 0, "stderr: {stderr}");
    assert_eq!(stdout.trim(), "3");
}
