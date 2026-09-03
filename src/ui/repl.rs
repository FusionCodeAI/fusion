use crossterm::{
    cursor, execute,
    terminal::{self, ClearType},
};
use std::io::{stdout, Write};
use std::time::Instant;

use crate::agent::{AgentEvent, AgentRunner, Session};
use crate::config::Config;
use crate::ui::markdown::MarkdownRenderer;
use crate::ui::prompt::{Prompt, PromptResult};

pub fn print_banner(_config: &Config) {}

pub fn print_help() {
    crate::ui::slash::print_command_palette(None);
}

pub fn handle_command(cmd: &str, runner: &mut AgentRunner, session: &mut Session) -> bool {
    if let Some(result) = crate::ui::slash::handle_slash_command(cmd, runner, session) {
        result.is_exit()
    } else {
        false
    }
}

/// Format elapsed duration concisely: e.g. "5s", "1m 7s", "1h2m3s".
pub fn format_duration_compact(duration: std::time::Duration) -> String {
    let total_secs = duration.as_secs();
    let secs = total_secs % 60;
    let total_mins = total_secs / 60;
    let mins = total_mins % 60;
    let hours = total_mins / 60;

    if hours > 0 {
        format!("{}h{}m{}s", hours, mins, secs)
    } else if mins > 0 {
        format!("{}m {}s", mins, secs)
    } else {
        format!("{}s", secs)
    }
}

/// Format token count compactly: e.g. "1", "57", "1.2k", "45k".
pub fn format_tokens_compact(tokens: u64) -> String {
    if tokens < 1000 {
        format!("{}", tokens)
    } else {
        let whole = tokens / 1000;
        let tenths = (tokens % 1000) / 100;
        if whole < 10 && tenths > 0 {
            format!("{}.{}k", whole, tenths)
        } else {
            format!("{}k", whole)
        }
    }
}

/// Format raw model string into display label.
pub fn format_model_label(model: &str) -> &str {
    match model {
        "deepseek-ai/DeepSeek-V4-Flash-0731" | "flash" | "v4" => "DeepSeek V4 Flash",
        "MiniMaxAI/MiniMax-M2.7" | "minimax" => "MiniMax M2.7",
        "moonshotai/Kimi-K2.6" | "kimi" => "Kimi K2.6",
        other => {
            if let Some((_, name)) = other.split_once('/') {
                name
            } else if other.is_empty() {
                "auto"
            } else {
                other
            }
        }
    }
}

/// Format real-time thinking status frame.
pub fn format_thinking_status(
    elapsed: std::time::Duration,
    in_tokens: u64,
    out_tokens: u64,
    model_label: &str,
) -> String {
    let elapsed_str = format_duration_compact(elapsed);
    let in_str = format_tokens_compact(in_tokens);
    let out_str = format_tokens_compact(out_tokens);
    format!(
        "  \x1b[2;37mThinking ({}) (↑{} ↓{})\x1b[0m\r\n\r\n\x1b[38;5;75m┃\x1b[0m \r\n\x1b[2;37menter queue · auto · {}\x1b[0m\r\n",
        elapsed_str, in_str, out_str, model_label
    )
}

/// Format completed turn summary: duration and tokens.
pub fn format_turn_summary(
    elapsed: std::time::Duration,
    in_tokens: u64,
    out_tokens: u64,
) -> String {
    let elapsed_str = format_duration_compact(elapsed);
    let in_str = format_tokens_compact(in_tokens);
    let out_str = format_tokens_compact(out_tokens);
    format!("  \x1b[2;37m{} (↑{} ↓{})\x1b[0m\r\n\r\n", elapsed_str, in_str, out_str)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolCallItem {
    pub name: String,
    pub label: String,
    pub category: String,
}

impl ToolCallItem {
    pub fn new(name: impl Into<String>, label: impl Into<String>, category: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            label: label.into(),
            category: category.into(),
        }
    }
}

pub fn parse_tool_info(name: &str, args: &serde_json::Value) -> (String, String) {
    match name {
        "glob" | "file_glob" | "glob_files" => {
            let pattern = args
                .get("pattern")
                .or_else(|| args.get("glob"))
                .and_then(|v| v.as_str())
                .unwrap_or("*");
            (format!("Matched {}", pattern), "list".to_string())
        }
        "grep" | "file_grep" | "grep_files" => {
            let pattern = args
                .get("pattern")
                .or_else(|| args.get("query"))
                .and_then(|v| v.as_str())
                .unwrap_or("");
            (format!("Searched {}", pattern), "list".to_string())
        }
        "file" | "read" | "file_read" | "read_file" => {
            let action = args.get("action").and_then(|v| v.as_str()).unwrap_or("read");
            let path = args
                .get("path")
                .or_else(|| args.get("file"))
                .and_then(|v| v.as_str())
                .unwrap_or("file");
            if action == "write" {
                (format!("Wrote {}", path), "write".to_string())
            } else {
                (format!("Read {}", path), "read".to_string())
            }
        }
        "write" | "file_write" | "write_file" => {
            let path = args
                .get("path")
                .or_else(|| args.get("file"))
                .and_then(|v| v.as_str())
                .unwrap_or("file");
            (format!("Wrote {}", path), "write".to_string())
        }
        "edit" | "file_edit" | "edit_file" => {
            let path = args
                .get("path")
                .or_else(|| args.get("file"))
                .and_then(|v| v.as_str())
                .unwrap_or("file");
            (format!("Edited {}", path), "edit".to_string())
        }
        "patch" | "apply_patch" => {
            let path = args
                .get("path")
                .or_else(|| args.get("file"))
                .and_then(|v| v.as_str())
                .unwrap_or("file");
            (format!("Patched {}", path), "edit".to_string())
        }
        "bash" | "shell" | "process" => {
            let cmd = args
                .get("command")
                .or_else(|| args.get("cmd"))
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let single_line = cmd.replace('\n', " ").replace('\r', " ");
            let trimmed = single_line.trim();
            let preview = if trimmed.len() > 40 {
                let end = trimmed
                    .char_indices()
                    .map(|(i, _)| i)
                    .take(39)
                    .last()
                    .unwrap_or(trimmed.len());
                format!("{}…", &trimmed[..end])
            } else {
                trimmed.to_string()
            };
            (format!("Ran {}", preview), "command".to_string())
        }
        "web_search" => {
            let q = args
                .get("query")
                .or_else(|| args.get("q"))
                .and_then(|v| v.as_str())
                .unwrap_or("");
            (format!("Searched {}", q), "read".to_string())
        }
        "fetch" | "web_fetch" => {
            let url = args.get("url").and_then(|v| v.as_str()).unwrap_or("");
            (format!("Fetched {}", url), "read".to_string())
        }
        other => (format!("Used {}", other), "other".to_string()),
    }
}

/// Format an aggregated tool call tree string.
pub fn format_tool_tree(tool_batch: &[ToolCallItem]) -> String {
    if tool_batch.is_empty() {
        return String::new();
    }
    let total = tool_batch.len();
    let mut category_counts: std::collections::BTreeMap<&str, usize> =
        std::collections::BTreeMap::new();
    for item in tool_batch {
        *category_counts.entry(&item.category).or_insert(0) += 1;
    }
    let mut sorted_cats: Vec<_> = category_counts.into_iter().collect();
    sorted_cats.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(b.0)));
    let mut breakdown = Vec::new();
    for (cat, count) in sorted_cats {
        breakdown.push(format!("{} {}", count, cat));
    }
    let breakdown_str = if breakdown.is_empty() {
        String::new()
    } else {
        format!(" · {}", breakdown.join(" · "))
    };

    let call_label = if total == 1 { "tool call" } else { "tool calls" };
    let mut out = format!("● {} {}{}\n", total, call_label, breakdown_str);
    for (i, item) in tool_batch.iter().enumerate() {
        let connector = if i == total - 1 { "└─" } else { "├─" };
        out.push_str(&format!("{} {}\n", connector, item.label));
    }
    out
}

fn clear_thinking_frame() {
    let mut out = stdout();
    let _ = execute!(
        out,
        cursor::MoveUp(2),
        cursor::MoveToColumn(0),
        terminal::Clear(ClearType::FromCursorDown)
    );
    let _ = out.flush();
}
/// Render tool call tree to any generic writer with ANSI styling.
pub fn render_tool_tree_to<W: std::io::Write>(out: &mut W, tool_batch: &[ToolCallItem]) -> std::io::Result<()> {
    if tool_batch.is_empty() {
        return Ok(());
    }
    let total = tool_batch.len();
    let mut category_counts: std::collections::BTreeMap<&str, usize> =
        std::collections::BTreeMap::new();
    for item in tool_batch {
        *category_counts.entry(&item.category).or_insert(0) += 1;
    }
    let mut sorted_cats: Vec<_> = category_counts.into_iter().collect();
    sorted_cats.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(b.0)));
    let mut breakdown = Vec::new();
    for (cat, count) in sorted_cats {
        breakdown.push(format!("{} {}", count, cat));
    }
    let breakdown_str = if breakdown.is_empty() {
        String::new()
    } else {
        format!(" · {}", breakdown.join(" · "))
    };

    let call_label = if total == 1 { "tool call" } else { "tool calls" };
    write!(out, "\x1b[2;37m● {} {}{}\x1b[0m\r\n", total, call_label, breakdown_str)?;
    for (i, item) in tool_batch.iter().enumerate() {
        let connector = if i == total - 1 { "└─" } else { "├─" };
        write!(out, "\x1b[2;37m{} {}\x1b[0m\r\n", connector, item.label)?;
    }
    write!(out, "\r\n")?;
    out.flush()?;
    Ok(())
}

pub fn render_tool_tree(tool_batch: &[ToolCallItem]) {
    let _ = render_tool_tree_to(&mut stdout(), tool_batch);
}

pub async fn run_turn_ui(
    runner: &AgentRunner,
    session: &mut Session,
    user_input: &str,
) -> anyhow::Result<String> {
    let start_time = Instant::now();
    let mut input_tokens = (crate::agent::tokens::estimate_text_tokens(user_input)
        + session.estimate_tokens())
    .max(1) as u64;
    let mut output_tokens = 0u64;
    let model_label = format_model_label(session.active_model()).to_string();
    let mut md = MarkdownRenderer::new().with_indent(2);

    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    let runner_task = runner.run_turn_stream(session, user_input, tx);
    tokio::pin!(runner_task);

    let mut is_thinking = true;
    let mut thinking_displayed = false;
    let mut tool_batch: Vec<ToolCallItem> = Vec::new();
    let mut ticker = tokio::time::interval(std::time::Duration::from_millis(250));

    loop {
        tokio::select! {
            _ = ticker.tick(), if is_thinking => {
                let elapsed = start_time.elapsed();
                let elapsed_str = format_duration_compact(elapsed);
                let in_str = format_tokens_compact(input_tokens);
                let out_str = format_tokens_compact(output_tokens);

                let mut out = stdout();
                if thinking_displayed {
                    clear_thinking_frame();
                }

                let _ = write!(
                    out,
                    "  \x1b[2;37mThinking ({}) (↑{} ↓{})\x1b[0m\r\n\r\n\x1b[38;5;75m┃\x1b[0m \r\n\x1b[2;37menter queue · auto · {}\x1b[0m\r\n",
                    elapsed_str, in_str, out_str, model_label
                );
                let _ = execute!(out, cursor::MoveUp(2), cursor::MoveToColumn(2));
                let _ = out.flush();
                thinking_displayed = true;
            }
            res = &mut runner_task => {
                while let Ok(event) = rx.try_recv() {
                    match event {
                        AgentEvent::TextDelta(d) => {
                            if thinking_displayed {
                                clear_thinking_frame();
                                thinking_displayed = false;
                            }
                            if is_thinking {
                                is_thinking = false;
                            }
                            if !tool_batch.is_empty() {
                                render_tool_tree(&tool_batch);
                                tool_batch.clear();
                            }
                            output_tokens += crate::agent::tokens::estimate_text_tokens(&d) as u64;
                            md.push(&d);
                        }
                        AgentEvent::ThinkingDelta(th) => {
                            output_tokens += crate::agent::tokens::estimate_text_tokens(&th) as u64;
                        }
                        AgentEvent::Finished { usage } => {
                            if let Some(u) = &usage {
                                if let Some(pt) = u.get("prompt_tokens").and_then(|v| v.as_u64()) {
                                    input_tokens = pt;
                                }
                                if let Some(ct) = u.get("completion_tokens").and_then(|v| v.as_u64()) {
                                    output_tokens = ct;
                                }
                            }
                        }
                        _ => {}
                    }
                }
                if thinking_displayed {
                    clear_thinking_frame();
                }
                if !tool_batch.is_empty() {
                    render_tool_tree(&tool_batch);
                    tool_batch.clear();
                }
                md.finish();

                let elapsed = start_time.elapsed();
                let elapsed_str = format_duration_compact(elapsed);
                let in_str = format_tokens_compact(input_tokens);
                let out_str = format_tokens_compact(output_tokens);
                println!("\x1b[2;37m  {} (↑{} ↓{})\x1b[0m\n", elapsed_str, in_str, out_str);
                let _ = stdout().flush();
                return res;
            }
            Some(event) = rx.recv() => {
                match event {
                    AgentEvent::ThinkingDelta(th) => {
                        output_tokens += crate::agent::tokens::estimate_text_tokens(&th) as u64;
                    }
                    AgentEvent::TextDelta(d) => {
                        if thinking_displayed {
                            clear_thinking_frame();
                            thinking_displayed = false;
                        }
                        if is_thinking {
                            is_thinking = false;
                        }
                        if !tool_batch.is_empty() {
                            render_tool_tree(&tool_batch);
                            tool_batch.clear();
                        }
                        output_tokens += crate::agent::tokens::estimate_text_tokens(&d) as u64;
                        md.push(&d);
                    }
                    AgentEvent::ToolStarted { name, args, .. } => {
                        if thinking_displayed {
                            clear_thinking_frame();
                            thinking_displayed = false;
                        }
                        is_thinking = false;
                        md.finish();
                        let (label, category) = parse_tool_info(&name, &args);
                        tool_batch.push(ToolCallItem {
                            name,
                            label,
                            category,
                        });
                    }
                    AgentEvent::ToolFinished { .. } => {
                        // Tool completed, wait for next events or tree flush
                    }
                    AgentEvent::Error(err) => {
                        if thinking_displayed {
                            clear_thinking_frame();
                            thinking_displayed = false;
                        }
                        eprintln!("\n\x1b[31m❌ Error: {}\x1b[0m", err);
                    }
                    AgentEvent::Finished { usage } => {
                        if let Some(u) = &usage {
                            if let Some(pt) = u.get("prompt_tokens").and_then(|v| v.as_u64()) {
                                input_tokens = pt;
                            }
                            if let Some(ct) = u.get("completion_tokens").and_then(|v| v.as_u64()) {
                                output_tokens = ct;
                            }
                        }
                        if !tool_batch.is_empty() {
                            render_tool_tree(&tool_batch);
                            tool_batch.clear();
                        }
                        md.finish();
                    }
                    _ => {}
                }
            }
        }
    }
}
/// Run the interactive lightweight inline REPL loop.
pub async fn run_repl(mut runner: AgentRunner) -> anyhow::Result<()> {
    let mut session = Session::new(&runner.config().default_model);
    let mut prompt = Prompt::new()
        .with_model(&runner.config().default_model)
        .with_models(model_picker_list(&crate::provider::catalog::get_catalog()));

    // Clear terminal screen and place cursor at top left (clean startup like fx)
    let _ = execute!(
        stdout(),
        terminal::Clear(ClearType::All),
        cursor::MoveTo(0, 0)
    );
    let _ = stdout().flush();

    // Clear any leftover recovery crash state so we start cleanly with a blank prompt
    let _ = runner.recovery().clear();

    loop {
        prompt.set_model(&runner.config().default_model);
        match prompt.read_input() {
            Ok(PromptResult::Submit(input)) => {
                let trimmed = input.trim();
                if trimmed.is_empty() {
                    continue;
                }

                // Handle slash commands
                if trimmed.starts_with('/') {
                    if handle_command(trimmed, &mut runner, &mut session) {
                        break;
                    }
                    prompt.set_model(&runner.config().default_model);
                    continue;
                }

                // Execute turn with live streaming
                let _ = run_turn_ui(&runner, &mut session, trimmed).await;
            }
            Ok(PromptResult::Cancel) => {
                println!("\x1b[2;37m(Turn canceled)\x1b[0m\n");
            }
            Ok(PromptResult::Exit) => {
                match session.save() {
                    Ok(path) => println!(
                        "\x1b[2;37mSession saved. Resume later with \x1b[1;36m/session load {}\x1b[0m",
                        path.file_stem().map(|s| s.to_string_lossy()).unwrap_or_default()
                    ),
                    Err(_) => println!("\x1b[2;37mGoodbye!\x1b[0m"),
                }
                break;
            }
            Err(e) => {
                eprintln!("Error reading input: {}", e);
                break;
            }
        }
    }

    Ok(())
}

/// Build the model list for the prompt picker dialog, showing only Fusion models.
fn model_picker_list(_catalog: &crate::provider::catalog::ModelCatalog) -> Vec<(String, String)> {
    vec![
        (
            "deepseek-ai/DeepSeek-V4-Flash-0731".to_string(),
            "DeepSeek V4 Flash (1M context · fast)".to_string(),
        ),
        (
            "MiniMaxAI/MiniMax-M2.7".to_string(),
            "MiniMax M2.7 (Reasoning · coding)".to_string(),
        ),
        (
            "moonshotai/Kimi-K2.6".to_string(),
            "Kimi K2.6 (Reasoning · 200K context)".to_string(),
        ),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn test_format_duration_compact() {
        assert_eq!(format_duration_compact(Duration::from_secs(0)), "0s");
        assert_eq!(format_duration_compact(Duration::from_secs(5)), "5s");
        assert_eq!(format_duration_compact(Duration::from_secs(67)), "1m 7s");
        assert_eq!(format_duration_compact(Duration::from_secs(3663)), "1h1m3s");
    }

    #[test]
    fn test_format_tokens_compact() {
        assert_eq!(format_tokens_compact(0), "0");
        assert_eq!(format_tokens_compact(1), "1");
        assert_eq!(format_tokens_compact(57), "57");
        assert_eq!(format_tokens_compact(999), "999");
        assert_eq!(format_tokens_compact(1000), "1k");
        assert_eq!(format_tokens_compact(1200), "1.2k");
        assert_eq!(format_tokens_compact(45000), "45k");
    }

    #[test]
    fn test_parse_tool_info() {
        let (label, cat) = parse_tool_info("glob", &serde_json::json!({ "pattern": "**/*.rs" }));
        assert_eq!(label, "Matched **/*.rs");
        assert_eq!(cat, "list");

        let (label, cat) = parse_tool_info("grep", &serde_json::json!({ "pattern": "mermaid" }));
        assert_eq!(label, "Searched mermaid");
        assert_eq!(cat, "list");

        let (label, cat) = parse_tool_info("file", &serde_json::json!({ "action": "read", "path": "README.md" }));
        assert_eq!(label, "Read README.md");
        assert_eq!(cat, "read");

        let (label, cat) = parse_tool_info("file", &serde_json::json!({ "action": "write", "path": "test.txt" }));
        assert_eq!(label, "Wrote test.txt");
        assert_eq!(cat, "write");

        let (label, cat) = parse_tool_info("bash", &serde_json::json!({ "command": "cargo test\necho ok" }));
        assert_eq!(label, "Ran cargo test echo ok");
        assert_eq!(cat, "command");
    }

    #[test]
    fn test_format_tool_tree() {
        let items = vec![
            ToolCallItem {
                name: "glob".to_string(),
                label: "Matched **/*.md".to_string(),
                category: "list".to_string(),
            },
            ToolCallItem {
                name: "file".to_string(),
                label: "Read README.md".to_string(),
                category: "read".to_string(),
            },
        ];
        let formatted = format_tool_tree(&items);
        assert!(formatted.contains("● 2 tool calls"));
        assert!(formatted.contains("├─ Matched **/*.md"));
        assert!(formatted.contains("└─ Read README.md"));
    }
}
