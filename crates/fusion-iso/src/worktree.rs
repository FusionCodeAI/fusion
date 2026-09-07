//! Git worktree lifecycle management.

use std::path::{Path, PathBuf};
use std::process::Command;

use crate::{command_failed, IsoError, IsoResult};

/// Manages git worktrees within a base git repository.
#[derive(Debug, Clone)]
pub struct WorktreeManager {
	base_path: PathBuf,
}

impl WorktreeManager {
	/// Creates a new `WorktreeManager` rooted at `base_path`.
	pub fn new(base_path: impl Into<PathBuf>) -> Self {
		Self {
			base_path: base_path.into(),
		}
	}

	/// Returns the base repository path.
	pub fn base_path(&self) -> &Path {
		&self.base_path
	}

	/// Computes the default worktree path for a task: `.fusion/worktrees/<task_id>`.
	pub fn worktree_path_for_task(&self, task_id: &str) -> PathBuf {
		self.base_path.join(".fusion").join("worktrees").join(task_id)
	}

	/// Computes the default branch name for a task: `fusion-task-<task_id>`.
	pub fn branch_for_task(&self, task_id: &str) -> String {
		format!("fusion-task-{task_id}")
	}

	/// Creates an isolated git worktree at `path` on `branch`.
	/// Uses `-B` to safely create or reset the branch to `HEAD`.
	pub fn create_worktree(&self, path: &Path, branch: &str) -> IsoResult<()> {
		if let Some(parent) = path.parent() {
			std::fs::create_dir_all(parent).map_err(|e| {
				IsoError::other(format!("failed to create worktree parent directory: {e}"))
			})?;
		}

		// Clean up if path already exists
		if path.exists() {
			let _ = self.remove_worktree(path);
			if path.exists() {
				let _ = std::fs::remove_dir_all(path);
			}
		}

		let output = Command::new("git")
			.arg("-C")
			.arg(&self.base_path)
			.args(["worktree", "add", "-B", branch])
			.arg(path)
			.arg("HEAD")
			.stdin(std::process::Stdio::null())
			.stdout(std::process::Stdio::piped())
			.stderr(std::process::Stdio::piped())
			.output()
			.map_err(|err| {
				if err.kind() == std::io::ErrorKind::NotFound {
					IsoError::unavailable("`git` not on PATH")
				} else {
					IsoError::other(format!("spawn git worktree add: {err}"))
				}
			})?;

		if !output.status.success() {
			return Err(command_failed(
				"git worktree add",
				output.status.code().unwrap_or(-1),
				&output.stderr,
			));
		}

		Ok(())
	}

	/// Removes a worktree and forces cleanup of any remaining files.
	pub fn remove_worktree(&self, path: &Path) -> IsoResult<()> {
		let output = Command::new("git")
			.arg("-C")
			.arg(&self.base_path)
			.args(["worktree", "remove", "--force"])
			.arg(path)
			.stdin(std::process::Stdio::null())
			.stdout(std::process::Stdio::piped())
			.stderr(std::process::Stdio::piped())
			.output();

		if path.exists() {
			let _ = std::fs::remove_dir_all(path);
		}

		let _ = Command::new("git")
			.arg("-C")
			.arg(&self.base_path)
			.args(["worktree", "prune"])
			.stdin(std::process::Stdio::null())
			.stdout(std::process::Stdio::null())
			.stderr(std::process::Stdio::null())
			.output();

		if let Ok(out) = output {
			if !out.status.success() && path.exists() {
				return Err(command_failed(
					"git worktree remove",
					out.status.code().unwrap_or(-1),
					&out.stderr,
				));
			}
		}

		Ok(())
	}

	/// Deletes a branch by name.
	pub fn delete_branch(&self, branch: &str) -> IsoResult<()> {
		let output = Command::new("git")
			.arg("-C")
			.arg(&self.base_path)
			.args(["branch", "-D", branch])
			.stdin(std::process::Stdio::null())
			.stdout(std::process::Stdio::piped())
			.stderr(std::process::Stdio::piped())
			.output()
			.map_err(|err| IsoError::other(format!("spawn git branch -D: {err}")))?;

		if !output.status.success() {
			let stderr = String::from_utf8_lossy(&output.stderr);
			// If the branch is already gone, that is acceptable
			if !stderr.contains("not found") {
				return Err(command_failed(
					"git branch -D",
					output.status.code().unwrap_or(-1),
					&output.stderr,
				));
			}
		}

		Ok(())
	}

	/// Computes the unified diff of modifications made within a worktree.
	pub fn collect_diff(&self, worktree_path: &Path) -> IsoResult<String> {
		// Run git add -N . to mark untracked files for inclusion in the diff
		let _ = Command::new("git")
			.arg("-C")
			.arg(worktree_path)
			.args(["add", "-N", "."])
			.stdin(std::process::Stdio::null())
			.stdout(std::process::Stdio::null())
			.stderr(std::process::Stdio::null())
			.output();

		let output = Command::new("git")
			.arg("-C")
			.arg(worktree_path)
			.args(["-c", "core.quotepath=off", "diff", "--no-color", "HEAD"])
			.stdin(std::process::Stdio::null())
			.stdout(std::process::Stdio::piped())
			.stderr(std::process::Stdio::piped())
			.output()
			.map_err(|err| {
				if err.kind() == std::io::ErrorKind::NotFound {
					IsoError::unavailable("`git` not on PATH")
				} else {
					IsoError::other(format!("spawn git diff: {err}"))
				}
			})?;

		if !output.status.success() {
			return Err(command_failed(
				"git diff",
				output.status.code().unwrap_or(-1),
				&output.stderr,
			));
		}

		String::from_utf8(output.stdout)
			.map_err(|e| IsoError::other(format!("git diff output not UTF-8: {e}")))
	}

	/// Checks if a branch exists.
	pub fn branch_exists(&self, branch: &str) -> bool {
		let output = Command::new("git")
			.arg("-C")
			.arg(&self.base_path)
			.args(["branch", "--list", branch])
			.output();
		if let Ok(out) = output {
			let s = String::from_utf8_lossy(&out.stdout);
			!s.trim().is_empty()
		} else {
			false
		}
	}

	/// Lists all worktree paths associated with the base repository.
	pub fn list_worktrees(&self) -> IsoResult<Vec<PathBuf>> {
		let output = Command::new("git")
			.arg("-C")
			.arg(&self.base_path)
			.args(["worktree", "list", "--porcelain"])
			.stdin(std::process::Stdio::null())
			.stdout(std::process::Stdio::piped())
			.stderr(std::process::Stdio::piped())
			.output()
			.map_err(|err| {
				if err.kind() == std::io::ErrorKind::NotFound {
					IsoError::unavailable("`git` not on PATH")
				} else {
					IsoError::other(format!("spawn git worktree list: {err}"))
				}
			})?;

		if !output.status.success() {
			return Err(command_failed(
				"git worktree list",
				output.status.code().unwrap_or(-1),
				&output.stderr,
			));
		}

		let stdout = String::from_utf8_lossy(&output.stdout);
		let mut paths = Vec::new();
		for line in stdout.lines() {
			if let Some(rest) = line.strip_prefix("worktree ") {
				paths.push(PathBuf::from(rest.trim()));
			}
		}

		Ok(paths)
	}
}
