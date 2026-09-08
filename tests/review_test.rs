//! Tests for the interactive diff review widget and session.
//!
//! Covers:
//! - Unified diff parsing across single and multiple files.
//! - Handling of git headers, plain diffs, new files, and deleted files.
//! - Hunk review state transitions (accept, reject, accept_all, reject_all).
//! - Navigation (next_hunk, prev_hunk, bounds checking).
//! - Patch generation for accepted hunks (partial acceptance, all accepted, all rejected).
//! - Summary reporting ((total, accepted, rejected)).
//! - TUI ReviewWidget rendering to buffer.

#[path = "../src/ui/review.rs"]
mod review;

use ratatui::{buffer::Buffer, layout::Rect, widgets::Widget};
use review::*;

// ============================================================================
// 1. Parsing Tests
// ============================================================================

#[test]
fn test_parse_single_file_single_hunk() {
    let diff = r#"diff --git a/src/main.rs b/src/main.rs
index abcdef1..1234567 100644
--- a/src/main.rs
+++ b/src/main.rs
@@ -1,4 +1,5 @@
 fn main() {
-    println!("hello");
+    println!("hello world");
+    println!("second line");
 }
"#;

    let session = ReviewSession::parse_unified_diff(diff);
    assert_eq!(session.len(), 1);
    assert_eq!(session.active_index, 0);

    let hunk = &session.hunks[0];
    assert_eq!(hunk.file_path, "src/main.rs");
    assert_eq!(hunk.old_start, 1);
    assert_eq!(hunk.old_len, 4);
    assert_eq!(hunk.new_start, 1);
    assert_eq!(hunk.new_len, 5);
    assert_eq!(hunk.state, DiffHunkReviewState::Pending);

    // Verify lines
    assert_eq!(hunk.lines.len(), 5);
    assert_eq!(
        hunk.lines[0],
        (ChangeKind::Context, "fn main() {".to_string())
    );
    assert_eq!(
        hunk.lines[1],
        (ChangeKind::Deletion, "    println!(\"hello\");".to_string())
    );
    assert_eq!(
        hunk.lines[2],
        (
            ChangeKind::Addition,
            "    println!(\"hello world\");".to_string()
        )
    );
    assert_eq!(
        hunk.lines[3],
        (
            ChangeKind::Addition,
            "    println!(\"second line\");".to_string()
        )
    );
    assert_eq!(hunk.lines[4], (ChangeKind::Context, "}".to_string()));

    assert_eq!(hunk.additions_count(), 2);
    assert_eq!(hunk.deletions_count(), 1);
}

#[test]
fn test_parse_single_file_multiple_hunks() {
    let diff = r#"--- a/src/lib.rs
+++ b/src/lib.rs
@@ -10,3 +10,4 @@
 line 10
-line 11 old
+line 11 new
+line 11.5 new
 line 12
@@ -50,4 +51,3 @@
 line 50
-line 51 removed
-line 52 removed
 line 53
"#;

    let session = ReviewSession::parse_unified_diff(diff);
    assert_eq!(session.len(), 2);

    let h0 = &session.hunks[0];
    assert_eq!(h0.file_path, "src/lib.rs");
    assert_eq!(h0.old_start, 10);
    assert_eq!(h0.old_len, 3);
    assert_eq!(h0.new_start, 10);
    assert_eq!(h0.new_len, 4);
    assert_eq!(h0.additions_count(), 2);
    assert_eq!(h0.deletions_count(), 1);

    let h1 = &session.hunks[1];
    assert_eq!(h1.file_path, "src/lib.rs");
    assert_eq!(h1.old_start, 50);
    assert_eq!(h1.old_len, 4);
    assert_eq!(h1.new_start, 51);
    assert_eq!(h1.new_len, 3);
    assert_eq!(h1.additions_count(), 0);
    assert_eq!(h1.deletions_count(), 2);
}

#[test]
fn test_parse_multiple_files() {
    let diff = r#"diff --git a/src/foo.rs b/src/foo.rs
--- a/src/foo.rs
+++ b/src/foo.rs
@@ -1,3 +1,3 @@
 context a
-foo_old
+foo_new
 context b
diff --git a/src/bar.rs b/src/bar.rs
--- a/src/bar.rs
+++ b/src/bar.rs
@@ -20,2 +20,3 @@
 bar_line_1
+bar_line_added
 bar_line_2
"#;

    let session = ReviewSession::parse_unified_diff(diff);
    assert_eq!(session.len(), 2);

    assert_eq!(session.hunks[0].file_path, "src/foo.rs");
    assert_eq!(session.hunks[0].old_start, 1);
    assert_eq!(session.hunks[0].new_start, 1);

    assert_eq!(session.hunks[1].file_path, "src/bar.rs");
    assert_eq!(session.hunks[1].old_start, 20);
    assert_eq!(session.hunks[1].new_start, 20);
}

#[test]
fn test_parse_new_and_deleted_files() {
    let diff = r#"diff --git a/src/new.rs b/src/new.rs
new file mode 100644
--- /dev/null
+++ b/src/new.rs
@@ -0,0 +1,2 @@
+pub fn new_fn() {}
+pub fn second_fn() {}
diff --git a/src/old.rs b/src/old.rs
deleted file mode 100644
--- a/src/old.rs
+++ /dev/null
@@ -1,2 +0,0 @@
-pub fn deleted_1() {}
-pub fn deleted_2() {}
"#;

    let session = ReviewSession::parse_unified_diff(diff);
    assert_eq!(session.len(), 2);

    let h_new = &session.hunks[0];
    assert_eq!(h_new.file_path, "src/new.rs");
    assert_eq!(h_new.old_start, 0);
    assert_eq!(h_new.old_len, 0);
    assert_eq!(h_new.new_start, 1);
    assert_eq!(h_new.new_len, 2);

    let h_del = &session.hunks[1];
    assert_eq!(h_del.file_path, "src/old.rs");
    assert_eq!(h_del.old_start, 1);
    assert_eq!(h_del.old_len, 2);
    assert_eq!(h_del.new_start, 0);
    assert_eq!(h_del.new_len, 0);
}

#[test]
fn test_parse_plain_diff_no_prefixes() {
    let diff = r#"--- file1.txt
+++ file1.txt
@@ -1 +1,2 @@
-single old line
+first new line
+second new line
"#;

    let session = ReviewSession::parse_unified_diff(diff);
    assert_eq!(session.len(), 1);
    assert_eq!(session.hunks[0].file_path, "file1.txt");
    assert_eq!(session.hunks[0].old_start, 1);
    assert_eq!(session.hunks[0].old_len, 1);
    assert_eq!(session.hunks[0].new_start, 1);
    assert_eq!(session.hunks[0].new_len, 2);
}

// ============================================================================
// 2. Review Decision & Navigation Tests
// ============================================================================

#[test]
fn test_hunk_acceptance_and_rejection() {
    let diff = r#"--- a/file.rs
+++ b/file.rs
@@ -1,2 +1,2 @@
-one
+1
 ctx
@@ -10,2 +10,2 @@
-two
+2
 ctx
@@ -20,2 +20,2 @@
-three
+3
 ctx
"#;

    let mut session = ReviewSession::parse_unified_diff(diff);
    assert_eq!(session.len(), 3);
    assert_eq!(session.summary(), (3, 0, 0));

    // Initially at index 0, pending
    assert_eq!(session.active_index, 0);
    assert_eq!(session.hunks[0].state, DiffHunkReviewState::Pending);

    // Accept first hunk
    session.accept_current();
    assert_eq!(session.hunks[0].state, DiffHunkReviewState::Accepted);
    assert_eq!(session.summary(), (3, 1, 0));

    // Move to next hunk and reject
    session.next_hunk();
    assert_eq!(session.active_index, 1);
    session.reject_current();
    assert_eq!(session.hunks[1].state, DiffHunkReviewState::Rejected);
    assert_eq!(session.summary(), (3, 1, 1));

    // Move to third hunk
    session.next_hunk();
    assert_eq!(session.active_index, 2);
    // Boundary test: next_hunk at end does not overflow
    session.next_hunk();
    assert_eq!(session.active_index, 2);

    // Accept third hunk
    session.accept_current();
    assert_eq!(session.hunks[2].state, DiffHunkReviewState::Accepted);
    assert_eq!(session.summary(), (3, 2, 1));

    // Navigate back to hunk 0
    session.prev_hunk();
    assert_eq!(session.active_index, 1);
    session.prev_hunk();
    assert_eq!(session.active_index, 0);
    // Boundary test: prev_hunk at start stays at 0
    session.prev_hunk();
    assert_eq!(session.active_index, 0);
}

#[test]
fn test_accept_all_and_reject_all() {
    let diff = r#"--- a/file.rs
+++ b/file.rs
@@ -1,2 +1,2 @@
-a
+b
 c
@@ -5,2 +5,2 @@
-x
+y
 z
"#;

    let mut session = ReviewSession::parse_unified_diff(diff);
    assert_eq!(session.summary(), (2, 0, 0));

    session.accept_all();
    assert_eq!(session.summary(), (2, 2, 0));
    assert!(session.hunks.iter().all(|h| h.state.is_accepted()));

    session.reject_all();
    assert_eq!(session.summary(), (2, 0, 2));
    assert!(session.hunks.iter().all(|h| h.state.is_rejected()));
}

// ============================================================================
// 3. Patch Generation Tests
// ============================================================================

#[test]
fn test_generate_accepted_patch_partial() {
    let diff = r#"--- a/src/feature.rs
+++ b/src/feature.rs
@@ -10,3 +10,4 @@
 context_1
-remove_me
+add_me
+extra_line
 context_2
@@ -30,3 +31,3 @@
 context_3
-bad_change
+worse_change
 context_4
--- a/src/other.rs
+++ b/src/other.rs
@@ -1,2 +1,2 @@
-old_other
+new_other
 context_5
"#;

    let mut session = ReviewSession::parse_unified_diff(diff);
    assert_eq!(session.len(), 3);

    // Accept hunk 0 (feature.rs first hunk)
    session.accept_current();

    // Reject hunk 1 (feature.rs second hunk)
    session.next_hunk();
    session.reject_current();

    // Accept hunk 2 (other.rs)
    session.next_hunk();
    session.accept_current();

    let patch = session.generate_accepted_patch();

    // Assert accepted changes are present
    assert!(patch.contains("--- a/src/feature.rs"));
    assert!(patch.contains("+++ b/src/feature.rs"));
    assert!(patch.contains("+add_me"));
    assert!(patch.contains("+extra_line"));
    assert!(patch.contains("-remove_me"));

    assert!(patch.contains("--- a/src/other.rs"));
    assert!(patch.contains("+++ b/src/other.rs"));
    assert!(patch.contains("-old_other"));
    assert!(patch.contains("+new_other"));

    // Assert rejected hunk is NOT present
    assert!(!patch.contains("bad_change"));
    assert!(!patch.contains("worse_change"));

    // The generated patch itself should parse into 2 hunks
    let parsed_back = ReviewSession::parse_unified_diff(&patch);
    assert_eq!(parsed_back.len(), 2);
    assert_eq!(parsed_back.hunks[0].file_path, "src/feature.rs");
    assert_eq!(parsed_back.hunks[1].file_path, "src/other.rs");
}

#[test]
fn test_generate_accepted_patch_none_accepted() {
    let diff = r#"--- a/src/foo.rs
+++ b/src/foo.rs
@@ -1,2 +1,2 @@
-foo
+bar
 ctx
"#;

    let mut session = ReviewSession::parse_unified_diff(diff);
    // Default is Pending
    assert_eq!(session.generate_accepted_patch(), "");

    session.reject_all();
    assert_eq!(session.generate_accepted_patch(), "");
}

#[test]
fn test_generate_accepted_patch_all_accepted() {
    let diff = r#"--- a/src/code.rs
+++ b/src/code.rs
@@ -1,3 +1,4 @@
 init();
-run_legacy();
+prepare();
+run_modern();
 shutdown();
"#;

    let mut session = ReviewSession::parse_unified_diff(diff);
    session.accept_all();

    let patch = session.generate_accepted_patch();
    assert!(patch.contains("--- a/src/code.rs"));
    assert!(patch.contains("+++ b/src/code.rs"));
    assert!(patch.contains("@@ -1,3 +1,4 @@"));
    assert!(patch.contains("-run_legacy();"));
    assert!(patch.contains("+prepare();"));
    assert!(patch.contains("+run_modern();"));
}

// ============================================================================
// 4. Widget Rendering Tests
// ============================================================================

#[test]
fn test_widget_render_pending_hunk() {
    let diff = r#"--- a/src/demo.rs
+++ b/src/demo.rs
@@ -5,3 +5,4 @@
  context_line
 -deleted_line
 +added_line_1
 +added_line_2
  context_end
"#;

    let session = ReviewSession::parse_unified_diff(diff);
    let widget = ReviewWidget::new(&session);

    let area = Rect::new(0, 0, 100, 20);
    let mut buf = Buffer::empty(area);

    widget.render(area, &mut buf);

    // Verify key content rendered onto the buffer
    let mut text = String::new();
    for y in 0..area.height {
        for x in 0..area.width {
            text.push_str(buf.get(x, y).symbol());
        }
        text.push('\n');
    }

    assert!(text.contains("src/demo.rs"));
    assert!(text.contains("[PENDING]"));
    assert!(text.contains("+added_line_1"));
    assert!(text.contains("-deleted_line"));
    assert!(text.contains("[y]"));
    assert!(text.contains("Accept"));
    assert!(text.contains("[n]"));
    assert!(text.contains("Reject"));
    assert!(text.contains("[q]"));
    assert!(text.contains("Finish"));
}

#[test]
fn test_widget_render_accepted_and_rejected_badges() {
    let diff = r#"--- a/src/a.rs
+++ b/src/a.rs
@@ -1,2 +1,2 @@
-a
+b
 ctx
"#;

    let mut session = ReviewSession::parse_unified_diff(diff);
    session.accept_current();

    let area = Rect::new(0, 0, 80, 20);
    let mut buf = Buffer::empty(area);

    ReviewWidget::new(&session).render(area, &mut buf);

    let mut text_acc = String::new();
    for y in 0..area.height {
        for x in 0..area.width {
            text_acc.push_str(buf.get(x, y).symbol());
        }
        text_acc.push('\n');
    }
    assert!(text_acc.contains("[ACCEPTED]"));

    session.reject_current();
    let mut buf_rej = Buffer::empty(area);
    ReviewWidget::new(&session).render(area, &mut buf_rej);

    let mut text_rej = String::new();
    for y in 0..area.height {
        for x in 0..area.width {
            text_rej.push_str(buf_rej.get(x, y).symbol());
        }
        text_rej.push('\n');
    }
    assert!(text_rej.contains("[REJECTED]"));
}

#[test]
fn test_widget_render_empty_session() {
    let session = ReviewSession::new(Vec::new());
    let area = Rect::new(0, 0, 80, 20);
    let mut buf = Buffer::empty(area);

    ReviewWidget::new(&session).render(area, &mut buf);

    let mut text = String::new();
    for y in 0..area.height {
        for x in 0..area.width {
            text.push_str(buf.get(x, y).symbol());
        }
        text.push('\n');
    }

    assert!(text.contains("No diff hunks to review"));
    assert!(text.contains("[q]"));
}
