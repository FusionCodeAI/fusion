use clap::{CommandFactory, Parser};
pub use fusion::cli::Cli;
use std::path::PathBuf;
#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();

    if let Some(shell) = cli.generate_completion {
        fusion::cli::completion::print_completion(shell, &mut Cli::command());
        return Ok(());
    }

    // Initialize tracing
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .with_writer(std::io::stderr)
        .init();
    // Load config and apply CLI overrides
    let mut config = fusion::config::Config::load();
    if let Some(preset_name) = &cli.preset {
        if let Some(preset) = fusion::config::ConfigPreset::from_str_loose(preset_name) {
            config.apply_preset(preset);
        } else {
            eprintln!(
                "Warning: Unknown preset '{}'. Available presets: {}",
                preset_name,
                fusion::config::presets::available_presets_list()
            );
        }
    }
    if let Some(m) = cli.model {
        let (prov, resolved) = fusion::config::Config::resolve_model(
            &m,
            cli.provider.as_deref().or(Some(&config.default_provider)),
        );
        config.default_model = resolved;
        if cli.provider.is_none() {
            config.default_provider = prov;
        }
    }
    if let Some(p) = cli.provider {
        config.default_provider = p;
    }
    if cli.no_advisors {
        config.advisors_enabled = false;
    }

    let cwd = cli
        .cwd
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));

    // Create tool registry with standard cross-platform tools
    let tools = fusion::tools::default_registry();
    let tool_ctx = fusion::tools::ToolContext {
        cwd,
        env: std::env::vars().collect(),
    };

    let client = fusion::provider::LlmClient::new();
    let mut runner = fusion::agent::AgentRunner::new(client, config, tools, tool_ctx);

    if cli.acp {
        // Agent Client Protocol (ACP) stdio adapter mode for editors & IDEs
        tokio::select! {
            res = fusion::acp::run_stdio_server(runner) => {
                res?;
            }
            _ = tokio::signal::ctrl_c() => {
                tracing::info!("Received interrupt signal, shutting down ACP server");
            }
        }
    } else if let Some(user_prompt) = cli.prompt {
        let trimmed = user_prompt.trim();
        let mut session = fusion::agent::Session::new(&runner.config().default_model);

        if trimmed.starts_with('/') {
            // Slash command execution in single-run mode
            fusion::ui::slash::handle_slash_command(trimmed, &mut runner, &mut session);
        } else {
            // Non-interactive single turn
            tokio::select! {
                res = runner.run_turn(&mut session, trimmed) => {
                    match &res {
                        Ok(_) => {
                            fusion::ui::sound::play_turn_complete(runner.config());
                            fusion::ui::notify::notify_turn_complete(
                                runner.config(),
                                "Turn Complete",
                                &runner.config().default_model,
                                None,
                            );
                        }
                        Err(err) => {
                            fusion::ui::sound::play_error(runner.config());
                            fusion::ui::notify::notify_error(
                                runner.config(),
                                "Turn Error",
                                &err.to_string(),
                            );
                        }
                    }
                    res?;
                }
                _ = tokio::signal::ctrl_c() => {
                    eprintln!("\nOperation canceled by user");
                }
            }
        }
    } else {
        // Interactive REPL with inline Ratatui UI
        let initial_session = if let Some(session_id) = &cli.resume {
            match fusion::agent::Session::load_from_str(session_id) {
                Ok(s) => Some(s),
                Err(e) => {
                    eprintln!("Failed to resume session '{}': {}", session_id, e);
                    None
                }
            }
        } else {
            None
        };
        fusion::ui::run_repl_with_session(runner, initial_session).await?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cli_acp_flag() {
        let cli = Cli::try_parse_from(["fusion", "--acp"]).unwrap();
        assert!(cli.acp);
        assert!(cli.prompt.is_none());
    }

    #[test]
    fn test_cli_model_and_provider_flags() {
        let cli = Cli::try_parse_from([
            "fusion",
            "--model",
            "claude-3-5-sonnet",
            "--provider",
            "anthropic",
            "--no-advisors",
            "-C",
            "/tmp",
        ])
        .unwrap();

        assert_eq!(cli.model.as_deref(), Some("claude-3-5-sonnet"));
        assert_eq!(cli.provider.as_deref(), Some("anthropic"));
        assert!(cli.no_advisors);
        assert_eq!(cli.cwd, Some(PathBuf::from("/tmp")));
        assert!(!cli.acp);
    }

    #[test]
    fn test_cli_slash_command_prompt() {
        let cli = Cli::try_parse_from(["fusion", "/help"]).unwrap();
        assert_eq!(cli.prompt.as_deref(), Some("/help"));
    }

    #[test]
    fn test_cli_single_prompt() {
        let cli = Cli::try_parse_from(["fusion", "explain main.rs"]).unwrap();
        assert_eq!(cli.prompt.as_deref(), Some("explain main.rs"));
    }

    #[test]
    fn test_cli_generate_completion_flag() {
        let cli = Cli::try_parse_from(["fusion", "--generate-completion", "bash"]).unwrap();
        assert_eq!(cli.generate_completion, Some(clap_complete::Shell::Bash));

        let cli_zsh = Cli::try_parse_from(["fusion", "--generate-completion", "zsh"]).unwrap();
        assert_eq!(cli_zsh.generate_completion, Some(clap_complete::Shell::Zsh));

        let cli_fish = Cli::try_parse_from(["fusion", "--generate-completion", "fish"]).unwrap();
        assert_eq!(
            cli_fish.generate_completion,
            Some(clap_complete::Shell::Fish)
        );

        let cli_pwsh =
            Cli::try_parse_from(["fusion", "--generate-completion", "powershell"]).unwrap();
        assert_eq!(
            cli_pwsh.generate_completion,
            Some(clap_complete::Shell::PowerShell)
        );
    }

    #[test]
    fn test_cli_preset_parsing() {
        let cli = Cli::try_parse_from(["fusion", "--preset", "deep-reasoning"]).unwrap();
        assert_eq!(cli.preset.as_deref(), Some("deep-reasoning"));

        let mut config = fusion::config::Config::default();
        let preset =
            fusion::config::ConfigPreset::from_str_loose(cli.preset.as_deref().unwrap()).unwrap();
        config.apply_preset(preset);
        assert_eq!(config.default_provider, "deepseek");
        assert_eq!(config.default_model, "deepseek-reasoner");
    }
}
