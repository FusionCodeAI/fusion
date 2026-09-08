//! Interactive Hunk-by-Hunk Diff Reviewer.
//!
//! Provides a TUI widget and review session model for reviewing code changes
//! hunk-by-hunk, with support for individual acceptance, rejection, bulk decisions,
//! navigation, and generating unified patches from accepted hunks.

use crossterm::{
    cursor,
    event::{self, Event, KeyCode, KeyEventKind, KeyModifiers},
    execute,
    terminal::{self, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{
    backend::CrosstermBackend,
    buffer::Buffer,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph, Widget},
    Terminal,
};
use serde::{Deserialize, Serialize};
use std::fmt;
use std::io::stdout;

// ============================================================================
// Review State & Change Types
// ============================================================================

/// Review decision state for a single diff hunk.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
pub enum DiffHunkReviewState {
    /// Decision pending (default state).
    #[default]
    Pending,
    /// Hunk has been accepted.
    Accepted,
    /// Hunk has been rejected.
    Rejected,
}

impl DiffHunkReviewState {
    pub fn is_pending(&self) -> bool {
        matches!(self, Self::Pending)
    }

    pub fn is_accepted(&self) -> bool {
        matches!(self, Self::Accepted)
    }

    pub fn is_rejected(&self) -> bool {
        matches!(self, Self::Rejected)
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Pending => "PENDING",
            Self::Accepted => "ACCEPTED",
            Self::Rejected => "REJECTED",
        }
    }

    pub fn badge_label(&self) -> &'static str {
        match self {
            Self::Pending => "[PENDING]",
            Self::Accepted => "[ACCEPTED]",
            Self::Rejected => "[REJECTED]",
        }
    }
}

impl fmt::Display for DiffHunkReviewState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

/// The nature of a single line change within a hunk.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
pub enum ChangeKind {
    /// Unchanged context line.
    #[default]
    Context,
    /// Added line (+).
    Addition,
    /// Deleted line (-).
    Deletion,
}

impl ChangeKind {
    pub const Added: ChangeKind = ChangeKind::Addition;
    pub const Deleted: ChangeKind = ChangeKind::Deletion;
    pub const Unchanged: ChangeKind = ChangeKind::Context;

    pub fn prefix(&self) -> char {
        match self {
            Self::Context => ' ',
            Self::Addition => '+',
            Self::Deletion => '-',
        }
    }

    pub fn is_addition(&self) -> bool {
        matches!(self, Self::Addition)
    }

    pub fn is_deletion(&self) -> bool {
        matches!(self, Self::Deletion)
    }

    pub fn is_context(&self) -> bool {
        matches!(self, Self::Context)
    }
}

impl fmt::Display for ChangeKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Context => write!(f, "context"),
            Self::Addition => write!(f, "addition"),
            Self::Deletion => write!(f, "deletion"),
        }
    }
}

// ============================================================================
// Review Hunk
// ============================================================================

/// A single hunk within a file diff pending or completing review.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReviewHunk {
    pub file_path: String,
    pub old_start: usize,
    pub old_len: usize,
    pub new_start: usize,
    pub new_len: usize,
    pub lines: Vec<(ChangeKind, String)>,
    pub state: DiffHunkReviewState,
}

impl ReviewHunk {
    /// Create a new review hunk in the `Pending` state.
    pub fn new(
        file_path: impl Into<String>,
        old_start: usize,
        old_len: usize,
        new_start: usize,
        new_len: usize,
        lines: Vec<(ChangeKind, String)>,
    ) -> Self {
        Self {
            file_path: file_path.into(),
            old_start,
            old_len,
            new_start,
            new_len,
            lines,
            state: DiffHunkReviewState::Pending,
        }
    }

    /// Format standard unified diff hunk header `@@ -old_start,old_len +new_start,new_len @@`.
    pub fn header(&self) -> String {
        let old_range = match self.old_len {
            1 => format!("-{}", self.old_start),
            _ => format!("-{},{}", self.old_start, self.old_len),
        };
        let new_range = match self.new_len {
            1 => format!("+{}", self.new_start),
            _ => format!("+{},{}", self.new_start, self.new_len),
        };
        format!("@@ {old_range} {new_range} @@")
    }

    /// Format this hunk into unified diff text.
    pub fn format_unified(&self) -> String {
        let mut out = format!("{}\n", self.header());
        for (kind, line) in &self.lines {
            let clean = line.trim_end_matches(['\r', '\n']);
            out.push(kind.prefix());
            out.push_str(clean);
            out.push('\n');
        }
        out
    }

    /// Count added lines in this hunk.
    pub fn additions_count(&self) -> usize {
        self.lines
            .iter()
            .filter(|(k, _)| *k == ChangeKind::Addition)
            .count()
    }

    /// Count deleted lines in this hunk.
    pub fn deletions_count(&self) -> usize {
        self.lines
            .iter()
            .filter(|(k, _)| *k == ChangeKind::Deletion)
            .count()
    }
}

// ============================================================================
// Review Session
// ============================================================================

/// An interactive review session tracking all hunks across files and active index.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct ReviewSession {
    pub hunks: Vec<ReviewHunk>,
    pub active_index: usize,
}

impl ReviewSession {
    /// Create a new session with the given hunks.
    pub fn new(hunks: Vec<ReviewHunk>) -> Self {
        Self {
            hunks,
            active_index: 0,
        }
    }

    /// Parse a unified diff string into a `ReviewSession`.
    pub fn parse_unified_diff(diff_text: &str) -> Self {
        let mut hunks = Vec::new();
        let mut current_old_path: Option<String> = None;
        let mut current_new_path: Option<String> = None;
        let mut current_file_path: Option<String> = None;

        struct OpenHunk {
            file_path: String,
            old_start: usize,
            old_len: usize,
            new_start: usize,
            new_len: usize,
            lines: Vec<(ChangeKind, String)>,
        }

        let mut open_hunk: Option<OpenHunk> = None;

        let flush_open_hunk = |open: &mut Option<OpenHunk>, dest: &mut Vec<ReviewHunk>| {
            if let Some(h) = open.take() {
                dest.push(ReviewHunk {
                    file_path: h.file_path,
                    old_start: h.old_start,
                    old_len: h.old_len,
                    new_start: h.new_start,
                    new_len: h.new_len,
                    lines: h.lines,
                    state: DiffHunkReviewState::Pending,
                });
            }
        };

        let lines: Vec<&str> = diff_text.lines().collect();
        let mut idx = 0;

        while idx < lines.len() {
            let line = lines[idx];

            // 1. git diff header
            if line.starts_with("diff --git ") {
                flush_open_hunk(&mut open_hunk, &mut hunks);
                if let Some((old_p, new_p)) = parse_git_diff_paths(line) {
                    current_old_path = Some(old_p);
                    current_new_path = Some(new_p);
                    current_file_path = Some(resolve_target_file_path(
                        current_old_path.as_deref(),
                        current_new_path.as_deref(),
                    ));
                }
                idx += 1;
                continue;
            }

            // 2. file header `--- old_path`
            // If open_hunk is active, verify if this `--- ` is a genuine file header
            // (typically followed by `+++ ` on the next or subsequent line), not code deletion.
            let is_genuine_file_header_old = if line.starts_with("--- ") {
                if open_hunk.is_some() {
                    idx + 1 < lines.len() && lines[idx + 1].starts_with("+++ ")
                } else {
                    true
                }
            } else {
                false
            };

            if is_genuine_file_header_old {
                flush_open_hunk(&mut open_hunk, &mut hunks);
                let raw_path = line[4..].trim();
                let clean = clean_path_spec(raw_path);
                current_old_path = Some(clean);
                current_file_path = Some(resolve_target_file_path(
                    current_old_path.as_deref(),
                    current_new_path.as_deref(),
                ));
                idx += 1;
                continue;
            }

            // 3. file header `+++ new_path`
            if line.starts_with("+++ ") {
                flush_open_hunk(&mut open_hunk, &mut hunks);
                let raw_path = line[4..].trim();
                let clean = clean_path_spec(raw_path);
                current_new_path = Some(clean);
                current_file_path = Some(resolve_target_file_path(
                    current_old_path.as_deref(),
                    current_new_path.as_deref(),
                ));
                idx += 1;
                continue;
            }

            // 4. hunk header `@@ -old_start[,old_len] +new_start[,new_len] @@`
            if line.starts_with("@@ ") || line.starts_with("@@-") {
                flush_open_hunk(&mut open_hunk, &mut hunks);

                if let Some((old_start, old_len, new_start, new_len)) =
                    parse_hunk_coordinates(line)
                {
                    let file_path = current_file_path
                        .clone()
                        .unwrap_or_else(|| "unknown".to_string());
                    open_hunk = Some(OpenHunk {
                        file_path,
                        old_start,
                        old_len,
                        new_start,
                        new_len,
                        lines: Vec::new(),
                    });
                }
                idx += 1;
                continue;
            }

            // 5. lines within an open hunk
            if let Some(hunk) = &mut open_hunk {
                if line.starts_with('+') {
                    hunk.lines
                        .push((ChangeKind::Addition, line[1..].to_string()));
                } else if line.starts_with('-') {
                    hunk.lines
                        .push((ChangeKind::Deletion, line[1..].to_string()));
                } else if line.starts_with(' ') {
                    hunk.lines
                        .push((ChangeKind::Context, line[1..].to_string()));
                } else if line.is_empty() {
                    // Empty context line (some formatters omit the leading space)
                    hunk.lines
                        .push((ChangeKind::Context, String::new()));
                } else if line.starts_with('\\') {
                    // Ignore metadata lines like `\ No newline at end of file`
                } else {
                    // Stray or unexpected non-hunk content marks the end of the hunk
                    flush_open_hunk(&mut open_hunk, &mut hunks);
                }
            }

            idx += 1;
        }

        flush_open_hunk(&mut open_hunk, &mut hunks);

        Self {
            hunks,
            active_index: 0,
        }
    }

    /// Mark the current active hunk as `Accepted`.
    pub fn accept_current(&mut self) {
        if let Some(hunk) = self.hunks.get_mut(self.active_index) {
            hunk.state = DiffHunkReviewState::Accepted;
        }
    }

    /// Mark the current active hunk as `Rejected`.
    pub fn reject_current(&mut self) {
        if let Some(hunk) = self.hunks.get_mut(self.active_index) {
            hunk.state = DiffHunkReviewState::Rejected;
        }
    }

    /// Mark all hunks in this session as `Accepted`.
    pub fn accept_all(&mut self) {
        for hunk in &mut self.hunks {
            hunk.state = DiffHunkReviewState::Accepted;
        }
    }

    /// Mark all hunks in this session as `Rejected`.
    pub fn reject_all(&mut self) {
        for hunk in &mut self.hunks {
            hunk.state = DiffHunkReviewState::Rejected;
        }
    }

    /// Navigate to the next hunk if available.
    pub fn next_hunk(&mut self) {
        if !self.hunks.is_empty() && self.active_index + 1 < self.hunks.len() {
            self.active_index += 1;
        }
    }

    /// Navigate to the previous hunk if available.
    pub fn prev_hunk(&mut self) {
        if self.active_index > 0 {
            self.active_index -= 1;
        }
    }

    /// Reference to the currently active hunk, if any.
    pub fn current_hunk(&self) -> Option<&ReviewHunk> {
        self.hunks.get(self.active_index)
    }

    /// Mutable reference to the currently active hunk, if any.
    pub fn current_hunk_mut(&mut self) -> Option<&mut ReviewHunk> {
        self.hunks.get_mut(self.active_index)
    }

    /// Total number of hunks in this session.
    pub fn len(&self) -> usize {
        self.hunks.len()
    }

    /// Returns true if there are no hunks to review.
    pub fn is_empty(&self) -> bool {
        self.hunks.is_empty()
    }

    /// Summary of review progress: `(total, accepted, rejected)`.
    pub fn summary(&self) -> (usize, usize, usize) {
        let total = self.hunks.len();
        let accepted = self
            .hunks
            .iter()
            .filter(|h| h.state == DiffHunkReviewState::Accepted)
            .count();
        let rejected = self
            .hunks
            .iter()
            .filter(|h| h.state == DiffHunkReviewState::Rejected)
            .count();
        (total, accepted, rejected)
    }

    /// Number of hunks still pending review.
    pub fn pending_count(&self) -> usize {
        self.hunks
            .iter()
            .filter(|h| h.state == DiffHunkReviewState::Pending)
            .count()
    }

    /// Number of accepted hunks.
    pub fn accepted_count(&self) -> usize {
        self.hunks
            .iter()
            .filter(|h| h.state == DiffHunkReviewState::Accepted)
            .count()
    }

    /// Number of rejected hunks.
    pub fn rejected_count(&self) -> usize {
        self.hunks
            .iter()
            .filter(|h| h.state == DiffHunkReviewState::Rejected)
            .count()
    }

    /// Generate unified patch text containing only the accepted hunks.
    pub fn generate_accepted_patch(&self) -> String {
        // Collect files in order of appearance that have at least one accepted hunk
        let mut ordered_files = Vec::new();
        for hunk in &self.hunks {
            if hunk.state == DiffHunkReviewState::Accepted
                && !ordered_files.contains(&hunk.file_path)
            {
                ordered_files.push(hunk.file_path.clone());
            }
        }

        if ordered_files.is_empty() {
            return String::new();
        }

        let mut patch = String::new();

        for file_path in ordered_files {
            let file_hunks: Vec<&ReviewHunk> = self
                .hunks
                .iter()
                .filter(|h| h.file_path == file_path && h.state == DiffHunkReviewState::Accepted)
                .collect();

            if file_hunks.is_empty() {
                continue;
            }

            let is_all_new = file_hunks
                .iter()
                .all(|h| h.old_start == 0 && h.old_len == 0);
            let is_all_deleted = file_hunks
                .iter()
                .all(|h| h.new_start == 0 && h.new_len == 0);

            let old_prefix = if is_all_new {
                "/dev/null".to_string()
            } else if file_path.starts_with("a/") {
                file_path.clone()
            } else {
                format!("a/{file_path}")
            };

            let new_prefix = if is_all_deleted {
                "/dev/null".to_string()
            } else if file_path.starts_with("b/") {
                file_path.clone()
            } else {
                format!("b/{file_path}")
            };

            patch.push_str(&format!("--- {old_prefix}\n+++ {new_prefix}\n"));

            for hunk in file_hunks {
                patch.push_str(&hunk.format_unified());
            }
        }

        patch
    }
}

// ============================================================================
// Parsing Helpers
// ============================================================================

/// Strip surrounding quotes, tabbed timestamps, and leading `a/` or `b/` prefixes.
fn clean_path_spec(raw: &str) -> String {
    let s = raw.trim();
    let s = s.split('\t').next().unwrap_or(s).trim();
    let s = if s.len() >= 2 && s.starts_with('"') && s.ends_with('"') {
        &s[1..s.len() - 1]
    } else {
        s
    };

    if let Some(stripped) = s.strip_prefix("a/") {
        stripped.to_string()
    } else if let Some(stripped) = s.strip_prefix("b/") {
        stripped.to_string()
    } else {
        s.to_string()
    }
}

/// Resolve clean target path, preferring `new_path` then `old_path` unless `/dev/null`.
fn resolve_target_file_path(old_p: Option<&str>, new_p: Option<&str>) -> String {
    if let Some(new_p) = new_p {
        let cleaned = clean_path_spec(new_p);
        if cleaned != "/dev/null" && !cleaned.is_empty() {
            return cleaned;
        }
    }
    if let Some(old_p) = old_p {
        let cleaned = clean_path_spec(old_p);
        if cleaned != "/dev/null" && !cleaned.is_empty() {
            return cleaned;
        }
    }
    "unknown".to_string()
}

/// Parse paths from `diff --git a/path b/path`.
fn parse_git_diff_paths(line: &str) -> Option<(String, String)> {
    let rest = line.strip_prefix("diff --git ")?.trim();
    let parts: Vec<&str> = rest.split_whitespace().collect();
    if parts.len() >= 2 {
        let old_p = clean_path_spec(parts[0]);
        let new_p = clean_path_spec(parts[1]);
        Some((old_p, new_p))
    } else {
        None
    }
}

/// Parse `(old_start, old_len, new_start, new_len)` from `@@ -A[,B] +C[,D] @@`.
fn parse_hunk_coordinates(line: &str) -> Option<(usize, usize, usize, usize)> {
    let trimmed = line.trim();
    let rest = trimmed.strip_prefix("@@")?.trim_start();
    let end_idx = rest.find("@@")?;
    let ranges_str = rest[..end_idx].trim();
    let mut parts = ranges_str.split_whitespace();

    let old_part = parts.next()?;
    let new_part = parts.next()?;

    let (old_start, old_len) = parse_single_range(old_part, '-')?;
    let (new_start, new_len) = parse_single_range(new_part, '+')?;

    Some((old_start, old_len, new_start, new_len))
}

/// Parse `-A[,B]` or `+C[,D]`. If length is omitted, defaults to 1.
fn parse_single_range(part: &str, prefix: char) -> Option<(usize, usize)> {
    let s = part.strip_prefix(prefix).unwrap_or(part);
    if let Some((start_s, len_s)) = s.split_once(',') {
        let start = start_s.parse::<usize>().ok()?;
        let len = len_s.parse::<usize>().ok()?;
        Some((start, len))
    } else {
        let start = s.parse::<usize>().ok()?;
        Some((start, 1))
    }
}

// ============================================================================
// Ratatui Widget Implementation
// ============================================================================

/// Ratatui widget that renders the current hunk review interface.
pub struct ReviewWidget<'a> {
    pub session: &'a ReviewSession,
}

impl<'a> ReviewWidget<'a> {
    pub fn new(session: &'a ReviewSession) -> Self {
        Self { session }
    }
}

impl<'a> Widget for ReviewWidget<'a> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        (&self).render(area, buf);
    }
}

impl<'a> Widget for &ReviewWidget<'a> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        if area.width == 0 || area.height == 0 {
            return;
        }

        // Handle empty session
        if self.session.hunks.is_empty() {
            let empty_text = vec![
                Line::from(""),
                Line::from(Span::styled(
                    " No diff hunks to review.",
                    Style::default().fg(Color::DarkGray),
                )),
                Line::from(""),
                Line::from(vec![
                    Span::styled("[q]", Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)),
                    Span::styled(" Finish / Quit", Style::default().fg(Color::White)),
                ]),
            ];
            let paragraph = Paragraph::new(empty_text).block(
                Block::default()
                    .borders(Borders::ALL)
                    .title(" Diff Review "),
            );
            paragraph.render(area, buf);
            return;
        }

        let hunk = &self.session.hunks[self.session.active_index];

        // Layout:
        // - Top bar (height: 3): file path, coordinates, counter, decision badge
        // - Middle content: hunk lines with +/- colors
        // - Bottom bar (height: 1): keybindings
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(3), // Header block
                Constraint::Min(3),    // Hunk content
                Constraint::Length(1), // Keybindings footer
            ])
            .split(area);

        // 1. Render Header Block
        let badge_style = match hunk.state {
            DiffHunkReviewState::Accepted => {
                Style::default().fg(Color::Green).add_modifier(Modifier::BOLD)
            }
            DiffHunkReviewState::Rejected => {
                Style::default().fg(Color::Red).add_modifier(Modifier::BOLD)
            }
            DiffHunkReviewState::Pending => {
                Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)
            }
        };

        let badge_span = Span::styled(format!(" {} ", hunk.state.badge_label()), badge_style);

        let (total, accepted, rejected) = self.session.summary();
        let pending = total.saturating_sub(accepted + rejected);

        let header_line = Line::from(vec![
            Span::styled(" File: ", Style::default().fg(Color::DarkGray)),
            Span::styled(
                &hunk.file_path,
                Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD),
            ),
            Span::styled("  ", Style::default()),
            Span::styled(
                hunk.header(),
                Style::default().fg(Color::Magenta).add_modifier(Modifier::BOLD),
            ),
            Span::styled("  ", Style::default()),
            Span::styled(
                format!(
                    "[{}/{}] (✓{} ✗{} ?{})",
                    self.session.active_index + 1,
                    total,
                    accepted,
                    rejected,
                    pending
                ),
                Style::default().fg(Color::DarkGray),
            ),
            Span::styled("   ", Style::default()),
            badge_span,
        ]);

        let header_paragraph = Paragraph::new(vec![header_line]).block(
            Block::default()
                .borders(Borders::ALL)
                .title(" Diff Hunk Review "),
        );
        header_paragraph.render(chunks[0], buf);

        // 2. Render Hunk Lines
        // Render additions in green (+), deletions in red (-), context in white ( )
        let mut display_lines = Vec::with_capacity(hunk.lines.len());

        for (kind, content) in &hunk.lines {
            let clean = content.trim_end_matches(['\r', '\n']);
            match kind {
                ChangeKind::Addition => {
                    display_lines.push(Line::from(Span::styled(
                        format!("+{clean}"),
                        Style::default().fg(Color::Green),
                    )));
                }
                ChangeKind::Deletion => {
                    display_lines.push(Line::from(Span::styled(
                        format!("-{clean}"),
                        Style::default().fg(Color::Red),
                    )));
                }
                ChangeKind::Context => {
                    display_lines.push(Line::from(Span::styled(
                        format!(" {clean}"),
                        Style::default().fg(Color::White),
                    )));
                }
            }
        }

        let content_block = Block::default()
            .borders(Borders::ALL)
            .title(format!(" Lines ({}) ", hunk.lines.len()));
        let content_paragraph = Paragraph::new(display_lines).block(content_block);
        content_paragraph.render(chunks[1], buf);

        // 3. Render Bottom Keybinding Bar
        // `[y] Accept  [n] Reject  [a] Accept All  [r] Reject All  [↑↓] Prev/Next  [q] Finish`
        let footer_spans = vec![
            Span::styled(" ", Style::default()),
            Span::styled("[y]", Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)),
            Span::styled(" Accept  ", Style::default().fg(Color::White)),
            Span::styled("[n]", Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)),
            Span::styled(" Reject  ", Style::default().fg(Color::White)),
            Span::styled("[a]", Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)),
            Span::styled(" Accept All  ", Style::default().fg(Color::White)),
            Span::styled("[r]", Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)),
            Span::styled(" Reject All  ", Style::default().fg(Color::White)),
            Span::styled("[↑↓]", Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)),
            Span::styled(" Prev/Next  ", Style::default().fg(Color::White)),
            Span::styled("[q]", Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)),
            Span::styled(" Finish", Style::default().fg(Color::White)),
        ];

        let footer_line = Line::from(footer_spans);
        Paragraph::new(footer_line).render(chunks[2], buf);
    }
}

// ============================================================================
// Interactive TUI Runner
// ============================================================================

/// RAII guard ensuring the terminal leaves alternate screen and restores cursor.
struct TerminalRestoreGuard;

impl Drop for TerminalRestoreGuard {
    fn drop(&mut self) {
        let _ = execute!(stdout(), cursor::Show, LeaveAlternateScreen);
        let _ = terminal::disable_raw_mode();
    }
}

/// Run an interactive full-screen TUI diff review session.
///
/// Parses the provided unified diff, presents each hunk sequentially with
/// color-coded lines, accepts interactive decisions, and returns the finished session.
pub fn run_diff_review_interactive(diff_text: &str) -> std::io::Result<ReviewSession> {
    let mut session = ReviewSession::parse_unified_diff(diff_text);
    if session.is_empty() {
        return Ok(session);
    }

    let mut out = stdout();
    execute!(out, EnterAlternateScreen, cursor::Hide)?;
    terminal::enable_raw_mode()?;
    let _restore_guard = TerminalRestoreGuard;

    let backend = CrosstermBackend::new(out);
    let mut term = Terminal::new(backend)?;

    loop {
        term.draw(|f| {
            let widget = ReviewWidget::new(&session);
            f.render_widget(widget, f.area());
        })?;

        if let Event::Key(key) = event::read()? {
            if key.kind != KeyEventKind::Press {
                continue;
            }

            // Ctrl+C immediate exit
            if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
                break;
            }

            match key.code {
                KeyCode::Char('y') | KeyCode::Char('Y') => {
                    session.accept_current();
                    session.next_hunk();
                }
                KeyCode::Char('n') | KeyCode::Char('N') => {
                    session.reject_current();
                    session.next_hunk();
                }
                KeyCode::Char('a') | KeyCode::Char('A') => {
                    session.accept_all();
                }
                KeyCode::Char('r') | KeyCode::Char('R') => {
                    session.reject_all();
                }
                KeyCode::Up | KeyCode::Char('k') | KeyCode::Char('p') | KeyCode::Left => {
                    session.prev_hunk();
                }
                KeyCode::Down | KeyCode::Char('j') | KeyCode::Right => {
                    session.next_hunk();
                }
                KeyCode::Char('q') | KeyCode::Char('Q') | KeyCode::Esc => {
                    break;
                }
                _ => {}
            }
        }
    }

    Ok(session)
}
