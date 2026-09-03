//! Multi-turn diff statistics aggregator for session-wide code modifications.
//!
//! Aggregates line additions, line deletions, file modifications, creations, deletions,
//! and renames across multiple conversation turns or subagent executions.
//! Provides comprehensive statistics, git-style diffstat charts (with optional ANSI colors),
//! turn-by-turn timelines, language distributions, and markdown/JSON exports.

use async_trait::async_trait;
use globset::{Glob, GlobMatcher};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use similar::{ChangeTag, TextDiff};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, LazyLock, RwLock};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::tools::types::{Tool, ToolContext};

// ---------------------------------------------------------------------------
// Change Kind & File Diff Record
// ---------------------------------------------------------------------------

/// Type of file system change observed in a diff or tool execution.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DiffChangeType {
    /// File content modified.
    Modified,
    /// New file created / added.
    Added,
    /// File deleted.
    Deleted,
    /// File renamed or moved.
    Renamed,
    /// File copied.
    Copied,
    /// Untracked or unknown modification.
    Untracked,
}

impl DiffChangeType {
    pub fn as_str(&self) -> &'static str {
        match self {
            DiffChangeType::Modified => "modified",
            DiffChangeType::Added => "added",
            DiffChangeType::Deleted => "deleted",
            DiffChangeType::Renamed => "renamed",
            DiffChangeType::Copied => "copied",
            DiffChangeType::Untracked => "untracked",
        }
    }

    pub fn symbol(&self) -> &'static str {
        match self {
            DiffChangeType::Modified => "M",
            DiffChangeType::Added => "A",
            DiffChangeType::Deleted => "D",
            DiffChangeType::Renamed => "R",
            DiffChangeType::Copied => "C",
            DiffChangeType::Untracked => "?",
        }
    }
}

impl std::fmt::Display for DiffChangeType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

/// Statistics on line additions and deletions for a specific file change.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct LineStats {
    /// Total lines added/inserted.
    pub additions: usize,
    /// Total lines deleted/removed.
    pub deletions: usize,
}

impl LineStats {
    pub fn new(additions: usize, deletions: usize) -> Self {
        Self {
            additions,
            deletions,
        }
    }

    /// Net line delta (additions - deletions).
    pub fn net(&self) -> i64 {
        self.additions as i64 - self.deletions as i64
    }

    /// Total churn (additions + deletions).
    pub fn churn(&self) -> usize {
        self.additions + self.deletions
    }

    /// Returns true if no lines were added or deleted.
    pub fn is_empty(&self) -> bool {
        self.additions == 0 && self.deletions == 0
    }
}

/// A recorded diff modification for an individual file in a specific turn.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileDiffRecord {
    /// Target file path (relative to workspace root or normalized).
    pub path: String,
    /// Original file path (if renamed/moved).
    pub old_path: Option<String>,
    /// Turn index / ID when this change was executed.
    pub turn_id: usize,
    /// Timestamp (epoch milliseconds) when the change was recorded.
    pub timestamp: u64,
    /// Lines added in this change.
    pub additions: usize,
    /// Lines deleted in this change.
    pub deletions: usize,
    /// Nature of the change.
    pub change_type: DiffChangeType,
    /// Optional unified diff text snippet.
    pub unified_diff: Option<String>,
    /// Number of hunks in the diff.
    pub hunks_count: usize,
    /// Whether this file is binary.
    pub is_binary: bool,
    /// Author / subagent that performed the change (e.g. "Main", "Coder", "Reviewer").
    pub author_agent: Option<String>,
    /// Tool source that initiated the change (e.g. "edit", "write", "patch", "git").
    pub tool_source: String,
}

impl FileDiffRecord {
    /// Net line delta for this file diff.
    pub fn net_lines(&self) -> i64 {
        self.additions as i64 - self.deletions as i64
    }

    /// Total line churn (additions + deletions).
    pub fn churn(&self) -> usize {
        self.additions + self.deletions
    }

    /// Inferred programming language or file category based on extension.
    pub fn language(&self) -> &'static str {
        detect_language_from_path(&self.path)
    }

    /// Inferred file extension (without dot).
    pub fn extension(&self) -> &str {
        Path::new(&self.path)
            .extension()
            .and_then(|ext| ext.to_str())
            .unwrap_or("")
    }
}

/// Turn history entry for an individual file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileTurnHistoryEntry {
    pub turn_id: usize,
    pub timestamp: u64,
    pub additions: usize,
    pub deletions: usize,
    pub net_lines: i64,
    pub change_type: DiffChangeType,
    pub author_agent: Option<String>,
    pub tool_source: String,
}

/// Aggregated diff statistics for a specific file across all observed turns.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AggregatedFileStats {
    /// File path.
    pub path: String,
    /// Total lines added across all turns.
    pub total_additions: usize,
    /// Total lines deleted across all turns.
    pub total_deletions: usize,
    /// Net line change (total_additions - total_deletions).
    pub net_lines: i64,
    /// Total code churn (total_additions + total_deletions).
    pub total_churn: usize,
    /// Number of distinct turns in which this file was modified.
    pub modification_count: usize,
    /// First turn ID where this file was touched.
    pub first_turn: usize,
    /// Last turn ID where this file was touched.
    pub last_turn: usize,
    /// Latest observed change kind.
    pub latest_change_type: DiffChangeType,
    /// Whether the file is binary.
    pub is_binary: bool,
    /// Inferred language.
    pub language: String,
    /// Full chronological history of modifications for this file.
    pub history: Vec<FileTurnHistoryEntry>,
}

impl AggregatedFileStats {
    pub fn new(path: String) -> Self {
        let lang = detect_language_from_path(&path).to_string();
        Self {
            path,
            total_additions: 0,
            total_deletions: 0,
            net_lines: 0,
            total_churn: 0,
            modification_count: 0,
            first_turn: 0,
            last_turn: 0,
            latest_change_type: DiffChangeType::Modified,
            is_binary: false,
            language: lang,
            history: Vec::new(),
        }
    }

    /// Record an additional modification into this file's aggregated stats.
    pub fn record(&mut self, record: &FileDiffRecord) {
        if self.modification_count == 0 {
            self.first_turn = record.turn_id;
        }
        self.last_turn = record.turn_id;
        self.modification_count += 1;
        self.total_additions += record.additions;
        self.total_deletions += record.deletions;
        self.net_lines = self.total_additions as i64 - self.total_deletions as i64;
        self.total_churn = self.total_additions + self.total_deletions;
        self.latest_change_type = record.change_type;
        if record.is_binary {
            self.is_binary = true;
        }

        self.history.push(FileTurnHistoryEntry {
            turn_id: record.turn_id,
            timestamp: record.timestamp,
            additions: record.additions,
            deletions: record.deletions,
            net_lines: record.net_lines(),
            change_type: record.change_type,
            author_agent: record.author_agent.clone(),
            tool_source: record.tool_source.clone(),
        });
    }
}

/// Aggregated diff statistics for a single conversation turn.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TurnDiffSummary {
    /// Turn index / identifier.
    pub turn_id: usize,
    /// Optional turn label or description.
    pub turn_label: Option<String>,
    /// Number of distinct files modified in this turn.
    pub files_changed: usize,
    /// Total lines added in this turn.
    pub additions: usize,
    /// Total lines deleted in this turn.
    pub deletions: usize,
    /// Net line delta for this turn.
    pub net_lines: i64,
    /// Total line churn for this turn.
    pub churn: usize,
    /// Primary author or subagent for this turn.
    pub author_agent: Option<String>,
    /// Timestamp (epoch milliseconds) of the turn.
    pub timestamp: u64,
    /// File records belonging to this turn.
    pub files: Vec<FileDiffRecord>,
}

impl TurnDiffSummary {
    pub fn new(turn_id: usize) -> Self {
        Self {
            turn_id,
            turn_label: None,
            files_changed: 0,
            additions: 0,
            deletions: 0,
            net_lines: 0,
            churn: 0,
            author_agent: None,
            timestamp: current_timestamp_millis(),
            files: Vec::new(),
        }
    }

    /// Add a file record to this turn summary and update totals.
    pub fn add_file_record(&mut self, record: FileDiffRecord) {
        if self.author_agent.is_none() && record.author_agent.is_some() {
            self.author_agent = record.author_agent.clone();
        }
        self.additions += record.additions;
        self.deletions += record.deletions;
        self.net_lines = self.additions as i64 - self.deletions as i64;
        self.churn = self.additions + self.deletions;
        self.files.push(record);

        // Recompute distinct files count
        let unique_paths: HashSet<&str> = self.files.iter().map(|f| f.path.as_str()).collect();
        self.files_changed = unique_paths.len();
    }
}

/// Statistics grouped by programming language or file extension.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LanguageDiffStats {
    /// Language name (e.g. "Rust", "TypeScript", "Python", "Markdown").
    pub language: String,
    /// Distinct files changed in this language.
    pub files_count: usize,
    /// Total lines added.
    pub additions: usize,
    /// Total lines deleted.
    pub deletions: usize,
    /// Net line delta.
    pub net_lines: i64,
    /// Total code churn.
    pub churn: usize,
}

/// Complete multi-turn session diff statistics aggregation report.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionDiffStats {
    /// Total turns recorded in the session.
    pub total_turns: usize,
    /// Number of turns that contained at least one file change.
    pub active_turns_count: usize,
    /// Total distinct files modified across all turns.
    pub total_files_changed: usize,
    /// Total file modification events across all turns.
    pub total_file_modifications: usize,
    /// Total lines added / inserted across the entire session.
    pub total_additions: usize,
    /// Total lines deleted / removed across the entire session.
    pub total_deletions: usize,
    /// Net line delta across the entire session (total_additions - total_deletions).
    pub net_lines: i64,
    /// Total line churn (total_additions + total_deletions).
    pub total_churn: usize,
    /// Number of files added/created.
    pub files_added_count: usize,
    /// Number of files modified.
    pub files_modified_count: usize,
    /// Number of files deleted.
    pub files_deleted_count: usize,
    /// Number of files renamed.
    pub files_renamed_count: usize,
    /// Detailed statistics for each modified file.
    pub files: Vec<AggregatedFileStats>,
    /// Turn-by-turn diff summaries.
    pub turns: Vec<TurnDiffSummary>,
    /// Breakdown of changes by language.
    pub language_breakdown: Vec<LanguageDiffStats>,
    /// Top files ranked by total churn (additions + deletions).
    pub most_modified_files: Vec<(String, usize)>,
    /// Top files ranked by additions.
    pub top_additions_files: Vec<(String, usize)>,
    /// Top files ranked by deletions.
    pub top_deletions_files: Vec<(String, usize)>,
    /// Earliest recorded change timestamp (epoch ms).
    pub start_timestamp: u64,
    /// Latest recorded change timestamp (epoch ms).
    pub last_timestamp: u64,
}

impl Default for SessionDiffStats {
    fn default() -> Self {
        Self {
            total_turns: 0,
            active_turns_count: 0,
            total_files_changed: 0,
            total_file_modifications: 0,
            total_additions: 0,
            total_deletions: 0,
            net_lines: 0,
            total_churn: 0,
            files_added_count: 0,
            files_modified_count: 0,
            files_deleted_count: 0,
            files_renamed_count: 0,
            files: Vec::new(),
            turns: Vec::new(),
            language_breakdown: Vec::new(),
            most_modified_files: Vec::new(),
            top_additions_files: Vec::new(),
            top_deletions_files: Vec::new(),
            start_timestamp: 0,
            last_timestamp: 0,
        }
    }
}

impl SessionDiffStats {
    /// Returns true if no changes have been recorded across the session.
    pub fn is_empty(&self) -> bool {
        self.total_files_changed == 0 && self.total_additions == 0 && self.total_deletions == 0
    }

    /// Formats a concise git-style summary string:
    /// `X files changed, Y insertions(+), Z deletions(-)`
    pub fn summary_string(&self) -> String {
        format_git_summary_line(
            self.total_files_changed,
            self.total_additions,
            self.total_deletions,
        )
    }

    /// Formats a compact status badge:
    /// `[Δ 3 files | +45/-12 (net +33)]`
    pub fn badge_string(&self) -> String {
        let sign = if self.net_lines >= 0 { "+" } else { "" };
        format!(
            "[Δ {} file{} | +{}/-{} (net {}{})]",
            self.total_files_changed,
            if self.total_files_changed == 1 {
                ""
            } else {
                "s"
            },
            self.total_additions,
            self.total_deletions,
            sign,
            self.net_lines
        )
    }

    /// Formats a git-like diffstat table with bar charts.
    pub fn diffstat_string(&self, colorize: bool, max_bar_width: usize) -> String {
        format_diffstat_table(
            &self.files,
            self.total_files_changed,
            self.total_additions,
            self.total_deletions,
            colorize,
            max_bar_width,
        )
    }

    /// Formats a comprehensive Markdown report.
    pub fn markdown_report(&self) -> String {
        format_markdown_report(self)
    }

    /// Formats a detailed multi-turn terminal view.
    pub fn detailed_terminal_string(&self, colorize: bool) -> String {
        format_detailed_terminal(self, colorize)
    }
}

// ---------------------------------------------------------------------------
// Diff Parsing & Line Difference Computations
// ---------------------------------------------------------------------------

/// Computes additions and deletions between two text strings.
pub fn compute_line_diff_stats(old_content: &str, new_content: &str) -> LineStats {
    let diff = TextDiff::from_lines(old_content, new_content);
    let mut additions = 0;
    let mut deletions = 0;

    for change in diff.iter_all_changes() {
        match change.tag() {
            ChangeTag::Delete => deletions += 1,
            ChangeTag::Insert => additions += 1,
            ChangeTag::Equal => {}
        }
    }

    LineStats::new(additions, deletions)
}

/// Creates a `FileDiffRecord` from an edit operation (comparing `old_content` with `new_content`).
pub fn create_edit_record(
    path: &str,
    old_content: &str,
    new_content: &str,
    turn_id: usize,
    agent: Option<&str>,
) -> FileDiffRecord {
    let stats = compute_line_diff_stats(old_content, new_content);
    let diff = TextDiff::from_lines(old_content, new_content);
    let hunks_count = diff.unified_diff().iter_hunks().count();
    let unified = diff
        .unified_diff()
        .header(&format!("a/{}", path), &format!("b/{}", path))
        .to_string();

    let change_type = if old_content.is_empty() && !new_content.is_empty() {
        DiffChangeType::Added
    } else if !old_content.is_empty() && new_content.is_empty() {
        DiffChangeType::Deleted
    } else {
        DiffChangeType::Modified
    };

    FileDiffRecord {
        path: normalize_file_path(path),
        old_path: None,
        turn_id,
        timestamp: current_timestamp_millis(),
        additions: stats.additions,
        deletions: stats.deletions,
        change_type,
        unified_diff: if unified.is_empty() {
            None
        } else {
            Some(unified)
        },
        hunks_count,
        is_binary: false,
        author_agent: agent.map(|s| s.to_string()),
        tool_source: "edit".to_string(),
    }
}

/// Creates a `FileDiffRecord` from a file write operation.
pub fn create_write_record(
    path: &str,
    old_content: Option<&str>,
    new_content: &str,
    turn_id: usize,
    agent: Option<&str>,
) -> FileDiffRecord {
    let norm_path = normalize_file_path(path);
    match old_content {
        Some(old) => {
            let mut record = create_edit_record(&norm_path, old, new_content, turn_id, agent);
            record.tool_source = "write".to_string();
            record
        }
        None => {
            // Newly created file
            let line_count = if new_content.is_empty() {
                0
            } else {
                new_content.lines().count()
            };
            FileDiffRecord {
                path: norm_path,
                old_path: None,
                turn_id,
                timestamp: current_timestamp_millis(),
                additions: line_count,
                deletions: 0,
                change_type: DiffChangeType::Added,
                unified_diff: None,
                hunks_count: if line_count > 0 { 1 } else { 0 },
                is_binary: false,
                author_agent: agent.map(|s| s.to_string()),
                tool_source: "write".to_string(),
            }
        }
    }
}

/// Creates a `FileDiffRecord` for a deleted file.
pub fn create_delete_record(
    path: &str,
    old_content: &str,
    turn_id: usize,
    agent: Option<&str>,
) -> FileDiffRecord {
    let line_count = if old_content.is_empty() {
        0
    } else {
        old_content.lines().count()
    };
    FileDiffRecord {
        path: normalize_file_path(path),
        old_path: None,
        turn_id,
        timestamp: current_timestamp_millis(),
        additions: 0,
        deletions: line_count,
        change_type: DiffChangeType::Deleted,
        unified_diff: None,
        hunks_count: if line_count > 0 { 1 } else { 0 },
        is_binary: false,
        author_agent: agent.map(|s| s.to_string()),
        tool_source: "delete".to_string(),
    }
}

/// Parses a unified git diff string into a list of `FileDiffRecord` entries.
///
/// Accurately handles:
/// - `diff --git a/... b/...`
/// - `--- a/...` and `+++ b/...`
/// - `new file mode ...`
/// - `deleted file mode ...`
/// - `rename from ...` and `rename to ...`
/// - Binary files notice
/// - Hunk headers `@@ -l,s +l,s @@`
/// - Addition (`+`) and deletion (`-`) line counting
pub fn parse_unified_diff_to_records(
    diff_text: &str,
    turn_id: usize,
    agent: Option<&str>,
) -> Vec<FileDiffRecord> {
    let mut records = Vec::new();
    let lines: Vec<&str> = diff_text.lines().collect();
    let mut i = 0;

    while i < lines.len() {
        let line = lines[i];

        if line.starts_with("diff --git ") || line.starts_with("--- ") {
            let mut old_path = None;
            let mut new_path = None;
            let mut change_type = DiffChangeType::Modified;
            let mut is_binary = false;
            let mut additions = 0;
            let mut deletions = 0;
            let mut hunks_count = 0;
            let diff_start_idx = i;

            if line.starts_with("diff --git ") {
                let parts: Vec<&str> = line.split_whitespace().collect();
                if parts.len() >= 4 {
                    let a_part = parts[2].trim_start_matches("a/");
                    let b_part = parts[3].trim_start_matches("b/");
                    old_path = Some(a_part.to_string());
                    new_path = Some(b_part.to_string());
                }
                i += 1;
            }

            // Scan headers until hunks or next diff
            while i < lines.len()
                && !lines[i].starts_with("diff --git ")
                && !lines[i].starts_with("@@ ")
            {
                let hline = lines[i];
                if hline.starts_with("new file mode") {
                    change_type = DiffChangeType::Added;
                } else if hline.starts_with("deleted file mode") {
                    change_type = DiffChangeType::Deleted;
                } else if hline.starts_with("rename from ") {
                    old_path = Some(hline["rename from ".len()..].trim().to_string());
                    change_type = DiffChangeType::Renamed;
                } else if hline.starts_with("rename to ") {
                    new_path = Some(hline["rename to ".len()..].trim().to_string());
                    change_type = DiffChangeType::Renamed;
                } else if hline.starts_with("--- ") {
                    let p = hline["--- ".len()..].trim();
                    if p != "/dev/null" {
                        let clean_p = p.trim_start_matches("a/").trim_start_matches("b/");
                        if old_path.is_none() {
                            old_path = Some(clean_p.to_string());
                        }
                    } else {
                        change_type = DiffChangeType::Added;
                    }
                } else if hline.starts_with("+++ ") {
                    let p = hline["+++ ".len()..].trim();
                    if p != "/dev/null" {
                        let clean_p = p.trim_start_matches("a/").trim_start_matches("b/");
                        new_path = Some(clean_p.to_string());
                    } else {
                        change_type = DiffChangeType::Deleted;
                    }
                } else if hline.contains("Binary files ") || hline.contains("GIT binary patch") {
                    is_binary = true;
                }
                i += 1;
            }

            // Scan hunks and count lines
            while i < lines.len() && !lines[i].starts_with("diff --git ") {
                let hline = lines[i];
                if hline.starts_with("@@ ") {
                    hunks_count += 1;
                } else if hline.starts_with('+') && !hline.starts_with("+++") {
                    additions += 1;
                } else if hline.starts_with('-') && !hline.starts_with("---") {
                    deletions += 1;
                } else if hline.contains("Binary files ") {
                    is_binary = true;
                }
                i += 1;
            }

            let diff_snippet = lines[diff_start_idx..i].join("\n");
            let target_path = new_path
                .or(old_path.clone())
                .unwrap_or_else(|| format!("file_{}", records.len() + 1));

            records.push(FileDiffRecord {
                path: normalize_file_path(&target_path),
                old_path: old_path.map(|p| normalize_file_path(&p)),
                turn_id,
                timestamp: current_timestamp_millis(),
                additions,
                deletions,
                change_type,
                unified_diff: Some(diff_snippet),
                hunks_count,
                is_binary,
                author_agent: agent.map(|s| s.to_string()),
                tool_source: "diff".to_string(),
            });
        } else {
            i += 1;
        }
    }

    records
}

// ---------------------------------------------------------------------------
// Multi-turn Diff Aggregator Engine
// ---------------------------------------------------------------------------

/// Thread-safe multi-turn diff statistics accumulator and query engine.
#[derive(Debug, Clone)]
pub struct DiffAggregator {
    /// Current active conversation turn ID.
    current_turn: usize,
    /// Ordered turn records by turn_id.
    turns: BTreeMap<usize, TurnDiffSummary>,
    /// Global file-level aggregated stats cache.
    files: HashMap<String, AggregatedFileStats>,
    /// Aggregator creation timestamp.
    created_at: u64,
}

impl Default for DiffAggregator {
    fn default() -> Self {
        Self::new()
    }
}

impl DiffAggregator {
    /// Create a new, empty diff aggregator initialized at turn 1.
    pub fn new() -> Self {
        Self {
            current_turn: 1,
            turns: BTreeMap::new(),
            files: HashMap::new(),
            created_at: current_timestamp_millis(),
        }
    }

    /// Get current turn index.
    pub fn current_turn(&self) -> usize {
        self.current_turn
    }

    /// Advance or set the current turn index.
    pub fn set_turn(&mut self, turn_id: usize) {
        self.current_turn = turn_id.max(1);
    }

    /// Advance to the next turn (current_turn + 1).
    pub fn advance_turn(&mut self) -> usize {
        self.current_turn += 1;
        self.current_turn
    }

    /// Record an edit operation in the current turn.
    pub fn record_edit(
        &mut self,
        path: &str,
        old_content: &str,
        new_content: &str,
        agent: Option<&str>,
    ) -> FileDiffRecord {
        let record = create_edit_record(path, old_content, new_content, self.current_turn, agent);
        self.record_file_diff(record.clone());
        record
    }

    /// Record a file write operation in the current turn.
    pub fn record_write(
        &mut self,
        path: &str,
        old_content: Option<&str>,
        new_content: &str,
        agent: Option<&str>,
    ) -> FileDiffRecord {
        let record = create_write_record(path, old_content, new_content, self.current_turn, agent);
        self.record_file_diff(record.clone());
        record
    }

    /// Record a file deletion operation in the current turn.
    pub fn record_delete(
        &mut self,
        path: &str,
        old_content: &str,
        agent: Option<&str>,
    ) -> FileDiffRecord {
        let record = create_delete_record(path, old_content, self.current_turn, agent);
        self.record_file_diff(record.clone());
        record
    }

    /// Parse and record a unified diff in the current turn.
    pub fn record_diff_text(
        &mut self,
        diff_text: &str,
        agent: Option<&str>,
    ) -> Vec<FileDiffRecord> {
        let records = parse_unified_diff_to_records(diff_text, self.current_turn, agent);
        for record in &records {
            self.record_file_diff(record.clone());
        }
        records
    }

    /// Record a direct `FileDiffRecord` into the aggregator state.
    pub fn record_file_diff(&mut self, record: FileDiffRecord) {
        let turn_id = record.turn_id;

        // Update per-turn summary
        let turn_summary = self
            .turns
            .entry(turn_id)
            .or_insert_with(|| TurnDiffSummary::new(turn_id));
        turn_summary.add_file_record(record.clone());

        // Update aggregated file stats
        let file_stats = self
            .files
            .entry(record.path.clone())
            .or_insert_with(|| AggregatedFileStats::new(record.path.clone()));
        file_stats.record(&record);
    }

    /// Compute and return complete session-wide aggregated diff statistics.
    pub fn aggregate(&self) -> SessionDiffStats {
        self.aggregate_filtered(None, None, None, None)
    }

    /// Compute aggregated diff statistics for a specific turn range [start_turn, end_turn].
    pub fn aggregate_turn_range(&self, start_turn: usize, end_turn: usize) -> SessionDiffStats {
        self.aggregate_filtered(Some(start_turn), Some(end_turn), None, None)
    }

    /// Compute aggregated diff statistics for a single turn.
    pub fn aggregate_turn(&self, turn_id: usize) -> Option<TurnDiffSummary> {
        self.turns.get(&turn_id).cloned()
    }

    /// Compute aggregated stats for a specific file.
    pub fn aggregate_file(&self, path: &str) -> Option<AggregatedFileStats> {
        let norm = normalize_file_path(path);
        self.files.get(&norm).cloned()
    }

    /// Filter session diff stats using turn range, path pattern, and author agent filter.
    pub fn aggregate_filtered(
        &self,
        start_turn: Option<usize>,
        end_turn: Option<usize>,
        path_pattern: Option<&str>,
        agent_filter: Option<&str>,
    ) -> SessionDiffStats {
        let matcher: Option<GlobMatcher> =
            path_pattern.and_then(|p| Glob::new(p).ok().map(|g| g.compile_matcher()));

        let mut filtered_turns = Vec::new();
        let mut aggregated_files_map: HashMap<String, AggregatedFileStats> = HashMap::new();
        let mut total_additions = 0;
        let mut total_deletions = 0;
        let mut total_modifications = 0;
        let mut active_turns = 0;
        let mut earliest_ts = u64::MAX;
        let mut latest_ts = 0;

        for (&t_id, turn_summary) in &self.turns {
            if let Some(start) = start_turn {
                if t_id < start {
                    continue;
                }
            }
            if let Some(end) = end_turn {
                if t_id > end {
                    continue;
                }
            }

            let mut matched_files = Vec::new();
            for file_rec in &turn_summary.files {
                // Check agent filter
                if let Some(agent) = agent_filter {
                    if file_rec.author_agent.as_deref() != Some(agent) {
                        continue;
                    }
                }

                // Check path filter
                if let Some(m) = &matcher {
                    if !m.is_match(&file_rec.path) {
                        continue;
                    }
                } else if let Some(sub) = path_pattern {
                    if !file_rec.path.contains(sub) {
                        continue;
                    }
                }

                total_additions += file_rec.additions;
                total_deletions += file_rec.deletions;
                total_modifications += 1;
                earliest_ts = earliest_ts.min(file_rec.timestamp);
                latest_ts = latest_ts.max(file_rec.timestamp);

                let fstats = aggregated_files_map
                    .entry(file_rec.path.clone())
                    .or_insert_with(|| AggregatedFileStats::new(file_rec.path.clone()));
                fstats.record(file_rec);

                matched_files.push(file_rec.clone());
            }

            if !matched_files.is_empty() {
                active_turns += 1;
                let mut filtered_turn = TurnDiffSummary::new(t_id);
                filtered_turn.turn_label = turn_summary.turn_label.clone();
                filtered_turn.author_agent = turn_summary.author_agent.clone();
                filtered_turn.timestamp = turn_summary.timestamp;
                for f in matched_files {
                    filtered_turn.add_file_record(f);
                }
                filtered_turns.push(filtered_turn);
            }
        }

        let mut files_vec: Vec<AggregatedFileStats> = aggregated_files_map.into_values().collect();
        files_vec.sort_by(|a, b| {
            b.total_churn
                .cmp(&a.total_churn)
                .then_with(|| a.path.cmp(&b.path))
        });

        let mut files_added = 0;
        let mut files_modified = 0;
        let mut files_deleted = 0;
        let mut files_renamed = 0;

        for f in &files_vec {
            match f.latest_change_type {
                DiffChangeType::Added => files_added += 1,
                DiffChangeType::Modified | DiffChangeType::Untracked => files_modified += 1,
                DiffChangeType::Deleted => files_deleted += 1,
                DiffChangeType::Renamed | DiffChangeType::Copied => files_renamed += 1,
            }
        }

        // Language breakdown
        let mut lang_map: HashMap<String, (usize, usize, usize)> = HashMap::new();
        for f in &files_vec {
            let entry = lang_map.entry(f.language.clone()).or_insert((0, 0, 0));
            entry.0 += 1; // files count
            entry.1 += f.total_additions;
            entry.2 += f.total_deletions;
        }

        let mut language_breakdown: Vec<LanguageDiffStats> = lang_map
            .into_iter()
            .map(|(lang, (cnt, adds, dels))| LanguageDiffStats {
                language: lang,
                files_count: cnt,
                additions: adds,
                deletions: dels,
                net_lines: adds as i64 - dels as i64,
                churn: adds + dels,
            })
            .collect();
        language_breakdown.sort_by(|a, b| b.churn.cmp(&a.churn));

        // Rankings
        let mut most_modified_files: Vec<(String, usize)> = files_vec
            .iter()
            .map(|f| (f.path.clone(), f.total_churn))
            .collect();
        most_modified_files.sort_by(|a, b| b.1.cmp(&a.1));
        most_modified_files.truncate(10);

        let mut top_additions_files: Vec<(String, usize)> = files_vec
            .iter()
            .map(|f| (f.path.clone(), f.total_additions))
            .collect();
        top_additions_files.sort_by(|a, b| b.1.cmp(&a.1));
        top_additions_files.truncate(10);

        let mut top_deletions_files: Vec<(String, usize)> = files_vec
            .iter()
            .map(|f| (f.path.clone(), f.total_deletions))
            .collect();
        top_deletions_files.sort_by(|a, b| b.1.cmp(&a.1));
        top_deletions_files.truncate(10);

        let total_unique_files = files_vec.len();
        let net_lines = total_additions as i64 - total_deletions as i64;
        let total_churn = total_additions + total_deletions;
        let total_turns_count = if self.turns.is_empty() {
            0
        } else {
            *self.turns.keys().next_back().unwrap_or(&0)
        };

        SessionDiffStats {
            total_turns: total_turns_count,
            active_turns_count: active_turns,
            total_files_changed: total_unique_files,
            total_file_modifications: total_modifications,
            total_additions,
            total_deletions,
            net_lines,
            total_churn,
            files_added_count: files_added,
            files_modified_count: files_modified,
            files_deleted_count: files_deleted,
            files_renamed_count: files_renamed,
            files: files_vec,
            turns: filtered_turns,
            language_breakdown,
            most_modified_files,
            top_additions_files,
            top_deletions_files,
            start_timestamp: if earliest_ts == u64::MAX {
                self.created_at
            } else {
                earliest_ts
            },
            last_timestamp: if latest_ts == 0 {
                current_timestamp_millis()
            } else {
                latest_ts
            },
        }
    }

    /// Remove / roll back records for a specific turn.
    pub fn rollback_turn(&mut self, turn_id: usize) -> bool {
        if self.turns.remove(&turn_id).is_some() {
            // Rebuild file-level aggregated stats from remaining turns
            self.rebuild_file_stats();
            true
        } else {
            false
        }
    }

    /// Clear all recorded turns and reset state.
    pub fn clear(&mut self) {
        self.current_turn = 1;
        self.turns.clear();
        self.files.clear();
        self.created_at = current_timestamp_millis();
    }

    /// Total unique files modified across all turns.
    pub fn total_files_changed(&self) -> usize {
        self.files.len()
    }

    /// Total insertions across all turns.
    pub fn total_insertions(&self) -> usize {
        self.files.values().map(|f| f.total_additions).sum()
    }

    /// Total deletions across all turns.
    pub fn total_deletions(&self) -> usize {
        self.files.values().map(|f| f.total_deletions).sum()
    }

    /// Net line changes across all turns.
    pub fn net_lines(&self) -> i64 {
        self.total_insertions() as i64 - self.total_deletions() as i64
    }

    /// Rebuilds the internal `files` map from `turns`.
    fn rebuild_file_stats(&mut self) {
        self.files.clear();
        for turn in self.turns.values() {
            for record in &turn.files {
                let fstats = self
                    .files
                    .entry(record.path.clone())
                    .or_insert_with(|| AggregatedFileStats::new(record.path.clone()));
                fstats.record(record);
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Global Singleton Session Aggregator
// ---------------------------------------------------------------------------

/// Global singleton instance for tracking session-wide multi-turn diffs.
pub static GLOBAL_DIFF_AGGREGATOR: LazyLock<Arc<RwLock<DiffAggregator>>> =
    LazyLock::new(|| Arc::new(RwLock::new(DiffAggregator::new())));

/// Returns a shared reference to the global diff statistics aggregator.
pub fn global_diff_aggregator() -> Arc<RwLock<DiffAggregator>> {
    GLOBAL_DIFF_AGGREGATOR.clone()
}

/// Convenience helper to record an edit in the global diff aggregator.
pub fn record_session_edit(
    path: &str,
    old_content: &str,
    new_content: &str,
    agent: Option<&str>,
) -> FileDiffRecord {
    if let Ok(mut agg) = GLOBAL_DIFF_AGGREGATOR.write() {
        agg.record_edit(path, old_content, new_content, agent)
    } else {
        create_edit_record(path, old_content, new_content, 1, agent)
    }
}

/// Convenience helper to record a unified diff text in the global aggregator.
pub fn record_session_diff(diff_text: &str, agent: Option<&str>) -> Vec<FileDiffRecord> {
    if let Ok(mut agg) = GLOBAL_DIFF_AGGREGATOR.write() {
        agg.record_diff_text(diff_text, agent)
    } else {
        parse_unified_diff_to_records(diff_text, 1, agent)
    }
}

/// Convenience helper to record a file write in the global aggregator.
pub fn record_session_write(
    path: &str,
    old_content: Option<&str>,
    new_content: &str,
    agent: Option<&str>,
) -> FileDiffRecord {
    if let Ok(mut agg) = GLOBAL_DIFF_AGGREGATOR.write() {
        agg.record_write(path, old_content, new_content, agent)
    } else {
        create_write_record(path, old_content, new_content, 1, agent)
    }
}

/// Convenience helper to record a file deletion in the global aggregator.
pub fn record_session_delete(path: &str, old_content: &str, agent: Option<&str>) -> FileDiffRecord {
    if let Ok(mut agg) = GLOBAL_DIFF_AGGREGATOR.write() {
        agg.record_delete(path, old_content, agent)
    } else {
        create_delete_record(path, old_content, 1, agent)
    }
}

/// Retrieves the complete session diff statistics from the global aggregator.
pub fn get_session_diff_stats() -> SessionDiffStats {
    GLOBAL_DIFF_AGGREGATOR
        .read()
        .map(|agg| agg.aggregate())
        .unwrap_or_default()
}

/// Advances the turn index in the global diff aggregator.
pub fn advance_session_turn(turn_id: Option<usize>) -> usize {
    if let Ok(mut agg) = GLOBAL_DIFF_AGGREGATOR.write() {
        if let Some(id) = turn_id {
            agg.set_turn(id);
            id
        } else {
            agg.advance_turn()
        }
    } else {
        1
    }
}

/// Resets and clears the global diff aggregator.
pub fn reset_session_diff_stats() {
    if let Ok(mut agg) = GLOBAL_DIFF_AGGREGATOR.write() {
        agg.clear();
    }
}

// ---------------------------------------------------------------------------
// Output Formatting & Visualization
// ---------------------------------------------------------------------------

/// Output format modes supported by the diff aggregator and tool.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DiffStatsOutputFormat {
    /// Concise git summary line: "3 files changed, 45 insertions(+), 12 deletions(-)"
    Summary,
    /// Git-style diffstat chart with bar histogram.
    Diffstat,
    /// Detailed terminal report with turn timeline and file breakdowns.
    Detailed,
    /// Formatted Markdown report suitable for PR summaries or export.
    Markdown,
    /// Full structured JSON output.
    Json,
    /// Compact single-line metrics.
    Compact,
    /// Visual inline status badge: "[Δ 3 files | +45/-12 (net +33)]"
    Badge,
}

impl DiffStatsOutputFormat {
    pub fn from_str_loose(s: &str) -> Self {
        match s.trim().to_lowercase().as_str() {
            "diffstat" | "diff_stat" | "chart" | "stat" => DiffStatsOutputFormat::Diffstat,
            "detailed" | "detail" | "full" | "timeline" => DiffStatsOutputFormat::Detailed,
            "markdown" | "md" | "report" => DiffStatsOutputFormat::Markdown,
            "json" | "raw" => DiffStatsOutputFormat::Json,
            "compact" | "one_line" | "oneline" => DiffStatsOutputFormat::Compact,
            "badge" | "status" => DiffStatsOutputFormat::Badge,
            _ => DiffStatsOutputFormat::Summary,
        }
    }
}

/// Formats standard git-style summary line:
/// `X files changed, Y insertions(+), Z deletions(-)`
pub fn format_git_summary_line(files_changed: usize, additions: usize, deletions: usize) -> String {
    let file_str = if files_changed == 1 {
        "1 file changed"
    } else {
        &format!("{} files changed", files_changed)
    };

    let ins_str = if additions == 1 {
        "1 insertion(+)".to_string()
    } else {
        format!("{} insertions(+)", additions)
    };

    let del_str = if deletions == 1 {
        "1 deletion(-)".to_string()
    } else {
        format!("{} deletions(-)", deletions)
    };

    if additions > 0 && deletions > 0 {
        format!("{}, {}, {}", file_str, ins_str, del_str)
    } else if additions > 0 {
        format!("{}, {}", file_str, ins_str)
    } else if deletions > 0 {
        format!("{}, {}", file_str, del_str)
    } else if files_changed > 0 {
        format!("{}, 0 insertions(+), 0 deletions(-)", file_str)
    } else {
        "0 files changed, 0 insertions(+), 0 deletions(-)".to_string()
    }
}

/// Formats a git-style diffstat table with proportional `+` and `-` bars.
pub fn format_diffstat_table(
    files: &[AggregatedFileStats],
    total_files: usize,
    total_adds: usize,
    total_dels: usize,
    colorize: bool,
    max_bar_width: usize,
) -> String {
    if files.is_empty() {
        return "0 files changed, 0 insertions(+), 0 deletions(-)".to_string();
    }

    let mut output = String::new();
    let max_path_len = files
        .iter()
        .map(|f| f.path.len())
        .max()
        .unwrap_or(10)
        .min(50);
    let max_churn = files.iter().map(|f| f.total_churn).max().unwrap_or(1);
    let bar_cap = if max_bar_width == 0 {
        30
    } else {
        max_bar_width.min(60)
    };

    for file in files {
        let path_display = if file.path.len() > 50 {
            format!("...{}", &file.path[file.path.len() - 47..])
        } else {
            file.path.clone()
        };

        let total_changes = file.total_additions + file.total_deletions;
        let change_str = format!("{}", total_changes);

        // Calculate proportional bar segments
        let bar_len = if max_churn == 0 {
            0
        } else {
            ((total_changes as f64 / max_churn as f64) * bar_cap as f64).round() as usize
        };
        let bar_len = bar_len.max(if total_changes > 0 { 1 } else { 0 });

        let plus_count = if total_changes == 0 {
            0
        } else {
            ((file.total_additions as f64 / total_changes as f64) * bar_len as f64).round() as usize
        };
        let minus_count = bar_len.saturating_sub(plus_count);

        if colorize {
            let green_plus = if plus_count > 0 {
                format!("\x1b[32m{}\x1b[0m", "+".repeat(plus_count))
            } else {
                String::new()
            };
            let red_minus = if minus_count > 0 {
                format!("\x1b[31m{}\x1b[0m", "-".repeat(minus_count))
            } else {
                String::new()
            };
            output.push_str(&format!(
                " \x1b[1m{:<width$}\x1b[0m | {:>4} {}{}\n",
                path_display,
                change_str,
                green_plus,
                red_minus,
                width = max_path_len
            ));
        } else {
            let pluses = "+".repeat(plus_count);
            let minuses = "-".repeat(minus_count);
            output.push_str(&format!(
                " {:<width$} | {:>4} {}{}\n",
                path_display,
                change_str,
                pluses,
                minuses,
                width = max_path_len
            ));
        }
    }

    let summary_line = format_git_summary_line(total_files, total_adds, total_dels);
    if colorize {
        output.push_str(&format!(" \x1b[1m{}\x1b[0m", summary_line));
    } else {
        output.push_str(&format!(" {}", summary_line));
    }

    output
}

/// Formats a complete Markdown report.
pub fn format_markdown_report(stats: &SessionDiffStats) -> String {
    let mut md = String::new();
    md.push_str("# Session Diff Statistics\n\n");

    // Summary Metric Badges / Table
    md.push_str("### Overview\n\n");
    md.push_str(&format!(
        "- **Total Turns Recorded**: {}\n",
        stats.total_turns
    ));
    md.push_str(&format!(
        "- **Active Mutation Turns**: {}\n",
        stats.active_turns_count
    ));
    md.push_str(&format!(
        "- **Files Changed**: {}\n",
        stats.total_files_changed
    ));
    md.push_str(&format!(
        "- **Lines Inserted**: +{}\n",
        stats.total_additions
    ));
    md.push_str(&format!(
        "- **Lines Deleted**: -{}\n",
        stats.total_deletions
    ));
    let sign = if stats.net_lines >= 0 { "+" } else { "" };
    md.push_str(&format!("- **Net Lines**: {}{}\n", sign, stats.net_lines));
    md.push_str(&format!("- **Total Churn**: {}\n\n", stats.total_churn));

    // File Modifications Breakdown Table
    if !stats.files.is_empty() {
        md.push_str("### Modified Files Breakdown\n\n");
        md.push_str("| File | Type | Language | + Add | - Del | Net | Turns |\n");
        md.push_str("| :--- | :---: | :--- | :---: | :---: | :---: | :---: |\n");
        for f in &stats.files {
            let f_sign = if f.net_lines >= 0 { "+" } else { "" };
            md.push_str(&format!(
                "| `{}` | {} | {} | +{} | -{} | {}{} | {} |\n",
                f.path,
                f.latest_change_type.symbol(),
                f.language,
                f.total_additions,
                f.total_deletions,
                f_sign,
                f.net_lines,
                f.modification_count
            ));
        }
        md.push('\n');
    }

    // Language Breakdown Table
    if !stats.language_breakdown.is_empty() {
        md.push_str("### Language Distribution\n\n");
        md.push_str("| Language | Files | Additions | Deletions | Churn |\n");
        md.push_str("| :--- | :---: | :---: | :---: | :---: |\n");
        for l in &stats.language_breakdown {
            md.push_str(&format!(
                "| {} | {} | +{} | -{} | {} |\n",
                l.language, l.files_count, l.additions, l.deletions, l.churn
            ));
        }
        md.push('\n');
    }

    // Turn Timeline Summary
    if !stats.turns.is_empty() {
        md.push_str("### Turn-by-Turn Timeline\n\n");
        for t in &stats.turns {
            let author = t.author_agent.as_deref().unwrap_or("User/Agent");
            let t_sign = if t.net_lines >= 0 { "+" } else { "" };
            md.push_str(&format!("#### Turn {} ({})\n", t.turn_id, author));
            md.push_str(&format!(
                "*{} file(s) changed, +{} / -{} (net {}{})*\n\n",
                t.files_changed, t.additions, t.deletions, t_sign, t.net_lines
            ));
            for f in &t.files {
                md.push_str(&format!(
                    "- `{}` (+{}/-{}) [{}]\n",
                    f.path, f.additions, f.deletions, f.tool_source
                ));
            }
            md.push('\n');
        }
    }

    md
}

/// Formats a detailed multi-turn terminal view.
pub fn format_detailed_terminal(stats: &SessionDiffStats, colorize: bool) -> String {
    let mut out = String::new();

    if colorize {
        out.push_str("\x1b[1;34m=== Session Multi-Turn Diff Statistics ===\x1b[0m\n\n");
    } else {
        out.push_str("=== Session Multi-Turn Diff Statistics ===\n\n");
    }

    // Overview lines
    let sign = if stats.net_lines >= 0 { "+" } else { "" };
    out.push_str(&format!(
        "Turns: {} (active: {})\n",
        stats.total_turns, stats.active_turns_count
    ));
    out.push_str(&format!("Files Changed: {}\n", stats.total_files_changed));
    if colorize {
        out.push_str(&format!(
            "Lines: \x1b[32m+{}\x1b[0m / \x1b[31m-{}\x1b[0m (net \x1b[1m{}{}\x1b[0m, churn {})\n\n",
            stats.total_additions, stats.total_deletions, sign, stats.net_lines, stats.total_churn
        ));
    } else {
        out.push_str(&format!(
            "Lines: +{} / -{} (net {}{}, churn {})\n\n",
            stats.total_additions, stats.total_deletions, sign, stats.net_lines, stats.total_churn
        ));
    }

    // Diffstat section
    out.push_str("Diffstat:\n");
    out.push_str(&stats.diffstat_string(colorize, 35));
    out.push_str("\n\n");

    // Turn breakdown
    if !stats.turns.is_empty() {
        if colorize {
            out.push_str("\x1b[1mTurn Timeline:\x1b[0m\n");
        } else {
            out.push_str("Turn Timeline:\n");
        }
        for t in &stats.turns {
            let author = t.author_agent.as_deref().unwrap_or("Agent");
            let t_sign = if t.net_lines >= 0 { "+" } else { "" };
            if colorize {
                out.push_str(&format!(
                    "  \x1b[36mTurn {:>2}\x1b[0m [{:<8}] {:>2} files | \x1b[32m+{:>3}\x1b[0m / \x1b[31m-{:<3}\x1b[0m (net {}{})\n",
                    t.turn_id, author, t.files_changed, t.additions, t.deletions, t_sign, t.net_lines
                ));
            } else {
                out.push_str(&format!(
                    "  Turn {:>2} [{:<8}] {:>2} files | +{:>3} / -{:<3} (net {}{})\n",
                    t.turn_id,
                    author,
                    t.files_changed,
                    t.additions,
                    t.deletions,
                    t_sign,
                    t.net_lines
                ));
            }
        }
    }

    out
}

// ---------------------------------------------------------------------------
// Helper Utilities
// ---------------------------------------------------------------------------

/// Returns current timestamp in epoch milliseconds.
pub fn current_timestamp_millis() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

/// Normalizes a file path string by replacing backslashes and removing leading `./`.
pub fn normalize_file_path(path: &str) -> String {
    let clean = path.replace('\\', "/");
    let trimmed = clean.trim();
    if let Some(stripped) = trimmed.strip_prefix("./") {
        stripped.to_string()
    } else {
        trimmed.to_string()
    }
}

/// Detects programming language or file type name from a path.
pub fn detect_language_from_path(path_str: &str) -> &'static str {
    let path = Path::new(path_str);
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_lowercase();

    let file_name = path
        .file_name()
        .and_then(|f| f.to_str())
        .unwrap_or("")
        .to_lowercase();

    if file_name == "dockerfile" || file_name.starts_with("dockerfile.") {
        return "Dockerfile";
    }
    if file_name == "makefile" || file_name == "gnumakefile" {
        return "Makefile";
    }
    if file_name == "cargo.toml" || file_name == "cargo.lock" {
        return "Rust (Cargo)";
    }

    match ext.as_str() {
        "rs" => "Rust",
        "ts" | "mts" | "cts" => "TypeScript",
        "tsx" => "TypeScript (React)",
        "js" | "mjs" | "cjs" => "JavaScript",
        "jsx" => "JavaScript (React)",
        "py" | "pyi" => "Python",
        "go" => "Go",
        "c" | "h" => "C",
        "cpp" | "cc" | "cxx" | "hpp" | "hh" | "hxx" => "C++",
        "java" => "Java",
        "cs" => "C#",
        "kt" | "kts" => "Kotlin",
        "swift" => "Swift",
        "rb" => "Ruby",
        "php" => "PHP",
        "zig" => "Zig",
        "dart" => "Dart",
        "lua" => "Lua",
        "sh" | "bash" | "zsh" => "Shell",
        "sql" => "SQL",
        "html" | "htm" => "HTML",
        "css" | "scss" | "sass" | "less" => "CSS",
        "json" | "json5" | "jsonc" => "JSON",
        "toml" => "TOML",
        "yaml" | "yml" => "YAML",
        "md" | "markdown" | "mdx" => "Markdown",
        "xml" | "svg" => "XML",
        "graphql" | "gql" => "GraphQL",
        "proto" => "Protobuf",
        "wasm" => "WebAssembly",
        _ => "Text",
    }
}

// ---------------------------------------------------------------------------
// Tool Implementation (DiffStatsTool)
// ---------------------------------------------------------------------------

/// Action modes supported by the `DiffStatsTool`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DiffStatsAction {
    /// Return the session-wide or filtered summary string (default).
    Summary,
    /// Return git-style diffstat chart.
    Diffstat,
    /// Return detailed turn timeline and file breakdown.
    Breakdown,
    /// List all modified files with their individual line stats.
    Files,
    /// List turn-by-turn history.
    Turns,
    /// Inspect detailed modification history for a specific file.
    FileDetail,
    /// Parse and record a raw unified diff into the session stats.
    RecordDiff,
    /// Compute and record an edit diff from `old_content` and `new_content`.
    RecordEdit,
    /// Record a file write operation.
    RecordWrite,
    /// Compare statistics between two turns.
    CompareTurns,
    /// Reset / clear session diff stats.
    Reset,
    /// Export full session stats as JSON.
    Export,
}

impl DiffStatsAction {
    pub fn from_str_loose(s: &str) -> Self {
        match s.trim().to_lowercase().as_str() {
            "diffstat" | "diff_stat" | "stat" | "chart" => DiffStatsAction::Diffstat,
            "breakdown" | "detailed" | "detail" | "report" => DiffStatsAction::Breakdown,
            "files" | "file_list" | "list_files" => DiffStatsAction::Files,
            "turns" | "timeline" | "history" => DiffStatsAction::Turns,
            "file_detail" | "inspect_file" | "file" => DiffStatsAction::FileDetail,
            "record_diff" | "record" | "add_diff" => DiffStatsAction::RecordDiff,
            "record_edit" | "add_edit" => DiffStatsAction::RecordEdit,
            "record_write" | "add_write" => DiffStatsAction::RecordWrite,
            "compare_turns" | "compare" => DiffStatsAction::CompareTurns,
            "reset" | "clear" => DiffStatsAction::Reset,
            "export" | "json" => DiffStatsAction::Export,
            _ => DiffStatsAction::Summary,
        }
    }
}

/// Tool for querying and aggregating multi-turn diff statistics across an assistant session.
pub struct DiffStatsTool {
    aggregator: Arc<RwLock<DiffAggregator>>,
}

impl DiffStatsTool {
    pub fn new() -> Self {
        Self {
            aggregator: GLOBAL_DIFF_AGGREGATOR.clone(),
        }
    }

    pub fn with_aggregator(aggregator: Arc<RwLock<DiffAggregator>>) -> Self {
        Self { aggregator }
    }
}

impl Default for DiffStatsTool {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Tool for DiffStatsTool {
    fn name(&self) -> &str {
        "diff_stats"
    }

    fn description(&self) -> &str {
        "Multi-turn diff aggregator reporting total files changed, lines inserted, and lines deleted across session, with per-file, per-turn, and git-style diffstat breakdown."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "action": {
                    "type": "string",
                    "enum": [
                        "summary",
                        "diffstat",
                        "breakdown",
                        "files",
                        "turns",
                        "file_detail",
                        "record_diff",
                        "record_edit",
                        "record_write",
                        "compare_turns",
                        "reset",
                        "export"
                    ],
                    "description": "Action to perform: 'summary' (default), 'diffstat' (git-style chart), 'breakdown' (detailed report), 'files' (file list), 'turns' (timeline), 'file_detail' (history for path), 'record_diff', 'record_edit', 'record_write', 'compare_turns', 'reset', or 'export'."
                },
                "format": {
                    "type": "string",
                    "enum": ["summary", "diffstat", "detailed", "markdown", "json", "compact", "badge"],
                    "description": "Output format: 'summary' (default), 'diffstat', 'detailed', 'markdown', 'json', 'compact', or 'badge'."
                },
                "turn": {
                    "type": "integer",
                    "description": "Specific turn ID to query or filter."
                },
                "start_turn": {
                    "type": "integer",
                    "description": "Start turn ID for range filtering."
                },
                "end_turn": {
                    "type": "integer",
                    "description": "End turn ID for range filtering."
                },
                "path": {
                    "type": "string",
                    "description": "Optional file path or glob pattern to filter, or target path for record actions."
                },
                "agent": {
                    "type": "string",
                    "description": "Optional subagent / author name to filter or record (e.g. 'Coder', 'Main')."
                },
                "diff": {
                    "type": "string",
                    "description": "Unified git diff text to parse and record into session statistics (for 'record_diff')."
                },
                "old_content": {
                    "type": "string",
                    "description": "Original file text before edit (for 'record_edit' or 'record_write')."
                },
                "new_content": {
                    "type": "string",
                    "description": "New file text after edit (for 'record_edit' or 'record_write')."
                },
                "color": {
                    "type": "boolean",
                    "description": "Whether to colorize terminal output with ANSI escapes (default: false)."
                }
            }
        })
    }

    async fn execute(&self, args: Value, _ctx: &ToolContext) -> anyhow::Result<String> {
        let action_str = args
            .get("action")
            .and_then(|v| v.as_str())
            .unwrap_or("summary");
        let action = DiffStatsAction::from_str_loose(action_str);

        let format_str = args
            .get("format")
            .and_then(|v| v.as_str())
            .unwrap_or("summary");
        let format = DiffStatsOutputFormat::from_str_loose(format_str);

        let turn_filter = args
            .get("turn")
            .and_then(|v| v.as_u64())
            .map(|n| n as usize);
        let start_turn = args
            .get("start_turn")
            .and_then(|v| v.as_u64())
            .map(|n| n as usize)
            .or(turn_filter);
        let end_turn = args
            .get("end_turn")
            .and_then(|v| v.as_u64())
            .map(|n| n as usize)
            .or(turn_filter);

        let path_filter = args.get("path").and_then(|v| v.as_str());
        let agent_filter = args.get("agent").and_then(|v| v.as_str());
        let colorize = args.get("color").and_then(|v| v.as_bool()).unwrap_or(false);

        match action {
            DiffStatsAction::RecordDiff => {
                let diff_text = args.get("diff").and_then(|v| v.as_str()).ok_or_else(|| {
                    anyhow::anyhow!("Missing 'diff' parameter for record_diff action")
                })?;

                let mut agg = self
                    .aggregator
                    .write()
                    .map_err(|_| anyhow::anyhow!("Aggregator lock poisoned"))?;
                if let Some(t) = turn_filter {
                    agg.set_turn(t);
                }
                let records = agg.record_diff_text(diff_text, agent_filter);
                let total_adds: usize = records.iter().map(|r| r.additions).sum();
                let total_dels: usize = records.iter().map(|r| r.deletions).sum();

                Ok(format!(
                    "Recorded diff for {} file(s) in Turn {}: +{} / -{}",
                    records.len(),
                    agg.current_turn(),
                    total_adds,
                    total_dels
                ))
            }

            DiffStatsAction::RecordEdit => {
                let path = path_filter
                    .ok_or_else(|| anyhow::anyhow!("Missing 'path' parameter for record_edit"))?;
                let old_content = args
                    .get("old_content")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                let new_content = args
                    .get("new_content")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");

                let mut agg = self
                    .aggregator
                    .write()
                    .map_err(|_| anyhow::anyhow!("Aggregator lock poisoned"))?;
                if let Some(t) = turn_filter {
                    agg.set_turn(t);
                }
                let record = agg.record_edit(path, old_content, new_content, agent_filter);

                Ok(format!(
                    "Recorded edit on '{}' in Turn {}: +{} / -{}",
                    record.path, record.turn_id, record.additions, record.deletions
                ))
            }

            DiffStatsAction::RecordWrite => {
                let path = path_filter
                    .ok_or_else(|| anyhow::anyhow!("Missing 'path' parameter for record_write"))?;
                let old_content = args.get("old_content").and_then(|v| v.as_str());
                let new_content = args
                    .get("new_content")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");

                let mut agg = self
                    .aggregator
                    .write()
                    .map_err(|_| anyhow::anyhow!("Aggregator lock poisoned"))?;
                if let Some(t) = turn_filter {
                    agg.set_turn(t);
                }
                let record = agg.record_write(path, old_content, new_content, agent_filter);

                Ok(format!(
                    "Recorded write on '{}' ({}) in Turn {}: +{} / -{}",
                    record.path,
                    record.change_type,
                    record.turn_id,
                    record.additions,
                    record.deletions
                ))
            }

            DiffStatsAction::Reset => {
                let mut agg = self
                    .aggregator
                    .write()
                    .map_err(|_| anyhow::anyhow!("Aggregator lock poisoned"))?;
                agg.clear();
                Ok("Session diff statistics reset successfully.".to_string())
            }

            DiffStatsAction::FileDetail => {
                let path = path_filter
                    .ok_or_else(|| anyhow::anyhow!("Missing 'path' parameter for file_detail"))?;
                let agg = self
                    .aggregator
                    .read()
                    .map_err(|_| anyhow::anyhow!("Aggregator lock poisoned"))?;
                match agg.aggregate_file(path) {
                    Some(fstats) => {
                        if format == DiffStatsOutputFormat::Json {
                            Ok(serde_json::to_string_pretty(&fstats)?)
                        } else {
                            let mut s = format!(
                                "File: {}\nLanguage: {}\nType: {}\nModifications: {} turn(s)\nAdditions: +{}\nDeletions: -{}\nNet Lines: {}\nChurn: {}\nFirst Turn: {}\nLast Turn: {}\n\nHistory:\n",
                                fstats.path,
                                fstats.language,
                                fstats.latest_change_type,
                                fstats.modification_count,
                                fstats.total_additions,
                                fstats.total_deletions,
                                fstats.net_lines,
                                fstats.total_churn,
                                fstats.first_turn,
                                fstats.last_turn
                            );
                            for h in &fstats.history {
                                s.push_str(&format!(
                                    "  - Turn {}: +{}/-{} [{}] (by {})\n",
                                    h.turn_id,
                                    h.additions,
                                    h.deletions,
                                    h.tool_source,
                                    h.author_agent.as_deref().unwrap_or("agent")
                                ));
                            }
                            Ok(s)
                        }
                    }
                    None => Ok(format!("No recorded modifications for file '{}'", path)),
                }
            }

            DiffStatsAction::CompareTurns => {
                let t1 = start_turn.unwrap_or(1);
                let t2 = end_turn.unwrap_or(2);
                let agg = self
                    .aggregator
                    .read()
                    .map_err(|_| anyhow::anyhow!("Aggregator lock poisoned"))?;

                let s1 = agg
                    .aggregate_turn(t1)
                    .unwrap_or_else(|| TurnDiffSummary::new(t1));
                let s2 = agg
                    .aggregate_turn(t2)
                    .unwrap_or_else(|| TurnDiffSummary::new(t2));

                let comparison = json!({
                    "turn_a": {
                        "turn_id": t1,
                        "files_changed": s1.files_changed,
                        "additions": s1.additions,
                        "deletions": s1.deletions,
                        "net_lines": s1.net_lines,
                        "churn": s1.churn
                    },
                    "turn_b": {
                        "turn_id": t2,
                        "files_changed": s2.files_changed,
                        "additions": s2.additions,
                        "deletions": s2.deletions,
                        "net_lines": s2.net_lines,
                        "churn": s2.churn
                    },
                    "delta": {
                        "files_delta": s2.files_changed as i64 - s1.files_changed as i64,
                        "additions_delta": s2.additions as i64 - s1.additions as i64,
                        "deletions_delta": s2.deletions as i64 - s1.deletions as i64,
                        "churn_delta": s2.churn as i64 - s1.churn as i64
                    }
                });

                if format == DiffStatsOutputFormat::Json {
                    Ok(serde_json::to_string_pretty(&comparison)?)
                } else {
                    Ok(format!(
                        "Turn {} vs Turn {}:\n  Turn {}: {} files, +{}/-{} (churn {})\n  Turn {}: {} files, +{}/-{} (churn {})\n  Delta: additions: {:+}, deletions: {:+}, churn: {:+}",
                        t1, t2,
                        t1, s1.files_changed, s1.additions, s1.deletions, s1.churn,
                        t2, s2.files_changed, s2.additions, s2.deletions, s2.churn,
                        s2.additions as i64 - s1.additions as i64,
                        s2.deletions as i64 - s1.deletions as i64,
                        s2.churn as i64 - s1.churn as i64
                    ))
                }
            }

            DiffStatsAction::Export => {
                let agg = self
                    .aggregator
                    .read()
                    .map_err(|_| anyhow::anyhow!("Aggregator lock poisoned"))?;
                let stats = agg.aggregate_filtered(start_turn, end_turn, path_filter, agent_filter);
                Ok(serde_json::to_string_pretty(&stats)?)
            }

            DiffStatsAction::Summary
            | DiffStatsAction::Diffstat
            | DiffStatsAction::Breakdown
            | DiffStatsAction::Files
            | DiffStatsAction::Turns => {
                let agg = self
                    .aggregator
                    .read()
                    .map_err(|_| anyhow::anyhow!("Aggregator lock poisoned"))?;
                let stats = agg.aggregate_filtered(start_turn, end_turn, path_filter, agent_filter);

                // Override format if action was specific
                let effective_format = match action {
                    DiffStatsAction::Diffstat => DiffStatsOutputFormat::Diffstat,
                    DiffStatsAction::Breakdown => DiffStatsOutputFormat::Detailed,
                    _ => format,
                };

                match effective_format {
                    DiffStatsOutputFormat::Summary => Ok(stats.summary_string()),
                    DiffStatsOutputFormat::Diffstat => Ok(stats.diffstat_string(colorize, 35)),
                    DiffStatsOutputFormat::Detailed => Ok(stats.detailed_terminal_string(colorize)),
                    DiffStatsOutputFormat::Markdown => Ok(stats.markdown_report()),
                    DiffStatsOutputFormat::Json => Ok(serde_json::to_string_pretty(&stats)?),
                    DiffStatsOutputFormat::Compact => Ok(format!(
                        "files: {}, +{}, -{}, net: {:+}, churn: {}",
                        stats.total_files_changed,
                        stats.total_additions,
                        stats.total_deletions,
                        stats.net_lines,
                        stats.total_churn
                    )),
                    DiffStatsOutputFormat::Badge => Ok(stats.badge_string()),
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Unit Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_empty_diff_aggregator() {
        let agg = DiffAggregator::new();
        let stats = agg.aggregate();

        assert_eq!(stats.total_files_changed, 0);
        assert_eq!(stats.total_additions, 0);
        assert_eq!(stats.total_deletions, 0);
        assert_eq!(stats.net_lines, 0);
        assert_eq!(stats.total_churn, 0);
        assert!(stats.is_empty());
        assert_eq!(
            stats.summary_string(),
            "0 files changed, 0 insertions(+), 0 deletions(-)"
        );
    }

    #[test]
    fn test_record_single_edit() {
        let mut agg = DiffAggregator::new();
        let old = "fn main() {\n    println!(\"Hello\");\n}\n";
        let new =
            "fn main() {\n    println!(\"Hello, World!\");\n    println!(\"Fusion v2\");\n}\n";

        let rec = agg.record_edit("src/main.rs", old, new, Some("Coder"));

        assert_eq!(rec.path, "src/main.rs");
        assert_eq!(rec.additions, 2);
        assert_eq!(rec.deletions, 1);
        assert_eq!(rec.turn_id, 1);
        assert_eq!(rec.author_agent.as_deref(), Some("Coder"));

        let stats = agg.aggregate();
        assert_eq!(stats.total_files_changed, 1);
        assert_eq!(stats.total_additions, 2);
        assert_eq!(stats.total_deletions, 1);
        assert_eq!(stats.net_lines, 1);
        assert_eq!(stats.total_churn, 3);
        assert_eq!(
            stats.summary_string(),
            "1 file changed, 2 insertions(+), 1 deletion(-)"
        );
    }

    #[test]
    fn test_multi_turn_diff_aggregation() {
        let mut agg = DiffAggregator::new();

        // Turn 1: Add new file
        agg.set_turn(1);
        agg.record_write(
            "src/lib.rs",
            None,
            "pub fn add(a: i32, b: i32) -> i32 {\n    a + b\n}\n",
            Some("Coder"),
        );

        // Turn 2: Edit existing file
        agg.set_turn(2);
        agg.record_edit(
            "src/lib.rs",
            "pub fn add(a: i32, b: i32) -> i32 {\n    a + b\n}\n",
            "pub fn add(a: i32, b: i32) -> i32 {\n    // addition helper\n    a + b\n}\n\npub fn sub(a: i32, b: i32) -> i32 {\n    a - b\n}\n",
            Some("Coder"),
        );

        // Turn 2: Add another file
        agg.record_write("src/utils.rs", None, "pub fn helper() {}\n", Some("Coder"));

        // Turn 3: Modify utils
        agg.set_turn(3);
        agg.record_edit(
            "src/utils.rs",
            "pub fn helper() {}\n",
            "pub fn helper() -> bool {\n    true\n}\n",
            Some("Reviewer"),
        );

        let stats = agg.aggregate();
        assert_eq!(stats.total_turns, 3);
        assert_eq!(stats.active_turns_count, 3);
        assert_eq!(stats.total_files_changed, 2); // src/lib.rs and src/utils.rs
        assert_eq!(stats.total_file_modifications, 4);

        // Check specific file aggregated stats
        let lib_stats = agg.aggregate_file("src/lib.rs").unwrap();
        assert_eq!(lib_stats.modification_count, 2);
        assert_eq!(lib_stats.first_turn, 1);
        assert_eq!(lib_stats.last_turn, 2);
        assert_eq!(lib_stats.total_additions, 3 + 5); // 3 from create + 5 from edit
        assert_eq!(lib_stats.total_deletions, 1); // 1 deleted line in edit

        let utils_stats = agg.aggregate_file("src/utils.rs").unwrap();
        assert_eq!(utils_stats.modification_count, 2);
        assert_eq!(utils_stats.first_turn, 2);
        assert_eq!(utils_stats.last_turn, 3);

        // Check turn breakdown
        let turn1 = agg.aggregate_turn(1).unwrap();
        assert_eq!(turn1.files_changed, 1);
        assert_eq!(turn1.additions, 3);
        assert_eq!(turn1.deletions, 0);

        let turn2 = agg.aggregate_turn(2).unwrap();
        assert_eq!(turn2.files_changed, 2);

        let turn3 = agg.aggregate_turn(3).unwrap();
        assert_eq!(turn3.files_changed, 1);
        assert_eq!(turn3.author_agent.as_deref(), Some("Reviewer"));
    }

    #[test]
    fn test_parse_unified_diff_to_records() {
        let unified_diff = r#"
diff --git a/src/main.rs b/src/main.rs
index e69de29..d95f3ad 100644
--- a/src/main.rs
+++ b/src/main.rs
@@ -1,3 +1,4 @@
 fn main() {
-    println!("Old");
+    println!("New 1");
+    println!("New 2");
 }
diff --git a/README.md b/README.md
new file mode 100644
--- /dev/null
+++ b/README.md
@@ -0,0 +1,5 @@
+# Fusion Project
+Fast AI coding assistant.
+Line 3
+Line 4
+Line 5
"#;

        let records = parse_unified_diff_to_records(unified_diff, 1, Some("GitApply"));
        assert_eq!(records.len(), 2);

        assert_eq!(records[0].path, "src/main.rs");
        assert_eq!(records[0].additions, 2);
        assert_eq!(records[0].deletions, 1);
        assert_eq!(records[0].change_type, DiffChangeType::Modified);

        assert_eq!(records[1].path, "README.md");
        assert_eq!(records[1].additions, 5);
        assert_eq!(records[1].deletions, 0);
        assert_eq!(records[1].change_type, DiffChangeType::Added);
    }

    #[test]
    fn test_turn_range_filtering() {
        let mut agg = DiffAggregator::new();

        agg.set_turn(1);
        agg.record_write("file1.txt", None, "line1\nline2\n", None);

        agg.set_turn(2);
        agg.record_write("file2.txt", None, "alpha\nbeta\ngamma\n", None);

        agg.set_turn(3);
        agg.record_write("file3.txt", None, "x\n", None);

        // Aggregate turn 2 only
        let stats_turn2 = agg.aggregate_turn_range(2, 2);
        assert_eq!(stats_turn2.total_files_changed, 1);
        assert_eq!(stats_turn2.total_additions, 3);
        assert_eq!(stats_turn2.files[0].path, "file2.txt");

        // Aggregate turn 2 to 3
        let stats_turn2_3 = agg.aggregate_turn_range(2, 3);
        assert_eq!(stats_turn2_3.total_files_changed, 2);
        assert_eq!(stats_turn2_3.total_additions, 4);
    }

    #[test]
    fn test_path_and_agent_filtering() {
        let mut agg = DiffAggregator::new();

        agg.set_turn(1);
        agg.record_write("src/core.rs", None, "fn core() {}\n", Some("Alice"));
        agg.record_write("docs/readme.md", None, "# Readme\n", Some("Bob"));

        let stats_alice = agg.aggregate_filtered(None, None, None, Some("Alice"));
        assert_eq!(stats_alice.total_files_changed, 1);
        assert_eq!(stats_alice.files[0].path, "src/core.rs");

        let stats_docs = agg.aggregate_filtered(None, None, Some("docs/*"), None);
        assert_eq!(stats_docs.total_files_changed, 1);
        assert_eq!(stats_docs.files[0].path, "docs/readme.md");
    }

    #[test]
    fn test_language_detection_and_breakdown() {
        let mut agg = DiffAggregator::new();

        agg.record_write("src/lib.rs", None, "pub fn a() {}\n", None);
        agg.record_write(
            "web/app.tsx",
            None,
            "export const App = () => <div />;\n",
            None,
        );
        agg.record_write("scripts/test.py", None, "print('test')\n", None);

        let stats = agg.aggregate();
        assert_eq!(stats.language_breakdown.len(), 3);

        let langs: HashSet<&str> = stats
            .language_breakdown
            .iter()
            .map(|l| l.language.as_str())
            .collect();
        assert!(langs.contains("Rust"));
        assert!(langs.contains("TypeScript (React)"));
        assert!(langs.contains("Python"));
    }

    #[test]
    fn test_diffstat_formatting() {
        let mut agg = DiffAggregator::new();

        agg.record_edit(
            "src/main.rs",
            "line1\nline2\n",
            "line1\nline2_modified\nline3\n",
            None,
        );
        agg.record_write("src/lib.rs", None, "1\n2\n3\n4\n5\n", None);

        let stats = agg.aggregate();
        let stat_str = stats.diffstat_string(false, 30);

        assert!(stat_str.contains("src/main.rs"));
        assert!(stat_str.contains("src/lib.rs"));
        assert!(stat_str.contains("2 files changed, 6 insertions(+), 1 deletion(-)"));
        assert!(stat_str.contains('+'));
    }

    #[test]
    fn test_badge_and_markdown_formatting() {
        let mut agg = DiffAggregator::new();
        agg.record_write("src/main.rs", None, "fn main() {}\n", None);

        let stats = agg.aggregate();
        let badge = stats.badge_string();
        assert_eq!(badge, "[Δ 1 file | +1/-0 (net +1)]");

        let md = stats.markdown_report();
        assert!(md.contains("# Session Diff Statistics"));
        assert!(md.contains("`src/main.rs`"));
        assert!(md.contains("Rust"));
    }

    #[test]
    fn test_rollback_turn() {
        let mut agg = DiffAggregator::new();

        agg.set_turn(1);
        agg.record_write("file1.rs", None, "pub fn a() {}\n", None);

        agg.set_turn(2);
        agg.record_write("file2.rs", None, "pub fn b() {}\n", None);

        assert_eq!(agg.total_files_changed(), 2);

        let rolled = agg.rollback_turn(2);
        assert!(rolled);
        assert_eq!(agg.total_files_changed(), 1);
        assert!(agg.aggregate_file("file2.rs").is_none());
        assert!(agg.aggregate_file("file1.rs").is_some());
    }

    #[tokio::test]
    async fn test_diff_stats_tool_execution() {
        let tool = DiffStatsTool::new();
        let ctx = ToolContext::default();

        // 1. Reset
        let res = tool
            .execute(json!({ "action": "reset" }), &ctx)
            .await
            .unwrap();
        assert!(res.contains("reset successfully"));

        // 2. Record edit
        let edit_res = tool
            .execute(
                json!({
                    "action": "record_edit",
                    "path": "src/test.rs",
                    "old_content": "let a = 1;\n",
                    "new_content": "let a = 1;\nlet b = 2;\n",
                    "turn": 1,
                    "agent": "Coder"
                }),
                &ctx,
            )
            .await
            .unwrap();
        assert!(edit_res.contains("Recorded edit on 'src/test.rs'"));

        // 3. Query summary
        let summary_res = tool
            .execute(json!({ "action": "summary" }), &ctx)
            .await
            .unwrap();
        assert_eq!(summary_res, "1 file changed, 1 insertion(+)");

        // 4. Query diffstat
        let diffstat_res = tool
            .execute(json!({ "action": "diffstat" }), &ctx)
            .await
            .unwrap();
        assert!(diffstat_res.contains("src/test.rs"));

        // 5. Query file detail
        let detail_res = tool
            .execute(
                json!({ "action": "file_detail", "path": "src/test.rs" }),
                &ctx,
            )
            .await
            .unwrap();
        assert!(detail_res.contains("File: src/test.rs"));
        assert!(detail_res.contains("Language: Rust"));

        // 6. Export JSON
        let export_res = tool
            .execute(json!({ "action": "export" }), &ctx)
            .await
            .unwrap();
        let parsed: Value = serde_json::from_str(&export_res).unwrap();
        assert_eq!(parsed["total_files_changed"], 1);
        assert_eq!(parsed["total_additions"], 1);
        assert_eq!(parsed["total_deletions"], 0);
    }
}
