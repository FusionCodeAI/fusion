use async_trait::async_trait;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

use crate::tools::edit::resolve_path;
use crate::tools::types::{Tool, ToolContext};

/// Options for controlling patch application.
#[derive(Debug, Clone)]
pub struct PatchOptions {
    /// Maximum fuzz factor (number of leading/trailing context lines to ignore when matching).
    pub fuzz: usize,
    /// If true, test application without writing changes to disk.
    pub dry_run: bool,
    /// If true, apply the patch in reverse (undo patch).
    pub reverse: bool,
    /// Number of leading path components to strip from file names (default: 1, e.g. "a/foo.rs" -> "foo.rs").
    pub strip: usize,
    /// Optional override for target file path (useful when patch only applies to one file or diff headers lack path).
    pub target_path: Option<PathBuf>,
}

impl Default for PatchOptions {
    fn default() -> Self {
        Self {
            fuzz: 2,
            dry_run: false,
            reverse: false,
            strip: 1,
            target_path: None,
        }
    }
}

/// A line in a diff hunk.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HunkLine {
    /// Unmodified context line (prefixed with ' ')
    Context(String),
    /// Added line (prefixed with '+')
    Add(String),
    /// Removed line (prefixed with '-')
    Remove(String),
}

impl HunkLine {
    pub fn content(&self) -> &str {
        match self {
            HunkLine::Context(s) | HunkLine::Add(s) | HunkLine::Remove(s) => s.as_str(),
        }
    }
}

/// A single hunk in a unified diff.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hunk {
    pub old_start: usize,
    pub old_lines: usize,
    pub new_start: usize,
    pub new_lines: usize,
    pub header: String,
    pub lines: Vec<HunkLine>,
}

impl Hunk {
    /// Returns the expected original lines (Context and Remove) for matching.
    pub fn expected_old_lines(&self, reverse: bool) -> Vec<&str> {
        self.lines
            .iter()
            .filter_map(|l| match (l, reverse) {
                (HunkLine::Context(s), _) => Some(s.as_str()),
                (HunkLine::Remove(s), false) => Some(s.as_str()),
                (HunkLine::Add(s), true) => Some(s.as_str()),
                _ => None,
            })
            .collect()
    }
}

/// A patch affecting a single file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FilePatch {
    pub old_path: Option<String>,
    pub new_path: Option<String>,
    pub is_new: bool,
    pub is_deleted: bool,
    pub hunks: Vec<Hunk>,
}

impl FilePatch {
    /// Determine the best path to use, taking into account `strip` count.
    pub fn target_path(&self, strip: usize, reverse: bool) -> Option<PathBuf> {
        let raw = if reverse {
            self.old_path.as_deref().or(self.new_path.as_deref())
        } else {
            self.new_path.as_deref().or(self.old_path.as_deref())
        }?;

        if raw == "/dev/null" {
            // Check opposite
            let opp = if reverse {
                self.new_path.as_deref()
            } else {
                self.old_path.as_deref()
            }?;
            return strip_components(opp, strip);
        }

        strip_components(raw, strip)
    }
}

/// Normalize a path from a diff header for cross-platform handling:
/// - strips surrounding double quotes (git quotes paths with special characters)
/// - converts Windows-style backslash separators to `/`
/// - preserves Windows verbatim/device prefixes (`\\?\`, `\\.\`) untouched
pub fn normalize_diff_path(path_str: &str) -> String {
    let s = path_str.trim();
    let s = if s.len() >= 2 && s.starts_with('"') && s.ends_with('"') {
        &s[1..s.len() - 1]
    } else {
        s
    };
    if s.starts_with("\\\\?\\") || s.starts_with("\\\\.\\") {
        return s.to_string();
    }
    s.replace('\\', "/")
}

/// Splits a leading Windows drive prefix (`C:/`) from a normalized path.
/// Returns `("", path)` when no drive prefix is present.
fn split_drive_prefix(path: &str) -> (&str, &str) {
    let b = path.as_bytes();
    if b.len() >= 3 && b[0].is_ascii_alphabetic() && b[1] == b':' && (b[2] == b'/' || b[2] == b'\\')
    {
        path.split_at(3)
    } else {
        ("", path)
    }
}

fn strip_components(path_str: &str, strip: usize) -> Option<PathBuf> {
    let normalized = normalize_diff_path(path_str);
    // A Windows drive prefix is not a real component: never count it toward `strip`.
    let (_, without_drive) = split_drive_prefix(&normalized);
    let mut components = Path::new(without_drive).components();
    for _ in 0..strip {
        components.next()?;
    }
    let res = components.as_path();
    if res.as_os_str().is_empty() {
        Some(PathBuf::from(without_drive))
    } else {
        Some(res.to_path_buf())
    }
}

/// Flushes any open hunk into the current file and the current file into `files`.
fn flush_pending(
    current_file: &mut Option<FilePatch>,
    current_hunk: &mut Option<Hunk>,
    files: &mut Vec<FilePatch>,
) {
    if let Some(hunk) = current_hunk.take() {
        if let Some(file) = &mut *current_file {
            file.hunks.push(hunk);
        }
    }
    if let Some(file) = current_file.take() {
        files.push(file);
    }
}

/// Status and statistics for an applied hunk.
#[derive(Debug, Clone)]
pub struct HunkReport {
    pub hunk_index: usize,
    pub applied_at_line: usize,
    pub line_offset: isize,
    pub fuzz_used: usize,
    pub relaxed_whitespace: bool,
}

/// Result of applying a patch to a single file.
#[derive(Debug, Clone)]
pub struct FileApplyResult {
    pub path: PathBuf,
    pub is_new: bool,
    pub is_deleted: bool,
    pub hunks: Vec<HunkReport>,
    pub additions: usize,
    pub deletions: usize,
    pub modified_content: Option<String>,
}

/// Full result of applying a multi-file patch.
#[derive(Debug, Clone)]
pub struct PatchResult {
    pub files: Vec<FileApplyResult>,
    pub dry_run: bool,
}

// ---------------------------------------------------------------------------
// Unified Diff Parser
// ---------------------------------------------------------------------------

/// Parse a unified diff or git patch string into a list of `FilePatch` structs.
pub fn parse_unified_diff(diff_str: &str) -> anyhow::Result<Vec<FilePatch>> {
    let mut files = Vec::new();
    let mut current_file: Option<FilePatch> = None;
    let mut current_hunk: Option<Hunk> = None;

    let lines: Vec<&str> = diff_str.lines().collect();
    let mut idx = 0;

    while idx < lines.len() {
        let line = lines[idx];

        // Detect git diff header: diff --git a/foo b/foo (also svn/mercurial style `diff -r`/`diff -u`)
        if line.starts_with("diff --git ")
            || line.starts_with("diff -r")
            || line.starts_with("diff -u")
            || line.starts_with("diff -N")
            || line.starts_with("diff -c")
        {
            flush_pending(&mut current_file, &mut current_hunk, &mut files);
            let parts = split_git_header_args(&line["diff --git ".len()..]);
            let (old_p, new_p) = if parts.len() >= 2 {
                (
                    Some(normalize_diff_path(&parts[0])),
                    Some(normalize_diff_path(&parts[1])),
                )
            } else {
                (None, None)
            };

            current_file = Some(FilePatch {
                old_path: old_p,
                new_path: new_p,
                is_new: false,
                is_deleted: false,
                hunks: Vec::new(),
            });
            idx += 1;
            continue;
        }

        // Detect new file mode / deleted file mode
        if line.starts_with("new file mode") {
            if let Some(file) = &mut current_file {
                file.is_new = true;
            }
            idx += 1;
            continue;
        }
        if line.starts_with("deleted file mode") {
            if let Some(file) = &mut current_file {
                file.is_deleted = true;
            }
            idx += 1;
            continue;
        }

        // Detect file header: --- old_path
        // A `---` header closes any open hunk. If the open file patch already
        // has hunks (multi-file plain diff without `diff --git` separators),
        // start a fresh FilePatch for the next file instead of overwriting its
        // paths.
        if line.starts_with("--- ") {
            if let Some(hunk) = current_hunk.take() {
                if let Some(file) = &mut current_file {
                    file.hunks.push(hunk);
                }
            }
            let starts_new_file = current_file.as_ref().is_some_and(|f| !f.hunks.is_empty());
            if starts_new_file {
                if let Some(file) = current_file.take() {
                    files.push(file);
                }
            }

            let old_p = &line[4..];
            // Strip quotes and normalize separators for cross-platform paths.
            let old_clean = normalize_diff_path(old_p.split('\t').next().unwrap_or(old_p));

            if current_file.is_none() {
                current_file = Some(FilePatch {
                    old_path: Some(old_clean.clone()),
                    new_path: None,
                    is_new: old_clean == "/dev/null",
                    is_deleted: false,
                    hunks: Vec::new(),
                });
            } else if let Some(file) = &mut current_file {
                file.old_path = Some(old_clean);
                if file.old_path.as_deref() == Some("/dev/null") {
                    file.is_new = true;
                }
            }
            idx += 1;
            continue;
        }

        // Detect file header: +++ new_path
        if line.starts_with("+++ ") {
            let new_p = &line[4..];
            let new_clean = normalize_diff_path(new_p.split('\t').next().unwrap_or(new_p));

            if let Some(file) = &mut current_file {
                file.new_path = Some(new_clean);
                if file.new_path.as_deref() == Some("/dev/null") {
                    file.is_deleted = true;
                }
            } else {
                current_file = Some(FilePatch {
                    old_path: None,
                    new_path: Some(new_clean.clone()),
                    is_new: false,
                    is_deleted: new_clean == "/dev/null",
                    hunks: Vec::new(),
                });
            }
            idx += 1;
            continue;
        }

        // Detect hunk header: @@ -old_start,old_lines +new_start,new_lines @@
        if line.starts_with("@@ ") {
            if let Some(hunk) = current_hunk.take() {
                if let Some(file) = &mut current_file {
                    file.hunks.push(hunk);
                }
            }

            if current_file.is_none() {
                // Anonymous file patch (single file with implicit path)
                current_file = Some(FilePatch {
                    old_path: None,
                    new_path: None,
                    is_new: false,
                    is_deleted: false,
                    hunks: Vec::new(),
                });
            }

            let hunk = parse_hunk_header(line)?;
            current_hunk = Some(hunk);
            idx += 1;
            continue;
        }

        // Inside a hunk
        if let Some(hunk) = &mut current_hunk {
            if line.starts_with('+') {
                hunk.lines.push(HunkLine::Add(line[1..].to_string()));
            } else if line.starts_with('-') {
                hunk.lines.push(HunkLine::Remove(line[1..].to_string()));
            } else if line.starts_with(' ') {
                hunk.lines.push(HunkLine::Context(line[1..].to_string()));
            } else if line.is_empty() {
                // LLMs sometimes omit leading space for empty context lines
                hunk.lines.push(HunkLine::Context(String::new()));
            } else if line.starts_with('\\') {
                // e.g. "\ No newline at end of file" - ignore metadata line
            } else {
                // Stray non-hunk content (e.g. `diff -ruN` command lines or
                // `Index:` lines from svn-style diffs): close the hunk so the
                // next header round starts a fresh file patch.
                flush_pending(&mut current_file, &mut current_hunk, &mut files);
            }
        }

        idx += 1;
    }

    // Flush any pending hunk and file
    flush_pending(&mut current_file, &mut current_hunk, &mut files);

    if files.is_empty() {
        anyhow::bail!("No valid diff hunks or file headers found in patch text.");
    }

    Ok(files)
}

/// Splits git `diff --git` header arguments, honoring double-quoted paths
/// (git quotes paths containing spaces or special characters) and backslash
/// escapes inside quotes.
fn split_git_header_args(rest: &str) -> Vec<String> {
    let mut parts = Vec::new();
    let mut cur = String::new();
    let mut in_quotes = false;
    let mut has_token = false;
    let mut chars = rest.chars();
    while let Some(c) = chars.next() {
        match c {
            '"' => {
                in_quotes = !in_quotes;
                has_token = true;
            }
            '\\' if in_quotes => {
                if let Some(esc) = chars.next() {
                    cur.push(esc);
                }
                has_token = true;
            }
            ' ' | '\t' if !in_quotes => {
                if has_token {
                    parts.push(std::mem::take(&mut cur));
                    has_token = false;
                }
            }
            _ => {
                cur.push(c);
                has_token = true;
            }
        }
    }
    if has_token {
        parts.push(cur);
    }
    parts
}

fn parse_hunk_header(header: &str) -> anyhow::Result<Hunk> {
    // Format: @@ -old_start[,old_lines] +new_start[,new_lines] @@ [optional text]
    let parts: Vec<&str> = header.split("@@").collect();
    if parts.len() < 3 {
        anyhow::bail!("Malformed hunk header: '{}'", header);
    }

    let range_str = parts[1].trim();
    let mut range_parts = range_str.split_whitespace();

    let old_range = range_parts
        .next()
        .ok_or_else(|| anyhow::anyhow!("Missing old range in hunk header: '{}'", header))?;
    let new_range = range_parts
        .next()
        .ok_or_else(|| anyhow::anyhow!("Missing new range in hunk header: '{}'", header))?;

    let (old_start, old_lines) = parse_range(old_range, '-')?;
    let (new_start, new_lines) = parse_range(new_range, '+')?;

    Ok(Hunk {
        old_start,
        old_lines,
        new_start,
        new_lines,
        header: header.to_string(),
        lines: Vec::new(),
    })
}

fn parse_range(range_part: &str, prefix: char) -> anyhow::Result<(usize, usize)> {
    let s = range_part.strip_prefix(prefix).unwrap_or(range_part);
    if let Some((start_s, count_s)) = s.split_once(',') {
        let start: usize = start_s.parse()?;
        let count: usize = count_s.parse()?;
        Ok((start, count))
    } else {
        let start: usize = s.parse()?;
        Ok((start, 1))
    }
}

// ---------------------------------------------------------------------------
// Fuzzy Matching & Hunk Application Engine
// ---------------------------------------------------------------------------

/// Normalize a line for relaxed whitespace comparison:
/// - strip CR
/// - trim trailing whitespace
fn normalize_relaxed(s: &str) -> &str {
    s.trim_end_matches(|c| c == '\r' || c == ' ' || c == '\t')
}

/// Normalize whitespace aggressively:
/// - trim both ends
/// - collapse repeated internal whitespace
fn normalize_aggressive(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Match location candidates
struct MatchCandidate {
    line_index: usize,
    matched_lines_count: usize,
    fuzz: usize,
    relaxed_whitespace: bool,
    distance_from_target: usize,
    indent_shift: isize,
}

/// Attempt to find the best match location in `file_lines` for `expected` lines from a hunk.
fn find_hunk_location(
    file_lines: &[String],
    hunk: &Hunk,
    reverse: bool,
    max_fuzz: usize,
    target_line_idx: usize,
) -> anyhow::Result<MatchCandidate> {
    let expected = hunk.expected_old_lines(reverse);

    // If there are no expected lines to remove/context (pure addition e.g. at start/end of file)
    if expected.is_empty() {
        let clamped = target_line_idx.min(file_lines.len());
        return Ok(MatchCandidate {
            line_index: clamped,
            matched_lines_count: 0,
            fuzz: 0,
            relaxed_whitespace: false,
            distance_from_target: 0,
            indent_shift: 0,
        });
    }

    let mut best_match: Option<MatchCandidate> = None;

    // Search tiers:
    // Tier 0: exact match, fuzz 0
    // Tier 1: relaxed whitespace, fuzz 0
    // Tier 2: aggressive whitespace, fuzz 0
    // Tier 3..N: increasing fuzz factors 1..=max_fuzz
    for fuzz in 0..=max_fuzz {
        // Can we apply fuzz? We need enough context lines to shave off
        if fuzz * 2 >= expected.len() && expected.len() > 1 {
            // Cannot shave off more than total lines minus 1
            continue;
        }

        let slice = if fuzz == 0 {
            &expected[..]
        } else {
            &expected[fuzz..(expected.len() - fuzz)]
        };

        if slice.is_empty() {
            continue;
        }

        let slice_len = slice.len();
        if file_lines.len() < slice_len {
            continue;
        }

        // 1. Search for Exact Match
        for start_idx in 0..=(file_lines.len() - slice_len) {
            let matches = slice
                .iter()
                .zip(&file_lines[start_idx..start_idx + slice_len])
                .all(|(&exp, act)| exp == act.as_str());

            if matches {
                let dist = start_idx.abs_diff(target_line_idx);
                let cand = MatchCandidate {
                    line_index: start_idx,
                    matched_lines_count: slice_len,
                    fuzz,
                    relaxed_whitespace: false,
                    distance_from_target: dist,
                    indent_shift: 0,
                };

                if let Some(current) = &best_match {
                    if dist < current.distance_from_target {
                        best_match = Some(cand);
                    }
                } else {
                    best_match = Some(cand);
                }
            }
        }

        if best_match.is_some() {
            return Ok(best_match.unwrap());
        }

        // 2. Search for Relaxed Whitespace Match (trailing spaces & CR)
        for start_idx in 0..=(file_lines.len() - slice_len) {
            let matches = slice
                .iter()
                .zip(&file_lines[start_idx..start_idx + slice_len])
                .all(|(&exp, act)| normalize_relaxed(exp) == normalize_relaxed(act));

            if matches {
                let dist = start_idx.abs_diff(target_line_idx);
                let cand = MatchCandidate {
                    line_index: start_idx,
                    matched_lines_count: slice_len,
                    fuzz,
                    relaxed_whitespace: true,
                    distance_from_target: dist,
                    indent_shift: 0,
                };

                if let Some(current) = &best_match {
                    if dist < current.distance_from_target {
                        best_match = Some(cand);
                    }
                } else {
                    best_match = Some(cand);
                }
            }
        }

        if best_match.is_some() {
            return Ok(best_match.unwrap());
        }

        // 3. Search with Indentation and Collapsed Whitespace
        for start_idx in 0..=(file_lines.len() - slice_len) {
            let matches = slice
                .iter()
                .zip(&file_lines[start_idx..start_idx + slice_len])
                .all(|(&exp, act)| normalize_aggressive(exp) == normalize_aggressive(act));

            if matches {
                // Compute indentation shift between expected and actual
                let first_exp_indent = slice[0].len() - slice[0].trim_start().len();
                let first_act_indent =
                    file_lines[start_idx].len() - file_lines[start_idx].trim_start().len();
                let indent_shift = first_act_indent as isize - first_exp_indent as isize;

                let dist = start_idx.abs_diff(target_line_idx);
                let cand = MatchCandidate {
                    line_index: start_idx,
                    matched_lines_count: slice_len,
                    fuzz,
                    relaxed_whitespace: true,
                    distance_from_target: dist,
                    indent_shift,
                };

                if let Some(current) = &best_match {
                    if dist < current.distance_from_target {
                        best_match = Some(cand);
                    }
                } else {
                    best_match = Some(cand);
                }
            }
        }

        if best_match.is_some() {
            return Ok(best_match.unwrap());
        }
    }

    // No match found
    let snippet: Vec<&str> = expected.iter().take(3).copied().collect();
    anyhow::bail!(
        "Hunk failed to match: expected lines starting at original line {} ({:?}) not found in target file (fuzz factor up to {} tried).",
        hunk.old_start,
        snippet,
        max_fuzz
    );
}

/// Apply a single hunk to the target file lines.
fn apply_hunk(
    file_lines: &mut Vec<String>,
    hunk: &Hunk,
    reverse: bool,
    max_fuzz: usize,
    cumulative_offset: isize,
) -> anyhow::Result<HunkReport> {
    // Target 0-based line index in original file
    let base_target = hunk.old_start.saturating_sub(1);
    let target_idx = ((base_target as isize) + cumulative_offset).max(0) as usize;

    let candidate = find_hunk_location(file_lines, hunk, reverse, max_fuzz, target_idx)?;

    // Construct replacement lines
    let mut replacement = Vec::new();
    let fuzz = candidate.fuzz;
    let expected = hunk.expected_old_lines(reverse);

    // If fuzz was used, we need to preserve any skipped leading context lines from the target file
    // But since the candidate matched at line_index for the slice, the skipped leading context lines
    // were already in the file before line_index, so they naturally remain in file_lines[..candidate.line_index]!

    for line in &hunk.lines {
        match (line, reverse) {
            (HunkLine::Context(c), _) => {
                // If indent shift was detected, apply it
                if candidate.indent_shift != 0 {
                    replacement.push(adjust_indentation(c, candidate.indent_shift));
                } else {
                    replacement.push(c.clone());
                }
            }
            (HunkLine::Add(a), false) | (HunkLine::Remove(a), true) => {
                // Insertion
                if candidate.indent_shift != 0 {
                    replacement.push(adjust_indentation(a, candidate.indent_shift));
                } else {
                    replacement.push(a.clone());
                }
            }
            (HunkLine::Remove(_), false) | (HunkLine::Add(_), true) => {
                // Deletion - skipped from replacement
            }
        }
    }

    // If fuzz was used, the replacement lines correspond to the entire hunk.
    // However, candidate.matched_lines_count is only the count of the sliced expected lines.
    // If fuzz > 0:
    // The first `fuzz` context lines in the hunk were not in the slice.
    // The last `fuzz` context lines in the hunk were not in the slice.
    // So in `replacement`, we should strip the first `fuzz` context lines and last `fuzz` context lines
    // because those lines are already before `candidate.line_index` and after `candidate.line_index + candidate.matched_lines_count`.
    let final_replacement = if fuzz > 0 && expected.len() > fuzz * 2 {
        let mut trimmed = Vec::new();
        let mut context_count_front = 0;
        let mut remaining_lines: Vec<String> = replacement;

        // Remove first `fuzz` context lines
        let mut kept_after_front = Vec::new();
        for item in remaining_lines.drain(..) {
            if context_count_front < fuzz {
                context_count_front += 1;
            } else {
                kept_after_front.push(item);
            }
        }

        // Remove last `fuzz` context lines
        let mut context_count_back = 0;
        for item in kept_after_front.into_iter().rev() {
            if context_count_back < fuzz {
                context_count_back += 1;
            } else {
                trimmed.push(item);
            }
        }
        trimmed.reverse();
        trimmed
    } else {
        replacement
    };

    // Splice replacement into file_lines
    let start = candidate.line_index;
    let end = start + candidate.matched_lines_count;
    file_lines.splice(start..end, final_replacement);

    let offset = (candidate.line_index as isize) - (target_idx as isize);

    Ok(HunkReport {
        hunk_index: 0, // set by caller
        applied_at_line: candidate.line_index + 1,
        line_offset: offset,
        fuzz_used: candidate.fuzz,
        relaxed_whitespace: candidate.relaxed_whitespace,
    })
}

fn adjust_indentation(s: &str, shift: isize) -> String {
    if shift > 0 {
        format!("{}{}", " ".repeat(shift as usize), s)
    } else {
        let remove_count = (-shift) as usize;
        let mut trimmed = 0;
        let mut chars = s.chars();
        while trimmed < remove_count {
            match chars.clone().next() {
                Some(' ') => {
                    chars.next();
                    trimmed += 1;
                }
                Some('\t') => {
                    chars.next();
                    trimmed += 4; // approximate tab
                }
                _ => break,
            }
        }
        chars.collect()
    }
}

/// Apply a single FilePatch to a string content.
pub fn apply_file_patch_to_string(
    content: &str,
    patch: &FilePatch,
    options: &PatchOptions,
) -> anyhow::Result<(String, Vec<HunkReport>)> {
    let has_crlf = content.contains("\r\n");
    let has_trailing_newline = content.ends_with('\n');

    let mut file_lines: Vec<String> = content
        .lines()
        .map(|l| l.trim_end_matches('\r').to_string())
        .collect();

    // If file is empty and we have an empty line from split
    if content.is_empty() {
        file_lines.clear();
    }

    let mut cumulative_offset: isize = 0;
    let mut reports = Vec::new();

    for (i, hunk) in patch.hunks.iter().enumerate() {
        let lines_before = file_lines.len();
        let mut report = apply_hunk(
            &mut file_lines,
            hunk,
            options.reverse,
            options.fuzz,
            cumulative_offset,
        )?;
        report.hunk_index = i + 1;

        let lines_after = file_lines.len();
        let net_change = (lines_after as isize) - (lines_before as isize);
        cumulative_offset += net_change;

        reports.push(report);
    }

    let newline_sep = if has_crlf { "\r\n" } else { "\n" };
    let mut output = file_lines.join(newline_sep);
    if (has_trailing_newline || patch.is_new) && !output.is_empty() {
        output.push_str(newline_sep);
    }

    Ok((output, reports))
}

/// Apply a parsed `FilePatch` against disk files in the workspace.
pub async fn apply_file_patch(
    patch: &FilePatch,
    cwd: &Path,
    options: &PatchOptions,
) -> anyhow::Result<FileApplyResult> {
    let target = if let Some(override_path) = &options.target_path {
        override_path.clone()
    } else {
        patch
            .target_path(options.strip, options.reverse)
            .ok_or_else(|| anyhow::anyhow!("Cannot determine target file path from patch header"))?
    };

    let full_path = if target.is_absolute() {
        target.clone()
    } else {
        cwd.join(&target)
    };

    // Calculate additions & deletions
    let mut additions = 0;
    let mut deletions = 0;
    for hunk in &patch.hunks {
        for line in &hunk.lines {
            match (line, options.reverse) {
                (HunkLine::Add(_), false) | (HunkLine::Remove(_), true) => additions += 1,
                (HunkLine::Remove(_), false) | (HunkLine::Add(_), true) => deletions += 1,
                _ => {}
            }
        }
    }

    // Handle file deletion
    if (patch.is_deleted && !options.reverse) || (patch.is_new && options.reverse) {
        if !options.dry_run && full_path.exists() {
            tokio::fs::remove_file(&full_path).await?;
        }
        return Ok(FileApplyResult {
            path: target,
            is_new: false,
            is_deleted: true,
            hunks: Vec::new(),
            additions,
            deletions,
            modified_content: None,
        });
    }

    // Handle new file or modified file
    let existing_content = if full_path.exists() {
        tokio::fs::read_to_string(&full_path).await?
    } else if patch.is_new || options.reverse {
        String::new()
    } else {
        return Err(anyhow::anyhow!(
            "Target file '{}' does not exist.",
            full_path.display()
        ));
    };

    let (new_content, reports) = apply_file_patch_to_string(&existing_content, patch, options)?;

    if !options.dry_run {
        if let Some(parent) = full_path.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }
        tokio::fs::write(&full_path, &new_content).await?;
    }

    Ok(FileApplyResult {
        path: target,
        is_new: patch.is_new && !options.reverse,
        is_deleted: false,
        hunks: reports,
        additions,
        deletions,
        modified_content: Some(new_content),
    })
}

/// Apply a multi-file patch string to workspace.
pub async fn apply_patch_string(
    diff_text: &str,
    cwd: &Path,
    options: &PatchOptions,
) -> anyhow::Result<PatchResult> {
    let file_patches = parse_unified_diff(diff_text)?;
    let mut results = Vec::new();

    for file_patch in &file_patches {
        let res = apply_file_patch(file_patch, cwd, options).await?;
        results.push(res);
    }

    Ok(PatchResult {
        files: results,
        dry_run: options.dry_run,
    })
}

// ---------------------------------------------------------------------------
// PatchTool (implements Tool trait)
// ---------------------------------------------------------------------------

#[derive(Default, Debug, Clone)]
pub struct PatchTool;

impl PatchTool {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl Tool for PatchTool {
    fn name(&self) -> &str {
        "patch"
    }

    fn description(&self) -> &str {
        "Apply a unified diff or multi-line git patch to workspace files with intelligent fuzzy matching (supports line offsets, whitespace variations, and context reduction)."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "patch": {
                    "type": "string",
                    "description": "Unified diff or git patch text containing hunk headers (@@ -start,len +start,len @@) and changes (+/-)."
                },
                "path": {
                    "type": "string",
                    "description": "Optional path to target file if diff headers omit file paths or to override the patch target."
                },
                "fuzz": {
                    "type": "integer",
                    "description": "Maximum fuzzy context lines to ignore when matching hunks (default: 2)."
                },
                "strip": {
                    "type": "integer",
                    "description": "Number of leading path components to strip from diff headers (e.g. 1 to strip 'a/' and 'b/'). Default: 1."
                },
                "dry_run": {
                    "type": "boolean",
                    "description": "If true, tests if patch applies cleanly without writing changes to disk."
                },
                "reverse": {
                    "type": "boolean",
                    "description": "If true, applies the patch in reverse (reverts changes)."
                }
            },
            "required": ["patch"]
        })
    }

    async fn execute(&self, args: Value, ctx: &ToolContext) -> anyhow::Result<String> {
        let patch_text = args
            .get("patch")
            .and_then(|v| v.as_str())
            .or_else(|| args.get("diff").and_then(|v| v.as_str()))
            .or_else(|| args.get("unified_diff").and_then(|v| v.as_str()))
            .ok_or_else(|| anyhow::anyhow!("Missing required parameter: patch"))?;

        let fuzz = args
            .get("fuzz")
            .and_then(|v| v.as_u64())
            .map(|v| v as usize)
            .unwrap_or(2);

        let strip = args
            .get("strip")
            .and_then(|v| v.as_u64())
            .map(|v| v as usize)
            .unwrap_or(1);

        let dry_run = args
            .get("dry_run")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        let reverse = args
            .get("reverse")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        let target_path = args
            .get("path")
            .and_then(|v| v.as_str())
            .or_else(|| args.get("file_path").and_then(|v| v.as_str()))
            .or_else(|| args.get("file").and_then(|v| v.as_str()))
            .map(|s| resolve_path(s, &ctx.cwd));

        let options = PatchOptions {
            fuzz,
            dry_run,
            reverse,
            strip,
            target_path,
        };

        let result = apply_patch_string(patch_text, &ctx.cwd, &options).await?;

        // Format a human and LLM-friendly summary
        let mut output = String::new();
        if dry_run {
            output.push_str("[DRY RUN] Patch validated successfully without disk modifications:\n");
        } else {
            output.push_str("Patch applied successfully:\n");
        }

        for file in &result.files {
            let status = if file.is_new {
                "created"
            } else if file.is_deleted {
                "deleted"
            } else {
                "modified"
            };

            output.push_str(&format!(
                "- {} ({}, +{} -{} lines across {} hunk{}):\n",
                file.path.display(),
                status,
                file.additions,
                file.deletions,
                file.hunks.len(),
                if file.hunks.len() == 1 { "" } else { "s" }
            ));

            for hunk in &file.hunks {
                let mut details = Vec::new();
                if hunk.line_offset != 0 {
                    details.push(format!("offset: {:+}", hunk.line_offset));
                }
                if hunk.fuzz_used > 0 {
                    details.push(format!("fuzz: {}", hunk.fuzz_used));
                }
                if hunk.relaxed_whitespace {
                    details.push("relaxed whitespace".to_string());
                }

                let details_str = if details.is_empty() {
                    "clean".to_string()
                } else {
                    details.join(", ")
                };

                output.push_str(&format!(
                    "  • Hunk #{} applied at line {} ({})\n",
                    hunk.hunk_index, hunk.applied_at_line, details_str
                ));
            }
        }

        Ok(output)
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_unified_diff_basic() {
        let diff = r#"--- a/src/lib.rs
+++ b/src/lib.rs
@@ -1,3 +1,4 @@
 pub mod config;
+pub mod patch;
 pub mod tools;
"#;
        let patches = parse_unified_diff(diff).expect("Failed to parse diff");
        assert_eq!(patches.len(), 1);
        let p = &patches[0];
        assert_eq!(p.old_path.as_deref(), Some("a/src/lib.rs"));
        assert_eq!(p.new_path.as_deref(), Some("b/src/lib.rs"));
        assert_eq!(p.hunks.len(), 1);
        let h = &p.hunks[0];
        assert_eq!(h.old_start, 1);
        assert_eq!(h.lines.len(), 4);
    }

    #[test]
    fn test_apply_clean_patch() {
        let original = "line1\nline2\nline3\n";
        let diff = r#"--- a/file.txt
+++ b/file.txt
@@ -1,3 +1,3 @@
 line1
-line2
+line2_modified
 line3
"#;
        let patches = parse_unified_diff(diff).unwrap();
        let options = PatchOptions::default();
        let (res, reports) = apply_file_patch_to_string(original, &patches[0], &options).unwrap();
        assert_eq!(res, "line1\nline2_modified\nline3\n");
        assert_eq!(reports.len(), 1);
        assert_eq!(reports[0].fuzz_used, 0);
        assert_eq!(reports[0].line_offset, 0);
    }

    #[test]
    fn test_apply_patch_with_line_offset() {
        // Original has new lines added at the top, shifting the target lines down
        let original = "top_extra_1\ntop_extra_2\nline1\nline2\nline3\n";
        let diff = r#"--- a/file.txt
+++ b/file.txt
@@ -1,3 +1,4 @@
 line1
 line2
+line2_sub
 line3
"#;
        let patches = parse_unified_diff(diff).unwrap();
        let options = PatchOptions::default();
        let (res, reports) = apply_file_patch_to_string(original, &patches[0], &options).unwrap();
        assert_eq!(
            res,
            "top_extra_1\ntop_extra_2\nline1\nline2\nline2_sub\nline3\n"
        );
        assert_eq!(reports.len(), 1);
        assert_eq!(reports[0].line_offset, 2);
    }

    #[test]
    fn test_apply_patch_relaxed_whitespace() {
        // Original has CRLF and trailing spaces
        let original = "line1  \r\nline2\r\nline3\r\n";
        let diff = r#"--- a/file.txt
+++ b/file.txt
@@ -1,3 +1,3 @@
 line1
-line2
+line2_fixed
 line3
"#;
        let patches = parse_unified_diff(diff).unwrap();
        let options = PatchOptions::default();
        let (res, reports) = apply_file_patch_to_string(original, &patches[0], &options).unwrap();
        assert!(res.contains("line2_fixed"));
        assert_eq!(reports[0].relaxed_whitespace, true);
    }

    #[test]
    fn test_apply_patch_fuzz_context_reduction() {
        // Context line 1 has changed in the file, but inner context and removal match
        let original = "line1_changed_elsewhere\nline2\nline3\nline4\n";
        let diff = r#"--- a/file.txt
+++ b/file.txt
@@ -1,4 +1,4 @@
 line1
 line2
-line3
+line3_replaced
 line4
"#;
        let patches = parse_unified_diff(diff).unwrap();
        let mut options = PatchOptions::default();
        options.fuzz = 1;
        let (res, reports) = apply_file_patch_to_string(original, &patches[0], &options).unwrap();
        assert_eq!(
            res,
            "line1_changed_elsewhere\nline2\nline3_replaced\nline4\n"
        );
        assert_eq!(reports[0].fuzz_used, 1);
    }

    #[test]
    fn test_apply_patch_reverse() {
        let modified = "line1\nline2_modified\nline3\n";
        let diff = r#"--- a/file.txt
+++ b/file.txt
@@ -1,3 +1,3 @@
 line1
-line2
+line2_modified
 line3
"#;
        let patches = parse_unified_diff(diff).unwrap();
        let mut options = PatchOptions::default();
        options.reverse = true;
        let (res, _) = apply_file_patch_to_string(modified, &patches[0], &options).unwrap();
        assert_eq!(res, "line1\nline2\nline3\n");
    }

    #[test]
    fn test_normalize_diff_path_strips_quotes_and_backslashes() {
        // Windows-style separators normalized to forward slashes
        assert_eq!(normalize_diff_path("src\\lib\\main.rs"), "src/lib/main.rs");
        // Git-quoted paths (paths with spaces or special chars)
        assert_eq!(
            normalize_diff_path("\"my dir/file name.rs\""),
            "my dir/file name.rs"
        );
        // Verbatim/device prefixes preserved untouched
        assert_eq!(
            normalize_diff_path("\\\\?\\C:\\very\\long\\path"),
            "\\\\?\\C:\\very\\long\\path"
        );
        assert_eq!(
            normalize_diff_path("\\\\.\\PhysicalDrive0"),
            "\\\\.\\PhysicalDrive0"
        );
        // Plain POSIX path untouched
        assert_eq!(normalize_diff_path("a/src/lib.rs"), "a/src/lib.rs");
        // Tab-stripped upstream; quotes handled before separator normalization
        assert_eq!(normalize_diff_path("\"tabs\\there\""), "tabs/there");
    }

    #[test]
    fn test_strip_components_with_windows_paths() {
        // Backslash-separated path with a/ prefix
        assert_eq!(
            strip_components("a\\src\\lib.rs", 1),
            Some(PathBuf::from("src/lib.rs"))
        );
        // Drive prefix is not counted as a component
        assert_eq!(
            strip_components("C:/a/src/lib.rs", 1),
            Some(PathBuf::from("src/lib.rs"))
        );
        assert_eq!(
            strip_components("C:\\work\\a\\src\\lib.rs", 2),
            Some(PathBuf::from("src/lib.rs"))
        );
        // POSIX path still works as before
        assert_eq!(
            strip_components("a/src/lib.rs", 1),
            Some(PathBuf::from("src/lib.rs"))
        );
        // Stripping more components than exist yields None
        assert_eq!(strip_components("a/lib.rs", 5), None);
    }

    #[test]
    fn test_parse_git_diff_with_quoted_paths() {
        let diff = "diff --git \"a/dir with space/file.rs\" \"b/dir with space/file.rs\"\n\
                    index 0000000..e69de29\n\
                    --- \"a/dir with space/file.rs\"\n\
                    +++ \"b/dir with space/file.rs\"\n\
                    @@ -0,0 +1,1 @@\n\
                    +hello\n";
        let patches = parse_unified_diff(diff).unwrap();
        assert_eq!(patches.len(), 1);
        let p = &patches[0];
        assert_eq!(p.old_path.as_deref(), Some("a/dir with space/file.rs"));
        assert_eq!(p.new_path.as_deref(), Some("b/dir with space/file.rs"));
        assert_eq!(p.hunks.len(), 1);
        // Target path should resolve through the quoted name
        assert_eq!(
            p.target_path(1, false),
            Some(PathBuf::from("dir with space/file.rs"))
        );
    }

    #[test]
    fn test_parse_git_diff_with_windows_backslash_paths() {
        let diff = "diff --git a/src\\win\\mod.rs b/src\\win\\mod.rs\n\
                    --- a/src\\win\\mod.rs\n\
                    +++ b/src\\win\\mod.rs\n\
                    @@ -1,1 +1,2 @@\n\
                    existing\n\
                    +added\n";
        let patches = parse_unified_diff(diff).unwrap();
        assert_eq!(patches.len(), 1);
        let p = &patches[0];
        // Separators normalized at parse time
        assert_eq!(p.old_path.as_deref(), Some("a/src/win/mod.rs"));
        assert_eq!(
            p.target_path(1, false),
            Some(PathBuf::from("src/win/mod.rs"))
        );
    }

    #[test]
    fn test_parse_multi_file_plain_diff() {
        // Plain (non-git) multi-file diff: `---`/`+++` pairs only.
        let diff = "--- a/one.txt\n\
                    +++ b/one.txt\n\
                    @@ -1,1 +1,2 @@\n\
                    first\n\
                    +second\n\
                    --- a/two.txt\n\
                    +++ b/two.txt\n\
                    @@ -1,1 +1,1 @@\n\
                    -old\n\
                    +new\n";
        let patches = parse_unified_diff(diff).unwrap();
        assert_eq!(patches.len(), 2);
        assert_eq!(patches[0].old_path.as_deref(), Some("a/one.txt"));
        assert_eq!(patches[0].hunks.len(), 1);
        assert_eq!(patches[1].old_path.as_deref(), Some("a/two.txt"));
        assert_eq!(patches[1].hunks.len(), 1);
    }

    #[test]
    fn test_parse_diff_with_stray_content_between_hunks() {
        // svn-style `Index:` lines and `=====` separators between hunks
        // must not be swallowed as context lines, and should not corrupt parsing.
        let diff = "Index: foo.rs\n\
                    ===================================================================\n\
                    --- a/foo.rs\n\
                    +++ b/foo.rs\n\
                    @@ -1,1 +1,2 @@\n\
                    alpha\n\
                    +beta\n\
                    Index: bar.rs\n\
                    ===================================================================\n\
                    --- a/bar.rs\n\
                    +++ b/bar.rs\n\
                    @@ -1,1 +1,1 @@\n\
                    -gone\n\
                    +here\n";
        let patches = parse_unified_diff(diff).unwrap();
        assert_eq!(patches.len(), 2);
        assert_eq!(patches[0].old_path.as_deref(), Some("a/foo.rs"));
        // Context must not contain the stray Index line
        assert_eq!(patches[0].hunks[0].lines.len(), 2);
        assert_eq!(patches[1].old_path.as_deref(), Some("a/bar.rs"));
    }

    #[test]
    fn test_parse_new_and_deleted_file_modes() {
        let new_diff = "diff --git a/created.rs b/created.rs\n\
                        new file mode 100644\n\
                        --- /dev/null\n\
                        +++ b/created.rs\n\
                        @@ -0,0 +1,1 @@\n\
                        +brand new\n";
        let patches = parse_unified_diff(new_diff).unwrap();
        assert_eq!(patches.len(), 1);
        assert!(patches[0].is_new);
        assert!(!patches[0].is_deleted);

        let del_diff = "diff --git a/removed.rs b/removed.rs\n\
                        deleted file mode 100644\n\
                        --- a/removed.rs\n\
                        +++ /dev/null\n\
                        @@ -1,1 +0,0 @@\n\
                        -gone forever\n";
        let patches = parse_unified_diff(del_diff).unwrap();
        assert_eq!(patches.len(), 1);
        assert!(patches[0].is_deleted);
        assert!(!patches[0].is_new);
    }

    #[test]
    fn test_apply_patch_new_file_from_empty_content() {
        let diff = "--- /dev/null\n\
                    +++ b/fresh.txt\n\
                    @@ -0,0 +1,2 @@\n\
                    +first line\n\
                    +second line\n";
        let patches = parse_unified_diff(diff).unwrap();
        assert!(patches[0].is_new);
        let options = PatchOptions::default();
        let (res, reports) = apply_file_patch_to_string("", &patches[0], &options).unwrap();
        assert_eq!(res, "first line\nsecond line\n");
        assert_eq!(reports.len(), 1);
        assert_eq!(reports[0].applied_at_line, 1);
    }

    #[test]
    fn test_apply_patch_preserves_crlf() {
        let original = "alpha\r\nbeta\r\ngamma\r\n";
        let diff = "--- a/crlf.txt\n\
                    +++ b/crlf.txt\n\
                    @@ -1,3 +1,3 @@\n\
                    alpha\n\
                    -beta\n\
                    +beta_prime\n\
                    gamma\n";
        let patches = parse_unified_diff(diff).unwrap();
        let options = PatchOptions::default();
        let (res, _) = apply_file_patch_to_string(original, &patches[0], &options).unwrap();
        assert_eq!(res, "alpha\r\nbeta_prime\r\ngamma\r\n");
    }

    #[test]
    fn test_apply_patch_no_trailing_newline_preserved() {
        let original = "one\ntwo\nthree";
        let diff = "--- a/nonl.txt\n\
                    +++ b/nonl.txt\n\
                    @@ -1,3 +1,3 @@\n\
                    one\n\
                    -two\n\
                    +TWO\n\
                    three\n";
        let patches = parse_unified_diff(diff).unwrap();
        let options = PatchOptions::default();
        let (res, _) = apply_file_patch_to_string(original, &patches[0], &options).unwrap();
        assert_eq!(res, "one\nTWO\nthree");
    }

    #[test]
    fn test_parse_range_defaults() {
        // Single-line ranges omit the count: `@@ -5 +5 @@`
        let hunk = parse_hunk_header("@@ -5 +5 @@").unwrap();
        assert_eq!(hunk.old_start, 5);
        assert_eq!(hunk.old_lines, 1);
        assert_eq!(hunk.new_start, 5);
        assert_eq!(hunk.new_lines, 1);

        let hunk = parse_hunk_header("@@ -3,7 +3,8 @@ optional section text").unwrap();
        assert_eq!(hunk.old_start, 3);
        assert_eq!(hunk.old_lines, 7);
        assert_eq!(hunk.new_start, 3);
        assert_eq!(hunk.new_lines, 8);
        assert_eq!(hunk.header, "@@ -3,7 +3,8 @@ optional section text");
    }

    #[test]
    fn test_parse_range_empty_file_zero_start() {
        // Empty files use `@@ -0,0 +1,N @@`
        let hunk = parse_hunk_header("@@ -0,0 +1,3 @@").unwrap();
        assert_eq!(hunk.old_start, 0);
        assert_eq!(hunk.old_lines, 0);
        assert_eq!(hunk.new_start, 1);
        assert_eq!(hunk.new_lines, 3);
    }

    #[test]
    fn test_parse_invalid_hunk_header_errors() {
        assert!(parse_hunk_header("@@").is_err());
        assert!(parse_hunk_header("@@ only-one-range @@").is_err());
    }

    #[test]
    fn test_hunk_expected_old_lines_reverse() {
        let hunk = parse_hunk_header("@@ -1,3 +1,3 @@").unwrap();
        // Populate lines manually for the reverse check
        let hunk = Hunk {
            lines: vec![
                HunkLine::Context("ctx".to_string()),
                HunkLine::Remove("old".to_string()),
                HunkLine::Add("new".to_string()),
            ],
            ..hunk
        };
        assert_eq!(hunk.expected_old_lines(false), vec!["ctx", "old"]);
        assert_eq!(hunk.expected_old_lines(true), vec!["ctx", "new"]);
    }

    #[test]
    fn test_split_git_header_args_quoting() {
        let parts = split_git_header_args("a/simple.rs b/simple.rs");
        assert_eq!(parts, vec!["a/simple.rs", "b/simple.rs"]);

        let parts = split_git_header_args("\"a/with space.txt\" \"b/with space.txt\"");
        assert_eq!(parts, vec!["a/with space.txt", "b/with space.txt"]);

        let parts = split_git_header_args("\"a/esc\\\"q.txt\" b/plain.txt");
        assert_eq!(parts, vec!["a/esc\"q.txt", "b/plain.txt"]);
    }

    #[test]
    fn test_file_patch_target_path_dev_null() {
        // New file: old path is /dev/null, must fall back to the new path
        let patch = FilePatch {
            old_path: Some("/dev/null".to_string()),
            new_path: Some("b/src/new.rs".to_string()),
            is_new: true,
            is_deleted: false,
            hunks: Vec::new(),
        };
        assert_eq!(
            patch.target_path(1, false),
            Some(PathBuf::from("src/new.rs"))
        );

        // Deleted file: new path is /dev/null, must fall back to the old path
        let patch = FilePatch {
            old_path: Some("a/src/gone.rs".to_string()),
            new_path: Some("/dev/null".to_string()),
            is_new: false,
            is_deleted: true,
            hunks: Vec::new(),
        };
        assert_eq!(
            patch.target_path(1, false),
            Some(PathBuf::from("src/gone.rs"))
        );
    }
}
