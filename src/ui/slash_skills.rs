//! Slash command handlers for extensible domain skills (`/skills`).
//!
//! Provides:
//! - `/skills` or `/skills list`: Renders a clean formatted table of all discovered
//!   project and global skills with columns: Name, Source, Triggers, Enabled status.
//! - `/skills show <name>`: Fetches the skill from [`SkillRegistry::scan_default`]
//!   and returns its formatted markdown instruction block.
//! - `/skills enable <name>` / `/skills disable <name>`: Toggles the skill enabled
//!   status in the registry.
//! - [`format_skills_palette`]: Formats a slice of skills into a clean terminal palette view.

use std::collections::HashMap;
use std::path::Path;
use std::sync::{LazyLock, Mutex};

use crate::agent::skills::{Skill, SkillRegistry, SkillSource};
use crate::ui::table::{ColumnAlign, Table};

/// In-memory override map for skill enabled/disabled states across command calls.
static SKILL_OVERRIDES: LazyLock<Mutex<HashMap<String, bool>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

/// Resets all skill enabled/disabled overrides. Primarily used for unit testing.
pub fn reset_skill_overrides() {
    if let Ok(mut overrides) = SKILL_OVERRIDES.lock() {
        overrides.clear();
    }
}

/// Helper to parse raw slash command arguments into `(subcommand, target_name)`.
fn parse_command_args(args: &[String]) -> (String, String) {
    if args.is_empty() {
        return ("list".to_string(), String::new());
    }

    let first = args[0].trim();
    if first.is_empty() {
        return ("list".to_string(), String::new());
    }

    // Handle single string with whitespace, e.g. ["show my-skill"] or ["enable foo"]
    if let Some((cmd, rest)) = first.split_once(char::is_whitespace) {
        let cmd = cmd.trim().to_lowercase();
        let name = if rest.trim().is_empty() && args.len() > 1 {
            args[1..].join(" ").trim().to_string()
        } else {
            rest.trim().to_string()
        };
        return (cmd, name);
    }

    let cmd = first.to_lowercase();
    let name = if args.len() > 1 {
        args[1..].join(" ").trim().to_string()
    } else {
        String::new()
    };

    (cmd, name)
}

/// Renders a formatted table summarizing all registered skills.
///
/// Table columns:
/// - Name: Canonical skill identifier.
/// - Source: Origin of skill (Project, Global, Builtin, Custom).
/// - Triggers: Matching triggers/keywords comma-separated, or `-` if none.
/// - Enabled Status: `Enabled` or `Disabled`.
pub fn render_skills_table(skills: &[&Skill]) -> String {
    let mut table = Table::new()
        .with_headers(vec!["Name", "Source", "Triggers", "Enabled Status"])
        .with_alignments(vec![
            ColumnAlign::Left,
            ColumnAlign::Left,
            ColumnAlign::Left,
            ColumnAlign::Center,
        ])
        .with_card_fallback(false);

    for skill in skills {
        let source_str = match &skill.source {
            SkillSource::Project(_) => "Project",
            SkillSource::Global(_) => "Global",
            SkillSource::Custom(_) => "Custom",
            SkillSource::Builtin => "Builtin",
        };

        let triggers_str = if skill.triggers().is_empty() {
            "-".to_string()
        } else {
            skill.triggers().join(", ")
        };

        let status_str = if skill.is_enabled() {
            "Enabled"
        } else {
            "Disabled"
        };

        table.add_row(vec![
            skill.name().to_string(),
            source_str.to_string(),
            triggers_str,
            status_str.to_string(),
        ]);
    }

    table.render()
}

/// Formats a list of skills into a clean, visually structured terminal palette view.
///
/// Displays each skill's active badge, identifier, source category, description,
/// and trigger keywords.
pub fn format_skills_palette(skills: &[&crate::agent::skills::Skill]) -> String {
    if skills.is_empty() {
        return "\x1b[1;33mℹ\x1b[0m No skills available in palette.\n".to_string();
    }

    let mut out = String::new();
    out.push_str("\x1b[1;36m╭────────────────────────────────────────────────────────────────────────────────╮\x1b[0m\n");
    out.push_str(&format!(
        "\x1b[1;36m│\x1b[0m \x1b[1;37m✦ Fusion Skills Palette\x1b[0m \x1b[2;37m({} registered)\x1b[0m                                      \x1b[1;36m│\x1b[0m\n",
        skills.len()
    ));
    out.push_str("\x1b[1;36m│\x1b[0m \x1b[2;37mUse \x1b[1;36m/skills show <name>\x1b[2;37m to inspect, or \x1b[1;36m@skill:<name>\x1b[2;37m to inject in prompt    \x1b[1;36m│\x1b[0m\n");
    out.push_str("\x1b[1;36m╰────────────────────────────────────────────────────────────────────────────────╯\x1b[0m\n\n");

    for skill in skills {
        let status_badge = if skill.is_enabled() {
            "\x1b[1;32m● Active\x1b[0m"
        } else {
            "\x1b[2;37m○ Disabled\x1b[0m"
        };

        let source_label = match &skill.source {
            SkillSource::Project(_) => "\x1b[1;34m[project]\x1b[0m",
            SkillSource::Global(_) => "\x1b[1;35m[global]\x1b[0m",
            SkillSource::Custom(_) => "\x1b[1;33m[custom]\x1b[0m",
            SkillSource::Builtin => "\x1b[1;32m[builtin]\x1b[0m",
        };

        let version_str = skill
            .metadata
            .version
            .as_deref()
            .map(|v| format!(" \x1b[2;37mv{}\x1b[0m", v))
            .unwrap_or_default();

        out.push_str(&format!(
            "  {} \x1b[1;37m{:<24}\x1b[0m {} {}\n",
            status_badge,
            skill.name(),
            source_label,
            version_str,
        ));

        if !skill.description().is_empty() {
            out.push_str(&format!("    \x1b[2;37m{}\x1b[0m\n", skill.description()));
        }

        if !skill.triggers().is_empty() {
            out.push_str(&format!(
                "    \x1b[2;36mTriggers:\x1b[0m \x1b[37m{}\x1b[0m\n",
                skill.triggers().join(", ")
            ));
        }

        out.push('\n');
    }

    out
}

/// Returns usage instructions for the `/skills` slash command.
fn format_skills_help() -> String {
    r#"Extensible Domain Skills Management:
  /skills [list]           - List all discovered project and global skills
  /skills show <name>      - View formatted markdown instruction block for a skill
  /skills enable <name>    - Enable a skill in the registry
  /skills disable <name>   - Disable a skill in the registry
  /skills palette          - Display interactive skills palette
"#
    .to_string()
}

/// Executes a skills slash command using an existing mutable [`SkillRegistry`].
pub fn handle_skills_command_with_registry(
    args: &[String],
    registry: &mut SkillRegistry,
) -> String {
    let (subcmd, target) = parse_command_args(args);

    match subcmd.as_str() {
        "" | "list" => {
            let list = registry.list();
            render_skills_table(&list)
        }
        "show" => {
            if target.is_empty() {
                return "Please specify a skill name: /skills show <name>".to_string();
            }
            let found = registry.get(&target).or_else(|| {
                registry
                    .list()
                    .into_iter()
                    .find(|s| s.name().eq_ignore_ascii_case(&target))
            });
            match found {
                Some(skill) => skill.format_prompt_block(),
                None => format!("Skill '{}' not found.", target),
            }
        }
        "enable" => {
            if target.is_empty() {
                return "Please specify a skill name: /skills enable <name>".to_string();
            }
            let canonical = registry
                .get(&target)
                .map(|s| s.name().to_string())
                .or_else(|| {
                    registry
                        .list()
                        .into_iter()
                        .find(|s| s.name().eq_ignore_ascii_case(&target))
                        .map(|s| s.name().to_string())
                });
            match canonical {
                Some(name) => {
                    registry.set_enabled(&name, true);
                    if let Ok(mut overrides) = SKILL_OVERRIDES.lock() {
                        overrides.insert(name.clone(), true);
                    }
                    format!("Enabled skill '{}'.", name)
                }
                None => format!("Skill '{}' not found.", target),
            }
        }
        "disable" => {
            if target.is_empty() {
                return "Please specify a skill name: /skills disable <name>".to_string();
            }
            let canonical = registry
                .get(&target)
                .map(|s| s.name().to_string())
                .or_else(|| {
                    registry
                        .list()
                        .into_iter()
                        .find(|s| s.name().eq_ignore_ascii_case(&target))
                        .map(|s| s.name().to_string())
                });
            match canonical {
                Some(name) => {
                    registry.set_enabled(&name, false);
                    if let Ok(mut overrides) = SKILL_OVERRIDES.lock() {
                        overrides.insert(name.clone(), false);
                    }
                    format!("Disabled skill '{}'.", name)
                }
                None => format!("Skill '{}' not found.", target),
            }
        }
        "palette" => {
            let list = registry.list();
            format_skills_palette(&list)
        }
        "help" => format_skills_help(),
        other => format!(
            "Unknown subcommand '{}'. Usage: /skills [list | show <name> | enable <name> | disable <name> | palette]",
            other
        ),
    }
}

/// Primary entry point for the `/skills` slash command.
///
/// Automatically scans discovered project skills (under `workspace_root/.fusion/skills/`)
/// and global skills (under `~/.fusion/skills/` and `~/.claude/skills/`).
///
/// Supported subcommands:
/// - `""` or `"list"`: Renders a clean formatted table of all discovered project
///   and global skills with columns: Name, Source, Triggers, Enabled status.
/// - `"show <name>"`: Fetches the skill from [`SkillRegistry::scan_default`]
///   and returns its formatted markdown instruction block.
/// - `"enable <name>"` / `"disable <name>"`: Toggles the skill enabled status in the registry.
pub fn handle_skills_command(args: &[String], workspace_root: Option<&Path>) -> String {
    let mut registry = SkillRegistry::scan_default(workspace_root);

    // Apply any active overrides
    if let Ok(overrides) = SKILL_OVERRIDES.lock() {
        for (name, enabled) in overrides.iter() {
            registry.set_enabled(name, *enabled);
        }
    }

    handle_skills_command_with_registry(args, &mut registry)
}
