use std::collections::BTreeMap;
use std::time::Duration;

use anyhow::Context;
use eventsource_stream::Eventsource;
use futures::StreamExt;
use reqwest::header::{HeaderMap, HeaderValue, CONTENT_TYPE};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tokio::sync::mpsc;

use crate::provider::types::{Message, Role, StreamChunk, ToolCall, ToolDefinition};

pub const DEFAULT_ANTHROPIC_URL: &str = "https://api.anthropic.com/v1";
pub const DEFAULT_ANTHROPIC_VERSION: &str = "2023-06-01";
pub const DEFAULT_MAX_TOKENS: u32 = 8192;

// ============================================================================
// Anthropic Request & Response Types
// ============================================================================

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AnthropicRequest {
    pub model: String,
    pub max_tokens: u32,
    pub messages: Vec<AnthropicMessage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub system: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub top_p: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub top_k: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tools: Option<Vec<AnthropicTool>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_choice: Option<AnthropicToolChoice>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub thinking: Option<AnthropicThinking>,
    pub stream: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stop_sequences: Option<Vec<String>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum AnthropicRole {
    User,
    Assistant,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AnthropicMessage {
    pub role: AnthropicRole,
    pub content: AnthropicContent,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum AnthropicContent {
    Text(String),
    Blocks(Vec<AnthropicContentBlock>),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum AnthropicContentBlock {
    #[serde(rename = "text")]
    Text {
        text: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        cache_control: Option<AnthropicCacheControl>,
    },
    #[serde(rename = "image")]
    Image { source: AnthropicImageSource },
    #[serde(rename = "tool_use")]
    ToolUse {
        id: String,
        name: String,
        input: Value,
    },
    #[serde(rename = "tool_result")]
    ToolResult {
        tool_use_id: String,
        content: Value,
        #[serde(skip_serializing_if = "Option::is_none")]
        is_error: Option<bool>,
        #[serde(skip_serializing_if = "Option::is_none")]
        cache_control: Option<AnthropicCacheControl>,
    },
    #[serde(rename = "thinking")]
    Thinking {
        thinking: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        signature: Option<String>,
    },
    #[serde(rename = "redacted_thinking")]
    RedactedThinking { data: String },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AnthropicCacheControl {
    #[serde(rename = "type")]
    pub control_type: String, // "ephemeral"
}

impl AnthropicCacheControl {
    pub fn ephemeral() -> Self {
        Self {
            control_type: "ephemeral".to_string(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AnthropicImageSource {
    #[serde(rename = "type")]
    pub source_type: String, // "base64"
    pub media_type: String, // e.g. "image/jpeg", "image/png", "image/gif", "image/webp"
    pub data: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AnthropicTool {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    pub input_schema: Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cache_control: Option<AnthropicCacheControl>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum AnthropicToolChoice {
    #[serde(rename = "auto")]
    Auto,
    #[serde(rename = "any")]
    Any,
    #[serde(rename = "tool")]
    Tool { name: String },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AnthropicThinking {
    #[serde(rename = "type")]
    pub thinking_type: String, // "enabled"
    pub budget_tokens: u32,
}

impl AnthropicThinking {
    pub fn enabled(budget_tokens: u32) -> Self {
        Self {
            thinking_type: "enabled".to_string(),
            budget_tokens,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AnthropicUsage {
    #[serde(default)]
    pub input_tokens: u32,
    #[serde(default)]
    pub output_tokens: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cache_creation_input_tokens: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cache_read_input_tokens: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AnthropicResponse {
    pub id: String,
    #[serde(rename = "type")]
    pub response_type: String,
    pub role: AnthropicRole,
    pub content: Vec<AnthropicContentBlock>,
    pub model: String,
    pub stop_reason: Option<String>,
    pub stop_sequence: Option<String>,
    pub usage: AnthropicUsage,
}

// ============================================================================
// Tool Call & Stream Accumulator
// ============================================================================

#[derive(Debug, Clone)]
pub enum BlockState {
    Text {
        text: String,
    },
    Thinking {
        thinking: String,
        signature: String,
    },
    ToolUse {
        id: String,
        name: String,
        json_buf: String,
    },
}

#[derive(Debug, Clone, Default)]
pub struct AnthropicStreamAccumulator {
    pub blocks: BTreeMap<usize, BlockState>,
    pub prompt_tokens: Option<u32>,
    pub completion_tokens: Option<u32>,
    pub finish_reason: Option<String>,
}

impl AnthropicStreamAccumulator {
    pub fn new() -> Self {
        Self::default()
    }

    /// Process a raw Anthropic SSE JSON event and optionally return a `StreamChunk`
    pub fn process_event(&mut self, val: &Value) -> Vec<StreamChunk> {
        let mut chunks = Vec::new();
        let event_type = val.get("type").and_then(|v| v.as_str()).unwrap_or("");

        match event_type {
            "message_start" => {
                if let Some(msg) = val.get("message") {
                    if let Some(usage) = msg.get("usage") {
                        if let Some(it) = usage.get("input_tokens").and_then(|v| v.as_u64()) {
                            self.prompt_tokens = Some(it as u32);
                        }
                    }
                }
            }
            "content_block_start" => {
                let index = val.get("index").and_then(|v| v.as_u64()).unwrap_or(0) as usize;
                if let Some(cb) = val.get("content_block") {
                    let cb_type = cb.get("type").and_then(|v| v.as_str()).unwrap_or("");
                    match cb_type {
                        "text" => {
                            let text = cb
                                .get("text")
                                .and_then(|v| v.as_str())
                                .unwrap_or("")
                                .to_string();
                            self.blocks.insert(index, BlockState::Text { text });
                        }
                        "thinking" => {
                            let thinking = cb
                                .get("thinking")
                                .and_then(|v| v.as_str())
                                .unwrap_or("")
                                .to_string();
                            self.blocks.insert(
                                index,
                                BlockState::Thinking {
                                    thinking,
                                    signature: String::new(),
                                },
                            );
                        }
                        "tool_use" => {
                            let id = cb
                                .get("id")
                                .and_then(|v| v.as_str())
                                .unwrap_or("")
                                .to_string();
                            let name = cb
                                .get("name")
                                .and_then(|v| v.as_str())
                                .unwrap_or("")
                                .to_string();
                            self.blocks.insert(
                                index,
                                BlockState::ToolUse {
                                    id: id.clone(),
                                    name: name.clone(),
                                    json_buf: String::new(),
                                },
                            );
                            chunks.push(StreamChunk::ToolCallDelta {
                                index,
                                id: Some(id),
                                name: Some(name),
                                arguments_delta: String::new(),
                            });
                        }
                        _ => {}
                    }
                }
            }
            "content_block_delta" => {
                let index = val.get("index").and_then(|v| v.as_u64()).unwrap_or(0) as usize;
                if let Some(delta) = val.get("delta") {
                    let delta_type = delta.get("type").and_then(|v| v.as_str()).unwrap_or("");
                    match delta_type {
                        "text_delta" => {
                            if let Some(text) = delta.get("text").and_then(|v| v.as_str()) {
                                if !text.is_empty() {
                                    if let Some(BlockState::Text { text: buf }) =
                                        self.blocks.get_mut(&index)
                                    {
                                        buf.push_str(text);
                                    } else {
                                        self.blocks.insert(
                                            index,
                                            BlockState::Text {
                                                text: text.to_string(),
                                            },
                                        );
                                    }
                                    chunks.push(StreamChunk::ContentDelta(text.to_string()));
                                }
                            }
                        }
                        "thinking_delta" => {
                            if let Some(th) = delta.get("thinking").and_then(|v| v.as_str()) {
                                if !th.is_empty() {
                                    if let Some(BlockState::Thinking { thinking: buf, .. }) =
                                        self.blocks.get_mut(&index)
                                    {
                                        buf.push_str(th);
                                    } else {
                                        self.blocks.insert(
                                            index,
                                            BlockState::Thinking {
                                                thinking: th.to_string(),
                                                signature: String::new(),
                                            },
                                        );
                                    }
                                    chunks.push(StreamChunk::ThinkingDelta(th.to_string()));
                                }
                            }
                        }
                        "signature_delta" => {
                            if let Some(sig) = delta.get("signature").and_then(|v| v.as_str()) {
                                if let Some(BlockState::Thinking { signature, .. }) =
                                    self.blocks.get_mut(&index)
                                {
                                    signature.push_str(sig);
                                }
                            }
                        }
                        "input_json_delta" => {
                            let partial = delta
                                .get("partial_json")
                                .and_then(|v| v.as_str())
                                .unwrap_or("");
                            if let Some(BlockState::ToolUse { json_buf, .. }) =
                                self.blocks.get_mut(&index)
                            {
                                json_buf.push_str(partial);
                            } else {
                                self.blocks.insert(
                                    index,
                                    BlockState::ToolUse {
                                        id: String::new(),
                                        name: String::new(),
                                        json_buf: partial.to_string(),
                                    },
                                );
                            }
                            chunks.push(StreamChunk::ToolCallDelta {
                                index,
                                id: None,
                                name: None,
                                arguments_delta: partial.to_string(),
                            });
                        }
                        _ => {}
                    }
                }
            }
            "content_block_stop" => {
                // End of individual block
            }
            "message_delta" => {
                if let Some(delta) = val.get("delta") {
                    if let Some(sr) = delta.get("stop_reason").and_then(|v| v.as_str()) {
                        self.finish_reason = Some(sr.to_string());
                    }
                }
                if let Some(usage) = val.get("usage") {
                    if let Some(ot) = usage.get("output_tokens").and_then(|v| v.as_u64()) {
                        self.completion_tokens = Some(ot as u32);
                    }
                }
            }
            "message_stop" => {
                chunks.push(StreamChunk::Done {
                    finish_reason: self.finish_reason.clone(),
                    prompt_tokens: self.prompt_tokens,
                    completion_tokens: self.completion_tokens,
                });
            }
            "error" => {
                let err_msg = val
                    .get("error")
                    .and_then(|e| e.get("message"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("Anthropic API error");
                chunks.push(StreamChunk::Error(err_msg.to_string()));
            }
            _ => {}
        }

        chunks
    }

    /// Extract concatenated plain text content
    pub fn get_text(&self) -> String {
        let mut out = String::new();
        for state in self.blocks.values() {
            if let BlockState::Text { text } = state {
                out.push_str(text);
            }
        }
        out
    }

    /// Extract concatenated thinking content if any
    pub fn get_thinking(&self) -> Option<String> {
        let mut out = String::new();
        for state in self.blocks.values() {
            if let BlockState::Thinking { thinking, .. } = state {
                out.push_str(thinking);
            }
        }
        if out.is_empty() {
            None
        } else {
            Some(out)
        }
    }

    /// Extract all accumulated tool calls in order
    pub fn get_tool_calls(&self) -> Vec<ToolCall> {
        let mut tools = Vec::new();
        for state in self.blocks.values() {
            if let BlockState::ToolUse { id, name, json_buf } = state {
                tools.push(ToolCall {
                    id: if id.is_empty() {
                        uuid::Uuid::new_v4().to_string()
                    } else {
                        id.clone()
                    },
                    name: name.clone(),
                    arguments: json_buf.clone(),
                });
            }
        }
        tools
    }
}

// ============================================================================
// Anthropic Client
// ============================================================================

#[derive(Clone)]
pub struct AnthropicClient {
    client: reqwest::Client,
    api_key: Option<String>,
    base_url: String,
    anthropic_version: String,
    anthropic_beta: Option<String>,
    default_max_tokens: u32,
}

impl Default for AnthropicClient {
    fn default() -> Self {
        Self::new(None, None)
    }
}

impl AnthropicClient {
    pub fn new(api_key: Option<String>, base_url: Option<String>) -> Self {
        let client = reqwest::Client::builder()
            .use_rustls_tls()
            .connect_timeout(Duration::from_secs(30))
            .build()
            .unwrap_or_else(|_| reqwest::Client::new());

        let url = base_url.unwrap_or_else(|| DEFAULT_ANTHROPIC_URL.to_string());
        Self {
            client,
            api_key,
            base_url: url,
            anthropic_version: DEFAULT_ANTHROPIC_VERSION.to_string(),
            anthropic_beta: None,
            default_max_tokens: DEFAULT_MAX_TOKENS,
        }
    }

    pub fn with_client(
        client: reqwest::Client,
        api_key: Option<String>,
        base_url: Option<String>,
    ) -> Self {
        let url = base_url.unwrap_or_else(|| DEFAULT_ANTHROPIC_URL.to_string());
        Self {
            client,
            api_key,
            base_url: url,
            anthropic_version: DEFAULT_ANTHROPIC_VERSION.to_string(),
            anthropic_beta: None,
            default_max_tokens: DEFAULT_MAX_TOKENS,
        }
    }

    pub fn with_version(mut self, version: impl Into<String>) -> Self {
        self.anthropic_version = version.into();
        self
    }

    pub fn with_beta(mut self, beta: impl Into<String>) -> Self {
        self.anthropic_beta = Some(beta.into());
        self
    }

    pub fn with_max_tokens(mut self, max_tokens: u32) -> Self {
        self.default_max_tokens = max_tokens;
        self
    }

    pub fn base_url(&self) -> &str {
        &self.base_url
    }

    pub fn api_key(&self) -> Option<&str> {
        self.api_key.as_deref()
    }

    /// Construct endpoint URL for Anthropic Messages API (`/v1/messages`)
    pub fn messages_url(&self) -> String {
        construct_anthropic_url(&self.base_url)
    }

    /// Build HTTP headers required by Anthropic API
    pub fn build_headers(&self) -> HeaderMap {
        let mut headers = HeaderMap::new();
        headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
        if let Ok(hv) = HeaderValue::from_str(&self.anthropic_version) {
            headers.insert("anthropic-version", hv);
        }

        if let Some(beta) = &self.anthropic_beta {
            if let Ok(hv) = HeaderValue::from_str(beta) {
                headers.insert("anthropic-beta", hv);
            }
        }

        if let Some(key) = &self.api_key {
            let key = key.trim();
            if !key.is_empty() {
                if let Ok(hv) = HeaderValue::from_str(key) {
                    headers.insert("x-api-key", hv);
                }
            }
        }

        headers
    }

    /// Convert generic messages and tools into an AnthropicRequest payload
    pub fn create_request(
        &self,
        model: &str,
        messages: &[Message],
        tools: &[ToolDefinition],
        temperature: Option<f32>,
        max_tokens: Option<u32>,
    ) -> AnthropicRequest {
        let (system_val, anthropic_messages) = convert_messages_to_anthropic(messages);
        let anthropic_tools = if tools.is_empty() {
            None
        } else {
            Some(convert_tools_to_anthropic(tools))
        };

        AnthropicRequest {
            model: model.to_string(),
            max_tokens: max_tokens.unwrap_or(self.default_max_tokens),
            messages: anthropic_messages,
            system: system_val,
            temperature,
            top_p: None,
            top_k: None,
            tools: anthropic_tools,
            tool_choice: None,
            thinking: None,
            stream: true,
            stop_sequences: None,
        }
    }

    /// Stream Anthropic Messages API with a typed AnthropicRequest
    pub async fn stream(
        &self,
        request: &AnthropicRequest,
    ) -> anyhow::Result<mpsc::Receiver<StreamChunk>> {
        let url = self.messages_url();
        let headers = self.build_headers();

        let response = self
            .client
            .post(&url)
            .headers(headers)
            .json(request)
            .send()
            .await
            .with_context(|| format!("Failed to send request to Anthropic API ({})", url))?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            anyhow::bail!(
                "Anthropic API request to {} failed ({}): {}",
                url,
                status,
                body
            );
        }

        let (tx, rx) = mpsc::channel(256);
        let mut stream = response.bytes_stream().eventsource();

        tokio::spawn(async move {
            let mut accumulator = AnthropicStreamAccumulator::new();
            let mut done_sent = false;

            while let Some(event_res) = stream.next().await {
                match event_res {
                    Ok(event) => {
                        let data = event.data.trim();
                        if data.is_empty() {
                            continue;
                        }

                        if let Ok(val) = serde_json::from_str::<Value>(data) {
                            let chunks = accumulator.process_event(&val);
                            for chunk in chunks {
                                if let StreamChunk::Done { .. } = &chunk {
                                    done_sent = true;
                                }
                                if tx.send(chunk).await.is_err() {
                                    return;
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
                        finish_reason: accumulator.finish_reason,
                        prompt_tokens: accumulator.prompt_tokens,
                        completion_tokens: accumulator.completion_tokens,
                    })
                    .await;
            }
        });

        Ok(rx)
    }

    /// Stream chat completion using generic messages and tools
    pub async fn stream_chat(
        &self,
        model: &str,
        messages: &[Message],
        tools: &[ToolDefinition],
        temperature: Option<f32>,
        max_tokens: Option<u32>,
    ) -> anyhow::Result<mpsc::Receiver<StreamChunk>> {
        let mut req = self.create_request(model, messages, tools, temperature, max_tokens);
        req.stream = true;
        self.stream(&req).await
    }

    /// Non-streaming complete request using Anthropic API
    pub async fn complete(&self, request: &AnthropicRequest) -> anyhow::Result<AnthropicResponse> {
        let mut req = request.clone();
        req.stream = false;

        let url = self.messages_url();
        let headers = self.build_headers();

        let response = self
            .client
            .post(&url)
            .headers(headers)
            .json(&req)
            .send()
            .await
            .with_context(|| format!("Failed to send request to Anthropic API ({})", url))?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            anyhow::bail!(
                "Anthropic API request to {} failed ({}): {}",
                url,
                status,
                body
            );
        }

        let resp: AnthropicResponse = response
            .json()
            .await
            .context("Failed to parse Anthropic JSON response")?;
        Ok(resp)
    }

    /// Non-streaming chat helper returning (text, optional thinking, tool calls)
    pub async fn complete_chat(
        &self,
        model: &str,
        messages: &[Message],
        tools: &[ToolDefinition],
        temperature: Option<f32>,
        max_tokens: Option<u32>,
    ) -> anyhow::Result<(String, Option<String>, Vec<ToolCall>)> {
        let mut rx = self
            .stream_chat(model, messages, tools, temperature, max_tokens)
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
                    anyhow::bail!("Stream error: {}", err);
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
// Message & Tool Conversion Helpers
// ============================================================================

/// Construct the proper `/v1/messages` endpoint URL from a base URL
pub fn construct_anthropic_url(base_url: &str) -> String {
    let base = base_url.trim_end_matches('/');
    if base.ends_with("/messages") {
        base.to_string()
    } else if base.ends_with("/v1") {
        format!("{}/messages", base)
    } else {
        format!("{}/v1/messages", base)
    }
}

/// Convert generic Message list into (Option<system_json>, Vec<AnthropicMessage>)
pub fn convert_messages_to_anthropic(
    messages: &[Message],
) -> (Option<Value>, Vec<AnthropicMessage>) {
    // 1. Extract system messages
    let system_prompts: Vec<&str> = messages
        .iter()
        .filter(|m| m.role == Role::System)
        .map(|m| m.content.as_str())
        .filter(|c| !c.is_empty())
        .collect();

    let system_val = if system_prompts.is_empty() {
        None
    } else {
        Some(json!(system_prompts.join("\n\n")))
    };

    // 2. Convert user/assistant/tool messages
    let mut anthropic_messages: Vec<AnthropicMessage> = Vec::new();

    for msg in messages {
        match msg.role {
            Role::System => {
                // Handled in system parameter
                continue;
            }
            Role::User => {
                anthropic_messages.push(AnthropicMessage {
                    role: AnthropicRole::User,
                    content: AnthropicContent::Text(msg.content.clone()),
                });
            }
            Role::Assistant => {
                if let Some(tool_calls) = &msg.tool_calls {
                    if !tool_calls.is_empty() {
                        let mut content_blocks = Vec::new();
                        if !msg.content.is_empty() {
                            content_blocks.push(AnthropicContentBlock::Text {
                                text: msg.content.clone(),
                                cache_control: None,
                            });
                        }
                        for tc in tool_calls {
                            let input_val: Value =
                                serde_json::from_str(&tc.arguments).unwrap_or_else(|_| json!({}));
                            content_blocks.push(AnthropicContentBlock::ToolUse {
                                id: tc.id.clone(),
                                name: tc.name.clone(),
                                input: input_val,
                            });
                        }
                        anthropic_messages.push(AnthropicMessage {
                            role: AnthropicRole::Assistant,
                            content: AnthropicContent::Blocks(content_blocks),
                        });
                    } else {
                        anthropic_messages.push(AnthropicMessage {
                            role: AnthropicRole::Assistant,
                            content: AnthropicContent::Text(msg.content.clone()),
                        });
                    }
                } else {
                    anthropic_messages.push(AnthropicMessage {
                        role: AnthropicRole::Assistant,
                        content: AnthropicContent::Text(msg.content.clone()),
                    });
                }
            }
            Role::Tool => {
                let tool_use_id = msg.tool_call_id.clone().unwrap_or_default();
                let block = AnthropicContentBlock::ToolResult {
                    tool_use_id,
                    content: Value::String(msg.content.clone()),
                    is_error: None,
                    cache_control: None,
                };
                anthropic_messages.push(AnthropicMessage {
                    role: AnthropicRole::User,
                    content: AnthropicContent::Blocks(vec![block]),
                });
            }
        }
    }

    (system_val, anthropic_messages)
}

/// Convert generic ToolDefinition list into AnthropicTool list
pub fn convert_tools_to_anthropic(tools: &[ToolDefinition]) -> Vec<AnthropicTool> {
    tools
        .iter()
        .map(|t| AnthropicTool {
            name: t.name.clone(),
            description: if t.description.is_empty() {
                None
            } else {
                Some(t.description.clone())
            },
            input_schema: t.parameters.clone(),
            cache_control: None,
        })
        .collect()
}

// ============================================================================
// Unit Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_construct_anthropic_url() {
        assert_eq!(
            construct_anthropic_url("https://api.anthropic.com"),
            "https://api.anthropic.com/v1/messages"
        );
        assert_eq!(
            construct_anthropic_url("https://api.anthropic.com/v1"),
            "https://api.anthropic.com/v1/messages"
        );
        assert_eq!(
            construct_anthropic_url("https://api.anthropic.com/v1/messages"),
            "https://api.anthropic.com/v1/messages"
        );
        assert_eq!(
            construct_anthropic_url(
                "https://gateway.ai.cloudflare.com/v1/account/gateway/anthropic"
            ),
            "https://gateway.ai.cloudflare.com/v1/account/gateway/anthropic/v1/messages"
        );
    }

    #[test]
    fn test_convert_messages_with_system_and_tools() {
        let messages = vec![
            Message::system("You are a helpful coding assistant."),
            Message::user("Please read main.rs"),
            Message::assistant_with_tools(
                "I will read main.rs for you.",
                vec![ToolCall {
                    id: "toolu_01".to_string(),
                    name: "read_file".to_string(),
                    arguments: r#"{"path": "src/main.rs"}"#.to_string(),
                }],
            ),
            Message::tool_result("toolu_01", "fn main() { println!(\"hello\"); }"),
        ];

        let (system, conv) = convert_messages_to_anthropic(&messages);
        assert_eq!(system, Some(json!("You are a helpful coding assistant.")));
        assert_eq!(conv.len(), 3);

        // User message
        assert_eq!(conv[0].role, AnthropicRole::User);
        match &conv[0].content {
            AnthropicContent::Text(t) => assert_eq!(t, "Please read main.rs"),
            _ => panic!("Expected text content"),
        }

        // Assistant message with tool use
        assert_eq!(conv[1].role, AnthropicRole::Assistant);
        match &conv[1].content {
            AnthropicContent::Blocks(blocks) => {
                assert_eq!(blocks.len(), 2);
                match &blocks[0] {
                    AnthropicContentBlock::Text { text, .. } => {
                        assert_eq!(text, "I will read main.rs for you.");
                    }
                    _ => panic!("Expected text block"),
                }
                match &blocks[1] {
                    AnthropicContentBlock::ToolUse { id, name, input } => {
                        assert_eq!(id, "toolu_01");
                        assert_eq!(name, "read_file");
                        assert_eq!(input["path"], "src/main.rs");
                    }
                    _ => panic!("Expected tool_use block"),
                }
            }
            _ => panic!("Expected blocks content"),
        }

        // Tool result message as user role
        assert_eq!(conv[2].role, AnthropicRole::User);
        match &conv[2].content {
            AnthropicContent::Blocks(blocks) => {
                assert_eq!(blocks.len(), 1);
                match &blocks[0] {
                    AnthropicContentBlock::ToolResult {
                        tool_use_id,
                        content,
                        ..
                    } => {
                        assert_eq!(tool_use_id, "toolu_01");
                        assert_eq!(
                            content,
                            &Value::String("fn main() { println!(\"hello\"); }".to_string())
                        );
                    }
                    _ => panic!("Expected tool_result block"),
                }
            }
            _ => panic!("Expected blocks content"),
        }
    }

    #[test]
    fn test_stream_accumulator_text_and_tool_call() {
        let mut acc = AnthropicStreamAccumulator::new();

        // 1. message_start
        let msg_start = json!({
            "type": "message_start",
            "message": {
                "id": "msg_01",
                "type": "message",
                "role": "assistant",
                "model": "claude-3-5-sonnet-20241022",
                "usage": { "input_tokens": 42, "output_tokens": 1 }
            }
        });
        let chunks = acc.process_event(&msg_start);
        assert!(chunks.is_empty());
        assert_eq!(acc.prompt_tokens, Some(42));

        // 2. content_block_start for text
        let cb_start_0 = json!({
            "type": "content_block_start",
            "index": 0,
            "content_block": { "type": "text", "text": "" }
        });
        let _ = acc.process_event(&cb_start_0);

        // 3. content_block_delta for text
        let cb_delta_0_1 = json!({
            "type": "content_block_delta",
            "index": 0,
            "delta": { "type": "text_delta", "text": "Let me " }
        });
        let c1 = acc.process_event(&cb_delta_0_1);
        assert_eq!(c1.len(), 1);
        match &c1[0] {
            StreamChunk::ContentDelta(s) => assert_eq!(s, "Let me "),
            _ => panic!("Expected ContentDelta"),
        }

        let cb_delta_0_2 = json!({
            "type": "content_block_delta",
            "index": 0,
            "delta": { "type": "text_delta", "text": "check." }
        });
        let c2 = acc.process_event(&cb_delta_0_2);
        assert_eq!(c2.len(), 1);

        // 4. content_block_stop for text
        let cb_stop_0 = json!({ "type": "content_block_stop", "index": 0 });
        let _ = acc.process_event(&cb_stop_0);

        // 5. content_block_start for tool_use
        let cb_start_1 = json!({
            "type": "content_block_start",
            "index": 1,
            "content_block": {
                "type": "tool_use",
                "id": "toolu_abc",
                "name": "grep_files",
                "input": {}
            }
        });
        let c3 = acc.process_event(&cb_start_1);
        assert_eq!(c3.len(), 1);
        match &c3[0] {
            StreamChunk::ToolCallDelta {
                index,
                id,
                name,
                arguments_delta,
            } => {
                assert_eq!(*index, 1);
                assert_eq!(id.as_deref(), Some("toolu_abc"));
                assert_eq!(name.as_deref(), Some("grep_files"));
                assert_eq!(arguments_delta, "");
            }
            _ => panic!("Expected ToolCallDelta"),
        }

        // 6. content_block_delta with input_json_delta
        let cb_delta_1_1 = json!({
            "type": "content_block_delta",
            "index": 1,
            "delta": {
                "type": "input_json_delta",
                "partial_json": "{\"pattern\":"
            }
        });
        let c4 = acc.process_event(&cb_delta_1_1);
        assert_eq!(c4.len(), 1);
        match &c4[0] {
            StreamChunk::ToolCallDelta {
                arguments_delta, ..
            } => {
                assert_eq!(arguments_delta, "{\"pattern\":");
            }
            _ => panic!("Expected ToolCallDelta"),
        }

        let cb_delta_1_2 = json!({
            "type": "content_block_delta",
            "index": 1,
            "delta": {
                "type": "input_json_delta",
                "partial_json": " \"TODO\"}"
            }
        });
        let c5 = acc.process_event(&cb_delta_1_2);
        assert_eq!(c5.len(), 1);

        // 7. content_block_stop for tool_use
        let cb_stop_1 = json!({ "type": "content_block_stop", "index": 1 });
        let _ = acc.process_event(&cb_stop_1);

        // 8. message_delta
        let msg_delta = json!({
            "type": "message_delta",
            "delta": { "stop_reason": "tool_use" },
            "usage": { "output_tokens": 58 }
        });
        let _ = acc.process_event(&msg_delta);
        assert_eq!(acc.finish_reason.as_deref(), Some("tool_use"));
        assert_eq!(acc.completion_tokens, Some(58));

        // 9. message_stop
        let msg_stop = json!({ "type": "message_stop" });
        let c6 = acc.process_event(&msg_stop);
        assert_eq!(c6.len(), 1);
        match &c6[0] {
            StreamChunk::Done {
                finish_reason,
                prompt_tokens,
                completion_tokens,
            } => {
                assert_eq!(finish_reason.as_deref(), Some("tool_use"));
                assert_eq!(*prompt_tokens, Some(42));
                assert_eq!(*completion_tokens, Some(58));
            }
            _ => panic!("Expected Done"),
        }

        // Verify final accumulated text and tool calls
        assert_eq!(acc.get_text(), "Let me check.");
        let tools = acc.get_tool_calls();
        assert_eq!(tools.len(), 1);
        assert_eq!(tools[0].id, "toolu_abc");
        assert_eq!(tools[0].name, "grep_files");
        assert_eq!(tools[0].arguments, "{\"pattern\": \"TODO\"}");
    }

    #[test]
    fn test_stream_accumulator_thinking() {
        let mut acc = AnthropicStreamAccumulator::new();

        let cb_start = json!({
            "type": "content_block_start",
            "index": 0,
            "content_block": { "type": "thinking", "thinking": "" }
        });
        let _ = acc.process_event(&cb_start);

        let cb_delta = json!({
            "type": "content_block_delta",
            "index": 0,
            "delta": { "type": "thinking_delta", "thinking": "Let's analyze the problem..." }
        });
        let chunks = acc.process_event(&cb_delta);
        assert_eq!(chunks.len(), 1);
        match &chunks[0] {
            StreamChunk::ThinkingDelta(t) => assert_eq!(t, "Let's analyze the problem..."),
            _ => panic!("Expected ThinkingDelta"),
        }

        assert_eq!(
            acc.get_thinking(),
            Some("Let's analyze the problem...".to_string())
        );
    }
}
