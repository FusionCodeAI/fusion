//! Slash command handlers for DAG-based autonomous planning (`/goal` and `/plan`).
//!
//! Provides:
//! - `/goal <prompt>`: Decomposes high-level user intent into a structured `SubagentDag`
//!   and returns a visual ASCII preview along with the generated DAG.
//! - `/plan <subcommand>`: Manages and inspects the active plan (`status`, `stages`, `cancel`).

use crate::agent::plan_runner::{PlanRunner, PlanRunnerError, PlanSummary};
use crate::agent::planner_dag::{DecompositionStrategy, SubagentDag, TaskDecomposer};
use crate::agent::subagent::SubagentManager;

/// Alias constant for standard feature decomposition strategy.
#[allow(non_upper_case_globals)]
pub const StandardFeature: DecompositionStrategy = DecompositionStrategy::SoftwareFeature {
    split_frontend_backend: true,
    include_security_audit: true,
};

/// Handles the `/goal <prompt>` slash command.
///
/// Decomposes `goal_prompt` into a `SubagentDag` using the standard feature decomposition
/// strategy and returns a tuple of `(preview_string, dag)`.
pub fn handle_goal_command(goal_prompt: &str) -> (String, crate::agent::planner_dag::SubagentDag) {
    let strategy = DecompositionStrategy::default();
    let dag = TaskDecomposer::decompose(goal_prompt, strategy).unwrap_or_else(|_| {
        let title = goal_prompt
            .lines()
            .next()
            .map(|l| l.trim())
            .filter(|l| !l.is_empty())
            .unwrap_or("Task");
        let mut fallback = SubagentDag::new(format!("Plan: {}", title), goal_prompt);
        let _ = fallback.compute_stages();
        fallback
    });

    let preview = dag.to_ascii_tree();
    (preview, dag)
}

/// Handles the `/plan [subcommand]` slash command.
///
/// Supported subcommands:
/// - `"status"`: Displays the ASCII execution status / timeline tree of the active DAG plan.
/// - `"stages"`: Lists all stages and their constituent task nodes with descriptions.
/// - `"run"` / `"execute"` / `"start"`: Runs or inspects execution of the active plan.
/// - `"cancel"`: Resets the active plan.
pub fn handle_plan_command(args: &[String], current_dag: Option<&SubagentDag>) -> String {
    let subcommand = match args.first() {
        Some(cmd) => cmd.trim().to_lowercase(),
        None => {
            return format_plan_help();
        }
    };

    match subcommand.as_str() {
        "status" => match current_dag {
            Some(dag) => dag.to_ascii_tree(),
            None => "No active plan. Use /goal <prompt> to create a plan.".to_string(),
        },
        "stages" => match current_dag {
            Some(dag) => format_stages(dag),
            None => "No active plan. Use /goal <prompt> to create a plan.".to_string(),
        },
        "cancel" => match current_dag {
            Some(_) => "Active plan has been cancelled and reset.".to_string(),
            None => "No active plan to cancel.".to_string(),
        },
        "run" | "execute" | "start" => match current_dag {
            Some(dag) => {
                let summary = PlanSummary::from_dag(dag);
                format_execution_report(&summary)
            }
            None => "No active plan to run. Use /goal <objective> first to create a plan.".to_string(),
        },
        "help" | "-h" | "--help" => format_plan_help(),
        other => format!(
            "Unknown plan subcommand: '{}'. Available subcommands: status, stages, cancel.\n{}",
            other,
            format_plan_help()
        ),
    }
}

/// Formats detailed stage and task node breakdown with descriptions.
fn format_stages(dag: &SubagentDag) -> String {
    let mut out = String::new();
    out.push_str(&format!("Stages for Plan: {} (ID: {})\n", dag.name, dag.id));
    out.push_str(&format!("Goal: {}\n", dag.goal));
    out.push_str(&"=".repeat(60));
    out.push('\n');

    if dag.stages.is_empty() {
        if dag.tasks.is_empty() {
            out.push_str("No stages or tasks defined in active plan.\n");
        } else {
            out.push_str("\nTasks (Unpartitioned):\n");
            let mut tasks: Vec<_> = dag.tasks.values().collect();
            tasks.sort_by_key(|t| &t.id);
            for task in tasks {
                out.push_str(&format!(
                    "  • [{}] {} (Role: {}, Status: {})\n",
                    task.id,
                    task.title,
                    task.role,
                    task.status.status_label()
                ));
                out.push_str(&format!("    Description: {}\n", task.description.trim()));
                if !task.dependencies.is_empty() {
                    out.push_str(&format!("    Dependencies: {}\n", task.dependencies.join(", ")));
                }
            }
        }
    } else {
        for stage in &dag.stages {
            let stage_title = if stage.name.to_lowercase().starts_with("stage") {
                format!("▶ {} [{}]", stage.name, stage.status)
            } else {
                format!("▶ Stage {}: {} [{}]", stage.stage_index + 1, stage.name, stage.status)
            };
            out.push_str(&format!("\n{}\n", stage_title));

            for task_id in &stage.task_ids {
                if let Some(task) = dag.tasks.get(task_id) {
                    let dep_str = if task.dependencies.is_empty() {
                        "root".to_string()
                    } else {
                        format!("after: {}", task.dependencies.join(", "))
                    };
                    out.push_str(&format!(
                        "  • [{}] {} (Role: {}, Status: {}, Deps: {})\n",
                        task.id,
                        task.title,
                        task.role,
                        task.status.status_label(),
                        dep_str
                    ));
                    out.push_str(&format!("    Description: {}\n", task.description.trim()));
                } else {
                    out.push_str(&format!("  • [{}] (task details missing)\n", task_id));
                }
            }
        }
    }

    out
}

/// Returns formatted usage help for `/plan`.
fn format_plan_help() -> String {
    let mut out = String::new();
    out.push_str("Usage: /plan <subcommand>\n\n");
    out.push_str("Subcommands:\n");
    out.push_str("  status  - Displays the ASCII execution status of the active DAG plan\n");
    out.push_str("  stages  - Lists all stages and their constituent task nodes with descriptions\n");
    out.push_str("  run     - Executes the active plan autonomously (aliases: execute, start)\n");
    out.push_str("  cancel  - Resets the active plan\n");
    out
}

/// Executes the active DAG plan autonomously using the provided SubagentManager.
///
/// Instantiates `PlanRunner::new(dag, manager.clone())` and runs all stages sequentially
/// via `run_autonomous().await`. Upon completion, updates `dag` in place with the final state
/// and returns the resulting `PlanSummary`.
pub async fn execute_active_plan(
    dag: &mut SubagentDag,
    manager: &SubagentManager,
) -> Result<PlanSummary, PlanRunnerError> {
    let mut runner = PlanRunner::new(dag.clone(), manager.clone());
    let result = runner.run_autonomous().await;
    *dag = runner.dag;
    result
}

/// Formats a `PlanSummary` into a clean report showing stages executed,
/// subagent tasks run, tokens spent, and final status (`[✓] Completed in N stages`).
pub fn format_execution_report(summary: &PlanSummary) -> String {
    summary.format_report()
}
