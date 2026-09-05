use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

pub mod migration;
pub use migration::{
    detect_version, migrate_file, migrate_file_if_needed, migrate_str, preview_migration,
    restore_backup, MigrationError, MigrationOutcome, CURRENT_SCHEMA_VERSION,
    LEGACY_SCHEMA_VERSION, V1_SCHEMA_VERSION,
};

pub mod presets;
pub use presets::{
    available_presets_list, format_presets_table, ConfigPreset, PresetError, PresetInfo,
};

pub mod env_loader;
pub use env_loader::{
    expand_variables, find_global_config_file, find_project_config_file, global_config_paths,
    is_secret_key, is_secret_value, load_dotenv, load_dotenv_from, load_dotenv_with_mode,
    load_hierarchy, load_hierarchy_from, mask_api_key, mask_secret_value, parse_json_config_str,
    project_config_paths, sanitize_text_secrets, EnvError, EnvLoader, EnvSource, EnvVariable,
    HierarchyTier, LoadedEnv, MaskStyle,
};
pub mod workspace;
pub use workspace::{
    find_workspace_config_file, find_workspace_root, is_workspace_config_file,
    workspace_config_candidates, LoadedWorkspaceConfig, WorkspaceConfig, WorkspaceConfigError,
    WorkspaceConfigFormat, WorkspaceMcpServerConfig, WorkspaceOverrideEntry, WorkspaceToolSettings,
};
/// Supported LLM providers in Fusion.
pub const SUPPORTED_PROVIDERS: &[&str] = &[
    "fusion",
    "deepseek",
    "anthropic",
    "openai",
    "xai",
    "openrouter",
    "ollama",
];

pub const MODEL_SHORTHANDS: &[(&str, &str, &str)] = &[
    // Fusion Gateway models
    ("minimax", "fusion", "MiniMaxAI/MiniMax-M2.7"),
    ("minimax-m2.7", "fusion", "MiniMaxAI/MiniMax-M2.7"),
    ("minimax-m2", "fusion", "MiniMaxAI/MiniMax-M2.7"),
    (
        "deepseek-v4",
        "fusion",
        "deepseek-ai/DeepSeek-V4-Flash-0731",
    ),
    ("flash", "fusion", "deepseek-ai/DeepSeek-V4-Flash-0731"),
    ("v4", "fusion", "deepseek-ai/DeepSeek-V4-Flash-0731"),
    ("fusion", "fusion", "deepseek-ai/DeepSeek-V4-Flash-0731"),
    (
        "fusion-default",
        "fusion",
        "deepseek-ai/DeepSeek-V4-Flash-0731",
    ),
    ("default", "fusion", "deepseek-ai/DeepSeek-V4-Flash-0731"),
    // DeepSeek
    ("deepseek", "deepseek", "deepseek-chat"),
    ("deepseek-chat", "deepseek", "deepseek-chat"),
    ("deepseek-v3", "deepseek", "deepseek-chat"),
    ("chat", "deepseek", "deepseek-chat"),
    ("v3", "deepseek", "deepseek-chat"),
    ("deepseek-reasoner", "deepseek", "deepseek-reasoner"),
    ("deepseek-r1", "deepseek", "deepseek-reasoner"),
    ("r1", "deepseek", "deepseek-reasoner"),
    ("reasoner", "deepseek", "deepseek-reasoner"),
    ("deepseek-coder", "deepseek", "deepseek-coder"),
    ("coder", "deepseek", "deepseek-coder"),
    // Anthropic / Claude
    (
        "claude-3-7-sonnet-latest",
        "anthropic",
        "claude-3-7-sonnet-20250219",
    ),
    (
        "claude-3-7-sonnet-20250219",
        "anthropic",
        "claude-3-7-sonnet-20250219",
    ),
    (
        "claude-3-7-sonnet",
        "anthropic",
        "claude-3-7-sonnet-20250219",
    ),
    (
        "claude-3.7-sonnet",
        "anthropic",
        "claude-3-7-sonnet-20250219",
    ),
    ("claude-3-7", "anthropic", "claude-3-7-sonnet-20250219"),
    ("claude-3.7", "anthropic", "claude-3-7-sonnet-20250219"),
    ("3.7-sonnet", "anthropic", "claude-3-7-sonnet-20250219"),
    ("sonnet-3.7", "anthropic", "claude-3-7-sonnet-20250219"),
    (
        "claude-3-5-sonnet-latest",
        "anthropic",
        "claude-3-5-sonnet-20241022",
    ),
    (
        "claude-3-5-sonnet-20241022",
        "anthropic",
        "claude-3-5-sonnet-20241022",
    ),
    (
        "claude-3-5-sonnet",
        "anthropic",
        "claude-3-5-sonnet-20241022",
    ),
    (
        "claude-3.5-sonnet",
        "anthropic",
        "claude-3-5-sonnet-20241022",
    ),
    ("sonnet-3.5", "anthropic", "claude-3-5-sonnet-20241022"),
    ("sonnet-3-5", "anthropic", "claude-3-5-sonnet-20241022"),
    ("claude-sonnet", "anthropic", "claude-3-5-sonnet-20241022"),
    ("sonnet", "anthropic", "claude-3-5-sonnet-20241022"),
    ("claude", "anthropic", "claude-3-5-sonnet-20241022"),
    (
        "claude-3-5-haiku-latest",
        "anthropic",
        "claude-3-5-haiku-20241022",
    ),
    (
        "claude-3-5-haiku-20241022",
        "anthropic",
        "claude-3-5-haiku-20241022",
    ),
    ("claude-3-5-haiku", "anthropic", "claude-3-5-haiku-20241022"),
    ("claude-3.5-haiku", "anthropic", "claude-3-5-haiku-20241022"),
    ("haiku-3.5", "anthropic", "claude-3-5-haiku-20241022"),
    ("haiku-3-5", "anthropic", "claude-3-5-haiku-20241022"),
    ("claude-haiku", "anthropic", "claude-3-5-haiku-20241022"),
    ("haiku", "anthropic", "claude-3-5-haiku-20241022"),
    (
        "claude-3-opus-latest",
        "anthropic",
        "claude-3-opus-20240229",
    ),
    (
        "claude-3-opus-20240229",
        "anthropic",
        "claude-3-opus-20240229",
    ),
    ("claude-3-opus", "anthropic", "claude-3-opus-20240229"),
    ("claude-3.0-opus", "anthropic", "claude-3-opus-20240229"),
    ("claude-opus", "anthropic", "claude-3-opus-20240229"),
    ("opus", "anthropic", "claude-3-opus-20240229"),
    // OpenAI
    ("gpt-4o", "openai", "gpt-4o"),
    ("4o", "openai", "gpt-4o"),
    ("gpt-4o-mini", "openai", "gpt-4o-mini"),
    ("4o-mini", "openai", "gpt-4o-mini"),
    ("o1", "openai", "o1"),
    ("o1-preview", "openai", "o1-preview"),
    ("o1-mini", "openai", "o1-mini"),
    ("o3-mini", "openai", "o3-mini"),
    ("o3", "openai", "o3-mini"),
    ("o3-mini-high", "openai", "o3-mini"),
    ("gpt-4-turbo", "openai", "gpt-4-turbo"),
    ("gpt-4-turbo-preview", "openai", "gpt-4-turbo"),
    ("4-turbo", "openai", "gpt-4-turbo"),
    ("gpt-4", "openai", "gpt-4"),
    ("gpt-3.5-turbo", "openai", "gpt-3.5-turbo"),
    ("3.5-turbo", "openai", "gpt-3.5-turbo"),
    ("gpt-3.5", "openai", "gpt-3.5-turbo"),
    // xAI / Grok
    ("grok-2-latest", "xai", "grok-2-latest"),
    ("grok-2", "xai", "grok-2-latest"),
    ("grok-2-1212", "xai", "grok-2-1212"),
    ("grok", "xai", "grok-2-latest"),
    ("grok-beta", "xai", "grok-beta"),
    ("grok-2-vision-1212", "xai", "grok-2-vision-1212"),
    ("grok-2-vision-latest", "xai", "grok-2-vision-1212"),
    ("grok-2-vision", "xai", "grok-2-vision-1212"),
    ("grok-vision", "xai", "grok-2-vision-1212"),
    // Ollama / Local models
    ("llama3.3", "ollama", "llama3.3"),
    ("llama3.2", "ollama", "llama3.2"),
    ("llama3.1", "ollama", "llama3.1"),
    ("llama3", "ollama", "llama3"),
    ("mistral", "ollama", "mistral"),
    ("mixtral", "ollama", "mixtral"),
    ("codellama", "ollama", "codellama"),
    ("qwen2.5", "ollama", "qwen2.5"),
    ("qwen2.5-coder", "ollama", "qwen2.5-coder"),
    ("qwen-coder", "ollama", "qwen2.5-coder"),
    ("qwen", "ollama", "qwen2.5-coder"),
];

/// Configuration errors with actionable resolution hints.
#[derive(Debug, thiserror::Error, Clone, PartialEq)]
pub enum ConfigError {
    #[error("Missing API key for provider '{provider}'.\nHint: {hint}")]
    MissingApiKey { provider: String, hint: String },

    #[error("Invalid base URL for provider '{provider}': '{url}'.\nHint: {hint}")]
    InvalidBaseUrl {
        provider: String,
        url: String,
        hint: String,
    },

    #[error("Unknown provider '{provider}'.\nHint: Available providers are: {}", SUPPORTED_PROVIDERS.join(", "))]
    UnknownProvider { provider: String },

    #[error("Invalid configuration for '{field}': {message}\nHint: {hint}")]
    InvalidValue {
        field: String,
        message: String,
        hint: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    #[serde(default = "default_config_version")]
    pub version: u32,

    #[serde(
        default = "default_provider_name",
        alias = "provider",
        alias = "model_provider"
    )]
    pub default_provider: String,

    #[serde(
        default = "default_model_name",
        alias = "model",
        alias = "selected_model"
    )]
    pub default_model: String,

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

    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        alias = "turns",
        alias = "max_turn"
    )]
    pub max_turns: Option<usize>,

    // Provider API keys & custom base URLs
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
        alias = "grok_api_key"
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

    // Fusion API (primary provider)
    #[serde(default, skip_serializing_if = "Option::is_none", alias = "fusion_key")]
    pub fusion_api_key: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none", alias = "fusion_url")]
    pub fusion_base_url: Option<String>,

    // Multi-Agent & Advisor configurations
    #[serde(
        default = "default_false",
        alias = "advisors",
        alias = "enable_advisors"
    )]
    pub advisors_enabled: bool,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub advisor_model: Option<String>,

    // Terminal Audio / Sound cues configuration
    #[serde(
        default = "default_false",
        alias = "sound",
        alias = "audio",
        alias = "audio_cues",
        alias = "bell"
    )]
    pub sound_enabled: bool,

    #[serde(
        default = "default_true",
        alias = "bell_completion",
        alias = "sound_on_completion"
    )]
    pub bell_on_completion: bool,

    #[serde(
        default = "default_true",
        alias = "bell_error",
        alias = "sound_on_error"
    )]
    pub bell_on_error: bool,

    // Desktop & Terminal Notification configuration
    #[serde(
        default = "default_true",
        alias = "notify",
        alias = "notification_enabled",
        alias = "notifications"
    )]
    pub notify_enabled: bool,

    #[serde(
        default = "default_true",
        alias = "notify_completion",
        alias = "notify_on_completion"
    )]
    pub notify_on_completion: bool,

    #[serde(
        default = "default_true",
        alias = "notify_error",
        alias = "notify_on_error"
    )]
    pub notify_on_error: bool,

    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        alias = "notify_min_duration"
    )]
    pub notify_min_duration_secs: Option<f64>,
}

fn default_config_version() -> u32 {
    CURRENT_SCHEMA_VERSION
}

fn default_provider_name() -> String {
    "fusion".to_string()
}

fn default_model_name() -> String {
    "deepseek-ai/DeepSeek-V4-Flash-0731".to_string()
}

fn default_true() -> bool {
    true
}

fn default_false() -> bool {
    false
}

impl Default for Config {
    fn default() -> Self {
        Self {
            version: CURRENT_SCHEMA_VERSION,
            default_provider: default_provider_name(),
            default_model: default_model_name(),
            default_temperature: Some(0.2),
            max_tokens: Some(8192),
            max_turns: Some(100),

            openai_api_key: None,
            openai_base_url: None,

            anthropic_api_key: None,
            anthropic_base_url: None,

            deepseek_api_key: None,
            deepseek_base_url: None,

            xai_api_key: None,
            xai_base_url: None,

            openrouter_api_key: None,
            openrouter_base_url: None,

            ollama_base_url: Some("http://localhost:11434".to_string()),

            fusion_api_key: None,
            fusion_base_url: Some("https://api.fusioncode.app/v1".to_string()),

            advisors_enabled: false,
            advisor_model: None,

            sound_enabled: false,
            bell_on_completion: true,
            bell_on_error: true,

            notify_enabled: true,
            notify_on_completion: true,
            notify_on_error: true,
            notify_min_duration_secs: None,
        }
    }
}

/// Sanitizes an environment variable value by trimming whitespace,
/// removing optional surrounding quotes, and rejecting empty strings or common placeholder templates.
pub fn sanitize_env_var(val: &str) -> Option<String> {
    let trimmed = val.trim();
    let unquoted = if (trimmed.starts_with('"') && trimmed.ends_with('"') && trimmed.len() >= 2)
        || (trimmed.starts_with('\'') && trimmed.ends_with('\'') && trimmed.len() >= 2)
    {
        &trimmed[1..trimmed.len() - 1]
    } else {
        trimmed
    };
    let clean = unquoted.trim();
    if clean.is_empty() {
        return None;
    }

    let lower = clean.to_lowercase();
    if lower == "your-api-key-here"
        || lower == "your_api_key_here"
        || lower == "your_api_key"
        || lower == "your-api-key"
        || lower == "your-key-here"
        || lower == "your_key_here"
        || lower == "sk-..."
        || lower == "sk-xxx"
        || lower == "xxx"
        || lower == "none"
        || lower == "null"
        || lower == "todo"
        || lower == "placeholder"
        || lower == "change_me"
        || lower == "changeme"
        || (lower.starts_with('<') && lower.ends_with('>'))
    {
        return None;
    }

    Some(clean.to_string())
}

/// Sanitizes and normalizes a base URL.
pub fn sanitize_base_url(val: &str) -> Option<String> {
    let raw = sanitize_env_var(val)?;
    let trimmed = raw.trim_end_matches('/');
    if trimmed.starts_with("http://") || trimmed.starts_with("https://") {
        Some(trimmed.to_string())
    } else if trimmed.starts_with("localhost:") || trimmed.starts_with("127.0.0.1:") {
        Some(format!("http://{}", trimmed))
    } else {
        Some(trimmed.to_string())
    }
}

/// Returns the first non-empty, sanitized value from a list of environment variable names.
fn first_non_empty_env(var_names: &[&str]) -> Option<String> {
    for name in var_names {
        if let Ok(val) = std::env::var(name) {
            if let Some(clean) = sanitize_env_var(&val) {
                return Some(clean);
            }
        }
    }
    None
}

/// Returns the first non-empty, sanitized URL from a list of environment variable names.
fn first_non_empty_url_env(var_names: &[&str]) -> Option<String> {
    for name in var_names {
        if let Ok(val) = std::env::var(name) {
            if let Some(clean) = sanitize_base_url(&val) {
                return Some(clean);
            }
        }
    }
    None
}

impl Config {
    /// Returns the global configuration directory (`~/.fusion`).
    pub fn config_dir() -> PathBuf {
        dirs::home_dir()
            .map(|h| h.join(".fusion"))
            .unwrap_or_else(|| PathBuf::from(".fusion"))
    }

    /// Returns the full path to `~/.fusion/config.json`.
    pub fn config_path() -> PathBuf {
        Self::config_dir().join("config.json")
    }

    /// Load configuration with multi-stage precedence:
    /// 1. Built-in defaults
    /// 2. `~/.fusion/config.json` (if present)
    /// 3. Workspace configuration (`.fusion.toml` / `.fusion.json` in current directory or parents)
    /// 4. Environment variables (with fallback aliases and validation)
    pub fn load() -> Self {
        let current_dir = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
        Self::load_for_workspace(&current_dir)
    }

    /// Load configuration for a specific workspace directory with multi-stage precedence:
    /// 1. Built-in defaults
    /// 2. `~/.fusion/config.json` (if present)
    /// 3. Workspace configuration (`.fusion.toml` / `.fusion.json` in `workspace_dir` or its parents)
    /// 4. Environment variables (with fallback aliases and validation)
    pub fn load_for_workspace(workspace_dir: &Path) -> Self {
        let mut cfg = Self::default();
        let path = Self::config_path();

        if path.exists() {
            match migration::migrate_file_if_needed(&path) {
                Ok(loaded) => cfg = loaded,
                Err(e) => {
                    tracing::warn!(
                        "Failed to auto-migrate config from {}: {}. Falling back to standard load.",
                        path.display(),
                        e
                    );
                    if let Ok(content) = std::fs::read_to_string(&path) {
                        if let Ok(loaded) = serde_json::from_str::<Config>(&content) {
                            cfg = loaded;
                        }
                    }
                }
            }
        }

        // Apply workspace-level overrides (.fusion.toml / .fusion.json) if discovered
        if let Ok(Some(loaded_ws)) = WorkspaceConfig::find_and_load(workspace_dir) {
            tracing::debug!("{}", loaded_ws.summary());
            loaded_ws.config.apply_to(&mut cfg);
        }

        Self::apply_env_overrides(&mut cfg);
        cfg
    }

    /// Applies environment variable overrides with fallback priority chains in-place.
    pub fn apply_env_overrides(cfg: &mut Self) {
        // Apply environment variable overrides with fallback priority chains

        // DeepSeek
        if let Some(k) = first_non_empty_env(&[
            "DEEPSEEK_API_KEY",
            "DEEPSEEK_KEY",
            "DEEP_SEEK_API_KEY",
            "FUSION_DEEPSEEK_API_KEY",
        ]) {
            cfg.deepseek_api_key = Some(k);
        }
        if let Some(u) = first_non_empty_url_env(&[
            "DEEPSEEK_BASE_URL",
            "DEEPSEEK_API_BASE",
            "FUSION_DEEPSEEK_BASE_URL",
        ]) {
            cfg.deepseek_base_url = Some(u);
        }

        // OpenAI
        if let Some(k) = first_non_empty_env(&[
            "OPENAI_API_KEY",
            "OPENAI_KEY",
            "OPEN_AI_API_KEY",
            "FUSION_OPENAI_API_KEY",
        ]) {
            cfg.openai_api_key = Some(k);
        }
        if let Some(u) = first_non_empty_url_env(&[
            "OPENAI_BASE_URL",
            "OPENAI_API_BASE",
            "FUSION_OPENAI_BASE_URL",
        ]) {
            cfg.openai_base_url = Some(u);
        }

        // Anthropic
        if let Some(k) = first_non_empty_env(&[
            "ANTHROPIC_API_KEY",
            "ANTHROPIC_KEY",
            "CLAUDE_API_KEY",
            "FUSION_ANTHROPIC_API_KEY",
        ]) {
            cfg.anthropic_api_key = Some(k);
        }
        if let Some(u) = first_non_empty_url_env(&[
            "ANTHROPIC_BASE_URL",
            "ANTHROPIC_API_BASE",
            "FUSION_ANTHROPIC_BASE_URL",
        ]) {
            cfg.anthropic_base_url = Some(u);
        }

        // xAI / Grok
        if let Some(k) = first_non_empty_env(&[
            "XAI_API_KEY",
            "XAI_KEY",
            "GROK_API_KEY",
            "FUSION_XAI_API_KEY",
        ]) {
            cfg.xai_api_key = Some(k);
        }
        if let Some(u) = first_non_empty_url_env(&[
            "XAI_BASE_URL",
            "XAI_API_BASE",
            "GROK_BASE_URL",
            "FUSION_XAI_BASE_URL",
        ]) {
            cfg.xai_base_url = Some(u);
        }

        // OpenRouter
        if let Some(k) = first_non_empty_env(&[
            "OPENROUTER_API_KEY",
            "OPENROUTER_KEY",
            "OPEN_ROUTER_API_KEY",
            "OPEN_ROUTER_KEY",
            "FUSION_OPENROUTER_API_KEY",
        ]) {
            cfg.openrouter_api_key = Some(k);
        }
        if let Some(u) = first_non_empty_url_env(&[
            "OPENROUTER_BASE_URL",
            "OPENROUTER_API_BASE",
            "FUSION_OPENROUTER_BASE_URL",
        ]) {
            cfg.openrouter_base_url = Some(u);
        }

        // Ollama
        if let Some(u) = first_non_empty_url_env(&[
            "OLLAMA_BASE_URL",
            "OLLAMA_HOST",
            "OLLAMA_URL",
            "OLLAMA_API_BASE",
            "FUSION_OLLAMA_BASE_URL",
        ]) {
            cfg.ollama_base_url = Some(u);
        }

        // Generic API Key fallback if configured
        if let Some(generic_k) = first_non_empty_env(&["FUSION_API_KEY", "LLM_API_KEY"]) {
            if cfg.deepseek_api_key.is_none() && cfg.default_provider == "deepseek" {
                cfg.deepseek_api_key = Some(generic_k.clone());
            }
            if cfg.openai_api_key.is_none() && cfg.default_provider == "openai" {
                cfg.openai_api_key = Some(generic_k.clone());
            }
            if cfg.anthropic_api_key.is_none() && cfg.default_provider == "anthropic" {
                cfg.anthropic_api_key = Some(generic_k.clone());
            }
            if cfg.xai_api_key.is_none()
                && (cfg.default_provider == "xai" || cfg.default_provider == "grok")
            {
                cfg.xai_api_key = Some(generic_k.clone());
            }
            if cfg.openrouter_api_key.is_none() && cfg.default_provider == "openrouter" {
                cfg.openrouter_api_key = Some(generic_k);
            }
        }

        // Model & Provider overrides from environment
        if let Some(p) = first_non_empty_env(&["FUSION_PROVIDER", "DEFAULT_PROVIDER"]) {
            cfg.default_provider = p.to_lowercase();
        }

        if let Some(m) = first_non_empty_env(&["FUSION_MODEL", "DEFAULT_MODEL"]) {
            let (provider, model) = Self::resolve_model(&m, Some(&cfg.default_provider));
            cfg.default_model = model;
            if first_non_empty_env(&["FUSION_PROVIDER", "DEFAULT_PROVIDER"]).is_none() {
                cfg.default_provider = provider;
            }
        } else {
            // Resolve default model shorthand if needed
            let (_, resolved) =
                Self::resolve_model(&cfg.default_model, Some(&cfg.default_provider));
            cfg.default_model = resolved;
        }

        // Temperature & Token overrides
        if let Some(t_str) = first_non_empty_env(&["FUSION_TEMPERATURE", "TEMPERATURE"]) {
            if let Ok(t) = t_str.parse::<f32>() {
                if (0.0..=2.0).contains(&t) {
                    cfg.default_temperature = Some(t);
                }
            }
        }

        if let Some(tok_str) = first_non_empty_env(&["FUSION_MAX_TOKENS", "MAX_TOKENS"]) {
            if let Ok(tok) = tok_str.parse::<u32>() {
                if tok > 0 {
                    cfg.max_tokens = Some(tok);
                }
            }
        }

        // Max turns overrides (clamped min 10, max 500)
        if let Some(turns_str) = first_non_empty_env(&["FUSION_MAX_TURNS", "MAX_TURNS"]) {
            if let Ok(turns) = turns_str.parse::<usize>() {
                cfg.max_turns = Some(turns.clamp(10, 500));
            }
        } else if let Some(turns) = cfg.max_turns {
            cfg.max_turns = Some(turns.clamp(10, 500));
        }

        // Advisor settings
        if let Some(no_adv) = first_non_empty_env(&["FUSION_NO_ADVISORS"]) {
            let lower = no_adv.to_lowercase();
            if lower == "1" || lower == "true" || lower == "yes" {
                cfg.advisors_enabled = false;
            }
        }

        if let Some(adv_model) = first_non_empty_env(&["FUSION_ADVISOR_MODEL", "ADVISOR_MODEL"]) {
            let (_, resolved) = Self::resolve_model(&adv_model, Some(&cfg.default_provider));
            cfg.advisor_model = Some(resolved);
        }

        // Audio & Terminal Bell settings
        if let Some(snd) = first_non_empty_env(&[
            "FUSION_SOUND",
            "FUSION_BELL",
            "FUSION_AUDIO_CUES",
            "SOUND_ENABLED",
        ]) {
            let lower = snd.to_lowercase();
            if lower == "1" || lower == "true" || lower == "yes" || lower == "on" {
                cfg.sound_enabled = true;
            } else if lower == "0" || lower == "false" || lower == "no" || lower == "off" {
                cfg.sound_enabled = false;
            }
        }
        if let Some(no_bell) = first_non_empty_env(&["NO_BELL", "FUSION_NO_BELL"]) {
            let lower = no_bell.to_lowercase();
            if lower != "0" && lower != "false" {
                cfg.sound_enabled = false;
            }
        }
        if let Some(bell_comp) =
            first_non_empty_env(&["FUSION_BELL_COMPLETION", "BELL_ON_COMPLETION"])
        {
            let lower = bell_comp.to_lowercase();
            if lower == "0" || lower == "false" || lower == "no" || lower == "off" {
                cfg.bell_on_completion = false;
            } else if lower == "1" || lower == "true" || lower == "yes" || lower == "on" {
                cfg.bell_on_completion = true;
            }
        }
        if let Some(bell_err) = first_non_empty_env(&["FUSION_BELL_ERROR", "BELL_ON_ERROR"]) {
            let lower = bell_err.to_lowercase();
            if lower == "0" || lower == "false" || lower == "no" || lower == "off" {
                cfg.bell_on_error = false;
            } else if lower == "1" || lower == "true" || lower == "yes" || lower == "on" {
                cfg.bell_on_error = true;
            }
        }
        if let Some(notif) =
            first_non_empty_env(&["FUSION_NOTIFY", "FUSION_NOTIFICATIONS", "NOTIFY_ENABLED"])
        {
            let lower = notif.to_lowercase();
            if lower == "1" || lower == "true" || lower == "yes" || lower == "on" {
                cfg.notify_enabled = true;
            } else if lower == "0" || lower == "false" || lower == "no" || lower == "off" {
                cfg.notify_enabled = false;
            }
        }
        if let Some(no_notif) = first_non_empty_env(&["NO_NOTIFY", "FUSION_NO_NOTIFY"]) {
            let lower = no_notif.to_lowercase();
            if lower != "0" && lower != "false" {
                cfg.notify_enabled = false;
            }
        }
        if let Some(min_d) =
            first_non_empty_env(&["FUSION_NOTIFY_MIN_DURATION", "NOTIFY_MIN_DURATION"])
        {
            if let Ok(secs) = min_d.parse::<f64>() {
                if secs >= 0.0 {
                    cfg.notify_min_duration_secs = Some(secs);
                }
            }
        }
    }

    /// Applies workspace configuration overrides in-place.
    pub fn apply_workspace(&mut self, ws: &WorkspaceConfig) -> Vec<WorkspaceOverrideEntry> {
        ws.apply_to(self)
    }

    /// Applies workspace configuration loaded from a specific file path.
    pub fn apply_workspace_file(
        &mut self,
        path: &Path,
    ) -> Result<LoadedWorkspaceConfig, WorkspaceConfigError> {
        let format = WorkspaceConfigFormat::from_path(path).ok_or_else(|| {
            WorkspaceConfigError::UnsupportedFormat {
                path: path.to_path_buf(),
            }
        })?;
        let ws = WorkspaceConfig::load_from_file(path)?;
        let overridden_fields = ws.detect_overridden_fields();
        ws.apply_to(self);
        Ok(LoadedWorkspaceConfig {
            config: ws,
            source_path: path.to_path_buf(),
            format,
            overridden_fields,
        })
    }

    /// Save current configuration to `~/.fusion/config.json`.
    pub fn save(&self) -> std::io::Result<()> {
        let dir = Self::config_dir();
        std::fs::create_dir_all(&dir)?;
        let content = serde_json::to_string_pretty(self)?;
        std::fs::write(Self::config_path(), content)
    }

    /// Returns a `SoundConfig` reflecting the active audio cue settings.
    pub fn sound_config(&self) -> crate::ui::sound::SoundConfig {
        crate::ui::sound::SoundConfig {
            enabled: self.sound_enabled,
            bell_on_completion: self.bell_on_completion,
            bell_on_error: self.bell_on_error,
            tty_only: true,
        }
    }

    /// Returns a `VoiceConfig` reflecting the active voice input settings.
    pub fn voice_config(&self) -> crate::ui::voice::VoiceConfig {
        let mut voice = crate::ui::voice::VoiceConfig::from_env();
        if let Some(key) = &self.openai_api_key {
            if voice.api_key.is_none() {
                voice.api_key = Some(key.clone());
            }
        }
        voice
    }

    /// Returns a `NotificationConfig` reflecting the active notification settings.
    pub fn notification_config(&self) -> crate::ui::notify::NotificationConfig {
        crate::ui::notify::NotificationConfig {
            enabled: self.notify_enabled,
            desktop_enabled: true,
            terminal_enabled: false,
            sound: self.sound_enabled,
            min_duration_secs: self.notify_min_duration_secs,
            ..Default::default()
        }
    }

    /// Parse and auto-migrate configuration from a JSON string.
    pub fn from_json(json_str: &str) -> Result<Self, MigrationError> {
        let (cfg, _) = migration::migrate_str(json_str)?;
        Ok(cfg)
    }

    /// Load configuration from a specific path, running auto-migration and backing up if necessary.
    pub fn load_from_file(path: &Path) -> Result<(Self, MigrationOutcome), MigrationError> {
        migration::migrate_file(path, true)
    }

    /// Resolves a model alias or shorthand into `(provider, canonical_model)`.
    ///
    /// Examples:
    /// - `"sonnet"` -> `Some(("anthropic", "claude-3-5-sonnet-20241022"))`
    /// - `"4o"` -> `Some(("openai", "gpt-4o"))`
    /// - `"r1"` -> `Some(("deepseek", "deepseek-reasoner"))`
    pub fn resolve_model_shorthand(shorthand: &str) -> Option<(&'static str, &'static str)> {
        let clean = shorthand.trim().to_lowercase();
        for &(alias, provider, canonical) in MODEL_SHORTHANDS {
            if clean == alias {
                return Some((provider, canonical));
            }
        }
        None
    }

    pub fn detect_provider_for_model(model: &str) -> Option<&'static str> {
        let lower = model.trim().to_lowercase();

        if lower.starts_with("minimax") || lower.contains("minimax") {
            return Some("fusion");
        }
        if lower == "deepseek-ai/deepseek-v4-flash-0731" || lower.starts_with("deepseek-ai/") {
            return Some("fusion");
        }

        if lower.starts_with("claude") {
            return Some("anthropic");
        }
        if lower.starts_with("gpt-")
            || lower.starts_with("o1")
            || lower.starts_with("o3")
            || lower.starts_with("text-embedding")
        {
            return Some("openai");
        }
        if lower.starts_with("deepseek") {
            return Some("deepseek");
        }
        if lower.starts_with("grok") {
            return Some("xai");
        }
        if lower.contains('/') {
            return Some("openrouter");
        }
        if lower.starts_with("llama")
            || lower.starts_with("mistral")
            || lower.starts_with("mixtral")
            || lower.starts_with("qwen")
            || lower.contains(':')
        {
            return Some("ollama");
        }

        Some("fusion")
    }

    /// Resolves any model input, handling explicit prefixes (`provider:model` or `provider/model`),
    /// known shorthands, and fallbacks.
    ///
    /// Returns `(resolved_provider, resolved_model_name)`.
    pub fn resolve_model(input: &str, current_provider: Option<&str>) -> (String, String) {
        let trimmed = input.trim();

        // 1. Check for explicit "provider:model" syntax
        if let Some((prov, model_part)) = trimmed.split_once(':') {
            let prov_lower = prov.trim().to_lowercase();
            if Self::is_known_provider(&prov_lower) {
                let canonical = Self::canonical_model_for(&prov_lower, model_part.trim());
                return (prov_lower, canonical);
            }
        }

        // 2. Check for explicit "provider/model" syntax if the first part is a known provider
        if let Some((prov, model_part)) = trimmed.split_once('/') {
            let prov_lower = prov.trim().to_lowercase();
            if Self::is_known_provider(&prov_lower) {
                let canonical = Self::canonical_model_for(&prov_lower, model_part.trim());
                return (prov_lower, canonical);
            }
        }

        // 3. Check model shorthands
        if let Some((prov, canonical)) = Self::resolve_model_shorthand(trimmed) {
            return (prov.to_string(), canonical.to_string());
        }

        // 4. Try to infer provider from model name
        if let Some(detected_prov) = Self::detect_provider_for_model(trimmed) {
            return (detected_prov.to_string(), trimmed.to_string());
        }

        // 5. Fallback to current or default provider (fusion)
        let provider = current_provider.unwrap_or("fusion").to_lowercase();
        let canonical = Self::canonical_model_for(&provider, trimmed);
        (provider, canonical)
    }

    pub fn is_known_provider(provider: &str) -> bool {
        let lower = provider.trim().to_lowercase();
        SUPPORTED_PROVIDERS.contains(&lower.as_str())
            || lower == "grok"
            || lower == "claude"
            || lower == "fusion"
    }

    /// Normalizes model name for a specific provider.
    pub fn canonical_model_for(provider: &str, model: &str) -> String {
        let model_clean = model.trim();
        let model_lower = model_clean.to_lowercase();

        // Check if the model itself is a shorthand for this provider
        for &(alias, prov, canonical) in MODEL_SHORTHANDS {
            if prov == provider && (alias == model_lower || canonical.to_lowercase() == model_lower)
            {
                return canonical.to_string();
            }
        }

        model_clean.to_string()
    }

    /// Updates the default model, auto-detecting and updating the provider if appropriate.
    pub fn set_model(&mut self, model_or_shorthand: &str) {
        let (provider, model) =
            Self::resolve_model(model_or_shorthand, Some(&self.default_provider));
        self.default_provider = provider;
        self.default_model = model;
    }

    pub fn get_key_and_url(&self, provider: &str) -> (Option<String>, String) {
        let prov = provider.to_lowercase();
        match prov.as_str() {
            "fusion" => {
                let key = self
                    .fusion_api_key
                    .clone()
                    .or_else(|| std::env::var("FUSION_API_KEY").ok());
                let url = self
                    .fusion_base_url
                    .clone()
                    .or_else(|| std::env::var("FUSION_BASE_URL").ok())
                    .unwrap_or_else(|| "https://api.fusioncode.app/v1".to_string())
                    .replace("http://api.fusioncode.app", "https://api.fusioncode.app");
                (key, url)
            }
            "deepseek" => {
                let key = self.deepseek_api_key.clone();
                let url = self
                    .deepseek_base_url
                    .clone()
                    .unwrap_or_else(|| "https://api.deepseek.com".to_string());
                (key, url)
            }
            "anthropic" | "claude" => {
                let key = self.anthropic_api_key.clone();
                let url = self
                    .anthropic_base_url
                    .clone()
                    .unwrap_or_else(|| "https://api.anthropic.com/v1".to_string());
                (key, url)
            }
            "openai" => {
                let key = self.openai_api_key.clone();
                let url = self
                    .openai_base_url
                    .clone()
                    .unwrap_or_else(|| "https://api.openai.com/v1".to_string());
                (key, url)
            }
            "xai" | "grok" => {
                let key = self.xai_api_key.clone();
                let url = self
                    .xai_base_url
                    .clone()
                    .unwrap_or_else(|| "https://api.x.ai/v1".to_string());
                (key, url)
            }
            "openrouter" => {
                let key = self.openrouter_api_key.clone();
                let url = self
                    .openrouter_base_url
                    .clone()
                    .unwrap_or_else(|| "https://openrouter.ai/api/v1".to_string());
                (key, url)
            }
            "ollama" => {
                let url = self
                    .ollama_base_url
                    .clone()
                    .unwrap_or_else(|| "http://localhost:11434".to_string());
                (None, url)
            }
            _ => {
                let key = self
                    .fusion_api_key
                    .clone()
                    .or_else(|| std::env::var("FUSION_API_KEY").ok());
                let url = self
                    .fusion_base_url
                    .clone()
                    .or_else(|| std::env::var("FUSION_BASE_URL").ok())
                    .unwrap_or_else(|| "https://api.fusioncode.app/v1".to_string())
                    .replace("http://api.fusioncode.app", "https://api.fusioncode.app");
                (key, url)
            }
        }
    }

    /// Returns an actionable setup hint for the given provider's API key.
    pub fn key_hint(provider: &str) -> &'static str {
        match provider.to_lowercase().as_str() {
            "deepseek" => "Set DEEPSEEK_API_KEY in your environment (export DEEPSEEK_API_KEY=sk-...) or add \"deepseek_api_key\": \"sk-...\" to ~/.fusion/config.json. Get a key at https://platform.deepseek.com/",
            "anthropic" | "claude" => "Set ANTHROPIC_API_KEY in your environment (export ANTHROPIC_API_KEY=sk-ant-...) or add \"anthropic_api_key\": \"sk-ant-...\" to ~/.fusion/config.json. Get a key at https://console.anthropic.com/",
            "openai" => "Set OPENAI_API_KEY in your environment (export OPENAI_API_KEY=sk-...) or add \"openai_api_key\": \"sk-...\" to ~/.fusion/config.json. Get a key at https://platform.openai.com/api-keys",
            "xai" | "grok" => "Set XAI_API_KEY in your environment (export XAI_API_KEY=xai-...) or add \"xai_api_key\": \"xai-...\" to ~/.fusion/config.json. Get a key at https://console.x.ai/",
            "openrouter" => "Set OPENROUTER_API_KEY in your environment (export OPENROUTER_API_KEY=sk-or-...) or add \"openrouter_api_key\": \"sk-or-...\" to ~/.fusion/config.json. Get a key at https://openrouter.ai/keys",
            "ollama" => "Ensure Ollama is running locally ('ollama serve' or 'ollama run <model>') and accessible at http://localhost:11434, or set OLLAMA_BASE_URL.",
            _ => "Set FUSION_API_KEY or configure the provider API key in ~/.fusion/config.json",
        }
    }

    /// Validates configuration for a specific provider.
    pub fn validate_provider(&self, provider: &str) -> Result<(), ConfigError> {
        let prov_lower = provider.to_lowercase();
        if !Self::is_known_provider(&prov_lower) {
            return Err(ConfigError::UnknownProvider {
                provider: provider.to_string(),
            });
        }

        let (key, url) = self.get_key_and_url(&prov_lower);

        // Ollama does not require an API key
        if prov_lower == "ollama" {
            if url.trim().is_empty() {
                return Err(ConfigError::InvalidBaseUrl {
                    provider: provider.to_string(),
                    url,
                    hint: Self::key_hint("ollama").to_string(),
                });
            }
            return Ok(());
        }

        // Other providers require an API key
        match key {
            Some(k) if !k.trim().is_empty() => Ok(()),
            _ => Err(ConfigError::MissingApiKey {
                provider: provider.to_string(),
                hint: Self::key_hint(&prov_lower).to_string(),
            }),
        }
    }

    /// Validates the active configuration, checking the default provider, model, temperature, and tokens.
    pub fn validate(&self) -> Result<(), ConfigError> {
        self.validate_provider(&self.default_provider)?;

        if let Some(temp) = self.default_temperature {
            if !(0.0..=2.0).contains(&temp) {
                return Err(ConfigError::InvalidValue {
                    field: "default_temperature".to_string(),
                    message: format!("temperature {} is outside valid range [0.0, 2.0]", temp),
                    hint: "Set temperature between 0.0 (deterministic) and 2.0 (creative)."
                        .to_string(),
                });
            }
        }

        if let Some(tokens) = self.max_tokens {
            if tokens == 0 {
                return Err(ConfigError::InvalidValue {
                    field: "max_tokens".to_string(),
                    message: "max_tokens cannot be 0".to_string(),
                    hint: "Set max_tokens to a positive integer (e.g. 4096 or 8192).".to_string(),
                });
            }
        }

        if let Some(turns) = self.max_turns {
            if !(10..=500).contains(&turns) {
                return Err(ConfigError::InvalidValue {
                    field: "max_turns".to_string(),
                    message: format!("max_turns {} is outside valid range [10, 500]", turns),
                    hint: "Set max_turns between 10 and 500.".to_string(),
                });
            }
        }

        Ok(())
    }

    /// Returns a list of all providers that currently have valid API keys (or Ollama).
    pub fn configured_providers(&self) -> Vec<String> {
        let mut available = Vec::new();
        for &prov in SUPPORTED_PROVIDERS {
            if self.validate_provider(prov).is_ok() {
                available.push(prov.to_string());
            }
        }
        available
    }

    /// Creates a new configuration instance initialized from a preset profile.
    pub fn from_preset(preset: ConfigPreset) -> Self {
        preset.to_config()
    }

    /// Applies a preset profile to this configuration in-place.
    pub fn apply_preset(&mut self, preset: ConfigPreset) {
        preset.apply_to(self);
    }

    /// Looks up a preset by loose name/alias and applies it in-place.
    pub fn apply_preset_by_name(&mut self, name: &str) -> Result<ConfigPreset, PresetError> {
        let preset =
            ConfigPreset::from_str_loose(name).ok_or_else(|| PresetError::UnknownPreset {
                name: name.to_string(),
            })?;
        self.apply_preset(preset);
        Ok(preset)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sanitize_env_var() {
        assert_eq!(
            sanitize_env_var("  sk-123456  "),
            Some("sk-123456".to_string())
        );
        assert_eq!(
            sanitize_env_var("\"sk-123456\""),
            Some("sk-123456".to_string())
        );
        assert_eq!(
            sanitize_env_var("'sk-123456'"),
            Some("sk-123456".to_string())
        );
        assert_eq!(sanitize_env_var(""), None);
        assert_eq!(sanitize_env_var("   "), None);
        assert_eq!(sanitize_env_var("your-api-key-here"), None);
        assert_eq!(sanitize_env_var("YOUR_API_KEY"), None);
        assert_eq!(sanitize_env_var("sk-..."), None);
        assert_eq!(sanitize_env_var("<API_KEY>"), None);
        assert_eq!(sanitize_env_var("none"), None);
    }

    #[test]
    fn test_sanitize_base_url() {
        assert_eq!(
            sanitize_base_url("https://api.openai.com/v1/"),
            Some("https://api.openai.com/v1".to_string())
        );
        assert_eq!(
            sanitize_base_url("http://localhost:11434/"),
            Some("http://localhost:11434".to_string())
        );
        assert_eq!(
            sanitize_base_url("localhost:11434"),
            Some("http://localhost:11434".to_string())
        );
        assert_eq!(sanitize_base_url(""), None);
    }

    #[test]
    fn test_model_shorthands_resolution() {
        // DeepSeek
        assert_eq!(
            Config::resolve_model_shorthand("r1"),
            Some(("deepseek", "deepseek-reasoner"))
        );
        assert_eq!(
            Config::resolve_model_shorthand("deepseek-chat"),
            Some(("deepseek", "deepseek-chat"))
        );
        assert_eq!(
            Config::resolve_model_shorthand("v3"),
            Some(("deepseek", "deepseek-chat"))
        );

        // Anthropic
        assert_eq!(
            Config::resolve_model_shorthand("sonnet"),
            Some(("anthropic", "claude-3-5-sonnet-20241022"))
        );
        assert_eq!(
            Config::resolve_model_shorthand("claude-3.7-sonnet"),
            Some(("anthropic", "claude-3-7-sonnet-20250219"))
        );
        assert_eq!(
            Config::resolve_model_shorthand("haiku"),
            Some(("anthropic", "claude-3-5-haiku-20241022"))
        );
        assert_eq!(
            Config::resolve_model_shorthand("opus"),
            Some(("anthropic", "claude-3-opus-20240229"))
        );

        // OpenAI
        assert_eq!(
            Config::resolve_model_shorthand("4o"),
            Some(("openai", "gpt-4o"))
        );
        assert_eq!(
            Config::resolve_model_shorthand("4o-mini"),
            Some(("openai", "gpt-4o-mini"))
        );
        assert_eq!(
            Config::resolve_model_shorthand("o3-mini"),
            Some(("openai", "o3-mini"))
        );

        // xAI
        assert_eq!(
            Config::resolve_model_shorthand("grok"),
            Some(("xai", "grok-2-latest"))
        );
        assert_eq!(
            Config::resolve_model_shorthand("grok-vision"),
            Some(("xai", "grok-2-vision-1212"))
        );

        // Ollama
        assert_eq!(
            Config::resolve_model_shorthand("llama3.3"),
            Some(("ollama", "llama3.3"))
        );
        assert_eq!(
            Config::resolve_model_shorthand("qwen-coder"),
            Some(("ollama", "qwen2.5-coder"))
        );

        // Fusion
        assert_eq!(
            Config::resolve_model_shorthand("minimax"),
            Some(("fusion", "MiniMaxAI/MiniMax-M2.7"))
        );
        assert_eq!(
            Config::resolve_model_shorthand("deepseek-v4"),
            Some(("fusion", "deepseek-ai/DeepSeek-V4-Flash-0731"))
        );
        assert_eq!(
            Config::resolve_model_shorthand("flash"),
            Some(("fusion", "deepseek-ai/DeepSeek-V4-Flash-0731"))
        );
    }

    #[test]
    fn test_resolve_model_with_prefix() {
        assert_eq!(
            Config::resolve_model("openai:gpt-4o", None),
            ("openai".to_string(), "gpt-4o".to_string())
        );
        assert_eq!(
            Config::resolve_model("anthropic:sonnet", None),
            (
                "anthropic".to_string(),
                "claude-3-5-sonnet-20241022".to_string()
            )
        );
        assert_eq!(
            Config::resolve_model("openrouter/meta-llama/llama-3-70b", None),
            (
                "openrouter".to_string(),
                "meta-llama/llama-3-70b".to_string()
            )
        );
    }

    #[test]
    fn test_provider_detection() {
        assert_eq!(
            Config::detect_provider_for_model("claude-3-5-sonnet-20241022"),
            Some("anthropic")
        );
        assert_eq!(Config::detect_provider_for_model("gpt-4o"), Some("openai"));
        assert_eq!(
            Config::detect_provider_for_model("deepseek-chat"),
            Some("deepseek")
        );
        assert_eq!(
            Config::detect_provider_for_model("grok-2-latest"),
            Some("xai")
        );
        assert_eq!(
            Config::detect_provider_for_model("meta-llama/llama-3-70b-instruct"),
            Some("openrouter")
        );
        assert_eq!(
            Config::detect_provider_for_model("llama3.2:3b"),
            Some("ollama")
        );
        assert_eq!(
            Config::detect_provider_for_model("MiniMaxAI/MiniMax-M2.7"),
            Some("fusion")
        );
        assert_eq!(
            Config::detect_provider_for_model("deepseek-ai/DeepSeek-V4-Flash-0731"),
            Some("fusion")
        );
    }

    #[test]
    fn test_config_validation_and_hints() {
        let mut cfg = Config::default();
        cfg.default_provider = "anthropic".to_string();
        cfg.anthropic_api_key = None;

        let res = cfg.validate();
        assert!(res.is_err());
        if let Err(ConfigError::MissingApiKey { provider, hint }) = res {
            assert_eq!(provider, "anthropic");
            assert!(hint.contains("ANTHROPIC_API_KEY"));
        } else {
            panic!("Expected MissingApiKey error");
        }

        // Ollama validation passes without key
        let mut ollama_cfg = Config::default();
        ollama_cfg.default_provider = "ollama".to_string();
        assert!(ollama_cfg.validate().is_ok());
    }

    #[test]
    fn test_config_serialization() {
        let cfg = Config::default();
        let json = serde_json::to_string(&cfg).expect("serialize config");
        let parsed: Config = serde_json::from_str(&json).expect("deserialize config");
        assert_eq!(parsed.default_provider, "fusion");
        assert_eq!(parsed.default_model, "deepseek-ai/DeepSeek-V4-Flash-0731");
        assert!(!parsed.advisors_enabled);
    }

    #[test]
    fn test_sound_config_defaults_and_serde() {
        let cfg = Config::default();
        assert!(!cfg.sound_enabled);
        assert!(cfg.bell_on_completion);
        assert!(cfg.bell_on_error);

        let sound_cfg = cfg.sound_config();
        assert!(!sound_cfg.enabled);
        assert!(sound_cfg.bell_on_completion);
        assert!(sound_cfg.bell_on_error);

        // Roundtrip JSON
        let json = serde_json::json!({
            "sound_enabled": true,
            "bell_on_completion": true,
            "bell_on_error": false
        });
        let parsed: Config = serde_json::from_value(json).unwrap();
        assert!(parsed.sound_enabled);
        assert!(parsed.bell_on_completion);
        assert!(!parsed.bell_on_error);
    }

    #[test]
    fn test_notification_config_defaults_and_serde() {
        let cfg = Config::default();
        assert!(cfg.notify_enabled);
        assert!(cfg.notify_on_completion);
        assert!(cfg.notify_on_error);
        assert!(cfg.notify_min_duration_secs.is_none());

        let notif_cfg = cfg.notification_config();
        assert!(notif_cfg.enabled);
        assert!(notif_cfg.desktop_enabled);
        assert!(!notif_cfg.terminal_enabled);
        assert!(!notif_cfg.sound);

        // Roundtrip JSON
        let json = serde_json::json!({
            "notify_enabled": false,
            "notify_on_completion": true,
            "notify_on_error": false,
            "notify_min_duration_secs": 5.5
        });
        let parsed: Config = serde_json::from_value(json).unwrap();
        assert!(!parsed.notify_enabled);
        assert!(parsed.notify_on_completion);
        assert!(!parsed.notify_on_error);
        assert_eq!(parsed.notify_min_duration_secs, Some(5.5));
    }

    #[test]
    fn test_config_from_preset() {
        let cfg = Config::from_preset(ConfigPreset::CodingFast);
        assert_eq!(cfg.default_provider, "anthropic");
        assert_eq!(cfg.default_model, "claude-3-5-sonnet-20241022");
        assert_eq!(cfg.max_tokens, Some(8192));
        assert!(cfg.advisors_enabled);
    }

    #[test]
    fn test_config_apply_preset_by_name() {
        let mut cfg = Config::default();
        let preset = cfg.apply_preset_by_name("offline-ollama").unwrap();
        assert_eq!(preset, ConfigPreset::OfflineOllama);
        assert_eq!(cfg.default_provider, "ollama");
        assert_eq!(cfg.default_model, "qwen2.5-coder");
        assert!(!cfg.advisors_enabled);

        let err = cfg.apply_preset_by_name("non-existent");
        assert!(err.is_err());
    }

    #[test]
    fn test_load_for_workspace_with_local_toml() {
        let temp_dir = tempfile::tempdir().unwrap();
        let ws_toml = temp_dir.path().join(".fusion.toml");
        std::fs::write(
            &ws_toml,
            r#"
provider = "xai"
model = "grok-2"
temperature = 0.8
max_tokens = 2048
advisors_enabled = false
"#,
        )
        .unwrap();

        let cfg = Config::load_for_workspace(temp_dir.path());
        assert_eq!(cfg.default_provider, "xai");
        assert_eq!(cfg.default_model, "grok-2-latest");
        assert_eq!(cfg.default_temperature, Some(0.8));
        assert_eq!(cfg.max_tokens, Some(2048));
        assert!(!cfg.advisors_enabled);
    }

    #[test]
    fn test_load_for_workspace_with_local_json() {
        let temp_dir = tempfile::tempdir().unwrap();
        let ws_json = temp_dir.path().join(".fusion.json");
        std::fs::write(
            &ws_json,
            r#"{
  "provider": "openrouter",
  "model": "deepseek-r1",
  "temperature": 0.1,
  "max_tokens": 16384,
  "sound_enabled": true
}"#,
        )
        .unwrap();

        let cfg = Config::load_for_workspace(temp_dir.path());
        assert_eq!(cfg.default_provider, "openrouter");
        assert_eq!(cfg.default_model, "deepseek-reasoner");
        assert_eq!(cfg.default_temperature, Some(0.1));
        assert_eq!(cfg.max_tokens, Some(16384));
        assert!(cfg.sound_enabled);
    }

    #[test]
    fn test_config_max_turns_default() {
        let cfg = Config::default();
        assert_eq!(cfg.max_turns, Some(100));
    }

    #[test]
    fn test_config_max_turns_env_override_and_clamp() {
        let mut cfg = Config::default();
        // Test clamping below min (10)
        std::env::set_var("FUSION_MAX_TURNS", "5");
        Config::apply_env_overrides(&mut cfg);
        assert_eq!(cfg.max_turns, Some(10));

        // Test clamping above max (500)
        std::env::set_var("FUSION_MAX_TURNS", "999");
        Config::apply_env_overrides(&mut cfg);
        assert_eq!(cfg.max_turns, Some(500));

        // Test normal in-range value
        std::env::set_var("FUSION_MAX_TURNS", "75");
        Config::apply_env_overrides(&mut cfg);
        assert_eq!(cfg.max_turns, Some(75));

        // Test MAX_TURNS alias
        std::env::remove_var("FUSION_MAX_TURNS");
        std::env::set_var("MAX_TURNS", "120");
        Config::apply_env_overrides(&mut cfg);
        assert_eq!(cfg.max_turns, Some(120));

        std::env::remove_var("MAX_TURNS");
    }

    #[test]
    fn test_config_max_turns_serde_json() {
        let json = r#"{
            "provider": "deepseek",
            "model": "deepseek-chat",
            "max_turns": 42
        }"#;
        let cfg: Config = serde_json::from_str(json).unwrap();
        assert_eq!(cfg.max_turns, Some(42));
    }

    #[test]
    fn test_config_max_turns_validation() {
        let mut cfg = Config::default();
        cfg.max_turns = Some(5);
        assert!(cfg.validate().is_err());

        cfg.max_turns = Some(600);
        assert!(cfg.validate().is_err());

        cfg.max_turns = Some(100);
        cfg.default_provider = "ollama".to_string();
        assert!(cfg.validate().is_ok());

        // Outside valid range should fail even if provider is valid
        cfg.max_turns = Some(5);
        assert!(cfg.validate().is_err());
        cfg.max_turns = Some(501);
        assert!(cfg.validate().is_err());
    }
}
