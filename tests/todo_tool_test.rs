use fusion::agent::todo_state::TodoStatus;
use fusion::tools::todo::{TodoTool, GLOBAL_TODO_STATE};
use fusion::tools::{Tool, ToolContext};
use serde_json::json;

#[tokio::test]
async fn test_todo_tool_name_and_metadata() {
    let tool = TodoTool::new();
    assert_eq!(tool.name(), "todo");
    assert!(!tool.description().is_empty());
    assert!(
        tool.description().contains("init"),
        "Description should describe operations including 'init'"
    );
    assert!(
        tool.description().contains("start"),
        "Description should describe 'start'"
    );
    assert!(
        tool.description().contains("done"),
        "Description should describe 'done'"
    );
    assert!(
        tool.description().contains("block"),
        "Description should describe 'block'"
    );
    assert!(
        tool.description().contains("view"),
        "Description should describe 'view'"
    );

    let def = tool.definition();
    assert_eq!(def.name, "todo");
    assert_eq!(def.description, tool.description());
    assert_eq!(def.parameters, tool.parameters());
}

#[tokio::test]
async fn test_todo_tool_parameter_schema() {
    let tool = TodoTool::new();
    let schema = tool.parameters();

    assert_eq!(
        schema.get("type").and_then(|v| v.as_str()),
        Some("object"),
        "Schema root must be an object"
    );

    let required = schema
        .get("required")
        .and_then(|v| v.as_array())
        .expect("parameters schema must specify required fields");
    assert!(
        required.iter().any(|v| v.as_str() == Some("op")),
        "'op' must be in required list"
    );

    let props = schema
        .get("properties")
        .and_then(|v| v.as_object())
        .expect("parameters schema must specify properties");

    assert!(props.contains_key("op"));
    assert!(props.contains_key("list"));
    assert!(props.contains_key("task"));
    assert!(props.contains_key("phase"));
    assert!(props.contains_key("items"));
    assert!(props.contains_key("reason"));

    let op_prop = props.get("op").unwrap();
    let op_enums: Vec<&str> = op_prop
        .get("enum")
        .and_then(|v| v.as_array())
        .expect("op must have an enum of valid operations")
        .iter()
        .filter_map(|v| v.as_str())
        .collect();

    let expected_ops = [
        "init", "start", "done", "rm", "drop", "block", "unblock", "append", "view",
    ];
    for expected in expected_ops {
        assert!(
            op_enums.contains(&expected),
            "op enum should contain '{}'",
            expected
        );
    }

    let list_prop = props.get("list").unwrap();
    assert_eq!(
        list_prop.get("type").and_then(|v| v.as_str()),
        Some("array")
    );
}

#[tokio::test]
async fn test_todo_tool_init_and_view() {
    TodoTool::reset().await;
    let tool = TodoTool::new();
    let ctx = ToolContext::default();

    let init_args = json!({
        "op": "init",
        "list": [
            {
                "phase": "Phase 1 - Setup",
                "items": [
                    "Install dependencies",
                    "Configure environment"
                ]
            },
            {
                "phase": "Phase 2 - Execution",
                "items": [
                    "Run core engine",
                    "Verify results"
                ]
            }
        ]
    });

    let res = tool
        .execute(init_args, &ctx)
        .await
        .expect("init should succeed");
    assert!(res.contains("Initialized todo tracker"));
    assert!(res.contains("4 task(s)"));
    assert!(res.contains("2 phase(s)"));
    assert!(res.contains("Phase 1 - Setup"));
    assert!(res.contains("Phase 2 - Execution"));
    assert!(res.contains("[>] #1: Install dependencies"));
    assert!(res.contains("[ ] #2: Configure environment"));

    let view_args = json!({ "op": "view" });
    let view_res = tool
        .execute(view_args, &ctx)
        .await
        .expect("view should succeed");
    assert!(view_res.contains("# Tasks"));
    assert!(view_res.contains("Phase 1 - Setup"));
    assert!(view_res.contains("Phase 2 - Execution"));
    assert!(view_res.contains("[0/4]"));

    let state = GLOBAL_TODO_STATE.read().await;
    assert_eq!(state.total_count(), 4);
    assert_eq!(state.completed_count(), 0);
    assert_eq!(state.in_progress_count(), 1);
    assert_eq!(state.pending_count(), 3);
}

#[tokio::test]
async fn test_todo_tool_lifecycle_transitions() {
    TodoTool::reset().await;
    let tool = TodoTool::new();
    let ctx = ToolContext::default();

    // 1. Initialize
    let init_args = json!({
        "op": "init",
        "list": [
            {
                "phase": "Milestone A",
                "items": ["First task", "Second task"]
            },
            {
                "phase": "Milestone B",
                "items": ["Third task"]
            }
        ]
    });
    tool.execute(init_args, &ctx).await.unwrap();

    // 2. Start a specific task
    let start_args = json!({
        "op": "start",
        "task": "Second task"
    });
    let start_res = tool.execute(start_args, &ctx).await.unwrap();
    assert!(start_res.contains("Task started"));
    assert!(start_res.contains("Second task"));

    {
        let state = GLOBAL_TODO_STATE.read().await;
        let item = state.get_task("Second task").unwrap();
        assert_eq!(item.status, TodoStatus::InProgress);
        let first = state.get_task("First task").unwrap();
        assert_eq!(first.status, TodoStatus::Pending);
    }

    // 3. Mark the in-progress task as done
    let done_args = json!({
        "op": "done",
        "task": "Second task"
    });
    let done_res = tool.execute(done_args, &ctx).await.unwrap();
    assert!(done_res.contains("Marked done"));
    assert!(done_res.contains("1/3 completed"));

    {
        let state = GLOBAL_TODO_STATE.read().await;
        assert_eq!(state.completed_count(), 1);
        let done_item = state.get_task("Second task").unwrap();
        assert_eq!(done_item.status, TodoStatus::Completed);
    }

    // 4. Block a task with a reason
    let block_args = json!({
        "op": "block",
        "task": "Third task",
        "reason": "Missing credentials"
    });
    let block_res = tool.execute(block_args, &ctx).await.unwrap();
    assert!(block_res.contains("Blocked task"));
    assert!(block_res.contains("Missing credentials"));

    {
        let state = GLOBAL_TODO_STATE.read().await;
        let blocked_item = state.get_task("Third task").unwrap();
        assert_eq!(blocked_item.status, TodoStatus::Blocked);
        assert_eq!(blocked_item.reason.as_deref(), Some("Missing credentials"));
    }

    // 5. Unblock the task
    let unblock_args = json!({
        "op": "unblock",
        "task": "Third task"
    });
    let unblock_res = tool.execute(unblock_args, &ctx).await.unwrap();
    assert!(unblock_res.contains("Unblocked task"));

    {
        let state = GLOBAL_TODO_STATE.read().await;
        let unblocked = state.get_task("Third task").unwrap();
        assert_eq!(unblocked.status, TodoStatus::Pending);
        assert!(unblocked.reason.is_none());
    }

    // 6. Drop/cancel a task
    let drop_args = json!({
        "op": "drop",
        "task": "Third task"
    });
    let drop_res = tool.execute(drop_args, &ctx).await.unwrap();
    assert!(drop_res.contains("Dropped task"));

    {
        let state = GLOBAL_TODO_STATE.read().await;
        let dropped = state.get_task("Third task").unwrap();
        assert_eq!(dropped.status, TodoStatus::Dropped);
    }

    // 7. Complete remaining task in Milestone A
    let done_all_args = json!({
        "op": "done",
        "task": "First task"
    });
    let done_all_res = tool.execute(done_all_args, &ctx).await.unwrap();
    assert!(done_all_res.contains("Marked done"));

    {
        let state = GLOBAL_TODO_STATE.read().await;
        assert_eq!(state.completed_count(), 2);
    }
}

#[tokio::test]
async fn test_todo_tool_append_operation() {
    TodoTool::reset().await;
    let tool = TodoTool::new();
    let ctx = ToolContext::default();

    let init_args = json!({
        "op": "init",
        "list": [
            {
                "phase": "Initial Phase",
                "items": ["Base task"]
            }
        ]
    });
    tool.execute(init_args, &ctx).await.unwrap();

    let append_args = json!({
        "op": "append",
        "phase": "Extended Phase",
        "items": ["New feature A", "New feature B"]
    });
    let append_res = tool.execute(append_args, &ctx).await.unwrap();
    assert!(append_res.contains("Appended 2 task(s) to phase \"Extended Phase\""));
    assert!(append_res.contains("Total tasks: 3"));

    let state = GLOBAL_TODO_STATE.read().await;
    assert_eq!(state.total_count(), 3);
    assert_eq!(state.phases.len(), 2);
    assert!(state.phases[1].name == "Extended Phase");
    assert_eq!(state.phases[1].items.len(), 2);
}

#[tokio::test]
async fn test_todo_tool_error_handling() {
    TodoTool::reset().await;
    let tool = TodoTool::new();
    let ctx = ToolContext::default();

    // Missing 'op' parameter
    let err_no_op = tool.execute(json!({}), &ctx).await;
    assert!(err_no_op.is_err());
    assert!(err_no_op
        .unwrap_err()
        .to_string()
        .contains("Missing required parameter 'op'"));

    // Unknown operation
    let err_bad_op = tool.execute(json!({ "op": "dance" }), &ctx).await;
    assert!(err_bad_op.is_err());
    assert!(err_bad_op
        .unwrap_err()
        .to_string()
        .contains("Unknown todo operation 'dance'"));

    // Start with non-existent task
    let err_start_missing = tool
        .execute(json!({ "op": "start", "task": "Ghost" }), &ctx)
        .await;
    assert!(err_start_missing.is_err());
    assert!(err_start_missing
        .unwrap_err()
        .to_string()
        .contains("no task matching 'Ghost'"));

    // Done with non-existent task
    let err_done_missing = tool
        .execute(json!({ "op": "done", "task": "Ghost" }), &ctx)
        .await;
    assert!(err_done_missing.is_err());
    assert!(err_done_missing
        .unwrap_err()
        .to_string()
        .contains("no task or phase matching 'Ghost'"));

    // Block with non-existent task
    let err_block_missing = tool
        .execute(json!({ "op": "block", "task": "Ghost" }), &ctx)
        .await;
    assert!(err_block_missing.is_err());
    assert!(err_block_missing
        .unwrap_err()
        .to_string()
        .contains("no task matching 'Ghost'"));

    // Append with empty items
    let err_append_empty = tool
        .execute(
            json!({ "op": "append", "phase": "Empty", "items": [] }),
            &ctx,
        )
        .await;
    assert!(err_append_empty.is_err());
    assert!(err_append_empty
        .unwrap_err()
        .to_string()
        .contains("requires 'items'"));

    // Init with empty list
    let err_init_empty = tool
        .execute(json!({ "op": "init", "list": [] }), &ctx)
        .await;
    assert!(err_init_empty.is_err());
    assert!(err_init_empty
        .unwrap_err()
        .to_string()
        .contains("must be a non-empty array"));
}
