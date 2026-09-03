//! Integration tests for Fusion's FX-style minimal Terminal UI contracts.
//!
//! Verifies:
//! 1. Startup screen clearing and cursor reset to top-left.
//! 2. Prompt vertical rail symbol (`┃`) and status line (`auto · <model>`).
//! 3. Submitted user prompt rendering (`┃ <user text>` followed by a blank line).
//! 4. Real-time thinking status formatting (`Thinking ({time}) (↑{in} ↓{out})`).
//! 5. Active queue prompt hint format (`enter queue · auto · <model>`).
//! 6. Tool call tree aggregation with `●`, `├─`, `└─`, category breakdown, and clean action labels.
//! 7. Completed turn summary format (`  {time} (↑{in} ↓{out})`).
//! 8. Markdown 2-space indentation and code block dividers.
//! 9. All tests execute completely in-memory without requiring a physical PTY/TTY.

use std::time::Duration;
use crossterm::{cursor, execute, terminal::{self, ClearType}};
use serde_json::json;

use fusion::ui::{
    format_model_label, format_repl_duration_compact, format_repl_tokens_compact,
    format_thinking_status, format_tool_tree, format_turn_summary,
    parse_tool_info, render_tool_tree_to, strip_ansi, MarkdownRenderer,
    Prompt, ToolCallItem,
};

// ===========================================================================
// Contract 1: Screen clearing and cursor reset on startup
// ===========================================================================

#[test]
fn test_startup_screen_clearing_and_cursor_reset() {
    let mut buf = Vec::new();
    let res = execute!(
        buf,
        terminal::Clear(ClearType::All),
        cursor::MoveTo(0, 0)
    );
    assert!(res.is_ok(), "Startup execute! sequence should succeed on in-memory buffer");

    let output = String::from_utf8_lossy(&buf);
    // Crossterm Clear(ClearType::All) emits "\x1b[2J" and MoveTo(0,0) emits "\x1b[1;1H" or "\x1b[H"
    assert!(
        output.contains("\x1b[2J"),
        "Startup sequence must contain terminal ClearType::All escape sequence"
    );
    assert!(
        output.contains("\x1b[1;1H") || output.contains("\x1b[H"),
        "Startup sequence must contain cursor reset to top-left (0,0)"
    );
}

// ===========================================================================
// Contract 2: Prompt rail symbol `┃` and status line `auto · <model>`
// ===========================================================================

#[test]
fn test_prompt_default_rail_symbols() {
    let prompt = Prompt::new();
    assert!(
        prompt.prompt_symbol().contains('┃'),
        "Default prompt symbol must contain the vertical rail '┃', found: {}",
        prompt.prompt_symbol()
    );
    assert!(
        prompt.multiline_symbol().contains('┃'),
        "Default multiline symbol must contain the vertical rail '┃', found: {}",
        prompt.multiline_symbol()
    );
}

#[test]
fn test_prompt_in_memory_render_rail_and_status_line() {
    let prompt = Prompt::new().with_model("deepseek-ai/DeepSeek-V4-Flash-0731");
    let mut buf = Vec::new();
    let buffer_chars: Vec<char> = "hello world".chars().collect();
    let mut last_lines = 0;
    let mut last_cursor = 0;

    let res = prompt.render_to(&mut buf, &buffer_chars, 11, &mut last_lines, &mut last_cursor);
    assert!(res.is_ok(), "Prompt render_to must succeed on in-memory buffer");

    let raw_out = String::from_utf8_lossy(&buf);
    let plain_out = strip_ansi(&raw_out);

    // Verify input line contains rail and text
    assert!(
        plain_out.contains("┃ hello world"),
        "Rendered prompt must contain '┃ hello world', got:\n{}",
        plain_out
    );

    // Verify status line displays "auto · <model>"
    assert!(
        plain_out.contains("auto · DeepSeek V4 Flash"),
        "Rendered prompt status line must contain 'auto · DeepSeek V4 Flash', got:\n{}",
        plain_out
    );
}

#[test]
fn test_prompt_render_multiline_input() {
    let prompt = Prompt::new().with_model("MiniMaxAI/MiniMax-M2.7");
    let mut buf = Vec::new();
    let multiline_text = "first line\nsecond line\nthird line";
    let buffer_chars: Vec<char> = multiline_text.chars().collect();
    let mut last_lines = 0;
    let mut last_cursor = 0;

    prompt
        .render_to(&mut buf, &buffer_chars, multiline_text.len(), &mut last_lines, &mut last_cursor)
        .expect("render_to multiline failed");

    let plain = strip_ansi(&String::from_utf8_lossy(&buf));
    assert!(plain.contains("┃ first line"));
    assert!(plain.contains("┃ second line"));
    assert!(plain.contains("┃ third line"));
    assert!(plain.contains("auto · MiniMax M2.7"));
}

#[test]
fn test_prompt_model_labels_formatting() {
    assert_eq!(format_model_label("deepseek-ai/DeepSeek-V4-Flash-0731"), "DeepSeek V4 Flash");
    assert_eq!(format_model_label("flash"), "DeepSeek V4 Flash");
    assert_eq!(format_model_label("v4"), "DeepSeek V4 Flash");
    assert_eq!(format_model_label("MiniMaxAI/MiniMax-M2.7"), "MiniMax M2.7");
    assert_eq!(format_model_label("minimax"), "MiniMax M2.7");
    assert_eq!(format_model_label("moonshotai/Kimi-K2.6"), "Kimi K2.6");
    assert_eq!(format_model_label("kimi"), "Kimi K2.6");
    assert_eq!(format_model_label("openai/gpt-4o"), "gpt-4o");
    assert_eq!(format_model_label(""), "auto");
    assert_eq!(format_model_label("custom-model"), "custom-model");
}

// ===========================================================================
// Contract 3: User prompt submission formatting (`┃ <user text>` + blank line)
// ===========================================================================

#[test]
fn test_submitted_prompt_rendering() {
    let mut buf = Vec::new();
    Prompt::render_submitted_prompt_to(&mut buf, "Fix the authentication bug")
        .expect("render_submitted_prompt_to should succeed");

    let raw = String::from_utf8_lossy(&buf);
    let plain = strip_ansi(&raw);

    assert!(
        plain.starts_with("┃ Fix the authentication bug\r\n\r\n")
            || plain.starts_with("┃ Fix the authentication bug\n\n"),
        "Submitted prompt must be followed by blank line, got: {:?}",
        plain
    );
}

#[test]
fn test_submitted_prompt_multiline_rendering() {
    let mut buf = Vec::new();
    Prompt::render_submitted_prompt_to(&mut buf, "line one\nline two\nline three")
        .expect("render_submitted_prompt_to should succeed");

    let plain = strip_ansi(&String::from_utf8_lossy(&buf));
    let expected = "┃ line one\r\n┃ line two\r\n┃ line three\r\n\r\n";
    let expected_lf = "┃ line one\n┃ line two\n┃ line three\n\n";

    assert!(
        plain == expected || plain == expected_lf,
        "Submitted multiline prompt must render rail on each line, got: {:?}",
        plain
    );
}

// ===========================================================================
// Contract 4: Thinking status format `Thinking ({time}) (↑{in} ↓{out})`
// ===========================================================================

#[test]
fn test_thinking_status_compact_formatting() {
    // Duration formatting
    assert_eq!(format_repl_duration_compact(Duration::from_secs(0)), "0s");
    assert_eq!(format_repl_duration_compact(Duration::from_secs(1)), "1s");
    assert_eq!(format_repl_duration_compact(Duration::from_secs(5)), "5s");
    assert_eq!(format_repl_duration_compact(Duration::from_secs(59)), "59s");
    assert_eq!(format_repl_duration_compact(Duration::from_secs(67)), "1m 7s");
    assert_eq!(format_repl_duration_compact(Duration::from_secs(3723)), "1h2m3s");

    // Token compact formatting
    assert_eq!(format_repl_tokens_compact(0), "0");
    assert_eq!(format_repl_tokens_compact(1), "1");
    assert_eq!(format_repl_tokens_compact(57), "57");
    assert_eq!(format_repl_tokens_compact(999), "999");
    assert_eq!(format_repl_tokens_compact(1200), "1.2k");
    assert_eq!(format_repl_tokens_compact(5000), "5k");
    assert_eq!(format_repl_tokens_compact(45000), "45k");
    assert_eq!(format_repl_tokens_compact(120000), "120k");
}

#[test]
fn test_thinking_status_full_frame() {
    let frame = format_thinking_status(
        Duration::from_secs(1),
        1,
        0,
        "DeepSeek V4 Flash",
    );
    let plain = strip_ansi(&frame);

    assert!(
        plain.contains("Thinking (1s) (↑1 ↓0)"),
        "Thinking status must contain 'Thinking (1s) (↑1 ↓0)', got:\n{}",
        plain
    );
    assert!(
        plain.contains("┃ "),
        "Thinking status frame must include active queue rail '┃ ', got:\n{}",
        plain
    );
    assert!(
        plain.contains("enter queue · auto · DeepSeek V4 Flash"),
        "Thinking status frame must include queue hint, got:\n{}",
        plain
    );
}

// ===========================================================================
// Contract 5: Queue hint format `enter queue · auto · <model>`
// ===========================================================================

#[test]
fn test_queue_hint_variations() {
    let models = [
        ("deepseek-ai/DeepSeek-V4-Flash-0731", "DeepSeek V4 Flash"),
        ("MiniMaxAI/MiniMax-M2.7", "MiniMax M2.7"),
        ("moonshotai/Kimi-K2.6", "Kimi K2.6"),
        ("anthropic/claude-3-5-sonnet", "claude-3-5-sonnet"),
    ];

    for (raw_model, expected_label) in models {
        let label = format_model_label(raw_model);
        assert_eq!(label, expected_label);

        let frame = format_thinking_status(Duration::from_secs(3), 150, 42, label);
        let plain = strip_ansi(&frame);
        let expected_hint = format!("enter queue · auto · {}", expected_label);
        assert!(
            plain.contains(&expected_hint),
            "Queue hint mismatch for model {}: expected to contain '{}', got:\n{}",
            raw_model,
            expected_hint,
            plain
        );
    }
}

// ===========================================================================
// Contract 6: Tool call tree structure with `●`, `├─`, `└─` and category breakdown
// ===========================================================================

#[test]
fn test_parse_tool_info_action_labels_and_categories() {
    // 1. Glob / Matched
    let (lbl, cat) = parse_tool_info("glob", &json!({"pattern": "src/**/*.rs"}));
    assert_eq!(lbl, "Matched src/**/*.rs");
    assert_eq!(cat, "list");

    // 2. Grep / Searched
    let (lbl, cat) = parse_tool_info("grep", &json!({"pattern": "fn run_turn"}));
    assert_eq!(lbl, "Searched fn run_turn");
    assert_eq!(cat, "list");

    // 3. Read / Read
    let (lbl, cat) = parse_tool_info("file", &json!({"action": "read", "path": "src/ui/repl.rs"}));
    assert_eq!(lbl, "Read src/ui/repl.rs");
    assert_eq!(cat, "read");

    let (lbl, cat) = parse_tool_info("read_file", &json!({"file": "Cargo.toml"}));
    assert_eq!(lbl, "Read Cargo.toml");
    assert_eq!(cat, "read");

    // 4. Write / Wrote
    let (lbl, cat) = parse_tool_info("write", &json!({"path": "tests/new_test.rs"}));
    assert_eq!(lbl, "Wrote tests/new_test.rs");
    assert_eq!(cat, "write");

    let (lbl, cat) = parse_tool_info("file", &json!({"action": "write", "path": "output.txt"}));
    assert_eq!(lbl, "Wrote output.txt");
    assert_eq!(cat, "write");

    // 5. Edit / Edited
    let (lbl, cat) = parse_tool_info("edit", &json!({"path": "src/ui/prompt.rs"}));
    assert_eq!(lbl, "Edited src/ui/prompt.rs");
    assert_eq!(cat, "edit");

    // 6. Patch / Patched
    let (lbl, cat) = parse_tool_info("patch", &json!({"file": "src/config.rs"}));
    assert_eq!(lbl, "Patched src/config.rs");
    assert_eq!(cat, "edit");

    // 7. Bash / Ran
    let (lbl, cat) = parse_tool_info("bash", &json!({"command": "cargo check"}));
    assert_eq!(lbl, "Ran cargo check");
    assert_eq!(cat, "command");

    // 8. Long command truncation
    let long_cmd = "cargo test --test tui_fx_style_test -- --nocapture --exact";
    let (lbl, cat) = parse_tool_info("bash", &json!({"command": long_cmd}));
    assert!(lbl.starts_with("Ran cargo test --test tui_fx_style_test"));
    assert!(lbl.ends_with('…'));
    assert_eq!(cat, "command");

    // 9. Web search / Searched
    let (lbl, cat) = parse_tool_info("web_search", &json!({"query": "rust ratatui inline"}));
    assert_eq!(lbl, "Searched rust ratatui inline");
    assert_eq!(cat, "read");

    // 10. Web fetch / Fetched
    let (lbl, cat) = parse_tool_info("fetch", &json!({"url": "https://crates.io/api/v1/crates/ratatui"}));
    assert_eq!(lbl, "Fetched https://crates.io/api/v1/crates/ratatui");
    assert_eq!(cat, "read");
}

#[test]
fn test_tool_tree_single_call_formatting() {
    let items = vec![
        ToolCallItem::new("file", "Read src/main.rs", "read"),
    ];

    let tree = format_tool_tree(&items);
    assert_eq!(
        tree,
        "● 1 tool call · 1 read\n└─ Read src/main.rs\n",
        "Single tool call must use singular 'tool call' and '└─' connector"
    );
}

#[test]
fn test_tool_tree_multi_call_aggregation() {
    let items = vec![
        ToolCallItem::new("glob", "Matched src/*.rs", "list"),
        ToolCallItem::new("grep", "Searched parse_tool_info", "list"),
        ToolCallItem::new("file", "Read src/ui/repl.rs", "read"),
        ToolCallItem::new("edit", "Edited src/ui/repl.rs", "edit"),
    ];

    let tree = format_tool_tree(&items);
    let lines: Vec<&str> = tree.lines().collect();

    assert_eq!(lines.len(), 5);
    // Header should list total and category counts sorted descending
    assert_eq!(lines[0], "● 4 tool calls · 2 list · 1 edit · 1 read");
    assert_eq!(lines[1], "├─ Matched src/*.rs");
    assert_eq!(lines[2], "├─ Searched parse_tool_info");
    assert_eq!(lines[3], "├─ Read src/ui/repl.rs");
    assert_eq!(lines[4], "└─ Edited src/ui/repl.rs");
}

#[test]
fn test_render_tool_tree_to_writer() {
    let mut buf = Vec::new();
    let items = vec![
        ToolCallItem::new("glob", "Matched tests/*.rs", "list"),
        ToolCallItem::new("file", "Read tests/smoke_test.rs", "read"),
    ];

    render_tool_tree_to(&mut buf, &items).expect("render_tool_tree_to must succeed");

    let raw = String::from_utf8_lossy(&buf);
    let plain = strip_ansi(&raw);

    assert!(plain.contains("● 2 tool calls · 1 list · 1 read"));
    assert!(plain.contains("├─ Matched tests/*.rs"));
    assert!(plain.contains("└─ Read tests/smoke_test.rs"));
}

// ===========================================================================
// Contract 7: Completed turn summary format `  {time} (↑{in} ↓{out})`
// ===========================================================================

#[test]
fn test_completed_turn_summary_formatting() {
    // 5s (↑1 ↓57)
    let summary1 = format_turn_summary(Duration::from_secs(5), 1, 57);
    let plain1 = strip_ansi(&summary1);
    assert_eq!(plain1, "  5s (↑1 ↓57)\r\n\r\n");

    // 1m 7s (↑5 ↓1.2k)
    let summary2 = format_turn_summary(Duration::from_secs(67), 5, 1200);
    let plain2 = strip_ansi(&summary2);
    assert_eq!(plain2, "  1m 7s (↑5 ↓1.2k)\r\n\r\n");

    // 1h2m3s (↑45k ↓120k)
    let summary3 = format_turn_summary(Duration::from_secs(3723), 45000, 120000);
    let plain3 = strip_ansi(&summary3);
    assert_eq!(plain3, "  1h2m3s (↑45k ↓120k)\r\n\r\n");
}

// ===========================================================================
// Contract 8: Markdown 2-space indentation and code block dividers
// ===========================================================================

#[test]
fn test_markdown_renderer_indent_and_code_blocks() {
    let mut md = MarkdownRenderer::buffered().with_indent(2);
    let input = "# Heading\nHere is a paragraph.\n```rust\nlet x = 42;\n```\nDone.";
    
    let output = md.push(input);
    let finish = md.finish();
    let total = format!("{}{}", output, finish);
    let plain = strip_ansi(&total);

    // Each emitted line should start with 2 spaces
    for line in plain.lines() {
        if !line.trim().is_empty() {
            assert!(
                line.starts_with("  "),
                "Line must be 2-space indented: {:?}",
                line
            );
        }
    }

    // Code block header and footer
    assert!(plain.contains("rust"), "Should contain language tag");
    assert!(plain.contains("let x = 42;"), "Should contain code line");
}

// ===========================================================================
// Contract 9: Full In-Memory REPL Turn Lifecycle Simulation
// ===========================================================================

#[test]
fn test_full_turn_lifecycle_in_memory() {
    let mut session_log = Vec::new();

    // 1. Startup: Clear screen & reset cursor
    execute!(
        session_log,
        terminal::Clear(ClearType::All),
        cursor::MoveTo(0, 0)
    ).unwrap();

    // 2. User submits prompt: Prompt rail `┃ <prompt>`
    Prompt::render_submitted_prompt_to(&mut session_log, "Find and fix memory leak").unwrap();

    // 3. Model is thinking: Thinking (1s) (↑120 ↓0) and queue hint
    let thinking = format_thinking_status(
        Duration::from_secs(1),
        120,
        0,
        "DeepSeek V4 Flash",
    );
    session_log.extend_from_slice(thinking.as_bytes());

    // 4. Tools executed: Tool tree
    let tools = vec![
        ToolCallItem::new("grep", "Searched Arc::clone", "list"),
        ToolCallItem::new("file", "Read src/agent/mesh.rs", "read"),
        ToolCallItem::new("edit", "Edited src/agent/mesh.rs", "edit"),
    ];
    render_tool_tree_to(&mut session_log, &tools).unwrap();

    // 5. Assistant response: 2-space indented markdown
    let mut md = MarkdownRenderer::buffered().with_indent(2);
    let md_out = md.push("I investigated and fixed the memory leak in `mesh.rs`.\n");
    let md_finish = md.finish();
    session_log.extend_from_slice(md_out.as_bytes());
    session_log.extend_from_slice(md_finish.as_bytes());

    // 6. Turn completion summary: 3s (↑120 ↓85)
    let summary = format_turn_summary(Duration::from_secs(3), 120, 85);
    session_log.extend_from_slice(summary.as_bytes());

    // Verify complete log content without PTY
    let plain_log = strip_ansi(&String::from_utf8_lossy(&session_log));

    assert!(plain_log.contains("┃ Find and fix memory leak"));
    assert!(plain_log.contains("Thinking (1s) (↑120 ↓0)"));
    assert!(plain_log.contains("enter queue · auto · DeepSeek V4 Flash"));
    assert!(plain_log.contains("● 3 tool calls · 1 edit · 1 list · 1 read"));
    assert!(plain_log.contains("├─ Searched Arc::clone"));
    assert!(plain_log.contains("├─ Read src/agent/mesh.rs"));
    assert!(plain_log.contains("└─ Edited src/agent/mesh.rs"));
    assert!(plain_log.contains("  I investigated and fixed the memory leak"));
    assert!(plain_log.contains("  3s (↑120 ↓85)"));
}
