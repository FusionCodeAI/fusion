//! Integration tests for STRACE: Structural Trajectory Analysis and Causal Extraction
//! (arXiv:2607.07702).
//!
//! Validates:
//! 1. Execution Dependency Graph (EDG) topology: node registration, control/data edges,
//!    and upstream dependency queries.
//! 2. Structural path anomaly detection: self-loops (3+ uncommitted repetitions),
//!    oscillations (alternating non-state-changing cycles), and dead-end terminal failures.
//! 3. Backward causal slicing with multi-segment clustering: grouping consecutive manifestation
//!    attempts sharing state contexts, representative position selection, and non-causal
//!    exploratory step pruning.
//! 4. Root cause attribution engine: re-mapping executor manifestation failures back to
//!    upstream decision-makers generating faulty inputs.
//! 5. End-to-end session diagnosis: full STRACE trajectory extraction and causal localization
//!    from conversational sessions containing tool failures.

use fusion::agent::session::Session;
use fusion::agent::strace::{
    detect_path_anomalies, BackwardCausalSlicer, CausalAttribution, CausalSegment, CausalSlice,
    ComponentNode, ComponentRole, DependencyKind, ExecutionDependencyGraph, PathAnomaly,
    RootCauseAttributor, StepOutcome, TrajectoryStep,
};
use fusion::provider::types::ToolCall;

// ============================================================================
// Test 1: EDG Topological Dependencies
// ============================================================================

#[test]
fn test_strace_edg_topological_dependencies() {
    let mut edg = ExecutionDependencyGraph::new();
    assert!(edg.nodes.is_empty());
    assert!(edg.edges.is_empty());

    // 1. Add heterogeneous nodes across multiple architectural roles
    edg.add_node(
        "Orchestrator",
        ComponentRole::DecisionMaker,
        vec!["task_plan".to_string(), "tool_args".to_string()],
        vec!["user_prompt".to_string()],
        Some("orchestrator_prompt.md".to_string()),
    );
    edg.add_node(
        "SafetyAdvisor",
        ComponentRole::Advisor,
        vec!["safety_verdict".to_string()],
        vec!["task_plan".to_string()],
        None,
    );
    edg.add_node(
        "CodeExecutor",
        ComponentRole::Executor,
        vec!["patch".to_string(), "exit_code".to_string()],
        vec!["tool_args".to_string()],
        None,
    );
    edg.add_node(
        "WorkspaceState",
        ComponentRole::PassiveState,
        vec!["file_content".to_string(), "ast_tree".to_string()],
        vec!["patch".to_string()],
        None,
    );
    edg.add_node(
        "LinterVerifier",
        ComponentRole::Executor,
        vec!["diagnostics".to_string()],
        vec!["file_content".to_string()],
        None,
    );

    assert_eq!(edg.nodes.len(), 5);

    // Verify node properties
    let orchestrator: &ComponentNode = edg
        .nodes
        .get("Orchestrator")
        .expect("Orchestrator must exist");
    assert_eq!(orchestrator.role, ComponentRole::DecisionMaker);
    assert_eq!(orchestrator.produces, vec!["task_plan", "tool_args"]);
    assert_eq!(orchestrator.consumes, vec!["user_prompt"]);
    assert_eq!(
        orchestrator.prompt_file.as_deref(),
        Some("orchestrator_prompt.md")
    );

    let workspace = edg
        .nodes
        .get("WorkspaceState")
        .expect("WorkspaceState must exist");
    assert_eq!(workspace.role, ComponentRole::PassiveState);
    assert_eq!(workspace.produces, vec!["file_content", "ast_tree"]);

    // 2. Connect nodes with Control and Data dependency edges
    edg.add_edge(
        "Orchestrator",
        "SafetyAdvisor",
        DependencyKind::Control {
            condition: "strict_security_mode".to_string(),
            rationale: "Orchestrator proposals require safety committee review".to_string(),
        },
    );
    edg.add_edge(
        "Orchestrator",
        "CodeExecutor",
        DependencyKind::Data {
            artifact: "tool_args".to_string(),
            rationale: "CodeExecutor executes planned tool arguments".to_string(),
        },
    );
    edg.add_edge(
        "CodeExecutor",
        "WorkspaceState",
        DependencyKind::Data {
            artifact: "patch".to_string(),
            rationale: "Applied patch mutates persistent workspace state".to_string(),
        },
    );
    edg.add_edge(
        "WorkspaceState",
        "LinterVerifier",
        DependencyKind::Data {
            artifact: "file_content".to_string(),
            rationale: "Linter consumes file content to compute diagnostics".to_string(),
        },
    );
    edg.add_edge(
        "WorkspaceState",
        "LinterVerifier",
        DependencyKind::Control {
            condition: "files_modified".to_string(),
            rationale: "Verification only triggered when workspace state changed".to_string(),
        },
    );

    assert_eq!(edg.edges.len(), 5);

    // 3. Query upstream dependencies
    let upstreams_linter = edg.upstream_dependencies("LinterVerifier");
    assert_eq!(upstreams_linter.len(), 2);
    assert!(upstreams_linter
        .iter()
        .all(|(from, _)| *from == "WorkspaceState"));

    let has_linter_data = upstreams_linter.iter().any(|(_, kind)| match kind {
        DependencyKind::Data {
            artifact,
            rationale,
        } => artifact == "file_content" && rationale.contains("Linter consumes file content"),
        _ => false,
    });
    assert!(
        has_linter_data,
        "LinterVerifier must have Data dependency on WorkspaceState"
    );

    let has_linter_control = upstreams_linter.iter().any(|(_, kind)| match kind {
        DependencyKind::Control {
            condition,
            rationale,
        } => condition == "files_modified" && rationale.contains("Verification only triggered"),
        _ => false,
    });
    assert!(
        has_linter_control,
        "LinterVerifier must have Control dependency on WorkspaceState"
    );

    let upstreams_safety = edg.upstream_dependencies("SafetyAdvisor");
    assert_eq!(upstreams_safety.len(), 1);
    assert_eq!(upstreams_safety[0].0, "Orchestrator");
    match upstreams_safety[0].1 {
        DependencyKind::Control {
            condition,
            rationale,
        } => {
            assert_eq!(condition, "strict_security_mode");
            assert!(rationale.contains("safety committee"));
        }
        _ => panic!("Expected Control dependency for SafetyAdvisor"),
    }

    let upstreams_executor = edg.upstream_dependencies("CodeExecutor");
    assert_eq!(upstreams_executor.len(), 1);
    assert_eq!(upstreams_executor[0].0, "Orchestrator");
    match upstreams_executor[0].1 {
        DependencyKind::Data {
            artifact,
            rationale,
        } => {
            assert_eq!(artifact, "tool_args");
            assert!(rationale.contains("CodeExecutor executes"));
        }
        _ => panic!("Expected Data dependency for CodeExecutor"),
    }

    let upstreams_orchestrator = edg.upstream_dependencies("Orchestrator");
    assert!(
        upstreams_orchestrator.is_empty(),
        "Top-level Orchestrator should have no upstream dependencies"
    );

    // 4. Verify standard Fusion default EDG topology
    let fusion_edg = ExecutionDependencyGraph::fusion_default();
    assert!(fusion_edg.nodes.contains_key("Planner"));
    assert!(fusion_edg.nodes.contains_key("AdvisorCommittee"));
    assert!(fusion_edg.nodes.contains_key("ToolExecutor"));
    assert!(fusion_edg.nodes.contains_key("FileSystemState"));
    assert!(fusion_edg.nodes.contains_key("CompilerVerifier"));

    assert_eq!(
        fusion_edg.nodes["Planner"].role,
        ComponentRole::DecisionMaker
    );
    assert_eq!(
        fusion_edg.nodes["ToolExecutor"].role,
        ComponentRole::Executor
    );
    assert_eq!(
        fusion_edg.nodes["AdvisorCommittee"].role,
        ComponentRole::Advisor
    );
    assert_eq!(
        fusion_edg.nodes["FileSystemState"].role,
        ComponentRole::PassiveState
    );

    let planner_to_tool = fusion_edg.upstream_dependencies("ToolExecutor");
    assert!(planner_to_tool.iter().any(|(from, kind)| {
        *from == "Planner"
            && matches!(kind, DependencyKind::Data { artifact, .. } if artifact == "tool_args")
    }));
}

// ============================================================================
// Test 2: Structural Path Anomalies
// ============================================================================

#[test]
fn test_strace_path_anomalies() {
    // 1. Self-Loop Detection: 3+ consecutive invocations of the same component
    // without state progression.
    let self_loop_steps = vec![
        TrajectoryStep {
            position: 1,
            component: "edit".to_string(),
            role: ComponentRole::Executor,
            action: "edit_file".to_string(),
            outcome: StepOutcome::Failed,
            state_changed: false,
            inputs: Vec::new(),
            outputs: Vec::new(),
            error: Some("Syntax replacement error: hunk does not match".to_string()),
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
            error: Some("Syntax replacement error: hunk does not match".to_string()),
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
            error: Some("Syntax replacement error: hunk does not match".to_string()),
        },
        TrajectoryStep {
            position: 4,
            component: "edit".to_string(),
            role: ComponentRole::Executor,
            action: "edit_file".to_string(),
            outcome: StepOutcome::Failed,
            state_changed: false,
            inputs: Vec::new(),
            outputs: Vec::new(),
            error: Some("Syntax replacement error: hunk does not match".to_string()),
        },
    ];

    let loop_anomalies = detect_path_anomalies(&self_loop_steps);
    let self_loops: Vec<_> = loop_anomalies
        .iter()
        .filter_map(|a| match a {
            PathAnomaly::SelfLoop {
                component,
                start_pos,
                repetitions,
            } => Some((component.as_str(), *start_pos, *repetitions)),
            _ => None,
        })
        .collect();

    assert_eq!(self_loops.len(), 1);
    assert_eq!(self_loops[0], ("edit", 1, 4));

    // Self-loop negative check: 2 repetitions should NOT trigger self-loop
    let short_steps = vec![self_loop_steps[0].clone(), self_loop_steps[1].clone()];
    let short_anomalies = detect_path_anomalies(&short_steps);
    assert!(
        !short_anomalies
            .iter()
            .any(|a| matches!(a, PathAnomaly::SelfLoop { .. })),
        "Two repetitions must not trigger a self-loop anomaly"
    );

    // Self-loop negative check: intermediate state change breaks the loop
    let mut broken_loop_steps = self_loop_steps.clone();
    broken_loop_steps[1].state_changed = true;
    let broken_anomalies = detect_path_anomalies(&broken_loop_steps);
    assert!(
        !broken_anomalies
            .iter()
            .any(|a| matches!(a, PathAnomaly::SelfLoop { .. })),
        "Intermediate state-changing step must break self-loop detection"
    );

    // 2. Oscillation Detection: alternating uncommitted calls (A -> B -> A -> B)
    let oscillation_steps = vec![
        TrajectoryStep {
            position: 1,
            component: "grep".to_string(),
            role: ComponentRole::Executor,
            action: "search_symbol".to_string(),
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
            action: "read_file".to_string(),
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
            action: "search_symbol".to_string(),
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
            action: "read_file".to_string(),
            outcome: StepOutcome::Success,
            state_changed: false,
            inputs: Vec::new(),
            outputs: Vec::new(),
            error: None,
        },
    ];

    let osc_anomalies = detect_path_anomalies(&oscillation_steps);
    let oscillations: Vec<_> = osc_anomalies
        .iter()
        .filter_map(|a| match a {
            PathAnomaly::Oscillation {
                components,
                start_pos,
                cycle_count,
            } => Some((
                components.0.as_str(),
                components.1.as_str(),
                *start_pos,
                *cycle_count,
            )),
            _ => None,
        })
        .collect();

    assert_eq!(oscillations.len(), 1);
    assert_eq!(oscillations[0], ("grep", "read", 1, 2));

    // Oscillation negative check: state change in second cycle breaks oscillation
    let mut progressive_osc_steps = oscillation_steps.clone();
    progressive_osc_steps[3].state_changed = true;
    let progressive_anomalies = detect_path_anomalies(&progressive_osc_steps);
    assert!(
        !progressive_anomalies
            .iter()
            .any(|a| matches!(a, PathAnomaly::Oscillation { .. })),
        "State change in second cycle must break oscillation detection"
    );

    // 3. Dead-End Terminal Failure Detection
    let dead_end_steps = vec![
        TrajectoryStep {
            position: 1,
            component: "Planner".to_string(),
            role: ComponentRole::DecisionMaker,
            action: "plan_execution".to_string(),
            outcome: StepOutcome::Success,
            state_changed: false,
            inputs: Vec::new(),
            outputs: Vec::new(),
            error: None,
        },
        TrajectoryStep {
            position: 2,
            component: "compiler".to_string(),
            role: ComponentRole::Executor,
            action: "compile_check".to_string(),
            outcome: StepOutcome::Failed,
            state_changed: false,
            inputs: Vec::new(),
            outputs: Vec::new(),
            error: Some("abort: catastrophic compilation failure on target".to_string()),
        },
    ];

    let dead_end_anomalies = detect_path_anomalies(&dead_end_steps);
    let dead_ends: Vec<_> = dead_end_anomalies
        .iter()
        .filter_map(|a| match a {
            PathAnomaly::DeadEnd {
                component,
                terminal_pos,
                error,
            } => Some((component.as_str(), *terminal_pos, error.as_str())),
            _ => None,
        })
        .collect();

    assert_eq!(dead_ends.len(), 1);
    assert_eq!(
        dead_ends[0],
        (
            "compiler",
            2,
            "abort: catastrophic compilation failure on target"
        )
    );

    // Timeout terminal failure also triggers DeadEnd
    let timeout_steps = vec![
        TrajectoryStep {
            position: 1,
            component: "fetch".to_string(),
            role: ComponentRole::Executor,
            action: "http_get".to_string(),
            outcome: StepOutcome::Timeout,
            state_changed: false,
            inputs: Vec::new(),
            outputs: Vec::new(),
            error: Some("socket read deadline exceeded".to_string()),
        },
        TrajectoryStep {
            position: 2,
            component: "fetch".to_string(),
            role: ComponentRole::Executor,
            action: "http_get_retry".to_string(),
            outcome: StepOutcome::Timeout,
            state_changed: false,
            inputs: Vec::new(),
            outputs: Vec::new(),
            error: Some("socket connection timeout after 30s".to_string()),
        },
    ];
    let timeout_anomalies = detect_path_anomalies(&timeout_steps);
    assert!(timeout_anomalies.iter().any(|a| matches!(
        a,
        PathAnomaly::DeadEnd {
            component,
            terminal_pos,
            error
        } if component == "fetch" && *terminal_pos == 2 && error.contains("30s")
    )));

    // Dead-End negative check: terminal step with Success produces no DeadEnd
    let successful_terminal = vec![
        TrajectoryStep {
            position: 1,
            component: "edit".to_string(),
            role: ComponentRole::Executor,
            action: "edit_file".to_string(),
            outcome: StepOutcome::Success,
            state_changed: true,
            inputs: Vec::new(),
            outputs: Vec::new(),
            error: None,
        },
        TrajectoryStep {
            position: 2,
            component: "compiler".to_string(),
            role: ComponentRole::Executor,
            action: "compile".to_string(),
            outcome: StepOutcome::Success,
            state_changed: false,
            inputs: Vec::new(),
            outputs: Vec::new(),
            error: None,
        },
    ];
    let success_anomalies = detect_path_anomalies(&successful_terminal);
    assert!(
        !success_anomalies
            .iter()
            .any(|a| matches!(a, PathAnomaly::DeadEnd { .. })),
        "Successful terminal step must not produce a dead-end anomaly"
    );
}

// ============================================================================
// Test 3: Backward Causal Slicing with Multi-Segment Clustering
// ============================================================================

#[test]
fn test_strace_backward_slicing_multi_segment() {
    // Construct a complex trajectory with:
    // - 3 state changes (Steps 1, 7, 10)
    // - 4 manifestation attempts of target "compiler" (Steps 4, 5, 9, 12)
    //   - Steps 4 and 5 are consecutive attempts with NO state change between -> Segment 1
    //   - Step 9 follows state change at Step 7 -> Segment 2
    //   - Step 12 follows state change at Step 10 -> Segment 3
    // - Unrelated exploratory and planning steps (Steps 2, 3, 6, 8, 11) that should be pruned as noise
    let steps = vec![
        // Pos 1: User prompt creates initial problem state
        TrajectoryStep {
            position: 1,
            component: "User".to_string(),
            role: ComponentRole::PassiveState,
            action: "user_turn_prompt".to_string(),
            outcome: StepOutcome::Success,
            state_changed: true,
            inputs: Vec::new(),
            outputs: vec!["problem_statement".to_string()],
            error: None,
        },
        // Pos 2: Planner (exploratory planning - pruned)
        TrajectoryStep {
            position: 2,
            component: "Planner".to_string(),
            role: ComponentRole::DecisionMaker,
            action: "formulate_strategy".to_string(),
            outcome: StepOutcome::Success,
            state_changed: false,
            inputs: vec!["problem_statement".to_string()],
            outputs: vec!["initial_strategy".to_string()],
            error: None,
        },
        // Pos 3: Read tool (exploratory read - pruned)
        TrajectoryStep {
            position: 3,
            component: "read".to_string(),
            role: ComponentRole::Executor,
            action: "read_unrelated_docs".to_string(),
            outcome: StepOutcome::Success,
            state_changed: false,
            inputs: Vec::new(),
            outputs: vec!["docs_content".to_string()],
            error: None,
        },
        // Pos 4: Manifestation attempt 1 (Segment 1 representative)
        TrajectoryStep {
            position: 4,
            component: "compiler".to_string(),
            role: ComponentRole::Executor,
            action: "compile_verification".to_string(),
            outcome: StepOutcome::Failed,
            state_changed: false,
            inputs: Vec::new(),
            outputs: Vec::new(),
            error: Some("error[E0425]: cannot find value `config`".to_string()),
        },
        // Pos 5: Manifestation attempt 2 (same context as Pos 4, uncommitted retry)
        TrajectoryStep {
            position: 5,
            component: "compiler".to_string(),
            role: ComponentRole::Executor,
            action: "compile_verification_retry".to_string(),
            outcome: StepOutcome::Failed,
            state_changed: false,
            inputs: Vec::new(),
            outputs: Vec::new(),
            error: Some("error[E0425]: cannot find value `config`".to_string()),
        },
        // Pos 6: Grep tool (exploratory search - pruned)
        TrajectoryStep {
            position: 6,
            component: "grep".to_string(),
            role: ComponentRole::Executor,
            action: "search_config_definition".to_string(),
            outcome: StepOutcome::Success,
            state_changed: false,
            inputs: Vec::new(),
            outputs: Vec::new(),
            error: None,
        },
        // Pos 7: Edit tool commits a change to the codebase
        TrajectoryStep {
            position: 7,
            component: "edit".to_string(),
            role: ComponentRole::Executor,
            action: "write_config_struct".to_string(),
            outcome: StepOutcome::Success,
            state_changed: true,
            inputs: Vec::new(),
            outputs: vec!["config_patch".to_string()],
            error: None,
        },
        // Pos 8: Glob tool (exploratory search - pruned)
        TrajectoryStep {
            position: 8,
            component: "glob".to_string(),
            role: ComponentRole::Executor,
            action: "glob_test_files".to_string(),
            outcome: StepOutcome::Success,
            state_changed: false,
            inputs: Vec::new(),
            outputs: Vec::new(),
            error: None,
        },
        // Pos 9: Manifestation attempt 3 (Segment 2 representative after Pos 7 state change)
        TrajectoryStep {
            position: 9,
            component: "compiler".to_string(),
            role: ComponentRole::Executor,
            action: "recompile_verification".to_string(),
            outcome: StepOutcome::Failed,
            state_changed: false,
            inputs: Vec::new(),
            outputs: Vec::new(),
            error: Some("error[E0308]: mismatched types in config parser".to_string()),
        },
        // Pos 10: Edit tool commits second state change
        TrajectoryStep {
            position: 10,
            component: "edit".to_string(),
            role: ComponentRole::Executor,
            action: "fix_type_signature".to_string(),
            outcome: StepOutcome::Success,
            state_changed: true,
            inputs: Vec::new(),
            outputs: vec!["signature_patch".to_string()],
            error: None,
        },
        // Pos 11: Read tool (exploratory read - pruned)
        TrajectoryStep {
            position: 11,
            component: "read".to_string(),
            role: ComponentRole::Executor,
            action: "read_signature_check".to_string(),
            outcome: StepOutcome::Success,
            state_changed: false,
            inputs: Vec::new(),
            outputs: Vec::new(),
            error: None,
        },
        // Pos 12: Manifestation attempt 4 (Segment 3 representative after Pos 10 state change)
        TrajectoryStep {
            position: 12,
            component: "compiler".to_string(),
            role: ComponentRole::Executor,
            action: "final_compile_attempt".to_string(),
            outcome: StepOutcome::Failed,
            state_changed: false,
            inputs: Vec::new(),
            outputs: Vec::new(),
            error: Some("error[E0599]: no method `load` found for struct `Config`".to_string()),
        },
    ];

    let slice = BackwardCausalSlicer::slice(&steps, "compiler").expect("slicing must succeed");

    // 1. Verify target component and global representative manifestation position
    assert_eq!(slice.target_component, "compiler");
    assert_eq!(
        slice.manifestation_pos, 4,
        "Manifestation position must be the first segment's representative"
    );

    // 2. Verify segmentation: exactly 3 causal segments formed
    assert_eq!(
        slice.segments.len(),
        3,
        "Expected exactly 3 causal segments corresponding to distinct state contexts"
    );

    // Segment 0: Encompasses manifestation attempts 4 and 5 (grouped because no state change occurred between them)
    let seg0 = &slice.segments[0];
    assert_eq!(seg0.segment_positions, vec![4, 5]);
    assert_eq!(
        seg0.representative_pos, 4,
        "First manifestation position in run must be chosen as representative"
    );
    assert_eq!(
        seg0.causal_chain,
        vec![1, 4],
        "Causal chain for Segment 0 must contain preceding state change (1) and representative (4)"
    );

    // Segment 1: Encompasses manifestation attempt 9
    let seg1 = &slice.segments[1];
    assert_eq!(seg1.segment_positions, vec![9]);
    assert_eq!(seg1.representative_pos, 9);
    assert_eq!(
        seg1.causal_chain,
        vec![1, 7, 9],
        "Causal chain for Segment 1 must contain state changes (1, 7) and representative (9)"
    );

    // Segment 2: Encompasses manifestation attempt 12
    let seg2 = &slice.segments[2];
    assert_eq!(seg2.segment_positions, vec![12]);
    assert_eq!(seg2.representative_pos, 12);
    assert_eq!(
        seg2.causal_chain,
        vec![1, 7, 10, 12],
        "Causal chain for Segment 2 must contain state changes (1, 7, 10) and representative (12)"
    );

    // 3. Verify causal step retention across the union of all segment causal chains
    let causal_positions: Vec<usize> = slice.causal_steps.iter().map(|s| s.position).collect();
    assert_eq!(
        causal_positions,
        vec![1, 4, 7, 9, 10, 12],
        "Causal steps must include exactly the union of all segment causal chains"
    );

    // 4. Verify noise pruning: exploratory steps (2, 3, 6, 8, 11) and redundant manifestation (5) pruned
    assert_eq!(
        slice.pruned_step_positions,
        vec![2, 3, 5, 6, 8, 11],
        "Non-causal exploratory steps and duplicate manifestations must be pruned"
    );

    // 5. Error handling for empty trajectory
    let empty_err = BackwardCausalSlicer::slice(&[], "compiler");
    assert!(empty_err.is_err());
    assert_eq!(empty_err.unwrap_err(), "Cannot slice empty trajectory");
}

// ============================================================================
// Test 4: Root Cause Attribution Re-mapping to Upstream Decision-Maker
// ============================================================================

#[test]
fn test_strace_root_cause_attribution_decision_maker() {
    // Construct an execution trajectory where an upstream Planner (DecisionMaker)
    // commits a plan containing erroneous parameters, leading downstream executor "bash"
    // to fail during command execution.
    let steps = vec![
        TrajectoryStep {
            position: 1,
            component: "User".to_string(),
            role: ComponentRole::PassiveState,
            action: "user_turn_prompt".to_string(),
            outcome: StepOutcome::Success,
            state_changed: true,
            inputs: Vec::new(),
            outputs: vec!["intent: run tests in release mode".to_string()],
            error: None,
        },
        TrajectoryStep {
            position: 2,
            component: "Planner".to_string(),
            role: ComponentRole::DecisionMaker,
            action: "plan_bash_invocation".to_string(),
            outcome: StepOutcome::Success,
            state_changed: true, // Planner commits execution plan to session context
            inputs: vec!["intent".to_string()],
            outputs: vec!["tool_args: cargo test --nonexistent-cargo-flag".to_string()],
            error: None,
        },
        TrajectoryStep {
            position: 3,
            component: "read".to_string(),
            role: ComponentRole::Executor,
            action: "read_cargo_toml".to_string(),
            outcome: StepOutcome::Success,
            state_changed: false,
            inputs: Vec::new(),
            outputs: Vec::new(),
            error: None,
        },
        TrajectoryStep {
            position: 4,
            component: "bash".to_string(),
            role: ComponentRole::Executor,
            action: "execute_cargo_test".to_string(),
            outcome: StepOutcome::Failed,
            state_changed: false,
            inputs: vec!["cargo test --nonexistent-cargo-flag".to_string()],
            outputs: Vec::new(),
            error: Some("error: unrecognized option '--nonexistent-cargo-flag'".to_string()),
        },
    ];

    let slice = BackwardCausalSlicer::slice(&steps, "bash").expect("slicing must succeed");
    assert_eq!(slice.manifestation_pos, 4);
    assert_eq!(slice.target_component, "bash");
    // Step 3 (read) is non-state-changing noise and must be pruned
    assert_eq!(slice.pruned_step_positions, vec![3]);

    let edg = ExecutionDependencyGraph::fusion_default();
    let attribution: CausalAttribution = RootCauseAttributor::attribute(&slice, &edg);
    // Verify manifestation symptom
    assert_eq!(attribution.manifestation_node, "bash");
    assert_eq!(attribution.manifestation_pos, 4);

    // Verify re-mapping: root cause must be the upstream DecisionMaker, NOT the executor
    assert_eq!(
        attribution.root_cause_node, "Planner",
        "Failure in executor 'bash' must be re-mapped to upstream DecisionMaker 'Planner'"
    );
    assert_eq!(attribution.root_cause_pos, 2);

    // Verify explanatory reason and diagnostic heuristic
    assert!(
        attribution.reason.contains("Upstream DecisionMaker 'Planner' at Step #2 supplied invalid parameters or misrouted to 'bash'"),
        "Attribution reason should indicate Planner supplied invalid parameters: {}",
        attribution.reason
    );
    assert!(
        attribution.suggested_heuristic.contains("Verify input arguments and prerequisite assumptions in 'Planner' before invoking downstream tool 'bash'"),
        "Attribution heuristic should guide Planner input validation: {}",
        attribution.suggested_heuristic
    );

    // Verify causal chain and pruned count
    assert_eq!(attribution.causal_chain, vec![1, 2, 4]);
    assert_eq!(attribution.pruned_steps_count, 1);

    // Also test scenario where Planner did not commit state change directly, but is present
    // in causal steps (e.g. from an explicit sub-agent decision step)
    let explicit_slice = CausalSlice {
        target_component: "git_checkout".to_string(),
        manifestation_pos: 2,
        segments: vec![CausalSegment {
            segment_positions: vec![2],
            representative_pos: 2,
            causal_chain: vec![1, 2],
        }],
        causal_steps: vec![
            TrajectoryStep {
                position: 1,
                component: "BranchRouter".to_string(),
                role: ComponentRole::DecisionMaker,
                action: "select_target_branch".to_string(),
                outcome: StepOutcome::Success,
                state_changed: false,
                inputs: Vec::new(),
                outputs: vec!["branch: typo-branch-name".to_string()],
                error: None,
            },
            TrajectoryStep {
                position: 2,
                component: "git_checkout".to_string(),
                role: ComponentRole::Executor,
                action: "checkout".to_string(),
                outcome: StepOutcome::Failed,
                state_changed: false,
                inputs: vec!["typo-branch-name".to_string()],
                outputs: Vec::new(),
                error: Some("pathspec 'typo-branch-name' did not match any file(s)".to_string()),
            },
        ],
        pruned_step_positions: Vec::new(),
    };

    let router_attr: CausalAttribution = RootCauseAttributor::attribute(&explicit_slice, &edg);
    assert_eq!(router_attr.root_cause_node, "BranchRouter");
    assert_eq!(router_attr.root_cause_pos, 1);
    assert!(router_attr.reason.contains("BranchRouter"));
}

// ============================================================================
// Test 5: End-to-End STRACE Session Diagnosis
// ============================================================================

#[test]
fn test_strace_diagnose_session() {
    // 1. Construct a conversational Session with:
    // - User prompt (Step 1: User, PassiveState, state_changed: true)
    // - Assistant response with tool calls (Step 2: Planner, Step 3: tool executor)
    // - Tool result containing an error (Step 4: ToolResult, Executor, Failed)
    let mut session = Session::new("claude-3-7-sonnet");
    session.add_user_message("Please refactor the user authentication database schema.");

    session.add_assistant_with_tools(
        "I will inspect the database migration file.",
        vec![ToolCall {
            id: "call_read_migration_01".to_string(),
            name: "read_file".to_string(),
            arguments: r#"{"path": "migrations/20260101_auth.sql"}"#.to_string(),
        }],
    );

    session.add_tool_result(
        "call_read_migration_01",
        "Error: file not found: migrations/20260101_auth.sql",
    );

    // Verify session step extraction directly
    let steps = BackwardCausalSlicer::extract_steps_from_session(&session);
    assert_eq!(steps.len(), 4);
    assert_eq!(steps[0].component, "User");
    assert!(steps[0].state_changed);
    assert_eq!(steps[1].component, "Planner");
    assert_eq!(steps[1].role, ComponentRole::DecisionMaker);
    assert_eq!(steps[2].component, "read_file");
    assert_eq!(steps[2].role, ComponentRole::Executor);
    assert_eq!(steps[3].component, "ToolResult");
    assert_eq!(steps[3].outcome, StepOutcome::Failed);
    assert!(steps[3]
        .error
        .as_deref()
        .unwrap()
        .contains("file not found"));

    // Run end-to-end diagnosis on the session
    let attribution: CausalAttribution = RootCauseAttributor::diagnose_session(&session)
        .expect("Session diagnosis must return Ok(attribution)");
    assert_eq!(attribution.manifestation_node, "ToolResult");
    assert_eq!(attribution.manifestation_pos, 4);
    assert_eq!(
        attribution.root_cause_node, "User",
        "When no intermediate mutation occurred, root cause attributes to initial state-committing User turn"
    );
    assert_eq!(attribution.root_cause_pos, 1);
    assert!(attribution
        .reason
        .contains("Upstream state-committing node 'User' at Step #1"));
    assert!(attribution
        .suggested_heuristic
        .contains("Validate invariant consistency in 'User'"));
    assert_eq!(attribution.causal_chain, vec![1, 4]);
    assert_eq!(
        attribution.pruned_steps_count, 2,
        "Steps 2 (Planner) and 3 (read_file) must be pruned as non-causal intermediaries"
    );

    // 2. Scenario with state mutation prior to tool failure:
    // User -> Tool writes file (state changed!) -> Tool fails
    let mut mutation_session = Session::new("claude-3-7-sonnet");
    mutation_session.add_user_message("Create a new helper file and verify it.");

    mutation_session.add_assistant_with_tools(
        "I will write the helper module.",
        vec![ToolCall {
            id: "call_write_helper".to_string(),
            name: "write".to_string(),
            arguments: r#"{"path": "src/helper.rs"}"#.to_string(),
        }],
    );
    // Tool returns success with recognizable state-changing marker
    mutation_session.add_tool_result(
        "call_write_helper",
        "Successfully wrote 24 lines to src/helper.rs",
    );

    mutation_session.add_assistant_with_tools(
        "Now I will verify the build.",
        vec![ToolCall {
            id: "call_bash_check".to_string(),
            name: "bash".to_string(),
            arguments: r#"{"command": "cargo check"}"#.to_string(),
        }],
    );
    // Downstream verification fails
    mutation_session.add_tool_result(
        "call_bash_check",
        "Error: cargo check failed: unresolved import `super::unknown_type`",
    );

    let mutation_steps = BackwardCausalSlicer::extract_steps_from_session(&mutation_session);
    assert_eq!(mutation_steps.len(), 7);
    // Step 1: User (state_changed: true)
    // Step 2: Planner
    // Step 3: write
    // Step 4: ToolResult ("Successfully wrote" -> state_changed: true)
    // Step 5: Planner
    // Step 6: bash
    // Step 7: ToolResult ("Error" -> Failed)
    assert!(mutation_steps[3].state_changed);
    assert_eq!(mutation_steps[3].component, "ToolResult");
    assert_eq!(mutation_steps[6].outcome, StepOutcome::Failed);

    let mutation_attr: CausalAttribution = RootCauseAttributor::diagnose_session(&mutation_session)
        .expect("Mutation session diagnosis must succeed");
    assert_eq!(mutation_attr.manifestation_node, "ToolResult");
    assert_eq!(mutation_attr.manifestation_pos, 7);
    // The upstream state-committing node is Step 4 (the tool mutation that introduced the invalid code)
    assert_eq!(
        mutation_attr.root_cause_node, "ToolResult",
        "Root cause must be attributed to the upstream state-committing ToolResult"
    );
    assert_eq!(mutation_attr.root_cause_pos, 4);
    assert!(mutation_attr
        .reason
        .contains("Upstream state-committing node 'ToolResult' at Step #4"));
    assert_eq!(mutation_attr.causal_chain, vec![1, 4, 7]);

    // 3. Error case: Empty session returns diagnostic error
    let empty_session = Session::new("gpt-4o");
    let empty_err = RootCauseAttributor::diagnose_session(&empty_session);
    assert!(empty_err.is_err());
    assert_eq!(
        empty_err.unwrap_err(),
        "Session has no trajectory steps to analyze"
    );
}
