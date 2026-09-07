pub mod completion;

use clap::Parser;
use std::path::PathBuf;
#[derive(clap::Subcommand, Debug, Clone, PartialEq, Eq)]
pub enum Commands {
    /// Log in to Fusion Code AI via browser authorization
    Login {
        /// Optional direct API key
        #[arg(long)]
        key: Option<String>,
    },
}


/// Command line interface arguments for fusion
#[derive(Parser, Debug, Clone)]
#[command(
    name = "fusion",
    version = env!("CARGO_PKG_VERSION"),
    about = "Fast, lightweight AI coding assistant with subagents and advisors"
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Option<Commands>,

    /// Optional one-off prompt to run non-interactively or slash command
    #[arg(value_name = "PROMPT")]
    pub prompt: Option<String>,

    /// Override model (e.g. deepseek-chat, claude-3-5-sonnet-20241022, gpt-4o)
    #[arg(short, long)]
    pub model: Option<String>,

    /// Override provider (deepseek, anthropic, openai, xai, openrouter, ollama)
    #[arg(short, long)]
    pub provider: Option<String>,

    /// Apply pre-built configuration preset (coding-fast, deep-reasoning, cheap, offline-ollama, termux-mobile)
    #[arg(short = 'P', long, value_name = "PRESET")]
    pub preset: Option<String>,

    /// Working directory (defaults to current directory)
    #[arg(short = 'C', long, value_name = "DIR")]
    pub cwd: Option<PathBuf>,

    /// Disable parallel advisor critiques
    #[arg(long)]
    pub no_advisors: bool,

    /// Start Agent Client Protocol (ACP) JSON-RPC stdio server
    #[arg(long)]
    pub acp: bool,

    /// Generate shell completion script (bash, zsh, fish, powershell, elvish)
    #[arg(long = "generate-completion", value_name = "SHELL")]
    pub generate_completion: Option<clap_complete::Shell>,

    /// Resume a previously saved session by id or prefix
    #[arg(short = 'r', long, value_name = "SESSION_ID")]
    pub resume: Option<String>,

    /// Maximum execution turns per agent turn (defaults to 100, clamped 10..=500)
    #[arg(long, value_name = "N")]
    pub max_turns: Option<usize>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cli_preset_flag() {
        let cli = Cli::try_parse_from(["fusion", "--preset", "coding-fast"]).unwrap();
        assert_eq!(cli.preset.as_deref(), Some("coding-fast"));

        let cli_short = Cli::try_parse_from(["fusion", "-P", "offline-ollama"]).unwrap();
        assert_eq!(cli_short.preset.as_deref(), Some("offline-ollama"));
    }

    #[test]
    fn test_cli_max_turns_flag() {
        let cli = Cli::try_parse_from(["fusion", "--max-turns", "75"]).unwrap();
        assert_eq!(cli.max_turns, Some(75));
    }

    #[test]
    fn test_cli_login_subcommand() {
        let cli = Cli::try_parse_from(["fusion", "login"]).unwrap();
        assert_eq!(cli.command, Some(Commands::Login { key: None }));

        let cli_with_key =
            Cli::try_parse_from(["fusion", "login", "--key", "sk-fusion-123"]).unwrap();
        assert_eq!(
            cli_with_key.command,
            Some(Commands::Login {
                key: Some("sk-fusion-123".to_string())
            })
        );
    }
}
