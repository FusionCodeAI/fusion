//! Repository context detection and `@file` mention expansion.
//!
//! Automatically loads instructions from `AGENTS.md` / `CLAUDE.md` and
//! expands inline file mentions (e.g. `@src/main.rs`) by injecting file content
//! directly into LLM prompts.

use std::collections::HashSet;
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};

/// Maximum file size allowed for mention injection (50 KB).
pub const MAX_FILE_SIZE_BYTES: usize = 50 * 1024;

/// Ordered list of candidate context files searched in the workspace.
pub const REPO_CONTEXT_CANDIDATES: &[&str] = &[
    "AGENTS.md",
    ".fusion/AGENTS.md",
    "CLAUDE.md",
    ".claude/CLAUDE.md",
];

/// Detects and loads repository context from standard instruction files.
///
/// Checks in order:
/// 1. `<workspace>/AGENTS.md`
/// 2. `<workspace>/.fusion/AGENTS.md`
/// 3. `<workspace>/CLAUDE.md`
/// 4. `<workspace>/.claude/CLAUDE.md`
///
/// If found and not empty, returns:
/// `"\n\n### Repository Context (from {}):\n{}"`
pub fn detect_and_load_repo_context(workspace_root: &Path) -> Option<String> {
    for rel_path in REPO_CONTEXT_CANDIDATES {
        let file_path = workspace_root.join(rel_path);
        if file_path.is_file() {
            if let Ok(content) = fs::read_to_string(&file_path) {
                let trimmed = content.trim();
                if !trimmed.is_empty() {
                    return Some(format!(
                        "\n\n### Repository Context (from {}):\n{}",
                        rel_path, trimmed
                    ));
                }
            }
        }
    }
    None
}

/// Checks if preceding character is a valid prefix for an `@file` mention trigger.
pub fn is_valid_at_prefix(prev: char) -> bool {
    !prev.is_alphanumeric() && prev != '_'
}

/// Trailing punctuation characters stripped from raw `@mention` tokens.
const TRAILING_PUNCTUATION: &[char] = &[
    '.', ',', ':', ';', '?', '!', ')', ']', '}', '>', '\'', '"', '`',
];

/// Scans a user prompt and extracts potential `@path` candidate strings.
pub fn extract_mention_candidates(prompt: &str) -> Vec<String> {
    let mut candidates = Vec::new();
    let chars: Vec<(usize, char)> = prompt.char_indices().collect();
    let n = chars.len();

    let mut i = 0;
    while i < n {
        let (_, ch) = chars[i];
        if ch == '@' {
            let valid_prefix = if i == 0 {
                true
            } else {
                let (_, prev_ch) = chars[i - 1];
                is_valid_at_prefix(prev_ch)
            };

            if valid_prefix && i + 1 < n {
                let (_, next_ch) = chars[i + 1];

                // Handle quoted paths: @"path/with spaces" or @'path/with spaces'
                if next_ch == '"' || next_ch == '\'' {
                    let quote = next_ch;
                    let start = i + 2;
                    let mut end = start;
                    while end < n && chars[end].1 != quote {
                        end += 1;
                    }
                    if end < n && chars[end].1 == quote {
                        let start_byte = chars[start].0;
                        let end_byte = chars[end].0;
                        let raw = &prompt[start_byte..end_byte];
                        if !raw.trim().is_empty() {
                            candidates.push(raw.to_string());
                        }
                        i = end + 1;
                        continue;
                    }
                }

                // Unquoted mention: scan until whitespace
                let start = i + 1;
                let mut end = start;
                while end < n && !chars[end].1.is_whitespace() {
                    end += 1;
                }
                let start_byte = chars[start].0;
                let end_byte = if end < n { chars[end].0 } else { prompt.len() };
                let raw = &prompt[start_byte..end_byte];
                if !raw.is_empty() {
                    candidates.push(raw.to_string());
                }
                i = end;
                continue;
            }
        }
        i += 1;
    }

    candidates
}

/// Reads a file capped to `MAX_FILE_SIZE_BYTES` (50 KB), returning `None` if it is binary.
fn read_file_capped(file_path: &Path) -> Option<String> {
    let mut file = fs::File::open(file_path).ok()?;
    let mut buffer = Vec::new();

    // Read up to MAX_FILE_SIZE_BYTES
    file.take(MAX_FILE_SIZE_BYTES as u64)
        .read_to_end(&mut buffer)
        .ok()?;

    if buffer.is_empty() {
        return Some(String::new());
    }

    // Binary probe: check first 4096 bytes for null bytes
    let sniff_len = buffer.len().min(4096);
    if buffer[..sniff_len].contains(&0) {
        return None;
    }

    // Attempt to decode as UTF-8, backing up to char boundary if truncated
    let mut end = buffer.len();
    while end > 0 {
        if let Ok(s) = std::str::from_utf8(&buffer[..end]) {
            return Some(s.to_string());
        }
        end -= 1;
    }

    None
}

/// Resolves a candidate string to an existing file path under `workspace_root`.
///
/// Tries the exact string first, then trims trailing sentence punctuation.
/// Returns the normalized relative/mentioned path string and the resolved PathBuf on disk.
fn resolve_candidate_path(candidate: &str, workspace_root: &Path) -> Option<(String, PathBuf)> {
    let candidate = candidate.trim();
    if candidate.is_empty() {
        return None;
    }

    // 1. Try exact candidate
    let clean = candidate.strip_prefix("./").unwrap_or(candidate);
    let target = workspace_root.join(clean);
    if target.is_file() {
        return Some((clean.to_string(), target));
    }

    // 2. Try trimming trailing punctuation (e.g. `@src/main.rs,` -> `src/main.rs`)
    let trimmed = candidate.trim_end_matches(TRAILING_PUNCTUATION);
    if !trimmed.is_empty() && trimmed != candidate {
        let clean = trimmed.strip_prefix("./").unwrap_or(trimmed);
        let target = workspace_root.join(clean);
        if target.is_file() {
            return Some((clean.to_string(), target));
        }
    }

    None
}

/// Expands `@path` file mentions in a user prompt.
///
/// Scans `user_prompt` for `@path` patterns (e.g. `@src/main.rs`, `@Cargo.toml`).
/// For each mention where `workspace_root.join(path)` is a valid file:
/// - Reads the file (capped to max 50KB to prevent context blowout)
/// - Skips binaries (files containing NUL bytes or invalid UTF-8)
/// - Appends a `<file-mention path="...">\n...\n</file-mention>` block
///
/// Returns the expanded prompt string and the list of injected file paths.
pub fn expand_file_mentions(user_prompt: &str, workspace_root: &Path) -> (String, Vec<PathBuf>) {
    let candidates = extract_mention_candidates(user_prompt);
    if candidates.is_empty() {
        return (user_prompt.to_string(), Vec::new());
    }

    let mut injected_paths = Vec::new();
    let mut seen_paths = HashSet::new();
    let mut mention_blocks = Vec::new();

    for candidate in candidates {
        if let Some((clean_path, target_file)) = resolve_candidate_path(&candidate, workspace_root)
        {
            let path_buf = PathBuf::from(&clean_path);
            if seen_paths.insert(path_buf.clone()) {
                if let Some(content) = read_file_capped(&target_file) {
                    let block = if content.is_empty() {
                        format!("<file-mention path=\"{}\">\n</file-mention>", clean_path)
                    } else if content.ends_with('\n') {
                        format!(
                            "<file-mention path=\"{}\">\n{}</file-mention>",
                            clean_path, content
                        )
                    } else {
                        format!(
                            "<file-mention path=\"{}\">\n{}\n</file-mention>",
                            clean_path, content
                        )
                    };

                    mention_blocks.push(block);
                    injected_paths.push(path_buf);
                }
            }
        }
    }

    if mention_blocks.is_empty() {
        return (user_prompt.to_string(), Vec::new());
    }

    let mut expanded = user_prompt.to_string();
    for block in mention_blocks {
        if !expanded.ends_with("\n\n") {
            if expanded.ends_with('\n') {
                expanded.push('\n');
            } else {
                expanded.push_str("\n\n");
            }
        }
        expanded.push_str(&block);
    }

    (expanded, injected_paths)
}
