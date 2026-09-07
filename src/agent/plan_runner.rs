//! PlanRunner: Autonomous execution engine for Subagent DAG plans.
//!
//! Coordinates the concurrent execution of multi-agent plans organized as Directed Acyclic Graphs (DAGs).
//! Manages topological execution stages, dispatches subagent batches concurrently via `SubagentManager::spawn_batch`,
//! handles task dependency propagation, tracks status transitions, and provides ASCII progress visualization.

use std::collections::HashMap;
use std::time::Instant;

use chrono::Utc;
use futures::future::join_all;
use serde::{Deserialize, Serialize};

use crate::agent::planner_dag::{
    ContextResolver, DagOverallStatus, DagPlannerError, DagStage, DagTaskStatus, StageStatus,
    SubagentDag,
};
use crate::agent::subagent::{SubagentManager, SubagentTask};

/// Errors that can occur during plan preparation, stage execution, or autonomous runs.
#[derive(Debug, thiserror::Error, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum PlanRunnerError {
    #[error("Stage index {stage_idx} is out of bounds (total stages: {total_stages})")]
    StageNotFound {
        stage_idx: usize,
        total_stages: usize,
    },

    #[error("Task '{task_id}' failed: {error}")]
    TaskExecutionFailed {
        task_id: String,
        error: String,
    },

    #[error("Task '{task_id}' prerequisite dependency '{dep_id}' not completed (status: {dep_status})")]
    DependencyNotMet {
        task_id: String,
        dep_id: String,
        dep_status: String,
    },

    #[error("Task '{task_id}' not found in DAG")]
    TaskNotFound {
        task_id: String,
    },

    #[error("Cannot run empty plan")]
    EmptyPlan,

    #[error("DAG planner error: {0}")]
    DagError(String),

    #[error("Subagent manager error: {0}")]
    SubagentError(String),

    #[error("Stage #{stage_idx} failed: {failed_tasks:?}")]
    StageFailed {
        stage_idx: usize,
        failed_tasks: Vec<String>,
    },
}

impl From<DagPlannerError> for PlanRunnerError {
    fn from(err: DagPlannerError) -> Self {
        PlanRunnerError::DagError(err.to_string())
    }
}

impl From<anyhow::Error> for PlanRunnerError {
    fn from(err: anyhow::Error) -> Self {
        PlanRunnerError::SubagentError(err.to_string())
    }
}

/// Result of executing a single topological stage wave of concurrent tasks.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StageExecutionResult {
    /// Zero-based stage index.
    pub stage_idx: usize,
    /// Human-readable name of the stage.
    pub stage_name: String,
    /// All task IDs scheduled for this stage.
    pub task_ids: Vec<String>,
    /// Task IDs that completed successfully.
    pub completed_tasks: Vec<String>,
    /// Task IDs that failed, along with their error messages.
    pub failed_tasks: Vec<(String, String)>,
    /// Output produced by each completed task node.
    pub task_outputs: HashMap<String, String>,
    /// Total wall-clock duration of the stage in milliseconds.
    pub duration_ms: u64,
    /// Whether all tasks in the stage completed successfully without failure.
    pub success: bool,
}

impl StageExecutionResult {
    /// Checks whether all tasks in the stage succeeded.
    pub fn is_success(&self) -> bool {
        self.success
    }

    /// Number of tasks successfully completed.
    pub fn completed_count(&self) -> usize {
        self.completed_tasks.len()
    }

    /// Number of tasks that failed.
    pub fn failed_count(&self) -> usize {
        self.failed_tasks.len()
    }
}

/// Comprehensive summary report produced upon DAG plan execution.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlanSummary {
    /// Unique plan/DAG identifier.
    pub dag_id: String,
    /// Name/title of the DAG plan.
    pub dag_name: String,
    /// High-level goal of the plan.
    pub goal: String,
    /// Overall lifecycle status.
    pub overall_status: DagOverallStatus,
    /// Total number of tasks in the plan.
    pub total_tasks: usize,
    /// Number of successfully completed tasks.
    pub completed_tasks: usize,
    /// Number of failed tasks.
    pub failed_tasks: usize,
    /// Number of skipped tasks.
    pub skipped_tasks: usize,
    /// Total number of topological stages.
    pub total_stages: usize,
    /// Number of stages that completed successfully.
    pub completed_stages: usize,
    /// Total wall-clock execution duration in milliseconds.
    pub wall_duration_ms: u64,
    /// Task outputs keyed by task ID.
    pub task_results: HashMap<String, String>,
    /// Individual stage execution results.
    pub stage_results: Vec<StageExecutionResult>,
}

impl PlanSummary {
    /// Returns true if all tasks in the plan succeeded with zero failures.
    pub fn is_success(&self) -> bool {
        self.failed_tasks == 0 && self.completed_tasks == self.total_tasks
    }

    /// Formats the summary into a structured Markdown document.
    pub fn format_markdown(&self) -> String {
        let mut out = String::new();
        out.push_str(&format!("# Plan Execution Report: {}\n\n", self.dag_name));
        out.push_str(&format!("- **Plan ID:** `{}`\n", self.dag_id));
        out.push_str(&format!("- **Goal:** {}\n", self.goal));
        out.push_str(&format!("- **Overall Status:** {}\n", self.overall_status));
        out.push_str(&format!(
            "- **Tasks:** {} completed, {} failed, {} skipped (Total: {})\n",
            self.completed_tasks, self.failed_tasks, self.skipped_tasks, self.total_tasks
        ));
        out.push_str(&format!(
            "- **Stages:** {}/{} completed\n",
            self.completed_stages, self.total_stages
        ));
        out.push_str(&format!(
            "- **Wall Clock Time:** {}ms\n\n",
            self.wall_duration_ms
        ));

        if !self.task_results.is_empty() {
            out.push_str("## Task Results\n\n");
            let mut sorted_keys: Vec<_> = self.task_results.keys().collect();
            sorted_keys.sort();
            for key in sorted_keys {
                let output = &self.task_results[key];
                out.push_str(&format!("### Task `{}`\n{}\n\n", key, output.trim()));
            }
        }

        out
    }
}

/// Orchestrates execution of a Subagent DAG using a `SubagentManager`.
pub struct PlanRunner {
    /// The DAG defining tasks, dependencies, stages, and execution state.
    pub dag: SubagentDag,
    /// Subagent manager responsible for concurrency permits, task loops, and event emission.
    pub manager: SubagentManager,
}

impl PlanRunner {
    /// Creates a new `PlanRunner` instance wrapping a DAG and SubagentManager.
    pub fn new(dag: SubagentDag, manager: SubagentManager) -> Self {
        Self { dag, manager }
    }

    /// Returns an immutable reference to the inner DAG.
    pub fn dag(&self) -> &SubagentDag {
        &self.dag
    }

    /// Returns a mutable reference to the inner DAG.
    pub fn dag_mut(&mut self) -> &mut SubagentDag {
        &mut self.dag
    }

    /// Returns an immutable reference to the subagent manager.
    pub fn manager(&self) -> &SubagentManager {
        &self.manager
    }

    /// Returns a mutable reference to the subagent manager.
    pub fn manager_mut(&mut self) -> &mut SubagentManager {
        &mut self.manager
    }

    /// Total number of tasks in the DAG.
    pub fn task_count(&self) -> usize {
        self.dag.task_count()
    }

    /// Total number of stages in the DAG (if computed).
    pub fn stage_count(&self) -> usize {
        self.dag.stages.len()
    }

    /// Returns the overall lifecycle status of the DAG.
    pub fn overall_status(&self) -> DagOverallStatus {
        self.dag.overall_status()
    }

    /// Ensures that topological stages are computed for the DAG.
    pub fn ensure_stages(&mut self) -> Result<&Vec<DagStage>, PlanRunnerError> {
        if self.dag.stages.is_empty() {
            if self.dag.tasks.is_empty() {
                return Err(PlanRunnerError::EmptyPlan);
            }
            self.dag.compute_stages().map_err(PlanRunnerError::from)?;
        }
        Ok(&self.dag.stages)
    }

    /// Returns a status symbol for a task by ID.
    fn task_status_symbol(&self, task_id: &str) -> &'static str {
        if let Some(task) = self.dag.get_task(task_id) {
            match &task.status {
                DagTaskStatus::Completed { .. } => "[✓]",
                DagTaskStatus::Running { .. } => "[•]",
                DagTaskStatus::Ready => "[·]",
                DagTaskStatus::Pending => "[ ]",
                DagTaskStatus::Failed { .. } => "[✗]",
                DagTaskStatus::Skipped { .. } => "[-]",
                DagTaskStatus::Blocked { .. } => "[⏸]",
            }
        } else {
            "[?]"
        }
    }

    /// Renders an ASCII visualization of the DAG execution progress.
    ///
    /// Example output:
    /// `[✓] scout -> [•] coder -> [ ] tester`
    /// Or with parallel tasks:
    /// `([✓] scout_a, [✓] scout_b) -> [•] coder -> [ ] tester`
    pub fn render_progress_ascii(&self) -> String {
        if self.dag.tasks.is_empty() {
            return String::from("[ ] (empty plan)");
        }

        // If stages are computed, render stage waves chained by " -> "
        if !self.dag.stages.is_empty() {
            let stage_strs: Vec<String> = self
                .dag
                .stages
                .iter()
                .map(|stage| {
                    if stage.task_ids.len() == 1 {
                        let tid = &stage.task_ids[0];
                        format!("{} {}", self.task_status_symbol(tid), tid)
                    } else {
                        let tasks: Vec<String> = stage
                            .task_ids
                            .iter()
                            .map(|tid| format!("{} {}", self.task_status_symbol(tid), tid))
                            .collect();
                        format!("({})", tasks.join(", "))
                    }
                })
                .collect();
            return stage_strs.join(" -> ");
        }

        // If stages not yet computed, compute groups via topological depth
        if let Ok(topo) = self.dag.topological_sort() {
            let mut levels: HashMap<String, usize> = HashMap::new();
            for id in &topo {
                let task = &self.dag.tasks[id];
                if task.dependencies.is_empty() {
                    levels.insert(id.clone(), 0);
                } else {
                    let max_dep = task
                        .dependencies
                        .iter()
                        .map(|d| *levels.get(d).unwrap_or(&0))
                        .max()
                        .unwrap_or(0);
                    levels.insert(id.clone(), max_dep + 1);
                }
            }
            let max_level = levels.values().copied().max().unwrap_or(0);
            let mut groups = vec![Vec::new(); max_level + 1];
            for id in topo {
                groups[levels[&id]].push(id);
            }
            let stage_strs: Vec<String> = groups
                .into_iter()
                .map(|group| {
                    if group.len() == 1 {
                        let tid = &group[0];
                        format!("{} {}", self.task_status_symbol(tid), tid)
                    } else {
                        let tasks: Vec<String> = group
                            .iter()
                            .map(|tid| format!("{} {}", self.task_status_symbol(tid), tid))
                            .collect();
                        format!("({})", tasks.join(", "))
                    }
                })
                .collect();
            stage_strs.join(" -> ")
        } else {
            let mut task_ids: Vec<String> = self.dag.tasks.keys().cloned().collect();
            task_ids.sort();
            let tasks: Vec<String> = task_ids
                .iter()
                .map(|tid| format!("{} {}", self.task_status_symbol(tid), tid))
                .collect();
            tasks.join(" -> ")
        }
    }

    /// Renders a multi-line detailed ASCII progress report.
    pub fn render_detailed_ascii(&self) -> String {
        self.dag.to_ascii_tree()
    }

    /// Generates a snapshot summary of the plan's current state.
    pub fn get_summary(&self) -> PlanSummary {
        let mut completed_tasks = 0;
        let mut failed_tasks = 0;
        let mut skipped_tasks = 0;
        let mut task_results = HashMap::new();

        for (id, task) in &self.dag.tasks {
            match &task.status {
                DagTaskStatus::Completed { output, .. } => {
                    completed_tasks += 1;
                    task_results.insert(id.clone(), output.clone());
                }
                DagTaskStatus::Failed { .. } => {
                    failed_tasks += 1;
                }
                DagTaskStatus::Skipped { .. } => {
                    skipped_tasks += 1;
                }
                _ => {}
            }
        }

        let completed_stages = self
            .dag
            .stages
            .iter()
            .filter(|s| s.status == StageStatus::Completed)
            .count();

        PlanSummary {
            dag_id: self.dag.id.clone(),
            dag_name: self.dag.name.clone(),
            goal: self.dag.goal.clone(),
            overall_status: self.dag.overall_status(),
            total_tasks: self.dag.task_count(),
            completed_tasks,
            failed_tasks,
            skipped_tasks,
            total_stages: self.dag.stages.len(),
            completed_stages,
            wall_duration_ms: 0,
            task_results,
            stage_results: Vec::new(),
        }
    }

    /// Executes all tasks in the given topological stage concurrently via `SubagentManager::spawn_batch`.
    pub async fn execute_stage(
        &mut self,
        stage_idx: usize,
    ) -> Result<StageExecutionResult, PlanRunnerError> {
        self.ensure_stages()?;

        let total_stages = self.dag.stages.len();
        if stage_idx >= total_stages {
            return Err(PlanRunnerError::StageNotFound {
                stage_idx,
                total_stages,
            });
        }

        let stage_name = self.dag.stages[stage_idx].name.clone();
        let stage_task_ids = self.dag.stages[stage_idx].task_ids.clone();

        // Validate dependencies for all tasks in this stage
        for task_id in &stage_task_ids {
            let task = self
                .dag
                .get_task(task_id)
                .ok_or_else(|| PlanRunnerError::TaskNotFound {
                    task_id: task_id.clone(),
                })?;
            for dep in &task.dependencies {
                let dep_task = self
                    .dag
                    .get_task(dep)
                    .ok_or_else(|| PlanRunnerError::TaskNotFound {
                        task_id: dep.clone(),
                    })?;
                if !dep_task.status.is_completed() {
                    return Err(PlanRunnerError::DependencyNotMet {
                        task_id: task_id.clone(),
                        dep_id: dep.clone(),
                        dep_status: dep_task.status.status_label().to_string(),
                    });
                }
            }
        }

        let stage_start = Instant::now();
        self.dag.stages[stage_idx].status = StageStatus::InProgress;
        self.dag.stages[stage_idx].started_at = Some(Utc::now().to_rfc3339());

        // Build SubagentTask for each task node
        let mut subagent_tasks = Vec::with_capacity(stage_task_ids.len());
        let mut task_order = Vec::with_capacity(stage_task_ids.len());

        for task_id in &stage_task_ids {
            let (prompt, role, title, model) = {
                let task = self.dag.get_task(task_id).unwrap();
                let prompt = ContextResolver::resolve_task_prompt(task, &self.dag, &self.dag.goal);
                let role = task.role.clone();
                let title = task.title.clone();
                let model = task
                    .metadata
                    .get("model")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string());
                (prompt, role, title, model)
            };

            let agent_id = format!("{}-{}", task_id, &uuid::Uuid::new_v4().to_string()[..6]);

            // Mark task as running in DAG
            if let Some(t_mut) = self.dag.get_task_mut(task_id) {
                t_mut.status = DagTaskStatus::Running {
                    agent_id: agent_id.clone(),
                    started_at: Utc::now().to_rfc3339(),
                };
                t_mut.assigned_agent_id = Some(agent_id);
                t_mut.started_at = Some(Utc::now().to_rfc3339());
            }

            let mut subagent_task = SubagentTask::new(role, prompt)
                .with_id(task_id.clone())
                .with_name(title);

            if let Some(ref m) = model {
                subagent_task = subagent_task.with_model(m);
            }

            subagent_tasks.push(subagent_task);
            task_order.push(task_id.clone());
        }

        // Spawn batch concurrently via SubagentManager
        let handles = self.manager.spawn_batch(subagent_tasks);

        // Await all subagent handles concurrently
        let mut futures = Vec::with_capacity(handles.len());
        for handle in handles {
            futures.push(handle.wait());
        }
        let results = join_all(futures).await;

        // Process outcomes
        let mut completed_tasks = Vec::new();
        let mut failed_tasks = Vec::new();
        let mut task_outputs = HashMap::new();

        for (task_id, result) in task_order.into_iter().zip(results.into_iter()) {
            let task_elapsed = stage_start.elapsed().as_millis() as u64;
            match result {
                Ok(sub_res) => {
                    if sub_res.success {
                        self.dag
                            .mark_task_completed(&task_id, sub_res.output.clone(), task_elapsed);
                        completed_tasks.push(task_id.clone());
                        task_outputs.insert(task_id, sub_res.output);
                    } else {
                        let err_msg = if sub_res.output.is_empty() {
                            format!("Task '{}' subagent returned failure", task_id)
                        } else {
                            sub_res.output.clone()
                        };
                        self.dag.mark_task_failed(&task_id, err_msg.clone(), 0);
                        failed_tasks.push((task_id, err_msg));
                    }
                }
                Err(err) => {
                    let err_msg = err.to_string();
                    self.dag.mark_task_failed(&task_id, err_msg.clone(), 0);
                    failed_tasks.push((task_id, err_msg));
                }
            }
        }

        let stage_duration_ms = stage_start.elapsed().as_millis() as u64;
        let success = failed_tasks.is_empty();

        // Update stage lifecycle status in DAG
        self.dag.stages[stage_idx].status = if success {
            StageStatus::Completed
        } else if !completed_tasks.is_empty() {
            StageStatus::PartialSuccess
        } else {
            StageStatus::Failed
        };
        self.dag.stages[stage_idx].finished_at = Some(Utc::now().to_rfc3339());
        self.dag.stages[stage_idx].duration_ms = Some(stage_duration_ms);

        Ok(StageExecutionResult {
            stage_idx,
            stage_name,
            task_ids: stage_task_ids,
            completed_tasks,
            failed_tasks,
            task_outputs,
            duration_ms: stage_duration_ms,
            success,
        })
    }

    /// Iterates through all topological stages sequentially until the DAG is complete or a task fails.
    pub async fn run_autonomous(&mut self) -> Result<PlanSummary, PlanRunnerError> {
        self.ensure_stages()?;

        let start_time = Instant::now();
        let total_stages = self.dag.stages.len();
        let mut stage_results = Vec::with_capacity(total_stages);
        let mut total_completed = 0;
        let mut total_failed = 0;
        let mut all_outputs = HashMap::new();

        for stage_idx in 0..total_stages {
            let stage_res = self.execute_stage(stage_idx).await?;
            total_completed += stage_res.completed_tasks.len();
            total_failed += stage_res.failed_tasks.len();

            for (k, v) in &stage_res.task_outputs {
                all_outputs.insert(k.clone(), v.clone());
            }

            let is_success = stage_res.success;
            let failed_list = stage_res.failed_tasks.clone();
            stage_results.push(stage_res);

            if !is_success {
                let (failed_task, error) = failed_list
                    .first()
                    .cloned()
                    .unwrap_or_else(|| ("unknown".to_string(), "Stage execution failed".to_string()));
                return Err(PlanRunnerError::TaskExecutionFailed {
                    task_id: failed_task,
                    error,
                });
            }
        }

        let wall_duration_ms = start_time.elapsed().as_millis() as u64;
        let skipped_tasks = self
            .dag
            .tasks
            .values()
            .filter(|t| t.status.is_skipped())
            .count();
        let completed_stages = stage_results.iter().filter(|s| s.success).count();

        Ok(PlanSummary {
            dag_id: self.dag.id.clone(),
            dag_name: self.dag.name.clone(),
            goal: self.dag.goal.clone(),
            overall_status: self.dag.overall_status(),
            total_tasks: self.dag.task_count(),
            completed_tasks: total_completed,
            failed_tasks: total_failed,
            skipped_tasks,
            total_stages,
            completed_stages,
            wall_duration_ms,
            task_results: all_outputs,
            stage_results,
        })
    }
}
