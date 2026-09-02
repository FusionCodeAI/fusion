use std::io::{stdout, Write};
use std::path::PathBuf;
use uuid::Uuid;

use crate::agent::loop_runner::AgentRunner;
use crate::agent::session::{Session, SessionSummary};
use crate::config::Config;
use crate::ui::markdown::print_markdown;

/// Supported export formats for conversational session transcripts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExportFormat {
    /// GitHub-flavored Markdown document (.md)
    Markdown,
    /// Standalone, responsive, dark-mode syntax-highlighted HTML (.html)
    Html,
    /// Unified multi-file session patch (.patch)
    Patch,
}

impl ExportFormat {
    /// Parses an export format loosely from string tokens.
    pub fn from_str_loose(s: &str) -> Option<Self> {
        match s.trim().to_lowercase().as_str() {
            "md" | "markdown" => Some(ExportFormat::Markdown),
            "html" | "htm" => Some(ExportFormat::Html),
            "patch" | "diff" | "unified" => Some(ExportFormat::Patch),
            _ => None,
        }
    }

    /// File extension associated with this format without leading dot.
    pub fn extension(&self) -> &'static str {
        match self {
            ExportFormat::Markdown => "md",
            ExportFormat::Html => "html",
            ExportFormat::Patch => "patch",
        }
    }

    /// Human readable display label.
    pub fn display_name(&self) -> &'static str {
        match self {
            ExportFormat::Markdown => "Markdown",
            ExportFormat::Html => "HTML",
            ExportFormat::Patch => "Unified Patch",
        }
    }
}

/// Represents all supported top-level interactive slash commands in Fusion REPL.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SlashCommand {
    /// Show general help, command palette, or help for a specific command: `/help [command]`
    Help { command: Option<String> },
    /// Open the rich interactive categorized command palette: `/palette [filter]`
    Palette { filter: Option<String> },
    /// Inspect or switch the active LLM model: `/model [name]`
    Model { name: Option<String> },
    /// Inspect or switch the active LLM provider: `/provider [name]`
    Provider { name: Option<String> },
    /// Log in to the Fusion API via browser: `/login`
    Login,
    /// Enable, disable, toggle, or query advisors: `/advisors [on|off|toggle|status]`
    Advisors { state: Option<String> },
    /// Manage, recall, and insert code snippets: `/snippet [subcommand]`
    Snippet { args: Vec<String> },
    /// Organize and filter sessions with customizable tags: `/tag [subcommand]`
    Tag { args: Vec<String> },
    /// Manage persistent sessions: `/session [subcommand]`
    Session(SessionCommand),
    /// Pin important turns and manage session checkpoints: `/bookmark [name]`
    Bookmark { args: Vec<String> },
    /// Branch the current session into an independent fork: `/fork [title] [turn]`
    Fork {
        title: Option<String>,
        turn: Option<usize>,
    },
    /// Undo/rewind the last N conversational turns: `/rewind [N]`
    Rewind { turns: Option<usize> },
    /// Force context window compaction to reduce token overhead: `/compact`
    Compact,
    /// Display comprehensive token consumption, session duration, and cost breakdown: `/stats`
    Stats,
    /// Export conversation transcript to Markdown or HTML: `/export [md|html] [path]`
    Export {
        format: Option<ExportFormat>,
        path: Option<String>,
    },
    /// Export sanitized privacy-respecting diagnostic trace: `/trace [path]`
    Trace { path: Option<String> },
    /// Clear conversation history in the active session and reset view: `/clear`
    Clear,
    /// Exit the interactive REPL session: `/quit`
    Quit,
    /// Display active runtime environment status: `/status`
    Status,
    /// View or update runtime configuration: `/config [subcommand]`
    Config(ConfigCommand),
    /// Apply or inspect pre-built configuration presets: `/preset [name]`
    Preset { name: Option<String> },
    /// List all available registered tools: `/tools`
    Tools,
    /// Interactive fuzzy file finder: `/file [query]`
    File { query: Option<String> },
    /// Manage extensible domain skills: `/skills [subcommand]`
    Skills(SkillsCommand),
    /// Manage, save, and load prompt templates: `/prompt [subcommand]`
    Prompt(PromptCommand),
    /// Inspect crash state and resume interrupted turns: `/recover [subcommand]`
    Recover { args: Vec<String> },
    /// Benchmark configured LLM providers measuring TTFT and tokens/sec: `/benchmark [provider] [options]`
    Benchmark { args: Vec<String> },
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
    /// Search across historical sessions: `/session search <query>`
    Search { query: String },
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
    /// Apply or list pre-built configuration presets: `/config preset [name]`
    Preset { name: Option<String> },
}

/// Subcommands for the `/skills` slash command.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SkillsCommand {
    /// List all discovered skills: `/skills list` or `/skills`
    List,
    /// View detailed information for a skill: `/skills info <name>`
    Info { name: String },
    /// Reload/rescan all skills from `.fusion/skills/` and `~/.fusion/skills/`: `/skills reload`
    Reload,
    /// Enable a skill: `/skills enable <name>`
    Enable { name: String },
    /// Disable a skill: `/skills disable <name>`
    Disable { name: String },
    /// Test trigger matching against a prompt query: `/skills test <query>`
    Test { query: String },
}

/// Subcommands for the `/prompt` slash command.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PromptCommand {
    /// List available prompt templates: `/prompt list [filter]` or `/prompt`
    List { filter: Option<String> },
    /// Save current prompt / text as a template: `/prompt save <name> [template...] [--local]`
    Save {
        name: String,
        template: Option<String>,
        local: bool,
    },
    /// Load and render a prompt template with arguments: `/prompt load <name> [args...]`
    Load {
        name: String,
        args: Vec<String>,
    },
    /// Inspect template details, metadata, and variables: `/prompt show <name>`
    Show { name: String },
    /// Delete a template from disk and memory: `/prompt delete <name>`
    Delete { name: String },
    /// Search templates: `/prompt search <query>`
    Search { query: String },
    /// Export templates to a JSON or Markdown file: `/prompt export [path]`
    Export { path: Option<String> },
    /// Import templates from a JSON or Markdown file: `/prompt import <path>`
    Import { path: String },
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

/// Metadata descriptor for a command in the rich command palette.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandDescriptor {
    /// Canonical command invocation (e.g. `"/fork"`)
    pub name: &'static str,
    /// Alternative command aliases (e.g. `&["/branch"]`)
    pub aliases: &'static [&'static str],
    /// Parameter syntax guide (e.g. `"/fork [title] [turn]"`)
    pub syntax: &'static str,
    /// Category grouping in palette
    pub category: CommandCategory,
    /// One-line summary description
    pub description: &'static str,
    /// Realistic usage examples
    pub examples: &'static [&'static str],
}

/// Category grouping for command palette presentation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum CommandCategory {
    /// Essential shell navigation, screen control, and exits
    Core,
    /// Session lifecycle, persistence, branching, and rewinding
    Session,
    /// LLM model, provider, advisors, and token/cost analytics
    Model,
    /// Tool registration, system configuration, and diagnostic tracing
    Config,
}

impl CommandCategory {
    /// Formatted header with icon for terminal display.
    pub fn display_name(&self) -> &'static str {
        match self {
            CommandCategory::Core => "⚡ Core & REPL Navigation",
            CommandCategory::Session => "🌿 Session, History & Branching",
            CommandCategory::Model => "🧠 Model, Advisors & Cost Analytics",
            CommandCategory::Config => "⚙️ Configuration, Tools & Diagnostics",
        }
    }
}

/// Static registry of all available REPL slash commands for the command palette.
pub static COMMAND_PALETTE: &[CommandDescriptor] = &[
    // Core & REPL Navigation
    CommandDescriptor {
        name: "/help",
        aliases: &["/?", "/h"],
        syntax: "/help [command]",
        category: CommandCategory::Core,
        description: "Open command palette or detailed documentation for a command",
        examples: &["/help", "/help fork", "/help model"],
    },
    CommandDescriptor {
        name: "/palette",
        aliases: &["/commands", "/pal"],
        syntax: "/palette [filter]",
        category: CommandCategory::Core,
        description: "Display the rich categorized interactive command palette",
        examples: &["/palette", "/palette session", "/commands cost"],
    },
    CommandDescriptor {
        name: "/clear",
        aliases: &["/cls", "/c"],
        syntax: "/clear",
        category: CommandCategory::Core,
        description: "Clear terminal screen and flush active session message history",
        examples: &["/clear"],
    },
    CommandDescriptor {
        name: "/file",
        aliases: &["/f", "/find"],
        syntax: "/file [query]",
        category: CommandCategory::Core,
        description: "Interactive fuzzy file finder (Ctrl+P) to find and inspect workspace files",
        examples: &["/file", "/file picker", "/f main.rs"],
    },
    CommandDescriptor {
        name: "/status",
        aliases: &["/st"],
        syntax: "/status",
        category: CommandCategory::Core,
        description: "Display runtime environment, active provider, model, and advisors",
        examples: &["/status"],
    },
    CommandDescriptor {
        name: "/quit",
        aliases: &["/exit", "/q"],
        syntax: "/quit",
        category: CommandCategory::Core,
        description: "Checkpoint current session to disk and exit the Fusion REPL",
        examples: &["/quit", "/exit"],
    },
    // Session, History & Branching
    CommandDescriptor {
        name: "/bookmark",
        aliases: &["/bm", "/mark"],
        syntax: "/bookmark [name|list|recall|checkpoint|restore|fork|pin|del]",
        category: CommandCategory::Session,
        description: "Pin important turns and manage restorable checkpoints",
        examples: &[
            "/bookmark fix-auth-bug",
            "/bookmark list",
            "/bookmark recall fix-auth-bug",
            "/bookmark checkpoint v1-stable",
            "/bookmark restore fix-auth-bug",
        ],
    },
    CommandDescriptor {
        name: "/tag",
        aliases: &["/tags"],
        syntax: "/tag <add|list|filter|remove|clear|stats> [args...]",
        category: CommandCategory::Session,
        description: "Organize, categorize, list, and filter conversational sessions by tag",
        examples: &[
            "/tag add rust-backend",
            "/tag list",
            "/tag filter rust-backend",
            "/tag remove rust-backend",
            "/tag stats",
        ],
    },
    CommandDescriptor {
        name: "/session",
        aliases: &["/s"],
        syntax: "/session <list|search|new|load|save|delete|info|clear>",
        category: CommandCategory::Session,
        description: "Manage and search persistent sessions stored in ~/.fusion/sessions",
        examples: &["/session list", "/session search \"auth bug\"", "/session load a1b2c3"],
    },
    CommandDescriptor {
        name: "/fork",
        aliases: &["/branch"],
        syntax: "/fork [title] [turn]",
        category: CommandCategory::Session,
        description: "Branch current session into an independent fork with lineage tracking",
        examples: &["/fork", "/fork \"experiment-refactor\"", "/fork 2"],
    },
    CommandDescriptor {
        name: "/rewind",
        aliases: &["/undo", "/rw"],
        syntax: "/rewind [N]",
        category: CommandCategory::Session,
        description: "Revert the last N conversation turns (default 1 turn)",
        examples: &["/rewind", "/rewind 2", "/undo"],
    },
    CommandDescriptor {
        name: "/compact",
        aliases: &["/compress"],
        syntax: "/compact",
        category: CommandCategory::Session,
        description: "Force context history compaction to prune older messages & save tokens",
        examples: &["/compact"],
    },
    CommandDescriptor {
        name: "/export",
        aliases: &["/exp"],
        syntax: "/export [md|html] [path]",
        category: CommandCategory::Session,
        description: "Export conversation transcript to formatted Markdown or standalone HTML",
        examples: &["/export", "/export html", "/export md transcript.md"],
    },
    CommandDescriptor {
        name: "/prompt",
        aliases: &["/tmpl", "/template", "/prompts"],
        syntax: "/prompt <list|save|load|show|delete|search> [args...]",
        category: CommandCategory::Session,
        description: "Save, load, and execute prompt templates with dynamic variable substitution",
        examples: &[
            "/prompt list",
            "/prompt save my-review",
            "/prompt save test-gen \"Generate unit tests for {{code}}\"",
            "/prompt load review code=\"fn sum(a: i32, b: i32) -> i32 { a + b }\"",
            "/prompt load explain",
            "/prompt show refactor",
            "/prompt search test",
        ],
    },
    CommandDescriptor {
        name: "/snippet",
        aliases: &["/snip", "/sn"],
        syntax: "/snippet <save|insert|recall|show|list|search|delete|clear|export|import> [args...]",
        category: CommandCategory::Session,
        description: "Save, recall, and insert reusable code snippets stored in ~/.fusion/snippets/",
        examples: &[
            "/snippet save auth-middleware",
            "/snippet save tokio-main",
            "/snippet insert auth-middleware",
            "/snippet show auth-middleware",
            "/snippet list",
            "/snippet search auth",
            "/snippet delete auth-middleware",
        ],
    },
    CommandDescriptor {
        name: "/recover",
        aliases: &["/recovery", "/rec"],
        syntax: "/recover [status|resume|diff|discard]",
        category: CommandCategory::Session,
        description: "Inspect crash state and resume interrupted turns from .fusion/recovery.json",
        examples: &[
            "/recover",
            "/recover status",
            "/recover resume continue",
            "/recover resume replay",
            "/recover diff",
            "/recover discard",
        ],
    },
    // Model, Advisors & Cost Analytics
    CommandDescriptor {
        name: "/model",
        aliases: &["/m"],
        syntax: "/model [name]",
        category: CommandCategory::Model,
        description: "Inspect or switch active LLM completion model",
        examples: &["/model", "/model claude-3-5-sonnet-20241022", "/model deepseek-chat"],
    },
    CommandDescriptor {
        name: "/provider",
        aliases: &["/p"],
        syntax: "/provider [name]",
        category: CommandCategory::Model,
        description: "Inspect or switch active LLM provider (deepseek, anthropic, openai, ollama...)",
        examples: &["/provider", "/provider anthropic", "/provider openrouter"],
    },
    CommandDescriptor {
        name: "/login",
        aliases: &["/auth", "/signin"],
        syntax: "/login",
        category: CommandCategory::Model,
        description: "Log in to the Fusion API via browser authorization",
        examples: &["/login"],
    },
    CommandDescriptor {
        name: "/advisors",
        aliases: &["/advisor", "/adv"],
        syntax: "/advisors <on|off|toggle|status>",
        category: CommandCategory::Model,
        description: "Control parallel pre-execution security, architecture, and performance advisors",
        examples: &["/advisors toggle", "/advisors on", "/advisors status"],
    },
    CommandDescriptor {
        name: "/stats",
        aliases: &["/usage", "/cost"],
        syntax: "/stats",
        category: CommandCategory::Model,
        description: "Display detailed token consumption, duration, and itemized USD cost analytics",
        examples: &["/stats"],
    },
    CommandDescriptor {
        name: "/benchmark",
        aliases: &["/bench", "/latency", "/speed"],
        syntax: "/benchmark [provider] [options]",
        category: CommandCategory::Model,
        description: "Benchmark configured LLM providers measuring TTFT (Time to First Token) and tokens/sec",
        examples: &[
            "/benchmark",
            "/benchmark active",
            "/benchmark deepseek",
            "/benchmark compare deepseek anthropic openai",
            "/benchmark -n 3 --parallel",
            "/benchmark --ping",
        ],
    },
    // Configuration, Tools & Diagnostics
    CommandDescriptor {
        name: "/config",
        aliases: &["/cfg"],
        syntax: "/config <show|path|save|set>",
        category: CommandCategory::Config,
        description: "View or modify runtime configuration settings (~/.fusion/config.json)",
        examples: &["/config show", "/config path", "/config set default_model gpt-4o"],
    },
    CommandDescriptor {
        name: "/tools",
        aliases: &["/t"],
        syntax: "/tools",
        category: CommandCategory::Config,
        description: "List all registered agent tools, schemas, and parameter specifications",
        examples: &["/tools"],
    },
    CommandDescriptor {
        name: "/trace",
        aliases: &["/tr"],
        syntax: "/trace [path]",
        category: CommandCategory::Config,
        description: "Export sanitized privacy-respecting diagnostic trace of agent turns and tools",
        examples: &["/trace", "/trace debug-run.md"],
    },
    CommandDescriptor {
        name: "/preset",
        aliases: &["/presets", "/pre"],
        syntax: "/preset [coding-fast|deep-reasoning|cheap|offline-ollama|termux-mobile]",
        category: CommandCategory::Config,
        description: "Apply or inspect pre-built configuration profiles (speed, reasoning, budget, offline, mobile)",
        examples: &["/preset", "/preset coding-fast", "/preset deep-reasoning", "/preset offline-ollama"],
    },
    CommandDescriptor {
        name: "/skills",
        aliases: &["/skill", "/sk"],
        syntax: "/skills <list|info|reload|enable|disable|test>",
        category: CommandCategory::Config,
        description: "Manage domain skills loaded from .fusion/skills/ and ~/.fusion/skills/",
        examples: &["/skills", "/skills list", "/skills info cloudflare", "/skills reload", "/skills test \"wrangler deploy\""],
    },
];

/// Returns all static command palette entries.
pub fn get_command_palette() -> &'static [CommandDescriptor] {
    COMMAND_PALETTE
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
            "/palette" | "/commands" | "/pal" => {
                let filter = args.first().cloned();
                SlashCommand::Palette { filter }
            }
            "/model" | "/m" => {
                let name = args.first().cloned();
                SlashCommand::Model { name }
            }
            "/login" | "/auth" | "/signin" => SlashCommand::Login,
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
            "/bookmark" | "/bm" | "/mark" => {
                SlashCommand::Bookmark {
                    args: args.to_vec(),
                }
            }
            "/fork" | "/branch" => {
                let (title, turn) = parse_fork_args(args);
                SlashCommand::Fork { title, turn }
            }
            "/rewind" | "/undo" | "/rw" => {
                let turns = parse_rewind_args(args);
                SlashCommand::Rewind { turns }
            }
            "/compact" | "/compress" => SlashCommand::Compact,
            "/stats" | "/usage" | "/cost" => SlashCommand::Stats,
            "/benchmark" | "/bench" | "/latency" | "/speed" => {
                SlashCommand::Benchmark {
                    args: args.to_vec(),
                }
            }
            "/export" | "/exp" => {
                let (format, path) = parse_export_args(args);
                SlashCommand::Export { format, path }
            }
            "/trace" | "/tr" => {
                let path = args.first().cloned();
                SlashCommand::Trace { path }
            }
            "/recover" | "/recovery" | "/rec" => SlashCommand::Recover {
                args: args.to_vec(),
            },
            "/clear" | "/cls" | "/c" => SlashCommand::Clear,
            "/quit" | "/exit" | "/q" => SlashCommand::Quit,
            "/status" | "/st" => SlashCommand::Status,
            "/config" | "/cfg" => {
                let config_cmd = parse_config_subcommand(args);
                SlashCommand::Config(config_cmd)
            }
            "/preset" | "/presets" | "/pre" => {
                let name = args.first().cloned();
                SlashCommand::Preset { name }
            }
            "/tools" | "/t" => SlashCommand::Tools,
            "/file" | "/f" | "/find" => {
                let query = args.first().cloned();
                SlashCommand::File { query }
            }
            "/skills" | "/skill" | "/sk" => {
                let skills_cmd = parse_skills_subcommand(args);
                SlashCommand::Skills(skills_cmd)
            }
            "/snippet" | "/snip" | "/sn" => {
                SlashCommand::Snippet {
                    args: args.to_vec(),
                }
            }
            "/tag" | "/tags" => SlashCommand::Tag {
                args: args.to_vec(),
            },
            "/prompt" | "/prompts" | "/tmpl" | "/template" => {
                let prompt_cmd = parse_prompt_subcommand(args);
                SlashCommand::Prompt(prompt_cmd)
            }
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

fn parse_fork_args(args: &[String]) -> (Option<String>, Option<usize>) {
    if args.is_empty() {
        return (None, None);
    }

    // Case 1: single numeric argument e.g. "/fork 2"
    if args.len() == 1 {
        if let Ok(turn) = args[0].parse::<usize>() {
            return (None, Some(turn));
        } else {
            return (Some(args[0].clone()), None);
        }
    }

    // Case 2: "/fork at <turn>"
    if args[0].to_lowercase() == "at" && args.len() > 1 {
        if let Ok(turn) = args[1].parse::<usize>() {
            return (None, Some(turn));
        }
    }

    // Case 3: "/fork <title...> [turn]"
    if let Ok(turn) = args.last().unwrap().parse::<usize>() {
        let title_parts = &args[..args.len() - 1];
        let title = title_parts.join(" ");
        if title.is_empty() {
            (None, Some(turn))
        } else {
            (Some(title), Some(turn))
        }
    } else {
        (Some(args.join(" ")), None)
    }
}

fn parse_rewind_args(args: &[String]) -> Option<usize> {
    args.first().and_then(|s| s.parse::<usize>().ok())
}

fn parse_export_args(args: &[String]) -> (Option<ExportFormat>, Option<String>) {
    if args.is_empty() {
        return (Some(ExportFormat::Markdown), None);
    }

    let first = args[0].to_lowercase();
    match first.as_str() {
        "md" | "markdown" => {
            let path = args.get(1).cloned();
            (Some(ExportFormat::Markdown), path)
        }
        "html" | "htm" => {
            let path = args.get(1).cloned();
            (Some(ExportFormat::Html), path)
        }
        other => {
            // Check file extension on the first argument
            if other.ends_with(".html") || other.ends_with(".htm") {
                (Some(ExportFormat::Html), Some(args[0].clone()))
            } else if other.ends_with(".md") || other.ends_with(".markdown") {
                (Some(ExportFormat::Markdown), Some(args[0].clone()))
            } else {
                // If format keyword wasn't recognized, treat it as a path defaulting to Markdown
                (Some(ExportFormat::Markdown), Some(args[0].clone()))
            }
        }
    }
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
        "search" | "find" | "query" | "grep" => {
            let query = args[1..].join(" ");
            SessionCommand::Search { query }
        }
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
        "preset" | "presets" | "pre" => {
            let name = args.get(1).cloned();
            ConfigCommand::Preset { name }
        }
        _ => ConfigCommand::Show,
    }
}

fn parse_skills_subcommand(args: &[String]) -> SkillsCommand {
    if args.is_empty() {
        return SkillsCommand::List;
    }

    match args[0].to_lowercase().as_str() {
        "list" | "ls" => SkillsCommand::List,
        "info" | "show" | "view" => {
            let name = args.get(1).cloned().unwrap_or_default();
            SkillsCommand::Info { name }
        }
        "reload" | "refresh" | "rescan" => SkillsCommand::Reload,
        "enable" | "on" => {
            let name = args.get(1).cloned().unwrap_or_default();
            SkillsCommand::Enable { name }
        }
        "disable" | "off" => {
            let name = args.get(1).cloned().unwrap_or_default();
            SkillsCommand::Disable { name }
        }
        "test" | "eval" | "match" => {
            let query = args[1..].join(" ");
            SkillsCommand::Test { query }
        }
        _ => SkillsCommand::List,
    }
}

fn parse_prompt_subcommand(args: &[String]) -> PromptCommand {
    if args.is_empty() {
        return PromptCommand::List { filter: None };
    }

    match args[0].to_lowercase().as_str() {
        "list" | "ls" => {
            let filter = args.get(1).cloned();
            PromptCommand::List { filter }
        }
        "save" | "add" | "new" | "create" => {
            let name = args.get(1).cloned().unwrap_or_default();
            let mut local = false;
            let mut body_parts = Vec::new();
            for arg in &args[2..] {
                if arg == "--local" || arg == "-l" {
                    local = true;
                } else if arg == "--global" || arg == "-g" {
                    local = false;
                } else {
                    body_parts.push(arg.clone());
                }
            }
            let template = if body_parts.is_empty() {
                None
            } else {
                Some(body_parts.join(" "))
            };
            PromptCommand::Save {
                name,
                template,
                local,
            }
        }
        "load" | "use" | "run" | "apply" => {
            let name = args.get(1).cloned().unwrap_or_default();
            let prompt_args = if args.len() > 2 {
                args[2..].to_vec()
            } else {
                Vec::new()
            };
            PromptCommand::Load {
                name,
                args: prompt_args,
            }
        }
        "show" | "view" | "info" | "cat" | "inspect" => {
            let name = args.get(1).cloned().unwrap_or_default();
            PromptCommand::Show { name }
        }
        "delete" | "del" | "rm" | "remove" => {
            let name = args.get(1).cloned().unwrap_or_default();
            PromptCommand::Delete { name }
        }
        "search" | "find" | "grep" => {
            let query = args[1..].join(" ");
            PromptCommand::Search { query }
        }
        "export" | "exp" => {
            let path = args.get(1).cloned();
            PromptCommand::Export { path }
        }
        "import" | "imp" => {
            let path = args.get(1).cloned().unwrap_or_default();
            PromptCommand::Import { path }
        }
        other => {
            if args.len() > 1 {
                PromptCommand::Load {
                    name: other.to_string(),
                    args: args[1..].to_vec(),
                }
            } else {
                PromptCommand::List {
                    filter: Some(other.to_string()),
                }
            }
        }
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
        SlashCommand::Palette { filter } => {
            handle_palette(filter.as_deref());
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
        SlashCommand::Login => {
            handle_login(runner);
            CommandResult::Continue
        }
        SlashCommand::Advisors { state } => {
            handle_advisors(state.as_deref(), runner);
            CommandResult::Continue
        }
        SlashCommand::Session(subcmd) => handle_session(subcmd, runner, session),
        SlashCommand::Fork { title, turn } => {
            handle_fork(title.as_deref(), *turn, session)
        }
        SlashCommand::Rewind { turns } => {
            handle_rewind(*turns, runner, session);
            CommandResult::Continue
        }
        SlashCommand::Compact => {
            handle_compact(session);
            CommandResult::Continue
        }
        SlashCommand::Stats => {
            handle_stats(runner, session);
            CommandResult::Continue
        }
        SlashCommand::Export { format, path } => {
            handle_export(*format, path.as_deref(), session);
            CommandResult::Continue
        }
        SlashCommand::Trace { path } => {
            crate::agent::trace::handle_trace_command(path.as_deref(), runner, session);
            CommandResult::Continue
        }
        SlashCommand::Clear => handle_clear(session),
        SlashCommand::Quit => handle_quit(),
        SlashCommand::Status => {
            handle_status(runner, session);
            CommandResult::Continue
        }
        SlashCommand::Config(subcmd) => {
            handle_config(subcmd, runner, session);
            CommandResult::Continue
        }
        SlashCommand::Preset { name } => {
            handle_preset(name.as_deref(), runner, session);
            CommandResult::Continue
        }
        SlashCommand::Tools => {
            handle_tools(runner);
            CommandResult::Continue
        }
        SlashCommand::Skills(subcmd) => {
            handle_skills(subcmd, runner);
            CommandResult::Continue
        }
        SlashCommand::File { query } => {
            handle_file(query.as_deref());
            CommandResult::Continue
        }
        SlashCommand::Bookmark { args } => {
            let output = crate::agent::bookmark::handle_bookmark_command(args, session);
            println!("{}", output);
            CommandResult::Continue
        }
        SlashCommand::Prompt(subcmd) => {
            handle_prompt(subcmd, runner, session);
            CommandResult::Continue
        }
        SlashCommand::Snippet { args } => {
            let output = crate::agent::snippets::handle_snippet_command(args, session);
            println!("{}", output);
            CommandResult::Continue
        }
        SlashCommand::Benchmark { args } => {
            crate::ui::bench_cmd::handle_benchmark_command(args, runner, session);
            CommandResult::Continue
        }
        SlashCommand::Recover { args } => {
            let args_str = args.join(" ");
            let output = crate::agent::recovery::handle_recovery_command(&args_str, &runner.tool_ctx().cwd, session);
            println!("{}", output);
            CommandResult::Continue
        }
        SlashCommand::Tag { args } => {
            let output = crate::agent::tagging::handle_tag_command(args, session);
            println!("{}", output);
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

fn handle_file(query: Option<&str>) {
    let mut picker = crate::ui::file_picker::FilePicker::new();
    if let Some(q) = query {
        picker.set_query(q);
    }
    match picker.run_interactive(None) {
        Ok(Some(entry)) => {
            println!(
                "\x1b[1;32m✓\x1b[0m Selected \x1b[1;37m{}\x1b[0m \x1b[2;37m({})\x1b[0m\n",
                entry.relative_path,
                entry.formatted_size()
            );
        }
        Ok(None) => {}
        Err(e) => {
            eprintln!("\x1b[1;31mError running file picker:\x1b[0m {e}\n");
        }
    }
}

fn handle_help(command: Option<&str>) {
    if let Some(topic) = command {
        match topic.to_lowercase().as_str() {
            "fork" | "branch" => {
                let text = r#"
# Slash Command: `/fork [title] [turn]`

Branch the current conversational session into an independent session with parent lineage.

### Usage
- `/fork` - Fork active session at the latest turn with default title `"<title> (fork)"`.
- `/fork <title>` - Fork active session with custom title.
- `/fork <turn_index>` - Fork at a specific historical turn (e.g. `/fork 2`).
- `/fork <title> <turn_index>` - Fork at a historical turn with a custom branch name.

### Examples
- `/fork`
- `/fork "experiment-refactor"`
- `/fork 2`
- `/fork "parser-rewrite" 3`
"#;
                print_markdown(text);
            }
            "rewind" | "undo" | "rw" => {
                let text = r#"
# Slash Command: `/rewind [N]`

Undo and revert the last N conversational turns in the active session.

A turn consists of your user prompt and any subsequent assistant replies or tool calls.
Prelude system instructions are preserved. Changes are immediately saved to disk.

### Usage
- `/rewind` or `/undo` - Undo the most recent user turn (N=1).
- `/rewind <N>` - Undo the last N user turns.

### Examples
- `/rewind`
- `/rewind 2`
- `/undo`
"#;
                print_markdown(text);
            }
            "compact" | "compress" => {
                let text = r#"
# Slash Command: `/compact`

Force context compaction on the active conversation history.

Compaction prunes verbose historical tool outputs, summarizes older turns into an
informative context recap, and preserves the latest turns intact. This dramatically
reduces token usage and latency without losing essential conversational continuity.

### Usage
- `/compact` - Perform in-place context window compaction.
"#;
                print_markdown(text);
            }
            "stats" | "usage" | "cost" => {
                let text = r#"
# Slash Command: `/stats`

Display detailed token analytics, session duration, and itemized USD cost estimates.

### Metrics Included
- **Tokens**: Prompt tokens, completion tokens, total tokens, cache reads/writes.
- **Cache Hit Rate**: Percentage of prompt tokens read from provider context cache.
- **Duration**: Active elapsed session time since creation.
- **Cost Analytics**: Estimated input cost, output cost, cache read savings, and total USD.

### Usage
- `/stats` - View session usage and cost breakdown.
- Aliases: `/usage`, `/cost`.
"#;
                print_markdown(text);
            }
            "benchmark" | "bench" | "latency" | "speed" => {
                let text = r#"
# Slash Command: `/benchmark [provider] [options]`

Benchmark configured LLM providers, measuring Time to First Token (TTFT), generation throughput (tokens/sec), and total latency.

### Usage
- `/benchmark` - Benchmark all configured providers.
- `/benchmark active` or `/benchmark current` - Benchmark only the active provider and model.
- `/benchmark <provider>` - Benchmark a specific provider (e.g. `/benchmark deepseek`).
- `/benchmark <provider> <model>` - Benchmark a specific provider and model.
- `/benchmark compare <p1> <p2> ...` - Compare multiple specific providers.

### Flags & Options
- `-n <rounds>` / `--rounds <N>` - Number of measurement rounds to average (default: 1).
- `-p <prompt>` / `--prompt <text>` - Custom prompt to test with.
- `--ping` / `--ping-only` - Minimal prompt test for fast TTFT round-trip.
- `--parallel` / `--concurrent` - Benchmark providers concurrently.
- `--max-tokens <N>` - Limit maximum completion tokens (default: 96).
- `--timeout <secs>` - Per-provider request timeout (default: 20s).
- `--json` - Output machine-readable JSON results.
- `--markdown` / `--md` - Output formatted Markdown table.
- `--quiet` / `-q` - Suppress interactive progress spinners.

### Examples
- `/benchmark`
- `/benchmark deepseek`
- `/benchmark compare anthropic openai deepseek`
- `/benchmark -n 3 --parallel`
- `/benchmark --ping --json`
"#;
                print_markdown(text);
            }
            "export" | "exp" => {
                let text = r#"
# Slash Command: `/export [md|html] [path]`

Export the active conversation transcript to formatted GitHub-flavored Markdown or standalone HTML.

### Formats Supported
- `md` / `markdown` - Formatted Markdown with code blocks, timestamps, and tool records.
- `html` - Standalone, responsive HTML document with syntax styling and dark mode.

### Usage
- `/export` - Export to Markdown in `~/.fusion/exports/<title>.md`.
- `/export html` - Export to HTML in `~/.fusion/exports/<title>.html`.
- `/export md <path>` - Export Markdown to custom file path.
- `/export html <path>` - Export HTML to custom file path.

### Examples
- `/export`
- `/export html`
- `/export html ~/Desktop/session.html`
- `/export md ./notes/review.md`
"#;
                print_markdown(text);
            }
            "trace" | "tr" => {
                let text = r#"
# Slash Command: `/trace [path]`

Generate a sanitized, privacy-respecting diagnostic trace of tool executions and system events.

Sensitive credentials (API keys, auth tokens, passwords, private keys, emails) are
automatically redacted to ensure safe sharing for debugging and bug reports.

### Usage
- `/trace` - Export trace Markdown to `~/.fusion/traces/trace-<id>.md`.
- `/trace <path>` - Save diagnostic trace to a specific file.
"#;
                print_markdown(text);
            }
            "palette" | "commands" | "pal" => {
                print_command_palette(None);
            }
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
            "bookmark" | "bm" | "mark" => {
                let text = r#"
# Slash Command: `/bookmark <subcommand>`

Pin important conversation turns and manage restorable checkpoints with snapshots.

### Subcommands
- `/bookmark <name> [note]` - Pin current turn with a name and optional note.
- `/bookmark list` - List all bookmarks in this session.
- `/bookmark recall <name>` - View bookmark details, turn preview, and drift.
- `/bookmark checkpoint <name>` - Save a full restorable state snapshot checkpoint.
- `/bookmark restore <name>` - Rewind / restore session back to this bookmark.
- `/bookmark fork <name> [title]` - Fork session at bookmark into an independent branch.
- `/bookmark pin [turn]` - Pin turn to protect it from compaction.
- `/bookmark turn <turn> <name>` - Bookmark a specific historical turn.
- `/bookmark tag <name> <tag>` - Add category tag to bookmark.
- `/bookmark delete <name>` - Delete bookmark from session.
- `/bookmark clear` - Clear all bookmarks from session.
- `/bookmark export [path]` - Export bookmarks to Markdown file.

### Examples
- `/bookmark architecture-decision`
- `/bookmark list`
- `/bookmark recall architecture-decision`
- `/bookmark checkpoint pre-refactor`
- `/bookmark restore pre-refactor`
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
            "snippet" | "snip" | "sn" => {
                let text = r#"
# Slash Command: `/snippet <subcommand>`

Manage and recall reusable code snippets persisted in `~/.fusion/snippets/`.

### Subcommands
- `/snippet save <name> [content...]` - Save code snippet (auto-extracts from last turn if content omitted).
- `/snippet insert <name>` - Recall and inject code snippet into active conversation.
- `/snippet recall <name>` or `/snippet show <name>` - Display snippet details and code.
- `/snippet list [filter]` - List all saved snippets.
- `/snippet search <query>` - Search snippets by name, tags, description, or content.
- `/snippet delete <name>` - Delete a snippet from storage.
- `/snippet export [path]` - Export all snippets to a JSON file.
- `/snippet import <path>` - Import snippets from a JSON file.
- `/snippet clear` - Delete all snippets.
- `/snippet help` - Show help guide.

### Examples
- `/snippet save auth-handler fn authenticate(token: &str) -> bool { ... }`
- `/snippet save tokio-main` (extracts recent code block from conversation)
- `/snippet insert auth-handler`
- `/snippet show auth-handler`
- `/snippet list`
- `/snippet delete auth-handler`
"#;
                print_markdown(text);
            }
            "prompt" | "prompts" | "tmpl" | "template" => {
                let text = r#"
# Slash Command: `/prompt [subcommand]`

Save, load, execute, and organize custom prompt templates with dynamic variable interpolation.

### Subcommands
- `/prompt` or `/prompt list [filter]` - List all prompt templates.
- `/prompt save <name> [template...] [--local]` - Save prompt template to global `~/.fusion/prompts/` (or `.fusion/prompts/` with `--local`). If no template text is given, saves the last user prompt in current session.
- `/prompt load <name> [args...]` - Load and render prompt template with given variable arguments (`key=value` or positional) and insert into conversation.
- `/prompt show <name>` - View full metadata, description, tags, variables, and raw template body.
- `/prompt delete <name>` - Delete a template from disk and library.
- `/prompt search <query>` - Full-text search across template names, tags, and bodies.
- `/prompt export [path]` - Export prompt collection as JSON.
- `/prompt import <path>` - Import prompt collection from JSON file.

### Examples
- `/prompt list`
- `/prompt save my-review` (saves last user message)
- `/prompt save test-gen "Generate unit tests for {{code}}"`
- `/prompt load review code="fn sum(a: i32, b: i32) -> i32 { a + b }"`
- `/prompt load explain`
- `/prompt show refactor`
- `/prompt search security`
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
                println!(
                    "\x1b[1;33mℹ\x1b[0m No specific help topic found for \x1b[1;37m{}\x1b[0m. Showing command palette:\n",
                    other
                );
                print_command_palette(None);
            }
        }
    } else {
        print_command_palette(None);
    }
}

fn handle_palette(filter: Option<&str>) {
    print_command_palette(filter);
}

/// Render and print the categorized interactive command palette.
pub fn print_command_palette(filter: Option<&str>) {
    let output = render_command_palette(filter);
    print!("{}", output);
    let _ = stdout().flush();
}

/// Renders the categorized command palette into an ANSI-formatted string.
pub fn render_command_palette(filter: Option<&str>) -> String {
    let mut out = String::new();
    let filter_clean = filter.map(|f| f.trim().to_lowercase());

    out.push_str("\x1b[1;36m╭────────────────────────────────────────────────────────────────────────────────╮\x1b[0m\n");
    out.push_str("\x1b[1;36m│\x1b[0m \x1b[1;37m✦ Fusion Command Palette\x1b[0m                                                       \x1b[1;36m│\x1b[0m\n");
    out.push_str("\x1b[1;36m│\x1b[0m \x1b[2;37mType \x1b[1;36m/help <cmd>\x1b[2;37m for full details, or \x1b[1;36m/palette <query>\x1b[2;37m to search commands\x1b[0m       \x1b[1;36m│\x1b[0m\n");
    out.push_str("\x1b[1;36m╰────────────────────────────────────────────────────────────────────────────────╯\x1b[0m\n");

    let categories = [
        CommandCategory::Core,
        CommandCategory::Session,
        CommandCategory::Model,
        CommandCategory::Config,
    ];

    let mut total_rendered = 0;

    for cat in categories {
        let matching: Vec<&CommandDescriptor> = COMMAND_PALETTE
            .iter()
            .filter(|cmd| cmd.category == cat)
            .filter(|cmd| {
                if let Some(f) = &filter_clean {
                    cmd.name.to_lowercase().contains(f)
                        || cmd.description.to_lowercase().contains(f)
                        || cmd.syntax.to_lowercase().contains(f)
                        || cmd.aliases.iter().any(|a| a.to_lowercase().contains(f))
                } else {
                    true
                }
            })
            .collect();

        if matching.is_empty() {
            continue;
        }

        total_rendered += matching.len();
        out.push_str(&format!("\n  \x1b[1;33m{}\x1b[0m\n", cat.display_name()));

        for cmd in matching {
            let aliases_str = if cmd.aliases.is_empty() {
                String::new()
            } else {
                format!(" \x1b[2;37m({})\x1b[0m", cmd.aliases.join(", "))
            };

            out.push_str(&format!(
                "    \x1b[1;36m{:<10}\x1b[0m \x1b[1;37m{:<28}\x1b[0m \x1b[2;37m{}\x1b[0m{}\n",
                cmd.name, cmd.syntax, cmd.description, aliases_str
            ));
        }
    }

    if total_rendered == 0 {
        if let Some(f) = filter {
            out.push_str(&format!(
                "\n  \x1b[1;33mℹ\x1b[0m No commands matched filter query: \x1b[1;37m\"{}\"\x1b[0m\n",
                f
            ));
        }
    }

    out.push_str("\n\x1b[2;37mShortcuts: \x1b[1;37mEnter\x1b[2;37m submit │ \x1b[1;37mAlt+Enter\x1b[2;37m multiline │ \x1b[1;37mUp/Down\x1b[2;37m history │ \x1b[1;37mCtrl+C\x1b[2;37m cancel │ \x1b[1;37mCtrl+D\x1b[2;37m exit\x1b[0m\n\n");

    out
}

fn handle_fork(title: Option<&str>, turn: Option<usize>, session: &mut Session) -> CommandResult {
    let orig_id = session.id();
    let orig_turns = crate::agent::fork::count_turns(session);
    let orig_title = session.title().unwrap_or("Session").to_string();

    if orig_turns == 0 && session.total_messages() == 0 {
        println!("\x1b[1;33m⚠\x1b[0m Current session has no conversation turns yet. Nothing to fork.\n");
        return CommandResult::Continue;
    }

    // Auto-save original session before forking if it contains messages
    let _ = session.save();

    let mut forked = crate::agent::fork::fork_session_in_memory(session, turn);

    // Set custom or derived title
    let new_title = match title {
        Some(t) if !t.trim().is_empty() => t.trim().to_string(),
        _ => match turn {
            Some(t) => format!("{} (fork @ turn {})", orig_title, t),
            None => format!("{} (fork)", orig_title),
        },
    };
    forked.set_title(&new_title);

    // Record lineage metadata
    forked.set_metadata("forked_from", orig_id.to_string());
    forked.set_metadata("fork_timestamp", chrono::Utc::now().to_rfc3339());
    if let Some(t) = turn {
        forked.set_metadata("fork_turn_index", t.to_string());
    }

    let forked_id = forked.id();
    let forked_path = match forked.save() {
        Ok(p) => p.display().to_string(),
        Err(_) => format!("{}.json", forked_id),
    };

    let new_turns = crate::agent::fork::count_turns(&forked);
    let new_msgs = forked.total_messages();

    *session = forked.clone();

    println!("\x1b[1;32m✦ Session Forked Successfully!\x1b[0m");
    println!("  \x1b[1;36mBranch ID:\x1b[0m        \x1b[1;37m{}\x1b[0m", forked_id);
    println!("  \x1b[1;36mBranch Title:\x1b[0m     \x1b[1;33m{}\x1b[0m", new_title);
    println!(
        "  \x1b[1;36mParent Session:\x1b[0m   \x1b[2;37m{} (\"{}\")\x1b[0m",
        orig_id, orig_title
    );
    if let Some(t) = turn {
        println!("  \x1b[1;36mForked At:\x1b[0m        Turn {} of {}", t, orig_turns);
    }
    println!(
        "  \x1b[1;36mHistory Branched:\x1b[0m {} turns ({} messages)",
        new_turns, new_msgs
    );
    println!("  \x1b[1;36mActive Model:\x1b[0m     \x1b[1;37m{}\x1b[0m", session.active_model());
    println!("  \x1b[1;36mSaved To:\x1b[0m         \x1b[2;37m{}\x1b[0m", forked_path);
    println!();

    CommandResult::SessionSwitched(forked)
}

fn handle_rewind(turns: Option<usize>, runner: &AgentRunner, session: &mut Session) {
    let count = turns.unwrap_or(1);
    if count == 0 {
        println!(
            "\x1b[1;33m⚠\x1b[0m Specify at least 1 turn to rewind/undo (e.g. \x1b[1;36m/undo 1\x1b[0m).\n"
        );
        return;
    }

    // Revert file mutation checkpoints if any were recorded
    if let Ok(mut mgr) = runner.checkpoints().lock() {
        if mgr.can_undo() {
            match mgr.undo_n(count, &runner.tool_ctx().cwd) {
                Ok(results) => {
                    for res in &results {
                        print!("{}", crate::agent::undo::format_undo_report(res));
                    }
                }
                Err(e) => {
                    eprintln!("\x1b[1;31m⚠ Checkpoint undo error:\x1b[0m {}", e);
                }
            }
        }
    }
    let before_turns = crate::agent::fork::count_turns(session);
    let before_messages = session.total_messages();

    if before_turns == 0 && before_messages == 0 {
        println!("\x1b[1;33m⚠\x1b[0m Session is empty. No turns to rewind.\n");
        return;
    }

    let reverted = crate::agent::fork::rewind_session_in_place(session, count);
    let _ = session.save();

    let after_turns = crate::agent::fork::count_turns(session);
    let after_messages = session.total_messages();
    let removed_messages = before_messages.saturating_sub(after_messages);

    if reverted > 0 {
        println!(
            "\x1b[1;32m✓\x1b[0m Rewound \x1b[1;37m{}\x1b[0m turn{} (removed {} messages).",
            reverted,
            if reverted == 1 { "" } else { "s" },
            removed_messages
        );
        println!(
            "  Session now has \x1b[1;36m{}\x1b[0m turn{} remaining ({} total messages).\n",
            after_turns,
            if after_turns == 1 { "" } else { "s" },
            after_messages
        );
    } else {
        println!("\x1b[1;33mℹ\x1b[0m No conversational turns were available to rewind.\n");
    }
}

fn handle_compact(session: &mut Session) {
    let before_msgs = session.total_messages();
    if before_msgs <= 2 {
        println!(
            "\x1b[1;33mℹ\x1b[0m History is already compact ({} message{}). Compaction requires at least 3 messages.\n",
            before_msgs,
            if before_msgs == 1 { "" } else { "s" }
        );
        return;
    }

    let before_tokens = crate::agent::compaction::estimate_text_tokens(
        &session
            .messages()
            .iter()
            .map(|m| m.content.as_str())
            .collect::<Vec<_>>()
            .join(" "),
    );

    let result = crate::agent::compaction::compact_session(session, None);
    let _ = session.save();

    if result.compacted {
        let reduction_pct = if result.original_tokens > 0 {
            let saved = result.original_tokens.saturating_sub(result.compacted_tokens);
            (saved as f64 / result.original_tokens as f64) * 100.0
        } else {
            0.0
        };

        println!("\x1b[1;32m✦ Context Compaction Complete!\x1b[0m");
        println!(
            "  \x1b[1;36mMessages:\x1b[0m  {} ➔ \x1b[1;32m{}\x1b[0m (pruned {} message{})",
            result.original_messages,
            result.compacted_messages,
            result.original_messages.saturating_sub(result.compacted_messages),
            if result.original_messages.saturating_sub(result.compacted_messages) == 1 {
                ""
            } else {
                "s"
            }
        );
        println!(
            "  \x1b[1;36mTokens:\x1b[0m    ~{} ➔ \x1b[1;32m~{}\x1b[0m (\x1b[1;32m-{:.1}%\x1b[0m context reduction)",
            result.original_tokens, result.compacted_tokens, reduction_pct
        );
        if let Some(summary) = &result.summary {
            println!("  \x1b[1;36mSummary:\x1b[0m   \x1b[2;37m\"{}\"\x1b[0m", summary);
        }
        println!("  \x1b[1;36mStatus:\x1b[0m    Session history compacted and saved.\n");
    } else {
        println!(
            "\x1b[1;33mℹ\x1b[0m Context is already compact (~{} tokens across {} messages). No compaction needed.\n",
            result.original_tokens.max(before_tokens),
            before_msgs
        );
    }
}

fn handle_stats(runner: &AgentRunner, session: &Session) {
    let stats = session.token_stats();
    let provider = &runner.config().default_provider;
    let cost = crate::agent::cost::estimate_session_cost(session, Some(provider));
    let pricing = crate::agent::cost::get_model_pricing(provider, session.active_model());

    // Compute duration
    let duration_str = match chrono::DateTime::parse_from_rfc3339(session.created_at()) {
        Ok(created) => {
            let elapsed =
                chrono::Utc::now().signed_duration_since(created.with_timezone(&chrono::Utc));
            let secs = elapsed.num_seconds().max(0);
            if secs < 60 {
                format!("{}s", secs)
            } else if secs < 3600 {
                format!("{}m {}s", secs / 60, secs % 60)
            } else {
                format!("{}h {}m", secs / 3600, (secs % 3600) / 60)
            }
        }
        Err(_) => "unknown".to_string(),
    };

    let turns_count = crate::agent::fork::count_turns(session);
    let cache_hit_pct = if stats.prompt_tokens > 0 && stats.cache_read_tokens > 0 {
        format!(
            "{:.1}%",
            (stats.cache_read_tokens as f64
                / (stats.prompt_tokens + stats.cache_read_tokens) as f64)
                * 100.0
        )
    } else {
        "0.0%".to_string()
    };

    println!("\x1b[1;36m╭─────────────────────────────────────────────────────────────╮\x1b[0m");
    println!(
        "\x1b[1;36m│\x1b[0m \x1b[1;37m✦ Fusion Session Analytics & Cost Breakdown\x1b[0m                  \x1b[1;36m│\x1b[0m"
    );
    println!("\x1b[1;36m├─────────────────────────────────────────────────────────────┤\x1b[0m");
    println!("  \x1b[1;34mSession ID:\x1b[0m       \x1b[1;37m{}\x1b[0m", session.id());
    if let Some(title) = session.title() {
        println!("  \x1b[1;34mTitle:\x1b[0m            \x1b[1;33m{}\x1b[0m", title);
    }
    println!(
        "  \x1b[1;34mActive Model:\x1b[0m     \x1b[1;37m{}\x1b[0m (Provider: \x1b[1;33m{}\x1b[0m)",
        session.active_model(),
        provider
    );
    println!(
        "  \x1b[1;34mSession Duration:\x1b[0m \x1b[1;32m{}\x1b[0m (created: \x1b[2;37m{}\x1b[0m)",
        duration_str,
        session.created_at()
    );
    println!(
        "  \x1b[1;34mConversation:\x1b[0m     \x1b[1;37m{}\x1b[0m turns (\x1b[1;37m{}\x1b[0m messages)",
        turns_count,
        session.total_messages()
    );
    println!("\x1b[1;36m├─────────────────────────────────────────────────────────────┤\x1b[0m");
    println!("  \x1b[1;35mToken Consumption:\x1b[0m");
    println!(
        "    • Prompt Tokens:      \x1b[1;37m{:>10}\x1b[0m",
        format_number(stats.prompt_tokens)
    );
    println!(
        "    • Completion Tokens:  \x1b[1;37m{:>10}\x1b[0m",
        format_number(stats.completion_tokens)
    );
    println!(
        "    • Cache Read Tokens:  \x1b[1;32m{:>10}\x1b[0m  (Cache hit: \x1b[1;32m{}\x1b[0m)",
        format_number(stats.cache_read_tokens),
        cache_hit_pct
    );
    println!(
        "    • Cache Write Tokens: \x1b[1;37m{:>10}\x1b[0m",
        format_number(stats.cache_write_tokens)
    );
    println!("    ──────────────────────────────────");
    println!(
        "    • Total Tokens:       \x1b[1;36m{:>10}\x1b[0m",
        format_number(stats.total_tokens)
    );
    println!("\x1b[1;36m├─────────────────────────────────────────────────────────────┤\x1b[0m");
    println!("  \x1b[1;33mEstimated Cost Breakdown:\x1b[0m (pricing per 1M tokens)");
    println!(
        "    • Rates:              in: ${:.2}/M, out: ${:.2}/M",
        pricing.input_per_million, pricing.output_per_million
    );
    println!(
        "    • Input Cost:         \x1b[1;37m{}\x1b[0m",
        crate::agent::cost::format_usd(cost.input_cost)
    );
    println!(
        "    • Output Cost:        \x1b[1;37m{}\x1b[0m",
        crate::agent::cost::format_usd(cost.output_cost)
    );
    if cost.cache_read_cost > 0.0 {
        println!(
            "    • Cache Read Cost:    \x1b[1;37m{}\x1b[0m",
            crate::agent::cost::format_usd(cost.cache_read_cost)
        );
    }
    if cost.cache_savings > 0.0 {
        println!(
            "    • Cache Savings:     \x1b[1;32m-{}\x1b[0m",
            crate::agent::cost::format_usd(cost.cache_savings)
        );
    }
    println!(
        "    • Total Session Cost: \x1b[1;32m{}\x1b[0m",
        cost.format_usd()
    );
    println!("\x1b[1;36m╰─────────────────────────────────────────────────────────────╯\x1b[0m\n");
}

fn format_number(n: u64) -> String {
    let s = n.to_string();
    let mut result = String::new();
    let len = s.len();
    for (idx, c) in s.chars().enumerate() {
        if idx > 0 && (len - idx) % 3 == 0 {
            result.push(',');
        }
        result.push(c);
    }
    result
}

fn handle_export(format: Option<ExportFormat>, custom_path: Option<&str>, session: &Session) {
    let fmt = format.unwrap_or(ExportFormat::Markdown);
    let export_dir = Config::config_dir().join("exports");
    if let Err(e) = std::fs::create_dir_all(&export_dir) {
        println!(
            "\x1b[1;31m✗\x1b[0m Failed to create export directory {}: {}\n",
            export_dir.display(),
            e
        );
        return;
    }

    let target_path = match custom_path {
        Some(p) if !p.trim().is_empty() => PathBuf::from(p.trim()),
        _ => {
            let stem = session
                .title()
                .map(sanitize_filename)
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| session.id().to_string());
            export_dir.join(format!("{}.{}", stem, fmt.extension()))
        }
    };

    let result = match fmt {
        ExportFormat::Markdown => {
            let md = session.export_markdown();
            std::fs::write(&target_path, md.as_bytes())
        }
        ExportFormat::Html => {
            let html = session.export_html();
            std::fs::write(&target_path, html.as_bytes())
        }
        ExportFormat::Patch => {
            let patch = session.export_patch();
            std::fs::write(&target_path, patch.as_bytes())
        }
    };

    match result {
        Ok(_) => {
            let file_size = std::fs::metadata(&target_path)
                .map(|m| m.len())
                .unwrap_or(0);
            let size_display = if file_size < 1024 {
                format!("{} B", file_size)
            } else {
                format!("{:.1} KB", file_size as f64 / 1024.0)
            };

            println!(
                "\x1b[1;32m✓\x1b[0m Session exported to \x1b[1;37m{}\x1b[0m format!",
                fmt.display_name()
            );
            println!("  \x1b[1;36mPath:\x1b[0m     \x1b[1;33m{}\x1b[0m", target_path.display());
            println!("  \x1b[1;36mSize:\x1b[0m     {}", size_display);
            println!("  \x1b[1;36mMessages:\x1b[0m {}", session.total_messages());
            println!();
        }
        Err(e) => {
            println!(
                "\x1b[1;31m✗\x1b[0m Failed to export session to {}: {}\n",
                target_path.display(),
                e
            );
        }
    }
}

fn sanitize_filename(name: &str) -> String {
    name.chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .take(50)
        .collect()
}

fn handle_model(name: Option<&str>, runner: &mut AgentRunner, session: &mut Session) {
    if let Some(model_name) = name {
        let trimmed = model_name.trim();
        if trimmed.is_empty() {
            print_model_info(runner, session);
            return;
        }

        let (provider, canonical_model) =
            Config::resolve_model(trimmed, Some(&runner.config().default_provider));
        runner.config_mut().default_provider = provider.clone();
        runner.config_mut().default_model = canonical_model.clone();
        session.set_active_model(&canonical_model);
        println!(
            "\x1b[1;32m✓\x1b[0m Switched active model to \x1b[1;37m{}\x1b[0m (provider: \x1b[1;33m{}\x1b[0m)\n",
            canonical_model, provider
        );
    } else {
        print_model_info(runner, session);
    }
}

fn print_model_info(runner: &AgentRunner, session: &Session) {
    let current_model = session.active_model();
    let current_provider = &runner.config().default_provider;

    println!(
        "\x1b[1;36mActive Model:\x1b[0m \x1b[1;37m{}\x1b[0m (Provider: \x1b[1;33m{}\x1b[0m)",
        current_model, current_provider
    );
    println!("\n\x1b[1;34mFusion Gateway Models:\x1b[0m");
    println!("  \x1b[1;33mDeepSeek V4 Flash:\x1b[0m deepseek-ai/DeepSeek-V4-Flash-0731 (shorthands: deepseek, flash, v4, fusion)");
    println!("  \x1b[1;33mMiniMax M2.7:\x1b[0m      MiniMaxAI/MiniMax-M2.7 (shorthands: minimax, minimax-m2.7)");
    println!("  \x1b[1;33mKimi K2.6:\x1b[0m         moonshotai/Kimi-K2.6 (shorthands: kimi, kimi-k2.6)");
    println!("\nUsage: \x1b[1;36m/model <model_name>\x1b[0m to switch.\n");
}

fn handle_provider(name: Option<&str>, runner: &mut AgentRunner) {
    if let Some(provider_name) = name {
        let trimmed = provider_name.trim().to_lowercase();
        if trimmed.is_empty() {
            print_provider_info(runner);
            return;
        }

        if trimmed == "fusion" {
            runner.config_mut().default_provider = "fusion".to_string();
            println!(
                "\x1b[1;32m✓\x1b[0m Active provider is \x1b[1;37mfusion\x1b[0m\n"
            );
        } else {
            println!(
                "\x1b[1;33mNote:\x1b[0m Fusion CLI uses the Fusion Gateway. Supported provider is 'fusion'.\n"
            );
        }
    } else {
        print_provider_info(runner);
    }
}

/// Log in to the Fusion API via the browser flow:
/// 1. POST /v1/api/cli/token/init -> tokenId
/// 2. Open https://fusioncode.app/cli-auth?tokenId=... in the browser
/// 3. Poll GET /v1/api/cli/token/:tokenId/poll until authorized
/// 4. Store the returned API key in config
fn handle_login(runner: &mut AgentRunner) {
    use std::time::{Duration, Instant};

    const INIT_URL: &str = "https://api.fusioncode.app/v1/api/cli/token/init";
    const AUTH_PAGE: &str = "https://fusioncode.app/cli-auth";
    const API_KEY_BASE: &str = "https://api.fusioncode.app";

    // We're already inside the tokio main runtime (REPL), so use
    // block_in_place + a handle to the CURRENT runtime — creating and
    // dropping a second Runtime here panics.
    let handle = match tokio::runtime::Handle::try_current() {
        Ok(h) => h,
        Err(e) => {
            eprintln!("\x1b[1;31mLogin error:\x1b[0m No async runtime: {}", e);
            return;
        }
    };

    tokio::task::block_in_place(|| handle.block_on(async move {
    println!("\x1b[1;36m⟳ Starting Fusion login...\x1b[0m");

    let client = reqwest::Client::new();

    // Cloudflare edge blocks non-browser clients (error 1010) unless a
    // browser User-Agent is sent, so every request carries one.
    fn browser_ua() -> &'static str {
        "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/125.0.0.0 Safari/537.36"
    }

    // 1. Initialize CLI token
    let init: serde_json::Value = match client
        .post(INIT_URL)
        .header(reqwest::header::USER_AGENT, browser_ua())
        .header(reqwest::header::ACCEPT, "application/json")
        .send()
        .await
    {
        Ok(r) if r.status().is_success() => match r.json().await {
            Ok(v) => v,
            Err(e) => {
                eprintln!("\x1b[1;31mLogin error:\x1b[0m Failed to parse init response: {}", e);
                return;
            }
        },
        Ok(r) => {
            eprintln!("\x1b[1;31mLogin error:\x1b[0m HTTP {} from {}", r.status(), INIT_URL);
            return;
        }
        Err(e) => {
            eprintln!("\x1b[1;31mLogin error:\x1b[0m {}", e);
            return;
        }
    };

    let token_id = match init.get("tokenId").and_then(|v| v.as_str()) {
        Some(t) => t.to_string(),
        None => {
            eprintln!("\x1b[1;31mLogin error:\x1b[0m No tokenId in init response: {}", init);
            return;
        }
    };

    // 2. Open the browser for the user to authorize.
    // The web page reads the CLI token from the `token` query param.
    let auth_url = format!("{}?token={}", AUTH_PAGE, token_id);
    let opened = open_browser(&auth_url);
    println!();
    println!("\x1b[1;33m  ┌────────────────────────────────────────────────────────────┐\x1b[0m");
    println!("\x1b[1;33m  │ \x1b[0mLog in to Fusion to authorize this device                \x1b[1;33m│\x1b[0m");
    println!("\x1b[1;33m  │\x1b[0m                                                          \x1b[1;33m│\x1b[0m");
    println!("\x1b[1;33m  │ \x1b[1;37m{}[0m  \x1b[1;33m│\x1b[0m", auth_url);
    println!("\x1b[1;33m  │\x1b[0m                                                          \x1b[1;33m│\x1b[0m");
    println!("\x1b[1;33m  │ \x1b[2;37mWaiting for authorization (10 min timeout)...\x1b[0m      \x1b[1;33m│\x1b[0m");
    println!("\x1b[1;33m  └────────────────────────────────────────────────────────────┘\x1b[0m");
    println!();
    if !opened {
        println!("\x1b[2;37mIf the browser didn't open, copy the URL above manually.\x1b[0m\n");
    }

    // 3. Poll for authorization (up to 10 minutes, matching the token TTL)
    let poll_url = format!("{}/v1/api/cli/token/{}/poll", API_KEY_BASE, token_id);
    let deadline = Instant::now() + Duration::from_secs(600);
    loop {
        if Instant::now() >= deadline {
            eprintln!("\x1b[1;31mLogin timed out after 10 minutes.\x1b[0m");
            return;
        }

        match client
            .get(&poll_url)
            .header(reqwest::header::USER_AGENT, browser_ua())
            .header(reqwest::header::ACCEPT, "application/json")
            .send()
            .await
        {
            Ok(r) if r.status().is_success() => {
                match r.json::<serde_json::Value>().await {
                    Ok(v) => {
                        let status = v.get("status").and_then(|s| s.as_str()).unwrap_or("pending");
                        if status == "authorized" {
                            if let Some(api_key) = v.get("apiKey").and_then(|k| k.as_str()) {
                                // 4. Persist the key
                                runner.config_mut().fusion_api_key = Some(api_key.to_string());
                                if let Err(e) = runner.config().save() {
                                    eprintln!("\x1b[1;31mWarning:\x1b[0m Failed to save config: {}", e);
                                }
                                let email = v
                                    .get("userEmail")
                                    .and_then(|e| e.as_str())
                                    .unwrap_or("your account");
                                println!();
                                println!("\x1b[1;32m✓\x1b[0m Login successful! Authenticated as \x1b[1;37m{}\x1b[0m", email);
                                println!("\x1b[2;37mFusion API key saved to {}\x1b[0m", crate::config::Config::config_path().display());
                                println!();
                                return;
                            }
                            eprintln!("\x1b[1;31mLogin error:\x1b[0m No apiKey in authorized response.");
                            return;
                        }
                        if status == "expired" || status == "invalid" {
                            eprintln!("\x1b[1;31mLogin failed:\x1b[0m Token {} expired or invalid. Run /login again.", status);
                            return;
                        }
                    }
                    Err(e) => {
                        eprintln!("\x1b[1;31mLogin error:\x1b[0m Bad poll response: {}", e);
                        return;
                    }
                }
            }
            Ok(_) => {
                eprintln!("\x1b[1;31mLogin error:\x1b[0m Poll request failed (HTTP error).");
                return;
            }
            Err(e) => {
                eprintln!("\x1b[1;31mLogin error:\x1b[0m {}", e);
                return;
            }
        }

        tokio::time::sleep(Duration::from_secs(2)).await;
    }
    }))
}

/// Open a URL in the system browser (best-effort).
fn open_browser(url: &str) -> bool {
    use std::process::Command as Shell;
    #[cfg(target_os = "macos")]
    let result = Shell::new("open").arg(url).spawn().map(|_| ());
    #[cfg(target_os = "linux")]
    let result = Shell::new("xdg-open").arg(url).spawn().map(|_| ());
    #[cfg(target_os = "windows")]
    let result = Shell::new("cmd").args(["/c", "start", url]).spawn().map(|_| ());
    #[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
    let result = Err(std::io::Error::new(std::io::ErrorKind::Other, "unsupported platform"));
    result.is_ok()
}

fn print_provider_info(runner: &AgentRunner) {
    let current_provider = &runner.config().default_provider;
    println!(
        "\x1b[1;36mActive Provider:\x1b[0m \x1b[1;33m{}\x1b[0m (Fusion Gateway - https://api.fusioncode.app/v1)\n",
        current_provider
    );

    let fusion_key = std::env::var("FUSION_API_KEY").is_ok()
        || runner.config().fusion_api_key.is_some();

    print_provider_status("fusion", "Fusion Gateway", fusion_key, current_provider);
    println!("\nAuthenticate with \x1b[1;36m/login\x1b[0m or set \x1b[1;33mFUSION_API_KEY\x1b[0m.\n");
}

fn print_provider_status(name: &str, label: &str, has_key: bool, current: &str) {
    let is_current = current == name;
    let marker = if is_current {
        "\x1b[1;32m* (active)\x1b[0m"
    } else {
        "          "
    };

    let key_status = if has_key {
        "\x1b[1;32m[key configured]\x1b[0m"
    } else {
        "\x1b[2;37m[no API key found]\x1b[0m"
    };

    println!("  {} \x1b[1;37m{:<12}\x1b[0m {}", marker, label, key_status);
}

fn handle_advisors(state: Option<&str>, runner: &mut AgentRunner) {
    if let Some(action) = state {
        match action.to_lowercase().as_str() {
            "on" | "enable" | "true" | "1" => {
                runner.config_mut().advisors_enabled = true;
                println!("\x1b[1;32m✓\x1b[0m Multi-domain advisors \x1b[1;32mENABLED\x1b[0m (Security, Architecture, Performance, Linux, Mobile, Windows)\n");
            }
            "off" | "disable" | "false" | "0" => {
                runner.config_mut().advisors_enabled = false;
                println!("\x1b[1;33m✓\x1b[0m Multi-domain advisors \x1b[1;31mDISABLED\x1b[0m\n");
            }
            "toggle" | "t" => {
                let current = runner.config().advisors_enabled;
                runner.config_mut().advisors_enabled = !current;
                if !current {
                    println!("\x1b[1;32m✓\x1b[0m Multi-domain advisors \x1b[1;32mENABLED\x1b[0m\n");
                } else {
                    println!("\x1b[1;33m✓\x1b[0m Multi-domain advisors \x1b[1;31mDISABLED\x1b[0m\n");
                }
            }
            "status" | "info" => {
                print_advisor_status(runner);
            }
            other => {
                println!(
                    "\x1b[1;31m✗\x1b[0m Unknown advisors argument: \x1b[1;37m{}\x1b[0m\n",
                    other
                );
                println!("Usage: \x1b[1;36m/advisors <on|off|toggle|status>\x1b[0m\n");
            }
        }
    } else {
        print_advisor_status(runner);
    }
}

fn print_advisor_status(runner: &AgentRunner) {
    let enabled = runner.config().advisors_enabled;
    let status_str = if enabled {
        "\x1b[1;32mENABLED\x1b[0m"
    } else {
        "\x1b[1;31mDISABLED\x1b[0m"
    };

    println!("\x1b[1;36mAdvisor Critique Subsystem:\x1b[0m {}", status_str);
    println!("\n\x1b[1;34mActive Domains:\x1b[0m");
    println!("  • \x1b[1;33mSecurity Advisor\x1b[0m      - Command safety, secrets scanning, privilege limits");
    println!("  • \x1b[1;33mArchitecture Advisor\x1b[0m  - Clean layering, zero-cost modularity, cross-platform");
    println!("  • \x1b[1;33mPerformance Advisor\x1b[0m   - Zero-copy, allocation audits, algorithmic complexity");
    println!("  • \x1b[1;33mLinux Advisor\x1b[0m         - POSIX conformance, system paths, procfs/sysfs nuances");
    println!("  • \x1b[1;33mMobile Advisor\x1b[0m        - Termux constraints, single-process limits, storage");
    println!("  • \x1b[1;33mWindows Advisor\x1b[0m       - UNC paths, PowerShell quirks, CRLF handling");
    println!("\nUsage: \x1b[1;36m/advisors <on|off|toggle>\x1b[0m to change.\n");
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
                        println!(
                            "\x1b[1;33mℹ\x1b[0m No saved sessions found in \x1b[2;37m{}\x1b[0m\n",
                            Session::sessions_dir().display()
                        );
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
                        println!(
                            "\nUsage: \x1b[1;36m/session load <id>\x1b[0m to restore a session.\n"
                        );
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
                        // Auto-save current session first
                        if session.total_messages() > 0 {
                            let _ = session.save();
                        }

                        let title_disp = loaded
                            .title
                            .clone()
                            .unwrap_or_else(|| "Untitled session".to_string());
                        let msg_count = loaded.messages.len();
                        *session = loaded.clone();

                        println!(
                            "\x1b[1;32m✓\x1b[0m Loaded session \x1b[1;37m{}\x1b[0m (\"{}\", {} messages)\n",
                            uuid, title_disp, msg_count
                        );
                        return CommandResult::SessionSwitched(loaded);
                    }
                    Err(e) => {
                        println!(
                            "\x1b[1;31m✗\x1b[0m Failed to load session {}: {}\n",
                            uuid, e
                        );
                    }
                }
            }

            CommandResult::Continue
        }
        SessionCommand::Save => {
            match session.save() {
                Ok(path) => {
                    println!(
                        "\x1b[1;32m✓\x1b[0m Session checkpoint saved to \x1b[2;37m{}\x1b[0m\n",
                        path.display()
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
                    Ok(summaries) => {
                        let matches: Vec<&SessionSummary> = summaries
                            .iter()
                            .filter(|s| s.id.to_string().starts_with(id_or_prefix))
                            .collect();
                        if matches.len() == 1 {
                            Some(matches[0].id)
                        } else {
                            None
                        }
                    }
                    Err(_) => None,
                }
            };

            if let Some(uuid) = target_uuid {
                match Session::delete(uuid) {
                    Ok(_) => {
                        println!("\x1b[1;32m✓\x1b[0m Deleted session {}\n", uuid);
                    }
                    Err(e) => {
                        println!("\x1b[1;31m✗\x1b[0m Failed to delete session: {}\n", e);
                    }
                }
            } else {
                println!(
                    "\x1b[1;31m✗\x1b[0m No unique session found matching: \x1b[1;37m{}\x1b[0m\n",
                    id_or_prefix
                );
            }
            CommandResult::Continue
        }
        SessionCommand::Search { query } => {
            if query.trim().is_empty() {
                println!("Usage: \x1b[1;36m/session search <query_or_operators>\x1b[0m\n");
                println!("Examples:");
                println!("  /session search \"jwt validation\"");
                println!("  /session search bug role:user model:gpt-4o");
                println!("  /session search auth mode:semantic\n");
                return CommandResult::Continue;
            }
            match crate::agent::search::search_sessions(query) {
                Ok(report) => {
                    print!("{}", report.format_terminal(true));
                }
                Err(e) => {
                    println!("\x1b[1;31m✗\x1b[0m Failed to search sessions: {}\n", e);
                }
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
    println!(
        "  Registered Tools: \x1b[1;34m{}\x1b[0m",
        runner.tools().definitions().len()
    );
    println!(
        "  Working Dir:      \x1b[2;37m{}\x1b[0m",
        runner.tool_ctx().cwd.display()
    );
    println!();
}

fn handle_config(subcmd: &ConfigCommand, runner: &mut AgentRunner, session: &mut Session) {
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
        ConfigCommand::Preset { name } => {
            handle_preset(name.as_deref(), runner, session);
        }
    }
}

fn handle_preset(name: Option<&str>, runner: &mut AgentRunner, session: &mut Session) {
    match name {
        None => {
            println!("\x1b[1;36mAvailable Configuration Presets:\x1b[0m");
            println!("{}", crate::config::format_presets_table());
            println!("Usage: \x1b[1;33m/preset <name>\x1b[0m (e.g. \x1b[1;32m/preset coding-fast\x1b[0m)\n");
        }
        Some(preset_name) => {
            match crate::config::ConfigPreset::from_str_loose(preset_name) {
                Some(preset) => {
                    let cfg = runner.config_mut();
                    cfg.apply_preset(preset);
                    session.active_model = cfg.default_model.clone();

                    println!(
                        "\x1b[1;32m✓\x1b[0m Switched to preset \x1b[1;36m{}\x1b[0m ({})",
                        preset.id(),
                        preset.title()
                    );
                    println!("  • \x1b[1;33mProvider:\x1b[0m     {}", preset.provider());
                    println!("  • \x1b[1;33mModel:\x1b[0m        {}", preset.model());
                    println!(
                        "  • \x1b[1;33mMax Tokens:\x1b[0m   {}",
                        preset.max_tokens().map(|t| t.to_string()).unwrap_or_else(|| "-".to_string())
                    );
                    println!(
                        "  • \x1b[1;33mTemperature:\x1b[0m  {}",
                        preset.temperature().map(|t| format!("{:.1}", t)).unwrap_or_else(|| "-".to_string())
                    );
                    println!(
                        "  • \x1b[1;33mAdvisors:\x1b[0m     {}",
                        if preset.advisors_enabled() { "enabled" } else { "disabled" }
                    );
                    println!("  • \x1b[1;33mRecommended:\x1b[0m  {}\n", preset.recommended_for());
                }
                None => {
                    println!(
                        "\x1b[1;31m✗\x1b[0m Unknown preset: \x1b[1;37m{}\x1b[0m\nHint: Available presets are: {}\n",
                        preset_name,
                        crate::config::available_presets_list()
                    );
                }
            }
        }
    }
}

fn handle_skills(subcmd: &SkillsCommand, runner: &mut AgentRunner) {
    match subcmd {
        SkillsCommand::List => {
            let skills = runner.skills().list();
            if skills.is_empty() {
                println!("\x1b[1;33mℹ\x1b[0m No skills registered.");
                println!("  Place \x1b[1;37mSKILL.md\x1b[0m in \x1b[1;36m.fusion/skills/<name>/\x1b[0m or \x1b[1;36m~/.fusion/skills/<name>/\x1b[0m.\n");
            } else {
                println!("\x1b[1;36mRegistered Domain Skills:\x1b[0m ({} total)", skills.len());
                for skill in skills {
                    let status_badge = if skill.is_enabled() {
                        "\x1b[1;32m● Active\x1b[0m"
                    } else {
                        "\x1b[1;30m○ Off\x1b[0m"
                    };
                    let source_label = match &skill.source {
                        crate::agent::skills::SkillSource::Project(_) => "\x1b[1;34m[project]\x1b[0m",
                        crate::agent::skills::SkillSource::Global(_) => "\x1b[1;35m[global]\x1b[0m",
                        crate::agent::skills::SkillSource::Custom(_) => "\x1b[1;33m[custom]\x1b[0m",
                        crate::agent::skills::SkillSource::Builtin => "\x1b[1;32m[builtin]\x1b[0m",
                    };
                    let triggers_desc = if skill.triggers().is_empty() {
                        "".to_string()
                    } else {
                        format!(" \x1b[2;37m(triggers: {})\x1b[0m", skill.triggers().join(", "))
                    };
                    println!(
                        "  {} {} \x1b[1;37m{}\x1b[0m - {}{}",
                        status_badge,
                        source_label,
                        skill.name(),
                        if skill.description().is_empty() { "No description" } else { skill.description() },
                        triggers_desc
                    );
                }
                println!();
            }
        }
        SkillsCommand::Info { name } => {
            if name.is_empty() {
                println!("\x1b[1;31m✗\x1b[0m Please specify a skill name: \x1b[1;36m/skills info <name>\x1b[0m\n");
                return;
            }
            if let Some(skill) = runner.skills().get(name) {
                println!("\x1b[1;36mSkill Details:\x1b[0m \x1b[1;37m{}\x1b[0m", skill.name());
                println!("  \x1b[1;33mStatus:\x1b[0m       {}", if skill.is_enabled() { "Enabled" } else { "Disabled" });
                println!("  \x1b[1;33mSource:\x1b[0m       {}", skill.source);
                if let Some(path) = &skill.path {
                    println!("  \x1b[1;33mPath:\x1b[0m         {}", path.display());
                }
                println!("  \x1b[1;33mDescription:\x1b[0m  {}", if skill.description().is_empty() { "-" } else { skill.description() });
                let triggers_str = skill.triggers().join(", ");
                println!("  \x1b[1;33mTriggers:\x1b[0m     {}", if skill.triggers().is_empty() { "-" } else { &triggers_str });
                let tags_str = skill.tags().join(", ");
                println!("  \x1b[1;33mTags:\x1b[0m         {}", if skill.tags().is_empty() { "-" } else { &tags_str });
                println!("  \x1b[1;33mAlways On:\x1b[0m    {}", skill.is_always_active());
                println!("\n\x1b[1;36mInstructions:\x1b[0m\n{}\n", skill.instructions().trim());
            } else {
                println!("\x1b[1;31m✗\x1b[0m Skill '\x1b[1;37m{}\x1b[0m' not found. Type \x1b[1;36m/skills list\x1b[0m to see available skills.\n", name);
            }
        }
        SkillsCommand::Reload => {
            let working_dir = runner.tool_ctx().cwd.clone();
            let reloaded = crate::agent::skills::SkillRegistry::scan_default(Some(&working_dir));
            let count = reloaded.len();
            *runner.skills_mut() = reloaded;
            println!("\x1b[1;32m✓\x1b[0m Reloaded skills registry: \x1b[1;37m{} skills\x1b[0m loaded from disk.\n", count);
        }
        SkillsCommand::Enable { name } => {
            if name.is_empty() {
                println!("\x1b[1;31m✗\x1b[0m Please specify a skill name: \x1b[1;36m/skills enable <name>\x1b[0m\n");
                return;
            }
            if runner.skills_mut().set_enabled(name, true) {
                println!("\x1b[1;32m✓\x1b[0m Enabled skill '\x1b[1;37m{}\x1b[0m'.\n", name);
            } else {
                println!("\x1b[1;31m✗\x1b[0m Skill '\x1b[1;37m{}\x1b[0m' not found.\n", name);
            }
        }
        SkillsCommand::Disable { name } => {
            if name.is_empty() {
                println!("\x1b[1;31m✗\x1b[0m Please specify a skill name: \x1b[1;36m/skills disable <name>\x1b[0m\n");
                return;
            }
            if runner.skills_mut().set_enabled(name, false) {
                println!("\x1b[1;32m✓\x1b[0m Disabled skill '\x1b[1;37m{}\x1b[0m'.\n", name);
            } else {
                println!("\x1b[1;31m✗\x1b[0m Skill '\x1b[1;37m{}\x1b[0m' not found.\n", name);
            }
        }
        SkillsCommand::Test { query } => {
            if query.is_empty() {
                println!("\x1b[1;31m✗\x1b[0m Please specify a test query: \x1b[1;36m/skills test <query>\x1b[0m\n");
                return;
            }
            let working_dir = runner.tool_ctx().cwd.clone();
            let skill_matches = runner.skills().find_relevant(query, Some(&working_dir));
            println!("\x1b[1;36mRelevance Matches for:\x1b[0m \"\x1b[1;37m{}\x1b[0m\" ({} matches)", query, skill_matches.len());
            if skill_matches.is_empty() {
                println!("  \x1b[2;37mNo skills matched above relevance threshold.\x1b[0m\n");
            } else {
                for (i, m) in skill_matches.iter().enumerate() {
                    println!(
                        "  {}. \x1b[1;33m{:<20}\x1b[0m (score: \x1b[1;32m{:.2}\x1b[0m) - \x1b[2;37m{}\x1b[0m",
                        i + 1,
                        m.skill.name(),
                        m.score,
                        m.reason
                    );
                }
                println!();
            }
        }
    }
}

fn handle_prompt(
    subcmd: &PromptCommand,
    _runner: &mut AgentRunner,
    session: &mut Session,
) {
    let mut lib = crate::agent::prompt_lib::PromptLibrary::new();

    match subcmd {
        PromptCommand::List { filter } => {
            let templates = if let Some(ref f) = filter {
                let trimmed = f.trim();
                if trimmed.is_empty() {
                    lib.list()
                } else {
                    lib.search(trimmed)
                }
            } else {
                lib.list()
            };

            if templates.is_empty() {
                if let Some(ref f) = filter {
                    println!("\n\x1b[1;33mNo prompt templates matched filter:\x1b[0m \x1b[1;37m{}\x1b[0m\n", f);
                } else {
                    println!("\n\x1b[1;33mPrompt library is empty.\x1b[0m\n");
                }
                return;
            }

            println!("\n\x1b[1;36mPrompt Template Library\x1b[0m ({} templates)", templates.len());
            println!("\x1b[2;37mUse \x1b[1;37m/prompt load <name> [args...]\x1b[0m\x1b[2;37m to render a template into the conversation.\x1b[0m\n");

            let mut by_cat: std::collections::BTreeMap<&str, Vec<&crate::agent::prompt_lib::PromptTemplate>> = std::collections::BTreeMap::new();
            for tmpl in &templates {
                let cat = tmpl.category.as_deref().unwrap_or("General");
                by_cat.entry(cat).or_default().push(tmpl);
            }

            for (cat, tmpls) in by_cat {
                println!("  \x1b[1;35m📂 {}\x1b[0m", cat);
                for t in tmpls {
                    let builtin_badge = if t.is_builtin {
                        "\x1b[2;37m[builtin]\x1b[0m"
                    } else {
                        "\x1b[1;32m[custom]\x1b[0m"
                    };

                    let vars_preview = if t.variables.is_empty() {
                        String::new()
                    } else {
                        let names: Vec<String> = t.variables.iter().map(|v| {
                            if v.required {
                                format!("\x1b[1;33m{{{{{}}}}}\x1b[0m", v.name)
                            } else {
                                format!("\x1b[2;37m{{{{{}}}}}\x1b[0m", v.name)
                            }
                        }).collect();
                        format!(" {}", names.join(" "))
                    };

                    println!(
                        "    • \x1b[1;36m{:<16}\x1b[0m {} \x1b[0m{}\x1b[0m{}",
                        t.name,
                        builtin_badge,
                        t.description,
                        vars_preview
                    );
                }
                println!();
            }
        }

        PromptCommand::Save { name, template, local } => {
            let name_trimmed = name.trim();
            if name_trimmed.is_empty() {
                println!("\x1b[1;31mError:\x1b[0m Missing template name. Usage: \x1b[1;36m/prompt save <name> [template...]\x1b[0m");
                return;
            }

            let template_body = if let Some(ref t) = template {
                t.trim().to_string()
            } else {
                session.messages()
                    .iter()
                    .rev()
                    .find(|m| m.role == crate::provider::types::Role::User)
                    .map(|m| m.content.clone())
                    .unwrap_or_default()
            };

            if template_body.is_empty() {
                println!(
                    "\x1b[1;31mError:\x1b[0m No prompt text provided and no user message found in active session.\n\
                     Usage: \x1b[1;36m/prompt save <name> \"Your prompt template with {{{{code}}}}\"\x1b[0m"
                );
                return;
            }

            let mut tmpl = crate::agent::prompt_lib::PromptTemplate::new(
                name_trimmed,
                format!("Custom prompt: {}", name_trimmed),
                &template_body,
            );
            tmpl.category = Some("Custom".to_string());

            let save_result = if *local {
                lib.save_to_local(tmpl.clone())
            } else {
                lib.save_to_global(tmpl.clone())
            };

            match save_result {
                Ok(path) => {
                    println!("\n\x1b[1;32m✓ Saved prompt template:\x1b[0m \x1b[1;36m{}\x1b[0m", tmpl.name);
                    println!("  \x1b[2;37mLocation:\x1b[0m  \x1b[1;37m{}\x1b[0m", path.display());
                    if !tmpl.variables.is_empty() {
                        let var_names: Vec<&str> = tmpl.variables.iter().map(|v| v.name.as_str()).collect();
                        println!("  \x1b[2;37mVariables:\x1b[0m \x1b[1;33m{}\x1b[0m", var_names.join(", "));
                    }
                    println!("  \x1b[2;37mExecute:\x1b[0m   \x1b[1;36m/prompt load {}\x1b[0m\n", tmpl.name);
                }
                Err(e) => {
                    println!("\x1b[1;31mFailed to save prompt template:\x1b[0m {}", e);
                }
            }
        }

        PromptCommand::Load { name, args } => {
            let name_trimmed = name.trim();
            if name_trimmed.is_empty() {
                println!("\x1b[1;31mError:\x1b[0m Missing template name. Usage: \x1b[1;36m/prompt load <name> [args...]\x1b[0m");
                return;
            }

            let tmpl = match lib.get(name_trimmed) {
                Some(t) => t.clone(),
                None => {
                    println!("\x1b[1;31mError:\x1b[0m Template \x1b[1;37m'{}'\x1b[0m not found in library.", name_trimmed);
                    let suggestions = lib.search(name_trimmed);
                    if !suggestions.is_empty() {
                        let names: Vec<&str> = suggestions.iter().take(3).map(|s| s.name.as_str()).collect();
                        println!("  \x1b[2;37mDid you mean:\x1b[0m \x1b[1;36m{}\x1b[0m", names.join(", "));
                    }
                    println!("  \x1b[2;37mType \x1b[1;36m/prompt list\x1b[0m\x1b[2;37m to inspect available templates.\x1b[0m\n");
                    return;
                }
            };

            let rendered_res = if args.is_empty() {
                let has_required = tmpl.variables.iter().any(|v| v.required);
                if has_required {
                    if let Some(user_msg) = session.messages().iter().rev().find(|m| m.role == crate::provider::types::Role::User) {
                        tmpl.render_positional(&[&user_msg.content])
                    } else {
                        tmpl.render_cli_args(args)
                    }
                } else {
                    tmpl.render_cli_args(args)
                }
            } else {
                tmpl.render_cli_args(args)
            };

            match rendered_res {
                Ok(rendered) => {
                    println!("\n\x1b[1;32m✓ Loaded template:\x1b[0m \x1b[1;36m{}\x1b[0m", tmpl.name);
                    println!("\x1b[2;37m--------------------------------------------------\x1b[0m");
                    println!("{}", rendered);
                    println!("\x1b[2;37m--------------------------------------------------\x1b[0m\n");

                    session.add_user_message(&rendered);
                    println!("\x1b[1;32m✓ Injected rendered prompt into active session.\x1b[0m");
                }
                Err(crate::agent::prompt_lib::PromptLibError::MissingVariable { variable, .. }) => {
                    println!("\n\x1b[1;33mTemplate '{}' requires variable:\x1b[0m \x1b[1;31m{{{{{}}}}}\x1b[0m", tmpl.name, variable);
                    println!("\x1b[2;37mUsage:\x1b[0m \x1b[1;36m/prompt load {} {}=\"<value>\"\x1b[0m", tmpl.name, variable);
                    if !tmpl.variables.is_empty() {
                        println!("\n\x1b[1;37mTemplate Variables:\x1b[0m");
                        for v in &tmpl.variables {
                            let req = if v.required { "\x1b[1;31m[required]\x1b[0m" } else { "\x1b[2;37m[optional]\x1b[0m" };
                            let desc = v.description.as_deref().unwrap_or("");
                            println!("  • \x1b[1;33m{:<12}\x1b[0m {} {}", v.name, req, desc);
                        }
                    }
                    println!();
                }
                Err(e) => {
                    println!("\x1b[1;31mFailed to render template:\x1b[0m {}\n", e);
                }
            }
        }

        PromptCommand::Show { name } => {
            let name_trimmed = name.trim();
            match lib.get(name_trimmed) {
                Some(tmpl) => {
                    let card = tmpl.format_markdown_card();
                    print_markdown(&card);
                }
                None => {
                    println!("\x1b[1;31mError:\x1b[0m Template \x1b[1;37m'{}'\x1b[0m not found.\n", name_trimmed);
                }
            }
        }

        PromptCommand::Delete { name } => {
            let name_trimmed = name.trim();
            match lib.delete_persisted(name_trimmed) {
                Ok(true) => {
                    println!("\x1b[1;32m✓ Deleted prompt template:\x1b[0m \x1b[1;36m{}\x1b[0m\n", name_trimmed);
                }
                Ok(false) => {
                    println!("\x1b[1;33mTemplate '{}' not found to delete.\x1b[0m\n", name_trimmed);
                }
                Err(e) => {
                    println!("\x1b[1;31mFailed to delete template:\x1b[0m {}\n", e);
                }
            }
        }

        PromptCommand::Search { query } => {
            let matches = lib.search(query);
            if matches.is_empty() {
                println!("\x1b[1;33mNo templates matched query:\x1b[0m \x1b[1;37m{}\x1b[0m\n", query);
            } else {
                println!("\n\x1b[1;36mSearch Results for:\x1b[0m \x1b[1;37m\"{}\"\x1b[0m ({} matches)\n", query, matches.len());
                for t in matches {
                    let cat = t.category.as_deref().unwrap_or("General");
                    println!("  • \x1b[1;36m{:<16}\x1b[0m \x1b[2;37m[{}]\x1b[0m {}", t.name, cat, t.description);
                }
                println!();
            }
        }

        PromptCommand::Export { path } => {
            let target_path = path.as_deref().map(PathBuf::from).unwrap_or_else(|| {
                Config::config_dir().join("exports").join("prompts.json")
            });

            if let Some(parent) = target_path.parent() {
                let _ = std::fs::create_dir_all(parent);
            }

            match lib.export_all_json() {
                Ok(json) => {
                    if let Err(e) = std::fs::write(&target_path, json) {
                        println!("\x1b[1;31mFailed to write export file:\x1b[0m {}", e);
                    } else {
                        println!("\x1b[1;32m✓ Exported {} prompt templates to:\x1b[0m \x1b[1;37m{}\x1b[0m\n", lib.count(), target_path.display());
                    }
                }
                Err(e) => {
                    println!("\x1b[1;31mFailed to export templates:\x1b[0m {}\n", e);
                }
            }
        }

        PromptCommand::Import { path } => {
            let file_path = PathBuf::from(path);
            if !file_path.exists() {
                println!("\x1b[1;31mError:\x1b[0m File not found: \x1b[1;37m{}\x1b[0m\n", file_path.display());
                return;
            }

            match std::fs::read_to_string(&file_path) {
                Ok(content) => {
                    match lib.import_all_json(&content) {
                        Ok(count) => {
                            println!("\x1b[1;32m✓ Successfully imported {} prompt templates from:\x1b[0m \x1b[1;37m{}\x1b[0m\n", count, file_path.display());
                        }
                        Err(_) => {
                            match lib.load_from_file(&file_path) {
                                Ok(tmpl) => {
                                    let name = tmpl.name.clone();
                                    lib.insert(tmpl);
                                    println!("\x1b[1;32m✓ Successfully imported prompt template '{}' from:\x1b[0m \x1b[1;37m{}\x1b[0m\n", name, file_path.display());
                                }
                                Err(e) => {
                                    println!("\x1b[1;31mFailed to import prompt templates:\x1b[0m {}\n", e);
                                }
                            }
                        }
                    }
                }
                Err(e) => {
                    println!("\x1b[1;31mFailed to read import file:\x1b[0m {}\n", e);
                }
            }
        }
    }
}

fn handle_tools(runner: &AgentRunner) {
    let defs = runner.tools().definitions();
    println!("\x1b[1;36mRegistered Tools:\x1b[0m ({} total)", defs.len());
    for tool in defs {
        println!(
            "  • \x1b[1;33m{:<15}\x1b[0m \x1b[2;37m{}\x1b[0m",
            tool.name, tool.description
        );
    }
    println!();
}

fn handle_unknown(name: &str, _args: &[String]) {
    println!(
        "\x1b[1;31mUnknown command:\x1b[0m \x1b[1;37m{}\x1b[0m. Type \x1b[1;36m/help\x1b[0m or \x1b[1;36m/palette\x1b[0m to see available commands.\n",
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
    fn test_parse_palette() {
        assert_eq!(
            SlashCommand::parse("/palette"),
            Some(SlashCommand::Palette { filter: None })
        );
        assert_eq!(
            SlashCommand::parse("/commands"),
            Some(SlashCommand::Palette { filter: None })
        );
        assert_eq!(
            SlashCommand::parse("/pal session"),
            Some(SlashCommand::Palette {
                filter: Some("session".to_string())
            })
        );
    }

    #[test]
    fn test_parse_fork() {
        assert_eq!(
            SlashCommand::parse("/fork"),
            Some(SlashCommand::Fork {
                title: None,
                turn: None
            })
        );
        assert_eq!(
            SlashCommand::parse("/branch"),
            Some(SlashCommand::Fork {
                title: None,
                turn: None
            })
        );
        assert_eq!(
            SlashCommand::parse("/fork my-experiment"),
            Some(SlashCommand::Fork {
                title: Some("my-experiment".to_string()),
                turn: None
            })
        );
        assert_eq!(
            SlashCommand::parse("/fork 3"),
            Some(SlashCommand::Fork {
                title: None,
                turn: Some(3)
            })
        );
        assert_eq!(
            SlashCommand::parse("/fork \"my experiment\" 2"),
            Some(SlashCommand::Fork {
                title: Some("my experiment".to_string()),
                turn: Some(2)
            })
        );
        assert_eq!(
            SlashCommand::parse("/fork at 4"),
            Some(SlashCommand::Fork {
                title: None,
                turn: Some(4)
            })
        );
    }

    #[test]
    fn test_parse_rewind() {
        assert_eq!(
            SlashCommand::parse("/rewind"),
            Some(SlashCommand::Rewind { turns: None })
        );
        assert_eq!(
            SlashCommand::parse("/rewind 2"),
            Some(SlashCommand::Rewind { turns: Some(2) })
        );
        assert_eq!(
            SlashCommand::parse("/undo"),
            Some(SlashCommand::Rewind { turns: None })
        );
        assert_eq!(
            SlashCommand::parse("/undo 3"),
            Some(SlashCommand::Rewind { turns: Some(3) })
        );
        assert_eq!(
            SlashCommand::parse("/rw 1"),
            Some(SlashCommand::Rewind { turns: Some(1) })
        );
    }

    #[test]
    fn test_parse_compact() {
        assert_eq!(SlashCommand::parse("/compact"), Some(SlashCommand::Compact));
        assert_eq!(
            SlashCommand::parse("/compress"),
            Some(SlashCommand::Compact)
        );
    }

    #[test]
    fn test_parse_stats() {
        assert_eq!(SlashCommand::parse("/stats"), Some(SlashCommand::Stats));
        assert_eq!(SlashCommand::parse("/usage"), Some(SlashCommand::Stats));
        assert_eq!(SlashCommand::parse("/cost"), Some(SlashCommand::Stats));
    }
    #[test]
    fn test_parse_benchmark() {
        assert_eq!(
            SlashCommand::parse("/benchmark"),
            Some(SlashCommand::Benchmark { args: Vec::new() })
        );
        assert_eq!(
            SlashCommand::parse("/bench deepseek"),
            Some(SlashCommand::Benchmark {
                args: vec!["deepseek".to_string()]
            })
        );
        assert_eq!(
            SlashCommand::parse("/latency -n 3 --parallel"),
            Some(SlashCommand::Benchmark {
                args: vec!["-n".to_string(), "3".to_string(), "--parallel".to_string()]
            })
        );
        assert_eq!(
            SlashCommand::parse("/speed --ping"),
            Some(SlashCommand::Benchmark {
                args: vec!["--ping".to_string()]
            })
        );
    }
    #[test]
    fn test_parse_bookmark() {
        assert_eq!(
            SlashCommand::parse("/bookmark"),
            Some(SlashCommand::Bookmark { args: Vec::new() })
        );
        assert_eq!(
            SlashCommand::parse("/bookmark my-point"),
            Some(SlashCommand::Bookmark {
                args: vec!["my-point".to_string()]
            })
        );
        assert_eq!(
            SlashCommand::parse("/bm list"),
            Some(SlashCommand::Bookmark {
                args: vec!["list".to_string()]
            })
        );
        assert_eq!(
            SlashCommand::parse("/mark recall v1"),
            Some(SlashCommand::Bookmark {
                args: vec!["recall".to_string(), "v1".to_string()]
            })
        );
    }

    #[test]
    fn test_parse_snippet() {
        assert_eq!(
            SlashCommand::parse("/snippet"),
            Some(SlashCommand::Snippet { args: Vec::new() })
        );
        assert_eq!(
            SlashCommand::parse("/snippet save auth-fn fn authenticate() {}"),
            Some(SlashCommand::Snippet {
                args: vec![
                    "save".to_string(),
                    "auth-fn".to_string(),
                    "fn".to_string(),
                    "authenticate()".to_string(),
                    "{}".to_string(),
                ]
            })
        );
        assert_eq!(
            SlashCommand::parse("/snippet insert auth-fn"),
            Some(SlashCommand::Snippet {
                args: vec!["insert".to_string(), "auth-fn".to_string()]
            })
        );
        assert_eq!(
            SlashCommand::parse("/snip list"),
            Some(SlashCommand::Snippet {
                args: vec!["list".to_string()]
            })
        );
        assert_eq!(
            SlashCommand::parse("/sn recall my-snip"),
            Some(SlashCommand::Snippet {
                args: vec!["recall".to_string(), "my-snip".to_string()]
            })
        );
    }

    #[test]
    fn test_parse_preset() {
        assert_eq!(
            SlashCommand::parse("/preset"),
            Some(SlashCommand::Preset { name: None })
        );
        assert_eq!(
            SlashCommand::parse("/preset coding-fast"),
            Some(SlashCommand::Preset {
                name: Some("coding-fast".to_string())
            })
        );
        assert_eq!(
            SlashCommand::parse("/presets deep-reasoning"),
            Some(SlashCommand::Preset {
                name: Some("deep-reasoning".to_string())
            })
        );
        assert_eq!(
            SlashCommand::parse("/pre cheap"),
            Some(SlashCommand::Preset {
                name: Some("cheap".to_string())
            })
        );
        assert_eq!(
            SlashCommand::parse("/config preset offline-ollama"),
            Some(SlashCommand::Config(ConfigCommand::Preset {
                name: Some("offline-ollama".to_string())
            }))
        );
    }

    #[test]
    fn test_parse_export() {
        assert_eq!(
            SlashCommand::parse("/export"),
            Some(SlashCommand::Export {
                format: Some(ExportFormat::Markdown),
                path: None
            })
        );
        assert_eq!(
            SlashCommand::parse("/export md"),
            Some(SlashCommand::Export {
                format: Some(ExportFormat::Markdown),
                path: None
            })
        );
        assert_eq!(
            SlashCommand::parse("/export html"),
            Some(SlashCommand::Export {
                format: Some(ExportFormat::Html),
                path: None
            })
        );
        assert_eq!(
            SlashCommand::parse("/export html out.html"),
            Some(SlashCommand::Export {
                format: Some(ExportFormat::Html),
                path: Some("out.html".to_string())
            })
        );
        assert_eq!(
            SlashCommand::parse("/export md /tmp/notes.md"),
            Some(SlashCommand::Export {
                format: Some(ExportFormat::Markdown),
                path: Some("/tmp/notes.md".to_string())
            })
        );
        assert_eq!(
            SlashCommand::parse("/export my_log.html"),
            Some(SlashCommand::Export {
                format: Some(ExportFormat::Html),
                path: Some("my_log.html".to_string())
            })
        );
    }

    #[test]
    fn test_parse_trace() {
        assert_eq!(
            SlashCommand::parse("/trace"),
            Some(SlashCommand::Trace { path: None })
        );
        assert_eq!(
            SlashCommand::parse("/trace debug.md"),
            Some(SlashCommand::Trace {
                path: Some("debug.md".to_string())
            })
        );
    }

    #[test]
    fn test_parse_file() {
        assert_eq!(
            SlashCommand::parse("/file"),
            Some(SlashCommand::File { query: None })
        );
        assert_eq!(
            SlashCommand::parse("/f"),
            Some(SlashCommand::File { query: None })
        );
        assert_eq!(
            SlashCommand::parse("/find main.rs"),
            Some(SlashCommand::File {
                query: Some("main.rs".to_string())
            })
        );
        assert_eq!(
            SlashCommand::parse("/file picker"),
            Some(SlashCommand::File {
                query: Some("picker".to_string())
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
        assert_eq!(
            SlashCommand::parse("/session search \"auth bug\""),
            Some(SlashCommand::Session(SessionCommand::Search {
                query: "auth bug".to_string()
            }))
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
    fn test_execute_fork() {
        let client = crate::provider::LlmClient::new();
        let config = Config::default();
        let tools = crate::tools::ToolRegistry::new();
        let tool_ctx = crate::tools::ToolContext {
            cwd: std::path::PathBuf::from("."),
            env: std::collections::HashMap::new(),
        };
        let mut runner = AgentRunner::new(client, config, tools, tool_ctx);
        let mut session = Session::new("deepseek-chat");
        session.add_user_message("Write an HTTP server");
        session.add_assistant_message("Here is code...");
        assert_eq!(session.total_messages(), 2);

        let orig_id = session.id();
        let res = handle_slash_command("/fork \"my-test-branch\"", &mut runner, &mut session);
        assert!(res.is_some());
        assert!(matches!(res.unwrap(), CommandResult::SessionSwitched(_)));

        assert_ne!(session.id(), orig_id);
        assert_eq!(session.title().unwrap(), "my-test-branch");
        assert_eq!(session.total_messages(), 2);
        assert_eq!(
            session.get_metadata("forked_from").unwrap(),
            orig_id.to_string()
        );
    }

    #[test]
    fn test_execute_rewind() {
        let client = crate::provider::LlmClient::new();
        let config = Config::default();
        let tools = crate::tools::ToolRegistry::new();
        let tool_ctx = crate::tools::ToolContext {
            cwd: std::path::PathBuf::from("."),
            env: std::collections::HashMap::new(),
        };
        let mut runner = AgentRunner::new(client, config, tools, tool_ctx);
        let mut session = Session::new("deepseek-chat");
        session.add_user_message("First question");
        session.add_assistant_message("First answer");
        session.add_user_message("Second question");
        session.add_assistant_message("Second answer");
        assert_eq!(session.total_messages(), 4);

        let res = handle_slash_command("/rewind 1", &mut runner, &mut session);
        assert!(res.is_some());
        assert_eq!(session.total_messages(), 2);
        assert_eq!(session.messages()[1].content, "First answer");
    }

    #[test]
    fn test_execute_compact() {
        let client = crate::provider::LlmClient::new();
        let config = Config::default();
        let tools = crate::tools::ToolRegistry::new();
        let tool_ctx = crate::tools::ToolContext {
            cwd: std::path::PathBuf::from("."),
            env: std::collections::HashMap::new(),
        };
        let mut runner = AgentRunner::new(client, config, tools, tool_ctx);
        let mut session = Session::new("deepseek-chat");

        // Compacting short session reports no compaction needed
        session.add_user_message("Hi");
        let res = handle_slash_command("/compact", &mut runner, &mut session);
        assert!(res.is_some());

        // Add more messages
        for i in 0..10 {
            session.add_user_message(format!("Turn question {}", i));
            session.add_assistant_message(format!("Turn response {}", i));
        }
        let res2 = handle_slash_command("/compact", &mut runner, &mut session);
        assert!(res2.is_some());
    }

    #[test]
    fn test_execute_stats() {
        let client = crate::provider::LlmClient::new();
        let config = Config::default();
        let tools = crate::tools::ToolRegistry::new();
        let tool_ctx = crate::tools::ToolContext {
            cwd: std::path::PathBuf::from("."),
            env: std::collections::HashMap::new(),
        };
        let mut runner = AgentRunner::new(client, config, tools, tool_ctx);
        let mut session = Session::new("deepseek-chat");
        session.add_user_message("What is Fusion?");
        session.add_assistant_message("Fusion is a pure-Rust AI coding assistant.");
        session.record_usage(150, 45);

        let res = handle_slash_command("/stats", &mut runner, &mut session);
        assert!(res.is_some());
    }

    #[test]
    fn test_execute_export() {
        let client = crate::provider::LlmClient::new();
        let config = Config::default();
        let tools = crate::tools::ToolRegistry::new();
        let tool_ctx = crate::tools::ToolContext {
            cwd: std::path::PathBuf::from("."),
            env: std::collections::HashMap::new(),
        };
        let mut runner = AgentRunner::new(client, config, tools, tool_ctx);
        let mut session = Session::new("deepseek-chat");
        session.add_user_message("Test export question");
        session.add_assistant_message("Test export response");

        let temp_md = std::env::temp_dir().join("fusion_test_export.md");
        let temp_html = std::env::temp_dir().join("fusion_test_export.html");

        let res_md = handle_slash_command(
            &format!("/export md {}", temp_md.display()),
            &mut runner,
            &mut session,
        );
        assert!(res_md.is_some());
        assert!(temp_md.exists());
        let md_content = std::fs::read_to_string(&temp_md).unwrap();
        assert!(md_content.contains("Test export question"));
        let _ = std::fs::remove_file(&temp_md);

        let res_html = handle_slash_command(
            &format!("/export html {}", temp_html.display()),
            &mut runner,
            &mut session,
        );
        assert!(res_html.is_some());
        assert!(temp_html.exists());
        let html_content = std::fs::read_to_string(&temp_html).unwrap();
        assert!(html_content.contains("<!DOCTYPE html>"));
        assert!(html_content.contains("Test export question"));
        let _ = std::fs::remove_file(&temp_html);
    }

    #[test]
    fn test_command_palette_render() {
        let all = render_command_palette(None);
        assert!(all.contains("Fusion Command Palette"));
        assert!(all.contains("/fork"));
        assert!(all.contains("/rewind"));
        assert!(all.contains("/compact"));
        assert!(all.contains("/stats"));
        assert!(all.contains("/export"));

        let filtered = render_command_palette(Some("branch"));
        assert!(filtered.contains("/fork"));
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

        let res = handle_slash_command("/provider fusion", &mut runner, &mut session);
        assert!(res.is_some());
        assert_eq!(runner.config().default_provider, "fusion");
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

    #[test]
    fn test_parse_prompt_commands() {
        assert_eq!(
            SlashCommand::parse("/prompt"),
            Some(SlashCommand::Prompt(PromptCommand::List { filter: None }))
        );
        assert_eq!(
            SlashCommand::parse("/prompt list"),
            Some(SlashCommand::Prompt(PromptCommand::List { filter: None }))
        );
        assert_eq!(
            SlashCommand::parse("/prompt list review"),
            Some(SlashCommand::Prompt(PromptCommand::List {
                filter: Some("review".to_string())
            }))
        );
        assert_eq!(
            SlashCommand::parse("/prompt save my-review \"Please check {{code}}\""),
            Some(SlashCommand::Prompt(PromptCommand::Save {
                name: "my-review".to_string(),
                template: Some("Please check {{code}}".to_string()),
                local: false,
            }))
        );
        assert_eq!(
            SlashCommand::parse("/prompt load review code=\"fn main() {}\""),
            Some(SlashCommand::Prompt(PromptCommand::Load {
                name: "review".to_string(),
                args: vec!["code=fn main() {}".to_string()],
            }))
        );
        assert_eq!(
            SlashCommand::parse("/prompt show refactor"),
            Some(SlashCommand::Prompt(PromptCommand::Show {
                name: "refactor".to_string(),
            }))
        );
        assert_eq!(
            SlashCommand::parse("/prompt delete custom-tmpl"),
            Some(SlashCommand::Prompt(PromptCommand::Delete {
                name: "custom-tmpl".to_string(),
            }))
        );
        assert_eq!(
            SlashCommand::parse("/prompt search testing"),
            Some(SlashCommand::Prompt(PromptCommand::Search {
                query: "testing".to_string(),
            }))
        );
    }

    #[test]
    fn test_execute_prompt_commands() {
        let client = crate::provider::LlmClient::new();
        let config = Config::default();
        let tools = crate::tools::ToolRegistry::new();
        let tool_ctx = crate::tools::ToolContext {
            cwd: std::path::PathBuf::from("."),
            env: std::collections::HashMap::new(),
        };
        let mut runner = AgentRunner::new(client, config, tools, tool_ctx);
        let mut session = Session::new("deepseek-chat");

        // 1. List
        let res_list = handle_slash_command("/prompt list", &mut runner, &mut session);
        assert!(res_list.is_some());

        // 2. Show
        let res_show = handle_slash_command("/prompt show review", &mut runner, &mut session);
        assert!(res_show.is_some());

        // 3. Search
        let res_search = handle_slash_command("/prompt search refactor", &mut runner, &mut session);
        assert!(res_search.is_some());

        // 4. Load with argument
        let res_load = handle_slash_command(
            "/prompt load review code=\"fn add(a: i32, b: i32) -> i32 { a + b }\"",
            &mut runner,
            &mut session,
        );
        assert!(res_load.is_some());
        assert_eq!(session.total_messages(), 1);
        assert!(session.messages()[0].content.contains("fn add"));
    }

    #[test]
    fn test_parse_tag_commands() {
        assert_eq!(
            SlashCommand::parse("/tag"),
            Some(SlashCommand::Tag { args: Vec::new() })
        );
        assert_eq!(
            SlashCommand::parse("/tags"),
            Some(SlashCommand::Tag { args: Vec::new() })
        );
        assert_eq!(
            SlashCommand::parse("/tag add rust backend"),
            Some(SlashCommand::Tag {
                args: vec!["add".to_string(), "rust".to_string(), "backend".to_string()]
            })
        );
        assert_eq!(
            SlashCommand::parse("/tag list --all"),
            Some(SlashCommand::Tag {
                args: vec!["list".to_string(), "--all".to_string()]
            })
        );
        assert_eq!(
            SlashCommand::parse("/tag filter rust"),
            Some(SlashCommand::Tag {
                args: vec!["filter".to_string(), "rust".to_string()]
            })
        );
        assert_eq!(
            SlashCommand::parse("/tag rm rust"),
            Some(SlashCommand::Tag {
                args: vec!["rm".to_string(), "rust".to_string()]
            })
        );
    }

    #[test]
    fn test_execute_tag_commands() {
        let client = crate::provider::LlmClient::new();
        let config = Config::default();
        let tools = crate::tools::ToolRegistry::new();
        let tool_ctx = crate::tools::ToolContext {
            cwd: std::path::PathBuf::from("."),
            env: std::collections::HashMap::new(),
        };
        let mut runner = AgentRunner::new(client, config, tools, tool_ctx);
        let mut session = Session::new("gpt-4o");

        // 1. Tag active session
        let res_add = handle_slash_command("/tag add rust backend", &mut runner, &mut session);
        assert!(res_add.is_some());
        assert!(crate::agent::tagging::has_tag(&session, "rust"));
        assert!(crate::agent::tagging::has_tag(&session, "backend"));

        // 2. List tags
        let res_list = handle_slash_command("/tag list", &mut runner, &mut session);
        assert!(res_list.is_some());

        // 3. Remove tag
        let res_rm = handle_slash_command("/tag rm backend", &mut runner, &mut session);
        assert!(res_rm.is_some());
        assert!(!crate::agent::tagging::has_tag(&session, "backend"));
        assert!(crate::agent::tagging::has_tag(&session, "rust"));
    }
}
