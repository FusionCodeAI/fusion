use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

use crate::tools::file::resolve_path;
use crate::tools::git::{find_git_root, run_git_command, GitProcessOutput};
use crate::tools::types::{Tool, ToolContext};

// ============================================================================
// Data Types
// ============================================================================

/// Information about a single Git branch.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BranchInfo {
    /// Name of the branch (e.g. "main", "feature/auth", "origin/main").
    pub name: String,
    /// Whether this is the currently checked-out branch.
    pub is_current: bool,
    /// Whether this is a remote-tracking branch.
    pub is_remote: bool,
    /// Short SHA of the latest commit on this branch.
    pub commit_hash: String,
    /// Subject line of the latest commit.
    pub commit_subject: String,
    /// Relative or ISO commit date (e.g. "2 hours ago").
    pub commit_date: Option<String>,
    /// Upstream tracking branch name (e.g. "origin/main").
    pub upstream: Option<String>,
    /// Commits ahead of upstream.
    pub ahead: usize,
    /// Commits behind upstream.
    pub behind: usize,
    /// Whether the upstream tracking branch has been deleted (gone).
    pub is_gone: bool,
}

/// Structured report of branch listing.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct BranchListReport {
    /// Active branch name (if not detached).
    pub current_branch: Option<String>,
    /// Whether repository is in detached HEAD state.
    pub is_detached: bool,
    /// Commit hash if in detached HEAD state.
    pub detached_commit: Option<String>,
    /// Subject if in detached HEAD state.
    pub detached_subject: Option<String>,
    /// List of discovered branches.
    pub branches: Vec<BranchInfo>,
    /// Path to the repository root.
    pub repo_root: String,
}

/// Result of a branch manipulation operation (create, switch, delete, rename).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BranchOpResult {
    /// Action performed (e.g. "list", "create", "switch", "delete", "rename", "upstream").
    pub action: String,
    /// Target branch name.
    pub branch: String,
    /// Whether the operation succeeded.
    pub success: bool,
    /// Detailed human-readable message.
    pub message: String,
    /// Previous branch name (for switch / rename).
    pub previous_branch: Option<String>,
    /// Commit hash associated with the operation.
    pub commit: Option<String>,
}

/// Information about a single Git worktree.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorktreeInfo {
    /// Filesystem path of the worktree directory.
    pub path: String,
    /// HEAD commit hash (short SHA).
    pub head_commit: String,
    /// Branch checked out in this worktree (None if detached or bare).
    pub branch: Option<String>,
    /// Whether this worktree is in detached HEAD state.
    pub is_detached: bool,
    /// Whether this worktree represents a bare repository.
    pub is_bare: bool,
    /// Whether this worktree is locked.
    pub is_locked: bool,
    /// Optional lock reason.
    pub lock_reason: Option<String>,
    /// Whether this worktree is prunable.
    pub is_prunable: bool,
    /// Optional prune reason.
    pub prune_reason: Option<String>,
    /// Whether this is the main (primary) worktree.
    pub is_main: bool,
}

/// Structured report of worktree listing.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorktreeListReport {
    /// List of discovered worktrees.
    pub worktrees: Vec<WorktreeInfo>,
    /// Path to the main repository root.
    pub repo_root: String,
    /// Total number of worktrees.
    pub total_count: usize,
}

/// Result of a worktree operation (add, remove, prune, lock, unlock).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorktreeOpResult {
    /// Action performed (e.g. "create_worktree", "remove_worktree", "prune_worktrees", "lock_worktree").
    pub action: String,
    /// Target worktree path.
    pub path: String,
    /// Branch name associated with this worktree.
    pub branch: Option<String>,
    /// Whether the operation succeeded.
    pub success: bool,
    /// Detailed human-readable message.
    pub message: String,
    /// Commit hash at the worktree HEAD.
    pub commit: Option<String>,
}

// ============================================================================
// Branch Name Validation
// ============================================================================

/// Validates a Git branch name according to git-check-ref-format specifications.
///
/// Rules:
/// - Cannot be empty
/// - Cannot start or end with `/` or contain `//`
/// - Cannot start or end with `.`
/// - Cannot end with `.lock`
/// - Cannot contain `..`, `~`, `^`, `:`, `?`, `*`, `[`, `\`, `@{`
/// - Cannot contain ASCII control characters or whitespace
/// - Cannot be single `@`
pub fn validate_branch_name(name: &str) -> anyhow::Result<()> {
    let name = name.trim();
    if name.is_empty() {
        anyhow::bail!("Branch name cannot be empty");
    }

    if name == "@" {
        anyhow::bail!("Branch name cannot be single '@'");
    }

    if name.starts_with('/') || name.ends_with('/') {
        anyhow::bail!("Branch name cannot start or end with '/'");
    }

    if name.contains("//") {
        anyhow::bail!("Branch name cannot contain consecutive slashes ('//')");
    }

    if name.starts_with('.') || name.ends_with('.') {
        anyhow::bail!("Branch name cannot start or end with a dot ('.')");
    }

    if name.ends_with(".lock") {
        anyhow::bail!("Branch name cannot end with '.lock'");
    }

    if name.contains("..") {
        anyhow::bail!("Branch name cannot contain double dots ('..')");
    }

    if name.contains("@{") {
        anyhow::bail!("Branch name cannot contain '@{{' sequence");
    }

    for (idx, ch) in name.chars().enumerate() {
        if ch.is_ascii_control() || ch.is_whitespace() {
            anyhow::bail!(
                "Branch name cannot contain whitespace or control characters (found at character index {idx})"
            );
        }
        match ch {
            '~' | '^' | ':' | '?' | '*' | '[' | '\\' => {
                anyhow::bail!("Branch name cannot contain reserved character '{ch}'");
            }
            _ => {}
        }
    }

    Ok(())
}

// ============================================================================
// Git Branch Helper Functions
// ============================================================================

/// Parses tracking info from `%(upstream:track)` format string.
/// E.g.: `[ahead 2, behind 1]`, `[ahead 3]`, `[behind 5]`, `[gone]`.
pub fn parse_upstream_tracking(track_str: &str) -> (usize, usize, bool) {
    let mut ahead = 0;
    let mut behind = 0;
    let mut is_gone = false;

    let clean = track_str.trim().trim_start_matches('[').trim_end_matches(']');
    if clean.is_empty() {
        return (ahead, behind, is_gone);
    }

    if clean.contains("gone") {
        is_gone = true;
    }

    for part in clean.split(',') {
        let part = part.trim();
        if let Some(ahead_val) = part.strip_prefix("ahead ") {
            if let Ok(n) = ahead_val.trim().parse::<usize>() {
                ahead = n;
            }
        } else if let Some(behind_val) = part.strip_prefix("behind ") {
            if let Ok(n) = behind_val.trim().parse::<usize>() {
                behind = n;
            }
        }
    }

    (ahead, behind, is_gone)
}

/// Checks whether the Git working directory contains uncommitted changes.
pub async fn check_working_tree_dirty(repo_dir: &Path) -> anyhow::Result<bool> {
    let output = run_git_command(&["status", "--porcelain=v1"], repo_dir, 10).await?;
    if !output.success {
        return Ok(false);
    }
    Ok(!output.stdout.trim().is_empty())
}

/// Queries the active HEAD branch or detached state.
pub async fn get_current_head_info(
    repo_dir: &Path,
) -> anyhow::Result<(Option<String>, bool, Option<String>, Option<String>)> {
    // Check if HEAD is symbolic (named branch)
    let sym_output = run_git_command(&["symbolic-ref", "--short", "-q", "HEAD"], repo_dir, 5).await?;
    if sym_output.success && !sym_output.stdout.trim().is_empty() {
        let branch = sym_output.stdout.trim().to_string();
        return Ok((Some(branch), false, None, None));
    }

    // Otherwise, detached HEAD
    let hash_output = run_git_command(&["rev-parse", "--short", "HEAD"], repo_dir, 5).await?;
    let commit_hash = if hash_output.success && !hash_output.stdout.trim().is_empty() {
        Some(hash_output.stdout.trim().to_string())
    } else {
        None
    };

    let subject_output = run_git_command(&["log", "-1", "--format=%s", "HEAD"], repo_dir, 5).await?;
    let commit_subject = if subject_output.success && !subject_output.stdout.trim().is_empty() {
        Some(subject_output.stdout.trim().to_string())
    } else {
        None
    };

    Ok((None, true, commit_hash, commit_subject))
}

/// Checks if a local branch exists by name.
pub async fn local_branch_exists(name: &str, repo_dir: &Path) -> bool {
    let full_ref = format!("refs/heads/{name}");
    let out = run_git_command(&["rev-parse", "--verify", "--quiet", &full_ref], repo_dir, 5).await;
    match out {
        Ok(res) => res.success && !res.stdout.trim().is_empty(),
        Err(_) => false,
    }
}

/// Checks if a remote tracking branch exists by name (e.g. "origin/main").
pub async fn remote_branch_exists(name: &str, repo_dir: &Path) -> bool {
    let full_ref = format!("refs/remotes/{name}");
    let out = run_git_command(&["rev-parse", "--verify", "--quiet", &full_ref], repo_dir, 5).await;
    match out {
        Ok(res) => res.success && !res.stdout.trim().is_empty(),
        Err(_) => false,
    }
}

/// Lists branches in the repository using `git for-each-ref`.
pub async fn list_branches(
    repo_dir: &Path,
    include_remotes: bool,
    merged: Option<bool>,
    contains: Option<&str>,
    sort_by: Option<&str>,
) -> anyhow::Result<BranchListReport> {
    let (current_branch, is_detached, detached_commit, detached_subject) =
        get_current_head_info(repo_dir).await?;

    let mut args = vec![
        "for-each-ref".to_string(),
        "--format=%(refname)|%(refname:short)|%(HEAD)|%(objectname:short)|%(committerdate:relative)|%(upstream:short)|%(upstream:track)|%(subject)".to_string(),
    ];

    let sort_arg = format!("--sort={}", sort_by.unwrap_or("-committerdate"));
    args.push(sort_arg);

    if let Some(true) = merged {
        args.push("--merged=HEAD".to_string());
    } else if let Some(false) = merged {
        args.push("--no-merged=HEAD".to_string());
    }

    if let Some(c) = contains {
        if !c.trim().is_empty() {
            args.push(format!("--contains={}", c.trim()));
        }
    }

    args.push("refs/heads/".to_string());
    if include_remotes {
        args.push("refs/remotes/".to_string());
    }

    let args_str: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
    let output = run_git_command(&args_str, repo_dir, 15).await?;

    if !output.success {
        // In a fresh repo with no commits yet, for-each-ref returns nothing or fails quietly
        let err_msg = output.stderr.trim();
        if err_msg.is_empty() || err_msg.contains("fatal: not a git repository") {
            anyhow::bail!("Failed to list branches: {err_msg}");
        }
    }

    let mut branches = Vec::new();
    for line in output.stdout.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }

        let parts: Vec<&str> = line.splitn(8, '|').collect();
        if parts.len() < 8 {
            continue;
        }

        let full_ref = parts[0];
        let short_name = parts[1].to_string();
        let head_marker = parts[2].trim();
        let commit_hash = parts[3].to_string();
        let commit_date = if parts[4].is_empty() {
            None
        } else {
            Some(parts[4].to_string())
        };
        let upstream = if parts[5].is_empty() {
            None
        } else {
            Some(parts[5].to_string())
        };
        let track_str = parts[6];
        let commit_subject = parts[7].to_string();

        let is_remote = full_ref.starts_with("refs/remotes/");
        let is_current = head_marker == "*"
            || (!is_remote
                && current_branch
                    .as_ref()
                    .map(|cb| cb == &short_name)
                    .unwrap_or(false));

        let (ahead, behind, is_gone) = parse_upstream_tracking(track_str);

        branches.push(BranchInfo {
            name: short_name,
            is_current,
            is_remote,
            commit_hash,
            commit_subject,
            commit_date,
            upstream,
            ahead,
            behind,
            is_gone,
        });
    }

    Ok(BranchListReport {
        current_branch,
        is_detached,
        detached_commit,
        detached_subject,
        branches,
        repo_root: repo_dir.display().to_string(),
    })
}

/// Formats the branch list report into human-readable terminal text.
pub fn format_branch_list(report: &BranchListReport) -> String {
    let mut out = String::new();

    if report.branches.is_empty() {
        if report.is_detached {
            if let Some(hash) = &report.detached_commit {
                let subj = report.detached_subject.as_deref().unwrap_or("No commit subject");
                out.push_str(&format!(
                    "* (HEAD detached at {hash}) {subj}\n\n(No local branches found)\n"
                ));
            } else {
                out.push_str("(No branches or commits in repository yet)\n");
            }
        } else if let Some(cb) = &report.current_branch {
            out.push_str(&format!("* {cb} (initial branch, no commits yet)\n"));
        } else {
            out.push_str("No branches found in repository.\n");
        }
        return out;
    }

    // If detached HEAD, display top banner
    if report.is_detached {
        let hash = report.detached_commit.as_deref().unwrap_or("unknown");
        let subj = report.detached_subject.as_deref().unwrap_or("");
        out.push_str(&format!("* (HEAD detached at {hash}) {subj}\n"));
    }

    // Determine column widths for alignment
    let max_name_len = report
        .branches
        .iter()
        .map(|b| b.name.len())
        .max()
        .unwrap_or(10)
        .max(12);

    for branch in &report.branches {
        let current_marker = if branch.is_current { "* " } else { "  " };

        let mut tracking_info = String::new();
        if let Some(up) = &branch.upstream {
            if branch.is_gone {
                tracking_info = format!(" [{up}: gone]");
            } else if branch.ahead > 0 && branch.behind > 0 {
                tracking_info = format!(" [{up}: ahead {}, behind {}]", branch.ahead, branch.behind);
            } else if branch.ahead > 0 {
                tracking_info = format!(" [{up}: ahead {}]", branch.ahead);
            } else if branch.behind > 0 {
                tracking_info = format!(" [{up}: behind {}]", branch.behind);
            } else {
                tracking_info = format!(" [{up}]");
            }
        }

        let date_str = branch
            .commit_date
            .as_ref()
            .map(|d| format!(" ({d})"))
            .unwrap_or_default();

        let remote_tag = if branch.is_remote { " [remote]" } else { "" };

        out.push_str(&format!(
            "{}{:<width$}{} {} {}{}{}\n",
            current_marker,
            branch.name,
            remote_tag,
            branch.commit_hash,
            branch.commit_subject,
            tracking_info,
            date_str,
            width = max_name_len
        ));
    }

    out
}

// ============================================================================
// Git Worktree Helper Functions
// ============================================================================

/// Parses worktrees from `git worktree list --porcelain` format.
pub fn parse_worktree_porcelain(output: &str, repo_root: &Path) -> Vec<WorktreeInfo> {
    let mut worktrees = Vec::new();
    let mut current_path: Option<String> = None;
    let mut current_head = String::new();
    let mut current_branch: Option<String> = None;
    let mut is_detached = false;
    let mut is_bare = false;
    let mut is_locked = false;
    let mut lock_reason: Option<String> = None;
    let mut is_prunable = false;
    let mut prune_reason: Option<String> = None;

    let flush_entry = |worktrees: &mut Vec<WorktreeInfo>,
                       path: &mut Option<String>,
                       head: &mut String,
                       branch: &mut Option<String>,
                       detached: &mut bool,
                       bare: &mut bool,
                       locked: &mut bool,
                       lock_r: &mut Option<String>,
                       prunable: &mut bool,
                       prune_r: &mut Option<String>| {
        if let Some(p) = path.take() {
            let is_main = worktrees.is_empty()
                || repo_root
                    .to_str()
                    .map(|rr| rr.trim_end_matches('/') == p.trim_end_matches('/'))
                    .unwrap_or(false);

            let short_head = if head.len() > 7 {
                head[..7].to_string()
            } else {
                head.clone()
            };

            worktrees.push(WorktreeInfo {
                path: p,
                head_commit: if short_head.is_empty() {
                    "HEAD".to_string()
                } else {
                    short_head
                },
                branch: branch.take(),
                is_detached: *detached,
                is_bare: *bare,
                is_locked: *locked,
                lock_reason: lock_r.take(),
                is_prunable: *prunable,
                prune_reason: prune_r.take(),
                is_main,
            });

            *head = String::new();
            *detached = false;
            *bare = false;
            *locked = false;
            *prunable = false;
        }
    };

    for line in output.lines() {
        let line = line.trim();
        if line.is_empty() {
            flush_entry(
                &mut worktrees,
                &mut current_path,
                &mut current_head,
                &mut current_branch,
                &mut is_detached,
                &mut is_bare,
                &mut is_locked,
                &mut lock_reason,
                &mut is_prunable,
                &mut prune_reason,
            );
            continue;
        }

        if let Some(p) = line.strip_prefix("worktree ") {
            flush_entry(
                &mut worktrees,
                &mut current_path,
                &mut current_head,
                &mut current_branch,
                &mut is_detached,
                &mut is_bare,
                &mut is_locked,
                &mut lock_reason,
                &mut is_prunable,
                &mut prune_reason,
            );
            current_path = Some(p.trim().to_string());
        } else if let Some(h) = line.strip_prefix("HEAD ") {
            current_head = h.trim().to_string();
        } else if let Some(b) = line.strip_prefix("branch ") {
            let b_clean = b.trim();
            let branch_name = b_clean
                .strip_prefix("refs/heads/")
                .unwrap_or(b_clean)
                .to_string();
            current_branch = Some(branch_name);
        } else if line == "detached" {
            is_detached = true;
        } else if line == "bare" {
            is_bare = true;
        } else if line.starts_with("locked") {
            is_locked = true;
            let reason = line
                .strip_prefix("locked")
                .map(|s| s.trim())
                .filter(|s| !s.is_empty());
            lock_reason = reason.map(|s| s.to_string());
        } else if line.starts_with("prunable") {
            is_prunable = true;
            let reason = line
                .strip_prefix("prunable")
                .map(|s| s.trim())
                .filter(|s| !s.is_empty());
            prune_reason = reason.map(|s| s.to_string());
        }
    }

    // Flush last entry
    flush_entry(
        &mut worktrees,
        &mut current_path,
        &mut current_head,
        &mut current_branch,
        &mut is_detached,
        &mut is_bare,
        &mut is_locked,
        &mut lock_reason,
        &mut is_prunable,
        &mut prune_reason,
    );

    worktrees
}

/// Fallback parser for standard (non-porcelain) `git worktree list` output.
pub fn parse_worktree_standard(output: &str, repo_root: &Path) -> Vec<WorktreeInfo> {
    let mut worktrees = Vec::new();

    for (idx, line) in output.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }

        let is_bare = line.contains("(bare)");
        let is_detached = line.contains("(detached HEAD)") || line.contains("(detached)");
        let is_locked = line.contains("locked");
        let is_prunable = line.contains("prunable");

        let parts: Vec<&str> = line.split_whitespace().collect();
        if parts.is_empty() {
            continue;
        }

        let path = parts[0].to_string();
        let head_commit = if parts.len() > 1 && !parts[1].starts_with('(') && !parts[1].starts_with('[') {
            parts[1].to_string()
        } else {
            "HEAD".to_string()
        };

        let mut branch = None;
        for part in &parts {
            if part.starts_with('[') && part.ends_with(']') {
                let b = &part[1..part.len() - 1];
                branch = Some(b.to_string());
                break;
            }
        }

        let is_main = idx == 0
            || repo_root
                .to_str()
                .map(|rr| rr.trim_end_matches('/') == path.trim_end_matches('/'))
                .unwrap_or(false);

        worktrees.push(WorktreeInfo {
            path,
            head_commit,
            branch,
            is_detached,
            is_bare,
            is_locked,
            lock_reason: None,
            is_prunable,
            prune_reason: None,
            is_main,
        });
    }

    worktrees
}

/// Lists all worktrees configured in the repository.
pub async fn list_worktrees(repo_dir: &Path) -> anyhow::Result<WorktreeListReport> {
    let output = run_git_command(&["worktree", "list", "--porcelain"], repo_dir, 15).await?;

    let worktrees = if output.success && !output.stdout.trim().is_empty() {
        parse_worktree_porcelain(&output.stdout, repo_dir)
    } else {
        // Fallback to standard listing
        let std_output = run_git_command(&["worktree", "list"], repo_dir, 15).await?;
        if std_output.success {
            parse_worktree_standard(&std_output.stdout, repo_dir)
        } else {
            Vec::new()
        }
    };

    let total = worktrees.len();
    Ok(WorktreeListReport {
        worktrees,
        repo_root: repo_dir.display().to_string(),
        total_count: total,
    })
}

/// Formats a worktree list report into human-readable terminal text.
pub fn format_worktree_list(report: &WorktreeListReport) -> String {
    let mut out = String::new();
    if report.worktrees.is_empty() {
        out.push_str("No worktrees found in repository.\n");
        return out;
    }

    out.push_str(&format!(
        "Worktrees ({}) in repository '{}':\n\n",
        report.total_count, report.repo_root
    ));

    let max_path_len = report
        .worktrees
        .iter()
        .map(|w| w.path.len())
        .max()
        .unwrap_or(20)
        .max(20);

    for wt in &report.worktrees {
        let prefix = if wt.is_main { "* [main] " } else { "  [work] " };
        let mut details = Vec::new();

        if let Some(b) = &wt.branch {
            details.push(format!("branch: {b}"));
        } else if wt.is_detached {
            details.push("detached HEAD".to_string());
        } else if wt.is_bare {
            details.push("bare repository".to_string());
        }

        if !wt.head_commit.is_empty() {
            details.push(format!("commit: {}", wt.head_commit));
        }

        if wt.is_locked {
            if let Some(r) = &wt.lock_reason {
                details.push(format!("locked ({r})"));
            } else {
                details.push("locked".to_string());
            }
        }

        if wt.is_prunable {
            if let Some(r) = &wt.prune_reason {
                details.push(format!("prunable ({r})"));
            } else {
                details.push("prunable".to_string());
            }
        }

        out.push_str(&format!(
            "{}{:<width$}  ({})\n",
            prefix,
            wt.path,
            details.join(", "),
            width = max_path_len
        ));
    }

    out
}

/// Creates a new Git worktree at `worktree_path`.
pub async fn create_worktree(
    repo_dir: &Path,
    worktree_path: &Path,
    branch: Option<&str>,
    create_branch: bool,
    base: Option<&str>,
    force: bool,
    detach: bool,
) -> anyhow::Result<WorktreeOpResult> {
    let path_str = worktree_path.to_str().ok_or_else(|| {
        anyhow::anyhow!("Invalid worktree path encoding: {}", worktree_path.display())
    })?;

    let mut cmd_args: Vec<&str> = vec!["worktree", "add"];

    if force {
        cmd_args.push("-f");
    }

    if detach {
        cmd_args.push("--detach");
        cmd_args.push(path_str);
        if let Some(b) = base.or(branch) {
            cmd_args.push(b);
        }
    } else if create_branch {
        let branch_name = branch.ok_or_else(|| {
            anyhow::anyhow!("Branch name is required when creating a new branch for a worktree")
        })?;
        validate_branch_name(branch_name)?;

        cmd_args.push("-b");
        cmd_args.push(branch_name);
        cmd_args.push(path_str);
        if let Some(bs) = base {
            cmd_args.push(bs);
        }
    } else {
        cmd_args.push(path_str);
        if let Some(b) = branch {
            cmd_args.push(b);
        }
    }

    let output = run_git_command(&cmd_args, repo_dir, 30).await?;
    if !output.success {
        let err = output.stderr.trim();
        if err.contains("already checked out") {
            anyhow::bail!(
                "Cannot create worktree: branch is already checked out in another worktree.\n{err}"
            );
        }
        if err.contains("already exists") {
            anyhow::bail!(
                "Cannot create worktree: target path '{}' already exists.\n{err}",
                worktree_path.display()
            );
        }
        anyhow::bail!("Failed to create worktree: {err}");
    }

    // Query HEAD in the newly created worktree
    let head_commit = run_git_command(&["rev-parse", "--short", "HEAD"], worktree_path, 5)
        .await
        .ok()
        .map(|o| o.stdout.trim().to_string());

    let branch_desc = if detach {
        "detached HEAD".to_string()
    } else if let Some(b) = branch {
        format!("branch '{b}'")
    } else {
        "active branch".to_string()
    };

    Ok(WorktreeOpResult {
        action: "create_worktree".to_string(),
        path: worktree_path.display().to_string(),
        branch: if detach { None } else { branch.map(|s| s.to_string()) },
        success: true,
        message: format!(
            "Created worktree at '{}' with {}{}",
            worktree_path.display(),
            branch_desc,
            head_commit
                .as_ref()
                .map(|h| format!(" ({h})"))
                .unwrap_or_default()
        ),
        commit: head_commit,
    })
}

/// Removes a Git worktree.
pub async fn remove_worktree(
    repo_dir: &Path,
    worktree_target: &str,
    force: bool,
) -> anyhow::Result<WorktreeOpResult> {
    let report = list_worktrees(repo_dir).await?;

    let target_clean = worktree_target.trim();

    // Look for matching worktree by path or by branch
    let matched_wt = report.worktrees.iter().find(|wt| {
        wt.path == target_clean
            || wt.path.ends_with(target_clean)
            || wt.branch.as_deref() == Some(target_clean)
    });

    let (resolved_path, is_main, branch_name) = if let Some(wt) = matched_wt {
        (wt.path.clone(), wt.is_main, wt.branch.clone())
    } else {
        (target_clean.to_string(), false, None)
    };

    if is_main
        || repo_dir
            .to_str()
            .map(|rr| rr.trim_end_matches('/') == resolved_path.trim_end_matches('/'))
            .unwrap_or(false)
    {
        anyhow::bail!(
            "Cannot remove main worktree at '{}'. Only secondary / linked worktrees can be removed.",
            resolved_path
        );
    }

    let mut cmd_args = vec!["worktree", "remove"];
    if force {
        cmd_args.push("-f");
    }
    cmd_args.push(&resolved_path);

    let output = run_git_command(&cmd_args, repo_dir, 30).await?;
    if !output.success {
        let err = output.stderr.trim();
        // If the directory was already manually deleted, run prune
        if err.contains("No such file or directory") || err.contains("is missing") {
            let _ = run_git_command(&["worktree", "prune"], repo_dir, 15).await;
            return Ok(WorktreeOpResult {
                action: "remove_worktree".to_string(),
                path: resolved_path.clone(),
                branch: branch_name,
                success: true,
                message: format!(
                    "Worktree directory at '{}' was missing; pruned worktree record from repository.",
                    resolved_path
                ),
                commit: None,
            });
        }
        anyhow::bail!("Failed to remove worktree '{}': {err}", resolved_path);
    }

    Ok(WorktreeOpResult {
        action: "remove_worktree".to_string(),
        path: resolved_path.clone(),
        branch: branch_name,
        success: true,
        message: format!("Removed worktree at '{}'.", resolved_path),
        commit: None,
    })
}

/// Prunes stale worktrees from the repository.
pub async fn prune_worktrees(
    repo_dir: &Path,
    expire: Option<&str>,
    dry_run: bool,
) -> anyhow::Result<WorktreeOpResult> {
    let mut cmd_args = vec!["worktree", "prune", "-v"];
    if dry_run {
        cmd_args.push("-n");
    }
    if let Some(exp) = expire {
        cmd_args.push("--expire");
        cmd_args.push(exp);
    }

    let output = run_git_command(&cmd_args, repo_dir, 15).await?;
    if !output.success {
        let err = output.stderr.trim();
        anyhow::bail!("Failed to prune worktrees: {err}");
    }

    let details = if output.stdout.trim().is_empty() {
        "No stale worktrees to prune.".to_string()
    } else {
        output.stdout.trim().to_string()
    };

    Ok(WorktreeOpResult {
        action: "prune_worktrees".to_string(),
        path: repo_dir.display().to_string(),
        branch: None,
        success: true,
        message: format!(
            "{}{}",
            if dry_run { "[dry-run] " } else { "" },
            details
        ),
        commit: None,
    })
}

/// Locks a worktree to prevent automatic pruning.
pub async fn lock_worktree(
    repo_dir: &Path,
    worktree_target: &str,
    reason: Option<&str>,
) -> anyhow::Result<WorktreeOpResult> {
    let mut cmd_args = vec!["worktree", "lock"];
    if let Some(r) = reason {
        cmd_args.push("--reason");
        cmd_args.push(r);
    }
    cmd_args.push(worktree_target);

    let output = run_git_command(&cmd_args, repo_dir, 10).await?;
    if !output.success {
        let err = output.stderr.trim();
        anyhow::bail!("Failed to lock worktree '{worktree_target}': {err}");
    }

    Ok(WorktreeOpResult {
        action: "lock_worktree".to_string(),
        path: worktree_target.to_string(),
        branch: None,
        success: true,
        message: format!(
            "Locked worktree '{}'{}",
            worktree_target,
            reason.map(|r| format!(" (reason: {r})")).unwrap_or_default()
        ),
        commit: None,
    })
}

/// Unlocks a locked worktree.
pub async fn unlock_worktree(
    repo_dir: &Path,
    worktree_target: &str,
) -> anyhow::Result<WorktreeOpResult> {
    let output = run_git_command(&["worktree", "unlock", worktree_target], repo_dir, 10).await?;
    if !output.success {
        let err = output.stderr.trim();
        anyhow::bail!("Failed to unlock worktree '{worktree_target}': {err}");
    }

    Ok(WorktreeOpResult {
        action: "unlock_worktree".to_string(),
        path: worktree_target.to_string(),
        branch: None,
        success: true,
        message: format!("Unlocked worktree '{}'.", worktree_target),
        commit: None,
    })
}

// ============================================================================
// GitBranchTool Implementation
// ============================================================================

#[derive(Default, Debug, Clone)]
pub struct GitBranchTool;

impl GitBranchTool {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl Tool for GitBranchTool {
    fn name(&self) -> &str {
        "git_branch"
    }

    fn description(&self) -> &str {
        "Manage Git branches and worktrees safely: list branches and worktrees, create/switch/delete/rename branches, create/remove/list/prune/lock worktrees, manage upstream tracking, and inspect current repository and worktree status."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "action": {
                    "type": "string",
                    "enum": [
                        "list", "create", "switch", "checkout", "delete", "rename",
                        "current", "status", "upstream",
                        "create_worktree", "remove_worktree", "list_worktrees",
                        "prune_worktrees", "lock_worktree", "unlock_worktree"
                    ],
                    "description": "Operation to perform: 'list' (branches), 'create', 'switch' (checkout), 'delete', 'rename', 'current' (status), 'upstream', 'create_worktree', 'remove_worktree', 'list_worktrees', 'prune_worktrees', 'lock_worktree', 'unlock_worktree'."
                },
                "branch": {
                    "type": "string",
                    "description": "Name of the branch to create, switch to, delete, rename, or bind to a worktree. (Aliases: 'name', 'target')."
                },
                "name": {
                    "type": "string",
                    "description": "Alias for 'branch'."
                },
                "from": {
                    "type": "string",
                    "description": "Starting commit, tag, or base branch for branch creation or worktree base. (Aliases: 'base', 'start_point')."
                },
                "base": {
                    "type": "string",
                    "description": "Alias for 'from'."
                },
                "start_point": {
                    "type": "string",
                    "description": "Alias for 'from'."
                },
                "switch": {
                    "type": "boolean",
                    "description": "When creating a branch, whether to switch/checkout to it immediately. Default is false."
                },
                "checkout": {
                    "type": "boolean",
                    "description": "Alias for 'switch'."
                },
                "create": {
                    "type": "boolean",
                    "description": "When switching/checking out or adding a worktree, whether to create the branch if it does not already exist."
                },
                "create_if_missing": {
                    "type": "boolean",
                    "description": "Alias for 'create'."
                },
                "new_name": {
                    "type": "string",
                    "description": "Target new branch name when performing 'rename' action."
                },
                "all": {
                    "type": "boolean",
                    "description": "When listing branches, whether to include remote-tracking branches ('refs/remotes/'). Default is false."
                },
                "remote": {
                    "type": "boolean",
                    "description": "When listing branches, include remote branches. When deleting, delete remote branch."
                },
                "force": {
                    "type": "boolean",
                    "description": "Force branch or worktree operation: '-D' for delete unmerged branches, '-M' for rename override, force switch, or force worktree add/remove. Default is false."
                },
                "merged": {
                    "type": "boolean",
                    "description": "When listing branches, filter branches merged into HEAD (true) or unmerged (false)."
                },
                "contains": {
                    "type": "string",
                    "description": "When listing branches, only show branches that contain the specified commit."
                },
                "sort": {
                    "type": "string",
                    "description": "Sorting criteria for branch listing (e.g. '-committerdate', 'refname', '-authordate'). Default is '-committerdate'."
                },
                "upstream": {
                    "type": "string",
                    "description": "Upstream branch name to set for 'upstream' action (e.g. 'origin/main')."
                },
                "unset": {
                    "type": "boolean",
                    "description": "When performing 'upstream' action, whether to unset the upstream tracking branch."
                },
                "worktree_path": {
                    "type": "string",
                    "description": "Target directory path when creating, removing, locking, or unlocking a worktree. (Aliases: 'worktree', 'target_path')."
                },
                "worktree": {
                    "type": "string",
                    "description": "Alias for 'worktree_path'."
                },
                "detach": {
                    "type": "boolean",
                    "description": "When creating a worktree, whether to check out in detached HEAD state. Default is false."
                },
                "reason": {
                    "type": "string",
                    "description": "Lock reason when performing 'lock_worktree'."
                },
                "dry_run": {
                    "type": "boolean",
                    "description": "When pruning worktrees, whether to only report what would be pruned without deleting."
                },
                "json": {
                    "type": "boolean",
                    "description": "If true, return output formatted as structured JSON."
                },
                "path": {
                    "type": "string",
                    "description": "Path to the Git repository or any directory inside it. Defaults to current working directory."
                }
            }
        })
    }

    async fn execute(&self, args: Value, ctx: &ToolContext) -> anyhow::Result<String> {
        let target_dir = if let Some(path_str) = args.get("path").and_then(|v| v.as_str()) {
            resolve_path(path_str, &ctx.cwd)
        } else {
            ctx.cwd.clone()
        };

        let repo_root = find_git_root(&target_dir).ok_or_else(|| {
            anyhow::anyhow!(
                "Not a git repository (or any parent directory): {}",
                target_dir.display()
            )
        })?;

        // Normalize action string
        let action = args
            .get("action")
            .and_then(|v| v.as_str())
            .unwrap_or("list")
            .to_lowercase();

        // Normalize branch name from parameters
        let branch_opt = args
            .get("branch")
            .or_else(|| args.get("name"))
            .or_else(|| args.get("target"))
            .and_then(|v| v.as_str())
            .map(|s| s.trim());

        // Normalize base / start point
        let from_opt = args
            .get("from")
            .or_else(|| args.get("base"))
            .or_else(|| args.get("start_point"))
            .and_then(|v| v.as_str())
            .map(|s| s.trim());

        // Normalize worktree path
        let worktree_opt = args
            .get("worktree_path")
            .or_else(|| args.get("worktree"))
            .or_else(|| args.get("target_path"))
            .or_else(|| args.get("path_target"))
            .and_then(|v| v.as_str())
            .map(|s| s.trim());

        let force = args
            .get("force")
            .or_else(|| args.get("force_delete"))
            .or_else(|| args.get("force_switch"))
            .or_else(|| args.get("allow_dirty"))
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        let json_output = args
            .get("json")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        match action.as_str() {
            // ----------------------------------------------------------------
            // 1. List Branches
            // ----------------------------------------------------------------
            "list" | "list_branches" | "branches" => {
                let include_remotes = args
                    .get("all")
                    .or_else(|| args.get("remote"))
                    .or_else(|| args.get("remotes"))
                    .or_else(|| args.get("include_remotes"))
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false);

                let merged = args.get("merged").and_then(|v| v.as_bool());
                let contains = args.get("contains").and_then(|v| v.as_str());
                let sort_by = args.get("sort").and_then(|v| v.as_str());

                let report = list_branches(
                    &repo_root,
                    include_remotes,
                    merged,
                    contains,
                    sort_by,
                )
                .await?;

                if json_output {
                    Ok(serde_json::to_string_pretty(&report)?)
                } else {
                    Ok(format_branch_list(&report))
                }
            }

            // ----------------------------------------------------------------
            // 2. Create Branch
            // ----------------------------------------------------------------
            "create" | "new" | "create_branch" => {
                let branch_name = branch_opt.ok_or_else(|| {
                    anyhow::anyhow!("Branch name is required for 'create' action (provide 'branch' or 'name')")
                })?;

                validate_branch_name(branch_name)?;

                if local_branch_exists(branch_name, &repo_root).await {
                    anyhow::bail!(
                        "Branch '{branch_name}' already exists. Use 'switch' to switch to it, or 'delete' it first."
                    );
                }

                // Verify base / start point if specified
                if let Some(base) = from_opt {
                    let check_base = run_git_command(
                        &["rev-parse", "--verify", "--quiet", base],
                        &repo_root,
                        5,
                    )
                    .await?;
                    if !check_base.success {
                        anyhow::bail!("Base reference '{base}' does not exist in repository.");
                    }
                }

                let should_switch = args
                    .get("switch")
                    .or_else(|| args.get("checkout"))
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false);

                if should_switch {
                    let mut cmd_args = vec!["checkout", "-b", branch_name];
                    if let Some(base) = from_opt {
                        cmd_args.push(base);
                    }

                    let output = run_git_command(&cmd_args, &repo_root, 15).await?;
                    if !output.success {
                        let err = output.stderr.trim();
                        anyhow::bail!("Failed to create and switch to branch '{branch_name}': {err}");
                    }

                    let head_hash = run_git_command(&["rev-parse", "--short", "HEAD"], &repo_root, 5)
                        .await
                        .ok()
                        .map(|o| o.stdout.trim().to_string());

                    let res = BranchOpResult {
                        action: "create_and_switch".to_string(),
                        branch: branch_name.to_string(),
                        success: true,
                        message: format!(
                            "Created and switched to new branch '{branch_name}'{}",
                            from_opt
                                .map(|b| format!(" (from base '{b}')"))
                                .unwrap_or_default()
                        ),
                        previous_branch: None,
                        commit: head_hash,
                    };

                    if json_output {
                        Ok(serde_json::to_string_pretty(&res)?)
                    } else {
                        Ok(format!("{}\n", res.message))
                    }
                } else {
                    let mut cmd_args = vec!["branch", branch_name];
                    if let Some(base) = from_opt {
                        cmd_args.push(base);
                    }

                    let output = run_git_command(&cmd_args, &repo_root, 10).await?;
                    if !output.success {
                        let err = output.stderr.trim();
                        anyhow::bail!("Failed to create branch '{branch_name}': {err}");
                    }

                    let branch_commit = run_git_command(
                        &["rev-parse", "--short", branch_name],
                        &repo_root,
                        5,
                    )
                    .await
                    .ok()
                    .map(|o| o.stdout.trim().to_string());

                    let res = BranchOpResult {
                        action: "create".to_string(),
                        branch: branch_name.to_string(),
                        success: true,
                        message: format!(
                            "Created branch '{branch_name}'{}",
                            from_opt
                                .map(|b| format!(" (from base '{b}')"))
                                .unwrap_or_default()
                        ),
                        previous_branch: None,
                        commit: branch_commit,
                    };

                    if json_output {
                        Ok(serde_json::to_string_pretty(&res)?)
                    } else {
                        Ok(format!("{}\n", res.message))
                    }
                }
            }

            // ----------------------------------------------------------------
            // 3. Switch / Checkout Branch
            // ----------------------------------------------------------------
            "switch" | "checkout" | "switch_branch" => {
                let branch_name = branch_opt.ok_or_else(|| {
                    anyhow::anyhow!("Branch name is required for 'switch' action (provide 'branch' or 'name')")
                })?;

                let create_if_missing = args
                    .get("create")
                    .or_else(|| args.get("create_if_missing"))
                    .or_else(|| args.get("create_branch"))
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false);

                let exists_locally = local_branch_exists(branch_name, &repo_root).await;

                let (prev_branch, _, _, _) = get_current_head_info(&repo_root).await?;

                if !exists_locally {
                    if create_if_missing {
                        validate_branch_name(branch_name)?;
                        let mut cmd_args = vec!["checkout", "-b", branch_name];
                        if let Some(base) = from_opt {
                            cmd_args.push(base);
                        }

                        let output = run_git_command(&cmd_args, &repo_root, 15).await?;
                        if !output.success {
                            let err = output.stderr.trim();
                            anyhow::bail!("Failed to create and switch to branch '{branch_name}': {err}");
                        }

                        let head_hash = run_git_command(&["rev-parse", "--short", "HEAD"], &repo_root, 5)
                            .await
                            .ok()
                            .map(|o| o.stdout.trim().to_string());

                        let res = BranchOpResult {
                            action: "create_and_switch".to_string(),
                            branch: branch_name.to_string(),
                            success: true,
                            message: format!(
                                "Branch did not exist; created and switched to new branch '{branch_name}'{}",
                                from_opt
                                    .map(|b| format!(" (from base '{b}')"))
                                    .unwrap_or_default()
                            ),
                            previous_branch: prev_branch,
                            commit: head_hash,
                        };

                        if json_output {
                            return Ok(serde_json::to_string_pretty(&res)?);
                        } else {
                            return Ok(format!("{}\n", res.message));
                        }
                    } else {
                        // Check if remote tracking branch exists
                        let remote_name = format!("origin/{branch_name}");
                        if remote_branch_exists(&remote_name, &repo_root).await {
                            // Can checkout remote branch with tracking
                            let output = run_git_command(
                                &["checkout", "--track", &remote_name],
                                &repo_root,
                                15,
                            )
                            .await?;
                            if !output.success {
                                let err = output.stderr.trim();
                                anyhow::bail!("Failed to switch to remote branch '{remote_name}': {err}");
                            }

                            let head_hash = run_git_command(&["rev-parse", "--short", "HEAD"], &repo_root, 5)
                                .await
                                .ok()
                                .map(|o| o.stdout.trim().to_string());

                            let res = BranchOpResult {
                                action: "switch".to_string(),
                                branch: branch_name.to_string(),
                                success: true,
                                message: format!(
                                    "Switched to a new local branch '{branch_name}' tracking remote '{remote_name}'"
                                ),
                                previous_branch: prev_branch,
                                commit: head_hash,
                            };

                            if json_output {
                                return Ok(serde_json::to_string_pretty(&res)?);
                            } else {
                                return Ok(format!("{}\n", res.message));
                            }
                        }

                        anyhow::bail!(
                            "Branch '{branch_name}' does not exist locally or on remote. Set 'create: true' to create a new branch with this name."
                        );
                    }
                }

                // Check if already on this branch
                if let Some(current) = &prev_branch {
                    if current == branch_name {
                        return Ok(format!("Already on branch '{branch_name}'.\n"));
                    }
                }

                // Check for safety / dirty state
                let is_dirty = check_working_tree_dirty(&repo_root).await.unwrap_or(false);

                let mut cmd_args = vec!["checkout"];
                if force {
                    cmd_args.push("-f");
                }
                cmd_args.push(branch_name);

                let output = run_git_command(&cmd_args, &repo_root, 15).await?;
                if !output.success {
                    let err = output.stderr.trim();
                    if err.contains("local changes") || err.contains("would be overwritten") {
                        anyhow::bail!(
                            "Cannot switch to branch '{branch_name}': you have uncommitted changes that would be overwritten by checkout.\nCommit your changes, stash them ('git stash'), or pass 'force: true' to discard uncommitted changes."
                        );
                    }
                    anyhow::bail!("Failed to switch to branch '{branch_name}': {err}");
                }

                let head_hash = run_git_command(&["rev-parse", "--short", "HEAD"], &repo_root, 5)
                    .await
                    .ok()
                    .map(|o| o.stdout.trim().to_string());

                let dirty_note = if is_dirty && !force {
                    " (uncommitted working tree changes carried over)"
                } else if is_dirty && force {
                    " (uncommitted changes were discarded via force)"
                } else {
                    ""
                };

                let res = BranchOpResult {
                    action: "switch".to_string(),
                    branch: branch_name.to_string(),
                    success: true,
                    message: format!(
                        "Switched to branch '{branch_name}'{}{}",
                        head_hash
                            .as_ref()
                            .map(|h| format!(" ({h})"))
                            .unwrap_or_default(),
                        dirty_note
                    ),
                    previous_branch: prev_branch,
                    commit: head_hash,
                };

                if json_output {
                    Ok(serde_json::to_string_pretty(&res)?)
                } else {
                    Ok(format!("{}\n", res.message))
                }
            }

            // ----------------------------------------------------------------
            // 4. Delete Branch
            // ----------------------------------------------------------------
            "delete" | "remove" | "del" | "delete_branch" => {
                let branch_name = branch_opt.ok_or_else(|| {
                    anyhow::anyhow!("Branch name is required for 'delete' action (provide 'branch' or 'name')")
                })?;

                let is_remote = args.get("remote").and_then(|v| v.as_bool()).unwrap_or(false)
                    || branch_name.starts_with("origin/");

                if is_remote {
                    let remote_ref = if let Some(stripped) = branch_name.strip_prefix("origin/") {
                        stripped
                    } else {
                        branch_name
                    };

                    let output = run_git_command(
                        &["push", "origin", "--delete", remote_ref],
                        &repo_root,
                        30,
                    )
                    .await?;

                    if !output.success {
                        let err = output.stderr.trim();
                        anyhow::bail!("Failed to delete remote branch '{branch_name}': {err}");
                    }

                    let res = BranchOpResult {
                        action: "delete_remote".to_string(),
                        branch: branch_name.to_string(),
                        success: true,
                        message: format!("Deleted remote branch '{branch_name}'."),
                        previous_branch: None,
                        commit: None,
                    };

                    if json_output {
                        return Ok(serde_json::to_string_pretty(&res)?);
                    } else {
                        return Ok(format!("{}\n", res.message));
                    }
                }

                // Safety: Cannot delete currently checked-out branch
                let (current_branch, _, _, _) = get_current_head_info(&repo_root).await?;
                if let Some(current) = current_branch {
                    if current == branch_name {
                        anyhow::bail!(
                            "Cannot delete the currently checked-out branch '{branch_name}'. Switch to a different branch first (e.g. action='switch', branch='main')."
                        );
                    }
                }

                if !local_branch_exists(branch_name, &repo_root).await {
                    anyhow::bail!("Branch '{branch_name}' does not exist locally.");
                }

                // Get commit hash before deleting for reporting
                let last_commit = run_git_command(
                    &["rev-parse", "--short", branch_name],
                    &repo_root,
                    5,
                )
                .await
                .ok()
                .map(|o| o.stdout.trim().to_string());

                let flag = if force { "-D" } else { "-d" };
                let output = run_git_command(&["branch", flag, branch_name], &repo_root, 10).await?;

                if !output.success {
                    let err = output.stderr.trim();
                    if err.contains("not fully merged") {
                        anyhow::bail!(
                            "Branch '{branch_name}' is not fully merged into HEAD. If you are sure you want to delete it, pass 'force: true' to force deletion."
                        );
                    }
                    anyhow::bail!("Failed to delete branch '{branch_name}': {err}");
                }

                let res = BranchOpResult {
                    action: "delete".to_string(),
                    branch: branch_name.to_string(),
                    success: true,
                    message: format!(
                        "Deleted branch '{branch_name}'{}",
                        last_commit
                            .as_ref()
                            .map(|h| format!(" (was at {h})"))
                            .unwrap_or_default()
                    ),
                    previous_branch: None,
                    commit: last_commit,
                };

                if json_output {
                    Ok(serde_json::to_string_pretty(&res)?)
                } else {
                    Ok(format!("{}\n", res.message))
                }
            }

            // ----------------------------------------------------------------
            // 5. Rename Branch
            // ----------------------------------------------------------------
            "rename" | "move" | "mv" | "rename_branch" => {
                let new_name = args
                    .get("new_name")
                    .or_else(|| args.get("to"))
                    .and_then(|v| v.as_str())
                    .map(|s| s.trim())
                    .ok_or_else(|| {
                        anyhow::anyhow!("'new_name' is required for 'rename' action")
                    })?;

                validate_branch_name(new_name)?;

                if local_branch_exists(new_name, &repo_root).await && !force {
                    anyhow::bail!(
                        "A branch named '{new_name}' already exists. Pass 'force: true' to overwrite it."
                    );
                }

                let flag = if force { "-M" } else { "-m" };
                let output = if let Some(old_name) = branch_opt {
                    validate_branch_name(old_name)?;
                    if !local_branch_exists(old_name, &repo_root).await {
                        anyhow::bail!("Source branch '{old_name}' does not exist.");
                    }
                    run_git_command(&["branch", flag, old_name, new_name], &repo_root, 10).await?
                } else {
                    // Rename current branch
                    run_git_command(&["branch", flag, new_name], &repo_root, 10).await?
                };

                if !output.success {
                    let err = output.stderr.trim();
                    anyhow::bail!("Failed to rename branch: {err}");
                }

                let res = BranchOpResult {
                    action: "rename".to_string(),
                    branch: new_name.to_string(),
                    success: true,
                    message: if let Some(old) = branch_opt {
                        format!("Renamed branch '{old}' to '{new_name}'.")
                    } else {
                        format!("Renamed current branch to '{new_name}'.")
                    },
                    previous_branch: branch_opt.map(|s| s.to_string()),
                    commit: None,
                };

                if json_output {
                    Ok(serde_json::to_string_pretty(&res)?)
                } else {
                    Ok(format!("{}\n", res.message))
                }
            }

            // ----------------------------------------------------------------
            // 6. Current Status & Inspection
            // ----------------------------------------------------------------
            "current" | "status" | "show" => {
                let (current_branch, is_detached, detached_commit, detached_subject) =
                    get_current_head_info(&repo_root).await?;

                let is_dirty = check_working_tree_dirty(&repo_root).await.unwrap_or(false);

                // Worktree inspection: check if target_dir is inside a linked worktree
                let is_linked_worktree = target_dir.join(".git").is_file();
                let worktree_report = list_worktrees(&repo_root).await.ok();
                let worktree_count = worktree_report.as_ref().map(|w| w.total_count).unwrap_or(1);

                if is_detached {
                    let hash = detached_commit.as_deref().unwrap_or("unknown");
                    let subj = detached_subject.as_deref().unwrap_or("No subject");
                    if json_output {
                        Ok(serde_json::to_string_pretty(&json!({
                            "current_branch": null,
                            "is_detached": true,
                            "commit": hash,
                            "subject": subj,
                            "is_dirty": is_dirty,
                            "is_linked_worktree": is_linked_worktree,
                            "worktree_count": worktree_count,
                            "repo_root": repo_root.display().to_string(),
                        }))?)
                    } else {
                        let mut msg = format!("HEAD is detached at commit {hash}: {subj}\n");
                        if is_dirty {
                            msg.push_str("Working directory: DIRTY (uncommitted changes present)\n");
                        } else {
                            msg.push_str("Working directory: CLEAN\n");
                        }
                        if is_linked_worktree {
                            msg.push_str(&format!(
                                "Worktree context:  Linked worktree (main repo at '{}')\n",
                                repo_root.display()
                            ));
                        }
                        Ok(msg)
                    }
                } else if let Some(curr) = current_branch {
                    // Get commit info
                    let commit_out = run_git_command(
                        &["log", "-1", "--format=%h|%s|%cr", &curr],
                        &repo_root,
                        5,
                    )
                    .await?;

                    let parts: Vec<&str> = commit_out.stdout.trim().splitn(3, '|').collect();
                    let hash = parts.first().copied().unwrap_or("");
                    let subject = parts.get(1).copied().unwrap_or("");
                    let date = parts.get(2).copied().unwrap_or("");

                    // Get upstream tracking info
                    let track_out = run_git_command(
                        &[
                            "for-each-ref",
                            "--format=%(upstream:short)|%(upstream:track)",
                            &format!("refs/heads/{curr}"),
                        ],
                        &repo_root,
                        5,
                    )
                    .await?;

                    let track_parts: Vec<&str> = track_out.stdout.trim().splitn(2, '|').collect();
                    let upstream = if track_parts.first().map(|s| s.is_empty()).unwrap_or(true) {
                        None
                    } else {
                        Some(track_parts[0].to_string())
                    };
                    let (ahead, behind, is_gone) = if track_parts.len() > 1 {
                        parse_upstream_tracking(track_parts[1])
                    } else {
                        (0, 0, false)
                    };

                    if json_output {
                        Ok(serde_json::to_string_pretty(&json!({
                            "current_branch": curr,
                            "is_detached": false,
                            "commit": hash,
                            "subject": subject,
                            "date": date,
                            "upstream": upstream,
                            "ahead": ahead,
                            "behind": behind,
                            "is_gone": is_gone,
                            "is_dirty": is_dirty,
                            "is_linked_worktree": is_linked_worktree,
                            "worktree_count": worktree_count,
                            "repo_root": repo_root.display().to_string(),
                        }))?)
                    } else {
                        let mut msg = format!("Current branch:    {curr}\n");
                        if !hash.is_empty() {
                            msg.push_str(&format!("Latest commit:     {hash} - {subject} ({date})\n"));
                        }
                        if let Some(up) = upstream {
                            if is_gone {
                                msg.push_str(&format!("Upstream:          {up} [gone]\n"));
                            } else if ahead > 0 || behind > 0 {
                                msg.push_str(&format!(
                                    "Upstream:          {up} [ahead {ahead}, behind {behind}]\n"
                                ));
                            } else {
                                msg.push_str(&format!("Upstream:          {up} [up to date]\n"));
                            }
                        } else {
                            msg.push_str("Upstream:          None (not tracking a remote branch)\n");
                        }

                        if is_dirty {
                            msg.push_str("Working directory: DIRTY (uncommitted changes present)\n");
                        } else {
                            msg.push_str("Working directory: CLEAN\n");
                        }

                        if is_linked_worktree {
                            msg.push_str(&format!(
                                "Worktree context:  Linked worktree (main repo at '{}')\n",
                                repo_root.display()
                            ));
                        } else if worktree_count > 1 {
                            msg.push_str(&format!(
                                "Worktrees:         {} active worktrees configured\n",
                                worktree_count
                            ));
                        }

                        Ok(msg)
                    }
                } else {
                    Ok("No current branch (empty repository).\n".to_string())
                }
            }

            // ----------------------------------------------------------------
            // 7. Upstream Setting
            // ----------------------------------------------------------------
            "upstream" | "track" | "set_upstream" => {
                let unset = args.get("unset").and_then(|v| v.as_bool()).unwrap_or(false);
                let branch_name = if let Some(b) = branch_opt {
                    b.to_string()
                } else {
                    let (curr, _, _, _) = get_current_head_info(&repo_root).await?;
                    curr.ok_or_else(|| {
                        anyhow::anyhow!("No active branch found. Specify 'branch' parameter.")
                    })?
                };

                if unset {
                    let output = run_git_command(
                        &["branch", "--unset-upstream", &branch_name],
                        &repo_root,
                        10,
                    )
                    .await?;
                    if !output.success {
                        let err = output.stderr.trim();
                        anyhow::bail!("Failed to unset upstream for branch '{branch_name}': {err}");
                    }
                    Ok(format!("Unset upstream for branch '{branch_name}'.\n"))
                } else {
                    let upstream_target = args
                        .get("upstream")
                        .and_then(|v| v.as_str())
                        .map(|s| s.trim())
                        .ok_or_else(|| {
                            anyhow::anyhow!("'upstream' parameter is required to set upstream tracking (e.g. 'origin/main')")
                        })?;

                    let set_arg = format!("--set-upstream-to={upstream_target}");
                    let output = run_git_command(
                        &["branch", &set_arg, &branch_name],
                        &repo_root,
                        10,
                    )
                    .await?;
                    if !output.success {
                        let err = output.stderr.trim();
                        anyhow::bail!(
                            "Failed to set upstream for branch '{branch_name}' to '{upstream_target}': {err}"
                        );
                    }
                    Ok(format!(
                        "Branch '{branch_name}' set up to track remote branch '{upstream_target}'.\n"
                    ))
                }
            }

            // ----------------------------------------------------------------
            // 8. Create Worktree
            // ----------------------------------------------------------------
            "create_worktree" | "worktree_create" | "worktree_add" | "add_worktree" => {
                let wt_path_str = worktree_opt.ok_or_else(|| {
                    anyhow::anyhow!("'worktree_path' is required for creating a worktree (provide 'worktree_path' or 'worktree')")
                })?;

                let resolved_wt_path = resolve_path(wt_path_str, &ctx.cwd);

                let create_branch = args
                    .get("create")
                    .or_else(|| args.get("create_branch"))
                    .or_else(|| args.get("new_branch"))
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false);

                let detach = args
                    .get("detach")
                    .or_else(|| args.get("detached"))
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false);

                let result = create_worktree(
                    &repo_root,
                    &resolved_wt_path,
                    branch_opt,
                    create_branch,
                    from_opt,
                    force,
                    detach,
                )
                .await?;

                if json_output {
                    Ok(serde_json::to_string_pretty(&result)?)
                } else {
                    Ok(format!("{}\n", result.message))
                }
            }

            // ----------------------------------------------------------------
            // 9. Remove Worktree
            // ----------------------------------------------------------------
            "remove_worktree" | "worktree_remove" | "worktree_delete" | "delete_worktree" => {
                let wt_target = worktree_opt
                    .or(branch_opt)
                    .ok_or_else(|| {
                        anyhow::anyhow!("'worktree_path' or 'branch' is required to identify the worktree to remove")
                    })?;

                let result = remove_worktree(&repo_root, wt_target, force).await?;

                if json_output {
                    Ok(serde_json::to_string_pretty(&result)?)
                } else {
                    Ok(format!("{}\n", result.message))
                }
            }

            // ----------------------------------------------------------------
            // 10. List Worktrees
            // ----------------------------------------------------------------
            "list_worktrees" | "worktree_list" | "worktrees" => {
                let report = list_worktrees(&repo_root).await?;

                if json_output {
                    Ok(serde_json::to_string_pretty(&report)?)
                } else {
                    Ok(format_worktree_list(&report))
                }
            }

            // ----------------------------------------------------------------
            // 11. Prune Worktrees
            // ----------------------------------------------------------------
            "prune_worktrees" | "worktree_prune" | "prune" => {
                let expire = args.get("expire").and_then(|v| v.as_str());
                let dry_run = args.get("dry_run").and_then(|v| v.as_bool()).unwrap_or(false);

                let result = prune_worktrees(&repo_root, expire, dry_run).await?;

                if json_output {
                    Ok(serde_json::to_string_pretty(&result)?)
                } else {
                    Ok(format!("{}\n", result.message))
                }
            }

            // ----------------------------------------------------------------
            // 12. Lock Worktree
            // ----------------------------------------------------------------
            "lock_worktree" | "worktree_lock" => {
                let wt_target = worktree_opt
                    .or(branch_opt)
                    .ok_or_else(|| {
                        anyhow::anyhow!("'worktree_path' is required to specify worktree to lock")
                    })?;

                let reason = args.get("reason").and_then(|v| v.as_str());

                let result = lock_worktree(&repo_root, wt_target, reason).await?;

                if json_output {
                    Ok(serde_json::to_string_pretty(&result)?)
                } else {
                    Ok(format!("{}\n", result.message))
                }
            }

            // ----------------------------------------------------------------
            // 13. Unlock Worktree
            // ----------------------------------------------------------------
            "unlock_worktree" | "worktree_unlock" => {
                let wt_target = worktree_opt
                    .or(branch_opt)
                    .ok_or_else(|| {
                        anyhow::anyhow!("'worktree_path' is required to specify worktree to unlock")
                    })?;

                let result = unlock_worktree(&repo_root, wt_target).await?;

                if json_output {
                    Ok(serde_json::to_string_pretty(&result)?)
                } else {
                    Ok(format!("{}\n", result.message))
                }
            }

            other => {
                anyhow::bail!(
                    "Unknown action '{other}'. Supported actions: 'list', 'create', 'switch' (checkout), 'delete', 'rename', 'current' (status), 'upstream', 'create_worktree', 'remove_worktree', 'list_worktrees', 'prune_worktrees', 'lock_worktree', 'unlock_worktree'."
                );
            }
        }
    }
}

// ============================================================================
// Unit & Integration Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::process::Command;

    struct TempGitRepo {
        path: PathBuf,
    }

    impl TempGitRepo {
        fn new() -> Self {
            let id = uuid::Uuid::new_v4();
            let mut path = std::env::temp_dir();
            path.push(format!("fusion_git_branch_test_{id}"));
            fs::create_dir_all(&path).expect("failed to create temp git dir");

            // Initialize git repository
            let status = Command::new("git")
                .arg("init")
                .current_dir(&path)
                .status()
                .expect("failed to execute git init");
            assert!(status.success(), "git init must succeed");

            // Configure git user
            Command::new("git")
                .args(["config", "user.name", "Fusion Branch Test"])
                .current_dir(&path)
                .status()
                .expect("git config user.name");
            Command::new("git")
                .args(["config", "user.email", "branch_test@fusion.local"])
                .current_dir(&path)
                .status()
                .expect("git config user.email");

            // Initial commit so HEAD is valid
            let init_file = path.join("README.md");
            fs::write(&init_file, "# Test Repo\n").expect("write readme");
            Command::new("git")
                .args(["add", "README.md"])
                .current_dir(&path)
                .status()
                .expect("git add");
            Command::new("git")
                .args(["commit", "-m", "Initial commit"])
                .current_dir(&path)
                .status()
                .expect("git commit");

            Self { path }
        }
    }

    impl Drop for TempGitRepo {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.path);
        }
    }

    #[test]
    fn test_validate_branch_name() {
        assert!(validate_branch_name("main").is_ok());
        assert!(validate_branch_name("feature/login-system").is_ok());
        assert!(validate_branch_name("bugfix/issue_123").is_ok());
        assert!(validate_branch_name("release-v1.0.0").is_ok());

        assert!(validate_branch_name("").is_err());
        assert!(validate_branch_name("   ").is_err());
        assert!(validate_branch_name("/starts-with-slash").is_err());
        assert!(validate_branch_name("ends-with-slash/").is_err());
        assert!(validate_branch_name("consecutive//slashes").is_err());
        assert!(validate_branch_name(".starts-with-dot").is_err());
        assert!(validate_branch_name("ends-with-dot.").is_err());
        assert!(validate_branch_name("ends-with.lock").is_err());
        assert!(validate_branch_name("double..dots").is_err());
        assert!(validate_branch_name("branch with space").is_err());
        assert!(validate_branch_name("bad~char").is_err());
        assert!(validate_branch_name("bad^char").is_err());
        assert!(validate_branch_name("bad:char").is_err());
        assert!(validate_branch_name("bad?char").is_err());
        assert!(validate_branch_name("bad*char").is_err());
        assert!(validate_branch_name("bad[char").is_err());
        assert!(validate_branch_name("bad\\char").is_err());
        assert!(validate_branch_name("@{upstream}").is_err());
        assert!(validate_branch_name("@").is_err());
    }

    #[test]
    fn test_parse_upstream_tracking() {
        let (ahead, behind, gone) = parse_upstream_tracking("[ahead 2, behind 1]");
        assert_eq!(ahead, 2);
        assert_eq!(behind, 1);
        assert!(!gone);

        let (ahead, behind, gone) = parse_upstream_tracking("[ahead 5]");
        assert_eq!(ahead, 5);
        assert_eq!(behind, 0);
        assert!(!gone);

        let (ahead, behind, gone) = parse_upstream_tracking("[behind 3]");
        assert_eq!(ahead, 0);
        assert_eq!(behind, 3);
        assert!(!gone);

        let (ahead, behind, gone) = parse_upstream_tracking("[gone]");
        assert_eq!(ahead, 0);
        assert_eq!(behind, 0);
        assert!(gone);

        let (ahead, behind, gone) = parse_upstream_tracking("");
        assert_eq!(ahead, 0);
        assert_eq!(behind, 0);
        assert!(!gone);
    }

    #[test]
    fn test_parse_worktree_porcelain() {
        let sample = r#"
worktree /Users/alice/projects/fusion
HEAD a1b2c3d4e5f60718293a4b5c6d7e8f9012345678
branch refs/heads/main

worktree /Users/alice/projects/fusion-feat
HEAD 1234567890abcdef1234567890abcdef12345678
branch refs/heads/feature/auth

worktree /Users/alice/projects/fusion-detached
HEAD 9876543210fedcba9876543210fedcba98765432
detached

worktree /Users/alice/projects/fusion-locked
HEAD 5555555555555555555555555555555555555555
branch refs/heads/release
locked on review

worktree /Users/alice/projects/fusion-prunable
HEAD 6666666666666666666666666666666666666666
branch refs/heads/scratch
prunable directory removed
"#;

        let root = Path::new("/Users/alice/projects/fusion");
        let worktrees = parse_worktree_porcelain(sample, root);

        assert_eq!(worktrees.len(), 5);

        // First worktree: main
        assert_eq!(worktrees[0].path, "/Users/alice/projects/fusion");
        assert_eq!(worktrees[0].branch.as_deref(), Some("main"));
        assert!(worktrees[0].is_main);
        assert!(!worktrees[0].is_detached);
        assert_eq!(worktrees[0].head_commit, "a1b2c3d");

        // Second: feature/auth
        assert_eq!(worktrees[1].path, "/Users/alice/projects/fusion-feat");
        assert_eq!(worktrees[1].branch.as_deref(), Some("feature/auth"));
        assert!(!worktrees[1].is_main);

        // Third: detached
        assert_eq!(worktrees[2].path, "/Users/alice/projects/fusion-detached");
        assert!(worktrees[2].is_detached);
        assert_eq!(worktrees[2].branch, None);

        // Fourth: locked
        assert_eq!(worktrees[3].path, "/Users/alice/projects/fusion-locked");
        assert!(worktrees[3].is_locked);
        assert_eq!(worktrees[3].lock_reason.as_deref(), Some("on review"));

        // Fifth: prunable
        assert_eq!(worktrees[4].path, "/Users/alice/projects/fusion-prunable");
        assert!(worktrees[4].is_prunable);
        assert_eq!(worktrees[4].prune_reason.as_deref(), Some("directory removed"));
    }

    #[test]
    fn test_format_worktree_list() {
        let report = WorktreeListReport {
            worktrees: vec![
                WorktreeInfo {
                    path: "/fusion".to_string(),
                    head_commit: "a1b2c3d".to_string(),
                    branch: Some("main".to_string()),
                    is_detached: false,
                    is_bare: false,
                    is_locked: false,
                    lock_reason: None,
                    is_prunable: false,
                    prune_reason: None,
                    is_main: true,
                },
                WorktreeInfo {
                    path: "/fusion-wt1".to_string(),
                    head_commit: "fedcba9".to_string(),
                    branch: Some("feature/xyz".to_string()),
                    is_detached: false,
                    is_bare: false,
                    is_locked: true,
                    lock_reason: Some("testing".to_string()),
                    is_prunable: false,
                    prune_reason: None,
                    is_main: false,
                },
            ],
            repo_root: "/fusion".to_string(),
            total_count: 2,
        };

        let formatted = format_worktree_list(&report);
        assert!(formatted.contains("* [main]"));
        assert!(formatted.contains("branch: main"));
        assert!(formatted.contains("branch: feature/xyz"));
        assert!(formatted.contains("locked (testing)"));
    }

    #[tokio::test]
    async fn test_git_branch_list_and_current() {
        let repo = TempGitRepo::new();
        let tool = GitBranchTool::new();
        let ctx = ToolContext {
            cwd: repo.path.clone(),
            env: Default::default(),
        };

        // Current branch
        let cur_res = tool
            .execute(json!({"action": "current"}), &ctx)
            .await
            .unwrap();
        assert!(cur_res.contains("Current branch:"));

        // List branches
        let list_res = tool
            .execute(json!({"action": "list"}), &ctx)
            .await
            .unwrap();
        assert!(list_res.contains("* "));
        assert!(list_res.contains("Initial commit"));

        // List branches as JSON
        let json_res = tool
            .execute(json!({"action": "list", "json": true}), &ctx)
            .await
            .unwrap();
        let report: BranchListReport = serde_json::from_str(&json_res).unwrap();
        assert!(!report.branches.is_empty());
        assert!(report.branches.iter().any(|b| b.is_current));
    }

    #[tokio::test]
    async fn test_git_branch_create_and_switch() {
        let repo = TempGitRepo::new();
        let tool = GitBranchTool::new();
        let ctx = ToolContext {
            cwd: repo.path.clone(),
            env: Default::default(),
        };

        // 1. Create a branch without switching
        let create_res = tool
            .execute(
                json!({
                    "action": "create",
                    "branch": "feature/alpha"
                }),
                &ctx,
            )
            .await
            .unwrap();
        assert!(create_res.contains("Created branch 'feature/alpha'"));

        // Verify alpha exists
        assert!(local_branch_exists("feature/alpha", &repo.path).await);

        // 2. Creating duplicate fails
        let dup_err = tool
            .execute(
                json!({
                    "action": "create",
                    "branch": "feature/alpha"
                }),
                &ctx,
            )
            .await;
        assert!(dup_err.is_err());
        assert!(dup_err.unwrap_err().to_string().contains("already exists"));

        // 3. Switch to feature/alpha
        let switch_res = tool
            .execute(
                json!({
                    "action": "switch",
                    "branch": "feature/alpha"
                }),
                &ctx,
            )
            .await
            .unwrap();
        assert!(switch_res.contains("Switched to branch 'feature/alpha'"));

        // Verify current branch is feature/alpha
        let cur = tool
            .execute(json!({"action": "current"}), &ctx)
            .await
            .unwrap();
        assert!(cur.contains("Current branch:    feature/alpha"));

        // 4. Create and switch simultaneously
        let create_sw_res = tool
            .execute(
                json!({
                    "action": "create",
                    "branch": "feature/beta",
                    "switch": true
                }),
                &ctx,
            )
            .await
            .unwrap();
        assert!(create_sw_res.contains("Created and switched to new branch 'feature/beta'"));

        let cur_beta = tool
            .execute(json!({"action": "current"}), &ctx)
            .await
            .unwrap();
        assert!(cur_beta.contains("Current branch:    feature/beta"));

        // 5. Switch with create_if_missing: true
        let auto_create_res = tool
            .execute(
                json!({
                    "action": "switch",
                    "branch": "feature/gamma",
                    "create": true
                }),
                &ctx,
            )
            .await
            .unwrap();
        assert!(auto_create_res.contains("created and switched to new branch 'feature/gamma'"));
    }

    #[tokio::test]
    async fn test_git_branch_rename_and_delete() {
        let repo = TempGitRepo::new();
        let tool = GitBranchTool::new();
        let ctx = ToolContext {
            cwd: repo.path.clone(),
            env: Default::default(),
        };

        // Create feature branch
        tool.execute(
            json!({"action": "create", "branch": "temp-feature"}),
            &ctx,
        )
        .await
        .unwrap();

        // Rename branch
        let rename_res = tool
            .execute(
                json!({
                    "action": "rename",
                    "branch": "temp-feature",
                    "new_name": "renamed-feature"
                }),
                &ctx,
            )
            .await
            .unwrap();
        assert!(rename_res.contains("Renamed branch 'temp-feature' to 'renamed-feature'"));
        assert!(!local_branch_exists("temp-feature", &repo.path).await);
        assert!(local_branch_exists("renamed-feature", &repo.path).await);

        // Delete branch
        let del_res = tool
            .execute(
                json!({
                    "action": "delete",
                    "branch": "renamed-feature"
                }),
                &ctx,
            )
            .await
            .unwrap();
        assert!(del_res.contains("Deleted branch 'renamed-feature'"));
        assert!(!local_branch_exists("renamed-feature", &repo.path).await);

        // Safety: Attempting to delete current branch fails
        let (curr_branch, _, _, _) = get_current_head_info(&repo.path).await.unwrap();
        let curr = curr_branch.unwrap();
        let del_curr_err = tool
            .execute(json!({"action": "delete", "branch": curr}), &ctx)
            .await;
        assert!(del_curr_err.is_err());
        assert!(del_curr_err
            .unwrap_err()
            .to_string()
            .contains("Cannot delete the currently checked-out branch"));
    }

    #[tokio::test]
    async fn test_git_branch_switch_safety_with_dirty_tree() {
        let repo = TempGitRepo::new();
        let tool = GitBranchTool::new();
        let ctx = ToolContext {
            cwd: repo.path.clone(),
            env: Default::default(),
        };

        // Create branch-a and commit a file
        tool.execute(
            json!({"action": "create", "branch": "branch-a", "switch": true}),
            &ctx,
        )
        .await
        .unwrap();

        let file_a = repo.path.join("file_a.txt");
        fs::write(&file_a, "version 1 on branch A\n").unwrap();
        Command::new("git")
            .args(["add", "file_a.txt"])
            .current_dir(&repo.path)
            .status()
            .unwrap();
        Command::new("git")
            .args(["commit", "-m", "Add file_a on branch-a"])
            .current_dir(&repo.path)
            .status()
            .unwrap();

        // Create branch-b from initial commit (not containing file_a)
        tool.execute(
            json!({"action": "create", "branch": "branch-b", "from": "HEAD~1"}),
            &ctx,
        )
        .await
        .unwrap();

        // Switch to branch-b
        tool.execute(
            json!({"action": "switch", "branch": "branch-b"}),
            &ctx,
        )
        .await
        .unwrap();

        // Now create file_a.txt untracked/modified on branch-b with conflicting content
        fs::write(&file_a, "conflicting untracked file\n").unwrap();

        // Switching back to branch-a would overwrite local file_a.txt
        let conflict_err = tool
            .execute(
                json!({"action": "switch", "branch": "branch-a"}),
                &ctx,
            )
            .await;
        assert!(conflict_err.is_err());
        let err_str = conflict_err.unwrap_err().to_string();
        assert!(err_str.contains("uncommitted changes") || err_str.contains("overwritten"));

        // Passing force: true succeeds and forces switch
        let force_res = tool
            .execute(
                json!({
                    "action": "switch",
                    "branch": "branch-a",
                    "force": true
                }),
                &ctx,
            )
            .await
            .unwrap();
        assert!(force_res.contains("Switched to branch 'branch-a'"));
    }

    #[tokio::test]
    async fn test_git_worktree_create_list_and_remove() {
        let repo = TempGitRepo::new();
        let tool = GitBranchTool::new();
        let ctx = ToolContext {
            cwd: repo.path.clone(),
            env: Default::default(),
        };

        // Create a secondary worktree path
        let wt_path = std::env::temp_dir().join(format!("fusion_wt_test_{}", uuid::Uuid::new_v4()));

        // 1. Create worktree with a new branch
        let create_res = tool
            .execute(
                json!({
                    "action": "create_worktree",
                    "worktree_path": wt_path.to_str().unwrap(),
                    "branch": "wt-feature",
                    "create": true
                }),
                &ctx,
            )
            .await
            .unwrap();
        assert!(create_res.contains("Created worktree at"));
        assert!(create_res.contains("wt-feature"));

        // 2. List worktrees
        let list_res = tool
            .execute(json!({"action": "list_worktrees"}), &ctx)
            .await
            .unwrap();
        assert!(list_res.contains("* [main]"));
        assert!(list_res.contains("wt-feature"));

        // 3. List worktrees as JSON
        let json_res = tool
            .execute(json!({"action": "list_worktrees", "json": true}), &ctx)
            .await
            .unwrap();
        let report: WorktreeListReport = serde_json::from_str(&json_res).unwrap();
        assert_eq!(report.total_count, 2);
        assert!(report.worktrees.iter().any(|w| w.is_main));
        assert!(report.worktrees.iter().any(|w| w.branch.as_deref() == Some("wt-feature")));

        // 4. Remove worktree
        let remove_res = tool
            .execute(
                json!({
                    "action": "remove_worktree",
                    "worktree_path": wt_path.to_str().unwrap()
                }),
                &ctx,
            )
            .await
            .unwrap();
        assert!(remove_res.contains("Removed worktree at"));

        // Verify worktree list is back to 1
        let list_after = tool
            .execute(json!({"action": "list_worktrees", "json": true}), &ctx)
            .await
            .unwrap();
        let report_after: WorktreeListReport = serde_json::from_str(&list_after).unwrap();
        assert_eq!(report_after.total_count, 1);
    }

    #[tokio::test]
    async fn test_git_worktree_safety_cannot_remove_main() {
        let repo = TempGitRepo::new();
        let tool = GitBranchTool::new();
        let ctx = ToolContext {
            cwd: repo.path.clone(),
            env: Default::default(),
        };

        let rem_err = tool
            .execute(
                json!({
                    "action": "remove_worktree",
                    "worktree_path": repo.path.to_str().unwrap()
                }),
                &ctx,
            )
            .await;

        assert!(rem_err.is_err());
        assert!(rem_err.unwrap_err().to_string().contains("Cannot remove main worktree"));
    }

    #[tokio::test]
    async fn test_git_status_action() {
        let repo = TempGitRepo::new();
        let tool = GitBranchTool::new();
        let ctx = ToolContext {
            cwd: repo.path.clone(),
            env: Default::default(),
        };

        let status_res = tool
            .execute(json!({"action": "status"}), &ctx)
            .await
            .unwrap();
        assert!(status_res.contains("Current branch:"));
        assert!(status_res.contains("Working directory: CLEAN"));

        let status_json = tool
            .execute(json!({"action": "status", "json": true}), &ctx)
            .await
            .unwrap();
        let val: Value = serde_json::from_str(&status_json).unwrap();
        assert_eq!(val["is_dirty"], false);
        assert_eq!(val["is_detached"], false);
    }
}
