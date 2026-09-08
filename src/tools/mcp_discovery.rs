//! Model Context Protocol (MCP) Server Auto-Discovery and Inspection Engine.
//!
//! Provides heterogeneous discovery, inspection, schema normalization, and validation
//! of external MCP servers configured across modern developer environments:
//! 1. Fusion workspace configurations (`.fusion/mcp.json`, `.fusion/mcp_servers.json`)
//! 2. Fusion global configuration (`~/.fusion/mcp.json`)
//! 3. Cursor IDE configurations (`.cursor/mcp.json`, `~/.cursor/mcp.json`)
//! 4. Visual Studio Code MCP configurations (`.vscode/mcp.json`)
//! 5. Claude Desktop & Claude Code configurations (`~/.claude.json`, `.claude/mcp.json`)
//!
//! Features:
//! - Normalizes heterogeneous JSON schemas (nested server maps, direct server maps, arrays, JSONC comments).
//! - Sanitizes sensitive environment variables (API keys, authentication tokens, passwords).
//! - Validates binary execution paths against the filesystem and system `PATH`.
//! - Implements the Fusion [`Tool`] trait via [`McpDiscoveryTool`].

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::RwLock;

use anyhow::Context;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::tools::types::{Tool, ToolContext};

// ===========================================================================
// McpServerConfig Struct
// ===========================================================================

/// Represents the configuration for an external Model Context Protocol (MCP) server.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct McpServerConfig {
    /// Unique identifier or server name (e.g., "filesystem", "github", "sqlite").
    pub name: String,

    /// Executable binary or script command (e.g., "node", "python", "npx", "uvx").
    pub command: String,

    /// Command-line arguments passed to the server process.
    #[serde(default)]
    pub args: Vec<String>,

    /// Environment variables passed to the server process (sanitized of sensitive secrets).
    #[serde(default)]
    pub env: HashMap<String, String>,

    /// Path to the configuration file where this server was discovered.
    #[serde(default)]
    pub source_file: PathBuf,
}

impl Default for McpServerConfig {
    fn default() -> Self {
        Self {
            name: String::new(),
            command: String::new(),
            args: Vec::new(),
            env: HashMap::new(),
            source_file: PathBuf::new(),
        }
    }
}

impl McpServerConfig {
    /// Creates a new MCP server configuration with the given name and command.
    pub fn new(name: impl Into<String>, command: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            command: command.into(),
            args: Vec::new(),
            env: HashMap::new(),
            source_file: PathBuf::new(),
        }
    }

    /// Appends a command-line argument.
    pub fn with_arg(mut self, arg: impl Into<String>) -> Self {
        self.args.push(arg.into());
        self
    }

    /// Appends multiple command-line arguments.
    pub fn with_args<I, S>(mut self, args: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        for a in args {
            self.args.push(a.into());
        }
        self
    }

    /// Adds an environment variable.
    pub fn with_env(mut self, key: impl Into<String>, val: impl Into<String>) -> Self {
        self.env.insert(key.into(), val.into());
        self
    }

    /// Sets the source file path.
    pub fn with_source_file(mut self, path: impl Into<PathBuf>) -> Self {
        self.source_file = path.into();
        self
    }

    /// Validates whether the server binary/executable exists on disk or on PATH.
    pub fn is_binary_valid(&self) -> bool {
        let base_dir = self.source_file.parent();
        validate_binary_path(&self.command, base_dir)
    }

    /// Resolves the command to a full executable path if found.
    pub fn resolve_binary(&self) -> Option<PathBuf> {
        let base_dir = self.source_file.parent();
        resolve_binary_path(&self.command, base_dir)
    }

    /// Sanitizes environment variables in-place, masking sensitive credentials and tokens.
    pub fn sanitize_env(&mut self) {
        self.env = sanitize_environment_map(&self.env);
    }
}

impl From<McpServerConfig> for crate::tools::mcp::McpServerConfig {
    fn from(cfg: McpServerConfig) -> Self {
        let cwd = cfg.source_file.parent().map(|p| p.to_path_buf());
        crate::tools::mcp::McpServerConfig {
            name: cfg.name,
            command: cfg.command,
            args: cfg.args,
            env: cfg.env,
            cwd,
            disabled: false,
            timeout_secs: Some(crate::tools::mcp::DEFAULT_TIMEOUT_SECS),
            prefix: None,
            auto_approve: Vec::new(),
        }
    }
}

// ===========================================================================
// JSONC Comment Stripper
// ===========================================================================

/// Strips single-line (`//`) and multi-line (`/* ... */`) comments from JSONC text,
/// respecting string literals and escaped quotes.
pub fn strip_json_comments(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let chars: Vec<char> = input.chars().collect();
    let len = chars.len();
    let mut i = 0;
    let mut in_string = false;
    let mut escape = false;

    while i < len {
        let c = chars[i];

        if in_string {
            out.push(c);
            if escape {
                escape = false;
            } else if c == '\\' {
                escape = true;
            } else if c == '"' {
                in_string = false;
            }
            i += 1;
            continue;
        }

        if c == '"' {
            in_string = true;
            out.push(c);
            i += 1;
            continue;
        }

        // Single-line comment: // ...
        if c == '/' && i + 1 < len && chars[i + 1] == '/' {
            i += 2;
            while i < len && chars[i] != '\n' && chars[i] != '\r' {
                i += 1;
            }
            continue;
        }

        // Block comment: /* ... */
        if c == '/' && i + 1 < len && chars[i + 1] == '*' {
            i += 2;
            while i + 1 < len && !(chars[i] == '*' && chars[i + 1] == '/') {
                i += 1;
            }
            if i + 1 < len {
                i += 2; // skip */
            } else {
                i = len;
            }
            continue;
        }

        out.push(c);
        i += 1;
    }

    out
}

// ===========================================================================
// Environment Sanitization & Secret Masking
// ===========================================================================

/// Determines whether an environment variable key or value contains sensitive information.
pub fn is_sensitive_variable(key: &str, val: &str) -> bool {
    let key_upper = key.to_uppercase();

    // Check known secret keywords in key name
    const SENSITIVE_KEYWORDS: &[&str] = &[
        "KEY",
        "TOKEN",
        "SECRET",
        "AUTH",
        "PASS",
        "CREDENTIAL",
        "PRIVATE",
        "BEARER",
        "SIGNATURE",
        "CERT",
        "TICKET",
    ];

    for kw in SENSITIVE_KEYWORDS {
        if key_upper.contains(kw) {
            return true;
        }
    }

    // Check with Fusion's dedicated env_cleaner
    crate::tools::env_cleaner::is_sensitive_key(key)
        || crate::tools::env_cleaner::is_sensitive_value(val)
}

/// Masks a sensitive credential value for safe display and logging.
pub fn mask_credential_value(val: &str) -> String {
    crate::tools::env_cleaner::mask_value(val)
}

/// Sanitizes an environment map by masking sensitive variable values.
pub fn sanitize_environment_map(env: &HashMap<String, String>) -> HashMap<String, String> {
    let mut clean = HashMap::with_capacity(env.len());
    for (k, v) in env {
        if is_sensitive_variable(k, v) {
            clean.insert(k.clone(), mask_credential_value(v));
        } else {
            clean.insert(k.clone(), v.clone());
        }
    }
    clean
}

// ===========================================================================
// Binary Path Validation & Resolution
// ===========================================================================

/// Searches the system `PATH` for an executable binary.
pub fn which(binary: &str) -> Option<PathBuf> {
    let path_var = std::env::var_os("PATH")?;
    for dir in std::env::split_paths(&path_var) {
        let candidate = dir.join(binary);
        if candidate.is_file() {
            return Some(candidate);
        }
        #[cfg(windows)]
        {
            for ext in &[".exe", ".cmd", ".bat"] {
                let with_ext = dir.join(format!("{}{}", binary, ext));
                if with_ext.is_file() {
                    return Some(with_ext);
                }
            }
        }
    }
    None
}

/// Resolves a command string to a valid filesystem path if it exists on disk or on PATH.
pub fn resolve_binary_path(command: &str, base_dir: Option<&Path>) -> Option<PathBuf> {
    let cmd = command.trim();
    if cmd.is_empty() {
        return None;
    }

    let path = Path::new(cmd);

    // 1. Absolute path check
    if path.is_absolute() {
        if path.is_file() || path.exists() {
            return Some(path.to_path_buf());
        }
        return None;
    }

    // 2. Home directory expansion (~/...)
    if cmd.starts_with("~/") || cmd == "~" {
        if let Some(home) = dirs::home_dir() {
            let relative = cmd.trim_start_matches("~/");
            let expanded = home.join(relative);
            if expanded.is_file() || expanded.exists() {
                return Some(expanded);
            }
        }
        return None;
    }

    // 3. Relative path with directory separators (./foo, ../bar, dir/bin)
    if cmd.contains('/') || cmd.contains('\\') {
        if let Some(base) = base_dir {
            let candidate = base.join(path);
            if candidate.is_file() || candidate.exists() {
                return Some(candidate);
            }
        }
        if let Ok(cwd) = std::env::current_dir() {
            let candidate = cwd.join(path);
            if candidate.is_file() || candidate.exists() {
                return Some(candidate);
            }
        }
        return None;
    }

    // 4. Bare command name: look up in PATH
    which(cmd)
}

/// Validates whether a command exists as an executable file on disk or on PATH.
pub fn validate_binary_path(command: &str, base_dir: Option<&Path>) -> bool {
    resolve_binary_path(command, base_dir).is_some()
}

// ===========================================================================
// McpDiscoveryEngine
// ===========================================================================

/// Multi-directory and format-agnostic MCP server discovery engine.
#[derive(Debug, Default, Clone)]
pub struct McpDiscoveryEngine;

impl McpDiscoveryEngine {
    /// Discovers all MCP servers across workspace and global configurations.
    ///
    /// Searches in order of precedence:
    /// 1. `.fusion/mcp.json` and `.fusion/mcp_servers.json` in workspace.
    /// 2. `~/.fusion/mcp.json` global config.
    /// 3. `.cursor/mcp.json` and `~/.cursor/mcp.json` (Cursor IDE configs).
    /// 4. `.vscode/mcp.json` (VS Code MCP configs).
    /// 5. `~/.claude.json` and `.claude/mcp.json` (Claude Desktop & Code configs).
    ///
    /// Merges servers where earlier/higher-precedence configs override later ones for each server name.
    pub fn discover_all(workspace_root: &Path) -> Vec<McpServerConfig> {
        Self::discover_all_with_home(workspace_root, dirs::home_dir().as_deref())
    }

    /// Discovers MCP servers strictly within the workspace directory.
    pub fn discover_workspace_only(workspace_root: &Path) -> Vec<McpServerConfig> {
        Self::discover_all_with_home(workspace_root, None)
    }

    /// Discovers MCP servers with an optional explicit home directory.
    pub fn discover_all_with_home(
        workspace_root: &Path,
        home_dir: Option<&Path>,
    ) -> Vec<McpServerConfig> {
        let paths = Self::candidate_paths_with_home(workspace_root, home_dir);
        let mut configs = Vec::new();
        let mut seen_names = HashSet::new();
        let mut seen_files = HashSet::new();

        for path in paths {
            if path.is_file() {
                let canonical = path.canonicalize().unwrap_or_else(|_| path.clone());
                if !seen_files.insert(canonical) {
                    continue;
                }

                match Self::load_file(&path) {
                    Ok(file_configs) => {
                        for cfg in file_configs {
                            if seen_names.insert(cfg.name.clone()) {
                                configs.push(cfg);
                            }
                        }
                    }
                    Err(e) => {
                        tracing::warn!("Failed to load MCP config from {}: {}", path.display(), e);
                    }
                }
            }
        }

        configs
    }

    /// Returns all candidate configuration file paths in precedence order.
    pub fn candidate_paths(workspace_root: &Path) -> Vec<PathBuf> {
        Self::candidate_paths_with_home(workspace_root, dirs::home_dir().as_deref())
    }

    /// Returns candidate configuration file paths with an optional explicit home directory.
    pub fn candidate_paths_with_home(
        workspace_root: &Path,
        home_dir: Option<&Path>,
    ) -> Vec<PathBuf> {
        let mut candidates = Vec::new();

        // 1. Workspace: .fusion/mcp.json and .fusion/mcp_servers.json
        candidates.push(workspace_root.join(".fusion").join("mcp.json"));
        candidates.push(workspace_root.join(".fusion").join("mcp_servers.json"));

        // 3. Workspace: .cursor/mcp.json
        candidates.push(workspace_root.join(".cursor").join("mcp.json"));

        // 4. Workspace: .vscode/mcp.json
        candidates.push(workspace_root.join(".vscode").join("mcp.json"));

        // 5. Workspace: .claude/mcp.json and .claude.json
        candidates.push(workspace_root.join(".claude").join("mcp.json"));
        candidates.push(workspace_root.join(".claude.json"));

        // Global / Home configurations
        if let Some(home) = home_dir {
            // 2. ~/.fusion/mcp.json global config
            candidates.push(home.join(".fusion").join("mcp.json"));
            candidates.push(home.join(".fusion").join("mcp_servers.json"));

            // 3. ~/.cursor/mcp.json
            candidates.push(home.join(".cursor").join("mcp.json"));

            // 4. ~/.vscode/mcp.json
            candidates.push(home.join(".vscode").join("mcp.json"));

            // 5. ~/.claude.json and ~/.claude/mcp.json
            candidates.push(home.join(".claude.json"));
            candidates.push(home.join(".claude").join("mcp.json"));

            // Platform-specific Claude Desktop config locations
            #[cfg(target_os = "macos")]
            candidates
                .push(home.join("Library/Application Support/Claude/claude_desktop_config.json"));

            #[cfg(target_os = "windows")]
            if let Ok(appdata) = std::env::var("APPDATA") {
                candidates.push(
                    PathBuf::from(appdata)
                        .join("Claude")
                        .join("claude_desktop_config.json"),
                );
            }

            #[cfg(all(unix, not(target_os = "macos")))]
            candidates.push(
                home.join(".config")
                    .join("claude")
                    .join("claude_desktop_config.json"),
            );
        }

        candidates
    }

    /// Loads and parses server configurations from a single file on disk.
    pub fn load_file(path: &Path) -> anyhow::Result<Vec<McpServerConfig>> {
        let content = std::fs::read_to_string(path)
            .with_context(|| format!("Could not read MCP config file at '{}'", path.display()))?;
        Self::parse_json_str(&content, path)
    }

    /// Parses a JSON or JSONC string into a list of [`McpServerConfig`]s.
    pub fn parse_json_str(
        content: &str,
        source_file: &Path,
    ) -> anyhow::Result<Vec<McpServerConfig>> {
        let clean = strip_json_comments(content);
        let val: Value = serde_json::from_str(&clean)
            .with_context(|| format!("Invalid JSON in '{}'", source_file.display()))?;
        Self::parse_value(&val, source_file)
    }

    /// Normalizes heterogeneous JSON values into a uniform list of [`McpServerConfig`]s.
    pub fn parse_value(val: &Value, source_file: &Path) -> anyhow::Result<Vec<McpServerConfig>> {
        let mut configs = Vec::new();

        // Case 1: Top-level array of server configurations
        if let Some(arr) = val.as_array() {
            for (idx, item) in arr.iter().enumerate() {
                let default_name = format!("server_{}", idx + 1);
                if let Some(cfg) = parse_server_item(item, Some(&default_name), source_file) {
                    configs.push(cfg);
                }
            }
            return Ok(configs);
        }

        // Case 2: Object with sections or nested mappings
        if let Some(obj) = val.as_object() {
            let mut matched_sections = false;

            // Check standard section keys: "mcpServers", "mcp_servers", "servers", "globalMcpServers"
            let section_keys = ["mcpServers", "mcp_servers", "servers", "globalMcpServers"];
            for key in section_keys {
                if let Some(section_val) = obj.get(key) {
                    matched_sections = true;
                    if let Some(section_obj) = section_val.as_object() {
                        for (name, item) in section_obj {
                            if let Some(cfg) = parse_server_item(item, Some(name), source_file) {
                                configs.push(cfg);
                            }
                        }
                    } else if let Some(section_arr) = section_val.as_array() {
                        for (idx, item) in section_arr.iter().enumerate() {
                            let default_name = format!("{}_{}", key, idx + 1);
                            if let Some(cfg) =
                                parse_server_item(item, Some(&default_name), source_file)
                            {
                                configs.push(cfg);
                            }
                        }
                    }
                }
            }

            // Check VS Code style: { "mcp": { "servers": { ... } } }
            if let Some(mcp_val) = obj.get("mcp") {
                if let Some(mcp_obj) = mcp_val.as_object() {
                    if let Some(servers_val) = mcp_obj.get("servers") {
                        matched_sections = true;
                        if let Some(servers_obj) = servers_val.as_object() {
                            for (name, item) in servers_obj {
                                if let Some(cfg) = parse_server_item(item, Some(name), source_file)
                                {
                                    configs.push(cfg);
                                }
                            }
                        } else if let Some(servers_arr) = servers_val.as_array() {
                            for (idx, item) in servers_arr.iter().enumerate() {
                                let default_name = format!("vscode_server_{}", idx + 1);
                                if let Some(cfg) =
                                    parse_server_item(item, Some(&default_name), source_file)
                                {
                                    configs.push(cfg);
                                }
                            }
                        }
                    }
                }
            }

            // Check Claude Code projects style: { "projects": { "<path>": { "mcpServers": { ... } } } }
            if let Some(projects_val) = obj.get("projects") {
                if let Some(projects_obj) = projects_val.as_object() {
                    for (_proj_path, proj_conf) in projects_obj {
                        if let Some(mcp_servers) = proj_conf.get("mcpServers") {
                            matched_sections = true;
                            if let Some(servers_obj) = mcp_servers.as_object() {
                                for (name, item) in servers_obj {
                                    if let Some(cfg) =
                                        parse_server_item(item, Some(name), source_file)
                                    {
                                        configs.push(cfg);
                                    }
                                }
                            }
                        }
                    }
                }
            }

            if matched_sections {
                return Ok(configs);
            }

            // Case 3: Direct map of server name -> server config
            // Identified if object values are objects with a command/cmd property
            let is_direct_server_map = obj.values().any(|v| {
                v.is_object()
                    && (v.get("command").is_some()
                        || v.get("cmd").is_some()
                        || v.get("executable").is_some()
                        || v.get("bin").is_some())
            });

            if is_direct_server_map {
                for (name, item) in obj {
                    if let Some(cfg) = parse_server_item(item, Some(name), source_file) {
                        configs.push(cfg);
                    }
                }
                return Ok(configs);
            }

            // Case 4: Single server config at root level
            if obj.get("command").is_some()
                || obj.get("cmd").is_some()
                || obj.get("executable").is_some()
                || obj.get("bin").is_some()
            {
                if let Some(cfg) = parse_server_item(val, None, source_file) {
                    return Ok(vec![cfg]);
                }
            }
        }

        Ok(configs)
    }

    /// Validates whether a command exists on disk or on PATH.
    pub fn validate_binary(command: &str, base_dir: Option<&Path>) -> bool {
        validate_binary_path(command, base_dir)
    }

    /// Resolves a binary command to a full path if it exists.
    pub fn resolve_binary(command: &str, base_dir: Option<&Path>) -> Option<PathBuf> {
        resolve_binary_path(command, base_dir)
    }

    /// Sanitizes an environment map by masking sensitive variables.
    pub fn sanitize_env(env: &HashMap<String, String>) -> HashMap<String, String> {
        sanitize_environment_map(env)
    }
}

// ===========================================================================
// Server Item Parser Helper
// ===========================================================================

fn parse_server_item(
    item: &Value,
    default_name: Option<&str>,
    source_file: &Path,
) -> Option<McpServerConfig> {
    let obj = item.as_object()?;

    // 1. Extract command
    let (command, extra_args) = extract_command(obj)?;

    // 2. Extract name
    let name = obj
        .get("name")
        .or_else(|| obj.get("id"))
        .or_else(|| obj.get("serverName"))
        .or_else(|| obj.get("server_name"))
        .and_then(|v| v.as_str())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .or_else(|| default_name.map(|s| s.trim().to_string()))
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| {
            source_file
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("mcp_server")
                .to_string()
        });

    // 3. Extract args
    let mut args = extra_args;
    if let Some(args_val) = obj
        .get("args")
        .or_else(|| obj.get("arguments"))
        .or_else(|| obj.get("parameters"))
    {
        if let Some(arr) = args_val.as_array() {
            for a in arr {
                if let Some(s) = a.as_str() {
                    args.push(s.to_string());
                } else {
                    args.push(a.to_string());
                }
            }
        } else if let Some(s) = args_val.as_str() {
            for part in s.split_whitespace() {
                args.push(part.to_string());
            }
        }
    }

    // 4. Extract env
    let mut env = HashMap::new();
    if let Some(env_val) = obj
        .get("env")
        .or_else(|| obj.get("environment"))
        .or_else(|| obj.get("envVars"))
        .or_else(|| obj.get("environmentVariables"))
    {
        if let Some(env_obj) = env_val.as_object() {
            for (k, v) in env_obj {
                let v_str = if let Some(s) = v.as_str() {
                    s.to_string()
                } else {
                    v.to_string()
                };
                env.insert(k.clone(), v_str);
            }
        }
    }

    // Sanitize environment variables
    let sanitized_env = sanitize_environment_map(&env);

    Some(McpServerConfig {
        name,
        command,
        args,
        env: sanitized_env,
        source_file: source_file.to_path_buf(),
    })
}

fn extract_command(obj: &serde_json::Map<String, Value>) -> Option<(String, Vec<String>)> {
    let cmd_val = obj
        .get("command")
        .or_else(|| obj.get("cmd"))
        .or_else(|| obj.get("executable"))
        .or_else(|| obj.get("bin"))
        .or_else(|| obj.get("path"))?;

    if let Some(s) = cmd_val.as_str() {
        let trimmed = s.trim();
        if trimmed.is_empty() {
            return None;
        }
        Some((trimmed.to_string(), Vec::new()))
    } else if let Some(arr) = cmd_val.as_array() {
        let mut it = arr.iter();
        let first = it.next()?.as_str()?.trim().to_string();
        if first.is_empty() {
            return None;
        }
        let rest = it
            .filter_map(|v| v.as_str().map(|s| s.to_string()))
            .collect();
        Some((first, rest))
    } else {
        None
    }
}

// ===========================================================================
// McpDiscoveryTool
// ===========================================================================

/// Tool for discovering and inspecting MCP servers configured across Fusion, Cursor, VS Code, and Claude.
#[derive(Debug, Default)]
pub struct McpDiscoveryTool {
    cache: RwLock<HashMap<PathBuf, Vec<McpServerConfig>>>,
}

impl McpDiscoveryTool {
    /// Creates a new instance of [`McpDiscoveryTool`].
    pub fn new() -> Self {
        Self {
            cache: RwLock::new(HashMap::new()),
        }
    }

    /// Clears the internal discovery cache.
    pub fn clear_cache(&self) {
        if let Ok(mut c) = self.cache.write() {
            c.clear();
        }
    }
}

#[async_trait]
impl Tool for McpDiscoveryTool {
    fn name(&self) -> &str {
        "mcp_discovery"
    }

    fn description(&self) -> &str {
        "Discovers and inspects all MCP servers configured across Fusion, Cursor, VS Code, and Claude."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "Optional workspace root directory to scan for MCP configurations. Defaults to current working directory."
                },
                "reload": {
                    "type": "boolean",
                    "description": "Whether to bypass cached discovery results and perform a fresh scan. Defaults to false."
                },
                "workspace_only": {
                    "type": "boolean",
                    "description": "Whether to restrict discovery strictly to workspace configurations, ignoring global configs. Defaults to false."
                },
                "format": {
                    "type": "string",
                    "enum": ["summary", "json", "detailed"],
                    "description": "Output formatting style: 'summary' (human-readable table), 'detailed' (full details with validation), or 'json' (structured JSON array). Defaults to 'summary'."
                }
            }
        })
    }

    async fn execute(&self, args: Value, ctx: &ToolContext) -> anyhow::Result<String> {
        let root = if let Some(p) = args
            .get("path")
            .and_then(|v| v.as_str())
            .filter(|s| !s.trim().is_empty())
        {
            PathBuf::from(p)
        } else {
            ctx.cwd.clone()
        };

        let force_reload = args
            .get("reload")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        let workspace_only = args
            .get("workspace_only")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        let format = args
            .get("format")
            .and_then(|v| v.as_str())
            .unwrap_or("summary");

        let configs = if !force_reload {
            let cached = self.cache.read().ok().and_then(|c| c.get(&root).cloned());
            if let Some(c) = cached {
                c
            } else {
                let fresh = if workspace_only {
                    McpDiscoveryEngine::discover_workspace_only(&root)
                } else {
                    McpDiscoveryEngine::discover_all(&root)
                };
                if let Ok(mut c) = self.cache.write() {
                    c.insert(root.clone(), fresh.clone());
                }
                fresh
            }
        } else {
            let fresh = if workspace_only {
                McpDiscoveryEngine::discover_workspace_only(&root)
            } else {
                McpDiscoveryEngine::discover_all(&root)
            };
            if let Ok(mut c) = self.cache.write() {
                c.insert(root.clone(), fresh.clone());
            }
            fresh
        };

        if configs.is_empty() {
            return Ok(format!(
                "No MCP servers discovered in workspace '{}' or global configurations.\n\
                Checked paths:\n\
                - .fusion/mcp.json, .fusion/mcp_servers.json\n\
                - ~/.fusion/mcp.json\n\
                - .cursor/mcp.json, ~/.cursor/mcp.json\n\
                - .vscode/mcp.json\n\
                - ~/.claude.json, .claude/mcp.json",
                root.display()
            ));
        }

        if format == "json" {
            return Ok(serde_json::to_string_pretty(&configs)?);
        }

        // Render human-readable summary
        let mut out = String::new();
        out.push_str(&format!(
            "Discovered {} MCP server(s) across configuration files:\n\n",
            configs.len()
        ));

        for (idx, cfg) in configs.iter().enumerate() {
            let resolved = cfg.resolve_binary();
            let status = match &resolved {
                Some(p) => format!("✓ valid ({})", p.display()),
                None => format!("✗ binary '{}' not found on PATH or disk", cfg.command),
            };

            out.push_str(&format!(
                "{}. [{}]\n\
                 - Command: {}\n\
                 - Status: {}\n\
                 - Args: {}\n\
                 - Source: {}\n",
                idx + 1,
                cfg.name,
                cfg.command,
                status,
                if cfg.args.is_empty() {
                    "(none)".to_string()
                } else {
                    cfg.args.join(" ")
                },
                cfg.source_file.display()
            ));

            if !cfg.env.is_empty() {
                out.push_str(" - Environment (sanitized):\n");
                let mut sorted_keys: Vec<_> = cfg.env.keys().collect();
                sorted_keys.sort();
                for k in sorted_keys {
                    let v = &cfg.env[k];
                    out.push_str(&format!("     * {}={}\n", k, v));
                }
            }

            out.push('\n');
        }

        Ok(out.trim_end().to_string())
    }
}
