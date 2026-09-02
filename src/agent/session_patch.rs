//! Session patch aggregator and unified multi-file diff exporter.
//!
//! Combines all file creations, edits, and deletions performed across a conversational
//! session into a single, standard Git-compatible unified `session.patch` file.
//!
//! # Features
//! - **Cumulative State Reconstruction**: Multi-turn edits to the same file are aggregated
//!   into a single clean diff from initial base state to final session state.
//! - **Standard Git Patch Format**: Full support for `diff --git a/... b/...`, `new file mode 100644`,
//!   `deleted file mode 100644`, index headers, and hunk line numbers.
//! - **Multi-Source Aggregation**: Reconstructs patches from `Session` messages/tool calls,
//!   `Checkpoint` history, or in-memory file change builders.
//! - **Diff Statistics & Summaries**: Computes per-file and total additions/deletions,
//!   with Git-style `--stat` formatting and visual change graphs.
//! - **Reverse / Rollback Patches**: Inverts session changes to produce ready-to-apply rollback patches.
//! - **Patch Verification & Application**: Validates whether a patch can apply cleanly and
//!   applies aggregated changes directly to a workspace directory.

use std::collections::{HashMap, HashSet};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use chrono::Utc;
use serde::{Deserialize, Serialize};
use similar::{ChangeTag, TextDiff};
use thiserror::Error;
use uuid::Uuid;

use crate::agent::session::{Session, TokenStats};
use crate::agent::tokens::{estimate_message_tokens, estimate_messages_tokens, estimate_text_tokens};
use crate::agent::undo::Checkpoint;
use crate::provider::types::{Message, Role, ToolCall};
use crate::tools::edit::resolve_path;
use crate::tools::patch::{apply_file_patch_to_string, parse_unified_diff, PatchOptions};
/// The default filename used for exported session patches.
pub const DEFAULT_SESSION_PATCH_FILENAME: &str = "session.patch";

/// Default number of context lines around changes in unified diff hunks.
pub const DEFAULT_CONTEXT_RADIUS: usize = 3;

// ---------------------------------------------------------------------------
// FileEditKind
// ---------------------------------------------------------------------------

/// The nature of a file modification within a session.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum FileEditKind {
    /// A new file created during the session (Absent -> Present).
    Created,
    /// An existing file whose content was modified (Present -> Present with different content).
    Modified,
    /// A file that was deleted during the session (Present -> Absent).
    Deleted,
}

impl FileEditKind {
    /// Returns a human-readable display string.
    pub fn display_name(&self) -> &'static str {
        match self {
            FileEditKind::Created => "created",
            FileEditKind::Modified => "modified",
            FileEditKind::Deleted => "deleted",
        }
    }

    /// Single character Git status code ('A' = added, 'M' = modified, 'D' = deleted).
    pub fn status_code(&self) -> char {
        match self {
            FileEditKind::Created => 'A',
            FileEditKind::Modified => 'M',
            FileEditKind::Deleted => 'D',
        }
    }
}

// ---------------------------------------------------------------------------
// PatchFileStats
// ---------------------------------------------------------------------------

/// Additions and deletions count for a single file.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct PatchFileStats {
    /// Number of inserted lines (`+`).
    pub additions: usize,
    /// Number of deleted lines (`-`).
    pub deletions: usize,
}

impl PatchFileStats {
    /// Creates a new `PatchFileStats`.
    pub const fn new(additions: usize, deletions: usize) -> Self {
        Self {
            additions,
            deletions,
        }
    }

    /// Total number of changed lines (additions + deletions).
    pub fn total_changes(&self) -> usize {
        self.additions.saturating_add(self.deletions)
    }

    /// Whether there are zero line changes.
    pub fn is_empty(&self) -> bool {
        self.additions == 0 && self.deletions == 0
    }

    /// Formats a concise diff stat indicator (e.g. `+12, -4`).
    pub fn format_stat(&self) -> String {
        format!("+{}, -{}", self.additions, self.deletions)
    }

    /// Formats a visual histogram graph (e.g. `+++++---`) scaled to `max_width`.
    pub fn format_graph(&self, max_width: usize) -> String {
        let total = self.total_changes();
        if total == 0 || max_width == 0 {
            return String::new();
        }

        let (plus_count, minus_count) = if total <= max_width {
            (self.additions, self.deletions)
        } else {
            let scale = max_width as f64 / total as f64;
            let p = (self.additions as f64 * scale).round() as usize;
            let m = (self.deletions as f64 * scale).round() as usize;
            // Guarantee at least 1 symbol if non-zero
            let p = if self.additions > 0 && p == 0 { 1 } else { p };
            let m = if self.deletions > 0 && m == 0 { 1 } else { m };
            (p, m)
        };

        let mut s = String::with_capacity(plus_count + minus_count);
        for _ in 0..plus_count {
            s.push('+');
        }
        for _ in 0..minus_count {
            s.push('-');
        }
        s
    }
}

// ---------------------------------------------------------------------------
// SessionFilePatch
// ---------------------------------------------------------------------------

/// Represents the net diff for a single file across an entire session.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionFilePatch {
    /// Relative path of the file within the workspace.
    pub path: PathBuf,
    /// Kind of change (Created, Modified, Deleted).
    pub kind: FileEditKind,
    /// Baseline content before session edits (None for Created files).
    pub old_content: Option<String>,
    /// Final content after all session edits (None for Deleted files).
    pub new_content: Option<String>,
    /// Line addition and deletion statistics.
    pub stats: PatchFileStats,
    /// Whether the file is detected as binary.
    pub is_binary: bool,
    /// POSIX file mode (default 0o100644).
    pub file_mode: u32,
}

impl SessionFilePatch {
    /// Creates a new `SessionFilePatch` computing diff stats automatically.
    pub fn new(
        path: impl Into<PathBuf>,
        kind: FileEditKind,
        old_content: Option<String>,
        new_content: Option<String>,
    ) -> Self {
        let path = path.into();
        let stats = Self::compute_stats(old_content.as_deref(), new_content.as_deref());
        Self {
            path,
            kind,
            old_content,
            new_content,
            stats,
            is_binary: false,
            file_mode: 0o100644,
        }
    }

    /// Convenience constructor for a newly created file.
    pub fn created(path: impl Into<PathBuf>, content: impl Into<String>) -> Self {
        let content_str = content.into();
        Self::new(path, FileEditKind::Created, None, Some(content_str))
    }

    /// Convenience constructor for a modified file.
    pub fn modified(
        path: impl Into<PathBuf>,
        old_content: impl Into<String>,
        new_content: impl Into<String>,
    ) -> Self {
        Self::new(
            path,
            FileEditKind::Modified,
            Some(old_content.into()),
            Some(new_content.into()),
        )
    }

    /// Convenience constructor for a deleted file.
    pub fn deleted(path: impl Into<PathBuf>, old_content: impl Into<String>) -> Self {
        Self::new(
            path,
            FileEditKind::Deleted,
            Some(old_content.into()),
            None,
        )
    }

    /// Path formatted as a clean POSIX-style relative string (forward slashes).
    pub fn path_str(&self) -> String {
        self.path.to_string_lossy().replace('\\', "/")
    }

    /// Computes addition and deletion line stats between old and new text.
    fn compute_stats(old_text: Option<&str>, new_text: Option<&str>) -> PatchFileStats {
        let old_s = old_text.unwrap_or("");
        let new_s = new_text.unwrap_or("");

        let diff = TextDiff::from_lines(old_s, new_s);
        let mut additions = 0;
        let mut deletions = 0;

        for change in diff.iter_all_changes() {
            match change.tag() {
                ChangeTag::Insert => additions += 1,
                ChangeTag::Delete => deletions += 1,
                ChangeTag::Equal => {}
            }
        }

        PatchFileStats::new(additions, deletions)
    }

    /// Generates unified diff text for this file using the specified options.
    pub fn unified_diff(&self, options: &SessionPatchOptions) -> String {
        let path_str = self.path_str();
        let old_text = self.old_content.as_deref().unwrap_or("");
        let new_text = self.new_content.as_deref().unwrap_or("");

        let mut out = String::new();

        if options.git_format {
            out.push_str(&format!(
                "diff --git {}{} {}{}\n",
                options.path_prefix_a, path_str, options.path_prefix_b, path_str
            ));

            match self.kind {
                FileEditKind::Created => {
                    out.push_str(&format!("new file mode {:06o}\n", self.file_mode));
                    out.push_str("--- /dev/null\n");
                    out.push_str(&format!("+++ {}{}\n", options.path_prefix_b, path_str));
                }
                FileEditKind::Deleted => {
                    out.push_str(&format!("deleted file mode {:06o}\n", self.file_mode));
                    out.push_str(&format!("--- {}{}\n", options.path_prefix_a, path_str));
                    out.push_str("+++ /dev/null\n");
                }
                FileEditKind::Modified => {
                    out.push_str(&format!("--- {}{}\n", options.path_prefix_a, path_str));
                    out.push_str(&format!("+++ {}{}\n", options.path_prefix_b, path_str));
                }
            }
        } else {
            let header_a = match self.kind {
                FileEditKind::Created => "/dev/null".to_string(),
                _ => format!("{}{}", options.path_prefix_a, path_str),
            };
            let header_b = match self.kind {
                FileEditKind::Deleted => "/dev/null".to_string(),
                _ => format!("{}{}", options.path_prefix_b, path_str),
            };
            out.push_str(&format!("--- {}\n", header_a));
            out.push_str(&format!("+++ {}\n", header_b));
        }

        let diff = TextDiff::from_lines(old_text, new_text);
        let header_a = format!("{}{}", options.path_prefix_a, path_str);
        let header_b = format!("{}{}", options.path_prefix_b, path_str);

        let mut udiff = diff.unified_diff();
        udiff.context_radius(options.context_radius);
        udiff.header(&header_a, &header_b);

        for hunk in udiff.iter_hunks() {
            out.push_str(&format!("{}\n", hunk.header()));
            for change in hunk.iter_changes() {
                let prefix = match change.tag() {
                    ChangeTag::Delete => "-",
                    ChangeTag::Insert => "+",
                    ChangeTag::Equal => " ",
                };
                out.push_str(prefix);
                out.push_str(change.value());
                if !change.value().ends_with('\n') {
                    out.push('\n');
                }
            }
        }

        out
    }

    /// Generates ANSI-colorized diff text for terminal inspection.
    pub fn colorized_diff(&self, options: &SessionPatchOptions) -> String {
        let path_str = self.path_str();
        let old_text = self.old_content.as_deref().unwrap_or("");
        let new_text = self.new_content.as_deref().unwrap_or("");

        let mut out = String::new();

        if options.git_format {
            out.push_str(&format!(
                "\x1b[1mdiff --git {}{} {}{}\x1b[0m\n",
                options.path_prefix_a, path_str, options.path_prefix_b, path_str
            ));

            match self.kind {
                FileEditKind::Created => {
                    out.push_str(&format!(
                        "\x1b[1;32mnew file mode {:06o}\x1b[0m\n",
                        self.file_mode
                    ));
                    out.push_str("\x1b[1;31m--- /dev/null\x1b[0m\n");
                    out.push_str(&format!(
                        "\x1b[1;32m+++ {}{}\x1b[0m\n",
                        options.path_prefix_b, path_str
                    ));
                }
                FileEditKind::Deleted => {
                    out.push_str(&format!(
                        "\x1b[1;31mdeleted file mode {:06o}\x1b[0m\n",
                        self.file_mode
                    ));
                    out.push_str(&format!(
                        "\x1b[1;31m--- {}{}\x1b[0m\n",
                        options.path_prefix_a, path_str
                    ));
                    out.push_str("\x1b[1;32m+++ /dev/null\x1b[0m\n");
                }
                FileEditKind::Modified => {
                    out.push_str(&format!(
                        "\x1b[1;31m--- {}{}\x1b[0m\n",
                        options.path_prefix_a, path_str
                    ));
                    out.push_str(&format!(
                        "\x1b[1;32m+++ {}{}\x1b[0m\n",
                        options.path_prefix_b, path_str
                    ));
                }
            }
        }

        let diff = TextDiff::from_lines(old_text, new_text);
        let header_a = format!("{}{}", options.path_prefix_a, path_str);
        let header_b = format!("{}{}", options.path_prefix_b, path_str);

        let mut udiff = diff.unified_diff();
        udiff.context_radius(options.context_radius);
        udiff.header(&header_a, &header_b);

        for hunk in udiff.iter_hunks() {
            out.push_str(&format!("\x1b[36m{}\x1b[0m\n", hunk.header()));
            for change in hunk.iter_changes() {
                match change.tag() {
                    ChangeTag::Delete => {
                        out.push_str("\x1b[31m-");
                        out.push_str(change.value());
                        out.push_str("\x1b[0m");
                    }
                    ChangeTag::Insert => {
                        out.push_str("\x1b[32m+");
                        out.push_str(change.value());
                        out.push_str("\x1b[0m");
                    }
                    ChangeTag::Equal => {
                        out.push(' ');
                        out.push_str(change.value());
                    }
                }
                if !change.value().ends_with('\n') {
                    out.push('\n');
                }
            }
        }

        out
    }

    /// Creates an inverted patch representing the rollback of this file edit.
    pub fn reverse(&self) -> Self {
        let reversed_kind = match self.kind {
            FileEditKind::Created => FileEditKind::Deleted,
            FileEditKind::Deleted => FileEditKind::Created,
            FileEditKind::Modified => FileEditKind::Modified,
        };

        Self::new(
            self.path.clone(),
            reversed_kind,
            self.new_content.clone(),
            self.old_content.clone(),
        )
    }
}

// ---------------------------------------------------------------------------
// SessionPatchSummary
// ---------------------------------------------------------------------------

/// Aggregate change statistics across all files in a session patch.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct SessionPatchSummary {
    /// Total number of affected files.
    pub total_files: usize,
    /// Number of new files created.
    pub files_created: usize,
    /// Number of existing files modified.
    pub files_modified: usize,
    /// Number of files deleted.
    pub files_deleted: usize,
    /// Total line additions across all files.
    pub total_additions: usize,
    /// Total line deletions across all files.
    pub total_deletions: usize,
    /// Per-file stats: (path, change_kind, stats).
    pub file_stats: Vec<(PathBuf, FileEditKind, PatchFileStats)>,
}

impl SessionPatchSummary {
    /// Computes summary statistics from a slice of `SessionFilePatch`.
    pub fn from_files(files: &[SessionFilePatch]) -> Self {
        let mut total_files = 0;
        let mut files_created = 0;
        let mut files_modified = 0;
        let mut files_deleted = 0;
        let mut total_additions: usize = 0;
        let mut total_deletions: usize = 0;
        let mut file_stats = Vec::with_capacity(files.len());

        for f in files {
            total_files += 1;
            match f.kind {
                FileEditKind::Created => files_created += 1,
                FileEditKind::Modified => files_modified += 1,
                FileEditKind::Deleted => files_deleted += 1,
            }
            total_additions = total_additions.saturating_add(f.stats.additions);
            total_deletions = total_deletions.saturating_add(f.stats.deletions);
            file_stats.push((f.path.clone(), f.kind, f.stats));
        }

        Self {
            total_files,
            files_created,
            files_modified,
            files_deleted,
            total_additions,
            total_deletions,
            file_stats,
        }
    }

    /// Formats a single-line summary (e.g. `3 files changed, 45 insertions(+), 12 deletions(-)`).
    pub fn format_summary_line(&self) -> String {
        let file_word = if self.total_files == 1 { "file" } else { "files" };
        let ins_word = if self.total_additions == 1 {
            "insertion(+)"
        } else {
            "insertions(+)"
        };
        let del_word = if self.total_deletions == 1 {
            "deletion(-)"
        } else {
            "deletions(-)"
        };

        format!(
            "{} {} changed, {} {}, {} {}",
            self.total_files, file_word, self.total_additions, ins_word, self.total_deletions, del_word
        )
    }

    /// Formats a Git-style `--stat` table with aligned file paths, changes, and histogram graphs.
    pub fn format_table(&self) -> String {
        if self.file_stats.is_empty() {
            return "0 files changed\n".to_string();
        }

        let max_path_len = self
            .file_stats
            .iter()
            .map(|(p, _, _)| p.to_string_lossy().len())
            .max()
            .unwrap_or(10)
            .min(50);

        let max_changes = self
            .file_stats
            .iter()
            .map(|(_, _, s)| s.total_changes())
            .max()
            .unwrap_or(0);

        let count_width = format!("{}", max_changes).len().max(1);
        let graph_width = 30;

        let mut out = String::new();

        for (path, kind, stats) in &self.file_stats {
            let path_str = path.to_string_lossy().replace('\\', "/");
            let total = stats.total_changes();
            let graph = stats.format_graph(graph_width);

            let kind_marker = match kind {
                FileEditKind::Created => " (new)",
                FileEditKind::Deleted => " (gone)",
                FileEditKind::Modified => "",
            };

            let display_path = format!("{}{}", path_str, kind_marker);

            out.push_str(&format!(
                " {:<width$} | {:>count_width$} {}\n",
                display_path,
                total,
                graph,
                width = max_path_len + 7,
                count_width = count_width
            ));
        }

        out.push_str(&format!(" {}\n", self.format_summary_line()));
        out
    }
}

// ---------------------------------------------------------------------------
// SessionPatchOptions
// ---------------------------------------------------------------------------

/// Configuration options for generating and formatting session patches.
#[derive(Debug, Clone)]
pub struct SessionPatchOptions {
    /// Number of context lines in unified diff hunks (default: 3).
    pub context_radius: usize,
    /// Whether to include metadata header comments at the top of the patch.
    pub include_headers: bool,
    /// Whether to output standard `diff --git` headers (default: true).
    pub git_format: bool,
    /// Whether to include the `--stat` summary in header comments (default: true).
    pub include_stats_comment: bool,
    /// Prefix for the 'old' file path header (default: "a/").
    pub path_prefix_a: String,
    /// Prefix for the 'new' file path header (default: "b/").
    pub path_prefix_b: String,
    /// Optional whitelist of file paths to include (filters out other files).
    pub file_filter: Option<HashSet<PathBuf>>,
    /// Optional working directory / workspace root.
    pub working_dir: Option<PathBuf>,
    /// Whether to produce ANSI-colorized output (default: false).
    pub colorized: bool,
}

impl Default for SessionPatchOptions {
    fn default() -> Self {
        Self {
            context_radius: DEFAULT_CONTEXT_RADIUS,
            include_headers: true,
            git_format: true,
            include_stats_comment: true,
            path_prefix_a: "a/".to_string(),
            path_prefix_b: "b/".to_string(),
            file_filter: None,
            working_dir: None,
            colorized: false,
        }
    }
}

impl SessionPatchOptions {
    /// Creates default options.
    pub fn new() -> Self {
        Self::default()
    }

    /// Sets the context line radius around diff changes.
    pub fn with_context_radius(mut self, radius: usize) -> Self {
        self.context_radius = radius;
        self
    }

    /// Toggles metadata header comments.
    pub fn with_headers(mut self, include: bool) -> Self {
        self.include_headers = include;
        self
    }

    /// Toggles `diff --git` header formatting.
    pub fn with_git_format(mut self, git_format: bool) -> Self {
        self.git_format = git_format;
        self
    }

    /// Toggles `--stat` summary table in comment headers.
    pub fn with_stats_comment(mut self, include: bool) -> Self {
        self.include_stats_comment = include;
        self
    }

    /// Sets path prefixes (e.g. `a/` and `b/`).
    pub fn with_prefixes(mut self, prefix_a: impl Into<String>, prefix_b: impl Into<String>) -> Self {
        self.path_prefix_a = prefix_a.into();
        self.path_prefix_b = prefix_b.into();
        self
    }

    /// Sets a file filter whitelist.
    pub fn with_filter<I, P>(mut self, paths: I) -> Self
    where
        I: IntoIterator<Item = P>,
        P: Into<PathBuf>,
    {
        let set: HashSet<PathBuf> = paths.into_iter().map(Into::into).collect();
        self.file_filter = Some(set);
        self
    }

    /// Sets the working directory context.
    pub fn with_working_dir(mut self, cwd: impl Into<PathBuf>) -> Self {
        self.working_dir = Some(cwd.into());
        self
    }

    /// Toggles ANSI colorized output.
    pub fn with_color(mut self, colorized: bool) -> Self {
        self.colorized = colorized;
        self
    }
}

// ---------------------------------------------------------------------------
// SessionPatch
// ---------------------------------------------------------------------------

/// Complete aggregated patch representing all file edits across a session.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionPatch {
    /// Optional session identifier (UUID).
    pub session_id: Option<String>,
    /// Optional session title.
    pub session_title: Option<String>,
    /// Active model used in the session.
    pub model: Option<String>,
    /// ISO 8601 / RFC 3339 creation timestamp.
    pub created_at: String,
    /// List of file patches in this session patch.
    pub files: Vec<SessionFilePatch>,
    /// Aggregate summary statistics.
    pub summary: SessionPatchSummary,
    /// Optional custom metadata key-value pairs.
    pub metadata: HashMap<String, String>,
}

impl SessionPatch {
    /// Creates a new `SessionPatch` from a collection of file patches.
    pub fn new(files: Vec<SessionFilePatch>) -> Self {
        let summary = SessionPatchSummary::from_files(&files);
        Self {
            session_id: None,
            session_title: None,
            model: None,
            created_at: Utc::now().to_rfc3339(),
            files,
            summary,
            metadata: HashMap::new(),
        }
    }

    /// Creates a new `SessionPatchBuilder` for fluent construction.
    pub fn builder() -> SessionPatchBuilder {
        SessionPatchBuilder::new()
    }

    /// Returns the number of modified files in this patch.
    pub fn file_count(&self) -> usize {
        self.files.len()
    }

    /// Returns true if this patch contains zero file modifications.
    pub fn is_empty(&self) -> bool {
        self.files.is_empty()
    }

    /// Returns a list of all affected file paths.
    pub fn file_paths(&self) -> Vec<PathBuf> {
        self.files.iter().map(|f| f.path.clone()).collect()
    }

    /// Retrieves a file patch by relative path.
    pub fn get_file(&self, path: impl AsRef<Path>) -> Option<&SessionFilePatch> {
        let p = path.as_ref();
        self.files.iter().find(|f| f.path == p)
    }

    /// Generates unified diff text for the entire session.
    pub fn to_unified_diff(&self, options: &SessionPatchOptions) -> String {
        if options.colorized {
            return self.to_colorized_diff(options);
        }

        let mut out = String::new();

        if options.include_headers {
            out.push_str("# ====================================================================\n");
            out.push_str("# Fusion Session Unified Patch\n");
            if let Some(id) = &self.session_id {
                out.push_str(&format!("# Session ID:   {}\n", id));
            }
            if let Some(title) = &self.session_title {
                out.push_str(&format!("# Title:        {}\n", title));
            }
            if let Some(model) = &self.model {
                out.push_str(&format!("# Model:        {}\n", model));
            }
            out.push_str(&format!("# Created:      {}\n", self.created_at));
            out.push_str(&format!("# Summary:      {}\n", self.summary.format_summary_line()));

            if options.include_stats_comment && !self.files.is_empty() {
                out.push_str("#\n# File Breakdown:\n");
                let stat_table = self.summary.format_table();
                for line in stat_table.lines() {
                    out.push_str(&format!("#   {}\n", line));
                }
            }
            out.push_str("# ====================================================================\n\n");
        }

        for file in &self.files {
            if let Some(filter) = &options.file_filter {
                if !filter.contains(&file.path) {
                    continue;
                }
            }
            out.push_str(&file.unified_diff(options));
            out.push('\n');
        }

        out
    }

    /// Generates ANSI-colorized diff text for terminal display.
    pub fn to_colorized_diff(&self, options: &SessionPatchOptions) -> String {
        let mut out = String::new();

        if options.include_headers {
            out.push_str("\x1b[1;36m=== Fusion Session Unified Patch ===\x1b[0m\n");
            if let Some(id) = &self.session_id {
                out.push_str(&format!("\x1b[1mSession ID:\x1b[0m   {}\n", id));
            }
            if let Some(title) = &self.session_title {
                out.push_str(&format!("\x1b[1mTitle:\x1b[0m        {}\n", title));
            }
            if let Some(model) = &self.model {
                out.push_str(&format!("\x1b[1mModel:\x1b[0m        {}\n", model));
            }
            out.push_str(&format!("\x1b[1mSummary:\x1b[0m      {}\n\n", self.summary.format_summary_line()));

            if options.include_stats_comment && !self.files.is_empty() {
                out.push_str("\x1b[1;33mDiff Statistics:\x1b[0m\n");
                out.push_str(&self.summary.format_table());
                out.push('\n');
            }
        }

        for file in &self.files {
            if let Some(filter) = &options.file_filter {
                if !filter.contains(&file.path) {
                    continue;
                }
            }
            out.push_str(&file.colorized_diff(options));
            out.push('\n');
        }

        out
    }

    /// Formats the Git-style `--stat` summary.
    pub fn format_stat(&self) -> String {
        self.summary.format_table()
    }

    /// Saves the unified patch string to a file at the specified path.
    pub fn save_to_file(
        &self,
        path: impl AsRef<Path>,
        options: &SessionPatchOptions,
    ) -> io::Result<PathBuf> {
        let path = path.as_ref();
        if let Some(parent) = path.parent() {
            if !parent.exists() {
                fs::create_dir_all(parent)?;
            }
        }

        let patch_text = self.to_unified_diff(options);
        fs::write(path, patch_text.as_bytes())?;
        Ok(path.to_path_buf())
    }

    /// Saves the patch as `session.patch` inside the specified directory.
    pub fn save_session_patch(&self, directory: impl AsRef<Path>) -> io::Result<PathBuf> {
        let target = directory.as_ref().join(DEFAULT_SESSION_PATCH_FILENAME);
        self.save_to_file(&target, &SessionPatchOptions::default())
    }

    /// Saves the patch as `session.patch` in the current working directory or session working directory.
    pub fn save_default(&self) -> io::Result<PathBuf> {
        let dir = PathBuf::from(".");
        self.save_session_patch(dir)
    }

    /// Creates an inverted patch that undoes/rolls back all changes in this session.
    pub fn reverse(&self) -> Self {
        let reversed_files: Vec<SessionFilePatch> = self.files.iter().map(|f| f.reverse()).collect();
        let summary = SessionPatchSummary::from_files(&reversed_files);
        Self {
            session_id: self.session_id.clone(),
            session_title: self.session_title.as_ref().map(|t| format!("Rollback: {}", t)),
            model: self.model.clone(),
            created_at: Utc::now().to_rfc3339(),
            files: reversed_files,
            summary,
            metadata: self.metadata.clone(),
        }
    }

    /// Filters files in this patch by a predicate.
    pub fn filter<F>(&self, predicate: F) -> Self
    where
        F: Fn(&SessionFilePatch) -> bool,
    {
        let filtered: Vec<SessionFilePatch> = self.files.iter().filter(|f| predicate(f)).cloned().collect();
        let summary = SessionPatchSummary::from_files(&filtered);
        Self {
            session_id: self.session_id.clone(),
            session_title: self.session_title.clone(),
            model: self.model.clone(),
            created_at: self.created_at.clone(),
            files: filtered,
            summary,
            metadata: self.metadata.clone(),
        }
    }

    /// Checks if all file patches can be cleanly applied to the target directory.
    pub fn can_apply_cleanly(&self, target_dir: &Path) -> bool {
        for file in &self.files {
            let full_path = target_dir.join(&file.path);
            match file.kind {
                FileEditKind::Created => {
                    // Creating is clean if the file doesn't already exist with conflicting content
                    if full_path.exists() {
                        if let Ok(existing) = fs::read_to_string(&full_path) {
                            if let Some(new_content) = &file.new_content {
                                if &existing == new_content {
                                    continue;
                                }
                            }
                        }
                        return false;
                    }
                }
                FileEditKind::Deleted => {
                    // Deletion is clean if file exists and matches old_content
                    if !full_path.exists() {
                        return false;
                    }
                }
                FileEditKind::Modified => {
                    if !full_path.exists() {
                        return false;
                    }
                    if let Ok(existing) = fs::read_to_string(&full_path) {
                        if let Some(old_content) = &file.old_content {
                            if &existing != old_content {
                                return false;
                            }
                        }
                    } else {
                        return false;
                    }
                }
            }
        }
        true
    }

    /// Applies the aggregated patch directly to the target directory.
    /// Returns the list of modified/created/deleted file paths.
    pub fn apply_to_dir(&self, target_dir: &Path) -> io::Result<Vec<PathBuf>> {
        let mut affected = Vec::new();

        for file in &self.files {
            let full_path = target_dir.join(&file.path);

            match file.kind {
                FileEditKind::Created => {
                    if let Some(parent) = full_path.parent() {
                        fs::create_dir_all(parent)?;
                    }
                    let content = file.new_content.as_deref().unwrap_or("");
                    fs::write(&full_path, content.as_bytes())?;
                    affected.push(full_path);
                }
                FileEditKind::Modified => {
                    if let Some(parent) = full_path.parent() {
                        fs::create_dir_all(parent)?;
                    }
                    let content = file.new_content.as_deref().unwrap_or("");
                    fs::write(&full_path, content.as_bytes())?;
                    affected.push(full_path);
                }
                FileEditKind::Deleted => {
                    if full_path.exists() {
                        fs::remove_file(&full_path)?;
                        affected.push(full_path);
                    }
                }
            }
        }

        Ok(affected)
    }
}

// ---------------------------------------------------------------------------
// SessionPatchBuilder
// ---------------------------------------------------------------------------

/// Fluent builder for creating a `SessionPatch` programmatically.
#[derive(Debug, Default)]
pub struct SessionPatchBuilder {
    session_id: Option<String>,
    session_title: Option<String>,
    model: Option<String>,
    files: Vec<SessionFilePatch>,
    metadata: HashMap<String, String>,
}

impl SessionPatchBuilder {
    /// Creates a new empty builder.
    pub fn new() -> Self {
        Self::default()
    }

    /// Sets session ID.
    pub fn session_id(mut self, id: impl Into<String>) -> Self {
        self.session_id = Some(id.into());
        self
    }

    /// Sets session title.
    pub fn session_title(mut self, title: impl Into<String>) -> Self {
        self.session_title = Some(title.into());
        self
    }

    /// Sets active model.
    pub fn model(mut self, model: impl Into<String>) -> Self {
        self.model = Some(model.into());
        self
    }

    /// Adds a file patch.
    pub fn add_file(mut self, file: SessionFilePatch) -> Self {
        self.files.push(file);
        self
    }

    /// Adds multiple file patches.
    pub fn add_files<I>(mut self, files: I) -> Self
    where
        I: IntoIterator<Item = SessionFilePatch>,
    {
        self.files.extend(files);
        self
    }

    /// Adds a newly created file.
    pub fn add_created(mut self, path: impl Into<PathBuf>, content: impl Into<String>) -> Self {
        self.files.push(SessionFilePatch::created(path, content));
        self
    }

    /// Adds a modified file.
    pub fn add_modified(
        mut self,
        path: impl Into<PathBuf>,
        old_content: impl Into<String>,
        new_content: impl Into<String>,
    ) -> Self {
        self.files
            .push(SessionFilePatch::modified(path, old_content, new_content));
        self
    }

    /// Adds a deleted file.
    pub fn add_deleted(mut self, path: impl Into<PathBuf>, old_content: impl Into<String>) -> Self {
        self.files
            .push(SessionFilePatch::deleted(path, old_content));
        self
    }

    /// Adds metadata key-value pair.
    pub fn metadata(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.metadata.insert(key.into(), value.into());
        self
    }

    /// Builds the `SessionPatch`.
    pub fn build(self) -> SessionPatch {
        let summary = SessionPatchSummary::from_files(&self.files);
        SessionPatch {
            session_id: self.session_id,
            session_title: self.session_title,
            model: self.model,
            created_at: Utc::now().to_rfc3339(),
            files: self.files,
            summary,
            metadata: self.metadata,
        }
    }
}

// ---------------------------------------------------------------------------
// Internal File Tracker for Session Reconstruction
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
struct FileTracker {
    original_path: PathBuf,
    initial_content: Option<String>,
    current_content: Option<String>,
    was_created: bool,
    was_deleted: bool,
    edit_count: usize,
}

// ---------------------------------------------------------------------------
// SessionPatchAggregator
// ---------------------------------------------------------------------------

/// Engine for extracting and aggregating file modifications from sessions,
/// checkpoints, and tool call histories.
#[derive(Debug, Default)]
pub struct SessionPatchAggregator;

impl SessionPatchAggregator {
    /// Extracts a `SessionPatch` from a `Session` with default options.
    pub fn from_session(session: &Session, options: &SessionPatchOptions) -> SessionPatch {
        let cwd = options
            .working_dir
            .as_deref()
            .or_else(|| session.working_dir());

        Self::from_session_with_cwd(session, cwd, options)
    }

    /// Extracts a `SessionPatch` from a `Session` using an explicit working directory.
    pub fn from_session_with_cwd(
        session: &Session,
        cwd: Option<&Path>,
        options: &SessionPatchOptions,
    ) -> SessionPatch {
        let mut patch = Self::from_messages(session.messages(), cwd, options);
        patch.session_id = Some(session.id_str());
        patch.session_title = session.title().map(|s| s.to_string());
        patch.model = Some(session.active_model().to_string());
        patch.created_at = session.created_at().to_string();
        patch
    }

    /// Extracts and aggregates all file edits from a chronological slice of conversation messages.
    pub fn from_messages(
        messages: &[Message],
        cwd: Option<&Path>,
        _options: &SessionPatchOptions,
    ) -> SessionPatch {
        let mut trackers: HashMap<PathBuf, FileTracker> = HashMap::new();

        // Map of tool_call_id -> result content / success status
        let mut tool_results: HashMap<String, String> = HashMap::new();
        for msg in messages {
            if msg.role == Role::Tool {
                if let Some(id) = &msg.tool_call_id {
                    tool_results.insert(id.clone(), msg.content.clone());
                }
            }
        }

        // Process assistant tool calls
        for msg in messages {
            if msg.role != Role::Assistant {
                continue;
            }

            if let Some(tool_calls) = &msg.tool_calls {
                for tool_call in tool_calls {
                    // If we have recorded tool result, check if it was an error
                    if let Some(res) = tool_results.get(&tool_call.id) {
                        if is_tool_error_result(res) {
                            continue;
                        }
                    }

                    Self::process_tool_call(tool_call, cwd, &mut trackers);
                }
            }
        }

        // Construct SessionFilePatch items from trackers
        let mut file_patches = Vec::new();

        // Sort keys for deterministic output
        let mut paths: Vec<PathBuf> = trackers.keys().cloned().collect();
        paths.sort();

        for path in paths {
            if let Some(tracker) = trackers.get(&path) {
                // Determine change kind
                let kind = match (&tracker.initial_content, &tracker.current_content) {
                    (None, Some(_)) => FileEditKind::Created,
                    (Some(_), None) => FileEditKind::Deleted,
                    (Some(initial), Some(current)) => {
                        if initial == current {
                            // Net zero change across the session -> omit
                            continue;
                        }
                        FileEditKind::Modified
                    }
                    (None, None) => continue, // Created then deleted -> net zero change
                };

                let file_patch = SessionFilePatch::new(
                    tracker.original_path.clone(),
                    kind,
                    tracker.initial_content.clone(),
                    tracker.current_content.clone(),
                );

                file_patches.push(file_patch);
            }
        }

        let summary = SessionPatchSummary::from_files(&file_patches);

        SessionPatch {
            session_id: None,
            session_title: None,
            model: None,
            created_at: Utc::now().to_rfc3339(),
            files: file_patches,
            summary,
            metadata: HashMap::new(),
        }
    }

    /// Reconstructs a `SessionPatch` from a slice of historical `Checkpoint` objects.
    pub fn from_checkpoints(
        checkpoints: &[Checkpoint],
        cwd: Option<&Path>,
        _options: &SessionPatchOptions,
    ) -> SessionPatch {
        let mut trackers: HashMap<PathBuf, FileTracker> = HashMap::new();

        for chk in checkpoints {
            for snap in chk.snapshots.values() {
                let norm_path = normalize_relative_path(&snap.original_path);

                let tracker = trackers.entry(norm_path.clone()).or_insert_with(|| {
                    let initial = snap.state_before.content_as_str().map(|s| s.to_string());
                    FileTracker {
                        original_path: PathBuf::from(&snap.original_path),
                        initial_content: initial,
                        current_content: None,
                        was_created: snap.state_before.is_absent(),
                        was_deleted: false,
                        edit_count: 0,
                    }
                });

                tracker.edit_count += 1;

                if let Some(after) = &snap.state_after {
                    tracker.current_content = after.content_as_str().map(|s| s.to_string());
                    tracker.was_deleted = after.is_absent();
                } else if let Some(cwd_path) = cwd {
                    let full = cwd_path.join(&snap.original_path);
                    if full.exists() {
                        if let Ok(disk_content) = fs::read_to_string(&full) {
                            tracker.current_content = Some(disk_content);
                        }
                    }
                }
            }
        }

        let mut file_patches = Vec::new();
        let mut paths: Vec<PathBuf> = trackers.keys().cloned().collect();
        paths.sort();

        for path in paths {
            if let Some(tracker) = trackers.get(&path) {
                let kind = match (&tracker.initial_content, &tracker.current_content) {
                    (None, Some(_)) => FileEditKind::Created,
                    (Some(_), None) => FileEditKind::Deleted,
                    (Some(initial), Some(current)) => {
                        if initial == current {
                            continue;
                        }
                        FileEditKind::Modified
                    }
                    (None, None) => continue,
                };

                file_patches.push(SessionFilePatch::new(
                    tracker.original_path.clone(),
                    kind,
                    tracker.initial_content.clone(),
                    tracker.current_content.clone(),
                ));
            }
        }

        let summary = SessionPatchSummary::from_files(&file_patches);

        SessionPatch {
            session_id: None,
            session_title: None,
            model: None,
            created_at: Utc::now().to_rfc3339(),
            files: file_patches,
            summary,
            metadata: HashMap::new(),
        }
    }

    /// Internal helper to process a single tool call and update file trackers.
    fn process_tool_call(
        tool_call: &ToolCall,
        cwd: Option<&Path>,
        trackers: &mut HashMap<PathBuf, FileTracker>,
    ) {
        let name = tool_call.name.to_lowercase();
        let args_val: serde_json::Value =
            serde_json::from_str(&tool_call.arguments).unwrap_or(serde_json::Value::Null);

        match name.as_str() {
            "write" | "write_file" => {
                let path_str = extract_string_arg(&args_val, &["path", "file_path", "file", "target"]);
                let content = extract_string_arg(&args_val, &["content", "text", "body", "code"]);

                if let (Some(path_s), Some(new_content)) = (path_str, content) {
                    let norm = normalize_relative_path(&path_s);
                    let orig_path = PathBuf::from(path_s);

                    let tracker = trackers.entry(norm).or_insert_with(|| {
                        let initial = read_disk_baseline(&orig_path, cwd);
                        let was_created = initial.is_none();
                        FileTracker {
                            original_path: orig_path,
                            initial_content: initial,
                            current_content: None,
                            was_created,
                            was_deleted: false,
                            edit_count: 0,
                        }
                    });

                    tracker.edit_count += 1;
                    tracker.current_content = Some(new_content);
                    tracker.was_deleted = false;
                }
            }
            "edit" | "edit_file" => {
                let path_str = extract_string_arg(&args_val, &["path", "file_path", "file", "target"]);
                let old_text = extract_string_arg(&args_val, &["old_text", "old_string", "old_content", "find", "search"]);
                let new_text = extract_string_arg(&args_val, &["new_text", "new_string", "new_content", "replace"]);

                if let (Some(path_s), Some(old_t), Some(new_t)) = (path_str, old_text, new_text) {
                    let norm = normalize_relative_path(&path_s);
                    let orig_path = PathBuf::from(path_s);

                    let tracker = trackers.entry(norm).or_insert_with(|| {
                        let initial = read_disk_baseline(&orig_path, cwd);
                        // If file didn't exist on disk, synthesize initial text around the edit
                        let base = initial.or_else(|| Some(old_t.clone()));
                        FileTracker {
                            original_path: orig_path,
                            initial_content: base.clone(),
                            current_content: base,
                            was_created: false,
                            was_deleted: false,
                            edit_count: 0,
                        }
                    });

                    tracker.edit_count += 1;

                    let current = tracker
                        .current_content
                        .clone()
                        .or_else(|| tracker.initial_content.clone())
                        .unwrap_or_default();

                    if current.contains(&old_t) {
                        // Apply exact substitution
                        let updated = current.replacen(&old_t, &new_t, 1);
                        tracker.current_content = Some(updated);
                    } else if current.is_empty() {
                        tracker.current_content = Some(new_t);
                    }
                }
            }
            "patch" | "apply_patch" => {
                if let Some(patch_str) = extract_string_arg(&args_val, &["patch", "diff", "unified_diff", "content"]) {
                    if let Ok(file_patches) = parse_unified_diff(&patch_str) {
                        for fp in file_patches {
                            if let Some(target_p) = fp.target_path(1, false) {
                                let path_s = target_p.to_string_lossy().to_string();
                                let norm = normalize_relative_path(&path_s);
                                let orig_path = PathBuf::from(path_s);

                                let tracker = trackers.entry(norm).or_insert_with(|| {
                                    let initial = read_disk_baseline(&orig_path, cwd);
                                    let was_created = fp.is_new || initial.is_none();
                                    FileTracker {
                                        original_path: orig_path,
                                        initial_content: initial,
                                        current_content: None,
                                        was_created,
                                        was_deleted: false,
                                        edit_count: 0,
                                    }
                                });

                                tracker.edit_count += 1;

                                if fp.is_deleted {
                                    tracker.current_content = None;
                                    tracker.was_deleted = true;
                                } else {
                                    let base = tracker
                                        .current_content
                                        .as_deref()
                                        .or(tracker.initial_content.as_deref())
                                        .unwrap_or("");

                                    let patch_opts = PatchOptions {
                                        fuzz: 2,
                                        dry_run: false,
                                        reverse: false,
                                        strip: 1,
                                        target_path: None,
                                    };

                                    if let Ok(applied) = apply_file_patch_to_string(base, &fp, &patch_opts) {
                                        tracker.current_content = Some(applied.0);
                                    }
                                }
                            }
                        }
                    }
                }
            }
            "delete_file" | "remove_file" | "rm" => {
                let path_str = extract_string_arg(&args_val, &["path", "file_path", "file", "target"]);
                if let Some(path_s) = path_str {
                    let norm = normalize_relative_path(&path_s);
                    let orig_path = PathBuf::from(path_s);

                    let tracker = trackers.entry(norm).or_insert_with(|| {
                        let initial = read_disk_baseline(&orig_path, cwd);
                        FileTracker {
                            original_path: orig_path,
                            initial_content: initial,
                            current_content: None,
                            was_created: false,
                            was_deleted: true,
                            edit_count: 0,
                        }
                    });

                    tracker.edit_count += 1;
                    tracker.current_content = None;
                    tracker.was_deleted = true;
                }
            }
            _ => {}
        }
    }
}

// ---------------------------------------------------------------------------
// Helper Functions
// ---------------------------------------------------------------------------

/// Normalizes a path to clean relative components without leading `./` or drive roots.
fn normalize_relative_path(path_str: &str) -> PathBuf {
    let clean = path_str.trim().replace('\\', "/");
    let clean = clean.trim_start_matches("./");
    PathBuf::from(clean)
}

/// Helper to read baseline content from disk if available.
fn read_disk_baseline(path: &Path, cwd: Option<&Path>) -> Option<String> {
    if let Some(cwd_dir) = cwd {
        let full = resolve_path(&path.to_string_lossy(), cwd_dir);
        if full.exists() && full.is_file() {
            return fs::read_to_string(&full).ok();
        }
    } else if path.exists() && path.is_file() {
        return fs::read_to_string(path).ok();
    }
    None
}

/// Helper to extract string arguments from JSON Values.
fn extract_string_arg(val: &serde_json::Value, keys: &[&str]) -> Option<String> {
    for k in keys {
        if let Some(s) = val.get(*k).and_then(|v| v.as_str()) {
            return Some(s.to_string());
        }
    }
    None
}

/// Helper to determine if a tool result string indicates failure.
fn is_tool_error_result(result_text: &str) -> bool {
    let lower = result_text.trim().to_lowercase();
    lower.starts_with("error:")
        || lower.starts_with("failed to")
        || lower.starts_with("io error:")
        || lower.contains("file not found")
        || lower.contains("permission denied")
        || lower.contains("patch failed")
}

// ---------------------------------------------------------------------------
// Public Exporter Functions
// ---------------------------------------------------------------------------

/// Generates a complete `SessionPatch` for the given session.
pub fn export_session_patch(session: &Session, options: &SessionPatchOptions) -> SessionPatch {
    SessionPatchAggregator::from_session(session, options)
}

/// Generates a unified diff string representing all file modifications in a session.
pub fn export_session_patch_string(session: &Session, options: &SessionPatchOptions) -> String {
    let patch = export_session_patch(session, options);
    patch.to_unified_diff(options)
}

/// Saves all session file edits into a single `session.patch` file at the specified path.
pub fn export_session_patch_file(
    session: &Session,
    path: impl AsRef<Path>,
    options: &SessionPatchOptions,
) -> io::Result<PathBuf> {
    let patch = export_session_patch(session, options);
    patch.save_to_file(path, options)
}

/// Saves all session file edits into `session.patch` in the current working directory.
pub fn export_session_patch_default(session: &Session) -> io::Result<PathBuf> {
    let patch = export_session_patch(session, &SessionPatchOptions::default());
    patch.save_default()
}

/// Formats a concise summary and `--stat` breakdown for a session patch.
pub fn format_session_patch_summary(patch: &SessionPatch) -> String {
    patch.format_stat()
}

// ===========================================================================
// SURGICAL CONVERSATIONAL SESSION PATCHING SUBSYSTEM
// ===========================================================================

/// Error conditions that can occur during surgical session patching and turn manipulation.
#[derive(Debug, Error)]
pub enum SessionPatchError {
    #[error("Turn index {turn_index} is out of bounds (session has {total_turns} turn(s))")]
    TurnNotFound {
        turn_index: usize,
        total_turns: usize,
    },

    #[error("Invalid turn range {start}..={end} (session has {total_turns} turn(s))")]
    TurnRangeInvalid {
        start: usize,
        end: usize,
        total_turns: usize,
    },

    #[error("Message index {message_index} is out of bounds (session has {total_messages} message(s))")]
    MessageNotFound {
        message_index: usize,
        total_messages: usize,
    },

    #[error("Cannot perform operation on empty turn / empty message list")]
    EmptyTurn,

    #[error("Session integrity validation failed: {details}")]
    IntegrityError {
        details: String,
    },

    #[error("Session patching conflict: {0}")]
    Conflict(String),

    #[error("Invalid patch operation: {0}")]
    InvalidOperation(String),

    #[error("I/O error during session patching: {0}")]
    Io(#[from] io::Error),

    #[error("JSON serialization error: {0}")]
    Serialization(#[from] serde_json::Error),
}

/// Relative position for inserting a turn or message.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum InsertPosition {
    /// Insert before the designated reference index.
    Before,
    /// Insert after the designated reference index.
    After,
}

/// Container for a turn's conversational messages when constructing or modifying turns.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TurnData {
    /// List of messages comprising the turn (typically User message + optional Assistant response / Tool calls).
    pub messages: Vec<Message>,
}

impl TurnData {
    /// Creates a standard user-assistant turn.
    pub fn new(user_content: impl Into<String>, assistant_content: impl Into<String>) -> Self {
        Self {
            messages: vec![
                Message::user(user_content),
                Message::assistant(assistant_content),
            ],
        }
    }

    /// Creates a user-only turn (e.g. pending assistant response).
    pub fn user_only(user_content: impl Into<String>) -> Self {
        Self {
            messages: vec![Message::user(user_content)],
        }
    }

    /// Creates an assistant-only turn (e.g. system greeting or unsolicited message).
    pub fn assistant_only(assistant_content: impl Into<String>) -> Self {
        Self {
            messages: vec![Message::assistant(assistant_content)],
        }
    }

    /// Creates a turn from an arbitrary list of messages.
    pub fn from_messages(messages: Vec<Message>) -> Self {
        Self { messages }
    }

    /// Creates a turn with tool calls and their execution results.
    pub fn with_tools(
        user_content: impl Into<String>,
        assistant_with_tool_calls: Message,
        tool_results: Vec<Message>,
        final_assistant_message: Option<Message>,
    ) -> Self {
        let mut messages = vec![Message::user(user_content), assistant_with_tool_calls];
        messages.extend(tool_results);
        if let Some(final_msg) = final_assistant_message {
            messages.push(final_msg);
        }
        Self { messages }
    }

    /// Returns the text content of the first User message in this turn, if any.
    pub fn user_content(&self) -> Option<&str> {
        self.messages
            .iter()
            .find(|m| m.role == Role::User)
            .map(|m| m.content.as_str())
    }

    /// Returns the text content of the last Assistant message in this turn, if any.
    pub fn assistant_content(&self) -> Option<&str> {
        self.messages
            .iter()
            .rev()
            .find(|m| m.role == Role::Assistant)
            .map(|m| m.content.as_str())
    }

    /// Returns the number of tool calls initiated in this turn.
    pub fn tool_calls_count(&self) -> usize {
        self.messages
            .iter()
            .filter_map(|m| m.tool_calls.as_ref())
            .map(|calls| calls.len())
            .sum()
    }

    /// Returns true if this turn has no messages.
    pub fn is_empty(&self) -> bool {
        self.messages.is_empty()
    }

    /// Returns the number of messages in this turn.
    pub fn len(&self) -> usize {
        self.messages.len()
    }

    /// Estimates the total tokens consumed by this turn.
    pub fn estimate_tokens(&self) -> usize {
        estimate_messages_tokens(&self.messages)
    }
}

/// Represents a discrete conversational turn extracted from a session for inspection and patching.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PatchableTurn {
    /// 1-based turn index (1, 2, 3...)
    pub turn_index: usize,
    /// Starting message index in `session.messages` (inclusive).
    pub start_message_index: usize,
    /// Ending message index in `session.messages` (exclusive).
    pub end_message_index: usize,
    /// User prompt content initiating this turn, if any.
    pub user_message: Option<String>,
    /// Final assistant response content in this turn, if any.
    pub assistant_message: Option<String>,
    /// Number of tool calls executed during this turn.
    pub tool_calls_count: usize,
    /// Total message count in this turn.
    pub message_count: usize,
    /// Estimated token count for all messages in this turn.
    pub estimated_tokens: usize,
    /// The actual messages belonging to this turn.
    pub messages: Vec<Message>,
}

impl PatchableTurn {
    /// Generates a human-readable one-line preview of the turn.
    pub fn preview(&self) -> String {
        let user = self
            .user_message
            .as_deref()
            .map(|s| {
                let trimmed = s.trim();
                let chars: String = trimmed.chars().take(40).collect();
                if trimmed.chars().count() > 40 {
                    format!("{}...", chars)
                } else {
                    chars
                }
            })
            .unwrap_or_else(|| "<no user prompt>".to_string());

        let assistant = self
            .assistant_message
            .as_deref()
            .map(|s| {
                let trimmed = s.trim();
                let chars: String = trimmed.chars().take(40).collect();
                if trimmed.chars().count() > 40 {
                    format!("{}...", chars)
                } else {
                    chars
                }
            })
            .unwrap_or_else(|| "<no response>".to_string());

        format!(
            "Turn {}: User: \"{}\" -> Assistant: \"{}\" [{} msgs, ~{} tokens]",
            self.turn_index, user, assistant, self.message_count, self.estimated_tokens
        )
    }
}

/// Identifies and extracts all conversational turns from a `Session`.
pub fn extract_patchable_turns(session: &Session) -> Vec<PatchableTurn> {
    extract_patchable_turns_from_messages(&session.messages)
}

/// Identifies and extracts all conversational turns from an arbitrary slice of `Message`s.
pub fn extract_patchable_turns_from_messages(messages: &[Message]) -> Vec<PatchableTurn> {
    let mut turns = Vec::new();
    if messages.is_empty() {
        return turns;
    }

    let user_indices: Vec<usize> = messages
        .iter()
        .enumerate()
        .filter(|(_, m)| m.role == Role::User)
        .map(|(i, _)| i)
        .collect();

    if user_indices.is_empty() {
        let first_non_sys = messages.iter().position(|m| m.role != Role::System);
        if let Some(start) = first_non_sys {
            let end = messages.len();
            let slice = &messages[start..end];
            let assistant_msg = slice
                .iter()
                .rev()
                .find(|m| m.role == Role::Assistant)
                .map(|m| m.content.clone());
            let tool_calls_count = slice
                .iter()
                .filter_map(|m| m.tool_calls.as_ref())
                .map(|tc| tc.len())
                .sum();
            let tokens = estimate_messages_tokens(slice);
            turns.push(PatchableTurn {
                turn_index: 1,
                start_message_index: start,
                end_message_index: end,
                user_message: None,
                assistant_message: assistant_msg,
                tool_calls_count,
                message_count: end - start,
                estimated_tokens: tokens,
                messages: slice.to_vec(),
            });
        }
        return turns;
    }

    let first_user_idx = user_indices[0];
    let first_non_sys = messages.iter().position(|m| m.role != Role::System);
    let mut current_turn_idx = 1;

    if let Some(start) = first_non_sys {
        if start < first_user_idx {
            let slice = &messages[start..first_user_idx];
            let assistant_msg = slice
                .iter()
                .rev()
                .find(|m| m.role == Role::Assistant)
                .map(|m| m.content.clone());
            let tool_calls_count = slice
                .iter()
                .filter_map(|m| m.tool_calls.as_ref())
                .map(|tc| tc.len())
                .sum();
            let tokens = estimate_messages_tokens(slice);
            turns.push(PatchableTurn {
                turn_index: current_turn_idx,
                start_message_index: start,
                end_message_index: first_user_idx,
                user_message: None,
                assistant_message: assistant_msg,
                tool_calls_count,
                message_count: first_user_idx - start,
                estimated_tokens: tokens,
                messages: slice.to_vec(),
            });
            current_turn_idx += 1;
        }
    }

    for (i, &start_idx) in user_indices.iter().enumerate() {
        let end_idx = if i + 1 < user_indices.len() {
            user_indices[i + 1]
        } else {
            messages.len()
        };

        let slice = &messages[start_idx..end_idx];
        let user_msg = slice.first().map(|m| m.content.clone());
        let assistant_msg = slice
            .iter()
            .rev()
            .find(|m| m.role == Role::Assistant)
            .map(|m| m.content.clone());
        let tool_calls_count = slice
            .iter()
            .filter_map(|m| m.tool_calls.as_ref())
            .map(|tc| tc.len())
            .sum();
        let tokens = estimate_messages_tokens(slice);

        turns.push(PatchableTurn {
            turn_index: current_turn_idx,
            start_message_index: start_idx,
            end_message_index: end_idx,
            user_message: user_msg,
            assistant_message: assistant_msg,
            tool_calls_count,
            message_count: end_idx - start_idx,
            estimated_tokens: tokens,
            messages: slice.to_vec(),
        });
        current_turn_idx += 1;
    }

    turns
}

/// Individual atomic patch operation on a conversational session.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum SessionPatchOp {
    /// Modifies the user message in turn `turn_index` (1-based).
    ModifyUser {
        turn_index: usize,
        new_content: String,
    },
    /// Modifies the assistant message in turn `turn_index` (1-based).
    ModifyAssistant {
        turn_index: usize,
        new_content: String,
    },
    /// Replaces the entire turn `turn_index` (1-based) with new turn data.
    ModifyTurn {
        turn_index: usize,
        turn: TurnData,
    },
    /// Inserts a new turn before or after turn `turn_index` (1-based).
    InsertTurn {
        turn_index: usize,
        turn: TurnData,
        position: InsertPosition,
    },
    /// Appends a new turn to the end of the conversation.
    AppendTurn {
        turn: TurnData,
    },
    /// Prepends a new turn to the beginning of the conversation.
    PrependTurn {
        turn: TurnData,
    },
    /// Deletes turn `turn_index` (1-based).
    DeleteTurn {
        turn_index: usize,
    },
    /// Deletes a range of turns [start_turn..=end_turn] (1-based, inclusive).
    DeleteTurnRange {
        start_turn: usize,
        end_turn: usize,
    },
    /// Truncates the session, keeping only turns 1..=turn_index.
    TruncateAfterTurn {
        turn_index: usize,
    },
    /// Truncates the session, dropping turns 1..turn_index and keeping turns from turn_index onward.
    TruncateBeforeTurn {
        turn_index: usize,
    },
    /// Swaps two turns by their 1-based indices.
    SwapTurns {
        turn_a: usize,
        turn_b: usize,
    },
    /// Modifies a specific message by 0-based message index.
    ModifyMessage {
        message_index: usize,
        new_message: Message,
    },
    /// Deletes a specific message by 0-based message index.
    DeleteMessage {
        message_index: usize,
    },
    /// Inserts a message at 0-based message index.
    InsertMessage {
        message_index: usize,
        message: Message,
    },
    /// Sets or removes the session system prompt.
    SetSystemPrompt {
        system_prompt: Option<String>,
    },
    /// Sets the active LLM model.
    SetActiveModel {
        model: String,
    },
    /// Sets or removes the session title.
    SetTitle {
        title: Option<String>,
    },
    /// Sets a metadata key-value pair.
    SetMetadata {
        key: String,
        value: String,
    },
    /// Removes a metadata key.
    RemoveMetadata {
        key: String,
    },
}

pub type TurnPatchOp = SessionPatchOp;

/// Detailed report of token recomputation after session patching.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct TokenRecomputeReport {
    /// Recalculated prompt tokens across system prompt and input messages.
    pub prompt_tokens: u64,
    /// Recalculated completion tokens generated by the assistant.
    pub completion_tokens: u64,
    /// Total recalculated tokens (prompt + completion).
    pub total_tokens: u64,
    /// Total number of conversational turns.
    pub total_turns: u64,
    /// Token count before recomputation.
    pub previous_total_tokens: u64,
    /// Net token delta (positive = increase, negative = decrease).
    pub token_delta: i64,
    /// Total messages in the session.
    pub message_count: usize,
}

impl TokenRecomputeReport {
    /// Formats a concise summary string of the token recomputation.
    pub fn format_summary(&self) -> String {
        let delta_str = if self.token_delta >= 0 {
            format!("+{}", self.token_delta)
        } else {
            format!("{}", self.token_delta)
        };
        format!(
            "Tokens: {} total ({} prompt, {} completion, delta: {}) across {} turns ({} messages)",
            self.total_tokens,
            self.prompt_tokens,
            self.completion_tokens,
            delta_str,
            self.total_turns,
            self.message_count
        )
    }
}

/// Recomputes token usage and turn counts for an entire session.
pub fn recompute_session_token_stats(session: &mut Session) -> TokenRecomputeReport {
    let previous_total_tokens = session.token_stats.total_tokens;
    let mut prompt_tokens = 0u64;
    let mut completion_tokens = 0u64;

    if let Some(sys) = session.system_prompt.as_deref() {
        if !sys.is_empty() {
            prompt_tokens += (estimate_text_tokens(sys) + 4) as u64;
        }
    }

    for msg in &session.messages {
        let msg_tokens = estimate_message_tokens(msg) as u64;
        match msg.role {
            Role::System | Role::User | Role::Tool => {
                prompt_tokens += msg_tokens;
            }
            Role::Assistant => {
                completion_tokens += msg_tokens;
            }
        }
    }

    let total_tokens = prompt_tokens + completion_tokens;
    let turns = extract_patchable_turns(session);
    let total_turns = turns.len() as u64;

    session.token_stats.prompt_tokens = prompt_tokens;
    session.token_stats.completion_tokens = completion_tokens;
    session.token_stats.total_tokens = total_tokens;
    session.token_stats.total_turns = total_turns;
    session.touch();

    TokenRecomputeReport {
        prompt_tokens,
        completion_tokens,
        total_tokens,
        total_turns,
        previous_total_tokens,
        token_delta: total_tokens as i64 - previous_total_tokens as i64,
        message_count: session.messages.len(),
    }
}

/// Severity level of a message integrity issue.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum IntegritySeverity {
    /// Critical error that violates message protocol (e.g. orphan tool result).
    Error,
    /// Structural anomaly or warning (e.g. consecutive same-role messages, dangling tool call).
    Warning,
}

/// A specific integrity issue detected within a session.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IntegrityIssue {
    /// Severity level of the issue.
    pub severity: IntegritySeverity,
    /// Zero-based message index where the issue was observed, if applicable.
    pub message_index: Option<usize>,
    /// One-based turn index where the issue was observed, if applicable.
    pub turn_index: Option<usize>,
    /// Machine-readable issue code.
    pub code: String,
    /// Human-readable explanation.
    pub description: String,
    /// Whether this issue can be automatically resolved via `repair_session_integrity`.
    pub auto_fixable: bool,
}

/// Comprehensive report on conversational session integrity.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionIntegrityReport {
    /// True if there are zero errors (warnings may still be present).
    pub is_valid: bool,
    /// List of all detected integrity issues.
    pub issues: Vec<IntegrityIssue>,
    /// Count of critical errors.
    pub error_count: usize,
    /// Count of non-critical warnings.
    pub warning_count: usize,
}

impl SessionIntegrityReport {
    /// Creates a report with no issues (valid session).
    pub fn valid() -> Self {
        Self {
            is_valid: true,
            issues: Vec::new(),
            error_count: 0,
            warning_count: 0,
        }
    }

    /// Creates a report from a list of issues.
    pub fn from_issues(issues: Vec<IntegrityIssue>) -> Self {
        let error_count = issues
            .iter()
            .filter(|i| i.severity == IntegritySeverity::Error)
            .count();
        let warning_count = issues
            .iter()
            .filter(|i| i.severity == IntegritySeverity::Warning)
            .count();
        Self {
            is_valid: error_count == 0,
            issues,
            error_count,
            warning_count,
        }
    }

    /// Returns true if any critical errors exist.
    pub fn has_errors(&self) -> bool {
        self.error_count > 0
    }

    /// Returns true if any warnings exist.
    pub fn has_warnings(&self) -> bool {
        self.warning_count > 0
    }

    /// Formats a human-readable summary of the integrity report.
    pub fn format_report(&self) -> String {
        if self.issues.is_empty() {
            return "Session integrity: Valid (0 errors, 0 warnings)".to_string();
        }

        let mut lines = Vec::new();
        lines.push(format!(
            "Session integrity: {} ({} errors, {} warnings)",
            if self.is_valid { "Valid with warnings" } else { "Invalid" },
            self.error_count,
            self.warning_count
        ));

        for (idx, issue) in self.issues.iter().enumerate() {
            let loc = match (issue.turn_index, issue.message_index) {
                (Some(t), Some(m)) => format!("Turn {}, Msg {}", t, m),
                (Some(t), None) => format!("Turn {}", t),
                (None, Some(m)) => format!("Msg {}", m),
                (None, None) => "Session".to_string(),
            };
            let sev = match issue.severity {
                IntegritySeverity::Error => "ERROR",
                IntegritySeverity::Warning => "WARN",
            };
            let fix = if issue.auto_fixable { " [auto-fixable]" } else { "" };
            lines.push(format!(
                "  {}. [{}] [{}] {}: {}{}",
                idx + 1,
                sev,
                issue.code,
                loc,
                issue.description,
                fix
            ));
        }

        lines.join("\n")
    }
}

/// Analyzes a session's message stream for protocol and structural integrity anomalies.
pub fn validate_session_integrity(session: &Session) -> SessionIntegrityReport {
    let mut issues = Vec::new();
    let turns = extract_patchable_turns(session);

    let mut open_tool_calls: HashMap<String, usize> = HashMap::new();
    let mut first_user_seen = false;
    let mut prev_role: Option<Role> = None;

    for (msg_idx, msg) in session.messages.iter().enumerate() {
        let turn_num = turns.iter().find_map(|t| {
            if msg_idx >= t.start_message_index && msg_idx < t.end_message_index {
                Some(t.turn_index)
            } else {
                None
            }
        });

        match msg.role {
            Role::System => {
                if first_user_seen {
                    issues.push(IntegrityIssue {
                        severity: IntegritySeverity::Warning,
                        message_index: Some(msg_idx),
                        turn_index: turn_num,
                        code: "STRAY_SYSTEM_MESSAGE".to_string(),
                        description: "System message found after conversation initiation".to_string(),
                        auto_fixable: true,
                    });
                }
            }
            Role::User => {
                first_user_seen = true;

                if !open_tool_calls.is_empty() {
                    for (call_id, asst_idx) in open_tool_calls.drain() {
                        issues.push(IntegrityIssue {
                            severity: IntegritySeverity::Warning,
                            message_index: Some(asst_idx),
                            turn_index: turn_num,
                            code: "DANGLING_TOOL_CALL".to_string(),
                            description: format!(
                                "Tool call '{}' was never answered with a tool result before user message",
                                call_id
                            ),
                            auto_fixable: true,
                        });
                    }
                }

                if prev_role == Some(Role::User) {
                    issues.push(IntegrityIssue {
                        severity: IntegritySeverity::Warning,
                        message_index: Some(msg_idx),
                        turn_index: turn_num,
                        code: "CONSECUTIVE_USER_MESSAGES".to_string(),
                        description: "Consecutive user message without assistant response".to_string(),
                        auto_fixable: true,
                    });
                }
            }
            Role::Assistant => {
                if let Some(tool_calls) = &msg.tool_calls {
                    for call in tool_calls {
                        open_tool_calls.insert(call.id.clone(), msg_idx);
                    }
                }

                if prev_role == Some(Role::Assistant) && msg.tool_calls.is_none() {
                    issues.push(IntegrityIssue {
                        severity: IntegritySeverity::Warning,
                        message_index: Some(msg_idx),
                        turn_index: turn_num,
                        code: "CONSECUTIVE_ASSISTANT_MESSAGES".to_string(),
                        description: "Consecutive assistant messages without intervening user or tool interaction".to_string(),
                        auto_fixable: true,
                    });
                }
            }
            Role::Tool => {
                match &msg.tool_call_id {
                    Some(call_id) => {
                        if open_tool_calls.remove(call_id).is_none() {
                            issues.push(IntegrityIssue {
                                severity: IntegritySeverity::Error,
                                message_index: Some(msg_idx),
                                turn_index: turn_num,
                                code: "ORPHAN_TOOL_RESULT".to_string(),
                                description: format!(
                                    "Tool message references call id '{}' which has no open assistant tool call",
                                    call_id
                                ),
                                auto_fixable: true,
                            });
                        }
                    }
                    None => {
                        issues.push(IntegrityIssue {
                            severity: IntegritySeverity::Error,
                            message_index: Some(msg_idx),
                            turn_index: turn_num,
                            code: "MISSING_TOOL_CALL_ID".to_string(),
                            description: "Tool message is missing required 'tool_call_id' field".to_string(),
                            auto_fixable: true,
                        });
                    }
                }
            }
        }

        let has_text = !msg.content.trim().is_empty();
        let has_tools = msg.tool_calls.as_ref().map_or(false, |tc| !tc.is_empty());
        if !has_text && !has_tools {
            issues.push(IntegrityIssue {
                severity: IntegritySeverity::Warning,
                message_index: Some(msg_idx),
                turn_index: turn_num,
                code: "EMPTY_MESSAGE".to_string(),
                description: "Message content is empty and contains no tool calls".to_string(),
                auto_fixable: true,
            });
        }

        prev_role = Some(msg.role);
    }

    if !open_tool_calls.is_empty() {
        for (call_id, asst_idx) in open_tool_calls {
            issues.push(IntegrityIssue {
                severity: IntegritySeverity::Warning,
                message_index: Some(asst_idx),
                turn_index: turns.last().map(|t| t.turn_index),
                code: "DANGLING_TOOL_CALL".to_string(),
                description: format!(
                    "Tool call '{}' was never answered before session conclusion",
                    call_id
                ),
                auto_fixable: true,
            });
        }
    }

    SessionIntegrityReport::from_issues(issues)
}

/// Configuration options for repairing session integrity anomalies.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IntegrityRepairOptions {
    /// Remove or convert orphaned tool response messages that have no matching tool call.
    pub fix_orphan_tool_results: bool,
    /// Insert synthetic empty result responses for tool calls that never received an execution result.
    pub fix_dangling_tool_calls: bool,
    /// Relocate stray system messages to session system_prompt or beginning.
    pub fix_stray_system_messages: bool,
    /// Merge consecutive user or consecutive assistant messages.
    pub merge_consecutive_same_roles: bool,
    /// Remove messages that have completely empty content and no tool calls.
    pub remove_empty_messages: bool,
    /// Automatically recompute token statistics after repairs.
    pub recompute_tokens: bool,
}

impl Default for IntegrityRepairOptions {
    fn default() -> Self {
        Self {
            fix_orphan_tool_results: true,
            fix_dangling_tool_calls: true,
            fix_stray_system_messages: true,
            merge_consecutive_same_roles: false,
            remove_empty_messages: true,
            recompute_tokens: true,
        }
    }
}

/// Summary of repairs applied to a session.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct IntegrityRepairReport {
    /// Number of orphan tool results fixed/removed.
    pub orphan_tools_fixed: usize,
    /// Number of dangling tool calls synthesized.
    pub dangling_calls_fixed: usize,
    /// Number of stray system messages relocated.
    pub stray_systems_fixed: usize,
    /// Number of empty messages purged.
    pub empty_messages_removed: usize,
    /// Number of consecutive same-role messages merged.
    pub consecutive_merged: usize,
    /// Total repairs performed.
    pub total_repairs: usize,
    /// Token recompute report if tokens were recalculated.
    pub token_report: Option<TokenRecomputeReport>,
}

impl IntegrityRepairReport {
    /// Returns true if any repairs were performed.
    pub fn has_repairs(&self) -> bool {
        self.total_repairs > 0
    }
}

/// Automatically repairs common structural integrity anomalies in a session.
pub fn repair_session_integrity(
    session: &mut Session,
    options: &IntegrityRepairOptions,
) -> IntegrityRepairReport {
    let mut orphan_tools_fixed = 0;
    let mut dangling_calls_fixed = 0;
    let mut stray_systems_fixed = 0;
    let mut empty_messages_removed = 0;
    let mut consecutive_merged = 0;

    let mut new_messages: Vec<Message> = Vec::new();
    let mut stray_system_texts: Vec<String> = Vec::new();
    let mut pending_tool_calls: HashMap<String, ToolCall> = HashMap::new();
    let mut first_user_seen = false;

    for msg in session.messages.drain(..) {
        if options.remove_empty_messages {
            let has_text = !msg.content.trim().is_empty();
            let has_tools = msg.tool_calls.as_ref().map_or(false, |tc| !tc.is_empty());
            if !has_text && !has_tools {
                empty_messages_removed += 1;
                continue;
            }
        }

        if msg.role == Role::System {
            if first_user_seen && options.fix_stray_system_messages {
                stray_system_texts.push(msg.content);
                stray_systems_fixed += 1;
                continue;
            }
        } else if msg.role == Role::User {
            first_user_seen = true;

            if options.fix_dangling_tool_calls && !pending_tool_calls.is_empty() {
                for (call_id, _) in pending_tool_calls.drain() {
                    new_messages.push(Message::tool_result(
                        call_id,
                        "[Tool result synthesized during integrity repair]",
                    ));
                    dangling_calls_fixed += 1;
                }
            }
        }

        if msg.role == Role::Tool {
            if let Some(call_id) = &msg.tool_call_id {
                if pending_tool_calls.remove(call_id).is_some() {
                    new_messages.push(msg);
                } else if options.fix_orphan_tool_results {
                    orphan_tools_fixed += 1;
                } else {
                    new_messages.push(msg);
                }
            } else if options.fix_orphan_tool_results {
                orphan_tools_fixed += 1;
            } else {
                new_messages.push(msg);
            }
            continue;
        }

        if msg.role == Role::Assistant {
            if let Some(tool_calls) = &msg.tool_calls {
                for call in tool_calls {
                    pending_tool_calls.insert(call.id.clone(), call.clone());
                }
            }
        }

        if options.merge_consecutive_same_roles {
            if let Some(last_msg) = new_messages.last_mut() {
                if last_msg.role == msg.role
                    && last_msg.tool_calls.is_none()
                    && msg.tool_calls.is_none()
                    && (msg.role == Role::User || msg.role == Role::Assistant)
                {
                    last_msg.content.push_str("\n\n");
                    last_msg.content.push_str(&msg.content);
                    consecutive_merged += 1;
                    continue;
                }
            }
        }

        new_messages.push(msg);
    }

    if options.fix_dangling_tool_calls && !pending_tool_calls.is_empty() {
        for (call_id, _) in pending_tool_calls.drain() {
            new_messages.push(Message::tool_result(
                call_id,
                "[Tool result synthesized during integrity repair]",
            ));
            dangling_calls_fixed += 1;
        }
    }

    if !stray_system_texts.is_empty() {
        let combined_stray = stray_system_texts.join("\n\n");
        if let Some(existing) = &mut session.system_prompt {
            existing.push_str("\n\n");
            existing.push_str(&combined_stray);
        } else {
            session.system_prompt = Some(combined_stray);
        }
    }

    session.messages = new_messages;
    session.touch();

    let token_report = if options.recompute_tokens {
        Some(recompute_session_token_stats(session))
    } else {
        None
    };

    let total_repairs = orphan_tools_fixed
        + dangling_calls_fixed
        + stray_systems_fixed
        + empty_messages_removed
        + consecutive_merged;

    IntegrityRepairReport {
        orphan_tools_fixed,
        dangling_calls_fixed,
        stray_systems_fixed,
        empty_messages_removed,
        consecutive_merged,
        total_repairs,
        token_report,
    }
}

/// Configuration options for creating a session fork from a specific turn.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ForkPatchOptions {
    /// Optional custom title for the new forked session.
    pub custom_title: Option<String>,
    /// Optional active model override for the forked session.
    pub custom_model: Option<String>,
    /// Additional metadata key-value pairs to attach to the forked session.
    pub additional_metadata: HashMap<String, String>,
    /// Optional system prompt override.
    pub custom_system_prompt: Option<String>,
}

impl Default for ForkPatchOptions {
    fn default() -> Self {
        Self {
            custom_title: None,
            custom_model: None,
            additional_metadata: HashMap::new(),
            custom_system_prompt: None,
        }
    }
}

/// Creates an independent branch/fork of a session containing only history up to `turn_index` (1-based).
pub fn fork_session_at_turn(
    session: &Session,
    turn_index: usize,
    options: Option<&ForkPatchOptions>,
) -> Result<Session, SessionPatchError> {
    let turns = extract_patchable_turns(session);
    if turn_index == 0 || turn_index > turns.len() {
        return Err(SessionPatchError::TurnNotFound {
            turn_index,
            total_turns: turns.len(),
        });
    }

    let turn = &turns[turn_index - 1];
    let sliced_messages = session.messages[..turn.end_message_index].to_vec();

    let default_opts = ForkPatchOptions::default();
    let opts = options.unwrap_or(&default_opts);

    let model = opts
        .custom_model
        .clone()
        .unwrap_or_else(|| session.active_model.clone());

    let forked_id = Uuid::new_v4();
    let now = Utc::now().to_rfc3339();

    let mut forked = Session::with_id(forked_id, model);
    forked.created_at = now.clone();
    forked.updated_at = now.clone();
    forked.system_prompt = opts
        .custom_system_prompt
        .clone()
        .or_else(|| session.system_prompt.clone());
    forked.working_dir = session.working_dir.clone();
    forked.messages = sliced_messages;

    forked.metadata = session.metadata.clone();
    for (k, v) in &opts.additional_metadata {
        forked.metadata.insert(k.clone(), v.clone());
    }
    forked.metadata.insert(
        "forked_from_session_id".to_string(),
        session.id().to_string(),
    );
    forked
        .metadata
        .insert("forked_at_turn".to_string(), turn_index.to_string());
    forked
        .metadata
        .insert("forked_at_timestamp".to_string(), now);

    let title = if let Some(custom) = &opts.custom_title {
        custom.clone()
    } else if let Some(orig) = &session.title {
        format!("{} (Fork @ Turn {})", orig, turn_index)
    } else {
        format!("Session Fork @ Turn {}", turn_index)
    };
    forked.title = Some(title);

    recompute_session_token_stats(&mut forked);

    Ok(forked)
}

/// Convenience helper to create a session fork with an optional custom title.
pub fn create_session_fork(
    session: &Session,
    turn_index: usize,
    new_title: Option<String>,
) -> Result<Session, SessionPatchError> {
    let opts = ForkPatchOptions {
        custom_title: new_title,
        ..Default::default()
    };
    fork_session_at_turn(session, turn_index, Some(&opts))
}

/// Summary report of a completed surgical session patch operation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PatchReport {
    /// Number of turns before patching.
    pub turns_before: usize,
    /// Number of turns after patching.
    pub turns_after: usize,
    /// Number of messages before patching.
    pub messages_before: usize,
    /// Number of messages after patching.
    pub messages_after: usize,
    /// Number of patch operations successfully executed.
    pub operations_applied: usize,
    /// Token recomputation details.
    pub token_report: TokenRecomputeReport,
    /// Post-patch integrity validation status.
    pub integrity: SessionIntegrityReport,
}

impl PatchReport {
    /// Formats a concise human-readable summary of the patch report.
    pub fn format_summary(&self) -> String {
        format!(
            "Session Patch Applied: {} op(s) | Turns: {} -> {} | Messages: {} -> {} | {}",
            self.operations_applied,
            self.turns_before,
            self.turns_after,
            self.messages_before,
            self.messages_after,
            self.token_report.format_summary()
        )
    }
}

/// Dry-run simulation report showing planned changes before applying a patch.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PatchDryRunReport {
    /// Total operations in the plan.
    pub total_operations: usize,
    /// Estimated turns before and after.
    pub turns_before: usize,
    pub turns_after: usize,
    /// Estimated messages before and after.
    pub messages_before: usize,
    pub messages_after: usize,
    /// Projected token count change.
    pub projected_token_report: TokenRecomputeReport,
    /// Projected integrity issues on the resulting session.
    pub projected_integrity: SessionIntegrityReport,
}

fn apply_single_op(session: &mut Session, op: &SessionPatchOp) -> Result<(), SessionPatchError> {
    match op {
        SessionPatchOp::ModifyUser {
            turn_index,
            new_content,
        } => {
            let turns = extract_patchable_turns(session);
            if *turn_index == 0 || *turn_index > turns.len() {
                return Err(SessionPatchError::TurnNotFound {
                    turn_index: *turn_index,
                    total_turns: turns.len(),
                });
            }
            let turn = &turns[*turn_index - 1];
            let user_msg_idx = session.messages[turn.start_message_index..turn.end_message_index]
                .iter()
                .position(|m| m.role == Role::User)
                .map(|p| turn.start_message_index + p);

            if let Some(idx) = user_msg_idx {
                session.messages[idx].content = new_content.clone();
            } else {
                session.messages.insert(turn.start_message_index, Message::user(new_content.clone()));
            }
            session.touch();
            Ok(())
        }
        SessionPatchOp::ModifyAssistant {
            turn_index,
            new_content,
        } => {
            let turns = extract_patchable_turns(session);
            if *turn_index == 0 || *turn_index > turns.len() {
                return Err(SessionPatchError::TurnNotFound {
                    turn_index: *turn_index,
                    total_turns: turns.len(),
                });
            }
            let turn = &turns[*turn_index - 1];
            let asst_msg_idx = session.messages[turn.start_message_index..turn.end_message_index]
                .iter()
                .rposition(|m| m.role == Role::Assistant)
                .map(|p| turn.start_message_index + p);

            if let Some(idx) = asst_msg_idx {
                session.messages[idx].content = new_content.clone();
            } else {
                session.messages.insert(turn.end_message_index, Message::assistant(new_content.clone()));
            }
            session.touch();
            Ok(())
        }
        SessionPatchOp::ModifyTurn { turn_index, turn } => {
            if turn.is_empty() {
                return Err(SessionPatchError::EmptyTurn);
            }
            let turns = extract_patchable_turns(session);
            if *turn_index == 0 || *turn_index > turns.len() {
                return Err(SessionPatchError::TurnNotFound {
                    turn_index: *turn_index,
                    total_turns: turns.len(),
                });
            }
            let t = &turns[*turn_index - 1];
            session.messages.splice(t.start_message_index..t.end_message_index, turn.messages.clone());
            session.touch();
            Ok(())
        }
        SessionPatchOp::InsertTurn {
            turn_index,
            turn,
            position,
        } => {
            if turn.is_empty() {
                return Err(SessionPatchError::EmptyTurn);
            }
            let turns = extract_patchable_turns(session);
            if turns.is_empty() {
                let insert_pos = session
                    .messages
                    .iter()
                    .position(|m| m.role != Role::System)
                    .unwrap_or(session.messages.len());
                session.messages.splice(insert_pos..insert_pos, turn.messages.clone());
            } else {
                if *turn_index == 0 || *turn_index > turns.len() + 1 {
                    return Err(SessionPatchError::TurnNotFound {
                        turn_index: *turn_index,
                        total_turns: turns.len(),
                    });
                }
                let insert_idx = match position {
                    InsertPosition::Before => {
                        if *turn_index <= turns.len() {
                            turns[*turn_index - 1].start_message_index
                        } else {
                            session.messages.len()
                        }
                    }
                    InsertPosition::After => {
                        if *turn_index <= turns.len() {
                            turns[*turn_index - 1].end_message_index
                        } else {
                            session.messages.len()
                        }
                    }
                };
                session.messages.splice(insert_idx..insert_idx, turn.messages.clone());
            }
            session.touch();
            Ok(())
        }
        SessionPatchOp::AppendTurn { turn } => {
            if turn.is_empty() {
                return Err(SessionPatchError::EmptyTurn);
            }
            session.messages.extend(turn.messages.clone());
            session.touch();
            Ok(())
        }
        SessionPatchOp::PrependTurn { turn } => {
            if turn.is_empty() {
                return Err(SessionPatchError::EmptyTurn);
            }
            let insert_pos = session
                .messages
                .iter()
                .position(|m| m.role != Role::System)
                .unwrap_or(0);
            session.messages.splice(insert_pos..insert_pos, turn.messages.clone());
            session.touch();
            Ok(())
        }
        SessionPatchOp::DeleteTurn { turn_index } => {
            let turns = extract_patchable_turns(session);
            if *turn_index == 0 || *turn_index > turns.len() {
                return Err(SessionPatchError::TurnNotFound {
                    turn_index: *turn_index,
                    total_turns: turns.len(),
                });
            }
            let turn = &turns[*turn_index - 1];
            session.messages.drain(turn.start_message_index..turn.end_message_index);
            session.touch();
            Ok(())
        }
        SessionPatchOp::DeleteTurnRange {
            start_turn,
            end_turn,
        } => {
            if *start_turn == 0 || *start_turn > *end_turn {
                return Err(SessionPatchError::TurnRangeInvalid {
                    start: *start_turn,
                    end: *end_turn,
                    total_turns: extract_patchable_turns(session).len(),
                });
            }
            let turns = extract_patchable_turns(session);
            if *end_turn > turns.len() {
                return Err(SessionPatchError::TurnRangeInvalid {
                    start: *start_turn,
                    end: *end_turn,
                    total_turns: turns.len(),
                });
            }
            let start_idx = turns[*start_turn - 1].start_message_index;
            let end_idx = turns[*end_turn - 1].end_message_index;
            session.messages.drain(start_idx..end_idx);
            session.touch();
            Ok(())
        }
        SessionPatchOp::TruncateAfterTurn { turn_index } => {
            let turns = extract_patchable_turns(session);
            if *turn_index == 0 || *turn_index > turns.len() {
                return Err(SessionPatchError::TurnNotFound {
                    turn_index: *turn_index,
                    total_turns: turns.len(),
                });
            }
            let end_idx = turns[*turn_index - 1].end_message_index;
            session.messages.truncate(end_idx);
            session.touch();
            Ok(())
        }
        SessionPatchOp::TruncateBeforeTurn { turn_index } => {
            let turns = extract_patchable_turns(session);
            if *turn_index == 0 || *turn_index > turns.len() {
                return Err(SessionPatchError::TurnNotFound {
                    turn_index: *turn_index,
                    total_turns: turns.len(),
                });
            }
            let first_non_sys = session
                .messages
                .iter()
                .position(|m| m.role != Role::System)
                .unwrap_or(0);
            let start_cut = first_non_sys;
            let end_cut = turns[*turn_index - 1].start_message_index;
            if end_cut > start_cut {
                session.messages.drain(start_cut..end_cut);
            }
            session.touch();
            Ok(())
        }
        SessionPatchOp::SwapTurns { turn_a, turn_b } => {
            if turn_a == turn_b {
                return Ok(());
            }
            let turns = extract_patchable_turns(session);
            if *turn_a == 0 || *turn_a > turns.len() {
                return Err(SessionPatchError::TurnNotFound {
                    turn_index: *turn_a,
                    total_turns: turns.len(),
                });
            }
            if *turn_b == 0 || *turn_b > turns.len() {
                return Err(SessionPatchError::TurnNotFound {
                    turn_index: *turn_b,
                    total_turns: turns.len(),
                });
            }
            let (idx_first, idx_second) = if turn_a < turn_b {
                (*turn_a, *turn_b)
            } else {
                (*turn_b, *turn_a)
            };

            let msgs_first = turns[idx_first - 1].messages.clone();
            let msgs_second = turns[idx_second - 1].messages.clone();

            let range_second = turns[idx_second - 1].start_message_index..turns[idx_second - 1].end_message_index;
            let range_first = turns[idx_first - 1].start_message_index..turns[idx_first - 1].end_message_index;

            session.messages.splice(range_second, msgs_first);
            session.messages.splice(range_first, msgs_second);
            session.touch();
            Ok(())
        }
        SessionPatchOp::ModifyMessage {
            message_index,
            new_message,
        } => {
            if *message_index >= session.messages.len() {
                return Err(SessionPatchError::MessageNotFound {
                    message_index: *message_index,
                    total_messages: session.messages.len(),
                });
            }
            session.messages[*message_index] = new_message.clone();
            session.touch();
            Ok(())
        }
        SessionPatchOp::DeleteMessage { message_index } => {
            if *message_index >= session.messages.len() {
                return Err(SessionPatchError::MessageNotFound {
                    message_index: *message_index,
                    total_messages: session.messages.len(),
                });
            }
            session.messages.remove(*message_index);
            session.touch();
            Ok(())
        }
        SessionPatchOp::InsertMessage {
            message_index,
            message,
        } => {
            if *message_index > session.messages.len() {
                return Err(SessionPatchError::MessageNotFound {
                    message_index: *message_index,
                    total_messages: session.messages.len(),
                });
            }
            session.messages.insert(*message_index, message.clone());
            session.touch();
            Ok(())
        }
        SessionPatchOp::SetSystemPrompt { system_prompt } => {
            session.system_prompt = system_prompt.clone();
            session.touch();
            Ok(())
        }
        SessionPatchOp::SetActiveModel { model } => {
            session.active_model = model.clone();
            session.touch();
            Ok(())
        }
        SessionPatchOp::SetTitle { title } => {
            session.title = title.clone();
            session.touch();
            Ok(())
        }
        SessionPatchOp::SetMetadata { key, value } => {
            session.metadata.insert(key.clone(), value.clone());
            session.touch();
            Ok(())
        }
        SessionPatchOp::RemoveMetadata { key } => {
            session.metadata.remove(key);
            session.touch();
            Ok(())
        }
    }
}

/// Builder and execution engine for applying sequential surgical patches to a conversational session.
#[derive(Debug, Clone)]
pub struct SessionPatcher {
    session: Session,
    operations: Vec<SessionPatchOp>,
    auto_recompute_tokens: bool,
    strict_integrity: bool,
    auto_repair_integrity: bool,
    repair_options: IntegrityRepairOptions,
}

impl SessionPatcher {
    /// Creates a new `SessionPatcher` consuming the provided session.
    pub fn new(session: Session) -> Self {
        Self {
            session,
            operations: Vec::new(),
            auto_recompute_tokens: true,
            strict_integrity: false,
            auto_repair_integrity: false,
            repair_options: IntegrityRepairOptions::default(),
        }
    }

    /// Creates a new `SessionPatcher` by cloning the provided session reference.
    pub fn from_ref(session: &Session) -> Self {
        Self::new(session.clone())
    }

    /// Enables or disables automatic token count recomputation on patch completion (default: true).
    pub fn with_auto_recompute_tokens(mut self, enabled: bool) -> Self {
        self.auto_recompute_tokens = enabled;
        self
    }

    /// Enables or disables strict integrity validation (fails on critical errors, default: false).
    pub fn with_strict_integrity(mut self, strict: bool) -> Self {
        self.strict_integrity = strict;
        self
    }

    /// Enables automatic repair of integrity anomalies (default: false).
    pub fn with_auto_repair(mut self, auto_repair: bool) -> Self {
        self.auto_repair_integrity = auto_repair;
        self
    }

    /// Configures custom integrity repair options.
    pub fn with_repair_options(mut self, options: IntegrityRepairOptions) -> Self {
        self.repair_options = options;
        self.auto_repair_integrity = true;
        self
    }

    /// Modifies the user message content in turn `turn_index` (1-based).
    pub fn modify_user(&mut self, turn_index: usize, content: impl Into<String>) -> &mut Self {
        self.operations.push(SessionPatchOp::ModifyUser {
            turn_index,
            new_content: content.into(),
        });
        self
    }

    /// Modifies the assistant message content in turn `turn_index` (1-based).
    pub fn modify_assistant(&mut self, turn_index: usize, content: impl Into<String>) -> &mut Self {
        self.operations.push(SessionPatchOp::ModifyAssistant {
            turn_index,
            new_content: content.into(),
        });
        self
    }

    /// Replaces the entire turn `turn_index` (1-based) with new turn data.
    pub fn modify_turn(&mut self, turn_index: usize, turn: TurnData) -> &mut Self {
        self.operations.push(SessionPatchOp::ModifyTurn {
            turn_index,
            turn,
        });
        self
    }

    /// Inserts a new turn before turn `turn_index` (1-based).
    pub fn insert_turn_before(&mut self, turn_index: usize, turn: TurnData) -> &mut Self {
        self.operations.push(SessionPatchOp::InsertTurn {
            turn_index,
            turn,
            position: InsertPosition::Before,
        });
        self
    }

    /// Inserts a new turn after turn `turn_index` (1-based).
    pub fn insert_turn_after(&mut self, turn_index: usize, turn: TurnData) -> &mut Self {
        self.operations.push(SessionPatchOp::InsertTurn {
            turn_index,
            turn,
            position: InsertPosition::After,
        });
        self
    }

    /// Inserts a new turn at a specific position relative to `turn_index`.
    pub fn insert_turn(
        &mut self,
        turn_index: usize,
        turn: TurnData,
        position: InsertPosition,
    ) -> &mut Self {
        self.operations.push(SessionPatchOp::InsertTurn {
            turn_index,
            turn,
            position,
        });
        self
    }

    /// Appends a new turn to the end of the conversation.
    pub fn append_turn(&mut self, turn: TurnData) -> &mut Self {
        self.operations.push(SessionPatchOp::AppendTurn { turn });
        self
    }

    /// Prepends a new turn to the beginning of the conversation.
    pub fn prepend_turn(&mut self, turn: TurnData) -> &mut Self {
        self.operations.push(SessionPatchOp::PrependTurn { turn });
        self
    }

    /// Deletes turn `turn_index` (1-based).
    pub fn delete_turn(&mut self, turn_index: usize) -> &mut Self {
        self.operations.push(SessionPatchOp::DeleteTurn { turn_index });
        self
    }

    /// Deletes a range of turns [start_turn..=end_turn] (1-based, inclusive).
    pub fn delete_turns(&mut self, start_turn: usize, end_turn: usize) -> &mut Self {
        self.operations.push(SessionPatchOp::DeleteTurnRange {
            start_turn,
            end_turn,
        });
        self
    }

    /// Truncates the session after turn `turn_index` (1-based).
    pub fn truncate_after(&mut self, turn_index: usize) -> &mut Self {
        self.operations
            .push(SessionPatchOp::TruncateAfterTurn { turn_index });
        self
    }

    /// Truncates the session before turn `turn_index` (1-based).
    pub fn truncate_before(&mut self, turn_index: usize) -> &mut Self {
        self.operations
            .push(SessionPatchOp::TruncateBeforeTurn { turn_index });
        self
    }

    /// Swaps the positions of two turns.
    pub fn swap_turns(&mut self, turn_a: usize, turn_b: usize) -> &mut Self {
        self.operations
            .push(SessionPatchOp::SwapTurns { turn_a, turn_b });
        self
    }

    /// Modifies a specific message by 0-based message index.
    pub fn modify_message(&mut self, message_index: usize, new_message: Message) -> &mut Self {
        self.operations.push(SessionPatchOp::ModifyMessage {
            message_index,
            new_message,
        });
        self
    }

    /// Deletes a specific message by 0-based message index.
    pub fn delete_message(&mut self, message_index: usize) -> &mut Self {
        self.operations
            .push(SessionPatchOp::DeleteMessage { message_index });
        self
    }

    /// Inserts a message at 0-based message index.
    pub fn insert_message(&mut self, message_index: usize, message: Message) -> &mut Self {
        self.operations.push(SessionPatchOp::InsertMessage {
            message_index,
            message,
        });
        self
    }

    /// Updates the session-level system prompt.
    pub fn set_system_prompt(&mut self, prompt: Option<String>) -> &mut Self {
        self.operations
            .push(SessionPatchOp::SetSystemPrompt { system_prompt: prompt });
        self
    }

    /// Updates the active model.
    pub fn set_active_model(&mut self, model: impl Into<String>) -> &mut Self {
        self.operations.push(SessionPatchOp::SetActiveModel {
            model: model.into(),
        });
        self
    }

    /// Updates the session title.
    pub fn set_title(&mut self, title: Option<String>) -> &mut Self {
        self.operations.push(SessionPatchOp::SetTitle { title });
        self
    }

    /// Adds or updates a metadata key-value pair.
    pub fn set_metadata(&mut self, key: impl Into<String>, value: impl Into<String>) -> &mut Self {
        self.operations.push(SessionPatchOp::SetMetadata {
            key: key.into(),
            value: value.into(),
        });
        self
    }

    /// Removes a metadata key.
    pub fn remove_metadata(&mut self, key: impl Into<String>) -> &mut Self {
        self.operations.push(SessionPatchOp::RemoveMetadata {
            key: key.into(),
        });
        self
    }

    /// Simulates applying the patch operations and returns a projected dry-run report.
    pub fn dry_run(&self) -> Result<PatchDryRunReport, SessionPatchError> {
        let mut sim_session = self.session.clone();
        let turns_before = extract_patchable_turns(&sim_session).len();
        let messages_before = sim_session.messages.len();

        for op in &self.operations {
            apply_single_op(&mut sim_session, op)?;
        }

        if self.auto_repair_integrity {
            repair_session_integrity(&mut sim_session, &self.repair_options);
        }

        let token_report = recompute_session_token_stats(&mut sim_session);
        let integrity = validate_session_integrity(&sim_session);
        let turns_after = extract_patchable_turns(&sim_session).len();
        let messages_after = sim_session.messages.len();

        Ok(PatchDryRunReport {
            total_operations: self.operations.len(),
            turns_before,
            turns_after,
            messages_before,
            messages_after,
            projected_token_report: token_report,
            projected_integrity: integrity,
        })
    }

    /// Applies all planned patch operations and returns the modified `Session`.
    pub fn apply(self) -> Result<Session, SessionPatchError> {
        let (session, _) = self.apply_with_report()?;
        Ok(session)
    }

    /// Applies all planned patch operations, returning both the modified `Session` and a `PatchReport`.
    pub fn apply_with_report(mut self) -> Result<(Session, PatchReport), SessionPatchError> {
        let turns_before = extract_patchable_turns(&self.session).len();
        let messages_before = self.session.messages.len();
        let ops_count = self.operations.len();

        for op in &self.operations {
            apply_single_op(&mut self.session, op)?;
        }

        if self.auto_repair_integrity {
            repair_session_integrity(&mut self.session, &self.repair_options);
        }

        let token_report = if self.auto_recompute_tokens {
            recompute_session_token_stats(&mut self.session)
        } else {
            TokenRecomputeReport {
                prompt_tokens: self.session.token_stats.prompt_tokens,
                completion_tokens: self.session.token_stats.completion_tokens,
                total_tokens: self.session.token_stats.total_tokens,
                total_turns: self.session.token_stats.total_turns,
                previous_total_tokens: self.session.token_stats.total_tokens,
                token_delta: 0,
                message_count: self.session.messages.len(),
            }
        };

        let integrity = validate_session_integrity(&self.session);
        if self.strict_integrity && integrity.has_errors() {
            return Err(SessionPatchError::IntegrityError {
                details: integrity.format_report(),
            });
        }

        let turns_after = extract_patchable_turns(&self.session).len();
        let messages_after = self.session.messages.len();

        let report = PatchReport {
            turns_before,
            turns_after,
            messages_before,
            messages_after,
            operations_applied: ops_count,
            token_report,
            integrity,
        };

        Ok((self.session, report))
    }
}

/// Surgically modifies the user message in turn `turn_index` (1-based).
pub fn patch_modify_turn_user(
    session: &mut Session,
    turn_index: usize,
    content: impl Into<String>,
) -> Result<PatchReport, SessionPatchError> {
    let mut patcher = SessionPatcher::from_ref(session);
    patcher.modify_user(turn_index, content);
    let (patched, report) = patcher.apply_with_report()?;
    *session = patched;
    Ok(report)
}

/// Surgically modifies the assistant message in turn `turn_index` (1-based).
pub fn patch_modify_turn_assistant(
    session: &mut Session,
    turn_index: usize,
    content: impl Into<String>,
) -> Result<PatchReport, SessionPatchError> {
    let mut patcher = SessionPatcher::from_ref(session);
    patcher.modify_assistant(turn_index, content);
    let (patched, report) = patcher.apply_with_report()?;
    *session = patched;
    Ok(report)
}

/// Surgically replaces turn `turn_index` (1-based) with the provided `TurnData`.
pub fn patch_modify_turn(
    session: &mut Session,
    turn_index: usize,
    turn: TurnData,
) -> Result<PatchReport, SessionPatchError> {
    let mut patcher = SessionPatcher::from_ref(session);
    patcher.modify_turn(turn_index, turn);
    let (patched, report) = patcher.apply_with_report()?;
    *session = patched;
    Ok(report)
}

/// Inserts a new turn before or after turn `turn_index` (1-based).
pub fn patch_insert_turn(
    session: &mut Session,
    turn_index: usize,
    turn: TurnData,
    position: InsertPosition,
) -> Result<PatchReport, SessionPatchError> {
    let mut patcher = SessionPatcher::from_ref(session);
    patcher.insert_turn(turn_index, turn, position);
    let (patched, report) = patcher.apply_with_report()?;
    *session = patched;
    Ok(report)
}

/// Appends a new turn to the end of the conversation.
pub fn patch_append_turn(
    session: &mut Session,
    turn: TurnData,
) -> Result<PatchReport, SessionPatchError> {
    let mut patcher = SessionPatcher::from_ref(session);
    patcher.append_turn(turn);
    let (patched, report) = patcher.apply_with_report()?;
    *session = patched;
    Ok(report)
}

/// Prepends a new turn to the beginning of the conversation.
pub fn patch_prepend_turn(
    session: &mut Session,
    turn: TurnData,
) -> Result<PatchReport, SessionPatchError> {
    let mut patcher = SessionPatcher::from_ref(session);
    patcher.prepend_turn(turn);
    let (patched, report) = patcher.apply_with_report()?;
    *session = patched;
    Ok(report)
}

/// Deletes turn `turn_index` (1-based).
pub fn patch_delete_turn(
    session: &mut Session,
    turn_index: usize,
) -> Result<PatchReport, SessionPatchError> {
    let mut patcher = SessionPatcher::from_ref(session);
    patcher.delete_turn(turn_index);
    let (patched, report) = patcher.apply_with_report()?;
    *session = patched;
    Ok(report)
}

/// Deletes a range of turns [start_turn..=end_turn] (1-based, inclusive).
pub fn patch_delete_turns(
    session: &mut Session,
    start_turn: usize,
    end_turn: usize,
) -> Result<PatchReport, SessionPatchError> {
    let mut patcher = SessionPatcher::from_ref(session);
    patcher.delete_turns(start_turn, end_turn);
    let (patched, report) = patcher.apply_with_report()?;
    *session = patched;
    Ok(report)
}

/// Truncates the session after turn `turn_index` (1-based).
pub fn patch_truncate_after(
    session: &mut Session,
    turn_index: usize,
) -> Result<PatchReport, SessionPatchError> {
    let mut patcher = SessionPatcher::from_ref(session);
    patcher.truncate_after(turn_index);
    let (patched, report) = patcher.apply_with_report()?;
    *session = patched;
    Ok(report)
}

/// Truncates the session before turn `turn_index` (1-based).
pub fn patch_truncate_before(
    session: &mut Session,
    turn_index: usize,
) -> Result<PatchReport, SessionPatchError> {
    let mut patcher = SessionPatcher::from_ref(session);
    patcher.truncate_before(turn_index);
    let (patched, report) = patcher.apply_with_report()?;
    *session = patched;
    Ok(report)
}

/// Swaps two turns by their 1-based indices.
pub fn patch_swap_turns(
    session: &mut Session,
    turn_a: usize,
    turn_b: usize,
) -> Result<PatchReport, SessionPatchError> {
    let mut patcher = SessionPatcher::from_ref(session);
    patcher.swap_turns(turn_a, turn_b);
    let (patched, report) = patcher.apply_with_report()?;
    *session = patched;
    Ok(report)
}

/// Modifies a specific message by 0-based message index.
pub fn patch_modify_message(
    session: &mut Session,
    message_index: usize,
    new_message: Message,
) -> Result<PatchReport, SessionPatchError> {
    let mut patcher = SessionPatcher::from_ref(session);
    patcher.modify_message(message_index, new_message);
    let (patched, report) = patcher.apply_with_report()?;
    *session = patched;
    Ok(report)
}

/// Deletes a specific message by 0-based message index.
pub fn patch_delete_message(
    session: &mut Session,
    message_index: usize,
) -> Result<PatchReport, SessionPatchError> {
    let mut patcher = SessionPatcher::from_ref(session);
    patcher.delete_message(message_index);
    let (patched, report) = patcher.apply_with_report()?;
    *session = patched;
    Ok(report)
}

/// Inserts a message at 0-based message index.
pub fn patch_insert_message(
    session: &mut Session,
    message_index: usize,
    message: Message,
) -> Result<PatchReport, SessionPatchError> {
    let mut patcher = SessionPatcher::from_ref(session);
    patcher.insert_message(message_index, message);
    let (patched, report) = patcher.apply_with_report()?;
    *session = patched;
    Ok(report)
}

/// Loads a saved session from a JSON file, applies the given patch operations,
/// recomputes tokens, validates integrity, and writes it back to disk.
pub fn patch_saved_session_file(
    path: impl AsRef<Path>,
    operations: &[SessionPatchOp],
) -> Result<Session, SessionPatchError> {
    let p = path.as_ref();
    let content = fs::read_to_string(p)?;
    let mut session: Session = serde_json::from_str(&content)?;

    for op in operations {
        apply_single_op(&mut session, op)?;
    }

    recompute_session_token_stats(&mut session);

    let serialized = serde_json::to_string_pretty(&session)?;
    fs::write(p, serialized)?;

    Ok(session)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use tempfile::tempdir;

    fn make_test_session() -> Session {
        let mut session = Session::new("claude-3-5-sonnet");
        session.set_title("Test Patch Session");

        // Turn 1: Create a new file `src/lib.rs`
        session.add_user_message("Please create src/lib.rs with an add function.");
        session.add_assistant_with_tools(
            "Creating src/lib.rs",
            vec![ToolCall {
                id: "call_1".to_string(),
                name: "write".to_string(),
                arguments: json!({
                    "path": "src/lib.rs",
                    "content": "pub fn add(a: i32, b: i32) -> i32 {\n    a + b\n}\n"
                })
                .to_string(),
            }],
        );
        session.add_tool_result("call_1", "Successfully wrote 44 bytes to 'src/lib.rs'");

        // Turn 2: Edit `src/lib.rs` to add doc comments and sub function
        session.add_user_message("Add a subtract function to src/lib.rs");
        session.add_assistant_with_tools(
            "Adding sub function",
            vec![ToolCall {
                id: "call_2".to_string(),
                name: "edit".to_string(),
                arguments: json!({
                    "path": "src/lib.rs",
                    "old_text": "pub fn add(a: i32, b: i32) -> i32 {\n    a + b\n}\n",
                    "new_text": "/// Adds two integers.\npub fn add(a: i32, b: i32) -> i32 {\n    a + b\n}\n\n/// Subtracts two integers.\npub fn sub(a: i32, b: i32) -> i32 {\n    a - b\n}\n"
                })
                .to_string(),
            }],
        );
        session.add_tool_result("call_2", "Successfully edited 'src/lib.rs'");

        // Turn 3: Create `src/main.rs`
        session.add_user_message("Now create src/main.rs");
        session.add_assistant_with_tools(
            "Creating main.rs",
            vec![ToolCall {
                id: "call_3".to_string(),
                name: "write".to_string(),
                arguments: json!({
                    "path": "src/main.rs",
                    "content": "fn main() {\n    println!(\"Hello Fusion!\");\n}\n"
                })
                .to_string(),
            }],
        );
        session.add_tool_result("call_3", "Successfully wrote 42 bytes to 'src/main.rs'");

        session
    }

    #[test]
    fn test_patch_file_stats_calculation() {
        let stats = PatchFileStats::new(10, 3);
        assert_eq!(stats.total_changes(), 13);
        assert_eq!(stats.format_stat(), "+10, -3");

        let graph = stats.format_graph(20);
        assert!(graph.contains('+'));
        assert!(graph.contains('-'));
    }

    #[test]
    fn test_session_patch_builder() {
        let patch = SessionPatch::builder()
            .session_id("test-session-123")
            .session_title("Refactor Database")
            .model("gpt-4o")
            .add_created("src/db.rs", "pub struct Database;\n")
            .add_modified("src/main.rs", "fn old() {}\n", "fn new() {}\n")
            .add_deleted("src/legacy.rs", "legacy code\n")
            .build();

        assert_eq!(patch.file_count(), 3);
        assert_eq!(patch.summary.files_created, 1);
        assert_eq!(patch.summary.files_modified, 1);
        assert_eq!(patch.summary.files_deleted, 1);

        let diff_str = patch.to_unified_diff(&SessionPatchOptions::default());
        assert!(diff_str.contains("diff --git a/src/db.rs b/src/db.rs"));
        assert!(diff_str.contains("new file mode 100644"));
        assert!(diff_str.contains("diff --git a/src/main.rs b/src/main.rs"));
        assert!(diff_str.contains("diff --git a/src/legacy.rs b/src/legacy.rs"));
        assert!(diff_str.contains("deleted file mode 100644"));
    }

    #[test]
    fn test_aggregation_from_session_messages() {
        let session = make_test_session();
        let options = SessionPatchOptions::default();
        let patch = export_session_patch(&session, &options);

        // Expect 2 files: src/lib.rs (created + edited = created with final state) and src/main.rs
        assert_eq!(patch.file_count(), 2);

        let lib_patch = patch.get_file("src/lib.rs").expect("src/lib.rs must exist");
        assert_eq!(lib_patch.kind, FileEditKind::Created);
        assert!(lib_patch
            .new_content
            .as_ref()
            .unwrap()
            .contains("pub fn sub"));

        let main_patch = patch.get_file("src/main.rs").expect("src/main.rs must exist");
        assert_eq!(main_patch.kind, FileEditKind::Created);

        let patch_text = patch.to_unified_diff(&options);
        assert!(patch_text.contains("Fusion Session Unified Patch"));
        assert!(patch_text.contains("diff --git a/src/lib.rs b/src/lib.rs"));
        assert!(patch_text.contains("diff --git a/src/main.rs b/src/main.rs"));
    }

    #[test]
    fn test_stat_table_formatting() {
        let patch = SessionPatch::builder()
            .add_created("src/lib.rs", "fn hello() {}\n")
            .add_modified("Cargo.toml", "[package]\n", "[package]\nversion = \"0.2.0\"\n")
            .build();

        let stat_table = patch.format_stat();
        assert!(stat_table.contains("src/lib.rs (new)"));
        assert!(stat_table.contains("Cargo.toml"));
        assert!(stat_table.contains("2 files changed"));
    }

    #[test]
    fn test_patch_file_saving() {
        let temp = tempdir().unwrap();
        let session = make_test_session();
        let patch_path = temp.path().join("session.patch");

        let options = SessionPatchOptions::default();
        let saved_path = export_session_patch_file(&session, &patch_path, &options).unwrap();

        assert!(saved_path.exists());
        let content = fs::read_to_string(&saved_path).unwrap();
        assert!(content.contains("diff --git a/src/lib.rs b/src/lib.rs"));
        assert!(content.contains("diff --git a/src/main.rs b/src/main.rs"));
    }

    #[test]
    fn test_patch_reverse_rollback() {
        let patch = SessionPatch::builder()
            .session_title("Original Change")
            .add_created("src/new.rs", "new content\n")
            .add_modified("src/mod.rs", "old text\n", "new text\n")
            .add_deleted("src/old.rs", "deleted text\n")
            .build();

        let rev = patch.reverse();
        assert_eq!(rev.file_count(), 3);

        let rev_new = rev.get_file("src/new.rs").unwrap();
        assert_eq!(rev_new.kind, FileEditKind::Deleted);

        let rev_mod = rev.get_file("src/mod.rs").unwrap();
        assert_eq!(rev_mod.kind, FileEditKind::Modified);
        assert_eq!(rev_mod.old_content.as_deref(), Some("new text\n"));
        assert_eq!(rev_mod.new_content.as_deref(), Some("old text\n"));

        let rev_old = rev.get_file("src/old.rs").unwrap();
        assert_eq!(rev_old.kind, FileEditKind::Created);
    }

    #[test]
    fn test_patch_application_to_directory() {
        let temp = tempdir().unwrap();
        let target_dir = temp.path();

        let patch = SessionPatch::builder()
            .add_created("src/hello.txt", "Hello World\n")
            .add_created("docs/README.md", "# Documentation\n")
            .build();

        assert!(patch.can_apply_cleanly(target_dir));
        let applied = patch.apply_to_dir(target_dir).unwrap();
        assert_eq!(applied.len(), 2);

        assert_eq!(
            fs::read_to_string(target_dir.join("src/hello.txt")).unwrap(),
            "Hello World\n"
        );
        assert_eq!(
            fs::read_to_string(target_dir.join("docs/README.md")).unwrap(),
            "# Documentation\n"
        );
    }

    #[test]
    fn test_net_zero_reverted_edits_omitted() {
        let mut session = Session::new("gpt-4o");

        // Create file
        session.add_assistant_with_tools(
            "Write file",
            vec![ToolCall {
                id: "c1".to_string(),
                name: "write".to_string(),
                arguments: json!({
                    "path": "temp.txt",
                    "content": "temporary content\n"
                })
                .to_string(),
            }],
        );
        session.add_tool_result("c1", "ok");

        // Delete file in same session
        session.add_assistant_with_tools(
            "Delete file",
            vec![ToolCall {
                id: "c2".to_string(),
                name: "delete_file".to_string(),
                arguments: json!({
                    "path": "temp.txt"
                })
                .to_string(),
            }],
        );
        session.add_tool_result("c2", "ok");

        let patch = export_session_patch(&session, &SessionPatchOptions::default());
        // Net zero change -> 0 files in patch
        assert_eq!(patch.file_count(), 0);
    }

    // ===================================================================
    // Surgical Session Patching Tests
    // ===================================================================

    /// Build a simple 3-turn session for surgical patching tests.
    fn make_surgical_session() -> Session {
        let mut s = Session::new("test-model");
        s.set_title("Surgical Test");
        // Turn 1
        s.add_user_message("Hello");
        s.add_assistant_message("Hi there!");
        // Turn 2
        s.add_user_message("How are you?");
        s.add_assistant_message("I am fine.");
        // Turn 3
        s.add_user_message("Goodbye");
        s.add_assistant_message("Bye!");
        s
    }

    #[test]
    fn test_extract_patchable_turns_basic() {
        let s = make_surgical_session();
        let turns = extract_patchable_turns(&s);
        assert_eq!(turns.len(), 3);

        assert_eq!(turns[0].turn_index, 1);
        assert_eq!(turns[0].user_message.as_deref(), Some("Hello"));
        assert_eq!(turns[0].assistant_message.as_deref(), Some("Hi there!"));
        assert_eq!(turns[0].message_count, 2);

        assert_eq!(turns[1].turn_index, 2);
        assert_eq!(turns[1].user_message.as_deref(), Some("How are you?"));

        assert_eq!(turns[2].turn_index, 3);
        assert_eq!(turns[2].user_message.as_deref(), Some("Goodbye"));
    }

    #[test]
    fn test_extract_patchable_turns_empty() {
        let s = Session::new("m");
        let turns = extract_patchable_turns(&s);
        assert!(turns.is_empty());
    }

    #[test]
    fn test_extract_patchable_turns_assistant_only() {
        let mut s = Session::new("m");
        s.add_assistant_message("unsolicited greeting");
        let turns = extract_patchable_turns(&s);
        assert_eq!(turns.len(), 1);
        assert!(turns[0].user_message.is_none());
        assert_eq!(turns[0].assistant_message.as_deref(), Some("unsolicited greeting"));
    }

    #[test]
    fn test_turn_data_constructors() {
        let td = TurnData::new("q", "a");
        assert_eq!(td.user_content(), Some("q"));
        assert_eq!(td.assistant_content(), Some("a"));
        assert_eq!(td.len(), 2);
        assert!(!td.is_empty());
        assert_eq!(td.tool_calls_count(), 0);

        let uo = TurnData::user_only("q");
        assert_eq!(uo.user_content(), Some("q"));
        assert!(uo.assistant_content().is_none());

        let ao = TurnData::assistant_only("a");
        assert!(ao.user_content().is_none());
        assert_eq!(ao.assistant_content(), Some("a"));
    }

    #[test]
    fn test_turn_data_estimate_tokens() {
        let td = TurnData::new("hello world", "goodbye");
        let tokens = td.estimate_tokens();
        assert!(tokens > 0);
    }

    #[test]
    fn test_patchable_turn_preview() {
        let s = make_surgical_session();
        let turns = extract_patchable_turns(&s);
        let preview = turns[0].preview();
        assert!(preview.contains("Turn 1"));
        assert!(preview.contains("Hello"));
        assert!(preview.contains("Hi there!"));
    }

    // --- Modify operations ---

    #[test]
    fn test_modify_user_message() {
        let mut s = make_surgical_session();
        let report = patch_modify_turn_user(&mut s, 1, "Updated question").unwrap();
        assert_eq!(report.operations_applied, 1);

        let turns = extract_patchable_turns(&s);
        assert_eq!(turns[0].user_message.as_deref(), Some("Updated question"));
        // Other turns untouched
        assert_eq!(turns[1].user_message.as_deref(), Some("How are you?"));
    }

    #[test]
    fn test_modify_assistant_message() {
        let mut s = make_surgical_session();
        patch_modify_turn_assistant(&mut s, 2, "Actually great!").unwrap();

        let turns = extract_patchable_turns(&s);
        assert_eq!(turns[1].assistant_message.as_deref(), Some("Actually great!"));
    }

    #[test]
    fn test_modify_turn_replaces_entirely() {
        let mut s = make_surgical_session();
        let new_turn = TurnData::new("Replaced user", "Replaced assistant");
        patch_modify_turn(&mut s, 2, new_turn).unwrap();

        let turns = extract_patchable_turns(&s);
        assert_eq!(turns.len(), 3);
        assert_eq!(turns[1].user_message.as_deref(), Some("Replaced user"));
        assert_eq!(turns[1].assistant_message.as_deref(), Some("Replaced assistant"));
    }

    #[test]
    fn test_modify_turn_out_of_bounds() {
        let mut s = make_surgical_session();
        let result = patch_modify_turn_user(&mut s, 99, "nope");
        assert!(result.is_err());
        match result.unwrap_err() {
            SessionPatchError::TurnNotFound { turn_index, .. } => assert_eq!(turn_index, 99),
            other => panic!("Expected TurnNotFound, got: {:?}", other),
        }
    }

    #[test]
    fn test_modify_turn_zero_index_error() {
        let mut s = make_surgical_session();
        let result = patch_modify_turn_user(&mut s, 0, "nope");
        assert!(result.is_err());
    }

    // --- Insert operations ---

    #[test]
    fn test_insert_turn_before() {
        let mut s = make_surgical_session();
        let new_turn = TurnData::new("Inserted Q", "Inserted A");
        let report = patch_insert_turn(&mut s, 2, new_turn, InsertPosition::Before).unwrap();

        assert_eq!(report.turns_before, 3);
        assert_eq!(report.turns_after, 4);

        let turns = extract_patchable_turns(&s);
        assert_eq!(turns.len(), 4);
        assert_eq!(turns[0].user_message.as_deref(), Some("Hello"));
        assert_eq!(turns[1].user_message.as_deref(), Some("Inserted Q"));
        assert_eq!(turns[2].user_message.as_deref(), Some("How are you?"));
        assert_eq!(turns[3].user_message.as_deref(), Some("Goodbye"));
    }

    #[test]
    fn test_insert_turn_after() {
        let mut s = make_surgical_session();
        let new_turn = TurnData::new("After Q", "After A");
        patch_insert_turn(&mut s, 1, new_turn, InsertPosition::After).unwrap();

        let turns = extract_patchable_turns(&s);
        assert_eq!(turns.len(), 4);
        assert_eq!(turns[1].user_message.as_deref(), Some("After Q"));
    }

    #[test]
    fn test_append_turn() {
        let mut s = make_surgical_session();
        let new_turn = TurnData::new("Last Q", "Last A");
        let report = patch_append_turn(&mut s, new_turn).unwrap();

        assert_eq!(report.turns_after, 4);

        let turns = extract_patchable_turns(&s);
        assert_eq!(turns.last().unwrap().user_message.as_deref(), Some("Last Q"));
    }

    #[test]
    fn test_prepend_turn() {
        let mut s = make_surgical_session();
        let new_turn = TurnData::new("First Q", "First A");
        patch_prepend_turn(&mut s, new_turn).unwrap();

        let turns = extract_patchable_turns(&s);
        assert_eq!(turns.len(), 4);
        assert_eq!(turns[0].user_message.as_deref(), Some("First Q"));
        assert_eq!(turns[1].user_message.as_deref(), Some("Hello"));
    }

    #[test]
    fn test_insert_empty_turn_error() {
        let mut s = make_surgical_session();
        let empty = TurnData::from_messages(vec![]);
        let result = patch_append_turn(&mut s, empty);
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            SessionPatchError::EmptyTurn
        ));
    }

    // --- Delete operations ---

    #[test]
    fn test_delete_turn() {
        let mut s = make_surgical_session();
        let report = patch_delete_turn(&mut s, 2).unwrap();

        assert_eq!(report.turns_before, 3);
        assert_eq!(report.turns_after, 2);

        let turns = extract_patchable_turns(&s);
        assert_eq!(turns.len(), 2);
        assert_eq!(turns[0].user_message.as_deref(), Some("Hello"));
        assert_eq!(turns[1].user_message.as_deref(), Some("Goodbye"));
    }

    #[test]
    fn test_delete_turn_range() {
        let mut s = make_surgical_session();
        let report = patch_delete_turns(&mut s, 1, 2).unwrap();

        assert_eq!(report.turns_before, 3);
        assert_eq!(report.turns_after, 1);

        let turns = extract_patchable_turns(&s);
        assert_eq!(turns[0].user_message.as_deref(), Some("Goodbye"));
    }

    #[test]
    fn test_delete_turn_range_invalid() {
        let mut s = make_surgical_session();
        // start > end
        let result = patch_delete_turns(&mut s, 3, 1);
        assert!(result.is_err());
    }

    #[test]
    fn test_delete_turn_range_exceeds_bounds() {
        let mut s = make_surgical_session();
        let result = patch_delete_turns(&mut s, 1, 10);
        assert!(result.is_err());
    }

    // --- Truncate operations ---

    #[test]
    fn test_truncate_after() {
        let mut s = make_surgical_session();
        let report = patch_truncate_after(&mut s, 2).unwrap();

        assert_eq!(report.turns_after, 2);

        let turns = extract_patchable_turns(&s);
        assert_eq!(turns.len(), 2);
        assert_eq!(turns[0].user_message.as_deref(), Some("Hello"));
        assert_eq!(turns[1].user_message.as_deref(), Some("How are you?"));
    }

    #[test]
    fn test_truncate_before() {
        let mut s = make_surgical_session();
        patch_truncate_before(&mut s, 3).unwrap();

        let turns = extract_patchable_turns(&s);
        assert_eq!(turns.len(), 1);
        assert_eq!(turns[0].user_message.as_deref(), Some("Goodbye"));
    }

    #[test]
    fn test_truncate_out_of_bounds() {
        let mut s = make_surgical_session();
        assert!(patch_truncate_after(&mut s, 0).is_err());
        assert!(patch_truncate_after(&mut s, 99).is_err());
    }

    // --- Swap operations ---

    #[test]
    fn test_swap_turns() {
        let mut s = make_surgical_session();
        patch_swap_turns(&mut s, 1, 3).unwrap();

        let turns = extract_patchable_turns(&s);
        assert_eq!(turns.len(), 3);
        assert_eq!(turns[0].user_message.as_deref(), Some("Goodbye"));
        assert_eq!(turns[1].user_message.as_deref(), Some("How are you?"));
        assert_eq!(turns[2].user_message.as_deref(), Some("Hello"));
    }

    #[test]
    fn test_swap_same_turn_noop() {
        let mut s = make_surgical_session();
        // Swapping a turn with itself should be a no-op
        let report = patch_swap_turns(&mut s, 2, 2).unwrap();
        assert_eq!(report.turns_after, 3);
        let turns = extract_patchable_turns(&s);
        assert_eq!(turns[1].user_message.as_deref(), Some("How are you?"));
    }

    // --- Message-level operations ---

    #[test]
    fn test_modify_message_by_index() {
        let mut s = make_surgical_session();
        let new_msg = Message::user("Replaced at index 0");
        patch_modify_message(&mut s, 0, new_msg).unwrap();

        assert_eq!(s.messages[0].content, "Replaced at index 0");
        assert_eq!(s.messages[0].role, Role::User);
    }

    #[test]
    fn test_delete_message_by_index() {
        let mut s = make_surgical_session();
        let original_len = s.messages.len();
        patch_delete_message(&mut s, 0).unwrap();

        assert_eq!(s.messages.len(), original_len - 1);
    }

    #[test]
    fn test_insert_message_by_index() {
        let mut s = make_surgical_session();
        let original_len = s.messages.len();
        let new_msg = Message::system("injected system");
        patch_insert_message(&mut s, 0, new_msg).unwrap();

        assert_eq!(s.messages.len(), original_len + 1);
        assert_eq!(s.messages[0].role, Role::System);
        assert_eq!(s.messages[0].content, "injected system");
    }

    #[test]
    fn test_message_index_out_of_bounds() {
        let mut s = make_surgical_session();
        let result = patch_modify_message(&mut s, 999, Message::user("nope"));
        assert!(matches!(
            result.unwrap_err(),
            SessionPatchError::MessageNotFound { .. }
        ));

        let result = patch_delete_message(&mut s, 999);
        assert!(matches!(
            result.unwrap_err(),
            SessionPatchError::MessageNotFound { .. }
        ));
    }

    // --- Token recomputation ---

    #[test]
    fn test_recompute_session_token_stats() {
        let mut s = make_surgical_session();
        s.token_stats.prompt_tokens = 0;
        s.token_stats.completion_tokens = 0;
        s.token_stats.total_tokens = 0;
        s.token_stats.total_turns = 0;

        let report = recompute_session_token_stats(&mut s);
        // Should have recomputed non-zero tokens
        assert!(report.total_tokens > 0);
        assert!(report.prompt_tokens > 0);
        assert!(report.completion_tokens > 0);
        assert_eq!(report.total_turns, 3);
        assert_eq!(report.message_count, 6);
        assert_eq!(report.previous_total_tokens, 0);
        assert!(report.token_delta > 0);

        // Session stats should be updated
        assert_eq!(s.token_stats.total_tokens, report.total_tokens);
        assert_eq!(s.token_stats.prompt_tokens, report.prompt_tokens);
        assert_eq!(s.token_stats.completion_tokens, report.completion_tokens);
        assert_eq!(s.token_stats.total_turns, 3);
    }

    #[test]
    fn test_recompute_with_system_prompt() {
        let mut s = Session::with_system_prompt("m", "You are a helpful assistant.");
        s.add_user_message("Hi");
        s.add_assistant_message("Hello!");

        let report = recompute_session_token_stats(&mut s);
        // System prompt tokens should be included in prompt_tokens
        assert!(report.prompt_tokens > 0);
        assert!(report.total_tokens > 0);
    }

    #[test]
    fn test_token_recompute_report_format() {
        let report = TokenRecomputeReport {
            prompt_tokens: 100,
            completion_tokens: 50,
            total_tokens: 150,
            total_turns: 3,
            previous_total_tokens: 120,
            token_delta: 30,
            message_count: 6,
        };
        let summary = report.format_summary();
        assert!(summary.contains("150"));
        assert!(summary.contains("+30"));
        assert!(summary.contains("3 turns"));
    }

    // --- Integrity validation ---

    #[test]
    fn test_validate_clean_session() {
        let s = make_surgical_session();
        let report = validate_session_integrity(&s);
        assert!(report.is_valid);
        assert_eq!(report.error_count, 0);
    }

    #[test]
    fn test_validate_orphan_tool_result() {
        let mut s = Session::new("m");
        s.add_user_message("Hi");
        // Tool result with no prior tool call
        s.add_tool_result("nonexistent_call", "some result");

        let report = validate_session_integrity(&s);
        assert!(report.has_errors());
        let orphan = report.issues.iter().find(|i| i.code == "ORPHAN_TOOL_RESULT");
        assert!(orphan.is_some());
    }

    #[test]
    fn test_validate_dangling_tool_call() {
        let mut s = Session::new("m");
        s.add_user_message("Do something");
        s.add_assistant_with_tools(
            "Calling tool",
            vec![ToolCall {
                id: "tc_1".to_string(),
                name: "test_tool".to_string(),
                arguments: "{}".to_string(),
            }],
        );
        // No tool result, session ends with dangling call

        let report = validate_session_integrity(&s);
        let dangling = report.issues.iter().find(|i| i.code == "DANGLING_TOOL_CALL");
        assert!(dangling.is_some());
    }

    #[test]
    fn test_validate_consecutive_user_messages() {
        let mut s = Session::new("m");
        s.add_user_message("First user");
        s.add_user_message("Second user");
        s.add_assistant_message("Response");

        let report = validate_session_integrity(&s);
        let consecutive = report.issues.iter().find(|i| i.code == "CONSECUTIVE_USER_MESSAGES");
        assert!(consecutive.is_some());
    }

    #[test]
    fn test_validate_empty_message() {
        let mut s = Session::new("m");
        s.add_user_message("Hi");
        s.add_assistant_message("");

        let report = validate_session_integrity(&s);
        let empty = report.issues.iter().find(|i| i.code == "EMPTY_MESSAGE");
        assert!(empty.is_some());
    }

    #[test]
    fn test_integrity_report_format() {
        let report = SessionIntegrityReport::valid();
        let text = report.format_report();
        assert!(text.contains("Valid"));
        assert!(text.contains("0 errors"));
    }

    // --- Integrity repair ---

    #[test]
    fn test_repair_orphan_tool_results() {
        let mut s = Session::new("m");
        s.add_user_message("Hi");
        s.add_assistant_message("Hello");
        s.add_tool_result("orphan_id", "orphan result");

        let opts = IntegrityRepairOptions::default();
        let report = repair_session_integrity(&mut s, &opts);

        assert!(report.has_repairs());
        assert_eq!(report.orphan_tools_fixed, 1);
        // Orphan tool result should be removed
        assert!(!s.messages.iter().any(|m| m.role == Role::Tool));
    }

    #[test]
    fn test_repair_dangling_tool_calls() {
        let mut s = Session::new("m");
        s.add_user_message("Do it");
        s.add_assistant_with_tools(
            "Calling",
            vec![ToolCall {
                id: "tc_x".to_string(),
                name: "some_tool".to_string(),
                arguments: "{}".to_string(),
            }],
        );
        // No tool result, then next user message
        s.add_user_message("Next question");
        s.add_assistant_message("answer");

        let opts = IntegrityRepairOptions::default();
        let report = repair_session_integrity(&mut s, &opts);

        assert_eq!(report.dangling_calls_fixed, 1);
        // A synthesized tool result should have been inserted
        assert!(s.messages.iter().any(|m| m.role == Role::Tool));
    }

    #[test]
    fn test_repair_empty_messages() {
        let mut s = Session::new("m");
        s.add_user_message("Hi");
        s.add_assistant_message(""); // empty
        s.add_user_message("Bye");
        s.add_assistant_message("Later");

        let opts = IntegrityRepairOptions::default();
        let report = repair_session_integrity(&mut s, &opts);

        assert_eq!(report.empty_messages_removed, 1);
    }

    #[test]
    fn test_repair_stray_system_messages() {
        let mut s = Session::new("m");
        s.add_user_message("Hi");
        s.add_assistant_message("Hello");
        // Stray system message mid-conversation
        s.messages.push(Message::system("stray system instruction"));
        s.add_user_message("Bye");
        s.add_assistant_message("Later");

        let opts = IntegrityRepairOptions::default();
        let report = repair_session_integrity(&mut s, &opts);

        assert_eq!(report.stray_systems_fixed, 1);
        // Stray content moved to system_prompt
        assert!(s.system_prompt.as_deref().unwrap_or("").contains("stray system instruction"));
    }

    // --- Session forking ---

    #[test]
    fn test_fork_at_turn_1() {
        let s = make_surgical_session();
        let forked = fork_session_at_turn(&s, 1, None).unwrap();

        assert_ne!(forked.id(), s.id());
        let turns = extract_patchable_turns(&forked);
        assert_eq!(turns.len(), 1);
        assert_eq!(turns[0].user_message.as_deref(), Some("Hello"));
        assert!(forked.title.as_deref().unwrap().contains("Fork @ Turn 1"));
        assert_eq!(
            forked.metadata.get("forked_from_session_id").unwrap(),
            &s.id().to_string()
        );
    }

    #[test]
    fn test_fork_at_turn_2() {
        let s = make_surgical_session();
        let forked = fork_session_at_turn(&s, 2, None).unwrap();

        let turns = extract_patchable_turns(&forked);
        assert_eq!(turns.len(), 2);
        assert_eq!(turns[1].user_message.as_deref(), Some("How are you?"));
    }

    #[test]
    fn test_fork_at_last_turn() {
        let s = make_surgical_session();
        let forked = fork_session_at_turn(&s, 3, None).unwrap();

        let turns = extract_patchable_turns(&forked);
        assert_eq!(turns.len(), 3);
    }

    #[test]
    fn test_fork_out_of_bounds() {
        let s = make_surgical_session();
        assert!(fork_session_at_turn(&s, 0, None).is_err());
        assert!(fork_session_at_turn(&s, 99, None).is_err());
    }

    #[test]
    fn test_fork_with_custom_options() {
        let s = make_surgical_session();
        let opts = ForkPatchOptions {
            custom_title: Some("My Fork".to_string()),
            custom_model: Some("gpt-4o".to_string()),
            custom_system_prompt: Some("Custom prompt".to_string()),
            additional_metadata: {
                let mut m = HashMap::new();
                m.insert("branch".to_string(), "experimental".to_string());
                m
            },
        };
        let forked = fork_session_at_turn(&s, 2, Some(&opts)).unwrap();

        assert_eq!(forked.title.as_deref(), Some("My Fork"));
        assert_eq!(forked.active_model, "gpt-4o");
        assert_eq!(forked.system_prompt.as_deref(), Some("Custom prompt"));
        assert_eq!(forked.metadata.get("branch").unwrap(), "experimental");
    }

    #[test]
    fn test_create_session_fork_convenience() {
        let s = make_surgical_session();
        let forked = create_session_fork(&s, 1, Some("Quick Fork".to_string())).unwrap();
        assert_eq!(forked.title.as_deref(), Some("Quick Fork"));
    }

    #[test]
    fn test_fork_preserves_system_prompt() {
        let mut s = Session::with_system_prompt("m", "Be helpful");
        s.add_user_message("Hi");
        s.add_assistant_message("Hello");
        s.add_user_message("Bye");
        s.add_assistant_message("Cya");

        let forked = fork_session_at_turn(&s, 1, None).unwrap();
        assert_eq!(forked.system_prompt.as_deref(), Some("Be helpful"));
    }

    #[test]
    fn test_fork_recomputes_tokens() {
        let s = make_surgical_session();
        let forked = fork_session_at_turn(&s, 1, None).unwrap();
        assert!(forked.token_stats.total_tokens > 0);
        assert_eq!(forked.token_stats.total_turns, 1);
    }

    // --- SessionPatcher builder ---

    #[test]
    fn test_patcher_chained_operations() {
        let s = make_surgical_session();
        let mut patcher = SessionPatcher::new(s);
        patcher
            .modify_user(1, "Changed Q1")
            .modify_assistant(1, "Changed A1")
            .delete_turn(3);

        let (result, report) = patcher.apply_with_report().unwrap();

        assert_eq!(report.operations_applied, 3);
        let turns = extract_patchable_turns(&result);
        assert_eq!(turns.len(), 2);
        assert_eq!(turns[0].user_message.as_deref(), Some("Changed Q1"));
        assert_eq!(turns[0].assistant_message.as_deref(), Some("Changed A1"));
    }

    #[test]
    fn test_patcher_dry_run() {
        let s = make_surgical_session();
        let mut patcher = SessionPatcher::new(s.clone());
        patcher.delete_turn(1);

        let dry_report = patcher.dry_run().unwrap();
        assert_eq!(dry_report.total_operations, 1);
        assert_eq!(dry_report.turns_before, 3);
        assert_eq!(dry_report.turns_after, 2);

        // Original session should be unchanged after dry_run
        let turns = extract_patchable_turns(&s);
        assert_eq!(turns.len(), 3);
    }

    #[test]
    fn test_patcher_set_metadata() {
        let s = make_surgical_session();
        let mut patcher = SessionPatcher::new(s);
        patcher
            .set_metadata("key1", "val1")
            .set_title(Some("New Title".to_string()))
            .set_active_model("gpt-4o")
            .set_system_prompt(Some("New prompt".to_string()));

        let result = patcher.apply().unwrap();
        assert_eq!(result.metadata.get("key1").unwrap(), "val1");
        assert_eq!(result.title.as_deref(), Some("New Title"));
        assert_eq!(result.active_model, "gpt-4o");
        assert_eq!(result.system_prompt.as_deref(), Some("New prompt"));
    }

    #[test]
    fn test_patcher_remove_metadata() {
        let mut s = make_surgical_session();
        s.metadata.insert("existing".to_string(), "value".to_string());

        let mut patcher = SessionPatcher::new(s);
        patcher.remove_metadata("existing");

        let result = patcher.apply().unwrap();
        assert!(result.metadata.get("existing").is_none());
    }

    #[test]
    fn test_patcher_strict_integrity_failure() {
        let mut s = Session::new("m");
        // Create a session that will have integrity errors after patching
        s.add_user_message("Hi");
        s.add_assistant_message("Hello");

        let mut patcher = SessionPatcher::new(s)
            .with_strict_integrity(true);
        // Insert an orphan tool result to trigger integrity error
        patcher.insert_message(
            2,
            Message::tool_result("no_such_call", "orphaned"),
        );

        let result = patcher.apply();
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            SessionPatchError::IntegrityError { .. }
        ));
    }

    #[test]
    fn test_patcher_with_auto_repair() {
        let mut s = Session::new("m");
        s.add_user_message("Hi");
        s.add_assistant_message("Hello");

        let mut patcher = SessionPatcher::new(s)
            .with_auto_repair(true)
            .with_strict_integrity(false);
        patcher.insert_message(
            2,
            Message::tool_result("no_such_call", "orphaned"),
        );

        // Should succeed because auto-repair removes the orphan
        let result = patcher.apply();
        assert!(result.is_ok());
    }

    #[test]
    fn test_patcher_no_token_recompute() {
        let s = make_surgical_session();
        let mut patcher = SessionPatcher::new(s)
            .with_auto_recompute_tokens(false);
        patcher.modify_user(1, "Changed");

        let (_, report) = patcher.apply_with_report().unwrap();
        assert_eq!(report.token_report.token_delta, 0);
    }

    // --- PatchReport ---

    #[test]
    fn test_patch_report_format() {
        let mut s = make_surgical_session();
        let report = patch_delete_turn(&mut s, 1).unwrap();
        let summary = report.format_summary();
        assert!(summary.contains("1 op(s)"));
        assert!(summary.contains("Turns: 3 -> 2"));
    }

    // --- Patch saved session file ---

    #[test]
    fn test_patch_saved_session_file() {
        let temp = tempdir().unwrap();
        let path = temp.path().join("test_session.json");

        let s = make_surgical_session();
        let content = serde_json::to_string_pretty(&s).unwrap();
        fs::write(&path, &content).unwrap();

        let ops = vec![
            SessionPatchOp::ModifyUser {
                turn_index: 1,
                new_content: "Modified from file".to_string(),
            },
            SessionPatchOp::DeleteTurn { turn_index: 3 },
        ];

        let result = patch_saved_session_file(&path, &ops).unwrap();
        let turns = extract_patchable_turns(&result);
        assert_eq!(turns.len(), 2);
        assert_eq!(turns[0].user_message.as_deref(), Some("Modified from file"));

        // Verify file was written back
        let reloaded: Session = serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(reloaded.messages.len(), result.messages.len());
    }

    // --- Turn with tool calls ---

    #[test]
    fn test_extract_turns_with_tool_calls() {
        let s = make_test_session(); // uses tool calls
        let turns = extract_patchable_turns(&s);

        assert_eq!(turns.len(), 3);
        assert!(turns[0].tool_calls_count > 0);
        assert_eq!(turns[0].user_message.as_deref(), Some("Please create src/lib.rs with an add function."));
    }

    #[test]
    fn test_modify_turn_with_tool_calls() {
        let mut s = make_test_session();
        patch_modify_turn_user(&mut s, 1, "Updated tool question").unwrap();

        let turns = extract_patchable_turns(&s);
        assert_eq!(turns[0].user_message.as_deref(), Some("Updated tool question"));
        // Tool call messages should still be there
        assert!(turns[0].tool_calls_count > 0);
    }

    #[test]
    fn test_delete_turn_with_tool_calls() {
        let mut s = make_test_session();
        let original_msg_count = s.messages.len();
        patch_delete_turn(&mut s, 1).unwrap();

        // Turn 1 had user + assistant(with tools) + tool_result = 3 messages
        assert!(s.messages.len() < original_msg_count);
        let turns = extract_patchable_turns(&s);
        assert_eq!(turns.len(), 2);
    }

    // --- TurnData with tools ---

    #[test]
    fn test_turn_data_with_tools() {
        let tool_calls = vec![ToolCall {
            id: "tc1".to_string(),
            name: "read".to_string(),
            arguments: "{}".to_string(),
        }];
        let asst = Message::assistant_with_tools("reading file", tool_calls);
        let tool_results = vec![Message::tool_result("tc1", "file content")];
        let final_msg = Message::assistant("Done reading");

        let td = TurnData::with_tools("Read it", asst, tool_results, Some(final_msg));
        assert_eq!(td.len(), 4);
        assert_eq!(td.user_content(), Some("Read it"));
        assert_eq!(td.assistant_content(), Some("Done reading"));
        assert_eq!(td.tool_calls_count(), 1);
    }

    // --- Edge cases ---

    #[test]
    fn test_patcher_on_empty_session() {
        let s = Session::new("m");

        // Append a turn to an empty session
        let mut patcher = SessionPatcher::new(s);
        patcher.append_turn(TurnData::new("Q", "A"));

        let (result, report) = patcher.apply_with_report().unwrap();
        assert_eq!(report.turns_before, 0);
        assert_eq!(report.turns_after, 1);

        let turns = extract_patchable_turns(&result);
        assert_eq!(turns[0].user_message.as_deref(), Some("Q"));
    }

    #[test]
    fn test_insert_into_empty_session() {
        let mut s = Session::new("m");
        let turn = TurnData::new("Q", "A");
        // InsertTurn on empty session should work gracefully
        let mut patcher = SessionPatcher::new(s);
        patcher.insert_turn_before(1, turn);

        let result = patcher.apply().unwrap();
        let turns = extract_patchable_turns(&result);
        assert_eq!(turns.len(), 1);
    }

    #[test]
    fn test_session_with_system_messages_preserved() {
        let mut s = Session::with_system_prompt("m", "System context");
        s.messages.insert(0, Message::system("System context"));
        s.add_user_message("Q1");
        s.add_assistant_message("A1");
        s.add_user_message("Q2");
        s.add_assistant_message("A2");

        // Delete turn 1 should preserve system messages
        let mut patcher = SessionPatcher::new(s);
        patcher.delete_turn(1);

        let result = patcher.apply().unwrap();
        // System message at index 0 should remain
        assert_eq!(result.messages[0].role, Role::System);
        let turns = extract_patchable_turns(&result);
        assert_eq!(turns.len(), 1);
        assert_eq!(turns[0].user_message.as_deref(), Some("Q2"));
    }

    #[test]
    fn test_multiple_patches_sequential() {
        let mut s = make_surgical_session();

        // First patch: modify turn 1
        patch_modify_turn_user(&mut s, 1, "Changed 1").unwrap();

        // Second patch: delete turn 2 (which is now the original turn 2)
        patch_delete_turn(&mut s, 2).unwrap();

        // Third patch: append
        patch_append_turn(&mut s, TurnData::new("New Q", "New A")).unwrap();

        let turns = extract_patchable_turns(&s);
        assert_eq!(turns.len(), 3);
        assert_eq!(turns[0].user_message.as_deref(), Some("Changed 1"));
        assert_eq!(turns[1].user_message.as_deref(), Some("Goodbye"));
        assert_eq!(turns[2].user_message.as_deref(), Some("New Q"));
    }

    #[test]
    fn test_insert_position_serialization() {
        let before = serde_json::to_string(&InsertPosition::Before).unwrap();
        let after = serde_json::to_string(&InsertPosition::After).unwrap();
        assert_eq!(serde_json::from_str::<InsertPosition>(&before).unwrap(), InsertPosition::Before);
        assert_eq!(serde_json::from_str::<InsertPosition>(&after).unwrap(), InsertPosition::After);
    }

    #[test]
    fn test_session_patch_op_serialization() {
        let op = SessionPatchOp::ModifyUser {
            turn_index: 1,
            new_content: "test".to_string(),
        };
        let json = serde_json::to_string(&op).unwrap();
        let deserialized: SessionPatchOp = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized, op);
    }

    #[test]
    fn test_integrity_severity_levels() {
        let issue_err = IntegrityIssue {
            severity: IntegritySeverity::Error,
            message_index: Some(0),
            turn_index: Some(1),
            code: "TEST_ERROR".to_string(),
            description: "test error".to_string(),
            auto_fixable: false,
        };
        let issue_warn = IntegrityIssue {
            severity: IntegritySeverity::Warning,
            message_index: None,
            turn_index: None,
            code: "TEST_WARN".to_string(),
            description: "test warning".to_string(),
            auto_fixable: true,
        };

        let report = SessionIntegrityReport::from_issues(vec![issue_err, issue_warn]);
        assert!(!report.is_valid);
        assert_eq!(report.error_count, 1);
        assert_eq!(report.warning_count, 1);
        assert!(report.has_errors());
        assert!(report.has_warnings());

        let formatted = report.format_report();
        assert!(formatted.contains("Invalid"));
        assert!(formatted.contains("TEST_ERROR"));
        assert!(formatted.contains("[auto-fixable]"));
    }
}
