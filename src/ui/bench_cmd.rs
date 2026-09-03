//! High-precision interactive LLM provider latency and throughput benchmarking tool.
//!
//! Provides the `/benchmark` (aliases `/bench`, `/latency`, `/speed`) interactive slash command
//! that pings configured LLM providers, measures Time to First Token (TTFT), tokens per second (tok/s),
//! generation latency, total round-trip time, and formats performance comparison tables.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::future::Future;
use std::io::{stdout, IsTerminal, Write};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use crate::agent::loop_runner::AgentRunner;
use crate::agent::session::Session;
use crate::agent::tokens::estimate_text_tokens;
use crate::config::Config;
use crate::provider::client::LlmClient;
use crate::provider::types::{Message, StreamChunk};
use crate::ui::spinner::{Spinner, SpinnerHandle};
use crate::ui::table::{ColumnAlign, Table, TableTheme};

/// Default test prompt for latency and throughput measurement.
pub const DEFAULT_BENCHMARK_PROMPT: &str =
    "Write a concise 3-line haiku or poem about writing high-performance code in Rust.";

/// Default ping-only test prompt for minimal TTFT round-trip.
pub const DEFAULT_PING_PROMPT: &str = "Reply with the single word 'PONG'.";

/// Default max tokens generated during benchmark to prevent excessive token consumption.
pub const DEFAULT_BENCHMARK_MAX_TOKENS: u32 = 96;

/// Default timeout per provider benchmark request in seconds.
pub const DEFAULT_BENCHMARK_TIMEOUT_SECS: u64 = 20;

/// Default number of rounds/iterations per provider.
pub const DEFAULT_BENCHMARK_ROUNDS: usize = 1;

// ---------------------------------------------------------------------------
// 1. Data Structures & Options
// ---------------------------------------------------------------------------

/// Output formatting mode for benchmark results.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum BenchmarkOutputFormat {
    /// Rich ANSI-styled responsive terminal table
    #[default]
    Table,
    /// Standard JSON format for machine parsing and scripting
    Json,
    /// GitHub-flavored Markdown table for documentation
    Markdown,
    /// Brief one-line summary per provider
    Summary,
}

impl BenchmarkOutputFormat {
    pub fn from_str_loose(s: &str) -> Option<Self> {
        match s.trim().to_lowercase().as_str() {
            "table" | "tbl" | "t" => Some(Self::Table),
            "json" | "js" | "j" => Some(Self::Json),
            "markdown" | "md" | "m" => Some(Self::Markdown),
            "summary" | "sum" | "s" | "brief" => Some(Self::Summary),
            _ => None,
        }
    }
}

/// User options configuring the benchmark run.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BenchmarkOptions {
    /// Specific providers to benchmark (e.g. `["deepseek", "anthropic"]`). If empty, benchmarks all configured.
    pub provider_filters: Vec<String>,
    /// Model override (e.g. `gpt-4o-mini`). If None, uses default model per provider.
    pub model_override: Option<String>,
    /// Number of measurement rounds/iterations to average (default: 1).
    pub rounds: usize,
    /// Custom benchmark prompt text.
    pub prompt: String,
    /// Maximum completion tokens to request.
    pub max_tokens: u32,
    /// Temperature for generation (default: 0.2 for deterministic timing).
    pub temperature: Option<f32>,
    /// Request timeout per provider in seconds.
    pub timeout_secs: u64,
    /// Whether to run provider benchmarks concurrently.
    pub parallel: bool,
    /// Output presentation format.
    pub output_format: BenchmarkOutputFormat,
    /// Suppress interactive spinners and live progress indicators.
    pub quiet: bool,
    /// Run lightweight ping-only check (minimal response length).
    pub ping_only: bool,
    /// Include unconfigured providers in status report.
    pub include_unconfigured: bool,
}

impl Default for BenchmarkOptions {
    fn default() -> Self {
        Self {
            provider_filters: Vec::new(),
            model_override: None,
            rounds: DEFAULT_BENCHMARK_ROUNDS,
            prompt: DEFAULT_BENCHMARK_PROMPT.to_string(),
            max_tokens: DEFAULT_BENCHMARK_MAX_TOKENS,
            temperature: Some(0.2),
            timeout_secs: DEFAULT_BENCHMARK_TIMEOUT_SECS,
            parallel: false,
            output_format: BenchmarkOutputFormat::Table,
            quiet: false,
            ping_only: false,
            include_unconfigured: false,
        }
    }
}

/// A target provider and model candidate to benchmark.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BenchmarkTarget {
    /// Canonical provider name (e.g. `deepseek`, `anthropic`, `openai`, `ollama`, `xai`, `openrouter`).
    pub provider: String,
    /// Model identifier to test (e.g. `deepseek-chat`, `claude-3-5-sonnet-20241022`).
    pub model: String,
    /// Resolved API key, if configured.
    pub api_key: Option<String>,
    /// Base URL for API requests.
    pub base_url: String,
    /// Whether the provider has required credentials or reachable local service.
    pub is_configured: bool,
    /// Actionable setup hint or missing key explanation.
    pub setup_hint: Option<String>,
}

/// Result of a single benchmark round execution.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BenchmarkRunResult {
    /// Provider tested.
    pub provider: String,
    /// Model tested.
    pub model: String,
    /// Round iteration index (1-based).
    pub round: usize,
    /// Time To First Token (duration from dispatch to first stream chunk).
    pub ttft: Duration,
    /// Generation duration (duration from first token to stream completion).
    pub generation_duration: Duration,
    /// Total round-trip latency (request dispatch to stream completion).
    pub total_latency: Duration,
    /// Completion tokens generated.
    pub tokens_generated: usize,
    /// Generation throughput in tokens per second (`tokens_generated / generation_duration`).
    pub tokens_per_second: f64,
    /// Whether the request succeeded completely.
    pub success: bool,
    /// Error message if the run failed.
    pub error_message: Option<String>,
    /// Truncated preview of the generated response.
    pub response_preview: String,
    /// Timestamp (Unix seconds) when the run started.
    pub timestamp_unix: u64,
}

impl BenchmarkRunResult {
    /// Create a failed run result with duration and error description.
    pub fn failed(
        provider: &str,
        model: &str,
        round: usize,
        elapsed: Duration,
        error: impl Into<String>,
    ) -> Self {
        Self {
            provider: provider.to_string(),
            model: model.to_string(),
            round,
            ttft: elapsed,
            generation_duration: Duration::ZERO,
            total_latency: elapsed,
            tokens_generated: 0,
            tokens_per_second: 0.0,
            success: false,
            error_message: Some(error.into()),
            response_preview: String::new(),
            timestamp_unix: current_timestamp(),
        }
    }
}

/// Performance qualitative rating.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum PerformanceRating {
    /// No benchmark data / evaluation error.
    Error,
    /// Failed / Error
    Failed,
    /// Unconfigured / Missing Key
    Unconfigured,
    /// High latency or low throughput (>2500ms TTFT or <15 tok/s)
    Slow,
    /// Moderate latency (1200ms-2500ms TTFT or 15-40 tok/s)
    Moderate,
    /// Good performance (600ms-1200ms TTFT and 40-70 tok/s)
    Good,
    /// Fast response (<600ms TTFT and >70 tok/s)
    Fast,
    /// Ultra-fast response (<350ms TTFT and >100 tok/s)
    Blazing,
}

impl PerformanceRating {
    /// Evaluates TTFT and tokens per second into a composite rating.
    pub fn evaluate(ttft: Duration, tokens_per_sec: f64, success: bool) -> Self {
        if !success {
            return Self::Failed;
        }

        let ttft_ms = ttft.as_millis();

        if ttft_ms <= 350 && tokens_per_sec >= 90.0 {
            Self::Blazing
        } else if ttft_ms <= 650 && tokens_per_sec >= 60.0 {
            Self::Fast
        } else if ttft_ms <= 1300 && tokens_per_sec >= 30.0 {
            Self::Good
        } else if ttft_ms <= 2600 || tokens_per_sec >= 12.0 {
            Self::Moderate
        } else {
            Self::Slow
        }
    }

    /// Display badge label with ANSI styling.
    pub fn badge(&self, color: bool) -> &'static str {
        if color {
            match self {
                Self::Blazing => "\x1b[1;36m⚡ Blazing\x1b[0m",
                Self::Fast => "\x1b[1;32m🚀 Fast\x1b[0m",
                Self::Good => "\x1b[32m✓ Good\x1b[0m",
                Self::Moderate => "\x1b[33m⏳ Moderate\x1b[0m",
                Self::Slow => "\x1b[31m⚠️ Slow\x1b[0m",
                Self::Failed => "\x1b[1;31m✗ Failed\x1b[0m",
                Self::Error => "\x1b[1;31m✗ Error\x1b[0m",
                Self::Unconfigured => "\x1b[2;37m- Unset\x1b[0m",
            }
        } else {
            match self {
                Self::Blazing => "⚡ Blazing",
                Self::Fast => "🚀 Fast",
                Self::Good => "✓ Good",
                Self::Moderate => "⏳ Moderate",
                Self::Slow => "⚠️ Slow",
                Self::Failed => "✗ Failed",
                Self::Error => "✗ Error",
                Self::Unconfigured => "- Unset",
            }
        }
    }

    /// Plain-text badge label (no ANSI styling) for status bars and TUI renders.
    pub fn badge_text(&self) -> String {
        self.badge(false).to_string()
    }
}

/// Aggregated multi-round benchmark summary for a provider/model target.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderBenchmarkSummary {
    pub provider: String,
    pub model: String,
    pub is_configured: bool,
    pub setup_hint: Option<String>,
    pub total_rounds: usize,
    pub successful_rounds: usize,
    pub runs: Vec<BenchmarkRunResult>,
    /// Average Time To First Token across successful rounds.
    pub avg_ttft: Option<Duration>,
    /// Minimum Time To First Token recorded.
    pub min_ttft: Duration,
    /// Maximum Time To First Token recorded.
    pub max_ttft: Duration,
    /// Median (P50) Time To First Token.
    pub median_ttft: Duration,
    /// 95th percentile Time To First Token.
    pub p95_ttft: Duration,
    /// Average generation throughput (tok/s).
    pub avg_tokens_per_second: f64,
    /// Peak generation throughput (tok/s).
    pub max_tokens_per_sec: f64,
    /// Minimum generation throughput (tok/s).
    pub min_tokens_per_sec: f64,
    /// Average total latency from request to completion.
    pub avg_latency: Duration,
    /// Total tokens generated across all successful rounds.
    pub total_tokens_generated: usize,
    /// Average generated completion tokens.
    pub avg_completion_tokens: f64,
    /// Performance qualitative evaluation.
    pub rating: PerformanceRating,
    /// Last error message encountered, if any.
    pub last_error: Option<String>,
}

impl ProviderBenchmarkSummary {
    /// Constructs a summary from a collection of individual run results.
    ///
    /// Configuration state (`is_configured`/`setup_hint`) is derived from the
    /// runs: a target whose failures are all missing-credentials hints is
    /// treated as unconfigured, and the first such hint is kept as `setup_hint`.
    pub fn from_runs(provider: &str, model: &str, runs: &[BenchmarkRunResult]) -> Self {
        let total_rounds = runs.len();
        let successful_runs: Vec<&BenchmarkRunResult> = runs.iter().filter(|r| r.success).collect();
        let successful_rounds = successful_runs.len();

        // A target counts as configured if any round succeeded or any failure was
        // a genuine request error rather than a missing-credentials hint.
        let is_configured = successful_rounds > 0
            || runs.iter().any(|r| {
                r.error_message
                    .as_deref()
                    .map_or(false, |e| !looks_like_config_error(e))
            });
        let setup_hint = if is_configured {
            None
        } else {
            runs.iter().find_map(|r| r.error_message.clone())
        };

        if successful_runs.is_empty() {
            let last_error = runs
                .iter()
                .rev()
                .find_map(|r| r.error_message.clone())
                .or_else(|| {
                    if !is_configured {
                        setup_hint.clone()
                    } else {
                        Some("All benchmark rounds failed".to_string())
                    }
                });

            return Self {
                provider: provider.to_string(),
                model: model.to_string(),
                is_configured,
                setup_hint,
                total_rounds,
                successful_rounds: 0,
                runs: runs.to_vec(),
                avg_ttft: None,
                min_ttft: Duration::ZERO,
                max_ttft: Duration::ZERO,
                median_ttft: Duration::ZERO,
                p95_ttft: Duration::ZERO,
                avg_tokens_per_second: 0.0,
                max_tokens_per_sec: 0.0,
                min_tokens_per_sec: 0.0,
                avg_latency: Duration::ZERO,
                total_tokens_generated: 0,
                avg_completion_tokens: 0.0,
                rating: if is_configured {
                    PerformanceRating::Failed
                } else {
                    PerformanceRating::Unconfigured
                },
                last_error,
            };
        }

        // TTFT stats
        let mut ttft_nanos: Vec<u128> = successful_runs.iter().map(|r| r.ttft.as_nanos()).collect();
        ttft_nanos.sort_unstable();

        let sum_ttft_nanos: u128 = ttft_nanos.iter().sum();
        let avg_ttft = Duration::from_nanos((sum_ttft_nanos / (ttft_nanos.len() as u128)) as u64);
        let min_ttft = Duration::from_nanos(*ttft_nanos.first().unwrap_or(&0) as u64);
        let max_ttft = Duration::from_nanos(*ttft_nanos.last().unwrap_or(&0) as u64);

        let median_idx = ttft_nanos.len() / 2;
        let median_ttft = Duration::from_nanos(ttft_nanos[median_idx] as u64);

        let p95_idx = ((ttft_nanos.len() as f64 * 0.95).ceil() as usize)
            .saturating_sub(1)
            .min(ttft_nanos.len() - 1);
        let p95_ttft = Duration::from_nanos(ttft_nanos[p95_idx] as u64);

        // Throughput stats
        let mut tps_vals: Vec<f64> = successful_runs
            .iter()
            .map(|r| r.tokens_per_second)
            .collect();
        tps_vals.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));

        let sum_tps: f64 = tps_vals.iter().sum();
        let avg_tokens_per_second = sum_tps / (tps_vals.len() as f64);
        let min_tokens_per_sec = *tps_vals.first().unwrap_or(&0.0);
        let max_tokens_per_sec = *tps_vals.last().unwrap_or(&0.0);

        // Total latency & generated tokens
        let sum_total_latency: u128 = successful_runs
            .iter()
            .map(|r| r.total_latency.as_nanos())
            .sum();
        let avg_latency =
            Duration::from_nanos((sum_total_latency / (successful_runs.len() as u128)) as u64);

        let total_tokens_generated: usize =
            successful_runs.iter().map(|r| r.tokens_generated).sum();
        let avg_completion_tokens =
            (total_tokens_generated as f64) / (successful_runs.len() as f64);

        let rating = PerformanceRating::evaluate(avg_ttft, avg_tokens_per_second, true);

        Self {
            provider: provider.to_string(),
            model: model.to_string(),
            is_configured,
            setup_hint,
            total_rounds,
            successful_rounds,
            runs: runs.to_vec(),
            avg_ttft: Some(avg_ttft),
            min_ttft,
            max_ttft,
            median_ttft,
            p95_ttft,
            avg_tokens_per_second,
            max_tokens_per_sec,
            min_tokens_per_sec,
            avg_latency,
            total_tokens_generated,
            avg_completion_tokens,
            rating,
            last_error: None,
        }
    }

    /// Success rate of completed rounds as a ratio between 0.0 and 1.0.
    pub fn success_rate(&self) -> f64 {
        if self.total_rounds == 0 {
            0.0
        } else {
            self.successful_rounds as f64 / self.total_rounds as f64
        }
    }
}

/// Detects whether a run failure message indicates missing provider credentials
/// rather than a genuine request/network error.
fn looks_like_config_error(error: &str) -> bool {
    let lower = error.to_lowercase();
    lower.contains("configured")
        || lower.contains("api_key")
        || lower.contains("api key")
        || lower.contains("missing key")
        || lower.contains("credential")
}

// ---------------------------------------------------------------------------
// 2. Target Discovery & Resolution
// ---------------------------------------------------------------------------

/// Resolves standard default model for a given provider name.
pub fn default_model_for_provider(provider: &str) -> &'static str {
    match provider.to_lowercase().as_str() {
        "deepseek" => "deepseek-chat",
        "anthropic" | "claude" => "claude-3-5-sonnet-20241022",
        "openai" => "gpt-4o",
        "xai" | "grok" => "grok-2",
        "openrouter" => "anthropic/claude-3.5-sonnet",
        "ollama" => "llama3.2",
        _ => "gpt-4o",
    }
}

/// Discovers candidate benchmark targets based on configuration and user filter options.
pub fn discover_benchmark_targets(
    config: &Config,
    options: &BenchmarkOptions,
) -> Vec<BenchmarkTarget> {
    let all_known_providers = [
        "deepseek",
        "anthropic",
        "openai",
        "xai",
        "openrouter",
        "ollama",
    ];

    let requested_providers: Vec<String> = if !options.provider_filters.is_empty() {
        options
            .provider_filters
            .iter()
            .map(|p| p.trim().to_lowercase())
            .filter(|p| !p.is_empty())
            .collect()
    } else {
        // Benchmark all known providers plus active provider if custom
        let mut list: Vec<String> = all_known_providers.iter().map(|&s| s.to_string()).collect();
        let active = config.default_provider.to_lowercase();
        if !list.contains(&active) {
            list.insert(0, active);
        }
        list
    };

    let mut targets = Vec::new();

    for prov in requested_providers {
        let (key, url) = config.get_key_and_url(&prov);
        let is_ollama = prov == "ollama";

        let is_configured = if is_ollama {
            !url.trim().is_empty()
        } else {
            key.as_deref()
                .map(|k| !k.trim().is_empty())
                .unwrap_or(false)
        };

        // Determine target model
        let model = if let Some(m_override) = &options.model_override {
            m_override.clone()
        } else if prov.eq_ignore_ascii_case(&config.default_provider) {
            config.default_model.clone()
        } else {
            default_model_for_provider(&prov).to_string()
        };

        let setup_hint = if !is_configured {
            Some(Config::key_hint(&prov).to_string())
        } else {
            None
        };

        targets.push(BenchmarkTarget {
            provider: prov,
            model,
            api_key: key,
            base_url: url,
            is_configured,
            setup_hint,
        });
    }

    targets
}

// ---------------------------------------------------------------------------
// 3. High-Precision Benchmark Execution Engine
// ---------------------------------------------------------------------------

/// Executes a single provider benchmark request, measuring TTFT, generation throughput, and total latency.
pub async fn benchmark_single_provider(
    client: &LlmClient,
    target: &BenchmarkTarget,
    prompt: &str,
    max_tokens: u32,
    temperature: Option<f32>,
    timeout_secs: u64,
    round: usize,
) -> BenchmarkRunResult {
    if !target.is_configured {
        return BenchmarkRunResult::failed(
            &target.provider,
            &target.model,
            round,
            Duration::ZERO,
            target
                .setup_hint
                .clone()
                .unwrap_or_else(|| "Provider credentials not configured".to_string()),
        );
    }

    let messages = vec![Message::user(prompt)];

    let start_instant = Instant::now();
    let timeout_duration = Duration::from_secs(timeout_secs);

    // Initiate streaming completion with timeout guard
    let stream_result = tokio::time::timeout(
        timeout_duration,
        client.stream_chat_with(
            &target.provider,
            &target.model,
            temperature,
            Some(max_tokens),
            target.api_key.as_deref(),
            &target.base_url,
            &messages,
            &[],
        ),
    )
    .await;

    let mut stream_rx = match stream_result {
        Ok(Ok(rx)) => rx,
        Ok(Err(err)) => {
            return BenchmarkRunResult::failed(
                &target.provider,
                &target.model,
                round,
                start_instant.elapsed(),
                format!("Connection error: {err}"),
            );
        }
        Err(_) => {
            return BenchmarkRunResult::failed(
                &target.provider,
                &target.model,
                round,
                timeout_duration,
                format!("Request timed out after {timeout_secs}s before connection established"),
            );
        }
    };

    let mut first_token_instant: Option<Instant> = None;
    let mut collected_text = String::new();
    let mut server_reported_completion_tokens: Option<u32> = None;
    let mut stream_error: Option<String> = None;

    // Consume stream chunks
    loop {
        let remaining_timeout = match timeout_duration.checked_sub(start_instant.elapsed()) {
            Some(rem) if !rem.is_zero() => rem,
            _ => {
                stream_error = Some(format!(
                    "Streaming timed out after {timeout_secs}s total duration"
                ));
                break;
            }
        };

        match tokio::time::timeout(remaining_timeout, stream_rx.recv()).await {
            Ok(Some(chunk)) => match chunk {
                StreamChunk::ContentDelta(delta) => {
                    if first_token_instant.is_none() && !delta.trim().is_empty() {
                        first_token_instant = Some(Instant::now());
                    }
                    collected_text.push_str(&delta);
                }
                StreamChunk::ThinkingDelta(delta) => {
                    if first_token_instant.is_none() && !delta.trim().is_empty() {
                        first_token_instant = Some(Instant::now());
                    }
                    collected_text.push_str(&delta);
                }
                StreamChunk::Done {
                    completion_tokens, ..
                } => {
                    if let Some(toks) = completion_tokens {
                        server_reported_completion_tokens = Some(toks);
                    }
                    break;
                }
                StreamChunk::Error(err) => {
                    stream_error = Some(err);
                    break;
                }
                _ => {}
            },
            Ok(None) => {
                // Stream closed normally
                break;
            }
            Err(_) => {
                stream_error = Some(format!("Stream stalled for >{timeout_secs}s"));
                break;
            }
        }
    }

    let end_instant = Instant::now();
    let total_latency = end_instant.duration_since(start_instant);

    if let Some(err) = stream_error {
        return BenchmarkRunResult::failed(
            &target.provider,
            &target.model,
            round,
            total_latency,
            err,
        );
    }

    // Measure TTFT
    let first_instant = first_token_instant.unwrap_or(end_instant);
    let ttft = first_instant.duration_since(start_instant);
    let generation_duration = end_instant.duration_since(first_instant);

    // Compute token count
    let tokens_generated = server_reported_completion_tokens
        .map(|t| t as usize)
        .unwrap_or_else(|| estimate_text_tokens(&collected_text).max(1));

    // Throughput calculation: generated tokens / generation duration
    let gen_secs = generation_duration.as_secs_f64();
    let tokens_per_second = if gen_secs > 0.001 {
        (tokens_generated as f64) / gen_secs
    } else {
        // If generation finished instantaneously (e.g. mock or fast local buffer)
        (tokens_generated as f64) / total_latency.as_secs_f64().max(0.001)
    };

    BenchmarkRunResult {
        provider: target.provider.clone(),
        model: target.model.clone(),
        round,
        ttft,
        generation_duration,
        total_latency,
        tokens_generated,
        tokens_per_second,
        success: true,
        error_message: None,
        response_preview: sanitize_preview_text(&collected_text, 50),
        timestamp_unix: current_timestamp(),
    }
}

/// Executes benchmarks across all discovered targets according to options.
pub async fn run_benchmark_suite<F>(
    client: &LlmClient,
    targets: &[BenchmarkTarget],
    options: &BenchmarkOptions,
    mut progress_callback: F,
) -> Vec<ProviderBenchmarkSummary>
where
    F: FnMut(&str, usize, usize, Option<&BenchmarkRunResult>),
{
    let prompt = if options.ping_only {
        DEFAULT_PING_PROMPT
    } else {
        &options.prompt
    };

    let total_targets = targets.len();
    let mut summaries = Vec::with_capacity(total_targets);

    if options.parallel && targets.len() > 1 {
        // Concurrent execution across providers
        let mut futures = Vec::new();

        for target in targets {
            let client = client.clone();
            let target = target.clone();
            let prompt = prompt.to_string();
            let options = options.clone();

            futures.push(tokio::spawn(async move {
                let mut runs = Vec::with_capacity(options.rounds);
                for r in 1..=options.rounds {
                    let res = benchmark_single_provider(
                        &client,
                        &target,
                        &prompt,
                        options.max_tokens,
                        options.temperature,
                        options.timeout_secs,
                        r,
                    )
                    .await;
                    runs.push(res);
                }

                ProviderBenchmarkSummary::from_runs(&target.provider, &target.model, &runs)
            }));
        }

        for (idx, fut) in futures.into_iter().enumerate() {
            progress_callback(
                "Running parallel benchmark...",
                idx + 1,
                total_targets,
                None,
            );
            if let Ok(summary) = fut.await {
                summaries.push(summary);
            }
        }
    } else {
        // Sequential execution with detailed per-round feedback
        for (idx, target) in targets.iter().enumerate() {
            let mut runs = Vec::with_capacity(options.rounds);

            for r in 1..=options.rounds {
                let msg = format!(
                    "[{}/{}] Benchmarking {} ({}) [round {}/{}]...",
                    idx + 1,
                    total_targets,
                    target.provider,
                    target.model,
                    r,
                    options.rounds
                );

                progress_callback(&msg, idx + 1, total_targets, None);

                let res = benchmark_single_provider(
                    client,
                    target,
                    prompt,
                    options.max_tokens,
                    options.temperature,
                    options.timeout_secs,
                    r,
                )
                .await;

                progress_callback(&msg, idx + 1, total_targets, Some(&res));
                runs.push(res);
            }

            let summary =
                ProviderBenchmarkSummary::from_runs(&target.provider, &target.model, &runs);

            summaries.push(summary);
        }
    }

    summaries
}

// ---------------------------------------------------------------------------
// 4. Formatting & Presentation
// ---------------------------------------------------------------------------

/// Formats duration into compact human readable milliseconds / seconds (e.g. `245ms`, `1.42s`).
pub fn format_duration_compact(d: Duration) -> String {
    let millis = d.as_millis();
    if millis < 1000 {
        format!("{}ms", millis)
    } else {
        let secs = d.as_secs_f64();
        format!("{:.2}s", secs)
    }
}

/// Formats TTFT with ANSI color coding based on speed threshold.
pub fn format_ttft_colored(d: Duration, color: bool) -> String {
    let text = format_duration_compact(d);
    if !color {
        return text;
    }

    let ms = d.as_millis();
    if ms == 0 {
        "-".to_string()
    } else if ms < 400 {
        format!("\x1b[1;32m{}\x1b[0m", text) // Bright green
    } else if ms < 1000 {
        format!("\x1b[32m{}\x1b[0m", text) // Green
    } else if ms < 2500 {
        format!("\x1b[33m{}\x1b[0m", text) // Yellow
    } else {
        format!("\x1b[31m{}\x1b[0m", text) // Red
    }
}

/// Formats throughput (tokens/sec) with ANSI color coding.
pub fn format_tps_colored(tps: f64, color: bool) -> String {
    if tps <= 0.0 {
        return "-".to_string();
    }

    let text = format!("{:.1} tok/s", tps);
    if !color {
        return text;
    }

    if tps >= 80.0 {
        format!("\x1b[1;36m{}\x1b[0m", text) // Bright cyan
    } else if tps >= 45.0 {
        format!("\x1b[1;32m{}\x1b[0m", text) // Bright green
    } else if tps >= 20.0 {
        format!("\x1b[32m{}\x1b[0m", text) // Green
    } else if tps >= 10.0 {
        format!("\x1b[33m{}\x1b[0m", text) // Yellow
    } else {
        format!("\x1b[2;37m{}\x1b[0m", text) // Muted gray
    }
}

/// Renders benchmark summaries as an ANSI-formatted responsive table.
pub fn format_benchmark_table(
    summaries: &[ProviderBenchmarkSummary],
    color: bool,
    rounds: usize,
) -> String {
    let mut table = Table::new()
        .with_headers(vec![
            "Provider",
            "Model",
            "Status",
            if rounds > 1 { "Avg TTFT" } else { "TTFT" },
            if rounds > 1 { "Avg Speed" } else { "Gen Speed" },
            "Total Latency",
            "Tokens",
            "Rating",
        ])
        .with_alignments(vec![
            ColumnAlign::Left,
            ColumnAlign::Left,
            ColumnAlign::Center,
            ColumnAlign::Right,
            ColumnAlign::Right,
            ColumnAlign::Right,
            ColumnAlign::Right,
            ColumnAlign::Center,
        ]);

    for s in summaries {
        let status_str = if !s.is_configured {
            if color {
                "\x1b[2;37mUnset\x1b[0m".to_string()
            } else {
                "Unset".to_string()
            }
        } else if s.successful_rounds == s.total_rounds && s.total_rounds > 0 {
            if color {
                "\x1b[1;32mOnline\x1b[0m".to_string()
            } else {
                "Online".to_string()
            }
        } else if s.successful_rounds > 0 {
            if color {
                format!(
                    "\x1b[1;33m{}/{}\x1b[0m",
                    s.successful_rounds, s.total_rounds
                )
            } else {
                format!("{}/{}", s.successful_rounds, s.total_rounds)
            }
        } else if color {
            "\x1b[1;31mError\x1b[0m".to_string()
        } else {
            "Error".to_string()
        };

        let ttft_str = if s.successful_rounds > 0 {
            format_ttft_colored(s.avg_ttft.unwrap_or(Duration::ZERO), color)
        } else {
            "-".to_string()
        };

        let speed_str = if s.successful_rounds > 0 {
            format_tps_colored(s.avg_tokens_per_second, color)
        } else {
            "-".to_string()
        };

        let total_str = if s.successful_rounds > 0 {
            format_duration_compact(s.avg_latency)
        } else {
            "-".to_string()
        };

        let tokens_str = if s.successful_rounds > 0 {
            format!("{:.0}", s.avg_completion_tokens)
        } else {
            "-".to_string()
        };

        let rating_str = s.rating.badge(color).to_string();

        table.add_row(vec![
            s.provider.clone(),
            s.model.clone(),
            status_str,
            ttft_str,
            speed_str,
            total_str,
            tokens_str,
            rating_str,
        ]);
    }

    table.render()
}

/// Renders benchmark summaries as a GitHub-flavored Markdown table.
pub fn format_benchmark_markdown(summaries: &[ProviderBenchmarkSummary], rounds: usize) -> String {
    let mut out = String::new();
    out.push_str("### ⚡ LLM Provider Benchmark Results\n\n");
    out.push_str(
        "| Provider | Model | Status | TTFT | Speed (tok/s) | Total Latency | Tokens | Rating |\n",
    );
    out.push_str(
        "|:---------|:------|:------:|-----:|--------------:|--------------:|-------:|:------:|\n",
    );

    for s in summaries {
        let status = if !s.is_configured {
            "Unconfigured"
        } else if s.successful_rounds == s.total_rounds && s.total_rounds > 0 {
            "Online"
        } else if s.successful_rounds > 0 {
            "Degraded"
        } else {
            "Error"
        };

        let ttft = if s.successful_rounds > 0 {
            format_duration_compact(s.avg_ttft.unwrap_or(Duration::ZERO))
        } else {
            "-".to_string()
        };

        let speed = if s.successful_rounds > 0 {
            format!("{:.1}", s.avg_tokens_per_second)
        } else {
            "-".to_string()
        };

        let total = if s.successful_rounds > 0 {
            format_duration_compact(s.avg_latency)
        } else {
            "-".to_string()
        };

        let tokens = if s.successful_rounds > 0 {
            format!("{:.0}", s.avg_completion_tokens)
        } else {
            "-".to_string()
        };

        let rating = s.rating.badge(false);

        out.push_str(&format!(
            "| {} | {} | {} | {} | {} | {} | {} | {} |\n",
            s.provider, s.model, status, ttft, speed, total, tokens, rating
        ));
    }

    if rounds > 1 {
        out.push_str(&format!(
            "\n*Averaged across {} rounds per provider.*\n",
            rounds
        ));
    }

    out
}

/// Renders benchmark summaries in machine-readable JSON format.
pub fn format_benchmark_json(summaries: &[ProviderBenchmarkSummary]) -> String {
    serde_json::to_string_pretty(summaries).unwrap_or_else(|_| "[]".to_string())
}

/// Computes top performers (fastest TTFT, highest throughput, and overall recommendation).
pub fn format_rankings_and_recommendation(
    summaries: &[ProviderBenchmarkSummary],
    active_provider: &str,
    color: bool,
) -> String {
    let successful: Vec<&ProviderBenchmarkSummary> = summaries
        .iter()
        .filter(|s| s.successful_rounds > 0)
        .collect();

    if successful.is_empty() {
        return String::new();
    }

    // Fastest TTFT
    let fastest_ttft = successful
        .iter()
        .min_by_key(|s| s.avg_ttft.unwrap_or(Duration::ZERO).as_nanos())
        .copied();

    // Highest Throughput
    let fastest_speed = successful
        .iter()
        .max_by(|a, b| {
            a.avg_tokens_per_second
                .partial_cmp(&b.avg_tokens_per_second)
                .unwrap_or(std::cmp::Ordering::Equal)
        })
        .copied();

    let mut out = String::new();
    if color {
        out.push_str("\n\x1b[1;37m🏆 Benchmark Highlights & Recommendations:\x1b[0m\n");
    } else {
        out.push_str("\n🏆 Benchmark Highlights & Recommendations:\n");
    }

    if let Some(f) = fastest_ttft {
        let label = format!("{} ({})", f.provider, f.model);
        let val = format_duration_compact(f.avg_ttft.unwrap_or(Duration::ZERO));
        if color {
            out.push_str(&format!(
                "  \x1b[1;33m🥇 Lowest TTFT:\x1b[0m      \x1b[1;37m{}\x1b[0m \x1b[1;32m({})\x1b[0m\n",
                label, val
            ));
        } else {
            out.push_str(&format!("  🥇 Lowest TTFT:      {} ({})\n", label, val));
        }
    }

    if let Some(s) = fastest_speed {
        let label = format!("{} ({})", s.provider, s.model);
        let val = format!("{:.1} tok/s", s.avg_tokens_per_second);
        if color {
            out.push_str(&format!(
                "  \x1b[1;36m🚀 Max Throughput:\x1b[0m   \x1b[1;37m{}\x1b[0m \x1b[1;36m({})\x1b[0m\n",
                label, val
            ));
        } else {
            out.push_str(&format!("  🚀 Max Throughput:   {} ({})\n", label, val));
        }
    }

    // Recommendation logic
    if let Some(best) = fastest_ttft {
        if !best.provider.eq_ignore_ascii_case(active_provider) {
            if color {
                out.push_str(&format!(
                    "  \x1b[35m💡 Tip:\x1b[0m Switch active provider to \x1b[1;37m{}\x1b[0m with \x1b[36m/provider {}\x1b[0m for faster responses.\n",
                    best.provider, best.provider
                ));
            } else {
                out.push_str(&format!(
                    "  💡 Tip: Switch active provider to {} with /provider {} for faster responses.\n",
                    best.provider, best.provider
                ));
            }
        }
    }

    out
}

/// Formats actionable setup hints for any unconfigured or failing providers.
pub fn format_troubleshooting_and_unconfigured(
    summaries: &[ProviderBenchmarkSummary],
    color: bool,
) -> String {
    let unconfigured_or_failed: Vec<&ProviderBenchmarkSummary> = summaries
        .iter()
        .filter(|s| !s.is_configured || s.successful_rounds == 0)
        .collect();

    if unconfigured_or_failed.is_empty() {
        return String::new();
    }

    let mut out = String::new();
    if color {
        out.push_str("\n\x1b[1;33m⚠️ Provider Configuration & Troubleshooting:\x1b[0m\n");
    } else {
        out.push_str("\n⚠️ Provider Configuration & Troubleshooting:\n");
    }

    for s in unconfigured_or_failed {
        if !s.is_configured {
            let hint = s
                .setup_hint
                .as_deref()
                .unwrap_or("Configure API key in ~/.fusion/config.json");
            if color {
                out.push_str(&format!(
                    "  • \x1b[1;37m{}\x1b[0m: \x1b[2;37m{}\x1b[0m\n",
                    s.provider, hint
                ));
            } else {
                out.push_str(&format!("  • {}: {}\n", s.provider, hint));
            }
        } else if let Some(err) = &s.last_error {
            if color {
                out.push_str(&format!(
                    "  • \x1b[1;31m{}\x1b[0m error: \x1b[2;37m{}\x1b[0m\n",
                    s.provider, err
                ));
            } else {
                out.push_str(&format!("  • {} error: {}\n", s.provider, err));
            }
        }
    }

    out
}

// ---------------------------------------------------------------------------
// 5. Slash Command Parsing & Dispatch
// ---------------------------------------------------------------------------

/// Parses command-line tokens into `BenchmarkOptions`.
pub fn parse_benchmark_args(args: &[String]) -> BenchmarkOptions {
    let mut opts = BenchmarkOptions::default();
    let mut idx = 0;

    while idx < args.len() {
        let token = &args[idx];

        match token.as_str() {
            "-n" | "--rounds" | "--count" => {
                if idx + 1 < args.len() {
                    if let Ok(n) = args[idx + 1].parse::<usize>() {
                        opts.rounds = n.max(1).min(20);
                        idx += 2;
                        continue;
                    }
                }
            }
            "-p" | "--prompt" => {
                if idx + 1 < args.len() {
                    opts.prompt = args[idx + 1].clone();
                    idx += 2;
                    continue;
                }
            }
            "-m" | "--model" => {
                if idx + 1 < args.len() {
                    opts.model_override = Some(args[idx + 1].clone());
                    idx += 2;
                    continue;
                }
            }
            "--max-tokens" | "--tokens" => {
                if idx + 1 < args.len() {
                    if let Ok(t) = args[idx + 1].parse::<u32>() {
                        opts.max_tokens = t.max(8).min(2048);
                        idx += 2;
                        continue;
                    }
                }
            }
            "--timeout" | "-t" => {
                if idx + 1 < args.len() {
                    if let Ok(t) = args[idx + 1].parse::<u64>() {
                        opts.timeout_secs = t.max(2).min(120);
                        idx += 2;
                        continue;
                    }
                }
            }
            "--parallel" | "--concurrent" => {
                opts.parallel = true;
                idx += 1;
                continue;
            }
            "--sequential" | "--sync" => {
                opts.parallel = false;
                idx += 1;
                continue;
            }
            "--ping" | "--ping-only" | "--fast" => {
                opts.ping_only = true;
                idx += 1;
                continue;
            }
            "--json" => {
                opts.output_format = BenchmarkOutputFormat::Json;
                opts.quiet = true;
                idx += 1;
                continue;
            }
            "--markdown" | "--md" => {
                opts.output_format = BenchmarkOutputFormat::Markdown;
                idx += 1;
                continue;
            }
            "--summary" | "--brief" => {
                opts.output_format = BenchmarkOutputFormat::Summary;
                idx += 1;
                continue;
            }
            "--table" => {
                opts.output_format = BenchmarkOutputFormat::Table;
                idx += 1;
                continue;
            }
            "--quiet" | "-q" => {
                opts.quiet = true;
                idx += 1;
                continue;
            }
            "--all" => {
                opts.provider_filters.clear();
                opts.include_unconfigured = true;
                idx += 1;
                continue;
            }
            "compare" => {
                // `/benchmark compare deepseek anthropic openai`
                idx += 1;
                while idx < args.len() && !args[idx].starts_with('-') {
                    opts.provider_filters.push(args[idx].clone());
                    idx += 1;
                }
                continue;
            }
            "active" | "current" => {
                opts.provider_filters.push("active".to_string());
                idx += 1;
                continue;
            }
            other if !other.starts_with('-') => {
                // If it looks like a provider name
                opts.provider_filters.push(other.to_string());
                idx += 1;
                // Next token might be model name if not a flag
                if idx < args.len() && !args[idx].starts_with('-') && opts.model_override.is_none()
                {
                    opts.model_override = Some(args[idx].clone());
                    idx += 1;
                }
                continue;
            }
            _ => {}
        }

        idx += 1;
    }

    // Resolve "active" provider token
    if opts.provider_filters.len() == 1 && opts.provider_filters[0] == "active" {
        opts.provider_filters.clear();
    }

    opts
}

/// Helper to execute async futures cleanly within synchronous REPL slash command contexts.
pub fn run_async_future<F: Future>(f: F) -> F::Output {
    if let Ok(handle) = tokio::runtime::Handle::try_current() {
        tokio::task::block_in_place(|| handle.block_on(f))
    } else {
        tokio::runtime::Runtime::new()
            .expect("Failed to initialize tokio runtime for benchmark")
            .block_on(f)
    }
}

/// Top-level handler for `/benchmark` command.
pub fn handle_benchmark_command(args: &[String], runner: &mut AgentRunner, _session: &mut Session) {
    let mut opts = parse_benchmark_args(args);

    // If user asked for active provider specifically
    if args.first().map(|s| s.as_str()) == Some("active")
        || args.first().map(|s| s.as_str()) == Some("current")
    {
        opts.provider_filters = vec![runner.config().default_provider.clone()];
    }

    let is_interactive = stdout().is_terminal() && !opts.quiet;
    let color_enabled = stdout().is_terminal();

    if !opts.quiet {
        println!(
            "\n\x1b[1;36m✦ LLM Provider Latency & Throughput Benchmark\x1b[0m \x1b[2;37m(TTFT & tok/s)\x1b[0m"
        );
        let prompt_desc = if opts.ping_only {
            "Quick Ping (PONG)"
        } else {
            "Standard generation"
        };
        println!(
            "\x1b[2;37mTest mode: \x1b[0m\x1b[1;37m{}\x1b[0m \x1b[2;37m| Rounds: \x1b[0m\x1b[1;37m{}\x1b[0m \x1b[2;37m| Max tokens: \x1b[0m\x1b[1;37m{}\x1b[0m\n",
            prompt_desc, opts.rounds, opts.max_tokens
        );
    }

    let targets = discover_benchmark_targets(runner.config(), &opts);

    if targets.is_empty() {
        println!("\x1b[1;31mNo matching providers found to benchmark.\x1b[0m");
        return;
    }

    let client = runner.client().clone();
    let runner_active_provider = runner.config().default_provider.clone();

    // Execute benchmark
    let summaries = run_async_future(async {
        let mut active_spinner: Option<SpinnerHandle> = None;

        let results =
            run_benchmark_suite(&client, &targets, &opts, |msg, _curr, _total, run_res| {
                if is_interactive {
                    if let Some(res) = run_res {
                        if let Some(sp) = active_spinner.take() {
                            if res.success {
                                sp.success(&format!(
                                    "{} ({}) - TTFT: {}, Speed: {:.1} tok/s",
                                    res.provider,
                                    res.model,
                                    format_duration_compact(res.ttft),
                                    res.tokens_per_second
                                ));
                            } else {
                                sp.error(&format!(
                                    "{} ({}) - {}",
                                    res.provider,
                                    res.model,
                                    res.error_message.as_deref().unwrap_or("Failed")
                                ));
                            }
                        }
                    } else {
                        if let Some(sp) = &active_spinner {
                            sp.set_message(msg.to_string());
                        } else {
                            active_spinner = Some(Spinner::start(msg.to_string()));
                        }
                    }
                }
            })
            .await;

        if let Some(sp) = active_spinner {
            sp.stop();
        }

        results
    });

    // Output formatted results
    match opts.output_format {
        BenchmarkOutputFormat::Table => {
            let table_str = format_benchmark_table(&summaries, color_enabled, opts.rounds);
            println!("{}", table_str);
            let highlights = format_rankings_and_recommendation(
                &summaries,
                &runner_active_provider,
                color_enabled,
            );
            if !highlights.is_empty() {
                println!("{}", highlights);
            }
            let trouble = format_troubleshooting_and_unconfigured(&summaries, color_enabled);
            if !trouble.is_empty() {
                println!("{}", trouble);
            }
        }
        BenchmarkOutputFormat::Markdown => {
            let md = format_benchmark_markdown(&summaries, opts.rounds);
            println!("{}", md);
        }
        BenchmarkOutputFormat::Json => {
            let json = format_benchmark_json(&summaries);
            println!("{}", json);
        }
        BenchmarkOutputFormat::Summary => {
            for s in &summaries {
                if s.successful_rounds > 0 {
                    println!(
                        "{} ({}): TTFT={} | Speed={:.1} tok/s | Total={} | Rating={}",
                        s.provider,
                        s.model,
                        format_duration_compact(s.avg_ttft.unwrap_or(Duration::ZERO)),
                        s.avg_tokens_per_second,
                        format_duration_compact(s.avg_latency),
                        s.rating.badge(false)
                    );
                } else {
                    println!(
                        "{} ({}): Error - {}",
                        s.provider,
                        s.model,
                        s.last_error.as_deref().unwrap_or("Failed")
                    );
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// 6. Utility Functions
// ---------------------------------------------------------------------------

fn current_timestamp() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn sanitize_preview_text(s: &str, max_len: usize) -> String {
    let single_line = s.replace('\n', " ").replace('\r', " ");
    let trimmed = single_line.trim();
    if trimmed.chars().count() <= max_len {
        trimmed.to_string()
    } else {
        let mut out = String::new();
        for (i, c) in trimmed.chars().enumerate() {
            if i >= max_len.saturating_sub(3) {
                out.push_str("...");
                break;
            }
            out.push(c);
        }
        out
    }
}

// ---------------------------------------------------------------------------
// 7. Unit Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_benchmark_args_defaults() {
        let args = Vec::<String>::new();
        let opts = parse_benchmark_args(&args);
        assert_eq!(opts.rounds, 1);
        assert_eq!(opts.output_format, BenchmarkOutputFormat::Table);
        assert!(!opts.parallel);
        assert!(!opts.ping_only);
        assert!(opts.provider_filters.is_empty());
    }

    #[test]
    fn test_parse_benchmark_args_custom() {
        let args = vec![
            "deepseek".to_string(),
            "-n".to_string(),
            "3".to_string(),
            "--max-tokens".to_string(),
            "150".to_string(),
            "--parallel".to_string(),
            "--ping".to_string(),
        ];
        let opts = parse_benchmark_args(&args);
        assert_eq!(opts.rounds, 3);
        assert_eq!(opts.max_tokens, 150);
        assert!(opts.parallel);
        assert!(opts.ping_only);
        assert_eq!(opts.provider_filters, vec!["deepseek".to_string()]);
    }

    #[test]
    fn test_parse_benchmark_args_compare() {
        let args = vec![
            "compare".to_string(),
            "anthropic".to_string(),
            "openai".to_string(),
            "ollama".to_string(),
            "--json".to_string(),
        ];
        let opts = parse_benchmark_args(&args);
        assert_eq!(
            opts.provider_filters,
            vec![
                "anthropic".to_string(),
                "openai".to_string(),
                "ollama".to_string()
            ]
        );
        assert_eq!(opts.output_format, BenchmarkOutputFormat::Json);
        assert!(opts.quiet);
    }

    #[test]
    fn test_performance_rating_evaluation() {
        // Blazing
        let r1 = PerformanceRating::evaluate(Duration::from_millis(200), 120.0, true);
        assert_eq!(r1, PerformanceRating::Blazing);

        // Fast
        let r2 = PerformanceRating::evaluate(Duration::from_millis(500), 75.0, true);
        assert_eq!(r2, PerformanceRating::Fast);

        // Good
        let r3 = PerformanceRating::evaluate(Duration::from_millis(900), 45.0, true);
        assert_eq!(r3, PerformanceRating::Good);

        // Moderate
        let r4 = PerformanceRating::evaluate(Duration::from_millis(1800), 20.0, true);
        assert_eq!(r4, PerformanceRating::Moderate);

        // Slow
        let r5 = PerformanceRating::evaluate(Duration::from_millis(4000), 5.0, true);
        assert_eq!(r5, PerformanceRating::Slow);

        // Failed
        let r6 = PerformanceRating::evaluate(Duration::from_millis(100), 100.0, false);
        assert_eq!(r6, PerformanceRating::Failed);
    }

    #[test]
    fn test_summary_calculation_and_percentiles() {
        let run1 = BenchmarkRunResult {
            provider: "deepseek".to_string(),
            model: "deepseek-chat".to_string(),
            round: 1,
            ttft: Duration::from_millis(300),
            generation_duration: Duration::from_millis(1000),
            total_latency: Duration::from_millis(1300),
            tokens_generated: 50,
            tokens_per_second: 50.0,
            success: true,
            error_message: None,
            response_preview: "Hello world".to_string(),
            timestamp_unix: 1000,
        };

        let run2 = BenchmarkRunResult {
            provider: "deepseek".to_string(),
            model: "deepseek-chat".to_string(),
            round: 2,
            ttft: Duration::from_millis(500),
            generation_duration: Duration::from_millis(1000),
            total_latency: Duration::from_millis(1500),
            tokens_generated: 60,
            tokens_per_second: 60.0,
            success: true,
            error_message: None,
            response_preview: "Rust speed".to_string(),
            timestamp_unix: 1001,
        };

        let runs = vec![run1, run2];
        let summary = ProviderBenchmarkSummary::from_runs("deepseek", "deepseek-chat", &runs);

        assert_eq!(summary.total_rounds, 2);
        assert_eq!(summary.successful_rounds, 2);
        assert_eq!(summary.avg_ttft, Some(Duration::from_millis(400)));
        assert_eq!(summary.min_ttft, Duration::from_millis(300));
        assert_eq!(summary.max_ttft, Duration::from_millis(500));
        assert_eq!(summary.avg_tokens_per_second, 55.0);
        assert_eq!(summary.max_tokens_per_sec, 60.0);
        assert_eq!(summary.total_tokens_generated, 110);
        assert_eq!(summary.avg_completion_tokens, 55.0);
    }

    #[test]
    fn test_format_table_rendering() {
        let summary = ProviderBenchmarkSummary {
            provider: "openai".to_string(),
            model: "gpt-4o".to_string(),
            is_configured: true,
            setup_hint: None,
            total_rounds: 1,
            successful_rounds: 1,
            runs: Vec::new(),
            avg_ttft: Some(Duration::from_millis(320)),
            min_ttft: Duration::from_millis(320),
            max_ttft: Duration::from_millis(320),
            median_ttft: Duration::from_millis(320),
            p95_ttft: Duration::from_millis(320),
            avg_tokens_per_second: 88.5,
            max_tokens_per_sec: 88.5,
            min_tokens_per_sec: 88.5,
            avg_latency: Duration::from_millis(1200),
            total_tokens_generated: 45,
            avg_completion_tokens: 45.0,
            rating: PerformanceRating::Blazing,
            last_error: None,
        };

        let output = format_benchmark_table(&[summary], false, 1);
        assert!(output.contains("openai"));
        assert!(output.contains("gpt-4o"));
        assert!(output.contains("320ms"));
        assert!(output.contains("88.5 tok/s"));
    }

    #[test]
    fn test_format_markdown_rendering() {
        let summary = ProviderBenchmarkSummary {
            provider: "anthropic".to_string(),
            model: "claude-3-5-sonnet".to_string(),
            is_configured: true,
            setup_hint: None,
            total_rounds: 1,
            successful_rounds: 1,
            runs: Vec::new(),
            avg_ttft: Some(Duration::from_millis(450)),
            min_ttft: Duration::from_millis(450),
            max_ttft: Duration::from_millis(450),
            median_ttft: Duration::from_millis(450),
            p95_ttft: Duration::from_millis(450),
            avg_tokens_per_second: 64.0,
            max_tokens_per_sec: 64.0,
            min_tokens_per_sec: 64.0,
            avg_latency: Duration::from_millis(1500),
            total_tokens_generated: 60,
            avg_completion_tokens: 60.0,
            rating: PerformanceRating::Fast,
            last_error: None,
        };

        let md = format_benchmark_markdown(&[summary], 1);
        assert!(md.contains("| anthropic | claude-3-5-sonnet |"));
        assert!(md.contains("450ms"));
        assert!(md.contains("64.0"));
    }

    #[test]
    fn test_format_json_rendering() {
        let summary = ProviderBenchmarkSummary {
            provider: "ollama".to_string(),
            model: "llama3.2".to_string(),
            is_configured: true,
            setup_hint: None,
            total_rounds: 1,
            successful_rounds: 1,
            runs: Vec::new(),
            avg_ttft: Some(Duration::from_millis(180)),
            min_ttft: Duration::from_millis(180),
            max_ttft: Duration::from_millis(180),
            median_ttft: Duration::from_millis(180),
            p95_ttft: Duration::from_millis(180),
            avg_tokens_per_second: 42.0,
            max_tokens_per_sec: 42.0,
            min_tokens_per_sec: 42.0,
            avg_latency: Duration::from_millis(900),
            total_tokens_generated: 30,
            avg_completion_tokens: 30.0,
            rating: PerformanceRating::Good,
            last_error: None,
        };

        let json = format_benchmark_json(&[summary]);
        assert!(json.contains("\"provider\": \"ollama\""));
        assert!(json.contains("\"model\": \"llama3.2\""));
    }
}
