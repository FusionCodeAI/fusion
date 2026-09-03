use anyhow::{Context, Result};
use clap::{Args, Command, CommandFactory};
use clap_complete::generate;
pub use clap_complete::Shell;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

/// List of shell names supported by completion generation
pub const SUPPORTED_SHELLS: &[&str] = &["bash", "zsh", "fish", "powershell", "elvish"];

/// Parse a shell string into a `Shell` enum.
///
/// Accepts "bash", "zsh", "fish", "powershell" (or "pwsh"), and "elvish" (case-insensitive).
pub fn parse_shell(name: &str) -> Result<Shell, String> {
    match name.trim().to_ascii_lowercase().as_str() {
        "bash" => Ok(Shell::Bash),
        "zsh" => Ok(Shell::Zsh),
        "fish" => Ok(Shell::Fish),
        "powershell" | "pwsh" => Ok(Shell::PowerShell),
        "elvish" => Ok(Shell::Elvish),
        other => Err(format!(
            "Unsupported shell '{}'. Supported shells: {}",
            other,
            SUPPORTED_SHELLS.join(", ")
        )),
    }
}

/// Generates shell completion script for the given shell and command, writing to the provided writer.
pub fn generate_completion(
    shell: Shell,
    cmd: &mut Command,
    bin_name: &str,
    writer: &mut dyn io::Write,
) {
    generate(shell, cmd, bin_name, writer);
}

/// Generates shell completion script for the given shell and command as a String.
pub fn generate_completion_string(shell: Shell, cmd: &mut Command, bin_name: &str) -> String {
    let mut buf = Vec::new();
    generate(shell, cmd, bin_name, &mut buf);
    String::from_utf8_lossy(&buf).into_owned()
}

/// Generates shell completion for fusion CLI and writes it to standard output.
pub fn print_completion(shell: Shell, cmd: &mut Command) {
    let mut stdout = io::stdout();
    generate_completion(shell, cmd, "fusion", &mut stdout);
}

/// Generate completion for the default Fusion CLI command to the given writer.
pub fn generate_for_default_cli(shell: Shell, writer: &mut dyn io::Write) {
    let mut cmd = super::Cli::command();
    generate_completion(shell, &mut cmd, "fusion", writer);
}

/// Generate completion for the default Fusion CLI command and return it as a String.
pub fn generate_for_default_cli_string(shell: Shell) -> String {
    let mut cmd = super::Cli::command();
    generate_completion_string(shell, &mut cmd, "fusion")
}

/// Print completion for the default Fusion CLI command to standard output.
pub fn print_default_completion(shell: Shell) {
    let mut cmd = super::Cli::command();
    print_completion(shell, &mut cmd);
}

#[derive(Args, Debug)]
pub struct CompletionsArgs {
    /// Shell to generate completions for
    #[arg(value_name = "SHELL")]
    pub shell: String,

    /// Write the completion script to this file instead of stdout
    #[arg(short, long, value_name = "FILE")]
    pub output: Option<PathBuf>,
}

/// Builds the `completions <shell>` subcommand for registration on a CLI command tree.
///
/// Returns a `Command` named "completions" taking one positional `<shell>` argument and an
/// optional `--output <FILE>` flag, so callers can attach it via `Cli::command().subcommand(...)`.
pub fn completions_command() -> Command {
    CompletionsArgs::augment_args(Command::new("completions").about(
        "Generate shell completion script for the given shell (bash, zsh, fish, powershell, elvish)",
    ))
}

/// Dispatches a parsed `completions <shell>` invocation: renders the completion script for the
/// default Fusion CLI either to stdout or to the file given via `--output`.
pub fn run_completions(args: &CompletionsArgs) -> Result<()> {
    let shell = parse_shell(&args.shell).map_err(|e| anyhow::anyhow!("{}", e))?;
    match &args.output {
        Some(path) => write_completion_file(shell, path).map(|_| ()),
        None => {
            print_default_completion(shell);
            Ok(())
        }
    }
}

/// Detects the current shell from the `SHELL` environment variable.
///
/// Only the `SHELL` environment variable is consulted; callers needing a different source
/// should use [`detect_shell_from_path`] directly. Returns an error when the variable is
/// unset or names an unsupported shell.
pub fn detect_shell() -> Result<Shell> {
    let shell_var = std::env::var("SHELL")
        .context("SHELL environment variable is not set; cannot detect current shell")?;
    detect_shell_from_path(&shell_var)
}

/// Detects a `Shell` from a shell executable path such as `/bin/zsh` or `/usr/bin/fish`.
pub fn detect_shell_from_path(shell_path: &str) -> Result<Shell> {
    let name = Path::new(shell_path)
        .file_name()
        .and_then(|n| n.to_str())
        .context(format!(
            "Invalid SHELL path '{}': no usable executable name",
            shell_path
        ))?;
    parse_shell(name).map_err(|e| anyhow::anyhow!("{} (from SHELL='{}')", e, shell_path))
}

/// Renders the completion script for the default Fusion CLI to a file, creating parent
/// directories as needed. Returns the canonical path written.
pub fn write_completion_file(shell: Shell, path: &Path) -> Result<PathBuf> {
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() && !parent.exists() {
            fs::create_dir_all(parent)
                .with_context(|| format!("Failed to create directory {}", parent.display()))?;
        }
    }
    let script = generate_for_default_cli_string(shell);
    fs::write(path, script)
        .with_context(|| format!("Failed to write completion script to {}", path.display()))?;
    Ok(path.to_path_buf())
}

/// Renders a completion script for an arbitrary command to a file, creating parent directories
/// as needed. Returns the path written.
pub fn write_completion_file_for_cmd(
    shell: Shell,
    cmd: &mut Command,
    bin_name: &str,
    path: &Path,
) -> Result<PathBuf> {
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() && !parent.exists() {
            fs::create_dir_all(parent)
                .with_context(|| format!("Failed to create directory {}", parent.display()))?;
        }
    }
    let mut file = fs::File::create(path)
        .with_context(|| format!("Failed to create completion file {}", path.display()))?;
    generate_completion(shell, cmd, bin_name, &mut file);
    Ok(path.to_path_buf())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_shell() {
        assert_eq!(parse_shell("bash").unwrap(), Shell::Bash);
        assert_eq!(parse_shell("BASH").unwrap(), Shell::Bash);
        assert_eq!(parse_shell("zsh").unwrap(), Shell::Zsh);
        assert_eq!(parse_shell("fish").unwrap(), Shell::Fish);
        assert_eq!(parse_shell("powershell").unwrap(), Shell::PowerShell);
        assert_eq!(parse_shell("pwsh").unwrap(), Shell::PowerShell);
        assert_eq!(parse_shell("elvish").unwrap(), Shell::Elvish);

        assert!(parse_shell("cmd").is_err());
        assert!(parse_shell("unknown").is_err());
    }

    #[test]
    fn test_generate_bash_completion() {
        let script = generate_for_default_cli_string(Shell::Bash);
        assert!(!script.is_empty(), "Bash completion should not be empty");
        assert!(
            script.contains("fusion"),
            "Bash completion script must reference 'fusion'"
        );
        assert!(
            script.contains("--generate-completion"),
            "Script should contain --generate-completion flag"
        );
        assert!(
            script.contains("--model"),
            "Script should contain --model flag"
        );
        assert!(
            script.contains("--provider"),
            "Script should contain --provider flag"
        );
        assert!(script.contains("--acp"), "Script should contain --acp flag");
    }

    #[test]
    fn test_generate_zsh_completion() {
        let script = generate_for_default_cli_string(Shell::Zsh);
        assert!(!script.is_empty(), "Zsh completion should not be empty");
        assert!(
            script.contains("fusion") || script.contains("#compdef"),
            "Zsh completion script must reference 'fusion' or #compdef"
        );
        assert!(
            script.contains("--generate-completion"),
            "Script should contain --generate-completion flag"
        );
    }

    #[test]
    fn test_generate_fish_completion() {
        let script = generate_for_default_cli_string(Shell::Fish);
        assert!(!script.is_empty(), "Fish completion should not be empty");
        assert!(
            script.contains("complete -c fusion"),
            "Fish completion script should define completions for command 'fusion'"
        );
        assert!(
            script.contains("generate-completion"),
            "Fish completion should include generate-completion"
        );
    }

    #[test]
    fn test_generate_powershell_completion() {
        let script = generate_for_default_cli_string(Shell::PowerShell);
        assert!(
            !script.is_empty(),
            "PowerShell completion should not be empty"
        );
        assert!(
            script.contains("fusion"),
            "PowerShell completion script must reference 'fusion'"
        );
        assert!(
            script.contains("Register-ArgumentCompleter"),
            "PowerShell completion script should use Register-ArgumentCompleter"
        );
    }

    #[test]
    fn test_generate_elvish_completion() {
        let script = generate_for_default_cli_string(Shell::Elvish);
        assert!(!script.is_empty(), "Elvish completion should not be empty");
        assert!(
            script.contains("fusion"),
            "Elvish completion script must reference 'fusion'"
        );
    }

    #[test]
    fn test_generate_completion_to_writer() {
        let mut buf = Vec::new();
        generate_for_default_cli(Shell::Bash, &mut buf);
        let output = String::from_utf8(buf).unwrap();
        assert!(output.contains("fusion"));
    }
}
