use crossterm::{
    cursor, execute,
    event::{Event, EventStream, KeyCode, KeyModifiers},
    terminal::{self, ClearType},
};
use futures::StreamExt;
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

/// Format elapsed duration concisely: e.g. "5s", "1m10s", "1h1m3s".
pub fn format_duration_compact(duration: std::time::Duration) -> String {
    let total_secs = duration.as_secs();
    let secs = total_secs % 60;
    let total_mins = total_secs / 60;
    let mins = total_mins % 60;
    let hours = total_mins / 60;

    if hours > 0 {
        format!("{}h{}m{}s", hours, mins, secs)
    } else if mins > 0 {
        format!("{}m{}s", mins, secs)
    } else {
        format!("{}s", secs)
    }
}

/// Format duration for individual tool executions.
pub fn format_tool_duration(duration: std::time::Duration) -> String {
    let millis = duration.as_millis();
    if millis < 1000 {
        format!("{}ms", millis)
    } else {
        format_duration_compact(duration)
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

/// Format real-time activity status (e.g. running, thinking).
pub fn format_activity_status(
    status_text: &str,
    elapsed: std::time::Duration,
    in_tokens: u64,
    out_tokens: u64,
    model_label: &str,
) -> String {
    let elapsed_str = format_duration_compact(elapsed);
    let in_str = format_tokens_compact(in_tokens);
    let out_str = format_tokens_compact(out_tokens);
    let status = if status_text.is_empty() {
        "• Running"
    } else {
        status_text
    };
    format!(
        "  \x1b[2;37m{} ({}) (↑{} ↓{})\x1b[0m\r\n\r\n\x1b[1m┃\x1b[0m \r\n\r\n\x1b[2;37menter queue · auto · {}\x1b[0m\r\n",
        status, elapsed_str, in_str, out_str, model_label
    )
}

/// Format real-time thinking status frame.
pub fn format_thinking_status(
    elapsed: std::time::Duration,
    in_tokens: u64,
    out_tokens: u64,
    model_label: &str,
) -> String {
    format_activity_status("• Running", elapsed, in_tokens, out_tokens, model_label)
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

/// Render real-time thinking frame with status, queue line, and model footer.
pub fn render_thinking_frame_to<W: std::io::Write>(
    out: &mut W,
    status_text: &str,
    elapsed: std::time::Duration,
    in_tokens: u64,
    out_tokens: u64,
    model_label: &str,
    queue_text: &str,
) -> std::io::Result<()> {
    let elapsed_str = format_duration_compact(elapsed);
    let in_str = format_tokens_compact(in_tokens);
    let out_str = format_tokens_compact(out_tokens);
    let status = if status_text.is_empty() {
        "• Running"
    } else {
        status_text
    };
    write!(
        out,
        "  \x1b[2;37m{} ({}) (↑{} ↓{})\x1b[0m\r\n\r\n\x1b[1m┃\x1b[0m {}\r\n\r\n\x1b[2;37menter queue · auto · {}\x1b[0m",
        status, elapsed_str, in_str, out_str, queue_text, model_label
    )?;
    let col = 2 + queue_text.len() as u16;
    execute!(out, cursor::MoveUp(2), cursor::MoveToColumn(col))?;
    out.flush()?;
    Ok(())
}

pub fn render_thinking_frame(
    status_text: &str,
    elapsed: std::time::Duration,
    in_tokens: u64,
    out_tokens: u64,
    model_label: &str,
    queue_text: &str,
) {
    let _ = render_thinking_frame_to(
        &mut stdout(),
        status_text,
        elapsed,
        in_tokens,
        out_tokens,
        model_label,
        queue_text,
    );
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolCallItem {
    pub name: String,
    pub label: String,
    pub category: String,
    pub failed: bool,
}

impl ToolCallItem {
    pub fn new(name: impl Into<String>, label: impl Into<String>, category: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            label: label.into(),
            category: category.into(),
            failed: false,
        }
    }

    pub fn with_failed(mut self, failed: bool) -> Self {
        self.failed = failed;
        self
    }
}

pub fn parse_tool_info(name: &str, args: &serde_json::Value) -> (String, String) {
    match name {
        "skill" | "load_skill" | "install_skill" | "use_skill" | "skill_runner" => {
            let skill = args
                .get("name")
                .or_else(|| args.get("skill"))
                .or_else(|| args.get("skill_name"))
                .or_else(|| args.get("id"))
                .and_then(|v| v.as_str())
                .unwrap_or("skill");
            (format!("Loaded skill {}", skill), "read".to_string())
        }
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
            let preview = if trimmed.len() > 120 {
                let end = trimmed
                    .char_indices()
                    .map(|(i, _)| i)
                    .take(119)
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

pub fn parse_tool_active_label(name: &str, args: &serde_json::Value) -> String {
    match name {
        "skill" | "load_skill" | "install_skill" | "use_skill" | "skill_runner" => {
            let skill = args
                .get("name")
                .or_else(|| args.get("skill"))
                .or_else(|| args.get("skill_name"))
                .or_else(|| args.get("id"))
                .and_then(|v| v.as_str())
                .unwrap_or("skill");
            format!("Loading skill {}", skill)
        }
        "glob" | "file_glob" | "glob_files" => {
            let pattern = args
                .get("pattern")
                .or_else(|| args.get("glob"))
                .and_then(|v| v.as_str())
                .unwrap_or("*");
            format!("Matching {}", pattern)
        }
        "grep" | "file_grep" | "grep_files" => {
            let pattern = args
                .get("pattern")
                .or_else(|| args.get("query"))
                .and_then(|v| v.as_str())
                .unwrap_or("");
            format!("Searching {}", pattern)
        }
        "file" | "read" | "file_read" | "read_file" => {
            let action = args.get("action").and_then(|v| v.as_str()).unwrap_or("read");
            let path = args
                .get("path")
                .or_else(|| args.get("file"))
                .and_then(|v| v.as_str())
                .unwrap_or("file");
            if action == "write" {
                format!("Writing {}", path)
            } else {
                format!("Reading {}", path)
            }
        }
        "write" | "file_write" | "write_file" => {
            let path = args
                .get("path")
                .or_else(|| args.get("file"))
                .and_then(|v| v.as_str())
                .unwrap_or("file");
            format!("Writing {}", path)
        }
        "edit" | "file_edit" | "edit_file" => {
            let path = args
                .get("path")
                .or_else(|| args.get("file"))
                .and_then(|v| v.as_str())
                .unwrap_or("file");
            format!("Editing {}", path)
        }
        "patch" | "apply_patch" => {
            let path = args
                .get("path")
                .or_else(|| args.get("file"))
                .and_then(|v| v.as_str())
                .unwrap_or("file");
            format!("Patching {}", path)
        }
        "bash" | "shell" | "process" => {
            let cmd = args
                .get("command")
                .or_else(|| args.get("cmd"))
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let single_line = cmd.replace('\n', " ").replace('\r', " ");
            let trimmed = single_line.trim();
            let preview = if trimmed.len() > 120 {
                let end = trimmed
                    .char_indices()
                    .map(|(i, _)| i)
                    .take(119)
                    .last()
                    .unwrap_or(trimmed.len());
                format!("{}…", &trimmed[..end])
            } else {
                trimmed.to_string()
            };
            format!("Running {}", preview)
        }
        "web_search" => {
            let q = args
                .get("query")
                .or_else(|| args.get("q"))
                .and_then(|v| v.as_str())
                .unwrap_or("");
            format!("Searching {}", q)
        }
        "fetch" | "web_fetch" => {
            let url = args.get("url").and_then(|v| v.as_str()).unwrap_or("");
            format!("Fetching {}", url)
        }
        other => format!("Using {}", other),
    }
}

/// Format an aggregated tool call tree string.
pub fn format_tool_tree(tool_batch: &[ToolCallItem]) -> String {
    if tool_batch.is_empty() {
        return String::new();
    }
    let total = tool_batch.len();
    let failed_count = tool_batch.iter().filter(|item| item.failed).count();
    let mut category_counts: std::collections::BTreeMap<&str, usize> =
        std::collections::BTreeMap::new();
    for item in tool_batch {
        *category_counts.entry(&item.category).or_insert(0) += 1;
    }
    let mut sorted_cats: Vec<_> = category_counts.into_iter().collect();
    sorted_cats.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(b.0)));
    let mut breakdown = Vec::new();
    for (cat, count) in sorted_cats {
        let display_cat = match (cat, count) {
            ("command", c) if c > 1 => "commands",
            (other, _) => other,
        };
        breakdown.push(format!("{} {}", count, display_cat));
    }
    if failed_count > 0 {
        breakdown.push(format!("{} failed", failed_count));
    }
    let breakdown_str = if breakdown.is_empty() {
        String::new()
    } else {
        format!(" · {}", breakdown.join(" · "))
    };

    let call_label = if total == 1 { "tool call" } else { "tool calls" };
    let mut out = format!("● {} {}{}\n", total, call_label, breakdown_str);
    for (i, item) in tool_batch.iter().enumerate() {
        let connector = if i == total - 1 { "└ " } else { "├ " };
        out.push_str(&format!("{}{}\n", connector, item.label));
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
    let failed_count = tool_batch.iter().filter(|item| item.failed).count();
    let mut category_counts: std::collections::BTreeMap<&str, usize> =
        std::collections::BTreeMap::new();
    for item in tool_batch {
        *category_counts.entry(&item.category).or_insert(0) += 1;
    }
    let mut sorted_cats: Vec<_> = category_counts.into_iter().collect();
    sorted_cats.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(b.0)));
    let mut breakdown = Vec::new();
    for (cat, count) in sorted_cats {
        let display_cat = match (cat, count) {
            ("command", c) if c > 1 => "commands",
            (other, _) => other,
        };
        breakdown.push(format!("{} {}", count, display_cat));
    }
    if failed_count > 0 {
        breakdown.push(format!("{} failed", failed_count));
    }
    let breakdown_str = if breakdown.is_empty() {
        String::new()
    } else {
        format!(" · {}", breakdown.join(" · "))
    };

    let call_label = if total == 1 { "tool call" } else { "tool calls" };
    write!(out, "\r\n\x1b[2;37m● {} {}{}\x1b[0m\r\n", total, call_label, breakdown_str)?;
    for (i, item) in tool_batch.iter().enumerate() {
        let connector = if i == total - 1 { "└ " } else { "├ " };
        write!(out, "\x1b[2;37m{}{}\x1b[0m\r\n", connector, item.label)?;
    }
    write!(out, "\r\n")?;
    out.flush()?;
    Ok(())
}

pub fn render_tool_tree(tool_batch: &[ToolCallItem]) {
    let _ = render_tool_tree_to(&mut stdout(), tool_batch);
}

fn parse_exit_code(output: &str) -> i32 {
    let lower = output.to_lowercase();
    if let Some(pos) = lower.find("exit code ") {
        let rest = &output[pos + 10..];
        if let Some(code) = rest
            .split_whitespace()
            .next()
            .and_then(|s| s.trim_matches(|c: char| !c.is_ascii_digit()).parse::<i32>().ok())
        {
            return code;
        }
    }
    if let Some(pos) = lower.find("exited ") {
        let rest = &output[pos + 7..];
        if let Some(code) = rest
            .split_whitespace()
            .next()
            .and_then(|s| s.trim_matches(|c: char| !c.is_ascii_digit()).parse::<i32>().ok())
        {
            return code;
        }
    }
    1
}

pub async fn run_turn_ui(
    runner: &AgentRunner,
    session: &mut Session,
    user_input: &str,
) -> anyhow::Result<(String, Option<String>)> {
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

    // Enter raw mode to allow interactive typing and Esc cancel without OS line-buffering
    let _raw_guard = crate::ui::prompt::RawModeGuard::enter().ok();
    let mut event_stream = EventStream::new();
    let mut queue_buffer: Vec<char> = Vec::new();
    let mut queue_cursor: usize = 0;
    let mut queued_prompt: Option<String> = None;
    let mut active_tool_label: Option<String> = None;

    let mut is_thinking = true;
    let mut thinking_displayed = false;
    let mut tool_batch: Vec<ToolCallItem> = Vec::new();
    let mut ticker = tokio::time::interval(std::time::Duration::from_millis(200));

    loop {
        tokio::select! {
            _ = ticker.tick(), if is_thinking => {
                let elapsed = start_time.elapsed();
                let mut out = stdout();
                if thinking_displayed {
                    clear_thinking_frame();
                }
                let queue_text: String = queue_buffer.iter().collect();
                let status_verb = match active_tool_label.as_deref() {
                    Some(label) => format!("• {}", label),
                    None => "• Running".to_string(),
                };
                let _ = render_thinking_frame_to(
                    &mut out,
                    &status_verb,
                    elapsed,
                    input_tokens,
                    output_tokens,
                    &model_label,
                    &queue_text,
                );
                let target_col = (2 + queue_cursor) as u16;
                let _ = execute!(out, cursor::MoveUp(2), cursor::MoveToColumn(target_col));
                let _ = out.flush();
                thinking_displayed = true;
            }
            Some(event_res) = event_stream.next() => {
                if let Ok(Event::Key(key)) = event_res {
                    if key.kind == crossterm::event::KeyEventKind::Press {
                        // Esc key or Ctrl+C immediately cancels running / thinking!
                        if key.code == KeyCode::Esc || (key.modifiers.contains(KeyModifiers::CONTROL) && (key.code == KeyCode::Char('c') || key.code == KeyCode::Char('C'))) {
                            if thinking_displayed {
                                clear_thinking_frame();
                                thinking_displayed = false;
                            }
                            let mut out = stdout();
                            if let Some(tool) = &active_tool_label {
                                let _ = write!(out, "■ Cancelled {} · What can fusion do differently?\r\n\r\n", tool);
                            } else {
                                let _ = write!(out, "  \x1b[2;37m(Turn canceled)\x1b[0m\r\n\r\n");
                            }
                            let _ = execute!(out, cursor::MoveToColumn(0));
                            let _ = out.flush();
                            return Ok((String::new(), None));
                        }

                        if is_thinking {
                            match key.code {
                                KeyCode::Char(c) => {
                                    queue_buffer.insert(queue_cursor, c);
                                    queue_cursor += 1;
                                    let mut out = stdout();
                                    if thinking_displayed {
                                        clear_thinking_frame();
                                    }
                                    let queue_text: String = queue_buffer.iter().collect();
                                    let elapsed = start_time.elapsed();
                                    let status_verb = match active_tool_label.as_deref() {
                                        Some(label) => format!("• {}", label),
                                        None => "• Running".to_string(),
                                    };
                                    let _ = render_thinking_frame_to(
                                        &mut out,
                                        &status_verb,
                                        elapsed,
                                        input_tokens,
                                        output_tokens,
                                        &model_label,
                                        &queue_text,
                                    );
                                    let target_col = (2 + queue_cursor) as u16;
                                    let _ = execute!(out, cursor::MoveUp(2), cursor::MoveToColumn(target_col));
                                    let _ = out.flush();
                                    thinking_displayed = true;
                                }
                                KeyCode::Backspace => {
                                    if queue_cursor > 0 {
                                        queue_buffer.remove(queue_cursor - 1);
                                        queue_cursor -= 1;
                                        let mut out = stdout();
                                        if thinking_displayed {
                                            clear_thinking_frame();
                                        }
                                        let queue_text: String = queue_buffer.iter().collect();
                                        let elapsed = start_time.elapsed();
                                        let status_verb = match active_tool_label.as_deref() {
                                            Some(label) => format!("• {}", label),
                                            None => "• Running".to_string(),
                                        };
                                        let _ = render_thinking_frame_to(
                                            &mut out,
                                            &status_verb,
                                            elapsed,
                                            input_tokens,
                                            output_tokens,
                                            &model_label,
                                            &queue_text,
                                        );
                                        let target_col = (2 + queue_cursor) as u16;
                                        let _ = execute!(out, cursor::MoveUp(2), cursor::MoveToColumn(target_col));
                                        let _ = out.flush();
                                        thinking_displayed = true;
                                    }
                                }
                                KeyCode::Left => {
                                    if queue_cursor > 0 {
                                        queue_cursor -= 1;
                                        let mut out = stdout();
                                        let _ = execute!(out, cursor::MoveToColumn((2 + queue_cursor) as u16));
                                        let _ = out.flush();
                                    }
                                }
                                KeyCode::Right => {
                                    if queue_cursor < queue_buffer.len() {
                                        queue_cursor += 1;
                                        let mut out = stdout();
                                        let _ = execute!(out, cursor::MoveToColumn((2 + queue_cursor) as u16));
                                        let _ = out.flush();
                                    }
                                }
                                KeyCode::Enter => {
                                    let text: String = queue_buffer.drain(..).collect();
                                    let trimmed = text.trim().to_string();
                                    queue_cursor = 0;
                                    if !trimmed.is_empty() {
                                        queued_prompt = Some(trimmed);
                                    }
                                    let mut out = stdout();
                                    if thinking_displayed {
                                        clear_thinking_frame();
                                    }
                                    let elapsed = start_time.elapsed();
                                    let status_verb = match active_tool_label.as_deref() {
                                        Some(label) => format!("• {}", label),
                                        None => "• Running".to_string(),
                                    };
                                    let _ = render_thinking_frame_to(
                                        &mut out,
                                        &status_verb,
                                        elapsed,
                                        input_tokens,
                                        output_tokens,
                                        &model_label,
                                        "",
                                    );
                                    let _ = execute!(out, cursor::MoveUp(2), cursor::MoveToColumn(2));
                                    let _ = out.flush();
                                    thinking_displayed = true;
                                }
                                _ => {}
                            }
                        }
                    }
                }
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
                        AgentEvent::ToolStarted { name, args, .. } => {
                            if thinking_displayed {
                                clear_thinking_frame();
                                thinking_displayed = false;
                            }
                            is_thinking = false;
                            md.finish();
                            let (label, category) = parse_tool_info(&name, &args);
                            tool_batch.push(ToolCallItem::new(name, label, category));
                        }
                        AgentEvent::ToolFinished { success, output, .. } => {
                            if let Some(item) = tool_batch.last_mut() {
                                if !success {
                                    item.failed = true;
                                    if item.category == "command" {
                                        let cmd = item.label.strip_prefix("Ran ").unwrap_or(&item.label);
                                        let code = parse_exit_code(&output);
                                        item.label = format!("Exited {} {}", code, cmd);
                                    } else if !item.label.starts_with("Failed ") {
                                        item.label = format!("Failed {}", item.label);
                                    }
                                }
                            }
                            if !tool_batch.is_empty() {
                                render_tool_tree(&tool_batch);
                                tool_batch.clear();
                            }
                            is_thinking = true;
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
                    thinking_displayed = false;
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
                let mut out = stdout();
                let _ = write!(out, "  \x1b[2;37m{} (↑{} ↓{})\x1b[0m\r\n\r\n", elapsed_str, in_str, out_str);
                let _ = execute!(out, cursor::MoveToColumn(0));
                let _ = out.flush();
                return res.map(|content| (content, queued_prompt));
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
                        tool_batch.push(ToolCallItem::new(name, label, category));
                    }
                    AgentEvent::ToolFinished { success, output, .. } => {
                        if let Some(item) = tool_batch.last_mut() {
                            if !success {
                                item.failed = true;
                                if item.category == "command" {
                                    let cmd = item.label.strip_prefix("Ran ").unwrap_or(&item.label);
                                    let code = parse_exit_code(&output);
                                    item.label = format!("Exited {} {}", code, cmd);
                                } else if !item.label.starts_with("Failed ") {
                                    item.label = format!("Failed {}", item.label);
                                }
                            }
                        }
                        if !tool_batch.is_empty() {
                            render_tool_tree(&tool_batch);
                            tool_batch.clear();
                        }
                        is_thinking = true;
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

/// Run the interactive lightweight inline REPL loop with optional resumed session.
pub async fn run_repl_with_session(
    mut runner: AgentRunner,
    initial_session: Option<Session>,
) -> anyhow::Result<()> {
    let mut session =
        initial_session.unwrap_or_else(|| Session::new(&runner.config().default_model));
    let mut prompt = Prompt::new()
        .with_model(&runner.config().default_model)
        .with_models(model_picker_list(&crate::provider::catalog::get_catalog()));

    // Clear terminal screen and place cursor at top left (clean startup like fx)
    let _ = execute!(
        stdout(),
        terminal::Clear(ClearType::All),
        cursor::MoveTo(0, 0)
    );
    let _ = write!(
        stdout(),
        "\x1b[2;37mfusion v{} · Run /help for commands\x1b[0m\r\n\r\n",
        env!("CARGO_PKG_VERSION")
    );
    let _ = stdout().flush();

    // Clear any leftover recovery crash state so we start cleanly with a blank prompt
    let _ = runner.recovery().clear();

    let mut pending_prompt: Option<String> = None;

    loop {
        let input = if let Some(queued) = pending_prompt.take() {
            print!("\x1b[1m┃ {}\x1b[0m\r\n\r\n", queued);
            let _ = stdout().flush();
            queued
        } else {
            prompt.set_model(&runner.config().default_model);
            match prompt.read_input() {
                Ok(PromptResult::Submit(input)) => input,
                Ok(PromptResult::Cancel) => {
                    println!("\x1b[2;37m(Turn canceled)\x1b[0m\r\n");
                    continue;
                }
                Ok(PromptResult::Exit) => {
                    match session.save() {
                        Ok(path) => {
                            let session_id = path
                                .file_stem()
                                .map(|s| s.to_string_lossy())
                                .unwrap_or_default();
                            println!(
                                "\x1b[2;37mContinue session with: fusion --resume {}\x1b[0m\r\n",
                                session_id
                            );
                        }
                        Err(_) => println!("\x1b[2;37mGoodbye!\x1b[0m\r\n"),
                    }
                    break;
                }
                Err(e) => {
                    eprintln!("Error reading input: {}", e);
                    break;
                }
            }
        };

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

        // Execute turn with live streaming and capture any queued prompt
        match run_turn_ui(&runner, &mut session, trimmed).await {
            Ok((_content, queued)) => {
                pending_prompt = queued;
            }
            Err(e) => {
                eprintln!("Error during turn: {}", e);
            }
        }
    }

    Ok(())
}

/// Run the interactive lightweight inline REPL loop.
pub async fn run_repl(runner: AgentRunner) -> anyhow::Result<()> {
    run_repl_with_session(runner, None).await
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
        assert_eq!(format_duration_compact(Duration::from_secs(67)), "1m7s");
        assert_eq!(format_duration_compact(Duration::from_secs(70)), "1m10s");
        assert_eq!(format_duration_compact(Duration::from_secs(3663)), "1h1m3s");
    }

    #[test]
    fn test_format_tool_duration() {
        assert_eq!(format_tool_duration(Duration::from_millis(450)), "450ms");
        assert_eq!(format_tool_duration(Duration::from_secs(5)), "5s");
        assert_eq!(format_tool_duration(Duration::from_secs(70)), "1m10s");
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
        let (label, cat) = parse_tool_info("skill", &serde_json::json!({ "name": "agents-sdk" }));
        assert_eq!(label, "Loaded skill agents-sdk");
        assert_eq!(cat, "read");

        let (label, cat) = parse_tool_info("load_skill", &serde_json::json!({ "skill": "browser-harness" }));
        assert_eq!(label, "Loaded skill browser-harness");
        assert_eq!(cat, "read");

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

        let (label, cat) = parse_tool_info("bash", &serde_json::json!({ "command": "which browser-use || python3 -m site --user-base" }));
        assert_eq!(label, "Ran which browser-use || python3 -m site --user-base");
        assert_eq!(cat, "command");
    }

    #[test]
    fn test_parse_tool_active_label() {
        assert_eq!(
            parse_tool_active_label("skill", &serde_json::json!({ "name": "agents-sdk" })),
            "Loading skill agents-sdk"
        );
        assert_eq!(
            parse_tool_active_label("bash", &serde_json::json!({ "command": "cargo test" })),
            "Running cargo test"
        );
        assert_eq!(
            parse_tool_active_label("glob", &serde_json::json!({ "pattern": "*.rs" })),
            "Matching *.rs"
        );
    }

    #[test]
    fn test_format_tool_tree() {
        let single_cmd = vec![ToolCallItem::new(
            "bash",
            "Ran which browser-use || python3 -m site --user-base",
            "command",
        )];
        let formatted = format_tool_tree(&single_cmd);
        assert_eq!(
            formatted,
            "● 1 tool call · 1 command\n└ Ran which browser-use || python3 -m site --user-base\n"
        );

        let failed_cmd = vec![ToolCallItem::new(
            "bash",
            "Exited 1 ls \"/Users/...\"",
            "command",
        )
        .with_failed(true)];
        let formatted_failed = format_tool_tree(&failed_cmd);
        assert_eq!(
            formatted_failed,
            "● 1 tool call · 1 command · 1 failed\n└ Exited 1 ls \"/Users/...\"\n"
        );

        let multi = vec![
            ToolCallItem::new("glob", "Matched **/*.md", "list"),
            ToolCallItem::new("file", "Read README.md", "read"),
        ];
        let formatted_multi = format_tool_tree(&multi);
        assert_eq!(
            formatted_multi,
            "● 2 tool calls · 1 list · 1 read\n├ Matched **/*.md\n└ Read README.md\n"
        );

        let multi_cmds = vec![
            ToolCallItem::new("bash", "Ran echo 1", "command"),
            ToolCallItem::new("bash", "Ran echo 2", "command"),
        ];
        let formatted_cmds = format_tool_tree(&multi_cmds);
        assert_eq!(
            formatted_cmds,
            "● 2 tool calls · 2 commands\n├ Ran echo 1\n└ Ran echo 2\n"
        );
    }

    #[test]
    fn test_format_activity_status() {
        let status = format_activity_status(
            "• Running",
            Duration::from_secs(70),
            9,
            641,
            "DeepSeek V4 Flash",
        );
        assert!(status.contains("• Running (1m10s) (↑9 ↓641)"));
        assert!(status.contains("┃"));
        assert!(status.contains("enter queue · auto · DeepSeek V4 Flash"));
    }
}
