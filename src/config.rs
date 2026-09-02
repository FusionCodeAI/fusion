use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Supported LLM providers in Fusion.
pub const SUPPORTED_PROVIDERS: &[&str] = &[
    "deepseek",
    "anthropic",
    "openai",
    "xai",
    "openrouter",
    "ollama",
];

/// Known model shorthands mapping an alias to `(provider, canonical_model_name)`.
pub const MODEL_SHORTHANDS: &[(&str, &str, &str)] = &[
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
    ("claude-3-7-sonnet-latest", "anthropic", "claude-3-7-sonnet-20250219"),
    ("claude-3-7-sonnet-20250219", "anthropic", "claude-3-7-sonnet-20250219"),
    ("claude-3-7-sonnet", "anthropic", "claude-3-7-sonnet-20250219"),
    ("claude-3.7-sonnet", "anthropic", "claude-3-7-sonnet-20250219"),
    ("claude-3-7", "anthropic", "claude-3-7-sonnet-20250219"),
    ("claude-3.7", "anthropic", "claude-3-7-sonnet-20250219"),
    ("3.7-sonnet", "anthropic", "claude-3-7-sonnet-20250219"),
    ("sonnet-3.7", "anthropic", "claude-3-7-sonnet-20250219"),
    ("claude-3-5-sonnet-latest", "anthropic", "claude-3-5-sonnet-20241022"),
    ("claude-3-5-sonnet-20241022", "anthropic", "claude-3-5-sonnet-20241022"),
    ("claude-3-5-sonnet", "anthropic", "claude-3-5-sonnet-20241022"),
    ("claude-3.5-sonnet", "anthropic", "claude-3-5-sonnet-20241022"),
    ("sonnet-3.5", "anthropic", "claude-3-5-sonnet-20241022"),
    ("sonnet-3-5", "anthropic", "claude-3-5-sonnet-20241022"),
    ("claude-sonnet", "anthropic", "claude-3-5-sonnet-20241022"),
    ("sonnet", "anthropic", "claude-3-5-sonnet-20241022"),
    ("claude", "anthropic", "claude-3-5-sonnet-20241022"),
    ("claude-3-5-haiku-latest", "anthropic", "claude-3-5-haiku-20241022"),
    ("claude-3-5-haiku-20241022", "anthropic", "claude-3-5-haiku-20241022"),
    ("claude-3-5-haiku", "anthropic", "claude-3-5-haiku-20241022"),
    ("claude-3.5-haiku", "anthropic", "claude-3-5-haiku-20241022"),
    ("haiku-3.5", "anthropic", "claude-3-5-haiku-20241022"),
    ("haiku-3-5", "anthropic", "claude-3-5-haiku-20241022"),
    ("claude-haiku", "anthropic", "claude-3-5-haiku-20241022"),
    ("haiku", "anthropic", "claude-3-5-haiku-20241022"),
    ("claude-3-opus-latest", "anthropic", "claude-3-opus-20240229"),
    ("claude-3-opus-20240229", "anthropic", "claude-3-opus-20240229"),
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
    MissingApiKey {
        provider: String,
        hint: String,
    },

    #[error("Invalid base URL for provider '{provider}': '{url}'.\nHint: {hint}")]
    InvalidBaseUrl {
        provider: String,
        url: String,
        hint: String,
    },

    #[error("Unknown provider '{provider}'.\nHint: Available providers are: {}", SUPPORTED_PROVIDERS.join(", "))]
    UnknownProvider {
        provider: String,
    },

    #[error("Invalid configuration for '{field}': {message}\nHint: {hint}")]
    InvalidValue {
        field: String,
        message: String,
        hint: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    #[serde(default = "default_provider_name")]
    pub default_provider: String,

    #[serde(default = "default_model_name")]
    pub default_model: String,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_temperature: Option<f32>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_tokens: Option<u32>,

    // Provider API keys & custom base URLs
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub openai_api_key: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub openai_base_url: Option<String>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub anthropic_api_key: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub anthropic_base_url: Option<String>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deepseek_api_key: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deepseek_base_url: Option<String>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub xai_api_key: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub xai_base_url: Option<String>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub openrouter_api_key: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub openrouter_base_url: Option<String>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ollama_base_url: Option<String>,

    // Multi-Agent & Advisor configurations
    #[serde(default = "default_true")]
    pub advisors_enabled: bool,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub advisor_model: Option<String>,
}

fn default_provider_name() -> String {
    "deepseek".to_string()
}

fn default_model_name() -> String {
    "deepseek-chat".to_string()
}

fn default_true() -> bool {
    true
}

impl Default for Config {
    fn default() -> Self {
        Self {
            default_provider: default_provider_name(),
            default_model: default_model_name(),
            default_temperature: Some(0.2),
            max_tokens: Some(8192),

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

            advisors_enabled: true,
            advisor_model: None,
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
    /// 3. Environment variables (with fallback aliases and validation)
    pub fn load() -> Self {
        let mut cfg = Self::default();
        let path = Self::config_path();

        if path.exists() {
            if let Ok(content) = std::fs::read_to_string(&path) {
                if let Ok(loaded) = serde_json::from_str::<Config>(&content) {
                    cfg = loaded;
                }
            }
        }

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
            if cfg.xai_api_key.is_none() && (cfg.default_provider == "xai" || cfg.default_provider == "grok") {
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
            let (_, resolved) = Self::resolve_model(&cfg.default_model, Some(&cfg.default_provider));
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

        cfg
    }

    /// Save current configuration to `~/.fusion/config.json`.
    pub fn save(&self) -> std::io::Result<()> {
        let dir = Self::config_dir();
        std::fs::create_dir_all(&dir)?;
        let content = serde_json::to_string_pretty(self)?;
        std::fs::write(Self::config_path(), content)
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

    /// Detects the most probable provider for a model string based on naming conventions.
    pub fn detect_provider_for_model(model: &str) -> Option<&'static str> {
        let lower = model.trim().to_lowercase();

        if lower.starts_with("claude") {
            return Some("anthropic");
        }
        if lower.starts_with("gpt-") || lower.starts_with("o1") || lower.starts_with("o3") || lower.starts_with("text-embedding") {
            return Some("openai");
        }
        if lower.starts_with("deepseek") {
            return Some("deepseek");
        }
        if lower.starts_with("grok") {
            return Some("xai");
        }
        if lower.contains('/') {
            // Models containing a slash like "meta-llama/llama-3-70b" are standard OpenRouter models
            return Some("openrouter");
        }
        if lower.starts_with("llama") || lower.starts_with("mistral") || lower.starts_with("mixtral") || lower.starts_with("qwen") || lower.contains(':') {
            return Some("ollama");
        }

        None
    }

    /// Resolves any model input, handling explicit prefixes (`provider:model` or `provider/model`),
    /// known shorthands, and fallbacks.
    ///
    /// Returns `(resolved_provider, resolved_model_name)`.
    pub fn resolve_model(input: &str, current_provider: Option<&str>) -> (String, String) {
        let trimmed = input.trim();

        // 1. Check for explicit "provider:model" syntax (e.g. "openai:gpt-4o", "anthropic:sonnet")
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

        // 5. Fallback to current or default provider
        let provider = current_provider.unwrap_or("deepseek").to_lowercase();
        let canonical = Self::canonical_model_for(&provider, trimmed);
        (provider, canonical)
    }

    /// Checks if a provider identifier matches a known provider.
    pub fn is_known_provider(provider: &str) -> bool {
        let lower = provider.trim().to_lowercase();
        SUPPORTED_PROVIDERS.contains(&lower.as_str()) || lower == "grok" || lower == "claude"
    }

    /// Normalizes model name for a specific provider.
    pub fn canonical_model_for(provider: &str, model: &str) -> String {
        let model_clean = model.trim();
        let model_lower = model_clean.to_lowercase();

        // Check if the model itself is a shorthand for this provider
        for &(alias, prov, canonical) in MODEL_SHORTHANDS {
            if prov == provider && (alias == model_lower || canonical.to_lowercase() == model_lower) {
                return canonical.to_string();
            }
        }

        model_clean.to_string()
    }

    /// Updates the default model, auto-detecting and updating the provider if appropriate.
    pub fn set_model(&mut self, model_or_shorthand: &str) {
        let (provider, model) = Self::resolve_model(model_or_shorthand, Some(&self.default_provider));
        self.default_provider = provider;
        self.default_model = model;
    }

    /// Returns the API key and Base URL for the specified provider, using fallback resolution.
    pub fn get_key_and_url(&self, provider: &str) -> (Option<String>, String) {
        let prov = provider.to_lowercase();
        match prov.as_str() {
            "deepseek" => {
                let key = self.deepseek_api_key.clone();
                let url = self.deepseek_base_url.clone().unwrap_or_else(|| "https://api.deepseek.com".to_string());
                (key, url)
            }
            "anthropic" | "claude" => {
                let key = self.anthropic_api_key.clone();
                let url = self.anthropic_base_url.clone().unwrap_or_else(|| "https://api.anthropic.com/v1".to_string());
                (key, url)
            }
            "openai" => {
                let key = self.openai_api_key.clone();
                let url = self.openai_base_url.clone().unwrap_or_else(|| "https://api.openai.com/v1".to_string());
                (key, url)
            }
            "xai" | "grok" => {
                let key = self.xai_api_key.clone();
                let url = self.xai_base_url.clone().unwrap_or_else(|| "https://api.x.ai/v1".to_string());
                (key, url)
            }
            "openrouter" => {
                let key = self.openrouter_api_key.clone();
                let url = self.openrouter_base_url.clone().unwrap_or_else(|| "https://openrouter.ai/api/v1".to_string());
                (key, url)
            }
            "ollama" => {
                let url = self.ollama_base_url.clone().unwrap_or_else(|| "http://localhost:11434".to_string());
                (None, url)
            }
            _ => {
                // Fallback for custom or unknown provider
                (
                    self.openai_api_key.clone(),
                    self.openai_base_url
                        .clone()
                        .unwrap_or_else(|| "https://api.openai.com/v1".to_string()),
                )
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
                    hint: "Set temperature between 0.0 (deterministic) and 2.0 (creative).".to_string(),
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
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sanitize_env_var() {
        assert_eq!(sanitize_env_var("  sk-123456  "), Some("sk-123456".to_string()));
        assert_eq!(sanitize_env_var("\"sk-123456\""), Some("sk-123456".to_string()));
        assert_eq!(sanitize_env_var("'sk-123456'"), Some("sk-123456".to_string()));
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
    }

    #[test]
    fn test_resolve_model_with_prefix() {
        assert_eq!(
            Config::resolve_model("openai:gpt-4o", None),
            ("openai".to_string(), "gpt-4o".to_string())
        );
        assert_eq!(
            Config::resolve_model("anthropic:sonnet", None),
            ("anthropic".to_string(), "claude-3-5-sonnet-20241022".to_string())
        );
        assert_eq!(
            Config::resolve_model("openrouter/meta-llama/llama-3-70b", None),
            ("openrouter".to_string(), "meta-llama/llama-3-70b".to_string())
        );
    }

    #[test]
    fn test_provider_detection() {
        assert_eq!(
            Config::detect_provider_for_model("claude-3-5-sonnet-20241022"),
            Some("anthropic")
        );
        assert_eq!(
            Config::detect_provider_for_model("gpt-4o"),
            Some("openai")
        );
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
        assert_eq!(parsed.default_provider, "deepseek");
        assert_eq!(parsed.default_model, "deepseek-chat");
        assert!(parsed.advisors_enabled);
    }
}
