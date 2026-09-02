use std::collections::HashMap;
use std::fmt;
use std::str::FromStr;
use std::sync::Arc;

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
            .finish()
    }
}

impl SubagentTask {
    /// Creates a new subagent task specification.
    pub fn new(role: SubagentRole, task: impl Into<String>) -> Self {
        let task_str = task.into();
        let role_name = role.default_name().to_string();
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
    pub fn take_progress_receiver(
        &mut self,
    ) -> Option<mpsc::UnboundedReceiver<SubagentProgress>> {
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
}

impl SubagentManager {
    /// Creates a new `SubagentManager`.
    pub fn new(client: Arc<LlmClient>, config: Config, tools: ToolRegistry) -> Self {
        let (global_event_tx, _) = broadcast::channel(256);
        let max_concurrent = 8;
        Self {
            client,
            config,
            tools,
            max_concurrent,
            semaphore: Arc::new(Semaphore::new(max_concurrent)),
            active_agents: Arc::new(RwLock::new(HashMap::new())),
            cancels: Arc::new(RwLock::new(HashMap::new())),
            global_event_tx,
        }
    }

    /// Configures the maximum number of subagents allowed to run concurrently.
    pub fn with_max_concurrent(mut self, max: usize) -> Self {
        self.max_concurrent = max;
        self.semaphore = Arc::new(Semaphore::new(max));
        self
    }

    /// Subscribes to the broadcast channel of all subagent progress events.
    pub fn subscribe(&self) -> broadcast::Receiver<SubagentProgress> {
        self.global_event_tx.subscribe()
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

        let task_id = id.clone();
        let task_name = name.clone();
        let task_role = role.clone();
        let custom_model = task.model.clone();
        let custom_temperature = task.temperature;

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
            let _ = global_tx.send(start_event);

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
                &client,
                &config,
                &progress_tx,
                &global_tx,
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
    client: &LlmClient,
    config: &Config,
    progress_tx: &mpsc::UnboundedSender<SubagentProgress>,
    global_tx: &broadcast::Sender<SubagentProgress>,
    cancel_rx: watch::Receiver<bool>,
    active_agents: Arc<RwLock<HashMap<String, SubagentInfo>>>,
) -> anyhow::Result<SubagentResult> {
    let mut session =
        crate::agent::session::Session::new(model.unwrap_or(&config.default_model));
    session.add_system_message(system_prompt);
    session.add_user_message(task);

    let tool_ctx = ToolContext::default();
    let mut turns = 0;

    let target_model = model.unwrap_or(&config.default_model);
    let target_temp = temperature.or(config.default_temperature);

    while turns < max_turns {
        if *cancel_rx.borrow() {
            let cancel_event = SubagentProgress::Cancelled {
                id: id.to_string(),
            };
            let _ = progress_tx.send(cancel_event.clone());
            let _ = global_tx.send(cancel_event);

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
            });
        }

        turns += 1;

        // Emit TurnStarted
        let turn_event = SubagentProgress::TurnStarted {
            id: id.to_string(),
            turn: turns,
            max_turns,
        };
        let _ = progress_tx.send(turn_event.clone());
        let _ = global_tx.send(turn_event);

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
                let err_msg = format!("LLM completion failed: {}", e);
                let failed_event = SubagentProgress::Failed {
                    id: id.to_string(),
                    error: err_msg.clone(),
                };
                let _ = progress_tx.send(failed_event.clone());
                let _ = global_tx.send(failed_event);

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
                let _ = progress_tx.send(thinking_event.clone());
                let _ = global_tx.send(thinking_event);
            }
        }

        // If no tools called, task completed
        if tool_calls.is_empty() {
            session.add_assistant_message(&content);

            let completed_event = SubagentProgress::Completed {
                id: id.to_string(),
                output: content.clone(),
                turns_taken: turns,
            };
            let _ = progress_tx.send(completed_event.clone());
            let _ = global_tx.send(completed_event);

            let mut guard = active_agents.write().await;
            if let Some(info) = guard.get_mut(id) {
                info.status = SubagentStatus::Completed {
                    output: content.clone(),
                    turns,
                };
                info.completed_at = Some(Utc::now().to_rfc3339());
                info.turns = turns;
            }

            return Ok(SubagentResult {
                id: id.to_string(),
                name: name.to_string(),
                role: role.clone(),
                task: task.to_string(),
                output: content,
                turns,
                success: true,
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
            let _ = progress_tx.send(tool_started_event.clone());
            let _ = global_tx.send(tool_started_event);

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
            let _ = progress_tx.send(tool_completed_event.clone());
            let _ = global_tx.send(tool_completed_event);

            session.add_tool_result(&tc.id, result_str);
        }
    }

    let err_msg = format!(
        "Subagent '{}' ({}) exceeded maximum turns ({})",
        name, id, max_turns
    );
    let failed_event = SubagentProgress::Failed {
        id: id.to_string(),
        error: err_msg.clone(),
    };
    let _ = progress_tx.send(failed_event.clone());
    let _ = global_tx.send(failed_event);

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
        let manager = SubagentManager::new(
            Arc::new(client.clone()),
            config.clone(),
            self.tools.clone(),
        );
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
        let task = SubagentTask::new(role, task_str).with_name(name_str);
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
            subagent_tasks.push(SubagentTask::new(role, task_str).with_name(name_str));
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

        let coder_task = SubagentTask::coder("Fix bug in tokenizer");
        assert_eq!(coder_task.role, SubagentRole::Coder);
        assert_eq!(coder_task.name, "Coder");

        let tester_task = SubagentTask::tester("Run cargo test");
        assert_eq!(tester_task.role, SubagentRole::Tester);
        assert_eq!(tester_task.name, "Tester");
    }

    #[test]
    fn test_role_from_str_and_display() {
        assert_eq!(SubagentRole::from_str("scout").unwrap(), SubagentRole::Scout);
        assert_eq!(SubagentRole::from_str("CODER").unwrap(), SubagentRole::Coder);
        assert_eq!(SubagentRole::from_str("Tester").unwrap(), SubagentRole::Tester);
        assert_eq!(SubagentRole::from_str("Reviewer").unwrap(), SubagentRole::Reviewer);
        assert_eq!(SubagentRole::from_str("other").unwrap(), SubagentRole::General);

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

    #[test]
    fn test_spawn_subagent_tool_schema() {
        let client = Arc::new(LlmClient::new());
        let config = Config::default();
        let tools = ToolRegistry::new();

        let tool = SpawnSubagentTool::new(client.clone(), config.clone(), tools.clone());
        assert_eq!(tool.name(), "spawn_subagent");
        let params = tool.parameters();
        assert!(params.get("properties").is_some());
        assert!(params["properties"].get("role").is_some());
        assert!(params["properties"].get("task").is_some());

        let batch_tool = SpawnBatchSubagentsTool::new(client, config, tools);
        assert_eq!(batch_tool.name(), "spawn_subagents_batch");
        let batch_params = batch_tool.parameters();
        assert!(batch_params["properties"].get("tasks").is_some());
    }
}
