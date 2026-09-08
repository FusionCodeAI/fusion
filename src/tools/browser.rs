//! Browser automation tool with Chrome DevTools Protocol (CDP) client and headless HTTP fallback.
//!
//! Provides comprehensive browser automation, frontend UI testing, page scraping,
//! screenshotting, and DOM verification.
//!
//! ### Architecture
//! - **CDP Client**: Pure-Rust HTTP & WebSocket client communicating with Chrome/Chromium
//!   running with `--remote-debugging-port=9222`. Supports tab management (`/json/list`,
//!   `/json/new`, `/json/close`), navigation (`Page.navigate`), visual captures
//!   (`Page.captureScreenshot`), JavaScript evaluation (`Runtime.evaluate`), and DOM
//!   queries (`DOM.querySelector`, element clicking, and form typing).
//! - **Pure-Rust WebSocket**: Compliant RFC 6455 client implemented over `tokio::net::TcpStream`
//!   with frame masking, continuation support, and JSON-RPC message correlation.
//! - **Headless HTTP Fallback**: When no CDP browser is active, falls back to direct HTTP
//!   fetching and DOM reader text extraction, providing actionable instructions on launching
//!   Chrome with remote debugging enabled.
//! - **Zero C/C++ Dependencies**: 100% pure Rust using `tokio`, `reqwest` (rustls), `serde`,
//!   and `serde_json`.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::fmt;
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

use crate::tools::types::{Tool, ToolContext};

/// Default Chrome DevTools Protocol HTTP base endpoint.
pub const DEFAULT_CDP_HTTP_URL: &str = "http://127.0.0.1:9222";

/// Default user agent mimicking desktop Chrome for HTTP fallback requests.
pub const DEFAULT_BROWSER_USER_AGENT: &str =
    "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/124.0.0.0 Safari/537.36 (Fusion/2.0)";

/// Supported browser automation actions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BrowserAction {
    /// Open a new browser tab or initialize a session at a target URL.
    Open,
    /// Navigate the current browser tab to a specified URL.
    Navigate,
    /// Capture a screenshot of the visible viewport or a specific element.
    Screenshot,
    /// Click an element matching a CSS selector.
    Click,
    /// Type text into an input or textarea element matching a CSS selector.
    Type,
    /// Evaluate a JavaScript expression or script in the page context.
    #[serde(alias = "evaluate", alias = "eval", alias = "eval_js", alias = "js")]
    EvaluateJs,
    /// Extract text, markdown, and DOM content of the active page.
    #[serde(alias = "read", alias = "dom", alias = "text")]
    Content,
    /// Close the active tab or browser session.
    #[serde(alias = "quit", alias = "exit")]
    Close,
}

impl BrowserAction {
    /// Returns the canonical string representation of this action.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Open => "open",
            Self::Navigate => "navigate",
            Self::Screenshot => "screenshot",
            Self::Click => "click",
            Self::Type => "type",
            Self::EvaluateJs => "evaluate_js",
            Self::Content => "content",
            Self::Close => "close",
        }
    }
}

impl fmt::Display for BrowserAction {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

impl FromStr for BrowserAction {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.trim().to_ascii_lowercase().as_str() {
            "open" => Ok(Self::Open),
            "navigate" | "nav" | "goto" => Ok(Self::Navigate),
            "screenshot" | "screen" | "capture" | "snap" => Ok(Self::Screenshot),
            "click" => Ok(Self::Click),
            "type" | "input" | "fill" | "write" => Ok(Self::Type),
            "evaluate_js" | "evaluate" | "eval" | "eval_js" | "js" => Ok(Self::EvaluateJs),
            "content" | "read" | "text" | "body" | "dom" => Ok(Self::Content),
            "close" | "quit" | "exit" => Ok(Self::Close),
            other => anyhow::bail!(
                "Unknown browser action '{}'. Supported actions: open, navigate, screenshot, click, type, evaluate_js, content, close",
                other
            ),
        }
    }
}

/// Arguments passed to the browser tool.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct BrowserArgs {
    /// Action to perform.
    pub action: Option<BrowserAction>,
    /// Target web page URL (for open, navigate, or HTTP content fetch).
    #[serde(default)]
    pub url: Option<String>,
    /// CSS selector for element interaction (click, type, screenshot).
    #[serde(default)]
    pub selector: Option<String>,
    /// Text to type into an input element.
    #[serde(default)]
    pub text: Option<String>,
    /// JavaScript expression or script to evaluate.
    #[serde(default)]
    pub code: Option<String>,
    /// Optional Chrome DevTools Protocol endpoint (default: http://127.0.0.1:9222).
    #[serde(default)]
    pub cdp_url: Option<String>,
    /// Optional output file path where screenshot PNG will be saved.
    #[serde(default)]
    pub screenshot_path: Option<String>,
}

/// Chrome DevTools Protocol version info retrieved from `/json/version`.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct CdpVersion {
    #[serde(rename = "Browser", default)]
    pub browser: String,
    #[serde(rename = "Protocol-Version", default)]
    pub protocol_version: String,
    #[serde(rename = "User-Agent", default)]
    pub user_agent: String,
    #[serde(rename = "V8-Version", default)]
    pub v8_version: String,
    #[serde(rename = "webSocketDebuggerUrl", default)]
    pub websocket_debugger_url: Option<String>,
}

/// Chrome DevTools Protocol target/tab retrieved from `/json/list`.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct CdpTab {
    #[serde(default)]
    pub id: String,
    #[serde(rename = "type", default)]
    pub target_type: String,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub url: String,
    #[serde(rename = "webSocketDebuggerUrl", default)]
    pub websocket_debugger_url: Option<String>,
    #[serde(rename = "devtoolsFrontendUrl", default)]
    pub devtools_frontend_url: Option<String>,
}

/// Extracted page content from the browser DOM.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct PageContent {
    pub title: String,
    pub url: String,
    pub text: String,
    pub html: String,
}

/// Result of a headless HTTP fallback fetch.
#[derive(Debug, Clone)]
pub struct HttpFallbackResult {
    pub url: String,
    pub status: u16,
    pub title: String,
    pub content: String,
    pub content_length: usize,
}

/// Pure-Rust RFC 6455 WebSocket client communicating with CDP over TCP.
pub struct CdpWebSocket {
    stream: tokio::net::TcpStream,
    next_id: u64,
}

impl CdpWebSocket {
    /// Connects to a CDP WebSocket endpoint (e.g. `ws://127.0.0.1:9222/devtools/page/XYZ`).
    pub async fn connect(ws_url: &str) -> anyhow::Result<Self> {
        let (host, port, path) = parse_ws_url(ws_url)?;
        let addr = format!("{}:{}", host, port);

        let mut stream = tokio::time::timeout(
            Duration::from_secs(5),
            tokio::net::TcpStream::connect(&addr),
        )
        .await
        .map_err(|_| anyhow::anyhow!("Connection to CDP WebSocket at {} timed out", addr))?
        .map_err(|e| anyhow::anyhow!("Failed to connect to CDP WebSocket at {}: {}", addr, e))?;

        // Perform RFC 6455 HTTP handshake
        let handshake_req = format!(
            "GET {} HTTP/1.1\r\n\
             Host: {}:{}\r\n\
             Upgrade: websocket\r\n\
             Connection: Upgrade\r\n\
             Sec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\n\
             Sec-WebSocket-Version: 13\r\n\
             User-Agent: Fusion-CDP/2.0\r\n\
             \r\n",
            path, host, port
        );

        stream.write_all(handshake_req.as_bytes()).await?;
        stream.flush().await?;

        // Read handshake response until header termination (\r\n\r\n)
        let mut response_buf = Vec::new();
        let mut temp_buf = [0u8; 1024];
        let mut handshake_ok = false;

        let handshake_timeout = Duration::from_secs(5);
        let start = std::time::Instant::now();

        while start.elapsed() < handshake_timeout {
            let n = tokio::time::timeout(Duration::from_secs(2), stream.read(&mut temp_buf))
                .await
                .map_err(|_| anyhow::anyhow!("CDP WebSocket handshake read timed out"))?
                .map_err(|e| anyhow::anyhow!("Error reading CDP WebSocket handshake: {}", e))?;

            if n == 0 {
                anyhow::bail!("CDP WebSocket server closed connection during handshake");
            }

            response_buf.extend_from_slice(&temp_buf[..n]);
            if let Some(pos) = find_subsequence(&response_buf, b"\r\n\r\n") {
                let header_str = String::from_utf8_lossy(&response_buf[..pos]);
                if header_str.starts_with("HTTP/1.1 101") || header_str.starts_with("HTTP/1.0 101")
                {
                    handshake_ok = true;
                    break;
                } else {
                    anyhow::bail!(
                        "CDP WebSocket handshake rejected by server: {}",
                        header_str.lines().next().unwrap_or("Unknown status")
                    );
                }
            }
        }

        if !handshake_ok {
            anyhow::bail!("Timed out waiting for CDP WebSocket handshake response");
        }

        Ok(Self {
            stream,
            next_id: 1,
        })
    }

    /// Sends a masked text frame to the CDP WebSocket server (RFC 6455).
    pub async fn send_text(&mut self, text: &str) -> anyhow::Result<()> {
        let payload = text.as_bytes();
        let len = payload.len();
        let mut frame = Vec::with_capacity(14 + len);

        // Byte 0: FIN (0x80) | Opcode text (0x01) -> 0x81
        frame.push(0x81);

        // Byte 1: Mask bit (0x80) | 7-bit length or extension indicator
        if len < 126 {
            frame.push(0x80 | (len as u8));
        } else if len <= 65535 {
            frame.push(0x80 | 126);
            frame.extend_from_slice(&(len as u16).to_be_bytes());
        } else {
            frame.push(0x80 | 127);
            frame.extend_from_slice(&(len as u64).to_be_bytes());
        }

        // Masking key (4 bytes)
        let mask_key: [u8; 4] = [0x5A, 0x3C, 0xA5, 0xC3];
        frame.extend_from_slice(&mask_key);

        // Mask payload
        for (i, &b) in payload.iter().enumerate() {
            frame.push(b ^ mask_key[i % 4]);
        }

        self.stream.write_all(&frame).await?;
        self.stream.flush().await?;
        Ok(())
    }

    /// Reads the next complete WebSocket message payload (supporting multi-frame messages).
    pub async fn read_message(&mut self) -> anyhow::Result<String> {
        let mut full_payload = Vec::new();

        loop {
            let mut header = [0u8; 2];
            self.stream.read_exact(&mut header).await?;

            let fin = (header[0] & 0x80) != 0;
            let opcode = header[0] & 0x0f;
            let masked = (header[1] & 0x80) != 0;
            let mut payload_len = (header[1] & 0x7f) as u64;

            if payload_len == 126 {
                let mut ext = [0u8; 2];
                self.stream.read_exact(&mut ext).await?;
                payload_len = u16::from_be_bytes(ext) as u64;
            } else if payload_len == 127 {
                let mut ext = [0u8; 8];
                self.stream.read_exact(&mut ext).await?;
                payload_len = u64::from_be_bytes(ext);
            }

            let mask = if masked {
                let mut m = [0u8; 4];
                self.stream.read_exact(&mut m).await?;
                Some(m)
            } else {
                None
            };

            let mut chunk = vec![0u8; payload_len as usize];
            self.stream.read_exact(&mut chunk).await?;

            if let Some(m) = mask {
                for (i, b) in chunk.iter_mut().enumerate() {
                    *b ^= m[i % 4];
                }
            }

            match opcode {
                1 | 0 => {
                    // Text frame or continuation frame
                    full_payload.extend_from_slice(&chunk);
                    if fin {
                        let text = String::from_utf8(full_payload).map_err(|e| {
                            anyhow::anyhow!("Invalid UTF-8 in CDP WebSocket frame: {}", e)
                        })?;
                        return Ok(text);
                    }
                }
                8 => {
                    // Connection close
                    anyhow::bail!("CDP WebSocket server closed the connection");
                }
                9 => {
                    // Ping -> Reply with Pong (opcode 10, FIN=1: 0x8A)
                    let pong = [0x8A, 0x00];
                    let _ = self.stream.write_all(&pong).await;
                    let _ = self.stream.flush().await;
                }
                10 => {
                    // Pong received -> ignore
                }
                _ => {
                    // Ignore unknown opcodes
                }
            }
        }
    }

    /// Invokes a CDP JSON-RPC method, waiting for the matching response ID.
    pub async fn call_method(&mut self, method: &str, params: Value) -> anyhow::Result<Value> {
        let id = self.next_id;
        self.next_id += 1;

        let req = json!({
            "id": id,
            "method": method,
            "params": params
        });

        self.send_text(&req.to_string()).await?;

        // Wait for response matching our request ID (skipping asynchronous events)
        let timeout = Duration::from_secs(15);
        let start = std::time::Instant::now();

        while start.elapsed() < timeout {
            let msg = tokio::time::timeout(Duration::from_secs(5), self.read_message())
                .await
                .map_err(|_| anyhow::anyhow!("Timeout reading response for '{}'", method))??;

            if let Ok(val) = serde_json::from_str::<Value>(&msg) {
                if val.get("id").and_then(|v| v.as_u64()) == Some(id) {
                    if let Some(err) = val.get("error") {
                        let msg = err
                            .get("message")
                            .and_then(|v| v.as_str())
                            .unwrap_or("Unknown CDP error");
                        anyhow::bail!("CDP method '{}' returned error: {}", method, msg);
                    }
                    return Ok(val.get("result").cloned().unwrap_or(Value::Null));
                }
            }
        }

        anyhow::bail!("Timed out waiting for CDP response to '{}'", method);
    }

    /// Enables the Page domain and navigates the tab to the given URL.
    pub async fn navigate(&mut self, url: &str) -> anyhow::Result<Value> {
        let _ = self.call_method("Page.enable", json!({})).await;
        let res = self
            .call_method("Page.navigate", json!({ "url": url }))
            .await?;
        // Brief pause to allow page transition to start
        tokio::time::sleep(Duration::from_millis(300)).await;
        Ok(res)
    }

    /// Captures a screenshot as a base64-encoded string.
    pub async fn capture_screenshot(
        &mut self,
        selector: Option<&str>,
    ) -> anyhow::Result<String> {
        let _ = self.call_method("Page.enable", json!({})).await;

        let clip = if let Some(sel) = selector {
            // Retrieve bounding rect for element screenshot
            let js = format!(
                r#"(() => {{
                    const el = document.querySelector({sel:?});
                    if (!el) return null;
                    const r = el.getBoundingClientRect();
                    return {{
                        x: Math.max(0, r.left + window.scrollX),
                        y: Math.max(0, r.top + window.scrollY),
                        width: Math.max(1, r.width),
                        height: Math.max(1, r.height),
                        scale: 1
                    }};
                }})()"#,
                sel = sel
            );
            let eval_res = self.evaluate_js(&js).await?;
            if eval_res.is_null() {
                None
            } else {
                Some(eval_res)
            }
        } else {
            None
        };

        let mut params = json!({
            "format": "png",
            "quality": 90,
            "fromSurface": true,
        });

        if let Some(c) = clip {
            if let Some(obj) = params.as_object_mut() {
                obj.insert("clip".to_string(), c);
            }
        }

        let res = self.call_method("Page.captureScreenshot", params).await?;
        let data = res
            .get("data")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("Missing 'data' field in captureScreenshot response"))?;

        Ok(data.to_string())
    }

    /// Evaluates a JavaScript expression and returns the value.
    pub async fn evaluate_js(&mut self, code: &str) -> anyhow::Result<Value> {
        let _ = self.call_method("Runtime.enable", json!({})).await;
        let res = self
            .call_method(
                "Runtime.evaluate",
                json!({
                    "expression": code,
                    "returnByValue": true,
                    "awaitPromise": true,
                }),
            )
            .await?;

        if let Some(exception) = res.get("exceptionDetails") {
            let desc = exception
                .get("exception")
                .and_then(|e| e.get("description"))
                .and_then(|v| v.as_str())
                .or_else(|| exception.get("text").and_then(|v| v.as_str()))
                .unwrap_or("JavaScript execution exception");
            anyhow::bail!("JavaScript error: {}", desc);
        }

        let val = res
            .get("result")
            .and_then(|r| r.get("value"))
            .cloned()
            .unwrap_or(Value::Null);

        Ok(val)
    }

    /// Queries a node ID using `DOM.querySelector`.
    pub async fn query_selector(&mut self, selector: &str) -> anyhow::Result<Option<i64>> {
        let _ = self.call_method("DOM.enable", json!({})).await;
        let doc = self.call_method("DOM.getDocument", json!({ "depth": 1 })).await?;
        let root_id = doc
            .get("root")
            .and_then(|r| r.get("nodeId"))
            .and_then(|v| v.as_i64())
            .unwrap_or(1);

        let res = self
            .call_method(
                "DOM.querySelector",
                json!({
                    "nodeId": root_id,
                    "selector": selector
                }),
            )
            .await?;

        let node_id = res.get("nodeId").and_then(|v| v.as_i64());
        Ok(node_id.filter(|&id| id > 0))
    }

    /// Clicks an element matching the CSS selector.
    pub async fn click(&mut self, selector: &str) -> anyhow::Result<Value> {
        let js = format!(
            r#"(() => {{
                const el = document.querySelector({sel:?});
                if (!el) throw new Error("Element not found for selector: " + {sel:?});
                el.scrollIntoView({{ behavior: "instant", block: "center" }});
                el.focus();
                el.click();
                return {{
                    tag: el.tagName.toLowerCase(),
                    text: (el.innerText || el.textContent || "").trim().slice(0, 120),
                    id: el.id || null,
                    classes: el.className || null
                }};
            }})()"#,
            sel = selector
        );
        self.evaluate_js(&js).await
    }

    /// Types text into an element matching the CSS selector.
    pub async fn type_text(&mut self, selector: &str, text: &str) -> anyhow::Result<Value> {
        let js = format!(
            r#"(() => {{
                const el = document.querySelector({sel:?});
                if (!el) throw new Error("Element not found for selector: " + {sel:?});
                el.scrollIntoView({{ behavior: "instant", block: "center" }});
                el.focus();
                if ('value' in el) {{
                    el.value = {txt:?};
                    el.dispatchEvent(new Event('input', {{ bubbles: true }}));
                    el.dispatchEvent(new Event('change', {{ bubbles: true }}));
                }} else {{
                    el.innerText = {txt:?};
                }}
                return {{
                    tag: el.tagName.toLowerCase(),
                    value: el.value || el.innerText,
                    selector: {sel:?}
                }};
            }})()"#,
            sel = selector,
            txt = text
        );
        self.evaluate_js(&js).await
    }

    /// Retrieves title, URL, readable text, and full HTML of the active page.
    pub async fn get_content(&mut self) -> anyhow::Result<PageContent> {
        let js = r#"(() => {
            return {
                title: document.title || "",
                url: window.location.href || "",
                text: document.body ? (document.body.innerText || document.body.textContent || "") : "",
                html: document.documentElement ? document.documentElement.outerHTML : ""
            };
        })()"#;

        let val = self.evaluate_js(js).await?;
        let title = val.get("title").and_then(|v| v.as_str()).unwrap_or("").to_string();
        let url = val.get("url").and_then(|v| v.as_str()).unwrap_or("").to_string();
        let text = val.get("text").and_then(|v| v.as_str()).unwrap_or("").to_string();
        let html = val.get("html").and_then(|v| v.as_str()).unwrap_or("").to_string();

        Ok(PageContent {
            title,
            url,
            text,
            html,
        })
    }
}

/// Pure-Rust HTTP client for Chrome DevTools Protocol management endpoints.
#[derive(Clone, Debug)]
pub struct CdpClient {
    client: reqwest::Client,
    base_url: String,
}

impl CdpClient {
    /// Creates a new CDP HTTP client with the given base URL (e.g. `http://127.0.0.1:9222`).
    pub fn new(base_url: impl Into<String>) -> Self {
        let base = base_url.into();
        let trimmed = base.trim_end_matches('/').to_string();
        let client = reqwest::Client::builder()
            .connect_timeout(Duration::from_millis(1500))
            .timeout(Duration::from_secs(10))
            .build()
            .unwrap_or_else(|_| reqwest::Client::new());

        Self {
            client,
            base_url: trimmed,
        }
    }

    /// Creates a CDP client with a custom `reqwest::Client`.
    pub fn with_client(client: reqwest::Client, base_url: impl Into<String>) -> Self {
        Self {
            client,
            base_url: base_url.into().trim_end_matches('/').to_string(),
        }
    }

    /// Returns the configured CDP base URL.
    pub fn base_url(&self) -> &str {
        &self.base_url
    }

    /// Checks if the CDP browser endpoint is reachable and responsive.
    pub async fn is_online(&self) -> bool {
        self.version().await.is_ok()
    }

    /// Queries the `/json/version` endpoint.
    pub async fn version(&self) -> anyhow::Result<CdpVersion> {
        let url = format!("{}/json/version", self.base_url);
        let resp = self.client.get(&url).send().await?;
        if !resp.status().is_success() {
            anyhow::bail!("CDP /json/version returned HTTP {}", resp.status());
        }
        let ver: CdpVersion = resp.json().await?;
        Ok(ver)
    }

    /// Queries the `/json/list` endpoint to find all active browser tabs.
    pub async fn list_tabs(&self) -> anyhow::Result<Vec<CdpTab>> {
        let url = format!("{}/json/list", self.base_url);
        let resp = match self.client.get(&url).send().await {
            Ok(r) if r.status().is_success() => r,
            _ => {
                let fallback = format!("{}/json", self.base_url);
                self.client.get(&fallback).send().await?
            }
        };

        if !resp.status().is_success() {
            anyhow::bail!("CDP /json/list returned HTTP {}", resp.status());
        }

        let tabs: Vec<CdpTab> = resp.json().await?;
        let filtered: Vec<CdpTab> = tabs
            .into_iter()
            .filter(|t| t.target_type == "page" || t.websocket_debugger_url.is_some())
            .collect();
        Ok(filtered)
    }

    /// Opens a new browser tab via `/json/new` and optionally navigates to a URL.
    pub async fn new_tab(&self, url: Option<&str>) -> anyhow::Result<CdpTab> {
        let req_url = match url {
            Some(u) if !u.is_empty() => format!("{}/json/new?{}", self.base_url, u),
            _ => format!("{}/json/new", self.base_url),
        };

        let resp = match self.client.put(&req_url).send().await {
            Ok(r) if r.status().is_success() => r,
            _ => self.client.get(&req_url).send().await?,
        };

        if !resp.status().is_success() {
            anyhow::bail!("CDP /json/new returned HTTP {}", resp.status());
        }

        let tab: CdpTab = resp.json().await?;
        Ok(tab)
    }

    /// Closes a tab via `/json/close/{id}`.
    pub async fn close_tab(&self, tab_id: &str) -> anyhow::Result<bool> {
        let url = format!("{}/json/close/{}", self.base_url, tab_id);
        let resp = self.client.get(&url).send().await?;
        Ok(resp.status().is_success())
    }

    /// Connects a WebSocket to the specified tab.
    pub async fn connect_tab(&self, tab: &CdpTab) -> anyhow::Result<CdpWebSocket> {
        let ws_url = tab.websocket_debugger_url.as_deref().ok_or_else(|| {
            anyhow::anyhow!("Tab '{}' has no webSocketDebuggerUrl", tab.id)
        })?;
        CdpWebSocket::connect(ws_url).await
    }
}

/// Browser automation tool implementing the Fusion `Tool` trait.
#[derive(Clone)]
pub struct BrowserTool {
    client: reqwest::Client,
    default_cdp_url: String,
}

impl Default for BrowserTool {
    fn default() -> Self {
        Self::new()
    }
}

impl BrowserTool {
    /// Creates a new browser tool with default settings.
    pub fn new() -> Self {
        let client = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(3))
            .timeout(Duration::from_secs(15))
            .build()
            .unwrap_or_else(|_| reqwest::Client::new());

        Self {
            client,
            default_cdp_url: DEFAULT_CDP_HTTP_URL.to_string(),
        }
    }

    /// Overrides the default Chrome DevTools Protocol URL.
    pub fn with_cdp_url(mut self, url: impl Into<String>) -> Self {
        self.default_cdp_url = url.into();
        self
    }

    /// Fetches a web page via direct HTTP (used in headless fallback mode).
    pub async fn fetch_http(&self, url: &str) -> anyhow::Result<HttpFallbackResult> {
        let norm_url = normalize_url(url);
        let resp = self
            .client
            .get(&norm_url)
            .header(reqwest::header::USER_AGENT, DEFAULT_BROWSER_USER_AGENT)
            .header(
                reqwest::header::ACCEPT,
                "text/html,application/xhtml+xml,application/xml;q=0.9,*/*;q=0.8",
            )
            .send()
            .await
            .map_err(|e| anyhow::anyhow!("HTTP fallback request failed for '{}': {}", norm_url, e))?;

        let status = resp.status().as_u16();
        let body = resp.text().await.unwrap_or_default();
        let content_length = body.len();
        let (title, content) = extract_dom_text_and_title(&body);

        Ok(HttpFallbackResult {
            url: norm_url,
            status,
            title,
            content,
            content_length,
        })
    }
}

#[async_trait]
impl Tool for BrowserTool {
    fn name(&self) -> &str {
        "browser"
    }

    fn description(&self) -> &str {
        "Interactive browser automation and web testing tool powered by Chrome DevTools Protocol (CDP). \
         Supports opening tabs, navigating URLs, capturing viewport/element PNG screenshots, \
         clicking CSS selectors, filling form inputs, evaluating client-side JavaScript, and reading DOM content. \
         Includes automatic headless HTTP fallback and diagnostics if Chrome remote debugging is offline."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "action": {
                    "type": "string",
                    "enum": ["open", "navigate", "screenshot", "click", "type", "evaluate_js", "content", "close"],
                    "description": "Browser action to execute: 'open', 'navigate', 'screenshot', 'click', 'type', 'evaluate_js', 'content', or 'close'."
                },
                "url": {
                    "type": "string",
                    "description": "Target web page URL (for 'open' and 'navigate' actions, or 'content' in fallback mode)."
                },
                "selector": {
                    "type": "string",
                    "description": "CSS selector targeting a DOM element (for 'click', 'type', or element 'screenshot')."
                },
                "text": {
                    "type": "string",
                    "description": "Text string to type into the selected input/textarea element (for 'type' action)."
                },
                "code": {
                    "type": "string",
                    "description": "JavaScript code snippet or expression to evaluate in the browser page context (for 'evaluate_js' action)."
                },
                "cdp_url": {
                    "type": "string",
                    "description": "Optional Chrome DevTools Protocol endpoint (defaults to 'http://127.0.0.1:9222')."
                },
                "screenshot_path": {
                    "type": "string",
                    "description": "Optional file path where the captured screenshot PNG will be saved (for 'screenshot' action)."
                }
            },
            "required": ["action"]
        })
    }

    async fn execute(&self, args: Value, ctx: &ToolContext) -> anyhow::Result<String> {
        let parsed_args = parse_browser_args(&args)?;
        let action = parsed_args.action.ok_or_else(|| {
            anyhow::anyhow!(
                "Missing required parameter 'action'. Supported actions: open, navigate, screenshot, click, type, evaluate_js, content, close"
            )
        })?;

        let cdp_base = parsed_args
            .cdp_url
            .as_deref()
            .unwrap_or(&self.default_cdp_url);

        let cdp_client = CdpClient::with_client(self.client.clone(), cdp_base);
        let cdp_online = cdp_client.is_online().await;

        match action {
            BrowserAction::Open => {
                let target_url = parsed_args
                    .url
                    .as_deref()
                    .unwrap_or("about:blank");

                if cdp_online {
                    let tab = cdp_client.new_tab(Some(target_url)).await?;
                    let mut out = String::from("# Browser Tab Opened (CDP)\n\n");
                    out.push_str(&format!("- **Target URL**: {}\n", target_url));
                    out.push_str(&format!("- **Tab ID**: {}\n", tab.id));
                    out.push_str(&format!("- **Title**: {}\n", if tab.title.is_empty() { "New Tab" } else { &tab.title }));
                    out.push_str(&format!("- **CDP Endpoint**: {}\n", cdp_client.base_url()));
                    if let Some(ws) = &tab.websocket_debugger_url {
                        out.push_str(&format!("- **WebSocket**: `{}`\n", ws));
                    }
                    Ok(out)
                } else if target_url != "about:blank" {
                    // Headless HTTP fallback
                    let fallback = self.fetch_http(target_url).await?;
                    let mut out = String::from("# Page Fetched (Headless HTTP Fallback Mode)\n\n");
                    out.push_str(&format!("- **URL**: {}\n", fallback.url));
                    out.push_str(&format!("- **Title**: {}\n", fallback.title));
                    out.push_str(&format!("- **HTTP Status**: {}\n", fallback.status));
                    out.push_str(&format!("- **Content Length**: {} bytes\n\n", fallback.content_length));
                    out.push_str("## Page Content Preview\n\n");
                    let preview = truncate_content(&fallback.content, 4000);
                    out.push_str(&preview);
                    out.push_str("\n\n---\n");
                    out.push_str(&cdp_startup_advice(cdp_base));
                    Ok(out)
                } else {
                    let mut out = String::from("# Browser Open (CDP Required)\n\n");
                    out.push_str("Opening an empty browser tab requires an active Chrome DevTools Protocol session.\n\n");
                    out.push_str(&cdp_startup_advice(cdp_base));
                    Ok(out)
                }
            }

            BrowserAction::Navigate => {
                let target_url = parsed_args.url.as_deref().ok_or_else(|| {
                    anyhow::anyhow!("Parameter 'url' is required for action 'navigate'")
                })?;
                let norm_url = normalize_url(target_url);

                if cdp_online {
                    let tabs = cdp_client.list_tabs().await?;
                    let active_tab = if let Some(first) = tabs.first() {
                        first.clone()
                    } else {
                        cdp_client.new_tab(Some(&norm_url)).await?
                    };

                    let mut ws = cdp_client.connect_tab(&active_tab).await?;
                    let _ = ws.navigate(&norm_url).await?;

                    let mut out = String::from("# Browser Navigation Complete (CDP)\n\n");
                    out.push_str(&format!("- **Navigated To**: {}\n", norm_url));
                    out.push_str(&format!("- **Tab ID**: {}\n", active_tab.id));
                    out.push_str(&format!("- **Status**: Navigation requested successfully\n"));
                    Ok(out)
                } else {
                    // Headless HTTP fallback
                    let fallback = self.fetch_http(&norm_url).await?;
                    let mut out = String::from("# Navigation Complete (Headless HTTP Fallback Mode)\n\n");
                    out.push_str(&format!("- **URL**: {}\n", fallback.url));
                    out.push_str(&format!("- **Title**: {}\n", fallback.title));
                    out.push_str(&format!("- **HTTP Status**: {}\n", fallback.status));
                    out.push_str(&format!("- **Content Length**: {} bytes\n\n", fallback.content_length));
                    out.push_str("## Page Content Preview\n\n");
                    let preview = truncate_content(&fallback.content, 4000);
                    out.push_str(&preview);
                    out.push_str("\n\n---\n");
                    out.push_str(&cdp_startup_advice(cdp_base));
                    Ok(out)
                }
            }

            BrowserAction::Screenshot => {
                let target_path_str = parsed_args
                    .screenshot_path
                    .clone()
                    .unwrap_or_else(|| "screenshot.png".to_string());
                let output_path = resolve_path(&ctx.cwd, &target_path_str);

                if cdp_online {
                    let tabs = cdp_client.list_tabs().await?;
                    let active_tab = tabs.first().ok_or_else(|| {
                        anyhow::anyhow!("No active browser tab found to screenshot")
                    })?;

                    let mut ws = cdp_client.connect_tab(active_tab).await?;

                    // If URL is supplied and differs, navigate first
                    if let Some(u) = &parsed_args.url {
                        let norm = normalize_url(u);
                        if active_tab.url != norm {
                            let _ = ws.navigate(&norm).await;
                            tokio::time::sleep(Duration::from_millis(500)).await;
                        }
                    }

                    let b64 = ws
                        .capture_screenshot(parsed_args.selector.as_deref())
                        .await?;
                    let bytes = base64_decode(&b64).ok_or_else(|| {
                        anyhow::anyhow!("Failed to decode base64 screenshot data from CDP")
                    })?;

                    if let Some(parent) = output_path.parent() {
                        let _ = tokio::fs::create_dir_all(parent).await;
                    }
                    tokio::fs::write(&output_path, &bytes).await.map_err(|e| {
                        anyhow::anyhow!(
                            "Failed to write screenshot to '{}': {}",
                            output_path.display(),
                            e
                        )
                    })?;

                    let mut out = String::from("# Screenshot Captured Successfully\n\n");
                    out.push_str(&format!("- **Output Path**: `{}`\n", output_path.display()));
                    out.push_str(&format!("- **File Size**: {} bytes\n", bytes.len()));
                    out.push_str(&format!("- **Format**: PNG\n"));
                    out.push_str(&format!("- **Tab Title**: {}\n", active_tab.title));
                    if let Some(sel) = &parsed_args.selector {
                        out.push_str(&format!("- **Element Selector**: `{}`\n", sel));
                    } else {
                        out.push_str("- **Capture Mode**: Full Viewport\n");
                    }
                    Ok(out)
                } else {
                    let mut out = String::from("# Screenshot Unavailable (CDP Browser Required)\n\n");
                    out.push_str(
                        "Capturing rendered visual screenshots requires an active Chrome DevTools Protocol session.\n\n",
                    );
                    if let Some(u) = &parsed_args.url {
                        out.push_str(&format!("Target URL requested: `{}`\n\n", u));
                    }
                    out.push_str(&cdp_startup_advice(cdp_base));
                    Ok(out)
                }
            }

            BrowserAction::Click => {
                let selector = parsed_args.selector.as_deref().ok_or_else(|| {
                    anyhow::anyhow!("Parameter 'selector' is required for action 'click'")
                })?;

                if cdp_online {
                    let tabs = cdp_client.list_tabs().await?;
                    let active_tab = tabs.first().ok_or_else(|| {
                        anyhow::anyhow!("No active browser tab found to perform click action")
                    })?;

                    let mut ws = cdp_client.connect_tab(active_tab).await?;
                    let result = ws.click(selector).await?;

                    let tag = result.get("tag").and_then(|v| v.as_str()).unwrap_or("element");
                    let text = result.get("text").and_then(|v| v.as_str()).unwrap_or("");

                    let mut out = String::from("# Element Clicked Successfully\n\n");
                    out.push_str(&format!("- **Selector**: `{}`\n", selector));
                    out.push_str(&format!("- **Tag Name**: `<{}>`\n", tag));
                    if !text.is_empty() {
                        out.push_str(&format!("- **Inner Text**: \"{}\"\n", text));
                    }
                    out.push_str(&format!("- **Tab Title**: {}\n", active_tab.title));
                    Ok(out)
                } else {
                    let mut out = String::from("# Click Action Unavailable (CDP Browser Required)\n\n");
                    out.push_str(&format!(
                        "Interactively clicking DOM element `{}` requires an active Chrome DevTools Protocol session.\n\n",
                        selector
                    ));
                    out.push_str(&cdp_startup_advice(cdp_base));
                    Ok(out)
                }
            }

            BrowserAction::Type => {
                let selector = parsed_args.selector.as_deref().ok_or_else(|| {
                    anyhow::anyhow!("Parameter 'selector' is required for action 'type'")
                })?;
                let text = parsed_args
                    .text
                    .as_deref()
                    .ok_or_else(|| anyhow::anyhow!("Parameter 'text' is required for action 'type'"))?;

                if cdp_online {
                    let tabs = cdp_client.list_tabs().await?;
                    let active_tab = tabs.first().ok_or_else(|| {
                        anyhow::anyhow!("No active browser tab found to perform typing action")
                    })?;

                    let mut ws = cdp_client.connect_tab(active_tab).await?;
                    let result = ws.type_text(selector, text).await?;

                    let tag = result.get("tag").and_then(|v| v.as_str()).unwrap_or("input");

                    let mut out = String::from("# Text Input Successful\n\n");
                    out.push_str(&format!("- **Selector**: `{}`\n", selector));
                    out.push_str(&format!("- **Tag Name**: `<{}>`\n", tag));
                    out.push_str(&format!("- **Typed Length**: {} characters\n", text.len()));
                    out.push_str(&format!("- **Value Set**: \"{}\"\n", text));
                    out.push_str(&format!("- **Tab Title**: {}\n", active_tab.title));
                    Ok(out)
                } else {
                    let mut out = String::from("# Type Action Unavailable (CDP Browser Required)\n\n");
                    out.push_str(&format!(
                        "Typing into DOM element `{}` requires an active Chrome DevTools Protocol session.\n\n",
                        selector
                    ));
                    out.push_str(&cdp_startup_advice(cdp_base));
                    Ok(out)
                }
            }

            BrowserAction::EvaluateJs => {
                let code = parsed_args.code.as_deref().ok_or_else(|| {
                    anyhow::anyhow!("Parameter 'code' is required for action 'evaluate_js'")
                })?;

                if cdp_online {
                    let tabs = cdp_client.list_tabs().await?;
                    let active_tab = tabs.first().ok_or_else(|| {
                        anyhow::anyhow!("No active browser tab found for JavaScript evaluation")
                    })?;

                    let mut ws = cdp_client.connect_tab(active_tab).await?;
                    let result = ws.evaluate_js(code).await?;

                    let mut out = String::from("# JavaScript Execution Result\n\n");
                    out.push_str("**Code**:\n```javascript\n");
                    out.push_str(code);
                    out.push_str("\n```\n\n**Result**:\n```json\n");
                    out.push_str(
                        &serde_json::to_string_pretty(&result)
                            .unwrap_or_else(|_| result.to_string()),
                    );
                    out.push_str("\n```\n");
                    Ok(out)
                } else {
                    let mut out = format!(
                        "# JavaScript Evaluation Unavailable (CDP Browser Required)\n\n"
                    );
                    out.push_str(
                        "Evaluating client-side JavaScript in a web page requires an active Chrome DevTools Protocol session.\n\n",
                    );
                    out.push_str(&cdp_startup_advice(cdp_base));
                    Ok(out)
                }
            }

            BrowserAction::Content => {
                if cdp_online {
                    let tabs = cdp_client.list_tabs().await?;
                    let active_tab = tabs.first().ok_or_else(|| {
                        anyhow::anyhow!("No active browser tab found to retrieve content")
                    })?;

                    let mut ws = cdp_client.connect_tab(active_tab).await?;
                    let content = ws.get_content().await?;

                    let mut out = String::from("# Page Content (CDP Active Tab)\n\n");
                    out.push_str(&format!("- **Title**: {}\n", content.title));
                    out.push_str(&format!("- **URL**: {}\n", content.url));
                    out.push_str(&format!("- **Text Length**: {} characters\n\n", content.text.len()));
                    out.push_str("## Extracted Text Content\n\n");
                    out.push_str(&truncate_content(&content.text, 8000));
                    out.push('\n');
                    Ok(out)
                } else if let Some(u) = &parsed_args.url {
                    // Headless HTTP fallback
                    let fallback = self.fetch_http(u).await?;
                    let mut out = String::from("# Page Content (Headless HTTP Fallback Mode)\n\n");
                    out.push_str(&format!("- **Title**: {}\n", fallback.title));
                    out.push_str(&format!("- **URL**: {}\n", fallback.url));
                    out.push_str(&format!("- **HTTP Status**: {}\n", fallback.status));
                    out.push_str(&format!("- **Length**: {} bytes\n\n", fallback.content_length));
                    out.push_str("## Extracted Text Content\n\n");
                    out.push_str(&truncate_content(&fallback.content, 8000));
                    out.push_str("\n\n---\n");
                    out.push_str(&cdp_startup_advice(cdp_base));
                    Ok(out)
                } else {
                    let mut out = String::from("# Content Unavailable (CDP Required)\n\n");
                    out.push_str(
                        "Extracting active tab content without specifying a URL requires an active Chrome DevTools Protocol session.\n\n",
                    );
                    out.push_str(&cdp_startup_advice(cdp_base));
                    Ok(out)
                }
            }

            BrowserAction::Close => {
                if cdp_online {
                    let tabs = cdp_client.list_tabs().await?;
                    if let Some(tab) = tabs.first() {
                        let _ = cdp_client.close_tab(&tab.id).await;
                        let mut out = String::from("# Browser Tab Closed (CDP)\n\n");
                        out.push_str(&format!("- **Closed Tab ID**: {}\n", tab.id));
                        out.push_str(&format!("- **Tab Title**: {}\n", tab.title));
                        Ok(out)
                    } else {
                        Ok("# Browser Close\n\nNo active tabs found to close.\n".to_string())
                    }
                } else {
                    let mut out = String::from("# Browser Session Closed\n\n");
                    out.push_str(&format!(
                        "No active Chrome DevTools Protocol session was connected on `{}`.\n",
                        cdp_base
                    ));
                    Ok(out)
                }
            }
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Helpers & Utilities
// ─────────────────────────────────────────────────────────────────────────────

/// Parses a JSON value into structured `BrowserArgs`, inferring sensible defaults.
pub fn parse_browser_args(args: &Value) -> anyhow::Result<BrowserArgs> {
    let mut parsed: BrowserArgs = serde_json::from_value(args.clone()).unwrap_or_default();

    // If action was not deserialized from enum, attempt string fallback
    if parsed.action.is_none() {
        if let Some(act_str) = args.get("action").and_then(|v| v.as_str()) {
            parsed.action = Some(BrowserAction::from_str(act_str)?);
        } else if args.get("url").is_some() {
            parsed.action = Some(BrowserAction::Open);
        } else if args.get("code").is_some() {
            parsed.action = Some(BrowserAction::EvaluateJs);
        } else if args.get("selector").is_some() && args.get("text").is_some() {
            parsed.action = Some(BrowserAction::Type);
        } else if args.get("selector").is_some() {
            parsed.action = Some(BrowserAction::Click);
        }
    }

    if let Some(u) = args.get("url").and_then(|v| v.as_str()) {
        if !u.trim().is_empty() {
            parsed.url = Some(u.trim().to_string());
        }
    }

    if let Some(s) = args.get("selector").and_then(|v| v.as_str()) {
        if !s.trim().is_empty() {
            parsed.selector = Some(s.trim().to_string());
        }
    }

    if let Some(t) = args.get("text").and_then(|v| v.as_str()) {
        parsed.text = Some(t.to_string());
    }

    if let Some(c) = args.get("code").and_then(|v| v.as_str()) {
        if !c.trim().is_empty() {
            parsed.code = Some(c.trim().to_string());
        }
    }

    if let Some(cdp) = args.get("cdp_url").and_then(|v| v.as_str()) {
        if !cdp.trim().is_empty() {
            parsed.cdp_url = Some(cdp.trim().to_string());
        }
    }

    if let Some(p) = args.get("screenshot_path").and_then(|v| v.as_str()) {
        if !p.trim().is_empty() {
            parsed.screenshot_path = Some(p.trim().to_string());
        }
    }

    Ok(parsed)
}

/// Normalizes a URL by prepending `https://` if no scheme is present.
pub fn normalize_url(url: &str) -> String {
    let trimmed = url.trim();
    if trimmed.starts_with("http://")
        || trimmed.starts_with("https://")
        || trimmed.starts_with("about:")
        || trimmed.starts_with("data:")
        || trimmed.starts_with("file://")
    {
        trimmed.to_string()
    } else {
        format!("https://{}", trimmed)
    }
}

/// Resolves a path relative to the current working directory.
pub fn resolve_path(cwd: &Path, path: &str) -> PathBuf {
    let p = Path::new(path);
    if p.is_absolute() {
        p.to_path_buf()
    } else {
        cwd.join(p)
    }
}

/// Parses a WebSocket URL into `(host, port, path)`.
pub fn parse_ws_url(url: &str) -> anyhow::Result<(String, u16, String)> {
    let trimmed = url.trim();
    let without_scheme = if let Some(rest) = trimmed.strip_prefix("ws://") {
        rest
    } else if let Some(rest) = trimmed.strip_prefix("wss://") {
        rest
    } else if let Some(rest) = trimmed.strip_prefix("http://") {
        rest
    } else if let Some(rest) = trimmed.strip_prefix("https://") {
        rest
    } else {
        trimmed
    };

    let (host_port, path) = match without_scheme.find('/') {
        Some(idx) => (&without_scheme[..idx], &without_scheme[idx..]),
        None => (without_scheme, "/"),
    };

    let (host, port) = match host_port.find(':') {
        Some(idx) => {
            let h = &host_port[..idx];
            let p: u16 = host_port[idx + 1..]
                .parse()
                .map_err(|e| anyhow::anyhow!("Invalid port in WebSocket URL '{}': {}", url, e))?;
            (h.to_string(), p)
        }
        None => (host_port.to_string(), 9222),
    };

    let host = if host.is_empty() {
        "127.0.0.1".to_string()
    } else {
        host
    };

    Ok((host, port, path.to_string()))
}

/// Finds a subsequence in a byte slice.
pub fn find_subsequence(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

/// Generates standard instructions on launching Chrome with remote debugging.
pub fn cdp_startup_advice(cdp_url: &str) -> String {
    format!(
        "### Chrome DevTools Protocol (CDP) Not Detected\n\
         Could not establish a connection to Chrome DevTools Protocol at `{cdp_url}`.\n\n\
         To enable full interactive automation (element clicking, text typing, JavaScript evaluation, visual screenshots):\n\
         Launch Google Chrome or Chromium with remote debugging enabled:\n\n\
         ```bash\n\
         # macOS\n\
         /Applications/Google\\ Chrome.app/Contents/MacOS/Google\\ Chrome --remote-debugging-port=9222 --headless=new\n\n\
         # Linux / WSL\n\
         google-chrome --remote-debugging-port=9222 --headless=new\n\n\
         # Windows (Command Prompt / PowerShell)\n\
         chrome.exe --remote-debugging-port=9222 --headless=new\n\
         ```\n"
    )
}

/// Extracts title and readable DOM text from HTML markup.
pub fn extract_dom_text_and_title(html: &str) -> (String, String) {
    let title = extract_title(html).unwrap_or_else(|| "Untitled Page".to_string());
    let clean_text = html_to_readable_text(html);
    (title, clean_text)
}

/// Extracts page title from `<title>...</title>`.
pub fn extract_title(html: &str) -> Option<String> {
    let lower = html.to_ascii_lowercase();
    let start_tag = "<title>";
    let end_tag = "</title>";

    let start = lower.find(start_tag)? + start_tag.len();
    let end = lower[start..].find(end_tag)? + start;

    let raw = &html[start..end];
    let decoded = unescape_html_entities(raw);
    let trimmed = decoded.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

/// Converts raw HTML string into clean, readable plain text.
pub fn html_to_readable_text(html: &str) -> String {
    let stripped = strip_non_content_tags(html);
    let mut out = String::with_capacity(stripped.len());
    let mut in_tag = false;
    let mut last_char_ws = false;

    let mut chars = stripped.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '<' {
            in_tag = true;
            let mut tag_name = String::new();
            while let Some(&c) = chars.peek() {
                if c == '>' || c.is_ascii_whitespace() {
                    break;
                }
                tag_name.push(chars.next().unwrap());
            }
            let tag_lower = tag_name.to_ascii_lowercase();
            if tag_lower == "p"
                || tag_lower == "br"
                || tag_lower == "div"
                || tag_lower == "tr"
                || tag_lower.starts_with('h')
            {
                if !out.ends_with('\n') {
                    out.push('\n');
                }
            } else if tag_lower == "li" {
                if !out.ends_with('\n') {
                    out.push('\n');
                }
                out.push_str("- ");
            }
        } else if ch == '>' {
            in_tag = false;
        } else if !in_tag {
            if ch.is_whitespace() {
                if !last_char_ws {
                    out.push(' ');
                    last_char_ws = true;
                }
            } else {
                out.push(ch);
                last_char_ws = false;
            }
        }
    }

    let unescaped = unescape_html_entities(&out);
    normalize_newlines(&unescaped)
}

/// Strips `<script>`, `<style>`, `<noscript>`, `<svg>`, and HTML comments.
pub fn strip_non_content_tags(html: &str) -> String {
    let mut out = String::with_capacity(html.len());
    let mut chars = html.chars().peekable();

    while let Some(ch) = chars.next() {
        if ch == '<' {
            if let Some(&'!') = chars.peek() {
                let rest: String = chars.clone().take(3).collect();
                if rest.starts_with("--") {
                    chars.next();
                    chars.next();
                    while let Some(c) = chars.next() {
                        if c == '-' {
                            let mut lookahead = chars.clone();
                            if lookahead.next() == Some('-') && lookahead.next() == Some('>') {
                                chars.next();
                                chars.next();
                                break;
                            }
                        }
                    }
                    continue;
                }
            }

            let mut tag_name = String::new();
            let mut tag_full = String::from("<");
            while let Some(&c) = chars.peek() {
                tag_full.push(c);
                chars.next();
                if c == '>' {
                    break;
                }
                if c.is_ascii_whitespace() && tag_name.is_empty() {
                    continue;
                }
                if !c.is_ascii_whitespace() && tag_name.len() < 12 {
                    tag_name.push(c);
                }
            }

            let tag_lower = tag_name.to_ascii_lowercase();
            if tag_lower == "script"
                || tag_lower == "style"
                || tag_lower == "noscript"
                || tag_lower == "svg"
                || tag_lower == "canvas"
            {
                let close_tag = format!("</{}>", tag_lower);
                while let Some(c) = chars.next() {
                    if c == '<' {
                        let mut lookahead = chars.clone();
                        let check: String = std::iter::once('<')
                            .chain(lookahead.by_ref().take(close_tag.len() - 1))
                            .collect();
                        if check.eq_ignore_ascii_case(&close_tag) {
                            for _ in 0..close_tag.len() - 1 {
                                chars.next();
                            }
                            break;
                        }
                    }
                }
                continue;
            }

            out.push_str(&tag_full);
        } else {
            out.push(ch);
        }
    }

    out
}

/// Unescapes common HTML entities.
pub fn unescape_html_entities(s: &str) -> String {
    let mut res = s.replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&apos;", "'")
        .replace("&nbsp;", " ")
        .replace("&copy;", "©")
        .replace("&mdash;", "—")
        .replace("&ndash;", "–")
        .replace("&#x27;", "'");

    // Handle numeric entities &#123;
    let mut final_str = String::with_capacity(res.len());
    let mut iter = res.chars().peekable();
    while let Some(c) = iter.next() {
        if c == '&' && iter.peek() == Some(&'#') {
            iter.next();
            let mut num_str = String::new();
            while let Some(&nc) = iter.peek() {
                if nc == ';' {
                    iter.next();
                    break;
                }
                if nc.is_ascii_digit() {
                    num_str.push(nc);
                    iter.next();
                } else {
                    break;
                }
            }
            if let Ok(code) = num_str.parse::<u32>() {
                if let Some(ch) = char::from_u32(code) {
                    final_str.push(ch);
                    continue;
                }
            }
            final_str.push('&');
            final_str.push('#');
            final_str.push_str(&num_str);
        } else {
            final_str.push(c);
        }
    }
    final_str
}

/// Normalizes consecutive newlines so there are at most two in a row.
pub fn normalize_newlines(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut newline_count = 0;

    for line in s.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            if newline_count < 1 && !out.is_empty() {
                out.push('\n');
                newline_count += 1;
            }
        } else {
            if !out.is_empty() && newline_count == 0 {
                out.push('\n');
            }
            out.push_str(trimmed);
            newline_count = 0;
        }
    }

    out.trim().to_string()
}

/// Truncates content if it exceeds `max_chars`.
pub fn truncate_content(content: &str, max_chars: usize) -> String {
    if content.len() <= max_chars {
        content.to_string()
    } else {
        let mut truncated = content[..max_chars].to_string();
        truncated.push_str("\n\n... [Content truncated for display]");
        truncated
    }
}

/// Encodes raw bytes into standard RFC 4648 base64 string.
pub fn base64_encode(data: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity((data.len() + 2) / 3 * 4);
    for chunk in data.chunks(3) {
        let b0 = chunk[0];
        let b1 = chunk.get(1).copied().unwrap_or(0);
        let b2 = chunk.get(2).copied().unwrap_or(0);

        out.push(TABLE[(b0 >> 2) as usize] as char);
        out.push(TABLE[(((b0 & 0x03) << 4) | (b1 >> 4)) as usize] as char);
        if chunk.len() > 1 {
            out.push(TABLE[(((b1 & 0x0F) << 2) | (b2 >> 6)) as usize] as char);
        } else {
            out.push('=');
        }
        if chunk.len() > 2 {
            out.push(TABLE[(b2 & 0x3F) as usize] as char);
        } else {
            out.push('=');
        }
    }
    out
}

/// Decodes standard RFC 4648 base64 string into raw bytes.
pub fn base64_decode(input: &str) -> Option<Vec<u8>> {
    let clean: Vec<u8> = input
        .bytes()
        .filter(|b| !b.is_ascii_whitespace())
        .collect();
    if clean.is_empty() {
        return Some(Vec::new());
    }
    if clean.len() % 4 != 0 {
        return None;
    }

    fn val(c: u8) -> Option<u8> {
        match c {
            b'A'..=b'Z' => Some(c - b'A'),
            b'a'..=b'z' => Some(c - b'a' + 26),
            b'0'..=b'9' => Some(c - b'0' + 52),
            b'+' => Some(62),
            b'/' => Some(63),
            _ => None,
        }
    }

    let mut out = Vec::with_capacity((clean.len() / 4) * 3);
    for chunk in clean.chunks_exact(4) {
        let c0 = val(chunk[0])?;
        let c1 = val(chunk[1])?;

        if chunk[2] == b'=' {
            if chunk[3] != b'=' {
                return None;
            }
            out.push((c0 << 2) | (c1 >> 4));
        } else if chunk[3] == b'=' {
            let c2 = val(chunk[2])?;
            out.push((c0 << 2) | (c1 >> 4));
            out.push(((c1 & 0x0F) << 4) | (c2 >> 2));
        } else {
            let c2 = val(chunk[2])?;
            let c3 = val(chunk[3])?;
            out.push((c0 << 2) | (c1 >> 4));
            out.push(((c1 & 0x0F) << 4) | (c2 >> 2));
            out.push(((c2 & 0x03) << 6) | c3);
        }
    }
    Some(out)
}
