//! Integration tests for fusion-edit hashline engine.
//!
//! Covers:
//! - Hashline patch parsing and application
//! - Single-line replacement (`PUT 1.=1:`)
//! - Block deletion (`CUT 5.=10`)
//! - Insertion before/after anchors (`PUT <1:`, `PUT >N:`)
//! - Multi-line replacements (`PUT N.=M:`)
//! - Named and anonymous clipboard cuts and pastes (`CUT ... @reg`, `PUT ... @reg`)
//! - Syntax tolerance, lenient separators, and invalid range error messages
//! - End-to-end integration via `fusion::tools::edit::apply_hashline_patch`

use fusion_edit::modes::hashline::{
    apply::{apply_edits, ApplyOptions, EmptyPaste},
    format::{
        format_cut_header, format_hashline_header, format_numbered_line, format_numbered_lines,
        format_replace_header,
    },
    input::{contains_recognizable_hashline_operations, Patch, SplitOptions},
    parser::{parse_patch, AbsoluteRangeOp, ParseFailure},
    types::{Anchor, Cursor, Edit},
};

fn default_options<'a>() -> SplitOptions<'a> {
    SplitOptions {
        cwd: None,
        path: None,
    }
}

// ============================================================================
// 1. Hashline Patch Parsing
// ============================================================================

#[test]
fn test_hashline_header_formatting_and_detection() {
    assert_eq!(
        format_hashline_header("src/main.rs", "A1B2"),
        "[src/main.rs#A1B2]"
    );
    assert_eq!(format_replace_header(1, 1), "PUT 1.=1:");
    assert_eq!(format_cut_header(5, 10), "CUT 5.=10");
    assert_eq!(format_numbered_line(1, "hello"), "1:hello");
    assert_eq!(format_numbered_lines("foo\nbar\n", 1), "1:foo\n2:bar\n3:");

    assert!(contains_recognizable_hashline_operations(
        "PUT 1.=1:\n+test"
    ));
    assert!(contains_recognizable_hashline_operations("CUT 5.=10"));
    assert!(contains_recognizable_hashline_operations("PUT <1:\n+first"));
    assert!(contains_recognizable_hashline_operations("PUT >5:\n+after"));
    assert!(!contains_recognizable_hashline_operations(
        "just plain text\nwithout ops"
    ));
}

#[test]
fn test_parse_patch_single_line_replacement() {
    let diff = "PUT 1.=1:\n+let x = 42;";
    let parsed = parse_patch(diff).expect("should parse PUT 1.=1:");

    assert_eq!(parsed.edits.len(), 2);
    // First edit is the insert of replacement text at line 1
    assert!(matches!(
        &parsed.edits[0],
        Edit::Insert {
            cursor: Cursor::BeforeAnchor(Anchor { line: 1 }),
            text,
            replacement: true,
            ..
        } if text == "let x = 42;"
    ));
    // Second edit is the deletion of line 1
    assert!(matches!(
        &parsed.edits[1],
        Edit::Delete {
            anchor: Anchor { line: 1 },
            ..
        }
    ));
}

#[test]
fn test_parse_patch_block_deletion() {
    let diff = "CUT 5.=10";
    let parsed = parse_patch(diff).expect("should parse CUT 5.=10");

    // Must contain Edit::Cut for lines 5..=10 and individual line deletions 5..=10
    let cut_edit = parsed
        .edits
        .iter()
        .find(|e| matches!(e, Edit::Cut { .. }))
        .expect("must contain an Edit::Cut");

    if let Edit::Cut {
        range, register, ..
    } = cut_edit
    {
        assert_eq!(range.start.line, 5);
        assert_eq!(range.end.line, 10);
        assert_eq!(*register, None);
    } else {
        panic!("expected Edit::Cut");
    }

    let deleted_lines: Vec<u32> = parsed
        .edits
        .iter()
        .filter_map(|e| match e {
            Edit::Delete { anchor, .. } => Some(anchor.line),
            _ => None,
        })
        .collect();

    assert_eq!(deleted_lines, vec![5, 6, 7, 8, 9, 10]);
}

#[test]
fn test_parse_multi_section_patch() {
    let patch_text = r#"
[src/a.rs#1111]
PUT 1.=1:
+// File A header

[src/b.rs#2222]
CUT 5.=10
"#;
    let patch =
        Patch::parse(patch_text, &default_options()).expect("should parse multi-section patch");
    assert_eq!(patch.sections.len(), 2);

    assert_eq!(patch.sections[0].path, "src/a.rs");
    assert_eq!(patch.sections[0].file_hash.as_deref(), Some("1111"));
    let parsed_a = patch.sections[0].parse().expect("section a should parse");
    assert!(parsed_a.edits.iter().any(|e| matches!(
        e,
        Edit::Insert {
            replacement: true,
            ..
        }
    )));

    assert_eq!(patch.sections[1].path, "src/b.rs");
    assert_eq!(patch.sections[1].file_hash.as_deref(), Some("2222"));
    let parsed_b = patch.sections[1].parse().expect("section b should parse");
    assert!(parsed_b.edits.iter().any(|e| matches!(e, Edit::Cut { .. })));
}

#[test]
fn test_parse_lenient_range_separators() {
    // Hashline supports tolerant range separators: .=, -, .., =, etc.
    for sep in [".=", "-", "..", "=", "…"] {
        let diff = format!("PUT 2{sep}3:\n+updated line");
        let parsed = parse_patch(&diff).unwrap_or_else(|e| panic!("failed on sep '{sep}': {e}"));
        assert_eq!(
            parsed
                .edits
                .iter()
                .filter(|e| matches!(e, Edit::Delete { .. }))
                .count(),
            2,
            "Separator '{sep}' should delete 2 lines (2 and 3)"
        );
    }
}

#[test]
fn test_parse_invalid_ranges_rejected() {
    // Inverted ranges should yield structured diagnostic
    let err = parse_patch("CUT 10.=5").expect_err("inverted range must fail");
    match err {
        ParseFailure::InvalidAbsoluteRange(range) => {
            assert_eq!(range.op, AbsoluteRangeOp::Cut);
            assert_eq!(range.start_line, 10);
            assert_eq!(range.end_line, 5);
        }
        _ => panic!("expected InvalidAbsoluteRange"),
    }

    let err_put = parse_patch("PUT 5.=2:\n+bad").expect_err("inverted range must fail");
    match err_put {
        ParseFailure::InvalidAbsoluteRange(range) => {
            assert_eq!(range.op, AbsoluteRangeOp::Replace);
            assert_eq!(range.start_line, 5);
            assert_eq!(range.end_line, 2);
        }
        _ => panic!("expected InvalidAbsoluteRange"),
    }
}

// ============================================================================
// 2. Hashline Patch Application
// ============================================================================

#[test]
fn test_apply_single_line_replacement_first_line() {
    let original = "line one\nline two\nline three\n";
    let diff = "PUT 1.=1:\n+FIRST LINE UPDATED";

    let parsed = parse_patch(diff).expect("parse failed");
    let result = apply_edits(
        original,
        &parsed.edits,
        ApplyOptions {
            clipboard: None,
            path: Some("test.txt"),
            on_empty_paste: EmptyPaste::Throw,
        },
    )
    .expect("apply failed");

    assert_eq!(result.text, "FIRST LINE UPDATED\nline two\nline three\n");
    assert_eq!(result.first_changed_line, Some(1));
}

#[test]
fn test_apply_single_line_replacement_middle_line() {
    let original = "fn start() {}\nfn compute() -> i32 { 0 }\nfn end() {}\n";
    let diff = "PUT 2.=2:\n+fn compute() -> i32 { 42 }";

    let parsed = parse_patch(diff).expect("parse failed");
    let result = apply_edits(
        original,
        &parsed.edits,
        ApplyOptions {
            clipboard: None,
            path: Some("main.rs"),
            on_empty_paste: EmptyPaste::Throw,
        },
    )
    .expect("apply failed");

    assert_eq!(
        result.text,
        "fn start() {}\nfn compute() -> i32 { 42 }\nfn end() {}\n"
    );
    assert_eq!(result.first_changed_line, Some(2));
}

#[test]
fn test_apply_block_deletion_middle() {
    // 15 lines
    let mut lines = Vec::new();
    for i in 1..=15 {
        lines.push(format!("line {i}"));
    }
    let original = format!("{}\n", lines.join("\n"));

    let diff = "CUT 5.=10";
    let parsed = parse_patch(diff).expect("parse failed");

    let result = apply_edits(
        &original,
        &parsed.edits,
        ApplyOptions {
            clipboard: None,
            path: Some("data.txt"),
            on_empty_paste: EmptyPaste::Throw,
        },
    )
    .expect("apply failed");

    let expected_lines = vec![
        "line 1", "line 2", "line 3", "line 4", // lines 5..=10 cut
        "line 11", "line 12", "line 13", "line 14", "line 15",
    ];
    let expected = format!("{}\n", expected_lines.join("\n"));
    assert_eq!(result.text, expected);
    assert_eq!(result.first_changed_line, Some(5));
}

#[test]
fn test_apply_combined_put_and_cut() {
    let original = "header\nkeep_1\nto_delete_1\nto_delete_2\nto_delete_3\nold_footer\n";

    // Replace line 1 with new header, cut lines 3..=5, replace line 6
    let diff = "PUT 1.=1:\n+NEW HEADER\nCUT 3.=5\nPUT 6.=6:\n+NEW FOOTER";
    let parsed = parse_patch(diff).expect("parse failed");

    let result = apply_edits(
        original,
        &parsed.edits,
        ApplyOptions {
            clipboard: None,
            path: Some("combined.txt"),
            on_empty_paste: EmptyPaste::Throw,
        },
    )
    .expect("apply failed");

    assert_eq!(result.text, "NEW HEADER\nkeep_1\nNEW FOOTER\n");
}

#[test]
fn test_apply_insertions_before_and_after() {
    let original = "line 1\nline 2\nline 3\n";

    // Insert before line 1 (BOF) and insert after line 3 (EOF)
    let diff = "PUT <1:\n+top of file\nPUT >3:\n+bottom of file";
    let parsed = parse_patch(diff).expect("parse failed");

    let result = apply_edits(
        original,
        &parsed.edits,
        ApplyOptions {
            clipboard: None,
            path: Some("insert.txt"),
            on_empty_paste: EmptyPaste::Throw,
        },
    )
    .expect("apply failed");

    assert_eq!(
        result.text,
        "top of file\nline 1\nline 2\nline 3\nbottom of file\n"
    );
}

#[test]
fn test_apply_clipboard_cut_and_paste_register() {
    let original = "1: first\n2: cut me\n3: cut me too\n4: third\n5: destination\n";
    let diff = "CUT 2.=3 @stash\nPUT >5 @stash";

    let parsed = parse_patch(diff).expect("parse failed");
    let result = apply_edits(
        original,
        &parsed.edits,
        ApplyOptions {
            clipboard: None,
            path: Some("clipboard.txt"),
            on_empty_paste: EmptyPaste::Throw,
        },
    )
    .expect("apply failed");

    assert_eq!(
        result.text,
        "1: first\n4: third\n5: destination\n2: cut me\n3: cut me too\n"
    );
}

// ============================================================================
// 3. End-to-End Filesystem Integration via fusion::tools::edit
// ============================================================================

#[tokio::test]
async fn test_end_to_end_apply_hashline_patch() {
    let temp_dir = tempfile::tempdir().expect("failed to create temp dir");
    let cwd = temp_dir.path();

    let target_file = cwd.join("sample.rs");
    let original_content = r#"fn main() {
    let old_var = 10;
    // dead code start
    let unused_1 = 1;
    let unused_2 = 2;
    let unused_3 = 3;
    let unused_4 = 4;
    let unused_5 = 5;
    let unused_6 = 6;
    // dead code end
    println!("done");
}
"#;
    tokio::fs::write(&target_file, original_content)
        .await
        .expect("write sample.rs");

    // Single-line replacement on line 2 (PUT 2.=2:)
    // and block deletion on lines 3..=10 (CUT 3.=10)
    let patch = r#"[sample.rs]
PUT 2.=2:
+    let new_var = 20;
CUT 3.=10
"#;

    assert!(fusion::tools::edit::is_hashline_input(patch));

    let output = fusion::tools::edit::apply_hashline_patch(patch, None, cwd)
        .await
        .expect("apply_hashline_patch failed");

    assert!(output.contains("Successfully edited 'sample.rs'"));

    let updated_content = tokio::fs::read_to_string(&target_file)
        .await
        .expect("read sample.rs");

    let expected_content = r#"fn main() {
    let new_var = 20;
    println!("done");
}
"#;
    assert_eq!(updated_content, expected_content);
}

#[tokio::test]
async fn test_end_to_end_hashline_headerless_with_default_path() {
    let temp_dir = tempfile::tempdir().expect("failed to create temp dir");
    let cwd = temp_dir.path();

    let file_path = cwd.join("config.toml");
    tokio::fs::write(&file_path, "name = \"old\"\nversion = \"1.0\"\n")
        .await
        .expect("write config.toml");

    let patch = "PUT 1.=1:\n+name = \"fusion\"";
    assert!(fusion::tools::edit::is_hashline_input(patch));

    let output = fusion::tools::edit::apply_hashline_patch(patch, Some("config.toml"), cwd)
        .await
        .expect("headerless apply failed");

    assert!(output.contains("Successfully edited 'config.toml'"));

    let updated = tokio::fs::read_to_string(&file_path)
        .await
        .expect("read config.toml");
    assert_eq!(updated, "name = \"fusion\"\nversion = \"1.0\"\n");
}
