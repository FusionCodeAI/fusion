//! Comprehensive integration test suite for `ToolOutputCard` and `render_tool_cards`.
//!
//! Verifies:
//! 1. Card layout with rounded box-drawing characters (`╭`, `─`, `╮`, `│`, `╰`, `╯`).
//! 2. Header chip formatting: tool name, title/summary, elapsed duration in parentheses.
//! 3. Content line padding and exact visible width matching `terminal_width`.
//! 4. Automatic text wrapping to `terminal_width - 4` using `wrap_ansi`.
//! 5. Output truncation for giant outputs (>30 lines) with `… N earlier lines elided …`.
//! 6. Status-based header coloring (green for success, red for failure, cyan for read-only tools).
//! 7. Multi-card rendering via `render_tool_cards`.
//! 8. Plain-text rendering without ANSI codes (`render_plain`).

use std::time::Duration;

use fusion::ui;

#[path = "../src/ui/tool_card.rs"]
pub mod tool_card;

use fusion::ui::table::{strip_ansi, visible_width};
use tool_card::{
    format_duration, is_read_only_tool, render_tool_cards, ToolOutputCard, ToolStatus, ANSI_CYAN,
    ANSI_GRAY, ANSI_GREEN, ANSI_RED, ANSI_RESET,
};

// ============================================================================
// 1. Layout & Border Tests
// ============================================================================

#[test]
fn test_tool_card_basic_layout() {
    let card = ToolOutputCard::success(
        "bash",
        "cargo test",
        "15 passed in 0.02s",
        Some(Duration::from_millis(15)),
    );

    let rendered = card.render(63);
    let plain = strip_ansi(&rendered);
    let lines: Vec<&str> = plain.lines().collect();

    assert_eq!(lines.len(), 3, "Expected 3 lines (top, content, bottom)");

    // Top border: ╭─ bash: cargo test (15ms) ───────────────────────────────────╮
    assert!(
        lines[0].starts_with("╭─ "),
        "Top border must start with '╭─ '"
    );
    assert!(lines[0].ends_with('╮'), "Top border must end with '╮'");
    assert!(
        lines[0].contains("bash: cargo test (15ms)"),
        "Top line must contain tool name, title and duration"
    );

    // Content: │ 15 passed in 0.02s ... │
    assert!(
        lines[1].starts_with("│ "),
        "Content line must start with '│ '"
    );
    assert!(lines[1].ends_with(" │"), "Content line must end with ' │'");
    assert!(
        lines[1].contains("15 passed in 0.02s"),
        "Content line must contain stdout"
    );

    // Bottom border: ╰─────────────────────────────────────────────────────────────╯
    assert!(
        lines[2].starts_with('╰'),
        "Bottom border must start with '╰'"
    );
    assert!(lines[2].ends_with('╯'), "Bottom border must end with '╯'");
    assert!(
        lines[2].chars().all(|c| c == '╰' || c == '╯' || c == '─'),
        "Bottom border must consist only of bottom corners and dashes"
    );

    // Verify exact visible widths
    for (idx, line) in lines.iter().enumerate() {
        assert_eq!(
            visible_width(line),
            63,
            "Line {} visible width must exactly match terminal width 63, got {}",
            idx,
            visible_width(line)
        );
    }
}

#[test]
fn test_tool_card_empty_content() {
    let card = ToolOutputCard::success("touch", "foo.txt", "", Some(Duration::from_millis(5)));

    let rendered = card.render(60);
    let plain = strip_ansi(&rendered);
    let lines: Vec<&str> = plain.lines().collect();

    assert_eq!(
        lines.len(),
        2,
        "Empty content card should have 2 lines (top & bottom border)"
    );
    assert_eq!(visible_width(lines[0]), 60);
    assert_eq!(visible_width(lines[1]), 60);
}

#[test]
fn test_tool_card_multiline_content() {
    let content = "line 1\nline 2\nline 3";
    let card = ToolOutputCard::success("echo", "multiline", content, None);

    let rendered = card.render(50);
    let plain = strip_ansi(&rendered);
    let lines: Vec<&str> = plain.lines().collect();

    assert_eq!(
        lines.len(),
        5,
        "Expected 5 lines (top + 3 content + bottom)"
    );
    for (i, line) in lines.iter().enumerate() {
        assert_eq!(visible_width(line), 50, "Line {} width mismatch", i);
    }
    assert!(lines[1].contains("line 1"));
    assert!(lines[2].contains("line 2"));
    assert!(lines[3].contains("line 3"));
}

#[test]
fn test_render_tool_cards_multiple() {
    let card1 = ToolOutputCard::success(
        "bash",
        "cargo test",
        "15 passed in 0.02s",
        Some(Duration::from_millis(15)),
    );
    let card2 = ToolOutputCard::success(
        "read",
        "src/main.rs",
        "fn main() {}",
        Some(Duration::from_millis(2)),
    );

    let combined = render_tool_cards(&[card1, card2], 60);
    assert!(combined.contains("bash: cargo test"));
    assert!(combined.contains("read: src/main.rs"));

    let plain = strip_ansi(&combined);
    let cards_blocks: Vec<&str> = plain.split("\n\n").collect();
    assert_eq!(cards_blocks.len(), 2, "Expected 2 separated card blocks");
}

#[test]
fn test_render_tool_cards_empty_slice() {
    let combined = render_tool_cards(&[], 80);
    assert!(combined.is_empty(), "Empty slice must render empty string");
}

// ============================================================================
// 2. Text Wrapping Tests
// ============================================================================

#[test]
fn test_tool_card_wrapping() {
    let long_line = "The quick brown fox jumps over the lazy dog and runs across the wide open green meadow under the warm golden sun.";
    let card = ToolOutputCard::success("echo", "wrap test", long_line, None);

    // Terminal width 50 -> inner content width is 50 - 4 = 46
    let rendered = card.render(50);
    let plain = strip_ansi(&rendered);
    let lines: Vec<&str> = plain.lines().collect();

    assert!(
        lines.len() > 3,
        "Long line must be wrapped into multiple lines, got total lines: {}",
        lines.len()
    );

    for (i, line) in lines.iter().enumerate() {
        assert_eq!(
            visible_width(line),
            50,
            "Wrapped line {} width must match terminal_width 50",
            i
        );
        assert!(line.starts_with('│') || line.starts_with('╭') || line.starts_with('╰'));
        assert!(line.ends_with('│') || line.ends_with('╮') || line.ends_with('╯'));
    }
}

#[test]
fn test_tool_card_wrapping_with_ansi() {
    let styled_content = "\x1b[32mPASS\x1b[0m \x1b[1mtests::test_foo\x1b[0m - Successfully verified all invariants with high precision and zero errors";
    let card = ToolOutputCard::success("test_runner", "suite", styled_content, None);

    let rendered = card.render(40);
    let plain = strip_ansi(&rendered);
    let lines: Vec<&str> = plain.lines().collect();

    assert!(lines.len() > 3, "Styled content should wrap cleanly");
    for (i, line) in lines.iter().enumerate() {
        assert_eq!(visible_width(line), 40, "Line {} width should be 40", i);
    }
    // ANSI codes must be in raw output
    assert!(rendered.contains("\x1b[32mPASS"));
}

// ============================================================================
// 3. Truncation Tests (>30 lines)
// ============================================================================

#[test]
fn test_tool_card_not_truncated_at_30_lines() {
    let mut content = String::new();
    for i in 1..=30 {
        content.push_str(&format!("Row {}\n", i));
    }

    let card = ToolOutputCard::success("data", "dump", content.trim_end(), None);
    let rendered = card.render(60);

    assert!(
        !rendered.contains("elided"),
        "Output with <= 30 lines must NOT be truncated"
    );

    let plain = strip_ansi(&rendered);
    let lines: Vec<&str> = plain.lines().collect();
    // 1 top + 30 content + 1 bottom = 32 lines
    assert_eq!(lines.len(), 32);
    assert!(lines[1].contains("Row 1"));
    assert!(lines[30].contains("Row 30"));
}

#[test]
fn test_tool_card_truncated_over_30_lines() {
    let mut content = String::new();
    for i in 1..=50 {
        content.push_str(&format!("Line number {}\n", i));
    }

    let card = ToolOutputCard::success("runner", "long_job", content.trim_end(), None);
    let rendered = card.render(60);

    // 50 total lines - 30 displayed = 20 earlier lines elided
    assert!(
        rendered.contains("… 20 earlier lines elided …"),
        "Must contain exact elision string: '… 20 earlier lines elided …', got:\n{}",
        rendered
    );

    let plain = strip_ansi(&rendered);
    let lines: Vec<&str> = plain.lines().collect();

    // 1 top + 1 elided + 30 content + 1 bottom = 33 lines
    assert_eq!(lines.len(), 33, "Expected 33 lines after truncation");
    assert!(lines[1].contains("… 20 earlier lines elided …"));
    // First displayed row after elision must be row 21
    assert!(lines[2].contains("Line number 21"));
    // Last displayed row must be row 50
    assert!(lines[31].contains("Line number 50"));
}

#[test]
fn test_tool_card_truncated_31_lines() {
    let mut content = String::new();
    for i in 1..=31 {
        content.push_str(&format!("Item {}\n", i));
    }

    let card = ToolOutputCard::success("query", "items", content.trim_end(), None);
    let rendered = card.render(60);

    assert!(
        rendered.contains("… 1 earlier lines elided …"),
        "31 lines should elide exactly 1 line"
    );
    assert!(rendered.contains("Item 2"));
    assert!(rendered.contains("Item 31"));
}

#[test]
fn test_tool_card_custom_max_lines() {
    let mut content = String::new();
    for i in 1..=10 {
        content.push_str(&format!("Entry {}\n", i));
    }

    let card = ToolOutputCard::success("log", "tail", content.trim_end(), None).with_max_lines(5);

    let rendered = card.render(60);
    assert!(
        rendered.contains("… 5 earlier lines elided …"),
        "10 lines with max_lines=5 should elide 5 lines"
    );
    assert!(rendered.contains("Entry 6"));
    assert!(rendered.contains("Entry 10"));
}

// ============================================================================
// 4. ANSI Color Code Tests
// ============================================================================

#[test]
fn test_tool_card_colors_success_green() {
    let card = ToolOutputCard::success(
        "bash",
        "cargo build --release",
        "Finished release [optimized] target(s) in 1.45s",
        Some(Duration::from_millis(1450)),
    );

    let rendered = card.render(80);
    // Success header chip must be green (\x1b[32m)
    assert!(
        rendered.contains(ANSI_GREEN),
        "Successful non-read-only tool must have green header chip (\\x1b[32m)"
    );
    assert!(
        !rendered.contains(ANSI_RED),
        "Successful tool must NOT have red header chip"
    );
}

#[test]
fn test_tool_card_colors_failure_red() {
    let card = ToolOutputCard::failure(
        "bash",
        "cargo test",
        "error[E0432]: unresolved import `foo`\n1 error generated",
        Some(Duration::from_millis(200)),
    );

    let rendered = card.render(80);
    // Failed tool header chip must be red (\x1b[31m)
    assert!(
        rendered.contains(ANSI_RED),
        "Failed tool must have red header chip (\\x1b[31m)"
    );
    assert!(
        !rendered.contains(ANSI_GREEN),
        "Failed tool must NOT have green header chip"
    );
}

#[test]
fn test_tool_card_colors_read_only_cyan() {
    let read_tools = ["read", "grep", "glob", "web_search", "fetch", "view", "cat"];
    for tool in read_tools {
        let card = ToolOutputCard::success(
            tool,
            "test_target",
            "search results...",
            Some(Duration::from_millis(10)),
        );

        let rendered = card.render(70);
        assert!(
            rendered.contains(ANSI_CYAN),
            "Read-only tool '{}' on success must have cyan header chip (\\x1b[36m)",
            tool
        );
        assert!(
            !rendered.contains(ANSI_GREEN),
            "Read-only tool '{}' should use cyan, not green",
            tool
        );
    }
}

#[test]
fn test_tool_card_read_only_failure_is_red() {
    // If a read tool fails (e.g. file not found), failure status takes precedence -> RED
    let card = ToolOutputCard::failure(
        "read",
        "non_existent.rs",
        "Error: No such file or directory",
        Some(Duration::from_millis(1)),
    );

    let rendered = card.render(70);
    assert!(
        rendered.contains(ANSI_RED),
        "Failed read tool must be colored RED, not cyan"
    );
    assert!(
        !rendered.contains(ANSI_CYAN),
        "Failed read tool must not be cyan"
    );
}

#[test]
fn test_tool_card_explicit_read_only_override() {
    let card = ToolOutputCard::success(
        "custom_query",
        "SELECT * FROM users",
        "10 rows returned",
        Some(Duration::from_millis(50)),
    )
    .with_read_only(true);

    let rendered = card.render(70);
    assert!(
        rendered.contains(ANSI_CYAN),
        "Explicitly marked read-only tool must have cyan header chip"
    );
}

#[test]
fn test_tool_card_border_styling() {
    let card = ToolOutputCard::success("bash", "test", "ok", None);
    let rendered = card.render(60);

    // Borders must be styled in gray (\x1b[90m)
    assert!(
        rendered.contains(ANSI_GRAY),
        "Borders must contain gray ANSI code (\\x1b[90m)"
    );
    // Reset codes must be present
    assert!(
        rendered.contains(ANSI_RESET),
        "Must contain ANSI reset codes"
    );
}

// ============================================================================
// 5. Edge Cases & Helpers
// ============================================================================

#[test]
fn test_tool_card_render_plain() {
    let card = ToolOutputCard::success(
        "bash",
        "cargo test",
        "15 passed in 0.02s",
        Some(Duration::from_millis(15)),
    );

    let plain = card.render_plain(63);
    assert!(
        !plain.contains("\x1b["),
        "render_plain must not contain ANSI escape sequences"
    );
    assert!(plain.contains("bash: cargo test (15ms)"));
    assert!(plain.contains("15 passed in 0.02s"));
}

#[test]
fn test_header_chip_long_title_truncation() {
    let very_long_title = "a".repeat(100);
    let card = ToolOutputCard::success("bash", &very_long_title, "done", None);

    let rendered = card.render(50);
    let plain = strip_ansi(&rendered);
    let lines: Vec<&str> = plain.lines().collect();

    assert_eq!(
        visible_width(lines[0]),
        50,
        "Header line with very long title must fit terminal width 50"
    );
    assert!(
        lines[0].contains('…'),
        "Long title should be truncated with ellipsis"
    );
}

#[test]
fn test_format_duration_helper() {
    assert_eq!(format_duration(Duration::from_millis(15)), "15ms");
    assert_eq!(format_duration(Duration::from_millis(500)), "500ms");
    assert_eq!(format_duration(Duration::from_millis(1000)), "1s");
    assert_eq!(format_duration(Duration::from_millis(1500)), "1.5s");
    assert_eq!(format_duration(Duration::from_secs(60)), "1m");
    assert_eq!(format_duration(Duration::from_secs(75)), "1m 15s");
}

#[test]
fn test_is_read_only_tool_helper() {
    assert!(is_read_only_tool("read"));
    assert!(is_read_only_tool("READ"));
    assert!(is_read_only_tool("read_file"));
    assert!(is_read_only_tool("grep"));
    assert!(is_read_only_tool("glob"));
    assert!(is_read_only_tool("web_search"));
    assert!(is_read_only_tool("fetch"));
    assert!(is_read_only_tool("view"));
    assert!(is_read_only_tool("cat"));
    assert!(is_read_only_tool("scout"));

    assert!(!is_read_only_tool("bash"));
    assert!(!is_read_only_tool("write"));
    assert!(!is_read_only_tool("edit"));
    assert!(!is_read_only_tool("git_commit"));
}

#[test]
fn test_tool_status_enum_and_methods() {
    let s_card = ToolOutputCard::success("bash", "test", "ok", None);
    assert_eq!(s_card.status(), ToolStatus::Success);
    assert!(s_card.is_success());
    assert!(!s_card.is_failed());

    let f_card = ToolOutputCard::failure("bash", "test", "err", None);
    assert_eq!(f_card.status(), ToolStatus::Failed);
    assert!(!f_card.is_success());
    assert!(f_card.is_failed());
}
