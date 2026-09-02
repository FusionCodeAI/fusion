use clap::Parser;
use std::path::PathBuf;

#[derive(Parser, Debug)]
#[command(
    name = "fusion",
    version = "0.3.0",
    about = "Fast, lightweight AI coding assistant with subagents and advisors"
)]
pub struct Cli {
    /// Optional one-off prompt to run non-interactively or slash command
    #[arg(value_name = "PROMPT")]
    pub prompt: Option<String>,

    /// Override model (e.g. deepseek-chat, claude-3-5-sonnet-20241022, gpt-4o)
    #[arg(short, long)]
    pub model: Option<String>,

    /// Override provider (deepseek, anthropic, openai, xai, openrouter, ollama)
    #[arg(short, long)]
    pub provider: Option<String>,

    /// Working directory (defaults to current directory)
    #[arg(short = 'C', long, value_name = "DIR")]
    pub cwd: Option<PathBuf>,

    /// Disable parallel advisor critiques
    #[arg(long)]
    pub no_advisors: bool,

    /// Start Agent Client Protocol (ACP) JSON-RPC stdio server
    #[arg(long)]
    pub acp: bool,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Initialize tracing
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .with_writer(std::io::stderr)
        .init();

    let cli = Cli::parse();

    // Load config and apply CLI overrides
    let mut config = fusion::config::Config::load();
    if let Some(m) = cli.model {
        config.default_model = m;
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
                    res?;
                }
                _ = tokio::signal::ctrl_c() => {
                    eprintln!("\nOperation canceled by user");
                }
            }
        }
    } else {
        // Interactive REPL with inline Ratatui UI
        fusion::ui::run_repl(runner).await?;
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
}
