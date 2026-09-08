//! Slash command handler for `/mcp` — Model Context Protocol management.
//!
//! Provides interactive commands to inspect configured MCP servers, test connectivity,
//! list registered tools, and dynamically reload server configs from disk.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::tools::mcp::{McpManager, McpServersConfig};
use crate::tools::mcp_bridge::McpToolBridge;
use crate::tools::types::DynTool;
/// Handles the `/mcp` slash command with subcommands: `list` (default), `status`, and `reload`.
pub async fn handle_mcp_command(args: &[String], workspace_root: &Path) -> String {
    let subcommand = args
        .first()
        .map(|s| s.trim().to_lowercase())
        .unwrap_or_default();

    match subcommand.as_str() {
        "" | "list" => handle_list(workspace_root).await,
        "status" => handle_status(workspace_root).await,
        "reload" => handle_reload(workspace_root).await,
        other => {
            format!(
                "Unknown MCP subcommand: '{}'.\n\nUsage:\n  /mcp [list]    List configured MCP servers and discovered tools\n  /mcp status    Test connectivity and display server status\n  /mcp reload    Reload and re-initialize MCP tools from disk\n",
                other
            )
        }
    }
}

/// Formats a clean Unicode box-drawing table of MCP server statuses.
///
/// Columns: Server Name, Config Source, Status, Tool Count.
pub fn format_mcp_servers_table(servers: &[(&str, &str, usize, bool)]) -> String {
    if servers.is_empty() {
        return "No MCP servers configured.\n".to_string();
    }

    let h_name = "Server Name";
    let h_source = "Config Source";
    let h_status = "Status";
    let h_tools = "Tool Count";

    let mut name_width = h_name.len();
    let mut source_width = h_source.len();
    let status_width = h_status.len().max("Connected".len()).max("Failed".len());
    let mut tools_width = h_tools.len();

    for (name, source, tool_count, _) in servers {
        name_width = name_width.max(name.len());
        source_width = source_width.max(source.len());
        tools_width = tools_width.max(tool_count.to_string().len());
    }

    let mut out = String::new();

    // Top border
    out.push('┌');
    out.push_str(&"─".repeat(name_width + 2));
    out.push('┬');
    out.push_str(&"─".repeat(source_width + 2));
    out.push('┬');
    out.push_str(&"─".repeat(status_width + 2));
    out.push('┬');
    out.push_str(&"─".repeat(tools_width + 2));
    out.push_str("┐\n");

    // Header row
    out.push_str(&format!(
        "│ {:<nw$} │ {:<sw$} │ {:<stw$} │ {:<tw$} │\n",
        h_name,
        h_source,
        h_status,
        h_tools,
        nw = name_width,
        sw = source_width,
        stw = status_width,
        tw = tools_width,
    ));

    // Header separator
    out.push('├');
    out.push_str(&"─".repeat(name_width + 2));
    out.push('┼');
    out.push_str(&"─".repeat(source_width + 2));
    out.push('┼');
    out.push_str(&"─".repeat(status_width + 2));
    out.push('┼');
    out.push_str(&"─".repeat(tools_width + 2));
    out.push_str("┤\n");

    // Data rows
    for (name, source, tool_count, is_connected) in servers {
        let status_str = if *is_connected { "Connected" } else { "Failed" };
        out.push_str(&format!(
            "│ {:<nw$} │ {:<sw$} │ {:<stw$} │ {:<tw$} │\n",
            name,
            source,
            status_str,
            tool_count,
            nw = name_width,
            sw = source_width,
            stw = status_width,
            tw = tools_width,
        ));
    }

    // Bottom border
    out.push('└');
    out.push_str(&"─".repeat(name_width + 2));
    out.push('┴');
    out.push_str(&"─".repeat(source_width + 2));
    out.push('┴');
    out.push_str(&"─".repeat(status_width + 2));
    out.push('┴');
    out.push_str(&"─".repeat(tools_width + 2));
    out.push_str("┘\n");

    out
}

/// Subcommand `list`: Queries discovered MCP servers via `McpToolBridge::discover_mcp_config_paths`
/// and lists configured servers and discovered tools.
async fn handle_list(workspace_root: &Path) -> String {
    let config_paths: Vec<PathBuf> = McpToolBridge::discover_mcp_config_paths(workspace_root);
    if config_paths.is_empty() {
        return "No MCP configuration files discovered.\n\nTo configure MCP servers, create a config file at:\n  • .fusion/mcp.json\n  • .cursor/mcp.json\n  • ~/.claude.json\n".to_string();
    }

    let mut out = String::new();
    out.push_str("MCP Configuration Sources:\n");
    for path in &config_paths {
        out.push_str(&format!("  • {}\n", path.display()));
    }
    out.push('\n');

    // Read configured servers across discovered configs
    let mut configured_servers = Vec::new();
    let mut seen_names = HashMap::new();

    for path in &config_paths {
        let content = match std::fs::read_to_string(path) {
            Ok(c) => c,
            Err(e) => {
                out.push_str(&format!(
                    "Warning: Could not read {}: {}\n",
                    path.display(),
                    e
                ));
                continue;
            }
        };

        let configs = match McpServersConfig::from_json_str(&content) {
            Ok(c) => c,
            Err(e) => {
                out.push_str(&format!(
                    "Warning: Could not parse {}: {}\n",
                    path.display(),
                    e
                ));
                continue;
            }
        };

        let display_path = path.display().to_string();
        for cfg in configs {
            if !seen_names.contains_key(&cfg.name) {
                seen_names.insert(cfg.name.clone(), display_path.clone());
                configured_servers.push((cfg, display_path.clone()));
            }
        }
    }

    if configured_servers.is_empty() {
        out.push_str("No MCP servers configured in discovered files.\n");
        return out;
    }

    out.push_str(&format!(
        "Configured Servers ({}):\n",
        configured_servers.len()
    ));
    for (cfg, source) in &configured_servers {
        let status = if cfg.disabled { " (disabled)" } else { "" };
        out.push_str(&format!(
            "  • {} [{}]{}: {} {}\n",
            cfg.name,
            source,
            status,
            cfg.command,
            cfg.args.join(" ")
        ));
    }
    out.push('\n');

    // Discover tools
    let tools: Vec<DynTool> = McpToolBridge::load_from_root(workspace_root).await;
    if tools.is_empty() {
        out.push_str("Discovered Tools: 0 (no tools registered or servers unreachable)\n");
    } else {
        out.push_str(&format!("Discovered Tools ({}):\n", tools.len()));
        for tool in tools {
            out.push_str(&format!("  • {}: {}\n", tool.name(), tool.description()));
        }
    }

    out
}

/// Subcommand `status`: Tests connectivity to configured servers and returns status table.
async fn handle_status(workspace_root: &Path) -> String {
    let config_paths: Vec<PathBuf> = McpToolBridge::discover_mcp_config_paths(workspace_root);
    if config_paths.is_empty() {
        return "No MCP servers configured.\n".to_string();
    }

    let mut configured_servers = Vec::new();
    let mut seen_names = HashMap::new();

    for path in &config_paths {
        let content = match std::fs::read_to_string(path) {
            Ok(c) => c,
            Err(_) => continue,
        };

        let configs = match McpServersConfig::from_json_str(&content) {
            Ok(c) => c,
            Err(_) => continue,
        };

        let display_path = path.display().to_string();
        for cfg in configs {
            if !seen_names.contains_key(&cfg.name) {
                seen_names.insert(cfg.name.clone(), display_path.clone());
                configured_servers.push((cfg, display_path.clone()));
            }
        }
    }

    if configured_servers.is_empty() {
        return "No MCP servers configured.\n".to_string();
    }

    let mut rows: Vec<(String, String, usize, bool)> = Vec::new();

    for (cfg, source) in configured_servers {
        if cfg.disabled {
            rows.push((cfg.name, source, 0, false));
            continue;
        }

        let name = cfg.name.clone();
        let manager = McpManager::new();
        // Test connectivity with 5-second timeout to prevent hanging on stalled processes
        let test_res =
            tokio::time::timeout(Duration::from_secs(5), manager.connect_server(cfg)).await;

        match test_res {
            Ok(Ok(tools)) => {
                let tool_count = tools.len();
                let _ = manager.disconnect_server(&name).await;
                rows.push((name, source, tool_count, true));
            }
            _ => {
                rows.push((name, source, 0, false));
            }
        }
    }

    let table_inputs: Vec<(&str, &str, usize, bool)> = rows
        .iter()
        .map(|(n, s, count, conn)| (n.as_str(), s.as_str(), *count, *conn))
        .collect();

    format_mcp_servers_table(&table_inputs)
}

/// Subcommand `reload`: Reloads and re-initializes all MCP tools from disk.
async fn handle_reload(workspace_root: &Path) -> String {
    let config_paths: Vec<PathBuf> = McpToolBridge::discover_mcp_config_paths(workspace_root);
    let tools: Vec<DynTool> = McpToolBridge::load_from_root(workspace_root).await;

    let mut out = String::new();
    out.push_str(&format!(
        "Reloaded MCP configuration from {} config source(s).\n",
        config_paths.len()
    ));
    out.push_str(&format!(
        "Successfully registered {} MCP tool(s).\n",
        tools.len()
    ));

    if !tools.is_empty() {
        out.push_str("\nRegistered Tools:\n");
        for tool in tools {
            out.push_str(&format!("  • {}: {}\n", tool.name(), tool.description()));
        }
    }

    out
}
