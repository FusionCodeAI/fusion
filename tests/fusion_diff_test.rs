//! Integration tests for `fusion-diff`.
//!
//! Comprehensive test suite verifying:
//! 1. Line-by-line diffing via `fusion_diff::line_changes_str`
//! 2. Structured patch hunks via `fusion_diff::structured_patch_hunks_u16`
//! 3. Line runs via `fusion_diff::line_runs_str`
//! 4. Cross-engine invariants and edge-case handling

use fusion_diff::{
    diff_lines_u16, line_changes_str, line_runs_str, line_tokens_str, line_tokens_u16,
    structured_patch_hunks_from_runs_u16, structured_patch_hunks_u16, Change, Hunk, Run,
};

/// Helper to encode a string as UTF-16 code units.
fn to_u16(s: &str) -> Vec<u16> {
    s.encode_utf16().collect()
}

/// Helper to convert a hunk line (UTF-16) to a Rust String.
fn hunk_line_to_str(line: &[u16]) -> String {
    String::from_utf16(line).expect("valid UTF-16 line")
}

/// Helper to convert an entire Hunk's lines to Strings.
fn hunk_lines_to_strings(hunk: &Hunk) -> Vec<String> {
    hunk.lines.iter().map(|l| hunk_line_to_str(l)).collect()
}

// ============================================================================
// 1. Line-by-Line Diffing (`fusion_diff::line_changes_str`)
// ============================================================================

#[test]
fn test_line_changes_empty_strings() {
    let changes = line_changes_str("", "");
    assert!(
        changes.is_empty(),
        "diffing two empty strings should produce no changes"
    );
}

#[test]
fn test_line_changes_identical_strings() {
    let text = "alpha\nbeta\ngamma\n";
    let changes = line_changes_str(text, text);
    assert_eq!(changes.len(), 1);
    assert_eq!(
        changes[0],
        Change {
            value: text.to_string(),
            count: 3,
            added: false,
            removed: false,
        }
    );
}

#[test]
fn test_line_changes_pure_insertion() {
    let old = "";
    let new = "first line\nsecond line\n";
    let changes = line_changes_str(old, new);
    assert_eq!(changes.len(), 1);
    assert_eq!(
        changes[0],
        Change {
            value: new.to_string(),
            count: 2,
            added: true,
            removed: false,
        }
    );
}

#[test]
fn test_line_changes_pure_deletion() {
    let old = "foo\nbar\nbaz\n";
    let new = "";
    let changes = line_changes_str(old, new);
    assert_eq!(changes.len(), 1);
    assert_eq!(
        changes[0],
        Change {
            value: old.to_string(),
            count: 3,
            added: false,
            removed: true,
        }
    );
}

#[test]
fn test_line_changes_single_line_replacement() {
    let old = "header\nold_value\nfooter\n";
    let new = "header\nnew_value\nfooter\n";
    let changes = line_changes_str(old, new);

    assert_eq!(changes.len(), 4);
    // 1. Common "header\n"
    assert_eq!(
        changes[0],
        Change {
            value: "header\n".to_string(),
            count: 1,
            added: false,
            removed: false,
        }
    );
    // 2. Removed "old_value\n"
    assert_eq!(
        changes[1],
        Change {
            value: "old_value\n".to_string(),
            count: 1,
            added: false,
            removed: true,
        }
    );
    // 3. Added "new_value\n"
    assert_eq!(
        changes[2],
        Change {
            value: "new_value\n".to_string(),
            count: 1,
            added: true,
            removed: false,
        }
    );
    // 4. Common "footer\n"
    assert_eq!(
        changes[3],
        Change {
            value: "footer\n".to_string(),
            count: 1,
            added: false,
            removed: false,
        }
    );
}

#[test]
fn test_line_changes_multi_line_replacement_and_reconstruction() {
    let old = "fn main() {\n    let a = 1;\n    let b = 2;\n    println!(\"{}\", a + b);\n}\n";
    let new = "fn main() {\n    let a = 10;\n    let c = 30;\n    println!(\"{}\", a + c);\n}\n";

    let changes = line_changes_str(old, new);

    // Reconstruct old text from unchanged + removed changes
    let mut reconstructed_old = String::new();
    // Reconstruct new text from unchanged + added changes
    let mut reconstructed_new = String::new();

    for change in &changes {
        if !change.added {
            reconstructed_old.push_str(&change.value);
        }
        if !change.removed {
            reconstructed_new.push_str(&change.value);
        }
    }

    assert_eq!(reconstructed_old, old);
    assert_eq!(reconstructed_new, new);
}

#[test]
fn test_line_changes_missing_trailing_newline() {
    // Both lines lack newline
    let old = "a\nb";
    let new = "a\nc";
    let changes = line_changes_str(old, new);

    assert_eq!(changes.len(), 3);
    assert_eq!(changes[0].value, "a\n");
    assert!(!changes[0].added && !changes[0].removed);
    assert_eq!(changes[1].value, "b");
    assert!(changes[1].removed);
    assert_eq!(changes[2].value, "c");
    assert!(changes[2].added);

    // Old has trailing newline, new does not
    let old2 = "line1\nline2\n";
    let new2 = "line1\nline2";
    let changes2 = line_changes_str(old2, new2);
    assert_eq!(changes2.len(), 3);
    assert_eq!(changes2[0].value, "line1\n");
    assert_eq!(changes2[1].value, "line2\n");
    assert!(changes2[1].removed);
    assert_eq!(changes2[2].value, "line2");
    assert!(changes2[2].added);
}

#[test]
fn test_line_changes_consecutive_empty_lines() {
    let old = "start\n\n\nend\n";
    let new = "start\n\nmiddle\n\nend\n";
    let changes = line_changes_str(old, new);

    let mut reconstructed_old = String::new();
    let mut reconstructed_new = String::new();
    for c in &changes {
        if !c.added {
            reconstructed_old.push_str(&c.value);
        }
        if !c.removed {
            reconstructed_new.push_str(&c.value);
        }
    }
    assert_eq!(reconstructed_old, old);
    assert_eq!(reconstructed_new, new);
}

#[test]
fn test_line_changes_unicode_and_emojis() {
    let old = "Hello World\n🦀 Rust is awesome 🚀\n日本語のテキスト\nEnd\n";
    let new = "Hello World\n🦀 Rust is incredibly fast ⚡\n日本語のテスト\nEnd\n";

    let changes = line_changes_str(old, new);

    let mut rec_old = String::new();
    let mut rec_new = String::new();
    for c in &changes {
        if !c.added {
            rec_old.push_str(&c.value);
        }
        if !c.removed {
            rec_new.push_str(&c.value);
        }
    }
    assert_eq!(rec_old, old);
    assert_eq!(rec_new, new);
}

// ============================================================================
// 2. Line Runs (`fusion_diff::line_runs_str`)
// ============================================================================

#[test]
fn test_line_runs_empty_inputs() {
    let runs = line_runs_str("", "");
    assert!(runs.is_empty(), "runs on empty strings should be empty");
}

#[test]
fn test_line_runs_identical_content() {
    let text = "line1\nline2\nline3\nline4\n";
    let runs = line_runs_str(text, text);
    assert_eq!(runs.len(), 1);
    assert_eq!(
        runs[0],
        Run {
            count: 4,
            added: false,
            removed: false,
        }
    );
}

#[test]
fn test_line_runs_pure_addition_and_deletion() {
    let text = "line1\nline2\n";

    let add_runs = line_runs_str("", text);
    assert_eq!(add_runs.len(), 1);
    assert_eq!(
        add_runs[0],
        Run {
            count: 2,
            added: true,
            removed: false,
        }
    );

    let del_runs = line_runs_str(text, "");
    assert_eq!(del_runs.len(), 1);
    assert_eq!(
        del_runs[0],
        Run {
            count: 2,
            added: false,
            removed: true,
        }
    );
}

#[test]
fn test_line_runs_interleaved_modifications() {
    let old = "common1\nremoved1\ncommon2\nremoved2\ncommon3\n";
    let new = "common1\nadded1\ncommon2\nadded2\ncommon3\n";

    let runs = line_runs_str(old, new);

    // Expect: common1 (1), removed1 (1), added1 (1), common2 (1), removed2 (1), added2 (1), common3 (1)
    assert_eq!(runs.len(), 7);
    assert_eq!(runs[0], Run { count: 1, added: false, removed: false });
    assert_eq!(runs[1], Run { count: 1, added: false, removed: true });
    assert_eq!(runs[2], Run { count: 1, added: true, removed: false });
    assert_eq!(runs[3], Run { count: 1, added: false, removed: false });
    assert_eq!(runs[4], Run { count: 1, added: false, removed: true });
    assert_eq!(runs[5], Run { count: 1, added: true, removed: false });
    assert_eq!(runs[6], Run { count: 1, added: false, removed: false });
}

#[test]
fn test_line_runs_token_count_conservation() {
    let old = "a\nb\nc\nd\ne\nf\ng\nh\n";
    let new = "a\nb\nx\ny\nz\ne\ng\nh\ni\n";

    let old_tokens = line_tokens_str(old);
    let new_tokens = line_tokens_str(new);
    let runs = line_runs_str(old, new);

    let mut old_token_count = 0u32;
    let mut new_token_count = 0u32;

    for run in &runs {
        if run.removed {
            old_token_count += run.count;
        } else if run.added {
            new_token_count += run.count;
        } else {
            old_token_count += run.count;
            new_token_count += run.count;
        }
    }

    assert_eq!(old_token_count as usize, old_tokens.len());
    assert_eq!(new_token_count as usize, new_tokens.len());
}

// ============================================================================
// 3. Structured Patch Hunks (`fusion_diff::structured_patch_hunks_u16`)
// ============================================================================

#[test]
fn test_structured_patch_hunks_identical_yields_empty() {
    let text = to_u16("identical line 1\nidentical line 2\n");
    let hunks = structured_patch_hunks_u16(&text, &text, Some(3));
    assert!(
        hunks.is_empty(),
        "identical texts should produce zero patch hunks"
    );
}

#[test]
fn test_structured_patch_hunks_single_replacement_default_context() {
    let old = to_u16("1\n2\n3\n4\n5\nold\n7\n8\n9\n10\n11\n");
    let new = to_u16("1\n2\n3\n4\n5\nnew\n7\n8\n9\n10\n11\n");

    // Default context is 4
    let hunks = structured_patch_hunks_u16(&old, &new, None);
    assert_eq!(hunks.len(), 1);

    let hunk = &hunks[0];
    assert_eq!(hunk.old_start, 2); // 6 - 4 context lines = line 2
    assert_eq!(hunk.old_lines, 9); // 4 before + 1 changed + 4 after = 9 lines
    assert_eq!(hunk.new_start, 2);
    assert_eq!(hunk.new_lines, 9);

    let lines = hunk_lines_to_strings(hunk);
    assert_eq!(
        lines,
        vec![
            " 2", " 3", " 4", " 5", "-old", "+new", " 7", " 8", " 9", " 10"
        ]
    );
}

#[test]
fn test_structured_patch_hunks_zero_context() {
    let old = to_u16("prefix\nold_val\nsuffix\n");
    let new = to_u16("prefix\nnew_val\nsuffix\n");

    let hunks = structured_patch_hunks_u16(&old, &new, Some(0));
    assert_eq!(hunks.len(), 1);

    let hunk = &hunks[0];
    assert_eq!(hunk.old_start, 2);
    assert_eq!(hunk.old_lines, 1);
    assert_eq!(hunk.new_start, 2);
    assert_eq!(hunk.new_lines, 1);

    let lines = hunk_lines_to_strings(hunk);
    assert_eq!(lines, vec!["-old_val", "+new_val"]);
}

#[test]
fn test_structured_patch_hunks_separated_by_large_gap_creates_multiple_hunks() {
    // 2 changes separated by 10 unchanged lines; context = 2 (2 * context = 4 < 10)
    let mut old_s = String::new();
    let mut new_s = String::new();

    old_s.push_str("top\nchange_1_old\n");
    new_s.push_str("top\nchange_1_new\n");

    for i in 0..10 {
        let line = format!("gap_line_{i}\n");
        old_s.push_str(&line);
        new_s.push_str(&line);
    }

    old_s.push_str("change_2_old\nbottom\n");
    new_s.push_str("change_2_new\nbottom\n");

    let hunks = structured_patch_hunks_u16(&to_u16(&old_s), &to_u16(&new_s), Some(2));
    assert_eq!(
        hunks.len(),
        2,
        "changes separated by more than 2 * context lines must split into distinct hunks"
    );

    // Hunk 1 should contain change_1
    let lines1 = hunk_lines_to_strings(&hunks[0]);
    assert!(lines1.contains(&"-change_1_old".to_string()));
    assert!(lines1.contains(&"+change_1_new".to_string()));

    // Hunk 2 should contain change_2
    let lines2 = hunk_lines_to_strings(&hunks[1]);
    assert!(lines2.contains(&"-change_2_old".to_string()));
    assert!(lines2.contains(&"+change_2_new".to_string()));
}

#[test]
fn test_structured_patch_hunks_separated_by_small_gap_merges_hunks() {
    // 2 changes separated by 3 unchanged lines; context = 2 (2 * context = 4 >= 3)
    let old_s = "start\nch1_old\nmid1\nmid2\nmid3\nch2_old\nend\n";
    let new_s = "start\nch1_new\nmid1\nmid2\nmid3\nch2_new\nend\n";

    let hunks = structured_patch_hunks_u16(&to_u16(old_s), &to_u16(new_s), Some(2));
    assert_eq!(
        hunks.len(),
        1,
        "changes separated by <= 2 * context lines must coalesce into a single hunk"
    );

    let lines = hunk_lines_to_strings(&hunks[0]);
    assert!(lines.contains(&"-ch1_old".to_string()));
    assert!(lines.contains(&"+ch1_new".to_string()));
    assert!(lines.contains(&" mid1".to_string()));
    assert!(lines.contains(&" mid2".to_string()));
    assert!(lines.contains(&" mid3".to_string()));
    assert!(lines.contains(&"-ch2_old".to_string()));
    assert!(lines.contains(&"+ch2_new".to_string()));
}

#[test]
fn test_structured_patch_hunks_eof_newline_markers() {
    let no_nl = "\\ No newline at end of file";

    // Case 1: Old text missing EOF newline, new text has it
    let old1 = to_u16("first\nold_tail");
    let new1 = to_u16("first\nnew_tail\n");
    let hunks1 = structured_patch_hunks_u16(&old1, &new1, Some(1));
    assert_eq!(hunks1.len(), 1);
    let lines1 = hunk_lines_to_strings(&hunks1[0]);
    assert_eq!(
        lines1,
        vec![" first", "-old_tail", no_nl, "+new_tail"]
    );

    // Case 2: New text missing EOF newline, old text has it
    let old2 = to_u16("first\nold_tail\n");
    let new2 = to_u16("first\nnew_tail");
    let hunks2 = structured_patch_hunks_u16(&old2, &new2, Some(1));
    assert_eq!(hunks2.len(), 1);
    let lines2 = hunk_lines_to_strings(&hunks2[0]);
    assert_eq!(
        lines2,
        vec![" first", "-old_tail", "+new_tail", no_nl]
    );

    // Case 3: Both missing EOF newline
    let old3 = to_u16("first\nold_tail");
    let new3 = to_u16("first\nnew_tail");
    let hunks3 = structured_patch_hunks_u16(&old3, &new3, Some(1));
    assert_eq!(hunks3.len(), 1);
    let lines3 = hunk_lines_to_strings(&hunks3[0]);
    assert_eq!(
        lines3,
        vec![" first", "-old_tail", no_nl, "+new_tail", no_nl]
    );
}

#[test]
fn test_structured_patch_hunks_pure_file_creation_and_deletion() {
    // Creation
    let empty = to_u16("");
    let content = to_u16("line 1\nline 2\n");
    let hunks_create = structured_patch_hunks_u16(&empty, &content, Some(3));
    assert_eq!(hunks_create.len(), 1);
    assert_eq!(hunks_create[0].old_start, 1);
    assert_eq!(hunks_create[0].old_lines, 0);
    assert_eq!(hunks_create[0].new_start, 1);
    assert_eq!(hunks_create[0].new_lines, 2);
    assert_eq!(
        hunk_lines_to_strings(&hunks_create[0]),
        vec!["+line 1", "+line 2"]
    );

    // Deletion
    let hunks_delete = structured_patch_hunks_u16(&content, &empty, Some(3));
    assert_eq!(hunks_delete.len(), 1);
    assert_eq!(hunks_delete[0].old_start, 1);
    assert_eq!(hunks_delete[0].old_lines, 2);
    assert_eq!(hunks_delete[0].new_start, 1);
    assert_eq!(hunks_delete[0].new_lines, 0);
    assert_eq!(
        hunk_lines_to_strings(&hunks_delete[0]),
        vec!["-line 1", "-line 2"]
    );
}

// ============================================================================
// 4. Cross-Verification & Parity Tests
// ============================================================================

#[test]
fn test_parity_runs_and_changes() {
    let old = "fn add(a: i32, b: i32) -> i32 {\n    a + b\n}\n";
    let new = "pub fn add(a: i32, b: i32) -> i32 {\n    // addition\n    a + b\n}\n";

    let runs = line_runs_str(old, new);
    let changes = line_changes_str(old, new);

    assert_eq!(runs.len(), changes.len());
    for (run, change) in runs.iter().zip(changes.iter()) {
        assert_eq!(run.count, change.count);
        assert_eq!(run.added, change.added);
        assert_eq!(run.removed, change.removed);
    }
}

#[test]
fn test_parity_utf8_and_utf16_diff_lines() {
    let old = "fn compute() {\n    let x = 1;\n    let y = 2;\n}\n";
    let new = "fn compute() {\n    let x = 100;\n    let y = 2;\n    println!(\"done\");\n}\n";

    let changes_utf8 = line_changes_str(old, new);
    let changes_utf16 = diff_lines_u16(&to_u16(old), &to_u16(new));

    assert_eq!(changes_utf8.len(), changes_utf16.len());
    for (u8_change, u16_change) in changes_utf8.iter().zip(changes_utf16.iter()) {
        assert_eq!(u8_change.count, u16_change.count);
        assert_eq!(u8_change.added, u16_change.added);
        assert_eq!(u8_change.removed, u16_change.removed);
        assert_eq!(
            u8_change.value,
            String::from_utf16(&u16_change.value).unwrap()
        );
    }
}

#[test]
fn test_precomputed_runs_matches_direct_hunks() {
    let old_text = to_u16("aaa\nbbb\nccc\nddd\neee\n");
    let new_text = to_u16("aaa\nBBB\nccc\nDDD\neee\n");

    let old_tokens = line_tokens_u16(&old_text);
    let new_tokens = line_tokens_u16(&new_text);
    let runs = line_runs_str(
        "aaa\nbbb\nccc\nddd\neee\n",
        "aaa\nBBB\nccc\nDDD\neee\n",
    );

    let direct_hunks = structured_patch_hunks_u16(&old_text, &new_text, Some(1));
    let precomputed_hunks = structured_patch_hunks_from_runs_u16(
        Some(1),
        &old_tokens,
        &new_tokens,
        &runs,
    );

    assert_eq!(direct_hunks, precomputed_hunks);
}

#[test]
fn test_line_changes_crlf_newlines() {
    let old = "line 1\r\nline 2\r\nline 3\r\n";
    let new = "line 1\r\nline 2 modified\r\nline 3\r\n";

    let changes = line_changes_str(old, new);
    assert_eq!(changes.len(), 4);
    assert_eq!(changes[0].value, "line 1\r\n");
    assert!(!changes[0].added && !changes[0].removed);
    assert_eq!(changes[1].value, "line 2\r\n");
    assert!(changes[1].removed);
    assert_eq!(changes[2].value, "line 2 modified\r\n");
    assert!(changes[2].added);
    assert_eq!(changes[3].value, "line 3\r\n");
    assert!(!changes[3].added && !changes[3].removed);
}

#[test]
fn test_line_changes_repeated_identical_lines() {
    let old = "a\na\na\na\n";
    let new = "a\na\n";

    let changes = line_changes_str(old, new);
    let mut rec_old = String::new();
    let mut rec_new = String::new();
    for c in &changes {
        if !c.added {
            rec_old.push_str(&c.value);
        }
        if !c.removed {
            rec_new.push_str(&c.value);
        }
    }
    assert_eq!(rec_old, old);
    assert_eq!(rec_new, new);
}

#[test]
fn test_line_runs_completely_disjoint() {
    let old = "foo\nbar\n";
    let new = "baz\nqux\n";

    let runs = line_runs_str(old, new);
    assert_eq!(runs.len(), 2);
    assert_eq!(runs[0], Run { count: 2, added: false, removed: true });
    assert_eq!(runs[1], Run { count: 2, added: true, removed: false });
}

#[test]
fn test_structured_patch_hunks_deletion_in_middle() {
    let old = to_u16("line 1\nline 2\nline 3\nline 4\nline 5\n");
    let new = to_u16("line 1\nline 2\nline 4\nline 5\n");

    let hunks = structured_patch_hunks_u16(&old, &new, Some(1));
    assert_eq!(hunks.len(), 1);
    let hunk = &hunks[0];
    assert_eq!(hunk.old_start, 2);
    assert_eq!(hunk.old_lines, 3);
    assert_eq!(hunk.new_start, 2);
    assert_eq!(hunk.new_lines, 2);

    let lines = hunk_lines_to_strings(hunk);
    assert_eq!(lines, vec![" line 2", "-line 3", " line 4"]);
}

#[test]
fn test_structured_patch_hunks_addition_in_middle() {
    let old = to_u16("line 1\nline 2\nline 4\nline 5\n");
    let new = to_u16("line 1\nline 2\nline 3\nline 4\nline 5\n");

    let hunks = structured_patch_hunks_u16(&old, &new, Some(1));
    assert_eq!(hunks.len(), 1);
    let hunk = &hunks[0];
    assert_eq!(hunk.old_start, 2);
    assert_eq!(hunk.old_lines, 2);
    assert_eq!(hunk.new_start, 2);
    assert_eq!(hunk.new_lines, 3);

    let lines = hunk_lines_to_strings(hunk);
    assert_eq!(lines, vec![" line 2", "+line 3", " line 4"]);
}

#[test]
fn test_structured_patch_hunks_context_at_boundaries() {
    // Change at the very first line
    let old1 = to_u16("first_old\nsecond\nthird\n");
    let new1 = to_u16("first_new\nsecond\nthird\n");
    let hunks1 = structured_patch_hunks_u16(&old1, &new1, Some(3));
    assert_eq!(hunks1.len(), 1);
    assert_eq!(hunks1[0].old_start, 1);
    assert_eq!(hunks1[0].new_start, 1);
    let lines1 = hunk_lines_to_strings(&hunks1[0]);
    assert_eq!(lines1, vec!["-first_old", "+first_new", " second", " third"]);

    // Change at the very last line
    let old2 = to_u16("first\nsecond\nlast_old\n");
    let new2 = to_u16("first\nsecond\nlast_new\n");
    let hunks2 = structured_patch_hunks_u16(&old2, &new2, Some(3));
    assert_eq!(hunks2.len(), 1);
    assert_eq!(hunks2[0].old_start, 1);
    assert_eq!(hunks2[0].new_start, 1);
    let lines2 = hunk_lines_to_strings(&hunks2[0]);
    assert_eq!(lines2, vec![" first", " second", "-last_old", "+last_new"]);
}

#[test]
fn test_structured_patch_hunks_surrogate_pairs() {
    let old = to_u16("🦀 crab\n🚀 rocket\n");
    let new = to_u16("🦀 crab\n⚡ lightning\n");

    let hunks = structured_patch_hunks_u16(&old, &new, Some(1));
    assert_eq!(hunks.len(), 1);
    let lines = hunk_lines_to_strings(&hunks[0]);
    assert_eq!(lines, vec![" 🦀 crab", "-🚀 rocket", "+⚡ lightning"]);
}
