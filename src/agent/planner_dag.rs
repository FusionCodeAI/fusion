use std::collections::{HashMap, HashSet, VecDeque};
use std::fmt;
use std::str::FromStr;
use std::sync::Arc;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use chrono::Utc;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tokio::sync::{mpsc, Semaphore};
use uuid::Uuid;

use crate::agent::subagent::{SubagentManager, SubagentRole, SubagentTask};
use crate::tools::types::{Tool, ToolContext};

/// Errors encountered during DAG planning, validation, decomposition, or execution.
#[derive(Debug, thiserror::Error, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum DagPlannerError {
    #[error("Task '{task_id}' specifies missing dependency '{missing_dep}'")]
    MissingDependency {
        task_id: String,
        missing_dep: String,
    },

    #[error("Cycle detected in DAG involving task(s): {cycle:?}")]
    CycleDetected { cycle: Vec<String> },

    #[error("Duplicate task ID '{task_id}' in DAG")]
    DuplicateTaskId { task_id: String },

    #[error("Task '{task_id}' not found in DAG")]
    TaskNotFound { task_id: String },

    #[error("Cannot plan or execute an empty DAG")]
    EmptyDag,

    #[error("Task '{task_id}' cannot depend on itself")]
    SelfDependency { task_id: String },

    #[error("DAG execution failed: {reason}")]
    ExecutionFailed { reason: String },

    #[error("Stage #{stage_index} execution error: {reason}")]
    StageError { stage_index: usize, reason: String },

    #[error("JSON parsing error: {error}")]
    JsonParseError { error: String },

    #[error("Task '{task_id}' timed out after {timeout_secs}s")]
    TaskTimeout { task_id: String, timeout_secs: u64 },

    #[error("Invalid decomposition strategy: {message}")]
    InvalidStrategy { message: String },
}

/// Priority ranking for DAG task execution ordering.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DagTaskPriority {
    Low = 0,
    Normal = 1,
    High = 2,
    Critical = 3,
}

impl Default for DagTaskPriority {
    fn default() -> Self {
        DagTaskPriority::Normal
    }
}

impl fmt::Display for DagTaskPriority {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DagTaskPriority::Low => write!(f, "low"),
            DagTaskPriority::Normal => write!(f, "normal"),
            DagTaskPriority::High => write!(f, "high"),
            DagTaskPriority::Critical => write!(f, "critical"),
        }
    }
}

impl FromStr for DagTaskPriority {
    type Err = std::convert::Infallible;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(match s.trim().to_lowercase().as_str() {
            "low" => DagTaskPriority::Low,
            "high" => DagTaskPriority::High,
            "critical" | "urgent" => DagTaskPriority::Critical,
            _ => DagTaskPriority::Normal,
        })
    }
}

/// Lifecycle status of an individual task node in the DAG.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "status", content = "data", rename_all = "snake_case")]
pub enum DagTaskStatus {
    /// Task has been defined but its upstream dependencies have not completed yet.
    Pending,
    /// All upstream dependencies completed successfully; task is queued and ready for an execution slot.
    Ready,
    /// Task is actively executing on a subagent.
    Running {
        agent_id: String,
        started_at: String,
    },
    /// Task completed successfully with output.
    Completed {
        output: String,
        finished_at: String,
        duration_ms: u64,
    },
    /// Task failed with error details.
    Failed {
        error: String,
        failed_at: String,
        retry_count: usize,
    },
    /// Task was skipped (e.g. upstream failure in strict mode or conditional bypass).
    Skipped {
        reason: String,
    },
    /// Task is blocked by external constraint or manual hold.
    Blocked {
        reason: String,
    },
}

impl DagTaskStatus {
    pub fn is_pending(&self) -> bool {
        matches!(self, DagTaskStatus::Pending)
    }

    pub fn is_ready(&self) -> bool {
        matches!(self, DagTaskStatus::Ready)
    }

    pub fn is_running(&self) -> bool {
        matches!(self, DagTaskStatus::Running { .. })
    }

    pub fn is_completed(&self) -> bool {
        matches!(self, DagTaskStatus::Completed { .. })
    }

    pub fn is_failed(&self) -> bool {
        matches!(self, DagTaskStatus::Failed { .. })
    }

    pub fn is_skipped(&self) -> bool {
        matches!(self, DagTaskStatus::Skipped { .. })
    }

    pub fn is_blocked(&self) -> bool {
        matches!(self, DagTaskStatus::Blocked { .. })
    }

    pub fn is_terminal(&self) -> bool {
        matches!(
            self,
            DagTaskStatus::Completed { .. }
                | DagTaskStatus::Failed { .. }
                | DagTaskStatus::Skipped { .. }
        )
    }

    pub fn status_label(&self) -> &'static str {
        match self {
            DagTaskStatus::Pending => "Pending",
            DagTaskStatus::Ready => "Ready",
            DagTaskStatus::Running { .. } => "Running",
            DagTaskStatus::Completed { .. } => "Completed",
            DagTaskStatus::Failed { .. } => "Failed",
            DagTaskStatus::Skipped { .. } => "Skipped",
            DagTaskStatus::Blocked { .. } => "Blocked",
        }
    }

    pub fn status_symbol(&self) -> &'static str {
        match self {
            DagTaskStatus::Pending => "[ ]",
            DagTaskStatus::Ready => "[·]",
            DagTaskStatus::Running { .. } => "[▶]",
            DagTaskStatus::Completed { .. } => "[✓]",
            DagTaskStatus::Failed { .. } => "[✗]",
            DagTaskStatus::Skipped { .. } => "[-]",
            DagTaskStatus::Blocked { .. } => "[⏸]",
        }
    }
}

impl fmt::Display for DagTaskStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.status_label())
    }
}

/// Overall lifecycle status of the entire DAG.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DagOverallStatus {
    NotStarted,
    InProgress,
    Completed,
    Failed,
    PartiallyCompleted,
    Cancelled,
}

impl fmt::Display for DagOverallStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DagOverallStatus::NotStarted => write!(f, "Not Started"),
            DagOverallStatus::InProgress => write!(f, "In Progress"),
            DagOverallStatus::Completed => write!(f, "Completed"),
            DagOverallStatus::Failed => write!(f, "Failed"),
            DagOverallStatus::PartiallyCompleted => write!(f, "Partially Completed"),
            DagOverallStatus::Cancelled => write!(f, "Cancelled"),
        }
    }
}

/// Status of a parallel execution stage wave.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StageStatus {
    Pending,
    InProgress,
    Completed,
    Failed,
    Skipped,
    PartialSuccess,
}

impl fmt::Display for StageStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            StageStatus::Pending => write!(f, "Pending"),
            StageStatus::InProgress => write!(f, "In Progress"),
            StageStatus::Completed => write!(f, "Completed"),
            StageStatus::Failed => write!(f, "Failed"),
            StageStatus::Skipped => write!(f, "Skipped"),
            StageStatus::PartialSuccess => write!(f, "Partial Success"),
        }
    }
}

/// An individual task node in the Subagent DAG graph.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DagTask {
    /// Unique task ID (e.g. "scout_arch", "coder_auth", "tester_e2e").
    pub id: String,
    /// Short human-readable title.
    pub title: String,
    /// Detailed prompt / instructions for the assigned subagent.
    pub description: String,
    /// Subagent role specialized for this task.
    pub role: SubagentRole,
    /// List of task IDs that must complete before this task can execute.
    pub dependencies: Vec<String>,
    /// Current execution status.
    pub status: DagTaskStatus,
    /// Scheduling priority.
    pub priority: DagTaskPriority,
    /// Output result payload if completed.
    pub result: Option<String>,
    /// Error message if failed.
    pub error: Option<String>,
    /// Number of retry attempts made so far.
    pub retry_count: usize,
    /// Maximum number of retries allowed before declaring failure.
    pub max_retries: usize,
    /// Optional execution timeout in seconds.
    pub timeout_secs: Option<u64>,
    /// ID of assigned subagent during execution.
    pub assigned_agent_id: Option<String>,
    /// ISO-8601 timestamp when execution started.
    pub started_at: Option<String>,
    /// ISO-8601 timestamp when execution finished.
    pub finished_at: Option<String>,
    /// Total duration in milliseconds.
    pub duration_ms: Option<u64>,
    /// Arbitrary tags for categorization and filtering.
    pub tags: Vec<String>,
    /// Optional template string with placeholders (e.g. `{{upstream.scout.output}}`).
    pub context_template: Option<String>,
    /// Additional metadata dictionary.
    pub metadata: HashMap<String, Value>,
}

impl DagTask {
    /// Creates a new DAG task with required fields and defaults.
    pub fn new(
        id: impl Into<String>,
        title: impl Into<String>,
        role: SubagentRole,
        description: impl Into<String>,
    ) -> Self {
        Self {
            id: id.into(),
            title: title.into(),
            description: description.into(),
            role,
            dependencies: Vec::new(),
            status: DagTaskStatus::Pending,
            priority: DagTaskPriority::Normal,
            result: None,
            error: None,
            retry_count: 0,
            max_retries: 1,
            timeout_secs: None,
            assigned_agent_id: None,
            started_at: None,
            finished_at: None,
            duration_ms: None,
            tags: Vec::new(),
            context_template: None,
            metadata: HashMap::new(),
        }
    }

    /// Builder: adds a prerequisite dependency.
    pub fn with_dependency(mut self, dep_id: impl Into<String>) -> Self {
        let dep = dep_id.into();
        if !self.dependencies.contains(&dep) {
            self.dependencies.push(dep);
        }
        self
    }

    /// Builder: adds multiple prerequisite dependencies.
    pub fn with_dependencies<I, S>(mut self, deps: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        for dep in deps {
            let dep_str = dep.into();
            if !self.dependencies.contains(&dep_str) {
                self.dependencies.push(dep_str);
            }
        }
        self
    }

    /// Builder: sets task priority.
    pub fn with_priority(mut self, priority: DagTaskPriority) -> Self {
        self.priority = priority;
        self
    }

    /// Builder: sets max retry attempts.
    pub fn with_max_retries(mut self, max_retries: usize) -> Self {
        self.max_retries = max_retries;
        self
    }

    /// Builder: sets execution timeout in seconds.
    pub fn with_timeout_secs(mut self, timeout_secs: u64) -> Self {
        self.timeout_secs = Some(timeout_secs);
        self
    }

    /// Builder: adds a tag.
    pub fn with_tag(mut self, tag: impl Into<String>) -> Self {
        self.tags.push(tag.into());
        self
    }

    /// Builder: sets context template.
    pub fn with_context_template(mut self, template: impl Into<String>) -> Self {
        self.context_template = Some(template.into());
        self
    }

    /// Builder: adds a key-value metadata entry.
    pub fn with_metadata(mut self, key: impl Into<String>, value: Value) -> Self {
        self.metadata.insert(key.into(), value);
        self
    }
}

/// A stage wave of tasks that can execute in parallel.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DagStage {
    /// Zero-based stage index in topological order.
    pub stage_index: usize,
    /// Human-readable name for the stage (e.g. "Stage 1: Exploration & Research").
    pub name: String,
    /// List of task IDs belonging to this parallel wave.
    pub task_ids: Vec<String>,
    /// Execution status of the stage.
    pub status: StageStatus,
    /// Timestamp when stage execution started.
    pub started_at: Option<String>,
    /// Timestamp when stage execution completed.
    pub finished_at: Option<String>,
    /// Total duration of the stage in milliseconds.
    pub duration_ms: Option<u64>,
}

impl DagStage {
    pub fn new(stage_index: usize, name: impl Into<String>, task_ids: Vec<String>) -> Self {
        Self {
            stage_index,
            name: name.into(),
            task_ids,
            status: StageStatus::Pending,
            started_at: None,
            finished_at: None,
            duration_ms: None,
        }
    }

    pub fn task_count(&self) -> usize {
        self.task_ids.len()
    }

    pub fn is_completed(&self) -> bool {
        self.status == StageStatus::Completed
    }

    pub fn is_failed(&self) -> bool {
        self.status == StageStatus::Failed
    }
}

/// The complete Subagent DAG (Directed Acyclic Graph) container.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubagentDag {
    /// Unique identifier for the DAG.
    pub id: String,
    /// Name/title of the DAG plan.
    pub name: String,
    /// Overarching goal being decomposed.
    pub goal: String,
    /// Task nodes keyed by task ID.
    pub tasks: HashMap<String, DagTask>,
    /// Computed parallel execution stages.
    pub stages: Vec<DagStage>,
    /// Timestamp when DAG was created.
    pub created_at: String,
    /// Additional metadata.
    pub metadata: HashMap<String, Value>,
}

impl SubagentDag {
    /// Creates a new empty Subagent DAG.
    pub fn new(name: impl Into<String>, goal: impl Into<String>) -> Self {
        Self {
            id: Uuid::new_v4().to_string()[..8].to_string(),
            name: name.into(),
            goal: goal.into(),
            tasks: HashMap::new(),
            stages: Vec::new(),
            created_at: Utc::now().to_rfc3339(),
            metadata: HashMap::new(),
        }
    }

    /// Adds a task to the DAG. Returns error if task ID is duplicated.
    pub fn add_task(&mut self, task: DagTask) -> Result<(), DagPlannerError> {
        if self.tasks.contains_key(&task.id) {
            return Err(DagPlannerError::DuplicateTaskId {
                task_id: task.id.clone(),
            });
        }
        self.tasks.insert(task.id.clone(), task);
        Ok(())
    }

    /// Removes a task and cleans up any references from other tasks.
    pub fn remove_task(&mut self, task_id: &str) -> Option<DagTask> {
        let removed = self.tasks.remove(task_id);
        if removed.is_some() {
            for task in self.tasks.values_mut() {
                task.dependencies.retain(|dep| dep != task_id);
            }
            self.stages.clear();
        }
        removed
    }

    /// Returns a reference to a task by ID.
    pub fn get_task(&self, task_id: &str) -> Option<&DagTask> {
        self.tasks.get(task_id)
    }

    /// Returns a mutable reference to a task by ID.
    pub fn get_task_mut(&mut self, task_id: &str) -> Option<&mut DagTask> {
        self.tasks.get_mut(task_id)
    }

    /// Total number of tasks in the DAG.
    pub fn task_count(&self) -> usize {
        self.tasks.len()
    }

    /// Checks if the DAG is empty.
    pub fn is_empty(&self) -> bool {
        self.tasks.is_empty()
    }

    /// Validates the DAG: checks for missing dependencies, self-loops, and cycles.
    pub fn validate(&self) -> Result<(), DagPlannerError> {
        if self.tasks.is_empty() {
            return Err(DagPlannerError::EmptyDag);
        }

        // 1. Check for missing dependencies and self-loops
        for (id, task) in &self.tasks {
            for dep in &task.dependencies {
                if dep == id {
                    return Err(DagPlannerError::SelfDependency {
                        task_id: id.clone(),
                    });
                }
                if !self.tasks.contains_key(dep) {
                    return Err(DagPlannerError::MissingDependency {
                        task_id: id.clone(),
                        missing_dep: dep.clone(),
                    });
                }
            }
        }

        // 2. Cycle detection via Kahn's algorithm
        let mut in_degree: HashMap<String, usize> = HashMap::new();
        let mut adjacency: HashMap<String, Vec<String>> = HashMap::new();

        for id in self.tasks.keys() {
            in_degree.insert(id.clone(), 0);
            adjacency.insert(id.clone(), Vec::new());
        }

        for (id, task) in &self.tasks {
            for dep in &task.dependencies {
                // dep -> id
                adjacency.get_mut(dep).unwrap().push(id.clone());
                *in_degree.get_mut(id).unwrap() += 1;
            }
        }

        let mut queue: VecDeque<String> = VecDeque::new();
        for (id, &deg) in &in_degree {
            if deg == 0 {
                queue.push_back(id.clone());
            }
        }

        let mut visited_count = 0;
        while let Some(node) = queue.pop_front() {
            visited_count += 1;
            if let Some(neighbors) = adjacency.get(&node) {
                for neighbor in neighbors {
                    let deg = in_degree.get_mut(neighbor).unwrap();
                    *deg -= 1;
                    if *deg == 0 {
                        queue.push_back(neighbor.clone());
                    }
                }
            }
        }

        if visited_count < self.tasks.len() {
            // Collect nodes participating in cycles
            let cycle_nodes: Vec<String> = in_degree
                .into_iter()
                .filter(|(_, deg)| *deg > 0)
                .map(|(id, _)| id)
                .collect();
            return Err(DagPlannerError::CycleDetected { cycle: cycle_nodes });
        }

        Ok(())
    }

    /// Computes a valid topological ordering of task IDs.
    pub fn topological_sort(&self) -> Result<Vec<String>, DagPlannerError> {
        self.validate()?;

        let mut in_degree: HashMap<String, usize> = HashMap::new();
        let mut adjacency: HashMap<String, Vec<String>> = HashMap::new();

        for id in self.tasks.keys() {
            in_degree.insert(id.clone(), 0);
            adjacency.insert(id.clone(), Vec::new());
        }

        for (id, task) in &self.tasks {
            for dep in &task.dependencies {
                adjacency.get_mut(dep).unwrap().push(id.clone());
                *in_degree.get_mut(id).unwrap() += 1;
            }
        }

        // Priority-aware queue: sort initial roots by priority descending
        let mut ready_nodes: Vec<String> = in_degree
            .iter()
            .filter(|(_, &deg)| deg == 0)
            .map(|(id, _)| id.clone())
            .collect();

        ready_nodes.sort_by(|a, b| {
            let prio_a = self.tasks.get(a).map(|t| t.priority).unwrap_or_default();
            let prio_b = self.tasks.get(b).map(|t| t.priority).unwrap_or_default();
            prio_b.cmp(&prio_a).then_with(|| a.cmp(b))
        });

        let mut queue: VecDeque<String> = ready_nodes.into();
        let mut result = Vec::with_capacity(self.tasks.len());

        while let Some(node) = queue.pop_front() {
            result.push(node.clone());
            if let Some(neighbors) = adjacency.get(&node) {
                let mut newly_ready = Vec::new();
                for neighbor in neighbors {
                    let deg = in_degree.get_mut(neighbor).unwrap();
                    *deg -= 1;
                    if *deg == 0 {
                        newly_ready.push(neighbor.clone());
                    }
                }
                // Sort newly ready by priority descending
                newly_ready.sort_by(|a, b| {
                    let prio_a = self.tasks.get(a).map(|t| t.priority).unwrap_or_default();
                    let prio_b = self.tasks.get(b).map(|t| t.priority).unwrap_or_default();
                    prio_b.cmp(&prio_a).then_with(|| a.cmp(b))
                });
                for r in newly_ready {
                    queue.push_back(r);
                }
            }
        }

        Ok(result)
    }

    /// Partitions tasks into parallel execution stages (waves) where every task in Stage K
    /// only depends on tasks from Stages < K.
    pub fn compute_stages(&mut self) -> Result<&Vec<DagStage>, DagPlannerError> {
        self.validate()?;

        // Calculate depth/level for each node: level(node) = 0 if root else max(level(parent)) + 1
        let mut levels: HashMap<String, usize> = HashMap::new();
        let topo = self.topological_sort()?;

        for id in &topo {
            let task = &self.tasks[id];
            if task.dependencies.is_empty() {
                levels.insert(id.clone(), 0);
            } else {
                let max_parent_level = task
                    .dependencies
                    .iter()
                    .map(|dep| *levels.get(dep).unwrap_or(&0))
                    .max()
                    .unwrap_or(0);
                levels.insert(id.clone(), max_parent_level + 1);
            }
        }

        let max_level = levels.values().copied().max().unwrap_or(0);
        let mut stage_groups: Vec<Vec<String>> = vec![Vec::new(); max_level + 1];

        for id in topo {
            let lvl = levels[&id];
            stage_groups[lvl].push(id);
        }

        let mut stages = Vec::with_capacity(stage_groups.len());
        for (idx, task_ids) in stage_groups.into_iter().enumerate() {
            let stage_name = self.infer_stage_name(idx, &task_ids);
            stages.push(DagStage::new(idx, stage_name, task_ids));
        }

        self.stages = stages;
        Ok(&self.stages)
    }

    /// Infers a human-friendly name for a stage based on the roles of its member tasks.
    fn infer_stage_name(&self, idx: usize, task_ids: &[String]) -> String {
        let mut roles: HashSet<SubagentRole> = HashSet::new();
        for id in task_ids {
            if let Some(task) = self.tasks.get(id) {
                roles.insert(task.role.clone());
            }
        }

        let role_desc = if roles.len() == 1 {
            match roles.iter().next().unwrap() {
                SubagentRole::Scout => "Exploration & Codebase Discovery",
                SubagentRole::Coder => "Implementation & Modification",
                SubagentRole::Tester => "Testing & Behavior Verification",
                SubagentRole::Reviewer => "Code Quality & Security Review",
                SubagentRole::General => "General Task Execution",
                SubagentRole::Custom { name, .. } => name.as_str(),
            }
        } else if roles.contains(&SubagentRole::Scout) {
            "Research & Discovery Wave"
        } else if roles.contains(&SubagentRole::Coder) {
            "Parallel Development Wave"
        } else if roles.contains(&SubagentRole::Tester) {
            "Verification & QA Wave"
        } else if roles.contains(&SubagentRole::Reviewer) {
            "Multi-Agent Review Wave"
        } else {
            "Parallel Task Wave"
        };

        format!("Stage {}: {}", idx + 1, role_desc)
    }

    /// Computes the critical path (longest path of tasks from root to leaf).
    pub fn critical_path(&self) -> Vec<String> {
        let topo = match self.topological_sort() {
            Ok(t) => t,
            Err(_) => return Vec::new(),
        };

        let mut dist: HashMap<String, usize> = HashMap::new();
        let mut prev: HashMap<String, Option<String>> = HashMap::new();

        for id in &topo {
            dist.insert(id.clone(), 1);
            prev.insert(id.clone(), None);
        }

        for id in &topo {
            let cur_dist = dist[id];
            let task = &self.tasks[id];
            for dep in &task.dependencies {
                let candidate_dist = dist[dep] + 1;
                if candidate_dist > cur_dist {
                    dist.insert(id.clone(), candidate_dist);
                    prev.insert(id.clone(), Some(dep.clone()));
                }
            }
        }

        // Find max terminal node
        let leaf_candidates = self.leaf_tasks();
        let max_leaf = leaf_candidates
            .into_iter()
            .max_by_key(|t| dist.get(&t.id).copied().unwrap_or(0));

        let mut path = Vec::new();
        if let Some(leaf) = max_leaf {
            let mut curr = Some(leaf.id.clone());
            while let Some(node) = curr {
                path.push(node.clone());
                curr = prev.get(&node).cloned().flatten();
            }
            path.reverse();
        }

        path
    }

    /// Returns all root tasks (tasks with zero dependencies).
    pub fn root_tasks(&self) -> Vec<&DagTask> {
        self.tasks
            .values()
            .filter(|t| t.dependencies.is_empty())
            .collect()
    }

    /// Returns all leaf tasks (tasks that are not dependencies of any other task).
    pub fn leaf_tasks(&self) -> Vec<&DagTask> {
        let all_deps: HashSet<&str> = self
            .tasks
            .values()
            .flat_map(|t| t.dependencies.iter().map(|s| s.as_str()))
            .collect();

        self.tasks
            .values()
            .filter(|t| !all_deps.contains(t.id.as_str()))
            .collect()
    }

    /// Returns immediate child tasks that depend directly on `task_id`.
    pub fn direct_dependents(&self, task_id: &str) -> Vec<String> {
        self.tasks
            .values()
            .filter(|t| t.dependencies.iter().any(|dep| dep == task_id))
            .map(|t| t.id.clone())
            .collect()
    }

    /// Returns all downstream descendant task IDs of `task_id` recursively.
    pub fn all_descendants(&self, task_id: &str) -> HashSet<String> {
        let mut descendants = HashSet::new();
        let mut queue = VecDeque::new();
        queue.push_back(task_id.to_string());

        while let Some(curr) = queue.pop_front() {
            for child in self.direct_dependents(&curr) {
                if descendants.insert(child.clone()) {
                    queue.push_back(child);
                }
            }
        }

        descendants
    }

    /// Returns all upstream ancestor task IDs of `task_id` recursively.
    pub fn all_ancestors(&self, task_id: &str) -> HashSet<String> {
        let mut ancestors = HashSet::new();
        let mut queue = VecDeque::new();
        queue.push_back(task_id.to_string());

        while let Some(curr) = queue.pop_front() {
            if let Some(task) = self.tasks.get(&curr) {
                for dep in &task.dependencies {
                    if ancestors.insert(dep.clone()) {
                        queue.push_back(dep.clone());
                    }
                }
            }
        }

        ancestors
    }

    /// Returns IDs of tasks in `Pending` status whose dependencies have all completed.
    pub fn ready_tasks(&self) -> Vec<String> {
        let mut ready = Vec::new();
        for (id, task) in &self.tasks {
            if task.status.is_pending() {
                let all_deps_completed = task.dependencies.iter().all(|dep_id| {
                    self.tasks
                        .get(dep_id)
                        .map(|t| t.status.is_completed())
                        .unwrap_or(false)
                });
                if all_deps_completed {
                    ready.push(id.clone());
                }
            }
        }

        // Sort by priority descending
        ready.sort_by(|a, b| {
            let prio_a = self.tasks.get(a).map(|t| t.priority).unwrap_or_default();
            let prio_b = self.tasks.get(b).map(|t| t.priority).unwrap_or_default();
            prio_b.cmp(&prio_a).then_with(|| a.cmp(b))
        });

        ready
    }

    /// Marks a task as completed with output.
    pub fn mark_task_completed(&mut self, task_id: &str, output: String, duration_ms: u64) {
        if let Some(task) = self.tasks.get_mut(task_id) {
            let now = Utc::now().to_rfc3339();
            task.result = Some(output.clone());
            task.finished_at = Some(now.clone());
            task.duration_ms = Some(duration_ms);
            task.status = DagTaskStatus::Completed {
                output,
                finished_at: now,
                duration_ms,
            };
        }
    }

    /// Marks a task as failed with error.
    pub fn mark_task_failed(&mut self, task_id: &str, error: String, retry_count: usize) {
        if let Some(task) = self.tasks.get_mut(task_id) {
            let now = Utc::now().to_rfc3339();
            task.error = Some(error.clone());
            task.finished_at = Some(now.clone());
            task.retry_count = retry_count;
            task.status = DagTaskStatus::Failed {
                error,
                failed_at: now,
                retry_count,
            };
        }
    }

    /// Marks a task as skipped.
    pub fn mark_task_skipped(&mut self, task_id: &str, reason: String) {
        if let Some(task) = self.tasks.get_mut(task_id) {
            task.status = DagTaskStatus::Skipped { reason };
        }
    }

    /// Returns the overall lifecycle status of the DAG.
    pub fn overall_status(&self) -> DagOverallStatus {
        if self.tasks.is_empty() {
            return DagOverallStatus::NotStarted;
        }

        let mut completed = 0;
        let mut failed = 0;
        let mut skipped = 0;
        let mut running = 0;
        let mut pending_or_ready = 0;

        for task in self.tasks.values() {
            match &task.status {
                DagTaskStatus::Completed { .. } => completed += 1,
                DagTaskStatus::Failed { .. } => failed += 1,
                DagTaskStatus::Skipped { .. } => skipped += 1,
                DagTaskStatus::Running { .. } => running += 1,
                DagTaskStatus::Pending | DagTaskStatus::Ready | DagTaskStatus::Blocked { .. } => {
                    pending_or_ready += 1
                }
            }
        }

        if completed == self.tasks.len() {
            DagOverallStatus::Completed
        } else if running > 0 || (completed > 0 && pending_or_ready > 0) {
            DagOverallStatus::InProgress
        } else if failed > 0 && pending_or_ready == 0 && running == 0 {
            if completed > 0 {
                DagOverallStatus::PartiallyCompleted
            } else {
                DagOverallStatus::Failed
            }
        } else if completed + skipped == self.tasks.len() && completed > 0 {
            DagOverallStatus::Completed
        } else if completed == 0 && failed == 0 && running == 0 && skipped == 0 {
            DagOverallStatus::NotStarted
        } else {
            DagOverallStatus::InProgress
        }
    }

    /// Renders the DAG as a Mermaid flowchart string.
    pub fn to_mermaid(&self) -> String {
        let mut out = String::from("```mermaid\ngraph TD\n");

        // Styling class definitions
        out.push_str("    classDef pending fill:#e2e8f0,stroke:#94a3b8,color:#334155;\n");
        out.push_str("    classDef ready fill:#dbeafe,stroke:#3b82f6,color:#1e3a8a;\n");
        out.push_str("    classDef running fill:#fef3c7,stroke:#f59e0b,color:#78350f;\n");
        out.push_str("    classDef completed fill:#dcfce7,stroke:#22c55e,color:#14532d;\n");
        out.push_str("    classDef failed fill:#fee2e2,stroke:#ef4444,color:#7f1d1d;\n");
        out.push_str("    classDef skipped fill:#f3f4f6,stroke:#6b7280,color:#374151;\n\n");

        // Nodes by stage subgraphs if stages are computed
        if !self.stages.is_empty() {
            for stage in &self.stages {
                out.push_str(&format!(
                    "    subgraph {}[\"{}\"]\n",
                    format!("stage_{}", stage.stage_index),
                    stage.name
                ));
                for task_id in &stage.task_ids {
                    if let Some(task) = self.tasks.get(task_id) {
                        let safe_title = task.title.replace('"', "'");
                        let role_tag = format!("[{}]", task.role);
                        out.push_str(&format!(
                            "        {}[\"<b>{}</b><br/>{} {}\"]\n",
                            task.id, task.id, role_tag, safe_title
                        ));
                    }
                }
                out.push_str("    end\n\n");
            }
        } else {
            for task in self.tasks.values() {
                let safe_title = task.title.replace('"', "'");
                let role_tag = format!("[{}]", task.role);
                out.push_str(&format!(
                    "    {}[\"<b>{}</b><br/>{} {}\"]\n",
                    task.id, task.id, role_tag, safe_title
                ));
            }
        }

        // Edges
        out.push('\n');
        for (id, task) in &self.tasks {
            for dep in &task.dependencies {
                out.push_str(&format!("    {} --> {}\n", dep, id));
            }
        }

        // Apply classes
        out.push('\n');
        for (id, task) in &self.tasks {
            let class_name = match &task.status {
                DagTaskStatus::Pending => "pending",
                DagTaskStatus::Ready => "ready",
                DagTaskStatus::Running { .. } => "running",
                DagTaskStatus::Completed { .. } => "completed",
                DagTaskStatus::Failed { .. } => "failed",
                DagTaskStatus::Skipped { .. } | DagTaskStatus::Blocked { .. } => "skipped",
            };
            out.push_str(&format!("    class {} {};\n", id, class_name));
        }

        out.push_str("```\n");
        out
    }

    /// Renders a hierarchical ASCII tree / stage timeline of the DAG.
    pub fn to_ascii_tree(&self) -> String {
        let mut out = String::new();
        out.push_str(&format!("DAG Plan: {} (ID: {})\n", self.name, self.id));
        out.push_str(&format!("Goal: {}\n", self.goal));
        out.push_str(&format!("Overall Status: {}\n", self.overall_status()));
        out.push_str(&"=".repeat(60));
        out.push('\n');

        if !self.stages.is_empty() {
            for stage in &self.stages {
                out.push_str(&format!("\n▶ {} [{}]\n", stage.name, stage.status));
                for (i, task_id) in stage.task_ids.iter().enumerate() {
                    let is_last = i == stage.task_ids.len() - 1;
                    let branch = if is_last { "└── " } else { "├── " };

                    if let Some(task) = self.tasks.get(task_id) {
                        let dep_str = if task.dependencies.is_empty() {
                            String::from("root")
                        } else {
                            format!("after: {}", task.dependencies.join(", "))
                        };
                        let duration_str = task
                            .duration_ms
                            .map(|d| format!(" ({}ms)", d))
                            .unwrap_or_default();
                        out.push_str(&format!(
                            "  {}{} [{}] {}: {} ({}){}\n",
                            branch,
                            task.status.status_symbol(),
                            task.role,
                            task.id,
                            task.title,
                            dep_str,
                            duration_str
                        ));
                    }
                }
            }
        } else {
            for task in self.tasks.values() {
                let dep_str = if task.dependencies.is_empty() {
                    String::from("root")
                } else {
                    format!("after: {}", task.dependencies.join(", "))
                };
                out.push_str(&format!(
                    "  {} [{}] {}: {} ({})\n",
                    task.status.status_symbol(),
                    task.role,
                    task.id,
                    task.title,
                    dep_str
                ));
            }
        }

        out
    }

    /// Renders the DAG in Graphviz DOT format.
    pub fn to_dot(&self) -> String {
        let mut out = String::from("digraph SubagentDAG {\n");
        out.push_str("    rankdir=TB;\n");
        out.push_str("    node [shape=box, style=filled, fontname=\"Helvetica\"];\n\n");

        for task in self.tasks.values() {
            let color = match &task.status {
                DagTaskStatus::Pending => "#e2e8f0",
                DagTaskStatus::Ready => "#dbeafe",
                DagTaskStatus::Running { .. } => "#fef3c7",
                DagTaskStatus::Completed { .. } => "#dcfce7",
                DagTaskStatus::Failed { .. } => "#fee2e2",
                DagTaskStatus::Skipped { .. } | DagTaskStatus::Blocked { .. } => "#f3f4f6",
            };
            out.push_str(&format!(
                "    \"{}\" [label=\"{}\\n[{}] {}\", fillcolor=\"{}\"];\n",
                task.id, task.id, task.role, task.title, color
            ));
        }

        out.push('\n');
        for (id, task) in &self.tasks {
            for dep in &task.dependencies {
                out.push_str(&format!("    \"{}\" -> \"{}\";\n", dep, id));
            }
        }

        out.push_str("}\n");
        out
    }

    /// Serializes the DAG to a JSON string.
    pub fn to_json(&self) -> Result<String, DagPlannerError> {
        serde_json::to_string_pretty(self).map_err(|e| DagPlannerError::JsonParseError {
            error: e.to_string(),
        })
    }

    /// Deserializes a DAG from a JSON string.
    pub fn from_json(json_str: &str) -> Result<Self, DagPlannerError> {
        let dag: SubagentDag =
            serde_json::from_str(json_str).map_err(|e| DagPlannerError::JsonParseError {
                error: e.to_string(),
            })?;
        dag.validate()?;
        Ok(dag)
    }
}

/// Predefined task decomposition archetypes and strategies.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum DecompositionStrategy {
    /// Comprehensive feature implementation: Exploration -> Parallel Dev -> Testing -> Review.
    SoftwareFeature {
        split_frontend_backend: bool,
        include_security_audit: bool,
    },
    /// Targeted bug fix: Root cause reproduction -> Minimal fix -> Regression verification -> Audit.
    BugFix { deep_reproduction: bool },
    /// Structural refactoring: Dependency mapping -> Baseline tests -> Parallel refactoring -> Full verification.
    Refactoring { parallel_modules: bool },
    /// Multi-agent security & vulnerability audit: Surface mapping -> Parallel domain reviews -> Remediation plan.
    SecurityAudit { depth: usize },
    /// Performance tuning: Profiling & bottleneck analysis -> Parallel optimizations -> Benchmark verification.
    PerformanceOptimization,
    /// Fully custom pipeline.
    Custom { name: String },
}

impl Default for DecompositionStrategy {
    fn default() -> Self {
        DecompositionStrategy::SoftwareFeature {
            split_frontend_backend: true,
            include_security_audit: true,
        }
    }
}

/// High-level task decomposition engine that transforms a complex prompt or goal into a validated Subagent DAG.
pub struct TaskDecomposer;

impl TaskDecomposer {
    /// Decomposes a user goal into a complete, validated Subagent DAG using heuristic domain strategies.
    pub fn decompose(
        goal: &str,
        strategy: DecompositionStrategy,
    ) -> Result<SubagentDag, DagPlannerError> {
        let mut dag = SubagentDag::new(
            format!("Plan: {}", goal.lines().next().unwrap_or("Task").trim()),
            goal,
        );

        match strategy {
            DecompositionStrategy::SoftwareFeature {
                split_frontend_backend,
                include_security_audit,
            } => {
                // Stage 1: Exploration & Codebase Discovery
                dag.add_task(
                    DagTask::new(
                        "scout_arch",
                        "Architecture & Codebase Discovery",
                        SubagentRole::Scout,
                        format!(
                            "Analyze codebase architecture and locate all relevant files, interfaces, and integration points for:\n{}",
                            goal
                        ),
                    )
                    .with_priority(DagTaskPriority::High)
                    .with_tag("scout"),
                )?;

                // Stage 2: Parallel Implementation
                if split_frontend_backend {
                    dag.add_task(
                        DagTask::new(
                            "coder_core",
                            "Core Logic & Backend Implementation",
                            SubagentRole::Coder,
                            format!(
                                "Implement core domain types, business logic, data models, and backend logic for:\n{}\n\nIncorporate discoveries from architecture scout.",
                                goal
                            ),
                        )
                        .with_dependency("scout_arch")
                        .with_priority(DagTaskPriority::High)
                        .with_tag("backend"),
                    )?;

                    dag.add_task(
                        DagTask::new(
                            "coder_interface",
                            "Interface, CLI & UI Integration",
                            SubagentRole::Coder,
                            format!(
                                "Implement user interface, command-line arguments, API endpoints, and display formatting for:\n{}\n\nIntegrate cleanly with core domain logic.",
                                goal
                            ),
                        )
                        .with_dependency("scout_arch")
                        .with_priority(DagTaskPriority::Normal)
                        .with_tag("frontend"),
                    )?;
                } else {
                    dag.add_task(
                        DagTask::new(
                            "coder_feature",
                            "Feature Implementation",
                            SubagentRole::Coder,
                            format!(
                                "Implement complete feature functionality cleanly following project conventions:\n{}",
                                goal
                            ),
                        )
                        .with_dependency("scout_arch")
                        .with_priority(DagTaskPriority::High)
                        .with_tag("feature"),
                    )?;
                }

                // Stage 3: Parallel Verification & Testing
                let impl_deps = if split_frontend_backend {
                    vec!["coder_core".to_string(), "coder_interface".to_string()]
                } else {
                    vec!["coder_feature".to_string()]
                };

                dag.add_task(
                    DagTask::new(
                        "tester_unit",
                        "Unit & Behavioral Testing",
                        SubagentRole::Tester,
                        format!(
                            "Write and execute comprehensive unit and behavioral tests for:\n{}\n\nVerify all edge cases, error handling, and core paths.",
                            goal
                        ),
                    )
                    .with_dependencies(impl_deps.clone())
                    .with_priority(DagTaskPriority::High)
                    .with_tag("test"),
                )?;

                dag.add_task(
                    DagTask::new(
                        "tester_integration",
                        "Integration & Smoke Verification",
                        SubagentRole::Tester,
                        format!(
                            "Verify end-to-end integration and smoke test the entire workflow for:\n{}",
                            goal
                        ),
                    )
                    .with_dependencies(impl_deps.clone())
                    .with_priority(DagTaskPriority::Normal)
                    .with_tag("test"),
                )?;

                // Stage 4: Review & Security
                let mut review_deps = vec!["tester_unit".to_string(), "tester_integration".to_string()];
                dag.add_task(
                    DagTask::new(
                        "reviewer_quality",
                        "Code Quality & Architecture Review",
                        SubagentRole::Reviewer,
                        format!(
                            "Thoroughly review code modifications for cleanliness, idiomatic Rust patterns, maintainability, and architectural integrity for:\n{}",
                            goal
                        ),
                    )
                    .with_dependencies(review_deps.clone())
                    .with_priority(DagTaskPriority::Normal)
                    .with_tag("review"),
                )?;

                if include_security_audit {
                    dag.add_task(
                        DagTask::new(
                            "reviewer_security",
                            "Security & Vulnerability Audit",
                            SubagentRole::Reviewer,
                            format!(
                                "Audit changes for memory safety, input sanitization, error leakage, boundary conditions, and cross-platform safety for:\n{}",
                                goal
                            ),
                        )
                        .with_dependencies(review_deps)
                        .with_priority(DagTaskPriority::High)
                        .with_tag("security"),
                    )?;
                }
            }

            DecompositionStrategy::BugFix { deep_reproduction } => {
                // Stage 1: Root Cause & Reproduction
                let scout_desc = if deep_reproduction {
                    format!(
                        "Deeply inspect logs, stack traces, and codebase to construct a minimal reproduction case and isolate the exact root cause for:\n{}",
                        goal
                    )
                } else {
                    format!(
                        "Inspect code to locate bug origin and root cause for:\n{}",
                        goal
                    )
                };

                dag.add_task(
                    DagTask::new(
                        "scout_repro",
                        "Root Cause & Bug Isolation",
                        SubagentRole::Scout,
                        scout_desc,
                    )
                    .with_priority(DagTaskPriority::Critical)
                    .with_tag("bugfix"),
                )?;

                // Stage 2: Minimal Targeted Fix
                dag.add_task(
                    DagTask::new(
                        "coder_fix",
                        "Targeted Bug Fix Implementation",
                        SubagentRole::Coder,
                        format!(
                            "Implement minimal, surgical bug fix addressing the isolated root cause without regressing existing behavior for:\n{}",
                            goal
                        ),
                    )
                    .with_dependency("scout_repro")
                    .with_priority(DagTaskPriority::Critical)
                    .with_tag("bugfix"),
                )?;

                // Stage 3: Regression Test Verification
                dag.add_task(
                    DagTask::new(
                        "tester_verify",
                        "Regression Test & Verification",
                        SubagentRole::Tester,
                        format!(
                            "Add regression tests preventing the bug from reoccurring and verify the full test suite passes for:\n{}",
                            goal
                        ),
                    )
                    .with_dependency("coder_fix")
                    .with_priority(DagTaskPriority::High)
                    .with_tag("test"),
                )?;

                // Stage 4: Review
                dag.add_task(
                    DagTask::new(
                        "reviewer_audit",
                        "Fix Safety & Side-Effect Review",
                        SubagentRole::Reviewer,
                        format!(
                            "Review bug fix for potential side effects, unexpected edge cases, and ensure clean adherence to invariants for:\n{}",
                            goal
                        ),
                    )
                    .with_dependency("tester_verify")
                    .with_priority(DagTaskPriority::Normal)
                    .with_tag("review"),
                )?;
            }

            DecompositionStrategy::Refactoring { parallel_modules } => {
                // Stage 1: Dependency Mapping
                dag.add_task(
                    DagTask::new(
                        "scout_deps",
                        "Dependency & Reference Mapping",
                        SubagentRole::Scout,
                        format!(
                            "Map all call sites, type dependencies, and public API surface before refactoring:\n{}",
                            goal
                        ),
                    )
                    .with_priority(DagTaskPriority::High)
                    .with_tag("refactor"),
                )?;

                // Stage 2: Baseline Tests
                dag.add_task(
                    DagTask::new(
                        "tester_baseline",
                        "Establish Test Baseline",
                        SubagentRole::Tester,
                        "Run existing test suite to ensure clean baseline before modifications begin.",
                    )
                    .with_dependency("scout_deps")
                    .with_priority(DagTaskPriority::Normal)
                    .with_tag("test"),
                )?;

                // Stage 3: Refactoring Execution
                if parallel_modules {
                    dag.add_task(
                        DagTask::new(
                            "coder_refactor_core",
                            "Core Logic & Model Refactor",
                            SubagentRole::Coder,
                            format!(
                                "Refactor core modules, simplifying abstractions and reducing duplication for:\n{}",
                                goal
                            ),
                        )
                        .with_dependency("tester_baseline")
                        .with_priority(DagTaskPriority::High)
                        .with_tag("refactor"),
                    )?;

                    dag.add_task(
                        DagTask::new(
                            "coder_refactor_callers",
                            "Callers & Interface Cutover",
                            SubagentRole::Coder,
                            format!(
                                "Update all downstream callers, tests, and interfaces to cleanly cut over to new refactored structures for:\n{}",
                                goal
                            ),
                        )
                        .with_dependency("tester_baseline")
                        .with_priority(DagTaskPriority::Normal)
                        .with_tag("refactor"),
                    )?;

                    // Stage 4: Test Suite Verification
                    dag.add_task(
                        DagTask::new(
                            "tester_full",
                            "Full Regression & Performance Check",
                            SubagentRole::Tester,
                            "Execute full test suite to guarantee behavior is preserved with zero regressions.",
                        )
                        .with_dependencies(vec![
                            "coder_refactor_core".to_string(),
                            "coder_refactor_callers".to_string(),
                        ])
                        .with_priority(DagTaskPriority::High)
                        .with_tag("test"),
                    )?;
                } else {
                    dag.add_task(
                        DagTask::new(
                            "coder_refactor",
                            "Unified Codebase Refactor",
                            SubagentRole::Coder,
                            format!(
                                "Execute unified refactoring preserving all external behaviors for:\n{}",
                                goal
                            ),
                        )
                        .with_dependency("tester_baseline")
                        .with_priority(DagTaskPriority::High)
                        .with_tag("refactor"),
                    )?;

                    dag.add_task(
                        DagTask::new(
                            "tester_full",
                            "Full Regression & Invariant Verification",
                            SubagentRole::Tester,
                            "Execute full test suite to guarantee behavior preservation.",
                        )
                        .with_dependency("coder_refactor")
                        .with_priority(DagTaskPriority::High)
                        .with_tag("test"),
                    )?;
                }

                // Stage 5: Final Cleanliness Review
                dag.add_task(
                    DagTask::new(
                        "reviewer_cleanliness",
                        "Refactoring Cleanliness Audit",
                        SubagentRole::Reviewer,
                        "Verify that all dead code and deprecated paths were cleanly eliminated.",
                    )
                    .with_dependency("tester_full")
                    .with_priority(DagTaskPriority::Normal)
                    .with_tag("review"),
                )?;
            }

            DecompositionStrategy::SecurityAudit { depth: _ } => {
                // Stage 1: Attack Surface Mapping
                dag.add_task(
                    DagTask::new(
                        "scout_surface",
                        "Attack Surface & Boundary Mapping",
                        SubagentRole::Scout,
                        format!(
                            "Catalog all external entry points, unvalidated input boundaries, and authentication checkpoints for:\n{}",
                            goal
                        ),
                    )
                    .with_priority(DagTaskPriority::Critical)
                    .with_tag("security"),
                )?;

                // Stage 2: Parallel Domain Audits
                dag.add_task(
                    DagTask::new(
                        "reviewer_auth",
                        "Authentication & Access Control Audit",
                        SubagentRole::Reviewer,
                        "Audit access control, session management, token handling, and authorization boundaries.",
                    )
                    .with_dependency("scout_surface")
                    .with_priority(DagTaskPriority::Critical)
                    .with_tag("security"),
                )?;

                dag.add_task(
                    DagTask::new(
                        "reviewer_input",
                        "Input Validation & Injection Audit",
                        SubagentRole::Reviewer,
                        "Audit for command injection, path traversal, buffer overflows, and unsafe deserialization.",
                    )
                    .with_dependency("scout_surface")
                    .with_priority(DagTaskPriority::High)
                    .with_tag("security"),
                )?;

                dag.add_task(
                    DagTask::new(
                        "reviewer_secrets",
                        "Secrets & Cryptography Review",
                        SubagentRole::Reviewer,
                        "Audit hardcoded secrets, key storage, cipher parameters, and sensitive data leakage.",
                    )
                    .with_dependency("scout_surface")
                    .with_priority(DagTaskPriority::High)
                    .with_tag("security"),
                )?;

                // Stage 3: Consolidated Remediation Plan
                dag.add_task(
                    DagTask::new(
                        "general_remediation",
                        "Consolidated Risk Matrix & Remediation Roadmap",
                        SubagentRole::General,
                        "Synthesize all domain review findings into prioritized vulnerability matrix with actionable remediation patches.",
                    )
                    .with_dependencies(vec![
                        "reviewer_auth".to_string(),
                        "reviewer_input".to_string(),
                        "reviewer_secrets".to_string(),
                    ])
                    .with_priority(DagTaskPriority::Critical)
                    .with_tag("security"),
                )?;
            }

            DecompositionStrategy::PerformanceOptimization => {
                // Stage 1: Profiling
                dag.add_task(
                    DagTask::new(
                        "scout_profile",
                        "Hotpath Profiling & Bottleneck Identification",
                        SubagentRole::Scout,
                        format!(
                            "Identify hot loops, redundant allocations, unindexed queries, and locks for:\n{}",
                            goal
                        ),
                    )
                    .with_priority(DagTaskPriority::High)
                    .with_tag("perf"),
                )?;

                // Stage 2: Parallel Optimizations
                dag.add_task(
                    DagTask::new(
                        "coder_opt_alloc",
                        "Memory & Allocation Optimization",
                        SubagentRole::Coder,
                        "Reduce heap allocations, leverage zero-copy slices, and optimize buffer reuse.",
                    )
                    .with_dependency("scout_profile")
                    .with_priority(DagTaskPriority::High)
                    .with_tag("perf"),
                )?;

                dag.add_task(
                    DagTask::new(
                        "coder_opt_algo",
                        "Algorithmic & Concurrency Tuning",
                        SubagentRole::Coder,
                        "Optimize algorithm complexity, batch I/O, and fine-tune lock contention.",
                    )
                    .with_dependency("scout_profile")
                    .with_priority(DagTaskPriority::High)
                    .with_tag("perf"),
                )?;

                // Stage 3: Benchmark Verification
                dag.add_task(
                    DagTask::new(
                        "tester_benchmark",
                        "Benchmark Comparison & Correctness",
                        SubagentRole::Tester,
                        "Run comparative benchmarks against baseline and verify zero functional regressions.",
                    )
                    .with_dependencies(vec![
                        "coder_opt_alloc".to_string(),
                        "coder_opt_algo".to_string(),
                    ])
                    .with_priority(DagTaskPriority::High)
                    .with_tag("test"),
                )?;

                // Stage 4: Code Review
                dag.add_task(
                    DagTask::new(
                        "reviewer_safety",
                        "Optimization Safety & Readability Audit",
                        SubagentRole::Reviewer,
                        "Ensure optimizations maintain readability, safety invariants, and maintainability.",
                    )
                    .with_dependency("tester_benchmark")
                    .with_priority(DagTaskPriority::Normal)
                    .with_tag("review"),
                )?;
            }

            DecompositionStrategy::Custom { ref name } => {
                dag.add_task(
                    DagTask::new(
                        "task_main",
                        name.as_str(),
                        SubagentRole::General,
                        goal,
                    )
                    .with_priority(DagTaskPriority::Normal),
                )?;
            }
        }

        dag.compute_stages()?;
        Ok(dag)
    }

    /// Generates a structured prompt to instruct an LLM to decompose a complex task into a DAG.
    pub fn build_llm_decomposition_prompt(goal: &str, codebase_context: Option<&str>) -> String {
        let mut prompt = String::new();
        prompt.push_str("You are an expert AI DAG planning engine.\n");
        prompt.push_str("Decompose the following high-level engineering goal into a Directed Acyclic Graph (DAG) of specialized subagent tasks.\n\n");
        prompt.push_str(&format!("### Goal:\n{}\n\n", goal));

        if let Some(ctx) = codebase_context {
            prompt.push_str(&format!("### Codebase Context:\n{}\n\n", ctx));
        }

        prompt.push_str("### Rules & Guidelines:\n");
        prompt.push_str("1. Roles must be one of: 'scout' (read-only discovery), 'coder' (implementation), 'tester' (test & verify), 'reviewer' (audit/security), 'general'.\n");
        prompt.push_str("2. Maximize safe parallelism: tasks that do not depend on each other MUST have empty or shared dependencies so they execute in the same stage wave.\n");
        prompt.push_str("3. Dependencies must refer only to valid task IDs defined in the plan.\n");
        prompt.push_str("4. The graph MUST be acyclic (no cycles).\n");
        prompt.push_str("5. Priority can be 'low', 'normal', 'high', 'critical'.\n\n");

        prompt.push_str("### Required JSON Output Format:\n");
        prompt.push_str("```json\n");
        prompt.push_str("{\n");
        prompt.push_str("  \"name\": \"Short Plan Title\",\n");
        prompt.push_str("  \"tasks\": [\n");
        prompt.push_str("    {\n");
        prompt.push_str("      \"id\": \"scout_discovery\",\n");
        prompt.push_str("      \"title\": \"Discover Codebase Patterns\",\n");
        prompt.push_str("      \"role\": \"scout\",\n");
        prompt.push_str("      \"description\": \"Detailed task prompt...\",\n");
        prompt.push_str("      \"dependencies\": [],\n");
        prompt.push_str("      \"priority\": \"high\",\n");
        prompt.push_str("      \"tags\": [\"scout\", \"discovery\"]\n");
        prompt.push_str("    }\n");
        prompt.push_str("  ]\n");
        prompt.push_str("}\n");
        prompt.push_str("```\n");

        prompt
    }

    /// Parses an LLM JSON output response into a validated Subagent DAG.
    /// Handles markdown code block stripping, cycle repair, and automatic stage calculation.
    pub fn parse_llm_decomposition(
        json_response: &str,
        goal: &str,
    ) -> Result<SubagentDag, DagPlannerError> {
        let clean_json = extract_json_block(json_response);
        let parsed: Value =
            serde_json::from_str(&clean_json).map_err(|e| DagPlannerError::JsonParseError {
                error: format!("Failed to parse JSON: {}", e),
            })?;

        let name = parsed
            .get("name")
            .and_then(|v| v.as_str())
            .unwrap_or("Dynamic Subagent Plan");

        let mut dag = SubagentDag::new(name, goal);

        let tasks_arr = parsed
            .get("tasks")
            .and_then(|v| v.as_array())
            .ok_or_else(|| DagPlannerError::JsonParseError {
                error: "Missing 'tasks' array in JSON".to_string(),
            })?;

        if tasks_arr.is_empty() {
            return Err(DagPlannerError::EmptyDag);
        }

        for (idx, task_val) in tasks_arr.iter().enumerate() {
            let id = task_val
                .get("id")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string())
                .unwrap_or_else(|| format!("task_{}", idx + 1));

            let title = task_val
                .get("title")
                .and_then(|v| v.as_str())
                .unwrap_or("Task");

            let role_str = task_val
                .get("role")
                .and_then(|v| v.as_str())
                .unwrap_or("general");
            let role = SubagentRole::from_str(role_str).unwrap_or(SubagentRole::General);

            let desc = task_val
                .get("description")
                .and_then(|v| v.as_str())
                .unwrap_or(title);

            let priority_str = task_val
                .get("priority")
                .and_then(|v| v.as_str())
                .unwrap_or("normal");
            let priority =
                DagTaskPriority::from_str(priority_str).unwrap_or(DagTaskPriority::Normal);

            let mut deps = Vec::new();
            if let Some(deps_arr) = task_val.get("dependencies").and_then(|v| v.as_array()) {
                for d in deps_arr {
                    if let Some(dep_str) = d.as_str() {
                        deps.push(dep_str.to_string());
                    }
                }
            }

            let mut task = DagTask::new(id, title, role, desc)
                .with_dependencies(deps)
                .with_priority(priority);

            if let Some(tags_arr) = task_val.get("tags").and_then(|v| v.as_array()) {
                for t in tags_arr {
                    if let Some(tag_str) = t.as_str() {
                        task = task.with_tag(tag_str);
                    }
                }
            }

            dag.add_task(task)?;
        }

        // Validate and compute stages
        dag.compute_stages()?;
        Ok(dag)
    }
}

/// Helper function to strip Markdown ```json fences from LLM strings.
fn extract_json_block(text: &str) -> String {
    let trimmed = text.trim();
    if let Some(start) = trimmed.find("```json") {
        let content_start = start + 7;
        if let Some(end) = trimmed[content_start..].find("```") {
            return trimmed[content_start..content_start + end].trim().to_string();
        }
    } else if let Some(start) = trimmed.find("```") {
        let content_start = start + 3;
        if let Some(end) = trimmed[content_start..].find("```") {
            return trimmed[content_start..content_start + end].trim().to_string();
        }
    }
    trimmed.to_string()
}

/// Resolves dynamic context and template substitutions for a task by interpolating upstream outputs.
pub struct ContextResolver;

impl ContextResolver {
    /// Builds the final prompt for a task by interpolating upstream dependencies' outputs and metadata.
    pub fn resolve_task_prompt(task: &DagTask, dag: &SubagentDag, global_goal: &str) -> String {
        let mut prompt = String::new();

        // 1. Context header with global goal
        prompt.push_str(&format!("# Global Goal\n{}\n\n", global_goal));

        // 2. Upstream dependency context if applicable
        if !task.dependencies.is_empty() {
            prompt.push_str("# Upstream Task Findings & Context\n");
            for dep_id in &task.dependencies {
                if let Some(dep_task) = dag.get_task(dep_id) {
                    prompt.push_str(&format!("### [{}] {}\n", dep_task.role, dep_task.title));
                    if let Some(result) = &dep_task.result {
                        prompt.push_str(&format!("Output:\n{}\n\n", result.trim()));
                    } else if let Some(err) = &dep_task.error {
                        prompt.push_str(&format!("Status: Failed with error: {}\n\n", err));
                    } else {
                        prompt.push_str("Status: Completed (no output text)\n\n");
                    }
                }
            }
        }

        // 3. Main task instructions
        prompt.push_str("# Your Assigned Task\n");
        let mut task_desc = task.description.clone();

        // Template variable replacement: {{upstream.TASK_ID.output}}
        for dep_id in &task.dependencies {
            if let Some(dep_task) = dag.get_task(dep_id) {
                let out_val = dep_task.result.as_deref().unwrap_or("");
                let placeholder = format!("{{{{upstream.{}.output}}}}", dep_id);
                task_desc = task_desc.replace(&placeholder, out_val);
                let placeholder_res = format!("{{{{upstream.{}.result}}}}", dep_id);
                task_desc = task_desc.replace(&placeholder_res, out_val);
            }
        }

        task_desc = task_desc.replace("{{goal}}", global_goal);
        task_desc = task_desc.replace("{{task.title}}", &task.title);

        prompt.push_str(&task_desc);
        prompt
    }
}

/// Execution progress event emitted during DAG execution.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum DagExecutionEvent {
    DagStarted {
        dag_id: String,
        total_tasks: usize,
        total_stages: usize,
    },
    StageStarted {
        stage_index: usize,
        stage_name: String,
        task_ids: Vec<String>,
    },
    TaskStarted {
        task_id: String,
        role: SubagentRole,
        title: String,
    },
    TaskProgress {
        task_id: String,
        message: String,
    },
    TaskCompleted {
        task_id: String,
        output: String,
        duration_ms: u64,
    },
    TaskFailed {
        task_id: String,
        error: String,
        retry_count: usize,
        will_retry: bool,
    },
    TaskSkipped {
        task_id: String,
        reason: String,
    },
    StageCompleted {
        stage_index: usize,
        successful: usize,
        failed: usize,
        skipped: usize,
        duration_ms: u64,
    },
    DagCompleted {
        dag_id: String,
        total_duration_ms: u64,
        speedup_ratio: f64,
    },
    DagFailed {
        dag_id: String,
        reason: String,
    },
}

/// Execution configuration parameters for the DAG runner.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DagExecutionConfig {
    /// Maximum number of subagents allowed to execute concurrently across the fleet.
    pub max_concurrency: usize,
    /// If true, non-dependent branches continue executing even if a sibling task fails.
    pub continue_on_failure: bool,
    /// Default retry attempts for failed tasks.
    pub default_max_retries: usize,
    /// Overall execution timeout in seconds.
    pub global_timeout_secs: Option<u64>,
    /// If true, simulates task execution without invoking live LLMs.
    pub dry_run: bool,
}

impl Default for DagExecutionConfig {
    fn default() -> Self {
        Self {
            max_concurrency: 4,
            continue_on_failure: false,
            default_max_retries: 1,
            global_timeout_secs: None,
            dry_run: false,
        }
    }
}

/// Abstraction for executing individual subagent tasks.
#[async_trait]
pub trait TaskExecutor: Send + Sync {
    async fn execute_task(
        &self,
        task: &DagTask,
        resolved_prompt: &str,
    ) -> Result<String, String>;
}

/// Mock task executor for deterministic unit tests and simulated runs.
pub struct MockTaskExecutor {
    pub default_response: String,
    pub task_responses: HashMap<String, Result<String, String>>,
    pub simulated_delay: Duration,
}

impl MockTaskExecutor {
    pub fn new() -> Self {
        Self {
            default_response: "Mock task completed successfully.".to_string(),
            task_responses: HashMap::new(),
            simulated_delay: Duration::from_millis(1),
        }
    }

    pub fn with_task_result(mut self, task_id: impl Into<String>, result: Result<String, String>) -> Self {
        self.task_responses.insert(task_id.into(), result);
        self
    }
}

#[async_trait]
impl TaskExecutor for MockTaskExecutor {
    async fn execute_task(
        &self,
        task: &DagTask,
        _resolved_prompt: &str,
    ) -> Result<String, String> {
        if !self.simulated_delay.is_zero() {
            tokio::time::sleep(self.simulated_delay).await;
        }

        if let Some(res) = self.task_responses.get(&task.id) {
            res.clone()
        } else {
            Ok(format!("{}: {}", task.title, self.default_response))
        }
    }
}

/// SubagentManager task executor that dispatches real subagent tasks via `SubagentManager`.
pub struct SubagentManagerExecutor {
    pub manager: Arc<SubagentManager>,
}

impl SubagentManagerExecutor {
    pub fn new(manager: Arc<SubagentManager>) -> Self {
        Self { manager }
    }
}

#[async_trait]
impl TaskExecutor for SubagentManagerExecutor {
    async fn execute_task(
        &self,
        task: &DagTask,
        resolved_prompt: &str,
    ) -> Result<String, String> {
        let subagent_task = SubagentTask::new(task.role.clone(), resolved_prompt);
        let mut handle = self.manager.spawn(subagent_task);

        let res = handle
            .wait()
            .await
            .map_err(|e| format!("Subagent execution error on task '{}': {}", task.id, e))?;

        if res.success {
            Ok(res.output)
        } else {
            Err(res.output)
        }
    }
}

/// Comprehensive summary report produced upon DAG completion.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DagExecutionSummary {
    pub dag_id: String,
    pub dag_name: String,
    pub goal: String,
    pub overall_status: DagOverallStatus,
    pub total_tasks: usize,
    pub completed_tasks: usize,
    pub failed_tasks: usize,
    pub skipped_tasks: usize,
    pub total_stages: usize,
    pub wall_duration_ms: u64,
    pub cumulative_task_duration_ms: u64,
    pub speedup_ratio: f64,
    pub task_results: HashMap<String, DagTaskSummary>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DagTaskSummary {
    pub id: String,
    pub title: String,
    pub role: SubagentRole,
    pub status: String,
    pub duration_ms: Option<u64>,
    pub output_snippet: Option<String>,
    pub error: Option<String>,
}

impl DagExecutionSummary {
    /// Formats the summary into a readable markdown report.
    pub fn format_markdown_report(&self) -> String {
        let mut out = String::new();
        out.push_str(&format!("# Subagent DAG Execution Report: {}\n\n", self.dag_name));
        out.push_str(&format!("- **DAG ID:** `{}`\n", self.dag_id));
        out.push_str(&format!("- **Goal:** {}\n", self.goal));
        out.push_str(&format!("- **Status:** {}\n", self.overall_status));
        out.push_str(&format!(
            "- **Tasks:** {} completed, {} failed, {} skipped (Total: {})\n",
            self.completed_tasks, self.failed_tasks, self.skipped_tasks, self.total_tasks
        ));
        out.push_str(&format!("- **Total Stages (Waves):** {}\n", self.total_stages));
        out.push_str(&format!("- **Wall Clock Time:** {}ms\n", self.wall_duration_ms));
        out.push_str(&format!(
            "- **Cumulative Subagent Time:** {}ms\n",
            self.cumulative_task_duration_ms
        ));
        out.push_str(&format!(
            "- **Parallel Speedup Factor:** {:.2}x\n\n",
            self.speedup_ratio
        ));

        out.push_str("### Task Execution Details\n\n");
        out.push_str("| Task ID | Title | Role | Status | Duration | Result/Error |\n");
        out.push_str("|---------|-------|------|--------|----------|--------------|\n");

        let mut sorted_tasks: Vec<_> = self.task_results.values().collect();
        sorted_tasks.sort_by(|a, b| a.id.cmp(&b.id));

        for t in sorted_tasks {
            let dur = t.duration_ms.map(|d| format!("{}ms", d)).unwrap_or_else(|| "-".to_string());
            let outcome = if let Some(err) = &t.error {
                format!("Err: {}", err.replace('|', "\\|").lines().next().unwrap_or(""))
            } else if let Some(snip) = &t.output_snippet {
                snip.replace('|', "\\|").lines().next().unwrap_or("").to_string()
            } else {
                "-".to_string()
            };
            out.push_str(&format!(
                "| `{}` | {} | `{}` | {} | {} | {} |\n",
                t.id, t.title, t.role, t.status, dur, outcome
            ));
        }

        out
    }
}

/// The DAG Execution Engine coordinating concurrency, dependencies, stages, retries, and events.
pub struct DagExecutor {
    config: DagExecutionConfig,
}

impl DagExecutor {
    /// Creates a new DAG executor with the given configuration.
    pub fn new(config: DagExecutionConfig) -> Self {
        Self { config }
    }

    /// Executes the complete Subagent DAG to completion.
    pub async fn run(
        &self,
        dag: &mut SubagentDag,
        executor: Arc<dyn TaskExecutor>,
        event_tx: Option<mpsc::UnboundedSender<DagExecutionEvent>>,
    ) -> Result<DagExecutionSummary, DagPlannerError> {
        dag.compute_stages()?;
        let wall_start = Instant::now();

        if let Some(tx) = &event_tx {
            let _ = tx.send(DagExecutionEvent::DagStarted {
                dag_id: dag.id.clone(),
                total_tasks: dag.task_count(),
                total_stages: dag.stages.len(),
            });
        }

        let semaphore = Arc::new(Semaphore::new(self.config.max_concurrency.max(1)));
        let mut cumulative_task_duration_ms: u64 = 0;

        // Iterate through stages in topological wave order
        for stage_idx in 0..dag.stages.len() {
            let stage_start = Instant::now();
            let (stage_name, task_ids) = {
                let stg = &dag.stages[stage_idx];
                (stg.name.clone(), stg.task_ids.clone())
            };

            dag.stages[stage_idx].status = StageStatus::InProgress;
            dag.stages[stage_idx].started_at = Some(Utc::now().to_rfc3339());

            if let Some(tx) = &event_tx {
                let _ = tx.send(DagExecutionEvent::StageStarted {
                    stage_index: stage_idx,
                    stage_name: stage_name.clone(),
                    task_ids: task_ids.clone(),
                });
            }

            // Execute all tasks in the current stage concurrently up to semaphore limit
            let mut join_set = tokio::task::JoinSet::new();

            for task_id in &task_ids {
                let task = dag.get_task(task_id).unwrap().clone();

                // Check if any upstream dependency failed or was skipped
                let has_failed_dep = task.dependencies.iter().any(|dep_id| {
                    dag.get_task(dep_id)
                        .map(|t| t.status.is_failed() || t.status.is_skipped())
                        .unwrap_or(false)
                });

                if has_failed_dep {
                    // Skip task because dependency failed
                    dag.mark_task_skipped(
                        task_id,
                        "Skipped due to upstream prerequisite failure.".to_string(),
                    );
                    if let Some(tx) = &event_tx {
                        let _ = tx.send(DagExecutionEvent::TaskSkipped {
                            task_id: task_id.clone(),
                            reason: "Upstream prerequisite failed".to_string(),
                        });
                    }
                    continue;
                }

                // Resolve prompt with context
                let resolved_prompt = ContextResolver::resolve_task_prompt(&task, dag, &dag.goal);
                let task_executor = executor.clone();
                let sem = semaphore.clone();
                let tx_clone = event_tx.clone();
                let max_retries = task.max_retries.max(self.config.default_max_retries);
                let task_id_cloned = task_id.clone();
                let task_cloned = task.clone();

                join_set.spawn(async move {
                    let _permit = sem.acquire().await.unwrap();

                    if let Some(tx) = &tx_clone {
                        let _ = tx.send(DagExecutionEvent::TaskStarted {
                            task_id: task_id_cloned.clone(),
                            role: task_cloned.role.clone(),
                            title: task_cloned.title.clone(),
                        });
                    }

                    let t_start = Instant::now();
                    let mut attempts = 0;
                    let mut last_err = String::new();

                    while attempts <= max_retries {
                        match task_executor.execute_task(&task_cloned, &resolved_prompt).await {
                            Ok(output) => {
                                let dur = t_start.elapsed().as_millis() as u64;
                                return (task_id_cloned, Ok((output, dur)));
                            }
                            Err(e) => {
                                attempts += 1;
                                last_err = e.clone();
                                let will_retry = attempts <= max_retries;

                                if let Some(tx) = &tx_clone {
                                    let _ = tx.send(DagExecutionEvent::TaskFailed {
                                        task_id: task_id_cloned.clone(),
                                        error: e,
                                        retry_count: attempts,
                                        will_retry,
                                    });
                                }
                            }
                        }
                    }

                    let dur = t_start.elapsed().as_millis() as u64;
                    (task_id_cloned, Err((last_err, dur, attempts)))
                });
            }

            // Await all spawned tasks for this stage
            let mut stage_successful = 0;
            let mut stage_failed = 0;
            let mut stage_skipped = 0;

            while let Some(res) = join_set.join_next().await {
                match res {
                    Ok((task_id, Ok((output, duration_ms)))) => {
                        stage_successful += 1;
                        cumulative_task_duration_ms += duration_ms;
                        dag.mark_task_completed(&task_id, output.clone(), duration_ms);

                        if let Some(tx) = &event_tx {
                            let _ = tx.send(DagExecutionEvent::TaskCompleted {
                                task_id,
                                output,
                                duration_ms,
                            });
                        }
                    }
                    Ok((task_id, Err((error, duration_ms, retry_count)))) => {
                        stage_failed += 1;
                        cumulative_task_duration_ms += duration_ms;
                        dag.mark_task_failed(&task_id, error, retry_count);
                    }
                    Err(join_err) => {
                        stage_failed += 1;
                        eprintln!("DAG Task Join Error: {}", join_err);
                    }
                }
            }

            // Count skipped tasks in this stage
            for tid in &task_ids {
                if let Some(t) = dag.get_task(tid) {
                    if t.status.is_skipped() {
                        stage_skipped += 1;
                    }
                }
            }

            let stage_dur = stage_start.elapsed().as_millis() as u64;
            dag.stages[stage_idx].duration_ms = Some(stage_dur);
            dag.stages[stage_idx].finished_at = Some(Utc::now().to_rfc3339());

            if stage_failed > 0 {
                if stage_successful > 0 {
                    dag.stages[stage_idx].status = StageStatus::PartialSuccess;
                } else {
                    dag.stages[stage_idx].status = StageStatus::Failed;
                }

                if !self.config.continue_on_failure {
                    // Abort subsequent stages and cascade skips
                    for rem_idx in (stage_idx + 1)..dag.stages.len() {
                        dag.stages[rem_idx].status = StageStatus::Skipped;
                        let rem_ids: Vec<String> = dag.stages[rem_idx].task_ids.clone();
                        for rem_tid in &rem_ids {
                            dag.mark_task_skipped(
                                rem_tid,
                                "Aborted due to preceding stage failure.".to_string(),
                            );
                        }
                    }

                    if let Some(tx) = &event_tx {
                        let _ = tx.send(DagExecutionEvent::DagFailed {
                            dag_id: dag.id.clone(),
                            reason: format!(
                                "Stage #{} ('{}') encountered failures in strict mode.",
                                stage_idx + 1,
                                stage_name
                            ),
                        });
                    }
                    break;
                }
            } else {
                dag.stages[stage_idx].status = StageStatus::Completed;
            }

            if let Some(tx) = &event_tx {
                let _ = tx.send(DagExecutionEvent::StageCompleted {
                    stage_index: stage_idx,
                    successful: stage_successful,
                    failed: stage_failed,
                    skipped: stage_skipped,
                    duration_ms: stage_dur,
                });
            }
        }

        let wall_duration_ms = wall_start.elapsed().as_millis() as u64;
        let speedup_ratio = if wall_duration_ms > 0 {
            (cumulative_task_duration_ms as f64) / (wall_duration_ms as f64)
        } else {
            1.0
        };

        let overall = dag.overall_status();
        let mut completed_tasks = 0;
        let mut failed_tasks = 0;
        let mut skipped_tasks = 0;
        let mut task_results = HashMap::new();

        for (id, task) in &dag.tasks {
            match &task.status {
                DagTaskStatus::Completed { output, .. } => {
                    completed_tasks += 1;
                    task_results.insert(
                        id.clone(),
                        DagTaskSummary {
                            id: id.clone(),
                            title: task.title.clone(),
                            role: task.role.clone(),
                            status: "Completed".to_string(),
                            duration_ms: task.duration_ms,
                            output_snippet: Some(output.chars().take(80).collect()),
                            error: None,
                        },
                    );
                }
                DagTaskStatus::Failed { error, .. } => {
                    failed_tasks += 1;
                    task_results.insert(
                        id.clone(),
                        DagTaskSummary {
                            id: id.clone(),
                            title: task.title.clone(),
                            role: task.role.clone(),
                            status: "Failed".to_string(),
                            duration_ms: task.duration_ms,
                            output_snippet: None,
                            error: Some(error.clone()),
                        },
                    );
                }
                DagTaskStatus::Skipped { reason } => {
                    skipped_tasks += 1;
                    task_results.insert(
                        id.clone(),
                        DagTaskSummary {
                            id: id.clone(),
                            title: task.title.clone(),
                            role: task.role.clone(),
                            status: "Skipped".to_string(),
                            duration_ms: None,
                            output_snippet: None,
                            error: Some(reason.clone()),
                        },
                    );
                }
                _ => {
                    task_results.insert(
                        id.clone(),
                        DagTaskSummary {
                            id: id.clone(),
                            title: task.title.clone(),
                            role: task.role.clone(),
                            status: task.status.status_label().to_string(),
                            duration_ms: None,
                            output_snippet: None,
                            error: None,
                        },
                    );
                }
            }
        }

        if let Some(tx) = &event_tx {
            let _ = tx.send(DagExecutionEvent::DagCompleted {
                dag_id: dag.id.clone(),
                total_duration_ms: wall_duration_ms,
                speedup_ratio,
            });
        }

        Ok(DagExecutionSummary {
            dag_id: dag.id.clone(),
            dag_name: dag.name.clone(),
            goal: dag.goal.clone(),
            overall_status: overall,
            total_tasks: dag.task_count(),
            completed_tasks,
            failed_tasks,
            skipped_tasks,
            total_stages: dag.stages.len(),
            wall_duration_ms,
            cumulative_task_duration_ms,
            speedup_ratio,
            task_results,
        })
    }
}

/// Tool exposing the Subagent DAG planner for interactive AI agent and user invocation.
pub struct PlanSubagentDagTool;

#[async_trait]
impl Tool for PlanSubagentDagTool {
    fn name(&self) -> &str {
        "plan_subagent_dag"
    }

    fn description(&self) -> &str {
        "Decomposes a complex goal into a Directed Acyclic Graph (DAG) of parallel subagent execution stages, with topological validation, stage wave scheduling, and Mermaid export."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "goal": {
                    "type": "string",
                    "description": "The engineering goal or task to decompose into a subagent DAG."
                },
                "strategy": {
                    "type": "string",
                    "enum": ["feature", "bugfix", "refactor", "security", "perf", "custom"],
                    "default": "feature",
                    "description": "Decomposition archetype to use."
                },
                "format": {
                    "type": "string",
                    "enum": ["ascii", "mermaid", "json", "all"],
                    "default": "all",
                    "description": "Output formatting representation."
                }
            },
            "required": ["goal"]
        })
    }

    async fn execute(&self, args: Value, _ctx: &ToolContext) -> anyhow::Result<String> {
        let goal = args
            .get("goal")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("Missing 'goal' parameter"))?;

        let strategy_str = args
            .get("strategy")
            .and_then(|v| v.as_str())
            .unwrap_or("feature");

        let strategy = match strategy_str {
            "bugfix" => DecompositionStrategy::BugFix {
                deep_reproduction: true,
            },
            "refactor" => DecompositionStrategy::Refactoring {
                parallel_modules: true,
            },
            "security" => DecompositionStrategy::SecurityAudit { depth: 2 },
            "perf" => DecompositionStrategy::PerformanceOptimization,
            _ => DecompositionStrategy::SoftwareFeature {
                split_frontend_backend: true,
                include_security_audit: true,
            },
        };

        let format = args
            .get("format")
            .and_then(|v| v.as_str())
            .unwrap_or("all");

        let mut dag = TaskDecomposer::decompose(goal, strategy)
            .map_err(|e| anyhow::anyhow!("Decomposition error: {}", e))?;

        dag.compute_stages()
            .map_err(|e| anyhow::anyhow!("Stage calculation error: {}", e))?;

        let mut out = String::new();
        match format {
            "ascii" => {
                out.push_str(&dag.to_ascii_tree());
            }
            "mermaid" => {
                out.push_str(&dag.to_mermaid());
            }
            "json" => {
                out.push_str(&dag.to_json().map_err(|e| anyhow::anyhow!("{}", e))?);
            }
            _ => {
                out.push_str(&dag.to_ascii_tree());
                out.push_str("\n\n");
                out.push_str(&dag.to_mermaid());
            }
        }

        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_dag_creation_and_topological_sort() {
        let mut dag = SubagentDag::new("Test DAG", "Implement feature");
        let t1 = DagTask::new("t1", "Scout", SubagentRole::Scout, "Explore codebase");
        let t2 = DagTask::new("t2", "Backend", SubagentRole::Coder, "Implement backend").with_dependency("t1");
        let t3 = DagTask::new("t3", "Frontend", SubagentRole::Coder, "Implement frontend").with_dependency("t1");
        let t4 = DagTask::new("t4", "Test", SubagentRole::Tester, "Run tests")
            .with_dependency("t2")
            .with_dependency("t3");

        dag.add_task(t1).unwrap();
        dag.add_task(t2).unwrap();
        dag.add_task(t3).unwrap();
        dag.add_task(t4).unwrap();

        assert_eq!(dag.task_count(), 4);
        dag.validate().unwrap();

        let topo = dag.topological_sort().unwrap();
        assert_eq!(topo[0], "t1");
        assert!(topo[1] == "t2" || topo[1] == "t3");
        assert!(topo[2] == "t2" || topo[2] == "t3");
        assert_eq!(topo[3], "t4");
    }

    #[test]
    fn test_dag_cycle_detection() {
        let mut dag = SubagentDag::new("Cycle DAG", "Goal");
        let t1 = DagTask::new("t1", "T1", SubagentRole::General, "desc").with_dependency("t3");
        let t2 = DagTask::new("t2", "T2", SubagentRole::General, "desc").with_dependency("t1");
        let t3 = DagTask::new("t3", "T3", SubagentRole::General, "desc").with_dependency("t2");

        dag.add_task(t1).unwrap();
        dag.add_task(t2).unwrap();
        dag.add_task(t3).unwrap();

        let res = dag.validate();
        assert!(res.is_err());
        match res.unwrap_err() {
            DagPlannerError::CycleDetected { cycle } => {
                assert_eq!(cycle.len(), 3);
            }
            other => panic!("Expected CycleDetected error, got {:?}", other),
        }
    }

    #[test]
    fn test_stage_wave_partitioning() {
        let mut dag = SubagentDag::new("Waves DAG", "Build system");
        let t1 = DagTask::new("scout", "Scout", SubagentRole::Scout, "Scout");
        let t2 = DagTask::new("coder_a", "Coder A", SubagentRole::Coder, "Coder A").with_dependency("scout");
        let t3 = DagTask::new("coder_b", "Coder B", SubagentRole::Coder, "Coder B").with_dependency("scout");
        let t4 = DagTask::new("tester", "Tester", SubagentRole::Tester, "Tester")
            .with_dependency("coder_a")
            .with_dependency("coder_b");
        let t5 = DagTask::new("reviewer", "Reviewer", SubagentRole::Reviewer, "Reviewer").with_dependency("tester");

        dag.add_task(t1).unwrap();
        dag.add_task(t2).unwrap();
        dag.add_task(t3).unwrap();
        dag.add_task(t4).unwrap();
        dag.add_task(t5).unwrap();

        let stages = dag.compute_stages().unwrap();
        assert_eq!(stages.len(), 4);
        assert_eq!(stages[0].task_ids, vec!["scout"]);
        assert_eq!(stages[1].task_ids.len(), 2);
        assert!(stages[1].task_ids.contains(&"coder_a".to_string()));
        assert!(stages[1].task_ids.contains(&"coder_b".to_string()));
        assert_eq!(stages[2].task_ids, vec!["tester"]);
        assert_eq!(stages[3].task_ids, vec!["reviewer"]);
    }

    #[test]
    fn test_critical_path() {
        let mut dag = SubagentDag::new("Critical Path", "Goal");
        let t1 = DagTask::new("t1", "T1", SubagentRole::Scout, "");
        let t2 = DagTask::new("t2", "T2", SubagentRole::Coder, "").with_dependency("t1");
        let t3 = DagTask::new("t3", "T3", SubagentRole::Coder, "").with_dependency("t2");
        let t4 = DagTask::new("t4", "T4", SubagentRole::Coder, "").with_dependency("t1"); // shorter branch

        dag.add_task(t1).unwrap();
        dag.add_task(t2).unwrap();
        dag.add_task(t3).unwrap();
        dag.add_task(t4).unwrap();

        let cp = dag.critical_path();
        assert_eq!(cp, vec!["t1", "t2", "t3"]);
    }

    #[test]
    fn test_heuristic_feature_decomposition() {
        let dag = TaskDecomposer::decompose(
            "Add streaming JSON output export support",
            DecompositionStrategy::SoftwareFeature {
                split_frontend_backend: true,
                include_security_audit: true,
            },
        )
        .unwrap();

        assert!(dag.task_count() >= 6);
        assert!(dag.tasks.contains_key("scout_arch"));
        assert!(dag.tasks.contains_key("coder_core"));
        assert!(dag.tasks.contains_key("coder_interface"));
        assert!(dag.tasks.contains_key("tester_unit"));
        assert!(dag.tasks.contains_key("reviewer_security"));
    }

    #[test]
    fn test_heuristic_bugfix_decomposition() {
        let dag = TaskDecomposer::decompose(
            "Fix deadlock in async connection pool",
            DecompositionStrategy::BugFix { deep_reproduction: true },
        )
        .unwrap();

        assert_eq!(dag.task_count(), 4);
        assert!(dag.tasks.contains_key("scout_repro"));
        assert!(dag.tasks.contains_key("coder_fix"));
        assert!(dag.tasks.contains_key("tester_verify"));
        assert!(dag.tasks.contains_key("reviewer_audit"));
    }

    #[test]
    fn test_llm_json_parsing_and_recovery() {
        let llm_json = r#"
        ```json
        {
            "name": "Custom Cache Implementation",
            "tasks": [
                {
                    "id": "explore",
                    "title": "Explore Caching Points",
                    "role": "scout",
                    "description": "Find where cache should be attached",
                    "dependencies": [],
                    "priority": "high"
                },
                {
                    "id": "implement",
                    "title": "Implement LRU Cache",
                    "role": "coder",
                    "description": "Build LRU struct with concurrency controls",
                    "dependencies": ["explore"],
                    "priority": "critical"
                },
                {
                    "id": "verify",
                    "title": "Run Thread Stress Tests",
                    "role": "tester",
                    "description": "Simulate 1000 concurrent threads",
                    "dependencies": ["implement"],
                    "priority": "normal"
                }
            ]
        }
        ```
        "#;

        let dag = TaskDecomposer::parse_llm_decomposition(llm_json, "Cache Goal").unwrap();
        assert_eq!(dag.name, "Custom Cache Implementation");
        assert_eq!(dag.task_count(), 3);
        assert_eq!(dag.stages.len(), 3);
    }

    #[tokio::test]
    async fn test_dag_executor_mock_run() {
        let mut dag = SubagentDag::new("Exec Test", "Run parallel tasks");
        let t1 = DagTask::new("s1", "Scout 1", SubagentRole::Scout, "Explore A");
        let t2 = DagTask::new("s2", "Scout 2", SubagentRole::Scout, "Explore B");
        let t3 = DagTask::new("c1", "Coder", SubagentRole::Coder, "Combine {{upstream.s1.output}} and {{upstream.s2.output}}")
            .with_dependencies(vec!["s1", "s2"]);

        dag.add_task(t1).unwrap();
        dag.add_task(t2).unwrap();
        dag.add_task(t3).unwrap();

        let mock_executor = Arc::new(
            MockTaskExecutor::new()
                .with_task_result("s1", Ok("Found module A".to_string()))
                .with_task_result("s2", Ok("Found module B".to_string()))
                .with_task_result("c1", Ok("Implemented integration".to_string())),
        );

        let runner = DagExecutor::new(DagExecutionConfig::default());
        let summary = runner.run(&mut dag, mock_executor, None).await.unwrap();

        assert_eq!(summary.overall_status, DagOverallStatus::Completed);
        assert_eq!(summary.completed_tasks, 3);
        assert_eq!(summary.failed_tasks, 0);
        assert_eq!(summary.skipped_tasks, 0);
        assert!(dag.get_task("c1").unwrap().status.is_completed());
    }

    #[tokio::test]
    async fn test_dag_executor_failure_cascade_and_skip() {
        let mut dag = SubagentDag::new("Failure Test", "Check skip cascade");
        let t1 = DagTask::new("t1", "Task 1", SubagentRole::Coder, "Failing step");
        let t2 = DagTask::new("t2", "Task 2", SubagentRole::Tester, "Should be skipped").with_dependency("t1");

        dag.add_task(t1).unwrap();
        dag.add_task(t2).unwrap();

        let mock_executor = Arc::new(
            MockTaskExecutor::new()
                .with_task_result("t1", Err("Compiler syntax error".to_string())),
        );

        let mut config = DagExecutionConfig::default();
        config.default_max_retries = 0;
        config.continue_on_failure = false;

        let runner = DagExecutor::new(config);
        let summary = runner.run(&mut dag, mock_executor, None).await.unwrap();

        assert_eq!(summary.failed_tasks, 1);
        assert_eq!(summary.skipped_tasks, 1);
        assert!(dag.get_task("t2").unwrap().status.is_skipped());
    }

    #[test]
    fn test_mermaid_and_ascii_exports() {
        let dag = TaskDecomposer::decompose(
            "Build auth system",
            DecompositionStrategy::SoftwareFeature {
                split_frontend_backend: true,
                include_security_audit: true,
            },
        )
        .unwrap();

        let mermaid = dag.to_mermaid();
        assert!(mermaid.contains("graph TD"));
        assert!(mermaid.contains("scout_arch"));
        assert!(mermaid.contains("coder_core"));

        let ascii = dag.to_ascii_tree();
        assert!(ascii.contains("DAG Plan:"));
        assert!(ascii.contains("scout_arch"));
    }
}
