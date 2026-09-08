//! Local subscription daemon probing and authentication engine.
//!
//! Provides zero-configuration discovery and routing for:
//! 1. **Antigravity Tools** daemon (`127.0.0.1:8045`) for zero-cost Claude Opus, Sonnet, and Gemini models.
//! 2. **Codex** CLI credentials (`~/.codex/auth.json`) for zero-cost ChatGPT Plus/Pro inference.

use std::net::{SocketAddr, TcpStream};
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use tracing::{debug, info, warn};

use crate::provider::catalog::CatalogModel;

/// Default loopback port for the Antigravity Tools proxy daemon.
pub const DEFAULT_ANTIGRAVITY_PORT: u16 = 8045;

/// Default base URL for Antigravity Tools loopback proxy.
pub const DEFAULT_ANTIGRAVITY_BASE_URL: &str = "http://127.0.0.1:8045/v1";

/// Default base URL for ChatGPT Codex backend API.
pub const DEFAULT_CODEX_BASE_URL: &str = "https://chatgpt.com/backend-api/codex";

/// OAuth token endpoint for Codex token refresh.
pub const CODEX_OAUTH_TOKEN_URL: &str = "https://auth.openai.com/oauth/token";

/// Public OAuth client ID used by the Codex CLI.
pub const CODEX_CLIENT_ID: &str = "app_EMoamEEZ73f0CkXaXp7hrann";

/// Default TCP connection timeout for local daemon port probing.
pub const DEFAULT_PROBE_TIMEOUT: Duration = Duration::from_millis(800);

/// Represents a discovered local daemon endpoint or credential set.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LocalDaemonEndpoint {
    /// Provider identifier (e.g. `"antigravity"` or `"codex"`).
    pub provider: String,
    /// Base URL for inference requests (e.g. `"http://127.0.0.1:8045/v1"`).
    pub base_url: String,
    /// API key or bearer access token.
    pub api_key: String,
    /// Optional account ID (used for Codex organization/account routing).
    pub account_id: Option<String>,
    /// Whether the endpoint is currently alive and responsive.
    pub is_alive: bool,
}

impl LocalDaemonEndpoint {
    /// Creates a new local daemon endpoint descriptor.
    pub fn new(
        provider: impl Into<String>,
        base_url: impl Into<String>,
        api_key: impl Into<String>,
        account_id: Option<String>,
        is_alive: bool,
    ) -> Self {
        Self {
            provider: provider.into(),
            base_url: base_url.into(),
            api_key: api_key.into(),
            account_id,
            is_alive,
        }
    }
}

// ---------------------------------------------------------------------------
// Antigravity Tools Configuration Format
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct AntigravityGuiConfig {
    #[serde(default)]
    pub proxy: Option<AntigravityProxyConfig>,
    #[serde(default)]
    pub auto_launch: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct AntigravityProxyConfig {
    #[serde(default)]
    pub enabled: Option<bool>,
    #[serde(default)]
    pub allow_lan_access: Option<bool>,
    #[serde(default)]
    pub auth_mode: Option<String>,
    #[serde(default)]
    pub port: Option<u16>,
    #[serde(default)]
    pub api_key: Option<String>,
}

/// Parses the JSON content of an Antigravity Tools `gui_config.json` file.
pub fn parse_antigravity_config(content: &str) -> Result<AntigravityGuiConfig, serde_json::Error> {
    serde_json::from_str(content)
}

// ---------------------------------------------------------------------------
// Codex CLI Authentication Format
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct CodexAuthFile {
    #[serde(default)]
    pub auth_mode: Option<String>,
    #[serde(default, rename = "OPENAI_API_KEY")]
    pub openai_api_key: Option<String>,
    #[serde(default)]
    pub tokens: Option<CodexTokens>,
    #[serde(default)]
    pub last_refresh: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct CodexTokens {
    #[serde(default)]
    pub id_token: Option<String>,
    #[serde(default)]
    pub access_token: Option<String>,
    #[serde(default)]
    pub refresh_token: Option<String>,
    #[serde(default)]
    pub account_id: Option<String>,
}

/// Parses the JSON content of a Codex `auth.json` file.
pub fn parse_codex_auth(content: &str) -> Result<CodexAuthFile, serde_json::Error> {
    serde_json::from_str(content)
}

// ---------------------------------------------------------------------------
// Path Resolution Helpers
// ---------------------------------------------------------------------------

/// Returns the path to the Antigravity Tools GUI config file, checking
/// `ANTIGRAVITY_CONFIG_PATH` env var first, then `~/.antigravity_tools/gui_config.json`.
pub fn default_antigravity_config_path() -> Option<PathBuf> {
    if let Ok(env_path) = std::env::var("ANTIGRAVITY_CONFIG_PATH") {
        let p = PathBuf::from(env_path);
        if p.exists() {
            return Some(p);
        }
    }
    dirs::home_dir().map(|h| h.join(".antigravity_tools").join("gui_config.json"))
}

/// Returns the path to the Codex auth file, checking `CODEX_AUTH_PATH` env var first,
/// then `~/.codex/auth.json`.
pub fn default_codex_auth_path() -> Option<PathBuf> {
    if let Ok(env_path) = std::env::var("CODEX_AUTH_PATH") {
        let p = PathBuf::from(env_path);
        if p.exists() {
            return Some(p);
        }
    }
    dirs::home_dir().map(|h| h.join(".codex").join("auth.json"))
}

// ---------------------------------------------------------------------------
// Network & Port Probing
// ---------------------------------------------------------------------------

/// Checks if a TCP port on localhost (`127.0.0.1`) is accessible.
pub fn check_port_accessible(port: u16, timeout: Duration) -> bool {
    let addr = SocketAddr::from(([127, 0, 0, 1], port));
    TcpStream::connect_timeout(&addr, timeout).is_ok()
}

// ---------------------------------------------------------------------------
// Detection Functions
// ---------------------------------------------------------------------------

/// Detects the local Antigravity Tools daemon from `~/.antigravity_tools/gui_config.json`.
///
/// Reads the proxy port (defaults to 8045) and `api_key`. Checks if the port is
/// accessible, and returns `Some(LocalDaemonEndpoint)` with
/// `base_url = format!("http://127.0.0.1:{}/v1", port)`.
pub fn detect_antigravity_daemon() -> Option<LocalDaemonEndpoint> {
    let path = default_antigravity_config_path()?;
    detect_antigravity_daemon_at(&path)
}

/// Detects the Antigravity Tools daemon from a specific configuration file path.
pub fn detect_antigravity_daemon_at(path: &Path) -> Option<LocalDaemonEndpoint> {
    if !path.exists() {
        debug!("Antigravity config not found at {}", path.display());
        return None;
    }

    let content = match std::fs::read_to_string(path) {
        Ok(c) => c,
        Err(e) => {
            warn!("Failed to read Antigravity config {}: {e}", path.display());
            return None;
        }
    };

    let config = match parse_antigravity_config(&content) {
        Ok(c) => c,
        Err(e) => {
            warn!("Failed to parse Antigravity config {}: {e}", path.display());
            return None;
        }
    };

    let (port, api_key) = if let Some(proxy) = config.proxy {
        let port = proxy.port.unwrap_or(DEFAULT_ANTIGRAVITY_PORT);
        let api_key = proxy.api_key.unwrap_or_default();
        (port, api_key)
    } else {
        (DEFAULT_ANTIGRAVITY_PORT, String::new())
    };

    let is_alive = check_port_accessible(port, DEFAULT_PROBE_TIMEOUT);
    debug!(
        "Detected Antigravity Tools config (port: {}, is_alive: {})",
        port, is_alive
    );

    Some(LocalDaemonEndpoint {
        provider: "antigravity".to_string(),
        base_url: format!("http://127.0.0.1:{}/v1", port),
        api_key,
        account_id: None,
        is_alive,
    })
}

/// Detects Codex authentication credentials from `~/.codex/auth.json`.
///
/// Parses `tokens.access_token`, `tokens.account_id`, and `tokens.refresh_token`.
/// Returns `Some(LocalDaemonEndpoint)` with `base_url = "https://chatgpt.com/backend-api/codex"`.
pub fn detect_codex_auth() -> Option<LocalDaemonEndpoint> {
    let path = default_codex_auth_path()?;
    detect_codex_auth_at(&path)
}

/// Detects Codex authentication credentials from a specific file path.
pub fn detect_codex_auth_at(path: &Path) -> Option<LocalDaemonEndpoint> {
    if !path.exists() {
        debug!("Codex auth file not found at {}", path.display());
        return None;
    }

    let content = match std::fs::read_to_string(path) {
        Ok(c) => c,
        Err(e) => {
            warn!("Failed to read Codex auth file {}: {e}", path.display());
            return None;
        }
    };

    let auth = match parse_codex_auth(&content) {
        Ok(a) => a,
        Err(e) => {
            warn!("Failed to parse Codex auth file {}: {e}", path.display());
            return None;
        }
    };

    let tokens = auth.tokens?;
    let access_token = tokens.access_token?;
    if access_token.trim().is_empty() {
        debug!("Codex access_token is empty");
        return None;
    }

    let account_id = tokens.account_id.filter(|id| !id.trim().is_empty());

    Some(LocalDaemonEndpoint {
        provider: "codex".to_string(),
        base_url: DEFAULT_CODEX_BASE_URL.to_string(),
        api_key: access_token,
        account_id,
        is_alive: true,
    })
}

// ---------------------------------------------------------------------------
// Probing & Model Discovery
// ---------------------------------------------------------------------------

/// Probes and collects all available (live) endpoints from Antigravity Tools and Codex.
pub async fn probe_local_daemons() -> Vec<LocalDaemonEndpoint> {
    let mut endpoints = Vec::new();

    if let Some(ep) = detect_antigravity_daemon() {
        if ep.is_alive {
            info!("Local Antigravity daemon active at {}", ep.base_url);
            endpoints.push(ep);
        } else {
            debug!("Antigravity config found but port is not accessible");
        }
    }

    if let Some(ep) = detect_codex_auth() {
        if ep.is_alive {
            info!(
                "Codex auth credentials found (account: {:?})",
                ep.account_id
            );
            endpoints.push(ep);
        }
    }

    endpoints
}

/// Probes local daemons using specific configuration file paths.
pub async fn probe_local_daemons_from_paths(
    antigravity_path: Option<&Path>,
    codex_path: Option<&Path>,
) -> Vec<LocalDaemonEndpoint> {
    let mut endpoints = Vec::new();

    if let Some(path) = antigravity_path {
        if let Some(ep) = detect_antigravity_daemon_at(path) {
            if ep.is_alive {
                endpoints.push(ep);
            }
        }
    }

    if let Some(path) = codex_path {
        if let Some(ep) = detect_codex_auth_at(path) {
            if ep.is_alive {
                endpoints.push(ep);
            }
        }
    }

    endpoints
}

// ---------------------------------------------------------------------------
// Model Fetching & Parsing
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
struct OpenAiModelListResponse {
    #[serde(default)]
    data: Vec<OpenAiModelItem>,
}

#[derive(Debug, Deserialize)]
struct OpenAiModelItem {
    id: String,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    description: Option<String>,
}

/// Parses an OpenAI-compatible `/models` JSON response into a list of `CatalogModel`s.
pub fn parse_antigravity_models_json(
    content: &str,
) -> Result<Vec<CatalogModel>, serde_json::Error> {
    let parsed: OpenAiModelListResponse = serde_json::from_str(content)?;
    let models = parsed
        .data
        .into_iter()
        .map(|item| {
            let name = item.name.unwrap_or_else(|| item.id.clone());
            let mut model = CatalogModel::new(&item.id, name, "antigravity")
                .with_badge("Local Daemon (Free)")
                .with_pricing(0.0, 0.0);

            if let Some(desc) = item.description {
                model.description = Some(desc);
            }

            // Infer common capabilities from model name
            let id_lower = item.id.to_lowercase();
            if id_lower.contains("thinking")
                || id_lower.contains("opus")
                || id_lower.contains("reasoning")
            {
                model = model.with_reasoning();
            }
            if id_lower.contains("claude")
                || id_lower.contains("gemini")
                || id_lower.contains("gpt-4")
                || id_lower.contains("image")
            {
                model = model.with_vision().with_tool_use().with_function_calling();
            }

            model
        })
        .collect();

    Ok(models)
}

/// Fetches available models from the Antigravity Tools daemon.
///
/// Queries `GET {base_url}/models` with `Authorization: Bearer {api_key}`.
/// Parses `data[].id` and maps to `CatalogModel` with `provider: "antigravity"`
/// and badge `"Local Daemon (Free)"`.
pub async fn fetch_antigravity_models(endpoint: &LocalDaemonEndpoint) -> Vec<CatalogModel> {
    let url = format!("{}/models", endpoint.base_url.trim_end_matches('/'));

    let client = match reqwest::Client::builder()
        .use_rustls_tls()
        .timeout(Duration::from_secs(4))
        .build()
    {
        Ok(c) => c,
        Err(e) => {
            warn!("Failed to create reqwest client for antigravity models: {e}");
            return Vec::new();
        }
    };

    let mut req = client.get(&url);
    if !endpoint.api_key.is_empty() {
        req = req.header("Authorization", format!("Bearer {}", endpoint.api_key));
    }

    let resp = match req.send().await {
        Ok(r) if r.status().is_success() => r,
        Ok(r) => {
            warn!(
                "Failed to fetch Antigravity models from {}: status {}",
                url,
                r.status()
            );
            return Vec::new();
        }
        Err(e) => {
            warn!(
                "Network error fetching Antigravity models from {}: {e}",
                url
            );
            return Vec::new();
        }
    };

    let body = match resp.text().await {
        Ok(b) => b,
        Err(e) => {
            warn!("Failed to read Antigravity models response body: {e}");
            return Vec::new();
        }
    };

    match parse_antigravity_models_json(&body) {
        Ok(models) => {
            debug!("Fetched {} models from Antigravity daemon", models.len());
            models
        }
        Err(e) => {
            warn!("Failed to parse Antigravity models JSON from {}: {e}", url);
            Vec::new()
        }
    }
}

/// Returns the curated default models available through the local Antigravity daemon.
pub fn default_local_models() -> Vec<CatalogModel> {
    vec![
        CatalogModel::new("claude-opus-4-6", "Claude Opus 4.6", "antigravity")
            .with_context(200_000)
            .with_max_output(64_000)
            .with_badges(["Reasoning", "Thinking", "Local Daemon (Free)"])
            .with_pricing(0.0, 0.0)
            .with_description("Anthropic Claude Opus 4.6 with reasoning via local Antigravity Tools daemon (127.0.0.1:8045)"),
        CatalogModel::new("claude-sonnet-4-6", "Claude Sonnet 4.6", "antigravity")
            .with_context(200_000)
            .with_max_output(64_000)
            .with_badges(["Flagship", "Coding", "Local Daemon (Free)"])
            .with_pricing(0.0, 0.0)
            .with_description("Anthropic Claude Sonnet 4.6 via local Antigravity Tools daemon (127.0.0.1:8045)"),
        CatalogModel::new("gemini-3.8-flash-high", "Gemini 3.8 Flash High", "antigravity")
            .with_context(1_048_576)
            .with_max_output(64_000)
            .with_badges(["1M Context", "Ultra-Fast", "Local Daemon (Free)"])
            .with_pricing(0.0, 0.0)
            .with_description("Google Gemini 3.8 Flash High via local Antigravity Tools daemon (127.0.0.1:8045)"),
    ]
}

// ---------------------------------------------------------------------------
// Codex OAuth Token Refresh (Spec 2.2)
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize)]
struct RefreshTokenRequest<'a> {
    grant_type: &'a str,
    client_id: &'a str,
    refresh_token: &'a str,
}

#[derive(Debug, Deserialize)]
struct RefreshTokenResponse {
    access_token: String,
    #[serde(default)]
    refresh_token: Option<String>,
}

/// Refreshes the Codex access token using the stored refresh token in `auth.json`
/// and writes the updated tokens back to disk.
pub async fn refresh_codex_token_at(path: &Path) -> Result<LocalDaemonEndpoint, String> {
    let content = std::fs::read_to_string(path)
        .map_err(|e| format!("Failed to read {}: {e}", path.display()))?;

    let mut auth_file: CodexAuthFile = serde_json::from_str(&content)
        .map_err(|e| format!("Failed to parse {}: {e}", path.display()))?;

    let tokens = auth_file
        .tokens
        .as_ref()
        .ok_or_else(|| "No tokens object in auth.json".to_string())?;

    let refresh_token = tokens
        .refresh_token
        .as_ref()
        .ok_or_else(|| "No refresh_token found in auth.json".to_string())?;

    let client = reqwest::Client::builder()
        .use_rustls_tls()
        .timeout(Duration::from_secs(10))
        .build()
        .map_err(|e| format!("Failed to build HTTP client: {e}"))?;

    let payload = RefreshTokenRequest {
        grant_type: "refresh_token",
        client_id: CODEX_CLIENT_ID,
        refresh_token,
    };

    let resp = client
        .post(CODEX_OAUTH_TOKEN_URL)
        .json(&payload)
        .send()
        .await
        .map_err(|e| format!("OAuth refresh request failed: {e}"))?;

    if !resp.status().is_success() {
        return Err(format!("OAuth refresh returned status {}", resp.status()));
    }

    let new_tokens: RefreshTokenResponse = resp
        .json()
        .await
        .map_err(|e| format!("Failed to parse OAuth refresh response: {e}"))?;

    // Update tokens in memory
    if let Some(t) = auth_file.tokens.as_mut() {
        t.access_token = Some(new_tokens.access_token.clone());
        if let Some(rt) = new_tokens.refresh_token {
            t.refresh_token = Some(rt);
        }
    }
    auth_file.last_refresh = Some(chrono_iso_now());

    // Write back to disk
    let updated_json = serde_json::to_string_pretty(&auth_file)
        .map_err(|e| format!("Failed to serialize updated auth: {e}"))?;
    std::fs::write(path, updated_json)
        .map_err(|e| format!("Failed to write updated auth to {}: {e}", path.display()))?;

    let account_id = auth_file.tokens.as_ref().and_then(|t| t.account_id.clone());

    Ok(LocalDaemonEndpoint {
        provider: "codex".to_string(),
        base_url: DEFAULT_CODEX_BASE_URL.to_string(),
        api_key: new_tokens.access_token,
        account_id,
        is_alive: true,
    })
}

fn chrono_iso_now() -> String {
    // Basic UTC ISO-8601 timestamp without extra dependencies
    use std::time::SystemTime;
    let now = SystemTime::now();
    let dur = now
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or_default();
    format!("{}.{}Z", dur.as_secs(), dur.subsec_millis())
}
