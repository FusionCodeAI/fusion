//! `/login` -- log in or re-authenticate with your account.

use crate::app::actions::Action;
use crate::slash::command::{CommandExecCtx, CommandResult, SlashCommand};

pub struct LoginCommand;

impl SlashCommand for LoginCommand {
    fn name(&self) -> &str {
        "login"
    }

    fn description(&self) -> &str {
        "Log in to Fusion AI (or '/login x' for X.ai / Grok)"
    }

    fn usage(&self) -> &str {
        "/login [fusion|x]"
    }

    fn run(&self, _ctx: &mut CommandExecCtx, args: &str) -> CommandResult {
        let arg = args.trim().to_lowercase();
        if arg == "x" || arg == "grok" {
            CommandResult::Action(Action::Login)
        } else {
            CommandResult::Action(Action::FusionLogin)
        }
    }
}
