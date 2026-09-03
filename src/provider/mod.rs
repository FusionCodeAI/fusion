pub mod anthropic;
pub mod catalog;
pub mod client;
pub mod offline;
pub mod ollama;
pub mod openai;
pub mod openrouter;
pub mod partial_json;
pub mod retry;
pub mod types;

pub use anthropic::AnthropicClient;
pub use catalog::{
    get_catalog, get_models, refresh_catalog, static_model_list, sync_catalog, CatalogFetcher,
    CatalogModel, CatalogSource, ModelCatalog, ModelCatalogCache, ModelEntry,
};
pub use client::LlmClient;
pub use offline::{
    auto_switch_offline, auto_switch_offline_sync, check_internet_connectivity,
    check_internet_connectivity_async, is_offline, is_online, ping_ollama, ping_ollama_sync,
    select_best_local_model, select_best_local_model_from_names, ConnectivityProber,
    DefaultConnectivityProber, DefaultOllamaProber, MockConnectivityProber, MockOllamaProber,
    NetworkEnvironmentStatus, OfflineDetector, OfflineDetectorConfig, OfflineReason,
    OfflineTransitionResult, OllamaProber, DEFAULT_OLLAMA_ADDR, DEFAULT_OLLAMA_HOST,
    DEFAULT_OLLAMA_PORT, DEFAULT_OLLAMA_SOCKET_ADDR, DEFAULT_OLLAMA_URL,
};
pub use ollama::{OllamaClient, OllamaModelInfo, OllamaTagsResponse};
pub use openrouter::OpenRouterClient;
pub use partial_json::{
    parse_partial_json, parse_partial_json_lossy, parse_partial_json_with_options, repair_json,
    repair_partial_json, strip_markdown, PartialJsonError, PartialJsonOptions, PartialToolCall,
    StreamingJsonParser, StreamingToolCallAccumulator, UnclosedKeyStrategy,
};
pub use retry::{
    classify_error, classify_error_str, classify_status_code, classify_stream_chunk,
    is_retryable_status, parse_retry_after_header, parse_retry_after_value, retry_async,
    retry_stream, Backoff, FastRng, HttpError, JitterMode, RetryPolicy, RetryPolicyBuilder,
    RetryReason, RetryStats, RetryingLlmClient, RetryingStream, DEFAULT_BACKOFF_FACTOR,
    DEFAULT_INITIAL_DELAY, DEFAULT_MAX_DELAY, DEFAULT_MAX_RETRIES,
};
pub use types::*;
