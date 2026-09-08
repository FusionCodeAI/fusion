//! Subagent isolated workspace management using git worktrees.
//!
//! Provides [`IsolatedWorkspace`], enabling subagents to perform isolated edits
//! in private git worktrees under `.fusion/worktrees/<task_id>` and compute unified
//! diffs against the base repository.

use std::fmt;
use std::path::{Path, PathBuf};

pub use fusion_iso::{IsoError, WorktreeManager};

/// Represents an isolated git worktree workspace created for a subagent task.
pub struct IsolatedWorkspace {
    /// Unique identifier for the task owning this workspace.
    pub task_id: String,
    /// Absolute or relative path to the isolated worktree directory.
    pub path: PathBuf,
    /// Base repository path.
    pub base_path: PathBuf,
    /// Git branch dedicated to this isolated workspace.
    pub branch: String,
    /// Underlying worktree manager.
    pub manager: WorktreeManager,
    /// Internal flag tracking whether cleanup has already executed.
    cleaned_up: bool,
}

impl fmt::Debug for IsolatedWorkspace {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("IsolatedWorkspace")
            .field("task_id", &self.task_id)
            .field("path", &self.path)
            .field("base_path", &self.base_path)
            .field("branch", &self.branch)
            .field("cleaned_up", &self.cleaned_up)
            .finish()
    }
}

impl IsolatedWorkspace {
    /// Creates an isolated git worktree in `.fusion/worktrees/<task_id>`.
    ///
    /// The worktree is created from `HEAD` on a new branch named `fusion-task-<task_id>`.
    /// If a previous run left the branch or path intact, `-B` safely reinitializes it.
    pub fn create_for_task(task_id: &str, base_path: &Path) -> Result<Self, IsoError> {
        let base_path =
            std::fs::canonicalize(base_path).unwrap_or_else(|_| base_path.to_path_buf());
        let manager = WorktreeManager::new(&base_path);

        let worktree_path = manager.worktree_path_for_task(task_id);
        let safe_branch_suffix = sanitize_branch_name(task_id);
        let branch = format!("fusion-task-{safe_branch_suffix}");

        manager.create_worktree(&worktree_path, &branch)?;

        Ok(Self {
            task_id: task_id.to_string(),
            path: worktree_path,
            base_path,
            branch,
            manager,
            cleaned_up: false,
        })
    }

    /// Computes the unified diff of modifications made within this isolated workspace.
    pub fn collect_diff(&self) -> Result<String, IsoError> {
        self.manager.collect_diff(&self.path)
    }

    /// Removes the worktree and cleans up the branch.
    pub fn cleanup(mut self) -> Result<(), IsoError> {
        self.perform_cleanup()
    }

    /// Internal cleanup implementation called by both `cleanup()` and `drop()`.
    fn perform_cleanup(&mut self) -> Result<(), IsoError> {
        if self.cleaned_up {
            return Ok(());
        }
        self.cleaned_up = true;

        let mut errors = Vec::new();
        if let Err(e) = self.manager.remove_worktree(&self.path) {
            errors.push(format!("remove worktree: {e}"));
        }
        if let Err(e) = self.manager.delete_branch(&self.branch) {
            errors.push(format!("delete branch: {e}"));
        }

        if errors.is_empty() {
            Ok(())
        } else {
            Err(IsoError::other(errors.join("; ")))
        }
    }

    /// Accessor for task ID.
    pub fn task_id(&self) -> &str {
        &self.task_id
    }

    /// Accessor for worktree directory path.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Accessor for base repository path.
    pub fn base_path(&self) -> &Path {
        &self.base_path
    }

    /// Accessor for branch name.
    pub fn branch(&self) -> &str {
        &self.branch
    }

    /// Accessor for the underlying worktree manager.
    pub fn manager(&self) -> &WorktreeManager {
        &self.manager
    }

    /// Checks whether cleanup has already occurred.
    pub fn is_cleaned_up(&self) -> bool {
        self.cleaned_up
    }
}

impl Drop for IsolatedWorkspace {
    fn drop(&mut self) {
        let _ = self.perform_cleanup();
    }
}

/// Sanitizes a string for use in a git branch name.
fn sanitize_branch_name(input: &str) -> String {
    let sanitized: String = input
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '-'
            }
        })
        .collect();

    if sanitized.is_empty() {
        "task".to_string()
    } else {
        sanitized
    }
}
