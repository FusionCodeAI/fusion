use fusion::tools::default_registry;
use fusion::tools::lsp::LspTool;
use fusion::tools::types::{Tool, ToolContext};
use serde_json::json;

#[test]
fn test_default_registry_contains_lsp() {
    let registry = default_registry();
    assert!(
        registry.get("lsp").is_some(),
        "default_registry() must register 'lsp' tool"
    );
    let tool = registry.get("lsp").unwrap();
    assert_eq!(tool.name(), "lsp");
}

#[test]
fn test_lsp_tool_schema_properties() {
    let tool = LspTool::new();
    assert_eq!(tool.name(), "lsp");
    assert!(
        tool.description().contains("language servers"),
        "description should describe querying language servers"
    );

    let params = tool.parameters();
    let properties = params
        .get("properties")
        .expect("parameters schema must have 'properties'");

    // Verify tool schema contains "action" and "file"
    assert!(
        properties.get("action").is_some(),
        "schema must contain 'action'"
    );
    assert!(
        properties.get("file").is_some(),
        "schema must contain 'file'"
    );

    // Verify action enum contains expected values
    let action_prop = properties.get("action").unwrap();
    let enums = action_prop
        .get("enum")
        .and_then(|v| v.as_array())
        .expect("action must have enum list");
    let enum_strs: Vec<&str> = enums.iter().filter_map(|v| v.as_str()).collect();
    assert!(enum_strs.contains(&"definition"));
    assert!(enum_strs.contains(&"references"));
    assert!(enum_strs.contains(&"type_definition"));
    assert!(enum_strs.contains(&"diagnostics"));
    assert!(enum_strs.contains(&"symbols"));

    // Verify required fields contain "action" and "file"
    let required = params
        .get("required")
        .and_then(|v| v.as_array())
        .expect("schema must have 'required'");
    let req_strs: Vec<&str> = required.iter().filter_map(|v| v.as_str()).collect();
    assert!(req_strs.contains(&"action"));
    assert!(req_strs.contains(&"file"));
}

#[tokio::test]
async fn test_lsp_definition_nonexistent_server_fallback() {
    let tool = LspTool::new();
    let ctx = ToolContext::default();

    // 1. Non-existent file extension (no server configured)
    let args = json!({
        "action": "definition",
        "file": "nonexistent_module_xyz_123.fakeext",
        "line": 10
    });
    let res = tool.execute(args, &ctx).await;
    assert!(
        res.is_ok(),
        "executing definition with non-existent server should return Ok without panicking"
    );
    let output = res.unwrap();
    assert!(
        output.contains("[LSP Fallback]"),
        "expected fallback message, got: {output}"
    );

    // 2. Existing file with symbol fallback
    let args_with_sym = json!({
        "action": "definition",
        "file": "tests/lsp_tool_test.rs",
        "line": 1,
        "symbol": "test_default_registry_contains_lsp"
    });
    let res_sym = tool.execute(args_with_sym, &ctx).await;
    assert!(
        res_sym.is_ok(),
        "definition with symbol should succeed without panicking"
    );
    let output_sym = res_sym.unwrap();
    assert!(!output_sym.is_empty(), "output should not be empty");
}

#[tokio::test]
async fn test_lsp_symbols_action_fallback() {
    let tool = LspTool::new();
    let ctx = ToolContext::default();

    let args = json!({
        "action": "symbols",
        "file": "tests/lsp_tool_test.rs"
    });
    let res = tool.execute(args, &ctx).await;
    assert!(res.is_ok());
    let output = res.unwrap();
    assert!(!output.is_empty());
}

#[tokio::test]
async fn test_lsp_diagnostics_action_fallback() {
    let tool = LspTool::new();
    let ctx = ToolContext::default();

    let args = json!({
        "action": "diagnostics",
        "file": "tests/lsp_tool_test.rs"
    });
    let res = tool.execute(args, &ctx).await;
    assert!(res.is_ok());
    let output = res.unwrap();
    assert!(!output.is_empty());
}
