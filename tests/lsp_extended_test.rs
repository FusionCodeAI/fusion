//! Extended integration tests for `src/tools/lsp/`:
//! - Test fallback behavior when server fails
//! - Test format_locations and format_lsp_diagnostics
//! - Test symbol extraction and URI conversion helpers

use std::path::{Path, PathBuf};
use serde_json::json;
use fusion::tools::lsp::{
    extract_identifier_at_pos, extract_symbol_from_file, format_locations, format_lsp_diagnostics,
    format_lsp_symbols, parse_locations_response, uri_to_path_buf, LspTool,
};
use fusion::tools::lsp::client::path_to_uri;
use fusion::tools::types::{Tool, ToolContext};

// =========================================================================
// 1. format_locations Tests
// =========================================================================

#[test]
fn test_format_locations_empty() {
    let cwd = Path::new("/workspace");
    let result = format_locations(&[], cwd);
    assert_eq!(result, "No locations found.");
}

#[test]
fn test_format_locations_single_standard_location() {
    let cwd = Path::new("/workspace");
    let loc = json!({
        "uri": "file:///workspace/src/lib.rs",
        "range": {
            "start": { "line": 41, "character": 7 },
            "end": { "line": 41, "character": 15 }
        }
    });

    let result = format_locations(&[loc], cwd);
    // Relative path should be "src/lib.rs", 1-indexed: line 42, col 8
    assert!(
        result.contains("src/lib.rs:42:8"),
        "expected 'src/lib.rs:42:8' in formatted output, got: {result}"
    );
}

#[test]
fn test_format_locations_relative_path_and_file_preview() {
    // Use an actual existing file in the project to verify line preview extraction
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let target_file = manifest_dir.join("tests/lsp_tool_test.rs");
    assert!(target_file.exists(), "tests/lsp_tool_test.rs must exist");

    let file_uri = path_to_uri(&target_file);
    let loc = json!({
        "uri": file_uri,
        "range": {
            "start": { "line": 0, "character": 0 },
            "end": { "line": 0, "character": 10 }
        }
    });

    let result = format_locations(&[loc], &manifest_dir);
    // Line 0 is line 1 in 1-indexed: tests/lsp_tool_test.rs:1:1
    assert!(
        result.contains("tests/lsp_tool_test.rs:1:1"),
        "expected relative path 'tests/lsp_tool_test.rs:1:1' in: {result}"
    );
    // The first line in tests/lsp_tool_test.rs contains "use fusion::tools"
    assert!(
        result.contains("use fusion::tools"),
        "expected line preview to contain 'use fusion::tools', got: {result}"
    );
}

#[test]
fn test_format_locations_multiple_locations() {
    let cwd = Path::new("/workspace");
    let loc1 = json!({
        "uri": "file:///workspace/src/a.rs",
        "range": { "start": { "line": 9, "character": 0 } }
    });
    let loc2 = json!({
        "uri": "file:///workspace/src/b.rs",
        "range": { "start": { "line": 19, "character": 4 } }
    });

    let result = format_locations(&[loc1, loc2], cwd);
    let lines: Vec<&str> = result.lines().collect();
    assert_eq!(lines.len(), 2);
    assert!(lines[0].contains("src/a.rs:10:1"));
    assert!(lines[1].contains("src/b.rs:20:5"));
}

#[test]
fn test_format_locations_location_link_spec() {
    // LSP 3.14+ LocationLink uses targetUri and targetSelectionRange / targetRange
    let cwd = Path::new("/workspace");
    let link = json!({
        "targetUri": "file:///workspace/src/model.rs",
        "targetSelectionRange": {
            "start": { "line": 100, "character": 12 },
            "end": { "line": 100, "character": 20 }
        },
        "targetRange": {
            "start": { "line": 95, "character": 0 },
            "end": { "line": 110, "character": 1 }
        }
    });

    let result = format_locations(&[link], cwd);
    assert!(
        result.contains("src/model.rs:101:13"),
        "LocationLink should format targetUri and targetSelectionRange, got: {result}"
    );
}

#[test]
fn test_format_locations_non_file_uri() {
    let cwd = Path::new("/workspace");
    let loc = json!({
        "uri": "untitled:Untitled-1",
        "range": { "start": { "line": 5, "character": 2 } }
    });

    let result = format_locations(&[loc], cwd);
    assert!(
        result.contains("untitled:Untitled-1:6:3"),
        "non-file URI should be preserved as-is, got: {result}"
    );
}

#[test]
fn test_parse_locations_response() {
    // Null response
    assert!(parse_locations_response(json!(null)).is_empty());

    // Single object response
    let single = json!({
        "uri": "file:///test.rs",
        "range": { "start": { "line": 0, "character": 0 } }
    });
    let parsed_single = parse_locations_response(single.clone());
    assert_eq!(parsed_single.len(), 1);
    assert_eq!(parsed_single[0], single);

    // Array response
    let arr = json!([
        { "uri": "file:///test1.rs" },
        { "uri": "file:///test2.rs" }
    ]);
    let parsed_arr = parse_locations_response(arr);
    assert_eq!(parsed_arr.len(), 2);

    // Unsupported value type
    assert!(parse_locations_response(json!(12345)).is_empty());
}

// =========================================================================
// 2. format_lsp_diagnostics Tests
// =========================================================================

#[test]
fn test_format_lsp_diagnostics_empty_cases() {
    // Null value
    let res_null = format_lsp_diagnostics(&json!(null), "src/main.rs");
    assert_eq!(res_null, "No diagnostics reported for 'src/main.rs'.");

    // Empty object
    let res_empty_obj = format_lsp_diagnostics(&json!({}), "src/main.rs");
    assert_eq!(res_empty_obj, "No diagnostics reported for 'src/main.rs'.");

    // Empty items array
    let res_empty_items = format_lsp_diagnostics(&json!({ "items": [] }), "src/main.rs");
    assert_eq!(res_empty_items, "No diagnostics reported for 'src/main.rs'.");

    // Empty array directly
    let res_empty_arr = format_lsp_diagnostics(&json!([]), "src/main.rs");
    assert_eq!(res_empty_arr, "No diagnostics reported for 'src/main.rs'.");
}

#[test]
fn test_format_lsp_diagnostics_severities() {
    let diag = json!({
        "items": [
            {
                "message": "syntax error",
                "severity": 1,
                "range": { "start": { "line": 0, "character": 0 } }
            },
            {
                "message": "unused variable",
                "severity": 2,
                "range": { "start": { "line": 1, "character": 4 } }
            },
            {
                "message": "type inferred",
                "severity": 3,
                "range": { "start": { "line": 2, "character": 8 } }
            },
            {
                "message": "can be simplified",
                "severity": 4,
                "range": { "start": { "line": 3, "character": 12 } }
            },
            {
                "message": "custom unknown severity",
                "severity": 99,
                "range": { "start": { "line": 4, "character": 0 } }
            }
        ]
    });

    let result = format_lsp_diagnostics(&diag, "src/lib.rs");
    let lines: Vec<&str> = result.lines().collect();
    assert_eq!(lines.len(), 5);

    // Severity 1 -> error
    assert_eq!(lines[0], "src/lib.rs:1:1: error: syntax error");
    // Severity 2 -> warning
    assert_eq!(lines[1], "src/lib.rs:2:5: warning: unused variable");
    // Severity 3 -> info
    assert_eq!(lines[2], "src/lib.rs:3:9: info: type inferred");
    // Severity 4 -> hint
    assert_eq!(lines[3], "src/lib.rs:4:13: hint: can be simplified");
    // Other -> diagnostic
    assert_eq!(lines[4], "src/lib.rs:5:1: diagnostic: custom unknown severity");
}

#[test]
fn test_format_lsp_diagnostics_with_source() {
    let diag = json!({
        "items": [
            {
                "message": "cannot find value `foo`",
                "severity": 1,
                "source": "rustc",
                "range": { "start": { "line": 24, "character": 10 } }
            },
            {
                "message": "variable does not need to be mutable",
                "severity": 2,
                "source": "clippy",
                "range": { "start": { "line": 30, "character": 8 } }
            }
        ]
    });

    let result = format_lsp_diagnostics(&diag, "src/parser.rs");
    assert!(
        result.contains("src/parser.rs:25:11: [rustc] error: cannot find value `foo`"),
        "expected rustc source prefix, got: {result}"
    );
    assert!(
        result.contains("src/parser.rs:31:9: [clippy] warning: variable does not need to be mutable"),
        "expected clippy source prefix, got: {result}"
    );
}

#[test]
fn test_format_lsp_diagnostics_bare_array_format() {
    // Diagnostic list provided as top-level array
    let diag = json!([
        {
            "message": "unreachable code",
            "severity": 2,
            "range": { "start": { "line": 10, "character": 2 } }
        }
    ]);

    let result = format_lsp_diagnostics(&diag, "src/main.rs");
    assert_eq!(result, "src/main.rs:11:3: warning: unreachable code");
}

#[test]
fn test_format_lsp_symbols_formatting() {
    let syms = json!([
        {
            "name": "Config",
            "kind": 23, // struct
            "range": { "start": { "line": 10, "character": 0 } },
            "children": [
                {
                    "name": "timeout",
                    "kind": 8, // field
                    "range": { "start": { "line": 11, "character": 4 } }
                }
            ]
        },
        {
            "name": "execute",
            "kind": 12, // function
            "range": { "start": { "line": 20, "character": 0 } }
        }
    ]);

    let result = format_lsp_symbols(&syms);
    assert!(result.contains("[struct] Config (line 11)"));
    assert!(result.contains("  [field] timeout (line 12)"));
    assert!(result.contains("[function] execute (line 21)"));

    // Empty symbols array
    let empty_result = format_lsp_symbols(&json!([]));
    assert_eq!(empty_result, "No symbols returned by LSP server.");
}

// =========================================================================
// 3. Fallback Behavior When Server Fails
// =========================================================================

#[tokio::test]
async fn test_fallback_definition_nonexistent_file() {
    let tool = LspTool::new();
    let ctx = ToolContext::default();

    let args = json!({
        "action": "definition",
        "file": "missing_file_for_test.fake_ext",
        "line": 42
    });

    let res = tool.execute(args, &ctx).await;
    assert!(res.is_ok(), "fallback should not fail with Err");
    let out = res.unwrap();
    assert!(out.contains("[LSP Fallback]"));
    assert!(out.contains("No LSP server configured for 'missing_file_for_test.fake_ext'"));
    assert!(out.contains("No symbol specified or found at line 42"));
}

#[tokio::test]
async fn test_fallback_definition_with_explicit_symbol() {
    let tool = LspTool::new();
    let ctx = ToolContext::default();

    let args = json!({
        "action": "definition",
        "file": "tests/lsp_tool_test.rs",
        "symbol": "test_default_registry_contains_lsp"
    });

    let res = tool.execute(args, &ctx).await;
    assert!(res.is_ok());
    let out = res.unwrap();
    // In fallback, should either locate the symbol in tests/lsp_tool_test.rs or report fallback status
    assert!(out.contains("[LSP Fallback"));
    assert!(out.contains("test_default_registry_contains_lsp"));
}

#[tokio::test]
async fn test_fallback_references_with_symbol() {
    let tool = LspTool::new();
    let ctx = ToolContext::default();

    let args = json!({
        "action": "references",
        "file": "tests/lsp_tool_test.rs",
        "symbol": "test_lsp_tool_schema_properties"
    });

    let res = tool.execute(args, &ctx).await;
    assert!(res.is_ok());
    let out = res.unwrap();
    assert!(out.contains("[LSP Fallback"));
    assert!(out.contains("test_lsp_tool_schema_properties"));
}

#[tokio::test]
async fn test_fallback_references_without_symbol() {
    let tool = LspTool::new();
    let ctx = ToolContext::default();

    // No symbol provided and non-existent file
    let args = json!({
        "action": "references",
        "file": "nonexistent_ref_file.xyz"
    });

    let res = tool.execute(args, &ctx).await;
    assert!(res.is_ok());
    let out = res.unwrap();
    assert!(out.contains("[LSP Fallback]"));
    assert!(out.contains("No symbol specified for references lookup"));
}

#[tokio::test]
async fn test_fallback_type_definition_with_symbol() {
    let tool = LspTool::new();
    let ctx = ToolContext::default();

    let args = json!({
        "action": "type_definition",
        "file": "tests/lsp_tool_test.rs",
        "symbol": "LspTool"
    });

    let res = tool.execute(args, &ctx).await;
    assert!(res.is_ok());
    let out = res.unwrap();
    assert!(out.contains("[LSP Fallback"));
    assert!(out.contains("LspTool"));
}

#[tokio::test]
async fn test_fallback_type_definition_without_symbol() {
    let tool = LspTool::new();
    let ctx = ToolContext::default();

    let args = json!({
        "action": "type_definition",
        "file": "nonexistent_typedef_file.xyz"
    });

    let res = tool.execute(args, &ctx).await;
    assert!(res.is_ok());
    let out = res.unwrap();
    assert!(out.contains("[LSP Fallback]"));
    assert!(out.contains("No symbol specified for type definition lookup"));
}

#[tokio::test]
async fn test_fallback_diagnostics_existing_file() {
    let tool = LspTool::new();
    let ctx = ToolContext::default();

    let args = json!({
        "action": "diagnostics",
        "file": "tests/lsp_tool_test.rs"
    });

    let res = tool.execute(args, &ctx).await;
    assert!(res.is_ok());
    let out = res.unwrap();
    // For an existing file, diagnostics fall back to SyntaxCheckTool
    assert!(
        out.contains("[LSP Fallback -> Syntax]") || out.contains("[LSP Fallback]"),
        "expected fallback message, got: {out}"
    );
}

#[tokio::test]
async fn test_fallback_diagnostics_workspace_wildcard() {
    let tool = LspTool::new();
    let ctx = ToolContext::default();

    // Diagnostics on '*' requires a specific file or active language server
    let args = json!({
        "action": "diagnostics",
        "file": "*"
    });

    let res = tool.execute(args, &ctx).await;
    assert!(res.is_ok());
    let out = res.unwrap();
    assert!(out.contains("[LSP Fallback]"));
    assert!(out.contains("Diagnostics require a specific file or active language server"));
}

#[tokio::test]
async fn test_fallback_symbols_action() {
    let tool = LspTool::new();
    let ctx = ToolContext::default();

    let args = json!({
        "action": "symbols",
        "file": "tests/lsp_tool_test.rs",
        "symbol": "test_lsp"
    });

    let res = tool.execute(args, &ctx).await;
    assert!(res.is_ok());
    let out = res.unwrap();
    assert!(out.contains("[LSP Fallback -> Symbols]"));
}

#[tokio::test]
async fn test_unsupported_action_error() {
    let tool = LspTool::new();
    let ctx = ToolContext::default();

    let args = json!({
        "action": "hover",
        "file": "tests/lsp_tool_test.rs"
    });

    let res = tool.execute(args, &ctx).await;
    assert!(res.is_err(), "unsupported action 'hover' should fail");
    let err_str = res.unwrap_err().to_string();
    assert!(err_str.contains("Unsupported action: hover"));
}

#[tokio::test]
async fn test_missing_action_parameter() {
    let tool = LspTool::new();
    let ctx = ToolContext::default();

    let args = json!({
        "file": "tests/lsp_tool_test.rs"
    });

    let res = tool.execute(args, &ctx).await;
    assert!(res.is_err());
    let err = res.unwrap_err().to_string();
    assert!(err.contains("Missing required parameter: 'action'"));
}

#[tokio::test]
async fn test_missing_file_parameter() {
    let tool = LspTool::new();
    let ctx = ToolContext::default();

    let args = json!({
        "action": "definition"
    });

    let res = tool.execute(args, &ctx).await;
    assert!(res.is_err());
    let err = res.unwrap_err().to_string();
    assert!(err.contains("Missing required parameter: 'file'"));
}

// =========================================================================
// 4. Helper Function Tests
// =========================================================================

#[test]
fn test_uri_to_path_buf_helper() {
    let uri = "file:///tmp/test_dir/main.rs";
    let pb = uri_to_path_buf(uri).expect("should parse file:// URI");
    assert!(pb.to_string_lossy().contains("test_dir"));

    let non_file = "http://example.com/test.rs";
    assert!(uri_to_path_buf(non_file).is_none());
}

#[test]
fn test_extract_identifier_at_pos_helper() {
    let line = "    let my_special_ident = compute_value(42);";
    // Point inside `my_special_ident` (index 10)
    let ident = extract_identifier_at_pos(line, 10).expect("should extract identifier");
    assert_eq!(ident, "my_special_ident");

    // Point inside `compute_value` (index 28)
    let ident_func = extract_identifier_at_pos(line, 28).expect("should extract function name");
    assert_eq!(ident_func, "compute_value");

    // Whitespace only
    assert!(extract_identifier_at_pos("    \t  ", 2).is_none());
}

#[test]
fn test_extract_symbol_from_file_helper() {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let target_file = manifest_dir.join("tests/lsp_tool_test.rs");

    // Line 1 is 1-indexed, character 10 is inside `fusion`
    let sym = extract_symbol_from_file(&target_file, 1, 10);
    assert!(sym.is_some());

    // Non-existent file
    let no_file = Path::new("non_existent_file_xyz_123.rs");
    assert!(extract_symbol_from_file(no_file, 1, 1).is_none());
}
