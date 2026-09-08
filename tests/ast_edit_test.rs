//! Integration tests for the `ast_edit` tool.
//!
//! Tests:
//! 1. Tool metadata, name, description, and JSON schema.
//! 2. Pattern matching and template replacement with metavariables across languages (Rust, JS, Python, Go).
//! 3. Argument swapping, spread metavariables ($$$ARGS, $$$BODY), and multi-op rewrites.
//! 4. Tool execution against files on disk (single and multiple files).
//! 5. Error handling for unsupported languages, invalid patterns, and missing files.

use fusion::tools::ast_edit::*;
use fusion::tools::types::{Tool, ToolContext};
use serde_json::json;
use std::collections::HashMap;
use tempfile::tempdir;

// ---------------------------------------------------------------------------
// Metadata & Schema Tests
// ---------------------------------------------------------------------------

#[test]
fn test_tool_metadata() {
    let tool = AstEditTool::new();
    assert_eq!(tool.name(), "ast_edit");
    assert!(tool
        .description()
        .contains("Structural syntax-aware code rewriting"));
    assert!(tool.description().contains("tree-sitter AST patterns"));

    let schema = tool.parameters();
    assert_eq!(schema["type"], "object");
    assert!(schema["properties"]["paths"].is_object());
    assert!(schema["properties"]["ops"].is_object());

    let required = schema["required"].as_array().expect("required array");
    assert!(required.iter().any(|v| v == "paths"));
    assert!(required.iter().any(|v| v == "ops"));
}

// ---------------------------------------------------------------------------
// Rust Pattern Rewriting Tests
// ---------------------------------------------------------------------------

#[test]
fn test_rust_function_renaming_with_body_capture() {
    let source = r#"
fn calculate_total(a: i32, b: i32) -> i32 {
    let sum = a + b;
    sum * 2
}
"#;

    let pat = "fn calculate_total($$$ARGS) -> i32 { $$$BODY }";
    let template = "fn compute_total($$$ARGS) -> i32 { $$$BODY }";

    let result = apply_rewrite(source, "rust", pat, template).expect("rewrite should succeed");

    assert!(result.contains("fn compute_total(a: i32, b: i32) -> i32 {"));
    assert!(result.contains("let sum = a + b;"));
    assert!(!result.contains("fn calculate_total("));
}

#[test]
fn test_rust_call_expression_metavariables() {
    let source = r#"
fn main() {
    process_values(100, 200);
}
"#;

    let pat = "process_values($A, $B)";
    let template = "process_values($B, $A)";

    let result = apply_rewrite(source, "rust", pat, template).expect("rewrite should succeed");
    assert!(result.contains("process_values(200, 100)"));
}

#[test]
fn test_rust_via_tool_associated_function() {
    let source = "fn hello() { println!(\"world\"); }";
    let rewritten = AstEditTool::apply_rewrite(
        source,
        "rs",
        "fn hello() { $$$BODY }",
        "pub fn hello() { $$$BODY }",
    )
    .expect("rewrite should succeed");

    assert!(rewritten.contains("pub fn hello() {"));
}

// ---------------------------------------------------------------------------
// JavaScript & TypeScript Pattern Rewriting Tests
// ---------------------------------------------------------------------------

#[test]
fn test_javascript_console_log_to_warn() {
    let source = r#"
function run() {
    console.log("Starting server", port);
    start();
}
"#;

    let pat = "console.log($$$ARGS)";
    let template = "console.warn($$$ARGS)";

    let result =
        apply_rewrite(source, "javascript", pat, template).expect("rewrite should succeed");
    assert!(result.contains("console.warn(\"Starting server\", port)"));
    assert!(!result.contains("console.log"));
}

#[test]
fn test_typescript_require_to_import() {
    let source = r#"const express = require("express");"#;
    let pat = "const $MOD = require($PATH)";
    let template = "import $MOD from $PATH";

    let result =
        apply_rewrite(source, "typescript", pat, template).expect("rewrite should succeed");
    assert_eq!(result.trim(), r#"import express from "express";"#);
}

// ---------------------------------------------------------------------------
// Python Pattern Rewriting Tests
// ---------------------------------------------------------------------------

#[test]
fn test_python_function_definition_rewrite() {
    let source = "def greet(name):\n    return f\"Hello, {name}\"\n";
    let pat = "def greet($PARAM):\n    $$$BODY";
    let template = "def welcome($PARAM):\n    $$$BODY";

    let result = apply_rewrite(source, "python", pat, template).expect("rewrite should succeed");
    assert!(result.contains("def welcome(name):"));
    assert!(result.contains("return f\"Hello, {name}\""));
}

#[test]
fn test_python_call_argument_swap() {
    let source = "result = pair_up(first, second)\n";
    let pat = "pair_up($A, $B)";
    let template = "pair_up($B, $A)";

    let result = apply_rewrite(source, "py", pat, template).expect("rewrite should succeed");
    assert_eq!(result, "result = pair_up(second, first)\n");
}

// ---------------------------------------------------------------------------
// Multiple Operations & Chaining Tests
// ---------------------------------------------------------------------------

#[test]
fn test_apply_rewrites_multiple_ops() {
    let source = r#"
fn test() {
    let a = old_fn_one(1);
    let b = old_fn_two(2);
}
"#;

    let ops = vec![
        RewriteOp {
            pat: "old_fn_one($X)".to_string(),
            out: "new_fn_one($X)".to_string(),
        },
        RewriteOp {
            pat: "old_fn_two($Y)".to_string(),
            out: "new_fn_two($Y)".to_string(),
        },
    ];

    let (result, count) =
        apply_rewrites(source, "rust", &ops).expect("chained rewrites should succeed");

    assert_eq!(count, 2);
    assert!(result.contains("new_fn_one(1)"));
    assert!(result.contains("new_fn_two(2)"));
    assert!(!result.contains("old_fn_one"));
    assert!(!result.contains("old_fn_two"));
}

// ---------------------------------------------------------------------------
// Error Handling & Edge Cases
// ---------------------------------------------------------------------------

#[test]
fn test_unsupported_language_returns_error() {
    let source = "some random text";
    let err = apply_rewrite(source, "brainfuck_xyz", "foo", "bar").unwrap_err();
    match err {
        AstEditError::UnsupportedLanguage(lang) => {
            assert_eq!(lang, "brainfuck_xyz");
        }
        other => panic!("Expected UnsupportedLanguage, got {other:?}"),
    }
}

#[test]
fn test_no_matches_returns_unmodified_source() {
    let source = "fn foo() { let x = 42; }";
    let result = apply_rewrite(
        source,
        "rust",
        "fn nonexistent() { $$$ }",
        "fn replacement() {}",
    )
    .expect("unmatched pattern should return unmodified source without error");
    assert_eq!(result, source);
}

// ---------------------------------------------------------------------------
// Tool Execute Integration Tests (File I/O)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_tool_execute_single_file() {
    let dir = tempdir().expect("tempdir");
    let file_path = dir.path().join("main.rs");
    tokio::fs::write(&file_path, "fn alpha() {\n    println!(\"hello\");\n}\n")
        .await
        .expect("write test file");

    let tool = AstEditTool::new();
    let ctx = ToolContext {
        cwd: dir.path().to_path_buf(),
        env: HashMap::new(),
    };

    let args = json!({
        "paths": ["main.rs"],
        "ops": [{
            "pat": "fn alpha() { $$$BODY }",
            "out": "fn beta() { $$$BODY }"
        }]
    });

    let output = tool
        .execute(args, &ctx)
        .await
        .expect("execute should succeed");
    assert!(output.contains("Successfully applied 1 AST replacement"));
    assert!(output.contains("main.rs"));

    let updated_content = tokio::fs::read_to_string(&file_path)
        .await
        .expect("read updated file");
    assert!(updated_content.contains("fn beta() {"));
    assert!(!updated_content.contains("fn alpha() {"));
}

#[tokio::test]
async fn test_tool_execute_multiple_files() {
    let dir = tempdir().expect("tempdir");
    let file1 = dir.path().join("a.js");
    let file2 = dir.path().join("b.js");

    tokio::fs::write(&file1, "console.log(\"file1\");\n")
        .await
        .expect("write a.js");
    tokio::fs::write(&file2, "console.log(\"file2\");\n")
        .await
        .expect("write b.js");

    let tool = AstEditTool::new();
    let ctx = ToolContext {
        cwd: dir.path().to_path_buf(),
        env: HashMap::new(),
    };

    let args = json!({
        "paths": ["a.js", "b.js"],
        "ops": [{
            "pat": "console.log($ARG)",
            "out": "console.info($ARG)"
        }]
    });

    let output = tool
        .execute(args, &ctx)
        .await
        .expect("execute should succeed");
    assert!(output.contains("Successfully applied 2 AST replacement(s) across 2 file(s)"));

    let c1 = tokio::fs::read_to_string(&file1).await.expect("read a.js");
    let c2 = tokio::fs::read_to_string(&file2).await.expect("read b.js");

    assert!(c1.contains("console.info(\"file1\")"));
    assert!(c2.contains("console.info(\"file2\")"));
}

#[tokio::test]
async fn test_tool_execute_no_matches() {
    let dir = tempdir().expect("tempdir");
    let file = dir.path().join("app.py");
    tokio::fs::write(&file, "x = 10\n")
        .await
        .expect("write app.py");

    let tool = AstEditTool::new();
    let ctx = ToolContext {
        cwd: dir.path().to_path_buf(),
        env: HashMap::new(),
    };

    let args = json!({
        "paths": ["app.py"],
        "ops": [{
            "pat": "def missing_fn(): pass",
            "out": "def new_fn(): pass"
        }]
    });

    let output = tool
        .execute(args, &ctx)
        .await
        .expect("execute should succeed");
    assert!(output.contains("No matching AST patterns found"));
}

#[tokio::test]
async fn test_tool_execute_missing_file_error() {
    let dir = tempdir().expect("tempdir");
    let tool = AstEditTool::new();
    let ctx = ToolContext {
        cwd: dir.path().to_path_buf(),
        env: HashMap::new(),
    };

    let args = json!({
        "paths": ["nonexistent_file.rs"],
        "ops": [{
            "pat": "foo()",
            "out": "bar()"
        }]
    });

    let err = tool.execute(args, &ctx).await.unwrap_err();
    assert!(err.to_string().contains("File not found"));
}

#[tokio::test]
async fn test_tool_execute_missing_parameters_error() {
    let dir = tempdir().expect("tempdir");
    let tool = AstEditTool::new();
    let ctx = ToolContext {
        cwd: dir.path().to_path_buf(),
        env: HashMap::new(),
    };

    // Missing ops
    let args1 = json!({ "paths": ["foo.rs"] });
    assert!(tool.execute(args1, &ctx).await.is_err());

    // Missing paths
    let args2 = json!({ "ops": [{"pat": "a", "out": "b"}] });
    assert!(tool.execute(args2, &ctx).await.is_err());

    // Empty ops
    let args3 = json!({ "paths": ["foo.rs"], "ops": [] });
    assert!(tool.execute(args3, &ctx).await.is_err());
}
