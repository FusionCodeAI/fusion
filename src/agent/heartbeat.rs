//! Subagent Heartbeat Monitor and Hang Detector.
//!
//! Provides real-time health tracking, stall detection, hang diagnosis, and
//! automated recovery for background subagent tasks.
//!
//! ### Key Capabilities:
//! 1. **Continuous Heartbeat Ingestion**: Subagents report heartbeats, phase transitions,
//!    progress increments, and execution metrics.
//! 2. **Multi-level Stall & Hang Diagnosis**: Evaluates missed heartbeats, stuck phases,
//!    progress stagnation, and hard execution deadlines.
//! 3. **Intelligent Diagnostics & Root Cause Analysis**: Identifies if a subagent is stuck in
//!    LLM inference, long tool runs, peer deadlocks, or resource starvation.
//! 4. **Automated Recovery Policies**: Ping probes, phase interrupts, cancellation, restarts,
//!    and parent escalations.
//! 5. **Event-Driven Pub-Sub Fabric**: Broadcasts health transitions and hang alerts via Tokio channels.
//! 6. **RAII Subagent Heartbeat Handle**: Zero-friction ergonomic handle for running subagents.
//! 7. **Terminal ASCII & Diagnostic Reports**: Formatted status dashboards and hang inspection reports.
//! 8. **Built-in Agent Tool**: `subagent_heartbeat_monitor` tool for primary agents to query and manage subagents.

use std::collections::{HashMap, VecDeque};
use std::fmt;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tokio::sync::{broadcast, RwLock};
use tokio::task::JoinHandle;

use crate::tools::types::{Tool, ToolContext};

// ============================================================================
// 1. Constants & Defaults
// ============================================================================

/// Default expected heartbeat interval (5 seconds).
pub const DEFAULT_HEARTBEAT_INTERVAL: Duration = Duration::from_secs(5);
/// Default warning threshold for missed heartbeats (15 seconds).
pub const DEFAULT_WARNING_THRESHOLD: Duration = Duration::from_secs(15);
/// Default threshold to mark a subagent as Stalled (30 seconds).
pub const DEFAULT_STALL_THRESHOLD: Duration = Duration::from_secs(30);
/// Default threshold to mark a subagent as Hung (60 seconds).
pub const DEFAULT_HANG_THRESHOLD: Duration = Duration::from_secs(60);
/// Default threshold to mark a subagent as Dead / unreachable (180 seconds).
pub const DEFAULT_DEAD_THRESHOLD: Duration = Duration::from_secs(180);
/// Default maximum phase duration before flag (120 seconds).
pub const DEFAULT_MAX_PHASE_DURATION: Duration = Duration::from_secs(120);
/// Default maximum duration without progress updates (180 seconds).
pub const DEFAULT_MAX_STUCK_PROGRESS_DURATION: Duration = Duration::from_secs(180);
/// Maximum history of heartbeats retained per subagent for diagnostics.
pub const MAX_HEARTBEAT_HISTORY: usize = 32;
/// Maximum phase transition history retained per subagent.
pub const MAX_PHASE_HISTORY: usize = 32;
/// Default broadcast channel capacity for heartbeat events.
pub const DEFAULT_EVENT_CHANNEL_CAPACITY: usize = 512;

// ============================================================================
// 2. Subagent Phase & Execution States
// ============================================================================

/// Granular operational phases of a running subagent.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SubagentPhase {
    /// Subagent is initializing runtime context, tools, and prompts.
    Initializing,
    /// Subagent is formulating strategy or reasoning locally.
    Thinking,
    /// Subagent is waiting for LLM provider response.
    PromptingLlm,
    /// Subagent is streaming tokens from LLM provider.
    StreamingResponse,
    /// Subagent is executing a specific tool.
    ExecutingTool { tool_name: String },
    /// Subagent is waiting for a peer agent response or RPC.
    WaitingForPeer { peer_id: String },
    /// Subagent is waiting for a shared resource lock.
    WaitingForResource { resource: String },
    /// Subagent is writing or modifying workspace files.
    WritingFiles,
    /// Subagent is executing tests or compilation commands.
    RunningTests,
    /// Subagent is analyzing outputs or performing evaluation.
    AnalyzingResults,
    /// Subagent is idle / waiting for next turn or instruction.
    Idle,
    /// Subagent is completing execution and returning artifacts.
    Finishing,
    /// Custom domain-specific phase.
    Custom(String),
}

impl fmt::Display for SubagentPhase {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Initializing => write!(f, "Initializing"),
            Self::Thinking => write!(f, "Thinking"),
            Self::PromptingLlm => write!(f, "Prompting LLM"),
            Self::StreamingResponse => write!(f, "Streaming Response"),
            Self::ExecutingTool { tool_name } => write!(f, "Tool: {tool_name}"),
            Self::WaitingForPeer { peer_id } => write!(f, "Waiting for Peer: {peer_id}"),
            Self::WaitingForResource { resource } => write!(f, "Waiting for Resource: {resource}"),
            Self::WritingFiles => write!(f, "Writing Files"),
            Self::RunningTests => write!(f, "Running Tests"),
            Self::AnalyzingResults => write!(f, "Analyzing Results"),
            Self::Idle => write!(f, "Idle"),
            Self::Finishing => write!(f, "Finishing"),
            Self::Custom(name) => write!(f, "{name}"),
        }
    }
}

// ============================================================================
// 3. Health & Status Classifications
// ============================================================================

/// High-level lifecycle status of a monitored subagent.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HeartbeatStatus {
    /// Actively executing instructions.
    Active,
    /// Execution delayed or heartbeat overdue.
    Stalled,
    /// Inactive / uncommunicative beyond critical hang threshold.
    Hung,
    /// Exceeded maximum allowed task execution duration.
    TimedOut,
    /// Successfully completed its task.
    Completed,
    /// Failed with an error.
    Failed,
    /// Cancelled explicitly or by recovery policy.
    Cancelled,
    /// Forcibly terminated.
    Terminated,
}

impl fmt::Display for HeartbeatStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Active => write!(f, "Active"),
            Self::Stalled => write!(f, "Stalled"),
            Self::Hung => write!(f, "Hung"),
            Self::TimedOut => write!(f, "Timed Out"),
            Self::Completed => write!(f, "Completed"),
            Self::Failed => write!(f, "Failed"),
            Self::Cancelled => write!(f, "Cancelled"),
            Self::Terminated => write!(f, "Terminated"),
        }
    }
}

/// Granular health condition of a monitored subagent.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HealthStatus {
    /// Heartbeats arriving regularly, normal progress.
    Healthy,
    /// Heartbeat delayed beyond warning threshold but not yet stalled.
    Degraded,
    /// Heartbeat missing beyond stall threshold.
    Stalled,
    /// Critical hang detected; subagent is unresponsive.
    Hung,
    /// Completely unreachable beyond dead threshold.
    Dead,
    /// Hard execution deadline exceeded.
    TimedOut,
}

impl fmt::Display for HealthStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Healthy => write!(f, "Healthy"),
            Self::Degraded => write!(f, "Degraded (Warning)"),
            Self::Stalled => write!(f, "Stalled"),
            Self::Hung => write!(f, "HUNG (Critical)"),
            Self::Dead => write!(f, "DEAD / Unreachable"),
            Self::TimedOut => write!(f, "Timed Out"),
        }
    }
}

/// Diagnosed root causes for subagent stalls and hangs.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HangReason {
    /// Subagent stopped sending heartbeats entirely.
    MissedHeartbeats,
    /// Subagent has spent excessive time in a single phase without transition.
    PhaseTimeout,
    /// Subagent continues ticking but progress percentage has not advanced.
    StuckProgress,
    /// Task exceeded its configured maximum total runtime deadline.
    TaskDeadlineExceeded,
    /// Deadlock or long wait on a peer subagent or coordination channel.
    PeerUnresponsive,
    /// Deadlock or long wait acquiring a shared file or resource lock.
    ResourceContention,
    /// LLM inference or streaming stalled with no tokens received.
    UnresponsiveLlm,
    /// External tool execution (e.g. bash, git, compiler) hung.
    StuckToolExecution,
    /// Suspected infinite loop or recursive invocation without state change.
    SuspectedInfiniteLoop,
    /// Custom diagnostic reason.
    Custom(String),
}

impl fmt::Display for HangReason {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissedHeartbeats => write!(f, "Missed periodic heartbeats"),
            Self::PhaseTimeout => write!(f, "Exceeded maximum phase execution duration"),
            Self::StuckProgress => write!(f, "Progress stagnation (ticking without advancement)"),
            Self::TaskDeadlineExceeded => write!(f, "Exceeded overall task execution deadline"),
            Self::PeerUnresponsive => write!(f, "Waiting on unresponsive peer agent"),
            Self::ResourceContention => write!(f, "Blocked on shared resource / file lock"),
            Self::UnresponsiveLlm => write!(f, "Unresponsive LLM API call / stream hung"),
            Self::StuckToolExecution => write!(f, "Tool execution process stalled / blocked"),
            Self::SuspectedInfiniteLoop => write!(f, "Suspected infinite loop / recursion"),
            Self::Custom(msg) => write!(f, "{msg}"),
        }
    }
}

/// Recovery action recommended or automatically triggered for a hung subagent.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RecoveryAction {
    /// No action required.
    None,
    /// Send an explicit ping/probe signal to check responsiveness.
    Ping,
    /// Emit a warning alert to logs / monitors.
    Warn,
    /// Interrupt current tool or phase without killing subagent.
    InterruptCurrentPhase,
    /// Cancel the subagent gracefully.
    Cancel,
    /// Cancel and restart the subagent with identical task spec.
    Restart,
    /// Forcibly terminate the subagent task.
    ForceKill,
    /// Escalate issue to parent agent or human operator.
    EscalateToParent,
    /// Custom recovery handler.
    Custom(String),
}

impl fmt::Display for RecoveryAction {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::None => write!(f, "None"),
            Self::Ping => write!(f, "Ping Probe"),
            Self::Warn => write!(f, "Warning Alert"),
            Self::InterruptCurrentPhase => write!(f, "Interrupt Phase"),
            Self::Cancel => write!(f, "Cancel Subagent"),
            Self::Restart => write!(f, "Restart Subagent"),
            Self::ForceKill => write!(f, "Force Kill"),
            Self::EscalateToParent => write!(f, "Escalate to Parent"),
            Self::Custom(name) => write!(f, "Custom: {name}"),
        }
    }
}

// ============================================================================
// 4. Metrics & Records
// ============================================================================

/// Execution metrics attached to heartbeats.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct HeartbeatMetrics {
    /// Total dialogue turns or reasoning iterations completed.
    pub turns_completed: u32,
    /// Total tool invocations executed.
    pub tools_executed: u32,
    /// Total tokens consumed (prompt + completion).
    pub tokens_used: u64,
    /// Total files created or edited.
    pub files_modified: u32,
    /// Estimated memory usage in bytes (if available).
    pub memory_bytes: Option<u64>,
    /// Custom user-defined numeric metrics.
    pub custom: HashMap<String, f64>,
}

/// A snapshot of a single heartbeat event.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HeartbeatRecord {
    /// Sequence number of this heartbeat for the subagent.
    pub sequence: u64,
    /// Timestamp when heartbeat was recorded.
    pub timestamp: DateTime<Utc>,
    /// Current operational phase.
    pub phase: SubagentPhase,
    /// Completion progress percentage (0.0 to 1.0).
    pub progress: Option<f32>,
    /// Optional description of current active step.
    pub current_step: Option<String>,
    /// Execution metrics at time of heartbeat.
    pub metrics: HeartbeatMetrics,
    /// Optional status or diagnostic message.
    pub message: Option<String>,
}

/// Record of a phase transition.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PhaseTransition {
    /// Source phase.
    pub from: SubagentPhase,
    /// Target phase.
    pub to: SubagentPhase,
    /// Timestamp of transition.
    pub timestamp: DateTime<Utc>,
    /// Duration spent in previous phase.
    pub duration_in_prev_phase: Duration,
    /// Step description at transition time.
    pub step: Option<String>,
}

// ============================================================================
// 5. Configurable Thresholds & Policies
// ============================================================================

/// Timeout and detection thresholds for subagents.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HeartbeatThresholds {
    /// Expected interval between consecutive heartbeats.
    pub heartbeat_interval: Duration,
    /// Delay threshold before emitting warning.
    pub warning_threshold: Duration,
    /// Overdue duration before marking Stalled.
    pub stall_threshold: Duration,
    /// Overdue duration before marking Hung.
    pub hang_threshold: Duration,
    /// Overdue duration before marking Dead.
    pub dead_threshold: Duration,
    /// Maximum allowed duration in a single phase without transition.
    pub max_phase_duration: Duration,
    /// Maximum allowed duration without progress advancement.
    pub max_stuck_progress_duration: Duration,
    /// Hard cap on overall task execution duration.
    pub max_task_duration: Option<Duration>,
    /// Phase-specific timeout overrides (e.g. "executing_tool" -> 180s).
    pub phase_timeouts: HashMap<String, Duration>,
}

impl Default for HeartbeatThresholds {
    fn default() -> Self {
        Self {
            heartbeat_interval: DEFAULT_HEARTBEAT_INTERVAL,
            warning_threshold: DEFAULT_WARNING_THRESHOLD,
            stall_threshold: DEFAULT_STALL_THRESHOLD,
            hang_threshold: DEFAULT_HANG_THRESHOLD,
            dead_threshold: DEFAULT_DEAD_THRESHOLD,
            max_phase_duration: DEFAULT_MAX_PHASE_DURATION,
            max_stuck_progress_duration: DEFAULT_MAX_STUCK_PROGRESS_DURATION,
            max_task_duration: Some(Duration::from_secs(600)), // 10 minutes default max
            phase_timeouts: HashMap::new(),
        }
    }
}

impl HeartbeatThresholds {
    /// Creates thresholds tailored for quick interactive subagents.
    pub fn fast() -> Self {
        Self {
            heartbeat_interval: Duration::from_secs(2),
            warning_threshold: Duration::from_secs(6),
            stall_threshold: Duration::from_secs(12),
            hang_threshold: Duration::from_secs(25),
            dead_threshold: Duration::from_secs(60),
            max_phase_duration: Duration::from_secs(45),
            max_stuck_progress_duration: Duration::from_secs(60),
            max_task_duration: Some(Duration::from_secs(180)),
            phase_timeouts: HashMap::new(),
        }
    }

    /// Creates thresholds tailored for long-running batch or compilation tasks.
    pub fn long_running() -> Self {
        Self {
            heartbeat_interval: Duration::from_secs(10),
            warning_threshold: Duration::from_secs(30),
            stall_threshold: Duration::from_secs(60),
            hang_threshold: Duration::from_secs(180),
            dead_threshold: Duration::from_secs(360),
            max_phase_duration: Duration::from_secs(300),
            max_stuck_progress_duration: Duration::from_secs(300),
            max_task_duration: Some(Duration::from_secs(1800)), // 30 minutes
            phase_timeouts: HashMap::new(),
        }
    }

    /// Returns the applicable timeout for a given phase.
    pub fn timeout_for_phase(&self, phase: &SubagentPhase) -> Duration {
        let key = match phase {
            SubagentPhase::Initializing => "initializing",
            SubagentPhase::Thinking => "thinking",
            SubagentPhase::PromptingLlm => "prompting_llm",
            SubagentPhase::StreamingResponse => "streaming_response",
            SubagentPhase::ExecutingTool { .. } => "executing_tool",
            SubagentPhase::WaitingForPeer { .. } => "waiting_for_peer",
            SubagentPhase::WaitingForResource { .. } => "waiting_for_resource",
            SubagentPhase::WritingFiles => "writing_files",
            SubagentPhase::RunningTests => "running_tests",
            SubagentPhase::AnalyzingResults => "analyzing_results",
            SubagentPhase::Idle => "idle",
            SubagentPhase::Finishing => "finishing",
            SubagentPhase::Custom(s) => s.as_str(),
        };

        self.phase_timeouts
            .get(key)
            .copied()
            .unwrap_or(self.max_phase_duration)
    }
}

// ============================================================================
// 6. Monitored Subagent State
// ============================================================================

/// Full internal state of a registered and monitored subagent.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MonitoredSubagent {
    /// Unique subagent identifier.
    pub id: String,
    /// Human-friendly subagent name (e.g. "Scout1", "CoderRefactor").
    pub name: String,
    /// Subagent role or archetype.
    pub role: String,
    /// Optional identifier of parent agent that spawned this subagent.
    pub parent_id: Option<String>,
    /// Optional high-level task description.
    pub task_description: Option<String>,
    /// Timestamp when subagent was registered.
    pub registered_at: DateTime<Utc>,
    /// Timestamp of most recent heartbeat.
    pub last_heartbeat_at: DateTime<Utc>,
    /// Timestamp when progress percentage last advanced.
    pub last_progress_at: DateTime<Utc>,
    /// Timestamp when phase last changed.
    pub last_phase_change_at: DateTime<Utc>,
    /// Current operational phase.
    pub current_phase: SubagentPhase,
    /// Current step description.
    pub current_step: Option<String>,
    /// Current progress percentage (0.0 to 1.0).
    pub current_progress: Option<f32>,
    /// Total heartbeats received so far.
    pub heartbeat_count: u64,
    /// Lifecycle status.
    pub status: HeartbeatStatus,
    /// Computed health status.
    pub health: HealthStatus,
    /// Latest cumulative metrics.
    pub metrics: HeartbeatMetrics,
    /// Recent heartbeat records (ring buffer).
    pub recent_heartbeats: VecDeque<HeartbeatRecord>,
    /// Recent phase transitions (ring buffer).
    pub phase_history: VecDeque<PhaseTransition>,
    /// Custom threshold overrides (if any).
    pub custom_thresholds: Option<HeartbeatThresholds>,
    /// Metadata tags.
    pub metadata: HashMap<String, String>,
    /// Last recorded completion result or error.
    pub exit_result: Option<String>,
}

impl MonitoredSubagent {
    /// Creates a new monitored subagent entry.
    pub fn new(
        id: impl Into<String>,
        name: impl Into<String>,
        role: impl Into<String>,
        parent_id: Option<String>,
        task_description: Option<String>,
        thresholds: Option<HeartbeatThresholds>,
    ) -> Self {
        let now = Utc::now();
        Self {
            id: id.into(),
            name: name.into(),
            role: role.into(),
            parent_id,
            task_description,
            registered_at: now,
            last_heartbeat_at: now,
            last_progress_at: now,
            last_phase_change_at: now,
            current_phase: SubagentPhase::Initializing,
            current_step: None,
            current_progress: Some(0.0),
            heartbeat_count: 0,
            status: HeartbeatStatus::Active,
            health: HealthStatus::Healthy,
            metrics: HeartbeatMetrics::default(),
            recent_heartbeats: VecDeque::with_capacity(MAX_HEARTBEAT_HISTORY),
            phase_history: VecDeque::with_capacity(MAX_PHASE_HISTORY),
            custom_thresholds: thresholds,
            metadata: HashMap::new(),
            exit_result: None,
        }
    }

    /// Total runtime duration from registration to now (or completion).
    pub fn total_runtime(&self, now: DateTime<Utc>) -> Duration {
        let elapsed_millis = (now - self.registered_at).num_milliseconds().max(0) as u64;
        Duration::from_millis(elapsed_millis)
    }

    /// Elapsed duration since last heartbeat.
    pub fn elapsed_since_heartbeat(&self, now: DateTime<Utc>) -> Duration {
        let elapsed_millis = (now - self.last_heartbeat_at).num_milliseconds().max(0) as u64;
        Duration::from_millis(elapsed_millis)
    }

    /// Elapsed duration in current phase.
    pub fn elapsed_in_current_phase(&self, now: DateTime<Utc>) -> Duration {
        let elapsed_millis = (now - self.last_phase_change_at).num_milliseconds().max(0) as u64;
        Duration::from_millis(elapsed_millis)
    }

    /// Elapsed duration since last progress update.
    pub fn elapsed_since_progress(&self, now: DateTime<Utc>) -> Duration {
        let elapsed_millis = (now - self.last_progress_at).num_milliseconds().max(0) as u64;
        Duration::from_millis(elapsed_millis)
    }

    /// Whether subagent is in a terminal state (Completed, Failed, Cancelled, Terminated).
    pub fn is_finished(&self) -> bool {
        matches!(
            self.status,
            HeartbeatStatus::Completed
                | HeartbeatStatus::Failed
                | HeartbeatStatus::Cancelled
                | HeartbeatStatus::Terminated
        )
    }
}

// ============================================================================
// 7. Hang Diagnosis & Diagnostics
// ============================================================================

/// Detailed diagnosis report for a subagent experiencing health issues.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HangDiagnosis {
    /// Subagent identifier.
    pub subagent_id: String,
    /// Subagent human-friendly name.
    pub subagent_name: String,
    /// Subagent role.
    pub subagent_role: String,
    /// Health condition evaluated.
    pub health: HealthStatus,
    /// Primary root cause reason.
    pub reason: HangReason,
    /// Elapsed duration since last heartbeat was received.
    pub elapsed_since_heartbeat: Duration,
    /// Elapsed duration spent in the current phase.
    pub elapsed_in_current_phase: Duration,
    /// Total execution runtime so far.
    pub total_runtime: Duration,
    /// Operational phase at time of diagnosis.
    pub current_phase: SubagentPhase,
    /// Current step description (if set).
    pub current_step: Option<String>,
    /// Recommended recovery action.
    pub recommended_action: RecoveryAction,
    /// Human-readable diagnosis summary.
    pub diagnosis_summary: String,
    /// Step-by-step suggested recovery recommendations.
    pub suggested_recovery_steps: Vec<String>,
}

impl fmt::Display for HangDiagnosis {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(
            f,
            "[{}] Subagent '{}' ({}) - {}",
            self.health, self.subagent_name, self.subagent_id, self.reason
        )?;
        writeln!(f, "  Phase:       {}", self.current_phase)?;
        if let Some(step) = &self.current_step {
            writeln!(f, "  Step:        {}", step)?;
        }
        writeln!(
            f,
            "  Last seen:   {:.1}s ago | Phase duration: {:.1}s | Total runtime: {:.1}s",
            self.elapsed_since_heartbeat.as_secs_f64(),
            self.elapsed_in_current_phase.as_secs_f64(),
            self.total_runtime.as_secs_f64()
        )?;
        writeln!(f, "  Summary:     {}", self.diagnosis_summary)?;
        writeln!(f, "  Recommended: {}", self.recommended_action)?;
        if !self.suggested_recovery_steps.is_empty() {
            writeln!(f, "  Steps:")?;
            for step in &self.suggested_recovery_steps {
                writeln!(f, "    - {step}")?;
            }
        }
        Ok(())
    }
}

// ============================================================================
// 8. Event System
// ============================================================================

/// Real-time events broadcast by the heartbeat monitor.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum HeartbeatEvent {
    /// New subagent registered for monitoring.
    SubagentRegistered {
        id: String,
        name: String,
        role: String,
        timestamp: DateTime<Utc>,
    },
    /// Periodic heartbeat received.
    HeartbeatReceived {
        id: String,
        sequence: u64,
        phase: SubagentPhase,
        progress: Option<f32>,
        timestamp: DateTime<Utc>,
    },
    /// Operational phase transition occurred.
    PhaseChanged {
        id: String,
        from: SubagentPhase,
        to: SubagentPhase,
        duration: Duration,
        timestamp: DateTime<Utc>,
    },
    /// Health degraded to warning/stalled.
    HealthDegraded {
        id: String,
        health: HealthStatus,
        diagnosis: HangDiagnosis,
        timestamp: DateTime<Utc>,
    },
    /// Subagent diagnosed as hung.
    HangDetected {
        id: String,
        diagnosis: HangDiagnosis,
        timestamp: DateTime<Utc>,
    },
    /// Hard execution deadline reached.
    TaskTimedOut {
        id: String,
        total_duration: Duration,
        timestamp: DateTime<Utc>,
    },
    /// Automated or manual recovery action triggered.
    RecoveryTriggered {
        id: String,
        action: RecoveryAction,
        details: String,
        timestamp: DateTime<Utc>,
    },
    /// Subagent completed successfully.
    SubagentCompleted {
        id: String,
        total_duration: Duration,
        metrics: HeartbeatMetrics,
        timestamp: DateTime<Utc>,
    },
    /// Subagent encountered an error.
    SubagentFailed {
        id: String,
        error: String,
        total_duration: Duration,
        timestamp: DateTime<Utc>,
    },
    /// Subagent cancelled.
    SubagentCancelled {
        id: String,
        reason: String,
        timestamp: DateTime<Utc>,
    },
}

/// Result of executing a recovery action.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RecoveryResult {
    pub subagent_id: String,
    pub action: RecoveryAction,
    pub success: bool,
    pub details: String,
    pub timestamp: DateTime<Utc>,
}

// ============================================================================
// 9. HeartbeatMonitor Core Engine
// ============================================================================

/// Thread-safe central engine for monitoring subagent heartbeats and detecting hangs.
#[derive(Clone)]
pub struct HeartbeatMonitor {
    subagents: Arc<RwLock<HashMap<String, MonitoredSubagent>>>,
    default_thresholds: Arc<RwLock<HeartbeatThresholds>>,
    event_tx: broadcast::Sender<HeartbeatEvent>,
}

impl Default for HeartbeatMonitor {
    fn default() -> Self {
        Self::new(HeartbeatThresholds::default())
    }
}

impl HeartbeatMonitor {
    /// Creates a new HeartbeatMonitor with specific default thresholds.
    pub fn new(thresholds: HeartbeatThresholds) -> Self {
        let (event_tx, _) = broadcast::channel(DEFAULT_EVENT_CHANNEL_CAPACITY);
        Self {
            subagents: Arc::new(RwLock::new(HashMap::new())),
            default_thresholds: Arc::new(RwLock::new(thresholds)),
            event_tx,
        }
    }

    /// Subscribes to real-time heartbeat and hang detection events.
    pub fn subscribe(&self) -> broadcast::Receiver<HeartbeatEvent> {
        self.event_tx.subscribe()
    }

    /// Updates global default thresholds.
    pub async fn set_default_thresholds(&self, thresholds: HeartbeatThresholds) {
        let mut th = self.default_thresholds.write().await;
        *th = thresholds;
    }

    /// Returns a copy of the current default thresholds.
    pub async fn default_thresholds(&self) -> HeartbeatThresholds {
        self.default_thresholds.read().await.clone()
    }

    // ------------------------------------------------------------------------
    // Subagent Lifecycle & Registration
    // ------------------------------------------------------------------------

    /// Registers a new subagent to be monitored and returns an ergonomic RAII handle.
    pub async fn register(
        &self,
        id: impl Into<String>,
        name: impl Into<String>,
        role: impl Into<String>,
        parent_id: Option<String>,
        task_description: Option<String>,
        thresholds: Option<HeartbeatThresholds>,
    ) -> SubagentHeartbeatHandle {
        let id_str = id.into();
        let name_str = name.into();
        let role_str = role.into();

        let subagent = MonitoredSubagent::new(
            id_str.clone(),
            name_str.clone(),
            role_str.clone(),
            parent_id,
            task_description,
            thresholds,
        );

        {
            let mut map = self.subagents.write().await;
            map.insert(id_str.clone(), subagent);
        }

        let _ = self.event_tx.send(HeartbeatEvent::SubagentRegistered {
            id: id_str.clone(),
            name: name_str,
            role: role_str,
            timestamp: Utc::now(),
        });

        SubagentHeartbeatHandle::new(id_str, self.clone())
    }

    /// Unregisters or removes a subagent record.
    pub async fn unregister(&self, subagent_id: &str) -> Option<MonitoredSubagent> {
        let mut map = self.subagents.write().await;
        map.remove(subagent_id)
    }

    // ------------------------------------------------------------------------
    // Heartbeat & Progress Recording
    // ------------------------------------------------------------------------

    /// Records a periodic heartbeat tick from a subagent.
    pub async fn record_heartbeat(
        &self,
        subagent_id: &str,
        phase: Option<SubagentPhase>,
        progress: Option<f32>,
        current_step: Option<String>,
        metrics: Option<HeartbeatMetrics>,
        message: Option<String>,
    ) -> bool {
        let now = Utc::now();
        let mut map = self.subagents.write().await;
        let subagent = match map.get_mut(subagent_id) {
            Some(s) => s,
            None => return false,
        };

        if subagent.is_finished() {
            return false;
        }

        subagent.heartbeat_count += 1;
        subagent.last_heartbeat_at = now;
        subagent.health = HealthStatus::Healthy;
        subagent.status = HeartbeatStatus::Active;

        // Check if progress updated
        if let Some(new_p) = progress {
            if subagent.current_progress != Some(new_p) {
                subagent.current_progress = Some(new_p);
                subagent.last_progress_at = now;
            }
        }

        // Check if step updated
        if let Some(step) = current_step.clone() {
            subagent.current_step = Some(step);
        }

        // Check if phase changed
        if let Some(new_phase) = phase.clone() {
            if subagent.current_phase != new_phase {
                let prev_phase = subagent.current_phase.clone();
                let duration_in_prev = subagent.elapsed_in_current_phase(now);

                let transition = PhaseTransition {
                    from: prev_phase.clone(),
                    to: new_phase.clone(),
                    timestamp: now,
                    duration_in_prev_phase: duration_in_prev,
                    step: current_step.clone(),
                };

                if subagent.phase_history.len() >= MAX_PHASE_HISTORY {
                    subagent.phase_history.pop_front();
                }
                subagent.phase_history.push_back(transition);

                subagent.current_phase = new_phase.clone();
                subagent.last_phase_change_at = now;

                let _ = self.event_tx.send(HeartbeatEvent::PhaseChanged {
                    id: subagent_id.to_string(),
                    from: prev_phase,
                    to: new_phase.clone(),
                    duration: duration_in_prev,
                    timestamp: now,
                });
            }
        }

        if let Some(m) = metrics.clone() {
            subagent.metrics = m;
        }

        let record = HeartbeatRecord {
            sequence: subagent.heartbeat_count,
            timestamp: now,
            phase: subagent.current_phase.clone(),
            progress: subagent.current_progress,
            current_step: subagent.current_step.clone(),
            metrics: subagent.metrics.clone(),
            message,
        };

        if subagent.recent_heartbeats.len() >= MAX_HEARTBEAT_HISTORY {
            subagent.recent_heartbeats.pop_front();
        }
        subagent.recent_heartbeats.push_back(record);

        let _ = self.event_tx.send(HeartbeatEvent::HeartbeatReceived {
            id: subagent_id.to_string(),
            sequence: subagent.heartbeat_count,
            phase: subagent.current_phase.clone(),
            progress: subagent.current_progress,
            timestamp: now,
        });

        true
    }

    /// Explicitly updates the active operational phase of a subagent.
    pub async fn update_phase(
        &self,
        subagent_id: &str,
        phase: SubagentPhase,
        current_step: Option<String>,
    ) -> bool {
        self.record_heartbeat(subagent_id, Some(phase), None, current_step, None, None)
            .await
    }

    /// Explicitly updates the progress and step description.
    pub async fn update_progress(
        &self,
        subagent_id: &str,
        progress: f32,
        current_step: Option<String>,
    ) -> bool {
        self.record_heartbeat(subagent_id, None, Some(progress), current_step, None, None)
            .await
    }

    /// Marks a subagent as successfully completed.
    pub async fn mark_completed(&self, subagent_id: &str, result_summary: Option<String>) -> bool {
        let now = Utc::now();
        let mut map = self.subagents.write().await;
        let subagent = match map.get_mut(subagent_id) {
            Some(s) => s,
            None => return false,
        };

        subagent.status = HeartbeatStatus::Completed;
        subagent.health = HealthStatus::Healthy;
        subagent.current_phase = SubagentPhase::Finishing;
        subagent.current_progress = Some(1.0);
        subagent.exit_result = result_summary;
        let total_duration = subagent.total_runtime(now);
        let metrics = subagent.metrics.clone();

        let _ = self.event_tx.send(HeartbeatEvent::SubagentCompleted {
            id: subagent_id.to_string(),
            total_duration,
            metrics,
            timestamp: now,
        });

        true
    }

    /// Marks a subagent as failed with an error message.
    pub async fn mark_failed(&self, subagent_id: &str, error: String) -> bool {
        let now = Utc::now();
        let mut map = self.subagents.write().await;
        let subagent = match map.get_mut(subagent_id) {
            Some(s) => s,
            None => return false,
        };

        subagent.status = HeartbeatStatus::Failed;
        subagent.health = HealthStatus::Degraded;
        subagent.exit_result = Some(error.clone());
        let total_duration = subagent.total_runtime(now);

        let _ = self.event_tx.send(HeartbeatEvent::SubagentFailed {
            id: subagent_id.to_string(),
            error,
            total_duration,
            timestamp: now,
        });

        true
    }

    /// Marks a subagent as cancelled.
    pub async fn mark_cancelled(&self, subagent_id: &str, reason: String) -> bool {
        let now = Utc::now();
        let mut map = self.subagents.write().await;
        let subagent = match map.get_mut(subagent_id) {
            Some(s) => s,
            None => return false,
        };

        subagent.status = HeartbeatStatus::Cancelled;
        subagent.exit_result = Some(reason.clone());

        let _ = self.event_tx.send(HeartbeatEvent::SubagentCancelled {
            id: subagent_id.to_string(),
            reason,
            timestamp: now,
        });

        true
    }

    // ------------------------------------------------------------------------
    // Hang & Stall Scanning & Diagnostics
    // ------------------------------------------------------------------------

    /// Scans all active subagents, evaluates health conditions, and produces diagnoses.
    pub async fn scan(&self) -> Vec<HangDiagnosis> {
        let now = Utc::now();
        let default_thresh = self.default_thresholds().await;
        let mut diagnoses = Vec::new();
        let mut events_to_emit = Vec::new();

        {
            let mut map = self.subagents.write().await;

            for (id, subagent) in map.iter_mut() {
                if subagent.is_finished() {
                    continue;
                }

                let thresh = subagent
                    .custom_thresholds
                    .as_ref()
                    .unwrap_or(&default_thresh);

                let elapsed_hb = subagent.elapsed_since_heartbeat(now);
                let elapsed_phase = subagent.elapsed_in_current_phase(now);
                let elapsed_progress = subagent.elapsed_since_progress(now);
                let total_runtime = subagent.total_runtime(now);

                let mut current_health = HealthStatus::Healthy;
                let mut hang_reason = None;
                let mut rec_action = RecoveryAction::None;
                let mut diagnosis_msg = String::new();
                let mut steps = Vec::new();

                // 1. Check hard task timeout
                if let Some(max_task_dur) = thresh.max_task_duration {
                    if total_runtime > max_task_dur {
                        current_health = HealthStatus::TimedOut;
                        hang_reason = Some(HangReason::TaskDeadlineExceeded);
                        rec_action = RecoveryAction::Cancel;
                        diagnosis_msg = format!(
                            "Task runtime ({:.1}s) exceeded hard execution deadline ({:.1}s)",
                            total_runtime.as_secs_f64(),
                            max_task_dur.as_secs_f64()
                        );
                        steps.push("Cancel subagent and collect partial outputs.".to_string());
                        steps.push("Partition task into smaller sub-slices if needed.".to_string());
                    }
                }

                // 2. Check missed heartbeats (Dead, Hung, Stalled, Degraded)
                if current_health == HealthStatus::Healthy {
                    if elapsed_hb >= thresh.dead_threshold {
                        current_health = HealthStatus::Dead;
                        hang_reason = Some(HangReason::MissedHeartbeats);
                        rec_action = RecoveryAction::ForceKill;
                        diagnosis_msg = format!(
                            "No heartbeat received for {:.1}s (exceeded dead threshold {:.1}s). Process likely terminated or deadlocked.",
                            elapsed_hb.as_secs_f64(),
                            thresh.dead_threshold.as_secs_f64()
                        );
                        steps.push("Force kill orphaned subagent process/task.".to_string());
                        steps.push("Inspect system resources and stderr logs.".to_string());
                    } else if elapsed_hb >= thresh.hang_threshold {
                        current_health = HealthStatus::Hung;
                        hang_reason = Some(HangReason::MissedHeartbeats);
                        rec_action = RecoveryAction::Cancel;
                        diagnosis_msg = format!(
                            "No heartbeat received for {:.1}s (exceeded hang threshold {:.1}s).",
                            elapsed_hb.as_secs_f64(),
                            thresh.hang_threshold.as_secs_f64()
                        );
                        steps.push("Send cancellation request to subagent.".to_string());
                        steps.push(
                            "Consider automatic retry if restart policy is enabled.".to_string(),
                        );
                    } else if elapsed_hb >= thresh.stall_threshold {
                        current_health = HealthStatus::Stalled;
                        hang_reason = Some(HangReason::MissedHeartbeats);
                        rec_action = RecoveryAction::Ping;
                        diagnosis_msg = format!(
                            "Heartbeat delayed by {:.1}s (exceeded stall threshold {:.1}s).",
                            elapsed_hb.as_secs_f64(),
                            thresh.stall_threshold.as_secs_f64()
                        );
                        steps
                            .push("Issue ping probe to check subagent responsiveness.".to_string());
                    } else if elapsed_hb >= thresh.warning_threshold {
                        current_health = HealthStatus::Degraded;
                        hang_reason = Some(HangReason::MissedHeartbeats);
                        rec_action = RecoveryAction::Warn;
                        diagnosis_msg = format!(
                            "Heartbeat delayed by {:.1}s (warning threshold {:.1}s).",
                            elapsed_hb.as_secs_f64(),
                            thresh.warning_threshold.as_secs_f64()
                        );
                        steps.push("Monitor subagent for potential stall.".to_string());
                    }
                }

                // 3. Check stuck operational phase (even if heartbeats are arriving)
                if current_health == HealthStatus::Healthy
                    || current_health == HealthStatus::Degraded
                {
                    let phase_timeout = thresh.timeout_for_phase(&subagent.current_phase);
                    if elapsed_phase >= phase_timeout {
                        let is_severe = elapsed_phase >= phase_timeout * 2;
                        current_health = if is_severe {
                            HealthStatus::Hung
                        } else {
                            HealthStatus::Stalled
                        };

                        let (reason, action, msg, sub_steps) = match &subagent.current_phase {
                            SubagentPhase::PromptingLlm | SubagentPhase::StreamingResponse => (
                                HangReason::UnresponsiveLlm,
                                if is_severe {
                                    RecoveryAction::Restart
                                } else {
                                    RecoveryAction::Ping
                                },
                                format!(
                                    "Stuck in LLM generation for {:.1}s (timeout: {:.1}s). Provider API may be hanging or rate-limited.",
                                    elapsed_phase.as_secs_f64(),
                                    phase_timeout.as_secs_f64()
                                ),
                                vec![
                                    "Check LLM provider API endpoint latency and rate limits.".to_string(),
                                    "Cancel and retry turn with a shorter completion timeout.".to_string(),
                                ],
                            ),
                            SubagentPhase::ExecutingTool { tool_name } => (
                                HangReason::StuckToolExecution,
                                if is_severe {
                                    RecoveryAction::InterruptCurrentPhase
                                } else {
                                    RecoveryAction::Warn
                                },
                                format!(
                                    "Tool '{tool_name}' execution has been running for {:.1}s (timeout: {:.1}s). Child command may be blocked on I/O or input.",
                                    elapsed_phase.as_secs_f64(),
                                    phase_timeout.as_secs_f64()
                                ),
                                vec![
                                    format!("Verify process spawned by tool '{tool_name}' is not awaiting terminal stdin."),
                                    "Send SIGINT or interrupt signal to tool process.".to_string(),
                                ],
                            ),
                            SubagentPhase::WaitingForPeer { peer_id } => (
                                HangReason::PeerUnresponsive,
                                RecoveryAction::EscalateToParent,
                                format!(
                                    "Subagent has been waiting on peer '{peer_id}' for {:.1}s (timeout: {:.1}s). Potential inter-agent deadlock.",
                                    elapsed_phase.as_secs_f64(),
                                    phase_timeout.as_secs_f64()
                                ),
                                vec![
                                    format!("Check status and health of target peer '{peer_id}'."),
                                    "Release peer mailbox lock or time out RPC query.".to_string(),
                                ],
                            ),
                            SubagentPhase::WaitingForResource { resource } => (
                                HangReason::ResourceContention,
                                RecoveryAction::InterruptCurrentPhase,
                                format!(
                                    "Blocked waiting for lock on resource '{resource}' for {:.1}s (timeout: {:.1}s).",
                                    elapsed_phase.as_secs_f64(),
                                    phase_timeout.as_secs_f64()
                                ),
                                vec![
                                    format!("Inspect current holder of resource '{resource}'."),
                                    "Force-break stale lock if owner agent died.".to_string(),
                                ],
                            ),
                            _ => (
                                HangReason::PhaseTimeout,
                                if is_severe {
                                    RecoveryAction::Cancel
                                } else {
                                    RecoveryAction::Warn
                                },
                                format!(
                                    "Subagent stuck in phase '{}' for {:.1}s (timeout: {:.1}s).",
                                    subagent.current_phase,
                                    elapsed_phase.as_secs_f64(),
                                    phase_timeout.as_secs_f64()
                                ),
                                vec![
                                    "Inspect subagent execution loop and break potential tight loop.".to_string(),
                                ],
                            ),
                        };

                        hang_reason = Some(reason);
                        rec_action = action;
                        diagnosis_msg = msg;
                        steps = sub_steps;
                    }
                }

                // 4. Check progress stagnation
                if current_health == HealthStatus::Healthy
                    && elapsed_progress >= thresh.max_stuck_progress_duration
                    && subagent.current_phase != SubagentPhase::Idle
                {
                    current_health = HealthStatus::Stalled;
                    hang_reason = Some(HangReason::StuckProgress);
                    rec_action = RecoveryAction::Warn;
                    diagnosis_msg = format!(
                        "No progress advancement for {:.1}s while actively in phase '{}'. Suspected infinite loop or thrashing.",
                        elapsed_progress.as_secs_f64(),
                        subagent.current_phase
                    );
                    steps.push(
                        "Check if subagent is repeatedly issuing failing edits or queries."
                            .to_string(),
                    );
                }

                // Update subagent status & health
                subagent.health = current_health;
                if current_health == HealthStatus::Hung || current_health == HealthStatus::Dead {
                    subagent.status = HeartbeatStatus::Hung;
                } else if current_health == HealthStatus::Stalled {
                    subagent.status = HeartbeatStatus::Stalled;
                } else if current_health == HealthStatus::TimedOut {
                    subagent.status = HeartbeatStatus::TimedOut;
                }

                // If non-healthy, create diagnosis record
                if current_health != HealthStatus::Healthy {
                    let diagnosis = HangDiagnosis {
                        subagent_id: id.clone(),
                        subagent_name: subagent.name.clone(),
                        subagent_role: subagent.role.clone(),
                        health: current_health,
                        reason: hang_reason.unwrap_or(HangReason::MissedHeartbeats),
                        elapsed_since_heartbeat: elapsed_hb,
                        elapsed_in_current_phase: elapsed_phase,
                        total_runtime,
                        current_phase: subagent.current_phase.clone(),
                        current_step: subagent.current_step.clone(),
                        recommended_action: rec_action,
                        diagnosis_summary: diagnosis_msg,
                        suggested_recovery_steps: steps,
                    };

                    diagnoses.push(diagnosis.clone());

                    if current_health == HealthStatus::Hung || current_health == HealthStatus::Dead
                    {
                        events_to_emit.push(HeartbeatEvent::HangDetected {
                            id: id.clone(),
                            diagnosis: diagnosis.clone(),
                            timestamp: now,
                        });
                    } else if current_health == HealthStatus::TimedOut {
                        events_to_emit.push(HeartbeatEvent::TaskTimedOut {
                            id: id.clone(),
                            total_duration: total_runtime,
                            timestamp: now,
                        });
                    } else {
                        events_to_emit.push(HeartbeatEvent::HealthDegraded {
                            id: id.clone(),
                            health: current_health,
                            diagnosis,
                            timestamp: now,
                        });
                    }
                }
            }
        }

        // Emit events outside lock
        for event in events_to_emit {
            let _ = self.event_tx.send(event);
        }

        diagnoses
    }

    // ------------------------------------------------------------------------
    // Automated Recovery & Action Execution
    // ------------------------------------------------------------------------

    /// Executes recovery actions on stalled or hung subagents.
    pub async fn auto_recover(&self, diagnoses: &[HangDiagnosis]) -> Vec<RecoveryResult> {
        let mut results = Vec::new();
        let now = Utc::now();

        for d in diagnoses {
            match &d.recommended_action {
                RecoveryAction::None => {}
                RecoveryAction::Ping => {
                    // Send probe event
                    let _ = self.event_tx.send(HeartbeatEvent::RecoveryTriggered {
                        id: d.subagent_id.clone(),
                        action: RecoveryAction::Ping,
                        details: "Sent keepalive ping probe".to_string(),
                        timestamp: now,
                    });
                    results.push(RecoveryResult {
                        subagent_id: d.subagent_id.clone(),
                        action: RecoveryAction::Ping,
                        success: true,
                        details: "Probe sent".to_string(),
                        timestamp: now,
                    });
                }
                RecoveryAction::Warn => {
                    let _ = self.event_tx.send(HeartbeatEvent::RecoveryTriggered {
                        id: d.subagent_id.clone(),
                        action: RecoveryAction::Warn,
                        details: format!("Warning alert logged: {}", d.diagnosis_summary),
                        timestamp: now,
                    });
                    results.push(RecoveryResult {
                        subagent_id: d.subagent_id.clone(),
                        action: RecoveryAction::Warn,
                        success: true,
                        details: "Warning logged".to_string(),
                        timestamp: now,
                    });
                }
                RecoveryAction::Cancel | RecoveryAction::ForceKill => {
                    self.mark_cancelled(
                        &d.subagent_id,
                        format!(
                            "Automated hang detector cancellation: {}",
                            d.diagnosis_summary
                        ),
                    )
                    .await;

                    let _ = self.event_tx.send(HeartbeatEvent::RecoveryTriggered {
                        id: d.subagent_id.clone(),
                        action: d.recommended_action.clone(),
                        details: format!("Cancelled hung subagent: {}", d.diagnosis_summary),
                        timestamp: now,
                    });

                    results.push(RecoveryResult {
                        subagent_id: d.subagent_id.clone(),
                        action: d.recommended_action.clone(),
                        success: true,
                        details: "Subagent cancelled".to_string(),
                        timestamp: now,
                    });
                }
                RecoveryAction::InterruptCurrentPhase => {
                    let _ = self.event_tx.send(HeartbeatEvent::RecoveryTriggered {
                        id: d.subagent_id.clone(),
                        action: RecoveryAction::InterruptCurrentPhase,
                        details: format!("Interrupted phase: {}", d.current_phase),
                        timestamp: now,
                    });
                    results.push(RecoveryResult {
                        subagent_id: d.subagent_id.clone(),
                        action: RecoveryAction::InterruptCurrentPhase,
                        success: true,
                        details: "Phase interrupt dispatched".to_string(),
                        timestamp: now,
                    });
                }
                RecoveryAction::Restart => {
                    self.mark_cancelled(
                        &d.subagent_id,
                        format!("Restarting hung subagent: {}", d.diagnosis_summary),
                    )
                    .await;

                    let _ = self.event_tx.send(HeartbeatEvent::RecoveryTriggered {
                        id: d.subagent_id.clone(),
                        action: RecoveryAction::Restart,
                        details: "Restart requested".to_string(),
                        timestamp: now,
                    });

                    results.push(RecoveryResult {
                        subagent_id: d.subagent_id.clone(),
                        action: RecoveryAction::Restart,
                        success: true,
                        details: "Subagent cancelled and flagged for restart".to_string(),
                        timestamp: now,
                    });
                }
                RecoveryAction::EscalateToParent => {
                    let _ = self.event_tx.send(HeartbeatEvent::RecoveryTriggered {
                        id: d.subagent_id.clone(),
                        action: RecoveryAction::EscalateToParent,
                        details: format!("Escalated deadlock to parent: {}", d.diagnosis_summary),
                        timestamp: now,
                    });
                    results.push(RecoveryResult {
                        subagent_id: d.subagent_id.clone(),
                        action: RecoveryAction::EscalateToParent,
                        success: true,
                        details: "Escalated to parent agent".to_string(),
                        timestamp: now,
                    });
                }
                RecoveryAction::Custom(name) => {
                    results.push(RecoveryResult {
                        subagent_id: d.subagent_id.clone(),
                        action: RecoveryAction::Custom(name.clone()),
                        success: true,
                        details: format!("Custom action '{name}' recorded"),
                        timestamp: now,
                    });
                }
            }
        }

        results
    }

    // ------------------------------------------------------------------------
    // Inspection & Queries
    // ------------------------------------------------------------------------

    /// Retrieves full state for a specific subagent.
    pub async fn get_subagent(&self, subagent_id: &str) -> Option<MonitoredSubagent> {
        let map = self.subagents.read().await;
        map.get(subagent_id).cloned()
    }

    /// Retrieves all registered subagents.
    pub async fn get_all_subagents(&self) -> Vec<MonitoredSubagent> {
        let map = self.subagents.read().await;
        map.values().cloned().collect()
    }

    /// Retrieves only active, stalled, or hung subagents (excludes finished ones).
    pub async fn get_active_subagents(&self) -> Vec<MonitoredSubagent> {
        let map = self.subagents.read().await;
        map.values().filter(|s| !s.is_finished()).cloned().collect()
    }

    /// Retrieves currently stalled or hung subagents.
    pub async fn get_stalled_or_hung(&self) -> Vec<MonitoredSubagent> {
        let map = self.subagents.read().await;
        map.values()
            .filter(|s| {
                matches!(
                    s.health,
                    HealthStatus::Degraded
                        | HealthStatus::Stalled
                        | HealthStatus::Hung
                        | HealthStatus::Dead
                        | HealthStatus::TimedOut
                ) && !s.is_finished()
            })
            .cloned()
            .collect()
    }

    // ------------------------------------------------------------------------
    // Background Scanner Runner
    // ------------------------------------------------------------------------

    /// Spawns a background scanner loop that continuously monitors subagents.
    pub fn start_background_scanner(
        &self,
        interval: Duration,
        auto_recover: bool,
    ) -> BackgroundScannerHandle {
        let monitor = self.clone();
        let running = Arc::new(AtomicBool::new(true));
        let running_clone = running.clone();

        let join_handle = tokio::spawn(async move {
            let mut ticker = tokio::time::interval(interval);
            ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

            while running_clone.load(Ordering::Relaxed) {
                ticker.tick().await;
                if !running_clone.load(Ordering::Relaxed) {
                    break;
                }

                let diagnoses = monitor.scan().await;
                if auto_recover && !diagnoses.is_empty() {
                    let _ = monitor.auto_recover(&diagnoses).await;
                }
            }
        });

        BackgroundScannerHandle {
            running,
            join_handle: Some(join_handle),
        }
    }

    // ------------------------------------------------------------------------
    // Reporting & Dashboards
    // ------------------------------------------------------------------------

    /// Generates a terminal-friendly ASCII status table of all subagents.
    pub async fn format_ascii_table(&self) -> String {
        let now = Utc::now();
        let subagents = self.get_all_subagents().await;

        if subagents.is_empty() {
            return "No subagents registered in heartbeat monitor.\n".to_string();
        }

        let mut out = String::new();
        out.push_str("┌──────────────────────┬─────────────┬───────────┬──────────────────────┬──────────────┬──────────────┐\n");
        out.push_str("│ Subagent Name / ID   │ Role        │ Status    │ Current Phase        │ Last Seen    │ Runtime      │\n");
        out.push_str("├──────────────────────┼─────────────┼───────────┼──────────────────────┼──────────────┼──────────────┤\n");

        for s in &subagents {
            let name_or_id = if s.name.len() <= 20 {
                s.name.clone()
            } else {
                format!("{}…", &s.name[..19])
            };

            let role = if s.role.len() <= 11 {
                s.role.clone()
            } else {
                format!("{}…", &s.role[..10])
            };

            let status_str = match s.health {
                HealthStatus::Healthy => "Healthy".to_string(),
                HealthStatus::Degraded => "Warning".to_string(),
                HealthStatus::Stalled => "STALLED".to_string(),
                HealthStatus::Hung => "HUNG!".to_string(),
                HealthStatus::Dead => "DEAD".to_string(),
                HealthStatus::TimedOut => "TIMEOUT".to_string(),
            };

            let phase_str = match &s.current_phase {
                SubagentPhase::ExecutingTool { tool_name } => format!("Tool:{tool_name}"),
                SubagentPhase::WaitingForPeer { peer_id } => format!("Peer:{peer_id}"),
                other => other.to_string(),
            };
            let phase_display = if phase_str.len() <= 20 {
                phase_str
            } else {
                format!("{}…", &phase_str[..19])
            };

            let last_seen_dur = s.elapsed_since_heartbeat(now);
            let last_seen_str = if s.is_finished() {
                "-".to_string()
            } else {
                format!("{:.1}s ago", last_seen_dur.as_secs_f64())
            };

            let runtime_dur = s.total_runtime(now);
            let runtime_str = format!("{:.1}s", runtime_dur.as_secs_f64());

            out.push_str(&format!(
                "│ {:<20} │ {:<11} │ {:<9} │ {:<20} │ {:<12} │ {:<12} │\n",
                name_or_id, role, status_str, phase_display, last_seen_str, runtime_str
            ));
        }

        out.push_str("└──────────────────────┴─────────────┴───────────┴──────────────────────┴──────────────┴──────────────┘\n");
        out
    }

    /// Generates a diagnostic summary report with root cause analysis.
    pub async fn format_diagnostic_report(&self) -> String {
        let diagnoses = self.scan().await;
        if diagnoses.is_empty() {
            return "✓ All active subagents are healthy. No stalls or hangs detected.\n"
                .to_string();
        }

        let mut out = String::new();
        out.push_str(&format!(
            "⚠ Detected {} subagent(s) with health issues:\n\n",
            diagnoses.len()
        ));

        for (idx, d) in diagnoses.iter().enumerate() {
            out.push_str(&format!("--- [Issue #{}] ---\n", idx + 1));
            out.push_str(&format!("{d}\n"));
        }

        out
    }
}

// ============================================================================
// 10. RAII Subagent Heartbeat Handle
// ============================================================================

/// Ergonomic RAII handle for worker subagents to report periodic heartbeats and state transitions.
pub struct SubagentHeartbeatHandle {
    subagent_id: String,
    monitor: HeartbeatMonitor,
    completed: AtomicBool,
    sequence: AtomicU64,
}

impl SubagentHeartbeatHandle {
    /// Creates a new subagent heartbeat handle.
    pub fn new(subagent_id: String, monitor: HeartbeatMonitor) -> Self {
        Self {
            subagent_id,
            monitor,
            completed: AtomicBool::new(false),
            sequence: AtomicU64::new(0),
        }
    }

    /// Returns the monitored subagent ID.
    pub fn id(&self) -> &str {
        &self.subagent_id
    }

    /// Records a simple heartbeat tick.
    pub async fn tick(&self) -> bool {
        self.sequence.fetch_add(1, Ordering::Relaxed);
        self.monitor
            .record_heartbeat(&self.subagent_id, None, None, None, None, None)
            .await
    }

    /// Records a full heartbeat update with phase, progress, and metrics.
    pub async fn heartbeat(
        &self,
        phase: Option<SubagentPhase>,
        progress: Option<f32>,
        current_step: Option<String>,
        metrics: Option<HeartbeatMetrics>,
        message: Option<String>,
    ) -> bool {
        self.sequence.fetch_add(1, Ordering::Relaxed);
        self.monitor
            .record_heartbeat(
                &self.subagent_id,
                phase,
                progress,
                current_step,
                metrics,
                message,
            )
            .await
    }

    /// Updates current phase and active step.
    pub async fn set_phase(&self, phase: SubagentPhase, current_step: Option<String>) -> bool {
        self.monitor
            .update_phase(&self.subagent_id, phase, current_step)
            .await
    }

    /// Updates progress percentage and active step.
    pub async fn set_progress(&self, progress: f32, current_step: Option<String>) -> bool {
        self.monitor
            .update_progress(&self.subagent_id, progress, current_step)
            .await
    }

    /// Explicitly marks the subagent as completed with an optional result artifact.
    pub async fn complete(&self, result_summary: Option<String>) -> bool {
        self.completed.store(true, Ordering::SeqCst);
        self.monitor
            .mark_completed(&self.subagent_id, result_summary)
            .await
    }

    /// Explicitly marks the subagent as failed with an error message.
    pub async fn fail(&self, error: String) -> bool {
        self.completed.store(true, Ordering::SeqCst);
        self.monitor.mark_failed(&self.subagent_id, error).await
    }

    /// Spawns an automatic background heartbeat ticker that periodically sends keepalives.
    pub fn start_auto_ticker(&self, interval: Duration) -> AutoTickerHandle {
        let monitor = self.monitor.clone();
        let subagent_id = self.subagent_id.clone();
        let running = Arc::new(AtomicBool::new(true));
        let running_clone = running.clone();

        let join_handle = tokio::spawn(async move {
            let mut ticker = tokio::time::interval(interval);
            ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

            while running_clone.load(Ordering::Relaxed) {
                ticker.tick().await;
                if !running_clone.load(Ordering::Relaxed) {
                    break;
                }
                let ok = monitor
                    .record_heartbeat(&subagent_id, None, None, None, None, None)
                    .await;
                if !ok {
                    break;
                }
            }
        });

        AutoTickerHandle {
            running,
            join_handle: Some(join_handle),
        }
    }
}

// ============================================================================
// 11. Scanner & Ticker Handles
// ============================================================================

/// Handle to a running background health scanner.
pub struct BackgroundScannerHandle {
    running: Arc<AtomicBool>,
    join_handle: Option<JoinHandle<()>>,
}

impl BackgroundScannerHandle {
    /// Stops the background scanner loop.
    pub fn stop(&mut self) {
        self.running.store(false, Ordering::Relaxed);
        if let Some(handle) = self.join_handle.take() {
            handle.abort();
        }
    }
}

impl Drop for BackgroundScannerHandle {
    fn drop(&mut self) {
        self.stop();
    }
}

/// Handle to an automatic heartbeat ticker.
pub struct AutoTickerHandle {
    running: Arc<AtomicBool>,
    join_handle: Option<JoinHandle<()>>,
}

impl AutoTickerHandle {
    /// Stops the automatic heartbeat ticker.
    pub fn stop(&mut self) {
        self.running.store(false, Ordering::Relaxed);
        if let Some(handle) = self.join_handle.take() {
            handle.abort();
        }
    }
}

impl Drop for AutoTickerHandle {
    fn drop(&mut self) {
        self.stop();
    }
}

// ============================================================================
// 12. SubagentHeartbeatTool (LLM Tool Integration)
// ============================================================================

/// Tool enabling agents to monitor subagent heartbeats and detect/resolve hangs.
pub struct SubagentHeartbeatTool {
    monitor: HeartbeatMonitor,
}

impl SubagentHeartbeatTool {
    /// Creates a new SubagentHeartbeatTool backed by a HeartbeatMonitor.
    pub fn new(monitor: HeartbeatMonitor) -> Self {
        Self { monitor }
    }
}

#[async_trait]
impl Tool for SubagentHeartbeatTool {
    fn name(&self) -> &str {
        "subagent_heartbeat_monitor"
    }

    fn description(&self) -> &str {
        "Monitors subagent health and detects hung or stalled background tasks. \
         Allows listing subagents, diagnosing root causes of hangs, querying status tables, \
         and triggering recovery or cancellation of stuck workers."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "action": {
                    "type": "string",
                    "enum": ["status", "diagnose", "list", "inspect", "recover", "cancel"],
                    "description": "The heartbeat monitor action to perform: 'status' for formatted dashboard, 'diagnose' to scan for hung/stalled subagents, 'list' for structured JSON list, 'inspect' for deep subagent details, 'recover' to auto-recover hung subagents, 'cancel' to cancel a stuck subagent."
                },
                "subagent_id": {
                    "type": "string",
                    "description": "Specific subagent ID required for 'inspect' and 'cancel' actions."
                },
                "reason": {
                    "type": "string",
                    "description": "Optional cancellation reason when cancelling a subagent."
                }
            },
            "required": ["action"]
        })
    }

    async fn execute(&self, args: Value, _ctx: &ToolContext) -> anyhow::Result<String> {
        let action = args
            .get("action")
            .and_then(|v| v.as_str())
            .unwrap_or("status");

        match action {
            "status" => {
                let table = self.monitor.format_ascii_table().await;
                let report = self.monitor.format_diagnostic_report().await;
                Ok(format!("{table}\n{report}"))
            }
            "diagnose" => {
                let report = self.monitor.format_diagnostic_report().await;
                let diagnoses = self.monitor.scan().await;
                let json_data = serde_json::to_string_pretty(&diagnoses)?;
                Ok(format!("{report}\nStructured Diagnoses:\n{json_data}"))
            }
            "list" => {
                let subagents = self.monitor.get_all_subagents().await;
                Ok(serde_json::to_string_pretty(&subagents)?)
            }
            "inspect" => {
                let subagent_id = args
                    .get("subagent_id")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| {
                        anyhow::anyhow!("'subagent_id' is required for 'inspect' action")
                    })?;

                match self.monitor.get_subagent(subagent_id).await {
                    Some(s) => Ok(serde_json::to_string_pretty(&s)?),
                    None => Ok(format!(
                        "Subagent '{subagent_id}' not found in heartbeat monitor."
                    )),
                }
            }
            "recover" => {
                let diagnoses = self.monitor.scan().await;
                if diagnoses.is_empty() {
                    return Ok(
                        "No stalled or hung subagents detected. No recovery needed.".to_string()
                    );
                }
                let results = self.monitor.auto_recover(&diagnoses).await;
                Ok(format!(
                    "Recovery initiated for {} subagent(s):\n{}",
                    results.len(),
                    serde_json::to_string_pretty(&results)?
                ))
            }
            "cancel" => {
                let subagent_id = args
                    .get("subagent_id")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| {
                        anyhow::anyhow!("'subagent_id' is required for 'cancel' action")
                    })?;
                let reason = args
                    .get("reason")
                    .and_then(|v| v.as_str())
                    .unwrap_or("Cancelled via heartbeat monitor tool");

                let ok = self
                    .monitor
                    .mark_cancelled(subagent_id, reason.to_string())
                    .await;
                if ok {
                    Ok(format!("Subagent '{subagent_id}' successfully cancelled."))
                } else {
                    Ok(format!(
                        "Subagent '{subagent_id}' not found or already finished."
                    ))
                }
            }
            other => Err(anyhow::anyhow!("Unknown action '{other}'")),
        }
    }
}

// ============================================================================
// 13. Unit & Integration Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::time::sleep;

    #[tokio::test]
    async fn test_subagent_registration_and_heartbeat() {
        let monitor = HeartbeatMonitor::default();
        let handle = monitor
            .register(
                "sub-1",
                "Scout1",
                "scout",
                None,
                Some("Explore codebase".to_string()),
                None,
            )
            .await;

        let subagent = monitor.get_subagent("sub-1").await.unwrap();
        assert_eq!(subagent.id, "sub-1");
        assert_eq!(subagent.name, "Scout1");
        assert_eq!(subagent.status, HeartbeatStatus::Active);
        assert_eq!(subagent.health, HealthStatus::Healthy);

        // Send a heartbeat tick
        assert!(handle.tick().await);
        let subagent_after = monitor.get_subagent("sub-1").await.unwrap();
        assert_eq!(subagent_after.heartbeat_count, 1);

        // Update phase and progress
        assert!(
            handle
                .heartbeat(
                    Some(SubagentPhase::Thinking),
                    Some(0.25),
                    Some("Analyzing architecture".to_string()),
                    None,
                    None,
                )
                .await
        );

        let subagent_phase = monitor.get_subagent("sub-1").await.unwrap();
        assert_eq!(subagent_phase.current_phase, SubagentPhase::Thinking);
        assert_eq!(subagent_phase.current_progress, Some(0.25));
        assert_eq!(
            subagent_phase.current_step.as_deref(),
            Some("Analyzing architecture")
        );
        assert_eq!(subagent_phase.phase_history.len(), 1);
    }

    #[tokio::test]
    async fn test_stall_and_hang_detection_thresholds() {
        let thresholds = HeartbeatThresholds {
            heartbeat_interval: Duration::from_millis(10),
            warning_threshold: Duration::from_millis(30),
            stall_threshold: Duration::from_millis(60),
            hang_threshold: Duration::from_millis(100),
            dead_threshold: Duration::from_millis(200),
            max_phase_duration: Duration::from_secs(60),
            max_stuck_progress_duration: Duration::from_secs(60),
            max_task_duration: None,
            phase_timeouts: HashMap::new(),
        };

        let monitor = HeartbeatMonitor::new(thresholds);
        let _handle = monitor
            .register("sub-hang", "CoderHang", "coder", None, None, None)
            .await;

        // Initially healthy
        let initial_diagnoses = monitor.scan().await;
        assert!(initial_diagnoses.is_empty());

        // Wait beyond stall threshold
        sleep(Duration::from_millis(70)).await;
        let stall_diagnoses = monitor.scan().await;
        assert_eq!(stall_diagnoses.len(), 1);
        assert_eq!(stall_diagnoses[0].health, HealthStatus::Stalled);
        assert_eq!(stall_diagnoses[0].recommended_action, RecoveryAction::Ping);

        // Wait beyond hang threshold
        sleep(Duration::from_millis(50)).await;
        let hang_diagnoses = monitor.scan().await;
        assert_eq!(hang_diagnoses.len(), 1);
        assert_eq!(hang_diagnoses[0].health, HealthStatus::Hung);
        assert_eq!(hang_diagnoses[0].recommended_action, RecoveryAction::Cancel);
    }

    #[tokio::test]
    async fn test_phase_timeout_detection() {
        let mut phase_timeouts = HashMap::new();
        phase_timeouts.insert("executing_tool".to_string(), Duration::from_millis(50));

        let thresholds = HeartbeatThresholds {
            heartbeat_interval: Duration::from_millis(10),
            warning_threshold: Duration::from_secs(10),
            stall_threshold: Duration::from_secs(10),
            hang_threshold: Duration::from_secs(10),
            dead_threshold: Duration::from_secs(10),
            max_phase_duration: Duration::from_millis(50),
            max_stuck_progress_duration: Duration::from_secs(10),
            max_task_duration: None,
            phase_timeouts,
        };

        let monitor = HeartbeatMonitor::new(thresholds);
        let handle = monitor
            .register("sub-tool", "TesterTool", "tester", None, None, None)
            .await;

        handle
            .set_phase(
                SubagentPhase::ExecutingTool {
                    tool_name: "bash".to_string(),
                },
                Some("Running cargo test".to_string()),
            )
            .await;

        // Keep heartbeats ticking so it doesn't fail on missed heartbeats
        sleep(Duration::from_millis(60)).await;
        handle.tick().await;

        let diagnoses = monitor.scan().await;
        assert_eq!(diagnoses.len(), 1);
        assert_eq!(diagnoses[0].reason, HangReason::StuckToolExecution);
    }

    #[tokio::test]
    async fn test_auto_recover_execution() {
        let thresholds = HeartbeatThresholds {
            heartbeat_interval: Duration::from_millis(10),
            warning_threshold: Duration::from_millis(20),
            stall_threshold: Duration::from_millis(30),
            hang_threshold: Duration::from_millis(50),
            dead_threshold: Duration::from_millis(100),
            max_phase_duration: Duration::from_secs(60),
            max_stuck_progress_duration: Duration::from_secs(60),
            max_task_duration: None,
            phase_timeouts: HashMap::new(),
        };

        let monitor = HeartbeatMonitor::new(thresholds);
        let _handle = monitor
            .register("sub-recover", "Worker", "general", None, None, None)
            .await;

        sleep(Duration::from_millis(60)).await;
        let diagnoses = monitor.scan().await;
        assert!(!diagnoses.is_empty());

        let results = monitor.auto_recover(&diagnoses).await;
        assert_eq!(results.len(), 1);
        assert!(results[0].success);

        let subagent = monitor.get_subagent("sub-recover").await.unwrap();
        assert_eq!(subagent.status, HeartbeatStatus::Cancelled);
    }

    #[tokio::test]
    async fn test_completion_and_failure_lifecycle() {
        let monitor = HeartbeatMonitor::default();
        let handle1 = monitor
            .register("sub-c1", "Done1", "scout", None, None, None)
            .await;
        let handle2 = monitor
            .register("sub-f1", "Fail1", "coder", None, None, None)
            .await;

        assert!(handle1.complete(Some("All files found".to_string())).await);
        assert!(
            handle2
                .fail("Compilation error in main.rs".to_string())
                .await
        );

        let s1 = monitor.get_subagent("sub-c1").await.unwrap();
        assert_eq!(s1.status, HeartbeatStatus::Completed);
        assert_eq!(s1.current_progress, Some(1.0));
        assert_eq!(s1.exit_result.as_deref(), Some("All files found"));

        let s2 = monitor.get_subagent("sub-f1").await.unwrap();
        assert_eq!(s2.status, HeartbeatStatus::Failed);
        assert_eq!(
            s2.exit_result.as_deref(),
            Some("Compilation error in main.rs")
        );

        // Finished subagents should not appear in scan diagnoses even after time passes
        let diagnoses = monitor.scan().await;
        assert!(diagnoses.is_empty());
    }

    #[tokio::test]
    async fn test_ascii_table_and_tool_execution() {
        let monitor = HeartbeatMonitor::default();
        let _handle = monitor
            .register(
                "sub-table",
                "Reviewer1",
                "reviewer",
                None,
                Some("Audit code quality".to_string()),
                None,
            )
            .await;

        let table = monitor.format_ascii_table().await;
        assert!(table.contains("Reviewer1"));
        assert!(table.contains("reviewer"));

        let tool = SubagentHeartbeatTool::new(monitor.clone());
        let ctx = ToolContext::default();

        let res = tool
            .execute(json!({ "action": "status" }), &ctx)
            .await
            .unwrap();
        assert!(res.contains("Reviewer1"));

        let inspect_res = tool
            .execute(
                json!({ "action": "inspect", "subagent_id": "sub-table" }),
                &ctx,
            )
            .await
            .unwrap();
        assert!(inspect_res.contains("sub-table"));
    }
}
