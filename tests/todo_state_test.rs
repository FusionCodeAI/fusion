//! Comprehensive unit and integration tests for `TodoState`.
//!
//! Tests verify:
//! 1. Initial state & initialization from list of phases.
//! 2. Auto-promotion of earliest pending task to `InProgress`.
//! 3. `start` query matching and demoting existing in-progress tasks.
//! 4. `done` transitions for single tasks and entire phases, with auto-promotion.
//! 5. `block` transitions with reason and handing `InProgress` to next pending.
//! 6. `unblock` transitioning blocked tasks back to `Pending`.
//! 7. `drop_task` transitions with proper promotion if active.
//! 8. `append` to existing phases and dynamically creating new phases.
//! 9. `view` formatting with markdown checklist and summary counts.
//! 10. `compact_summary` formatting for TUI rail across various states.
//! 11. Serde JSON roundtrip serialization.

#[path = "../src/agent/todo_state.rs"]
pub mod todo_state;

use todo_state::{TodoItem, TodoPhase, TodoState, TodoStatus};

// ============================================================================
// 1. Initial State & Initialization Tests
// ============================================================================

#[test]
fn test_initial_empty_state() {
    let state = TodoState::new();
    assert!(state.is_empty());
    assert_eq!(state.total_count(), 0);
    assert_eq!(state.completed_count(), 0);
    assert_eq!(state.pending_count(), 0);
    assert_eq!(state.in_progress_count(), 0);
    assert_eq!(state.blocked_count(), 0);
    assert_eq!(state.dropped_count(), 0);
    assert!(state.current_task().is_none());
    assert_eq!(state.compact_summary(), "[0/0 done] No tasks");
    assert!(state.view().contains("[0/0]"));
    assert!(state.view().contains("No tasks scheduled"));
}

#[test]
fn test_init_replaces_and_auto_promotes() {
    let mut state = TodoState::new();
    let initial_phases = vec![
        (
            "Phase 1: Architecture".to_string(),
            vec![
                "Define module boundaries".to_string(),
                "Document invariants".to_string(),
            ],
        ),
        (
            "Phase 2: Implementation".to_string(),
            vec![
                "Implement todo_state.rs".to_string(),
                "Implement todo_tool.rs".to_string(),
            ],
        ),
    ];

    state.init(initial_phases);

    assert_eq!(state.phases.len(), 2);
    assert_eq!(state.total_count(), 4);
    assert_eq!(state.completed_count(), 0);
    assert_eq!(state.pending_count(), 3);
    assert_eq!(state.in_progress_count(), 1);

    // Earliest pending task should be automatically promoted to InProgress
    let current = state.current_task().expect("Expected an active task");
    assert_eq!(current.id, 1);
    assert_eq!(current.task, "Define module boundaries");
    assert_eq!(current.status, TodoStatus::InProgress);
    assert_eq!(current.phase, "Phase 1: Architecture");

    // Check remaining tasks are Pending
    let task2 = state.get_task_by_id(2).unwrap();
    assert_eq!(task2.status, TodoStatus::Pending);
    let task3 = state.get_task_by_id(3).unwrap();
    assert_eq!(task3.status, TodoStatus::Pending);
    let task4 = state.get_task_by_id(4).unwrap();
    assert_eq!(task4.status, TodoStatus::Pending);

    // Verify compact summary reflects active task
    assert_eq!(
        state.compact_summary(),
        "[0/4 done] Current: Define module boundaries"
    );
}

// ============================================================================
// 2. Task Switching & Start Tests
// ============================================================================

#[test]
fn test_start_by_substring_and_demotion() {
    let mut state = TodoState::new();
    state.init(vec![
        (
            "Core".to_string(),
            vec!["Task Alpha".to_string(), "Task Beta".to_string()],
        ),
    ]);

    assert_eq!(state.current_task().unwrap().task, "Task Alpha");

    // Switch to Task Beta by substring
    let switched = state.start("Beta");
    assert!(switched);

    let current = state.current_task().unwrap();
    assert_eq!(current.task, "Task Beta");
    assert_eq!(current.status, TodoStatus::InProgress);

    // Verify Task Alpha was demoted back to Pending
    let alpha = state.get_task_by_id(1).unwrap();
    assert_eq!(alpha.status, TodoStatus::Pending);

    // Switch by ID
    let switched_id = state.start("#1");
    assert!(switched_id);
    assert_eq!(state.current_task().unwrap().task, "Task Alpha");
    assert_eq!(state.get_task_by_id(2).unwrap().status, TodoStatus::Pending);

    // Switching to unknown task fails gracefully
    assert!(!state.start("NonExistentTask"));
}

// ============================================================================
// 3. Done Transitions & Auto-Promotion Tests
// ============================================================================

#[test]
fn test_done_task_promotes_next_pending() {
    let mut state = TodoState::new();
    state.init(vec![
        (
            "Phase 1".to_string(),
            vec!["Step 1".to_string(), "Step 2".to_string()],
        ),
        (
            "Phase 2".to_string(),
            vec!["Step 3".to_string()],
        ),
    ]);

    // Initial: Step 1 is in progress
    assert_eq!(state.current_task().unwrap().task, "Step 1");

    // Complete Step 1
    let ok = state.done("Step 1");
    assert!(ok);

    let step1 = state.get_task_by_id(1).unwrap();
    assert_eq!(step1.status, TodoStatus::Completed);
    assert!(step1.completed_at_ms.is_some());
    assert_eq!(state.completed_count(), 1);

    // Step 2 should be auto-promoted to InProgress
    let current = state.current_task().unwrap();
    assert_eq!(current.id, 2);
    assert_eq!(current.task, "Step 2");

    // Complete Step 2
    state.done("#2");
    assert_eq!(state.completed_count(), 2);

    // Step 3 (in Phase 2) should be auto-promoted across phases
    let current = state.current_task().unwrap();
    assert_eq!(current.id, 3);
    assert_eq!(current.task, "Step 3");

    // Complete final step
    state.done("Step 3");
    assert_eq!(state.completed_count(), 3);
    assert!(state.current_task().is_none());
    assert_eq!(state.compact_summary(), "[3/3 done] All tasks completed");
}

#[test]
fn test_done_entire_phase() {
    let mut state = TodoState::new();
    state.init(vec![
        (
            "Setup".to_string(),
            vec!["Init repo".to_string(), "Add dependencies".to_string()],
        ),
        (
            "Build".to_string(),
            vec!["Compile binary".to_string()],
        ),
    ]);

    // Complete the entire "Setup" phase by name
    let ok = state.done("Setup");
    assert!(ok);

    assert_eq!(state.get_task_by_id(1).unwrap().status, TodoStatus::Completed);
    assert_eq!(state.get_task_by_id(2).unwrap().status, TodoStatus::Completed);
    assert_eq!(state.completed_count(), 2);

    // Next phase task "Compile binary" should now be in progress
    let current = state.current_task().unwrap();
    assert_eq!(current.task, "Compile binary");
    assert_eq!(current.status, TodoStatus::InProgress);
}

// ============================================================================
// 4. Block & Unblock Transitions Tests
// ============================================================================

#[test]
fn test_block_and_unblock_workflow() {
    let mut state = TodoState::new();
    state.init(vec![
        (
            "Core".to_string(),
            vec![
                "First Task".to_string(),
                "Second Task".to_string(),
                "Third Task".to_string(),
            ],
        ),
    ]);

    // First Task is in progress
    assert_eq!(state.current_task().unwrap().task, "First Task");

    // Block First Task
    let blocked = state.block("First", Some("Waiting on credentials".to_string()));
    assert!(blocked);

    let first = state.get_task_by_id(1).unwrap();
    assert_eq!(first.status, TodoStatus::Blocked);
    assert_eq!(first.reason.as_deref(), Some("Waiting on credentials"));
    assert_eq!(state.blocked_count(), 1);

    // InProgress should have been handed to Second Task
    let current = state.current_task().unwrap();
    assert_eq!(current.id, 2);
    assert_eq!(current.task, "Second Task");

    // Unblock First Task -> moves back to Pending with reason cleared
    let unblocked = state.unblock("First");
    assert!(unblocked);

    let first_unblocked = state.get_task_by_id(1).unwrap();
    assert_eq!(first_unblocked.status, TodoStatus::Pending);
    assert!(first_unblocked.reason.is_none());

    // Second Task is still InProgress
    assert_eq!(state.current_task().unwrap().task, "Second Task");
}

// ============================================================================
// 5. Drop Task Transition Tests
// ============================================================================

#[test]
fn test_drop_task_transitions() {
    let mut state = TodoState::new();
    state.init(vec![
        (
            "Phase 1".to_string(),
            vec![
                "Active Task".to_string(),
                "Obsolete Task".to_string(),
                "Later Task".to_string(),
            ],
        ),
    ]);

    // Drop non-active task
    let dropped_non_active = state.drop_task("Obsolete Task");
    assert!(dropped_non_active);
    assert_eq!(state.get_task_by_id(2).unwrap().status, TodoStatus::Dropped);
    assert_eq!(state.current_task().unwrap().task, "Active Task");

    // Drop currently in-progress task
    let dropped_active = state.drop_task("Active Task");
    assert!(dropped_active);
    assert_eq!(state.get_task_by_id(1).unwrap().status, TodoStatus::Dropped);

    // Next pending task "Later Task" should become InProgress
    let current = state.current_task().unwrap();
    assert_eq!(current.id, 3);
    assert_eq!(current.task, "Later Task");
    assert_eq!(current.status, TodoStatus::InProgress);
}

// ============================================================================
// 6. Append Tasks Tests
// ============================================================================

#[test]
fn test_append_existing_and_new_phase() {
    let mut state = TodoState::new();
    state.init(vec![
        ("Planning".to_string(), vec!["Design spec".to_string()]),
    ]);

    // Append to existing phase
    state.append("Planning", vec!["Review spec".to_string(), "Approve spec".to_string()]);
    assert_eq!(state.phases[0].items.len(), 3);
    assert_eq!(state.total_count(), 3);
    assert_eq!(state.get_task_by_id(2).unwrap().task, "Review spec");
    assert_eq!(state.get_task_by_id(3).unwrap().task, "Approve spec");

    // Append to new phase
    state.append("Execution", vec!["Write code".to_string()]);
    assert_eq!(state.phases.len(), 2);
    assert_eq!(state.phases[1].name, "Execution");
    assert_eq!(state.total_count(), 4);
    assert_eq!(state.get_task_by_id(4).unwrap().task, "Write code");
    assert_eq!(state.get_task_by_id(4).unwrap().phase, "Execution");
}

// ============================================================================
// 7. View & Compact Summary Formatting Tests
// ============================================================================

#[test]
fn test_view_formatting_with_statuses() {
    let mut state = TodoState::new();
    state.init(vec![
        (
            "Phase 1: Foundation".to_string(),
            vec![
                "Setup project".to_string(),
                "Write core logic".to_string(),
                "Add database layer".to_string(),
                "Legacy migration".to_string(),
                "Verify release".to_string(),
            ],
        ),
    ]);

    // Setup diverse task statuses
    state.done("Setup project"); // Completed
    state.start("Write core logic"); // InProgress
    state.block("database", Some("Database credentials pending".to_string())); // Blocked
    state.drop_task("migration"); // Dropped

    let view_output = state.view();

    // Verify checklist headers and count
    assert!(view_output.contains("# Tasks [1/5]"), "View missing tasks header with count");
    assert!(view_output.contains("## Phase 1: Foundation"), "View missing phase header");

    // Verify status symbols
    assert!(view_output.contains("- [x] #1: Setup project"));
    assert!(view_output.contains("- [>] #2: Write core logic (in progress)"));
    assert!(view_output.contains("- [!] #3: Add database layer (blocked: Database credentials pending)"));
    assert!(view_output.contains("- [-] #4: Legacy migration (dropped)"));
    assert!(view_output.contains("- [ ] #5: Verify release"));

    // Verify compact summary
    let summary = state.compact_summary();
    assert_eq!(summary, "[1/5 done] Current: Write core logic");
}

#[test]
fn test_compact_summary_states() {
    let mut state = TodoState::new();
    assert_eq!(state.compact_summary(), "[0/0 done] No tasks");

    state.init(vec![
        ("Phase".to_string(), vec!["Task 1".to_string(), "Task 2".to_string()]),
    ]);
    assert_eq!(state.compact_summary(), "[0/2 done] Current: Task 1");

    state.done("Task 1");
    assert_eq!(state.compact_summary(), "[1/2 done] Current: Task 2");

    state.done("Task 2");
    assert_eq!(state.compact_summary(), "[2/2 done] All tasks completed");

    // When all tasks are blocked/dropped
    let mut state2 = TodoState::new();
    state2.append("P", vec!["T1".to_string()]);
    state2.block("T1", None);
    assert_eq!(state2.compact_summary(), "[0/1 done] No active task");
}

// ============================================================================
// 8. Serde JSON Roundtrip Test
// ============================================================================

#[test]
fn test_serde_roundtrip() {
    let mut state = TodoState::new();
    state.init(vec![
        (
            "Phase Alpha".to_string(),
            vec!["Task 1".to_string(), "Task 2".to_string()],
        ),
    ]);
    state.done("Task 1");

    let json_str = serde_json::to_string_pretty(&state).expect("Failed to serialize TodoState");
    let deserialized: TodoState =
        serde_json::from_str(&json_str).expect("Failed to deserialize TodoState");

    assert_eq!(state, deserialized);
    assert_eq!(deserialized.total_count(), 2);
    assert_eq!(deserialized.completed_count(), 1);
    assert_eq!(deserialized.current_task().unwrap().task, "Task 2");
}
