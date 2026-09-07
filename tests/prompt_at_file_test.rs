//! Test suite for `@file` query parsing, suggestion filtering, and buffer insertion in `src/ui/prompt.rs`.

use crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers};
use fusion::ui::prompt::{
    extract_at_trigger, fuzzy_match_files, is_query_char, is_valid_at_prefix, AtFileTrigger, Prompt,
};

// =========================================================================
// 1. Tests for `@` prefix detection and query extraction
// =========================================================================

#[test]
fn test_extract_at_trigger_at_start_of_buffer() {
    let buf: Vec<char> = "@".chars().collect();
    let trigger = extract_at_trigger(&buf, 1);
    assert_eq!(
        trigger,
        Some(AtFileTrigger {
            at_index: 0,
            query: String::new(),
        })
    );
}

#[test]
fn test_extract_at_trigger_simple_word() {
    let buf: Vec<char> = "@main".chars().collect();
    let trigger = extract_at_trigger(&buf, 5);
    assert_eq!(
        trigger,
        Some(AtFileTrigger {
            at_index: 0,
            query: "main".to_string(),
        })
    );
}

#[test]
fn test_extract_at_trigger_after_whitespace() {
    let buf: Vec<char> = "check @main".chars().collect();
    let trigger = extract_at_trigger(&buf, 11);
    assert_eq!(
        trigger,
        Some(AtFileTrigger {
            at_index: 6,
            query: "main".to_string(),
        })
    );
}

#[test]
fn test_extract_at_trigger_with_path_characters() {
    let buf: Vec<char> = "open @src/ui/prompt.rs".chars().collect();
    let trigger = extract_at_trigger(&buf, 22);
    assert_eq!(
        trigger,
        Some(AtFileTrigger {
            at_index: 5,
            query: "src/ui/prompt.rs".to_string(),
        })
    );
}

#[test]
fn test_extract_at_trigger_with_delimiters_and_punctuation() {
    // Delimiters before @ such as '(', '[', '{', ':', '=', ',' are valid prefixes
    let cases = [
        ("(@main", 1, 6, "main"),
        ("[@src/lib.rs", 1, 12, "src/lib.rs"),
        ("file=@main", 5, 10, "main"),
        ("foo, @test", 5, 10, "test"),
    ];

    for (text, expected_at, cursor, expected_query) in cases {
        let buf: Vec<char> = text.chars().collect();
        let trigger = extract_at_trigger(&buf, cursor);
        assert_eq!(
            trigger,
            Some(AtFileTrigger {
                at_index: expected_at,
                query: expected_query.to_string(),
            }),
            "Failed for text: {}",
            text
        );
    }
}

#[test]
fn test_extract_at_trigger_invalid_prefixes_rejected() {
    // Email addresses or identifiers containing @ should NOT trigger
    let buf: Vec<char> = "user@example.com".chars().collect();
    assert_eq!(extract_at_trigger(&buf, 16), None);

    let buf2: Vec<char> = "some_var@file".chars().collect();
    assert_eq!(extract_at_trigger(&buf2, 13), None);

    let buf3: Vec<char> = "123@file".chars().collect();
    assert_eq!(extract_at_trigger(&buf3, 8), None);
}

#[test]
fn test_extract_at_trigger_cursor_positions() {
    // Cursor at 0
    let buf: Vec<char> = "@main".chars().collect();
    assert_eq!(extract_at_trigger(&buf, 0), None);

    // Cursor right after @
    let trigger = extract_at_trigger(&buf, 1);
    assert_eq!(
        trigger,
        Some(AtFileTrigger {
            at_index: 0,
            query: String::new(),
        })
    );

    // Cursor in the middle of query: "look @main.rs" with cursor right after @m
    let buf: Vec<char> = "look @main.rs".chars().collect();
    let trigger = extract_at_trigger(&buf, 7);
    assert_eq!(
        trigger,
        Some(AtFileTrigger {
            at_index: 5,
            query: "m".to_string(),
        })
    );
}

#[test]
fn test_extract_at_trigger_disallowed_query_characters() {
    // Space ends trigger
    let buf: Vec<char> = "hello @main ".chars().collect();
    assert_eq!(extract_at_trigger(&buf, 12), None);

    // Special symbols like #, ?, !, $ terminate path query
    let buf: Vec<char> = "test @foo#bar".chars().collect();
    assert_eq!(extract_at_trigger(&buf, 13), None);
}

#[test]
fn test_is_query_char_and_is_valid_at_prefix() {
    // Query chars: alphanumeric, _, -, ., /, \
    assert!(is_query_char('a'));
    assert!(is_query_char('Z'));
    assert!(is_query_char('9'));
    assert!(is_query_char('_'));
    assert!(is_query_char('-'));
    assert!(is_query_char('.'));
    assert!(is_query_char('/'));
    assert!(is_query_char('\\'));
    assert!(!is_query_char(' '));
    assert!(!is_query_char('@'));
    assert!(!is_query_char('#'));

    // Valid prefix: not alphanumeric and not '_'
    assert!(is_valid_at_prefix(' '));
    assert!(is_valid_at_prefix('\t'));
    assert!(is_valid_at_prefix('('));
    assert!(is_valid_at_prefix('['));
    assert!(is_valid_at_prefix(':'));
    assert!(is_valid_at_prefix('='));
    assert!(!is_valid_at_prefix('a'));
    assert!(!is_valid_at_prefix('1'));
    assert!(!is_valid_at_prefix('_'));
}

// =========================================================================
// 2. Tests for path substring and fuzzy suggestion filtering
// =========================================================================

#[test]
fn test_fuzzy_match_files_empty_query() {
    let files = vec![
        "src/main.rs".to_string(),
        "src/ui/prompt.rs".to_string(),
        "Cargo.toml".to_string(),
    ];
    let matches = fuzzy_match_files("", &files);
    assert_eq!(matches.len(), 3);
    assert_eq!(matches, files);
}

#[test]
fn test_fuzzy_match_files_exact_filename_ranks_highest() {
    let files = vec![
        "crates/fusion-shell/src/main.rs".to_string(),
        "src/main.rs".to_string(),
        "src/domain/main_entry.rs".to_string(),
        "tests/main_test.rs".to_string(),
    ];

    let matches = fuzzy_match_files("main.rs", &files);
    assert!(!matches.is_empty());
    // Shorter path "src/main.rs" with exact filename should rank first
    assert_eq!(matches[0], "src/main.rs");
    assert_eq!(matches[1], "crates/fusion-shell/src/main.rs");
}

#[test]
fn test_fuzzy_match_files_path_substring_matching() {
    let files = vec![
        "src/ui/prompt.rs".to_string(),
        "src/ui/theme.rs".to_string(),
        "src/tools/read.rs".to_string(),
        "crates/fusion-ast/src/summary.rs".to_string(),
    ];

    // Directory path substring matches rank higher than loose subsequences
    let ui_matches = fuzzy_match_files("ui/", &files);
    assert!(ui_matches.len() >= 2);
    assert_eq!(ui_matches[0], "src/ui/theme.rs");
    assert_eq!(ui_matches[1], "src/ui/prompt.rs");

    // Stem match
    let prompt_matches = fuzzy_match_files("prompt", &files);
    assert_eq!(prompt_matches.len(), 1);
    assert_eq!(prompt_matches[0], "src/ui/prompt.rs");

    // Directory match for tools/
    let tools_matches = fuzzy_match_files("tools/", &files);
    assert_eq!(tools_matches.len(), 1);
    assert_eq!(tools_matches[0], "src/tools/read.rs");

    // Substring across crate path
    let summary_matches = fuzzy_match_files("summary", &files);
    assert_eq!(summary_matches.len(), 1);
    assert_eq!(summary_matches[0], "crates/fusion-ast/src/summary.rs");
}

#[test]
fn test_fuzzy_match_files_case_insensitive() {
    let files = vec![
        "src/UI/Prompt.rs".to_string(),
        "README.md".to_string(),
        "Cargo.toml".to_string(),
    ];

    let matches_lower = fuzzy_match_files("prompt", &files);
    assert_eq!(matches_lower.len(), 1);
    assert_eq!(matches_lower[0], "src/UI/Prompt.rs");

    let matches_upper = fuzzy_match_files("PROMPT", &files);
    assert_eq!(matches_upper.len(), 1);
    assert_eq!(matches_upper[0], "src/UI/Prompt.rs");

    let readme_matches = fuzzy_match_files("readme", &files);
    assert_eq!(readme_matches.len(), 1);
    assert_eq!(readme_matches[0], "README.md");
}

#[test]
fn test_fuzzy_match_files_subsequence_fuzzy_matching() {
    let files = vec![
        "crates/fusion-shell/src/main.rs".to_string(),
        "src/ui/prompt.rs".to_string(),
        "tests/prompt_at_file_test.rs".to_string(),
    ];

    // "paft" matches "tests/prompt_at_file_test.rs"
    let matches = fuzzy_match_files("paft", &files);
    assert!(!matches.is_empty());
    assert_eq!(matches[0], "tests/prompt_at_file_test.rs");

    // "fsm" matches "crates/fusion-shell/src/main.rs"
    let fsm_matches = fuzzy_match_files("fsm", &files);
    assert!(!fsm_matches.is_empty());
    assert_eq!(fsm_matches[0], "crates/fusion-shell/src/main.rs");
}

#[test]
fn test_fuzzy_match_files_no_match() {
    let files = vec![
        "src/main.rs".to_string(),
        "Cargo.toml".to_string(),
    ];

    let matches = fuzzy_match_files("nonexistent_symbol_xyz", &files);
    assert!(matches.is_empty());
}

// =========================================================================
// 3. Tests for insertion into prompt buffer
// =========================================================================

#[test]
fn test_apply_at_file_completion_at_start_of_buffer() {
    let mut prompt = Prompt::new();
    prompt.buffer = "@main".chars().collect();
    prompt.cursor_pos = 5;

    let success = prompt.apply_at_file_completion("src/main.rs");
    assert!(success);
    assert_eq!(prompt.buffer_text(), "src/main.rs");
    assert_eq!(prompt.cursor_pos, 11);
}

#[test]
fn test_apply_at_file_completion_in_middle_of_sentence() {
    let mut prompt = Prompt::new();
    prompt.buffer = "review @main for bugs".chars().collect();
    prompt.cursor_pos = 12; // right after @main

    let success = prompt.apply_at_file_completion("src/main.rs");
    assert!(success);
    assert_eq!(prompt.buffer_text(), "review src/main.rs for bugs");
    // "review " is 7 chars + "src/main.rs" is 11 chars = cursor at 18
    assert_eq!(prompt.cursor_pos, 18);
}

#[test]
fn test_apply_at_file_completion_replaces_partial_path() {
    let mut prompt = Prompt::new();
    prompt.buffer = "look at @src/ui/pr".chars().collect();
    prompt.cursor_pos = 18;

    let success = prompt.apply_at_file_completion("src/ui/prompt.rs");
    assert!(success);
    assert_eq!(prompt.buffer_text(), "look at src/ui/prompt.rs");
    assert_eq!(prompt.cursor_pos, 8 + "src/ui/prompt.rs".len());
}

#[test]
fn test_apply_at_file_completion_when_no_trigger_returns_false() {
    let mut prompt = Prompt::new();
    prompt.buffer = "look at main.rs without at sign".chars().collect();
    prompt.cursor_pos = 15;

    let success = prompt.apply_at_file_completion("src/main.rs");
    assert!(!success);
    assert_eq!(prompt.buffer_text(), "look at main.rs without at sign");
}

#[test]
fn test_select_at_file_completion_using_file_cache() {
    let mut prompt = Prompt::new().with_file_cache(vec![
        "src/main.rs".to_string(),
        "crates/fusion-shell/src/main.rs".to_string(),
    ]);

    prompt.buffer = "edit @main".chars().collect();
    prompt.cursor_pos = 10;

    // Selection 0 -> first match "src/main.rs"
    prompt.set_at_file_selection(0);
    assert_eq!(prompt.at_file_selection(), 0);

    let completed = prompt.select_at_file_completion();
    assert!(completed);
    assert_eq!(prompt.buffer_text(), "edit src/main.rs");
    assert_eq!(prompt.cursor_pos, 5 + "src/main.rs".len());
}

#[test]
fn test_select_at_file_completion_second_selection() {
    let mut prompt = Prompt::new().with_file_cache(vec![
        "src/main.rs".to_string(),
        "crates/fusion-shell/src/main.rs".to_string(),
    ]);

    prompt.buffer = "@main".chars().collect();
    prompt.cursor_pos = 5;

    // Select second item
    prompt.set_at_file_selection(1);
    assert_eq!(prompt.at_file_selection(), 1);

    let completed = prompt.select_at_file_completion();
    assert!(completed);
    assert_eq!(prompt.buffer_text(), "crates/fusion-shell/src/main.rs");
}

#[test]
fn test_prompt_handle_event_arrow_navigation_and_tab_insertion() {
    let mut prompt = Prompt::new().with_file_cache(vec![
        "src/main.rs".to_string(),
        "crates/fusion-shell/src/main.rs".to_string(),
    ]);

    prompt.buffer = "open @main".chars().collect();
    prompt.cursor_pos = 10;

    // Down arrow moves selection from 0 to 1
    let down_event = Event::Key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
    let res = prompt.handle_event(down_event).expect("handle_event failed");
    assert_eq!(res, None);
    assert_eq!(prompt.at_file_selection(), 1);

    // Up arrow moves selection back from 1 to 0
    let up_event = Event::Key(KeyEvent::new(KeyCode::Up, KeyModifiers::NONE));
    let res = prompt.handle_event(up_event).expect("handle_event failed");
    assert_eq!(res, None);
    assert_eq!(prompt.at_file_selection(), 0);

    // Tab key inserts the selected item
    let tab_event = Event::Key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE));
    let res = prompt.handle_event(tab_event).expect("handle_event failed");
    assert_eq!(res, None);
    assert_eq!(prompt.buffer_text(), "open src/main.rs");
    assert_eq!(prompt.cursor_pos, 5 + "src/main.rs".len());
}

#[test]
fn test_prompt_handle_event_enter_insertion() {
    let mut prompt = Prompt::new().with_file_cache(vec![
        "src/main.rs".to_string(),
        "crates/fusion-shell/src/main.rs".to_string(),
    ]);

    prompt.buffer = "read @main".chars().collect();
    prompt.cursor_pos = 10;
    prompt.set_at_file_selection(1);

    // Enter key inserts the selected file without submitting the prompt
    let enter_event = Event::Key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    let res = prompt.handle_event(enter_event).expect("handle_event failed");
    // Should NOT submit the prompt, but return None and insert text
    assert_eq!(res, None);
    assert_eq!(prompt.buffer_text(), "read crates/fusion-shell/src/main.rs");
}

#[test]
fn test_prompt_handle_event_esc_dismisses_dropdown() {
    let mut prompt = Prompt::new().with_file_cache(vec!["src/main.rs".to_string()]);

    prompt.buffer = "@main".chars().collect();
    prompt.cursor_pos = 5;

    assert!(!prompt.at_file_dismissed);

    let esc_event = Event::Key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
    let res = prompt.handle_event(esc_event).expect("handle_event failed");
    assert_eq!(res, None);
    assert!(prompt.at_file_dismissed);

    // Buffer remains untouched
    assert_eq!(prompt.buffer_text(), "@main");
}
