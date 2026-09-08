//! Comprehensive unit and integration tests for `EditorBuffer` and `EditorWidget`.
//!
//! Tests:
//! 1. Buffer initialization from empty text, single line, multi-line, and Windows CRLF.
//! 2. Text insertion at start, middle, end of lines, and multi-byte UTF-8.
//! 3. Newline splitting and line insertion across various cursor positions.
//! 4. Backspace joining and forward deletion line merging.
//! 5. Cursor movement navigation and boundary clamping.
//! 6. Text export and round-trip consistency.
//! 7. File saving and modification state tracking.
//! 8. Viewport scrolling adjustment for 2D coordinate visibility.
//! 9. EditorWidget ratatui rendering (gutter line numbers, syntax highlighting, status bar).
//! 10. Language detection and syntax tokenization.

#[path = "../src/ui/editor.rs"]
pub mod editor;

use editor::{
    detect_language, tokenize_line, CursorMove, EditorBuffer, EditorWidget, Language, TokenKind,
};
use ratatui::{buffer::Buffer, layout::Rect, style::Color, widgets::Widget};
use std::fs;
use std::path::Path;

// ============================================================================
// 1. Buffer Initialization Tests
// ============================================================================

#[test]
fn test_buffer_initialization_empty() {
    let buf = EditorBuffer::new();
    assert_eq!(buf.lines.len(), 1);
    assert_eq!(buf.lines[0], "");
    assert_eq!(buf.cursor_row, 0);
    assert_eq!(buf.cursor_col, 0);
    assert_eq!(buf.scroll_offset_row, 0);
    assert_eq!(buf.scroll_offset_col, 0);
    assert!(!buf.is_modified);
    assert!(buf.file_path.is_none());
    assert!(buf.is_empty());
    assert_eq!(buf.to_text(), "");
}

#[test]
fn test_buffer_initialization_multiline() {
    let text = "alpha\nbeta\ngamma";
    let buf = EditorBuffer::from_text(text);
    assert_eq!(buf.line_count(), 3);
    assert_eq!(buf.lines[0], "alpha");
    assert_eq!(buf.lines[1], "beta");
    assert_eq!(buf.lines[2], "gamma");
    assert_eq!(buf.cursor_row, 0);
    assert_eq!(buf.cursor_col, 0);
    assert!(!buf.is_modified);
    assert_eq!(buf.to_text(), text);
}

#[test]
fn test_buffer_initialization_crlf_normalization() {
    let text = "hello\r\nworld\r\nagain";
    let buf = EditorBuffer::from_text(text);
    assert_eq!(buf.line_count(), 3);
    assert_eq!(buf.lines[0], "hello");
    assert_eq!(buf.lines[1], "world");
    assert_eq!(buf.lines[2], "again");
    assert_eq!(buf.to_text(), "hello\nworld\nagain");
}

// ============================================================================
// 2. Text Insertion Tests
// ============================================================================

#[test]
fn test_text_insertion_empty_buffer() {
    let mut buf = EditorBuffer::new();
    buf.insert_char('H');
    buf.insert_char('i');
    assert_eq!(buf.lines[0], "Hi");
    assert_eq!(buf.cursor_row, 0);
    assert_eq!(buf.cursor_col, 2);
    assert!(buf.is_modified);
    assert_eq!(buf.to_text(), "Hi");
}

#[test]
fn test_text_insertion_middle_and_end() {
    let mut buf = EditorBuffer::from_text("world");
    // Insert at start
    buf.move_to_line_start();
    buf.insert_str("hello ");
    assert_eq!(buf.lines[0], "hello world");
    assert_eq!(buf.cursor_col, 6);

    // Insert in middle
    buf.insert_str("brave ");
    assert_eq!(buf.lines[0], "hello brave world");
    assert_eq!(buf.cursor_col, 12);

    // Insert at end
    buf.move_to_line_end();
    buf.insert_char('!');
    assert_eq!(buf.lines[0], "hello brave world!");
    assert_eq!(buf.cursor_col, 18);
}

#[test]
fn test_text_insertion_utf8_multibyte() {
    let mut buf = EditorBuffer::new();
    buf.insert_str("こんにちは"); // 5 Japanese characters
    assert_eq!(buf.lines[0], "こんにちは");
    assert_eq!(buf.cursor_col, 5);

    // Move cursor to character index 2 (after 'ん')
    buf.cursor_col = 2;
    buf.insert_char('✨'); // multi-byte emoji
    assert_eq!(buf.lines[0], "こん✨にちは");
    assert_eq!(buf.cursor_col, 3);
}

// ============================================================================
// 3. Newline Splitting Tests
// ============================================================================

#[test]
fn test_newline_splitting_middle() {
    let mut buf = EditorBuffer::from_text("foobar");
    buf.cursor_row = 0;
    buf.cursor_col = 3; // between "foo" and "bar"

    buf.insert_newline();
    assert_eq!(buf.line_count(), 2);
    assert_eq!(buf.lines[0], "foo");
    assert_eq!(buf.lines[1], "bar");
    assert_eq!(buf.cursor_row, 1);
    assert_eq!(buf.cursor_col, 0);
    assert!(buf.is_modified);
    assert_eq!(buf.to_text(), "foo\nbar");
}

#[test]
fn test_newline_splitting_start() {
    let mut buf = EditorBuffer::from_text("start");
    buf.cursor_row = 0;
    buf.cursor_col = 0;

    buf.insert_newline();
    assert_eq!(buf.line_count(), 2);
    assert_eq!(buf.lines[0], "");
    assert_eq!(buf.lines[1], "start");
    assert_eq!(buf.cursor_row, 1);
    assert_eq!(buf.cursor_col, 0);
    assert_eq!(buf.to_text(), "\nstart");
}

#[test]
fn test_newline_splitting_end() {
    let mut buf = EditorBuffer::from_text("end");
    buf.move_to_line_end();
    assert_eq!(buf.cursor_col, 3);

    buf.insert_newline();
    assert_eq!(buf.line_count(), 2);
    assert_eq!(buf.lines[0], "end");
    assert_eq!(buf.lines[1], "");
    assert_eq!(buf.cursor_row, 1);
    assert_eq!(buf.cursor_col, 0);
    assert_eq!(buf.to_text(), "end\n");
}

#[test]
fn test_insert_char_newline() {
    let mut buf = EditorBuffer::from_text("test");
    buf.cursor_col = 2;
    buf.insert_char('\n');
    assert_eq!(buf.lines[0], "te");
    assert_eq!(buf.lines[1], "st");
    assert_eq!(buf.cursor_row, 1);
    assert_eq!(buf.cursor_col, 0);
}

// ============================================================================
// 4. Backspace and Forward Deletion Tests
// ============================================================================

#[test]
fn test_backspace_middle_of_line() {
    let mut buf = EditorBuffer::from_text("rust");
    buf.cursor_col = 3; // after 's'

    buf.delete_backwards();
    assert_eq!(buf.lines[0], "rut");
    assert_eq!(buf.cursor_col, 2);
    assert!(buf.is_modified);
}

#[test]
fn test_backspace_joining_lines() {
    let mut buf = EditorBuffer::from_text("hello\nworld");
    buf.cursor_row = 1;
    buf.cursor_col = 0; // start of second line

    buf.delete_backwards();
    assert_eq!(buf.line_count(), 1);
    assert_eq!(buf.lines[0], "helloworld");
    assert_eq!(buf.cursor_row, 0);
    assert_eq!(buf.cursor_col, 5); // joined at boundary
    assert!(buf.is_modified);
}

#[test]
fn test_backspace_at_origin_is_noop() {
    let mut buf = EditorBuffer::from_text("hello");
    buf.cursor_row = 0;
    buf.cursor_col = 0;

    buf.delete_backwards();
    assert_eq!(buf.lines[0], "hello");
    assert_eq!(buf.cursor_row, 0);
    assert_eq!(buf.cursor_col, 0);
    assert!(!buf.is_modified);
}

#[test]
fn test_backspace_utf8_multibyte() {
    let mut buf = EditorBuffer::from_text("こん✨にちは");
    buf.cursor_col = 3; // after emoji '✨'

    buf.delete_backwards();
    assert_eq!(buf.lines[0], "こんにちは");
    assert_eq!(buf.cursor_col, 2);
}

#[test]
fn test_delete_forwards_middle() {
    let mut buf = EditorBuffer::from_text("cargo");
    buf.cursor_col = 1; // at 'a'

    buf.delete_forwards();
    assert_eq!(buf.lines[0], "crgo");
    assert_eq!(buf.cursor_col, 1);
    assert!(buf.is_modified);
}

#[test]
fn test_delete_forwards_line_joining() {
    let mut buf = EditorBuffer::from_text("foo\nbar");
    buf.cursor_row = 0;
    buf.cursor_col = 3; // at end of first line

    buf.delete_forwards();
    assert_eq!(buf.line_count(), 1);
    assert_eq!(buf.lines[0], "foobar");
    assert_eq!(buf.cursor_row, 0);
    assert_eq!(buf.cursor_col, 3);
}

#[test]
fn test_delete_forwards_at_end_of_buffer_is_noop() {
    let mut buf = EditorBuffer::from_text("final");
    buf.cursor_row = 0;
    buf.cursor_col = 5;

    buf.delete_forwards();
    assert_eq!(buf.lines[0], "final");
    assert_eq!(buf.cursor_col, 5);
    assert!(!buf.is_modified);
}

// ============================================================================
// 5. Cursor Movement & Clamping Tests
// ============================================================================

#[test]
fn test_cursor_movement_cardinal() {
    let mut buf = EditorBuffer::from_text("line1\nlongerline2\nl3");

    // Down
    buf.move_cursor(CursorMove::Down);
    assert_eq!(buf.cursor_row, 1);
    assert_eq!(buf.cursor_col, 0);

    // Right
    buf.move_cursor(CursorMove::Right);
    buf.move_cursor(CursorMove::Right);
    assert_eq!(buf.cursor_col, 2);

    // Up
    buf.move_cursor(CursorMove::Up);
    assert_eq!(buf.cursor_row, 0);
    assert_eq!(buf.cursor_col, 2);

    // Left
    buf.move_cursor(CursorMove::Left);
    assert_eq!(buf.cursor_col, 1);
}

#[test]
fn test_cursor_boundary_clamping_on_vertical_move() {
    let mut buf = EditorBuffer::from_text("very long line here\nshort");
    buf.cursor_row = 0;
    buf.cursor_col = 15;

    // Moving down to a line with only 5 characters clamps column to 5
    buf.move_cursor(CursorMove::Down);
    assert_eq!(buf.cursor_row, 1);
    assert_eq!(buf.cursor_col, 5);

    // Moving down again at last line does nothing
    buf.move_cursor(CursorMove::Down);
    assert_eq!(buf.cursor_row, 1);
    assert_eq!(buf.cursor_col, 5);

    // Moving up to longer line keeps column 5
    buf.move_cursor(CursorMove::Up);
    assert_eq!(buf.cursor_row, 0);
    assert_eq!(buf.cursor_col, 5);

    // Moving up at first line stays at row 0
    buf.move_cursor(CursorMove::Up);
    assert_eq!(buf.cursor_row, 0);
    assert_eq!(buf.cursor_col, 5);
}

#[test]
fn test_cursor_horizontal_line_wrapping() {
    let mut buf = EditorBuffer::from_text("ab\ncd");

    // Move to end of line 0
    buf.cursor_row = 0;
    buf.cursor_col = 2;

    // Moving right at end of line wraps to col 0 of line 1
    buf.move_cursor(CursorMove::Right);
    assert_eq!(buf.cursor_row, 1);
    assert_eq!(buf.cursor_col, 0);

    // Moving left at col 0 wraps to end of line 0
    buf.move_cursor(CursorMove::Left);
    assert_eq!(buf.cursor_row, 0);
    assert_eq!(buf.cursor_col, 2);
}

#[test]
fn test_cursor_home_and_end() {
    let mut buf = EditorBuffer::from_text("hello world");
    buf.cursor_col = 5;

    buf.move_to_line_start();
    assert_eq!(buf.cursor_col, 0);

    buf.move_to_line_end();
    assert_eq!(buf.cursor_col, 11);
}

#[test]
fn test_cursor_buffer_start_and_end() {
    let mut buf = EditorBuffer::from_text("line1\nline2\nline3");
    buf.cursor_row = 1;
    buf.cursor_col = 2;

    buf.move_to_buffer_start();
    assert_eq!(buf.cursor_row, 0);
    assert_eq!(buf.cursor_col, 0);

    buf.move_to_buffer_end();
    assert_eq!(buf.cursor_row, 2);
    assert_eq!(buf.cursor_col, 5);
}

#[test]
fn test_page_up_page_down() {
    let lines: Vec<String> = (0..50).map(|i| format!("Line {}", i)).collect();
    let mut buf = EditorBuffer::from_text(&lines.join("\n"));

    buf.move_cursor(CursorMove::PageDown);
    assert_eq!(buf.cursor_row, 10);

    buf.move_cursor(CursorMove::PageDown);
    assert_eq!(buf.cursor_row, 20);

    buf.move_cursor(CursorMove::PageUp);
    assert_eq!(buf.cursor_row, 10);

    buf.move_cursor(CursorMove::PageUp);
    assert_eq!(buf.cursor_row, 0);
}

// ============================================================================
// 6. Text Export & Round-Trip Tests
// ============================================================================

#[test]
fn test_round_trip_export() {
    let test_cases = vec![
        "",
        "single line",
        "two\nlines",
        "multiple\nempty\n\nlines\n",
        "trailing newline\n",
        "emoji 🚀 and tabs    spaces",
    ];

    for text in test_cases {
        let buf = EditorBuffer::from_text(text);
        assert_eq!(buf.to_text(), text);
    }
}

// ============================================================================
// 7. File Saving & Modification Tests
// ============================================================================

#[test]
fn test_save_to_file_and_reload() {
    let temp_dir = tempfile::tempdir().expect("Failed to create tempdir");
    let file_path = temp_dir.path().join("sub").join("test_file.rs");

    let mut buf = EditorBuffer::from_text("fn main() {\n    println!(\"hi\");\n}\n");
    assert!(!buf.is_modified);

    buf.insert_str("// comment\n");
    assert!(buf.is_modified);

    // Save with explicit path
    buf.save_to_file(Some(&file_path)).expect("Save failed");
    assert!(!buf.is_modified);
    assert_eq!(buf.file_path, Some(file_path.clone()));

    // Verify written content matches
    let content = fs::read_to_string(&file_path).expect("Read failed");
    assert_eq!(content, buf.to_text());

    // Save again with None (using stored path)
    buf.insert_char('!');
    assert!(buf.is_modified);
    buf.save_to_file(None).expect("Subsequent save failed");
    assert!(!buf.is_modified);

    let reloaded = EditorBuffer::from_file(&file_path).expect("Load failed");
    assert_eq!(reloaded.to_text(), buf.to_text());
}

#[test]
fn test_save_to_file_without_path_errors() {
    let mut buf = EditorBuffer::from_text("text");
    let res = buf.save_to_file(None);
    assert!(res.is_err());
    assert_eq!(res.unwrap_err().kind(), std::io::ErrorKind::NotFound);
}

// ============================================================================
// 8. Viewport Scrolling Tests
// ============================================================================

#[test]
fn test_viewport_scrolling_adjust() {
    let lines: Vec<String> = (0..30).map(|i| format!("Line {}", i)).collect();
    let mut buf = EditorBuffer::from_text(&lines.join("\n"));

    // Viewport: 10 rows high, 40 cols wide
    buf.adjust_scroll(40, 10);
    assert_eq!(buf.scroll_offset_row, 0);

    // Move cursor past viewport bottom (to line 12)
    buf.cursor_row = 12;
    buf.adjust_scroll(40, 10);
    assert_eq!(buf.scroll_offset_row, 3); // 12 - 10 + 1 = 3

    // Move cursor back to top
    buf.cursor_row = 1;
    buf.adjust_scroll(40, 10);
    assert_eq!(buf.scroll_offset_row, 1);

    // Horizontal scroll
    buf.lines[0] = "a".repeat(100);
    buf.cursor_row = 0;
    buf.cursor_col = 50;
    buf.adjust_scroll(40, 10);
    assert_eq!(buf.scroll_offset_col, 11); // 50 - 40 + 1 = 11
}

// ============================================================================
// 9. EditorWidget Ratatui Rendering Tests
// ============================================================================

#[test]
fn test_widget_rendering_empty_buffer() {
    let buf = EditorBuffer::new();
    let widget = EditorWidget::new(&buf);

    let mut render_buf = Buffer::empty(Rect::new(0, 0, 80, 24));
    widget.render(Rect::new(0, 0, 80, 24), &mut render_buf);

    // Check line number gutter in dimmed grey at top left
    let cell_gutter = render_buf.cell((0, 0)).expect("Cell 0,0 must exist");
    assert_eq!(cell_gutter.style().fg, Some(Color::DarkGray));

    // Check status bar at bottom row (y = 23)
    let mut status_row_text = String::new();
    for x in 0..80 {
        if let Some(c) = render_buf.cell((x, 23)) {
            status_row_text.push_str(c.symbol());
        }
    }

    assert!(status_row_text.contains("[No Name]"));
    assert!(status_row_text.contains("Ln 1, Col 1"));
    assert!(status_row_text.contains("Ctrl+S: Save | Esc: Exit"));
}

#[test]
fn test_widget_rendering_with_filename_and_modified() {
    let mut buf = EditorBuffer::from_text("let x = 42;");
    buf.file_path = Some(Path::new("src/main.rs").to_path_buf());
    buf.is_modified = true;

    let widget = EditorWidget::new(&buf).with_status_message("Unit test active");
    let mut render_buf = Buffer::empty(Rect::new(0, 0, 80, 24));
    widget.render(Rect::new(0, 0, 80, 24), &mut render_buf);

    let mut status_text = String::new();
    for x in 0..80 {
        if let Some(c) = render_buf.cell((x, 23)) {
            status_text.push_str(c.symbol());
        }
    }

    assert!(status_text.contains("main.rs"));
    assert!(status_text.contains("[+]"));
    assert!(status_text.contains("Unit test active"));
    assert!(status_text.contains("Ctrl+S: Save | Esc: Exit"));
}

#[test]
fn test_widget_rendering_dimmed_gutter_numbers() {
    let buf = EditorBuffer::from_text("line 1\nline 2\nline 3");
    let widget = EditorWidget::new(&buf);

    let mut render_buf = Buffer::empty(Rect::new(0, 0, 40, 10));
    widget.render(Rect::new(0, 0, 40, 10), &mut render_buf);

    // Row 0 has line number 1 in DarkGray
    let c0 = render_buf.cell((0, 0)).unwrap();
    assert_eq!(c0.style().fg, Some(Color::DarkGray));

    // Row 1 has line number 2 in DarkGray
    let c1 = render_buf.cell((0, 1)).unwrap();
    assert_eq!(c1.style().fg, Some(Color::DarkGray));

    // Row 2 has line number 3 in DarkGray
    let c2 = render_buf.cell((0, 2)).unwrap();
    assert_eq!(c2.style().fg, Some(Color::DarkGray));

    // Row 3 (past content) has '~' in DarkGray
    let c3 = render_buf.cell((0, 3)).unwrap();
    assert_eq!(c3.style().fg, Some(Color::DarkGray));
}

// ============================================================================
// 10. Language Detection & Syntax Tokenization Tests
// ============================================================================

#[test]
fn test_language_detection() {
    assert_eq!(detect_language(Some(Path::new("file.rs"))), Language::Rust);
    assert_eq!(detect_language(Some(Path::new("app.py"))), Language::Python);
    assert_eq!(
        detect_language(Some(Path::new("index.ts"))),
        Language::TypeScript
    );
    assert_eq!(
        detect_language(Some(Path::new("script.js"))),
        Language::JavaScript
    );
    assert_eq!(
        detect_language(Some(Path::new("config.toml"))),
        Language::Toml
    );
    assert_eq!(
        detect_language(Some(Path::new("data.json"))),
        Language::Json
    );
    assert_eq!(
        detect_language(Some(Path::new("README.md"))),
        Language::Markdown
    );
    assert_eq!(detect_language(Some(Path::new("run.sh"))), Language::Shell);
    assert_eq!(detect_language(Some(Path::new("main.c"))), Language::C);
    assert_eq!(detect_language(Some(Path::new("main.go"))), Language::Go);
    assert_eq!(
        detect_language(Some(Path::new("unknown.xyz"))),
        Language::Plain
    );
    assert_eq!(detect_language(None), Language::Plain);
}

#[test]
fn test_syntax_tokenization_rust() {
    let line = "pub fn add(a: usize) -> usize { // adds one";
    let tokens = tokenize_line(line, Language::Rust);

    // Verify keywords
    assert!(tokens
        .iter()
        .any(|t| t.kind == TokenKind::Keyword && t.text == "pub"));
    assert!(tokens
        .iter()
        .any(|t| t.kind == TokenKind::Keyword && t.text == "fn"));

    // Verify function name
    assert!(tokens
        .iter()
        .any(|t| t.kind == TokenKind::Function && t.text == "add"));

    // Verify types
    assert!(tokens
        .iter()
        .any(|t| t.kind == TokenKind::Type && t.text == "usize"));

    // Verify comments
    assert!(tokens
        .iter()
        .any(|t| t.kind == TokenKind::Comment && t.text.contains("adds one")));
}

#[test]
fn test_syntax_tokenization_python() {
    let line = "def greet(name: str = 'world'): # greeting";
    let tokens = tokenize_line(line, Language::Python);

    assert!(tokens
        .iter()
        .any(|t| t.kind == TokenKind::Keyword && t.text == "def"));
    assert!(tokens
        .iter()
        .any(|t| t.kind == TokenKind::Function && t.text == "greet"));
    assert!(tokens
        .iter()
        .any(|t| t.kind == TokenKind::StringLiteral && t.text == "'world'"));
    assert!(tokens
        .iter()
        .any(|t| t.kind == TokenKind::Comment && t.text.contains("greeting")));
}

#[test]
fn test_syntax_tokenization_numbers_and_strings() {
    let line = "let x = 42 + 3.14 + 0xff; let s = \"string\\\"value\";";
    let tokens = tokenize_line(line, Language::Rust);

    assert!(tokens
        .iter()
        .any(|t| t.kind == TokenKind::NumberLiteral && t.text == "42"));
    assert!(tokens
        .iter()
        .any(|t| t.kind == TokenKind::NumberLiteral && t.text == "3.14"));
    assert!(tokens
        .iter()
        .any(|t| t.kind == TokenKind::NumberLiteral && t.text == "0xff"));
    assert!(tokens
        .iter()
        .any(|t| t.kind == TokenKind::StringLiteral && t.text == "\"string\\\"value\""));
}
