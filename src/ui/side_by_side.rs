//! Adaptive Side-by-Side and Unified Terminal Diff Viewer
//!
//! Provides a high-fidelity, adaptive diff renderer for terminal environments:
//! - **Adaptive Mode Selection**: Automatically renders side-by-side (2-column) diffs
//!   when terminal width >= 80 columns, and seamlessly falls back to unified linear diffs
//!   on narrow terminals or Android Termux screens.
//! - **Side-by-Side (Split) View**: Clean dual-column layout with aligned row-by-row
//!   additions, deletions, and unchanged context, complete with line numbers and borders.
//! - **Intra-Line Word Diffing**: Word-level and character-level difference highlighting
//!   within modified lines for instant visual recognition of exact edits.
//! - **Unified Diff Fallback**: Compact, color-coded unified diff view optimized for narrow
//!   mobile or embedded terminals (< 80 columns).
//! - **Rich ANSI & Ratatui Integration**: Produces colored ANSI strings for CLI/REPL output
//!   and implements the Ratatui [`Widget`] trait for interactive TUI applications.

use ratatui::{
    buffer::Buffer,
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Paragraph, Widget},
};
use serde::{Deserialize, Serialize};
use similar::{ChangeTag, TextDiff};
use std::borrow::Cow;

use crate::ui::diff_view::{
    highlight_tokens_to_spans, tokenize_line, DiffFile, DiffLine, DiffLineType, SyntaxLanguage,
};
use crate::ui::table::{get_terminal_width, visible_width};
use crate::ui::termux::is_termux;
use crate::ui::theme::Theme;

// ============================================================================
// Constants
// ============================================================================

/// Default minimum terminal width (in columns) required for side-by-side display.
pub const DEFAULT_SIDE_BY_SIDE_MIN_WIDTH: usize = 80;

/// Default tab width in spaces when expanding tab characters in diffs.
pub const DEFAULT_TAB_WIDTH: usize = 4;

/// Default context lines surrounding diff hunks.
pub const DEFAULT_CONTEXT_RADIUS: usize = 3;

/// Narrow terminal threshold for mobile / Termux environments.
pub const NARROW_SCREEN_THRESHOLD: usize = 80;

// ============================================================================
// Enums & Configuration Types
// ============================================================================

/// Presentation mode for the diff viewer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
pub enum DiffDisplayMode {
    /// Automatically select Side-by-Side if terminal width >= 80 and not a narrow Termux screen,
    /// otherwise fall back to Unified diff.
    #[default]
    Auto,
    /// Force side-by-side (2-column) split diff.
    SideBySide,
    /// Force linear unified diff.
    Unified,
}

impl DiffDisplayMode {
    pub fn is_auto(&self) -> bool {
        matches!(self, Self::Auto)
    }

    pub fn is_side_by_side(&self) -> bool {
        matches!(self, Self::SideBySide)
    }

    pub fn is_unified(&self) -> bool {
        matches!(self, Self::Unified)
    }

    pub fn name(&self) -> &'static str {
        match self {
            Self::Auto => "Auto",
            Self::SideBySide => "Side-by-Side",
            Self::Unified => "Unified",
        }
    }
}

/// Border style for side-by-side and unified diff frames.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
pub enum DiffBorderStyle {
    /// Smooth rounded unicode box-drawing characters (┌, ─, ┐, │, └, ┘, etc.).
    #[default]
    Rounded,
    /// Sharp square unicode box-drawing characters (┌, ─, ┐, │, └, ┘).
    Sharp,
    /// Double-line unicode box-drawing characters (╔, ═, ╗, ║, ╚, ╝).
    Double,
    /// Plain ASCII characters (+, -, |).
    Ascii,
    /// Minimal border with only vertical column dividers.
    Minimal,
    /// No borders or surrounding frame.
    None,
}

/// Change classification for a single line or cell.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
pub enum DiffChangeKind {
    /// Context line (unmodified in both versions).
    #[default]
    Unchanged,
    /// Added line (present in modified version only).
    Added,
    /// Deleted line (present in original version only).
    Deleted,
    /// Modified line (both versions have corresponding changes).
    Modified,
    /// Empty padding cell (used for visual vertical alignment).
    Empty,
}

impl DiffChangeKind {
    pub fn is_empty(&self) -> bool {
        matches!(self, Self::Empty)
    }

    pub fn is_added(&self) -> bool {
        matches!(self, Self::Added)
    }

    pub fn is_deleted(&self) -> bool {
        matches!(self, Self::Deleted)
    }

    pub fn is_modified(&self) -> bool {
        matches!(self, Self::Modified)
    }

    pub fn is_unchanged(&self) -> bool {
        matches!(self, Self::Unchanged)
    }

    pub fn symbol(&self) -> &'static str {
        match self {
            Self::Added => "+",
            Self::Deleted => "-",
            Self::Modified => "~",
            Self::Unchanged => " ",
            Self::Empty => " ",
        }
    }
}

/// Statistics and line counts for a diff.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct DiffStats {
    /// Number of added lines.
    pub additions: usize,
    /// Number of deleted lines.
    pub deletions: usize,
    /// Number of modified/paired lines.
    pub modifications: usize,
    /// Number of unchanged context lines.
    pub unchanged: usize,
    /// Total number of hunks.
    pub hunks_count: usize,
}

impl DiffStats {
    /// Total net changes (additions + deletions).
    pub fn total_changes(&self) -> usize {
        self.additions + self.deletions
    }

    /// Formats a compact summary string (e.g. "+5 -3 lines in 2 hunks").
    pub fn summary(&self) -> String {
        format!(
            "+{} -{} lines across {} {}",
            self.additions,
            self.deletions,
            self.hunks_count,
            if self.hunks_count == 1 {
                "hunk"
            } else {
                "hunks"
            }
        )
    }
}

/// Configuration options for adaptive diff rendering.
#[derive(Debug, Clone)]
pub struct AdaptiveDiffConfig {
    /// Display mode preference.
    pub mode: DiffDisplayMode,
    /// Minimum width required for side-by-side display (default: 80).
    pub min_side_by_side_width: usize,
    /// Explicit terminal width override (if None, detected automatically).
    pub terminal_width: Option<usize>,
    /// Explicit Termux environment override (if None, detected automatically).
    pub force_termux: Option<bool>,
    /// Whether ANSI colors are enabled.
    pub color: bool,
    /// Whether syntax highlighting is enabled.
    pub syntax_highlighting: bool,
    /// Whether line numbers are rendered in the columns.
    pub show_line_numbers: bool,
    /// Whether file and hunk headers are rendered.
    pub show_headers: bool,
    /// Whether the footer statistics bar is rendered.
    pub show_summary: bool,
    /// Number of context lines surrounding changed hunks.
    pub context_radius: usize,
    /// Tab expansion width.
    pub tab_width: usize,
    /// Header label for the left (old) column.
    pub left_header: Option<String>,
    /// Header label for the right (new) column.
    pub right_header: Option<String>,
    /// Border style for the diff box.
    pub border_style: DiffBorderStyle,
    /// Whether intra-line word/character difference highlighting is enabled.
    pub word_diff: bool,
    /// Visual color theme.
    pub theme: Theme,
}

impl Default for AdaptiveDiffConfig {
    fn default() -> Self {
        Self {
            mode: DiffDisplayMode::Auto,
            min_side_by_side_width: DEFAULT_SIDE_BY_SIDE_MIN_WIDTH,
            terminal_width: None,
            force_termux: None,
            color: true,
            syntax_highlighting: true,
            show_line_numbers: true,
            show_headers: true,
            show_summary: true,
            context_radius: DEFAULT_CONTEXT_RADIUS,
            tab_width: DEFAULT_TAB_WIDTH,
            left_header: None,
            right_header: None,
            border_style: DiffBorderStyle::Rounded,
            word_diff: true,
            theme: Theme::default(),
        }
    }
}

impl AdaptiveDiffConfig {
    /// Creates a new configuration with default settings.
    pub fn new() -> Self {
        Self::default()
    }

    /// Sets the display mode.
    pub fn with_mode(mut self, mode: DiffDisplayMode) -> Self {
        self.mode = mode;
        self
    }

    /// Sets the minimum side-by-side width threshold.
    pub fn with_min_width(mut self, min_width: usize) -> Self {
        self.min_side_by_side_width = min_width;
        self
    }

    /// Overrides the terminal width.
    pub fn with_terminal_width(mut self, width: usize) -> Self {
        self.terminal_width = Some(width);
        self
    }

    /// Overrides the Termux environment detection.
    pub fn with_force_termux(mut self, is_termux: bool) -> Self {
        self.force_termux = Some(is_termux);
        self
    }

    /// Enables or disables ANSI colors.
    pub fn with_color(mut self, color: bool) -> Self {
        self.color = color;
        self
    }

    /// Enables or disables syntax highlighting.
    pub fn with_syntax_highlighting(mut self, syntax_highlighting: bool) -> Self {
        self.syntax_highlighting = syntax_highlighting;
        self
    }

    /// Sets the context line radius.
    pub fn with_context_radius(mut self, radius: usize) -> Self {
        self.context_radius = radius;
        self
    }

    /// Sets the visual theme.
    pub fn with_theme(mut self, theme: Theme) -> Self {
        self.theme = theme;
        self
    }

    /// Sets the border style.
    pub fn with_border_style(mut self, style: DiffBorderStyle) -> Self {
        self.border_style = style;
        self
    }

    /// Sets whether intra-line word diffs are calculated.
    pub fn with_word_diff(mut self, word_diff: bool) -> Self {
        self.word_diff = word_diff;
        self
    }
}

// ============================================================================
// Core Diff Data Model
// ============================================================================

/// A character range representing an intra-line difference.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct HighlightRange {
    /// Start byte index in UTF-8 string.
    pub start: usize,
    /// End byte index in UTF-8 string.
    pub end: usize,
}

/// A single cell on one side of a side-by-side row.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SideBySideCell {
    /// Line number (if not an empty spacer cell).
    pub line_number: Option<usize>,
    /// Content string for this line.
    pub content: String,
    /// Change classification for this cell.
    pub kind: DiffChangeKind,
    /// Character ranges within `content` that were specifically changed (word diff).
    pub highlights: Vec<HighlightRange>,
    /// True if this is an empty padding cell.
    pub is_empty: bool,
}

impl SideBySideCell {
    /// Creates a populated cell.
    pub fn new(
        line_number: Option<usize>,
        content: impl Into<String>,
        kind: DiffChangeKind,
    ) -> Self {
        Self {
            line_number,
            content: content.into(),
            kind,
            highlights: Vec::new(),
            is_empty: false,
        }
    }

    /// Creates an empty padding cell.
    pub fn empty() -> Self {
        Self {
            line_number: None,
            content: String::new(),
            kind: DiffChangeKind::Empty,
            highlights: Vec::new(),
            is_empty: true,
        }
    }

    /// Adds an intra-line highlight range.
    pub fn with_highlight(mut self, start: usize, end: usize) -> Self {
        if start < end && end <= self.content.len() {
            self.highlights.push(HighlightRange { start, end });
        }
        self
    }
}

/// A paired row representing a synchronized line in a side-by-side diff.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SideBySideRow {
    /// Left cell (Original / Deleted / Unchanged / Empty).
    pub left: SideBySideCell,
    /// Right cell (Modified / Added / Unchanged / Empty).
    pub right: SideBySideCell,
    /// Overall classification of this paired row.
    pub kind: DiffChangeKind,
}

impl SideBySideRow {
    /// Creates an unchanged context row.
    pub fn context(old_no: usize, new_no: usize, content: impl Into<String>) -> Self {
        let text = content.into();
        Self {
            left: SideBySideCell::new(Some(old_no), text.clone(), DiffChangeKind::Unchanged),
            right: SideBySideCell::new(Some(new_no), text, DiffChangeKind::Unchanged),
            kind: DiffChangeKind::Unchanged,
        }
    }

    /// Creates a deletion row (left has content, right is empty).
    pub fn deletion(old_no: usize, content: impl Into<String>) -> Self {
        Self {
            left: SideBySideCell::new(Some(old_no), content, DiffChangeKind::Deleted),
            right: SideBySideCell::empty(),
            kind: DiffChangeKind::Deleted,
        }
    }

    /// Creates an addition row (left is empty, right has content).
    pub fn addition(new_no: usize, content: impl Into<String>) -> Self {
        Self {
            left: SideBySideCell::empty(),
            right: SideBySideCell::new(Some(new_no), content, DiffChangeKind::Added),
            kind: DiffChangeKind::Added,
        }
    }

    /// Creates a modified row pairing old and new contents with optional word highlights.
    pub fn modified(
        old_no: usize,
        old_content: impl Into<String>,
        new_no: usize,
        new_content: impl Into<String>,
    ) -> Self {
        Self {
            left: SideBySideCell::new(Some(old_no), old_content, DiffChangeKind::Modified),
            right: SideBySideCell::new(Some(new_no), new_content, DiffChangeKind::Modified),
            kind: DiffChangeKind::Modified,
        }
    }
}

/// A single diff hunk containing synchronized side-by-side rows.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SideBySideHunk {
    pub old_start: usize,
    pub old_lines: usize,
    pub new_start: usize,
    pub new_lines: usize,
    pub header: String,
    pub rows: Vec<SideBySideRow>,
}

impl SideBySideHunk {
    /// Creates a new hunk.
    pub fn new(
        old_start: usize,
        old_lines: usize,
        new_start: usize,
        new_lines: usize,
        header: impl Into<String>,
        rows: Vec<SideBySideRow>,
    ) -> Self {
        Self {
            old_start,
            old_lines,
            new_start,
            new_lines,
            header: header.into(),
            rows,
        }
    }

    /// Formats the unified hunk header (e.g. `@@ -1,5 +1,6 @@`).
    pub fn unified_header(&self) -> String {
        if self.header.is_empty() {
            format!(
                "@@ -{},{} +{},{} @@",
                self.old_start, self.old_lines, self.new_start, self.new_lines
            )
        } else {
            format!(
                "@@ -{},{} +{},{} @@ {}",
                self.old_start, self.old_lines, self.new_start, self.new_lines, self.header
            )
        }
    }
}

/// A complete diff document representing changes across a file or buffer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SideBySideDocument {
    /// Primary file path (or description).
    pub file_path: Option<String>,
    /// Old version file path.
    pub old_path: Option<String>,
    /// New version file path.
    pub new_path: Option<String>,
    /// Parsed hunks.
    pub hunks: Vec<SideBySideHunk>,
    /// Diff metrics and line counts.
    pub stats: DiffStats,
    /// Indicates whether the content was detected as binary.
    pub is_binary: bool,
}

impl SideBySideDocument {
    /// Creates a new empty document.
    pub fn new() -> Self {
        Self {
            file_path: None,
            old_path: None,
            new_path: None,
            hunks: Vec::new(),
            stats: DiffStats::default(),
            is_binary: false,
        }
    }

    /// Builds a `SideBySideDocument` from two raw strings (old vs new).
    pub fn from_texts(
        old_content: &str,
        new_content: &str,
        file_path: Option<&str>,
        context_radius: usize,
        compute_word_diff: bool,
    ) -> Self {
        let diff = TextDiff::from_lines(old_content, new_content);
        let mut hunks = Vec::new();
        let mut stats = DiffStats::default();

        let path_str = file_path.map(|s| s.to_string());

        for group in diff.grouped_ops(context_radius) {
            if group.is_empty() {
                continue;
            }

            let mut old_start = 0;
            let mut old_count = 0;
            let mut new_start = 0;
            let mut new_count = 0;
            let mut is_first = true;

            // Collect operations for this hunk
            for op in &group {
                if is_first {
                    old_start = op.old_range().start + 1;
                    new_start = op.new_range().start + 1;
                    is_first = false;
                }
                old_count += op.old_range().len();
                new_count += op.new_range().len();
            }

            // Build synchronized rows for this hunk
            let mut rows = Vec::new();

            for op in &group {
                match op {
                    similar::DiffOp::Equal { .. } => {
                        for change in diff.iter_changes(op) {
                            let old_no = change.old_index().unwrap_or(0) + 1;
                            let new_no = change.new_index().unwrap_or(0) + 1;
                            let text = change.value().trim_end_matches(['\r', '\n']);
                            rows.push(SideBySideRow::context(old_no, new_no, text));
                            stats.unchanged += 1;
                        }
                    }
                    similar::DiffOp::Delete { .. } => {
                        for change in diff.iter_changes(op) {
                            let old_no = change.old_index().unwrap_or(0) + 1;
                            let text = change.value().trim_end_matches(['\r', '\n']);
                            rows.push(SideBySideRow::deletion(old_no, text));
                            stats.deletions += 1;
                        }
                    }
                    similar::DiffOp::Insert { .. } => {
                        for change in diff.iter_changes(op) {
                            let new_no = change.new_index().unwrap_or(0) + 1;
                            let text = change.value().trim_end_matches(['\r', '\n']);
                            rows.push(SideBySideRow::addition(new_no, text));
                            stats.additions += 1;
                        }
                    }
                    similar::DiffOp::Replace { .. } => {
                        let deletes: Vec<_> = diff
                            .iter_changes(op)
                            .filter(|c| c.tag() == ChangeTag::Delete)
                            .collect();
                        let inserts: Vec<_> = diff
                            .iter_changes(op)
                            .filter(|c| c.tag() == ChangeTag::Insert)
                            .collect();

                        let max_len = deletes.len().max(inserts.len());

                        for i in 0..max_len {
                            let del_opt = deletes.get(i);
                            let ins_opt = inserts.get(i);

                            match (del_opt, ins_opt) {
                                (Some(del), Some(ins)) => {
                                    let old_no = del.old_index().unwrap_or(0) + 1;
                                    let new_no = ins.new_index().unwrap_or(0) + 1;
                                    let del_text = del.value().trim_end_matches(['\r', '\n']);
                                    let ins_text = ins.value().trim_end_matches(['\r', '\n']);

                                    let mut row =
                                        SideBySideRow::modified(old_no, del_text, new_no, ins_text);

                                    if compute_word_diff {
                                        let (del_ranges, ins_ranges) =
                                            compute_intra_line_highlights(del_text, ins_text);
                                        row.left.highlights = del_ranges;
                                        row.right.highlights = ins_ranges;
                                    }

                                    rows.push(row);
                                    stats.modifications += 1;
                                    stats.deletions += 1;
                                    stats.additions += 1;
                                }
                                (Some(del), None) => {
                                    let old_no = del.old_index().unwrap_or(0) + 1;
                                    let del_text = del.value().trim_end_matches(['\r', '\n']);
                                    rows.push(SideBySideRow::deletion(old_no, del_text));
                                    stats.deletions += 1;
                                }
                                (None, Some(ins)) => {
                                    let new_no = ins.new_index().unwrap_or(0) + 1;
                                    let ins_text = ins.value().trim_end_matches(['\r', '\n']);
                                    rows.push(SideBySideRow::addition(new_no, ins_text));
                                    stats.additions += 1;
                                }
                                (None, None) => {}
                            }
                        }
                    }
                }
            }

            let hunk = SideBySideHunk::new(old_start, old_count, new_start, new_count, "", rows);
            hunks.push(hunk);
        }

        stats.hunks_count = hunks.len();

        Self {
            file_path: path_str.clone(),
            old_path: path_str.clone().map(|p| format!("a/{p}")),
            new_path: path_str.map(|p| format!("b/{p}")),
            hunks,
            stats,
            is_binary: false,
        }
    }

    /// Converts a [`DiffFile`] from `diff_view.rs` into a `SideBySideDocument`.
    pub fn from_diff_file(file: &DiffFile, compute_word_diff: bool) -> Self {
        let mut hunks = Vec::new();
        let mut stats = DiffStats::default();

        for hunk in &file.hunks {
            let mut rows = Vec::new();
            let mut pending_deletes = Vec::new();
            let mut pending_inserts = Vec::new();

            let flush_pending = |deletes: &mut Vec<&DiffLine>,
                                 inserts: &mut Vec<&DiffLine>,
                                 rows: &mut Vec<SideBySideRow>,
                                 stats: &mut DiffStats| {
                let max_len = deletes.len().max(inserts.len());
                for i in 0..max_len {
                    match (deletes.get(i), inserts.get(i)) {
                        (Some(del), Some(ins)) => {
                            let old_no = del.old_lineno.unwrap_or(0);
                            let new_no = ins.new_lineno.unwrap_or(0);
                            let mut row =
                                SideBySideRow::modified(old_no, &del.content, new_no, &ins.content);
                            if compute_word_diff {
                                let (del_ranges, ins_ranges) =
                                    compute_intra_line_highlights(&del.content, &ins.content);
                                row.left.highlights = del_ranges;
                                row.right.highlights = ins_ranges;
                            }
                            rows.push(row);
                            stats.modifications += 1;
                            stats.deletions += 1;
                            stats.additions += 1;
                        }
                        (Some(del), None) => {
                            let old_no = del.old_lineno.unwrap_or(0);
                            rows.push(SideBySideRow::deletion(old_no, &del.content));
                            stats.deletions += 1;
                        }
                        (None, Some(ins)) => {
                            let new_no = ins.new_lineno.unwrap_or(0);
                            rows.push(SideBySideRow::addition(new_no, &ins.content));
                            stats.additions += 1;
                        }
                        (None, None) => {}
                    }
                }
                deletes.clear();
                inserts.clear();
            };

            for line in &hunk.lines {
                match line.line_type {
                    DiffLineType::Context => {
                        flush_pending(
                            &mut pending_deletes,
                            &mut pending_inserts,
                            &mut rows,
                            &mut stats,
                        );
                        let old_no = line.old_lineno.unwrap_or(0);
                        let new_no = line.new_lineno.unwrap_or(0);
                        rows.push(SideBySideRow::context(old_no, new_no, &line.content));
                        stats.unchanged += 1;
                    }
                    DiffLineType::Deletion => {
                        pending_deletes.push(line);
                    }
                    DiffLineType::Addition => {
                        pending_inserts.push(line);
                    }
                    _ => {}
                }
            }

            flush_pending(
                &mut pending_deletes,
                &mut pending_inserts,
                &mut rows,
                &mut stats,
            );

            let sbs_hunk = SideBySideHunk::new(
                hunk.old_start,
                hunk.old_lines,
                hunk.new_start,
                hunk.new_lines,
                &hunk.header,
                rows,
            );
            hunks.push(sbs_hunk);
        }

        stats.hunks_count = hunks.len();

        Self {
            file_path: Some(file.path.clone()),
            old_path: file.old_path.clone(),
            new_path: file.new_path.clone(),
            hunks,
            stats,
            is_binary: file.is_binary,
        }
    }
}

impl Default for SideBySideDocument {
    fn default() -> Self {
        Self::new()
    }
}

// ============================================================================
// Intra-Line Word Diff Algorithm
// ============================================================================

/// Computes byte ranges of differing words / characters between two modified lines.
pub fn compute_intra_line_highlights(
    old_line: &str,
    new_line: &str,
) -> (Vec<HighlightRange>, Vec<HighlightRange>) {
    if old_line == new_line {
        return (Vec::new(), Vec::new());
    }

    let word_diff = TextDiff::from_words(old_line, new_line);
    let mut old_ranges = Vec::new();
    let mut new_ranges = Vec::new();

    let mut old_offset = 0;
    let mut new_offset = 0;

    for change in word_diff.iter_all_changes() {
        let val = change.value();
        let len = val.len();

        match change.tag() {
            ChangeTag::Equal => {
                old_offset += len;
                new_offset += len;
            }
            ChangeTag::Delete => {
                let start = old_offset;
                let end = old_offset + len;
                old_offset += len;
                if start < end {
                    old_ranges.push(HighlightRange { start, end });
                }
            }
            ChangeTag::Insert => {
                let start = new_offset;
                let end = new_offset + len;
                new_offset += len;
                if start < end {
                    new_ranges.push(HighlightRange { start, end });
                }
            }
        }
    }

    (
        merge_adjacent_ranges(old_ranges),
        merge_adjacent_ranges(new_ranges),
    )
}

/// Merges contiguous or overlapping highlight ranges for cleaner rendering.
fn merge_adjacent_ranges(ranges: Vec<HighlightRange>) -> Vec<HighlightRange> {
    if ranges.is_empty() {
        return Vec::new();
    }

    let mut merged = Vec::with_capacity(ranges.len());
    let mut current = ranges[0];

    for r in ranges.into_iter().skip(1) {
        if r.start <= current.end {
            current.end = current.end.max(r.end);
        } else {
            merged.push(current);
            current = r;
        }
    }
    merged.push(current);
    merged
}

// ============================================================================
// Adaptive Mode Resolution
// ============================================================================

/// Returns true if the environment supports side-by-side terminal rendering.
///
/// Side-by-side mode requires:
/// 1. Terminal width >= `min_width` (typically 80 columns).
/// 2. If running inside Termux, screen width must meet the minimum threshold (mobile terminals
///    with narrow widths will automatically fall back to unified mode).
pub fn is_side_by_side_supported(width: usize, is_termux_env: bool, min_width: usize) -> bool {
    if is_termux_env && width < NARROW_SCREEN_THRESHOLD {
        return false;
    }
    width >= min_width
}

/// Resolves the effective terminal width based on explicit config or runtime environment.
pub fn get_effective_width(config: &AdaptiveDiffConfig) -> usize {
    if let Some(w) = config.terminal_width {
        w
    } else {
        get_terminal_width()
    }
}

/// Resolves the effective Termux status based on explicit config or runtime environment.
pub fn get_effective_termux(config: &AdaptiveDiffConfig) -> bool {
    config.force_termux.unwrap_or_else(is_termux)
}

/// Resolves the effective display mode (Side-by-Side vs Unified) for the given configuration.
pub fn resolve_display_mode(config: &AdaptiveDiffConfig) -> DiffDisplayMode {
    match config.mode {
        DiffDisplayMode::SideBySide => DiffDisplayMode::SideBySide,
        DiffDisplayMode::Unified => DiffDisplayMode::Unified,
        DiffDisplayMode::Auto => {
            let width = get_effective_width(config);
            let termux = get_effective_termux(config);
            if is_side_by_side_supported(width, termux, config.min_side_by_side_width) {
                DiffDisplayMode::SideBySide
            } else {
                DiffDisplayMode::Unified
            }
        }
    }
}

// ============================================================================
// ANSI String Rendering Engine
// ============================================================================

/// ANSI terminal color escape codes.
mod ansi {
    pub const RESET: &str = "\x1b[0m";
    pub const BOLD: &str = "\x1b[1m";
    pub const DIM: &str = "\x1b[2m";
    pub const INVERT: &str = "\x1b[7m";

    // Standard foregrounds
    pub const RED: &str = "\x1b[31m";
    pub const GREEN: &str = "\x1b[32m";
    pub const YELLOW: &str = "\x1b[33m";
    pub const BLUE: &str = "\x1b[34m";
    pub const MAGENTA: &str = "\x1b[35m";
    pub const CYAN: &str = "\x1b[36m";
    pub const WHITE: &str = "\x1b[37m";

    // Bright foregrounds
    pub const BRIGHT_RED: &str = "\x1b[91m";
    pub const BRIGHT_GREEN: &str = "\x1b[92m";
    pub const BRIGHT_CYAN: &str = "\x1b[96m";
    pub const BRIGHT_WHITE: &str = "\x1b[97m";

    // Backgrounds for word diff highlights
    pub const BG_RED: &str = "\x1b[41m";
    pub const BG_GREEN: &str = "\x1b[42m";
    pub const BG_DARK_RED: &str = "\x1b[48;5;52m";
    pub const BG_DARK_GREEN: &str = "\x1b[48;5;22m";
}

/// Renders an adaptive diff string from raw old/new text.
///
/// Automatically switches between Side-by-Side and Unified views depending on the terminal width
/// (>= 80 columns) and Termux mobile environment constraints.
pub fn render_adaptive(
    old_content: &str,
    new_content: &str,
    file_path: Option<&str>,
    config: Option<&AdaptiveDiffConfig>,
) -> String {
    let default_cfg = AdaptiveDiffConfig::default();
    let cfg = config.unwrap_or(&default_cfg);

    let doc = SideBySideDocument::from_texts(
        old_content,
        new_content,
        file_path,
        cfg.context_radius,
        cfg.word_diff,
    );

    let mode = resolve_display_mode(cfg);
    let width = get_effective_width(cfg);

    match mode {
        DiffDisplayMode::SideBySide => render_side_by_side_ansi(&doc, width, cfg),
        DiffDisplayMode::Unified | DiffDisplayMode::Auto => render_unified_ansi(&doc, width, cfg),
    }
}

/// Renders a side-by-side diff document to a formatted ANSI terminal string.
pub fn render_side_by_side_ansi(
    doc: &SideBySideDocument,
    width: usize,
    config: &AdaptiveDiffConfig,
) -> String {
    if doc.hunks.is_empty() {
        return if config.color {
            format!("{}No changes detected.{}", ansi::DIM, ansi::RESET)
        } else {
            "No changes detected.".to_string()
        };
    }

    let mut out = String::new();
    let border = BorderChars::from_style(config.border_style);
    let color = config.color;

    // Determine line number column width (minimum 3 digits + 1 space)
    let max_old_line = doc
        .hunks
        .iter()
        .flat_map(|h| h.rows.iter())
        .filter_map(|r| r.left.line_number)
        .max()
        .unwrap_or(0);
    let max_new_line = doc
        .hunks
        .iter()
        .flat_map(|h| h.rows.iter())
        .filter_map(|r| r.right.line_number)
        .max()
        .unwrap_or(0);
    let lineno_digits = max_old_line.max(max_new_line).to_string().len().max(3);

    // Compute column widths
    // Format per column: [Border] [LineNo (lineno_digits)] [Sign (1)] [Space] [Content]
    let lineno_prefix_len = if config.show_line_numbers {
        lineno_digits + 2 // digits + sign + space
    } else {
        2 // sign + space
    };

    let total_border_overhead = if config.border_style == DiffBorderStyle::None {
        1 // just center divider
    } else {
        3 // left border, center divider, right border
    };

    let available_content = width.saturating_sub(total_border_overhead);
    let left_col_width = available_content / 2;
    let right_col_width = available_content.saturating_sub(left_col_width);

    let left_text_width = left_col_width.saturating_sub(lineno_prefix_len);
    let right_text_width = right_col_width.saturating_sub(lineno_prefix_len);

    // 1. Render Top Header
    if config.show_headers {
        let left_title = config
            .left_header
            .as_deref()
            .or(doc.old_path.as_deref())
            .unwrap_or("Original");
        let right_title = config
            .right_header
            .as_deref()
            .or(doc.new_path.as_deref())
            .unwrap_or("Modified");

        if config.border_style != DiffBorderStyle::None {
            // Top border: ┌─── title ───┬─── title ───┐
            out.push_str(&render_top_header(
                left_title,
                right_title,
                left_col_width,
                right_col_width,
                &border,
                color,
            ));
            out.push('\n');
        } else {
            // Minimal header line
            out.push_str(&format!(
                "--- {} │ +++ {}\n",
                truncate_or_pad(left_title, left_col_width),
                truncate_or_pad(right_title, right_col_width)
            ));
        }
    }

    // 2. Render Hunks & Synchronized Rows
    for (h_idx, hunk) in doc.hunks.iter().enumerate() {
        if config.show_headers {
            let hunk_hdr = hunk.unified_header();
            if config.border_style != DiffBorderStyle::None {
                out.push_str(&render_hunk_separator(
                    &hunk_hdr,
                    left_col_width,
                    right_col_width,
                    &border,
                    color,
                ));
            } else {
                if color {
                    out.push_str(&format!(
                        "{}{}{}{}\n",
                        ansi::CYAN,
                        ansi::BOLD,
                        hunk_hdr,
                        ansi::RESET
                    ));
                } else {
                    out.push_str(&format!("{}\n", hunk_hdr));
                }
            }
            out.push('\n');
        }

        for row in &hunk.rows {
            let left_cell_str = render_side_cell(
                &row.left,
                left_text_width,
                lineno_digits,
                config.show_line_numbers,
                color,
                true,
                config.tab_width,
            );
            let right_cell_str = render_side_cell(
                &row.right,
                right_text_width,
                lineno_digits,
                config.show_line_numbers,
                color,
                false,
                config.tab_width,
            );

            // Row format: [Left Border] [Left Cell] [Center Border] [Right Cell] [Right Border]
            if config.border_style != DiffBorderStyle::None {
                if color {
                    out.push_str(&format!(
                        "{}{}{}{}{}{}{}{}\n",
                        ansi::DIM,
                        border.v,
                        ansi::RESET,
                        left_cell_str,
                        ansi::DIM,
                        border.v,
                        ansi::RESET,
                        right_cell_str
                    ));
                } else {
                    out.push_str(&format!(
                        "{}{}{}{}{}\n",
                        border.v, left_cell_str, border.v, right_cell_str, border.v
                    ));
                }
            } else {
                out.push_str(&format!("{}│{}\n", left_cell_str, right_cell_str));
            }
        }

        if h_idx + 1 < doc.hunks.len() && config.border_style != DiffBorderStyle::None {
            // Intermediate hunk separator
            out.push_str(&render_mid_separator(
                left_col_width,
                right_col_width,
                &border,
                color,
            ));
            out.push('\n');
        }
    }

    // 3. Render Bottom Summary Frame
    if config.show_summary {
        let summary_text = doc.stats.summary();
        if config.border_style != DiffBorderStyle::None {
            out.push_str(&render_bottom_summary(
                &summary_text,
                left_col_width,
                right_col_width,
                &border,
                color,
            ));
        } else {
            if color {
                out.push_str(&format!(
                    "{}{}{}{}\n",
                    ansi::DIM,
                    ansi::BOLD,
                    summary_text,
                    ansi::RESET
                ));
            } else {
                out.push_str(&format!("{}\n", summary_text));
            }
        }
    }

    out
}

/// Renders a unified linear diff document to a formatted ANSI terminal string.
pub fn render_unified_ansi(
    doc: &SideBySideDocument,
    _width: usize,
    config: &AdaptiveDiffConfig,
) -> String {
    if doc.hunks.is_empty() {
        return if config.color {
            format!("{}No changes detected.{}", ansi::DIM, ansi::RESET)
        } else {
            "No changes detected.".to_string()
        };
    }

    let mut out = String::new();
    let color = config.color;

    // File Headers
    if config.show_headers {
        let old_p = doc.old_path.as_deref().unwrap_or("--- a/original");
        let new_p = doc.new_path.as_deref().unwrap_or("+++ b/modified");

        if color {
            out.push_str(&format!(
                "{}{}{}{}\n{}{}{}{}\n",
                ansi::RED,
                ansi::BOLD,
                old_p,
                ansi::RESET,
                ansi::GREEN,
                ansi::BOLD,
                new_p,
                ansi::RESET
            ));
        } else {
            out.push_str(&format!("{}\n{}\n", old_p, new_p));
        }
    }

    // Determine line number column width
    let max_old_line = doc
        .hunks
        .iter()
        .flat_map(|h| h.rows.iter())
        .filter_map(|r| r.left.line_number)
        .max()
        .unwrap_or(0);
    let max_new_line = doc
        .hunks
        .iter()
        .flat_map(|h| h.rows.iter())
        .filter_map(|r| r.right.line_number)
        .max()
        .unwrap_or(0);
    let lineno_digits = max_old_line.max(max_new_line).to_string().len().max(3);

    for (h_idx, hunk) in doc.hunks.iter().enumerate() {
        if h_idx > 0 {
            out.push('\n');
        }

        let hdr = hunk.unified_header();
        if color {
            out.push_str(&format!(
                "{}{}{}{}\n",
                ansi::CYAN,
                ansi::BOLD,
                hdr,
                ansi::RESET
            ));
        } else {
            out.push_str(&format!("{}\n", hdr));
        }

        for row in &hunk.rows {
            match row.kind {
                DiffChangeKind::Unchanged => {
                    let old_no = row.left.line_number.unwrap_or(0);
                    let new_no = row.right.line_number.unwrap_or(0);
                    let content = expand_tabs(&row.left.content, config.tab_width);

                    if config.show_line_numbers {
                        if color {
                            out.push_str(&format!(
                                " {}{:>w$}{} {}{:>w$}{} │ {}\n",
                                ansi::CYAN,
                                old_no,
                                ansi::RESET,
                                ansi::CYAN,
                                new_no,
                                ansi::RESET,
                                content,
                                w = lineno_digits
                            ));
                        } else {
                            out.push_str(&format!(
                                " {:>w$} {:>w$} │ {}\n",
                                old_no,
                                new_no,
                                content,
                                w = lineno_digits
                            ));
                        }
                    } else {
                        out.push_str(&format!("  {}\n", content));
                    }
                }
                DiffChangeKind::Deleted => {
                    let old_no = row.left.line_number.unwrap_or(0);
                    let content = expand_tabs(&row.left.content, config.tab_width);

                    if config.show_line_numbers {
                        if color {
                            out.push_str(&format!(
                                "{}-{} {}{:>w$}{} {:>w$} │ {}{}{}\n",
                                ansi::RED,
                                ansi::RESET,
                                ansi::CYAN,
                                old_no,
                                ansi::RESET,
                                "",
                                ansi::RED,
                                content,
                                ansi::RESET,
                                w = lineno_digits
                            ));
                        } else {
                            out.push_str(&format!(
                                "- {:>w$} {:>w$} │ {}\n",
                                old_no,
                                "",
                                content,
                                w = lineno_digits
                            ));
                        }
                    } else {
                        if color {
                            out.push_str(&format!("{}- {}{}\n", ansi::RED, content, ansi::RESET));
                        } else {
                            out.push_str(&format!("- {}\n", content));
                        }
                    }
                }
                DiffChangeKind::Added => {
                    let new_no = row.right.line_number.unwrap_or(0);
                    let content = expand_tabs(&row.right.content, config.tab_width);

                    if config.show_line_numbers {
                        if color {
                            out.push_str(&format!(
                                "{}+{} {:>w$} {}{:>w$}{} │ {}{}{}\n",
                                ansi::GREEN,
                                ansi::RESET,
                                "",
                                ansi::CYAN,
                                new_no,
                                ansi::RESET,
                                ansi::GREEN,
                                content,
                                ansi::RESET,
                                w = lineno_digits
                            ));
                        } else {
                            out.push_str(&format!(
                                "+ {:>w$} {:>w$} │ {}\n",
                                "",
                                new_no,
                                content,
                                w = lineno_digits
                            ));
                        }
                    } else {
                        if color {
                            out.push_str(&format!("{}+ {}{}\n", ansi::GREEN, content, ansi::RESET));
                        } else {
                            out.push_str(&format!("+ {}\n", content));
                        }
                    }
                }
                DiffChangeKind::Modified => {
                    // Render deletion line then addition line
                    let old_no = row.left.line_number.unwrap_or(0);
                    let new_no = row.right.line_number.unwrap_or(0);
                    let del_content = expand_tabs(&row.left.content, config.tab_width);
                    let ins_content = expand_tabs(&row.right.content, config.tab_width);

                    // Deletion line with intra-line highlights
                    let del_formatted = format_highlighted_line(
                        &del_content,
                        &row.left.highlights,
                        ansi::RED,
                        ansi::BG_DARK_RED,
                        color,
                    );
                    let ins_formatted = format_highlighted_line(
                        &ins_content,
                        &row.right.highlights,
                        ansi::GREEN,
                        ansi::BG_DARK_GREEN,
                        color,
                    );

                    if config.show_line_numbers {
                        if color {
                            out.push_str(&format!(
                                "{}-{} {}{:>w$}{} {:>w$} │ {}\n",
                                ansi::RED,
                                ansi::RESET,
                                ansi::CYAN,
                                old_no,
                                ansi::RESET,
                                "",
                                del_formatted,
                                w = lineno_digits
                            ));
                            out.push_str(&format!(
                                "{}+{} {:>w$} {}{:>w$}{} │ {}\n",
                                ansi::GREEN,
                                ansi::RESET,
                                "",
                                ansi::CYAN,
                                new_no,
                                ansi::RESET,
                                ins_formatted,
                                w = lineno_digits
                            ));
                        } else {
                            out.push_str(&format!(
                                "- {:>w$} {:>w$} │ {}\n",
                                old_no,
                                "",
                                del_content,
                                w = lineno_digits
                            ));
                            out.push_str(&format!(
                                "+ {:>w$} {:>w$} │ {}\n",
                                "",
                                new_no,
                                ins_content,
                                w = lineno_digits
                            ));
                        }
                    } else {
                        if color {
                            out.push_str(&format!(
                                "{}-{} {}\n{}+{} {}\n",
                                ansi::RED,
                                ansi::RESET,
                                del_formatted,
                                ansi::GREEN,
                                ansi::RESET,
                                ins_formatted
                            ));
                        } else {
                            out.push_str(&format!("- {}\n+ {}\n", del_content, ins_content));
                        }
                    }
                }
                DiffChangeKind::Empty => {}
            }
        }
    }

    if config.show_summary {
        let summary = doc.stats.summary();
        if color {
            out.push_str(&format!(
                "\n{}{}{}{}\n",
                ansi::DIM,
                ansi::BOLD,
                summary,
                ansi::RESET
            ));
        } else {
            out.push_str(&format!("\n{}\n", summary));
        }
    }

    out
}

// ============================================================================
// ANSI Formatting Helpers
// ============================================================================

fn render_side_cell(
    cell: &SideBySideCell,
    content_width: usize,
    lineno_digits: usize,
    show_lineno: bool,
    color: bool,
    is_left: bool,
    tab_width: usize,
) -> String {
    if cell.is_empty {
        // Return blank cell with padding matching content_width
        let total_w = if show_lineno {
            lineno_digits + 2 + content_width
        } else {
            2 + content_width
        };
        return " ".repeat(total_w);
    }

    let mut out = String::new();
    let content = expand_tabs(&cell.content, tab_width);

    // Line number prefix
    if show_lineno {
        let lineno_str = cell
            .line_number
            .map(|n| format!("{:>w$}", n, w = lineno_digits))
            .unwrap_or_else(|| " ".repeat(lineno_digits));

        if color {
            let num_color = ansi::CYAN;
            out.push_str(&format!("{}{}{}", num_color, lineno_str, ansi::RESET));
        } else {
            out.push_str(&lineno_str);
        }
    }

    // Change sign glyph
    let sign = match cell.kind {
        DiffChangeKind::Deleted => "-",
        DiffChangeKind::Added => "+",
        DiffChangeKind::Modified => {
            if is_left {
                "-"
            } else {
                "+"
            }
        }
        _ => " ",
    };

    if color {
        let sign_color = match cell.kind {
            DiffChangeKind::Deleted => ansi::RED,
            DiffChangeKind::Added => ansi::GREEN,
            DiffChangeKind::Modified => {
                if is_left {
                    ansi::RED
                } else {
                    ansi::GREEN
                }
            }
            _ => ansi::DIM,
        };
        out.push_str(&format!(" {}{}{} ", sign_color, sign, ansi::RESET));
    } else {
        out.push_str(&format!(" {} ", sign));
    }

    // Format content with truncation / padding to fixed width
    let truncated_content = truncate_to_visible_width(&content, content_width);
    let vis_w = visible_width(&truncated_content);
    let pad_len = content_width.saturating_sub(vis_w);

    if color {
        let (base_color, hl_bg) = match cell.kind {
            DiffChangeKind::Deleted => (ansi::RED, ansi::BG_DARK_RED),
            DiffChangeKind::Added => (ansi::GREEN, ansi::BG_DARK_GREEN),
            DiffChangeKind::Modified => {
                if is_left {
                    (ansi::RED, ansi::BG_DARK_RED)
                } else {
                    (ansi::GREEN, ansi::BG_DARK_GREEN)
                }
            }
            _ => ("", ""),
        };

        if !cell.highlights.is_empty() {
            let highlighted = format_highlighted_line(
                &truncated_content,
                &cell.highlights,
                base_color,
                hl_bg,
                true,
            );
            out.push_str(&highlighted);
        } else if !base_color.is_empty() {
            out.push_str(&format!(
                "{}{}{}",
                base_color,
                truncated_content,
                ansi::RESET
            ));
        } else {
            out.push_str(&truncated_content);
        }
    } else {
        out.push_str(&truncated_content);
    }

    out.push_str(&" ".repeat(pad_len));
    out
}

/// Applies intra-line word difference highlighting.
fn format_highlighted_line(
    content: &str,
    highlights: &[HighlightRange],
    base_fg: &str,
    highlight_bg: &str,
    color: bool,
) -> String {
    if !color || highlights.is_empty() {
        return content.to_string();
    }

    let mut out = String::new();
    let mut curr_idx = 0;
    let bytes = content.as_bytes();

    for h in highlights {
        let start = h.start.min(bytes.len());
        let end = h.end.min(bytes.len());

        if start > curr_idx {
            if let Ok(segment) = std::str::from_utf8(&bytes[curr_idx..start]) {
                out.push_str(&format!("{}{}{}", base_fg, segment, ansi::RESET));
            }
        }

        if start < end {
            if let Ok(segment) = std::str::from_utf8(&bytes[start..end]) {
                out.push_str(&format!(
                    "{}{}{}{}{}",
                    highlight_bg,
                    ansi::BRIGHT_WHITE,
                    ansi::BOLD,
                    segment,
                    ansi::RESET
                ));
            }
        }

        curr_idx = end;
    }

    if curr_idx < bytes.len() {
        if let Ok(segment) = std::str::from_utf8(&bytes[curr_idx..]) {
            out.push_str(&format!("{}{}{}", base_fg, segment, ansi::RESET));
        }
    }

    out
}

/// Expands tab characters (`\t`) into spaces based on tab width.
fn expand_tabs(s: &str, tab_width: usize) -> String {
    if !s.contains('\t') {
        return s.to_string();
    }
    let mut out = String::with_capacity(s.len() + 8);
    let mut col = 0;
    for c in s.chars() {
        if c == '\t' {
            let spaces = tab_width - (col % tab_width);
            for _ in 0..spaces {
                out.push(' ');
            }
            col += spaces;
        } else {
            out.push(c);
            col += 1;
        }
    }
    out
}

/// Truncates a string so that its visible monospace width does not exceed `max_width`.
fn truncate_to_visible_width(s: &str, max_width: usize) -> String {
    let mut current_w = 0;
    let mut out = String::new();

    for c in s.chars() {
        let char_w = if c.is_control() {
            0
        } else if ('\u{1100}'..='\u{115F}').contains(&c)
            || ('\u{2E80}'..='\u{A4CF}').contains(&c)
            || ('\u{AC00}'..='\u{D7A3}').contains(&c)
            || ('\u{F900}'..='\u{FAFF}').contains(&c)
            || ('\u{1F300}'..='\u{1FAFF}').contains(&c)
        {
            2
        } else {
            1
        };

        if current_w + char_w > max_width {
            break;
        }

        out.push(c);
        current_w += char_w;
    }

    out
}

fn truncate_or_pad(s: &str, width: usize) -> String {
    let vis_w = visible_width(s);
    if vis_w > width {
        let mut out = truncate_to_visible_width(s, width.saturating_sub(1));
        out.push('…');
        out
    } else {
        let pad = width.saturating_sub(vis_w);
        format!("{}{}", s, " ".repeat(pad))
    }
}

// ============================================================================
// Box Drawing Characters & Frame Builders
// ============================================================================

struct BorderChars {
    tl: &'static str,
    tr: &'static str,
    bl: &'static str,
    br: &'static str,
    h: &'static str,
    v: &'static str,
    tm: &'static str,
    bm: &'static str,
    ml: &'static str,
    mr: &'static str,
    mm: &'static str,
}

impl BorderChars {
    fn from_style(style: DiffBorderStyle) -> Self {
        match style {
            DiffBorderStyle::Rounded => Self {
                tl: "╭",
                tr: "╮",
                bl: "╰",
                br: "╯",
                h: "─",
                v: "│",
                tm: "┬",
                bm: "┴",
                ml: "├",
                mr: "┤",
                mm: "┼",
            },
            DiffBorderStyle::Sharp => Self {
                tl: "┌",
                tr: "┐",
                bl: "└",
                br: "┘",
                h: "─",
                v: "│",
                tm: "┬",
                bm: "┴",
                ml: "├",
                mr: "┤",
                mm: "┼",
            },
            DiffBorderStyle::Double => Self {
                tl: "╔",
                tr: "╗",
                bl: "╚",
                br: "╝",
                h: "═",
                v: "║",
                tm: "╦",
                bm: "╩",
                ml: "╠",
                mr: "╣",
                mm: "╬",
            },
            DiffBorderStyle::Ascii => Self {
                tl: "+",
                tr: "+",
                bl: "+",
                br: "+",
                h: "-",
                v: "|",
                tm: "+",
                bm: "+",
                ml: "+",
                mr: "+",
                mm: "+",
            },
            DiffBorderStyle::Minimal | DiffBorderStyle::None => Self {
                tl: " ",
                tr: " ",
                bl: " ",
                br: " ",
                h: "─",
                v: "│",
                tm: "─",
                bm: "─",
                ml: "─",
                mr: "─",
                mm: "│",
            },
        }
    }
}

fn render_top_header(
    left_title: &str,
    right_title: &str,
    left_w: usize,
    right_w: usize,
    b: &BorderChars,
    color: bool,
) -> String {
    let l_badge = format!(" {} ", left_title);
    let r_badge = format!(" {} ", right_title);

    let l_line = make_header_segment(&l_badge, left_w, b.h);
    let r_line = make_header_segment(&r_badge, right_w, b.h);

    if color {
        format!(
            "{}{}{}{}{}{}{}{}",
            ansi::DIM,
            b.tl,
            ansi::RESET,
            l_line,
            ansi::DIM,
            b.tm,
            ansi::RESET,
            r_line,
        )
    } else {
        format!("{}{}{}{}{}", b.tl, l_line, b.tm, r_line, b.tr)
    }
}

fn make_header_segment(badge: &str, width: usize, fill_char: &str) -> String {
    let badge_w = visible_width(badge);
    if badge_w >= width {
        truncate_or_pad(badge, width)
    } else {
        let remaining = width - badge_w;
        let left_pad = 2.min(remaining);
        let right_pad = remaining.saturating_sub(left_pad);
        format!(
            "{}{}{}",
            fill_char.repeat(left_pad),
            badge,
            fill_char.repeat(right_pad)
        )
    }
}

fn render_hunk_separator(
    hunk_hdr: &str,
    left_w: usize,
    right_w: usize,
    b: &BorderChars,
    color: bool,
) -> String {
    let badge = format!(" {} ", hunk_hdr);
    let total_w = left_w + right_w + 1;
    let segment = make_header_segment(&badge, total_w, b.h);

    if color {
        format!(
            "{}{}{}{}{}{}",
            ansi::DIM,
            b.ml,
            ansi::RESET,
            segment,
            ansi::DIM,
            b.mr
        )
    } else {
        format!("{}{}{}", b.ml, segment, b.mr)
    }
}

fn render_mid_separator(left_w: usize, right_w: usize, b: &BorderChars, color: bool) -> String {
    let l_line = b.h.repeat(left_w);
    let r_line = b.h.repeat(right_w);

    if color {
        format!(
            "{}{}{}{}{}{}{}{}",
            ansi::DIM,
            b.ml,
            l_line,
            b.mm,
            r_line,
            b.mr,
            ansi::RESET,
            ""
        )
    } else {
        format!("{}{}{}{}{}", b.ml, l_line, b.mm, r_line, b.mr)
    }
}

fn render_bottom_summary(
    summary: &str,
    left_w: usize,
    right_w: usize,
    b: &BorderChars,
    color: bool,
) -> String {
    let badge = format!(" {} ", summary);
    let total_w = left_w + right_w + 1;
    let segment = make_header_segment(&badge, total_w, b.h);

    if color {
        format!(
            "{}{}{}{}{}{}",
            ansi::DIM,
            b.bl,
            ansi::RESET,
            segment,
            ansi::DIM,
            b.br
        )
    } else {
        format!("{}{}{}", b.bl, segment, b.br)
    }
}

// ============================================================================
// Ratatui Widget Integration
// ============================================================================

/// Ratatui [`Widget`] for rendering adaptive side-by-side or unified diffs in interactive TUIs.
pub struct SideBySideWidget<'a> {
    doc: Cow<'a, SideBySideDocument>,
    config: AdaptiveDiffConfig,
    scroll_y: usize,
    scroll_x: usize,
}

impl<'a> SideBySideWidget<'a> {
    /// Creates a widget from a reference to a [`SideBySideDocument`].
    pub fn new(doc: &'a SideBySideDocument) -> Self {
        Self {
            doc: Cow::Borrowed(doc),
            config: AdaptiveDiffConfig::default(),
            scroll_y: 0,
            scroll_x: 0,
        }
    }

    /// Creates a widget by taking ownership of a [`SideBySideDocument`].
    pub fn from_owned(doc: SideBySideDocument) -> Self {
        Self {
            doc: Cow::Owned(doc),
            config: AdaptiveDiffConfig::default(),
            scroll_y: 0,
            scroll_x: 0,
        }
    }

    /// Creates a widget by computing the diff between two strings.
    pub fn from_texts(
        old_content: &str,
        new_content: &str,
        file_path: Option<&str>,
        context_radius: usize,
    ) -> Self {
        let doc = SideBySideDocument::from_texts(
            old_content,
            new_content,
            file_path,
            context_radius,
            true,
        );
        Self::from_owned(doc)
    }

    /// Sets the diff configuration.
    pub fn with_config(mut self, config: AdaptiveDiffConfig) -> Self {
        self.config = config;
        self
    }

    /// Sets the vertical scroll line offset.
    pub fn with_scroll_y(mut self, scroll_y: usize) -> Self {
        self.scroll_y = scroll_y;
        self
    }

    /// Sets the horizontal scroll character offset.
    pub fn with_scroll_x(mut self, scroll_x: usize) -> Self {
        self.scroll_x = scroll_x;
        self
    }
}

impl<'a> Widget for SideBySideWidget<'a> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        if area.width < 10 || area.height < 3 {
            return;
        }

        let theme = &self.config.theme;

        // Adaptive layout decision based on available rect width
        let is_termux_env = self.config.force_termux.unwrap_or_else(is_termux);
        let render_side_by_side = match self.config.mode {
            DiffDisplayMode::SideBySide => true,
            DiffDisplayMode::Unified => false,
            DiffDisplayMode::Auto => is_side_by_side_supported(
                area.width as usize,
                is_termux_env,
                self.config.min_side_by_side_width,
            ),
        };

        if render_side_by_side {
            self.render_ratatui_side_by_side(area, buf, theme);
        } else {
            self.render_ratatui_unified(area, buf, theme);
        }
    }
}

impl<'a> SideBySideWidget<'a> {
    fn render_ratatui_side_by_side(&self, area: Rect, buf: &mut Buffer, theme: &Theme) {
        let title = self
            .doc
            .file_path
            .as_deref()
            .map(|p| format!(" Side-by-Side: {} ", p))
            .unwrap_or_else(|| " Side-by-Side Diff ".to_string());

        let block = Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(theme.border_focused))
            .title(title)
            .title_alignment(Alignment::Left);

        let inner = block.inner(area);
        block.render(area, buf);

        if inner.width < 20 || inner.height == 0 {
            return;
        }

        // Split columns horizontally
        let chunks = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
            .split(inner);

        let left_area = chunks[0];
        let right_area = chunks[1];

        let lang = self
            .doc
            .file_path
            .as_deref()
            .map(SyntaxLanguage::from_path)
            .unwrap_or(SyntaxLanguage::PlainText);

        let mut left_lines = Vec::new();
        let mut right_lines = Vec::new();

        for hunk in &self.doc.hunks {
            let _hunk_hdr = hunk.unified_header();

            // Hunk header row
            left_lines.push(Line::from(vec![Span::styled(
                format!("@@ -{},{} @@", hunk.old_start, hunk.old_lines),
                Style::default()
                    .fg(theme.accent)
                    .add_modifier(Modifier::BOLD),
            )]));
            right_lines.push(Line::from(vec![Span::styled(
                format!("@@ +{},{} @@", hunk.new_start, hunk.new_lines),
                Style::default()
                    .fg(theme.accent)
                    .add_modifier(Modifier::BOLD),
            )]));

            for row in &hunk.rows {
                // Left column line
                let mut l_spans = Vec::new();
                if let Some(old_no) = row.left.line_number {
                    l_spans.push(Span::styled(
                        format!("{:4} │ ", old_no),
                        Style::default().fg(Color::Cyan),
                    ));
                } else {
                    l_spans.push(Span::styled("     │ ", Style::default().fg(theme.border)));
                }

                match row.left.kind {
                    DiffChangeKind::Deleted | DiffChangeKind::Modified => {
                        l_spans.push(Span::styled("- ", Style::default().fg(Color::Red)));
                        let tokens = tokenize_line(&row.left.content, lang);
                        let spans =
                            highlight_tokens_to_spans(&tokens, DiffLineType::Deletion, theme);
                        l_spans.extend(spans);
                    }
                    DiffChangeKind::Unchanged => {
                        l_spans.push(Span::styled("  ", Style::default().fg(theme.muted)));
                        let tokens = tokenize_line(&row.left.content, lang);
                        let spans =
                            highlight_tokens_to_spans(&tokens, DiffLineType::Context, theme);
                        l_spans.extend(spans);
                    }
                    DiffChangeKind::Empty | DiffChangeKind::Added => {
                        // Empty spacer
                    }
                }
                left_lines.push(Line::from(l_spans));

                // Right column line
                let mut r_spans = Vec::new();
                if let Some(new_no) = row.right.line_number {
                    r_spans.push(Span::styled(
                        format!("{:4} │ ", new_no),
                        Style::default().fg(Color::Cyan),
                    ));
                } else {
                    r_spans.push(Span::styled("     │ ", Style::default().fg(theme.border)));
                }

                match row.right.kind {
                    DiffChangeKind::Added | DiffChangeKind::Modified => {
                        r_spans.push(Span::styled("+ ", Style::default().fg(Color::Green)));
                        let tokens = tokenize_line(&row.right.content, lang);
                        let spans =
                            highlight_tokens_to_spans(&tokens, DiffLineType::Addition, theme);
                        r_spans.extend(spans);
                    }
                    DiffChangeKind::Unchanged => {
                        r_spans.push(Span::styled("  ", Style::default().fg(theme.muted)));
                        let tokens = tokenize_line(&row.right.content, lang);
                        let spans =
                            highlight_tokens_to_spans(&tokens, DiffLineType::Context, theme);
                        r_spans.extend(spans);
                    }
                    DiffChangeKind::Empty | DiffChangeKind::Deleted => {
                        // Empty spacer
                    }
                }
                right_lines.push(Line::from(r_spans));
            }

            // Hunk separator line
            left_lines.push(Line::from(Span::styled(
                "─".repeat(left_area.width as usize),
                Style::default().fg(theme.border),
            )));
            right_lines.push(Line::from(Span::styled(
                "─".repeat(right_area.width as usize),
                Style::default().fg(theme.border),
            )));
        }

        let left_visible: Vec<Line> = left_lines.into_iter().skip(self.scroll_y).collect();
        let right_visible: Vec<Line> = right_lines.into_iter().skip(self.scroll_y).collect();

        Paragraph::new(left_visible)
            .scroll((0, self.scroll_x as u16))
            .render(left_area, buf);
        Paragraph::new(right_visible)
            .scroll((0, self.scroll_x as u16))
            .render(right_area, buf);
    }

    fn render_ratatui_unified(&self, area: Rect, buf: &mut Buffer, theme: &Theme) {
        let title = self
            .doc
            .file_path
            .as_deref()
            .map(|p| format!(" Unified: {} ", p))
            .unwrap_or_else(|| " Unified Diff ".to_string());

        let block = Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(theme.border_focused))
            .title(title)
            .title_alignment(Alignment::Left);

        let inner = block.inner(area);
        block.render(area, buf);

        if inner.width == 0 || inner.height == 0 {
            return;
        }

        let lang = self
            .doc
            .file_path
            .as_deref()
            .map(SyntaxLanguage::from_path)
            .unwrap_or(SyntaxLanguage::PlainText);

        let mut lines = Vec::new();

        for hunk in &self.doc.hunks {
            lines.push(Line::from(vec![
                Span::styled(
                    "@@ ",
                    Style::default()
                        .fg(theme.accent)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    format!(
                        "-{},{} +{},{}",
                        hunk.old_start, hunk.old_lines, hunk.new_start, hunk.new_lines
                    ),
                    Style::default().fg(theme.accent),
                ),
                Span::styled(
                    " @@",
                    Style::default()
                        .fg(theme.accent)
                        .add_modifier(Modifier::BOLD),
                ),
            ]));

            for row in &hunk.rows {
                match row.kind {
                    DiffChangeKind::Unchanged => {
                        let tokens = tokenize_line(&row.left.content, lang);
                        let spans =
                            highlight_tokens_to_spans(&tokens, DiffLineType::Context, theme);
                        let mut r = vec![
                            Span::styled(
                                format!("{:4} │ ", row.left.line_number.unwrap_or(0)),
                                Style::default().fg(Color::Cyan),
                            ),
                            Span::styled("  ", Style::default().fg(theme.muted)),
                        ];
                        r.extend(spans);
                        lines.push(Line::from(r));
                    }
                    DiffChangeKind::Deleted => {
                        let tokens = tokenize_line(&row.left.content, lang);
                        let spans =
                            highlight_tokens_to_spans(&tokens, DiffLineType::Deletion, theme);
                        let mut r = vec![
                            Span::styled(
                                format!("{:4} │ ", row.left.line_number.unwrap_or(0)),
                                Style::default().fg(Color::Cyan),
                            ),
                            Span::styled("- ", Style::default().fg(Color::Red)),
                        ];
                        r.extend(spans);
                        lines.push(Line::from(r));
                    }
                    DiffChangeKind::Added => {
                        let tokens = tokenize_line(&row.right.content, lang);
                        let spans =
                            highlight_tokens_to_spans(&tokens, DiffLineType::Addition, theme);
                        let mut r = vec![
                            Span::styled(
                                format!("{:4} │ ", row.right.line_number.unwrap_or(0)),
                                Style::default().fg(Color::Cyan),
                            ),
                            Span::styled("+ ", Style::default().fg(Color::Green)),
                        ];
                        r.extend(spans);
                        lines.push(Line::from(r));
                    }
                    DiffChangeKind::Modified => {
                        // Deletion line
                        let del_tokens = tokenize_line(&row.left.content, lang);
                        let del_spans =
                            highlight_tokens_to_spans(&del_tokens, DiffLineType::Deletion, theme);
                        let mut del_r = vec![
                            Span::styled(
                                format!("{:4} │ ", row.left.line_number.unwrap_or(0)),
                                Style::default().fg(Color::Cyan),
                            ),
                            Span::styled("- ", Style::default().fg(Color::Red)),
                        ];
                        del_r.extend(del_spans);
                        lines.push(Line::from(del_r));

                        // Addition line
                        let ins_tokens = tokenize_line(&row.right.content, lang);
                        let ins_spans =
                            highlight_tokens_to_spans(&ins_tokens, DiffLineType::Addition, theme);
                        let mut ins_r = vec![
                            Span::styled(
                                format!("{:4} │ ", row.right.line_number.unwrap_or(0)),
                                Style::default().fg(Color::Cyan),
                            ),
                            Span::styled("+ ", Style::default().fg(Color::Green)),
                        ];
                        ins_r.extend(ins_spans);
                        lines.push(Line::from(ins_r));
                    }
                    DiffChangeKind::Empty => {}
                }
            }

            lines.push(Line::from(Span::styled(
                "─".repeat(inner.width as usize),
                Style::default().fg(theme.border),
            )));
        }

        let visible: Vec<Line> = lines.into_iter().skip(self.scroll_y).collect();
        Paragraph::new(visible)
            .scroll((0, self.scroll_x as u16))
            .render(inner, buf);
    }
}

// ============================================================================
// Convenience Functions
// ============================================================================

/// Quick helper to format and print an adaptive diff between two strings.
pub fn print_diff(old_content: &str, new_content: &str, file_path: Option<&str>) {
    let rendered = render_adaptive(old_content, new_content, file_path, None);
    println!("{}", rendered);
}

/// Helper to render a diff with a specified explicit terminal width.
pub fn render_diff_with_width(
    old_content: &str,
    new_content: &str,
    file_path: Option<&str>,
    width: usize,
) -> String {
    let config = AdaptiveDiffConfig::default().with_terminal_width(width);
    render_adaptive(old_content, new_content, file_path, Some(&config))
}

// ============================================================================
// Unit Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_side_by_side_support_threshold() {
        // >= 80 columns in standard terminal -> side by side supported
        assert!(is_side_by_side_supported(80, false, 80));
        assert!(is_side_by_side_supported(120, false, 80));

        // < 80 columns in standard terminal -> not supported
        assert!(!is_side_by_side_supported(79, false, 80));
        assert!(!is_side_by_side_supported(60, false, 80));

        // Termux environment on narrow screens -> forced fallback to unified
        assert!(!is_side_by_side_supported(60, true, 80));
        assert!(!is_side_by_side_supported(79, true, 80));
        assert!(is_side_by_side_supported(100, true, 80));
    }

    #[test]
    fn test_resolve_display_mode() {
        let mut cfg = AdaptiveDiffConfig::default();

        // Auto mode with width 120 -> Side-by-Side
        cfg.terminal_width = Some(120);
        cfg.force_termux = Some(false);
        assert_eq!(resolve_display_mode(&cfg), DiffDisplayMode::SideBySide);

        // Auto mode with width 60 -> Unified
        cfg.terminal_width = Some(60);
        assert_eq!(resolve_display_mode(&cfg), DiffDisplayMode::Unified);

        // Auto mode in Termux with width 60 -> Unified
        cfg.force_termux = Some(true);
        assert_eq!(resolve_display_mode(&cfg), DiffDisplayMode::Unified);

        // Forced modes override automatic detection
        cfg.mode = DiffDisplayMode::SideBySide;
        assert_eq!(resolve_display_mode(&cfg), DiffDisplayMode::SideBySide);

        cfg.mode = DiffDisplayMode::Unified;
        cfg.terminal_width = Some(160);
        assert_eq!(resolve_display_mode(&cfg), DiffDisplayMode::Unified);
    }

    #[test]
    fn test_document_from_texts_pairing() {
        let old = "fn main() {\n    println!(\"Hello\");\n}\n";
        let new = "fn main() {\n    println!(\"Hello, World!\");\n    return;\n}\n";

        let doc = SideBySideDocument::from_texts(old, new, Some("main.rs"), 3, true);

        assert_eq!(doc.hunks.len(), 1);
        assert!(doc.stats.additions > 0);
        assert!(doc.stats.deletions > 0);

        let hunk = &doc.hunks[0];
        // Ensure unchanged rows pair properly
        assert_eq!(hunk.rows[0].kind, DiffChangeKind::Unchanged);
        assert_eq!(hunk.rows[0].left.content, "fn main() {");
        assert_eq!(hunk.rows[0].right.content, "fn main() {");
    }

    #[test]
    fn test_intra_line_word_diff() {
        let old_line = "let count = 42;";
        let new_line = "let count = 100;";

        let (del_ranges, ins_ranges) = compute_intra_line_highlights(old_line, new_line);

        assert_eq!(del_ranges.len(), 1);
        assert_eq!(ins_ranges.len(), 1);

        assert_eq!(&old_line[del_ranges[0].start..del_ranges[0].end], "42");
        assert_eq!(&new_line[ins_ranges[0].start..ins_ranges[0].end], "100");
    }

    #[test]
    fn test_render_side_by_side_ansi_no_panic() {
        let old = "alpha\nbeta\ngamma\n";
        let new = "alpha\nbeta_updated\ngamma\ndelta\n";

        let config = AdaptiveDiffConfig::default()
            .with_terminal_width(100)
            .with_color(false);

        let doc = SideBySideDocument::from_texts(old, new, Some("test.txt"), 3, true);
        let rendered = render_side_by_side_ansi(&doc, 100, &config);

        assert!(rendered.contains("alpha"));
        assert!(rendered.contains("beta_updated"));
        assert!(rendered.contains("test.txt"));
    }

    #[test]
    fn test_render_unified_ansi_fallback() {
        let old = "foo\nbar\n";
        let new = "foo\nbaz\n";

        let config = AdaptiveDiffConfig::default()
            .with_terminal_width(60)
            .with_color(false);

        let doc = SideBySideDocument::from_texts(old, new, Some("sample.txt"), 3, true);
        let rendered = render_unified_ansi(&doc, 60, &config);

        assert!(rendered.contains("--- a/sample.txt") || rendered.contains("sample.txt"));
        assert!(rendered.contains("- "));
        assert!(rendered.contains("+ "));
    }

    #[test]
    fn test_adaptive_render_width_switching() {
        let old = "Line 1\nLine 2\nLine 3\n";
        let new = "Line 1\nLine 2 modified\nLine 3\n";

        // Wide terminal -> Side-by-Side (contains divider border)
        let wide_rendered = render_diff_with_width(old, new, Some("file.rs"), 100);
        assert!(wide_rendered.contains("│") || wide_rendered.contains("|"));

        // Narrow terminal -> Unified fallback
        let narrow_rendered = render_diff_with_width(old, new, Some("file.rs"), 50);
        assert!(narrow_rendered.contains("@@"));
    }

    #[test]
    fn test_empty_diff_and_identical_content() {
        let text = "exact same content\nsecond line\n";
        let rendered = render_adaptive(text, text, Some("same.txt"), None);
        assert!(rendered.contains("No changes detected"));
    }

    #[test]
    fn test_ratatui_widget_rendering() {
        let old = "first\nsecond\n";
        let new = "first\nsecond_new\n";

        let doc = SideBySideDocument::from_texts(old, new, Some("test.rs"), 3, true);

        // Wide area -> Side-by-side
        let mut buf_wide = Buffer::empty(Rect::new(0, 0, 100, 20));
        let widget_wide = SideBySideWidget::new(&doc);
        widget_wide.render(Rect::new(0, 0, 100, 20), &mut buf_wide);

        // Narrow area -> Unified
        let mut buf_narrow = Buffer::empty(Rect::new(0, 0, 50, 20));
        let widget_narrow = SideBySideWidget::new(&doc);
        widget_narrow.render(Rect::new(0, 0, 50, 20), &mut buf_narrow);
    }

    #[test]
    fn test_diff_alignment_pure_addition() {
        let old = "line 1\nline 2\n";
        let new = "line 1\nnew line inserted\nline 2\n";

        let doc = SideBySideDocument::from_texts(old, new, None, 3, true);
        assert_eq!(doc.hunks.len(), 1);

        let hunk = &doc.hunks[0];
        assert_eq!(hunk.rows.len(), 3);

        // Row 0: Context line 1
        assert_eq!(hunk.rows[0].kind, DiffChangeKind::Unchanged);
        assert_eq!(hunk.rows[0].left.line_number, Some(1));
        assert_eq!(hunk.rows[0].right.line_number, Some(1));

        // Row 1: Addition (empty left spacer, populated right cell)
        assert_eq!(hunk.rows[1].kind, DiffChangeKind::Added);
        assert_eq!(hunk.rows[1].left.is_empty, true);
        assert_eq!(hunk.rows[1].left.line_number, None);
        assert_eq!(hunk.rows[1].right.is_empty, false);
        assert_eq!(hunk.rows[1].right.line_number, Some(2));
        assert_eq!(hunk.rows[1].right.content, "new line inserted");

        // Row 2: Context line 2
        assert_eq!(hunk.rows[2].kind, DiffChangeKind::Unchanged);
        assert_eq!(hunk.rows[2].left.line_number, Some(2));
        assert_eq!(hunk.rows[2].right.line_number, Some(3));
    }

    #[test]
    fn test_diff_alignment_pure_deletion() {
        let old = "line 1\nline to delete\nline 2\n";
        let new = "line 1\nline 2\n";

        let doc = SideBySideDocument::from_texts(old, new, None, 3, true);
        assert_eq!(doc.hunks.len(), 1);

        let hunk = &doc.hunks[0];
        assert_eq!(hunk.rows.len(), 3);

        // Row 0: Context line 1
        assert_eq!(hunk.rows[0].kind, DiffChangeKind::Unchanged);
        assert_eq!(hunk.rows[0].left.line_number, Some(1));
        assert_eq!(hunk.rows[0].right.line_number, Some(1));

        // Row 1: Deletion (populated left cell, empty right spacer)
        assert_eq!(hunk.rows[1].kind, DiffChangeKind::Deleted);
        assert_eq!(hunk.rows[1].left.is_empty, false);
        assert_eq!(hunk.rows[1].left.line_number, Some(2));
        assert_eq!(hunk.rows[1].left.content, "line to delete");
        assert_eq!(hunk.rows[1].right.is_empty, true);
        assert_eq!(hunk.rows[1].right.line_number, None);

        // Row 2: Context line 2
        assert_eq!(hunk.rows[2].kind, DiffChangeKind::Unchanged);
        assert_eq!(hunk.rows[2].left.line_number, Some(3));
        assert_eq!(hunk.rows[2].right.line_number, Some(2));
    }

    #[test]
    fn test_diff_alignment_replacement_modification() {
        let old = "let alpha = 10;\nlet beta = 20;\n";
        let new = "let alpha = 99;\nlet beta = 20;\n";

        let doc = SideBySideDocument::from_texts(old, new, None, 3, true);
        assert_eq!(doc.hunks.len(), 1);

        let hunk = &doc.hunks[0];
        // Row 0: Modified line
        assert_eq!(hunk.rows[0].kind, DiffChangeKind::Modified);
        assert_eq!(hunk.rows[0].left.line_number, Some(1));
        assert_eq!(hunk.rows[0].left.content, "let alpha = 10;");
        assert_eq!(hunk.rows[0].right.line_number, Some(1));
        assert_eq!(hunk.rows[0].right.content, "let alpha = 99;");
        assert!(!hunk.rows[0].left.highlights.is_empty());
        assert!(!hunk.rows[0].right.highlights.is_empty());

        // Row 1: Context line
        assert_eq!(hunk.rows[1].kind, DiffChangeKind::Unchanged);
        assert_eq!(hunk.rows[1].left.line_number, Some(2));
        assert_eq!(hunk.rows[1].right.line_number, Some(2));
    }

    #[test]
    fn test_diff_alignment_mixed_hunks() {
        let old = "header\nold1\nold2\ncommon_mid\ncommon_mid2\nold_tail\nfooter\n";
        let new = "header\nnew1\ncommon_mid\ncommon_mid2\nnew_tail1\nnew_tail2\nfooter\n";

        let doc = SideBySideDocument::from_texts(old, new, Some("mixed.rs"), 1, true);
        assert!(!doc.hunks.is_empty());

        for hunk in &doc.hunks {
            for row in &hunk.rows {
                // Ensure synchronized left and right representation
                match row.kind {
                    DiffChangeKind::Unchanged => {
                        assert!(row.left.line_number.is_some());
                        assert!(row.right.line_number.is_some());
                        assert_eq!(row.left.content, row.right.content);
                    }
                    DiffChangeKind::Deleted => {
                        assert!(row.left.line_number.is_some());
                        assert!(row.right.is_empty);
                    }
                    DiffChangeKind::Added => {
                        assert!(row.left.is_empty);
                        assert!(row.right.line_number.is_some());
                    }
                    DiffChangeKind::Modified => {
                        assert!(row.left.line_number.is_some());
                        assert!(row.right.line_number.is_some());
                    }
                    DiffChangeKind::Empty => {}
                }
            }
        }
    }

    #[test]
    fn test_line_splitting_crlf_and_lf() {
        let old = "line1\r\nline2\r\nline3\r\n";
        let new = "line1\nline2_mod\nline3\n";

        let doc = SideBySideDocument::from_texts(old, new, None, 3, false);
        assert_eq!(doc.hunks.len(), 1);

        let hunk = &doc.hunks[0];
        for row in &hunk.rows {
            assert!(!row.left.content.ends_with('\r'));
            assert!(!row.left.content.ends_with('\n'));
            assert!(!row.right.content.ends_with('\r'));
            assert!(!row.right.content.ends_with('\n'));
        }
    }

    #[test]
    fn test_line_splitting_empty_lines_and_trailing() {
        let old = "start\n\nmiddle\nend";
        let new = "start\n\nmiddle_updated\nend\n";

        let doc = SideBySideDocument::from_texts(old, new, None, 3, true);
        assert_eq!(doc.hunks.len(), 1);

        let hunk = &doc.hunks[0];
        // Ensure empty line preserved in diff
        assert_eq!(hunk.rows[1].left.content, "");
        assert_eq!(hunk.rows[1].right.content, "");
        assert_eq!(hunk.rows[1].kind, DiffChangeKind::Unchanged);
    }

    #[test]
    fn test_line_splitting_tabs_expansion() {
        assert_eq!(expand_tabs("\thello", 4), "    hello");
        assert_eq!(expand_tabs("a\tb", 4), "a   b");
        assert_eq!(expand_tabs("abcd\te", 4), "abcd    e");
        assert_eq!(expand_tabs("no_tabs", 4), "no_tabs");
    }

    #[test]
    fn test_truncate_to_visible_width() {
        assert_eq!(truncate_to_visible_width("hello world", 5), "hello");
        assert_eq!(truncate_to_visible_width("short", 10), "short");
        assert_eq!(truncate_to_visible_width("abc", 0), "");
    }

    #[test]
    fn test_ratatui_widget_scrolling() {
        let old = (1..=50)
            .map(|i| format!("old line {}", i))
            .collect::<Vec<_>>()
            .join("\n");
        let new = (1..=50)
            .map(|i| format!("new line {}", i))
            .collect::<Vec<_>>()
            .join("\n");

        let doc = SideBySideDocument::from_texts(&old, &new, Some("scroll.txt"), 1, true);

        // Render with vertical scroll offset
        let mut buf = Buffer::empty(Rect::new(0, 0, 100, 10));
        let widget = SideBySideWidget::new(&doc)
            .with_scroll_y(5)
            .with_scroll_x(2);

        widget.render(Rect::new(0, 0, 100, 10), &mut buf);
        // Ensure widget renders without panic under scroll offsets
    }

    #[test]
    fn test_ratatui_widget_color_styling_spans() {
        let old = "remove_me\nsame\n";
        let new = "insert_me\nsame\n";

        let doc = SideBySideDocument::from_texts(old, new, None, 3, true);
        let mut buf = Buffer::empty(Rect::new(0, 0, 100, 10));
        let widget = SideBySideWidget::new(&doc);
        widget.render(Rect::new(0, 0, 100, 10), &mut buf);

        // Verify buffer has characters written
        let mut has_non_empty = false;
        for y in 0..10 {
            for x in 0..100 {
                if buf.get(x, y).symbol() != " " {
                    has_non_empty = true;
                    break;
                }
            }
        }
        assert!(has_non_empty);
    }

    #[test]
    fn test_side_by_side_cell_and_row_constructors() {
        let cell =
            SideBySideCell::new(Some(42), "content", DiffChangeKind::Added).with_highlight(0, 4);
        assert_eq!(cell.line_number, Some(42));
        assert_eq!(cell.content, "content");
        assert_eq!(cell.kind, DiffChangeKind::Added);
        assert_eq!(cell.highlights.len(), 1);
        assert!(!cell.is_empty);

        let empty_cell = SideBySideCell::empty();
        assert!(empty_cell.is_empty);
        assert_eq!(empty_cell.line_number, None);

        let ctx_row = SideBySideRow::context(1, 1, "unchanged");
        assert_eq!(ctx_row.kind, DiffChangeKind::Unchanged);

        let del_row = SideBySideRow::deletion(5, "deleted");
        assert_eq!(del_row.kind, DiffChangeKind::Deleted);
        assert!(del_row.right.is_empty);

        let add_row = SideBySideRow::addition(6, "added");
        assert_eq!(add_row.kind, DiffChangeKind::Added);
        assert!(add_row.left.is_empty);
    }
}
