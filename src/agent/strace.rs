//! STRACE: Structural Trajectory Analysis and Causal Extraction.
//!
//! Pure-Rust implementation of the STRACE agent optimization and diagnosis framework
//! (Chang et al., arXiv:2607.07702):
//!
//! 1. **Structural Modeling**: Textual Execution Dependency Graph (EDG) mapping data
//!    and control dependency priors across agent components.
//! 2. **Failure Pattern Mining**: Statistical severity and structural path anomalies
//!    (self-loops, oscillations, dead-ends).
//! 3. **Causal Localization**: Backward causal slicing from manifestation node $v_m$
//!    to extract compact causal slice $\mathcal{C}_{slice}$ and isolate root cause $v_r$.
//! 4. **Inductive Policy Optimization**: Synthesis of transferable heuristics for prompt
//!    and skill evolution.

use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};

use crate::agent::session::Session;
use crate::provider::types::Role;

// ============================================================================
// Phase 1: Execution Dependency Graph (EDG)
// ============================================================================

/// Functional classification of a component in an agent system.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ComponentRole {
    /// Planners, routers, orchestrators, dispatchers selecting downstream actions.
    DecisionMaker,
    /// Action agents, tools, code executors, verifiers performing concrete work.
    Executor,
    /// Memory stores, workspace files, persistent session states.
    PassiveState,
    /// Pre-execution safety, architecture, or quality critics.
    Advisor,
}

/// Inter-component dependency relationship in the Execution Dependency Graph.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum DependencyKind {
    /// B consumes an artifact produced by A.
    Data { artifact: String, rationale: String },
    /// A's decision or condition dictates whether/how B executes.
    Control {
        condition: String,
        rationale: String,
    },
}

/// Specification of an atomic component node within the EDG.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ComponentNode {
    pub name: String,
    pub role: ComponentRole,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub produces: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub consumes: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prompt_file: Option<String>,
}

/// Execution Dependency Graph (EDG) serving as the structural dependency prior.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExecutionDependencyGraph {
    pub nodes: HashMap<String, ComponentNode>,
    pub edges: Vec<(String, String, DependencyKind)>, // (source, target, kind)
}

impl ExecutionDependencyGraph {
    /// Creates an empty dependency graph.
    pub fn new() -> Self {
        Self::default()
    }

    /// Constructs the standard default EDG for Fusion's multi-agent architecture.
    pub fn fusion_default() -> Self {
        let mut edg = Self::new();

        edg.add_node(
            "Planner",
            ComponentRole::DecisionMaker,
            vec!["plan".to_string(), "tool_args".to_string()],
            vec!["user_prompt".to_string(), "session_history".to_string()],
            Some("system_prompt".to_string()),
        );
        edg.add_node(
            "AdvisorCommittee",
            ComponentRole::Advisor,
            vec!["critique".to_string(), "approved".to_string()],
            vec!["plan".to_string(), "tool_args".to_string()],
            None,
        );
        edg.add_node(
            "ToolExecutor",
            ComponentRole::Executor,
            vec!["tool_result".to_string(), "file_mutation".to_string()],
            vec!["tool_args".to_string()],
            None,
        );
        edg.add_node(
            "FileSystemState",
            ComponentRole::PassiveState,
            vec!["file_content".to_string()],
            vec!["file_mutation".to_string()],
            None,
        );
        edg.add_node(
            "CompilerVerifier",
            ComponentRole::Executor,
            vec!["diagnostics".to_string(), "compile_success".to_string()],
            vec!["file_mutation".to_string()],
            None,
        );

        // Control & data dependencies
        edg.add_edge(
            "Planner",
            "AdvisorCommittee",
            DependencyKind::Control {
                condition: "advisors_enabled".to_string(),
                rationale: "Planner proposals trigger advisor evaluations".to_string(),
            },
        );
        edg.add_edge(
            "Planner",
            "ToolExecutor",
            DependencyKind::Data {
                artifact: "tool_args".to_string(),
                rationale: "ToolExecutor executes parameters planned by Planner".to_string(),
            },
        );
        edg.add_edge(
            "ToolExecutor",
            "FileSystemState",
            DependencyKind::Data {
                artifact: "file_mutation".to_string(),
                rationale: "File edits mutate workspace state".to_string(),
            },
        );
        edg.add_edge(
            "FileSystemState",
            "CompilerVerifier",
            DependencyKind::Data {
                artifact: "file_content".to_string(),
                rationale: "Compiler verifies code state on disk".to_string(),
            },
        );

        edg
    }

    /// Registers a component node into the graph.
    pub fn add_node(
        &mut self,
        name: impl Into<String>,
        role: ComponentRole,
        produces: Vec<String>,
        consumes: Vec<String>,
        prompt_file: Option<String>,
    ) {
        let name_str = name.into();
        self.nodes.insert(
            name_str.clone(),
            ComponentNode {
                name: name_str,
                role,
                produces,
                consumes,
                prompt_file,
            },
        );
    }

    /// Connects two components with a directed dependency prior edge.
    pub fn add_edge(&mut self, from: &str, to: &str, kind: DependencyKind) {
        self.edges.push((from.to_string(), to.to_string(), kind));
    }

    /// Returns direct upstream dependencies for a target component.
    pub fn upstream_dependencies(&self, target: &str) -> Vec<(&str, &DependencyKind)> {
        self.edges
            .iter()
            .filter(|(_, to, _)| to == target)
            .map(|(from, _, kind)| (from.as_str(), kind))
            .collect()
    }
}

// ============================================================================
// Phase 2: Trajectory Steps & Failure Pattern Mining
// ============================================================================

/// Outcome classification of an individual trajectory step.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StepOutcome {
    Success,
    Failed,
    Timeout,
    Rejected,
    LoopDetected,
}

/// A discrete, ordered execution step extracted from an agent's trace.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TrajectoryStep {
    /// 1-based step sequence index.
    pub position: usize,
    /// Name of the component executing this step (e.g. "Planner", "bash", "edit").
    pub component: String,
    /// Functional role of the component.
    pub role: ComponentRole,
    /// Action summary (e.g. tool name, plan text, or prompt command).
    pub action: String,
    /// Outcome status of this step.
    pub outcome: StepOutcome,
    /// True only if this position committed an accepted change to shared context/files.
    pub state_changed: bool,
    /// Input variables/artifacts consumed.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub inputs: Vec<String>,
    /// Artifacts produced.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub outputs: Vec<String>,
    /// Raw error output or diagnostic message if failed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// Pathological structural topologies detected in multi-step execution.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum PathAnomaly {
    /// Component invoked repeatedly without state progression.
    SelfLoop {
        component: String,
        start_pos: usize,
        repetitions: usize,
    },
    /// Execution oscillating between two components without progress.
    Oscillation {
        components: (String, String),
        start_pos: usize,
        cycle_count: usize,
    },
    /// Repeated failure sequence leading to abrupt turn abort.
    DeadEnd {
        component: String,
        terminal_pos: usize,
        error: String,
    },
}

/// Detects recurring pathological topologies across trajectory steps.
pub fn detect_path_anomalies(steps: &[TrajectoryStep]) -> Vec<PathAnomaly> {
    let mut anomalies = Vec::new();
    if steps.len() < 2 {
        return anomalies;
    }

    // 1. Detect Self-Loops (>= 3 consecutive attempts without state change)
    let mut i = 0;
    while i < steps.len() {
        if steps[i].state_changed {
            i += 1;
            continue;
        }
        let comp = &steps[i].component;
        let mut count = 1;
        while i + count < steps.len()
            && &steps[i + count].component == comp
            && !steps[i + count].state_changed
        {
            count += 1;
        }
        if count >= 3 {
            anomalies.push(PathAnomaly::SelfLoop {
                component: comp.clone(),
                start_pos: steps[i].position,
                repetitions: count,
            });
            i += count;
        } else {
            i += 1;
        }
    }

    // 2. Detect Oscillations (e.g. A -> B -> A -> B with no state change)
    if steps.len() >= 4 {
        for window_start in 0..(steps.len() - 3) {
            let a = &steps[window_start].component;
            let b = &steps[window_start + 1].component;
            if a != b
                && &steps[window_start + 2].component == a
                && &steps[window_start + 3].component == b
                && !steps[window_start + 2].state_changed
                && !steps[window_start + 3].state_changed
            {
                anomalies.push(PathAnomaly::Oscillation {
                    components: (a.clone(), b.clone()),
                    start_pos: steps[window_start].position,
                    cycle_count: 2,
                });
                break;
            }
        }
    }

    // 3. Detect Dead-End terminal failures
    if let Some(last) = steps.last() {
        if last.outcome == StepOutcome::Failed || last.outcome == StepOutcome::Timeout {
            if let Some(err) = &last.error {
                anomalies.push(PathAnomaly::DeadEnd {
                    component: last.component.clone(),
                    terminal_pos: last.position,
                    error: err.clone(),
                });
            }
        }
    }

    anomalies
}

// ============================================================================
// Phase 3: Causal Localization & Backward Slicing
// ============================================================================

/// Maximal run of consecutive target positions sharing identical state context.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CausalSegment {
    /// Positions encompassed in this contiguous segment.
    pub segment_positions: Vec<usize>,
    /// The representative (first) position where the symptom manifested in this segment.
    pub representative_pos: usize,
    /// Chronological causal chain of state-changing positions leading up to representative.
    pub causal_chain: Vec<usize>,
}

/// Noise-pruned causal slice capturing only causally relevant execution steps.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CausalSlice {
    pub target_component: String,
    pub manifestation_pos: usize,
    pub segments: Vec<CausalSegment>,
    pub causal_steps: Vec<TrajectoryStep>,
    pub pruned_step_positions: Vec<usize>,
}

/// Attributed root cause connecting the manifestation symptom to its logical origin.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CausalAttribution {
    pub manifestation_node: String,
    pub manifestation_pos: usize,
    pub root_cause_node: String,
    pub root_cause_pos: usize,
    pub reason: String,
    pub suggested_heuristic: String,
    pub causal_chain: Vec<usize>,
    pub pruned_steps_count: usize,
}

/// Pure-Rust Backward Causal Slicer.
pub struct BackwardCausalSlicer;

impl BackwardCausalSlicer {
    /// Performs backward slicing on a trajectory for a given manifestation component.
    pub fn slice(
        steps: &[TrajectoryStep],
        manifestation_comp: &str,
    ) -> Result<CausalSlice, String> {
        if steps.is_empty() {
            return Err("Cannot slice empty trajectory".to_string());
        }

        // 1. Identify state-changing positions
        let state_changing_positions: Vec<usize> = steps
            .iter()
            .filter(|s| s.state_changed)
            .map(|s| s.position)
            .collect();

        // 2. Locate manifestation positions where target failed or did not change state
        let manifestation_positions: Vec<usize> = steps
            .iter()
            .filter(|s| {
                s.component == manifestation_comp
                    && (!s.state_changed || s.outcome == StepOutcome::Failed)
            })
            .map(|s| s.position)
            .collect();

        if manifestation_positions.is_empty() {
            // Fallback: take the final step if no explicit non-state-changing target step found
            let last_pos = steps.last().unwrap().position;
            return Self::build_slice_for_single_target(
                steps,
                manifestation_comp,
                last_pos,
                &state_changing_positions,
            );
        }

        // 3. Group manifestation positions into causal segments
        let mut segments = Vec::new();
        let mut current_segment: Vec<usize> = Vec::new();

        for pos in &manifestation_positions {
            if current_segment.is_empty() {
                current_segment.push(*pos);
            } else {
                let prev_pos = *current_segment.last().unwrap();
                // Check if any state change occurred between prev_pos and pos
                let state_changed_between = state_changing_positions
                    .iter()
                    .any(|sc| *sc > prev_pos && *sc < *pos);
                if state_changed_between {
                    // Start new segment
                    let rep = current_segment[0];
                    let chain = Self::build_chain_for_pos(rep, &state_changing_positions);
                    segments.push(CausalSegment {
                        segment_positions: current_segment,
                        representative_pos: rep,
                        causal_chain: chain,
                    });
                    current_segment = vec![*pos];
                } else {
                    current_segment.push(*pos);
                }
            }
        }

        if !current_segment.is_empty() {
            let rep = current_segment[0];
            let chain = Self::build_chain_for_pos(rep, &state_changing_positions);
            segments.push(CausalSegment {
                segment_positions: current_segment,
                representative_pos: rep,
                causal_chain: chain,
            });
        }

        // 4. Collect unique causal positions across all segments
        let mut causal_set: HashSet<usize> = HashSet::new();
        for seg in &segments {
            for pos in &seg.causal_chain {
                causal_set.insert(*pos);
            }
        }

        let mut causal_steps: Vec<TrajectoryStep> = Vec::new();
        let mut pruned_positions: Vec<usize> = Vec::new();

        for step in steps {
            if causal_set.contains(&step.position) {
                causal_steps.push(step.clone());
            } else {
                pruned_step_positions_push(&mut pruned_positions, step.position);
            }
        }

        let rep_manifest_pos = segments
            .first()
            .map(|s| s.representative_pos)
            .unwrap_or_else(|| steps.last().unwrap().position);

        Ok(CausalSlice {
            target_component: manifestation_comp.to_string(),
            manifestation_pos: rep_manifest_pos,
            segments,
            causal_steps,
            pruned_step_positions: pruned_positions,
        })
    }

    fn build_chain_for_pos(pos: usize, state_changing: &[usize]) -> Vec<usize> {
        let mut chain: Vec<usize> = state_changing
            .iter()
            .filter(|sc| **sc < pos)
            .cloned()
            .collect();
        chain.push(pos);
        chain
    }

    fn build_slice_for_single_target(
        steps: &[TrajectoryStep],
        comp: &str,
        target_pos: usize,
        state_changing: &[usize],
    ) -> Result<CausalSlice, String> {
        let chain = Self::build_chain_for_pos(target_pos, state_changing);
        let causal_set: HashSet<usize> = chain.iter().cloned().collect();

        let mut causal_steps = Vec::new();
        let mut pruned = Vec::new();
        for step in steps {
            if causal_set.contains(&step.position) {
                causal_steps.push(step.clone());
            } else {
                pruned.push(step.position);
            }
        }

        Ok(CausalSlice {
            target_component: comp.to_string(),
            manifestation_pos: target_pos,
            segments: vec![CausalSegment {
                segment_positions: vec![target_pos],
                representative_pos: target_pos,
                causal_chain: chain,
            }],
            causal_steps,
            pruned_step_positions: pruned,
        })
    }

    /// Converts active conversation turns inside a `Session` into structured trajectory steps.
    pub fn extract_steps_from_session(session: &Session) -> Vec<TrajectoryStep> {
        let mut steps = Vec::new();
        let mut pos = 1;

        for msg in &session.messages {
            match &msg.role {
                Role::User => {
                    steps.push(TrajectoryStep {
                        position: pos,
                        component: "User".to_string(),
                        role: ComponentRole::PassiveState,
                        action: "user_turn_prompt".to_string(),
                        outcome: StepOutcome::Success,
                        state_changed: true,
                        inputs: Vec::new(),
                        outputs: vec!["user_intent".to_string()],
                        error: None,
                    });
                    pos += 1;
                }
                Role::Assistant => {
                    let has_tools = msg.tool_calls.as_ref().is_some_and(|tc| !tc.is_empty());
                    steps.push(TrajectoryStep {
                        position: pos,
                        component: "Planner".to_string(),
                        role: ComponentRole::DecisionMaker,
                        action: if has_tools {
                            "plan_tool_call".to_string()
                        } else {
                            "response_message".to_string()
                        },
                        outcome: StepOutcome::Success,
                        state_changed: false,
                        inputs: vec!["session_history".to_string()],
                        outputs: vec!["plan".to_string()],
                        error: None,
                    });
                    pos += 1;

                    if let Some(tool_calls) = &msg.tool_calls {
                        for tc in tool_calls {
                            steps.push(TrajectoryStep {
                                position: pos,
                                component: tc.name.clone(),
                                role: ComponentRole::Executor,
                                action: format!("call_{}", tc.name),
                                outcome: StepOutcome::Success,
                                state_changed: false,
                                inputs: vec![tc.arguments.clone()],
                                outputs: Vec::new(),
                                error: None,
                            });
                            pos += 1;
                        }
                    }
                }
                Role::Tool => {
                    let is_error = msg.content.contains("Error") || msg.content.contains("failed");
                    let state_changed = !is_error
                        && (msg.content.contains("Successfully wrote")
                            || msg.content.contains("Successfully edited"));

                    steps.push(TrajectoryStep {
                        position: pos,
                        component: "ToolResult".to_string(),
                        role: ComponentRole::Executor,
                        action: "tool_execution_result".to_string(),
                        outcome: if is_error {
                            StepOutcome::Failed
                        } else {
                            StepOutcome::Success
                        },
                        state_changed,
                        inputs: Vec::new(),
                        outputs: vec!["tool_output".to_string()],
                        error: if is_error {
                            Some(msg.content.clone())
                        } else {
                            None
                        },
                    });
                    pos += 1;
                }
                Role::System => {}
            }
        }

        steps
    }
}

fn pruned_step_positions_push(vec: &mut Vec<usize>, val: usize) {
    vec.push(val);
}

// ============================================================================
// Phase 3b: Root Cause Attribution Engine
// ============================================================================

/// Evaluates causal slices to isolate the upstream root cause from manifestation symptoms.
pub struct RootCauseAttributor;

impl RootCauseAttributor {
    /// Attributes a failure in a causal slice to its root-cause node.
    pub fn attribute(slice: &CausalSlice, edg: &ExecutionDependencyGraph) -> CausalAttribution {
        let manifest_node = &slice.target_component;
        let manifest_pos = slice.manifestation_pos;

        // 1. Look back across causal steps for the proximate upstream DecisionMaker
        let mut root_cause_node = manifest_node.clone();
        let mut root_cause_pos = manifest_pos;
        let mut reason = format!(
            "Downstream component '{}' encountered failure.",
            manifest_node
        );
        let mut heuristic = "Review component execution approach.".to_string();

        let causal_steps = &slice.causal_steps;
        let manifest_idx = causal_steps
            .iter()
            .position(|s| s.position == manifest_pos)
            .unwrap_or(causal_steps.len().saturating_sub(1));

        // Trace backward from manifestation step
        for i in (0..manifest_idx).rev() {
            let step = &causal_steps[i];

            // If an upstream DecisionMaker selected this executor or generated parameters
            if step.role == ComponentRole::DecisionMaker {
                root_cause_node = step.component.clone();
                root_cause_pos = step.position;
                reason = format!(
                    "Upstream DecisionMaker '{}' at Step #{} supplied invalid parameters or misrouted to '{}'.",
                    step.component, step.position, manifest_node
                );
                heuristic = format!(
                    "Verify input arguments and prerequisite assumptions in '{}' before invoking downstream tool '{}'.",
                    step.component, manifest_node
                );
                break;
            }

            // Or if an upstream state-committing node corrupted shared context
            if step.state_changed && step.position < manifest_pos {
                root_cause_node = step.component.clone();
                root_cause_pos = step.position;
                reason = format!(
                    "Upstream state-committing node '{}' at Step #{} committed state that invalidated downstream execution.",
                    step.component, step.position
                );
                heuristic = format!(
                    "Validate invariant consistency in '{}' prior to committing state changes.",
                    step.component
                );
                break;
            }
        }

        // If the EDG specifies an upstream control dependency, verify attribution alignment
        let upstreams = edg.upstream_dependencies(manifest_node);
        for (up_name, dep) in upstreams {
            if let DependencyKind::Control { rationale, .. } = dep {
                if root_cause_node == *manifest_node {
                    root_cause_node = up_name.to_string();
                    reason = format!(
                        "Control dependency indicates upstream '{}' governs '{}': {}",
                        up_name, manifest_node, rationale
                    );
                }
            }
        }

        let full_chain: Vec<usize> = slice
            .segments
            .iter()
            .flat_map(|s| s.causal_chain.iter().cloned())
            .collect();

        CausalAttribution {
            manifestation_node: manifest_node.clone(),
            manifestation_pos: manifest_pos,
            root_cause_node,
            root_cause_pos,
            reason,
            suggested_heuristic: heuristic,
            causal_chain: full_chain,
            pruned_steps_count: slice.pruned_step_positions.len(),
        }
    }

    /// Convenience helper to run full end-to-end STRACE diagnosis directly on a `Session`.
    pub fn diagnose_session(session: &Session) -> Result<CausalAttribution, String> {
        let steps = BackwardCausalSlicer::extract_steps_from_session(session);
        if steps.is_empty() {
            return Err("Session has no trajectory steps to analyze".to_string());
        }

        // Find the last failed component or fallback to the last step
        let manifest_comp = steps
            .iter()
            .rev()
            .find(|s| s.outcome == StepOutcome::Failed)
            .map(|s| s.component.as_str())
            .unwrap_or_else(|| steps.last().unwrap().component.as_str());

        let slice = BackwardCausalSlicer::slice(&steps, manifest_comp)?;
        let edg = ExecutionDependencyGraph::fusion_default();
        Ok(Self::attribute(&slice, &edg))
    }
}

// ============================================================================
// Unit Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_edg_construction_and_lookup() {
        let edg = ExecutionDependencyGraph::fusion_default();
        assert!(edg.nodes.contains_key("Planner"));
        assert!(edg.nodes.contains_key("ToolExecutor"));
        assert!(edg.nodes.contains_key("AdvisorCommittee"));

        let upstreams = edg.upstream_dependencies("ToolExecutor");
        assert!(upstreams.iter().any(|(name, _)| *name == "Planner"));
    }

    #[test]
    fn test_detect_self_loop_anomaly() {
        let steps = vec![
            TrajectoryStep {
                position: 1,
                component: "edit".to_string(),
                role: ComponentRole::Executor,
                action: "edit_file".to_string(),
                outcome: StepOutcome::Failed,
                state_changed: false,
                inputs: Vec::new(),
                outputs: Vec::new(),
                error: Some("File locked".to_string()),
            },
            TrajectoryStep {
                position: 2,
                component: "edit".to_string(),
                role: ComponentRole::Executor,
                action: "edit_file".to_string(),
                outcome: StepOutcome::Failed,
                state_changed: false,
                inputs: Vec::new(),
                outputs: Vec::new(),
                error: Some("File locked".to_string()),
            },
            TrajectoryStep {
                position: 3,
                component: "edit".to_string(),
                role: ComponentRole::Executor,
                action: "edit_file".to_string(),
                outcome: StepOutcome::Failed,
                state_changed: false,
                inputs: Vec::new(),
                outputs: Vec::new(),
                error: Some("File locked".to_string()),
            },
        ];

        let anomalies = detect_path_anomalies(&steps);
        assert_eq!(anomalies.len(), 2); // 1 SelfLoop, 1 DeadEnd
        match &anomalies[0] {
            PathAnomaly::SelfLoop {
                component,
                repetitions,
                ..
            } => {
                assert_eq!(component, "edit");
                assert_eq!(*repetitions, 3);
            }
            _ => panic!("Expected SelfLoop anomaly"),
        }
    }

    #[test]
    fn test_detect_oscillation_anomaly() {
        let steps = vec![
            TrajectoryStep {
                position: 1,
                component: "grep".to_string(),
                role: ComponentRole::Executor,
                action: "search".to_string(),
                outcome: StepOutcome::Success,
                state_changed: false,
                inputs: Vec::new(),
                outputs: Vec::new(),
                error: None,
            },
            TrajectoryStep {
                position: 2,
                component: "read".to_string(),
                role: ComponentRole::Executor,
                action: "read".to_string(),
                outcome: StepOutcome::Success,
                state_changed: false,
                inputs: Vec::new(),
                outputs: Vec::new(),
                error: None,
            },
            TrajectoryStep {
                position: 3,
                component: "grep".to_string(),
                role: ComponentRole::Executor,
                action: "search".to_string(),
                outcome: StepOutcome::Success,
                state_changed: false,
                inputs: Vec::new(),
                outputs: Vec::new(),
                error: None,
            },
            TrajectoryStep {
                position: 4,
                component: "read".to_string(),
                role: ComponentRole::Executor,
                action: "read".to_string(),
                outcome: StepOutcome::Success,
                state_changed: false,
                inputs: Vec::new(),
                outputs: Vec::new(),
                error: None,
            },
        ];

        let anomalies = detect_path_anomalies(&steps);
        assert_eq!(anomalies.len(), 1);
        match &anomalies[0] {
            PathAnomaly::Oscillation { components, .. } => {
                assert_eq!(components.0, "grep");
                assert_eq!(components.1, "read");
            }
            _ => panic!("Expected Oscillation anomaly"),
        }
    }

    #[test]
    fn test_backward_causal_slicing_and_pruning() {
        // Trace with:
        // Pos 1: User prompt (state-changing)
        // Pos 2: Planner (decision)
        // Pos 3: Unrelated Read tool (non-state-changing, pruned noise)
        // Pos 4: Unrelated Glob tool (non-state-changing, pruned noise)
        // Pos 5: Edit tool (manifestation, failed)
        let steps = vec![
            TrajectoryStep {
                position: 1,
                component: "User".to_string(),
                role: ComponentRole::PassiveState,
                action: "user_intent".to_string(),
                outcome: StepOutcome::Success,
                state_changed: true,
                inputs: Vec::new(),
                outputs: vec!["prompt".to_string()],
                error: None,
            },
            TrajectoryStep {
                position: 2,
                component: "Planner".to_string(),
                role: ComponentRole::DecisionMaker,
                action: "plan_edit".to_string(),
                outcome: StepOutcome::Success,
                state_changed: false,
                inputs: Vec::new(),
                outputs: vec!["edit_args".to_string()],
                error: None,
            },
            TrajectoryStep {
                position: 3,
                component: "read".to_string(),
                role: ComponentRole::Executor,
                action: "read_unrelated".to_string(),
                outcome: StepOutcome::Success,
                state_changed: false,
                inputs: Vec::new(),
                outputs: Vec::new(),
                error: None,
            },
            TrajectoryStep {
                position: 4,
                component: "glob".to_string(),
                role: ComponentRole::Executor,
                action: "glob_unrelated".to_string(),
                outcome: StepOutcome::Success,
                state_changed: false,
                inputs: Vec::new(),
                outputs: Vec::new(),
                error: None,
            },
            TrajectoryStep {
                position: 5,
                component: "edit".to_string(),
                role: ComponentRole::Executor,
                action: "edit_file".to_string(),
                outcome: StepOutcome::Failed,
                state_changed: false,
                inputs: Vec::new(),
                outputs: Vec::new(),
                error: Some("Syntax replacement error".to_string()),
            },
        ];

        let slice = BackwardCausalSlicer::slice(&steps, "edit").expect("slice failed");
        assert_eq!(slice.manifestation_pos, 5);
        assert_eq!(slice.segments.len(), 1);

        // Causal chain must retain state-changing step 1 and manifestation step 5
        let chain = &slice.segments[0].causal_chain;
        assert_eq!(chain, &[1, 5]);

        // Steps 2, 3, 4 should be pruned as non-causal noise
        assert_eq!(slice.pruned_step_positions, vec![2, 3, 4]);

        // Verify Root Cause Attribution re-mapping
        let edg = ExecutionDependencyGraph::fusion_default();
        let attr = RootCauseAttributor::attribute(&slice, &edg);

        // In the causal steps, step 1 is the state-committing node
        assert_eq!(attr.manifestation_node, "edit");
        assert_eq!(attr.manifestation_pos, 5);
        assert_eq!(attr.root_cause_node, "User");
        assert_eq!(attr.root_cause_pos, 1);
        assert_eq!(attr.pruned_steps_count, 3);
    }
}
