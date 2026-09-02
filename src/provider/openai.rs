use std::time::Duration;

use anyhow::Context;
use eventsource_stream::Eventsource;
use futures::StreamExt;
use reqwest::header::{HeaderMap, HeaderValue, AUTHORIZATION, CONTENT_TYPE};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tokio::sync::mpsc;

use crate::provider::types::{Message, Role, StreamChunk, ToolCall, ToolDefinition};

pub const DEFAULT_OPENAI_BASE_URL: &str = "https://api.openai.com/v1";
pub const DEFAULT_DEEPSEEK_BASE_URL: &str = "https://api.deepseek.com";
pub const DEFAULT_XAI_BASE_URL: &str = "https://api.x.ai/v1";
pub const DEFAULT_GROQ_BASE_URL: &str = "https://api.groq.com/openai/v1";
pub const DEFAULT_OLLAMA_BASE_URL: &str = "http://localhost:11434/v1";
pub const DEFAULT_OPENROUTER_BASE_URL: &str = "https://openrouter.ai/api/v1";

/// Provider flavors that use the OpenAI-compatible chat completions specification.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OpenAiProviderFlavor {
    OpenAi,
    DeepSeek,
    Xai,
    Groq,
    Ollama,
    OpenRouter,
    Custom(String),
}

impl OpenAiProviderFlavor {
    pub fn from_name_and_url(provider: &str, base_url: &str) -> Self {
        let p_lower = provider.to_lowercase();
        let u_lower = base_url.to_lowercase();

        if p_lower == "openrouter" || u_lower.contains("openrouter.ai") {
            Self::OpenRouter
        } else if p_lower == "deepseek" || u_lower.contains("deepseek.com") {
            Self::DeepSeek
        } else if p_lower == "xai" || p_lower == "grok" || u_lower.contains("x.ai") {
            Self::Xai
        } else if p_lower == "groq" || u_lower.contains("groq.com") {
            Self::Groq
        } else if p_lower == "ollama" || u_lower.contains(":11434") {
            Self::Ollama
        } else if p_lower == "openai" || u_lower.contains("openai.com") {
            Self::OpenAi
        } else {
            Self::Custom(provider.to_string())
        }
    }
}

/// Normalizes and constructs the full chat completions endpoint URL.
pub fn construct_openai_url(base_url: &str) -> String {
    let base = base_url.trim().trim_end_matches('/');
    if base.ends_with("/chat/completions") {
        base.to_string()
    } else if base.ends_with("/v1") {
        format!("{}/chat/completions", base)
    } else if base.contains(":11434") {
        // Ollama OpenAI compatibility endpoint
        format!("{}/v1/chat/completions", base)
    } else {
        format!("{}/chat/completions", base)
    }
}

/// Builds the HTTP request headers for an OpenAI-compatible provider.
pub fn build_openai_headers(
    provider: &str,
    api_key: Option<&str>,
    base_url: &str,
) -> HeaderMap {
    let mut headers = HeaderMap::new();
    headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));

    if let Some(key) = api_key {
        let key = key.trim();
        if !key.is_empty() {
            let auth_val = format!("Bearer {}", key);
            if let Ok(hv) = HeaderValue::from_str(&auth_val) {
                headers.insert(AUTHORIZATION, hv);
            }
        }
    }

    let flavor = OpenAiProviderFlavor::from_name_and_url(provider, base_url);
    if flavor == OpenAiProviderFlavor::OpenRouter {
        if let Ok(hv) = HeaderValue::from_str("https://github.com/theaungmyatmoe/fusion") {
            headers.insert("HTTP-Referer", hv);
        }
        if let Ok(hv) = HeaderValue::from_str("Fusion AI Assistant") {
            headers.insert("X-Title", hv);
        }
    }

    headers
}

/// Builds JSON payload for OpenAI-compatible `/chat/completions` request.
pub fn build_openai_payload(
    model: &str,
    messages: &[Message],
    tools: &[ToolDefinition],
    temperature: Option<f32>,
    max_tokens: Option<u32>,
    stream: bool,
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

    let mut messages_json = Vec::with_capacity(messages.len());
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

    payload
}

/// Fluent builder for OpenAI-compatible chat completion requests.
#[derive(Debug, Clone)]
pub struct OpenAiRequestBuilder {
    pub provider: String,
    pub model: String,
    pub messages: Vec<Message>,
    pub tools: Vec<ToolDefinition>,
    pub temperature: Option<f32>,
    pub max_tokens: Option<u32>,
    pub stream: bool,
    pub include_usage: bool,
}

impl OpenAiRequestBuilder {
    pub fn new(model: impl Into<String>) -> Self {
        Self {
            provider: "openai".to_string(),
            model: model.into(),
            messages: Vec::new(),
            tools: Vec::new(),
            temperature: None,
            max_tokens: None,
            stream: true,
            include_usage: true,
        }
    }

    pub fn provider(mut self, provider: impl Into<String>) -> Self {
        self.provider = provider.into();
        self
    }

    pub fn messages(mut self, messages: Vec<Message>) -> Self {
        self.messages = messages;
        self
    }

    pub fn add_message(mut self, message: Message) -> Self {
        self.messages.push(message);
        self
    }

    pub fn tools(mut self, tools: Vec<ToolDefinition>) -> Self {
        self.tools = tools;
        self
    }

    pub fn add_tool(mut self, tool: ToolDefinition) -> Self {
        self.tools.push(tool);
        self
    }

    pub fn temperature(mut self, temperature: f32) -> Self {
        self.temperature = Some(temperature);
        self
    }

    pub fn optional_temperature(mut self, temperature: Option<f32>) -> Self {
        self.temperature = temperature;
        self
    }

    pub fn max_tokens(mut self, max_tokens: u32) -> Self {
        self.max_tokens = Some(max_tokens);
        self
    }

    pub fn optional_max_tokens(mut self, max_tokens: Option<u32>) -> Self {
        self.max_tokens = max_tokens;
        self
    }

    pub fn stream(mut self, stream: bool) -> Self {
        self.stream = stream;
        self
    }

    pub fn include_usage(mut self, include_usage: bool) -> Self {
        self.include_usage = include_usage;
        self
    }

    /// Builds the request payload as a serde_json Value.
    pub fn build_payload(&self) -> Value {
        let mut payload = build_openai_payload(
            &self.model,
            &self.messages,
            &self.tools,
            self.temperature,
            self.max_tokens,
            self.stream,
        );

        if self.stream && !self.include_usage {
            if let Some(obj) = payload.as_object_mut() {
                obj.remove("stream_options");
            }
        }

        payload
    }

    /// Builds the HTTP request ready to be sent.
    pub fn build_request(
        &self,
        client: &reqwest::Client,
        base_url: &str,
        api_key: Option<&str>,
    ) -> anyhow::Result<reqwest::Request> {
        let url = construct_openai_url(base_url);
        let headers = build_openai_headers(&self.provider, api_key, base_url);
        let payload = self.build_payload();

        let request = client
            .post(&url)
            .headers(headers)
            .json(&payload)
            .build()
            .with_context(|| format!("Failed to build OpenAI HTTP request for {}", url))?;

        Ok(request)
    }
}

/// Stateful parser for OpenAI SSE streams.
/// Handles standard text deltas, tool call deltas, usage stats, finish reasons,
/// and reasoning/thinking deltas from DeepSeek (R1/V3), Groq, OpenRouter, and Ollama.
#[derive(Debug, Default, Clone)]
pub struct OpenAiSseParser {
    prompt_tokens: Option<u32>,
    completion_tokens: Option<u32>,
    done_emitted: bool,
}

impl OpenAiSseParser {
    pub fn new() -> Self {
        Self {
            prompt_tokens: None,
            completion_tokens: None,
            done_emitted: false,
        }
    }

    pub fn reset(&mut self) {
        self.prompt_tokens = None;
        self.completion_tokens = None;
        self.done_emitted = false;
    }

    pub fn prompt_tokens(&self) -> Option<u32> {
        self.prompt_tokens
    }

    pub fn completion_tokens(&self) -> Option<u32> {
        self.completion_tokens
    }

    pub fn is_done(&self) -> bool {
        self.done_emitted
    }

    /// Parses an SSE data chunk/line and returns generated `StreamChunk`s.
    pub fn parse_line(&mut self, line: &str) -> Vec<StreamChunk> {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with(':') {
            return Vec::new();
        }

        let data = if let Some(stripped) = trimmed.strip_prefix("data:") {
            stripped.trim()
        } else {
            trimmed
        };

        if data == "[DONE]" {
            if !self.done_emitted {
                self.done_emitted = true;
                return vec![StreamChunk::Done {
                    finish_reason: None,
                    prompt_tokens: self.prompt_tokens,
                    completion_tokens: self.completion_tokens,
                }];
            }
            return Vec::new();
        }

        if let Ok(val) = serde_json::from_str::<Value>(data) {
            self.parse_json_value(&val)
        } else {
            Vec::new()
        }
    }

    /// Parses a multi-line SSE text buffer containing multiple `data:` lines.
    pub fn parse_chunk(&mut self, chunk: &str) -> Vec<StreamChunk> {
        let mut results = Vec::new();
        for line in chunk.lines() {
            results.extend(self.parse_line(line));
        }
        results
    }

    /// Parses a deserialized JSON value from an SSE event.
    pub fn parse_json_value(&mut self, val: &Value) -> Vec<StreamChunk> {
        let mut chunks = Vec::new();

        // Check for top-level error in stream
        if let Some(err) = val.get("error") {
            let msg = if let Some(m) = err.get("message").and_then(|v| v.as_str()) {
                m.to_string()
            } else if let Some(s) = err.as_str() {
                s.to_string()
            } else {
                err.to_string()
            };
            chunks.push(StreamChunk::Error(msg));
            return chunks;
        }

        // Extract usage token counts if present (e.g. stream_options or final chunk)
        if let Some(usage) = val.get("usage") {
            if let Some(pt) = usage.get("prompt_tokens").and_then(|v| v.as_u64()) {
                self.prompt_tokens = Some(pt as u32);
            }
            if let Some(ct) = usage.get("completion_tokens").and_then(|v| v.as_u64()) {
                self.completion_tokens = Some(ct as u32);
            }
        }

        // Extract choices
        if let Some(choices) = val.get("choices").and_then(|v| v.as_array()) {
            for choice in choices {
                let finish_reason = choice
                    .get("finish_reason")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string());

                if let Some(delta) = choice.get("delta") {
                    // 1. Thinking / Reasoning Delta Extraction:
                    // Supported fields:
                    // - `reasoning_content` (DeepSeek, Groq DeepSeek-R1, OpenAI proxies)
                    // - `reasoning` (OpenRouter, various proxies)
                    // - `thought` (various model adapters)
                    // - `thinking` (various model adapters)
                    let reasoning_opt = delta
                        .get("reasoning_content")
                        .or_else(|| delta.get("reasoning"))
                        .or_else(|| delta.get("thought"))
                        .or_else(|| delta.get("thinking"))
                        .and_then(|v| v.as_str());

                    if let Some(reasoning) = reasoning_opt {
                        if !reasoning.is_empty() {
                            chunks.push(StreamChunk::ThinkingDelta(reasoning.to_string()));
                        }
                    }

                    // 2. Standard Content Delta Extraction:
                    if let Some(content) = delta.get("content").and_then(|v| v.as_str()) {
                        if !content.is_empty() {
                            chunks.push(StreamChunk::ContentDelta(content.to_string()));
                        }
                    }

                    // 3. Tool Calls Delta Extraction:
                    if let Some(tool_calls) = delta.get("tool_calls").and_then(|v| v.as_array()) {
                        for tc in tool_calls {
                            let index = tc
                                .get("index")
                                .and_then(|v| v.as_u64())
                                .unwrap_or(0) as usize;

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

                            chunks.push(StreamChunk::ToolCallDelta {
                                index,
                                id,
                                name,
                                arguments_delta,
                            });
                        }
                    }
                }

                // If choice specifies finish_reason
                if let Some(fr) = finish_reason {
                    self.done_emitted = true;
                    chunks.push(StreamChunk::Done {
                        finish_reason: Some(fr),
                        prompt_tokens: self.prompt_tokens,
                        completion_tokens: self.completion_tokens,
                    });
                }
            }
        }

        chunks
    }

    /// Finishes the stream, emitting Done if not already emitted.
    pub fn finish(&mut self) -> Option<StreamChunk> {
        if !self.done_emitted {
            self.done_emitted = true;
            Some(StreamChunk::Done {
                finish_reason: None,
                prompt_tokens: self.prompt_tokens,
                completion_tokens: self.completion_tokens,
            })
        } else {
            None
        }
    }
}

/// Helper function to parse an OpenAI SSE data string into a vector of StreamChunks.
pub fn parse_openai_sse_event(event_data: &str) -> Vec<StreamChunk> {
    let mut parser = OpenAiSseParser::new();
    parser.parse_line(event_data)
}

/// Streams a chat completion request to an OpenAI-compatible provider.
#[allow(clippy::too_many_arguments)]
pub async fn stream_openai_chat(
    client: &reqwest::Client,
    provider: &str,
    model: &str,
    temperature: Option<f32>,
    max_tokens: Option<u32>,
    api_key: Option<&str>,
    base_url: &str,
    messages: &[Message],
    tools: &[ToolDefinition],
) -> anyhow::Result<mpsc::Receiver<StreamChunk>> {
    let url = construct_openai_url(base_url);
    let builder = OpenAiRequestBuilder::new(model)
        .provider(provider)
        .messages(messages.to_vec())
        .tools(tools.to_vec())
        .optional_temperature(temperature)
        .optional_max_tokens(max_tokens)
        .stream(true)
        .include_usage(true);

    let request = builder.build_request(client, base_url, api_key)?;

    let response = client
        .execute(request)
        .await
        .with_context(|| format!("Failed to send request to {}", url))?;

    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        anyhow::bail!("API request to {} failed ({}): {}", url, status, body);
    }

    let (tx, rx) = mpsc::channel(256);
    let mut stream = response.bytes_stream().eventsource();

    tokio::spawn(async move {
        let mut parser = OpenAiSseParser::new();

        while let Some(event_res) = stream.next().await {
            match event_res {
                Ok(event) => {
                    let chunks = parser.parse_line(&event.data);
                    for chunk in chunks {
                        let _ = tx.send(chunk).await;
                    }
                }
                Err(e) => {
                    let _ = tx.send(StreamChunk::Error(e.to_string())).await;
                    break;
                }
            }
        }

        if let Some(done_chunk) = parser.finish() {
            let _ = tx.send(done_chunk).await;
        }
    });

    Ok(rx)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_construct_openai_url() {
        assert_eq!(
            construct_openai_url("https://api.openai.com/v1"),
            "https://api.openai.com/v1/chat/completions"
        );
        assert_eq!(
            construct_openai_url("https://api.openai.com/v1/"),
            "https://api.openai.com/v1/chat/completions"
        );
        assert_eq!(
            construct_openai_url("https://api.openai.com/v1/chat/completions"),
            "https://api.openai.com/v1/chat/completions"
        );
        assert_eq!(
            construct_openai_url("https://api.deepseek.com"),
            "https://api.deepseek.com/chat/completions"
        );
        assert_eq!(
            construct_openai_url("https://api.deepseek.com/v1"),
            "https://api.deepseek.com/v1/chat/completions"
        );
        assert_eq!(
            construct_openai_url("http://localhost:11434"),
            "http://localhost:11434/v1/chat/completions"
        );
        assert_eq!(
            construct_openai_url("http://localhost:11434/v1"),
            "http://localhost:11434/v1/chat/completions"
        );
        assert_eq!(
            construct_openai_url("https://openrouter.ai/api/v1"),
            "https://openrouter.ai/api/v1/chat/completions"
        );
        assert_eq!(
            construct_openai_url("https://api.groq.com/openai/v1"),
            "https://api.groq.com/openai/v1/chat/completions"
        );
        assert_eq!(
            construct_openai_url("https://api.x.ai/v1"),
            "https://api.x.ai/v1/chat/completions"
        );
    }

    #[test]
    fn test_build_headers() {
        let headers = build_openai_headers("openai", Some("sk-test123"), "https://api.openai.com/v1");
        assert_eq!(headers.get(CONTENT_TYPE).unwrap(), "application/json");
        assert_eq!(headers.get(AUTHORIZATION).unwrap(), "Bearer sk-test123");
        assert!(headers.get("HTTP-Referer").is_none());

        // OpenRouter headers
        let or_headers = build_openai_headers(
            "openrouter",
            Some("sk-or-v1-test"),
            "https://openrouter.ai/api/v1",
        );
        assert_eq!(or_headers.get(AUTHORIZATION).unwrap(), "Bearer sk-or-v1-test");
        assert_eq!(
            or_headers.get("HTTP-Referer").unwrap(),
            "https://github.com/theaungmyatmoe/fusion"
        );
        assert_eq!(or_headers.get("X-Title").unwrap(), "Fusion AI Assistant");
    }

    #[test]
    fn test_build_payload_basic() {
        let messages = vec![
            Message::system("You are a helpful assistant."),
            Message::user("Hello!"),
        ];

        let payload = build_openai_payload("gpt-4o", &messages, &[], Some(0.7), Some(2048), true);

        assert_eq!(payload["model"], "gpt-4o");
        assert_eq!(payload["stream"], true);
        assert!((payload["temperature"].as_f64().unwrap() - 0.7).abs() < 1e-4);
        assert_eq!(payload["max_tokens"], 2048);
        assert_eq!(payload["stream_options"]["include_usage"], true);

        let msgs = payload["messages"].as_array().unwrap();
        assert_eq!(msgs.len(), 2);
        assert_eq!(msgs[0]["role"], "system");
        assert_eq!(msgs[0]["content"], "You are a helpful assistant.");
        assert_eq!(msgs[1]["role"], "user");
        assert_eq!(msgs[1]["content"], "Hello!");
    }

    #[test]
    fn test_build_payload_with_tools_and_assistant_tool_calls() {
        let tool_def = ToolDefinition {
            name: "read_file".to_string(),
            description: "Reads a file".to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string" }
                },
                "required": ["path"]
            }),
        };

        let messages = vec![
            Message::user("Read file foo.txt"),
            Message::assistant_with_tools(
                "",
                vec![ToolCall {
                    id: "call_123".to_string(),
                    name: "read_file".to_string(),
                    arguments: "{\"path\":\"foo.txt\"}".to_string(),
                }],
            ),
            Message::tool_result("call_123", "contents of foo.txt"),
        ];

        let payload = build_openai_payload("gpt-4o-mini", &messages, &[tool_def], None, None, true);

        let tools = payload["tools"].as_array().unwrap();
        assert_eq!(tools.len(), 1);
        assert_eq!(tools[0]["type"], "function");
        assert_eq!(tools[0]["function"]["name"], "read_file");

        let msgs = payload["messages"].as_array().unwrap();
        assert_eq!(msgs.len(), 3);
        assert_eq!(msgs[1]["role"], "assistant");
        let tc = &msgs[1]["tool_calls"].as_array().unwrap()[0];
        assert_eq!(tc["id"], "call_123");
        assert_eq!(tc["function"]["name"], "read_file");
        assert_eq!(tc["function"]["arguments"], "{\"path\":\"foo.txt\"}");

        assert_eq!(msgs[2]["role"], "tool");
        assert_eq!(msgs[2]["tool_call_id"], "call_123");
        assert_eq!(msgs[2]["content"], "contents of foo.txt");
    }

    #[test]
    fn test_parser_content_delta() {
        let mut parser = OpenAiSseParser::new();
        let chunk_json = r#"{"id":"chatcmpl-1","choices":[{"index":0,"delta":{"content":"Hello world!"},"finish_reason":null}]}"#;

        let chunks = parser.parse_line(chunk_json);
        assert_eq!(chunks.len(), 1);
        match &chunks[0] {
            StreamChunk::ContentDelta(text) => assert_eq!(text, "Hello world!"),
            other => panic!("Unexpected chunk: {:?}", other),
        }
    }

    #[test]
    fn test_parser_deepseek_reasoning_content_delta() {
        let mut parser = OpenAiSseParser::new();
        let chunk_json = r#"{"id":"chatcmpl-2","choices":[{"index":0,"delta":{"reasoning_content":"Let's analyze step 1."},"finish_reason":null}]}"#;

        let chunks = parser.parse_line(chunk_json);
        assert_eq!(chunks.len(), 1);
        match &chunks[0] {
            StreamChunk::ThinkingDelta(thinking) => {
                assert_eq!(thinking, "Let's analyze step 1.");
            }
            other => panic!("Unexpected chunk: {:?}", other),
        }
    }

    #[test]
    fn test_parser_openrouter_reasoning_delta() {
        let mut parser = OpenAiSseParser::new();
        let chunk_json = r#"data: {"id":"gen-123","choices":[{"index":0,"delta":{"reasoning":"Thinking through OpenRouter..."}}]}"#;

        let chunks = parser.parse_line(chunk_json);
        assert_eq!(chunks.len(), 1);
        match &chunks[0] {
            StreamChunk::ThinkingDelta(thinking) => {
                assert_eq!(thinking, "Thinking through OpenRouter...");
            }
            other => panic!("Unexpected chunk: {:?}", other),
        }
    }

    #[test]
    fn test_parser_thought_and_thinking_aliases() {
        let mut parser = OpenAiSseParser::new();

        let chunk1 = r#"{"choices":[{"delta":{"thought":"Thought delta"}}]}"#;
        let res1 = parser.parse_line(chunk1);
        match &res1[0] {
            StreamChunk::ThinkingDelta(t) => assert_eq!(t, "Thought delta"),
            other => panic!("Unexpected chunk: {:?}", other),
        }

        let chunk2 = r#"{"choices":[{"delta":{"thinking":"Thinking delta"}}]}"#;
        let res2 = parser.parse_line(chunk2);
        match &res2[0] {
            StreamChunk::ThinkingDelta(t) => assert_eq!(t, "Thinking delta"),
            other => panic!("Unexpected chunk: {:?}", other),
        }
    }

    #[test]
    fn test_parser_mixed_reasoning_and_content() {
        let mut parser = OpenAiSseParser::new();
        let chunk_json = r#"{"choices":[{"index":0,"delta":{"reasoning_content":"Step 1","content":"Final answer"}}]}"#;

        let chunks = parser.parse_line(chunk_json);
        assert_eq!(chunks.len(), 2);
        match &chunks[0] {
            StreamChunk::ThinkingDelta(t) => assert_eq!(t, "Step 1"),
            other => panic!("Unexpected chunk: {:?}", other),
        }
        match &chunks[1] {
            StreamChunk::ContentDelta(c) => assert_eq!(c, "Final answer"),
            other => panic!("Unexpected chunk: {:?}", other),
        }
    }

    #[test]
    fn test_parser_tool_call_delta() {
        let mut parser = OpenAiSseParser::new();

        // 1. Initial tool call header with ID and function name
        let chunk1 = r#"{"choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"id":"call_abc","type":"function","function":{"name":"bash","arguments":""}}]}}]}"#;
        let res1 = parser.parse_line(chunk1);
        assert_eq!(res1.len(), 1);
        match &res1[0] {
            StreamChunk::ToolCallDelta {
                index,
                id,
                name,
                arguments_delta,
            } => {
                assert_eq!(*index, 0);
                assert_eq!(id.as_deref(), Some("call_abc"));
                assert_eq!(name.as_deref(), Some("bash"));
                assert_eq!(arguments_delta, "");
            }
            other => panic!("Unexpected chunk: {:?}", other),
        }

        // 2. Argument fragment
        let chunk2 = r#"{"choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"function":{"arguments":"{\"cmd\":"}}]}}]}"#;
        let res2 = parser.parse_line(chunk2);
        assert_eq!(res2.len(), 1);
        match &res2[0] {
            StreamChunk::ToolCallDelta {
                index,
                id,
                name,
                arguments_delta,
            } => {
                assert_eq!(*index, 0);
                assert_eq!(id, &None);
                assert_eq!(name, &None);
                assert_eq!(arguments_delta, "{\"cmd\":");
            }
            other => panic!("Unexpected chunk: {:?}", other),
        }
    }

    #[test]
    fn test_parser_usage_and_done() {
        let mut parser = OpenAiSseParser::new();

        // Usage chunk
        let usage_chunk = r#"{"usage":{"prompt_tokens":42,"completion_tokens":108,"total_tokens":150},"choices":[]}"#;
        let res_u = parser.parse_line(usage_chunk);
        assert!(res_u.is_empty());
        assert_eq!(parser.prompt_tokens(), Some(42));
        assert_eq!(parser.completion_tokens(), Some(108));

        // Finish reason chunk
        let finish_chunk = r#"{"choices":[{"index":0,"delta":{},"finish_reason":"stop"}]}"#;
        let res_f = parser.parse_line(finish_chunk);
        assert_eq!(res_f.len(), 1);
        match &res_f[0] {
            StreamChunk::Done {
                finish_reason,
                prompt_tokens,
                completion_tokens,
            } => {
                assert_eq!(finish_reason.as_deref(), Some("stop"));
                assert_eq!(*prompt_tokens, Some(42));
                assert_eq!(*completion_tokens, Some(108));
            }
            other => panic!("Unexpected chunk: {:?}", other),
        }

        // Subsequent [DONE] should not duplicate Done chunk
        let done_res = parser.parse_line("data: [DONE]");
        assert!(done_res.is_empty());
    }

    #[test]
    fn test_parser_done_without_prior_finish() {
        let mut parser = OpenAiSseParser::new();

        let done_res = parser.parse_line("data: [DONE]");
        assert_eq!(done_res.len(), 1);
        match &done_res[0] {
            StreamChunk::Done {
                finish_reason,
                prompt_tokens,
                completion_tokens,
            } => {
                assert_eq!(*finish_reason, None);
                assert_eq!(*prompt_tokens, None);
                assert_eq!(*completion_tokens, None);
            }
            other => panic!("Unexpected chunk: {:?}", other),
        }
    }

    #[test]
    fn test_parser_error_payload() {
        let mut parser = OpenAiSseParser::new();
        let error_json = r#"{"error":{"message":"Rate limit exceeded","type":"requests","code":"rate_limit"}}"#;

        let res = parser.parse_line(error_json);
        assert_eq!(res.len(), 1);
        match &res[0] {
            StreamChunk::Error(msg) => assert_eq!(msg, "Rate limit exceeded"),
            other => panic!("Unexpected chunk: {:?}", other),
        }
    }

    #[test]
    fn test_builder_fluent_interface() {
        let builder = OpenAiRequestBuilder::new("deepseek-chat")
            .provider("deepseek")
            .temperature(0.2)
            .max_tokens(4096)
            .add_message(Message::user("Explain Rust lifetimies"));

        assert_eq!(builder.provider, "deepseek");
        assert_eq!(builder.model, "deepseek-chat");
        assert_eq!(builder.temperature, Some(0.2));
        assert_eq!(builder.max_tokens, Some(4096));
        assert_eq!(builder.messages.len(), 1);

        let payload = builder.build_payload();
        assert_eq!(payload["model"], "deepseek-chat");
        assert!((payload["temperature"].as_f64().unwrap() - 0.2).abs() < 1e-4);
        assert_eq!(payload["max_tokens"], 4096);
    }

    #[test]
    fn test_parser_multi_line_stream_session() {
        let mut parser = OpenAiSseParser::new();
        let sse_stream = r#"
: keep-alive

data: {"id":"chatcmpl-99","choices":[{"index":0,"delta":{"role":"assistant","reasoning_content":"Let's ponder..."}}]}

data: {"id":"chatcmpl-99","choices":[{"index":0,"delta":{"content":"Here is the answer."}}]}

data: {"id":"chatcmpl-99","choices":[{"index":0,"delta":{},"finish_reason":"stop"}],"usage":{"prompt_tokens":15,"completion_tokens":25}}

data: [DONE]
"#;

        let chunks = parser.parse_chunk(sse_stream);
        assert_eq!(chunks.len(), 3);
        match &chunks[0] {
            StreamChunk::ThinkingDelta(t) => assert_eq!(t, "Let's ponder..."),
            other => panic!("Unexpected chunk 0: {:?}", other),
        }
        match &chunks[1] {
            StreamChunk::ContentDelta(c) => assert_eq!(c, "Here is the answer."),
            other => panic!("Unexpected chunk 1: {:?}", other),
        }
        match &chunks[2] {
            StreamChunk::Done {
                finish_reason,
                prompt_tokens,
                completion_tokens,
            } => {
                assert_eq!(finish_reason.as_deref(), Some("stop"));
                assert_eq!(*prompt_tokens, Some(15));
                assert_eq!(*completion_tokens, Some(25));
            }
            other => panic!("Unexpected chunk 2: {:?}", other),
        }
    }
}
