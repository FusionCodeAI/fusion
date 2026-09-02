//! Checkpoint undo and redo subsystem for Fusion.
//!
//! Provides automated, reliable snapshotting of files before tool executions
//! and manual checkpoints, allowing instant `/undo` to revert file mutations
//! (modifications, creations, deletions) back to their pre-execution state.
//!
//! # Key Features
//! - **Transactional Snapshots**: Complete pre-edit and post-edit snapshots with
//!   byte/content hashing, line stats, and POSIX permissions.
//! - **Multi-Step Undo & Redo**: Deep undo stack and redo stack with multi-turn
//!   traversal (`undo_n`, `undo_to`, `redo_n`, `redo_to`).
//! - **Surgical Revert**: Revert single files or detect dirty file tree conflicts
//!   before restoring on-disk content.
//! - **Diff Inspection**: Unified, ANSI-colorized, and structured hunk diffs with
//!   Git-like statistics (insertions, deletions, bytes delta).
//! - **Persistence**: Optional disk serialization to `.fusion/checkpoints/` for cross-session recovery.
//! - **Bounded Memory**: Configurable checkpoint retention limit with FIFO and memory-size pruning.

use std::collections::{HashMap, HashSet};
use std::fs;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use chrono::Utc;
use serde::{Deserialize, Serialize};
use similar::{ChangeTag, TextDiff};
use thiserror::Error;

/// Default maximum number of checkpoints retained in history.
pub const DEFAULT_MAX_CHECKPOINTS: usize = 100;

/// Default maximum memory footprint for all retained checkpoints in bytes (100 MB).
pub const DEFAULT_MAX_MEMORY_BYTES: usize = 100 * 1024 * 1024;

/// Maximum size of an individual file snapshot in bytes (50 MB).
pub const MAX_SNAPSHOT_FILE_SIZE: usize = 50 * 1024 * 1024;

/// Default context radius for unified diff generation.
pub const DEFAULT_DIFF_CONTEXT_RADIUS: usize = 3;

// ---------------------------------------------------------------------------
// FileState
// ---------------------------------------------------------------------------

/// Represents the physical state of a file at a specific moment in time.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", content = "data")]
pub enum FileState {
    /// File existed on disk with specific content and metadata.
    Present {
        /// Raw file bytes.
        content: Vec<u8>,
        /// Whether the file appears to be non-UTF8 binary data.
        is_binary: bool,
        /// POSIX file mode / permissions if available.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        permissions: Option<u32>,
        /// File size in bytes.
        size: usize,
        /// Fast content hash for equality checks.
        hash: String,
        /// Total number of text lines (0 if binary).
        #[serde(default)]
        line_count: usize,
        /// Timestamp when the file state was captured.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        modified_timestamp: Option<String>,
    },
    /// File did not exist on disk.
    Absent,
    /// Path existed as a directory.
    Directory,
    /// Path existed as a symbolic link.
    Symlink {
        /// Target path pointed to by the symlink.
        target: PathBuf,
    },
}

impl FileState {
    /// Captures the current state of the filesystem entry at `path`.
    pub fn from_path(path: &Path) -> std::io::Result<Self> {
        if !path.exists() {
            // Check if it's a broken symlink
            if let Ok(target) = fs::read_link(path) {
                return Ok(FileState::Symlink { target });
            }
            if path.symlink_metadata().is_ok() {
                // Symlink exists but target is missing
                let _ = fs::remove_file(path);
                return Ok(FileState::Absent);
            }
            return Ok(FileState::Absent);
        }

        let metadata = match path.symlink_metadata() {
            Ok(m) => m,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(FileState::Absent),
            Err(e) => return Err(e),
        };

        if metadata.file_type().is_symlink() {
            if let Ok(target) = fs::read_link(path) {
                return Ok(FileState::Symlink { target });
            }
        }

        if metadata.is_dir() {
            return Ok(FileState::Directory);
        }

        let size = metadata.len() as usize;
        if size > MAX_SNAPSHOT_FILE_SIZE {
            return Ok(FileState::Present {
                content: Vec::new(),
                is_binary: true,
                permissions: get_permissions(&metadata),
                size,
                hash: format!("oversized_{}", size),
                line_count: 0,
                modified_timestamp: Some(Utc::now().to_rfc3339()),
            });
        }

        let bytes = fs::read(path)?;
        let is_binary = is_binary_content(&bytes);
        let hash = compute_content_hash(&bytes);
        let permissions = get_permissions(&metadata);
        let line_count = if is_binary { 0 } else { count_lines(&bytes) };

        Ok(FileState::Present {
            content: bytes,
            is_binary,
            permissions,
            size,
            hash,
            line_count,
            modified_timestamp: Some(Utc::now().to_rfc3339()),
        })
    }

    /// Creates a `FileState::Present` from raw byte data.
    pub fn from_bytes(bytes: Vec<u8>, permissions: Option<u32>) -> Self {
        let is_binary = is_binary_content(&bytes);
        let hash = compute_content_hash(&bytes);
        let size = bytes.len();
        let line_count = if is_binary { 0 } else { count_lines(&bytes) };
        FileState::Present {
            content: bytes,
            is_binary,
            permissions,
            size,
            hash,
            line_count,
            modified_timestamp: Some(Utc::now().to_rfc3339()),
        }
    }

    /// Creates a `FileState::Present` from UTF-8 string slice.
    pub fn from_str(text: &str) -> Self {
        Self::from_bytes(text.as_bytes().to_vec(), None)
    }

    /// Restores this state to disk at `path`.
    pub fn restore_to_path(&self, path: &Path) -> std::io::Result<FileActionTaken> {
        match self {
            FileState::Absent => {
                if path.exists() || path.symlink_metadata().is_ok() {
                    if path.is_dir() {
                        fs::remove_dir_all(path)?;
                    } else {
                        fs::remove_file(path)?;
                    }
                    Ok(FileActionTaken::DeletedFile)
                } else {
                    Ok(FileActionTaken::Unchanged)
                }
            }
            FileState::Present {
                content,
                permissions,
                ..
            } => {
                // Ensure parent directory exists
                if let Some(parent) = path.parent() {
                    if !parent.exists() {
                        fs::create_dir_all(parent)?;
                    }
                }

                // Check if current on-disk content already matches exactly
                if path.is_file() {
                    if let Ok(current_bytes) = fs::read(path) {
                        if current_bytes == *content {
                            return Ok(FileActionTaken::Unchanged);
                        }
                    }
                }

                fs::write(path, content)?;

                #[cfg(unix)]
                if let Some(mode) = permissions {
                    use std::os::unix::fs::PermissionsExt;
                    let _ = fs::set_permissions(path, fs::Permissions::from_mode(*mode));
                }

                Ok(FileActionTaken::RestoredFile)
            }
            FileState::Directory => {
                if !path.exists() {
                    fs::create_dir_all(path)?;
                    Ok(FileActionTaken::RecreatedDir)
                } else {
                    Ok(FileActionTaken::Unchanged)
                }
            }
            FileState::Symlink { target } => {
                if path.exists() || path.symlink_metadata().is_ok() {
                    let _ = fs::remove_file(path);
                }
                #[cfg(unix)]
                {
                    std::os::unix::fs::symlink(target, path)?;
                    Ok(FileActionTaken::RestoredFile)
                }
                #[cfg(not(unix))]
                {
                    let _ = target;
                    Ok(FileActionTaken::Skipped)
                }
            }
        }
    }

    /// Returns content as a UTF-8 string slice if present and valid UTF-8.
    pub fn content_as_str(&self) -> Option<&str> {
        match self {
            FileState::Present {
                content, is_binary, ..
            } if !is_binary => std::str::from_utf8(content).ok(),
            _ => None,
        }
    }

    /// Returns the raw content bytes if present.
    pub fn content_bytes(&self) -> Option<&[u8]> {
        match self {
            FileState::Present { content, .. } => Some(content),
            _ => None,
        }
    }

    /// Returns true if the file existed.
    pub fn is_present(&self) -> bool {
        matches!(self, FileState::Present { .. })
    }

    /// Returns true if the file did not exist.
    pub fn is_absent(&self) -> bool {
        matches!(self, FileState::Absent)
    }

    /// Returns true if the file was a directory.
    pub fn is_directory(&self) -> bool {
        matches!(self, FileState::Directory)
    }

    /// Returns true if the file is binary.
    pub fn is_binary(&self) -> bool {
        match self {
            FileState::Present { is_binary, .. } => *is_binary,
            _ => false,
        }
    }

    /// Returns the size in bytes (0 if absent, directory, or symlink).
    pub fn size(&self) -> usize {
        match self {
            FileState::Present { size, .. } => *size,
            _ => 0,
        }
    }

    /// Returns the hash if present.
    pub fn hash(&self) -> Option<&str> {
        match self {
            FileState::Present { hash, .. } => Some(hash.as_str()),
            _ => None,
        }
    }

    /// Returns total lines of text (0 if binary, absent, or directory).
    pub fn line_count(&self) -> usize {
        match self {
            FileState::Present { line_count, .. } => *line_count,
            _ => 0,
        }
    }

    /// Returns true if the on-disk state matches this snapshot.
    pub fn matches_disk(&self, path: &Path) -> bool {
        match (self, FileState::from_path(path)) {
            (FileState::Absent, Ok(FileState::Absent)) => true,
            (FileState::Directory, Ok(FileState::Directory)) => true,
            (FileState::Symlink { target: t1 }, Ok(FileState::Symlink { target: t2 })) => *t1 == t2,
            (
                FileState::Present { hash: h1, .. },
                Ok(FileState::Present { hash: h2, .. }),
            ) => h1 == &h2,
            _ => false,
        }
    }
}

// ---------------------------------------------------------------------------
// FileActionTaken & FileChangeType
// ---------------------------------------------------------------------------

/// Action taken during file state restoration.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum FileActionTaken {
    /// File was restored to its previous content.
    RestoredFile,
    /// File was deleted because it did not exist before the tool.
    DeletedFile,
    /// Directory was recreated.
    RecreatedDir,
    /// No change was needed (already in target state).
    Unchanged,
    /// Operation skipped or failed.
    Skipped,
    /// Operation refused due to detected on-disk conflict.
    ConflictRefused,
}

/// Categorization of changes between two file states.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum FileChangeType {
    /// File was created (Absent -> Present).
    Created,
    /// File content was modified (Present -> Present with different content).
    Modified,
    /// File was deleted (Present -> Absent).
    Deleted,
    /// File was unchanged (same content or both absent).
    Unchanged,
    /// File type changed (e.g. file -> directory or vice versa).
    TypeChanged,
}

impl FileChangeType {
    /// Determines the change type between a `before` and `after` file state.
    pub fn detect(before: &FileState, after: &FileState) -> Self {
        match (before, after) {
            (FileState::Absent, FileState::Present { .. }) => FileChangeType::Created,
            (FileState::Present { .. }, FileState::Absent) => FileChangeType::Deleted,
            (
                FileState::Present { hash: h1, .. },
                FileState::Present { hash: h2, .. },
            ) => {
                if h1 == h2 {
                    FileChangeType::Unchanged
                } else {
                    FileChangeType::Modified
                }
            }
            (FileState::Absent, FileState::Absent) => FileChangeType::Unchanged,
            (FileState::Directory, FileState::Directory) => FileChangeType::Unchanged,
            (FileState::Symlink { target: t1 }, FileState::Symlink { target: t2 }) if t1 == t2 => {
                FileChangeType::Unchanged
            }
            _ => FileChangeType::TypeChanged,
        }
    }

    /// Returns a short human-readable label.
    pub fn label(&self) -> &'static str {
        match self {
            FileChangeType::Created => "created",
            FileChangeType::Modified => "modified",
            FileChangeType::Deleted => "deleted",
            FileChangeType::Unchanged => "unchanged",
            FileChangeType::TypeChanged => "type changed",
        }
    }

    /// Returns an ANSI-colorized badge string.
    pub fn badge(&self) -> &'static str {
        match self {
            FileChangeType::Created => "\x1b[32m[+] created\x1b[0m",
            FileChangeType::Modified => "\x1b[33m[~] modified\x1b[0m",
            FileChangeType::Deleted => "\x1b[31m[-] deleted\x1b[0m",
            FileChangeType::Unchanged => "\x1b[2;37m[=] unchanged\x1b[0m",
            FileChangeType::TypeChanged => "\x1b[35m[*] type changed\x1b[0m",
        }
    }
}

// ---------------------------------------------------------------------------
// Revert Safety & Conflict Errors
// ---------------------------------------------------------------------------

/// Status of safety check before executing an undo or revert on a file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum RevertSafety {
    /// Safe to revert: on-disk state matches expected post-state or expected change.
    Safe,
    /// Safe: file is already in the target pre-edit state.
    AlreadyInPreState,
    /// Conflicted: on-disk file has been modified externally after the checkpoint.
    Conflicted {
        reason: String,
        current_hash: Option<String>,
        expected_hash: Option<String>,
    },
}

/// Errors occurring during revert conflict analysis.
#[derive(Debug, Error)]
pub enum RevertConflictError {
    #[error("Conflict detected reverting '{path}': {reason}")]
    Conflict {
        path: PathBuf,
        reason: String,
        current_hash: Option<String>,
        expected_hash: Option<String>,
    },
    #[error("I/O error during revert of '{path}': {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
}

// ---------------------------------------------------------------------------
// DiffStats, DiffHunk & FileDiff
// ---------------------------------------------------------------------------

/// Statistics for lines and bytes changed across one or more files.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiffStats {
    /// Number of added/inserted lines.
    pub insertions: usize,
    /// Number of removed/deleted lines.
    pub deletions: usize,
    /// Number of files with modifications.
    pub files_changed: usize,
    /// Net change in file size in bytes (post - pre).
    pub bytes_delta: i64,
}

impl DiffStats {
    /// Formats a concise summary string (e.g. `+12 -4 (1 file)`).
    pub fn format_short(&self) -> String {
        format!(
            "+{} -{} ({} {})",
            self.insertions,
            self.deletions,
            self.files_changed,
            if self.files_changed == 1 { "file" } else { "files" }
        )
    }

    /// Formats a Git-style `--stat` line for a single file.
    pub fn format_git_stat(&self, file_name: &str, max_width: usize) -> String {
        let total = self.insertions + self.deletions;
        if total == 0 {
            return format!("{:<width$} | 0", file_name, width = max_width);
        }

        let bar_width = 20.min(total);
        let plus_count = (self.insertions * bar_width) / total.max(1);
        let minus_count = bar_width.saturating_sub(plus_count);

        let plus_bar = "+".repeat(plus_count);
        let minus_bar = "-".repeat(minus_count);

        format!(
            "{:<width$} | {:>4} \x1b[32m{}\x1b[31m{}\x1b[0m",
            file_name,
            total,
            plus_bar,
            minus_bar,
            width = max_width
        )
    }
}

/// Type of line inside a unified diff hunk.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum HunkLineKind {
    Context,
    Addition,
    Deletion,
}

/// A single line in a diff hunk with line numbering.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HunkLine {
    pub kind: HunkLineKind,
    pub content: String,
    pub old_lineno: Option<usize>,
    pub new_lineno: Option<usize>,
}

/// A unified diff hunk representation with parsed line ranges.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiffHunk {
    pub header: String,
    pub old_start: usize,
    pub old_lines: usize,
    pub new_start: usize,
    pub new_lines: usize,
    pub lines: Vec<HunkLine>,
}

/// Detailed diff representation for a file in a checkpoint.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileDiff {
    pub path: PathBuf,
    pub change_type: FileChangeType,
    pub unified_diff: Option<String>,
    pub colorized_diff: Option<String>,
    pub hunks: Vec<DiffHunk>,
    pub stats: DiffStats,
    pub is_binary: bool,
    pub bytes_before: usize,
    pub bytes_after: usize,
}

// ---------------------------------------------------------------------------
// FileSnapshot
// ---------------------------------------------------------------------------

/// A recorded snapshot of a single file's state before and after an operation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileSnapshot {
    /// Canonicalized or normalized path relative to workspace or absolute.
    pub path: PathBuf,
    /// Original path string provided in the tool call arguments.
    pub original_path: String,
    /// State of the file before tool execution.
    pub state_before: FileState,
    /// State of the file after tool execution (if captured).
    pub state_after: Option<FileState>,
    /// Timestamp when this snapshot was captured.
    pub timestamp: String,
    /// Arbitrary metadata key-value pairs.
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub metadata: HashMap<String, String>,
}

impl FileSnapshot {
    /// Creates a new `FileSnapshot` capturing the current state before tool execution.
    pub fn capture_before(path: PathBuf, original_path: String, cwd: &Path) -> std::io::Result<Self> {
        let full_path = resolve_target_path(&original_path, cwd);
        let state_before = FileState::from_path(&full_path)?;
        Ok(Self {
            path,
            original_path,
            state_before,
            state_after: None,
            timestamp: Utc::now().to_rfc3339(),
            metadata: HashMap::new(),
        })
    }

    /// Creates a `FileSnapshot` from explicit pre and post states.
    pub fn from_states(
        path: PathBuf,
        original_path: String,
        state_before: FileState,
        state_after: Option<FileState>,
    ) -> Self {
        Self {
            path,
            original_path,
            state_before,
            state_after,
            timestamp: Utc::now().to_rfc3339(),
            metadata: HashMap::new(),
        }
    }

    /// Captures the post-execution state of the file.
    pub fn capture_after(&mut self, cwd: &Path) -> std::io::Result<()> {
        let full_path = resolve_target_path(&self.original_path, cwd);
        let state_after = FileState::from_path(&full_path)?;
        self.state_after = Some(state_after);
        Ok(())
    }

    /// Computes the change type between `state_before` and `state_after` (or current disk).
    pub fn change_type(&self, cwd: Option<&Path>) -> FileChangeType {
        if let Some(after) = &self.state_after {
            FileChangeType::detect(&self.state_before, after)
        } else if let Some(cwd_path) = cwd {
            let full_path = resolve_target_path(&self.original_path, cwd_path);
            let current = FileState::from_path(&full_path).unwrap_or(FileState::Absent);
            FileChangeType::detect(&self.state_before, &current)
        } else {
            FileChangeType::Unchanged
        }
    }

    /// Performs a pre-flight safety check against on-disk state.
    pub fn check_revert_safety(&self, cwd: &Path) -> RevertSafety {
        let full_path = resolve_target_path(&self.original_path, cwd);
        let current = FileState::from_path(&full_path).unwrap_or(FileState::Absent);

        // Case 1: On-disk is already in pre-edit state
        if FileChangeType::detect(&self.state_before, &current) == FileChangeType::Unchanged {
            return RevertSafety::AlreadyInPreState;
        }

        // Case 2: Post-state was recorded and matches on-disk
        if let Some(after) = &self.state_after {
            if FileChangeType::detect(after, &current) == FileChangeType::Unchanged {
                return RevertSafety::Safe;
            }

            return RevertSafety::Conflicted {
                reason: format!(
                    "File '{}' has modified on disk since tool execution (current hash: {:?}, expected post hash: {:?})",
                    self.original_path,
                    current.hash(),
                    after.hash()
                ),
                current_hash: current.hash().map(|s| s.to_string()),
                expected_hash: after.hash().map(|s| s.to_string()),
            };
        }

        // If no post-state was recorded, consider it safe to revert
        RevertSafety::Safe
    }

    /// Reverts the file on disk to its `state_before`.
    pub fn revert(&self, cwd: &Path) -> std::io::Result<FileActionTaken> {
        let full_path = resolve_target_path(&self.original_path, cwd);
        self.state_before.restore_to_path(&full_path)
    }

    /// Reverts with optional safety check to prevent overwriting external changes.
    pub fn revert_with_safety_check(
        &self,
        cwd: &Path,
        force: bool,
    ) -> Result<FileActionTaken, RevertConflictError> {
        if !force {
            match self.check_revert_safety(cwd) {
                RevertSafety::Safe => {}
                RevertSafety::AlreadyInPreState => return Ok(FileActionTaken::Unchanged),
                RevertSafety::Conflicted {
                    reason,
                    current_hash,
                    expected_hash,
                } => {
                    return Err(RevertConflictError::Conflict {
                        path: self.path.clone(),
                        reason,
                        current_hash,
                        expected_hash,
                    });
                }
            }
        }

        self.revert(cwd).map_err(|e| RevertConflictError::Io {
            path: self.path.clone(),
            source: e,
        })
    }

    /// Re-applies the file changes from `state_after` to disk.
    pub fn redo(&self, cwd: &Path) -> std::io::Result<FileActionTaken> {
        if let Some(after) = &self.state_after {
            let full_path = resolve_target_path(&self.original_path, cwd);
            after.restore_to_path(&full_path)
        } else {
            Ok(FileActionTaken::Skipped)
        }
    }

    /// Computes unified diff string between `state_before` and `state_after` (or current disk).
    pub fn unified_diff(&self, cwd: Option<&Path>, context_radius: usize) -> Option<String> {
        let before_text = self.state_before.content_as_str().unwrap_or("");
        let after_text_owned;
        let after_text = if let Some(after) = &self.state_after {
            after.content_as_str().unwrap_or("")
        } else if let Some(cwd_path) = cwd {
            let full_path = resolve_target_path(&self.original_path, cwd_path);
            let current = FileState::from_path(&full_path).unwrap_or(FileState::Absent);
            match current {
                FileState::Present {
                    content,
                    is_binary: false,
                    ..
                } => {
                    after_text_owned = String::from_utf8_lossy(&content).to_string();
                    &after_text_owned
                }
                _ => "",
            }
        } else {
            return None;
        };

        if before_text == after_text && self.state_before.is_present() == (self.state_after.as_ref().map_or(false, |s| s.is_present())) {
            return None;
        }

        let diff = TextDiff::from_lines(before_text, after_text);
        Some(
            diff.unified_diff()
                .context_radius(context_radius)
                .header(
                    &format!("a/{}", self.original_path),
                    &format!("b/{}", self.original_path),
                )
                .to_string(),
        )
    }

    /// Computes line and byte change statistics for this snapshot.
    pub fn diff_stats(&self, cwd: Option<&Path>) -> DiffStats {
        let before_text = self.state_before.content_as_str().unwrap_or("");
        let after_text_owned;
        let after_text = if let Some(after) = &self.state_after {
            after.content_as_str().unwrap_or("")
        } else if let Some(cwd_path) = cwd {
            let full_path = resolve_target_path(&self.original_path, cwd_path);
            let current = FileState::from_path(&full_path).unwrap_or(FileState::Absent);
            match current {
                FileState::Present {
                    content,
                    is_binary: false,
                    ..
                } => {
                    after_text_owned = String::from_utf8_lossy(&content).to_string();
                    &after_text_owned
                }
                _ => "",
            }
        } else {
            ""
        };

        let diff = TextDiff::from_lines(before_text, after_text);
        let mut insertions = 0;
        let mut deletions = 0;

        for change in diff.iter_all_changes() {
            match change.tag() {
                ChangeTag::Insert => insertions += 1,
                ChangeTag::Delete => deletions += 1,
                ChangeTag::Equal => {}
            }
        }

        let bytes_before = self.state_before.size();
        let bytes_after = self.state_after.as_ref().map_or(0, |s| s.size());
        let bytes_delta = bytes_after as i64 - bytes_before as i64;
        let files_changed = if insertions > 0 || deletions > 0 || bytes_before != bytes_after {
            1
        } else {
            0
        };

        DiffStats {
            insertions,
            deletions,
            files_changed,
            bytes_delta,
        }
    }

    /// Computes approximate in-memory byte size of this snapshot.
    pub fn memory_size(&self) -> usize {
        self.state_before.size() + self.state_after.as_ref().map_or(0, |s| s.size()) + 256
    }
}

// ---------------------------------------------------------------------------
// FileTransaction
// ---------------------------------------------------------------------------

/// Lifecycle status of an in-flight file modification transaction.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TransactionStatus {
    /// Active transaction in progress.
    Pending,
    /// Successfully committed and moved to the undo stack.
    Committed,
    /// Rolled back to pre-edit state.
    RolledBack,
    /// Aborted and discarded without reverting disk.
    Aborted,
}

/// A transactional file modification session supporting atomic rollback.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileTransaction {
    /// Unique transaction ID.
    pub id: String,
    /// Tool name that initiated the transaction.
    pub tool_name: String,
    /// Human description of the transaction.
    pub description: String,
    /// Tool arguments.
    pub tool_args: Option<serde_json::Value>,
    /// Accumulated snapshots in this transaction.
    pub snapshots: HashMap<PathBuf, FileSnapshot>,
    /// Current transaction status.
    pub status: TransactionStatus,
    /// Creation timestamp.
    pub created_at: String,
    /// Arbitrary metadata.
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub metadata: HashMap<String, String>,
}

impl FileTransaction {
    /// Creates a new pending `FileTransaction`.
    pub fn new(
        id: impl Into<String>,
        tool_name: impl Into<String>,
        description: impl Into<String>,
    ) -> Self {
        Self {
            id: id.into(),
            tool_name: tool_name.into(),
            description: description.into(),
            tool_args: None,
            snapshots: HashMap::new(),
            status: TransactionStatus::Pending,
            created_at: Utc::now().to_rfc3339(),
            metadata: HashMap::new(),
        }
    }

    /// Records pre-edit state of a file if not already recorded.
    pub fn record_pre_edit(
        &mut self,
        path: PathBuf,
        original_path: String,
        cwd: &Path,
    ) -> std::io::Result<()> {
        if !self.snapshots.contains_key(&path) {
            let snapshot = FileSnapshot::capture_before(path.clone(), original_path, cwd)?;
            self.snapshots.insert(path, snapshot);
        }
        Ok(())
    }

    /// Records post-edit state of a file in the transaction.
    pub fn record_post_edit(&mut self, path: &Path, cwd: &Path) -> std::io::Result<()> {
        if let Some(snapshot) = self.snapshots.get_mut(path) {
            snapshot.capture_after(cwd)?;
        }
        Ok(())
    }

    /// Records an explicit file change with provided before/after states.
    pub fn record_explicit_change(
        &mut self,
        path: PathBuf,
        original_path: String,
        before: FileState,
        after: Option<FileState>,
    ) {
        let snapshot = FileSnapshot::from_states(path.clone(), original_path, before, after);
        self.snapshots.insert(path, snapshot);
    }

    /// Commits the transaction and converts it into a finalized `Checkpoint`.
    pub fn commit(mut self, cwd: &Path) -> Checkpoint {
        for snapshot in self.snapshots.values_mut() {
            if snapshot.state_after.is_none() {
                let _ = snapshot.capture_after(cwd);
            }
        }
        self.status = TransactionStatus::Committed;

        let mut chk = Checkpoint::new(self.id, self.tool_name, self.description);
        chk.tool_args = self.tool_args;
        chk.created_at = self.created_at;
        chk.snapshots = self.snapshots;
        chk.metadata = self.metadata;
        chk.status = CheckpointStatus::Active;
        chk
    }

    /// Rolls back all recorded files in this transaction immediately.
    pub fn rollback(&mut self, cwd: &Path) -> UndoResult {
        let mut chk = Checkpoint::new(&self.id, &self.tool_name, &self.description);
        chk.snapshots = self.snapshots.clone();
        let res = chk.revert(cwd);
        self.status = TransactionStatus::RolledBack;
        res
    }

    /// Aborts the transaction without restoring files.
    pub fn abort(&mut self) {
        self.status = TransactionStatus::Aborted;
    }
}

// ---------------------------------------------------------------------------
// CheckpointStatus & Checkpoint
// ---------------------------------------------------------------------------

/// Status of a checkpoint in the undo/redo history.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CheckpointStatus {
    /// Active checkpoint, candidate for undo.
    Active,
    /// Has been reverted via undo, candidate for redo.
    Undone,
    /// Has been re-applied via redo.
    Redone,
}

/// A recorded checkpoint containing snapshots of one or more files modified by a tool or action.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Checkpoint {
    /// Unique checkpoint identifier (e.g. `chk_1_1725280000000`).
    pub id: String,
    /// Conversational turn index when this checkpoint occurred.
    pub turn_index: Option<usize>,
    /// Tool call ID that triggered this checkpoint.
    pub tool_call_id: Option<String>,
    /// Name of the tool executed (e.g. `write`, `edit`, `patch`).
    pub tool_name: String,
    /// Tool arguments passed to the execution.
    pub tool_args: Option<serde_json::Value>,
    /// Human-readable description of what this checkpoint captures.
    pub description: String,
    /// Timestamp when the checkpoint was created.
    pub created_at: String,
    /// File snapshots keyed by canonical / relative path.
    pub snapshots: HashMap<PathBuf, FileSnapshot>,
    /// Lifecycle status of this checkpoint.
    pub status: CheckpointStatus,
    /// Arbitrary metadata key-value pairs.
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub metadata: HashMap<String, String>,
}

impl Checkpoint {
    /// Creates a new empty `Checkpoint`.
    pub fn new(
        id: impl Into<String>,
        tool_name: impl Into<String>,
        description: impl Into<String>,
    ) -> Self {
        Self {
            id: id.into(),
            turn_index: None,
            tool_call_id: None,
            tool_name: tool_name.into(),
            tool_args: None,
            description: description.into(),
            created_at: Utc::now().to_rfc3339(),
            snapshots: HashMap::new(),
            status: CheckpointStatus::Active,
            metadata: HashMap::new(),
        }
    }

    /// Adds a file snapshot to this checkpoint.
    pub fn add_snapshot(&mut self, snapshot: FileSnapshot) {
        self.snapshots.insert(snapshot.path.clone(), snapshot);
    }

    /// Returns list of all affected file paths in this checkpoint.
    pub fn file_paths(&self) -> Vec<&PathBuf> {
        self.snapshots.keys().collect()
    }

    /// Returns number of snapshotted files.
    pub fn file_count(&self) -> usize {
        self.snapshots.len()
    }

    /// Returns true if this checkpoint includes the given path.
    pub fn contains_path(&self, path: &Path) -> bool {
        self.snapshots.contains_key(path)
            || self
                .snapshots
                .values()
                .any(|s| s.original_path == path.to_string_lossy())
    }

    /// Gets reference to a file snapshot if present.
    pub fn get_snapshot(&self, path: &Path) -> Option<&FileSnapshot> {
        self.snapshots.get(path).or_else(|| {
            self.snapshots
                .values()
                .find(|s| s.original_path == path.to_string_lossy())
        })
    }

    /// Reverts all files in this checkpoint to their `state_before`.
    pub fn revert(&mut self, cwd: &Path) -> UndoResult {
        self.revert_with_safety(cwd, true)
    }

    /// Reverts files in this checkpoint with optional conflict safety checking.
    pub fn revert_with_safety(&mut self, cwd: &Path, force: bool) -> UndoResult {
        let mut result = UndoResult {
            checkpoint_id: self.id.clone(),
            tool_name: self.tool_name.clone(),
            description: self.description.clone(),
            restored_files: Vec::new(),
            deleted_files: Vec::new(),
            recreated_dirs: Vec::new(),
            unchanged_files: Vec::new(),
            conflicts: Vec::new(),
            errors: Vec::new(),
            success: true,
        };

        for snapshot in self.snapshots.values_mut() {
            // If post-state is not captured yet, capture current state as state_after before reverting
            if snapshot.state_after.is_none() {
                let _ = snapshot.capture_after(cwd);
            }

            match snapshot.revert_with_safety_check(cwd, force) {
                Ok(FileActionTaken::RestoredFile) => {
                    result.restored_files.push(snapshot.path.clone());
                }
                Ok(FileActionTaken::DeletedFile) => {
                    result.deleted_files.push(snapshot.path.clone());
                }
                Ok(FileActionTaken::RecreatedDir) => {
                    result.recreated_dirs.push(snapshot.path.clone());
                }
                Ok(FileActionTaken::Unchanged) => {
                    result.unchanged_files.push(snapshot.path.clone());
                }
                Ok(FileActionTaken::Skipped) => {}
                Ok(FileActionTaken::ConflictRefused) => {
                    result.success = false;
                    result
                        .conflicts
                        .push((snapshot.path.clone(), "Conflict refused".to_string()));
                }
                Err(RevertConflictError::Conflict { reason, .. }) => {
                    result.success = false;
                    result.conflicts.push((snapshot.path.clone(), reason));
                }
                Err(RevertConflictError::Io { source, .. }) => {
                    result.success = false;
                    result
                        .errors
                        .push((snapshot.path.clone(), source.to_string()));
                }
            }
        }

        if result.success {
            self.status = CheckpointStatus::Undone;
        }

        result
    }

    /// Reverts a single file within this checkpoint (surgical revert).
    pub fn revert_single_file(
        &mut self,
        path: &Path,
        cwd: &Path,
        force: bool,
    ) -> anyhow::Result<FileActionTaken> {
        let snapshot = match self.snapshots.get_mut(path) {
            Some(s) => s,
            None => self
                .snapshots
                .values_mut()
                .find(|s| s.original_path == path.to_string_lossy())
                .ok_or_else(|| {
                    anyhow::anyhow!("File '{}' not found in checkpoint '{}'", path.display(), self.id)
                })?,
        };

        if snapshot.state_after.is_none() {
            let _ = snapshot.capture_after(cwd);
        }

        let action = snapshot
            .revert_with_safety_check(cwd, force)
            .map_err(|e| anyhow::anyhow!("{}", e))?;

        Ok(action)
    }

    /// Re-applies all files in this checkpoint to their `state_after`.
    pub fn redo(&mut self, cwd: &Path) -> RedoResult {
        let mut result = RedoResult {
            checkpoint_id: self.id.clone(),
            tool_name: self.tool_name.clone(),
            description: self.description.clone(),
            reapplied_files: Vec::new(),
            recreated_files: Vec::new(),
            deleted_files: Vec::new(),
            errors: Vec::new(),
            success: true,
        };

        for snapshot in self.snapshots.values() {
            match snapshot.redo(cwd) {
                Ok(FileActionTaken::RestoredFile) => {
                    result.reapplied_files.push(snapshot.path.clone());
                }
                Ok(FileActionTaken::DeletedFile) => {
                    result.deleted_files.push(snapshot.path.clone());
                }
                Ok(FileActionTaken::RecreatedDir) => {
                    result.recreated_files.push(snapshot.path.clone());
                }
                Ok(FileActionTaken::Unchanged | FileActionTaken::Skipped | FileActionTaken::ConflictRefused) => {}
                Err(e) => {
                    result.success = false;
                    result
                        .errors
                        .push((snapshot.path.clone(), e.to_string()));
                }
            }
        }

        if result.success {
            self.status = CheckpointStatus::Redone;
        }

        result
    }

    /// Computes aggregated diff statistics across all files in this checkpoint.
    pub fn diff_stats(&self, cwd: Option<&Path>) -> DiffStats {
        let mut total = DiffStats::default();
        for snapshot in self.snapshots.values() {
            let stats = snapshot.diff_stats(cwd);
            total.insertions += stats.insertions;
            total.deletions += stats.deletions;
            total.files_changed += stats.files_changed;
            total.bytes_delta += stats.bytes_delta;
        }
        total
    }

    /// Computes detailed file diffs for all files in this checkpoint.
    pub fn compute_diffs(&self, cwd: Option<&Path>, context_radius: usize) -> Vec<FileDiff> {
        let mut diffs = Vec::new();
        for snapshot in self.snapshots.values() {
            let change_type = snapshot.change_type(cwd);
            let unified = snapshot.unified_diff(cwd, context_radius);
            let colorized = unified.as_ref().map(|u| colorize_diff_text(u));
            let hunks = unified
                .as_ref()
                .map(|u| parse_hunks_from_diff(u))
                .unwrap_or_default();
            let stats = snapshot.diff_stats(cwd);
            let is_binary = snapshot.state_before.is_binary()
                || snapshot.state_after.as_ref().map_or(false, |s| s.is_binary());

            diffs.push(FileDiff {
                path: snapshot.path.clone(),
                change_type,
                unified_diff: unified,
                colorized_diff: colorized,
                hunks,
                stats,
                is_binary,
                bytes_before: snapshot.state_before.size(),
                bytes_after: snapshot.state_after.as_ref().map_or(0, |s| s.size()),
            });
        }
        diffs
    }

    /// Generates a lightweight summary of this checkpoint.
    pub fn summary(&self) -> CheckpointSummary {
        let mut files: Vec<PathBuf> = self.snapshots.keys().cloned().collect();
        files.sort();
        let stats = Some(self.diff_stats(None));
        CheckpointSummary {
            id: self.id.clone(),
            tool_name: self.tool_name.clone(),
            description: self.description.clone(),
            created_at: self.created_at.clone(),
            file_count: self.snapshots.len(),
            files,
            status: self.status,
            stats,
        }
    }

    /// Computes approximate in-memory byte size of this checkpoint.
    pub fn memory_size(&self) -> usize {
        self.snapshots.values().map(|s| s.memory_size()).sum::<usize>() + 512
    }
}

// ---------------------------------------------------------------------------
// CheckpointSummary & Inspection
// ---------------------------------------------------------------------------

/// Lightweight summary for UI display or command output.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CheckpointSummary {
    pub id: String,
    pub tool_name: String,
    pub description: String,
    pub created_at: String,
    pub file_count: usize,
    pub files: Vec<PathBuf>,
    pub status: CheckpointStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stats: Option<DiffStats>,
}

/// Full inspection of a checkpoint's modifications and diffs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CheckpointDiffInspection {
    pub checkpoint_id: String,
    pub tool_name: String,
    pub description: String,
    pub created_at: String,
    pub files: Vec<FileDiff>,
    pub total_stats: DiffStats,
}

/// Summary of history stacks and memory usage.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CheckpointHistorySummary {
    pub undo_count: usize,
    pub redo_count: usize,
    pub active_checkpoints: Vec<CheckpointSummary>,
    pub undone_checkpoints: Vec<CheckpointSummary>,
    pub total_memory_bytes: usize,
}

// ---------------------------------------------------------------------------
// UndoResult, RedoResult & SurgicalRevertResult
// ---------------------------------------------------------------------------

/// Outcome of an `/undo` operation reverting a checkpoint.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct UndoResult {
    pub checkpoint_id: String,
    pub tool_name: String,
    pub description: String,
    pub restored_files: Vec<PathBuf>,
    pub deleted_files: Vec<PathBuf>,
    pub recreated_dirs: Vec<PathBuf>,
    pub unchanged_files: Vec<PathBuf>,
    pub conflicts: Vec<(PathBuf, String)>,
    pub errors: Vec<(PathBuf, String)>,
    pub success: bool,
}

impl UndoResult {
    /// Returns total count of file mutations reverted.
    pub fn total_reverted(&self) -> usize {
        self.restored_files.len() + self.deleted_files.len() + self.recreated_dirs.len()
    }

    /// Returns true if any errors occurred during reversion.
    pub fn has_errors(&self) -> bool {
        !self.errors.is_empty()
    }

    /// Returns true if any conflicts were detected during reversion.
    pub fn has_conflicts(&self) -> bool {
        !self.conflicts.is_empty()
    }

    /// Formats a concise summary string.
    pub fn format_summary(&self) -> String {
        let mut parts = Vec::new();
        if !self.restored_files.is_empty() {
            parts.push(format!("{} restored", self.restored_files.len()));
        }
        if !self.deleted_files.is_empty() {
            parts.push(format!("{} deleted", self.deleted_files.len()));
        }
        if !self.recreated_dirs.is_empty() {
            parts.push(format!("{} directories recreated", self.recreated_dirs.len()));
        }
        if parts.is_empty() {
            if !self.unchanged_files.is_empty() {
                "files already in pre-tool state".to_string()
            } else {
                "no files modified".to_string()
            }
        } else {
            parts.join(", ")
        }
    }
}

/// Outcome of a `/redo` operation re-applying a checkpoint.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RedoResult {
    pub checkpoint_id: String,
    pub tool_name: String,
    pub description: String,
    pub reapplied_files: Vec<PathBuf>,
    pub recreated_files: Vec<PathBuf>,
    pub deleted_files: Vec<PathBuf>,
    pub errors: Vec<(PathBuf, String)>,
    pub success: bool,
}

impl RedoResult {
    /// Returns total count of file mutations re-applied.
    pub fn total_reapplied(&self) -> usize {
        self.reapplied_files.len() + self.recreated_files.len() + self.deleted_files.len()
    }

    /// Returns true if any errors occurred during redo.
    pub fn has_errors(&self) -> bool {
        !self.errors.is_empty()
    }

    /// Formats a concise summary string.
    pub fn format_summary(&self) -> String {
        let mut parts = Vec::new();
        if !self.reapplied_files.is_empty() {
            parts.push(format!("{} reapplied", self.reapplied_files.len()));
        }
        if !self.recreated_files.is_empty() {
            parts.push(format!("{} recreated", self.recreated_files.len()));
        }
        if !self.deleted_files.is_empty() {
            parts.push(format!("{} deleted", self.deleted_files.len()));
        }
        if parts.is_empty() {
            "no files re-applied".to_string()
        } else {
            parts.join(", ")
        }
    }
}

/// Outcome of a surgical revert on an individual file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SurgicalRevertResult {
    pub checkpoint_id: String,
    pub path: PathBuf,
    pub action: FileActionTaken,
    pub was_forced: bool,
    pub diff_reverted: Option<String>,
    pub success: bool,
    pub error: Option<String>,
}

// ---------------------------------------------------------------------------
// Target Path Extraction
// ---------------------------------------------------------------------------

/// Intelligently extracts file paths targeted by a tool invocation from its name and JSON arguments.
pub fn extract_target_paths(
    tool_name: &str,
    args: &serde_json::Value,
    cwd: &Path,
) -> Vec<PathBuf> {
    let mut paths = Vec::new();

    let clean_name = tool_name.to_lowercase();
    match clean_name.as_str() {
        "write" | "write_file" | "edit" | "edit_file" => {
            if let Some(path_str) =
                extract_string_param(args, &["path", "file_path", "target", "file"])
            {
                paths.push(resolve_target_path(&path_str, cwd));
            }
        }
        "patch" | "apply_patch" => {
            if let Some(path_str) = extract_string_param(args, &["path", "file_path", "target"]) {
                paths.push(resolve_target_path(&path_str, cwd));
            }
            // Parse unified diff headers in patch text for target paths
            if let Some(patch_text) = extract_string_param(args, &["patch", "diff", "content"]) {
                for header_path in parse_patch_paths(&patch_text) {
                    paths.push(resolve_target_path(&header_path, cwd));
                }
            }
        }
        "git" | "git_checkout" | "git_reset" | "git_revert" => {
            if let Some(path_str) = extract_string_param(args, &["path", "file", "target"]) {
                paths.push(resolve_target_path(&path_str, cwd));
            }
        }
        "bash" | "exec" | "terminal" => {
            // Check for explicit file parameters or parse simple command targets
            if let Some(cmd) = extract_string_param(args, &["command", "cmd"]) {
                for extracted in extract_paths_from_shell_command(&cmd, cwd) {
                    paths.push(extracted);
                }
            }
            if let Some(path_str) =
                extract_string_param(args, &["path", "file_path", "target", "file"])
            {
                paths.push(resolve_target_path(&path_str, cwd));
            }
        }
        _ => {
            // Generic fallback: recursively scan JSON object for path-like fields
            scan_value_for_paths(args, cwd, &mut paths);
        }
    }

    // Deduplicate while preserving order
    let mut seen = HashSet::new();
    paths.retain(|p| seen.insert(p.clone()));
    paths
}

/// Recursively scans a JSON Value for path parameters.
fn scan_value_for_paths(value: &serde_json::Value, cwd: &Path, paths: &mut Vec<PathBuf>) {
    match value {
        serde_json::Value::Object(map) => {
            for (k, v) in map {
                let k_lower = k.to_lowercase();
                if matches!(
                    k_lower.as_str(),
                    "path"
                        | "file_path"
                        | "filepath"
                        | "target"
                        | "filename"
                        | "file"
                        | "dest"
                        | "destination"
                        | "source"
                        | "src"
                ) {
                    if let Some(s) = v.as_str() {
                        if !s.trim().is_empty() && !s.contains('\n') {
                            paths.push(resolve_target_path(s, cwd));
                        }
                    }
                } else if matches!(k_lower.as_str(), "files" | "paths" | "targets") {
                    if let Some(arr) = v.as_array() {
                        for item in arr {
                            if let Some(s) = item.as_str() {
                                paths.push(resolve_target_path(s, cwd));
                            }
                        }
                    }
                } else {
                    scan_value_for_paths(v, cwd, paths);
                }
            }
        }
        serde_json::Value::Array(arr) => {
            for v in arr {
                scan_value_for_paths(v, cwd, paths);
            }
        }
        _ => {}
    }
}

/// Extracts a string parameter matching one of several candidate keys.
fn extract_string_param(value: &serde_json::Value, keys: &[&str]) -> Option<String> {
    if let serde_json::Value::Object(map) = value {
        for key in keys {
            if let Some(val) = map.get(*key) {
                if let Some(s) = val.as_str() {
                    return Some(s.to_string());
                }
            }
        }
    }
    None
}

/// Parses target file paths from unified diff patch headers (`--- a/...` and `+++ b/...`).
fn parse_patch_paths(patch: &str) -> Vec<String> {
    let mut paths = Vec::new();
    for line in patch.lines() {
        let trimmed = line.trim();
        if let Some(rest) = trimmed.strip_prefix("+++ ") {
            let clean = rest.trim_start_matches("b/").trim();
            if clean != "/dev/null" && !clean.is_empty() {
                paths.push(clean.to_string());
            }
        } else if let Some(rest) = trimmed.strip_prefix("--- ") {
            let clean = rest.trim_start_matches("a/").trim();
            if clean != "/dev/null" && !clean.is_empty() {
                paths.push(clean.to_string());
            }
        }
    }
    paths
}

/// Heuristically extracts target file paths from common shell mutation commands.
fn extract_paths_from_shell_command(cmd: &str, cwd: &Path) -> Vec<PathBuf> {
    let mut paths = Vec::new();
    let trimmed = cmd.trim();

    // Check for output redirection `> file` or `>> file`
    if let Some(idx) = trimmed.rfind('>') {
        let after = trimmed[idx + 1..].trim();
        let target = after.split_whitespace().next().unwrap_or("");
        let clean = target.trim_matches(|c| c == '\'' || c == '"');
        if !clean.is_empty() && !clean.starts_with('&') {
            paths.push(resolve_target_path(clean, cwd));
        }
    }

    // Check for common commands: rm, touch, mv, cp
    let tokens: Vec<&str> = trimmed.split_whitespace().collect();
    if !tokens.is_empty() {
        let cmd_name = tokens[0];
        if matches!(cmd_name, "rm" | "touch" | "mv" | "cp" | "truncate") {
            for &token in &tokens[1..] {
                if !token.starts_with('-') {
                    let clean = token.trim_matches(|c| c == '\'' || c == '"');
                    if !clean.is_empty() {
                        paths.push(resolve_target_path(clean, cwd));
                    }
                }
            }
        }
    }

    paths
}

/// Resolves a path string relative to `cwd`, expanding `~` if applicable.
pub fn resolve_target_path(path_str: &str, cwd: &Path) -> PathBuf {
    let trimmed = path_str.trim();
    if let Some(stripped) = trimmed.strip_prefix("~/") {
        if let Some(home) = dirs::home_dir() {
            return home.join(stripped);
        }
    } else if trimmed == "~" {
        if let Some(home) = dirs::home_dir() {
            return home;
        }
    }

    let p = Path::new(trimmed);
    if p.is_absolute() {
        p.to_path_buf()
    } else {
        cwd.join(p)
    }
}

// ---------------------------------------------------------------------------
// CheckpointManager
// ---------------------------------------------------------------------------

/// Manages undo and redo stacks of file mutation checkpoints.
#[derive(Debug, Clone)]
pub struct CheckpointManager {
    /// Stack of checkpoints that can be undone (most recent last).
    undo_stack: Vec<Checkpoint>,
    /// Stack of checkpoints that can be redone (most recent undone last).
    redo_stack: Vec<Checkpoint>,
    /// Active in-flight file transaction (if any).
    active_transaction: Option<FileTransaction>,
    /// Maximum number of checkpoints to retain.
    max_checkpoints: usize,
    /// Maximum total memory footprint in bytes for retained snapshots.
    max_memory_bytes: usize,
    /// Context radius for unified diff generation.
    diff_context_radius: usize,
    /// Optional directory on disk where checkpoints are persisted.
    storage_dir: Option<PathBuf>,
    /// Working directory for path resolution.
    cwd: PathBuf,
    /// Monotonically increasing counter for checkpoint IDs.
    counter: usize,
    /// Whether to automatically persist checkpoints to disk.
    auto_persist: bool,
}

impl CheckpointManager {
    /// Creates a new `CheckpointManager` for the specified working directory.
    pub fn new(cwd: PathBuf) -> Self {
        Self {
            undo_stack: Vec::new(),
            redo_stack: Vec::new(),
            active_transaction: None,
            max_checkpoints: DEFAULT_MAX_CHECKPOINTS,
            max_memory_bytes: DEFAULT_MAX_MEMORY_BYTES,
            diff_context_radius: DEFAULT_DIFF_CONTEXT_RADIUS,
            storage_dir: None,
            cwd,
            counter: 0,
            auto_persist: false,
        }
    }

    /// Sets a custom storage directory for disk persistence.
    pub fn with_storage_dir(mut self, dir: PathBuf) -> Self {
        self.storage_dir = Some(dir);
        self.auto_persist = true;
        self
    }

    /// Sets the maximum checkpoint retention count.
    pub fn with_max_checkpoints(mut self, max: usize) -> Self {
        self.max_checkpoints = max.max(1);
        self
    }

    /// Sets the maximum memory footprint in bytes.
    pub fn with_max_memory_bytes(mut self, max_bytes: usize) -> Self {
        self.max_memory_bytes = max_bytes.max(1024 * 1024);
        self
    }

    /// Sets the default context radius for diffs.
    pub fn with_diff_context(mut self, radius: usize) -> Self {
        self.diff_context_radius = radius;
        self
    }

    /// Toggles auto-persistence on/off.
    pub fn with_auto_persist(mut self, auto: bool) -> Self {
        self.auto_persist = auto;
        self
    }

    // -----------------------------------------------------------------------
    // Transactional & Pre/Post Tool Execution
    // -----------------------------------------------------------------------

    /// Captures file snapshots before executing a tool call.
    /// Returns the assigned `checkpoint_id` if any files were targeted for snapshotting.
    pub fn capture_before_tool(
        &mut self,
        tool_name: &str,
        args: &serde_json::Value,
        cwd: &Path,
    ) -> anyhow::Result<Option<String>> {
        let targets = extract_target_paths(tool_name, args, cwd);
        if targets.is_empty() {
            return Ok(None);
        }

        self.counter += 1;
        let checkpoint_id = format!("chk_{}_{}", self.counter, Utc::now().timestamp_millis());
        let description = format!("Before tool '{}'", tool_name);

        let mut checkpoint = Checkpoint::new(&checkpoint_id, tool_name, description);
        checkpoint.tool_args = Some(args.clone());

        for target in targets {
            let orig_str = make_relative_or_string(&target, cwd);
            match FileSnapshot::capture_before(target.clone(), orig_str, cwd) {
                Ok(snapshot) => {
                    checkpoint.add_snapshot(snapshot);
                }
                Err(e) => {
                    tracing::warn!(
                        "Failed to capture pre-tool snapshot for '{}': {}",
                        target.display(),
                        e
                    );
                }
            }
        }

        if checkpoint.file_count() == 0 {
            return Ok(None);
        }

        self.push_checkpoint(checkpoint);
        self.redo_stack.clear();

        if self.auto_persist {
            let _ = self.persist_active_checkpoint(&checkpoint_id);
        }

        Ok(Some(checkpoint_id))
    }

    /// Captures the post-execution state of all files in the active checkpoint.
    pub fn capture_after_tool(
        &mut self,
        checkpoint_id: &str,
        cwd: &Path,
    ) -> anyhow::Result<()> {
        if let Some(checkpoint) = self.undo_stack.iter_mut().find(|c| c.id == checkpoint_id) {
            for snapshot in checkpoint.snapshots.values_mut() {
                let _ = snapshot.capture_after(cwd);
            }

            if self.auto_persist {
                let _ = self.persist_active_checkpoint(checkpoint_id);
            }
        }
        Ok(())
    }

    /// Begins an explicit transaction for grouped file mutations.
    pub fn begin_transaction(
        &mut self,
        tool_name: &str,
        description: &str,
        args: Option<serde_json::Value>,
    ) -> anyhow::Result<String> {
        self.counter += 1;
        let tx_id = format!("tx_{}_{}", self.counter, Utc::now().timestamp_millis());
        let mut tx = FileTransaction::new(&tx_id, tool_name, description);
        tx.tool_args = args;
        self.active_transaction = Some(tx);
        Ok(tx_id)
    }

    /// Records a pre-edit file snapshot inside the active transaction.
    pub fn record_pre_edit_for_transaction(
        &mut self,
        path: &Path,
        cwd: &Path,
    ) -> anyhow::Result<()> {
        let tx = self
            .active_transaction
            .as_mut()
            .ok_or_else(|| anyhow::anyhow!("No active transaction in progress"))?;
        let full_path = if path.is_absolute() {
            path.to_path_buf()
        } else {
            cwd.join(path)
        };
        let orig_str = make_relative_or_string(&full_path, cwd);
        tx.record_pre_edit(full_path, orig_str, cwd)?;
        Ok(())
    }

    /// Records a post-edit file snapshot inside the active transaction.
    pub fn record_post_edit_for_transaction(
        &mut self,
        path: &Path,
        cwd: &Path,
    ) -> anyhow::Result<()> {
        let tx = self
            .active_transaction
            .as_mut()
            .ok_or_else(|| anyhow::anyhow!("No active transaction in progress"))?;
        let full_path = if path.is_absolute() {
            path.to_path_buf()
        } else {
            cwd.join(path)
        };
        tx.record_post_edit(&full_path, cwd)?;
        Ok(())
    }

    /// Commits the active transaction into a permanent checkpoint on the undo stack.
    pub fn commit_transaction(&mut self, cwd: &Path) -> anyhow::Result<String> {
        let tx = self
            .active_transaction
            .take()
            .ok_or_else(|| anyhow::anyhow!("No active transaction in progress"))?;
        let id = tx.id.clone();
        let checkpoint = tx.commit(cwd);
        self.push_checkpoint(checkpoint);
        self.redo_stack.clear();

        if self.auto_persist {
            let _ = self.persist_active_checkpoint(&id);
        }

        Ok(id)
    }

    /// Rolls back the active transaction immediately.
    pub fn rollback_transaction(&mut self, cwd: &Path) -> anyhow::Result<UndoResult> {
        let mut tx = self
            .active_transaction
            .take()
            .ok_or_else(|| anyhow::anyhow!("No active transaction in progress"))?;
        let res = tx.rollback(cwd);
        Ok(res)
    }

    /// Aborts the active transaction without modifying files on disk.
    pub fn abort_transaction(&mut self) {
        if let Some(mut tx) = self.active_transaction.take() {
            tx.abort();
        }
    }

    /// Returns true if there is an active transaction in progress.
    pub fn has_active_transaction(&self) -> bool {
        self.active_transaction.is_some()
    }

    /// Returns reference to active transaction if present.
    pub fn active_transaction(&self) -> Option<&FileTransaction> {
        self.active_transaction.as_ref()
    }

    /// Manually creates a named checkpoint capturing a list of explicit paths.
    pub fn create_manual_checkpoint(
        &mut self,
        description: &str,
        paths: &[PathBuf],
        cwd: &Path,
    ) -> anyhow::Result<String> {
        self.counter += 1;
        let checkpoint_id =
            format!("chk_manual_{}_{}", self.counter, Utc::now().timestamp_millis());

        let mut checkpoint = Checkpoint::new(&checkpoint_id, "manual", description);

        for path in paths {
            let full_path = if path.is_absolute() {
                path.clone()
            } else {
                cwd.join(path)
            };
            let orig_str = make_relative_or_string(&full_path, cwd);
            let snapshot = FileSnapshot::capture_before(full_path, orig_str, cwd)?;
            checkpoint.add_snapshot(snapshot);
        }

        self.push_checkpoint(checkpoint);
        self.redo_stack.clear();

        if self.auto_persist {
            let _ = self.persist_active_checkpoint(&checkpoint_id);
        }

        Ok(checkpoint_id)
    }

    /// Pushes a checkpoint to the undo stack, enforcing `max_checkpoints` and `max_memory_bytes`.
    pub fn push_checkpoint(&mut self, checkpoint: Checkpoint) {
        if self.undo_stack.len() >= self.max_checkpoints {
            self.undo_stack.remove(0);
        }
        self.undo_stack.push(checkpoint);
        self.prune_memory_if_needed();
    }

    // -----------------------------------------------------------------------
    // Undo & Redo Stack Operations
    // -----------------------------------------------------------------------

    /// Reverts the most recent checkpoint on the undo stack.
    pub fn undo(&mut self, cwd: &Path) -> anyhow::Result<UndoResult> {
        self.undo_with_options(cwd, true)
    }

    /// Reverts the most recent checkpoint with optional forced conflict override.
    pub fn undo_with_options(&mut self, cwd: &Path, force: bool) -> anyhow::Result<UndoResult> {
        let mut checkpoint = self
            .undo_stack
            .pop()
            .ok_or_else(|| anyhow::anyhow!("No checkpoints available to undo."))?;

        let result = checkpoint.revert_with_safety(cwd, force);
        self.redo_stack.push(checkpoint);
        Ok(result)
    }

    /// Reverts the last `count` checkpoints in reverse chronological order.
    pub fn undo_n(&mut self, count: usize, cwd: &Path) -> anyhow::Result<Vec<UndoResult>> {
        let mut results = Vec::new();
        let target_count = count.min(self.undo_stack.len());

        for _ in 0..target_count {
            match self.undo(cwd) {
                Ok(res) => results.push(res),
                Err(e) => {
                    tracing::error!("Error during multi-step undo: {}", e);
                    break;
                }
            }
        }

        Ok(results)
    }

    /// Undoes checkpoints back to and including the specified `checkpoint_id`.
    pub fn undo_to(&mut self, checkpoint_id: &str, cwd: &Path) -> anyhow::Result<Vec<UndoResult>> {
        let idx = self
            .undo_stack
            .iter()
            .rposition(|c| c.id == checkpoint_id)
            .ok_or_else(|| anyhow::anyhow!("Checkpoint '{}' not found on undo stack", checkpoint_id))?;

        let count = self.undo_stack.len() - idx;
        self.undo_n(count, cwd)
    }

    /// Re-applies the most recent undone checkpoint from the redo stack.
    pub fn redo(&mut self, cwd: &Path) -> anyhow::Result<RedoResult> {
        let mut checkpoint = self
            .redo_stack
            .pop()
            .ok_or_else(|| anyhow::anyhow!("No undone checkpoints available to redo."))?;

        let result = checkpoint.redo(cwd);
        self.undo_stack.push(checkpoint);
        Ok(result)
    }

    /// Re-applies the last `count` undone checkpoints.
    pub fn redo_n(&mut self, count: usize, cwd: &Path) -> anyhow::Result<Vec<RedoResult>> {
        let mut results = Vec::new();
        let target_count = count.min(self.redo_stack.len());

        for _ in 0..target_count {
            match self.redo(cwd) {
                Ok(res) => results.push(res),
                Err(e) => {
                    tracing::error!("Error during multi-step redo: {}", e);
                    break;
                }
            }
        }

        Ok(results)
    }

    /// Redoes checkpoints up to and including the specified `checkpoint_id`.
    pub fn redo_to(&mut self, checkpoint_id: &str, cwd: &Path) -> anyhow::Result<Vec<RedoResult>> {
        let idx = self
            .redo_stack
            .iter()
            .rposition(|c| c.id == checkpoint_id)
            .ok_or_else(|| anyhow::anyhow!("Checkpoint '{}' not found on redo stack", checkpoint_id))?;

        let count = self.redo_stack.len() - idx;
        self.redo_n(count, cwd)
    }

    // -----------------------------------------------------------------------
    // Surgical Revert & Conflict Detection
    // -----------------------------------------------------------------------

    /// Reverts a single file within a specific checkpoint without rolling back other files.
    pub fn surgical_revert_file(
        &mut self,
        checkpoint_id: &str,
        file_path: &Path,
        force: bool,
        cwd: &Path,
    ) -> anyhow::Result<SurgicalRevertResult> {
        let checkpoint = self
            .undo_stack
            .iter_mut()
            .chain(self.redo_stack.iter_mut())
            .find(|c| c.id == checkpoint_id)
            .ok_or_else(|| anyhow::anyhow!("Checkpoint '{}' not found", checkpoint_id))?;

        let snapshot = checkpoint
            .get_snapshot(file_path)
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "File '{}' not found in checkpoint '{}'",
                    file_path.display(),
                    checkpoint_id
                )
            })?;

        let diff_text = snapshot.unified_diff(Some(cwd), self.diff_context_radius);
        let path_buf = snapshot.path.clone();

        match checkpoint.revert_single_file(file_path, cwd, force) {
            Ok(action) => Ok(SurgicalRevertResult {
                checkpoint_id: checkpoint_id.to_string(),
                path: path_buf,
                action,
                was_forced: force,
                diff_reverted: diff_text,
                success: true,
                error: None,
            }),
            Err(e) => Ok(SurgicalRevertResult {
                checkpoint_id: checkpoint_id.to_string(),
                path: path_buf,
                action: FileActionTaken::Skipped,
                was_forced: force,
                diff_reverted: diff_text,
                success: false,
                error: Some(e.to_string()),
            }),
        }
    }

    /// Checks whether reverting a file in a checkpoint is safe against current on-disk state.
    pub fn check_file_revert_safety(
        &self,
        checkpoint_id: &str,
        file_path: &Path,
        cwd: &Path,
    ) -> anyhow::Result<RevertSafety> {
        let checkpoint = self
            .get_checkpoint(checkpoint_id)
            .ok_or_else(|| anyhow::anyhow!("Checkpoint '{}' not found", checkpoint_id))?;

        let snapshot = checkpoint
            .get_snapshot(file_path)
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "File '{}' not found in checkpoint '{}'",
                    file_path.display(),
                    checkpoint_id
                )
            })?;

        Ok(snapshot.check_revert_safety(cwd))
    }

    // -----------------------------------------------------------------------
    // Inspection & Queries
    // -----------------------------------------------------------------------

    /// Returns a list of summaries for all active checkpoints on the undo stack.
    pub fn list_checkpoints(&self) -> Vec<CheckpointSummary> {
        self.undo_stack.iter().map(|c| c.summary()).collect()
    }

    /// Returns a reference to a specific checkpoint by ID if present.
    pub fn get_checkpoint(&self, id: &str) -> Option<&Checkpoint> {
        self.undo_stack
            .iter()
            .chain(self.redo_stack.iter())
            .find(|c| c.id == id)
    }

    /// Computes unified diffs for all files within a checkpoint.
    pub fn get_checkpoint_diff(&self, id: &str, cwd: &Path) -> anyhow::Result<Vec<FileDiff>> {
        let checkpoint = self
            .get_checkpoint(id)
            .ok_or_else(|| anyhow::anyhow!("Checkpoint '{}' not found", id))?;

        Ok(checkpoint.compute_diffs(Some(cwd), self.diff_context_radius))
    }

    /// Inspects a complete checkpoint including diffs, hunks, and aggregated statistics.
    pub fn inspect_checkpoint(
        &self,
        id: &str,
        cwd: &Path,
    ) -> anyhow::Result<CheckpointDiffInspection> {
        let checkpoint = self
            .get_checkpoint(id)
            .ok_or_else(|| anyhow::anyhow!("Checkpoint '{}' not found", id))?;

        let files = checkpoint.compute_diffs(Some(cwd), self.diff_context_radius);
        let total_stats = checkpoint.diff_stats(Some(cwd));

        Ok(CheckpointDiffInspection {
            checkpoint_id: checkpoint.id.clone(),
            tool_name: checkpoint.tool_name.clone(),
            description: checkpoint.description.clone(),
            created_at: checkpoint.created_at.clone(),
            files,
            total_stats,
        })
    }

    /// Inspects the diff for a single file in a checkpoint.
    pub fn inspect_file_diff(
        &self,
        id: &str,
        file_path: &Path,
        cwd: &Path,
    ) -> anyhow::Result<FileDiff> {
        let checkpoint = self
            .get_checkpoint(id)
            .ok_or_else(|| anyhow::anyhow!("Checkpoint '{}' not found", id))?;

        let snapshot = checkpoint
            .get_snapshot(file_path)
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "File '{}' not found in checkpoint '{}'",
                    file_path.display(),
                    id
                )
            })?;

        let change_type = snapshot.change_type(Some(cwd));
        let unified = snapshot.unified_diff(Some(cwd), self.diff_context_radius);
        let colorized = unified.as_ref().map(|u| colorize_diff_text(u));
        let hunks = unified
            .as_ref()
            .map(|u| parse_hunks_from_diff(u))
            .unwrap_or_default();
        let stats = snapshot.diff_stats(Some(cwd));
        let is_binary = snapshot.state_before.is_binary()
            || snapshot.state_after.as_ref().map_or(false, |s| s.is_binary());

        Ok(FileDiff {
            path: snapshot.path.clone(),
            change_type,
            unified_diff: unified,
            colorized_diff: colorized,
            hunks,
            stats,
            is_binary,
            bytes_before: snapshot.state_before.size(),
            bytes_after: snapshot.state_after.as_ref().map_or(0, |s| s.size()),
        })
    }

    /// Returns a full summary of history stacks and memory usage.
    pub fn history_summary(&self) -> CheckpointHistorySummary {
        CheckpointHistorySummary {
            undo_count: self.undo_stack.len(),
            redo_count: self.redo_stack.len(),
            active_checkpoints: self.undo_stack.iter().map(|c| c.summary()).collect(),
            undone_checkpoints: self.redo_stack.iter().map(|c| c.summary()).collect(),
            total_memory_bytes: self.total_memory_usage(),
        }
    }

    /// Returns true if there are checkpoints available to undo.
    pub fn can_undo(&self) -> bool {
        !self.undo_stack.is_empty()
    }

    /// Returns true if there are undone checkpoints available to redo.
    pub fn can_redo(&self) -> bool {
        !self.redo_stack.is_empty()
    }

    /// Number of checkpoints on the undo stack.
    pub fn undo_count(&self) -> usize {
        self.undo_stack.len()
    }

    /// Number of checkpoints on the redo stack.
    pub fn redo_count(&self) -> usize {
        self.redo_stack.len()
    }

    /// Returns reference to the top-most undo checkpoint without popping.
    pub fn peek_undo(&self) -> Option<&Checkpoint> {
        self.undo_stack.last()
    }

    /// Returns reference to the top-most redo checkpoint without popping.
    pub fn peek_redo(&self) -> Option<&Checkpoint> {
        self.redo_stack.last()
    }

    /// Clears all undo and redo history.
    pub fn clear(&mut self) {
        self.undo_stack.clear();
        self.redo_stack.clear();
        self.active_transaction = None;
    }

    /// Clears only redo history.
    pub fn clear_redo(&mut self) {
        self.redo_stack.clear();
    }

    // -----------------------------------------------------------------------
    // Memory Management & Persistence
    // -----------------------------------------------------------------------

    /// Calculates total approximate memory footprint of all retained snapshots.
    pub fn total_memory_usage(&self) -> usize {
        let undo_bytes: usize = self.undo_stack.iter().map(|c| c.memory_size()).sum();
        let redo_bytes: usize = self.redo_stack.iter().map(|c| c.memory_size()).sum();
        undo_bytes + redo_bytes
    }

    /// Prunes older checkpoints if total memory usage exceeds `max_memory_bytes`.
    pub fn prune_memory_if_needed(&mut self) {
        while self.total_memory_usage() > self.max_memory_bytes && self.undo_stack.len() > 1 {
            self.undo_stack.remove(0);
        }
    }

    /// Persists a single active checkpoint to disk.
    fn persist_active_checkpoint(&self, checkpoint_id: &str) -> anyhow::Result<()> {
        if let Some(dir) = &self.storage_dir {
            if !dir.exists() {
                fs::create_dir_all(dir)?;
            }
            if let Some(checkpoint) = self.get_checkpoint(checkpoint_id) {
                let json = serde_json::to_string_pretty(checkpoint)?;
                let file_path = dir.join(format!("{}.json", checkpoint_id));
                fs::write(file_path, json)?;
            }
        }
        Ok(())
    }

    /// Saves all checkpoints to a specified directory.
    pub fn save_to_disk(&self, dir: &Path) -> anyhow::Result<()> {
        if !dir.exists() {
            fs::create_dir_all(dir)?;
        }
        for checkpoint in &self.undo_stack {
            let json = serde_json::to_string_pretty(checkpoint)?;
            let file_path = dir.join(format!("{}.json", checkpoint.id));
            fs::write(file_path, json)?;
        }
        Ok(())
    }

    /// Loads checkpoints from a directory into this manager.
    pub fn load_from_disk(&mut self, dir: &Path) -> anyhow::Result<usize> {
        if !dir.exists() {
            return Ok(0);
        }
        let mut loaded = 0;
        let mut entries: Vec<PathBuf> = fs::read_dir(dir)?
            .filter_map(|e| e.ok().map(|ent| ent.path()))
            .filter(|p| p.extension().and_then(|ext| ext.to_str()) == Some("json"))
            .collect();
        entries.sort();

        for path in entries {
            if let Ok(content) = fs::read_to_string(&path) {
                if let Ok(checkpoint) = serde_json::from_str::<Checkpoint>(&content) {
                    self.push_checkpoint(checkpoint);
                    loaded += 1;
                }
            }
        }
        Ok(loaded)
    }
}

// ---------------------------------------------------------------------------
// Thread-safe Shared CheckpointManager
// ---------------------------------------------------------------------------

/// Thread-safe reference-counted `CheckpointManager`.
pub type SharedCheckpointManager = Arc<Mutex<CheckpointManager>>;

/// Creates a new `SharedCheckpointManager`.
pub fn new_shared_checkpoint_manager(cwd: PathBuf) -> SharedCheckpointManager {
    Arc::new(Mutex::new(CheckpointManager::new(cwd)))
}

// ---------------------------------------------------------------------------
// CLI & Terminal Formatting Helpers
// ---------------------------------------------------------------------------

/// Formats a rich, colorized terminal report for an undo operation.
pub fn format_undo_report(result: &UndoResult) -> String {
    let mut out = String::new();

    if result.success {
        out.push_str(&format!(
            "\x1b[1;32m✓\x1b[0m \x1b[1;37mCheckpoint Reverted:\x1b[0m \x1b[1;36m{}\x1b[0m (\x1b[2;37m{}\x1b[0m)\n",
            result.tool_name, result.checkpoint_id
        ));
    } else {
        out.push_str(&format!(
            "\x1b[1;31m✕\x1b[0m \x1b[1;37mCheckpoint Reversion Encountered Issues:\x1b[0m \x1b[1;36m{}\x1b[0m\n",
            result.tool_name
        ));
    }

    for path in &result.restored_files {
        out.push_str(&format!(
            "  \x1b[1;32m⟲ Restored:\x1b[0m {}\n",
            path.display()
        ));
    }

    for path in &result.deleted_files {
        out.push_str(&format!(
            "  \x1b[1;31m✕ Deleted (newly created):\x1b[0m {}\n",
            path.display()
        ));
    }

    for path in &result.recreated_dirs {
        out.push_str(&format!(
            "  \x1b[1;34m📁 Directory recreated:\x1b[0m {}\n",
            path.display()
        ));
    }

    for (path, conflict) in &result.conflicts {
        out.push_str(&format!(
            "  \x1b[1;33m⚠ Conflict on {}:\x1b[0m {}\n",
            path.display(),
            conflict
        ));
    }

    for (path, err) in &result.errors {
        out.push_str(&format!(
            "  \x1b[1;31m⚠ Error on {}:\x1b[0m {}\n",
            path.display(),
            err
        ));
    }

    if result.total_reverted() == 0 && result.errors.is_empty() && result.conflicts.is_empty() {
        out.push_str("  \x1b[2;37m(Files were already in pre-tool state)\x1b[0m\n");
    }

    out
}

/// Formats a rich, colorized terminal report for a redo operation.
pub fn format_redo_report(result: &RedoResult) -> String {
    let mut out = String::new();

    if result.success {
        out.push_str(&format!(
            "\x1b[1;32m✓\x1b[0m \x1b[1;37mCheckpoint Re-applied:\x1b[0m \x1b[1;36m{}\x1b[0m (\x1b[2;37m{}\x1b[0m)\n",
            result.tool_name, result.checkpoint_id
        ));
    } else {
        out.push_str(&format!(
            "\x1b[1;31m✕\x1b[0m \x1b[1;37mCheckpoint Redo Encountered Errors:\x1b[0m \x1b[1;36m{}\x1b[0m\n",
            result.tool_name
        ));
    }

    for path in &result.reapplied_files {
        out.push_str(&format!(
            "  \x1b[1;32m⟳ Re-applied:\x1b[0m {}\n",
            path.display()
        ));
    }

    for path in &result.recreated_files {
        out.push_str(&format!(
            "  \x1b[1;34m📁 Re-created:\x1b[0m {}\n",
            path.display()
        ));
    }

    for path in &result.deleted_files {
        out.push_str(&format!(
            "  \x1b[1;31m✕ Deleted:\x1b[0m {}\n",
            path.display()
        ));
    }

    for (path, err) in &result.errors {
        out.push_str(&format!(
            "  \x1b[1;31m⚠ Error on {}:\x1b[0m {}\n",
            path.display(),
            err
        ));
    }

    out
}

/// Formats a terminal report for a surgical single-file revert.
pub fn format_surgical_revert_report(result: &SurgicalRevertResult) -> String {
    let mut out = String::new();
    if result.success {
        out.push_str(&format!(
            "\x1b[1;32m✓\x1b[0m \x1b[1;37mSurgically Reverted:\x1b[0m \x1b[1;36m{}\x1b[0m (from {})\n",
            result.path.display(),
            result.checkpoint_id
        ));
    } else {
        out.push_str(&format!(
            "\x1b[1;31m✕\x1b[0m \x1b[1;37mSurgical Revert Failed for:\x1b[0m \x1b[1;36m{}\x1b[0m: {}\n",
            result.path.display(),
            result.error.as_deref().unwrap_or("unknown error")
        ));
    }
    if let Some(diff) = &result.diff_reverted {
        out.push_str("\n\x1b[2mReverted Diff:\x1b[0m\n");
        out.push_str(&colorize_diff_text(diff));
    }
    out
}

/// Formats a table of checkpoints for listing in CLI or `/checkpoints`.
pub fn format_checkpoints_table(checkpoints: &[CheckpointSummary]) -> String {
    if checkpoints.is_empty() {
        return "\x1b[2;37mNo checkpoints recorded in this session.\x1b[0m\n".to_string();
    }

    let mut out = String::new();
    out.push_str(&format!(
        "\x1b[1;36mRecorded File Mutation Checkpoints:\x1b[0m ({} total)\n",
        checkpoints.len()
    ));
    out.push_str(&format!(
        "  {:<24} {:<12} {:<6} {:<12} {:<10} {}\n",
        "ID", "TOOL", "FILES", "CHANGES", "STATUS", "AFFECTED PATHS"
    ));
    out.push_str(&format!("  {}\n", "─".repeat(80)));

    for chk in checkpoints.iter().rev() {
        let status_str = match chk.status {
            CheckpointStatus::Active => "\x1b[32mActive\x1b[0m",
            CheckpointStatus::Undone => "\x1b[33mUndone\x1b[0m",
            CheckpointStatus::Redone => "\x1b[36mRedone\x1b[0m",
        };
        let stats_preview = chk
            .stats
            .as_ref()
            .map(|s| format!("+{} -{}", s.insertions, s.deletions))
            .unwrap_or_else(|| "-".to_string());

        let paths_preview = if chk.files.is_empty() {
            "-".to_string()
        } else {
            chk.files
                .iter()
                .map(|p| p.display().to_string())
                .collect::<Vec<_>>()
                .join(", ")
        };

        out.push_str(&format!(
            "  {:<24} {:<12} {:<6} {:<12} {:<19} {}\n",
            chk.id, chk.tool_name, chk.file_count, stats_preview, status_str, paths_preview
        ));
    }

    out
}

/// Formats a complete diff inspection for terminal display.
pub fn format_diff_inspection(inspection: &CheckpointDiffInspection) -> String {
    let mut out = String::new();
    out.push_str(&format!(
        "\x1b[1;36mCheckpoint Inspection:\x1b[0m \x1b[1;37m{}\x1b[0m ({})\n",
        inspection.checkpoint_id, inspection.tool_name
    ));
    out.push_str(&format!(
        "  \x1b[2;37mCreated:\x1b[0m {} | \x1b[2;37mDescription:\x1b[0m {}\n",
        inspection.created_at, inspection.description
    ));
    out.push_str(&format!(
        "  \x1b[1;37mTotal Changes:\x1b[0m \x1b[32m+{}\x1b[0m \x1b[31m-{}\x1b[0m ({} files)\n\n",
        inspection.total_stats.insertions,
        inspection.total_stats.deletions,
        inspection.total_stats.files_changed
    ));

    for file_diff in &inspection.files {
        out.push_str(&format!(
            "{} \x1b[1;37m{}\x1b[0m (+{} -{})\n",
            file_diff.change_type.badge(),
            file_diff.path.display(),
            file_diff.stats.insertions,
            file_diff.stats.deletions
        ));

        if let Some(colorized) = &file_diff.colorized_diff {
            out.push_str(colorized);
            out.push('\n');
        } else if file_diff.is_binary {
            out.push_str("  \x1b[2;37m(Binary file changed)\x1b[0m\n\n");
        }
    }

    out
}

// ---------------------------------------------------------------------------
// Internal Helpers
// ---------------------------------------------------------------------------

/// Computes a fast deterministic hash of raw byte slice.
fn compute_content_hash(bytes: &[u8]) -> String {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    bytes.hash(&mut hasher);
    format!("{:016x}", hasher.finish())
}

/// Checks if byte slice appears to contain binary data (null bytes in first 8KB).
fn is_binary_content(bytes: &[u8]) -> bool {
    bytes.iter().take(8192).any(|&b| b == 0)
}

/// Counts total number of lines in byte slice.
fn count_lines(bytes: &[u8]) -> usize {
    if bytes.is_empty() {
        return 0;
    }
    let mut count = bytes.iter().filter(|&&b| b == b'\n').count();
    if bytes.last() != Some(&b'\n') {
        count += 1;
    }
    count
}

/// Retrieves POSIX file permissions if on Unix.
fn get_permissions(metadata: &fs::Metadata) -> Option<u32> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        Some(metadata.permissions().mode())
    }
    #[cfg(not(unix))]
    {
        let _ = metadata;
        None
    }
}

/// Converts a path to a clean relative string against `cwd` if possible.
fn make_relative_or_string(path: &Path, cwd: &Path) -> String {
    if let Ok(rel) = path.strip_prefix(cwd) {
        rel.to_string_lossy().to_string()
    } else {
        path.to_string_lossy().to_string()
    }
}

/// Converts unified diff text into ANSI colorized output.
fn colorize_diff_text(diff_text: &str) -> String {
    let mut out = String::new();
    for line in diff_text.lines() {
        if line.starts_with("+++") || line.starts_with("---") {
            out.push_str(&format!("\x1b[1m{}\x1b[0m\n", line));
        } else if line.starts_with('+') {
            out.push_str(&format!("\x1b[32m{}\x1b[0m\n", line));
        } else if line.starts_with('-') {
            out.push_str(&format!("\x1b[31m{}\x1b[0m\n", line));
        } else if line.starts_with("@@") {
            out.push_str(&format!("\x1b[36m{}\x1b[0m\n", line));
        } else {
            out.push_str(&format!("{}\n", line));
        }
    }
    out
}

/// Parses structured hunks and lines from a unified diff text string.
pub fn parse_hunks_from_diff(diff_text: &str) -> Vec<DiffHunk> {
    let mut hunks = Vec::new();
    let mut current_hunk: Option<DiffHunk> = None;
    let mut old_line_cur = 0;
    let mut new_line_cur = 0;

    for line in diff_text.lines() {
        if line.starts_with("@@") {
            if let Some(hunk) = current_hunk.take() {
                hunks.push(hunk);
            }
            let (old_start, old_len, new_start, new_len) = parse_hunk_header(line);
            old_line_cur = old_start;
            new_line_cur = new_start;
            current_hunk = Some(DiffHunk {
                header: line.to_string(),
                old_start,
                old_lines: old_len,
                new_start,
                new_lines: new_len,
                lines: Vec::new(),
            });
        } else if let Some(hunk) = &mut current_hunk {
            if let Some(content) = line.strip_prefix('+') {
                hunk.lines.push(HunkLine {
                    kind: HunkLineKind::Addition,
                    content: content.to_string(),
                    old_lineno: None,
                    new_lineno: Some(new_line_cur),
                });
                new_line_cur += 1;
            } else if let Some(content) = line.strip_prefix('-') {
                hunk.lines.push(HunkLine {
                    kind: HunkLineKind::Deletion,
                    content: content.to_string(),
                    old_lineno: Some(old_line_cur),
                    new_lineno: None,
                });
                old_line_cur += 1;
            } else if let Some(content) = line.strip_prefix(' ') {
                hunk.lines.push(HunkLine {
                    kind: HunkLineKind::Context,
                    content: content.to_string(),
                    old_lineno: Some(old_line_cur),
                    new_lineno: Some(new_line_cur),
                });
                old_line_cur += 1;
                new_line_cur += 1;
            }
        }
    }

    if let Some(hunk) = current_hunk {
        hunks.push(hunk);
    }

    hunks
}

/// Parses the numeric start and line count from a `@@ -l,s +l,s @@` hunk header.
fn parse_hunk_header(header: &str) -> (usize, usize, usize, usize) {
    let parts: Vec<&str> = header.split("@@").collect();
    if parts.len() < 2 {
        return (1, 0, 1, 0);
    }
    let middle = parts[1].trim();
    let sections: Vec<&str> = middle.split_whitespace().collect();
    let mut old_start = 1;
    let mut old_len = 1;
    let mut new_start = 1;
    let mut new_len = 1;

    for sec in sections {
        if let Some(stripped) = sec.strip_prefix('-') {
            let nums: Vec<&str> = stripped.split(',').collect();
            if let Some(n) = nums.first().and_then(|s| s.parse::<usize>().ok()) {
                old_start = n;
            }
            if nums.len() > 1 {
                if let Some(n) = nums.get(1).and_then(|s| s.parse::<usize>().ok()) {
                    old_len = n;
                }
            } else {
                old_len = 1;
            }
        } else if let Some(stripped) = sec.strip_prefix('+') {
            let nums: Vec<&str> = stripped.split(',').collect();
            if let Some(n) = nums.first().and_then(|s| s.parse::<usize>().ok()) {
                new_start = n;
            }
            if nums.len() > 1 {
                if let Some(n) = nums.get(1).and_then(|s| s.parse::<usize>().ok()) {
                    new_len = n;
                }
            } else {
                new_len = 1;
            }
        }
    }

    (old_start, old_len, new_start, new_len)
}

// ---------------------------------------------------------------------------
// Unit Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    static TEST_COUNTER: AtomicUsize = AtomicUsize::new(0);

    fn make_temp_dir() -> PathBuf {
        let count = TEST_COUNTER.fetch_add(1, Ordering::SeqCst);
        let dir = std::env::temp_dir().join(format!(
            "fusion_undo_test_{}_{}_{}",
            std::process::id(),
            Utc::now().timestamp_nanos_opt().unwrap_or(0),
            count
        ));
        fs::create_dir_all(&dir).expect("failed to create temp dir");
        dir
    }

    #[test]
    fn test_file_state_absent_and_present() {
        let dir = make_temp_dir();
        let file_path = dir.join("test.txt");

        // Before creation -> Absent
        let state_before = FileState::from_path(&file_path).unwrap();
        assert!(state_before.is_absent());
        assert_eq!(state_before.size(), 0);
        assert_eq!(state_before.line_count(), 0);

        // Create file
        fs::write(&file_path, "Hello, Checkpoint Undo!\nSecond line").unwrap();

        // After creation -> Present
        let state_after = FileState::from_path(&file_path).unwrap();
        assert!(state_after.is_present());
        assert_eq!(
            state_after.content_as_str(),
            Some("Hello, Checkpoint Undo!\nSecond line")
        );
        assert_eq!(state_after.size(), 35);
        assert_eq!(state_after.line_count(), 2);

        // Test FileChangeType detection
        assert_eq!(
            FileChangeType::detect(&state_before, &state_after),
            FileChangeType::Created
        );
        assert_eq!(
            FileChangeType::detect(&state_after, &state_before),
            FileChangeType::Deleted
        );

        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn test_extract_target_paths() {
        let cwd = PathBuf::from("/workspace/project");

        // write tool
        let args = serde_json::json!({ "path": "src/main.rs", "content": "fn main() {}" });
        let paths = extract_target_paths("write", &args, &cwd);
        assert_eq!(paths, vec![cwd.join("src/main.rs")]);

        // edit tool
        let args = serde_json::json!({ "file_path": "lib/utils.rs", "old_text": "a", "new_text": "b" });
        let paths = extract_target_paths("edit", &args, &cwd);
        assert_eq!(paths, vec![cwd.join("lib/utils.rs")]);

        // patch tool with patch content
        let patch_content = "--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1 +1 @@\n-old\n+new\n";
        let args = serde_json::json!({ "patch": patch_content });
        let paths = extract_target_paths("patch", &args, &cwd);
        assert_eq!(paths, vec![cwd.join("src/lib.rs")]);

        // bash redirection
        let args = serde_json::json!({ "command": "echo 'hello' > output.log" });
        let paths = extract_target_paths("bash", &args, &cwd);
        assert_eq!(paths, vec![cwd.join("output.log")]);
    }

    #[test]
    fn test_checkpoint_undo_modified_file() {
        let dir = make_temp_dir();
        let file_path = dir.join("code.rs");
        fs::write(&file_path, "original version").unwrap();

        let mut mgr = CheckpointManager::new(dir.clone());

        // Capture before tool modification
        let args = serde_json::json!({ "path": "code.rs", "content": "modified version" });
        let chk_id = mgr
            .capture_before_tool("write", &args, &dir)
            .unwrap()
            .unwrap();

        // Simulate tool execution writing to file
        fs::write(&file_path, "modified version").unwrap();
        mgr.capture_after_tool(&chk_id, &dir).unwrap();

        assert_eq!(fs::read_to_string(&file_path).unwrap(), "modified version");
        assert_eq!(mgr.undo_count(), 1);
        assert!(mgr.can_undo());

        // Perform undo!
        let undo_res = mgr.undo(&dir).unwrap();
        assert!(undo_res.success);
        assert_eq!(undo_res.restored_files, vec![file_path.clone()]);
        assert_eq!(fs::read_to_string(&file_path).unwrap(), "original version");
        assert_eq!(mgr.undo_count(), 0);
        assert_eq!(mgr.redo_count(), 1);
        assert!(mgr.can_redo());

        // Perform redo!
        let redo_res = mgr.redo(&dir).unwrap();
        assert!(redo_res.success);
        assert_eq!(redo_res.reapplied_files, vec![file_path.clone()]);
        assert_eq!(fs::read_to_string(&file_path).unwrap(), "modified version");

        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn test_checkpoint_undo_newly_created_file() {
        let dir = make_temp_dir();
        let new_file = dir.join("brand_new.txt");
        assert!(!new_file.exists());

        let mut mgr = CheckpointManager::new(dir.clone());

        // Capture before tool creates file
        let args = serde_json::json!({ "path": "brand_new.txt", "content": "created by AI" });
        let chk_id = mgr
            .capture_before_tool("write", &args, &dir)
            .unwrap()
            .unwrap();

        // Simulate tool creating the file
        fs::write(&new_file, "created by AI").unwrap();
        mgr.capture_after_tool(&chk_id, &dir).unwrap();
        assert!(new_file.exists());

        // Undo -> should delete the newly created file
        let undo_res = mgr.undo(&dir).unwrap();
        assert!(undo_res.success);
        assert_eq!(undo_res.deleted_files, vec![new_file.clone()]);
        assert!(!new_file.exists());

        // Redo -> should recreate the file
        let redo_res = mgr.redo(&dir).unwrap();
        assert!(redo_res.success);
        assert!(new_file.exists());
        assert_eq!(fs::read_to_string(&new_file).unwrap(), "created by AI");

        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn test_multi_step_undo_and_undo_to() {
        let dir = make_temp_dir();
        let file_path = dir.join("steps.txt");
        fs::write(&file_path, "step 0").unwrap();

        let mut mgr = CheckpointManager::new(dir.clone());

        // Step 1
        let args1 = serde_json::json!({ "path": "steps.txt", "content": "step 1" });
        let id1 = mgr
            .capture_before_tool("write", &args1, &dir)
            .unwrap()
            .unwrap();
        fs::write(&file_path, "step 1").unwrap();
        mgr.capture_after_tool(&id1, &dir).unwrap();

        // Step 2
        let args2 = serde_json::json!({ "path": "steps.txt", "content": "step 2" });
        let id2 = mgr
            .capture_before_tool("write", &args2, &dir)
            .unwrap()
            .unwrap();
        fs::write(&file_path, "step 2").unwrap();
        mgr.capture_after_tool(&id2, &dir).unwrap();

        // Step 3
        let args3 = serde_json::json!({ "path": "steps.txt", "content": "step 3" });
        let id3 = mgr
            .capture_before_tool("write", &args3, &dir)
            .unwrap()
            .unwrap();
        fs::write(&file_path, "step 3").unwrap();
        mgr.capture_after_tool(&id3, &dir).unwrap();

        assert_eq!(mgr.undo_count(), 3);
        assert_eq!(fs::read_to_string(&file_path).unwrap(), "step 3");

        // Undo to step 2 (should undo step 3 and step 2)
        let results = mgr.undo_to(&id2, &dir).unwrap();
        assert_eq!(results.len(), 2);
        assert_eq!(fs::read_to_string(&file_path).unwrap(), "step 1");
        assert_eq!(mgr.undo_count(), 1);
        assert_eq!(mgr.redo_count(), 2);

        // Redo to step 3
        let redo_results = mgr.redo_to(&id3, &dir).unwrap();
        assert_eq!(redo_results.len(), 2);
        assert_eq!(fs::read_to_string(&file_path).unwrap(), "step 3");
        assert_eq!(mgr.undo_count(), 3);
        assert_eq!(mgr.redo_count(), 0);

        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn test_transactional_lifecycle() {
        let dir = make_temp_dir();
        let file1 = dir.join("t1.txt");
        let file2 = dir.join("t2.txt");
        fs::write(&file1, "tx original 1").unwrap();
        fs::write(&file2, "tx original 2").unwrap();

        let mut mgr = CheckpointManager::new(dir.clone());

        // 1. Rollback a transaction
        let _tx_id = mgr
            .begin_transaction("batch_edit", "Refactor two files", None)
            .unwrap();
        assert!(mgr.has_active_transaction());

        mgr.record_pre_edit_for_transaction(&file1, &dir).unwrap();
        mgr.record_pre_edit_for_transaction(&file2, &dir).unwrap();

        fs::write(&file1, "tx modified 1").unwrap();
        fs::write(&file2, "tx modified 2").unwrap();

        let rollback_res = mgr.rollback_transaction(&dir).unwrap();
        assert!(rollback_res.success);
        assert_eq!(fs::read_to_string(&file1).unwrap(), "tx original 1");
        assert_eq!(fs::read_to_string(&file2).unwrap(), "tx original 2");
        assert!(!mgr.has_active_transaction());

        // 2. Commit a transaction
        let _tx_id2 = mgr
            .begin_transaction("batch_edit_2", "Apply final changes", None)
            .unwrap();
        mgr.record_pre_edit_for_transaction(&file1, &dir).unwrap();
        mgr.record_pre_edit_for_transaction(&file2, &dir).unwrap();

        fs::write(&file1, "tx committed 1").unwrap();
        fs::write(&file2, "tx committed 2").unwrap();
        mgr.record_post_edit_for_transaction(&file1, &dir).unwrap();
        mgr.record_post_edit_for_transaction(&file2, &dir).unwrap();

        let chk_id = mgr.commit_transaction(&dir).unwrap();
        assert_eq!(mgr.undo_count(), 1);

        // Check inspection
        let inspection = mgr.inspect_checkpoint(&chk_id, &dir).unwrap();
        assert_eq!(inspection.files.len(), 2);
        assert_eq!(inspection.total_stats.files_changed, 2);

        // Undo committed transaction
        let undo_res = mgr.undo(&dir).unwrap();
        assert!(undo_res.success);
        assert_eq!(fs::read_to_string(&file1).unwrap(), "tx original 1");
        assert_eq!(fs::read_to_string(&file2).unwrap(), "tx original 2");

        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn test_surgical_file_revert() {
        let dir = make_temp_dir();
        let file1 = dir.join("f1.txt");
        let file2 = dir.join("f2.txt");
        fs::write(&file1, "f1 orig").unwrap();
        fs::write(&file2, "f2 orig").unwrap();

        let mut mgr = CheckpointManager::new(dir.clone());
        let _tx = mgr.begin_transaction("multi", "Two files", None).unwrap();
        mgr.record_pre_edit_for_transaction(&file1, &dir).unwrap();
        mgr.record_pre_edit_for_transaction(&file2, &dir).unwrap();

        fs::write(&file1, "f1 mod").unwrap();
        fs::write(&file2, "f2 mod").unwrap();
        let chk_id = mgr.commit_transaction(&dir).unwrap();

        // Surgically revert only file 1
        let revert_res = mgr
            .surgical_revert_file(&chk_id, &file1, true, &dir)
            .unwrap();
        assert!(revert_res.success);
        assert_eq!(fs::read_to_string(&file1).unwrap(), "f1 orig");
        assert_eq!(fs::read_to_string(&file2).unwrap(), "f2 mod");

        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn test_conflict_detection_and_safety_check() {
        let dir = make_temp_dir();
        let file_path = dir.join("conflict.txt");
        fs::write(&file_path, "version 1").unwrap();

        let mut mgr = CheckpointManager::new(dir.clone());
        let args = serde_json::json!({ "path": "conflict.txt", "content": "version 2" });
        let chk_id = mgr
            .capture_before_tool("write", &args, &dir)
            .unwrap()
            .unwrap();

        fs::write(&file_path, "version 2").unwrap();
        mgr.capture_after_tool(&chk_id, &dir).unwrap();

        // Now simulate an external user edit outside of the agent: version 3
        fs::write(&file_path, "version 3 - external edit").unwrap();

        // Safety check should report Conflicted
        let safety = mgr
            .check_file_revert_safety(&chk_id, &file_path, &dir)
            .unwrap();
        assert!(matches!(safety, RevertSafety::Conflicted { .. }));

        // Non-forced undo should record a conflict
        let undo_res = mgr.undo_with_options(&dir, false).unwrap();
        assert!(!undo_res.success);
        assert!(undo_res.has_conflicts());
        assert_eq!(
            fs::read_to_string(&file_path).unwrap(),
            "version 3 - external edit"
        );

        // Forced undo should overwrite the conflict
        let redo = mgr.redo(&dir).unwrap();
        assert!(redo.success);
        let forced_undo = mgr.undo_with_options(&dir, true).unwrap();
        assert!(forced_undo.success);
        assert_eq!(fs::read_to_string(&file_path).unwrap(), "version 1");

        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn test_checkpoint_diff_hunks_and_formatting() {
        let dir = make_temp_dir();
        let file_path = dir.join("diff_test.rs");
        fs::write(&file_path, "fn hello() {\n    println!(\"old\");\n}\n").unwrap();

        let mut mgr = CheckpointManager::new(dir.clone());

        let args = serde_json::json!({ "path": "diff_test.rs", "old_text": "old", "new_text": "new" });
        let id = mgr.capture_before_tool("edit", &args, &dir).unwrap().unwrap();

        fs::write(&file_path, "fn hello() {\n    println!(\"new\");\n}\n").unwrap();
        mgr.capture_after_tool(&id, &dir).unwrap();

        let diffs = mgr.get_checkpoint_diff(&id, &dir).unwrap();
        assert_eq!(diffs.len(), 1);
        assert_eq!(diffs[0].change_type, FileChangeType::Modified);
        assert!(diffs[0].unified_diff.is_some());
        assert!(!diffs[0].hunks.is_empty());
        assert_eq!(diffs[0].stats.insertions, 1);
        assert_eq!(diffs[0].stats.deletions, 1);

        let list = mgr.list_checkpoints();
        assert_eq!(list.len(), 1);
        let table = format_checkpoints_table(&list);
        assert!(table.contains("edit"));
        assert!(table.contains("diff_test.rs"));

        let inspection = mgr.inspect_checkpoint(&id, &dir).unwrap();
        let report = format_diff_inspection(&inspection);
        assert!(report.contains("Checkpoint Inspection"));
        assert!(report.contains("diff_test.rs"));

        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn test_manual_checkpoint_and_disk_persistence() {
        let dir = make_temp_dir();
        let save_dir = dir.join(".checkpoints");
        let file1 = dir.join("f1.txt");
        let file2 = dir.join("f2.txt");
        fs::write(&file1, "file 1 initial").unwrap();
        fs::write(&file2, "file 2 initial").unwrap();

        let mut mgr = CheckpointManager::new(dir.clone()).with_storage_dir(save_dir.clone());

        let _manual_id = mgr
            .create_manual_checkpoint(
                "Before refactor",
                &[PathBuf::from("f1.txt"), PathBuf::from("f2.txt")],
                &dir,
            )
            .unwrap();

        fs::write(&file1, "file 1 modified").unwrap();
        fs::write(&file2, "file 2 modified").unwrap();

        // Load into new manager
        let mut loaded_mgr = CheckpointManager::new(dir.clone());
        let count = loaded_mgr.load_from_disk(&save_dir).unwrap();
        assert_eq!(count, 1);
        assert_eq!(loaded_mgr.undo_count(), 1);

        let undo_res = loaded_mgr.undo(&dir).unwrap();
        assert!(undo_res.success);
        assert_eq!(fs::read_to_string(&file1).unwrap(), "file 1 initial");
        assert_eq!(fs::read_to_string(&file2).unwrap(), "file 2 initial");

        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn test_memory_pruning_and_fifo_eviction() {
        let dir = make_temp_dir();
        let file_path = dir.join("bounded.txt");
        fs::write(&file_path, "initial").unwrap();

        // Limit to max 3 checkpoints
        let mut mgr = CheckpointManager::new(dir.clone()).with_max_checkpoints(3);

        for i in 1..=5 {
            let args = serde_json::json!({ "path": "bounded.txt", "content": format!("v{}", i) });
            let id = mgr.capture_before_tool("write", &args, &dir).unwrap().unwrap();
            fs::write(&file_path, format!("v{}", i)).unwrap();
            mgr.capture_after_tool(&id, &dir).unwrap();
        }

        assert_eq!(mgr.undo_count(), 3);
        let list = mgr.list_checkpoints();
        assert_eq!(list.len(), 3);

        let _ = fs::remove_dir_all(dir);
    }
}
