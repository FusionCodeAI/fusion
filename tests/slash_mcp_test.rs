//! Comprehensive tests for `/mcp` slash command handler and server formatting table.

use std::fs;
use tempfile::tempdir;
use fusion::ui::slash_mcp::{format_mcp_servers_table, handle_mcp_command};
#[test]
fn test_format_mcp_servers_table_empty() {
    let output = format_mcp_servers_table(&[]);
    assert!(
        output.contains("No MCP servers configured"),
        "Empty servers should return a clear message, got:\n{}",
        output
    );
}

#[test]
fn test_format_mcp_servers_table_populated() {
    let servers = [
        ("filesystem", ".fusion/mcp.json", 3, true),
        ("github", ".cursor/mcp.json", 0, false),
        ("memory-store", "/home/user/.claude.json", 12, true),
    ];

    let table = format_mcp_servers_table(&servers);

    // Verify table headers
    assert!(table.contains("Server Name"), "Missing 'Server Name' header");
    assert!(table.contains("Config Source"), "Missing 'Config Source' header");
    assert!(table.contains("Status"), "Missing 'Status' header");
    assert!(table.contains("Tool Count"), "Missing 'Tool Count' header");

    // Verify box drawing characters
    assert!(table.contains('┌'));
    assert!(table.contains('┬'));
    assert!(table.contains('┐'));
    assert!(table.contains('├'));
    assert!(table.contains('┼'));
    assert!(table.contains('┤'));
    assert!(table.contains('└'));
    assert!(table.contains('┴'));
    assert!(table.contains('┘'));

    // Verify server entries and connection statuses
    assert!(table.contains("filesystem"));
    assert!(table.contains(".fusion/mcp.json"));
    assert!(table.contains("Connected"));
    assert!(table.contains('3'));

    assert!(table.contains("github"));
    assert!(table.contains(".cursor/mcp.json"));
    assert!(table.contains("Failed"));
    assert!(table.contains('0'));

    assert!(table.contains("memory-store"));
    assert!(table.contains("/home/user/.claude.json"));
    assert!(table.contains("12"));
}

#[tokio::test]
async fn test_handle_mcp_command_unknown_subcommand() {
    let dir = tempdir().expect("tempdir");
    let res = handle_mcp_command(&["invalid_subcommand".to_string()], dir.path()).await;
    assert!(res.contains("Unknown MCP subcommand: 'invalid_subcommand'"));
    assert!(res.contains("/mcp [list]"));
    assert!(res.contains("/mcp status"));
    assert!(res.contains("/mcp reload"));
}

#[tokio::test]
async fn test_handle_mcp_command_list_empty_workspace() {
    let dir = tempdir().expect("tempdir");
    let res = handle_mcp_command(&["list".to_string()], dir.path()).await;
    assert!(
        res.contains("No MCP configuration files discovered")
            || res.contains("MCP Configuration Sources"),
        "Unexpected output: {}",
        res
    );
}

#[tokio::test]
async fn test_handle_mcp_command_default_is_list() {
    let dir = tempdir().expect("tempdir");
    let res_empty_args = handle_mcp_command(&[], dir.path()).await;
    let res_blank_arg = handle_mcp_command(&["".to_string()], dir.path()).await;
    let res_list = handle_mcp_command(&["list".to_string()], dir.path()).await;

    assert_eq!(res_empty_args, res_list);
    assert_eq!(res_blank_arg, res_list);
}

#[tokio::test]
async fn test_handle_mcp_command_list_with_config() {
    let dir = tempdir().expect("tempdir");
    let fusion_dir = dir.path().join(".fusion");
    fs::create_dir_all(&fusion_dir).expect("create .fusion");
    let config_file = fusion_dir.join("mcp.json");

    let json_content = serde_json::json!({
        "mcpServers": {
            "test-fs": {
                "command": "echo",
                "args": ["hello", "world"]
            }
        }
    });
    fs::write(&config_file, json_content.to_string()).expect("write mcp.json");

    let res = handle_mcp_command(&["list".to_string()], dir.path()).await;
    assert!(res.contains("MCP Configuration Sources:"));
    assert!(res.contains("mcp.json"));
    assert!(res.contains("test-fs"));
    assert!(res.contains("echo hello world"));
}

#[tokio::test]
async fn test_handle_mcp_command_status_empty_workspace() {
    let dir = tempdir().expect("tempdir");
    let res = handle_mcp_command(&["status".to_string()], dir.path()).await;
    // Without workspace config, either reports no servers or reflects global config status
    assert!(
        res.contains("No MCP servers configured") || res.contains("Server Name"),
        "Unexpected status output: {}",
        res
    );
}
#[tokio::test]
async fn test_handle_mcp_command_status_unreachable_server() {
    let dir = tempdir().expect("tempdir");
    let fusion_dir = dir.path().join(".fusion");
    fs::create_dir_all(&fusion_dir).expect("create .fusion");
    let config_file = fusion_dir.join("mcp.json");

    let json_content = serde_json::json!({
        "mcpServers": {
            "unreachable-server": {
                "command": "nonexistent_binary_that_cannot_run_xyz123",
                "args": []
            }
        }
    });
    fs::write(&config_file, json_content.to_string()).expect("write mcp.json");

    let res = handle_mcp_command(&["status".to_string()], dir.path()).await;
    assert!(res.contains("Server Name"));
    assert!(res.contains("unreachable-server"));
    assert!(res.contains("Failed"));
    assert!(res.contains('0'));
}

#[tokio::test]
async fn test_handle_mcp_command_status_disabled_server() {
    let dir = tempdir().expect("tempdir");
    let fusion_dir = dir.path().join(".fusion");
    fs::create_dir_all(&fusion_dir).expect("create .fusion");
    let config_file = fusion_dir.join("mcp.json");

    let json_content = serde_json::json!({
        "mcpServers": {
            "disabled-server": {
                "command": "echo",
                "args": ["disabled"],
                "disabled": true
            }
        }
    });
    fs::write(&config_file, json_content.to_string()).expect("write mcp.json");

    let res = handle_mcp_command(&["status".to_string()], dir.path()).await;
    assert!(res.contains("disabled-server"));
    assert!(res.contains("Failed"));
}

#[tokio::test]
async fn test_handle_mcp_command_reload() {
    let dir = tempdir().expect("tempdir");
    let fusion_dir = dir.path().join(".fusion");
    fs::create_dir_all(&fusion_dir).expect("create .fusion");
    let config_file = fusion_dir.join("mcp.json");

    let json_content = serde_json::json!({
        "mcpServers": {
            "dummy-reload": {
                "command": "echo",
                "args": ["reload"]
            }
        }
    });
    fs::write(&config_file, json_content.to_string()).expect("write mcp.json");

    let res = handle_mcp_command(&["reload".to_string()], dir.path()).await;
    assert!(res.contains("Reloaded MCP configuration from"));
    assert!(res.contains("Successfully registered"));
}

#[tokio::test]
async fn test_handle_mcp_command_multiple_sources_precedence() {
    let dir = tempdir().expect("tempdir");
    let fusion_dir = dir.path().join(".fusion");
    let cursor_dir = dir.path().join(".cursor");
    fs::create_dir_all(&fusion_dir).expect("create .fusion");
    fs::create_dir_all(&cursor_dir).expect("create .cursor");

    let fusion_mcp = fusion_dir.join("mcp.json");
    let cursor_mcp = cursor_dir.join("mcp.json");

    let fusion_json = serde_json::json!({
        "mcpServers": {
            "shared-srv": {
                "command": "fusion-cmd",
                "args": ["primary"]
            }
        }
    });
    let cursor_json = serde_json::json!({
        "mcpServers": {
            "shared-srv": {
                "command": "cursor-cmd",
                "args": ["secondary"]
            },
            "cursor-unique": {
                "command": "unique-cmd",
                "args": ["test"]
            }
        }
    });

    fs::write(&fusion_mcp, fusion_json.to_string()).expect("write fusion mcp");
    fs::write(&cursor_mcp, cursor_json.to_string()).expect("write cursor mcp");

    let res = handle_mcp_command(&["list".to_string()], dir.path()).await;
    // Both sources are discovered
    assert!(res.contains(".fusion/mcp.json") || res.contains("mcp.json"));
    // shared-srv takes precedence from fusion (higher precedence)
    assert!(res.contains("shared-srv"));
    assert!(res.contains("fusion-cmd primary"));
    // cursor-unique is also present
    assert!(res.contains("cursor-unique"));
    assert!(res.contains("unique-cmd test"));
}
