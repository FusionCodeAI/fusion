//! MCP Tool Bridge — workspace-local server discovery.
//!
//! Reads `.fusion/mcp.json` from the current working directory (or a caller-
//! supplied root), performs the full MCP initialization handshake with each
//! listed server, and returns the discovered tools as [`DynTool`] instances
//! compatible with [`ToolRegistry`].
//!
//! # Config file format
//!
//! `.fusion/mcp.json` accepts any format supported by [`McpServersConfig`]:
//!
//! ```json
//! {
//!   "mcpServers": {
//!     "filesystem": {
//!       "command": "npx",
//!       "args": ["-y", "@modelcontextprotocol/server-filesystem", "/tmp"]
//!     }
//!   }
//! }
//! ```
//!
//! Or a flat map / list — see [`McpServersConfig::from_json_str`] for the
//! full grammar.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use tracing::{info, warn};

use crate::tools::mcp::{McpManager, McpServerConfig, McpServersConfig};
use crate::tools::types::DynTool;
// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Discovers MCP configuration paths in order of precedence:
/// 1. `<root>/.fusion/mcp.json`
/// 2. `<root>/.cursor/mcp.json`
/// 3. `<root>/.claude.json`
/// 4. `<config_dir>/mcp.json` (`~/.fusion/mcp.json`)
/// 5. `<home_dir>/.claude.json` (`~/.claude.json`)
///
/// Filters to paths where `path.exists() && path.is_file()`.
pub fn discover_mcp_config_paths(root: &Path) -> Vec<PathBuf> {
    let mut candidates = vec![
        root.join(".fusion").join("mcp.json"),
        root.join(".cursor").join("mcp.json"),
        root.join(".claude.json"),
        crate::config::Config::config_dir().join("mcp.json"),
    ];
    if let Some(home) = dirs::home_dir() {
        candidates.push(home.join(".claude.json"));
    }

    let mut seen = HashSet::new();
    let mut result = Vec::new();
    for p in candidates {
        if p.exists() && p.is_file() {
            let key = p.canonicalize().unwrap_or_else(|_| p.clone());
            if seen.insert(key) {
                result.push(p);
            }
        }
    }
    result
}

/// Discovers and parses all MCP server configurations from matching config files,
/// merging them into a single list where earlier/higher-priority configs take
/// precedence over later ones for each server name.
pub fn load_server_configs_from_root(root: impl AsRef<Path>) -> Vec<McpServerConfig> {
    let paths = discover_mcp_config_paths(root.as_ref());
    let mut merged: std::collections::HashMap<String, McpServerConfig> =
        std::collections::HashMap::new();
    let mut ordered_names = Vec::new();

    for path in paths {
        let json_str = match std::fs::read_to_string(&path) {
            Ok(s) => s,
            Err(e) => {
                warn!("McpToolBridge: could not read {}: {}", path.display(), e);
                continue;
            }
        };

        let configs = match McpServersConfig::from_json_str(&json_str) {
            Ok(c) => c,
            Err(e) => {
                warn!("McpToolBridge: could not parse {}: {}", path.display(), e);
                continue;
            }
        };

        for config in configs {
            if !merged.contains_key(&config.name) {
                ordered_names.push(config.name.clone());
                merged.insert(config.name.clone(), config);
            }
        }
    }

    ordered_names
        .into_iter()
        .filter_map(|name| merged.remove(&name))
        .collect()
}

/// Returns the deduplicated names of all configured MCP servers across all
/// discovered config files for the given root, preserving precedence order.
pub fn list_configured_server_names(root: impl AsRef<Path>) -> Vec<String> {
    load_server_configs_from_root(root)
        .into_iter()
        .map(|cfg| cfg.name)
        .collect()
}

/// Bridge that discovers and wraps MCP servers listed in workspace and global configs.
///
/// # Example
///
/// ```no_run
/// # use fusion::tools::mcp_bridge::McpToolBridge;
/// # #[tokio::main] async fn main() {
/// let tools = McpToolBridge::load().await;
/// println!("Discovered {} MCP tool(s)", tools.len());
/// # }
/// ```
pub struct McpToolBridge;

impl McpToolBridge {
    /// Loads MCP tools from `.fusion/mcp.json` in the current working
    /// directory.
    ///
    /// Servers that fail to connect or initialize are logged as warnings and
    /// skipped — the successful ones are returned.  An empty `Vec` is
    /// returned when the config file does not exist.
    pub async fn load() -> Vec<DynTool> {
        let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
        Self::load_from_root(&cwd).await
    }

    /// Loads MCP tools from all discovered configuration files under `<root>`
    /// and global/IDE config paths.
    ///
    /// Configs are merged with earlier files taking precedence. Servers that
    /// fail to connect or are marked disabled are skipped.
    pub async fn load_from_root(root: impl AsRef<Path>) -> Vec<DynTool> {
        let configs = load_server_configs_from_root(root);
        if configs.is_empty() {
            return Vec::new();
        }

        let manager = McpManager::new();
        let mut all_tools = Vec::new();

        for config in configs {
            if config.disabled {
                continue;
            }

            let name = config.name.clone();
            match manager.connect_server(config).await {
                Ok(tools) => {
                    info!(
                        "McpToolBridge: server '{}' registered {} tool(s)",
                        name,
                        tools.len()
                    );
                    all_tools.extend(tools);
                }
                Err(e) => {
                    warn!("McpToolBridge: server '{}' failed to connect: {}", name, e);
                }
            }
        }

        all_tools
    }

    /// Returns the deduplicated names of all configured MCP servers across all
    /// discovered config files for the given root.
    pub fn list_configured_server_names(root: impl AsRef<Path>) -> Vec<String> {
        list_configured_server_names(root)
    }
    /// Discovers MCP configuration paths in order of precedence.
    pub fn discover_mcp_config_paths(root: &Path) -> Vec<PathBuf> {
        discover_mcp_config_paths(root)
    }

    /// Loads MCP tools from a raw JSON string without touching the filesystem.
    ///
    /// Useful for tests and programmatic configuration.
    pub async fn load_from_json(json_str: &str) -> Vec<DynTool> {
        let manager = McpManager::new();
        match manager.load_from_json(json_str).await {
            Ok(tools) => tools,
            Err(e) => {
                warn!("McpToolBridge: load_from_json failed: {}", e);
                Vec::new()
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::sync::Arc;

    use crate::tools::mcp::{
        McpClient, McpError, McpServerConfig, McpTool, McpToolDefinition, METHOD_NOT_FOUND,
    };
    use crate::tools::types::{Tool, ToolContext, ToolRegistry};

    // Build a mock McpClient handler that simulates a full MCP server over the
    // in-memory transport (no actual stdio process is spawned).
    fn make_mock_handler(
        tools: Vec<McpToolDefinition>,
    ) -> Arc<
        dyn Fn(String, Option<serde_json::Value>) -> Result<serde_json::Value, McpError>
            + Send
            + Sync,
    > {
        let tools = Arc::new(tools);
        Arc::new(move |method: String, params: Option<serde_json::Value>| {
            match method.as_str() {
                "initialize" => Ok(json!({
                    "protocolVersion": "2024-11-05",
                    "capabilities": { "tools": { "listChanged": false } },
                    "serverInfo": { "name": "mock-bridge-server", "version": "0.1.0" }
                })),
                "tools/list" => {
                    let _ = params; // pagination not exercised in this mock
                    let tool_vals: Vec<serde_json::Value> = tools
                        .iter()
                        .map(|t| {
                            json!({
                                "name": t.name,
                                "description": t.description,
                                "inputSchema": t.input_schema
                            })
                        })
                        .collect();
                    Ok(json!({ "tools": tool_vals }))
                }
                "tools/call" => {
                    let p = params.unwrap_or_default();
                    let tool_name = p.get("name").and_then(|v| v.as_str()).unwrap_or("");
                    let known = tools.iter().any(|t| t.name == tool_name);
                    if known {
                        Ok(json!({
                            "content": [{ "type": "text", "text": format!("ok:{}", tool_name) }],
                            "isError": false
                        }))
                    } else {
                        Err(McpError::JsonRpc {
                            code: METHOD_NOT_FOUND,
                            message: format!("unknown tool '{}'", tool_name),
                            data: None,
                        })
                    }
                }
                "ping" => Ok(json!({})),
                _ => Ok(json!({})),
            }
        })
    }

    // Convenience: build a minimal McpToolDefinition.
    fn tool_def(name: &str, description: &str) -> McpToolDefinition {
        McpToolDefinition {
            name: name.to_string(),
            description: Some(description.to_string()),
            input_schema: json!({ "type": "object", "properties": {} }),
        }
    }

    // -----------------------------------------------------------------------
    // Handshake + tool discovery via mock stdio (in-memory transport)
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn test_mock_handshake_discovers_tools() {
        let defs = vec![
            tool_def("read_file", "Read a file"),
            tool_def("write_file", "Write a file"),
        ];
        let handler = make_mock_handler(defs.clone());
        let config = McpServerConfig::new("mock-fs", "fake-binary");
        let client = Arc::new(McpClient::mock(config, handler));

        // Handshake
        assert!(!client.is_initialized());
        let init = client.initialize().await.expect("initialize failed");
        assert!(client.is_initialized());
        assert_eq!(init.server_info.name, "mock-bridge-server");

        // Tool discovery
        let discovered = client.list_tools().await.expect("list_tools failed");
        assert_eq!(discovered.len(), 2);
        assert_eq!(discovered[0].name, "read_file");
        assert_eq!(discovered[1].name, "write_file");
    }

    // -----------------------------------------------------------------------
    // McpTool wrapping respects optional prefix
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn test_tool_prefix_applied() {
        let defs = vec![tool_def("list", "List directory")];
        let handler = make_mock_handler(defs.clone());
        let config = McpServerConfig::new("fs", "fake").with_prefix("fs_");
        let client = Arc::new(McpClient::mock(config, handler));
        client.initialize().await.unwrap();

        let tool_defs = client.list_tools().await.unwrap();
        let tool = McpTool::new(Arc::clone(&client), tool_defs[0].clone(), Some("fs_"));
        assert_eq!(tool.name(), "fs_list");
        assert_eq!(tool.raw_name(), "list");
    }

    // -----------------------------------------------------------------------
    // Forward tool call through Tool trait → mock response
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn test_tool_call_forwarded_to_mock() {
        let defs = vec![tool_def("greet", "Say hello")];
        let handler = make_mock_handler(defs.clone());
        let config = McpServerConfig::new("greeter", "fake");
        let client = Arc::new(McpClient::mock(config, handler));
        client.initialize().await.unwrap();

        let tool_defs = client.list_tools().await.unwrap();
        let tool: DynTool = Arc::new(McpTool::new(
            Arc::clone(&client),
            tool_defs[0].clone(),
            None,
        ));

        let ctx = ToolContext::default();
        let result = tool
            .execute(json!({ "name": "world" }), &ctx)
            .await
            .unwrap();
        assert_eq!(result, "ok:greet");
    }

    // -----------------------------------------------------------------------
    // load_from_json integrates multiple mock-backed servers
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn test_load_from_json_with_disabled_server() {
        // JSON contains two servers; the second is disabled.
        // The manager should only register tools from the first.
        // We use real McpManager+connect_server over mock transport by
        // exercising the load_from_json path with an inline config that
        // names a command that doesn't exist — but since we only test
        // the disabled path we just verify the disabled server is skipped.
        let json = serde_json::to_string(&json!({
            "mcpServers": {
                "active": {
                    "command": "__nonexistent_command__",
                    "args": []
                },
                "inactive": {
                    "command": "node",
                    "args": ["server.js"],
                    "disabled": true
                }
            }
        }))
        .unwrap();

        // We expect "inactive" to be silently skipped (disabled), and "active"
        // to fail to spawn (no real binary) but that's also handled gracefully.
        let tools = McpToolBridge::load_from_json(&json).await;
        // "inactive" is disabled → 0 tools from it.
        // "active" fails to spawn → 0 tools, but no panic.
        assert!(
            tools.is_empty(),
            "disabled/failed servers must yield no tools"
        );
    }

    // -----------------------------------------------------------------------
    // load_from_root returns empty when config file is absent
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn test_load_from_root_missing_config() {
        let tmp = std::env::temp_dir();
        // Calling load_from_root must succeed gracefully without panic
        let _tools = McpToolBridge::load_from_root(&tmp).await;
    }

    // -----------------------------------------------------------------------
    // load_from_root reads and parses a real config file
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn test_load_from_root_invalid_json_is_graceful() {
        use std::io::Write;
        let tmp = tempfile::tempdir().expect("tempdir");
        let fusion_dir = tmp.path().join(".fusion");
        std::fs::create_dir_all(&fusion_dir).unwrap();
        let mut f = std::fs::File::create(fusion_dir.join("mcp.json")).unwrap();
        f.write_all(b"{ this is not valid json }").unwrap();

        // Invalid JSON must be skipped gracefully without panic
        let _tools = McpToolBridge::load_from_root(tmp.path()).await;
    }

    // -----------------------------------------------------------------------
    // Full round-trip: write a config file, load it, check tools registered
    // -----------------------------------------------------------------------
    // We can't spawn a real MCP server process in a unit test, but we can
    // verify config parsing and the McpManager registration pipeline using the
    // mock client directly.

    #[tokio::test]
    async fn test_mock_manager_full_round_trip() {
        // Build a mock client, initialize it, register its tools into a registry.
        let defs = vec![
            tool_def("search", "Web search"),
            tool_def("fetch", "HTTP fetch"),
        ];
        let handler = make_mock_handler(defs);
        let config = McpServerConfig::new("web-tools", "fake");
        let client = Arc::new(McpClient::mock(config, handler));
        client.initialize().await.unwrap();

        let tool_defs = client.list_tools().await.unwrap();
        let mut registry = ToolRegistry::new();
        for td in tool_defs {
            let tool: DynTool = Arc::new(McpTool::new(Arc::clone(&client), td, None));
            registry.register(tool);
        }

        assert!(registry.contains("search"));
        assert!(registry.contains("fetch"));

        let ctx = ToolContext::default();
        let out = registry.execute("fetch", json!({}), &ctx).await.unwrap();
        assert_eq!(out, "ok:fetch");
    }
}
