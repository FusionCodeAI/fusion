use std::collections::HashMap;
use std::fmt;
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::sync::Arc;

use fusion_iso::{ChangeKind, Diff, IsolationBackend};
use async_trait::async_trait;
use chrono::Utc;
use futures::future::join_all;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tokio::sync::{broadcast, mpsc, watch, RwLock, Semaphore};

use crate::config::Config;
use crate::provider::LlmClient;
use crate::tools::types::{Tool, ToolContext, ToolRegistry};

/// Specialized roles for worker subagents.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SubagentRole {
    /// Fast, read-only exploration and analysis agent.
    Scout,
    /// Implementation, refactoring, and code modification specialist.
    Coder,
    /// Testing, verification, and failure diagnosis specialist.
    Tester,
    /// Code quality, security, and architectural review specialist.
    Reviewer,
    /// General-purpose worker subagent.
    General,
    /// Custom user-defined role with a tailored prompt.
    Custom { name: String, prompt: String },
}

impl fmt::Display for SubagentRole {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SubagentRole::Scout => write!(f, "Scout"),
            SubagentRole::Coder => write!(f, "Coder"),
            SubagentRole::Tester => write!(f, "Tester"),
            SubagentRole::Reviewer => write!(f, "Reviewer"),
            SubagentRole::General => write!(f, "General"),
            SubagentRole::Custom { name, .. } => write!(f, "{}", name),
        }
    }
}

impl FromStr for SubagentRole {
    type Err = std::convert::Infallible;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(match s.trim().to_lowercase().as_str() {
            "scout" => SubagentRole::Scout,
            "coder" => SubagentRole::Coder,
            "tester" => SubagentRole::Tester,
            "reviewer" => SubagentRole::Reviewer,
            _ => SubagentRole::General,
        })
    }
}

impl SubagentRole {
    /// Returns the default display name for this role.
    pub fn default_name(&self) -> &str {
        match self {
            SubagentRole::Scout => "Scout",
            SubagentRole::Coder => "Coder",
            SubagentRole::Tester => "Tester",
            SubagentRole::Reviewer => "Reviewer",
            SubagentRole::General => "Worker",
            SubagentRole::Custom { name, .. } => name.as_str(),
        }
    }

    /// Generates a role-tailored system prompt for the subagent.
    pub fn system_prompt(&self, name: &str) -> String {
        match self {
            SubagentRole::Scout => format!(
                "You are a fast, read-only exploration and analysis agent named '{name}'.\n\
                 Your mission is to search, inspect, and analyze the codebase to answer questions, locate relevant files, and collect architectural context.\n\
                 Guidelines:\n\
                 - Search efficiently using grep, glob, and read tools.\n\
                 - Read only relevant lines and avoid massive unneeded file dumps.\n\
                 - Do NOT attempt to modify files or run destructive commands.\n\
                 - Synthesize clear, concise findings and return a definitive summary."
            ),
            SubagentRole::Coder => format!(
                "You are an expert coding and implementation agent named '{name}'.\n\
                 Your mission is to implement features, fix bugs, and refactor code cleanly and robustly.\n\
                 Guidelines:\n\
                 - Follow existing project conventions, idioms, and architecture.\n\
                 - Make targeted edits with precision using edit and write tools.\n\
                 - Keep changes minimal, robust, cross-platform, and safe.\n\
                 - Verify your changes before returning."
            ),
            SubagentRole::Tester => format!(
                "You are a testing and quality assurance agent named '{name}'.\n\
                 Your mission is to execute tests, verify behavior, diagnose failures, and ensure reliability.\n\
                 Guidelines:\n\
                 - Run targeted tests using the bash tool.\n\
                 - Inspect test output and failures carefully.\n\
                 - Accurately report failure modes, root causes, and suggested fixes."
            ),
            SubagentRole::Reviewer => format!(
                "You are a thorough code review and security agent named '{name}'.\n\
                 Your mission is to review diffs and source files for bugs, security vulnerabilities, edge cases, and architectural integrity.\n\
                 Guidelines:\n\
                 - Inspect code carefully for logic errors, memory/resource leaks, and security risks.\n\
                 - Verify cross-platform compatibility and edge cases.\n\
                 - Provide constructive, actionable critique."
            ),
            SubagentRole::General => format!(
                "You are a specialized worker subagent named '{name}'.\n\
                 Focus solely on the assigned task and return concise, actionable results."
            ),
            SubagentRole::Custom { prompt, .. } => prompt.clone(),
        }
    }

    /// Allowed tool names specifically dedicated to this role.
    pub fn allowed_tool_names(&self) -> Option<Vec<&'static str>> {
        match self {
            SubagentRole::Scout | SubagentRole::Reviewer => {
                Some(vec!["read", "read_file", "grep", "glob"])
            }
            SubagentRole::Coder => Some(vec![
                "read",
                "read_file",
                "write",
                "write_file",
                "edit",
                "edit_file",
                "grep",
                "glob",
            ]),
            SubagentRole::Tester => Some(vec!["bash", "read", "read_file", "grep", "glob"]),
            SubagentRole::General | SubagentRole::Custom { .. } => None,
        }
    }

    /// Filters a base `ToolRegistry` to only include tools dedicated to this role.
    pub fn filter_tools(&self, base: &ToolRegistry) -> ToolRegistry {
        if let Some(allowed) = self.allowed_tool_names() {
            let mut filtered = ToolRegistry::new();
            for name in allowed {
                if let Some(tool) = base.get(name) {
                    filtered.register(tool);
                }
            }
            filtered
        } else {
            base.clone()
        }
    }
}

/// Progress and communication events emitted throughout a subagent's execution lifecycle.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum SubagentProgress {
    /// Subagent has started.
    Started {
        id: String,
        name: String,
        role: SubagentRole,
        task: String,
    },
    /// A new turn has begun.
    TurnStarted {
        id: String,
        turn: usize,
        max_turns: usize,
    },
    /// Model reasoning/thinking chunk.
    Thinking { id: String, delta: String },
    /// Text message emitted by the subagent.
    Message { id: String, content: String },
    /// Subagent started executing a tool.
    ToolStarted {
        id: String,
        tool: String,
        args: Value,
    },
    /// Subagent completed executing a tool.
    ToolCompleted {
        id: String,
        tool: String,
        output: String,
        success: bool,
    },
    /// Subagent completed its task successfully.
    Completed {
        id: String,
        output: String,
        turns_taken: usize,
    },
    /// Subagent encountered an unrecoverable failure.
    Failed { id: String, error: String },
    /// Subagent was cancelled.
    Cancelled { id: String },
}

impl SubagentProgress {
    /// Returns the unique ID of the subagent associated with this event.
    pub fn id(&self) -> &str {
        match self {
            SubagentProgress::Started { id, .. }
            | SubagentProgress::TurnStarted { id, .. }
            | SubagentProgress::Thinking { id, .. }
            | SubagentProgress::Message { id, .. }
            | SubagentProgress::ToolStarted { id, .. }
            | SubagentProgress::ToolCompleted { id, .. }
            | SubagentProgress::Completed { id, .. }
            | SubagentProgress::Failed { id, .. }
            | SubagentProgress::Cancelled { id, .. } => id,
        }
    }
}

/// Execution status of a subagent.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SubagentStatus {
    Pending,
    Running {
        turn: usize,
        current_tool: Option<String>,
    },
    Completed {
        output: String,
        turns: usize,
    },
    Failed {
        error: String,
    },
    Cancelled,
}

/// Snapshot summary of a subagent's state.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubagentInfo {
    pub id: String,
    pub name: String,
    pub role: SubagentRole,
    pub task: String,
    pub status: SubagentStatus,
    pub started_at: String,
    pub completed_at: Option<String>,
    pub turns: usize,
}

/// Result returned upon subagent completion.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubagentResult {
    pub id: String,
    pub name: String,
    pub role: SubagentRole,
    pub task: String,
    pub output: String,
    pub turns: usize,
    pub success: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub patch: Option<String>,
}

/// Task specification for spawning a subagent.
#[derive(Clone)]
pub struct SubagentTask {
    pub id: String,
    pub name: String,
    pub role: SubagentRole,
    pub task: String,
    pub system_prompt: Option<String>,
    pub tools: Option<ToolRegistry>,
    pub max_turns: usize,
    pub model: Option<String>,
    pub temperature: Option<f32>,
    pub isolated: bool,
}

impl fmt::Debug for SubagentTask {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SubagentTask")
            .field("id", &self.id)
            .field("name", &self.name)
            .field("role", &self.role)
            .field("task", &self.task)
            .field("system_prompt", &self.system_prompt)
            .field("has_custom_tools", &self.tools.is_some())
            .field("max_turns", &self.max_turns)
            .field("model", &self.model)
            .field("temperature", &self.temperature)
            .field("isolated", &self.isolated)
            .finish()
    }
}

impl SubagentTask {
    pub fn new(role: SubagentRole, task: impl Into<String>) -> Self {
        let task_str = task.into();
        let role_name = role.default_name().to_string();
        let isolated = matches!(role, SubagentRole::Coder);
        Self {
            id: uuid::Uuid::new_v4().to_string()[..8].to_string(),
            name: role_name,
            role,
            task: task_str,
            system_prompt: None,
            tools: None,
            max_turns: 20,
            model: None,
            temperature: None,
            isolated,
        }
    }

    /// Creates a scout exploration task.
    pub fn scout(task: impl Into<String>) -> Self {
        Self::new(SubagentRole::Scout, task)
    }

    /// Creates a coder implementation task.
    pub fn coder(task: impl Into<String>) -> Self {
        Self::new(SubagentRole::Coder, task)
    }

    /// Creates a tester verification task.
    pub fn tester(task: impl Into<String>) -> Self {
        Self::new(SubagentRole::Tester, task)
    }

    /// Creates a reviewer critique task.
    pub fn reviewer(task: impl Into<String>) -> Self {
        Self::new(SubagentRole::Reviewer, task)
    }

    /// Creates a general worker task.
    pub fn general(task: impl Into<String>) -> Self {
        Self::new(SubagentRole::General, task)
    }

    /// Creates a custom role task.
    pub fn custom(
        name: impl Into<String>,
        prompt: impl Into<String>,
        task: impl Into<String>,
    ) -> Self {
        let name_str = name.into();
        let prompt_str = prompt.into();
        let role = SubagentRole::Custom {
            name: name_str.clone(),
            prompt: prompt_str,
        };
        let mut t = Self::new(role, task);
        t.name = name_str;
        t
    }

    pub fn with_id(mut self, id: impl Into<String>) -> Self {
        self.id = id.into();
        self
    }

    pub fn with_name(mut self, name: impl Into<String>) -> Self {
        self.name = name.into();
        self
    }

    pub fn with_system_prompt(mut self, prompt: impl Into<String>) -> Self {
        self.system_prompt = Some(prompt.into());
        self
    }

    pub fn with_tools(mut self, tools: ToolRegistry) -> Self {
        self.tools = Some(tools);
        self
    }

    pub fn with_max_turns(mut self, max_turns: usize) -> Self {
        self.max_turns = max_turns;
        self
    }

    pub fn with_model(mut self, model: impl Into<String>) -> Self {
        self.model = Some(model.into());
        self
    }

    pub fn with_temperature(mut self, temperature: f32) -> Self {
        self.temperature = Some(temperature);
        self
    }

    pub fn with_isolated(mut self, isolated: bool) -> Self {
        self.isolated = isolated;
        self
    }
}

/// Handle to a running background subagent task.
pub struct SubagentHandle {
    pub id: String,
    pub name: String,
    pub role: SubagentRole,
    pub(crate) task_handle: tokio::task::JoinHandle<anyhow::Result<SubagentResult>>,
    pub(crate) cancel_tx: watch::Sender<bool>,
    pub(crate) progress_rx: Option<mpsc::UnboundedReceiver<SubagentProgress>>,
}

impl SubagentHandle {
    pub fn id(&self) -> &str {
        &self.id
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn role(&self) -> &SubagentRole {
        &self.role
    }

    /// Signals the running subagent to cancel.
    pub fn cancel(&self) {
        let _ = self.cancel_tx.send(true);
    }

    /// Asynchronously receives the next progress event from this subagent.
    pub async fn recv_progress(&mut self) -> Option<SubagentProgress> {
        if let Some(rx) = &mut self.progress_rx {
            rx.recv().await
        } else {
            None
        }
    }

    /// Takes the progress receiver channel out of this handle.
    pub fn take_progress_receiver(&mut self) -> Option<mpsc::UnboundedReceiver<SubagentProgress>> {
        self.progress_rx.take()
    }

    /// Waits for the subagent task to complete and returns its result.
    pub async fn wait(self) -> anyhow::Result<SubagentResult> {
        match self.task_handle.await {
            Ok(res) => res,
            Err(e) => anyhow::bail!("Subagent task join error: {}", e),
        }
    }
}

/// Default maximum concurrent subagents allowed to run concurrently.
pub const DEFAULT_MAX_CONCURRENT_SUBAGENTS: usize = 16;

/// Central orchestrator managing concurrency, spawning, communication channels,
/// and lifecycle tracking for specialized subagents.
#[derive(Clone)]
pub struct SubagentManager {
    client: Arc<LlmClient>,
    config: Config,
    tools: ToolRegistry,
    max_concurrent: usize,
    semaphore: Arc<Semaphore>,
    active_agents: Arc<RwLock<HashMap<String, SubagentInfo>>>,
    cancels: Arc<RwLock<HashMap<String, watch::Sender<bool>>>>,
    global_event_tx: broadcast::Sender<SubagentProgress>,
    metrics: Arc<crate::agent::metrics::SubagentMetricsCollector>,
    event_tx: Option<tokio::sync::mpsc::UnboundedSender<crate::agent::loop_runner::AgentEvent>>,
}

impl SubagentManager {
    /// Creates a new `SubagentManager`.
    pub fn new(client: Arc<LlmClient>, config: Config, tools: ToolRegistry) -> Self {
        let (global_event_tx, _) = broadcast::channel(256);
        let max_concurrent = config
            .max_concurrent_subagents
            .unwrap_or(DEFAULT_MAX_CONCURRENT_SUBAGENTS);
        let mgr = Self {
            client,
            config,
            tools,
            max_concurrent,
            semaphore: Arc::new(Semaphore::new(max_concurrent)),
            active_agents: Arc::new(RwLock::new(HashMap::new())),
            cancels: Arc::new(RwLock::new(HashMap::new())),
            global_event_tx: global_event_tx.clone(),
            metrics: Arc::new(crate::agent::metrics::SubagentMetricsCollector::new()),
            event_tx: None,
        };
        if tokio::runtime::Handle::try_current().is_ok() {
            crate::agent::metrics::SubagentMetricsCollector::spawn_event_listener(
                mgr.metrics.clone(),
                global_event_tx.subscribe(),
            );
        }
        mgr
    }

    /// Configures the maximum number of subagents allowed to run concurrently.
    pub fn with_max_concurrent(mut self, max: usize) -> Self {
        self.max_concurrent = max;
        self.semaphore = Arc::new(Semaphore::new(max));
        self
    }

    /// Returns the maximum number of subagents allowed to run concurrently.
    pub fn max_concurrent(&self) -> usize {
        self.max_concurrent
    }

    /// Subscribes to the broadcast channel of all subagent progress events.
    pub fn subscribe(&self) -> broadcast::Receiver<SubagentProgress> {
        self.global_event_tx.subscribe()
    }

    /// Sets the channel sender to forward live progress events as `AgentEvent::SubagentProgressEvent`.
    pub fn with_event_sender(
        mut self,
        tx: tokio::sync::mpsc::UnboundedSender<crate::agent::loop_runner::AgentEvent>,
    ) -> Self {
        self.event_tx = Some(tx);
        self
    }

    /// Sets or clears the optional channel sender for progress events.
    pub fn with_opt_event_sender(
        mut self,
        tx: Option<tokio::sync::mpsc::UnboundedSender<crate::agent::loop_runner::AgentEvent>>,
    ) -> Self {
        self.event_tx = tx;
        self
    }

    /// Mutates the event sender on this manager instance.
    pub fn set_event_sender(
        &mut self,
        tx: Option<tokio::sync::mpsc::UnboundedSender<crate::agent::loop_runner::AgentEvent>>,
    ) {
        self.event_tx = tx;
    }

    /// Bridges progress events to the given channel.
    pub fn bridge_progress_to(
        &mut self,
        tx: Option<tokio::sync::mpsc::UnboundedSender<crate::agent::loop_runner::AgentEvent>>,
    ) {
        self.event_tx = tx;
    }

    /// Returns a reference to the active event sender, if any.
    pub fn event_sender(
        &self,
    ) -> Option<&tokio::sync::mpsc::UnboundedSender<crate::agent::loop_runner::AgentEvent>> {
        self.event_tx.as_ref()
    }

    /// Spawns an isolated subagent task and bridges its progress events to the provided channel.
    pub fn spawn_with_channel(
        &self,
        task: SubagentTask,
        event_tx: tokio::sync::mpsc::UnboundedSender<crate::agent::loop_runner::AgentEvent>,
    ) -> SubagentHandle {
        let mut mgr = self.clone();
        mgr.event_tx = Some(event_tx);
        mgr.spawn(task)
    }

    /// Spawns an isolated subagent task in the background.
    pub fn spawn(&self, task: SubagentTask) -> SubagentHandle {
        let id = task.id.clone();
        let name = task.name.clone();
        let role = task.role.clone();
        let task_text = task.task.clone();
        let system_prompt = task
            .system_prompt
            .clone()
            .unwrap_or_else(|| role.system_prompt(&name));
        let dedicated_tools = task
            .tools
            .clone()
            .unwrap_or_else(|| role.filter_tools(&self.tools));
        let max_turns = task.max_turns;

        let (progress_tx, progress_rx) = mpsc::unbounded_channel();
        let (cancel_tx, cancel_rx) = watch::channel(false);

        let initial_info = SubagentInfo {
            id: id.clone(),
            name: name.clone(),
            role: role.clone(),
            task: task_text.clone(),
            status: SubagentStatus::Pending,
            started_at: Utc::now().to_rfc3339(),
            completed_at: None,
            turns: 0,
        };

        // Record active subagent and cancel handle
        {
            let active = self.active_agents.clone();
            let cancels = self.cancels.clone();
            let id_clone = id.clone();
            let cancel_tx_clone = cancel_tx.clone();
            tokio::spawn(async move {
                active.write().await.insert(id_clone.clone(), initial_info);
                cancels.write().await.insert(id_clone, cancel_tx_clone);
            });
        }

        let client = self.client.clone();
        let config = self.config.clone();
        let semaphore = self.semaphore.clone();
        let active_agents = self.active_agents.clone();
        let cancels = self.cancels.clone();
        let global_tx = self.global_event_tx.clone();
        let loop_event_tx = self.event_tx.clone();
        let task_id = id.clone();
        let task_name = name.clone();
        let task_role = role.clone();
        let custom_model = task.model.clone();
        let custom_temperature = task.temperature;
        let isolated = task.isolated;
        let join_handle = tokio::spawn(async move {
            let _permit = semaphore.acquire().await.ok();

            // Emit Started event
            let start_event = SubagentProgress::Started {
                id: task_id.clone(),
                name: task_name.clone(),
                role: task_role.clone(),
                task: task_text.clone(),
            };
            let _ = progress_tx.send(start_event.clone());
            let _ = global_tx.send(start_event.clone());
            if let Some(tx) = &loop_event_tx {
                let _ = tx.send(crate::agent::loop_runner::AgentEvent::SubagentProgressEvent {
                    id: task_id.clone(),
                    name: task_name.clone(),
                    role: task_role.clone(),
                    progress: start_event,
                });
            }
            // Update status to Running
            {
                let mut guard = active_agents.write().await;
                if let Some(info) = guard.get_mut(&task_id) {
                    info.status = SubagentStatus::Running {
                        turn: 1,
                        current_tool: None,
                    };
                }
            }

            let result = execute_subagent_loop(
                &task_id,
                &task_name,
                &task_role,
                &system_prompt,
                &task_text,
                &dedicated_tools,
                max_turns,
                custom_model.as_deref(),
                custom_temperature,
                isolated,
                &client,
                &config,
                &progress_tx,
                &global_tx,
                loop_event_tx.as_ref(),
                cancel_rx,
                active_agents.clone(),
            )
            .await;

            // Cleanup cancels map
            cancels.write().await.remove(&task_id);

            result
        });

        SubagentHandle {
            id,
            name,
            role,
            task_handle: join_handle,
            cancel_tx,
            progress_rx: Some(progress_rx),
        }
    }

    /// Spawns a subagent with standard parameters.
    pub fn spawn_role(&self, role: SubagentRole, name: &str, task: &str) -> SubagentHandle {
        let mut t = SubagentTask::new(role, task);
        if !name.is_empty() {
            t.name = name.to_string();
        }
        self.spawn(t)
    }

    /// Spawns a batch of subagent tasks concurrently.
    pub fn spawn_batch(&self, tasks: Vec<SubagentTask>) -> Vec<SubagentHandle> {
        tasks.into_iter().map(|t| self.spawn(t)).collect()
    }

    /// Executes multiple subagent tasks concurrently and awaits all results.
    pub async fn run_concurrent(
        &self,
        tasks: Vec<SubagentTask>,
    ) -> Vec<anyhow::Result<SubagentResult>> {
        let handles = self.spawn_batch(tasks);
        let mut futures = Vec::new();
        for handle in handles {
            futures.push(handle.wait());
        }
        join_all(futures).await
    }

    /// Retrieves the status of a specific subagent.
    pub async fn get_status(&self, id: &str) -> Option<SubagentStatus> {
        let guard = self.active_agents.read().await;
        guard.get(id).map(|info| info.status.clone())
    }

    /// Retrieves full info for a specific subagent.
    pub async fn get_info(&self, id: &str) -> Option<SubagentInfo> {
        let guard = self.active_agents.read().await;
        guard.get(id).cloned()
    }

    /// Lists all registered subagent tasks.
    pub async fn list_subagents(&self) -> Vec<SubagentInfo> {
        let guard = self.active_agents.read().await;
        guard.values().cloned().collect()
    }

    /// Cancels a running subagent by ID. Returns `true` if subagent was found.
    pub async fn cancel(&self, id: &str) -> bool {
        let guard = self.cancels.read().await;
        if let Some(tx) = guard.get(id) {
            let _ = tx.send(true);
            true
        } else {
            false
        }
    }

    /// Cancels all currently running subagents.
    pub async fn cancel_all(&self) {
        let guard = self.cancels.read().await;
        for tx in guard.values() {
            let _ = tx.send(true);
        }
    }

    /// Returns the metrics collector tracking all managed subagents.
    pub fn metrics(&self) -> Arc<crate::agent::metrics::SubagentMetricsCollector> {
        self.metrics.clone()
    }

    /// Returns detailed metrics for a specific subagent by ID.
    pub fn get_metrics(&self, id: &str) -> Option<crate::agent::metrics::SubagentMetrics> {
        self.metrics.get(id)
    }

    /// Returns fleet-wide telemetry aggregation across all managed subagents.
    pub fn fleet_metrics(&self) -> crate::agent::metrics::SubagentFleetMetrics {
        self.metrics.fleet_summary()
    }
}

/// Manages an isolated workspace lifecycle powered by `fusion_iso`.
pub struct WorkspaceIsolation {
    pub lower: PathBuf,
    pub merged: PathBuf,
    backend: &'static dyn IsolationBackend,
    cleaned_up: bool,
}

impl fmt::Debug for WorkspaceIsolation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("WorkspaceIsolation")
            .field("lower", &self.lower)
            .field("merged", &self.merged)
            .field("backend", &self.backend.kind())
            .field("cleaned_up", &self.cleaned_up)
            .finish()
    }
}

impl WorkspaceIsolation {
    /// Creates an isolated workspace mirroring `lower` at a temporary location.
    pub fn create(lower: &Path, id: &str) -> anyhow::Result<Self> {
        let lower = std::fs::canonicalize(lower).unwrap_or_else(|_| lower.to_path_buf());
        let merged = std::env::temp_dir().join(format!("fusion-iso-{}", id));

        let resolution = fusion_iso::resolve(None);
        let mut chosen_backend = None;
        let mut last_err = None;

        for kind in resolution.candidates {
            let backend = fusion_iso::backend(kind);
            if merged.exists() {
                let _ = std::fs::remove_dir_all(&merged);
            }
            match backend.start(&lower, &merged) {
                Ok(()) => {
                    chosen_backend = Some(backend);
                    break;
                }
                Err(e) => {
                    let _ = backend.stop(&merged);
                    if merged.exists() {
                        let _ = std::fs::remove_dir_all(&merged);
                    }
                    last_err = Some(e);
                }
            }
        }

        let backend = chosen_backend.ok_or_else(|| {
            anyhow::anyhow!(
                "Failed to initialize workspace isolation backend: {:?}",
                last_err
            )
        })?;

        Ok(Self {
            lower,
            merged,
            backend,
            cleaned_up: false,
        })
    }

    /// Returns the raw diff from the underlying isolation backend.
    pub async fn diff(&self) -> anyhow::Result<Diff> {
        self.backend
            .diff(&self.lower, &self.merged)
            .await
            .map_err(|e| anyhow::anyhow!("Diff failed: {e}"))
    }

    /// Captures file mutations that actually differ between `merged` and `lower`.
    pub async fn capture_changes(&self) -> anyhow::Result<Diff> {
        let raw_diff = self.diff().await?;
        let mut subagent_changes = Vec::new();

        for file in raw_diff.files {
            if file.path.components().any(|c| c.as_os_str() == ".git") {
                continue;
            }

            let src_path = self.merged.join(&file.path);
            let dest_path = self.lower.join(&file.path);

            match file.op {
                ChangeKind::Added => {
                    if src_path.exists() {
                        if dest_path.exists() {
                            if let (Ok(c1), Ok(c2)) = (tokio::fs::read(&src_path).await, tokio::fs::read(&dest_path).await) {
                                if c1 == c2 {
                                    continue;
                                }
                            }
                        }
                        subagent_changes.push(file);
                    }
                }
                ChangeKind::Modified => {
                    if src_path.exists() && dest_path.exists() {
                        if let (Ok(c1), Ok(c2)) = (tokio::fs::read(&src_path).await, tokio::fs::read(&dest_path).await) {
                            if c1 == c2 {
                                continue;
                            }
                        }
                    }
                    subagent_changes.push(file);
                }
                ChangeKind::Removed => {
                    if dest_path.exists() && !src_path.exists() {
                        subagent_changes.push(file);
                    }
                }
            }
        }

        Ok(Diff { files: subagent_changes })
    }

    /// Merges the captured file mutations from `merged` back into `lower`.
    pub async fn merge(&self, diff: &Diff) -> anyhow::Result<()> {
        for file in &diff.files {
            if file.path.components().any(|c| c.as_os_str() == ".git") {
                continue;
            }

            let src_path = self.merged.join(&file.path);
            let dest_path = self.lower.join(&file.path);

            match file.op {
                ChangeKind::Added | ChangeKind::Modified => {
                    if let Some(parent) = dest_path.parent() {
                        tokio::fs::create_dir_all(parent).await?;
                    }
                    if src_path.is_symlink() {
                        let target = tokio::fs::read_link(&src_path).await?;
                        if dest_path.exists() || dest_path.is_symlink() {
                            let _ = tokio::fs::remove_file(&dest_path).await;
                        }
                        #[cfg(unix)]
                        tokio::fs::symlink(target, &dest_path).await?;
                    } else if src_path.exists() {
                        tokio::fs::copy(&src_path, &dest_path).await?;
                    }
                }
                ChangeKind::Removed => {
                    if dest_path.exists() || dest_path.is_symlink() {
                        let _ = tokio::fs::remove_file(&dest_path).await;
                    }
                }
            }
        }
        Ok(())
    }

    /// Cleans up and tears down the isolated workspace.
    pub fn cleanup(&mut self) {
        if !self.cleaned_up {
            self.cleaned_up = true;
            let _ = self.backend.stop(&self.merged);
            if self.merged.exists() {
                let _ = std::fs::remove_dir_all(&self.merged);
            }
        }
    }
}

impl Drop for WorkspaceIsolation {
    fn drop(&mut self) {
        self.cleanup();
    }
}

/// Core execution loop for a subagent.
async fn execute_subagent_loop(
    id: &str,
    name: &str,
    role: &SubagentRole,
    system_prompt: &str,
    task: &str,
    tools: &ToolRegistry,
    max_turns: usize,
    model: Option<&str>,
    temperature: Option<f32>,
    isolated: bool,
    client: &LlmClient,
    config: &Config,
    progress_tx: &mpsc::UnboundedSender<SubagentProgress>,
    global_tx: &broadcast::Sender<SubagentProgress>,
    loop_event_tx: Option<&mpsc::UnboundedSender<crate::agent::loop_runner::AgentEvent>>,
    cancel_rx: watch::Receiver<bool>,
    active_agents: Arc<RwLock<HashMap<String, SubagentInfo>>>,
) -> anyhow::Result<SubagentResult> {
    let mut session = crate::agent::session::Session::new(model.unwrap_or(&config.default_model));
    session.add_system_message(system_prompt);
    session.add_user_message(task);

    let workspace_iso = if isolated {
        let current_dir = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
        match WorkspaceIsolation::create(&current_dir, id) {
            Ok(iso) => {
                tracing::info!(
                    "Created isolated workspace for subagent '{}' ({}) at {}",
                    name,
                    id,
                    iso.merged.display()
                );
                Some(iso)
            }
            Err(e) => {
                tracing::warn!(
                    "Failed to create isolated workspace for subagent '{}' ({}): {}. Falling back to non-isolated execution.",
                    name,
                    id,
                    e
                );
                None
            }
        }
    } else {
        None
    };

    let tool_ctx = if let Some(iso) = &workspace_iso {
        ToolContext {
            cwd: iso.merged.clone(),
            env: std::env::vars().collect(),
        }
    } else {
        ToolContext::default()
    };
    let mut turns = 0;

    let target_model = model.unwrap_or(&config.default_model);
    let target_temp = temperature.or(config.default_temperature);

    let emit_progress = |event: SubagentProgress| {
        let _ = progress_tx.send(event.clone());
        let _ = global_tx.send(event.clone());
        if let Some(tx) = loop_event_tx {
            if matches!(
                event,
                SubagentProgress::Started { .. }
                    | SubagentProgress::ToolStarted { .. }
                    | SubagentProgress::ToolCompleted { .. }
                    | SubagentProgress::Completed { .. }
                    | SubagentProgress::Failed { .. }
            ) {
                let _ = tx.send(crate::agent::loop_runner::AgentEvent::SubagentProgressEvent {
                    id: id.to_string(),
                    name: name.to_string(),
                    role: role.clone(),
                    progress: event,
                });
            }
        }
    };

    while turns < max_turns {
        if *cancel_rx.borrow() {
            if let Some(mut iso) = workspace_iso {
                iso.cleanup();
            }
            let cancel_event = SubagentProgress::Cancelled { id: id.to_string() };
            emit_progress(cancel_event);

            let mut guard = active_agents.write().await;
            if let Some(info) = guard.get_mut(id) {
                info.status = SubagentStatus::Cancelled;
                info.completed_at = Some(Utc::now().to_rfc3339());
                info.turns = turns;
            }

            return Ok(SubagentResult {
                id: id.to_string(),
                name: name.to_string(),
                role: role.clone(),
                task: task.to_string(),
                output: "Subagent cancelled.".to_string(),
                turns,
                success: false,
                patch: None,
            });
        }

        turns += 1;

        // Emit TurnStarted
        let turn_event = SubagentProgress::TurnStarted {
            id: id.to_string(),
            turn: turns,
            max_turns,
        };
        emit_progress(turn_event);

        // Update active status
        {
            let mut guard = active_agents.write().await;
            if let Some(info) = guard.get_mut(id) {
                info.status = SubagentStatus::Running {
                    turn: turns,
                    current_tool: None,
                };
                info.turns = turns;
            }
        }

        let tool_defs = tools.definitions();

        // Query LLM
        let (key, url) = config.get_key_and_url(&config.default_provider);
        let completion_res = client
            .complete_with(
                &config.default_provider,
                target_model,
                target_temp,
                config.max_tokens,
                key.as_deref(),
                &url,
                session.messages(),
                &tool_defs,
            )
            .await;

        let (content, reasoning, tool_calls) = match completion_res {
            Ok(res) => res,
            Err(e) => {
                if let Some(mut iso) = workspace_iso {
                    iso.cleanup();
                }
                let err_msg = format!("LLM completion failed: {}", e);
                let failed_event = SubagentProgress::Failed {
                    id: id.to_string(),
                    error: err_msg.clone(),
                };
                emit_progress(failed_event);

                let mut guard = active_agents.write().await;
                if let Some(info) = guard.get_mut(id) {
                    info.status = SubagentStatus::Failed {
                        error: err_msg.clone(),
                    };
                    info.completed_at = Some(Utc::now().to_rfc3339());
                    info.turns = turns;
                }

                return Err(anyhow::anyhow!(err_msg));
            }
        };

        // Emit thinking if any
        if let Some(r) = reasoning {
            if !r.is_empty() {
                let thinking_event = SubagentProgress::Thinking {
                    id: id.to_string(),
                    delta: r,
                };
                emit_progress(thinking_event);
            }
        }

        // If no tools called, task completed
        if tool_calls.is_empty() {
            session.add_assistant_message(&content);

            let patch = if let Some(mut iso) = workspace_iso {
                let diff_res = iso.capture_changes().await;
                let captured_patch = match diff_res {
                    Ok(diff) => {
                        let text = diff.unified_text();
                        if !diff.is_empty() {
                            if let Err(e) = iso.merge(&diff).await {
                                tracing::error!(
                                    "Failed to merge isolated changes for subagent '{}' ({}): {}",
                                    name, id, e
                                );
                            } else {
                                tracing::info!(
                                    "Merged {} changed files from isolated workspace for subagent '{}' ({})",
                                    diff.files.len(), name, id
                                );
                            }
                        }
                        iso.cleanup();
                        if text.is_empty() { None } else { Some(text) }
                    }
                    Err(e) => {
                        tracing::warn!(
                            "Failed to capture diff for isolated workspace '{}' ({}): {}",
                            name, id, e
                        );
                        iso.cleanup();
                        None
                    }
                };
                captured_patch
            } else {
                None
            };

            let completed_event = SubagentProgress::Completed {
                id: id.to_string(),
                output: content.clone(),
                turns_taken: turns,
            };
            emit_progress(completed_event);

            let mut guard = active_agents.write().await;
            if let Some(info) = guard.get_mut(id) {
                info.status = SubagentStatus::Completed {
                    output: content.clone(),
                    turns,
                };
                info.completed_at = Some(Utc::now().to_rfc3339());
                info.turns = turns;
            }

            // Persist output artifact for agent://<id> and agent://<name> URI resolution
            let artifacts_dir = tool_ctx.cwd.join(".fusion").join("artifacts");
            if let Ok(_) = std::fs::create_dir_all(&artifacts_dir) {
                let _ = std::fs::write(artifacts_dir.join(format!("{}.txt", id)), &content);
                let _ = std::fs::write(artifacts_dir.join(format!("{}.txt", name)), &content);
            }
            return Ok(SubagentResult {
                id: id.to_string(),
                name: name.to_string(),
                role: role.clone(),
                task: task.to_string(),
                output: content,
                turns,
                success: true,
                patch,
            });
        }

        // Add assistant message with tool calls
        session.add_assistant_with_tools(&content, tool_calls.clone());

        // Execute tools
        for tc in tool_calls {
            if *cancel_rx.borrow() {
                break;
            }

            let parsed_args = match serde_json::from_str::<Value>(&tc.arguments) {
                Ok(v) => v,
                Err(e) => {
                    let err_msg = format!("Invalid JSON arguments: {}", e);
                    session.add_tool_result(&tc.id, &err_msg);
                    continue;
                }
            };

            // Update status current_tool
            {
                let mut guard = active_agents.write().await;
                if let Some(info) = guard.get_mut(id) {
                    info.status = SubagentStatus::Running {
                        turn: turns,
                        current_tool: Some(tc.name.clone()),
                    };
                }
            }

            // Emit ToolStarted
            let tool_started_event = SubagentProgress::ToolStarted {
                id: id.to_string(),
                tool: tc.name.clone(),
                args: parsed_args.clone(),
            };
            emit_progress(tool_started_event);

            // Execute tool
            let exec_res = tools.execute(&tc.name, parsed_args, &tool_ctx).await;
            let (result_str, success) = match exec_res {
                Ok(out) => (out, true),
                Err(e) => (format!("Tool '{}' error: {}", tc.name, e), false),
            };

            // Emit ToolCompleted
            let tool_completed_event = SubagentProgress::ToolCompleted {
                id: id.to_string(),
                tool: tc.name.clone(),
                output: result_str.clone(),
                success,
            };
            emit_progress(tool_completed_event);

            session.add_tool_result(&tc.id, result_str);
        }
    }

    if let Some(mut iso) = workspace_iso {
        iso.cleanup();
    }
    let err_msg = format!(
        "Subagent '{}' ({}) exceeded maximum turns ({})",
        name, id, max_turns
    );
    let failed_event = SubagentProgress::Failed {
        id: id.to_string(),
        error: err_msg.clone(),
    };
    emit_progress(failed_event);

    let mut guard = active_agents.write().await;
    if let Some(info) = guard.get_mut(id) {
        info.status = SubagentStatus::Failed {
            error: err_msg.clone(),
        };
        info.completed_at = Some(Utc::now().to_rfc3339());
        info.turns = turns;
    }

    anyhow::bail!(err_msg)
}

/// Represents a specialized worker subagent with an isolated role, system prompt, and tool registry.
#[derive(Clone)]
pub struct Subagent {
    pub name: String,
    pub role: SubagentRole,
    pub system_prompt: String,
    pub tools: ToolRegistry,
    pub max_turns: usize,
}

impl Subagent {
    /// Creates a new custom Subagent.
    pub fn new(
        name: impl Into<String>,
        role: SubagentRole,
        system_prompt: impl Into<String>,
        tools: ToolRegistry,
    ) -> Self {
        Self {
            name: name.into(),
            role,
            system_prompt: system_prompt.into(),
            tools,
            max_turns: 20,
        }
    }

    /// Sets the maximum turns allowed for the subagent.
    pub fn with_max_turns(mut self, max: usize) -> Self {
        self.max_turns = max;
        self
    }

    /// Predefined Scout subagent for read-only exploration and searching.
    pub fn scout(tools: ToolRegistry) -> Self {
        let role = SubagentRole::Scout;
        let prompt = role.system_prompt("Scout");
        let dedicated = role.filter_tools(&tools);
        Self::new("Scout", role, prompt, dedicated)
    }

    /// Predefined Coder subagent for implementation and refactoring.
    pub fn coder(tools: ToolRegistry) -> Self {
        let role = SubagentRole::Coder;
        let prompt = role.system_prompt("Coder");
        let dedicated = role.filter_tools(&tools);
        Self::new("Coder", role, prompt, dedicated)
    }

    /// Predefined Tester subagent for testing and verification.
    pub fn tester(tools: ToolRegistry) -> Self {
        let role = SubagentRole::Tester;
        let prompt = role.system_prompt("Tester");
        let dedicated = role.filter_tools(&tools);
        Self::new("Tester", role, prompt, dedicated)
    }

    /// Predefined Reviewer subagent for code quality and security review.
    pub fn reviewer(tools: ToolRegistry) -> Self {
        let role = SubagentRole::Reviewer;
        let prompt = role.system_prompt("Reviewer");
        let dedicated = role.filter_tools(&tools);
        Self::new("Reviewer", role, prompt, dedicated)
    }

    /// Runs the subagent on a specific task and returns its final response.
    pub async fn run(
        &self,
        task: &str,
        client: &LlmClient,
        config: &Config,
    ) -> anyhow::Result<String> {
        let manager =
            SubagentManager::new(Arc::new(client.clone()), config.clone(), self.tools.clone());
        let subagent_task = SubagentTask::new(self.role.clone(), task)
            .with_name(&self.name)
            .with_system_prompt(&self.system_prompt)
            .with_tools(self.tools.clone())
            .with_max_turns(self.max_turns);
        let handle = manager.spawn(subagent_task);
        let res = handle.wait().await?;
        if res.success {
            Ok(res.output)
        } else {
            anyhow::bail!("Subagent failed: {}", res.output)
        }
    }
}

/// Spawns and executes a subagent with the given parameters to completion.
pub async fn run_subagent(
    name: &str,
    role: &str,
    task: &str,
    tools: ToolRegistry,
    client: &LlmClient,
    config: &Config,
) -> anyhow::Result<String> {
    let subagent_role = SubagentRole::from_str(role).unwrap_or(SubagentRole::General);
    let manager = SubagentManager::new(Arc::new(client.clone()), config.clone(), tools);
    let subagent_task = SubagentTask::new(subagent_role, task).with_name(name);
    let handle = manager.spawn(subagent_task);
    let res = handle.wait().await?;
    if res.success {
        Ok(res.output)
    } else {
        anyhow::bail!("Subagent execution failed: {}", res.output)
    }
}

/// A Tool that allows the primary agent to delegate tasks to specialized subagents.
pub struct SpawnSubagentTool {
    manager: SubagentManager,
}

impl SpawnSubagentTool {
    pub fn new(client: Arc<LlmClient>, config: Config, tools: ToolRegistry) -> Self {
        Self {
            manager: SubagentManager::new(client, config, tools),
        }
    }

    pub fn from_manager(manager: SubagentManager) -> Self {
        Self { manager }
    }

    pub fn with_event_sender(
        mut self,
        tx: tokio::sync::mpsc::UnboundedSender<crate::agent::loop_runner::AgentEvent>,
    ) -> Self {
        self.manager = self.manager.with_event_sender(tx);
        self
    }

    pub fn set_event_sender(
        &mut self,
        tx: Option<tokio::sync::mpsc::UnboundedSender<crate::agent::loop_runner::AgentEvent>>,
    ) {
        self.manager.set_event_sender(tx);
    }
}

#[async_trait]
impl Tool for SpawnSubagentTool {
    fn name(&self) -> &str {
        "spawn_subagent"
    }

    fn description(&self) -> &str {
        "Spawns a specialized worker subagent (e.g. 'scout', 'coder', 'tester', 'reviewer') to execute an isolated background task with dedicated tools and returns its output."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "role": {
                    "type": "string",
                    "enum": ["scout", "coder", "tester", "reviewer", "general"],
                    "description": "The specialization role of the subagent"
                },
                "name": {
                    "type": "string",
                    "description": "Optional custom name for the subagent"
                },
                "task": {
                    "type": "string",
                    "description": "The detailed instructions/task for the subagent to execute"
                },
                "isolated": {
                    "type": "boolean",
                    "description": "Whether to run the subagent in an isolated copy-on-write workspace"
                }
            },
            "required": ["role", "task"]
        })
    }

    async fn execute(&self, args: Value, _ctx: &ToolContext) -> anyhow::Result<String> {
        let role_str = args
            .get("role")
            .and_then(|v| v.as_str())
            .unwrap_or("general");
        let name_str = args
            .get("name")
            .and_then(|v| v.as_str())
            .unwrap_or(role_str);
        let task_str = args
            .get("task")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("Missing required 'task' argument"))?;

        let role = SubagentRole::from_str(role_str).unwrap_or(SubagentRole::General);
        let mut task = SubagentTask::new(role, task_str).with_name(name_str);
        if let Some(iso) = args.get("isolated").and_then(|v| v.as_bool()) {
            task = task.with_isolated(iso);
        }
        let handle = self.manager.spawn(task);
        let res = handle.wait().await?;
        if res.success {
            Ok(res.output)
        } else {
            anyhow::bail!("Subagent execution failed: {}", res.output)
        }
    }
}

/// A Tool that allows the primary agent to delegate multiple tasks concurrently.
pub struct SpawnBatchSubagentsTool {
    manager: SubagentManager,
}

impl SpawnBatchSubagentsTool {
    pub fn new(client: Arc<LlmClient>, config: Config, tools: ToolRegistry) -> Self {
        Self {
            manager: SubagentManager::new(client, config, tools),
        }
    }

    pub fn from_manager(manager: SubagentManager) -> Self {
        Self { manager }
    }

    pub fn with_event_sender(
        mut self,
        tx: tokio::sync::mpsc::UnboundedSender<crate::agent::loop_runner::AgentEvent>,
    ) -> Self {
        self.manager = self.manager.with_event_sender(tx);
        self
    }

    pub fn set_event_sender(
        &mut self,
        tx: Option<tokio::sync::mpsc::UnboundedSender<crate::agent::loop_runner::AgentEvent>>,
    ) {
        self.manager.set_event_sender(tx);
    }
}

#[async_trait]
impl Tool for SpawnBatchSubagentsTool {
    fn name(&self) -> &str {
        "spawn_subagents_batch"
    }

    fn description(&self) -> &str {
        "Spawns multiple specialized subagents in parallel to execute independent tasks concurrently and returns all outputs."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "tasks": {
                    "type": "array",
                    "description": "List of subagent tasks to run concurrently",
                    "items": {
                        "type": "object",
                        "properties": {
                            "role": {
                                "type": "string",
                                "enum": ["scout", "coder", "tester", "reviewer", "general"],
                                "description": "The role of the subagent"
                            },
                            "name": {
                                "type": "string",
                                "description": "Optional name"
                            },
                            "task": {
                                "type": "string",
                                "description": "The task instructions"
                            },
                            "isolated": {
                                "type": "boolean",
                                "description": "Whether to run in an isolated workspace"
                            }
                        },
                        "required": ["role", "task"]
                    }
                }
            },
            "required": ["tasks"]
        })
    }

    async fn execute(&self, args: Value, _ctx: &ToolContext) -> anyhow::Result<String> {
        let tasks_arr = args
            .get("tasks")
            .and_then(|v| v.as_array())
            .ok_or_else(|| anyhow::anyhow!("Missing 'tasks' array"))?;

        let mut subagent_tasks = Vec::new();
        for item in tasks_arr {
            let role_str = item
                .get("role")
                .and_then(|v| v.as_str())
                .unwrap_or("general");
            let name_str = item
                .get("name")
                .and_then(|v| v.as_str())
                .unwrap_or(role_str);
            let task_str = item.get("task").and_then(|v| v.as_str()).unwrap_or("");
            if task_str.is_empty() {
                continue;
            }
            let role = SubagentRole::from_str(role_str).unwrap_or(SubagentRole::General);
            let mut subagent_task = SubagentTask::new(role, task_str).with_name(name_str);
            if let Some(iso) = item.get("isolated").and_then(|v| v.as_bool()) {
                subagent_task = subagent_task.with_isolated(iso);
            }
            subagent_tasks.push(subagent_task);
        }

        if subagent_tasks.is_empty() {
            return Ok("No valid tasks provided.".to_string());
        }

        let results = self.manager.run_concurrent(subagent_tasks).await;
        let mut output = String::new();
        for (i, res) in results.into_iter().enumerate() {
            match res {
                Ok(r) => {
                    output.push_str(&format!(
                        "### Subagent {} ({})\n{}\n\n",
                        r.name, r.role, r.output
                    ));
                }
                Err(e) => {
                    output.push_str(&format!(
                        "### Subagent task {} failed\nError: {}\n\n",
                        i + 1,
                        e
                    ));
                }
            }
        }

        Ok(output)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    struct DummyTool {
        name: &'static str,
    }

    #[async_trait]
    impl Tool for DummyTool {
        fn name(&self) -> &str {
            self.name
        }
        fn description(&self) -> &str {
            "dummy tool"
        }
        fn parameters(&self) -> Value {
            json!({"type": "object"})
        }
        async fn execute(&self, _args: Value, _ctx: &ToolContext) -> anyhow::Result<String> {
            Ok(format!("executed {}", self.name))
        }
    }

    fn create_full_test_registry() -> ToolRegistry {
        let mut reg = ToolRegistry::new();
        reg.register(Arc::new(DummyTool { name: "read" }));
        reg.register(Arc::new(DummyTool { name: "write" }));
        reg.register(Arc::new(DummyTool { name: "edit" }));
        reg.register(Arc::new(DummyTool { name: "grep" }));
        reg.register(Arc::new(DummyTool { name: "glob" }));
        reg.register(Arc::new(DummyTool { name: "bash" }));
        reg
    }

    #[test]
    fn test_role_filtering() {
        let full = create_full_test_registry();

        // Scout: read, grep, glob
        let scout_tools = SubagentRole::Scout.filter_tools(&full);
        assert!(scout_tools.get("read").is_some());
        assert!(scout_tools.get("grep").is_some());
        assert!(scout_tools.get("glob").is_some());
        assert!(scout_tools.get("write").is_none());
        assert!(scout_tools.get("edit").is_none());
        assert!(scout_tools.get("bash").is_none());

        // Coder: read, write, edit, grep, glob
        let coder_tools = SubagentRole::Coder.filter_tools(&full);
        assert!(coder_tools.get("read").is_some());
        assert!(coder_tools.get("write").is_some());
        assert!(coder_tools.get("edit").is_some());
        assert!(coder_tools.get("grep").is_some());
        assert!(coder_tools.get("glob").is_some());
        assert!(coder_tools.get("bash").is_none());

        // Tester: bash, read, grep, glob
        let tester_tools = SubagentRole::Tester.filter_tools(&full);
        assert!(tester_tools.get("bash").is_some());
        assert!(tester_tools.get("read").is_some());
        assert!(tester_tools.get("grep").is_some());
        assert!(tester_tools.get("glob").is_some());
        assert!(tester_tools.get("write").is_none());
        assert!(tester_tools.get("edit").is_none());

        // Reviewer: read, grep, glob
        let reviewer_tools = SubagentRole::Reviewer.filter_tools(&full);
        assert!(reviewer_tools.get("read").is_some());
        assert!(reviewer_tools.get("grep").is_some());
        assert!(reviewer_tools.get("glob").is_some());
        assert!(reviewer_tools.get("bash").is_none());
        assert!(reviewer_tools.get("write").is_none());

        // General: all
        let general_tools = SubagentRole::General.filter_tools(&full);
        assert!(general_tools.get("bash").is_some());
        assert!(general_tools.get("write").is_some());
    }

    #[test]
    fn test_subagent_task_builders() {
        let task = SubagentTask::scout("Find authentication references")
            .with_name("AuthScout")
            .with_max_turns(10)
            .with_model("claude-3-7-sonnet")
            .with_temperature(0.2);

        assert_eq!(task.role, SubagentRole::Scout);
        assert_eq!(task.name, "AuthScout");
        assert_eq!(task.max_turns, 10);
        assert_eq!(task.model.as_deref(), Some("claude-3-7-sonnet"));
        assert_eq!(task.temperature, Some(0.2));
        assert_eq!(task.task, "Find authentication references");
        assert!(!task.isolated);

        let coder_task = SubagentTask::coder("Fix bug in tokenizer");
        assert_eq!(coder_task.role, SubagentRole::Coder);
        assert_eq!(coder_task.name, "Coder");
        assert!(coder_task.isolated);

        let non_isolated_coder = coder_task.with_isolated(false);
        assert!(!non_isolated_coder.isolated);

        let tester_task = SubagentTask::tester("Run cargo test");
        assert_eq!(tester_task.role, SubagentRole::Tester);
        assert_eq!(tester_task.name, "Tester");
        assert!(!tester_task.isolated);

        let isolated_tester = tester_task.with_isolated(true);
        assert!(isolated_tester.isolated);

        let general_task = SubagentTask::general("General task");
        assert!(!general_task.isolated);

        let reviewer_task = SubagentTask::reviewer("Review code");
        assert!(!reviewer_task.isolated);
    }

    #[test]
    fn test_role_from_str_and_display() {
        assert_eq!(
            SubagentRole::from_str("scout").unwrap(),
            SubagentRole::Scout
        );
        assert_eq!(
            SubagentRole::from_str("CODER").unwrap(),
            SubagentRole::Coder
        );
        assert_eq!(
            SubagentRole::from_str("Tester").unwrap(),
            SubagentRole::Tester
        );
        assert_eq!(
            SubagentRole::from_str("Reviewer").unwrap(),
            SubagentRole::Reviewer
        );
        assert_eq!(
            SubagentRole::from_str("other").unwrap(),
            SubagentRole::General
        );

        assert_eq!(format!("{}", SubagentRole::Scout), "Scout");
        assert_eq!(format!("{}", SubagentRole::Coder), "Coder");
    }

    #[tokio::test]
    async fn test_subagent_manager_channels_and_cancel() {
        let client = Arc::new(LlmClient::new());
        let config = Config::default();
        let tools = create_full_test_registry();

        let manager = SubagentManager::new(client, config, tools).with_max_concurrent(4);
        let mut subscriber = manager.subscribe();

        let task = SubagentTask::scout("Analyze memory usage")
            .with_id("test-scout-1")
            .with_name("MemScout");

        let handle = manager.spawn(task);
        assert_eq!(handle.id(), "test-scout-1");
        assert_eq!(handle.name(), "MemScout");

        // Cancel the task
        handle.cancel();

        // Check if event arrives via subscriber or handle
        let event = subscriber.recv().await;
        assert!(event.is_ok());
        let ev = event.unwrap();
        assert_eq!(ev.id(), "test-scout-1");
    }

    #[tokio::test]
    async fn test_spawn_subagent_tool_schema() {
        let client = Arc::new(LlmClient::new());
        let config = Config::default();
        let tools = ToolRegistry::new();

        let tool = SpawnSubagentTool::new(client.clone(), config.clone(), tools.clone());
        assert_eq!(tool.name(), "spawn_subagent");
        let params = tool.parameters();
        assert!(params.get("properties").is_some());
        assert!(params["properties"].get("role").is_some());
        assert!(params["properties"].get("task").is_some());
        assert!(params["properties"].get("isolated").is_some());

        let batch_tool = SpawnBatchSubagentsTool::new(client, config, tools);
        assert_eq!(batch_tool.name(), "spawn_subagents_batch");
        let batch_params = batch_tool.parameters();
        assert!(batch_params["properties"].get("tasks").is_some());
        let task_item_props = &batch_params["properties"]["tasks"]["items"]["properties"];
        assert!(task_item_props.get("isolated").is_some());
    }

    #[tokio::test]
    async fn test_subagent_manager_concurrency_defaults_and_config() {
        let client = Arc::new(LlmClient::new());
        let tools = ToolRegistry::new();

        // 1. Default config uses DEFAULT_MAX_CONCURRENT_SUBAGENTS (16)
        let config = Config::default();
        let manager = SubagentManager::new(client.clone(), config, tools.clone());
        assert_eq!(manager.max_concurrent(), DEFAULT_MAX_CONCURRENT_SUBAGENTS);
        assert_eq!(manager.semaphore.available_permits(), 16);

        // 2. Custom config override
        let mut custom_config = Config::default();
        custom_config.max_concurrent_subagents = Some(24);
        let manager2 = SubagentManager::new(client, custom_config, tools);
        assert_eq!(manager2.max_concurrent(), 24);
        assert_eq!(manager2.semaphore.available_permits(), 24);
    }

    #[tokio::test]
    async fn test_subagent_progress_event_bridging() {
        let client = Arc::new(LlmClient::new());
        let config = Config::default();
        let tools = create_full_test_registry();

        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        let manager = SubagentManager::new(client, config, tools)
            .with_max_concurrent(4)
            .with_event_sender(tx);

        let task = SubagentTask::scout("Inspect workspace")
            .with_id("test-bridge-1")
            .with_name("BridgeScout");

        let handle = manager.spawn(task);
        let event = rx.recv().await;
        assert!(event.is_some());
        if let Some(crate::agent::loop_runner::AgentEvent::SubagentProgressEvent {
            id,
            name,
            role,
            progress,
        }) = event
        {
            assert_eq!(id, "test-bridge-1");
            assert_eq!(name, "BridgeScout");
            assert_eq!(role, SubagentRole::Scout);
            assert!(matches!(progress, SubagentProgress::Started { .. }));
        } else {
            panic!("Expected SubagentProgressEvent");
        }
        handle.cancel();
    }
    #[tokio::test]
    async fn test_workspace_isolation_lifecycle() {
        let test_id = format!("test-life-{}", uuid::Uuid::new_v4().to_string()[..8].to_string());
        let temp_dir = std::env::temp_dir().join(&test_id);
        let _ = std::fs::remove_dir_all(&temp_dir);
        std::fs::create_dir_all(&temp_dir).expect("Failed to create test temp dir");

        let hello_path = temp_dir.join("hello.txt");
        std::fs::write(&hello_path, "Hello Initial").expect("Failed to write initial file");

        let mut iso = WorkspaceIsolation::create(&temp_dir, &test_id)
            .expect("Failed to create WorkspaceIsolation");

        assert!(iso.merged.exists());
        assert_eq!(
            std::fs::read_to_string(iso.merged.join("hello.txt")).unwrap(),
            "Hello Initial"
        );

        // Modify an existing file and create a new file in isolated workspace
        std::fs::write(iso.merged.join("hello.txt"), "Hello Modified in Isolated Workspace")
            .expect("Failed to write modified file");
        std::fs::write(iso.merged.join("created.txt"), "Newly Created File")
            .expect("Failed to write newly created file");

        // Verify the original directory is completely untouched!
        assert_eq!(
            std::fs::read_to_string(&hello_path).unwrap(),
            "Hello Initial"
        );
        assert!(!temp_dir.join("created.txt").exists());

        // Capture changes
        let changes = iso.capture_changes().await.expect("Failed to capture changes");
        assert!(!changes.is_empty());

        // Merge changes back into lower
        iso.merge(&changes).await.expect("Failed to merge changes");

        // Verify lower now has the merged files
        assert_eq!(
            std::fs::read_to_string(&hello_path).unwrap(),
            "Hello Modified in Isolated Workspace"
        );
        assert_eq!(
            std::fs::read_to_string(temp_dir.join("created.txt")).unwrap(),
            "Newly Created File"
        );

        // Cleanup
        iso.cleanup();
        assert!(!iso.merged.exists());

        let _ = std::fs::remove_dir_all(&temp_dir);
    }

    #[tokio::test]
    async fn test_workspace_isolation_cancelled_no_merge() {
        let test_id = format!("test-cancel-{}", uuid::Uuid::new_v4().to_string()[..8].to_string());
        let temp_dir = std::env::temp_dir().join(&test_id);
        let _ = std::fs::remove_dir_all(&temp_dir);
        std::fs::create_dir_all(&temp_dir).expect("Failed to create test temp dir");

        let data_path = temp_dir.join("data.txt");
        std::fs::write(&data_path, "Original Data").expect("Failed to write original data");

        let mut iso = WorkspaceIsolation::create(&temp_dir, &test_id)
            .expect("Failed to create WorkspaceIsolation");

        // Mutate in isolated workspace
        std::fs::write(iso.merged.join("data.txt"), "Mutated Data in Isolation")
            .expect("Failed to write mutated data");
        std::fs::write(iso.merged.join("bad.txt"), "Unwanted file")
            .expect("Failed to write unwanted file");

        // Simulate cancellation / failure: cleanup without merge
        iso.cleanup();

        // Verify original directory is completely untouched
        assert_eq!(
            std::fs::read_to_string(&data_path).unwrap(),
            "Original Data"
        );
        assert!(!temp_dir.join("bad.txt").exists());
        assert!(!iso.merged.exists());

        let _ = std::fs::remove_dir_all(&temp_dir);
    }
}
