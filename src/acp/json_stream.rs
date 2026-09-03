//! Structured JSONL / NDJSON Event Logger and Streamer.
//!
//! Provides a high-performance, newline-delimited JSON (NDJSON) event streaming engine
//! and structured logger for editor plugins (Zed, VS Code, JetBrains, Neovim) and headless automation
//! (CI/CD test harnesses, automated benchmarks, subagent pipelines, external script integrations).
//!
//! # Architecture
//!
//! ```text
//!  ┌───────────────────────────────┐
//!  │ AgentRunner / AcpEventBridge  │
//!  └──────────────┬────────────────┘
//!                 │ AgentEvent / AcpSessionEvent
//!                 ▼
//!  ┌───────────────────────────────┐
//!  │         JsonLogEvent          │ (Normalized Envelope: seq, ts, kind, payload)
//!  └───────┬──────────────┬────────┘
//!          │              │
//!          ▼              ▼
//!   ┌─────────────┐ ┌──────────────┐
//!   │ JsonStreamer│ │FileLogger    │
//!   │ (AsyncWrite)│ │(.jsonl file) │
//!   └──────┬──────┘ └──────────────┘
//!          │
//!          ▼
//!   ┌─────────────────────────────┐
//!   │ Stdout / Socket / Pipe      │ -> Editor Plugin / Headless Runner
//!   └─────────────────────────────┘
//! ```

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncBufRead, AsyncBufReadExt, AsyncWrite, AsyncWriteExt, BufWriter};
use tokio::sync::broadcast;

use crate::acp::events::{AcpSessionEvent, AdvisorConsensus, ToolExecutionState};
use crate::agent::loop_runner::AgentEvent;

/// Returns current epoch time in milliseconds.
#[inline]
fn current_epoch_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or(Duration::ZERO)
        .as_millis() as u64
}

/// Returns formatted ISO 8601 UTC timestamp.
#[inline]
fn current_iso8601() -> String {
    let now = SystemTime::now();
    let dt: DateTime<Utc> = now.into();
    dt.to_rfc3339()
}

// ============================================================================
// Event Kinds and Top-Level Event Envelope
// ============================================================================

/// Semantic event classification for structured JSONL logging and NDJSON streaming.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum JsonLogEventKind {
    /// Agent or ACP session initialized.
    SessionStart,
    /// Agent or ACP session terminated.
    SessionEnd,
    /// Turn/prompt execution initiated.
    TurnStart,
    /// Turn/prompt execution finished.
    TurnEnd,
    /// Incremental assistant text chunk.
    TextDelta,
    /// Incremental model reasoning / thought chunk.
    ThinkingDelta,
    /// Tool execution scheduled or started.
    ToolStart,
    /// Incremental progress or partial output from tool.
    ToolProgress,
    /// Tool execution completed.
    ToolFinish,
    /// Automated advisor review started.
    AdvisorStart,
    /// Automated advisor feedback or critique emitted.
    AdvisorFeedback,
    /// Consensus verdict reached across multiple advisors.
    AdvisorConsensus,
    /// Subagent task delegation started.
    SubagentStart,
    /// Subagent task delegation completed.
    SubagentFinish,
    /// Execution plan progress update.
    PlanUpdate,
    /// Real-time token usage and throughput metrics.
    TokenStats,
    /// General status or diagnostic log message.
    Status,
    /// Execution or provider error notification.
    Error,
    /// Raw unparsed ACP session event wrapper.
    RawAcp,
    /// Custom event with user-defined JSON payload.
    Custom,
}

impl std::fmt::Display for JsonLogEventKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::SessionStart => write!(f, "session_start"),
            Self::SessionEnd => write!(f, "session_end"),
            Self::TurnStart => write!(f, "turn_start"),
            Self::TurnEnd => write!(f, "turn_end"),
            Self::TextDelta => write!(f, "text_delta"),
            Self::ThinkingDelta => write!(f, "thinking_delta"),
            Self::ToolStart => write!(f, "tool_start"),
            Self::ToolProgress => write!(f, "tool_progress"),
            Self::ToolFinish => write!(f, "tool_finish"),
            Self::AdvisorStart => write!(f, "advisor_start"),
            Self::AdvisorFeedback => write!(f, "advisor_feedback"),
            Self::AdvisorConsensus => write!(f, "advisor_consensus"),
            Self::SubagentStart => write!(f, "subagent_start"),
            Self::SubagentFinish => write!(f, "subagent_finish"),
            Self::PlanUpdate => write!(f, "plan_update"),
            Self::TokenStats => write!(f, "token_stats"),
            Self::Status => write!(f, "status"),
            Self::Error => write!(f, "error"),
            Self::RawAcp => write!(f, "raw_acp"),
            Self::Custom => write!(f, "custom"),
        }
    }
}

/// The standard structured JSONL / NDJSON event envelope.
///
/// Designed for high compatibility with editor plugins (Zed, VS Code, JetBrains, Neovim)
/// and headless automation pipelines.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct JsonLogEvent {
    /// Monotonically increasing sequence number for ordering and gap detection.
    pub seq: u64,
    /// ISO 8601 UTC timestamp string (e.g. "2026-09-02T14:30:00.123Z").
    pub timestamp: String,
    /// Unix timestamp in milliseconds for fast arithmetic.
    pub timestamp_ms: u64,
    /// Optional session identifier.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    /// Optional turn/request identifier.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub turn_id: Option<String>,
    /// Component or subsystem emitting the event (e.g. "fusion", "acp", "agent", "tool").
    #[serde(default = "default_source")]
    pub source: String,
    /// Semantic classification of the event.
    pub kind: JsonLogEventKind,
    /// Strongly-typed structured payload.
    pub payload: JsonLogPayload,
    /// Optional metadata key-value tags.
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub metadata: HashMap<String, serde_json::Value>,
}

fn default_source() -> String {
    "fusion".to_string()
}

// ============================================================================
// Granular Event Payloads
// ============================================================================

/// Polymorphic payload data carried inside a [`JsonLogEvent`].
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(untagged)]
pub enum JsonLogPayload {
    SessionStart(SessionStartPayload),
    SessionEnd(SessionEndPayload),
    TurnStart(TurnStartPayload),
    TurnEnd(TurnEndPayload),
    TextDelta(TextDeltaPayload),
    ThinkingDelta(ThinkingDeltaPayload),
    ToolStart(ToolStartPayload),
    ToolProgress(ToolProgressPayload),
    ToolFinish(ToolFinishPayload),
    AdvisorStart(AdvisorStartPayload),
    AdvisorFeedback(AdvisorFeedbackPayload),
    AdvisorConsensus(AdvisorConsensusPayload),
    SubagentStart(SubagentStartPayload),
    SubagentFinish(SubagentFinishPayload),
    PlanUpdate(PlanUpdatePayload),
    TokenStats(TokenStatsPayload),
    Status(StatusPayload),
    Error(ErrorPayload),
    Raw(serde_json::Value),
}

/// Payload for `session_start` event.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SessionStartPayload {
    pub session_id: String,
    pub model: Option<String>,
    pub provider: Option<String>,
    pub working_dir: Option<String>,
    pub client_name: Option<String>,
}

/// Payload for `session_end` event.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SessionEndPayload {
    pub session_id: String,
    pub duration_ms: u64,
    pub total_tokens: u64,
    pub reason: Option<String>,
}

/// Payload for `turn_start` event.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct TurnStartPayload {
    pub turn_id: String,
    pub prompt: String,
    pub model: Option<String>,
}

/// Payload for `turn_end` event.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct TurnEndPayload {
    pub turn_id: String,
    pub stop_reason: String,
    pub duration_ms: u64,
    pub prompt_tokens: u64,
    pub completion_tokens: u64,
    pub total_tokens: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub response_text: Option<String>,
}

/// Payload for incremental text streaming (`text_delta`).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct TextDeltaPayload {
    pub delta: String,
    pub index: u64,
    pub is_first: bool,
    pub is_last: bool,
}

/// Payload for incremental thinking / reasoning streaming (`thinking_delta`).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ThinkingDeltaPayload {
    pub delta: String,
    pub index: u64,
    pub elapsed_ms: u64,
}

/// Payload for tool execution start (`tool_start`).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ToolStartPayload {
    pub call_id: String,
    pub tool_name: String,
    pub arguments: serde_json::Value,
}

/// Payload for tool execution progress (`tool_progress`).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ToolProgressPayload {
    pub call_id: String,
    pub tool_name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub partial_output: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub percentage: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

/// Payload for tool execution completion (`tool_finish`).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ToolFinishPayload {
    pub call_id: String,
    pub tool_name: String,
    pub success: bool,
    pub output: String,
    pub duration_ms: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// Payload for advisor start (`advisor_start`).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AdvisorStartPayload {
    pub advisor: String,
    pub role: String,
}

/// Payload for advisor feedback / critique (`advisor_feedback`).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct AdvisorFeedbackPayload {
    pub advisor: String,
    pub role: Option<String>,
    pub approved: bool,
    pub critique: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub suggestions: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub severity: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub score: Option<f32>,
}

/// Payload for collective advisor consensus (`advisor_consensus`).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AdvisorConsensusPayload {
    pub total_advisors: usize,
    pub approved_count: usize,
    pub rejected_count: usize,
    pub warning_count: usize,
    pub overall_approved: bool,
    pub summary: String,
}

/// Payload for subagent delegation start (`subagent_start`).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SubagentStartPayload {
    pub subagent_name: String,
    pub task: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parent_id: Option<String>,
}

/// Payload for subagent delegation completion (`subagent_finish`).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SubagentFinishPayload {
    pub subagent_name: String,
    pub success: bool,
    pub output: String,
    pub duration_ms: u64,
}

/// Payload for execution plan progress update (`plan_update`).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct PlanUpdatePayload {
    pub current_step: usize,
    pub total_steps: usize,
    pub description: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub completed_steps: Vec<String>,
}

/// Payload for token usage & throughput metrics (`token_stats`).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct TokenStatsPayload {
    pub prompt_tokens: u64,
    pub completion_tokens: u64,
    pub total_tokens: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tokens_per_second: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cost_usd: Option<f64>,
}

/// Payload for general status / informational logs (`status`).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct StatusPayload {
    pub level: String,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub category: Option<String>,
}

/// Payload for error logs (`error`).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ErrorPayload {
    pub error: String,
    pub recoverable: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub code: Option<String>,
}

// ============================================================================
// JsonLogEvent Methods & Constructors
// ============================================================================

impl JsonLogEvent {
    /// Creates a new generic event with generated timestamp.
    pub fn new(
        seq: u64,
        kind: JsonLogEventKind,
        payload: JsonLogPayload,
        session_id: Option<String>,
    ) -> Self {
        Self {
            seq,
            timestamp: current_iso8601(),
            timestamp_ms: current_epoch_ms(),
            session_id,
            turn_id: None,
            source: default_source(),
            kind,
            payload,
            metadata: HashMap::new(),
        }
    }

    /// Builder helper to attach turn ID.
    pub fn with_turn_id(mut self, turn_id: impl Into<String>) -> Self {
        self.turn_id = Some(turn_id.into());
        self
    }

    /// Builder helper to set the emitting source.
    pub fn with_source(mut self, source: impl Into<String>) -> Self {
        self.source = source.into();
        self
    }

    /// Builder helper to insert a metadata key-value tag.
    pub fn with_meta(
        mut self,
        key: impl Into<String>,
        value: impl Into<serde_json::Value>,
    ) -> Self {
        self.metadata.insert(key.into(), value.into());
        self
    }

    // --- Fast Constructors ---

    /// Creates a `session_start` event.
    pub fn session_start(
        seq: u64,
        session_id: impl Into<String>,
        model: Option<String>,
        provider: Option<String>,
        working_dir: Option<String>,
    ) -> Self {
        let sid = session_id.into();
        Self::new(
            seq,
            JsonLogEventKind::SessionStart,
            JsonLogPayload::SessionStart(SessionStartPayload {
                session_id: sid.clone(),
                model,
                provider,
                working_dir,
                client_name: None,
            }),
            Some(sid),
        )
    }

    /// Creates a `session_end` event.
    pub fn session_end(
        seq: u64,
        session_id: impl Into<String>,
        duration_ms: u64,
        total_tokens: u64,
        reason: Option<String>,
    ) -> Self {
        let sid = session_id.into();
        Self::new(
            seq,
            JsonLogEventKind::SessionEnd,
            JsonLogPayload::SessionEnd(SessionEndPayload {
                session_id: sid.clone(),
                duration_ms,
                total_tokens,
                reason,
            }),
            Some(sid),
        )
    }

    /// Creates a `turn_start` event.
    pub fn turn_start(
        seq: u64,
        session_id: Option<String>,
        turn_id: impl Into<String>,
        prompt: impl Into<String>,
        model: Option<String>,
    ) -> Self {
        let tid = turn_id.into();
        Self::new(
            seq,
            JsonLogEventKind::TurnStart,
            JsonLogPayload::TurnStart(TurnStartPayload {
                turn_id: tid.clone(),
                prompt: prompt.into(),
                model,
            }),
            session_id,
        )
        .with_turn_id(tid)
    }

    /// Creates a `turn_end` event.
    pub fn turn_end(
        seq: u64,
        session_id: Option<String>,
        turn_id: impl Into<String>,
        stop_reason: impl Into<String>,
        duration_ms: u64,
        prompt_tokens: u64,
        completion_tokens: u64,
        total_tokens: u64,
        response_text: Option<String>,
    ) -> Self {
        let tid = turn_id.into();
        Self::new(
            seq,
            JsonLogEventKind::TurnEnd,
            JsonLogPayload::TurnEnd(TurnEndPayload {
                turn_id: tid.clone(),
                stop_reason: stop_reason.into(),
                duration_ms,
                prompt_tokens,
                completion_tokens,
                total_tokens,
                response_text,
            }),
            session_id,
        )
        .with_turn_id(tid)
    }

    /// Creates a `text_delta` event.
    pub fn text_delta(
        seq: u64,
        session_id: Option<String>,
        delta: impl Into<String>,
        index: u64,
        is_first: bool,
        is_last: bool,
    ) -> Self {
        Self::new(
            seq,
            JsonLogEventKind::TextDelta,
            JsonLogPayload::TextDelta(TextDeltaPayload {
                delta: delta.into(),
                index,
                is_first,
                is_last,
            }),
            session_id,
        )
    }

    /// Creates a `thinking_delta` event.
    pub fn thinking_delta(
        seq: u64,
        session_id: Option<String>,
        delta: impl Into<String>,
        index: u64,
        elapsed_ms: u64,
    ) -> Self {
        Self::new(
            seq,
            JsonLogEventKind::ThinkingDelta,
            JsonLogPayload::ThinkingDelta(ThinkingDeltaPayload {
                delta: delta.into(),
                index,
                elapsed_ms,
            }),
            session_id,
        )
    }

    /// Creates a `tool_start` event.
    pub fn tool_start(
        seq: u64,
        session_id: Option<String>,
        call_id: impl Into<String>,
        tool_name: impl Into<String>,
        arguments: serde_json::Value,
    ) -> Self {
        Self::new(
            seq,
            JsonLogEventKind::ToolStart,
            JsonLogPayload::ToolStart(ToolStartPayload {
                call_id: call_id.into(),
                tool_name: tool_name.into(),
                arguments,
            }),
            session_id,
        )
        .with_source("tool")
    }

    /// Creates a `tool_progress` event.
    pub fn tool_progress(
        seq: u64,
        session_id: Option<String>,
        call_id: impl Into<String>,
        tool_name: impl Into<String>,
        partial_output: Option<String>,
        percentage: Option<f32>,
        message: Option<String>,
    ) -> Self {
        Self::new(
            seq,
            JsonLogEventKind::ToolProgress,
            JsonLogPayload::ToolProgress(ToolProgressPayload {
                call_id: call_id.into(),
                tool_name: tool_name.into(),
                partial_output,
                percentage,
                message,
            }),
            session_id,
        )
        .with_source("tool")
    }

    /// Creates a `tool_finish` event.
    pub fn tool_finish(
        seq: u64,
        session_id: Option<String>,
        call_id: impl Into<String>,
        tool_name: impl Into<String>,
        success: bool,
        output: impl Into<String>,
        duration_ms: u64,
        error: Option<String>,
    ) -> Self {
        Self::new(
            seq,
            JsonLogEventKind::ToolFinish,
            JsonLogPayload::ToolFinish(ToolFinishPayload {
                call_id: call_id.into(),
                tool_name: tool_name.into(),
                success,
                output: output.into(),
                duration_ms,
                error,
            }),
            session_id,
        )
        .with_source("tool")
    }

    /// Creates an `advisor_start` event.
    pub fn advisor_start(
        seq: u64,
        session_id: Option<String>,
        advisor: impl Into<String>,
        role: impl Into<String>,
    ) -> Self {
        Self::new(
            seq,
            JsonLogEventKind::AdvisorStart,
            JsonLogPayload::AdvisorStart(AdvisorStartPayload {
                advisor: advisor.into(),
                role: role.into(),
            }),
            session_id,
        )
        .with_source("advisor")
    }

    /// Creates an `advisor_feedback` event.
    pub fn advisor_feedback(
        seq: u64,
        session_id: Option<String>,
        advisor: impl Into<String>,
        role: Option<String>,
        approved: bool,
        critique: impl Into<String>,
        suggestions: Vec<String>,
        severity: Option<String>,
        score: Option<f32>,
    ) -> Self {
        Self::new(
            seq,
            JsonLogEventKind::AdvisorFeedback,
            JsonLogPayload::AdvisorFeedback(AdvisorFeedbackPayload {
                advisor: advisor.into(),
                role,
                approved,
                critique: critique.into(),
                suggestions,
                severity,
                score,
            }),
            session_id,
        )
        .with_source("advisor")
    }

    /// Creates an `advisor_consensus` event.
    pub fn advisor_consensus(
        seq: u64,
        session_id: Option<String>,
        consensus: &AdvisorConsensus,
    ) -> Self {
        Self::new(
            seq,
            JsonLogEventKind::AdvisorConsensus,
            JsonLogPayload::AdvisorConsensus(AdvisorConsensusPayload {
                total_advisors: consensus.total_advisors,
                approved_count: consensus.approved_count,
                rejected_count: consensus.rejected_count,
                warning_count: consensus.warning_count,
                overall_approved: consensus.overall_approved,
                summary: consensus.summary.clone(),
            }),
            session_id,
        )
        .with_source("advisor")
    }

    /// Creates a `subagent_start` event.
    pub fn subagent_start(
        seq: u64,
        session_id: Option<String>,
        subagent_name: impl Into<String>,
        task: impl Into<String>,
        parent_id: Option<String>,
    ) -> Self {
        Self::new(
            seq,
            JsonLogEventKind::SubagentStart,
            JsonLogPayload::SubagentStart(SubagentStartPayload {
                subagent_name: subagent_name.into(),
                task: task.into(),
                parent_id,
            }),
            session_id,
        )
        .with_source("subagent")
    }

    /// Creates a `subagent_finish` event.
    pub fn subagent_finish(
        seq: u64,
        session_id: Option<String>,
        subagent_name: impl Into<String>,
        success: bool,
        output: impl Into<String>,
        duration_ms: u64,
    ) -> Self {
        Self::new(
            seq,
            JsonLogEventKind::SubagentFinish,
            JsonLogPayload::SubagentFinish(SubagentFinishPayload {
                subagent_name: subagent_name.into(),
                success,
                output: output.into(),
                duration_ms,
            }),
            session_id,
        )
        .with_source("subagent")
    }

    /// Creates a `plan_update` event.
    pub fn plan_update(
        seq: u64,
        session_id: Option<String>,
        current_step: usize,
        total_steps: usize,
        description: impl Into<String>,
        completed_steps: Vec<String>,
    ) -> Self {
        Self::new(
            seq,
            JsonLogEventKind::PlanUpdate,
            JsonLogPayload::PlanUpdate(PlanUpdatePayload {
                current_step,
                total_steps,
                description: description.into(),
                completed_steps,
            }),
            session_id,
        )
    }

    /// Creates a `token_stats` event.
    pub fn token_stats(
        seq: u64,
        session_id: Option<String>,
        prompt_tokens: u64,
        completion_tokens: u64,
        total_tokens: u64,
        tokens_per_second: Option<f64>,
    ) -> Self {
        Self::new(
            seq,
            JsonLogEventKind::TokenStats,
            JsonLogPayload::TokenStats(TokenStatsPayload {
                prompt_tokens,
                completion_tokens,
                total_tokens,
                tokens_per_second,
                cost_usd: None,
            }),
            session_id,
        )
    }

    /// Creates a `status` log event.
    pub fn status(
        seq: u64,
        session_id: Option<String>,
        level: impl Into<String>,
        message: impl Into<String>,
    ) -> Self {
        Self::new(
            seq,
            JsonLogEventKind::Status,
            JsonLogPayload::Status(StatusPayload {
                level: level.into(),
                message: message.into(),
                category: None,
            }),
            session_id,
        )
    }

    /// Creates an `error` event.
    pub fn error(
        seq: u64,
        session_id: Option<String>,
        error: impl Into<String>,
        recoverable: bool,
    ) -> Self {
        Self::new(
            seq,
            JsonLogEventKind::Error,
            JsonLogPayload::Error(ErrorPayload {
                error: error.into(),
                recoverable,
                code: None,
            }),
            session_id,
        )
    }

    /// Creates a `custom` event with an arbitrary JSON value.
    pub fn custom(seq: u64, session_id: Option<String>, payload: serde_json::Value) -> Self {
        Self::new(
            seq,
            JsonLogEventKind::Custom,
            JsonLogPayload::Raw(payload),
            session_id,
        )
    }

    // --- Serialization & Deserialization ---

    /// Serializes this event into a single-line compact NDJSON string ending with `\n`.
    pub fn to_ndjson_line(&self) -> Result<String, serde_json::Error> {
        let mut json = serde_json::to_string(self)?;
        json.push('\n');
        Ok(json)
    }

    /// Deserializes a single NDJSON line into a `JsonLogEvent`.
    /// Handles optional trailing `\r` and `\n` line endings.
    pub fn from_ndjson_line(line: &str) -> Result<Self, serde_json::Error> {
        let trimmed = line.trim();
        serde_json::from_str(trimmed)
    }

    // --- Bridge Converters ---

    /// Converts a Fusion [`AgentEvent`] to a structured [`JsonLogEvent`].
    pub fn from_agent_event(seq: u64, session_id: Option<&str>, event: &AgentEvent) -> Self {
        let sid = session_id.map(|s| s.to_string());
        match event {
            AgentEvent::TextDelta(delta) => Self::text_delta(seq, sid, delta, 0, false, false),
            AgentEvent::ThinkingDelta(delta) => Self::thinking_delta(seq, sid, delta, 0, 0),
            AgentEvent::ToolStarted { id, name, args } => {
                Self::tool_start(seq, sid, id, name, args.clone())
            }
            AgentEvent::ToolFinished {
                id,
                name,
                success,
                output,
                duration,
            } => Self::tool_finish(
                seq,
                sid,
                id,
                name,
                *success,
                output,
                duration.as_millis() as u64,
                if !*success {
                    Some(output.clone())
                } else {
                    None
                },
            ),
            AgentEvent::AdvisorStarted { advisor, role } => {
                Self::advisor_start(seq, sid, advisor, role)
            }
            AgentEvent::AdvisorCritique {
                advisor,
                approved,
                critique,
            } => Self::advisor_feedback(
                seq,
                sid,
                advisor,
                None,
                *approved,
                critique,
                vec![],
                None,
                None,
            ),
            AgentEvent::SubagentStarted { name, task } => {
                Self::subagent_start(seq, sid, name, task, None)
            }
            AgentEvent::SubagentFinished {
                name,
                success,
                output,
            } => Self::subagent_finish(seq, sid, name, *success, output, 0),
            AgentEvent::Status(msg) => Self::status(seq, sid, "info", msg),
            AgentEvent::Finished { usage } => {
                let (pt, ct, tt) = if let Some(u) = usage {
                    (
                        u.get("prompt_tokens").and_then(|v| v.as_u64()).unwrap_or(0),
                        u.get("completion_tokens")
                            .and_then(|v| v.as_u64())
                            .unwrap_or(0),
                        u.get("total_tokens").and_then(|v| v.as_u64()).unwrap_or(0),
                    )
                } else {
                    (0, 0, 0)
                };
                Self::token_stats(seq, sid, pt, ct, tt, None)
            }
            AgentEvent::Error(err) => Self::error(seq, sid, err, false),
        }
    }

    /// Converts a rich ACP [`AcpSessionEvent`] to a structured [`JsonLogEvent`].
    pub fn from_acp_session_event(
        seq: u64,
        session_id: Option<&str>,
        event: &AcpSessionEvent,
    ) -> Self {
        let sid = session_id.map(|s| s.to_string());
        match event {
            AcpSessionEvent::TokenChunk(chunk) => Self::text_delta(
                seq,
                sid,
                &chunk.delta,
                chunk.index,
                chunk.is_first,
                chunk.is_last,
            ),
            AcpSessionEvent::ThinkingChunk(chunk) => {
                Self::thinking_delta(seq, sid, &chunk.delta, chunk.index, chunk.elapsed_ms)
            }
            AcpSessionEvent::ToolStarted(update) => Self::tool_start(
                seq,
                sid,
                &update.call_id,
                &update.name,
                update.args.clone().unwrap_or(serde_json::Value::Null),
            ),
            AcpSessionEvent::ToolProgress(update) => Self::tool_progress(
                seq,
                sid,
                &update.call_id,
                &update.name,
                update.partial_output.clone(),
                update.progress.map(|p| p as f32),
                Some(update.status.clone()),
            ),
            AcpSessionEvent::ToolCompleted(update) => {
                let success = update.state == ToolExecutionState::Completed
                    || update.success.unwrap_or(false);
                Self::tool_finish(
                    seq,
                    sid,
                    &update.call_id,
                    &update.name,
                    success,
                    update.output.clone().unwrap_or_default(),
                    update.duration_ms.unwrap_or(0),
                    update.error.clone(),
                )
            }
            AcpSessionEvent::AdvisorStarted(update) => {
                Self::advisor_start(seq, sid, &update.advisor, &update.role)
            }
            AcpSessionEvent::AdvisorFeedback(update) => Self::advisor_feedback(
                seq,
                sid,
                &update.advisor,
                Some(update.role.clone()),
                update.approved,
                &update.critique,
                update.suggestions.clone(),
                Some(update.severity.to_string()),
                update.confidence,
            ),
            AcpSessionEvent::AdvisorConsensus(consensus) => {
                Self::advisor_consensus(seq, sid, consensus)
            }
            AcpSessionEvent::TokenStats(stats) => Self::token_stats(
                seq,
                sid,
                stats.prompt_tokens,
                stats.completion_tokens,
                stats.total_tokens,
                stats.tokens_per_second,
            ),
            AcpSessionEvent::Subagent(sub) => {
                if sub.status == "finished" {
                    Self::subagent_finish(
                        seq,
                        sid,
                        &sub.name,
                        sub.success.unwrap_or(true),
                        sub.output.clone().unwrap_or_default(),
                        0,
                    )
                } else {
                    Self::subagent_start(
                        seq,
                        sid,
                        &sub.name,
                        sub.task.clone().unwrap_or_default(),
                        None,
                    )
                }
            }
            AcpSessionEvent::Plan(plan) => {
                let curr_desc = plan
                    .current_step_index
                    .and_then(|idx| plan.steps.get(idx))
                    .map(|s| s.description.clone())
                    .unwrap_or_default();
                Self::plan_update(
                    seq,
                    sid,
                    plan.current_step_index.unwrap_or(0),
                    plan.total_count,
                    curr_desc,
                    plan.steps
                        .iter()
                        .filter(|s| s.status == "completed")
                        .map(|s| s.description.clone())
                        .collect(),
                )
            }
            AcpSessionEvent::Status { message, level, .. } => {
                Self::status(seq, sid, level, message)
            }
            AcpSessionEvent::Error {
                error, recoverable, ..
            } => Self::error(seq, sid, error, *recoverable),
        }
    }
}

// ============================================================================
// Batch Formatting and Parsing Helpers
// ============================================================================

/// Serializes a slice of [`JsonLogEvent`]s into a multi-line NDJSON string.
pub fn format_ndjson_batch(events: &[JsonLogEvent]) -> Result<String, serde_json::Error> {
    let mut buffer = String::with_capacity(events.len() * 128);
    for event in events {
        let line = event.to_ndjson_line()?;
        buffer.push_str(&line);
    }
    Ok(buffer)
}

/// Parses an NDJSON string containing multiple lines into a `Vec<JsonLogEvent>`.
/// Skips empty lines and comment lines (starting with `#` or `//`).
pub fn parse_ndjson_lines(input: &str) -> Result<Vec<JsonLogEvent>, serde_json::Error> {
    let mut results = Vec::new();
    for line in input.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') || trimmed.starts_with("//") {
            continue;
        }
        let event = JsonLogEvent::from_ndjson_line(trimmed)?;
        results.push(event);
    }
    Ok(results)
}

// ============================================================================
// Async NDJSON Event Streamer (`JsonLogStreamer`)
// ============================================================================

/// High-performance asynchronous NDJSON event streamer.
///
/// Wraps any [`AsyncWrite`] stream (e.g. stdout, unix socket, tcp stream, pipe, file)
/// and writes newline-delimited JSON events with monotonic sequence numbering.
pub struct JsonLogStreamer<W> {
    writer: W,
    seq: Arc<AtomicU64>,
    session_id: Option<String>,
    source: String,
    flush_on_write: bool,
}

impl<W: AsyncWrite + Unpin + Send> JsonLogStreamer<W> {
    /// Creates a new streamer wrapping the provided asynchronous writer.
    pub fn new(writer: W) -> Self {
        Self {
            writer,
            seq: Arc::new(AtomicU64::new(1)),
            session_id: None,
            source: default_source(),
            flush_on_write: true,
        }
    }

    /// Attaches a default session ID to all emitted events.
    pub fn with_session_id(mut self, session_id: impl Into<String>) -> Self {
        self.session_id = Some(session_id.into());
        self
    }

    /// Attaches a default source tag to all emitted events.
    pub fn with_source(mut self, source: impl Into<String>) -> Self {
        self.source = source.into();
        self
    }

    /// Controls whether each event write automatically flushes the underlying stream.
    pub fn with_flush_on_write(mut self, flush: bool) -> Self {
        self.flush_on_write = flush;
        self
    }

    /// Returns the current sequence number counter reference.
    pub fn sequence_counter(&self) -> Arc<AtomicU64> {
        self.seq.clone()
    }

    /// Next monotonic sequence number.
    #[inline]
    fn next_seq(&self) -> u64 {
        self.seq.fetch_add(1, Ordering::SeqCst)
    }

    /// Writes an existing pre-built [`JsonLogEvent`] as an NDJSON line.
    pub async fn write_event(&mut self, event: &JsonLogEvent) -> std::io::Result<()> {
        let line = event
            .to_ndjson_line()
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
        self.writer.write_all(line.as_bytes()).await?;
        if self.flush_on_write {
            self.writer.flush().await?;
        }
        Ok(())
    }

    /// Emits a generic event with the given kind and payload.
    pub async fn emit_event(
        &mut self,
        kind: JsonLogEventKind,
        payload: JsonLogPayload,
    ) -> std::io::Result<u64> {
        let seq = self.next_seq();
        let event = JsonLogEvent::new(seq, kind, payload, self.session_id.clone())
            .with_source(self.source.clone());
        self.write_event(&event).await?;
        Ok(seq)
    }

    /// Emits an incremental assistant text delta.
    pub async fn emit_text_delta(
        &mut self,
        delta: impl Into<String>,
        index: u64,
        is_first: bool,
        is_last: bool,
    ) -> std::io::Result<u64> {
        let seq = self.next_seq();
        let event = JsonLogEvent::text_delta(
            seq,
            self.session_id.clone(),
            delta,
            index,
            is_first,
            is_last,
        )
        .with_source(self.source.clone());
        self.write_event(&event).await?;
        Ok(seq)
    }

    /// Emits an incremental thinking/reasoning delta.
    pub async fn emit_thinking_delta(
        &mut self,
        delta: impl Into<String>,
        index: u64,
        elapsed_ms: u64,
    ) -> std::io::Result<u64> {
        let seq = self.next_seq();
        let event =
            JsonLogEvent::thinking_delta(seq, self.session_id.clone(), delta, index, elapsed_ms)
                .with_source(self.source.clone());
        self.write_event(&event).await?;
        Ok(seq)
    }

    /// Emits a tool execution start event.
    pub async fn emit_tool_start(
        &mut self,
        call_id: impl Into<String>,
        tool_name: impl Into<String>,
        arguments: serde_json::Value,
    ) -> std::io::Result<u64> {
        let seq = self.next_seq();
        let event =
            JsonLogEvent::tool_start(seq, self.session_id.clone(), call_id, tool_name, arguments);
        self.write_event(&event).await?;
        Ok(seq)
    }

    /// Emits a tool execution completion event.
    pub async fn emit_tool_finish(
        &mut self,
        call_id: impl Into<String>,
        tool_name: impl Into<String>,
        success: bool,
        output: impl Into<String>,
        duration_ms: u64,
        error: Option<String>,
    ) -> std::io::Result<u64> {
        let seq = self.next_seq();
        let event = JsonLogEvent::tool_finish(
            seq,
            self.session_id.clone(),
            call_id,
            tool_name,
            success,
            output,
            duration_ms,
            error,
        );
        self.write_event(&event).await?;
        Ok(seq)
    }

    /// Emits a Fusion [`AgentEvent`] as an NDJSON line.
    pub async fn emit_agent_event(&mut self, event: &AgentEvent) -> std::io::Result<u64> {
        let seq = self.next_seq();
        let json_event = JsonLogEvent::from_agent_event(seq, self.session_id.as_deref(), event);
        self.write_event(&json_event).await?;
        Ok(seq)
    }

    /// Emits an ACP [`AcpSessionEvent`] as an NDJSON line.
    pub async fn emit_acp_event(&mut self, event: &AcpSessionEvent) -> std::io::Result<u64> {
        let seq = self.next_seq();
        let json_event =
            JsonLogEvent::from_acp_session_event(seq, self.session_id.as_deref(), event);
        self.write_event(&json_event).await?;
        Ok(seq)
    }

    /// Flushes any buffered bytes to the underlying writer.
    pub async fn flush(&mut self) -> std::io::Result<()> {
        self.writer.flush().await
    }

    /// Consumes the streamer and returns the underlying writer.
    pub fn into_inner(self) -> W {
        self.writer
    }
}

// ============================================================================
// Async NDJSON Event Reader (`JsonLogReader`)
// ============================================================================

/// Asynchronous reader for consuming newline-delimited JSON event streams.
///
/// Wraps any [`AsyncBufRead`] instance and yields [`JsonLogEvent`]s one line at a time.
pub struct JsonLogReader<R> {
    reader: R,
    line_number: usize,
}

impl<R: AsyncBufRead + Unpin + Send> JsonLogReader<R> {
    /// Creates a new reader from the given buffered async reader.
    pub fn new(reader: R) -> Self {
        Self {
            reader,
            line_number: 0,
        }
    }

    /// Reads and parses the next [`JsonLogEvent`] from the stream.
    /// Returns `Ok(None)` when EOF is reached.
    /// Skips blank lines and comment lines.
    pub async fn next_event(&mut self) -> std::io::Result<Option<JsonLogEvent>> {
        let mut line = String::new();
        loop {
            line.clear();
            let bytes_read = self.reader.read_line(&mut line).await?;
            if bytes_read == 0 {
                return Ok(None); // EOF
            }
            self.line_number += 1;

            let trimmed = line.trim();
            if trimmed.is_empty() || trimmed.starts_with('#') || trimmed.starts_with("//") {
                continue;
            }

            match JsonLogEvent::from_ndjson_line(trimmed) {
                Ok(event) => return Ok(Some(event)),
                Err(e) => {
                    return Err(std::io::Error::new(
                        std::io::ErrorKind::InvalidData,
                        format!(
                            "Line {}: failed to parse NDJSON event: {}",
                            self.line_number, e
                        ),
                    ));
                }
            }
        }
    }

    /// Reads all remaining events from the stream until EOF.
    pub async fn read_all(&mut self) -> std::io::Result<Vec<JsonLogEvent>> {
        let mut events = Vec::new();
        while let Some(event) = self.next_event().await? {
            events.push(event);
        }
        Ok(events)
    }

    /// Returns the current 1-based line number processed.
    pub fn current_line_number(&self) -> usize {
        self.line_number
    }

    /// Consumes the reader and returns the underlying stream.
    pub fn into_inner(self) -> R {
        self.reader
    }
}

// ============================================================================
// Multi-Subscriber Broadcaster (`JsonLogBroadcaster`)
// ============================================================================

/// In-memory pub/sub broadcaster for distributing live NDJSON events to multiple consumers.
///
/// Ideal for editor plugin multiplexing, real-time UI widgets, and background telemetry observers.
#[derive(Clone)]
pub struct JsonLogBroadcaster {
    sender: Arc<broadcast::Sender<JsonLogEvent>>,
}

impl Default for JsonLogBroadcaster {
    fn default() -> Self {
        Self::new(1024)
    }
}

impl JsonLogBroadcaster {
    /// Creates a new broadcaster with the given channel capacity.
    pub fn new(capacity: usize) -> Self {
        let (sender, _) = broadcast::channel(capacity);
        Self {
            sender: Arc::new(sender),
        }
    }

    /// Subscribes to the live event stream.
    pub fn subscribe(&self) -> broadcast::Receiver<JsonLogEvent> {
        self.sender.subscribe()
    }

    /// Publishes an event to all active subscribers.
    pub fn publish(
        &self,
        event: JsonLogEvent,
    ) -> Result<usize, broadcast::error::SendError<JsonLogEvent>> {
        self.sender.send(event)
    }

    /// Returns the number of active subscriber receivers.
    pub fn receiver_count(&self) -> usize {
        self.sender.receiver_count()
    }
}

// ============================================================================
// Structured Persistent File Logger (`NdjsonFileLogger`)
// ============================================================================

/// High-throughput file logger for writing structured `.jsonl` audit trails.
pub struct NdjsonFileLogger {
    file_path: PathBuf,
    writer: BufWriter<tokio::fs::File>,
    seq: u64,
}

impl NdjsonFileLogger {
    /// Opens or creates a `.jsonl` file at the specified path in append mode.
    pub async fn open(path: impl AsRef<Path>) -> std::io::Result<Self> {
        let path_buf = path.as_ref().to_path_buf();
        if let Some(parent) = path_buf.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }

        let file = tokio::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path_buf)
            .await?;

        Ok(Self {
            file_path: path_buf,
            writer: BufWriter::new(file),
            seq: 1,
        })
    }

    /// Opens a standard session log in `.fusion/logs/<session_id>.jsonl`.
    pub async fn open_session_log(session_id: &str) -> std::io::Result<Self> {
        let logs_dir = PathBuf::from(".fusion").join("logs");
        let safe_name = format!("{}.jsonl", session_id.replace('/', "_"));
        Self::open(logs_dir.join(safe_name)).await
    }

    /// Appends a structured [`JsonLogEvent`] to the log file.
    pub async fn append(&mut self, event: &JsonLogEvent) -> std::io::Result<()> {
        let line = event
            .to_ndjson_line()
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
        self.writer.write_all(line.as_bytes()).await?;
        self.seq += 1;
        Ok(())
    }

    /// Appends a batch of [`JsonLogEvent`]s to the log file.
    pub async fn append_batch(&mut self, events: &[JsonLogEvent]) -> std::io::Result<()> {
        for event in events {
            self.append(event).await?;
        }
        self.writer.flush().await?;
        Ok(())
    }

    /// Flushes any buffered bytes to disk.
    pub async fn flush(&mut self) -> std::io::Result<()> {
        self.writer.flush().await
    }

    /// Returns the target path of the log file.
    pub fn path(&self) -> &Path {
        &self.file_path
    }
}

// ============================================================================
// Event Filtering Engine (`JsonLogFilter`)
// ============================================================================

/// Configurable filter for selecting and transforming NDJSON event streams.
#[derive(Debug, Clone, Default)]
pub struct JsonLogFilter {
    /// If non-empty, only events with matching kinds are allowed.
    pub allowed_kinds: Option<HashSet<JsonLogEventKind>>,
    /// If set, filters events by matching session ID.
    pub session_id: Option<String>,
    /// If set, filters events by matching source.
    pub source: Option<String>,
    /// Controls whether granular token deltas are included (can be disabled to reduce bandwidth).
    pub include_tokens: bool,
    /// Minimum status level ("debug", "info", "warn", "error").
    pub min_level: Option<String>,
}

impl JsonLogFilter {
    /// Creates a new filter with default settings (includes tokens).
    pub fn new() -> Self {
        Self {
            allowed_kinds: None,
            session_id: None,
            source: None,
            include_tokens: true,
            min_level: None,
        }
    }

    /// Restricts allowed event kinds.
    pub fn with_allowed_kinds(mut self, kinds: impl IntoIterator<Item = JsonLogEventKind>) -> Self {
        self.allowed_kinds = Some(kinds.into_iter().collect());
        self
    }

    /// Restricts to a specific session ID.
    pub fn with_session_id(mut self, session_id: impl Into<String>) -> Self {
        self.session_id = Some(session_id.into());
        self
    }

    /// Restricts to a specific source component.
    pub fn with_source(mut self, source: impl Into<String>) -> Self {
        self.source = Some(source.into());
        self
    }

    /// Sets whether to include granular token deltas.
    pub fn with_tokens(mut self, include_tokens: bool) -> Self {
        self.include_tokens = include_tokens;
        self
    }

    /// Evaluates whether an event passes the filter criteria.
    pub fn matches(&self, event: &JsonLogEvent) -> bool {
        // Token exclusion check
        if !self.include_tokens
            && (event.kind == JsonLogEventKind::TextDelta
                || event.kind == JsonLogEventKind::ThinkingDelta)
        {
            return false;
        }

        // Kinds check
        if let Some(allowed) = &self.allowed_kinds {
            if !allowed.contains(&event.kind) {
                return false;
            }
        }

        // Session ID check
        if let Some(sid) = &self.session_id {
            if event.session_id.as_deref() != Some(sid.as_str()) {
                return false;
            }
        }

        // Source check
        if let Some(src) = &self.source {
            if &event.source != src {
                return false;
            }
        }
        true
    }
}

// ============================================================================
// Headless Event Collector & Report
// ============================================================================

/// Summary record of a tool invocation observed during a headless run.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct HeadlessToolRecord {
    pub call_id: String,
    pub tool_name: String,
    pub success: bool,
    pub duration_ms: u64,
    pub output_preview: String,
}

/// Comprehensive execution report produced by [`HeadlessEventCollector`].
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct HeadlessExecutionReport {
    pub session_id: Option<String>,
    pub turn_id: Option<String>,
    pub full_assistant_text: String,
    pub full_thinking_text: String,
    pub tools_executed: Vec<HeadlessToolRecord>,
    pub advisor_critiques: Vec<AdvisorFeedbackPayload>,
    pub consensus: Option<AdvisorConsensusPayload>,
    pub token_stats: Option<TokenStatsPayload>,
    pub errors: Vec<String>,
    pub stop_reason: Option<String>,
    pub total_events: usize,
}

/// In-flight event accumulator for headless automation and CI/CD test harnesses.
#[derive(Debug, Clone, Default)]
pub struct HeadlessEventCollector {
    report: HeadlessExecutionReport,
}

impl HeadlessEventCollector {
    /// Creates a new empty event accumulator.
    pub fn new() -> Self {
        Self::default()
    }

    /// Ingests a structured [`JsonLogEvent`] into the accumulated report.
    pub fn ingest(&mut self, event: &JsonLogEvent) {
        self.report.total_events += 1;

        if self.report.session_id.is_none() && event.session_id.is_some() {
            self.report.session_id = event.session_id.clone();
        }
        if self.report.turn_id.is_none() && event.turn_id.is_some() {
            self.report.turn_id = event.turn_id.clone();
        }

        match &event.payload {
            JsonLogPayload::TextDelta(p) => {
                self.report.full_assistant_text.push_str(&p.delta);
            }
            JsonLogPayload::ThinkingDelta(p) => {
                self.report.full_thinking_text.push_str(&p.delta);
            }
            JsonLogPayload::ToolFinish(p) => {
                let preview = if p.output.len() > 120 {
                    format!("{}...", &p.output[..120])
                } else {
                    p.output.clone()
                };
                self.report.tools_executed.push(HeadlessToolRecord {
                    call_id: p.call_id.clone(),
                    tool_name: p.tool_name.clone(),
                    success: p.success,
                    duration_ms: p.duration_ms,
                    output_preview: preview,
                });
            }
            JsonLogPayload::AdvisorFeedback(p) => {
                self.report.advisor_critiques.push(p.clone());
            }
            JsonLogPayload::AdvisorConsensus(p) => {
                self.report.consensus = Some(p.clone());
            }
            JsonLogPayload::TokenStats(p) => {
                self.report.token_stats = Some(p.clone());
            }
            JsonLogPayload::TurnEnd(p) => {
                self.report.stop_reason = Some(p.stop_reason.clone());
            }
            JsonLogPayload::Error(p) => {
                self.report.errors.push(p.error.clone());
            }
            _ => {}
        }
    }

    /// Consumes the collector and returns the final execution report.
    pub fn finish(self) -> HeadlessExecutionReport {
        self.report
    }

    /// Returns a reference to the current in-progress report.
    pub fn report(&self) -> &HeadlessExecutionReport {
        &self.report
    }
}

// ============================================================================
// Unit Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::duplex;

    #[test]
    fn test_event_kinds_serialization() {
        let kinds = vec![
            (JsonLogEventKind::SessionStart, "\"session_start\""),
            (JsonLogEventKind::TextDelta, "\"text_delta\""),
            (JsonLogEventKind::ToolStart, "\"tool_start\""),
            (JsonLogEventKind::AdvisorConsensus, "\"advisor_consensus\""),
            (JsonLogEventKind::Error, "\"error\""),
        ];

        for (kind, expected) in kinds {
            let json = serde_json::to_string(&kind).unwrap();
            assert_eq!(json, expected);
            let parsed: JsonLogEventKind = serde_json::from_str(&json).unwrap();
            assert_eq!(parsed, kind);
        }
    }

    #[test]
    fn test_ndjson_line_roundtrip() {
        let event = JsonLogEvent::text_delta(
            1,
            Some("sess-123".to_string()),
            "Hello world!",
            0,
            true,
            false,
        )
        .with_turn_id("turn-001")
        .with_meta("model", "gpt-4o");

        let line = event.to_ndjson_line().unwrap();
        assert!(line.ends_with('\n'), "NDJSON line must end with newline");
        assert!(
            !line[..line.len() - 1].contains('\n'),
            "NDJSON line must not contain unescaped newline"
        );

        let parsed = JsonLogEvent::from_ndjson_line(&line).unwrap();
        assert_eq!(parsed.seq, 1);
        assert_eq!(parsed.session_id.as_deref(), Some("sess-123"));
        assert_eq!(parsed.turn_id.as_deref(), Some("turn-001"));
        assert_eq!(parsed.kind, JsonLogEventKind::TextDelta);
        assert_eq!(
            parsed.metadata.get("model").unwrap(),
            &serde_json::json!("gpt-4o")
        );

        if let JsonLogPayload::TextDelta(payload) = parsed.payload {
            assert_eq!(payload.delta, "Hello world!");
            assert!(payload.is_first);
            assert!(!payload.is_last);
        } else {
            panic!("Expected TextDelta payload");
        }
    }

    #[test]
    fn test_batch_format_and_parse() {
        let events = vec![
            JsonLogEvent::session_start(
                1,
                "s-1",
                Some("claude-3-5-sonnet".to_string()),
                None,
                None,
            ),
            JsonLogEvent::text_delta(2, Some("s-1".to_string()), "Thinking...", 0, true, false),
            JsonLogEvent::tool_start(
                3,
                Some("s-1".to_string()),
                "c-1",
                "file_read",
                serde_json::json!({ "path": "Cargo.toml" }),
            ),
            JsonLogEvent::tool_finish(
                4,
                Some("s-1".to_string()),
                "c-1",
                "file_read",
                true,
                "[package]...",
                25,
                None,
            ),
            JsonLogEvent::status(5, Some("s-1".to_string()), "info", "Turn complete"),
        ];

        let batch_ndjson = format_ndjson_batch(&events).unwrap();
        assert_eq!(batch_ndjson.lines().count(), 5);

        let parsed_batch = parse_ndjson_lines(&batch_ndjson).unwrap();
        assert_eq!(parsed_batch.len(), 5);
        assert_eq!(parsed_batch[0].kind, JsonLogEventKind::SessionStart);
        assert_eq!(parsed_batch[2].kind, JsonLogEventKind::ToolStart);
        assert_eq!(parsed_batch[3].kind, JsonLogEventKind::ToolFinish);
    }

    #[tokio::test]
    async fn test_async_streamer_and_reader_duplex() {
        let (client_write, server_read) = duplex(4096);

        let mut streamer = JsonLogStreamer::new(client_write).with_session_id("sess-duplex");
        let mut reader = JsonLogReader::new(tokio::io::BufReader::new(server_read));

        // Spawn writer task
        tokio::spawn(async move {
            streamer
                .emit_text_delta("Part 1 ", 0, true, false)
                .await
                .unwrap();
            streamer
                .emit_text_delta("Part 2 ", 1, false, false)
                .await
                .unwrap();
            streamer
                .emit_tool_start("call-99", "bash", serde_json::json!({ "cmd": "ls" }))
                .await
                .unwrap();
            streamer
                .emit_tool_finish("call-99", "bash", true, "src\nCargo.toml", 15, None)
                .await
                .unwrap();
            streamer.flush().await.unwrap();
        });

        // Read from reader
        let e1 = reader.next_event().await.unwrap().expect("Event 1");
        assert_eq!(e1.seq, 1);
        assert_eq!(e1.kind, JsonLogEventKind::TextDelta);

        let e2 = reader.next_event().await.unwrap().expect("Event 2");
        assert_eq!(e2.seq, 2);
        assert_eq!(e2.kind, JsonLogEventKind::TextDelta);

        let e3 = reader.next_event().await.unwrap().expect("Event 3");
        assert_eq!(e3.seq, 3);
        assert_eq!(e3.kind, JsonLogEventKind::ToolStart);

        let e4 = reader.next_event().await.unwrap().expect("Event 4");
        assert_eq!(e4.seq, 4);
        assert_eq!(e4.kind, JsonLogEventKind::ToolFinish);

        if let JsonLogPayload::ToolFinish(tf) = e4.payload {
            assert_eq!(tf.call_id, "call-99");
            assert_eq!(tf.tool_name, "bash");
            assert!(tf.success);
            assert_eq!(tf.duration_ms, 15);
        } else {
            panic!("Expected ToolFinish payload");
        }
    }

    #[tokio::test]
    async fn test_broadcaster_multi_subscriber() {
        let broadcaster = JsonLogBroadcaster::new(32);
        let mut sub1 = broadcaster.subscribe();
        let mut sub2 = broadcaster.subscribe();

        let event = JsonLogEvent::status(
            100,
            Some("sess-bcast".to_string()),
            "warn",
            "Rate limit near",
        );
        broadcaster.publish(event.clone()).unwrap();

        let r1 = sub1.recv().await.unwrap();
        let r2 = sub2.recv().await.unwrap();

        assert_eq!(r1.seq, 100);
        assert_eq!(r2.seq, 100);
        assert_eq!(r1.kind, JsonLogEventKind::Status);
    }

    #[test]
    fn test_filter_engine() {
        let e_text = JsonLogEvent::text_delta(1, Some("s1".to_string()), "foo", 0, true, false);
        let e_tool = JsonLogEvent::tool_start(
            2,
            Some("s1".to_string()),
            "c1",
            "grep",
            serde_json::json!({}),
        );
        let e_other_sess = JsonLogEvent::tool_start(
            3,
            Some("s2".to_string()),
            "c2",
            "grep",
            serde_json::json!({}),
        );

        let filter_no_tokens = JsonLogFilter::new().with_tokens(false);
        assert!(!filter_no_tokens.matches(&e_text));
        assert!(filter_no_tokens.matches(&e_tool));

        let filter_s1 = JsonLogFilter::new().with_session_id("s1");
        assert!(filter_s1.matches(&e_tool));
        assert!(!filter_s1.matches(&e_other_sess));

        let filter_tools_only = JsonLogFilter::new()
            .with_allowed_kinds([JsonLogEventKind::ToolStart, JsonLogEventKind::ToolFinish]);
        assert!(filter_tools_only.matches(&e_tool));
        assert!(!filter_tools_only.matches(&e_text));
    }

    #[test]
    fn test_headless_event_collector() {
        let mut collector = HeadlessEventCollector::new();

        let events = vec![
            JsonLogEvent::text_delta(1, Some("sess-h".to_string()), "Hello ", 0, true, false),
            JsonLogEvent::text_delta(2, Some("sess-h".to_string()), "world!", 1, false, true),
            JsonLogEvent::thinking_delta(3, Some("sess-h".to_string()), "Thinking hard...", 0, 100),
            JsonLogEvent::tool_finish(
                4,
                Some("sess-h".to_string()),
                "call-1",
                "compile",
                true,
                "Build succeeded",
                250,
                None,
            ),
            JsonLogEvent::advisor_feedback(
                5,
                Some("sess-h".to_string()),
                "SecurityAdvisor",
                Some("security".to_string()),
                true,
                "Looks safe",
                vec!["Add more tests".to_string()],
                Some("info".to_string()),
                Some(0.95f32),
            ),
            JsonLogEvent::turn_end(
                6,
                Some("sess-h".to_string()),
                "turn-1",
                "end_turn",
                450,
                120,
                35,
                155,
                Some("Hello world!".to_string()),
            ),
        ];

        for ev in &events {
            collector.ingest(ev);
        }

        let report = collector.finish();
        assert_eq!(report.session_id.as_deref(), Some("sess-h"));
        assert_eq!(report.full_assistant_text, "Hello world!");
        assert_eq!(report.full_thinking_text, "Thinking hard...");
        assert_eq!(report.tools_executed.len(), 1);
        assert_eq!(report.tools_executed[0].tool_name, "compile");
        assert_eq!(report.advisor_critiques.len(), 1);
        assert_eq!(report.advisor_critiques[0].advisor, "SecurityAdvisor");
        assert_eq!(report.stop_reason.as_deref(), Some("end_turn"));
        assert_eq!(report.total_events, 6);
    }

    #[tokio::test]
    async fn test_file_logger() {
        let temp_dir = tempfile::tempdir().unwrap();
        let log_path = temp_dir.path().join("test_audit.jsonl");

        let mut logger = NdjsonFileLogger::open(&log_path).await.unwrap();
        let event1 =
            JsonLogEvent::status(1, Some("sess-file".to_string()), "info", "File log started");
        let event2 = JsonLogEvent::text_delta(
            2,
            Some("sess-file".to_string()),
            "Logged data",
            0,
            true,
            true,
        );

        logger.append(&event1).await.unwrap();
        logger.append(&event2).await.unwrap();
        logger.flush().await.unwrap();

        // Read back with JsonLogReader
        let file = tokio::fs::File::open(&log_path).await.unwrap();
        let mut reader = JsonLogReader::new(tokio::io::BufReader::new(file));
        let read_events = reader.read_all().await.unwrap();

        assert_eq!(read_events.len(), 2);
        assert_eq!(read_events[0].seq, 1);
        assert_eq!(read_events[1].seq, 2);
        assert_eq!(read_events[0].kind, JsonLogEventKind::Status);
        assert_eq!(read_events[1].kind, JsonLogEventKind::TextDelta);
    }

    #[test]
    fn test_from_agent_and_acp_events() {
        let agent_ev = AgentEvent::ToolStarted {
            id: "t-1".to_string(),
            name: "test_tool".to_string(),
            args: serde_json::json!({ "arg1": "val1" }),
        };
        let log_ev = JsonLogEvent::from_agent_event(42, Some("sess-conv"), &agent_ev);
        assert_eq!(log_ev.seq, 42);
        assert_eq!(log_ev.kind, JsonLogEventKind::ToolStart);

        let acp_ev = AcpSessionEvent::token(10, "streamed chunk", false, false, 50);
        let log_acp = JsonLogEvent::from_acp_session_event(43, Some("sess-conv"), &acp_ev);
        assert_eq!(log_acp.seq, 43);
        assert_eq!(log_acp.kind, JsonLogEventKind::TextDelta);
    }
}
