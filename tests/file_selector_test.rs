//! Tests for the enhanced `read` tool selectors:
//! - `:start-end` line slicing (e.g. `:2-4`)
//! - `:raw` verbatim output
//! - Line number formatting
//! - `:defs` structural AST outline
//! - Automatic summary on oversized (>500 lines) files

use fusion::tools::file::ReadFileTool;
use fusion::tools::types::{Tool, ToolContext};
use serde_json::json;
use std::collections::HashMap;
use std::fs;
use tempfile::tempdir;

fn setup_test_context() -> (tempfile::TempDir, ToolContext, ReadFileTool) {
    let dir = tempdir().expect("Failed to create tempdir");
    let ctx = ToolContext {
        cwd: dir.path().to_path_buf(),
        env: HashMap::new(),
    };
    let tool = ReadFileTool::new();
    (dir, ctx, tool)
}

#[tokio::test]
async fn test_read_line_slicing_start_end() {
    let (dir, ctx, tool) = setup_test_context();
    let file_path = dir.path().join("sample.txt");
    let sample_content = "Line 1\nLine 2\nLine 3\nLine 4\nLine 5\nLine 6\n";
    fs::write(&file_path, sample_content).expect("Failed to write sample file");

    // Test slicing lines 2 through 4 inclusive via `:2-4` selector
    let result = tool
        .execute(
            json!({
                "path": "sample.txt:2-4"
            }),
            &ctx,
        )
        .await
        .expect("Read tool execution failed for :2-4 selector");

    // Must contain lines 2, 3, and 4
    assert!(result.contains("Line 2"), "Result should contain Line 2: {result}");
    assert!(result.contains("Line 3"), "Result should contain Line 3: {result}");
    assert!(result.contains("Line 4"), "Result should contain Line 4: {result}");

    // Must not contain lines 1, 5, or 6
    assert!(!result.contains("Line 1"), "Result should not contain Line 1: {result}");
    assert!(!result.contains("Line 5"), "Result should not contain Line 5: {result}");
    assert!(!result.contains("Line 6"), "Result should not contain Line 6: {result}");
}

#[tokio::test]
async fn test_read_line_number_formatting() {
    let (dir, ctx, tool) = setup_test_context();
    let file_path = dir.path().join("numbered.txt");
    let sample_content = "Alpha\nBeta\nGamma\nDelta\nEpsilon\n";
    fs::write(&file_path, sample_content).expect("Failed to write sample file");

    // Read lines 2 to 4 with default line numbers
    let result = tool
        .execute(
            json!({
                "path": "numbered.txt:2-4"
            }),
            &ctx,
        )
        .await
        .expect("Read tool execution failed");

    // Line numbers should follow "{:6} | <line>" format with 1-based original line indexing
    assert!(result.contains("     2 | Beta"), "Line 2 formatting mismatch: {result}");
    assert!(result.contains("     3 | Gamma"), "Line 3 formatting mismatch: {result}");
    assert!(result.contains("     4 | Delta"), "Line 4 formatting mismatch: {result}");

    // Whole file read formatting
    let whole_result = tool
        .execute(
            json!({
                "path": "numbered.txt"
            }),
            &ctx,
        )
        .await
        .expect("Read tool execution failed for whole file");

    assert!(whole_result.contains("     1 | Alpha"), "Line 1 formatting mismatch: {whole_result}");
    assert!(whole_result.contains("     5 | Epsilon"), "Line 5 formatting mismatch: {whole_result}");
}

#[tokio::test]
async fn test_read_raw_verbatim_output() {
    let (dir, ctx, tool) = setup_test_context();
    let file_path = dir.path().join("raw_test.txt");
    let sample_content = "First line\nSecond line\nThird line\nFourth line\n";
    fs::write(&file_path, sample_content).expect("Failed to write sample file");

    // Test `:raw` selector on entire file
    let result = tool
        .execute(
            json!({
                "path": "raw_test.txt:raw"
            }),
            &ctx,
        )
        .await
        .expect("Read tool execution failed for :raw selector");

    // Should match verbatim content without any line number prefixes (no " | ")
    assert_eq!(result, sample_content, "Raw output should be exact verbatim content");
    assert!(!result.contains("     1 | "), "Raw output must not contain line number formatting");
}

#[tokio::test]
async fn test_read_combined_slicing_and_raw() {
    let (dir, ctx, tool) = setup_test_context();
    let file_path = dir.path().join("combined.txt");
    let sample_content = "Row 1\nRow 2\nRow 3\nRow 4\nRow 5\n";
    fs::write(&file_path, sample_content).expect("Failed to write sample file");

    // Test `:2-4:raw` combined selector
    let result = tool
        .execute(
            json!({
                "path": "combined.txt:2-4:raw"
            }),
            &ctx,
        )
        .await
        .expect("Read tool execution failed for :2-4:raw selector");

    // Verbatim slice of rows 2, 3, 4 without line number formatting
    assert_eq!(result, "Row 2\nRow 3\nRow 4\n", "Combined slice and raw should return exact sliced lines");
    assert!(!result.contains(" | "), "Combined slice and raw must not contain line number formatting");
}

#[tokio::test]
async fn test_read_start_offset_selector() {
    let (dir, ctx, tool) = setup_test_context();
    let file_path = dir.path().join("offset.txt");
    let sample_content = "Line 1\nLine 2\nLine 3\nLine 4\nLine 5\n";
    fs::write(&file_path, sample_content).expect("Failed to write sample file");

    // Test `:3` selector (starting at line 3 to EOF)
    let result = tool
        .execute(
            json!({
                "path": "offset.txt:3"
            }),
            &ctx,
        )
        .await
        .expect("Read tool execution failed for :3 selector");

    assert!(!result.contains("Line 1"), "Should not contain Line 1");
    assert!(!result.contains("Line 2"), "Should not contain Line 2");
    assert!(result.contains("     3 | Line 3"), "Should contain Line 3 with line number 3");
    assert!(result.contains("     4 | Line 4"), "Should contain Line 4 with line number 4");
    assert!(result.contains("     5 | Line 5"), "Should contain Line 5 with line number 5");
}

#[tokio::test]
async fn test_read_single_line_slice() {
    let (dir, ctx, tool) = setup_test_context();
    let file_path = dir.path().join("single.txt");
    let sample_content = "Line 1\nLine 2\nLine 3\n";
    fs::write(&file_path, sample_content).expect("Failed to write sample file");

    // Test `:2-2` single line slice
    let result = tool
        .execute(
            json!({
                "path": "single.txt:2-2"
            }),
            &ctx,
        )
        .await
        .expect("Read tool execution failed for :2-2 selector");

    assert!(!result.contains("Line 1"));
    assert!(result.contains("     2 | Line 2"));
    assert!(!result.contains("Line 3"));
}

#[tokio::test]
async fn test_read_defs_outline() {
    let (dir, ctx, tool) = setup_test_context();
    let file_path = dir.path().join("code.rs");
    let code = r#"// Header comment
pub fn compute_sum(a: i32, b: i32) -> i32 {
    let x = a * 2;
    let y = b * 3;
    let z = x + y;
    z
}

pub struct UserRecord {
    pub id: u64,
    pub username: String,
    pub email: String,
}
"#;
    fs::write(&file_path, code).expect("Failed to write code.rs");

    // Test `:defs` selector
    let result = tool
        .execute(
            json!({
                "path": "code.rs:defs"
            }),
            &ctx,
        )
        .await
        .expect("Read tool execution failed for :defs selector");

    // Must retain declarations
    assert!(result.contains("compute_sum"), "Output should contain function declaration: {result}");
    assert!(result.contains("UserRecord"), "Output should contain struct declaration: {result}");
    // Must contain elision marker `...`
    assert!(result.contains("..."), "Output should contain elision marker `...`: {result}");
}

#[tokio::test]
async fn test_read_oversized_file_automatic_summary() {
    let (dir, ctx, tool) = setup_test_context();
    let file_path = dir.path().join("large_module.rs");

    // Generate a file with >500 lines containing functions
    let mut large_content = String::new();
    large_content.push_str("// Large source module\n\n");
    for i in 0..110 {
        large_content.push_str(&format!(
            "pub fn function_{i}() -> usize {{\n    let val = {i} * 2;\n    let res = val + 1;\n    let fin = res * 3;\n    fin\n}}\n\n"
        ));
    }
    // Verify line count exceeds 500
    let line_count = large_content.lines().count();
    assert!(line_count > 500, "Test file should exceed 500 lines, got {line_count}");
    fs::write(&file_path, &large_content).expect("Failed to write large_module.rs");

    // Reading without explicit selectors or limits triggers automatic summary
    let result = tool
        .execute(
            json!({
                "path": "large_module.rs"
            }),
            &ctx,
        )
        .await
        .expect("Read tool execution failed for oversized file");

    assert!(result.contains("Summary:"), "Oversized file should contain Summary banner: {result}");
    assert!(
        result.contains("lines elided; re-issue with line range selector"),
        "Oversized file should guide re-issue with line range selector: {result}"
    );
}
