//! Side-by-side terminal diff viewer.
//!
//! Parses unified diffs into structured side-by-side rows and renders
//! them with ANSI colors and Unicode box-drawing borders:
//!
//! ```text
//! ╭─ diff: src/main.rs ─────────────────────────────────────────╮
//! │ 10: - old_function();       │ 10: + new_function();         │
//! ╰─────────────────────────────┴───────────────────────────────╯
//! ```
//!
//! - Left column displays old/deleted lines in red (`\x1b[31m`).
//! - Right column displays new/added lines in green (`\x1b[32m`).
//! - Line numbers are displayed in dim gray (`\x1b[90m`).
//! - Middle column divider is `│`.
//! - Automatically handles terminal resizing and column width allocation.

use std::path::Path;
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

// ============================================================================
// ANSI Color Constants
// ============================================================================

mod ansi {
    pub const RESET: &str = "\x1b[0m";
    pub const RED: &str = "\x1b[31m";
    pub const GREEN: &str = "\x1b[32m";
    pub const DIM_GRAY: &str = "\x1b[90m";
}

// ============================================================================
// Data Types
// ============================================================================

/// Type of change represented in a single diff cell.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ChangeType {
    /// Context line unchanged in both versions.
    Context,
    /// Added line in the new version.
    Addition,
    /// Deleted line from the old version.
    Deletion,
    /// Empty cell for alignment padding.
    Empty,
}

/// A single cell (left or right side) of a side-by-side diff row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiffCell {
    /// 1-based line number in the corresponding source file.
    pub line_number: Option<usize>,
    /// Classification of this cell.
    pub change_type: ChangeType,
    /// Line content text (without unified diff +/- prefix).
    pub content: String,
}

impl DiffCell {
    /// Create an empty cell for alignment padding.
    pub fn empty() -> Self {
        Self {
            line_number: None,
            change_type: ChangeType::Empty,
            content: String::new(),
        }
    }

    /// Create an unchanged context cell.
    pub fn context(line_number: usize, content: impl Into<String>) -> Self {
        Self {
            line_number: Some(line_number),
            change_type: ChangeType::Context,
            content: content.into(),
        }
    }

    /// Create a deletion cell (left column).
    pub fn deletion(line_number: usize, content: impl Into<String>) -> Self {
        Self {
            line_number: Some(line_number),
            change_type: ChangeType::Deletion,
            content: content.into(),
        }
    }

    /// Create an addition cell (right column).
    pub fn addition(line_number: usize, content: impl Into<String>) -> Self {
        Self {
            line_number: Some(line_number),
            change_type: ChangeType::Addition,
            content: content.into(),
        }
    }

    /// Returns `true` if this cell has no content.
    pub fn is_empty(&self) -> bool {
        self.change_type == ChangeType::Empty
    }
}

/// A synchronized side-by-side row containing a left cell and a right cell.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SideBySideRow {
    pub left: DiffCell,
    pub right: DiffCell,
}

/// Diff entries and metadata for an individual file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileDiff {
    pub old_path: Option<String>,
    pub new_path: Option<String>,
    pub rows: Vec<SideBySideRow>,
}

impl FileDiff {
    /// Best display path for this file.
    pub fn display_path(&self) -> &str {
        if let Some(p) = &self.new_path {
            if p != "/dev/null" && !p.is_empty() {
                return p.as_str();
            }
        }
        if let Some(p) = &self.old_path {
            if p != "/dev/null" && !p.is_empty() {
                return p.as_str();
            }
        }
        "unknown"
    }

    /// Find the highest line number present in this file's rows.
    pub fn max_line_number(&self) -> usize {
        let mut max = 0;
        for row in &self.rows {
            if let Some(n) = row.left.line_number {
                if n > max {
                    max = n;
                }
            }
            if let Some(n) = row.right.line_number {
                if n > max {
                    max = n;
                }
            }
        }
        max
    }
}

// ============================================================================
// SideBySideDiffViewer
// ============================================================================

/// Side-by-side diff parser and terminal renderer.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SideBySideDiffViewer {
    pub files: Vec<FileDiff>,
}

impl SideBySideDiffViewer {
    /// Create a new empty diff viewer.
    pub fn new() -> Self {
        Self { files: Vec::new() }
    }

    /// Parses unified diff text into a structured `SideBySideDiffViewer`.
    pub fn parse(diff_text: &str) -> Self {
        let mut files: Vec<FileDiff> = Vec::new();
        let mut current_file: Option<FileDiff> = None;

        let mut old_line_num: usize = 1;
        let mut new_line_num: usize = 1;
        let mut pending_deletions: Vec<(usize, String)> = Vec::new();
        let mut pending_additions: Vec<(usize, String)> = Vec::new();

        let flush_pending = |file: &mut FileDiff,
                             dels: &mut Vec<(usize, String)>,
                             adds: &mut Vec<(usize, String)>| {
            if dels.is_empty() && adds.is_empty() {
                return;
            }
            let count = dels.len().max(adds.len());
            for i in 0..count {
                let left = if i < dels.len() {
                    DiffCell::deletion(dels[i].0, &dels[i].1)
                } else {
                    DiffCell::empty()
                };
                let right = if i < adds.len() {
                    DiffCell::addition(adds[i].0, &adds[i].1)
                } else {
                    DiffCell::empty()
                };
                file.rows.push(SideBySideRow { left, right });
            }
            dels.clear();
            adds.clear();
        };

        for raw_line in diff_text.lines() {
            let line = raw_line.strip_suffix('\r').unwrap_or(raw_line);

            // 1. Check for git header or file headers
            if line.starts_with("diff --git ") {
                if let Some(mut file) = current_file.take() {
                    flush_pending(&mut file, &mut pending_deletions, &mut pending_additions);
                    if !file.rows.is_empty() || file.new_path.is_some() || file.old_path.is_some() {
                        files.push(file);
                    }
                }

                let (old_p, new_p) = parse_git_diff_header(&line["diff --git ".len()..]);
                current_file = Some(FileDiff {
                    old_path: old_p,
                    new_path: new_p,
                    rows: Vec::new(),
                });
                continue;
            }

            if line.starts_with("--- ") {
                let path_str = line["--- ".len()..].trim();
                let clean = clean_path(path_str);
                if let Some(file) = &mut current_file {
                    file.old_path = Some(clean);
                } else {
                    current_file = Some(FileDiff {
                        old_path: Some(clean),
                        new_path: None,
                        rows: Vec::new(),
                    });
                }
                continue;
            }

            if line.starts_with("+++ ") {
                let path_str = line["+++ ".len()..].trim();
                let clean = clean_path(path_str);
                if let Some(file) = &mut current_file {
                    file.new_path = Some(clean);
                } else {
                    current_file = Some(FileDiff {
                        old_path: None,
                        new_path: Some(clean),
                        rows: Vec::new(),
                    });
                }
                continue;
            }

            // 2. Check for hunk header @@ -old,len +new,len @@
            if line.starts_with("@@ ") {
                if current_file.is_none() {
                    current_file = Some(FileDiff {
                        old_path: None,
                        new_path: None,
                        rows: Vec::new(),
                    });
                }

                if let Some(file) = &mut current_file {
                    flush_pending(file, &mut pending_deletions, &mut pending_additions);
                }

                if let Some((old_start, new_start)) = parse_hunk_header(line) {
                    old_line_num = old_start;
                    new_line_num = new_start;
                }
                continue;
            }

            // 3. Skip metadata lines (index, new file mode, etc.)
            if line.starts_with("index ")
                || line.starts_with("new file mode ")
                || line.starts_with("deleted file mode ")
                || line.starts_with("similarity index ")
                || line.starts_with("rename from ")
                || line.starts_with("rename to ")
                || line.starts_with('\\')
            {
                continue;
            }

            // 4. Hunk lines (+, -, ' ', or empty context line)
            if let Some(file) = &mut current_file {
                if let Some(rest) = line.strip_prefix('-') {
                    // If we were already collecting additions, flush before starting new deletions
                    if !pending_additions.is_empty() {
                        flush_pending(file, &mut pending_deletions, &mut pending_additions);
                    }
                    pending_deletions.push((old_line_num, rest.to_string()));
                    old_line_num += 1;
                } else if let Some(rest) = line.strip_prefix('+') {
                    pending_additions.push((new_line_num, rest.to_string()));
                    new_line_num += 1;
                } else {
                    // Context line (starts with ' ' or empty line inside hunk)
                    let content = if let Some(c) = line.strip_prefix(' ') {
                        c
                    } else if line.is_empty() {
                        ""
                    } else {
                        continue;
                    };

                    flush_pending(file, &mut pending_deletions, &mut pending_additions);
                    file.rows.push(SideBySideRow {
                        left: DiffCell::context(old_line_num, content),
                        right: DiffCell::context(new_line_num, content),
                    });
                    old_line_num += 1;
                    new_line_num += 1;
                }
            }
        }

        if let Some(mut file) = current_file.take() {
            flush_pending(&mut file, &mut pending_deletions, &mut pending_additions);
            if !file.rows.is_empty() || file.new_path.is_some() || file.old_path.is_some() {
                files.push(file);
            }
        }

        Self { files }
    }

    /// Returns `true` if there are no files or all files have no diff rows.
    pub fn is_empty(&self) -> bool {
        self.files.is_empty() || self.files.iter().all(|f| f.rows.is_empty())
    }

    /// Slice of files parsed.
    pub fn files(&self) -> &[FileDiff] {
        &self.files
    }

    /// Total count of side-by-side rows across all files.
    pub fn total_rows(&self) -> usize {
        self.files.iter().map(|f| f.rows.len()).sum()
    }

    /// Renders side-by-side comparison with vertical separator `│` and header bar.
    pub fn render(&self, terminal_width: usize) -> String {
        if self.is_empty() {
            return String::new();
        }

        // Clamp width to safe terminal dimensions (minimum 20 columns)
        let effective_width = if terminal_width == 0 {
            80
        } else {
            terminal_width.max(20)
        };

        // Total overhead for borders: Left border (1) + Center divider (1) + Right border (1) = 3
        let content_width = effective_width.saturating_sub(3);
        let left_col_width = content_width / 2;
        let right_col_width = content_width.saturating_sub(left_col_width);

        let mut output = String::new();

        for (file_idx, file) in self.files.iter().enumerate() {
            if file.rows.is_empty() {
                continue;
            }

            if file_idx > 0 {
                output.push('\n');
            }

            // Determine line number column width for this file
            let max_line = file.max_line_number();
            let lineno_width = max_line.to_string().len().max(2);

            // 1. Top Header Bar: ╭─ diff: <file> ───╮
            let path_display = file.display_path();
            let header_prefix = format!("╭─ diff: {path_display} ");
            let prefix_vis = visible_width(&header_prefix);

            let header_line = if prefix_vis + 1 <= effective_width {
                let fill_count = effective_width - prefix_vis - 1;
                format!("{}{fill}╮", header_prefix, fill = "─".repeat(fill_count))
            } else {
                // Header text too wide for terminal, truncate path
                let diff_label = "╭─ diff: ";
                let avail = effective_width.saturating_sub(visible_width(diff_label) + 4);
                let truncated_path = truncate_path(path_display, avail);
                let short_prefix = format!("{diff_label}{truncated_path} ");
                let fill_count = effective_width.saturating_sub(visible_width(&short_prefix) + 1);
                format!("{}{fill}╮", short_prefix, fill = "─".repeat(fill_count))
            };
            output.push_str(&header_line);
            output.push('\n');

            // 2. Data Rows: │ 10: - old_function(); │ 10: + new_function(); │
            for row in &file.rows {
                let left_cell = format_cell(&row.left, left_col_width, lineno_width);
                let right_cell = format_cell(&row.right, right_col_width, lineno_width);

                output.push('│');
                output.push_str(&left_cell);
                output.push('│');
                output.push_str(&right_cell);
                output.push('│');
                output.push('\n');
            }

            // 3. Bottom Border: ╰──────┴──────╯
            let bottom_line = format!(
                "╰{left_fill}┴{right_fill}╯",
                left_fill = "─".repeat(left_col_width),
                right_fill = "─".repeat(right_col_width)
            );
            output.push_str(&bottom_line);
            output.push('\n');
        }

        output
    }

    /// Associated function to parse and render in one step.
    pub fn render_diff_terminal(diff_text: &str, terminal_width: usize) -> String {
        let viewer = Self::parse(diff_text);
        viewer.render(terminal_width)
    }
}

// ============================================================================
// Public Standalone Functions
// ============================================================================

/// Renders a unified diff as side-by-side terminal output with borders and colors.
///
/// ```text
/// ╭─ diff: src/main.rs ─────────────────────────────────────────╮
/// │ 10: - old_function();       │ 10: + new_function();         │
/// ╰─────────────────────────────┴───────────────────────────────╯
/// ```
pub fn render_diff_terminal(diff_text: &str, terminal_width: usize) -> String {
    SideBySideDiffViewer::render_diff_terminal(diff_text, terminal_width)
}

/// Helper function to query the working tree git diff for a repository root.
pub fn git_working_tree_diff(workspace_root: &Path) -> anyhow::Result<String> {
    let output = std::process::Command::new("git")
        .arg("diff")
        .current_dir(workspace_root)
        .output()?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!("git diff failed: {}", stderr.trim());
    }

    Ok(String::from_utf8_lossy(&output.stdout).to_string())
}

// ============================================================================
// Helper Formatting Functions
// ============================================================================

/// Formats an individual cell into exact terminal column width with ANSI colors.
fn format_cell(cell: &DiffCell, col_width: usize, lineno_width: usize) -> String {
    if cell.is_empty() {
        return " ".repeat(col_width);
    }

    // Line number formatted as e.g. " 10: " in dim gray
    let (lineno_colored, lineno_vis_len) = match cell.line_number {
        Some(num) => {
            let plain = format!(" {:>width$}:", num, width = lineno_width);
            let vis_len = plain.len() + 1; // including the space after colon
            let colored = format!("{}{plain}{}", ansi::DIM_GRAY, ansi::RESET);
            (format!("{colored} "), vis_len)
        }
        None => {
            let vis_len = lineno_width + 3;
            (" ".repeat(vis_len), vis_len)
        }
    };

    let sign = match cell.change_type {
        ChangeType::Deletion => "- ",
        ChangeType::Addition => "+ ",
        ChangeType::Context => "  ",
        ChangeType::Empty => "  ",
    };
    let sign_vis_len = 2;

    let available_content = col_width.saturating_sub(lineno_vis_len + sign_vis_len);
    let truncated_content = truncate_to_visible_width(&cell.content, available_content);
    let content_vis_len = visible_width(&truncated_content);

    // Color the sign and content according to change type
    let text_part = format!("{sign}{truncated_content}");
    let colored_content = match cell.change_type {
        ChangeType::Deletion => format!("{}{text_part}{}", ansi::RED, ansi::RESET),
        ChangeType::Addition => format!("{}{text_part}{}", ansi::GREEN, ansi::RESET),
        ChangeType::Context | ChangeType::Empty => text_part,
    };

    let total_vis_len = lineno_vis_len + sign_vis_len + content_vis_len;
    let pad_spaces = col_width.saturating_sub(total_vis_len);

    format!("{lineno_colored}{colored_content}{}", " ".repeat(pad_spaces))
}

/// Parse git header arguments like "a/src/main.rs b/src/main.rs".
fn parse_git_diff_header(rest: &str) -> (Option<String>, Option<String>) {
    let parts: Vec<&str> = rest.split_whitespace().collect();
    if parts.len() >= 2 {
        (Some(clean_path(parts[0])), Some(clean_path(parts[1])))
    } else {
        (None, None)
    }
}

/// Clean a diff path by removing `a/`, `b/`, and outer quotes.
fn clean_path(path: &str) -> String {
    let unquoted = path.trim().trim_matches('"');
    if let Some(stripped) = unquoted.strip_prefix("a/") {
        stripped.to_string()
    } else if let Some(stripped) = unquoted.strip_prefix("b/") {
        stripped.to_string()
    } else {
        unquoted.to_string()
    }
}

/// Parses hunk header `@@ -old_start[,old_len] +new_start[,new_len] @@`.
fn parse_hunk_header(line: &str) -> Option<(usize, usize)> {
    if !line.starts_with("@@ ") {
        return None;
    }
    let after_at = &line[3..];
    let end_idx = after_at.find(" @@")?;
    let ranges = &after_at[..end_idx].trim();

    let mut parts = ranges.split_whitespace();
    let old_part = parts.next()?.strip_prefix('-')?;
    let new_part = parts.next()?.strip_prefix('+')?;

    let old_start = old_part.split(',').next()?.parse::<usize>().ok()?;
    let new_start = new_part.split(',').next()?.parse::<usize>().ok()?;

    Some((old_start, new_start))
}

/// Strip ANSI CSI escape sequences from a string.
pub fn strip_ansi(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();

    while let Some(c) = chars.next() {
        if c == '\x1b' {
            if let Some(&'[') = chars.peek() {
                chars.next();
                for next in chars.by_ref() {
                    if (0x40..=0x7e).contains(&(next as u32)) {
                        break;
                    }
                }
                continue;
            }
        }
        out.push(c);
    }
    out
}

/// Compute visible character width of a string on a terminal monospace grid.
pub fn visible_width(s: &str) -> usize {
    let plain = strip_ansi(s);
    UnicodeWidthStr::width(plain.as_str())
}

/// Truncate text so that visible width does not exceed `max_width`.
fn truncate_to_visible_width(s: &str, max_width: usize) -> String {
    let mut current_width = 0;
    let mut result = String::with_capacity(s.len());

    for c in s.chars() {
        let cw = UnicodeWidthChar::width(c).unwrap_or(0);
        if current_width + cw > max_width {
            break;
        }
        result.push(c);
        current_width += cw;
    }

    result
}

/// Truncate a file path to fit within `max_width`, retaining the end filename.
fn truncate_path(path: &str, max_width: usize) -> String {
    if visible_width(path) <= max_width {
        return path.to_string();
    }
    if max_width < 4 {
        return "…".to_string();
    }
    let target = max_width - 1; // for '…'
    let mut rev_chars: Vec<char> = Vec::new();
    let mut width = 0;

    for c in path.chars().rev() {
        let cw = UnicodeWidthChar::width(c).unwrap_or(0);
        if width + cw > target {
            break;
        }
        rev_chars.push(c);
        width += cw;
    }

    rev_chars.reverse();
    let end_str: String = rev_chars.into_iter().collect();
    format!("…{end_str}")
}
