//! Test suite for `src/agent/context_injector.rs`.
//!
//! Covers:
//! - Context file discovery (AGENTS.md, .fusion/AGENTS.md, CLAUDE.md, .claude/CLAUDE.md)
//! - Precedence order & fallback on empty/whitespace-only files
//! - Missing context files returning None
//! - `@file` mention extraction and expansion with `<file-mention>` blocks
//! - Multiple and duplicate mentions
//! - Sentence punctuation handling (trailing commas, periods, question marks, brackets)
//! - Quoted path mentions
//! - Path normalization (stripping `./`)
//! - Non-mentions (email addresses, lone `@`, `@@`)
//! - Non-existent file and directory mention skips
//! - Binary file detection and skipping (null bytes, invalid UTF-8)
//! - Maximum file size capping at 50 KB (51,200 bytes)

#[path = "../src/agent/context_injector.rs"]
mod context_injector;

use context_injector::{
    detect_and_load_repo_context, expand_file_mentions, extract_mention_candidates,
    is_valid_at_prefix, MAX_FILE_SIZE_BYTES, REPO_CONTEXT_CANDIDATES,
};
use std::fs;
use std::path::PathBuf;
use tempfile::tempdir;

// =========================================================================
// 1. Tests for detect_and_load_repo_context
// =========================================================================

#[test]
fn test_detect_and_load_repo_context_root_agents_md() {
    let temp = tempdir().unwrap();
    let agents_path = temp.path().join("AGENTS.md");
    fs::write(&agents_path, "# Agents Guidelines\nFollow TDD strictly.").unwrap();

    let result = detect_and_load_repo_context(temp.path());
    assert!(result.is_some());
    let content = result.unwrap();
    assert_eq!(
        content,
        "\n\n### Repository Context (from AGENTS.md):\n# Agents Guidelines\nFollow TDD strictly."
    );
}

#[test]
fn test_detect_and_load_repo_context_fusion_agents_md() {
    let temp = tempdir().unwrap();
    let fusion_dir = temp.path().join(".fusion");
    fs::create_dir_all(&fusion_dir).unwrap();
    let agents_path = fusion_dir.join("AGENTS.md");
    fs::write(&agents_path, "Fusion-specific instructions").unwrap();

    let result = detect_and_load_repo_context(temp.path());
    assert!(result.is_some());
    let content = result.unwrap();
    assert_eq!(
        content,
        "\n\n### Repository Context (from .fusion/AGENTS.md):\nFusion-specific instructions"
    );
}

#[test]
fn test_detect_and_load_repo_context_root_claude_md() {
    let temp = tempdir().unwrap();
    let claude_path = temp.path().join("CLAUDE.md");
    fs::write(&claude_path, "Claude instructions here").unwrap();

    let result = detect_and_load_repo_context(temp.path());
    assert!(result.is_some());
    let content = result.unwrap();
    assert_eq!(
        content,
        "\n\n### Repository Context (from CLAUDE.md):\nClaude instructions here"
    );
}

#[test]
fn test_detect_and_load_repo_context_dot_claude_md() {
    let temp = tempdir().unwrap();
    let claude_dir = temp.path().join(".claude");
    fs::create_dir_all(&claude_dir).unwrap();
    let claude_path = claude_dir.join("CLAUDE.md");
    fs::write(&claude_path, "Dot-claude instructions").unwrap();

    let result = detect_and_load_repo_context(temp.path());
    assert!(result.is_some());
    let content = result.unwrap();
    assert_eq!(
        content,
        "\n\n### Repository Context (from .claude/CLAUDE.md):\nDot-claude instructions"
    );
}

#[test]
fn test_detect_and_load_repo_context_precedence_order() {
    let temp = tempdir().unwrap();
    // Create both AGENTS.md and CLAUDE.md
    fs::write(temp.path().join("AGENTS.md"), "Root AGENTS").unwrap();
    fs::write(temp.path().join("CLAUDE.md"), "Root CLAUDE").unwrap();

    let result = detect_and_load_repo_context(temp.path());
    assert!(result.is_some());
    let content = result.unwrap();
    assert!(content.contains("(from AGENTS.md)"));
    assert!(content.contains("Root AGENTS"));
    assert!(!content.contains("Root CLAUDE"));
}

#[test]
fn test_detect_and_load_repo_context_empty_file_fallback() {
    let temp = tempdir().unwrap();
    // AGENTS.md is empty (0 bytes), CLAUDE.md has content
    fs::write(temp.path().join("AGENTS.md"), "").unwrap();
    fs::write(temp.path().join("CLAUDE.md"), "Fallback CLAUDE content").unwrap();

    let result = detect_and_load_repo_context(temp.path());
    assert!(result.is_some());
    let content = result.unwrap();
    assert!(content.contains("(from CLAUDE.md)"));
    assert!(content.contains("Fallback CLAUDE content"));
}

#[test]
fn test_detect_and_load_repo_context_whitespace_only_fallback() {
    let temp = tempdir().unwrap();
    // AGENTS.md is whitespace only, .fusion/AGENTS.md has content
    fs::write(temp.path().join("AGENTS.md"), "   \n\t  \n").unwrap();
    let fusion_dir = temp.path().join(".fusion");
    fs::create_dir_all(&fusion_dir).unwrap();
    fs::write(fusion_dir.join("AGENTS.md"), "Fusion content").unwrap();

    let result = detect_and_load_repo_context(temp.path());
    assert!(result.is_some());
    let content = result.unwrap();
    assert!(content.contains("(from .fusion/AGENTS.md)"));
    assert!(content.contains("Fusion content"));
}

#[test]
fn test_detect_and_load_repo_context_none_found() {
    let temp = tempdir().unwrap();
    let result = detect_and_load_repo_context(temp.path());
    assert!(result.is_none());
}

#[test]
fn test_detect_and_load_repo_context_all_empty() {
    let temp = tempdir().unwrap();
    fs::write(temp.path().join("AGENTS.md"), "   \n").unwrap();
    fs::write(temp.path().join("CLAUDE.md"), "").unwrap();
    let result = detect_and_load_repo_context(temp.path());
    assert!(result.is_none());
}

// =========================================================================
// 2. Tests for extract_mention_candidates and prefix validation
// =========================================================================

#[test]
fn test_is_valid_at_prefix() {
    assert!(is_valid_at_prefix(' '));
    assert!(is_valid_at_prefix('\t'));
    assert!(is_valid_at_prefix('\n'));
    assert!(is_valid_at_prefix('('));
    assert!(is_valid_at_prefix('['));
    assert!(is_valid_at_prefix('{'));
    assert!(is_valid_at_prefix(':'));
    assert!(is_valid_at_prefix(';'));
    assert!(is_valid_at_prefix('`'));
    assert!(is_valid_at_prefix('"'));
    assert!(is_valid_at_prefix('\''));

    // Alphanumeric and underscore are invalid prefixes
    assert!(!is_valid_at_prefix('a'));
    assert!(!is_valid_at_prefix('Z'));
    assert!(!is_valid_at_prefix('0'));
    assert!(!is_valid_at_prefix('_'));
}

#[test]
fn test_extract_mention_candidates() {
    let candidates = extract_mention_candidates("Please check @src/main.rs and @Cargo.toml.");
    assert_eq!(candidates, vec!["src/main.rs", "Cargo.toml."]);

    let email_test = extract_mention_candidates("Contact user@example.com for info");
    assert!(email_test.is_empty());

    let quoted_test = extract_mention_candidates(r#"Look at @"my path/with spaces.txt" now"#);
    assert_eq!(quoted_test, vec!["my path/with spaces.txt"]);

    let start_of_str = extract_mention_candidates("@README.md is important");
    assert_eq!(start_of_str, vec!["README.md"]);

    let backticks = extract_mention_candidates("Check `@src/lib.rs` for details");
    assert_eq!(backticks, vec!["src/lib.rs`"]);
}

// =========================================================================
// 3. Tests for expand_file_mentions
// =========================================================================

#[test]
fn test_expand_file_mentions_single_file() {
    let temp = tempdir().unwrap();
    let file_path = temp.path().join("main.rs");
    fs::write(&file_path, "fn main() {\n    println!(\"Hello!\");\n}\n").unwrap();

    let prompt = "Explain the logic in @main.rs";
    let (expanded, injected) = expand_file_mentions(prompt, temp.path());

    assert_eq!(injected, vec![PathBuf::from("main.rs")]);
    assert!(expanded.starts_with("Explain the logic in @main.rs"));
    assert!(expanded.contains("<file-mention path=\"main.rs\">"));
    assert!(expanded.contains("fn main() {\n    println!(\"Hello!\");\n}"));
    assert!(expanded.contains("</file-mention>"));
}

#[test]
fn test_expand_file_mentions_multiple_files() {
    let temp = tempdir().unwrap();
    let file1 = temp.path().join("src").join("lib.rs");
    let file2 = temp.path().join("Cargo.toml");
    fs::create_dir_all(file1.parent().unwrap()).unwrap();
    fs::write(&file1, "pub fn add(a: i32, b: i32) -> i32 { a + b }\n").unwrap();
    fs::write(&file2, "[package]\nname = \"my-pkg\"\n").unwrap();

    let prompt = "Compare @src/lib.rs with @Cargo.toml configurations";
    let (expanded, injected) = expand_file_mentions(prompt, temp.path());

    assert_eq!(
        injected,
        vec![PathBuf::from("src/lib.rs"), PathBuf::from("Cargo.toml")]
    );
    assert!(expanded.contains("<file-mention path=\"src/lib.rs\">\npub fn add"));
    assert!(expanded.contains("<file-mention path=\"Cargo.toml\">\n[package]"));
}

#[test]
fn test_expand_file_mentions_deduplication() {
    let temp = tempdir().unwrap();
    let file_path = temp.path().join("Cargo.toml");
    fs::write(&file_path, "[package]\nname = \"test\"\n").unwrap();

    let prompt = "Look at @Cargo.toml and also check @Cargo.toml again";
    let (expanded, injected) = expand_file_mentions(prompt, temp.path());

    assert_eq!(injected, vec![PathBuf::from("Cargo.toml")]);
    // Ensure only one file-mention tag is appended
    let count = expanded
        .matches("<file-mention path=\"Cargo.toml\">")
        .count();
    assert_eq!(count, 1);
}

#[test]
fn test_expand_file_mentions_sentence_punctuation() {
    let temp = tempdir().unwrap();
    let file1 = temp.path().join("file1.rs");
    let file2 = temp.path().join("file2.rs");
    let file3 = temp.path().join("file3.rs");
    let file4 = temp.path().join("file4.rs");
    fs::write(&file1, "// file1\n").unwrap();
    fs::write(&file2, "// file2\n").unwrap();
    fs::write(&file3, "// file3\n").unwrap();
    fs::write(&file4, "// file4\n").unwrap();

    let prompt = "Check @file1.rs, review @file2.rs. What about @file3.rs? Inspect (@file4.rs)!";
    let (expanded, injected) = expand_file_mentions(prompt, temp.path());

    assert_eq!(
        injected,
        vec![
            PathBuf::from("file1.rs"),
            PathBuf::from("file2.rs"),
            PathBuf::from("file3.rs"),
            PathBuf::from("file4.rs"),
        ]
    );
    assert!(expanded.contains("<file-mention path=\"file1.rs\">"));
    assert!(expanded.contains("<file-mention path=\"file2.rs\">"));
    assert!(expanded.contains("<file-mention path=\"file3.rs\">"));
    assert!(expanded.contains("<file-mention path=\"file4.rs\">"));
}

#[test]
fn test_expand_file_mentions_dot_slash_prefix() {
    let temp = tempdir().unwrap();
    let file_path = temp.path().join("src").join("main.rs");
    fs::create_dir_all(file_path.parent().unwrap()).unwrap();
    fs::write(&file_path, "fn main() {}\n").unwrap();

    let prompt = "Examine @./src/main.rs";
    let (expanded, injected) = expand_file_mentions(prompt, temp.path());

    assert_eq!(injected, vec![PathBuf::from("src/main.rs")]);
    assert!(expanded.contains("<file-mention path=\"src/main.rs\">"));
}

#[test]
fn test_expand_file_mentions_quoted_path() {
    let temp = tempdir().unwrap();
    let file_path = temp.path().join("my script.py");
    fs::write(&file_path, "print('hello')\n").unwrap();

    let prompt = r#"Run @"my script.py" please"#;
    let (expanded, injected) = expand_file_mentions(prompt, temp.path());

    assert_eq!(injected, vec![PathBuf::from("my script.py")]);
    assert!(expanded.contains("<file-mention path=\"my script.py\">"));
    assert!(expanded.contains("print('hello')"));
}

#[test]
fn test_expand_file_mentions_ignores_email_and_invalid_prefix() {
    let temp = tempdir().unwrap();
    let file_path = temp.path().join("example.com");
    fs::write(&file_path, "not intended\n").unwrap();

    let prompt = "Send mail to user@example.com or admin@example.com";
    let (expanded, injected) = expand_file_mentions(prompt, temp.path());

    assert!(injected.is_empty());
    assert_eq!(expanded, prompt);
}

#[test]
fn test_expand_file_mentions_nonexistent_file() {
    let temp = tempdir().unwrap();
    let prompt = "Look at @nonexistent.rs";
    let (expanded, injected) = expand_file_mentions(prompt, temp.path());

    assert!(injected.is_empty());
    assert_eq!(expanded, prompt);
}

#[test]
fn test_expand_file_mentions_directory_is_skipped() {
    let temp = tempdir().unwrap();
    let dir_path = temp.path().join("src");
    fs::create_dir_all(&dir_path).unwrap();

    let prompt = "Check directory @src for files";
    let (expanded, injected) = expand_file_mentions(prompt, temp.path());

    assert!(injected.is_empty());
    assert_eq!(expanded, prompt);
}

#[test]
fn test_expand_file_mentions_binary_file_skipped() {
    let temp = tempdir().unwrap();
    // Null byte binary
    let bin_path = temp.path().join("image.png");
    let mut binary_data = vec![0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
    binary_data.push(0x00); // null byte
    binary_data.extend_from_slice(&[0x01, 0x02, 0x03]);
    fs::write(&bin_path, &binary_data).unwrap();

    // Valid text file
    let text_path = temp.path().join("notes.txt");
    fs::write(&text_path, "Valid text content\n").unwrap();

    let prompt = "Check @image.png and @notes.txt";
    let (expanded, injected) = expand_file_mentions(prompt, temp.path());

    // Only notes.txt should be injected; image.png skipped
    assert_eq!(injected, vec![PathBuf::from("notes.txt")]);
    assert!(!expanded.contains("<file-mention path=\"image.png\">"));
    assert!(expanded.contains("<file-mention path=\"notes.txt\">"));
    assert!(expanded.contains("Valid text content"));
}

#[test]
fn test_expand_file_mentions_non_utf8_binary_skipped() {
    let temp = tempdir().unwrap();
    let bin_path = temp.path().join("bad_utf8.bin");
    // Invalid UTF-8 sequence without null bytes
    let invalid_utf8 = vec![0xFF, 0xFE, 0xFD, 0xFC];
    fs::write(&bin_path, &invalid_utf8).unwrap();

    let prompt = "Check @bad_utf8.bin";
    let (expanded, injected) = expand_file_mentions(prompt, temp.path());

    assert!(injected.is_empty());
    assert_eq!(expanded, prompt);
}

#[test]
fn test_expand_file_mentions_size_cap_50kb() {
    let temp = tempdir().unwrap();
    let large_file = temp.path().join("large.txt");

    // Generate a file of ~75 KB (larger than 50 KB = 51,200 bytes)
    let line = "0123456789abcdef\n"; // 17 bytes
    let total_lines = (75 * 1024) / line.len();
    let large_content = line.repeat(total_lines);
    assert!(large_content.len() > MAX_FILE_SIZE_BYTES);
    fs::write(&large_file, &large_content).unwrap();

    let prompt = "Analyze @large.txt";
    let (expanded, injected) = expand_file_mentions(prompt, temp.path());

    assert_eq!(injected, vec![PathBuf::from("large.txt")]);
    assert!(expanded.contains("<file-mention path=\"large.txt\">"));

    // Extract the content inside the file-mention block
    let tag_open = "<file-mention path=\"large.txt\">\n";
    let tag_close = "\n</file-mention>";
    let start = expanded.find(tag_open).unwrap() + tag_open.len();
    let end = expanded.find(tag_close).unwrap();
    let injected_content = &expanded[start..end];

    // Verify size cap: content must not exceed MAX_FILE_SIZE_BYTES
    assert!(injected_content.len() <= MAX_FILE_SIZE_BYTES);
    assert_eq!(injected_content.len(), MAX_FILE_SIZE_BYTES);
}

#[test]
fn test_expand_file_mentions_empty_file() {
    let temp = tempdir().unwrap();
    let empty_file = temp.path().join("empty.txt");
    fs::write(&empty_file, "").unwrap();

    let prompt = "Check @empty.txt";
    let (expanded, injected) = expand_file_mentions(prompt, temp.path());

    assert_eq!(injected, vec![PathBuf::from("empty.txt")]);
    assert!(expanded.contains("<file-mention path=\"empty.txt\">\n</file-mention>"));
}

#[test]
fn test_expand_file_mentions_no_mentions() {
    let temp = tempdir().unwrap();
    let prompt = "Hello, how do I configure Tokio runtime?";
    let (expanded, injected) = expand_file_mentions(prompt, temp.path());

    assert!(injected.is_empty());
    assert_eq!(expanded, prompt);
}
