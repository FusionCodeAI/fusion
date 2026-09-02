use std::collections::BTreeMap;
use std::time::Duration;

use anyhow::Context;
use eventsource_stream::Eventsource;
use futures::StreamExt;
use reqwest::header::{HeaderMap, HeaderValue, AUTHORIZATION, CONTENT_TYPE};
use serde_json::{json, Value};
use tokio::sync::mpsc;

use crate::config::Config;
use crate::provider::types::{Message, Role, StreamChunk, ToolCall, ToolDefinition};

#[derive(Clone)]
pub struct LlmClient {
    client: reqwest::Client,
}

impl Default for LlmClient {
    fn default() -> Self {
        Self::new()
    }
}

impl LlmClient {
    pub fn new() -> Self {
        let client = reqwest::Client::builder()
            .use_rustls_tls()
            .connect_timeout(Duration::from_secs(30))
            .build()
            .unwrap_or_else(|_| reqwest::Client::new());
        Self { client }
    }

    pub fn with_client(client: reqwest::Client) -> Self {
        Self { client }
    }

    /// Stream a chat completion using the provided Config settings.
    pub async fn stream_chat(
        &self,
        config: &Config,
        messages: &[Message],
        tools: &[ToolDefinition],
    ) -> anyhow::Result<mpsc::Receiver<StreamChunk>> {
        let (key, url) = config.get_key_and_url(&config.default_provider);
        self.stream_chat_with(
            &config.default_provider,
            &config.default_model,
            config.default_temperature,
            config.max_tokens,
            key.as_deref(),
            &url,
            messages,
            tools,
        )
        .await
    }

    /// Stream a chat completion with explicit provider and connection parameters.
    pub async fn stream_chat_with(
        &self,
        provider: &str,
        model: &str,
        temperature: Option<f32>,
        max_tokens: Option<u32>,
        api_key: Option<&str>,
        base_url: &str,
        messages: &[Message],
        tools: &[ToolDefinition],
    ) -> anyhow::Result<mpsc::Receiver<StreamChunk>> {
        let provider_lower = provider.to_lowercase();
        if provider_lower == "anthropic" {
            self.stream_anthropic(
                model,
                temperature,
                max_tokens,
                api_key,
                base_url,
                messages,
                tools,
            )
            .await
        } else if provider_lower == "ollama" {
            let ollama_client = crate::provider::ollama::OllamaClient::with_client(
                self.client.clone(),
                Some(base_url),
            );
            ollama_client
                .stream_chat(
                    model,
                    temperature,
                    max_tokens,
                    api_key,
                    messages,
                    tools,
                )
                .await
        } else {
            self.stream_openai_compatible(
                &provider_lower,
                model,
                temperature,
                max_tokens,
                api_key,
                base_url,
                messages,
                tools,
            )
            .await
        }
    }

    /// Helper method to collect the full chat completion into text, optional thinking, and tool calls.
    pub async fn complete(
        &self,
        config: &Config,
        messages: &[Message],
        tools: &[ToolDefinition],
    ) -> anyhow::Result<(String, Option<String>, Vec<ToolCall>)> {
        let mut rx = self.stream_chat(config, messages, tools).await?;
        Self::aggregate_stream(&mut rx).await
    }

    /// Helper method to collect the full chat completion with explicit provider parameters.
    pub async fn complete_with(
        &self,
        provider: &str,
        model: &str,
        temperature: Option<f32>,
        max_tokens: Option<u32>,
        api_key: Option<&str>,
        base_url: &str,
        messages: &[Message],
        tools: &[ToolDefinition],
    ) -> anyhow::Result<(String, Option<String>, Vec<ToolCall>)> {
        let mut rx = self
            .stream_chat_with(
                provider,
                model,
                temperature,
                max_tokens,
                api_key,
                base_url,
                messages,
                tools,
            )
            .await?;
        Self::aggregate_stream(&mut rx).await
    }

    /// Aggregate a stream of `StreamChunk` into a complete response.
    pub async fn aggregate_stream(
        rx: &mut mpsc::Receiver<StreamChunk>,
    ) -> anyhow::Result<(String, Option<String>, Vec<ToolCall>)> {
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

    // ==========================================
    // OpenAI-Compatible Streaming Implementation
    // ==========================================

    #[allow(clippy::too_many_arguments)]
    async fn stream_openai_compatible(
        &self,
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

        if provider == "openrouter" || base_url.contains("openrouter.ai") {
            if let Ok(hv) = HeaderValue::from_str("https://github.com/theaungmyatmoe/fusion") {
                headers.insert("HTTP-Referer", hv);
            }
            if let Ok(hv) = HeaderValue::from_str("Fusion AI Assistant") {
                headers.insert("X-Title", hv);
            }
        }

        let payload = build_openai_payload(model, temperature, max_tokens, messages, tools);

        let response = self
            .client
            .post(&url)
            .headers(headers)
            .json(&payload)
            .send()
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
                            // Extract usage if available
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

                            // Extract choices
                            if let Some(choices) = val.get("choices").and_then(|v| v.as_array()) {
                                for choice in choices {
                                    let finish_reason = choice
                                        .get("finish_reason")
                                        .and_then(|v| v.as_str())
                                        .map(|s| s.to_string());

                                    if let Some(delta) = choice.get("delta") {
                                        // DeepSeek R1 / Reasoning models
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

                                        // Standard text content
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

                                        // Tool calls
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

    // ==========================================
    // Anthropic Messages Streaming Implementation
    // ==========================================

    async fn stream_anthropic(
        &self,
        model: &str,
        temperature: Option<f32>,
        max_tokens: Option<u32>,
        api_key: Option<&str>,
        base_url: &str,
        messages: &[Message],
        tools: &[ToolDefinition],
    ) -> anyhow::Result<mpsc::Receiver<StreamChunk>> {
        let url = construct_anthropic_url(base_url);
        let mut headers = HeaderMap::new();
        headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
        headers.insert(
            "anthropic-version",
            HeaderValue::from_static("2023-06-01"),
        );

        if let Some(key) = api_key {
            let key = key.trim();
            if !key.is_empty() {
                if let Ok(hv) = HeaderValue::from_str(key) {
                    headers.insert("x-api-key", hv);
                }
            }
        }

        let payload = build_anthropic_payload(model, temperature, max_tokens, messages, tools);

        let response = self
            .client
            .post(&url)
            .headers(headers)
            .json(&payload)
            .send()
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
            let mut prompt_tokens = None;
            let mut completion_tokens = None;
            let mut finish_reason = None;
            let mut done_sent = false;

            while let Some(event_res) = stream.next().await {
                match event_res {
                    Ok(event) => {
                        let data = event.data.trim();
                        if data.is_empty() {
                            continue;
                        }

                        if let Ok(val) = serde_json::from_str::<Value>(data) {
                            let event_type = val
                                .get("type")
                                .and_then(|v| v.as_str())
                                .unwrap_or(event.event.as_str());

                            match event_type {
                                "message_start" => {
                                    if let Some(msg) = val.get("message") {
                                        if let Some(usage) = msg.get("usage") {
                                            if let Some(it) = usage
                                                .get("input_tokens")
                                                .and_then(|v| v.as_u64())
                                            {
                                                prompt_tokens = Some(it as u32);
                                            }
                                        }
                                    }
                                }
                                "content_block_start" => {
                                    let index = val
                                        .get("index")
                                        .and_then(|v| v.as_u64())
                                        .unwrap_or(0)
                                        as usize;
                                    if let Some(cb) = val.get("content_block") {
                                        let cb_type = cb.get("type").and_then(|v| v.as_str());
                                        if cb_type == Some("tool_use") {
                                            let id = cb
                                                .get("id")
                                                .and_then(|v| v.as_str())
                                                .map(|s| s.to_string());
                                            let name = cb
                                                .get("name")
                                                .and_then(|v| v.as_str())
                                                .map(|s| s.to_string());
                                            let _ = tx
                                                .send(StreamChunk::ToolCallDelta {
                                                    index,
                                                    id,
                                                    name,
                                                    arguments_delta: String::new(),
                                                })
                                                .await;
                                        }
                                    }
                                }
                                "content_block_delta" => {
                                    let index = val
                                        .get("index")
                                        .and_then(|v| v.as_u64())
                                        .unwrap_or(0)
                                        as usize;
                                    if let Some(delta) = val.get("delta") {
                                        let delta_type =
                                            delta.get("type").and_then(|v| v.as_str());
                                        match delta_type {
                                            Some("text_delta") => {
                                                if let Some(text) =
                                                    delta.get("text").and_then(|v| v.as_str())
                                                {
                                                    if !text.is_empty() {
                                                        let _ = tx
                                                            .send(StreamChunk::ContentDelta(
                                                                text.to_string(),
                                                            ))
                                                            .await;
                                                    }
                                                }
                                            }
                                            Some("thinking_delta") => {
                                                if let Some(thinking) = delta
                                                    .get("thinking")
                                                    .and_then(|v| v.as_str())
                                                {
                                                    if !thinking.is_empty() {
                                                        let _ = tx
                                                            .send(StreamChunk::ThinkingDelta(
                                                                thinking.to_string(),
                                                            ))
                                                            .await;
                                                    }
                                                }
                                            }
                                            Some("input_json_delta") => {
                                                let partial = delta
                                                    .get("partial_json")
                                                    .and_then(|v| v.as_str())
                                                    .unwrap_or("")
                                                    .to_string();
                                                let _ = tx
                                                    .send(StreamChunk::ToolCallDelta {
                                                        index,
                                                        id: None,
                                                        name: None,
                                                        arguments_delta: partial,
                                                    })
                                                    .await;
                                            }
                                            _ => {}
                                        }
                                    }
                                }
                                "content_block_stop" => {}
                                "message_delta" => {
                                    if let Some(delta) = val.get("delta") {
                                        if let Some(sr) = delta
                                            .get("stop_reason")
                                            .and_then(|v| v.as_str())
                                        {
                                            finish_reason = Some(sr.to_string());
                                        }
                                    }
                                    if let Some(usage) = val.get("usage") {
                                        if let Some(ot) =
                                            usage.get("output_tokens").and_then(|v| v.as_u64())
                                        {
                                            completion_tokens = Some(ot as u32);
                                        }
                                    }
                                }
                                "message_stop" => {
                                    if !done_sent {
                                        let _ = tx
                                            .send(StreamChunk::Done {
                                                finish_reason: finish_reason.clone(),
                                                prompt_tokens,
                                                completion_tokens,
                                            })
                                            .await;
                                        done_sent = true;
                                    }
                                }
                                "error" => {
                                    let err_msg = val
                                        .get("error")
                                        .and_then(|e| e.get("message"))
                                        .and_then(|v| v.as_str())
                                        .unwrap_or("Unknown Anthropic error");
                                    let _ = tx
                                        .send(StreamChunk::Error(err_msg.to_string()))
                                        .await;
                                }
                                _ => {}
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
                        finish_reason,
                        prompt_tokens,
                        completion_tokens,
                    })
                    .await;
            }
        });

        Ok(rx)
    }
}

pub fn construct_openai_url(base_url: &str) -> String {
    let base = base_url.trim_end_matches('/');
    if base.ends_with("/chat/completions") {
        base.to_string()
    } else if base.ends_with("/v1") {
        format!("{}/chat/completions", base)
    } else if base.contains(":11434") {
        format!("{}/v1/chat/completions", base)
    } else {
        format!("{}/chat/completions", base)
    }
}

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

pub fn build_openai_payload(
    model: &str,
    temperature: Option<f32>,
    max_tokens: Option<u32>,
    messages: &[Message],
    tools: &[ToolDefinition],
) -> Value {
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

    payload
}

pub fn build_anthropic_payload(
    model: &str,
    temperature: Option<f32>,
    max_tokens: Option<u32>,
    messages: &[Message],
    tools: &[ToolDefinition],
) -> Value {
    let mut payload = json!({
        "model": model,
        "max_tokens": max_tokens.unwrap_or(4096),
        "stream": true,
    });

    if let Some(temp) = temperature {
        payload["temperature"] = json!(temp);
    }

    // Extract system prompt
    let system_prompts: Vec<&str> = messages
        .iter()
        .filter(|m| m.role == Role::System)
        .map(|m| m.content.as_str())
        .filter(|c| !c.is_empty())
        .collect();

    if !system_prompts.is_empty() {
        payload["system"] = json!(system_prompts.join("\n\n"));
    }

    // Convert non-system messages
    let mut anthropic_messages: Vec<Value> = Vec::new();
    for msg in messages {
        match msg.role {
            Role::System => {
                // Handled in system parameter above
                continue;
            }
            Role::User => {
                anthropic_messages.push(json!({
                    "role": "user",
                    "content": msg.content,
                }));
            }
            Role::Assistant => {
                if let Some(tool_calls) = &msg.tool_calls {
                    if !tool_calls.is_empty() {
                        let mut content_arr = Vec::new();
                        if !msg.content.is_empty() {
                            content_arr.push(json!({
                                "type": "text",
                                "text": msg.content,
                            }));
                        }
                        for tc in tool_calls {
                            let input_val: Value =
                                serde_json::from_str(&tc.arguments).unwrap_or_else(|_| json!({}));
                            content_arr.push(json!({
                                "type": "tool_use",
                                "id": tc.id,
                                "name": tc.name,
                                "input": input_val,
                            }));
                        }
                        anthropic_messages.push(json!({
                            "role": "assistant",
                            "content": content_arr,
                        }));
                    } else {
                        anthropic_messages.push(json!({
                            "role": "assistant",
                            "content": msg.content,
                        }));
                    }
                } else {
                    anthropic_messages.push(json!({
                        "role": "assistant",
                        "content": msg.content,
                    }));
                }
            }
            Role::Tool => {
                let tool_use_id = msg.tool_call_id.clone().unwrap_or_default();
                anthropic_messages.push(json!({
                    "role": "user",
                    "content": [
                        {
                            "type": "tool_result",
                            "tool_use_id": tool_use_id,
                            "content": msg.content,
                        }
                    ]
                }));
            }
        }
    }
    payload["messages"] = json!(anthropic_messages);

    if !tools.is_empty() {
        let tools_json: Vec<Value> = tools
            .iter()
            .map(|t| {
                json!({
                    "name": t.name,
                    "description": t.description,
                    "input_schema": t.parameters,
                })
            })
            .collect();
        payload["tools"] = json!(tools_json);
    }

    payload
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_url_construction() {
        assert_eq!(
            construct_openai_url("https://api.openai.com/v1"),
            "https://api.openai.com/v1/chat/completions"
        );
        assert_eq!(
            construct_openai_url("https://api.deepseek.com"),
            "https://api.deepseek.com/chat/completions"
        );
        assert_eq!(
            construct_openai_url("http://localhost:11434"),
            "http://localhost:11434/v1/chat/completions"
        );
        assert_eq!(
            construct_openai_url("https://openrouter.ai/api/v1"),
            "https://openrouter.ai/api/v1/chat/completions"
        );
        assert_eq!(
            construct_openai_url("https://api.openai.com/v1/chat/completions"),
            "https://api.openai.com/v1/chat/completions"
        );

        assert_eq!(
            construct_anthropic_url("https://api.anthropic.com/v1"),
            "https://api.anthropic.com/v1/messages"
        );
        assert_eq!(
            construct_anthropic_url("https://api.anthropic.com"),
            "https://api.anthropic.com/v1/messages"
        );
        assert_eq!(
            construct_anthropic_url("https://api.anthropic.com/v1/messages"),
            "https://api.anthropic.com/v1/messages"
        );
    }

    #[test]
    fn test_build_openai_payload() {
        let messages = vec![
            Message::system("You are a helpful assistant."),
            Message::user("Hello!"),
            Message::assistant_with_tools(
                "Let me check",
                vec![ToolCall {
                    id: "call_1".to_string(),
                    name: "read".to_string(),
                    arguments: r#"{"path":"foo.rs"}"#.to_string(),
                }],
            ),
            Message::tool_result("call_1", "file content"),
        ];

        let tools = vec![ToolDefinition {
            name: "read".to_string(),
            description: "Read a file".to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string" }
                },
                "required": ["path"]
            }),
        }];

        let payload = build_openai_payload("gpt-4o", Some(0.7), Some(2048), &messages, &tools);

        assert_eq!(payload["model"], "gpt-4o");
        assert_eq!(payload["stream"], true);
        assert!((payload["temperature"].as_f64().unwrap() - 0.7).abs() < 1e-4);
        assert_eq!(payload["max_tokens"], 2048);

        let msgs = payload["messages"].as_array().unwrap();
        assert_eq!(msgs.len(), 4);
        assert_eq!(msgs[0]["role"], "system");
        assert_eq!(msgs[0]["content"], "You are a helpful assistant.");

        assert_eq!(msgs[1]["role"], "user");
        assert_eq!(msgs[1]["content"], "Hello!");

        assert_eq!(msgs[2]["role"], "assistant");
        assert_eq!(msgs[2]["content"], "Let me check");
        assert_eq!(msgs[2]["tool_calls"][0]["id"], "call_1");
        assert_eq!(msgs[2]["tool_calls"][0]["function"]["name"], "read");

        assert_eq!(msgs[3]["role"], "tool");
        assert_eq!(msgs[3]["tool_call_id"], "call_1");
        assert_eq!(msgs[3]["content"], "file content");

        let tools_out = payload["tools"].as_array().unwrap();
        assert_eq!(tools_out.len(), 1);
        assert_eq!(tools_out[0]["function"]["name"], "read");
    }

    #[test]
    fn test_build_anthropic_payload() {
        let messages = vec![
            Message::system("System instruction"),
            Message::user("Please read the file"),
            Message::assistant_with_tools(
                "Reading it now",
                vec![ToolCall {
                    id: "toolu_123".to_string(),
                    name: "read".to_string(),
                    arguments: r#"{"path":"test.txt"}"#.to_string(),
                }],
            ),
            Message::tool_result("toolu_123", "contents"),
        ];

        let tools = vec![ToolDefinition {
            name: "read".to_string(),
            description: "Read a file".to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string" }
                }
            }),
        }];

        let payload =
            build_anthropic_payload("claude-3-5-sonnet", None, Some(4096), &messages, &tools);

        assert_eq!(payload["model"], "claude-3-5-sonnet");
        assert_eq!(payload["max_tokens"], 4096);
        assert_eq!(payload["system"], "System instruction");

        let msgs = payload["messages"].as_array().unwrap();
        assert_eq!(msgs.len(), 3); // system is extracted

        assert_eq!(msgs[0]["role"], "user");
        assert_eq!(msgs[0]["content"], "Please read the file");

        assert_eq!(msgs[1]["role"], "assistant");
        let content_arr = msgs[1]["content"].as_array().unwrap();
        assert_eq!(content_arr[0]["type"], "text");
        assert_eq!(content_arr[0]["text"], "Reading it now");
        assert_eq!(content_arr[1]["type"], "tool_use");
        assert_eq!(content_arr[1]["id"], "toolu_123");
        assert_eq!(content_arr[1]["name"], "read");
        assert_eq!(content_arr[1]["input"]["path"], "test.txt");

        assert_eq!(msgs[2]["role"], "user");
        assert_eq!(msgs[2]["content"][0]["type"], "tool_result");
        assert_eq!(msgs[2]["content"][0]["tool_use_id"], "toolu_123");
        assert_eq!(msgs[2]["content"][0]["content"], "contents");

        let tools_out = payload["tools"].as_array().unwrap();
        assert_eq!(tools_out[0]["name"], "read");
        assert_eq!(tools_out[0]["input_schema"]["type"], "object");
    }

    #[tokio::test]
    async fn test_aggregate_stream() {
        let (tx, mut rx) = mpsc::channel(16);

        tokio::spawn(async move {
            let _ = tx
                .send(StreamChunk::ThinkingDelta("Thinking step 1...".into()))
                .await;
            let _ = tx
                .send(StreamChunk::ContentDelta("Hello, ".into()))
                .await;
            let _ = tx
                .send(StreamChunk::ContentDelta("world!".into()))
                .await;
            let _ = tx
                .send(StreamChunk::ToolCallDelta {
                    index: 0,
                    id: Some("tc_1".into()),
                    name: Some("read".into()),
                    arguments_delta: r#"{"pa"#.into(),
                })
                .await;
            let _ = tx
                .send(StreamChunk::ToolCallDelta {
                    index: 0,
                    id: None,
                    name: None,
                    arguments_delta: r#"th":"file.txt"}"#.into(),
                })
                .await;
            let _ = tx
                .send(StreamChunk::Done {
                    finish_reason: Some("tool_calls".into()),
                    prompt_tokens: Some(10),
                    completion_tokens: Some(25),
                })
                .await;
        });

        let (content, thinking, tool_calls) =
            LlmClient::aggregate_stream(&mut rx).await.unwrap();

        assert_eq!(content, "Hello, world!");
        assert_eq!(thinking, Some("Thinking step 1...".to_string()));
        assert_eq!(tool_calls.len(), 1);
        assert_eq!(tool_calls[0].id, "tc_1");
        assert_eq!(tool_calls[0].name, "read");
        assert_eq!(tool_calls[0].arguments, r#"{"path":"file.txt"}"#);
    }
}
