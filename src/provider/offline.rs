//! Automatic Offline Mode Detection and Seamless Local Ollama Transition.
//!
//! This module provides automatic network connectivity probing, local Ollama backend detection,
//! intelligent local model scoring and ranking (prioritizing high-capability coding models),
//! and seamless configuration switching when the host machine loses internet access.
//!
//! # Key Capabilities
//!
//! - **Fast Connectivity Probing**: Multi-target TCP socket probes to public DNS resolvers
//!   (`1.1.1.1:53`, `8.8.8.8:53`, `9.9.9.9:53`) and HTTP endpoints with configurable sub-second timeouts.
//! - **Local Ollama Ping & Health Check**: Probing `127.0.0.1:11434` (or custom base URL) via TCP socket
//!   and HTTP `/api/tags` to verify Ollama daemon responsiveness.
//! - **Intelligent Model Selection**: Scores available local models by family suitability (e.g. `qwen2.5-coder`,
//!   `deepseek-coder`, `codellama`, `deepseek-r1`, `llama3.3`), parameter sizes (`70b` > `32b` > `14b` > `7b` > `3b`),
//!   quantization levels, and in-memory residency (`/api/ps`).
//! - **Seamless Config Cutover**: Updates [`Config::default_provider`] to `"ollama"`, sets [`Config::default_model`]
//!   to the best scored local model, ensures [`Config::ollama_base_url`] is configured, and provides clear user notices.
//! - **Mockable Architecture**: Pluggable [`ConnectivityProber`] and [`OllamaProber`] traits for deterministic unit testing.

use std::cmp::Ordering;
use std::net::{TcpStream, ToSocketAddrs};
use std::time::Duration;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use tracing::{debug, info, warn};

use crate::config::Config;
use crate::provider::ollama::{OllamaModelInfo, OllamaTagsResponse};

/// Default loopback host for local Ollama instances.
pub const DEFAULT_OLLAMA_HOST: &str = "127.0.0.1";

/// Default port for local Ollama instances.
pub const DEFAULT_OLLAMA_PORT: u16 = 11434;

/// Default socket address string for local Ollama.
pub const DEFAULT_OLLAMA_SOCKET_ADDR: &str = "127.0.0.1:11434";
pub const DEFAULT_OLLAMA_ADDR: &str = DEFAULT_OLLAMA_SOCKET_ADDR;

/// Default HTTP base URL for local Ollama.
pub const DEFAULT_OLLAMA_URL: &str = "http://127.0.0.1:11434";

/// Default public DNS probe socket addresses (Cloudflare, Google, Quad9, OpenDNS).
pub const DEFAULT_DNS_PROBES: &[&str] = &[
    "1.1.1.1:53",
    "8.8.8.8:53",
    "9.9.9.9:53",
    "208.67.222.222:53",
];

/// Default HTTP connectivity probe endpoints.
pub const DEFAULT_HTTP_PROBES: &[&str] = &[
    "http://1.1.1.1",
    "http://connectivitycheck.gstatic.com/generate_204",
];

/// Default socket connect timeout for internet reachability checks.
pub const DEFAULT_CONNECTIVITY_TIMEOUT: Duration = Duration::from_millis(800);

/// Default ping timeout for local Ollama reachability checks.
pub const DEFAULT_OLLAMA_PING_TIMEOUT: Duration = Duration::from_millis(1200);

/// Preferred fallback model when Ollama is running but model catalog is empty.
pub const DEFAULT_OFFLINE_MODEL_FALLBACK: &str = "qwen2.5-coder";

// ============================================================================
// Network Status & Transition Types
// ============================================================================

/// Network reachability and environment status.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum NetworkEnvironmentStatus {
    /// Internet connection is available and external providers are reachable.
    Online,
    /// Internet connection is unavailable or external probes failed.
    Offline,
}

/// Reason triggering an offline transition or detection.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum OfflineReason {
    /// No external DNS/socket probes responded within timeout.
    NoInternetConnection,
    /// DNS host resolution failed for external endpoints.
    DnsResolutionFailure,
    /// External probe requests timed out.
    ProbeTimeout,
    /// The user explicitly requested or configured offline mode.
    ManualOfflineEnforced,
    /// The active remote provider endpoint failed to connect.
    ProviderUnreachable { provider: String, details: String },
}

impl std::fmt::Display for OfflineReason {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NoInternetConnection => write!(f, "no internet connection detected"),
            Self::DnsResolutionFailure => write!(f, "external DNS resolution failed"),
            Self::ProbeTimeout => write!(f, "external network probes timed out"),
            Self::ManualOfflineEnforced => write!(f, "offline mode explicitly requested"),
            Self::ProviderUnreachable { provider, details } => {
                write!(f, "provider '{provider}' unreachable: {details}")
            }
        }
    }
}

/// Result of evaluating offline status and performing automatic configuration transition.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum OfflineTransitionResult {
    /// Network is online; kept current provider and model configuration unchanged.
    StayedOnline { provider: String, model: String },
    /// Offline detected; successfully switched default provider to Ollama with the best local model.
    SwitchedToOllama {
        previous_provider: String,
        previous_model: String,
        selected_model: String,
        available_models: Vec<String>,
        reason: OfflineReason,
        notice: String,
    },
    /// The active provider was already Ollama; no provider transition needed.
    AlreadyOllama {
        model: String,
        available_models: Vec<String>,
    },
    /// Offline detected, but local Ollama is not running or unreachable on the configured host/port.
    OfflineNoLocalBackend {
        reason: OfflineReason,
        attempted_url: String,
        remediation: String,
    },
}

impl OfflineTransitionResult {
    /// Returns true if the configuration was modified / switched to Ollama.
    pub fn is_switched(&self) -> bool {
        matches!(self, Self::SwitchedToOllama { .. })
    }

    /// Returns true if the session is running with a working local Ollama provider.
    pub fn is_offline_ready(&self) -> bool {
        matches!(
            self,
            Self::SwitchedToOllama { .. } | Self::AlreadyOllama { .. }
        )
    }

    /// Returns a human-readable summary message suitable for logging or terminal display.
    pub fn message(&self) -> &str {
        match self {
            Self::StayedOnline { .. } => "Internet connected; keeping current provider.",
            Self::SwitchedToOllama { notice, .. } => notice.as_str(),
            Self::AlreadyOllama { .. } => "Already using local Ollama provider.",
            Self::OfflineNoLocalBackend { remediation, .. } => remediation.as_str(),
        }
    }

    /// Builds structured, actionable user guidance for this transition outcome.
    ///
    /// Tells the user exactly what to do next: nothing (online), continue with
    /// the selected local model, or start/pull Ollama models when no local
    /// backend is available.
    pub fn offline_guidance(&self) -> OfflineGuidance {
        match self {
            Self::StayedOnline { .. } => OfflineGuidance {
                headline: "Internet connection is available; remote providers are reachable."
                    .to_string(),
                steps: Vec::new(),
            },
            Self::SwitchedToOllama {
                selected_model,
                available_models,
                ..
            } => OfflineGuidance {
                headline: format!(
                    "Working offline via local Ollama with model '{selected_model}'."
                ),
                steps: if available_models.is_empty() {
                    vec![format!(
                        "No models downloaded yet — run 'ollama pull {}' for a capable coding model.",
                        DEFAULT_OFFLINE_MODEL_FALLBACK
                    )]
                } else {
                    Vec::new()
                },
            },
            Self::AlreadyOllama { model, .. } => OfflineGuidance {
                headline: format!("Continuing offline with local Ollama model '{model}'."),
                steps: Vec::new(),
            },
            Self::OfflineNoLocalBackend { attempted_url, .. } => OfflineGuidance {
                headline: format!(
                    "No internet connection and no local Ollama daemon at '{attempted_url}'."
                ),
                steps: vec![
                    "Install Ollama if needed: https://ollama.com/download".to_string(),
                    "Start the local daemon: 'ollama serve'".to_string(),
                    format!("Pull a coding model: 'ollama pull {DEFAULT_OFFLINE_MODEL_FALLBACK}'"),
                    "Re-run Fusion; it will detect Ollama and switch automatically.".to_string(),
                ],
            },
        }
    }
}

/// Structured, actionable guidance for the user when the network is unavailable.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OfflineGuidance {
    /// Short headline describing the current network/offline state.
    pub headline: String,
    /// Ordered, user-executable steps to restore or continue working.
    pub steps: Vec<String>,
}

impl OfflineGuidance {
    /// Returns true if the guidance implies the user must take action to continue.
    pub fn requires_action(&self) -> bool {
        !self.steps.is_empty()
    }
}

impl std::fmt::Display for OfflineGuidance {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        writeln!(f, "{}", self.headline)?;
        for (i, step) in self.steps.iter().enumerate() {
            writeln!(f, "  {}. {}", i + 1, step)?;
        }
        Ok(())
    }
}

// ============================================================================
// Detector Configuration
// ============================================================================

/// Configuration parameters for the offline mode detector.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OfflineDetectorConfig {
    /// Base URL for the local Ollama daemon. Defaults to `http://127.0.0.1:11434`.
    pub ollama_url: String,
    /// Timeout for external internet connectivity probes.
    pub connectivity_timeout: Duration,
    /// Timeout for local Ollama ping and model catalog queries.
    pub ollama_timeout: Duration,
    /// List of TCP socket probe endpoints for internet reachability.
    pub socket_probes: Vec<String>,
    /// Fallback model name when Ollama returns an empty model catalog.
    pub fallback_model: String,
    /// List of prioritized model family patterns in order of preference.
    pub preferred_model_families: Vec<String>,
    /// When true, skip internet probing entirely and always evaluate the local
    /// Ollama backend (explicit offline mode requested by the user).
    #[serde(default)]
    pub manual_offline_enforced: bool,
}

impl Default for OfflineDetectorConfig {
    fn default() -> Self {
        Self {
            ollama_url: DEFAULT_OLLAMA_URL.to_string(),
            connectivity_timeout: DEFAULT_CONNECTIVITY_TIMEOUT,
            ollama_timeout: DEFAULT_OLLAMA_PING_TIMEOUT,
            socket_probes: DEFAULT_DNS_PROBES.iter().map(|s| s.to_string()).collect(),
            fallback_model: DEFAULT_OFFLINE_MODEL_FALLBACK.to_string(),
            preferred_model_families: vec![
                "qwen2.5-coder".to_string(),
                "qwen-coder".to_string(),
                "deepseek-coder".to_string(),
                "deepseek-r1".to_string(),
                "codellama".to_string(),
                "starcoder2".to_string(),
                "codeqwen".to_string(),
                "llama3.3".to_string(),
                "llama3.2".to_string(),
                "llama3.1".to_string(),
                "llama3".to_string(),
                "phi4".to_string(),
                "phi3.5".to_string(),
                "gemma2".to_string(),
            ],
            manual_offline_enforced: false,
        }
    }
}

// ============================================================================
// Connectivity Prober Abstraction
// ============================================================================

/// Trait for probing network reachability, enabling mock injections in tests.
pub trait ConnectivityProber: Send + Sync {
    /// Returns true if external internet connectivity is detected.
    fn is_online(&self, timeout: Duration) -> bool;
}

/// Standard connectivity prober testing external TCP socket connectivity to known DNS IPs.
#[derive(Debug, Clone, Default)]
pub struct DefaultConnectivityProber {
    probes: Vec<String>,
}

impl DefaultConnectivityProber {
    pub fn new(probes: Vec<String>) -> Self {
        Self { probes }
    }
}

impl ConnectivityProber for DefaultConnectivityProber {
    fn is_online(&self, timeout: Duration) -> bool {
        let targets = if self.probes.is_empty() {
            DEFAULT_DNS_PROBES.to_vec()
        } else {
            self.probes.iter().map(|s| s.as_str()).collect()
        };

        for target in targets {
            if let Ok(mut addrs) = target.to_socket_addrs() {
                if let Some(addr) = addrs.next() {
                    if TcpStream::connect_timeout(&addr, timeout).is_ok() {
                        debug!("Internet connectivity probe succeeded via {}", target);
                        return true;
                    }
                }
            }
        }
        false
    }
}

/// Mock connectivity prober for unit testing.
#[derive(Debug, Clone)]
pub struct MockConnectivityProber {
    pub online: bool,
}

impl ConnectivityProber for MockConnectivityProber {
    fn is_online(&self, _timeout: Duration) -> bool {
        self.online
    }
}

// ============================================================================
// Ollama Prober Abstraction
// ============================================================================

/// Trait for querying local Ollama instance reachability and models.
#[async_trait::async_trait]
pub trait OllamaProber: Send + Sync {
    /// Ping the local Ollama instance to check reachability.
    async fn ping(&self, base_url: &str, timeout: Duration) -> bool;

    /// Retrieve the list of downloaded local models from Ollama.
    async fn list_models(&self, base_url: &str, timeout: Duration) -> Result<Vec<OllamaModelInfo>>;

    /// Synchronous ping for fast synchronous checks.
    fn ping_sync(&self, base_url: &str, timeout: Duration) -> bool;
}

/// Default Ollama prober using HTTP client requests.
#[derive(Debug, Clone, Default)]
pub struct DefaultOllamaProber;

#[async_trait::async_trait]
impl OllamaProber for DefaultOllamaProber {
    async fn ping(&self, base_url: &str, timeout: Duration) -> bool {
        ping_ollama_endpoint_internal(base_url, timeout).await
    }

    async fn list_models(&self, base_url: &str, timeout: Duration) -> Result<Vec<OllamaModelInfo>> {
        let url = format!("{}/api/tags", base_url.trim_end_matches('/'));
        let client = reqwest::Client::builder()
            .timeout(timeout)
            .connect_timeout(timeout)
            .build()
            .context("Failed to build reqwest client for Ollama probing")?;

        let resp = client
            .get(&url)
            .send()
            .await
            .context("Failed to send GET request to Ollama /api/tags")?;

        if !resp.status().is_success() {
            anyhow::bail!("Ollama /api/tags returned HTTP {}", resp.status());
        }

        let tags: OllamaTagsResponse = resp
            .json()
            .await
            .context("Failed to parse Ollama /api/tags JSON response")?;

        Ok(tags.models)
    }

    fn ping_sync(&self, base_url: &str, timeout: Duration) -> bool {
        ping_ollama_socket_sync(base_url, timeout)
    }
}

/// Mock Ollama prober for unit testing.
#[derive(Debug, Clone)]
pub struct MockOllamaProber {
    pub reachable: bool,
    pub models: Vec<OllamaModelInfo>,
}

#[async_trait::async_trait]
impl OllamaProber for MockOllamaProber {
    async fn ping(&self, _base_url: &str, _timeout: Duration) -> bool {
        self.reachable
    }

    async fn list_models(
        &self,
        _base_url: &str,
        _timeout: Duration,
    ) -> Result<Vec<OllamaModelInfo>> {
        if self.reachable {
            Ok(self.models.clone())
        } else {
            anyhow::bail!("Mock Ollama unreachable")
        }
    }

    fn ping_sync(&self, _base_url: &str, _timeout: Duration) -> bool {
        self.reachable
    }
}

// ============================================================================
// Model Scoring & Best Selection Heuristics
// ============================================================================

/// Evaluates and scores an Ollama model info entry to rank the best model for coding tasks.
///
/// Higher scores represent higher preference.
///
/// Factors evaluated:
/// 1. Known coding specializations (`qwen2.5-coder`, `deepseek-coder`, `codellama`, `starcoder2`).
/// 2. General reasoning strength (`deepseek-r1`, `llama3.3`, `llama3.2`, `llama3.1`, `phi4`, `mistral`).
/// 3. Parameter count (extracted from model name tags like `:32b`, `:14b`, `:8b`, `:7b`, `:3b`, `:1.5b`
///    or `details.parameter_size`).
/// 4. Quantization preference (`Q8` > `Q6` > `Q4_K_M` > `Q4` > `Q2`).
/// 5. In-VRAM active model residency (`size_vram > 0`).
pub fn score_ollama_model(model: &OllamaModelInfo) -> f64 {
    let name_lower = model.name.to_lowercase();
    let clean_name = name_lower.trim();

    let mut score = 10.0; // Base score for any existing model

    // 1. Model Family Suitability
    if clean_name.contains("qwen2.5-coder") || clean_name.contains("qwen2.5_coder") {
        score += 1000.0;
    } else if clean_name.contains("qwen-coder") || clean_name.contains("codeqwen") {
        score += 850.0;
    } else if clean_name.contains("deepseek-coder") {
        score += 800.0;
    } else if clean_name.contains("deepseek-r1") || clean_name.contains("deepseek_r1") {
        score += 750.0;
    } else if clean_name.contains("starcoder") {
        score += 700.0;
    } else if clean_name.contains("codellama") || clean_name.contains("code-llama") {
        score += 680.0;
    } else if clean_name.contains("wizardcoder") {
        score += 650.0;
    } else if clean_name.contains("llama3.3") || clean_name.contains("llama-3.3") {
        score += 600.0;
    } else if clean_name.contains("llama3.2") || clean_name.contains("llama-3.2") {
        score += 550.0;
    } else if clean_name.contains("llama3.1") || clean_name.contains("llama-3.1") {
        score += 520.0;
    } else if clean_name.contains("llama3") || clean_name.contains("llama-3") {
        score += 480.0;
    } else if clean_name.contains("phi4") || clean_name.contains("phi-4") {
        score += 460.0;
    } else if clean_name.contains("qwen2.5") {
        score += 440.0;
    } else if clean_name.contains("phi3.5") || clean_name.contains("phi3") {
        score += 420.0;
    } else if clean_name.contains("mistral-nemo") {
        score += 400.0;
    } else if clean_name.contains("mistral") || clean_name.contains("mixtral") {
        score += 380.0;
    } else if clean_name.contains("gemma2") || clean_name.contains("gemma-2") {
        score += 360.0;
    } else if clean_name.contains("code") || clean_name.contains("coder") {
        score += 300.0;
    }

    // 2. Parameter Size Extraction and Scoring
    let param_billion = extract_parameter_billions(model);
    if let Some(billions) = param_billion {
        // Optimal coding sweet-spot for local developer workstations: 7B - 32B
        // Models over 70B can be slow without large VRAM, but still high capability
        if (6.0..=35.0).contains(&billions) {
            score += billions * 5.0; // e.g. 32b -> +160, 14b -> +70, 7b -> +35
        } else if billions > 35.0 {
            score += 150.0 + (billions * 1.0); // e.g. 70b -> +220
        } else {
            // Smaller models (1.5b, 3b)
            score += billions * 4.0;
        }
    }

    // 3. Quantization bonus
    if let Some(details) = &model.details {
        if let Some(quant) = &details.quantization_level {
            let q_upper = quant.to_uppercase();
            if q_upper.contains("Q8") || q_upper.contains("FP16") {
                score += 30.0;
            } else if q_upper.contains("Q6") {
                score += 20.0;
            } else if q_upper.contains("Q5") {
                score += 15.0;
            } else if q_upper.contains("Q4_K_M") || q_upper.contains("Q4_1") {
                score += 10.0;
            } else if q_upper.contains("Q4") {
                score += 5.0;
            }
        }
    }

    // 4. In-VRAM active loaded bonus (instant response without cold-start model load)
    if let Some(vram) = model.size_vram {
        if vram > 0 {
            score += 40.0;
        }
    }

    // 5. Tie-breaker by download file size if available (larger weights usually mean higher fidelity)
    if let Some(size) = model.size {
        score += (size as f64) / 1_000_000_000_000.0; // Sub-point tiebreaker
    }

    score
}

/// Attempts to parse the model parameter size in billions (e.g. `32b` -> 32.0, `1.5b` -> 1.5).
fn extract_parameter_billions(model: &OllamaModelInfo) -> Option<f64> {
    // First check explicit details.parameter_size
    if let Some(details) = &model.details {
        if let Some(param_str) = &details.parameter_size {
            if let Some(parsed) = parse_param_string(param_str) {
                return Some(parsed);
            }
        }
    }

    // Next parse model name and tags (e.g., "qwen2.5-coder:32b", "deepseek-r1:14b-q4_K_M")
    let name = &model.name;
    if let Some(tag) = name.split(':').nth(1) {
        if let Some(parsed) = parse_param_string(tag) {
            return Some(parsed);
        }
    }

    // Check full name parts
    for part in name.split(|c: char| !c.is_alphanumeric() && c != '.') {
        if let Some(parsed) = parse_param_string(part) {
            return Some(parsed);
        }
    }

    None
}

/// Helper to parse strings like "32b", "32B", "1.5b", "70B", "7b", "0.5b".
fn parse_param_string(s: &str) -> Option<f64> {
    let lower = s.trim().to_lowercase();
    if lower.ends_with('b') {
        let num_part = lower.trim_end_matches('b');
        if let Ok(val) = num_part.parse::<f64>() {
            if (0.1..=500.0).contains(&val) {
                return Some(val);
            }
        }
    } else if lower.ends_with('m') {
        let num_part = lower.trim_end_matches('m');
        if let Ok(val) = num_part.parse::<f64>() {
            if (1.0..=100_000.0).contains(&val) {
                return Some(val / 1000.0);
            }
        }
    }
    None
}

/// Selects the best available local model from a list of [`OllamaModelInfo`].
///
/// Returns `None` if the model list is empty.
pub fn select_best_local_model(models: &[OllamaModelInfo]) -> Option<String> {
    if models.is_empty() {
        return None;
    }

    let mut scored: Vec<(&OllamaModelInfo, f64)> =
        models.iter().map(|m| (m, score_ollama_model(m))).collect();

    // Sort descending by score
    scored.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(Ordering::Equal));

    scored.first().map(|(m, _)| m.name.clone())
}

/// Selects the best model from a list of raw string model names.
pub fn select_best_local_model_from_names(model_names: &[&str]) -> Option<String> {
    if model_names.is_empty() {
        return None;
    }

    let fake_infos: Vec<OllamaModelInfo> = model_names
        .iter()
        .map(|&name| OllamaModelInfo {
            name: name.to_string(),
            model: Some(name.to_string()),
            modified_at: None,
            size: None,
            digest: None,
            details: None,
            expires_at: None,
            size_vram: None,
        })
        .collect();

    select_best_local_model(&fake_infos)
}

// ============================================================================
// Ping & Socket Utilities
// ============================================================================

/// Ping a local Ollama instance on a specified base URL or default `127.0.0.1:11434`.
///
/// Returns `true` if Ollama is reachable and responds to ping.
pub async fn ping_ollama(base_url: Option<&str>) -> bool {
    let url = base_url.unwrap_or(DEFAULT_OLLAMA_URL);
    ping_ollama_endpoint_internal(url, DEFAULT_OLLAMA_PING_TIMEOUT).await
}

/// Ping a local Ollama instance on `127.0.0.1:11434` synchronously.
pub fn ping_ollama_sync(base_url: Option<&str>) -> bool {
    let url = base_url.unwrap_or(DEFAULT_OLLAMA_URL);
    ping_ollama_socket_sync(url, DEFAULT_OLLAMA_PING_TIMEOUT)
}

/// Pings a host and port endpoint via TCP socket connect.
pub fn ping_socket_addr(addr_str: &str, timeout: Duration) -> bool {
    if let Ok(mut addrs) = addr_str.to_socket_addrs() {
        if let Some(addr) = addrs.next() {
            return TcpStream::connect_timeout(&addr, timeout).is_ok();
        }
    }
    false
}

/// Synchronously checks if a URL's host:port is accepting TCP connections.
fn ping_ollama_socket_sync(base_url: &str, timeout: Duration) -> bool {
    let host_port = extract_host_port_from_url(base_url);
    ping_socket_addr(&host_port, timeout)
}

/// Asynchronously checks if Ollama responds to `/api/tags` or TCP connect.
async fn ping_ollama_endpoint_internal(base_url: &str, timeout: Duration) -> bool {
    let host_port = extract_host_port_from_url(base_url);

    // Fast socket check first
    let socket_ok = tokio::task::spawn_blocking({
        let hp = host_port.clone();
        move || ping_socket_addr(&hp, timeout)
    })
    .await
    .unwrap_or(false);

    if !socket_ok {
        return false;
    }

    // HTTP /api/tags verification
    let url = format!("{}/api/tags", base_url.trim_end_matches('/'));
    let client = match reqwest::Client::builder()
        .timeout(timeout)
        .connect_timeout(timeout)
        .build()
    {
        Ok(c) => c,
        Err(_) => return socket_ok,
    };

    match client.get(&url).send().await {
        Ok(resp) => resp.status().is_success() || resp.status().as_u16() == 404,
        Err(_) => socket_ok, // If socket connected, consider daemon alive
    }
}

/// Extracts `"host:port"` from a given URL like `"http://127.0.0.1:11434"` or `"http://localhost:11434"`.
pub fn extract_host_port_from_url(url: &str) -> String {
    let trimmed = url
        .trim_start_matches("http://")
        .trim_start_matches("https://")
        .trim_end_matches('/');

    let host_part = trimmed
        .split('/')
        .next()
        .unwrap_or(DEFAULT_OLLAMA_SOCKET_ADDR);

    if !host_part.contains(':') {
        format!("{}:{}", host_part, DEFAULT_OLLAMA_PORT)
    } else {
        host_part.to_string()
    }
}

/// Checks internet connectivity using default public DNS probes.
pub fn check_internet_connectivity(timeout: Duration) -> bool {
    let prober = DefaultConnectivityProber::default();
    prober.is_online(timeout)
}

/// Checks internet connectivity asynchronously.
pub async fn check_internet_connectivity_async(timeout: Duration) -> bool {
    tokio::task::spawn_blocking(move || check_internet_connectivity(timeout))
        .await
        .unwrap_or(false)
}

/// Returns true if the system is currently offline.
pub fn is_offline() -> bool {
    !check_internet_connectivity(DEFAULT_CONNECTIVITY_TIMEOUT)
}

/// Returns true if the system is currently online.
pub fn is_online() -> bool {
    check_internet_connectivity(DEFAULT_CONNECTIVITY_TIMEOUT)
}

// ============================================================================
// Offline Mode Detector & Auto-Transition Controller
// ============================================================================

/// Automatic Offline Mode Detector and Local Provider Switcher.
pub struct OfflineDetector {
    config: OfflineDetectorConfig,
    connectivity_prober: Box<dyn ConnectivityProber>,
    ollama_prober: Box<dyn OllamaProber>,
}

impl std::fmt::Debug for OfflineDetector {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OfflineDetector")
            .field("config", &self.config)
            .finish()
    }
}

impl Default for OfflineDetector {
    fn default() -> Self {
        Self::new()
    }
}

impl OfflineDetector {
    /// Creates a new `OfflineDetector` with default production probes.
    pub fn new() -> Self {
        let config = OfflineDetectorConfig::default();
        Self {
            connectivity_prober: Box::new(DefaultConnectivityProber::new(
                config.socket_probes.clone(),
            )),
            ollama_prober: Box::new(DefaultOllamaProber),
            config,
        }
    }

    /// Creates an `OfflineDetector` with custom configuration.
    pub fn with_config(config: OfflineDetectorConfig) -> Self {
        Self {
            connectivity_prober: Box::new(DefaultConnectivityProber::new(
                config.socket_probes.clone(),
            )),
            ollama_prober: Box::new(DefaultOllamaProber),
            config,
        }
    }

    /// Sets a custom Ollama URL (e.g. `http://127.0.0.1:11434`).
    pub fn with_ollama_url(mut self, url: impl Into<String>) -> Self {
        self.config.ollama_url = url.into();
        self
    }

    /// Injects custom probers (useful for unit tests and simulated network topologies).
    pub fn with_probers(
        mut self,
        connectivity_prober: Box<dyn ConnectivityProber>,
        ollama_prober: Box<dyn OllamaProber>,
    ) -> Self {
        self.connectivity_prober = connectivity_prober;
        self.ollama_prober = ollama_prober;
        self
    }

    /// Returns a reference to the detector configuration.
    pub fn config(&self) -> &OfflineDetectorConfig {
        &self.config
    }

    /// Checks if internet connectivity is currently available.
    pub fn is_online(&self) -> bool {
        self.connectivity_prober
            .is_online(self.config.connectivity_timeout)
    }

    /// Checks if local Ollama daemon is reachable.
    pub async fn ping_ollama(&self) -> bool {
        self.ollama_prober
            .ping(&self.config.ollama_url, self.config.ollama_timeout)
            .await
    }

    /// Synchronously checks if local Ollama is reachable.
    pub fn ping_ollama_sync(&self) -> bool {
        self.ollama_prober
            .ping_sync(&self.config.ollama_url, self.config.ollama_timeout)
    }

    /// Queries the list of installed local Ollama models.
    pub async fn list_local_models(&self) -> Result<Vec<OllamaModelInfo>> {
        self.ollama_prober
            .list_models(&self.config.ollama_url, self.config.ollama_timeout)
            .await
    }

    /// Detects current environment status (Online vs Offline).
    pub fn detect_network_status(&self) -> NetworkEnvironmentStatus {
        if self.is_online() {
            NetworkEnvironmentStatus::Online
        } else {
            NetworkEnvironmentStatus::Offline
        }
    }

    /// Checks network and Ollama state, automatically transitioning `config` to Ollama if offline.
    ///
    /// # Transition Rules
    /// 1. If online: Retains existing provider and model; returns [`OfflineTransitionResult::StayedOnline`].
    /// 2. If already using `"ollama"`: Queries installed models, selects best if default model is generic;
    ///    returns [`OfflineTransitionResult::AlreadyOllama`].
    /// 3. If offline and local Ollama is reachable:
    ///    - Fetches local models via `/api/tags`.
    ///    - Selects the best available model (e.g. `qwen2.5-coder:32b`, `qwen2.5-coder`, `deepseek-coder`).
    ///    - Updates `config.default_provider = "ollama"`.
    ///    - Updates `config.default_model = best_model`.
    ///    - Sets `config.ollama_base_url` if unset.
    ///    - Returns [`OfflineTransitionResult::SwitchedToOllama`].
    /// 4. If offline but Ollama is not reachable on `127.0.0.1:11434`:
    ///    - Returns [`OfflineTransitionResult::OfflineNoLocalBackend`] with actionable instructions.
    pub async fn auto_switch_if_offline(&self, config: &mut Config) -> OfflineTransitionResult {
        // 1. Check internet connectivity (skipped when manual offline mode is enforced)
        let online = !self.config.manual_offline_enforced && self.is_online();
        if online {
            debug!(
                "Network is online. Retaining provider '{}' (model '{}').",
                config.default_provider, config.default_model
            );
            return OfflineTransitionResult::StayedOnline {
                provider: config.default_provider.clone(),
                model: config.default_model.clone(),
            };
        }

        let reason = if self.config.manual_offline_enforced {
            OfflineReason::ManualOfflineEnforced
        } else {
            OfflineReason::NoInternetConnection
        };
        info!(
            "{}. Evaluating local Ollama backend for offline transition...",
            reason
        );

        // 2. Check if already configured for Ollama
        let was_already_ollama = config.default_provider.eq_ignore_ascii_case("ollama");

        // 3. Ping local Ollama daemon on 127.0.0.1:11434
        let ollama_alive = self.ping_ollama().await;

        if !ollama_alive {
            warn!(
                "Offline mode detected, but local Ollama is unreachable at {}.",
                self.config.ollama_url
            );
            let remediation = format!(
                "No internet connection detected, and local Ollama daemon is unreachable on {}.\n\
                 To work offline with local models:\n  \
                 1. Start Ollama: 'ollama serve'\n  \
                 2. Pull a coding model: 'ollama run qwen2.5-coder'\n  \
                 3. Fusion will automatically detect and switch to your local model.",
                self.config.ollama_url
            );

            return OfflineTransitionResult::OfflineNoLocalBackend {
                reason,
                attempted_url: self.config.ollama_url.clone(),
                remediation,
            };
        }

        // 4. Retrieve local models from Ollama
        let available_models = match self.list_local_models().await {
            Ok(models) => models,
            Err(e) => {
                warn!(
                    "Failed to query Ollama model list: {}. Using fallback model.",
                    e
                );
                Vec::new()
            }
        };

        let available_names: Vec<String> =
            available_models.iter().map(|m| m.name.clone()).collect();

        // 5. Select best available local model
        let selected_model = if let Some(best) = select_best_local_model(&available_models) {
            info!("Selected best available local Ollama model: '{}'", best);
            best
        } else if !config.default_model.is_empty() && was_already_ollama {
            config.default_model.clone()
        } else {
            info!(
                "No downloaded models found in Ollama; defaulting to '{}'.",
                self.config.fallback_model
            );
            self.config.fallback_model.clone()
        };

        // 6. Handle case where provider was already Ollama
        if was_already_ollama {
            // Update model if current model was not found and we have a better selected model
            if !available_names.is_empty()
                && !available_names.iter().any(|n| n == &config.default_model)
            {
                config.default_model = selected_model.clone();
            }
            return OfflineTransitionResult::AlreadyOllama {
                model: config.default_model.clone(),
                available_models: available_names,
            };
        }

        // 7. Perform seamless transition to Ollama
        let previous_provider = config.default_provider.clone();
        let previous_model = config.default_model.clone();

        config.default_provider = "ollama".to_string();
        config.default_model = selected_model.clone();
        if config.ollama_base_url.is_none() {
            config.ollama_base_url = Some(self.config.ollama_url.clone());
        }

        let notice = format!(
            "Offline mode activated: Switched provider from '{}' ({}) to local Ollama ({}). Available local models: [{}].",
            previous_provider,
            previous_model,
            selected_model,
            if available_names.is_empty() {
                "none (pull with 'ollama run qwen2.5-coder')".to_string()
            } else {
                available_names.join(", ")
            }
        );

        info!("{}", notice);

        OfflineTransitionResult::SwitchedToOllama {
            previous_provider,
            previous_model,
            selected_model,
            available_models: available_names,
            reason,
            notice,
        }
    }

    pub fn auto_switch_if_offline_sync(&self, config: &mut Config) -> OfflineTransitionResult {
        let online = !self.config.manual_offline_enforced && self.is_online();
        if online {
            return OfflineTransitionResult::StayedOnline {
                provider: config.default_provider.clone(),
                model: config.default_model.clone(),
            };
        }

        let reason = if self.config.manual_offline_enforced {
            OfflineReason::ManualOfflineEnforced
        } else {
            OfflineReason::NoInternetConnection
        };
        let was_already_ollama = config.default_provider.eq_ignore_ascii_case("ollama");
        let ollama_alive = self.ping_ollama_sync();

        if !ollama_alive {
            let remediation = format!(
                "No internet connection detected, and local Ollama daemon is unreachable on {}.\n\
                 Start Ollama with 'ollama serve' to enable offline coding assistant mode.",
                self.config.ollama_url
            );
            return OfflineTransitionResult::OfflineNoLocalBackend {
                reason,
                attempted_url: self.config.ollama_url.clone(),
                remediation,
            };
        }

        let previous_provider = config.default_provider.clone();
        let previous_model = config.default_model.clone();
        let selected_model = self.config.fallback_model.clone();

        if was_already_ollama {
            return OfflineTransitionResult::AlreadyOllama {
                model: config.default_model.clone(),
                available_models: Vec::new(),
            };
        }

        config.default_provider = "ollama".to_string();
        config.default_model = selected_model.clone();
        if config.ollama_base_url.is_none() {
            config.ollama_base_url = Some(self.config.ollama_url.clone());
        }

        let notice = format!(
            "Offline mode activated: Switched provider from '{}' ({}) to local Ollama ({}).",
            previous_provider, previous_model, selected_model
        );

        OfflineTransitionResult::SwitchedToOllama {
            previous_provider,
            previous_model,
            selected_model,
            available_models: Vec::new(),
            reason,
            notice,
        }
    }
}

/// Convenience function: Automatically detect offline state and transition configuration to local Ollama.
pub async fn auto_switch_offline(config: &mut Config) -> OfflineTransitionResult {
    let detector = OfflineDetector::new();
    detector.auto_switch_if_offline(config).await
}

/// Convenience function: Synchronously automatically detect offline state and transition configuration to local Ollama.
pub fn auto_switch_offline_sync(config: &mut Config) -> OfflineTransitionResult {
    let detector = OfflineDetector::new();
    detector.auto_switch_if_offline_sync(config)
}

// ============================================================================
// Unit Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::provider::ollama::OllamaModelDetails;

    fn make_test_model(
        name: &str,
        param_size: Option<&str>,
        quant: Option<&str>,
        size_bytes: Option<u64>,
        vram: Option<u64>,
    ) -> OllamaModelInfo {
        OllamaModelInfo {
            name: name.to_string(),
            model: Some(name.to_string()),
            modified_at: None,
            size: size_bytes,
            digest: None,
            details: Some(OllamaModelDetails {
                parent_model: None,
                format: Some("gguf".to_string()),
                family: None,
                families: None,
                parameter_size: param_size.map(|s| s.to_string()),
                quantization_level: quant.map(|s| s.to_string()),
            }),
            expires_at: None,
            size_vram: vram,
        }
    }

    #[test]
    fn test_extract_host_port() {
        assert_eq!(
            extract_host_port_from_url("http://127.0.0.1:11434"),
            "127.0.0.1:11434"
        );
        assert_eq!(
            extract_host_port_from_url("http://localhost:11434/"),
            "localhost:11434"
        );
        assert_eq!(
            extract_host_port_from_url("http://127.0.0.1"),
            "127.0.0.1:11434"
        );
        assert_eq!(
            extract_host_port_from_url("https://remote-ollama.internal:8080/v1"),
            "remote-ollama.internal:8080"
        );
    }

    #[test]
    fn test_parse_param_string() {
        assert_eq!(parse_param_string("32b"), Some(32.0));
        assert_eq!(parse_param_string("32B"), Some(32.0));
        assert_eq!(parse_param_string("14b"), Some(14.0));
        assert_eq!(parse_param_string("7b"), Some(7.0));
        assert_eq!(parse_param_string("1.5b"), Some(1.5));
        assert_eq!(parse_param_string("0.5b"), Some(0.5));
        assert_eq!(parse_param_string("70b"), Some(70.0));
        assert_eq!(parse_param_string("invalid"), None);
    }

    #[test]
    fn test_model_scoring_coder_preference() {
        let qwen_coder_32b = make_test_model(
            "qwen2.5-coder:32b",
            Some("32B"),
            Some("Q4_K_M"),
            Some(19_000_000_000),
            None,
        );
        let qwen_coder_7b = make_test_model(
            "qwen2.5-coder:7b",
            Some("7B"),
            Some("Q4_K_M"),
            Some(4_500_000_000),
            None,
        );
        let deepseek_coder = make_test_model(
            "deepseek-coder:6.7b",
            Some("6.7B"),
            Some("Q4_0"),
            Some(4_000_000_000),
            None,
        );
        let llama3 = make_test_model(
            "llama3.1:8b",
            Some("8B"),
            Some("Q4_0"),
            Some(4_800_000_000),
            None,
        );
        let generic_model = make_test_model(
            "orca-mini:3b",
            Some("3B"),
            Some("Q4_0"),
            Some(2_000_000_000),
            None,
        );

        let score_qwen32 = score_ollama_model(&qwen_coder_32b);
        let score_qwen7 = score_ollama_model(&qwen_coder_7b);
        let score_ds = score_ollama_model(&deepseek_coder);
        let score_llama = score_ollama_model(&llama3);
        let score_generic = score_ollama_model(&generic_model);

        // Verification order: Qwen 32B Coder > Qwen 7B Coder > DeepSeek Coder > Llama 3.1 > Generic
        assert!(score_qwen32 > score_qwen7);
        assert!(score_qwen7 > score_ds);
        assert!(score_ds > score_llama);
        assert!(score_llama > score_generic);
    }

    #[test]
    fn test_select_best_local_model() {
        let models = vec![
            make_test_model("llama3.2:3b", Some("3B"), None, None, None),
            make_test_model("deepseek-r1:14b", Some("14B"), None, None, None),
            make_test_model("qwen2.5-coder:14b", Some("14B"), None, None, None),
            make_test_model("mistral:7b", Some("7B"), None, None, None),
        ];

        let best = select_best_local_model(&models);
        assert_eq!(best.as_deref(), Some("qwen2.5-coder:14b"));
    }

    #[test]
    fn test_select_best_local_model_from_names() {
        let names = ["mistral", "qwen2.5-coder:7b", "llama3.2", "phi3"];
        let best = select_best_local_model_from_names(&names);
        assert_eq!(best.as_deref(), Some("qwen2.5-coder:7b"));
    }

    #[test]
    fn test_select_best_local_model_empty() {
        let empty: Vec<OllamaModelInfo> = Vec::new();
        assert_eq!(select_best_local_model(&empty), None);
        assert_eq!(select_best_local_model_from_names(&[]), None);
    }

    #[tokio::test]
    async fn test_auto_switch_stay_online_when_internet_present() {
        let mut config = Config::default();
        config.default_provider = "anthropic".to_string();
        config.default_model = "claude-3-7-sonnet".to_string();

        let detector = OfflineDetector::with_config(OfflineDetectorConfig::default()).with_probers(
            Box::new(MockConnectivityProber { online: true }),
            Box::new(MockOllamaProber {
                reachable: true,
                models: vec![make_test_model("qwen2.5-coder:7b", None, None, None, None)],
            }),
        );

        let result = detector.auto_switch_if_offline(&mut config).await;

        assert_eq!(
            result,
            OfflineTransitionResult::StayedOnline {
                provider: "anthropic".to_string(),
                model: "claude-3-7-sonnet".to_string(),
            }
        );
        assert_eq!(config.default_provider, "anthropic");
        assert_eq!(config.default_model, "claude-3-7-sonnet");
    }

    #[tokio::test]
    async fn test_auto_switch_to_ollama_when_offline() {
        let mut config = Config::default();
        config.default_provider = "deepseek".to_string();
        config.default_model = "deepseek-chat".to_string();

        let models = vec![
            make_test_model("llama3.1:8b", Some("8B"), None, None, None),
            make_test_model("qwen2.5-coder:32b", Some("32B"), None, None, None),
        ];

        let detector = OfflineDetector::with_config(OfflineDetectorConfig::default()).with_probers(
            Box::new(MockConnectivityProber { online: false }),
            Box::new(MockOllamaProber {
                reachable: true,
                models,
            }),
        );

        let result = detector.auto_switch_if_offline(&mut config).await;

        assert!(result.is_switched());
        assert!(result.is_offline_ready());
        assert_eq!(config.default_provider, "ollama");
        assert_eq!(config.default_model, "qwen2.5-coder:32b");
        assert_eq!(
            config.ollama_base_url.as_deref(),
            Some("http://127.0.0.1:11434")
        );

        match result {
            OfflineTransitionResult::SwitchedToOllama {
                previous_provider,
                previous_model,
                selected_model,
                available_models,
                reason,
                ..
            } => {
                assert_eq!(previous_provider, "deepseek");
                assert_eq!(previous_model, "deepseek-chat");
                assert_eq!(selected_model, "qwen2.5-coder:32b");
                assert_eq!(available_models.len(), 2);
                assert_eq!(reason, OfflineReason::NoInternetConnection);
            }
            _ => panic!("Expected SwitchedToOllama result"),
        }
    }

    #[tokio::test]
    async fn test_auto_switch_offline_no_local_ollama() {
        let mut config = Config::default();
        config.default_provider = "openai".to_string();
        config.default_model = "gpt-4o".to_string();

        let detector = OfflineDetector::with_config(OfflineDetectorConfig::default()).with_probers(
            Box::new(MockConnectivityProber { online: false }),
            Box::new(MockOllamaProber {
                reachable: false,
                models: Vec::new(),
            }),
        );

        let result = detector.auto_switch_if_offline(&mut config).await;

        assert!(!result.is_switched());
        assert!(!result.is_offline_ready());
        // Provider remains unchanged if local Ollama daemon is unreachable
        assert_eq!(config.default_provider, "openai");
        assert_eq!(config.default_model, "gpt-4o");

        match result {
            OfflineTransitionResult::OfflineNoLocalBackend {
                reason,
                attempted_url,
                remediation,
            } => {
                assert_eq!(reason, OfflineReason::NoInternetConnection);
                assert_eq!(attempted_url, "http://127.0.0.1:11434");
                assert!(remediation.contains("ollama serve"));
            }
            _ => panic!("Expected OfflineNoLocalBackend result"),
        }
    }

    #[tokio::test]
    async fn test_already_ollama_offline() {
        let mut config = Config::default();
        config.default_provider = "ollama".to_string();
        config.default_model = "qwen2.5-coder:7b".to_string();

        let models = vec![make_test_model(
            "qwen2.5-coder:7b",
            Some("7B"),
            None,
            None,
            None,
        )];

        let detector = OfflineDetector::with_config(OfflineDetectorConfig::default()).with_probers(
            Box::new(MockConnectivityProber { online: false }),
            Box::new(MockOllamaProber {
                reachable: true,
                models,
            }),
        );

        let result = detector.auto_switch_if_offline(&mut config).await;

        assert_eq!(
            result,
            OfflineTransitionResult::AlreadyOllama {
                model: "qwen2.5-coder:7b".to_string(),
                available_models: vec!["qwen2.5-coder:7b".to_string()],
            }
        );
        assert_eq!(config.default_provider, "ollama");
        assert_eq!(config.default_model, "qwen2.5-coder:7b");
    }

    #[test]
    fn test_sync_auto_switch_offline() {
        let mut config = Config::default();
        config.default_provider = "anthropic".to_string();
        config.default_model = "claude-3-5-sonnet".to_string();

        let detector = OfflineDetector::with_config(OfflineDetectorConfig::default()).with_probers(
            Box::new(MockConnectivityProber { online: false }),
            Box::new(MockOllamaProber {
                reachable: true,
                models: Vec::new(),
            }),
        );

        let result = detector.auto_switch_if_offline_sync(&mut config);
        assert!(result.is_switched());
        assert_eq!(config.default_provider, "ollama");
        assert_eq!(config.default_model, "qwen2.5-coder");
    }
}
