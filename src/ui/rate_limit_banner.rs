//! Inline Rate Limit Warning Banner & Countdown Timer Widget
//!
//! Provides high-polish visual warning banners, countdown timers, and progress meters
//! when LLM provider API rate limits (HTTP 429 / HTTP 529 / Token / Request quotas) are encountered:
//!
//! - **Real-Time Countdown Timer**: High-precision display of remaining wait time until next retry
//!   (e.g. `04.8s`, `1m 23s`, `450ms`, `00.0s [Retrying...]`).
//! - **Visual Countdown Progress Bar**: Animated / smooth Unicode progress meter showing elapsed vs remaining delay
//!   (`Smooth`, `Blocks`, `Ascii`, `Dotted`, `Braille`).
//! - **Rich Metadata Breakdown**:
//!   1. **Provider & Model**: Badges displaying active provider (e.g. `Anthropic`, `OpenAI`, `OpenRouter`)
//!      and model identifier (e.g. `claude-3-7-sonnet`, `gpt-4o`).
//!   2. **Rate Limit Reason**: `RequestsPerMinute` (RPM), `TokensPerMinute` (TPM), `TokensPerDay` (TPD),
//!      `ConcurrentRequests`, `QuotaExceeded`, `DailyQuota`, `ServerOverload` (529/503), `TurnRateLimit`.
//!   3. **Retry Attempt Counter**: Displaying current attempt out of max retries (e.g. `Attempt 2 / 5`).
//!   4. **Backoff Strategy**: `HeaderSpecified` (`Retry-After`), `ExponentialBackoff`, `Linear`, `FixedDelay`, `AdaptivePacing`.
//!   5. **Quota Metrics**: RPM / TPM remaining and reset timestamps extracted from headers when available.
//!   6. **Actionable Suggestions**: Quick shortcuts (e.g. `[Esc] Cancel`, `[/model] Switch Model`, `[/compact] Prune Tokens`).
//! - **Multi-Format Rendering**:
//!   - `RateLimitBannerWidget`: Full Ratatui widget with responsive tall, medium, and compact layouts.
//!   - `RateLimitMiniBannerWidget`: Single-line inline pill / status bar widget.
//!   - `RateLimitCountdownBarWidget`: Standalone countdown progress bar widget.
//!   - `render_rate_limit_banner_ansi`: Zero-dependency, pure-Rust ANSI terminal string formatter with box drawing.
//!   - `render_rate_limit_compact_ansi` & `render_rate_limit_pill_ansi`: Compact inline strings.
//! - **Stateful Tracker**:
//!   - `RateLimitTracker`: Manages active rate limits, backoff calculations, countdown ticks, and resolution events.

use std::fmt;
use std::time::{Duration, Instant};

use ratatui::{
    buffer::Buffer,
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Clear, Paragraph, Widget, Wrap},
    Frame,
};
use serde::{Deserialize, Serialize};

use crate::ui::theme::Theme;

// ---------------------------------------------------------------------------
// 1. Constants & Thresholds
// ---------------------------------------------------------------------------

/// Default standard banner width in terminal characters.
pub const DEFAULT_BANNER_WIDTH: usize = 72;

/// Minimum safe banner width in characters.
pub const MIN_BANNER_WIDTH: usize = 42;

/// Default countdown meter width in characters.
pub const DEFAULT_METER_WIDTH: usize = 24;

/// Spinner animation frames for active waiting / retry states.
pub const SPINNER_FRAMES: &[&str] = &["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];

// ANSI escape sequences
const ANSI_RESET: &str = "\x1b[0m";
const ANSI_BOLD: &str = "\x1b[1m";
const ANSI_DIM: &str = "\x1b[2m";
const ANSI_ITALIC: &str = "\x1b[3m";
const ANSI_YELLOW: &str = "\x1b[33m";
const ANSI_BOLD_YELLOW: &str = "\x1b[1;33m";
const ANSI_RED: &str = "\x1b[31m";
const ANSI_BOLD_RED: &str = "\x1b[1;31m";
const ANSI_BG_RED: &str = "\x1b[41;1;37m";
const ANSI_BG_YELLOW: &str = "\x1b[43;1;30m";
const ANSI_BG_DARK_YELLOW: &str = "\x1b[48;5;178;1;30m";
const ANSI_GREEN: &str = "\x1b[32m";
const ANSI_BOLD_GREEN: &str = "\x1b[1;32m";
const ANSI_CYAN: &str = "\x1b[36m";
const ANSI_BOLD_CYAN: &str = "\x1b[1;36m";
const ANSI_GRAY: &str = "\x1b[90m";
const ANSI_LIGHT_GRAY: &str = "\x1b[37m";
const ANSI_WHITE: &str = "\x1b[97m";
const ANSI_BOLD_WHITE: &str = "\x1b[1;97m";
const ANSI_MAGENTA: &str = "\x1b[35m";
const ANSI_BOLD_MAGENTA: &str = "\x1b[1;35m";

// ---------------------------------------------------------------------------
// 2. Rate Limit Reason & Classification
// ---------------------------------------------------------------------------

/// Categorization of the specific rate limit or throttle trigger encountered.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum RateLimitKind {
    /// Requests-per-minute (RPM) ceiling hit.
    RequestsPerMinute,
    /// Tokens-per-minute (TPM) quota reached.
    TokensPerMinute,
    /// Tokens-per-day (TPD) or daily token quota exhausted.
    TokensPerDay,
    /// Concurrent in-flight request limit exceeded.
    ConcurrentRequests,
    /// Monthly or billing tier quota exhausted.
    QuotaExceeded,
    /// Daily API credits / budget cap reached.
    DailyQuota,
    /// Account tier concurrency or usage cap.
    TierCap,
    /// Upstream provider capacity overload (e.g. Anthropic 529, OpenAI 503).
    ServerOverload,
    /// Local agent turn-rate throttle limit.
    TurnRateLimit,
    /// Generic HTTP 429 Too Many Requests.
    Generic429,
    /// Custom / provider-specific rate limit classification.
    Custom(String),
}

impl Default for RateLimitKind {
    fn default() -> Self {
        Self::Generic429
    }
}

impl RateLimitKind {
    /// Returns a short badge title for the rate limit reason.
    pub fn badge_label(&self) -> &'static str {
        match self {
            Self::RequestsPerMinute => "RPM LIMIT",
            Self::TokensPerMinute => "TPM LIMIT",
            Self::TokensPerDay => "DAILY TOKEN CAP",
            Self::ConcurrentRequests => "CONCURRENCY CAP",
            Self::QuotaExceeded => "QUOTA EXCEEDED",
            Self::DailyQuota => "DAILY QUOTA",
            Self::TierCap => "TIER LIMIT",
            Self::ServerOverload => "PROVIDER OVERLOAD",
            Self::TurnRateLimit => "TURN THROTTLE",
            Self::Generic429 => "RATE LIMITED",
            Self::Custom(_) => "CUSTOM LIMIT",
        }
    }

    /// Returns a human-friendly description of the limit.
    pub fn description(&self) -> &'static str {
        match self {
            Self::RequestsPerMinute => "Requests per minute limit reached",
            Self::TokensPerMinute => "Tokens per minute quota exceeded",
            Self::TokensPerDay => "Daily token allotment exhausted",
            Self::ConcurrentRequests => "Maximum concurrent requests in flight",
            Self::QuotaExceeded => "API account usage quota exhausted",
            Self::DailyQuota => "Daily budget limit reached",
            Self::TierCap => "API tier throughput limit reached",
            Self::ServerOverload => "Provider servers overloaded (HTTP 529/503)",
            Self::TurnRateLimit => "Agent turn rate limiter active",
            Self::Generic429 => "HTTP 429 Too Many Requests received",
            Self::Custom(_) => "Custom rate limit constraint active",
        }
    }

    /// Returns a standard icon / emoji representation.
    pub fn icon(&self) -> &'static str {
        match self {
            Self::RequestsPerMinute => "⚡",
            Self::TokensPerMinute => "📊",
            Self::TokensPerDay => "📅",
            Self::ConcurrentRequests => "🚦",
            Self::QuotaExceeded => "🛑",
            Self::DailyQuota => "💰",
            Self::TierCap => "🔒",
            Self::ServerOverload => "🔥",
            Self::TurnRateLimit => "⏱️",
            Self::Generic429 => "⏳",
            Self::Custom(_) => "⚠️",
        }
    }

    /// Canonical identifier string.
    pub fn as_str(&self) -> &str {
        match self {
            Self::RequestsPerMinute => "requests_per_minute",
            Self::TokensPerMinute => "tokens_per_minute",
            Self::TokensPerDay => "tokens_per_day",
            Self::ConcurrentRequests => "concurrent_requests",
            Self::QuotaExceeded => "quota_exceeded",
            Self::DailyQuota => "daily_quota",
            Self::TierCap => "tier_cap",
            Self::ServerOverload => "server_overload",
            Self::TurnRateLimit => "turn_rate_limit",
            Self::Generic429 => "generic_429",
            Self::Custom(s) => s.as_str(),
        }
    }
}

// ---------------------------------------------------------------------------
// 3. Backoff Strategy
// ---------------------------------------------------------------------------

/// Backoff algorithm or source dictating the retry delay.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum BackoffStrategy {
    /// Explicit `Retry-After` or `x-ratelimit-reset` header from server.
    HeaderSpecified,
    /// Exponential backoff with optional jitter ($initial \times factor^{attempt}$).
    Exponential,
    /// Linear step backoff.
    Linear,
    /// Fixed constant delay between retries.
    Fixed,
    /// Adaptive pacing / client-side rate limiter.
    AdaptivePacing,
}

impl Default for BackoffStrategy {
    fn default() -> Self {
        Self::HeaderSpecified
    }
}

impl BackoffStrategy {
    /// Human readable label.
    pub fn label(&self) -> &'static str {
        match self {
            Self::HeaderSpecified => "Header Specified (Retry-After)",
            Self::Exponential => "Exponential Backoff",
            Self::Linear => "Linear Backoff",
            Self::Fixed => "Fixed Interval",
            Self::AdaptivePacing => "Adaptive Pacing",
        }
    }

    /// Short badge name.
    pub fn short_name(&self) -> &'static str {
        match self {
            Self::HeaderSpecified => "Retry-After",
            Self::Exponential => "Exponential",
            Self::Linear => "Linear",
            Self::Fixed => "Fixed",
            Self::AdaptivePacing => "Pacing",
        }
    }
}

// ---------------------------------------------------------------------------
// 4. Rate Limit Lifecycle Status
// ---------------------------------------------------------------------------

/// Current lifecycle state of an active rate limit event.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum RateLimitStatus {
    /// Waiting for countdown timer to elapse before retrying.
    Waiting,
    /// Countdown elapsed; actively dispatching retry attempt.
    Retrying,
    /// Retry succeeded and request completed successfully.
    Succeeded,
    /// Max retries exceeded or fatal unrecoverable rate limit error.
    Failed,
    /// Retry was cancelled by user action (e.g. Esc, Ctrl-C, model switch).
    Cancelled,
}

impl Default for RateLimitStatus {
    fn default() -> Self {
        Self::Waiting
    }
}

impl RateLimitStatus {
    /// Returns true if currently in an active waiting countdown.
    #[inline]
    pub fn is_waiting(&self) -> bool {
        matches!(self, Self::Waiting)
    }

    /// Returns true if actively retrying.
    #[inline]
    pub fn is_retrying(&self) -> bool {
        matches!(self, Self::Retrying)
    }

    /// Returns true if the rate limit event is finished (Succeeded, Failed, Cancelled).
    #[inline]
    pub fn is_terminal(&self) -> bool {
        matches!(self, Self::Succeeded | Self::Failed | Self::Cancelled)
    }

    /// Status badge label.
    pub fn badge_label(&self) -> &'static str {
        match self {
            Self::Waiting => "WAITING",
            Self::Retrying => "RETRYING NOW",
            Self::Succeeded => "RECOVERED",
            Self::Failed => "RETRIES EXHAUSTED",
            Self::Cancelled => "CANCELLED",
        }
    }
}

// ---------------------------------------------------------------------------
// 5. RateLimitInfo (Core Data Model)
// ---------------------------------------------------------------------------

/// Complete diagnostic context and countdown state for an active rate limit incident.
#[derive(Debug, Clone)]
pub struct RateLimitInfo {
    /// LLM provider name (e.g. "Anthropic", "OpenAI", "OpenRouter", "Groq", "Local").
    pub provider: Option<String>,
    /// Model identifier (e.g. "claude-3-7-sonnet", "gpt-4o", "deepseek-chat").
    pub model: Option<String>,
    /// Categorized reason for the rate limit.
    pub kind: RateLimitKind,
    /// Current lifecycle state.
    pub status: RateLimitStatus,
    /// Current retry attempt number (1-indexed, e.g. attempt 2).
    pub attempt: u32,
    /// Maximum configured retry attempts before giving up (e.g. 5).
    pub max_retries: Option<u32>,
    /// Total duration allocated for this retry backoff cycle.
    pub total_delay: Duration,
    /// Exact instant when the rate limit backoff countdown started.
    pub started_at: Instant,
    /// Exact target instant when retry will be triggered.
    pub retry_at: Instant,
    /// Strategy used to calculate backoff duration.
    pub backoff_strategy: BackoffStrategy,
    /// Original error message or diagnostic snippet.
    pub message: Option<String>,
    /// Requests-per-minute limit from headers (if known).
    pub limit_rpm: Option<u32>,
    /// Requests-per-minute remaining from headers (if known).
    pub remaining_rpm: Option<u32>,
    /// Tokens-per-minute limit from headers (if known).
    pub limit_tpm: Option<u64>,
    /// Tokens-per-minute remaining from headers (if known).
    pub remaining_tpm: Option<u64>,
    /// Duration until quota window resets (if different from retry delay).
    pub reset_duration: Option<Duration>,
    /// Recommended user fallback action.
    pub suggested_action: Option<String>,
    /// Internal animation tick counter for spinner rendering.
    pub spinner_tick: usize,
}

impl RateLimitInfo {
    /// Creates a new `RateLimitInfo` with the given retry delay duration starting now.
    pub fn new(retry_after: Duration) -> Self {
        let now = Instant::now();
        Self {
            provider: None,
            model: None,
            kind: RateLimitKind::Generic429,
            status: RateLimitStatus::Waiting,
            attempt: 1,
            max_retries: Some(3),
            total_delay: retry_after,
            started_at: now,
            retry_at: now.checked_add(retry_after).unwrap_or(now),
            backoff_strategy: BackoffStrategy::HeaderSpecified,
            message: None,
            limit_rpm: None,
            remaining_rpm: None,
            limit_tpm: None,
            remaining_tpm: None,
            reset_duration: None,
            suggested_action: None,
            spinner_tick: 0,
        }
    }

    /// Sets the provider name.
    pub fn with_provider(mut self, provider: impl Into<String>) -> Self {
        self.provider = Some(provider.into());
        self
    }

    /// Sets the model name.
    pub fn with_model(mut self, model: impl Into<String>) -> Self {
        self.model = Some(model.into());
        self
    }

    /// Sets the rate limit reason kind.
    pub fn with_kind(mut self, kind: RateLimitKind) -> Self {
        self.kind = kind;
        self
    }

    /// Sets the attempt counter and optional max retries.
    pub fn with_attempt(mut self, attempt: u32, max_retries: Option<u32>) -> Self {
        self.attempt = attempt;
        self.max_retries = max_retries;
        self
    }

    /// Sets the backoff strategy.
    pub fn with_strategy(mut self, strategy: BackoffStrategy) -> Self {
        self.backoff_strategy = strategy;
        self
    }

    /// Sets the lifecycle status.
    pub fn with_status(mut self, status: RateLimitStatus) -> Self {
        self.status = status;
        self
    }

    /// Sets the diagnostic error message.
    pub fn with_message(mut self, message: impl Into<String>) -> Self {
        self.message = Some(message.into());
        self
    }

    /// Sets detailed rate limit quota numbers.
    pub fn with_limits(
        mut self,
        limit_rpm: Option<u32>,
        remaining_rpm: Option<u32>,
        limit_tpm: Option<u64>,
        remaining_tpm: Option<u64>,
    ) -> Self {
        self.limit_rpm = limit_rpm;
        self.remaining_rpm = remaining_rpm;
        self.limit_tpm = limit_tpm;
        self.remaining_tpm = remaining_tpm;
        self
    }

    /// Sets custom suggested action string.
    pub fn with_suggested_action(mut self, action: impl Into<String>) -> Self {
        self.suggested_action = Some(action.into());
        self
    }

    /// Parses a rate limit error string and constructs an initialized `RateLimitInfo`.
    pub fn from_error_str(error_str: &str, default_delay: Duration) -> Self {
        let lower = error_str.to_ascii_lowercase();

        // 1. Detect RateLimitKind
        let kind = if lower.contains("tokens per minute") || lower.contains("tpm") {
            RateLimitKind::TokensPerMinute
        } else if lower.contains("requests per minute") || lower.contains("rpm") {
            RateLimitKind::RequestsPerMinute
        } else if lower.contains("daily") || lower.contains("per day") || lower.contains("tpd") {
            RateLimitKind::TokensPerDay
        } else if lower.contains("concurrency") || lower.contains("concurrent") {
            RateLimitKind::ConcurrentRequests
        } else if lower.contains("quota exceeded") || lower.contains("insufficient_quota") {
            RateLimitKind::QuotaExceeded
        } else if lower.contains("529")
            || lower.contains("overloaded")
            || lower.contains("capacity")
        {
            RateLimitKind::ServerOverload
        } else if lower.contains("turn limit") || lower.contains("turn rate") {
            RateLimitKind::TurnRateLimit
        } else {
            RateLimitKind::Generic429
        };

        // 2. Extract Retry-After if present in string (e.g. "try again in 4.5s" or "Retry-After: 12")
        let parsed_delay = parse_delay_from_error_str(error_str).unwrap_or(default_delay);

        let mut info = Self::new(parsed_delay)
            .with_kind(kind)
            .with_message(error_str);

        // 3. Provider detection heuristic
        if lower.contains("anthropic") || lower.contains("claude") {
            info.provider = Some("Anthropic".to_string());
        } else if lower.contains("openai") || lower.contains("gpt-") {
            info.provider = Some("OpenAI".to_string());
        } else if lower.contains("openrouter") {
            info.provider = Some("OpenRouter".to_string());
        } else if lower.contains("groq") {
            info.provider = Some("Groq".to_string());
        } else if lower.contains("deepseek") {
            info.provider = Some("DeepSeek".to_string());
        }

        info
    }

    /// Calculates the remaining wait duration from current time until `retry_at`.
    #[inline]
    pub fn time_remaining(&self) -> Duration {
        self.time_remaining_at(Instant::now())
    }

    /// Calculates the remaining wait duration relative to a given `now` instant.
    pub fn time_remaining_at(&self, now: Instant) -> Duration {
        if now >= self.retry_at {
            Duration::from_secs(0)
        } else {
            self.retry_at.duration_since(now)
        }
    }

    /// Calculates the elapsed wait duration since `started_at`.
    #[inline]
    pub fn elapsed(&self) -> Duration {
        self.elapsed_at(Instant::now())
    }

    /// Calculates the elapsed wait duration relative to a given `now` instant.
    pub fn elapsed_at(&self, now: Instant) -> Duration {
        if now <= self.started_at {
            Duration::from_secs(0)
        } else {
            now.duration_since(self.started_at)
        }
    }

    /// Calculates progress ratio from 0.0 (just started waiting) to 1.0 (ready to retry).
    #[inline]
    pub fn progress(&self) -> f32 {
        self.progress_at(Instant::now())
    }

    /// Calculates progress ratio relative to a given `now` instant.
    pub fn progress_at(&self, now: Instant) -> f32 {
        let total_nanos = self.total_delay.as_nanos() as f64;
        if total_nanos <= 0.0 {
            return 1.0;
        }
        let elapsed_nanos = self.elapsed_at(now).as_nanos() as f64;
        let ratio = elapsed_nanos / total_nanos;
        ratio.clamp(0.0, 1.0) as f32
    }

    /// Returns `true` if the countdown timer has elapsed and retry should be initiated.
    #[inline]
    pub fn is_ready_to_retry(&self) -> bool {
        self.is_ready_to_retry_at(Instant::now())
    }

    /// Returns `true` if ready to retry relative to a given `now` instant.
    #[inline]
    pub fn is_ready_to_retry_at(&self, now: Instant) -> bool {
        now >= self.retry_at
    }

    /// Formats the remaining countdown time into a human-friendly string (e.g. `04.8s`, `1m 24s`, `350ms`).
    pub fn format_countdown(&self) -> String {
        self.format_countdown_at(Instant::now())
    }

    /// Formats the remaining countdown time relative to `now`.
    pub fn format_countdown_at(&self, now: Instant) -> String {
        let remaining = self.time_remaining_at(now);
        format_time_compact(remaining)
    }

    /// Formats the attempt badge (e.g. `Attempt 2/5` or `Attempt 1`).
    pub fn format_attempt_badge(&self) -> String {
        if let Some(max) = self.max_retries {
            format!("Attempt {}/{}", self.attempt, max)
        } else {
            format!("Attempt {}", self.attempt)
        }
    }

    /// Formats quota limits string if any are present (e.g. `TPM: 32.5k/40k | RPM: 0/500`).
    pub fn format_metrics_badge(&self) -> Option<String> {
        let mut parts = Vec::new();
        if let (Some(rem), Some(lim)) = (self.remaining_rpm, self.limit_rpm) {
            parts.push(format!("RPM: {}/{}", rem, lim));
        } else if let Some(lim) = self.limit_rpm {
            parts.push(format!("RPM Limit: {}", lim));
        }

        if let (Some(rem), Some(lim)) = (self.remaining_tpm, self.limit_tpm) {
            parts.push(format!(
                "TPM: {}/{}",
                format_tokens_compact(rem),
                format_tokens_compact(lim)
            ));
        } else if let Some(lim) = self.limit_tpm {
            parts.push(format!("TPM Limit: {}", format_tokens_compact(lim)));
        }

        if parts.is_empty() {
            None
        } else {
            Some(parts.join("  |  "))
        }
    }

    /// Current animated spinner character.
    pub fn spinner_frame(&self) -> &'static str {
        SPINNER_FRAMES[self.spinner_tick % SPINNER_FRAMES.len()]
    }

    /// Increments the internal animation tick.
    pub fn tick(&mut self) {
        self.spinner_tick = self.spinner_tick.wrapping_add(1);
    }
}

// ---------------------------------------------------------------------------
// 6. Time & Metric Formatting Utilities
// ---------------------------------------------------------------------------

/// Formats a `Duration` into a clean compact string:
/// - `>= 60s`: `1m 24s`
/// - `>= 10s`: `14.2s`
/// - `< 10s` and `> 0s`: `04.8s` or `0.6s`
/// - `== 0s`: `00.0s`
pub fn format_time_compact(d: Duration) -> String {
    let total_secs = d.as_secs_f64();
    if total_secs <= 0.0 {
        return "00.0s".to_string();
    }
    if total_secs >= 3600.0 {
        let hours = (total_secs / 3600.0).floor() as u64;
        let mins = ((total_secs % 3600.0) / 60.0).floor() as u64;
        format!("{}h {:02}m", hours, mins)
    } else if total_secs >= 60.0 {
        let mins = (total_secs / 60.0).floor() as u64;
        let secs = (total_secs % 60.0).floor() as u64;
        format!("{}m {:02}s", mins, secs)
    } else if total_secs >= 10.0 {
        format!("{:.1}s", total_secs)
    } else {
        format!("{:04.1}s", total_secs)
    }
}

/// Formats token counts cleanly (e.g. `1.2k`, `450k`, `2.1M`).
pub fn format_tokens_compact(count: u64) -> String {
    if count >= 1_000_000 {
        let val = (count as f64) / 1_000_000.0;
        if (val - val.floor()).abs() < 0.05 {
            format!("{:.0}M", val)
        } else {
            format!("{:.1}M", val)
        }
    } else if count >= 1_000 {
        let val = (count as f64) / 1_000.0;
        if (val - val.floor()).abs() < 0.05 {
            format!("{:.0}k", val)
        } else {
            format!("{:.1}k", val)
        }
    } else {
        count.to_string()
    }
}

/// Parses delay seconds from error message snippets.
fn parse_delay_from_error_str(error: &str) -> Option<Duration> {
    let lower = error.to_ascii_lowercase();

    // Look for "retry after X" or "try again in X" or "wait X seconds"
    let patterns = [
        "retry-after:",
        "retry after",
        "try again in",
        "retry in",
        "wait",
        "resets in",
    ];

    for pat in patterns {
        if let Some(pos) = lower.find(pat) {
            let slice = &error[pos + pat.len()..];
            let mut num_str = String::new();
            let mut found_digit = false;

            for ch in slice.trim_start().chars() {
                if ch.is_ascii_digit() || ch == '.' {
                    num_str.push(ch);
                    found_digit = true;
                } else if found_digit {
                    break;
                }
            }

            if let Ok(secs) = num_str.parse::<f64>() {
                if secs > 0.0 && secs < 86400.0 {
                    return Some(Duration::from_secs_f64(secs));
                }
            }
        }
    }

    None
}

// ---------------------------------------------------------------------------
// 7. Visual Progress Bar Styles & Box Styles
// ---------------------------------------------------------------------------

/// Visual style character set for the countdown progress bar.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ProgressBarStyle {
    /// Smooth high-resolution Unicode fractional sub-blocks (`█`, `▉`, `▊`, `▋`, `▌`, `▍`, `▎`, `▏`).
    #[default]
    Smooth,
    /// Discrete Unicode blocks (`■`, `□`).
    Blocks,
    /// Pure ASCII meter (`=`, `>`, `-`, ` `).
    Ascii,
    /// Circular dots (`●`, `○`).
    Dotted,
    /// Braille dots (`⣿`, `⣀`).
    Braille,
}

impl ProgressBarStyle {
    /// Renders a single progress bar string of specified width representing progress from 0.0 to 1.0.
    pub fn render_bar(&self, progress: f32, width: usize) -> String {
        let width = width.max(4);
        let clamped = progress.clamp(0.0, 1.0);

        match self {
            Self::Smooth => {
                const FRACTIONS: &[char] = &[' ', '▏', '▎', '▍', '▌', '▋', '▊', '▉', '█'];
                let full_blocks_f = clamped * (width as f32);
                let full_blocks = full_blocks_f.floor() as usize;
                let remainder = full_blocks_f - (full_blocks as f32);

                let mut result = String::with_capacity(width * 4);
                let full_count = full_blocks.min(width);
                for _ in 0..full_count {
                    result.push('█');
                }

                if full_count < width {
                    let frac_idx = (remainder * 8.0).round() as usize;
                    let frac_char = FRACTIONS[frac_idx.min(8)];
                    result.push(frac_char);

                    for _ in (full_count + 1)..width {
                        result.push('░');
                    }
                }
                result
            }
            Self::Blocks => {
                let filled = ((clamped * (width as f32)).round() as usize).min(width);
                let mut result = String::with_capacity(width * 4);
                for _ in 0..filled {
                    result.push('■');
                }
                for _ in filled..width {
                    result.push('□');
                }
                result
            }
            Self::Ascii => {
                let filled = ((clamped * (width as f32)).round() as usize).min(width);
                let mut result = String::with_capacity(width + 2);
                result.push('[');
                for i in 0..width {
                    if i < filled {
                        if i + 1 == filled && filled < width {
                            result.push('>');
                        } else {
                            result.push('=');
                        }
                    } else {
                        result.push('-');
                    }
                }
                result.push(']');
                result
            }
            Self::Dotted => {
                let filled = ((clamped * (width as f32)).round() as usize).min(width);
                let mut result = String::with_capacity(width * 4);
                for _ in 0..filled {
                    result.push('●');
                }
                for _ in filled..width {
                    result.push('○');
                }
                result
            }
            Self::Braille => {
                let filled = ((clamped * (width as f32)).round() as usize).min(width);
                let mut result = String::with_capacity(width * 4);
                for _ in 0..filled {
                    result.push('⣿');
                }
                for _ in filled..width {
                    result.push('⣀');
                }
                result
            }
        }
    }
}

/// Type alias for disambiguating from other UI progress bar styles.
pub type RateLimitProgressBarStyle = ProgressBarStyle;

/// Box border characters for terminal banners.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum BannerBoxStyle {
    /// Rounded corners: `╭─╮`, `│`, `╰─╯`
    #[default]
    Rounded,
    /// Heavy borders: `┏━┓`, `┃`, `┗━┛`
    Heavy,
    /// Double borders: `╔═╗`, `║`, `╚═╝`
    Double,
    /// Pure ASCII: `+--+`, `|`, `+--+`
    Ascii,
    /// Minimal vertical accent bar on left: `│`
    Minimal,
}

impl BannerBoxStyle {
    pub fn top_left(&self) -> &'static str {
        match self {
            Self::Rounded => "╭",
            Self::Heavy => "┏",
            Self::Double => "╔",
            Self::Ascii => "+",
            Self::Minimal => "│",
        }
    }

    pub fn top_right(&self) -> &'static str {
        match self {
            Self::Rounded => "╮",
            Self::Heavy => "┓",
            Self::Double => "╗",
            Self::Ascii => "+",
            Self::Minimal => "",
        }
    }

    pub fn bottom_left(&self) -> &'static str {
        match self {
            Self::Rounded => "╰",
            Self::Heavy => "┗",
            Self::Double => "╚",
            Self::Ascii => "+",
            Self::Minimal => "│",
        }
    }

    pub fn bottom_right(&self) -> &'static str {
        match self {
            Self::Rounded => "╯",
            Self::Heavy => "┛",
            Self::Double => "╝",
            Self::Ascii => "+",
            Self::Minimal => "",
        }
    }

    pub fn horizontal(&self) -> &'static str {
        match self {
            Self::Rounded | Self::Ascii | Self::Minimal => "─",
            Self::Heavy => "━",
            Self::Double => "═",
        }
    }

    pub fn vertical(&self) -> &'static str {
        match self {
            Self::Rounded | Self::Ascii | Self::Minimal => "│",
            Self::Heavy => "┃",
            Self::Double => "║",
        }
    }
}

/// Type alias for disambiguating from other UI banner box styles.
pub type RateLimitBannerBoxStyle = BannerBoxStyle;

// ---------------------------------------------------------------------------
// 8. ANSI Terminal Banner Rendering (Zero External Dependencies)
// ---------------------------------------------------------------------------

/// Renders a full multi-line rate limit warning banner into a standalone ANSI string.
pub fn render_rate_limit_banner_ansi(
    info: &RateLimitInfo,
    box_style: BannerBoxStyle,
    banner_width: usize,
) -> String {
    let width = banner_width.max(MIN_BANNER_WIDTH);
    let horiz = box_style.horizontal();
    let vert = box_style.vertical();

    let border_color = match info.status {
        RateLimitStatus::Waiting => ANSI_BOLD_YELLOW,
        RateLimitStatus::Retrying => ANSI_BOLD_CYAN,
        RateLimitStatus::Succeeded => ANSI_BOLD_GREEN,
        RateLimitStatus::Failed => ANSI_BOLD_RED,
        RateLimitStatus::Cancelled => ANSI_GRAY,
    };

    let mut out = String::new();

    // 1. Top border with title badge
    let icon = info.kind.icon();
    let title_badge = format!(" {} RATE LIMIT NOTICE ", icon);
    let top_bar_len = width.saturating_sub(title_badge.len() + 4);
    let left_bar_len = 3;
    let right_bar_len = top_bar_len.saturating_sub(left_bar_len);

    let left_bars = horiz.repeat(left_bar_len);
    let right_bars = horiz.repeat(right_bar_len);

    out.push_str(&format!(
        "{}{}{}{}{}{}{}{}{}\n",
        border_color,
        box_style.top_left(),
        left_bars,
        ANSI_BG_YELLOW,
        title_badge,
        ANSI_RESET,
        border_color,
        right_bars,
        box_style.top_right()
    ));

    // Helper for bordered lines
    let render_line = |content: &str, out: &mut String| {
        let visible_len = visible_char_len(content);
        let padding = width.saturating_sub(visible_len + 4);
        out.push_str(&format!(
            "{}{}  {}{}{}  {}{}\n",
            border_color,
            vert,
            ANSI_RESET,
            content,
            " ".repeat(padding),
            border_color,
            vert
        ));
    };

    // 2. Line 1: Reason and Provider / Model badges
    let provider_str = info.provider.as_deref().unwrap_or("API Provider");
    let model_str = info.model.as_deref().unwrap_or("Active Model");
    let reason_badge = info.kind.badge_label();

    let line1 = format!(
        "{}{}{}: {}{}{} ({}{}{})  |  {}{}{}",
        ANSI_BOLD_WHITE,
        provider_str,
        ANSI_RESET,
        ANSI_CYAN,
        model_str,
        ANSI_RESET,
        ANSI_DIM,
        info.kind.description(),
        ANSI_RESET,
        ANSI_BOLD_YELLOW,
        reason_badge,
        ANSI_RESET
    );
    render_line(&line1, &mut out);

    // 3. Line 2: Countdown Timer & Visual Progress Meter
    let countdown_str = info.format_countdown();
    let progress = info.progress();
    let meter_width = (width.saturating_sub(36)).clamp(10, 32);
    let meter = ProgressBarStyle::Smooth.render_bar(progress, meter_width);
    let progress_pct = (progress * 100.0).round() as u32;

    let countdown_color = if progress >= 0.90 {
        ANSI_BOLD_GREEN
    } else if progress >= 0.50 {
        ANSI_BOLD_YELLOW
    } else {
        ANSI_BOLD_RED
    };

    let status_desc = match info.status {
        RateLimitStatus::Waiting => format!(
            "Retrying in: {}{}{}",
            countdown_color, countdown_str, ANSI_RESET
        ),
        RateLimitStatus::Retrying => format!("{}Retrying now...{}", ANSI_BOLD_CYAN, ANSI_RESET),
        RateLimitStatus::Succeeded => {
            format!("{}Request succeeded!{}", ANSI_BOLD_GREEN, ANSI_RESET)
        }
        RateLimitStatus::Failed => format!("{}Max retries exceeded{}", ANSI_BOLD_RED, ANSI_RESET),
        RateLimitStatus::Cancelled => format!("{}Retry cancelled{}", ANSI_GRAY, ANSI_RESET),
    };

    let line2 = format!(
        "{} {}  {}{}{} {:>3}%",
        info.spinner_frame(),
        status_desc,
        countdown_color,
        meter,
        ANSI_RESET,
        progress_pct
    );
    render_line(&line2, &mut out);

    // 4. Line 3: Retry Attempts, Backoff Strategy & Quota Metrics
    let attempt_badge = info.format_attempt_badge();
    let strategy_label = info.backoff_strategy.short_name();
    let mut line3 = format!(
        "{} | Strategy: {}{}{}",
        attempt_badge, ANSI_DIM, strategy_label, ANSI_RESET
    );

    if let Some(metrics) = info.format_metrics_badge() {
        line3.push_str(&format!("  |  {}{}{}", ANSI_DIM, metrics, ANSI_RESET));
    }
    render_line(&line3, &mut out);

    // 5. Line 4: Actionable suggestions / hints
    let default_suggestion =
        "[Esc] Cancel  [/model] Switch Model  [/compact] Prune Tokens".to_string();
    let suggestion = info
        .suggested_action
        .as_ref()
        .unwrap_or(&default_suggestion);
    let line4 = format!("{}Action: {}{}", ANSI_DIM, suggestion, ANSI_RESET);
    render_line(&line4, &mut out);

    // 6. Bottom border
    let bottom_bars = horiz.repeat(width.saturating_sub(2));
    out.push_str(&format!(
        "{}{}{}{}{}\n",
        border_color,
        box_style.bottom_left(),
        bottom_bars,
        box_style.bottom_right(),
        ANSI_RESET
    ));

    out
}

/// Renders a compact 2-line inline rate limit notice string in ANSI format.
pub fn render_rate_limit_compact_ansi(info: &RateLimitInfo) -> String {
    let countdown = info.format_countdown();
    let progress = info.progress();
    let meter = ProgressBarStyle::Smooth.render_bar(progress, 16);
    let provider = info.provider.as_deref().unwrap_or("API");
    let model = info.model.as_deref().unwrap_or("Model");

    format!(
        "{}⚠️ RATE LIMITED{} [{} / {}] - Retrying in {}{}{} [{}] ({})\n",
        ANSI_BOLD_YELLOW,
        ANSI_RESET,
        provider,
        model,
        ANSI_BOLD_WHITE,
        countdown,
        ANSI_RESET,
        meter,
        info.format_attempt_badge()
    )
}

/// Renders a single-line inline pill badge (e.g. for status bars).
pub fn render_rate_limit_pill_ansi(info: &RateLimitInfo) -> String {
    let countdown = info.format_countdown();
    let icon = info.kind.icon();
    format!(
        "{}[{} Rate Limit: {} | {}]{}",
        ANSI_BG_YELLOW,
        icon,
        countdown,
        info.format_attempt_badge(),
        ANSI_RESET
    )
}

/// Computes visible character length stripping ANSI escape sequences.
fn visible_char_len(s: &str) -> usize {
    let mut in_escape = false;
    let mut count = 0;
    for ch in s.chars() {
        if ch == '\x1b' {
            in_escape = true;
        } else if in_escape {
            if ch == 'm' || ch == 'K' || ch == 'H' || ch == 'J' {
                in_escape = false;
            }
        } else {
            count += 1;
        }
    }
    count
}

// ---------------------------------------------------------------------------
// 9. Ratatui Widget Implementations
// ---------------------------------------------------------------------------

/// Full Ratatui Widget for rendering rich rate limit warning banners into terminal frames.
pub struct RateLimitBannerWidget<'a> {
    info: &'a RateLimitInfo,
    theme: Theme,
    border_type: BorderType,
    progress_style: ProgressBarStyle,
    show_suggestions: bool,
    show_metrics: bool,
}

impl<'a> RateLimitBannerWidget<'a> {
    /// Creates a new `RateLimitBannerWidget` for the specified rate limit info.
    pub fn new(info: &'a RateLimitInfo) -> Self {
        Self {
            info,
            theme: Theme::auto(),
            border_type: BorderType::Rounded,
            progress_style: ProgressBarStyle::Smooth,
            show_suggestions: true,
            show_metrics: true,
        }
    }

    /// Sets custom Theme.
    pub fn with_theme(mut self, theme: Theme) -> Self {
        self.theme = theme;
        self
    }

    /// Sets Ratatui border type.
    pub fn with_border_type(mut self, border_type: BorderType) -> Self {
        self.border_type = border_type;
        self
    }

    /// Sets progress bar rendering style.
    pub fn with_progress_style(mut self, progress_style: ProgressBarStyle) -> Self {
        self.progress_style = progress_style;
        self
    }

    /// Configures whether to display action suggestions.
    pub fn with_suggestions(mut self, show: bool) -> Self {
        self.show_suggestions = show;
        self
    }

    /// Configures whether to display detailed quota metrics.
    pub fn with_metrics(mut self, show: bool) -> Self {
        self.show_metrics = show;
        self
    }
}

impl<'a> Widget for RateLimitBannerWidget<'a> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        if area.width < 10 || area.height < 1 {
            return;
        }

        let alert_color = match self.info.status {
            RateLimitStatus::Waiting => self.theme.warning,
            RateLimitStatus::Retrying => self.theme.info,
            RateLimitStatus::Succeeded => self.theme.success,
            RateLimitStatus::Failed => self.theme.error,
            RateLimitStatus::Cancelled => self.theme.muted,
        };

        // If area is very small (1-2 lines), render compact inline banner
        if area.height <= 2 {
            let countdown = self.info.format_countdown();
            let provider = self.info.provider.as_deref().unwrap_or("API");
            let line = Line::from(vec![
                Span::styled(
                    " ⚠️ RATE LIMITED ",
                    Style::default()
                        .fg(Color::Black)
                        .bg(alert_color)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::raw(" "),
                Span::styled(
                    provider,
                    Style::default()
                        .fg(self.theme.primary)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::raw(" | Retrying in: "),
                Span::styled(
                    countdown,
                    Style::default()
                        .fg(alert_color)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::raw(" | "),
                Span::styled(
                    self.info.format_attempt_badge(),
                    Style::default().fg(self.theme.muted),
                ),
            ]);
            let paragraph = Paragraph::new(line);
            paragraph.render(area, buf);
            return;
        }

        // Standard bordered block
        let title_spans = vec![
            Span::styled(
                format!(" {} RATE LIMIT NOTICE ", self.info.kind.icon()),
                Style::default()
                    .fg(alert_color)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                format!("[ {} ]", self.info.status.badge_label()),
                Style::default().fg(self.theme.muted),
            ),
        ];

        let block = Block::default()
            .title(Line::from(title_spans))
            .borders(Borders::ALL)
            .border_type(self.border_type)
            .border_style(Style::default().fg(alert_color));

        let inner_area = block.inner(area);
        block.render(area, buf);

        if inner_area.height < 1 || inner_area.width < 8 {
            return;
        }

        let mut lines: Vec<Line> = Vec::new();

        // Line 1: Provider / Model info and Reason
        let provider_name = self.info.provider.as_deref().unwrap_or("API Provider");
        let model_name = self.info.model.as_deref().unwrap_or("Active Model");

        lines.push(Line::from(vec![
            Span::styled(
                provider_name,
                Style::default()
                    .fg(self.theme.primary)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw(" ("),
            Span::styled(model_name, Style::default().fg(self.theme.secondary)),
            Span::raw(")  |  Reason: "),
            Span::styled(
                self.info.kind.badge_label(),
                Style::default()
                    .fg(alert_color)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                format!(" - {}", self.info.kind.description()),
                Style::default().fg(self.theme.muted),
            ),
        ]));

        // Line 2: Countdown Timer & Visual Progress Meter
        let countdown = self.info.format_countdown();
        let progress = self.info.progress();
        let bar_width = (inner_area.width.saturating_sub(38) as usize).clamp(8, 36);
        let progress_bar = self.progress_style.render_bar(progress, bar_width);
        let pct = (progress * 100.0).round() as u32;

        lines.push(Line::from(vec![
            Span::styled(
                format!("{} ", self.info.spinner_frame()),
                Style::default().fg(alert_color),
            ),
            Span::raw("Retrying in: "),
            Span::styled(
                countdown,
                Style::default()
                    .fg(alert_color)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw("  [ "),
            Span::styled(progress_bar, Style::default().fg(alert_color)),
            Span::styled(
                format!(" ] {:>3}%", pct),
                Style::default().fg(self.theme.foreground),
            ),
        ]));

        // Line 3: Attempt counter, Strategy, and optional metrics
        if inner_area.height >= 3 {
            let mut line3_spans = vec![
                Span::styled(
                    self.info.format_attempt_badge(),
                    Style::default()
                        .fg(self.theme.foreground)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::raw("  |  Strategy: "),
                Span::styled(
                    self.info.backoff_strategy.label(),
                    Style::default().fg(self.theme.muted),
                ),
            ];

            if self.show_metrics {
                if let Some(metrics) = self.info.format_metrics_badge() {
                    line3_spans.push(Span::raw("  |  "));
                    line3_spans.push(Span::styled(metrics, Style::default().fg(self.theme.info)));
                }
            }

            lines.push(Line::from(line3_spans));
        }

        // Line 4: Action Suggestions
        if inner_area.height >= 4 && self.show_suggestions {
            let default_action =
                "[Esc] Cancel  [/model] Switch Model  [/compact] Prune Tokens".to_string();
            let action_text = self
                .info
                .suggested_action
                .as_ref()
                .unwrap_or(&default_action);

            lines.push(Line::from(vec![
                Span::styled("Actions: ", Style::default().fg(self.theme.muted)),
                Span::styled(
                    action_text.to_string(),
                    Style::default()
                        .fg(self.theme.foreground)
                        .add_modifier(Modifier::DIM),
                ),
            ]));
        }

        let paragraph = Paragraph::new(lines).wrap(Wrap { trim: true });
        paragraph.render(inner_area, buf);
    }
}

/// Standalone Single-Line Pill / Mini Banner Widget.
pub struct RateLimitMiniBannerWidget<'a> {
    info: &'a RateLimitInfo,
    theme: Theme,
}

impl<'a> RateLimitMiniBannerWidget<'a> {
    pub fn new(info: &'a RateLimitInfo) -> Self {
        Self {
            info,
            theme: Theme::auto(),
        }
    }

    pub fn with_theme(mut self, theme: Theme) -> Self {
        self.theme = theme;
        self
    }
}

impl<'a> Widget for RateLimitMiniBannerWidget<'a> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        if area.width < 6 || area.height < 1 {
            return;
        }

        let countdown = self.info.format_countdown();
        let provider = self.info.provider.as_deref().unwrap_or("API");
        let line = Line::from(vec![
            Span::styled(
                format!(" {} 429 ", self.info.kind.icon()),
                Style::default()
                    .fg(Color::Black)
                    .bg(self.theme.warning)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw(" "),
            Span::styled(
                provider,
                Style::default()
                    .fg(self.theme.primary)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw(": Retry in "),
            Span::styled(
                countdown,
                Style::default()
                    .fg(self.theme.warning)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw(" ("),
            Span::styled(
                self.info.format_attempt_badge(),
                Style::default().fg(self.theme.muted),
            ),
            Span::raw(")"),
        ]);

        let paragraph = Paragraph::new(line);
        paragraph.render(area, buf);
    }
}

/// Dedicated Countdown Progress Bar Widget.
pub struct RateLimitCountdownBarWidget<'a> {
    info: &'a RateLimitInfo,
    theme: Theme,
    style: ProgressBarStyle,
}

impl<'a> RateLimitCountdownBarWidget<'a> {
    pub fn new(info: &'a RateLimitInfo) -> Self {
        Self {
            info,
            theme: Theme::auto(),
            style: ProgressBarStyle::Smooth,
        }
    }

    pub fn with_theme(mut self, theme: Theme) -> Self {
        self.theme = theme;
        self
    }

    pub fn with_style(mut self, style: ProgressBarStyle) -> Self {
        self.style = style;
        self
    }
}

impl<'a> Widget for RateLimitCountdownBarWidget<'a> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        if area.width < 10 || area.height < 1 {
            return;
        }

        let progress = self.info.progress();
        let countdown = self.info.format_countdown();
        let bar_width = (area.width.saturating_sub(12) as usize).max(4);
        let bar_str = self.style.render_bar(progress, bar_width);

        let line = Line::from(vec![
            Span::styled(bar_str, Style::default().fg(self.theme.warning)),
            Span::raw(" "),
            Span::styled(
                countdown,
                Style::default()
                    .fg(self.theme.foreground)
                    .add_modifier(Modifier::BOLD),
            ),
        ]);

        let paragraph = Paragraph::new(line);
        paragraph.render(area, buf);
    }
}

/// Helper function to render a rate limit banner widget directly into a Ratatui Frame.
pub fn render_rate_limit_banner_widget(
    f: &mut Frame,
    area: Rect,
    info: &RateLimitInfo,
    theme: Option<&Theme>,
) {
    let t = theme.cloned().unwrap_or_else(Theme::auto);
    let widget = RateLimitBannerWidget::new(info).with_theme(t);
    f.render_widget(widget, area);
}

// ---------------------------------------------------------------------------
// 10. RateLimitTracker (Stateful Coordinator)
// ---------------------------------------------------------------------------

/// Lifecycle event outcome after ticking the rate limit tracker.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RateLimitTickOutcome {
    /// No active rate limit is currently in progress.
    Idle,
    /// Active countdown in progress, with time remaining.
    CountingDown(Duration),
    /// Countdown just reached zero and is ready to trigger a retry.
    ReadyToRetry,
}

/// Stateful manager tracking active and historical rate limit events.
#[derive(Debug, Clone, Default)]
pub struct RateLimitTracker {
    /// Active rate limit event (if any).
    active: Option<RateLimitInfo>,
    /// Cumulative count of rate limit events encountered in session.
    total_rate_limits_count: u32,
    /// Cumulative wait time spent in backoff across session.
    total_wait_duration: Duration,
}

impl RateLimitTracker {
    /// Creates a new empty `RateLimitTracker`.
    pub fn new() -> Self {
        Self::default()
    }

    /// Registers a new rate limit occurrence.
    pub fn register(&mut self, info: RateLimitInfo) {
        self.total_rate_limits_count += 1;
        self.total_wait_duration += info.total_delay;
        self.active = Some(info);
    }

    /// Returns a reference to the active rate limit info (if one exists).
    pub fn active(&self) -> Option<&RateLimitInfo> {
        self.active.as_ref()
    }

    /// Returns a mutable reference to the active rate limit info.
    pub fn active_mut(&mut self) -> Option<&mut RateLimitInfo> {
        self.active.as_mut()
    }

    /// Returns `true` if an active rate limit countdown is currently underway.
    pub fn is_active(&self) -> bool {
        self.active
            .as_ref()
            .map_or(false, |info| !info.status.is_terminal())
    }

    /// Advances the countdown timer and animation frames.
    pub fn tick(&mut self) -> RateLimitTickOutcome {
        let Some(info) = self.active.as_mut() else {
            return RateLimitTickOutcome::Idle;
        };

        if info.status.is_terminal() {
            return RateLimitTickOutcome::Idle;
        }

        info.tick();

        if info.is_ready_to_retry() {
            info.status = RateLimitStatus::Retrying;
            RateLimitTickOutcome::ReadyToRetry
        } else {
            RateLimitTickOutcome::CountingDown(info.time_remaining())
        }
    }

    /// Marks the active rate limit event as successfully resolved.
    pub fn record_success(&mut self) {
        if let Some(info) = self.active.as_mut() {
            info.status = RateLimitStatus::Succeeded;
        }
    }

    /// Marks the active rate limit event as failed (e.g. max retries exceeded).
    pub fn record_failure(&mut self) {
        if let Some(info) = self.active.as_mut() {
            info.status = RateLimitStatus::Failed;
        }
    }

    /// Cancels the active rate limit event.
    pub fn cancel(&mut self) {
        if let Some(info) = self.active.as_mut() {
            info.status = RateLimitStatus::Cancelled;
        }
    }

    /// Clears any active rate limit.
    pub fn clear(&mut self) {
        self.active = None;
    }

    /// Session total count of rate limit events encountered.
    pub fn total_count(&self) -> u32 {
        self.total_rate_limits_count
    }

    /// Session total time spent waiting on rate limits.
    pub fn total_wait_time(&self) -> Duration {
        self.total_wait_duration
    }
}

// ---------------------------------------------------------------------------
// 12. Simple Facade: RateLimitState & RateLimitBanner
// ---------------------------------------------------------------------------

/// Minimal state driving the `RateLimitBanner` widget.
///
/// Set `retry_after` to `Some(duration)` when a rate limit is active; the banner
/// renders a live countdown. Set to `None` (or leave default) when not rate-limited;
/// the widget renders nothing (zero height).
#[derive(Debug, Clone, Default)]
pub struct RateLimitState {
    /// Remaining wait time until the next retry is allowed.
    ///
    /// `None` → not currently rate-limited; banner is hidden.
    /// `Some(d)` → d is the time remaining; countdown is displayed.
    pub retry_after: Option<Duration>,
}

impl RateLimitState {
    /// Create a new state with no active rate limit.
    pub fn idle() -> Self {
        Self { retry_after: None }
    }

    /// Create a new state with an active rate limit countdown.
    pub fn waiting(retry_after: Duration) -> Self {
        Self {
            retry_after: Some(retry_after),
        }
    }

    /// Returns `true` when a rate limit is currently active.
    pub fn is_active(&self) -> bool {
        self.retry_after.is_some()
    }

    /// Advances the countdown by `elapsed`. Clears the limit when the timer reaches zero.
    pub fn tick(&mut self, elapsed: Duration) {
        if let Some(remaining) = self.retry_after.take() {
            self.retry_after = remaining.checked_sub(elapsed).filter(|d| !d.is_zero());
        }
    }
}

/// Lightweight Ratatui widget that displays a rate-limit countdown banner.
///
/// - When [`RateLimitState::retry_after`] is `None` the widget renders nothing and
///   occupies **zero rows** — callers can always allocate a fixed area without waste.
/// - When `Some(d)` the widget renders a compact one-line countdown pill:
///   `⏳ Rate limited — retry in <time>` styled in yellow.
///
/// For the full rich banner (provider, metrics, suggestions) use [`RateLimitBannerWidget`].
#[derive(Debug, Clone)]
pub struct RateLimitBanner<'a> {
    state: &'a RateLimitState,
    /// Label prefix shown before the countdown (default: `"Rate limited"`).
    label: String,
}

impl<'a> RateLimitBanner<'a> {
    /// Construct a banner tied to `state`.
    pub fn new(state: &'a RateLimitState) -> Self {
        Self {
            state,
            label: "Rate limited".to_string(),
        }
    }

    /// Override the prefix label.
    pub fn with_label(mut self, label: impl Into<String>) -> Self {
        self.label = label.into();
        self
    }

    /// Returns the height this widget will occupy: `1` when active, `0` when idle.
    pub fn height(&self) -> u16 {
        if self.state.is_active() {
            1
        } else {
            0
        }
    }
}

impl<'a> Widget for RateLimitBanner<'a> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        // Hidden when not rate-limited.
        let remaining = match self.state.retry_after {
            Some(d) => d,
            None => return,
        };

        if area.width < 4 || area.height < 1 {
            return;
        }

        let time_str = format_time_compact(remaining);
        let text = format!("⏳ {} — retry in {}", self.label, time_str);

        let line = Line::from(vec![Span::styled(
            text,
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        )]);
        Paragraph::new(line).render(area, buf);
    }
}

// ---------------------------------------------------------------------------
// 11. Unit Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_format_time_compact() {
        assert_eq!(format_time_compact(Duration::from_secs(0)), "00.0s");
        assert_eq!(format_time_compact(Duration::from_millis(450)), "00.5s");
        assert_eq!(format_time_compact(Duration::from_secs_f64(4.8)), "04.8s");
        assert_eq!(format_time_compact(Duration::from_secs_f64(14.2)), "14.2s");
        assert_eq!(format_time_compact(Duration::from_secs(84)), "1m 24s");
        assert_eq!(format_time_compact(Duration::from_secs(3665)), "1h 01m");
    }

    #[test]
    fn test_format_tokens_compact() {
        assert_eq!(format_tokens_compact(450), "450");
        assert_eq!(format_tokens_compact(1000), "1k");
        assert_eq!(format_tokens_compact(32500), "32.5k");
        assert_eq!(format_tokens_compact(1000000), "1M");
        assert_eq!(format_tokens_compact(2500000), "2.5M");
    }

    #[test]
    fn test_rate_limit_info_builder() {
        let info = RateLimitInfo::new(Duration::from_secs(10))
            .with_provider("Anthropic")
            .with_model("claude-3-7-sonnet")
            .with_kind(RateLimitKind::TokensPerMinute)
            .with_attempt(2, Some(5))
            .with_strategy(BackoffStrategy::Exponential)
            .with_limits(Some(500), Some(0), Some(40000), Some(0));

        assert_eq!(info.provider.as_deref(), Some("Anthropic"));
        assert_eq!(info.model.as_deref(), Some("claude-3-7-sonnet"));
        assert_eq!(info.kind, RateLimitKind::TokensPerMinute);
        assert_eq!(info.attempt, 2);
        assert_eq!(info.max_retries, Some(5));
        assert_eq!(info.format_attempt_badge(), "Attempt 2/5");
        assert_eq!(info.backoff_strategy, BackoffStrategy::Exponential);
        assert!(info.format_metrics_badge().is_some());
    }

    #[test]
    fn test_rate_limit_info_from_error_str() {
        let err1 = "HTTP 429: Rate limit exceeded. TPM limit reached for gpt-4o. Try again in 6.5s";
        let info1 = RateLimitInfo::from_error_str(err1, Duration::from_secs(10));
        assert_eq!(info1.kind, RateLimitKind::TokensPerMinute);
        assert_eq!(info1.provider.as_deref(), Some("OpenAI"));
        assert_eq!(info1.total_delay, Duration::from_secs_f64(6.5));

        let err2 = "Anthropic 529: Provider servers overloaded. Resets in 12 seconds.";
        let info2 = RateLimitInfo::from_error_str(err2, Duration::from_secs(5));
        assert_eq!(info2.kind, RateLimitKind::ServerOverload);
        assert_eq!(info2.provider.as_deref(), Some("Anthropic"));
        assert_eq!(info2.total_delay, Duration::from_secs(12));
    }

    #[test]
    fn test_countdown_progress_calculation() {
        let start = Instant::now();
        let total = Duration::from_secs(10);
        let mut info = RateLimitInfo::new(total);
        info.started_at = start;
        info.retry_at = start + total;

        // At start (0 elapsed)
        assert_eq!(info.progress_at(start), 0.0);
        assert_eq!(info.time_remaining_at(start), total);
        assert!(!info.is_ready_to_retry_at(start));

        // At 50% (5s elapsed)
        let mid = start + Duration::from_secs(5);
        assert!((info.progress_at(mid) - 0.5).abs() < 0.01);
        assert_eq!(info.time_remaining_at(mid), Duration::from_secs(5));
        assert!(!info.is_ready_to_retry_at(mid));

        // At 100% (10s elapsed)
        let end = start + total;
        assert_eq!(info.progress_at(end), 1.0);
        assert_eq!(info.time_remaining_at(end), Duration::from_secs(0));
        assert!(info.is_ready_to_retry_at(end));

        // Beyond 100% (12s elapsed)
        let past = start + Duration::from_secs(12);
        assert_eq!(info.progress_at(past), 1.0);
        assert_eq!(info.time_remaining_at(past), Duration::from_secs(0));
        assert!(info.is_ready_to_retry_at(past));
    }

    #[test]
    fn test_progress_bar_rendering_styles() {
        let smooth = ProgressBarStyle::Smooth.render_bar(0.5, 10);
        assert!(!smooth.is_empty());

        let blocks = ProgressBarStyle::Blocks.render_bar(0.6, 10);
        assert_eq!(blocks.chars().count(), 10);
        assert_eq!(blocks.chars().filter(|&c| c == '■').count(), 6);

        let ascii = ProgressBarStyle::Ascii.render_bar(0.5, 10);
        assert!(ascii.starts_with('['));
        assert!(ascii.ends_with(']'));

        let dotted = ProgressBarStyle::Dotted.render_bar(0.7, 10);
        assert_eq!(dotted.chars().count(), 10);

        let braille = ProgressBarStyle::Braille.render_bar(0.5, 10);
        assert_eq!(braille.chars().count(), 10);
    }

    #[test]
    fn test_render_rate_limit_banner_ansi() {
        let info = RateLimitInfo::new(Duration::from_secs(8))
            .with_provider("Anthropic")
            .with_model("claude-3-7-sonnet")
            .with_kind(RateLimitKind::RequestsPerMinute)
            .with_attempt(1, Some(3));

        let rendered = render_rate_limit_banner_ansi(&info, BannerBoxStyle::Rounded, 60);
        assert!(rendered.contains("Anthropic"));
        assert!(rendered.contains("claude-3-7-sonnet"));
        assert!(rendered.contains("RATE LIMIT NOTICE"));
        assert!(rendered.contains("RPM LIMIT"));
        assert!(rendered.contains("Attempt 1/3"));
    }

    #[test]
    fn test_render_rate_limit_compact_and_pill_ansi() {
        let info = RateLimitInfo::new(Duration::from_secs(5))
            .with_provider("OpenAI")
            .with_model("gpt-4o")
            .with_attempt(2, Some(4));

        let compact = render_rate_limit_compact_ansi(&info);
        assert!(compact.contains("RATE LIMITED"));
        assert!(compact.contains("OpenAI"));
        assert!(compact.contains("gpt-4o"));

        let pill = render_rate_limit_pill_ansi(&info);
        assert!(pill.contains("Rate Limit:"));
        assert!(pill.contains("Attempt 2/4"));
    }

    #[test]
    fn test_ratatui_widget_rendering() {
        let info = RateLimitInfo::new(Duration::from_secs(6))
            .with_provider("OpenRouter")
            .with_model("deepseek/deepseek-chat")
            .with_kind(RateLimitKind::TokensPerMinute)
            .with_attempt(1, Some(3));

        let mut buf = Buffer::empty(Rect::new(0, 0, 80, 8));
        let widget = RateLimitBannerWidget::new(&info);
        widget.render(Rect::new(0, 0, 80, 8), &mut buf);

        // Test rendering into small height
        let mut small_buf = Buffer::empty(Rect::new(0, 0, 80, 2));
        let small_widget = RateLimitBannerWidget::new(&info);
        small_widget.render(Rect::new(0, 0, 80, 2), &mut small_buf);

        // Test mini banner widget
        let mut mini_buf = Buffer::empty(Rect::new(0, 0, 60, 1));
        let mini_widget = RateLimitMiniBannerWidget::new(&info);
        mini_widget.render(Rect::new(0, 0, 60, 1), &mut mini_buf);

        // Test countdown bar widget
        let mut bar_buf = Buffer::empty(Rect::new(0, 0, 40, 1));
        let bar_widget = RateLimitCountdownBarWidget::new(&info);
        bar_widget.render(Rect::new(0, 0, 40, 1), &mut bar_buf);
    }

    #[test]
    fn test_rate_limit_tracker_lifecycle() {
        let mut tracker = RateLimitTracker::new();
        assert!(!tracker.is_active());
        assert_eq!(tracker.tick(), RateLimitTickOutcome::Idle);

        let start = Instant::now();
        let mut info = RateLimitInfo::new(Duration::from_millis(50));
        info.started_at = start;
        info.retry_at = start + Duration::from_millis(50);

        tracker.register(info);
        assert!(tracker.is_active());
        assert_eq!(tracker.total_count(), 1);

        // Wait a tiny bit and tick
        std::thread::sleep(Duration::from_millis(60));
        let outcome = tracker.tick();
        assert_eq!(outcome, RateLimitTickOutcome::ReadyToRetry);
        assert_eq!(tracker.active().unwrap().status, RateLimitStatus::Retrying);

        tracker.record_success();
        assert_eq!(tracker.active().unwrap().status, RateLimitStatus::Succeeded);
        assert!(!tracker.is_active());
    }
}
