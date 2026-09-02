use std::io::{stdout, Write};
use uuid::Uuid;

use crate::agent::loop_runner::AgentRunner;
use crate::agent::session::{Session, SessionSummary};
use crate::config::Config;
use crate::ui::markdown::print_markdown;

/// Represents all supported top-level interactive slash commands in Fusion REPL.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SlashCommand {
    /// Show general help or help for a specific command: `/help [command]`
    Help { command: Option<String> },
    /// Inspect or switch the active LLM model: `/model [name]`
    Model { name: Option<String> },
    /// Inspect or switch the active LLM provider: `/provider [name]`
    Provider { name: Option<String> },
    /// Enable, disable, toggle, or query advisors: `/advisors [on|off|toggle|status]`
    Advisors { state: Option<String> },
    /// Manage persistent sessions: `/session [subcommand]`
    Session(SessionCommand),
    /// Clear conversation history in the active session and reset view: `/clear`
    Clear,
    /// Exit the interactive REPL session: `/quit`
    Quit,
    /// Display active runtime environment status: `/status`
    Status,
    /// View or update runtime configuration: `/config [subcommand]`
    Config(ConfigCommand),
    /// List all available registered tools: `/tools`
    Tools,
    /// Unrecognized slash command.
    Unknown { name: String, args: Vec<String> },
}

/// Subcommands for the `/session` slash command.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SessionCommand {
    /// Display information about the currently active session: `/session info` or `/session`
    Info,
    /// List all saved sessions on disk: `/session list`
    List,
    /// Start a brand new session with optional model override: `/session new [model]`
    New { model: Option<String> },
    /// Load an existing session by full UUID or prefix: `/session load <id_or_prefix>`
    Load { id_or_prefix: String },
    /// Manually save/checkpoint the current session to disk: `/session save`
    Save,
    /// Delete a session by full UUID or prefix: `/session delete <id_or_prefix>`
    Delete { id_or_prefix: String },
    /// Clear all messages in the active session: `/session clear`
    Clear,
}

/// Subcommands for the `/config` slash command.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConfigCommand {
    /// Show current configuration values: `/config show` or `/config`
    Show,
    /// Display path to config file on disk: `/config path`
    Path,
    /// Save current configuration to disk: `/config save`
    Save,
    /// Set a configuration option: `/config set <key> <value>`
    Set { key: String, value: String },
}

/// Result returned from executing a slash command.
#[derive(Debug, Clone)]
pub enum CommandResult {
    /// The REPL should continue normally.
    Continue,
    /// The REPL should gracefully exit.
    Exit,
    /// A new session was loaded or created.
    SessionSwitched(Session),
    /// The current session history was cleared.
    SessionCleared,
    /// The terminal screen was cleared.
    ScreenCleared,
}

impl CommandResult {
    /// Returns true if this command requested the REPL loop to terminate.
    pub fn is_exit(&self) -> bool {
        matches!(self, CommandResult::Exit)
    }

    /// Alias for `is_exit`.
    pub fn should_exit(&self) -> bool {
        self.is_exit()
    }
}

impl SlashCommand {
    /// Parse a raw user input string into a `SlashCommand`.
    /// Returns `None` if the input is not a slash command (does not start with `/`).
    pub fn parse(input: &str) -> Option<Self> {
        let trimmed = input.trim();
        if !trimmed.starts_with('/') {
            return None;
        }

        let tokens = tokenize_command(trimmed);
        if tokens.is_empty() {
            return None;
        }

        let cmd = tokens[0].to_lowercase();
        let args = &tokens[1..];

        let parsed = match cmd.as_str() {
            "/help" | "/?" | "/h" => {
                let command = args.first().cloned();
                SlashCommand::Help { command }
            }
            "/model" | "/m" => {
                let name = args.first().cloned();
                SlashCommand::Model { name }
            }
            "/provider" | "/p" => {
                let name = args.first().cloned();
                SlashCommand::Provider { name }
            }
            "/advisors" | "/advisor" | "/adv" => {
                let state = args.first().cloned();
                SlashCommand::Advisors { state }
            }
            "/session" | "/s" => {
                let session_cmd = parse_session_subcommand(args);
                SlashCommand::Session(session_cmd)
            }
            "/clear" | "/cls" | "/c" => SlashCommand::Clear,
            "/quit" | "/exit" | "/q" => SlashCommand::Quit,
            "/status" | "/st" => SlashCommand::Status,
            "/config" | "/cfg" => {
                let config_cmd = parse_config_subcommand(args);
                SlashCommand::Config(config_cmd)
            }
            "/tools" | "/t" => SlashCommand::Tools,
            _ => SlashCommand::Unknown {
                name: tokens[0].clone(),
                args: args.to_vec(),
            },
        };

        Some(parsed)
    }
}

/// Tokenize command line string respecting single and double quotes.
pub fn tokenize_command(input: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut current = String::new();
    let mut in_single_quote = false;
    let mut in_double_quote = false;
    let mut chars = input.chars().peekable();

    while let Some(c) = chars.next() {
        match c {
            '\'' if !in_double_quote => {
                in_single_quote = !in_single_quote;
            }
            '"' if !in_single_quote => {
                in_double_quote = !in_double_quote;
            }
            '\\' => {
                if let Some(next_c) = chars.next() {
                    current.push(next_c);
                }
            }
            c if c.is_whitespace() && !in_single_quote && !in_double_quote => {
                if !current.is_empty() {
                    tokens.push(current.clone());
                    current.clear();
                }
            }
            _ => {
                current.push(c);
            }
        }
    }

    if !current.is_empty() {
        tokens.push(current);
    }

    tokens
}

fn parse_session_subcommand(args: &[String]) -> SessionCommand {
    if args.is_empty() {
        return SessionCommand::Info;
    }

    match args[0].to_lowercase().as_str() {
        "info" | "status" | "current" => SessionCommand::Info,
        "list" | "ls" => SessionCommand::List,
        "new" | "create" => {
            let model = args.get(1).cloned();
            SessionCommand::New { model }
        }
        "load" | "open" | "restore" | "switch" => {
            let id_or_prefix = args.get(1).cloned().unwrap_or_default();
            SessionCommand::Load { id_or_prefix }
        }
        "save" | "write" | "checkpoint" => SessionCommand::Save,
        "delete" | "del" | "rm" | "remove" => {
            let id_or_prefix = args.get(1).cloned().unwrap_or_default();
            SessionCommand::Delete { id_or_prefix }
        }
        "clear" | "reset" => SessionCommand::Clear,
        other => {
            // If argument looks like a UUID or load target
            if Uuid::parse_str(other).is_ok() || other.len() >= 6 {
                SessionCommand::Load {
                    id_or_prefix: other.to_string(),
                }
            } else {
                SessionCommand::Info
            }
        }
    }
}

fn parse_config_subcommand(args: &[String]) -> ConfigCommand {
    if args.is_empty() {
        return ConfigCommand::Show;
    }

    match args[0].to_lowercase().as_str() {
        "show" | "get" | "view" | "list" => ConfigCommand::Show,
        "path" | "file" | "dir" => ConfigCommand::Path,
        "save" | "write" => ConfigCommand::Save,
        "set" => {
            let key = args.get(1).cloned().unwrap_or_default();
            let value = args.get(2).cloned().unwrap_or_default();
            ConfigCommand::Set { key, value }
        }
        _ => ConfigCommand::Show,
    }
}

/// Convenience function to handle slash commands directly from user input.
/// Returns `Some(CommandResult)` if the input was a slash command, or `None` if it was regular prompt text.
pub fn handle_slash_command(
    input: &str,
    runner: &mut AgentRunner,
    session: &mut Session,
) -> Option<CommandResult> {
    let cmd = SlashCommand::parse(input)?;
    Some(execute_slash_command(&cmd, runner, session))
}

/// Execute a parsed `SlashCommand` against the given `AgentRunner` and active `Session`.
pub fn execute_slash_command(
    cmd: &SlashCommand,
    runner: &mut AgentRunner,
    session: &mut Session,
) -> CommandResult {
    match cmd {
        SlashCommand::Help { command } => {
            handle_help(command.as_deref());
            CommandResult::Continue
        }
        SlashCommand::Model { name } => {
            handle_model(name.as_deref(), runner, session);
            CommandResult::Continue
        }
        SlashCommand::Provider { name } => {
            handle_provider(name.as_deref(), runner);
            CommandResult::Continue
        }
        SlashCommand::Advisors { state } => {
            handle_advisors(state.as_deref(), runner);
            CommandResult::Continue
        }
        SlashCommand::Session(subcmd) => handle_session(subcmd, runner, session),
        SlashCommand::Clear => handle_clear(session),
        SlashCommand::Quit => handle_quit(),
        SlashCommand::Status => {
            handle_status(runner, session);
            CommandResult::Continue
        }
        SlashCommand::Config(subcmd) => {
            handle_config(subcmd, runner);
            CommandResult::Continue
        }
        SlashCommand::Tools => {
            handle_tools(runner);
            CommandResult::Continue
        }
        SlashCommand::Unknown { name, args } => {
            handle_unknown(name, args);
            CommandResult::Continue
        }
    }
}

// ---------------------------------------------------------------------------
// Command Handlers
// ---------------------------------------------------------------------------

fn handle_help(command: Option<&str>) {
    if let Some(topic) = command {
        match topic.to_lowercase().as_str() {
            "model" | "m" => {
                let text = r#"
# Slash Command: `/model [name]`

View or change the active LLM model used for chat completions.

### Usage
- `/model` - Display current active model and suggested models per provider.
- `/model <name>` - Switch active model to `<name>`.

### Examples
- `/model deepseek-chat`
- `/model claude-3-5-sonnet-20241022`
- `/model gpt-4o`
- `/model qwen2.5-coder:32b`
"#;
                print_markdown(text);
            }
            "provider" | "p" => {
                let text = r#"
# Slash Command: `/provider [name]`

View or change the active LLM provider.

### Supported Providers
- **deepseek** - DeepSeek AI (`DEEPSEEK_API_KEY`)
- **anthropic** - Anthropic Claude (`ANTHROPIC_API_KEY`)
- **openai** - OpenAI GPT models (`OPENAI_API_KEY`)
- **xai** - xAI Grok (`XAI_API_KEY`)
- **openrouter** - OpenRouter multi-provider gateway (`OPENROUTER_API_KEY`)
- **ollama** - Local Ollama instance (`OLLAMA_BASE_URL` or `http://localhost:11434`)

### Usage
- `/provider` - List all providers and API key configuration status.
- `/provider <name>` - Switch to provider `<name>`.
"#;
                print_markdown(text);
            }
            "advisors" | "advisor" | "adv" => {
                let text = r#"
# Slash Command: `/advisors <on|off|toggle|status>`

Control parallel advisor critiques.

When enabled, specialized domain advisors run in parallel before tool execution
and evaluate plans for security, architecture, performance, and cross-platform issues:
- **Security Advisor**: Command safety, secrets protection, privilege boundaries.
- **Architecture Advisor**: Modularity, clean layering, cross-platform compatibility.
- **Performance Advisor**: Algorithmic efficiency, zero-cost abstractions, memory overhead.
- **Linux Advisor**: POSIX conformance, system paths, procfs/sysfs nuances.
- **Mobile Advisor**: Android/Termux constraints, single-process limits, storage paths.
- **Windows Advisor**: Path separators, PowerShell/cmd quirks, line endings.

### Usage
- `/advisors on` - Enable advisors.
- `/advisors off` - Disable advisors.
- `/advisors toggle` - Toggle advisors on/off.
- `/advisors status` - View current advisor status.
"#;
                print_markdown(text);
            }
            "session" | "s" => {
                let text = r#"
# Slash Command: `/session <subcommand>`

Manage persistent conversational sessions stored in `~/.fusion/sessions/`.

### Subcommands
- `/session` or `/session info` - Display active session metadata.
- `/session list` - List all saved sessions with ID, title, and timestamp.
- `/session new [model]` - Start a fresh session (auto-saves current).
- `/session load <id_or_prefix>` - Load an existing session by UUID or prefix.
- `/session save` - Manually checkpoint active session to disk.
- `/session delete <id_or_prefix>` - Delete a saved session from disk.
- `/session clear` - Clear conversation history in the active session.
"#;
                print_markdown(text);
            }
            "clear" | "c" => {
                let text = r#"
# Slash Command: `/clear`

Clears the conversation message history in the active session and clears the terminal screen.
"#;
                print_markdown(text);
            }
            "quit" | "exit" | "q" => {
                let text = r#"
# Slash Command: `/quit`

Gracefully saves the active session and exits the Fusion interactive REPL.
Aliases: `/exit`, `/q`.
"#;
                print_markdown(text);
            }
            "status" => {
                let text = r#"
# Slash Command: `/status`

Displays runtime environment status, active provider, model, advisor state,
and session message statistics.
"#;
                print_markdown(text);
            }
            "config" | "cfg" => {
                let text = r#"
# Slash Command: `/config [show|path|save|set]`

View or modify Fusion configuration settings (`~/.fusion/config.json`).
"#;
                print_markdown(text);
            }
            "tools" | "t" => {
                let text = r#"
# Slash Command: `/tools`

Lists all registered tools available to the assistant during the conversation.
"#;
                print_markdown(text);
            }
            other => {
                println!("\x1b[1;33mℹ\x1b[0m No specific help topic found for \x1b[1;37m{}\x1b[0m. Showing general help:\n", other);
                print_general_help();
            }
        }
    } else {
        print_general_help();
    }
}

fn print_general_help() {
    let help_text = r#"
# Fusion Slash Commands Reference

| Command | Arguments | Description |
| :--- | :--- | :--- |
| `/help` | `[command]` | Show this help overview or detailed help for a command |
| `/model` | `[name]` | Inspect or switch the active LLM model |
| `/provider` | `[name]` | Inspect or switch LLM provider (deepseek, anthropic, openai, xai, openrouter, ollama) |
| `/advisors` | `<on\|off\|toggle>` | Enable or disable parallel domain advisor critiques |
| `/session` | `[subcommand]` | Manage sessions (`list`, `new`, `load`, `save`, `delete`, `info`, `clear`) |
| `/clear` | | Clear current conversation history and reset screen |
| `/status` | | Display active environment and runtime status |
| `/config` | `[show\|path\|set]` | Inspect or modify runtime configuration settings |
| `/tools` | | List all registered assistant tools |
| `/quit` | | Exit Fusion REPL (`/exit`, `/q`) |

### Keyboard Shortcuts
- `Enter` - Submit prompt
- `Alt+Enter` or `Esc+Enter` - Insert newline for multiline prompt
- `Up` / `Down` - Navigate prompt history
- `Ctrl+C` - Cancel current input or running generation
- `Ctrl+D` - Exit REPL on empty buffer
- `Ctrl+L` - Clear screen
"#;
    print_markdown(help_text);
}

fn handle_model(name: Option<&str>, runner: &mut AgentRunner, session: &mut Session) {
    if let Some(model_name) = name {
        let trimmed = model_name.trim();
        if trimmed.is_empty() {
            print_model_info(runner, session);
            return;
        }

        runner.config_mut().default_model = trimmed.to_string();
        session.set_active_model(trimmed);
        println!(
            "\x1b[1;32m✓\x1b[0m Switched active model to \x1b[1;37m{}\x1b[0m\n",
            trimmed
        );
    } else {
        print_model_info(runner, session);
    }
}

fn print_model_info(runner: &AgentRunner, session: &Session) {
    let current_model = session.active_model();
    let current_provider = &runner.config().default_provider;

    println!("\x1b[1;36mActive Model:\x1b[0m \x1b[1;37m{}\x1b[0m (Provider: \x1b[1;33m{}\x1b[0m)", current_model, current_provider);
    println!("\n\x1b[1;34mSuggested Models per Provider:\x1b[0m");
    println!("  \x1b[1;33mDeepSeek:\x1b[0m    deepseek-chat, deepseek-coder, deepseek-reasoner");
    println!("  \x1b[1;33mAnthropic:\x1b[0m   claude-3-5-sonnet-20241022, claude-3-5-haiku-20241022, claude-3-opus-20240229");
    println!("  \x1b[1;33mOpenAI:\x1b[0m      gpt-4o, gpt-4o-mini, o1, o3-mini");
    println!("  \x1b[1;33mxAI:\x1b[0m         grok-2-1212, grok-2-vision-1212, grok-beta");
    println!("  \x1b[1;33mOpenRouter:\x1b[0m  anthropic/claude-3.5-sonnet, deepseek/deepseek-chat, google/gemini-2.0-flash-001");
    println!("  \x1b[1;33mOllama:\x1b[0m      llama3.3, qwen2.5-coder:32b, deepseek-r1:14b, codellama");
    println!("\nUsage: \x1b[1;36m/model <model_name>\x1b[0m to switch.\n");
}

fn handle_provider(name: Option<&str>, runner: &mut AgentRunner) {
    if let Some(provider_name) = name {
        let trimmed = provider_name.trim().to_lowercase();
        if trimmed.is_empty() {
            print_provider_info(runner);
            return;
        }

        let canonical = match trimmed.as_str() {
            "deepseek" | "ds" => "deepseek",
            "anthropic" | "claude" => "anthropic",
            "openai" | "gpt" => "openai",
            "xai" | "grok" => "xai",
            "openrouter" | "or" => "openrouter",
            "ollama" | "local" => "ollama",
            other => {
                println!(
                    "\x1b[1;31m✗\x1b[0m Unknown provider: \x1b[1;37m{}\x1b[0m",
                    other
                );
                println!("Supported providers: deepseek, anthropic, openai, xai, openrouter, ollama\n");
                return;
            }
        };

        runner.config_mut().default_provider = canonical.to_string();
        let (key, url) = runner.config().get_key_and_url(canonical);

        if key.is_some() || canonical == "ollama" {
            println!(
                "\x1b[1;32m✓\x1b[0m Switched provider to \x1b[1;33m{}\x1b[0m (\x1b[2;37m{}\x1b[0m)\n",
                canonical, url
            );
        } else {
            let env_var = match canonical {
                "deepseek" => "DEEPSEEK_API_KEY",
                "anthropic" => "ANTHROPIC_API_KEY",
                "openai" => "OPENAI_API_KEY",
                "xai" => "XAI_API_KEY",
                "openrouter" => "OPENROUTER_API_KEY",
                _ => "API_KEY",
            };
            println!(
                "\x1b[1;33m⚠\x1b[0m Switched provider to \x1b[1;33m{}\x1b[0m, but no API key was found.",
                canonical
            );
            println!(
                "  Please set the environment variable \x1b[1;36mexport {}=...\x1b[0m or edit \x1b[2;37m~/.fusion/config.json\x1b[0m\n",
                env_var
            );
        }
    } else {
        print_provider_info(runner);
    }
}

fn print_provider_info(runner: &AgentRunner) {
    let current_provider = &runner.config().default_provider;
    println!("\x1b[1;36mConfigured LLM Providers:\x1b[0m");

    let providers = [
        ("deepseek", "DEEPSEEK_API_KEY", runner.config().deepseek_api_key.is_some(), &runner.config().deepseek_base_url),
        ("anthropic", "ANTHROPIC_API_KEY", runner.config().anthropic_api_key.is_some(), &runner.config().anthropic_base_url),
        ("openai", "OPENAI_API_KEY", runner.config().openai_api_key.is_some(), &runner.config().openai_base_url),
        ("xai", "XAI_API_KEY", runner.config().xai_api_key.is_some(), &runner.config().xai_base_url),
        ("openrouter", "OPENROUTER_API_KEY", runner.config().openrouter_api_key.is_some(), &runner.config().openrouter_base_url),
        ("ollama", "(Local)", true, &runner.config().ollama_base_url),
    ];

    for (name, env_key, is_ready, custom_url) in providers {
        let active_indicator = if name == current_provider.as_str() {
            "\x1b[1;32m* (active)\x1b[0m"
        } else {
            "          "
        };

        let status_badge = if is_ready {
            "\x1b[1;32m[Ready]\x1b[0m"
        } else {
            "\x1b[1;31m[Missing Key]\x1b[0m"
        };

        let url_info = custom_url.as_deref().unwrap_or("(default url)");

        println!(
            "  {} \x1b[1;33m{:<11}\x1b[0m {:<13} {:<20} \x1b[2;37m{}\x1b[0m",
            active_indicator, name, status_badge, env_key, url_info
        );
    }

    println!("\nUsage: \x1b[1;36m/provider <name>\x1b[0m to switch provider.\n");
}

fn handle_advisors(state: Option<&str>, runner: &mut AgentRunner) {
    if let Some(arg) = state {
        match arg.to_lowercase().as_str() {
            "on" | "enable" | "true" | "1" | "yes" => {
                runner.config_mut().advisors_enabled = true;
                println!("\x1b[1;32m✓\x1b[0m Advisors enabled.");
                println!("  Active: Security, Architecture, Performance, Linux, Mobile, Windows\n");
            }
            "off" | "disable" | "false" | "0" | "no" => {
                runner.config_mut().advisors_enabled = false;
                println!("\x1b[1;33m!\x1b[0m Advisors disabled. LLM turns will execute without parallel critiques.\n");
            }
            "toggle" => {
                let current = runner.config().advisors_enabled;
                runner.config_mut().advisors_enabled = !current;
                let new_state = if !current {
                    "\x1b[1;32menabled\x1b[0m"
                } else {
                    "\x1b[1;31mdisabled\x1b[0m"
                };
                println!("\x1b[1;32m✓\x1b[0m Advisors are now {}.\n", new_state);
            }
            "status" | "list" | "info" => {
                print_advisor_status(runner);
            }
            _ => {
                println!("Usage: \x1b[1;36m/advisors <on|off|toggle|status>\x1b[0m\n");
            }
        }
    } else {
        print_advisor_status(runner);
    }
}

fn print_advisor_status(runner: &AgentRunner) {
    let enabled = runner.config().advisors_enabled;
    let badge = if enabled {
        "\x1b[1;32mENABLED\x1b[0m"
    } else {
        "\x1b[1;31mDISABLED\x1b[0m"
    };

    println!("\x1b[1;36mAdvisor Subsystem Status:\x1b[0m [{}]", badge);
    println!("\n\x1b[1;34mAvailable Domain Advisors:\x1b[0m");
    println!("  • \x1b[1;35mSecurity Advisor:\x1b[0m     Command safety, secrets leakage, permission escalation");
    println!("  • \x1b[1;35mArchitecture Advisor:\x1b[0m Modularity, layering, cross-platform compatibility");
    println!("  • \x1b[1;35mPerformance Advisor:\x1b[0m  Zero-cost abstractions, memory overhead, concurrency");
    println!("  • \x1b[1;35mLinux Advisor:\x1b[0m        POSIX standards, procfs/sysfs, environment sanity");
    println!("  • \x1b[1;35mMobile Advisor:\x1b[0m       Android/Termux compatibility, constrained memory");
    println!("  • \x1b[1;35mWindows Advisor:\x1b[0m      Path separators, cmd/powershell nuances, CRLF");
    println!("\nUsage: \x1b[1;36m/advisors on\x1b[0m or \x1b[1;36m/advisors off\x1b[0m\n");
}

fn handle_session(
    subcmd: &SessionCommand,
    runner: &mut AgentRunner,
    session: &mut Session,
) -> CommandResult {
    match subcmd {
        SessionCommand::Info => {
            println!("\x1b[1;36mActive Session Details:\x1b[0m");
            println!("  ID:         \x1b[1;37m{}\x1b[0m", session.id());
            if let Some(title) = &session.title {
                println!("  Title:      \x1b[1;33m{}\x1b[0m", title);
            }
            println!("  Model:      \x1b[1;37m{}\x1b[0m", session.active_model());
            println!("  Created:    \x1b[2;37m{}\x1b[0m", session.created_at);
            println!("  Updated:    \x1b[2;37m{}\x1b[0m", session.updated_at);
            println!("  Messages:   \x1b[1;32m{}\x1b[0m", session.total_messages());
            println!(
                "  Path:       \x1b[2;37m{}\x1b[0m",
                Session::session_path(session.id()).display()
            );
            println!();
            CommandResult::Continue
        }
        SessionCommand::List => {
            match Session::list_sessions() {
                Ok(sessions) => {
                    if sessions.is_empty() {
                        println!("\x1b[1;33mℹ\x1b[0m No saved sessions found in \x1b[2;37m{}\x1b[0m\n", Session::sessions_dir().display());
                    } else {
                        println!("\x1b[1;36mSaved Sessions:\x1b[0m ({} total)", sessions.len());
                        for s in &sessions {
                            let is_current = s.id == session.id();
                            let marker = if is_current {
                                "\x1b[1;32m* (active)\x1b[0m"
                            } else {
                                "          "
                            };

                            let title_str = s.title.as_deref().unwrap_or("Untitled session");
                            let short_id: String = s.id.to_string().chars().take(8).collect();

                            println!(
                                "  {} \x1b[1;37m{}\x1b[0m  \x1b[1;33m{:<25}\x1b[0m  \x1b[2;37m({} msgs, {})\x1b[0m",
                                marker, short_id, title_str, s.message_count, s.active_model
                            );
                            println!("               \x1b[2;37m└─ \"{}\"\x1b[0m", s.preview);
                        }
                        println!("\nUsage: \x1b[1;36m/session load <id>\x1b[0m to restore a session.\n");
                    }
                }
                Err(e) => {
                    println!("\x1b[1;31m✗\x1b[0m Failed to list sessions: {}\n", e);
                }
            }
            CommandResult::Continue
        }
        SessionCommand::New { model } => {
            // Auto-save current session before switching if it contains messages
            if session.total_messages() > 0 {
                let _ = session.save();
            }

            let model_name = model
                .clone()
                .unwrap_or_else(|| runner.config().default_model.clone());
            let new_session = Session::new(&model_name);
            let new_id = new_session.id();
            *session = new_session.clone();

            println!(
                "\x1b[1;32m✓\x1b[0m Started new session \x1b[1;37m{}\x1b[0m (model: \x1b[1;33m{}\x1b[0m)\n",
                new_id, model_name
            );
            CommandResult::SessionSwitched(new_session)
        }
        SessionCommand::Load { id_or_prefix } => {
            if id_or_prefix.is_empty() {
                println!("Usage: \x1b[1;36m/session load <session_id_or_prefix>\x1b[0m\n");
                return CommandResult::Continue;
            }

            // Attempt exact UUID parse first
            let target_uuid = if let Ok(parsed) = Uuid::parse_str(id_or_prefix) {
                Some(parsed)
            } else {
                // Search list of sessions for matching prefix
                match Session::list_sessions() {
                    Ok(summaries) => {
                        let matches: Vec<&SessionSummary> = summaries
                            .iter()
                            .filter(|s| s.id.to_string().starts_with(id_or_prefix))
                            .collect();

                        if matches.is_empty() {
                            println!(
                                "\x1b[1;31m✗\x1b[0m No session found matching prefix: \x1b[1;37m{}\x1b[0m\n",
                                id_or_prefix
                            );
                            return CommandResult::Continue;
                        } else if matches.len() > 1 {
                            println!(
                                "\x1b[1;33m⚠\x1b[0m Ambiguous prefix: matched {} sessions:",
                                matches.len()
                            );
                            for m in matches {
                                println!("  • {}", m.id);
                            }
                            println!("Please specify more characters.\n");
                            return CommandResult::Continue;
                        } else {
                            Some(matches[0].id)
                        }
                    }
                    Err(e) => {
                        println!("\x1b[1;31m✗\x1b[0m Failed to list sessions: {}\n", e);
                        return CommandResult::Continue;
                    }
                }
            };

            if let Some(uuid) = target_uuid {
                match Session::load(uuid) {
                    Ok(loaded) => {
                        // Auto-save active session first
                        if session.total_messages() > 0 {
                            let _ = session.save();
                        }

                        let title = loaded.title.clone().unwrap_or_else(|| "Untitled".to_string());
                        let count = loaded.total_messages();
                        let active_model = loaded.active_model.clone();

                        runner.config_mut().default_model = active_model.clone();
                        *session = loaded.clone();

                        println!(
                            "\x1b[1;32m✓\x1b[0m Loaded session \x1b[1;37m{}\x1b[0m (\"{}\", {} messages, model: \x1b[1;33m{}\x1b[0m)\n",
                            uuid, title, count, active_model
                        );
                        return CommandResult::SessionSwitched(loaded);
                    }
                    Err(e) => {
                        println!("\x1b[1;31m✗\x1b[0m Failed to load session {}: {}\n", uuid, e);
                    }
                }
            }

            CommandResult::Continue
        }
        SessionCommand::Save => {
            match session.save() {
                Ok(path) => {
                    println!(
                        "\x1b[1;32m✓\x1b[0m Session saved to \x1b[2;37m{}\x1b[0m ({} messages)\n",
                        path.display(),
                        session.total_messages()
                    );
                }
                Err(e) => {
                    println!("\x1b[1;31m✗\x1b[0m Failed to save session: {}\n", e);
                }
            }
            CommandResult::Continue
        }
        SessionCommand::Delete { id_or_prefix } => {
            if id_or_prefix.is_empty() {
                println!("Usage: \x1b[1;36m/session delete <session_id_or_prefix>\x1b[0m\n");
                return CommandResult::Continue;
            }

            let target_uuid = if let Ok(parsed) = Uuid::parse_str(id_or_prefix) {
                Some(parsed)
            } else {
                match Session::list_sessions() {
                    Ok(summaries) => summaries
                        .iter()
                        .find(|s| s.id.to_string().starts_with(id_or_prefix))
                        .map(|s| s.id),
                    Err(_) => None,
                }
            };

            if let Some(uuid) = target_uuid {
                match Session::delete(uuid) {
                    Ok(_) => {
                        println!("\x1b[1;32m✓\x1b[0m Deleted session \x1b[1;37m{}\x1b[0m\n", uuid);
                    }
                    Err(e) => {
                        println!("\x1b[1;31m✗\x1b[0m Failed to delete session: {}\n", e);
                    }
                }
            } else {
                println!(
                    "\x1b[1;31m✗\x1b[0m No session found matching \x1b[1;37m{}\x1b[0m\n",
                    id_or_prefix
                );
            }
            CommandResult::Continue
        }
        SessionCommand::Clear => {
            session.clear();
            println!("\x1b[1;32m✓\x1b[0m Session conversation history cleared.\n");
            CommandResult::SessionCleared
        }
    }
}

fn handle_clear(session: &mut Session) -> CommandResult {
    session.clear();
    // ANSI clear screen and move cursor to home position
    print!("\x1b[2J\x1b[1;1H");
    let _ = stdout().flush();
    println!("\x1b[1;32m✓\x1b[0m Screen and conversation history cleared.\n");
    CommandResult::ScreenCleared
}

fn handle_quit() -> CommandResult {
    println!("\x1b[2;37mGoodbye!\x1b[0m");
    CommandResult::Exit
}

fn handle_status(runner: &AgentRunner, session: &Session) {
    let cfg = runner.config();
    println!("\x1b[1;36mFusion Runtime Status:\x1b[0m");
    println!("  Provider:         \x1b[1;33m{}\x1b[0m", cfg.default_provider);
    println!("  Model:            \x1b[1;37m{}\x1b[0m", cfg.default_model);
    println!(
        "  Advisors:         {}",
        if cfg.advisors_enabled {
            "\x1b[1;32mEnabled (Security, Architecture, Performance, Linux, Mobile, Windows)\x1b[0m"
        } else {
            "\x1b[1;31mDisabled\x1b[0m"
        }
    );
    println!("  Active Session:   \x1b[1;37m{}\x1b[0m", session.id());
    println!("  Session Messages: \x1b[1;32m{}\x1b[0m", session.total_messages());
    println!("  Registered Tools: \x1b[1;34m{}\x1b[0m", runner.tools().definitions().len());
    println!("  Working Dir:      \x1b[2;37m{}\x1b[0m", runner.tool_ctx().cwd.display());
    println!();
}

fn handle_config(subcmd: &ConfigCommand, runner: &mut AgentRunner) {
    match subcmd {
        ConfigCommand::Show => {
            let cfg = runner.config();
            if let Ok(json) = serde_json::to_string_pretty(cfg) {
                println!("\x1b[1;36mCurrent Configuration:\x1b[0m");
                println!("{}\n", json);
            }
        }
        ConfigCommand::Path => {
            println!(
                "\x1b[1;36mConfig File Path:\x1b[0m \x1b[2;37m{}\x1b[0m\n",
                Config::config_path().display()
            );
        }
        ConfigCommand::Save => match runner.config().save() {
            Ok(_) => {
                println!(
                    "\x1b[1;32m✓\x1b[0m Configuration saved to \x1b[2;37m{}\x1b[0m\n",
                    Config::config_path().display()
                );
            }
            Err(e) => {
                println!("\x1b[1;31m✗\x1b[0m Failed to save config: {}\n", e);
            }
        },
        ConfigCommand::Set { key, value } => {
            let cfg = runner.config_mut();
            match key.to_lowercase().as_str() {
                "default_provider" | "provider" => {
                    cfg.default_provider = value.clone();
                    println!("\x1b[1;32m✓\x1b[0m Updated default_provider to {}\n", value);
                }
                "default_model" | "model" => {
                    cfg.default_model = value.clone();
                    println!("\x1b[1;32m✓\x1b[0m Updated default_model to {}\n", value);
                }
                "advisors_enabled" | "advisors" => {
                    let enabled = value.parse::<bool>().unwrap_or(true);
                    cfg.advisors_enabled = enabled;
                    println!("\x1b[1;32m✓\x1b[0m Updated advisors_enabled to {}\n", enabled);
                }
                _ => {
                    println!("\x1b[1;31m✗\x1b[0m Unknown config key: {}\n", key);
                }
            }
        }
    }
}

fn handle_tools(runner: &AgentRunner) {
    let defs = runner.tools().definitions();
    println!("\x1b[1;36mRegistered Tools:\x1b[0m ({} total)", defs.len());
    for tool in defs {
        println!("  • \x1b[1;33m{:<15}\x1b[0m \x1b[2;37m{}\x1b[0m", tool.name, tool.description);
    }
    println!();
}

fn handle_unknown(name: &str, _args: &[String]) {
    println!(
        "\x1b[1;31mUnknown command:\x1b[0m \x1b[1;37m{}\x1b[0m. Type \x1b[1;36m/help\x1b[0m to see available commands.\n",
        name
    );
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_help() {
        assert_eq!(
            SlashCommand::parse("/help"),
            Some(SlashCommand::Help { command: None })
        );
        assert_eq!(
            SlashCommand::parse("/?"),
            Some(SlashCommand::Help { command: None })
        );
        assert_eq!(
            SlashCommand::parse("/h model"),
            Some(SlashCommand::Help {
                command: Some("model".to_string())
            })
        );
    }

    #[test]
    fn test_parse_model() {
        assert_eq!(
            SlashCommand::parse("/model"),
            Some(SlashCommand::Model { name: None })
        );
        assert_eq!(
            SlashCommand::parse("/model claude-3-5-sonnet-20241022"),
            Some(SlashCommand::Model {
                name: Some("claude-3-5-sonnet-20241022".to_string())
            })
        );
        assert_eq!(
            SlashCommand::parse("/m deepseek-chat"),
            Some(SlashCommand::Model {
                name: Some("deepseek-chat".to_string())
            })
        );
    }

    #[test]
    fn test_parse_provider() {
        assert_eq!(
            SlashCommand::parse("/provider"),
            Some(SlashCommand::Provider { name: None })
        );
        assert_eq!(
            SlashCommand::parse("/provider anthropic"),
            Some(SlashCommand::Provider {
                name: Some("anthropic".to_string())
            })
        );
        assert_eq!(
            SlashCommand::parse("/p ollama"),
            Some(SlashCommand::Provider {
                name: Some("ollama".to_string())
            })
        );
    }

    #[test]
    fn test_parse_advisors() {
        assert_eq!(
            SlashCommand::parse("/advisors"),
            Some(SlashCommand::Advisors { state: None })
        );
        assert_eq!(
            SlashCommand::parse("/advisors on"),
            Some(SlashCommand::Advisors {
                state: Some("on".to_string())
            })
        );
        assert_eq!(
            SlashCommand::parse("/adv off"),
            Some(SlashCommand::Advisors {
                state: Some("off".to_string())
            })
        );
    }

    #[test]
    fn test_parse_session() {
        assert_eq!(
            SlashCommand::parse("/session"),
            Some(SlashCommand::Session(SessionCommand::Info))
        );
        assert_eq!(
            SlashCommand::parse("/session list"),
            Some(SlashCommand::Session(SessionCommand::List))
        );
        assert_eq!(
            SlashCommand::parse("/s new gpt-4o"),
            Some(SlashCommand::Session(SessionCommand::New {
                model: Some("gpt-4o".to_string())
            }))
        );
        assert_eq!(
            SlashCommand::parse("/session clear"),
            Some(SlashCommand::Session(SessionCommand::Clear))
        );
    }

    #[test]
    fn test_parse_clear_and_quit() {
        assert_eq!(SlashCommand::parse("/clear"), Some(SlashCommand::Clear));
        assert_eq!(SlashCommand::parse("/cls"), Some(SlashCommand::Clear));
        assert_eq!(SlashCommand::parse("/c"), Some(SlashCommand::Clear));

        assert_eq!(SlashCommand::parse("/quit"), Some(SlashCommand::Quit));
        assert_eq!(SlashCommand::parse("/exit"), Some(SlashCommand::Quit));
        assert_eq!(SlashCommand::parse("/q"), Some(SlashCommand::Quit));
    }

    #[test]
    fn test_parse_non_slash() {
        assert_eq!(SlashCommand::parse("hello world"), None);
        assert_eq!(SlashCommand::parse(""), None);
        assert_eq!(SlashCommand::parse("   "), None);
    }

    #[test]
    fn test_tokenize_with_quotes() {
        let tokens = tokenize_command(r#"/model "gpt-4o mini" --temp '0.7'"#);
        assert_eq!(tokens, vec!["/model", "gpt-4o mini", "--temp", "0.7"]);
    }

    #[test]
    fn test_execute_model_switch() {
        let client = crate::provider::LlmClient::new();
        let config = Config::default();
        let tools = crate::tools::ToolRegistry::new();
        let tool_ctx = crate::tools::ToolContext {
            cwd: std::path::PathBuf::from("."),
            env: std::collections::HashMap::new(),
        };
        let mut runner = AgentRunner::new(client, config, tools, tool_ctx);
        let mut session = Session::new("deepseek-chat");

        let res = handle_slash_command("/model claude-3-5-sonnet", &mut runner, &mut session);
        assert!(res.is_some());
        assert!(!res.unwrap().is_exit());
        assert_eq!(runner.config().default_model, "claude-3-5-sonnet");
        assert_eq!(session.active_model(), "claude-3-5-sonnet");
    }

    #[test]
    fn test_execute_provider_switch() {
        let client = crate::provider::LlmClient::new();
        let config = Config::default();
        let tools = crate::tools::ToolRegistry::new();
        let tool_ctx = crate::tools::ToolContext {
            cwd: std::path::PathBuf::from("."),
            env: std::collections::HashMap::new(),
        };
        let mut runner = AgentRunner::new(client, config, tools, tool_ctx);
        let mut session = Session::new("deepseek-chat");

        let res = handle_slash_command("/provider anthropic", &mut runner, &mut session);
        assert!(res.is_some());
        assert_eq!(runner.config().default_provider, "anthropic");
    }

    #[test]
    fn test_execute_advisors_toggle() {
        let client = crate::provider::LlmClient::new();
        let mut config = Config::default();
        config.advisors_enabled = true;
        let tools = crate::tools::ToolRegistry::new();
        let tool_ctx = crate::tools::ToolContext {
            cwd: std::path::PathBuf::from("."),
            env: std::collections::HashMap::new(),
        };
        let mut runner = AgentRunner::new(client, config, tools, tool_ctx);
        let mut session = Session::new("deepseek-chat");

        let res = handle_slash_command("/advisors off", &mut runner, &mut session);
        assert!(res.is_some());
        assert!(!runner.config().advisors_enabled);

        let res2 = handle_slash_command("/advisors on", &mut runner, &mut session);
        assert!(res2.is_some());
        assert!(runner.config().advisors_enabled);

        let res3 = handle_slash_command("/advisors toggle", &mut runner, &mut session);
        assert!(res3.is_some());
        assert!(!runner.config().advisors_enabled);
    }

    #[test]
    fn test_execute_session_clear_and_new() {
        let client = crate::provider::LlmClient::new();
        let config = Config::default();
        let tools = crate::tools::ToolRegistry::new();
        let tool_ctx = crate::tools::ToolContext {
            cwd: std::path::PathBuf::from("."),
            env: std::collections::HashMap::new(),
        };
        let mut runner = AgentRunner::new(client, config, tools, tool_ctx);
        let mut session = Session::new("deepseek-chat");
        session.add_user_message("Hello world");
        assert_eq!(session.total_messages(), 1);

        let orig_id = session.id();
        let res = handle_slash_command("/session clear", &mut runner, &mut session);
        assert!(matches!(res, Some(CommandResult::SessionCleared)));
        assert_eq!(session.total_messages(), 0);

        let res_new = handle_slash_command("/session new gpt-4o", &mut runner, &mut session);
        assert!(matches!(res_new, Some(CommandResult::SessionSwitched(_))));
        assert_ne!(session.id(), orig_id);
        assert_eq!(session.active_model(), "gpt-4o");
    }

    #[test]
    fn test_execute_quit() {
        let client = crate::provider::LlmClient::new();
        let config = Config::default();
        let tools = crate::tools::ToolRegistry::new();
        let tool_ctx = crate::tools::ToolContext {
            cwd: std::path::PathBuf::from("."),
            env: std::collections::HashMap::new(),
        };
        let mut runner = AgentRunner::new(client, config, tools, tool_ctx);
        let mut session = Session::new("deepseek-chat");

        let res = handle_slash_command("/quit", &mut runner, &mut session);
        assert!(matches!(res, Some(CommandResult::Exit)));
        assert!(res.unwrap().is_exit());
    }
}
