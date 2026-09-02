use std::io::{stdout, Write};

use crate::agent::{AgentEvent, AgentRunner, Session};
use crate::config::Config;
use crate::ui::markdown::MarkdownRenderer;
use crate::ui::prompt::{Prompt, PromptResult};

pub fn print_banner(_config: &Config) {}

pub fn print_help() {
    crate::ui::slash::print_command_palette(None);
}

pub fn handle_command(cmd: &str, runner: &mut AgentRunner, session: &mut Session) -> bool {
    if let Some(result) = crate::ui::slash::handle_slash_command(cmd, runner, session) {
        result.is_exit()
    } else {
        false
    }
}

pub async fn run_turn_ui(
    runner: &AgentRunner,
    session: &mut Session,
    user_input: &str,
) -> anyhow::Result<String> {
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    let runner_task = runner.run_turn_stream(session, user_input, tx);
    tokio::pin!(runner_task);

    let mut was_thinking = false;
    let mut md = MarkdownRenderer::new();

    loop {
        tokio::select! {
            res = &mut runner_task => {
                while let Ok(event) = rx.try_recv() {
                    match event {
                        AgentEvent::TextDelta(d) => {
                            if was_thinking {
                                eprintln!();
                                was_thinking = false;
                            }
                            md.push(&d);
                        }
                        AgentEvent::ThinkingDelta(th) => {
                            was_thinking = true;
                            eprint!("\x1b[2;3m{}\x1b[0m", th);
                            let _ = std::io::stderr().flush();
                        }
                        AgentEvent::Finished { .. } => {
                            md.finish();
                        }
                        _ => {}
                    }
                }
                md.finish();
                return res;
            }
            Some(event) = rx.recv() => {
                match event {
                    AgentEvent::ThinkingDelta(th) => {
                        was_thinking = true;
                        eprint!("\x1b[2;3m{}\x1b[0m", th);
                        let _ = std::io::stderr().flush();
                    }
                    AgentEvent::TextDelta(d) => {
                        if was_thinking {
                            eprintln!();
                            was_thinking = false;
                        }
                        md.push(&d);
                    }
                    AgentEvent::ToolStarted { name, args, .. } => {
                        md.finish();
                        println!("\n⚙️  Tool [{}] with args: {}", name, args);
                    }
                    AgentEvent::ToolFinished { name, success, output, duration, .. } => {
                        let status = if success { "✓" } else { "✗" };
                        let preview: String = output.lines().take(3).collect::<Vec<_>>().join("\n");
                        println!("  {} Tool [{}] finished in {:.2?}: {}", status, name, duration, preview);
                    }
                    AgentEvent::Error(err) => {
                        eprintln!("\n❌ Error: {}", err);
                    }
                    AgentEvent::Finished { .. } => {
                        md.finish();
                    }
                    _ => {}
                }
            }
        }
    }
}

/// Run the interactive lightweight inline REPL loop.
pub async fn run_repl(mut runner: AgentRunner) -> anyhow::Result<()> {
    let mut session = Session::new(&runner.config().default_model);
    let mut prompt = Prompt::new()
        .with_model(&runner.config().default_model)
        .with_models(model_picker_list(&crate::provider::catalog::get_catalog()));

    // Clear any leftover recovery crash state so we start cleanly with a blank prompt
    let _ = runner.recovery().clear();

    loop {
        prompt.set_model(&runner.config().default_model);
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
                    prompt.set_model(&runner.config().default_model);
                    continue;
                }

                // Execute turn with live streaming
                let _ = run_turn_ui(&runner, &mut session, trimmed).await;
                println!();
            }
            Ok(PromptResult::Cancel) => {
                println!("\x1b[2;37m(Turn canceled)\x1b[0m\n");
            }
            Ok(PromptResult::Exit) => {
                match session.save() {
                    Ok(path) => println!(
                        "\x1b[2;37mSession saved. Resume later with \x1b[1;36m/session load {}\x1b[0m",
                        path.file_stem().map(|s| s.to_string_lossy()).unwrap_or_default()
                    ),
                    Err(_) => println!("\x1b[2;37mGoodbye!\x1b[0m"),
                }
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

/// Build the model list for the prompt picker dialog, showing only Fusion models.
fn model_picker_list(_catalog: &crate::provider::catalog::ModelCatalog) -> Vec<(String, String)> {
    vec![
        (
            "deepseek-ai/DeepSeek-V4-Flash-0731".to_string(),
            "DeepSeek V4 Flash (1M context · fast)".to_string(),
        ),
        (
            "MiniMaxAI/MiniMax-M2.7".to_string(),
            "MiniMax M2.7 (Reasoning · coding)".to_string(),
        ),
        (
            "moonshotai/Kimi-K2.6".to_string(),
            "Kimi K2.6 (Reasoning · 200K context)".to_string(),
        ),
    ]
}
