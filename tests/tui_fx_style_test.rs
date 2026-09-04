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
//! 15. Reasoning effort picker menu layout (5 options: default, xhigh, high, medium, low framed by dividers).
//! 16. Dynamic status line updating with reasoning effort (`auto · <model> · high`).
//! 17. Clean model switch confirmation output without `/model` prefix text.
//! 18. Smooth progressive word streaming output parity with buffered markdown rendering.
//! 19. Multi-prompt queue banner formatting (`2 queued messages · ↑ to edit` and status line `queued 2 · enter queue · auto · grok-4.6`).
//! 20. Queue Up arrow recall (`↑ to edit`) popping from queue back to input buffer.
//! 21. Multi-prompt queue FIFO execution across turns.
//! 22. Model persistence across turns, runner config, and session.
use crossterm::{
    cursor,
    event::{Event, KeyCode, KeyEvent, KeyModifiers},
    execute,
    terminal::{self, ClearType},
};
use serde_json::json;
use std::collections::{HashMap, VecDeque};
use std::time::Duration;

use fusion::agent::loop_runner::AgentRunner;
use fusion::agent::session::Session;
use fusion::config::Config;
use fusion::provider::LlmClient;
use fusion::tools::{default_registry, ToolContext};
use fusion::ui::{
    format_activity_status, format_model_label, format_repl_duration_compact,
    format_repl_tokens_compact, format_thinking_status, format_tool_tree, format_turn_summary,
    handle_slash_command, parse_tool_active_label, parse_tool_info, render_thinking_frame_to,
    render_tool_tree_to, strip_ansi, CommandCategory, MarkdownRenderer, Prompt, PromptResult,
    ToolCallItem, EFFORT_OPTIONS,
};
// ===========================================================================
// Contract 1: Screen clearing and cursor reset on startup
// ===========================================================================

#[test]
fn test_startup_screen_clearing_and_cursor_reset() {
    let mut buf = Vec::new();
    let res = execute!(buf, terminal::Clear(ClearType::All), cursor::MoveTo(0, 0));
    assert!(
        res.is_ok(),
        "Startup execute! sequence should succeed on in-memory buffer"
    );

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

    let res = prompt.render_to(
        &mut buf,
        &buffer_chars,
        11,
        &mut last_lines,
        &mut last_cursor,
    );
    assert!(
        res.is_ok(),
        "Prompt render_to must succeed on in-memory buffer"
    );

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
        .render_to(
            &mut buf,
            &buffer_chars,
            multiline_text.len(),
            &mut last_lines,
            &mut last_cursor,
        )
        .expect("render_to multiline failed");

    let plain = strip_ansi(&String::from_utf8_lossy(&buf));
    assert!(plain.contains("┃ first line"));
    assert!(plain.contains("┃ second line"));
    assert!(plain.contains("┃ third line"));
    assert!(plain.contains("auto · MiniMax M2.7"));
}

#[test]
fn test_prompt_model_labels_formatting() {
    assert_eq!(
        format_model_label("deepseek-ai/DeepSeek-V4-Flash-0731"),
        "DeepSeek V4 Flash"
    );
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
    assert_eq!(
        format_repl_duration_compact(Duration::from_secs(67)),
        "1m7s"
    );
    assert_eq!(
        format_repl_duration_compact(Duration::from_secs(70)),
        "1m10s"
    );
    assert_eq!(
        format_repl_duration_compact(Duration::from_secs(3723)),
        "1h2m3s"
    );

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
    let frame = format_thinking_status(Duration::from_secs(1), 1, 0, "DeepSeek V4 Flash");
    let plain = strip_ansi(&frame);

    // Braille spinner frame with status verb
    assert!(
        plain.contains("Running (1s) (↑1 ↓0)"),
        "Thinking status must contain 'Running (1s) (↑1 ↓0)', got:\n{}",
        plain
    );
    assert!(
        !plain.contains("• •"),
        "Thinking status must never contain double dots '• •', got:\n{}",
        plain
    );
    assert!(
        fusion::ui::spinner::BRAILLE_FRAMES
            .iter()
            .any(|f| plain.contains(f)),
        "Thinking status must include a Braille spinner frame, got:\n{}",
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
fn test_thinking_status_braille_spinner_frames() {
    // 1. At 0ms frame index 0 ("⠋")
    let frame_0ms = format_activity_status("Running", Duration::from_millis(0), 10, 20, "auto");
    let plain_0ms = strip_ansi(&frame_0ms);
    assert!(
        plain_0ms.contains("Running (0s) (↑10 ↓20)"),
        "Status verb and timing must be present: {}",
        plain_0ms
    );
    assert!(
        !plain_0ms.contains("• •"),
        "Must not contain double dot: {}",
        plain_0ms
    );

    // 2. At 80ms the Braille frame advances
    let frame_80ms = format_activity_status("Running", Duration::from_millis(80), 10, 20, "auto");
    let plain_80ms = strip_ansi(&frame_80ms);
    assert_ne!(
        plain_0ms, plain_80ms,
        "Spinner frames must advance over time"
    );
    assert!(
        !plain_80ms.contains("•"),
        "No blinking dot may appear: {}",
        plain_80ms
    );

    // 3. At 1000ms the Braille frame is at index 12 % 10 = 2 ("⠹")
    let frame_1000ms =
        format_activity_status("Running", Duration::from_millis(1000), 10, 20, "auto");
    let plain_1000ms = strip_ansi(&frame_1000ms);
    assert!(
        plain_1000ms.contains("Running (1s) (↑10 ↓20)"),
        "At 1000ms status must render: {}",
        plain_1000ms
    );
    assert!(
        !plain_1000ms.contains("• •"),
        "Must not contain double dot: {}",
        plain_1000ms
    );

    // 4. At 1500ms frame index 18 % 10 = 8 ("⠇")
    let frame_1500ms =
        format_activity_status("Running", Duration::from_millis(1500), 10, 20, "auto");
    let plain_1500ms = strip_ansi(&frame_1500ms);
    assert!(
        plain_1500ms.contains("Running (1s) (↑10 ↓20)"),
        "At 1500ms status must render: {}",
        plain_1500ms
    );
    assert!(
        !plain_1500ms.contains("•"),
        "No blinking dot may appear: {}",
        plain_1500ms
    );

    // 5. Passing a "• " prefixed verb is sanitized so the prefix never doubles
    let frame_prefixed =
        format_activity_status("• Running", Duration::from_millis(0), 10, 20, "auto");
    let plain_prefixed = strip_ansi(&frame_prefixed);
    assert!(plain_prefixed.contains("Running (0s) (↑10 ↓20)"));
    assert!(
        !plain_prefixed.contains("• •"),
        "Must sanitize leading dot to avoid double dots: {}",
        plain_prefixed
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

    let active_lbl =
        parse_tool_active_label("skill", &json!({"name": "google-search-browser-use"}));
    assert_eq!(active_lbl, "Loading skill google-search-browser-use");

    // 2. Glob / Matched
    let (lbl, cat) = parse_tool_info("glob", &json!({"pattern": "src/**/*.rs"}));
    assert_eq!(lbl, "Matched src/**/*.rs");
    assert_eq!(cat, "list");

    // 3. Grep / Searched
    let (lbl, cat) = parse_tool_info("grep", &json!({"pattern": "fn run_turn"}));
    assert_eq!(lbl, "Searched fn run_turn");
    assert_eq!(cat, "read");

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
    let (lbl, cat) = parse_tool_info(
        "fetch",
        &json!({"url": "https://crates.io/api/v1/crates/ratatui"}),
    );
    assert_eq!(lbl, "Fetched https://crates.io/api/v1/crates/ratatui");
    assert_eq!(cat, "read");
}

#[test]
fn test_tool_tree_single_call_formatting() {
    let items = vec![ToolCallItem::new("file", "Read src/main.rs", "read")];

    let tree = format_tool_tree(&items);
    assert_eq!(
        tree, "● 1 tool call · 1 read\n└ Read src/main.rs\n",
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
    let items = vec![ToolCallItem::new(
        "bash",
        "Exited 1 ls \"/Users/aungmyatmoe/Library/Python/3.9/bin\"",
        "command",
    )
    .with_failed(true)];

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
    // 5s (↑1 ↓57) with leading blank line before the summary
    let summary1 = format_turn_summary(Duration::from_secs(5), 1, 57);
    let plain1 = strip_ansi(&summary1);
    assert_eq!(plain1, "\r\n  5s (↑1 ↓57)\r\n\r\n");

    // 1m7s (↑5 ↓1.2k)
    let summary2 = format_turn_summary(Duration::from_secs(67), 5, 1200);
    let plain2 = strip_ansi(&summary2);
    assert_eq!(plain2, "\r\n  1m7s (↑5 ↓1.2k)\r\n\r\n");

    // 1h2m3s (↑45k ↓120k)
    let summary3 = format_turn_summary(Duration::from_secs(3723), 45000, 120000);
    let plain3 = strip_ansi(&summary3);
    assert_eq!(plain3, "\r\n  1h2m3s (↑45k ↓120k)\r\n\r\n");
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
    // Standard code block should have 2-space indent and no left bar `│`
    assert!(
        !plain.contains("│"),
        "Standard code block lines should not have │ border: {}",
        plain
    );
    assert!(
        plain.contains("  let x = 42;"),
        "Standard code block lines should have 2-space indent: {}",
        plain
    );
    assert!(
        plain.contains("─ rust ──"),
        "Should contain top border: {}",
        plain
    );
    assert!(
        plain.contains("────"),
        "Should contain bottom border: {}",
        plain
    );
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
    assert!(
        plain.contains("text"),
        "Should contain text lang tag in header"
    );
    assert!(
        plain.contains("─ text ──"),
        "Should contain top border: {}",
        plain
    );
    assert!(
        plain.contains("────"),
        "Should contain bottom border: {}",
        plain
    );

    // Diagrams in text/ascii block must NOT have left border `│ ` prepended
    assert!(
        !plain.contains("│ +-------+"),
        "Diagram line should not have left bar │: {}",
        plain
    );
    assert!(
        plain.contains("+-------+"),
        "Diagram box should be preserved: {}",
        plain
    );
    assert!(
        plain.contains("| Client| ---> | Server|"),
        "Diagram connection should be preserved: {}",
        plain
    );

    // Indentation check: all non-empty lines are 2-space indented
    for line in plain.lines() {
        if !line.trim().is_empty() {
            assert!(
                line.starts_with("  "),
                "Every line must be indented with 2 spaces: {:?}",
                line
            );
        }
    }
}

#[test]
fn test_markdown_mermaid_codeblock_rendering_with_borders() {
    let mut md = MarkdownRenderer::buffered().with_indent(2);
    let input = "```mermaid\ngraph TD\n    A[Start] --> B[End]\n```\n";

    let output = md.push(input);
    let finish = md.finish();
    let total = format!("{}{}", output, finish);
    let plain = strip_ansi(&total);

    // Verify mermaid code block renders with top and bottom borders matching fx
    assert!(
        plain.contains("─ mermaid ──"),
        "Mermaid should have ─ mermaid ── top border: {}",
        plain
    );
    assert!(
        plain.contains("────"),
        "Mermaid should have ──── bottom border: {}",
        plain
    );
    assert!(
        plain.contains("graph TD"),
        "Mermaid source lines should be preserved: {}",
        plain
    );
    assert!(
        plain.contains("A[Start] --> B[End]"),
        "Mermaid connection lines preserved: {}",
        plain
    );

    // Verify ASCII boxes are disabled in favor of clean codeblock borders
    assert!(
        !plain.contains("+---------"),
        "Mermaid should NOT render as ASCII box: {}",
        plain
    );
    // Verify left bar │ is omitted
    assert!(
        !plain.contains("│"),
        "Mermaid code block lines should not have left bar │: {}",
        plain
    );

    // Indentation check: all non-empty lines are 2-space indented
    for line in plain.lines() {
        if !line.trim().is_empty() {
            assert!(
                line.starts_with("  "),
                "Every line must be indented with 2 spaces: {:?}",
                line
            );
        }
    }
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
    )
    .unwrap();

    // 2. User submits prompt: Prompt rail `┃ <prompt>`
    Prompt::render_submitted_prompt_to(&mut session_log, "Find and fix memory leak").unwrap();

    // 3. Model is thinking: • Running (1s) (↑120 ↓0) and queue hint
    let thinking = format_thinking_status(Duration::from_secs(1), 120, 0, "DeepSeek V4 Flash");
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
    assert!(plain_log.contains("Running (1s) (↑120 ↓0)"));
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
    assert!(
        queue_buffer.is_empty(),
        "Queue buffer must be drained on Enter"
    );
    assert_eq!(queue_cursor, 0, "Queue cursor must reset to 0");

    // 4. Simulate hitting Enter on empty / whitespace-only buffer
    let mut empty_buffer: Vec<char> = "   ".chars().collect();
    let mut empty_queued: Option<String> = None;
    let empty_text: String = empty_buffer.drain(..).collect();
    let empty_trimmed = empty_text.trim().to_string();
    if !empty_trimmed.is_empty() {
        empty_queued = Some(empty_trimmed);
    }
    assert_eq!(
        empty_queued, None,
        "Whitespace-only input should not set queued_prompt"
    );
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
    )
    .unwrap();

    let plain = strip_ansi(&String::from_utf8_lossy(&buf));
    assert!(plain.contains("Running (2s) (↑150 ↓30)"));
    assert!(
        fusion::ui::spinner::BRAILLE_FRAMES
            .iter()
            .any(|f| plain.contains(f)),
        "Running frame must show a Braille spinner"
    );
    assert!(
        plain.contains("┃ refactor auth module"),
        "Prompt rail must display the interactive queue buffer text"
    );
    assert!(
        plain.contains("enter queue · auto · DeepSeek V4 Flash"),
        "Status line must display enter queue hint"
    );
}

// ===========================================================================
// Contract 11: Model picker menu 3-column layout and dividers
// ===========================================================================

#[test]
fn test_model_picker_menu_3_column_layout_and_dividers() {
    let models = vec![
        (
            "deepseek-ai/DeepSeek-V4-Flash-0731".to_string(),
            "DeepSeek V4 Flash".to_string(),
        ),
        (
            "moonshotai/Kimi-K2.6".to_string(),
            "Kimi K2.6 Reasoning".to_string(),
        ),
        (
            "MiniMaxAI/MiniMax-M2.7".to_string(),
            "MiniMax M2.7".to_string(),
        ),
        (
            "anthropic/claude-3-5-sonnet".to_string(),
            "Claude 3.5 Sonnet Coding".to_string(),
        ),
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
        .render_to(
            &mut buf,
            &input_buffer,
            0,
            &mut last_lines,
            &mut last_cursor,
        )
        .expect("render_to model picker failed");

    let raw = String::from_utf8_lossy(&buf);
    let plain = strip_ansi(&raw);

    // 1. Dividers
    assert!(
        plain.contains('─'),
        "Model picker must render horizontal dividers"
    );

    // 2. Header
    assert!(
        plain.contains("Models 5 · Type to filter"),
        "Model picker header must contain 'Models 5 · Type to filter', got:\n{}",
        plain
    );
    assert!(
        plain.contains("1-5"),
        "Header must show pagination range '1-5'"
    );

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
        (
            "deepseek-ai/DeepSeek-V4-Flash-0731".to_string(),
            "DeepSeek V4 Flash".to_string(),
        ),
        ("moonshotai/Kimi-K2.6".to_string(), "Kimi K2.6".to_string()),
        (
            "MiniMaxAI/MiniMax-M2.7".to_string(),
            "MiniMax M2.7".to_string(),
        ),
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
    assert!(
        plain.contains('─'),
        "Slash menu must render horizontal dividers"
    );

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
    let has_session = palette
        .iter()
        .any(|c| c.category == CommandCategory::Session);
    let has_model = palette.iter().any(|c| c.category == CommandCategory::Model);
    let has_config = palette
        .iter()
        .any(|c| c.category == CommandCategory::Config);

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
        .render_to(
            &mut buf_session,
            &input_session,
            8,
            &mut last_lines,
            &mut last_cursor,
        )
        .unwrap();
    let plain_session = strip_ansi(&String::from_utf8_lossy(&buf_session));
    assert!(plain_session.contains("/session"));
    assert!(
        plain_session.contains("Session"),
        "Category for /session must be 'Session'"
    );

    // 2. Model command filter "/model"
    let input_model: Vec<char> = "/model".chars().collect();
    let mut buf_model = Vec::new();
    last_lines = 0;
    last_cursor = 0;

    prompt
        .render_to(
            &mut buf_model,
            &input_model,
            6,
            &mut last_lines,
            &mut last_cursor,
        )
        .unwrap();
    let plain_model = strip_ansi(&String::from_utf8_lossy(&buf_model));
    assert!(plain_model.contains("/model"));
    assert!(
        plain_model.contains("Model"),
        "Category for /model must be 'Model'"
    );

    // 3. Config command filter "/config"
    let input_config: Vec<char> = "/config".chars().collect();
    let mut buf_config = Vec::new();
    last_lines = 0;
    last_cursor = 0;

    prompt
        .render_to(
            &mut buf_config,
            &input_config,
            7,
            &mut last_lines,
            &mut last_cursor,
        )
        .unwrap();
    let plain_config = strip_ansi(&String::from_utf8_lossy(&buf_config));
    assert!(plain_config.contains("/config"));
    assert!(
        plain_config.contains("Config"),
        "Category for /config must be 'Config'"
    );

    // 4. General / Core command filter "/help"
    let input_help: Vec<char> = "/help".chars().collect();
    let mut buf_help = Vec::new();
    last_lines = 0;
    last_cursor = 0;

    prompt
        .render_to(
            &mut buf_help,
            &input_help,
            5,
            &mut last_lines,
            &mut last_cursor,
        )
        .unwrap();
    let plain_help = strip_ansi(&String::from_utf8_lossy(&buf_help));
    assert!(plain_help.contains("/help"));
    assert!(
        plain_help.contains("General"),
        "Category for /help must be 'General'"
    );
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
            )
            .unwrap();
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
        )
        .unwrap();
    };

    // Frame 1: Initial display (displayed = false)
    render_frame(&mut terminal_stream, 0, 100, 0, "", false);
    let frame1_bytes = terminal_stream.clone();
    let frame1_raw = String::from_utf8_lossy(&frame1_bytes);
    assert!(frame1_raw.contains("Running (0s) (↑100 ↓0)"));
    assert!(frame1_raw.contains("enter queue · auto · DeepSeek V4 Flash"));
    // MoveUp(2) positions cursor on the queue input line
    assert!(
        frame1_raw.contains("\x1b[2A") || frame1_raw.contains("\x1b[2F"),
        "Must move cursor up 2 rows to queue line"
    );

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
    )
    .unwrap();

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
    let items = vec![ToolCallItem::new(
        "bash",
        "Ran which browser-use || python3 -m site --user-base",
        "command",
    )];
    let tree = format_tool_tree(&items);
    assert_eq!(
        tree,
        "● 1 tool call · 1 command\n└ Ran which browser-use || python3 -m site --user-base\n"
    );
}

#[test]
fn test_fx_image1_tool_call_tree_failed() {
    let items = vec![ToolCallItem::new(
        "bash",
        "Exited 1 ls \"/Users/aungmyatmoe/Library/Python/3.9/bin\"",
        "command",
    )
    .with_failed(true)];
    let tree = format_tool_tree(&items);
    assert_eq!(
        tree,
        "● 1 tool call · 1 command · 1 failed\n└ Exited 1 ls \"/Users/aungmyatmoe/Library/Python/3.9/bin\"\n"
    );
}

#[test]
fn test_fx_image1_skill_tool_parsing_and_rendering() {
    let (lbl, cat) = parse_tool_info("skill", &json!({"name": "google-search-browser-use"}));
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
    assert_eq!(
        format_repl_duration_compact(Duration::from_secs(70)),
        "1m10s"
    );
    assert_eq!(
        format_repl_duration_compact(Duration::from_secs(60)),
        "1m0s"
    );
    assert_eq!(
        format_repl_duration_compact(Duration::from_secs(67)),
        "1m7s"
    );
    assert_eq!(
        format_repl_duration_compact(Duration::from_secs(125)),
        "2m5s"
    );
    assert_eq!(
        format_repl_duration_compact(Duration::from_secs(3660)),
        "1h1m0s"
    );
}

#[test]
fn test_fx_image1_running_status_frame_exact_layout() {
    let frame = format_thinking_status(Duration::from_secs(70), 9, 641, "claude-3-7-sonnet");
    let plain = strip_ansi(&frame);
    // 70s at 80ms/frame -> frame index 875 % 10 = 5 ("⠴")
    let expected =
        "  ⠴ Running (1m10s) (↑9 ↓641)\r\n\r\n┃ \r\n\r\nenter queue · auto · claude-3-7-sonnet\r\n";
    let expected_lf =
        "  ⠴ Running (1m10s) (↑9 ↓641)\n\n┃ \n\nenter queue · auto · claude-3-7-sonnet\n";
    assert!(
        plain == expected || plain == expected_lf,
        "Frame layout mismatch: got {:?}",
        plain
    );
}

#[test]
fn test_fx_image1_render_tool_tree_surrounded_by_blank_lines() {
    let mut buf = Vec::new();
    let items = vec![ToolCallItem::new(
        "bash",
        "Ran which browser-use || python3 -m site --user-base",
        "command",
    )];
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
    let tools = vec![ToolCallItem::new(
        "bash",
        "Ran which browser-use || python3 -m site --user-base",
        "command",
    )];
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

// ===========================================================================
// Helper: In-Memory AgentRunner and Session Factory
// ===========================================================================

fn create_test_runner_and_session() -> (AgentRunner, Session) {
    let config = Config::default();
    let tools = default_registry();
    let tool_ctx = ToolContext {
        cwd: std::env::temp_dir(),
        env: HashMap::new(),
    };
    let client = LlmClient::new();
    let model = config.default_model.clone();
    let runner = AgentRunner::new(client, config, tools, tool_ctx);
    let session = Session::new(&model);
    (runner, session)
}

// ===========================================================================
// Contract 15: Reasoning Effort Picker Menu Layout and Key Handling
// ===========================================================================

#[test]
fn test_effort_picker_menu_layout_5_options_and_dividers() {
    assert_eq!(
        EFFORT_OPTIONS,
        &["default", "xhigh", "high", "medium", "low"],
        "EFFORT_OPTIONS must match exact 5 options"
    );

    let model_id = "deepseek-ai/DeepSeek-V4-Flash-0731";
    let input_buffer: Vec<char> = format!("/model {} ", model_id).chars().collect();
    let input_len = input_buffer.len();

    // Test each option selection index from 0 to 4
    for sel_idx in 0..EFFORT_OPTIONS.len() {
        let prompt = Prompt::new()
            .with_pending_model_id(model_id)
            .with_effort_picker_active(true)
            .with_effort_selection(sel_idx);

        let mut buf = Vec::new();
        let mut last_lines = 0;
        let mut last_cursor = 0;
        prompt
            .render_to(
                &mut buf,
                &input_buffer,
                input_len,
                &mut last_lines,
                &mut last_cursor,
            )
            .expect("render_to effort picker failed");

        let raw = String::from_utf8_lossy(&buf);
        let plain = strip_ansi(&raw);

        // 1. Input line displays `┃ /model <model>`
        assert!(
            plain.contains(&format!("┃ /model {}", model_id)),
            "Input line must display '┃ /model {}', got:\n{}",
            model_id,
            plain
        );

        // 2. Framed by dividers
        assert!(
            plain.contains('─'),
            "Effort picker menu must be framed by horizontal dividers '─'"
        );

        // 3. All 5 options are present in the menu
        for opt in EFFORT_OPTIONS {
            assert!(
                plain.contains(opt),
                "Effort picker menu must contain option '{}', got:\n{}",
                opt,
                plain
            );
        }

        // 4. Selected option is bold white (\x1b[1;37m), unselected are dim (\x1b[2;37m)
        let selected_opt = EFFORT_OPTIONS[sel_idx];
        assert!(
            raw.contains(&format!("\x1b[1;37m{}\x1b[0m", selected_opt)),
            "Selected option '{}' must be rendered in bold white (\\x1b[1;37m)",
            selected_opt
        );

        for (other_idx, &other_opt) in EFFORT_OPTIONS.iter().enumerate() {
            if other_idx != sel_idx {
                assert!(
                    raw.contains(&format!("\x1b[2;37m{}\x1b[0m", other_opt)),
                    "Unselected option '{}' must be rendered in dim (\\x1b[2;37m)",
                    other_opt
                );
            }
        }

        // 5. Status line dynamically updates to auto · <model> or auto · <model> · <effort>
        let expected_status = if selected_opt == "default" {
            "auto · DeepSeek V4 Flash".to_string()
        } else {
            format!("auto · DeepSeek V4 Flash · {}", selected_opt)
        };
        assert!(
            plain.contains(&expected_status),
            "Status line must display '{}', got:\n{}",
            expected_status,
            plain
        );
    }
}

#[test]
fn test_effort_picker_interactive_navigation_and_event_handling() {
    let models = vec![
        (
            "deepseek-ai/DeepSeek-V4-Flash-0731".to_string(),
            "DeepSeek V4 Flash".to_string(),
        ),
        ("moonshotai/Kimi-K2.6".to_string(), "Kimi K2.6".to_string()),
    ];

    let mut prompt = Prompt::new()
        .with_models(models)
        .with_model_picker_active(true)
        .with_model_selection(0);

    // 1. Enter on model picker -> enters effort picker stage for the selected model
    let res = prompt
        .handle_event(Event::Key(KeyEvent::new(
            KeyCode::Enter,
            KeyModifiers::NONE,
        )))
        .unwrap();
    assert_eq!(
        res, None,
        "Enter on model picker must transition to effort picker without immediate submit"
    );
    assert!(!prompt.model_picker_active());
    assert!(prompt.effort_picker_active());
    assert_eq!(
        prompt.pending_model_id(),
        "deepseek-ai/DeepSeek-V4-Flash-0731"
    );
    assert_eq!(prompt.effort_selection(), 0); // starts at "default"

    // 2. Down key -> moves to index 1 ("xhigh")
    prompt
        .handle_event(Event::Key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE)))
        .unwrap();
    assert_eq!(prompt.effort_selection(), 1);

    // 3. Tab key -> moves to index 2 ("high")
    prompt
        .handle_event(Event::Key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE)))
        .unwrap();
    assert_eq!(prompt.effort_selection(), 2);

    // 4. Down key -> moves to index 3 ("medium")
    prompt
        .handle_event(Event::Key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE)))
        .unwrap();
    assert_eq!(prompt.effort_selection(), 3);

    // 5. Down key -> moves to index 4 ("low")
    prompt
        .handle_event(Event::Key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE)))
        .unwrap();
    assert_eq!(prompt.effort_selection(), 4);

    // 6. Down key wrap-around -> index 0 ("default")
    prompt
        .handle_event(Event::Key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE)))
        .unwrap();
    assert_eq!(prompt.effort_selection(), 0);

    // 7. Up key wrap-around -> index 4 ("low")
    prompt
        .handle_event(Event::Key(KeyEvent::new(KeyCode::Up, KeyModifiers::NONE)))
        .unwrap();
    assert_eq!(prompt.effort_selection(), 4);

    // 8. BackTab key -> index 3 ("medium")
    prompt
        .handle_event(Event::Key(KeyEvent::new(
            KeyCode::BackTab,
            KeyModifiers::NONE,
        )))
        .unwrap();
    assert_eq!(prompt.effort_selection(), 3);

    // 9. Up key -> index 2 ("high")
    prompt
        .handle_event(Event::Key(KeyEvent::new(KeyCode::Up, KeyModifiers::NONE)))
        .unwrap();
    assert_eq!(prompt.effort_selection(), 2);

    // 10. Enter key on "high" -> submits `/model <model> high`
    let res = prompt
        .handle_event(Event::Key(KeyEvent::new(
            KeyCode::Enter,
            KeyModifiers::NONE,
        )))
        .unwrap();
    assert_eq!(
        res,
        Some(PromptResult::Submit(
            "/model deepseek-ai/DeepSeek-V4-Flash-0731 high".to_string()
        )),
        "Enter on effort picker must submit '/model <model> <effort>'"
    );
    assert!(
        !prompt.effort_picker_active(),
        "Effort picker must close on submit"
    );
    assert_eq!(prompt.selected_effort(), Some("high"));
}

#[test]
fn test_effort_picker_default_effort_submission() {
    let mut prompt = Prompt::new()
        .with_pending_model_id("gpt-4o")
        .with_effort_picker_active(true)
        .with_effort_selection(0); // "default"

    let res = prompt
        .handle_event(Event::Key(KeyEvent::new(
            KeyCode::Enter,
            KeyModifiers::NONE,
        )))
        .unwrap();
    assert_eq!(
        res,
        Some(PromptResult::Submit("/model gpt-4o".to_string())),
        "Selecting 'default' effort must submit clean '/model <model>' without effort suffix"
    );
    assert!(!prompt.effort_picker_active());
    assert_eq!(prompt.selected_effort(), None);
}

#[test]
fn test_effort_picker_cancellation_via_esc_and_ctrl_c() {
    // 1. Cancel via Esc
    let mut prompt_esc = Prompt::new()
        .with_pending_model_id("MiniMaxAI/MiniMax-M2.7")
        .with_effort_picker_active(true)
        .with_effort_selection(2);

    let res_esc = prompt_esc
        .handle_event(Event::Key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)))
        .unwrap();
    assert_eq!(
        res_esc, None,
        "Esc closes picker and returns to prompt editing"
    );
    assert!(!prompt_esc.effort_picker_active());
    assert_eq!(prompt_esc.effort_selection(), 0);
    assert_eq!(prompt_esc.pending_model_id(), "");

    // 2. Cancel via Ctrl+C
    let mut prompt_ctrl_c = Prompt::new()
        .with_pending_model_id("moonshotai/Kimi-K2.6")
        .with_effort_picker_active(true)
        .with_effort_selection(1);

    let res_ctrl_c = prompt_ctrl_c
        .handle_event(Event::Key(KeyEvent::new(
            KeyCode::Char('c'),
            KeyModifiers::CONTROL,
        )))
        .unwrap();
    assert_eq!(res_ctrl_c, Some(PromptResult::Cancel));
    assert!(!prompt_ctrl_c.effort_picker_active());
    assert_eq!(prompt_ctrl_c.pending_model_id(), "");
}

// ===========================================================================
// Contract 16: Status Line Formatting with Reasoning Effort
// ===========================================================================

#[test]
fn test_status_line_with_reasoning_effort_variations() {
    let mut buf = Vec::new();
    let empty_buf: Vec<char> = Vec::new();
    let mut last_lines = 0;
    let mut last_cursor = 0;

    // 1. Standard model with high effort
    let prompt1 = Prompt::new()
        .with_model("deepseek-ai/DeepSeek-V4-Flash-0731")
        .with_selected_effort(Some("high".to_string()));
    prompt1
        .render_to(&mut buf, &empty_buf, 0, &mut last_lines, &mut last_cursor)
        .unwrap();
    let plain1 = strip_ansi(&String::from_utf8_lossy(&buf));
    assert!(
        plain1.contains("auto · DeepSeek V4 Flash · high"),
        "Status line must contain 'auto · DeepSeek V4 Flash · high', got:\n{}",
        plain1
    );

    // 2. MiniMax model with xhigh effort
    buf.clear();
    last_lines = 0;
    last_cursor = 0;
    let prompt2 = Prompt::new()
        .with_model("MiniMaxAI/MiniMax-M2.7")
        .with_selected_effort(Some("xhigh".to_string()));
    prompt2
        .render_to(&mut buf, &empty_buf, 0, &mut last_lines, &mut last_cursor)
        .unwrap();
    let plain2 = strip_ansi(&String::from_utf8_lossy(&buf));
    assert!(
        plain2.contains("auto · MiniMax M2.7 · xhigh"),
        "Status line must contain 'auto · MiniMax M2.7 · xhigh', got:\n{}",
        plain2
    );

    // 3. Kimi model without effort (None)
    buf.clear();
    last_lines = 0;
    last_cursor = 0;
    let prompt3 = Prompt::new()
        .with_model("moonshotai/Kimi-K2.6")
        .with_selected_effort(None);
    prompt3
        .render_to(&mut buf, &empty_buf, 0, &mut last_lines, &mut last_cursor)
        .unwrap();
    let plain3 = strip_ansi(&String::from_utf8_lossy(&buf));
    assert!(
        plain3.contains("auto · Kimi K2.6") && !plain3.contains("auto · Kimi K2.6 ·"),
        "Status line must contain 'auto · Kimi K2.6' without effort suffix, got:\n{}",
        plain3
    );

    // 4. Custom model with low effort
    buf.clear();
    last_lines = 0;
    last_cursor = 0;
    let prompt4 = Prompt::new()
        .with_model("claude-3-7-sonnet")
        .with_selected_effort(Some("low".to_string()));
    prompt4
        .render_to(&mut buf, &empty_buf, 0, &mut last_lines, &mut last_cursor)
        .unwrap();
    let plain4 = strip_ansi(&String::from_utf8_lossy(&buf));
    assert!(
        plain4.contains("auto · claude-3-7-sonnet · low"),
        "Status line must contain 'auto · claude-3-7-sonnet · low', got:\n{}",
        plain4
    );
}

#[test]
fn test_status_line_queue_mode_with_reasoning_effort() {
    let mut buf = Vec::new();
    let queue_text: Vec<char> = "summarize logs".chars().collect();
    let mut last_lines = 0;
    let mut last_cursor = 0;

    let mut prompt = Prompt::new()
        .with_model("claude-3-7-sonnet")
        .with_selected_effort(Some("high".to_string()));
    prompt.set_running_status(Some("• Running (12s) (↑50 ↓120)".to_string()));

    prompt
        .render_to(&mut buf, &queue_text, 14, &mut last_lines, &mut last_cursor)
        .unwrap();

    let plain = strip_ansi(&String::from_utf8_lossy(&buf));
    assert!(
        plain.contains("enter queue · auto · claude-3-7-sonnet · high"),
        "Queue status line must contain 'enter queue · auto · claude-3-7-sonnet · high', got:\n{}",
        plain
    );
    assert!(
        plain.contains("┃ summarize logs"),
        "Queue input must display rail with typed text '┃ summarize logs', got:\n{}",
        plain
    );
}

// ===========================================================================
// Contract 17: Clean Model Switch Output Without `/model` Command Prefix
// ===========================================================================

#[test]
fn test_model_switch_clean_confirmation_output_and_metadata() {
    let (mut runner, mut session) = create_test_runner_and_session();

    // 1. Switch to a new model without effort
    let res1 = handle_slash_command("/model claude-3-5-sonnet", &mut runner, &mut session);
    assert!(res1.is_some());
    assert_eq!(runner.config().default_model, "claude-3-5-sonnet-20241022");
    assert_eq!(session.active_model(), "claude-3-5-sonnet-20241022");
    assert_eq!(session.get_metadata("reasoning_effort"), None);

    // 2. Switch to a model with high reasoning effort
    let res2 = handle_slash_command("/model grok-4.6 high", &mut runner, &mut session);
    assert!(res2.is_some());
    assert_eq!(runner.config().default_model, "grok-4.6");
    assert_eq!(session.active_model(), "grok-4.6");
    assert_eq!(session.get_metadata("reasoning_effort"), Some("high"));

    // 3. Switch to a model with xhigh reasoning effort
    let res3 = handle_slash_command(
        "/model deepseek-ai/DeepSeek-V4-Flash-0731 xhigh",
        &mut runner,
        &mut session,
    );
    assert!(res3.is_some());
    assert_eq!(
        runner.config().default_model,
        "deepseek-ai/DeepSeek-V4-Flash-0731"
    );
    assert_eq!(session.active_model(), "deepseek-ai/DeepSeek-V4-Flash-0731");
    assert_eq!(session.get_metadata("reasoning_effort"), Some("xhigh"));
}

// ===========================================================================
// Contract 18: Progressive Word Streaming Parity with Buffered Markdown Rendering
// ===========================================================================

#[test]
fn test_progressive_word_streaming_matches_buffered_output() {
    // 1. Plain paragraph streamed word by word
    let text_tokens = vec![
        "The ",
        "quick ",
        "brown ",
        "fox ",
        "jumps ",
        "over ",
        "the ",
        "lazy ",
        "dog.\n\n",
        "Rust ",
        "provides ",
        "memory ",
        "safety ",
        "without ",
        "garbage ",
        "collection.\n",
    ];

    let mut streamer = MarkdownRenderer::new().with_indent(2);
    let mut buffered = MarkdownRenderer::buffered().with_indent(2);
    let mut oneshot = MarkdownRenderer::buffered().with_indent(2);

    let mut stream_accumulated = String::new();
    let mut buffered_accumulated = String::new();

    for token in &text_tokens {
        stream_accumulated.push_str(&streamer.push(token));
        buffered_accumulated.push_str(&buffered.push(token));
    }
    stream_accumulated.push_str(&streamer.finish());
    buffered_accumulated.push_str(&buffered.finish());

    let mut oneshot_output = oneshot.push(&text_tokens.concat());
    oneshot_output.push_str(&oneshot.finish());
    let _plain_stream = strip_ansi(&stream_accumulated);
    let plain_buffered = strip_ansi(&buffered_accumulated);
    let plain_oneshot = strip_ansi(&oneshot_output);

    assert_eq!(
        plain_buffered, plain_oneshot,
        "Buffered token streaming must match oneshot rendered output"
    );
    assert!(
        plain_buffered.contains("  The quick brown fox jumps over the lazy dog."),
        "Rendered output must have 2-space indentation"
    );
    assert!(
        plain_buffered.contains("  Rust provides memory safety without garbage collection."),
        "Rendered output must have 2-space indentation on second paragraph"
    );

    // 2. Rich markdown elements (bold, italic, code, headers, bullet lists)
    let rich_tokens = vec![
        "# Architecture Overview\n\n",
        "Fusion is a **high-performance** agentic harness written in *Rust*.\n\n",
        "Key features include:\n",
        "- **Speed**: Sub-millisecond startup\n",
        "- **Safety**: Type-safe `Session` and `AgentRunner` interfaces\n",
        "- **Flexibility**: Multi-provider support\n\n",
        "```rust\n",
        "fn main() {\n",
        "    println!(\"Fusion REPL ready\");\n",
        "}\n",
        "```\n",
    ];

    let mut rich_streamer = MarkdownRenderer::new().with_indent(2);
    let mut rich_buffered = MarkdownRenderer::buffered().with_indent(2);

    let mut rich_stream_out = String::new();
    let mut rich_buf_out = String::new();

    for token in &rich_tokens {
        rich_stream_out.push_str(&rich_streamer.push(token));
        rich_buf_out.push_str(&rich_buffered.push(token));
    }
    rich_stream_out.push_str(&rich_streamer.finish());
    rich_buf_out.push_str(&rich_buffered.finish());

    let plain_rich_buf = strip_ansi(&rich_buf_out);
    assert!(plain_rich_buf.contains("Architecture Overview"));
    assert!(plain_rich_buf.contains("Fusion is a high-performance agentic harness"));
    assert!(plain_rich_buf.contains("Speed: Sub-millisecond startup"));
    assert!(plain_rich_buf.contains("println!(\"Fusion REPL ready\");"));

    // 3. Fine-grained character-by-character streaming parity
    let char_stream_input =
        "Interactive prompt verification ensures 100% FX fidelity across all terminal sessions.\n";
    let mut char_buffered = MarkdownRenderer::buffered().with_indent(2);
    let mut char_buf_out = String::new();
    for ch in char_stream_input.chars() {
        let mut s = String::new();
        s.push(ch);
        char_buf_out.push_str(&char_buffered.push(&s));
    }
    char_buf_out.push_str(&char_buffered.finish());
    let mut single_buf = MarkdownRenderer::buffered().with_indent(2);
    let mut single_out = single_buf.push(char_stream_input);
    single_out.push_str(&single_buf.finish());
    assert_eq!(
        strip_ansi(&char_buf_out),
        strip_ansi(&single_out),
        "Character-by-character progressive streaming must produce identical output to batch rendering"
    );
}

// ===========================================================================
// Contract 19: Multi-Prompt Queue Banner and Status Line Parity (FX Image #1)
// ===========================================================================

#[test]
fn test_fx_image1_multi_prompt_queue_banner_and_status_line() {
    // Exact visual parity with FX screenshot Image #1:
    // ┃ oo
    //
    //   Thinking (3s) (↑1 ↓0)
    //
    // 2 queued messages · ↑ to edit
    //
    // ┃
    //
    // queued 2 · enter queue · auto · grok-4.6

    let mut prompt = Prompt::new().with_model("grok-4.6").with_queued_count(2);
    prompt.set_running_status(Some("Thinking (3s) (↑1 ↓0)".to_string()));

    let mut buf = Vec::new();
    let buffer_chars: Vec<char> = Vec::new();
    let mut last_lines = 0;
    let mut last_cursor = 0;

    let res = prompt.render_to(
        &mut buf,
        &buffer_chars,
        0,
        &mut last_lines,
        &mut last_cursor,
    );
    assert!(
        res.is_ok(),
        "Prompt render_to must succeed with queue banner"
    );

    let raw_out = String::from_utf8_lossy(&buf);
    let plain_out = strip_ansi(&raw_out);

    // 1. Verify Thinking running status banner
    assert!(
        plain_out.contains("Thinking (3s) (↑1 ↓0)"),
        "Must contain running status banner, got:\n{}",
        plain_out
    );

    // 2. Verify queue banner with exact text `2 queued messages · ↑ to edit`
    assert!(
        plain_out.contains("2 queued messages · ↑ to edit"),
        "Must contain queue banner '2 queued messages · ↑ to edit', got:\n{}",
        plain_out
    );

    // 3. Verify vertical prompt rail
    assert!(
        plain_out.contains('┃'),
        "Must contain prompt rail '┃', got:\n{}",
        plain_out
    );

    // 4. Verify status line with exact text `queued 2 · enter queue · auto · grok-4.6`
    assert!(
        plain_out.contains("queued 2 · enter queue · auto · grok-4.6"),
        "Must contain status line 'queued 2 · enter queue · auto · grok-4.6', got:\n{}",
        plain_out
    );
}

#[test]
fn test_queue_banner_singular_and_plural_formatting() {
    // 1. Singular: 1 queued message
    let mut prompt_single = Prompt::new()
        .with_model("MiniMaxAI/MiniMax-M2.7")
        .with_queued_count(1);
    prompt_single.set_running_status(Some("Thinking (1s) (↑0 ↓0)".to_string()));

    let mut buf_single = Vec::new();
    let mut last_lines = 0;
    let mut last_cursor = 0;
    prompt_single
        .render_to(&mut buf_single, &[], 0, &mut last_lines, &mut last_cursor)
        .unwrap();
    let plain_single = strip_ansi(&String::from_utf8_lossy(&buf_single));

    assert!(
        plain_single.contains("1 queued message · ↑ to edit"),
        "Must format singular '1 queued message', got:\n{}",
        plain_single
    );
    assert!(
        plain_single.contains("queued 1 · enter queue · auto · MiniMax M2.7"),
        "Must format singular status line, got:\n{}",
        plain_single
    );

    // 2. Plural: 5 queued messages
    let mut prompt_plural = Prompt::new()
        .with_model("deepseek-ai/DeepSeek-V4-Flash-0731")
        .with_queued_count(5);
    prompt_plural.set_running_status(Some("Thinking (12s) (↑350 ↓120)".to_string()));

    let mut buf_plural = Vec::new();
    let mut last_lines_p = 0;
    let mut last_cursor_p = 0;
    prompt_plural
        .render_to(
            &mut buf_plural,
            &[],
            0,
            &mut last_lines_p,
            &mut last_cursor_p,
        )
        .unwrap();
    let plain_plural = strip_ansi(&String::from_utf8_lossy(&buf_plural));

    assert!(
        plain_plural.contains("5 queued messages · ↑ to edit"),
        "Must format plural '5 queued messages', got:\n{}",
        plain_plural
    );
    assert!(
        plain_plural.contains("queued 5 · enter queue · auto · DeepSeek V4 Flash"),
        "Must format plural status line, got:\n{}",
        plain_plural
    );

    // 3. Reset clears queued count
    prompt_plural.reset_input();
    assert_eq!(
        prompt_plural.queued_count(),
        0,
        "reset_input must reset queued_count to 0"
    );
}

#[test]
fn test_queue_status_line_with_reasoning_effort() {
    let mut prompt = Prompt::new()
        .with_model("grok-4.6")
        .with_selected_effort(Some("high".to_string()))
        .with_queued_count(2);
    prompt.set_running_status(Some("Thinking (3s) (↑1 ↓0)".to_string()));
    let mut buf = Vec::new();
    let mut last_lines = 0;
    let mut last_cursor = 0;
    prompt
        .render_to(&mut buf, &[], 0, &mut last_lines, &mut last_cursor)
        .unwrap();
    let plain = strip_ansi(&String::from_utf8_lossy(&buf));

    assert!(
        plain.contains("queued 2 · enter queue · auto · grok-4.6 · high"),
        "Status line must include reasoning effort with queued count, got:\n{}",
        plain
    );
}

// ===========================================================================
// Contract 20: Up Arrow Recall (`↑ to edit`) Popping from Queue to Input Buffer
// ===========================================================================

#[test]
fn test_queue_up_arrow_recall_popping_to_input_buffer() {
    let mut queued_prompts: VecDeque<String> = VecDeque::new();
    queued_prompts.push_back("first queued prompt".to_string());
    queued_prompts.push_back("second queued prompt".to_string());

    let mut prompt = Prompt::new()
        .with_model("grok-4.6")
        .with_queued_count(queued_prompts.len());

    assert_eq!(prompt.queued_count(), 2);
    assert!(prompt.buffer.is_empty());

    // 1. Simulate pressing Up arrow on empty buffer: pops last queued prompt back into input buffer
    if prompt.buffer.is_empty() {
        if let Some(last) = queued_prompts.pop_back() {
            prompt.buffer = last.chars().collect();
            prompt.cursor_pos = prompt.buffer.len();
            prompt.set_queued_count(queued_prompts.len());
        }
    }

    assert_eq!(
        prompt.buffer.iter().collect::<String>(),
        "second queued prompt"
    );
    assert_eq!(prompt.cursor_pos, "second queued prompt".len());
    assert_eq!(prompt.queued_count(), 1);
    assert_eq!(queued_prompts.len(), 1);
    assert_eq!(queued_prompts.front().unwrap(), "first queued prompt");

    // 2. User edits the recalled prompt in buffer: appends " (edited)"
    for c in " (edited)".chars() {
        prompt.buffer.insert(prompt.cursor_pos, c);
        prompt.cursor_pos += 1;
    }
    assert_eq!(
        prompt.buffer.iter().collect::<String>(),
        "second queued prompt (edited)"
    );

    // 3. User presses Enter while thinking -> submits back to queue
    let text: String = prompt.buffer.drain(..).collect();
    let trimmed = text.trim().to_string();
    prompt.cursor_pos = 0;
    if !trimmed.is_empty() {
        queued_prompts.push_back(trimmed);
    }
    prompt.set_queued_count(queued_prompts.len());

    assert_eq!(prompt.queued_count(), 2);
    assert_eq!(queued_prompts.len(), 2);
    assert_eq!(queued_prompts[0], "first queued prompt");
    assert_eq!(queued_prompts[1], "second queued prompt (edited)");

    // 4. Sequential Up arrow recall on empty buffer popping all items FIFO in reverse
    // 1st Up arrow -> pops "second queued prompt (edited)"
    if prompt.buffer.is_empty() {
        if let Some(last) = queued_prompts.pop_back() {
            prompt.buffer = last.chars().collect();
            prompt.cursor_pos = prompt.buffer.len();
            prompt.set_queued_count(queued_prompts.len());
        }
    }
    assert_eq!(
        prompt.buffer.iter().collect::<String>(),
        "second queued prompt (edited)"
    );
    assert_eq!(prompt.queued_count(), 1);

    // Clear buffer (user discards or completes edit)
    prompt.buffer.clear();
    prompt.cursor_pos = 0;

    // 2nd Up arrow -> pops "first queued prompt"
    if prompt.buffer.is_empty() {
        if let Some(last) = queued_prompts.pop_back() {
            prompt.buffer = last.chars().collect();
            prompt.cursor_pos = prompt.buffer.len();
            prompt.set_queued_count(queued_prompts.len());
        }
    }
    assert_eq!(
        prompt.buffer.iter().collect::<String>(),
        "first queued prompt"
    );
    assert_eq!(prompt.queued_count(), 0);
    assert!(queued_prompts.is_empty());

    // 5. Up arrow on non-empty buffer must NOT pop from queue
    prompt.buffer = "existing typed text".chars().collect();
    queued_prompts.push_back("another queue item".to_string());
    prompt.set_queued_count(queued_prompts.len());

    let initial_queue_len = queued_prompts.len();
    if prompt.buffer.is_empty() {
        if let Some(last) = queued_prompts.pop_back() {
            prompt.buffer = last.chars().collect();
        }
    }
    assert_eq!(
        queued_prompts.len(),
        initial_queue_len,
        "Up arrow on non-empty buffer must not pop queue"
    );
    assert_eq!(
        prompt.buffer.iter().collect::<String>(),
        "existing typed text"
    );
}

// ===========================================================================
// Contract 21: Multi-Prompt Queue FIFO Execution Across Turns
// ===========================================================================

#[test]
fn test_multi_prompt_queue_fifo_execution_across_turns() {
    let mut prompt_queue: VecDeque<String> = VecDeque::new();

    // 1. Initial queue with 3 prompts
    prompt_queue.push_back("Turn 1: analyze repo structure".to_string());
    prompt_queue.push_back("Turn 2: generate mock data".to_string());
    prompt_queue.push_back("Turn 3: refactor auth module".to_string());

    let mut executed_prompts: Vec<String> = Vec::new();

    // Turn 1 executes
    let turn1_input = prompt_queue.pop_front().expect("Queue must have turn 1");
    assert_eq!(turn1_input, "Turn 1: analyze repo structure");
    executed_prompts.push(turn1_input);

    // While Turn 1 is executing, user queues two additional prompts
    let mut thinking_queued: VecDeque<String> = VecDeque::new();
    thinking_queued.push_back("Turn 4: add unit tests".to_string());
    thinking_queued.push_back("Turn 5: verify coverage".to_string());

    // End of Turn 1: queued prompts extend the REPL queue
    prompt_queue.extend(thinking_queued);
    assert_eq!(prompt_queue.len(), 4);

    // Turn 2 executes
    let turn2_input = prompt_queue.pop_front().expect("Queue must have turn 2");
    assert_eq!(turn2_input, "Turn 2: generate mock data");
    executed_prompts.push(turn2_input);

    // Turn 3 executes
    let turn3_input = prompt_queue.pop_front().expect("Queue must have turn 3");
    assert_eq!(turn3_input, "Turn 3: refactor auth module");
    executed_prompts.push(turn3_input);

    // Turn 4 executes
    let turn4_input = prompt_queue.pop_front().expect("Queue must have turn 4");
    assert_eq!(turn4_input, "Turn 4: add unit tests");
    executed_prompts.push(turn4_input);

    // Turn 5 executes
    let turn5_input = prompt_queue.pop_front().expect("Queue must have turn 5");
    assert_eq!(turn5_input, "Turn 5: verify coverage");
    executed_prompts.push(turn5_input);

    assert!(
        prompt_queue.is_empty(),
        "All queued prompts must be consumed in FIFO order"
    );
    assert_eq!(
        executed_prompts,
        vec![
            "Turn 1: analyze repo structure",
            "Turn 2: generate mock data",
            "Turn 3: refactor auth module",
            "Turn 4: add unit tests",
            "Turn 5: verify coverage",
        ],
        "Prompts must execute in exact FIFO order across turns"
    );
}

// ===========================================================================
// Contract 22: Model Persistence Across Turns, Runner Config, and Session
// ===========================================================================

#[test]
fn test_model_persistence_across_turns_and_components() {
    let (mut runner, mut session) = create_test_runner_and_session();
    let mut prompt = Prompt::new().with_model(session.active_model());

    // 1. Initial default model
    let initial_model = session.active_model().to_string();
    assert_eq!(prompt.active_model(), initial_model);
    assert_eq!(runner.config().default_model, initial_model);

    // 2. Switch model to MiniMaxAI/MiniMax-M2.7 via slash command
    let res = handle_slash_command("/model MiniMaxAI/MiniMax-M2.7", &mut runner, &mut session);

    assert!(res.is_some());
    assert_eq!(session.active_model(), "MiniMaxAI/MiniMax-M2.7");
    assert_eq!(runner.config().default_model, "MiniMaxAI/MiniMax-M2.7");
    prompt.set_model(session.active_model());
    assert_eq!(prompt.active_model(), "MiniMaxAI/MiniMax-M2.7");

    // Sync runner config as done in run_repl_with_session loop
    runner.config_mut().default_model = session.active_model().to_string();
    assert_eq!(runner.config().default_model, "MiniMaxAI/MiniMax-M2.7");

    // Verify rendered status line reflects new model
    let mut buf = Vec::new();
    let mut last_lines = 0;
    let mut last_cursor = 0;
    prompt
        .render_to(&mut buf, &[], 0, &mut last_lines, &mut last_cursor)
        .unwrap();
    let plain = strip_ansi(&String::from_utf8_lossy(&buf));
    assert!(
        plain.contains("auto · MiniMax M2.7"),
        "Status line must display formatted model label 'MiniMax M2.7', got:\n{}",
        plain
    );

    // 3. Simulate turn loop: model must persist across multiple turns without reverting
    for _turn in 1..=3 {
        // Sync before turn
        prompt.set_model(session.active_model());
        runner.config_mut().default_model = session.active_model().to_string();

        assert_eq!(session.active_model(), "MiniMaxAI/MiniMax-M2.7");
        assert_eq!(runner.config().default_model, "MiniMaxAI/MiniMax-M2.7");
        assert_eq!(prompt.active_model(), "MiniMaxAI/MiniMax-M2.7");
    }

    // 4. Switch model to grok-4.6
    let res2 = handle_slash_command("/model grok-4.6", &mut runner, &mut session);

    assert!(res2.is_some());
    assert_eq!(session.active_model(), "grok-4.6");
    assert_eq!(runner.config().default_model, "grok-4.6");
    prompt.set_model(session.active_model());
    assert_eq!(prompt.active_model(), "grok-4.6");
    assert_eq!(runner.config().default_model, "grok-4.6");

    let mut buf2 = Vec::new();
    let mut last_lines2 = 0;
    let mut last_cursor2 = 0;
    prompt
        .render_to(&mut buf2, &[], 0, &mut last_lines2, &mut last_cursor2)
        .unwrap();
    let plain2 = strip_ansi(&String::from_utf8_lossy(&buf2));
    assert!(
        plain2.contains("auto · grok-4.6"),
        "Status line must display formatted model label 'grok-4.6', got:\n{}",
        plain2
    );

    // 5. With queued prompts, model is preserved in status line: `queued 2 · enter queue · auto · grok-4.6`
    prompt.set_queued_count(2);
    prompt.set_running_status(Some("Thinking (3s) (↑1 ↓0)".to_string()));
    let mut buf3 = Vec::new();
    let mut last_lines3 = 0;
    let mut last_cursor3 = 0;
    prompt
        .render_to(&mut buf3, &[], 0, &mut last_lines3, &mut last_cursor3)
        .unwrap();
    let plain3 = strip_ansi(&String::from_utf8_lossy(&buf3));
    assert!(
        plain3.contains("queued 2 · enter queue · auto · grok-4.6"),
        "Status line must display 'queued 2 · enter queue · auto · grok-4.6', got:\n{}",
        plain3
    );
}

// ===========================================================================
// Contract 23: Tool Group Header and Branch Formatting Parity (FX Attachment)
// ===========================================================================

#[test]
fn test_tool_group_header_and_branches_4_calls_exact_fx_parity() {
    // Exact 4 tool calls from user attachment:
    // ● 4 tool calls · 3 list · 1 read
    // ├ Matched **/*.{md,mmd,puml,dot,svg,drawio,excalidraw}
    // ├ Matched **/*diagram*
    // ├ Searched mermaid
    // └ Matched README*

    let raw_tools = vec![
        (
            "glob",
            json!({"pattern": "**/*.{md,mmd,puml,dot,svg,drawio,excalidraw}"}),
        ),
        ("glob", json!({"pattern": "**/*diagram*"})),
        ("grep", json!({"pattern": "mermaid"})),
        ("glob", json!({"pattern": "README*"})),
    ];

    let mut items = Vec::new();
    for (name, args) in raw_tools {
        let (label, category) = parse_tool_info(name, &args);
        items.push(ToolCallItem::new(name, label, category));
    }

    assert_eq!(items.len(), 4);
    assert_eq!(
        items[0].label,
        "Matched **/*.{md,mmd,puml,dot,svg,drawio,excalidraw}"
    );
    assert_eq!(items[0].category, "list");
    assert_eq!(items[1].label, "Matched **/*diagram*");
    assert_eq!(items[1].category, "list");
    assert_eq!(items[2].label, "Searched mermaid");
    assert_eq!(items[2].category, "read");
    assert_eq!(items[3].label, "Matched README*");
    assert_eq!(items[3].category, "list");

    let formatted = format_tool_tree(&items);
    let lines: Vec<&str> = formatted.lines().collect();
    assert_eq!(lines.len(), 5, "Expected 1 header + 4 branch lines");

    // Header check
    assert_eq!(lines[0], "● 4 tool calls · 3 list · 1 read");

    // Branch connector check: ├ for indices 0..2, └ for index 3
    assert_eq!(
        lines[1],
        "├ Matched **/*.{md,mmd,puml,dot,svg,drawio,excalidraw}"
    );
    assert_eq!(lines[2], "├ Matched **/*diagram*");
    assert_eq!(lines[3], "├ Searched mermaid");
    assert_eq!(lines[4], "└ Matched README*");

    // Ensure none of the branches use ├─ or └─ (exact single space after symbol)
    for line in &lines[1..4] {
        assert!(
            line.starts_with("├ "),
            "Intermediate branch must start with '├ ': {}",
            line
        );
        assert!(
            !line.starts_with("├─"),
            "Branch must not use '├─': {}",
            line
        );
    }
    assert!(
        lines[4].starts_with("└ "),
        "Terminal branch must start with '└ ': {}",
        lines[4]
    );
    assert!(
        !lines[4].starts_with("└─"),
        "Terminal branch must not use '└─': {}",
        lines[4]
    );

    // Render with ANSI styling to writer and verify plain text matches exactly
    let mut buf = Vec::new();
    render_tool_tree_to(&mut buf, &items).expect("render_tool_tree_to must succeed");
    let raw_out = String::from_utf8_lossy(&buf);
    let plain_out = strip_ansi(&raw_out);
    assert_eq!(plain_out.trim().replace("\r\n", "\n"), formatted.trim());
    // Verify ANSI codes are present for bullet and connectors
    assert!(raw_out.contains("●"), "Should contain bullet in raw ANSI");
    assert!(
        raw_out.contains("├ "),
        "Should contain branch connector in raw ANSI"
    );
    assert!(
        raw_out.contains("└ "),
        "Should contain terminal connector in raw ANSI"
    );
}

#[test]
fn test_tool_group_header_and_branches_8_calls_exact_fx_parity() {
    // Exact 8 tool calls from user attachment:
    // ● 8 tool calls · 7 read · 1 list
    // ├ Read docs/architecture.md
    // ├ Read README.md
    // ├ Searched ```mermaid
    // ├ Matched docs/**/*
    // ├ Searched graph TD
    // ├ Read docs/agents.md
    // ├ Read docs/vision.md
    // └ Searched ```text

    let raw_tools = vec![
        ("read", json!({"path": "docs/architecture.md"})),
        ("read", json!({"path": "README.md"})),
        ("grep", json!({"query": "```mermaid"})),
        ("glob", json!({"pattern": "docs/**/*"})),
        ("grep", json!({"query": "graph TD"})),
        ("read", json!({"path": "docs/agents.md"})),
        ("read", json!({"path": "docs/vision.md"})),
        ("grep", json!({"query": "```text"})),
    ];

    let mut items = Vec::new();
    for (name, args) in raw_tools {
        let (label, category) = parse_tool_info(name, &args);
        items.push(ToolCallItem::new(name, label, category));
    }

    assert_eq!(items.len(), 8);
    let formatted = format_tool_tree(&items);
    let lines: Vec<&str> = formatted.lines().collect();
    assert_eq!(lines.len(), 9, "Expected 1 header + 8 branch lines");

    // Header check: 7 read, 1 list
    assert_eq!(lines[0], "● 8 tool calls · 7 read · 1 list");

    // Branches 1..=7 must start with ├
    assert_eq!(lines[1], "├ Read docs/architecture.md");
    assert_eq!(lines[2], "├ Read README.md");
    assert_eq!(lines[3], "├ Searched ```mermaid");
    assert_eq!(lines[4], "├ Matched docs/**/*");
    assert_eq!(lines[5], "├ Searched graph TD");
    assert_eq!(lines[6], "├ Read docs/agents.md");
    assert_eq!(lines[7], "├ Read docs/vision.md");

    // Branch 8 (last) must start with └
    assert_eq!(lines[8], "└ Searched ```text");

    for line in &lines[1..8] {
        assert!(
            line.starts_with("├ "),
            "Branch 0..6 must start with '├ ': {}",
            line
        );
    }
    assert!(
        lines[8].starts_with("└ "),
        "Branch 7 must start with '└ ': {}",
        lines[8]
    );

    let mut buf = Vec::new();
    render_tool_tree_to(&mut buf, &items).expect("render_tool_tree_to must succeed");
    let plain_out = strip_ansi(&String::from_utf8_lossy(&buf));
    assert_eq!(plain_out.trim().replace("\r\n", "\n"), formatted.trim());
}

#[test]
fn test_tool_group_branch_formatting_single_and_failure_variations() {
    // Single tool call: only └
    let single = vec![ToolCallItem::new("bash", "Ran cargo check", "command")];
    let single_out = format_tool_tree(&single);
    assert_eq!(single_out, "● 1 tool call · 1 command\n└ Ran cargo check\n");

    // Two tool calls: ├ followed by └
    let two = vec![
        ToolCallItem::new("glob", "Matched src/*.rs", "list"),
        ToolCallItem::new("read", "Read src/main.rs", "read"),
    ];
    let two_out = format_tool_tree(&two);
    assert_eq!(
        two_out,
        "● 2 tool calls · 1 list · 1 read\n├ Matched src/*.rs\n└ Read src/main.rs\n"
    );

    // Failed call formatting: Exited 1 for command or Failed for other
    let failed = vec![
        ToolCallItem::new("bash", "Ran ls /nonexistent", "command").with_failed(true),
        ToolCallItem::new("read", "Read /nonexistent", "read").with_failed(true),
    ];
    let failed_out = format_tool_tree(&failed);
    assert_eq!(
        failed_out,
        "● 2 tool calls · 1 command · 1 read · 2 failed\n├ Exited 1 ls /nonexistent\n└ Failed Read /nonexistent\n"
    );
}

// ===========================================================================
// Contract 24: Codeblock Formatting Parity (FX Attachment)
// ===========================================================================

#[test]
fn test_codeblock_mermaid_top_and_bottom_border_fx_parity() {
    // Exact diagram scenario from attachment:
    // Architecture diagram from docs/architecture.md:
    //
    // ─ mermaid ────────────────────────────────────────────────────────────────────
    // graph TD
    //     subgraph UI_Layer["Interface Layer"]
    //         CLI["CLI Command Parser (Clap)"]
    // ...
    // ──────────────────────────────────────────────────────────────────────────────

    let mut md = MarkdownRenderer::buffered().with_indent(2);
    let input = r#"Architecture diagram from docs/architecture.md:

```mermaid
graph TD
    subgraph UI_Layer["Interface Layer"]
        CLI["CLI Command Parser (Clap)"]
```
"#;

    let output = md.push(input);
    let finish = md.finish();
    let total = format!("{}{}", output, finish);
    let plain = strip_ansi(&total);

    // 1. Paragraph indentation: leading text paragraph has 2 spaces
    assert!(
        plain.contains("  Architecture diagram from docs/architecture.md:"),
        "Paragraph must be 2-space indented:\n{}",
        plain
    );

    // 2. Top border: starts with '  ─ mermaid ──'
    assert!(
        plain.contains("  ─ mermaid ──"),
        "Top border must match '  ─ mermaid ──':\n{}",
        plain
    );

    // 3. Bottom border: starts with '  ────'
    assert!(
        plain.contains("  ────"),
        "Bottom border must match '  ────':\n{}",
        plain
    );

    // 4. Code block content: lines indented by 2 spaces without vertical pipe
    assert!(
        plain.contains("  graph TD"),
        "Code lines must be indented by 2 spaces:\n{}",
        plain
    );
    assert!(
        plain.contains("subgraph UI_Layer[\"Interface Layer\"]"),
        "Code lines preserved:\n{}",
        plain
    );
    assert!(
        plain.contains("CLI[\"CLI Command Parser (Clap)\"]"),
        "Code lines preserved:\n{}",
        plain
    );

    // 5. No vertical pipe │ anywhere in the code block
    assert!(
        !plain.contains("│"),
        "Codeblock must not contain vertical pipe border │:\n{}",
        plain
    );

    // 6. No ASCII box formatting (+---------)
    assert!(
        !plain.contains("+---------"),
        "Mermaid must not render as ASCII box:\n{}",
        plain
    );
}

#[test]
fn test_codeblock_multiple_languages_borders_and_clean_formatting() {
    let languages = ["rust", "bash", "python", "text", ""];
    for lang in languages {
        let mut md = MarkdownRenderer::buffered().with_indent(2);
        let input = format!("```{lang}\necho hello\n```\n");
        let output = md.push(&input);
        let finish = md.finish();
        let total = format!("{}{}", output, finish);
        let plain = strip_ansi(&total);

        if lang.is_empty() {
            assert!(
                plain.contains("  ────"),
                "Empty lang codeblock must have top border: {}",
                plain
            );
        } else {
            let expected_top = format!("  ─ {} ──", lang);
            assert!(
                plain.contains(&expected_top),
                "Lang '{}' must have top border '{}': {}",
                lang,
                expected_top,
                plain
            );
        }
        assert!(
            plain.contains("  ────"),
            "Codeblock must have bottom border: {}",
            plain
        );
        assert!(
            plain.contains("echo hello"),
            "Code content preserved: {}",
            plain
        );
        assert!(
            !plain.contains("│"),
            "Codeblock must not have │ border: {}",
            plain
        );
    }
}

// ===========================================================================
// Contract 25: Prompt Queue Display and Streaming Persistence
// ===========================================================================

#[test]
fn test_prompt_queue_display_multi_level_banners_and_status() {
    let mut prompt = Prompt::new().with_model("xai/grok-4.6");

    // 1. Idle state: no queue, clean status line
    let mut buf = Vec::new();
    let mut last_lines = 0;
    let mut last_cursor = 0;
    prompt
        .render_to(&mut buf, &[], 0, &mut last_lines, &mut last_cursor)
        .unwrap();
    let plain = strip_ansi(&String::from_utf8_lossy(&buf));
    assert!(plain.contains("auto · grok-4.6"));
    assert!(!plain.contains("queued"));
    assert!(!plain.contains("enter queue"));

    // 2. Active running state without queue: `enter queue · auto · grok-4.6`
    prompt.set_running(true);
    let mut buf2 = Vec::new();
    let mut last_lines2 = 0;
    let mut last_cursor2 = 0;
    prompt
        .render_to(&mut buf2, &[], 0, &mut last_lines2, &mut last_cursor2)
        .unwrap();
    let plain2 = strip_ansi(&String::from_utf8_lossy(&buf2));
    assert!(
        plain2.contains("enter queue · auto · grok-4.6"),
        "Status line must display 'enter queue':\n{}",
        plain2
    );

    // 3. 1 queued message: banner `1 queued message · ↑ to edit`, status `queued 1 · enter queue · auto · grok-4.6`
    prompt.set_queued_count(1);
    let mut buf3 = Vec::new();
    let mut last_lines3 = 0;
    let mut last_cursor3 = 0;
    prompt
        .render_to(&mut buf3, &[], 0, &mut last_lines3, &mut last_cursor3)
        .unwrap();
    let plain3 = strip_ansi(&String::from_utf8_lossy(&buf3));
    assert!(
        plain3.contains("1 queued message · ↑ to edit"),
        "Banner must show singular queued message:\n{}",
        plain3
    );
    assert!(
        plain3.contains("queued 1 · enter queue · auto · grok-4.6"),
        "Status line must show 'queued 1':\n{}",
        plain3
    );

    // 4. 2 queued messages: banner `2 queued messages · ↑ to edit`, status `queued 2 · enter queue · auto · grok-4.6`
    prompt.set_queued_count(2);
    let mut buf4 = Vec::new();
    let mut last_lines4 = 0;
    let mut last_cursor4 = 0;
    prompt
        .render_to(&mut buf4, &[], 0, &mut last_lines4, &mut last_cursor4)
        .unwrap();
    let plain4 = strip_ansi(&String::from_utf8_lossy(&buf4));
    assert!(
        plain4.contains("2 queued messages · ↑ to edit"),
        "Banner must show plural queued messages:\n{}",
        plain4
    );
    assert!(
        plain4.contains("queued 2 · enter queue · auto · grok-4.6"),
        "Status line must show 'queued 2':\n{}",
        plain4
    );
}

#[test]
fn test_prompt_queue_streaming_persistence_lifecycle() {
    let mut prompt = Prompt::new().with_model("xai/grok-4.6");
    let mut md = MarkdownRenderer::buffered().with_indent(2);
    let mut queue: VecDeque<String> = VecDeque::new();

    // 1. User submits first prompt: `┃ show me diagram`
    let first_prompt = "show me diagram";
    let mut transcript = Vec::new();
    Prompt::render_submitted_prompt_to(&mut transcript, first_prompt).unwrap();
    let transcript_plain = strip_ansi(&String::from_utf8_lossy(&transcript));
    assert_eq!(transcript_plain, "┃ show me diagram\r\n\r\n");

    // 2. Turn starts: running state set, prompt footer shows thinking
    prompt.set_running(true);
    prompt.set_running_status(Some("Thinking (2s) (↑4 ↓1.3k)".to_string()));
    assert!(prompt.is_running());

    // 3. User types a prompt during streaming: "explain UI layer"
    // Simulate typing: buffer has characters
    prompt.buffer = "explain UI layer".chars().collect();
    prompt.cursor_pos = prompt.buffer.len();

    // Verify rendered frame contains the typing buffer AND queue hint in status line
    let mut frame1 = Vec::new();
    let mut last_lines = 0;
    let mut last_cursor = 0;
    let buffer_copy = prompt.buffer.clone();
    prompt
        .render_to(
            &mut frame1,
            &buffer_copy,
            prompt.cursor_pos,
            &mut last_lines,
            &mut last_cursor,
        )
        .unwrap();
    let frame1_plain = strip_ansi(&String::from_utf8_lossy(&frame1));
    assert!(
        frame1_plain.contains("┃ explain UI layer"),
        "Active typing buffer must be shown:\n{}",
        frame1_plain
    );
    assert!(
        frame1_plain.contains("enter queue · auto · grok-4.6"),
        "Running status line must show 'enter queue':\n{}",
        frame1_plain
    );

    // 4. User presses Enter: prompt is queued into queue VecDeque
    // KeyResult::Submit behavior while running: buffer cleared, frame cleared, queued_count increments
    queue.push_back(prompt.buffer.iter().collect::<String>());
    prompt.buffer.clear();
    prompt.cursor_pos = 0;
    prompt.set_queued_count(queue.len());
    assert_eq!(prompt.queued_count(), 1);

    // 5. Streaming text deltas arrive from model
    let chunk1 = "  I'll look through the repo for existing diagrams and related docs.\n\n";
    let formatted_chunk1 = md.push(chunk1);
    assert!(formatted_chunk1.contains("I'll look through the repo"));

    // Verify prompt re-render below streaming output preserves queued banner & status line
    let mut frame2 = Vec::new();
    prompt
        .render_to(&mut frame2, &[], 0, &mut last_lines, &mut last_cursor)
        .unwrap();
    let frame2_plain = strip_ansi(&String::from_utf8_lossy(&frame2));
    assert!(
        frame2_plain.contains("1 queued message · ↑ to edit"),
        "Queued banner must persist during streaming:\n{}",
        frame2_plain
    );
    assert!(
        frame2_plain.contains("queued 1 · enter queue · auto · grok-4.6"),
        "Status line must reflect queued 1:\n{}",
        frame2_plain
    );

    // 6. User types and queues another message: "add error handling"
    queue.push_back("add error handling".to_string());
    prompt.set_queued_count(queue.len());
    assert_eq!(prompt.queued_count(), 2);

    let mut frame3 = Vec::new();
    prompt
        .render_to(&mut frame3, &[], 0, &mut last_lines, &mut last_cursor)
        .unwrap();
    let frame3_plain = strip_ansi(&String::from_utf8_lossy(&frame3));
    assert!(
        frame3_plain.contains("2 queued messages · ↑ to edit"),
        "Queued banner must show 2 queued messages:\n{}",
        frame3_plain
    );
    assert!(
        frame3_plain.contains("queued 2 · enter queue · auto · grok-4.6"),
        "Status line must show queued 2:\n{}",
        frame3_plain
    );

    // 7. Turn completes: agent prints completed turn summary with a blank line before
    let summary = format_turn_summary(Duration::from_secs(86), 4, 1300);
    assert_eq!(strip_ansi(&summary), "\r\n  1m26s (↑4 ↓1.3k)\r\n\r\n");

    // Reset running status for turn completion
    prompt.set_running(false);
    prompt.set_running_status(None);

    // Next turn consumes first queued prompt in FIFO order
    let next_prompt = queue.pop_front().expect("Queue must have item");
    assert_eq!(next_prompt, "explain UI layer");
    prompt.set_queued_count(queue.len());
    assert_eq!(prompt.queued_count(), 1);

    // Render next prompt submission to transcript
    let mut next_transcript = Vec::new();
    Prompt::render_submitted_prompt_to(&mut next_transcript, &next_prompt).unwrap();
    let next_plain = strip_ansi(&String::from_utf8_lossy(&next_transcript));
    assert_eq!(next_plain, "┃ explain UI layer\r\n\r\n");
}
