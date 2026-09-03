//! Interactive LLM Provider Latency, TTFT, and Stream Throughput Benchmark Runner.
//!
//! Provides a high-polish, interactive benchmarking dashboard and live status display
//! for measuring and comparing LLM providers and models:
//! - **Real-time Live Metrics**:
//!   - **TTFT (Time To First Token)**: High-resolution timing until initial chunk delivery.
//!   - **Streaming Throughput (tokens/sec)**: Instantaneous and rolling token generation rate.
//!   - **Total Latency (RTT)**: End-to-end round trip duration.
//!   - **Chunk Jitter & Variance**: Inter-token arrival intervals and standard deviation.
//!   - **Live Preview Buffer**: Streaming text output preview with dynamic updates.
//! - **Interactive Multi-Tab TUI**:
//!   1. `[ 1. Overview ]`: Live execution cards, overall progress gauge, active target stream metrics,
//!      and target queue status.
//!   2. `[ 2. Results ]`: Sortable performance comparison table with color-coded TTFT, tok/s, latency,
//!      and qualitative rating badges.
//!   3. `[ 3. Charts ]`: Visual ASCII/Unicode comparative horizontal bar distribution graphs for TTFT
//!      and generation throughput.
//!   4. `[ 4. Inspector ]`: Granular round-by-round chunk timeline, interval variance, and full response inspector.
//!   5. `[ 5. Recommendations ]`: Automated winner analysis (🏆 Fastest TTFT, ⚡ Top Throughput, 🎯 Best Value),
//!      active provider comparison, and troubleshooting hints.
//! - **Interactive Controls**:
//!   - `Tab` / `Shift+Tab` / `1-5`: Switch views and tabs.
//!   - `Space` / `r`: Run / Re-run benchmark suite or selected provider.
//!   - `s`: Cycle sort column (TTFT, Throughput, Latency, Provider, Rating).
//!   - `p`: Toggle Parallel vs Sequential benchmark execution.
//!   - `m`: Cycle prompt presets (Ping, Haiku, Code, Reasoning, Stress Test).
//!   - `↑` / `↓` / `j` / `k`: Navigate provider list and inspector rounds.
//!   - `Enter`: Drill down into selected target in Inspector view.
//!   - `Esc` / `q`: Exit runner.
//! - **Dual-Engine Rendering**:
//!   - Full interactive Ratatui widget dashboard for TUI and alternate-screen viewports.
//!   - Pure ANSI live streaming status renderer for inline terminals and headless logs.

use std::cmp::Ordering;
use std::io::stdout;
use std::time::{Duration, Instant};

use crossterm::{
    cursor,
    event::{self, Event, KeyCode, KeyEventKind},
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{
    backend::CrosstermBackend,
    buffer::Buffer,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Gauge, Paragraph, Row, Table as RatatuiTable, Widget},
};
use serde::{Deserialize, Serialize};

use crate::config::Config;
use crate::provider::client::LlmClient;
use crate::provider::types::{Message, StreamChunk};
use crate::ui::bench_cmd::{
    discover_benchmark_targets, format_duration_compact, format_rankings_and_recommendation,
    format_tps_colored, format_troubleshooting_and_unconfigured, format_ttft_colored,
    BenchmarkOptions, BenchmarkRunResult, BenchmarkTarget, PerformanceRating,
    ProviderBenchmarkSummary, DEFAULT_BENCHMARK_PROMPT, DEFAULT_BENCHMARK_TIMEOUT_SECS,
    DEFAULT_PING_PROMPT,
};
use crate::ui::prompt::RawModeGuard;
use crate::ui::table::{get_terminal_width, ColumnAlign, Table};
use crate::ui::theme::Theme;
// ============================================================================
// 1. Core Data Models & Stream Phase
// ============================================================================

/// Current execution phase for a streaming benchmark target.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum StreamPhase {
    /// Target is queued and waiting to run.
    #[default]
    Idle,
    /// Establishing connection and sending request.
    Connecting,
    /// Connection open, waiting for first token (TTFT window).
    WaitingForFirstToken,
    /// Streaming chunks and tokens actively.
    Streaming,
    /// Benchmark completed successfully.
    Completed,
    /// Benchmark failed with an error.
    Failed,
    /// Target skipped (e.g. unconfigured or filtered out).
    Skipped,
}

impl StreamPhase {
    /// Returns human-readable label.
    pub fn label(&self) -> &'static str {
        match self {
            Self::Idle => "IDLE",
            Self::Connecting => "CONNECTING",
            Self::WaitingForFirstToken => "WAITING TTFT",
            Self::Streaming => "STREAMING",
            Self::Completed => "COMPLETED",
            Self::Failed => "FAILED",
            Self::Skipped => "SKIPPED",
        }
    }

    /// Returns styled ANSI badge.
    pub fn badge_ansi(&self, color: bool) -> String {
        if !color {
            return format!("[{}]", self.label());
        }
        match self {
            Self::Idle => "\x1b[2;37m[ IDLE ]\x1b[0m".to_string(),
            Self::Connecting => "\x1b[1;34m[ CONNECTING ]\x1b[0m".to_string(),
            Self::WaitingForFirstToken => "\x1b[1;33m[ WAITING TTFT ]\x1b[0m".to_string(),
            Self::Streaming => "\x1b[1;36m[ STREAMING ]\x1b[0m".to_string(),
            Self::Completed => "\x1b[1;32m[ COMPLETED ]\x1b[0m".to_string(),
            Self::Failed => "\x1b[1;31m[ FAILED ]\x1b[0m".to_string(),
            Self::Skipped => "\x1b[2;37m[ SKIPPED ]\x1b[0m".to_string(),
        }
    }

    /// Returns Ratatui color style.
    pub fn style(&self) -> Style {
        match self {
            Self::Idle => Style::default().fg(Color::DarkGray),
            Self::Connecting => Style::default()
                .fg(Color::Blue)
                .add_modifier(Modifier::BOLD),
            Self::WaitingForFirstToken => Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
            Self::Streaming => Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
            Self::Completed => Style::default()
                .fg(Color::Green)
                .add_modifier(Modifier::BOLD),
            Self::Failed => Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
            Self::Skipped => Style::default().fg(Color::DarkGray),
        }
    }

    /// Returns single-character status icon.
    pub fn icon(&self) -> &'static str {
        match self {
            Self::Idle => "○",
            Self::Connecting => "◐",
            Self::WaitingForFirstToken => "⏳",
            Self::Streaming => "⚡",
            Self::Completed => "✓",
            Self::Failed => "✗",
            Self::Skipped => "⊘",
        }
    }
}

/// A captured streaming chunk event.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LiveStreamChunk {
    /// Milliseconds elapsed from start of request.
    pub timestamp_ms: u64,
    /// Chunk sequence index.
    pub chunk_idx: usize,
    /// Byte length of chunk payload.
    pub byte_count: usize,
    /// Estimated or reported token count in chunk.
    pub token_count: usize,
    /// Text snippet received.
    pub text_snippet: String,
    /// Milliseconds elapsed since the previous chunk.
    pub interval_ms: u64,
}

/// Real-time live metrics tracked during stream generation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LiveStreamMetrics {
    /// Millisecond offset when connection initiated.
    pub start_timestamp_ms: u64,
    /// Time To First Token (TTFT) duration once observed.
    pub ttft: Option<Duration>,
    /// Elapsed stream generation time (after first token).
    pub stream_duration: Duration,
    /// Total round-trip latency.
    pub total_latency: Duration,
    /// Total tokens generated so far.
    pub total_tokens: usize,
    /// Total bytes received so far.
    pub total_bytes: usize,
    /// Total chunks received.
    pub chunk_count: usize,
    /// Instantaneous or rolling throughput (tokens / second).
    pub live_tokens_per_sec: f64,
    /// Average throughput across the generation window.
    pub average_tokens_per_sec: f64,
    /// Minimum chunk interval in milliseconds.
    pub min_chunk_interval_ms: u64,
    /// Maximum chunk interval in milliseconds.
    pub max_chunk_interval_ms: u64,
    /// Average chunk interval in milliseconds.
    pub avg_chunk_interval_ms: f64,
    /// Chunk jitter (standard deviation of arrival intervals) in milliseconds.
    pub jitter_ms: f64,
    /// Recent streaming text output preview buffer.
    pub preview_buffer: String,
    /// Recent chunk history for jitter analysis and sparkline rendering.
    pub recent_chunks: Vec<LiveStreamChunk>,
}

impl Default for LiveStreamMetrics {
    fn default() -> Self {
        Self {
            start_timestamp_ms: 0,
            ttft: None,
            stream_duration: Duration::ZERO,
            total_latency: Duration::ZERO,
            total_tokens: 0,
            total_bytes: 0,
            chunk_count: 0,
            live_tokens_per_sec: 0.0,
            average_tokens_per_sec: 0.0,
            min_chunk_interval_ms: 0,
            max_chunk_interval_ms: 0,
            avg_chunk_interval_ms: 0.0,
            jitter_ms: 0.0,
            preview_buffer: String::new(),
            recent_chunks: Vec::new(),
        }
    }
}

impl LiveStreamMetrics {
    /// Creates a new initialized metrics instance at the start of a run.
    pub fn new() -> Self {
        Self::default()
    }

    /// Records the arrival of the first token (TTFT).
    pub fn record_first_token(&mut self, ttft: Duration) {
        self.ttft = Some(ttft);
        self.total_latency = ttft;
    }

    /// Records a new streaming chunk and recomputes live rates and jitter.
    pub fn record_chunk(&mut self, chunk: LiveStreamChunk) {
        let interval = chunk.interval_ms;
        self.total_tokens += chunk.token_count;
        self.total_bytes += chunk.byte_count;
        self.chunk_count += 1;

        if !chunk.text_snippet.is_empty() {
            self.preview_buffer.push_str(&chunk.text_snippet);
            // Cap preview buffer to prevent unbounded allocation
            if self.preview_buffer.len() > 1024 {
                let keep_from = self.preview_buffer.len() - 1024;
                self.preview_buffer = self.preview_buffer[keep_from..].to_string();
            }
        }

        // Interval statistics
        if self.chunk_count == 1 {
            self.min_chunk_interval_ms = interval;
            self.max_chunk_interval_ms = interval;
            self.avg_chunk_interval_ms = interval as f64;
        } else {
            if interval < self.min_chunk_interval_ms {
                self.min_chunk_interval_ms = interval;
            }
            if interval > self.max_chunk_interval_ms {
                self.max_chunk_interval_ms = interval;
            }
            let n = self.chunk_count as f64;
            self.avg_chunk_interval_ms =
                ((self.avg_chunk_interval_ms * (n - 1.0)) + interval as f64) / n;
        }

        // Rolling throughput calculation
        if let Some(ttft) = self.ttft {
            let elapsed_stream = Duration::from_millis(chunk.timestamp_ms).saturating_sub(ttft);
            self.stream_duration = elapsed_stream;
            self.total_latency = Duration::from_millis(chunk.timestamp_ms);

            let stream_secs = elapsed_stream.as_secs_f64();
            if stream_secs > 0.05 {
                self.average_tokens_per_sec = self.total_tokens as f64 / stream_secs;
            }

            // Compute instantaneous tok/s over the last 5 chunks
            let window_chunks = self.recent_chunks.iter().rev().take(5).collect::<Vec<_>>();
            if !window_chunks.is_empty() {
                let window_tokens: usize =
                    window_chunks.iter().map(|c| c.token_count).sum::<usize>() + chunk.token_count;
                let window_ms: u64 =
                    window_chunks.iter().map(|c| c.interval_ms).sum::<u64>() + chunk.interval_ms;
                let window_secs = window_ms as f64 / 1000.0;
                if window_secs > 0.01 {
                    self.live_tokens_per_sec = window_tokens as f64 / window_secs;
                } else {
                    self.live_tokens_per_sec = self.average_tokens_per_sec;
                }
            } else {
                self.live_tokens_per_sec = self.average_tokens_per_sec;
            }
        }

        self.recent_chunks.push(chunk);
        if self.recent_chunks.len() > 64 {
            self.recent_chunks.remove(0);
        }

        // Compute jitter standard deviation
        self.compute_jitter();
    }

    /// Recomputes chunk interval jitter standard deviation.
    fn compute_jitter(&mut self) {
        if self.recent_chunks.len() < 2 {
            self.jitter_ms = 0.0;
            return;
        }
        let mean = self.avg_chunk_interval_ms;
        let variance_sum: f64 = self
            .recent_chunks
            .iter()
            .map(|c| {
                let diff = c.interval_ms as f64 - mean;
                diff * diff
            })
            .sum();
        self.jitter_ms = (variance_sum / (self.recent_chunks.len() - 1) as f64).sqrt();
    }
}

/// Benchmark prompt presets for interactive selection.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum BenchPromptPreset {
    Ping,
    Haiku,
    CodeSnippet,
    Reasoning,
    StressTest,
    Custom(String),
}

impl BenchPromptPreset {
    /// All standard prompt presets.
    pub const ALL: [BenchPromptPreset; 5] = [
        BenchPromptPreset::Ping,
        BenchPromptPreset::Haiku,
        BenchPromptPreset::CodeSnippet,
        BenchPromptPreset::Reasoning,
        BenchPromptPreset::StressTest,
    ];

    pub fn label(&self) -> &'static str {
        match self {
            Self::Ping => "1. Ping (Minimal TTFT)",
            Self::Haiku => "2. Haiku (Standard Latency)",
            Self::CodeSnippet => "3. Fast Code (Rust Fib)",
            Self::Reasoning => "4. Logic Riddle (Reasoning)",
            Self::StressTest => "5. Long Stream (200 Words)",
            Self::Custom(_) => "Custom Prompt",
        }
    }

    pub fn prompt_text(&self) -> &str {
        match self {
            Self::Ping => DEFAULT_PING_PROMPT,
            Self::Haiku => DEFAULT_BENCHMARK_PROMPT,
            Self::CodeSnippet => "Write a fast Rust function to calculate Fibonacci numbers using matrix exponentiation.",
            Self::Reasoning => "Solve this riddle: A farmer has 17 sheep, and all but 9 die. How many sheep are left? Explain in one sentence.",
            Self::StressTest => "Generate a detailed 200-word explanation of how B-trees differ from LSM-trees in modern database engines.",
            Self::Custom(s) => s.as_str(),
        }
    }

    pub fn max_tokens(&self) -> u32 {
        match self {
            Self::Ping => 16,
            Self::Haiku => 96,
            Self::CodeSnippet => 192,
            Self::Reasoning => 128,
            Self::StressTest => 384,
            Self::Custom(_) => 128,
        }
    }

    pub fn next(&self) -> Self {
        match self {
            Self::Ping => Self::Haiku,
            Self::Haiku => Self::CodeSnippet,
            Self::CodeSnippet => Self::Reasoning,
            Self::Reasoning => Self::StressTest,
            Self::StressTest => Self::Ping,
            Self::Custom(_) => Self::Ping,
        }
    }
}

// ============================================================================
// 2. Interactive Navigation & Sort Columns
// ============================================================================

/// Tabs in the interactive benchmark runner TUI.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
pub enum BenchTab {
    #[default]
    Overview,
    Results,
    Charts,
    Inspector,
    Recommendations,
}

impl BenchTab {
    pub const ALL: [BenchTab; 5] = [
        BenchTab::Overview,
        BenchTab::Results,
        BenchTab::Charts,
        BenchTab::Inspector,
        BenchTab::Recommendations,
    ];

    pub fn label(&self) -> &'static str {
        match self {
            Self::Overview => "1. Live Overview",
            Self::Results => "2. Results Table",
            Self::Charts => "3. Visual Charts",
            Self::Inspector => "4. Round Inspector",
            Self::Recommendations => "5. Recommendations",
        }
    }

    pub fn next(&self) -> Self {
        match self {
            Self::Overview => Self::Results,
            Self::Results => Self::Charts,
            Self::Charts => Self::Inspector,
            Self::Inspector => Self::Recommendations,
            Self::Recommendations => Self::Overview,
        }
    }

    pub fn prev(&self) -> Self {
        match self {
            Self::Overview => Self::Recommendations,
            Self::Results => Self::Overview,
            Self::Charts => Self::Results,
            Self::Inspector => Self::Charts,
            Self::Recommendations => Self::Inspector,
        }
    }
}

/// Column selection for sorting results table.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
pub enum BenchSortColumn {
    Provider,
    #[default]
    TTFT,
    Throughput,
    Latency,
    Rating,
    Status,
}

impl BenchSortColumn {
    pub const ALL: [BenchSortColumn; 6] = [
        BenchSortColumn::TTFT,
        BenchSortColumn::Throughput,
        BenchSortColumn::Latency,
        BenchSortColumn::Rating,
        BenchSortColumn::Provider,
        BenchSortColumn::Status,
    ];

    pub fn label(&self) -> &'static str {
        match self {
            Self::Provider => "Provider",
            Self::TTFT => "TTFT",
            Self::Throughput => "Speed (tok/s)",
            Self::Latency => "Total Latency",
            Self::Rating => "Rating",
            Self::Status => "Status",
        }
    }

    pub fn next(&self) -> Self {
        match self {
            Self::TTFT => Self::Throughput,
            Self::Throughput => Self::Latency,
            Self::Latency => Self::Rating,
            Self::Rating => Self::Provider,
            Self::Provider => Self::Status,
            Self::Status => Self::TTFT,
        }
    }
}

// ============================================================================
// 3. Target & Runner State
// ============================================================================

/// Complete live state of an individual target provider.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BenchTargetState {
    /// Benchmark target configuration.
    pub target: BenchmarkTarget,
    /// Current execution phase.
    pub phase: StreamPhase,
    /// Active test round index (1-based).
    pub current_round: usize,
    /// Total planned rounds.
    pub total_rounds: usize,
    /// Live stream metrics during active execution.
    pub live_metrics: Option<LiveStreamMetrics>,
    /// Completed round results.
    pub completed_rounds: Vec<BenchmarkRunResult>,
    /// Aggregated summary across completed rounds.
    pub summary: Option<ProviderBenchmarkSummary>,
    /// Error message if target failed.
    pub error_message: Option<String>,
    /// Whether this target is selected for benchmark execution.
    pub is_selected: bool,
    /// Whether this target matches the active session provider.
    pub is_active_provider: bool,
}

impl BenchTargetState {
    pub fn new(target: BenchmarkTarget, rounds: usize, is_active: bool) -> Self {
        let is_configured = target.is_configured;
        Self {
            target,
            phase: if is_configured {
                StreamPhase::Idle
            } else {
                StreamPhase::Skipped
            },
            current_round: 0,
            total_rounds: rounds,
            live_metrics: None,
            completed_rounds: Vec::new(),
            summary: None,
            error_message: None,
            is_selected: is_configured,
            is_active_provider: is_active,
        }
    }

    /// Resets execution state for a fresh benchmark run.
    pub fn reset_for_run(&mut self, rounds: usize) {
        self.total_rounds = rounds;
        self.current_round = 0;
        self.completed_rounds.clear();
        self.summary = None;
        self.error_message = None;
        self.live_metrics = None;
        if self.target.is_configured && self.is_selected {
            self.phase = StreamPhase::Idle;
        } else {
            self.phase = StreamPhase::Skipped;
        }
    }

    /// Records completion of a single round and updates summary.
    pub fn record_round_result(&mut self, result: BenchmarkRunResult) {
        if !result.success {
            self.error_message = result.error_message.clone();
            self.phase = StreamPhase::Failed;
        }
        self.completed_rounds.push(result);
        self.summary = Some(ProviderBenchmarkSummary::from_runs(
            &self.target.provider,
            &self.target.model,
            &self.completed_rounds,
        ));
        if self.completed_rounds.len() >= self.total_rounds {
            if self
                .summary
                .as_ref()
                .map(|s| s.success_rate() > 0.0)
                .unwrap_or(false)
            {
                self.phase = StreamPhase::Completed;
            } else {
                self.phase = StreamPhase::Failed;
            }
        }
    }
}

/// Global interactive benchmark runner state.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BenchRunnerState {
    /// List of candidate benchmark targets.
    pub targets: Vec<BenchTargetState>,
    /// Currently active navigation tab.
    pub active_tab: BenchTab,
    /// Cursor index in target list.
    pub selected_target_idx: usize,
    /// Cursor index in round inspector.
    pub selected_round_idx: usize,
    /// Active sort column for results.
    pub sort_column: BenchSortColumn,
    /// Whether sorting is descending.
    pub sort_descending: bool,
    /// Whether benchmark suite is currently executing.
    pub is_running: bool,
    /// Whether parallel execution mode is enabled.
    pub is_parallel: bool,
    /// Active prompt preset.
    pub prompt_preset: BenchPromptPreset,
    /// Custom prompt text override.
    pub custom_prompt: String,
    /// Number of measurement rounds.
    pub rounds: usize,
    /// Max tokens per generation.
    pub max_tokens: u32,
    /// Request timeout in seconds.
    pub timeout_secs: u64,
    /// Total elapsed benchmark duration.
    pub total_elapsed: Duration,
    /// Index of currently running target.
    pub active_running_target_idx: Option<usize>,
    /// Status or diagnostic message banner.
    pub status_message: String,
    /// Spinner animation tick counter.
    pub spinner_tick: usize,
    /// Scroll offset for long lists.
    pub scroll_offset: usize,
    /// Active provider name in session.
    pub active_provider: String,
}

impl BenchRunnerState {
    /// Creates a new runner state initialized from discovered targets.
    pub fn new(
        targets: Vec<BenchmarkTarget>,
        active_provider: &str,
        rounds: usize,
        preset: BenchPromptPreset,
    ) -> Self {
        let max_tokens = preset.max_tokens();
        let target_states = targets
            .into_iter()
            .map(|t| {
                let is_active = t.provider.eq_ignore_ascii_case(active_provider);
                BenchTargetState::new(t, rounds, is_active)
            })
            .collect();

        Self {
            targets: target_states,
            active_tab: BenchTab::Overview,
            selected_target_idx: 0,
            selected_round_idx: 0,
            sort_column: BenchSortColumn::TTFT,
            sort_descending: false,
            is_running: false,
            is_parallel: false,
            prompt_preset: preset,
            custom_prompt: String::new(),
            rounds,
            max_tokens,
            timeout_secs: DEFAULT_BENCHMARK_TIMEOUT_SECS,
            total_elapsed: Duration::ZERO,
            active_running_target_idx: None,
            status_message: "Ready to benchmark. Press [Space] or [r] to start.".to_string(),
            spinner_tick: 0,
            scroll_offset: 0,
            active_provider: active_provider.to_string(),
        }
    }

    /// Returns the active effective benchmark prompt text.
    pub fn effective_prompt(&self) -> &str {
        if !self.custom_prompt.trim().is_empty() {
            &self.custom_prompt
        } else {
            self.prompt_preset.prompt_text()
        }
    }

    /// Selects the next target item down.
    pub fn select_next(&mut self) {
        if !self.targets.is_empty() {
            self.selected_target_idx = (self.selected_target_idx + 1) % self.targets.len();
        }
    }

    /// Selects the previous target item up.
    pub fn select_prev(&mut self) {
        if !self.targets.is_empty() {
            if self.selected_target_idx == 0 {
                self.selected_target_idx = self.targets.len() - 1;
            } else {
                self.selected_target_idx -= 1;
            }
        }
    }

    /// Toggles selection of current target for execution.
    pub fn toggle_selected_target(&mut self) {
        if let Some(target) = self.targets.get_mut(self.selected_target_idx) {
            target.is_selected = !target.is_selected;
        }
    }

    /// Cycles active sort column.
    pub fn cycle_sort_column(&mut self) {
        self.sort_column = self.sort_column.next();
    }

    /// Toggles sort order between ascending and descending.
    pub fn toggle_sort_direction(&mut self) {
        self.sort_descending = !self.sort_descending;
    }

    /// Cycles prompt preset.
    pub fn cycle_prompt_preset(&mut self) {
        self.prompt_preset = self.prompt_preset.next();
        self.max_tokens = self.prompt_preset.max_tokens();
        self.status_message = format!("Switched prompt to: {}", self.prompt_preset.label());
    }

    /// Toggles parallel execution mode.
    pub fn toggle_parallel(&mut self) {
        self.is_parallel = !self.is_parallel;
        self.status_message = if self.is_parallel {
            "Enabled Parallel benchmarking mode.".to_string()
        } else {
            "Enabled Sequential benchmarking mode.".to_string()
        };
    }

    /// Returns completed summaries across all targets.
    pub fn get_completed_summaries(&self) -> Vec<ProviderBenchmarkSummary> {
        self.targets
            .iter()
            .filter_map(|t| t.summary.clone())
            .collect()
    }

    /// Returns sorted indices according to active sort column and direction.
    pub fn sorted_target_indices(&self) -> Vec<usize> {
        let mut indices: Vec<usize> = (0..self.targets.len()).collect();
        indices.sort_by(|&a, &b| {
            let state_a = &self.targets[a];
            let state_b = &self.targets[b];

            let ord = match self.sort_column {
                BenchSortColumn::Provider => state_a.target.provider.cmp(&state_b.target.provider),
                BenchSortColumn::TTFT => {
                    let ttft_a = state_a
                        .summary
                        .as_ref()
                        .and_then(|s| s.avg_ttft)
                        .unwrap_or(Duration::from_secs(999));
                    let ttft_b = state_b
                        .summary
                        .as_ref()
                        .and_then(|s| s.avg_ttft)
                        .unwrap_or(Duration::from_secs(999));
                    ttft_a.cmp(&ttft_b)
                }
                BenchSortColumn::Throughput => {
                    let tps_a = state_a
                        .summary
                        .as_ref()
                        .map(|s| s.avg_tokens_per_second)
                        .unwrap_or(0.0);
                    let tps_b = state_b
                        .summary
                        .as_ref()
                        .map(|s| s.avg_tokens_per_second)
                        .unwrap_or(0.0);
                    tps_b.partial_cmp(&tps_a).unwrap_or(Ordering::Equal)
                }
                BenchSortColumn::Latency => {
                    let lat_a = state_a
                        .summary
                        .as_ref()
                        .map(|s| s.avg_latency)
                        .unwrap_or(Duration::from_secs(999));
                    let lat_b = state_b
                        .summary
                        .as_ref()
                        .map(|s| s.avg_latency)
                        .unwrap_or(Duration::from_secs(999));
                    lat_a.cmp(&lat_b)
                }
                BenchSortColumn::Rating => {
                    let rate_a = state_a
                        .summary
                        .as_ref()
                        .map(|s| s.rating)
                        .unwrap_or(PerformanceRating::Error);
                    let rate_b = state_b
                        .summary
                        .as_ref()
                        .map(|s| s.rating)
                        .unwrap_or(PerformanceRating::Error);
                    rate_a.cmp(&rate_b)
                }
                BenchSortColumn::Status => state_a.phase.label().cmp(state_b.phase.label()),
            };

            if self.sort_descending {
                ord.reverse()
            } else {
                ord
            }
        });
        indices
    }

    /// Progress completion percentage across selected targets (0.0 - 1.0).
    pub fn progress_fraction(&self) -> f64 {
        let total_planned_rounds: usize = self
            .targets
            .iter()
            .filter(|t| t.is_selected)
            .map(|t| t.total_rounds)
            .sum();
        if total_planned_rounds == 0 {
            return 1.0;
        }
        let completed_rounds: usize = self
            .targets
            .iter()
            .filter(|t| t.is_selected)
            .map(|t| t.completed_rounds.len())
            .sum();
        (completed_rounds as f64 / total_planned_rounds as f64).clamp(0.0, 1.0)
    }
}

// ============================================================================
// 4. Live Streaming Measurement Engine
// ============================================================================

/// Callback event emitted during live stream execution.
#[derive(Debug, Clone)]
pub enum LiveStreamEvent {
    PhaseChanged(StreamPhase),
    FirstToken { ttft: Duration },
    ChunkReceived(LiveStreamChunk),
    RoundDone(BenchmarkRunResult),
}

/// Measures live latency, TTFT, and throughput for a single provider request with streaming callbacks.
pub async fn measure_streaming_provider<F>(
    client: &LlmClient,
    target: &BenchmarkTarget,
    prompt: &str,
    max_tokens: u32,
    temperature: Option<f32>,
    timeout_secs: u64,
    round: usize,
    mut on_event: F,
) -> BenchmarkRunResult
where
    F: FnMut(LiveStreamEvent),
{
    let start_instant = Instant::now();
    on_event(LiveStreamEvent::PhaseChanged(StreamPhase::Connecting));

    if !target.is_configured {
        let res = BenchmarkRunResult {
            provider: target.provider.clone(),
            model: target.model.clone(),
            round,
            ttft: Duration::ZERO,
            generation_duration: Duration::ZERO,
            total_latency: Duration::ZERO,
            tokens_generated: 0,
            tokens_per_second: 0.0,
            success: false,
            error_message: Some(
                target
                    .setup_hint
                    .clone()
                    .unwrap_or_else(|| "Provider unconfigured (missing API key)".to_string()),
            ),
            response_preview: String::new(),
            timestamp_unix: current_timestamp(),
        };
        on_event(LiveStreamEvent::PhaseChanged(StreamPhase::Skipped));
        on_event(LiveStreamEvent::RoundDone(res.clone()));
        return res;
    }

    let messages = vec![Message::user(prompt.to_string())];

    on_event(LiveStreamEvent::PhaseChanged(
        StreamPhase::WaitingForFirstToken,
    ));

    let stream_fut = async {
        let mut stream = client
            .stream_chat_with(
                &target.provider,
                &target.model,
                temperature,
                Some(max_tokens),
                target.api_key.as_deref(),
                &target.base_url,
                &messages,
                &[],
            )
            .await?;
        let mut first_token_time: Option<Instant> = None;
        let mut ttft = Duration::ZERO;
        let mut generated_text = String::new();
        let mut chunk_count: usize = 0;
        let mut last_chunk_instant = Instant::now();

        while let Some(chunk) = stream.recv().await {
            let now = Instant::now();
            if first_token_time.is_none() {
                let recorded_ttft = now.duration_since(start_instant);
                first_token_time = Some(now);
                ttft = recorded_ttft;
                on_event(LiveStreamEvent::FirstToken {
                    ttft: recorded_ttft,
                });
                on_event(LiveStreamEvent::PhaseChanged(StreamPhase::Streaming));
            }

            match chunk {
                StreamChunk::ContentDelta(delta) => {
                    if !delta.is_empty() {
                        let interval_ms = now.duration_since(last_chunk_instant).as_millis() as u64;
                        last_chunk_instant = now;
                        chunk_count += 1;

                        // Approximate token count: ~4 chars per token, minimum 1 token per chunk
                        let token_est = (delta.len() / 4).max(1);
                        let stream_chunk = LiveStreamChunk {
                            timestamp_ms: now.duration_since(start_instant).as_millis() as u64,
                            chunk_idx: chunk_count,
                            byte_count: delta.len(),
                            token_count: token_est,
                            text_snippet: delta.clone(),
                            interval_ms,
                        };

                        generated_text.push_str(&delta);
                        on_event(LiveStreamEvent::ChunkReceived(stream_chunk));
                    }
                }
                StreamChunk::ThinkingDelta(_) | StreamChunk::ToolCallDelta { .. } => {}
                StreamChunk::Error(err) => {
                    return Err(anyhow::anyhow!("Stream error: {}", err));
                }
                StreamChunk::Done { .. } => {}
            }
        }

        let end_instant = Instant::now();
        let total_latency = end_instant.duration_since(start_instant);
        let generation_duration = first_token_time
            .map(|ft| end_instant.duration_since(ft))
            .unwrap_or(Duration::ZERO);

        let final_tokens = (generated_text.len() / 4).max(chunk_count).max(1);
        let gen_secs = generation_duration.as_secs_f64();
        let tokens_per_second = if gen_secs > 0.02 {
            final_tokens as f64 / gen_secs
        } else {
            0.0
        };

        Ok(BenchmarkRunResult {
            provider: target.provider.clone(),
            model: target.model.clone(),
            round,
            ttft,
            generation_duration,
            total_latency,
            tokens_generated: final_tokens,
            tokens_per_second,
            success: true,
            error_message: None,
            response_preview: sanitize_preview_text(&generated_text, 120),
            timestamp_unix: current_timestamp(),
        })
    };

    let result = match tokio::time::timeout(Duration::from_secs(timeout_secs), stream_fut).await {
        Ok(Ok(res)) => {
            on_event(LiveStreamEvent::PhaseChanged(StreamPhase::Completed));
            res
        }
        Ok(Err(e)) => {
            on_event(LiveStreamEvent::PhaseChanged(StreamPhase::Failed));
            BenchmarkRunResult {
                provider: target.provider.clone(),
                model: target.model.clone(),
                round,
                ttft: Duration::ZERO,
                generation_duration: Duration::ZERO,
                total_latency: start_instant.elapsed(),
                tokens_generated: 0,
                tokens_per_second: 0.0,
                success: false,
                error_message: Some(e.to_string()),
                response_preview: String::new(),
                timestamp_unix: current_timestamp(),
            }
        }
        Err(_) => {
            on_event(LiveStreamEvent::PhaseChanged(StreamPhase::Failed));
            BenchmarkRunResult {
                provider: target.provider.clone(),
                model: target.model.clone(),
                round,
                ttft: Duration::ZERO,
                generation_duration: Duration::ZERO,
                total_latency: Duration::from_secs(timeout_secs),
                tokens_generated: 0,
                tokens_per_second: 0.0,
                success: false,
                error_message: Some(format!("Request timed out after {timeout_secs}s")),
                response_preview: String::new(),
                timestamp_unix: current_timestamp(),
            }
        }
    };

    on_event(LiveStreamEvent::RoundDone(result.clone()));
    result
}

// ============================================================================
// 5. ANSI Live Terminal Status & Visual Formatter
// ============================================================================

/// Renders a dynamic live terminal status banner and progress block during execution.
pub fn render_live_status_ansi(state: &BenchRunnerState, color: bool) -> String {
    let mut out = String::with_capacity(1024);
    let term_width = get_terminal_width().max(60);

    // Header
    if color {
        out.push_str("\x1b[1;36m✦ LLM Provider Benchmark Runner\x1b[0m \x1b[2;37m(TTFT & Stream tok/s)\x1b[0m\n");
    } else {
        out.push_str("✦ LLM Provider Benchmark Runner (TTFT & Stream tok/s)\n");
    }

    // Config line
    let prompt_name = state.prompt_preset.label();
    let mode_str = if state.is_parallel {
        "Parallel"
    } else {
        "Sequential"
    };
    if color {
        out.push_str(&format!(
            "\x1b[2;37mPreset: \x1b[0m\x1b[1;37m{}\x1b[0m \x1b[2;37m| Mode: \x1b[0m\x1b[1;37m{}\x1b[0m \x1b[2;37m| Rounds: \x1b[0m\x1b[1;37m{}\x1b[0m \x1b[2;37m| Max Tokens: \x1b[0m\x1b[1;37m{}\x1b[0m\n",
            prompt_name, mode_str, state.rounds, state.max_tokens
        ));
    } else {
        out.push_str(&format!(
            "Preset: {} | Mode: {} | Rounds: {} | Max Tokens: {}\n",
            prompt_name, mode_str, state.rounds, state.max_tokens
        ));
    }

    // Progress Bar
    let pct = state.progress_fraction();
    let bar_width = (term_width.saturating_sub(24)).clamp(15, 40);
    let filled = (pct * bar_width as f64).round() as usize;
    let unfilled = bar_width.saturating_sub(filled);

    let bar_str = format!(
        "[{}{}] {:.0}%",
        "█".repeat(filled),
        "░".repeat(unfilled),
        pct * 100.0
    );
    if color {
        out.push_str(&format!("\x1b[1;32m{}\x1b[0m\n\n", bar_str));
    } else {
        out.push_str(&format!("{}\n\n", bar_str));
    }

    // Active Target Card
    if let Some(active_idx) = state.active_running_target_idx {
        if let Some(target_state) = state.targets.get(active_idx) {
            let phase_badge = target_state.phase.badge_ansi(color);
            let round_str = format!(
                "Round {}/{}",
                target_state.current_round.max(1),
                target_state.total_rounds
            );

            if color {
                out.push_str(&format!(
                    "  \x1b[1;37m▶ Provider: \x1b[1;36m{}\x1b[0m \x1b[2;37m({})\x1b[0m  {}  \x1b[2;37m[{}]\x1b[0m\n",
                    target_state.target.provider, target_state.target.model, phase_badge, round_str
                ));
            } else {
                out.push_str(&format!(
                    "  ▶ Provider: {} ({})  {}  [{}]\n",
                    target_state.target.provider, target_state.target.model, phase_badge, round_str
                ));
            }

            if let Some(metrics) = &target_state.live_metrics {
                let ttft_str = metrics
                    .ttft
                    .map(|d| format_ttft_colored(d, color))
                    .unwrap_or_else(|| {
                        if color {
                            "\x1b[2;37mmeasuring...\x1b[0m".to_string()
                        } else {
                            "measuring...".to_string()
                        }
                    });

                let tps_str = format_tps_colored(metrics.live_tokens_per_sec, color);

                if color {
                    out.push_str(&format!(
                        "    \x1b[2;37mTTFT:\x1b[0m {}  \x1b[2;37m| Stream Speed:\x1b[0m {}  \x1b[2;37m| Tokens:\x1b[0m \x1b[1;37m{}\x1b[0m  \x1b[2;37m| Jitter:\x1b[0m \x1b[1;37m{:.1}ms\x1b[0m\n",
                        ttft_str, tps_str, metrics.total_tokens, metrics.jitter_ms
                    ));
                } else {
                    out.push_str(&format!(
                        "    TTFT: {}  | Stream Speed: {}  | Tokens: {}  | Jitter: {:.1}ms\n",
                        ttft_str, tps_str, metrics.total_tokens, metrics.jitter_ms
                    ));
                }

                if !metrics.preview_buffer.is_empty() {
                    let preview = sanitize_preview_text(&metrics.preview_buffer, 80);
                    if color {
                        out.push_str(&format!(
                            "    \x1b[2;37mStream Preview: \x1b[0;32m\"{}\"\x1b[0m\n",
                            preview
                        ));
                    } else {
                        out.push_str(&format!("    Stream Preview: \"{}\"\n", preview));
                    }
                }
            }
            out.push('\n');
        }
    }

    // Queue status list
    out.push_str("  Queue:\n");
    for (i, t) in state.targets.iter().enumerate() {
        let is_current = state.active_running_target_idx == Some(i);
        let pointer = if is_current { "➔ " } else { "  " };
        let icon = t.phase.icon();
        let name = &t.target.provider;
        let model = &t.target.model;

        let status_desc = match &t.summary {
            Some(s) if s.success_rate() > 0.0 => {
                let ttft_str = s.avg_ttft.map(format_duration_compact).unwrap_or_default();
                format!(
                    "TTFT: {}, {:.1} tok/s ({})",
                    ttft_str,
                    s.avg_tokens_per_second,
                    s.rating.badge_text()
                )
            }
            Some(_) => "Failed".to_string(),
            None => match t.phase {
                StreamPhase::Idle => "Queued".to_string(),
                StreamPhase::Connecting => "Connecting...".to_string(),
                StreamPhase::WaitingForFirstToken => "Waiting for first token...".to_string(),
                StreamPhase::Streaming => "Streaming response...".to_string(),
                StreamPhase::Skipped => "Skipped (unconfigured)".to_string(),
                _ => t.phase.label().to_string(),
            },
        };

        if color {
            let color_code = match t.phase {
                StreamPhase::Completed => "\x1b[1;32m",
                StreamPhase::Streaming => "\x1b[1;36m",
                StreamPhase::WaitingForFirstToken => "\x1b[1;33m",
                StreamPhase::Failed => "\x1b[1;31m",
                _ => "\x1b[2;37m",
            };
            out.push_str(&format!(
                "  {}{}{} {} ({})\x1b[0m - \x1b[2;37m{}\x1b[0m\n",
                pointer, color_code, icon, name, model, status_desc
            ));
        } else {
            out.push_str(&format!(
                "  {}{} {} ({}) - {}\n",
                pointer, icon, name, model, status_desc
            ));
        }
    }

    out
}

/// Renders ASCII/Unicode comparative horizontal bar distribution graphs for TTFT and Throughput.
pub fn render_charts_ansi(summaries: &[ProviderBenchmarkSummary], color: bool) -> String {
    let mut out = String::with_capacity(1024);
    if summaries.is_empty() {
        return "No benchmark data available to render charts.\n".to_string();
    }

    let successful: Vec<&ProviderBenchmarkSummary> = summaries
        .iter()
        .filter(|s| s.success_rate() > 0.0)
        .collect();
    if successful.is_empty() {
        return "All benchmark targets failed.\n".to_string();
    }

    // 1. TTFT Comparison Bar Chart (Lower is faster)
    if color {
        out.push_str("\n\x1b[1;36m✦ Time to First Token (TTFT) Comparison\x1b[0m \x1b[2;37m(Lower is faster)\x1b[0m\n");
    } else {
        out.push_str("\n✦ Time to First Token (TTFT) Comparison (Lower is faster)\n");
    }

    let min_ttft_ms = successful
        .iter()
        .filter_map(|s| s.avg_ttft.map(|d| d.as_millis() as f64))
        .fold(f64::INFINITY, f64::min);
    let max_ttft_ms = successful
        .iter()
        .filter_map(|s| s.avg_ttft.map(|d| d.as_millis() as f64))
        .fold(0.0, f64::max)
        .max(10.0);

    let max_label_len = successful
        .iter()
        .map(|s| s.provider.len())
        .max()
        .unwrap_or(8)
        .max(8);

    for s in &successful {
        let ttft_ms = s.avg_ttft.map(|d| d.as_millis() as f64).unwrap_or(0.0);
        let bar_len = ((ttft_ms / max_ttft_ms) * 32.0).round() as usize;
        let bar_str = "█".repeat(bar_len.max(1));
        let duration_str = s.avg_ttft.map(format_duration_compact).unwrap_or_default();
        let is_fastest = (ttft_ms - min_ttft_ms).abs() < 1.0;

        let badge = if is_fastest { " 🏆 FASTEST" } else { "" };

        if color {
            let bar_color = if ttft_ms < 300.0 {
                "\x1b[1;32m"
            } else if ttft_ms < 800.0 {
                "\x1b[1;33m"
            } else {
                "\x1b[1;31m"
            };
            out.push_str(&format!(
                "  {:>width$} │ {}{}\x1b[0m {:<6}{}\n",
                s.provider,
                bar_color,
                bar_str,
                duration_str,
                badge,
                width = max_label_len
            ));
        } else {
            out.push_str(&format!(
                "  {:>width$} │ {} {:<6}{}\n",
                s.provider,
                bar_str,
                duration_str,
                badge,
                width = max_label_len
            ));
        }
    }

    // 2. Throughput Comparison Bar Chart (Higher is faster)
    if color {
        out.push_str("\n\x1b[1;36m✦ Generation Throughput Comparison\x1b[0m \x1b[2;37m(Higher is faster, tok/s)\x1b[0m\n");
    } else {
        out.push_str("\n✦ Generation Throughput Comparison (Higher is faster, tok/s)\n");
    }

    let max_tps = successful
        .iter()
        .map(|s| s.avg_tokens_per_second)
        .fold(0.0, f64::max)
        .max(1.0);

    for s in &successful {
        let tps = s.avg_tokens_per_second;
        let bar_len = ((tps / max_tps) * 32.0).round() as usize;
        let bar_str = "█".repeat(bar_len.max(1));
        let is_top = (tps - max_tps).abs() < 0.1;
        let badge = if is_top { " ⚡ TOP SPEED" } else { "" };

        if color {
            let bar_color = if tps >= 80.0 {
                "\x1b[1;36m"
            } else if tps >= 40.0 {
                "\x1b[1;32m"
            } else if tps >= 15.0 {
                "\x1b[1;33m"
            } else {
                "\x1b[1;31m"
            };
            out.push_str(&format!(
                "  {:>width$} │ {}{}\x1b[0m {:>5.1} tok/s{}\n",
                s.provider,
                bar_color,
                bar_str,
                tps,
                badge,
                width = max_label_len
            ));
        } else {
            out.push_str(&format!(
                "  {:>width$} │ {} {:>5.1} tok/s{}\n",
                s.provider,
                bar_str,
                tps,
                badge,
                width = max_label_len
            ));
        }
    }

    out
}

/// Renders detailed round inspector breakdown as an ANSI string.
pub fn render_inspector_ansi(target: &BenchTargetState, color: bool) -> String {
    let mut out = String::with_capacity(1024);
    if color {
        out.push_str(&format!(
            "\x1b[1;36m✦ Target Drilldown:\x1b[0m \x1b[1;37m{}\x1b[0m \x1b[2;37m(Model: {})\x1b[0m\n",
            target.target.provider, target.target.model
        ));
    } else {
        out.push_str(&format!(
            "✦ Target Drilldown: {} (Model: {})\n",
            target.target.provider, target.target.model
        ));
    }

    if target.completed_rounds.is_empty() {
        out.push_str("  No completed rounds for this provider yet.\n");
        return out;
    }

    let mut tbl = Table::new()
        .with_headers([
            "Round",
            "TTFT",
            "Stream Speed",
            "Latency",
            "Tokens",
            "Status",
        ])
        .with_alignments([
            ColumnAlign::Right,
            ColumnAlign::Right,
            ColumnAlign::Right,
            ColumnAlign::Right,
            ColumnAlign::Right,
            ColumnAlign::Left,
        ])
        .with_terminal_width(get_terminal_width());

    for r in &target.completed_rounds {
        let ttft_str = format_duration_compact(r.ttft);
        let tps_str = format!("{:.1} tok/s", r.tokens_per_second);
        let lat_str = format_duration_compact(r.total_latency);
        let tok_str = r.tokens_generated.to_string();
        let status_str = if r.success { "OK" } else { "Failed" };

        tbl.add_row(vec![
            r.round.to_string(),
            ttft_str,
            tps_str,
            lat_str,
            tok_str,
            status_str.to_string(),
        ]);
    }

    out.push_str(&tbl.render());
    out.push('\n');

    if let Some(metrics) = &target.live_metrics {
        if color {
            out.push_str(&format!(
                "  \x1b[2;37mChunk Stats: Total chunks: \x1b[1;37m{}\x1b[0m \x1b[2;37m| Min interval: \x1b[1;37m{}ms\x1b[0m \x1b[2;37m| Max interval: \x1b[1;37m{}ms\x1b[0m \x1b[2;37m| Avg: \x1b[1;37m{:.1}ms\x1b[0m \x1b[2;37m| Jitter: \x1b[1;37m{:.1}ms\x1b[0m\n",
                metrics.chunk_count, metrics.min_chunk_interval_ms, metrics.max_chunk_interval_ms, metrics.avg_chunk_interval_ms, metrics.jitter_ms
            ));
        } else {
            out.push_str(&format!(
                "  Chunk Stats: Total chunks: {} | Min interval: {}ms | Max interval: {}ms | Avg: {:.1}ms | Jitter: {:.1}ms\n",
                metrics.chunk_count, metrics.min_chunk_interval_ms, metrics.max_chunk_interval_ms, metrics.avg_chunk_interval_ms, metrics.jitter_ms
            ));
        }
    }

    if let Some(last_run) = target.completed_rounds.last() {
        if !last_run.response_preview.is_empty() {
            if color {
                out.push_str(&format!(
                    "  \x1b[2;37mGenerated Sample:\x1b[0m \x1b[0;32m\"{}\"\x1b[0m\n",
                    last_run.response_preview
                ));
            } else {
                out.push_str(&format!(
                    "  Generated Sample: \"{}\"\n",
                    last_run.response_preview
                ));
            }
        }
    }

    out
}

// ============================================================================
// 6. Interactive Ratatui Widgets & Dashboard
// ============================================================================

/// Full interactive Ratatui dashboard widget for benchmark runner.
pub struct BenchRunnerWidget<'a> {
    pub state: &'a BenchRunnerState,
    pub theme: Theme,
}

impl<'a> BenchRunnerWidget<'a> {
    pub fn new(state: &'a BenchRunnerState, theme: Theme) -> Self {
        Self { state, theme }
    }
}

impl<'a> Widget for BenchRunnerWidget<'a> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        if area.width < 20 || area.height < 6 {
            return;
        }

        // Layout: Header, Tab bar, Main body, Status bar, Key hints footer
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(2), // Header title
                Constraint::Length(2), // Tab selector bar
                Constraint::Min(6),    // Active tab body
                Constraint::Length(1), // Status message
                Constraint::Length(1), // Keybindings footer
            ])
            .split(area);

        self.render_header(chunks[0], buf);
        self.render_tabs(chunks[1], buf);

        match self.state.active_tab {
            BenchTab::Overview => self.render_overview_tab(chunks[2], buf),
            BenchTab::Results => self.render_results_tab(chunks[2], buf),
            BenchTab::Charts => self.render_charts_tab(chunks[2], buf),
            BenchTab::Inspector => self.render_inspector_tab(chunks[2], buf),
            BenchTab::Recommendations => self.render_recommendations_tab(chunks[2], buf),
        }

        self.render_status_bar(chunks[3], buf);
        self.render_footer(chunks[4], buf);
    }
}

impl<'a> BenchRunnerWidget<'a> {
    fn render_header(&self, area: Rect, buf: &mut Buffer) {
        let title_spans = vec![
            Span::styled(
                "✦ LLM Provider Benchmark Runner ",
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled("• TTFT & Throughput", Style::default().fg(Color::DarkGray)),
        ];

        let mode_str = if self.state.is_parallel {
            "Parallel"
        } else {
            "Sequential"
        };
        let right_spans = vec![
            Span::styled(
                format!("Mode: {} ", mode_str),
                Style::default().fg(Color::Yellow),
            ),
            Span::styled(
                format!("Rounds: {} ", self.state.rounds),
                Style::default().fg(Color::White),
            ),
            Span::styled(
                format!("Max: {} tok", self.state.max_tokens),
                Style::default().fg(Color::DarkGray),
            ),
        ];

        let line = Line::from(title_spans);
        buf.set_line(area.x, area.y, &line, area.width);

        let right_line = Line::from(right_spans);
        let right_width = right_line.width() as u16;
        if area.width > right_width {
            buf.set_line(
                area.x + area.width - right_width,
                area.y,
                &right_line,
                right_width,
            );
        }
    }

    fn render_tabs(&self, area: Rect, buf: &mut Buffer) {
        let mut spans = Vec::new();
        for tab in BenchTab::ALL {
            let is_active = self.state.active_tab == tab;
            let style = if is_active {
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD | Modifier::UNDERLINED)
            } else {
                Style::default().fg(Color::DarkGray)
            };

            let prefix = if is_active { "▶ " } else { "  " };
            spans.push(Span::styled(format!("{}{}", prefix, tab.label()), style));
            spans.push(Span::raw("   "));
        }

        let line = Line::from(spans);
        buf.set_line(area.x, area.y, &line, area.width);
    }

    fn render_overview_tab(&self, area: Rect, buf: &mut Buffer) {
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(3), // Progress Gauge
                Constraint::Min(5),    // Split: Active Card (left) & Queue List (right)
            ])
            .split(area);

        // Progress Gauge
        let pct = (self.state.progress_fraction() * 100.0) as u16;
        let gauge = Gauge::default()
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_type(BorderType::Rounded)
                    .title(" Overall Benchmark Progress "),
            )
            .gauge_style(Style::default().fg(Color::Green).bg(Color::DarkGray))
            .percent(pct)
            .label(format!(
                "{}% ({} / {} targets completed)",
                pct,
                self.state
                    .targets
                    .iter()
                    .filter(|t| t.phase == StreamPhase::Completed)
                    .count(),
                self.state.targets.len()
            ));
        gauge.render(chunks[0], buf);

        // Split view
        let body_chunks = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([
                Constraint::Percentage(55), // Active Live Stream Card
                Constraint::Percentage(45), // Target Queue
            ])
            .split(chunks[1]);

        self.render_active_target_card(body_chunks[0], buf);
        self.render_queue_list(body_chunks[1], buf);
    }

    fn render_active_target_card(&self, area: Rect, buf: &mut Buffer) {
        let block = Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .title(" Live Streaming Stream Metrics ");
        let inner = block.inner(area);
        block.render(area, buf);

        if let Some(active_idx) = self.state.active_running_target_idx {
            if let Some(target_state) = self.state.targets.get(active_idx) {
                let mut lines = Vec::new();

                lines.push(Line::from(vec![
                    Span::styled("Provider: ", Style::default().fg(Color::DarkGray)),
                    Span::styled(
                        &target_state.target.provider,
                        Style::default()
                            .fg(Color::Cyan)
                            .add_modifier(Modifier::BOLD),
                    ),
                    Span::styled(
                        format!(" ({})", target_state.target.model),
                        Style::default().fg(Color::White),
                    ),
                ]));

                lines.push(Line::from(vec![
                    Span::styled("Phase: ", Style::default().fg(Color::DarkGray)),
                    Span::styled(target_state.phase.label(), target_state.phase.style()),
                    Span::styled(
                        format!(
                            "  Round: {}/{}",
                            target_state.current_round.max(1),
                            target_state.total_rounds
                        ),
                        Style::default().fg(Color::Yellow),
                    ),
                ]));

                lines.push(Line::raw(""));

                if let Some(metrics) = &target_state.live_metrics {
                    let ttft_val = metrics
                        .ttft
                        .map(format_duration_compact)
                        .unwrap_or_else(|| "measuring...".to_string());
                    lines.push(Line::from(vec![
                        Span::styled("TTFT: ", Style::default().fg(Color::DarkGray)),
                        Span::styled(
                            ttft_val,
                            Style::default()
                                .fg(Color::Green)
                                .add_modifier(Modifier::BOLD),
                        ),
                        Span::styled("   Stream Speed: ", Style::default().fg(Color::DarkGray)),
                        Span::styled(
                            format!("{:.1} tok/s", metrics.live_tokens_per_sec),
                            Style::default()
                                .fg(Color::Cyan)
                                .add_modifier(Modifier::BOLD),
                        ),
                    ]));

                    lines.push(Line::from(vec![
                        Span::styled("Tokens: ", Style::default().fg(Color::DarkGray)),
                        Span::styled(
                            metrics.total_tokens.to_string(),
                            Style::default().fg(Color::White),
                        ),
                        Span::styled("   Jitter: ", Style::default().fg(Color::DarkGray)),
                        Span::styled(
                            format!("{:.1} ms", metrics.jitter_ms),
                            Style::default().fg(Color::Yellow),
                        ),
                    ]));

                    lines.push(Line::raw(""));
                    lines.push(Line::styled(
                        "Live Stream Preview:",
                        Style::default().fg(Color::DarkGray),
                    ));
                    let preview = sanitize_preview_text(
                        &metrics.preview_buffer,
                        inner.width.saturating_sub(4) as usize,
                    );
                    lines.push(Line::styled(
                        format!("\"{}\"", preview),
                        Style::default().fg(Color::Green),
                    ));
                } else {
                    lines.push(Line::styled(
                        "Waiting for stream events...",
                        Style::default().fg(Color::DarkGray),
                    ));
                }

                Paragraph::new(lines).render(inner, buf);
                return;
            }
        }

        let idle_text = vec![
            Line::styled(
                "Benchmark Runner Idle",
                Style::default()
                    .fg(Color::White)
                    .add_modifier(Modifier::BOLD),
            ),
            Line::raw(""),
            Line::styled(
                "Press [Space] or [r] to start benchmarking.",
                Style::default().fg(Color::Cyan),
            ),
            Line::styled(
                "Press [m] to cycle test prompt presets.",
                Style::default().fg(Color::DarkGray),
            ),
            Line::styled(
                "Press [p] to toggle parallel mode.",
                Style::default().fg(Color::DarkGray),
            ),
        ];
        Paragraph::new(idle_text).render(inner, buf);
    }

    fn render_queue_list(&self, area: Rect, buf: &mut Buffer) {
        let block = Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .title(" Provider Queue ");
        let inner = block.inner(area);
        block.render(area, buf);

        let mut lines = Vec::new();
        for (i, t) in self.state.targets.iter().enumerate() {
            let is_selected = i == self.state.selected_target_idx;
            let pointer = if is_selected { "▶ " } else { "  " };
            let icon = t.phase.icon();
            let style = if is_selected {
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(Color::White)
            };

            let status_badge = match &t.summary {
                Some(s) if s.success_rate() > 0.0 => {
                    format!("{:.1} tok/s", s.avg_tokens_per_second)
                }
                Some(_) => "Failed".to_string(),
                None => t.phase.label().to_string(),
            };

            lines.push(Line::from(vec![
                Span::styled(pointer, Style::default().fg(Color::Cyan)),
                Span::styled(format!("{} ", icon), t.phase.style()),
                Span::styled(&t.target.provider, style),
                Span::styled(
                    format!(" ({})", t.target.model),
                    Style::default().fg(Color::DarkGray),
                ),
                Span::styled(
                    format!(" [{}]", status_badge),
                    Style::default().fg(Color::Yellow),
                ),
            ]));
        }

        Paragraph::new(lines).render(inner, buf);
    }

    fn render_results_tab(&self, area: Rect, buf: &mut Buffer) {
        let block = Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .title(" Benchmark Results Table ");
        let inner = block.inner(area);
        block.render(area, buf);

        let sorted_indices = self.state.sorted_target_indices();
        let header = Row::new(vec![
            "Provider",
            "Model",
            "TTFT",
            "Speed (tok/s)",
            "Latency",
            "Tokens",
            "Rating",
            "Status",
        ])
        .style(
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        );

        let rows: Vec<Row> = sorted_indices
            .iter()
            .map(|&idx| {
                let target = &self.state.targets[idx];
                let is_cursor = idx == self.state.selected_target_idx;
                let (ttft_str, tps_str, lat_str, tok_str, rating_str) = match &target.summary {
                    Some(s) if s.success_rate() > 0.0 => (
                        s.avg_ttft.map(format_duration_compact).unwrap_or_default(),
                        format!("{:.1}", s.avg_tokens_per_second),
                        format_duration_compact(s.avg_latency),
                        s.total_tokens_generated.to_string(),
                        s.rating.badge_text().to_string(),
                    ),
                    _ => (
                        "-".to_string(),
                        "-".to_string(),
                        "-".to_string(),
                        "-".to_string(),
                        "-".to_string(),
                    ),
                };

                let row_style = if is_cursor {
                    Style::default()
                        .fg(Color::White)
                        .bg(Color::DarkGray)
                        .add_modifier(Modifier::BOLD)
                } else {
                    Style::default().fg(Color::White)
                };

                Row::new(vec![
                    target.target.provider.clone(),
                    target.target.model.clone(),
                    ttft_str,
                    tps_str,
                    lat_str,
                    tok_str,
                    rating_str,
                    target.phase.label().to_string(),
                ])
                .style(row_style)
            })
            .collect();

        let table = RatatuiTable::new(
            rows,
            [
                Constraint::Length(14),
                Constraint::Length(18),
                Constraint::Length(10),
                Constraint::Length(14),
                Constraint::Length(10),
                Constraint::Length(8),
                Constraint::Length(14),
                Constraint::Length(12),
            ],
        )
        .header(header);

        table.render(inner, buf);
    }

    fn render_charts_tab(&self, area: Rect, buf: &mut Buffer) {
        let block = Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .title(" Performance Visual Comparison Charts ");
        let inner = block.inner(area);
        block.render(area, buf);

        let summaries = self.state.get_completed_summaries();
        let chart_text = render_charts_ansi(&summaries, false);
        let lines: Vec<Line> = chart_text
            .lines()
            .map(|l| Line::raw(l.to_string()))
            .collect();
        Paragraph::new(lines).render(inner, buf);
    }

    fn render_inspector_tab(&self, area: Rect, buf: &mut Buffer) {
        let block = Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .title(" Target Round Inspector & Jitter Drilldown ");
        let inner = block.inner(area);
        block.render(area, buf);

        if let Some(target) = self.state.targets.get(self.state.selected_target_idx) {
            let inspector_text = render_inspector_ansi(target, false);
            let lines: Vec<Line> = inspector_text
                .lines()
                .map(|l| Line::raw(l.to_string()))
                .collect();
            Paragraph::new(lines).render(inner, buf);
        } else {
            Paragraph::new("No provider selected.").render(inner, buf);
        }
    }

    fn render_recommendations_tab(&self, area: Rect, buf: &mut Buffer) {
        let block = Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .title(" Recommendations & Diagnostics ");
        let inner = block.inner(area);
        block.render(area, buf);

        let summaries = self.state.get_completed_summaries();
        let recs =
            format_rankings_and_recommendation(&summaries, &self.state.active_provider, false);
        let unconf = format_troubleshooting_and_unconfigured(&summaries, false);

        let mut lines = Vec::new();
        for l in recs.lines() {
            lines.push(Line::raw(l.to_string()));
        }
        lines.push(Line::raw(""));
        for l in unconf.lines() {
            lines.push(Line::raw(l.to_string()));
        }

        Paragraph::new(lines).render(inner, buf);
    }

    fn render_status_bar(&self, area: Rect, buf: &mut Buffer) {
        let line = Line::from(vec![
            Span::styled(" Status: ", Style::default().fg(Color::DarkGray)),
            Span::styled(
                &self.state.status_message,
                Style::default().fg(Color::Yellow),
            ),
        ]);
        buf.set_line(area.x, area.y, &line, area.width);
    }

    fn render_footer(&self, area: Rect, buf: &mut Buffer) {
        let spans = vec![
            Span::styled("[Tab] ", Style::default().fg(Color::Cyan)),
            Span::raw("Switch Tab   "),
            Span::styled("[Space/r] ", Style::default().fg(Color::Cyan)),
            Span::raw("Run Benchmark   "),
            Span::styled("[s] ", Style::default().fg(Color::Cyan)),
            Span::raw("Sort   "),
            Span::styled("[p] ", Style::default().fg(Color::Cyan)),
            Span::raw("Parallel   "),
            Span::styled("[m] ", Style::default().fg(Color::Cyan)),
            Span::raw("Prompt   "),
            Span::styled("[↑↓] ", Style::default().fg(Color::Cyan)),
            Span::raw("Select   "),
            Span::styled("[Esc/q] ", Style::default().fg(Color::Cyan)),
            Span::raw("Exit"),
        ];
        let line = Line::from(spans);
        buf.set_line(area.x, area.y, &line, area.width);
    }
}

// ============================================================================
// 7. Interactive TUI Runner & Event Loop
// ============================================================================

/// Runs the interactive benchmark runner application until user exit.
pub async fn run_interactive_benchmark(
    config: &Config,
    client: &LlmClient,
    options: &BenchmarkOptions,
) -> Vec<ProviderBenchmarkSummary> {
    let targets = discover_benchmark_targets(config, options);
    let preset = if options.ping_only {
        BenchPromptPreset::Ping
    } else {
        BenchPromptPreset::Haiku
    };

    let mut state =
        BenchRunnerState::new(targets, &config.default_provider, options.rounds, preset);
    state.is_parallel = options.parallel;

    let mut terminal_guard = match RawModeGuard::enter() {
        Ok(guard) => Some(guard),
        Err(_) => None,
    };

    let mut stdout_handle = stdout();
    let _ = execute!(stdout_handle, EnterAlternateScreen, cursor::Hide);
    let backend = CrosstermBackend::new(stdout_handle);
    let mut terminal = match ratatui::Terminal::new(backend) {
        Ok(t) => t,
        Err(_) => {
            if let Some(guard) = terminal_guard.take() {
                drop(guard);
            }
            return Vec::new();
        }
    };

    let theme = Theme::auto();

    loop {
        // Render TUI frame
        let _ = terminal.draw(|f| {
            let area = f.area();
            f.render_widget(BenchRunnerWidget::new(&state, theme.clone()), area);
        });

        // Handle Events
        if event::poll(Duration::from_millis(50)).unwrap_or(false) {
            if let Ok(Event::Key(key)) = event::read() {
                if key.kind == KeyEventKind::Press {
                    match key.code {
                        KeyCode::Char('q') | KeyCode::Esc => {
                            break;
                        }
                        KeyCode::Tab => {
                            state.active_tab = state.active_tab.next();
                        }
                        KeyCode::BackTab => {
                            state.active_tab = state.active_tab.prev();
                        }
                        KeyCode::Char('1') => state.active_tab = BenchTab::Overview,
                        KeyCode::Char('2') => state.active_tab = BenchTab::Results,
                        KeyCode::Char('3') => state.active_tab = BenchTab::Charts,
                        KeyCode::Char('4') => state.active_tab = BenchTab::Inspector,
                        KeyCode::Char('5') => state.active_tab = BenchTab::Recommendations,
                        KeyCode::Up | KeyCode::Char('k') => state.select_prev(),
                        KeyCode::Down | KeyCode::Char('j') => state.select_next(),
                        KeyCode::Char('s') => state.cycle_sort_column(),
                        KeyCode::Char('p') => state.toggle_parallel(),
                        KeyCode::Char('m') => state.cycle_prompt_preset(),
                        KeyCode::Enter => {
                            state.active_tab = BenchTab::Inspector;
                        }
                        KeyCode::Char(' ') | KeyCode::Char('r') => {
                            if !state.is_running {
                                execute_benchmark_run(
                                    &mut state,
                                    client,
                                    &mut terminal,
                                    theme.clone(),
                                )
                                .await;
                            }
                        }
                        _ => {}
                    }
                }
            }
        }

        state.spinner_tick = state.spinner_tick.wrapping_add(1);
    }

    // Cleanup terminal
    let _ = execute!(terminal.backend_mut(), LeaveAlternateScreen, cursor::Show);
    let _ = crossterm::terminal::disable_raw_mode();
    if let Some(guard) = terminal_guard.take() {
        drop(guard);
    }

    state.get_completed_summaries()
}

/// Executes all selected target benchmarks while updating the TUI in real-time.
async fn execute_benchmark_run<B: ratatui::backend::Backend>(
    state: &mut BenchRunnerState,
    client: &LlmClient,
    terminal: &mut ratatui::Terminal<B>,
    theme: Theme,
) {
    state.is_running = true;
    let rounds = state.rounds;
    let prompt = state.effective_prompt().to_string();
    let max_tokens = state.max_tokens;
    let timeout_secs = state.timeout_secs;

    for target in &mut state.targets {
        target.reset_for_run(rounds);
    }

    let selected_indices: Vec<usize> = state
        .targets
        .iter()
        .enumerate()
        .filter(|(_, t)| t.is_selected)
        .map(|(i, _)| i)
        .collect();

    let theme_inner = theme.clone();

    for &target_idx in &selected_indices {
        state.active_running_target_idx = Some(target_idx);

        for r in 1..=rounds {
            if let Some(target_state) = state.targets.get_mut(target_idx) {
                target_state.current_round = r;
                target_state.live_metrics = Some(LiveStreamMetrics::new());
            }

            let target_clone = state.targets[target_idx].target.clone();

            let run_res = measure_streaming_provider(
                client,
                &target_clone,
                &prompt,
                max_tokens,
                Some(0.2),
                timeout_secs,
                r,
                |evt| {
                    match evt {
                        LiveStreamEvent::PhaseChanged(phase) => {
                            if let Some(ts) = state.targets.get_mut(target_idx) {
                                ts.phase = phase;
                            }
                        }
                        LiveStreamEvent::FirstToken { ttft } => {
                            if let Some(ts) = state.targets.get_mut(target_idx) {
                                if let Some(metrics) = &mut ts.live_metrics {
                                    metrics.record_first_token(ttft);
                                }
                            }
                        }
                        LiveStreamEvent::ChunkReceived(chunk) => {
                            if let Some(ts) = state.targets.get_mut(target_idx) {
                                if let Some(metrics) = &mut ts.live_metrics {
                                    metrics.record_chunk(chunk);
                                }
                            }
                        }
                        LiveStreamEvent::RoundDone(_) => {}
                    }

                    // Render interim frame during streaming
                    let _ = terminal.draw(|f| {
                        let area = f.area();
                        f.render_widget(BenchRunnerWidget::new(state, theme_inner.clone()), area);
                    });
                },
            )
            .await;

            if let Some(target_state) = state.targets.get_mut(target_idx) {
                target_state.record_round_result(run_res);
            }

            let _ = terminal.draw(|f| {
                let area = f.area();
                f.render_widget(BenchRunnerWidget::new(state, theme.clone()), area);
            });
        }
    }

    state.active_running_target_idx = None;
    state.is_running = false;
    state.status_message = "Benchmark suite completed successfully!".to_string();
}

// ============================================================================
// 8. Utility Functions
// ============================================================================

fn current_timestamp() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or(Duration::ZERO)
        .as_secs()
}

fn sanitize_preview_text(s: &str, max_len: usize) -> String {
    let clean = s
        .chars()
        .map(|c| {
            if c == '\n' || c == '\r' || c == '\t' {
                ' '
            } else {
                c
            }
        })
        .collect::<String>();
    let trimmed = clean.trim();
    if trimmed.chars().count() > max_len {
        let truncated: String = trimmed.chars().take(max_len.saturating_sub(3)).collect();
        format!("{truncated}...")
    } else {
        trimmed.to_string()
    }
}

// ============================================================================
// 9. Unit Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn create_test_target(provider: &str, model: &str, configured: bool) -> BenchmarkTarget {
        BenchmarkTarget {
            provider: provider.to_string(),
            model: model.to_string(),
            api_key: if configured {
                Some("test_key".to_string())
            } else {
                None
            },
            base_url: "https://api.example.com".to_string(),
            is_configured: configured,
            setup_hint: None,
        }
    }

    #[test]
    fn test_stream_phase_properties() {
        assert_eq!(StreamPhase::Idle.label(), "IDLE");
        assert_eq!(StreamPhase::Streaming.label(), "STREAMING");
        assert_eq!(StreamPhase::Completed.icon(), "✓");
        assert_eq!(StreamPhase::Failed.icon(), "✗");

        let badge = StreamPhase::Streaming.badge_ansi(true);
        assert!(badge.contains("STREAMING"));
    }

    #[test]
    fn test_live_stream_metrics_computation() {
        let mut metrics = LiveStreamMetrics::new();
        assert_eq!(metrics.chunk_count, 0);

        // Record TTFT
        metrics.record_first_token(Duration::from_millis(250));
        assert_eq!(metrics.ttft, Some(Duration::from_millis(250)));

        // Record chunk 1
        metrics.record_chunk(LiveStreamChunk {
            timestamp_ms: 300,
            chunk_idx: 1,
            byte_count: 16,
            token_count: 4,
            text_snippet: "Hello world ".to_string(),
            interval_ms: 50,
        });

        assert_eq!(metrics.total_tokens, 4);
        assert_eq!(metrics.chunk_count, 1);
        assert_eq!(metrics.min_chunk_interval_ms, 50);
        assert_eq!(metrics.max_chunk_interval_ms, 50);

        // Record chunk 2
        metrics.record_chunk(LiveStreamChunk {
            timestamp_ms: 360,
            chunk_idx: 2,
            byte_count: 20,
            token_count: 5,
            text_snippet: "from Rust benchmarks".to_string(),
            interval_ms: 60,
        });

        assert_eq!(metrics.total_tokens, 9);
        assert_eq!(metrics.chunk_count, 2);
        assert_eq!(metrics.min_chunk_interval_ms, 50);
        assert_eq!(metrics.max_chunk_interval_ms, 60);
        assert_eq!(metrics.avg_chunk_interval_ms, 55.0);
        assert!(metrics
            .preview_buffer
            .contains("Hello world from Rust benchmarks"));
    }

    #[test]
    fn test_bench_runner_state_navigation() {
        let targets = vec![
            create_test_target("anthropic", "claude-3-5-sonnet", true),
            create_test_target("openai", "gpt-4o", true),
            create_test_target("deepseek", "deepseek-chat", true),
        ];

        let mut state = BenchRunnerState::new(targets, "anthropic", 1, BenchPromptPreset::Haiku);
        assert_eq!(state.selected_target_idx, 0);
        assert_eq!(state.active_tab, BenchTab::Overview);

        state.select_next();
        assert_eq!(state.selected_target_idx, 1);

        state.select_next();
        assert_eq!(state.selected_target_idx, 2);

        state.select_next();
        assert_eq!(state.selected_target_idx, 0); // Wrap around

        state.select_prev();
        assert_eq!(state.selected_target_idx, 2); // Wrap backwards
    }

    #[test]
    fn test_bench_runner_sort_indices() {
        let targets = vec![
            create_test_target("anthropic", "claude-3-5-sonnet", true),
            create_test_target("deepseek", "deepseek-chat", true),
        ];

        let mut state = BenchRunnerState::new(targets, "anthropic", 1, BenchPromptPreset::Haiku);

        // Anthropic: 400ms TTFT, 50 tok/s
        state.targets[0].record_round_result(BenchmarkRunResult {
            provider: "anthropic".to_string(),
            model: "claude-3-5-sonnet".to_string(),
            round: 1,
            ttft: Duration::from_millis(400),
            generation_duration: Duration::from_millis(1000),
            total_latency: Duration::from_millis(1400),
            tokens_generated: 50,
            tokens_per_second: 50.0,
            success: true,
            error_message: None,
            response_preview: "test".to_string(),
            timestamp_unix: 1000,
        });

        // Deepseek: 150ms TTFT, 90 tok/s
        state.targets[1].record_round_result(BenchmarkRunResult {
            provider: "deepseek".to_string(),
            model: "deepseek-chat".to_string(),
            round: 1,
            ttft: Duration::from_millis(150),
            generation_duration: Duration::from_millis(1000),
            total_latency: Duration::from_millis(1150),
            tokens_generated: 90,
            tokens_per_second: 90.0,
            success: true,
            error_message: None,
            response_preview: "test".to_string(),
            timestamp_unix: 1000,
        });

        // Sort by TTFT (ascending: fastest first)
        state.sort_column = BenchSortColumn::TTFT;
        state.sort_descending = false;
        let sorted = state.sorted_target_indices();
        assert_eq!(sorted, vec![1, 0]); // Deepseek (150ms) before Anthropic (400ms)

        // Sort by Throughput (descending: highest tok/s first)
        state.sort_column = BenchSortColumn::Throughput;
        let sorted_tps = state.sorted_target_indices();
        assert_eq!(sorted_tps, vec![1, 0]); // Deepseek (90 tok/s) before Anthropic (50 tok/s)
    }

    #[test]
    fn test_prompt_presets() {
        let p = BenchPromptPreset::Ping;
        assert_eq!(p.max_tokens(), 16);
        assert_eq!(p.prompt_text(), DEFAULT_PING_PROMPT);

        let next_p = p.next();
        assert_eq!(next_p, BenchPromptPreset::Haiku);
        assert_eq!(next_p.max_tokens(), 96);
    }

    #[test]
    fn test_ansi_renderers_output() {
        let targets = vec![create_test_target("openai", "gpt-4o", true)];
        let mut state = BenchRunnerState::new(targets, "openai", 1, BenchPromptPreset::Haiku);

        state.targets[0].record_round_result(BenchmarkRunResult {
            provider: "openai".to_string(),
            model: "gpt-4o".to_string(),
            round: 1,
            ttft: Duration::from_millis(300),
            generation_duration: Duration::from_millis(1000),
            total_latency: Duration::from_millis(1300),
            tokens_generated: 60,
            tokens_per_second: 60.0,
            success: true,
            error_message: None,
            response_preview: "AI fast code".to_string(),
            timestamp_unix: 1000,
        });

        let status_ansi = render_live_status_ansi(&state, false);
        assert!(status_ansi.contains("LLM Provider Benchmark Runner"));
        assert!(status_ansi.contains("openai"));

        let summaries = state.get_completed_summaries();
        let charts_ansi = render_charts_ansi(&summaries, false);
        assert!(charts_ansi.contains("Time to First Token"));
        assert!(charts_ansi.contains("Generation Throughput"));

        let inspector_ansi = render_inspector_ansi(&state.targets[0], false);
        assert!(inspector_ansi.contains("Target Drilldown"));
        assert!(inspector_ansi.contains("Round"));
    }

    #[test]
    fn test_ratatui_widget_rendering() {
        let targets = vec![
            create_test_target("anthropic", "claude-3-5-sonnet", true),
            create_test_target("deepseek", "deepseek-chat", true),
        ];
        let state = BenchRunnerState::new(targets, "anthropic", 1, BenchPromptPreset::Haiku);
        let theme = Theme::tokyo_night();

        let widget = BenchRunnerWidget::new(&state, theme);
        let area = Rect::new(0, 0, 100, 30);
        let mut buffer = Buffer::empty(area);

        // Rendering should complete cleanly without panic
        widget.render(area, &mut buffer);
    }
}
