//! Comprehensive subagent metrics, telemetry, and profiling collector.
//!
//! Provides:
//! 1. Per-subagent telemetry tracking: duration, prompt tokens, completion tokens,
//!    cache token accounting, cost estimation, tool call counts, and success rates.
//! 2. Granular turn-level profiling: per-turn latency, token consumption, reasoning size,
//!    and tool execution breakdown.
//! 3. Granular tool-level profiling: execution count, success rate, error count,
//!    latency statistics (min, max, avg, total duration), and payload volume.
//! 4. Fleet-wide rollup & statistical profiling: overall success rates, percentile
//!    latencies (p50, p90, p95, p99), token throughput, cost rollup, role breakdown,
//!    and aggregate tool performance.
//! 5. Event-driven observer: automatically derives metrics from [`SubagentProgress`]
//!    event streams or explicit lifecycle hooks.
//! 6. Rich export & visualization formats: formatted ASCII/Unicode tables, detailed
//!    diagnostic reports, Markdown summaries, CSV exports, and structured JSON.

use std::collections::HashMap;
use std::fmt;
use std::sync::{Arc, Mutex};
use std::time::Instant;

use chrono::Utc;
use serde::{Deserialize, Serialize};
use tokio::sync::broadcast;

use crate::agent::cost::{estimate_cost, CostBreakdown};
use crate::agent::session::TokenStats;
use crate::agent::subagent::{SubagentProgress, SubagentRole, SubagentStatus};
use crate::agent::tokens::estimate_text_tokens;

// ---------------------------------------------------------------------------
// 1. Status & Core Enums
// ---------------------------------------------------------------------------

/// Execution status of a subagent from a metrics perspective.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SubagentMetricStatus {
    /// Subagent is initialized and waiting for an execution slot.
    Pending,
    /// Subagent is actively running turns or executing tools.
    Running,
    /// Subagent completed its assigned mission successfully.
    Completed,
    /// Subagent encountered an unrecoverable failure or turn limit.
    Failed,
    /// Subagent was cancelled before normal completion.
    Cancelled,
    /// Subagent timed out waiting for completion or execution.
    TimedOut,
}

impl SubagentMetricStatus {
    /// Returns `true` if the subagent status represents a finalized terminal state.
    pub fn is_terminal(&self) -> bool {
        matches!(
            self,
            Self::Completed | Self::Failed | Self::Cancelled | Self::TimedOut
        )
    }

    /// Returns `true` if the status indicates successful completion.
    pub fn is_success(&self) -> bool {
        matches!(self, Self::Completed)
    }

    /// Returns `true` if the status indicates an error or failure condition.
    pub fn is_failure(&self) -> bool {
        matches!(self, Self::Failed | Self::TimedOut)
    }

    /// Maps from [`SubagentStatus`] to [`SubagentMetricStatus`].
    pub fn from_subagent_status(status: &SubagentStatus) -> Self {
        match status {
            SubagentStatus::Pending => Self::Pending,
            SubagentStatus::Running { .. } => Self::Running,
            SubagentStatus::Completed { .. } => Self::Completed,
            SubagentStatus::Failed { .. } => Self::Failed,
            SubagentStatus::Cancelled => Self::Cancelled,
        }
    }
}

impl fmt::Display for SubagentMetricStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Pending => write!(f, "Pending"),
            Self::Running => write!(f, "Running"),
            Self::Completed => write!(f, "Completed"),
            Self::Failed => write!(f, "Failed"),
            Self::Cancelled => write!(f, "Cancelled"),
            Self::TimedOut => write!(f, "TimedOut"),
        }
    }
}

// ---------------------------------------------------------------------------
// 2. Tool-Level Profiling Records
// ---------------------------------------------------------------------------

/// Detailed record of an individual tool call execution sample.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolCallSample {
    /// Unique call identifier.
    pub call_id: String,
    /// Name of the invoked tool.
    pub tool_name: String,
    /// Turn index during which the tool was invoked.
    pub turn: usize,
    /// RFC 3339 timestamp when execution began.
    pub started_at: String,
    /// Execution duration in milliseconds.
    pub duration_ms: u64,
    /// Whether the tool returned a success result.
    pub success: bool,
    /// Size of the returned output in bytes.
    pub output_bytes: usize,
    /// Optional error message if the tool failed.
    pub error: Option<String>,
}

/// Aggregated usage statistics and latency profile for a specific tool.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolUsageMetrics {
    /// Name of the tool.
    pub tool_name: String,
    /// Total number of invocations.
    pub invocations: usize,
    /// Number of successful invocations.
    pub successes: usize,
    /// Number of failed invocations.
    pub failures: usize,
    /// Total cumulative duration in milliseconds.
    pub total_duration_ms: u64,
    /// Minimum observed execution duration in milliseconds.
    pub min_duration_ms: u64,
    /// Maximum observed execution duration in milliseconds.
    pub max_duration_ms: u64,
    /// Total bytes generated across all outputs.
    pub total_output_bytes: usize,
}

impl ToolUsageMetrics {
    /// Creates a new, zeroed `ToolUsageMetrics` record for a named tool.
    pub fn new(tool_name: impl Into<String>) -> Self {
        Self {
            tool_name: tool_name.into(),
            invocations: 0,
            successes: 0,
            failures: 0,
            total_duration_ms: 0,
            min_duration_ms: 0,
            max_duration_ms: 0,
            total_output_bytes: 0,
        }
    }

    /// Records an execution sample into the cumulative statistics.
    pub fn record(&mut self, duration_ms: u64, success: bool, output_bytes: usize) {
        if self.invocations == 0 {
            self.min_duration_ms = duration_ms;
            self.max_duration_ms = duration_ms;
        } else {
            self.min_duration_ms = self.min_duration_ms.min(duration_ms);
            self.max_duration_ms = self.max_duration_ms.max(duration_ms);
        }

        self.invocations += 1;
        if success {
            self.successes += 1;
        } else {
            self.failures += 1;
        }

        self.total_duration_ms = self.total_duration_ms.saturating_add(duration_ms);
        self.total_output_bytes = self.total_output_bytes.saturating_add(output_bytes);
    }

    /// Calculates the average execution duration in milliseconds.
    pub fn avg_duration_ms(&self) -> f64 {
        if self.invocations == 0 {
            0.0
        } else {
            self.total_duration_ms as f64 / self.invocations as f64
        }
    }

    /// Calculates the success rate as a ratio between 0.0 and 1.0.
    pub fn success_rate(&self) -> f64 {
        if self.invocations == 0 {
            1.0
        } else {
            self.successes as f64 / self.invocations as f64
        }
    }

    /// Calculates the failure rate as a ratio between 0.0 and 1.0.
    pub fn failure_rate(&self) -> f64 {
        if self.invocations == 0 {
            0.0
        } else {
            self.failures as f64 / self.invocations as f64
        }
    }

    /// Calculates the average output payload size in bytes.
    pub fn avg_output_bytes(&self) -> f64 {
        if self.invocations == 0 {
            0.0
        } else {
            self.total_output_bytes as f64 / self.invocations as f64
        }
    }
}

// ---------------------------------------------------------------------------
// 3. Turn-Level Profiling Records
// ---------------------------------------------------------------------------

/// Granular telemetry snapshot for an individual turn in a subagent session.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TurnMetric {
    /// 1-based turn index.
    pub turn: usize,
    /// RFC 3339 timestamp when the turn started.
    pub started_at: String,
    /// Turn execution duration in milliseconds.
    pub duration_ms: u64,
    /// Prompt tokens sent in this turn.
    pub prompt_tokens: u64,
    /// Completion tokens generated in this turn.
    pub completion_tokens: u64,
    /// Total tokens consumed in this turn.
    pub total_tokens: u64,
    /// Number of tool calls executed in this turn.
    pub tool_calls_count: usize,
    /// Detailed records of tool calls executed during this turn.
    pub tool_calls: Vec<ToolCallSample>,
    /// Number of characters of reasoning / thinking generated.
    pub reasoning_chars: usize,
    /// Number of characters of content generated.
    pub content_chars: usize,
    /// Whether the turn completed without unhandled errors.
    pub success: bool,
    /// Optional error message encountered during the turn.
    pub error: Option<String>,
}

impl TurnMetric {
    /// Creates a new empty turn metric starting at the current time.
    pub fn new(turn: usize) -> Self {
        Self {
            turn,
            started_at: Utc::now().to_rfc3339(),
            duration_ms: 0,
            prompt_tokens: 0,
            completion_tokens: 0,
            total_tokens: 0,
            tool_calls_count: 0,
            tool_calls: Vec::new(),
            reasoning_chars: 0,
            content_chars: 0,
            success: true,
            error: None,
        }
    }
}

// ---------------------------------------------------------------------------
// 4. Per-Subagent Metrics & Profile
// ---------------------------------------------------------------------------

/// Comprehensive telemetry and profiling record for a single subagent.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SubagentMetrics {
    /// Unique identifier for the subagent.
    pub id: String,
    /// Display name of the subagent.
    pub name: String,
    /// Specialized role (e.g. "Scout", "Coder", "Tester", "Reviewer", "General").
    pub role: String,
    /// Assigned task description.
    pub task: String,
    /// LLM model name used for inference.
    pub model: String,
    /// Current execution status.
    pub status: SubagentMetricStatus,
    /// RFC 3339 timestamp when the subagent started.
    pub started_at: String,
    /// RFC 3339 timestamp when the subagent completed (if finished).
    pub completed_at: Option<String>,
    /// Total wall-clock execution duration in milliseconds.
    pub duration_ms: u64,
    /// Number of turns executed so far.
    pub turns: usize,
    /// Maximum allowed turns.
    pub max_turns: usize,
    /// Cumulative prompt tokens consumed.
    pub prompt_tokens: u64,
    /// Cumulative completion tokens generated.
    pub completion_tokens: u64,
    /// Cumulative total tokens consumed (prompt + completion).
    pub total_tokens: u64,
    /// Cached tokens read from provider context cache.
    pub cache_read_tokens: u64,
    /// Cached tokens written to provider context cache.
    pub cache_write_tokens: u64,
    /// Estimated financial cost in USD.
    pub estimated_cost_usd: f64,
    /// Detailed financial cost breakdown.
    pub cost_breakdown: CostBreakdown,
    /// Per-tool usage and performance breakdown.
    pub tool_metrics: HashMap<String, ToolUsageMetrics>,
    /// Sequential timeline of all executed tool calls.
    pub tool_calls_history: Vec<ToolCallSample>,
    /// Turn-by-turn chronological profiling records.
    pub turns_history: Vec<TurnMetric>,
    /// Error message if the subagent failed.
    pub error: Option<String>,
    /// Snippet or preview of the final output (truncated if large).
    pub output_preview: Option<String>,
}

impl SubagentMetrics {
    /// Creates a new initialized `SubagentMetrics` instance.
    pub fn new(
        id: impl Into<String>,
        name: impl Into<String>,
        role: impl Into<String>,
        task: impl Into<String>,
        model: impl Into<String>,
        max_turns: usize,
    ) -> Self {
        Self {
            id: id.into(),
            name: name.into(),
            role: role.into(),
            task: task.into(),
            model: model.into(),
            status: SubagentMetricStatus::Pending,
            started_at: Utc::now().to_rfc3339(),
            completed_at: None,
            duration_ms: 0,
            turns: 0,
            max_turns,
            prompt_tokens: 0,
            completion_tokens: 0,
            total_tokens: 0,
            cache_read_tokens: 0,
            cache_write_tokens: 0,
            estimated_cost_usd: 0.0,
            cost_breakdown: CostBreakdown::zero(),
            tool_metrics: HashMap::new(),
            tool_calls_history: Vec::new(),
            turns_history: Vec::new(),
            error: None,
            output_preview: None,
        }
    }

    /// Returns `true` if the subagent finished successfully.
    pub fn is_success(&self) -> bool {
        self.status.is_success()
    }

    /// Calculates the total number of tool invocations across all tools.
    pub fn total_tool_calls(&self) -> usize {
        self.tool_metrics.values().map(|m| m.invocations).sum()
    }

    /// Calculates the total number of successful tool invocations.
    pub fn total_tool_successes(&self) -> usize {
        self.tool_metrics.values().map(|m| m.successes).sum()
    }

    /// Calculates the total number of failed tool invocations.
    pub fn total_tool_failures(&self) -> usize {
        self.tool_metrics.values().map(|m| m.failures).sum()
    }

    /// Calculates the overall tool invocation success rate as a ratio (0.0 to 1.0).
    pub fn tool_success_rate(&self) -> f64 {
        let total = self.total_tool_calls();
        if total == 0 {
            1.0
        } else {
            self.total_tool_successes() as f64 / total as f64
        }
    }

    /// Calculates token throughput in tokens generated per second.
    pub fn completion_tokens_per_second(&self) -> f64 {
        if self.duration_ms == 0 {
            0.0
        } else {
            (self.completion_tokens as f64 / self.duration_ms as f64) * 1000.0
        }
    }

    /// Calculates total token throughput in tokens per second.
    pub fn total_tokens_per_second(&self) -> f64 {
        if self.duration_ms == 0 {
            0.0
        } else {
            (self.total_tokens as f64 / self.duration_ms as f64) * 1000.0
        }
    }

    /// Calculates the average latency per turn in milliseconds.
    pub fn avg_turn_duration_ms(&self) -> f64 {
        if self.turns == 0 {
            0.0
        } else {
            self.duration_ms as f64 / self.turns as f64
        }
    }

    /// Re-estimates the financial cost using the active model pricing registry.
    pub fn update_cost(&mut self, provider: &str) {
        let stats = TokenStats {
            prompt_tokens: self.prompt_tokens,
            completion_tokens: self.completion_tokens,
            total_tokens: self.total_tokens,
            cache_read_tokens: self.cache_read_tokens,
            cache_write_tokens: self.cache_write_tokens,
            total_turns: self.turns as u64,
        };
        let breakdown = estimate_cost(provider, &self.model, &stats);
        self.cost_breakdown = breakdown;
        self.estimated_cost_usd = breakdown.total_cost;
    }

    /// Formats a concise one-line summary string.
    pub fn format_summary(&self) -> String {
        format!(
            "[{}] {} ({}) | Status: {} | Duration: {:.2}s | Turns: {}/{} | Tokens: {} (prompt: {}, compl: {}) | Tools: {} ({:.1}% succ) | Cost: ${:.4}",
            self.id,
            self.name,
            self.role,
            self.status,
            self.duration_ms as f64 / 1000.0,
            self.turns,
            self.max_turns,
            self.total_tokens,
            self.prompt_tokens,
            self.completion_tokens,
            self.total_tool_calls(),
            self.tool_success_rate() * 100.0,
            self.estimated_cost_usd
        )
    }

    /// Formats an in-depth diagnostic profiling report.
    pub fn format_detailed_profile(&self) -> String {
        let mut out = String::new();
        out.push_str(&format!(
            "════════════════════════════════════════════════════════════════════════════════\n"
        ));
        out.push_str(&format!(" SUBAGENT PROFILE: {} ({})\n", self.name, self.id));
        out.push_str(&format!(
            "════════════════════════════════════════════════════════════════════════════════\n"
        ));
        out.push_str(&format!("  Role:             {}\n", self.role));
        out.push_str(&format!("  Model:            {}\n", self.model));
        out.push_str(&format!("  Status:           {}\n", self.status));
        out.push_str(&format!("  Task:             {}\n", self.task));
        out.push_str(&format!("  Started At:       {}\n", self.started_at));
        if let Some(end) = &self.completed_at {
            out.push_str(&format!("  Completed At:     {}\n", end));
        }
        out.push_str(&format!(
            "  Duration:         {:.3}s ({} ms)\n",
            self.duration_ms as f64 / 1000.0,
            self.duration_ms
        ));
        out.push_str(&format!(
            "  Turns Executed:   {}/{} (avg {:.1} ms/turn)\n",
            self.turns,
            self.max_turns,
            self.avg_turn_duration_ms()
        ));
        out.push_str(&format!("  Tokens:\n"));
        out.push_str(&format!("    - Prompt:       {}\n", self.prompt_tokens));
        out.push_str(&format!("    - Completion:   {}\n", self.completion_tokens));
        out.push_str(&format!("    - Total:        {}\n", self.total_tokens));
        if self.cache_read_tokens > 0 || self.cache_write_tokens > 0 {
            out.push_str(&format!("    - Cache Read:   {}\n", self.cache_read_tokens));
            out.push_str(&format!(
                "    - Cache Write:  {}\n",
                self.cache_write_tokens
            ));
        }
        out.push_str(&format!(
            "  Throughput:       {:.1} compl tokens/sec, {:.1} total tokens/sec\n",
            self.completion_tokens_per_second(),
            self.total_tokens_per_second()
        ));
        out.push_str(&format!(
            "  Estimated Cost:   ${:.5} (input: ${:.5}, output: ${:.5})\n",
            self.estimated_cost_usd,
            self.cost_breakdown.input_cost,
            self.cost_breakdown.output_cost
        ));

        let total_tools = self.total_tool_calls();
        out.push_str(&format!(
            "  Tool Invocations: {} (success: {}, failed: {}, rate: {:.1}%)\n",
            total_tools,
            self.total_tool_successes(),
            self.total_tool_failures(),
            self.tool_success_rate() * 100.0
        ));

        if !self.tool_metrics.is_empty() {
            out.push_str("\n  ┌─ Tool Breakdown ─────────────────────────────────────────────────────────────\n");
            out.push_str("  │ Tool Name          Calls   Succ   Fail   Succ Rate   Avg Latency   Total Latency\n");
            out.push_str("  ├─────────────────────────────────────────────────────────────────────────────\n");
            let mut sorted_tools: Vec<_> = self.tool_metrics.values().collect();
            sorted_tools.sort_by(|a, b| b.invocations.cmp(&a.invocations));
            for t in sorted_tools {
                out.push_str(&format!(
                    "  │ {:<18} {:<7} {:<6} {:<6} {:<11.1}% {:<13.1}ms {:<11.1}ms\n",
                    t.tool_name,
                    t.invocations,
                    t.successes,
                    t.failures,
                    t.success_rate() * 100.0,
                    t.avg_duration_ms(),
                    t.total_duration_ms as f64
                ));
            }
            out.push_str("  └─────────────────────────────────────────────────────────────────────────────\n");
        }

        if !self.turns_history.is_empty() {
            out.push_str("\n  ┌─ Turn Timeline ─────────────────────────────────────────────────────────────\n");
            out.push_str(
                "  │ Turn   Duration    Prompt Tok   Compl Tok   Total Tok   Tools   Status\n",
            );
            out.push_str("  ├─────────────────────────────────────────────────────────────────────────────\n");
            for turn in &self.turns_history {
                out.push_str(&format!(
                    "  │ {:<6} {:<11.1}ms {:<12} {:<11} {:<11} {:<7} {}\n",
                    turn.turn,
                    turn.duration_ms as f64,
                    turn.prompt_tokens,
                    turn.completion_tokens,
                    turn.total_tokens,
                    turn.tool_calls_count,
                    if turn.success { "OK" } else { "ERROR" }
                ));
            }
            out.push_str("  └─────────────────────────────────────────────────────────────────────────────\n");
        }

        if let Some(err) = &self.error {
            out.push_str(&format!("\n  Error: {}\n", err));
        }

        if let Some(prev) = &self.output_preview {
            out.push_str(&format!("\n  Output Preview: {}\n", prev));
        }

        out.push_str(
            "════════════════════════════════════════════════════════════════════════════════\n",
        );
        out
    }
}

// ---------------------------------------------------------------------------
// 5. Role-Level & Fleet-Level Rollup Metrics
// ---------------------------------------------------------------------------

/// Aggregate metrics and telemetry grouped by specialized subagent role.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RoleAggregateMetrics {
    /// Subagent role identifier (e.g. "scout", "coder", "tester", "reviewer").
    pub role: String,
    /// Total number of subagents assigned this role.
    pub agent_count: usize,
    /// Number of successfully completed subagents.
    pub completed_count: usize,
    /// Number of failed subagents.
    pub failed_count: usize,
    /// Number of cancelled subagents.
    pub cancelled_count: usize,
    /// Role-level mission success rate (0.0 to 1.0).
    pub success_rate: f64,
    /// Total execution duration across all agents in milliseconds.
    pub total_duration_ms: u64,
    /// Average execution duration in milliseconds.
    pub avg_duration_ms: f64,
    /// Cumulative prompt tokens consumed.
    pub total_prompt_tokens: u64,
    /// Cumulative completion tokens generated.
    pub total_completion_tokens: u64,
    /// Cumulative total tokens consumed.
    pub total_tokens: u64,
    /// Average tokens per subagent.
    pub avg_tokens_per_agent: f64,
    /// Total estimated cost in USD.
    pub total_cost_usd: f64,
    /// Total tool invocations made by agents with this role.
    pub total_tool_calls: usize,
    /// Total successful tool invocations.
    pub tool_successes: usize,
    /// Total failed tool invocations.
    pub tool_failures: usize,
    /// Tool call success rate for this role.
    pub tool_success_rate: f64,
}

/// Comprehensive fleet-wide telemetry rollup and statistical profiling for subagents.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SubagentFleetMetrics {
    /// Total number of subagents tracked.
    pub total_subagents: usize,
    /// Total successfully completed subagents.
    pub completed_count: usize,
    /// Total failed subagents.
    pub failed_count: usize,
    /// Total cancelled subagents.
    pub cancelled_count: usize,
    /// Currently running subagents.
    pub running_count: usize,
    /// Pending subagents waiting to run.
    pub pending_count: usize,
    /// Overall fleet completion success rate (0.0 to 1.0).
    pub overall_success_rate: f64,
    /// Total cumulative duration across all subagents in milliseconds.
    pub total_duration_ms: u64,
    /// Average execution duration per subagent in milliseconds.
    pub avg_duration_ms: f64,
    /// Minimum observed subagent duration in milliseconds.
    pub min_duration_ms: u64,
    /// Maximum observed subagent duration in milliseconds.
    pub max_duration_ms: u64,
    /// Median (p50) execution duration in milliseconds.
    pub p50_duration_ms: f64,
    /// 90th percentile (p90) execution duration in milliseconds.
    pub p90_duration_ms: f64,
    /// 95th percentile (p95) execution duration in milliseconds.
    pub p95_duration_ms: f64,
    /// 99th percentile (p99) execution duration in milliseconds.
    pub p99_duration_ms: f64,
    /// Cumulative prompt tokens consumed by the fleet.
    pub total_prompt_tokens: u64,
    /// Cumulative completion tokens generated by the fleet.
    pub total_completion_tokens: u64,
    /// Cumulative total tokens consumed by the fleet.
    pub total_tokens: u64,
    /// Average tokens consumed per subagent.
    pub avg_tokens_per_subagent: f64,
    /// Cumulative estimated cost in USD across all subagents.
    pub total_cost_usd: f64,
    /// Total tool invocations across the entire fleet.
    pub total_tool_calls: usize,
    /// Total successful tool invocations.
    pub total_tool_successes: usize,
    /// Total failed tool invocations.
    pub total_tool_failures: usize,
    /// Overall tool success rate (0.0 to 1.0).
    pub tool_success_rate: f64,
    /// Breakdown of metrics grouped by subagent role.
    pub role_breakdown: HashMap<String, RoleAggregateMetrics>,
    /// Fleet-wide aggregation of tool usage and performance.
    pub tool_breakdown: HashMap<String, ToolUsageMetrics>,
}

impl SubagentFleetMetrics {
    /// Formats a clean ASCII summary table of the entire fleet.
    pub fn format_summary_table(&self) -> String {
        let mut out = String::new();
        out.push_str(
            "┌─ Subagent Fleet Summary ───────────────────────────────────────────────────────┐\n",
        );
        out.push_str(&format!(
            "│ Agents: {:<4} Total | {:<3} Succ | {:<3} Fail | {:<3} Cancel | {:<3} Running | Succ Rate: {:<5.1}% │\n",
            self.total_subagents,
            self.completed_count,
            self.failed_count,
            self.cancelled_count,
            self.running_count,
            self.overall_success_rate * 100.0
        ));
        out.push_str(&format!(
            "│ Duration: Total {:.2}s | Avg {:.2}s | p50 {:.2}s | p90 {:.2}s | p95 {:.2}s | max {:.2}s │\n",
            self.total_duration_ms as f64 / 1000.0,
            self.avg_duration_ms / 1000.0,
            self.p50_duration_ms / 1000.0,
            self.p90_duration_ms / 1000.0,
            self.p95_duration_ms / 1000.0,
            self.max_duration_ms as f64 / 1000.0
        ));
        out.push_str(&format!(
            "│ Tokens: {:<7} Total | {:<7} Prompt | {:<7} Compl | Cost: ${:<8.4}          │\n",
            self.total_tokens,
            self.total_prompt_tokens,
            self.total_completion_tokens,
            self.total_cost_usd
        ));
        out.push_str(&format!(
            "│ Tools: {:<5} Calls | {:<4} Succ | {:<4} Fail | Tool Succ Rate: {:<5.1}%             │\n",
            self.total_tool_calls,
            self.total_tool_successes,
            self.total_tool_failures,
            self.tool_success_rate * 100.0
        ));
        out.push_str(
            "└───────────────────────────────────────────────────────────────────────────────┘\n",
        );
        out
    }

    /// Formats an interactive multi-section dashboard string.
    pub fn format_dashboard(&self) -> String {
        let mut out = self.format_summary_table();

        if !self.role_breakdown.is_empty() {
            out.push_str("\n┌─ Role Breakdown ───────────────────────────────────────────────────────────────┐\n");
            out.push_str("│ Role       Agents   Succ   Fail   Rate      Tokens   Avg Dur     Cost     Tool Rate │\n");
            out.push_str("├───────────────────────────────────────────────────────────────────────────────┤\n");
            let mut sorted_roles: Vec<_> = self.role_breakdown.values().collect();
            sorted_roles.sort_by(|a, b| b.agent_count.cmp(&a.agent_count));
            for r in sorted_roles {
                out.push_str(&format!(
                    "│ {:<10} {:<8} {:<6} {:<6} {:<7.1}% {:<8} {:<9.2}s ${:<8.4} {:<7.1}% │\n",
                    r.role,
                    r.agent_count,
                    r.completed_count,
                    r.failed_count,
                    r.success_rate * 100.0,
                    r.total_tokens,
                    r.avg_duration_ms / 1000.0,
                    r.total_cost_usd,
                    r.tool_success_rate * 100.0
                ));
            }
            out.push_str("└───────────────────────────────────────────────────────────────────────────────┘\n");
        }

        if !self.tool_breakdown.is_empty() {
            out.push_str("\n┌─ Aggregate Tool Usage ─────────────────────────────────────────────────────────┐\n");
            out.push_str("│ Tool Name          Invocations   Success   Failure   Success Rate   Avg Latency │\n");
            out.push_str("├───────────────────────────────────────────────────────────────────────────────┤\n");
            let mut sorted_tools: Vec<_> = self.tool_breakdown.values().collect();
            sorted_tools.sort_by(|a, b| b.invocations.cmp(&a.invocations));
            for t in sorted_tools {
                out.push_str(&format!(
                    "│ {:<18} {:<13} {:<9} {:<9} {:<12.1}% {:<9.1}ms │\n",
                    t.tool_name,
                    t.invocations,
                    t.successes,
                    t.failures,
                    t.success_rate() * 100.0,
                    t.avg_duration_ms()
                ));
            }
            out.push_str("└───────────────────────────────────────────────────────────────────────────────┘\n");
        }

        out
    }

    /// Formats a complete Markdown report for inclusion in trace logs or reports.
    pub fn format_markdown_report(&self) -> String {
        let mut md = String::new();
        md.push_str("# Subagent Fleet Telemetry & Profiling Report\n\n");
        md.push_str("## Executive Summary\n\n");
        md.push_str("| Metric | Value |\n");
        md.push_str("| :--- | :--- |\n");
        md.push_str(&format!(
            "| **Total Subagents** | {} |\n",
            self.total_subagents
        ));
        md.push_str(&format!(
            "| **Successful Missions** | {} ({:.1}%) |\n",
            self.completed_count,
            self.overall_success_rate * 100.0
        ));
        md.push_str(&format!(
            "| **Failed Missions** | {} |\n",
            self.failed_count
        ));
        md.push_str(&format!(
            "| **Cancelled Missions** | {} |\n",
            self.cancelled_count
        ));
        md.push_str(&format!(
            "| **Total Execution Duration** | {:.2}s |\n",
            self.total_duration_ms as f64 / 1000.0
        ));
        md.push_str(&format!("| **Latency Percentiles (p50 / p90 / p95 / max)** | {:.2}s / {:.2}s / {:.2}s / {:.2}s |\n", self.p50_duration_ms / 1000.0, self.p90_duration_ms / 1000.0, self.p95_duration_ms / 1000.0, self.max_duration_ms as f64 / 1000.0));
        md.push_str(&format!(
            "| **Tokens (Total / Prompt / Completion)** | {} / {} / {} |\n",
            self.total_tokens, self.total_prompt_tokens, self.total_completion_tokens
        ));
        md.push_str(&format!(
            "| **Average Tokens / Subagent** | {:.0} |\n",
            self.avg_tokens_per_subagent
        ));
        md.push_str(&format!(
            "| **Total Estimated Cost** | ${:.4} |\n",
            self.total_cost_usd
        ));
        md.push_str(&format!(
            "| **Total Tool Invocations** | {} (Success Rate: {:.1}%) |\n\n",
            self.total_tool_calls,
            self.tool_success_rate * 100.0
        ));

        if !self.role_breakdown.is_empty() {
            md.push_str("## Role Performance Breakdown\n\n");
            md.push_str("| Role | Agents | Success Rate | Total Tokens | Avg Duration | Estimated Cost | Tool Success Rate |\n");
            md.push_str("| :--- | :--- | :--- | :--- | :--- | :--- | :--- |\n");
            let mut sorted_roles: Vec<_> = self.role_breakdown.values().collect();
            sorted_roles.sort_by(|a, b| b.agent_count.cmp(&a.agent_count));
            for r in sorted_roles {
                md.push_str(&format!(
                    "| **{}** | {} | {:.1}% | {} | {:.2}s | ${:.4} | {:.1}% |\n",
                    r.role,
                    r.agent_count,
                    r.success_rate * 100.0,
                    r.total_tokens,
                    r.avg_duration_ms / 1000.0,
                    r.total_cost_usd,
                    r.tool_success_rate * 100.0
                ));
            }
            md.push('\n');
        }

        if !self.tool_breakdown.is_empty() {
            md.push_str("## Tool Profiling Breakdown\n\n");
            md.push_str("| Tool Name | Invocations | Successes | Failures | Success Rate | Avg Latency | Total Latency |\n");
            md.push_str("| :--- | :--- | :--- | :--- | :--- | :--- | :--- |\n");
            let mut sorted_tools: Vec<_> = self.tool_breakdown.values().collect();
            sorted_tools.sort_by(|a, b| b.invocations.cmp(&a.invocations));
            for t in sorted_tools {
                md.push_str(&format!(
                    "| `{}` | {} | {} | {} | {:.1}% | {:.1}ms | {:.1}ms |\n",
                    t.tool_name,
                    t.invocations,
                    t.successes,
                    t.failures,
                    t.success_rate() * 100.0,
                    t.avg_duration_ms(),
                    t.total_duration_ms as f64
                ));
            }
            md.push('\n');
        }

        md
    }

    /// Exports summary data as a JSON value.
    pub fn to_json(&self) -> serde_json::Value {
        serde_json::to_value(self).unwrap_or(serde_json::Value::Null)
    }
}

// ---------------------------------------------------------------------------
// 6. Percentile Calculation Helper
// ---------------------------------------------------------------------------

/// Calculates the p-th percentile from a slice of numbers.
///
/// Uses linear interpolation between closest ranks. Handles empty and single-element inputs safely.
pub fn calculate_percentile(values: &[u64], percentile: f64) -> f64 {
    if values.is_empty() {
        return 0.0;
    }
    if values.len() == 1 {
        return values[0] as f64;
    }

    let mut sorted = values.to_vec();
    sorted.sort_unstable();

    let p = percentile.clamp(0.0, 100.0);
    let rank = (p / 100.0) * (sorted.len() - 1) as f64;
    let lower_idx = rank.floor() as usize;
    let upper_idx = rank.ceil() as usize;

    if lower_idx == upper_idx {
        sorted[lower_idx] as f64
    } else {
        let weight = rank - lower_idx as f64;
        sorted[lower_idx] as f64 * (1.0 - weight) + sorted[upper_idx] as f64 * weight
    }
}

// ---------------------------------------------------------------------------
// 7. Subagent Metrics Collector Engine
// ---------------------------------------------------------------------------

/// Internal mutable state for active in-flight subagents.
#[derive(Debug, Default)]
struct CollectorState {
    /// Stored metrics for all recorded subagents (indexed by ID).
    subagents: HashMap<String, SubagentMetrics>,
    /// High-resolution starting monotonic instants for active subagents.
    agent_start_instants: HashMap<String, Instant>,
    /// High-resolution starting monotonic instants for active turns: (turn_index, Instant).
    turn_start_instants: HashMap<String, (usize, Instant)>,
    /// High-resolution starting monotonic instants for active tools: agent_id -> tool_key -> (tool_name, Instant).
    tool_start_instants: HashMap<String, HashMap<String, (String, Instant)>>,
    /// In-flight turn metric accumulators being built before turn finish.
    in_flight_turns: HashMap<String, TurnMetric>,
}

/// Thread-safe metrics collector, aggregator, and profiler for subagents.
#[derive(Debug, Clone)]
pub struct SubagentMetricsCollector {
    state: Arc<Mutex<CollectorState>>,
}

impl Default for SubagentMetricsCollector {
    fn default() -> Self {
        Self::new()
    }
}

impl SubagentMetricsCollector {
    /// Creates a new empty `SubagentMetricsCollector`.
    pub fn new() -> Self {
        Self {
            state: Arc::new(Mutex::new(CollectorState::default())),
        }
    }

    // --- Subagent Lifecycle Hooks ---

    /// Registers the start of a new subagent.
    pub fn start_subagent(
        &self,
        id: impl Into<String>,
        name: impl Into<String>,
        role: impl Into<String>,
        task: impl Into<String>,
        model: impl Into<String>,
        max_turns: usize,
    ) {
        let id_str = id.into();
        let name_str = name.into();
        let role_str = role.into();
        let task_str = task.into();
        let model_str = model.into();

        let mut metrics = SubagentMetrics::new(
            id_str.clone(),
            name_str,
            role_str,
            task_str,
            model_str,
            max_turns,
        );
        metrics.status = SubagentMetricStatus::Running;

        let mut lock = self.state.lock().unwrap();
        lock.subagents.insert(id_str.clone(), metrics);
        lock.agent_start_instants.insert(id_str, Instant::now());
    }

    /// Registers the start of a new turn for a subagent.
    pub fn start_turn(&self, id: &str, turn: usize) {
        let mut lock = self.state.lock().unwrap();
        if let Some(agent) = lock.subagents.get_mut(id) {
            agent.turns = agent.turns.max(turn);
            agent.status = SubagentMetricStatus::Running;
        }
        lock.turn_start_instants
            .insert(id.to_string(), (turn, Instant::now()));
        lock.in_flight_turns
            .insert(id.to_string(), TurnMetric::new(turn));
    }

    /// Records token consumption for the active turn and subagent.
    pub fn record_tokens(&self, id: &str, prompt: u64, completion: u64) {
        let mut lock = self.state.lock().unwrap();
        if let Some(agent) = lock.subagents.get_mut(id) {
            agent.prompt_tokens = agent.prompt_tokens.saturating_add(prompt);
            agent.completion_tokens = agent.completion_tokens.saturating_add(completion);
            agent.total_tokens = agent
                .total_tokens
                .saturating_add(prompt)
                .saturating_add(completion);
            // Default provider update if possible
            agent.update_cost("openrouter");
        }
        if let Some(turn) = lock.in_flight_turns.get_mut(id) {
            turn.prompt_tokens = turn.prompt_tokens.saturating_add(prompt);
            turn.completion_tokens = turn.completion_tokens.saturating_add(completion);
            turn.total_tokens = turn
                .total_tokens
                .saturating_add(prompt)
                .saturating_add(completion);
        }
    }

    /// Accumulates thinking / reasoning delta text tokens and chars.
    pub fn record_thinking_delta(&self, id: &str, delta: &str) {
        let chars = delta.chars().count();
        let tokens = estimate_text_tokens(delta) as u64;

        let mut lock = self.state.lock().unwrap();
        if let Some(turn) = lock.in_flight_turns.get_mut(id) {
            turn.reasoning_chars = turn.reasoning_chars.saturating_add(chars);
            turn.completion_tokens = turn.completion_tokens.saturating_add(tokens);
            turn.total_tokens = turn.total_tokens.saturating_add(tokens);
        }
        if let Some(agent) = lock.subagents.get_mut(id) {
            agent.completion_tokens = agent.completion_tokens.saturating_add(tokens);
            agent.total_tokens = agent.total_tokens.saturating_add(tokens);
            agent.update_cost("openrouter");
        }
    }

    /// Accumulates assistant message delta text tokens and chars.
    pub fn record_message_delta(&self, id: &str, delta: &str) {
        let chars = delta.chars().count();
        let tokens = estimate_text_tokens(delta) as u64;

        let mut lock = self.state.lock().unwrap();
        if let Some(turn) = lock.in_flight_turns.get_mut(id) {
            turn.content_chars = turn.content_chars.saturating_add(chars);
            turn.completion_tokens = turn.completion_tokens.saturating_add(tokens);
            turn.total_tokens = turn.total_tokens.saturating_add(tokens);
        }
        if let Some(agent) = lock.subagents.get_mut(id) {
            agent.completion_tokens = agent.completion_tokens.saturating_add(tokens);
            agent.total_tokens = agent.total_tokens.saturating_add(tokens);
            agent.update_cost("openrouter");
        }
    }

    /// Registers the start of a tool invocation.
    pub fn start_tool(&self, id: &str, tool_name: &str, call_key: Option<&str>) {
        let key = call_key.unwrap_or(tool_name).to_string();
        let mut lock = self.state.lock().unwrap();
        let entry = lock.tool_start_instants.entry(id.to_string()).or_default();
        entry.insert(key, (tool_name.to_string(), Instant::now()));
    }

    /// Registers the completion of a tool invocation.
    pub fn finish_tool(
        &self,
        id: &str,
        tool_name: &str,
        call_key: Option<&str>,
        output_bytes: usize,
        success: bool,
        error: Option<String>,
    ) {
        let key = call_key.unwrap_or(tool_name);
        let mut lock = self.state.lock().unwrap();

        let duration_ms = if let Some(map) = lock.tool_start_instants.get_mut(id) {
            if let Some((_, start)) = map.remove(key) {
                start.elapsed().as_millis() as u64
            } else {
                0
            }
        } else {
            0
        };

        let current_turn = lock
            .turn_start_instants
            .get(id)
            .map(|(t, _)| *t)
            .unwrap_or(1);

        let sample = ToolCallSample {
            call_id: key.to_string(),
            tool_name: tool_name.to_string(),
            turn: current_turn,
            started_at: Utc::now().to_rfc3339(),
            duration_ms,
            success,
            output_bytes,
            error,
        };

        if let Some(agent) = lock.subagents.get_mut(id) {
            let tool_metric = agent
                .tool_metrics
                .entry(tool_name.to_string())
                .or_insert_with(|| ToolUsageMetrics::new(tool_name));
            tool_metric.record(duration_ms, success, output_bytes);
            agent.tool_calls_history.push(sample.clone());
        }

        if let Some(turn) = lock.in_flight_turns.get_mut(id) {
            turn.tool_calls_count += 1;
            turn.tool_calls.push(sample);
        }
    }

    /// Registers the completion of an active turn.
    pub fn finish_turn(&self, id: &str, success: bool, error: Option<String>) {
        let mut lock = self.state.lock().unwrap();

        let duration_ms = if let Some((_, start)) = lock.turn_start_instants.remove(id) {
            start.elapsed().as_millis() as u64
        } else {
            0
        };

        if let Some(mut turn_metric) = lock.in_flight_turns.remove(id) {
            turn_metric.duration_ms = duration_ms;
            turn_metric.success = success;
            turn_metric.error = error;

            if let Some(agent) = lock.subagents.get_mut(id) {
                agent.turns_history.push(turn_metric);
            }
        }
    }

    /// Marks a subagent as successfully completed.
    pub fn complete_subagent(&self, id: &str, output: &str, turns_taken: usize) {
        let mut lock = self.state.lock().unwrap();

        let duration_ms = if let Some(start) = lock.agent_start_instants.remove(id) {
            start.elapsed().as_millis() as u64
        } else {
            0
        };

        // Clean up any remaining in-flight turn
        if let Some((_, start)) = lock.turn_start_instants.remove(id) {
            let turn_dur = start.elapsed().as_millis() as u64;
            if let Some(mut turn_metric) = lock.in_flight_turns.remove(id) {
                turn_metric.duration_ms = turn_dur;
                if let Some(agent) = lock.subagents.get_mut(id) {
                    agent.turns_history.push(turn_metric);
                }
            }
        }
        lock.tool_start_instants.remove(id);

        if let Some(agent) = lock.subagents.get_mut(id) {
            agent.status = SubagentMetricStatus::Completed;
            agent.completed_at = Some(Utc::now().to_rfc3339());
            agent.duration_ms = duration_ms;
            agent.turns = turns_taken;

            let preview_len = 256.min(output.len());
            let preview = if output.len() > preview_len {
                format!("{}...", &output[..preview_len])
            } else {
                output.to_string()
            };
            agent.output_preview = Some(preview);
        }
    }

    /// Marks a subagent as failed with an error message.
    pub fn fail_subagent(&self, id: &str, error: &str) {
        let mut lock = self.state.lock().unwrap();

        let duration_ms = if let Some(start) = lock.agent_start_instants.remove(id) {
            start.elapsed().as_millis() as u64
        } else {
            0
        };

        lock.turn_start_instants.remove(id);
        lock.in_flight_turns.remove(id);
        lock.tool_start_instants.remove(id);

        if let Some(agent) = lock.subagents.get_mut(id) {
            agent.status = SubagentMetricStatus::Failed;
            agent.completed_at = Some(Utc::now().to_rfc3339());
            agent.duration_ms = duration_ms;
            agent.error = Some(error.to_string());
        }
    }

    /// Marks a subagent as cancelled.
    pub fn cancel_subagent(&self, id: &str) {
        let mut lock = self.state.lock().unwrap();

        let duration_ms = if let Some(start) = lock.agent_start_instants.remove(id) {
            start.elapsed().as_millis() as u64
        } else {
            0
        };

        lock.turn_start_instants.remove(id);
        lock.in_flight_turns.remove(id);
        lock.tool_start_instants.remove(id);

        if let Some(agent) = lock.subagents.get_mut(id) {
            agent.status = SubagentMetricStatus::Cancelled;
            agent.completed_at = Some(Utc::now().to_rfc3339());
            agent.duration_ms = duration_ms;
        }
    }

    // --- Progress Event Observer ---

    /// Automatically records telemetry by observing a [`SubagentProgress`] event.
    pub fn observe_event(&self, event: &SubagentProgress) {
        match event {
            SubagentProgress::Started {
                id,
                name,
                role,
                task,
            } => {
                self.start_subagent(id, name, role.to_string(), task, "default", 20);
            }
            SubagentProgress::TurnStarted {
                id,
                turn,
                max_turns,
            } => {
                let mut lock = self.state.lock().unwrap();
                if let Some(agent) = lock.subagents.get_mut(id) {
                    agent.max_turns = *max_turns;
                }
                drop(lock);
                self.start_turn(id, *turn);
            }
            SubagentProgress::Thinking { id, delta } => {
                self.record_thinking_delta(id, delta);
            }
            SubagentProgress::Message { id, content } => {
                self.record_message_delta(id, content);
            }
            SubagentProgress::ToolStarted { id, tool, args } => {
                let call_key =
                    format!("{}_{}", tool, Utc::now().timestamp_nanos_opt().unwrap_or(0));
                let arg_bytes = serde_json::to_vec(args).map(|b| b.len()).unwrap_or(0);
                self.start_tool(id, tool, Some(&call_key));
                // Add estimated prompt tokens for tool call
                self.record_tokens(id, (arg_bytes / 4).max(1) as u64, 0);
            }
            SubagentProgress::ToolCompleted {
                id,
                tool,
                output,
                success,
            } => {
                let output_bytes = output.len();
                let err_opt = if *success { None } else { Some(output.clone()) };
                self.finish_tool(id, tool, None, output_bytes, *success, err_opt);
            }
            SubagentProgress::Completed {
                id,
                output,
                turns_taken,
            } => {
                self.complete_subagent(id, output, *turns_taken);
            }
            SubagentProgress::Failed { id, error } => {
                self.fail_subagent(id, error);
            }
            SubagentProgress::Cancelled { id } => {
                self.cancel_subagent(id);
            }
        }
    }

    /// Spawns a background task that listens to a broadcast channel of [`SubagentProgress`]
    /// events and automatically ingests them into this collector.
    pub fn spawn_event_listener(
        collector: Arc<Self>,
        mut rx: broadcast::Receiver<SubagentProgress>,
    ) -> tokio::task::JoinHandle<()> {
        tokio::spawn(async move {
            while let Ok(event) = rx.recv().await {
                collector.observe_event(&event);
            }
        })
    }

    // --- Querying & Filtering ---

    /// Returns a cloned copy of the metrics for a specific subagent.
    pub fn get(&self, id: &str) -> Option<SubagentMetrics> {
        let lock = self.state.lock().unwrap();
        lock.subagents.get(id).cloned()
    }

    /// Returns all tracked subagent metrics.
    pub fn all(&self) -> Vec<SubagentMetrics> {
        let lock = self.state.lock().unwrap();
        lock.subagents.values().cloned().collect()
    }

    /// Returns all subagents currently in the `Running` state.
    pub fn active(&self) -> Vec<SubagentMetrics> {
        let lock = self.state.lock().unwrap();
        lock.subagents
            .values()
            .filter(|a| a.status == SubagentMetricStatus::Running)
            .cloned()
            .collect()
    }

    /// Returns all successfully completed subagents.
    pub fn completed(&self) -> Vec<SubagentMetrics> {
        let lock = self.state.lock().unwrap();
        lock.subagents
            .values()
            .filter(|a| a.status == SubagentMetricStatus::Completed)
            .cloned()
            .collect()
    }

    /// Returns all failed subagents.
    pub fn failed(&self) -> Vec<SubagentMetrics> {
        let lock = self.state.lock().unwrap();
        lock.subagents
            .values()
            .filter(|a| a.status == SubagentMetricStatus::Failed)
            .cloned()
            .collect()
    }

    /// Filters subagents by role name (case-insensitive).
    pub fn by_role(&self, role: &str) -> Vec<SubagentMetrics> {
        let role_lower = role.to_lowercase();
        let lock = self.state.lock().unwrap();
        lock.subagents
            .values()
            .filter(|a| a.role.to_lowercase() == role_lower)
            .cloned()
            .collect()
    }

    /// Computes fleet-wide statistical aggregation across all recorded subagents.
    pub fn fleet_summary(&self) -> SubagentFleetMetrics {
        let lock = self.state.lock().unwrap();
        let agents: Vec<&SubagentMetrics> = lock.subagents.values().collect();

        let total_subagents = agents.len();
        let mut completed_count = 0;
        let mut failed_count = 0;
        let mut cancelled_count = 0;
        let mut running_count = 0;
        let mut pending_count = 0;

        let mut total_duration_ms: u64 = 0;
        let mut durations: Vec<u64> = Vec::with_capacity(total_subagents);

        let mut total_prompt_tokens: u64 = 0;
        let mut total_completion_tokens: u64 = 0;
        let mut total_tokens: u64 = 0;
        let mut total_cost_usd: f64 = 0.0;

        let mut total_tool_calls: usize = 0;
        let mut total_tool_successes: usize = 0;
        let mut total_tool_failures: usize = 0;

        let mut role_map: HashMap<String, Vec<&SubagentMetrics>> = HashMap::new();
        let mut tool_breakdown: HashMap<String, ToolUsageMetrics> = HashMap::new();

        for a in &agents {
            match a.status {
                SubagentMetricStatus::Completed => completed_count += 1,
                SubagentMetricStatus::Failed | SubagentMetricStatus::TimedOut => failed_count += 1,
                SubagentMetricStatus::Cancelled => cancelled_count += 1,
                SubagentMetricStatus::Running => running_count += 1,
                SubagentMetricStatus::Pending => pending_count += 1,
            }

            total_duration_ms = total_duration_ms.saturating_add(a.duration_ms);
            durations.push(a.duration_ms);

            total_prompt_tokens = total_prompt_tokens.saturating_add(a.prompt_tokens);
            total_completion_tokens = total_completion_tokens.saturating_add(a.completion_tokens);
            total_tokens = total_tokens.saturating_add(a.total_tokens);
            total_cost_usd += a.estimated_cost_usd;

            total_tool_calls += a.total_tool_calls();
            total_tool_successes += a.total_tool_successes();
            total_tool_failures += a.total_tool_failures();

            role_map.entry(a.role.clone()).or_default().push(a);

            for (name, tm) in &a.tool_metrics {
                let entry = tool_breakdown
                    .entry(name.clone())
                    .or_insert_with(|| ToolUsageMetrics::new(name));
                entry.invocations += tm.invocations;
                entry.successes += tm.successes;
                entry.failures += tm.failures;
                entry.total_duration_ms =
                    entry.total_duration_ms.saturating_add(tm.total_duration_ms);
                entry.total_output_bytes = entry
                    .total_output_bytes
                    .saturating_add(tm.total_output_bytes);
                if entry.min_duration_ms == 0
                    || (tm.min_duration_ms > 0 && tm.min_duration_ms < entry.min_duration_ms)
                {
                    entry.min_duration_ms = tm.min_duration_ms;
                }
                entry.max_duration_ms = entry.max_duration_ms.max(tm.max_duration_ms);
            }
        }

        let overall_success_rate = if total_subagents == 0 {
            1.0
        } else {
            completed_count as f64 / total_subagents as f64
        };

        let avg_duration_ms = if total_subagents == 0 {
            0.0
        } else {
            total_duration_ms as f64 / total_subagents as f64
        };

        let min_duration_ms = durations.iter().copied().min().unwrap_or(0);
        let max_duration_ms = durations.iter().copied().max().unwrap_or(0);
        let p50_duration_ms = calculate_percentile(&durations, 50.0);
        let p90_duration_ms = calculate_percentile(&durations, 90.0);
        let p95_duration_ms = calculate_percentile(&durations, 95.0);
        let p99_duration_ms = calculate_percentile(&durations, 99.0);

        let avg_tokens_per_subagent = if total_subagents == 0 {
            0.0
        } else {
            total_tokens as f64 / total_subagents as f64
        };

        let tool_success_rate = if total_tool_calls == 0 {
            1.0
        } else {
            total_tool_successes as f64 / total_tool_calls as f64
        };

        let mut role_breakdown = HashMap::new();
        for (role_name, role_agents) in role_map {
            let count = role_agents.len();
            let succ = role_agents
                .iter()
                .filter(|a| a.status == SubagentMetricStatus::Completed)
                .count();
            let fail = role_agents
                .iter()
                .filter(|a| {
                    a.status == SubagentMetricStatus::Failed
                        || a.status == SubagentMetricStatus::TimedOut
                })
                .count();
            let canc = role_agents
                .iter()
                .filter(|a| a.status == SubagentMetricStatus::Cancelled)
                .count();

            let dur: u64 = role_agents.iter().map(|a| a.duration_ms).sum();
            let p_tok: u64 = role_agents.iter().map(|a| a.prompt_tokens).sum();
            let c_tok: u64 = role_agents.iter().map(|a| a.completion_tokens).sum();
            let t_tok: u64 = role_agents.iter().map(|a| a.total_tokens).sum();
            let cost: f64 = role_agents.iter().map(|a| a.estimated_cost_usd).sum();

            let t_calls: usize = role_agents.iter().map(|a| a.total_tool_calls()).sum();
            let t_succ: usize = role_agents.iter().map(|a| a.total_tool_successes()).sum();
            let t_fail: usize = role_agents.iter().map(|a| a.total_tool_failures()).sum();

            let r_succ_rate = if count == 0 {
                1.0
            } else {
                succ as f64 / count as f64
            };
            let r_tool_rate = if t_calls == 0 {
                1.0
            } else {
                t_succ as f64 / t_calls as f64
            };

            role_breakdown.insert(
                role_name.clone(),
                RoleAggregateMetrics {
                    role: role_name,
                    agent_count: count,
                    completed_count: succ,
                    failed_count: fail,
                    cancelled_count: canc,
                    success_rate: r_succ_rate,
                    total_duration_ms: dur,
                    avg_duration_ms: if count == 0 {
                        0.0
                    } else {
                        dur as f64 / count as f64
                    },
                    total_prompt_tokens: p_tok,
                    total_completion_tokens: c_tok,
                    total_tokens: t_tok,
                    avg_tokens_per_agent: if count == 0 {
                        0.0
                    } else {
                        t_tok as f64 / count as f64
                    },
                    total_cost_usd: cost,
                    total_tool_calls: t_calls,
                    tool_successes: t_succ,
                    tool_failures: t_fail,
                    tool_success_rate: r_tool_rate,
                },
            );
        }

        SubagentFleetMetrics {
            total_subagents,
            completed_count,
            failed_count,
            cancelled_count,
            running_count,
            pending_count,
            overall_success_rate,
            total_duration_ms,
            avg_duration_ms,
            min_duration_ms,
            max_duration_ms,
            p50_duration_ms,
            p90_duration_ms,
            p95_duration_ms,
            p99_duration_ms,
            total_prompt_tokens,
            total_completion_tokens,
            total_tokens,
            avg_tokens_per_subagent,
            total_cost_usd,
            total_tool_calls,
            total_tool_successes,
            total_tool_failures,
            tool_success_rate,
            role_breakdown,
            tool_breakdown,
        }
    }

    /// Resets and clears all collected metrics.
    pub fn clear(&self) {
        let mut lock = self.state.lock().unwrap();
        *lock = CollectorState::default();
    }

    /// Removes a specific subagent metric by ID.
    pub fn remove(&self, id: &str) -> Option<SubagentMetrics> {
        let mut lock = self.state.lock().unwrap();
        lock.subagents.remove(id)
    }

    // --- Formatted Exports ---

    /// Generates a formatted ASCII table of all tracked subagents.
    pub fn export_table(&self) -> String {
        let subagents = self.all();
        if subagents.is_empty() {
            return "No subagent metrics recorded.\n".to_string();
        }

        let mut out = String::new();
        out.push_str("┌──────────────────────────────────────────────────────────────────────────────────────────────────┐\n");
        out.push_str("│ ID       Name         Role       Status      Duration   Turns   Tokens   Tools (Succ%)   Cost    │\n");
        out.push_str("├──────────────────────────────────────────────────────────────────────────────────────────────────┤\n");
        for a in &subagents {
            out.push_str(&format!(
                "│ {:<8} {:<12} {:<10} {:<11} {:<7.2}s   {:<5}   {:<8} {:<4} ({:<5.1}%)   ${:<7.4} │\n",
                a.id,
                if a.name.len() > 12 { &a.name[..12] } else { &a.name },
                if a.role.len() > 10 { &a.role[..10] } else { &a.role },
                a.status.to_string(),
                a.duration_ms as f64 / 1000.0,
                format!("{}/{}", a.turns, a.max_turns),
                a.total_tokens,
                a.total_tool_calls(),
                a.tool_success_rate() * 100.0,
                a.estimated_cost_usd
            ));
        }
        out.push_str("└──────────────────────────────────────────────────────────────────────────────────────────────────┘\n");
        out
    }

    /// Exports all metrics as a serialized JSON string.
    pub fn export_json(&self) -> Result<String, serde_json::Error> {
        let lock = self.state.lock().unwrap();
        serde_json::to_string_pretty(&lock.subagents)
    }

    /// Exports summary telemetry in CSV format.
    pub fn export_csv(&self) -> String {
        let mut csv = String::new();
        csv.push_str("id,name,role,model,status,duration_ms,turns,max_turns,prompt_tokens,completion_tokens,total_tokens,cost_usd,tool_calls,tool_successes,tool_failures,tool_success_rate\n");
        for a in self.all() {
            csv.push_str(&format!(
                "{},\"{}\",\"{}\",\"{}\",{},{},{},{},{},{},{},{:.6},{},{},{},{:.4}\n",
                a.id,
                a.name.replace('\"', "\"\""),
                a.role.replace('\"', "\"\""),
                a.model.replace('\"', "\"\""),
                a.status,
                a.duration_ms,
                a.turns,
                a.max_turns,
                a.prompt_tokens,
                a.completion_tokens,
                a.total_tokens,
                a.estimated_cost_usd,
                a.total_tool_calls(),
                a.total_tool_successes(),
                a.total_tool_failures(),
                a.tool_success_rate()
            ));
        }
        csv
    }
}

// ---------------------------------------------------------------------------
// 8. Unit Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_subagent_metric_status_mapping() {
        assert_eq!(
            SubagentMetricStatus::from_subagent_status(&SubagentStatus::Pending),
            SubagentMetricStatus::Pending
        );
        assert_eq!(
            SubagentMetricStatus::from_subagent_status(&SubagentStatus::Running {
                turn: 1,
                current_tool: None
            }),
            SubagentMetricStatus::Running
        );
        assert_eq!(
            SubagentMetricStatus::from_subagent_status(&SubagentStatus::Completed {
                output: "ok".into(),
                turns: 2
            }),
            SubagentMetricStatus::Completed
        );
        assert_eq!(
            SubagentMetricStatus::from_subagent_status(&SubagentStatus::Failed {
                error: "err".into()
            }),
            SubagentMetricStatus::Failed
        );
        assert_eq!(
            SubagentMetricStatus::from_subagent_status(&SubagentStatus::Cancelled),
            SubagentMetricStatus::Cancelled
        );

        assert!(SubagentMetricStatus::Completed.is_terminal());
        assert!(SubagentMetricStatus::Completed.is_success());
        assert!(!SubagentMetricStatus::Completed.is_failure());

        assert!(SubagentMetricStatus::Failed.is_terminal());
        assert!(!SubagentMetricStatus::Failed.is_success());
        assert!(SubagentMetricStatus::Failed.is_failure());
    }

    #[test]
    fn test_tool_usage_metrics_recording() {
        let mut tool = ToolUsageMetrics::new("grep");
        assert_eq!(tool.invocations, 0);
        assert_eq!(tool.success_rate(), 1.0);
        assert_eq!(tool.avg_duration_ms(), 0.0);

        tool.record(100, true, 500);
        assert_eq!(tool.invocations, 1);
        assert_eq!(tool.successes, 1);
        assert_eq!(tool.failures, 0);
        assert_eq!(tool.min_duration_ms, 100);
        assert_eq!(tool.max_duration_ms, 100);
        assert_eq!(tool.total_duration_ms, 100);
        assert_eq!(tool.avg_duration_ms(), 100.0);
        assert_eq!(tool.success_rate(), 1.0);

        tool.record(200, false, 50);
        assert_eq!(tool.invocations, 2);
        assert_eq!(tool.successes, 1);
        assert_eq!(tool.failures, 1);
        assert_eq!(tool.min_duration_ms, 100);
        assert_eq!(tool.max_duration_ms, 200);
        assert_eq!(tool.total_duration_ms, 300);
        assert_eq!(tool.avg_duration_ms(), 150.0);
        assert_eq!(tool.success_rate(), 0.5);
        assert_eq!(tool.failure_rate(), 0.5);
        assert_eq!(tool.avg_output_bytes(), 275.0);
    }

    #[test]
    fn test_percentile_calculation() {
        assert_eq!(calculate_percentile(&[], 50.0), 0.0);
        assert_eq!(calculate_percentile(&[42], 90.0), 42.0);

        let data = vec![10, 20, 30, 40, 50, 60, 70, 80, 90, 100];
        assert_eq!(calculate_percentile(&data, 0.0), 10.0);
        assert_eq!(calculate_percentile(&data, 50.0), 55.0);
        assert_eq!(calculate_percentile(&data, 100.0), 100.0);
    }

    #[test]
    fn test_subagent_lifecycle_collection() {
        let collector = SubagentMetricsCollector::new();

        collector.start_subagent(
            "sub1",
            "ScoutAgent",
            "Scout",
            "Find tests",
            "claude-3-5",
            10,
        );
        assert_eq!(collector.active().len(), 1);

        collector.start_turn("sub1", 1);
        collector.record_tokens("sub1", 500, 150);
        collector.start_tool("sub1", "glob", Some("call_1"));
        collector.finish_tool("sub1", "glob", Some("call_1"), 1024, true, None);
        collector.finish_turn("sub1", true, None);

        collector.complete_subagent("sub1", "Found 12 test files.", 1);

        let m = collector.get("sub1").expect("should exist");
        assert_eq!(m.name, "ScoutAgent");
        assert_eq!(m.role, "Scout");
        assert_eq!(m.turns, 1);
        assert_eq!(m.prompt_tokens, 500);
        assert_eq!(m.completion_tokens, 150);
        assert_eq!(m.total_tokens, 650);
        assert_eq!(m.status, SubagentMetricStatus::Completed);
        assert!(m.is_success());
        assert_eq!(m.total_tool_calls(), 1);
        assert_eq!(m.tool_success_rate(), 1.0);

        let profile = m.format_detailed_profile();
        assert!(profile.contains("SUBAGENT PROFILE: ScoutAgent"));
        assert!(profile.contains("Role:             Scout"));
    }

    #[test]
    fn test_event_driven_observer() {
        let collector = SubagentMetricsCollector::new();

        collector.observe_event(&SubagentProgress::Started {
            id: "agent_42".into(),
            name: "CoderBot".into(),
            role: SubagentRole::Coder,
            task: "Implement feature".into(),
        });

        collector.observe_event(&SubagentProgress::TurnStarted {
            id: "agent_42".into(),
            turn: 1,
            max_turns: 20,
        });

        collector.observe_event(&SubagentProgress::Thinking {
            id: "agent_42".into(),
            delta: "Analyzing requirements...".into(),
        });

        collector.observe_event(&SubagentProgress::Message {
            id: "agent_42".into(),
            content: "I will edit the source code.".into(),
        });

        collector.observe_event(&SubagentProgress::ToolStarted {
            id: "agent_42".into(),
            tool: "edit".into(),
            args: serde_json::json!({"path": "src/main.rs"}),
        });

        collector.observe_event(&SubagentProgress::ToolCompleted {
            id: "agent_42".into(),
            tool: "edit".into(),
            output: "Applied 1 edit".into(),
            success: true,
        });

        collector.observe_event(&SubagentProgress::Completed {
            id: "agent_42".into(),
            output: "Feature implemented".into(),
            turns_taken: 1,
        });

        let m = collector.get("agent_42").expect("agent must exist");
        assert_eq!(m.name, "CoderBot");
        assert_eq!(m.role, "Coder");
        assert_eq!(m.status, SubagentMetricStatus::Completed);
        assert!(m.completion_tokens > 0);
        assert_eq!(m.total_tool_calls(), 1);
        assert_eq!(m.total_tool_successes(), 1);
    }

    #[test]
    fn test_fleet_metrics_summary() {
        let collector = SubagentMetricsCollector::new();

        // Agent 1: Success
        collector.start_subagent("a1", "Scout1", "Scout", "Task 1", "gpt-4", 10);
        collector.record_tokens("a1", 100, 50);
        collector.complete_subagent("a1", "done", 1);

        // Agent 2: Failed
        collector.start_subagent("a2", "Coder1", "Coder", "Task 2", "gpt-4", 10);
        collector.record_tokens("a2", 200, 100);
        collector.fail_subagent("a2", "syntax error");

        // Agent 3: Cancelled
        collector.start_subagent("a3", "Tester1", "Tester", "Task 3", "gpt-4", 10);
        collector.cancel_subagent("a3");

        let fleet = collector.fleet_summary();
        assert_eq!(fleet.total_subagents, 3);
        assert_eq!(fleet.completed_count, 1);
        assert_eq!(fleet.failed_count, 1);
        assert_eq!(fleet.cancelled_count, 1);
        assert_eq!(fleet.total_tokens, 450);
        assert_eq!(fleet.role_breakdown.len(), 3);

        let table = fleet.format_summary_table();
        assert!(table.contains("Subagent Fleet Summary"));

        let md = fleet.format_markdown_report();
        assert!(md.contains("# Subagent Fleet Telemetry & Profiling Report"));

        let csv = collector.export_csv();
        assert!(csv.contains("Scout1"));
        assert!(csv.contains("Coder1"));
    }
}
