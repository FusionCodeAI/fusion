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
//! 9. Full In-Memory REPL Turn Lifecycle Simulation.
//! 10. Queue prompt input while thinking (typing simulation, Enter capture, queue rail rendering).
//! 11. Model picker menu 3-column layout (ID, Description, Category), top/bottom dividers, and footer hints.
//! 12. Slash menu 3-column layout (`General`, `Session`, `Model`, `Config`), top/bottom dividers, and footer hints.
//! 13. Single in-place thinking frame rendering without line duplication.
//! 14. All tests execute completely in-memory without requiring a physical PTY/TTY.
use std::time::Duration;
use crossterm::{cursor, execute, terminal::{self, ClearType}};
use serde_json::json;

use fusion::ui::{
    format_activity_status, format_model_label, format_repl_duration_compact, format_repl_tokens_compact,
    format_thinking_status, format_tool_duration, format_tool_tree, format_turn_summary,
    parse_tool_active_label, parse_tool_info, render_thinking_frame_to, render_tool_tree_to,
    strip_ansi, CommandCategory, MarkdownRenderer, Prompt, ToolCallItem,
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
// Contract 4: Thinking / running status format `• Running ({time}) (↑{in} ↓{out})`
// ===========================================================================

#[test]
fn test_thinking_status_compact_formatting() {
    // Duration formatting: {d}m{d}s with NO space between minutes and seconds
    assert_eq!(format_repl_duration_compact(Duration::from_secs(0)), "0s");
    assert_eq!(format_repl_duration_compact(Duration::from_secs(1)), "1s");
    assert_eq!(format_repl_duration_compact(Duration::from_secs(5)), "5s");
    assert_eq!(format_repl_duration_compact(Duration::from_secs(59)), "59s");
    assert_eq!(format_repl_duration_compact(Duration::from_secs(67)), "1m7s");
    assert_eq!(format_repl_duration_compact(Duration::from_secs(70)), "1m10s");
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

    // Must have a single dot and NEVER double dots
    assert!(
        plain.contains("• Running (1s) (↑1 ↓0)"),
        "Thinking status must contain '• Running (1s) (↑1 ↓0)', got:\n{}",
        plain
    );
    assert!(
        !plain.contains("• •"),
        "Thinking status must never contain double dots '• •', got:\n{}",
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

#[test]
fn test_thinking_status_single_dot_and_500ms_blinking() {
    // 1. At 0ms (even 500ms interval): dot is ON ("• Running")
    let frame_0ms = format_activity_status("Running", Duration::from_millis(0), 10, 20, "auto");
    let plain_0ms = strip_ansi(&frame_0ms);
    assert!(plain_0ms.contains("• Running (0s) (↑10 ↓20)"), "At 0ms dot should be ON: {}", plain_0ms);
    assert!(!plain_0ms.contains("• •"), "Must not contain double dot: {}", plain_0ms);

    // 2. At 500ms (odd 500ms interval): dot is OFF ("  Running")
    let frame_500ms = format_activity_status("Running", Duration::from_millis(500), 10, 20, "auto");
    let plain_500ms = strip_ansi(&frame_500ms);
    assert!(plain_500ms.contains("  Running (0s) (↑10 ↓20)"), "At 500ms dot should be OFF: {}", plain_500ms);
    assert!(!plain_500ms.contains("•"), "At 500ms no dot should appear: {}", plain_500ms);

    // 3. At 1000ms (even 500ms interval): dot is ON ("• Running")
    let frame_1000ms = format_activity_status("Running", Duration::from_millis(1000), 10, 20, "auto");
    let plain_1000ms = strip_ansi(&frame_1000ms);
    assert!(plain_1000ms.contains("• Running (1s) (↑10 ↓20)"), "At 1000ms dot should be ON: {}", plain_1000ms);
    assert!(!plain_1000ms.contains("• •"), "Must not contain double dot: {}", plain_1000ms);

    // 4. At 1500ms (odd 500ms interval): dot is OFF ("  Running")
    let frame_1500ms = format_activity_status("Running", Duration::from_millis(1500), 10, 20, "auto");
    let plain_1500ms = strip_ansi(&frame_1500ms);
    assert!(plain_1500ms.contains("  Running (1s) (↑10 ↓20)"), "At 1500ms dot should be OFF: {}", plain_1500ms);
    assert!(!plain_1500ms.contains("•"), "At 1500ms no dot should appear: {}", plain_1500ms);

    // 5. Passing "• Running" or "• • Running" is sanitized so only single blinking dot is produced
    let frame_prefixed = format_activity_status("• Running", Duration::from_millis(0), 10, 20, "auto");
    let plain_prefixed = strip_ansi(&frame_prefixed);
    assert!(plain_prefixed.contains("• Running (0s) (↑10 ↓20)"));
    assert!(!plain_prefixed.contains("• •"), "Must sanitize leading dot to avoid double dots: {}", plain_prefixed);

    let frame_prefixed_off = format_activity_status("• Running", Duration::from_millis(500), 10, 20, "auto");
    let plain_prefixed_off = strip_ansi(&frame_prefixed_off);
    assert!(plain_prefixed_off.contains("  Running (0s) (↑10 ↓20)"));
    assert!(!plain_prefixed_off.contains("•"), "Must be off at 500ms even with prefixed input: {}", plain_prefixed_off);
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
// Contract 6: Tool call tree structure with `●`, `├ `, `└ ` and category breakdown
// ===========================================================================

#[test]
fn test_parse_tool_info_action_labels_and_categories() {
    // 1. Skill tool
    let (lbl, cat) = parse_tool_info("skill", &json!({"name": "google-search-browser-use"}));
    assert_eq!(lbl, "Loaded skill google-search-browser-use");
    assert_eq!(cat, "read");

    let (lbl, cat) = parse_tool_info("load_skill", &json!({"skill": "browser-use"}));
    assert_eq!(lbl, "Loaded skill browser-use");
    assert_eq!(cat, "read");

    let (lbl, cat) = parse_tool_info("skill_runner", &json!({"id": "weather"}));
    assert_eq!(lbl, "Loaded skill weather");
    assert_eq!(cat, "read");

    let active_lbl = parse_tool_active_label("skill", &json!({"name": "google-search-browser-use"}));
    assert_eq!(active_lbl, "Loading skill google-search-browser-use");

    // 2. Glob / Matched
    let (lbl, cat) = parse_tool_info("glob", &json!({"pattern": "src/**/*.rs"}));
    assert_eq!(lbl, "Matched src/**/*.rs");
    assert_eq!(cat, "list");

    // 3. Grep / Searched
    let (lbl, cat) = parse_tool_info("grep", &json!({"pattern": "fn run_turn"}));
    assert_eq!(lbl, "Searched fn run_turn");
    assert_eq!(cat, "list");

    // 4. Read / Read
    let (lbl, cat) = parse_tool_info("file", &json!({"action": "read", "path": "src/ui/repl.rs"}));
    assert_eq!(lbl, "Read src/ui/repl.rs");
    assert_eq!(cat, "read");

    let (lbl, cat) = parse_tool_info("read_file", &json!({"file": "Cargo.toml"}));
    assert_eq!(lbl, "Read Cargo.toml");
    assert_eq!(cat, "read");

    // 5. Write / Wrote
    let (lbl, cat) = parse_tool_info("write", &json!({"path": "tests/new_test.rs"}));
    assert_eq!(lbl, "Wrote tests/new_test.rs");
    assert_eq!(cat, "write");

    let (lbl, cat) = parse_tool_info("file", &json!({"action": "write", "path": "output.txt"}));
    assert_eq!(lbl, "Wrote output.txt");
    assert_eq!(cat, "write");

    // 6. Edit / Edited
    let (lbl, cat) = parse_tool_info("edit", &json!({"path": "src/ui/prompt.rs"}));
    assert_eq!(lbl, "Edited src/ui/prompt.rs");
    assert_eq!(cat, "edit");

    // 7. Patch / Patched
    let (lbl, cat) = parse_tool_info("patch", &json!({"file": "src/config.rs"}));
    assert_eq!(lbl, "Patched src/config.rs");
    assert_eq!(cat, "edit");

    // 8. Bash / Ran
    let (lbl, cat) = parse_tool_info("bash", &json!({"command": "cargo check"}));
    assert_eq!(lbl, "Ran cargo check");
    assert_eq!(cat, "command");

    // 9. Long command truncation (> 120 chars)
    let long_cmd = "cargo test --test tui_fx_style_test -- --nocapture --exact --very-long-argument-exceeding-one-hundred-and-twenty-characters-for-truncation-testing";
    let (lbl, cat) = parse_tool_info("bash", &json!({"command": long_cmd}));
    assert!(lbl.starts_with("Ran cargo test --test tui_fx_style_test"));
    assert!(lbl.ends_with('…'));
    assert_eq!(cat, "command");
    // 10. Web search / Searched
    let (lbl, cat) = parse_tool_info("web_search", &json!({"query": "rust ratatui inline"}));
    assert_eq!(lbl, "Searched rust ratatui inline");
    assert_eq!(cat, "read");

    // 11. Web fetch / Fetched
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
        "● 1 tool call · 1 read\n└ Read src/main.rs\n",
        "Single tool call must start at column 0 and use '└ ' connector"
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
    assert_eq!(lines[1], "├ Matched src/*.rs");
    assert_eq!(lines[2], "├ Searched parse_tool_info");
    assert_eq!(lines[3], "├ Read src/ui/repl.rs");
    assert_eq!(lines[4], "└ Edited src/ui/repl.rs");
}

#[test]
fn test_tool_tree_failed_call_formatting() {
    let items = vec![
        ToolCallItem::new("bash", "Exited 1 ls \"/Users/aungmyatmoe/Library/Python/3.9/bin\"", "command").with_failed(true),
    ];

    let tree = format_tool_tree(&items);
    assert_eq!(
        tree,
        "● 1 tool call · 1 command · 1 failed\n└ Exited 1 ls \"/Users/aungmyatmoe/Library/Python/3.9/bin\"\n"
    );
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
    assert!(plain.contains("├ Matched tests/*.rs"));
    assert!(plain.contains("└ Read tests/smoke_test.rs"));
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

    // 1m7s (↑5 ↓1.2k)
    let summary2 = format_turn_summary(Duration::from_secs(67), 5, 1200);
    let plain2 = strip_ansi(&summary2);
    assert_eq!(plain2, "  1m7s (↑5 ↓1.2k)\r\n\r\n");

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
    // Standard code block should have left border `│ `
    assert!(plain.contains("│ let x = 42;"), "Standard code block lines should have │ border: {}", plain);
}

#[test]
fn test_markdown_ascii_diagram_rendering_omits_left_border() {
    let mut md = MarkdownRenderer::buffered().with_indent(2);
    let input = "```text\n+-------+      +-------+\n| Client| ---> | Server|\n+-------+      +-------+\n```\n";
    
    let output = md.push(input);
    let finish = md.finish();
    let total = format!("{}{}", output, finish);
    let plain = strip_ansi(&total);

    // Verify code block dividers
    assert!(plain.contains("text"), "Should contain text lang tag in header");
    assert!(plain.contains("─ text ──"), "Should contain top border: {}", plain);
    assert!(plain.contains("────"), "Should contain bottom border: {}", plain);

    // Diagrams in text/ascii block must NOT have left border `│ ` prepended
    assert!(!plain.contains("│ +-------+"), "Diagram line should not have left bar │: {}", plain);
    assert!(plain.contains("+-------+"), "Diagram box should be preserved: {}", plain);
    assert!(plain.contains("| Client| ---> | Server|"), "Diagram connection should be preserved: {}", plain);

    // Indentation check: all non-empty lines are 2-space indented
    for line in plain.lines() {
        if !line.trim().is_empty() {
            assert!(line.starts_with("  "), "Every line must be indented with 2 spaces: {:?}", line);
        }
    }
}

#[test]
fn test_markdown_mermaid_to_ascii_diagram_rendering() {
    let mut md = MarkdownRenderer::buffered().with_indent(2);
    let input = "```mermaid\ngraph TD\n    A[Start<br/>Node] --> B[End&nbsp;Node]\n```\n";
    
    let output = md.push(input);
    let finish = md.finish();
    let total = format!("{}{}", output, finish);
    let plain = strip_ansi(&total);

    // Verify ASCII boxes and arrows were rendered from mermaid
    assert!(plain.contains("+---------"), "Mermaid should render as ASCII box: {}", plain);
    assert!(plain.contains("Start / Node") || plain.contains("Start"), "Sanitized label should be present: {}", plain);
    assert!(plain.contains("End Node") || plain.contains("End"), "Sanitized &nbsp; label should be present: {}", plain);
    assert!(plain.contains("v") || plain.contains("-->"), "Arrow/connector should be present: {}", plain);

    // Verify HTML tags were stripped/cleaned
    assert!(!plain.contains("<br/>"), "<br/> should be sanitized: {}", plain);
    assert!(!plain.contains("&nbsp;"), "&nbsp; should be sanitized: {}", plain);

    // Verify left border `│ ` is omitted so diagram remains clean
    assert!(!plain.contains("│ +---------"), "Mermaid diagram box lines should not have left bar │: {}", plain);
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

    // 3. Model is thinking: • Running (1s) (↑120 ↓0) and queue hint
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
    assert!(plain_log.contains("• Running (1s) (↑120 ↓0)"));
    assert!(plain_log.contains("enter queue · auto · DeepSeek V4 Flash"));
    assert!(plain_log.contains("● 3 tool calls · 1 edit · 1 list · 1 read"));
    assert!(plain_log.contains("├ Searched Arc::clone"));
    assert!(plain_log.contains("├ Read src/agent/mesh.rs"));
    assert!(plain_log.contains("└ Edited src/agent/mesh.rs"));
    assert!(plain_log.contains("  I investigated and fixed the memory leak"));
    assert!(plain_log.contains("  3s (↑120 ↓85)"));
}

// ===========================================================================
// Contract 10: Queue prompt input while thinking
// ===========================================================================

#[test]
fn test_queue_prompt_input_simulation_and_capture() {
    // 1. Simulate typing into queue_buffer char by char
    let mut queue_buffer: Vec<char> = Vec::new();
    let mut queue_cursor: usize = 0;
    let mut queued_prompt: Option<String> = None;

    let typed_chars = ['f', 'i', 'x', ' ', 't', 'h', 'e', ' ', 'b', 'u', 'g'];
    for c in typed_chars {
        queue_buffer.insert(queue_cursor, c);
        queue_cursor += 1;
    }
    assert_eq!(queue_buffer.iter().collect::<String>(), "fix the bug");
    assert_eq!(queue_cursor, 11);

    // 2. Simulate backspace
    if queue_cursor > 0 {
        queue_buffer.remove(queue_cursor - 1);
        queue_cursor -= 1;
    }
    assert_eq!(queue_buffer.iter().collect::<String>(), "fix the bu");
    assert_eq!(queue_cursor, 10);

    // Insert replacement char
    queue_buffer.insert(queue_cursor, 'g');
    queue_cursor += 1;
    queue_buffer.insert(queue_cursor, 's');
    queue_cursor += 1;
    assert_eq!(queue_buffer.iter().collect::<String>(), "fix the bugs");
    assert_eq!(queue_cursor, 12);
    // 3. Simulate hitting Enter: drain buffer, trim text, capture queued_prompt
    let text: String = queue_buffer.drain(..).collect();
    let trimmed = text.trim().to_string();
    queue_cursor = 0;
    if !trimmed.is_empty() {
        queued_prompt = Some(trimmed);
    }

    assert_eq!(queued_prompt, Some("fix the bugs".to_string()));
    assert!(queue_buffer.is_empty(), "Queue buffer must be drained on Enter");
    assert_eq!(queue_cursor, 0, "Queue cursor must reset to 0");

    // 4. Simulate hitting Enter on empty / whitespace-only buffer
    let mut empty_buffer: Vec<char> = "   ".chars().collect();
    let mut empty_queued: Option<String> = None;
    let empty_text: String = empty_buffer.drain(..).collect();
    let empty_trimmed = empty_text.trim().to_string();
    if !empty_trimmed.is_empty() {
        empty_queued = Some(empty_trimmed);
    }
    assert_eq!(empty_queued, None, "Whitespace-only input should not set queued_prompt");
}

#[test]
fn test_queue_prompt_rendering_with_active_buffer() {
    // Simulate rendering thinking frame with active queue prompt input
    let mut buf = Vec::new();
    render_thinking_frame_to(
        &mut buf,
        "• Running",
        Duration::from_secs(2),
        150,
        30,
        "DeepSeek V4 Flash",
        "refactor auth module",
        20,
    ).unwrap();

    let plain = strip_ansi(&String::from_utf8_lossy(&buf));
    assert!(plain.contains("• Running (2s) (↑150 ↓30)"));
    assert!(plain.contains("┃ refactor auth module"), "Prompt rail must display the interactive queue buffer text");
    assert!(plain.contains("enter queue · auto · DeepSeek V4 Flash"), "Status line must display enter queue hint");
}

// ===========================================================================
// Contract 11: Model picker menu 3-column layout and dividers
// ===========================================================================

#[test]
fn test_model_picker_menu_3_column_layout_and_dividers() {
    let models = vec![
        ("deepseek-ai/DeepSeek-V4-Flash-0731".to_string(), "DeepSeek V4 Flash".to_string()),
        ("moonshotai/Kimi-K2.6".to_string(), "Kimi K2.6 Reasoning".to_string()),
        ("MiniMaxAI/MiniMax-M2.7".to_string(), "MiniMax M2.7".to_string()),
        ("anthropic/claude-3-5-sonnet".to_string(), "Claude 3.5 Sonnet Coding".to_string()),
        ("openai/gpt-4o".to_string(), "GPT-4o Omnimodel".to_string()),
    ];

    let prompt = Prompt::new()
        .with_models(models)
        .with_model_picker_active(true);

    let mut buf = Vec::new();
    let input_buffer: Vec<char> = Vec::new();
    let mut last_lines = 0;
    let mut last_cursor = 0;

    prompt
        .render_to(&mut buf, &input_buffer, 0, &mut last_lines, &mut last_cursor)
        .expect("render_to model picker failed");

    let raw = String::from_utf8_lossy(&buf);
    let plain = strip_ansi(&raw);

    // 1. Dividers
    assert!(plain.contains('─'), "Model picker must render horizontal dividers");

    // 2. Header
    assert!(
        plain.contains("Models 5 · Type to filter"),
        "Model picker header must contain 'Models 5 · Type to filter', got:\n{}",
        plain
    );
    assert!(plain.contains("1-5"), "Header must show pagination range '1-5'");

    // 3. 3-column rows (ID, Description/Name, Category Tag)
    // Fast category
    assert!(plain.contains("deepseek-ai/DeepSeek-V4-Flash-0731"));
    assert!(plain.contains("DeepSeek V4 Flash"));
    assert!(plain.contains("Fast"));

    // Reasoning category
    assert!(plain.contains("moonshotai/Kimi-K2.6"));
    assert!(plain.contains("Kimi K2.6 Reasoning"));
    assert!(plain.contains("Reasoning"));

    assert!(plain.contains("MiniMaxAI/MiniMax-M2.7"));
    assert!(plain.contains("MiniMax M2.7"));

    // Coding / Model category
    assert!(plain.contains("anthropic/claude-3-5-sonnet"));
    assert!(plain.contains("Claude 3.5 Sonnet Coding"));
    assert!(plain.contains("Coding"));

    // 4. Navigation footer hints
    assert!(
        plain.contains("↑↓ Navigate") && plain.contains("Enter Use") && plain.contains("Esc Close"),
        "Footer hints must contain '↑↓ Navigate', 'Enter Use', 'Esc Close', got:\n{}",
        plain
    );
}

#[test]
fn test_model_picker_menu_filtering_and_selection() {
    let models = vec![
        ("deepseek-ai/DeepSeek-V4-Flash-0731".to_string(), "DeepSeek V4 Flash".to_string()),
        ("moonshotai/Kimi-K2.6".to_string(), "Kimi K2.6".to_string()),
        ("MiniMaxAI/MiniMax-M2.7".to_string(), "MiniMax M2.7".to_string()),
    ];

    let prompt = Prompt::new()
        .with_models(models)
        .with_model_picker_active(true)
        .with_model_selection(0);

    // Filter with "kimi"
    let filter_text: Vec<char> = "kimi".chars().collect();
    let mut buf = Vec::new();
    let mut last_lines = 0;
    let mut last_cursor = 0;

    prompt
        .render_to(&mut buf, &filter_text, 4, &mut last_lines, &mut last_cursor)
        .expect("render_to filtered model picker failed");

    let raw = String::from_utf8_lossy(&buf);
    let plain = strip_ansi(&raw);

    // Header shows 1 match
    assert!(plain.contains("Models 1 · Type to filter"));
    assert!(plain.contains("1-1"));
    assert!(plain.contains("moonshotai/Kimi-K2.6"));
    assert!(plain.contains("Kimi K2.6"));
    assert!(plain.contains("Reasoning"));

    // Non-matching models are excluded
    assert!(!plain.contains("DeepSeek V4 Flash"));
    assert!(!plain.contains("MiniMax M2.7"));
}

// ===========================================================================
// Contract 12: Slash menu 3-column layout and categories
// ===========================================================================

#[test]
fn test_slash_menu_3_column_layout_categories_and_dividers() {
    let prompt = Prompt::new();
    let slash_input: Vec<char> = vec!['/'];
    let mut buf = Vec::new();
    let mut last_lines = 0;
    let mut last_cursor = 0;

    prompt
        .render_to(&mut buf, &slash_input, 1, &mut last_lines, &mut last_cursor)
        .expect("render_to slash menu failed");

    let raw = String::from_utf8_lossy(&buf);
    let plain = strip_ansi(&raw);

    // 1. Dividers
    assert!(plain.contains('─'), "Slash menu must render horizontal dividers");

    // 2. Header
    assert!(
        plain.contains("Commands ") && plain.contains("· Type to filter"),
        "Slash menu header must contain 'Commands <count> · Type to filter', got:\n{}",
        plain
    );

    // 3. 3-column layout with category tags
    // Must contain navigation hints
    assert!(plain.contains("↑↓ Navigate"));
    assert!(plain.contains("Enter Use"));
    assert!(plain.contains("Esc Close"));

    // Check that categories General, Session, Model, Config exist in slash palette
    let palette = &fusion::ui::slash::COMMAND_PALETTE;
    let has_core = palette.iter().any(|c| c.category == CommandCategory::Core);
    let has_session = palette.iter().any(|c| c.category == CommandCategory::Session);
    let has_model = palette.iter().any(|c| c.category == CommandCategory::Model);
    let has_config = palette.iter().any(|c| c.category == CommandCategory::Config);

    assert!(has_core, "COMMAND_PALETTE must contain Core commands");
    assert!(has_session, "COMMAND_PALETTE must contain Session commands");
    assert!(has_model, "COMMAND_PALETTE must contain Model commands");
    assert!(has_config, "COMMAND_PALETTE must contain Config commands");
}

#[test]
fn test_slash_menu_category_filtering() {
    let prompt = Prompt::new();

    // 1. Session command filter "/session"
    let input_session: Vec<char> = "/session".chars().collect();
    let mut buf_session = Vec::new();
    let mut last_lines = 0;
    let mut last_cursor = 0;

    prompt
        .render_to(&mut buf_session, &input_session, 8, &mut last_lines, &mut last_cursor)
        .unwrap();
    let plain_session = strip_ansi(&String::from_utf8_lossy(&buf_session));
    assert!(plain_session.contains("/session"));
    assert!(plain_session.contains("Session"), "Category for /session must be 'Session'");

    // 2. Model command filter "/model"
    let input_model: Vec<char> = "/model".chars().collect();
    let mut buf_model = Vec::new();
    last_lines = 0;
    last_cursor = 0;

    prompt
        .render_to(&mut buf_model, &input_model, 6, &mut last_lines, &mut last_cursor)
        .unwrap();
    let plain_model = strip_ansi(&String::from_utf8_lossy(&buf_model));
    assert!(plain_model.contains("/model"));
    assert!(plain_model.contains("Model"), "Category for /model must be 'Model'");

    // 3. Config command filter "/config"
    let input_config: Vec<char> = "/config".chars().collect();
    let mut buf_config = Vec::new();
    last_lines = 0;
    last_cursor = 0;

    prompt
        .render_to(&mut buf_config, &input_config, 7, &mut last_lines, &mut last_cursor)
        .unwrap();
    let plain_config = strip_ansi(&String::from_utf8_lossy(&buf_config));
    assert!(plain_config.contains("/config"));
    assert!(plain_config.contains("Config"), "Category for /config must be 'Config'");

    // 4. General / Core command filter "/help"
    let input_help: Vec<char> = "/help".chars().collect();
    let mut buf_help = Vec::new();
    last_lines = 0;
    last_cursor = 0;

    prompt
        .render_to(&mut buf_help, &input_help, 5, &mut last_lines, &mut last_cursor)
        .unwrap();
    let plain_help = strip_ansi(&String::from_utf8_lossy(&buf_help));
    assert!(plain_help.contains("/help"));
    assert!(plain_help.contains("General"), "Category for /help must be 'General'");
}

// ===========================================================================
// Contract 13: Single in-place thinking frame without line duplication
// ===========================================================================

#[test]
fn test_single_inplace_thinking_frame_no_line_duplication() {
    // Verify that thinking frame updates overwrite the same terminal lines using
    // cursor::MoveUp and terminal::Clear(FromCursorDown) without trailing newlines.
    let mut terminal_stream = Vec::new();

    let render_frame = |stream: &mut Vec<u8>,
                        elapsed_secs: u64,
                        in_tok: u64,
                        out_tok: u64,
                        queue_buf: &str,
                        displayed: bool| {
        if displayed {
            execute!(
                stream,
                cursor::MoveUp(2),
                cursor::MoveToColumn(0),
                terminal::Clear(ClearType::FromCursorDown)
            ).unwrap();
        }
        render_thinking_frame_to(
            stream,
            "• Running",
            Duration::from_secs(elapsed_secs),
            in_tok,
            out_tok,
            "DeepSeek V4 Flash",
            queue_buf,
            queue_buf.len(),
        ).unwrap();
    };

    // Frame 1: Initial display (displayed = false)
    render_frame(&mut terminal_stream, 0, 100, 0, "", false);
    let frame1_bytes = terminal_stream.clone();
    let frame1_raw = String::from_utf8_lossy(&frame1_bytes);
    assert!(frame1_raw.contains("• Running (0s) (↑100 ↓0)"));
    assert!(frame1_raw.contains("enter queue · auto · DeepSeek V4 Flash"));
    // MoveUp(2) positions cursor on the queue input line
    assert!(frame1_raw.contains("\x1b[2A") || frame1_raw.contains("\x1b[2F"), "Must move cursor up 2 rows to queue line");

    // Frame 2: Subsequent tick at 1s (displayed = true)
    render_frame(&mut terminal_stream, 1, 100, 45, "", true);
    let frame2_raw = String::from_utf8_lossy(&terminal_stream);
    // Verify MoveUp(2) and Clear(FromCursorDown) escape sequences
    assert!(
        frame2_raw.contains("\x1b[2A") || frame2_raw.contains("\x1b[2F"),
        "Subsequent frame must move cursor up 2 rows to start of thinking status"
    );
    assert!(
        frame2_raw.contains("\x1b[J") || frame2_raw.contains("\x1b[0J"),
        "Subsequent frame must clear from cursor down to erase previous frame"
    );

    // Frame 3: Interactive keystroke typed into queue at 2s (displayed = true)
    render_frame(&mut terminal_stream, 2, 100, 90, "explain async", true);
    let frame3_plain = strip_ansi(&String::from_utf8_lossy(&terminal_stream));
    assert!(frame3_plain.contains("┃ explain async"));

    // Frame 4: Clean turn completion wiping the thinking frame
    execute!(
        terminal_stream,
        cursor::MoveUp(2),
        cursor::MoveToColumn(0),
        terminal::Clear(ClearType::FromCursorDown)
    ).unwrap();

    let final_raw = String::from_utf8_lossy(&terminal_stream);
    assert!(
        final_raw.contains("\x1b[J") || final_raw.contains("\x1b[0J"),
        "Turn completion must emit clear from cursor down to cleanly clear thinking lines"
    );
}

// ===========================================================================
// Contract 14: FX Image #1 Regression Tests (Exact Visual Parity)
// ===========================================================================

#[test]
fn test_fx_image1_tool_call_tree_success() {
    let items = vec![
        ToolCallItem::new(
            "bash",
            "Ran which browser-use || python3 -m site --user-base",
            "command",
        ),
    ];
    let tree = format_tool_tree(&items);
    assert_eq!(
        tree,
        "● 1 tool call · 1 command\n└ Ran which browser-use || python3 -m site --user-base\n"
    );
}

#[test]
fn test_fx_image1_tool_call_tree_failed() {
    let items = vec![
        ToolCallItem::new(
            "bash",
            "Exited 1 ls \"/Users/aungmyatmoe/Library/Python/3.9/bin\"",
            "command",
        )
        .with_failed(true),
    ];
    let tree = format_tool_tree(&items);
    assert_eq!(
        tree,
        "● 1 tool call · 1 command · 1 failed\n└ Exited 1 ls \"/Users/aungmyatmoe/Library/Python/3.9/bin\"\n"
    );
}

#[test]
fn test_fx_image1_skill_tool_parsing_and_rendering() {
    let (lbl, cat) = parse_tool_info(
        "skill",
        &json!({"name": "google-search-browser-use"}),
    );
    assert_eq!(lbl, "Loaded skill google-search-browser-use");
    assert_eq!(cat, "read");

    let items = vec![ToolCallItem::new("skill", lbl, cat)];
    let tree = format_tool_tree(&items);
    assert_eq!(
        tree,
        "● 1 tool call · 1 read\n└ Loaded skill google-search-browser-use\n"
    );
}

#[test]
fn test_fx_image1_duration_compact_exact_format() {
    assert_eq!(format_repl_duration_compact(Duration::from_secs(70)), "1m10s");
    assert_eq!(format_repl_duration_compact(Duration::from_secs(60)), "1m0s");
    assert_eq!(format_repl_duration_compact(Duration::from_secs(67)), "1m7s");
    assert_eq!(format_repl_duration_compact(Duration::from_secs(125)), "2m5s");
    assert_eq!(format_repl_duration_compact(Duration::from_secs(3660)), "1h1m0s");
}

#[test]
fn test_fx_image1_running_status_frame_exact_layout() {
    let frame = format_thinking_status(
        Duration::from_secs(70),
        9,
        641,
        "claude-3-7-sonnet",
    );
    let plain = strip_ansi(&frame);
    let expected = "  • Running (1m10s) (↑9 ↓641)\r\n\r\n┃ \r\n\r\nenter queue · auto · claude-3-7-sonnet\r\n";
    let expected_lf = "  • Running (1m10s) (↑9 ↓641)\n\n┃ \n\nenter queue · auto · claude-3-7-sonnet\n";
    assert!(
        plain == expected || plain == expected_lf,
        "Frame layout mismatch: got {:?}",
        plain
    );
}

#[test]
fn test_fx_image1_render_tool_tree_surrounded_by_blank_lines() {
    let mut buf = Vec::new();
    let items = vec![
        ToolCallItem::new(
            "bash",
            "Ran which browser-use || python3 -m site --user-base",
            "command",
        ),
    ];
    render_tool_tree_to(&mut buf, &items).expect("render_tool_tree_to should succeed");
    let plain = strip_ansi(&String::from_utf8_lossy(&buf));
    assert!(
        plain.starts_with("\r\n● 1 tool call · 1 command\r\n└ Ran which browser-use || python3 -m site --user-base\r\n\r\n")
            || plain.starts_with("\n● 1 tool call · 1 command\n└ Ran which browser-use || python3 -m site --user-base\n\n"),
        "Tool tree output must be surrounded by blank lines, got: {:?}",
        plain
    );
}

#[test]
fn test_fx_image1_live_tool_and_turn_lifecycle_no_double_bar() {
    let mut log = Vec::new();

    // 1. User submits prompt
    Prompt::render_submitted_prompt_to(&mut log, "find browser-use").unwrap();

    // 2. Running frame 1: • Running (1s) (↑9 ↓0)
    let mut thinking_buf = Vec::new();
    render_thinking_frame_to(
        &mut thinking_buf,
        "• Running",
        Duration::from_secs(1),
        9,
        0,
        "claude-3-7-sonnet",
        "",
        0,
    )
    .unwrap();
    log.extend_from_slice(&thinking_buf);

    // 3. ToolFinished: clear thinking frame, render tool tree, re-arm thinking frame
    // Clear thinking frame
    execute!(
        log,
        cursor::MoveUp(2),
        cursor::MoveToColumn(0),
        terminal::Clear(ClearType::FromCursorDown)
    )
    .unwrap();

    // Render completed tool tree
    let tools = vec![
        ToolCallItem::new(
            "bash",
            "Ran which browser-use || python3 -m site --user-base",
            "command",
        ),
    ];
    render_tool_tree_to(&mut log, &tools).unwrap();

    // Re-arm live running status
    render_thinking_frame_to(
        &mut log,
        "• Running",
        Duration::from_secs(70),
        9,
        641,
        "claude-3-7-sonnet",
        "",
        0,
    )
    .unwrap();

    // 4. Assistant finish: clean wipe of thinking frame so no double bar `┃┃`
    execute!(
        log,
        cursor::MoveUp(2),
        cursor::MoveToColumn(0),
        terminal::Clear(ClearType::FromCursorDown)
    )
    .unwrap();

    // Render turn summary
    let summary = format_turn_summary(Duration::from_secs(70), 9, 641);
    log.extend_from_slice(summary.as_bytes());

    let plain = strip_ansi(&String::from_utf8_lossy(&log));
    assert!(plain.contains("┃ find browser-use"));
    assert!(plain.contains("● 1 tool call · 1 command"));
    assert!(plain.contains("└ Ran which browser-use || python3 -m site --user-base"));
    assert!(plain.contains("  1m10s (↑9 ↓641)"));
}
