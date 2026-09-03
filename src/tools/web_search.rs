//! Web search tool for live documentation and technical references lookup.
//!
//! Provides zero-API-key web search using DuckDuckGo (HTML and Lite) and
//! SearXNG JSON endpoints. Designed for lightweight, fast retrieval without
//! external credentials or complex dependencies.

use async_trait::async_trait;
use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::time::Duration;

use crate::tools::types::{Tool, ToolContext};

/// User-Agent header mimicking a standard browser to avoid anti-bot blocks on public HTML endpoints.
const DEFAULT_USER_AGENT: &str =
    "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/124.0.0.0 Safari/537.36";

/// Individual search result item.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SearchResult {
    pub title: String,
    pub url: String,
    pub snippet: String,
}

/// Lightweight web search tool using DuckDuckGo or SearXNG.
#[derive(Clone)]
pub struct WebSearchTool {
    client: reqwest::Client,
    default_searxng_url: Option<String>,
}

impl Default for WebSearchTool {
    fn default() -> Self {
        Self::new()
    }
}

impl WebSearchTool {
    /// Create a new web search tool with default network timeouts and pure-Rust TLS.
    pub fn new() -> Self {
        let client = reqwest::Client::builder()
            .use_rustls_tls()
            .connect_timeout(Duration::from_secs(10))
            .timeout(Duration::from_secs(15))
            .build()
            .unwrap_or_else(|_| reqwest::Client::new());

        Self {
            client,
            default_searxng_url: None,
        }
    }

    /// Create with a custom HTTP client (useful for mocking or custom proxy settings).
    pub fn with_client(client: reqwest::Client) -> Self {
        Self {
            client,
            default_searxng_url: None,
        }
    }

    /// Set a default SearXNG instance URL.
    pub fn with_searxng_url(mut self, url: impl Into<String>) -> Self {
        self.default_searxng_url = Some(url.into());
        self
    }

    /// Format results into clean, readable Markdown text suitable for LLM consumption.
    pub fn format_results(query: &str, results: &[SearchResult]) -> String {
        if results.is_empty() {
            return format!("No search results found for: \"{}\"", query);
        }

        let mut out = format!("Search results for: \"{}\"\n\n", query);
        for (i, r) in results.iter().enumerate() {
            out.push_str(&format!("{}. **{}**\n", i + 1, r.title));
            out.push_str(&format!("   URL: {}\n", r.url));
            if !r.snippet.is_empty() {
                out.push_str(&format!("   Summary: {}\n", r.snippet));
            }
            out.push('\n');
        }
        out.trim_end().to_string()
    }

    /// Execute a search against DuckDuckGo HTML, falling back to DuckDuckGo Lite if needed.
    pub async fn search_duckduckgo(
        &self,
        query: &str,
        limit: usize,
    ) -> anyhow::Result<Vec<SearchResult>> {
        let encoded_query = url_encode(query);
        let ddg_html_url = format!("https://html.duckduckgo.com/html/?q={}", encoded_query);

        let response = self
            .client
            .get(&ddg_html_url)
            .header(reqwest::header::USER_AGENT, DEFAULT_USER_AGENT)
            .header(
                reqwest::header::ACCEPT,
                "text/html,application/xhtml+xml,application/xml;q=0.9,*/*;q=0.8",
            )
            .header(reqwest::header::ACCEPT_LANGUAGE, "en-US,en;q=0.9")
            .send()
            .await;

        if let Ok(res) = response {
            if res.status().is_success() {
                if let Ok(body) = res.text().await {
                    let results = parse_ddg_html(&body, limit);
                    if !results.is_empty() {
                        return Ok(results);
                    }
                }
            }
        }

        // Fallback: DuckDuckGo Lite via POST
        let lite_res = self
            .client
            .post("https://lite.duckduckgo.com/lite/")
            .header(reqwest::header::USER_AGENT, DEFAULT_USER_AGENT)
            .header(
                reqwest::header::CONTENT_TYPE,
                "application/x-www-form-urlencoded",
            )
            .body(format!("q={}", encoded_query))
            .send()
            .await;

        if let Ok(res) = lite_res {
            if res.status().is_success() {
                if let Ok(body) = res.text().await {
                    let results = parse_ddg_lite(&body, limit);
                    if !results.is_empty() {
                        return Ok(results);
                    }
                }
            }
        }
        // Automatic fallbacks when DuckDuckGo HTML/Lite are blocked by CAPTCHA
        if let Ok(gh) = self.search_github(query, limit).await {
            if !gh.is_empty() {
                return Ok(gh);
            }
        }

        if let Ok(wiki) = self.search_wikipedia(query, limit).await {
            if !wiki.is_empty() {
                return Ok(wiki);
            }
        }

        Ok(Vec::new())
    }

    /// Fallback search using Wikipedia Search API.
    pub async fn search_wikipedia(
        &self,
        query: &str,
        limit: usize,
    ) -> anyhow::Result<Vec<SearchResult>> {
        let encoded = url_encode(query);
        let url = format!(
            "https://en.wikipedia.org/w/api.php?action=query&list=search&srsearch={}&format=json&utf8=1",
            encoded
        );

        let res = self
            .client
            .get(&url)
            .header(reqwest::header::USER_AGENT, DEFAULT_USER_AGENT)
            .send()
            .await?;

        if !res.status().is_success() {
            anyhow::bail!("Wikipedia API returned status: {}", res.status());
        }

        let val: Value = res.json().await?;
        let mut results = Vec::new();

        if let Some(items) = val.pointer("/query/search").and_then(|v| v.as_array()) {
            for item in items.iter().take(limit) {
                let title = item
                    .get("title")
                    .and_then(|v| v.as_str())
                    .unwrap_or_default();
                let snippet = item
                    .get("snippet")
                    .and_then(|v| v.as_str())
                    .unwrap_or_default();
                let clean_snippet = decode_html_entities(&strip_html_tags(snippet));
                let page_url = format!("https://en.wikipedia.org/wiki/{}", url_encode(title));

                if !title.is_empty() {
                    results.push(SearchResult {
                        title: title.to_string(),
                        url: page_url,
                        snippet: clean_snippet,
                    });
                }
            }
        }

        Ok(results)
    }

    /// Fallback search using GitHub API for developers, organizations, and repositories.
    pub async fn search_github(
        &self,
        query: &str,
        limit: usize,
    ) -> anyhow::Result<Vec<SearchResult>> {
        let encoded = url_encode(query);
        let mut results = Vec::new();

        // 1. Search GitHub users / orgs
        let user_url = format!(
            "https://api.github.com/search/users?q={}&per_page={}",
            encoded,
            limit.min(5)
        );
        if let Ok(res) = self
            .client
            .get(&user_url)
            .header(reqwest::header::USER_AGENT, DEFAULT_USER_AGENT)
            .send()
            .await
        {
            if res.status().is_success() {
                if let Ok(val) = res.json::<Value>().await {
                    if let Some(items) = val.get("items").and_then(|v| v.as_array()) {
                        for item in items {
                            let login = item
                                .get("login")
                                .and_then(|v| v.as_str())
                                .unwrap_or_default();
                            let html_url = item
                                .get("html_url")
                                .and_then(|v| v.as_str())
                                .unwrap_or_default();
                            let user_type =
                                item.get("type").and_then(|v| v.as_str()).unwrap_or("User");
                            if !login.is_empty() && !html_url.is_empty() {
                                results.push(SearchResult {
                                    title: format!("{} ({}) on GitHub", login, user_type),
                                    url: html_url.to_string(),
                                    snippet: format!(
                                        "GitHub profile for {} ({}).",
                                        login, user_type
                                    ),
                                });
                            }
                        }
                    }
                }
            }
        }

        // 2. Search GitHub repositories
        let repo_url = format!(
            "https://api.github.com/search/repositories?q={}&per_page={}",
            encoded,
            limit.min(5)
        );
        if let Ok(res) = self
            .client
            .get(&repo_url)
            .header(reqwest::header::USER_AGENT, DEFAULT_USER_AGENT)
            .send()
            .await
        {
            if res.status().is_success() {
                if let Ok(val) = res.json::<Value>().await {
                    if let Some(items) = val.get("items").and_then(|v| v.as_array()) {
                        for item in items {
                            let full_name = item
                                .get("full_name")
                                .and_then(|v| v.as_str())
                                .unwrap_or_default();
                            let html_url = item
                                .get("html_url")
                                .and_then(|v| v.as_str())
                                .unwrap_or_default();
                            let desc = item
                                .get("description")
                                .and_then(|v| v.as_str())
                                .unwrap_or_default();
                            if !full_name.is_empty() && !html_url.is_empty() {
                                results.push(SearchResult {
                                    title: format!("{} on GitHub", full_name),
                                    url: html_url.to_string(),
                                    snippet: desc.to_string(),
                                });
                            }
                        }
                    }
                }
            }
        }

        results.truncate(limit);
        Ok(results)
    }
    /// Execute a search against a SearXNG instance JSON endpoint.
    pub async fn search_searxng(
        &self,
        instance_url: &str,
        query: &str,
        limit: usize,
    ) -> anyhow::Result<Vec<SearchResult>> {
        let base = instance_url.trim_end_matches('/');
        let encoded_query = url_encode(query);
        let search_url = format!(
            "{}/search?q={}&format=json&categories=general",
            base, encoded_query
        );

        let res = self
            .client
            .get(&search_url)
            .header(reqwest::header::USER_AGENT, DEFAULT_USER_AGENT)
            .header(reqwest::header::ACCEPT, "application/json")
            .send()
            .await
            .map_err(|e| anyhow::anyhow!("SearXNG request failed: {}", e))?;

        if !res.status().is_success() {
            anyhow::bail!("SearXNG returned HTTP status: {}", res.status());
        }

        let json_val: Value = res
            .json()
            .await
            .map_err(|e| anyhow::anyhow!("Failed to parse SearXNG JSON: {}", e))?;

        Ok(parse_searxng_json(&json_val, limit))
    }

    /// Perform search with auto engine detection or explicit provider selection.
    pub async fn search(
        &self,
        query: &str,
        limit: usize,
        engine: &str,
        searxng_url: Option<&str>,
    ) -> anyhow::Result<Vec<SearchResult>> {
        let chosen_searxng = searxng_url
            .map(str::to_string)
            .or_else(|| self.default_searxng_url.clone())
            .or_else(|| std::env::var("SEARXNG_URL").ok())
            .or_else(|| std::env::var("FUSION_SEARXNG_URL").ok());

        match engine.to_lowercase().as_str() {
            "searxng" => {
                let url = chosen_searxng.ok_or_else(|| {
                    anyhow::anyhow!(
                        "SearXNG engine requested but no SearXNG URL provided via searxng_url parameter or SEARXNG_URL environment variable"
                    )
                })?;
                self.search_searxng(&url, query, limit).await
            }
            "duckduckgo" | "ddg" => self.search_duckduckgo(query, limit).await,
            "auto" | "" => {
                // If a SearXNG URL is explicitly configured in env or params, try it first; else DuckDuckGo
                if let Some(url) = &chosen_searxng {
                    match self.search_searxng(url, query, limit).await {
                        Ok(res) if !res.is_empty() => return Ok(res),
                        _ => {
                            // Fallback to DuckDuckGo on SearXNG failure in auto mode
                        }
                    }
                }
                self.search_duckduckgo(query, limit).await
            }
            other => anyhow::bail!(
                "Unsupported search engine '{}'. Choose 'auto', 'duckduckgo', or 'searxng'.",
                other
            ),
        }
    }
}

#[async_trait]
impl Tool for WebSearchTool {
    fn name(&self) -> &str {
        "web_search"
    }

    fn description(&self) -> &str {
        "Search the web using DuckDuckGo or SearXNG (no API key required) for live documentation, APIs, technical references, and code examples."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "query": {
                    "type": "string",
                    "description": "The search query (e.g. 'rust tokio tutorial', 'site:docs.rs reqwest', 'actix-web websocket example')."
                },
                "limit": {
                    "type": "integer",
                    "description": "Maximum number of search results to return (default: 5, min: 1, max: 20)."
                },
                "engine": {
                    "type": "string",
                    "enum": ["auto", "duckduckgo", "searxng"],
                    "description": "Search engine backend: 'auto' (default), 'duckduckgo', or 'searxng'."
                },
                "searxng_url": {
                    "type": "string",
                    "description": "Optional SearXNG instance base URL (e.g. 'https://searx.be'). Can also be set via SEARXNG_URL environment variable."
                },
                "format": {
                    "type": "string",
                    "enum": ["text", "json"],
                    "description": "Output format: 'text' (default markdown list) or 'json'."
                }
            },
            "required": ["query"]
        })
    }

    async fn execute(&self, args: Value, ctx: &ToolContext) -> anyhow::Result<String> {
        let query = args
            .get("query")
            .and_then(|v| v.as_str())
            .or_else(|| args.get("q").and_then(|v| v.as_str()))
            .ok_or_else(|| anyhow::anyhow!("Missing required parameter: query"))?
            .trim();

        if query.is_empty() {
            anyhow::bail!("Search query cannot be empty");
        }

        let limit = args
            .get("limit")
            .and_then(|v| v.as_u64())
            .or_else(|| args.get("max_results").and_then(|v| v.as_u64()))
            .map(|n| (n as usize).clamp(1, 20))
            .unwrap_or(5);

        let engine = args
            .get("engine")
            .and_then(|v| v.as_str())
            .unwrap_or("auto");

        let searxng_url = args
            .get("searxng_url")
            .and_then(|v| v.as_str())
            .or_else(|| ctx.env.get("SEARXNG_URL").map(|s| s.as_str()));

        let format_type = args
            .get("format")
            .and_then(|v| v.as_str())
            .unwrap_or("text");

        let results = self.search(query, limit, engine, searxng_url).await?;

        if format_type.eq_ignore_ascii_case("json") {
            Ok(serde_json::to_string_pretty(&results)?)
        } else {
            Ok(Self::format_results(query, &results))
        }
    }
}

// ---------------------------------------------------------------------------
// HTML and JSON Parsers
// ---------------------------------------------------------------------------

/// Parse DuckDuckGo HTML search results page.
pub fn parse_ddg_html(html_text: &str, limit: usize) -> Vec<SearchResult> {
    let mut results = Vec::new();

    // Regex to extract title, href, and optional snippet
    // In DDG HTML, each result has <h2 class="result__title">...<a href="...">Title</a></h2>
    // and snippet in <a class="result__snippet" ...>Snippet</a>
    let title_regex = Regex::new(
        r#"(?s)<h2[^>]*class="[^"]*result__title[^"]*"[^>]*>\s*<a[^>]*href="([^"]+)"[^>]*>(.*?)</a>"#,
    )
    .ok();

    let snippet_regex =
        Regex::new(r#"(?s)<a[^>]*class="[^"]*result__snippet[^"]*"[^>]*>(.*?)</a>"#).ok();

    if let Some(t_re) = title_regex {
        // Split by result divider or scan blocks
        let blocks = html_text.split("<div class=\"result results_links");
        for block in blocks.skip(1) {
            if let Some(t_cap) = t_re.captures(block) {
                let raw_href = t_cap.get(1).map_or("", |m| m.as_str());
                let raw_title = t_cap.get(2).map_or("", |m| m.as_str());

                let clean_url = extract_ddg_redirect_url(raw_href);
                let clean_title = decode_html_entities(&strip_html_tags(raw_title));

                let clean_snippet = if let Some(s_re) = &snippet_regex {
                    s_re.captures(block)
                        .and_then(|c| c.get(1))
                        .map(|m| decode_html_entities(&strip_html_tags(m.as_str())))
                        .unwrap_or_default()
                } else {
                    String::new()
                };

                if !clean_title.is_empty() && !clean_url.is_empty() {
                    results.push(SearchResult {
                        title: clean_title,
                        url: clean_url,
                        snippet: clean_snippet,
                    });
                }

                if results.len() >= limit {
                    break;
                }
            }
        }
    }

    results
}

/// Parse DuckDuckGo Lite HTML search results page.
pub fn parse_ddg_lite(html_text: &str, limit: usize) -> Vec<SearchResult> {
    let mut results = Vec::new();

    // DDG Lite rows:
    // <a rel="nofollow" href="..." class='result-link'>Title</a>
    // and subsequent row:
    // <td class='result-snippet'>Snippet</td>
    let link_re = Regex::new(
        r#"(?s)<a\s+[^>]*href=['"]([^'"]+)['"][^>]*class=['"]result-link['"][^>]*>(.*?)</a>"#,
    )
    .ok();
    let alt_link_re = Regex::new(
        r#"(?s)<a\s+[^>]*class=['"]result-link['"][^>]*href=['"]([^'"]+)['"][^>]*>(.*?)</a>"#,
    )
    .ok();
    let snippet_re = Regex::new(r#"(?s)<td[^>]*class=['"]result-snippet['"][^>]*>(.*?)</td>"#).ok();

    if let (Some(l_re), Some(s_re)) = (link_re.or(alt_link_re), snippet_re) {
        // Find positions of result-link
        let matches: Vec<_> = l_re.captures_iter(html_text).collect();
        for (i, cap) in matches.iter().enumerate() {
            let raw_href = cap.get(1).map_or("", |m| m.as_str());
            let raw_title = cap.get(2).map_or("", |m| m.as_str());

            let clean_url = extract_ddg_redirect_url(raw_href);
            let clean_title = decode_html_entities(&strip_html_tags(raw_title));

            // Find snippet in text slice between current result and next result
            let start = cap.get(0).map_or(0, |m| m.end());
            let end = if i + 1 < matches.len() {
                matches[i + 1].get(0).map_or(html_text.len(), |m| m.start())
            } else {
                (start + 2000).min(html_text.len())
            };

            let slice = &html_text[start..end];
            let clean_snippet = s_re
                .captures(slice)
                .and_then(|c| c.get(1))
                .map(|m| decode_html_entities(&strip_html_tags(m.as_str())))
                .unwrap_or_default();

            if !clean_title.is_empty() && !clean_url.is_empty() {
                results.push(SearchResult {
                    title: clean_title,
                    url: clean_url,
                    snippet: clean_snippet,
                });
            }

            if results.len() >= limit {
                break;
            }
        }
    }

    results
}

/// Parse SearXNG JSON API response.
pub fn parse_searxng_json(json_val: &Value, limit: usize) -> Vec<SearchResult> {
    let mut results = Vec::new();

    if let Some(arr) = json_val.get("results").and_then(|v| v.as_array()) {
        for item in arr.iter().take(limit) {
            let title = item
                .get("title")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .trim();

            let url = item
                .get("url")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .trim();

            let snippet = item
                .get("content")
                .or_else(|| item.get("snippet"))
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .trim();

            if !title.is_empty() && !url.is_empty() {
                results.push(SearchResult {
                    title: decode_html_entities(&strip_html_tags(title)),
                    url: url.to_string(),
                    snippet: decode_html_entities(&strip_html_tags(snippet)),
                });
            }
        }
    }

    results
}

// ---------------------------------------------------------------------------
// Text & URL Helpers
// ---------------------------------------------------------------------------

/// Strips HTML tags (`<...>`) and normalizes consecutive whitespace.
pub fn strip_html_tags(input: &str) -> String {
    let mut in_tag = false;
    let mut out = String::with_capacity(input.len());
    for ch in input.chars() {
        if ch == '<' {
            in_tag = true;
        } else if ch == '>' {
            in_tag = false;
        } else if !in_tag {
            out.push(ch);
        }
    }
    let words: Vec<&str> = out.split_whitespace().collect();
    words.join(" ")
}

/// Decodes standard HTML entities (both named and numeric).
pub fn decode_html_entities(input: &str) -> String {
    let mut res = String::with_capacity(input.len());
    let mut chars = input.chars().peekable();

    while let Some(ch) = chars.next() {
        if ch == '&' {
            let mut entity = String::new();
            let mut found_semicolon = false;
            for _ in 0..12 {
                match chars.peek() {
                    Some(';') => {
                        chars.next();
                        found_semicolon = true;
                        break;
                    }
                    Some(&c) if c.is_ascii_alphanumeric() || c == '#' => {
                        entity.push(chars.next().unwrap());
                    }
                    _ => break,
                }
            }

            if found_semicolon {
                if let Some(decoded) = decode_single_entity(&entity) {
                    res.push_str(&decoded);
                    continue;
                }
            }

            res.push('&');
            res.push_str(&entity);
            if found_semicolon {
                res.push(';');
            }
        } else {
            res.push(ch);
        }
    }

    res
}

fn decode_single_entity(entity: &str) -> Option<String> {
    match entity {
        "quot" => Some("\"".to_string()),
        "apos" => Some("'".to_string()),
        "amp" => Some("&".to_string()),
        "lt" => Some("<".to_string()),
        "gt" => Some(">".to_string()),
        "nbsp" => Some(" ".to_string()),
        "copy" => Some("©".to_string()),
        "reg" => Some("®".to_string()),
        "trade" => Some("™".to_string()),
        "mdash" => Some("—".to_string()),
        "ndash" => Some("–".to_string()),
        "hellip" => Some("…".to_string()),
        "bull" => Some("•".to_string()),
        s if s.starts_with("#x") || s.starts_with("#X") => u32::from_str_radix(&s[2..], 16)
            .ok()
            .and_then(char::from_u32)
            .map(|c| c.to_string()),
        s if s.starts_with('#') => s[1..]
            .parse::<u32>()
            .ok()
            .and_then(char::from_u32)
            .map(|c| c.to_string()),
        _ => None,
    }
}

/// Decodes percent-encoded URL sequences like `%20`, `%2F`, etc.
pub fn percent_decode(input: &str) -> String {
    let mut bytes = Vec::with_capacity(input.len());
    let input_bytes = input.as_bytes();
    let mut i = 0;
    while i < input_bytes.len() {
        if input_bytes[i] == b'%' && i + 2 < input_bytes.len() {
            if let Ok(byte) = u8::from_str_radix(
                std::str::from_utf8(&input_bytes[i + 1..i + 3]).unwrap_or(""),
                16,
            ) {
                bytes.push(byte);
                i += 3;
                continue;
            }
        }
        bytes.push(input_bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&bytes).into_owned()
}

/// Percent-encodes a query string for safe URL query parameters.
pub fn url_encode(input: &str) -> String {
    let mut encoded = String::with_capacity(input.len() * 2);
    for byte in input.bytes() {
        match byte {
            b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                encoded.push(byte as char);
            }
            b' ' => encoded.push('+'),
            _ => {
                encoded.push_str(&format!("%{:02X}", byte));
            }
        }
    }
    encoded
}

/// Extracts clean target URL from DuckDuckGo redirect link.
pub fn extract_ddg_redirect_url(raw_href: &str) -> String {
    let unescaped = decode_html_entities(raw_href);
    if let Some(idx) = unescaped.find("uddg=") {
        let after = &unescaped[idx + 5..];
        let end_idx = after.find('&').unwrap_or(after.len());
        let encoded_url = &after[..end_idx];
        return percent_decode(encoded_url);
    }
    if unescaped.starts_with("//") {
        return format!("https:{}", unescaped);
    }
    unescaped
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_url_encode() {
        assert_eq!(url_encode("rust tokio tutorial"), "rust+tokio+tutorial");
        assert_eq!(url_encode("site:docs.rs reqwest"), "site%3Adocs.rs+reqwest");
        assert_eq!(url_encode("foo & bar = 42"), "foo+%26+bar+%3D+42");
    }

    #[test]
    fn test_percent_decode() {
        assert_eq!(
            percent_decode("https%3A%2F%2Ftokio.rs%2Ftokio%2Ftutorial"),
            "https://tokio.rs/tokio/tutorial"
        );
        assert_eq!(percent_decode("hello%20world"), "hello world");
    }

    #[test]
    fn test_decode_html_entities() {
        assert_eq!(
            decode_html_entities("Rust &amp; &quot;Tokio&quot; &lt;runtime&gt;"),
            "Rust & \"Tokio\" <runtime>"
        );
        assert_eq!(
            decode_html_entities("It&#39;s an &#x26; test"),
            "It's an & test"
        );
        assert_eq!(
            decode_html_entities("&copy; 2026 &mdash; Fusion"),
            "© 2026 — Fusion"
        );
    }

    #[test]
    fn test_strip_html_tags() {
        let html = "Learn how to write <b>Tokio</b> applications in <a href=\"#\">Rust</a>.";
        assert_eq!(
            strip_html_tags(html),
            "Learn how to write Tokio applications in Rust."
        );
    }

    #[test]
    fn test_extract_ddg_redirect_url() {
        let raw =
            "//duckduckgo.com/l/?uddg=https%3A%2F%2Ftokio.rs%2Ftokio%2Ftutorial&amp;rut=12345";
        assert_eq!(
            extract_ddg_redirect_url(raw),
            "https://tokio.rs/tokio/tutorial"
        );

        let direct = "https://example.com/page";
        assert_eq!(extract_ddg_redirect_url(direct), "https://example.com/page");
    }

    #[test]
    fn test_parse_ddg_html_fixture() {
        let sample_html = r#"
            <div class="result results_links results_links_deep web-result ">
                <div class="links_main links_deep result__body">
                    <h2 class="result__title">
                        <a class="result__a" href="//duckduckgo.com/l/?uddg=https%3A%2F%2Ftokio.rs%2F&rut=abc">Tutorial | Tokio</a>
                    </h2>
                    <a class="result__snippet" href="//duckduckgo.com/l/?uddg=https%3A%2F%2Ftokio.rs%2F&rut=abc">
                        Learn how to write <b>asynchronous</b> applications in Rust.
                    </a>
                </div>
            </div>
            <div class="result results_links results_links_deep web-result ">
                <div class="links_main links_deep result__body">
                    <h2 class="result__title">
                        <a class="result__a" href="//duckduckgo.com/l/?uddg=https%3A%2F%2Fdocs.rs%2Freqwest&rut=xyz">Reqwest Docs</a>
                    </h2>
                    <a class="result__snippet" href="//duckduckgo.com/l/?uddg=https%3A%2F%2Fdocs.rs%2Freqwest&rut=xyz">
                        High level HTTP client for Rust.
                    </a>
                </div>
            </div>
        "#;

        let results = parse_ddg_html(sample_html, 5);
        assert_eq!(results.len(), 2);
        assert_eq!(results[0].title, "Tutorial | Tokio");
        assert_eq!(results[0].url, "https://tokio.rs/");
        assert_eq!(
            results[0].snippet,
            "Learn how to write asynchronous applications in Rust."
        );

        assert_eq!(results[1].title, "Reqwest Docs");
        assert_eq!(results[1].url, "https://docs.rs/reqwest");
    }

    #[test]
    fn test_parse_ddg_lite_fixture() {
        let sample_lite = r#"
            <table>
            <tr>
                <td>
                    <a rel="nofollow" href="https://tokio.rs/tokio/tutorial" class='result-link'>Tutorial | Tokio Runtime</a>
                </td>
            </tr>
            <tr>
                <td>&nbsp;</td>
                <td class='result-snippet'>
                    Learn how to build <b>async</b> services with Tokio in Rust.
                </td>
            </tr>
            <tr>
                <td>
                    <a rel="nofollow" href="https://github.com/tokio-rs/tokio" class='result-link'>GitHub Tokio</a>
                </td>
            </tr>
            <tr>
                <td>&nbsp;</td>
                <td class='result-snippet'>
                    Tokio source code and repository.
                </td>
            </tr>
            </table>
        "#;

        let results = parse_ddg_lite(sample_lite, 5);
        assert_eq!(results.len(), 2);
        assert_eq!(results[0].title, "Tutorial | Tokio Runtime");
        assert_eq!(results[0].url, "https://tokio.rs/tokio/tutorial");
        assert_eq!(
            results[0].snippet,
            "Learn how to build async services with Tokio in Rust."
        );
        assert_eq!(results[1].title, "GitHub Tokio");
        assert_eq!(results[1].url, "https://github.com/tokio-rs/tokio");
    }

    #[test]
    fn test_parse_searxng_json_fixture() {
        let sample_json = json!({
            "query": "rust tokio",
            "results": [
                {
                    "title": "Tokio &mdash; An asynchronous Rust runtime",
                    "url": "https://tokio.rs",
                    "content": "A runtime for writing reliable, <b>asynchronous</b> applications with Rust."
                },
                {
                    "title": "tokio - crates.io",
                    "url": "https://crates.io/crates/tokio",
                    "content": "An event-driven, non-blocking I/O platform."
                }
            ]
        });

        let results = parse_searxng_json(&sample_json, 10);
        assert_eq!(results.len(), 2);
        assert_eq!(results[0].title, "Tokio — An asynchronous Rust runtime");
        assert_eq!(results[0].url, "https://tokio.rs");
        assert_eq!(
            results[0].snippet,
            "A runtime for writing reliable, asynchronous applications with Rust."
        );
    }

    #[test]
    fn test_web_search_tool_parameters_and_definition() {
        let tool = WebSearchTool::new();
        assert_eq!(tool.name(), "web_search");
        assert!(!tool.description().is_empty());

        let def = tool.definition();
        assert_eq!(def.name, "web_search");
        let params = tool.parameters();
        assert_eq!(params["type"], "object");
        assert!(params["properties"]["query"].is_object());
        assert!(params["properties"]["limit"].is_object());
        assert!(params["properties"]["engine"].is_object());
    }

    #[tokio::test]
    async fn test_web_search_empty_query_error() {
        let tool = WebSearchTool::new();
        let ctx = ToolContext::default();

        let res = tool.execute(json!({"query": ""}), &ctx).await;
        assert!(res.is_err());
        assert!(res.unwrap_err().to_string().contains("cannot be empty"));

        let missing_res = tool.execute(json!({}), &ctx).await;
        assert!(missing_res.is_err());
        assert!(missing_res
            .unwrap_err()
            .to_string()
            .contains("Missing required parameter"));
    }

    #[test]
    fn test_format_results_text_and_json() {
        let results = vec![SearchResult {
            title: "Tokio Async".to_string(),
            url: "https://tokio.rs".to_string(),
            snippet: "Async runtime for Rust".to_string(),
        }];

        let text = WebSearchTool::format_results("tokio", &results);
        assert!(text.contains("Search results for: \"tokio\""));
        assert!(text.contains("Tokio Async"));
        assert!(text.contains("https://tokio.rs"));

        let empty_text = WebSearchTool::format_results("nonexistent123", &[]);
        assert!(empty_text.contains("No search results found"));
    }
}
