//! Integration tests for multi-source MCP server discovery and precedence.
//!
//! Validates that `discover_mcp_config_paths`, `load_server_configs_from_root`,
//! `list_configured_server_names`, and `McpToolBridge` handle:
//! 1. Multi-file discovery across `.fusion/mcp.json`, `.cursor/mcp.json`, and `.claude.json`.
//! 2. Correct precedence where earlier config files override later ones.
//! 3. Deduplication of server names across multiple configuration files.
//! 4. Non-existent files and directories named like files being skipped.
//! 5. Malformed JSON files being skipped gracefully without crashing the pipeline.
//! 6. Disabled server overrides.

use std::fs;
use std::path::Path;
use tempfile::tempdir;

use fusion::tools::mcp_bridge::{
    discover_mcp_config_paths, list_configured_server_names, load_server_configs_from_root,
    McpToolBridge,
};

#[test]
fn test_discover_mcp_config_paths_all_workspace_candidates() {
    let tmp = tempdir().expect("tempdir");
    let root = tmp.path();

    // Create candidate 1: <root>/.fusion/mcp.json
    let fusion_dir = root.join(".fusion");
    fs::create_dir_all(&fusion_dir).unwrap();
    let fusion_mcp = fusion_dir.join("mcp.json");
    fs::write(&fusion_mcp, r#"{"mcpServers": {}}"#).unwrap();

    // Create candidate 2: <root>/.cursor/mcp.json
    let cursor_dir = root.join(".cursor");
    fs::create_dir_all(&cursor_dir).unwrap();
    let cursor_mcp = cursor_dir.join("mcp.json");
    fs::write(&cursor_mcp, r#"{"mcpServers": {}}"#).unwrap();

    // Create candidate 3: <root>/.claude.json
    let claude_json = root.join(".claude.json");
    fs::write(&claude_json, r#"{"mcpServers": {}}"#).unwrap();

    let paths = discover_mcp_config_paths(root);

    // Verify workspace paths appear in exact precedence order
    let fusion_idx = paths.iter().position(|p| p == &fusion_mcp);
    let cursor_idx = paths.iter().position(|p| p == &cursor_mcp);
    let claude_idx = paths.iter().position(|p| p == &claude_json);

    assert!(
        fusion_idx.is_some(),
        ".fusion/mcp.json should be discovered"
    );
    assert!(
        cursor_idx.is_some(),
        ".cursor/mcp.json should be discovered"
    );
    assert!(
        claude_idx.is_some(),
        ".claude.json should be discovered"
    );

    assert!(
        fusion_idx.unwrap() < cursor_idx.unwrap(),
        ".fusion/mcp.json must precede .cursor/mcp.json"
    );
    assert!(
        cursor_idx.unwrap() < claude_idx.unwrap(),
        ".cursor/mcp.json must precede .claude.json"
    );
}

#[test]
fn test_discover_mcp_config_paths_partial_and_missing() {
    let tmp = tempdir().expect("tempdir");
    let root = tmp.path();

    // Only create .cursor/mcp.json
    let cursor_dir = root.join(".cursor");
    fs::create_dir_all(&cursor_dir).unwrap();
    let cursor_mcp = cursor_dir.join("mcp.json");
    fs::write(&cursor_mcp, r#"{"mcpServers": {}}"#).unwrap();

    let paths = discover_mcp_config_paths(root);

    assert!(
        paths.contains(&cursor_mcp),
        ".cursor/mcp.json must be found"
    );
    assert!(
        !paths.contains(&root.join(".fusion").join("mcp.json")),
        "non-existent .fusion/mcp.json must not be included"
    );
    assert!(
        !paths.contains(&root.join(".claude.json")),
        "non-existent .claude.json in root must not be included"
    );
}

#[test]
fn test_discover_mcp_config_paths_ignores_directories() {
    let tmp = tempdir().expect("tempdir");
    let root = tmp.path();

    // Create a directory named mcp.json instead of a file
    let fusion_dir = root.join(".fusion");
    let fake_mcp_dir = fusion_dir.join("mcp.json");
    fs::create_dir_all(&fake_mcp_dir).unwrap();

    let paths = discover_mcp_config_paths(root);
    assert!(
        !paths.contains(&fake_mcp_dir),
        "directories named mcp.json must not be included"
    );
}

#[test]
fn test_multi_source_precedence_earlier_config_overrides_later() {
    let tmp = tempdir().expect("tempdir");
    let root = tmp.path();

    // 1. .fusion/mcp.json (highest priority)
    let fusion_dir = root.join(".fusion");
    fs::create_dir_all(&fusion_dir).unwrap();
    fs::write(
        fusion_dir.join("mcp.json"),
        r#"{
            "mcpServers": {
                "shared": {
                    "command": "fusion-cmd",
                    "args": ["--source", "fusion"]
                }
            }
        }"#,
    )
    .unwrap();

    // 2. .cursor/mcp.json (second priority)
    let cursor_dir = root.join(".cursor");
    fs::create_dir_all(&cursor_dir).unwrap();
    fs::write(
        cursor_dir.join("mcp.json"),
        r#"{
            "mcpServers": {
                "shared": {
                    "command": "cursor-cmd",
                    "args": ["--source", "cursor"]
                },
                "cursor-only": {
                    "command": "cursor-tool",
                    "args": ["--run"]
                }
            }
        }"#,
    )
    .unwrap();

    // 3. .claude.json (third priority)
    fs::write(
        root.join(".claude.json"),
        r#"{
            "mcpServers": {
                "shared": {
                    "command": "claude-cmd",
                    "args": ["--source", "claude"]
                },
                "claude-only": {
                    "command": "claude-tool",
                    "args": ["--start"]
                }
            }
        }"#,
    )
    .unwrap();

    let configs = load_server_configs_from_root(root);

    // Find servers in merged configs
    let shared = configs.iter().find(|c| c.name == "shared");
    let cursor_only = configs.iter().find(|c| c.name == "cursor-only");
    let claude_only = configs.iter().find(|c| c.name == "claude-only");

    assert!(shared.is_some(), "shared server must be present");
    let shared = shared.unwrap();
    assert_eq!(
        shared.command, "fusion-cmd",
        "earlier config (.fusion) must override later config (.cursor / .claude)"
    );
    assert_eq!(shared.args, vec!["--source", "fusion"]);

    assert!(
        cursor_only.is_some(),
        "cursor-only server from lower-priority config must be included"
    );
    assert_eq!(cursor_only.unwrap().command, "cursor-tool");

    assert!(
        claude_only.is_some(),
        "claude-only server from lowest-priority workspace config must be included"
    );
    assert_eq!(claude_only.unwrap().command, "claude-tool");
}

#[test]
fn test_disabled_server_precedence() {
    let tmp = tempdir().expect("tempdir");
    let root = tmp.path();

    // .fusion/mcp.json disables "telemetry"
    let fusion_dir = root.join(".fusion");
    fs::create_dir_all(&fusion_dir).unwrap();
    fs::write(
        fusion_dir.join("mcp.json"),
        r#"{
            "mcpServers": {
                "telemetry": {
                    "command": "telemetry-agent",
                    "disabled": true
                }
            }
        }"#,
    )
    .unwrap();

    // .cursor/mcp.json enables "telemetry"
    let cursor_dir = root.join(".cursor");
    fs::create_dir_all(&cursor_dir).unwrap();
    fs::write(
        cursor_dir.join("mcp.json"),
        r#"{
            "mcpServers": {
                "telemetry": {
                    "command": "telemetry-agent",
                    "disabled": false
                }
            }
        }"#,
    )
    .unwrap();

    let configs = load_server_configs_from_root(root);
    let telemetry = configs.iter().find(|c| c.name == "telemetry").unwrap();
    assert!(
        telemetry.disabled,
        "disabled flag from higher-priority config must take precedence"
    );
}

#[test]
fn test_list_configured_server_names_deduplication_and_order() {
    let tmp = tempdir().expect("tempdir");
    let root = tmp.path();

    let fusion_dir = root.join(".fusion");
    fs::create_dir_all(&fusion_dir).unwrap();
    fs::write(
        fusion_dir.join("mcp.json"),
        r#"{
            "mcpServers": {
                "alpha": { "command": "a" },
                "beta": { "command": "b1" }
            }
        }"#,
    )
    .unwrap();

    let cursor_dir = root.join(".cursor");
    fs::create_dir_all(&cursor_dir).unwrap();
    fs::write(
        cursor_dir.join("mcp.json"),
        r#"{
            "mcpServers": {
                "beta": { "command": "b2" },
                "gamma": { "command": "g" }
            }
        }"#,
    )
    .unwrap();

    let names = list_configured_server_names(root);

    // Verify alpha, beta, gamma are all present
    assert!(names.contains(&"alpha".to_string()));
    assert!(names.contains(&"beta".to_string()));
    assert!(names.contains(&"gamma".to_string()));

    // Verify beta is not duplicated
    let beta_count = names.iter().filter(|n| *n == "beta").count();
    assert_eq!(beta_count, 1, "server names must be deduplicated");

    // Verify alpha appears before beta, which appears before gamma
    let alpha_pos = names.iter().position(|n| n == "alpha").unwrap();
    let beta_pos = names.iter().position(|n| n == "beta").unwrap();
    let gamma_pos = names.iter().position(|n| n == "gamma").unwrap();

    assert!(alpha_pos < beta_pos);
    assert!(beta_pos < gamma_pos);
}

#[test]
fn test_malformed_json_file_resilience() {
    let tmp = tempdir().expect("tempdir");
    let root = tmp.path();

    // Corrupted .fusion/mcp.json
    let fusion_dir = root.join(".fusion");
    fs::create_dir_all(&fusion_dir).unwrap();
    fs::write(fusion_dir.join("mcp.json"), "{ invalid: json... }").unwrap();

    // Valid .cursor/mcp.json
    let cursor_dir = root.join(".cursor");
    fs::create_dir_all(&cursor_dir).unwrap();
    fs::write(
        cursor_dir.join("mcp.json"),
        r#"{
            "mcpServers": {
                "resilient": { "command": "resilient-cmd" }
            }
        }"#,
    )
    .unwrap();

    // Loading should skip corrupted file and load valid file
    let configs = load_server_configs_from_root(root);
    assert!(
        configs.iter().any(|c| c.name == "resilient"),
        "valid configs must load even if another config file is corrupted"
    );
}

#[test]
fn test_mcp_tool_bridge_associated_methods() {
    let tmp = tempdir().expect("tempdir");
    let root = tmp.path();

    let fusion_dir = root.join(".fusion");
    fs::create_dir_all(&fusion_dir).unwrap();
    fs::write(
        fusion_dir.join("mcp.json"),
        r#"{
            "mcpServers": {
                "bridge_test": { "command": "test-cmd" }
            }
        }"#,
    )
    .unwrap();

    // Test McpToolBridge::discover_mcp_config_paths
    let paths = McpToolBridge::discover_mcp_config_paths(root);
    assert!(paths.iter().any(|p| p.ends_with(".fusion/mcp.json")));

    // Test McpToolBridge::list_configured_server_names
    let names = McpToolBridge::list_configured_server_names(root);
    assert!(names.contains(&"bridge_test".to_string()));
}

#[test]
fn test_flat_map_and_servers_key_formats() {
    let tmp = tempdir().expect("tempdir");
    let root = tmp.path();

    // 1. .fusion/mcp.json with "servers" key
    let fusion_dir = root.join(".fusion");
    fs::create_dir_all(&fusion_dir).unwrap();
    fs::write(
        fusion_dir.join("mcp.json"),
        r#"{
            "servers": {
                "from_servers": { "command": "cmd1" }
            }
        }"#,
    )
    .unwrap();

    // 2. .cursor/mcp.json with flat map format
    let cursor_dir = root.join(".cursor");
    fs::create_dir_all(&cursor_dir).unwrap();
    fs::write(
        cursor_dir.join("mcp.json"),
        r#"{
            "from_flat_map": { "command": "cmd2" }
        }"#,
    )
    .unwrap();

    let names = list_configured_server_names(root);
    assert!(names.contains(&"from_servers".to_string()));
    assert!(names.contains(&"from_flat_map".to_string()));
}
