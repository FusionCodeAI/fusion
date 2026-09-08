//! Integration tests for MCP server discovery and inspection.
//!
//! Tests parsing Cursor, VS Code, Claude, and Fusion MCP configuration formats,
//! validating environment variable sanitization, binary path validation,
//! comment stripping (JSONC), and tool execution.

use std::collections::HashMap;
use std::fs;
use std::path::Path;
use tempfile::tempdir;

use fusion::tools::mcp_discovery::{
    is_sensitive_variable, mask_credential_value, resolve_binary_path, sanitize_environment_map,
    strip_json_comments, validate_binary_path, McpDiscoveryEngine, McpDiscoveryTool,
    McpServerConfig,
};
use fusion::tools::types::{Tool, ToolContext};
use serde_json::json;

// ===========================================================================
// Format Parsing Tests: Cursor, VS Code, Claude, Fusion
// ===========================================================================

#[test]
fn test_parse_cursor_format() {
    let tmp = tempdir().expect("tempdir");
    let root = tmp.path();

    // Create .cursor/mcp.json
    let cursor_dir = root.join(".cursor");
    fs::create_dir_all(&cursor_dir).unwrap();
    let cursor_mcp = cursor_dir.join("mcp.json");

    let cursor_content = r#"{
        "mcpServers": {
            "weather-service": {
                "command": "node",
                "args": ["dist/index.js", "--port", "8080"],
                "env": {
                    "API_KEY": "cursor-secret-key-1234567890",
                    "PORT": "8080",
                    "DEBUG": "true"
                }
            }
        }
    }"#;
    fs::write(&cursor_mcp, cursor_content).unwrap();

    let configs = McpDiscoveryEngine::discover_workspace_only(root);
    assert_eq!(
        configs.len(),
        1,
        "Should discover 1 server from .cursor/mcp.json"
    );

    let cfg = &configs[0];
    assert_eq!(cfg.name, "weather-service");
    assert_eq!(cfg.command, "node");
    assert_eq!(cfg.args, vec!["dist/index.js", "--port", "8080"]);
    assert_eq!(cfg.source_file, cursor_mcp);

    // Environment sanitization verification
    assert_eq!(cfg.env.get("PORT").map(|s| s.as_str()), Some("8080"));
    assert_eq!(cfg.env.get("DEBUG").map(|s| s.as_str()), Some("true"));
    let masked_key = cfg.env.get("API_KEY").expect("API_KEY should be present");
    assert_ne!(
        masked_key, "cursor-secret-key-1234567890",
        "API_KEY must be sanitized"
    );
    assert!(masked_key.contains("***") || masked_key.contains("..."));
}

#[test]
fn test_parse_vscode_format_object() {
    let tmp = tempdir().expect("tempdir");
    let root = tmp.path();

    // Create .vscode/mcp.json with JSONC comments
    let vscode_dir = root.join(".vscode");
    fs::create_dir_all(&vscode_dir).unwrap();
    let vscode_mcp = vscode_dir.join("mcp.json");

    let vscode_content = r#"// VS Code MCP configuration with comments
    {
        /* Primary development servers */
        "servers": {
            "vscode-sqlite": {
                "command": "uvx",
                "args": ["mcp-server-sqlite", "--db", "workspace.db"],
                "env": {
                    "SQLITE_PATH": "/data/workspace.db"
                }
            }
        }
    }"#;
    fs::write(&vscode_mcp, vscode_content).unwrap();

    let configs = McpDiscoveryEngine::discover_workspace_only(root);
    assert_eq!(
        configs.len(),
        1,
        "Should discover 1 server from .vscode/mcp.json"
    );

    let cfg = &configs[0];
    assert_eq!(cfg.name, "vscode-sqlite");
    assert_eq!(cfg.command, "uvx");
    assert_eq!(cfg.args, vec!["mcp-server-sqlite", "--db", "workspace.db"]);
    assert_eq!(
        cfg.env.get("SQLITE_PATH").map(|s| s.as_str()),
        Some("/data/workspace.db")
    );
}

#[test]
fn test_parse_vscode_format_nested_mcp() {
    let tmp = tempdir().expect("tempdir");
    let root = tmp.path();

    let vscode_dir = root.join(".vscode");
    fs::create_dir_all(&vscode_dir).unwrap();
    let vscode_mcp = vscode_dir.join("mcp.json");

    let nested_content = r#"{
        "mcp": {
            "servers": {
                "vscode-nested-server": {
                    "command": "python",
                    "args": ["-m", "mcp_service"]
                }
            }
        }
    }"#;
    fs::write(&vscode_mcp, nested_content).unwrap();

    let configs = McpDiscoveryEngine::discover_workspace_only(root);
    assert_eq!(configs.len(), 1);
    assert_eq!(configs[0].name, "vscode-nested-server");
    assert_eq!(configs[0].command, "python");
    assert_eq!(configs[0].args, vec!["-m", "mcp_service"]);
}

#[test]
fn test_parse_vscode_format_array() {
    let tmp = tempdir().expect("tempdir");
    let root = tmp.path();

    let vscode_dir = root.join(".vscode");
    fs::create_dir_all(&vscode_dir).unwrap();
    let vscode_mcp = vscode_dir.join("mcp.json");

    let array_content = r#"{
        "servers": [
            {
                "name": "array-server-1",
                "command": "npx",
                "args": ["-y", "mcp-pkg-1"]
            },
            {
                "name": "array-server-2",
                "command": "cargo",
                "args": ["run", "--bin", "mcp-server"]
            }
        ]
    }"#;
    fs::write(&vscode_mcp, array_content).unwrap();

    let configs = McpDiscoveryEngine::discover_workspace_only(root);
    assert_eq!(configs.len(), 2);
    assert_eq!(configs[0].name, "array-server-1");
    assert_eq!(configs[0].command, "npx");
    assert_eq!(configs[1].name, "array-server-2");
    assert_eq!(configs[1].command, "cargo");
}

#[test]
fn test_parse_claude_desktop_format() {
    let tmp = tempdir().expect("tempdir");
    let root = tmp.path();

    // Create .claude/mcp.json
    let claude_dir = root.join(".claude");
    fs::create_dir_all(&claude_dir).unwrap();
    let claude_mcp = claude_dir.join("mcp.json");

    let claude_content = r#"{
        "mcpServers": {
            "filesystem": {
                "command": "npx",
                "args": ["-y", "@modelcontextprotocol/server-filesystem", "/workspace"]
            },
            "github": {
                "command": "npx",
                "args": ["-y", "@modelcontextprotocol/server-github"],
                "env": {
                    "GITHUB_TOKEN": "ghp_mocktoken9876543210fedcba"
                }
            }
        }
    }"#;
    fs::write(&claude_mcp, claude_content).unwrap();

    let configs = McpDiscoveryEngine::discover_workspace_only(root);
    assert_eq!(
        configs.len(),
        2,
        "Should discover 2 servers from .claude/mcp.json"
    );

    let fs_cfg = configs.iter().find(|c| c.name == "filesystem").unwrap();
    assert_eq!(fs_cfg.command, "npx");
    assert_eq!(
        fs_cfg.args,
        vec![
            "-y",
            "@modelcontextprotocol/server-filesystem",
            "/workspace"
        ]
    );

    let gh_cfg = configs.iter().find(|c| c.name == "github").unwrap();
    assert_eq!(gh_cfg.command, "npx");
    let token = gh_cfg.env.get("GITHUB_TOKEN").expect("GITHUB_TOKEN exists");
    assert_ne!(
        token, "ghp_mocktoken9876543210fedcba",
        "Token must be sanitized"
    );
}

#[test]
fn test_parse_claude_project_config() {
    let tmp = tempdir().expect("tempdir");
    let root = tmp.path();

    // Create .claude.json at root
    let claude_json = root.join(".claude.json");
    let content = r#"{
        "mcpServers": {
            "claude-code-tool": {
                "command": "python",
                "args": ["analyzer.py"]
            }
        }
    }"#;
    fs::write(&claude_json, content).unwrap();

    let configs = McpDiscoveryEngine::discover_workspace_only(root);
    assert_eq!(configs.len(), 1);
    assert_eq!(configs[0].name, "claude-code-tool");
    assert_eq!(configs[0].command, "python");
    assert_eq!(configs[0].args, vec!["analyzer.py"]);
}

#[test]
fn test_parse_fusion_workspace_configs_and_precedence() {
    let tmp = tempdir().expect("tempdir");
    let root = tmp.path();

    // Create .fusion/mcp.json
    let fusion_dir = root.join(".fusion");
    fs::create_dir_all(&fusion_dir).unwrap();
    let fusion_mcp = fusion_dir.join("mcp.json");
    let fusion_servers = fusion_dir.join("mcp_servers.json");

    // .fusion/mcp.json defines "shared-server" with Fusion args
    fs::write(
        &fusion_mcp,
        r#"{
        "mcpServers": {
            "shared-server": {
                "command": "fusion-bin",
                "args": ["--fusion-mode"]
            },
            "fusion-only": {
                "command": "fusion-tool"
            }
        }
    }"#,
    )
    .unwrap();

    // .fusion/mcp_servers.json defines "server-from-mcp-servers"
    fs::write(
        &fusion_servers,
        r#"{
        "mcpServers": {
            "server-from-mcp-servers": {
                "command": "tool-2"
            }
        }
    }"#,
    )
    .unwrap();

    // .cursor/mcp.json also defines "shared-server" with Cursor args
    let cursor_dir = root.join(".cursor");
    fs::create_dir_all(&cursor_dir).unwrap();
    fs::write(
        cursor_dir.join("mcp.json"),
        r#"{
        "mcpServers": {
            "shared-server": {
                "command": "cursor-bin",
                "args": ["--cursor-mode"]
            },
            "cursor-only": {
                "command": "cursor-tool"
            }
        }
    }"#,
    )
    .unwrap();

    let configs = McpDiscoveryEngine::discover_workspace_only(root);
    assert_eq!(configs.len(), 4, "Should discover 4 unique servers");

    // "shared-server" must be from .fusion/mcp.json due to precedence
    let shared = configs.iter().find(|c| c.name == "shared-server").unwrap();
    assert_eq!(shared.command, "fusion-bin");
    assert_eq!(shared.args, vec!["--fusion-mode"]);
    assert_eq!(shared.source_file, fusion_mcp);

    // Other non-conflicting servers are all present
    assert!(configs.iter().any(|c| c.name == "fusion-only"));
    assert!(configs.iter().any(|c| c.name == "server-from-mcp-servers"));
    assert!(configs.iter().any(|c| c.name == "cursor-only"));
}

// ===========================================================================
// Environment Variable Sanitization Tests
// ===========================================================================

#[test]
fn test_environment_sanitization_rules() {
    let mut env = HashMap::new();
    env.insert(
        "OPENAI_API_KEY".to_string(),
        "sk-proj-1234567890abcdef1234567890".to_string(),
    );
    env.insert(
        "ANTHROPIC_API_KEY".to_string(),
        "sk-ant-api03-abcdef1234567890".to_string(),
    );
    env.insert(
        "DATABASE_PASSWORD".to_string(),
        "supersecretpassword".to_string(),
    );
    env.insert(
        "AWS_SECRET_ACCESS_KEY".to_string(),
        "wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY".to_string(),
    );
    env.insert("CLIENT_SECRET".to_string(), "secret123".to_string());
    env.insert(
        "AUTH_BEARER_TOKEN".to_string(),
        "bearer_token_xyz".to_string(),
    );

    // Safe variables
    env.insert("PORT".to_string(), "3000".to_string());
    env.insert("HOST".to_string(), "127.0.0.1".to_string());
    env.insert("NODE_ENV".to_string(), "production".to_string());
    env.insert("RUST_LOG".to_string(), "info".to_string());

    let sanitized = sanitize_environment_map(&env);

    // Verify safe variables are untouched
    assert_eq!(sanitized.get("PORT").unwrap(), "3000");
    assert_eq!(sanitized.get("HOST").unwrap(), "127.0.0.1");
    assert_eq!(sanitized.get("NODE_ENV").unwrap(), "production");
    assert_eq!(sanitized.get("RUST_LOG").unwrap(), "info");

    // Verify sensitive variables are sanitized
    assert_ne!(
        sanitized.get("OPENAI_API_KEY").unwrap(),
        "sk-proj-1234567890abcdef1234567890"
    );
    assert_ne!(
        sanitized.get("ANTHROPIC_API_KEY").unwrap(),
        "sk-ant-api03-abcdef1234567890"
    );
    assert_ne!(
        sanitized.get("DATABASE_PASSWORD").unwrap(),
        "supersecretpassword"
    );
    assert_ne!(
        sanitized.get("AWS_SECRET_ACCESS_KEY").unwrap(),
        "wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY"
    );
    assert_ne!(sanitized.get("CLIENT_SECRET").unwrap(), "secret123");
    assert_ne!(
        sanitized.get("AUTH_BEARER_TOKEN").unwrap(),
        "bearer_token_xyz"
    );

    // Verification of helper functions
    assert!(is_sensitive_variable("OPENAI_API_KEY", "sk-..."));
    assert!(is_sensitive_variable("MY_TOKEN", "any"));
    assert!(!is_sensitive_variable("PORT", "3000"));

    let masked = mask_credential_value("abcdefghijklmnop");
    assert!(masked.contains("..."));
}

// ===========================================================================
// Binary Path Validation Tests
// ===========================================================================

#[test]
fn test_binary_path_validation() {
    let tmp = tempdir().expect("tempdir");
    let base = tmp.path();

    // 1. Existing local executable script
    let mock_script = base.join("mock_mcp_binary.sh");
    fs::write(&mock_script, "#!/bin/sh\necho 'running'").unwrap();

    // Validate relative resolution
    assert!(
        validate_binary_path("./mock_mcp_binary.sh", Some(base)),
        "Relative path should resolve against base_dir"
    );
    assert!(
        validate_binary_path(mock_script.to_str().unwrap(), None),
        "Absolute path should resolve"
    );

    // 2. Non-existent command
    assert!(
        !validate_binary_path("__fusion_nonexistent_binary_xyz_98765__", None),
        "Non-existent binary should fail validation"
    );

    // 3. McpServerConfig method
    let cfg_valid = McpServerConfig::new("test-srv", "./mock_mcp_binary.sh")
        .with_source_file(base.join("mcp.json"));
    assert!(cfg_valid.is_binary_valid());
    assert!(cfg_valid.resolve_binary().is_some());

    let cfg_invalid = McpServerConfig::new("test-srv", "__fake_cmd_404__")
        .with_source_file(base.join("mcp.json"));
    assert!(!cfg_invalid.is_binary_valid());
    assert!(cfg_invalid.resolve_binary().is_none());
}

// ===========================================================================
// JSONC Comment Stripping Tests
// ===========================================================================

#[test]
fn test_strip_json_comments() {
    let jsonc = r#"{
        // Line comment
        "name": "test", /* Inline comment */
        "url": "http://example.com/api", // comment after value
        "escaped": "quoted \"// not a comment\" inside",
        "block": "/* not a comment */"
    }"#;

    let stripped = strip_json_comments(jsonc);
    let parsed: serde_json::Value =
        serde_json::from_str(&stripped).expect("Stripped JSONC must parse as valid standard JSON");

    assert_eq!(parsed["name"], "test");
    assert_eq!(parsed["url"], "http://example.com/api");
    assert_eq!(parsed["escaped"], "quoted \"// not a comment\" inside");
    assert_eq!(parsed["block"], "/* not a comment */");
}

// ===========================================================================
// McpDiscoveryTool Execution Tests
// ===========================================================================

#[tokio::test]
async fn test_mcp_discovery_tool_execute() {
    let tmp = tempdir().expect("tempdir");
    let root = tmp.path();

    // Create a mock server in .cursor/mcp.json
    let cursor_dir = root.join(".cursor");
    fs::create_dir_all(&cursor_dir).unwrap();
    fs::write(
        cursor_dir.join("mcp.json"),
        r#"{
        "mcpServers": {
            "demo-tool": {
                "command": "node",
                "args": ["app.js"],
                "env": {
                    "DEMO_SECRET": "top-secret-val",
                    "PORT": "9000"
                }
            }
        }
    }"#,
    )
    .unwrap();

    let tool = McpDiscoveryTool::new();
    assert_eq!(tool.name(), "mcp_discovery");
    assert!(tool.description().contains("Discovers and inspects"));

    let ctx = ToolContext {
        cwd: root.to_path_buf(),
        env: HashMap::new(),
    };

    // 1. Execute with default summary format
    let summary_result = tool
        .execute(
            json!({
                "path": root.to_str().unwrap(),
                "reload": true,
                "workspace_only": true,
                "format": "summary"
            }),
            &ctx,
        )
        .await
        .expect("Tool execution should succeed");

    assert!(summary_result.contains("demo-tool"));
    assert!(summary_result.contains("node"));
    assert!(summary_result.contains("PORT=9000"));
    assert!(
        !summary_result.contains("top-secret-val"),
        "Secret must be sanitized"
    );

    // 2. Execute with JSON format
    let json_result = tool
        .execute(
            json!({
                "path": root.to_str().unwrap(),
                "reload": false,
                "workspace_only": true,
                "format": "json"
            }),
            &ctx,
        )
        .await
        .expect("JSON execution should succeed");

    let parsed: Vec<McpServerConfig> =
        serde_json::from_str(&json_result).expect("Output must be valid McpServerConfig JSON");
    assert_eq!(parsed.len(), 1);
    assert_eq!(parsed[0].name, "demo-tool");
    assert_eq!(parsed[0].command, "node");
    assert_eq!(parsed[0].args, vec!["app.js"]);
}

#[tokio::test]
async fn test_mcp_discovery_tool_empty_workspace() {
    let tmp = tempdir().expect("tempdir");
    let root = tmp.path();

    let tool = McpDiscoveryTool::new();
    let ctx = ToolContext {
        cwd: root.to_path_buf(),
        env: HashMap::new(),
    };

    let result = tool
        .execute(
            json!({
                "path": root.to_str().unwrap(),
                "reload": true,
                "workspace_only": true
            }),
            &ctx,
        )
        .await
        .expect("Tool execution should succeed");

    assert!(result.contains("No MCP servers discovered") || result.contains("Discovered 0"));
}
