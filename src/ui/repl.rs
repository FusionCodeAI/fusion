use crossterm::{
    cursor,
    event::{Event, EventStream, KeyCode, KeyEventKind},
    execute,
    terminal::{self, ClearType},
};
use futures::StreamExt;
use std::collections::VecDeque;
use std::io::{stdout, Write};
use std::time::Instant;

use crate::agent::{AgentEvent, AgentRunner, Session};
use crate::config::Config;
use crate::ui::markdown::MarkdownRenderer;
use crate::ui::prompt::{Prompt, PromptResult, SlashSuggestion};

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

/// Map a skill's source to a short display label for the picker tabs.
pub fn skill_source_label(source: &crate::agent::skills::SkillSource) -> &'static str {
    match source {
        crate::agent::skills::SkillSource::Project(p) => {
            if p.to_string_lossy().contains(".claude") {
                "Claude"
            } else {
                "Fusion"
            }
        }
        crate::agent::skills::SkillSource::Global(_) => "Global",
        crate::agent::skills::SkillSource::Custom(_) => "Custom",
        crate::agent::skills::SkillSource::Builtin => "Builtin",
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
    let frames = crate::ui::spinner::BRAILLE_FRAMES;
    let frame_idx = (elapsed.as_millis() / 80) as usize % frames.len();
    let dot = format!("{} ", frames[frame_idx]);
    let raw = if status_text.is_empty() {
        "Running"
    } else {
        status_text
    };
    let verb = raw.strip_prefix("• ").unwrap_or(raw);
    let status = format!("{}{}", dot, verb);
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
    format_activity_status("Running", elapsed, in_tokens, out_tokens, model_label)
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
    format!(
        "\r\n  \x1b[2;37m{} (↑{} ↓{})\x1b[0m\r\n\r\n",
        elapsed_str, in_str, out_str
    )
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
    queue_cursor: usize,
) -> std::io::Result<()> {
    let elapsed_str = format_duration_compact(elapsed);
    let in_str = format_tokens_compact(in_tokens);
    let out_str = format_tokens_compact(out_tokens);
    let frames = crate::ui::spinner::BRAILLE_FRAMES;
    let frame_idx = (elapsed.as_millis() / 80) as usize % frames.len();
    let dot = format!("{} ", frames[frame_idx]);
    let raw = if status_text.is_empty() {
        "Running"
    } else {
        status_text
    };
    let verb = raw.strip_prefix("• ").unwrap_or(raw);
    let status = format!("{}{}", dot, verb);
    write!(
        out,
        "  \x1b[2;37m{} ({}) (↑{} ↓{})\x1b[0m\r\n\r\n\x1b[1m┃\x1b[0m {}\r\n\r\n\x1b[2;37menter queue · auto · {}\x1b[0m",
        status, elapsed_str, in_str, out_str, queue_text, model_label
    )?;
    let col = (2 + queue_cursor) as u16;
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
    queue_cursor: usize,
) {
    let _ = render_thinking_frame_to(
        &mut stdout(),
        status_text,
        elapsed,
        in_tokens,
        out_tokens,
        model_label,
        queue_text,
        queue_cursor,
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
    pub fn new(
        name: impl Into<String>,
        label: impl Into<String>,
        category: impl Into<String>,
    ) -> Self {
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
                .or_else(|| args.get("path"))
                .and_then(|v| v.as_str())
                .unwrap_or("*");
            (format!("Matched {}", pattern), "list".to_string())
        }
        "grep" | "file_grep" | "grep_files" => {
            let query = args
                .get("query")
                .or_else(|| args.get("pattern"))
                .or_else(|| args.get("q"))
                .and_then(|v| v.as_str())
                .unwrap_or("");
            (format!("Searched {}", query), "read".to_string())
        }
        "read" | "file_read" | "read_file" => {
            let path = args
                .get("path")
                .or_else(|| args.get("file"))
                .or_else(|| args.get("filepath"))
                .and_then(|v| v.as_str())
                .unwrap_or("file");
            (format!("Read {}", path), "read".to_string())
        }
        "file" => {
            let action = args
                .get("action")
                .and_then(|v| v.as_str())
                .unwrap_or("read");
            let path = args
                .get("path")
                .or_else(|| args.get("file"))
                .or_else(|| args.get("filepath"))
                .and_then(|v| v.as_str())
                .unwrap_or("file");
            if action == "write" {
                (format!("Wrote {}", path), "write".to_string())
            } else if action == "edit" {
                (format!("Edited {}", path), "edit".to_string())
            } else {
                (format!("Read {}", path), "read".to_string())
            }
        }
        "write" | "file_write" | "write_file" => {
            let path = args
                .get("path")
                .or_else(|| args.get("file"))
                .or_else(|| args.get("filepath"))
                .and_then(|v| v.as_str())
                .unwrap_or("file");
            (format!("Wrote {}", path), "write".to_string())
        }
        "edit" | "file_edit" | "edit_file" => {
            let path = args
                .get("path")
                .or_else(|| args.get("file"))
                .or_else(|| args.get("filepath"))
                .and_then(|v| v.as_str())
                .unwrap_or("file");
            (format!("Edited {}", path), "edit".to_string())
        }
        "patch" | "apply_patch" => {
            let path = args
                .get("path")
                .or_else(|| args.get("file"))
                .or_else(|| args.get("filepath"))
                .and_then(|v| v.as_str())
                .unwrap_or("file");
            (format!("Patched {}", path), "edit".to_string())
        }
        "bash" | "shell" | "process" | "cmd" | "command" | "exec" => {
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
        "tree" | "list" | "list_dir" | "dir_list" => {
            let path = args
                .get("path")
                .or_else(|| args.get("dir"))
                .or_else(|| args.get("directory"))
                .and_then(|v| v.as_str())
                .unwrap_or(".");
            (format!("Listed {}", path), "list".to_string())
        }
        "web_search" | "search" => {
            let q = args
                .get("query")
                .or_else(|| args.get("q"))
                .or_else(|| args.get("pattern"))
                .and_then(|v| v.as_str())
                .unwrap_or("");
            (format!("Searched {}", q), "read".to_string())
        }
        "fetch" | "web_fetch" => {
            let url = args
                .get("url")
                .or_else(|| args.get("uri"))
                .and_then(|v| v.as_str())
                .unwrap_or("");
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
                .or_else(|| args.get("path"))
                .and_then(|v| v.as_str())
                .unwrap_or("*");
            format!("Matching {}", pattern)
        }
        "grep" | "file_grep" | "grep_files" => {
            let pattern = args
                .get("query")
                .or_else(|| args.get("pattern"))
                .or_else(|| args.get("q"))
                .and_then(|v| v.as_str())
                .unwrap_or("");
            format!("Searching {}", pattern)
        }
        "read" | "file_read" | "read_file" => {
            let path = args
                .get("path")
                .or_else(|| args.get("file"))
                .or_else(|| args.get("filepath"))
                .and_then(|v| v.as_str())
                .unwrap_or("file");
            format!("Reading {}", path)
        }
        "file" => {
            let action = args
                .get("action")
                .and_then(|v| v.as_str())
                .unwrap_or("read");
            let path = args
                .get("path")
                .or_else(|| args.get("file"))
                .or_else(|| args.get("filepath"))
                .and_then(|v| v.as_str())
                .unwrap_or("file");
            if action == "write" {
                format!("Writing {}", path)
            } else if action == "edit" {
                format!("Editing {}", path)
            } else {
                format!("Reading {}", path)
            }
        }
        "write" | "file_write" | "write_file" => {
            let path = args
                .get("path")
                .or_else(|| args.get("file"))
                .or_else(|| args.get("filepath"))
                .and_then(|v| v.as_str())
                .unwrap_or("file");
            format!("Writing {}", path)
        }
        "edit" | "file_edit" | "edit_file" => {
            let path = args
                .get("path")
                .or_else(|| args.get("file"))
                .or_else(|| args.get("filepath"))
                .and_then(|v| v.as_str())
                .unwrap_or("file");
            format!("Editing {}", path)
        }
        "patch" | "apply_patch" => {
            let path = args
                .get("path")
                .or_else(|| args.get("file"))
                .or_else(|| args.get("filepath"))
                .and_then(|v| v.as_str())
                .unwrap_or("file");
            format!("Patching {}", path)
        }
        "bash" | "shell" | "process" | "cmd" | "command" | "exec" => {
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
        "tree" | "list" | "list_dir" | "dir_list" => {
            let path = args
                .get("path")
                .or_else(|| args.get("dir"))
                .or_else(|| args.get("directory"))
                .and_then(|v| v.as_str())
                .unwrap_or(".");
            format!("Listing {}", path)
        }
        "web_search" | "search" => {
            let q = args
                .get("query")
                .or_else(|| args.get("q"))
                .or_else(|| args.get("pattern"))
                .and_then(|v| v.as_str())
                .unwrap_or("");
            format!("Searching {}", q)
        }
        "fetch" | "web_fetch" => {
            let url = args
                .get("url")
                .or_else(|| args.get("uri"))
                .and_then(|v| v.as_str())
                .unwrap_or("");
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

    let call_label = if total == 1 {
        "tool call"
    } else {
        "tool calls"
    };
    let mut out = format!("● {} {}{}\n", total, call_label, breakdown_str);
    for (i, item) in tool_batch.iter().enumerate() {
        let connector = if i == total - 1 { "└ " } else { "├ " };
        let display_label = if item.failed
            && !item.label.starts_with("Failed ")
            && !item.label.starts_with("Exited ")
        {
            if item.category == "command" {
                let cmd = item.label.strip_prefix("Ran ").unwrap_or(&item.label);
                format!("Exited 1 {}", cmd)
            } else {
                format!("Failed {}", item.label)
            }
        } else {
            item.label.clone()
        };
        out.push_str(&format!("{}{}\n", connector, display_label));
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
pub fn render_tool_tree_to<W: std::io::Write>(
    out: &mut W,
    tool_batch: &[ToolCallItem],
) -> std::io::Result<()> {
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
    write!(out, "\r\n")?;
    let call_label = if total == 1 {
        "tool call"
    } else {
        "tool calls"
    };
    write!(
        out,
        "\x1b[2;37m● {} {}{}\x1b[0m\r\n",
        total, call_label, breakdown_str
    )?;
    for (i, item) in tool_batch.iter().enumerate() {
        let connector = if i == total - 1 { "└ " } else { "├ " };
        let display_label = if item.failed
            && !item.label.starts_with("Failed ")
            && !item.label.starts_with("Exited ")
        {
            if item.category == "command" {
                let cmd = item.label.strip_prefix("Ran ").unwrap_or(&item.label);
                format!("Exited 1 {}", cmd)
            } else {
                format!("Failed {}", item.label)
            }
        } else {
            item.label.clone()
        };
        if item.failed {
            write!(
                out,
                "\x1b[2;37m{}\x1b[31m{}\x1b[0m\r\n",
                connector, display_label
            )?;
        } else {
            write!(out, "\x1b[2;37m{}{}\x1b[0m\r\n", connector, display_label)?;
        }
    }
    write!(out, "\r\n")?;
    out.flush()?;
    Ok(())
}

pub fn render_tool_tree(tool_batch: &[ToolCallItem]) {
    let mut out = stdout();
    let _ = write!(out, "\r\x1b[2K");
    let _ = render_tool_tree_to(&mut out, tool_batch);
}

fn parse_exit_code(output: &str) -> i32 {
    let lower = output.to_lowercase();
    if let Some(pos) = lower.find("exit code ") {
        let rest = &output[pos + 10..];
        if let Some(code) = rest.split_whitespace().next().and_then(|s| {
            s.trim_matches(|c: char| !c.is_ascii_digit())
                .parse::<i32>()
                .ok()
        }) {
            return code;
        }
    }
    if let Some(pos) = lower.find("exited ") {
        let rest = &output[pos + 7..];
        if let Some(code) = rest.split_whitespace().next().and_then(|s| {
            s.trim_matches(|c: char| !c.is_ascii_digit())
                .parse::<i32>()
                .ok()
        }) {
            return code;
        }
    }
    1
}
#[inline]
pub(crate) fn reset_prompt_render_state(prompt: &mut Prompt) {
    prompt.reset_render_state();
}

#[inline]
pub(crate) fn clear_prompt_frame(prompt: &mut Prompt) {
    if prompt.last_rendered_lines > 0 || prompt.last_cursor_row > 0 {
        let _ = prompt.clear_frame();
    }
    prompt.reset_render_state();
}

pub async fn run_turn_ui(
    runner: &AgentRunner,
    session: &mut Session,
    user_input: &str,
    prompt: &mut Prompt,
) -> anyhow::Result<(String, VecDeque<String>)> {
    let start_time = Instant::now();
    let mut input_tokens = (crate::agent::tokens::estimate_text_tokens(user_input)
        + session.estimate_tokens())
    .max(1) as u64;
    let mut output_tokens = 0u64;
    let mut md = MarkdownRenderer::new().with_indent(2);

    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    let runner_task = runner.run_turn_stream(session, user_input, tx);
    tokio::pin!(runner_task);

    // Enter raw mode to allow interactive typing, slash suggestions, and Esc cancel
    let _raw_guard = crate::ui::prompt::RawModeGuard::enter().ok();
    let mut event_stream = EventStream::new();
    let mut queued_prompts: VecDeque<String> = VecDeque::new();
    let mut active_tool_label: Option<String> = None;

    let mut is_thinking = true;
    let mut tool_batch: Vec<ToolCallItem> = Vec::new();
    let mut ticker = tokio::time::interval(std::time::Duration::from_millis(300));

    clear_prompt_frame(prompt);
    prompt.reset_input();
    reset_prompt_render_state(prompt);
    prompt.set_running(true);

    let mut handle_agent_event = |event: AgentEvent,
                                  tool_batch: &mut Vec<ToolCallItem>,
                                  active_tool_label: &mut Option<String>,
                                  is_thinking: &mut bool,
                                  output_tokens: &mut u64,
                                  input_tokens: &mut u64,
                                  md: &mut MarkdownRenderer,
                                  prompt: &mut Prompt| {
        match event {
            AgentEvent::TextDelta(d) => {
                clear_prompt_frame(prompt);
                if *is_thinking {
                    *is_thinking = false;
                    *active_tool_label = None;
                    let mut out = stdout();
                    let _ = write!(out, "\r\x1b[2K");
                    let _ = out.flush();
                }
                if !tool_batch.is_empty() {
                    prompt.set_running_status(None);
                    let mut out = stdout();
                    let _ = write!(out, "\r\x1b[2K");
                    let _ = out.flush();
                    render_tool_tree(tool_batch);
                    tool_batch.clear();
                    reset_prompt_render_state(prompt);
                }
                *output_tokens += crate::agent::tokens::estimate_text_tokens(&d) as u64;
                md.push(&d);
                reset_prompt_render_state(prompt);
                let _ = prompt.render_current();
            }
            AgentEvent::ThinkingDelta(th) => {
                if !tool_batch.is_empty() {
                    clear_prompt_frame(prompt);
                    prompt.set_running_status(None);
                    let mut out = stdout();
                    let _ = write!(out, "\r\x1b[2K");
                    let _ = out.flush();
                    render_tool_tree(tool_batch);
                    tool_batch.clear();
                    reset_prompt_render_state(prompt);
                    let _ = prompt.render_current();
                }
                *output_tokens += crate::agent::tokens::estimate_text_tokens(&th) as u64;
            }
            AgentEvent::ToolStarted { name, args, .. } => {
                clear_prompt_frame(prompt);
                let mut out = stdout();
                let _ = write!(out, "\r\x1b[2K");
                let _ = out.flush();
                let active_label = parse_tool_active_label(&name, &args);
                let (completed_label, category) = parse_tool_info(&name, &args);
                *active_tool_label = Some(active_label.clone());
                tool_batch.push(ToolCallItem::new(name, completed_label, category));
                md.finish();
                reset_prompt_render_state(prompt);
                *is_thinking = true;
                let elapsed = start_time.elapsed();
                let frames = crate::ui::spinner::BRAILLE_FRAMES;
                let frame_idx = (elapsed.as_millis() / 80) as usize % frames.len();
                let dot = format!("{} ", frames[frame_idx]);
                let verb = active_label.strip_prefix("• ").unwrap_or(&active_label);
                let status = format!(
                    "\r\x1b[2K{}{} ({}) (↑{} ↓{})",
                    dot,
                    verb,
                    format_duration_compact(elapsed),
                    format_tokens_compact(*input_tokens),
                    format_tokens_compact(*output_tokens)
                );
                prompt.set_running_status(Some(status));
                let _ = prompt.render_current();
            }
            AgentEvent::ToolFinished {
                success, output, ..
            } => {
                clear_prompt_frame(prompt);
                let mut out = stdout();
                let _ = write!(out, "\r\x1b[2K");
                let _ = out.flush();
                *active_tool_label = None;
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
                prompt.set_running_status(None);
                *is_thinking = true;
                reset_prompt_render_state(prompt);
                let _ = prompt.render_current();
            }
            AgentEvent::Status(msg) => {
                clear_prompt_frame(prompt);
                if msg.contains("Waiting for model response") || !tool_batch.is_empty() {
                    if !tool_batch.is_empty() {
                        prompt.set_running_status(None);
                        let mut out = stdout();
                        let _ = write!(out, "\r\x1b[2K");
                        let _ = out.flush();
                        render_tool_tree(tool_batch);
                        tool_batch.clear();
                        reset_prompt_render_state(prompt);
                    }
                }
                reset_prompt_render_state(prompt);
                let _ = prompt.render_current();
            }
            AgentEvent::Error(err) => {
                clear_prompt_frame(prompt);
                prompt.set_running(false);
                prompt.set_running_status(None);
                let mut out = stdout();
                let _ = write!(out, "\r\x1b[2K\r\n\x1b[31m❌ Error: {}\x1b[0m\r\n\r\n", err);
                let _ = execute!(out, cursor::MoveToColumn(0));
                let _ = out.flush();
                reset_prompt_render_state(prompt);
            }
            AgentEvent::Finished { usage } => {
                if let Some(u) = &usage {
                    if let Some(pt) = u.get("prompt_tokens").and_then(|v| v.as_u64()) {
                        *input_tokens = pt;
                    }
                    if let Some(ct) = u.get("completion_tokens").and_then(|v| v.as_u64()) {
                        *output_tokens = ct;
                    }
                }
                clear_prompt_frame(prompt);
                prompt.set_running(false);
                prompt.set_running_status(None);
                let mut out = stdout();
                let _ = write!(out, "\r\x1b[2K");
                let _ = out.flush();
                if !tool_batch.is_empty() {
                    render_tool_tree(tool_batch);
                    tool_batch.clear();
                    reset_prompt_render_state(prompt);
                }
                md.finish();
                reset_prompt_render_state(prompt);
            }
            _ => {}
        }
    };

    let mut last_status_secs: Option<u64> = None;
    let mut last_status_dot_frame: Option<usize> = None;

    loop {
        tokio::select! {
            _ = ticker.tick() => {
                if !prompt.is_running() {
                    continue;
                }
                let elapsed = start_time.elapsed();
                let current_secs = elapsed.as_secs();
                let frames = crate::ui::spinner::BRAILLE_FRAMES;
                let frame_idx = (elapsed.as_millis() / 80) as usize % frames.len();
                let dot_frame = frame_idx;

                // Debounce: only re-render if elapsed second changed or spinner frame changed
                if prompt.running_status.is_some()
                    && last_status_secs == Some(current_secs)
                    && last_status_dot_frame == Some(dot_frame)
                {
                    continue;
                }
                last_status_secs = Some(current_secs);
                last_status_dot_frame = Some(dot_frame);

                let dot = format!("{} ", frames[frame_idx]);
                let verb = if is_thinking {
                    match active_tool_label.as_deref() {
                        Some(label) => label.strip_prefix("• ").unwrap_or(label),
                        None => "Running",
                    }
                } else {
                    "Streaming"
                };
                let status = format!(
                    "\r\x1b[2K{}{} ({}) (↑{} ↓{})",
                    dot,
                    verb,
                    format_duration_compact(elapsed),
                    format_tokens_compact(input_tokens),
                    format_tokens_compact(output_tokens)
                );
                clear_prompt_frame(prompt);
                prompt.set_running_status(Some(status));
                reset_prompt_render_state(prompt);
                let _ = prompt.render_current();
            }
            Some(event_res) = event_stream.next() => {
                if let Ok(ev) = event_res {
                    if let Event::Key(key) = ev {
                        if key.kind != KeyEventKind::Release
                            && key.code == KeyCode::Up
                            && prompt.buffer.is_empty()
                        {
                            if let Some(last) = queued_prompts.pop_back() {
                                prompt.buffer = last.chars().collect();
                                prompt.cursor_pos = prompt.buffer.len();
                                prompt.set_queued_count(queued_prompts.len());
                                let _ = prompt.render_current();
                                continue;
                            }
                        }
                    }
                    if let Some(res) = prompt.handle_event(ev)? {
                        match res {
                            PromptResult::Submit(text) => {
                                let trimmed = text.trim().to_string();
                                if !trimmed.is_empty() {
                                    queued_prompts.push_back(trimmed);
                                }
                                prompt.set_queued_count(queued_prompts.len());
                                prompt.buffer.clear();
                                prompt.cursor_pos = 0;
                                let _ = prompt.render_current();
                            }
                            PromptResult::Cancel => {
                                clear_prompt_frame(prompt);
                                prompt.set_running(false);
                                prompt.set_running_status(None);
                                prompt.set_queued_count(0);
                                let mut out = stdout();
                                if let Some(tool) = &active_tool_label {
                                    let _ = write!(out, "■ Cancelled {} · What can fusion do differently?\r\n\r\n", tool);
                                } else {
                                    let _ = write!(out, "  \x1b[2;37m(Turn canceled)\x1b[0m\r\n\r\n");
                                }
                                let _ = execute!(out, cursor::MoveToColumn(0));
                                let _ = out.flush();
                                reset_prompt_render_state(prompt);
                                return Ok((String::new(), queued_prompts));
                            }
                            PromptResult::Exit => {
                                clear_prompt_frame(prompt);
                                prompt.set_running(false);
                                prompt.set_running_status(None);
                                prompt.set_queued_count(0);
                                let _ = stdout().flush();
                                reset_prompt_render_state(prompt);
                                return Ok((String::new(), queued_prompts));
                            }
                        }
                    }
                }
            }
            res = &mut runner_task => {
                while let Ok(event) = rx.try_recv() {
                    handle_agent_event(
                        event,
                        &mut tool_batch,
                        &mut active_tool_label,
                        &mut is_thinking,
                        &mut output_tokens,
                        &mut input_tokens,
                        &mut md,
                        prompt,
                    );
                }
                clear_prompt_frame(prompt);
                prompt.set_running(false);
                prompt.set_running_status(None);
                prompt.set_queued_count(0);
                if !tool_batch.is_empty() {
                    render_tool_tree(&tool_batch);
                    tool_batch.clear();
                    reset_prompt_render_state(prompt);
                }
                md.finish();
                reset_prompt_render_state(prompt);

                let elapsed = start_time.elapsed();
                let elapsed_str = format_duration_compact(elapsed);
                let in_str = format_tokens_compact(input_tokens);
                let out_str = format_tokens_compact(output_tokens);
                let mut out = stdout();
                let _ = write!(out, "\r\n  \x1b[2;37m{} (↑{} ↓{})\x1b[0m\r\n\r\n", elapsed_str, in_str, out_str);
                let _ = execute!(out, cursor::MoveToColumn(0));
                let _ = out.flush();
                reset_prompt_render_state(prompt);
                return res.map(|content| (content, queued_prompts));
            }
            Some(event) = rx.recv() => {
                handle_agent_event(
                    event,
                    &mut tool_batch,
                    &mut active_tool_label,
                    &mut is_thinking,
                    &mut output_tokens,
                    &mut input_tokens,
                    &mut md,
                    prompt,
                );
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
    let skill_suggestions: Vec<SlashSuggestion> = runner
        .skills()
        .list()
        .iter()
        .map(|s| SlashSuggestion {
            name: s.name().to_string(),
            description: s.description().to_string(),
            category: "Skill".to_string(),
            is_skill: true,
            source: skill_source_label(&s.source).to_string(),
        })
        .collect();
    let mut prompt = Prompt::new()
        .with_model(session.active_model())
        .with_models(model_picker_list(&crate::provider::catalog::get_catalog()))
        .with_skill_suggestions(skill_suggestions);

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

    let mut prompt_queue: VecDeque<String> = VecDeque::new();
    let mut last_cancel_time: Option<Instant> = None;

    loop {
        // Model name sync: ensure switching to MiniMax or any model persists in prompt, runner.config, and session
        prompt.set_model(session.active_model());
        runner.config_mut().default_model = session.active_model().to_string();

        let input = if let Some(next_prompt) = prompt_queue.pop_front() {
            let trimmed = next_prompt.trim().to_string();
            if trimmed.is_empty() {
                continue;
            }
            clear_prompt_frame(&mut prompt);
            let mut out = stdout();
            let _ = write!(out, "\x1b[1m┃ {}\x1b[0m\r\n\r\n", trimmed);
            let _ = execute!(out, cursor::MoveToColumn(0));
            let _ = out.flush();
            reset_prompt_render_state(&mut prompt);
            trimmed
        } else {
            prompt.set_model(session.active_model());
            clear_prompt_frame(&mut prompt);
            let mut out = stdout();
            let _ = execute!(out, cursor::MoveToColumn(0));
            let _ = out.flush();
            prompt.reset_render_state();
            match prompt.read_input() {
                Ok(PromptResult::Submit(input)) => {
                    last_cancel_time = None;
                    input
                }
                Ok(PromptResult::Cancel) => {
                    let now = Instant::now();
                    if let Some(prev) = last_cancel_time {
                        if now.duration_since(prev) <= std::time::Duration::from_secs(2) {
                            println!("\x1b[2;37mGoodbye!\x1b[0m\r\n");
                            break;
                        }
                    }
                    last_cancel_time = Some(now);
                    println!("\x1b[2;37m(Turn canceled - press Ctrl+C again to exit)\x1b[0m\r\n");
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

        // Enable the skill selected from the skill picker, if any
        if let Some((skill_name, _)) = prompt.take_active_skill() {
            if runner.skills_mut().set_enabled(&skill_name, true) {
                println!("\x1b[2;37m✓ \x1b[1;36m{}\x1b[0m enabled\r\n", skill_name);
            }
        }

        // Handle slash commands
        if trimmed.starts_with('/') {
            if handle_command(trimmed, &mut runner, &mut session) {
                break;
            }
            prompt.set_model(session.active_model());
            runner.config_mut().default_model = session.active_model().to_string();
            // Refresh skill suggestions (e.g. after /skills reload/enable/disable)
            let refreshed: Vec<SlashSuggestion> = runner
                .skills()
                .list()
                .iter()
                .map(|s| SlashSuggestion {
                    name: s.name().to_string(),
                    description: s.description().to_string(),
                    category: "Skill".to_string(),
                    is_skill: true,
                    source: skill_source_label(&s.source).to_string(),
                })
                .collect();
            prompt.set_skill_suggestions(refreshed);
            clear_prompt_frame(&mut prompt);
            continue;
        }

        let turn_start = Instant::now();
        // Execute turn with live streaming and capture any queued prompt
        match run_turn_ui(&runner, &mut session, trimmed, &mut prompt).await {
            Ok((_content, queued)) => {
                prompt_queue.extend(queued);
                let turn_elapsed = turn_start.elapsed();
                crate::ui::notify::notify_turn_complete(
                    runner.config(),
                    trimmed,
                    session.active_model(),
                    Some(turn_elapsed.as_secs_f64()),
                );
            }
            Err(e) => {
                clear_prompt_frame(&mut prompt);
                eprintln!("Error during turn: {}\r\n", e);
                let _ = execute!(stdout(), cursor::MoveToColumn(0));
                let _ = stdout().flush();
                reset_prompt_render_state(&mut prompt);
            }
        }
        clear_prompt_frame(&mut prompt);
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

        let (label, cat) = parse_tool_info(
            "load_skill",
            &serde_json::json!({ "skill": "browser-harness" }),
        );
        assert_eq!(label, "Loaded skill browser-harness");
        assert_eq!(cat, "read");

        let (label, cat) = parse_tool_info("glob", &serde_json::json!({ "pattern": "**/*.rs" }));
        assert_eq!(label, "Matched **/*.rs");
        assert_eq!(cat, "list");

        let (label, cat) = parse_tool_info("grep", &serde_json::json!({ "pattern": "mermaid" }));
        assert_eq!(label, "Searched mermaid");
        assert_eq!(cat, "read");

        let (label, cat) = parse_tool_info(
            "file",
            &serde_json::json!({ "action": "read", "path": "README.md" }),
        );
        assert_eq!(label, "Read README.md");
        assert_eq!(cat, "read");

        let (label, cat) = parse_tool_info(
            "file",
            &serde_json::json!({ "action": "write", "path": "test.txt" }),
        );
        assert_eq!(label, "Wrote test.txt");
        assert_eq!(cat, "write");

        let (label, cat) = parse_tool_info("edit", &serde_json::json!({ "path": "src/main.rs" }));
        assert_eq!(label, "Edited src/main.rs");
        assert_eq!(cat, "edit");

        let (label, cat) = parse_tool_info(
            "bash",
            &serde_json::json!({ "command": "which browser-use || python3 -m site --user-base" }),
        );
        assert_eq!(
            label,
            "Ran which browser-use || python3 -m site --user-base"
        );
        assert_eq!(cat, "command");

        let (label, cat) = parse_tool_info("tree", &serde_json::json!({ "path": "src/ui" }));
        assert_eq!(label, "Listed src/ui");
        assert_eq!(cat, "list");

        let (label, cat) =
            parse_tool_info("web_search", &serde_json::json!({ "query": "rust async" }));
        assert_eq!(label, "Searched rust async");
        assert_eq!(cat, "read");

        let (label, cat) = parse_tool_info(
            "fetch",
            &serde_json::json!({ "url": "https://example.com" }),
        );
        assert_eq!(label, "Fetched https://example.com");
        assert_eq!(cat, "read");
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

        let failed_cmd = vec![
            ToolCallItem::new("bash", "Exited 1 ls \"/Users/...\"", "command").with_failed(true),
        ];
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

        let batch_4 = vec![
            ToolCallItem::new(
                "glob",
                "Matched **/*.{md,mmd,puml,dot,svg,drawio,excalidraw}",
                "list",
            ),
            ToolCallItem::new("glob", "Matched **/*diagram*", "list"),
            ToolCallItem::new("grep", "Searched mermaid", "read"),
            ToolCallItem::new("glob", "Matched README*", "list"),
        ];
        let formatted_4 = format_tool_tree(&batch_4);
        assert_eq!(
            formatted_4,
            "● 4 tool calls · 3 list · 1 read\n├ Matched **/*.{md,mmd,puml,dot,svg,drawio,excalidraw}\n├ Matched **/*diagram*\n├ Searched mermaid\n└ Matched README*\n"
        );

        let batch_8 = vec![
            ToolCallItem::new("file", "Read docs/architecture.md", "read"),
            ToolCallItem::new("file", "Read README.md", "read"),
            ToolCallItem::new("grep", "Searched ```mermaid", "read"),
            ToolCallItem::new("glob", "Matched docs/**/*", "list"),
            ToolCallItem::new("grep", "Searched graph TD", "read"),
            ToolCallItem::new("file", "Read docs/agents.md", "read"),
            ToolCallItem::new("file", "Read docs/vision.md", "read"),
            ToolCallItem::new("grep", "Searched ```text", "read"),
        ];
        let formatted_8 = format_tool_tree(&batch_8);
        assert_eq!(
            formatted_8,
            "● 8 tool calls · 7 read · 1 list\n├ Read docs/architecture.md\n├ Read README.md\n├ Searched ```mermaid\n├ Matched docs/**/*\n├ Searched graph TD\n├ Read docs/agents.md\n├ Read docs/vision.md\n└ Searched ```text\n"
        );
    }

    #[test]
    fn test_render_tool_tree_ansi_colors() {
        let mut buf = Vec::new();
        let items = vec![
            ToolCallItem::new("bash", "Ran echo hello", "command"),
            ToolCallItem::new("bash", "Ran false", "command").with_failed(true),
        ];
        render_tool_tree_to(&mut buf, &items).unwrap();
        let s = String::from_utf8_lossy(&buf);
        assert!(s.contains("● 2 tool calls · 2 commands · 1 failed"));
        assert!(s.contains("├ Ran echo hello"));
        assert!(s.contains("\x1b[31mExited 1 false\x1b[0m"));
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
        assert!(!status.contains("• •"));
        assert!(status.contains("┃"));
        assert!(status.contains("enter queue · auto · DeepSeek V4 Flash"));
    }

    #[test]
    fn test_format_activity_status_blinking() {
        let status_on = format_activity_status("Running", Duration::from_millis(0), 0, 0, "auto");
        assert!(status_on.contains("• Running"));

        let status_off =
            format_activity_status("Running", Duration::from_millis(500), 0, 0, "auto");
        assert!(status_off.contains("  Running"));
        assert!(!status_off.contains("• Running"));

        let status_on_2 =
            format_activity_status("• Running", Duration::from_millis(1000), 0, 0, "auto");
        assert!(status_on_2.contains("• Running"));
        assert!(!status_on_2.contains("• •"));

        let status_off_2 =
            format_activity_status("• Running", Duration::from_millis(1500), 0, 0, "auto");
        assert!(status_off_2.contains("  Running"));
        assert!(!status_off_2.contains("• •"));
    }

    #[test]
    fn test_prompt_queue_fifo_and_recall() {
        let mut queued_prompts: VecDeque<String> = VecDeque::new();
        let mut prompt = Prompt::new();

        // Enqueue two prompts
        queued_prompts.push_back("first message".to_string());
        queued_prompts.push_back("second message".to_string());
        prompt.set_queued_count(queued_prompts.len());
        assert_eq!(prompt.queued_count(), 2);

        // Recall last queued prompt on empty buffer via Up arrow logic
        assert!(prompt.buffer.is_empty());
        if let Some(last) = queued_prompts.pop_back() {
            prompt.buffer = last.chars().collect();
            prompt.cursor_pos = prompt.buffer.len();
            prompt.set_queued_count(queued_prompts.len());
        }
        assert_eq!(prompt.buffer.iter().collect::<String>(), "second message");
        assert_eq!(prompt.cursor_pos, 14);
        assert_eq!(prompt.queued_count(), 1);
        assert_eq!(queued_prompts.len(), 1);

        // FIFO pop of remaining queue
        assert_eq!(
            queued_prompts.pop_front(),
            Some("first message".to_string())
        );
        assert!(queued_prompts.is_empty());
    }

    #[test]
    fn test_model_sync_persistence() {
        let mut session = Session::new("minimax-text-01");
        let mut prompt = Prompt::new().with_model(session.active_model());
        let mut config = Config::default();
        config.default_model = session.active_model().to_string();

        assert_eq!(session.active_model(), "minimax-text-01");
        assert_eq!(prompt.active_model(), "minimax-text-01");
        assert_eq!(config.default_model, "minimax-text-01");

        // Switch model in session
        session.set_active_model("grok-4.6");
        prompt.set_model(session.active_model());
        config.default_model = session.active_model().to_string();

        assert_eq!(session.active_model(), "grok-4.6");
        assert_eq!(prompt.active_model(), "grok-4.6");
        assert_eq!(config.default_model, "grok-4.6");
    }

    #[test]
    fn test_double_ctrl_c_timing() {
        let mut last_cancel: Option<Instant> = None;
        let t0 = Instant::now();
        last_cancel = Some(t0);

        // Within 2 seconds -> should trigger exit
        let t1 = t0 + Duration::from_millis(500);
        assert!(t1.duration_since(last_cancel.unwrap()) <= Duration::from_secs(2));

        // After 3 seconds -> should not trigger exit
        let t2 = t0 + Duration::from_secs(3);
        assert!(t2.duration_since(last_cancel.unwrap()) > Duration::from_secs(2));
    }

    #[test]
    fn test_reset_prompt_render_state() {
        let mut prompt = Prompt::new();
        prompt.last_rendered_lines = 5;
        prompt.last_cursor_row = 2;
        reset_prompt_render_state(&mut prompt);
        assert_eq!(prompt.last_rendered_lines, 0);
        assert_eq!(prompt.last_cursor_row, 0);
    }

    #[test]
    fn test_clear_prompt_frame_state() {
        let mut prompt = Prompt::new();
        prompt.last_rendered_lines = 0;
        prompt.last_cursor_row = 3;
        clear_prompt_frame(&mut prompt);
        assert_eq!(prompt.last_rendered_lines, 0);
        assert_eq!(prompt.last_cursor_row, 0);
    }
}
