//! Turn rate limiting, LLM provider concurrency bounds, token consumption throttling,
//! priority queueing, and HTTP 429 exponential backoff with jitter.
//!
//! Provides:
//! 1. Dual Token Bucket Rate Limiting supporting Requests Per Minute (RPM) and Tokens Per Minute (TPM).
//! 2. Concurrency Limiting with RAII permit tracking to bound active in-flight provider calls.
//! 3. Exponential Backoff Retry with multiple Jitter Strategies on HTTP 429 Too Many Requests.
//! 4. HTTP `Retry-After` Header Parsing (seconds, fractional seconds, milliseconds, RFC 2822 dates).
//! 5. Priority Queue with Starvation Prevention (Aging) for User Interactive vs Subagent queries.
//! 6. Turn Rate Limiting (Sliding Window Log & Continuous Burst Token Bucket).
//! 7. Token Consumption & Financial Cost Quota Management (TPM, TPH, TPD, Session Caps).
//! 8. Speculative Token Reservations with commit/cancel lifecycle.
//! 9. Adaptive Micro-Pacing to smoothly distribute quota across time windows.
//! 10. Thread-safe Shared Engines and Async Dispatch Helpers.

use std::collections::VecDeque;
use std::fmt;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

// ===========================================================================
// 1. Quota & Throttle Types
// ===========================================================================

/// Types of rate limits and quota budgets enforced by the throttle engine.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum QuotaType {
    /// Maximum turns allowed per minute.
    TurnsPerMinute,
    /// Maximum turns allowed per hour.
    TurnsPerHour,
    /// Maximum total turns allowed in the current session.
    SessionTurns,
    /// Minimum cooldown interval required between consecutive turns.
    MinimumTurnInterval,
    /// Maximum total tokens consumed per minute (TPM).
    TokensPerMinute,
    /// Maximum total tokens consumed per hour (TPH).
    TokensPerHour,
    /// Maximum total tokens consumed per day (TPD).
    TokensPerDay,
    /// Maximum input/prompt tokens per minute.
    InputTokensPerMinute,
    /// Maximum output/completion tokens per minute.
    OutputTokensPerMinute,
    /// Hard cap on total tokens for the entire session.
    SessionTokens,
    /// Hard cap on financial cost (USD) for the entire session.
    SessionCostUsd,
    /// Hard cap on financial cost (USD) per hour.
    HourlyCostUsd,
    /// Hard cap on financial cost (USD) per day.
    DailyCostUsd,
    /// Maximum API requests allowed per minute (RPM).
    RequestsPerMinute,
    /// Maximum concurrent in-flight requests.
    ConcurrencyLimit,
    /// HTTP 429 provider backoff delay.
    Http429Backoff,
}

impl fmt::Display for QuotaType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TurnsPerMinute => write!(f, "Turns Per Minute (TPM)"),
            Self::TurnsPerHour => write!(f, "Turns Per Hour (TPH)"),
            Self::SessionTurns => write!(f, "Session Max Turns"),
            Self::MinimumTurnInterval => write!(f, "Turn Cooldown Interval"),
            Self::TokensPerMinute => write!(f, "Tokens Per Minute"),
            Self::TokensPerHour => write!(f, "Tokens Per Hour"),
            Self::TokensPerDay => write!(f, "Tokens Per Day"),
            Self::InputTokensPerMinute => write!(f, "Input Tokens Per Minute"),
            Self::OutputTokensPerMinute => write!(f, "Output Tokens Per Minute"),
            Self::SessionTokens => write!(f, "Session Token Budget"),
            Self::SessionCostUsd => write!(f, "Session Cost Limit (USD)"),
            Self::HourlyCostUsd => write!(f, "Hourly Cost Limit (USD)"),
            Self::DailyCostUsd => write!(f, "Daily Cost Limit (USD)"),
            Self::RequestsPerMinute => write!(f, "Requests Per Minute (RPM)"),
            Self::ConcurrencyLimit => write!(f, "Concurrency Limit"),
            Self::Http429Backoff => write!(f, "HTTP 429 Provider Backoff"),
        }
    }
}

/// Alert level for a quota budget.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, Default)]
pub enum QuotaLevel {
    /// Quota usage is within safe operating parameters (< soft threshold, e.g. < 80%).
    #[default]
    Normal,
    /// Quota usage reached soft threshold (>= 80% and < 95%). Warnings recommended.
    Warning,
    /// Quota usage reached danger threshold (>= 95% and < 100%). Pacing recommended.
    Danger,
    /// Quota limit completely exhausted (>= 100%). Operations blocked.
    Exhausted,
}

impl fmt::Display for QuotaLevel {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Normal => write!(f, "Normal (Safe)"),
            Self::Warning => write!(f, "Warning (Approaching Limit)"),
            Self::Danger => write!(f, "Danger (Near Exhaustion)"),
            Self::Exhausted => write!(f, "Exhausted (Limit Reached)"),
        }
    }
}

/// Policy governing how the throttler behaves when limits are approached or exceeded.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum ThrottlePolicy {
    /// Strictly block and return an error when any limit is exceeded.
    #[default]
    StrictReject,
    /// Advise or sleep until the quota window resets / tokens replenish.
    WaitAndRetry,
    /// Introduce progressive adaptive micro-delays as usage approaches limits.
    AdaptivePacing,
    /// Never block operations; emit warnings and collect violation metrics only.
    WarnOnly,
}

/// Decision returned by a throttle check.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum ThrottleDecision {
    /// Turn or token request is permitted.
    Allowed {
        /// Remaining tokens in the primary quota window (if applicable).
        remaining_tokens: Option<u64>,
        /// Optional advisory pacing delay recommended for smooth consumption.
        pacing_delay_ms: Option<u64>,
        /// Warning message if approaching quota threshold.
        warning: Option<String>,
    },
    /// Request is temporarily rate-limited; retry after the specified duration.
    Throttled {
        /// Time duration required to wait before retrying.
        wait_duration_ms: u64,
        /// The specific rate limit or interval that triggered throttling.
        quota_type: QuotaType,
        /// Human-readable explanation.
        reason: String,
    },
    /// Hard quota or session budget is fully exhausted.
    HardExhausted {
        /// The exhausted quota type.
        quota_type: QuotaType,
        /// Configured limit.
        limit: f64,
        /// Amount already consumed.
        used: f64,
        /// Time until quota resets (None for permanent session budgets).
        reset_in_ms: Option<u64>,
    },
}

impl ThrottleDecision {
    /// Returns `true` if the decision allows proceeding immediately without error.
    pub fn is_allowed(&self) -> bool {
        matches!(self, Self::Allowed { .. })
    }

    /// Returns `true` if the decision indicates hard quota exhaustion.
    pub fn is_exhausted(&self) -> bool {
        matches!(self, Self::HardExhausted { .. })
    }

    /// Returns the required or suggested wait duration in milliseconds, if any.
    pub fn wait_duration_ms(&self) -> Option<u64> {
        match self {
            Self::Allowed { pacing_delay_ms, .. } => *pacing_delay_ms,
            Self::Throttled { wait_duration_ms, .. } => Some(*wait_duration_ms),
            Self::HardExhausted { reset_in_ms, .. } => *reset_in_ms,
        }
    }
}

/// Errors raised when rate limits or quotas are violated under strict policies.
#[derive(Debug, Clone, thiserror::Error)]
pub enum ThrottleError {
    #[error("Turn rate limit exceeded ({limit} turns per {window_secs}s). Retry after {retry_after_ms}ms")]
    TurnRateLimitExceeded {
        limit: u32,
        window_secs: u64,
        retry_after_ms: u64,
    },

    #[error("Turn cooldown violation: minimum interval is {required_interval_ms}ms (elapsed: {elapsed_ms}ms). Retry after {wait_ms}ms")]
    MinimumIntervalViolation {
        required_interval_ms: u64,
        elapsed_ms: u64,
        wait_ms: u64,
    },

    #[error("Token rate limit exceeded for {quota_type}: limit={limit}, requested={requested}. Retry after {retry_after_ms}ms")]
    TokenRateLimitExceeded {
        quota_type: QuotaType,
        limit: u64,
        requested: u64,
        retry_after_ms: u64,
    },

    #[error("Session budget exhausted for {quota_type}: limit={limit}, used={used}")]
    SessionQuotaExhausted {
        quota_type: QuotaType,
        limit: u64,
        used: u64,
    },

    #[error("Cost budget limit exceeded: limit=${limit_usd:.4}, spent=${spent_usd:.4}")]
    CostLimitExceeded {
        limit_usd: f64,
        spent_usd: f64,
    },

    #[error("Token reservation ticket #{ticket_id} was not found or has expired")]
    ReservationNotFound {
        ticket_id: u64,
    },

    #[error("Concurrency limit exceeded: maximum {max_concurrency} in-flight requests reached (currently {active_in_flight} active)")]
    ConcurrencyLimitExceeded {
        max_concurrency: usize,
        active_in_flight: usize,
    },

    #[error("Provider rate limited (HTTP 429): retry after {retry_after_ms}ms (attempt {attempt}/{max_retries})")]
    Http429RateLimited {
        retry_after_ms: u64,
        attempt: u32,
        max_retries: u32,
    },

    #[error("Priority queue is full: capacity={capacity}, current={current}")]
    QueueFull {
        capacity: usize,
        current: usize,
    },
}

// ===========================================================================
// 2. Configuration Builders
// ===========================================================================

/// Configuration for turn rate limiting and pacing.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TurnRateLimitConfig {
    /// Maximum turns permitted in any rolling 60-second window.
    pub max_turns_per_minute: Option<u32>,
    /// Maximum turns permitted in any rolling 3600-second (1-hour) window.
    pub max_turns_per_hour: Option<u32>,
    /// Maximum burst of turns allowed in immediate succession.
    pub burst_capacity: u32,
    /// Minimum cooldown interval required between consecutive turns (milliseconds).
    pub min_interval_ms: u64,
    /// Hard limit on the number of turns for the entire session.
    pub max_turns_per_session: Option<u32>,
}

impl Default for TurnRateLimitConfig {
    fn default() -> Self {
        Self {
            max_turns_per_minute: Some(30),
            max_turns_per_hour: Some(500),
            burst_capacity: 5,
            min_interval_ms: 200,
            max_turns_per_session: Some(100),
        }
    }
}

impl TurnRateLimitConfig {
    /// Creates a new unconstrained turn rate limit configuration.
    pub fn unlimited() -> Self {
        Self {
            max_turns_per_minute: None,
            max_turns_per_hour: None,
            burst_capacity: u32::MAX,
            min_interval_ms: 0,
            max_turns_per_session: None,
        }
    }

    /// Sets maximum turns per minute.
    pub fn with_turns_per_minute(mut self, max: u32) -> Self {
        self.max_turns_per_minute = Some(max);
        self
    }

    /// Sets maximum turns per hour.
    pub fn with_turns_per_hour(mut self, max: u32) -> Self {
        self.max_turns_per_hour = Some(max);
        self
    }

    /// Sets burst capacity.
    pub fn with_burst_capacity(mut self, burst: u32) -> Self {
        self.burst_capacity = burst;
        self
    }

    /// Sets minimum interval between turns in milliseconds.
    pub fn with_min_interval_ms(mut self, ms: u64) -> Self {
        self.min_interval_ms = ms;
        self
    }

    /// Sets maximum turns per session.
    pub fn with_max_turns_per_session(mut self, max: u32) -> Self {
        self.max_turns_per_session = Some(max);
        self
    }
}

/// Configuration for token rate limits, session budgets, and financial cost caps.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TokenQuotaConfig {
    /// Maximum total tokens per minute (TPM).
    pub max_tpm: Option<u64>,
    /// Maximum total tokens per hour (TPH).
    pub max_tph: Option<u64>,
    /// Maximum total tokens per day (TPD).
    pub max_tpd: Option<u64>,
    /// Maximum input/prompt tokens per minute.
    pub max_input_tpm: Option<u64>,
    /// Maximum output/completion tokens per minute.
    pub max_output_tpm: Option<u64>,
    /// Hard token budget for the entire session.
    pub session_token_budget: Option<u64>,
    /// Soft quota threshold ratio (default 0.80 = 80%).
    pub soft_quota_ratio: f32,
    /// Danger quota threshold ratio (default 0.95 = 95%).
    pub danger_quota_ratio: f32,
    /// Maximum financial cost (USD) for the entire session.
    pub cost_limit_usd: Option<f64>,
    /// Maximum financial cost (USD) per hour.
    pub cost_per_hour_usd: Option<f64>,
    /// Maximum financial cost (USD) per day.
    pub cost_per_day_usd: Option<f64>,
}

impl Default for TokenQuotaConfig {
    fn default() -> Self {
        Self {
            max_tpm: Some(200_000),
            max_tph: Some(2_000_000),
            max_tpd: Some(20_000_000),
            max_input_tpm: Some(160_000),
            max_output_tpm: Some(40_000),
            session_token_budget: Some(1_000_000),
            soft_quota_ratio: 0.80,
            danger_quota_ratio: 0.95,
            cost_limit_usd: Some(10.0),
            cost_per_hour_usd: Some(5.0),
            cost_per_day_usd: Some(25.0),
        }
    }
}

impl TokenQuotaConfig {
    /// Creates an unlimited token quota configuration.
    pub fn unlimited() -> Self {
        Self {
            max_tpm: None,
            max_tph: None,
            max_tpd: None,
            max_input_tpm: None,
            max_output_tpm: None,
            session_token_budget: None,
            soft_quota_ratio: 0.80,
            danger_quota_ratio: 0.95,
            cost_limit_usd: None,
            cost_per_hour_usd: None,
            cost_per_day_usd: None,
        }
    }

    /// Sets Tokens Per Minute (TPM).
    pub fn with_max_tpm(mut self, tpm: u64) -> Self {
        self.max_tpm = Some(tpm);
        self
    }

    /// Sets Tokens Per Hour (TPH).
    pub fn with_max_tph(mut self, tph: u64) -> Self {
        self.max_tph = Some(tph);
        self
    }

    /// Sets Tokens Per Day (TPD).
    pub fn with_max_tpd(mut self, tpd: u64) -> Self {
        self.max_tpd = Some(tpd);
        self
    }

    /// Sets Input Tokens Per Minute.
    pub fn with_input_tpm(mut self, input_tpm: u64) -> Self {
        self.max_input_tpm = Some(input_tpm);
        self
    }

    /// Sets Output Tokens Per Minute.
    pub fn with_output_tpm(mut self, output_tpm: u64) -> Self {
        self.max_output_tpm = Some(output_tpm);
        self
    }

    /// Sets Session Token Budget.
    pub fn with_session_budget(mut self, budget: u64) -> Self {
        self.session_token_budget = Some(budget);
        self
    }

    /// Sets Financial Cost Limits (USD).
    pub fn with_cost_limits(mut self, session: Option<f64>, hourly: Option<f64>, daily: Option<f64>) -> Self {
        self.cost_limit_usd = session;
        self.cost_per_hour_usd = hourly;
        self.cost_per_day_usd = daily;
        self
    }

    /// Sets warning and danger threshold ratios.
    pub fn with_thresholds(mut self, soft: f32, danger: f32) -> Self {
        self.soft_quota_ratio = soft.clamp(0.1, 0.99);
        self.danger_quota_ratio = danger.clamp(self.soft_quota_ratio, 0.999);
        self
    }
}

/// Comprehensive throttle configuration combining turn limits, token quotas, and policies.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ThrottleConfig {
    /// Turn rate limiting settings.
    pub turns: TurnRateLimitConfig,
    /// Token consumption and cost quota settings.
    pub tokens: TokenQuotaConfig,
    /// Operational enforcement policy.
    pub policy: ThrottlePolicy,
    /// Whether adaptive micro-pacing is enabled.
    pub adaptive_pacing: bool,
    /// Maximum delay in milliseconds that adaptive pacing can insert per turn.
    pub max_pacing_delay_ms: u64,
}

impl Default for ThrottleConfig {
    fn default() -> Self {
        Self {
            turns: TurnRateLimitConfig::default(),
            tokens: TokenQuotaConfig::default(),
            policy: ThrottlePolicy::StrictReject,
            adaptive_pacing: true,
            max_pacing_delay_ms: 3_000,
        }
    }
}

impl ThrottleConfig {
    /// Creates an unlimited configuration without throttling.
    pub fn unlimited() -> Self {
        Self {
            turns: TurnRateLimitConfig::unlimited(),
            tokens: TokenQuotaConfig::unlimited(),
            policy: ThrottlePolicy::WarnOnly,
            adaptive_pacing: false,
            max_pacing_delay_ms: 0,
        }
    }

    /// Creates a conservative configuration suitable for low-tier or rate-limited API keys.
    pub fn conservative() -> Self {
        Self {
            turns: TurnRateLimitConfig {
                max_turns_per_minute: Some(15),
                max_turns_per_hour: Some(200),
                burst_capacity: 3,
                min_interval_ms: 1_000,
                max_turns_per_session: Some(50),
            },
            tokens: TokenQuotaConfig {
                max_tpm: Some(50_000),
                max_tph: Some(500_000),
                max_tpd: Some(5_000_000),
                max_input_tpm: Some(40_000),
                max_output_tpm: Some(10_000),
                session_token_budget: Some(250_000),
                soft_quota_ratio: 0.75,
                danger_quota_ratio: 0.90,
                cost_limit_usd: Some(3.0),
                cost_per_hour_usd: Some(1.5),
                cost_per_day_usd: Some(10.0),
            },
            policy: ThrottlePolicy::AdaptivePacing,
            adaptive_pacing: true,
            max_pacing_delay_ms: 5_000,
        }
    }

    /// Creates a strict budget configuration with a given token cap and USD limit.
    pub fn strict_budget(session_tokens: u64, session_cost_usd: f64) -> Self {
        Self {
            turns: TurnRateLimitConfig::default(),
            tokens: TokenQuotaConfig {
                session_token_budget: Some(session_tokens),
                cost_limit_usd: Some(session_cost_usd),
                ..TokenQuotaConfig::default()
            },
            policy: ThrottlePolicy::StrictReject,
            adaptive_pacing: true,
            max_pacing_delay_ms: 3_000,
        }
    }

    /// Sets the throttle policy.
    pub fn with_policy(mut self, policy: ThrottlePolicy) -> Self {
        self.policy = policy;
        self
    }

    /// Enables or disables adaptive pacing.
    pub fn with_adaptive_pacing(mut self, enabled: bool) -> Self {
        self.adaptive_pacing = enabled;
        self
    }
}

/// Configuration for provider LLM call rates (RPM/TPM) and concurrency bounds.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RpmTpmConfig {
    /// Maximum requests allowed per minute (RPM).
    pub max_rpm: Option<u32>,
    /// Maximum tokens allowed per minute (TPM).
    pub max_tpm: Option<u64>,
    /// Burst capacity for RPM bucket (defaults to max_rpm if not specified).
    pub rpm_burst: Option<u32>,
    /// Burst capacity for TPM bucket (defaults to max_tpm if not specified).
    pub tpm_burst: Option<u64>,
    /// Maximum simultaneous active in-flight requests.
    pub max_concurrency: Option<usize>,
}

impl Default for RpmTpmConfig {
    fn default() -> Self {
        Self {
            max_rpm: Some(60),
            max_tpm: Some(100_000),
            rpm_burst: Some(10),
            tpm_burst: Some(25_000),
            max_concurrency: Some(5),
        }
    }
}

impl RpmTpmConfig {
    /// Creates an unlimited RPM/TPM configuration without limits.
    pub fn unlimited() -> Self {
        Self {
            max_rpm: None,
            max_tpm: None,
            rpm_burst: None,
            tpm_burst: None,
            max_concurrency: None,
        }
    }

    /// Sets Requests Per Minute (RPM).
    pub fn with_rpm(mut self, rpm: u32) -> Self {
        self.max_rpm = Some(rpm);
        self
    }

    /// Sets Tokens Per Minute (TPM).
    pub fn with_tpm(mut self, tpm: u64) -> Self {
        self.max_tpm = Some(tpm);
        self
    }

    /// Sets custom burst capacities.
    pub fn with_burst(mut self, rpm_burst: u32, tpm_burst: u64) -> Self {
        self.rpm_burst = Some(rpm_burst);
        self.tpm_burst = Some(tpm_burst);
        self
    }

    /// Sets maximum allowed concurrency.
    pub fn with_max_concurrency(mut self, max: usize) -> Self {
        self.max_concurrency = Some(max);
        self
    }
}

/// Strategy for injecting randomized jitter into backoff retry calculations.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum JitterStrategy {
    /// Full jitter: uniformly distributed between 0 and calculated exponential backoff.
    #[default]
    Full,
    /// Equal jitter: half deterministic backoff, half randomized jitter.
    Equal,
    /// Decorrelated jitter: randomized based on previous backoff interval.
    Decorrelated,
    /// Proportional jitter: +/- 25% around the base exponential curve.
    Proportional,
    /// No jitter: exact exponential curve without randomization.
    None,
}

/// Configuration for exponential backoff and retry behavior upon receiving HTTP 429 errors.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BackoffConfig {
    /// Initial retry backoff interval in milliseconds (default: 500ms).
    pub initial_delay_ms: u64,
    /// Maximum retry backoff interval cap in milliseconds (default: 60,000ms = 60s).
    pub max_delay_ms: u64,
    /// Exponential base multiplier (default: 2.0).
    pub multiplier: f64,
    /// Jitter strategy used to avoid synchronized thundering-herd retries.
    pub jitter: JitterStrategy,
    /// Maximum number of retry attempts before reporting failure (default: 5).
    pub max_retries: u32,
    /// Whether to honor `Retry-After` HTTP response headers when provided by the provider.
    pub respect_retry_after: bool,
}

impl Default for BackoffConfig {
    fn default() -> Self {
        Self {
            initial_delay_ms: 500,
            max_delay_ms: 60_000,
            multiplier: 2.0,
            jitter: JitterStrategy::Full,
            max_retries: 5,
            respect_retry_after: true,
        }
    }
}

impl BackoffConfig {
    /// Sets initial backoff delay in milliseconds.
    pub fn with_initial_delay_ms(mut self, ms: u64) -> Self {
        self.initial_delay_ms = ms;
        self
    }

    /// Sets maximum backoff delay cap in milliseconds.
    pub fn with_max_delay_ms(mut self, ms: u64) -> Self {
        self.max_delay_ms = ms;
        self
    }

    /// Sets the exponential multiplier.
    pub fn with_multiplier(mut self, multiplier: f64) -> Self {
        self.multiplier = multiplier.max(1.0);
        self
    }

    /// Sets jitter strategy.
    pub fn with_jitter(mut self, jitter: JitterStrategy) -> Self {
        self.jitter = jitter;
        self
    }

    /// Sets maximum retry attempts.
    pub fn with_max_retries(mut self, max: u32) -> Self {
        self.max_retries = max;
        self
    }

    /// Configures whether to respect the Retry-After header.
    pub fn with_respect_retry_after(mut self, respect: bool) -> Self {
        self.respect_retry_after = respect;
        self
    }
}

// ===========================================================================
// 3. Sliding Window Log & Token Bucket Implementations
// ===========================================================================

/// An entry in a sliding time window log.
#[derive(Debug, Clone, Copy)]
struct LogEntry<T> {
    instant: Instant,
    value: T,
}

/// High-precision sliding window log for tracking events (turns, tokens, cost) over duration windows.
#[derive(Debug, Clone)]
pub struct SlidingWindowLog<T: Copy + std::ops::Add<Output = T> + Default> {
    entries: VecDeque<LogEntry<T>>,
}

impl<T: Copy + std::ops::Add<Output = T> + Default> Default for SlidingWindowLog<T> {
    fn default() -> Self {
        Self {
            entries: VecDeque::new(),
        }
    }
}

impl<T: Copy + std::ops::Add<Output = T> + Default> SlidingWindowLog<T> {
    /// Creates a new empty sliding window log.
    pub fn new() -> Self {
        Self::default()
    }

    /// Appends a new value recorded at the current instant.
    pub fn record(&mut self, value: T) {
        self.record_at(Instant::now(), value);
    }

    /// Appends a new value at a specific instant.
    pub fn record_at(&mut self, instant: Instant, value: T) {
        self.entries.push_back(LogEntry { instant, value });
    }

    /// Purges entries that occurred before `(now - window)`.
    pub fn purge_older_than(&mut self, window: Duration) {
        let now = Instant::now();
        while let Some(front) = self.entries.front() {
            if now.duration_since(front.instant) > window {
                self.entries.pop_front();
            } else {
                break;
            }
        }
    }

    /// Returns the number of events recorded within the specified rolling window.
    pub fn count_in_window(&mut self, window: Duration) -> usize {
        self.purge_older_than(window);
        self.entries.len()
    }

    /// Sums all recorded values within the specified rolling window.
    pub fn sum_in_window(&mut self, window: Duration) -> T {
        self.purge_older_than(window);
        let mut total = T::default();
        for entry in &self.entries {
            total = total + entry.value;
        }
        total
    }

    /// Calculates how long until the oldest entry expires from the window.
    pub fn time_until_oldest_expires(&mut self, window: Duration) -> Option<Duration> {
        self.purge_older_than(window);
        self.entries.front().map(|front| {
            let elapsed = Instant::now().duration_since(front.instant);
            if elapsed < window {
                window - elapsed
            } else {
                Duration::ZERO
            }
        })
    }

    /// Clears all entries.
    pub fn clear(&mut self) {
        self.entries.clear();
    }
}

/// Token Bucket algorithm implementation supporting continuous rate limiting and burst allowances.
#[derive(Debug, Clone)]
pub struct TokenBucket {
    /// Maximum capacity of the bucket (tokens or requests).
    capacity: f64,
    /// Currently available tokens in the bucket.
    tokens: f64,
    /// Continuous refill rate in tokens per second.
    refill_rate_per_sec: f64,
    /// Last instant the bucket was refilled.
    last_refill: Instant,
}

impl TokenBucket {
    /// Creates a new TokenBucket with given capacity and continuous refill rate per second.
    pub fn new(capacity: f64, refill_rate_per_sec: f64) -> Self {
        Self {
            capacity: capacity.max(0.0),
            tokens: capacity.max(0.0),
            refill_rate_per_sec: refill_rate_per_sec.max(0.0),
            last_refill: Instant::now(),
        }
    }

    /// Creates a TokenBucket configured for Requests Per Minute (RPM).
    pub fn with_rpm(rpm: u32, burst_capacity: Option<u32>) -> Self {
        let rate_per_sec = (rpm as f64) / 60.0;
        let cap = burst_capacity.unwrap_or(rpm.max(1)) as f64;
        Self::new(cap, rate_per_sec)
    }

    /// Creates a TokenBucket configured for Tokens Per Minute (TPM).
    pub fn with_tpm(tpm: u64, burst_capacity: Option<u64>) -> Self {
        let rate_per_sec = (tpm as f64) / 60.0;
        let cap = burst_capacity.unwrap_or(tpm.max(1)) as f64;
        Self::new(cap, rate_per_sec)
    }

    /// Refills the bucket based on elapsed time since the last refill.
    pub fn refill(&mut self) {
        self.refill_at(Instant::now());
    }

    /// Refills the bucket relative to a specific timestamp.
    pub fn refill_at(&mut self, now: Instant) {
        if now > self.last_refill {
            let elapsed_secs = now.duration_since(self.last_refill).as_secs_f64();
            if elapsed_secs > 0.0 && self.refill_rate_per_sec > 0.0 {
                let added = elapsed_secs * self.refill_rate_per_sec;
                self.tokens = (self.tokens + added).min(self.capacity);
            }
            self.last_refill = now;
        }
    }

    /// Attempts to consume `amount` tokens immediately. Returns `true` if successful.
    pub fn try_consume(&mut self, amount: f64) -> bool {
        self.refill();
        if self.tokens >= amount {
            self.tokens -= amount;
            true
        } else {
            false
        }
    }

    /// Consumes tokens or returns the duration to wait until enough tokens are available.
    pub fn consume(&mut self, amount: f64) -> Result<(), Duration> {
        self.refill();
        if self.tokens >= amount {
            self.tokens -= amount;
            Ok(())
        } else {
            Err(self.time_until_available(amount))
        }
    }

    /// Refunds previously consumed tokens back into the bucket (capped at capacity).
    pub fn refund(&mut self, amount: f64) {
        self.refill();
        self.tokens = (self.tokens + amount.max(0.0)).min(self.capacity);
    }

    /// Returns the number of currently available tokens after refilling.
    pub fn available_tokens(&mut self) -> f64 {
        self.refill();
        self.tokens
    }

    /// Calculates how long until `amount` tokens become available.
    pub fn time_until_available(&mut self, amount: f64) -> Duration {
        self.refill();
        if self.tokens >= amount {
            return Duration::ZERO;
        }
        if self.refill_rate_per_sec <= 0.0 {
            return Duration::from_secs(3600);
        }
        let needed = amount - self.tokens;
        let secs = needed / self.refill_rate_per_sec;
        Duration::from_secs_f64(secs)
    }

    /// Returns the maximum capacity of the bucket.
    pub fn capacity(&self) -> f64 {
        self.capacity
    }

    /// Returns the continuous refill rate in tokens per second.
    pub fn refill_rate_per_sec(&self) -> f64 {
        self.refill_rate_per_sec
    }

    /// Updates the capacity.
    pub fn set_capacity(&mut self, capacity: f64) {
        self.capacity = capacity.max(0.0);
        self.tokens = self.tokens.min(self.capacity);
    }

    /// Updates the refill rate per second.
    pub fn set_refill_rate_per_sec(&mut self, rate: f64) {
        self.refill();
        self.refill_rate_per_sec = rate.max(0.0);
    }

    /// Resets the bucket to full capacity.
    pub fn reset(&mut self) {
        self.tokens = self.capacity;
        self.last_refill = Instant::now();
    }
}

// ===========================================================================
// 4. Concurrency Limiter
// ===========================================================================

/// Tracks and bounds simultaneous in-flight LLM calls to prevent socket starvation and provider overload.
#[derive(Debug, Clone)]
pub struct ConcurrencyLimiter {
    max_concurrency: usize,
    in_flight: Arc<Mutex<usize>>,
    peak_in_flight: Arc<Mutex<usize>>,
    total_acquired: Arc<Mutex<u64>>,
    total_rejected: Arc<Mutex<u64>>,
}

/// RAII permit for an active in-flight request. Automatically decrements the in-flight counter when dropped.
#[derive(Debug)]
pub struct ConcurrencyPermit {
    in_flight: Arc<Mutex<usize>>,
    active: bool,
}

impl ConcurrencyPermit {
    /// Creates a standalone / dummy permit (e.g. for unlimited concurrency).
    pub fn dummy() -> Self {
        Self {
            in_flight: Arc::new(Mutex::new(0)),
            active: false,
        }
    }

    /// Explicitly releases the permit early.
    pub fn release(mut self) {
        self.deactivate();
    }

    fn deactivate(&mut self) {
        if self.active {
            self.active = false;
            if let Ok(mut count) = self.in_flight.lock() {
                if *count > 0 {
                    *count -= 1;
                }
            }
        }
    }
}

impl Drop for ConcurrencyPermit {
    fn drop(&mut self) {
        self.deactivate();
    }
}

impl ConcurrencyLimiter {
    /// Creates a new ConcurrencyLimiter with a given maximum concurrency limit.
    pub fn new(max_concurrency: usize) -> Self {
        Self {
            max_concurrency: max_concurrency.max(1),
            in_flight: Arc::new(Mutex::new(0)),
            peak_in_flight: Arc::new(Mutex::new(0)),
            total_acquired: Arc::new(Mutex::new(0)),
            total_rejected: Arc::new(Mutex::new(0)),
        }
    }

    /// Creates an unlimited concurrency limiter.
    pub fn unlimited() -> Self {
        Self::new(usize::MAX)
    }

    /// Attempts to acquire a concurrency permit immediately.
    pub fn try_acquire(&self) -> Result<ConcurrencyPermit, ThrottleError> {
        let mut count = self.in_flight.lock().map_err(|_| ThrottleError::ConcurrencyLimitExceeded {
            max_concurrency: self.max_concurrency,
            active_in_flight: self.max_concurrency,
        })?;

        if *count < self.max_concurrency {
            *count += 1;
            let current = *count;
            if let Ok(mut peak) = self.peak_in_flight.lock() {
                if current > *peak {
                    *peak = current;
                }
            }
            if let Ok(mut total) = self.total_acquired.lock() {
                *total += 1;
            }
            Ok(ConcurrencyPermit {
                in_flight: Arc::clone(&self.in_flight),
                active: true,
            })
        } else {
            if let Ok(mut rej) = self.total_rejected.lock() {
                *rej += 1;
            }
            Err(ThrottleError::ConcurrencyLimitExceeded {
                max_concurrency: self.max_concurrency,
                active_in_flight: *count,
            })
        }
    }

    /// Returns current in-flight requests count.
    pub fn in_flight(&self) -> usize {
        self.in_flight.lock().map(|g| *g).unwrap_or(0)
    }

    /// Returns peak in-flight requests observed.
    pub fn peak_in_flight(&self) -> usize {
        self.peak_in_flight.lock().map(|g| *g).unwrap_or(0)
    }

    /// Returns whether the limiter currently has available slots.
    pub fn has_capacity(&self) -> bool {
        self.in_flight() < self.max_concurrency
    }

    /// Returns the maximum allowed concurrency.
    pub fn max_concurrency(&self) -> usize {
        self.max_concurrency
    }

    /// Returns total permits successfully acquired over lifetime.
    pub fn total_acquired(&self) -> u64 {
        self.total_acquired.lock().map(|g| *g).unwrap_or(0)
    }

    /// Returns total requests rejected due to concurrency limit exhaustion.
    pub fn total_rejected(&self) -> u64 {
        self.total_rejected.lock().map(|g| *g).unwrap_or(0)
    }
}

// ===========================================================================
// 5. Provider Rate Throttle (RPM & TPM Limiter)
// ===========================================================================

/// Dual Token Bucket rate limiter governing Requests Per Minute (RPM) and Tokens Per Minute (TPM).
#[derive(Debug, Clone)]
pub struct RpmTpmRateLimiter {
    config: RpmTpmConfig,
    rpm_bucket: Option<TokenBucket>,
    tpm_bucket: Option<TokenBucket>,
    concurrency_limiter: ConcurrencyLimiter,
    total_requests: u64,
    total_tokens: u64,
    throttled_count: u64,
}

impl Default for RpmTpmRateLimiter {
    fn default() -> Self {
        Self::new(RpmTpmConfig::default())
    }
}

impl RpmTpmRateLimiter {
    /// Creates a new RpmTpmRateLimiter from configuration.
    pub fn new(config: RpmTpmConfig) -> Self {
        let rpm_bucket = config.max_rpm.map(|rpm| {
            let burst = config.rpm_burst.unwrap_or(rpm.max(1));
            TokenBucket::with_rpm(rpm, Some(burst))
        });

        let tpm_bucket = config.max_tpm.map(|tpm| {
            let burst = config.tpm_burst.unwrap_or(tpm.max(1));
            TokenBucket::with_tpm(tpm, Some(burst))
        });

        let concurrency_limiter = match config.max_concurrency {
            Some(c) => ConcurrencyLimiter::new(c),
            None => ConcurrencyLimiter::unlimited(),
        };

        Self {
            config,
            rpm_bucket,
            tpm_bucket,
            concurrency_limiter,
            total_requests: 0,
            total_tokens: 0,
            throttled_count: 0,
        }
    }

    /// Creates an unlimited rate limiter.
    pub fn unlimited() -> Self {
        Self::new(RpmTpmConfig::unlimited())
    }

    /// Evaluates if a request of given token size is permitted without modifying state.
    pub fn check_availability(&mut self, requests: u32, tokens: u64) -> ThrottleDecision {
        // 1. Check concurrency capacity
        if !self.concurrency_limiter.has_capacity() {
            return ThrottleDecision::Throttled {
                wait_duration_ms: 100,
                quota_type: QuotaType::ConcurrencyLimit,
                reason: format!(
                    "Concurrency limit reached ({}/{} active requests)",
                    self.concurrency_limiter.in_flight(),
                    self.concurrency_limiter.max_concurrency()
                ),
            };
        }

        // 2. Check RPM bucket
        if let Some(bucket) = &mut self.rpm_bucket {
            if bucket.available_tokens() < requests as f64 {
                let wait = bucket.time_until_available(requests as f64);
                return ThrottleDecision::Throttled {
                    wait_duration_ms: wait.as_millis().max(1) as u64,
                    quota_type: QuotaType::RequestsPerMinute,
                    reason: format!(
                        "RPM limit exceeded: need {} request permits ({:.1} available)",
                        requests,
                        bucket.available_tokens()
                    ),
                };
            }
        }

        // 3. Check TPM bucket
        if let Some(bucket) = &mut self.tpm_bucket {
            if bucket.available_tokens() < tokens as f64 {
                let wait = bucket.time_until_available(tokens as f64);
                return ThrottleDecision::Throttled {
                    wait_duration_ms: wait.as_millis().max(1) as u64,
                    quota_type: QuotaType::TokensPerMinute,
                    reason: format!(
                        "TPM limit exceeded: need {} tokens ({:.0} available)",
                        tokens,
                        bucket.available_tokens()
                    ),
                };
            }
        }

        let remaining = self.tpm_bucket.as_mut().map(|b| b.available_tokens() as u64);
        ThrottleDecision::Allowed {
            remaining_tokens: remaining,
            pacing_delay_ms: None,
            warning: None,
        }
    }

    /// Calculates required wait time until both RPM and TPM buckets satisfy the request.
    pub fn time_until_available(&mut self, requests: u32, tokens: u64) -> Duration {
        let rpm_wait = self
            .rpm_bucket
            .as_mut()
            .map(|b| b.time_until_available(requests as f64))
            .unwrap_or(Duration::ZERO);

        let tpm_wait = self
            .tpm_bucket
            .as_mut()
            .map(|b| b.time_until_available(tokens as f64))
            .unwrap_or(Duration::ZERO);

        rpm_wait.max(tpm_wait)
    }

    /// Attempts to consume permits and acquire an in-flight concurrency handle.
    pub fn try_acquire(
        &mut self,
        requests: u32,
        tokens: u64,
    ) -> Result<ConcurrencyPermit, ThrottleDecision> {
        let decision = self.check_availability(requests, tokens);
        if !decision.is_allowed() {
            self.throttled_count += 1;
            return Err(decision);
        }

        // Try acquire concurrency permit
        let permit = match self.concurrency_limiter.try_acquire() {
            Ok(p) => p,
            Err(_) => {
                self.throttled_count += 1;
                return Err(ThrottleDecision::Throttled {
                    wait_duration_ms: 100,
                    quota_type: QuotaType::ConcurrencyLimit,
                    reason: "Concurrency limit reached".to_string(),
                });
            }
        };

        // Consume tokens from buckets
        if let Some(bucket) = &mut self.rpm_bucket {
            let _ = bucket.try_consume(requests as f64);
        }
        if let Some(bucket) = &mut self.tpm_bucket {
            let _ = bucket.try_consume(tokens as f64);
        }

        self.total_requests += requests as u64;
        self.total_tokens += tokens;

        Ok(permit)
    }

    /// Refunds unused speculative tokens back to the TPM bucket.
    pub fn refund(&mut self, requests: u32, tokens: u64) {
        if let Some(bucket) = &mut self.rpm_bucket {
            bucket.refund(requests as f64);
        }
        if let Some(bucket) = &mut self.tpm_bucket {
            bucket.refund(tokens as f64);
        }
        self.total_requests = self.total_requests.saturating_sub(requests as u64);
        self.total_tokens = self.total_tokens.saturating_sub(tokens);
    }

    /// Records additional token consumption beyond original speculative estimate.
    pub fn record_usage(&mut self, requests: u32, tokens: u64) {
        if let Some(bucket) = &mut self.rpm_bucket {
            let _ = bucket.try_consume(requests as f64);
        }
        if let Some(bucket) = &mut self.tpm_bucket {
            let _ = bucket.try_consume(tokens as f64);
        }
        self.total_requests += requests as u64;
        self.total_tokens += tokens;
    }

    /// Returns currently available RPM request capacity.
    pub fn available_rpm(&mut self) -> Option<f64> {
        self.rpm_bucket.as_mut().map(|b| b.available_tokens())
    }

    /// Returns currently available TPM token capacity.
    pub fn available_tpm(&mut self) -> Option<f64> {
        self.tpm_bucket.as_mut().map(|b| b.available_tokens())
    }

    /// Reference to the underlying concurrency limiter.
    pub fn concurrency_limiter(&self) -> &ConcurrencyLimiter {
        &self.concurrency_limiter
    }

    /// Returns total lifetime requests served.
    pub fn total_requests(&self) -> u64 {
        self.total_requests
    }

    /// Returns total lifetime tokens consumed.
    pub fn total_tokens(&self) -> u64 {
        self.total_tokens
    }

    /// Returns total throttled events count.
    pub fn throttled_count(&self) -> u64 {
        self.throttled_count
    }

    /// Resets buckets and metrics.
    pub fn reset(&mut self) {
        if let Some(bucket) = &mut self.rpm_bucket {
            bucket.reset();
        }
        if let Some(bucket) = &mut self.tpm_bucket {
            bucket.reset();
        }
        self.total_requests = 0;
        self.total_tokens = 0;
        self.throttled_count = 0;
    }
}

// ===========================================================================
// 6. Exponential Backoff & HTTP 429 Handler
// ===========================================================================

/// Fast, lightweight, pure-Rust pseudo-random number generator (XorShift64).
#[derive(Debug, Clone)]
pub struct FastRng {
    state: u64,
}

impl FastRng {
    /// Creates a FastRng from an explicit seed.
    pub fn new(seed: u64) -> Self {
        Self {
            state: if seed == 0 { 0x853c49e6748fea9b } else { seed },
        }
    }

    /// Creates a FastRng seeded from the current timestamp.
    pub fn from_time() -> Self {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos() as u64)
            .unwrap_or(0x123456789abcdef0);
        Self::new(nanos)
    }

    /// Generates next pseudo-random u64.
    pub fn next_u64(&mut self) -> u64 {
        let mut x = self.state;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.state = x;
        x
    }

    /// Generates a float in `[0.0, 1.0)`.
    pub fn next_f64(&mut self) -> f64 {
        (self.next_u64() >> 11) as f64 / (1u64 << 53) as f64
    }

    /// Generates a float in range `[min, max)`.
    pub fn gen_range(&mut self, min: f64, max: f64) -> f64 {
        if min >= max {
            min
        } else {
            min + (max - min) * self.next_f64()
        }
    }
}

/// Exponential backoff calculator with jitter strategies.
pub struct ExponentialBackoff;

impl ExponentialBackoff {
    /// Calculates the backoff delay for attempt index `attempt` (0-based) using the configured jitter strategy.
    pub fn calculate_delay(attempt: u32, config: &BackoffConfig, rng: &mut FastRng) -> Duration {
        let base_ms = (config.initial_delay_ms as f64) * config.multiplier.powi(attempt as i32);
        let base_ms = base_ms.min(config.max_delay_ms as f64);

        let final_ms = match config.jitter {
            JitterStrategy::Full => rng.gen_range(0.0, base_ms),
            JitterStrategy::Equal => {
                let half = base_ms / 2.0;
                half + rng.gen_range(0.0, half)
            }
            JitterStrategy::Decorrelated => {
                let prev = (config.initial_delay_ms as f64)
                    * config.multiplier.powi(attempt.saturating_sub(1) as i32);
                let upper = (prev * 3.0).min(config.max_delay_ms as f64);
                rng.gen_range(config.initial_delay_ms as f64, upper.max(config.initial_delay_ms as f64))
            }
            JitterStrategy::Proportional => {
                let factor = rng.gen_range(0.75, 1.25);
                base_ms * factor
            }
            JitterStrategy::None => base_ms,
        };

        let clamped_ms = final_ms.clamp(0.0, config.max_delay_ms as f64);
        Duration::from_millis(clamped_ms as u64)
    }

    /// Deterministic calculation with explicit seed (ideal for unit testing).
    pub fn calculate_delay_deterministic(attempt: u32, config: &BackoffConfig, seed: u64) -> Duration {
        let mut rng = FastRng::new(seed);
        Self::calculate_delay(attempt, config, &mut rng)
    }
}

/// Helper for HTTP 429 Too Many Requests response handling and `Retry-After` header parsing.
pub struct Http429Handler;

impl Http429Handler {
    /// Parses HTTP `Retry-After` header values (integer seconds, float seconds, milliseconds, or RFC 2822 dates).
    pub fn parse_retry_after(header_val: &str) -> Option<Duration> {
        let trimmed = header_val.trim();
        if trimmed.is_empty() {
            return None;
        }

        // 1. Integer seconds (e.g. "15")
        if let Ok(secs) = trimmed.parse::<u64>() {
            return Some(Duration::from_secs(secs));
        }

        // 2. Fractional seconds (e.g. "1.5")
        if let Ok(secs_f) = trimmed.parse::<f64>() {
            if secs_f >= 0.0 {
                return Some(Duration::from_secs_f64(secs_f));
            }
        }

        // 3. Millisecond notation (e.g. "500ms" or "500 ms")
        let lower = trimmed.to_ascii_lowercase();
        if lower.ends_with("ms") {
            let num_part = lower.trim_end_matches("ms").trim();
            if let Ok(ms) = num_part.parse::<u64>() {
                return Some(Duration::from_millis(ms));
            }
            if let Ok(ms_f) = num_part.parse::<f64>() {
                if ms_f >= 0.0 {
                    return Some(Duration::from_secs_f64(ms_f / 1000.0));
                }
            }
        }

        // 4. RFC 2822 HTTP-date (e.g. "Wed, 21 Oct 2026 07:28:00 GMT")
        if let Ok(dt) = DateTime::parse_from_rfc2822(trimmed) {
            let target_utc = dt.with_timezone(&Utc);
            let now_utc = Utc::now();
            if target_utc > now_utc {
                if let Ok(diff) = (target_utc - now_utc).to_std() {
                    return Some(diff);
                }
            } else {
                return Some(Duration::from_millis(100));
            }
        }

        None
    }

    /// Computes effective backoff delay considering retry attempt and optional HTTP 429 Retry-After header.
    pub fn compute_backoff(attempt: u32, retry_after_header: Option<&str>, config: &BackoffConfig) -> Duration {
        if config.respect_retry_after {
            if let Some(header) = retry_after_header {
                if let Some(parsed_duration) = Self::parse_retry_after(header) {
                    let mut rng = FastRng::from_time();
                    let jitter_factor = rng.gen_range(1.0, 1.10);
                    let jittered_secs = parsed_duration.as_secs_f64() * jitter_factor;
                    let capped = jittered_secs.min(config.max_delay_ms as f64 / 1000.0);
                    return Duration::from_secs_f64(capped);
                }
            }
        }

        let mut rng = FastRng::from_time();
        ExponentialBackoff::calculate_delay(attempt, config, &mut rng)
    }

    /// Returns `true` if an HTTP status code represents a transient / retryable condition.
    pub fn is_retryable_status(status_code: u16) -> bool {
        matches!(status_code, 429 | 408 | 500 | 502 | 503 | 504)
    }
}

// ===========================================================================
// 7. Priority Queue & Request Scheduling
// ===========================================================================

/// Scheduling priority tier for LLM provider requests.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum RequestPriority {
    /// Low priority background batch operations (compaction, telemetry, indexing, advisor background checks).
    BackgroundBatch,
    /// Medium priority subagent worker execution tasks.
    SubagentWorker,
    /// High priority direct tool executions triggered by active user command.
    DirectTool,
    /// Highest priority interactive prompt in chat/CLI/TUI.
    UserInteractive,
    /// Custom explicit priority level (0-255).
    Custom(u8),
}

impl RequestPriority {
    /// Numeric rank for comparison and starvation calculations (higher is higher priority).
    pub fn rank(&self) -> u32 {
        match self {
            Self::BackgroundBatch => 10,
            Self::SubagentWorker => 20,
            Self::DirectTool => 30,
            Self::UserInteractive => 40,
            Self::Custom(n) => *n as u32,
        }
    }
}

impl PartialOrd for RequestPriority {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for RequestPriority {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.rank().cmp(&other.rank())
    }
}

/// A queued request item in the priority throttle queue.
#[derive(Debug, Clone)]
pub struct QueuedRequest<T> {
    /// Unique item identifier.
    pub id: u64,
    /// Scheduling priority.
    pub priority: RequestPriority,
    /// Speculative estimated tokens required.
    pub estimated_tokens: u64,
    /// Estimated request count (typically 1).
    pub estimated_requests: u32,
    /// Monotonic instant when item was enqueued.
    pub enqueued_at: Instant,
    /// Wall-clock timestamp when enqueued.
    pub enqueued_utc: DateTime<Utc>,
    /// Identifier of the caller / subagent.
    pub caller_id: String,
    /// Request payload.
    pub payload: T,
}

/// Multi-tier priority queue with starvation prevention (aging) for prompt dispatching.
#[derive(Debug, Clone)]
pub struct PriorityThrottleQueue<T> {
    requests: Vec<QueuedRequest<T>>,
    next_id: u64,
    /// Milliseconds before a queued request gains aging priority to prevent starvation.
    pub aging_threshold_ms: u64,
    /// Numerical rank boost added for each aging step passed.
    pub aging_boost: u32,
}

impl<T> Default for PriorityThrottleQueue<T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T> PriorityThrottleQueue<T> {
    /// Creates a new priority queue with standard aging parameters (5s threshold, +10 boost).
    pub fn new() -> Self {
        Self {
            requests: Vec::new(),
            next_id: 1,
            aging_threshold_ms: 5000,
            aging_boost: 10,
        }
    }

    /// Creates a priority queue with custom aging parameters.
    pub fn with_aging(threshold_ms: u64, aging_boost: u32) -> Self {
        Self {
            requests: Vec::new(),
            next_id: 1,
            aging_threshold_ms: threshold_ms.max(100),
            aging_boost,
        }
    }

    /// Enqueues a new item with given priority and estimated tokens.
    pub fn enqueue(
        &mut self,
        payload: T,
        priority: RequestPriority,
        estimated_tokens: u64,
        caller_id: impl Into<String>,
    ) -> u64 {
        let id = self.next_id;
        self.next_id += 1;

        let request = QueuedRequest {
            id,
            priority,
            estimated_tokens,
            estimated_requests: 1,
            enqueued_at: Instant::now(),
            enqueued_utc: Utc::now(),
            caller_id: caller_id.into(),
            payload,
        };

        self.requests.push(request);
        id
    }

    /// Calculates effective rank for ordering, factoring in priority and aging.
    fn effective_rank(&self, req: &QueuedRequest<T>, now: Instant) -> (u32, std::cmp::Reverse<Instant>) {
        let base_rank = req.priority.rank();
        let elapsed_ms = now.duration_since(req.enqueued_at).as_millis() as u64;
        let aging_steps = (elapsed_ms / self.aging_threshold_ms) as u32;
        let effective_rank = base_rank + (aging_steps * self.aging_boost);
        (effective_rank, std::cmp::Reverse(req.enqueued_at))
    }

    /// Peeks the highest priority item ready for execution.
    pub fn peek(&self) -> Option<&QueuedRequest<T>> {
        let now = Instant::now();
        self.requests.iter().max_by_key(|req| self.effective_rank(req, now))
    }

    /// Dequeues and returns the highest priority request.
    pub fn dequeue_next(&mut self) -> Option<QueuedRequest<T>> {
        if self.requests.is_empty() {
            return None;
        }

        let now = Instant::now();
        let best_idx = self
            .requests
            .iter()
            .enumerate()
            .max_by_key(|(_, req)| self.effective_rank(req, now))
            .map(|(idx, _)| idx)?;

        Some(self.requests.remove(best_idx))
    }

    /// Dequeues the highest priority request that fits within currently available rate limits.
    pub fn dequeue_matching_capacity(
        &mut self,
        available_requests: f64,
        available_tokens: f64,
        has_concurrency: bool,
    ) -> Option<QueuedRequest<T>> {
        if !has_concurrency || self.requests.is_empty() {
            return None;
        }

        let now = Instant::now();
        let mut candidates: Vec<(usize, (u32, std::cmp::Reverse<Instant>))> = self
            .requests
            .iter()
            .enumerate()
            .filter(|(_, req)| {
                req.estimated_requests as f64 <= available_requests
                    && req.estimated_tokens as f64 <= available_tokens
            })
            .map(|(idx, req)| (idx, self.effective_rank(req, now)))
            .collect();

        if candidates.is_empty() {
            return None;
        }

        candidates.sort_by(|a, b| b.1.cmp(&a.1));
        let best_idx = candidates[0].0;
        Some(self.requests.remove(best_idx))
    }

    /// Cancels and returns a queued request by ID.
    pub fn cancel(&mut self, id: u64) -> Option<QueuedRequest<T>> {
        let pos = self.requests.iter().position(|r| r.id == id)?;
        Some(self.requests.remove(pos))
    }

    /// Returns the number of queued requests.
    pub fn len(&self) -> usize {
        self.requests.len()
    }

    /// Returns `true` if the queue is empty.
    pub fn is_empty(&self) -> bool {
        self.requests.is_empty()
    }

    /// Returns count of requests per priority tier.
    pub fn count_by_priority(&self, priority: RequestPriority) -> usize {
        self.requests.iter().filter(|r| r.priority == priority).count()
    }

    /// Clears all queued requests.
    pub fn clear(&mut self) {
        self.requests.clear();
    }
}

// ===========================================================================
// 8. Turn Rate Limiter & Token Quota Manager
// ===========================================================================

/// Manages turn frequency, burst limits, and cooldown intervals.
#[derive(Debug, Clone)]
pub struct TurnRateLimiter {
    config: TurnRateLimitConfig,
    turn_log: SlidingWindowLog<u32>,
    last_turn_instant: Option<Instant>,
    session_turns_count: u32,
    burst_tokens: TokenBucket,
}

impl TurnRateLimiter {
    /// Creates a new TurnRateLimiter with the specified configuration.
    pub fn new(config: TurnRateLimitConfig) -> Self {
        let burst_capacity = config.burst_capacity.max(1) as f64;
        let refill_rate = match config.max_turns_per_minute {
            Some(tpm) => (tpm as f64) / 60.0,
            None => 100.0,
        };

        Self {
            burst_tokens: TokenBucket::new(burst_capacity, refill_rate),
            config,
            turn_log: SlidingWindowLog::new(),
            last_turn_instant: None,
            session_turns_count: 0,
        }
    }

    /// Checks whether a new turn is currently permitted without recording it.
    pub fn check_turn(&mut self) -> ThrottleDecision {
        let now = Instant::now();

        // 1. Session turn limit check
        if let Some(max_session) = self.config.max_turns_per_session {
            if self.session_turns_count >= max_session {
                return ThrottleDecision::HardExhausted {
                    quota_type: QuotaType::SessionTurns,
                    limit: max_session as f64,
                    used: self.session_turns_count as f64,
                    reset_in_ms: None,
                };
            }
        }

        // 2. Minimum cooldown interval check
        if let Some(last) = self.last_turn_instant {
            let min_interval = Duration::from_millis(self.config.min_interval_ms);
            let elapsed = now.duration_since(last);
            if elapsed < min_interval {
                let wait = min_interval - elapsed;
                return ThrottleDecision::Throttled {
                    wait_duration_ms: wait.as_millis() as u64,
                    quota_type: QuotaType::MinimumTurnInterval,
                    reason: format!(
                        "Cooldown violation: need {}ms between turns ({}ms elapsed)",
                        self.config.min_interval_ms,
                        elapsed.as_millis()
                    ),
                };
            }
        }

        // 3. Turns per minute (sliding window)
        if let Some(max_tpm) = self.config.max_turns_per_minute {
            let turns_last_min = self.turn_log.count_in_window(Duration::from_secs(60)) as u32;
            if turns_last_min >= max_tpm {
                let wait = self
                    .turn_log
                    .time_until_oldest_expires(Duration::from_secs(60))
                    .unwrap_or(Duration::from_secs(1));
                return ThrottleDecision::Throttled {
                    wait_duration_ms: wait.as_millis().max(1) as u64,
                    quota_type: QuotaType::TurnsPerMinute,
                    reason: format!(
                        "Exceeded turn limit of {} turns/min (currently {} turns in window)",
                        max_tpm, turns_last_min
                    ),
                };
            }
        }

        // 4. Turns per hour (sliding window)
        if let Some(max_tph) = self.config.max_turns_per_hour {
            let turns_last_hr = self.turn_log.count_in_window(Duration::from_secs(3600)) as u32;
            if turns_last_hr >= max_tph {
                let wait = self
                    .turn_log
                    .time_until_oldest_expires(Duration::from_secs(3600))
                    .unwrap_or(Duration::from_secs(60));
                return ThrottleDecision::Throttled {
                    wait_duration_ms: wait.as_millis().max(1) as u64,
                    quota_type: QuotaType::TurnsPerHour,
                    reason: format!(
                        "Exceeded turn limit of {} turns/hour (currently {} turns in window)",
                        max_tph, turns_last_hr
                    ),
                };
            }
        }

        // 5. Burst capacity check
        if !self.burst_tokens.try_consume(0.0) && self.burst_tokens.available_tokens() < 1.0 {
            let wait = self.burst_tokens.time_until_available(1.0);
            if wait > Duration::from_millis(50) {
                return ThrottleDecision::Throttled {
                    wait_duration_ms: wait.as_millis() as u64,
                    quota_type: QuotaType::TurnsPerMinute,
                    reason: "Turn burst capacity temporarily exhausted".to_string(),
                };
            }
        }

        ThrottleDecision::Allowed {
            remaining_tokens: None,
            pacing_delay_ms: None,
            warning: None,
        }
    }

    /// Records a turn execution. Returns the decision result or an error if rejected.
    pub fn record_turn(&mut self) -> Result<ThrottleDecision, ThrottleError> {
        let decision = self.check_turn();
        match &decision {
            ThrottleDecision::Allowed { .. } => {
                let now = Instant::now();
                self.turn_log.record(1);
                self.last_turn_instant = Some(now);
                self.session_turns_count += 1;
                let _ = self.burst_tokens.try_consume(1.0);
                Ok(decision)
            }
            ThrottleDecision::Throttled {
                wait_duration_ms,
                quota_type,
                ..
            } => match quota_type {
                QuotaType::MinimumTurnInterval => {
                    let elapsed = self
                        .last_turn_instant
                        .map(|t| Instant::now().duration_since(t).as_millis() as u64)
                        .unwrap_or(0);
                    Err(ThrottleError::MinimumIntervalViolation {
                        required_interval_ms: self.config.min_interval_ms,
                        elapsed_ms: elapsed,
                        wait_ms: *wait_duration_ms,
                    })
                }
                QuotaType::TurnsPerMinute => Err(ThrottleError::TurnRateLimitExceeded {
                    limit: self.config.max_turns_per_minute.unwrap_or(0),
                    window_secs: 60,
                    retry_after_ms: *wait_duration_ms,
                }),
                QuotaType::TurnsPerHour => Err(ThrottleError::TurnRateLimitExceeded {
                    limit: self.config.max_turns_per_hour.unwrap_or(0),
                    window_secs: 3600,
                    retry_after_ms: *wait_duration_ms,
                }),
                _ => Err(ThrottleError::TurnRateLimitExceeded {
                    limit: 0,
                    window_secs: 60,
                    retry_after_ms: *wait_duration_ms,
                }),
            },
            ThrottleDecision::HardExhausted {
                quota_type,
                limit,
                used,
                ..
            } => Err(ThrottleError::SessionQuotaExhausted {
                quota_type: *quota_type,
                limit: *limit as u64,
                used: *used as u64,
            }),
        }
    }

    /// Returns the number of turns executed in the last 60 seconds.
    pub fn turns_last_minute(&mut self) -> u32 {
        self.turn_log.count_in_window(Duration::from_secs(60)) as u32
    }

    /// Returns the number of turns executed in the last 3600 seconds.
    pub fn turns_last_hour(&mut self) -> u32 {
        self.turn_log.count_in_window(Duration::from_secs(3600)) as u32
    }

    /// Total turns executed in this session.
    pub fn session_turns(&self) -> u32 {
        self.session_turns_count
    }

    /// Resets turn tracking counters.
    pub fn reset(&mut self) {
        self.turn_log.clear();
        self.last_turn_instant = None;
        self.session_turns_count = 0;
        self.burst_tokens.reset();
    }
}

/// Handle for a speculative token reservation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReservationTicket {
    /// Unique ticket identifier.
    pub id: u64,
    /// Speculatively reserved tokens.
    pub estimated_tokens: u64,
    /// Instant when reservation was created.
    pub created_at: Instant,
}

/// Manages token rate limits (TPM/TPH/TPD), budgets, and financial cost caps.
#[derive(Debug, Clone)]
pub struct TokenQuotaManager {
    config: TokenQuotaConfig,
    token_log_min: SlidingWindowLog<u64>,
    token_log_hr: SlidingWindowLog<u64>,
    token_log_day: SlidingWindowLog<u64>,
    input_log_min: SlidingWindowLog<u64>,
    output_log_min: SlidingWindowLog<u64>,
    cost_log_hr: SlidingWindowLog<f64>,
    cost_log_day: SlidingWindowLog<f64>,

    // Session aggregates
    session_prompt_tokens: u64,
    session_completion_tokens: u64,
    session_total_tokens: u64,
    session_cost_usd: f64,

    // Token bucket for continuous TPM rate limiting
    tpm_bucket: Option<TokenBucket>,

    // Speculative reservations
    next_ticket_id: u64,
    active_reservations: Vec<ReservationTicket>,
}

impl TokenQuotaManager {
    /// Creates a new TokenQuotaManager.
    pub fn new(config: TokenQuotaConfig) -> Self {
        let tpm_bucket = config.max_tpm.map(|tpm| {
            let capacity = tpm as f64;
            let refill_per_sec = (tpm as f64) / 60.0;
            TokenBucket::new(capacity, refill_per_sec)
        });

        Self {
            config,
            token_log_min: SlidingWindowLog::new(),
            token_log_hr: SlidingWindowLog::new(),
            token_log_day: SlidingWindowLog::new(),
            input_log_min: SlidingWindowLog::new(),
            output_log_min: SlidingWindowLog::new(),
            cost_log_hr: SlidingWindowLog::new(),
            cost_log_day: SlidingWindowLog::new(),

            session_prompt_tokens: 0,
            session_completion_tokens: 0,
            session_total_tokens: 0,
            session_cost_usd: 0.0,

            tpm_bucket,
            next_ticket_id: 1,
            active_reservations: Vec::new(),
        }
    }

    /// Evaluates current token and cost budgets against an estimated token request.
    pub fn check_budget(&mut self, estimated_tokens: u64) -> ThrottleDecision {
        let pending_reserved: u64 = self.active_reservations.iter().map(|r| r.estimated_tokens).sum();
        let effective_tokens = estimated_tokens + pending_reserved;

        // 1. Session Token Budget Hard Limit
        if let Some(budget) = self.config.session_token_budget {
            let projected_total = self.session_total_tokens + effective_tokens;
            if projected_total > budget {
                return ThrottleDecision::HardExhausted {
                    quota_type: QuotaType::SessionTokens,
                    limit: budget as f64,
                    used: self.session_total_tokens as f64,
                    reset_in_ms: None,
                };
            }
        }

        // 2. Session Financial Cost Limit
        if let Some(max_cost) = self.config.cost_limit_usd {
            if self.session_cost_usd >= max_cost {
                return ThrottleDecision::HardExhausted {
                    quota_type: QuotaType::SessionCostUsd,
                    limit: max_cost,
                    used: self.session_cost_usd,
                    reset_in_ms: None,
                };
            }
        }

        // 3. Hourly Cost Limit
        if let Some(max_cost_hr) = self.config.cost_per_hour_usd {
            let cost_hr = self.cost_log_hr.sum_in_window(Duration::from_secs(3600));
            if cost_hr >= max_cost_hr {
                let wait = self
                    .cost_log_hr
                    .time_until_oldest_expires(Duration::from_secs(3600))
                    .unwrap_or(Duration::from_secs(60));
                return ThrottleDecision::Throttled {
                    wait_duration_ms: wait.as_millis().max(1) as u64,
                    quota_type: QuotaType::HourlyCostUsd,
                    reason: format!(
                        "Exceeded hourly cost cap of ${:.2} (spent ${:.4} in last hour)",
                        max_cost_hr, cost_hr
                    ),
                };
            }
        }

        // 4. Tokens Per Minute (TPM) limit check
        if let Some(max_tpm) = self.config.max_tpm {
            let used_min = self.token_log_min.sum_in_window(Duration::from_secs(60));
            if used_min + effective_tokens > max_tpm {
                let wait = self
                    .token_log_min
                    .time_until_oldest_expires(Duration::from_secs(60))
                    .unwrap_or(Duration::from_secs(1));
                return ThrottleDecision::Throttled {
                    wait_duration_ms: wait.as_millis().max(1) as u64,
                    quota_type: QuotaType::TokensPerMinute,
                    reason: format!(
                        "TPM limit exceeded: {}/{} tokens in rolling minute (requesting {})",
                        used_min, max_tpm, effective_tokens
                    ),
                };
            }
        }

        // 5. Tokens Per Hour (TPH) limit check
        if let Some(max_tph) = self.config.max_tph {
            let used_hr = self.token_log_hr.sum_in_window(Duration::from_secs(3600));
            if used_hr + effective_tokens > max_tph {
                let wait = self
                    .token_log_hr
                    .time_until_oldest_expires(Duration::from_secs(3600))
                    .unwrap_or(Duration::from_secs(60));
                return ThrottleDecision::Throttled {
                    wait_duration_ms: wait.as_millis().max(1) as u64,
                    quota_type: QuotaType::TokensPerHour,
                    reason: format!(
                        "TPH limit exceeded: {}/{} tokens in rolling hour",
                        used_hr, max_tph
                    ),
                };
            }
        }

        // 6. Tokens Per Day (TPD) limit check
        if let Some(max_tpd) = self.config.max_tpd {
            let used_day = self.token_log_day.sum_in_window(Duration::from_secs(86400));
            if used_day + effective_tokens > max_tpd {
                let wait = self
                    .token_log_day
                    .time_until_oldest_expires(Duration::from_secs(86400))
                    .unwrap_or(Duration::from_secs(300));
                return ThrottleDecision::Throttled {
                    wait_duration_ms: wait.as_millis().max(1) as u64,
                    quota_type: QuotaType::TokensPerDay,
                    reason: format!(
                        "TPD limit exceeded: {}/{} tokens in rolling day",
                        used_day, max_tpd
                    ),
                };
            }
        }

        // 7. Calculate remaining tokens & warning threshold
        let (remaining, warning) = if let Some(budget) = self.config.session_token_budget {
            let used = self.session_total_tokens + effective_tokens;
            let rem = budget.saturating_sub(used);
            let ratio = (used as f32) / (budget as f32);

            let warn = if ratio >= self.config.danger_quota_ratio {
                Some(format!(
                    "Danger: Token budget is at {:.1}% ({}/{} tokens used)",
                    ratio * 100.0,
                    used,
                    budget
                ))
            } else if ratio >= self.config.soft_quota_ratio {
                Some(format!(
                    "Warning: Token budget reached {:.1}% ({}/{} tokens used)",
                    ratio * 100.0,
                    used,
                    budget
                ))
            } else {
                None
            };
            (Some(rem), warn)
        } else {
            (None, None)
        };

        ThrottleDecision::Allowed {
            remaining_tokens: remaining,
            pacing_delay_ms: None,
            warning,
        }
    }

    /// Reserves a speculative token allowance before initiating an LLM call.
    pub fn reserve(&mut self, estimated_tokens: u64) -> Result<ReservationTicket, ThrottleError> {
        let decision = self.check_budget(estimated_tokens);
        match decision {
            ThrottleDecision::Allowed { .. } => {
                let ticket = ReservationTicket {
                    id: self.next_ticket_id,
                    estimated_tokens,
                    created_at: Instant::now(),
                };
                self.next_ticket_id += 1;
                self.active_reservations.push(ticket);
                Ok(ticket)
            }
            ThrottleDecision::Throttled {
                wait_duration_ms,
                quota_type,
                ..
            } => Err(ThrottleError::TokenRateLimitExceeded {
                quota_type,
                limit: self.config.max_tpm.unwrap_or(0),
                requested: estimated_tokens,
                retry_after_ms: wait_duration_ms,
            }),
            ThrottleDecision::HardExhausted {
                quota_type,
                limit,
                used,
                ..
            } => Err(ThrottleError::SessionQuotaExhausted {
                quota_type,
                limit: limit as u64,
                used: used as u64,
            }),
        }
    }

    /// Commits actual token consumption and reconciles the reservation ticket.
    pub fn commit_reservation(
        &mut self,
        ticket: ReservationTicket,
        prompt_tokens: u64,
        completion_tokens: u64,
        cost_usd: Option<f64>,
    ) -> Result<(), ThrottleError> {
        if let Some(pos) = self.active_reservations.iter().position(|r| r.id == ticket.id) {
            self.active_reservations.remove(pos);
            self.record_usage(prompt_tokens, completion_tokens, cost_usd);
            Ok(())
        } else {
            Err(ThrottleError::ReservationNotFound { ticket_id: ticket.id })
        }
    }

    /// Cancels an active reservation ticket without recording usage.
    pub fn cancel_reservation(&mut self, ticket: ReservationTicket) {
        self.active_reservations.retain(|r| r.id != ticket.id);
    }

    /// Records actual token usage and estimated cost into sliding windows and session totals.
    pub fn record_usage(
        &mut self,
        prompt_tokens: u64,
        completion_tokens: u64,
        cost_usd: Option<f64>,
    ) {
        let total = prompt_tokens + completion_tokens;

        // Sliding window logs
        self.token_log_min.record(total);
        self.token_log_hr.record(total);
        self.token_log_day.record(total);
        self.input_log_min.record(prompt_tokens);
        self.output_log_min.record(completion_tokens);

        // Update continuous bucket
        if let Some(bucket) = &mut self.tpm_bucket {
            let _ = bucket.try_consume(total as f64);
        }

        // Session aggregates
        self.session_prompt_tokens += prompt_tokens;
        self.session_completion_tokens += completion_tokens;
        self.session_total_tokens += total;

        if let Some(cost) = cost_usd {
            self.session_cost_usd += cost;
            self.cost_log_hr.record(cost);
            self.cost_log_day.record(cost);
        }
    }

    /// Returns current quota utilization level for the session budget.
    pub fn quota_level(&self) -> QuotaLevel {
        if let Some(budget) = self.config.session_token_budget {
            if budget == 0 {
                return QuotaLevel::Exhausted;
            }
            let ratio = (self.session_total_tokens as f32) / (budget as f32);
            if ratio >= 1.0 {
                QuotaLevel::Exhausted
            } else if ratio >= self.config.danger_quota_ratio {
                QuotaLevel::Danger
            } else if ratio >= self.config.soft_quota_ratio {
                QuotaLevel::Warning
            } else {
                QuotaLevel::Normal
            }
        } else if let Some(max_cost) = self.config.cost_limit_usd {
            if max_cost <= 0.0 {
                return QuotaLevel::Exhausted;
            }
            let ratio = (self.session_cost_usd as f32) / (max_cost as f32);
            if ratio >= 1.0 {
                QuotaLevel::Exhausted
            } else if ratio >= self.config.danger_quota_ratio {
                QuotaLevel::Danger
            } else if ratio >= self.config.soft_quota_ratio {
                QuotaLevel::Warning
            } else {
                QuotaLevel::Normal
            }
        } else {
            QuotaLevel::Normal
        }
    }

    /// Total tokens consumed in the current session.
    pub fn session_total_tokens(&self) -> u64 {
        self.session_total_tokens
    }

    /// Total prompt/input tokens in the current session.
    pub fn session_prompt_tokens(&self) -> u64 {
        self.session_prompt_tokens
    }

    /// Total completion/output tokens in the current session.
    pub fn session_completion_tokens(&self) -> u64 {
        self.session_completion_tokens
    }

    /// Total financial cost accumulated in the current session (USD).
    pub fn session_cost_usd(&self) -> f64 {
        self.session_cost_usd
    }

    /// Tokens consumed in the rolling 60-second window.
    pub fn tokens_last_minute(&mut self) -> u64 {
        self.token_log_min.sum_in_window(Duration::from_secs(60))
    }

    /// Tokens consumed in the rolling 3600-second window.
    pub fn tokens_last_hour(&mut self) -> u64 {
        self.token_log_hr.sum_in_window(Duration::from_secs(3600))
    }

    /// Resets all counters and logs.
    pub fn reset(&mut self) {
        self.token_log_min.clear();
        self.token_log_hr.clear();
        self.token_log_day.clear();
        self.input_log_min.clear();
        self.output_log_min.clear();
        self.cost_log_hr.clear();
        self.cost_log_day.clear();
        self.session_prompt_tokens = 0;
        self.session_completion_tokens = 0;
        self.session_total_tokens = 0;
        self.session_cost_usd = 0.0;
        self.active_reservations.clear();
        if let Some(bucket) = &mut self.tpm_bucket {
            bucket.reset();
        }
    }
}

// ===========================================================================
// 9. Unified Throttle Engine & LLM Throttle Controller
// ===========================================================================

/// Execution metrics and quota statistics gathered during throttling.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ThrottleMetrics {
    /// Total turns executed.
    pub total_turns: u32,
    /// Turns executed in the last 60 seconds.
    pub turns_last_minute: u32,
    /// Turns executed in the last hour.
    pub turns_last_hour: u32,
    /// Total tokens consumed in the session.
    pub total_tokens: u64,
    /// Prompt tokens in the session.
    pub prompt_tokens: u64,
    /// Completion tokens in the session.
    pub completion_tokens: u64,
    /// Total tokens in the last minute.
    pub tokens_last_minute: u64,
    /// Total tokens in the last hour.
    pub tokens_last_hour: u64,
    /// Total financial cost in USD.
    pub total_cost_usd: f64,
    /// Current quota alert level.
    pub quota_level: QuotaLevel,
    /// Number of times execution was throttled.
    pub throttle_events_count: u32,
    /// Cumulative milliseconds waited due to throttling.
    pub cumulative_wait_ms: u64,
}

/// Detailed visual and numerical quota status report.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ThrottleStatusReport {
    /// Unix timestamp (seconds since epoch) when report was generated.
    pub timestamp: u64,
    /// Summary metrics.
    pub metrics: ThrottleMetrics,
    /// Percentage of session turn budget consumed (0.0 - 100.0), if configured.
    pub turn_budget_pct: Option<f32>,
    /// Percentage of session token budget consumed (0.0 - 100.0), if configured.
    pub token_budget_pct: Option<f32>,
    /// Percentage of session financial budget consumed (0.0 - 100.0), if configured.
    pub cost_budget_pct: Option<f32>,
    /// Configured maximum turns for the session.
    pub max_turns_per_session: Option<u32>,
    /// Configured session token budget.
    pub session_token_budget: Option<u64>,
    /// Configured session financial limit (USD).
    pub session_cost_limit_usd: Option<f64>,
    /// Enforcing policy.
    pub policy: ThrottlePolicy,
}

impl ThrottleStatusReport {
    /// Renders an ASCII / Unicode progress bar gauge for visualization.
    pub fn render_gauge(pct: f32, width: usize) -> String {
        let pct = pct.clamp(0.0, 100.0);
        let filled_chars = ((pct / 100.0) * (width as f32)).round() as usize;
        let empty_chars = width.saturating_sub(filled_chars);

        let filled = "█".repeat(filled_chars);
        let empty = "░".repeat(empty_chars);

        format!("[{}{}] {:.1}%", filled, empty, pct)
    }

    /// Formats a concise one-line status summary.
    pub fn format_summary(&self) -> String {
        let mut parts = Vec::new();

        parts.push(format!("Level: {}", self.metrics.quota_level));
        parts.push(format!("Turns: {}", self.metrics.total_turns));
        parts.push(format!("Tokens: {}", format_number(self.metrics.total_tokens)));

        if self.metrics.total_cost_usd > 0.0 {
            parts.push(format!("Cost: ${:.4}", self.metrics.total_cost_usd));
        }

        if let Some(pct) = self.token_budget_pct {
            parts.push(format!("Budget: {:.1}%", pct));
        }

        parts.join(" | ")
    }

    /// Formats a detailed multi-line report suitable for diagnostic output or terminal display.
    pub fn format_detailed_report(&self) -> String {
        let mut out = String::new();
        out.push_str("=== Fusion Quota & Throttle Status ===\n");
        let timestamp_str = DateTime::<Utc>::from_timestamp(self.timestamp as i64, 0)
            .map(|dt| dt.to_rfc3339())
            .unwrap_or_else(|| self.timestamp.to_string());
        out.push_str(&format!("Timestamp: {}\n", timestamp_str));
        out.push_str(&format!("Policy:    {:?}\n", self.policy));
        out.push_str(&format!("Status:    {}\n\n", self.metrics.quota_level));

        // Turn Budget
        out.push_str("1. Turns:\n");
        out.push_str(&format!("   - Total:       {}\n", self.metrics.total_turns));
        out.push_str(&format!("   - Last Minute: {}\n", self.metrics.turns_last_minute));
        out.push_str(&format!("   - Last Hour:   {}\n", self.metrics.turns_last_hour));
        if let (Some(pct), Some(max)) = (self.turn_budget_pct, self.max_turns_per_session) {
            out.push_str(&format!(
                "   - Session Cap: {}/{} {}\n",
                self.metrics.total_turns,
                max,
                Self::render_gauge(pct, 12)
            ));
        }

        // Token Budget
        out.push_str("\n2. Tokens:\n");
        out.push_str(&format!("   - Total:       {}\n", format_number(self.metrics.total_tokens)));
        out.push_str(&format!("   - Prompt:      {}\n", format_number(self.metrics.prompt_tokens)));
        out.push_str(&format!("   - Completion:  {}\n", format_number(self.metrics.completion_tokens)));
        out.push_str(&format!("   - Last Minute: {}\n", format_number(self.metrics.tokens_last_minute)));
        out.push_str(&format!("   - Last Hour:   {}\n", format_number(self.metrics.tokens_last_hour)));
        if let (Some(pct), Some(budget)) = (self.token_budget_pct, self.session_token_budget) {
            out.push_str(&format!(
                "   - Budget:      {}/{} {}\n",
                format_number(self.metrics.total_tokens),
                format_number(budget),
                Self::render_gauge(pct, 12)
            ));
        }

        // Financial Cost
        if self.metrics.total_cost_usd > 0.0 || self.session_cost_limit_usd.is_some() {
            out.push_str("\n3. Financial (USD):\n");
            out.push_str(&format!("   - Total Cost:  ${:.4}\n", self.metrics.total_cost_usd));
            if let (Some(pct), Some(limit)) = (self.cost_budget_pct, self.session_cost_limit_usd) {
                out.push_str(&format!(
                    "   - Cost Limit:  ${:.4}/${:.2} {}\n",
                    self.metrics.total_cost_usd,
                    limit,
                    Self::render_gauge(pct, 12)
                ));
            }
        }

        // Throttle statistics
        if self.metrics.throttle_events_count > 0 {
            out.push_str("\n4. Throttle Events:\n");
            out.push_str(&format!("   - Throttled Count: {}\n", self.metrics.throttle_events_count));
            out.push_str(&format!("   - Total Wait Time: {}ms\n", self.metrics.cumulative_wait_ms));
        }

        out
    }
}

/// The unified throttle engine governing agent turns, tokens, and cost protection.
#[derive(Debug, Clone)]
pub struct ThrottleEngine {
    config: ThrottleConfig,
    turn_limiter: TurnRateLimiter,
    token_manager: TokenQuotaManager,
    throttle_events_count: u32,
    cumulative_wait_ms: u64,
}

impl Default for ThrottleEngine {
    fn default() -> Self {
        Self::new(ThrottleConfig::default())
    }
}

impl ThrottleEngine {
    /// Creates a new ThrottleEngine with the given configuration.
    pub fn new(config: ThrottleConfig) -> Self {
        let turn_limiter = TurnRateLimiter::new(config.turns.clone());
        let token_manager = TokenQuotaManager::new(config.tokens.clone());

        Self {
            config,
            turn_limiter,
            token_manager,
            throttle_events_count: 0,
            cumulative_wait_ms: 0,
        }
    }

    /// Creates an unlimited ThrottleEngine with no restrictions.
    pub fn unlimited() -> Self {
        Self::new(ThrottleConfig::unlimited())
    }

    /// Creates a strict budget engine with given session tokens and USD cap.
    pub fn strict_budget(session_tokens: u64, session_cost_usd: f64) -> Self {
        Self::new(ThrottleConfig::strict_budget(session_tokens, session_cost_usd))
    }

    /// Returns a reference to the configuration.
    pub fn config(&self) -> &ThrottleConfig {
        &self.config
    }

    /// Updates the configuration.
    pub fn set_config(&mut self, config: ThrottleConfig) {
        self.turn_limiter = TurnRateLimiter::new(config.turns.clone());
        self.token_manager = TokenQuotaManager::new(config.tokens.clone());
        self.config = config;
    }

    /// Pre-turn check evaluating turn rates, token budget, and calculating adaptive delay.
    pub fn pre_turn_check(&mut self, estimated_tokens: u64) -> ThrottleDecision {
        // 1. Check turn rate limits
        let turn_decision = self.turn_limiter.check_turn();
        if !turn_decision.is_allowed() {
            if let ThrottleDecision::Throttled { wait_duration_ms, .. } = &turn_decision {
                self.throttle_events_count += 1;
                self.cumulative_wait_ms += wait_duration_ms;
            }
            return turn_decision;
        }

        // 2. Check token budget
        let token_decision = self.token_manager.check_budget(estimated_tokens);
        if !token_decision.is_allowed() {
            if let ThrottleDecision::Throttled { wait_duration_ms, .. } = &token_decision {
                self.throttle_events_count += 1;
                self.cumulative_wait_ms += wait_duration_ms;
            }
            return token_decision;
        }

        // 3. Compute adaptive pacing delay if enabled
        let pacing_delay = if self.config.adaptive_pacing {
            let delay_ms = self.calculate_adaptive_pacing_ms();
            if delay_ms > 0 {
                Some(delay_ms)
            } else {
                None
            }
        } else {
            None
        };

        // Combine warnings
        let (remaining_tokens, warning) = match token_decision {
            ThrottleDecision::Allowed {
                remaining_tokens,
                warning,
                ..
            } => (remaining_tokens, warning),
            _ => (None, None),
        };

        ThrottleDecision::Allowed {
            remaining_tokens,
            pacing_delay_ms: pacing_delay,
            warning,
        }
    }

    /// Records the start of a turn after checks pass.
    pub fn record_turn_start(&mut self) -> Result<ThrottleDecision, ThrottleError> {
        self.turn_limiter.record_turn()
    }

    /// Records actual token consumption when a turn completes.
    pub fn record_turn_finish(
        &mut self,
        prompt_tokens: u64,
        completion_tokens: u64,
        cost_usd: Option<f64>,
    ) {
        self.token_manager.record_usage(prompt_tokens, completion_tokens, cost_usd);
    }

    /// Reserves speculative token allowance.
    pub fn reserve_tokens(&mut self, estimated_tokens: u64) -> Result<ReservationTicket, ThrottleError> {
        self.token_manager.reserve(estimated_tokens)
    }

    /// Commits a speculative reservation ticket with actual tokens.
    pub fn commit_tokens(
        &mut self,
        ticket: ReservationTicket,
        prompt_tokens: u64,
        completion_tokens: u64,
        cost_usd: Option<f64>,
    ) -> Result<(), ThrottleError> {
        self.token_manager
            .commit_reservation(ticket, prompt_tokens, completion_tokens, cost_usd)
    }

    /// Cancels a speculative reservation ticket.
    pub fn cancel_reservation(&mut self, ticket: ReservationTicket) {
        self.token_manager.cancel_reservation(ticket);
    }

    /// Calculates adaptive pacing delay in milliseconds based on quota proximity.
    pub fn calculate_adaptive_pacing_ms(&self) -> u64 {
        if !self.config.adaptive_pacing || self.config.max_pacing_delay_ms == 0 {
            return 0;
        }

        let token_ratio = if let Some(budget) = self.config.tokens.session_token_budget {
            if budget > 0 {
                (self.token_manager.session_total_tokens() as f32) / (budget as f32)
            } else {
                0.0
            }
        } else {
            0.0
        };

        let turn_ratio = if let Some(max_turns) = self.config.turns.max_turns_per_session {
            if max_turns > 0 {
                (self.turn_limiter.session_turns() as f32) / (max_turns as f32)
            } else {
                0.0
            }
        } else {
            0.0
        };

        let max_ratio = token_ratio.max(turn_ratio);
        let soft_threshold = self.config.tokens.soft_quota_ratio;

        if max_ratio < soft_threshold {
            0
        } else {
            let progress_in_warning = (max_ratio - soft_threshold) / (1.0 - soft_threshold);
            let progress_clamped = progress_in_warning.clamp(0.0, 1.0);
            let delay = (progress_clamped * (self.config.max_pacing_delay_ms as f32)) as u64;
            delay
        }
    }

    /// Collects current throttle metrics.
    pub fn metrics(&mut self) -> ThrottleMetrics {
        let total_turns = self.turn_limiter.session_turns();
        let turns_last_minute = self.turn_limiter.turns_last_minute();
        let turns_last_hour = self.turn_limiter.turns_last_hour();

        let total_tokens = self.token_manager.session_total_tokens();
        let prompt_tokens = self.token_manager.session_prompt_tokens();
        let completion_tokens = self.token_manager.session_completion_tokens();
        let tokens_last_minute = self.token_manager.tokens_last_minute();
        let tokens_last_hour = self.token_manager.tokens_last_hour();
        let total_cost_usd = self.token_manager.session_cost_usd();
        let quota_level = self.token_manager.quota_level();

        ThrottleMetrics {
            total_turns,
            turns_last_minute,
            turns_last_hour,
            total_tokens,
            prompt_tokens,
            completion_tokens,
            tokens_last_minute,
            tokens_last_hour,
            total_cost_usd,
            quota_level,
            throttle_events_count: self.throttle_events_count,
            cumulative_wait_ms: self.cumulative_wait_ms,
        }
    }

    /// Generates a comprehensive status report.
    pub fn status_report(&mut self) -> ThrottleStatusReport {
        let metrics = self.metrics();

        let turn_budget_pct = self
            .config
            .turns
            .max_turns_per_session
            .map(|m| if m > 0 { (metrics.total_turns as f32 / m as f32) * 100.0 } else { 0.0 });

        let token_budget_pct = self
            .config
            .tokens
            .session_token_budget
            .map(|b| if b > 0 { (metrics.total_tokens as f32 / b as f32) * 100.0 } else { 0.0 });

        let cost_budget_pct = self
            .config
            .tokens
            .cost_limit_usd
            .map(|c| if c > 0.0 { (metrics.total_cost_usd as f32 / c as f32) * 100.0 } else { 0.0 });

        ThrottleStatusReport {
            timestamp: Utc::now().timestamp() as u64,
            metrics,
            turn_budget_pct,
            token_budget_pct,
            cost_budget_pct,
            max_turns_per_session: self.config.turns.max_turns_per_session,
            session_token_budget: self.config.tokens.session_token_budget,
            session_cost_limit_usd: self.config.tokens.cost_limit_usd,
            policy: self.config.policy,
        }
    }

    /// Resets all internal trackers for a new session.
    pub fn reset(&mut self) {
        self.turn_limiter.reset();
        self.token_manager.reset();
        self.throttle_events_count = 0;
        self.cumulative_wait_ms = 0;
    }
}

/// Statistics and observability metrics for the LLM Provider Throttle Controller.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct LlmThrottleMetrics {
    /// Total requests enqueued.
    pub total_enqueued: u64,
    /// Total requests successfully dispatched.
    pub total_dispatched: u64,
    /// Total requests completed.
    pub total_completed: u64,
    /// Total requests that failed or were cancelled.
    pub total_failed: u64,
    /// Total HTTP 429 errors handled.
    pub total_429_errors: u64,
    /// Total tokens consumed across all calls.
    pub total_tokens_used: u64,
    /// Current number of active in-flight requests.
    pub active_in_flight: usize,
    /// Total requests currently waiting in the priority queue.
    pub queued_count: usize,
    /// Number of UserInteractive requests queued.
    pub user_interactive_queued: usize,
    /// Number of SubagentWorker requests queued.
    pub subagent_queued: usize,
    /// Number of BackgroundBatch requests queued.
    pub background_queued: usize,
}

/// Comprehensive provider LLM throttle controller managing RPM/TPM rate limits,
/// concurrency bounds, HTTP 429 backoff retries with jitter, and priority scheduling.
#[derive(Debug, Clone)]
pub struct LlmThrottleController<T: Clone> {
    rate_limiter: RpmTpmRateLimiter,
    queue: PriorityThrottleQueue<T>,
    backoff_config: BackoffConfig,
    metrics: LlmThrottleMetrics,
}

impl<T: Clone> LlmThrottleController<T> {
    /// Creates a new LlmThrottleController with configured rate limits and backoff parameters.
    pub fn new(rpm_tpm_config: RpmTpmConfig, backoff_config: BackoffConfig) -> Self {
        Self {
            rate_limiter: RpmTpmRateLimiter::new(rpm_tpm_config),
            queue: PriorityThrottleQueue::new(),
            backoff_config,
            metrics: LlmThrottleMetrics::default(),
        }
    }

    /// Enqueues a prompt request with specified priority and estimated tokens. Returns assigned item ID.
    pub fn enqueue(
        &mut self,
        payload: T,
        priority: RequestPriority,
        estimated_tokens: u64,
        caller_id: impl Into<String>,
    ) -> u64 {
        self.metrics.total_enqueued += 1;
        self.queue.enqueue(payload, priority, estimated_tokens, caller_id)
    }

    /// Attempts to dequeue the next eligible request and acquire its concurrency permit and rate tokens.
    pub fn try_dispatch(&mut self) -> Option<(QueuedRequest<T>, ConcurrencyPermit)> {
        let has_concurrency = self.rate_limiter.concurrency_limiter().has_capacity();
        let avail_rpm = self.rate_limiter.available_rpm().unwrap_or(1000.0);
        let avail_tpm = self.rate_limiter.available_tpm().unwrap_or(1_000_000.0);

        let req = self.queue.dequeue_matching_capacity(avail_rpm, avail_tpm, has_concurrency)?;

        match self.rate_limiter.try_acquire(req.estimated_requests, req.estimated_tokens) {
            Ok(permit) => {
                self.metrics.total_dispatched += 1;
                Some((req, permit))
            }
            Err(_) => {
                // Put back in queue if acquisition failed due to race
                self.queue.enqueue(req.payload, req.priority, req.estimated_tokens, req.caller_id);
                None
            }
        }
    }

    /// Records completion of a dispatched request with actual token usage and frees the concurrency permit.
    pub fn on_success(&mut self, permit: ConcurrencyPermit, estimated_tokens: u64, actual_tokens: u64) {
        permit.release();
        self.metrics.total_completed += 1;
        self.metrics.total_tokens_used += actual_tokens;

        if actual_tokens < estimated_tokens {
            self.rate_limiter.refund(0, estimated_tokens - actual_tokens);
        } else if actual_tokens > estimated_tokens {
            self.rate_limiter.record_usage(0, actual_tokens - estimated_tokens);
        }
    }

    /// Handles an HTTP 429 Too Many Requests response, updating metrics and calculating backoff delay.
    pub fn on_429(&mut self, attempt: u32, retry_after_header: Option<&str>) -> Duration {
        self.metrics.total_429_errors += 1;
        Http429Handler::compute_backoff(attempt, retry_after_header, &self.backoff_config)
    }

    /// Records a failed or cancelled request and releases its permit.
    pub fn on_failure(&mut self, permit: ConcurrencyPermit, estimated_tokens: u64) {
        permit.release();
        self.metrics.total_failed += 1;
        self.rate_limiter.refund(1, estimated_tokens);
    }

    /// Returns current observability metrics.
    pub fn metrics(&self) -> LlmThrottleMetrics {
        let mut m = self.metrics.clone();
        m.active_in_flight = self.rate_limiter.concurrency_limiter().in_flight();
        m.queued_count = self.queue.len();
        m.user_interactive_queued = self.queue.count_by_priority(RequestPriority::UserInteractive);
        m.subagent_queued = self.queue.count_by_priority(RequestPriority::SubagentWorker);
        m.background_queued = self.queue.count_by_priority(RequestPriority::BackgroundBatch);
        m
    }

    /// Reference to the underlying rate limiter.
    pub fn rate_limiter(&mut self) -> &mut RpmTpmRateLimiter {
        &mut self.rate_limiter
    }

    /// Reference to the priority queue.
    pub fn queue(&mut self) -> &mut PriorityThrottleQueue<T> {
        &mut self.queue
    }

    /// Resets all internal trackers and queues.
    pub fn reset(&mut self) {
        self.rate_limiter.reset();
        self.queue.clear();
        self.metrics = LlmThrottleMetrics::default();
    }
}

// ===========================================================================
// 10. Thread-Safe Wrappers & Async Helpers
// ===========================================================================

/// Thread-safe shared ThrottleEngine handle.
pub type SharedThrottleEngine = Arc<Mutex<ThrottleEngine>>;

/// Creates a new shared ThrottleEngine handle.
pub fn new_shared_throttle_engine(config: ThrottleConfig) -> SharedThrottleEngine {
    Arc::new(Mutex::new(ThrottleEngine::new(config)))
}

/// Thread-safe shared LlmThrottleController handle.
pub type SharedLlmThrottleController<T> = Arc<Mutex<LlmThrottleController<T>>>;

/// Creates a new shared LlmThrottleController handle.
pub fn new_shared_llm_throttle_controller<T: Clone>(
    rpm_tpm_config: RpmTpmConfig,
    backoff_config: BackoffConfig,
) -> SharedLlmThrottleController<T> {
    Arc::new(Mutex::new(LlmThrottleController::new(rpm_tpm_config, backoff_config)))
}

/// Asynchronously enforces quota checks and sleeps if throttling or adaptive pacing is requested.
pub async fn enforce_throttle_async(
    engine: &SharedThrottleEngine,
    estimated_tokens: u64,
) -> Result<ThrottleDecision, ThrottleError> {
    let decision = {
        let mut guard = engine.lock().map_err(|_| ThrottleError::SessionQuotaExhausted {
            quota_type: QuotaType::SessionTurns,
            limit: 0,
            used: 0,
        })?;
        guard.pre_turn_check(estimated_tokens)
    };

    match &decision {
        ThrottleDecision::Allowed { pacing_delay_ms, .. } => {
            if let Some(delay_ms) = pacing_delay_ms {
                if *delay_ms > 0 {
                    tokio::time::sleep(Duration::from_millis(*delay_ms)).await;
                }
            }
            // Record turn start
            let mut guard = engine.lock().map_err(|_| ThrottleError::SessionQuotaExhausted {
                quota_type: QuotaType::SessionTurns,
                limit: 0,
                used: 0,
            })?;
            guard.record_turn_start()?;
            Ok(decision)
        }
        ThrottleDecision::Throttled {
            wait_duration_ms,
            quota_type,
            ..
        } => {
            let policy = {
                let guard = engine.lock().map_err(|_| ThrottleError::SessionQuotaExhausted {
                    quota_type: QuotaType::SessionTurns,
                    limit: 0,
                    used: 0,
                })?;
                guard.config().policy
            };

            match policy {
                ThrottlePolicy::WaitAndRetry => {
                    tokio::time::sleep(Duration::from_millis(*wait_duration_ms)).await;
                    let mut guard = engine.lock().map_err(|_| ThrottleError::SessionQuotaExhausted {
                        quota_type: QuotaType::SessionTurns,
                        limit: 0,
                        used: 0,
                    })?;
                    guard.record_turn_start()?;
                    Ok(decision)
                }
                ThrottlePolicy::WarnOnly => {
                    let mut guard = engine.lock().map_err(|_| ThrottleError::SessionQuotaExhausted {
                        quota_type: QuotaType::SessionTurns,
                        limit: 0,
                        used: 0,
                    })?;
                    guard.record_turn_start()?;
                    Ok(decision)
                }
                ThrottlePolicy::StrictReject | ThrottlePolicy::AdaptivePacing => {
                    match quota_type {
                        QuotaType::MinimumTurnInterval => Err(ThrottleError::MinimumIntervalViolation {
                            required_interval_ms: 0,
                            elapsed_ms: 0,
                            wait_ms: *wait_duration_ms,
                        }),
                        QuotaType::TurnsPerMinute | QuotaType::TurnsPerHour => {
                            Err(ThrottleError::TurnRateLimitExceeded {
                                limit: 0,
                                window_secs: 60,
                                retry_after_ms: *wait_duration_ms,
                            })
                        }
                        _ => Err(ThrottleError::TokenRateLimitExceeded {
                            quota_type: *quota_type,
                            limit: 0,
                            requested: estimated_tokens,
                            retry_after_ms: *wait_duration_ms,
                        }),
                    }
                }
            }
        }
        ThrottleDecision::HardExhausted {
            quota_type,
            limit,
            used,
            ..
        } => {
            let policy = {
                let guard = engine.lock().map_err(|_| ThrottleError::SessionQuotaExhausted {
                    quota_type: QuotaType::SessionTurns,
                    limit: 0,
                    used: 0,
                })?;
                guard.config().policy
            };

            if policy == ThrottlePolicy::WarnOnly {
                let mut guard = engine.lock().map_err(|_| ThrottleError::SessionQuotaExhausted {
                    quota_type: QuotaType::SessionTurns,
                    limit: 0,
                    used: 0,
                })?;
                let _ = guard.record_turn_start();
                Ok(decision)
            } else {
                Err(ThrottleError::SessionQuotaExhausted {
                    quota_type: *quota_type,
                    limit: *limit as u64,
                    used: *used as u64,
                })
            }
        }
    }
}

// ===========================================================================
// 11. Formatting Helpers
// ===========================================================================

fn format_number(n: u64) -> String {
    if n >= 1_000_000 {
        format!("{:.1}M", n as f64 / 1_000_000.0)
    } else if n >= 1_000 {
        format!("{:.1}k", n as f64 / 1_000.0)
    } else {
        n.to_string()
    }
}

// ===========================================================================
// 12. Unit Tests
// ===========================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_turn_rate_limiter_basic() {
        let config = TurnRateLimitConfig {
            max_turns_per_minute: Some(3),
            max_turns_per_hour: Some(10),
            burst_capacity: 3,
            min_interval_ms: 0,
            max_turns_per_session: Some(5),
        };

        let mut limiter = TurnRateLimiter::new(config);

        // Turn 1, 2, 3 allowed
        assert!(limiter.record_turn().is_ok());
        assert!(limiter.record_turn().is_ok());
        assert!(limiter.record_turn().is_ok());

        assert_eq!(limiter.turns_last_minute(), 3);
        assert_eq!(limiter.session_turns(), 3);

        // Turn 4 exceeds max_turns_per_minute (3)
        let res = limiter.record_turn();
        assert!(res.is_err());
        assert!(matches!(res.unwrap_err(), ThrottleError::TurnRateLimitExceeded { .. }));
    }

    #[test]
    fn test_turn_cooldown_interval() {
        let config = TurnRateLimitConfig {
            max_turns_per_minute: Some(60),
            max_turns_per_hour: None,
            burst_capacity: 10,
            min_interval_ms: 50,
            max_turns_per_session: None,
        };

        let mut limiter = TurnRateLimiter::new(config);
        assert!(limiter.record_turn().is_ok());

        // Immediate subsequent turn should violate minimum interval
        let res = limiter.record_turn();
        assert!(res.is_err());
        assert!(matches!(res.unwrap_err(), ThrottleError::MinimumIntervalViolation { .. }));

        // Wait out the cooldown
        std::thread::sleep(Duration::from_millis(60));
        assert!(limiter.record_turn().is_ok());
    }

    #[test]
    fn test_session_turns_hard_cap() {
        let config = TurnRateLimitConfig {
            max_turns_per_minute: None,
            max_turns_per_hour: None,
            burst_capacity: 10,
            min_interval_ms: 0,
            max_turns_per_session: Some(2),
        };

        let mut limiter = TurnRateLimiter::new(config);
        assert!(limiter.record_turn().is_ok());
        assert!(limiter.record_turn().is_ok());

        let res = limiter.record_turn();
        assert!(res.is_err());
        assert!(matches!(res.unwrap_err(), ThrottleError::SessionQuotaExhausted { .. }));
    }

    #[test]
    fn test_token_bucket_refill() {
        let mut bucket = TokenBucket::new(100.0, 50.0); // 50 tokens/sec refill

        assert!(bucket.try_consume(80.0));
        assert!(!bucket.try_consume(30.0)); // only 20 left

        // Wait 0.5s -> should refill 25 tokens -> 45 total
        std::thread::sleep(Duration::from_millis(500));
        assert!(bucket.try_consume(40.0));
    }

    #[test]
    fn test_token_bucket_rpm_and_tpm() {
        // Test RPM bucket: 60 RPM = 1 req/sec, burst 2
        let mut rpm_bucket = TokenBucket::with_rpm(60, Some(2));
        assert_eq!(rpm_bucket.capacity(), 2.0);
        assert_eq!(rpm_bucket.refill_rate_per_sec(), 1.0);
        assert!(rpm_bucket.try_consume(1.0));
        assert!(rpm_bucket.try_consume(1.0));
        assert!(!rpm_bucket.try_consume(1.0));

        // Test refund
        rpm_bucket.refund(1.0);
        assert!(rpm_bucket.try_consume(1.0));

        // Test TPM bucket: 60,000 TPM = 1,000 tokens/sec
        let mut tpm_bucket = TokenBucket::with_tpm(60_000, Some(5_000));
        assert_eq!(tpm_bucket.capacity(), 5_000.0);
        assert_eq!(tpm_bucket.refill_rate_per_sec(), 1_000.0);
        assert!(tpm_bucket.try_consume(3_000.0));
        assert!(tpm_bucket.try_consume(2_000.0));
        assert!(!tpm_bucket.try_consume(500.0));
    }

    #[test]
    fn test_rpm_tpm_rate_limiter_dual_enforcement() {
        let config = RpmTpmConfig {
            max_rpm: Some(60),       // 1 req/s, burst 2
            max_tpm: Some(60_000),   // 1000 tok/s, burst 2000
            rpm_burst: Some(2),
            tpm_burst: Some(2000),
            max_concurrency: Some(3),
        };

        let mut limiter = RpmTpmRateLimiter::new(config);

        // 1. Acquire permit 1 (1000 tokens) -> OK
        let permit1 = limiter.try_acquire(1, 1000);
        assert!(permit1.is_ok());

        // 2. Acquire permit 2 (800 tokens) -> OK (total 1800 <= 2000)
        let permit2 = limiter.try_acquire(1, 800);
        assert!(permit2.is_ok());

        // 3. Third request should fail due to RPM burst exhaustion (2/2 requests consumed)
        let permit3 = limiter.try_acquire(1, 100);
        assert!(permit3.is_err());
        assert!(matches!(
            permit3.unwrap_err(),
            ThrottleDecision::Throttled { quota_type: QuotaType::RequestsPerMinute, .. }
        ));

        // Release permits
        drop(permit1);
        drop(permit2);
    }

    #[test]
    fn test_concurrency_limiter_permit_lifecycle() {
        let limiter = ConcurrencyLimiter::new(2);

        assert_eq!(limiter.in_flight(), 0);
        assert!(limiter.has_capacity());

        let p1 = limiter.try_acquire().expect("Permit 1 should acquire");
        assert_eq!(limiter.in_flight(), 1);
        assert_eq!(limiter.peak_in_flight(), 1);

        let p2 = limiter.try_acquire().expect("Permit 2 should acquire");
        assert_eq!(limiter.in_flight(), 2);
        assert_eq!(limiter.peak_in_flight(), 2);
        assert!(!limiter.has_capacity());

        // 3rd attempt should fail
        let p3_err = limiter.try_acquire();
        assert!(p3_err.is_err());
        assert_eq!(limiter.total_rejected(), 1);

        // Dropping p1 should free up a slot
        drop(p1);
        assert_eq!(limiter.in_flight(), 1);
        assert!(limiter.has_capacity());

        let p3 = limiter.try_acquire().expect("Permit 3 should acquire after drop");
        assert_eq!(limiter.in_flight(), 2);

        // Explicit release
        p2.release();
        assert_eq!(limiter.in_flight(), 1);

        drop(p3);
        assert_eq!(limiter.in_flight(), 0);
    }

    #[test]
    fn test_fast_rng_distribution() {
        let mut rng = FastRng::new(12345);
        for _ in 0..100 {
            let val = rng.gen_range(5.0, 15.0);
            assert!(val >= 5.0 && val < 15.0);
        }
    }

    #[test]
    fn test_exponential_backoff_with_jitter_bounds() {
        let config = BackoffConfig {
            initial_delay_ms: 100,
            max_delay_ms: 1000,
            multiplier: 2.0,
            jitter: JitterStrategy::Full,
            max_retries: 5,
            respect_retry_after: true,
        };

        // Full jitter for attempt 0: base is 100ms -> [0, 100ms]
        for seed in 1..=20 {
            let delay = ExponentialBackoff::calculate_delay_deterministic(0, &config, seed);
            assert!(delay <= Duration::from_millis(100));
        }

        // Attempt 10: base is 100 * 2^10 = 102,400ms -> capped at max_delay_ms (1000ms)
        for seed in 1..=20 {
            let delay = ExponentialBackoff::calculate_delay_deterministic(10, &config, seed);
            assert!(delay <= Duration::from_millis(1000));
        }

        // None jitter (exact exponential)
        let no_jitter_cfg = BackoffConfig {
            jitter: JitterStrategy::None,
            ..config.clone()
        };
        let d0 = ExponentialBackoff::calculate_delay_deterministic(0, &no_jitter_cfg, 1);
        let d1 = ExponentialBackoff::calculate_delay_deterministic(1, &no_jitter_cfg, 1);
        let d2 = ExponentialBackoff::calculate_delay_deterministic(2, &no_jitter_cfg, 1);
        assert_eq!(d0, Duration::from_millis(100));
        assert_eq!(d1, Duration::from_millis(200));
        assert_eq!(d2, Duration::from_millis(400));
    }

    #[test]
    fn test_http_429_retry_after_parsing() {
        // Integer seconds
        let d1 = Http429Handler::parse_retry_after("12").expect("12 seconds");
        assert_eq!(d1, Duration::from_secs(12));

        // Fractional seconds
        let d2 = Http429Handler::parse_retry_after("1.5").expect("1.5 seconds");
        assert_eq!(d2, Duration::from_millis(1500));

        // Milliseconds notation
        let d3 = Http429Handler::parse_retry_after("250ms").expect("250ms");
        assert_eq!(d3, Duration::from_millis(250));

        // Invalid / empty
        assert!(Http429Handler::parse_retry_after("").is_none());
        assert!(Http429Handler::parse_retry_after("invalid_header").is_none());
    }

    #[test]
    fn test_http_429_backoff_computation() {
        let config = BackoffConfig {
            initial_delay_ms: 200,
            max_delay_ms: 10_000,
            multiplier: 2.0,
            jitter: JitterStrategy::Full,
            max_retries: 5,
            respect_retry_after: true,
        };

        // When Retry-After header is provided:
        let delay_header = Http429Handler::compute_backoff(0, Some("3"), &config);
        assert!(delay_header >= Duration::from_millis(2900) && delay_header <= Duration::from_millis(3500));

        // When no Retry-After header is provided:
        let delay_calc = Http429Handler::compute_backoff(0, None, &config);
        assert!(delay_calc <= Duration::from_millis(200));

        // Retryable status codes
        assert!(Http429Handler::is_retryable_status(429));
        assert!(Http429Handler::is_retryable_status(503));
        assert!(Http429Handler::is_retryable_status(500));
        assert!(!Http429Handler::is_retryable_status(400));
        assert!(!Http429Handler::is_retryable_status(401));
    }

    #[test]
    fn test_priority_queue_ordering_user_vs_subagent() {
        let mut queue = PriorityThrottleQueue::<String>::new();

        // Enqueue items in reverse order of priority
        let id_bg = queue.enqueue("batch job".into(), RequestPriority::BackgroundBatch, 500, "indexer");
        let id_sub = queue.enqueue("subagent query".into(), RequestPriority::SubagentWorker, 1000, "worker-1");
        let id_tool = queue.enqueue("tool invocation".into(), RequestPriority::DirectTool, 800, "tool-exec");
        let id_user = queue.enqueue("user chat message".into(), RequestPriority::UserInteractive, 1200, "user");

        assert_eq!(queue.len(), 4);

        // Highest priority (UserInteractive) should be dequeued first
        let item1 = queue.dequeue_next().expect("Item 1");
        assert_eq!(item1.id, id_user);
        assert_eq!(item1.payload, "user chat message");

        // DirectTool second
        let item2 = queue.dequeue_next().expect("Item 2");
        assert_eq!(item2.id, id_tool);

        // SubagentWorker third
        let item3 = queue.dequeue_next().expect("Item 3");
        assert_eq!(item3.id, id_sub);

        // BackgroundBatch last
        let item4 = queue.dequeue_next().expect("Item 4");
        assert_eq!(item4.id, id_bg);

        assert!(queue.is_empty());
    }

    #[test]
    fn test_priority_queue_aging_starvation_prevention() {
        // Set aging threshold to 20ms with +35 boost
        let mut queue = PriorityThrottleQueue::<String>::with_aging(20, 35);

        // Enqueue a background batch job (base rank: 10)
        let id_bg = queue.enqueue("old batch job".into(), RequestPriority::BackgroundBatch, 500, "indexer");

        // Sleep 50ms so aging steps >= 2 -> effective rank = 10 + (2 * 35) = 80
        std::thread::sleep(Duration::from_millis(50));

        // Enqueue a new user interactive job (base rank: 40, aging: 0 -> effective rank: 40)
        let _id_user = queue.enqueue("new user prompt".into(), RequestPriority::UserInteractive, 500, "user");

        // The aged background job should now preempt the newer user prompt!
        let next = queue.dequeue_next().expect("Should dequeue next");
        assert_eq!(next.id, id_bg);
        assert_eq!(next.payload, "old batch job");
    }

    #[test]
    fn test_priority_queue_capacity_aware_dequeue() {
        let mut queue = PriorityThrottleQueue::<String>::new();

        // 1. High priority user prompt requesting 5000 tokens
        queue.enqueue("large user prompt".into(), RequestPriority::UserInteractive, 5000, "user");

        // 2. Lower priority subagent request requesting 500 tokens
        queue.enqueue("small subagent query".into(), RequestPriority::SubagentWorker, 500, "worker");

        // Available token budget is only 1000 tokens.
        // User prompt is too large, so subagent query should be dequeued!
        let dequeued = queue.dequeue_matching_capacity(10.0, 1000.0, true).expect("Should find matching item");
        assert_eq!(dequeued.payload, "small subagent query");
    }

    #[test]
    fn test_llm_throttle_controller_dispatch_and_429_recovery() {
        let rpm_tpm_config = RpmTpmConfig {
            max_rpm: Some(60),
            max_tpm: Some(100_000),
            rpm_burst: Some(5),
            tpm_burst: Some(10_000),
            max_concurrency: Some(2),
        };

        let backoff_config = BackoffConfig {
            initial_delay_ms: 100,
            max_delay_ms: 1000,
            multiplier: 2.0,
            jitter: JitterStrategy::None,
            max_retries: 3,
            respect_retry_after: true,
        };

        let mut controller = LlmThrottleController::<String>::new(rpm_tpm_config, backoff_config);

        controller.enqueue("user prompt 1".into(), RequestPriority::UserInteractive, 2000, "user");
        controller.enqueue("subagent task 1".into(), RequestPriority::SubagentWorker, 1500, "agent-1");

        // Dispatch 1
        let (req1, permit1) = controller.try_dispatch().expect("Dispatch 1 should succeed");
        assert_eq!(req1.payload, "user prompt 1");

        // Dispatch 2
        let (req2, permit2) = controller.try_dispatch().expect("Dispatch 2 should succeed");
        assert_eq!(req2.payload, "subagent task 1");

        // Active in-flight should now be 2 (max concurrency)
        assert_eq!(controller.metrics().active_in_flight, 2);

        // Simulate 429 backoff handling
        let delay = controller.on_429(0, Some("2"));
        assert!(delay >= Duration::from_millis(2000));
        assert_eq!(controller.metrics().total_429_errors, 1);

        // Success for req1
        controller.on_success(permit1, 2000, 1800); // 200 token refund
        assert_eq!(controller.metrics().active_in_flight, 1);
        assert_eq!(controller.metrics().total_completed, 1);

        // Failure for req2
        controller.on_failure(permit2, 1500);
        assert_eq!(controller.metrics().active_in_flight, 0);
        assert_eq!(controller.metrics().total_failed, 1);
    }

    #[test]
    fn test_token_quota_manager_tpm_and_budget() {
        let config = TokenQuotaConfig {
            max_tpm: Some(1000),
            max_tph: None,
            max_tpd: None,
            max_input_tpm: None,
            max_output_tpm: None,
            session_token_budget: Some(2000),
            soft_quota_ratio: 0.50,
            danger_quota_ratio: 0.90,
            cost_limit_usd: Some(1.0),
            cost_per_hour_usd: None,
            cost_per_day_usd: None,
        };

        let mut manager = TokenQuotaManager::new(config);

        // 1. Normal usage
        assert_eq!(manager.quota_level(), QuotaLevel::Normal);
        manager.record_usage(300, 200, Some(0.01)); // 500 total (25% of 2000 budget)
        assert_eq!(manager.session_total_tokens(), 500);
        assert_eq!(manager.quota_level(), QuotaLevel::Normal);

        // 2. Warning threshold (at or above 50%)
        manager.record_usage(300, 300, Some(0.02)); // +600 = 1100 total (55%)
        assert_eq!(manager.quota_level(), QuotaLevel::Warning);

        // 3. Danger threshold (at or above 90%)
        manager.record_usage(400, 400, Some(0.03)); // +800 = 1900 total (95%)
        assert_eq!(manager.quota_level(), QuotaLevel::Danger);

        // 4. Exhausted threshold
        manager.record_usage(100, 50, Some(0.01)); // +150 = 2050 total (> 2000)
        assert_eq!(manager.quota_level(), QuotaLevel::Exhausted);
    }

    #[test]
    fn test_speculative_token_reservations() {
        let config = TokenQuotaConfig {
            max_tpm: Some(1000),
            max_tph: None,
            max_tpd: None,
            max_input_tpm: None,
            max_output_tpm: None,
            session_token_budget: Some(1500),
            soft_quota_ratio: 0.80,
            danger_quota_ratio: 0.95,
            cost_limit_usd: None,
            cost_per_hour_usd: None,
            cost_per_day_usd: None,
        };

        let mut manager = TokenQuotaManager::new(config);

        // Reserve 800 tokens
        let ticket = manager.reserve(800).expect("Reservation should succeed");
        assert_eq!(ticket.estimated_tokens, 800);

        // Attempting to reserve another 800 should fail because 800 (reserved) + 800 = 1600 > 1500 budget
        let second_res = manager.reserve(800);
        assert!(second_res.is_err());

        // Commit reservation with actual 600 tokens
        assert!(manager.commit_reservation(ticket, 400, 200, None).is_ok());
        assert_eq!(manager.session_total_tokens(), 600);

        // Now reserving another 800 succeeds (600 + 800 = 1400 <= 1500)
        let ticket2 = manager.reserve(800).expect("Second reservation should now succeed");
        manager.cancel_reservation(ticket2);
    }

    #[test]
    fn test_throttle_engine_adaptive_pacing() {
        let config = ThrottleConfig {
            turns: TurnRateLimitConfig {
                max_turns_per_session: Some(10),
                min_interval_ms: 0,
                ..TurnRateLimitConfig::default()
            },
            tokens: TokenQuotaConfig {
                session_token_budget: Some(1000),
                soft_quota_ratio: 0.50,
                danger_quota_ratio: 0.90,
                ..TokenQuotaConfig::default()
            },
            policy: ThrottlePolicy::AdaptivePacing,
            adaptive_pacing: true,
            max_pacing_delay_ms: 2000,
        };

        let mut engine = ThrottleEngine::new(config);

        // 0% usage -> no delay
        let delay = engine.calculate_adaptive_pacing_ms();
        assert_eq!(delay, 0);

        // 60% usage -> soft threshold is 50%, so factor = (0.6 - 0.5) / 0.5 = 0.2 -> 400ms delay
        engine.record_turn_finish(400, 200, None); // 600 total
        let delay_60 = engine.calculate_adaptive_pacing_ms();
        assert!(delay_60 >= 350 && delay_60 <= 450);

        // Status report
        let report = engine.status_report();
        assert_eq!(report.metrics.total_tokens, 600);
        assert_eq!(report.metrics.quota_level, QuotaLevel::Warning);

        let summary = report.format_summary();
        assert!(summary.contains("Tokens: 600"));

        let detailed = report.format_detailed_report();
        assert!(detailed.contains("Fusion Quota & Throttle Status"));
    }

    #[test]
    fn test_unlimited_engine() {
        let mut engine = ThrottleEngine::unlimited();
        let decision = engine.pre_turn_check(100_000);
        assert!(decision.is_allowed());
        assert!(engine.record_turn_start().is_ok());
        engine.record_turn_finish(50_000, 50_000, Some(10.0));
        assert_eq!(engine.metrics().quota_level, QuotaLevel::Normal);
    }

    #[test]
    fn test_gauge_rendering() {
        let gauge_0 = ThrottleStatusReport::render_gauge(0.0, 10);
        assert_eq!(gauge_0, "[░░░░░░░░░░] 0.0%");

        let gauge_50 = ThrottleStatusReport::render_gauge(50.0, 10);
        assert_eq!(gauge_50, "[█████░░░░░] 50.0%");

        let gauge_100 = ThrottleStatusReport::render_gauge(100.0, 10);
        assert_eq!(gauge_100, "[██████████] 100.0%");
    }
}
