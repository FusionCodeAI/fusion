use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::Context;
use serde::{Deserialize, Serialize};
use tracing::{debug, error, info, warn};

use crate::config::Config;

/// Default time-to-live for the model catalog cache (24 hours in seconds).
pub const DEFAULT_CATALOG_TTL_SECS: u64 = 86_400;

/// Version number for the cache file format.
pub const CATALOG_CACHE_VERSION: u32 = 1;

/// Default HTTP request timeout when dynamically fetching provider models.
pub const CATALOG_FETCH_TIMEOUT: Duration = Duration::from_secs(5);

/// Default base URLs for provider model queries.
pub const DEFAULT_OPENAI_BASE_URL: &str = "https://api.openai.com/v1";
pub const DEFAULT_DEEPSEEK_BASE_URL: &str = "https://api.deepseek.com";
pub const DEFAULT_OPENROUTER_BASE_URL: &str = "https://openrouter.ai/api/v1";
pub const DEFAULT_GROQ_BASE_URL: &str = "https://api.groq.com/openai/v1";
pub const DEFAULT_FUSION_BASE_URL: &str = "https://api.fusioncode.app/v1";
pub const DEFAULT_ANTHROPIC_BASE_URL: &str = "https://api.anthropic.com/v1";
pub const DEFAULT_XAI_BASE_URL: &str = "https://api.x.ai/v1";

/// A single LLM model entry in the catalog.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub struct Capabilities {
    /// Excels at multi-step reasoning / chain-of-thought tasks.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub reasoning: bool,
    /// Accepts image inputs (multimodal).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub vision: bool,
    /// Supports free-form tool / function calling.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub tool_use: bool,
    /// Supports strict JSON-schema function calling.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub function_calling: bool,
}

impl Capabilities {
    /// A capability set with every flag enabled.
    pub const ALL: Self = Self {
        reasoning: true,
        vision: true,
        tool_use: true,
        function_calling: true,
    };

    /// A capability set with every flag disabled.
    pub const NONE: Self = Self {
        reasoning: false,
        vision: false,
        tool_use: false,
        function_calling: false,
    };

    /// Returns true if any capability flag is set.
    pub fn any(&self) -> bool {
        self.reasoning || self.vision || self.tool_use || self.function_calling
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Default)]
pub struct ModelPricing {
    /// Cost in USD per 1M input (prompt) tokens.
    pub input_per_m: f64,
    /// Cost in USD per 1M output (completion) tokens.
    pub output_per_m: f64,
}

impl ModelPricing {
    /// Creates a pricing entry from per-1M-token USD rates.
    pub fn new(input_per_m: f64, output_per_m: f64) -> Self {
        Self {
            input_per_m,
            output_per_m,
        }
    }

    /// Formats the pricing as a compact human-readable string (e.g. `"$3.00 / $15.00 per 1M"`).
    pub fn formatted(&self) -> String {
        format!("${:.2} / ${:.2} per 1M", self.input_per_m, self.output_per_m)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CatalogModel {
    pub id: String,
    pub name: String,
    pub provider: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context_window: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_output_tokens: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input_cost_per_m: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_cost_per_m: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub knowledge_cutoff: Option<String>,
    #[serde(default, skip_serializing_if = "Capabilities::any")]
    pub capabilities: Capabilities,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub badges: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

pub type ModelEntry = CatalogModel;

impl CatalogModel {
    pub fn new(
        id: impl Into<String>,
        name: impl Into<String>,
        provider: impl Into<String>,
    ) -> Self {
        Self {
            id: id.into(),
            name: name.into(),
            provider: provider.into(),
            context_window: None,
            max_output_tokens: None,
            input_cost_per_m: None,
            output_cost_per_m: None,
            knowledge_cutoff: None,
            capabilities: Capabilities::NONE,
            badges: Vec::new(),
            description: None,
        }
    }

    pub fn with_context(mut self, tokens: u64) -> Self {
        self.context_window = Some(tokens);
        self
    }

    pub fn with_max_output(mut self, tokens: u64) -> Self {
        self.max_output_tokens = Some(tokens);
        self
    }

    /// Sets the input/output pricing (USD per 1M tokens).
    pub fn with_pricing(mut self, input_per_m: f64, output_per_m: f64) -> Self {
        self.input_cost_per_m = Some(input_per_m);
        self.output_cost_per_m = Some(output_per_m);
        self
    }

    /// Sets the knowledge cutoff date (e.g. `"2024-10"`).
    pub fn with_knowledge_cutoff(mut self, cutoff: impl Into<String>) -> Self {
        self.knowledge_cutoff = Some(cutoff.into());
        self
    }

    /// Replaces the capability flags wholesale.
    pub fn with_capabilities(mut self, caps: Capabilities) -> Self {
        self.capabilities = caps;
        self
    }

    /// Enables the reasoning capability flag.
    pub fn with_reasoning(mut self) -> Self {
        self.capabilities.reasoning = true;
        self
    }

    /// Enables the vision capability flag.
    pub fn with_vision(mut self) -> Self {
        self.capabilities.vision = true;
        self
    }

    /// Enables the tool-use capability flag.
    pub fn with_tool_use(mut self) -> Self {
        self.capabilities.tool_use = true;
        self
    }

    /// Enables the function-calling capability flag.
    pub fn with_function_calling(mut self) -> Self {
        self.capabilities.function_calling = true;
        self
    }

    pub fn with_badge(mut self, badge: impl Into<String>) -> Self {
        self.badges.push(badge.into());
        self
    }

    pub fn with_badges<I, S>(mut self, badges: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        for b in badges {
            self.badges.push(b.into());
        }
        self
    }

    pub fn with_description(mut self, desc: impl Into<String>) -> Self {
        self.description = Some(desc.into());
        self
    }

    /// Formats the context window into a concise human-readable string (e.g. "200K context", "1M context").
    pub fn formatted_context_window(&self) -> String {
        match self.context_window {
            Some(w) if w >= 1_000_000 => {
                let m = (w as f64) / 1_000_000.0;
                if m.fract() == 0.0 {
                    format!("{:.0}M context", m)
                } else {
                    format!("{:.1}M context", m)
                }
            }
            Some(w) if w >= 1_000 => format!("{}K context", w / 1_000),
            Some(w) => format!("{} context", w),
            None => "-".to_string(),
        }
    }

    /// Formats the max output tokens into a concise human-readable string (e.g. "64K output", "8K output").
    pub fn formatted_max_output(&self) -> String {
        match self.max_output_tokens {
            Some(w) if w >= 1_000_000 => {
                let m = (w as f64) / 1_000_000.0;
                if m.fract() == 0.0 {
                    format!("{:.0}M output", m)
                } else {
                    format!("{:.1}M output", m)
                }
            }
            Some(w) if w >= 1_000 => format!("{}K output", w / 1_000),
            Some(w) => format!("{} output", w),
            None => "-".to_string(),
        }
    }

    /// Formats the combined input/output pricing (e.g. `"$3.00 / $15.00 per 1M"`), or `-` when unknown.
    pub fn formatted_pricing(&self) -> String {
        match (self.input_cost_per_m, self.output_cost_per_m) {
            (Some(in_cost), Some(out_cost)) => {
                ModelPricing::new(in_cost, out_cost).formatted()
            }
            _ => "-".to_string(),
        }
    }

    /// Checks whether this model possesses a specific badge (case-insensitive).
    pub fn has_badge(&self, badge: &str) -> bool {
        self.badges.iter().any(|b| b.eq_ignore_ascii_case(badge))
    }

    /// Determines if the model specializes in reasoning / thinking.
    pub fn is_reasoning(&self) -> bool {
        self.capabilities.reasoning
            || self.has_badge("reasoning")
            || self.id.contains("r1")
            || self.id.contains("o1")
            || self.id.contains("o3")
            || self.id.contains("reasoner")
            || self.id.contains("thinking")
    }

    /// Determines if the model supports multimodal / vision inputs.
    pub fn is_vision(&self) -> bool {
        self.capabilities.vision
            || self.has_badge("vision")
            || self.id.contains("vision")
            || self.id.contains("4o")
            || self.id.contains("sonnet")
    }

    /// Determines if the model accepts free-form tool / function-calling requests.
    pub fn is_tool_capable(&self) -> bool {
        self.capabilities.tool_use
            || self.capabilities.function_calling
            || self.has_badge("tools")
            || self.has_badge("function-calling")
    }

    /// Determines if the model runs locally (e.g. via Ollama).
    pub fn is_local(&self) -> bool {
        self.provider.eq_ignore_ascii_case("ollama") || self.has_badge("local")
    }

    /// Checks whether the model matches a free-text filter query.
    pub fn matches_query(&self, query: &str) -> bool {
        let q = query.trim().to_lowercase();
        if q.is_empty() {
            return true;
        }
        self.id.to_lowercase().contains(&q)
            || self.name.to_lowercase().contains(&q)
            || self.provider.to_lowercase().contains(&q)
            || self.badges.iter().any(|b| b.to_lowercase().contains(&q))
            || self
                .description
                .as_deref()
                .map(|d| d.to_lowercase().contains(&q))
                .unwrap_or(false)
    }
}

/// Source indicating how the model catalog was populated.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum CatalogSource {
    #[default]
    StaticFallback,
    DiskCache,
    LiveSync,
}

/// Disk cache envelope written to `~/.fusion/cache/models.json`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelCatalogCache {
    pub version: u32,
    pub updated_at: u64,
    pub ttl_secs: u64,
    pub models: Vec<CatalogModel>,
}

/// The aggregated model catalog with lookup and management capabilities.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ModelCatalog {
    pub models: Vec<CatalogModel>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_updated: Option<u64>,
    #[serde(default)]
    pub source: CatalogSource,
}

impl ModelCatalog {
    pub fn new(models: Vec<CatalogModel>, last_updated: Option<u64>, source: CatalogSource) -> Self {
        Self {
            models,
            last_updated,
            source,
        }
    }

    /// Creates a model catalog initialized with the built-in static models.
    pub fn static_catalog() -> Self {
        Self {
            models: static_model_list(),
            last_updated: None,
            source: CatalogSource::StaticFallback,
        }
    }

    /// Total number of models in the catalog.
    pub fn count(&self) -> usize {
        self.models.len()
    }

    /// Returns true if the catalog has no models.
    pub fn is_empty(&self) -> bool {
        self.models.is_empty()
    }

    /// Returns all models, optionally filtered by provider (case-insensitive).
    /// If provider is `None` or `"all"`, returns all models.
    pub fn get_models(&self, provider: Option<&str>) -> Vec<CatalogModel> {
        match provider {
            Some(p) if !p.is_empty() && !p.eq_ignore_ascii_case("all") => self
                .models
                .iter()
                .filter(|m| m.provider.eq_ignore_ascii_case(p))
                .cloned()
                .collect(),
            _ => self.models.clone(),
        }
    }

    /// Finds a model by exact ID or case-insensitive match.
    pub fn find_model(&self, id: &str) -> Option<&CatalogModel> {
        let lower = id.to_lowercase();
        self.models
            .iter()
            .find(|m| m.id.eq_ignore_ascii_case(&lower))
    }


    /// Finds a model using fuzzy resolution.
    ///
    /// Resolution order:
    /// 1. Exact / case-insensitive ID match
    /// 2. Canonical alias match (e.g. `"4o"` → `gpt-4o`, `"sonnet"` → Claude 3.7 Sonnet)
    /// 3. Case-insensitive display-name match
    /// 4. Unique case-insensitive ID prefix match
    /// 5. Unique case-insensitive ID suffix match
    /// 6. Unique last path-segment match (`claude-3.7-sonnet` → `anthropic/claude-3.7-sonnet`)
    /// 7. Unique case-insensitive ID substring match
    ///
    /// Ambiguous matches (more than one candidate at a stage) fall through to the
    /// next stage; if every stage is ambiguous, returns `None`.
    pub fn resolve_model(&self, query: &str) -> Option<&CatalogModel> {
        let q = query.trim();
        if q.is_empty() {
            return None;
        }

        // 1. Exact / case-insensitive ID
        if let Some(m) = self.find_model(q) {
            return Some(m);
        }

        let lower = q.to_lowercase();

        // 2. Canonical alias
        if let Some(canonical) = canonical_model_id(&lower) {
            if let Some(m) = self.find_model(canonical) {
                return Some(m);
            }
        }

        // 3. Display name
        if let Some(m) = self.models.iter().find(|m| m.name.to_lowercase() == lower) {
            return Some(m);
        }

        // 4. Unique prefix
        let prefix_matches: Vec<&CatalogModel> = self
            .models
            .iter()
            .filter(|m| m.id.to_lowercase().starts_with(&lower))
            .collect();
        if prefix_matches.len() == 1 {
            return Some(prefix_matches[0]);
        }

        // 5. Unique suffix
        let suffix_matches: Vec<&CatalogModel> = self
            .models
            .iter()
            .filter(|m| m.id.to_lowercase().ends_with(&lower))
            .collect();
        if suffix_matches.len() == 1 {
            return Some(suffix_matches[0]);
        }

        // 6. Unique last path segment (ignores vendor prefix)
        let segment_matches: Vec<&CatalogModel> = self
            .models
            .iter()
            .filter(|m| m.id.rsplit('/').next().unwrap_or(&m.id).to_lowercase() == lower)
            .collect();
        if segment_matches.len() == 1 {
            return Some(segment_matches[0]);
        }

        // 7. Unique substring
        let sub_matches: Vec<&CatalogModel> = self
            .models
            .iter()
            .filter(|m| m.id.to_lowercase().contains(&lower))
            .collect();
        if sub_matches.len() == 1 {
            return Some(sub_matches[0]);
        }

        None
    }
    /// Returns an alphabetically sorted list of unique provider names.
    pub fn providers(&self) -> Vec<String> {
        let mut provs = std::collections::BTreeSet::new();
        for m in &self.models {
            provs.insert(m.provider.to_lowercase());
        }
        provs.into_iter().collect()
    }

    /// Adds a model or updates an existing one matching the same ID and provider.
    pub fn add_or_update(&mut self, model: CatalogModel) {
        if let Some(pos) = self.models.iter().position(|m| {
            m.id.eq_ignore_ascii_case(&model.id)
                && m.provider.eq_ignore_ascii_case(&model.provider)
        }) {
            self.models[pos] = model;
        } else {
            self.models.push(model);
        }
    }

    /// Merges another catalog into this one.
    pub fn merge(&mut self, other: ModelCatalog) {
        for m in other.models {
            self.add_or_update(m);
        }
        if other.last_updated.is_some() {
            self.last_updated = other.last_updated;
        }
    }
}

// ============================================================================
// Cache Paths & File Operations
// ============================================================================

/// Returns current Unix timestamp in seconds.
pub fn current_timestamp_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

/// Returns the cache directory: `~/.fusion/cache`
pub fn cache_dir() -> PathBuf {
    Config::config_dir().join("cache")
}

/// Returns the cache file path: `~/.fusion/cache/models.json`
pub fn cache_path() -> PathBuf {
    cache_dir().join("models.json")
}

/// Returns true if the cache file exists, parses correctly, and is within the TTL window.
pub fn is_cache_fresh(path: &Path, ttl_secs: u64) -> bool {
    if !path.exists() {
        return false;
    }
    let data = match std::fs::read_to_string(path) {
        Ok(d) => d,
        Err(_) => return false,
    };
    let cache: ModelCatalogCache = match serde_json::from_str(&data) {
        Ok(c) => c,
        Err(_) => return false,
    };
    let now = current_timestamp_secs();
    now.saturating_sub(cache.updated_at) < ttl_secs
}

/// Loads the catalog from `~/.fusion/cache/models.json` if valid and unexpired.
pub fn load_cached_catalog() -> Option<ModelCatalog> {
    let path = cache_path();
    if !is_cache_fresh(&path, DEFAULT_CATALOG_TTL_SECS) {
        return None;
    }
    load_cache_file(&path, CatalogSource::DiskCache)
}

/// Loads the cached catalog even if the TTL has expired (offline fallback).
pub fn load_stale_cached_catalog() -> Option<ModelCatalog> {
    let path = cache_path();
    if !path.exists() {
        return None;
    }
    load_cache_file(&path, CatalogSource::DiskCache)
}

fn load_cache_file(path: &Path, source: CatalogSource) -> Option<ModelCatalog> {
    let data = std::fs::read_to_string(path).ok()?;
    let cache: ModelCatalogCache = serde_json::from_str(&data).ok()?;
    Some(ModelCatalog {
        models: cache.models,
        last_updated: Some(cache.updated_at),
        source,
    })
}

/// Saves the given catalog to `~/.fusion/cache/models.json`.
pub fn save_cached_catalog(catalog: &ModelCatalog) -> anyhow::Result<PathBuf> {
    let path = cache_path();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("Failed to create directory {:?}", parent))?;
    }
    let cache = ModelCatalogCache {
        version: CATALOG_CACHE_VERSION,
        updated_at: catalog.last_updated.unwrap_or_else(current_timestamp_secs),
        ttl_secs: DEFAULT_CATALOG_TTL_SECS,
        models: catalog.models.clone(),
    };
    let json_str = serde_json::to_string_pretty(&cache)
        .context("Failed to serialize model catalog cache to JSON")?;
    std::fs::write(&path, json_str)
        .with_context(|| format!("Failed to write cache file {:?}", path))?;
    Ok(path)
}

/// Deletes the cached catalog file.
pub fn clear_catalog_cache() -> std::io::Result<()> {
    let path = cache_path();
    if path.exists() {
        std::fs::remove_file(path)?;
    }
    Ok(())
}

// ============================================================================
// Dynamic Provider Fetching
// ============================================================================

/// Dynamic fetcher that communicates with remote provider endpoints.
pub struct CatalogFetcher {
    client: reqwest::Client,
}

impl Default for CatalogFetcher {
    fn default() -> Self {
        Self::new()
    }
}

fn resolve_api_key(passed_key: Option<&str>, env_var: &str) -> Option<String> {
    if let Some(k) = passed_key {
        let trimmed = k.trim();
        if !trimmed.is_empty() {
            return Some(trimmed.to_string());
        }
    }
    if let Ok(k) = std::env::var(env_var) {
        let trimmed = k.trim();
        if !trimmed.is_empty() {
            return Some(trimmed.to_string());
        }
    }
    None
}

impl CatalogFetcher {
    pub fn new() -> Self {
        let client = reqwest::Client::builder()
            .use_rustls_tls()
            .timeout(CATALOG_FETCH_TIMEOUT)
            .user_agent("Fusion-AI/0.3.0")
            .build()
            .unwrap_or_else(|_| reqwest::Client::new());
        Self { client }
    }

    /// Fetches models from all configured providers concurrently.
    ///
    /// The Fusion API (`https://api.fusioncode.app/v1`) is the primary provider;
    /// the Fusion key is always attempted, while other OpenAI-compatible vendors
    /// are queried only when their API keys are configured.
    pub async fn fetch_all(&self, config: &Config) -> ModelCatalog {
        // Fusion API is the primary (and only bundled) provider. Other
        // providers are opt-in via `/providers` login; their fetchers remain
        // below, commented out, and can be re-enabled per provider.
        let (fusion_key, fusion_url) = config.get_key_and_url("fusion");
        let fusion_fut = self.fetch_fusion(Some(&fusion_url), fusion_key.as_deref());
        let (fusion_res,) = tokio::join!(fusion_fut);

        let mut models = Vec::new();
        if let Ok(m) = fusion_res {
            models.extend(m);
        }

        ModelCatalog {
            models,
            last_updated: Some(current_timestamp_secs()),
            source: CatalogSource::LiveSync,
        }
    }

    // --- Optional providers (commented out; re-enable per provider when
    //     a user logs in via `/providers`). ---
    //
    // let (openai_key, openai_url) = config.get_key_and_url("openai");
    // let (deepseek_key, deepseek_url) = config.get_key_and_url("deepseek");
    // let (openrouter_key, openrouter_url) = config.get_key_and_url("openrouter");
    // let (xai_key, xai_url) = config.get_key_and_url("xai");
    // let (anthropic_key, anthropic_url) = config.get_key_and_url("anthropic");
    // let groq_key = std::env::var("GROQ_API_KEY").ok();
    // let groq_url = std::env::var("GROQ_BASE_URL")
    //     .unwrap_or_else(|_| DEFAULT_GROQ_BASE_URL.to_string());
    // let openai_fut = self.fetch_openai(Some(&openai_url), openai_key.as_deref());
    // let deepseek_fut = self.fetch_deepseek(Some(&deepseek_url), deepseek_key.as_deref());
    // let openrouter_fut = self.fetch_openrouter(Some(&openrouter_url), openrouter_key.as_deref());
    // let groq_fut = self.fetch_groq(Some(&groq_url), groq_key.as_deref());
    // let anthropic_fut = self.fetch_anthropic(Some(&anthropic_url), anthropic_key.as_deref());
    // let xai_fut = self.fetch_xai(Some(&xai_url), xai_key.as_deref());

    /// Fetches models from OpenAI `/v1/models`.
    pub async fn fetch_openai(
        &self,
        base_url: Option<&str>,
        api_key: Option<&str>,
    ) -> Result<Vec<CatalogModel>, String> {
        let key = match resolve_api_key(api_key, "OPENAI_API_KEY") {
            Some(k) => k,
            None => return Err("No OpenAI API key available".to_string()),
        };

        let base = base_url.unwrap_or(DEFAULT_OPENAI_BASE_URL).trim_end_matches('/');
        let url = if base.ends_with("/models") {
            base.to_string()
        } else {
            format!("{}/models", base)
        };

        let resp = self
            .client
            .get(&url)
            .bearer_auth(key)
            .send()
            .await
            .map_err(|e| format!("OpenAI request failed: {}", e))?;

        if !resp.status().is_success() {
            return Err(format!("OpenAI HTTP error: {}", resp.status()));
        }

        let body: serde_json::Value = resp
            .json()
            .await
            .map_err(|e| format!("OpenAI JSON decode error: {}", e))?;

        let mut list = Vec::new();
        if let Some(arr) = body.get("data").and_then(|d| d.as_array()) {
            for item in arr {
                if let Some(id) = item.get("id").and_then(|i| i.as_str()) {
                    // Filter out non-chat / non-reasoning assets (embeddings, audio, moderation, older legacy)
                    if id.contains("whisper")
                        || id.contains("tts")
                        || id.contains("dall-e")
                        || id.contains("embedding")
                        || id.contains("moderation")
                        || id.contains("babbage")
                        || id.contains("davinci")
                        || id.starts_with("text-")
                        || id.starts_with("canary")
                    {
                        continue;
                    }

                    let mut model = CatalogModel::new(id, format_display_name(id), "openai");
                    enrich_model_metadata(&mut model);
                    list.push(model);
                }
            }
        }

        Ok(list)
    }

    /// Fetches models from DeepSeek `/models`.
    pub async fn fetch_deepseek(
        &self,
        base_url: Option<&str>,
        api_key: Option<&str>,
    ) -> Result<Vec<CatalogModel>, String> {
        let key = match resolve_api_key(api_key, "DEEPSEEK_API_KEY") {
            Some(k) => k,
            None => return Err("No DeepSeek API key available".to_string()),
        };

        let base = base_url.unwrap_or(DEFAULT_DEEPSEEK_BASE_URL).trim_end_matches('/');
        let url = if base.ends_with("/models") {
            base.to_string()
        } else if base.ends_with("/v1") {
            format!("{}/models", base)
        } else {
            format!("{}/models", base)
        };

        let resp = self
            .client
            .get(&url)
            .bearer_auth(key)
            .send()
            .await
            .map_err(|e| format!("DeepSeek request failed: {}", e))?;

        if !resp.status().is_success() {
            return Err(format!("DeepSeek HTTP error: {}", resp.status()));
        }

        let body: serde_json::Value = resp
            .json()
            .await
            .map_err(|e| format!("DeepSeek JSON decode error: {}", e))?;

        let mut list = Vec::new();
        if let Some(arr) = body.get("data").and_then(|d| d.as_array()) {
            for item in arr {
                if let Some(id) = item.get("id").and_then(|i| i.as_str()) {
                    let mut model = CatalogModel::new(id, format_display_name(id), "deepseek");
                    enrich_model_metadata(&mut model);
                    list.push(model);
                }
            }
        }

        Ok(list)
    }

    /// Fetches models from OpenRouter `/v1/models` (which provides rich metadata).
    pub async fn fetch_openrouter(
        &self,
        base_url: Option<&str>,
        api_key: Option<&str>,
    ) -> Result<Vec<CatalogModel>, String> {
        let base = base_url.unwrap_or(DEFAULT_OPENROUTER_BASE_URL).trim_end_matches('/');
        let url = if base.ends_with("/models") {
            base.to_string()
        } else {
            format!("{}/models", base)
        };

        let mut req = self.client.get(&url);
        if let Some(k) = resolve_api_key(api_key, "OPENROUTER_API_KEY") {
            req = req.bearer_auth(k);
        }

        let resp = req
            .send()
            .await
            .map_err(|e| format!("OpenRouter request failed: {}", e))?;

        if !resp.status().is_success() {
            return Err(format!("OpenRouter HTTP error: {}", resp.status()));
        }

        let body: serde_json::Value = resp
            .json()
            .await
            .map_err(|e| format!("OpenRouter JSON decode error: {}", e))?;

        let mut list = Vec::new();
        if let Some(arr) = body.get("data").and_then(|d| d.as_array()) {
            for item in arr {
                if let Some(id) = item.get("id").and_then(|i| i.as_str()) {
                    let display_name = item
                        .get("name")
                        .and_then(|n| n.as_str())
                        .unwrap_or(id);
                    let description = item.get("description").and_then(|d| d.as_str());
                    let context_length = item.get("context_length").and_then(|c| c.as_u64());
                    let max_completion = item
                        .get("top_provider")
                        .and_then(|tp| tp.get("max_completion_tokens"))
                        .and_then(|m| m.as_u64());

                    let mut model = CatalogModel::new(id, display_name, "openrouter");
                    if let Some(ctx) = context_length {
                        model.context_window = Some(ctx);
                    }
                    if let Some(out) = max_completion {
                        model.max_output_tokens = Some(out);
                    }
                    if let Some(d) = description {
                        model.description = Some(d.to_string());
                    }

                    enrich_model_metadata(&mut model);
                    list.push(model);
                }
            }
        }

        Ok(list)
    }

    /// Fetches models from Groq `/v1/models`.
    pub async fn fetch_groq(
        &self,
        base_url: Option<&str>,
        api_key: Option<&str>,
    ) -> Result<Vec<CatalogModel>, String> {
        let key = match resolve_api_key(api_key, "GROQ_API_KEY") {
            Some(k) => k,
            None => return Err("No Groq API key available".to_string()),
        };

        let base = base_url.unwrap_or(DEFAULT_GROQ_BASE_URL).trim_end_matches('/');
        let url = if base.ends_with("/models") {
            base.to_string()
        } else {
            format!("{}/models", base)
        };

        let resp = self
            .client
            .get(&url)
            .bearer_auth(key)
            .send()
            .await
            .map_err(|e| format!("Groq request failed: {}", e))?;

        if !resp.status().is_success() {
            return Err(format!("Groq HTTP error: {}", resp.status()));
        }

        let body: serde_json::Value = resp
            .json()
            .await
            .map_err(|e| format!("Groq JSON decode error: {}", e))?;

        let mut list = Vec::new();
        if let Some(arr) = body.get("data").and_then(|d| d.as_array()) {
            for item in arr {
                if let Some(id) = item.get("id").and_then(|i| i.as_str()) {
                    if let Some(active) = item.get("active").and_then(|a| a.as_bool()) {
                        if !active {
                            continue;
                        }
                    }
                    if id.contains("whisper") || id.contains("distil-whisper") {
                        continue;
                    }

                    let context = item.get("context_window").and_then(|c| c.as_u64());
                    let mut model = CatalogModel::new(id, format_display_name(id), "groq");
                    if let Some(ctx) = context {
                        model.context_window = Some(ctx);
                    }
                    model.badges.push("Ultra-Fast".to_string());
                    enrich_model_metadata(&mut model);
                    list.push(model);
                }
            }
        }

        Ok(list)
    }

    /// Fetches models from the Fusion API `/v1/models` (OpenAI-compatible).
    ///
    /// The Fusion API is the project's own provider endpoint. The key is resolved
    /// from the passed value or `FUSION_API_KEY`; the base URL defaults to
    /// `https://api.fusioncode.app/v1`.
    pub async fn fetch_fusion(
        &self,
        base_url: Option<&str>,
        _api_key: Option<&str>,
    ) -> Result<Vec<CatalogModel>, String> {
        let base = base_url
            .unwrap_or(DEFAULT_FUSION_BASE_URL)
            .trim_end_matches('/');
        let url = if base.ends_with("/models") {
            base.to_string()
        } else if base.ends_with("/v1") {
            format!("{}/models", base)
        } else {
            format!("{}/v1/models", base)
        };

        // The Fusion API is keyless but Cloudflare's edge blocks clients
        // without a browser User-Agent (error 1010). Send a real UA.
        let resp = self
            .client
            .get(&url)
            .header(
                reqwest::header::USER_AGENT,
                "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/125.0.0.0 Safari/537.36",
            )
            .header(reqwest::header::ACCEPT, "application/json")
            .send()
            .await
            .map_err(|e| format!("Fusion API request failed: {}", e))?;

        if !resp.status().is_success() {
            return Err(format!("Fusion API HTTP error: {}", resp.status()));
        }

        let body: serde_json::Value = resp
            .json()
            .await
            .map_err(|e| format!("Fusion API JSON decode error: {}", e))?;

        let mut list = Vec::new();
        if let Some(arr) = body.get("data").and_then(|d| d.as_array()) {
            for item in arr {
                if let Some(id) = item.get("id").and_then(|i| i.as_str()) {
                    let display_name = item
                        .get("display_name")
                        .or_else(|| item.get("name"))
                        .and_then(|n| n.as_str())
                        .unwrap_or(id);
                    let mut model = CatalogModel::new(id, display_name, "fusion");
                    // Surface context length + pricing from the Fusion payload.
                    if let Some(ctx) = item.get("context_length").and_then(|c| c.as_u64()) {
                        model.context_window = Some(ctx);
                    }
                    if let Some(pr) = item.get("pricing").and_then(|p| p.as_object()) {
                        model.input_cost_per_m = pr.get("input").and_then(|v| v.as_f64());
                        model.output_cost_per_m = pr.get("output").and_then(|v| v.as_f64());
                    }
                    enrich_model_metadata(&mut model);
                    list.push(model);
                }
            }
        }

        Ok(list)
    }

    /// Fetches models from Anthropic `/v1/models`.
    pub async fn fetch_anthropic(
        &self,
        base_url: Option<&str>,
        api_key: Option<&str>,
    ) -> Result<Vec<CatalogModel>, String> {
        let key = match resolve_api_key(api_key, "ANTHROPIC_API_KEY") {
            Some(k) => k,
            None => return Err("No Anthropic API key available".to_string()),
        };

        let base = base_url.unwrap_or(DEFAULT_ANTHROPIC_BASE_URL).trim_end_matches('/');
        let url = if base.ends_with("/models") {
            base.to_string()
        } else {
            format!("{}/models", base)
        };

        let resp = self
            .client
            .get(&url)
            .header("x-api-key", &key)
            .header("anthropic-version", "2023-06-01")
            .send()
            .await
            .map_err(|e| format!("Anthropic request failed: {}", e))?;

        if !resp.status().is_success() {
            return Err(format!("Anthropic HTTP error: {}", resp.status()));
        }

        let body: serde_json::Value = resp
            .json()
            .await
            .map_err(|e| format!("Anthropic JSON decode error: {}", e))?;

        let mut list = Vec::new();
        if let Some(arr) = body.get("data").and_then(|d| d.as_array()) {
            for item in arr {
                if let Some(id) = item.get("id").and_then(|i| i.as_str()) {
                    let display_name = item
                        .get("display_name")
                        .and_then(|d| d.as_str())
                        .unwrap_or(id);
                    let mut model = CatalogModel::new(id, display_name, "anthropic");
                    enrich_model_metadata(&mut model);
                    list.push(model);
                }
            }
        }

        Ok(list)
    }

    /// Fetches models from xAI `/v1/models`.
    pub async fn fetch_xai(
        &self,
        base_url: Option<&str>,
        api_key: Option<&str>,
    ) -> Result<Vec<CatalogModel>, String> {
        let key = match resolve_api_key(api_key, "XAI_API_KEY") {
            Some(k) => k,
            None => return Err("No xAI API key available".to_string()),
        };

        let base = base_url.unwrap_or(DEFAULT_XAI_BASE_URL).trim_end_matches('/');
        let url = if base.ends_with("/models") {
            base.to_string()
        } else {
            format!("{}/models", base)
        };

        let resp = self
            .client
            .get(&url)
            .bearer_auth(key)
            .send()
            .await
            .map_err(|e| format!("xAI request failed: {}", e))?;

        if !resp.status().is_success() {
            return Err(format!("xAI HTTP error: {}", resp.status()));
        }

        let body: serde_json::Value = resp
            .json()
            .await
            .map_err(|e| format!("xAI JSON decode error: {}", e))?;

        let mut list = Vec::new();
        if let Some(arr) = body.get("data").and_then(|d| d.as_array()) {
            for item in arr {
                if let Some(id) = item.get("id").and_then(|i| i.as_str()) {
                    let mut model = CatalogModel::new(id, format_display_name(id), "xai");
                    enrich_model_metadata(&mut model);
                    list.push(model);
                }
            }
        }

        Ok(list)
    }
}

// ============================================================================

/// Resolves a free-form model-name query to its canonical catalog ID.
///
/// Handles shorthand aliases, version-less names, and case-insensitive input.
/// Returns `None` when the query matches no known alias.
pub fn canonical_model_id(query: &str) -> Option<&'static str> {
    let q = query.trim().to_lowercase();
    if q.is_empty() {
        return None;
    }
    const ALIASES: &[(&str, &str)] = &[
        // OpenAI
        ("4o", "gpt-4o"),
        ("gpt4o", "gpt-4o"),
        ("4o-mini", "gpt-4o-mini"),
        ("4omini", "gpt-4o-mini"),
        ("4-turbo", "gpt-4-turbo"),
        ("turbo", "gpt-4-turbo"),
        ("o1", "o1"),
        ("o1-mini", "o1-mini"),
        ("o3", "o3-mini"),
        ("o3-mini", "o3-mini"),
        // Anthropic
        ("sonnet", "claude-3-7-sonnet-20250219"),
        ("sonnet-3.7", "claude-3-7-sonnet-20250219"),
        ("sonnet-3.5", "claude-3-5-sonnet-20241022"),
        ("haiku", "claude-3-5-haiku-20241022"),
        ("opus", "claude-3-opus-20240229"),
        ("claude", "claude-3-7-sonnet-20250219"),
        ("claude-3.7", "claude-3-7-sonnet-20250219"),
        ("claude-3.7-sonnet", "claude-3-7-sonnet-20250219"),
        ("claude-3.5-sonnet", "claude-3-5-sonnet-20241022"),
        ("claude-3.5-haiku", "claude-3-5-haiku-20241022"),
        ("claude-3-opus", "claude-3-opus-20240229"),
        ("claude-sonnet", "claude-3-7-sonnet-20250219"),
        ("claude-haiku", "claude-3-5-haiku-20241022"),
        ("claude-opus", "claude-3-opus-20240229"),
        // DeepSeek
        ("deepseek", "deepseek-chat"),
        ("deepseek-v3", "deepseek-chat"),
        ("v3", "deepseek-chat"),
        ("deepseek-r1", "deepseek-reasoner"),
        ("r1", "deepseek-reasoner"),
        ("reasoner", "deepseek-reasoner"),
        // xAI
        ("grok", "grok-2-latest"),
        ("grok-2", "grok-2-latest"),
        ("grok-2-vision", "grok-2-vision-1212"),
        // Groq
        ("llama", "llama-3.3-70b-versatile"),
        ("llama-3.3", "llama-3.3-70b-versatile"),
        ("llama-3.1-8b", "llama-3.1-8b-instant"),
        ("8b-instant", "llama-3.1-8b-instant"),
        ("70b", "llama-3.3-70b-versatile"),
        // Fusion API (project's own provider; models follow vendor-prefixed IDs)
        ("fusion", "fusion-default"),
        ("fusion-chat", "fusion-default"),
        ("fusion-reasoner", "fusion-reasoner"),
    ];
    ALIASES
        .iter()
        .find(|(alias, _)| *alias == q)
        .map(|(_, canonical)| *canonical)
}

// High-Level Synchronization & Discovery
// ============================================================================

/// Synchronizes the model catalog.
///
/// If `force` is false and the disk cache (`~/.fusion/cache/models.json`) is younger than 24 hours,
/// the cached catalog is returned immediately.
/// Otherwise, queries configured providers dynamically, updates the cache, and falls back to
/// stale cache or the embedded static model list when offline.
pub async fn sync_catalog(config: &Config, force: bool) -> ModelCatalog {
    let path = cache_path();
    if !force && is_cache_fresh(&path, DEFAULT_CATALOG_TTL_SECS) {
        if let Some(cached) = load_cached_catalog() {
            debug!("Using fresh cached model catalog ({} models)", cached.count());
            return cached;
        }
    }

    info!("Synchronizing model catalog from configured providers...");
    let fetcher = CatalogFetcher::new();
    let fetched = fetcher.fetch_all(config).await;

    if !fetched.is_empty() {
        let mut full_catalog = ModelCatalog::static_catalog();
        full_catalog.merge(fetched);
        full_catalog.last_updated = Some(current_timestamp_secs());
        full_catalog.source = CatalogSource::LiveSync;

        if let Err(e) = save_cached_catalog(&full_catalog) {
            warn!("Failed to save model catalog cache to {:?}: {}", path, e);
        } else {
            debug!("Saved updated model catalog cache to {:?}", path);
        }
        full_catalog
    } else {
        warn!("Provider model fetch returned 0 models (offline or keys missing); using cache or static fallback");
        if let Some(stale) = load_stale_cached_catalog() {
            stale
        } else {
            ModelCatalog::static_catalog()
        }
    }
}

/// Convenience synchronous lookup for the active catalog.
/// Checks the 24-hour disk cache first, falling back to stale cache or the embedded static catalog.
pub fn get_catalog() -> ModelCatalog {
    if let Some(cached) = load_cached_catalog() {
        cached
    } else if let Some(stale) = load_stale_cached_catalog() {
        stale
    } else {
        ModelCatalog::static_catalog()
    }
}

/// Convenience function returning models, optionally filtered by provider.
pub fn get_models(provider: Option<&str>) -> Vec<CatalogModel> {
    get_catalog().get_models(provider)
}

/// Forces a fresh catalog synchronization and updates the cache.
pub async fn refresh_catalog(config: &Config) -> ModelCatalog {
    sync_catalog(config, true).await
}

// ============================================================================
// Metadata Enrichment & Formatting Helpers
// ============================================================================

/// Automatically enriches model metadata (context window, max output tokens, badges)
/// based on established architecture knowledge and model ID patterns.
pub fn enrich_model_metadata(model: &mut CatalogModel) {
    let lower = model.id.to_lowercase();

    // Deduplicate existing badges helper
    let add_badge_if_missing = |m: &mut CatalogModel, b: &str| {
        if !m.has_badge(b) {
            m.badges.push(b.to_string());
        }
    };

    // Context windows & Max output tokens inference
    if model.context_window.is_none() {
        if lower.contains("claude-3-7") || lower.contains("claude-3.7") {
            model.context_window = Some(200_000);
            if model.max_output_tokens.is_none() {
                model.max_output_tokens = Some(64_000);
            }
        } else if lower.contains("claude-3-5") || lower.contains("claude-3.5") {
            model.context_window = Some(200_000);
            if model.max_output_tokens.is_none() {
                model.max_output_tokens = Some(8_192);
            }
        } else if lower.contains("claude-3-opus") {
            model.context_window = Some(200_000);
            if model.max_output_tokens.is_none() {
                model.max_output_tokens = Some(4_096);
            }
        } else if lower.contains("o1-mini") {
            model.context_window = Some(128_000);
            if model.max_output_tokens.is_none() {
                model.max_output_tokens = Some(65_536);
            }
        } else if lower.contains("o1") || lower.contains("o3-mini") || lower.contains("o3") {
            model.context_window = Some(200_000);
            if model.max_output_tokens.is_none() {
                model.max_output_tokens = Some(100_000);
            }
        } else if lower.contains("gpt-4o") {
            model.context_window = Some(128_000);
            if model.max_output_tokens.is_none() {
                model.max_output_tokens = Some(16_384);
            }
        } else if lower.contains("gpt-4-turbo") || lower.contains("4-turbo") {
            model.context_window = Some(128_000);
            if model.max_output_tokens.is_none() {
                model.max_output_tokens = Some(4_096);
            }
        } else if lower.contains("gpt-4") {
            model.context_window = Some(8_192);
            if model.max_output_tokens.is_none() {
                model.max_output_tokens = Some(4_096);
            }
        } else if lower.contains("gpt-3.5") {
            model.context_window = Some(16_385);
            if model.max_output_tokens.is_none() {
                model.max_output_tokens = Some(4_096);
            }
        } else if lower.contains("deepseek") {
            model.context_window = Some(64_000);
            if model.max_output_tokens.is_none() {
                model.max_output_tokens = Some(8_192);
            }
        } else if lower.contains("llama-3.3") || lower.contains("llama3.3") {
            model.context_window = Some(131_072);
            if model.max_output_tokens.is_none() {
                model.max_output_tokens = Some(8_192);
            }
        } else if lower.contains("llama-3.1") || lower.contains("llama3.1") {
            model.context_window = Some(131_072);
            if model.max_output_tokens.is_none() {
                model.max_output_tokens = Some(8_192);
            }
        } else if lower.contains("llama-3") || lower.contains("llama3") {
            model.context_window = Some(8_192);
            if model.max_output_tokens.is_none() {
                model.max_output_tokens = Some(4_096);
            }
        } else if lower.contains("qwen2.5") {
            model.context_window = Some(32_768);
            if model.max_output_tokens.is_none() {
                model.max_output_tokens = Some(8_192);
            }
        } else if lower.contains("mistral") || lower.contains("mixtral") {
            model.context_window = Some(32_768);
            if model.max_output_tokens.is_none() {
                model.max_output_tokens = Some(8_192);
            }
        } else if lower.contains("grok") {
            model.context_window = Some(131_072);
            if model.max_output_tokens.is_none() {
                model.max_output_tokens = Some(8_192);
            }
        }
    }

    // Pricing (USD per 1M tokens)
    if model.input_cost_per_m.is_none() || model.output_cost_per_m.is_none() {
        let (input_per_m, output_per_m): (f64, f64) = if lower.contains("claude-3-7-sonnet")
            || lower.contains("claude-3.7-sonnet")
        {
            (3.0, 15.0)
        } else if lower.contains("claude-3-5-sonnet") || lower.contains("claude-3.5-sonnet") {
            (3.0, 15.0)
        } else if lower.contains("claude-3-5-haiku") || lower.contains("claude-3.5-haiku") {
            (0.8, 4.0)
        } else if lower.contains("claude-3-opus") {
            (15.0, 75.0)
        } else if lower == "gpt-4o" {
            (2.5, 10.0)
        } else if lower == "gpt-4o-mini" {
            (0.15, 0.6)
        } else if lower == "gpt-4-turbo" {
            (10.0, 30.0)
        } else if lower == "o1" {
            (15.0, 60.0)
        } else if lower == "o1-mini" {
            (1.1, 4.4)
        } else if lower == "o3-mini" {
            (1.1, 4.4)
        } else if lower.contains("gpt-4") {
            (30.0, 60.0)
        } else if lower.contains("gpt-3.5") {
            (0.5, 1.5)
        } else if lower.contains("deepseek") || lower.contains("r1") {
            (0.27, 1.1)
        } else if lower.contains("grok") {
            (2.0, 10.0)
        } else {
            (0.0, 0.0)
        };
        let known = input_per_m > 0.0 || output_per_m > 0.0;
        if known {
            if model.input_cost_per_m.is_none() {
                model.input_cost_per_m = Some(input_per_m);
            }
            if model.output_cost_per_m.is_none() {
                model.output_cost_per_m = Some(output_per_m);
            }
        }
    }

    // Knowledge cutoff dates
    if model.knowledge_cutoff.is_none() {
        model.knowledge_cutoff = if lower.contains("claude-3-7") || lower.contains("claude-3.7") {
            Some("2025-02".to_string())
        } else if lower.contains("claude-3-5") || lower.contains("claude-3.5") {
            Some("2024-04".to_string())
        } else if lower.contains("claude-3-opus") {
            Some("2023-08".to_string())
        } else if lower == "gpt-4o" || lower == "gpt-4o-mini" {
            Some("2023-10".to_string())
        } else if lower == "o1" || lower == "o1-mini" {
            Some("2023-10".to_string())
        } else if lower == "o3-mini" {
            Some("2023-10".to_string())
        } else if lower.contains("gpt-4") {
            Some("2023-04".to_string())
        } else if lower.contains("gpt-3.5") {
            Some("2021-09".to_string())
        } else if lower.contains("deepseek") {
            Some("2024-07".to_string())
        } else if lower.contains("grok") {
            Some("2024-04".to_string())
        } else if lower.contains("llama-3.3") || lower.contains("llama3.3") {
            Some("2023-12".to_string())
        } else if lower.contains("llama-3.1") || lower.contains("llama3.1") {
            Some("2023-03".to_string())
        } else if lower.contains("qwen2.5") {
            Some("2024-06".to_string())
        } else {
            None
        };
    }

    // Capability flags
    if lower.contains("r1")
        || lower.contains("reasoner")
        || lower.contains("o1")
        || lower.contains("o3")
        || lower.contains("thinking")
        || lower.contains("claude-3-7")
        || lower.contains("claude-3.7")
        || lower.contains("minimax")
        || lower.contains("kimi")
    {
        model.capabilities.reasoning = true;
        add_badge_if_missing(model, "Reasoning");
    }
    if lower.contains("vision")
        || lower.contains("4o")
        || lower.contains("sonnet")
        || lower.contains("opus")
    {
        model.capabilities.vision = true;
    }
    // Most hosted chat models support tools; hosted majors get strict function-calling too.
    let major_vendor = matches!(
        model.provider.to_lowercase().as_str(),
        "openai" | "anthropic" | "deepseek" | "xai" | "openrouter"
    );
    if major_vendor {
        model.capabilities.tool_use = true;
        model.capabilities.function_calling = true;
    }

    // Badge classification heuristics
    if lower.contains("vision") || lower.contains("4o") || lower.contains("sonnet") || lower.contains("opus") {
        add_badge_if_missing(model, "Vision");
    }

    if lower.contains("mini")
        || lower.contains("haiku")
        || lower.contains("flash")
        || lower.contains("instant")
        || lower.contains("turbo")
    {
        add_badge_if_missing(model, "Fast");
    }

    if lower.contains("coder") || lower.contains("code") {
        add_badge_if_missing(model, "Coding");
    }

    if lower.contains("llama") || lower.contains("qwen") || lower.contains("mistral") || lower.contains("deepseek") {
        if model.provider != "openai" && model.provider != "anthropic" {
            add_badge_if_missing(model, "Open-Weights");
        }
    }

    if lower == "deepseek-chat" || lower == "gpt-4o" || lower.starts_with("claude-3-7-sonnet") {
        add_badge_if_missing(model, "Default");
    }

    if lower.contains("claude-3-7") || lower.contains("o3") {
        add_badge_if_missing(model, "Latest");
    }
}

/// Formats a raw model ID into a presentable title.
fn format_display_name(id: &str) -> String {
    let clean = id.trim();
    match clean {
        "deepseek-chat" => "DeepSeek V3".to_string(),
        "deepseek-reasoner" => "DeepSeek R1".to_string(),
        "gpt-4o" => "GPT-4o".to_string(),
        "gpt-4o-mini" => "GPT-4o Mini".to_string(),
        "o1" => "OpenAI o1".to_string(),
        "o1-mini" => "OpenAI o1-mini".to_string(),
        "o3-mini" => "OpenAI o3-mini".to_string(),
        "gpt-4-turbo" => "GPT-4 Turbo".to_string(),
        "claude-3-7-sonnet-20250219" | "claude-3-7-sonnet-latest" => "Claude 3.7 Sonnet".to_string(),
        "claude-3-5-sonnet-20241022" | "claude-3-5-sonnet-latest" => "Claude 3.5 Sonnet".to_string(),
        "claude-3-5-haiku-20241022" | "claude-3-5-haiku-latest" => "Claude 3.5 Haiku".to_string(),
        "claude-3-opus-20240229" | "claude-3-opus-latest" => "Claude 3 Opus".to_string(),
        "grok-2-latest" | "grok-2" => "Grok 2".to_string(),
        "grok-2-vision-1212" => "Grok 2 Vision".to_string(),
        "llama-3.3-70b-versatile" => "Llama 3.3 70B".to_string(),
        "llama-3.1-8b-instant" => "Llama 3.1 8B Instant".to_string(),
        _ => {
            // Remove common prefixes/tags
            let stripped = clean.split('/').last().unwrap_or(clean);
            let parts: Vec<String> = stripped
                .split(|c| c == '-' || c == '_' || c == ':')
                .map(|s| {
                    if s.eq_ignore_ascii_case("gpt") {
                        "GPT".to_string()
                    } else if s.eq_ignore_ascii_case("v3") || s.eq_ignore_ascii_case("v2") {
                        s.to_uppercase()
                    } else if s.len() <= 2 {
                        s.to_uppercase()
                    } else {
                        let mut chars = s.chars();
                        match chars.next() {
                            Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
                            None => String::new(),
                        }
                    }
                })
                .collect();
            parts.join(" ")
        }
    }
}

// ============================================================================
// Embedded Static Model List (Zero-latency & Offline Fallback)
// ============================================================================

/// Returns the comprehensive built-in static catalog.
pub fn static_model_list() -> Vec<CatalogModel> {
    vec![
        // Fusion
        CatalogModel::new("deepseek-ai/DeepSeek-V4-Flash-0731", "DeepSeek V4 Flash", "fusion")
            .with_context(1_048_576)
            .with_max_output(8_192)
            .with_badges(["Fast", "Default"])
            .with_description("Fusion gateway high-speed 1M context flash model"),

        CatalogModel::new("MiniMaxAI/MiniMax-M2.7", "MiniMax M2.7", "fusion")
            .with_context(204_800)
            .with_max_output(8_192)
            .with_badges(["Reasoning"])
            .with_description("MiniMax M2.7 frontier coding and reasoning model"),

        CatalogModel::new("moonshotai/Kimi-K2.6", "Kimi K2.6", "fusion")
            .with_context(204_800)
            .with_max_output(8_192)
            .with_badges(["Reasoning"])
            .with_description("Kimi K2.6 long-context and reasoning model"),
    ]
}

// ============================================================================
// Unit Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_static_model_list_coverage() {
        let catalog = ModelCatalog::static_catalog();
        assert!(!catalog.is_empty());
        let providers = catalog.providers();
        assert!(providers.contains(&"fusion".to_string()));
    }

    #[test]
    fn test_formatting_helpers() {
        let model = CatalogModel::new("test", "Test", "custom")
            .with_context(200_000)
            .with_max_output(64_000);
        assert_eq!(model.formatted_context_window(), "200K context");
        assert_eq!(model.formatted_max_output(), "64K output");

        let million = CatalogModel::new("test", "Test", "custom")
            .with_context(1_000_000)
            .with_max_output(128_000);
        assert_eq!(million.formatted_context_window(), "1M context");
        assert_eq!(million.formatted_max_output(), "128K output");

        let none = CatalogModel::new("test", "Test", "custom");
        assert_eq!(none.formatted_context_window(), "-");
        assert_eq!(none.formatted_max_output(), "-");
    }

    #[test]
    fn test_query_and_badge_matching() {
        let model = CatalogModel::new("deepseek-reasoner", "DeepSeek R1", "deepseek")
            .with_badges(["Reasoning", "Budget"])
            .with_description("Open reasoning engine");

        assert!(model.is_reasoning());
        assert!(!model.is_vision());
        assert!(!model.is_local());
        assert!(model.has_badge("reasoning"));
        assert!(model.has_badge("budget"));

        assert!(model.matches_query("deepseek"));
        assert!(model.matches_query("r1"));
        assert!(model.matches_query("reasoning"));
        assert!(model.matches_query("budget"));
        assert!(model.matches_query("engine"));
        assert!(!model.matches_query("nonexistent"));
    }

    #[test]
    fn test_provider_filtering() {
        let catalog = ModelCatalog::static_catalog();
        let fusion_models = catalog.get_models(Some("fusion"));
        assert!(!fusion_models.is_empty());
        for m in &fusion_models {
            assert_eq!(m.provider, "fusion");
        }

        let all_models = catalog.get_models(Some("all"));
        assert_eq!(all_models.len(), catalog.count());

        let none_models = catalog.get_models(None);
        assert_eq!(none_models.len(), catalog.count());
    }

    #[test]
    fn test_enrichment_heuristics() {
        let mut model = CatalogModel::new("claude-3-7-sonnet-20250219", "claude-3-7", "anthropic");
        enrich_model_metadata(&mut model);
        assert_eq!(model.context_window, Some(200_000));
        assert_eq!(model.max_output_tokens, Some(64_000));
        assert!(model.is_reasoning());
        assert!(model.is_vision());
        assert!(model.has_badge("Latest"));

        let mut o3 = CatalogModel::new("o3-mini", "o3-mini", "openai");
        enrich_model_metadata(&mut o3);
        assert_eq!(o3.context_window, Some(200_000));
        assert_eq!(o3.max_output_tokens, Some(100_000));
        assert!(o3.is_reasoning());
        assert!(o3.has_badge("Fast"));
    }

    #[test]
    fn test_cache_serialization_and_freshness() {
        let tmp_dir = std::env::temp_dir().join(format!("fusion_test_cat_{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&tmp_dir).unwrap();
        let cache_file = tmp_dir.join("models.json");

        let catalog = ModelCatalog::static_catalog();
        let cache = ModelCatalogCache {
            version: CATALOG_CACHE_VERSION,
            updated_at: current_timestamp_secs(),
            ttl_secs: 86_400,
            models: catalog.models.clone(),
        };

        let json_str = serde_json::to_string_pretty(&cache).unwrap();
        std::fs::write(&cache_file, json_str).unwrap();

        assert!(is_cache_fresh(&cache_file, 86_400));
        // Stale if TTL is 0
        assert!(!is_cache_fresh(&cache_file, 0));

        let loaded = load_cache_file(&cache_file, CatalogSource::DiskCache).unwrap();
        assert_eq!(loaded.count(), catalog.count());
        assert_eq!(loaded.source, CatalogSource::DiskCache);

        let _ = std::fs::remove_dir_all(tmp_dir);
    }
}
