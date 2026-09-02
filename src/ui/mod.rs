pub mod inline;
pub mod markdown;
pub mod prompt;
pub mod repl;
pub mod spinner;
pub mod slash;

// Re-exports for convenient top-level access
pub use inline::{calculate_text_height, render_card, render_critique_card, render_status_bar, InlineTerminal, StatusInfo};
pub use markdown::{print_markdown, render_inline, render_line, render_markdown, MarkdownRenderer};
pub use prompt::{Prompt, PromptResult, RawModeGuard};
pub use repl::{handle_command, print_banner, print_help, run_repl, run_turn_ui};
pub use spinner::{format_duration, Spinner, SpinnerHandle};
pub use slash::{
    execute_slash_command, handle_slash_command, CommandResult, ConfigCommand, SessionCommand,
    SlashCommand,
};
