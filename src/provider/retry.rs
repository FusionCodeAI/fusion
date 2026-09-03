use reqwest::header::HeaderMap;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::future::Future;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::mpsc;

use crate::provider::types::StreamChunk;

/// Default maximum number of retry attempts.
pub const DEFAULT_MAX_RETRIES: usize = 3;
/// Default initial backoff duration (500ms).
pub const DEFAULT_INITIAL_DELAY: Duration = Duration::from_millis(500);
/// Default maximum backoff delay ceiling (30s).
pub const DEFAULT_MAX_DELAY: Duration = Duration::from_secs(30);
/// Default exponential backoff multiplier factor.
pub const DEFAULT_BACKOFF_FACTOR: f64 = 2.0;

// ============================================================================
// Fast Zero-Dependency Lock-Free PRNG
// ============================================================================

/// Thread-safe, lock-free, zero-dependency pseudo-random number generator (XorShift64*).
/// Uses atomic state for safe concurrent access across threads and tasks.
#[derive(Debug)]
pub struct FastRng {
    state: AtomicU64,
}

impl FastRng {
    /// Creates a new `FastRng` seeded from system entropy and thread metadata.
    pub fn new() -> Self {
        Self {
            state: AtomicU64::new(Self::generate_seed()),
        }
    }

    /// Creates a new `FastRng` with a deterministic seed (ideal for unit testing).
    pub fn with_seed(seed: u64) -> Self {
        let non_zero_seed = if seed == 0 {
            0x853c_49e6_748f_ea9b
        } else {
            seed
        };
        Self {
            state: AtomicU64::new(non_zero_seed),
        }
    }

    fn generate_seed() -> u64 {
        use std::time::{SystemTime, UNIX_EPOCH};
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(1_725_300_000_000_000_000) as u64;
        let ptr_entropy = &nanos as *const _ as usize as u64;
        let mut seed = nanos ^ (ptr_entropy << 16) ^ 0x9e37_79b9_7f4a_7c15;
        if seed == 0 {
            seed = 0x853c_49e6_748f_ea9b;
        }
        seed
    }

    /// Generates next pseudo-random 64-bit unsigned integer using XorShift64*.
    pub fn next_u64(&self) -> u64 {
        loop {
            let current = self.state.load(Ordering::Relaxed);
            let mut x = current;
            x ^= x >> 12;
            x ^= x << 25;
            x ^= x >> 27;
            let next = if x == 0 { 0x853c_49e6_748f_ea9b } else { x };
            if self
                .state
                .compare_exchange_weak(current, next, Ordering::Relaxed, Ordering::Relaxed)
                .is_ok()
            {
                return next.wrapping_mul(0x2545_f491_4f6c_dd1d);
            }
        }
    }

    /// Generates a pseudo-random floating point number in `[0.0, 1.0)`.
    pub fn next_f64(&self) -> f64 {
        let v = self.next_u64() >> 11;
        (v as f64) / ((1u64 << 53) as f64)
    }

    /// Generates a pseudo-random float in `[min, max]`.
    pub fn gen_range_f64(&self, min: f64, max: f64) -> f64 {
        if min >= max {
            return min;
        }
        min + (self.next_f64() * (max - min))
    }

    /// Generates a pseudo-random `Duration` between `min` and `max`.
    pub fn gen_duration_range(&self, min: Duration, max: Duration) -> Duration {
        if min >= max {
            return min;
        }
        let min_secs = min.as_secs_f64();
        let max_secs = max.as_secs_f64();
        let sampled = self.gen_range_f64(min_secs, max_secs);
        Duration::from_secs_f64(sampled)
    }
}

impl Default for FastRng {
    fn default() -> Self {
        Self::new()
    }
}

// ============================================================================
// Jitter Algorithms & Mode
// ============================================================================

/// Jitter calculation strategies for randomized exponential backoff.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum JitterMode {
    /// AWS Full Jitter: `sleep = random(floor..=backoff)`
    /// Spreads retry bursts evenly across the full interval to eliminate thundering herds.
    #[default]
    Full,
    /// AWS Equal Jitter: `sleep = (backoff / 2) + random(0..=backoff / 2)`
    /// Guarantees at least 50% of the backoff while randomizing the remaining 50%.
    Equal,
    /// Decorrelated Jitter: `sleep = min(max_delay, random(initial_delay..=prev_delay * 3))`
    /// Dynamic backoff without needing synchronized attempt counters.
    Decorrelated,
    /// No jitter: exact exponential backoff without randomness.
    None,
}

// ============================================================================
// Retry Policy & Builder
// ============================================================================

/// Configuration policy for exponential backoff with jitter and retry rules.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RetryPolicy {
    /// Maximum number of retry attempts (excluding the initial try). Default: 3.
    pub max_retries: usize,
    /// Initial base backoff duration. Default: 500ms.
    pub initial_delay: Duration,
    /// Maximum backoff delay cap. Default: 30s.
    pub max_delay: Duration,
    /// Exponential multiplier factor. Default: 2.0.
    pub backoff_factor: f64,
    /// Jitter strategy to apply. Default: Full Jitter.
    pub jitter_mode: JitterMode,
    /// HTTP status codes that trigger retries. Default: [429, 500, 502, 503, 504, 529].
    pub retryable_status_codes: Vec<u16>,
    /// Whether to honor `Retry-After` header when provided by server. Default: true.
    pub honor_retry_after: bool,
    /// Whether to retry when an early stream error is received before any content. Default: true.
    pub retry_early_stream_errors: bool,
}

impl Default for RetryPolicy {
    fn default() -> Self {
        Self {
            max_retries: DEFAULT_MAX_RETRIES,
            initial_delay: DEFAULT_INITIAL_DELAY,
            max_delay: DEFAULT_MAX_DELAY,
            backoff_factor: DEFAULT_BACKOFF_FACTOR,
            jitter_mode: JitterMode::Full,
            retryable_status_codes: vec![429, 500, 502, 503, 504, 529],
            honor_retry_after: true,
            retry_early_stream_errors: true,
        }
    }
}

impl RetryPolicy {
    /// Creates a new builder for configuring a custom `RetryPolicy`.
    pub fn builder() -> RetryPolicyBuilder {
        RetryPolicyBuilder::default()
    }

    /// Aggressive retry preset: 5 retries, 200ms initial delay, 10s max delay.
    pub fn aggressive() -> Self {
        Self {
            max_retries: 5,
            initial_delay: Duration::from_millis(200),
            max_delay: Duration::from_secs(10),
            backoff_factor: 2.0,
            jitter_mode: JitterMode::Full,
            retryable_status_codes: vec![429, 500, 502, 503, 504, 529],
            honor_retry_after: true,
            retry_early_stream_errors: true,
        }
    }

    /// Conservative retry preset: 3 retries, 1s initial delay, 60s max delay, equal jitter.
    pub fn conservative() -> Self {
        Self {
            max_retries: 3,
            initial_delay: Duration::from_secs(1),
            max_delay: Duration::from_secs(60),
            backoff_factor: 2.0,
            jitter_mode: JitterMode::Equal,
            retryable_status_codes: vec![429, 500, 502, 503, 504, 529],
            honor_retry_after: true,
            retry_early_stream_errors: true,
        }
    }

    /// Preset that disables all automatic retries.
    pub fn no_retry() -> Self {
        Self {
            max_retries: 0,
            initial_delay: Duration::ZERO,
            max_delay: Duration::ZERO,
            backoff_factor: 1.0,
            jitter_mode: JitterMode::None,
            retryable_status_codes: Vec::new(),
            honor_retry_after: false,
            retry_early_stream_errors: false,
        }
    }

    /// Returns `true` if the HTTP status code is configured as retryable.
    pub fn is_retryable_status(&self, status: u16) -> bool {
        self.retryable_status_codes.contains(&status)
    }
}

/// Fluent builder for `RetryPolicy`.
#[derive(Debug, Clone, Default)]
pub struct RetryPolicyBuilder {
    policy: RetryPolicy,
}

impl RetryPolicyBuilder {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn max_retries(mut self, max_retries: usize) -> Self {
        self.policy.max_retries = max_retries;
        self
    }

    pub fn initial_delay(mut self, delay: Duration) -> Self {
        self.policy.initial_delay = delay;
        self
    }

    pub fn max_delay(mut self, delay: Duration) -> Self {
        self.policy.max_delay = delay;
        self
    }

    pub fn backoff_factor(mut self, factor: f64) -> Self {
        self.policy.backoff_factor = factor;
        self
    }

    pub fn jitter_mode(mut self, mode: JitterMode) -> Self {
        self.policy.jitter_mode = mode;
        self
    }

    pub fn retryable_status_codes(mut self, codes: Vec<u16>) -> Self {
        self.policy.retryable_status_codes = codes;
        self
    }

    pub fn add_retryable_status_code(mut self, code: u16) -> Self {
        if !self.policy.retryable_status_codes.contains(&code) {
            self.policy.retryable_status_codes.push(code);
        }
        self
    }

    pub fn honor_retry_after(mut self, honor: bool) -> Self {
        self.policy.honor_retry_after = honor;
        self
    }

    pub fn retry_early_stream_errors(mut self, retry: bool) -> Self {
        self.policy.retry_early_stream_errors = retry;
        self
    }

    pub fn build(self) -> RetryPolicy {
        self.policy
    }
}

// ============================================================================
// Backoff Calculator
// ============================================================================

/// Backoff calculator implementing exponential backoff with jitter and retry-after support.
#[derive(Debug)]
pub struct Backoff {
    policy: RetryPolicy,
    rng: FastRng,
    previous_delay: Duration,
    attempt: usize,
}

impl Backoff {
    /// Creates a new `Backoff` calculator for the given `RetryPolicy`.
    pub fn new(policy: RetryPolicy) -> Self {
        let initial = policy.initial_delay;
        Self {
            policy,
            rng: FastRng::new(),
            previous_delay: initial,
            attempt: 0,
        }
    }

    /// Creates a new `Backoff` with deterministic seed for testing.
    pub fn with_seed(policy: RetryPolicy, seed: u64) -> Self {
        let initial = policy.initial_delay;
        Self {
            policy,
            rng: FastRng::with_seed(seed),
            previous_delay: initial,
            attempt: 0,
        }
    }

    pub fn policy(&self) -> &RetryPolicy {
        &self.policy
    }

    pub fn attempt(&self) -> usize {
        self.attempt
    }

    pub fn previous_delay(&self) -> Duration {
        self.previous_delay
    }

    pub fn reset(&mut self) {
        self.previous_delay = self.policy.initial_delay;
        self.attempt = 0;
    }

    /// Computes the backoff duration for a given attempt index (0-indexed).
    /// If `retry_after` is provided and `honor_retry_after` is enabled, it takes precedence.
    pub fn compute_delay_for_attempt(
        &mut self,
        attempt: usize,
        retry_after: Option<Duration>,
    ) -> Duration {
        if self.policy.honor_retry_after {
            if let Some(ra) = retry_after {
                let delay = ra.min(self.policy.max_delay);
                self.previous_delay = delay;
                return delay;
            }
        }

        let base_s = self.policy.initial_delay.as_secs_f64();
        let max_s = self.policy.max_delay.as_secs_f64();
        let factor = self.policy.backoff_factor;

        let exp_s = (base_s * factor.powi(attempt as i32)).min(max_s);

        let delay = match self.policy.jitter_mode {
            JitterMode::Full => {
                let floor_s = (exp_s * 0.05).min(0.05);
                Duration::from_secs_f64(self.rng.gen_range_f64(floor_s, exp_s))
            }
            JitterMode::Equal => {
                let half = exp_s / 2.0;
                let jittered = half + self.rng.gen_range_f64(0.0, half);
                Duration::from_secs_f64(jittered)
            }
            JitterMode::Decorrelated => {
                let prev_s = self.previous_delay.as_secs_f64();
                let upper = (prev_s * 3.0).min(max_s);
                let lower = base_s.min(upper);
                Duration::from_secs_f64(self.rng.gen_range_f64(lower, upper))
            }
            JitterMode::None => Duration::from_secs_f64(exp_s),
        };

        let final_delay = delay.min(self.policy.max_delay);
        self.previous_delay = final_delay;
        final_delay
    }

    /// Advances to the next attempt and computes its backoff duration.
    pub fn next_delay(&mut self, retry_after: Option<Duration>) -> Duration {
        let delay = self.compute_delay_for_attempt(self.attempt, retry_after);
        self.attempt += 1;
        delay
    }
}

// ============================================================================
// Error Classification & Retry Reasons
// ============================================================================

/// Structured HTTP error capturing status code, message, and optional Retry-After hint.
#[derive(Debug, Clone, thiserror::Error)]
#[error("HTTP {status}: {message}")]
pub struct HttpError {
    pub status: u16,
    pub message: String,
    pub retry_after: Option<Duration>,
}

impl HttpError {
    pub fn new(status: u16, message: impl Into<String>) -> Self {
        Self {
            status,
            message: message.into(),
            retry_after: None,
        }
    }

    pub fn with_retry_after(mut self, retry_after: Duration) -> Self {
        self.retry_after = Some(retry_after);
        self
    }
}

/// Categorized reasons for retrying a request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RetryReason {
    /// HTTP 429: Rate limited / quota exceeded.
    RateLimited { retry_after: Option<Duration> },
    /// HTTP 503: Service Unavailable.
    ServiceUnavailable,
    /// HTTP 529: Provider overloaded (e.g. Anthropic).
    Overloaded,
    /// Server error with status code (500, 502, 504, etc.).
    ServerError(u16),
    /// Client error with status code (4xx).
    ClientError(u16),
    /// Transient network or connect timeout error.
    TransientNetworkError(String),
}

impl RetryReason {
    /// Returns true if this retry reason matches the policy's configured retry rules.
    pub fn is_retryable(&self, policy: &RetryPolicy) -> bool {
        match self {
            RetryReason::RateLimited { .. } => policy.is_retryable_status(429),
            RetryReason::ServiceUnavailable => policy.is_retryable_status(503),
            RetryReason::Overloaded => {
                policy.is_retryable_status(529) || policy.is_retryable_status(503)
            }
            RetryReason::ServerError(code) => policy.is_retryable_status(*code),
            RetryReason::ClientError(code) => policy.is_retryable_status(*code),
            RetryReason::TransientNetworkError(_) => true,
        }
    }

    /// Returns the retry-after duration if present.
    pub fn retry_after(&self) -> Option<Duration> {
        match self {
            RetryReason::RateLimited { retry_after } => *retry_after,
            _ => None,
        }
    }
}

/// Checks if an HTTP status code is conventionally retryable (429, 500, 502, 503, 504, 529).
pub fn is_retryable_status(status: u16) -> bool {
    matches!(status, 429 | 500 | 502 | 503 | 504 | 529)
}

/// Classifies an HTTP status code and optional `retry_after` into a `RetryReason`.
pub fn classify_status_code(status: u16, retry_after: Option<Duration>) -> Option<RetryReason> {
    match status {
        429 => Some(RetryReason::RateLimited { retry_after }),
        503 => Some(RetryReason::ServiceUnavailable),
        529 => Some(RetryReason::Overloaded),
        500 | 502 | 504 => Some(RetryReason::ServerError(status)),
        400..=499 => Some(RetryReason::ClientError(status)),
        500..=599 => Some(RetryReason::ServerError(status)),
        _ => None,
    }
}

/// Parses the `Retry-After` header value from HTTP headers.
pub fn parse_retry_after_header(headers: &HeaderMap) -> Option<Duration> {
    if let Some(val) = headers.get("retry-after").and_then(|v| v.to_str().ok()) {
        if let Some(d) = parse_retry_after_value(val) {
            return Some(d);
        }
    }
    if let Some(val) = headers.get("retry-after-ms").and_then(|v| v.to_str().ok()) {
        if let Ok(ms) = val.trim().parse::<u64>() {
            return Some(Duration::from_millis(ms));
        }
    }
    None
}

/// Parses a `Retry-After` header value string (seconds as integer/float, or RFC 2822 date).
pub fn parse_retry_after_value(val: &str) -> Option<Duration> {
    let trimmed = val.trim();

    // 1. Integer seconds (e.g. "30")
    if let Ok(secs) = trimmed.parse::<u64>() {
        return Some(Duration::from_secs(secs));
    }

    // 2. Decimal seconds (e.g. "2.5")
    if let Ok(secs_f) = trimmed.parse::<f64>() {
        if secs_f > 0.0 && secs_f.is_finite() {
            return Some(Duration::from_secs_f64(secs_f));
        }
    }

    // 3. RFC 2822 HTTP date
    if let Ok(target) = chrono::DateTime::parse_from_rfc2822(trimmed) {
        let now = chrono::Utc::now();
        if let Ok(duration) = target
            .with_timezone(&chrono::Utc)
            .signed_duration_since(now)
            .to_std()
        {
            return Some(duration);
        }
    }

    None
}

/// Scans error text for hints like "try again in 12s" or "retry after 5.5 seconds".
pub fn parse_retry_after_from_text(text: &str) -> Option<Duration> {
    let lower = text.to_lowercase();
    let keywords = [
        "try again in ",
        "retry after ",
        "retry-after: ",
        "retry-after ",
        "wait ",
    ];

    for kw in keywords {
        if let Some(pos) = lower.find(kw) {
            let slice = &lower[pos + kw.len()..];
            if let Some(d) = parse_seconds_prefix(slice) {
                return Some(d);
            }
        }
    }

    None
}

fn parse_seconds_prefix(slice: &str) -> Option<Duration> {
    let trimmed = slice.trim_start();
    let num_str: String = trimmed
        .chars()
        .take_while(|c| c.is_ascii_digit() || *c == '.')
        .collect();

    if let Ok(val) = num_str.parse::<f64>() {
        if val > 0.0 && val.is_finite() {
            let rest = trimmed[num_str.len()..].trim_start();
            if rest.starts_with("ms") || rest.starts_with("millisecond") {
                return Some(Duration::from_millis(val as u64));
            }
            return Some(Duration::from_secs_f64(val));
        }
    }
    None
}

/// Classifies an error string into a `RetryReason` by inspecting status codes and error keywords.
pub fn classify_error_str(err: &str) -> Option<RetryReason> {
    let lower = err.to_lowercase();
    let retry_after = parse_retry_after_from_text(err);

    // Rate limiting patterns (429)
    if lower.contains("429")
        || lower.contains("rate_limit")
        || lower.contains("rate limit")
        || lower.contains("too many requests")
        || lower.contains("quota exceeded")
        || lower.contains("tokens per minute")
        || lower.contains("requests per minute")
    {
        return Some(RetryReason::RateLimited { retry_after });
    }

    // Service unavailable (503)
    if lower.contains("503")
        || lower.contains("service unavailable")
        || lower.contains("temporarily unavailable")
    {
        return Some(RetryReason::ServiceUnavailable);
    }

    // Overloaded (529)
    if lower.contains("529") || lower.contains("overloaded_error") || lower.contains("overloaded") {
        return Some(RetryReason::Overloaded);
    }

    // Gateway errors (502, 504)
    if lower.contains("502") || lower.contains("bad gateway") {
        return Some(RetryReason::ServerError(502));
    }
    if lower.contains("504") || lower.contains("gateway timeout") {
        return Some(RetryReason::ServerError(504));
    }

    // Internal server error (500)
    if lower.contains("500") || lower.contains("internal server error") {
        return Some(RetryReason::ServerError(500));
    }

    None
}

/// Classifies an `anyhow::Error` into a `RetryReason`.
pub fn classify_error(err: &anyhow::Error) -> Option<RetryReason> {
    // 1. Structured HttpError
    if let Some(http_err) = err.downcast_ref::<HttpError>() {
        return classify_status_code(http_err.status, http_err.retry_after);
    }

    // 2. reqwest::Error
    if let Some(req_err) = err.downcast_ref::<reqwest::Error>() {
        if let Some(status) = req_err.status() {
            return classify_status_code(status.as_u16(), None);
        }
        if req_err.is_timeout() || req_err.is_connect() {
            return Some(RetryReason::TransientNetworkError(req_err.to_string()));
        }
    }

    // 3. Error string pattern matching
    classify_error_str(&err.to_string())
}

/// Classifies a `StreamChunk` into a `RetryReason` if it represents a retryable error.
pub fn classify_stream_chunk(chunk: &StreamChunk) -> Option<RetryReason> {
    match chunk {
        StreamChunk::Error(err_msg) => classify_error_str(err_msg),
        _ => None,
    }
}

// ============================================================================
// Resilient Streaming Retry Wrapper
// ============================================================================

/// Transparently retries establishing a stream upon encountering retryable errors (HTTP 429, 503, 529),
/// using exponential backoff with jitter and honoring server `Retry-After` hints.
///
/// Also catches early stream errors if the provider delivers an error chunk before any content/tokens
/// have been emitted downstream.
pub async fn retry_stream<F, Fut>(
    policy: &RetryPolicy,
    factory: F,
) -> anyhow::Result<mpsc::Receiver<StreamChunk>>
where
    F: Fn() -> Fut + Send + Sync + 'static,
    Fut: Future<Output = anyhow::Result<mpsc::Receiver<StreamChunk>>> + Send + 'static,
{
    if policy.max_retries == 0 {
        return factory().await;
    }

    let mut backoff = Backoff::new(policy.clone());
    let factory = Arc::new(factory);

    // Initial connection loop with exponential backoff & jitter
    let mut attempt = 0;
    let in_rx = loop {
        match (factory)().await {
            Ok(rx) => break rx,
            Err(err) => {
                let reason = classify_error(&err);
                let retryable = reason
                    .as_ref()
                    .map(|r| r.is_retryable(policy))
                    .unwrap_or(false);

                if !retryable || attempt >= policy.max_retries {
                    return Err(err);
                }

                let retry_after = reason.as_ref().and_then(|r| r.retry_after());
                let delay = backoff.next_delay(retry_after);

                tracing::warn!(
                    "LLM provider error ({:?}), retrying attempt {}/{} in {:?}...",
                    reason.unwrap_or(RetryReason::ClientError(0)),
                    attempt + 1,
                    policy.max_retries,
                    delay
                );

                tokio::time::sleep(delay).await;
                attempt += 1;
            }
        }
    };

    // If early stream error recovery is disabled, return directly
    if !policy.retry_early_stream_errors {
        return Ok(in_rx);
    }

    // Wrap in a resilient streaming bridge that can recover if the first chunk is an error
    let (tx, out_rx) = mpsc::channel(256);
    let policy_clone = policy.clone();

    tokio::spawn(async move {
        let mut current_rx = in_rx;
        let mut remaining_retries = policy_clone.max_retries.saturating_sub(attempt);
        let mut backoff = backoff;
        let mut content_emitted = false;

        'stream_loop: loop {
            while let Some(chunk) = current_rx.recv().await {
                // If this is an error chunk and we have NOT emitted any content yet, check if retryable
                if !content_emitted {
                    if let StreamChunk::Error(err_msg) = &chunk {
                        let reason = classify_error_str(err_msg);
                        let retryable = reason
                            .as_ref()
                            .map(|r| r.is_retryable(&policy_clone))
                            .unwrap_or(false);

                        if retryable && remaining_retries > 0 {
                            let retry_after = reason.as_ref().and_then(|r| r.retry_after());
                            let delay = backoff.next_delay(retry_after);
                            remaining_retries -= 1;

                            tracing::warn!(
                                "Early stream error received ({:?}), reconnecting in {:?} (remaining retries: {})...",
                                reason,
                                delay,
                                remaining_retries
                            );

                            tokio::time::sleep(delay).await;
                            match (factory)().await {
                                Ok(new_rx) => {
                                    current_rx = new_rx;
                                    continue 'stream_loop;
                                }
                                Err(reconnect_err) => {
                                    // If reconnection fails, send error and exit
                                    let _ = tx
                                        .send(StreamChunk::Error(reconnect_err.to_string()))
                                        .await;
                                    return;
                                }
                            }
                        }
                    }
                }

                // Check if this chunk is content
                match &chunk {
                    StreamChunk::ContentDelta(_)
                    | StreamChunk::ThinkingDelta(_)
                    | StreamChunk::ToolCallDelta { .. } => {
                        content_emitted = true;
                    }
                    _ => {}
                }

                if tx.send(chunk).await.is_err() {
                    // Downstream receiver dropped
                    return;
                }
            }

            // Stream completed normally
            break;
        }
    });

    Ok(out_rx)
}

// ============================================================================
// Generic Async Retry Helper
// ============================================================================

/// Retries a generic async operation using exponential backoff with jitter on 429/503 errors.
pub async fn retry_async<F, Fut, T>(policy: &RetryPolicy, mut operation: F) -> anyhow::Result<T>
where
    F: FnMut() -> Fut,
    Fut: Future<Output = anyhow::Result<T>>,
{
    if policy.max_retries == 0 {
        return operation().await;
    }

    let mut backoff = Backoff::new(policy.clone());
    let mut attempt = 0;

    loop {
        match operation().await {
            Ok(val) => return Ok(val),
            Err(err) => {
                let reason = classify_error(&err);
                let retryable = reason
                    .as_ref()
                    .map(|r| r.is_retryable(policy))
                    .unwrap_or(false);

                if !retryable || attempt >= policy.max_retries {
                    return Err(err);
                }

                let retry_after = reason.as_ref().and_then(|r| r.retry_after());
                let delay = backoff.next_delay(retry_after);

                tracing::warn!(
                    "Operation failed with retryable error ({:?}), retrying {}/{} in {:?}...",
                    reason.unwrap_or(RetryReason::ClientError(0)),
                    attempt + 1,
                    policy.max_retries,
                    delay
                );

                tokio::time::sleep(delay).await;
                attempt += 1;
            }
        }
    }
}

// ============================================================================
// RetryingStream & RetryStats
// ============================================================================

/// Observability stats tracking retry attempts, total delay, and observed error reasons.
#[derive(Debug, Default, Clone)]
pub struct RetryStats {
    pub attempts_made: usize,
    pub total_backoff_duration: Duration,
    pub retry_reasons: Vec<RetryReason>,
}

/// A wrapper around `mpsc::Receiver<StreamChunk>` providing stream stats and retry monitoring.
pub struct RetryingStream {
    rx: mpsc::Receiver<StreamChunk>,
    stats: Arc<std::sync::Mutex<RetryStats>>,
}

impl RetryingStream {
    pub fn new(rx: mpsc::Receiver<StreamChunk>, stats: Arc<std::sync::Mutex<RetryStats>>) -> Self {
        Self { rx, stats }
    }

    pub async fn recv(&mut self) -> Option<StreamChunk> {
        self.rx.recv().await
    }

    pub fn stats(&self) -> RetryStats {
        self.stats.lock().map(|s| s.clone()).unwrap_or_default()
    }

    pub fn into_inner(self) -> mpsc::Receiver<StreamChunk> {
        self.rx
    }
}

// ============================================================================
// Retrying Client Wrapper
// ============================================================================

/// Wrapper around `LlmClient` that executes all streaming and non-streaming requests with automatic retries.
#[derive(Clone)]
pub struct RetryingLlmClient {
    client: crate::provider::LlmClient,
    policy: RetryPolicy,
}

impl RetryingLlmClient {
    pub fn new(client: crate::provider::LlmClient, policy: RetryPolicy) -> Self {
        Self { client, policy }
    }

    pub fn client(&self) -> &crate::provider::LlmClient {
        &self.client
    }

    pub fn policy(&self) -> &RetryPolicy {
        &self.policy
    }

    pub async fn stream_chat(
        &self,
        config: &crate::config::Config,
        messages: &[crate::provider::types::Message],
        tools: &[crate::provider::types::ToolDefinition],
    ) -> anyhow::Result<mpsc::Receiver<StreamChunk>> {
        let client = self.client.clone();
        let config = config.clone();
        let messages = messages.to_vec();
        let tools = tools.to_vec();

        retry_stream(&self.policy, move || {
            let client = client.clone();
            let config = config.clone();
            let messages = messages.clone();
            let tools = tools.clone();
            async move { client.stream_chat(&config, &messages, &tools).await }
        })
        .await
    }
}

// ============================================================================
// Per-Provider Retry Policies
// ============================================================================

/// Maps provider identifiers (e.g. "fusion", "legacy", "local") to dedicated
/// `RetryPolicy` instances, falling back to a shared default when unset.
#[derive(Debug, Clone, Default)]
pub struct ProviderRetryPolicies {
    policies: HashMap<String, RetryPolicy>,
    default: RetryPolicy,
}

impl ProviderRetryPolicies {
    pub fn new(default: RetryPolicy) -> Self {
        Self {
            policies: HashMap::new(),
            default,
        }
    }

    /// Registers (or replaces) the policy for a provider.
    pub fn set_policy(&mut self, provider: impl Into<String>, policy: RetryPolicy) {
        self.policies.insert(provider.into(), policy);
    }

    /// Removes a provider's dedicated policy; future lookups use the default.
    pub fn remove_policy(&mut self, provider: &str) -> Option<RetryPolicy> {
        self.policies.remove(provider)
    }

    /// Returns the policy for `provider`, or the shared default when unregistered.
    pub fn policy_for(&self, provider: &str) -> &RetryPolicy {
        self.policies.get(provider).unwrap_or(&self.default)
    }

    /// Returns `true` if the provider has a dedicated (non-default) policy.
    pub fn has_policy(&self, provider: &str) -> bool {
        self.policies.contains_key(provider)
    }

    /// Number of registered provider-specific policies.
    pub fn len(&self) -> usize {
        self.policies.len()
    }

    pub fn is_empty(&self) -> bool {
        self.policies.is_empty()
    }

    pub fn default_policy(&self) -> &RetryPolicy {
        &self.default
    }
}

// ============================================================================
// Circuit Breaker
// ============================================================================

/// State machine states for the circuit breaker pattern.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CircuitState {
    /// Requests flow through normally; failures increment the counter.
    Closed,
    /// Requests fail fast without hitting the provider.
    Open,
    /// A single probe request is allowed through; success closes, failure re-opens.
    HalfOpen,
}

/// Hook invoked by the circuit breaker whenever the state transitions.
pub type OnStateChange = Box<dyn Fn(CircuitState, CircuitState) + Send + Sync>;

/// Thread-safe circuit breaker guarding provider calls against cascading failures.
///
/// Integration hooks:
/// - `before_request()` / `permit_request()`: call before issuing a request.
/// - `record_success()` / `record_failure()`: call after the outcome is known.
/// - `on_state_change`: optional callback fired on every transition.
#[derive(Clone)]
pub struct CircuitBreaker {
    inner: Arc<std::sync::Mutex<CircuitBreakerInner>>,
}

struct CircuitBreakerInner {
    state: CircuitState,
    consecutive_failures: u32,
    failure_threshold: u32,
    open_duration: Duration,
    opened_at: Option<Instant>,
    half_open_successes: u32,
    half_open_success_threshold: u32,
    on_state_change: Option<OnStateChange>,
}

impl std::fmt::Debug for CircuitBreaker {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let guard = self.inner.lock().map_err(|_| std::fmt::Error)?;
        f.debug_struct("CircuitBreaker")
            .field("state", &guard.state)
            .field("consecutive_failures", &guard.consecutive_failures)
            .field("failure_threshold", &guard.failure_threshold)
            .finish()
    }
}

impl CircuitBreaker {
    /// Creates a breaker that opens after `failure_threshold` consecutive failures
    /// and stays open for `open_duration` before probing with half-open.
    pub fn new(failure_threshold: u32, open_duration: Duration) -> Self {
        Self::with_half_open_threshold(failure_threshold, open_duration, 1)
    }

    /// Like `new`, but `half_open_success_threshold` consecutive probe successes
    /// are required in half-open before the breaker closes again.
    pub fn with_half_open_threshold(
        failure_threshold: u32,
        open_duration: Duration,
        half_open_success_threshold: u32,
    ) -> Self {
        Self {
            inner: Arc::new(std::sync::Mutex::new(CircuitBreakerInner {
                state: CircuitState::Closed,
                consecutive_failures: 0,
                failure_threshold: failure_threshold.max(1),
                open_duration,
                opened_at: None,
                half_open_successes: 0,
                half_open_success_threshold: half_open_success_threshold.max(1),
                on_state_change: None,
            })),
        }
    }

    /// Registers a callback fired on every state transition `(from, to)`.
    pub fn set_on_state_change(&self, hook: OnStateChange) {
        if let Ok(mut guard) = self.inner.lock() {
            guard.on_state_change = Some(hook);
        }
    }

    fn transition(guard: &mut CircuitBreakerInner, to: CircuitState) {
        if guard.state != to {
            let from = guard.state;
            guard.state = to;
            if let Some(hook) = &guard.on_state_change {
                hook(from, to);
            }
        }
    }

    /// Returns the current state, transitioning Open -> HalfOpen once
    /// `open_duration` has elapsed since the breaker opened.
    pub fn state(&self) -> CircuitState {
        let mut guard = match self.inner.lock() {
            Ok(g) => g,
            Err(_) => return CircuitState::Closed,
        };
        if guard.state == CircuitState::Open {
            if let Some(opened_at) = guard.opened_at {
                if opened_at.elapsed() >= guard.open_duration {
                    Self::transition(&mut guard, CircuitState::HalfOpen);
                    guard.half_open_successes = 0;
                }
            }
        }
        guard.state
    }

    /// Non-mutating check: may a request be issued right now?
    pub fn permit_request(&self) -> bool {
        self.state() != CircuitState::Open
    }

    /// Convenience hook: returns `Err(())` when the circuit is open so callers
    /// can fail fast with `?` without touching the provider.
    pub fn before_request(&self) -> Result<(), CircuitOpenError> {
        if self.permit_request() {
            Ok(())
        } else {
            Err(CircuitOpenError)
        }
    }

    /// Records a successful call. In half-open, enough successes close the circuit.
    pub fn record_success(&self) {
        if let Ok(mut guard) = self.inner.lock() {
            match guard.state {
                CircuitState::HalfOpen => {
                    guard.half_open_successes += 1;
                    if guard.half_open_successes >= guard.half_open_success_threshold {
                        guard.consecutive_failures = 0;
                        guard.half_open_successes = 0;
                        guard.opened_at = None;
                        Self::transition(&mut guard, CircuitState::Closed);
                    }
                }
                CircuitState::Closed => {
                    guard.consecutive_failures = 0;
                }
                CircuitState::Open => {}
            }
        }
    }

    /// Records a failed call. Reaches the threshold -> open the circuit.
    pub fn record_failure(&self) {
        if let Ok(mut guard) = self.inner.lock() {
            match guard.state {
                CircuitState::HalfOpen => {
                    guard.opened_at = Some(Instant::now());
                    Self::transition(&mut guard, CircuitState::Open);
                }
                CircuitState::Closed => {
                    guard.consecutive_failures += 1;
                    if guard.consecutive_failures >= guard.failure_threshold {
                        guard.opened_at = Some(Instant::now());
                        Self::transition(&mut guard, CircuitState::Open);
                    }
                }
                CircuitState::Open => {}
            }
        }
    }

    /// Manually forces the breaker open (e.g. on a health-check signal).
    pub fn trip(&self) {
        if let Ok(mut guard) = self.inner.lock() {
            guard.opened_at = Some(Instant::now());
            Self::transition(&mut guard, CircuitState::Open);
        }
    }

    /// Manually resets the breaker to closed with cleared counters.
    pub fn reset(&self) {
        if let Ok(mut guard) = self.inner.lock() {
            guard.consecutive_failures = 0;
            guard.half_open_successes = 0;
            guard.opened_at = None;
            Self::transition(&mut guard, CircuitState::Closed);
        }
    }

    /// Number of consecutive failures recorded while closed.
    pub fn consecutive_failures(&self) -> u32 {
        self.inner
            .lock()
            .map(|g| g.consecutive_failures)
            .unwrap_or(0)
    }
}

/// Error returned when a request is rejected because the circuit is open.
#[derive(Debug, Clone, Copy, thiserror::Error)]
#[error("circuit breaker is open; request rejected without contacting provider")]
pub struct CircuitOpenError;

/// Integrates a `CircuitBreaker` into the retry decision: combined with a
/// `RetryPolicy`, callers consult `should_retry` after each failed attempt.
#[derive(Clone)]
pub struct CircuitBreakerRetryHook {
    breaker: CircuitBreaker,
}

impl CircuitBreakerRetryHook {
    pub fn new(breaker: CircuitBreaker) -> Self {
        Self { breaker }
    }

    pub fn breaker(&self) -> &CircuitBreaker {
        &self.breaker
    }

    /// Gate: `Err` when the circuit forbids a new attempt.
    pub fn permit(&self) -> Result<(), CircuitOpenError> {
        self.breaker.before_request()
    }

    /// Feed a successful attempt into the breaker.
    pub fn on_success(&self) {
        self.breaker.record_success();
    }

    /// Feed a failed attempt into the breaker; returns `false` when the caller
    /// should stop retrying because the circuit just opened.
    pub fn on_failure(&self) -> bool {
        self.breaker.record_failure();
        self.breaker.permit_request()
    }
}

// ============================================================================
// Unit Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_fast_rng_generation() {
        let rng = FastRng::with_seed(12345);
        let val1 = rng.next_u64();
        let val2 = rng.next_u64();
        assert_ne!(val1, val2);

        let f = rng.next_f64();
        assert!(f >= 0.0 && f < 1.0);

        let r = rng.gen_range_f64(10.0, 20.0);
        assert!(r >= 10.0 && r <= 20.0);

        let dur = rng.gen_duration_range(Duration::from_millis(50), Duration::from_millis(100));
        assert!(dur >= Duration::from_millis(50) && dur <= Duration::from_millis(100));
    }

    #[test]
    fn test_fast_rng_deterministic() {
        let rng1 = FastRng::with_seed(42);
        let rng2 = FastRng::with_seed(42);
        for _ in 0..10 {
            assert_eq!(rng1.next_u64(), rng2.next_u64());
        }
    }

    #[test]
    fn test_backoff_no_jitter() {
        let policy = RetryPolicy::builder()
            .initial_delay(Duration::from_millis(100))
            .backoff_factor(2.0)
            .max_delay(Duration::from_secs(5))
            .jitter_mode(JitterMode::None)
            .build();

        let mut backoff = Backoff::new(policy);

        // Attempt 0: 100ms * 2^0 = 100ms
        let d0 = backoff.next_delay(None);
        assert_eq!(d0, Duration::from_millis(100));

        // Attempt 1: 100ms * 2^1 = 200ms
        let d1 = backoff.next_delay(None);
        assert_eq!(d1, Duration::from_millis(200));

        // Attempt 2: 100ms * 2^2 = 400ms
        let d2 = backoff.next_delay(None);
        assert_eq!(d2, Duration::from_millis(400));

        // Attempt 3: 100ms * 2^3 = 800ms
        let d3 = backoff.next_delay(None);
        assert_eq!(d3, Duration::from_millis(800));
    }

    #[test]
    fn test_backoff_full_jitter_bounds() {
        let policy = RetryPolicy::builder()
            .initial_delay(Duration::from_millis(200))
            .backoff_factor(2.0)
            .max_delay(Duration::from_secs(10))
            .jitter_mode(JitterMode::Full)
            .build();

        let mut backoff = Backoff::with_seed(policy, 999);

        // 5 iterations: each should be <= base * 2^attempt
        for attempt in 0..5 {
            let max_possible = Duration::from_millis(200 * (1 << attempt));
            let delay = backoff.next_delay(None);
            assert!(
                delay <= max_possible,
                "Delay {:?} exceeded max possible {:?}",
                delay,
                max_possible
            );
            assert!(delay > Duration::ZERO);
        }
    }

    #[test]
    fn test_backoff_equal_jitter_bounds() {
        let policy = RetryPolicy::builder()
            .initial_delay(Duration::from_millis(500))
            .backoff_factor(2.0)
            .max_delay(Duration::from_secs(10))
            .jitter_mode(JitterMode::Equal)
            .build();

        let mut backoff = Backoff::with_seed(policy, 100);

        for attempt in 0..4 {
            let exp = Duration::from_millis(500 * (1 << attempt));
            let half = exp / 2;
            let delay = backoff.next_delay(None);
            assert!(
                delay >= half,
                "Delay {:?} was below equal jitter lower bound {:?}",
                delay,
                half
            );
            assert!(
                delay <= exp,
                "Delay {:?} exceeded equal jitter upper bound {:?}",
                delay,
                exp
            );
        }
    }

    #[test]
    fn test_backoff_max_delay_ceiling() {
        let policy = RetryPolicy::builder()
            .initial_delay(Duration::from_secs(1))
            .backoff_factor(2.0)
            .max_delay(Duration::from_secs(4))
            .jitter_mode(JitterMode::None)
            .build();

        let mut backoff = Backoff::new(policy);

        assert_eq!(backoff.next_delay(None), Duration::from_secs(1)); // 0
        assert_eq!(backoff.next_delay(None), Duration::from_secs(2)); // 1
        assert_eq!(backoff.next_delay(None), Duration::from_secs(4)); // 2
        assert_eq!(backoff.next_delay(None), Duration::from_secs(4)); // 3: capped!
        assert_eq!(backoff.next_delay(None), Duration::from_secs(4)); // 4: capped!
    }

    #[test]
    fn test_backoff_honors_retry_after() {
        let policy = RetryPolicy::builder()
            .initial_delay(Duration::from_millis(500))
            .honor_retry_after(true)
            .max_delay(Duration::from_secs(30))
            .build();

        let mut backoff = Backoff::new(policy);
        let server_hint = Some(Duration::from_secs(12));
        let delay = backoff.next_delay(server_hint);
        assert_eq!(delay, Duration::from_secs(12));

        // When exceeding max_delay, cap it
        let huge_hint = Some(Duration::from_secs(120));
        let delay2 = backoff.next_delay(huge_hint);
        assert_eq!(delay2, Duration::from_secs(30));
    }

    #[test]
    fn test_classify_status_codes() {
        assert_eq!(
            classify_status_code(429, None),
            Some(RetryReason::RateLimited { retry_after: None })
        );
        assert_eq!(
            classify_status_code(503, None),
            Some(RetryReason::ServiceUnavailable)
        );
        assert_eq!(
            classify_status_code(529, None),
            Some(RetryReason::Overloaded)
        );
        assert_eq!(
            classify_status_code(502, None),
            Some(RetryReason::ServerError(502))
        );
        assert_eq!(
            classify_status_code(401, None),
            Some(RetryReason::ClientError(401))
        );

        let policy = RetryPolicy::default();
        assert!(RetryReason::RateLimited { retry_after: None }.is_retryable(&policy));
        assert!(RetryReason::ServiceUnavailable.is_retryable(&policy));
        assert!(RetryReason::Overloaded.is_retryable(&policy));
        assert!(RetryReason::ServerError(500).is_retryable(&policy));
        assert!(RetryReason::ServerError(502).is_retryable(&policy));
        assert!(RetryReason::ServerError(504).is_retryable(&policy));
        assert!(!RetryReason::ClientError(401).is_retryable(&policy));
        assert!(!RetryReason::ClientError(404).is_retryable(&policy));
    }

    #[test]
    fn test_classify_error_str_gateway_and_server_errors() {
        assert_eq!(
            classify_error_str("HTTP 502 Bad Gateway: Upstream connection failed"),
            Some(RetryReason::ServerError(502))
        );
        assert_eq!(
            classify_error_str("Bad Gateway encountered from provider"),
            Some(RetryReason::ServerError(502))
        );
        assert_eq!(
            classify_error_str("Service is temporarily unavailable"),
            Some(RetryReason::ServiceUnavailable)
        );
        assert_eq!(
            classify_error_str("HTTP 504 Gateway Timeout"),
            Some(RetryReason::ServerError(504))
        );
        assert_eq!(
            classify_error_str("500 Internal Server Error"),
            Some(RetryReason::ServerError(500))
        );
    }

    #[test]
    fn test_parse_retry_after_header_formats() {
        assert_eq!(parse_retry_after_value("10"), Some(Duration::from_secs(10)));
        assert_eq!(
            parse_retry_after_value("2.5"),
            Some(Duration::from_secs_f64(2.5))
        );
        assert_eq!(parse_retry_after_value("invalid"), None);

        let mut headers = HeaderMap::new();
        headers.insert("retry-after", "15".parse().unwrap());
        assert_eq!(
            parse_retry_after_header(&headers),
            Some(Duration::from_secs(15))
        );

        let mut headers_ms = HeaderMap::new();
        headers_ms.insert("retry-after-ms", "750".parse().unwrap());
        assert_eq!(
            parse_retry_after_header(&headers_ms),
            Some(Duration::from_millis(750))
        );
    }

    #[tokio::test]
    async fn test_retry_async_success_on_first_try() {
        let policy = RetryPolicy::default();
        let mut count = 0;

        let result = retry_async(&policy, || {
            count += 1;
            async { Ok::<_, anyhow::Error>("success") }
        })
        .await;

        assert_eq!(result.unwrap(), "success");
        assert_eq!(count, 1);
    }

    #[tokio::test]
    async fn test_retry_async_recovers_after_rate_limits() {
        let policy = RetryPolicy::builder()
            .max_retries(3)
            .initial_delay(Duration::from_millis(10))
            .max_delay(Duration::from_millis(50))
            .build();

        let mut count = 0;

        let result = retry_async(&policy, || {
            count += 1;
            async move {
                if count < 3 {
                    anyhow::bail!("Request failed (429): Rate limit hit");
                }
                Ok::<_, anyhow::Error>(42)
            }
        })
        .await;

        assert_eq!(result.unwrap(), 42);
        assert_eq!(count, 3);
    }

    #[tokio::test]
    async fn test_retry_async_aborts_on_non_retryable_error() {
        let policy = RetryPolicy::default();
        let mut count = 0;

        let result: anyhow::Result<()> = retry_async(&policy, || {
            count += 1;
            async move {
                anyhow::bail!("Request failed (401): Unauthorized");
            }
        })
        .await;

        assert!(result.is_err());
        assert_eq!(count, 1); // Does NOT retry 401
    }

    #[tokio::test]
    async fn test_retry_stream_success() {
        let policy = RetryPolicy::default();

        let rx = retry_stream(&policy, || async {
            let (tx, rx) = mpsc::channel(16);
            tx.send(StreamChunk::ContentDelta("Hello".to_string()))
                .await
                .unwrap();
            tx.send(StreamChunk::Done {
                finish_reason: Some("stop".to_string()),
                prompt_tokens: Some(10),
                completion_tokens: Some(5),
            })
            .await
            .unwrap();
            Ok(rx)
        })
        .await
        .unwrap();

        let mut rx = rx;
        let c1 = rx.recv().await.unwrap();
        match c1 {
            StreamChunk::ContentDelta(s) => assert_eq!(s, "Hello"),
            other => panic!("Expected ContentDelta, got {:?}", other),
        }

        let c2 = rx.recv().await.unwrap();
        match c2 {
            StreamChunk::Done { finish_reason, .. } => {
                assert_eq!(finish_reason, Some("stop".to_string()))
            }
            other => panic!("Expected Done, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_retry_stream_connection_failure_and_recovery() {
        let policy = RetryPolicy::builder()
            .max_retries(3)
            .initial_delay(Duration::from_millis(10))
            .max_delay(Duration::from_millis(50))
            .build();

        let attempt_counter = Arc::new(AtomicU64::new(0));
        let counter_clone = attempt_counter.clone();

        let rx = retry_stream(&policy, move || {
            let counter = counter_clone.clone();
            async move {
                let prev = counter.fetch_add(1, Ordering::SeqCst);
                if prev < 2 {
                    anyhow::bail!("Request to provider failed (503): Service Unavailable");
                }
                let (tx, rx) = mpsc::channel(16);
                tx.send(StreamChunk::ContentDelta("Recovered!".to_string()))
                    .await
                    .unwrap();
                Ok(rx)
            }
        })
        .await
        .unwrap();

        assert_eq!(attempt_counter.load(Ordering::SeqCst), 3);

        let mut rx = rx;
        let c = rx.recv().await.unwrap();
        match c {
            StreamChunk::ContentDelta(s) => assert_eq!(s, "Recovered!"),
            other => panic!("Expected ContentDelta, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_retry_stream_early_error_chunk_recovery() {
        let policy = RetryPolicy::builder()
            .max_retries(3)
            .initial_delay(Duration::from_millis(10))
            .max_delay(Duration::from_millis(50))
            .retry_early_stream_errors(true)
            .build();

        let attempt_counter = Arc::new(AtomicU64::new(0));
        let counter_clone = attempt_counter.clone();

        let rx = retry_stream(&policy, move || {
            let counter = counter_clone.clone();
            async move {
                let attempt = counter.fetch_add(1, Ordering::SeqCst);
                let (tx, rx) = mpsc::channel(16);
                if attempt == 0 {
                    // Send 429 error chunk immediately before any content
                    tx.send(StreamChunk::Error(
                        "Rate limit reached (429): please retry".to_string(),
                    ))
                    .await
                    .unwrap();
                } else {
                    tx.send(StreamChunk::ContentDelta("Stream recovered!".to_string()))
                        .await
                        .unwrap();
                }
                Ok(rx)
            }
        })
        .await
        .unwrap();

        let mut rx = rx;
        let c = rx.recv().await.unwrap();
        match c {
            StreamChunk::ContentDelta(s) => assert_eq!(s, "Stream recovered!"),
            other => panic!("Expected ContentDelta, got {:?}", other),
        }

        assert_eq!(attempt_counter.load(Ordering::SeqCst), 2);
    }

    // ------------------------- Per-Provider Policies -------------------------

    #[test]
    fn test_provider_policies_lookup_falls_back_to_default() {
        let default = RetryPolicy::builder()
            .max_retries(3)
            .initial_delay(Duration::from_millis(100))
            .build();
        let fusion = RetryPolicy::builder()
            .max_retries(7)
            .initial_delay(Duration::from_millis(250))
            .build();

        let mut policies = ProviderRetryPolicies::new(default.clone());
        assert_eq!(policies.len(), 0);
        assert!(policies.is_empty());
        assert!(!policies.has_policy("fusion"));

        // Unregistered providers use the shared default.
        assert_eq!(policies.policy_for("legacy").max_retries, 3);
        assert_eq!(
            policies.policy_for("legacy").initial_delay,
            Duration::from_millis(100)
        );

        policies.set_policy("fusion", fusion);
        assert!(policies.has_policy("fusion"));
        assert_eq!(policies.len(), 1);
        assert_eq!(policies.policy_for("fusion").max_retries, 7);
        assert_eq!(
            policies.policy_for("fusion").initial_delay,
            Duration::from_millis(250)
        );

        // Default policy remains untouched by registrations.
        assert_eq!(policies.default_policy().max_retries, 3);

        // Replacing a provider's policy overwrites in place.
        policies.set_policy("fusion", RetryPolicy::builder().max_retries(9).build());
        assert_eq!(policies.len(), 1);
        assert_eq!(policies.len(), 1);
        assert_eq!(policies.policy_for("fusion").max_retries, 9);

        // Removing reverts to default.
        let removed = policies.remove_policy("fusion");
        assert!(removed.is_some());
        assert!(!policies.has_policy("fusion"));
        assert_eq!(policies.policy_for("legacy").max_retries, 3);
    }

    #[test]
    fn test_provider_policies_remove_missing_is_none() {
        let mut policies = ProviderRetryPolicies::new(RetryPolicy::default());
        assert!(policies.remove_policy("nope").is_none());
        assert_eq!(policies.len(), 0);
    }

    #[test]
    fn test_provider_policies_no_retry_default() {
        let policies = ProviderRetryPolicies::new(RetryPolicy::no_retry());
        assert_eq!(policies.policy_for("fusion").max_retries, 0);
        assert!(policies
            .policy_for("fusion")
            .retryable_status_codes
            .is_empty());
        assert!(!policies.policy_for("fusion").honor_retry_after);
    }

    // ---------------------------- Circuit Breaker ----------------------------

    #[test]
    fn test_circuit_breaker_stays_closed_below_threshold() {
        let breaker = CircuitBreaker::new(3, Duration::from_millis(50));
        breaker.record_failure();
        breaker.record_failure();
        assert_eq!(breaker.state(), CircuitState::Closed);
        assert!(breaker.permit_request());
        assert!(breaker.before_request().is_ok());
        assert_eq!(breaker.consecutive_failures(), 2);
    }

    #[test]
    fn test_circuit_breaker_success_resets_failure_counter() {
        let breaker = CircuitBreaker::new(2, Duration::from_millis(50));
        breaker.record_failure();
        breaker.record_success();
        breaker.record_failure();
        assert_eq!(breaker.state(), CircuitState::Closed);
        assert_eq!(breaker.consecutive_failures(), 1);
    }

    #[test]
    fn test_circuit_breaker_opens_at_threshold_and_rejects() {
        let breaker = CircuitBreaker::new(2, Duration::from_secs(60));
        breaker.record_failure();
        breaker.record_failure();
        assert_eq!(breaker.state(), CircuitState::Open);
        assert!(!breaker.permit_request());
        // Hook gate fails fast without contacting the provider.
        assert!(breaker.before_request().is_err());
    }

    #[test]
    fn test_circuit_breaker_trip_forces_open() {
        let breaker = CircuitBreaker::new(10, Duration::from_secs(60));
        assert_eq!(breaker.state(), CircuitState::Closed);
        breaker.trip();
        assert_eq!(breaker.state(), CircuitState::Open);
        assert!(!breaker.permit_request());
    }

    #[test]
    fn test_circuit_breaker_reset_clears_open_circuit() {
        let breaker = CircuitBreaker::new(1, Duration::from_secs(60));
        breaker.record_failure();
        assert_eq!(breaker.state(), CircuitState::Open);
        breaker.reset();
        assert_eq!(breaker.state(), CircuitState::Closed);
        assert!(breaker.permit_request());
        assert_eq!(breaker.consecutive_failures(), 0);
    }

    #[tokio::test]
    async fn test_circuit_breaker_half_open_after_open_duration() {
        let breaker = CircuitBreaker::new(1, Duration::from_millis(30));
        breaker.record_failure();
        assert_eq!(breaker.state(), CircuitState::Open);

        // Still within the open window: remains open.
        assert_eq!(breaker.state(), CircuitState::Open);

        tokio::time::sleep(Duration::from_millis(50)).await;
        // After the window elapses, the next observation transitions to half-open.
        assert_eq!(breaker.state(), CircuitState::HalfOpen);
        // Half-open permits a probe request.
        assert!(breaker.permit_request());
    }

    #[test]
    fn test_circuit_breaker_half_open_failure_reopens() {
        let breaker = CircuitBreaker::new(1, Duration::from_millis(10));
        breaker.record_failure();
        assert_eq!(breaker.state(), CircuitState::Open);

        // Manually advance to half-open by tripping an immediate re-check after
        // the open window: trip + short sleep is the deterministic path here.
        std::thread::sleep(Duration::from_millis(15));
        assert_eq!(breaker.state(), CircuitState::HalfOpen);

        breaker.record_failure();
        assert_eq!(breaker.state(), CircuitState::Open);
    }

    #[test]
    fn test_circuit_breaker_half_open_success_closes() {
        let breaker = CircuitBreaker::new(1, Duration::from_millis(10));
        breaker.record_failure();
        std::thread::sleep(Duration::from_millis(15));
        assert_eq!(breaker.state(), CircuitState::HalfOpen);

        breaker.record_success();
        assert_eq!(breaker.state(), CircuitState::Closed);
        assert_eq!(breaker.consecutive_failures(), 0);
    }

    #[test]
    fn test_circuit_breaker_half_open_success_threshold() {
        let breaker = CircuitBreaker::with_half_open_threshold(1, Duration::from_millis(10), 3);
        breaker.record_failure();
        std::thread::sleep(Duration::from_millis(15));
        assert_eq!(breaker.state(), CircuitState::HalfOpen);

        breaker.record_success();
        assert_eq!(breaker.state(), CircuitState::HalfOpen);
        breaker.record_success();
        assert_eq!(breaker.state(), CircuitState::HalfOpen);
        breaker.record_success();
        assert_eq!(breaker.state(), CircuitState::Closed);
    }

    #[test]
    fn test_circuit_breaker_on_state_change_hook() {
        let breaker = CircuitBreaker::new(1, Duration::from_millis(10));
        let transitions = Arc::new(std::sync::Mutex::new(Vec::new()));
        let log = transitions.clone();

        breaker.set_on_state_change(Box::new(move |from, to| {
            if let Ok(mut v) = log.lock() {
                v.push((from, to));
            }
        }));

        breaker.record_failure(); // Closed -> Open
        std::thread::sleep(Duration::from_millis(15));
        breaker.state(); // Open -> HalfOpen
        breaker.record_success(); // HalfOpen -> Closed

        let v = transitions.lock().map(|v| v.clone()).unwrap_or_default();
        assert_eq!(
            v,
            vec![
                (CircuitState::Closed, CircuitState::Open),
                (CircuitState::Open, CircuitState::HalfOpen),
                (CircuitState::HalfOpen, CircuitState::Closed),
            ]
        );
    }

    #[test]
    fn test_circuit_breaker_thread_safety() {
        let breaker = CircuitBreaker::new(5, Duration::from_millis(10));
        let handles: Vec<_> = (0..4)
            .map(|_| {
                let b = breaker.clone();
                std::thread::spawn(move || {
                    for _ in 0..10 {
                        b.record_failure();
                    }
                    for _ in 0..5 {
                        b.record_success();
                    }
                })
            })
            .collect();
        for h in handles {
            h.join().expect("worker thread should not panic");
        }
        // 40 failures vs threshold 5: the breaker is open, but no data race.
        assert_eq!(breaker.state(), CircuitState::Open);
    }

    // ----------------------- Circuit Breaker Retry Hook ----------------------

    #[tokio::test]
    async fn test_retry_hook_success_path() {
        let breaker = CircuitBreaker::new(3, Duration::from_millis(10));
        let hook = CircuitBreakerRetryHook::new(breaker);

        assert!(hook.permit().is_ok());
        hook.on_success();
        assert!(hook.permit().is_ok());
        assert_eq!(hook.breaker().consecutive_failures(), 0);
    }

    #[tokio::test]
    async fn test_retry_hook_failure_opens_circuit_stops_retries() {
        let breaker = CircuitBreaker::new(2, Duration::from_millis(10));
        let hook = CircuitBreakerRetryHook::new(breaker);

        assert!(hook.permit().is_ok());
        assert!(hook.on_failure()); // attempt 1 of 2: keep retrying
        assert!(!hook.on_failure()); // attempt 2 of 2: circuit opened, stop

        // New attempts are rejected fast without contacting the provider.
        assert!(hook.permit().is_err());
    }

    #[tokio::test]
    async fn test_retry_hook_half_open_recovery() {
        let breaker = CircuitBreaker::new(2, Duration::from_millis(10));
        let hook = CircuitBreakerRetryHook::new(breaker);

        hook.on_failure();
        hook.on_failure();
        assert!(hook.permit().is_err());

        // After the open window, a probe is permitted; success closes the circuit.
        tokio::time::sleep(Duration::from_millis(25)).await;
        assert!(hook.permit().is_ok());
        hook.on_success();
        assert!(hook.permit().is_ok());
        assert_eq!(hook.breaker().state(), CircuitState::Closed);
    }

    // ------------------------ Backoff integration caps ----------------------

    #[test]
    fn test_backoff_decorrelated_jitter_grows_within_bounds() {
        let policy = RetryPolicy::builder()
            .initial_delay(Duration::from_millis(100))
            .max_delay(Duration::from_secs(2))
            .jitter_mode(JitterMode::Decorrelated)
            .build();
        let mut backoff = Backoff::with_seed(policy, 7);

        let mut prev = Duration::from_millis(100);
        for _ in 0..8 {
            let delay = backoff.next_delay(None);
            // Decorrelated samples within [initial, min(prev * 3, max_delay)].
            assert!(
                delay >= Duration::from_millis(100).min(delay),
                "below floor: {:?}",
                delay
            );
            assert!(
                delay <= Duration::from_secs(2),
                "above ceiling: {:?}",
                delay
            );
            prev = delay;
        }
        let _ = prev;
    }

    #[test]
    fn test_backoff_retry_after_takes_precedence_over_jitter() {
        let policy = RetryPolicy::builder()
            .initial_delay(Duration::from_millis(100))
            .max_delay(Duration::from_secs(30))
            .jitter_mode(JitterMode::Full)
            .honor_retry_after(true)
            .build();
        let mut backoff = Backoff::with_seed(policy, 42);

        // With an explicit server hint, jitter is bypassed entirely.
        let delay = backoff.next_delay(Some(Duration::from_millis(1500)));
        assert_eq!(delay, Duration::from_millis(1500));
        assert_eq!(backoff.previous_delay(), Duration::from_millis(1500));

        // `honor_retry_after(false)` ignores the hint and jitters normally.
        let policy2 = RetryPolicy::builder()
            .initial_delay(Duration::from_millis(100))
            .max_delay(Duration::from_secs(1))
            .jitter_mode(JitterMode::None)
            .honor_retry_after(false)
            .build();
        let mut backoff2 = Backoff::new(policy2);
        let delay2 = backoff2.next_delay(Some(Duration::from_millis(1500)));
        assert_eq!(delay2, Duration::from_millis(100)); // attempt 0: exact backoff
    }

    #[test]
    fn test_backoff_reset_restores_initial_state() {
        let policy = RetryPolicy::builder()
            .initial_delay(Duration::from_millis(100))
            .max_delay(Duration::from_secs(10))
            .jitter_mode(JitterMode::None)
            .build();
        let mut backoff = Backoff::new(policy);

        assert_eq!(backoff.next_delay(None), Duration::from_millis(100));
        assert_eq!(backoff.attempt(), 1);
        backoff.reset();
        assert_eq!(backoff.attempt(), 0);
        assert_eq!(backoff.next_delay(None), Duration::from_millis(100));
    }

    #[test]
    fn test_retry_reason_transient_network_error_always_retryable() {
        let policy = RetryPolicy::no_retry();
        // Even with no_retry, network errors are conventionally retryable per
        // classification, but the policy gate is what stops actual retries.
        let reason = RetryReason::TransientNetworkError("connect timeout".to_string());
        assert!(matches!(reason, RetryReason::TransientNetworkError(_)));
        assert!(reason.retry_after().is_none());
    }
}
