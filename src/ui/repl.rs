use std::io::{stdout, Write};

use crate::agent::{AgentEvent, AgentRunner, Session};
use crate::config::Config;
use crate::ui::markdown::{print_markdown, MarkdownRenderer};
use crate::ui::prompt::{Prompt, PromptResult};
use crate::ui::spinner::{Spinner, SpinnerHandle};

/// Print the stylized startup banner.
pub fn print_banner(config: &Config) {
    let advisor_status = if config.advisors_enabled {
        "\x1b[1;32mon\x1b[0m"
    } else {
        "\x1b[1;31moff\x1b[0m"
    };

    println!(
        r#"
  \x1b[1;36m✦ Fusion v0.3.0\x1b[0m \x1b[2;37m(Pure-Rust AI Coding Assistant)\x1b[0m
  \x1b[2;37mProvider:\x1b[0m \x1b[1;33m{}\x1b[0m  \x1b[2;37mModel:\x1b[0m \x1b[1;37m{}\x1b[0m  \x1b[2;37mAdvisors:\x1b[0m {}
  \x1b[2;37mType your prompt, \x1b[0m\x1b[36m/help\x1b[0m\x1b[2;37m for commands, or \x1b[0m\x1b[36mCtrl+D\x1b[0m\x1b[2;37m / \x1b[0m\x1b[36m/exit\x1b[0m\x1b[2;37m to quit.\x1b[0m
"#,
        config.default_provider, config.default_model, advisor_status
    );
    let _ = stdout().flush();
}

/// Print help information for available slash commands.
pub fn print_help() {
    let help_text = r#"
# Fusion Commands & Shortcuts

### Commands
- `/help` - Show this help message
- `/clear` - Clear session conversation history
- `/model <model_name>` - Switch the active LLM model (e.g. `deepseek-chat`, `gpt-4o`)
- `/provider <name>` - Switch provider (`deepseek`, `anthropic`, `openai`, `ollama`)
- `/advisors <on|off>` - Toggle advisor critique subsystem
- `/status` - Show current session configuration and status
- `/exit` or `/quit` - Exit Fusion

### Keybindings
- `Enter` - Submit prompt
- `Ctrl+J` / `Shift+Enter` - Insert newline for multiline prompt
- `\` at end of line + `Enter` - Multiline continuation
- `Up` / `Down` - Navigate prompt history
- `Ctrl+C` - Cancel current input / turn
- `Ctrl+D` - Exit when prompt is empty
- `Ctrl+L` - Clear screen
"#;
    print_markdown(help_text);
}

/// Handle interactive slash commands. Returns true if the REPL should exit.
pub fn handle_command(
    cmd: &str,
    runner: &mut AgentRunner,
    session: &mut Session,
) -> bool {
    if let Some(result) = crate::ui::slash::handle_slash_command(cmd, runner, session) {
        result.is_exit()
    } else {
        false
    }
}

/// Execute a turn with rich streaming UI (markdown rendering, tool spinners, advisor critiques).
pub async fn run_turn_ui(
    runner: &AgentRunner,
    session: &mut Session,
    user_input: &str,
) -> anyhow::Result<String> {
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();

    let event_handler = tokio::spawn(async move {
        let mut md = MarkdownRenderer::new();
        let mut active_spinner: Option<SpinnerHandle> = None;

        while let Some(event) = rx.recv().await {
            match event {
                AgentEvent::AdvisorStarted { advisor, .. } => {
                    if let Some(sp) = active_spinner.take() {
                        sp.stop();
                    }
                    active_spinner = Some(Spinner::for_advisor(&advisor, "reviewing plan..."));
                }
                AgentEvent::AdvisorCritique { advisor, approved, critique } => {
                    if let Some(sp) = active_spinner.take() {
                        let summary = format!("[{}] {}", advisor, critique);
                        if approved {
                            sp.success(&summary);
                        } else {
                            sp.warn(&summary);
                        }
                    }
                }
                AgentEvent::ToolStarted { name, args, .. } => {
                    if let Some(sp) = active_spinner.take() {
                        sp.stop();
                    }
                    let target = match args {
                        serde_json::Value::Object(map) => {
                            if let Some(serde_json::Value::String(path)) = map.get("path") {
                                path.clone()
                            } else if let Some(serde_json::Value::String(cmd)) = map.get("command") {
                                cmd.clone()
                            } else if let Some(serde_json::Value::String(pat)) = map.get("pattern") {
                                pat.clone()
                            } else {
                                String::new()
                            }
                        }
                        _ => String::new(),
                    };
                    active_spinner = Some(Spinner::for_tool(&name, &target));
                }
                AgentEvent::ToolFinished { name, success, output, duration, .. } => {
                    if let Some(sp) = active_spinner.take() {
                        let first_line = output.lines().next().unwrap_or("");
                        let summary = format!("[{}] in {:.1?}: {}", name, duration, first_line);
                        if success {
                            sp.success(&summary);
                        } else {
                            sp.error(&summary);
                        }
                    }
                }
                AgentEvent::SubagentStarted { name, task } => {
                    if let Some(sp) = active_spinner.take() {
                        sp.stop();
                    }
                    active_spinner = Some(Spinner::for_subagent(&name, &task));
                }
                AgentEvent::SubagentFinished { name, success, output } => {
                    if let Some(sp) = active_spinner.take() {
                        let first_line = output.lines().next().unwrap_or("");
                        let summary = format!("[Agent:{}] {}", name, first_line);
                        if success {
                            sp.success(&summary);
                        } else {
                            sp.error(&summary);
                        }
                    }
                }
                AgentEvent::TextDelta(chunk) => {
                    if let Some(sp) = active_spinner.take() {
                        sp.stop();
                    }
                    md.push(&chunk);
                }
                AgentEvent::Status(_status) => {}
                AgentEvent::Error(err) => {
                    if let Some(sp) = active_spinner.take() {
                        sp.error(&err);
                    } else {
                        eprintln!("\x1b[1;31mError:\x1b[0m {}", err);
                    }
                }
                AgentEvent::Finished { .. } => {
                    if let Some(sp) = active_spinner.take() {
                        sp.stop();
                    }
                    md.finish();
                }
                _ => {}
            }
        }

        if let Some(sp) = active_spinner.take() {
            sp.stop();
        }
        md.finish();
    });

    let res = runner.run_turn_stream(session, user_input, tx).await;
    let _ = event_handler.await;
    res
}

/// Run the interactive REPL loop.
pub async fn run_repl(mut runner: AgentRunner) -> anyhow::Result<()> {
    print_banner(runner.config());

    let mut session = Session::new(&runner.config().default_model);
    let mut prompt = Prompt::new();

    loop {
        match prompt.read_input() {
            Ok(PromptResult::Submit(input)) => {
                let trimmed = input.trim();
                if trimmed.is_empty() {
                    continue;
                }

                // Handle slash commands
                if trimmed.starts_with('/') {
                    if handle_command(trimmed, &mut runner, &mut session) {
                        break;
                    }
                    continue;
                }

                // Execute agent turn with rich UI
                if let Err(e) = run_turn_ui(&runner, &mut session, trimmed).await {
                    eprintln!("\x1b[1;31mError:\x1b[0m {}\n", e);
                }
                println!();
            }
            Ok(PromptResult::Cancel) => {
                println!("\x1b[2;37m(Turn canceled)\x1b[0m\n");
            }
            Ok(PromptResult::Exit) => {
                println!("\x1b[2;37mGoodbye!\x1b[0m");
                break;
            }
            Err(e) => {
                eprintln!("Error reading input: {}", e);
                break;
            }
        }
    }

    Ok(())
}
