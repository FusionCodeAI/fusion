use std::collections::BTreeMap;
use std::time::Duration;

use anyhow::Context;
use eventsource_stream::Eventsource;
use futures::StreamExt;
use reqwest::header::{HeaderMap, HeaderName, HeaderValue, AUTHORIZATION, CONTENT_TYPE};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tokio::sync::mpsc;

use crate::config::Config;
use crate::provider::types::{Message, Role, StreamChunk, ToolCall, ToolDefinition};

pub const OPENROUTER_DEFAULT_BASE_URL: &str = "https://openrouter.ai/api/v1";
pub const OPENROUTER_DEFAULT_REFERER: &str = "https://github.com/theaungmyatmoe/fusion";
pub const OPENROUTER_DEFAULT_TITLE: &str = "Fusion AI Assistant";

/// OpenRouter custom provider routing and filtering preferences.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct OpenRouterProviderPreferences {
    /// Preferred provider order (e.g. `["Anthropic", "OpenAI", "Together"]`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub order: Option<Vec<String>>,

    /// Whether to allow automatic fallbacks to other providers.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub allow_fallbacks: Option<bool>,

    /// Only route to providers that support all request parameters.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub require_parameters: Option<bool>,

    /// Data collection policy (`"allow"` or `"deny"`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data_collection: Option<String>,

    /// List of provider slugs to ignore.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ignore: Option<Vec<String>>,

    /// Quantization level filter (e.g. `["int4", "int8", "fp16", "fp8", "bf16"]`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub quantizations: Option<Vec<String>>,

    /// Provider sorting strategy (`"price"`, `"throughput"`, or `"latency"`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sort: Option<String>,

    /// Maximum price limits per unit.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_price: Option<OpenRouterMaxPrice>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct OpenRouterMaxPrice {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub prompt: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub completion: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub image: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub request: Option<f64>,
}

/// OpenRouter plugin configuration (e.g. web search).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct OpenRouterPlugin {
    pub id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_results: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub search_prompt: Option<String>,
}

/// Extended OpenRouter-specific request options.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct OpenRouterRequestOptions {
    /// Array of fallback model IDs to try in order if the primary fails.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub models: Option<Vec<String>>,

    /// Route routing mode (e.g. `"fallback"`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub route: Option<String>,

    /// Prompt transforms (e.g. `["middle-out"]`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub transforms: Option<Vec<String>>,

    /// Custom provider routing and filter preferences.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider: Option<OpenRouterProviderPreferences>,

    /// Active plugins for this request.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub plugins: Option<Vec<OpenRouterPlugin>>,

    /// Explicitly request reasoning / thinking output where supported.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub include_reasoning: Option<bool>,

    /// Repetition penalty factor.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub repetition_penalty: Option<f32>,

    /// Top-K sampling cutoff.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub top_k: Option<u32>,

    /// Min-P sampling cutoff.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub min_p: Option<f32>,

    /// Top-A sampling cutoff.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub top_a: Option<f32>,

    /// Random seed for deterministic generation.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub seed: Option<u64>,
}

// ==========================================
// OpenRouter Model Listing Types
// ==========================================

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OpenRouterModelListResponse {
    pub data: Vec<OpenRouterModel>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct OpenRouterModel {
    pub id: String,
    pub name: Option<String>,
    pub created: Option<u64>,
    pub description: Option<String>,
    pub context_length: Option<u64>,
    pub pricing: Option<OpenRouterPricing>,
    pub top_provider: Option<OpenRouterTopProvider>,
    pub architecture: Option<OpenRouterArchitecture>,
    pub per_request_limits: Option<OpenRouterLimits>,
}

impl OpenRouterModel {
    /// Return the model's display name or fallback to its ID.
    pub fn display_name(&self) -> &str {
        self.name.as_deref().unwrap_or(&self.id)
    }

    /// Calculate estimated prompt price in USD per 1M tokens.
    pub fn prompt_price_per_million(&self) -> Option<f64> {
        self.pricing
            .as_ref()
            .and_then(|p| p.prompt.as_deref())
            .and_then(|s| s.parse::<f64>().ok())
            .map(|cost_per_token| cost_per_token * 1_000_000.0)
    }

    /// Calculate estimated completion price in USD per 1M tokens.
    pub fn completion_price_per_million(&self) -> Option<f64> {
        self.pricing
            .as_ref()
            .and_then(|p| p.completion.as_deref())
            .and_then(|s| s.parse::<f64>().ok())
            .map(|cost_per_token| cost_per_token * 1_000_000.0)
    }

    /// Whether this model is marked as free.
    pub fn is_free(&self) -> bool {
        if self.id.ends_with(":free") {
            return true;
        }
        if let Some(pricing) = &self.pricing {
            let p_zero = pricing.prompt.as_deref().map(|s| s == "0" || s == "0.0").unwrap_or(false);
            let c_zero = pricing.completion.as_deref().map(|s| s == "0" || s == "0.0").unwrap_or(false);
            if p_zero && c_zero {
                return true;
            }
        }
        false
    }

    /// Whether this model supports multimodal vision inputs.
    pub fn supports_vision(&self) -> bool {
        if let Some(arch) = &self.architecture {
            if let Some(modality) = &arch.modality {
                return modality.contains("image");
            }
        }
        false
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct OpenRouterPricing {
    pub prompt: Option<String>,
    pub completion: Option<String>,
    pub image: Option<String>,
    pub request: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct OpenRouterTopProvider {
    pub context_length: Option<u64>,
    pub max_completion_tokens: Option<u64>,
    pub is_moderated: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct OpenRouterArchitecture {
    pub modality: Option<String>,
    pub tokenizer: Option<String>,
    pub instruct_type: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct OpenRouterLimits {
    pub prompt_tokens: Option<u64>,
    pub completion_tokens: Option<u64>,
}

// ==========================================
// OpenRouter Account & Generation Stats Types
// ==========================================

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OpenRouterKeyInfoResponse {
    pub data: OpenRouterKeyData,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct OpenRouterKeyData {
    pub label: Option<String>,
    pub usage: Option<f64>,
    pub limit: Option<f64>,
    pub is_free_tier: Option<bool>,
    pub rate_limit: Option<OpenRouterRateLimit>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct OpenRouterRateLimit {
    pub requests: Option<u64>,
    pub interval: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OpenRouterGenerationResponse {
    pub data: OpenRouterGenerationData,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct OpenRouterGenerationData {
    pub id: Option<String>,
    pub model: Option<String>,
    pub streamed: Option<bool>,
    pub generation_time: Option<u64>,
    pub created_at: Option<String>,
    pub tokens_prompt: Option<u64>,
    pub tokens_completion: Option<u64>,
    pub native_tokens_prompt: Option<u64>,
    pub native_tokens_completion: Option<u64>,
    pub total_cost: Option<f64>,
    pub app_id: Option<u64>,
    pub latency: Option<f64>,
    pub moderation_latency: Option<f64>,
    pub generation_latency: Option<f64>,
    pub finish_reason: Option<String>,
}

// ==========================================
// OpenRouter Provider Client
// ==========================================

#[derive(Clone)]
pub struct OpenRouterClient {
    api_key: Option<String>,
    base_url: String,
    site_url: String,
    app_title: String,
    default_options: OpenRouterRequestOptions,
    client: reqwest::Client,
}

impl Default for OpenRouterClient {
    fn default() -> Self {
        Self::new(None::<String>)
    }
}

impl OpenRouterClient {
    /// Create a new OpenRouter client with optional API key.
    pub fn new(api_key: Option<impl Into<String>>) -> Self {
        let client = reqwest::Client::builder()
            .use_rustls_tls()
            .connect_timeout(Duration::from_secs(30))
            .build()
            .unwrap_or_else(|_| reqwest::Client::new());

        Self {
            api_key: api_key.map(|k| k.into()),
            base_url: OPENROUTER_DEFAULT_BASE_URL.to_string(),
            site_url: OPENROUTER_DEFAULT_REFERER.to_string(),
            app_title: OPENROUTER_DEFAULT_TITLE.to_string(),
            default_options: OpenRouterRequestOptions::default(),
            client,
        }
    }

    /// Create an OpenRouter client initialized from Fusion configuration.
    pub fn from_config(config: &Config) -> Self {
        let (key, url) = config.get_key_and_url("openrouter");
        let base_url = if url.is_empty() {
            OPENROUTER_DEFAULT_BASE_URL.to_string()
        } else {
            url
        };
        Self::new(key).with_base_url(base_url)
    }

    /// Set custom API base URL.
    pub fn with_base_url(mut self, base_url: impl Into<String>) -> Self {
        self.base_url = base_url.into();
        self
    }

    /// Set custom HTTP-Referer header value.
    pub fn with_referer(mut self, referer: impl Into<String>) -> Self {
        self.site_url = referer.into();
        self
    }

    /// Set custom X-Title header value.
    pub fn with_title(mut self, title: impl Into<String>) -> Self {
        self.app_title = title.into();
        self
    }

    /// Set default provider routing preferences.
    pub fn with_provider_preferences(mut self, prefs: OpenRouterProviderPreferences) -> Self {
        self.default_options.provider = Some(prefs);
        self
    }

    /// Set default request options.
    pub fn with_request_options(mut self, options: OpenRouterRequestOptions) -> Self {
        self.default_options = options;
        self
    }

    /// Use a specific reqwest client instance.
    pub fn with_http_client(mut self, client: reqwest::Client) -> Self {
        self.client = client;
        self
    }

    pub fn base_url(&self) -> &str {
        &self.base_url
    }

    pub fn site_url(&self) -> &str {
        &self.site_url
    }

    pub fn app_title(&self) -> &str {
        &self.app_title
    }

    pub fn api_key(&self) -> Option<&str> {
        self.api_key.as_deref()
    }

    pub fn default_options(&self) -> &OpenRouterRequestOptions {
        &self.default_options
    }

    /// Build standard OpenRouter HTTP headers including `HTTP-Referer` and `X-Title`.
    pub fn build_headers(&self) -> HeaderMap {
        let mut headers = HeaderMap::new();
        headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));

        if let Some(key) = &self.api_key {
            let trimmed = key.trim();
            if !trimmed.is_empty() {
                let auth_val = format!("Bearer {}", trimmed);
                if let Ok(hv) = HeaderValue::from_str(&auth_val) {
                    headers.insert(AUTHORIZATION, hv);
                }
            }
        }

        if !self.site_url.is_empty() {
            if let Ok(hv) = HeaderValue::from_str(&self.site_url) {
                headers.insert(HeaderName::from_static("http-referer"), hv);
            }
        }

        if !self.app_title.is_empty() {
            if let Ok(hv) = HeaderValue::from_str(&self.app_title) {
                headers.insert(HeaderName::from_static("x-title"), hv);
            }
        }

        headers
    }

    /// Fetch the full list of available models from OpenRouter (`GET /models`).
    pub async fn list_models(&self) -> anyhow::Result<Vec<OpenRouterModel>> {
        let base = self.base_url.trim_end_matches('/');
        let url = if base.ends_with("/models") {
            base.to_string()
        } else {
            format!("{}/models", base)
        };

        let headers = self.build_headers();
        let res = self
            .client
            .get(&url)
            .headers(headers)
            .send()
            .await
            .context("Failed to send list_models request to OpenRouter")?;

        let status = res.status();
        if !status.is_success() {
            let error_text = res.text().await.unwrap_or_default();
            anyhow::bail!(
                "OpenRouter models listing returned HTTP {}: {}",
                status,
                error_text
            );
        }

        let model_list: OpenRouterModelListResponse = res
            .json()
            .await
            .context("Failed to parse OpenRouter models response JSON")?;

        Ok(model_list.data)
    }

    /// Find a single model by ID from OpenRouter.
    pub async fn get_model(&self, model_id: &str) -> anyhow::Result<Option<OpenRouterModel>> {
        let models = self.list_models().await?;
        Ok(models.into_iter().find(|m| m.id == model_id))
    }

    /// Query the current API key details, credit balance, and limits (`GET /auth/key`).
    pub async fn get_key_info(&self) -> anyhow::Result<OpenRouterKeyData> {
        let base = self.base_url.trim_end_matches('/');
        let url = if base.ends_with("/auth/key") {
            base.to_string()
        } else {
            format!("{}/auth/key", base)
        };

        let headers = self.build_headers();
        let res = self
            .client
            .get(&url)
            .headers(headers)
            .send()
            .await
            .context("Failed to send auth/key request to OpenRouter")?;

        let status = res.status();
        if !status.is_success() {
            let error_text = res.text().await.unwrap_or_default();
            anyhow::bail!(
                "OpenRouter auth/key returned HTTP {}: {}",
                status,
                error_text
            );
        }

        let key_info: OpenRouterKeyInfoResponse = res
            .json()
            .await
            .context("Failed to parse OpenRouter key info response JSON")?;

        Ok(key_info.data)
    }

    /// Query generation stats and actual cost for a generation ID (`GET /generation?id=...`).
    pub async fn get_generation_stats(&self, generation_id: &str) -> anyhow::Result<OpenRouterGenerationData> {
        let base = self.base_url.trim_end_matches('/');
        let root = if base.ends_with("/api/v1") {
            base
        } else {
            base
        };
        let url = format!("{}/generation?id={}", root, generation_id);

        let headers = self.build_headers();
        let res = self
            .client
            .get(&url)
            .headers(headers)
            .send()
            .await
            .context("Failed to send generation stats request to OpenRouter")?;

        let status = res.status();
        if !status.is_success() {
            let error_text = res.text().await.unwrap_or_default();
            anyhow::bail!(
                "OpenRouter generation stats returned HTTP {}: {}",
                status,
                error_text
            );
        }

        let gen_resp: OpenRouterGenerationResponse = res
            .json()
            .await
            .context("Failed to parse OpenRouter generation response JSON")?;

        Ok(gen_resp.data)
    }

    /// Construct JSON request payload for OpenRouter chat completions.
    pub fn build_chat_payload(
        &self,
        model: &str,
        temperature: Option<f32>,
        max_tokens: Option<u32>,
        messages: &[Message],
        tools: &[ToolDefinition],
        stream: bool,
        custom_options: Option<&OpenRouterRequestOptions>,
    ) -> Value {
        let mut payload = json!({
            "model": model,
            "stream": stream,
        });

        if stream {
            payload["stream_options"] = json!({ "include_usage": true });
        }

        if let Some(temp) = temperature {
            payload["temperature"] = json!(temp);
        }
        if let Some(mt) = max_tokens {
            payload["max_tokens"] = json!(mt);
        }

        // Format messages
        let mut messages_json = Vec::new();
        for msg in messages {
            let mut item = json!({
                "role": match msg.role {
                    Role::System => "system",
                    Role::User => "user",
                    Role::Assistant => "assistant",
                    Role::Tool => "tool",
                },
                "content": msg.content,
            });

            if let Some(name) = &msg.name {
                item["name"] = json!(name);
            }

            if msg.role == Role::Assistant {
                if let Some(tool_calls) = &msg.tool_calls {
                    if !tool_calls.is_empty() {
                        let calls_json: Vec<Value> = tool_calls
                            .iter()
                            .map(|tc| {
                                json!({
                                    "id": tc.id,
                                    "type": "function",
                                    "function": {
                                        "name": tc.name,
                                        "arguments": tc.arguments,
                                    }
                                })
                            })
                            .collect();
                        item["tool_calls"] = json!(calls_json);
                    }
                }
            }

            if msg.role == Role::Tool {
                if let Some(tool_call_id) = &msg.tool_call_id {
                    item["tool_call_id"] = json!(tool_call_id);
                }
            }

            messages_json.push(item);
        }
        payload["messages"] = json!(messages_json);

        // Format tool definitions
        if !tools.is_empty() {
            let tools_json: Vec<Value> = tools
                .iter()
                .map(|t| {
                    json!({
                        "type": "function",
                        "function": {
                            "name": t.name,
                            "description": t.description,
                            "parameters": t.parameters,
                        }
                    })
                })
                .collect();
            payload["tools"] = json!(tools_json);
            payload["tool_choice"] = json!("auto");
        }

        // Merge request options (custom options override defaults)
        let opts = custom_options.unwrap_or(&self.default_options);

        if let Some(models) = &opts.models {
            payload["models"] = json!(models);
        }
        if let Some(route) = &opts.route {
            payload["route"] = json!(route);
        }
        if let Some(transforms) = &opts.transforms {
            payload["transforms"] = json!(transforms);
        }
        if let Some(provider) = &opts.provider {
            payload["provider"] = serde_json::to_value(provider).unwrap_or(Value::Null);
        }
        if let Some(plugins) = &opts.plugins {
            payload["plugins"] = serde_json::to_value(plugins).unwrap_or(Value::Null);
        }
        if let Some(include_reasoning) = opts.include_reasoning {
            payload["include_reasoning"] = json!(include_reasoning);
        }
        if let Some(rep_pen) = opts.repetition_penalty {
            payload["repetition_penalty"] = json!(rep_pen);
        }
        if let Some(top_k) = opts.top_k {
            payload["top_k"] = json!(top_k);
        }
        if let Some(min_p) = opts.min_p {
            payload["min_p"] = json!(min_p);
        }
        if let Some(top_a) = opts.top_a {
            payload["top_a"] = json!(top_a);
        }
        if let Some(seed) = opts.seed {
            payload["seed"] = json!(seed);
        }

        payload
    }

    /// Stream a chat completion through OpenRouter.
    pub async fn stream_chat(
        &self,
        model: &str,
        temperature: Option<f32>,
        max_tokens: Option<u32>,
        messages: &[Message],
        tools: &[ToolDefinition],
        custom_options: Option<&OpenRouterRequestOptions>,
    ) -> anyhow::Result<mpsc::Receiver<StreamChunk>> {
        let base = self.base_url.trim_end_matches('/');
        let url = if base.ends_with("/chat/completions") {
            base.to_string()
        } else if base.ends_with("/v1") {
            format!("{}/chat/completions", base)
        } else {
            format!("{}/chat/completions", base)
        };

        let headers = self.build_headers();
        let payload = self.build_chat_payload(
            model,
            temperature,
            max_tokens,
            messages,
            tools,
            true,
            custom_options,
        );

        let (tx, rx) = mpsc::channel::<StreamChunk>(256);

        let res = self
            .client
            .post(&url)
            .headers(headers)
            .json(&payload)
            .send()
            .await
            .context("Failed to send OpenRouter stream request")?;

        let status = res.status();
        if !status.is_success() {
            let error_text = res.text().await.unwrap_or_default();
            anyhow::bail!(
                "OpenRouter API returned HTTP {}: {}",
                status,
                error_text
            );
        }

        let mut stream = res.bytes_stream().eventsource();

        tokio::spawn(async move {
            let mut prompt_tokens = None;
            let mut completion_tokens = None;
            let mut done_sent = false;

            while let Some(event_res) = stream.next().await {
                match event_res {
                    Ok(event) => {
                        let data = event.data.trim();
                        if data == "[DONE]" {
                            if !done_sent {
                                let _ = tx
                                    .send(StreamChunk::Done {
                                        finish_reason: Some("stop".to_string()),
                                        prompt_tokens,
                                        completion_tokens,
                                    })
                                    .await;
                                done_sent = true;
                            }
                            break;
                        }

                        if data.is_empty() {
                            continue;
                        }

                        if let Ok(val) = serde_json::from_str::<Value>(data) {
                            // Check for OpenRouter error in chunk
                            if let Some(err) = val.get("error") {
                                let err_msg = err
                                    .get("message")
                                    .and_then(|v| v.as_str())
                                    .unwrap_or("Unknown OpenRouter error");
                                let _ = tx.send(StreamChunk::Error(err_msg.to_string())).await;
                                return;
                            }

                            // Extract usage statistics
                            if let Some(usage) = val.get("usage") {
                                if let Some(pt) =
                                    usage.get("prompt_tokens").and_then(|v| v.as_u64())
                                {
                                    prompt_tokens = Some(pt as u32);
                                }
                                if let Some(ct) =
                                    usage.get("completion_tokens").and_then(|v| v.as_u64())
                                {
                                    completion_tokens = Some(ct as u32);
                                }
                            }

                            // Extract choices delta
                            if let Some(choices) = val.get("choices").and_then(|v| v.as_array()) {
                                for choice in choices {
                                    let finish_reason = choice
                                        .get("finish_reason")
                                        .and_then(|v| v.as_str())
                                        .map(|s| s.to_string());

                                    if let Some(delta) = choice.get("delta") {
                                        // Handle reasoning / thinking tokens (e.g. DeepSeek R1, Claude reasoning via OpenRouter)
                                        if let Some(reasoning) = delta
                                            .get("reasoning_content")
                                            .or_else(|| delta.get("reasoning"))
                                            .or_else(|| delta.get("thinking"))
                                            .and_then(|v| v.as_str())
                                        {
                                            if !reasoning.is_empty() {
                                                let _ = tx
                                                    .send(StreamChunk::ThinkingDelta(
                                                        reasoning.to_string(),
                                                    ))
                                                    .await;
                                            }
                                        }

                                        // Handle standard content delta
                                        if let Some(content) =
                                            delta.get("content").and_then(|v| v.as_str())
                                        {
                                            if !content.is_empty() {
                                                let _ = tx
                                                    .send(StreamChunk::ContentDelta(
                                                        content.to_string(),
                                                    ))
                                                    .await;
                                            }
                                        }

                                        // Handle tool call streaming chunks
                                        if let Some(tool_calls) =
                                            delta.get("tool_calls").and_then(|v| v.as_array())
                                        {
                                            for tc in tool_calls {
                                                let index = tc
                                                    .get("index")
                                                    .and_then(|v| v.as_u64())
                                                    .unwrap_or(0)
                                                    as usize;
                                                let id = tc
                                                    .get("id")
                                                    .and_then(|v| v.as_str())
                                                    .map(|s| s.to_string());
                                                let name = tc
                                                    .get("function")
                                                    .and_then(|f| f.get("name"))
                                                    .and_then(|v| v.as_str())
                                                    .map(|s| s.to_string());
                                                let arguments_delta = tc
                                                    .get("function")
                                                    .and_then(|f| f.get("arguments"))
                                                    .and_then(|v| v.as_str())
                                                    .unwrap_or("")
                                                    .to_string();

                                                let _ = tx
                                                    .send(StreamChunk::ToolCallDelta {
                                                        index,
                                                        id,
                                                        name,
                                                        arguments_delta,
                                                    })
                                                    .await;
                                            }
                                        }
                                    }

                                    if let Some(fr) = finish_reason {
                                        let _ = tx
                                            .send(StreamChunk::Done {
                                                finish_reason: Some(fr),
                                                prompt_tokens,
                                                completion_tokens,
                                            })
                                            .await;
                                        done_sent = true;
                                    }
                                }
                            }
                        }
                    }
                    Err(e) => {
                        let _ = tx.send(StreamChunk::Error(e.to_string())).await;
                        break;
                    }
                }
            }

            if !done_sent {
                let _ = tx
                    .send(StreamChunk::Done {
                        finish_reason: None,
                        prompt_tokens,
                        completion_tokens,
                    })
                    .await;
            }
        });

        Ok(rx)
    }

    /// Collect full non-streamed response (or aggregated stream) into text, thinking, and tool calls.
    pub async fn complete(
        &self,
        model: &str,
        temperature: Option<f32>,
        max_tokens: Option<u32>,
        messages: &[Message],
        tools: &[ToolDefinition],
        custom_options: Option<&OpenRouterRequestOptions>,
    ) -> anyhow::Result<(String, Option<String>, Vec<ToolCall>)> {
        let mut rx = self
            .stream_chat(
                model,
                temperature,
                max_tokens,
                messages,
                tools,
                custom_options,
            )
            .await?;

        let mut content = String::new();
        let mut thinking = String::new();
        let mut tool_calls_map: BTreeMap<usize, (Option<String>, Option<String>, String)> =
            BTreeMap::new();

        while let Some(chunk) = rx.recv().await {
            match chunk {
                StreamChunk::ContentDelta(delta) => {
                    content.push_str(&delta);
                }
                StreamChunk::ThinkingDelta(delta) => {
                    thinking.push_str(&delta);
                }
                StreamChunk::ToolCallDelta {
                    index,
                    id,
                    name,
                    arguments_delta,
                } => {
                    let entry = tool_calls_map
                        .entry(index)
                        .or_insert_with(|| (None, None, String::new()));
                    if let Some(id_str) = id {
                        entry.0 = Some(id_str);
                    }
                    if let Some(name_str) = name {
                        entry.1 = Some(name_str);
                    }
                    entry.2.push_str(&arguments_delta);
                }
                StreamChunk::Error(err) => {
                    anyhow::bail!("OpenRouter stream error: {}", err);
                }
                StreamChunk::Done { .. } => {}
            }
        }

        let mut tool_calls = Vec::new();
        for (_, (id, name, args)) in tool_calls_map {
            tool_calls.push(ToolCall {
                id: id.unwrap_or_else(|| uuid::Uuid::new_v4().to_string()),
                name: name.unwrap_or_default(),
                arguments: args,
            });
        }

        let thinking_opt = if thinking.is_empty() {
            None
        } else {
            Some(thinking)
        };

        Ok((content, thinking_opt, tool_calls))
    }
}

// ==========================================
// Unit Tests
// ==========================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_openrouter_headers_construction() {
        let client = OpenRouterClient::new(Some("sk-or-test-key-12345"))
            .with_referer("https://mycustomapp.org")
            .with_title("Custom Fusion App");

        let headers = client.build_headers();

        assert_eq!(
            headers.get(CONTENT_TYPE).unwrap().to_str().unwrap(),
            "application/json"
        );
        assert_eq!(
            headers.get(AUTHORIZATION).unwrap().to_str().unwrap(),
            "Bearer sk-or-test-key-12345"
        );
        assert_eq!(
            headers.get("http-referer").unwrap().to_str().unwrap(),
            "https://mycustomapp.org"
        );
        assert_eq!(
            headers.get("x-title").unwrap().to_str().unwrap(),
            "Custom Fusion App"
        );
    }

    #[test]
    fn test_openrouter_default_headers() {
        let client = OpenRouterClient::new(None::<String>);
        let headers = client.build_headers();

        assert_eq!(
            headers.get(CONTENT_TYPE).unwrap().to_str().unwrap(),
            "application/json"
        );
        assert!(headers.get(AUTHORIZATION).is_none());
        assert_eq!(
            headers.get("http-referer").unwrap().to_str().unwrap(),
            OPENROUTER_DEFAULT_REFERER
        );
        assert_eq!(
            headers.get("x-title").unwrap().to_str().unwrap(),
            OPENROUTER_DEFAULT_TITLE
        );
    }

    #[test]
    fn test_chat_payload_custom_provider_flags() {
        let prefs = OpenRouterProviderPreferences {
            order: Some(vec!["Anthropic".to_string(), "OpenAI".to_string()]),
            allow_fallbacks: Some(false),
            require_parameters: Some(true),
            data_collection: Some("deny".to_string()),
            ignore: Some(vec!["Together".to_string()]),
            quantizations: Some(vec!["fp16".to_string(), "bf16".to_string()]),
            sort: Some("price".to_string()),
            max_price: Some(OpenRouterMaxPrice {
                prompt: Some(0.000005),
                completion: Some(0.000015),
                image: None,
                request: None,
            }),
        };

        let options = OpenRouterRequestOptions {
            models: Some(vec![
                "anthropic/claude-3.5-sonnet".to_string(),
                "openai/gpt-4o".to_string(),
            ]),
            route: Some("fallback".to_string()),
            transforms: Some(vec!["middle-out".to_string()]),
            provider: Some(prefs),
            plugins: Some(vec![OpenRouterPlugin {
                id: "web-search".to_string(),
                max_results: Some(5),
                search_prompt: None,
            }]),
            include_reasoning: Some(true),
            repetition_penalty: Some(1.1),
            top_k: Some(40),
            min_p: Some(0.05),
            top_a: Some(0.8),
            seed: Some(42),
        };

        let client = OpenRouterClient::new(Some("test-key")).with_request_options(options);

        let messages = vec![
            Message::system("You are a helpful coding assistant."),
            Message::user("Hello OpenRouter!"),
        ];

        let tools = vec![ToolDefinition {
            name: "read_file".to_string(),
            description: "Read a local file".to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string" }
                },
                "required": ["path"]
            }),
        }];

        let payload = client.build_chat_payload(
            "anthropic/claude-3.5-sonnet",
            Some(0.2),
            Some(4096),
            &messages,
            &tools,
            true,
            None,
        );

        assert_eq!(payload["model"], "anthropic/claude-3.5-sonnet");
        assert!((payload["temperature"].as_f64().unwrap() - 0.2).abs() < 1e-4);
        assert_eq!(payload["max_tokens"], 4096);
        assert_eq!(payload["stream"], true);
        assert_eq!(payload["stream_options"]["include_usage"], true);
        assert_eq!(payload["route"], "fallback");
        assert_eq!(payload["transforms"][0], "middle-out");
        assert_eq!(payload["include_reasoning"], true);
        assert!((payload["repetition_penalty"].as_f64().unwrap() - 1.1).abs() < 1e-4);
        assert_eq!(payload["top_k"], 40);
        assert!((payload["min_p"].as_f64().unwrap() - 0.05).abs() < 1e-4);
        assert!((payload["top_a"].as_f64().unwrap() - 0.8).abs() < 1e-4);
        assert_eq!(payload["seed"], 42);

        // Check fallback models
        let models_arr = payload["models"].as_array().unwrap();
        assert_eq!(models_arr.len(), 2);
        assert_eq!(models_arr[0], "anthropic/claude-3.5-sonnet");
        assert_eq!(models_arr[1], "openai/gpt-4o");

        // Check provider routing preferences
        let provider_obj = &payload["provider"];
        assert_eq!(provider_obj["allow_fallbacks"], false);
        assert_eq!(provider_obj["require_parameters"], true);
        assert_eq!(provider_obj["data_collection"], "deny");
        assert_eq!(provider_obj["sort"], "price");
        assert_eq!(provider_obj["order"][0], "Anthropic");
        assert_eq!(provider_obj["order"][1], "OpenAI");
        assert_eq!(provider_obj["ignore"][0], "Together");
        assert_eq!(provider_obj["quantizations"][0], "fp16");

        // Check tools & messages
        assert_eq!(payload["tools"].as_array().unwrap().len(), 1);
        assert_eq!(payload["messages"].as_array().unwrap().len(), 2);
    }

    #[test]
    fn test_model_list_parsing() {
        let sample_json = r#"{
            "data": [
                {
                    "id": "anthropic/claude-3.5-sonnet",
                    "name": "Anthropic: Claude 3.5 Sonnet",
                    "created": 1718841600,
                    "description": "Claude 3.5 Sonnet delivers better-than-Opus capabilities.",
                    "context_length": 200000,
                    "pricing": {
                        "prompt": "0.000003",
                        "completion": "0.000015",
                        "image": "0.0048",
                        "request": "0"
                    },
                    "top_provider": {
                        "context_length": 200000,
                        "max_completion_tokens": 8192,
                        "is_moderated": false
                    },
                    "architecture": {
                        "modality": "text+image->text",
                        "tokenizer": "Claude",
                        "instruct_type": null
                    },
                    "per_request_limits": {
                        "prompt_tokens": 200000,
                        "completion_tokens": 8192
                    }
                },
                {
                    "id": "meta-llama/llama-3.3-70b-instruct:free",
                    "name": "Meta: Llama 3.3 70B Instruct (free)",
                    "created": 1733443200,
                    "description": "Free tier Llama 3.3 70B.",
                    "context_length": 131072,
                    "pricing": {
                        "prompt": "0",
                        "completion": "0"
                    },
                    "architecture": {
                        "modality": "text->text"
                    }
                }
            ]
        }"#;

        let response: OpenRouterModelListResponse = serde_json::from_str(sample_json).unwrap();
        assert_eq!(response.data.len(), 2);

        let claude = &response.data[0];
        assert_eq!(claude.id, "anthropic/claude-3.5-sonnet");
        assert_eq!(claude.display_name(), "Anthropic: Claude 3.5 Sonnet");
        assert_eq!(claude.context_length, Some(200000));
        assert_eq!(claude.prompt_price_per_million(), Some(3.0));
        assert_eq!(claude.completion_price_per_million(), Some(15.0));
        assert!(claude.supports_vision());
        assert!(!claude.is_free());

        let free_llama = &response.data[1];
        assert_eq!(free_llama.id, "meta-llama/llama-3.3-70b-instruct:free");
        assert!(free_llama.is_free());
        assert!(!free_llama.supports_vision());
        assert_eq!(free_llama.prompt_price_per_million(), Some(0.0));
    }

    #[test]
    fn test_key_info_parsing() {
        let sample_json = r#"{
            "data": {
                "label": "My Test Key",
                "usage": 12.345,
                "limit": 50.0,
                "is_free_tier": false,
                "rate_limit": {
                    "requests": 200,
                    "interval": "10s"
                }
            }
        }"#;

        let response: OpenRouterKeyInfoResponse = serde_json::from_str(sample_json).unwrap();
        assert_eq!(response.data.label.as_deref(), Some("My Test Key"));
        assert_eq!(response.data.usage, Some(12.345));
        assert_eq!(response.data.limit, Some(50.0));
        assert_eq!(response.data.is_free_tier, Some(false));
        assert_eq!(
            response.data.rate_limit,
            Some(OpenRouterRateLimit {
                requests: Some(200),
                interval: Some("10s".to_string()),
            })
        );
    }

    #[test]
    fn test_generation_stats_parsing() {
        let sample_json = r#"{
            "data": {
                "id": "gen-12345678",
                "model": "anthropic/claude-3.5-sonnet",
                "streamed": true,
                "generation_time": 1240,
                "created_at": "2026-09-02T12:00:00Z",
                "tokens_prompt": 120,
                "tokens_completion": 450,
                "native_tokens_prompt": 120,
                "native_tokens_completion": 450,
                "total_cost": 0.00711,
                "latency": 0.85,
                "finish_reason": "stop"
            }
        }"#;

        let response: OpenRouterGenerationResponse = serde_json::from_str(sample_json).unwrap();
        assert_eq!(response.data.id.as_deref(), Some("gen-12345678"));
        assert_eq!(response.data.model.as_deref(), Some("anthropic/claude-3.5-sonnet"));
        assert_eq!(response.data.tokens_prompt, Some(120));
        assert_eq!(response.data.tokens_completion, Some(450));
        assert_eq!(response.data.total_cost, Some(0.00711));
    }
}
