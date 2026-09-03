//! HTTP client agent utility & Turn-to-cURL generator.
//!
//! Provides:
//! 1. **Robust HTTP Client Agent**:
//!    - Full support for GET, POST, PUT, DELETE, PATCH, HEAD, OPTIONS requests.
//!    - Headers, query parameters with automatic percent-encoding, and body payloads (Text, JSON, Bytes, Form).
//!    - Streaming response bodies with strict size limits to prevent memory exhaustion / OOM attacks.
//!    - Authentication schemes: Bearer tokens, Basic Auth (RFC 4648 Base64), and custom API keys.
//!    - Configurable timeouts (connect timeout, total request timeout) and redirect policies.
//!    - URL validation (http/https schemes, non-empty hosts, port checks, SSRF private IP checks).
//!    - Pure Rust TLS with zero external C/C++ dependencies.
//!
//! 2. **Turn-to-cURL Generator**:
//!    - High-fidelity cURL command export for any conversational turn, subagent step, or session.
//!    - Multi-provider support: OpenAI, Anthropic, OpenRouter, Ollama, DeepSeek, Groq, xAI, etc.
//!    - Multi-shell export: Bash/Zsh, PowerShell, Windows CMD, Fish shell.
//!    - Multi-language scripts: Python `requests`, JavaScript `fetch()`, Raw HTTP/1.1 wire format.

use std::collections::HashMap;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use thiserror::Error;

use crate::agent::fork::extract_turns;
use crate::agent::session::Session;
use crate::config::Config;
use crate::provider::types::{Message, Role, ToolDefinition};

// ============================================================================
// HTTP Client Agent Constants & Utilities
// ============================================================================

/// Default maximum response body size in bytes (10 Megabytes) to prevent memory exhaustion.
pub const DEFAULT_MAX_RESPONSE_BYTES: usize = 10 * 1024 * 1024;

/// Default request timeout in seconds for agent HTTP operations.
pub const DEFAULT_HTTP_TIMEOUT_SECS: u64 = 30;

/// Default connection timeout in seconds.
pub const DEFAULT_CONNECT_TIMEOUT_SECS: u64 = 10;

/// Standard User-Agent header for Fusion agent HTTP operations.
pub const DEFAULT_AGENT_USER_AGENT: &str = "Fusion-Agent/0.3.0";

/// Encodes raw bytes into standard RFC 4648 base64 string.
pub fn base64_encode(input: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity((input.len() + 2) / 3 * 4);
    let chunks = input.chunks_exact(3);
    let remainder = chunks.remainder();

    for chunk in chunks {
        let b0 = chunk[0];
        let b1 = chunk[1];
        let b2 = chunk[2];

        out.push(TABLE[(b0 >> 2) as usize] as char);
        out.push(TABLE[(((b0 & 0x03) << 4) | (b1 >> 4)) as usize] as char);
        out.push(TABLE[(((b1 & 0x0f) << 2) | (b2 >> 6)) as usize] as char);
        out.push(TABLE[(b2 & 0x3f) as usize] as char);
    }

    match remainder.len() {
        1 => {
            let b0 = remainder[0];
            out.push(TABLE[(b0 >> 2) as usize] as char);
            out.push(TABLE[((b0 & 0x03) << 4) as usize] as char);
            out.push('=');
            out.push('=');
        }
        2 => {
            let b0 = remainder[0];
            let b1 = remainder[1];
            out.push(TABLE[(b0 >> 2) as usize] as char);
            out.push(TABLE[(((b0 & 0x03) << 4) | (b1 >> 4)) as usize] as char);
            out.push(TABLE[((b1 & 0x0f) << 2) as usize] as char);
            out.push('=');
        }
        _ => {}
    }

    out
}

/// Validates a URL string ensuring proper scheme (`http` or `https`) and non-empty host.
pub fn validate_url(url_str: &str) -> anyhow::Result<reqwest::Url> {
    let trimmed = url_str.trim();
    if trimmed.is_empty() {
        anyhow::bail!("URL cannot be empty");
    }

    let parsed = reqwest::Url::parse(trimmed)
        .map_err(|e| anyhow::anyhow!("Invalid URL '{}': {}", trimmed, e))?;

    match parsed.scheme() {
        "http" | "https" => {}
        other => {
            anyhow::bail!(
                "Unsupported URL scheme '{}' in '{}'. Only 'http' and 'https' are allowed.",
                other,
                trimmed
            );
        }
    }

    if parsed.host_str().is_none() || parsed.host_str() == Some("") {
        anyhow::bail!("URL '{}' is missing a valid host name", trimmed);
    }

    Ok(parsed)
}

/// Checks if a parsed URL targets a private / loopback / local IP or host.
/// Useful for SSRF defense when executing untrusted agent requests.
pub fn is_private_or_local_host(url: &reqwest::Url) -> bool {
    let host = match url.host_str() {
        Some(h) => h.to_lowercase(),
        None => return true,
    };

    if host == "localhost"
        || host.ends_with(".localhost")
        || host.ends_with(".local")
        || host.ends_with(".internal")
    {
        return true;
    }

    if let Ok(ip) = host.parse::<std::net::IpAddr>() {
        match ip {
            std::net::IpAddr::V4(ipv4) => {
                ipv4.is_loopback()
                    || ipv4.is_private()
                    || ipv4.is_link_local()
                    || ipv4.is_broadcast()
                    || ipv4.is_documentation()
                    || ipv4.is_unspecified()
            }
            std::net::IpAddr::V6(ipv6) => ipv6.is_loopback() || ipv6.is_unspecified(),
        }
    } else {
        false
    }
}

/// Appends query parameters to a base URL string, properly merging with any existing query strings.
pub fn append_query_params(base_url: &str, params: &[(String, String)]) -> anyhow::Result<String> {
    if params.is_empty() {
        return Ok(base_url.to_string());
    }

    let mut parsed = validate_url(base_url)?;
    {
        let mut query_pairs = parsed.query_pairs_mut();
        for (k, v) in params {
            query_pairs.append_pair(k, v);
        }
    }

    Ok(parsed.to_string())
}

// ============================================================================
// HTTP Client Types: Methods, Auth, Body, Request, Response
// ============================================================================

/// HTTP request method enum supporting standard verbs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum HttpMethod {
    Get,
    Post,
    Put,
    Delete,
    Patch,
    Head,
    Options,
}

impl HttpMethod {
    /// Returns the uppercase standard string representation of the method.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Get => "GET",
            Self::Post => "POST",
            Self::Put => "PUT",
            Self::Delete => "DELETE",
            Self::Patch => "PATCH",
            Self::Head => "HEAD",
            Self::Options => "OPTIONS",
        }
    }

    /// Converts to the equivalent `reqwest::Method`.
    pub fn to_reqwest(&self) -> reqwest::Method {
        match self {
            Self::Get => reqwest::Method::GET,
            Self::Post => reqwest::Method::POST,
            Self::Put => reqwest::Method::PUT,
            Self::Delete => reqwest::Method::DELETE,
            Self::Patch => reqwest::Method::PATCH,
            Self::Head => reqwest::Method::HEAD,
            Self::Options => reqwest::Method::OPTIONS,
        }
    }
}

impl std::fmt::Display for HttpMethod {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

impl std::str::FromStr for HttpMethod {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.trim().to_ascii_uppercase().as_str() {
            "GET" => Ok(Self::Get),
            "POST" => Ok(Self::Post),
            "PUT" => Ok(Self::Put),
            "DELETE" => Ok(Self::Delete),
            "PATCH" => Ok(Self::Patch),
            "HEAD" => Ok(Self::Head),
            "OPTIONS" => Ok(Self::Options),
            other => anyhow::bail!(
                "Unsupported HTTP method: '{}'. Supported: GET, POST, PUT, DELETE, PATCH, HEAD, OPTIONS",
                other
            ),
        }
    }
}

/// Authentication scheme for HTTP requests.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum HttpAuth {
    /// Bearer token (sets `Authorization: Bearer <token>`).
    Bearer(String),
    /// Basic authentication with username and optional password (sets `Authorization: Basic <base64(user:pass)>`).
    Basic {
        username: String,
        password: Option<String>,
    },
    /// Custom API key header (e.g. `X-API-Key: <key>` or `api-key: <key>`).
    ApiKey { header_name: String, key: String },
    /// Custom raw header pair.
    Custom {
        header_name: String,
        header_value: String,
    },
}

impl HttpAuth {
    /// Creates Bearer token auth.
    pub fn bearer(token: impl Into<String>) -> Self {
        Self::Bearer(token.into())
    }

    /// Creates Basic auth.
    pub fn basic(username: impl Into<String>, password: Option<impl Into<String>>) -> Self {
        Self::Basic {
            username: username.into(),
            password: password.map(|p| p.into()),
        }
    }

    /// Creates API key header auth.
    pub fn api_key(header_name: impl Into<String>, key: impl Into<String>) -> Self {
        Self::ApiKey {
            header_name: header_name.into(),
            key: key.into(),
        }
    }

    /// Computes the header `(name, value)` pair for this auth scheme.
    pub fn to_header_pair(&self) -> (String, String) {
        match self {
            Self::Bearer(token) => ("Authorization".to_string(), format!("Bearer {}", token)),
            Self::Basic { username, password } => {
                let credentials = match password {
                    Some(pass) => format!("{}:{}", username, pass),
                    None => format!("{}:", username),
                };
                let encoded = base64_encode(credentials.as_bytes());
                ("Authorization".to_string(), format!("Basic {}", encoded))
            }
            Self::ApiKey { header_name, key } => (header_name.clone(), key.clone()),
            Self::Custom {
                header_name,
                header_value,
            } => (header_name.clone(), header_value.clone()),
        }
    }
}

/// Payload body for HTTP requests.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum HttpBody {
    /// No body / empty body.
    Empty,
    /// Raw text / string body (e.g. text/plain, markdown, XML, HTML).
    Text(String),
    /// JSON value payload (serialized to JSON string, auto Content-Type application/json).
    Json(Value),
    /// Raw binary bytes payload.
    Bytes(Vec<u8>),
    /// URL-encoded form data (key-value pairs, auto Content-Type application/x-www-form-urlencoded).
    Form(Vec<(String, String)>),
}

impl HttpBody {
    /// Returns true if the body is empty.
    pub fn is_empty(&self) -> bool {
        match self {
            Self::Empty => true,
            Self::Text(s) => s.is_empty(),
            Self::Bytes(b) => b.is_empty(),
            Self::Form(f) => f.is_empty(),
            Self::Json(Value::Null) => true,
            Self::Json(_) => false,
        }
    }

    /// Returns estimated byte length if available.
    pub fn len(&self) -> usize {
        match self {
            Self::Empty => 0,
            Self::Text(s) => s.len(),
            Self::Bytes(b) => b.len(),
            Self::Form(f) => f.iter().map(|(k, v)| k.len() + v.len() + 2).sum(),
            Self::Json(v) => serde_json::to_string(v).map(|s| s.len()).unwrap_or(0),
        }
    }

    /// Returns default `Content-Type` header value for this body type.
    pub fn default_content_type(&self) -> Option<&'static str> {
        match self {
            Self::Empty => None,
            Self::Text(_) => Some("text/plain; charset=utf-8"),
            Self::Json(_) => Some("application/json"),
            Self::Bytes(_) => Some("application/octet-stream"),
            Self::Form(_) => Some("application/x-www-form-urlencoded"),
        }
    }
}

/// Comprehensive HTTP request specification.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HttpRequest {
    /// HTTP method (GET, POST, PUT, DELETE, PATCH, HEAD, OPTIONS).
    pub method: HttpMethod,
    /// Destination URL.
    pub url: String,
    /// HTTP headers as key-value pairs.
    pub headers: Vec<(String, String)>,
    /// Query parameters to append to the URL.
    pub query_params: Vec<(String, String)>,
    /// Optional authentication scheme.
    pub auth: Option<HttpAuth>,
    /// Request payload body.
    pub body: HttpBody,
    /// Request timeout.
    pub timeout: Option<Duration>,
    /// Connection timeout.
    pub connect_timeout: Option<Duration>,
    /// Maximum response body size limit in bytes (defaults to 10MB).
    pub max_response_bytes: usize,
    /// Custom user agent string.
    pub user_agent: Option<String>,
    /// Whether to automatically follow HTTP redirects (up to 10).
    pub follow_redirects: bool,
}

impl HttpRequest {
    /// Creates a new HTTP request with default settings.
    pub fn new(method: HttpMethod, url: impl Into<String>) -> Self {
        Self {
            method,
            url: url.into(),
            headers: Vec::new(),
            query_params: Vec::new(),
            auth: None,
            body: HttpBody::Empty,
            timeout: Some(Duration::from_secs(DEFAULT_HTTP_TIMEOUT_SECS)),
            connect_timeout: Some(Duration::from_secs(DEFAULT_CONNECT_TIMEOUT_SECS)),
            max_response_bytes: DEFAULT_MAX_RESPONSE_BYTES,
            user_agent: None,
            follow_redirects: true,
        }
    }

    /// Convenience constructor for GET requests.
    pub fn get(url: impl Into<String>) -> Self {
        Self::new(HttpMethod::Get, url)
    }

    /// Convenience constructor for POST requests.
    pub fn post(url: impl Into<String>) -> Self {
        Self::new(HttpMethod::Post, url)
    }

    /// Convenience constructor for PUT requests.
    pub fn put(url: impl Into<String>) -> Self {
        Self::new(HttpMethod::Put, url)
    }

    /// Convenience constructor for DELETE requests.
    pub fn delete(url: impl Into<String>) -> Self {
        Self::new(HttpMethod::Delete, url)
    }

    /// Convenience constructor for PATCH requests.
    pub fn patch(url: impl Into<String>) -> Self {
        Self::new(HttpMethod::Patch, url)
    }

    /// Convenience constructor for HEAD requests.
    pub fn head(url: impl Into<String>) -> Self {
        Self::new(HttpMethod::Head, url)
    }

    /// Resolves the final target URL including all query parameters.
    pub fn resolved_url(&self) -> anyhow::Result<String> {
        append_query_params(&self.url, &self.query_params)
    }

    /// Resolves all effective headers including Auth and Content-Type.
    pub fn resolved_headers(&self) -> Vec<(String, String)> {
        let mut map = Vec::new();

        // 1. Base headers
        for (k, v) in &self.headers {
            map.push((k.clone(), v.clone()));
        }

        // 2. Auth header
        if let Some(auth) = &self.auth {
            let (auth_key, auth_val) = auth.to_header_pair();
            if !map.iter().any(|(k, _)| k.eq_ignore_ascii_case(&auth_key)) {
                map.push((auth_key, auth_val));
            }
        }

        // 3. Content-Type header if body has one and not already set
        if let Some(ct) = self.body.default_content_type() {
            if !map
                .iter()
                .any(|(k, _)| k.eq_ignore_ascii_case("content-type"))
            {
                map.push(("Content-Type".to_string(), ct.to_string()));
            }
        }

        map
    }

    /// Converts this `HttpRequest` into a `CurlCommand` for debugging, script export, or reproducibility.
    pub fn to_curl_command(&self) -> anyhow::Result<CurlCommand> {
        let full_url = self.resolved_url()?;
        let headers = self.resolved_headers();
        let body_json = match &self.body {
            HttpBody::Json(v) => v.clone(),
            HttpBody::Text(t) => {
                if let Ok(v) = serde_json::from_str::<Value>(t) {
                    v
                } else {
                    json!({ "raw": t })
                }
            }
            HttpBody::Form(f) => {
                let obj: serde_json::Map<String, Value> = f
                    .iter()
                    .map(|(k, v)| (k.clone(), Value::String(v.clone())))
                    .collect();
                Value::Object(obj)
            }
            HttpBody::Bytes(b) => {
                json!({ "bytes_length": b.len() })
            }
            HttpBody::Empty => Value::Null,
        };

        Ok(CurlCommand {
            method: self.method.as_str().to_string(),
            url: full_url,
            headers,
            body: body_json,
            provider: "http_client".to_string(),
            model: String::new(),
            turn_index: None,
            options: CurlExportOptions::default(),
            metadata: HashMap::new(),
        })
    }
}

/// Buffered HTTP response.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HttpResponse {
    /// HTTP status code (e.g. 200, 404, 500).
    pub status: u16,
    /// HTTP status text (e.g. "OK", "Not Found").
    pub status_text: String,
    /// Response headers.
    pub headers: HashMap<String, String>,
    /// Raw response body bytes (enforced under max_response_bytes).
    #[serde(skip_serializing)]
    pub body: Vec<u8>,
    /// Content-Type header value if present.
    pub content_type: Option<String>,
    /// Content-Length header value if reported by server.
    pub content_length: Option<usize>,
    /// Time taken to receive the full response.
    pub duration: Duration,
    /// Final resolved URL after any redirects.
    pub url: String,
}

impl HttpResponse {
    /// Returns `true` if the status code is 2xx (200..=299).
    pub fn is_success(&self) -> bool {
        (200..=299).contains(&self.status)
    }

    /// Returns `true` if the status code is 4xx (400..=499).
    pub fn is_client_error(&self) -> bool {
        (400..=499).contains(&self.status)
    }

    /// Returns `true` if the status code is 5xx (500..=599).
    pub fn is_server_error(&self) -> bool {
        (500..=599).contains(&self.status)
    }

    /// Converts the response body to UTF-8 text.
    pub fn text(&self) -> anyhow::Result<String> {
        String::from_utf8(self.body.clone())
            .map_err(|e| anyhow::anyhow!("Failed to decode response body as UTF-8: {}", e))
    }

    /// Parses the response body as JSON.
    pub fn json<T: for<'de> Deserialize<'de>>(&self) -> anyhow::Result<T> {
        serde_json::from_slice(&self.body)
            .map_err(|e| anyhow::anyhow!("Failed to deserialize response JSON: {}", e))
    }

    /// Parses the response body as generic `serde_json::Value`.
    pub fn json_value(&self) -> anyhow::Result<Value> {
        self.json::<Value>()
    }

    /// Returns the value of a specific header if present (case-insensitive).
    pub fn header(&self, name: &str) -> Option<&str> {
        let name_lower = name.to_lowercase();
        self.headers
            .iter()
            .find(|(k, _)| k.to_lowercase() == name_lower)
            .map(|(_, v)| v.as_str())
    }
}

/// Asynchronous response body stream with strict size limit enforcement.
pub struct ResponseStream {
    pub status: u16,
    pub status_text: String,
    pub headers: HashMap<String, String>,
    pub content_type: Option<String>,
    pub content_length: Option<usize>,
    pub url: String,
    response: reqwest::Response,
    max_bytes: usize,
    bytes_read: usize,
}

impl ResponseStream {
    /// Returns total bytes read from the stream so far.
    pub fn bytes_read(&self) -> usize {
        self.bytes_read
    }

    /// Returns the maximum permitted response size in bytes.
    pub fn max_bytes(&self) -> usize {
        self.max_bytes
    }

    /// Reads the next chunk from the streaming response, tracking bytes read and enforcing the size limit.
    pub async fn next_chunk(&mut self) -> anyhow::Result<Option<Vec<u8>>> {
        match self.response.chunk().await {
            Ok(Some(bytes)) => {
                self.bytes_read += bytes.len();
                if self.bytes_read > self.max_bytes {
                    anyhow::bail!(
                        "Response body exceeded maximum allowed limit of {} bytes (streamed {} bytes so far)",
                        self.max_bytes,
                        self.bytes_read
                    );
                }
                Ok(Some(bytes.to_vec()))
            }
            Ok(None) => Ok(None),
            Err(e) => anyhow::bail!("Failed to read response body chunk: {}", e),
        }
    }

    /// Collects the remaining stream into a full byte buffer with size limit checks.
    pub async fn collect_body(mut self) -> anyhow::Result<Vec<u8>> {
        let initial_cap = self.content_length.unwrap_or(0).min(self.max_bytes);
        let mut buffer = Vec::with_capacity(initial_cap);
        while let Some(chunk) = self.next_chunk().await? {
            buffer.extend_from_slice(&chunk);
        }
        Ok(buffer)
    }

    /// Collects the remaining stream as a UTF-8 string with size limit checks.
    pub async fn collect_text(self) -> anyhow::Result<String> {
        let bytes = self.collect_body().await?;
        String::from_utf8(bytes).map_err(|e| {
            anyhow::anyhow!("Failed to decode streamed response as valid UTF-8: {}", e)
        })
    }

    /// Collects the remaining stream and parses as JSON with size limit checks.
    pub async fn collect_json<T: for<'de> Deserialize<'de>>(self) -> anyhow::Result<T> {
        let bytes = self.collect_body().await?;
        serde_json::from_slice(&bytes)
            .map_err(|e| anyhow::anyhow!("Failed to parse streamed response as JSON: {}", e))
    }
}

/// Fluent request builder for constructing and sending HTTP requests.
#[derive(Debug, Clone)]
pub struct CurlRequestBuilder {
    client: Option<reqwest::Client>,
    request: HttpRequest,
}

impl CurlRequestBuilder {
    /// Initiates a new request builder.
    pub fn new(method: HttpMethod, url: impl Into<String>) -> Self {
        Self {
            client: None,
            request: HttpRequest::new(method, url),
        }
    }

    /// Associates a specific `reqwest::Client` with this builder.
    pub fn with_client(mut self, client: reqwest::Client) -> Self {
        self.client = Some(client);
        self
    }

    /// Adds a single request header.
    pub fn header(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.request.headers.push((key.into(), value.into()));
        self
    }

    /// Adds multiple request headers.
    pub fn headers(
        mut self,
        headers: impl IntoIterator<Item = (impl Into<String>, impl Into<String>)>,
    ) -> Self {
        for (k, v) in headers {
            self.request.headers.push((k.into(), v.into()));
        }
        self
    }

    /// Adds a single query parameter.
    pub fn query(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.request.query_params.push((key.into(), value.into()));
        self
    }

    /// Adds multiple query parameters.
    pub fn query_params(
        mut self,
        params: impl IntoIterator<Item = (impl Into<String>, impl Into<String>)>,
    ) -> Self {
        for (k, v) in params {
            self.request.query_params.push((k.into(), v.into()));
        }
        self
    }

    /// Sets Bearer token authorization header.
    pub fn bearer_auth(mut self, token: impl Into<String>) -> Self {
        self.request.auth = Some(HttpAuth::bearer(token));
        self
    }

    /// Sets Basic authorization header.
    pub fn basic_auth(
        mut self,
        username: impl Into<String>,
        password: Option<impl Into<String>>,
    ) -> Self {
        self.request.auth = Some(HttpAuth::basic(username, password));
        self
    }

    /// Sets custom API key header.
    pub fn api_key(mut self, header_name: impl Into<String>, key: impl Into<String>) -> Self {
        self.request.auth = Some(HttpAuth::api_key(header_name, key));
        self
    }

    /// Sets an explicit `HttpAuth` scheme.
    pub fn auth(mut self, auth: HttpAuth) -> Self {
        self.request.auth = Some(auth);
        self
    }

    /// Sets the request payload body.
    pub fn body(mut self, body: HttpBody) -> Self {
        self.request.body = body;
        self
    }

    /// Sets a raw text body.
    pub fn text(mut self, text: impl Into<String>) -> Self {
        self.request.body = HttpBody::Text(text.into());
        self
    }

    /// Sets a JSON body payload.
    pub fn json<T: Serialize>(mut self, value: &T) -> anyhow::Result<Self> {
        let val = serde_json::to_value(value)
            .map_err(|e| anyhow::anyhow!("Failed to serialize JSON body: {}", e))?;
        self.request.body = HttpBody::Json(val);
        Ok(self)
    }

    /// Sets a binary bytes body.
    pub fn bytes(mut self, bytes: impl Into<Vec<u8>>) -> Self {
        self.request.body = HttpBody::Bytes(bytes.into());
        self
    }

    /// Sets a URL-encoded form data body.
    pub fn form(
        mut self,
        params: impl IntoIterator<Item = (impl Into<String>, impl Into<String>)>,
    ) -> Self {
        self.request.body = HttpBody::Form(
            params
                .into_iter()
                .map(|(k, v)| (k.into(), v.into()))
                .collect(),
        );
        self
    }

    /// Sets the total request timeout.
    pub fn timeout(mut self, timeout: Duration) -> Self {
        self.request.timeout = Some(timeout);
        self
    }

    /// Sets the total request timeout in seconds.
    pub fn timeout_secs(mut self, secs: u64) -> Self {
        self.request.timeout = Some(Duration::from_secs(secs));
        self
    }

    /// Sets the connection timeout.
    pub fn connect_timeout(mut self, timeout: Duration) -> Self {
        self.request.connect_timeout = Some(timeout);
        self
    }

    /// Sets the connection timeout in seconds.
    pub fn connect_timeout_secs(mut self, secs: u64) -> Self {
        self.request.connect_timeout = Some(Duration::from_secs(secs));
        self
    }

    /// Sets the maximum response body size in bytes.
    pub fn max_response_bytes(mut self, max_bytes: usize) -> Self {
        self.request.max_response_bytes = max_bytes;
        self
    }

    /// Sets custom User-Agent.
    pub fn user_agent(mut self, ua: impl Into<String>) -> Self {
        self.request.user_agent = Some(ua.into());
        self
    }

    /// Sets whether to follow redirects.
    pub fn follow_redirects(mut self, follow: bool) -> Self {
        self.request.follow_redirects = follow;
        self
    }

    /// Validates the request configuration.
    pub fn validate(&self) -> anyhow::Result<()> {
        validate_url(&self.request.url)?;
        if self.request.max_response_bytes == 0 {
            anyhow::bail!("max_response_bytes cannot be 0");
        }
        Ok(())
    }

    /// Builds the validated `HttpRequest`.
    pub fn build(self) -> anyhow::Result<HttpRequest> {
        self.validate()?;
        Ok(self.request)
    }

    /// Converts this request into a `CurlCommand` without executing network I/O.
    pub fn to_curl_command(&self) -> anyhow::Result<CurlCommand> {
        self.request.to_curl_command()
    }

    /// Executes this request asynchronously, returning the fully buffered `HttpResponse` with size limits enforced.
    pub async fn send(self) -> anyhow::Result<HttpResponse> {
        let client = self.client.clone().unwrap_or_else(|| {
            create_default_reqwest_client(
                self.request.timeout,
                self.request.connect_timeout,
                self.request.follow_redirects,
            )
        });
        let req = self.build()?;
        execute_http_request(&client, &req).await
    }

    /// Executes this request asynchronously, returning a streaming `ResponseStream` with size limits enforced.
    pub async fn send_stream(self) -> anyhow::Result<ResponseStream> {
        let client = self.client.clone().unwrap_or_else(|| {
            create_default_reqwest_client(
                self.request.timeout,
                self.request.connect_timeout,
                self.request.follow_redirects,
            )
        });
        let req = self.build()?;
        execute_http_stream(&client, &req).await
    }
}

/// Agent HTTP client providing high-level execution of GET, POST, PUT, DELETE requests
/// with streaming, size limit protection, authentication, and timeouts.
#[derive(Debug, Clone)]
pub struct CurlAgent {
    client: reqwest::Client,
    default_timeout: Duration,
    default_connect_timeout: Duration,
    default_max_response_bytes: usize,
    default_user_agent: Option<String>,
}

impl Default for CurlAgent {
    fn default() -> Self {
        Self::new()
    }
}

impl CurlAgent {
    /// Creates a new `CurlAgent` with default timeout (30s) and response limit (10MB).
    pub fn new() -> Self {
        let client = reqwest::Client::builder()
            .use_rustls_tls()
            .connect_timeout(Duration::from_secs(DEFAULT_CONNECT_TIMEOUT_SECS))
            .timeout(Duration::from_secs(DEFAULT_HTTP_TIMEOUT_SECS))
            .redirect(reqwest::redirect::Policy::limited(10))
            .build()
            .unwrap_or_else(|_| reqwest::Client::new());

        Self {
            client,
            default_timeout: Duration::from_secs(DEFAULT_HTTP_TIMEOUT_SECS),
            default_connect_timeout: Duration::from_secs(DEFAULT_CONNECT_TIMEOUT_SECS),
            default_max_response_bytes: DEFAULT_MAX_RESPONSE_BYTES,
            default_user_agent: None,
        }
    }

    /// Creates a `CurlAgent` wrapping a custom `reqwest::Client`.
    pub fn with_client(client: reqwest::Client) -> Self {
        Self {
            client,
            default_timeout: Duration::from_secs(DEFAULT_HTTP_TIMEOUT_SECS),
            default_connect_timeout: Duration::from_secs(DEFAULT_CONNECT_TIMEOUT_SECS),
            default_max_response_bytes: DEFAULT_MAX_RESPONSE_BYTES,
            default_user_agent: None,
        }
    }

    /// Sets default timeout.
    pub fn with_default_timeout(mut self, timeout: Duration) -> Self {
        self.default_timeout = timeout;
        self
    }

    /// Sets default max response bytes.
    pub fn with_max_response_bytes(mut self, max_bytes: usize) -> Self {
        self.default_max_response_bytes = max_bytes;
        self
    }

    /// Sets default User-Agent.
    pub fn with_user_agent(mut self, user_agent: impl Into<String>) -> Self {
        self.default_user_agent = Some(user_agent.into());
        self
    }

    /// Initiates a GET request builder.
    pub fn get(&self, url: impl Into<String>) -> CurlRequestBuilder {
        self.request(HttpMethod::Get, url)
    }

    /// Initiates a POST request builder.
    pub fn post(&self, url: impl Into<String>) -> CurlRequestBuilder {
        self.request(HttpMethod::Post, url)
    }

    /// Initiates a PUT request builder.
    pub fn put(&self, url: impl Into<String>) -> CurlRequestBuilder {
        self.request(HttpMethod::Put, url)
    }

    /// Initiates a DELETE request builder.
    pub fn delete(&self, url: impl Into<String>) -> CurlRequestBuilder {
        self.request(HttpMethod::Delete, url)
    }

    /// Initiates a PATCH request builder.
    pub fn patch(&self, url: impl Into<String>) -> CurlRequestBuilder {
        self.request(HttpMethod::Patch, url)
    }

    /// Initiates a HEAD request builder.
    pub fn head(&self, url: impl Into<String>) -> CurlRequestBuilder {
        self.request(HttpMethod::Head, url)
    }

    /// Initiates an arbitrary HTTP method request builder.
    pub fn request(&self, method: HttpMethod, url: impl Into<String>) -> CurlRequestBuilder {
        let mut builder = CurlRequestBuilder::new(method, url)
            .with_client(self.client.clone())
            .timeout(self.default_timeout)
            .connect_timeout(self.default_connect_timeout)
            .max_response_bytes(self.default_max_response_bytes);

        if let Some(ua) = &self.default_user_agent {
            builder = builder.user_agent(ua);
        }

        builder
    }

    /// Executes an `HttpRequest` directly.
    pub async fn execute(&self, request: &HttpRequest) -> anyhow::Result<HttpResponse> {
        execute_http_request(&self.client, request).await
    }

    /// Executes an `HttpRequest` returning a `ResponseStream`.
    pub async fn execute_stream(&self, request: &HttpRequest) -> anyhow::Result<ResponseStream> {
        execute_http_stream(&self.client, request).await
    }
}

/// Helper to construct a standard `reqwest::Client` with rustls TLS and standard settings.
pub fn create_default_reqwest_client(
    timeout: Option<Duration>,
    connect_timeout: Option<Duration>,
    follow_redirects: bool,
) -> reqwest::Client {
    let mut builder = reqwest::Client::builder().use_rustls_tls();

    if let Some(to) = timeout {
        builder = builder.timeout(to);
    }
    if let Some(cto) = connect_timeout {
        builder = builder.connect_timeout(cto);
    }
    if follow_redirects {
        builder = builder.redirect(reqwest::redirect::Policy::limited(10));
    } else {
        builder = builder.redirect(reqwest::redirect::Policy::none());
    }

    builder.build().unwrap_or_else(|_| reqwest::Client::new())
}

/// Executes an HTTP request against a `reqwest::Client` with size limit enforcement.
pub async fn execute_http_request(
    client: &reqwest::Client,
    request: &HttpRequest,
) -> anyhow::Result<HttpResponse> {
    let full_url = request.resolved_url()?;
    let method = request.method.to_reqwest();

    let mut req_builder = client.request(method, &full_url);

    // Apply User-Agent
    if let Some(ua) = &request.user_agent {
        req_builder = req_builder.header(reqwest::header::USER_AGENT, ua.as_str());
    } else {
        req_builder = req_builder.header(reqwest::header::USER_AGENT, DEFAULT_AGENT_USER_AGENT);
    }

    // Apply headers
    let resolved_headers = request.resolved_headers();
    for (k, v) in &resolved_headers {
        if let (Ok(header_name), Ok(header_value)) = (
            reqwest::header::HeaderName::from_bytes(k.as_bytes()),
            reqwest::header::HeaderValue::from_str(v),
        ) {
            req_builder = req_builder.header(header_name, header_value);
        }
    }

    // Apply body
    match &request.body {
        HttpBody::Empty => {}
        HttpBody::Text(t) => {
            req_builder = req_builder.body(t.clone());
        }
        HttpBody::Json(v) => {
            let json_str = serde_json::to_string(v)
                .map_err(|e| anyhow::anyhow!("Failed to serialize JSON body: {}", e))?;
            req_builder = req_builder.body(json_str);
        }
        HttpBody::Bytes(b) => {
            req_builder = req_builder.body(b.clone());
        }
        HttpBody::Form(f) => {
            req_builder = req_builder.form(f);
        }
    }

    // Apply timeouts if specified
    if let Some(to) = request.timeout {
        req_builder = req_builder.timeout(to);
    }

    let start_time = std::time::Instant::now();
    let mut response = req_builder
        .send()
        .await
        .map_err(|e| anyhow::anyhow!("HTTP request to '{}' failed: {}", full_url, e))?;

    let duration = start_time.elapsed();
    let status = response.status().as_u16();
    let status_text = response
        .status()
        .canonical_reason()
        .unwrap_or("")
        .to_string();
    let resp_url = response.url().to_string();

    let mut headers = HashMap::new();
    for (k, v) in response.headers() {
        if let Ok(val_str) = v.to_str() {
            headers.insert(k.as_str().to_string(), val_str.to_string());
        }
    }

    let content_type = headers.get("content-type").cloned();
    let content_length = response.content_length().map(|l| l as usize);

    // Fast-fail if server declares Content-Length greater than max_response_bytes
    if let Some(cl) = content_length {
        if cl > request.max_response_bytes {
            anyhow::bail!(
                "Response Content-Length ({} bytes) exceeds maximum permitted limit of {} bytes for '{}'",
                cl,
                request.max_response_bytes,
                full_url
            );
        }
    }

    // Stream response chunks while enforcing size limit
    let initial_cap = content_length.unwrap_or(0).min(request.max_response_bytes);
    let mut body_bytes = Vec::with_capacity(initial_cap);
    let mut total_read = 0usize;

    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|e| anyhow::anyhow!("Failed to read response body chunk: {}", e))?
    {
        total_read += chunk.len();
        if total_read > request.max_response_bytes {
            anyhow::bail!(
                "Response body exceeded maximum permitted limit of {} bytes (streamed {} bytes so far) for '{}'",
                request.max_response_bytes,
                total_read,
                full_url
            );
        }
        body_bytes.extend_from_slice(&chunk);
    }

    Ok(HttpResponse {
        status,
        status_text,
        headers,
        body: body_bytes,
        content_type,
        content_length,
        duration,
        url: resp_url,
    })
}

/// Executes an HTTP request and returns an asynchronous `ResponseStream`.
pub async fn execute_http_stream(
    client: &reqwest::Client,
    request: &HttpRequest,
) -> anyhow::Result<ResponseStream> {
    let full_url = request.resolved_url()?;
    let method = request.method.to_reqwest();

    let mut req_builder = client.request(method, &full_url);

    if let Some(ua) = &request.user_agent {
        req_builder = req_builder.header(reqwest::header::USER_AGENT, ua.as_str());
    } else {
        req_builder = req_builder.header(reqwest::header::USER_AGENT, DEFAULT_AGENT_USER_AGENT);
    }

    let resolved_headers = request.resolved_headers();
    for (k, v) in &resolved_headers {
        if let (Ok(header_name), Ok(header_value)) = (
            reqwest::header::HeaderName::from_bytes(k.as_bytes()),
            reqwest::header::HeaderValue::from_str(v),
        ) {
            req_builder = req_builder.header(header_name, header_value);
        }
    }

    match &request.body {
        HttpBody::Empty => {}
        HttpBody::Text(t) => {
            req_builder = req_builder.body(t.clone());
        }
        HttpBody::Json(v) => {
            let json_str = serde_json::to_string(v)
                .map_err(|e| anyhow::anyhow!("Failed to serialize JSON body: {}", e))?;
            req_builder = req_builder.body(json_str);
        }
        HttpBody::Bytes(b) => {
            req_builder = req_builder.body(b.clone());
        }
        HttpBody::Form(f) => {
            req_builder = req_builder.form(f);
        }
    }

    if let Some(to) = request.timeout {
        req_builder = req_builder.timeout(to);
    }

    let response = req_builder
        .send()
        .await
        .map_err(|e| anyhow::anyhow!("HTTP stream request to '{}' failed: {}", full_url, e))?;

    let status = response.status().as_u16();
    let status_text = response
        .status()
        .canonical_reason()
        .unwrap_or("")
        .to_string();
    let resp_url = response.url().to_string();

    let mut headers = HashMap::new();
    for (k, v) in response.headers() {
        if let Ok(val_str) = v.to_str() {
            headers.insert(k.as_str().to_string(), val_str.to_string());
        }
    }

    let content_type = headers.get("content-type").cloned();
    let content_length = response.content_length().map(|l| l as usize);

    if let Some(cl) = content_length {
        if cl > request.max_response_bytes {
            anyhow::bail!(
                "Response Content-Length ({} bytes) exceeds maximum permitted limit of {} bytes for '{}'",
                cl,
                request.max_response_bytes,
                full_url
            );
        }
    }

    let max_bytes = request.max_response_bytes;

    Ok(ResponseStream {
        status,
        status_text,
        headers,
        content_type,
        content_length,
        url: resp_url,
        response,
        max_bytes,
        bytes_read: 0,
    })
}

/// Convenience function to execute a GET request.
pub async fn http_get(url: impl Into<String>) -> anyhow::Result<HttpResponse> {
    CurlAgent::new().get(url).send().await
}

/// Convenience function to execute a POST request with JSON payload.
pub async fn http_post_json<T: Serialize>(
    url: impl Into<String>,
    body: &T,
) -> anyhow::Result<HttpResponse> {
    CurlAgent::new().post(url).json(body)?.send().await
}

/// Convenience function to execute a POST request with text payload.
pub async fn http_post_text(
    url: impl Into<String>,
    body: impl Into<String>,
) -> anyhow::Result<HttpResponse> {
    CurlAgent::new().post(url).text(body).send().await
}

/// Convenience function to execute a PUT request with JSON payload.
pub async fn http_put_json<T: Serialize>(
    url: impl Into<String>,
    body: &T,
) -> anyhow::Result<HttpResponse> {
    CurlAgent::new().put(url).json(body)?.send().await
}

/// Convenience function to execute a DELETE request.
pub async fn http_delete(url: impl Into<String>) -> anyhow::Result<HttpResponse> {
    CurlAgent::new().delete(url).send().await
}

// ============================================================================
// Turn-to-cURL Generator Section
// ============================================================================
// Error Types
// ============================================================================

/// Errors encountered during cURL command generation or export.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum CurlExportError {
    #[error("Turn index {0} is out of bounds (session contains {1} turns)")]
    TurnOutOfBounds(usize, usize),

    #[error("Session contains no messages to export")]
    EmptySession,

    #[error("Unsupported or unknown provider: {0}")]
    UnknownProvider(String),

    #[error("Failed to serialize request payload: {0}")]
    SerializationError(String),

    #[error("Invalid export configuration: {0}")]
    InvalidConfig(String),
}

// ============================================================================
// Configuration & Options
// ============================================================================

/// Target shell dialect for formatting the cURL command.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum CurlShell {
    /// POSIX Bash / Zsh / Dash (default, single-quote payload with `\'` escaping).
    #[default]
    Bash,
    /// PowerShell (`Invoke-RestMethod` or `curl.exe` with `@' ... '@` here-strings).
    PowerShell,
    /// Windows Command Prompt (`cmd.exe` with caret line-breaks and escaped quotes).
    Cmd,
    /// Fish shell.
    Fish,
}

/// Output layout style for the generated cURL command.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum CurlFormatting {
    /// Multi-line with readable line continuations (`\`) and indented payload.
    #[default]
    Multiline,
    /// Single-line compact string ready for one-click copy and execution.
    SingleLine,
}

/// Controls how API keys and credentials appear in the exported cURL command.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum ApiKeyVisibility {
    /// Uses standard environment variable syntax (e.g. `$OPENAI_API_KEY` or `$ANTHROPIC_API_KEY`).
    #[default]
    EnvVar,
    /// Partially masks the secret, showing prefix and suffix (e.g. `sk-proj-***4a2b`).
    Masked,
    /// Completely obscures the secret as `[REDACTED]`.
    Redacted,
    /// Uses generic placeholder text (e.g. `YOUR_OPENAI_API_KEY` or `YOUR_API_KEY`).
    Placeholder,
    /// Includes the live API key verbatim (use with caution).
    Plain,
}

/// Scope of conversation messages included when generating a turn's cURL request.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum TurnRequestScope {
    /// Messages up to the initial user prompt of the turn (the prompt that initiated the LLM call).
    InitialPrompt,
    /// All messages up to the conclusion of the turn, including tool calls and tool responses.
    #[default]
    FullTurn,
    /// Every message in the entire active session buffer.
    AllMessages,
}

/// Comprehensive options for tailoring turn-to-cURL exports.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CurlExportOptions {
    /// Target shell dialect.
    pub shell: CurlShell,
    /// Formatting layout (multi-line vs single-line).
    pub formatting: CurlFormatting,
    /// How API keys are exposed or redacted.
    pub api_key_visibility: ApiKeyVisibility,
    /// Whether `"stream": true` is included in the request body (default: true).
    pub stream: bool,
    /// Whether tool definitions are included in the request body (default: true).
    pub include_tools: bool,
    /// Whether the JSON payload is formatted with indentation in multi-line mode.
    pub pretty_payload: bool,
    /// Additional custom HTTP headers to append (e.g. `[("X-Custom-Header", "Value")]`).
    pub custom_headers: Vec<(String, String)>,
    /// Additional cURL flags (e.g. `["-sS", "--compressed", "-N"]`).
    pub curl_flags: Vec<String>,
    /// Whether to prepend informative comments above the cURL command.
    pub include_comments: bool,
    /// Message range scope for turn extraction.
    pub scope: TurnRequestScope,
    /// Explicit provider override (e.g. "openai", "anthropic", "ollama").
    pub custom_provider: Option<String>,
    /// Explicit model override (e.g. "gpt-4o", "claude-3-7-sonnet-20250219").
    pub custom_model: Option<String>,
    /// Explicit base URL override.
    pub custom_base_url: Option<String>,
    /// Explicit API key override.
    pub custom_api_key: Option<String>,
    /// Explicit sampling temperature override.
    pub temperature: Option<f32>,
    /// Explicit max tokens completion limit override.
    pub max_tokens: Option<u32>,
}

impl Default for CurlExportOptions {
    fn default() -> Self {
        Self {
            shell: CurlShell::Bash,
            formatting: CurlFormatting::Multiline,
            api_key_visibility: ApiKeyVisibility::EnvVar,
            stream: true,
            include_tools: true,
            pretty_payload: true,
            custom_headers: Vec::new(),
            curl_flags: vec!["-sS".to_string()],
            include_comments: true,
            scope: TurnRequestScope::FullTurn,
            custom_provider: None,
            custom_model: None,
            custom_base_url: None,
            custom_api_key: None,
            temperature: None,
            max_tokens: None,
        }
    }
}

impl CurlExportOptions {
    /// Creates a new `CurlExportOptions` with default settings.
    pub fn new() -> Self {
        Self::default()
    }

    /// Sets the target shell dialect.
    pub fn with_shell(mut self, shell: CurlShell) -> Self {
        self.shell = shell;
        self
    }

    /// Sets the output formatting layout.
    pub fn with_formatting(mut self, formatting: CurlFormatting) -> Self {
        self.formatting = formatting;
        self
    }

    /// Sets API key visibility mode.
    pub fn with_api_key_visibility(mut self, visibility: ApiKeyVisibility) -> Self {
        self.api_key_visibility = visibility;
        self
    }

    /// Configures whether streaming is enabled in the request payload.
    pub fn with_stream(mut self, stream: bool) -> Self {
        self.stream = stream;
        self
    }

    /// Configures whether tool definitions are included.
    pub fn with_tools(mut self, include_tools: bool) -> Self {
        self.include_tools = include_tools;
        self
    }

    /// Configures pretty-printed JSON payloads in multi-line output.
    pub fn with_pretty_payload(mut self, pretty: bool) -> Self {
        self.pretty_payload = pretty;
        self
    }

    /// Adds a custom HTTP header.
    pub fn with_header(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.custom_headers.push((key.into(), value.into()));
        self
    }

    /// Adds a cURL CLI flag.
    pub fn with_curl_flag(mut self, flag: impl Into<String>) -> Self {
        self.curl_flags.push(flag.into());
        self
    }

    /// Sets whether comment headers are included in the generated output.
    pub fn with_comments(mut self, include_comments: bool) -> Self {
        self.include_comments = include_comments;
        self
    }

    /// Sets the turn message scope.
    pub fn with_scope(mut self, scope: TurnRequestScope) -> Self {
        self.scope = scope;
        self
    }

    /// Overrides the provider identifier.
    pub fn with_provider(mut self, provider: impl Into<String>) -> Self {
        self.custom_provider = Some(provider.into());
        self
    }

    /// Overrides the model name.
    pub fn with_model(mut self, model: impl Into<String>) -> Self {
        self.custom_model = Some(model.into());
        self
    }

    /// Overrides the base URL.
    pub fn with_base_url(mut self, base_url: impl Into<String>) -> Self {
        self.custom_base_url = Some(base_url.into());
        self
    }

    /// Overrides the API key.
    pub fn with_api_key(mut self, api_key: impl Into<String>) -> Self {
        self.custom_api_key = Some(api_key.into());
        self
    }

    /// Overrides sampling temperature.
    pub fn with_temperature(mut self, temperature: f32) -> Self {
        self.temperature = Some(temperature);
        self
    }

    /// Overrides max completion tokens.
    pub fn with_max_tokens(mut self, max_tokens: u32) -> Self {
        self.max_tokens = Some(max_tokens);
        self
    }
}

// ============================================================================
// CurlCommand Structure & Renderers
// ============================================================================

/// Represents a fully resolved, reproducible HTTP request as a cURL command.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CurlCommand {
    /// HTTP method (e.g. "POST").
    pub method: String,
    /// Target endpoint URL (e.g. `https://api.openai.com/v1/chat/completions`).
    pub url: String,
    /// Ordered list of HTTP request headers as `(name, value)` pairs.
    pub headers: Vec<(String, String)>,
    /// JSON request payload.
    pub body: Value,
    /// Provider name associated with this request (e.g. "openai", "anthropic", "ollama").
    pub provider: String,
    /// Model name targeted by this request (e.g. "gpt-4o", "claude-3-7-sonnet-20250219").
    pub model: String,
    /// 1-based turn index if generated from a turn context.
    pub turn_index: Option<usize>,
    /// Options used during command generation.
    pub options: CurlExportOptions,
    /// Contextual metadata key-value pairs (e.g. timestamp, token count, tool count).
    pub metadata: HashMap<String, String>,
}

impl CurlCommand {
    /// Returns the value of a specific header if present (case-insensitive key lookup).
    pub fn header_value(&self, key: &str) -> Option<&str> {
        let key_lower = key.to_lowercase();
        self.headers
            .iter()
            .find(|(k, _)| k.to_lowercase() == key_lower)
            .map(|(_, v)| v.as_str())
    }

    /// Returns `true` if a header with the given name is present.
    pub fn has_header(&self, key: &str) -> bool {
        self.header_value(key).is_some()
    }

    /// Formats the JSON request payload as a pretty-printed string with 2-space indentation.
    pub fn body_json_pretty(&self) -> String {
        serde_json::to_string_pretty(&self.body).unwrap_or_else(|_| self.body.to_string())
    }

    /// Formats the JSON request payload as a compact single-line string.
    pub fn body_json_compact(&self) -> String {
        serde_json::to_string(&self.body).unwrap_or_else(|_| self.body.to_string())
    }

    /// Produces a short human-readable summary of the request.
    pub fn summary(&self) -> String {
        let turn_desc = match self.turn_index {
            Some(t) => format!("Turn {}", t),
            None => "Session Request".to_string(),
        };
        let messages_count = self
            .body
            .get("messages")
            .and_then(|v| v.as_array())
            .map(|a| a.len())
            .unwrap_or(0);
        let tools_count = self
            .body
            .get("tools")
            .and_then(|v| v.as_array())
            .map(|a| a.len())
            .unwrap_or(0);

        format!(
            "{} [{} | {}] -> {} ({} msgs, {} tools)",
            turn_desc, self.provider, self.model, self.url, messages_count, tools_count
        )
    }

    /// Renders the cURL command according to the active options (shell & formatting).
    pub fn render(&self) -> String {
        self.render_for_shell(self.options.shell)
    }

    /// Renders the cURL command for a specific shell dialect.
    pub fn render_for_shell(&self, shell: CurlShell) -> String {
        match (shell, self.options.formatting) {
            (CurlShell::Bash | CurlShell::Fish, CurlFormatting::Multiline) => self.to_bash(),
            (CurlShell::Bash | CurlShell::Fish, CurlFormatting::SingleLine) => {
                self.to_single_line()
            }
            (CurlShell::PowerShell, CurlFormatting::Multiline) => self.to_powershell(),
            (CurlShell::PowerShell, CurlFormatting::SingleLine) => self.to_powershell_single_line(),
            (CurlShell::Cmd, CurlFormatting::Multiline) => self.to_cmd(),
            (CurlShell::Cmd, CurlFormatting::SingleLine) => self.to_cmd_single_line(),
        }
    }

    /// Formats the cURL command as a readable, multi-line POSIX Bash script snippet.
    pub fn to_bash(&self) -> String {
        let mut out = String::new();

        if self.options.include_comments {
            out.push_str(&self.format_comment_header("#"));
        }

        out.push_str("curl");

        for flag in &self.options.curl_flags {
            out.push(' ');
            out.push_str(flag);
        }

        out.push_str(" -X ");
        out.push_str(&self.method);
        out.push_str(" \"");
        out.push_str(&self.url);
        out.push_str("\" \\\n");

        for (k, v) in &self.headers {
            out.push_str("  -H \"");
            out.push_str(k);
            out.push_str(": ");
            out.push_str(&escape_double_quotes_for_bash(v));
            out.push_str("\" \\\n");
        }

        let json_body = if self.options.pretty_payload {
            self.body_json_pretty()
        } else {
            self.body_json_compact()
        };

        let escaped_body = escape_bash_single_quote(&json_body);
        out.push_str("  -d '");
        out.push_str(&escaped_body);
        out.push('\'');

        out
    }

    /// Formats the cURL command as a compact, single-line string.
    pub fn to_single_line(&self) -> String {
        let mut out = String::from("curl");

        for flag in &self.options.curl_flags {
            out.push(' ');
            out.push_str(flag);
        }

        out.push_str(" -X ");
        out.push_str(&self.method);
        out.push_str(" \"");
        out.push_str(&self.url);
        out.push('"');

        for (k, v) in &self.headers {
            out.push_str(" -H \"");
            out.push_str(k);
            out.push_str(": ");
            out.push_str(&escape_double_quotes_for_bash(v));
            out.push('"');
        }

        let json_body = self.body_json_compact();
        let escaped_body = escape_bash_single_quote(&json_body);
        out.push_str(" -d '");
        out.push_str(&escaped_body);
        out.push('\'');

        out
    }

    /// Formats the request as an idiomatic PowerShell command using `Invoke-RestMethod` and here-strings.
    pub fn to_powershell(&self) -> String {
        let mut out = String::new();

        if self.options.include_comments {
            out.push_str(&self.format_comment_header("#"));
        }

        out.push_str("$headers = @{\n");
        for (k, v) in &self.headers {
            let escaped_v = v.replace('"', "`\"");
            out.push_str(&format!("    \"{}\" = \"{}\"\n", k, escaped_v));
        }
        out.push_str("}\n\n");

        let json_body = if self.options.pretty_payload {
            self.body_json_pretty()
        } else {
            self.body_json_compact()
        };

        out.push_str("$body = @'\n");
        out.push_str(&json_body);
        out.push_str("\n'@\n\n");

        out.push_str(&format!(
            "Invoke-RestMethod -Uri \"{}\" -Method {} -Headers $headers -Body $body\n",
            self.url,
            capitalize_method(&self.method)
        ));

        out
    }

    /// Formats the request as a single-line PowerShell `curl.exe` command.
    pub fn to_powershell_single_line(&self) -> String {
        let mut out = String::from("curl.exe -X ");
        out.push_str(&self.method);
        out.push_str(" \"");
        out.push_str(&self.url);
        out.push('"');

        for (k, v) in &self.headers {
            let escaped_v = v.replace('"', "\\\"");
            out.push_str(&format!(" -H \"{}: {}\"", k, escaped_v));
        }

        let compact_json = self.body_json_compact().replace('"', "\\\"");
        out.push_str(&format!(" -d \"{}\"", compact_json));
        out
    }

    /// Formats the cURL command for Windows Command Prompt (`cmd.exe`).
    pub fn to_cmd(&self) -> String {
        let mut out = String::new();

        if self.options.include_comments {
            out.push_str(&self.format_comment_header("::"));
        }

        out.push_str("curl.exe -X ");
        out.push_str(&self.method);
        out.push_str(" \"");
        out.push_str(&self.url);
        out.push_str("\" ^\n");

        for (k, v) in &self.headers {
            let escaped_v = v.replace('"', "\\\"");
            out.push_str(&format!("  -H \"{}: {}\" ^\n", k, escaped_v));
        }

        let compact_json = escape_cmd_double_quote(&self.body_json_compact());
        out.push_str(&format!("  -d \"{}\"", compact_json));

        out
    }

    /// Formats the cURL command as a single-line Windows CMD command.
    pub fn to_cmd_single_line(&self) -> String {
        let mut out = format!("curl.exe -X {} \"{}\"", self.method, self.url);

        for (k, v) in &self.headers {
            let escaped_v = v.replace('"', "\\\"");
            out.push_str(&format!(" -H \"{}: {}\"", k, escaped_v));
        }

        let compact_json = escape_cmd_double_quote(&self.body_json_compact());
        out.push_str(&format!(" -d \"{}\"", compact_json));
        out
    }

    /// Formats the request as an executable Python 3 script using `requests`.
    pub fn to_python_requests(&self) -> String {
        let mut out = String::new();
        out.push_str("#!/usr/bin/env python3\n");
        out.push_str("# Generated by Fusion Turn-to-cURL Exporter\n");
        out.push_str("import os\n");
        out.push_str("import json\n");
        out.push_str("import requests\n\n");

        out.push_str(&format!("url = \"{}\"\n\n", self.url));

        out.push_str("headers = {\n");
        for (k, v) in &self.headers {
            let escaped_v = v.replace('\\', "\\\\").replace('"', "\\\"");
            out.push_str(&format!("    \"{}\": \"{}\",\n", k, escaped_v));
        }
        out.push_str("}\n\n");

        let json_body = self.body_json_pretty();
        out.push_str("payload = ");
        out.push_str(&json_body);
        out.push_str("\n\n");

        out.push_str("response = requests.post(url, headers=headers, json=payload, stream=True)\n");
        out.push_str("print(f\"Status: {response.status_code}\")\n");
        out.push_str("for line in response.iter_lines():\n");
        out.push_str("    if line:\n");
        out.push_str("        print(line.decode('utf-8'))\n");

        out
    }

    /// Formats the request as a modern JavaScript/TypeScript `fetch()` snippet with streaming response handling.
    pub fn to_fetch_js(&self) -> String {
        let mut out = String::new();
        out.push_str("// Generated by Fusion Turn-to-cURL Exporter\n");
        out.push_str("async function reproduceRequest() {\n");
        out.push_str(&format!("  const url = \"{}\";\n\n", self.url));

        out.push_str("  const headers = {\n");
        for (k, v) in &self.headers {
            let escaped_v = v.replace('\\', "\\\\").replace('"', "\\\"");
            out.push_str(&format!("    \"{}\": \"{}\",\n", k, escaped_v));
        }
        out.push_str("  };\n\n");

        out.push_str("  const payload = ");
        out.push_str(&self.body_json_pretty());
        out.push_str(";\n\n");

        out.push_str("  const response = await fetch(url, {\n");
        out.push_str("    method: \"POST\",\n");
        out.push_str("    headers,\n");
        out.push_str("    body: JSON.stringify(payload),\n");
        out.push_str("  });\n\n");
        out.push_str("  console.log(`Status: ${response.status} ${response.statusText}`);\n");
        out.push_str("  const reader = response.body.getReader();\n");
        out.push_str("  const decoder = new TextDecoder();\n");
        out.push_str("  while (true) {\n");
        out.push_str("    const { done, value } = await reader.read();\n");
        out.push_str("    if (done) break;\n");
        out.push_str("    console.log(decoder.decode(value, { stream: true }));\n");
        out.push_str("  }\n");
        out.push_str("}\n\n");
        out.push_str("reproduceRequest().catch(console.error);\n");

        out
    }

    /// Formats the request in raw HTTP/1.1 wire transmission format.
    pub fn to_http_raw(&self) -> String {
        let (host, path) = parse_url_host_and_path(&self.url);
        let mut out = String::new();

        out.push_str(&format!("{} {} HTTP/1.1\r\n", self.method, path));
        out.push_str(&format!("Host: {}\r\n", host));

        for (k, v) in &self.headers {
            out.push_str(&format!("{}: {}\r\n", k, v));
        }

        let body_str = self.body_json_compact();
        out.push_str(&format!("Content-Length: {}\r\n", body_str.len()));
        out.push_str("\r\n");
        out.push_str(&body_str);

        out
    }

    /// Generates a standalone, executable Bash reproduction script with sanity checks.
    pub fn to_script(&self) -> String {
        let mut out = String::new();
        out.push_str("#!/usr/bin/env bash\n");
        out.push_str(
            "# =============================================================================\n",
        );
        out.push_str("# Fusion LLM Request Reproduction Script\n");
        out.push_str(&format!("# Provider: {}\n", self.provider));
        out.push_str(&format!("# Model:    {}\n", self.model));
        if let Some(t) = self.turn_index {
            out.push_str(&format!("# Turn:     {}\n", t));
        }
        out.push_str(
            "# =============================================================================\n",
        );
        out.push_str("set -euo pipefail\n\n");

        // Environment variable check for API key
        let env_var = provider_env_var_name(&self.provider);
        if self.provider != "ollama" {
            out.push_str(&format!(
                "if [[ -z \"${{{}:-}}\" ]]; then\n  echo \"Error: Environment variable {} is not set.\" >&2\n  exit 1\nfi\n\n",
                env_var, env_var
            ));
        }

        out.push_str("echo \"Sending request to ");
        out.push_str(&self.url);
        out.push_str("...\"\n\n");

        out.push_str(&self.to_bash());
        out.push_str("\n\necho \"\"\necho \"Request complete.\"\n");

        out
    }

    fn format_comment_header(&self, comment_prefix: &str) -> String {
        let mut out = String::new();
        out.push_str(&format!(
            "{} -----------------------------------------------------------------------------\n",
            comment_prefix
        ));
        out.push_str(&format!("{} Fusion Turn-to-cURL Export\n", comment_prefix));
        if let Some(t) = self.turn_index {
            out.push_str(&format!("{} Turn:     {}\n", comment_prefix, t));
        }
        out.push_str(&format!("{} Provider: {}\n", comment_prefix, self.provider));
        out.push_str(&format!("{} Model:    {}\n", comment_prefix, self.model));
        out.push_str(&format!("{} Endpoint: {}\n", comment_prefix, self.url));
        out.push_str(&format!(
            "{} -----------------------------------------------------------------------------\n",
            comment_prefix
        ));
        out
    }
}

// ============================================================================
// Complete Reproduction Bundle
// ============================================================================

/// Complete multi-format reproduction artifact bundle for a turn or session request.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CurlReproductionBundle {
    /// 1-based turn index.
    pub turn_index: usize,
    /// Active provider name.
    pub provider: String,
    /// Active model name.
    pub model: String,
    /// Core `CurlCommand` structure.
    pub command: CurlCommand,
    /// Multi-line POSIX Bash cURL snippet.
    pub curl_bash: String,
    /// Single-line cURL snippet.
    pub curl_single_line: String,
    /// PowerShell script snippet.
    pub curl_powershell: String,
    /// Windows CMD snippet.
    pub curl_cmd: String,
    /// Python script snippet.
    pub python_script: String,
    /// JavaScript `fetch()` snippet.
    pub fetch_js: String,
    /// Raw HTTP wire format snippet.
    pub raw_http: String,
    /// Formatted JSON payload string.
    pub payload_json: String,
    /// Count of messages included in the request.
    pub messages_count: usize,
    /// Count of tool definitions included in the request.
    pub tools_count: usize,
    /// Estimated token count for the request prompt.
    pub estimated_tokens: u64,
    /// Markdown reproduction instructions and execution notes.
    pub reproduction_guide: String,
}

// ============================================================================
// Turn-to-cURL Generation Functions
// ============================================================================

/// Generates a reproducible `CurlCommand` representing the exact HTTP request sent for a specific 1-based turn.
pub fn generate_turn_curl(
    session: &Session,
    turn_index: usize,
    config: &Config,
    options: &CurlExportOptions,
) -> Result<CurlCommand, CurlExportError> {
    if turn_index == 0 {
        return Err(CurlExportError::TurnOutOfBounds(0, 0));
    }

    if session.messages.is_empty() {
        return Err(CurlExportError::EmptySession);
    }

    let turns = extract_turns(&session.messages);
    let total_turns = turns.len();

    if turn_index > total_turns {
        return Err(CurlExportError::TurnOutOfBounds(turn_index, total_turns));
    }

    let turn = &turns[turn_index - 1];

    // Determine message slice based on selected scope
    let slice_end = match options.scope {
        TurnRequestScope::InitialPrompt => {
            // Include messages up to and including the turn's user message
            turn.start_message_index + 1
        }
        TurnRequestScope::FullTurn => {
            // Include messages up to the end of this turn
            turn.end_message_index
        }
        TurnRequestScope::AllMessages => session.messages.len(),
    };

    let target_slice = &session.messages[..slice_end.min(session.messages.len())];

    // Build complete messages list, prepending system prompt if not already present
    let messages = construct_turn_messages(session, target_slice);

    // Resolve provider, model, url, api_key, tools
    let provider = options
        .custom_provider
        .clone()
        .unwrap_or_else(|| config.default_provider.clone());

    let model = options
        .custom_model
        .clone()
        .unwrap_or_else(|| session.active_model.clone());

    let (default_key, default_url) = config.get_key_and_url(&provider);

    let base_url = options.custom_base_url.clone().unwrap_or(default_url);

    let api_key = options.custom_api_key.as_deref().or(default_key.as_deref());

    // Extract tool definitions
    let tools = if options.include_tools {
        get_available_tool_definitions()
    } else {
        Vec::new()
    };

    let mut command = generate_curl_from_messages(
        &messages, &tools, &provider, &model, &base_url, api_key, options,
    )?;

    command.turn_index = Some(turn_index);
    command
        .metadata
        .insert("turn_index".to_string(), turn_index.to_string());
    command
        .metadata
        .insert("total_turns".to_string(), total_turns.to_string());
    command
        .metadata
        .insert("scope".to_string(), format!("{:?}", options.scope));

    Ok(command)
}

/// Generates a reproducible `CurlCommand` for the most recent conversation turn in the session.
pub fn generate_latest_turn_curl(
    session: &Session,
    config: &Config,
    options: &CurlExportOptions,
) -> Result<CurlCommand, CurlExportError> {
    if session.messages.is_empty() {
        return Err(CurlExportError::EmptySession);
    }

    let turns = extract_turns(&session.messages);
    let latest_index = if turns.is_empty() { 1 } else { turns.len() };

    generate_turn_curl(session, latest_index, config, options)
}

/// Generates a `CurlCommand` for the full session conversation buffer as currently constituted.
pub fn generate_curl_from_session(
    session: &Session,
    config: &Config,
    options: &CurlExportOptions,
) -> Result<CurlCommand, CurlExportError> {
    if session.messages.is_empty() {
        return Err(CurlExportError::EmptySession);
    }

    let mut opts = options.clone();
    opts.scope = TurnRequestScope::AllMessages;

    let turns = extract_turns(&session.messages);
    let latest_index = if turns.is_empty() { 1 } else { turns.len() };

    generate_turn_curl(session, latest_index, config, &opts)
}

/// Core low-level generator: builds a `CurlCommand` from explicit messages, tools, provider, and model parameters.
pub fn generate_curl_from_messages(
    messages: &[Message],
    tools: &[ToolDefinition],
    provider: &str,
    model: &str,
    base_url: &str,
    api_key: Option<&str>,
    options: &CurlExportOptions,
) -> Result<CurlCommand, CurlExportError> {
    let prov_lower = provider.to_lowercase();

    let (url, headers, payload) = if prov_lower == "anthropic" || prov_lower == "claude" {
        build_anthropic_request(messages, tools, model, base_url, api_key, options)
    } else if prov_lower == "ollama" {
        build_ollama_request(messages, tools, model, base_url, api_key, options)
    } else if prov_lower == "openrouter" || base_url.contains("openrouter.ai") {
        build_openrouter_request(messages, tools, model, base_url, api_key, options)
    } else {
        // OpenAI and OpenAI-compatible endpoints (DeepSeek, Groq, xAI, Mistral, Together, etc.)
        build_openai_compatible_request(
            &prov_lower,
            messages,
            tools,
            model,
            base_url,
            api_key,
            options,
        )
    };

    let mut metadata = HashMap::new();
    metadata.insert("messages_count".to_string(), messages.len().to_string());
    metadata.insert("tools_count".to_string(), tools.len().to_string());
    metadata.insert("stream".to_string(), options.stream.to_string());

    Ok(CurlCommand {
        method: "POST".to_string(),
        url,
        headers,
        body: payload,
        provider: provider.to_string(),
        model: model.to_string(),
        turn_index: None,
        options: options.clone(),
        metadata,
    })
}

/// Exports all turns from a session as a sequential list of `(turn_index, CurlCommand)`.
pub fn export_all_turns_curl(
    session: &Session,
    config: &Config,
    options: &CurlExportOptions,
) -> Result<Vec<(usize, CurlCommand)>, CurlExportError> {
    if session.messages.is_empty() {
        return Err(CurlExportError::EmptySession);
    }

    let turns = extract_turns(&session.messages);
    if turns.is_empty() {
        let cmd = generate_latest_turn_curl(session, config, options)?;
        return Ok(vec![(1, cmd)]);
    }

    let mut results = Vec::with_capacity(turns.len());
    for (i, _) in turns.iter().enumerate() {
        let turn_idx = i + 1;
        let cmd = generate_turn_curl(session, turn_idx, config, options)?;
        results.push((turn_idx, cmd));
    }

    Ok(results)
}

/// Generates a complete, multi-format `CurlReproductionBundle` for a given turn.
pub fn generate_reproduction_bundle(
    session: &Session,
    turn_index: usize,
    config: &Config,
    options: &CurlExportOptions,
) -> Result<CurlReproductionBundle, CurlExportError> {
    let command = generate_turn_curl(session, turn_index, config, options)?;

    let curl_bash = command.to_bash();
    let curl_single_line = command.to_single_line();
    let curl_powershell = command.to_powershell();
    let curl_cmd = command.to_cmd();
    let python_script = command.to_python_requests();
    let fetch_js = command.to_fetch_js();
    let raw_http = command.to_http_raw();
    let payload_json = command.body_json_pretty();

    let messages_count = command
        .metadata
        .get("messages_count")
        .and_then(|v| v.parse::<usize>().ok())
        .unwrap_or(0);

    let tools_count = command
        .metadata
        .get("tools_count")
        .and_then(|v| v.parse::<usize>().ok())
        .unwrap_or(0);

    // Approximate token estimation (chars / 4)
    let estimated_tokens = (payload_json.len() as f32 / 4.0).ceil() as u64;

    let reproduction_guide = format!(
        "# Reproduction Guide for Turn {}\n\n\
        - **Provider**: `{}`\n\
        - **Model**: `{}`\n\
        - **Endpoint**: `{}`\n\
        - **Messages**: `{}`\n\
        - **Tools**: `{}`\n\
        - **Estimated Prompt Tokens**: `~{}`\n\n\
        ### Execution\n\n\
        Execute the cURL command directly in your terminal:\n\n\
        ```bash\n\
        {}\n\
        ```\n",
        turn_index,
        command.provider,
        command.model,
        command.url,
        messages_count,
        tools_count,
        estimated_tokens,
        curl_bash
    );

    Ok(CurlReproductionBundle {
        turn_index,
        provider: command.provider.clone(),
        model: command.model.clone(),
        command,
        curl_bash,
        curl_single_line,
        curl_powershell,
        curl_cmd,
        python_script,
        fetch_js,
        raw_http,
        payload_json,
        messages_count,
        tools_count,
        estimated_tokens,
        reproduction_guide,
    })
}

/// Generates a standalone executable `.sh` script for reproducing turn(s).
pub fn generate_curl_script(
    session: &Session,
    turn_index: Option<usize>,
    config: &Config,
    options: &CurlExportOptions,
) -> Result<String, CurlExportError> {
    let cmd = match turn_index {
        Some(t) => generate_turn_curl(session, t, config, options)?,
        None => generate_latest_turn_curl(session, config, options)?,
    };

    Ok(cmd.to_script())
}

// ============================================================================
// Internal Provider Request Builders
// ============================================================================

/// Builds OpenAI-compatible endpoint URL, headers, and request payload.
fn build_openai_compatible_request(
    provider: &str,
    messages: &[Message],
    tools: &[ToolDefinition],
    model: &str,
    base_url: &str,
    api_key: Option<&str>,
    options: &CurlExportOptions,
) -> (String, Vec<(String, String)>, Value) {
    let url = construct_openai_url(base_url);
    let mut headers = Vec::new();
    headers.push(("Content-Type".to_string(), "application/json".to_string()));

    let auth_header_value = format_auth_bearer(provider, api_key, options.api_key_visibility);
    headers.push(("Authorization".to_string(), auth_header_value));

    for (k, v) in &options.custom_headers {
        headers.push((k.clone(), v.clone()));
    }

    let mut payload = json!({
        "model": model,
        "stream": options.stream,
    });

    if options.stream {
        payload["stream_options"] = json!({ "include_usage": true });
    }

    if let Some(temp) = options.temperature {
        payload["temperature"] = json!(temp);
    }

    if let Some(mt) = options.max_tokens {
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

    if !tools.is_empty() && options.include_tools {
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

    (url, headers, payload)
}

/// Builds OpenRouter endpoint URL, headers, and request payload.
fn build_openrouter_request(
    messages: &[Message],
    tools: &[ToolDefinition],
    model: &str,
    base_url: &str,
    api_key: Option<&str>,
    options: &CurlExportOptions,
) -> (String, Vec<(String, String)>, Value) {
    let (url, mut headers, payload) = build_openai_compatible_request(
        "openrouter",
        messages,
        tools,
        model,
        base_url,
        api_key,
        options,
    );

    headers.push((
        "HTTP-Referer".to_string(),
        "https://github.com/theaungmyatmoe/fusion".to_string(),
    ));
    headers.push(("X-Title".to_string(), "Fusion AI Assistant".to_string()));

    (url, headers, payload)
}

/// Builds Anthropic endpoint URL, headers, and request payload.
fn build_anthropic_request(
    messages: &[Message],
    tools: &[ToolDefinition],
    model: &str,
    base_url: &str,
    api_key: Option<&str>,
    options: &CurlExportOptions,
) -> (String, Vec<(String, String)>, Value) {
    let url = construct_anthropic_url(base_url);
    let mut headers = Vec::new();
    headers.push(("Content-Type".to_string(), "application/json".to_string()));
    headers.push(("anthropic-version".to_string(), "2023-06-01".to_string()));

    let key_val = format_api_key_value("anthropic", api_key, options.api_key_visibility);
    headers.push(("x-api-key".to_string(), key_val));

    for (k, v) in &options.custom_headers {
        headers.push((k.clone(), v.clone()));
    }

    let mut payload = json!({
        "model": model,
        "max_tokens": options.max_tokens.unwrap_or(4096),
        "stream": options.stream,
    });

    if let Some(temp) = options.temperature {
        payload["temperature"] = json!(temp);
    }

    // Extract system messages into root "system" parameter
    let system_prompts: Vec<&str> = messages
        .iter()
        .filter(|m| m.role == Role::System)
        .map(|m| m.content.as_str())
        .filter(|c| !c.is_empty())
        .collect();

    if !system_prompts.is_empty() {
        payload["system"] = json!(system_prompts.join("\n\n"));
    }

    // Convert conversation messages (User, Assistant, Tool)
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

    if !tools.is_empty() && options.include_tools {
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

    (url, headers, payload)
}

/// Builds Ollama endpoint URL, headers, and request payload.
fn build_ollama_request(
    messages: &[Message],
    tools: &[ToolDefinition],
    model: &str,
    base_url: &str,
    api_key: Option<&str>,
    options: &CurlExportOptions,
) -> (String, Vec<(String, String)>, Value) {
    let base = base_url.trim_end_matches('/');
    let url = if base.ends_with("/api/chat") {
        base.to_string()
    } else {
        format!("{}/api/chat", base)
    };

    let mut headers = Vec::new();
    headers.push(("Content-Type".to_string(), "application/json".to_string()));

    if let Some(key) = api_key {
        if !key.trim().is_empty() {
            let auth_val = format_auth_bearer("ollama", Some(key), options.api_key_visibility);
            headers.push(("Authorization".to_string(), auth_val));
        }
    }

    for (k, v) in &options.custom_headers {
        headers.push((k.clone(), v.clone()));
    }

    let mut ollama_messages = Vec::new();
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

        if let Some(tool_calls) = &msg.tool_calls {
            if !tool_calls.is_empty() {
                let tc_json: Vec<Value> = tool_calls
                    .iter()
                    .map(|tc| {
                        let args_val: Value =
                            serde_json::from_str(&tc.arguments).unwrap_or_else(|_| json!({}));
                        json!({
                            "function": {
                                "name": tc.name,
                                "arguments": args_val,
                            }
                        })
                    })
                    .collect();
                item["tool_calls"] = json!(tc_json);
            }
        }

        ollama_messages.push(item);
    }

    let mut payload = json!({
        "model": model,
        "messages": ollama_messages,
        "stream": options.stream,
    });

    if options.temperature.is_some() || options.max_tokens.is_some() {
        let mut opt_obj = json!({});
        if let Some(temp) = options.temperature {
            opt_obj["temperature"] = json!(temp);
        }
        if let Some(mt) = options.max_tokens {
            opt_obj["num_predict"] = json!(mt);
        }
        payload["options"] = opt_obj;
    }

    if !tools.is_empty() && options.include_tools {
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

    (url, headers, payload)
}

// ============================================================================
// URL Construction Helpers
// ============================================================================

/// Constructs the full OpenAI chat completions endpoint URL.
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

/// Constructs the full Anthropic messages endpoint URL.
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

// ============================================================================
// Credentials & Key Formatting
// ============================================================================

/// Formats the `Authorization: Bearer <...>` header value based on visibility mode.
pub fn format_auth_bearer(
    provider: &str,
    api_key: Option<&str>,
    visibility: ApiKeyVisibility,
) -> String {
    let key_str = format_api_key_value(provider, api_key, visibility);
    format!("Bearer {}", key_str)
}

/// Formats the raw API key string based on visibility mode.
pub fn format_api_key_value(
    provider: &str,
    api_key: Option<&str>,
    visibility: ApiKeyVisibility,
) -> String {
    match visibility {
        ApiKeyVisibility::EnvVar => {
            let env_var = provider_env_var_name(provider);
            format!("${}", env_var)
        }
        ApiKeyVisibility::Masked => {
            if let Some(key) = api_key {
                mask_api_key(key)
            } else {
                "[MASKED_API_KEY]".to_string()
            }
        }
        ApiKeyVisibility::Redacted => "[REDACTED]".to_string(),
        ApiKeyVisibility::Placeholder => {
            let env_var = provider_env_var_name(provider);
            format!("YOUR_{}", env_var)
        }
        ApiKeyVisibility::Plain => api_key.unwrap_or("").to_string(),
    }
}

/// Returns the standard environment variable name for a given provider.
pub fn provider_env_var_name(provider: &str) -> &'static str {
    match provider.to_lowercase().as_str() {
        "anthropic" | "claude" => "ANTHROPIC_API_KEY",
        "openai" => "OPENAI_API_KEY",
        "deepseek" => "DEEPSEEK_API_KEY",
        "xai" | "grok" => "XAI_API_KEY",
        "openrouter" => "OPENROUTER_API_KEY",
        "groq" => "GROQ_API_KEY",
        "mistral" => "MISTRAL_API_KEY",
        "together" => "TOGETHER_API_KEY",
        "ollama" => "OLLAMA_API_KEY",
        _ => "FUSION_API_KEY",
    }
}

/// Partially masks an API key, revealing prefix and suffix while obscuring the center.
pub fn mask_api_key(key: &str) -> String {
    let trimmed = key.trim();
    if trimmed.is_empty() {
        return String::new();
    }

    let char_count = trimmed.chars().count();
    if char_count <= 8 {
        return "***".to_string();
    }

    let prefix_len = if char_count > 20 { 7 } else { 3 };
    let suffix_len = if char_count > 20 { 4 } else { 3 };

    let prefix: String = trimmed.chars().take(prefix_len).collect();
    let suffix: String = trimmed.chars().skip(char_count - suffix_len).collect();

    format!("{}-***{}", prefix, suffix)
}

// ============================================================================
// Escaping & String Helpers
// ============================================================================

/// Safely escapes a string for inclusion inside POSIX single quotes (`'...'`).
/// In Bash/POSIX sh, single quotes cannot appear literally inside `'...'`;
/// each `'` is replaced with `'\''` (close quote, escaped quote, re-open quote).
pub fn escape_bash_single_quote(input: &str) -> String {
    input.replace('\'', "'\\''")
}

/// Escapes double quotes for inclusion inside Bash double-quoted strings (`"..."`).
pub fn escape_double_quotes_for_bash(input: &str) -> String {
    input.replace('\\', "\\\\").replace('"', "\\\"")
}

/// Escapes double quotes and special characters for Windows Command Prompt (`cmd.exe`).
pub fn escape_cmd_double_quote(input: &str) -> String {
    input.replace('\\', "\\\\").replace('"', "\\\"")
}

/// Extracts host and path components from a full URL.
fn parse_url_host_and_path(url: &str) -> (String, String) {
    if let Some(stripped) = url
        .strip_prefix("http://")
        .or_else(|| url.strip_prefix("https://"))
    {
        if let Some(slash_idx) = stripped.find('/') {
            let host = &stripped[..slash_idx];
            let path = &stripped[slash_idx..];
            (host.to_string(), path.to_string())
        } else {
            (stripped.to_string(), "/".to_string())
        }
    } else {
        ("localhost".to_string(), url.to_string())
    }
}

/// Capitalizes HTTP method for PowerShell formatting (e.g. "POST" -> "Post").
fn capitalize_method(method: &str) -> String {
    let mut chars = method.chars();
    match chars.next() {
        None => String::new(),
        Some(first) => first.to_uppercase().collect::<String>() + &chars.as_str().to_lowercase(),
    }
}

/// Prepares the message list for turn extraction, ensuring system prompt is included.
fn construct_turn_messages(session: &Session, target_slice: &[Message]) -> Vec<Message> {
    let has_system_in_slice = target_slice.iter().any(|m| m.role == Role::System);

    if !has_system_in_slice {
        if let Some(sys_prompt) = &session.system_prompt {
            if !sys_prompt.trim().is_empty() {
                let mut combined = Vec::with_capacity(target_slice.len() + 1);
                combined.push(Message::system(sys_prompt));
                combined.extend_from_slice(target_slice);
                return combined;
            }
        }
    }

    target_slice.to_vec()
}

/// Returns default registered tool definitions for tool inclusion.
fn get_available_tool_definitions() -> Vec<ToolDefinition> {
    // Construct default standard tool definitions
    vec![
        ToolDefinition {
            name: "read".to_string(),
            description: "Read a file from the filesystem".to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "Path to the file to read" }
                },
                "required": ["path"]
            }),
        },
        ToolDefinition {
            name: "write".to_string(),
            description: "Write content to a file".to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "Path to the file to write" },
                    "content": { "type": "string", "description": "Content to write" }
                },
                "required": ["path", "content"]
            }),
        },
        ToolDefinition {
            name: "edit".to_string(),
            description: "Edit a file using line-anchored patch operations".to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "input": { "type": "string", "description": "Patch input" }
                },
                "required": ["input"]
            }),
        },
        ToolDefinition {
            name: "bash".to_string(),
            description: "Execute a bash shell command".to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "command": { "type": "string", "description": "Command to execute" }
                },
                "required": ["command"]
            }),
        },
    ]
}

// ============================================================================
// Unit Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::provider::types::ToolCall;

    fn make_test_session() -> Session {
        let mut session = Session::new("gpt-4o");
        session.system_prompt = Some("You are a helpful Rust assistant.".to_string());

        // Turn 1
        session
            .messages
            .push(Message::user("What is the speed of light?"));
        session.messages.push(Message::assistant(
            "The speed of light is ~299,792,458 m/s.",
        ));

        // Turn 2 with tool calls
        session
            .messages
            .push(Message::user("Read Cargo.toml for me"));
        session.messages.push(Message::assistant_with_tools(
            "",
            vec![ToolCall {
                id: "call_123".to_string(),
                name: "read".to_string(),
                arguments: r#"{"path":"Cargo.toml"}"#.to_string(),
            }],
        ));
        session.messages.push(Message::tool_result(
            "call_123",
            "[package]\nname = \"fusion\"\n",
        ));
        session
            .messages
            .push(Message::assistant("The package name is 'fusion'."));

        session
    }

    #[test]
    fn test_generate_openai_curl_turn_1() {
        let session = make_test_session();
        let config = Config::default();
        let options = CurlExportOptions::default()
            .with_api_key_visibility(ApiKeyVisibility::EnvVar)
            .with_formatting(CurlFormatting::Multiline);

        let cmd = generate_turn_curl(&session, 1, &config, &options).expect("generate turn 1");

        assert_eq!(cmd.method, "POST");
        assert!(cmd.url.contains("/chat/completions"));
        assert!(cmd.has_header("Content-Type"));
        assert_eq!(cmd.header_value("Content-Type"), Some("application/json"));
        assert_eq!(
            cmd.header_value("Authorization"),
            Some("Bearer $OPENAI_API_KEY")
        );

        let body = &cmd.body;
        assert_eq!(body["model"], "gpt-4o");
        assert_eq!(body["stream"], true);

        let messages = body["messages"].as_array().expect("messages array");
        // System prompt + User prompt + Assistant response
        assert!(!messages.is_empty());
        assert_eq!(messages[0]["role"], "system");
        assert_eq!(messages[1]["role"], "user");

        let bash = cmd.to_bash();
        assert!(bash.starts_with("#"));
        assert!(bash.contains("curl -sS -X POST"));
        assert!(bash.contains("https://api.openai.com/v1/chat/completions"));
        assert!(bash.contains("-H \"Authorization: Bearer $OPENAI_API_KEY\""));
    }

    #[test]
    fn test_generate_anthropic_curl_turn_2() {
        let session = make_test_session();
        let mut config = Config::default();
        config.default_provider = "anthropic".to_string();
        config.default_model = "claude-3-7-sonnet-20250219".to_string();

        let options = CurlExportOptions::default()
            .with_provider("anthropic")
            .with_model("claude-3-7-sonnet-20250219")
            .with_api_key_visibility(ApiKeyVisibility::Masked)
            .with_api_key("sk-ant-api03-1234567890abcdefghijklmnopqrstuvwxyz");

        let cmd = generate_turn_curl(&session, 2, &config, &options).expect("generate turn 2");

        assert_eq!(cmd.method, "POST");
        assert!(cmd.url.contains("/v1/messages"));
        assert_eq!(cmd.header_value("Content-Type"), Some("application/json"));
        assert_eq!(cmd.header_value("anthropic-version"), Some("2023-06-01"));
        assert!(cmd.header_value("x-api-key").unwrap().contains("-***"));

        // Anthropic system prompt should be in root "system" property, not in messages array
        assert_eq!(cmd.body["system"], "You are a helpful Rust assistant.");

        let messages = cmd.body["messages"].as_array().expect("messages array");
        assert!(messages.iter().all(|m| m["role"] != "system"));

        // Check tools schema in Anthropic format (input_schema instead of parameters)
        let tools = cmd.body["tools"].as_array().expect("tools array");
        assert!(!tools.is_empty());
        assert!(tools[0].get("input_schema").is_some());
    }

    #[test]
    fn test_generate_ollama_curl() {
        let session = make_test_session();
        let mut config = Config::default();
        config.default_provider = "ollama".to_string();
        config.default_model = "llama3.2".to_string();

        let options = CurlExportOptions::default()
            .with_provider("ollama")
            .with_model("llama3.2");

        let cmd = generate_turn_curl(&session, 1, &config, &options).expect("generate ollama curl");

        assert_eq!(cmd.method, "POST");
        assert_eq!(cmd.url, "http://localhost:11434/api/chat");
        assert_eq!(cmd.body["model"], "llama3.2");
        assert_eq!(cmd.body["stream"], true);
    }

    #[test]
    fn test_openrouter_headers() {
        let session = make_test_session();
        let config = Config::default();
        let options = CurlExportOptions::default().with_provider("openrouter");

        let cmd = generate_turn_curl(&session, 1, &config, &options).expect("generate openrouter");

        assert_eq!(
            cmd.header_value("HTTP-Referer"),
            Some("https://github.com/theaungmyatmoe/fusion")
        );
        assert_eq!(cmd.header_value("X-Title"), Some("Fusion AI Assistant"));
        assert_eq!(
            cmd.header_value("Authorization"),
            Some("Bearer $OPENROUTER_API_KEY")
        );
    }

    #[test]
    fn test_api_key_visibility_modes() {
        let key = "sk-proj-1234567890abcdef12345678";

        assert_eq!(
            format_api_key_value("openai", Some(key), ApiKeyVisibility::EnvVar),
            "$OPENAI_API_KEY"
        );
        assert_eq!(
            format_api_key_value("anthropic", Some(key), ApiKeyVisibility::EnvVar),
            "$ANTHROPIC_API_KEY"
        );
        assert_eq!(
            format_api_key_value("openai", Some(key), ApiKeyVisibility::Redacted),
            "[REDACTED]"
        );
        assert_eq!(
            format_api_key_value("openai", Some(key), ApiKeyVisibility::Placeholder),
            "YOUR_OPENAI_API_KEY"
        );
        assert_eq!(
            format_api_key_value("openai", Some(key), ApiKeyVisibility::Plain),
            key
        );
        assert_eq!(mask_api_key(key), "sk-proj-***5678");
    }

    #[test]
    fn test_bash_single_quote_escaping() {
        let payload = r#"{"prompt":"don't fail on 'single quotes'!"}"#;
        let escaped = escape_bash_single_quote(payload);
        assert_eq!(
            escaped,
            r#"{"prompt":"don'\''t fail on '\''single quotes'\''!"}"#
        );
    }

    #[test]
    fn test_single_line_curl() {
        let session = make_test_session();
        let config = Config::default();
        let options = CurlExportOptions::default().with_formatting(CurlFormatting::SingleLine);

        let cmd = generate_turn_curl(&session, 1, &config, &options).expect("generate turn 1");
        let single = cmd.to_single_line();

        assert!(!single.contains('\n'));
        assert!(single.starts_with("curl"));
        assert!(single.contains("-X POST"));
    }

    #[test]
    fn test_powershell_and_cmd_formatting() {
        let session = make_test_session();
        let config = Config::default();
        let options = CurlExportOptions::default();

        let cmd = generate_turn_curl(&session, 1, &config, &options).expect("generate turn 1");

        let ps = cmd.to_powershell();
        assert!(ps.contains("$headers = @{"));
        assert!(ps.contains("Invoke-RestMethod"));

        let cmd_script = cmd.to_cmd();
        assert!(cmd_script.contains("curl.exe -X POST"));
        assert!(cmd_script.contains(" ^\n"));
    }

    #[test]
    fn test_python_and_js_generators() {
        let session = make_test_session();
        let config = Config::default();
        let options = CurlExportOptions::default();

        let cmd = generate_turn_curl(&session, 1, &config, &options).expect("generate turn 1");

        let py = cmd.to_python_requests();
        assert!(py.contains("import requests"));
        assert!(py.contains("response = requests.post(url, headers=headers"));

        let js = cmd.to_fetch_js();
        assert!(js.contains("async function reproduceRequest()"));
        assert!(js.contains("await fetch(url"));
    }

    #[test]
    fn test_raw_http_format() {
        let session = make_test_session();
        let config = Config::default();
        let options = CurlExportOptions::default();

        let cmd = generate_turn_curl(&session, 1, &config, &options).expect("generate turn 1");
        let raw = cmd.to_http_raw();

        assert!(raw.starts_with("POST /v1/chat/completions HTTP/1.1\r\n"));
        assert!(raw.contains("Host: api.openai.com\r\n"));
        assert!(raw.contains("Content-Length: "));
    }

    #[test]
    fn test_turn_out_of_bounds() {
        let session = make_test_session();
        let config = Config::default();
        let options = CurlExportOptions::default();

        let err = generate_turn_curl(&session, 999, &config, &options).unwrap_err();
        assert!(matches!(err, CurlExportError::TurnOutOfBounds(999, 2)));

        let err_zero = generate_turn_curl(&session, 0, &config, &options).unwrap_err();
        assert!(matches!(err_zero, CurlExportError::TurnOutOfBounds(0, 0)));
    }

    #[test]
    fn test_empty_session_error() {
        let session = Session::new("gpt-4o");
        let config = Config::default();
        let options = CurlExportOptions::default();

        let err = generate_turn_curl(&session, 1, &config, &options).unwrap_err();
        assert_eq!(err, CurlExportError::EmptySession);
    }

    #[test]
    fn test_export_all_turns() {
        let session = make_test_session();
        let config = Config::default();
        let options = CurlExportOptions::default();

        let all = export_all_turns_curl(&session, &config, &options).expect("export all turns");
        assert_eq!(all.len(), 2);
        assert_eq!(all[0].0, 1);
        assert_eq!(all[1].0, 2);
    }

    #[test]
    fn test_reproduction_bundle() {
        let session = make_test_session();
        let config = Config::default();
        let options = CurlExportOptions::default();

        let bundle = generate_reproduction_bundle(&session, 1, &config, &options).expect("bundle");
        assert_eq!(bundle.turn_index, 1);
        assert_eq!(bundle.provider, "openai");
        assert_eq!(bundle.model, "gpt-4o");
        assert!(!bundle.curl_bash.is_empty());
        assert!(!bundle.curl_powershell.is_empty());
        assert!(!bundle.python_script.is_empty());
        assert!(!bundle.fetch_js.is_empty());
        assert!(bundle.estimated_tokens > 0);
        assert!(bundle
            .reproduction_guide
            .contains("Reproduction Guide for Turn 1"));
    }

    #[test]
    fn test_generate_curl_script() {
        let session = make_test_session();
        let config = Config::default();
        let options = CurlExportOptions::default();

        let script = generate_curl_script(&session, Some(1), &config, &options).expect("script");
        assert!(script.starts_with("#!/usr/bin/env bash\n"));
        assert!(script.contains("set -euo pipefail"));
        assert!(script.contains("OPENAI_API_KEY"));
        assert!(script.contains("curl -sS -X POST"));
    }

    #[test]
    fn test_turn_scope_initial_prompt_vs_full_turn() {
        let session = make_test_session();
        let config = Config::default();

        // Initial prompt scope (only up to user prompt of turn 2)
        let opts_initial = CurlExportOptions::default().with_scope(TurnRequestScope::InitialPrompt);
        let cmd_initial = generate_turn_curl(&session, 2, &config, &opts_initial).expect("initial");
        let msgs_initial = cmd_initial.body["messages"].as_array().expect("msgs");

        // Full turn scope (includes tool results and assistant message)
        let opts_full = CurlExportOptions::default().with_scope(TurnRequestScope::FullTurn);
        let cmd_full = generate_turn_curl(&session, 2, &config, &opts_full).expect("full");
        let msgs_full = cmd_full.body["messages"].as_array().expect("msgs");

        assert!(msgs_initial.len() < msgs_full.len());
    }

    // ========================================================================
    // HTTP Client Agent Utility Unit Tests
    // ========================================================================

    #[test]
    fn test_base64_encode_rfc4648_test_vectors() {
        assert_eq!(base64_encode(b""), "");
        assert_eq!(base64_encode(b"f"), "Zg==");
        assert_eq!(base64_encode(b"fo"), "Zm8=");
        assert_eq!(base64_encode(b"foo"), "Zm9v");
        assert_eq!(base64_encode(b"foob"), "Zm9vYg==");
        assert_eq!(base64_encode(b"fooba"), "Zm9vYmE=");
        assert_eq!(base64_encode(b"foobar"), "Zm9vYmFy");
    }

    #[test]
    fn test_validate_url_valid() {
        assert!(validate_url("https://api.example.com/v1/resource").is_ok());
        assert!(validate_url("http://localhost:8080/health").is_ok());
        assert!(validate_url("https://127.0.0.1:3000/api?query=1").is_ok());
        assert!(validate_url("http://example.org:80/path/to/page").is_ok());
    }

    #[test]
    fn test_validate_url_invalid_schemes() {
        assert!(validate_url("ftp://ftp.example.com/file.zip").is_err());
        assert!(validate_url("file:///etc/passwd").is_err());
        assert!(validate_url("ssh://git@github.com").is_err());
        assert!(validate_url("javascript:alert(1)").is_err());
        assert!(validate_url("data:text/plain;base64,SGVsbG8=").is_err());
    }

    #[test]
    fn test_validate_url_empty_and_malformed() {
        assert!(validate_url("").is_err());
        assert!(validate_url("   ").is_err());
        assert!(validate_url("not-a-valid-url").is_err());
        assert!(validate_url("http://").is_err());
        assert!(validate_url("https://").is_err());
    }

    #[test]
    fn test_is_private_or_local_host() {
        let localhost_url = validate_url("http://localhost:3000").expect("url");
        assert!(is_private_or_local_host(&localhost_url));

        let loopback_url = validate_url("http://127.0.0.1:8080").expect("url");
        assert!(is_private_or_local_host(&loopback_url));

        let priv_10 = validate_url("http://10.0.0.1/api").expect("url");
        assert!(is_private_or_local_host(&priv_10));

        let priv_192 = validate_url("http://192.168.1.100/status").expect("url");
        assert!(is_private_or_local_host(&priv_192));

        let link_local = validate_url("http://169.254.169.254/latest/meta-data").expect("url");
        assert!(is_private_or_local_host(&link_local));

        let public_url = validate_url("https://api.github.com/repos").expect("url");
        assert!(!is_private_or_local_host(&public_url));
    }

    #[test]
    fn test_append_query_params() {
        let base = "https://api.example.com/search";
        let params = vec![
            ("q".to_string(), "rust coding".to_string()),
            ("limit".to_string(), "10".to_string()),
        ];
        let resolved = append_query_params(base, &params).expect("query append");
        assert!(resolved.contains("q=rust+coding") || resolved.contains("q=rust%20coding"));
        assert!(resolved.contains("limit=10"));

        // Appending to URL with existing query
        let base_with_query = "https://api.example.com/search?sort=desc";
        let extra = vec![("filter".to_string(), "active".to_string())];
        let resolved2 = append_query_params(base_with_query, &extra).expect("query append");
        assert!(resolved2.contains("sort=desc"));
        assert!(resolved2.contains("filter=active"));
    }

    #[test]
    fn test_http_method_conversions() {
        assert_eq!(HttpMethod::Get.as_str(), "GET");
        assert_eq!(HttpMethod::Post.as_str(), "POST");
        assert_eq!(HttpMethod::Put.as_str(), "PUT");
        assert_eq!(HttpMethod::Delete.as_str(), "DELETE");
        assert_eq!(HttpMethod::Patch.as_str(), "PATCH");
        assert_eq!(HttpMethod::Head.as_str(), "HEAD");
        assert_eq!(HttpMethod::Options.as_str(), "OPTIONS");

        assert_eq!("get".parse::<HttpMethod>().expect("parse"), HttpMethod::Get);
        assert_eq!(
            "POST".parse::<HttpMethod>().expect("parse"),
            HttpMethod::Post
        );
        assert_eq!("put".parse::<HttpMethod>().expect("parse"), HttpMethod::Put);
        assert_eq!(
            "delete".parse::<HttpMethod>().expect("parse"),
            HttpMethod::Delete
        );
        assert_eq!(
            "patch".parse::<HttpMethod>().expect("parse"),
            HttpMethod::Patch
        );
        assert_eq!(
            "head".parse::<HttpMethod>().expect("parse"),
            HttpMethod::Head
        );
        assert_eq!(
            "options".parse::<HttpMethod>().expect("parse"),
            HttpMethod::Options
        );

        assert!("invalid_method".parse::<HttpMethod>().is_err());
    }

    #[test]
    fn test_http_auth_bearer() {
        let auth = HttpAuth::bearer("secret_token_123");
        let (name, val) = auth.to_header_pair();
        assert_eq!(name, "Authorization");
        assert_eq!(val, "Bearer secret_token_123");
    }

    #[test]
    fn test_http_auth_basic() {
        let auth = HttpAuth::basic("admin", Some("password123"));
        let (name, val) = auth.to_header_pair();
        assert_eq!(name, "Authorization");
        // admin:password123 -> YWRtaW46cGFzc3dvcmQxMjM=
        assert_eq!(val, "Basic YWRtaW46cGFzc3dvcmQxMjM=");

        let auth_no_pass = HttpAuth::basic("user_only", None::<String>);
        let (name2, val2) = auth_no_pass.to_header_pair();
        assert_eq!(name2, "Authorization");
        assert!(val2.starts_with("Basic "));
    }

    #[test]
    fn test_http_auth_api_key() {
        let auth = HttpAuth::api_key("X-API-Key", "custom_key_abc");
        let (name, val) = auth.to_header_pair();
        assert_eq!(name, "X-API-Key");
        assert_eq!(val, "custom_key_abc");
    }

    #[test]
    fn test_request_builder_get() {
        let req = CurlRequestBuilder::new(HttpMethod::Get, "https://api.example.com/items")
            .header("Accept", "application/json")
            .query("page", "1")
            .query("limit", "25")
            .bearer_auth("token_xyz")
            .timeout_secs(15)
            .build()
            .expect("build get request");

        assert_eq!(req.method, HttpMethod::Get);
        assert_eq!(req.url, "https://api.example.com/items");
        assert_eq!(req.query_params.len(), 2);
        assert_eq!(req.auth, Some(HttpAuth::bearer("token_xyz")));
        assert_eq!(req.timeout, Some(Duration::from_secs(15)));

        let resolved_url = req.resolved_url().expect("resolved url");
        assert!(resolved_url.contains("page=1"));
        assert!(resolved_url.contains("limit=25"));

        let headers = req.resolved_headers();
        assert!(headers
            .iter()
            .any(|(k, v)| k == "Accept" && v == "application/json"));
        assert!(headers
            .iter()
            .any(|(k, v)| k == "Authorization" && v == "Bearer token_xyz"));
    }

    #[test]
    fn test_request_builder_post_json() {
        let payload = json!({
            "name": "Widget",
            "quantity": 42
        });

        let req = CurlRequestBuilder::new(HttpMethod::Post, "https://api.example.com/items")
            .json(&payload)
            .expect("json body")
            .build()
            .expect("build post request");

        assert_eq!(req.method, HttpMethod::Post);
        assert_eq!(req.body, HttpBody::Json(payload));

        let headers = req.resolved_headers();
        assert!(headers
            .iter()
            .any(|(k, v)| k.eq_ignore_ascii_case("content-type") && v == "application/json"));
    }

    #[test]
    fn test_request_builder_put_text() {
        let req = CurlRequestBuilder::new(HttpMethod::Put, "https://api.example.com/items/42")
            .text("raw updated text payload")
            .header("Content-Type", "text/markdown")
            .build()
            .expect("build put request");

        assert_eq!(req.method, HttpMethod::Put);
        assert_eq!(
            req.body,
            HttpBody::Text("raw updated text payload".to_string())
        );

        let headers = req.resolved_headers();
        assert!(headers
            .iter()
            .any(|(k, v)| k == "Content-Type" && v == "text/markdown"));
    }

    #[test]
    fn test_request_builder_delete() {
        let req = CurlRequestBuilder::new(HttpMethod::Delete, "https://api.example.com/items/42")
            .api_key("X-Auth", "admin_secret")
            .build()
            .expect("build delete request");

        assert_eq!(req.method, HttpMethod::Delete);
        assert_eq!(req.auth, Some(HttpAuth::api_key("X-Auth", "admin_secret")));
        let headers = req.resolved_headers();
        assert!(headers
            .iter()
            .any(|(k, v)| k == "X-Auth" && v == "admin_secret"));
    }

    #[test]
    fn test_request_builder_patch_form() {
        let form_data = vec![
            ("status".to_string(), "approved".to_string()),
            ("reviewer".to_string(), "alice".to_string()),
        ];

        let req = CurlRequestBuilder::new(HttpMethod::Patch, "https://api.example.com/items/42")
            .form(form_data.clone())
            .build()
            .expect("build patch request");

        assert_eq!(req.method, HttpMethod::Patch);
        assert_eq!(req.body, HttpBody::Form(form_data));

        let headers = req.resolved_headers();
        assert!(headers
            .iter()
            .any(|(k, v)| k.eq_ignore_ascii_case("content-type")
                && v == "application/x-www-form-urlencoded"));
    }

    #[test]
    fn test_request_builder_head() {
        let req = CurlRequestBuilder::new(HttpMethod::Head, "https://example.com/asset.tar.gz")
            .build()
            .expect("build head request");

        assert_eq!(req.method, HttpMethod::Head);
        assert_eq!(req.body, HttpBody::Empty);
    }

    #[test]
    fn test_request_to_curl_command() {
        let req = CurlRequestBuilder::new(HttpMethod::Post, "https://api.example.com/v1/messages")
            .bearer_auth("sk-test-12345")
            .json(&json!({ "model": "gpt-4o", "prompt": "hello" }))
            .expect("json")
            .build()
            .expect("build request");

        let curl_cmd = req.to_curl_command().expect("to curl command");
        assert_eq!(curl_cmd.method, "POST");
        assert_eq!(curl_cmd.url, "https://api.example.com/v1/messages");
        assert!(curl_cmd.has_header("Authorization"));
        assert!(curl_cmd.has_header("Content-Type"));

        let bash_script = curl_cmd.to_bash();
        assert!(bash_script.contains("curl -sS -X POST"));
        assert!(bash_script.contains("https://api.example.com/v1/messages"));
    }

    #[test]
    fn test_http_response_helpers() {
        let resp = HttpResponse {
            status: 200,
            status_text: "OK".to_string(),
            headers: {
                let mut h = HashMap::new();
                h.insert("content-type".to_string(), "application/json".to_string());
                h.insert("x-request-id".to_string(), "req_123".to_string());
                h
            },
            body: br#"{"status":"success","count":5}"#.to_vec(),
            content_type: Some("application/json".to_string()),
            content_length: Some(30),
            duration: Duration::from_millis(45),
            url: "https://api.example.com/data".to_string(),
        };

        assert!(resp.is_success());
        assert!(!resp.is_client_error());
        assert!(!resp.is_server_error());
        assert_eq!(resp.header("X-Request-ID"), Some("req_123"));
        assert_eq!(resp.header("content-type"), Some("application/json"));

        let text = resp.text().expect("text decode");
        assert!(text.contains("success"));

        let json_val = resp.json_value().expect("json parse");
        assert_eq!(json_val["status"], "success");
        assert_eq!(json_val["count"], 5);
    }

    #[test]
    fn test_curl_agent_creation_and_defaults() {
        let agent = CurlAgent::new()
            .with_default_timeout(Duration::from_secs(20))
            .with_max_response_bytes(5 * 1024 * 1024)
            .with_user_agent("Custom-Agent/1.0");

        let req_builder = agent.get("https://httpbin.org/get");
        let req = req_builder.build().expect("build");

        assert_eq!(req.method, HttpMethod::Get);
        assert_eq!(req.timeout, Some(Duration::from_secs(20)));
        assert_eq!(req.max_response_bytes, 5 * 1024 * 1024);
        assert_eq!(req.user_agent, Some("Custom-Agent/1.0".to_string()));
    }
}
