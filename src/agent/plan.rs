use std::collections::{HashMap, HashSet};
use std::fmt;
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::sync::Arc;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use chrono::Utc;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tokio::sync::{mpsc, oneshot, RwLock};
use uuid::Uuid;

use crate::agent::advisor::RiskLevel;
use crate::agent::loop_runner::AgentRunner;
use crate::agent::session::Session;
use crate::agent::subagent::SubagentRole;
use crate::config::Config;
use crate::provider::types::Message;
use crate::provider::LlmClient;
use crate::tools::types::{Tool, ToolContext};

/// Execution status of an individual step in a plan.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", content = "data", rename_all = "snake_case")]
pub enum StepStatus {
    /// Step has not yet begun.
    Pending,
    /// Step is actively executing.
    InProgress,
    /// Step completed successfully.
    Completed {
        result: Option<String>,
        finished_at: String,
    },
    /// Step failed during execution.
    Failed { error: String, failed_at: String },
    /// Step was deliberately skipped by the user or policy.
    Skipped { reason: String },
    /// Step is waiting on unsatisfied prerequisites.
    Blocked { blocked_by: Vec<String> },
}

impl StepStatus {
    /// Returns true if the step is pending.
    pub fn is_pending(&self) -> bool {
        matches!(self, StepStatus::Pending)
    }

    /// Returns true if the step is currently in progress.
    pub fn is_in_progress(&self) -> bool {
        matches!(self, StepStatus::InProgress)
    }

    /// Returns true if the step has completed successfully.
    pub fn is_completed(&self) -> bool {
        matches!(self, StepStatus::Completed { .. })
    }

    /// Returns true if the step has failed.
    pub fn is_failed(&self) -> bool {
        matches!(self, StepStatus::Failed { .. })
    }

    /// Returns true if the step was skipped.
    pub fn is_skipped(&self) -> bool {
        matches!(self, StepStatus::Skipped { .. })
    }

    /// Returns true if the step is blocked by dependencies.
    pub fn is_blocked(&self) -> bool {
        matches!(self, StepStatus::Blocked { .. })
    }

    /// Returns true if the step has reached a terminal state (completed, failed, or skipped).
    pub fn is_terminal(&self) -> bool {
        matches!(
            self,
            StepStatus::Completed { .. } | StepStatus::Failed { .. } | StepStatus::Skipped { .. }
        )
    }

    /// Terminal status checkbox symbol for interactive checklist rendering.
    pub fn checkbox_symbol(&self) -> &'static str {
        match self {
            StepStatus::Pending => "[ ]",
            StepStatus::InProgress => "[▶]",
            StepStatus::Completed { .. } => "[✓]",
            StepStatus::Failed { .. } => "[✗]",
            StepStatus::Skipped { .. } => "[-]",
            StepStatus::Blocked { .. } => "[⏸]",
        }
    }

    /// Descriptive textual label of status.
    pub fn label(&self) -> &'static str {
        match self {
            StepStatus::Pending => "Pending",
            StepStatus::InProgress => "In Progress",
            StepStatus::Completed { .. } => "Completed",
            StepStatus::Failed { .. } => "Failed",
            StepStatus::Skipped { .. } => "Skipped",
            StepStatus::Blocked { .. } => "Blocked",
        }
    }
}

impl fmt::Display for StepStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.label())
    }
}

/// Execution status of a logical phase.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PhaseStatus {
    Pending,
    InProgress,
    Completed,
    Failed,
    Skipped,
}

impl PhaseStatus {
    pub fn is_completed(&self) -> bool {
        matches!(self, PhaseStatus::Completed)
    }
    pub fn is_skipped(&self) -> bool {
        matches!(self, PhaseStatus::Skipped)
    }

    pub fn is_terminal(&self) -> bool {
        matches!(
            self,
            PhaseStatus::Completed | PhaseStatus::Failed | PhaseStatus::Skipped
        )
    }

    pub fn label(&self) -> &'static str {
        match self {
            PhaseStatus::Pending => "Pending",
            PhaseStatus::InProgress => "In Progress",
            PhaseStatus::Completed => "Completed",
            PhaseStatus::Failed => "Failed",
            PhaseStatus::Skipped => "Skipped",
        }
    }
}

impl fmt::Display for PhaseStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.label())
    }
}

/// Overall lifecycle state of a planning mode run.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PlanState {
    /// Plan drafted and ready for review or editing.
    Draft,
    /// Plan confirmed by user and ready for execution.
    Ready,
    /// Plan is actively executing tasks.
    Executing,
    /// Plan execution is paused at a confirmation checkpoint awaiting user input.
    PausedAtCheckpoint,
    /// All steps have been successfully executed.
    Completed,
    /// One or more steps failed and execution stopped.
    Failed,
    /// Execution was canceled or aborted by the user.
    Aborted,
}

impl fmt::Display for PlanState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PlanState::Draft => write!(f, "Draft"),
            PlanState::Ready => write!(f, "Ready"),
            PlanState::Executing => write!(f, "Executing"),
            PlanState::PausedAtCheckpoint => write!(f, "Paused at Checkpoint"),
            PlanState::Completed => write!(f, "Completed"),
            PlanState::Failed => write!(f, "Failed"),
            PlanState::Aborted => write!(f, "Aborted"),
        }
    }
}

/// Policy determining when confirmation checkpoints are triggered.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CheckpointPolicy {
    /// Pause and require confirmation before every single step.
    AlwaysConfirm,
    /// Pause and require confirmation at phase boundaries (start and end of phases).
    PhaseBoundary,
    /// Pause only for steps assessed as High or Critical risk, or with explicit flags.
    RiskBased,
    /// Pause only when a step or phase explicitly has `requires_confirmation = true`.
    ExplicitOnly,
    /// Fully autonomous execution without confirmation prompts (e.g. CI/scripts).
    AutoApprove,
}

impl Default for CheckpointPolicy {
    fn default() -> Self {
        CheckpointPolicy::PhaseBoundary
    }
}

impl FromStr for CheckpointPolicy {
    type Err = std::convert::Infallible;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(match s.trim().to_lowercase().as_str() {
            "always" | "all" | "step" => CheckpointPolicy::AlwaysConfirm,
            "phase" | "boundary" => CheckpointPolicy::PhaseBoundary,
            "risk" | "risk_based" | "safe" => CheckpointPolicy::RiskBased,
            "auto" | "unattended" | "none" => CheckpointPolicy::AutoApprove,
            _ => CheckpointPolicy::ExplicitOnly,
        })
    }
}

/// The nature/stage of a confirmation checkpoint.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CheckpointType {
    /// Before entering a new phase.
    PrePhase,
    /// Before beginning a specific step.
    PreStep,
    /// Verification check after completing a step.
    PostStepVerification,
    /// Checkpoint invoked after a step failure to decide next actions.
    FailureRecovery,
}

/// Detailed context presented to the user during a confirmation checkpoint.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConfirmationCheckpoint {
    /// Stage of the checkpoint.
    pub checkpoint_type: CheckpointType,
    /// ID of the relevant phase.
    pub phase_id: String,
    /// Name of the relevant phase.
    pub phase_name: String,
    /// ID of the specific step (if applicable).
    pub step_id: Option<String>,
    /// Title of the specific step (if applicable).
    pub step_title: Option<String>,
    /// Explanation of why the checkpoint was triggered.
    pub description: String,
    /// Assessed risk level.
    pub risk_level: RiskLevel,
    /// Files targeted or modified by this step or phase.
    pub targeted_files: Vec<String>,
    /// Summary of the upcoming action or tool calls.
    pub action_summary: String,
    /// Suggested prompt or options for the user.
    pub prompt: String,
}

/// User's decision when responding to a confirmation checkpoint.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum CheckpointDecision {
    /// Approve and proceed with the planned action.
    Approve,
    /// Approve with injected steering advice or modified instructions.
    ApproveWithFeedback { feedback: String },
    /// Skip this step and proceed to the next available step.
    Skip { reason: String },
    /// Retry the failed step.
    Retry,
    /// Abort plan execution immediately.
    Abort { reason: String },
}

fn default_step_risk() -> RiskLevel {
    RiskLevel::Low
}

/// A concrete, granular task inside a planning phase.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlanStep {
    /// Unique identifier for this step within the plan.
    pub id: String,
    /// Identifier of the parent phase.
    pub phase_id: String,
    /// Concise, imperative title of the step (e.g., "Inspect Cargo.toml dependencies").
    pub title: String,
    /// Comprehensive details on what needs to be done.
    pub description: String,
    /// Current execution status.
    pub status: StepStatus,
    /// Suggested subagent specialist role for this step.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub role: Option<SubagentRole>,
    /// Assessed risk level.
    #[serde(default = "default_step_risk")]
    pub risk_level: RiskLevel,
    /// Whether this step explicitly demands user confirmation before running.
    #[serde(default)]
    pub requires_confirmation: bool,
    /// IDs of steps that must be completed before this step can run.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub dependencies: Vec<String>,
    /// File paths targeted or expected to be modified.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub targeted_files: Vec<String>,
    /// Explicit verification checks to confirm step success.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub verification_criteria: Vec<String>,
    /// Notes, findings, or artifacts captured during execution.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub execution_notes: Option<String>,
    /// Standardized output from the step run.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output: Option<String>,
    /// ISO 8601 timestamp when the step was created.
    pub created_at: String,
    /// ISO 8601 timestamp when execution began.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub started_at: Option<String>,
    /// ISO 8601 timestamp when execution finished.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub completed_at: Option<String>,
    /// Execution duration in milliseconds.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duration_ms: Option<u64>,
}

impl PlanStep {
    /// Creates a new pending plan step with default settings.
    pub fn new(
        id: impl Into<String>,
        phase_id: impl Into<String>,
        title: impl Into<String>,
        description: impl Into<String>,
    ) -> Self {
        Self {
            id: id.into(),
            phase_id: phase_id.into(),
            title: title.into(),
            description: description.into(),
            status: StepStatus::Pending,
            role: None,
            risk_level: RiskLevel::Low,
            requires_confirmation: false,
            dependencies: Vec::new(),
            targeted_files: Vec::new(),
            verification_criteria: Vec::new(),
            execution_notes: None,
            output: None,
            created_at: Utc::now().to_rfc3339(),
            started_at: None,
            completed_at: None,
            duration_ms: None,
        }
    }

    /// Sets the assigned subagent role.
    pub fn with_role(mut self, role: SubagentRole) -> Self {
        self.role = Some(role);
        self
    }

    /// Sets the assessed risk level.
    pub fn with_risk(mut self, risk: RiskLevel) -> Self {
        self.risk_level = risk;
        self
    }

    /// Sets explicit confirmation requirement.
    pub fn require_confirmation(mut self) -> Self {
        self.requires_confirmation = true;
        self
    }

    /// Adds a prerequisite dependency.
    pub fn add_dependency(mut self, step_id: impl Into<String>) -> Self {
        self.dependencies.push(step_id.into());
        self
    }

    /// Adds targeted files.
    pub fn with_targeted_files(mut self, files: Vec<String>) -> Self {
        self.targeted_files = files;
        self
    }

    /// Adds verification criteria.
    pub fn with_verification_criteria(mut self, criteria: Vec<String>) -> Self {
        self.verification_criteria = criteria;
        self
    }

    /// Marks the step as in-progress.
    pub fn mark_in_progress(&mut self) {
        self.status = StepStatus::InProgress;
        self.started_at = Some(Utc::now().to_rfc3339());
    }

    /// Marks the step as successfully completed.
    pub fn mark_completed(&mut self, result: Option<String>, duration: Option<Duration>) {
        let now = Utc::now().to_rfc3339();
        self.status = StepStatus::Completed {
            result: result.clone(),
            finished_at: now.clone(),
        };
        self.output = result;
        self.completed_at = Some(now);
        if let Some(dur) = duration {
            self.duration_ms = Some(dur.as_millis() as u64);
        }
    }

    /// Marks the step as failed.
    pub fn mark_failed(&mut self, error: impl Into<String>, duration: Option<Duration>) {
        let now = Utc::now().to_rfc3339();
        let err_str = error.into();
        self.status = StepStatus::Failed {
            error: err_str.clone(),
            failed_at: now.clone(),
        };
        self.output = Some(format!("Error: {}", err_str));
        self.completed_at = Some(now);
        if let Some(dur) = duration {
            self.duration_ms = Some(dur.as_millis() as u64);
        }
    }

    /// Marks the step as skipped.
    pub fn mark_skipped(&mut self, reason: impl Into<String>) {
        self.status = StepStatus::Skipped {
            reason: reason.into(),
        };
        self.completed_at = Some(Utc::now().to_rfc3339());
    }
}

/// A logical stage grouping multiple related steps in a plan.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Phase {
    /// Unique identifier for this phase.
    pub id: String,
    /// Human-readable title of the phase.
    pub name: String,
    /// Detailed description of the phase objective.
    pub description: String,
    /// Ordered list of steps belonging to this phase.
    pub steps: Vec<PlanStep>,
    /// Aggregate status of the phase.
    pub status: PhaseStatus,
    /// Whether entering this phase requires explicit confirmation.
    #[serde(default)]
    pub requires_confirmation: bool,
}

impl Phase {
    /// Creates a new phase with empty steps.
    pub fn new(
        id: impl Into<String>,
        name: impl Into<String>,
        description: impl Into<String>,
    ) -> Self {
        Self {
            id: id.into(),
            name: name.into(),
            description: description.into(),
            steps: Vec::new(),
            status: PhaseStatus::Pending,
            requires_confirmation: false,
        }
    }

    /// Appends a step to this phase.
    pub fn add_step(&mut self, step: PlanStep) {
        self.steps.push(step);
    }

    /// Sets explicit confirmation requirement for the phase.
    pub fn require_confirmation(mut self) -> Self {
        self.requires_confirmation = true;
        self
    }

    /// Calculates (completed_steps, total_steps) for this phase.
    pub fn progress(&self) -> (usize, usize) {
        let total = self.steps.len();
        let completed = self
            .steps
            .iter()
            .filter(|s| s.status.is_completed() || s.status.is_skipped())
            .count();
        (completed, total)
    }

    /// Checks whether all steps in this phase have reached a terminal state.
    pub fn is_all_terminal(&self) -> bool {
        !self.steps.is_empty() && self.steps.iter().all(|s| s.status.is_terminal())
    }

    /// Checks whether all steps in this phase succeeded.
    pub fn is_completed(&self) -> bool {
        !self.steps.is_empty()
            && self
                .steps
                .iter()
                .all(|s| s.status.is_completed() || s.status.is_skipped())
    }

    /// Checks whether any step in this phase failed.
    pub fn has_failures(&self) -> bool {
        self.steps.iter().any(|s| s.status.is_failed())
    }

    /// Finds a step by ID within this phase.
    pub fn find_step(&self, step_id: &str) -> Option<&PlanStep> {
        self.steps.iter().find(|s| s.id == step_id)
    }

    /// Finds a mutable step by ID within this phase.
    pub fn find_step_mut(&mut self, step_id: &str) -> Option<&mut PlanStep> {
        self.steps.iter_mut().find(|s| s.id == step_id)
    }

    /// Updates the phase aggregate status based on its steps.
    pub fn update_status(&mut self) {
        if self.steps.is_empty() {
            self.status = PhaseStatus::Pending;
            return;
        }

        if self.has_failures() {
            self.status = PhaseStatus::Failed;
        } else if self.is_completed() {
            self.status = PhaseStatus::Completed;
        } else if self.steps.iter().any(|s| s.status.is_in_progress())
            || self.steps.iter().any(|s| s.status.is_completed())
        {
            self.status = PhaseStatus::InProgress;
        } else {
            self.status = PhaseStatus::Pending;
        }
    }
}

/// A structured multi-step execution plan for complex programming tasks.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Plan {
    /// Unique plan identifier.
    pub id: String,
    /// Descriptive title of the plan.
    pub title: String,
    /// High-level goal or problem statement to solve.
    pub goal: String,
    /// Optional background context or architectural constraints.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context: Option<String>,
    /// Ordered list of phases.
    pub phases: Vec<Phase>,
    /// Current lifecycle state of the plan.
    pub state: PlanState,
    /// Checkpoint policy governing confirmations.
    pub checkpoint_policy: CheckpointPolicy,
    /// ISO 8601 timestamp when plan was created.
    pub created_at: String,
    /// ISO 8601 timestamp when plan was last updated.
    pub updated_at: String,
    /// Arbitrary metadata key-values.
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub metadata: HashMap<String, String>,
}

impl Plan {
    /// Creates a new Plan with a freshly generated UUID.
    pub fn new(
        title: impl Into<String>,
        goal: impl Into<String>,
        policy: CheckpointPolicy,
    ) -> Self {
        let now = Utc::now().to_rfc3339();
        Self {
            id: Uuid::new_v4().to_string(),
            title: title.into(),
            goal: goal.into(),
            context: None,
            phases: Vec::new(),
            state: PlanState::Draft,
            checkpoint_policy: policy,
            created_at: now.clone(),
            updated_at: now,
            metadata: HashMap::new(),
        }
    }

    /// Sets optional context.
    pub fn with_context(mut self, context: impl Into<String>) -> Self {
        self.context = Some(context.into());
        self
    }

    /// Appends a phase to the plan.
    pub fn add_phase(&mut self, phase: Phase) {
        self.phases.push(phase);
        self.updated_at = Utc::now().to_rfc3339();
    }

    /// Returns the total number of steps across all phases.
    pub fn total_steps(&self) -> usize {
        self.phases.iter().map(|p| p.steps.len()).sum()
    }

    /// Returns the number of completed or skipped steps.
    pub fn completed_steps(&self) -> usize {
        self.phases
            .iter()
            .flat_map(|p| p.steps.iter())
            .filter(|s| s.status.is_completed() || s.status.is_skipped())
            .count()
    }

    /// Returns `(completed_steps, total_steps, percentage_completed)`.
    pub fn progress(&self) -> (usize, usize, f32) {
        let total = self.total_steps();
        let completed = self.completed_steps();
        let pct = if total == 0 {
            0.0
        } else {
            (completed as f32 / total as f32) * 100.0
        };
        (completed, total, pct)
    }

    /// Checks if all steps in the entire plan have succeeded.
    pub fn is_completed(&self) -> bool {
        let total = self.total_steps();
        total > 0 && self.phases.iter().all(|p| p.is_completed())
    }

    /// Checks if any step in the entire plan failed.
    pub fn has_failures(&self) -> bool {
        self.phases.iter().any(|p| p.has_failures())
    }

    /// Returns flat list of references to all steps.
    pub fn all_steps(&self) -> Vec<&PlanStep> {
        self.phases.iter().flat_map(|p| p.steps.iter()).collect()
    }

    /// Returns flat list of mutable references to all steps.
    pub fn all_steps_mut(&mut self) -> Vec<&mut PlanStep> {
        self.phases
            .iter_mut()
            .flat_map(|p| p.steps.iter_mut())
            .collect()
    }

    /// Finds a step by ID anywhere in the plan.
    pub fn find_step(&self, step_id: &str) -> Option<&PlanStep> {
        for phase in &self.phases {
            if let Some(step) = phase.find_step(step_id) {
                return Some(step);
            }
        }
        None
    }

    /// Finds a mutable step by ID anywhere in the plan.
    pub fn find_step_mut(&mut self, step_id: &str) -> Option<&mut PlanStep> {
        let phase_idx = self
            .phases
            .iter()
            .position(|p| p.find_step(step_id).is_some())?;
        let step = self.phases[phase_idx].find_step_mut(step_id);
        self.updated_at = Utc::now().to_rfc3339();
        step
    }

    /// Finds a phase by ID.
    pub fn find_phase(&self, phase_id: &str) -> Option<&Phase> {
        self.phases.iter().find(|p| p.id == phase_id)
    }

    /// Finds a mutable phase by ID.
    pub fn find_phase_mut(&mut self, phase_id: &str) -> Option<&mut Phase> {
        let phase = self.phases.iter_mut().find(|p| p.id == phase_id)?;
        self.updated_at = Utc::now().to_rfc3339();
        Some(phase)
    }

    /// Checks if a step's dependencies are all satisfied (completed or skipped).
    pub fn are_dependencies_satisfied(&self, step: &PlanStep) -> (bool, Vec<String>) {
        if step.dependencies.is_empty() {
            return (true, Vec::new());
        }

        let mut unsatisfied = Vec::new();
        for dep_id in &step.dependencies {
            match self.find_step(dep_id) {
                Some(dep_step) => {
                    if !dep_step.status.is_completed() && !dep_step.status.is_skipped() {
                        unsatisfied.push(dep_id.clone());
                    }
                }
                None => {
                    // Referenced nonexistent dependency
                    unsatisfied.push(format!("{}(missing)", dep_id));
                }
            }
        }

        let is_ok = unsatisfied.is_empty();
        (is_ok, unsatisfied)
    }

    /// Finds the next ready, unblocked step to execute.
    pub fn next_executable_step(&self) -> Option<(&Phase, &PlanStep)> {
        for phase in &self.phases {
            if phase.status.is_completed() || phase.status.is_skipped() {
                continue;
            }
            for step in &phase.steps {
                if step.status.is_pending() {
                    let (satisfied, _) = self.are_dependencies_satisfied(step);
                    if satisfied {
                        return Some((phase, step));
                    }
                }
            }
        }
        None
    }

    /// Evaluates whether a confirmation checkpoint should trigger for the given step.
    pub fn should_checkpoint_for_step(&self, phase: &Phase, step: &PlanStep) -> bool {
        match self.checkpoint_policy {
            CheckpointPolicy::AlwaysConfirm => true,
            CheckpointPolicy::PhaseBoundary => {
                // If it's the first pending step in a phase or marked explicit
                let is_first = phase
                    .steps
                    .first()
                    .map(|s| s.id == step.id)
                    .unwrap_or(false);
                is_first || step.requires_confirmation || phase.requires_confirmation
            }
            CheckpointPolicy::RiskBased => {
                step.risk_level.is_high_or_critical() || step.requires_confirmation
            }
            CheckpointPolicy::ExplicitOnly => {
                step.requires_confirmation || phase.requires_confirmation
            }
            CheckpointPolicy::AutoApprove => false,
        }
    }

    /// Evaluates whether a confirmation checkpoint should trigger before entering a phase.
    pub fn should_checkpoint_for_phase(&self, phase: &Phase) -> bool {
        match self.checkpoint_policy {
            CheckpointPolicy::AlwaysConfirm | CheckpointPolicy::PhaseBoundary => true,
            CheckpointPolicy::ExplicitOnly => phase.requires_confirmation,
            CheckpointPolicy::RiskBased => {
                phase.requires_confirmation
                    || phase
                        .steps
                        .iter()
                        .any(|s| s.risk_level.is_high_or_critical())
            }
            CheckpointPolicy::AutoApprove => false,
        }
    }

    /// Synchronizes all phase statuses and overall plan state.
    pub fn sync_state(&mut self) {
        for phase in &mut self.phases {
            phase.update_status();
        }

        if self.is_completed() {
            self.state = PlanState::Completed;
        } else if self.has_failures() {
            self.state = PlanState::Failed;
        } else if self
            .phases
            .iter()
            .any(|p| p.status == PhaseStatus::InProgress)
        {
            self.state = PlanState::Executing;
        }

        self.updated_at = Utc::now().to_rfc3339();
    }

    /// Inserts a new step into a specific phase.
    pub fn add_step_to_phase(&mut self, phase_id: &str, step: PlanStep) -> anyhow::Result<()> {
        let phase = self
            .find_phase_mut(phase_id)
            .ok_or_else(|| anyhow::anyhow!("Phase '{}' not found", phase_id))?;
        phase.add_step(step);
        self.sync_state();
        Ok(())
    }

    /// Removes a step by ID from whichever phase contains it.
    pub fn remove_step(&mut self, step_id: &str) -> bool {
        for phase in &mut self.phases {
            if let Some(pos) = phase.steps.iter().position(|s| s.id == step_id) {
                phase.steps.remove(pos);
                self.sync_state();
                return true;
            }
        }
        false
    }

    /// Renders an interactive CLI checklist representation.
    pub fn render_checklist(&self) -> String {
        let (completed, total, pct) = self.progress();
        let mut out = String::new();

        out.push_str(&format!(
            "📋 Plan: {} [{}/{} completed - {:.1}%]\n",
            self.title, completed, total, pct
        ));
        out.push_str(&format!("🎯 Goal: {}\n", self.goal));
        out.push_str(&format!(
            "⚙️  Policy: {:?} | State: {}\n",
            self.checkpoint_policy, self.state
        ));
        out.push_str("━".repeat(60).as_str());
        out.push('\n');

        if self.phases.is_empty() {
            out.push_str("  (No phases defined in plan)\n");
            return out;
        }

        for (idx, phase) in self.phases.iter().enumerate() {
            let (p_done, p_total) = phase.progress();
            let cp_badge = if phase.requires_confirmation {
                " ⏸ [Checkpoint Required]"
            } else {
                ""
            };

            out.push_str(&format!(
                "\nPhase {}: {} ({}/{}) - {}{}\n",
                idx + 1,
                phase.name,
                p_done,
                p_total,
                phase.status,
                cp_badge
            ));

            if !phase.description.is_empty() {
                out.push_str(&format!("  Description: {}\n", phase.description));
            }

            for step in &phase.steps {
                let symbol = step.status.checkbox_symbol();
                let role_tag = match &step.role {
                    Some(r) => format!(" [{}]", r),
                    None => String::new(),
                };
                let risk_tag = match step.risk_level {
                    RiskLevel::Low => "",
                    RiskLevel::Medium => " ⚠️ [MED RISK]",
                    RiskLevel::High => " 🚨 [HIGH RISK]",
                    RiskLevel::Critical => " 🔥 [CRITICAL RISK]",
                };
                let step_cp = if step.requires_confirmation {
                    " ⏸ [Confirm]"
                } else {
                    ""
                };

                out.push_str(&format!(
                    "  {} {}: {}{}{}{}\n",
                    symbol, step.id, step.title, role_tag, risk_tag, step_cp
                ));

                if !step.targeted_files.is_empty() {
                    out.push_str(&format!(
                        "     Target files: {}\n",
                        step.targeted_files.join(", ")
                    ));
                }

                if let StepStatus::Failed { error, .. } = &step.status {
                    out.push_str(&format!("     ✗ Error: {}\n", error));
                }
                if let StepStatus::Completed {
                    result: Some(res), ..
                } = &step.status
                {
                    let preview = res.lines().next().unwrap_or("Done");
                    out.push_str(&format!("     ✓ Result: {}\n", preview));
                }
            }
        }

        out
    }

    /// Formats a single-line summary string for REPL status bar.
    pub fn render_summary_line(&self) -> String {
        let (completed, total, pct) = self.progress();
        format!(
            "Plan [{}]: {}/{} steps ({:.0}%) - {}",
            self.title, completed, total, pct, self.state
        )
    }

    /// Converts the plan into a standard Markdown checklist document.
    pub fn to_markdown(&self) -> String {
        let mut md = String::new();
        md.push_str(&format!("# Plan: {}\n\n", self.title));
        md.push_str(&format!("**Goal:** {}\n\n", self.goal));

        if let Some(ctx) = &self.context {
            md.push_str(&format!("**Context:** {}\n\n", ctx));
        }

        let (completed, total, pct) = self.progress();
        md.push_str(&format!(
            "**Progress:** {}/{} steps completed ({:.1}%)\n\n",
            completed, total, pct
        ));

        for phase in &self.phases {
            md.push_str(&format!("## Phase: {}\n", phase.name));
            if !phase.description.is_empty() {
                md.push_str(&format!("{}\n\n", phase.description));
            }

            for step in &phase.steps {
                let check = if step.status.is_completed() { "x" } else { " " };
                let role = match &step.role {
                    Some(r) => format!(" `{}`", r),
                    None => String::new(),
                };
                md.push_str(&format!(
                    "- [{}] **{}**{}: {}\n",
                    check, step.id, role, step.title
                ));

                if !step.description.is_empty() && step.description != step.title {
                    md.push_str(&format!("  - Details: {}\n", step.description));
                }
                if !step.targeted_files.is_empty() {
                    md.push_str(&format!(
                        "  - Files: {}\n",
                        step.targeted_files
                            .iter()
                            .map(|f| format!("`{}`", f))
                            .collect::<Vec<_>>()
                            .join(", ")
                    ));
                }
            }
            md.push('\n');
        }

        md
    }

    /// Parses a structured plan from a Markdown checklist document.
    pub fn from_markdown(content: &str) -> anyhow::Result<Self> {
        let mut title = "Execution Plan".to_string();
        let mut goal = "Complete tasks".to_string();
        let mut phases: Vec<Phase> = Vec::new();
        let mut current_phase: Option<Phase> = None;
        let mut step_counter = 1;

        for line in content.lines() {
            let trimmed = line.trim();
            if trimmed.starts_with("# Plan:") {
                title = trimmed.trim_start_matches("# Plan:").trim().to_string();
            } else if trimmed.starts_with("**Goal:**") {
                goal = trimmed.trim_start_matches("**Goal:**").trim().to_string();
            } else if trimmed.starts_with("## Phase:")
                || trimmed.starts_with("## Phase ")
                || trimmed.starts_with("## ")
            {
                if let Some(phase) = current_phase.take() {
                    phases.push(phase);
                }
                let raw_name = trimmed.trim_start_matches('#').trim();
                let phase_name = raw_name
                    .strip_prefix("Phase:")
                    .or_else(|| raw_name.strip_prefix("Phase"))
                    .unwrap_or(raw_name)
                    .trim()
                    .to_string();

                let phase_id = format!("phase-{}", phases.len() + 1);
                current_phase = Some(Phase::new(phase_id, phase_name, ""));
            } else if trimmed.starts_with("- [ ]")
                || trimmed.starts_with("- [x]")
                || trimmed.starts_with("- [X]")
            {
                let is_checked = trimmed.starts_with("- [x]") || trimmed.starts_with("- [X]");
                let rest = trimmed[5..].trim();

                let step_id = format!("step-{}", step_counter);
                step_counter += 1;

                let phase_id = current_phase
                    .as_ref()
                    .map(|p| p.id.clone())
                    .unwrap_or_else(|| "phase-1".to_string());

                let mut step = PlanStep::new(step_id, phase_id, rest, rest);
                if is_checked {
                    step.mark_completed(Some("Completed in checklist".into()), None);
                }

                if current_phase.is_none() {
                    current_phase = Some(Phase::new("phase-1", "Default Phase", ""));
                }

                if let Some(p) = current_phase.as_mut() {
                    p.add_step(step);
                }
            }
        }

        if let Some(phase) = current_phase {
            phases.push(phase);
        }

        let mut plan = Plan::new(title, goal, CheckpointPolicy::default());
        plan.phases = phases;
        plan.sync_state();
        Ok(plan)
    }

    /// Serializes plan to a JSON file.
    pub fn save_to_file(&self, path: &Path) -> anyhow::Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let json = serde_json::to_string_pretty(self)?;
        std::fs::write(path, json)?;
        Ok(())
    }
    /// Saves the plan as a JSON file into the specified directory, returning the path to the saved file.
    pub fn save_to_dir(&self, dir: &Path) -> anyhow::Result<PathBuf> {
        let file_name = format!("plan-{}.json", self.id);
        let path = dir.join(file_name);
        self.save_to_file(&path)?;
        Ok(path)
    }

    /// Loads plan from a JSON file.
    pub fn load_from_file(path: &Path) -> anyhow::Result<Self> {
        let content = std::fs::read_to_string(path)?;
        let plan: Self = serde_json::from_str(&content)?;
        Ok(plan)
    }
}

/// Fluent builder for constructing structured plans programmatically.
pub struct PlanBuilder {
    plan: Plan,
}

impl PlanBuilder {
    /// Starts building a new plan with the specified title and goal.
    pub fn new(title: impl Into<String>, goal: impl Into<String>) -> Self {
        Self {
            plan: Plan::new(title, goal, CheckpointPolicy::default()),
        }
    }

    /// Sets the checkpoint policy.
    pub fn with_policy(mut self, policy: CheckpointPolicy) -> Self {
        self.plan.checkpoint_policy = policy;
        self
    }

    /// Sets optional context.
    pub fn with_context(mut self, context: impl Into<String>) -> Self {
        self.plan.context = Some(context.into());
        self
    }

    /// Adds a phase using a builder closure.
    pub fn phase<F>(mut self, name: impl Into<String>, description: impl Into<String>, f: F) -> Self
    where
        F: FnOnce(PhaseBuilder) -> PhaseBuilder,
    {
        let phase_id = format!("phase-{}", self.plan.phases.len() + 1);
        let builder = PhaseBuilder::new(phase_id, name, description);
        let phase = f(builder).build();
        self.plan.add_phase(phase);
        self
    }

    /// Finalizes and returns the constructed `Plan`.
    pub fn build(mut self) -> Plan {
        self.plan.sync_state();
        self.plan
    }
}

/// Builder for an individual `Phase` in a plan.
pub struct PhaseBuilder {
    phase: Phase,
}

impl PhaseBuilder {
    pub fn new(
        id: impl Into<String>,
        name: impl Into<String>,
        description: impl Into<String>,
    ) -> Self {
        Self {
            phase: Phase::new(id, name, description),
        }
    }

    pub fn require_confirmation(mut self) -> Self {
        self.phase.requires_confirmation = true;
        self
    }

    pub fn step(self, title: impl Into<String>, description: impl Into<String>) -> StepBuilder {
        let step_id = format!("{}-step-{}", self.phase.id, self.phase.steps.len() + 1);
        let step = PlanStep::new(step_id, self.phase.id.clone(), title, description);
        StepBuilder {
            phase_builder: self,
            step,
        }
    }

    pub fn add_step(mut self, step: PlanStep) -> Self {
        self.phase.add_step(step);
        self
    }

    pub fn build(mut self) -> Phase {
        self.phase.update_status();
        self.phase
    }
}

/// Builder for an individual `PlanStep`.
pub struct StepBuilder {
    phase_builder: PhaseBuilder,
    step: PlanStep,
}

impl StepBuilder {
    pub fn with_role(mut self, role: SubagentRole) -> Self {
        self.step.role = Some(role);
        self
    }

    pub fn with_risk(mut self, risk: RiskLevel) -> Self {
        self.step.risk_level = risk;
        self
    }

    pub fn require_confirmation(mut self) -> Self {
        self.step.requires_confirmation = true;
        self
    }

    pub fn add_dependency(mut self, dep: impl Into<String>) -> Self {
        self.step.dependencies.push(dep.into());
        self
    }

    pub fn with_targeted_files(mut self, files: Vec<String>) -> Self {
        self.step.targeted_files = files;
        self
    }

    pub fn with_verification(mut self, criteria: Vec<String>) -> Self {
        self.step.verification_criteria = criteria;
        self
    }

    /// Commits this step into the parent phase builder.
    pub fn done(mut self) -> PhaseBuilder {
        self.phase_builder.phase.add_step(self.step);
        self.phase_builder
    }
}

/// Asynchronous trait for handling confirmation checkpoints.
#[async_trait]
pub trait ConfirmationHandler: Send + Sync {
    /// Invoked when a confirmation checkpoint is reached, pausing execution until a decision is returned.
    async fn handle_checkpoint(&self, checkpoint: &ConfirmationCheckpoint) -> CheckpointDecision;
}

/// Non-interactive confirmation handler that automatically approves all checkpoints.
#[derive(Debug, Default, Clone)]
pub struct AutoApproveHandler;

#[async_trait]
impl ConfirmationHandler for AutoApproveHandler {
    async fn handle_checkpoint(&self, _checkpoint: &ConfirmationCheckpoint) -> CheckpointDecision {
        CheckpointDecision::Approve
    }
}

/// Confirmation handler driven by an asynchronous tokio channel for GUI/TUI/REPL integration.
pub struct ChannelConfirmationHandler {
    sender: mpsc::Sender<(ConfirmationCheckpoint, oneshot::Sender<CheckpointDecision>)>,
}

impl ChannelConfirmationHandler {
    pub fn new(
        sender: mpsc::Sender<(ConfirmationCheckpoint, oneshot::Sender<CheckpointDecision>)>,
    ) -> Self {
        Self { sender }
    }
}

#[async_trait]
impl ConfirmationHandler for ChannelConfirmationHandler {
    async fn handle_checkpoint(&self, checkpoint: &ConfirmationCheckpoint) -> CheckpointDecision {
        let (decision_tx, decision_rx) = oneshot::channel();
        if self
            .sender
            .send((checkpoint.clone(), decision_tx))
            .await
            .is_err()
        {
            // Receiver closed, fallback to aborting safely
            return CheckpointDecision::Abort {
                reason: "Confirmation channel receiver closed".to_string(),
            };
        }

        decision_rx.await.unwrap_or(CheckpointDecision::Abort {
            reason: "Confirmation response canceled".to_string(),
        })
    }
}

/// Interactive terminal CLI confirmation prompt handler.
pub struct CliPromptHandler;

#[async_trait]
impl ConfirmationHandler for CliPromptHandler {
    async fn handle_checkpoint(&self, checkpoint: &ConfirmationCheckpoint) -> CheckpointDecision {
        println!("\n━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
        println!(
            "⏸  CONFIRMATION CHECKPOINT: [{:?}]",
            checkpoint.checkpoint_type
        );
        println!("   Phase: {}", checkpoint.phase_name);
        if let Some(title) = &checkpoint.step_title {
            println!("   Step:  {}", title);
        }
        println!("   Risk:  {}", checkpoint.risk_level);
        if !checkpoint.targeted_files.is_empty() {
            println!("   Files: {}", checkpoint.targeted_files.join(", "));
        }
        if !checkpoint.action_summary.is_empty() {
            println!("   Action: {}", checkpoint.action_summary);
        }
        println!("   {}", checkpoint.prompt);
        println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
        print!("[y] Approve / [s] Skip / [f] Feedback / [n] Abort: ");
        let _ = std::io::Write::flush(&mut std::io::stdout());

        let mut input = String::new();
        if std::io::stdin().read_line(&mut input).is_err() {
            return CheckpointDecision::Abort {
                reason: "Failed to read user input".into(),
            };
        }

        match input.trim().to_lowercase().as_str() {
            "y" | "yes" | "" => CheckpointDecision::Approve,
            "s" | "skip" => CheckpointDecision::Skip {
                reason: "Skipped by user in CLI checkpoint".into(),
            },
            "f" | "feedback" => {
                print!("Enter feedback instructions: ");
                let _ = std::io::Write::flush(&mut std::io::stdout());
                let mut feedback = String::new();
                let _ = std::io::stdin().read_line(&mut feedback);
                CheckpointDecision::ApproveWithFeedback {
                    feedback: feedback.trim().to_string(),
                }
            }
            _ => CheckpointDecision::Abort {
                reason: "Aborted by user in CLI checkpoint".into(),
            },
        }
    }
}

/// High-level events emitted by the Plan Engine during phased execution.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum PlanEvent {
    /// Plan execution initiated.
    PlanStarted { plan_id: String, title: String },
    /// A new phase has begun.
    PhaseStarted { phase_id: String, name: String },
    /// A phase has completed.
    PhaseCompleted { phase_id: String, name: String },
    /// Execution of a step has started.
    StepStarted {
        step_id: String,
        title: String,
        role: Option<String>,
    },
    /// Progress update during step execution.
    StepProgress { step_id: String, message: String },
    /// A step completed successfully.
    StepCompleted {
        step_id: String,
        title: String,
        output: String,
        duration_ms: u64,
    },
    /// A step failed.
    StepFailed {
        step_id: String,
        title: String,
        error: String,
    },
    /// A step was skipped.
    StepSkipped {
        step_id: String,
        title: String,
        reason: String,
    },
    /// A confirmation checkpoint was encountered.
    CheckpointReached { checkpoint: ConfirmationCheckpoint },
    /// A confirmation checkpoint was resolved.
    CheckpointResolved {
        step_id: Option<String>,
        decision: String,
    },
    /// Entire plan finished successfully.
    PlanCompleted {
        total_steps: usize,
        duration_ms: u64,
    },
    /// Plan was aborted.
    PlanAborted { reason: String },
}

/// Trait abstracting individual step execution.
#[async_trait]
pub trait StepExecutor: Send + Sync {
    /// Executes a single plan step, returning the resulting text output or an error.
    async fn execute_step(
        &self,
        plan: &Plan,
        phase: &Phase,
        step: &PlanStep,
        feedback: Option<&str>,
    ) -> anyhow::Result<String>;
}

/// Concrete step executor that delegates each step to an `AgentRunner` execution turn.
pub struct AgentStepExecutor {
    runner: AgentRunner,
}

impl AgentStepExecutor {
    pub fn new(runner: AgentRunner) -> Self {
        Self { runner }
    }
}

#[async_trait]
impl StepExecutor for AgentStepExecutor {
    async fn execute_step(
        &self,
        plan: &Plan,
        phase: &Phase,
        step: &PlanStep,
        feedback: Option<&str>,
    ) -> anyhow::Result<String> {
        let mut session = Session::new(self.runner.config().default_model.clone());

        // Construct role and task context prompt
        let role_desc = step
            .role
            .as_ref()
            .map(|r| r.system_prompt("PlanWorker"))
            .unwrap_or_else(|| "You are an autonomous execution worker.".to_string());

        let feedback_section = match feedback {
            Some(fb) if !fb.trim().is_empty() => format!("\nUSER GUIDANCE & FEEDBACK:\n{}\n", fb),
            _ => String::new(),
        };

        let criteria_section = if !step.verification_criteria.is_empty() {
            format!(
                "\nVERIFICATION CRITERIA:\n{}",
                step.verification_criteria
                    .iter()
                    .map(|c| format!("- {}", c))
                    .collect::<Vec<_>>()
                    .join("\n")
            )
        } else {
            String::new()
        };

        let files_section = if !step.targeted_files.is_empty() {
            format!("\nTARGETED FILES:\n{}", step.targeted_files.join(", "))
        } else {
            String::new()
        };

        let prompt = format!(
            "{role_desc}\n\n\
             PLAN: {}\n\
             GOAL: {}\n\
             CURRENT PHASE: {}\n\
             STEP TITLE: {}\n\
             STEP DESCRIPTION:\n{}\
             {files_section}\
             {criteria_section}\
             {feedback_section}\n\n\
             Execute this step thoroughly. Use tools as needed. Once verified, provide a concise summary.",
            plan.title, plan.goal, phase.name, step.title, step.description
        );

        self.runner.run_turn(&mut session, &prompt).await
    }
}

/// Simulated mock step executor for testing.
#[derive(Default)]
pub struct MockStepExecutor {
    pub outputs: Arc<RwLock<HashMap<String, String>>>,
    pub failure_steps: Arc<RwLock<HashSet<String>>>,
}

impl MockStepExecutor {
    pub fn new() -> Self {
        Self::default()
    }

    pub async fn set_output(&self, step_id: &str, output: &str) {
        self.outputs
            .write()
            .await
            .insert(step_id.to_string(), output.to_string());
    }

    pub async fn set_failing(&self, step_id: &str) {
        self.failure_steps.write().await.insert(step_id.to_string());
    }
}

#[async_trait]
impl StepExecutor for MockStepExecutor {
    async fn execute_step(
        &self,
        _plan: &Plan,
        _phase: &Phase,
        step: &PlanStep,
        _feedback: Option<&str>,
    ) -> anyhow::Result<String> {
        if self.failure_steps.read().await.contains(&step.id) {
            anyhow::bail!("Simulated failure for step: {}", step.id);
        }

        let output = self
            .outputs
            .read()
            .await
            .get(&step.id)
            .cloned()
            .unwrap_or_else(|| format!("Successfully executed step '{}'", step.title));

        Ok(output)
    }
}

/// The core Planning Mode Engine orchestrating phased execution and confirmation checkpoints.
pub struct PlanEngine<E: StepExecutor, H: ConfirmationHandler> {
    executor: E,
    handler: H,
    event_tx: Option<mpsc::UnboundedSender<PlanEvent>>,
}

impl<E: StepExecutor, H: ConfirmationHandler> PlanEngine<E, H> {
    /// Creates a new PlanEngine with the provided executor and confirmation handler.
    pub fn new(executor: E, handler: H) -> Self {
        Self {
            executor,
            handler,
            event_tx: None,
        }
    }

    /// Attaches an event channel for streaming progress updates.
    pub fn with_event_sender(mut self, tx: mpsc::UnboundedSender<PlanEvent>) -> Self {
        self.event_tx = Some(tx);
        self
    }

    fn emit(&self, event: PlanEvent) {
        if let Some(tx) = &self.event_tx {
            let _ = tx.send(event);
        }
    }

    /// Executes the entire plan through all phases with confirmation checkpoints.
    pub async fn execute_plan(&self, plan: &mut Plan) -> anyhow::Result<()> {
        let start_time = Instant::now();
        plan.state = PlanState::Executing;
        plan.updated_at = Utc::now().to_rfc3339();

        self.emit(PlanEvent::PlanStarted {
            plan_id: plan.id.clone(),
            title: plan.title.clone(),
        });

        for phase_idx in 0..plan.phases.len() {
            // Check if phase is already completed
            if plan.phases[phase_idx].status.is_completed()
                || plan.phases[phase_idx].status.is_skipped()
            {
                continue;
            }

            let phase_id = plan.phases[phase_idx].id.clone();
            let phase_name = plan.phases[phase_idx].name.clone();

            // Evaluate phase-level confirmation checkpoint
            if plan.should_checkpoint_for_phase(&plan.phases[phase_idx]) {
                let checkpoint = ConfirmationCheckpoint {
                    checkpoint_type: CheckpointType::PrePhase,
                    phase_id: phase_id.clone(),
                    phase_name: phase_name.clone(),
                    step_id: None,
                    step_title: None,
                    description: format!("Entering phase: {}", phase_name),
                    risk_level: RiskLevel::Low,
                    targeted_files: Vec::new(),
                    action_summary: format!(
                        "Beginning Phase {}: {} ({} steps)",
                        phase_idx + 1,
                        phase_name,
                        plan.phases[phase_idx].steps.len()
                    ),
                    prompt: format!("Approve beginning Phase '{}'?", phase_name),
                };

                self.emit(PlanEvent::CheckpointReached {
                    checkpoint: checkpoint.clone(),
                });

                plan.state = PlanState::PausedAtCheckpoint;
                let decision = self.handler.handle_checkpoint(&checkpoint).await;
                self.emit(PlanEvent::CheckpointResolved {
                    step_id: None,
                    decision: format!("{:?}", decision),
                });

                match decision {
                    CheckpointDecision::Approve
                    | CheckpointDecision::ApproveWithFeedback { .. } => {
                        plan.state = PlanState::Executing;
                    }
                    CheckpointDecision::Skip { reason } => {
                        plan.phases[phase_idx].status = PhaseStatus::Skipped;
                        for step in &mut plan.phases[phase_idx].steps {
                            if step.status.is_pending() {
                                step.mark_skipped(&reason);
                            }
                        }
                        self.emit(PlanEvent::PhaseCompleted {
                            phase_id: phase_id.clone(),
                            name: phase_name.clone(),
                        });
                        continue;
                    }
                    CheckpointDecision::Retry => {}
                    CheckpointDecision::Abort { reason } => {
                        plan.state = PlanState::Aborted;
                        self.emit(PlanEvent::PlanAborted {
                            reason: reason.clone(),
                        });
                        anyhow::bail!("Plan aborted: {}", reason);
                    }
                }
            }

            plan.phases[phase_idx].status = PhaseStatus::InProgress;
            self.emit(PlanEvent::PhaseStarted {
                phase_id: phase_id.clone(),
                name: phase_name.clone(),
            });

            // Execute each step in the phase
            let step_count = plan.phases[phase_idx].steps.len();
            for step_idx in 0..step_count {
                if plan.phases[phase_idx].steps[step_idx].status.is_terminal() {
                    continue;
                }

                // Check dependencies
                let (deps_ok, missing_deps) =
                    plan.are_dependencies_satisfied(&plan.phases[phase_idx].steps[step_idx]);
                if !deps_ok {
                    let err = format!("Unsatisfied dependencies: {}", missing_deps.join(", "));
                    plan.phases[phase_idx].steps[step_idx].status = StepStatus::Blocked {
                        blocked_by: missing_deps,
                    };
                    self.emit(PlanEvent::StepFailed {
                        step_id: plan.phases[phase_idx].steps[step_idx].id.clone(),
                        title: plan.phases[phase_idx].steps[step_idx].title.clone(),
                        error: err.clone(),
                    });
                    continue;
                }

                let step_id = plan.phases[phase_idx].steps[step_idx].id.clone();
                let step_title = plan.phases[phase_idx].steps[step_idx].title.clone();
                let role_name = plan.phases[phase_idx].steps[step_idx]
                    .role
                    .as_ref()
                    .map(|r| r.to_string());

                // Check step-level confirmation checkpoint
                let mut feedback: Option<String> = None;
                if plan.should_checkpoint_for_step(
                    &plan.phases[phase_idx],
                    &plan.phases[phase_idx].steps[step_idx],
                ) {
                    let cp = ConfirmationCheckpoint {
                        checkpoint_type: CheckpointType::PreStep,
                        phase_id: phase_id.clone(),
                        phase_name: phase_name.clone(),
                        step_id: Some(step_id.clone()),
                        step_title: Some(step_title.clone()),
                        description: plan.phases[phase_idx].steps[step_idx].description.clone(),
                        risk_level: plan.phases[phase_idx].steps[step_idx].risk_level,
                        targeted_files: plan.phases[phase_idx].steps[step_idx]
                            .targeted_files
                            .clone(),
                        action_summary: format!(
                            "Execute step '{}'{}",
                            step_title,
                            role_name
                                .as_ref()
                                .map(|r| format!(" with role {}", r))
                                .unwrap_or_default()
                        ),
                        prompt: format!("Proceed with step '{}'?", step_title),
                    };

                    self.emit(PlanEvent::CheckpointReached {
                        checkpoint: cp.clone(),
                    });

                    plan.state = PlanState::PausedAtCheckpoint;
                    let decision = self.handler.handle_checkpoint(&cp).await;
                    self.emit(PlanEvent::CheckpointResolved {
                        step_id: Some(step_id.clone()),
                        decision: format!("{:?}", decision),
                    });

                    match decision {
                        CheckpointDecision::Approve => {
                            plan.state = PlanState::Executing;
                        }
                        CheckpointDecision::ApproveWithFeedback { feedback: fb } => {
                            plan.state = PlanState::Executing;
                            feedback = Some(fb);
                        }
                        CheckpointDecision::Skip { reason } => {
                            plan.phases[phase_idx].steps[step_idx].mark_skipped(&reason);
                            self.emit(PlanEvent::StepSkipped {
                                step_id: step_id.clone(),
                                title: step_title.clone(),
                                reason,
                            });
                            continue;
                        }
                        CheckpointDecision::Retry => {}
                        CheckpointDecision::Abort { reason } => {
                            plan.state = PlanState::Aborted;
                            self.emit(PlanEvent::PlanAborted {
                                reason: reason.clone(),
                            });
                            anyhow::bail!("Plan aborted by user: {}", reason);
                        }
                    }
                }

                // Execute the step
                plan.phases[phase_idx].steps[step_idx].mark_in_progress();
                self.emit(PlanEvent::StepStarted {
                    step_id: step_id.clone(),
                    title: step_title.clone(),
                    role: role_name,
                });

                let step_start = Instant::now();
                let current_step = plan.phases[phase_idx].steps[step_idx].clone();
                let current_phase = plan.phases[phase_idx].clone();

                let step_result = self
                    .executor
                    .execute_step(plan, &current_phase, &current_step, feedback.as_deref())
                    .await;

                let duration = step_start.elapsed();

                match step_result {
                    Ok(output) => {
                        plan.phases[phase_idx].steps[step_idx]
                            .mark_completed(Some(output.clone()), Some(duration));
                        self.emit(PlanEvent::StepCompleted {
                            step_id: step_id.clone(),
                            title: step_title.clone(),
                            output,
                            duration_ms: duration.as_millis() as u64,
                        });
                    }
                    Err(err) => {
                        let err_msg = err.to_string();
                        plan.phases[phase_idx].steps[step_idx]
                            .mark_failed(&err_msg, Some(duration));
                        self.emit(PlanEvent::StepFailed {
                            step_id: step_id.clone(),
                            title: step_title.clone(),
                            error: err_msg.clone(),
                        });

                        // Check failure recovery checkpoint
                        let failure_cp = ConfirmationCheckpoint {
                            checkpoint_type: CheckpointType::FailureRecovery,
                            phase_id: phase_id.clone(),
                            phase_name: phase_name.clone(),
                            step_id: Some(step_id.clone()),
                            step_title: Some(step_title.clone()),
                            description: format!("Step failed: {}", err_msg),
                            risk_level: RiskLevel::High,
                            targeted_files: plan.phases[phase_idx].steps[step_idx]
                                .targeted_files
                                .clone(),
                            action_summary: "Step failed with error. Choose recovery action."
                                .into(),
                            prompt: "How would you like to handle this failure?".into(),
                        };

                        let decision = self.handler.handle_checkpoint(&failure_cp).await;
                        match decision {
                            CheckpointDecision::Retry => {
                                // Re-run step once
                                plan.phases[phase_idx].steps[step_idx].mark_in_progress();
                                let retry_res = self
                                    .executor
                                    .execute_step(plan, &current_phase, &current_step, None)
                                    .await;
                                if let Ok(retry_output) = retry_res {
                                    plan.phases[phase_idx].steps[step_idx].mark_completed(
                                        Some(retry_output.clone()),
                                        Some(step_start.elapsed()),
                                    );
                                    self.emit(PlanEvent::StepCompleted {
                                        step_id: step_id.clone(),
                                        title: step_title.clone(),
                                        output: retry_output,
                                        duration_ms: duration.as_millis() as u64,
                                    });
                                }
                            }
                            CheckpointDecision::Skip { reason } => {
                                plan.phases[phase_idx].steps[step_idx].mark_skipped(reason);
                            }
                            _ => {
                                plan.state = PlanState::Failed;
                                anyhow::bail!("Plan halted due to step failure: {}", err_msg);
                            }
                        }
                    }
                }
            }

            plan.phases[phase_idx].update_status();
            self.emit(PlanEvent::PhaseCompleted {
                phase_id: phase_id.clone(),
                name: phase_name.clone(),
            });
        }

        plan.sync_state();
        let total_duration = start_time.elapsed().as_millis() as u64;

        if plan.is_completed() {
            plan.state = PlanState::Completed;
            self.emit(PlanEvent::PlanCompleted {
                total_steps: plan.total_steps(),
                duration_ms: total_duration,
            });
        } else if plan.has_failures() {
            plan.state = PlanState::Failed;
        }

        Ok(())
    }
}

/// Automatically generates structured multi-step plans using an LLM.
pub struct PlanGenerator {
    client: LlmClient,
    config: Config,
}

impl PlanGenerator {
    pub fn new(client: LlmClient, config: Config) -> Self {
        Self { client, config }
    }

    /// System prompt enforcing structured JSON plan generation.
    pub fn plan_generation_system_prompt() -> &'static str {
        r#"You are Fusion's Planning Mode Engine. Your mission is to analyze complex programming and engineering tasks, then generate a structured, phased execution plan.

Each plan is composed of logical Phases, and each Phase contains granular, verifiable Steps.

Output MUST be a valid JSON object matching this schema:
{
  "title": "Concise descriptive title of the overall plan",
  "goal": "Clear one-sentence statement of what will be accomplished",
  "context": "Architectural or workspace context",
  "phases": [
    {
      "name": "Phase Name (e.g. Discovery & Analysis)",
      "description": "Objective of this phase",
      "requires_confirmation": false,
      "steps": [
        {
          "title": "Concise imperative step title",
          "description": "Specific details of what to inspect, edit, or test",
          "role": "scout" | "coder" | "tester" | "reviewer",
          "risk_level": "low" | "medium" | "high" | "critical",
          "requires_confirmation": false,
          "dependencies": [],
          "targeted_files": ["path/to/file.rs"],
          "verification_criteria": ["Run cargo check", "Verify function X returns Y"]
        }
      ]
    }
  ]
}

Guidelines:
1. Divide complex tasks into 2-4 logical phases (e.g. 1: Exploration, 2: Core Implementation, 3: Verification).
2. Keep steps granular and testable.
3. Assign appropriate specialist roles: scout (read-only search), coder (writing/editing), tester (tests/benchmarks), reviewer (security/architecture).
4. Identify high-risk operations (e.g., editing critical configs, deleting files, database migrations) and set risk_level to 'high' or 'critical' with requires_confirmation=true.
5. Return ONLY the JSON object. Do not include extraneous markdown commentary outside ```json blocks."#
    }

    /// Generates a structured `Plan` from a user's task prompt.
    pub async fn generate_plan(
        &self,
        task_prompt: &str,
        workspace_context: Option<&str>,
        policy: CheckpointPolicy,
    ) -> anyhow::Result<Plan> {
        let mut user_msg = format!(
            "Please generate a detailed execution plan for this task:\n\n{}",
            task_prompt
        );
        if let Some(ctx) = workspace_context {
            user_msg.push_str(&format!("\n\nWorkspace Context:\n{}", ctx));
        }

        let messages = vec![
            Message::system(Self::plan_generation_system_prompt()),
            Message::user(user_msg),
        ];

        let (response, _, _) = self.client.complete(&self.config, &messages, &[]).await?;
        Self::parse_plan_response(&response, policy)
    }

    /// Parses the LLM's response (handles raw JSON or fenced ```json ... ``` blocks).
    pub fn parse_plan_response(response: &str, policy: CheckpointPolicy) -> anyhow::Result<Plan> {
        let trimmed = response.trim();

        // Extract JSON inside code fences if present
        let json_str = if let Some(start) = trimmed.find("```json") {
            let rest = &trimmed[start + 7..];
            if let Some(end) = rest.find("```") {
                rest[..end].trim()
            } else {
                rest.trim()
            }
        } else if let Some(start) = trimmed.find("```") {
            let rest = &trimmed[start + 3..];
            if let Some(end) = rest.find("```") {
                rest[..end].trim()
            } else {
                rest.trim()
            }
        } else {
            trimmed
        };

        // Attempt JSON parse
        match serde_json::from_str::<Value>(json_str) {
            Ok(v) => Self::convert_json_value_to_plan(v, policy),
            Err(e) => {
                // Fallback: try parsing as markdown checklist
                if trimmed.contains("- [ ]") || trimmed.contains("## Phase") {
                    Plan::from_markdown(trimmed)
                } else {
                    anyhow::bail!(
                        "Failed to parse plan JSON: {}. Response was: {}",
                        e,
                        trimmed
                    );
                }
            }
        }
    }

    fn convert_json_value_to_plan(v: Value, policy: CheckpointPolicy) -> anyhow::Result<Plan> {
        let title = v
            .get("title")
            .and_then(|s| s.as_str())
            .unwrap_or("Generated Execution Plan");
        let goal = v
            .get("goal")
            .and_then(|s| s.as_str())
            .unwrap_or("Execute tasks");
        let context = v.get("context").and_then(|s| s.as_str());

        let mut plan = Plan::new(title, goal, policy);
        if let Some(ctx) = context {
            plan.context = Some(ctx.to_string());
        }

        if let Some(phases_arr) = v.get("phases").and_then(|p| p.as_array()) {
            for (p_idx, p_val) in phases_arr.iter().enumerate() {
                let default_phase_name = format!("Phase {}", p_idx + 1);
                let phase_name = p_val
                    .get("name")
                    .and_then(|s| s.as_str())
                    .unwrap_or(&default_phase_name);
                let phase_desc = p_val
                    .get("description")
                    .and_then(|s| s.as_str())
                    .unwrap_or("");
                let phase_confirm = p_val
                    .get("requires_confirmation")
                    .and_then(|b| b.as_bool())
                    .unwrap_or(false);

                let phase_id = format!("phase-{}", p_idx + 1);
                let mut phase = Phase::new(phase_id.clone(), phase_name, phase_desc);
                phase.requires_confirmation = phase_confirm;

                if let Some(steps_arr) = p_val.get("steps").and_then(|s| s.as_array()) {
                    for (s_idx, s_val) in steps_arr.iter().enumerate() {
                        let step_id = format!("{}-step-{}", phase_id, s_idx + 1);
                        let step_title = s_val
                            .get("title")
                            .and_then(|s| s.as_str())
                            .unwrap_or("Task");
                        let step_desc = s_val
                            .get("description")
                            .and_then(|s| s.as_str())
                            .unwrap_or(step_title);

                        let mut step =
                            PlanStep::new(step_id, phase_id.clone(), step_title, step_desc);

                        if let Some(role_str) = s_val.get("role").and_then(|s| s.as_str()) {
                            step.role = Some(
                                SubagentRole::from_str(role_str).unwrap_or(SubagentRole::General),
                            );
                        }

                        if let Some(risk_str) = s_val.get("risk_level").and_then(|s| s.as_str()) {
                            step.risk_level = match risk_str.to_lowercase().as_str() {
                                "critical" => RiskLevel::Critical,
                                "high" => RiskLevel::High,
                                "medium" => RiskLevel::Medium,
                                _ => RiskLevel::Low,
                            };
                        }

                        if let Some(req_conf) =
                            s_val.get("requires_confirmation").and_then(|b| b.as_bool())
                        {
                            step.requires_confirmation = req_conf;
                        }

                        if let Some(files) = s_val.get("targeted_files").and_then(|f| f.as_array())
                        {
                            step.targeted_files = files
                                .iter()
                                .filter_map(|f| f.as_str().map(|s| s.to_string()))
                                .collect();
                        }

                        if let Some(criteria) = s_val
                            .get("verification_criteria")
                            .and_then(|c| c.as_array())
                        {
                            step.verification_criteria = criteria
                                .iter()
                                .filter_map(|c| c.as_str().map(|s| s.to_string()))
                                .collect();
                        }

                        phase.add_step(step);
                    }
                }

                plan.add_phase(phase);
            }
        }

        plan.sync_state();
        Ok(plan)
    }
}

/// Tool allowing agents to inspect and interact with the active execution plan.
pub struct PlanTool {
    active_plan: Arc<RwLock<Option<Plan>>>,
}

impl PlanTool {
    /// Creates a new PlanTool wrapping a shared active Plan state.
    pub fn new(active_plan: Arc<RwLock<Option<Plan>>>) -> Self {
        Self { active_plan }
    }
}

#[async_trait]
impl Tool for PlanTool {
    fn name(&self) -> &str {
        "manage_plan"
    }

    fn description(&self) -> &str {
        "Inspect, update, or advance the structured multi-step execution plan and checklist."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "action": {
                    "type": "string",
                    "enum": ["status", "checklist", "update_step", "mark_completed", "mark_failed", "next_step"],
                    "description": "The plan action to perform"
                },
                "step_id": {
                    "type": "string",
                    "description": "ID of the step to query or update (e.g. 'phase-1-step-1')"
                },
                "result": {
                    "type": "string",
                    "description": "Output or notes when marking a step completed or failed"
                },
                "reason": {
                    "type": "string",
                    "description": "Reason when skipping a step or explaining failure"
                }
            },
            "required": ["action"]
        })
    }

    async fn execute(&self, args: Value, _ctx: &ToolContext) -> anyhow::Result<String> {
        let action = args
            .get("action")
            .and_then(|a| a.as_str())
            .ok_or_else(|| anyhow::anyhow!("Missing 'action' parameter"))?;

        let mut plan_guard = self.active_plan.write().await;
        let plan = match plan_guard.as_mut() {
            Some(p) => p,
            None => return Ok("No active plan exists. Create a plan first.".to_string()),
        };

        match action {
            "status" => Ok(plan.render_summary_line()),
            "checklist" => Ok(plan.render_checklist()),
            "next_step" => {
                if let Some((phase, step)) = plan.next_executable_step() {
                    Ok(format!(
                        "Next Step:\nID: {}\nPhase: {}\nTitle: {}\nDescription: {}\nRisk: {}",
                        step.id, phase.name, step.title, step.description, step.risk_level
                    ))
                } else if plan.is_completed() {
                    Ok("All steps in the plan are already completed!".to_string())
                } else {
                    Ok(
                        "No pending steps ready for execution (some steps may be blocked)."
                            .to_string(),
                    )
                }
            }
            "mark_completed" => {
                let step_id = args
                    .get("step_id")
                    .and_then(|s| s.as_str())
                    .ok_or_else(|| anyhow::anyhow!("Missing 'step_id' parameter"))?;
                let result = args
                    .get("result")
                    .and_then(|r| r.as_str())
                    .map(|s| s.to_string());

                if let Some(step) = plan.find_step_mut(step_id) {
                    step.mark_completed(result, None);
                    plan.sync_state();
                    Ok(format!("Step '{}' marked as completed.", step_id))
                } else {
                    anyhow::bail!("Step '{}' not found in plan", step_id);
                }
            }
            "mark_failed" => {
                let step_id = args
                    .get("step_id")
                    .and_then(|s| s.as_str())
                    .ok_or_else(|| anyhow::anyhow!("Missing 'step_id' parameter"))?;
                let reason = args
                    .get("reason")
                    .and_then(|r| r.as_str())
                    .unwrap_or("Unknown error");

                if let Some(step) = plan.find_step_mut(step_id) {
                    step.mark_failed(reason, None);
                    plan.sync_state();
                    Ok(format!("Step '{}' marked as failed.", step_id))
                } else {
                    anyhow::bail!("Step '{}' not found in plan", step_id);
                }
            }
            "update_step" => {
                let step_id = args
                    .get("step_id")
                    .and_then(|s| s.as_str())
                    .ok_or_else(|| anyhow::anyhow!("Missing 'step_id' parameter"))?;

                if let Some(step) = plan.find_step_mut(step_id) {
                    if let Some(notes) = args.get("result").and_then(|r| r.as_str()) {
                        step.execution_notes = Some(notes.to_string());
                    }
                    plan.sync_state();
                    Ok(format!("Step '{}' updated successfully.", step_id))
                } else {
                    anyhow::bail!("Step '{}' not found in plan", step_id);
                }
            }
            unknown => anyhow::bail!("Unknown plan action: '{}'", unknown),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_plan_builder_and_progress() {
        let plan = PlanBuilder::new("Refactor Engine", "Modularize codebase")
            .with_policy(CheckpointPolicy::PhaseBoundary)
            .phase("Discovery", "Explore dependencies", |phase| {
                phase
                    .step("Analyze Cargo.toml", "Read dependencies")
                    .with_role(SubagentRole::Scout)
                    .with_risk(RiskLevel::Low)
                    .done()
                    .step("Identify hot paths", "Profile runtime bottlenecks")
                    .with_role(SubagentRole::Scout)
                    .done()
            })
            .phase("Implementation", "Refactor core modules", |phase| {
                phase
                    .step("Create new traits", "Define clean interfaces")
                    .with_role(SubagentRole::Coder)
                    .with_risk(RiskLevel::Medium)
                    .require_confirmation()
                    .done()
                    .step("Cutover callsites", "Migrate all consumers")
                    .with_role(SubagentRole::Coder)
                    .with_risk(RiskLevel::High)
                    .require_confirmation()
                    .done()
            })
            .build();

        assert_eq!(plan.total_steps(), 4);
        assert_eq!(plan.completed_steps(), 0);

        let (completed, total, pct) = plan.progress();
        assert_eq!(completed, 0);
        assert_eq!(total, 4);
        assert_eq!(pct, 0.0);

        // Checkpoint policy evaluation
        let p1 = &plan.phases[0];
        let p2 = &plan.phases[1];
        assert!(plan.should_checkpoint_for_phase(p1));
        assert!(plan.should_checkpoint_for_phase(p2));

        let s1 = &p1.steps[0];
        let s3 = &p2.steps[0];
        // In PhaseBoundary policy, first step of a phase requires confirmation
        assert!(plan.should_checkpoint_for_step(p1, s1));
        assert!(plan.should_checkpoint_for_step(p2, s3));
    }

    #[test]
    fn test_step_dependency_resolution() {
        let mut plan = Plan::new(
            "Deploy App",
            "Deploy service",
            CheckpointPolicy::AlwaysConfirm,
        );
        let mut phase = Phase::new("phase-1", "Deployment", "Setup and deploy");

        let mut step1 = PlanStep::new("step-1", "phase-1", "Build binary", "Run cargo build");
        let step2 = PlanStep::new("step-2", "phase-1", "Deploy binary", "Upload to server")
            .add_dependency("step-1");

        step1.mark_completed(Some("Built target/release/app".into()), None);
        phase.add_step(step1);
        phase.add_step(step2);
        plan.add_phase(phase);

        let (ready, missing) = plan.are_dependencies_satisfied(&plan.phases[0].steps[1]);
        assert!(ready);
        assert!(missing.is_empty());

        let next = plan.next_executable_step();
        assert!(next.is_some());
        assert_eq!(next.unwrap().1.id, "step-2");
    }

    #[test]
    fn test_markdown_checklist_roundtrip() {
        let original = r#"# Plan: Refactor Database
**Goal:** Migrate to pure Rust SQLite

## Phase: Schema Discovery
- [x] **step-1**: Read schema
- [ ] **step-2**: Dump table constraints

## Phase: Migration
- [ ] **step-3**: Apply migration
"#;

        let plan = Plan::from_markdown(original).expect("parse markdown");
        assert_eq!(plan.title, "Refactor Database");
        assert_eq!(plan.goal, "Migrate to pure Rust SQLite");
        assert_eq!(plan.phases.len(), 2);
        assert_eq!(plan.total_steps(), 3);
        assert_eq!(plan.completed_steps(), 1);

        let rendered = plan.render_checklist();
        assert!(rendered.contains("[✓]"));
        assert!(rendered.contains("[ ]"));
        assert!(rendered.contains("Refactor Database"));
    }

    #[tokio::test]
    async fn test_plan_engine_phased_execution_with_checkpoints() {
        let mut plan = PlanBuilder::new("Test Pipeline", "Verify phased execution")
            .with_policy(CheckpointPolicy::AlwaysConfirm)
            .phase("Phase 1", "Initial steps", |p| {
                p.step("Task 1", "Execute first task")
                    .done()
                    .step("Task 2", "Execute second task")
                    .done()
            })
            .build();

        let mock_executor = MockStepExecutor::new();
        mock_executor
            .set_output("phase-1-step-1", "Task 1 completed successfully")
            .await;
        mock_executor
            .set_output("phase-1-step-2", "Task 2 completed successfully")
            .await;

        let auto_handler = AutoApproveHandler;
        let (tx, mut rx) = mpsc::unbounded_channel();
        let engine = PlanEngine::new(mock_executor, auto_handler).with_event_sender(tx);

        let res = engine.execute_plan(&mut plan).await;
        assert!(res.is_ok());
        assert_eq!(plan.state, PlanState::Completed);
        assert!(plan.is_completed());

        // Verify events were emitted
        let mut events = Vec::new();
        while let Ok(evt) = rx.try_recv() {
            events.push(evt);
        }
        assert!(!events.is_empty());
        assert!(events
            .iter()
            .any(|e| matches!(e, PlanEvent::PlanStarted { .. })));
        assert!(events
            .iter()
            .any(|e| matches!(e, PlanEvent::PlanCompleted { .. })));
    }

    #[tokio::test]
    async fn test_plan_engine_step_failure_and_recovery() {
        let mut plan = PlanBuilder::new("Failure Test", "Test recovery")
            .with_policy(CheckpointPolicy::AutoApprove)
            .phase("Phase 1", "Tasks", |p| {
                p.step("Failing Task", "Will fail").done()
            })
            .build();

        let mock_executor = MockStepExecutor::new();
        mock_executor.set_failing("phase-1-step-1").await;

        // Custom handler that skips on failure
        struct SkipOnFailureHandler;
        #[async_trait]
        impl ConfirmationHandler for SkipOnFailureHandler {
            async fn handle_checkpoint(&self, cp: &ConfirmationCheckpoint) -> CheckpointDecision {
                if cp.checkpoint_type == CheckpointType::FailureRecovery {
                    CheckpointDecision::Skip {
                        reason: "Skipping failed test step".into(),
                    }
                } else {
                    CheckpointDecision::Approve
                }
            }
        }

        let engine = PlanEngine::new(mock_executor, SkipOnFailureHandler);
        let res = engine.execute_plan(&mut plan).await;
        assert!(res.is_ok());
        assert!(plan.phases[0].steps[0].status.is_skipped());
    }

    #[tokio::test]
    async fn test_plan_tool_actions() {
        let plan = PlanBuilder::new("Tool Plan", "Inspect tool interactions")
            .phase("P1", "Phase 1", |p| p.step("Step A", "Do work").done())
            .build();

        let active_plan = Arc::new(RwLock::new(Some(plan)));
        let tool = PlanTool::new(active_plan.clone());
        let ctx = ToolContext::default();

        // 1. Status action
        let status = tool
            .execute(json!({"action": "status"}), &ctx)
            .await
            .unwrap();
        assert!(status.contains("Tool Plan"));

        // 2. Checklist action
        let checklist = tool
            .execute(json!({"action": "checklist"}), &ctx)
            .await
            .unwrap();
        assert!(checklist.contains("Step A"));

        // 3. Next step action
        let next = tool
            .execute(json!({"action": "next_step"}), &ctx)
            .await
            .unwrap();
        assert!(next.contains("Step A"));

        // 4. Mark completed action
        let res = tool
            .execute(
                json!({
                    "action": "mark_completed",
                    "step_id": "phase-1-step-1",
                    "result": "Finished nicely"
                }),
                &ctx,
            )
            .await
            .unwrap();
        assert!(res.contains("marked as completed"));

        // Verify updated state in shared plan
        let plan_guard = active_plan.read().await;
        assert!(plan_guard.as_ref().unwrap().is_completed());
    }
}
