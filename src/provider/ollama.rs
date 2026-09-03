use std::collections::BTreeMap;
use std::time::Duration;

use anyhow::{Context, Result};
use eventsource_stream::Eventsource;
use futures::StreamExt;
use reqwest::header::{HeaderMap, HeaderValue, AUTHORIZATION, CONTENT_TYPE};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tokio::sync::mpsc;
use tracing::{debug, warn};

use crate::provider::types::{Message, Role, StreamChunk, ToolCall, ToolDefinition};

pub const DEFAULT_OLLAMA_BASE_URL: &str = "http://localhost:11434";

// ============================================================================
// Ollama API Data Structures
// ============================================================================

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OllamaModelDetails {
    #[serde(default)]
    pub parent_model: Option<String>,
    #[serde(default)]
    pub format: Option<String>,
    #[serde(default)]
    pub family: Option<String>,
    #[serde(default)]
    pub families: Option<Vec<String>>,
    #[serde(default)]
    pub parameter_size: Option<String>,
    #[serde(default)]
    pub quantization_level: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OllamaModelInfo {
    pub name: String,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub modified_at: Option<String>,
    #[serde(default)]
    pub size: Option<u64>,
    #[serde(default)]
    pub digest: Option<String>,
    #[serde(default)]
    pub details: Option<OllamaModelDetails>,
    #[serde(default)]
    pub expires_at: Option<String>,
    #[serde(default)]
    pub size_vram: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OllamaTagsResponse {
    #[serde(default)]
    pub models: Vec<OllamaModelInfo>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OllamaProcessModelInfo {
    pub name: String,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub size: Option<u64>,
    #[serde(default)]
    pub digest: Option<String>,
    #[serde(default)]
    pub details: Option<OllamaModelDetails>,
    #[serde(default)]
    pub expires_at: Option<String>,
    #[serde(default)]
    pub size_vram: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OllamaPsResponse {
    #[serde(default)]
    pub models: Vec<OllamaProcessModelInfo>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OllamaShowResponse {
    #[serde(default)]
    pub license: Option<String>,
    #[serde(default)]
    pub modelfile: Option<String>,
    #[serde(default)]
    pub parameters: Option<String>,
    #[serde(default)]
    pub template: Option<String>,
    #[serde(default)]
    pub system: Option<String>,
    #[serde(default)]
    pub details: Option<OllamaModelDetails>,
    #[serde(default)]
    pub model_info: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OllamaPullProgress {
    pub status: String,
    #[serde(default)]
    pub digest: Option<String>,
    #[serde(default)]
    pub total: Option<u64>,
    #[serde(default)]
    pub completed: Option<u64>,
}

// ----------------------------------------------------------------------------
// Chat Request Structures
// ----------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OllamaOptions {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub num_predict: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub top_p: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub top_k: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stop: Option<Vec<String>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OllamaFunctionCall {
    pub name: String,
    pub arguments: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OllamaToolCall {
    pub function: OllamaFunctionCall,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OllamaMessage {
    pub role: String,
    pub content: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub images: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_calls: Option<Vec<OllamaToolCall>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OllamaToolFunction {
    pub name: String,
    pub description: String,
    pub parameters: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OllamaTool {
    #[serde(rename = "type")]
    pub tool_type: String,
    pub function: OllamaToolFunction,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OllamaChatRequest {
    pub model: String,
    pub messages: Vec<OllamaMessage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tools: Option<Vec<OllamaTool>>,
    pub stream: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub format: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub options: Option<OllamaOptions>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub keep_alive: Option<String>,
}

// ----------------------------------------------------------------------------
// Chat Response Structures (NDJSON Streaming)
// ----------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OllamaResponseFunction {
    pub name: String,
    #[serde(default)]
    pub arguments: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OllamaResponseToolCall {
    pub function: OllamaResponseFunction,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OllamaResponseMessage {
    #[serde(default)]
    pub role: Option<String>,
    #[serde(default)]
    pub content: Option<String>,
    #[serde(default)]
    pub thinking: Option<String>,
    #[serde(default)]
    pub reasoning_content: Option<String>,
    #[serde(default)]
    pub tool_calls: Option<Vec<OllamaResponseToolCall>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OllamaChatChunk {
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub created_at: Option<String>,
    #[serde(default)]
    pub message: Option<OllamaResponseMessage>,
    #[serde(default)]
    pub done: bool,
    #[serde(default)]
    pub done_reason: Option<String>,
    #[serde(default)]
    pub total_duration: Option<u64>,
    #[serde(default)]
    pub load_duration: Option<u64>,
    #[serde(default)]
    pub prompt_eval_count: Option<u32>,
    #[serde(default)]
    pub prompt_eval_duration: Option<u64>,
    #[serde(default)]
    pub eval_count: Option<u32>,
    #[serde(default)]
    pub eval_duration: Option<u64>,
    #[serde(default)]
    pub error: Option<String>,
}

// ============================================================================
// Ollama Client Implementation
// ============================================================================

#[derive(Clone, Debug)]
pub struct OllamaClient {
    client: reqwest::Client,
    base_url: String,
}

impl Default for OllamaClient {
    fn default() -> Self {
        Self::new(None)
    }
}

impl OllamaClient {
    pub fn new(base_url: Option<&str>) -> Self {
        let client = reqwest::Client::builder()
            .use_rustls_tls()
            .connect_timeout(Duration::from_secs(15))
            .timeout(Duration::from_secs(300))
            .build()
            .unwrap_or_else(|_| reqwest::Client::new());

        let url = base_url
            .unwrap_or(DEFAULT_OLLAMA_BASE_URL)
            .trim_end_matches('/')
            .to_string();

        Self {
            client,
            base_url: url,
        }
    }

    pub fn with_client(client: reqwest::Client, base_url: Option<&str>) -> Self {
        let url = base_url
            .unwrap_or(DEFAULT_OLLAMA_BASE_URL)
            .trim_end_matches('/')
            .to_string();

        Self {
            client,
            base_url: url,
        }
    }

    pub fn base_url(&self) -> &str {
        &self.base_url
    }

    pub fn set_base_url(&mut self, base_url: impl Into<String>) {
        self.base_url = base_url.into().trim_end_matches('/').to_string();
    }

    // ========================================================================
    // Model Discovery and Management APIs
    // ========================================================================

    /// Ping the Ollama server to verify if it is reachable.
    pub async fn ping(&self) -> bool {
        let url = format!("{}/api/tags", self.base_url);
        match self
            .client
            .get(&url)
            .timeout(Duration::from_secs(5))
            .send()
            .await
        {
            Ok(resp) => resp.status().is_success(),
            Err(_) => false,
        }
    }

    /// List all locally downloaded/available models from Ollama (`GET /api/tags`).
    pub async fn list_models(&self) -> Result<Vec<OllamaModelInfo>> {
        let url = format!("{}/api/tags", self.base_url);
        let resp = self
            .client
            .get(&url)
            .send()
            .await
            .with_context(|| format!("Failed to connect to Ollama at {}", url))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            anyhow::bail!("Ollama list_models failed ({status}): {body}");
        }

        let tags: OllamaTagsResponse = resp
            .json()
            .await
            .with_context(|| "Failed to parse Ollama model tags response")?;

        Ok(tags.models)
    }

    /// List currently loaded/running models in VRAM (`GET /api/ps`).
    pub async fn list_running_models(&self) -> Result<Vec<OllamaProcessModelInfo>> {
        let url = format!("{}/api/ps", self.base_url);
        let resp = self
            .client
            .get(&url)
            .send()
            .await
            .with_context(|| format!("Failed to query Ollama running processes at {}", url))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            anyhow::bail!("Ollama list_running_models failed ({status}): {body}");
        }

        let ps: OllamaPsResponse = resp
            .json()
            .await
            .with_context(|| "Failed to parse Ollama ps response")?;

        Ok(ps.models)
    }

    /// Retrieve detailed information about a specific model (`POST /api/show`).
    pub async fn show_model(&self, model: &str) -> Result<OllamaShowResponse> {
        let url = format!("{}/api/show", self.base_url);
        let payload = json!({ "name": model });

        let resp = self
            .client
            .post(&url)
            .json(&payload)
            .send()
            .await
            .with_context(|| format!("Failed to query model details for {model}"))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            anyhow::bail!("Ollama show_model failed ({status}): {body}");
        }

        let info: OllamaShowResponse = resp
            .json()
            .await
            .with_context(|| "Failed to parse Ollama show response")?;

        Ok(info)
    }

    /// Check if a model is installed locally in Ollama.
    pub async fn has_model(&self, model_name: &str) -> Result<bool> {
        let models = self.list_models().await?;
        let target = model_name.trim();
        let target_with_tag = if !target.contains(':') {
            format!("{}:latest", target)
        } else {
            target.to_string()
        };

        for m in models {
            if m.name.eq_ignore_ascii_case(target)
                || m.name.eq_ignore_ascii_case(&target_with_tag)
                || m.model
                    .as_deref()
                    .unwrap_or("")
                    .eq_ignore_ascii_case(target)
                || m.model
                    .as_deref()
                    .unwrap_or("")
                    .eq_ignore_ascii_case(&target_with_tag)
            {
                return Ok(true);
            }
        }
        Ok(false)
    }

    /// Pull a model from Ollama library with streaming progress updates (`POST /api/pull`).
    pub async fn pull_model(&self, model_name: &str) -> Result<mpsc::Receiver<OllamaPullProgress>> {
        let url = format!("{}/api/pull", self.base_url);
        let payload = json!({ "name": model_name, "stream": true });

        let response = self
            .client
            .post(&url)
            .json(&payload)
            .send()
            .await
            .with_context(|| format!("Failed to initiate pull for {model_name}"))?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            anyhow::bail!("Ollama pull failed ({status}): {body}");
        }

        let (tx, rx) = mpsc::channel(64);
        let mut stream = response.bytes_stream();

        tokio::spawn(async move {
            let mut buffer = String::new();
            while let Some(chunk_res) = stream.next().await {
                match chunk_res {
                    Ok(bytes) => {
                        if let Ok(text) = std::str::from_utf8(&bytes) {
                            buffer.push_str(text);
                            while let Some(newline_pos) = buffer.find('\n') {
                                let line = buffer[..newline_pos].trim().to_string();
                                buffer.drain(..=newline_pos);
                                if !line.is_empty() {
                                    if let Ok(progress) =
                                        serde_json::from_str::<OllamaPullProgress>(&line)
                                    {
                                        if tx.send(progress).await.is_err() {
                                            return;
                                        }
                                    }
                                }
                            }
                        }
                    }
                    Err(e) => {
                        let _ = tx
                            .send(OllamaPullProgress {
                                status: format!("error: {e}"),
                                digest: None,
                                total: None,
                                completed: None,
                            })
                            .await;
                        break;
                    }
                }
            }
        });

        Ok(rx)
    }

    // ========================================================================
    // Chat Completion Streaming (Native /api/chat with OpenAI Fallback)
    // ========================================================================

    /// Stream a chat completion. Tries native `/api/chat` first; if that endpoint
    /// returns 404 Not Found, automatically falls back to `/v1/chat/completions`.
    #[allow(clippy::too_many_arguments)]
    pub async fn stream_chat(
        &self,
        model: &str,
        temperature: Option<f32>,
        max_tokens: Option<u32>,
        api_key: Option<&str>,
        messages: &[Message],
        tools: &[ToolDefinition],
    ) -> Result<mpsc::Receiver<StreamChunk>> {
        // Attempt native /api/chat
        match self
            .stream_chat_native(model, temperature, max_tokens, messages, tools)
            .await
        {
            Ok(rx) => Ok(rx),
            Err(e) => {
                let err_msg = e.to_string();
                if err_msg.contains("404") || err_msg.contains("Not Found") {
                    debug!("Ollama native /api/chat returned 404, attempting OpenAI compatible fallback");
                    self.stream_chat_openai_fallback(
                        model,
                        temperature,
                        max_tokens,
                        api_key,
                        messages,
                        tools,
                    )
                    .await
                } else {
                    Err(e)
                }
            }
        }
    }

    /// Stream chat completion using native Ollama `/api/chat` NDJSON format.
    pub async fn stream_chat_native(
        &self,
        model: &str,
        temperature: Option<f32>,
        max_tokens: Option<u32>,
        messages: &[Message],
        tools: &[ToolDefinition],
    ) -> Result<mpsc::Receiver<StreamChunk>> {
        let url = format!("{}/api/chat", self.base_url);

        let ollama_messages: Vec<OllamaMessage> = messages.iter().map(to_ollama_message).collect();

        let ollama_tools: Option<Vec<OllamaTool>> = if tools.is_empty() {
            None
        } else {
            Some(tools.iter().map(to_ollama_tool).collect())
        };

        let options = if temperature.is_some() || max_tokens.is_some() {
            Some(OllamaOptions {
                temperature,
                num_predict: max_tokens,
                top_p: None,
                top_k: None,
                stop: None,
            })
        } else {
            None
        };

        let request_body = OllamaChatRequest {
            model: model.to_string(),
            messages: ollama_messages,
            tools: ollama_tools,
            stream: true,
            format: None,
            options,
            keep_alive: None,
        };

        let response = self
            .client
            .post(&url)
            .header(CONTENT_TYPE, "application/json")
            .json(&request_body)
            .send()
            .await
            .with_context(|| format!("Failed to send request to Ollama /api/chat at {}", url))?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            anyhow::bail!("Ollama /api/chat failed ({status}): {body}");
        }

        let (tx, rx) = mpsc::channel(256);
        let mut stream = response.bytes_stream();

        tokio::spawn(async move {
            let mut buffer = String::new();
            let mut prompt_tokens = None;
            let mut completion_tokens = None;
            let mut finish_reason = None;
            let mut done_sent = false;

            // State for inline <think> tags parsing
            let mut think_parser = ThinkTagParser::new();

            while let Some(chunk_res) = stream.next().await {
                match chunk_res {
                    Ok(bytes) => {
                        if let Ok(text) = std::str::from_utf8(&bytes) {
                            buffer.push_str(text);

                            while let Some(newline_pos) = buffer.find('\n') {
                                let line = buffer[..newline_pos].trim().to_string();
                                buffer.drain(..=newline_pos);

                                if line.is_empty() {
                                    continue;
                                }

                                match serde_json::from_str::<OllamaChatChunk>(&line) {
                                    Ok(chunk) => {
                                        if let Some(err) = chunk.error {
                                            let _ = tx.send(StreamChunk::Error(err)).await;
                                            return;
                                        }

                                        if let Some(pt) = chunk.prompt_eval_count {
                                            prompt_tokens = Some(pt);
                                        }
                                        if let Some(ct) = chunk.eval_count {
                                            completion_tokens = Some(ct);
                                        }
                                        if let Some(reason) = &chunk.done_reason {
                                            finish_reason = Some(reason.clone());
                                        }

                                        if let Some(msg) = chunk.message {
                                            // Handle dedicated reasoning/thinking fields (e.g. DeepSeek R1 in Ollama)
                                            let reasoning = msg
                                                .thinking
                                                .or(msg.reasoning_content)
                                                .unwrap_or_default();

                                            if !reasoning.is_empty() {
                                                let _ = tx
                                                    .send(StreamChunk::ThinkingDelta(reasoning))
                                                    .await;
                                            }

                                            // Handle content and inline <think> tags
                                            if let Some(content) = msg.content {
                                                if !content.is_empty() {
                                                    let segments = think_parser.feed(&content);
                                                    for segment in segments {
                                                        match segment {
                                                            ParsedSegment::Content(c) => {
                                                                let _ = tx
                                                                    .send(
                                                                        StreamChunk::ContentDelta(
                                                                            c,
                                                                        ),
                                                                    )
                                                                    .await;
                                                            }
                                                            ParsedSegment::Thinking(t) => {
                                                                let _ = tx
                                                                    .send(
                                                                        StreamChunk::ThinkingDelta(
                                                                            t,
                                                                        ),
                                                                    )
                                                                    .await;
                                                            }
                                                        }
                                                    }
                                                }
                                            }

                                            // Handle tool calls
                                            if let Some(tool_calls) = msg.tool_calls {
                                                for (idx, tc) in tool_calls.into_iter().enumerate()
                                                {
                                                    let args_str =
                                                        if tc.function.arguments.is_string() {
                                                            tc.function
                                                                .arguments
                                                                .as_str()
                                                                .unwrap_or("")
                                                                .to_string()
                                                        } else {
                                                            tc.function.arguments.to_string()
                                                        };

                                                    let _ = tx
                                                        .send(StreamChunk::ToolCallDelta {
                                                            index: idx,
                                                            id: Some(
                                                                uuid::Uuid::new_v4().to_string(),
                                                            ),
                                                            name: Some(tc.function.name),
                                                            arguments_delta: args_str,
                                                        })
                                                        .await;
                                                }
                                            }
                                        }

                                        if chunk.done {
                                            // Flush any remaining thinking buffer
                                            if let Some(remaining) = think_parser.flush() {
                                                match remaining {
                                                    ParsedSegment::Content(c) => {
                                                        let _ = tx
                                                            .send(StreamChunk::ContentDelta(c))
                                                            .await;
                                                    }
                                                    ParsedSegment::Thinking(t) => {
                                                        let _ = tx
                                                            .send(StreamChunk::ThinkingDelta(t))
                                                            .await;
                                                    }
                                                }
                                            }

                                            let _ = tx
                                                .send(StreamChunk::Done {
                                                    finish_reason: finish_reason.take(),
                                                    prompt_tokens,
                                                    completion_tokens,
                                                })
                                                .await;
                                            done_sent = true;
                                            break;
                                        }
                                    }
                                    Err(parse_err) => {
                                        warn!(
                                            "Failed to parse Ollama NDJSON line '{}': {}",
                                            line, parse_err
                                        );
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

            // Handle trailing buffer without newline
            let remainder = buffer.trim();
            if !remainder.is_empty() {
                if let Ok(chunk) = serde_json::from_str::<OllamaChatChunk>(remainder) {
                    if let Some(msg) = chunk.message {
                        if let Some(content) = msg.content {
                            if !content.is_empty() {
                                let _ = tx.send(StreamChunk::ContentDelta(content)).await;
                            }
                        }
                    }
                }
            }

            if !done_sent {
                let _ = tx
                    .send(StreamChunk::Done {
                        finish_reason,
                        prompt_tokens,
                        completion_tokens,
                    })
                    .await;
            }
        });

        Ok(rx)
    }

    /// Stream chat completion using OpenAI compatibility mode (`/v1/chat/completions`).
    #[allow(clippy::too_many_arguments)]
    pub async fn stream_chat_openai_fallback(
        &self,
        model: &str,
        temperature: Option<f32>,
        max_tokens: Option<u32>,
        api_key: Option<&str>,
        messages: &[Message],
        tools: &[ToolDefinition],
    ) -> Result<mpsc::Receiver<StreamChunk>> {
        let url = construct_ollama_openai_url(&self.base_url);
        let mut headers = HeaderMap::new();
        headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));

        if let Some(key) = api_key {
            let trimmed = key.trim();
            if !trimmed.is_empty() {
                if let Ok(hv) = HeaderValue::from_str(&format!("Bearer {}", trimmed)) {
                    headers.insert(AUTHORIZATION, hv);
                }
            }
        }

        let mut payload = json!({
            "model": model,
            "stream": true,
            "stream_options": { "include_usage": true }
        });

        if let Some(temp) = temperature {
            payload["temperature"] = json!(temp);
        }
        if let Some(mt) = max_tokens {
            payload["max_tokens"] = json!(mt);
        }

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
                        let tc_json: Vec<Value> = tool_calls
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
                        item["tool_calls"] = json!(tc_json);
                    }
                }
            }

            if msg.role == Role::Tool {
                if let Some(id) = &msg.tool_call_id {
                    item["tool_call_id"] = json!(id);
                }
            }

            messages_json.push(item);
        }
        payload["messages"] = json!(messages_json);

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
        }

        let response = self
            .client
            .post(&url)
            .headers(headers)
            .json(&payload)
            .send()
            .await
            .with_context(|| format!("Failed to send OpenAI fallback request to {}", url))?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            anyhow::bail!("Ollama OpenAI fallback to {url} failed ({status}): {body}");
        }

        let (tx, rx) = mpsc::channel(256);
        let mut stream = response.bytes_stream().eventsource();

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
                                        finish_reason: None,
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

                            if let Some(choices) = val.get("choices").and_then(|v| v.as_array()) {
                                for choice in choices {
                                    let finish_reason = choice
                                        .get("finish_reason")
                                        .and_then(|v| v.as_str())
                                        .map(|s| s.to_string());

                                    if let Some(delta) = choice.get("delta") {
                                        if let Some(reasoning) = delta
                                            .get("reasoning_content")
                                            .or_else(|| delta.get("reasoning"))
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

    /// Aggregate a stream into full text, thinking, and tool calls.
    pub async fn complete(
        &self,
        model: &str,
        temperature: Option<f32>,
        max_tokens: Option<u32>,
        api_key: Option<&str>,
        messages: &[Message],
        tools: &[ToolDefinition],
    ) -> Result<(String, Option<String>, Vec<ToolCall>)> {
        let mut rx = self
            .stream_chat(model, temperature, max_tokens, api_key, messages, tools)
            .await?;
        Self::aggregate_stream(&mut rx).await
    }

    async fn aggregate_stream(
        rx: &mut mpsc::Receiver<StreamChunk>,
    ) -> Result<(String, Option<String>, Vec<ToolCall>)> {
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
                    anyhow::bail!("Ollama stream error: {}", err);
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

// ============================================================================
// Helper Conversions & Functions
// ============================================================================

pub fn construct_ollama_openai_url(base_url: &str) -> String {
    let base = base_url.trim_end_matches('/');
    if base.ends_with("/chat/completions") {
        base.to_string()
    } else if base.ends_with("/v1") {
        format!("{}/chat/completions", base)
    } else {
        format!("{}/v1/chat/completions", base)
    }
}

pub fn to_ollama_message(msg: &Message) -> OllamaMessage {
    let role = match msg.role {
        Role::System => "system",
        Role::User => "user",
        Role::Assistant => "assistant",
        Role::Tool => "tool",
    }
    .to_string();

    let tool_calls = msg.tool_calls.as_ref().map(|tcs| {
        tcs.iter()
            .map(|tc| {
                let args_val: Value =
                    serde_json::from_str(&tc.arguments).unwrap_or_else(|_| json!({}));
                OllamaToolCall {
                    function: OllamaFunctionCall {
                        name: tc.name.clone(),
                        arguments: args_val,
                    },
                }
            })
            .collect()
    });

    OllamaMessage {
        role,
        content: msg.content.clone(),
        images: None,
        tool_calls,
    }
}

pub fn to_ollama_tool(tool: &ToolDefinition) -> OllamaTool {
    OllamaTool {
        tool_type: "function".to_string(),
        function: OllamaToolFunction {
            name: tool.name.clone(),
            description: tool.description.clone(),
            parameters: tool.parameters.clone(),
        },
    }
}

// ============================================================================
// Inline <think> Tag Parser
// ============================================================================

#[derive(Debug, PartialEq, Eq)]
pub enum ParsedSegment {
    Content(String),
    Thinking(String),
}

#[derive(Debug, Default)]
pub struct ThinkTagParser {
    in_think: bool,
    pending: String,
}

impl ThinkTagParser {
    pub fn new() -> Self {
        Self {
            in_think: false,
            pending: String::new(),
        }
    }

    pub fn feed(&mut self, text: &str) -> Vec<ParsedSegment> {
        let mut segments = Vec::new();
        self.pending.push_str(text);

        while !self.pending.is_empty() {
            if self.in_think {
                if let Some(pos) = self.pending.find("</think>") {
                    let think_text = &self.pending[..pos];
                    if !think_text.is_empty() {
                        segments.push(ParsedSegment::Thinking(think_text.to_string()));
                    }
                    self.pending = self.pending[pos + 8..].to_string();
                    self.in_think = false;
                } else {
                    // Check if a prefix of "</think>" is at the end of pending
                    let mut partial_len = 0;
                    for i in 1..8.min(self.pending.len() + 1) {
                        if "</think>".starts_with(&self.pending[self.pending.len() - i..]) {
                            partial_len = i;
                        }
                    }

                    if partial_len > 0 {
                        let think_len = self.pending.len() - partial_len;
                        if think_len > 0 {
                            let think_text = self.pending[..think_len].to_string();
                            segments.push(ParsedSegment::Thinking(think_text));
                            self.pending = self.pending[think_len..].to_string();
                        }
                        break;
                    } else {
                        segments.push(ParsedSegment::Thinking(std::mem::take(&mut self.pending)));
                    }
                }
            } else if let Some(pos) = self.pending.find("<think>") {
                let content_text = &self.pending[..pos];
                if !content_text.is_empty() {
                    segments.push(ParsedSegment::Content(content_text.to_string()));
                }
                self.pending = self.pending[pos + 7..].to_string();
                self.in_think = true;
            } else {
                // Check if a prefix of "<think>" is at the end of pending
                let mut partial_len = 0;
                for i in 1..7.min(self.pending.len() + 1) {
                    if "<think>".starts_with(&self.pending[self.pending.len() - i..]) {
                        partial_len = i;
                    }
                }

                if partial_len > 0 {
                    let content_len = self.pending.len() - partial_len;
                    if content_len > 0 {
                        let content_text = self.pending[..content_len].to_string();
                        segments.push(ParsedSegment::Content(content_text));
                        self.pending = self.pending[content_len..].to_string();
                    }
                    break;
                } else {
                    segments.push(ParsedSegment::Content(std::mem::take(&mut self.pending)));
                }
            }
        }

        segments
    }

    pub fn flush(&mut self) -> Option<ParsedSegment> {
        if self.pending.is_empty() {
            None
        } else {
            let text = std::mem::take(&mut self.pending);
            if self.in_think {
                Some(ParsedSegment::Thinking(text))
            } else {
                Some(ParsedSegment::Content(text))
            }
        }
    }
}

// ============================================================================
// Unit Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_construct_ollama_openai_url() {
        assert_eq!(
            construct_ollama_openai_url("http://localhost:11434"),
            "http://localhost:11434/v1/chat/completions"
        );
        assert_eq!(
            construct_ollama_openai_url("http://localhost:11434/"),
            "http://localhost:11434/v1/chat/completions"
        );
        assert_eq!(
            construct_ollama_openai_url("http://localhost:11434/v1"),
            "http://localhost:11434/v1/chat/completions"
        );
        assert_eq!(
            construct_ollama_openai_url("http://localhost:11434/v1/chat/completions"),
            "http://localhost:11434/v1/chat/completions"
        );
    }

    #[test]
    fn test_to_ollama_message_and_tool() {
        let msg = Message::user("Hello Ollama");
        let ollama_msg = to_ollama_message(&msg);
        assert_eq!(ollama_msg.role, "user");
        assert_eq!(ollama_msg.content, "Hello Ollama");
        assert!(ollama_msg.tool_calls.is_none());

        let tool_call = ToolCall {
            id: "call_123".to_string(),
            name: "read_file".to_string(),
            arguments: r#"{"path":"test.txt"}"#.to_string(),
        };
        let assistant_msg = Message::assistant_with_tools("", vec![tool_call]);
        let ollama_assistant = to_ollama_message(&assistant_msg);
        assert_eq!(ollama_assistant.role, "assistant");
        let tcs = ollama_assistant.tool_calls.expect("Expected tool calls");
        assert_eq!(tcs.len(), 1);
        assert_eq!(tcs[0].function.name, "read_file");
        assert_eq!(tcs[0].function.arguments["path"], "test.txt");

        let tool_def = ToolDefinition {
            name: "bash".to_string(),
            description: "Run bash command".to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "command": { "type": "string" }
                }
            }),
        };
        let ollama_tool = to_ollama_tool(&tool_def);
        assert_eq!(ollama_tool.tool_type, "function");
        assert_eq!(ollama_tool.function.name, "bash");
        assert_eq!(ollama_tool.function.description, "Run bash command");
    }

    #[test]
    fn test_parse_ollama_chunk_ndjson() {
        let chunk_json = r#"{"model":"llama3","created_at":"2023-08-04T19:22:45.499127Z","message":{"role":"assistant","content":"Hello world"},"done":false}"#;
        let chunk: OllamaChatChunk = serde_json::from_str(chunk_json).unwrap();
        assert!(!chunk.done);
        let msg = chunk.message.unwrap();
        assert_eq!(msg.content.unwrap(), "Hello world");

        let done_json = r#"{"model":"llama3","created_at":"2023-08-04T19:22:45.499127Z","message":{"role":"assistant","content":""},"done":true,"done_reason":"stop","prompt_eval_count":12,"eval_count":45}"#;
        let done_chunk: OllamaChatChunk = serde_json::from_str(done_json).unwrap();
        assert!(done_chunk.done);
        assert_eq!(done_chunk.done_reason.as_deref(), Some("stop"));
        assert_eq!(done_chunk.prompt_eval_count, Some(12));
        assert_eq!(done_chunk.eval_count, Some(45));
    }

    #[test]
    fn test_think_tag_parser() {
        let mut parser = ThinkTagParser::new();

        // Standard text without thinking
        let res1 = parser.feed("Hello there! ");
        assert_eq!(
            res1,
            vec![ParsedSegment::Content("Hello there! ".to_string())]
        );

        // Beginning think tag
        let res2 = parser.feed("<think>I should check files");
        assert_eq!(
            res2,
            vec![ParsedSegment::Thinking("I should check files".to_string())]
        );

        // Ending think tag and content
        let res3 = parser.feed(" right now.</think>Here is the file.");
        assert_eq!(
            res3,
            vec![
                ParsedSegment::Thinking(" right now.".to_string()),
                ParsedSegment::Content("Here is the file.".to_string())
            ]
        );

        assert!(parser.flush().is_none());
    }

    #[test]
    fn test_think_tag_split_across_chunks() {
        let mut parser = ThinkTagParser::new();

        let r1 = parser.feed("Before <thi");
        assert_eq!(r1, vec![ParsedSegment::Content("Before ".to_string())]);

        let r2 = parser.feed("nk>Inside thought");
        assert_eq!(
            r2,
            vec![ParsedSegment::Thinking("Inside thought".to_string())]
        );

        let r3 = parser.feed("</th");
        assert!(r3.is_empty());

        let r4 = parser.feed("ink>After");
        assert_eq!(r4, vec![ParsedSegment::Content("After".to_string())]);
    }

    #[test]
    fn test_ollama_tags_response_deserialization() {
        let tags_json = r#"{
            "models": [
                {
                    "name": "llama3:latest",
                    "model": "llama3:latest",
                    "modified_at": "2024-04-18T19:22:45.499127Z",
                    "size": 4661224676,
                    "digest": "70e2c88864f7386302f800353644d50963b474eec5d429f1a2933e1ba46d4850",
                    "details": {
                        "parent_model": "",
                        "format": "gguf",
                        "family": "llama",
                        "families": ["llama"],
                        "parameter_size": "8.0B",
                        "quantization_level": "Q4_0"
                    }
                }
            ]
        }"#;

        let tags: OllamaTagsResponse = serde_json::from_str(tags_json).unwrap();
        assert_eq!(tags.models.len(), 1);
        assert_eq!(tags.models[0].name, "llama3:latest");
        assert_eq!(tags.models[0].size, Some(4661224676));
        let details = tags.models[0].details.as_ref().unwrap();
        assert_eq!(details.family.as_deref(), Some("llama"));
        assert_eq!(details.parameter_size.as_deref(), Some("8.0B"));
    }
}
