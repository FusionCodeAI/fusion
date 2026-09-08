//! Comprehensive integration tests for SideBySideDiffViewer.
//!
//! Tests:
//! 1. Parsing of unified diffs (git headers, plain diffs, multi-file, multi-hunk).
//! 2. Side-by-side row alignment (paired modifications, addition-only, deletion-only, context).
//! 3. ANSI color rendering (red deletions, green additions, dim gray line numbers, reset).
//! 4. Empty diff handling (empty string, whitespace, header-only).
//! 5. Terminal resizing and column width allocation (narrow, wide, standard, overflow truncation).
//! 6. Git working tree diff querying helper.

#[path = "../src/ui/diff_viewer.rs"]
mod diff_viewer;

use diff_viewer::*;
use std::path::Path;

// ============================================================================
// 1. Parsing Tests
// ============================================================================

#[test]
fn test_parse_basic_git_diff() {
    let diff = r#"diff --git a/src/main.rs b/src/main.rs
index 1234567..89abcdef 100644
--- a/src/main.rs
+++ b/src/main.rs
@@ -10,3 +10,3 @@ fn main() {
-old_function();
+new_function();
 context_line();
"#;

    let default_viewer = SideBySideDiffViewer::new();
    assert!(default_viewer.is_empty());
    assert_eq!(default_viewer.total_rows(), 0);
    assert_eq!(default_viewer.files().len(), 0);

    let viewer = SideBySideDiffViewer::parse(diff);
    assert_eq!(viewer.files().len(), 1);
    assert_eq!(viewer.total_rows(), 2);

    let file = &viewer.files()[0];
    assert_eq!(file.display_path(), "src/main.rs");
    assert_eq!(file.rows.len(), 2);

    // Row 1: paired deletion and addition
    let row1 = &file.rows[0];
    assert_eq!(row1.left.change_type, ChangeType::Deletion);
    assert_eq!(row1.left.line_number, Some(10));
    assert_eq!(row1.left.content, "old_function();");

    assert_eq!(row1.right.change_type, ChangeType::Addition);
    assert_eq!(row1.right.line_number, Some(10));
    assert_eq!(row1.right.content, "new_function();");

    // Row 2: context line
    let row2 = &file.rows[1];
    assert_eq!(row2.left.change_type, ChangeType::Context);
    assert_eq!(row2.left.line_number, Some(11));
    assert_eq!(row2.left.content, "context_line();");

    assert_eq!(row2.right.change_type, ChangeType::Context);
    assert_eq!(row2.right.line_number, Some(11));
    assert_eq!(row2.right.content, "context_line();");
}

#[test]
fn test_parse_plain_unified_diff_without_git_header() {
    let diff = r#"--- old.txt
+++ new.txt
@@ -1,2 +1,2 @@
-alpha
+beta
 gamma
"#;

    let viewer = SideBySideDiffViewer::parse(diff);
    assert_eq!(viewer.files.len(), 1);
    let file = &viewer.files[0];
    assert_eq!(file.display_path(), "new.txt");
    assert_eq!(file.rows.len(), 2);
}

#[test]
fn test_parse_multi_file_diff() {
    let diff = r#"diff --git a/file1.rs b/file1.rs
--- a/file1.rs
+++ b/file1.rs
@@ -1,1 +1,1 @@
-one
+two
diff --git a/file2.rs b/file2.rs
--- a/file2.rs
+++ b/file2.rs
@@ -5,1 +5,1 @@
-foo
+bar
"#;

    let viewer = SideBySideDiffViewer::parse(diff);
    assert_eq!(viewer.files.len(), 2);
    assert_eq!(viewer.files[0].display_path(), "file1.rs");
    assert_eq!(viewer.files[1].display_path(), "file2.rs");
}

#[test]
fn test_parse_multi_hunk_diff() {
    let diff = r#"diff --git a/src/lib.rs b/src/lib.rs
--- a/src/lib.rs
+++ b/src/lib.rs
@@ -1,2 +1,2 @@
-first_old
+first_new
 context1
@@ -100,2 +100,2 @@
-second_old
+second_new
 context2
"#;

    let viewer = SideBySideDiffViewer::parse(diff);
    assert_eq!(viewer.files.len(), 1);
    let file = &viewer.files[0];
    assert_eq!(file.rows.len(), 4);

    assert_eq!(file.rows[0].left.line_number, Some(1));
    assert_eq!(file.rows[2].left.line_number, Some(100));
}

// ============================================================================
// 2. Alignment Tests
// ============================================================================

#[test]
fn test_alignment_deletions_only() {
    let diff = r#"--- a/test.rs
+++ b/test.rs
@@ -1,3 +1,0 @@
-del1
-del2
-del3
"#;

    let viewer = SideBySideDiffViewer::parse(diff);
    let file = &viewer.files[0];
    assert_eq!(file.rows.len(), 3);

    for (i, row) in file.rows.iter().enumerate() {
        assert_eq!(row.left.change_type, ChangeType::Deletion);
        assert_eq!(row.left.line_number, Some(i + 1));
        assert_eq!(row.right.change_type, ChangeType::Empty);
        assert_eq!(row.right.line_number, None);
        assert!(row.right.is_empty());
    }
}

#[test]
fn test_alignment_additions_only() {
    let diff = r#"--- a/test.rs
+++ b/test.rs
@@ -1,0 +1,3 @@
+add1
+add2
+add3
"#;

    let viewer = SideBySideDiffViewer::parse(diff);
    let file = &viewer.files[0];
    assert_eq!(file.rows.len(), 3);

    for (i, row) in file.rows.iter().enumerate() {
        assert_eq!(row.left.change_type, ChangeType::Empty);
        assert_eq!(row.left.line_number, None);
        assert_eq!(row.right.change_type, ChangeType::Addition);
        assert_eq!(row.right.line_number, Some(i + 1));
    }
}

#[test]
fn test_alignment_unbalanced_changes() {
    // 1 deletion, 3 additions
    let diff = r#"--- a/test.rs
+++ b/test.rs
@@ -10,1 +10,3 @@
-deleted_only
+added_1
+added_2
+added_3
"#;

    let viewer = SideBySideDiffViewer::parse(diff);
    let file = &viewer.files[0];
    assert_eq!(file.rows.len(), 3);

    // Row 1: deleted_only paired with added_1
    assert_eq!(file.rows[0].left.content, "deleted_only");
    assert_eq!(file.rows[0].right.content, "added_1");

    // Row 2: empty left paired with added_2
    assert!(file.rows[1].left.is_empty());
    assert_eq!(file.rows[1].right.content, "added_2");

    // Row 3: empty left paired with added_3
    assert!(file.rows[2].left.is_empty());
    assert_eq!(file.rows[2].right.content, "added_3");
}

// ============================================================================
// 3. Color & Terminal Rendering Tests
// ============================================================================

#[test]
fn test_render_terminal_colors_and_delimiters() {
    let diff = r#"diff --git a/src/main.rs b/src/main.rs
--- a/src/main.rs
+++ b/src/main.rs
@@ -10,2 +10,2 @@
-old_function();
+new_function();
"#;

    let rendered = render_diff_terminal(diff, 80);

    // Header box borders
    assert!(rendered.contains("╭─ diff: src/main.rs"));
    assert!(rendered.contains('╮'));
    assert!(rendered.contains('│'));
    assert!(rendered.contains('╰'));
    assert!(rendered.contains('┴'));
    assert!(rendered.contains('╯'));

    // Colors
    assert!(rendered.contains("\x1b[31m"), "Should contain red for deletions");
    assert!(rendered.contains("\x1b[32m"), "Should contain green for additions");
    assert!(rendered.contains("\x1b[90m"), "Should contain dim gray for line numbers");
    assert!(rendered.contains("\x1b[0m"), "Should reset ANSI styling");

    // Code content
    assert!(rendered.contains("- old_function();"));
    assert!(rendered.contains("+ new_function();"));
}

#[test]
fn test_render_exact_layout_structure() {
    let diff = r#"diff --git a/src/main.rs b/src/main.rs
--- a/src/main.rs
+++ b/src/main.rs
@@ -10,1 +10,1 @@
-old_function();
+new_function();
"#;

    let rendered = render_diff_terminal(diff, 64);
    let plain = strip_ansi(&rendered);

    // Verify each line in output has exact visible width of 64 columns
    for line in plain.lines() {
        assert_eq!(
            visible_width(line),
            64,
            "Line should match terminal width 64: {:?}",
            line
        );
    }

    // Verify row structure
    assert!(plain.contains("│ 10: - old_function();"));
    assert!(plain.contains("│ 10: + new_function();"));
}

// ============================================================================
// 4. Empty Diff Handling Tests
// ============================================================================

#[test]
fn test_empty_diff_handling() {
    // Completely empty
    let viewer = SideBySideDiffViewer::parse("");
    assert!(viewer.is_empty());
    assert_eq!(viewer.files.len(), 0);
    assert_eq!(render_diff_terminal("", 80), "");

    // Whitespace only
    let viewer_ws = SideBySideDiffViewer::parse("   \n\n\t  \r\n");
    assert!(viewer_ws.is_empty());
    assert_eq!(render_diff_terminal("   \n\n\t  ", 80), "");

    // Header with no hunks
    let header_only = "diff --git a/foo.txt b/foo.txt\nnew file mode 100644\n";
    let viewer_h = SideBySideDiffViewer::parse(header_only);
    assert!(viewer_h.is_empty());
    assert_eq!(render_diff_terminal(header_only, 80), "");
}

// ============================================================================
// 5. Terminal Resizing and Width Allocation Tests
// ============================================================================

#[test]
fn test_terminal_resizing_widths() {
    let diff = r#"--- a/test.rs
+++ b/test.rs
@@ -1,1 +1,1 @@
-short
+short
"#;

    for width in [40, 60, 80, 100, 120] {
        let rendered = render_diff_terminal(diff, width);
        let plain = strip_ansi(&rendered);

        for line in plain.lines() {
            assert_eq!(
                visible_width(line),
                width,
                "Rendered line width should equal terminal width {}",
                width
            );
        }
    }
}

#[test]
fn test_long_line_truncation_preserves_grid() {
    let long_line = "x".repeat(200);
    let diff = format!(
        "--- a/long.rs\n+++ b/long.rs\n@@ -1,1 +1,1 @@\n-{long_line}\n+{long_line}\n"
    );

    let rendered = render_diff_terminal(&diff, 80);
    let plain = strip_ansi(&rendered);

    for line in plain.lines() {
        assert_eq!(
            visible_width(line),
            80,
            "Truncated line should strictly conform to column boundary"
        );
    }
}

#[test]
fn test_terminal_width_zero_fallback() {
    let diff = r#"--- a/test.rs
+++ b/test.rs
@@ -1,1 +1,1 @@
-old
+new
"#;

    // Terminal width 0 defaults safely to 80
    let rendered = render_diff_terminal(diff, 0);
    let plain = strip_ansi(&rendered);
    for line in plain.lines() {
        assert_eq!(visible_width(line), 80);
    }
}

// ============================================================================
// 6. Git Working Tree Diff Querying Test
// ============================================================================

#[test]
fn test_git_working_tree_diff_live_repo() {
    let repo_root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let res = git_working_tree_diff(repo_root);
    assert!(res.is_ok(), "git_working_tree_diff should succeed on workspace root");
}

#[test]
fn test_git_working_tree_diff_invalid_dir() {
    let invalid_dir = Path::new("/nonexistent_directory_for_test_12345");
    let res = git_working_tree_diff(invalid_dir);
    assert!(res.is_err(), "git_working_tree_diff should fail on non-existent directory");
}
