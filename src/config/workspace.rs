//! Per-workspace configuration overrides (`.fusion.toml` / `.fusion.json`).
//!
//! Allows repositories and project workspaces to define local overrides for:
//! - Default LLM provider and model (e.g. workspace-specific Claude 3.7 or DeepSeek-R1)
//! - Sampling parameters (temperature, max token limit)
//! - Multi-agent & advisor settings (enable/disable advisors, advisor model)
//! - Audio cues & desktop notification preferences
//! - Workspace instructions & system prompt guidelines
//! - Custom rules & file ignore patterns
//! - Workspace-specific environment variables
//! - Model Context Protocol (MCP) server configurations & tool permissions
//!
//! # Configuration Precedence
//! 1. Built-in defaults
//! 2. Global user configuration (`~/.fusion/config.json`)
//! 3. **Workspace configuration (`.fusion.toml` / `.fusion.json` in project root)**
//! 4. Environment variables (`FUSION_MODEL`, `DEEPSEEK_API_KEY`, etc.)
//! 5. CLI arguments (`--model`, `--provider`, `--no-advisors`)

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::collections::HashMap;
use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};

use crate::config::Config;

/// Standard file names checked for workspace configuration in priority order.
pub const WORKSPACE_CONFIG_FILES: &[&str] = &[
    ".fusion.toml",
    ".fusion.json",
    "fusion.toml",
    "fusion.json",
    ".fusion/config.toml",
    ".fusion/config.json",
    ".fusion/workspace.toml",
    ".fusion/workspace.json",
];

/// Supported workspace configuration file formats.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum WorkspaceConfigFormat {
    /// TOML format (`.fusion.toml` or `fusion.toml`).
    Toml,
    /// JSON format (`.fusion.json` or `fusion.json`).
    Json,
}

impl WorkspaceConfigFormat {
    /// Detects format from file extension.
    pub fn from_path(path: &Path) -> Option<Self> {
        let ext = path.extension()?.to_str()?.to_ascii_lowercase();
        match ext.as_str() {
            "toml" => Some(Self::Toml),
            "json" => Some(Self::Json),
            _ => None,
        }
    }

    /// Primary file extension for this format.
    pub const fn extension(&self) -> &'static str {
        match self {
            Self::Toml => "toml",
            Self::Json => "json",
        }
    }
}

impl fmt::Display for WorkspaceConfigFormat {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Toml => write!(f, "TOML"),
            Self::Json => write!(f, "JSON"),
        }
    }
}

/// Errors that can occur when discovering, parsing, or applying workspace configuration.
#[derive(Debug, thiserror::Error)]
pub enum WorkspaceConfigError {
    #[error("IO error loading workspace config '{path}': {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("JSON parse error in workspace config '{path}': {source}")]
    JsonParse {
        path: PathBuf,
        #[source]
        source: serde_json::Error,
    },

    #[error("TOML syntax error in workspace config '{path}' at line {line}, col {col}: {message}")]
    TomlParse {
        path: PathBuf,
        line: usize,
        col: usize,
        message: String,
    },

    #[error("Validation error in workspace config '{path}': {message}")]
    Validation {
        path: PathBuf,
        message: String,
    },

    #[error("Unsupported workspace config format for '{path}'. Expected .toml or .json")]
    UnsupportedFormat {
        path: PathBuf,
    },
}

/// Workspace-level MCP server definition.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct WorkspaceMcpServerConfig {
    /// Command executable (e.g. `npx`, `python`, `uvx`, `docker`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub command: Option<String>,

    /// Command arguments.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub args: Vec<String>,

    /// Environment variables specific to this MCP server.
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub env: HashMap<String, String>,

    /// Working directory for the MCP server process.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cwd: Option<PathBuf>,

    /// Whether this server is temporarily disabled.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub disabled: Option<bool>,

    /// Transport type: "stdio", "sse", or "http".
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub transport: Option<String>,

    /// Remote endpoint URL for SSE/HTTP transports.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
}

/// Tool execution permissions and sandbox constraints for the workspace.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct WorkspaceToolSettings {
    /// Explicitly enabled tools in this workspace.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub enabled: Vec<String>,

    /// Explicitly disabled tools in this workspace.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub disabled: Vec<String>,

    /// Maximum bash tool execution timeout in seconds.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bash_timeout_secs: Option<u64>,

    /// Auto-approve filesystem read operations without user prompt.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auto_approve_reads: Option<bool>,

    /// Auto-approve filesystem write/edit operations without user prompt.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auto_approve_writes: Option<bool>,

    /// Auto-approve bash execution without user prompt.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auto_approve_bash: Option<bool>,
}

impl WorkspaceToolSettings {
    pub fn is_default(&self) -> bool {
        self.enabled.is_empty()
            && self.disabled.is_empty()
            && self.bash_timeout_secs.is_none()
            && self.auto_approve_reads.is_none()
            && self.auto_approve_writes.is_none()
            && self.auto_approve_bash.is_none()
    }
}

/// Per-workspace configuration overrides loaded from `.fusion.toml` or `.fusion.json`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct WorkspaceConfig {
    /// Configuration schema version.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<u32>,

    /// Friendly workspace name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,

    /// Workspace description or domain summary.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,

    // Provider & Model Overrides
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        alias = "provider",
        alias = "model_provider"
    )]
    pub default_provider: Option<String>,

    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        alias = "model",
        alias = "selected_model"
    )]
    pub default_model: Option<String>,

    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        alias = "temperature",
        alias = "temp"
    )]
    pub default_temperature: Option<f32>,

    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        alias = "tokens",
        alias = "max_token"
    )]
    pub max_tokens: Option<u32>,

    // Provider API Keys & Base URLs
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        alias = "openai_key",
        alias = "open_ai_api_key"
    )]
    pub openai_api_key: Option<String>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        alias = "openai_url",
        alias = "openai_endpoint"
    )]
    pub openai_base_url: Option<String>,

    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        alias = "anthropic_key",
        alias = "claude_api_key"
    )]
    pub anthropic_api_key: Option<String>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        alias = "anthropic_url",
        alias = "anthropic_endpoint"
    )]
    pub anthropic_base_url: Option<String>,

    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        alias = "deepseek_key",
        alias = "deep_seek_api_key"
    )]
    pub deepseek_api_key: Option<String>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        alias = "deepseek_url",
        alias = "deepseek_endpoint"
    )]
    pub deepseek_base_url: Option<String>,

    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        alias = "xai_key",
        alias = "grok_key"
    )]
    pub xai_api_key: Option<String>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        alias = "xai_url",
        alias = "grok_url"
    )]
    pub xai_base_url: Option<String>,

    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        alias = "openrouter_key",
        alias = "open_router_api_key"
    )]
    pub openrouter_api_key: Option<String>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        alias = "openrouter_url"
    )]
    pub openrouter_base_url: Option<String>,

    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        alias = "ollama_url",
        alias = "ollama_host",
        alias = "ollama_endpoint"
    )]
    pub ollama_base_url: Option<String>,

    // Multi-Agent & Advisor Overrides
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        alias = "advisors",
        alias = "enable_advisors"
    )]
    pub advisors_enabled: Option<bool>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub advisor_model: Option<String>,

    // Audio & Terminal Sound Cues
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        alias = "sound",
        alias = "audio",
        alias = "bell"
    )]
    pub sound_enabled: Option<bool>,

    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        alias = "bell_completion"
    )]
    pub bell_on_completion: Option<bool>,

    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        alias = "bell_error"
    )]
    pub bell_on_error: Option<bool>,

    // Desktop & Terminal Notifications
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        alias = "notify",
        alias = "notifications"
    )]
    pub notify_enabled: Option<bool>,

    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        alias = "notify_completion"
    )]
    pub notify_on_completion: Option<bool>,

    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        alias = "notify_error"
    )]
    pub notify_on_error: Option<bool>,

    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        alias = "notify_min_duration"
    )]
    pub notify_min_duration_secs: Option<f64>,

    // Custom Workspace Instructions & Rules
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        alias = "system_prompt",
        alias = "prompt"
    )]
    pub instructions: Option<String>,

    /// Additional content appended to the system prompt after global instructions.
    /// Unlike `instructions` (which replaces the workspace prompt entirely), this
    /// text is always appended on top of whatever the global config provides.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        alias = "system_prompt_append",
        alias = "prompt_extra"
    )]
    pub system_prompt_extra: Option<String>,

    #[serde(default, skip_serializing_if = "Vec::is_empty", alias = "rules")]
    pub custom_rules: Vec<String>,

    #[serde(
        default,
        skip_serializing_if = "Vec::is_empty",
        alias = "ignore",
        alias = "ignore_patterns"
    )]
    pub ignored_patterns: Vec<String>,

    // Tool allow/deny lists — shorthand at the workspace level.
    // These merge with `tools.enabled` / `tools.disabled` when applying to Config.
    /// Explicit tool allow-list: only these tools may run (empty = no restriction).
    #[serde(
        default,
        skip_serializing_if = "Vec::is_empty",
        alias = "tools_allow",
        alias = "allowed_tools"
    )]
    pub tool_allow: Vec<String>,

    /// Explicit tool deny-list: these tools are always blocked.
    #[serde(
        default,
        skip_serializing_if = "Vec::is_empty",
        alias = "tools_deny",
        alias = "blocked_tools",
        alias = "denied_tools"
    )]
    pub tool_deny: Vec<String>,

    // Workspace-specific Environment Variables
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub env: HashMap<String, String>,

    // Workspace-specific MCP Servers
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub mcp_servers: HashMap<String, WorkspaceMcpServerConfig>,

    // Workspace Tool Settings
    #[serde(default, skip_serializing_if = "WorkspaceToolSettings::is_default")]
    pub tools: WorkspaceToolSettings,
}

/// Entry representing a single configuration override field.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkspaceOverrideEntry {
    pub field: String,
    pub old_value: String,
    pub new_value: String,
}

/// Metadata and contents of an actively loaded workspace configuration.
#[derive(Debug, Clone, PartialEq)]
pub struct LoadedWorkspaceConfig {
    /// Parsed workspace configuration values.
    pub config: WorkspaceConfig,
    /// Absolute path to the configuration file.
    pub source_path: PathBuf,
    /// Format (TOML or JSON).
    pub format: WorkspaceConfigFormat,
    /// Names of all fields overridden by this workspace configuration.
    pub overridden_fields: Vec<String>,
}

impl LoadedWorkspaceConfig {
    /// Formats a concise human-readable summary of overrides for terminal display.
    pub fn summary(&self) -> String {
        if self.overridden_fields.is_empty() {
            format!(
                "Workspace config loaded from {} (no overrides)",
                self.source_path.display()
            )
        } else {
            format!(
                "Workspace config loaded from {} (overrides: {})",
                self.source_path.display(),
                self.overridden_fields.join(", ")
            )
        }
    }
}

impl WorkspaceConfig {
    /// Parses workspace configuration from a TOML string.
    pub fn from_toml_str(content: &str) -> Result<Self, WorkspaceConfigError> {
        let dummy_path = PathBuf::from(".fusion.toml");
        Self::from_toml_str_with_path(content, &dummy_path)
    }

    /// Parses workspace configuration from a TOML string with an explicit path for error reporting.
    pub fn from_toml_str_with_path(
        content: &str,
        path: &Path,
    ) -> Result<Self, WorkspaceConfigError> {
        let parsed_val = parse_toml_to_json_value(content, path)?;
        let normalized_val = normalize_workspace_json_value(parsed_val);

        serde_json::from_value::<Self>(normalized_val).map_err(|e| {
            WorkspaceConfigError::TomlParse {
                path: path.to_path_buf(),
                line: 1,
                col: 1,
                message: format!("Failed to map TOML into workspace config: {}", e),
            }
        })
    }

    /// Parses workspace configuration from a JSON string.
    pub fn from_json_str(content: &str) -> Result<Self, WorkspaceConfigError> {
        let dummy_path = PathBuf::from(".fusion.json");
        Self::from_json_str_with_path(content, &dummy_path)
    }

    /// Parses workspace configuration from a JSON string with an explicit path for error reporting.
    pub fn from_json_str_with_path(
        content: &str,
        path: &Path,
    ) -> Result<Self, WorkspaceConfigError> {
        let parsed_val: Value =
            serde_json::from_str(content).map_err(|e| WorkspaceConfigError::JsonParse {
                path: path.to_path_buf(),
                source: e,
            })?;
        let normalized_val = normalize_workspace_json_value(parsed_val);

        serde_json::from_value::<Self>(normalized_val).map_err(|e| {
            WorkspaceConfigError::JsonParse {
                path: path.to_path_buf(),
                source: e,
            }
        })
    }

    /// Loads workspace configuration from a specific file path.
    pub fn load_from_file(path: &Path) -> Result<Self, WorkspaceConfigError> {
        let content = fs::read_to_string(path).map_err(|e| WorkspaceConfigError::Io {
            path: path.to_path_buf(),
            source: e,
        })?;

        let format = WorkspaceConfigFormat::from_path(path).ok_or_else(|| {
            WorkspaceConfigError::UnsupportedFormat {
                path: path.to_path_buf(),
            }
        })?;

        match format {
            WorkspaceConfigFormat::Toml => Self::from_toml_str_with_path(&content, path),
            WorkspaceConfigFormat::Json => Self::from_json_str_with_path(&content, path),
        }
    }

    /// Discovers and loads a workspace configuration file from a directory.
    pub fn find_and_load(
        workspace_dir: &Path,
    ) -> Result<Option<LoadedWorkspaceConfig>, WorkspaceConfigError> {
        if let Some(config_path) = find_workspace_config_file(workspace_dir, true) {
            let format = WorkspaceConfigFormat::from_path(&config_path).ok_or_else(|| {
                WorkspaceConfigError::UnsupportedFormat {
                    path: config_path.clone(),
                }
            })?;
            let cfg = Self::load_from_file(&config_path)?;
            let overridden_fields = cfg.detect_overridden_fields();
            Ok(Some(LoadedWorkspaceConfig {
                config: cfg,
                source_path: config_path,
                format,
                overridden_fields,
            }))
        } else {
            Ok(None)
        }
    }

    /// Detects the list of fields that this workspace configuration defines.
    pub fn detect_overridden_fields(&self) -> Vec<String> {
        let mut fields = Vec::new();

        if self.default_provider.is_some() {
            fields.push("default_provider".to_string());
        }
        if self.default_model.is_some() {
            fields.push("default_model".to_string());
        }
        if self.default_temperature.is_some() {
            fields.push("default_temperature".to_string());
        }
        if self.max_tokens.is_some() {
            fields.push("max_tokens".to_string());
        }
        if self.openai_api_key.is_some() {
            fields.push("openai_api_key".to_string());
        }
        if self.openai_base_url.is_some() {
            fields.push("openai_base_url".to_string());
        }
        if self.anthropic_api_key.is_some() {
            fields.push("anthropic_api_key".to_string());
        }
        if self.anthropic_base_url.is_some() {
            fields.push("anthropic_base_url".to_string());
        }
        if self.deepseek_api_key.is_some() {
            fields.push("deepseek_api_key".to_string());
        }
        if self.deepseek_base_url.is_some() {
            fields.push("deepseek_base_url".to_string());
        }
        if self.xai_api_key.is_some() {
            fields.push("xai_api_key".to_string());
        }
        if self.xai_base_url.is_some() {
            fields.push("xai_base_url".to_string());
        }
        if self.openrouter_api_key.is_some() {
            fields.push("openrouter_api_key".to_string());
        }
        if self.openrouter_base_url.is_some() {
            fields.push("openrouter_base_url".to_string());
        }
        if self.ollama_base_url.is_some() {
            fields.push("ollama_base_url".to_string());
        }
        if self.advisors_enabled.is_some() {
            fields.push("advisors_enabled".to_string());
        }
        if self.advisor_model.is_some() {
            fields.push("advisor_model".to_string());
        }
        if self.sound_enabled.is_some() {
            fields.push("sound_enabled".to_string());
        }
        if self.bell_on_completion.is_some() {
            fields.push("bell_on_completion".to_string());
        }
        if self.bell_on_error.is_some() {
            fields.push("bell_on_error".to_string());
        }
        if self.notify_enabled.is_some() {
            fields.push("notify_enabled".to_string());
        }
        if self.notify_on_completion.is_some() {
            fields.push("notify_on_completion".to_string());
        }
        if self.notify_on_error.is_some() {
            fields.push("notify_on_error".to_string());
        }
        if self.notify_min_duration_secs.is_some() {
            fields.push("notify_min_duration_secs".to_string());
        }
        if self.instructions.is_some() {
            fields.push("instructions".to_string());
        }
        if !self.custom_rules.is_empty() {
            fields.push("custom_rules".to_string());
        }
        if !self.ignored_patterns.is_empty() {
            fields.push("ignored_patterns".to_string());
        }
        if !self.env.is_empty() {
            fields.push("env".to_string());
        }
        if !self.mcp_servers.is_empty() {
            fields.push("mcp_servers".to_string());
        }
        if !self.tools.is_default() {
            fields.push("tools".to_string());
        }

        fields
    }

    /// Applies workspace overrides in-place to a target `Config`.
    pub fn apply_to(&self, config: &mut Config) -> Vec<WorkspaceOverrideEntry> {
        let mut diffs = Vec::new();

        // 1. Provider
        if let Some(p) = &self.default_provider {
            let p_clean = p.trim().to_lowercase();
            if !p_clean.is_empty() && p_clean != config.default_provider {
                diffs.push(WorkspaceOverrideEntry {
                    field: "default_provider".to_string(),
                    old_value: config.default_provider.clone(),
                    new_value: p_clean.clone(),
                });
                config.default_provider = p_clean;
            }
        }

        // 2. Model (resolve aliases/shorthands against the provider)
        if let Some(m) = &self.default_model {
            let m_trim = m.trim();
            if !m_trim.is_empty() {
                let (resolved_provider, resolved_model) =
                    Config::resolve_model(m_trim, Some(&config.default_provider));
                if self.default_provider.is_none() && resolved_provider != config.default_provider {
                    diffs.push(WorkspaceOverrideEntry {
                        field: "default_provider".to_string(),
                        old_value: config.default_provider.clone(),
                        new_value: resolved_provider.clone(),
                    });
                    config.default_provider = resolved_provider;
                }
                if resolved_model != config.default_model {
                    diffs.push(WorkspaceOverrideEntry {
                        field: "default_model".to_string(),
                        old_value: config.default_model.clone(),
                        new_value: resolved_model.clone(),
                    });
                    config.default_model = resolved_model;
                }
            }
        }

        // 3. Temperature
        if let Some(temp) = self.default_temperature {
            let clamped = temp.clamp(0.0, 2.0);
            if config.default_temperature != Some(clamped) {
                diffs.push(WorkspaceOverrideEntry {
                    field: "default_temperature".to_string(),
                    old_value: format!("{:?}", config.default_temperature),
                    new_value: format!("{:?}", Some(clamped)),
                });
                config.default_temperature = Some(clamped);
            }
        }

        // 4. Max Tokens
        if let Some(tok) = self.max_tokens {
            if tok > 0 && config.max_tokens != Some(tok) {
                diffs.push(WorkspaceOverrideEntry {
                    field: "max_tokens".to_string(),
                    old_value: format!("{:?}", config.max_tokens),
                    new_value: format!("{:?}", Some(tok)),
                });
                config.max_tokens = Some(tok);
            }
        }

        // 5. API Keys & Base URLs
        if let Some(k) = &self.openai_api_key {
            if let Some(clean) = crate::config::sanitize_env_var(k) {
                if config.openai_api_key.as_deref() != Some(&clean) {
                    diffs.push(WorkspaceOverrideEntry {
                        field: "openai_api_key".to_string(),
                        old_value: mask_key_preview(&config.openai_api_key),
                        new_value: mask_key_preview(&Some(clean.clone())),
                    });
                    config.openai_api_key = Some(clean);
                }
            }
        }
        if let Some(u) = &self.openai_base_url {
            if let Some(clean) = crate::config::sanitize_base_url(u) {
                if config.openai_base_url.as_deref() != Some(&clean) {
                    diffs.push(WorkspaceOverrideEntry {
                        field: "openai_base_url".to_string(),
                        old_value: config.openai_base_url.clone().unwrap_or_default(),
                        new_value: clean.clone(),
                    });
                    config.openai_base_url = Some(clean);
                }
            }
        }

        if let Some(k) = &self.anthropic_api_key {
            if let Some(clean) = crate::config::sanitize_env_var(k) {
                if config.anthropic_api_key.as_deref() != Some(&clean) {
                    diffs.push(WorkspaceOverrideEntry {
                        field: "anthropic_api_key".to_string(),
                        old_value: mask_key_preview(&config.anthropic_api_key),
                        new_value: mask_key_preview(&Some(clean.clone())),
                    });
                    config.anthropic_api_key = Some(clean);
                }
            }
        }
        if let Some(u) = &self.anthropic_base_url {
            if let Some(clean) = crate::config::sanitize_base_url(u) {
                if config.anthropic_base_url.as_deref() != Some(&clean) {
                    diffs.push(WorkspaceOverrideEntry {
                        field: "anthropic_base_url".to_string(),
                        old_value: config.anthropic_base_url.clone().unwrap_or_default(),
                        new_value: clean.clone(),
                    });
                    config.anthropic_base_url = Some(clean);
                }
            }
        }

        if let Some(k) = &self.deepseek_api_key {
            if let Some(clean) = crate::config::sanitize_env_var(k) {
                if config.deepseek_api_key.as_deref() != Some(&clean) {
                    diffs.push(WorkspaceOverrideEntry {
                        field: "deepseek_api_key".to_string(),
                        old_value: mask_key_preview(&config.deepseek_api_key),
                        new_value: mask_key_preview(&Some(clean.clone())),
                    });
                    config.deepseek_api_key = Some(clean);
                }
            }
        }
        if let Some(u) = &self.deepseek_base_url {
            if let Some(clean) = crate::config::sanitize_base_url(u) {
                if config.deepseek_base_url.as_deref() != Some(&clean) {
                    diffs.push(WorkspaceOverrideEntry {
                        field: "deepseek_base_url".to_string(),
                        old_value: config.deepseek_base_url.clone().unwrap_or_default(),
                        new_value: clean.clone(),
                    });
                    config.deepseek_base_url = Some(clean);
                }
            }
        }

        if let Some(k) = &self.xai_api_key {
            if let Some(clean) = crate::config::sanitize_env_var(k) {
                if config.xai_api_key.as_deref() != Some(&clean) {
                    diffs.push(WorkspaceOverrideEntry {
                        field: "xai_api_key".to_string(),
                        old_value: mask_key_preview(&config.xai_api_key),
                        new_value: mask_key_preview(&Some(clean.clone())),
                    });
                    config.xai_api_key = Some(clean);
                }
            }
        }
        if let Some(u) = &self.xai_base_url {
            if let Some(clean) = crate::config::sanitize_base_url(u) {
                if config.xai_base_url.as_deref() != Some(&clean) {
                    diffs.push(WorkspaceOverrideEntry {
                        field: "xai_base_url".to_string(),
                        old_value: config.xai_base_url.clone().unwrap_or_default(),
                        new_value: clean.clone(),
                    });
                    config.xai_base_url = Some(clean);
                }
            }
        }

        if let Some(k) = &self.openrouter_api_key {
            if let Some(clean) = crate::config::sanitize_env_var(k) {
                if config.openrouter_api_key.as_deref() != Some(&clean) {
                    diffs.push(WorkspaceOverrideEntry {
                        field: "openrouter_api_key".to_string(),
                        old_value: mask_key_preview(&config.openrouter_api_key),
                        new_value: mask_key_preview(&Some(clean.clone())),
                    });
                    config.openrouter_api_key = Some(clean);
                }
            }
        }
        if let Some(u) = &self.openrouter_base_url {
            if let Some(clean) = crate::config::sanitize_base_url(u) {
                if config.openrouter_base_url.as_deref() != Some(&clean) {
                    diffs.push(WorkspaceOverrideEntry {
                        field: "openrouter_base_url".to_string(),
                        old_value: config.openrouter_base_url.clone().unwrap_or_default(),
                        new_value: clean.clone(),
                    });
                    config.openrouter_base_url = Some(clean);
                }
            }
        }

        if let Some(u) = &self.ollama_base_url {
            if let Some(clean) = crate::config::sanitize_base_url(u) {
                if config.ollama_base_url.as_deref() != Some(&clean) {
                    diffs.push(WorkspaceOverrideEntry {
                        field: "ollama_base_url".to_string(),
                        old_value: config.ollama_base_url.clone().unwrap_or_default(),
                        new_value: clean.clone(),
                    });
                    config.ollama_base_url = Some(clean);
                }
            }
        }

        // 6. Multi-Agent & Advisors
        if let Some(adv) = self.advisors_enabled {
            if config.advisors_enabled != adv {
                diffs.push(WorkspaceOverrideEntry {
                    field: "advisors_enabled".to_string(),
                    old_value: config.advisors_enabled.to_string(),
                    new_value: adv.to_string(),
                });
                config.advisors_enabled = adv;
            }
        }
        if let Some(adv_m) = &self.advisor_model {
            let (_, resolved) =
                Config::resolve_model(adv_m, Some(&config.default_provider));
            if config.advisor_model.as_deref() != Some(&resolved) {
                diffs.push(WorkspaceOverrideEntry {
                    field: "advisor_model".to_string(),
                    old_value: config.advisor_model.clone().unwrap_or_default(),
                    new_value: resolved.clone(),
                });
                config.advisor_model = Some(resolved);
            }
        }

        // 7. Audio / Sound
        if let Some(snd) = self.sound_enabled {
            if config.sound_enabled != snd {
                diffs.push(WorkspaceOverrideEntry {
                    field: "sound_enabled".to_string(),
                    old_value: config.sound_enabled.to_string(),
                    new_value: snd.to_string(),
                });
                config.sound_enabled = snd;
            }
        }
        if let Some(bell_comp) = self.bell_on_completion {
            if config.bell_on_completion != bell_comp {
                diffs.push(WorkspaceOverrideEntry {
                    field: "bell_on_completion".to_string(),
                    old_value: config.bell_on_completion.to_string(),
                    new_value: bell_comp.to_string(),
                });
                config.bell_on_completion = bell_comp;
            }
        }
        if let Some(bell_err) = self.bell_on_error {
            if config.bell_on_error != bell_err {
                diffs.push(WorkspaceOverrideEntry {
                    field: "bell_on_error".to_string(),
                    old_value: config.bell_on_error.to_string(),
                    new_value: bell_err.to_string(),
                });
                config.bell_on_error = bell_err;
            }
        }

        // 8. Notifications
        if let Some(notif) = self.notify_enabled {
            if config.notify_enabled != notif {
                diffs.push(WorkspaceOverrideEntry {
                    field: "notify_enabled".to_string(),
                    old_value: config.notify_enabled.to_string(),
                    new_value: notif.to_string(),
                });
                config.notify_enabled = notif;
            }
        }
        if let Some(notif_comp) = self.notify_on_completion {
            if config.notify_on_completion != notif_comp {
                diffs.push(WorkspaceOverrideEntry {
                    field: "notify_on_completion".to_string(),
                    old_value: config.notify_on_completion.to_string(),
                    new_value: notif_comp.to_string(),
                });
                config.notify_on_completion = notif_comp;
            }
        }
        if let Some(notif_err) = self.notify_on_error {
            if config.notify_on_error != notif_err {
                diffs.push(WorkspaceOverrideEntry {
                    field: "notify_on_error".to_string(),
                    old_value: config.notify_on_error.to_string(),
                    new_value: notif_err.to_string(),
                });
                config.notify_on_error = notif_err;
            }
        }
        if let Some(min_d) = self.notify_min_duration_secs {
            let clamped = min_d.max(0.0);
            if config.notify_min_duration_secs != Some(clamped) {
                diffs.push(WorkspaceOverrideEntry {
                    field: "notify_min_duration_secs".to_string(),
                    old_value: format!("{:?}", config.notify_min_duration_secs),
                    new_value: format!("{:?}", Some(clamped)),
                });
                config.notify_min_duration_secs = Some(clamped);
            }
        }

        // 9. Export workspace environment variables into the process
        for (k, v) in &self.env {
            if !k.is_empty() {
                std::env::set_var(k, v);
            }
        }

        diffs
    }

    /// Serializes the workspace configuration to a clean TOML string.
    pub fn to_toml_string(&self) -> String {
        let mut out = String::new();

        out.push_str("# Fusion Workspace Configuration\n");
        if let Some(name) = &self.name {
            out.push_str(&format!("name = \"{}\"\n", escape_toml_str(name)));
        }
        if let Some(desc) = &self.description {
            out.push_str(&format!("description = \"{}\"\n", escape_toml_str(desc)));
        }
        if let Some(v) = self.version {
            out.push_str(&format!("version = {}\n", v));
        }

        // LLM & Provider
        if self.default_provider.is_some()
            || self.default_model.is_some()
            || self.default_temperature.is_some()
            || self.max_tokens.is_some()
        {
            out.push_str("\n# Model & Provider\n");
            if let Some(p) = &self.default_provider {
                out.push_str(&format!("provider = \"{}\"\n", escape_toml_str(p)));
            }
            if let Some(m) = &self.default_model {
                out.push_str(&format!("model = \"{}\"\n", escape_toml_str(m)));
            }
            if let Some(t) = self.default_temperature {
                out.push_str(&format!("temperature = {:.2}\n", t));
            }
            if let Some(tok) = self.max_tokens {
                out.push_str(&format!("max_tokens = {}\n", tok));
            }
        }

        // Advisors
        if self.advisors_enabled.is_some() || self.advisor_model.is_some() {
            out.push_str("\n[advisors]\n");
            if let Some(adv) = self.advisors_enabled {
                out.push_str(&format!("enabled = {}\n", adv));
            }
            if let Some(m) = &self.advisor_model {
                out.push_str(&format!("model = \"{}\"\n", escape_toml_str(m)));
            }
        }

        // Sound / Audio Cues
        if self.sound_enabled.is_some()
            || self.bell_on_completion.is_some()
            || self.bell_on_error.is_some()
        {
            out.push_str("\n[sound]\n");
            if let Some(s) = self.sound_enabled {
                out.push_str(&format!("enabled = {}\n", s));
            }
            if let Some(bc) = self.bell_on_completion {
                out.push_str(&format!("bell_on_completion = {}\n", bc));
            }
            if let Some(be) = self.bell_on_error {
                out.push_str(&format!("bell_on_error = {}\n", be));
            }
        }

        // Notifications
        if self.notify_enabled.is_some()
            || self.notify_on_completion.is_some()
            || self.notify_on_error.is_some()
            || self.notify_min_duration_secs.is_some()
        {
            out.push_str("\n[notifications]\n");
            if let Some(n) = self.notify_enabled {
                out.push_str(&format!("enabled = {}\n", n));
            }
            if let Some(nc) = self.notify_on_completion {
                out.push_str(&format!("notify_on_completion = {}\n", nc));
            }
            if let Some(ne) = self.notify_on_error {
                out.push_str(&format!("notify_on_error = {}\n", ne));
            }
            if let Some(min_d) = self.notify_min_duration_secs {
                out.push_str(&format!("min_duration_secs = {:.1}\n", min_d));
            }
        }

        // Instructions & Rules
        if self.instructions.is_some() || !self.custom_rules.is_empty() {
            out.push_str("\n[instructions]\n");
            if let Some(inst) = &self.instructions {
                if inst.contains('\n') {
                    out.push_str(&format!("prompt = \"\"\"\n{}\"\"\"\n", inst.trim()));
                } else {
                    out.push_str(&format!("prompt = \"{}\"\n", escape_toml_str(inst)));
                }
            }
            if !self.custom_rules.is_empty() {
                out.push_str("rules = [\n");
                for rule in &self.custom_rules {
                    out.push_str(&format!("    \"{}\",\n", escape_toml_str(rule)));
                }
                out.push_str("]\n");
            }
        }

        // Ignored patterns
        if !self.ignored_patterns.is_empty() {
            out.push_str("\nignored_patterns = [\n");
            for pat in &self.ignored_patterns {
                out.push_str(&format!("    \"{}\",\n", escape_toml_str(pat)));
            }
            out.push_str("]\n");
        }

        // Environment variables
        if !self.env.is_empty() {
            out.push_str("\n[env]\n");
            let mut sorted_keys: Vec<_> = self.env.keys().collect();
            sorted_keys.sort();
            for k in sorted_keys {
                if let Some(v) = self.env.get(k) {
                    out.push_str(&format!("{} = \"{}\"\n", k, escape_toml_str(v)));
                }
            }
        }

        // MCP Servers
        if !self.mcp_servers.is_empty() {
            let mut sorted_servers: Vec<_> = self.mcp_servers.keys().collect();
            sorted_servers.sort();
            for name in sorted_servers {
                if let Some(srv) = self.mcp_servers.get(name) {
                    out.push_str(&format!("\n[mcp_servers.{}]\n", name));
                    if let Some(cmd) = &srv.command {
                        out.push_str(&format!("command = \"{}\"\n", escape_toml_str(cmd)));
                    }
                    if !srv.args.is_empty() {
                        out.push_str("args = [");
                        let args_str: Vec<_> = srv
                            .args
                            .iter()
                            .map(|a| format!("\"{}\"", escape_toml_str(a)))
                            .collect();
                        out.push_str(&args_str.join(", "));
                        out.push_str("]\n");
                    }
                    if let Some(disabled) = srv.disabled {
                        out.push_str(&format!("disabled = {}\n", disabled));
                    }
                    if let Some(transport) = &srv.transport {
                        out.push_str(&format!("transport = \"{}\"\n", escape_toml_str(transport)));
                    }
                    if let Some(url) = &srv.url {
                        out.push_str(&format!("url = \"{}\"\n", escape_toml_str(url)));
                    }
                    if let Some(cwd) = &srv.cwd {
                        out.push_str(&format!(
                            "cwd = \"{}\"\n",
                            escape_toml_str(&cwd.display().to_string())
                        ));
                    }
                    if !srv.env.is_empty() {
                        out.push_str(&format!("[mcp_servers.{}.env]\n", name));
                        let mut env_keys: Vec<_> = srv.env.keys().collect();
                        env_keys.sort();
                        for ek in env_keys {
                            if let Some(ev) = srv.env.get(ek) {
                                out.push_str(&format!("{} = \"{}\"\n", ek, escape_toml_str(ev)));
                            }
                        }
                    }
                }
            }
        }

        // Tools
        if !self.tools.is_default() {
            out.push_str("\n[tools]\n");
            if !self.tools.enabled.is_empty() {
                out.push_str("enabled = [");
                let tools_str: Vec<_> = self
                    .tools
                    .enabled
                    .iter()
                    .map(|t| format!("\"{}\"", escape_toml_str(t)))
                    .collect();
                out.push_str(&tools_str.join(", "));
                out.push_str("]\n");
            }
            if !self.tools.disabled.is_empty() {
                out.push_str("disabled = [");
                let tools_str: Vec<_> = self
                    .tools
                    .disabled
                    .iter()
                    .map(|t| format!("\"{}\"", escape_toml_str(t)))
                    .collect();
                out.push_str(&tools_str.join(", "));
                out.push_str("]\n");
            }
            if let Some(timeout) = self.tools.bash_timeout_secs {
                out.push_str(&format!("bash_timeout_secs = {}\n", timeout));
            }
            if let Some(ar) = self.tools.auto_approve_reads {
                out.push_str(&format!("auto_approve_reads = {}\n", ar));
            }
            if let Some(aw) = self.tools.auto_approve_writes {
                out.push_str(&format!("auto_approve_writes = {}\n", aw));
            }
            if let Some(ab) = self.tools.auto_approve_bash {
                out.push_str(&format!("auto_approve_bash = {}\n", ab));
            }
        }

        out
    }

    /// Serializes the workspace configuration to formatted JSON.
    pub fn to_json_string(&self) -> Result<String, serde_json::Error> {
        serde_json::to_string_pretty(self)
    }

    /// Saves the workspace configuration to a file (detecting TOML/JSON from extension).
    pub fn save_to_file(&self, path: &Path) -> Result<(), WorkspaceConfigError> {
        if let Some(parent) = path.parent() {
            if !parent.as_os_str().is_empty() && !parent.exists() {
                fs::create_dir_all(parent).map_err(|e| WorkspaceConfigError::Io {
                    path: path.to_path_buf(),
                    source: e,
                })?;
            }
        }

        let format = WorkspaceConfigFormat::from_path(path).unwrap_or(WorkspaceConfigFormat::Toml);
        let content = match format {
            WorkspaceConfigFormat::Toml => self.to_toml_string(),
            WorkspaceConfigFormat::Json => self
                .to_json_string()
                .map_err(|e| WorkspaceConfigError::JsonParse {
                    path: path.to_path_buf(),
                    source: e,
                })?,
        };

        fs::write(path, content).map_err(|e| WorkspaceConfigError::Io {
            path: path.to_path_buf(),
            source: e,
        })
    }
}

/// Helper function to mask API keys for override diff logging.
fn mask_key_preview(key: &Option<String>) -> String {
    match key {
        Some(k) if k.len() > 8 => format!("{}...{}", &k[..4], &k[k.len() - 4..]),
        Some(_) => "(set)".to_string(),
        None => "(none)".to_string(),
    }
}

/// Escapes a string for inclusion in TOML literal values.
fn escape_toml_str(s: &str) -> String {
    s.replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', "\\n")
        .replace('\r', "\\r")
        .replace('\t', "\\t")
}

/// Discovers candidate workspace configuration paths for a directory.
pub fn workspace_config_candidates(dir: &Path) -> Vec<PathBuf> {
    let mut candidates = Vec::new();

    // 1. Explicit environment variable override
    if let Ok(env_path) = std::env::var("FUSION_WORKSPACE_CONFIG") {
        if !env_path.trim().is_empty() {
            candidates.push(PathBuf::from(env_path.trim()));
        }
    }
    if let Ok(env_path) = std::env::var("FUSION_PROJECT_CONFIG") {
        if !env_path.trim().is_empty() {
            candidates.push(PathBuf::from(env_path.trim()));
        }
    }

    // 2. Standard workspace config files in directory root and `.fusion/` subfolder
    for filename in WORKSPACE_CONFIG_FILES {
        candidates.push(dir.join(filename));
    }

    candidates
}

/// Searches for an existing workspace configuration file in `start_dir` or any parent directory.
pub fn find_workspace_config_file(start_dir: &Path, search_parents: bool) -> Option<PathBuf> {
    let mut current = if start_dir.is_file() {
        start_dir.parent()?.to_path_buf()
    } else {
        start_dir.to_path_buf()
    };

    loop {
        for candidate in workspace_config_candidates(&current) {
            if candidate.is_file() {
                return Some(candidate);
            }
        }

        if !search_parents || !current.pop() {
            break;
        }
    }

    None
}

/// Detects if a given path points to a known workspace configuration file name.
pub fn is_workspace_config_file(path: &Path) -> bool {
    if let Some(file_name) = path.file_name().and_then(|f| f.to_str()) {
        let name_lower = file_name.to_ascii_lowercase();
        if name_lower == ".fusion.toml"
            || name_lower == ".fusion.json"
            || name_lower == "fusion.toml"
            || name_lower == "fusion.json"
            || name_lower == "workspace.toml"
            || name_lower == "workspace.json"
        {
            return true;
        }
        if name_lower == "config.toml" || name_lower == "config.json" {
            if let Some(parent) = path.parent().and_then(|p| p.file_name()) {
                if parent == ".fusion" || parent == "fusion" {
                    return true;
                }
            }
        }
    }
    false
}

/// Finds the root directory of a project workspace by locating project markers
/// (`.fusion.toml`, `.fusion.json`, `.git`, `Cargo.toml`, `package.json`, etc.).
pub fn find_workspace_root(start_dir: &Path) -> Option<PathBuf> {
    let mut current = if start_dir.is_file() {
        start_dir.parent()?.to_path_buf()
    } else {
        start_dir.to_path_buf()
    };

    loop {
        // Priority 1: Direct Fusion workspace config
        for candidate in WORKSPACE_CONFIG_FILES {
            if current.join(candidate).exists() {
                return Some(current);
            }
        }

        // Priority 2: Common project root indicators
        if current.join(".git").exists()
            || current.join(".hg").exists()
            || current.join(".svn").exists()
            || current.join("Cargo.toml").exists()
            || current.join("package.json").exists()
            || current.join("pyproject.toml").exists()
            || current.join("go.mod").exists()
            || current.join("pom.xml").exists()
            || current.join("build.gradle").exists()
            || current.join("CMakeLists.txt").exists()
            || current.join(".fusion").is_dir()
        {
            return Some(current);
        }

        if !current.pop() {
            break;
        }
    }

    None
}

// ---------------------------------------------------------------------------
// Pure-Rust TOML Parser for Workspace Configuration
// ---------------------------------------------------------------------------

/// Parses a TOML string into a hierarchical `serde_json::Value` object.
fn parse_toml_to_json_value(
    content: &str,
    path_for_err: &Path,
) -> Result<Value, WorkspaceConfigError> {
    let mut root_obj = Map::new();
    let mut current_table_path: Vec<String> = Vec::new();

    let lines: Vec<&str> = content.lines().collect();
    let mut i = 0;

    while i < lines.len() {
        let line_num = i + 1;
        let line = lines[i];
        let trimmed = line.trim();

        // Skip empty lines and comment lines
        if trimmed.is_empty() || trimmed.starts_with('#') {
            i += 1;
            continue;
        }

        // Table Header: `[section]` or `[section.subsection]`
        if trimmed.starts_with('[') && trimmed.ends_with(']') && !trimmed.starts_with("[[") {
            let section_str = trimmed[1..trimmed.len() - 1].trim();
            if section_str.is_empty() {
                return Err(WorkspaceConfigError::TomlParse {
                    path: path_for_err.to_path_buf(),
                    line: line_num,
                    col: 1,
                    message: "Empty table header '[]'".to_string(),
                });
            }
            current_table_path = section_str
                .split('.')
                .map(|s| s.trim().trim_matches('"').trim_matches('\'').to_string())
                .filter(|s| !s.is_empty())
                .collect();
            i += 1;
            continue;
        }

        // Key = Value
        if let Some((raw_key, raw_val_part)) = split_toml_key_value(trimmed) {
            let key = raw_key
                .trim()
                .trim_matches('"')
                .trim_matches('\'')
                .to_string();
            if key.is_empty() {
                return Err(WorkspaceConfigError::TomlParse {
                    path: path_for_err.to_path_buf(),
                    line: line_num,
                    col: 1,
                    message: "Empty key in key-value assignment".to_string(),
                });
            }

            // Check if this is a multiline string: `"""...`
            let mut val_str = raw_val_part.to_string();
            if val_str.trim().starts_with("\"\"\"") && !val_str.trim()[3..].ends_with("\"\"\"") {
                // Collect multiline string lines
                let mut full_multiline = val_str;
                i += 1;
                let mut closed = false;
                while i < lines.len() {
                    let next_line = lines[i];
                    full_multiline.push('\n');
                    full_multiline.push_str(next_line);
                    if next_line.contains("\"\"\"") {
                        closed = true;
                        i += 1;
                        break;
                    }
                    i += 1;
                }
                if !closed {
                    return Err(WorkspaceConfigError::TomlParse {
                        path: path_for_err.to_path_buf(),
                        line: line_num,
                        col: 1,
                        message: "Unclosed multiline string literal".to_string(),
                    });
                }
                val_str = full_multiline;
            } else if val_str.trim().starts_with('[') && !val_str.contains(']') {
                // Collect multiline array lines
                let mut full_array = val_str;
                i += 1;
                let mut closed = false;
                while i < lines.len() {
                    let next_line = lines[i];
                    full_array.push(' ');
                    full_array.push_str(next_line.trim());
                    if next_line.contains(']') {
                        closed = true;
                        i += 1;
                        break;
                    }
                    i += 1;
                }
                if !closed {
                    return Err(WorkspaceConfigError::TomlParse {
                        path: path_for_err.to_path_buf(),
                        line: line_num,
                        col: 1,
                        message: "Unclosed array literal".to_string(),
                    });
                }
                val_str = full_array;
            } else {
                i += 1;
            }

            let parsed_value = parse_toml_primitive_or_composite(&val_str, line_num, path_for_err)?;
            insert_nested_value(
                &mut root_obj,
                &current_table_path,
                key,
                parsed_value,
                line_num,
                path_for_err,
            )?;
            continue;
        }

        return Err(WorkspaceConfigError::TomlParse {
            path: path_for_err.to_path_buf(),
            line: line_num,
            col: 1,
            message: format!("Unrecognized TOML syntax: '{}'", trimmed),
        });
    }

    Ok(Value::Object(root_obj))
}

/// Splits a TOML key-value line taking care of quotes.
fn split_toml_key_value(line: &str) -> Option<(&str, &str)> {
    let mut in_quote = false;
    let mut quote_char = '"';

    for (idx, ch) in line.char_indices() {
        if in_quote {
            if ch == quote_char {
                in_quote = false;
            }
        } else if ch == '"' || ch == '\'' {
            in_quote = true;
            quote_char = ch;
        } else if ch == '=' {
            let key = &line[..idx];
            let val = &line[idx + 1..];
            return Some((key, val));
        } else if ch == '#' {
            // Comment before '=' is not a valid assignment
            return None;
        }
    }
    None
}

/// Parses a TOML value token into a `serde_json::Value`.
fn parse_toml_primitive_or_composite(
    raw: &str,
    line_num: usize,
    path_for_err: &Path,
) -> Result<Value, WorkspaceConfigError> {
    let trimmed = raw.trim();

    // Strip trailing inline comments if not in string
    let clean = strip_trailing_comment(trimmed);
    let trimmed = clean.trim();

    // 1. Multiline string: `"""..."""`
    if trimmed.starts_with("\"\"\"") && trimmed.ends_with("\"\"\"") && trimmed.len() >= 6 {
        let inside = &trimmed[3..trimmed.len() - 3];
        let unescaped = unescape_toml_string(inside);
        return Ok(Value::String(unescaped));
    }

    // 2. Standard string: `"..."`
    if trimmed.starts_with('"') && trimmed.ends_with('"') && trimmed.len() >= 2 {
        let inside = &trimmed[1..trimmed.len() - 1];
        let unescaped = unescape_toml_string(inside);
        return Ok(Value::String(unescaped));
    }

    // 3. Literal string: `'...'`
    if trimmed.starts_with('\'') && trimmed.ends_with('\'') && trimmed.len() >= 2 {
        let inside = &trimmed[1..trimmed.len() - 1];
        return Ok(Value::String(inside.to_string()));
    }

    // 4. Booleans
    if trimmed.eq_ignore_ascii_case("true") {
        return Ok(Value::Bool(true));
    }
    if trimmed.eq_ignore_ascii_case("false") {
        return Ok(Value::Bool(false));
    }

    // 5. Arrays: `[...]`
    if trimmed.starts_with('[') && trimmed.ends_with(']') {
        let inside = &trimmed[1..trimmed.len() - 1].trim();
        if inside.is_empty() {
            return Ok(Value::Array(Vec::new()));
        }

        let elements = split_toml_array_elements(inside);
        let mut arr = Vec::new();
        for elem in elements {
            let parsed_elem = parse_toml_primitive_or_composite(&elem, line_num, path_for_err)?;
            arr.push(parsed_elem);
        }
        return Ok(Value::Array(arr));
    }

    // 6. Inline Tables: `{ key = "val", num = 1 }`
    if trimmed.starts_with('{') && trimmed.ends_with('}') {
        let inside = &trimmed[1..trimmed.len() - 1].trim();
        if inside.is_empty() {
            return Ok(Value::Object(Map::new()));
        }

        let mut map = Map::new();
        let pairs = split_toml_array_elements(inside);
        for pair in pairs {
            if let Some((k_part, v_part)) = split_toml_key_value(&pair) {
                let k = k_part
                    .trim()
                    .trim_matches('"')
                    .trim_matches('\'')
                    .to_string();
                let v = parse_toml_primitive_or_composite(v_part, line_num, path_for_err)?;
                map.insert(k, v);
            }
        }
        return Ok(Value::Object(map));
    }

    // 7. Numbers: Integer or Float
    if let Ok(i) = trimmed.parse::<i64>() {
        return Ok(serde_json::json!(i));
    }
    if let Ok(u) = trimmed.parse::<u64>() {
        return Ok(serde_json::json!(u));
    }
    if let Ok(f) = trimmed.parse::<f64>() {
        if let Some(num) = serde_json::Number::from_f64(f) {
            return Ok(Value::Number(num));
        }
    }

    // Fallback: unquoted identifier / bare string
    Ok(Value::String(trimmed.to_string()))
}

/// Strips trailing `# comments` from value tokens, ignoring hashes inside quotes.
fn strip_trailing_comment(s: &str) -> String {
    let mut in_quote = false;
    let mut quote_char = '"';
    let mut out = String::new();

    for ch in s.chars() {
        if in_quote {
            if ch == quote_char {
                in_quote = false;
            }
            out.push(ch);
        } else if ch == '"' || ch == '\'' {
            in_quote = true;
            quote_char = ch;
            out.push(ch);
        } else if ch == '#' {
            break;
        } else {
            out.push(ch);
        }
    }

    out
}

/// Splits array items taking string quotes and nested arrays/tables into account.
fn split_toml_array_elements(s: &str) -> Vec<String> {
    let mut items = Vec::new();
    let mut current = String::new();
    let mut in_quote = false;
    let mut quote_char = '"';
    let mut bracket_depth = 0;
    let mut brace_depth = 0;

    for ch in s.chars() {
        if in_quote {
            if ch == quote_char {
                in_quote = false;
            }
            current.push(ch);
        } else if ch == '"' || ch == '\'' {
            in_quote = true;
            quote_char = ch;
            current.push(ch);
        } else if ch == '[' {
            bracket_depth += 1;
            current.push(ch);
        } else if ch == ']' {
            if bracket_depth > 0 {
                bracket_depth -= 1;
            }
            current.push(ch);
        } else if ch == '{' {
            brace_depth += 1;
            current.push(ch);
        } else if ch == '}' {
            if brace_depth > 0 {
                brace_depth -= 1;
            }
            current.push(ch);
        } else if ch == ',' && bracket_depth == 0 && brace_depth == 0 {
            let item = current.trim().to_string();
            if !item.is_empty() {
                items.push(item);
            }
            current.clear();
        } else {
            current.push(ch);
        }
    }

    let last = current.trim().to_string();
    if !last.is_empty() {
        items.push(last);
    }

    items
}

/// Unescapes common escape sequences in TOML string literals.
fn unescape_toml_string(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();

    while let Some(ch) = chars.next() {
        if ch == '\\' {
            if let Some(&next) = chars.peek() {
                match next {
                    'n' => {
                        chars.next();
                        out.push('\n');
                    }
                    'r' => {
                        chars.next();
                        out.push('\r');
                    }
                    't' => {
                        chars.next();
                        out.push('\t');
                    }
                    '"' => {
                        chars.next();
                        out.push('"');
                    }
                    '\'' => {
                        chars.next();
                        out.push('\'');
                    }
                    '\\' => {
                        chars.next();
                        out.push('\\');
                    }
                    _ => {
                        out.push('\\');
                    }
                }
            } else {
                out.push('\\');
            }
        } else {
            out.push(ch);
        }
    }

    out
}

/// Inserts a value into a nested JSON object structure based on table path.
fn insert_nested_value(
    root: &mut Map<String, Value>,
    table_path: &[String],
    key: String,
    val: Value,
    line_num: usize,
    path_for_err: &Path,
) -> Result<(), WorkspaceConfigError> {
    let mut current_obj = root;

    for segment in table_path {
        if !current_obj.contains_key(segment) {
            current_obj.insert(segment.clone(), Value::Object(Map::new()));
        }

        let child = current_obj.get_mut(segment).unwrap();
        if let Value::Object(map) = child {
            current_obj = map;
        } else {
            return Err(WorkspaceConfigError::TomlParse {
                path: path_for_err.to_path_buf(),
                line: line_num,
                col: 1,
                message: format!(
                    "Path collision: '{}' is already defined as a non-table value",
                    segment
                ),
            });
        }
    }

    current_obj.insert(key, val);
    Ok(())
}

/// Normalizes nested sections (e.g. `[llm]`, `[provider]`, `[advisors]`, `[sound]`, `[notifications]`, `[instructions]`)
/// into the top-level `WorkspaceConfig` fields so both flat and structured TOML/JSON work seamlessly.
fn normalize_workspace_json_value(mut root: Value) -> Value {
    if let Value::Object(map) = &mut root {
        // 1. [llm] or [model] section
        if let Some(Value::Object(llm)) = map.remove("llm").or_else(|| map.remove("model_config")) {
            for (k, v) in llm {
                match k.as_str() {
                    "provider" | "default_provider" => {
                        map.entry("default_provider").or_insert(v);
                    }
                    "model" | "default_model" | "name" => {
                        map.entry("default_model").or_insert(v);
                    }
                    "temperature" | "default_temperature" | "temp" => {
                        map.entry("default_temperature").or_insert(v);
                    }
                    "max_tokens" | "tokens" => {
                        map.entry("max_tokens").or_insert(v);
                    }
                    other => {
                        map.entry(other.to_string()).or_insert(v);
                    }
                }
            }
        }

        // 2. [provider] section (e.g. `[provider]\ndefault = "anthropic"`)
        if let Some(Value::Object(prov_map)) = map.get("provider").cloned() {
            if let Some(def_prov) = prov_map.get("default").or_else(|| prov_map.get("name")) {
                map.entry("default_provider")
                    .or_insert_with(|| def_prov.clone());
            }
            if let Some(def_m) = prov_map.get("model") {
                map.entry("default_model").or_insert_with(|| def_m.clone());
            }
            map.remove("provider");
        }

        // 3. [advisors] section
        if let Some(Value::Object(adv)) = map.get("advisors").cloned() {
            if let Some(enabled) = adv.get("enabled").or_else(|| adv.get("enable")) {
                map.entry("advisors_enabled")
                    .or_insert_with(|| enabled.clone());
            }
            if let Some(model) = adv.get("model") {
                map.entry("advisor_model").or_insert_with(|| model.clone());
            }
            map.remove("advisors");
        }

        // 4. [sound] section
        if let Some(Value::Object(snd)) = map.get("sound").cloned() {
            if let Some(enabled) = snd.get("enabled") {
                map.entry("sound_enabled")
                    .or_insert_with(|| enabled.clone());
            }
            if let Some(bc) = snd.get("bell_on_completion") {
                map.entry("bell_on_completion").or_insert_with(|| bc.clone());
            }
            if let Some(be) = snd.get("bell_on_error") {
                map.entry("bell_on_error").or_insert_with(|| be.clone());
            }
            map.remove("sound");
        }

        // 5. [notifications] section
        if let Some(Value::Object(notif)) = map.get("notifications").cloned() {
            if let Some(enabled) = notif.get("enabled") {
                map.entry("notify_enabled")
                    .or_insert_with(|| enabled.clone());
            }
            if let Some(nc) = notif.get("notify_on_completion") {
                map.entry("notify_on_completion")
                    .or_insert_with(|| nc.clone());
            }
            if let Some(ne) = notif.get("notify_on_error") {
                map.entry("notify_on_error").or_insert_with(|| ne.clone());
            }
            if let Some(min_d) = notif
                .get("min_duration_secs")
                .or_else(|| notif.get("notify_min_duration_secs"))
            {
                map.entry("notify_min_duration_secs")
                    .or_insert_with(|| min_d.clone());
            }
            map.remove("notifications");
        }

        // 6. [instructions] or [prompt] section
        if let Some(Value::Object(inst)) = map.get("instructions").cloned() {
            if let Some(prompt) = inst.get("prompt").or_else(|| inst.get("system_prompt")) {
                map.entry("instructions")
                    .or_insert_with(|| prompt.clone());
            }
            if let Some(rules) = inst.get("rules") {
                map.entry("custom_rules").or_insert_with(|| rules.clone());
            }
            map.remove("instructions");
        }
    }
    root
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn test_parse_flat_toml_workspace_config() {
        let toml_str = r#"
# Workspace settings
name = "fusion-workspace"
description = "Test workspace"
provider = "anthropic"
model = "claude-3-7-sonnet"
temperature = 0.4
max_tokens = 4096
advisors_enabled = false
sound_enabled = true
notify_enabled = false

ignored_patterns = ["target/", "*.log", "dist/"]
"#;

        let ws = WorkspaceConfig::from_toml_str(toml_str).unwrap();
        assert_eq!(ws.name.as_deref(), Some("fusion-workspace"));
        assert_eq!(ws.description.as_deref(), Some("Test workspace"));
        assert_eq!(ws.default_provider.as_deref(), Some("anthropic"));
        assert_eq!(ws.default_model.as_deref(), Some("claude-3-7-sonnet"));
        assert_eq!(ws.default_temperature, Some(0.4));
        assert_eq!(ws.max_tokens, Some(4096));
        assert_eq!(ws.advisors_enabled, Some(false));
        assert_eq!(ws.sound_enabled, Some(true));
        assert_eq!(ws.notify_enabled, Some(false));
        assert_eq!(ws.ignored_patterns, vec!["target/", "*.log", "dist/"]);
    }

    #[test]
    fn test_parse_nested_sections_toml() {
        let toml_str = r#"
name = "deep-backend"

[llm]
provider = "deepseek"
model = "deepseek-reasoner"
temperature = 0.0
max_tokens = 16384

[advisors]
enabled = true
model = "claude-3-5-sonnet"

[sound]
enabled = true
bell_on_completion = true
bell_on_error = false

[notifications]
enabled = true
min_duration_secs = 3.5

[instructions]
prompt = """
Always adhere to strict Rust idioms.
No unwrap in production code.
"""
rules = ["Pure Rust dependencies", "Zero unsafe"]

[env]
RUST_LOG = "debug"
FUSION_TEST_MODE = "1"
"#;

        let ws = WorkspaceConfig::from_toml_str(toml_str).unwrap();
        assert_eq!(ws.name.as_deref(), Some("deep-backend"));
        assert_eq!(ws.default_provider.as_deref(), Some("deepseek"));
        assert_eq!(ws.default_model.as_deref(), Some("deepseek-reasoner"));
        assert_eq!(ws.default_temperature, Some(0.0));
        assert_eq!(ws.max_tokens, Some(16384));
        assert_eq!(ws.advisors_enabled, Some(true));
        assert_eq!(ws.advisor_model.as_deref(), Some("claude-3-5-sonnet"));
        assert_eq!(ws.sound_enabled, Some(true));
        assert_eq!(ws.bell_on_completion, Some(true));
        assert_eq!(ws.bell_on_error, Some(false));
        assert_eq!(ws.notify_enabled, Some(true));
        assert_eq!(ws.notify_min_duration_secs, Some(3.5));
        assert!(ws
            .instructions
            .as_ref()
            .unwrap()
            .contains("No unwrap in production code"));
        assert_eq!(
            ws.custom_rules,
            vec!["Pure Rust dependencies", "Zero unsafe"]
        );
        assert_eq!(ws.env.get("RUST_LOG").unwrap(), "debug");
        assert_eq!(ws.env.get("FUSION_TEST_MODE").unwrap(), "1");
    }

    #[test]
    fn test_parse_json_workspace_config() {
        let json_str = r#"{
            "name": "json-workspace",
            "provider": "openai",
            "model": "gpt-4o",
            "temperature": 0.7,
            "max_tokens": 8192,
            "advisors_enabled": false,
            "sound_enabled": true,
            "notify_enabled": true,
            "env": {
                "NODE_ENV": "development"
            }
        }"#;

        let ws = WorkspaceConfig::from_json_str(json_str).unwrap();
        assert_eq!(ws.name.as_deref(), Some("json-workspace"));
        assert_eq!(ws.default_provider.as_deref(), Some("openai"));
        assert_eq!(ws.default_model.as_deref(), Some("gpt-4o"));
        assert_eq!(ws.default_temperature, Some(0.7));
        assert_eq!(ws.max_tokens, Some(8192));
        assert_eq!(ws.advisors_enabled, Some(false));
        assert_eq!(ws.sound_enabled, Some(true));
        assert_eq!(ws.notify_enabled, Some(true));
        assert_eq!(ws.env.get("NODE_ENV").unwrap(), "development");
    }

    #[test]
    fn test_apply_workspace_overrides_to_config() {
        let mut base_config = Config::default();
        base_config.default_provider = "deepseek".to_string();
        base_config.default_model = "deepseek-chat".to_string();
        base_config.default_temperature = Some(0.2);
        base_config.max_tokens = Some(8192);
        base_config.advisors_enabled = true;
        base_config.sound_enabled = false;

        let ws = WorkspaceConfig {
            default_provider: Some("anthropic".to_string()),
            default_model: Some("claude-3-7-sonnet".to_string()),
            default_temperature: Some(0.5),
            max_tokens: Some(4096),
            advisors_enabled: Some(false),
            sound_enabled: Some(true),
            env: {
                let mut map = HashMap::new();
                map.insert("WORKSPACE_ENV_TEST".to_string(), "active".to_string());
                map
            },
            ..Default::default()
        };

        let diffs = ws.apply_to(&mut base_config);

        assert_eq!(base_config.default_provider, "anthropic");
        assert_eq!(base_config.default_model, "claude-3-7-sonnet-20250219");
        assert_eq!(base_config.default_temperature, Some(0.5));
        assert_eq!(base_config.max_tokens, Some(4096));
        assert_eq!(base_config.advisors_enabled, false);
        assert_eq!(base_config.sound_enabled, true);
        assert_eq!(std::env::var("WORKSPACE_ENV_TEST").unwrap(), "active");

        assert!(diffs.iter().any(|d| d.field == "default_provider"));
        assert!(diffs.iter().any(|d| d.field == "default_model"));
        assert!(diffs.iter().any(|d| d.field == "default_temperature"));
        assert!(diffs.iter().any(|d| d.field == "max_tokens"));
        assert!(diffs.iter().any(|d| d.field == "advisors_enabled"));
        assert!(diffs.iter().any(|d| d.field == "sound_enabled"));
    }

    #[test]
    fn test_partial_override_preserves_unset_fields() {
        let mut base_config = Config::default();
        base_config.default_provider = "anthropic".to_string();
        base_config.default_model = "claude-3-5-sonnet".to_string();
        base_config.default_temperature = Some(0.3);
        base_config.max_tokens = Some(8192);
        base_config.advisors_enabled = true;
        base_config.sound_enabled = false;

        // Workspace config only overrides temperature
        let ws = WorkspaceConfig {
            default_temperature: Some(0.9),
            ..Default::default()
        };

        ws.apply_to(&mut base_config);

        assert_eq!(base_config.default_provider, "anthropic");
        assert_eq!(base_config.default_model, "claude-3-5-sonnet");
        assert_eq!(base_config.default_temperature, Some(0.9));
        assert_eq!(base_config.max_tokens, Some(8192));
        assert_eq!(base_config.advisors_enabled, true);
        assert_eq!(base_config.sound_enabled, false);
    }

    #[test]
    fn test_workspace_file_discovery_and_save() {
        let dir = tempdir().unwrap();
        let ws_path = dir.path().join(".fusion.toml");

        let ws = WorkspaceConfig {
            name: Some("saved-workspace".to_string()),
            default_provider: Some("deepseek".to_string()),
            default_model: Some("deepseek-reasoner".to_string()),
            default_temperature: Some(0.0),
            max_tokens: Some(16384),
            advisors_enabled: Some(true),
            sound_enabled: Some(false),
            custom_rules: vec!["Rule A".to_string(), "Rule B".to_string()],
            ..Default::default()
        };

        ws.save_to_file(&ws_path).unwrap();
        assert!(ws_path.exists());

        // Discover and load
        let discovered = WorkspaceConfig::find_and_load(dir.path()).unwrap();
        assert!(discovered.is_some());
        let loaded = discovered.unwrap();
        assert_eq!(loaded.source_path, ws_path);
        assert_eq!(loaded.format, WorkspaceConfigFormat::Toml);
        assert_eq!(loaded.config.name.as_deref(), Some("saved-workspace"));
        assert_eq!(loaded.config.default_provider.as_deref(), Some("deepseek"));
        assert_eq!(loaded.config.default_model.as_deref(), Some("deepseek-reasoner"));
        assert_eq!(loaded.config.custom_rules, vec!["Rule A", "Rule B"]);
        assert!(loaded.overridden_fields.contains(&"default_provider".to_string()));
    }

    #[test]
    fn test_find_workspace_root_detection() {
        let dir = tempdir().unwrap();
        let sub_dir = dir.path().join("src").join("deep").join("nested");
        fs::create_dir_all(&sub_dir).unwrap();

        // Create Cargo.toml in root
        fs::write(dir.path().join("Cargo.toml"), "[package]\nname = \"foo\"\n").unwrap();

        let root = find_workspace_root(&sub_dir).unwrap();
        assert_eq!(root, dir.path());
    }

    #[test]
    fn test_roundtrip_toml_serialization() {
        let ws = WorkspaceConfig {
            name: Some("roundtrip-ws".to_string()),
            description: Some("Roundtrip test".to_string()),
            default_provider: Some("anthropic".to_string()),
            default_model: Some("claude-3-7-sonnet".to_string()),
            default_temperature: Some(0.35),
            max_tokens: Some(8192),
            advisors_enabled: Some(true),
            advisor_model: Some("deepseek-reasoner".to_string()),
            sound_enabled: Some(true),
            bell_on_completion: Some(true),
            bell_on_error: Some(false),
            notify_enabled: Some(true),
            notify_min_duration_secs: Some(4.0),
            instructions: Some("Custom instructions for workspace".to_string()),
            custom_rules: vec!["Strict checks".to_string()],
            ignored_patterns: vec!["target/".to_string()],
            env: {
                let mut map = HashMap::new();
                map.insert("FOO".to_string(), "BAR".to_string());
                map
            },
            ..Default::default()
        };

        let toml_str = ws.to_toml_string();
        let parsed = WorkspaceConfig::from_toml_str(&toml_str).unwrap();

        assert_eq!(parsed.name, ws.name);
        assert_eq!(parsed.description, ws.description);
        assert_eq!(parsed.default_provider, ws.default_provider);
        assert_eq!(parsed.default_model, ws.default_model);
        assert_eq!(parsed.default_temperature, ws.default_temperature);
        assert_eq!(parsed.max_tokens, ws.max_tokens);
        assert_eq!(parsed.advisors_enabled, ws.advisors_enabled);
        assert_eq!(parsed.advisor_model, ws.advisor_model);
        assert_eq!(parsed.sound_enabled, ws.sound_enabled);
        assert_eq!(parsed.bell_on_completion, ws.bell_on_completion);
        assert_eq!(parsed.bell_on_error, ws.bell_on_error);
        assert_eq!(parsed.notify_enabled, ws.notify_enabled);
        assert_eq!(parsed.instructions, ws.instructions);
        assert_eq!(parsed.custom_rules, ws.custom_rules);
        assert_eq!(parsed.ignored_patterns, ws.ignored_patterns);
        assert_eq!(parsed.env.get("FOO").unwrap(), "BAR");
    }
}
