pub mod completion;

use clap::Parser;
use std::path::PathBuf;

/// Command line interface arguments for fusion
#[derive(Parser, Debug, Clone)]
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
}
