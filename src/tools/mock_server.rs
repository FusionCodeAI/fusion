//! Local In-Process Mock HTTP Server Tool.
//!
//! Provides a lightweight, high-performance in-process HTTP mock server for testing
//! REST endpoints, webhooks, callbacks, and microservice integrations during development.
//!
//! Features:
//! - Pure-Rust async HTTP/1.1 server built on Tokio with zero external daemons.
//! - Dynamic or fixed port binding (`127.0.0.1:0` for ephemeral ports).
//! - Flexible route matching: method (GET, POST, PUT, DELETE, PATCH, etc. or ANY/*),
//!   exact paths, prefix wildcards (`/api/*`), path variables (`/users/:id`), and regex.
//! - Query parameter, header, and body matching criteria.
//! - Dynamic response templating (`{{method}}`, `{{path}}`, `{{uuid}}`, `{{timestamp}}`, `{{body}}`, `{{json.key}}`, `{{query.key}}`).
//! - Artificial latency simulation (`delay_ms`) and call limit countdowns.
//! - Request recording and full inspection (method, path, query, headers, body, timestamp, client IP).
//! - Webhook assertion & verification (`verify` action with counts, payload matching, and header checks).
//! - Built-in test request sender for self-contained integration checks.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::{HashMap, VecDeque};
use std::net::SocketAddr;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, LazyLock};
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{watch, RwLock};

use crate::tools::types::{Tool, ToolContext};

// ===========================================================================
// Data Models & Configuration
// ===========================================================================

/// Lifecycle status of a mock server.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MockServerStatus {
    Starting,
    Running,
    Stopped,
    Failed(String),
}

impl std::fmt::Display for MockServerStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            MockServerStatus::Starting => write!(f, "starting"),
            MockServerStatus::Running => write!(f, "running"),
            MockServerStatus::Stopped => write!(f, "stopped"),
            MockServerStatus::Failed(err) => write!(f, "failed: {}", err),
        }
    }
}

/// Recorded incoming HTTP request.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MockRequest {
    /// Unique identifier for this recorded request.
    pub id: String,
    /// ID of the mock server that handled the request.
    pub server_id: String,
    /// Timestamp in RFC 3339 format when the request arrived.
    pub timestamp: String,
    /// HTTP method in uppercase (e.g. GET, POST, PUT).
    pub method: String,
    /// Request URL path (e.g. `/api/v1/webhooks`).
    pub path: String,
    /// Raw query string if present (e.g. `foo=bar&baz=1`).
    pub query: Option<String>,
    /// Parsed query parameters map.
    pub query_params: HashMap<String, String>,
    /// Request headers map with lowercase keys.
    pub headers: HashMap<String, String>,
    /// Request body as UTF-8 string.
    pub body: Option<String>,
    /// Parsed JSON value if the body is valid JSON.
    pub body_json: Option<Value>,
    /// Client IP address and port.
    pub client_ip: String,
    /// ID of the mock route that matched, if any.
    pub matched_route_id: Option<String>,
    /// HTTP response status code sent back.
    pub response_status: u16,
}

/// A configured mock endpoint route.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MockRoute {
    /// Unique route identifier.
    #[serde(default = "generate_uuid")]
    pub id: String,
    /// Optional human-readable route name or description.
    pub name: Option<String>,
    /// HTTP method to match (e.g. "GET", "POST", "ANY", "*").
    #[serde(default = "default_route_method")]
    pub method: String,
    /// Path pattern to match (e.g. "/webhook", "/api/*", "/users/:id", "regex:^/items/\\d+$").
    pub path: String,
    /// Optional query parameters that must be present and match.
    pub query_params: Option<HashMap<String, String>>,
    /// Optional request headers that must be present and match.
    pub headers: Option<HashMap<String, String>>,
    /// Optional substring or regex pattern that the request body must match.
    pub body_match: Option<String>,
    /// Response HTTP status code (default 200).
    #[serde(default = "default_route_status")]
    pub status: u16,
    /// Optional custom HTTP status text (e.g. "OK", "Created").
    pub status_text: Option<String>,
    /// Response headers to send back.
    #[serde(default)]
    pub response_headers: HashMap<String, String>,
    /// Response body content (supports templating).
    #[serde(default = "default_route_body")]
    pub response_body: String,
    /// Artificial delay in milliseconds before returning response.
    pub delay_ms: Option<u64>,
    /// Optional limit on how many times this route can match.
    pub call_limit: Option<usize>,
    /// Number of times this route has been matched.
    #[serde(default)]
    pub call_count: usize,
    /// Route matching priority (higher priority checked first). Default 0.
    #[serde(default)]
    pub priority: i32,
}

fn generate_uuid() -> String {
    uuid::Uuid::new_v4().to_string()
}

fn default_route_method() -> String {
    "ANY".to_string()
}

fn default_route_status() -> u16 {
    200
}

fn default_route_body() -> String {
    "{\"status\":\"ok\"}".to_string()
}

impl MockRoute {
    /// Create a simple GET route.
    pub fn get(path: impl Into<String>, body: impl Into<String>) -> Self {
        let mut headers = HashMap::new();
        headers.insert("Content-Type".to_string(), "application/json".to_string());
        Self {
            id: generate_uuid(),
            name: None,
            method: "GET".to_string(),
            path: path.into(),
            query_params: None,
            headers: None,
            body_match: None,
            status: 200,
            status_text: Some("OK".to_string()),
            response_headers: headers,
            response_body: body.into(),
            delay_ms: None,
            call_limit: None,
            call_count: 0,
            priority: 0,
        }
    }

    /// Create a simple POST route.
    pub fn post(path: impl Into<String>, status: u16, body: impl Into<String>) -> Self {
        let mut headers = HashMap::new();
        headers.insert("Content-Type".to_string(), "application/json".to_string());
        Self {
            id: generate_uuid(),
            name: None,
            method: "POST".to_string(),
            path: path.into(),
            query_params: None,
            headers: None,
            body_match: None,
            status,
            status_text: None,
            response_headers: headers,
            response_body: body.into(),
            delay_ms: None,
            call_limit: None,
            call_count: 0,
            priority: 0,
        }
    }

    /// Check if an incoming request matches this route's criteria.
    pub fn matches(&self, method: &str, path: &str, query: &HashMap<String, String>, headers: &HashMap<String, String>, body: Option<&str>) -> bool {
        // Check call limit
        if let Some(limit) = self.call_limit {
            if self.call_count >= limit {
                return false;
            }
        }

        // 1. Method match
        let route_method = self.method.to_uppercase();
        if route_method != "ANY" && route_method != "*" && route_method != method.to_uppercase() {
            return false;
        }

        // 2. Path match
        if !match_path_pattern(&self.path, path) {
            return false;
        }

        // 3. Query match
        if let Some(required_query) = &self.query_params {
            for (k, v) in required_query {
                if query.get(k) != Some(v) {
                    return false;
                }
            }
        }

        // 4. Header match
        if let Some(required_headers) = &self.headers {
            for (k, v) in required_headers {
                let k_lower = k.to_lowercase();
                if headers.get(&k_lower) != Some(v) {
                    return false;
                }
            }
        }

        // 5. Body match
        if let Some(pattern) = &self.body_match {
            let body_str = body.unwrap_or("");
            if pattern.starts_with("regex:") {
                let re_str = &pattern[6..];
                if let Ok(re) = regex::Regex::new(re_str) {
                    if !re.is_match(body_str) {
                        return false;
                    }
                } else {
                    return false;
                }
            } else if !body_str.contains(pattern) {
                return false;
            }
        }

        true
    }
}

/// Matches path patterns including exact, wildcard prefix (`/api/*`), path variable (`/users/:id`), and regex.
pub fn match_path_pattern(pattern: &str, path: &str) -> bool {
    let p = pattern.trim();
    let target = path.trim();

    if p == target || p == "*" || p == "/*" {
        return true;
    }

    // Regex match
    if let Some(re_pattern) = p.strip_prefix("regex:") {
        if let Ok(re) = regex::Regex::new(re_pattern) {
            return re.is_match(target);
        }
        return false;
    }

    // Wildcard prefix / suffix: e.g. /api/* or /api/**
    if p.ends_with("/**") {
        let prefix = &p[..p.len() - 3];
        return target.starts_with(prefix) || target == prefix.trim_end_matches('/');
    }
    if p.ends_with("/*") {
        let prefix = &p[..p.len() - 2];
        return target.starts_with(prefix) || target == prefix.trim_end_matches('/');
    }
    if p.starts_with("*/") {
        let suffix = &p[1..];
        return target.ends_with(suffix);
    }

    // Parameterized path match (e.g. /users/:id/posts/:post_id)
    let p_segments: Vec<&str> = p.split('/').filter(|s| !s.is_empty()).collect();
    let t_segments: Vec<&str> = target.split('/').filter(|s| !s.is_empty()).collect();

    if p_segments.len() != t_segments.len() {
        return false;
    }

    for (p_seg, t_seg) in p_segments.iter().zip(t_segments.iter()) {
        if p_seg.starts_with(':') || *p_seg == "*" {
            continue;
        }
        if p_seg != t_seg {
            return false;
        }
    }

    true
}

/// Configuration options when starting a mock server.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MockServerConfig {
    /// Port to bind to (0 or None = dynamic port).
    pub port: Option<u16>,
    /// Host to bind to (default "127.0.0.1").
    pub host: Option<String>,
    /// Server name or tag.
    pub name: Option<String>,
    /// Enable CORS automatic headers & OPTIONS preflight (default true).
    pub cors: Option<bool>,
    /// Default status code if no route matches (default 404).
    pub default_status: Option<u16>,
    /// Default body content if no route matches.
    pub default_body: Option<String>,
    /// Maximum number of request history entries to keep (default 500).
    pub max_history: Option<usize>,
    /// Initial mock routes.
    #[serde(default)]
    pub routes: Vec<MockRoute>,
}

impl Default for MockServerConfig {
    fn default() -> Self {
        Self {
            port: None,
            host: Some("127.0.0.1".to_string()),
            name: None,
            cors: Some(true),
            default_status: Some(404),
            default_body: Some("{\"error\":\"not_found\",\"message\":\"No mock route matched\"}".to_string()),
            max_history: Some(500),
            routes: Vec::new(),
        }
    }
}

/// Snapshot summary of a mock server.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MockServerInfo {
    pub id: String,
    pub name: Option<String>,
    pub host: String,
    pub port: u16,
    pub url: String,
    pub status: MockServerStatus,
    pub started_at: String,
    pub route_count: usize,
    pub request_count: u64,
    pub routes: Vec<MockRoute>,
}

/// Criteria for filtering recorded requests.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct RequestFilter {
    pub method: Option<String>,
    pub path: Option<String>,
    pub limit: Option<usize>,
    pub since: Option<String>,
    pub matched_route_id: Option<String>,
}

/// Criteria for verifying received requests/webhooks.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VerificationCriteria {
    pub method: Option<String>,
    pub path: Option<String>,
    pub count: Option<usize>,
    pub min_count: Option<usize>,
    pub max_count: Option<usize>,
    pub body_contains: Option<String>,
    pub header_contains: Option<(String, String)>,
}

/// Result of a verification check.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VerificationResult {
    pub matched: bool,
    pub actual_count: usize,
    pub expected_description: String,
    pub matching_requests: Vec<MockRequest>,
    pub message: String,
}

// ===========================================================================
// Response Templating Engine
// ===========================================================================

/// Render dynamic template variables in the response body.
/// Supported tokens:
/// - `{{method}}`: HTTP method
/// - `{{path}}`: Request path
/// - `{{uuid}}`: Random UUID v4
/// - `{{timestamp}}` / `{{now}}`: ISO 8601 timestamp
/// - `{{date}}`: Date (YYYY-MM-DD)
/// - `{{body}}`: Raw request body
/// - `{{query.<name>}}`: Query parameter
/// - `{{header.<name>}}`: Header value
/// - `{{json.<path>}}`: Extracted field from request body JSON
/// - `{{status}}`: Response status code
/// - `{{count}}`: Call count
pub fn render_template(
    template: &str,
    method: &str,
    path: &str,
    query: &HashMap<String, String>,
    headers: &HashMap<String, String>,
    body: Option<&str>,
    body_json: Option<&Value>,
    status: u16,
    call_count: usize,
) -> String {
    if !template.contains("{{") {
        return template.to_string();
    }

    let mut result = template.to_string();
    let now = chrono::Utc::now();

    // Standard tokens
    result = result.replace("{{method}}", method);
    result = result.replace("{{path}}", path);
    result = result.replace("{{status}}", &status.to_string());
    result = result.replace("{{count}}", &call_count.to_string());
    result = result.replace("{{timestamp}}", &now.to_rfc3339());
    result = result.replace("{{now}}", &now.to_rfc3339());
    result = result.replace("{{date}}", &now.format("%Y-%m-%d").to_string());
    result = result.replace("{{time}}", &now.format("%H:%M:%S").to_string());
    result = result.replace("{{body}}", body.unwrap_or(""));

    // UUID replacements (each {{uuid}} gets a fresh UUID)
    while result.contains("{{uuid}}") {
        let u = uuid::Uuid::new_v4().to_string();
        result = result.replacen("{{uuid}}", &u, 1);
    }

    // Query parameters {{query.key}}
    for (k, v) in query {
        let placeholder = format!("{{{{query.{}}}}}", k);
        result = result.replace(&placeholder, v);
    }

    // Headers {{header.key}}
    for (k, v) in headers {
        let placeholder = format!("{{{{header.{}}}}}", k);
        result = result.replace(&placeholder, v);
    }

    // JSON fields {{json.key}}
    if let Some(val) = body_json {
        if let Value::Object(map) = val {
            for (k, v) in map {
                let placeholder = format!("{{{{json.{}}}}}", k);
                let rendered_val = match v {
                    Value::String(s) => s.clone(),
                    other => other.to_string(),
                };
                result = result.replace(&placeholder, &rendered_val);
            }
        }
    }

    result
}

// ===========================================================================
// In-Process HTTP Server Instance
// ===========================================================================

/// Active instance of an in-process mock HTTP server.
pub struct MockServerInstance {
    pub id: String,
    pub name: Option<String>,
    pub host: String,
    pub port: u16,
    pub url: String,
    pub status: Arc<RwLock<MockServerStatus>>,
    pub routes: Arc<RwLock<Vec<MockRoute>>>,
    pub history: Arc<RwLock<VecDeque<MockRequest>>>,
    pub max_history: usize,
    pub cors: bool,
    pub default_status: u16,
    pub default_body: String,
    pub started_at: String,
    pub request_counter: Arc<AtomicU64>,
    shutdown_tx: Option<watch::Sender<bool>>,
}

impl MockServerInstance {
    /// Start a new mock server instance based on the configuration.
    pub async fn start(config: MockServerConfig) -> anyhow::Result<Arc<Self>> {
        let host = config.host.unwrap_or_else(|| "127.0.0.1".to_string());
        let requested_port = config.port.unwrap_or(0);
        let bind_addr = format!("{}:{}", host, requested_port);

        let listener = TcpListener::bind(&bind_addr).await.map_err(|e| {
            anyhow::anyhow!("Failed to bind mock server to address {}: {}", bind_addr, e)
        })?;

        let local_addr = listener.local_addr()?;
        let actual_port = local_addr.port();
        let server_id = format!("srv_{}", uuid::Uuid::new_v4().to_string().replace('-', "")[..12].to_string());
        let server_url = format!("http://{}:{}", host, actual_port);

        let (shutdown_tx, shutdown_rx) = watch::channel(false);

        let status = Arc::new(RwLock::new(MockServerStatus::Running));
        let routes = Arc::new(RwLock::new(config.routes));
        let max_history = config.max_history.unwrap_or(500);
        let history = Arc::new(RwLock::new(VecDeque::with_capacity(max_history)));
        let request_counter = Arc::new(AtomicU64::new(0));
        let started_at = chrono::Utc::now().to_rfc3339();
        let cors = config.cors.unwrap_or(true);
        let default_status = config.default_status.unwrap_or(404);
        let default_body = config.default_body.unwrap_or_else(|| {
            "{\"error\":\"not_found\",\"message\":\"No mock route matched\"}".to_string()
        });

        let server = Arc::new(Self {
            id: server_id.clone(),
            name: config.name,
            host: host.clone(),
            port: actual_port,
            url: server_url,
            status: status.clone(),
            routes: routes.clone(),
            history: history.clone(),
            max_history,
            cors,
            default_status,
            default_body: default_body.clone(),
            started_at,
            request_counter: request_counter.clone(),
            shutdown_tx: Some(shutdown_tx),
        });

        // Spawn background listener task
        let srv_id_clone = server_id.clone();
        let routes_clone = routes.clone();
        let history_clone = history.clone();
        let request_counter_clone = request_counter.clone();
        let status_clone = status.clone();

        tokio::spawn(async move {
            run_listener_loop(
                listener,
                srv_id_clone,
                routes_clone,
                history_clone,
                max_history,
                request_counter_clone,
                cors,
                default_status,
                default_body,
                status_clone,
                shutdown_rx,
            )
            .await;
        });

        Ok(server)
    }

    /// Stop this server instance gracefully.
    pub async fn stop(&self) {
        if let Some(tx) = &self.shutdown_tx {
            let _ = tx.send(true);
        }
        let mut status = self.status.write().await;
        *status = MockServerStatus::Stopped;
    }

    /// Add a route to this server.
    pub async fn add_route(&self, route: MockRoute) -> String {
        let mut routes = self.routes.write().await;
        let id = route.id.clone();
        // Remove existing route with same ID if exists
        routes.retain(|r| r.id != id);
        routes.push(route);
        // Sort by priority descending
        routes.sort_by(|a, b| b.priority.cmp(&a.priority));
        id
    }

    /// Remove a route by ID.
    pub async fn remove_route(&self, route_id: &str) -> bool {
        let mut routes = self.routes.write().await;
        let len_before = routes.len();
        routes.retain(|r| r.id != route_id);
        routes.len() < len_before
    }

    /// Clear all routes.
    pub async fn clear_routes(&self) {
        let mut routes = self.routes.write().await;
        routes.clear();
    }

    /// Retrieve recorded requests matching filter.
    pub async fn get_requests(&self, filter: &RequestFilter) -> Vec<MockRequest> {
        let history = self.history.read().await;
        let mut results = Vec::new();

        for req in history.iter().rev() {
            if let Some(method) = &filter.method {
                if !req.method.eq_ignore_ascii_case(method) {
                    continue;
                }
            }
            if let Some(path) = &filter.path {
                if !match_path_pattern(path, &req.path) {
                    continue;
                }
            }
            if let Some(route_id) = &filter.matched_route_id {
                if req.matched_route_id.as_deref() != Some(route_id) {
                    continue;
                }
            }
            if let Some(since) = &filter.since {
                if req.timestamp < *since {
                    continue;
                }
            }
            results.push(req.clone());
            if let Some(limit) = filter.limit {
                if results.len() >= limit {
                    break;
                }
            }
        }

        results
    }

    /// Clear all recorded request history.
    pub async fn clear_history(&self) -> usize {
        let mut history = self.history.write().await;
        let count = history.len();
        history.clear();
        count
    }

    /// Get summary snapshot.
    pub async fn summary(&self) -> MockServerInfo {
        let status = self.status.read().await.clone();
        let routes = self.routes.read().await.clone();
        let route_count = routes.len();
        let request_count = self.request_counter.load(Ordering::Relaxed);

        MockServerInfo {
            id: self.id.clone(),
            name: self.name.clone(),
            host: self.host.clone(),
            port: self.port,
            url: self.url.clone(),
            status,
            started_at: self.started_at.clone(),
            route_count,
            request_count,
            routes,
        }
    }

    /// Verify webhook / REST expectations against recorded requests.
    pub async fn verify(&self, criteria: &VerificationCriteria) -> VerificationResult {
        let history = self.history.read().await;
        let mut matching = Vec::new();

        for req in history.iter() {
            if let Some(method) = &criteria.method {
                if !req.method.eq_ignore_ascii_case(method) {
                    continue;
                }
            }
            if let Some(path) = &criteria.path {
                if !match_path_pattern(path, &req.path) {
                    continue;
                }
            }
            if let Some(body_substr) = &criteria.body_contains {
                let body_str = req.body.as_deref().unwrap_or("");
                if !body_str.contains(body_substr) {
                    continue;
                }
            }
            if let Some((hk, hv)) = &criteria.header_contains {
                let hk_lower = hk.to_lowercase();
                if req.headers.get(&hk_lower).map(|v| v.as_str()) != Some(hv.as_str()) {
                    continue;
                }
            }
            matching.push(req.clone());
        }

        let actual_count = matching.len();
        let mut matched = true;
        let mut expected_parts = Vec::new();

        if let Some(m) = &criteria.method {
            expected_parts.push(format!("method {}", m));
        }
        if let Some(p) = &criteria.path {
            expected_parts.push(format!("path {}", p));
        }

        if let Some(exact_count) = criteria.count {
            expected_parts.push(format!("called exactly {} time(s)", exact_count));
            if actual_count != exact_count {
                matched = false;
            }
        }
        if let Some(min_count) = criteria.min_count {
            expected_parts.push(format!("called at least {} time(s)", min_count));
            if actual_count < min_count {
                matched = false;
            }
        }
        if let Some(max_count) = criteria.max_count {
            expected_parts.push(format!("called at most {} time(s)", max_count));
            if actual_count > max_count {
                matched = false;
            }
        }
        // Default requirement if no counts specified: called at least once
        if criteria.count.is_none() && criteria.min_count.is_none() && criteria.max_count.is_none() {
            expected_parts.push("called at least 1 time(s)".to_string());
            if actual_count == 0 {
                matched = false;
            }
        }

        let desc = expected_parts.join(", ");
        let message = if matched {
            format!("Verification passed: Expected [{}] matched {} request(s).", desc, actual_count)
        } else {
            format!("Verification failed: Expected [{}] but recorded {} matching request(s).", desc, actual_count)
        };

        VerificationResult {
            matched,
            actual_count,
            expected_description: desc,
            matching_requests: matching,
            message,
        }
    }

    /// Reset routes call counters and history.
    pub async fn reset(&self) {
        self.clear_history().await;
        let mut routes = self.routes.write().await;
        for r in routes.iter_mut() {
            r.call_count = 0;
        }
    }
}

// ===========================================================================
// Server Background Loop & Raw HTTP Parser
// ===========================================================================

async fn run_listener_loop(
    listener: TcpListener,
    server_id: String,
    routes: Arc<RwLock<Vec<MockRoute>>>,
    history: Arc<RwLock<VecDeque<MockRequest>>>,
    max_history: usize,
    request_counter: Arc<AtomicU64>,
    cors: bool,
    default_status: u16,
    default_body: String,
    status: Arc<RwLock<MockServerStatus>>,
    mut shutdown_rx: watch::Receiver<bool>,
) {
    loop {
        tokio::select! {
            _ = shutdown_rx.changed() => {
                if *shutdown_rx.borrow() {
                    break;
                }
            }
            accept_res = listener.accept() => {
                match accept_res {
                    Ok((stream, addr)) => {
                        let server_id = server_id.clone();
                        let routes = routes.clone();
                        let history = history.clone();
                        let request_counter = request_counter.clone();
                        let default_body = default_body.clone();

                        tokio::spawn(async move {
                            handle_connection(
                                stream,
                                addr,
                                server_id,
                                routes,
                                history,
                                max_history,
                                request_counter,
                                cors,
                                default_status,
                                default_body,
                            ).await;
                        });
                    }
                    Err(e) => {
                        // If listener closed or error
                        tracing::debug!("Mock server accept error: {}", e);
                        tokio::time::sleep(Duration::from_millis(10)).await;
                    }
                }
            }
        }
    }

    let mut st = status.write().await;
    *st = MockServerStatus::Stopped;
}

/// Handle a single incoming TCP connection and process HTTP request/response.
async fn handle_connection(
    mut stream: TcpStream,
    addr: SocketAddr,
    server_id: String,
    routes: Arc<RwLock<Vec<MockRoute>>>,
    history: Arc<RwLock<VecDeque<MockRequest>>>,
    max_history: usize,
    request_counter: Arc<AtomicU64>,
    cors: bool,
    default_status: u16,
    default_body: String,
) {
    let mut buffer = vec![0u8; 8192];
    let mut total_read = 0;

    // Read initial HTTP request header
    let (header_end, headers_bytes) = loop {
        match stream.read(&mut buffer[total_read..]).await {
            Ok(0) => return, // Client disconnected
            Ok(n) => {
                total_read += n;
                if let Some(pos) = find_subsequence(&buffer[..total_read], b"\r\n\r\n") {
                    break (pos + 4, &buffer[..pos]);
                }
                if let Some(pos) = find_subsequence(&buffer[..total_read], b"\n\n") {
                    break (pos + 2, &buffer[..pos]);
                }
                if total_read >= buffer.len() {
                    if buffer.len() > 1024 * 1024 {
                        // Header too large
                        return;
                    }
                    buffer.resize(buffer.len() * 2, 0);
                }
            }
            Err(_) => return,
        }
    };

    let header_str = match std::str::from_utf8(headers_bytes) {
        Ok(s) => s,
        Err(_) => return,
    };

    let mut lines = header_str.lines();
    let request_line = match lines.next() {
        Some(l) => l,
        None => return,
    };

    let parts: Vec<&str> = request_line.split_whitespace().collect();
    if parts.len() < 2 {
        return;
    }

    let method = parts[0].to_uppercase();
    let raw_uri = parts[1];

    // Parse path and query
    let (raw_path, raw_query) = match raw_uri.find('?') {
        Some(idx) => (&raw_uri[..idx], Some(&raw_uri[idx + 1..])),
        None => (raw_uri, None),
    };

    let path = url_decode(raw_path);
    let query_map = raw_query.map(parse_query_string).unwrap_or_default();

    // Parse headers
    let mut headers = HashMap::new();
    let mut content_length: Option<usize> = None;

    for line in lines {
        if let Some((k, v)) = line.split_once(':') {
            let key = k.trim().to_lowercase();
            let val = v.trim().to_string();
            if key == "content-length" {
                content_length = val.parse::<usize>().ok();
            }
            headers.insert(key, val);
        }
    }

    // Read body if content-length specified
    let mut body_bytes = Vec::new();
    let already_read_body = &buffer[header_end..total_read];
    body_bytes.extend_from_slice(already_read_body);

    if let Some(cl) = content_length {
        let max_body_size = 10 * 1024 * 1024; // 10MB safety limit
        let target_len = cl.min(max_body_size);
        while body_bytes.len() < target_len {
            let mut chunk = vec![0u8; (target_len - body_bytes.len()).min(16384)];
            match stream.read(&mut chunk).await {
                Ok(0) => break,
                Ok(n) => body_bytes.extend_from_slice(&chunk[..n]),
                Err(_) => break,
            }
        }
    }

    let body_str = if !body_bytes.is_empty() {
        String::from_utf8(body_bytes).ok()
    } else {
        None
    };

    let body_json = body_str.as_ref().and_then(|s| serde_json::from_str::<Value>(s).ok());

    // CORS preflight handling
    if cors && method == "OPTIONS" {
        let resp = "HTTP/1.1 204 No Content\r\n\
                    Access-Control-Allow-Origin: *\r\n\
                    Access-Control-Allow-Methods: GET, POST, PUT, DELETE, PATCH, OPTIONS, HEAD\r\n\
                    Access-Control-Allow-Headers: *\r\n\
                    Access-Control-Max-Age: 86400\r\n\
                    Content-Length: 0\r\n\
                    Connection: close\r\n\r\n";
        let _ = stream.write_all(resp.as_bytes()).await;
        let _ = stream.flush().await;
        return;
    }

    // Match route
    let mut matched_route_id = None;
    let mut resp_status = default_status;
    let mut resp_status_text = "OK".to_string();
    let mut resp_headers = HashMap::new();
    let mut resp_body = default_body;
    let mut delay_ms = None;

    {
        let mut routes_guard = routes.write().await;
        for route in routes_guard.iter_mut() {
            if route.matches(&method, &path, &query_map, &headers, body_str.as_deref()) {
                matched_route_id = Some(route.id.clone());
                route.call_count += 1;
                resp_status = route.status;
                resp_status_text = route.status_text.clone().unwrap_or_else(|| default_status_text(route.status).to_string());
                resp_headers = route.response_headers.clone();
                delay_ms = route.delay_ms;

                resp_body = render_template(
                    &route.response_body,
                    &method,
                    &path,
                    &query_map,
                    &headers,
                    body_str.as_deref(),
                    body_json.as_ref(),
                    resp_status,
                    route.call_count,
                );
                break;
            }
        }
    }

    // Record request in history
    let req_id = uuid::Uuid::new_v4().to_string();
    let mock_req = MockRequest {
        id: req_id,
        server_id: server_id.clone(),
        timestamp: chrono::Utc::now().to_rfc3339(),
        method: method.clone(),
        path: path.clone(),
        query: raw_query.map(|s| s.to_string()),
        query_params: query_map,
        headers: headers.clone(),
        body: body_str,
        body_json,
        client_ip: addr.to_string(),
        matched_route_id,
        response_status: resp_status,
    };

    request_counter.fetch_add(1, Ordering::Relaxed);

    {
        let mut hist = history.write().await;
        if hist.len() >= max_history {
            hist.pop_front();
        }
        hist.push_back(mock_req);
    }

    // Apply delay if configured
    if let Some(delay) = delay_ms {
        if delay > 0 {
            tokio::time::sleep(Duration::from_millis(delay)).await;
        }
    }

    // Default Content-Type to application/json if not explicitly specified and looks like json
    if !resp_headers.keys().any(|k| k.eq_ignore_ascii_case("content-type")) {
        let trimmed = resp_body.trim();
        if (trimmed.starts_with('{') && trimmed.ends_with('}')) || (trimmed.starts_with('[') && trimmed.ends_with(']')) {
            resp_headers.insert("Content-Type".to_string(), "application/json; charset=utf-8".to_string());
        } else {
            resp_headers.insert("Content-Type".to_string(), "text/plain; charset=utf-8".to_string());
        }
    }

    // Add CORS headers if enabled
    if cors && !resp_headers.keys().any(|k| k.eq_ignore_ascii_case("access-control-allow-origin")) {
        resp_headers.insert("Access-Control-Allow-Origin".to_string(), "*".to_string());
    }

    let body_bytes = resp_body.as_bytes();
    let mut response_raw = format!(
        "HTTP/1.1 {} {}\r\nServer: Fusion-MockServer/0.3\r\nContent-Length: {}\r\nConnection: close\r\n",
        resp_status, resp_status_text, body_bytes.len()
    );

    for (k, v) in &resp_headers {
        response_raw.push_str(&format!("{}: {}\r\n", k, v));
    }
    response_raw.push_str("\r\n");

    let _ = stream.write_all(response_raw.as_bytes()).await;
    let _ = stream.write_all(body_bytes).await;
    let _ = stream.flush().await;
}

fn find_subsequence(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack.windows(needle.len()).position(|window| window == needle)
}

fn default_status_text(status: u16) -> &'static str {
    match status {
        200 => "OK",
        201 => "Created",
        202 => "Accepted",
        204 => "No Content",
        301 => "Moved Permanently",
        302 => "Found",
        304 => "Not Modified",
        400 => "Bad Request",
        401 => "Unauthorized",
        403 => "Forbidden",
        404 => "Not Found",
        405 => "Method Not Allowed",
        409 => "Conflict",
        422 => "Unprocessable Entity",
        429 => "Too Many Requests",
        500 => "Internal Server Error",
        502 => "Bad Gateway",
        503 => "Service Unavailable",
        504 => "Gateway Timeout",
        _ => "Status",
    }
}

fn url_decode(s: &str) -> String {
    let mut result = String::with_capacity(s.len());
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let Ok(val) = u8::from_str_radix(std::str::from_utf8(&bytes[i + 1..i + 2 + 1]).unwrap_or(""), 16) {
                result.push(val as char);
                i += 3;
                continue;
            }
        }
        if bytes[i] == b'+' {
            result.push(' ');
        } else {
            result.push(bytes[i] as char);
        }
        i += 1;
    }
    result
}

fn parse_query_string(qs: &str) -> HashMap<String, String> {
    let mut map = HashMap::new();
    for pair in qs.split('&') {
        if pair.is_empty() {
            continue;
        }
        if let Some((k, v)) = pair.split_once('=') {
            map.insert(url_decode(k), url_decode(v));
        } else {
            map.insert(url_decode(pair), String::new());
        }
    }
    map
}

// ===========================================================================
// Global Mock Server Manager
// ===========================================================================

/// Global manager for multiple concurrent mock servers.
pub struct MockServerManager {
    servers: Arc<RwLock<HashMap<String, Arc<MockServerInstance>>>>,
}

impl Default for MockServerManager {
    fn default() -> Self {
        Self::new()
    }
}

impl MockServerManager {
    /// Create a new mock server manager.
    pub fn new() -> Self {
        Self {
            servers: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Start a new mock server.
    pub async fn start_server(&self, config: MockServerConfig) -> anyhow::Result<MockServerInfo> {
        let instance = MockServerInstance::start(config).await?;
        let summary = instance.summary().await;
        let mut servers = self.servers.write().await;
        servers.insert(instance.id.clone(), instance);
        Ok(summary)
    }

    /// Stop a mock server by ID.
    pub async fn stop_server(&self, id: &str) -> anyhow::Result<()> {
        let mut servers = self.servers.write().await;
        if let Some(srv) = servers.remove(id) {
            srv.stop().await;
            Ok(())
        } else {
            anyhow::bail!("Mock server '{}' not found", id);
        }
    }

    /// Stop all running mock servers.
    pub async fn stop_all(&self) -> usize {
        let mut servers = self.servers.write().await;
        let count = servers.len();
        for (_, srv) in servers.drain() {
            srv.stop().await;
        }
        count
    }

    /// Get a mock server instance by ID or name.
    pub async fn get_server(&self, id_or_name: &str) -> Option<Arc<MockServerInstance>> {
        let servers = self.servers.read().await;
        if let Some(srv) = servers.get(id_or_name) {
            return Some(srv.clone());
        }
        for srv in servers.values() {
            if srv.name.as_deref() == Some(id_or_name) {
                return Some(srv.clone());
            }
        }
        None
    }

    /// List all registered mock servers.
    pub async fn list_servers(&self) -> Vec<MockServerInfo> {
        let servers = self.servers.read().await;
        let mut results = Vec::new();
        for srv in servers.values() {
            results.push(srv.summary().await);
        }
        results.sort_by(|a, b| a.started_at.cmp(&b.started_at));
        results
    }
}

/// Global lazy singleton mock server manager.
pub static GLOBAL_MOCK_SERVER_MANAGER: LazyLock<MockServerManager> =
    LazyLock::new(MockServerManager::new);

/// Get a reference to the global mock server manager.
pub fn global_mock_server_manager() -> &'static MockServerManager {
    &GLOBAL_MOCK_SERVER_MANAGER
}

// ===========================================================================
// Tool Implementation (MockServerTool)
// ===========================================================================

/// Tool for managing local in-process HTTP mock servers during testing and development.
#[derive(Default, Debug, Clone)]
pub struct MockServerTool;

impl MockServerTool {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl Tool for MockServerTool {
    fn name(&self) -> &str {
        "mock_server"
    }

    fn description(&self) -> &str {
        "Local in-process mock HTTP server tool for testing REST endpoints, webhooks, callbacks, and microservice APIs. \
         Allows creating mock servers, defining flexible routes, simulating latency/errors, capturing requests, and verifying webhook payloads."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "action": {
                    "type": "string",
                    "description": "Action to perform: 'start', 'stop', 'list', 'add_route', 'remove_route', 'clear_routes', 'get_requests', 'get_last_request', 'clear_requests', 'verify', 'reset', or 'test_request'.",
                    "enum": [
                        "start",
                        "stop",
                        "list",
                        "add_route",
                        "remove_route",
                        "clear_routes",
                        "get_requests",
                        "get_last_request",
                        "clear_requests",
                        "verify",
                        "reset",
                        "test_request"
                    ]
                },
                "server_id": {
                    "type": "string",
                    "description": "ID or name of the target mock server instance."
                },
                "port": {
                    "type": "integer",
                    "description": "Port to bind to when starting server (0 or omit for automatic ephemeral port)."
                },
                "host": {
                    "type": "string",
                    "description": "Host interface to bind to (default: '127.0.0.1')."
                },
                "name": {
                    "type": "string",
                    "description": "Optional human-readable name for the server."
                },
                "cors": {
                    "type": "boolean",
                    "description": "Enable automatic CORS headers and OPTIONS preflight response (default: true)."
                },
                "default_status": {
                    "type": "integer",
                    "description": "Default HTTP status code when no route matches (default: 404)."
                },
                "default_body": {
                    "type": "string",
                    "description": "Default response body when no route matches."
                },
                "routes": {
                    "type": "array",
                    "description": "List of initial mock routes when starting the server.",
                    "items": {
                        "type": "object",
                        "properties": {
                            "method": { "type": "string", "description": "HTTP method (GET, POST, PUT, DELETE, PATCH, ANY, etc.)" },
                            "path": { "type": "string", "description": "Path pattern (/api/v1/users, /webhooks/*, /users/:id, regex:^/items/\\d+$)" },
                            "status": { "type": "integer", "description": "Response status code (default: 200)" },
                            "status_text": { "type": "string", "description": "Response status text (e.g. 'OK')" },
                            "response_body": { "type": "string", "description": "Response body template" },
                            "response_headers": { "type": "object", "description": "Custom response headers" },
                            "delay_ms": { "type": "integer", "description": "Simulated latency in ms" },
                            "call_limit": { "type": "integer", "description": "Max number of times this route will match" },
                            "priority": { "type": "integer", "description": "Matching priority (default 0)" }
                        },
                        "required": ["path"]
                    }
                },
                "route": {
                    "type": "object",
                    "description": "Single route configuration for 'add_route' action.",
                    "properties": {
                        "method": { "type": "string" },
                        "path": { "type": "string" },
                        "status": { "type": "integer" },
                        "status_text": { "type": "string" },
                        "response_body": { "type": "string" },
                        "response_headers": { "type": "object" },
                        "delay_ms": { "type": "integer" },
                        "call_limit": { "type": "integer" },
                        "priority": { "type": "integer" }
                    },
                    "required": ["path"]
                },
                "route_id": {
                    "type": "string",
                    "description": "ID of route to remove for 'remove_route' action."
                },
                "method": {
                    "type": "string",
                    "description": "HTTP method filter for 'get_requests' or 'verify' action."
                },
                "path": {
                    "type": "string",
                    "description": "Path pattern filter for 'get_requests' or 'verify' action."
                },
                "limit": {
                    "type": "integer",
                    "description": "Maximum number of requests to retrieve for 'get_requests'."
                },
                "count": {
                    "type": "integer",
                    "description": "Expected exact request count for 'verify' action."
                },
                "min_count": {
                    "type": "integer",
                    "description": "Expected minimum request count for 'verify' action."
                },
                "max_count": {
                    "type": "integer",
                    "description": "Expected maximum request count for 'verify' action."
                },
                "body_contains": {
                    "type": "string",
                    "description": "Substring required in request body for 'verify' action."
                },
                "url": {
                    "type": "string",
                    "description": "Full URL or path for 'test_request' action."
                },
                "request_body": {
                    "type": "string",
                    "description": "Payload body for 'test_request' action."
                },
                "request_headers": {
                    "type": "object",
                    "description": "Headers for 'test_request' action."
                }
            },
            "required": ["action"]
        })
    }

    async fn execute(&self, args: Value, _ctx: &ToolContext) -> anyhow::Result<String> {
        let action = args
            .get("action")
            .and_then(|v| v.as_str())
            .unwrap_or("list")
            .to_lowercase();

        let manager = global_mock_server_manager();

        match action.as_str() {
            "start" => {
                let config: MockServerConfig = serde_json::from_value(args.clone()).unwrap_or_default();
                let info = manager.start_server(config).await?;
                Ok(json!({
                    "status": "success",
                    "message": format!("Mock HTTP server started on {}", info.url),
                    "server": info
                }).to_string())
            }

            "stop" => {
                let target = args
                    .get("server_id")
                    .and_then(|v| v.as_str())
                    .or_else(|| args.get("id").and_then(|v| v.as_str()));

                if let Some(id) = target {
                    if id.eq_ignore_ascii_case("all") {
                        let stopped = manager.stop_all().await;
                        Ok(json!({
                            "status": "success",
                            "message": format!("Stopped {} mock server(s)", stopped),
                            "count": stopped
                        }).to_string())
                    } else {
                        manager.stop_server(id).await?;
                        Ok(json!({
                            "status": "success",
                            "message": format!("Stopped mock server '{}'", id),
                            "server_id": id
                        }).to_string())
                    }
                } else {
                    anyhow::bail!("Missing 'server_id' argument for 'stop' action");
                }
            }

            "list" | "status" => {
                let servers = manager.list_servers().await;
                Ok(json!({
                    "status": "success",
                    "count": servers.len(),
                    "servers": servers
                }).to_string())
            }

            "add_route" | "mock" => {
                let server_id = args
                    .get("server_id")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| anyhow::anyhow!("Missing 'server_id' argument"))?;

                let server = manager
                    .get_server(server_id)
                    .await
                    .ok_or_else(|| anyhow::anyhow!("Mock server '{}' not found", server_id))?;

                let route_val = args.get("route").cloned().unwrap_or_else(|| args.clone());
                let route: MockRoute = serde_json::from_value(route_val)
                    .map_err(|e| anyhow::anyhow!("Invalid route definition: {}", e))?;

                let route_id = server.add_route(route.clone()).await;
                Ok(json!({
                    "status": "success",
                    "message": format!("Added route [{}] {} to server '{}'", route.method, route.path, server_id),
                    "route_id": route_id,
                    "server_id": server_id
                }).to_string())
            }

            "remove_route" | "delete_route" => {
                let server_id = args
                    .get("server_id")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| anyhow::anyhow!("Missing 'server_id' argument"))?;

                let route_id = args
                    .get("route_id")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| anyhow::anyhow!("Missing 'route_id' argument"))?;

                let server = manager
                    .get_server(server_id)
                    .await
                    .ok_or_else(|| anyhow::anyhow!("Mock server '{}' not found", server_id))?;

                let removed = server.remove_route(route_id).await;
                Ok(json!({
                    "status": if removed { "success" } else { "not_found" },
                    "removed": removed,
                    "route_id": route_id,
                    "server_id": server_id
                }).to_string())
            }

            "clear_routes" => {
                let server_id = args
                    .get("server_id")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| anyhow::anyhow!("Missing 'server_id' argument"))?;

                let server = manager
                    .get_server(server_id)
                    .await
                    .ok_or_else(|| anyhow::anyhow!("Mock server '{}' not found", server_id))?;

                server.clear_routes().await;
                Ok(json!({
                    "status": "success",
                    "message": format!("Cleared all routes from server '{}'", server_id),
                    "server_id": server_id
                }).to_string())
            }

            "get_requests" | "requests" | "history" => {
                let server_id = args
                    .get("server_id")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| anyhow::anyhow!("Missing 'server_id' argument"))?;

                let server = manager
                    .get_server(server_id)
                    .await
                    .ok_or_else(|| anyhow::anyhow!("Mock server '{}' not found", server_id))?;

                let filter = RequestFilter {
                    method: args.get("method").and_then(|v| v.as_str()).map(ToString::to_string),
                    path: args.get("path").and_then(|v| v.as_str()).map(ToString::to_string),
                    limit: args.get("limit").and_then(|v| v.as_u64()).map(|n| n as usize),
                    since: args.get("since").and_then(|v| v.as_str()).map(ToString::to_string),
                    matched_route_id: args.get("matched_route_id").and_then(|v| v.as_str()).map(ToString::to_string),
                };

                let requests = server.get_requests(&filter).await;
                Ok(json!({
                    "status": "success",
                    "server_id": server_id,
                    "count": requests.len(),
                    "requests": requests
                }).to_string())
            }

            "get_last_request" | "last_request" => {
                let server_id = args
                    .get("server_id")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| anyhow::anyhow!("Missing 'server_id' argument"))?;

                let server = manager
                    .get_server(server_id)
                    .await
                    .ok_or_else(|| anyhow::anyhow!("Mock server '{}' not found", server_id))?;

                let filter = RequestFilter {
                    method: args.get("method").and_then(|v| v.as_str()).map(ToString::to_string),
                    path: args.get("path").and_then(|v| v.as_str()).map(ToString::to_string),
                    limit: Some(1),
                    since: None,
                    matched_route_id: None,
                };

                let requests = server.get_requests(&filter).await;
                let last = requests.into_iter().next();

                Ok(json!({
                    "status": "success",
                    "server_id": server_id,
                    "has_request": last.is_some(),
                    "request": last
                }).to_string())
            }

            "clear_requests" | "clear_history" => {
                let server_id = args
                    .get("server_id")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| anyhow::anyhow!("Missing 'server_id' argument"))?;

                let server = manager
                    .get_server(server_id)
                    .await
                    .ok_or_else(|| anyhow::anyhow!("Mock server '{}' not found", server_id))?;

                let cleared = server.clear_history().await;
                Ok(json!({
                    "status": "success",
                    "message": format!("Cleared {} recorded request(s)", cleared),
                    "cleared_count": cleared,
                    "server_id": server_id
                }).to_string())
            }

            "verify" => {
                let server_id = args
                    .get("server_id")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| anyhow::anyhow!("Missing 'server_id' argument"))?;

                let server = manager
                    .get_server(server_id)
                    .await
                    .ok_or_else(|| anyhow::anyhow!("Mock server '{}' not found", server_id))?;

                let criteria = VerificationCriteria {
                    method: args.get("method").and_then(|v| v.as_str()).map(ToString::to_string),
                    path: args.get("path").and_then(|v| v.as_str()).map(ToString::to_string),
                    count: args.get("count").and_then(|v| v.as_u64()).map(|n| n as usize),
                    min_count: args.get("min_count").and_then(|v| v.as_u64()).map(|n| n as usize),
                    max_count: args.get("max_count").and_then(|v| v.as_u64()).map(|n| n as usize),
                    body_contains: args.get("body_contains").and_then(|v| v.as_str()).map(ToString::to_string),
                    header_contains: args.get("header_key").and_then(|k| k.as_str()).and_then(|k| {
                        args.get("header_value").and_then(|v| v.as_str()).map(|v| (k.to_string(), v.to_string()))
                    }),
                };

                let result = server.verify(&criteria).await;
                Ok(json!({
                    "status": if result.matched { "passed" } else { "failed" },
                    "matched": result.matched,
                    "message": result.message,
                    "actual_count": result.actual_count,
                    "expected": result.expected_description,
                    "matching_requests": result.matching_requests
                }).to_string())
            }

            "reset" => {
                let server_id = args
                    .get("server_id")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| anyhow::anyhow!("Missing 'server_id' argument"))?;

                let server = manager
                    .get_server(server_id)
                    .await
                    .ok_or_else(|| anyhow::anyhow!("Mock server '{}' not found", server_id))?;

                server.reset().await;
                Ok(json!({
                    "status": "success",
                    "message": format!("Reset mock server '{}'", server_id),
                    "server_id": server_id
                }).to_string())
            }

            "test_request" | "send" => {
                let url_arg = args
                    .get("url")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| anyhow::anyhow!("Missing 'url' argument for test_request"))?;

                let target_url = if !url_arg.starts_with("http://") && !url_arg.starts_with("https://") {
                    // Prepend server url if server_id supplied
                    if let Some(srv_id) = args.get("server_id").and_then(|v| v.as_str()) {
                        let server = manager
                            .get_server(srv_id)
                            .await
                            .ok_or_else(|| anyhow::anyhow!("Mock server '{}' not found", srv_id))?;
                        let p = if url_arg.starts_with('/') { url_arg } else { &format!("/{}", url_arg) };
                        format!("{}{}", server.url, p)
                    } else {
                        anyhow::bail!("Relative URL provided but no 'server_id' specified: {}", url_arg);
                    }
                } else {
                    url_arg.to_string()
                };

                let method_str = args.get("method").and_then(|v| v.as_str()).unwrap_or("GET").to_uppercase();
                let body_opt = args.get("request_body").and_then(|v| v.as_str())
                    .or_else(|| args.get("body").and_then(|v| v.as_str()));

                let client = reqwest::Client::builder()
                    .timeout(Duration::from_secs(10))
                    .build()?;

                let mut req_builder = match method_str.as_str() {
                    "POST" => client.post(&target_url),
                    "PUT" => client.put(&target_url),
                    "DELETE" => client.delete(&target_url),
                    "PATCH" => client.patch(&target_url),
                    "HEAD" => client.head(&target_url),
                    _ => client.get(&target_url),
                };

                if let Some(headers_obj) = args.get("request_headers").and_then(|v| v.as_object()) {
                    for (k, v) in headers_obj {
                        if let Some(s) = v.as_str() {
                            req_builder = req_builder.header(k, s);
                        }
                    }
                }

                if let Some(b) = body_opt {
                    req_builder = req_builder.body(b.to_string());
                }

                let resp = req_builder.send().await?;
                let status = resp.status().as_u16();
                let status_text = resp.status().canonical_reason().unwrap_or("").to_string();
                let headers_map: HashMap<String, String> = resp
                    .headers()
                    .iter()
                    .map(|(k, v)| (k.as_str().to_string(), v.to_str().unwrap_or("").to_string()))
                    .collect();
                let resp_text = resp.text().await.unwrap_or_default();

                let json_val = serde_json::from_str::<Value>(&resp_text).ok();

                Ok(json!({
                    "status": "success",
                    "url": target_url,
                    "response": {
                        "status": status,
                        "status_text": status_text,
                        "headers": headers_map,
                        "body": resp_text,
                        "body_json": json_val
                    }
                }).to_string())
            }

            other => {
                anyhow::bail!("Unknown action '{}'. Valid actions: 'start', 'stop', 'list', 'add_route', 'remove_route', 'clear_routes', 'get_requests', 'get_last_request', 'clear_requests', 'verify', 'reset', 'test_request'", other);
            }
        }
    }
}

// ===========================================================================
// Unit and Integration Tests
// ===========================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_mock_server_start_stop_and_routes() {
        let manager = MockServerManager::new();

        // 1. Start server with an initial route
        let config = MockServerConfig {
            port: Some(0), // dynamic
            name: Some("test_auth_server".to_string()),
            routes: vec![
                MockRoute::get("/health", "{\"status\":\"healthy\"}"),
                MockRoute::post("/api/v1/auth", 200, "{\"token\":\"tok_12345\"}"),
            ],
            ..Default::default()
        };

        let server_info = manager.start_server(config).await.expect("start server");
        assert!(server_info.port > 0);
        assert!(server_info.url.starts_with("http://127.0.0.1:"));
        assert_eq!(server_info.route_count, 2);

        // 2. Fetch /health
        let client = reqwest::Client::new();
        let health_url = format!("{}/health", server_info.url);
        let resp = client.get(&health_url).send().await.expect("send GET /health");
        assert_eq!(resp.status().as_u16(), 200);
        let health_json: Value = resp.json().await.expect("parse json");
        assert_eq!(health_json["status"], "healthy");

        // 3. Post to /api/v1/auth
        let auth_url = format!("{}/api/v1/auth", server_info.url);
        let auth_resp = client
            .post(&auth_url)
            .header("Content-Type", "application/json")
            .body("{\"username\":\"admin\",\"password\":\"secret\"}")
            .send()
            .await
            .expect("send POST /api/v1/auth");
        assert_eq!(auth_resp.status().as_u16(), 200);
        let auth_json: Value = auth_resp.json().await.expect("parse auth json");
        assert_eq!(auth_json["token"], "tok_12345");

        // 4. Verify recorded requests
        let server = manager.get_server(&server_info.id).await.expect("get server");
        let requests = server.get_requests(&RequestFilter::default()).await;
        assert_eq!(requests.len(), 2);

        // Check auth request details
        let auth_req = requests.iter().find(|r| r.path == "/api/v1/auth").expect("find auth req");
        assert_eq!(auth_req.method, "POST");
        assert!(auth_req.body.as_ref().unwrap().contains("admin"));
        assert_eq!(auth_req.body_json.as_ref().unwrap()["username"], "admin");

        // 5. Add dynamic route with template
        let dynamic_route = MockRoute {
            id: generate_uuid(),
            name: Some("echo".to_string()),
            method: "POST".to_string(),
            path: "/echo/:service".to_string(),
            query_params: None,
            headers: None,
            body_match: None,
            status: 201,
            status_text: Some("Created".to_string()),
            response_headers: HashMap::new(),
            response_body: "{\"service\":\"{{path}}\",\"method\":\"{{method}}\",\"echo\":{{body}},\"uuid\":\"{{uuid}}\"}".to_string(),
            delay_ms: None,
            call_limit: None,
            call_count: 0,
            priority: 10,
        };

        server.add_route(dynamic_route).await;

        let echo_url = format!("{}/echo/payment", server_info.url);
        let echo_resp = client
            .post(&echo_url)
            .header("Content-Type", "application/json")
            .body("{\"amount\":100}")
            .send()
            .await
            .expect("send POST /echo/payment");
        assert_eq!(echo_resp.status().as_u16(), 201);
        let echo_json: Value = echo_resp.json().await.expect("parse echo json");
        assert_eq!(echo_json["service"], "/echo/payment");
        assert_eq!(echo_json["method"], "POST");
        assert_eq!(echo_json["echo"]["amount"], 100);
        assert!(!echo_json["uuid"].as_str().unwrap().is_empty());

        // 6. Verify assertions
        let verify_result = server
            .verify(&VerificationCriteria {
                method: Some("POST".to_string()),
                path: Some("/echo/*".to_string()),
                count: Some(1),
                min_count: None,
                max_count: None,
                body_contains: Some("100".to_string()),
                header_contains: None,
            })
            .await;
        assert!(verify_result.matched, "Verification message: {}", verify_result.message);

        // 7. Stop server
        manager.stop_server(&server_info.id).await.expect("stop server");
    }

    #[tokio::test]
    async fn test_mock_server_tool_actions() {
        let tool = MockServerTool::new();
        let ctx = ToolContext::default();

        // 1. Start server action
        let start_res = tool
            .execute(
                json!({
                    "action": "start",
                    "name": "webhook_receiver",
                    "routes": [
                        {
                            "method": "POST",
                            "path": "/webhook/stripe",
                            "status": 200,
                            "response_body": "{\"received\": true}"
                        }
                    ]
                }),
                &ctx,
            )
            .await
            .expect("execute start");

        let start_json: Value = serde_json::from_str(&start_res).unwrap();
        let server_id = start_json["server"]["id"].as_str().unwrap();
        let server_url = start_json["server"]["url"].as_str().unwrap();
        assert!(server_url.starts_with("http://127.0.0.1:"));
        // 2. Test request action sending POST to webhook
        let test_res = tool
            .execute(
                json!({
                    "action": "test_request",
                    "server_id": server_id,
                    "url": "/webhook/stripe",
                    "method": "POST",
                    "request_headers": {
                        "X-Stripe-Signature": "sig_abc123"
                    },
                    "request_body": "{\"event\": \"payment_intent.succeeded\", \"amount\": 5000}"
                }),
                &ctx,
            )
            .await
            .expect("execute test_request");

        let test_json: Value = serde_json::from_str(&test_res).unwrap();
        assert_eq!(test_json["response"]["status"], 200);
        assert_eq!(test_json["response"]["body_json"]["received"], true);

        // 3. Get last request action
        let last_res = tool
            .execute(
                json!({
                    "action": "get_last_request",
                    "server_id": server_id
                }),
                &ctx,
            )
            .await
            .expect("execute get_last_request");

        let last_json: Value = serde_json::from_str(&last_res).unwrap();
        assert_eq!(last_json["has_request"], true);
        let req = &last_json["request"];
        assert_eq!(req["path"], "/webhook/stripe");
        assert_eq!(req["method"], "POST");
        assert_eq!(req["headers"]["x-stripe-signature"], "sig_abc123");

        // 4. Verify action
        let verify_res = tool
            .execute(
                json!({
                    "action": "verify",
                    "server_id": server_id,
                    "method": "POST",
                    "path": "/webhook/stripe",
                    "count": 1,
                    "body_contains": "payment_intent.succeeded"
                }),
                &ctx,
            )
            .await
            .expect("execute verify");

        let verify_json: Value = serde_json::from_str(&verify_res).unwrap();
        assert_eq!(verify_json["matched"], true);

        // 5. Stop server action
        let stop_res = tool
            .execute(
                json!({
                    "action": "stop",
                    "server_id": server_id
                }),
                &ctx,
            )
            .await
            .expect("execute stop");

        let stop_json: Value = serde_json::from_str(&stop_res).unwrap();
        assert_eq!(stop_json["status"], "success");
    }

    #[test]
    fn test_path_matching_patterns() {
        assert!(match_path_pattern("/api/users", "/api/users"));
        assert!(!match_path_pattern("/api/users", "/api/posts"));

        // Wildcard
        assert!(match_path_pattern("/api/*", "/api/users"));
        assert!(match_path_pattern("/api/*", "/api/v1/users"));
        assert!(match_path_pattern("/api/**", "/api/v1/nested/path"));
        assert!(match_path_pattern("*/webhook", "/github/webhook"));

        // Parametric
        assert!(match_path_pattern("/users/:id", "/users/12345"));
        assert!(match_path_pattern("/users/:id/posts/:post_id", "/users/42/posts/99"));
        assert!(!match_path_pattern("/users/:id/posts/:post_id", "/users/42/comments/99"));

        // Regex
        assert!(match_path_pattern("regex:^/items/\\d+$", "/items/12345"));
        assert!(!match_path_pattern("regex:^/items/\\d+$", "/items/abc"));
    }

    #[test]
    fn test_template_rendering() {
        let mut query = HashMap::new();
        query.insert("page".to_string(), "2".to_string());

        let mut headers = HashMap::new();
        headers.insert("authorization".to_string(), "Bearer token123".to_string());

        let body_json = json!({
            "user": "alice",
            "role": "admin"
        });

        let template = "{\"method\":\"{{method}}\",\"path\":\"{{path}}\",\"page\":{{query.page}},\"user\":\"{{json.user}}\",\"status\":{{status}}}";
        let rendered = render_template(
            template,
            "POST",
            "/api/users",
            &query,
            &headers,
            Some("{\"user\":\"alice\",\"role\":\"admin\"}"),
            Some(&body_json),
            201,
            1,
        );

        let parsed: Value = serde_json::from_str(&rendered).expect("parse rendered template");
        assert_eq!(parsed["method"], "POST");
        assert_eq!(parsed["path"], "/api/users");
        assert_eq!(parsed["page"], 2);
        assert_eq!(parsed["user"], "alice");
        assert_eq!(parsed["status"], 201);
    }

    #[tokio::test]
    async fn test_mock_server_query_and_headers_matching() {
        let manager = MockServerManager::new();

        let mut req_query = HashMap::new();
        req_query.insert("filter".to_string(), "active".to_string());

        let mut req_headers = HashMap::new();
        req_headers.insert("X-Api-Key".to_string(), "secret_123".to_string());

        let route = MockRoute {
            id: generate_uuid(),
            name: Some("query_header_match".to_string()),
            method: "GET".to_string(),
            path: "/api/items".to_string(),
            query_params: Some(req_query),
            headers: Some(req_headers),
            body_match: None,
            status: 200,
            status_text: None,
            response_headers: HashMap::new(),
            response_body: "{\"items\":[\"a\",\"b\"]}".to_string(),
            delay_ms: None,
            call_limit: None,
            call_count: 0,
            priority: 0,
        };

        let config = MockServerConfig {
            port: Some(0),
            routes: vec![route],
            ..Default::default()
        };

        let server_info = manager.start_server(config).await.expect("start");
        let client = reqwest::Client::new();

        // Request without required query param -> 404 default
        let resp1 = client
            .get(&format!("{}/api/items", server_info.url))
            .header("X-Api-Key", "secret_123")
            .send()
            .await
            .expect("send 1");
        assert_eq!(resp1.status().as_u16(), 404);

        // Request with matching query and header -> 200 OK
        let resp2 = client
            .get(&format!("{}/api/items?filter=active", server_info.url))
            .header("X-Api-Key", "secret_123")
            .send()
            .await
            .expect("send 2");
        assert_eq!(resp2.status().as_u16(), 200);

        manager.stop_server(&server_info.id).await.expect("stop");
    }

    #[tokio::test]
    async fn test_mock_server_cors_options_preflight() {
        let manager = MockServerManager::new();
        let config = MockServerConfig {
            port: Some(0),
            cors: Some(true),
            ..Default::default()
        };

        let server_info = manager.start_server(config).await.expect("start");
        let client = reqwest::Client::new();

        let resp = client
            .request(reqwest::Method::OPTIONS, &format!("{}/api/anything", server_info.url))
            .header("Origin", "http://localhost:3000")
            .header("Access-Control-Request-Method", "POST")
            .send()
            .await
            .expect("options request");

        assert_eq!(resp.status().as_u16(), 204);
        assert_eq!(
            resp.headers().get("access-control-allow-origin").unwrap().to_str().unwrap(),
            "*"
        );

        manager.stop_server(&server_info.id).await.expect("stop");
    }

    #[tokio::test]
    async fn test_mock_server_call_limit_and_reset() {
        let manager = MockServerManager::new();
        let route = MockRoute {
            id: generate_uuid(),
            name: Some("limited".to_string()),
            method: "GET".to_string(),
            path: "/limited".to_string(),
            query_params: None,
            headers: None,
            body_match: None,
            status: 200,
            status_text: None,
            response_headers: HashMap::new(),
            response_body: "{\"ok\":true}".to_string(),
            delay_ms: None,
            call_limit: Some(2),
            call_count: 0,
            priority: 0,
        };

        let config = MockServerConfig {
            port: Some(0),
            routes: vec![route],
            ..Default::default()
        };

        let server_info = manager.start_server(config).await.expect("start");
        let client = reqwest::Client::new();
        let url = format!("{}/limited", server_info.url);

        // 1st call -> 200
        let r1 = client.get(&url).send().await.expect("r1");
        assert_eq!(r1.status().as_u16(), 200);

        // 2nd call -> 200
        let r2 = client.get(&url).send().await.expect("r2");
        assert_eq!(r2.status().as_u16(), 200);

        // 3rd call -> call limit exhausted -> 404
        let r3 = client.get(&url).send().await.expect("r3");
        assert_eq!(r3.status().as_u16(), 404);

        // Reset server -> resets count
        let server = manager.get_server(&server_info.id).await.expect("get server");
        server.reset().await;

        // 4th call after reset -> 200 again!
        let r4 = client.get(&url).send().await.expect("r4");
        assert_eq!(r4.status().as_u16(), 200);

        manager.stop_server(&server_info.id).await.expect("stop");
    }
}
