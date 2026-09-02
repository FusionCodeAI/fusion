//! HTTP Fetch Tool for fetching web pages, markdown docs, and JSON APIs.
//!
//! Provides direct HTTP/HTTPS fetching with automatic content-type detection,
//! HTML-to-text/markdown conversion, JSON pretty-printing, and configurable
//! response truncation. Pure-Rust TLS with zero external C/C++ dependencies.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::time::Duration;

use crate::tools::types::{Tool, ToolContext};

/// User-Agent header mimicking a standard browser to avoid anti-bot blocks on public endpoints.
pub const DEFAULT_FETCH_USER_AGENT: &str =
    "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/124.0.0.0 Safari/537.36 (Fusion/0.3)";

/// Default maximum character length to prevent context explosion.
pub const DEFAULT_MAX_CONTENT_LENGTH: usize = 50_000;

/// Default timeout in seconds for HTTP requests.
pub const DEFAULT_FETCH_TIMEOUT_SECS: u64 = 15;

/// Maximum allowed per-request timeout, in seconds.
pub const MAX_FETCH_TIMEOUT_SECS: u64 = 300;

/// Maximum allowed URL length, in characters.
pub const MAX_URL_LENGTH: usize = 2048;

/// Hard cap on response body size (10 MiB) to prevent memory exhaustion.
pub const MAX_RESPONSE_BYTES: usize = 10 * 1024 * 1024;

/// Output formatting mode for fetched content.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum FetchFormat {
    /// Automatically convert HTML to clean markdown/text and pretty-print JSON.
    Auto,
    /// Convert HTML to clean markdown/text.
    Markdown,
    /// Convert HTML to plain text without markdown markers.
    Text,
    /// Parse and format as pretty JSON.
    Json,
    /// Return raw response body without transformation.
    Raw,
    /// Alias for Raw.
    Html,
}

impl Default for FetchFormat {
    fn default() -> Self {
        Self::Auto
    }
}

impl std::str::FromStr for FetchFormat {
    type Err = std::convert::Infallible;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(match s.trim().to_ascii_lowercase().as_str() {
            "text" | "plain" => Self::Text,
            "markdown" | "md" => Self::Markdown,
            "json" => Self::Json,
            "raw" => Self::Raw,
            "html" => Self::Html,
            _ => Self::Auto,
        })
    }
}

/// Options for configuring an HTTP fetch request.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FetchOptions {
    pub url: String,
    #[serde(default)]
    pub format: FetchFormat,
    #[serde(default = "default_method")]
    pub method: String,
    #[serde(default)]
    pub headers: HashMap<String, String>,
    #[serde(default)]
    pub body: Option<String>,
    #[serde(default)]
    pub max_length: Option<usize>,
    #[serde(default)]
    pub timeout_secs: Option<u64>,
    #[serde(default)]
    pub user_agent: Option<String>,
    #[serde(default)]
    pub raw: bool,
    #[serde(default)]
    pub include_metadata: bool,
}

fn default_method() -> String {
    "GET".to_string()
}

impl FetchOptions {
    pub fn new(url: impl Into<String>) -> Self {
        Self {
            url: url.into(),
            format: FetchFormat::Auto,
            method: "GET".to_string(),
            headers: HashMap::new(),
            body: None,
            max_length: Some(DEFAULT_MAX_CONTENT_LENGTH),
            timeout_secs: Some(DEFAULT_FETCH_TIMEOUT_SECS),
            user_agent: None,
            raw: false,
            include_metadata: false,
        }
    }
}

/// Structured response from an HTTP fetch operation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FetchResult {
    pub url: String,
    pub status: u16,
    pub status_text: String,
    pub content_type: Option<String>,
    pub content_length: Option<usize>,
    pub content: String,
    pub is_truncated: bool,
    pub total_chars: usize,
}

impl FetchResult {
    /// Formats the fetch result with optional metadata header.
    pub fn formatted_output(&self, include_metadata: bool) -> String {
        if include_metadata {
            let mut meta = format!(
                "HTTP Status: {} {}\nURL: {}\n",
                self.status, self.status_text, self.url
            );
            if let Some(ct) = &self.content_type {
                meta.push_str(&format!("Content-Type: {}\n", ct));
            }
            if let Some(cl) = self.content_length {
                meta.push_str(&format!("Content-Length: {} bytes\n", cl));
            }
            if self.is_truncated {
                meta.push_str(&format!(
                    "Truncated: Showing {} of {} characters\n",
                    self.content.len(),
                    self.total_chars
                ));
            }
            meta.push_str("---\n\n");
            meta.push_str(&self.content);
            meta
        } else {
            self.content.clone()
        }
    }
}

/// Cross-platform HTTP fetch tool using pure-Rust TLS (`rustls`).
#[derive(Clone)]
pub struct HttpFetchTool {
    client: reqwest::Client,
    default_max_length: usize,
    default_timeout: Duration,
}

impl Default for HttpFetchTool {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Debug for HttpFetchTool {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HttpFetchTool")
            .field("default_max_length", &self.default_max_length)
            .field("default_timeout", &self.default_timeout)
            .finish()
    }
}

impl HttpFetchTool {
    /// Create a new HTTP fetch tool with pure-Rust TLS and standard timeouts.
    pub fn new() -> Self {
        let client = reqwest::Client::builder()
            .use_rustls_tls()
            .connect_timeout(Duration::from_secs(10))
            .timeout(Duration::from_secs(DEFAULT_FETCH_TIMEOUT_SECS))
            .redirect(reqwest::redirect::Policy::limited(10))
            .build()
            .unwrap_or_else(|_| reqwest::Client::new());

        Self {
            client,
            default_max_length: DEFAULT_MAX_CONTENT_LENGTH,
            default_timeout: Duration::from_secs(DEFAULT_FETCH_TIMEOUT_SECS),
        }
    }

    /// Create with a custom HTTP client.
    pub fn with_client(client: reqwest::Client) -> Self {
        Self {
            client,
            default_max_length: DEFAULT_MAX_CONTENT_LENGTH,
            default_timeout: Duration::from_secs(DEFAULT_FETCH_TIMEOUT_SECS),
        }
    }

    /// Set the default maximum content length in characters.
    pub fn with_max_length(mut self, max_length: usize) -> Self {
        self.default_max_length = max_length;
        self
    }

    /// Set the default request timeout.
    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.default_timeout = timeout;
        self
    }

    /// Perform the fetch with structured options.
    pub async fn fetch(&self, opts: &FetchOptions) -> anyhow::Result<FetchResult> {
        let url_str = opts.url.trim();
        let parsed_url = Self::validate_url(url_str)?;

        let method = match opts.method.trim().to_uppercase().as_str() {
            "GET" => reqwest::Method::GET,
            "POST" => reqwest::Method::POST,
            "PUT" => reqwest::Method::PUT,
            "DELETE" => reqwest::Method::DELETE,
            "PATCH" => reqwest::Method::PATCH,
            "HEAD" => reqwest::Method::HEAD,
            other => anyhow::bail!("Unsupported HTTP method: {}", other),
        };

        let user_agent = opts
            .user_agent
            .as_deref()
            .unwrap_or(DEFAULT_FETCH_USER_AGENT);

        // Clamp user-supplied timeout to a sane maximum so a bad argument
        // cannot pin the tool for minutes.
        let timeout = opts
            .timeout_secs
            .map(|s| Duration::from_secs(s.min(MAX_FETCH_TIMEOUT_SECS)))
            .unwrap_or(self.default_timeout);

        let mut req = self
            .client
            .request(method, parsed_url.clone())
            .header(reqwest::header::USER_AGENT, user_agent)
            .header(
                reqwest::header::ACCEPT,
                "text/html,application/xhtml+xml,application/xml;q=0.9,application/json,text/markdown,text/plain;q=0.8,*/*;q=0.7",
            )
            .header(reqwest::header::ACCEPT_LANGUAGE, "en-US,en;q=0.9")
            .timeout(timeout);

        // Apply custom headers
        for (k, v) in &opts.headers {
            if let (Ok(header_name), Ok(header_val)) = (
                reqwest::header::HeaderName::from_bytes(k.as_bytes()),
                reqwest::header::HeaderValue::from_str(v),
            ) {
                req = req.header(header_name, header_val);
            }
        }

        // Apply body if present
        if let Some(body_content) = &opts.body {
            req = req.body(body_content.clone());
        }

        let response = req.send().await.map_err(|e| {
            if e.is_timeout() {
                anyhow::anyhow!(
                    "Request timed out after {:?} fetching '{}'",
                    timeout,
                    url_str
                )
            } else if e.is_connect() {
                anyhow::anyhow!("Failed to connect to '{}': {}", url_str, e)
            } else {
                anyhow::anyhow!("HTTP request failed for '{}': {}", url_str, e)
            }
        })?;

        let status = response.status();
        let status_code = status.as_u16();
        let status_text = status.canonical_reason().unwrap_or("").to_string();

        let content_type = response
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .map(|s| s.to_string());

        let raw_bytes = response.bytes().await.map_err(|e| {
            anyhow::anyhow!("Failed to read response body from '{}': {}", url_str, e)
        })?;

        // Hard cap: refuse absurdly large bodies instead of exhausting memory.
        if raw_bytes.len() > MAX_RESPONSE_BYTES {
            anyhow::bail!(
                "Response from '{}' is {} bytes, exceeding the {} byte limit.",
                url_str,
                raw_bytes.len(),
                MAX_RESPONSE_BYTES
            );
        }

        let content_length = Some(raw_bytes.len());
        let raw_text = String::from_utf8_lossy(&raw_bytes).to_string();

        // Check for error status
        if !status.is_success() && !status.is_redirection() {
            let error_preview = if raw_text.chars().count() > 500 {
                format!("{}...", raw_text.chars().take(500).collect::<String>())
            } else {
                raw_text.clone()
            };
            anyhow::bail!(
                "HTTP {} {}: {}\nBody:\n{}",
                status_code,
                status_text,
                url_str,
                error_preview.trim()
            );
        }

        // Transform content based on format & content-type
        let format = if opts.raw {
            FetchFormat::Raw
        } else {
            opts.format
        };

        let processed = process_content(&raw_text, content_type.as_deref(), format);

        let max_len = opts.max_length.unwrap_or(self.default_max_length);
        let (final_content, is_truncated, total_chars) = truncate_content(&processed, max_len);

        Ok(FetchResult {
            url: url_str.to_string(),
            status: status_code,
            status_text,
            content_type,
            content_length,
            content: final_content,
            is_truncated,
            total_chars,
        })
    }

    /// Validates a URL for scheme, host presence, and SSRF safety.
    ///
    /// Returns the parsed URL, or an error describing exactly what failed.
    /// Blocks credentials-in-URL, non-HTTP(S) schemes, missing hosts, and
    /// loopback/private-network targets by IP literal.
    pub fn validate_url(url_str: &str) -> anyhow::Result<reqwest::Url> {
        let trimmed = url_str.trim();
        if trimmed.is_empty() {
            anyhow::bail!("URL cannot be empty");
        }
        if trimmed.len() > MAX_URL_LENGTH {
            anyhow::bail!(
                "URL is {} characters, exceeding the {} character limit.",
                trimmed.len(),
                MAX_URL_LENGTH
            );
        }

        let parsed = reqwest::Url::parse(trimmed)
            .map_err(|e| anyhow::anyhow!("Invalid URL '{}': {}", trimmed, e))?;

        let scheme = parsed.scheme().to_ascii_lowercase();
        if scheme != "http" && scheme != "https" {
            anyhow::bail!(
                "Unsupported URL scheme '{}'. Only 'http://' and 'https://' are supported.",
                scheme
            );
        }

        let host = parsed
            .host_str()
            .ok_or_else(|| anyhow::anyhow!("URL '{}' has no host.", trimmed))?;
        if host.is_empty() {
            anyhow::bail!("URL '{}' has an empty host.", trimmed);
        }

        // Credentials embedded in the URL leak secrets into logs.
        if !parsed.username().is_empty() || parsed.password().is_some() {
            anyhow::bail!(
                "URL '{}' embeds credentials; pass them via headers instead.",
                trimmed
            );
        }

        // SSRF guard for IP-literal hosts: block loopback and private ranges.
        if let Some(host) = parsed.host_str() {
            if let Ok(ip) = host.parse::<std::net::Ipv4Addr>() {
                if ip.is_loopback()
                    || ip.is_private()
                    || ip.is_link_local()
                    || ip.is_unspecified()
                    || ip.is_broadcast()
                {
                    anyhow::bail!(
                        "Refusing to fetch private/loopback address '{}' for SSRF safety.",
                        ip
                    );
                }
            }
        }

        Ok(parsed)
    }
}

#[async_trait]
impl Tool for HttpFetchTool {
    fn name(&self) -> &str {
        "fetch"
    }
    fn description(&self) -> &str {
        "Fetch web pages, markdown documents, or JSON APIs directly with automatic HTML-to-text conversion and clean formatting."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "url": {
                    "type": "string",
                    "description": "The HTTP or HTTPS URL to fetch (e.g. 'https://docs.rs/tokio/latest/tokio/', 'https://api.github.com/repos/rust-lang/rust')."
                },
                "format": {
                    "type": "string",
                    "enum": ["auto", "markdown", "text", "json", "raw", "html"],
                    "description": "Output format: 'auto' (default: strips HTML to clean markdown/text, formats JSON), 'markdown' (converts HTML to markdown), 'text' (plain text), 'json' (pretty JSON), or 'raw'/'html' (unmodified response body)."
                },
                "method": {
                    "type": "string",
                    "enum": ["GET", "POST", "PUT", "DELETE", "PATCH", "HEAD"],
                    "description": "HTTP method to use (default: 'GET')."
                },
                "headers": {
                    "type": "object",
                    "description": "Optional HTTP headers key-value map (e.g. {\"Accept\": \"application/json\", \"Authorization\": \"Bearer token\"})."
                },
                "body": {
                    "type": "string",
                    "description": "Optional request body string for POST, PUT, or PATCH requests."
                },
                "max_length": {
                    "type": "integer",
                    "description": "Maximum number of response characters to return (default: 50,000, 0 = unlimited). Appends truncation notice if exceeded."
                },
                "timeout": {
                    "type": "integer",
                    "description": "Request timeout in seconds (default: 15)."
                },
                "raw": {
                    "type": "boolean",
                    "description": "If true, returns raw unmodified response body without HTML stripping (shortcut for format='raw')."
                },
                "include_metadata": {
                    "type": "boolean",
                    "description": "If true, prepends HTTP status, content-type, and size metadata to the response."
                }
            },
            "required": ["url"]
        })
    }

    async fn execute(&self, args: Value, ctx: &ToolContext) -> anyhow::Result<String> {
        let url = args
            .get("url")
            .and_then(|v| v.as_str())
            .or_else(|| args.get("uri").and_then(|v| v.as_str()))
            .or_else(|| args.get("link").and_then(|v| v.as_str()))
            .ok_or_else(|| anyhow::anyhow!("Missing required parameter: url"))?
            .trim()
            .to_string();

        let format_str = args
            .get("format")
            .and_then(|v| v.as_str())
            .unwrap_or("auto");
        let format: FetchFormat = format_str.parse().unwrap_or(FetchFormat::Auto);

        let method = args
            .get("method")
            .and_then(|v| v.as_str())
            .unwrap_or("GET")
            .to_string();

        let raw = args.get("raw").and_then(|v| v.as_bool()).unwrap_or(false);

        let include_metadata = args
            .get("include_metadata")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        let max_length = args
            .get("max_length")
            .and_then(|v| v.as_u64())
            .or_else(|| args.get("max_bytes").and_then(|v| v.as_u64()))
            .or_else(|| args.get("limit").and_then(|v| v.as_u64()))
            .map(|n| n as usize);

        let timeout_secs = args
            .get("timeout")
            .and_then(|v| v.as_u64())
            .or_else(|| args.get("timeout_secs").and_then(|v| v.as_u64()));

        let user_agent = args
            .get("user_agent")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());

        let body = args
            .get("body")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());

        let mut headers = HashMap::new();
        if let Some(obj) = args.get("headers").and_then(|v| v.as_object()) {
            for (k, v) in obj {
                if let Some(v_str) = v.as_str() {
                    headers.insert(k.clone(), v_str.to_string());
                }
            }
        }

        // Allow Authorization header or API keys from environment if applicable
        if !headers.contains_key("Authorization") {
            if let Some(token) = ctx.env.get("HTTP_AUTH_TOKEN") {
                headers.insert("Authorization".to_string(), format!("Bearer {}", token));
            }
        }

        let opts = FetchOptions {
            url,
            format,
            method,
            headers,
            body,
            max_length,
            timeout_secs,
            user_agent,
            raw,
            include_metadata,
        };

        let result = self.fetch(&opts).await?;
        Ok(result.formatted_output(include_metadata))
    }
}

// ---------------------------------------------------------------------------
// Content Processing & Conversion
// ---------------------------------------------------------------------------

/// Processes content according to format and content-type.
pub fn process_content(raw: &str, content_type: Option<&str>, format: FetchFormat) -> String {
    match format {
        FetchFormat::Raw | FetchFormat::Html => raw.to_string(),
        FetchFormat::Json => {
            if let Ok(parsed) = serde_json::from_str::<Value>(raw) {
                serde_json::to_string_pretty(&parsed).unwrap_or_else(|_| raw.to_string())
            } else {
                raw.to_string()
            }
        }
        FetchFormat::Text => {
            if is_html(raw, content_type) {
                html_to_text(raw)
            } else if is_json(raw, content_type) {
                if let Ok(parsed) = serde_json::from_str::<Value>(raw) {
                    serde_json::to_string_pretty(&parsed).unwrap_or_else(|_| raw.to_string())
                } else {
                    raw.to_string()
                }
            } else {
                raw.to_string()
            }
        }
        FetchFormat::Markdown => {
            if is_html(raw, content_type) {
                html_to_markdown(raw)
            } else if is_json(raw, content_type) {
                if let Ok(parsed) = serde_json::from_str::<Value>(raw) {
                    format!(
                        "```json\n{}\n```",
                        serde_json::to_string_pretty(&parsed).unwrap_or_else(|_| raw.to_string())
                    )
                } else {
                    raw.to_string()
                }
            } else {
                raw.to_string()
            }
        }
        FetchFormat::Auto => {
            if is_html(raw, content_type) {
                html_to_markdown(raw)
            } else if is_json(raw, content_type) {
                if let Ok(parsed) = serde_json::from_str::<Value>(raw) {
                    serde_json::to_string_pretty(&parsed).unwrap_or_else(|_| raw.to_string())
                } else {
                    raw.to_string()
                }
            } else {
                raw.to_string()
            }
        }
    }
}

/// Determines if content is HTML from header or content sniffing.
pub fn is_html(content: &str, content_type: Option<&str>) -> bool {
    if let Some(ct) = content_type {
        let ct_lower = ct.to_ascii_lowercase();
        if ct_lower.contains("text/html")
            || ct_lower.contains("application/xhtml+xml")
            || ct_lower.contains("text/xml")
        {
            return true;
        }
    }

    let trimmed = content.trim();
    let lower = if trimmed.len() > 1024 {
        trimmed[..1024].to_ascii_lowercase()
    } else {
        trimmed.to_ascii_lowercase()
    };

    lower.starts_with("<!doctype html")
        || lower.starts_with("<html")
        || lower.contains("<body")
        || (lower.contains("<div") && lower.contains("</div>"))
        || (lower.contains("<p>") && lower.contains("</p>"))
}

/// Determines if content is JSON from header or content sniffing.
pub fn is_json(content: &str, content_type: Option<&str>) -> bool {
    if let Some(ct) = content_type {
        let ct_lower = ct.to_ascii_lowercase();
        if ct_lower.contains("application/json") || ct_lower.contains("+json") {
            return true;
        }
    }

    let trimmed = content.trim();
    (trimmed.starts_with('{') && trimmed.ends_with('}'))
        || (trimmed.starts_with('[') && trimmed.ends_with(']'))
}

/// Truncates content cleanly at line/word boundaries if it exceeds `max_length`.
pub fn truncate_content(content: &str, max_length: usize) -> (String, bool, usize) {
    let total_chars = content.chars().count();
    if max_length == 0 || total_chars <= max_length {
        return (content.to_string(), false, total_chars);
    }

    // Find clean truncate point
    let truncated_slice: String = content.chars().take(max_length).collect();
    let last_newline = truncated_slice.rfind('\n');
    let last_space = truncated_slice.rfind(' ');

    let cut_point = if let Some(nl) = last_newline {
        if nl > max_length.saturating_sub(500) {
            nl
        } else {
            last_space.unwrap_or(max_length)
        }
    } else {
        last_space.unwrap_or(max_length)
    };

    let final_text = if cut_point > 0 && cut_point < truncated_slice.len() {
        &truncated_slice[..cut_point]
    } else {
        &truncated_slice
    };

    let notice = format!(
        "\n\n... [Response truncated: showing {} of {} characters. Set 'max_length' to increase limit]",
        final_text.chars().count(),
        total_chars
    );

    let mut result = final_text.to_string();
    result.push_str(&notice);
    (result, true, total_chars)
}

// ---------------------------------------------------------------------------
// HTML to Markdown & Text Parser
// ---------------------------------------------------------------------------

/// Converts raw HTML string into clean, readable Markdown.
pub fn html_to_markdown(html: &str) -> String {
    let clean_html = strip_non_content_tags(html);
    let mut parser = HtmlParser::new(&clean_html, true);
    let raw_out = parser.parse();
    normalize_whitespace(&raw_out)
}

/// Converts raw HTML string into clean plain text without Markdown syntax.
pub fn html_to_text(html: &str) -> String {
    let clean_html = strip_non_content_tags(html);
    let mut parser = HtmlParser::new(&clean_html, false);
    let raw_out = parser.parse();
    normalize_whitespace(&raw_out)
}

/// Strips `<script>`, `<style>`, `<noscript>`, `<svg>`, `<canvas>`, `<head>`, and comments.
pub fn strip_non_content_tags(html: &str) -> String {
    let mut result = String::with_capacity(html.len());
    let mut chars = html.chars().peekable();

    while let Some(ch) = chars.next() {
        if ch == '<' {
            // Check for comment <!-- ... -->
            if let Some(&'!') = chars.peek() {
                let rest: String = chars.clone().take(3).collect();
                if rest.starts_with("--") {
                    // Consume '--'
                    chars.next();
                    chars.next();
                    // Consume until '-->'
                    while let Some(c) = chars.next() {
                        if c == '-' {
                            let mut lookahead = chars.clone();
                            if lookahead.next() == Some('-') && lookahead.next() == Some('>') {
                                chars.next(); // -
                                chars.next(); // >
                                break;
                            }
                        }
                    }
                    continue;
                }
            }

            // Read tag name
            let mut tag_name = String::new();
            let mut tag_full = String::from("<");
            let mut is_closing = false;

            if let Some(&'/') = chars.peek() {
                is_closing = true;
                tag_full.push(chars.next().unwrap());
            }

            while let Some(&c) = chars.peek() {
                if c.is_alphanumeric() || c == '-' || c == '_' || c == ':' {
                    tag_name.push(chars.next().unwrap());
                    tag_full.push(c);
                } else {
                    break;
                }
            }

            let tag_name_lower = tag_name.to_ascii_lowercase();

            // Tags whose entire inner content should be skipped
            let skip_tags = ["script", "style", "noscript", "svg", "canvas", "template"];

            if !is_closing && skip_tags.contains(&tag_name_lower.as_str()) {
                // Consume until closing tag </tag_name>
                let closing_needle = format!("</{}", tag_name_lower);
                let mut buffer = String::new();
                while let Some(c) = chars.next() {
                    buffer.push(c);
                    if buffer.to_ascii_lowercase().ends_with(&closing_needle) {
                        // Consume until '>'
                        for nc in chars.by_ref() {
                            if nc == '>' {
                                break;
                            }
                        }
                        break;
                    }
                }
                continue;
            }

            // Normal tag: copy as-is
            result.push_str(&tag_full);
        } else {
            result.push(ch);
        }
    }

    result
}

/// Internal stateful HTML-to-markdown/text parser.
struct HtmlParser<'a> {
    input: &'a str,
    markdown: bool,
    output: String,
    list_stack: Vec<ListState>,
    link_stack: Vec<Option<String>>,
    in_pre: bool,
    in_code: bool,
    table_state: Option<TableState>,
}

#[derive(Debug, Clone)]
enum ListState {
    Unordered,
    Ordered(usize),
}

#[derive(Debug, Clone, Default)]
struct TableState {
    in_header: bool,
    current_row: Vec<String>,
    header_row: Vec<String>,
    rows: Vec<Vec<String>>,
    current_cell: String,
}

impl<'a> HtmlParser<'a> {
    fn new(input: &'a str, markdown: bool) -> Self {
        Self {
            input,
            markdown,
            output: String::with_capacity(input.len()),
            list_stack: Vec::new(),
            link_stack: Vec::new(),
            in_pre: false,
            in_code: false,
            table_state: None,
        }
    }

    fn parse(&mut self) -> String {
        let mut chars = self.input.chars().peekable();

        while let Some(ch) = chars.next() {
            if ch == '<' {
                let mut tag_buf = String::new();
                for c in chars.by_ref() {
                    if c == '>' {
                        break;
                    }
                    tag_buf.push(c);
                }
                self.handle_tag(&tag_buf);
            } else if ch == '&' {
                // Decode HTML entity
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
                        self.push_text(&decoded);
                        continue;
                    }
                }

                self.push_text("&");
                self.push_text(&entity);
                if found_semicolon {
                    self.push_text(";");
                }
            } else {
                let mut s = String::new();
                s.push(ch);
                self.push_text(&s);
            }
        }

        // Flush any remaining table
        if let Some(table) = self.table_state.take() {
            self.render_table(table);
        }

        std::mem::take(&mut self.output)
    }

    fn push_text(&mut self, text: &str) {
        if let Some(table) = &mut self.table_state {
            table.current_cell.push_str(text);
        } else {
            self.output.push_str(text);
        }
    }

    fn ensure_newline(&mut self) {
        if self.table_state.is_some() {
            return;
        }
        if !self.output.is_empty() && !self.output.ends_with('\n') {
            self.output.push('\n');
        }
    }

    fn ensure_blank_line(&mut self) {
        if self.table_state.is_some() {
            return;
        }
        if !self.output.is_empty() {
            if !self.output.ends_with('\n') {
                self.output.push_str("\n\n");
            } else if !self.output.ends_with("\n\n") {
                self.output.push('\n');
            }
        }
    }

    fn handle_tag(&mut self, raw_tag: &str) {
        let trimmed = raw_tag.trim();
        if trimmed.is_empty() {
            return;
        }

        let is_closing = trimmed.starts_with('/');
        let tag_content = if is_closing {
            &trimmed[1..]
        } else {
            trimmed
        };

        let tag_name = tag_content
            .split_whitespace()
            .next()
            .unwrap_or("")
            .trim_end_matches('/')
            .to_ascii_lowercase();

        match (tag_name.as_str(), is_closing) {
            // Headings
            ("h1", false) => {
                self.ensure_blank_line();
                if self.markdown {
                    self.output.push_str("# ");
                }
            }
            ("h1", true) => self.ensure_blank_line(),

            ("h2", false) => {
                self.ensure_blank_line();
                if self.markdown {
                    self.output.push_str("## ");
                }
            }
            ("h2", true) => self.ensure_blank_line(),

            ("h3", false) => {
                self.ensure_blank_line();
                if self.markdown {
                    self.output.push_str("### ");
                }
            }
            ("h3", true) => self.ensure_blank_line(),

            ("h4", false) => {
                self.ensure_blank_line();
                if self.markdown {
                    self.output.push_str("#### ");
                }
            }
            ("h4", true) => self.ensure_blank_line(),

            ("h5", false) => {
                self.ensure_blank_line();
                if self.markdown {
                    self.output.push_str("##### ");
                }
            }
            ("h5", true) => self.ensure_blank_line(),

            ("h6", false) => {
                self.ensure_blank_line();
                if self.markdown {
                    self.output.push_str("###### ");
                }
            }
            ("h6", true) => self.ensure_blank_line(),

            // Paragraphs and blocks
            ("p", false) => self.ensure_blank_line(),
            ("p", true) => self.ensure_blank_line(),

            ("div" | "article" | "section" | "main" | "header" | "footer" | "aside" | "nav", false) => {
                self.ensure_newline();
            }
            ("div" | "article" | "section" | "main" | "header" | "footer" | "aside" | "nav", true) => {
                self.ensure_newline();
            }

            // Line break & HR
            ("br", _) => self.output.push('\n'),
            ("hr", _) => {
                self.ensure_blank_line();
                if self.markdown {
                    self.output.push_str("---\n\n");
                } else {
                    self.output.push_str("----------\n\n");
                }
            }

            // Blockquote
            ("blockquote", false) => {
                self.ensure_blank_line();
                if self.markdown {
                    self.output.push_str("> ");
                }
            }
            ("blockquote", true) => self.ensure_blank_line(),

            // Lists
            ("ul", false) => {
                self.ensure_newline();
                self.list_stack.push(ListState::Unordered);
            }
            ("ul", true) => {
                self.list_stack.pop();
                self.ensure_newline();
            }
            ("ol", false) => {
                self.ensure_newline();
                self.list_stack.push(ListState::Ordered(1));
            }
            ("ol", true) => {
                self.list_stack.pop();
                self.ensure_newline();
            }
            ("li", false) => {
                self.ensure_newline();
                let indent = "  ".repeat(self.list_stack.len().saturating_sub(1));
                if let Some(state) = self.list_stack.last_mut() {
                    match state {
                        ListState::Unordered => {
                            if self.markdown {
                                self.output.push_str(&format!("{}- ", indent));
                            } else {
                                self.output.push_str(&format!("{}• ", indent));
                            }
                        }
                        ListState::Ordered(count) => {
                            if self.markdown {
                                self.output.push_str(&format!("{}{}. ", indent, count));
                            } else {
                                self.output.push_str(&format!("{}{}. ", indent, count));
                            }
                            *count += 1;
                        }
                    }
                } else {
                    self.output.push_str("- ");
                }
            }
            ("li", true) => self.ensure_newline(),

            // Code blocks & inline code
            ("pre", false) => {
                self.ensure_blank_line();
                self.in_pre = true;
                if self.markdown {
                    self.output.push_str("```\n");
                }
            }
            ("pre", true) => {
                if self.markdown {
                    self.ensure_newline();
                    self.output.push_str("```\n\n");
                } else {
                    self.ensure_blank_line();
                }
                self.in_pre = false;
            }
            ("code", false) => {
                if !self.in_pre && self.markdown {
                    self.output.push('`');
                    self.in_code = true;
                }
            }
            ("code", true) => {
                if !self.in_pre && self.markdown && self.in_code {
                    self.output.push('`');
                    self.in_code = false;
                }
            }

            // Bold & Italic
            ("strong" | "b", false) => {
                if self.markdown && !self.in_pre {
                    self.output.push_str("**");
                }
            }
            ("strong" | "b", true) => {
                if self.markdown && !self.in_pre {
                    self.output.push_str("**");
                }
            }
            ("em" | "i", false) => {
                if self.markdown && !self.in_pre {
                    self.output.push('*');
                }
            }
            ("em" | "i", true) => {
                if self.markdown && !self.in_pre {
                    self.output.push('*');
                }
            }

            // Links
            ("a", false) => {
                let href = extract_attribute(tag_content, "href");
                let valid_href = href.filter(|u| {
                    !u.is_empty() && !u.starts_with('#') && !u.starts_with("javascript:")
                });
                if self.markdown && valid_href.is_some() {
                    self.output.push('[');
                }
                self.link_stack.push(valid_href);
            }
            ("a", true) => {
                if let Some(Some(href)) = self.link_stack.pop() {
                    if self.markdown {
                        if self.output.ends_with('[') {
                            self.output.pop();
                            self.output.push_str(&href);
                        } else {
                            self.output.push_str(&format!("]({})", href));
                        }
                    }
                }
            }

            // Images
            ("img", _) => {
                let alt = extract_attribute(tag_content, "alt").unwrap_or_default();
                let src = extract_attribute(tag_content, "src").unwrap_or_default();
                if self.markdown {
                    if !src.is_empty() {
                        self.output.push_str(&format!("![{}]({})", alt, src));
                    } else if !alt.is_empty() {
                        self.output.push_str(&format!("[Image: {}]", alt));
                    }
                } else if !alt.is_empty() {
                    self.output.push_str(&format!("[Image: {}]", alt));
                }
            }

            // Tables
            ("table", false) => {
                self.ensure_blank_line();
                self.table_state = Some(TableState::default());
            }
            ("table", true) => {
                if let Some(table) = self.table_state.take() {
                    self.render_table(table);
                }
            }
            ("thead", false) => {
                if let Some(table) = &mut self.table_state {
                    table.in_header = true;
                }
            }
            ("thead", true) => {
                if let Some(table) = &mut self.table_state {
                    table.in_header = false;
                }
            }
            ("tr", false) => {
                if let Some(table) = &mut self.table_state {
                    table.current_row.clear();
                }
            }
            ("tr", true) => {
                if let Some(table) = &mut self.table_state {
                    let row = std::mem::take(&mut table.current_row);
                    if !row.is_empty() {
                        if table.in_header || table.header_row.is_empty() {
                            table.header_row = row;
                        } else {
                            table.rows.push(row);
                        }
                    }
                }
            }
            ("th" | "td", false) => {
                if let Some(table) = &mut self.table_state {
                    table.current_cell.clear();
                }
            }
            ("th" | "td", true) => {
                if let Some(table) = &mut self.table_state {
                    let cell_content = std::mem::take(&mut table.current_cell);
                    table.current_row.push(cell_content.trim().to_string());
                }
            }

            _ => {}
        }
    }

    fn render_table(&mut self, table: TableState) {
        if self.markdown {
            if !table.header_row.is_empty() {
                self.output.push_str("| ");
                for h in &table.header_row {
                    self.output.push_str(h);
                    self.output.push_str(" | ");
                }
                self.output.push('\n');

                self.output.push_str("| ");
                for _ in &table.header_row {
                    self.output.push_str("--- | ");
                }
                self.output.push('\n');
            }

            for row in table.rows {
                self.output.push_str("| ");
                for cell in row {
                    self.output.push_str(&cell);
                    self.output.push_str(" | ");
                }
                self.output.push('\n');
            }
            self.ensure_blank_line();
        } else {
            for row in std::iter::once(table.header_row).chain(table.rows) {
                if !row.is_empty() {
                    self.output.push_str(&row.join("\t"));
                    self.output.push('\n');
                }
            }
            self.ensure_blank_line();
        }
    }
}

/// Extracts an attribute value from a tag string (e.g. `href="https://example.com"`).
pub fn extract_attribute(tag: &str, attr_name: &str) -> Option<String> {
    let tag_lower = tag.to_ascii_lowercase();
    let needle = format!("{}=", attr_name.to_ascii_lowercase());

    let idx = tag_lower.find(&needle)?;
    let after = &tag[idx + needle.len()..];
    let after_trimmed = after.trim_start();

    if let Some(quote) = after_trimmed.chars().next() {
        if quote == '"' || quote == '\'' {
            let val_content = &after_trimmed[1..];
            let end_idx = val_content.find(quote)?;
            return Some(val_content[..end_idx].to_string());
        }
    }

    // Unquoted attribute value
    let val_end = after_trimmed
        .find(|c: char| c.is_whitespace() || c == '>')
        .unwrap_or(after_trimmed.len());
    let val = &after_trimmed[..val_end];
    if !val.is_empty() {
        Some(val.to_string())
    } else {
        None
    }
}

/// Decodes standard HTML entities.
pub fn decode_single_entity(entity: &str) -> Option<String> {
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
        "ldquo" => Some("“".to_string()),
        "rdquo" => Some("”".to_string()),
        "lsquo" => Some("‘".to_string()),
        "rsquo" => Some("’".to_string()),
        s if s.starts_with("#x") || s.starts_with("#X") => {
            u32::from_str_radix(&s[2..], 16)
                .ok()
                .and_then(char::from_u32)
                .map(|c| c.to_string())
        }
        s if s.starts_with('#') => {
            s[1..]
                .parse::<u32>()
                .ok()
                .and_then(char::from_u32)
                .map(|c| c.to_string())
        }
        _ => None,
    }
}

/// Normalizes whitespace by collapsing multiple blank lines and cleaning up trailing spaces.
pub fn normalize_whitespace(input: &str) -> String {
    let mut lines: Vec<&str> = input.lines().map(|l| l.trim_end()).collect();

    // Collapse multiple empty lines to max 1 empty line (2 newlines)
    let mut cleaned_lines = Vec::with_capacity(lines.len());
    let mut prev_empty = false;

    for line in lines.drain(..) {
        let is_empty = line.trim().is_empty();
        if is_empty {
            if !prev_empty && !cleaned_lines.is_empty() {
                cleaned_lines.push("");
            }
            prev_empty = true;
        } else {
            cleaned_lines.push(line);
            prev_empty = false;
        }
    }

    // Trim trailing empty line
    while let Some(&"") = cleaned_lines.last() {
        cleaned_lines.pop();
    }

    cleaned_lines.join("\n")
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_fetch_tool_name_and_parameters() {
        let tool = HttpFetchTool::new();
        assert_eq!(tool.name(), "fetch");
        assert!(!tool.description().is_empty());

        let def = tool.definition();
        assert_eq!(def.name, "fetch");

        let params = tool.parameters();
        assert_eq!(params["type"], "object");
        assert!(params["properties"]["url"].is_object());
        assert!(params["properties"]["format"].is_object());
        assert!(params["properties"]["max_length"].is_object());
    }

    #[test]
    fn test_html_strip_scripts_and_styles() {
        let html = r#"
            <!DOCTYPE html>
            <html>
            <head>
                <title>Test Page</title>
                <style>body { background: #fff; }</style>
                <script>console.log("hello");</script>
            </head>
            <body>
                <h1>Hello World</h1>
                <p>This is a paragraph with <a href="https://example.com">a link</a>.</p>
                <script type="text/javascript">var x = 10;</script>
            </body>
            </html>
        "#;

        let md = html_to_markdown(html);
        assert!(md.contains("# Hello World"));
        assert!(md.contains("This is a paragraph"));
        assert!(md.contains("[a link](https://example.com)"));
        assert!(!md.contains("console.log"));
        assert!(!md.contains("background: #fff"));
        assert!(!md.contains("var x = 10"));
    }

    #[test]
    fn test_html_headings_and_lists() {
        let html = r#"
            <h2>Main Heading</h2>
            <ul>
                <li>First item</li>
                <li>Second item</li>
            </ul>
            <ol>
                <li>Step 1</li>
                <li>Step 2</li>
            </ol>
        "#;

        let md = html_to_markdown(html);
        assert!(md.contains("## Main Heading"));
        assert!(md.contains("- First item"));
        assert!(md.contains("- Second item"));
        assert!(md.contains("1. Step 1"));
        assert!(md.contains("2. Step 2"));
    }

    #[test]
    fn test_html_code_blocks() {
        let html = r#"
            <p>Here is some <code>inline_code()</code></p>
            <pre><code>fn main() {
    println!("Hello Fusion");
}</code></pre>
        "#;

        let md = html_to_markdown(html);
        assert!(md.contains("`inline_code()`"));
        assert!(md.contains("```"));
        assert!(md.contains("println!(\"Hello Fusion\");"));
    }

    #[test]
    fn test_html_tables() {
        let html = r#"
            <table>
                <thead>
                    <tr><th>Command</th><th>Description</th></tr>
                </thead>
                <tbody>
                    <tr><td>cargo build</td><td>Compiles project</td></tr>
                    <tr><td>cargo test</td><td>Runs test suite</td></tr>
                </tbody>
            </table>
        "#;

        let md = html_to_markdown(html);
        assert!(md.contains("| Command | Description |"));
        assert!(md.contains("| --- | --- |"));
        assert!(md.contains("| cargo build | Compiles project |"));
        assert!(md.contains("| cargo test | Runs test suite |"));
    }

    #[test]
    fn test_html_entities() {
        let html = "<p>&quot;Hello &amp; welcome &copy; 2026 &mdash; Fusion&quot; &lt;v2&gt; &#x27;fast&#x27;</p>";
        let md = html_to_markdown(html);
        assert!(md.contains("\"Hello & welcome © 2026 — Fusion\" <v2> 'fast'"));
    }

    #[test]
    fn test_json_pretty_printing() {
        let raw_json = r#"{"name":"fusion","version":"0.3.0","features":["wasm","tools"]}"#;
        let processed = process_content(raw_json, Some("application/json"), FetchFormat::Auto);
        assert!(processed.contains("{\n"));
        assert!(processed.contains("\"name\": \"fusion\""));
        assert!(processed.contains("\"version\": \"0.3.0\""));
    }

    #[test]
    fn test_content_truncation() {
        let long_text = "Word ".repeat(10_000); // 50,000 chars
        let (truncated, is_trunc, total) = truncate_content(&long_text, 100);
        assert!(is_trunc);
        assert!(total > 100);
        assert!(truncated.contains("Response truncated"));
        assert!(truncated.len() < 300);
    }

    #[test]
    fn test_extract_attribute() {
        let tag = r#"a href="https://example.com/api" target="_blank""#;
        assert_eq!(
            extract_attribute(tag, "href"),
            Some("https://example.com/api".to_string())
        );
        assert_eq!(
            extract_attribute(tag, "target"),
            Some("_blank".to_string())
        );
        assert_eq!(extract_attribute(tag, "missing"), None);
    }

    #[test]
    fn test_format_from_str() {
        assert_eq!("auto".parse::<FetchFormat>().unwrap(), FetchFormat::Auto);
        assert_eq!("markdown".parse::<FetchFormat>().unwrap(), FetchFormat::Markdown);
        assert_eq!("md".parse::<FetchFormat>().unwrap(), FetchFormat::Markdown);
        assert_eq!("text".parse::<FetchFormat>().unwrap(), FetchFormat::Text);
        assert_eq!("json".parse::<FetchFormat>().unwrap(), FetchFormat::Json);
        assert_eq!("raw".parse::<FetchFormat>().unwrap(), FetchFormat::Raw);
        assert_eq!("html".parse::<FetchFormat>().unwrap(), FetchFormat::Html);
    }

    #[tokio::test]
    async fn test_empty_url_error() {
        let tool = HttpFetchTool::new();
        let ctx = ToolContext::default();
        let res = tool.execute(json!({"url": ""}), &ctx).await;
        assert!(res.is_err());
    }

    #[tokio::test]
    async fn test_invalid_url_scheme_error() {
        let tool = HttpFetchTool::new();
        let ctx = ToolContext::default();
        let res = tool.execute(json!({"url": "ftp://ftp.example.com/file.txt"}), &ctx).await;
        assert!(res.is_err());
        assert!(res.unwrap_err().to_string().contains("Unsupported URL scheme"));
    }
}
