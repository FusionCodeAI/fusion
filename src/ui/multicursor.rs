//! Multiline text buffer manipulation and multi-cursor editing engine.
//!
//! Provides comprehensive text manipulation helpers for terminal prompt editing,
//! multi-line buffers, word wrapping, horizontal scrolling, line jumping,
//! multi-cursor coordination, rectangular block operations, undo/redo history,
//! and standard Emacs/Readline keybindings.
//!
//! # Architecture
//!
//! - [`MultilineBuffer`]: Core Unicode-aware character buffer with 2D coordinate indexing.
//! - [`Cursor`] & [`MultiCursorState`]: Multi-cursor position and selection manager with auto-merging.
//! - [`WordWrapEngine`] & [`WrappedLine`]: Visual layout, soft/hard wrapping, and paragraph reflow.
//! - [`HorizontalScrollState`]: Windowed viewport rendering for long lines.
//! - [`KillRing`]: Readline kill ring supporting cumulative kill appending and yank-pop cycling.
//! - [`ReadlineKey`] & [`ReadlineCommand`]: Event parsing and keybinding translation.
//! - [`LineJumpHelper`]: Target-based jumping, bracket matching, paragraph/heading navigation, subwords.
//! - [`BlockOperations`]: Column/rectangular selection, indentation, commenting, dragging, alignment.
//! - [`BufferHistory`]: Transactional undo/redo stack.
//! - [`EditorBuffer`]: High-level facade integrating buffer, multi-cursor, readline engine, and wrapping.

use serde::{Deserialize, Serialize};
use std::cmp::{max, min, Ordering};
use std::fmt;

// ---------------------------------------------------------------------------
// Position, Range, and Selection Data Structures
// ---------------------------------------------------------------------------

/// 2D text coordinate (0-indexed line and character column).
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, Default,
)]
pub struct Position {
    /// 0-indexed line number.
    pub line: usize,
    /// 0-indexed character column on the line.
    pub col: usize,
}

impl Position {
    /// Create a new 2D position.
    pub const fn new(line: usize, col: usize) -> Self {
        Self { line, col }
    }

    /// Origin position (0, 0).
    pub const fn origin() -> Self {
        Self { line: 0, col: 0 }
    }
}

impl fmt::Display for Position {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}", self.line + 1, self.col + 1)
    }
}

/// A 2D text range spanning between two [`Position`] coordinates.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
pub struct TextRange {
    /// Start position (inclusive).
    pub start: Position,
    /// End position (exclusive).
    pub end: Position,
}

impl TextRange {
    /// Create a new text range. Automatically normalizes start and end.
    pub fn new(p1: Position, p2: Position) -> Self {
        if p1 <= p2 {
            Self { start: p1, end: p2 }
        } else {
            Self { start: p2, end: p1 }
        }
    }

    /// Whether this range is collapsed (zero length).
    pub fn is_empty(&self) -> bool {
        self.start == self.end
    }

    /// Check if a position lies within this range (start inclusive, end exclusive).
    pub fn contains_position(&self, pos: Position) -> bool {
        pos >= self.start && pos < self.end
    }

    /// Check if another range overlaps with this range.
    pub fn overlaps(&self, other: &TextRange) -> bool {
        self.start < other.end && other.start < self.end
    }
}

/// Linear text selection defined by character offsets in the buffer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Selection {
    /// Fixed anchor point where selection started.
    pub anchor: usize,
    /// Active head point (current cursor location).
    pub head: usize,
}

impl Selection {
    /// Create a new selection from anchor to head.
    pub const fn new(anchor: usize, head: usize) -> Self {
        Self { anchor, head }
    }

    /// Create a collapsed selection (cursor without highlighted span).
    pub const fn collapsed(offset: usize) -> Self {
        Self {
            anchor: offset,
            head: offset,
        }
    }

    /// Returns true if selection is collapsed (empty).
    pub fn is_collapsed(&self) -> bool {
        self.anchor == self.head
    }

    /// Returns the minimum character offset (inclusive start).
    pub fn start(&self) -> usize {
        min(self.anchor, self.head)
    }

    /// Returns the maximum character offset (exclusive end).
    pub fn end(&self) -> usize {
        max(self.anchor, self.head)
    }

    /// Total character length of the selection.
    pub fn len(&self) -> usize {
        self.end() - self.start()
    }

    /// Returns true if selection is empty.
    pub fn is_empty(&self) -> bool {
        self.is_collapsed()
    }

    /// Whether selection is directed forward (head >= anchor).
    pub fn is_forward(&self) -> bool {
        self.head >= self.anchor
    }

    /// Whether selection is directed backward (head < anchor).
    pub fn is_backward(&self) -> bool {
        self.head < self.anchor
    }

    /// Check if this selection contains a given character offset.
    pub fn contains(&self, offset: usize) -> bool {
        offset >= self.start() && offset < self.end()
    }

    /// Check if this selection overlaps with another.
    pub fn overlaps(&self, other: &Selection) -> bool {
        self.start() < other.end() && other.start() < self.end()
    }

    /// Check if this selection touches or overlaps another.
    pub fn touches_or_overlaps(&self, other: &Selection) -> bool {
        self.start() <= other.end() && other.start() <= self.end()
    }
}

/// An individual editing cursor with optional selection and visual column memory.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Cursor {
    /// Active character offset in buffer.
    pub offset: usize,
    /// Selection anchor offset, if text is selected.
    pub anchor: Option<usize>,
    /// Preferred visual column when moving vertically across lines of varying length.
    pub desired_col: Option<usize>,
}

impl Cursor {
    /// Create a new collapsed cursor at character offset.
    pub const fn new(offset: usize) -> Self {
        Self {
            offset,
            anchor: None,
            desired_col: None,
        }
    }

    /// Create a cursor with an active selection range.
    pub fn with_selection(anchor: usize, head: usize) -> Self {
        Self {
            offset: head,
            anchor: if anchor != head { Some(anchor) } else { None },
            desired_col: None,
        }
    }

    /// Get current selection if active, or collapsed selection at cursor.
    pub fn selection(&self) -> Selection {
        match self.anchor {
            Some(anchor) => Selection::new(anchor, self.offset),
            None => Selection::collapsed(self.offset),
        }
    }

    /// Returns true if cursor has an active (non-collapsed) selection.
    pub fn has_selection(&self) -> bool {
        self.anchor.is_some() && self.anchor != Some(self.offset)
    }

    /// Selection start (min of offset and anchor).
    pub fn selection_start(&self) -> usize {
        self.selection().start()
    }

    /// Selection end (max of offset and anchor).
    pub fn selection_end(&self) -> usize {
        self.selection().end()
    }

    /// Collapse selection to current cursor offset.
    pub fn collapse(&mut self) {
        self.anchor = None;
    }

    /// Collapse selection to start offset.
    pub fn collapse_to_start(&mut self) {
        self.offset = self.selection_start();
        self.anchor = None;
    }

    /// Collapse selection to end offset.
    pub fn collapse_to_end(&mut self) {
        self.offset = self.selection_end();
        self.anchor = None;
    }
}

// ---------------------------------------------------------------------------
// Rectangular / Block Selection Range
// ---------------------------------------------------------------------------

/// 2D rectangular box selection spanning multiple lines between column bounds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct BlockRange {
    /// Starting line (inclusive).
    pub start_line: usize,
    /// Ending line (inclusive).
    pub end_line: usize,
    /// Starting column (inclusive).
    pub start_col: usize,
    /// Ending column (inclusive/exclusive boundary).
    pub end_col: usize,
}

impl BlockRange {
    /// Create a new block selection range. Normalizes lines and columns.
    pub fn new(l1: usize, l2: usize, c1: usize, c2: usize) -> Self {
        Self {
            start_line: min(l1, l2),
            end_line: max(l1, l2),
            start_col: min(c1, c2),
            end_col: max(c1, c2),
        }
    }

    /// Total number of lines in block.
    pub fn line_count(&self) -> usize {
        self.end_line - self.start_line + 1
    }

    /// Column width of block.
    pub fn col_width(&self) -> usize {
        self.end_col - self.start_col
    }

    /// Iterate over line indices covered by this block.
    pub fn lines(&self) -> impl Iterator<Item = usize> {
        self.start_line..=self.end_line
    }
}

// ---------------------------------------------------------------------------
// Core Multiline Character Buffer
// ---------------------------------------------------------------------------

/// Pure Rust Unicode-aware multiline text buffer with fast coordinate conversion.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct MultilineBuffer {
    /// Character storage.
    chars: Vec<char>,
}

impl MultilineBuffer {
    /// Create an empty multiline buffer.
    pub fn new() -> Self {
        Self { chars: Vec::new() }
    }

    /// Create a multiline buffer initialized from string.
    pub fn from_str(s: &str) -> Self {
        Self {
            chars: s.chars().collect(),
        }
    }

    /// Returns full text as a String.
    pub fn text(&self) -> String {
        self.chars.iter().collect()
    }

    /// Replace full buffer contents.
    pub fn set_text(&mut self, s: &str) {
        self.chars = s.chars().collect();
    }

    /// Total character count.
    pub fn len(&self) -> usize {
        self.chars.len()
    }

    /// Returns true if buffer contains zero characters.
    pub fn is_empty(&self) -> bool {
        self.chars.is_empty()
    }

    /// Direct slice access to character array.
    pub fn chars(&self) -> &[char] {
        &self.chars
    }

    /// Get character at index.
    pub fn char_at(&self, offset: usize) -> Option<char> {
        self.chars.get(offset).copied()
    }

    /// Extract a substring slice between character offsets `[start, end)`.
    pub fn slice(&self, start: usize, end: usize) -> String {
        let s = min(start, self.chars.len());
        let e = min(end, self.chars.len());
        if s < e {
            self.chars[s..e].iter().collect()
        } else {
            String::new()
        }
    }

    /// Total number of lines (at least 1, even if empty).
    pub fn line_count(&self) -> usize {
        if self.chars.is_empty() {
            return 1;
        }
        let newlines = self.chars.iter().filter(|&&c| c == '\n').count();
        newlines + 1
    }

    /// Calculate line ranges `Vec<(start_offset, char_length_excluding_newline)>`.
    pub fn line_ranges(&self) -> Vec<(usize, usize)> {
        let mut ranges = Vec::new();
        let mut line_start = 0;

        for (idx, &c) in self.chars.iter().enumerate() {
            if c == '\n' {
                ranges.push((line_start, idx - line_start));
                line_start = idx + 1;
            }
        }
        ranges.push((line_start, self.chars.len() - line_start));
        ranges
    }

    /// Get `(start_offset, char_length)` for a specific line.
    pub fn line_range(&self, line_idx: usize) -> Option<(usize, usize)> {
        let ranges = self.line_ranges();
        ranges.get(line_idx).copied()
    }

    /// Return string content of specific line (excluding trailing newline).
    pub fn line_text(&self, line_idx: usize) -> Option<String> {
        let (start, len) = self.line_range(line_idx)?;
        Some(self.chars[start..start + len].iter().collect())
    }

    /// Return all lines as a vector of Strings.
    pub fn lines(&self) -> Vec<String> {
        self.line_ranges()
            .into_iter()
            .map(|(start, len)| self.chars[start..start + len].iter().collect())
            .collect()
    }

    /// Convert character offset into 2D [`Position`] `(line, col)`.
    pub fn position_of(&self, offset: usize) -> Position {
        let offset = min(offset, self.chars.len());
        let ranges = self.line_ranges();

        for (line_idx, &(start, len)) in ranges.iter().enumerate() {
            if offset >= start && offset <= start + len {
                return Position::new(line_idx, offset - start);
            }
        }
        // Fallback to end of last line
        if let Some(&(_start, len)) = ranges.last() {
            Position::new(ranges.len() - 1, len)
        } else {
            Position::origin()
        }
    }

    /// Convert 2D [`Position`] into linear character offset.
    pub fn offset_of(&self, pos: Position) -> usize {
        let ranges = self.line_ranges();
        if ranges.is_empty() {
            return 0;
        }

        let line_idx = min(pos.line, ranges.len() - 1);
        let (start, len) = ranges[line_idx];
        let col = min(pos.col, len);
        start + col
    }

    /// Clamp a character offset to `[0, buffer.len()]`.
    pub fn clamp_offset(&self, offset: usize) -> usize {
        min(offset, self.chars.len())
    }

    /// Insert a character at offset.
    pub fn insert_char(&mut self, offset: usize, c: char) {
        let off = self.clamp_offset(offset);
        self.chars.insert(off, c);
    }

    /// Insert string at offset.
    pub fn insert_str(&mut self, offset: usize, s: &str) {
        let off = self.clamp_offset(offset);
        let incoming: Vec<char> = s.chars().collect();
        self.chars.splice(off..off, incoming);
    }

    /// Delete range `[start, end)` and return deleted string.
    pub fn delete_range(&mut self, start: usize, end: usize) -> String {
        let s = min(start, self.chars.len());
        let e = min(end, self.chars.len());
        if s < e {
            let drained: String = self.chars.drain(s..e).collect();
            drained
        } else {
            String::new()
        }
    }

    /// Replace range `[start, end)` with new string.
    pub fn replace_range(&mut self, start: usize, end: usize, s: &str) {
        let s_idx = min(start, self.chars.len());
        let e_idx = min(end, self.chars.len());
        let incoming: Vec<char> = s.chars().collect();
        self.chars.splice(s_idx..e_idx, incoming);
    }

    /// Clear all buffer content.
    pub fn clear(&mut self) {
        self.chars.clear();
    }
}

// ---------------------------------------------------------------------------
// Multi-Cursor State Manager
// ---------------------------------------------------------------------------

/// Manages multiple cursors and selections with automatic sorting and deduplication.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MultiCursorState {
    /// List of active cursors, maintained sorted by offset.
    cursors: Vec<Cursor>,
    /// Index of the primary (active) cursor.
    primary_idx: usize,
}

impl Default for MultiCursorState {
    fn default() -> Self {
        Self::new()
    }
}

impl MultiCursorState {
    /// Create state with a single cursor at offset 0.
    pub fn new() -> Self {
        Self {
            cursors: vec![Cursor::new(0)],
            primary_idx: 0,
        }
    }

    /// Create state with a single cursor at specified offset.
    pub fn with_offset(offset: usize) -> Self {
        Self {
            cursors: vec![Cursor::new(offset)],
            primary_idx: 0,
        }
    }

    /// Total number of active cursors.
    pub fn count(&self) -> usize {
        self.cursors.len()
    }

    /// Returns true if multiple cursors are active.
    pub fn is_multi(&self) -> bool {
        self.cursors.len() > 1
    }

    /// Slice of all cursors.
    pub fn cursors(&self) -> &[Cursor] {
        &self.cursors
    }

    /// Mutable slice of all cursors.
    pub fn cursors_mut(&mut self) -> &mut [Cursor] {
        &mut self.cursors
    }

    /// Reference to primary cursor.
    pub fn primary(&self) -> &Cursor {
        &self.cursors[self.primary_idx]
    }

    /// Mutable reference to primary cursor.
    pub fn primary_mut(&mut self) -> &mut Cursor {
        &mut self.cursors[self.primary_idx]
    }

    /// Primary cursor index.
    pub fn primary_index(&self) -> usize {
        self.primary_idx
    }

    /// Add a new cursor. Automatically merges overlapping cursors.
    pub fn add_cursor(&mut self, cursor: Cursor) {
        self.cursors.push(cursor);
        self.normalize();
    }

    /// Reset to a single primary cursor at given offset.
    pub fn reset_to_single(&mut self, offset: usize) {
        self.cursors = vec![Cursor::new(offset)];
        self.primary_idx = 0;
    }

    /// Clear all secondary cursors, keeping only the primary one.
    pub fn clear_secondary(&mut self) {
        if self.cursors.len() > 1 {
            let primary = self.cursors.remove(self.primary_idx);
            self.cursors = vec![primary];
            self.primary_idx = 0;
        }
    }

    /// Sort cursors by selection start and merge overlapping selections.
    pub fn normalize(&mut self) {
        if self.cursors.is_empty() {
            self.cursors.push(Cursor::new(0));
            self.primary_idx = 0;
            return;
        }

        // Retain primary identity
        let primary_offset = self
            .cursors
            .get(self.primary_idx)
            .map(|c| c.offset)
            .unwrap_or(0);

        // Sort by start position
        self.cursors.sort_by(|a, b| {
            let a_start = a.selection_start();
            let b_start = b.selection_start();
            a_start.cmp(&b_start).then_with(|| a.offset.cmp(&b.offset))
        });

        // Merge overlapping / identical cursors
        let mut merged: Vec<Cursor> = Vec::with_capacity(self.cursors.len());
        for cursor in self.cursors.drain(..) {
            if let Some(last) = merged.last_mut() {
                let last_sel = last.selection();
                let cur_sel = cursor.selection();

                // Merge if selections overlap or both are collapsed at same offset
                let should_merge = if last_sel.is_collapsed() && cur_sel.is_collapsed() {
                    last.offset == cursor.offset
                } else {
                    last_sel.touches_or_overlaps(&cur_sel)
                };

                if should_merge {
                    let new_start = min(last_sel.start(), cur_sel.start());
                    let new_end = max(last_sel.end(), cur_sel.end());
                    if new_start == new_end {
                        *last = Cursor::new(new_start);
                    } else {
                        *last = Cursor::with_selection(new_start, new_end);
                    }
                    continue;
                }
            }
            merged.push(cursor);
        }

        self.cursors = merged;

        // Restore closest primary index
        self.primary_idx = 0;
        let mut min_diff = usize::MAX;
        for (idx, c) in self.cursors.iter().enumerate() {
            let diff = (c.offset as isize - primary_offset as isize).unsigned_abs();
            if diff < min_diff {
                min_diff = diff;
                self.primary_idx = idx;
            }
        }
    }

    /// Add a cursor on the line immediately above the primary cursor.
    pub fn add_cursor_above(&mut self, buffer: &MultilineBuffer) {
        let primary = self.primary();
        let pos = buffer.position_of(primary.offset);
        if pos.line > 0 {
            let col = primary.desired_col.unwrap_or(pos.col);
            let target_pos = Position::new(pos.line - 1, col);
            let target_off = buffer.offset_of(target_pos);
            let mut new_cursor = Cursor::new(target_off);
            new_cursor.desired_col = Some(col);
            self.add_cursor(new_cursor);
        }
    }

    /// Add a cursor on the line immediately below the primary cursor.
    pub fn add_cursor_below(&mut self, buffer: &MultilineBuffer) {
        let primary = self.primary();
        let pos = buffer.position_of(primary.offset);
        if pos.line + 1 < buffer.line_count() {
            let col = primary.desired_col.unwrap_or(pos.col);
            let target_pos = Position::new(pos.line + 1, col);
            let target_off = buffer.offset_of(target_pos);
            let mut new_cursor = Cursor::new(target_off);
            new_cursor.desired_col = Some(col);
            self.add_cursor(new_cursor);
        }
    }

    /// Find next occurrence of current selected text or word under cursor and add cursor.
    pub fn add_cursor_at_next_match(
        &mut self,
        buffer: &MultilineBuffer,
        case_sensitive: bool,
    ) -> bool {
        let pattern = if self.primary().has_selection() {
            let sel = self.primary().selection();
            buffer.slice(sel.start(), sel.end())
        } else {
            // Find word under primary cursor
            if let Some((start, end)) =
                LineJumpHelper::find_word_range_at(buffer.chars(), self.primary().offset)
            {
                // Select word first if not selected
                let prim = self.primary_mut();
                *prim = Cursor::with_selection(start, end);
                return true;
            }
            return false;
        };

        if pattern.is_empty() {
            return false;
        }

        let full_text = buffer.text();
        let search_start = self
            .cursors
            .iter()
            .map(|c| c.selection_end())
            .max()
            .unwrap_or(0);

        let match_offset = if case_sensitive {
            full_text[search_start..]
                .find(&pattern)
                .map(|idx| search_start + idx)
                .or_else(|| full_text.find(&pattern))
        } else {
            let lower_pattern = pattern.to_lowercase();
            let lower_full = full_text.to_lowercase();
            lower_full[search_start..]
                .find(&lower_pattern)
                .map(|idx| search_start + idx)
                .or_else(|| lower_full.find(&lower_pattern))
        };

        if let Some(byte_off) = match_offset {
            let char_off = full_text[..byte_off].chars().count();
            let pattern_len = pattern.chars().count();
            let new_cursor = Cursor::with_selection(char_off, char_off + pattern_len);
            self.add_cursor(new_cursor);
            true
        } else {
            false
        }
    }

    /// Select all occurrences of current selection or word across entire buffer.
    pub fn select_all_matches(&mut self, buffer: &MultilineBuffer, case_sensitive: bool) -> usize {
        let pattern = if self.primary().has_selection() {
            let sel = self.primary().selection();
            buffer.slice(sel.start(), sel.end())
        } else if let Some((start, end)) =
            LineJumpHelper::find_word_range_at(buffer.chars(), self.primary().offset)
        {
            buffer.slice(start, end)
        } else {
            return 0;
        };

        if pattern.is_empty() {
            return 0;
        }

        let full_text = buffer.text();
        let pattern_len = pattern.chars().count();
        let mut new_cursors = Vec::new();

        let pat_to_find = if case_sensitive {
            pattern.clone()
        } else {
            pattern.to_lowercase()
        };
        let haystack = if case_sensitive {
            full_text.clone()
        } else {
            full_text.to_lowercase()
        };

        let mut search_idx = 0;
        while let Some(found_idx) = haystack[search_idx..].find(&pat_to_find) {
            let abs_byte_idx = search_idx + found_idx;
            let char_off = full_text[..abs_byte_idx].chars().count();
            new_cursors.push(Cursor::with_selection(char_off, char_off + pattern_len));
            search_idx = abs_byte_idx + pat_to_find.len();
            if search_idx >= haystack.len() {
                break;
            }
        }

        let count = new_cursors.len();
        if !new_cursors.is_empty() {
            self.cursors = new_cursors;
            self.primary_idx = 0;
            self.normalize();
        }
        count
    }

    /// Split a multiline selection into individual cursors on each line.
    pub fn split_selection_into_lines(&mut self, buffer: &MultilineBuffer) {
        let mut result = Vec::new();

        for cursor in &self.cursors {
            if !cursor.has_selection() {
                result.push(cursor.clone());
                continue;
            }

            let sel = cursor.selection();
            let start_pos = buffer.position_of(sel.start());
            let end_pos = buffer.position_of(sel.end());

            if start_pos.line == end_pos.line {
                result.push(cursor.clone());
                continue;
            }

            let ranges = buffer.line_ranges();
            for line_idx in start_pos.line..=end_pos.line {
                if line_idx >= ranges.len() {
                    break;
                }
                let (line_start, line_len) = ranges[line_idx];
                let cur_start = if line_idx == start_pos.line {
                    line_start + start_pos.col
                } else {
                    line_start
                };
                let cur_end = if line_idx == end_pos.line {
                    line_start + end_pos.col
                } else {
                    line_start + line_len
                };

                if cur_start <= cur_end {
                    result.push(Cursor::with_selection(cur_start, cur_end));
                }
            }
        }

        if !result.is_empty() {
            self.cursors = result;
            self.normalize();
        }
    }
}

// ---------------------------------------------------------------------------
// Word Wrapping & Visual Layout Engine
// ---------------------------------------------------------------------------

/// Word wrapping options and visual formatting configuration.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WrapOptions {
    /// Maximum line column width before wrapping.
    pub width: usize,
    /// Number of spaces for a tab character.
    pub tab_width: usize,
    /// Whether to break long words that exceed column width.
    pub break_words: bool,
    /// Subsequent line indentation width.
    pub subsequent_indent: usize,
    /// Prefix prepended to wrapped lines (e.g. `> ` or `// `).
    pub line_prefix: Option<String>,
}

impl Default for WrapOptions {
    fn default() -> Self {
        Self {
            width: 80,
            tab_width: 4,
            break_words: true,
            subsequent_indent: 0,
            line_prefix: None,
        }
    }
}

impl WrapOptions {
    /// Create new wrapping options with column width.
    pub fn new(width: usize) -> Self {
        Self {
            width,
            ..Default::default()
        }
    }

    /// Set break words flag.
    pub fn with_break_words(mut self, break_words: bool) -> Self {
        self.break_words = break_words;
        self
    }

    /// Set subsequent line indent width.
    pub fn with_subsequent_indent(mut self, indent: usize) -> Self {
        self.subsequent_indent = indent;
        self
    }

    /// Set line prefix.
    pub fn with_prefix(mut self, prefix: impl Into<String>) -> Self {
        self.line_prefix = Some(prefix.into());
        self
    }
}

/// A visual wrapped line mapping back to original buffer character offsets.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WrappedLine {
    /// Index of original line in buffer (0-indexed).
    pub original_line: usize,
    /// Subline index within original line (0 for first visual line).
    pub subline_idx: usize,
    /// Start character offset in buffer (inclusive).
    pub start_offset: usize,
    /// End character offset in buffer (exclusive).
    pub end_offset: usize,
    /// Rendered text content of this visual line.
    pub text: String,
}

/// Word wrapping engine providing hard wraps, soft wraps, and visual coordinate conversions.
pub struct WordWrapEngine;

impl WordWrapEngine {
    /// Wrap a single line of text according to [`WrapOptions`].
    pub fn wrap_line(line: &str, opts: &WrapOptions) -> Vec<String> {
        if line.is_empty() {
            return vec![String::new()];
        }

        let max_width = max(opts.width, 1);
        let mut result = Vec::new();
        let mut current_line = String::new();
        let mut current_width = 0;

        let words: Vec<&str> = line.split(' ').collect();
        for (i, word) in words.iter().enumerate() {
            let is_first = i == 0;
            let word_len = word.chars().count();
            let space_len = if is_first { 0 } else { 1 };

            if !is_first && current_width + space_len + word_len > max_width {
                if !current_line.is_empty() {
                    result.push(current_line);
                    current_line = String::new();
                    current_width = 0;
                }

                if opts.break_words && word_len > max_width {
                    let mut chunk = String::new();
                    for c in word.chars() {
                        if chunk.chars().count() >= max_width {
                            result.push(chunk);
                            chunk = String::new();
                        }
                        chunk.push(c);
                    }
                    if !chunk.is_empty() {
                        current_width = chunk.chars().count();
                        current_line = chunk;
                    }
                    continue;
                }
            }

            if !is_first && !current_line.is_empty() {
                current_line.push(' ');
                current_width += 1;
            }

            current_line.push_str(word);
            current_width += word_len;
        }

        if !current_line.is_empty() || result.is_empty() {
            result.push(current_line);
        }

        result
    }

    /// Compute visual wrapped lines for entire buffer with exact character offset mappings.
    pub fn wrap_buffer(buffer: &MultilineBuffer, opts: &WrapOptions) -> Vec<WrappedLine> {
        let mut wrapped = Vec::new();
        let ranges = buffer.line_ranges();

        for (line_idx, &(start_off, len)) in ranges.iter().enumerate() {
            let line_chars = &buffer.chars()[start_off..start_off + len];
            let line_str: String = line_chars.iter().collect();

            if line_str.is_empty() {
                wrapped.push(WrappedLine {
                    original_line: line_idx,
                    subline_idx: 0,
                    start_offset: start_off,
                    end_offset: start_off,
                    text: String::new(),
                });
                continue;
            }

            let sublines = Self::wrap_line(&line_str, opts);
            let mut current_sub_start = start_off;

            for (sub_idx, subline_text) in sublines.into_iter().enumerate() {
                let sub_char_count = subline_text.chars().count();
                let sub_end = min(current_sub_start + sub_char_count, start_off + len);

                wrapped.push(WrappedLine {
                    original_line: line_idx,
                    subline_idx: sub_idx,
                    start_offset: current_sub_start,
                    end_offset: sub_end,
                    text: subline_text,
                });

                current_sub_start = min(sub_end + 1, start_off + len);
            }
        }

        wrapped
    }

    /// Convert visual line/col coordinate to buffer character offset.
    pub fn visual_to_buffer_offset(
        visual_line: usize,
        visual_col: usize,
        wrapped: &[WrappedLine],
    ) -> usize {
        if wrapped.is_empty() {
            return 0;
        }
        let line_idx = min(visual_line, wrapped.len() - 1);
        let wl = &wrapped[line_idx];
        let max_col = wl.end_offset.saturating_sub(wl.start_offset);
        let col = min(visual_col, max_col);
        wl.start_offset + col
    }

    /// Convert linear character offset to 2D visual coordinate `(visual_line, visual_col)`.
    pub fn buffer_to_visual_pos(offset: usize, wrapped: &[WrappedLine]) -> Position {
        if wrapped.is_empty() {
            return Position::origin();
        }

        for (v_line, wl) in wrapped.iter().enumerate() {
            if offset >= wl.start_offset && offset <= wl.end_offset {
                return Position::new(v_line, offset - wl.start_offset);
            }
        }

        if let Some(last) = wrapped.last() {
            Position::new(
                wrapped.len() - 1,
                last.end_offset.saturating_sub(last.start_offset),
            )
        } else {
            Position::origin()
        }
    }

    /// Reflow and wrap a paragraph while preserving common comment or markdown blockquote prefixes.
    pub fn rewrap_paragraph(text: &str, width: usize) -> String {
        let lines: Vec<&str> = text.lines().collect();
        if lines.is_empty() {
            return String::new();
        }

        let prefix = detect_common_prefix(&lines);
        let prefix_len = prefix.chars().count();
        let target_width = width.saturating_sub(prefix_len);

        let mut words = Vec::new();
        for line in &lines {
            let content = if line.starts_with(&prefix) {
                &line[prefix.len()..]
            } else {
                line.trim_start()
            };
            for word in content.split_whitespace() {
                words.push(word);
            }
        }

        if words.is_empty() {
            return prefix;
        }

        let joined = words.join(" ");
        let opts = WrapOptions::new(target_width);
        let wrapped_sublines = Self::wrap_line(&joined, &opts);

        let formatted: Vec<String> = wrapped_sublines
            .into_iter()
            .map(|l| format!("{}{}", prefix, l))
            .collect();

        formatted.join("\n")
    }
}

/// Helper to detect comment or blockquote prefix from lines.
fn detect_common_prefix(lines: &[&str]) -> String {
    for prefix in &["// ", "/// ", "# ", "> ", "* ", "- "] {
        if lines
            .iter()
            .all(|l| l.trim().is_empty() || l.starts_with(prefix))
        {
            return (*prefix).to_string();
        }
    }
    String::new()
}

// ---------------------------------------------------------------------------
// Horizontal Viewport Windowing & Scrolling
// ---------------------------------------------------------------------------

/// Manages horizontal viewport windowing for long lines when soft wrapping is not active.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct HorizontalScrollState {
    /// Width of the visible viewport in terminal columns.
    pub viewport_width: usize,
    /// Current horizontal scroll offset in columns (0-indexed).
    pub scroll_offset: usize,
    /// Edge margin before triggering auto-scroll (e.g. 2 columns from edge).
    pub margin: usize,
}

impl Default for HorizontalScrollState {
    fn default() -> Self {
        Self::new(80, 2)
    }
}

impl HorizontalScrollState {
    /// Create a new horizontal scroll manager.
    pub fn new(viewport_width: usize, margin: usize) -> Self {
        let width = max(viewport_width, 1);
        let safe_margin = min(margin, width / 2);
        Self {
            viewport_width: width,
            scroll_offset: 0,
            margin: safe_margin,
        }
    }

    /// Adjust horizontal scroll so that `cursor_col` is comfortably visible within viewport.
    pub fn ensure_visible(&mut self, cursor_col: usize) {
        if self.viewport_width == 0 {
            return;
        }

        // Left boundary constraint
        if cursor_col < self.scroll_offset + self.margin {
            self.scroll_offset = cursor_col.saturating_sub(self.margin);
        }

        // Right boundary constraint
        let visible_right = self.scroll_offset + self.viewport_width;
        if cursor_col + self.margin >= visible_right {
            self.scroll_offset = (cursor_col + self.margin + 1).saturating_sub(self.viewport_width);
        }
    }

    /// Scroll horizontally to the left by `delta` columns.
    pub fn scroll_left(&mut self, delta: usize) {
        self.scroll_offset = self.scroll_offset.saturating_sub(delta);
    }

    /// Scroll horizontally to the right by `delta` columns.
    pub fn scroll_right(&mut self, delta: usize) {
        self.scroll_offset += delta;
    }

    /// Set absolute scroll offset.
    pub fn scroll_to(&mut self, offset: usize) {
        self.scroll_offset = offset;
    }

    /// Extract visible character slice of a single line according to current scroll offset and viewport width.
    pub fn visible_slice(&self, line: &str) -> String {
        let chars: Vec<char> = line.chars().collect();
        if self.scroll_offset >= chars.len() {
            return String::new();
        }
        let end = min(self.scroll_offset + self.viewport_width, chars.len());
        chars[self.scroll_offset..end].iter().collect()
    }

    /// Compute 0-indexed screen column corresponding to buffer column if visible.
    pub fn cursor_screen_col(&self, cursor_col: usize) -> Option<usize> {
        if cursor_col >= self.scroll_offset && cursor_col < self.scroll_offset + self.viewport_width
        {
            Some(cursor_col - self.scroll_offset)
        } else {
            None
        }
    }

    /// Render visible line with optional left/right overflow indicators (e.g. `«`, `»` or `…`).
    pub fn render_with_indicators(
        &self,
        line: &str,
        left_indicator: &str,
        right_indicator: &str,
    ) -> String {
        let chars: Vec<char> = line.chars().collect();
        let total_chars = chars.len();

        let has_left_overflow = self.scroll_offset > 0;
        let has_right_overflow = self.scroll_offset + self.viewport_width < total_chars;

        let left_ind_len = left_indicator.chars().count();
        let right_ind_len = right_indicator.chars().count();

        let mut available_width = self.viewport_width;
        if has_left_overflow {
            available_width = available_width.saturating_sub(left_ind_len);
        }
        if has_right_overflow {
            available_width = available_width.saturating_sub(right_ind_len);
        }

        let start = self.scroll_offset;
        let end = min(start + available_width, total_chars);

        let slice: String = if start < total_chars {
            chars[start..end].iter().collect()
        } else {
            String::new()
        };

        let mut result = String::new();
        if has_left_overflow {
            result.push_str(left_indicator);
        }
        result.push_str(&slice);
        if has_right_overflow {
            result.push_str(right_indicator);
        }

        result
    }

    /// Render line with scroll offset and return (rendered_text, screen_cursor_col).
    pub fn render_line_with_scroll(
        &self,
        line: &str,
        cursor_col: Option<usize>,
    ) -> (String, Option<usize>) {
        let visible = self.visible_slice(line);
        let screen_col = cursor_col.and_then(|col| self.cursor_screen_col(col));
        (visible, screen_col)
    }
}

// ---------------------------------------------------------------------------
// Readline Kill Ring
// ---------------------------------------------------------------------------

/// Ring buffer storing killed text spans for Readline cut/copy/yank operations.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct KillRing {
    /// Ring entries, where index 0 is the most recently killed text.
    entries: Vec<String>,
    /// Current yank index for `yank_pop` cycling.
    yank_idx: usize,
    /// Maximum ring capacity.
    capacity: usize,
}

impl Default for KillRing {
    fn default() -> Self {
        Self::new(64)
    }
}

impl KillRing {
    /// Create a new kill ring with capacity limit.
    pub fn new(capacity: usize) -> Self {
        Self {
            entries: Vec::new(),
            yank_idx: 0,
            capacity: max(capacity, 1),
        }
    }

    /// Push new killed text to the head of the ring.
    pub fn push(&mut self, text: &str) {
        if text.is_empty() {
            return;
        }
        self.entries.insert(0, text.to_string());
        if self.entries.len() > self.capacity {
            self.entries.truncate(self.capacity);
        }
        self.yank_idx = 0;
    }

    /// Append (or prepend) text to the current head entry (for consecutive kill operations).
    pub fn push_append(&mut self, text: &str, prepend: bool) {
        if text.is_empty() {
            return;
        }
        if self.entries.is_empty() {
            self.push(text);
        } else if prepend {
            let existing = &self.entries[0];
            self.entries[0] = format!("{}{}", text, existing);
        } else {
            self.entries[0].push_str(text);
        }
        self.yank_idx = 0;
    }

    /// Retrieve the most recently killed text (or current yank-pop position).
    pub fn yank(&self) -> Option<&str> {
        self.entries.get(self.yank_idx).map(|s| s.as_str())
    }

    /// Cycle to previous kill ring entry (`Alt+Y`) and return its content.
    pub fn yank_pop(&mut self) -> Option<&str> {
        if self.entries.is_empty() {
            return None;
        }
        self.yank_idx = (self.yank_idx + 1) % self.entries.len();
        self.yank()
    }

    /// Reset yank index back to the head of the ring.
    pub fn reset_yank_index(&mut self) {
        self.yank_idx = 0;
    }

    /// Returns true if kill ring has no entries.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Number of entries stored.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Direct slice to all stored kill ring entries.
    pub fn entries(&self) -> &[String] {
        &self.entries
    }

    /// Clear all kill ring entries.
    pub fn clear(&mut self) {
        self.entries.clear();
        self.yank_idx = 0;
    }
}

// ---------------------------------------------------------------------------
// Readline Keys, Commands, and Actions
// ---------------------------------------------------------------------------

/// Readline key event representation for input dispatch.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ReadlineKey {
    /// Regular printable character.
    Char(char),
    /// Control + character (e.g. `Ctrl('a')` for `Ctrl+A`).
    Ctrl(char),
    /// Alt / Meta + character (e.g. `Alt('b')` for `Alt+B`).
    Alt(char),
    /// Return / Enter.
    Enter,
    /// Shift + Enter.
    ShiftEnter,
    /// Backspace / Delete character backward.
    Backspace,
    /// Delete character forward.
    Delete,
    /// Tab key.
    Tab,
    /// Shift + Tab key.
    ShiftTab,
    /// Left arrow.
    Left,
    /// Right arrow.
    Right,
    /// Up arrow.
    Up,
    /// Down arrow.
    Down,
    /// Home key.
    Home,
    /// End key.
    End,
    /// Page Up key.
    PageUp,
    /// Page Down key.
    PageDown,
    /// Shift + Left arrow (extend selection left).
    ShiftLeft,
    /// Shift + Right arrow (extend selection right).
    ShiftRight,
    /// Shift + Up arrow (extend selection up).
    ShiftUp,
    /// Shift + Down arrow (extend selection down).
    ShiftDown,
    /// Ctrl + Left arrow (word left).
    CtrlLeft,
    /// Ctrl + Right arrow (word right).
    CtrlRight,
    /// Ctrl + Up arrow.
    CtrlUp,
    /// Ctrl + Down arrow.
    CtrlDown,
    /// Escape key.
    Escape,
    /// Paste raw text.
    Paste(String),
}

/// Readline and multi-cursor editing command.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ReadlineCommand {
    /// Insert a character at all cursors.
    InsertChar(char),
    /// Insert a string slice at all cursors.
    InsertStr(String),
    /// Insert a newline (with optional auto-indent).
    InsertNewline { auto_indent: bool },
    /// Backspace (delete previous char).
    Backspace,
    /// Delete (delete forward char).
    Delete,
    /// Move cursor left one character.
    MoveCharLeft { extend_selection: bool },
    /// Move cursor right one character.
    MoveCharRight { extend_selection: bool },
    /// Move cursor backward one word (`Alt+B` / `Ctrl+Left`).
    MoveWordLeft { extend_selection: bool },
    /// Move cursor forward one word (`Alt+F` / `Ctrl+Right`).
    MoveWordRight { extend_selection: bool },
    /// Move to beginning of current line (`Ctrl+A` / `Home`).
    MoveLineStart { extend_selection: bool },
    /// Move to end of current line (`Ctrl+E` / `End`).
    MoveLineEnd { extend_selection: bool },
    /// Move cursor up one line (`Ctrl+P` / `Up`).
    MoveLineUp { extend_selection: bool },
    /// Move cursor down one line (`Ctrl+N` / `Down`).
    MoveLineDown { extend_selection: bool },
    /// Move to beginning of buffer.
    MoveBufferStart { extend_selection: bool },
    /// Move to end of buffer.
    MoveBufferEnd { extend_selection: bool },
    /// Kill text from cursor to end of line (`Ctrl+K`).
    KillLineToEnd,
    /// Kill text from cursor to start of line (`Ctrl+U`).
    KillLineToStart,
    /// Kill word backward from cursor (`Ctrl+W`).
    KillWordBackward,
    /// Kill word forward from cursor (`Alt+D`).
    KillWordForward,
    /// Yank / paste most recently killed text (`Ctrl+Y`).
    Yank,
    /// Cycle to previous kill ring item (`Alt+Y`).
    YankPop,
    /// Transpose characters around cursor (`Ctrl+T`).
    TransposeChars,
    /// Transpose words around cursor (`Alt+T`).
    TransposeWords,
    /// Transform word case (`Alt+U`, `Alt+L`, `Alt+C`).
    TransformWord(CaseTransform),
    /// Undo last edit (`Ctrl+_` / `Ctrl+Z`).
    Undo,
    /// Redo last undone edit (`Ctrl+Shift+Z` / `Ctrl+R` / `Alt+U`).
    Redo,
    /// Add cursor on line above (`Alt+Up` / `Ctrl+Up`).
    AddCursorAbove,
    /// Add cursor on line below (`Alt+Down` / `Ctrl+Down`).
    AddCursorBelow,
    /// Add cursor at next occurrence of selection / word (`Ctrl+D` / `Alt+N`).
    AddCursorNextMatch { case_sensitive: bool },
    /// Select all occurrences across buffer (`Ctrl+Shift+L` / `Alt+A`).
    SelectAllMatches { case_sensitive: bool },
    /// Split multiline selection into line-by-line cursors.
    SplitSelectionLines,
    /// Clear all secondary cursors, collapsing to primary (`Escape`).
    ClearSecondaryCursors,
    /// Select entire buffer content (`Ctrl+A` with select all).
    SelectAll,
    /// Paste text (multi-line / multi-cursor aware).
    Paste(String),
    /// Re-wrap buffer to column width.
    Rewrap(usize),
    /// No operation.
    NoOp,
}

/// Outcome of processing a Readline key event.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ReadlineActionOutcome {
    /// Buffer text was modified.
    Modified,
    /// Cursor was moved or selection changed without text modification.
    Moved,
    /// Input submitted (Enter).
    Submitted,
    /// Input canceled (Ctrl+C / Escape when no secondary cursors).
    Canceled,
    /// Command performed with no visual change.
    NoOp,
}

impl ReadlineKey {
    /// Map standard Readline key to corresponding [`ReadlineCommand`].
    pub fn to_command(&self) -> ReadlineCommand {
        match self {
            ReadlineKey::Char(c) => ReadlineCommand::InsertChar(*c),
            ReadlineKey::Ctrl('a') => ReadlineCommand::MoveLineStart {
                extend_selection: false,
            },
            ReadlineKey::Ctrl('e') => ReadlineCommand::MoveLineEnd {
                extend_selection: false,
            },
            ReadlineKey::Ctrl('k') => ReadlineCommand::KillLineToEnd,
            ReadlineKey::Ctrl('u') => ReadlineCommand::KillLineToStart,
            ReadlineKey::Ctrl('w') => ReadlineCommand::KillWordBackward,
            ReadlineKey::Ctrl('y') => ReadlineCommand::Yank,
            ReadlineKey::Ctrl('b') => ReadlineCommand::MoveCharLeft {
                extend_selection: false,
            },
            ReadlineKey::Ctrl('f') => ReadlineCommand::MoveCharRight {
                extend_selection: false,
            },
            ReadlineKey::Ctrl('p') => ReadlineCommand::MoveLineUp {
                extend_selection: false,
            },
            ReadlineKey::Ctrl('n') => ReadlineCommand::MoveLineDown {
                extend_selection: false,
            },
            ReadlineKey::Ctrl('h') => ReadlineCommand::Backspace,
            ReadlineKey::Ctrl('d') => ReadlineCommand::Delete,
            ReadlineKey::Ctrl('t') => ReadlineCommand::TransposeChars,
            ReadlineKey::Ctrl('_') | ReadlineKey::Ctrl('z') => ReadlineCommand::Undo,
            ReadlineKey::Alt('b') => ReadlineCommand::MoveWordLeft {
                extend_selection: false,
            },
            ReadlineKey::Alt('f') => ReadlineCommand::MoveWordRight {
                extend_selection: false,
            },
            ReadlineKey::Alt('d') => ReadlineCommand::KillWordForward,
            ReadlineKey::Alt('y') => ReadlineCommand::YankPop,
            ReadlineKey::Alt('t') => ReadlineCommand::TransposeWords,
            ReadlineKey::Alt('u') => ReadlineCommand::TransformWord(CaseTransform::Uppercase),
            ReadlineKey::Alt('l') => ReadlineCommand::TransformWord(CaseTransform::Lowercase),
            ReadlineKey::Alt('c') => ReadlineCommand::TransformWord(CaseTransform::Titlecase),
            ReadlineKey::Alt('n') => ReadlineCommand::AddCursorNextMatch {
                case_sensitive: true,
            },
            ReadlineKey::Alt('a') => ReadlineCommand::SelectAllMatches {
                case_sensitive: true,
            },
            ReadlineKey::Enter => ReadlineCommand::InsertNewline { auto_indent: false },
            ReadlineKey::ShiftEnter => ReadlineCommand::InsertNewline { auto_indent: true },
            ReadlineKey::Backspace => ReadlineCommand::Backspace,
            ReadlineKey::Delete => ReadlineCommand::Delete,
            ReadlineKey::Tab => ReadlineCommand::InsertStr("    ".to_string()),
            ReadlineKey::Left => ReadlineCommand::MoveCharLeft {
                extend_selection: false,
            },
            ReadlineKey::Right => ReadlineCommand::MoveCharRight {
                extend_selection: false,
            },
            ReadlineKey::Up => ReadlineCommand::MoveLineUp {
                extend_selection: false,
            },
            ReadlineKey::Down => ReadlineCommand::MoveLineDown {
                extend_selection: false,
            },
            ReadlineKey::Home => ReadlineCommand::MoveLineStart {
                extend_selection: false,
            },
            ReadlineKey::End => ReadlineCommand::MoveLineEnd {
                extend_selection: false,
            },
            ReadlineKey::PageUp => ReadlineCommand::MoveBufferStart {
                extend_selection: false,
            },
            ReadlineKey::PageDown => ReadlineCommand::MoveBufferEnd {
                extend_selection: false,
            },
            ReadlineKey::ShiftLeft => ReadlineCommand::MoveCharLeft {
                extend_selection: true,
            },
            ReadlineKey::ShiftRight => ReadlineCommand::MoveCharRight {
                extend_selection: true,
            },
            ReadlineKey::ShiftUp => ReadlineCommand::MoveLineUp {
                extend_selection: true,
            },
            ReadlineKey::ShiftDown => ReadlineCommand::MoveLineDown {
                extend_selection: true,
            },
            ReadlineKey::CtrlLeft => ReadlineCommand::MoveWordLeft {
                extend_selection: false,
            },
            ReadlineKey::CtrlRight => ReadlineCommand::MoveWordRight {
                extend_selection: false,
            },
            ReadlineKey::CtrlUp => ReadlineCommand::AddCursorAbove,
            ReadlineKey::CtrlDown => ReadlineCommand::AddCursorBelow,
            ReadlineKey::Escape => ReadlineCommand::ClearSecondaryCursors,
            ReadlineKey::Paste(s) => ReadlineCommand::Paste(s.clone()),
            _ => ReadlineCommand::NoOp,
        }
    }
}

// ---------------------------------------------------------------------------
// Line Jumping & Navigation Helpers
// ---------------------------------------------------------------------------

/// Destination target for jumping across buffer lines.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum LineJumpTarget {
    /// Jump to the first line (line 0).
    FirstLine,
    /// Jump to the last line.
    LastLine,
    /// Jump to specific line index (0-indexed).
    Line(usize),
    /// Jump relative line delta (e.g. -5 for 5 lines up).
    Delta(isize),
    /// Jump to buffer percentage (0.0 to 1.0).
    Percentage(f32),
    /// Jump to start of current line (BOL).
    BeginningOfLine,
    /// Jump to end of current line (EOL).
    EndOfLine,
    /// Jump to first non-whitespace character on current line.
    FirstNonWhitespace,
}

/// Navigation and search helper for jumping between words, brackets, headings, and code blocks.
pub struct LineJumpHelper;

impl LineJumpHelper {
    /// Calculate new character offset for a line jump target.
    pub fn calculate_jump(
        buffer: &MultilineBuffer,
        current_offset: usize,
        target: LineJumpTarget,
        preserve_col: bool,
    ) -> usize {
        let cur_pos = buffer.position_of(current_offset);
        let ranges = buffer.line_ranges();
        let line_count = ranges.len();

        let target_line = match target {
            LineJumpTarget::FirstLine => 0,
            LineJumpTarget::LastLine => line_count.saturating_sub(1),
            LineJumpTarget::Line(l) => min(l, line_count.saturating_sub(1)),
            LineJumpTarget::Delta(d) => {
                let new_l = cur_pos.line as isize + d;
                new_l.clamp(0, line_count.saturating_sub(1) as isize) as usize
            }
            LineJumpTarget::Percentage(pct) => {
                let clamped = pct.clamp(0.0, 1.0);
                ((line_count.saturating_sub(1) as f32) * clamped).round() as usize
            }
            LineJumpTarget::BeginningOfLine => {
                let (start, _) = ranges[cur_pos.line];
                return start;
            }
            LineJumpTarget::EndOfLine => {
                let (start, len) = ranges[cur_pos.line];
                return start + len;
            }
            LineJumpTarget::FirstNonWhitespace => {
                let (start, len) = ranges[cur_pos.line];
                let mut idx = start;
                while idx < start + len && buffer.chars()[idx].is_whitespace() {
                    idx += 1;
                }
                return idx;
            }
        };

        let col = if preserve_col { cur_pos.col } else { 0 };
        buffer.offset_of(Position::new(target_line, col))
    }

    /// Find matching bracket offset for `(`, `)`, `[`, `]`, `{`, `}`, `<`, `>`.
    pub fn find_matching_bracket(chars: &[char], offset: usize) -> Option<usize> {
        let ch = chars.get(offset)?;
        let (open, close, forward) = match ch {
            '(' => ('(', ')', true),
            ')' => ('(', ')', false),
            '[' => ('[', ']', true),
            ']' => ('[', ']', false),
            '{' => ('{', '}', true),
            '}' => ('{', '}', false),
            '<' => ('<', '>', true),
            '>' => ('<', '>', false),
            _ => return None,
        };

        let mut depth = 0;
        if forward {
            for (idx, &c) in chars.iter().enumerate().skip(offset) {
                if c == open {
                    depth += 1;
                } else if c == close {
                    depth -= 1;
                    if depth == 0 {
                        return Some(idx);
                    }
                }
            }
        } else {
            for idx in (0..=offset).rev() {
                let c = chars[idx];
                if c == close {
                    depth += 1;
                } else if c == open {
                    depth -= 1;
                    if depth == 0 {
                        return Some(idx);
                    }
                }
            }
        }

        None
    }

    /// Find previous word start offset before `offset`. Supports camelCase and snake_case subwords.
    pub fn find_prev_word_boundary(chars: &[char], offset: usize, subword: bool) -> usize {
        if offset == 0 || chars.is_empty() {
            return 0;
        }

        let mut p = min(offset, chars.len());

        // Skip whitespace
        while p > 0 && chars[p - 1].is_whitespace() {
            p -= 1;
        }

        if p == 0 {
            return 0;
        }

        if subword {
            if chars[p - 1] == '_' {
                while p > 0 && chars[p - 1] == '_' {
                    p -= 1;
                }
                return p;
            }

            let is_upper = chars[p - 1].is_uppercase();
            while p > 0 && is_identifier_char(chars[p - 1]) && chars[p - 1] != '_' {
                if !is_upper && chars[p - 1].is_uppercase() {
                    p -= 1;
                    break;
                }
                p -= 1;
                if is_upper && p > 0 && chars[p - 1].is_lowercase() {
                    break;
                }
            }
        } else {
            let is_ident = is_identifier_char(chars[p - 1]);
            while p > 0
                && !chars[p - 1].is_whitespace()
                && is_identifier_char(chars[p - 1]) == is_ident
            {
                p -= 1;
            }
        }

        p
    }

    /// Find next word start offset after `offset`. Supports camelCase and snake_case subwords.
    pub fn find_next_word_boundary(chars: &[char], offset: usize, subword: bool) -> usize {
        let len = chars.len();
        if offset >= len {
            return len;
        }

        let mut p = offset;

        if subword {
            if chars[p] == '_' {
                while p < len && chars[p] == '_' {
                    p += 1;
                }
            } else if chars[p].is_uppercase() {
                p += 1;
                while p < len && chars[p].is_lowercase() {
                    p += 1;
                }
            } else {
                while p < len
                    && is_identifier_char(chars[p])
                    && !chars[p].is_uppercase()
                    && chars[p] != '_'
                {
                    p += 1;
                }
            }
        } else {
            let is_ident = is_identifier_char(chars[p]);
            while p < len && is_identifier_char(chars[p]) == is_ident && !chars[p].is_whitespace() {
                p += 1;
            }
        }

        // Skip whitespace to next word start
        while p < len && chars[p].is_whitespace() {
            p += 1;
        }

        p
    }

    /// Find word boundaries `(start, end)` encompassing `offset`.
    pub fn find_word_range_at(chars: &[char], offset: usize) -> Option<(usize, usize)> {
        if chars.is_empty() {
            return None;
        }

        let off = min(offset, chars.len().saturating_sub(1));
        if !is_identifier_char(chars[off]) {
            return None;
        }

        let mut start = off;
        while start > 0 && is_identifier_char(chars[start - 1]) {
            start -= 1;
        }

        let mut end = off;
        while end < chars.len() && is_identifier_char(chars[end]) {
            end += 1;
        }

        if start < end {
            Some((start, end))
        } else {
            None
        }
    }

    /// Jump to next paragraph boundary (blank line separated).
    pub fn next_paragraph(buffer: &MultilineBuffer, current_offset: usize) -> usize {
        let cur_pos = buffer.position_of(current_offset);
        let ranges = buffer.line_ranges();
        let total = ranges.len();

        let mut line = cur_pos.line;
        while line < total && ranges[line].1 > 0 {
            line += 1;
        }
        while line < total && ranges[line].1 == 0 {
            line += 1;
        }

        let target_line = min(line, total.saturating_sub(1));
        ranges[target_line].0
    }

    /// Jump to previous paragraph boundary (blank line separated).
    pub fn prev_paragraph(buffer: &MultilineBuffer, current_offset: usize) -> usize {
        let cur_pos = buffer.position_of(current_offset);
        let ranges = buffer.line_ranges();

        let mut line = cur_pos.line;
        while line > 0 && ranges[line].1 == 0 {
            line -= 1;
        }
        while line > 0 && ranges[line].1 > 0 {
            line -= 1;
        }

        ranges[line].0
    }

    /// Jump to next markdown heading (`# `, `## `, etc.).
    pub fn next_heading(buffer: &MultilineBuffer, current_offset: usize) -> usize {
        let cur_pos = buffer.position_of(current_offset);
        let ranges = buffer.line_ranges();

        for &(start, len) in ranges.iter().skip(cur_pos.line + 1) {
            let line_str: String = buffer.chars()[start..start + len].iter().collect();
            if line_str.starts_with('#') {
                return start;
            }
        }

        current_offset
    }

    /// Jump to next code block boundary (```).
    pub fn next_code_block(buffer: &MultilineBuffer, current_offset: usize) -> usize {
        let cur_pos = buffer.position_of(current_offset);
        let ranges = buffer.line_ranges();

        for &(start, len) in ranges.iter().skip(cur_pos.line + 1) {
            let line_str: String = buffer.chars()[start..start + len].iter().collect();
            if line_str.starts_with("```") {
                return start;
            }
        }

        current_offset
    }
}

/// Helper to check if a character is part of a standard word / identifier.
fn is_identifier_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

// ---------------------------------------------------------------------------
// Text Block & Rectangular Operations
// ---------------------------------------------------------------------------

/// Sorting options for text blocks.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct SortOptions {
    /// Sort in ascending order (true) or descending (false).
    pub ascending: bool,
    /// Case-sensitive comparison.
    pub case_sensitive: bool,
    /// Numeric value sorting (e.g. 2 before 10).
    pub numeric: bool,
    /// Remove duplicate lines.
    pub unique: bool,
}

impl Default for SortOptions {
    fn default() -> Self {
        Self {
            ascending: true,
            case_sensitive: true,
            numeric: false,
            unique: false,
        }
    }
}

/// Text case transformation kinds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum CaseTransform {
    Uppercase,
    Lowercase,
    Titlecase,
    SnakeCase,
    CamelCase,
    KebabCase,
}

/// Comprehensive line and rectangular block manipulation operations.
pub struct BlockOperations;

impl BlockOperations {
    /// Extract lines from a rectangular [`BlockRange`].
    pub fn extract_block(buffer: &MultilineBuffer, block: BlockRange) -> Vec<String> {
        let mut result = Vec::new();
        for line_idx in block.lines() {
            if let Some(line) = buffer.line_text(line_idx) {
                let chars: Vec<char> = line.chars().collect();
                let start = min(block.start_col, chars.len());
                let end = min(block.end_col, chars.len());
                let sub: String = if start < end {
                    chars[start..end].iter().collect()
                } else {
                    String::new()
                };
                result.push(sub);
            }
        }
        result
    }

    /// Indent a range of lines by `indent_str` (e.g. `"    "` or `"\t"`).
    pub fn indent_lines(
        buffer: &mut MultilineBuffer,
        start_line: usize,
        end_line: usize,
        indent_str: &str,
    ) {
        let total = buffer.line_count();
        let s = min(start_line, total.saturating_sub(1));
        let e = min(end_line, total.saturating_sub(1));

        let mut offset_acc = 0;
        let ranges = buffer.line_ranges();

        for line_idx in s..=e {
            let (orig_start, orig_len) = ranges[line_idx];
            if orig_len > 0 || line_idx == s {
                let insert_at = orig_start + offset_acc;
                buffer.insert_str(insert_at, indent_str);
                offset_acc += indent_str.chars().count();
            }
        }
    }

    /// Outdent a range of lines by up to `indent_width` leading spaces.
    pub fn outdent_lines(
        buffer: &mut MultilineBuffer,
        start_line: usize,
        end_line: usize,
        indent_width: usize,
    ) {
        let total = buffer.line_count();
        let s = min(start_line, total.saturating_sub(1));
        let e = min(end_line, total.saturating_sub(1));

        for line_idx in (s..=e).rev() {
            if let Some((start, len)) = buffer.line_range(line_idx) {
                let line_chars = &buffer.chars()[start..start + len];
                let mut spaces_to_remove = 0;
                while spaces_to_remove < indent_width
                    && spaces_to_remove < line_chars.len()
                    && line_chars[spaces_to_remove] == ' '
                {
                    spaces_to_remove += 1;
                }
                if spaces_to_remove == 0 && !line_chars.is_empty() && line_chars[0] == '\t' {
                    spaces_to_remove = 1;
                }
                if spaces_to_remove > 0 {
                    buffer.delete_range(start, start + spaces_to_remove);
                }
            }
        }
    }

    /// Toggle comment prefix across a range of lines.
    pub fn toggle_comment(
        buffer: &mut MultilineBuffer,
        start_line: usize,
        end_line: usize,
        prefix: &str,
    ) {
        let total = buffer.line_count();
        let s = min(start_line, total.saturating_sub(1));
        let e = min(end_line, total.saturating_sub(1));

        let formatted_prefix = if prefix.ends_with(' ') {
            prefix.to_string()
        } else {
            format!("{} ", prefix)
        };

        let mut all_commented = true;
        let mut has_non_empty = false;

        for line_idx in s..=e {
            if let Some(text) = buffer.line_text(line_idx) {
                let trimmed = text.trim();
                if !trimmed.is_empty() {
                    has_non_empty = true;
                    if !trimmed.starts_with(prefix.trim()) {
                        all_commented = false;
                        break;
                    }
                }
            }
        }

        if !has_non_empty {
            return;
        }

        if all_commented {
            for line_idx in (s..=e).rev() {
                if let Some((start, len)) = buffer.line_range(line_idx) {
                    let line_str: String = buffer.chars()[start..start + len].iter().collect();
                    if let Some(p_idx) = line_str.find(prefix.trim()) {
                        let to_remove = if line_str[p_idx..].starts_with(&formatted_prefix) {
                            formatted_prefix.chars().count()
                        } else {
                            prefix.trim().chars().count()
                        };
                        buffer.delete_range(start + p_idx, start + p_idx + to_remove);
                    }
                }
            }
        } else {
            for line_idx in (s..=e).rev() {
                if let Some((start, _)) = buffer.line_range(line_idx) {
                    buffer.insert_str(start, &formatted_prefix);
                }
            }
        }
    }

    /// Move line block up by one line (swap with line above). Returns true if moved.
    pub fn move_lines_up(buffer: &mut MultilineBuffer, start_line: usize, end_line: usize) -> bool {
        if start_line == 0 || end_line >= buffer.line_count() {
            return false;
        }

        let mut lines = buffer.lines();
        let prev_line = lines.remove(start_line - 1);
        lines.insert(end_line, prev_line);
        buffer.set_text(&lines.join("\n"));
        true
    }

    /// Move line block down by one line (swap with line below). Returns true if moved.
    pub fn move_lines_down(
        buffer: &mut MultilineBuffer,
        start_line: usize,
        end_line: usize,
    ) -> bool {
        if end_line + 1 >= buffer.line_count() {
            return false;
        }

        let mut lines = buffer.lines();
        let next_line = lines.remove(end_line + 1);
        lines.insert(start_line, next_line);
        buffer.set_text(&lines.join("\n"));
        true
    }

    /// Duplicate lines in range immediately below.
    pub fn duplicate_lines_down(buffer: &mut MultilineBuffer, start_line: usize, end_line: usize) {
        let lines = buffer.lines();
        let s = min(start_line, lines.len().saturating_sub(1));
        let e = min(end_line, lines.len().saturating_sub(1));

        let block: Vec<String> = lines[s..=e].to_vec();
        let mut new_lines = lines;
        for (offset, line) in block.into_iter().enumerate() {
            new_lines.insert(e + 1 + offset, line);
        }
        buffer.set_text(&new_lines.join("\n"));
    }

    /// Sort lines in specified range.
    pub fn sort_lines(
        buffer: &mut MultilineBuffer,
        start_line: usize,
        end_line: usize,
        opts: SortOptions,
    ) {
        let mut lines = buffer.lines();
        let s = min(start_line, lines.len().saturating_sub(1));
        let e = min(end_line, lines.len().saturating_sub(1));

        if s >= e {
            return;
        }

        let mut block = lines[s..=e].to_vec();

        block.sort_by(|a, b| {
            let order = if opts.numeric {
                let num_a = a.trim().parse::<f64>().unwrap_or(f64::NAN);
                let num_b = b.trim().parse::<f64>().unwrap_or(f64::NAN);
                if !num_a.is_nan() && !num_b.is_nan() {
                    num_a.partial_cmp(&num_b).unwrap_or(Ordering::Equal)
                } else if opts.case_sensitive {
                    a.cmp(b)
                } else {
                    a.to_lowercase().cmp(&b.to_lowercase())
                }
            } else if opts.case_sensitive {
                a.cmp(b)
            } else {
                a.to_lowercase().cmp(&b.to_lowercase())
            };

            if opts.ascending {
                order
            } else {
                order.reverse()
            }
        });

        if opts.unique {
            block.dedup();
        }

        lines.splice(s..=e, block);
        buffer.set_text(&lines.join("\n"));
    }

    /// Align column of characters vertically by delimiter (e.g. `:` or `=`).
    pub fn align_column_by_delimiter(
        buffer: &mut MultilineBuffer,
        start_line: usize,
        end_line: usize,
        delimiter: char,
    ) {
        let mut lines = buffer.lines();
        let s = min(start_line, lines.len().saturating_sub(1));
        let e = min(end_line, lines.len().saturating_sub(1));

        let mut max_pos = 0;
        for line in &lines[s..=e] {
            if let Some(pos) = line.find(delimiter) {
                let col = line[..pos].chars().count();
                max_pos = max(max_pos, col);
            }
        }

        if max_pos == 0 {
            return;
        }

        for line in &mut lines[s..=e] {
            if let Some(pos) = line.find(delimiter) {
                let left = line[..pos].trim_end();
                let right = line[pos + delimiter.len_utf8()..].trim_start();
                let left_len = left.chars().count();
                let pad = " ".repeat(max_pos.saturating_sub(left_len));
                *line = format!("{}{} {} {}", left, pad, delimiter, right);
            }
        }

        buffer.set_text(&lines.join("\n"));
    }

    /// Surround selection or range with opening and closing delimiters.
    pub fn surround(
        buffer: &mut MultilineBuffer,
        start: usize,
        end: usize,
        open: &str,
        close: &str,
    ) {
        let s = min(start, end);
        let e = max(start, end);
        let inner = buffer.slice(s, e);
        let replaced = format!("{}{}{}", open, inner, close);
        buffer.replace_range(s, e, &replaced);
    }

    /// Apply case transformation to text range.
    pub fn transform_case(
        buffer: &mut MultilineBuffer,
        start: usize,
        end: usize,
        transform: CaseTransform,
    ) {
        let s = min(start, end);
        let e = max(start, end);
        let text = buffer.slice(s, e);

        let transformed = match transform {
            CaseTransform::Uppercase => text.to_uppercase(),
            CaseTransform::Lowercase => text.to_lowercase(),
            CaseTransform::Titlecase => {
                let mut c = text.chars();
                match c.next() {
                    None => String::new(),
                    Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
                }
            }
            CaseTransform::SnakeCase => to_snake_case(&text),
            CaseTransform::CamelCase => to_camel_case(&text),
            CaseTransform::KebabCase => to_snake_case(&text).replace('_', "-"),
        };

        buffer.replace_range(s, e, &transformed);
    }

    /// Trim trailing whitespace on all lines.
    pub fn trim_trailing_whitespace(buffer: &mut MultilineBuffer) {
        let lines: Vec<String> = buffer
            .lines()
            .into_iter()
            .map(|l| l.trim_end().to_string())
            .collect();
        buffer.set_text(&lines.join("\n"));
    }
}

/// Helper for snake_case conversion.
fn to_snake_case(s: &str) -> String {
    let mut result = String::new();
    for (i, c) in s.chars().enumerate() {
        if c.is_uppercase() {
            if i > 0 && !result.ends_with('_') {
                result.push('_');
            }
            result.push(c.to_ascii_lowercase());
        } else if c == '-' || c == ' ' {
            if !result.ends_with('_') {
                result.push('_');
            }
        } else {
            result.push(c);
        }
    }
    result
}

/// Helper for camelCase conversion.
fn to_camel_case(s: &str) -> String {
    let mut result = String::new();
    let mut capitalize_next = false;
    for (i, c) in s.chars().enumerate() {
        if c == '_' || c == '-' || c == ' ' {
            capitalize_next = true;
        } else if capitalize_next {
            result.push(c.to_ascii_uppercase());
            capitalize_next = false;
        } else if i == 0 {
            result.push(c.to_ascii_lowercase());
        } else {
            result.push(c);
        }
    }
    result
}

// ---------------------------------------------------------------------------
// Transactional Undo / Redo History
// ---------------------------------------------------------------------------

/// Snapshot of editor state for undo/redo history.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BufferSnapshot {
    /// Buffer text characters.
    pub chars: Vec<char>,
    /// Multi-cursor state.
    pub cursors: MultiCursorState,
}

/// Undo and redo history manager with maximum depth limits.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BufferHistory {
    /// Undo history stack.
    undo_stack: Vec<BufferSnapshot>,
    /// Redo history stack.
    redo_stack: Vec<BufferSnapshot>,
    /// Maximum number of undo states to preserve.
    max_depth: usize,
}

impl Default for BufferHistory {
    fn default() -> Self {
        Self::new(100)
    }
}

impl BufferHistory {
    /// Create a new history manager with max undo depth.
    pub fn new(max_depth: usize) -> Self {
        Self {
            undo_stack: Vec::new(),
            redo_stack: Vec::new(),
            max_depth: max(max_depth, 10),
        }
    }

    /// Push current state to undo stack and clear redo stack.
    pub fn record(&mut self, buffer: &MultilineBuffer, cursors: &MultiCursorState) {
        self.redo_stack.clear();
        if self.undo_stack.len() >= self.max_depth {
            self.undo_stack.remove(0);
        }
        self.undo_stack.push(BufferSnapshot {
            chars: buffer.chars().to_vec(),
            cursors: cursors.clone(),
        });
    }

    /// Whether undo is available.
    pub fn can_undo(&self) -> bool {
        !self.undo_stack.is_empty()
    }

    /// Whether redo is available.
    pub fn can_redo(&self) -> bool {
        !self.redo_stack.is_empty()
    }

    /// Perform undo. Reverts buffer and cursors to prior snapshot.
    pub fn undo(&mut self, buffer: &mut MultilineBuffer, cursors: &mut MultiCursorState) -> bool {
        if let Some(snapshot) = self.undo_stack.pop() {
            self.redo_stack.push(BufferSnapshot {
                chars: buffer.chars().to_vec(),
                cursors: cursors.clone(),
            });
            buffer.chars = snapshot.chars;
            *cursors = snapshot.cursors;
            true
        } else {
            false
        }
    }

    /// Perform redo. Restores previously undone snapshot.
    pub fn redo(&mut self, buffer: &mut MultilineBuffer, cursors: &mut MultiCursorState) -> bool {
        if let Some(snapshot) = self.redo_stack.pop() {
            self.undo_stack.push(BufferSnapshot {
                chars: buffer.chars().to_vec(),
                cursors: cursors.clone(),
            });
            buffer.chars = snapshot.chars;
            *cursors = snapshot.cursors;
            true
        } else {
            false
        }
    }

    /// Clear all undo/redo history.
    pub fn clear(&mut self) {
        self.undo_stack.clear();
        self.redo_stack.clear();
    }
}

// ---------------------------------------------------------------------------
// High-Level Editor Buffer Facade
// ---------------------------------------------------------------------------

/// High-level interactive multiline prompt editor combining buffer, multi-cursors,
/// readline keybindings, kill ring, and horizontal scroll viewport.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct EditorBuffer {
    /// Underlying text buffer.
    pub buffer: MultilineBuffer,
    /// Multi-cursor state.
    pub cursors: MultiCursorState,
    /// Undo/redo history.
    pub history: BufferHistory,
    /// Readline kill ring.
    pub kill_ring: KillRing,
    /// Horizontal scroll tracking state.
    pub horizontal_scroll: HorizontalScrollState,
    /// Flag tracking whether the previous operation was a kill operation.
    pub last_action_was_kill: bool,
    /// Flag tracking whether the previous operation was a yank operation.
    pub last_action_was_yank: bool,
    /// Length of last yanked text per cursor (used to replace text during `yank_pop`).
    pub last_yank_lengths: Vec<usize>,
}

impl EditorBuffer {
    /// Create a new empty editor buffer.
    pub fn new() -> Self {
        Self {
            buffer: MultilineBuffer::new(),
            cursors: MultiCursorState::new(),
            history: BufferHistory::default(),
            kill_ring: KillRing::default(),
            horizontal_scroll: HorizontalScrollState::default(),
            last_action_was_kill: false,
            last_action_was_yank: false,
            last_yank_lengths: Vec::new(),
        }
    }

    /// Create an editor buffer initialized from string.
    pub fn from_str(s: &str) -> Self {
        let count = s.chars().count();
        let mut editor = Self {
            buffer: MultilineBuffer::from_str(s),
            cursors: MultiCursorState::with_offset(count),
            history: BufferHistory::default(),
            kill_ring: KillRing::default(),
            horizontal_scroll: HorizontalScrollState::default(),
            last_action_was_kill: false,
            last_action_was_yank: false,
            last_yank_lengths: Vec::new(),
        };
        editor.sync_scroll();
        editor
    }

    /// Create an editor buffer initialized from string with specific viewport width.
    pub fn with_viewport(s: &str, viewport_width: usize) -> Self {
        let count = s.chars().count();
        let mut editor = Self {
            buffer: MultilineBuffer::from_str(s),
            cursors: MultiCursorState::with_offset(count),
            history: BufferHistory::default(),
            kill_ring: KillRing::default(),
            horizontal_scroll: HorizontalScrollState::new(viewport_width, 2),
            last_action_was_kill: false,
            last_action_was_yank: false,
            last_yank_lengths: Vec::new(),
        };
        editor.sync_scroll();
        editor
    }

    /// Get current full text.
    pub fn text(&self) -> String {
        self.buffer.text()
    }

    /// Replace full buffer contents and reset cursor.
    pub fn set_text(&mut self, s: &str) {
        self.history.record(&self.buffer, &self.cursors);
        self.buffer.set_text(s);
        let new_off = min(self.cursors.primary().offset, self.buffer.len());
        self.cursors.reset_to_single(new_off);
        self.last_action_was_kill = false;
        self.last_action_was_yank = false;
        self.sync_scroll();
    }

    /// Character length.
    pub fn len(&self) -> usize {
        self.buffer.len()
    }

    /// Whether buffer is empty.
    pub fn is_empty(&self) -> bool {
        self.buffer.is_empty()
    }

    /// Line count.
    pub fn line_count(&self) -> usize {
        self.buffer.line_count()
    }

    /// Primary cursor 2D position `(line, col)`.
    pub fn cursor_position(&self) -> Position {
        self.buffer.position_of(self.cursors.primary().offset)
    }

    /// Primary cursor character offset.
    pub fn cursor_offset(&self) -> usize {
        self.cursors.primary().offset
    }

    /// Synchronize horizontal scroll to make primary cursor visible.
    pub fn sync_scroll(&mut self) {
        let pos = self.cursor_position();
        self.horizontal_scroll.ensure_visible(pos.col);
    }

    /// Insert a character at all active cursors (replacing selections if present).
    pub fn insert_char(&mut self, c: char) {
        self.history.record(&self.buffer, &self.cursors);
        self.last_action_was_kill = false;
        self.last_action_was_yank = false;

        let mut cursors_to_process = self.cursors.cursors().to_vec();
        cursors_to_process.sort_by_key(|c| std::cmp::Reverse(c.selection_start()));

        for cur in &mut cursors_to_process {
            if cur.has_selection() {
                let sel = cur.selection();
                self.buffer.delete_range(sel.start(), sel.end());
                cur.offset = sel.start();
                cur.collapse();
            }
            self.buffer.insert_char(cur.offset, c);
            cur.offset += 1;
            cur.desired_col = None;
        }

        self.cursors = MultiCursorState {
            cursors: cursors_to_process,
            primary_idx: 0,
        };
        self.cursors.normalize();
        self.sync_scroll();
    }

    /// Insert a string slice at all active cursors.
    pub fn insert_str(&mut self, s: &str) {
        self.history.record(&self.buffer, &self.cursors);
        self.last_action_was_kill = false;
        self.last_action_was_yank = false;

        let s_len = s.chars().count();
        let mut cursors_to_process = self.cursors.cursors().to_vec();
        cursors_to_process.sort_by_key(|c| std::cmp::Reverse(c.selection_start()));

        for cur in &mut cursors_to_process {
            if cur.has_selection() {
                let sel = cur.selection();
                self.buffer.delete_range(sel.start(), sel.end());
                cur.offset = sel.start();
                cur.collapse();
            }
            self.buffer.insert_str(cur.offset, s);
            cur.offset += s_len;
            cur.desired_col = None;
        }

        self.cursors = MultiCursorState {
            cursors: cursors_to_process,
            primary_idx: 0,
        };
        self.cursors.normalize();
        self.sync_scroll();
    }

    /// Insert a newline at all cursors, optionally copying current line's leading indentation.
    pub fn insert_newline(&mut self, auto_indent: bool) {
        self.history.record(&self.buffer, &self.cursors);
        self.last_action_was_kill = false;
        self.last_action_was_yank = false;

        let mut cursors_to_process = self.cursors.cursors().to_vec();
        cursors_to_process.sort_by_key(|c| std::cmp::Reverse(c.selection_start()));

        for cur in &mut cursors_to_process {
            if cur.has_selection() {
                let sel = cur.selection();
                self.buffer.delete_range(sel.start(), sel.end());
                cur.offset = sel.start();
                cur.collapse();
            }

            let indent_str = if auto_indent {
                let pos = self.buffer.position_of(cur.offset);
                if let Some(line) = self.buffer.line_text(pos.line) {
                    let mut spaces = 0;
                    for ch in line.chars() {
                        if ch == ' ' || ch == '\t' {
                            spaces += 1;
                        } else {
                            break;
                        }
                    }
                    line[..spaces].to_string()
                } else {
                    String::new()
                }
            } else {
                String::new()
            };

            let to_insert = format!("\n{}", indent_str);
            let insert_len = to_insert.chars().count();
            self.buffer.insert_str(cur.offset, &to_insert);
            cur.offset += insert_len;
            cur.desired_col = None;
        }

        self.cursors = MultiCursorState {
            cursors: cursors_to_process,
            primary_idx: 0,
        };
        self.cursors.normalize();
        self.sync_scroll();
    }

    /// Delete character immediately before cursor (Backspace / `Ctrl+H`).
    pub fn backspace(&mut self) {
        self.history.record(&self.buffer, &self.cursors);
        self.last_action_was_kill = false;
        self.last_action_was_yank = false;

        let mut cursors_to_process = self.cursors.cursors().to_vec();
        cursors_to_process.sort_by_key(|c| std::cmp::Reverse(c.selection_start()));

        for cur in &mut cursors_to_process {
            if cur.has_selection() {
                let sel = cur.selection();
                self.buffer.delete_range(sel.start(), sel.end());
                cur.offset = sel.start();
                cur.collapse();
            } else if cur.offset > 0 {
                cur.offset -= 1;
                self.buffer.delete_range(cur.offset, cur.offset + 1);
            }
            cur.desired_col = None;
        }

        self.cursors = MultiCursorState {
            cursors: cursors_to_process,
            primary_idx: 0,
        };
        self.cursors.normalize();
        self.sync_scroll();
    }

    /// Delete character under cursor (Delete key / `Ctrl+D`).
    pub fn delete(&mut self) {
        self.history.record(&self.buffer, &self.cursors);
        self.last_action_was_kill = false;
        self.last_action_was_yank = false;

        let mut cursors_to_process = self.cursors.cursors().to_vec();
        cursors_to_process.sort_by_key(|c| std::cmp::Reverse(c.selection_start()));

        for cur in &mut cursors_to_process {
            if cur.has_selection() {
                let sel = cur.selection();
                self.buffer.delete_range(sel.start(), sel.end());
                cur.offset = sel.start();
                cur.collapse();
            } else if cur.offset < self.buffer.len() {
                self.buffer.delete_range(cur.offset, cur.offset + 1);
            }
            cur.desired_col = None;
        }

        self.cursors = MultiCursorState {
            cursors: cursors_to_process,
            primary_idx: 0,
        };
        self.cursors.normalize();
        self.sync_scroll();
    }

    /// Kill text from cursor to end of line (`Ctrl+K`).
    ///
    /// If cursor is already at end of line and not the last line, deletes the newline character.
    /// Consecutive kills are appended into the same kill ring entry.
    pub fn kill_line_to_end(&mut self) {
        self.history.record(&self.buffer, &self.cursors);

        let is_consecutive = self.last_action_was_kill;
        let mut killed_fragments = Vec::new();

        let mut cursors_to_process = self.cursors.cursors().to_vec();
        cursors_to_process.sort_by_key(|c| std::cmp::Reverse(c.selection_start()));

        for cur in &mut cursors_to_process {
            if cur.has_selection() {
                let sel = cur.selection();
                let deleted = self.buffer.delete_range(sel.start(), sel.end());
                cur.offset = sel.start();
                cur.collapse();
                killed_fragments.push(deleted);
            } else {
                let pos = self.buffer.position_of(cur.offset);
                let ranges = self.buffer.line_ranges();
                if pos.line < ranges.len() {
                    let (line_start, line_len) = ranges[pos.line];
                    let line_end = line_start + line_len;

                    if cur.offset < line_end {
                        let deleted = self.buffer.delete_range(cur.offset, line_end);
                        killed_fragments.push(deleted);
                    } else if cur.offset < self.buffer.len() {
                        // At EOL, kill the newline character
                        let deleted = self.buffer.delete_range(cur.offset, cur.offset + 1);
                        killed_fragments.push(deleted);
                    }
                }
            }
            cur.desired_col = None;
        }

        self.cursors = MultiCursorState {
            cursors: cursors_to_process,
            primary_idx: 0,
        };
        self.cursors.normalize();

        // Primary killed fragment pushed to kill ring
        if let Some(first_killed) = killed_fragments.into_iter().next() {
            if is_consecutive {
                self.kill_ring.push_append(&first_killed, false);
            } else {
                self.kill_ring.push(&first_killed);
            }
        }

        self.last_action_was_kill = true;
        self.last_action_was_yank = false;
        self.sync_scroll();
    }

    /// Kill text from cursor to start of line (`Ctrl+U`).
    ///
    /// If cursor is already at start of line and not the first line, deletes previous newline.
    pub fn kill_line_to_start(&mut self) {
        self.history.record(&self.buffer, &self.cursors);

        let is_consecutive = self.last_action_was_kill;
        let mut killed_fragments = Vec::new();

        let mut cursors_to_process = self.cursors.cursors().to_vec();
        cursors_to_process.sort_by_key(|c| std::cmp::Reverse(c.selection_start()));

        for cur in &mut cursors_to_process {
            if cur.has_selection() {
                let sel = cur.selection();
                let deleted = self.buffer.delete_range(sel.start(), sel.end());
                cur.offset = sel.start();
                cur.collapse();
                killed_fragments.push(deleted);
            } else {
                let pos = self.buffer.position_of(cur.offset);
                let ranges = self.buffer.line_ranges();
                if pos.line < ranges.len() {
                    let (line_start, _) = ranges[pos.line];
                    if cur.offset > line_start {
                        let deleted = self.buffer.delete_range(line_start, cur.offset);
                        cur.offset = line_start;
                        killed_fragments.push(deleted);
                    } else if cur.offset > 0 {
                        // At BOL, kill previous newline
                        cur.offset -= 1;
                        let deleted = self.buffer.delete_range(cur.offset, cur.offset + 1);
                        killed_fragments.push(deleted);
                    }
                }
            }
            cur.desired_col = None;
        }

        self.cursors = MultiCursorState {
            cursors: cursors_to_process,
            primary_idx: 0,
        };
        self.cursors.normalize();

        if let Some(first_killed) = killed_fragments.into_iter().next() {
            if is_consecutive {
                self.kill_ring.push_append(&first_killed, true);
            } else {
                self.kill_ring.push(&first_killed);
            }
        }

        self.last_action_was_kill = true;
        self.last_action_was_yank = false;
        self.sync_scroll();
    }

    /// Kill word backward from cursor (`Ctrl+W`).
    pub fn kill_word_backward(&mut self) {
        self.history.record(&self.buffer, &self.cursors);

        let is_consecutive = self.last_action_was_kill;
        let mut killed_fragments = Vec::new();

        let mut cursors_to_process = self.cursors.cursors().to_vec();
        cursors_to_process.sort_by_key(|c| std::cmp::Reverse(c.selection_start()));

        for cur in &mut cursors_to_process {
            if cur.has_selection() {
                let sel = cur.selection();
                let deleted = self.buffer.delete_range(sel.start(), sel.end());
                cur.offset = sel.start();
                cur.collapse();
                killed_fragments.push(deleted);
            } else if cur.offset > 0 {
                let word_start =
                    LineJumpHelper::find_prev_word_boundary(self.buffer.chars(), cur.offset, false);
                let deleted = self.buffer.delete_range(word_start, cur.offset);
                cur.offset = word_start;
                killed_fragments.push(deleted);
            }
            cur.desired_col = None;
        }

        self.cursors = MultiCursorState {
            cursors: cursors_to_process,
            primary_idx: 0,
        };
        self.cursors.normalize();

        if let Some(first_killed) = killed_fragments.into_iter().next() {
            if is_consecutive {
                self.kill_ring.push_append(&first_killed, true);
            } else {
                self.kill_ring.push(&first_killed);
            }
        }

        self.last_action_was_kill = true;
        self.last_action_was_yank = false;
        self.sync_scroll();
    }

    /// Kill word forward from cursor (`Alt+D`).
    pub fn kill_word_forward(&mut self) {
        self.history.record(&self.buffer, &self.cursors);

        let is_consecutive = self.last_action_was_kill;
        let mut killed_fragments = Vec::new();

        let mut cursors_to_process = self.cursors.cursors().to_vec();
        cursors_to_process.sort_by_key(|c| std::cmp::Reverse(c.selection_start()));

        for cur in &mut cursors_to_process {
            if cur.has_selection() {
                let sel = cur.selection();
                let deleted = self.buffer.delete_range(sel.start(), sel.end());
                cur.offset = sel.start();
                cur.collapse();
                killed_fragments.push(deleted);
            } else if cur.offset < self.buffer.len() {
                let word_end =
                    LineJumpHelper::find_next_word_boundary(self.buffer.chars(), cur.offset, false);
                let deleted = self.buffer.delete_range(cur.offset, word_end);
                killed_fragments.push(deleted);
            }
            cur.desired_col = None;
        }

        self.cursors = MultiCursorState {
            cursors: cursors_to_process,
            primary_idx: 0,
        };
        self.cursors.normalize();

        if let Some(first_killed) = killed_fragments.into_iter().next() {
            if is_consecutive {
                self.kill_ring.push_append(&first_killed, false);
            } else {
                self.kill_ring.push(&first_killed);
            }
        }

        self.last_action_was_kill = true;
        self.last_action_was_yank = false;
        self.sync_scroll();
    }

    /// Yank (paste) most recently killed text at all cursors (`Ctrl+Y`).
    pub fn yank(&mut self) {
        let text_opt = self.kill_ring.yank().map(|s| s.to_string());
        if let Some(text) = text_opt {
            self.history.record(&self.buffer, &self.cursors);
            self.last_action_was_kill = false;
            self.last_action_was_yank = true;

            let num_cursors = self.cursors.count();
            let lines: Vec<&str> = text.lines().collect();

            let mut yank_lengths = Vec::new();
            let mut cursors_to_process = self.cursors.cursors().to_vec();
            cursors_to_process.sort_by_key(|c| std::cmp::Reverse(c.selection_start()));

            for (idx, cur) in cursors_to_process.iter_mut().enumerate() {
                if cur.has_selection() {
                    let sel = cur.selection();
                    self.buffer.delete_range(sel.start(), sel.end());
                    cur.offset = sel.start();
                    cur.collapse();
                }

                let piece = if num_cursors > 1 && lines.len() == num_cursors {
                    let line_idx = num_cursors.saturating_sub(1) - idx;
                    lines.get(line_idx).copied().unwrap_or(&text)
                } else {
                    &text
                };

                let piece_len = piece.chars().count();
                self.buffer.insert_str(cur.offset, piece);
                cur.offset += piece_len;
                cur.desired_col = None;
                yank_lengths.push(piece_len);
            }

            self.last_yank_lengths = yank_lengths;
            self.cursors = MultiCursorState {
                cursors: cursors_to_process,
                primary_idx: 0,
            };
            self.cursors.normalize();
            self.sync_scroll();
        }
    }

    /// Cycle to previous kill ring item replacing last yank (`Alt+Y`).
    pub fn yank_pop(&mut self) {
        if !self.last_action_was_yank || self.last_yank_lengths.is_empty() {
            return;
        }

        let new_text_opt = self.kill_ring.yank_pop().map(|s| s.to_string());
        if let Some(new_text) = new_text_opt {
            let num_cursors = self.cursors.count();
            let lines: Vec<&str> = new_text.lines().collect();

            let mut cursors_to_process = self.cursors.cursors().to_vec();
            cursors_to_process.sort_by_key(|c| std::cmp::Reverse(c.selection_start()));

            let mut new_yank_lengths = Vec::new();

            for (idx, cur) in cursors_to_process.iter_mut().enumerate() {
                let prev_len = self.last_yank_lengths.get(idx).copied().unwrap_or(0);
                if cur.offset >= prev_len {
                    let start = cur.offset - prev_len;
                    self.buffer.delete_range(start, cur.offset);
                    cur.offset = start;
                }

                let piece = if num_cursors > 1 && lines.len() == num_cursors {
                    let line_idx = num_cursors.saturating_sub(1) - idx;
                    lines.get(line_idx).copied().unwrap_or(&new_text)
                } else {
                    &new_text
                };

                let piece_len = piece.chars().count();
                self.buffer.insert_str(cur.offset, piece);
                cur.offset += piece_len;
                cur.desired_col = None;
                new_yank_lengths.push(piece_len);
            }

            self.last_yank_lengths = new_yank_lengths;
            self.cursors = MultiCursorState {
                cursors: cursors_to_process,
                primary_idx: 0,
            };
            self.cursors.normalize();
            self.last_action_was_yank = true;
            self.sync_scroll();
        }
    }

    /// Move cursor to beginning of current line (`Ctrl+A` / `Home`).
    pub fn move_line_start(&mut self, extend_selection: bool) {
        self.last_action_was_kill = false;
        self.last_action_was_yank = false;

        let ranges = self.buffer.line_ranges();
        for cur in self.cursors.cursors_mut() {
            let pos = self.buffer.position_of(cur.offset);
            if pos.line < ranges.len() {
                let (start, _) = ranges[pos.line];
                if extend_selection {
                    if cur.anchor.is_none() {
                        cur.anchor = Some(cur.offset);
                    }
                    cur.offset = start;
                } else {
                    cur.offset = start;
                    cur.collapse();
                }
                cur.desired_col = Some(0);
            }
        }
        self.cursors.normalize();
        self.sync_scroll();
    }

    /// Move cursor to end of current line (`Ctrl+E` / `End`).
    pub fn move_line_end(&mut self, extend_selection: bool) {
        self.last_action_was_kill = false;
        self.last_action_was_yank = false;

        let ranges = self.buffer.line_ranges();
        for cur in self.cursors.cursors_mut() {
            let pos = self.buffer.position_of(cur.offset);
            if pos.line < ranges.len() {
                let (start, len) = ranges[pos.line];
                let line_end = start + len;
                if extend_selection {
                    if cur.anchor.is_none() {
                        cur.anchor = Some(cur.offset);
                    }
                    cur.offset = line_end;
                } else {
                    cur.offset = line_end;
                    cur.collapse();
                }
                cur.desired_col = Some(len);
            }
        }
        self.cursors.normalize();
        self.sync_scroll();
    }

    /// Move cursor backward one word (`Alt+B` / `Ctrl+Left`).
    pub fn move_word_backward(&mut self, extend_selection: bool) {
        self.last_action_was_kill = false;
        self.last_action_was_yank = false;

        for cur in self.cursors.cursors_mut() {
            let target =
                LineJumpHelper::find_prev_word_boundary(self.buffer.chars(), cur.offset, false);
            if extend_selection {
                if cur.anchor.is_none() {
                    cur.anchor = Some(cur.offset);
                }
                cur.offset = target;
            } else {
                cur.offset = target;
                cur.collapse();
            }
            cur.desired_col = None;
        }
        self.cursors.normalize();
        self.sync_scroll();
    }

    /// Move cursor forward one word (`Alt+F` / `Ctrl+Right`).
    pub fn move_word_forward(&mut self, extend_selection: bool) {
        self.last_action_was_kill = false;
        self.last_action_was_yank = false;

        for cur in self.cursors.cursors_mut() {
            let target =
                LineJumpHelper::find_next_word_boundary(self.buffer.chars(), cur.offset, false);
            if extend_selection {
                if cur.anchor.is_none() {
                    cur.anchor = Some(cur.offset);
                }
                cur.offset = target;
            } else {
                cur.offset = target;
                cur.collapse();
            }
            cur.desired_col = None;
        }
        self.cursors.normalize();
        self.sync_scroll();
    }

    /// Move cursor backward one character (`Ctrl+B` / `Left`).
    pub fn move_char_backward(&mut self, extend_selection: bool) {
        self.last_action_was_kill = false;
        self.last_action_was_yank = false;

        for cur in self.cursors.cursors_mut() {
            if !extend_selection && cur.has_selection() {
                cur.collapse_to_start();
            } else if cur.offset > 0 {
                if extend_selection && cur.anchor.is_none() {
                    cur.anchor = Some(cur.offset);
                }
                cur.offset -= 1;
                if !extend_selection {
                    cur.collapse();
                }
            }
            cur.desired_col = None;
        }
        self.cursors.normalize();
        self.sync_scroll();
    }

    /// Move cursor forward one character (`Ctrl+F` / `Right`).
    pub fn move_char_forward(&mut self, extend_selection: bool) {
        self.last_action_was_kill = false;
        self.last_action_was_yank = false;

        let buf_len = self.buffer.len();
        for cur in self.cursors.cursors_mut() {
            if !extend_selection && cur.has_selection() {
                cur.collapse_to_end();
            } else if cur.offset < buf_len {
                if extend_selection && cur.anchor.is_none() {
                    cur.anchor = Some(cur.offset);
                }
                cur.offset += 1;
                if !extend_selection {
                    cur.collapse();
                }
            }
            cur.desired_col = None;
        }
        self.cursors.normalize();
        self.sync_scroll();
    }

    /// Move cursor up one line (`Ctrl+P` / `Up`).
    pub fn move_line_up(&mut self, extend_selection: bool) {
        self.last_action_was_kill = false;
        self.last_action_was_yank = false;

        let ranges = self.buffer.line_ranges();
        for cur in self.cursors.cursors_mut() {
            let pos = self.buffer.position_of(cur.offset);
            if pos.line > 0 {
                let target_line = pos.line - 1;
                let desired_col = cur.desired_col.unwrap_or(pos.col);
                let (line_start, line_len) = ranges[target_line];
                let target_col = min(desired_col, line_len);
                let target_off = line_start + target_col;

                if extend_selection {
                    if cur.anchor.is_none() {
                        cur.anchor = Some(cur.offset);
                    }
                    cur.offset = target_off;
                } else {
                    cur.offset = target_off;
                    cur.collapse();
                }
                cur.desired_col = Some(desired_col);
            }
        }
        self.cursors.normalize();
        self.sync_scroll();
    }

    /// Move cursor down one line (`Ctrl+N` / `Down`).
    pub fn move_line_down(&mut self, extend_selection: bool) {
        self.last_action_was_kill = false;
        self.last_action_was_yank = false;

        let ranges = self.buffer.line_ranges();
        for cur in self.cursors.cursors_mut() {
            let pos = self.buffer.position_of(cur.offset);
            if pos.line + 1 < ranges.len() {
                let target_line = pos.line + 1;
                let desired_col = cur.desired_col.unwrap_or(pos.col);
                let (line_start, line_len) = ranges[target_line];
                let target_col = min(desired_col, line_len);
                let target_off = line_start + target_col;

                if extend_selection {
                    if cur.anchor.is_none() {
                        cur.anchor = Some(cur.offset);
                    }
                    cur.offset = target_off;
                } else {
                    cur.offset = target_off;
                    cur.collapse();
                }
                cur.desired_col = Some(desired_col);
            }
        }
        self.cursors.normalize();
        self.sync_scroll();
    }

    /// Move cursor to beginning of entire buffer (`PageUp` / `Home` with buffer modifier).
    pub fn move_buffer_start(&mut self, extend_selection: bool) {
        self.last_action_was_kill = false;
        self.last_action_was_yank = false;

        for cur in self.cursors.cursors_mut() {
            if extend_selection {
                if cur.anchor.is_none() {
                    cur.anchor = Some(cur.offset);
                }
                cur.offset = 0;
            } else {
                cur.offset = 0;
                cur.collapse();
            }
            cur.desired_col = Some(0);
        }
        self.cursors.normalize();
        self.sync_scroll();
    }

    /// Move cursor to end of entire buffer (`PageDown` / `End` with buffer modifier).
    pub fn move_buffer_end(&mut self, extend_selection: bool) {
        self.last_action_was_kill = false;
        self.last_action_was_yank = false;

        let len = self.buffer.len();
        for cur in self.cursors.cursors_mut() {
            if extend_selection {
                if cur.anchor.is_none() {
                    cur.anchor = Some(cur.offset);
                }
                cur.offset = len;
            } else {
                cur.offset = len;
                cur.collapse();
            }
            cur.desired_col = None;
        }
        self.cursors.normalize();
        self.sync_scroll();
    }

    /// Transpose characters around cursor (`Ctrl+T`).
    pub fn transpose_chars(&mut self) {
        if self.buffer.len() < 2 {
            return;
        }

        self.history.record(&self.buffer, &self.cursors);
        self.last_action_was_kill = false;
        self.last_action_was_yank = false;

        for cur in self.cursors.cursors_mut() {
            if cur.offset == 0 {
                continue;
            }
            let swap_idx = if cur.offset >= self.buffer.len() {
                cur.offset.saturating_sub(1)
            } else {
                cur.offset
            };

            if swap_idx > 0 && swap_idx < self.buffer.len() {
                let c1 = self.buffer.chars()[swap_idx - 1];
                let c2 = self.buffer.chars()[swap_idx];
                self.buffer.chars[swap_idx - 1] = c2;
                self.buffer.chars[swap_idx] = c1;

                if cur.offset < self.buffer.len() {
                    cur.offset += 1;
                }
            }
            cur.desired_col = None;
        }

        self.cursors.normalize();
        self.sync_scroll();
    }

    /// Transpose words around cursor (`Alt+T`).
    pub fn transpose_words(&mut self) {
        self.history.record(&self.buffer, &self.cursors);
        self.last_action_was_kill = false;
        self.last_action_was_yank = false;

        let primary_off = self.cursors.primary().offset;
        let prev_start =
            LineJumpHelper::find_prev_word_boundary(self.buffer.chars(), primary_off, false);
        let prev_end =
            LineJumpHelper::find_next_word_boundary(self.buffer.chars(), prev_start, false);
        let next_start =
            LineJumpHelper::find_next_word_boundary(self.buffer.chars(), prev_end, false);
        let next_end =
            LineJumpHelper::find_next_word_boundary(self.buffer.chars(), next_start, false);

        if prev_start < prev_end && next_start < next_end && prev_end <= next_start {
            let word1 = self.buffer.slice(prev_start, prev_end);
            let word2 = self.buffer.slice(next_start, next_end);
            let separator = self.buffer.slice(prev_end, next_start);

            let combined = format!("{}{}{}", word2, separator, word1);
            self.buffer.replace_range(prev_start, next_end, &combined);
            self.cursors
                .reset_to_single(prev_start + combined.chars().count());
        }

        self.sync_scroll();
    }

    /// Apply word case transformation at all cursors (`Alt+U`, `Alt+L`, `Alt+C`).
    pub fn transform_word_case(&mut self, transform: CaseTransform) {
        self.history.record(&self.buffer, &self.cursors);
        self.last_action_was_kill = false;
        self.last_action_was_yank = false;

        let mut cursors_to_process = self.cursors.cursors().to_vec();
        cursors_to_process.sort_by_key(|c| std::cmp::Reverse(c.selection_start()));

        for cur in &mut cursors_to_process {
            let (start, end) = if cur.has_selection() {
                let sel = cur.selection();
                (sel.start(), sel.end())
            } else {
                let end =
                    LineJumpHelper::find_next_word_boundary(self.buffer.chars(), cur.offset, false);
                (cur.offset, end)
            };

            if start < end {
                BlockOperations::transform_case(&mut self.buffer, start, end, transform);
                cur.offset = end;
                cur.collapse();
            }
            cur.desired_col = None;
        }

        self.cursors = MultiCursorState {
            cursors: cursors_to_process,
            primary_idx: 0,
        };
        self.cursors.normalize();
        self.sync_scroll();
    }

    /// Paste text into buffer with multi-cursor intelligence and newline normalization.
    pub fn paste(&mut self, text: &str) {
        let cleaned = text
            .replace("\x1b[200~", "")
            .replace("\x1b[201~", "")
            .replace("\r\n", "\n")
            .replace('\r', "\n");

        self.history.record(&self.buffer, &self.cursors);
        self.last_action_was_kill = false;
        self.last_action_was_yank = false;

        let num_cursors = self.cursors.count();
        let lines: Vec<&str> = cleaned.lines().collect();

        let mut cursors_to_process = self.cursors.cursors().to_vec();
        cursors_to_process.sort_by_key(|c| std::cmp::Reverse(c.selection_start()));

        for (idx, cur) in cursors_to_process.iter_mut().enumerate() {
            if cur.has_selection() {
                let sel = cur.selection();
                self.buffer.delete_range(sel.start(), sel.end());
                cur.offset = sel.start();
                cur.collapse();
            }

            let piece = if num_cursors > 1 && lines.len() == num_cursors {
                let line_idx = num_cursors.saturating_sub(1) - idx;
                lines.get(line_idx).copied().unwrap_or(&cleaned)
            } else {
                &cleaned
            };

            let piece_len = piece.chars().count();
            self.buffer.insert_str(cur.offset, piece);
            cur.offset += piece_len;
            cur.desired_col = None;
        }

        self.cursors = MultiCursorState {
            cursors: cursors_to_process,
            primary_idx: 0,
        };
        self.cursors.normalize();
        self.sync_scroll();
    }

    /// Add a cursor on the line immediately above the primary cursor.
    pub fn add_cursor_above(&mut self) {
        self.cursors.add_cursor_above(&self.buffer);
        self.sync_scroll();
    }

    /// Add a cursor on the line immediately below the primary cursor.
    pub fn add_cursor_below(&mut self) {
        self.cursors.add_cursor_below(&self.buffer);
        self.sync_scroll();
    }

    /// Add a cursor at the next match of current selection or word under cursor.
    pub fn add_cursor_at_next_match(&mut self, case_sensitive: bool) -> bool {
        let res = self
            .cursors
            .add_cursor_at_next_match(&self.buffer, case_sensitive);
        self.sync_scroll();
        res
    }

    /// Select all occurrences of current selection across entire buffer.
    pub fn select_all_matches(&mut self, case_sensitive: bool) -> usize {
        let count = self
            .cursors
            .select_all_matches(&self.buffer, case_sensitive);
        self.sync_scroll();
        count
    }

    /// Split a multiline selection into line-by-line cursors.
    pub fn split_selection_into_lines(&mut self) {
        self.cursors.split_selection_into_lines(&self.buffer);
        self.sync_scroll();
    }

    /// Clear all secondary cursors, keeping only the primary cursor (`Escape`).
    pub fn clear_secondary_cursors(&mut self) {
        self.cursors.clear_secondary();
        self.sync_scroll();
    }

    /// Select entire buffer content.
    pub fn select_all(&mut self) {
        self.cursors.reset_to_single(self.buffer.len());
        self.cursors.primary_mut().anchor = Some(0);
        self.sync_scroll();
    }

    /// Jump primary cursor to destination.
    pub fn jump(&mut self, target: LineJumpTarget, preserve_col: bool) {
        let new_off = LineJumpHelper::calculate_jump(
            &self.buffer,
            self.cursors.primary().offset,
            target,
            preserve_col,
        );
        self.cursors.reset_to_single(new_off);
        self.sync_scroll();
    }

    /// Re-wrap current buffer or paragraph to specified column width.
    pub fn rewrap(&mut self, width: usize) {
        self.history.record(&self.buffer, &self.cursors);
        let rewrapped = WordWrapEngine::rewrap_paragraph(&self.buffer.text(), width);
        self.buffer.set_text(&rewrapped);
        self.cursors
            .reset_to_single(min(self.cursors.primary().offset, self.buffer.len()));
        self.sync_scroll();
    }

    /// Undo last edit.
    pub fn undo(&mut self) -> bool {
        let res = self.history.undo(&mut self.buffer, &mut self.cursors);
        if res {
            self.last_action_was_kill = false;
            self.last_action_was_yank = false;
            self.sync_scroll();
        }
        res
    }

    /// Redo last undone edit.
    pub fn redo(&mut self) -> bool {
        let res = self.history.redo(&mut self.buffer, &mut self.cursors);
        if res {
            self.last_action_was_kill = false;
            self.last_action_was_yank = false;
            self.sync_scroll();
        }
        res
    }

    /// Check if undo is available.
    pub fn can_undo(&self) -> bool {
        self.history.can_undo()
    }

    /// Check if redo is available.
    pub fn can_redo(&self) -> bool {
        self.history.can_redo()
    }

    /// Clear all text and reset cursor to origin.
    pub fn clear(&mut self) {
        self.history.record(&self.buffer, &self.cursors);
        self.buffer.clear();
        self.cursors.reset_to_single(0);
        self.last_action_was_kill = false;
        self.last_action_was_yank = false;
        self.sync_scroll();
    }

    /// Execute a [`ReadlineCommand`] on the buffer.
    pub fn execute_command(&mut self, cmd: &ReadlineCommand) -> bool {
        match cmd {
            ReadlineCommand::InsertChar(c) => {
                self.insert_char(*c);
                true
            }
            ReadlineCommand::InsertStr(s) => {
                self.insert_str(s);
                true
            }
            ReadlineCommand::InsertNewline { auto_indent } => {
                self.insert_newline(*auto_indent);
                true
            }
            ReadlineCommand::Backspace => {
                self.backspace();
                true
            }
            ReadlineCommand::Delete => {
                self.delete();
                true
            }
            ReadlineCommand::MoveCharLeft { extend_selection } => {
                self.move_char_backward(*extend_selection);
                true
            }
            ReadlineCommand::MoveCharRight { extend_selection } => {
                self.move_char_forward(*extend_selection);
                true
            }
            ReadlineCommand::MoveWordLeft { extend_selection } => {
                self.move_word_backward(*extend_selection);
                true
            }
            ReadlineCommand::MoveWordRight { extend_selection } => {
                self.move_word_forward(*extend_selection);
                true
            }
            ReadlineCommand::MoveLineStart { extend_selection } => {
                self.move_line_start(*extend_selection);
                true
            }
            ReadlineCommand::MoveLineEnd { extend_selection } => {
                self.move_line_end(*extend_selection);
                true
            }
            ReadlineCommand::MoveLineUp { extend_selection } => {
                self.move_line_up(*extend_selection);
                true
            }
            ReadlineCommand::MoveLineDown { extend_selection } => {
                self.move_line_down(*extend_selection);
                true
            }
            ReadlineCommand::MoveBufferStart { extend_selection } => {
                self.move_buffer_start(*extend_selection);
                true
            }
            ReadlineCommand::MoveBufferEnd { extend_selection } => {
                self.move_buffer_end(*extend_selection);
                true
            }
            ReadlineCommand::KillLineToEnd => {
                self.kill_line_to_end();
                true
            }
            ReadlineCommand::KillLineToStart => {
                self.kill_line_to_start();
                true
            }
            ReadlineCommand::KillWordBackward => {
                self.kill_word_backward();
                true
            }
            ReadlineCommand::KillWordForward => {
                self.kill_word_forward();
                true
            }
            ReadlineCommand::Yank => {
                self.yank();
                true
            }
            ReadlineCommand::YankPop => {
                self.yank_pop();
                true
            }
            ReadlineCommand::TransposeChars => {
                self.transpose_chars();
                true
            }
            ReadlineCommand::TransposeWords => {
                self.transpose_words();
                true
            }
            ReadlineCommand::TransformWord(t) => {
                self.transform_word_case(*t);
                true
            }
            ReadlineCommand::Undo => self.undo(),
            ReadlineCommand::Redo => self.redo(),
            ReadlineCommand::AddCursorAbove => {
                self.add_cursor_above();
                true
            }
            ReadlineCommand::AddCursorBelow => {
                self.add_cursor_below();
                true
            }
            ReadlineCommand::AddCursorNextMatch { case_sensitive } => {
                self.add_cursor_at_next_match(*case_sensitive)
            }
            ReadlineCommand::SelectAllMatches { case_sensitive } => {
                self.select_all_matches(*case_sensitive) > 0
            }
            ReadlineCommand::SplitSelectionLines => {
                self.split_selection_into_lines();
                true
            }
            ReadlineCommand::ClearSecondaryCursors => {
                self.clear_secondary_cursors();
                true
            }
            ReadlineCommand::SelectAll => {
                self.select_all();
                true
            }
            ReadlineCommand::Paste(s) => {
                self.paste(s);
                true
            }
            ReadlineCommand::Rewrap(w) => {
                self.rewrap(*w);
                true
            }
            ReadlineCommand::NoOp => false,
        }
    }

    /// Process a [`ReadlineKey`] and return the resulting [`ReadlineActionOutcome`].
    pub fn execute_key(&mut self, key: &ReadlineKey) -> ReadlineActionOutcome {
        if *key == ReadlineKey::Enter {
            return ReadlineActionOutcome::Submitted;
        }
        if *key == ReadlineKey::Escape && !self.cursors.is_multi() {
            return ReadlineActionOutcome::Canceled;
        }

        let cmd = key.to_command();
        if cmd == ReadlineCommand::NoOp {
            return ReadlineActionOutcome::NoOp;
        }

        let old_text = self.buffer.text();
        let old_offset = self.cursors.primary().offset;

        let executed = self.execute_command(&cmd);
        if !executed {
            return ReadlineActionOutcome::NoOp;
        }

        if self.buffer.text() != old_text {
            ReadlineActionOutcome::Modified
        } else if self.cursors.primary().offset != old_offset {
            ReadlineActionOutcome::Moved
        } else {
            ReadlineActionOutcome::NoOp
        }
    }

    /// Render visible text slice and screen cursor column for the primary line.
    pub fn render_prompt_line(&self) -> (String, Option<usize>) {
        let pos = self.cursor_position();
        let line_text = self.buffer.line_text(pos.line).unwrap_or_default();
        self.horizontal_scroll
            .render_line_with_scroll(&line_text, Some(pos.col))
    }
}

// ---------------------------------------------------------------------------
// Unit Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_multiline_buffer_coordinates() {
        let text = "hello\nworld\nfusion";
        let buf = MultilineBuffer::from_str(text);

        assert_eq!(buf.line_count(), 3);
        assert_eq!(buf.position_of(0), Position::new(0, 0));
        assert_eq!(buf.position_of(5), Position::new(0, 5));
        assert_eq!(buf.position_of(6), Position::new(1, 0)); // 'w'
        assert_eq!(buf.position_of(11), Position::new(1, 5));
        assert_eq!(buf.position_of(12), Position::new(2, 0)); // 'f'

        assert_eq!(buf.offset_of(Position::new(0, 0)), 0);
        assert_eq!(buf.offset_of(Position::new(1, 0)), 6);
        assert_eq!(buf.offset_of(Position::new(2, 2)), 14);
    }

    #[test]
    fn test_multiline_buffer_mutations() {
        let mut buf = MultilineBuffer::from_str("line1\nline3");
        buf.insert_str(6, "line2\n");
        assert_eq!(buf.text(), "line1\nline2\nline3");

        let deleted = buf.delete_range(6, 12);
        assert_eq!(deleted, "line2\n");
        assert_eq!(buf.text(), "line1\nline3");
    }

    #[test]
    fn test_multi_cursor_normalize_and_merge() {
        let mut state = MultiCursorState::new();
        state.add_cursor(Cursor::new(5));
        state.add_cursor(Cursor::new(2));
        state.add_cursor(Cursor::new(5)); // Duplicate offset

        assert_eq!(state.count(), 2);
        assert_eq!(state.cursors()[0].offset, 2);
        assert_eq!(state.cursors()[1].offset, 5);

        // Overlapping selections
        state.add_cursor(Cursor::with_selection(1, 4));
        assert_eq!(state.count(), 2);
    }

    #[test]
    fn test_multi_cursor_next_match() {
        let buf = MultilineBuffer::from_str("foo bar foo baz foo");
        let mut state = MultiCursorState::with_offset(0);

        // Select first foo
        state.primary_mut().anchor = Some(0);
        state.primary_mut().offset = 3;

        assert!(state.add_cursor_at_next_match(&buf, true));
        assert_eq!(state.count(), 2);

        assert!(state.add_cursor_at_next_match(&buf, true));
        assert_eq!(state.count(), 3);
    }

    #[test]
    fn test_select_all_matches() {
        let buf = MultilineBuffer::from_str("let x = 1;\nlet y = 2;\nlet z = 3;");
        let mut state = MultiCursorState::new();
        state.primary_mut().anchor = Some(0);
        state.primary_mut().offset = 3; // "let"

        let count = state.select_all_matches(&buf, true);
        assert_eq!(count, 3);
        assert_eq!(state.count(), 3);
    }

    #[test]
    fn test_word_wrapping_basic() {
        let text = "The quick brown fox jumps over the lazy dog";
        let opts = WrapOptions::new(15);
        let wrapped = WordWrapEngine::wrap_line(text, &opts);

        assert_eq!(wrapped.len(), 3);
        assert_eq!(wrapped[0], "The quick brown");
        assert_eq!(wrapped[1], "fox jumps over");
        assert_eq!(wrapped[2], "the lazy dog");
    }

    #[test]
    fn test_word_wrapping_buffer_mapping() {
        let text = "line one is very long and should wrap\nline two";
        let buf = MultilineBuffer::from_str(text);
        let opts = WrapOptions::new(15);
        let wrapped = WordWrapEngine::wrap_buffer(&buf, &opts);

        assert!(wrapped.len() >= 3);
        let off = WordWrapEngine::visual_to_buffer_offset(0, 4, &wrapped);
        assert_eq!(off, 4);
    }

    #[test]
    fn test_rewrap_paragraph_with_prefix() {
        let para = "// The quick brown fox\n// jumps over the\n// lazy dog";
        let rewrapped = WordWrapEngine::rewrap_paragraph(para, 30);
        assert!(rewrapped.starts_with("// "));
        for line in rewrapped.lines() {
            assert!(line.starts_with("// "));
        }
    }

    #[test]
    fn test_line_jump_targets() {
        let text = "first line\n  indented line\nthird line";
        let buf = MultilineBuffer::from_str(text);

        let bol = LineJumpHelper::calculate_jump(&buf, 15, LineJumpTarget::BeginningOfLine, false);
        assert_eq!(bol, 11);

        let fnw =
            LineJumpHelper::calculate_jump(&buf, 11, LineJumpTarget::FirstNonWhitespace, false);
        assert_eq!(fnw, 13); // After two spaces

        let last = LineJumpHelper::calculate_jump(&buf, 0, LineJumpTarget::LastLine, false);
        assert_eq!(last, 27);
    }

    #[test]
    fn test_bracket_matching() {
        let text: Vec<char> = "fn foo(a: [i32; 4]) { let x = (1 + 2); }".chars().collect();

        assert_eq!(LineJumpHelper::find_matching_bracket(&text, 6), Some(18)); // ( ... )
        assert_eq!(LineJumpHelper::find_matching_bracket(&text, 10), Some(17)); // [ ... ]
        assert_eq!(LineJumpHelper::find_matching_bracket(&text, 20), Some(40)); // { ... }
    }

    #[test]
    fn test_subword_navigation() {
        let text: Vec<char> = "camelCaseIdentifier_snake_case".chars().collect();

        let p1 = LineJumpHelper::find_next_word_boundary(&text, 0, true);
        assert_eq!(p1, 5); // at 'C'

        let p2 = LineJumpHelper::find_next_word_boundary(&text, 5, true);
        assert_eq!(p2, 9); // at 'I'
    }

    #[test]
    fn test_block_indent_and_outdent() {
        let mut buf = MultilineBuffer::from_str("line1\nline2\nline3");
        BlockOperations::indent_lines(&mut buf, 0, 1, "  ");
        assert_eq!(buf.text(), "  line1\n  line2\nline3");

        BlockOperations::outdent_lines(&mut buf, 0, 1, 2);
        assert_eq!(buf.text(), "line1\nline2\nline3");
    }

    #[test]
    fn test_toggle_comment() {
        let mut buf = MultilineBuffer::from_str("fn foo() {\n    let x = 1;\n}");
        BlockOperations::toggle_comment(&mut buf, 0, 2, "//");
        assert_eq!(buf.text(), "// fn foo() {\n//     let x = 1;\n// }");

        BlockOperations::toggle_comment(&mut buf, 0, 2, "//");
        assert_eq!(buf.text(), "fn foo() {\n    let x = 1;\n}");
    }

    #[test]
    fn test_move_lines() {
        let mut buf = MultilineBuffer::from_str("A\nB\nC");
        assert!(BlockOperations::move_lines_down(&mut buf, 0, 0));
        assert_eq!(buf.text(), "B\nA\nC");

        assert!(BlockOperations::move_lines_up(&mut buf, 1, 1));
        assert_eq!(buf.text(), "A\nB\nC");
    }

    #[test]
    fn test_sort_and_align_block() {
        let mut buf = MultilineBuffer::from_str("banana\napple\ncherry");
        BlockOperations::sort_lines(&mut buf, 0, 2, SortOptions::default());
        assert_eq!(buf.text(), "apple\nbanana\ncherry");

        let mut align_buf = MultilineBuffer::from_str(
            "name = \"fusion\"\nversion = \"1.0\"\ndescription = \"assistant\"",
        );
        BlockOperations::align_column_by_delimiter(&mut align_buf, 0, 2, '=');
        let lines = align_buf.lines();
        assert_eq!(lines[0], "name        = \"fusion\"");
        assert_eq!(lines[1], "version     = \"1.0\"");
        assert_eq!(lines[2], "description = \"assistant\"");
    }

    #[test]
    fn test_editor_buffer_multicursor_typing_and_undo() {
        let mut editor = EditorBuffer::from_str("foo\nfoo");
        editor.cursors.reset_to_single(0);
        editor.cursors.primary_mut().anchor = Some(0);
        editor.cursors.primary_mut().offset = 3;
        editor.cursors.select_all_matches(&editor.buffer, true);

        editor.insert_str("bar");
        assert_eq!(editor.buffer.text(), "bar\nbar");

        assert!(editor.undo());
        assert_eq!(editor.buffer.text(), "foo\nfoo");

        assert!(editor.redo());
        assert_eq!(editor.buffer.text(), "bar\nbar");
    }

    #[test]
    fn test_readline_keybindings_ctrl_a_e_k_u_w_y() {
        let mut editor = EditorBuffer::from_str("hello wonderful world");
        assert_eq!(editor.cursor_offset(), 21);

        // Ctrl+A: beginning of line
        editor.execute_key(&ReadlineKey::Ctrl('a'));
        assert_eq!(editor.cursor_offset(), 0);

        // Alt+F: forward word -> "hello" (offset 6)
        editor.execute_key(&ReadlineKey::Alt('f'));
        assert_eq!(editor.cursor_offset(), 6);

        // Ctrl+K: kill to end of line
        editor.execute_key(&ReadlineKey::Ctrl('k'));
        assert_eq!(editor.text(), "hello ");
        assert_eq!(editor.kill_ring.yank(), Some("wonderful world"));

        // Ctrl+Y: yank back
        editor.execute_key(&ReadlineKey::Ctrl('y'));
        assert_eq!(editor.text(), "hello wonderful world");

        // Alt+B: backward word -> at start of "world" (offset 16)
        editor.execute_key(&ReadlineKey::Alt('b'));
        assert_eq!(editor.cursor_offset(), 16);

        // Ctrl+W: kill word backward -> deletes "wonderful "
        editor.execute_key(&ReadlineKey::Ctrl('w'));
        assert_eq!(editor.text(), "hello world");
        assert_eq!(editor.cursor_offset(), 6);

        // Ctrl+E: end of line
        editor.execute_key(&ReadlineKey::Ctrl('e'));
        assert_eq!(editor.cursor_offset(), 11);

        // Ctrl+U: kill to start of line
        editor.execute_key(&ReadlineKey::Ctrl('u'));
        assert_eq!(editor.text(), "");
        assert_eq!(editor.cursor_offset(), 0);

        // Ctrl+Y: yank whole line back
        editor.execute_key(&ReadlineKey::Ctrl('y'));
        assert_eq!(editor.text(), "hello world");
    }

    #[test]
    fn test_readline_kill_ring_consecutive_append_and_yank_pop() {
        let mut editor = EditorBuffer::from_str("line1\nline2\nline3");
        editor.cursors.reset_to_single(0);

        // First Ctrl+K kills "line1"
        editor.execute_key(&ReadlineKey::Ctrl('k'));
        assert_eq!(editor.text(), "\nline2\nline3");

        // Consecutive Ctrl+K kills "\n" (appended to kill ring)
        editor.execute_key(&ReadlineKey::Ctrl('k'));
        assert_eq!(editor.text(), "line2\nline3");

        // Third Ctrl+K kills "line2" (appended to kill ring)
        editor.execute_key(&ReadlineKey::Ctrl('k'));
        assert_eq!(editor.text(), "\nline3");

        assert_eq!(editor.kill_ring.yank(), Some("line1\nline2"));

        // Yank restored the entire combined span
        editor.execute_key(&ReadlineKey::Ctrl('y'));
        assert_eq!(editor.text(), "line1\nline2\nline3");

        // Test yank-pop cycling
        let mut kr = KillRing::new(10);
        kr.push("first");
        kr.push("second");
        kr.push("third");

        assert_eq!(kr.yank(), Some("third"));
        assert_eq!(kr.yank_pop(), Some("second"));
        assert_eq!(kr.yank_pop(), Some("first"));
        assert_eq!(kr.yank_pop(), Some("third"));
    }

    #[test]
    fn test_readline_transposition_and_case_transform() {
        let mut editor = EditorBuffer::from_str("abc");
        editor.cursors.reset_to_single(1);

        // Ctrl+T: transpose 'a' and 'b' -> "bac"
        editor.execute_key(&ReadlineKey::Ctrl('t'));
        assert_eq!(editor.text(), "bac");
        assert_eq!(editor.cursor_offset(), 2);

        // Word case transform (Alt+U, Alt+L, Alt+C)
        let mut case_editor = EditorBuffer::from_str("hello world");
        case_editor.cursors.reset_to_single(0);

        case_editor.execute_key(&ReadlineKey::Alt('u'));
        assert_eq!(case_editor.text(), "HELLO world");

        case_editor.cursors.reset_to_single(6);
        case_editor.execute_key(&ReadlineKey::Alt('c'));
        assert_eq!(case_editor.text(), "HELLO World");
    }

    #[test]
    fn test_horizontal_scroll_windowing() {
        let mut scroll = HorizontalScrollState::new(10, 2);
        let line = "0123456789ABCDEFGHIJKLMNOP";

        assert_eq!(scroll.visible_slice(line), "0123456789");
        assert_eq!(scroll.cursor_screen_col(0), Some(0));
        assert_eq!(scroll.cursor_screen_col(5), Some(5));
        assert_eq!(scroll.cursor_screen_col(12), None);

        // Move cursor right past margin
        scroll.ensure_visible(12);
        assert!(scroll.scroll_offset > 0);
        let screen_col = scroll.cursor_screen_col(12);
        assert!(screen_col.is_some());

        // Render with indicators
        let rendered = scroll.render_with_indicators(line, "<", ">");
        assert!(rendered.starts_with('<'));
    }

    #[test]
    fn test_multicursor_paste_distribution() {
        let mut editor = EditorBuffer::from_str("item1\nitem2\nitem3");
        editor.cursors.reset_to_single(0);
        editor.add_cursor_below();
        editor.add_cursor_below();
        assert_eq!(editor.cursors.count(), 3);

        // Pasting 3 lines into 3 cursors distributes 1 line per cursor
        editor.paste("A\nB\nC");
        assert_eq!(editor.text(), "Aitem1\nBitem2\nCitem3");

        // Undo atomically restores original state
        assert!(editor.undo());
        assert_eq!(editor.text(), "item1\nitem2\nitem3");
    }

    #[test]
    fn test_multicursor_line_splitting_and_navigation() {
        let mut editor = EditorBuffer::from_str("alpha\nbeta\ngamma");
        editor.select_all();
        editor.split_selection_into_lines();

        assert_eq!(editor.cursors.count(), 3);

        // Move to start of lines
        editor.move_line_start(false);
        editor.insert_str("# ");
        assert_eq!(editor.text(), "# alpha\n# beta\n# gamma");

        // Move to end of lines
        editor.move_line_end(false);
        editor.insert_str(" !");
        assert_eq!(editor.text(), "# alpha !\n# beta !\n# gamma !");
    }
}
