//! Dynamic model pricing synchronizer for OpenRouter and multi-provider pricing APIs.
//!
//! Provides:
//! 1. Dynamic OpenRouter pricing fetcher retrieving real-time prompt, completion,
//!    and prompt-cache costs across hundreds of models.
//! 2. Multi-tier prompt caching cost inference (Anthropic 10%/125%, OpenAI 50%/100%,
//!    DeepSeek ~10%/100%, and custom provider ratios).
//! 3. Resilient multi-tier persistence with local JSON disk cache (`~/.fusion/cache/pricing_cache.json`),
//!    configurable TTL, offline fallback to stale cache, and embedded static fallbacks.
//! 4. Thread-safe `PricingSynchronizer` with on-demand and periodic background synchronization.
//! 5. Pricing diff and analytics tracking (cost changes, price drops/hikes, cheapest models per workload).
//! 6. Seamless integration with [`ModelPricingRegistry`], [`CostTracker`], and [`CostBreakdown`].
//! 7. Rich CLI/TUI formatting (tables, comparison views, sync summaries, diff reports).

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use chrono::Utc;
use serde::{Deserialize, Serialize};
use thiserror::Error;
use tokio::sync::RwLock;

use crate::agent::cost::{CostBreakdown, CostTracker, ModelPricing, ModelPricingRegistry};
use crate::config::Config;

// ============================================================================
// Constants
// ============================================================================

/// Default OpenRouter public models endpoint.
pub const OPENROUTER_MODELS_ENDPOINT: &str = "https://openrouter.ai/api/v1/models";

/// Default time-to-live for the pricing cache (24 hours).
pub const DEFAULT_PRICING_CACHE_TTL_SECS: u64 = 86_400;

/// Default HTTP request timeout for pricing synchronization.
pub const DEFAULT_PRICING_SYNC_TIMEOUT_SECS: u64 = 10;

/// Version number for the pricing cache schema.
pub const PRICING_CACHE_VERSION: u32 = 1;

/// Default file name for the pricing disk cache.
pub const DEFAULT_PRICING_CACHE_FILENAME: &str = "pricing_cache.json";

/// OpenRouter HTTP referer header value.
pub const DEFAULT_REFERER_HEADER: &str = "https://github.com/theaungmyatmoe/fusion";

/// OpenRouter HTTP app title header value.
pub const DEFAULT_TITLE_HEADER: &str = "Fusion AI Assistant";

// ============================================================================
// Error Types
// ============================================================================

/// Errors that can occur during model pricing synchronization.
#[derive(Debug, Error)]
pub enum PricingSyncError {
    /// Network or HTTP transport error.
    #[error("Network error: {0}")]
    Network(String),

    /// HTTP server error response with status code.
    #[error("HTTP error {status}: {message}")]
    Http { status: u16, message: String },

    /// JSON serialization or deserialization failure.
    #[error("JSON decode error: {0}")]
    Json(String),

    /// Cache reading, writing, or validation error.
    #[error("Cache error: {0}")]
    Cache(String),

    /// Filesystem I/O error.
    #[error("I/O error: {0}")]
    Io(String),

    /// Requested model pricing not found.
    #[error("Model pricing not found for: {0}")]
    ModelNotFound(String),

    /// Synchronization task was cancelled.
    #[error("Synchronization was cancelled")]
    Cancelled,
}

// ============================================================================
// OpenRouter API Raw DTOs
// ============================================================================

/// Raw top-level response envelope from OpenRouter `/api/v1/models`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OpenRouterResponseRaw {
    pub data: Vec<OpenRouterModelPricingRaw>,
}

/// Raw model entry from OpenRouter `/api/v1/models`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OpenRouterModelPricingRaw {
    pub id: String,
    pub name: Option<String>,
    pub description: Option<String>,
    pub context_length: Option<u64>,
    pub pricing: Option<OpenRouterPricingRaw>,
    pub top_provider: Option<OpenRouterTopProviderRaw>,
    pub architecture: Option<OpenRouterArchRaw>,
    pub per_request_limits: Option<OpenRouterLimitsRaw>,
}

/// Raw pricing breakdown from OpenRouter.
///
/// Note: Rates are given in USD per single token as string (e.g. `"0.000003"` = $3/1M).
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct OpenRouterPricingRaw {
    pub prompt: Option<String>,
    pub completion: Option<String>,
    pub request: Option<String>,
    pub image: Option<String>,
    pub input_cache_read: Option<String>,
    pub input_cache_write: Option<String>,
}

/// Top provider limits from OpenRouter.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct OpenRouterTopProviderRaw {
    pub context_length: Option<u64>,
    pub max_completion_tokens: Option<u64>,
    pub is_moderated: Option<bool>,
}

/// Architecture metadata from OpenRouter.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct OpenRouterArchRaw {
    pub modality: Option<String>,
    pub tokenizer: Option<String>,
    pub instruct_type: Option<String>,
}

/// Per-request limits from OpenRouter.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct OpenRouterLimitsRaw {
    pub prompt_tokens: Option<u64>,
    pub completion_tokens: Option<u64>,
}

// ============================================================================
// Core Synchronized Pricing Types
// ============================================================================

/// Source indicating where a model's pricing data originated.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PricingSource {
    /// Freshly retrieved live from OpenRouter or upstream API.
    Live,
    /// Loaded from local disk cache.
    DiskCache,
    /// Loaded from stale disk cache after network failure.
    StaleDiskCache,
    /// Hardcoded built-in fallback.
    BuiltinFallback,
    /// User or runtime custom override.
    CustomOverride,
}

impl std::fmt::Display for PricingSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Live => write!(f, "Live API"),
            Self::DiskCache => write!(f, "Disk Cache"),
            Self::StaleDiskCache => write!(f, "Stale Disk Cache"),
            Self::BuiltinFallback => write!(f, "Builtin Fallback"),
            Self::CustomOverride => write!(f, "Custom Override"),
        }
    }
}

/// Synchronized, canonical pricing and capability record for a single model.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ModelPricingRecord {
    /// Full model ID (e.g. `"anthropic/claude-3.7-sonnet"` or `"deepseek/deepseek-r1"`).
    pub model_id: String,

    /// Canonical upstream provider (e.g. `"anthropic"`, `"openai"`, `"deepseek"`, `"meta-llama"`).
    pub canonical_provider: String,

    /// Canonical model name without vendor prefix (e.g. `"claude-3.7-sonnet"`).
    pub canonical_name: String,

    /// Human-readable model display name (e.g. `"Anthropic: Claude 3.7 Sonnet"`).
    pub display_name: String,

    /// Structured token pricing (rates in USD per 1M tokens).
    pub pricing: ModelPricing,

    /// Maximum context window in tokens.
    pub context_length: Option<u64>,

    /// Maximum output / completion tokens.
    pub max_completion_tokens: Option<u64>,

    /// Whether this model is free to use ($0.00 / 1M tokens).
    pub is_free: bool,

    /// Whether this model supports multimodal vision inputs.
    pub supports_vision: bool,

    /// Optional description or capability summary.
    pub description: Option<String>,

    /// Unix timestamp (in seconds) when this record was synchronized.
    pub last_updated_at: u64,

    /// Origin of this pricing record.
    pub source: PricingSource,
}

impl ModelPricingRecord {
    /// Returns the input price in USD per 1,000,000 tokens.
    pub fn input_price_per_million(&self) -> f64 {
        self.pricing.input_per_million
    }

    /// Returns the output price in USD per 1,000,000 tokens.
    pub fn output_price_per_million(&self) -> f64 {
        self.pricing.output_per_million
    }

    /// Returns the cache read price in USD per 1,000,000 tokens.
    pub fn cache_read_price_per_million(&self) -> f64 {
        self.pricing.cache_read_per_million
    }

    /// Returns the cache write price in USD per 1,000,000 tokens.
    pub fn cache_write_price_per_million(&self) -> f64 {
        self.pricing.cache_write_per_million
    }

    /// Calculate estimated cost for a given token usage breakdown.
    pub fn calculate_cost(
        &self,
        prompt_tokens: u64,
        completion_tokens: u64,
        cache_read_tokens: u64,
        cache_write_tokens: u64,
    ) -> CostBreakdown {
        self.pricing.calculate(
            prompt_tokens,
            completion_tokens,
            cache_read_tokens,
            cache_write_tokens,
        )
    }

    /// Calculates blended cost for a standard turn (e.g. 80% cache hit ratio on input).
    pub fn calculate_blended_cost(
        &self,
        prompt_tokens: u64,
        completion_tokens: u64,
        cache_hit_ratio: f64,
    ) -> CostBreakdown {
        let hit_ratio = cache_hit_ratio.clamp(0.0, 1.0);
        let cache_read = (prompt_tokens as f64 * hit_ratio).round() as u64;
        self.calculate_cost(prompt_tokens, completion_tokens, cache_read, 0)
    }
}

/// Cache file envelope saved to disk.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PricingCacheEnvelope {
    pub version: u32,
    pub timestamp: u64,
    pub source: String,
    pub model_count: usize,
    pub models: HashMap<String, ModelPricingRecord>,
}

/// Statistics and telemetry from a pricing synchronization run.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct PricingSyncStats {
    pub models_fetched: usize,
    pub models_updated: usize,
    pub models_added: usize,
    pub free_models_count: usize,
    pub sync_duration_ms: u64,
    pub source: Option<PricingSource>,
    pub timestamp: u64,
    pub errors: Vec<String>,
}

/// Represents a detected price change for a model between synchronization cycles.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PricingDiff {
    pub model_id: String,
    pub display_name: String,
    pub old_input_per_million: f64,
    pub new_input_per_million: f64,
    pub old_output_per_million: f64,
    pub new_output_per_million: f64,
    pub input_change_pct: f64,
    pub output_change_pct: f64,
}

impl PricingDiff {
    /// Creates a pricing diff comparing old and new records.
    pub fn compute(old: &ModelPricingRecord, new: &ModelPricingRecord) -> Option<Self> {
        let input_diff = new.pricing.input_per_million - old.pricing.input_per_million;
        let output_diff = new.pricing.output_per_million - old.pricing.output_per_million;

        if input_diff.abs() < 1e-6 && output_diff.abs() < 1e-6 {
            return None;
        }

        let input_change_pct = if old.pricing.input_per_million > 0.0 {
            (input_diff / old.pricing.input_per_million) * 100.0
        } else if new.pricing.input_per_million > 0.0 {
            100.0
        } else {
            0.0
        };

        let output_change_pct = if old.pricing.output_per_million > 0.0 {
            (output_diff / old.pricing.output_per_million) * 100.0
        } else if new.pricing.output_per_million > 0.0 {
            100.0
        } else {
            0.0
        };

        Some(Self {
            model_id: new.model_id.clone(),
            display_name: new.display_name.clone(),
            old_input_per_million: old.pricing.input_per_million,
            new_input_per_million: new.pricing.input_per_million,
            old_output_per_million: old.pricing.output_per_million,
            new_output_per_million: new.pricing.output_per_million,
            input_change_pct,
            output_change_pct,
        })
    }

    /// Whether this diff represents a price decrease (discount).
    pub fn is_price_drop(&self) -> bool {
        self.new_input_per_million < self.old_input_per_million
            || self.new_output_per_million < self.old_output_per_million
    }

    /// Whether this diff represents a price increase.
    pub fn is_price_increase(&self) -> bool {
        self.new_input_per_million > self.old_input_per_million
            || self.new_output_per_million > self.old_output_per_million
    }
}

// ============================================================================
// Configuration
// ============================================================================

/// Configuration options for the dynamic pricing synchronizer.
#[derive(Debug, Clone)]
pub struct PricingSyncConfig {
    /// Custom base URL or endpoint (defaults to OpenRouter models endpoint).
    pub endpoint_url: String,

    /// Optional API key for authenticated queries (increases rate limits).
    pub api_key: Option<String>,

    /// Custom cache directory (defaults to `~/.fusion/cache`).
    pub cache_dir: Option<PathBuf>,

    /// Time-to-live for cache in seconds (defaults to 24h = 86,400s).
    pub ttl_secs: u64,

    /// Network timeout for sync requests.
    pub timeout: Duration,

    /// Whether to automatically persist successful syncs to disk.
    pub auto_save_cache: bool,

    /// Whether to infer prompt-cache read and write discount tiers if omitted by API.
    pub infer_cache_tiers: bool,
}

impl Default for PricingSyncConfig {
    fn default() -> Self {
        Self {
            endpoint_url: OPENROUTER_MODELS_ENDPOINT.to_string(),
            api_key: None,
            cache_dir: None,
            ttl_secs: DEFAULT_PRICING_CACHE_TTL_SECS,
            timeout: Duration::from_secs(DEFAULT_PRICING_SYNC_TIMEOUT_SECS),
            auto_save_cache: true,
            infer_cache_tiers: true,
        }
    }
}

impl PricingSyncConfig {
    /// Resolves the API key from explicit config or `OPENROUTER_API_KEY` environment variable.
    pub fn resolve_api_key(&self) -> Option<String> {
        self.api_key
            .clone()
            .or_else(|| std::env::var("OPENROUTER_API_KEY").ok())
            .filter(|k| !k.trim().is_empty())
    }

    /// Returns the resolved cache file path.
    pub fn cache_file_path(&self) -> PathBuf {
        let dir = self
            .cache_dir
            .clone()
            .unwrap_or_else(|| Config::config_dir().join("cache"));
        dir.join(DEFAULT_PRICING_CACHE_FILENAME)
    }
}

// ============================================================================
// Parsing & Conversion Helpers
// ============================================================================

/// Converts an OpenRouter per-token string price into USD per 1,000,000 tokens.
///
/// Example: `"0.000003"` -> `3.00`
pub fn parse_openrouter_price_per_million(price_str: Option<&str>) -> f64 {
    price_str
        .and_then(|s| s.trim().parse::<f64>().ok())
        .map(|cost_per_token| cost_per_token * 1_000_000.0)
        .unwrap_or(0.0)
}

/// Infers cache read and cache write rates per 1M tokens based on provider heuristics.
///
/// - **Anthropic**: Cache read = 10% of base input, Cache write = 125% of base input
/// - **OpenAI**: Cache read = 50% of base input, Cache write = base input
/// - **DeepSeek**: Cache read = ~10% of base input, Cache write = base input
/// - **Default / Generic**: Cache read = 50% of base input, Cache write = base input
pub fn infer_cache_rates(provider: &str, model: &str, input_rate: f64) -> (f64, f64) {
    let prov = provider.to_lowercase();
    let mod_name = model.to_lowercase();

    if prov.contains("anthropic") || mod_name.contains("claude") {
        (input_rate * 0.10, input_rate * 1.25)
    } else if prov.contains("deepseek") || mod_name.contains("deepseek") {
        (input_rate * 0.10, input_rate)
    } else if prov.contains("openai") || mod_name.contains("gpt") || mod_name.contains("o1") || mod_name.contains("o3") {
        (input_rate * 0.50, input_rate)
    } else {
        (input_rate * 0.50, input_rate)
    }
}

/// Normalizes an OpenRouter model ID into `(canonical_provider, canonical_model_name)`.
///
/// E.g. `"anthropic/claude-3.7-sonnet"` -> `("anthropic", "claude-3.7-sonnet")`
pub fn extract_provider_and_model_name(openrouter_id: &str) -> (String, String) {
    if let Some((prov, name)) = openrouter_id.split_once('/') {
        (prov.trim().to_lowercase(), name.trim().to_string())
    } else {
        ("openrouter".to_string(), openrouter_id.trim().to_string())
    }
}

/// Parses an OpenRouter raw model object into a structured [`ModelPricingRecord`].
pub fn parse_openrouter_model_pricing(
    raw: &OpenRouterModelPricingRaw,
    infer_cache: bool,
    source: PricingSource,
) -> ModelPricingRecord {
    let (provider, model_name) = extract_provider_and_model_name(&raw.id);

    let input_rate = raw
        .pricing
        .as_ref()
        .map(|p| parse_openrouter_price_per_million(p.prompt.as_deref()))
        .unwrap_or(0.0);

    let output_rate = raw
        .pricing
        .as_ref()
        .map(|p| parse_openrouter_price_per_million(p.completion.as_deref()))
        .unwrap_or(0.0);

    // Cache read & write pricing
    let (cache_read, cache_write) = if let Some(pricing) = &raw.pricing {
        let explicit_read = pricing
            .input_cache_read
            .as_deref()
            .map(|s| parse_openrouter_price_per_million(Some(s)));
        let explicit_write = pricing
            .input_cache_write
            .as_deref()
            .map(|s| parse_openrouter_price_per_million(Some(s)));

        match (explicit_read, explicit_write) {
            (Some(r), Some(w)) => (r, w),
            (Some(r), None) => (r, input_rate),
            (None, Some(w)) => (
                if infer_cache {
                    infer_cache_rates(&provider, &model_name, input_rate).0
                } else {
                    input_rate
                },
                w,
            ),
            (None, None) => {
                if infer_cache && input_rate > 0.0 {
                    infer_cache_rates(&provider, &model_name, input_rate)
                } else {
                    (input_rate, input_rate)
                }
            }
        }
    } else if infer_cache && input_rate > 0.0 {
        infer_cache_rates(&provider, &model_name, input_rate)
    } else {
        (input_rate, input_rate)
    };

    let is_free = raw.id.ends_with(":free") || (input_rate == 0.0 && output_rate == 0.0);

    let supports_vision = raw
        .architecture
        .as_ref()
        .and_then(|a| a.modality.as_deref())
        .map(|m| m.contains("image"))
        .unwrap_or(false);

    let max_completion = raw
        .top_provider
        .as_ref()
        .and_then(|tp| tp.max_completion_tokens)
        .or_else(|| {
            raw.per_request_limits
                .as_ref()
                .and_then(|l| l.completion_tokens)
        });

    let display_name = raw
        .name
        .clone()
        .unwrap_or_else(|| format!("{}: {}", provider, model_name));

    let pricing = ModelPricing::new(
        &provider,
        &model_name,
        input_rate,
        output_rate,
        cache_read,
        cache_write,
    );

    ModelPricingRecord {
        model_id: raw.id.clone(),
        canonical_provider: provider,
        canonical_name: model_name,
        display_name,
        pricing,
        context_length: raw.context_length,
        max_completion_tokens: max_completion,
        is_free,
        supports_vision,
        description: raw.description.clone(),
        last_updated_at: Utc::now().timestamp() as u64,
        source,
    }
}

// ============================================================================
// Disk Cache Persistence
// ============================================================================

/// Returns true if the cache file exists, is valid, and is unexpired according to `ttl_secs`.
pub fn is_pricing_cache_fresh(path: &Path, ttl_secs: u64) -> bool {
    if !path.exists() {
        return false;
    }
    match std::fs::read_to_string(path) {
        Ok(contents) => match serde_json::from_str::<PricingCacheEnvelope>(&contents) {
            Ok(cache) => {
                let now = Utc::now().timestamp() as u64;
                cache.version == PRICING_CACHE_VERSION && now.saturating_sub(cache.timestamp) <= ttl_secs
            }
            Err(_) => false,
        },
        Err(_) => false,
    }
}

/// Loads unexpired pricing cache from disk.
pub fn load_pricing_cache(path: &Path, ttl_secs: u64) -> Option<PricingCacheEnvelope> {
    if !path.exists() {
        return None;
    }
    let contents = std::fs::read_to_string(path).ok()?;
    let cache: PricingCacheEnvelope = serde_json::from_str(&contents).ok()?;
    let now = Utc::now().timestamp() as u64;
    if cache.version == PRICING_CACHE_VERSION && now.saturating_sub(cache.timestamp) <= ttl_secs {
        Some(cache)
    } else {
        None
    }
}

/// Loads cached pricing from disk even if expired (offline fallback).
pub fn load_stale_pricing_cache(path: &Path) -> Option<PricingCacheEnvelope> {
    if !path.exists() {
        return None;
    }
    let contents = std::fs::read_to_string(path).ok()?;
    let mut cache: PricingCacheEnvelope = serde_json::from_str(&contents).ok()?;
    // Mark source as stale
    for record in cache.models.values_mut() {
        record.source = PricingSource::StaleDiskCache;
    }
    Some(cache)
}

/// Saves the pricing cache envelope to disk safely.
pub fn save_pricing_cache(path: &Path, cache: &PricingCacheEnvelope) -> Result<(), PricingSyncError> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| PricingSyncError::Io(format!("Failed to create cache directory: {}", e)))?;
    }
    let json_bytes = serde_json::to_vec_pretty(cache)
        .map_err(|e| PricingSyncError::Json(format!("Failed to serialize cache: {}", e)))?;
    std::fs::write(path, json_bytes)
        .map_err(|e| PricingSyncError::Io(format!("Failed to write cache file: {}", e)))?;
    Ok(())
}

/// Clears the pricing cache file.
pub fn clear_pricing_cache(custom_path: Option<&Path>) -> Result<(), PricingSyncError> {
    let path = match custom_path {
        Some(p) => p.to_path_buf(),
        None => Config::config_dir().join("cache").join(DEFAULT_PRICING_CACHE_FILENAME),
    };
    if path.exists() {
        std::fs::remove_file(&path)
            .map_err(|e| PricingSyncError::Io(format!("Failed to delete cache file: {}", e)))?;
    }
    Ok(())
}

// ============================================================================
// Remote HTTP Fetcher
// ============================================================================

/// Directly fetches latest pricing from OpenRouter API over HTTP.
pub async fn fetch_openrouter_raw(
    client: &reqwest::Client,
    endpoint_url: &str,
    api_key: Option<&str>,
    timeout: Duration,
) -> Result<Vec<OpenRouterModelPricingRaw>, PricingSyncError> {
    let mut req = client
        .get(endpoint_url)
        .timeout(timeout)
        .header("HTTP-Referer", DEFAULT_REFERER_HEADER)
        .header("X-Title", DEFAULT_TITLE_HEADER);

    if let Some(key) = api_key.filter(|k| !k.is_empty()) {
        req = req.bearer_auth(key);
    }

    let resp = req.send().await.map_err(|e| {
        PricingSyncError::Network(format!("Failed to connect to OpenRouter pricing endpoint: {}", e))
    })?;

    let status = resp.status();
    if !status.is_success() {
        let msg = resp
            .text()
            .await
            .unwrap_or_else(|_| "Unknown error".to_string());
        return Err(PricingSyncError::Http {
            status: status.as_u16(),
            message: msg,
        });
    }

    let body: OpenRouterResponseRaw = resp.json().await.map_err(|e| {
        PricingSyncError::Json(format!("Failed to decode OpenRouter pricing JSON: {}", e))
    })?;

    Ok(body.data)
}

// ============================================================================
// Thread-Safe Pricing State & Synchronizer
// ============================================================================

#[derive(Debug, Default)]
struct SynchronizerState {
    /// In-memory catalog of synchronized model pricing records.
    /// Keyed by full model ID (e.g. `"anthropic/claude-3.7-sonnet"`)
    /// and alias keys (e.g. `"anthropic:claude-3-7-sonnet-20250219"`).
    models: HashMap<String, ModelPricingRecord>,

    /// Primary list of unique model records.
    unique_models: Vec<ModelPricingRecord>,

    /// Last sync statistics.
    last_stats: Option<PricingSyncStats>,

    /// Last sync timestamp.
    last_synced_at: Option<u64>,

    /// History of price diffs observed across runs.
    diff_history: Vec<PricingDiff>,
}

/// Control handle for a running background synchronization loop.
pub struct BackgroundSyncHandle {
    stop_signal: Arc<AtomicBool>,
    join_handle: tokio::task::JoinHandle<()>,
}

impl BackgroundSyncHandle {
    /// Signals the background sync task to terminate.
    pub fn stop(&self) {
        self.stop_signal.store(true, Ordering::SeqCst);
    }

    /// Checks if the background sync task is currently active.
    pub fn is_running(&self) -> bool {
        !self.stop_signal.load(Ordering::SeqCst) && !self.join_handle.is_finished()
    }
}

/// Production-ready dynamic pricing synchronizer.
///
/// Features:
/// - Thread-safe, non-blocking asynchronous state.
/// - Multi-tier fallback: Live OpenRouter API -> Fresh Disk Cache -> Stale Disk Cache -> Built-in.
/// - Rich lookup, filtering, cost calculation, and comparison capabilities.
/// - Periodic background synchronization support.
#[derive(Clone)]
pub struct PricingSynchronizer {
    config: PricingSyncConfig,
    client: reqwest::Client,
    state: Arc<RwLock<SynchronizerState>>,
    is_syncing: Arc<AtomicBool>,
}

impl Default for PricingSynchronizer {
    fn default() -> Self {
        Self::new(PricingSyncConfig::default())
    }
}

impl PricingSynchronizer {
    /// Creates a new `PricingSynchronizer` with the specified configuration.
    pub fn new(config: PricingSyncConfig) -> Self {
        let client = reqwest::Client::builder()
            .timeout(config.timeout)
            .build()
            .unwrap_or_default();

        Self {
            config,
            client,
            state: Arc::new(RwLock::new(SynchronizerState::default())),
            is_syncing: Arc::new(AtomicBool::new(false)),
        }
    }

    /// Creates a synchronizer initialized with default config and optional custom API key.
    pub fn with_api_key(api_key: Option<String>) -> Self {
        let mut config = PricingSyncConfig::default();
        config.api_key = api_key;
        Self::new(config)
    }

    /// Returns a reference to the active configuration.
    pub fn config(&self) -> &PricingSyncConfig {
        &self.config
    }

    /// Returns true if a synchronization pass is currently in progress.
    pub fn is_syncing(&self) -> bool {
        self.is_syncing.load(Ordering::SeqCst)
    }

    /// Returns the Unix timestamp of the most recent successful sync.
    pub async fn last_synced_at(&self) -> Option<u64> {
        let state = self.state.read().await;
        state.last_synced_at
    }

    /// Returns the most recent sync statistics.
    pub async fn last_stats(&self) -> Option<PricingSyncStats> {
        let state = self.state.read().await;
        state.last_stats.clone()
    }

    /// Returns total number of unique models currently indexed.
    pub async fn model_count(&self) -> usize {
        let state = self.state.read().await;
        state.unique_models.len()
    }

    /// Synchronizes pricing data.
    ///
    /// If `force` is `false` and a fresh disk cache exists (younger than `ttl_secs`),
    /// loads immediately from disk without network overhead.
    /// Otherwise, fetches live from OpenRouter and updates disk cache.
    pub async fn sync(&self, force: bool) -> Result<PricingSyncStats, PricingSyncError> {
        // Prevent concurrent overlapping sync runs
        if self
            .is_syncing
            .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
            .is_err()
        {
            // Already syncing, wait briefly or return last stats
            if let Some(stats) = self.last_stats().await {
                return Ok(stats);
            }
        }

        let start_time = std::time::Instant::now();
        let cache_path = self.config.cache_file_path();

        // 1. Check fresh disk cache unless forced
        if !force && is_pricing_cache_fresh(&cache_path, self.config.ttl_secs) {
            if let Some(cache) = load_pricing_cache(&cache_path, self.config.ttl_secs) {
                let stats = self.apply_cached_envelope(cache, PricingSource::DiskCache, start_time.elapsed().as_millis() as u64).await;
                self.is_syncing.store(false, Ordering::SeqCst);
                return Ok(stats);
            }
        }

        // 2. Fetch live from OpenRouter
        let api_key = self.config.resolve_api_key();
        match fetch_openrouter_raw(
            &self.client,
            &self.config.endpoint_url,
            api_key.as_deref(),
            self.config.timeout,
        )
        .await
        {
            Ok(raw_models) => {
                let mut records = Vec::with_capacity(raw_models.len());
                for raw in &raw_models {
                    let record = parse_openrouter_model_pricing(
                        raw,
                        self.config.infer_cache_tiers,
                        PricingSource::Live,
                    );
                    records.push(record);
                }

                let stats = self
                    .apply_live_records(records, start_time.elapsed().as_millis() as u64)
                    .await;

                // Auto-persist to disk cache
                if self.config.auto_save_cache {
                    let _ = self.save_cache_file().await;
                }

                self.is_syncing.store(false, Ordering::SeqCst);
                Ok(stats)
            }
            Err(err) => {
                // 3. Fallback to stale disk cache if available
                if let Some(stale_cache) = load_stale_pricing_cache(&cache_path) {
                    let mut stats = self
                        .apply_cached_envelope(
                            stale_cache,
                            PricingSource::StaleDiskCache,
                            start_time.elapsed().as_millis() as u64,
                        )
                        .await;
                    stats.errors.push(format!("Live sync failed ({}), fell back to stale disk cache", err));
                    self.is_syncing.store(false, Ordering::SeqCst);
                    return Ok(stats);
                }

                self.is_syncing.store(false, Ordering::SeqCst);
                Err(err)
            }
        }
    }

    /// Internal: applies loaded cache envelope into in-memory state.
    async fn apply_cached_envelope(
        &self,
        envelope: PricingCacheEnvelope,
        source: PricingSource,
        duration_ms: u64,
    ) -> PricingSyncStats {
        let mut state = self.state.write().await;
        let mut unique = Vec::with_capacity(envelope.models.len());
        let mut map = HashMap::with_capacity(envelope.models.len() * 2);
        let mut free_count = 0;

        for (id, mut record) in envelope.models {
            record.source = source;
            if record.is_free {
                free_count += 1;
            }

            // Index by ID and normalized keys
            Self::index_record(&mut map, &record);
            unique.push(record);
        }

        let model_count = unique.len();
        state.models = map;
        state.unique_models = unique;
        state.last_synced_at = Some(envelope.timestamp);

        let stats = PricingSyncStats {
            models_fetched: model_count,
            models_updated: 0,
            models_added: model_count,
            free_models_count: free_count,
            sync_duration_ms: duration_ms,
            source: Some(source),
            timestamp: envelope.timestamp,
            errors: Vec::new(),
        };

        state.last_stats = Some(stats.clone());
        stats
    }

    /// Internal: applies newly fetched live records into in-memory state and computes diffs.
    async fn apply_live_records(
        &self,
        records: Vec<ModelPricingRecord>,
        duration_ms: u64,
    ) -> PricingSyncStats {
        let mut state = self.state.write().await;
        let mut map = HashMap::with_capacity(records.len() * 2);
        let mut free_count = 0;
        let mut added = 0;
        let mut updated = 0;
        let mut diffs = Vec::new();

        let now = Utc::now().timestamp() as u64;

        for record in &records {
            if record.is_free {
                free_count += 1;
            }

            // Check diff against prior state
            if let Some(old_record) = state.models.get(&record.model_id) {
                if let Some(diff) = PricingDiff::compute(old_record, record) {
                    diffs.push(diff);
                    updated += 1;
                }
            } else {
                added += 1;
            }

            Self::index_record(&mut map, record);
        }

        let model_count = records.len();
        state.models = map;
        state.unique_models = records;
        state.last_synced_at = Some(now);
        state.diff_history.extend(diffs);

        let stats = PricingSyncStats {
            models_fetched: model_count,
            models_updated: updated,
            models_added: added,
            free_models_count: free_count,
            sync_duration_ms: duration_ms,
            source: Some(PricingSource::Live),
            timestamp: now,
            errors: Vec::new(),
        };

        state.last_stats = Some(stats.clone());
        stats
    }

    /// Indexes a record under various lookup keys (exact ID, normalized provider:model, etc.).
    fn index_record(map: &mut HashMap<String, ModelPricingRecord>, record: &ModelPricingRecord) {
        // 1. Exact OpenRouter ID (e.g. "anthropic/claude-3.7-sonnet")
        map.insert(record.model_id.to_lowercase(), record.clone());

        // 2. Provider:Model key (e.g. "anthropic:claude-3.7-sonnet")
        let prov_key = format!("{}:{}", record.canonical_provider.to_lowercase(), record.canonical_name.to_lowercase());
        map.insert(prov_key, record.clone());

        // 3. Simple Model Name (e.g. "claude-3.7-sonnet") if not conflicting
        let name_key = record.canonical_name.to_lowercase();
        map.entry(name_key).or_insert_with(|| record.clone());
    }

    /// Persists current in-memory pricing to disk cache.
    pub async fn save_cache_file(&self) -> Result<(), PricingSyncError> {
        let state = self.state.read().await;
        let mut models_map = HashMap::with_capacity(state.unique_models.len());
        for record in &state.unique_models {
            models_map.insert(record.model_id.clone(), record.clone());
        }

        let envelope = PricingCacheEnvelope {
            version: PRICING_CACHE_VERSION,
            timestamp: state.last_synced_at.unwrap_or_else(|| Utc::now().timestamp() as u64),
            source: "openrouter".to_string(),
            model_count: state.unique_models.len(),
            models: models_map,
        };

        save_pricing_cache(&self.config.cache_file_path(), &envelope)
    }

    /// Retrieves pricing for a specific provider and model identifier.
    ///
    /// Searches:
    /// 1. Exact model ID (e.g. `"anthropic/claude-3.7-sonnet"`)
    /// 2. `"provider:model"` key (e.g. `"anthropic:claude-3-7-sonnet"`)
    /// 3. Canonical model name match
    pub async fn get_pricing(&self, provider: &str, model: &str) -> Option<ModelPricing> {
        let state = self.state.read().await;
        let prov = provider.trim().to_lowercase();
        let m = model.trim().to_lowercase();

        // 1. Try "provider/model" or exact model id
        if let Some(rec) = state.models.get(&m) {
            return Some(rec.pricing.clone());
        }

        // 2. Try "provider:model"
        let key = format!("{}:{}", prov, m);
        if let Some(rec) = state.models.get(&key) {
            return Some(rec.pricing.clone());
        }

        // 3. If model contains '/'
        if let Some((sub_prov, sub_model)) = m.split_once('/') {
            let sub_key = format!("{}:{}", sub_prov, sub_model);
            if let Some(rec) = state.models.get(&sub_key) {
                return Some(rec.pricing.clone());
            }
        }

        None
    }

    /// Retrieves full capability and pricing record for a model.
    pub async fn get_record(&self, model_id: &str) -> Option<ModelPricingRecord> {
        let state = self.state.read().await;
        let id_lower = model_id.trim().to_lowercase();
        state.models.get(&id_lower).cloned()
    }

    /// Resolves pricing with automatic fallback to [`ModelPricingRegistry`]'s built-in database.
    pub async fn get_pricing_or_fallback(&self, provider: &str, model: &str) -> ModelPricing {
        if let Some(live) = self.get_pricing(provider, model).await {
            live
        } else {
            let registry = ModelPricingRegistry::new();
            registry.get(provider, model)
        }
    }

    /// Returns a list of all indexed model pricing records.
    pub async fn list_records(&self) -> Vec<ModelPricingRecord> {
        let state = self.state.read().await;
        state.unique_models.clone()
    }

    /// Searches indexed models by name, provider, or capability description.
    pub async fn search_models(&self, query: &str) -> Vec<ModelPricingRecord> {
        let state = self.state.read().await;
        let q = query.trim().to_lowercase();
        if q.is_empty() {
            return state.unique_models.clone();
        }

        state
            .unique_models
            .iter()
            .filter(|m| {
                m.model_id.to_lowercase().contains(&q)
                    || m.display_name.to_lowercase().contains(&q)
                    || m.canonical_provider.to_lowercase().contains(&q)
                    || m.canonical_name.to_lowercase().contains(&q)
                    || m.description.as_deref().unwrap_or("").to_lowercase().contains(&q)
            })
            .cloned()
            .collect()
    }

    /// Filters models by provider name.
    pub async fn filter_by_provider(&self, provider: &str) -> Vec<ModelPricingRecord> {
        let state = self.state.read().await;
        let prov = provider.trim().to_lowercase();
        state
            .unique_models
            .iter()
            .filter(|m| m.canonical_provider.to_lowercase() == prov)
            .cloned()
            .collect()
    }

    /// Filters models within a maximum price ceiling (rates per 1M tokens in USD).
    pub async fn filter_by_budget(
        &self,
        max_input_per_million: f64,
        max_output_per_million: f64,
    ) -> Vec<ModelPricingRecord> {
        let state = self.state.read().await;
        state
            .unique_models
            .iter()
            .filter(|m| {
                m.pricing.input_per_million <= max_input_per_million
                    && m.pricing.output_per_million <= max_output_per_million
            })
            .cloned()
            .collect()
    }

    /// Finds the top N cheapest models by blended input/output cost (assuming 3:1 input:output token ratio).
    pub async fn find_cheapest(&self, limit: usize) -> Vec<ModelPricingRecord> {
        let state = self.state.read().await;
        let mut list = state.unique_models.clone();
        list.sort_by(|a, b| {
            let cost_a = (a.pricing.input_per_million * 3.0) + a.pricing.output_per_million;
            let cost_b = (b.pricing.input_per_million * 3.0) + b.pricing.output_per_million;
            cost_a.partial_cmp(&cost_b).unwrap_or(std::cmp::Ordering::Equal)
        });
        list.truncate(limit);
        list
    }

    /// Finds the cheapest models for an exact workload (prompt and completion tokens).
    pub async fn find_cheapest_for_workload(
        &self,
        prompt_tokens: u64,
        completion_tokens: u64,
        limit: usize,
    ) -> Vec<(ModelPricingRecord, CostBreakdown)> {
        let state = self.state.read().await;
        let mut results: Vec<(ModelPricingRecord, CostBreakdown)> = state
            .unique_models
            .iter()
            .map(|m| {
                let cost = m.calculate_cost(prompt_tokens, completion_tokens, 0, 0);
                (m.clone(), cost)
            })
            .collect();

        results.sort_by(|a, b| {
            a.1.total_cost
                .partial_cmp(&b.1.total_cost)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        results.truncate(limit);
        results
    }

    /// Calculates cost breakdown for a specific turn using live or fallback pricing.
    pub async fn calculate_turn_cost(
        &self,
        provider: &str,
        model: &str,
        input_tokens: u64,
        output_tokens: u64,
        cache_read_tokens: u64,
        cache_write_tokens: u64,
    ) -> CostBreakdown {
        let pricing = self.get_pricing_or_fallback(provider, model).await;
        pricing.calculate(input_tokens, output_tokens, cache_read_tokens, cache_write_tokens)
    }

    /// Exports all currently synchronized models into a [`ModelPricingRegistry`].
    pub async fn export_registry(&self) -> ModelPricingRegistry {
        let state = self.state.read().await;
        let mut registry = ModelPricingRegistry::new();
        for record in &state.unique_models {
            registry.register(record.pricing.clone());
        }
        registry
    }

    /// Populates an existing [`CostTracker`] with the synchronizer's custom pricing entries.
    pub async fn populate_cost_tracker(&self, tracker: &mut CostTracker) {
        let registry = self.export_registry().await;
        *tracker = CostTracker::with_registry(registry);
    }

    /// Starts a periodic background synchronization task.
    pub fn start_background_sync(&self, interval: Duration) -> BackgroundSyncHandle {
        let stop_signal = Arc::new(AtomicBool::new(false));
        let stop_clone = Arc::clone(&stop_signal);
        let sync_clone = self.clone();

        let join_handle = tokio::spawn(async move {
            let mut ticker = tokio::time::interval(interval);
            ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

            while !stop_clone.load(Ordering::SeqCst) {
                ticker.tick().await;
                if stop_clone.load(Ordering::SeqCst) {
                    break;
                }
                let _ = sync_clone.sync(true).await;
            }
        });

        BackgroundSyncHandle {
            stop_signal,
            join_handle,
        }
    }
}

// ============================================================================
// Formatting & Reporting Helpers
// ============================================================================

/// Formats a list of model pricing records into a formatted ASCII table.
pub fn format_pricing_table(records: &[ModelPricingRecord]) -> String {
    if records.is_empty() {
        return "No model pricing records available.\n".to_string();
    }

    let mut out = String::new();
    out.push_str("┌──────────────────────────────────────────────┬──────────────┬──────────────┬──────────────┬─────────┐\n");
    out.push_str("│ Model Identifier                             │ Input ($/1M) │ Output ($/1M)│ Cache ($/1M) │ Context │\n");
    out.push_str("├──────────────────────────────────────────────┼──────────────┼──────────────┼──────────────┼─────────┤\n");

    for r in records {
        let id_trunc = if r.model_id.len() > 44 {
            format!("{}…", &r.model_id[..43])
        } else {
            format!("{:<44}", r.model_id)
        };

        let in_str = if r.is_free {
            "FREE".to_string()
        } else {
            format!("${:.2}", r.pricing.input_per_million)
        };
        let out_str = if r.is_free {
            "FREE".to_string()
        } else {
            format!("${:.2}", r.pricing.output_per_million)
        };
        let cache_str = if r.is_free {
            "FREE".to_string()
        } else {
            format!("${:.2}", r.pricing.cache_read_per_million)
        };
        let ctx_str = r
            .context_length
            .map(|c| {
                if c >= 1_000_000 {
                    format!("{:.1}M", c as f64 / 1_000_000.0)
                } else if c >= 1_000 {
                    format!("{}k", c / 1_000)
                } else {
                    format!("{}", c)
                }
            })
            .unwrap_or_else(|| "N/A".to_string());

        out.push_str(&format!(
            "│ {:<44} │ {:>12} │ {:>12} │ {:>12} │ {:>7} │\n",
            id_trunc, in_str, out_str, cache_str, ctx_str
        ));
    }

    out.push_str("└──────────────────────────────────────────────┴──────────────┴──────────────┴──────────────┴─────────┘\n");
    out
}

/// Formats a synchronization summary report.
pub fn format_sync_summary(stats: &PricingSyncStats) -> String {
    let source_str = stats
        .source
        .map(|s| s.to_string())
        .unwrap_or_else(|| "Unknown".to_string());

    let mut out = String::new();
    out.push_str("╔══════════════════════════════════════════════════════════════╗\n");
    out.push_str("║           OpenRouter Dynamic Pricing Sync Summary            ║\n");
    out.push_str("╠══════════════════════════════════════════════════════════════╣\n");
    out.push_str(&format!("║ Source:            {:<41} ║\n", source_str));
    out.push_str(&format!("║ Models Indexed:    {:<41} ║\n", stats.models_fetched));
    out.push_str(&format!("║ Newly Added:       {:<41} ║\n", stats.models_added));
    out.push_str(&format!("║ Prices Updated:    {:<41} ║\n", stats.models_updated));
    out.push_str(&format!("║ Free Models:       {:<41} ║\n", stats.free_models_count));
    out.push_str(&format!("║ Latency:           {:<41} ║\n", format!("{} ms", stats.sync_duration_ms)));
    if !stats.errors.is_empty() {
        out.push_str("╠──────────────────────────────────────────────────────────────╣\n");
        out.push_str("║ Warnings / Notes:                                            ║\n");
        for err in &stats.errors {
            let trunc = if err.len() > 58 {
                format!("{}…", &err[..57])
            } else {
                err.clone()
            };
            out.push_str(&format!("║ - {:<58} ║\n", trunc));
        }
    }
    out.push_str("╚══════════════════════════════════════════════════════════════╝\n");
    out
}

/// Formats a pricing diff report highlighting price changes.
pub fn format_pricing_diff_report(diffs: &[PricingDiff]) -> String {
    if diffs.is_empty() {
        return "No pricing changes detected.\n".to_string();
    }

    let mut out = String::new();
    out.push_str("Model Pricing Changes Detected:\n");
    for d in diffs {
        let direction = if d.is_price_drop() {
            "▼ PRICE DROP"
        } else {
            "▲ PRICE HIKE"
        };
        out.push_str(&format!(
            "- {} ({})\n  Input:  ${:.2} -> ${:.2} ({:+.1}%)\n  Output: ${:.2} -> ${:.2} ({:+.1}%)\n",
            d.model_id,
            direction,
            d.old_input_per_million,
            d.new_input_per_million,
            d.input_change_pct,
            d.old_output_per_million,
            d.new_output_per_million,
            d.output_change_pct,
        ));
    }
    out
}

/// Formats a model cost comparison for an estimated turn workload.
pub fn format_model_cost_comparison(
    models: &[ModelPricingRecord],
    prompt_tokens: u64,
    completion_tokens: u64,
) -> String {
    if models.is_empty() {
        return "No models to compare.\n".to_string();
    }

    let mut out = String::new();
    out.push_str(&format!(
        "Cost Comparison for Workload (Prompt: {} tokens, Completion: {} tokens):\n",
        prompt_tokens, completion_tokens
    ));
    out.push_str("┌──────────────────────────────────────────────┬──────────────┬──────────────┬──────────────┐\n");
    out.push_str("│ Model Identifier                             │ Input Cost   │ Output Cost  │ Total Cost   │\n");
    out.push_str("├──────────────────────────────────────────────┼──────────────┼──────────────┼──────────────┤\n");

    for m in models {
        let cost = m.calculate_cost(prompt_tokens, completion_tokens, 0, 0);
        let id_trunc = if m.model_id.len() > 44 {
            format!("{}…", &m.model_id[..43])
        } else {
            format!("{:<44}", m.model_id)
        };

        out.push_str(&format!(
            "│ {:<44} │ {:>12} │ {:>12} │ {:>12} │\n",
            id_trunc,
            cost.format_input_usd(),
            cost.format_output_usd(),
            cost.format_usd()
        ));
    }

    out.push_str("└──────────────────────────────────────────────┴──────────────┴──────────────┴──────────────┘\n");
    out
}

// ============================================================================
// High-Level Convenience Functions
// ============================================================================

/// Convenience helper to perform a one-shot pricing synchronization.
pub async fn sync_openrouter_pricing(force: bool) -> Result<PricingSyncStats, PricingSyncError> {
    let synchronizer = PricingSynchronizer::default();
    synchronizer.sync(force).await
}

/// Convenience helper to fetch live model pricing records from OpenRouter.
pub async fn fetch_live_openrouter_pricing() -> Result<Vec<ModelPricingRecord>, PricingSyncError> {
    let synchronizer = PricingSynchronizer::default();
    synchronizer.sync(true).await?;
    Ok(synchronizer.list_records().await)
}

// ============================================================================
// Unit Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_openrouter_price_per_million() {
        assert_eq!(parse_openrouter_price_per_million(Some("0.000003")), 3.0);
        assert_eq!(parse_openrouter_price_per_million(Some("0.000015")), 15.0);
        assert_eq!(parse_openrouter_price_per_million(Some("0.00000055")), 0.55);
        assert_eq!(parse_openrouter_price_per_million(Some("0")), 0.0);
        assert_eq!(parse_openrouter_price_per_million(None), 0.0);
        assert_eq!(parse_openrouter_price_per_million(Some("invalid")), 0.0);
    }

    #[test]
    fn test_extract_provider_and_model_name() {
        let (p1, m1) = extract_provider_and_model_name("anthropic/claude-3.7-sonnet");
        assert_eq!(p1, "anthropic");
        assert_eq!(m1, "claude-3.7-sonnet");

        let (p2, m2) = extract_provider_and_model_name("openai/gpt-4o");
        assert_eq!(p2, "openai");
        assert_eq!(m2, "gpt-4o");

        let (p3, m3) = extract_provider_and_model_name("deepseek/deepseek-r1");
        assert_eq!(p3, "deepseek");
        assert_eq!(m3, "deepseek-r1");

        let (p4, m4) = extract_provider_and_model_name("custom-model");
        assert_eq!(p4, "openrouter");
        assert_eq!(m4, "custom-model");
    }

    #[test]
    fn test_infer_cache_rates() {
        // Anthropic: 10% read, 125% write
        let (read_ant, write_ant) = infer_cache_rates("anthropic", "claude-3.7-sonnet", 3.00);
        assert!((read_ant - 0.30).abs() < 1e-6);
        assert!((write_ant - 3.75).abs() < 1e-6);

        // OpenAI: 50% read, 100% write
        let (read_oai, write_oai) = infer_cache_rates("openai", "gpt-4o", 2.50);
        assert!((read_oai - 1.25).abs() < 1e-6);
        assert!((write_oai - 2.50).abs() < 1e-6);

        // DeepSeek: 10% read, 100% write
        let (read_ds, write_ds) = infer_cache_rates("deepseek", "deepseek-chat", 0.14);
        assert!((read_ds - 0.014).abs() < 1e-6);
        assert!((write_ds - 0.14).abs() < 1e-6);
    }

    #[test]
    fn test_parse_openrouter_raw_model() {
        let raw = OpenRouterModelPricingRaw {
            id: "anthropic/claude-3.7-sonnet".to_string(),
            name: Some("Anthropic: Claude 3.7 Sonnet".to_string()),
            description: Some("State-of-the-art hybrid reasoning model".to_string()),
            context_length: Some(200_000),
            pricing: Some(OpenRouterPricingRaw {
                prompt: Some("0.000003".to_string()),
                completion: Some("0.000015".to_string()),
                request: None,
                image: None,
                input_cache_read: None,
                input_cache_write: None,
            }),
            top_provider: Some(OpenRouterTopProviderRaw {
                context_length: Some(200_000),
                max_completion_tokens: Some(64_000),
                is_moderated: Some(false),
            }),
            architecture: Some(OpenRouterArchRaw {
                modality: Some("text+image->text".to_string()),
                tokenizer: Some("claude".to_string()),
                instruct_type: None,
            }),
            per_request_limits: None,
        };

        let record = parse_openrouter_model_pricing(&raw, true, PricingSource::Live);
        assert_eq!(record.model_id, "anthropic/claude-3.7-sonnet");
        assert_eq!(record.canonical_provider, "anthropic");
        assert_eq!(record.canonical_name, "claude-3.7-sonnet");
        assert_eq!(record.pricing.input_per_million, 3.00);
        assert_eq!(record.pricing.output_per_million, 15.00);
        assert_eq!(record.pricing.cache_read_per_million, 0.30);
        assert_eq!(record.pricing.cache_write_per_million, 3.75);
        assert_eq!(record.context_length, Some(200_000));
        assert_eq!(record.max_completion_tokens, Some(64_000));
        assert!(!record.is_free);
        assert!(record.supports_vision);
    }

    #[test]
    fn test_free_model_detection() {
        let raw = OpenRouterModelPricingRaw {
            id: "meta-llama/llama-3.3-70b-instruct:free".to_string(),
            name: Some("Meta: Llama 3.3 70B (Free)".to_string()),
            description: None,
            context_length: Some(131_072),
            pricing: Some(OpenRouterPricingRaw {
                prompt: Some("0".to_string()),
                completion: Some("0".to_string()),
                request: None,
                image: None,
                input_cache_read: None,
                input_cache_write: None,
            }),
            top_provider: None,
            architecture: None,
            per_request_limits: None,
        };

        let record = parse_openrouter_model_pricing(&raw, true, PricingSource::Live);
        assert!(record.is_free);
        assert_eq!(record.pricing.input_per_million, 0.0);
        assert_eq!(record.pricing.output_per_million, 0.0);
    }

    #[test]
    fn test_pricing_diff_computation() {
        let old_rec = ModelPricingRecord {
            model_id: "deepseek/deepseek-r1".to_string(),
            canonical_provider: "deepseek".to_string(),
            canonical_name: "deepseek-r1".to_string(),
            display_name: "DeepSeek R1".to_string(),
            pricing: ModelPricing::new("deepseek", "deepseek-r1", 0.55, 2.19, 0.14, 0.55),
            context_length: Some(64_000),
            max_completion_tokens: Some(8_000),
            is_free: false,
            supports_vision: false,
            description: None,
            last_updated_at: 1000,
            source: PricingSource::Live,
        };

        let mut new_rec = old_rec.clone();
        new_rec.pricing.input_per_million = 0.45; // Price drop
        new_rec.pricing.output_per_million = 2.00; // Price drop

        let diff = PricingDiff::compute(&old_rec, &new_rec).expect("Diff should be detected");
        assert!(diff.is_price_drop());
        assert!(!diff.is_price_increase());
        assert!(diff.input_change_pct < 0.0);
        assert!(diff.output_change_pct < 0.0);
    }

    #[tokio::test]
    async fn test_synchronizer_in_memory_crud_and_search() {
        let sync = PricingSynchronizer::default();

        let raw1 = OpenRouterModelPricingRaw {
            id: "anthropic/claude-3.7-sonnet".to_string(),
            name: Some("Anthropic: Claude 3.7 Sonnet".to_string()),
            description: Some("Hybrid reasoning".to_string()),
            context_length: Some(200_000),
            pricing: Some(OpenRouterPricingRaw {
                prompt: Some("0.000003".to_string()),
                completion: Some("0.000015".to_string()),
                request: None,
                image: None,
                input_cache_read: None,
                input_cache_write: None,
            }),
            top_provider: None,
            architecture: None,
            per_request_limits: None,
        };

        let raw2 = OpenRouterModelPricingRaw {
            id: "openai/gpt-4o-mini".to_string(),
            name: Some("OpenAI: GPT-4o Mini".to_string()),
            description: Some("Fast low-cost model".to_string()),
            context_length: Some(128_000),
            pricing: Some(OpenRouterPricingRaw {
                prompt: Some("0.00000015".to_string()),
                completion: Some("0.00000060".to_string()),
                request: None,
                image: None,
                input_cache_read: None,
                input_cache_write: None,
            }),
            top_provider: None,
            architecture: None,
            per_request_limits: None,
        };

        let records = vec![
            parse_openrouter_model_pricing(&raw1, true, PricingSource::Live),
            parse_openrouter_model_pricing(&raw2, true, PricingSource::Live),
        ];

        let stats = sync.apply_live_records(records, 15).await;
        assert_eq!(stats.models_fetched, 2);
        assert_eq!(sync.model_count().await, 2);

        // Lookup by full ID
        let sonnet_pricing = sync.get_pricing("anthropic", "claude-3.7-sonnet").await;
        assert!(sonnet_pricing.is_some());
        assert_eq!(sonnet_pricing.unwrap().input_per_million, 3.00);

        // Lookup mini
        let mini_pricing = sync.get_pricing("openai", "gpt-4o-mini").await;
        assert!(mini_pricing.is_some());
        assert_eq!(mini_pricing.unwrap().input_per_million, 0.15);

        // Search
        let search_results = sync.search_models("reasoning").await;
        assert_eq!(search_results.len(), 1);
        assert_eq!(search_results[0].canonical_name, "claude-3.7-sonnet");

        // Cheapest
        let cheapest = sync.find_cheapest(1).await;
        assert_eq!(cheapest.len(), 1);
        assert_eq!(cheapest[0].canonical_name, "gpt-4o-mini");

        // Turn cost calculation
        let cost = sync
            .calculate_turn_cost("anthropic", "claude-3.7-sonnet", 100_000, 10_000, 0, 0)
            .await;
        // 100k @ $3/1M = $0.30, 10k @ $15/1M = $0.15 => total = $0.45
        assert!((cost.total_cost - 0.45).abs() < 1e-6);

        // Export registry
        let registry = sync.export_registry().await;
        let p = registry.get("anthropic", "claude-3.7-sonnet");
        assert_eq!(p.input_per_million, 3.00);
    }

    #[test]
    fn test_disk_cache_roundtrip() {
        let temp_dir = tempfile::tempdir().expect("Failed to create tempdir");
        let cache_file = temp_dir.path().join("pricing_cache.json");

        let mut models = HashMap::new();
        let record = ModelPricingRecord {
            model_id: "deepseek/deepseek-chat".to_string(),
            canonical_provider: "deepseek".to_string(),
            canonical_name: "deepseek-chat".to_string(),
            display_name: "DeepSeek V3".to_string(),
            pricing: ModelPricing::new("deepseek", "deepseek-chat", 0.14, 0.28, 0.014, 0.14),
            context_length: Some(64_000),
            max_completion_tokens: Some(8_000),
            is_free: false,
            supports_vision: false,
            description: None,
            last_updated_at: Utc::now().timestamp() as u64,
            source: PricingSource::Live,
        };
        models.insert(record.model_id.clone(), record);

        let envelope = PricingCacheEnvelope {
            version: PRICING_CACHE_VERSION,
            timestamp: Utc::now().timestamp() as u64,
            source: "openrouter".to_string(),
            model_count: 1,
            models,
        };

        save_pricing_cache(&cache_file, &envelope).expect("Failed to save pricing cache");
        assert!(is_pricing_cache_fresh(&cache_file, 3600));

        let loaded = load_pricing_cache(&cache_file, 3600).expect("Failed to load pricing cache");
        assert_eq!(loaded.model_count, 1);
        assert!(loaded.models.contains_key("deepseek/deepseek-chat"));
    }

    #[test]
    fn test_formatting_output() {
        let record = ModelPricingRecord {
            model_id: "anthropic/claude-3.7-sonnet".to_string(),
            canonical_provider: "anthropic".to_string(),
            canonical_name: "claude-3.7-sonnet".to_string(),
            display_name: "Claude 3.7 Sonnet".to_string(),
            pricing: ModelPricing::new("anthropic", "claude-3.7-sonnet", 3.00, 15.00, 0.30, 3.75),
            context_length: Some(200_000),
            max_completion_tokens: Some(64_000),
            is_free: false,
            supports_vision: true,
            description: None,
            last_updated_at: 1000,
            source: PricingSource::Live,
        };

        let table = format_pricing_table(&[record.clone()]);
        assert!(table.contains("anthropic/claude-3.7-sonnet"));
        assert!(table.contains("$3.00"));
        assert!(table.contains("$15.00"));

        let stats = PricingSyncStats {
            models_fetched: 10,
            models_updated: 2,
            models_added: 8,
            free_models_count: 1,
            sync_duration_ms: 120,
            source: Some(PricingSource::Live),
            timestamp: 1000,
            errors: vec![],
        };
        let summary = format_sync_summary(&stats);
        assert!(summary.contains("OpenRouter Dynamic Pricing Sync Summary"));
        assert!(summary.contains("10"));

        let comparison = format_model_cost_comparison(&[record], 100_000, 20_000);
        assert!(comparison.contains("Claude 3.7 Sonnet") || comparison.contains("anthropic/claude-3.7-sonnet"));
    }
}
