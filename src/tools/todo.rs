use async_trait::async_trait;
use serde_json::{json, Value};
use std::sync::LazyLock;
use tokio::sync::RwLock;

use crate::agent::todo_state::TodoState;
use crate::tools::types::{Tool, ToolContext};

/// Global thread-safe task tracker state shared across autonomous agent workflows.
pub static GLOBAL_TODO_STATE: LazyLock<RwLock<TodoState>> =
    LazyLock::new(|| RwLock::new(TodoState::new()));

/// Autonomous task checklist and execution state manager (arXiv:2608.26263).
///
/// Provides a unified tool interface for LLM agents to plan, track, execute,
/// block, unblock, and complete multi-phase tasks with full state persistence.
#[derive(Debug, Clone, Default)]
pub struct TodoTool;

impl TodoTool {
    /// Creates a new `TodoTool` instance.
    pub fn new() -> Self {
        Self
    }

    /// Resets the global todo state to an empty checklist.
    /// Primarily used for test isolation and clean session starts.
    pub async fn reset() {
        let mut state = GLOBAL_TODO_STATE.write().await;
        *state = TodoState::new();
    }
}

#[async_trait]
impl Tool for TodoTool {
    fn name(&self) -> &str {
        "todo"
    }

    fn description(&self) -> &str {
        "Tracks and manages phased task execution state across autonomous workflows (arXiv:2608.26263).\n\
         Operations via 'op':\n\
         - 'init': Initialize task checklist with 'list' ([{\"phase\": \"...\", \"items\": [\"...\"]}])\n\
         - 'start': Mark a task as in-progress using 'task' (task ID, exact text, or substring)\n\
         - 'done': Mark a task or entire phase as completed using 'task' or 'phase'\n\
         - 'block': Mark a task as blocked with an optional 'reason'\n\
         - 'unblock': Move a blocked task back to pending using 'task'\n\
         - 'drop' / 'rm': Cancel or drop a task using 'task'\n\
         - 'append': Append new tasks to a phase using 'phase' and 'items'\n\
         - 'view': Inspect current checklist, task progress, and completion statistics"
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "op": {
                    "type": "string",
                    "enum": ["init", "start", "done", "rm", "drop", "block", "unblock", "append", "view"],
                    "description": "Operation to perform: 'init', 'start', 'done', 'rm', 'drop', 'block', 'unblock', 'append', or 'view'."
                },
                "list": {
                    "type": "array",
                    "description": "Array of phased task groups for 'init'. Each item contains 'phase' (string) and 'items' (array of strings).",
                    "items": {
                        "type": "object",
                        "properties": {
                            "phase": {
                                "type": "string",
                                "description": "The name of the execution phase or milestone."
                            },
                            "items": {
                                "type": "array",
                                "items": {
                                    "type": "string"
                                },
                                "description": "List of task descriptions within this phase."
                            }
                        },
                        "required": ["phase", "items"]
                    }
                },
                "task": {
                    "type": "string",
                    "description": "Task identifier (#ID), exact title, or substring query for start, done, block, unblock, or drop."
                },
                "phase": {
                    "type": "string",
                    "description": "Target phase name for append or phase-level done operations."
                },
                "items": {
                    "type": "array",
                    "items": {
                        "type": "string"
                    },
                    "description": "Array of task descriptions to append to a phase."
                },
                "reason": {
                    "type": "string",
                    "description": "Optional explanation or blocker detail when marking a task as blocked."
                }
            },
            "required": ["op"]
        })
    }

    async fn execute(&self, args: Value, _ctx: &ToolContext) -> anyhow::Result<String> {
        let op = args
            .get("op")
            .and_then(|v| v.as_str())
            .map(|s| s.trim().to_lowercase())
            .ok_or_else(|| anyhow::anyhow!("Missing required parameter 'op'"))?;

        let extract_task_query = || -> Option<String> {
            args.get("task")
                .and_then(|v| v.as_str())
                .or_else(|| args.get("query").and_then(|v| v.as_str()))
                .map(|s| s.trim().to_string())
                .or_else(|| {
                    args.get("id").and_then(|v| {
                        if let Some(s) = v.as_str() {
                            Some(s.trim().to_string())
                        } else if let Some(n) = v.as_i64() {
                            Some(n.to_string())
                        } else {
                            None
                        }
                    })
                })
                .filter(|s| !s.is_empty())
        };

        let extract_phase = || -> Option<String> {
            args.get("phase")
                .and_then(|v| v.as_str())
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
        };

        match op.as_str() {
            "init" => {
                let mut phases_and_tasks: Vec<(String, Vec<String>)> = Vec::new();

                if let Some(list_val) = args.get("list").and_then(|v| v.as_array()) {
                    for entry in list_val {
                        if let Some(obj) = entry.as_object() {
                            let phase_name = obj
                                .get("phase")
                                .and_then(|v| v.as_str())
                                .unwrap_or("General")
                                .trim()
                                .to_string();
                            let mut tasks = Vec::new();
                            if let Some(items) = obj.get("items").and_then(|v| v.as_array()) {
                                for item in items {
                                    if let Some(t) = item.as_str() {
                                        let trimmed = t.trim();
                                        if !trimmed.is_empty() {
                                            tasks.push(trimmed.to_string());
                                        }
                                    }
                                }
                            }
                            if !tasks.is_empty() {
                                phases_and_tasks.push((phase_name, tasks));
                            }
                        }
                    }
                }

                // Fallback: top-level "phase" with "items" or single "task"
                if phases_and_tasks.is_empty() {
                    let phase_name = extract_phase().unwrap_or_else(|| "General".to_string());
                    let mut tasks = Vec::new();
                    if let Some(items) = args.get("items").and_then(|v| v.as_array()) {
                        for item in items {
                            if let Some(t) = item.as_str() {
                                let trimmed = t.trim();
                                if !trimmed.is_empty() {
                                    tasks.push(trimmed.to_string());
                                }
                            }
                        }
                    } else if let Some(single) = extract_task_query() {
                        tasks.push(single);
                    }
                    if !tasks.is_empty() {
                        phases_and_tasks.push((phase_name, tasks));
                    }
                }

                if phases_and_tasks.is_empty() {
                    anyhow::bail!("Cannot initialize todo tracker: 'list' must be a non-empty array of phased items.");
                }

                let mut state = GLOBAL_TODO_STATE.write().await;
                state.init(phases_and_tasks);

                let active_desc = state
                    .current_task()
                    .map(|t| format!(" Current active task: #{} \"{}\" (Phase: {}).", t.id, t.task, t.phase))
                    .unwrap_or_default();

                Ok(format!(
                    "Initialized todo tracker with {} task(s) across {} phase(s).{}\n\n{}",
                    state.total_count(),
                    state.phases.len(),
                    active_desc,
                    state.view()
                ))
            }

            "start" => {
                let query = extract_task_query().ok_or_else(|| {
                    anyhow::anyhow!("Operation 'start' requires a 'task' parameter (ID, exact name, or substring).")
                })?;

                let mut state = GLOBAL_TODO_STATE.write().await;
                let ok = state.start(&query);
                if !ok {
                    anyhow::bail!(
                        "Failed to start task: no task matching '{}' found in todo list.\n\nCurrent checklist:\n{}",
                        query,
                        state.view()
                    );
                }

                let active_info = state
                    .current_task()
                    .map(|t| format!("#{} \"{}\" ({})", t.id, t.task, t.phase))
                    .unwrap_or_else(|| query.clone());

                Ok(format!(
                    "Task started: {}\nProgress: {}/{} completed ({} in progress, {} pending, {} blocked).\n\n{}",
                    active_info,
                    state.completed_count(),
                    state.total_count(),
                    state.in_progress_count(),
                    state.pending_count(),
                    state.blocked_count(),
                    state.view()
                ))
            }

            "done" => {
                let query = extract_task_query()
                    .or_else(extract_phase)
                    .ok_or_else(|| {
                        anyhow::anyhow!("Operation 'done' requires a 'task' or 'phase' parameter.")
                    })?;

                let mut state = GLOBAL_TODO_STATE.write().await;
                let ok = state.done(&query);
                if !ok {
                    anyhow::bail!(
                        "Failed to mark completed: no task or phase matching '{}' found in todo list.\n\nCurrent checklist:\n{}",
                        query,
                        state.view()
                    );
                }

                let next_info = match state.current_task() {
                    Some(t) => format!("Next active task: #{} \"{}\" (Phase: {}).", t.id, t.task, t.phase),
                    None if state.completed_count() == state.total_count() && state.total_count() > 0 => {
                        "All tasks completed successfully!".to_string()
                    }
                    None => "No tasks currently in progress.".to_string(),
                };

                Ok(format!(
                    "Marked done: \"{}\". {}\nProgress: {}/{} completed ({} in progress, {} pending, {} blocked).\n\n{}",
                    query,
                    next_info,
                    state.completed_count(),
                    state.total_count(),
                    state.in_progress_count(),
                    state.pending_count(),
                    state.blocked_count(),
                    state.view()
                ))
            }

            "block" => {
                let query = extract_task_query().ok_or_else(|| {
                    anyhow::anyhow!("Operation 'block' requires a 'task' parameter (ID, exact name, or substring).")
                })?;

                let reason = args
                    .get("reason")
                    .and_then(|v| v.as_str())
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty());

                let mut state = GLOBAL_TODO_STATE.write().await;
                let ok = state.block(&query, reason.clone());
                if !ok {
                    anyhow::bail!(
                        "Failed to block task: no task matching '{}' found in todo list.\n\nCurrent checklist:\n{}",
                        query,
                        state.view()
                    );
                }

                let reason_desc = reason
                    .map(|r| format!(" (Reason: {})", r))
                    .unwrap_or_default();

                let next_info = match state.current_task() {
                    Some(t) => format!("Next active task: #{} \"{}\" (Phase: {}).", t.id, t.task, t.phase),
                    None => "No tasks currently in progress.".to_string(),
                };

                Ok(format!(
                    "Blocked task matching '{}'{}. {}\nProgress: {}/{} completed ({} blocked, {} in progress, {} pending).\n\n{}",
                    query,
                    reason_desc,
                    next_info,
                    state.completed_count(),
                    state.total_count(),
                    state.blocked_count(),
                    state.in_progress_count(),
                    state.pending_count(),
                    state.view()
                ))
            }

            "unblock" => {
                let query = extract_task_query().ok_or_else(|| {
                    anyhow::anyhow!("Operation 'unblock' requires a 'task' parameter (ID, exact name, or substring).")
                })?;

                let mut state = GLOBAL_TODO_STATE.write().await;
                let ok = state.unblock(&query);
                if !ok {
                    anyhow::bail!(
                        "Failed to unblock task: no blocked task matching '{}' found in todo list.\n\nCurrent checklist:\n{}",
                        query,
                        state.view()
                    );
                }

                Ok(format!(
                    "Unblocked task matching '{}'. Status moved back to pending.\nProgress: {}/{} completed ({} blocked, {} in progress, {} pending).\n\n{}",
                    query,
                    state.completed_count(),
                    state.total_count(),
                    state.blocked_count(),
                    state.in_progress_count(),
                    state.pending_count(),
                    state.view()
                ))
            }

            "drop" | "rm" => {
                let query = extract_task_query().ok_or_else(|| {
                    anyhow::anyhow!("Operation '{}' requires a 'task' parameter (ID, exact name, or substring).", op)
                })?;

                let mut state = GLOBAL_TODO_STATE.write().await;
                let ok = state.drop_task(&query);
                if !ok {
                    anyhow::bail!(
                        "Failed to drop task: no task matching '{}' found in todo list.\n\nCurrent checklist:\n{}",
                        query,
                        state.view()
                    );
                }

                let next_info = match state.current_task() {
                    Some(t) => format!("Next active task: #{} \"{}\" (Phase: {}).", t.id, t.task, t.phase),
                    None => "No tasks currently in progress.".to_string(),
                };

                Ok(format!(
                    "Dropped task matching '{}'. {}\nProgress: {}/{} completed ({} dropped, {} in progress, {} pending).\n\n{}",
                    query,
                    next_info,
                    state.completed_count(),
                    state.total_count(),
                    state.dropped_count(),
                    state.in_progress_count(),
                    state.pending_count(),
                    state.view()
                ))
            }

            "append" => {
                let phase_name = extract_phase().unwrap_or_else(|| "General".to_string());
                let mut tasks = Vec::new();

                if let Some(items) = args.get("items").and_then(|v| v.as_array()) {
                    for item in items {
                        if let Some(s) = item.as_str() {
                            let trimmed = s.trim();
                            if !trimmed.is_empty() {
                                tasks.push(trimmed.to_string());
                            }
                        }
                    }
                } else if let Some(single) = extract_task_query() {
                    tasks.push(single);
                }

                if tasks.is_empty() {
                    anyhow::bail!("Operation 'append' requires 'items' (array of strings) or a 'task' description.");
                }

                let count = tasks.len();
                let mut state = GLOBAL_TODO_STATE.write().await;
                state.append(&phase_name, tasks);

                Ok(format!(
                    "Appended {} task(s) to phase \"{}\".\nTotal tasks: {} ({} completed, {} in progress, {} pending).\n\n{}",
                    count,
                    phase_name,
                    state.total_count(),
                    state.completed_count(),
                    state.in_progress_count(),
                    state.pending_count(),
                    state.view()
                ))
            }

            "view" => {
                let state = GLOBAL_TODO_STATE.write().await;
                Ok(format!(
                    "Todo Status: {}/{} completed ({} in progress, {} pending, {} blocked, {} dropped).\n\n{}",
                    state.completed_count(),
                    state.total_count(),
                    state.in_progress_count(),
                    state.pending_count(),
                    state.blocked_count(),
                    state.dropped_count(),
                    state.view()
                ))
            }

            _ => anyhow::bail!(
                "Unknown todo operation '{}'. Supported operations: 'init', 'start', 'done', 'rm', 'drop', 'block', 'unblock', 'append', 'view'.",
                op
            ),
        }
    }
}
