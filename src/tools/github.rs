//! GitHub Inspection Tool.
//!
//! Provides inspection and analysis of GitHub Pull Requests, Issues, Repositories,
//! Releases, and CI Actions Workflow Runs via the `gh` CLI, GitHub REST API, and
//! local Git metadata.
//!
//! Features:
//! - **PR Inspection**: List PRs with filters (state, author, assignee, base, head, labels, search, draft),
//!   inspect full PR details (body, reviews, status checks, changed files, mergeability, diff statistics),
//!   retrieve PR diffs, commits, and review comments.
//! - **Issue Inspection**: List and view issues with labels, milestones, assignees, and comment threads.
//! - **Repository Metadata**: Inspect repository info (stars, forks, open issues, license, default branch, topics).
//! - **Release Inspection**: List and inspect releases, release assets, and release notes.
//! - **Workflow Runs**: Inspect recent GitHub Actions workflow runs, statuses, conclusions, and branch triggers.
//! - **Multiple Execution Backends**:
//!   1. `gh` CLI (Primary): Structured JSON extraction via `gh` CLI commands.
//!   2. GitHub REST API (Fallback): Direct HTTP queries via `reqwest` if `gh` is unavailable.
//!   3. Local Git Inspection (Offline): Remote parsing, branch tracking, and commit inspection.
//! - **Output Formats**: Clean structured JSON (default) or formatted Markdown summary.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::tools::file::resolve_path;
use crate::tools::git::{find_git_root, run_git_command, GitProcessOutput};
use crate::tools::types::{Tool, ToolContext};

// ============================================================================
// Data Models & Structures
// ============================================================================

/// Represents a GitHub user (author, assignee, reviewer, merged_by).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct GitHubUser {
    /// GitHub login / username (e.g. "octocat").
    pub login: String,
    /// Display name if available.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// Avatar URL.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub avatar_url: Option<String>,
    /// Web profile URL.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
}

impl GitHubUser {
    pub fn new(login: impl Into<String>) -> Self {
        Self {
            login: login.into(),
            name: None,
            avatar_url: None,
            url: None,
        }
    }
}

/// Represents a GitHub issue or PR label.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct GitHubLabel {
    /// Label name (e.g. "bug", "enhancement").
    pub name: String,
    /// Hex color code without '#' (e.g. "d73a4a").
    #[serde(skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
    /// Description of the label.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

/// Represents a GitHub milestone.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct GitHubMilestone {
    /// Milestone title.
    pub title: String,
    /// Milestone number.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub number: Option<u64>,
    /// State ("open", "closed").
    #[serde(skip_serializing_if = "Option::is_none")]
    pub state: Option<String>,
    /// Milestone description.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// Due date ISO string.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub due_on: Option<String>,
}

/// Represents a CI Check Run or Status Check.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct GitHubCheckRun {
    /// Name of the check run or workflow context (e.g. "ci / build (macos-14)").
    pub name: String,
    /// Status: "COMPLETED", "IN_PROGRESS", "QUEUED", "PENDING".
    pub status: String,
    /// Conclusion: "SUCCESS", "FAILURE", "NEUTRAL", "CANCELLED", "SKIPPED", "TIMED_OUT", "ACTION_REQUIRED".
    #[serde(skip_serializing_if = "Option::is_none")]
    pub conclusion: Option<String>,
    /// Link to build logs / details.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub details_url: Option<String>,
    /// Start timestamp.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub started_at: Option<String>,
    /// Completion timestamp.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub completed_at: Option<String>,
}

/// Represents a single file changed in a pull request.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct GitHubFileChange {
    /// File path.
    pub path: String,
    /// Number of added lines.
    pub additions: usize,
    /// Number of deleted lines.
    pub deletions: usize,
    /// Status: "added", "modified", "deleted", "renamed".
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
    /// Patch/diff hunk if available.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub patch: Option<String>,
}

/// Represents a single commit in a pull request.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct GitHubCommit {
    /// Git commit SHA.
    pub oid: String,
    /// Headline / subject line.
    pub message_headline: String,
    /// Detailed commit message body.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message_body: Option<String>,
    /// Authored ISO timestamp.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub authored_date: Option<String>,
    /// Authors list.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub authors: Vec<GitHubUser>,
}

/// Represents a pull request review.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct GitHubReview {
    /// Review ID.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    /// Review author.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub author: Option<GitHubUser>,
    /// Review state: "APPROVED", "CHANGES_REQUESTED", "COMMENTED", "DISMISSED", "PENDING".
    pub state: String,
    /// Review body text.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub body: Option<String>,
    /// Submission timestamp.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub submitted_at: Option<String>,
    /// Commit ID at the time of review.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub commit_id: Option<String>,
}

/// Represents a comment on an issue or PR.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct GitHubComment {
    /// Comment ID.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    /// Comment author.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub author: Option<GitHubUser>,
    /// Comment markdown body.
    pub body: String,
    /// Creation timestamp.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub created_at: Option<String>,
    /// Update timestamp.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub updated_at: Option<String>,
    /// Web URL.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
}

/// Comprehensive representation of a GitHub Pull Request.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct GitHubPr {
    /// PR Number.
    pub number: u64,
    /// PR Title.
    pub title: String,
    /// PR Body / Description.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub body: Option<String>,
    /// State: "OPEN", "CLOSED", "MERGED".
    pub state: String,
    /// Whether this PR is a draft.
    pub is_draft: bool,
    /// Web URL to the pull request.
    pub url: String,
    /// Author user.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub author: Option<GitHubUser>,
    /// Base branch name (e.g. "main").
    #[serde(skip_serializing_if = "Option::is_none")]
    pub base_ref_name: Option<String>,
    /// Head branch name (e.g. "feat/auth").
    #[serde(skip_serializing_if = "Option::is_none")]
    pub head_ref_name: Option<String>,
    /// Head commit SHA.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub head_sha: Option<String>,
    /// Total added lines.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub additions: Option<usize>,
    /// Total deleted lines.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub deletions: Option<usize>,
    /// Number of changed files.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub changed_files: Option<usize>,
    /// Attached labels.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub labels: Vec<GitHubLabel>,
    /// Assigned users.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub assignees: Vec<GitHubUser>,
    /// Requested reviewers.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub reviewers: Vec<GitHubUser>,
    /// Milestone.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub milestone: Option<GitHubMilestone>,
    /// Mergeable status: "MERGEABLE", "CONFLICTING", "UNKNOWN".
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mergeable: Option<String>,
    /// Detailed merge state status (e.g. "CLEAN", "DIRTY", "BEHIND", "BLOCKED", "HAS_HOOKS", "UNSTABLE").
    #[serde(skip_serializing_if = "Option::is_none")]
    pub merge_state_status: Option<String>,
    /// Timestamp when merged.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub merged_at: Option<String>,
    /// User who merged the PR.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub merged_by: Option<GitHubUser>,
    /// Creation timestamp.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub created_at: Option<String>,
    /// Update timestamp.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub updated_at: Option<String>,
    /// Closed timestamp.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub closed_at: Option<String>,
    /// Comments count.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub comments_count: Option<usize>,
    /// Reviews count.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reviews_count: Option<usize>,
    /// Associated CI check runs.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub status_checks: Vec<GitHubCheckRun>,
    /// List of modified files.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub files: Vec<GitHubFileChange>,
    /// List of commits.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub commits: Vec<GitHubCommit>,
    /// List of reviews.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub reviews: Vec<GitHubReview>,
    /// List of conversation comments.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub comments: Vec<GitHubComment>,
}

/// Report for a list of Pull Requests.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct GitHubPrListReport {
    /// Target repository ("owner/repo").
    pub repo: String,
    /// Total count of matching pull requests.
    pub total_count: usize,
    /// List of pull requests.
    pub pull_requests: Vec<GitHubPr>,
}

/// Comprehensive representation of a GitHub Issue.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct GitHubIssue {
    /// Issue Number.
    pub number: u64,
    /// Issue Title.
    pub title: String,
    /// Issue Body / Description.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub body: Option<String>,
    /// State: "OPEN", "CLOSED".
    pub state: String,
    /// Web URL to the issue.
    pub url: String,
    /// Author user.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub author: Option<GitHubUser>,
    /// Attached labels.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub labels: Vec<GitHubLabel>,
    /// Assigned users.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub assignees: Vec<GitHubUser>,
    /// Milestone.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub milestone: Option<GitHubMilestone>,
    /// Total comments count.
    pub comments_count: usize,
    /// Creation timestamp.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub created_at: Option<String>,
    /// Update timestamp.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub updated_at: Option<String>,
    /// Closed timestamp.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub closed_at: Option<String>,
    /// Comments on the issue.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub comments: Vec<GitHubComment>,
}

/// Report for a list of GitHub Issues.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct GitHubIssueListReport {
    /// Target repository ("owner/repo").
    pub repo: String,
    /// Total count of matching issues.
    pub total_count: usize,
    /// List of issues.
    pub issues: Vec<GitHubIssue>,
}

/// Information about a GitHub Repository.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct GitHubRepoInfo {
    /// Repository name (e.g. "fusion").
    pub name: String,
    /// Owner name (e.g. "theaungmyatmoe").
    pub owner: String,
    /// Full name ("owner/repo").
    pub full_name: String,
    /// Repository description.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// Default branch (e.g. "main").
    pub default_branch: String,
    /// Visibility: "PUBLIC", "PRIVATE", "INTERNAL".
    pub visibility: String,
    /// Whether the repository is private.
    pub is_private: bool,
    /// Whether the repository is a fork.
    pub is_fork: bool,
    /// Star count.
    pub stars_count: usize,
    /// Fork count.
    pub forks_count: usize,
    /// Open issues and PRs count.
    pub open_issues_count: usize,
    /// License name or key.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub license: Option<String>,
    /// Topic tags.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub topics: Vec<String>,
    /// Web URL.
    pub url: String,
    /// Homepage or documentation URL.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub homepage: Option<String>,
}

/// Asset attached to a GitHub Release.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct GitHubReleaseAsset {
    /// Filename of the asset.
    pub name: String,
    /// Size in bytes.
    pub size: u64,
    /// Total downloads count.
    pub download_count: u64,
    /// Direct download URL.
    pub browser_download_url: String,
}

/// Information about a GitHub Release.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct GitHubRelease {
    /// Tag name (e.g. "v0.3.0").
    pub tag_name: String,
    /// Release title/name.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// Release notes / body.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub body: Option<String>,
    /// Whether this is a draft.
    pub is_draft: bool,
    /// Whether this is a pre-release.
    pub is_prerelease: bool,
    /// Whether this is the latest release.
    pub is_latest: bool,
    /// Publication timestamp.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub published_at: Option<String>,
    /// Web URL.
    pub url: String,
    /// Attached downloadable assets.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub assets: Vec<GitHubReleaseAsset>,
}

/// Report for a list of Releases.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct GitHubReleaseListReport {
    /// Target repository ("owner/repo").
    pub repo: String,
    /// Total count of releases.
    pub total_count: usize,
    /// List of releases.
    pub releases: Vec<GitHubRelease>,
}

/// GitHub Actions Workflow Run.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct GitHubWorkflowRun {
    /// Run ID.
    pub id: u64,
    /// Workflow run title or commit message.
    pub name: String,
    /// Name of the workflow file/definition.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub workflow_name: Option<String>,
    /// Status: "completed", "in_progress", "queued", "waiting".
    pub status: String,
    /// Conclusion: "success", "failure", "neutral", "cancelled", "skipped", "timed_out".
    #[serde(skip_serializing_if = "Option::is_none")]
    pub conclusion: Option<String>,
    /// Triggering branch.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub branch: Option<String>,
    /// Trigger event: "push", "pull_request", "workflow_dispatch", "schedule".
    #[serde(skip_serializing_if = "Option::is_none")]
    pub event: Option<String>,
    /// Triggering commit SHA.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub head_sha: Option<String>,
    /// Creation timestamp.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub created_at: Option<String>,
    /// Web URL to the run.
    pub url: String,
}

/// Report for a list of Actions Workflow Runs.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct GitHubWorkflowRunListReport {
    /// Target repository ("owner/repo").
    pub repo: String,
    /// Total count of runs.
    pub total_count: usize,
    /// List of runs.
    pub runs: Vec<GitHubWorkflowRun>,
}

/// Top-level structured response payload returned by GitHubTool.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GitHubToolResponse {
    /// The action executed.
    pub action: String,
    /// Whether the operation succeeded.
    pub success: bool,
    /// Target repository identifier ("owner/repo").
    #[serde(skip_serializing_if = "Option::is_none")]
    pub repo: Option<String>,
    /// Active backend used: "gh_cli", "github_api", "git_local", "mock".
    pub backend: String,
    /// Structured data payload.
    pub data: Value,
    /// Status or error message.
    pub message: String,
    /// Human-readable summary if requested.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
}

// ============================================================================
// Git Remote & Repo Extraction
// ============================================================================

/// Parses owner and repo name from any GitHub remote URL format.
///
/// Supports:
/// - `git@github.com:owner/repo.git`
/// - `https://github.com/owner/repo.git`
/// - `http://github.com/owner/repo`
/// - `ssh://git@github.com/owner/repo.git`
/// - `github.com/owner/repo`
/// - `https://api.github.com/repos/owner/repo`
/// - GitHub Enterprise URLs (e.g. `git@ghe.mycompany.com:owner/repo.git`)
pub fn parse_github_repo_from_remote(remote_url: &str) -> Option<(String, String)> {
    let raw = remote_url.trim();
    if raw.is_empty() {
        return None;
    }

    // Strip .git suffix if present
    let trimmed = raw.strip_suffix(".git").unwrap_or(raw);

    // Strip fragment or query parameters
    let base = match trimmed.split_once('?') {
        Some((b, _)) => b,
        None => trimmed,
    };
    let base = match base.split_once('#') {
        Some((b, _)) => b,
        None => base,
    };

    // 1. SSH format: git@github.com:owner/repo or user@host:owner/repo
    if let Some((_host_part, path_part)) = base.split_once(':') {
        if !base.starts_with("http://") && !base.starts_with("https://") && !base.starts_with("ssh://") {
            let parts: Vec<&str> = path_part.trim_matches('/').split('/').collect();
            if parts.len() >= 2 {
                let owner = parts[parts.len() - 2].trim();
                let repo = parts[parts.len() - 1].trim();
                if !owner.is_empty() && !repo.is_empty() {
                    return Some((owner.to_string(), repo.to_string()));
                }
            }
        }
    }

    // 2. Standard URL formats: https://..., ssh://..., git://...
    let clean_url = base
        .strip_prefix("https://")
        .or_else(|| base.strip_prefix("http://"))
        .or_else(|| base.strip_prefix("ssh://"))
        .or_else(|| base.strip_prefix("git://"))
        .unwrap_or(base);

    // Strip username@ if present in URL (e.g. git@github.com/owner/repo)
    let path_only = match clean_url.split_once('@') {
        Some((_, p)) => p,
        None => clean_url,
    };

    // Split into path components
    let mut parts: Vec<&str> = path_only.split('/').filter(|s| !s.is_empty()).collect();
    if parts.is_empty() {
        return None;
    }

    // If first element is a domain (e.g. "github.com", "api.github.com"), skip domain
    if parts[0].contains('.') {
        parts.remove(0);
    }

    // If path starts with "repos" (e.g. from api.github.com/repos/owner/repo), skip it
    if !parts.is_empty() && parts[0] == "repos" {
        parts.remove(0);
    }

    if parts.len() >= 2 {
        let owner = parts[parts.len() - 2].trim();
        let repo = parts[parts.len() - 1].trim();
        if !owner.is_empty() && !repo.is_empty() {
            return Some((owner.to_string(), repo.to_string()));
        }
    }

    None
}

/// Attempts to detect GitHub owner/repo from current Git working directory.
pub async fn detect_repo_from_cwd(cwd: &Path) -> Option<(String, String)> {
    let repo_root = find_git_root(cwd)?;

    // Try reading remote.origin.url from git config
    let out = run_git_command(&["config", "--get", "remote.origin.url"], &repo_root, 5).await.ok()?;
    if out.success && !out.stdout.trim().is_empty() {
        if let Some(pair) = parse_github_repo_from_remote(out.stdout.trim()) {
            return Some(pair);
        }
    }

    // Try reading all remotes
    let remotes_out = run_git_command(&["remote", "-v"], &repo_root, 5).await.ok()?;
    if remotes_out.success {
        for line in remotes_out.stdout.lines() {
            let parts: Vec<&str> = line.split_whitespace().collect();
            if parts.len() >= 2 {
                if let Some(pair) = parse_github_repo_from_remote(parts[1]) {
                    return Some(pair);
                }
            }
        }
    }

    None
}

// ============================================================================
// Process Execution (gh CLI & fallback)
// ============================================================================

/// Executes a `gh` command in the given directory with timeout.
pub async fn run_gh_command(
    args: &[&str],
    cwd: &Path,
    env: &HashMap<String, String>,
    timeout_secs: u64,
) -> anyhow::Result<GitProcessOutput> {
    let mut cmd = tokio::process::Command::new("gh");
    cmd.current_dir(cwd);
    cmd.args(args);
    cmd.stdin(std::process::Stdio::null());
    cmd.stdout(std::process::Stdio::piped());
    cmd.stderr(std::process::Stdio::piped());
    cmd.kill_on_drop(true);

    // Forward environment (tokens, auth, config)
    for (k, v) in env {
        cmd.env(k, v);
    }

    let child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            anyhow::bail!(
                "GitHub CLI ('gh') not found in PATH. Please install gh or ensure it is accessible."
            );
        }
        Err(e) => {
            anyhow::bail!("Failed to spawn 'gh' process: {e}");
        }
    };

    let timeout_duration = Duration::from_secs(timeout_secs.max(1));
    let output = match tokio::time::timeout(timeout_duration, child.wait_with_output()).await {
        Ok(Ok(out)) => out,
        Ok(Err(e)) => anyhow::bail!("Failed reading 'gh' output: {e}"),
        Err(_) => anyhow::bail!("'gh' command timed out after {timeout_secs}s: gh {}", args.join(" ")),
    };

    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();
    let exit_code = output.status.code();
    let success = output.status.success();

    Ok(GitProcessOutput {
        stdout,
        stderr,
        exit_code,
        success,
    })
}

/// Checks whether `gh` CLI is installed and responsive.
pub async fn is_gh_cli_available(cwd: &Path) -> bool {
    let mut cmd = tokio::process::Command::new("gh");
    cmd.current_dir(cwd);
    cmd.arg("--version");
    cmd.stdin(std::process::Stdio::null());
    cmd.stdout(std::process::Stdio::null());
    cmd.stderr(std::process::Stdio::null());
    cmd.kill_on_drop(true);

    match tokio::time::timeout(Duration::from_secs(3), cmd.status()).await {
        Ok(Ok(status)) => status.success(),
        _ => false,
    }
}

// ============================================================================
// JSON Deserialization & Normalization Helpers
// ============================================================================

fn parse_user_from_val(val: &Value) -> Option<GitHubUser> {
    if val.is_null() {
        return None;
    }
    let login = val.get("login")?.as_str()?.to_string();
    let name = val.get("name").and_then(|v| v.as_str()).map(|s| s.to_string());
    let avatar_url = val
        .get("avatarUrl")
        .or_else(|| val.get("avatar_url"))
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    let url = val
        .get("url")
        .or_else(|| val.get("html_url"))
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    Some(GitHubUser {
        login,
        name,
        avatar_url,
        url,
    })
}

fn parse_labels_from_val(val: &Value) -> Vec<GitHubLabel> {
    let mut labels = Vec::new();
    if let Some(arr) = val.as_array() {
        for item in arr {
            if let Some(name) = item.get("name").and_then(|v| v.as_str()) {
                let color = item.get("color").and_then(|v| v.as_str()).map(|s| s.to_string());
                let description = item.get("description").and_then(|v| v.as_str()).map(|s| s.to_string());
                labels.push(GitHubLabel {
                    name: name.to_string(),
                    color,
                    description,
                });
            }
        }
    }
    labels
}

fn parse_milestone_from_val(val: &Value) -> Option<GitHubMilestone> {
    if val.is_null() {
        return None;
    }
    let title = val.get("title")?.as_str()?.to_string();
    let number = val.get("number").and_then(|v| v.as_u64());
    let state = val.get("state").and_then(|v| v.as_str()).map(|s| s.to_string());
    let description = val.get("description").and_then(|v| v.as_str()).map(|s| s.to_string());
    let due_on = val
        .get("dueOn")
        .or_else(|| val.get("due_on"))
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    Some(GitHubMilestone {
        title,
        number,
        state,
        description,
        due_on,
    })
}

fn parse_users_list_from_val(val: &Value) -> Vec<GitHubUser> {
    let mut users = Vec::new();
    if let Some(arr) = val.as_array() {
        for item in arr {
            if let Some(u) = parse_user_from_val(item) {
                users.push(u);
            }
        }
    }
    users
}

fn parse_status_checks_from_val(val: &Value) -> Vec<GitHubCheckRun> {
    let mut checks = Vec::new();
    // GH CLI format has `statusCheckRollup` array or object
    if let Some(arr) = val.as_array() {
        for item in arr {
            let name = item
                .get("name")
                .or_else(|| item.get("context"))
                .and_then(|v| v.as_str())
                .unwrap_or("check")
                .to_string();

            let status = item
                .get("status")
                .or_else(|| item.get("state"))
                .and_then(|v| v.as_str())
                .unwrap_or("UNKNOWN")
                .to_string();

            let conclusion = item
                .get("conclusion")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());

            let details_url = item
                .get("detailsUrl")
                .or_else(|| item.get("target_url"))
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());

            let started_at = item
                .get("startedAt")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());

            let completed_at = item
                .get("completedAt")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());

            checks.push(GitHubCheckRun {
                name,
                status,
                conclusion,
                details_url,
                started_at,
                completed_at,
            });
        }
    }
    checks
}

fn parse_files_from_val(val: &Value) -> Vec<GitHubFileChange> {
    let mut files = Vec::new();
    if let Some(arr) = val.as_array() {
        for item in arr {
            let path = item
                .get("path")
                .or_else(|| item.get("filename"))
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();

            if path.is_empty() {
                continue;
            }

            let additions = item.get("additions").and_then(|v| v.as_u64()).unwrap_or(0) as usize;
            let deletions = item.get("deletions").and_then(|v| v.as_u64()).unwrap_or(0) as usize;
            let status = item.get("status").and_then(|v| v.as_str()).map(|s| s.to_string());
            let patch = item.get("patch").and_then(|v| v.as_str()).map(|s| s.to_string());

            files.push(GitHubFileChange {
                path,
                additions,
                deletions,
                status,
                patch,
            });
        }
    }
    files
}

fn parse_commits_from_val(val: &Value) -> Vec<GitHubCommit> {
    let mut commits = Vec::new();
    if let Some(arr) = val.as_array() {
        for item in arr {
            let oid = item
                .get("oid")
                .or_else(|| item.get("sha"))
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();

            let message_headline = item
                .get("messageHeadline")
                .and_then(|v| v.as_str())
                .or_else(|| {
                    item.get("commit")
                        .and_then(|c| c.get("message"))
                        .and_then(|m| m.as_str())
                        .and_then(|s| s.lines().next())
                })
                .unwrap_or("")
                .to_string();

            let message_body = item
                .get("messageBody")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());

            let authored_date = item
                .get("authoredDate")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string())
                .or_else(|| {
                    item.get("commit")
                        .and_then(|c| c.get("author"))
                        .and_then(|a| a.get("date"))
                        .and_then(|d| d.as_str())
                        .map(|s| s.to_string())
                });

            let authors = item
                .get("authors")
                .map(parse_users_list_from_val)
                .unwrap_or_default();

            commits.push(GitHubCommit {
                oid,
                message_headline,
                message_body,
                authored_date,
                authors,
            });
        }
    }
    commits
}

fn parse_reviews_from_val(val: &Value) -> Vec<GitHubReview> {
    let mut reviews = Vec::new();
    if let Some(arr) = val.as_array() {
        for item in arr {
            let id = item.get("id").map(|v| v.to_string());
            let author = item.get("author").and_then(parse_user_from_val).or_else(|| {
                item.get("user").and_then(parse_user_from_val)
            });
            let state = item
                .get("state")
                .and_then(|v| v.as_str())
                .unwrap_or("PENDING")
                .to_string();
            let body = item.get("body").and_then(|v| v.as_str()).map(|s| s.to_string());
            let submitted_at = item
                .get("submittedAt")
                .or_else(|| item.get("submitted_at"))
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());
            let commit_id = item
                .get("commitId")
                .or_else(|| item.get("commit_id"))
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());

            reviews.push(GitHubReview {
                id,
                author,
                state,
                body,
                submitted_at,
                commit_id,
            });
        }
    }
    reviews
}

fn parse_comments_from_val(val: &Value) -> Vec<GitHubComment> {
    let mut comments = Vec::new();
    if let Some(arr) = val.as_array() {
        for item in arr {
            let id = item.get("id").map(|v| v.to_string());
            let author = item.get("author").and_then(parse_user_from_val).or_else(|| {
                item.get("user").and_then(parse_user_from_val)
            });
            let body = item
                .get("body")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let created_at = item
                .get("createdAt")
                .or_else(|| item.get("created_at"))
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());
            let updated_at = item
                .get("updatedAt")
                .or_else(|| item.get("updated_at"))
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());
            let url = item
                .get("url")
                .or_else(|| item.get("html_url"))
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());

            comments.push(GitHubComment {
                id,
                author,
                body,
                created_at,
                updated_at,
                url,
            });
        }
    }
    comments
}

/// Normalizes a single PR JSON object from `gh pr view / list` or API.
pub fn normalize_pr_json(val: &Value) -> GitHubPr {
    let number = val.get("number").and_then(|v| v.as_u64()).unwrap_or(0);
    let title = val
        .get("title")
        .and_then(|v| v.as_str())
        .unwrap_or("Untitled PR")
        .to_string();
    let body = val.get("body").and_then(|v| v.as_str()).map(|s| s.to_string());
    let state = val
        .get("state")
        .and_then(|v| v.as_str())
        .unwrap_or("OPEN")
        .to_uppercase();
    let is_draft = val
        .get("isDraft")
        .or_else(|| val.get("draft"))
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let url = val
        .get("url")
        .or_else(|| val.get("html_url"))
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();

    let author = val
        .get("author")
        .and_then(parse_user_from_val)
        .or_else(|| val.get("user").and_then(parse_user_from_val));

    let base_ref_name = val
        .get("baseRefName")
        .or_else(|| val.get("base").and_then(|b| b.get("ref")))
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    let head_ref_name = val
        .get("headRefName")
        .or_else(|| val.get("head").and_then(|h| h.get("ref")))
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    let head_sha = val
        .get("headRefOid")
        .or_else(|| val.get("head").and_then(|h| h.get("sha")))
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    let additions = val.get("additions").and_then(|v| v.as_u64()).map(|n| n as usize);
    let deletions = val.get("deletions").and_then(|v| v.as_u64()).map(|n| n as usize);
    let changed_files = val
        .get("changedFiles")
        .or_else(|| val.get("changed_files"))
        .and_then(|v| v.as_u64())
        .map(|n| n as usize);

    let labels = val.get("labels").map(parse_labels_from_val).unwrap_or_default();
    let assignees = val
        .get("assignees")
        .map(parse_users_list_from_val)
        .unwrap_or_default();
    let reviewers = val
        .get("reviewRequests")
        .or_else(|| val.get("requested_reviewers"))
        .map(parse_users_list_from_val)
        .unwrap_or_default();
    let milestone = val.get("milestone").and_then(parse_milestone_from_val);

    let mergeable = val
        .get("mergeable")
        .and_then(|v| {
            if let Some(b) = v.as_bool() {
                Some(if b { "MERGEABLE" } else { "CONFLICTING" }.to_string())
            } else {
                v.as_str().map(|s| s.to_string())
            }
        });

    let merge_state_status = val
        .get("mergeStateStatus")
        .or_else(|| val.get("mergeable_state"))
        .and_then(|v| v.as_str())
        .map(|s| s.to_uppercase());

    let merged_at = val
        .get("mergedAt")
        .or_else(|| val.get("merged_at"))
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    let merged_by = val
        .get("mergedBy")
        .or_else(|| val.get("merged_by"))
        .and_then(parse_user_from_val);

    let created_at = val
        .get("createdAt")
        .or_else(|| val.get("created_at"))
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    let updated_at = val
        .get("updatedAt")
        .or_else(|| val.get("updated_at"))
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    let closed_at = val
        .get("closedAt")
        .or_else(|| val.get("closed_at"))
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    let comments_count = val
        .get("comments")
        .and_then(|v| {
            if let Some(arr) = v.as_array() {
                Some(arr.len())
            } else if let Some(n) = v.as_u64() {
                Some(n as usize)
            } else if let Some(tot) = v.get("totalCount").and_then(|t| t.as_u64()) {
                Some(tot as usize)
            } else {
                None
            }
        });

    let reviews_count = val
        .get("reviews")
        .or_else(|| val.get("latestReviews"))
        .and_then(|v| {
            if let Some(arr) = v.as_array() {
                Some(arr.len())
            } else if let Some(tot) = v.get("totalCount").and_then(|t| t.as_u64()) {
                Some(tot as usize)
            } else {
                None
            }
        });

    let status_checks = val
        .get("statusCheckRollup")
        .map(parse_status_checks_from_val)
        .unwrap_or_default();

    let files = val.get("files").map(parse_files_from_val).unwrap_or_default();
    let commits = val.get("commits").map(parse_commits_from_val).unwrap_or_default();
    let reviews = val
        .get("reviews")
        .or_else(|| val.get("latestReviews"))
        .map(parse_reviews_from_val)
        .unwrap_or_default();
    let comments = val.get("comments").map(parse_comments_from_val).unwrap_or_default();

    GitHubPr {
        number,
        title,
        body,
        state,
        is_draft,
        url,
        author,
        base_ref_name,
        head_ref_name,
        head_sha,
        additions,
        deletions,
        changed_files,
        labels,
        assignees,
        reviewers,
        milestone,
        mergeable,
        merge_state_status,
        merged_at,
        merged_by,
        created_at,
        updated_at,
        closed_at,
        comments_count,
        reviews_count,
        status_checks,
        files,
        commits,
        reviews,
        comments,
    }
}

/// Normalizes a single Issue JSON object from `gh issue view / list` or API.
pub fn normalize_issue_json(val: &Value) -> GitHubIssue {
    let number = val.get("number").and_then(|v| v.as_u64()).unwrap_or(0);
    let title = val
        .get("title")
        .and_then(|v| v.as_str())
        .unwrap_or("Untitled Issue")
        .to_string();
    let body = val.get("body").and_then(|v| v.as_str()).map(|s| s.to_string());
    let state = val
        .get("state")
        .and_then(|v| v.as_str())
        .unwrap_or("OPEN")
        .to_uppercase();
    let url = val
        .get("url")
        .or_else(|| val.get("html_url"))
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();

    let author = val
        .get("author")
        .and_then(parse_user_from_val)
        .or_else(|| val.get("user").and_then(parse_user_from_val));

    let labels = val.get("labels").map(parse_labels_from_val).unwrap_or_default();
    let assignees = val
        .get("assignees")
        .map(parse_users_list_from_val)
        .unwrap_or_default();
    let milestone = val.get("milestone").and_then(parse_milestone_from_val);

    let comments_count = val
        .get("comments")
        .and_then(|v| {
            if let Some(arr) = v.as_array() {
                Some(arr.len())
            } else if let Some(n) = v.as_u64() {
                Some(n as usize)
            } else if let Some(tot) = v.get("totalCount").and_then(|t| t.as_u64()) {
                Some(tot as usize)
            } else {
                None
            }
        })
        .unwrap_or(0);

    let created_at = val
        .get("createdAt")
        .or_else(|| val.get("created_at"))
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    let updated_at = val
        .get("updatedAt")
        .or_else(|| val.get("updated_at"))
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    let closed_at = val
        .get("closedAt")
        .or_else(|| val.get("closed_at"))
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    let comments = val.get("comments").map(parse_comments_from_val).unwrap_or_default();

    GitHubIssue {
        number,
        title,
        body,
        state,
        url,
        author,
        labels,
        assignees,
        milestone,
        comments_count,
        created_at,
        updated_at,
        closed_at,
        comments,
    }
}

/// Normalizes repository metadata from `gh repo view` or GitHub API.
pub fn normalize_repo_json(val: &Value) -> GitHubRepoInfo {
    let name = val.get("name").and_then(|v| v.as_str()).unwrap_or("").to_string();
    let owner = val
        .get("owner")
        .and_then(|v| {
            if let Some(login) = v.get("login").and_then(|l| l.as_str()) {
                Some(login.to_string())
            } else {
                v.as_str().map(|s| s.to_string())
            }
        })
        .unwrap_or_default();

    let full_name = val
        .get("nameWithOwner")
        .or_else(|| val.get("full_name"))
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .unwrap_or_else(|| {
            if !owner.is_empty() && !name.is_empty() {
                format!("{owner}/{name}")
            } else {
                name.clone()
            }
        });

    let description = val.get("description").and_then(|v| v.as_str()).map(|s| s.to_string());

    let default_branch = val
        .get("defaultBranchRef")
        .and_then(|v| v.get("name"))
        .or_else(|| val.get("default_branch"))
        .and_then(|v| v.as_str())
        .unwrap_or("main")
        .to_string();

    let visibility = val
        .get("visibility")
        .and_then(|v| v.as_str())
        .unwrap_or("PUBLIC")
        .to_uppercase();

    let is_private = val
        .get("isPrivate")
        .or_else(|| val.get("private"))
        .and_then(|v| v.as_bool())
        .unwrap_or(visibility == "PRIVATE");

    let is_fork = val
        .get("isFork")
        .or_else(|| val.get("fork"))
        .and_then(|v| v.as_bool())
        .unwrap_or(false);

    let stars_count = val
        .get("stargazerCount")
        .or_else(|| val.get("stargazers_count"))
        .and_then(|v| v.as_u64())
        .unwrap_or(0) as usize;

    let forks_count = val
        .get("forksCount")
        .or_else(|| val.get("forks_count"))
        .and_then(|v| v.as_u64())
        .unwrap_or(0) as usize;

    let open_issues_count = val
        .get("openIssuesCount")
        .or_else(|| val.get("open_issues_count"))
        .and_then(|v| v.as_u64())
        .unwrap_or(0) as usize;

    let license = val
        .get("licenseInfo")
        .and_then(|v| v.get("name").or_else(|| v.get("spdxId")))
        .or_else(|| val.get("license").and_then(|v| v.get("name").or_else(|| v.get("spdx_id"))))
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    let mut topics = Vec::new();
    if let Some(arr) = val.get("repositoryTopics").or_else(|| val.get("topics")).and_then(|v| v.as_array()) {
        for t in arr {
            if let Some(s) = t.get("name").and_then(|v| v.as_str()).or_else(|| t.as_str()) {
                topics.push(s.to_string());
            }
        }
    }

    let url = val
        .get("url")
        .or_else(|| val.get("html_url"))
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();

    let homepage = val
        .get("homepageUrl")
        .or_else(|| val.get("homepage"))
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    GitHubRepoInfo {
        name,
        owner,
        full_name,
        description,
        default_branch,
        visibility,
        is_private,
        is_fork,
        stars_count,
        forks_count,
        open_issues_count,
        license,
        topics,
        url,
        homepage,
    }
}

/// Normalizes release JSON from `gh release view / list` or GitHub API.
pub fn normalize_release_json(val: &Value) -> GitHubRelease {
    let tag_name = val
        .get("tagName")
        .or_else(|| val.get("tag_name"))
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();

    let name = val.get("name").and_then(|v| v.as_str()).map(|s| s.to_string());
    let body = val.get("body").and_then(|v| v.as_str()).map(|s| s.to_string());
    let is_draft = val
        .get("isDraft")
        .or_else(|| val.get("draft"))
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let is_prerelease = val
        .get("isPrerelease")
        .or_else(|| val.get("prerelease"))
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let is_latest = val
        .get("isLatest")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);

    let published_at = val
        .get("publishedAt")
        .or_else(|| val.get("published_at"))
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    let url = val
        .get("url")
        .or_else(|| val.get("html_url"))
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();

    let mut assets = Vec::new();
    if let Some(arr) = val.get("assets").and_then(|v| v.as_array()) {
        for item in arr {
            let aname = item.get("name").and_then(|v| v.as_str()).unwrap_or("").to_string();
            let size = item.get("size").and_then(|v| v.as_u64()).unwrap_or(0);
            let download_count = item
                .get("downloadCount")
                .or_else(|| item.get("download_count"))
                .and_then(|v| v.as_u64())
                .unwrap_or(0);
            let browser_download_url = item
                .get("browserDownloadUrl")
                .or_else(|| item.get("browser_download_url"))
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();

            if !aname.is_empty() {
                assets.push(GitHubReleaseAsset {
                    name: aname,
                    size,
                    download_count,
                    browser_download_url,
                });
            }
        }
    }

    GitHubRelease {
        tag_name,
        name,
        body,
        is_draft,
        is_prerelease,
        is_latest,
        published_at,
        url,
        assets,
    }
}

/// Normalizes workflow run JSON from `gh run list` or GitHub API.
pub fn normalize_workflow_run_json(val: &Value) -> GitHubWorkflowRun {
    let id = val
        .get("databaseId")
        .or_else(|| val.get("id"))
        .and_then(|v| v.as_u64())
        .unwrap_or(0);

    let name = val
        .get("displayTitle")
        .or_else(|| val.get("name"))
        .or_else(|| val.get("head_commit").and_then(|c| c.get("message")))
        .and_then(|v| v.as_str())
        .unwrap_or("Workflow Run")
        .to_string();

    let workflow_name = val
        .get("workflowName")
        .or_else(|| val.get("name"))
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    let status = val
        .get("status")
        .and_then(|v| v.as_str())
        .unwrap_or("completed")
        .to_string();

    let conclusion = val
        .get("conclusion")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    let branch = val
        .get("headBranch")
        .or_else(|| val.get("head_branch"))
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    let event = val.get("event").and_then(|v| v.as_str()).map(|s| s.to_string());

    let head_sha = val
        .get("headSha")
        .or_else(|| val.get("head_sha"))
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    let created_at = val
        .get("createdAt")
        .or_else(|| val.get("created_at"))
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    let url = val
        .get("url")
        .or_else(|| val.get("html_url"))
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();

    GitHubWorkflowRun {
        id,
        name,
        workflow_name,
        status,
        conclusion,
        branch,
        event,
        head_sha,
        created_at,
        url,
    }
}

// ============================================================================
// High-Level Inspection Handlers
// ============================================================================

const PR_FIELDS: &str = "additions,assignees,author,baseRefName,body,changedFiles,closedAt,comments,commits,createdAt,deletions,files,headRefName,headRefOid,isDraft,labels,latestReviews,mergeStateStatus,mergeable,mergedAt,mergedBy,milestone,number,reviewRequests,reviews,state,statusCheckRollup,title,updatedAt,url";
const ISSUE_FIELDS: &str = "assignees,author,body,closedAt,comments,createdAt,labels,milestone,number,state,title,updatedAt,url";
const REPO_FIELDS: &str = "name,owner,nameWithOwner,description,defaultBranchRef,visibility,isPrivate,isFork,stargazerCount,forksCount,openIssuesCount,licenseInfo,repositoryTopics,url,homepageUrl";
const RELEASE_FIELDS: &str = "tagName,name,body,isDraft,isPrerelease,isLatest,publishedAt,url,assets";
const RUN_FIELDS: &str = "databaseId,displayTitle,workflowName,status,conclusion,headBranch,event,headSha,createdAt,url";

/// Lists pull requests via `gh pr list`.
pub async fn gh_list_prs(
    repo: Option<&str>,
    state: &str,
    limit: usize,
    author: Option<&str>,
    assignee: Option<&str>,
    base: Option<&str>,
    head: Option<&str>,
    label: Option<&str>,
    search: Option<&str>,
    draft: Option<bool>,
    cwd: &Path,
    env: &HashMap<String, String>,
) -> anyhow::Result<GitHubPrListReport> {
    let mut args = vec!["pr", "list", "--json", PR_FIELDS];

    let limit_str = limit.clamp(1, 100).to_string();
    args.push("--limit");
    args.push(&limit_str);

    let state_flag = match state.to_lowercase().as_str() {
        "all" => "all",
        "closed" => "closed",
        "merged" => "merged",
        _ => "open",
    };
    args.push("--state");
    args.push(state_flag);

    if let Some(r) = repo {
        args.push("-R");
        args.push(r);
    }
    if let Some(a) = author {
        args.push("--author");
        args.push(a);
    }
    if let Some(a) = assignee {
        args.push("--assignee");
        args.push(a);
    }
    if let Some(b) = base {
        args.push("--base");
        args.push(b);
    }
    if let Some(h) = head {
        args.push("--head");
        args.push(h);
    }
    if let Some(l) = label {
        args.push("--label");
        args.push(l);
    }
    if let Some(s) = search {
        args.push("--search");
        args.push(s);
    }
    if let Some(true) = draft {
        args.push("--draft");
    }

    let output = run_gh_command(&args, cwd, env, 30).await?;
    if !output.success {
        anyhow::bail!("gh pr list failed: {}", output.stderr.trim());
    }

    let parsed: Value = serde_json::from_str(&output.stdout)
        .map_err(|e| anyhow::anyhow!("Failed parsing gh pr list output: {e}\nRaw: {}", output.stdout))?;

    let mut pull_requests = Vec::new();
    if let Some(arr) = parsed.as_array() {
        for item in arr {
            pull_requests.push(normalize_pr_json(item));
        }
    }

    let repo_name = match repo {
        Some(s) => s.to_string(),
        None => detect_repo_from_cwd(cwd)
            .await
            .map(|(o, r)| format!("{o}/{r}"))
            .unwrap_or_else(|| "current-repo".to_string()),
    };

    let total_count = pull_requests.len();
    Ok(GitHubPrListReport {
        repo: repo_name,
        total_count,
        pull_requests,
    })
}

/// Views a pull request via `gh pr view`.
pub async fn gh_view_pr(
    repo: Option<&str>,
    pr_ref: &str,
    cwd: &Path,
    env: &HashMap<String, String>,
) -> anyhow::Result<GitHubPr> {
    let mut args = vec!["pr", "view", pr_ref, "--json", PR_FIELDS];
    if let Some(r) = repo {
        args.push("-R");
        args.push(r);
    }

    let output = run_gh_command(&args, cwd, env, 30).await?;
    if !output.success {
        anyhow::bail!("gh pr view failed for '{pr_ref}': {}", output.stderr.trim());
    }

    let parsed: Value = serde_json::from_str(&output.stdout)
        .map_err(|e| anyhow::anyhow!("Failed parsing gh pr view output: {e}\nRaw: {}", output.stdout))?;

    Ok(normalize_pr_json(&parsed))
}

/// Retrieves the raw diff of a pull request via `gh pr diff`.
pub async fn gh_pr_diff(
    repo: Option<&str>,
    pr_ref: &str,
    cwd: &Path,
    env: &HashMap<String, String>,
) -> anyhow::Result<String> {
    let mut args = vec!["pr", "diff", pr_ref];
    if let Some(r) = repo {
        args.push("-R");
        args.push(r);
    }

    let output = run_gh_command(&args, cwd, env, 30).await?;
    if !output.success {
        anyhow::bail!("gh pr diff failed for '{pr_ref}': {}", output.stderr.trim());
    }

    Ok(output.stdout)
}

/// Lists issues via `gh issue list`.
pub async fn gh_list_issues(
    repo: Option<&str>,
    state: &str,
    limit: usize,
    author: Option<&str>,
    assignee: Option<&str>,
    label: Option<&str>,
    milestone: Option<&str>,
    search: Option<&str>,
    cwd: &Path,
    env: &HashMap<String, String>,
) -> anyhow::Result<GitHubIssueListReport> {
    let mut args = vec!["issue", "list", "--json", ISSUE_FIELDS];

    let limit_str = limit.clamp(1, 100).to_string();
    args.push("--limit");
    args.push(&limit_str);

    let state_flag = match state.to_lowercase().as_str() {
        "all" => "all",
        "closed" => "closed",
        _ => "open",
    };
    args.push("--state");
    args.push(state_flag);

    if let Some(r) = repo {
        args.push("-R");
        args.push(r);
    }
    if let Some(a) = author {
        args.push("--author");
        args.push(a);
    }
    if let Some(a) = assignee {
        args.push("--assignee");
        args.push(a);
    }
    if let Some(l) = label {
        args.push("--label");
        args.push(l);
    }
    if let Some(m) = milestone {
        args.push("--milestone");
        args.push(m);
    }
    if let Some(s) = search {
        args.push("--search");
        args.push(s);
    }

    let output = run_gh_command(&args, cwd, env, 30).await?;
    if !output.success {
        anyhow::bail!("gh issue list failed: {}", output.stderr.trim());
    }

    let parsed: Value = serde_json::from_str(&output.stdout)
        .map_err(|e| anyhow::anyhow!("Failed parsing gh issue list output: {e}\nRaw: {}", output.stdout))?;

    let mut issues = Vec::new();
    if let Some(arr) = parsed.as_array() {
        for item in arr {
            issues.push(normalize_issue_json(item));
        }
    }

    let repo_name = match repo {
        Some(s) => s.to_string(),
        None => detect_repo_from_cwd(cwd)
            .await
            .map(|(o, r)| format!("{o}/{r}"))
            .unwrap_or_else(|| "current-repo".to_string()),
    };

    let total_count = issues.len();
    Ok(GitHubIssueListReport {
        repo: repo_name,
        total_count,
        issues,
    })
}

/// Views an issue via `gh issue view`.
pub async fn gh_view_issue(
    repo: Option<&str>,
    issue_ref: &str,
    cwd: &Path,
    env: &HashMap<String, String>,
) -> anyhow::Result<GitHubIssue> {
    let mut args = vec!["issue", "view", issue_ref, "--json", ISSUE_FIELDS];
    if let Some(r) = repo {
        args.push("-R");
        args.push(r);
    }

    let output = run_gh_command(&args, cwd, env, 30).await?;
    if !output.success {
        anyhow::bail!("gh issue view failed for '{issue_ref}': {}", output.stderr.trim());
    }

    let parsed: Value = serde_json::from_str(&output.stdout)
        .map_err(|e| anyhow::anyhow!("Failed parsing gh issue view output: {e}\nRaw: {}", output.stdout))?;

    Ok(normalize_issue_json(&parsed))
}

/// Views repository details via `gh repo view`.
pub async fn gh_view_repo(
    repo: Option<&str>,
    cwd: &Path,
    env: &HashMap<String, String>,
) -> anyhow::Result<GitHubRepoInfo> {
    let mut args = vec!["repo", "view", "--json", REPO_FIELDS];
    if let Some(r) = repo {
        args.push(r);
    }

    let output = run_gh_command(&args, cwd, env, 30).await?;
    if !output.success {
        anyhow::bail!("gh repo view failed: {}", output.stderr.trim());
    }

    let parsed: Value = serde_json::from_str(&output.stdout)
        .map_err(|e| anyhow::anyhow!("Failed parsing gh repo view output: {e}\nRaw: {}", output.stdout))?;

    Ok(normalize_repo_json(&parsed))
}

/// Lists releases via `gh release list`.
pub async fn gh_list_releases(
    repo: Option<&str>,
    limit: usize,
    cwd: &Path,
    env: &HashMap<String, String>,
) -> anyhow::Result<GitHubReleaseListReport> {
    let mut args = vec!["release", "list", "--json", RELEASE_FIELDS];
    let limit_str = limit.clamp(1, 100).to_string();
    args.push("--limit");
    args.push(&limit_str);

    if let Some(r) = repo {
        args.push("-R");
        args.push(r);
    }

    let output = run_gh_command(&args, cwd, env, 30).await?;
    if !output.success {
        anyhow::bail!("gh release list failed: {}", output.stderr.trim());
    }

    let parsed: Value = serde_json::from_str(&output.stdout)
        .map_err(|e| anyhow::anyhow!("Failed parsing gh release list output: {e}\nRaw: {}", output.stdout))?;

    let mut releases = Vec::new();
    if let Some(arr) = parsed.as_array() {
        for item in arr {
            releases.push(normalize_release_json(item));
        }
    }

    let repo_name = match repo {
        Some(s) => s.to_string(),
        None => detect_repo_from_cwd(cwd)
            .await
            .map(|(o, r)| format!("{o}/{r}"))
            .unwrap_or_else(|| "current-repo".to_string()),
    };

    let total_count = releases.len();
    Ok(GitHubReleaseListReport {
        repo: repo_name,
        total_count,
        releases,
    })
}

/// Views a specific release via `gh release view`.
pub async fn gh_view_release(
    repo: Option<&str>,
    tag: &str,
    cwd: &Path,
    env: &HashMap<String, String>,
) -> anyhow::Result<GitHubRelease> {
    let mut args = vec!["release", "view", tag, "--json", RELEASE_FIELDS];
    if let Some(r) = repo {
        args.push("-R");
        args.push(r);
    }

    let output = run_gh_command(&args, cwd, env, 30).await?;
    if !output.success {
        anyhow::bail!("gh release view failed for '{tag}': {}", output.stderr.trim());
    }

    let parsed: Value = serde_json::from_str(&output.stdout)
        .map_err(|e| anyhow::anyhow!("Failed parsing gh release view output: {e}\nRaw: {}", output.stdout))?;

    Ok(normalize_release_json(&parsed))
}

/// Lists recent GitHub Actions workflow runs via `gh run list`.
pub async fn gh_list_runs(
    repo: Option<&str>,
    limit: usize,
    workflow: Option<&str>,
    branch: Option<&str>,
    cwd: &Path,
    env: &HashMap<String, String>,
) -> anyhow::Result<GitHubWorkflowRunListReport> {
    let mut args = vec!["run", "list", "--json", RUN_FIELDS];
    let limit_str = limit.clamp(1, 100).to_string();
    args.push("--limit");
    args.push(&limit_str);

    if let Some(r) = repo {
        args.push("-R");
        args.push(r);
    }
    if let Some(w) = workflow {
        args.push("--workflow");
        args.push(w);
    }
    if let Some(b) = branch {
        args.push("--branch");
        args.push(b);
    }

    let output = run_gh_command(&args, cwd, env, 30).await?;
    if !output.success {
        anyhow::bail!("gh run list failed: {}", output.stderr.trim());
    }

    let parsed: Value = serde_json::from_str(&output.stdout)
        .map_err(|e| anyhow::anyhow!("Failed parsing gh run list output: {e}\nRaw: {}", output.stdout))?;

    let mut runs = Vec::new();
    if let Some(arr) = parsed.as_array() {
        for item in arr {
            runs.push(normalize_workflow_run_json(item));
        }
    }

    let repo_name = match repo {
        Some(s) => s.to_string(),
        None => detect_repo_from_cwd(cwd)
            .await
            .map(|(o, r)| format!("{o}/{r}"))
            .unwrap_or_else(|| "current-repo".to_string()),
    };

    let total_count = runs.len();
    Ok(GitHubWorkflowRunListReport {
        repo: repo_name,
        total_count,
        runs,
    })
}

// ============================================================================
// REST API Fallback (Direct reqwest queries)
// ============================================================================

/// Queries the GitHub REST API as a fallback when `gh` CLI is absent.
pub async fn query_github_api(
    endpoint: &str,
    env: &HashMap<String, String>,
) -> anyhow::Result<Value> {
    let client = reqwest::Client::builder()
        .user_agent("fusion-ai-assistant/0.3.0")
        .timeout(Duration::from_secs(20))
        .build()?;

    let url = if endpoint.starts_with("http://") || endpoint.starts_with("https://") {
        endpoint.to_string()
    } else {
        format!("https://api.github.com/{}", endpoint.trim_start_matches('/'))
    };

    let mut req = client.get(&url).header("Accept", "application/vnd.github.v3+json");

    // Check GITHUB_TOKEN or GH_TOKEN
    let token = env
        .get("GITHUB_TOKEN")
        .cloned()
        .or_else(|| env.get("GH_TOKEN").cloned())
        .or_else(|| std::env::var("GITHUB_TOKEN").ok())
        .or_else(|| std::env::var("GH_TOKEN").ok());

    if let Some(t) = token {
        if !t.is_empty() {
            req = req.header("Authorization", format!("Bearer {t}"));
        }
    }

    let res = req.send().await.map_err(|e| anyhow::anyhow!("GitHub API network error: {e}"))?;
    let status = res.status();
    if !status.is_success() {
        let text = res.text().await.unwrap_or_default();
        anyhow::bail!("GitHub API returned HTTP {status}: {text}");
    }

    let json_val: Value = res.json().await.map_err(|e| anyhow::anyhow!("GitHub API JSON parse error: {e}"))?;
    Ok(json_val)
}

// ============================================================================
// Human-Readable Summary Formatters
// ============================================================================

pub fn format_pr_list_summary(report: &GitHubPrListReport) -> String {
    let mut out = String::new();
    out.push_str(&format!(
        "# Pull Requests for '{}' (Total: {})\n\n",
        report.repo, report.total_count
    ));

    if report.pull_requests.is_empty() {
        out.push_str("No pull requests found matching criteria.\n");
        return out;
    }

    for pr in &report.pull_requests {
        let state_badge = match pr.state.as_str() {
            "OPEN" => if pr.is_draft { "[DRAFT]" } else { "[OPEN]" },
            "MERGED" => "[MERGED]",
            "CLOSED" => "[CLOSED]",
            _ => "[UNKNOWN]",
        };

        let author_str = pr
            .author
            .as_ref()
            .map(|u| format!("by @{}", u.login))
            .unwrap_or_default();

        let branch_str = match (&pr.head_ref_name, &pr.base_ref_name) {
            (Some(h), Some(b)) => format!(" ({h} -> {b})"),
            _ => String::new(),
        };

        let stats_str = match (pr.additions, pr.deletions) {
            (Some(a), Some(d)) => format!(" +{a}/-{d}"),
            _ => String::new(),
        };

        out.push_str(&format!(
            "- **#{}** {} **{}** {} {}{}\n  {}\n",
            pr.number, state_badge, pr.title, author_str, stats_str, branch_str, pr.url
        ));
    }

    out
}

pub fn format_pr_view_summary(pr: &GitHubPr) -> String {
    let mut out = String::new();
    let state_badge = match pr.state.as_str() {
        "OPEN" => if pr.is_draft { "[DRAFT]" } else { "[OPEN]" },
        "MERGED" => "[MERGED]",
        "CLOSED" => "[CLOSED]",
        _ => "[UNKNOWN]",
    };

    out.push_str(&format!("# PR #{}: {} {}\n\n", pr.number, pr.title, state_badge));
    out.push_str(&format!("**URL**: {}\n", pr.url));

    if let Some(author) = &pr.author {
        out.push_str(&format!("**Author**: @{}\n", author.login));
    }
    if let (Some(head), Some(base)) = (&pr.head_ref_name, &pr.base_ref_name) {
        out.push_str(&format!("**Branches**: `{head}` -> `{base}`\n"));
    }
    if let (Some(add), Some(del), Some(files)) = (pr.additions, pr.deletions, pr.changed_files) {
        out.push_str(&format!("**Diff Stats**: +{add} / -{del} across {files} changed files\n"));
    }
    if let Some(m) = &pr.mergeable {
        out.push_str(&format!("**Mergeable**: {} (Status: {})\n", m, pr.merge_state_status.as_deref().unwrap_or("N/A")));
    }
    if !pr.labels.is_empty() {
        let label_names: Vec<&str> = pr.labels.iter().map(|l| l.name.as_str()).collect();
        out.push_str(&format!("**Labels**: {}\n", label_names.join(", ")));
    }
    if !pr.assignees.is_empty() {
        let assignee_names: Vec<String> = pr.assignees.iter().map(|u| format!("@{}", u.login)).collect();
        out.push_str(&format!("**Assignees**: {}\n", assignee_names.join(", ")));
    }
    if !pr.reviewers.is_empty() {
        let reviewer_names: Vec<String> = pr.reviewers.iter().map(|u| format!("@{}", u.login)).collect();
        out.push_str(&format!("**Requested Reviewers**: {}\n", reviewer_names.join(", ")));
    }
    if let Some(ms) = &pr.milestone {
        out.push_str(&format!("**Milestone**: {}\n", ms.title));
    }

    if let Some(body) = &pr.body {
        out.push_str("\n### Description\n\n");
        out.push_str(body);
        out.push_str("\n\n");
    }

    if !pr.status_checks.is_empty() {
        out.push_str("### Status Checks\n\n");
        for check in &pr.status_checks {
            let res = check.conclusion.as_deref().unwrap_or(&check.status);
            out.push_str(&format!("- [{}] {}\n", res, check.name));
        }
        out.push('\n');
    }

    if !pr.files.is_empty() {
        out.push_str("### Changed Files\n\n");
        for file in &pr.files {
            out.push_str(&format!("- `{}` (+{}/-{})\n", file.path, file.additions, file.deletions));
        }
        out.push('\n');
    }

    out
}

pub fn format_issue_list_summary(report: &GitHubIssueListReport) -> String {
    let mut out = String::new();
    out.push_str(&format!(
        "# Issues for '{}' (Total: {})\n\n",
        report.repo, report.total_count
    ));

    if report.issues.is_empty() {
        out.push_str("No issues found matching criteria.\n");
        return out;
    }

    for issue in &report.issues {
        let state_badge = if issue.state == "OPEN" { "[OPEN]" } else { "[CLOSED]" };
        let author_str = issue
            .author
            .as_ref()
            .map(|u| format!("by @{}", u.login))
            .unwrap_or_default();

        let comments_str = if issue.comments_count > 0 {
            format!(" ({} comments)", issue.comments_count)
        } else {
            String::new()
        };

        out.push_str(&format!(
            "- **#{}** {} **{}** {}{}\n  {}\n",
            issue.number, state_badge, issue.title, author_str, comments_str, issue.url
        ));
    }

    out
}

pub fn format_issue_view_summary(issue: &GitHubIssue) -> String {
    let mut out = String::new();
    let state_badge = if issue.state == "OPEN" { "[OPEN]" } else { "[CLOSED]" };

    out.push_str(&format!("# Issue #{}: {} {}\n\n", issue.number, issue.title, state_badge));
    out.push_str(&format!("**URL**: {}\n", issue.url));

    if let Some(author) = &issue.author {
        out.push_str(&format!("**Author**: @{}\n", author.login));
    }
    if !issue.labels.is_empty() {
        let label_names: Vec<&str> = issue.labels.iter().map(|l| l.name.as_str()).collect();
        out.push_str(&format!("**Labels**: {}\n", label_names.join(", ")));
    }
    if !issue.assignees.is_empty() {
        let assignee_names: Vec<String> = issue.assignees.iter().map(|u| format!("@{}", u.login)).collect();
        out.push_str(&format!("**Assignees**: {}\n", assignee_names.join(", ")));
    }
    if let Some(ms) = &issue.milestone {
        out.push_str(&format!("**Milestone**: {}\n", ms.title));
    }

    if let Some(body) = &issue.body {
        out.push_str("\n### Description\n\n");
        out.push_str(body);
        out.push_str("\n\n");
    }

    if !issue.comments.is_empty() {
        out.push_str("### Comments\n\n");
        for (idx, c) in issue.comments.iter().enumerate() {
            let author_login = c.author.as_ref().map(|u| u.login.as_str()).unwrap_or("ghost");
            out.push_str(&format!("#### Comment {} by @{}\n\n{}\n\n", idx + 1, author_login, c.body));
        }
    }

    out
}

pub fn format_repo_view_summary(repo: &GitHubRepoInfo) -> String {
    let mut out = String::new();
    out.push_str(&format!("# Repository: {}\n\n", repo.full_name));
    out.push_str(&format!("**URL**: {}\n", repo.url));
    if let Some(desc) = &repo.description {
        out.push_str(&format!("**Description**: {}\n", desc));
    }
    out.push_str(&format!("**Default Branch**: `{}`\n", repo.default_branch));
    out.push_str(&format!("**Visibility**: {}\n", repo.visibility));
    out.push_str(&format!("**Stars**: {} | **Forks**: {} | **Open Issues/PRs**: {}\n", repo.stars_count, repo.forks_count, repo.open_issues_count));
    if let Some(lic) = &repo.license {
        out.push_str(&format!("**License**: {}\n", lic));
    }
    if !repo.topics.is_empty() {
        out.push_str(&format!("**Topics**: {}\n", repo.topics.join(", ")));
    }
    if let Some(hp) = &repo.homepage {
        out.push_str(&format!("**Homepage**: {}\n", hp));
    }
    out
}

pub fn format_release_list_summary(report: &GitHubReleaseListReport) -> String {
    let mut out = String::new();
    out.push_str(&format!("# Releases for '{}' (Total: {})\n\n", report.repo, report.total_count));
    if report.releases.is_empty() {
        out.push_str("No releases found.\n");
        return out;
    }

    for rel in &report.releases {
        let tag_badge = if rel.is_latest {
            "[LATEST]"
        } else if rel.is_prerelease {
            "[PRE-RELEASE]"
        } else if rel.is_draft {
            "[DRAFT]"
        } else {
            ""
        };

        let title = rel.name.as_deref().unwrap_or(&rel.tag_name);
        out.push_str(&format!("- **{}** ({}) {} {}\n", rel.tag_name, title, tag_badge, rel.url));
    }
    out
}

pub fn format_workflow_runs_summary(report: &GitHubWorkflowRunListReport) -> String {
    let mut out = String::new();
    out.push_str(&format!("# Actions Workflow Runs for '{}' (Total: {})\n\n", report.repo, report.total_count));
    if report.runs.is_empty() {
        out.push_str("No workflow runs found.\n");
        return out;
    }

    for run in &report.runs {
        let conclusion_str = run.conclusion.as_deref().unwrap_or(&run.status);
        let branch_str = run.branch.as_deref().map(|b| format!(" on `{b}`")).unwrap_or_default();
        out.push_str(&format!("- [{}] **#{}** {} ({}){}\n  {}\n", conclusion_str, run.id, run.name, run.workflow_name.as_deref().unwrap_or("workflow"), branch_str, run.url));
    }
    out
}

// ============================================================================
// GitHubTool Implementation
// ============================================================================

/// Comprehensive GitHub inspection tool for Pull Requests, Issues, Repositories, Releases, and CI Runs.
#[derive(Default, Debug, Clone)]
pub struct GitHubTool;

impl GitHubTool {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl Tool for GitHubTool {
    fn name(&self) -> &str {
        "github"
    }

    fn description(&self) -> &str {
        "Inspect GitHub Pull Requests, Issues, Repositories, Releases, and CI Actions Workflow Runs via gh CLI, GitHub REST API, or Git remotes with clean structured JSON output."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "action": {
                    "type": "string",
                    "enum": [
                        "pr_list", "list_prs", "prs",
                        "pr_view", "view_pr", "pr",
                        "pr_diff", "diff_pr",
                        "pr_commits", "commits_pr",
                        "pr_checks", "checks_pr", "status_checks",
                        "pr_reviews", "reviews_pr",
                        "issue_list", "list_issues", "issues",
                        "issue_view", "view_issue", "issue",
                        "issue_comments", "comments_issue",
                        "repo_view", "view_repo", "repo_info",
                        "release_list", "list_releases", "releases",
                        "release_view", "view_release",
                        "run_list", "list_runs", "actions",
                        "create_draft_pr", "draft_pr",
                        "add_comment", "comment"
                    ],
                    "description": "Operation to perform: 'pr_list' (list pull requests), 'pr_view' (view PR details), 'pr_diff' (view PR diff), 'pr_commits' (list PR commits), 'pr_checks' (inspect PR CI status checks), 'pr_reviews' (inspect PR reviews), 'issue_list' (list issues), 'issue_view' (view issue details), 'issue_comments' (inspect issue comments), 'repo_view' (view repository metadata), 'release_list' (list releases), 'release_view' (view release by tag), 'run_list' (list CI workflow runs), 'create_draft_pr' (create a new draft pull request), 'add_comment' (add a comment to a PR or issue)."
                },
                "pr_number": {
                    "type": "integer",
                    "description": "Pull request number (e.g. 42). Alternatively pass PR branch or full URL."
                },
                "issue_number": {
                    "type": "integer",
                    "description": "Issue number (e.g. 101) or URL."
                },
                "target": {
                    "type": "string",
                    "description": "Target PR number, issue number, branch name, release tag, or GitHub URL."
                },
                "repo": {
                    "type": "string",
                    "description": "Target GitHub repository in 'owner/repo' format (e.g. 'theaungmyatmoe/fusion' or 'rust-lang/rust'). Defaults to current repository origin."
                },
                "state": {
                    "type": "string",
                    "enum": ["open", "closed", "merged", "all"],
                    "description": "Filter by state: 'open' (default), 'closed', 'merged', or 'all'."
                },
                "limit": {
                    "type": "integer",
                    "description": "Maximum number of items to return (default 30, max 100)."
                },
                "author": {
                    "type": "string",
                    "description": "Filter by author username."
                },
                "assignee": {
                    "type": "string",
                    "description": "Filter by assignee username."
                },
                "base": {
                    "type": "string",
                    "description": "Filter pull requests by base branch (e.g. 'main')."
                },
                "head": {
                    "type": "string",
                    "description": "Filter pull requests by head branch."
                },
                "label": {
                    "type": "string",
                    "description": "Filter by label name."
                },
                "milestone": {
                    "type": "string",
                    "description": "Filter by milestone title or number."
                },
                "search": {
                    "type": "string",
                    "description": "Search query string to filter PRs or issues."
                },
                "draft": {
                    "type": "boolean",
                    "description": "Filter pull requests by draft status (true = draft only)."
                },
                "tag": {
                    "type": "string",
                    "description": "Release tag name (e.g. 'v0.3.0')."
                },
                "workflow": {
                    "type": "string",
                    "description": "Filter workflow runs by workflow name or filename (e.g. 'ci.yml')."
                },
                "branch": {
                    "type": "string",
                    "description": "Filter workflow runs by branch name."
                },
                "format": {
                    "type": "string",
                    "enum": ["json", "summary", "markdown"],
                    "description": "Output format: 'json' (clean structured JSON, default) or 'summary'/'markdown' (human-readable text)."
                },
                "title": {
                    "type": "string",
                    "description": "Title for a new pull request (required for 'create_draft_pr')."
                },
                "body": {
                    "type": "string",
                    "description": "Body/description for a new pull request or comment text for 'add_comment'."
                },
                "comment": {
                    "type": "string",
                    "description": "Comment body text to post (used by 'add_comment'; alias for 'body')."
                },
                "base_branch": {
                    "type": "string",
                    "description": "Base branch for 'create_draft_pr' (defaults to the repository default branch, e.g. 'main')."
                },
                "head_branch": {
                    "type": "string",
                    "description": "Head branch for 'create_draft_pr' (defaults to the current checked-out branch)."
                }
            },
            "required": ["action"]
        })
    }

    async fn execute(&self, args: Value, ctx: &ToolContext) -> anyhow::Result<String> {
        let action_str = args
            .get("action")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("Missing required parameter 'action'"))?;

        let repo_param = args.get("repo").and_then(|v| v.as_str());
        let state_param = args.get("state").and_then(|v| v.as_str()).unwrap_or("open");
        let limit_param = args.get("limit").and_then(|v| v.as_u64()).unwrap_or(30) as usize;
        let author_param = args.get("author").and_then(|v| v.as_str());
        let assignee_param = args.get("assignee").and_then(|v| v.as_str());
        let base_param = args.get("base").and_then(|v| v.as_str());
        let head_param = args.get("head").and_then(|v| v.as_str());
        let label_param = args.get("label").and_then(|v| v.as_str());
        let milestone_param = args.get("milestone").and_then(|v| v.as_str());
        let search_param = args.get("search").and_then(|v| v.as_str());
        let draft_param = args.get("draft").and_then(|v| v.as_bool());
        let tag_param = args.get("tag").and_then(|v| v.as_str());
        let workflow_param = args.get("workflow").and_then(|v| v.as_str());
        let branch_param = args.get("branch").and_then(|v| v.as_str());
        let format_param = args.get("format").and_then(|v| v.as_str()).unwrap_or("json");

        // Target resolver (pr_number, issue_number, target, or positional number)
        let pr_target = args
            .get("pr_number")
            .map(|v| v.to_string())
            .or_else(|| args.get("target").and_then(|v| v.as_str()).map(|s| s.to_string()));

        let issue_target = args
            .get("issue_number")
            .map(|v| v.to_string())
            .or_else(|| args.get("target").and_then(|v| v.as_str()).map(|s| s.to_string()));

        let release_target = tag_param.map(|s| s.to_string()).or_else(|| {
            args.get("target").and_then(|v| v.as_str()).map(|s| s.to_string())
        });

        // Resolve active repository
        let detected_repo = detect_repo_from_cwd(&ctx.cwd).await.map(|(o, r)| format!("{o}/{r}"));
        let effective_repo = repo_param.map(|s| s.to_string()).or(detected_repo);

        let gh_available = is_gh_cli_available(&ctx.cwd).await;
        let action_normalized = action_str.trim().to_lowercase();

        let response = match action_normalized.as_str() {
            // --- PR Listing ---
            "pr_list" | "list_prs" | "prs" => {
                if gh_available {
                    match gh_list_prs(
                        effective_repo.as_deref(),
                        state_param,
                        limit_param,
                        author_param,
                        assignee_param,
                        base_param,
                        head_param,
                        label_param,
                        search_param,
                        draft_param,
                        &ctx.cwd,
                        &ctx.env,
                    )
                    .await
                    {
                        Ok(report) => {
                            let summary = format_pr_list_summary(&report);
                            let data = serde_json::to_value(&report)?;
                            GitHubToolResponse {
                                action: "pr_list".to_string(),
                                success: true,
                                repo: Some(report.repo),
                                backend: "gh_cli".to_string(),
                                data,
                                message: format!("Retrieved {} pull requests", report.total_count),
                                summary: Some(summary),
                            }
                        }
                        Err(e) => {
                            // Try REST API fallback if repo is known
                            if let Some(r) = &effective_repo {
                                let endpoint = format!("repos/{r}/pulls?state={state_param}&per_page={limit_param}");
                                match query_github_api(&endpoint, &ctx.env).await {
                                    Ok(api_val) => {
                                        let mut pull_requests = Vec::new();
                                        if let Some(arr) = api_val.as_array() {
                                            for item in arr {
                                                pull_requests.push(normalize_pr_json(item));
                                            }
                                        }
                                        let total_count = pull_requests.len();
                                        let report = GitHubPrListReport {
                                            repo: r.clone(),
                                            total_count,
                                            pull_requests,
                                        };
                                        let summary = format_pr_list_summary(&report);
                                        let data = serde_json::to_value(&report)?;
                                        GitHubToolResponse {
                                            action: "pr_list".to_string(),
                                            success: true,
                                            repo: Some(r.clone()),
                                            backend: "github_api".to_string(),
                                            data,
                                            message: format!("Retrieved {} pull requests via GitHub API", total_count),
                                            summary: Some(summary),
                                        }
                                    }
                                    Err(api_err) => {
                                        anyhow::bail!("gh command failed: {e}\nAPI fallback failed: {api_err}");
                                    }
                                }
                            } else {
                                anyhow::bail!("Failed to list PRs: {e}");
                            }
                        }
                    }
                } else if let Some(r) = &effective_repo {
                    // Fallback to REST API
                    let endpoint = format!("repos/{r}/pulls?state={state_param}&per_page={limit_param}");
                    let api_val = query_github_api(&endpoint, &ctx.env).await?;
                    let mut pull_requests = Vec::new();
                    if let Some(arr) = api_val.as_array() {
                        for item in arr {
                            pull_requests.push(normalize_pr_json(item));
                        }
                    }
                    let total_count = pull_requests.len();
                    let report = GitHubPrListReport {
                        repo: r.clone(),
                        total_count,
                        pull_requests,
                    };
                    let summary = format_pr_list_summary(&report);
                    let data = serde_json::to_value(&report)?;
                    GitHubToolResponse {
                        action: "pr_list".to_string(),
                        success: true,
                        repo: Some(r.clone()),
                        backend: "github_api".to_string(),
                        data,
                        message: format!("Retrieved {} pull requests via GitHub API", total_count),
                        summary: Some(summary),
                    }
                } else {
                    anyhow::bail!("GitHub CLI ('gh') is not installed and no remote repository could be detected.");
                }
            }

            // --- PR View ---
            "pr_view" | "view_pr" | "pr" => {
                let target = pr_target.ok_or_else(|| {
                    anyhow::anyhow!("Missing required parameter 'pr_number' or 'target' for pr_view")
                })?;

                if gh_available {
                    match gh_view_pr(effective_repo.as_deref(), &target, &ctx.cwd, &ctx.env).await {
                        Ok(pr) => {
                            let summary = format_pr_view_summary(&pr);
                            let data = serde_json::to_value(&pr)?;
                            GitHubToolResponse {
                                action: "pr_view".to_string(),
                                success: true,
                                repo: effective_repo,
                                backend: "gh_cli".to_string(),
                                data,
                                message: format!("Retrieved PR #{}", pr.number),
                                summary: Some(summary),
                            }
                        }
                        Err(e) => {
                            if let Some(r) = &effective_repo {
                                let endpoint = format!("repos/{r}/pulls/{target}");
                                let api_val = query_github_api(&endpoint, &ctx.env).await?;
                                let pr = normalize_pr_json(&api_val);
                                let summary = format_pr_view_summary(&pr);
                                let data = serde_json::to_value(&pr)?;
                                GitHubToolResponse {
                                    action: "pr_view".to_string(),
                                    success: true,
                                    repo: Some(r.clone()),
                                    backend: "github_api".to_string(),
                                    data,
                                    message: format!("Retrieved PR #{} via GitHub API", pr.number),
                                    summary: Some(summary),
                                }
                            } else {
                                anyhow::bail!("Failed to view PR '{target}': {e}");
                            }
                        }
                    }
                } else if let Some(r) = &effective_repo {
                    let endpoint = format!("repos/{r}/pulls/{target}");
                    let api_val = query_github_api(&endpoint, &ctx.env).await?;
                    let pr = normalize_pr_json(&api_val);
                    let summary = format_pr_view_summary(&pr);
                    let data = serde_json::to_value(&pr)?;
                    GitHubToolResponse {
                        action: "pr_view".to_string(),
                        success: true,
                        repo: Some(r.clone()),
                        backend: "github_api".to_string(),
                        data,
                        message: format!("Retrieved PR #{} via GitHub API", pr.number),
                        summary: Some(summary),
                    }
                } else {
                    anyhow::bail!("GitHub CLI ('gh') is not installed and no remote repository was detected.");
                }
            }

            // --- PR Diff ---
            "pr_diff" | "diff_pr" => {
                let target = pr_target.ok_or_else(|| {
                    anyhow::anyhow!("Missing required parameter 'pr_number' or 'target' for pr_diff")
                })?;

                if gh_available {
                    let diff_text = gh_pr_diff(effective_repo.as_deref(), &target, &ctx.cwd, &ctx.env).await?;
                    GitHubToolResponse {
                        action: "pr_diff".to_string(),
                        success: true,
                        repo: effective_repo,
                        backend: "gh_cli".to_string(),
                        data: json!({ "target": target, "diff": diff_text }),
                        message: format!("Retrieved diff for PR {target}"),
                        summary: Some(format!("```diff\n{}\n```", diff_text)),
                    }
                } else {
                    anyhow::bail!("'pr_diff' requires the 'gh' CLI tool.");
                }
            }

            // --- PR Commits ---
            "pr_commits" | "commits_pr" => {
                let target = pr_target.ok_or_else(|| {
                    anyhow::anyhow!("Missing required parameter 'pr_number' or 'target' for pr_commits")
                })?;

                let pr = gh_view_pr(effective_repo.as_deref(), &target, &ctx.cwd, &ctx.env).await?;
                let data = serde_json::to_value(&pr.commits)?;
                GitHubToolResponse {
                    action: "pr_commits".to_string(),
                    success: true,
                    repo: effective_repo,
                    backend: "gh_cli".to_string(),
                    data,
                    message: format!("Retrieved {} commits for PR #{}", pr.commits.len(), pr.number),
                    summary: None,
                }
            }

            // --- PR Status Checks ---
            "pr_checks" | "checks_pr" | "status_checks" => {
                let target = pr_target.ok_or_else(|| {
                    anyhow::anyhow!("Missing required parameter 'pr_number' or 'target' for pr_checks")
                })?;

                let pr = gh_view_pr(effective_repo.as_deref(), &target, &ctx.cwd, &ctx.env).await?;
                let data = serde_json::to_value(&pr.status_checks)?;
                GitHubToolResponse {
                    action: "pr_checks".to_string(),
                    success: true,
                    repo: effective_repo,
                    backend: "gh_cli".to_string(),
                    data,
                    message: format!("Retrieved {} status checks for PR #{}", pr.status_checks.len(), pr.number),
                    summary: None,
                }
            }

            // --- PR Reviews ---
            "pr_reviews" | "reviews_pr" => {
                let target = pr_target.ok_or_else(|| {
                    anyhow::anyhow!("Missing required parameter 'pr_number' or 'target' for pr_reviews")
                })?;

                let pr = gh_view_pr(effective_repo.as_deref(), &target, &ctx.cwd, &ctx.env).await?;
                let data = serde_json::to_value(&pr.reviews)?;
                GitHubToolResponse {
                    action: "pr_reviews".to_string(),
                    success: true,
                    repo: effective_repo,
                    backend: "gh_cli".to_string(),
                    data,
                    message: format!("Retrieved {} reviews for PR #{}", pr.reviews.len(), pr.number),
                    summary: None,
                }
            }

            // --- Issue Listing ---
            "issue_list" | "list_issues" | "issues" => {
                if gh_available {
                    let report = gh_list_issues(
                        effective_repo.as_deref(),
                        state_param,
                        limit_param,
                        author_param,
                        assignee_param,
                        label_param,
                        milestone_param,
                        search_param,
                        &ctx.cwd,
                        &ctx.env,
                    )
                    .await?;

                    let summary = format_issue_list_summary(&report);
                    let data = serde_json::to_value(&report)?;
                    GitHubToolResponse {
                        action: "issue_list".to_string(),
                        success: true,
                        repo: Some(report.repo),
                        backend: "gh_cli".to_string(),
                        data,
                        message: format!("Retrieved {} issues", report.total_count),
                        summary: Some(summary),
                    }
                } else if let Some(r) = &effective_repo {
                    let endpoint = format!("repos/{r}/issues?state={state_param}&per_page={limit_param}");
                    let api_val = query_github_api(&endpoint, &ctx.env).await?;
                    let mut issues = Vec::new();
                    if let Some(arr) = api_val.as_array() {
                        for item in arr {
                            // Filter out pull requests if API returns them under issues
                            if item.get("pull_request").is_none() {
                                issues.push(normalize_issue_json(item));
                            }
                        }
                    }
                    let total_count = issues.len();
                    let report = GitHubIssueListReport {
                        repo: r.clone(),
                        total_count,
                        issues,
                    };
                    let summary = format_issue_list_summary(&report);
                    let data = serde_json::to_value(&report)?;
                    GitHubToolResponse {
                        action: "issue_list".to_string(),
                        success: true,
                        repo: Some(r.clone()),
                        backend: "github_api".to_string(),
                        data,
                        message: format!("Retrieved {} issues via GitHub API", total_count),
                        summary: Some(summary),
                    }
                } else {
                    anyhow::bail!("GitHub CLI ('gh') is not installed and no remote repository was detected.");
                }
            }

            // --- Issue View ---
            "issue_view" | "view_issue" | "issue" => {
                let target = issue_target.ok_or_else(|| {
                    anyhow::anyhow!("Missing required parameter 'issue_number' or 'target' for issue_view")
                })?;

                if gh_available {
                    let issue = gh_view_issue(effective_repo.as_deref(), &target, &ctx.cwd, &ctx.env).await?;
                    let summary = format_issue_view_summary(&issue);
                    let data = serde_json::to_value(&issue)?;
                    GitHubToolResponse {
                        action: "issue_view".to_string(),
                        success: true,
                        repo: effective_repo,
                        backend: "gh_cli".to_string(),
                        data,
                        message: format!("Retrieved Issue #{}", issue.number),
                        summary: Some(summary),
                    }
                } else if let Some(r) = &effective_repo {
                    let endpoint = format!("repos/{r}/issues/{target}");
                    let api_val = query_github_api(&endpoint, &ctx.env).await?;
                    let issue = normalize_issue_json(&api_val);
                    let summary = format_issue_view_summary(&issue);
                    let data = serde_json::to_value(&issue)?;
                    GitHubToolResponse {
                        action: "issue_view".to_string(),
                        success: true,
                        repo: Some(r.clone()),
                        backend: "github_api".to_string(),
                        data,
                        message: format!("Retrieved Issue #{} via GitHub API", issue.number),
                        summary: Some(summary),
                    }
                } else {
                    anyhow::bail!("GitHub CLI ('gh') is not installed and no remote repository was detected.");
                }
            }

            // --- Issue Comments ---
            "issue_comments" | "comments_issue" => {
                let target = issue_target.ok_or_else(|| {
                    anyhow::anyhow!("Missing required parameter 'issue_number' or 'target' for issue_comments")
                })?;

                let issue = gh_view_issue(effective_repo.as_deref(), &target, &ctx.cwd, &ctx.env).await?;
                let data = serde_json::to_value(&issue.comments)?;
                GitHubToolResponse {
                    action: "issue_comments".to_string(),
                    success: true,
                    repo: effective_repo,
                    backend: "gh_cli".to_string(),
                    data,
                    message: format!("Retrieved {} comments for Issue #{}", issue.comments.len(), issue.number),
                    summary: None,
                }
            }

            // --- Repo View ---
            "repo_view" | "view_repo" | "repo_info" => {
                if gh_available {
                    let repo_info = gh_view_repo(effective_repo.as_deref(), &ctx.cwd, &ctx.env).await?;
                    let summary = format_repo_view_summary(&repo_info);
                    let data = serde_json::to_value(&repo_info)?;
                    GitHubToolResponse {
                        action: "repo_view".to_string(),
                        success: true,
                        repo: Some(repo_info.full_name.clone()),
                        backend: "gh_cli".to_string(),
                        data,
                        message: format!("Retrieved repository info for '{}'", repo_info.full_name),
                        summary: Some(summary),
                    }
                } else if let Some(r) = &effective_repo {
                    let endpoint = format!("repos/{r}");
                    let api_val = query_github_api(&endpoint, &ctx.env).await?;
                    let repo_info = normalize_repo_json(&api_val);
                    let summary = format_repo_view_summary(&repo_info);
                    let data = serde_json::to_value(&repo_info)?;
                    GitHubToolResponse {
                        action: "repo_view".to_string(),
                        success: true,
                        repo: Some(repo_info.full_name.clone()),
                        backend: "github_api".to_string(),
                        data,
                        message: format!("Retrieved repository info for '{}' via GitHub API", repo_info.full_name),
                        summary: Some(summary),
                    }
                } else {
                    anyhow::bail!("GitHub CLI ('gh') is not installed and no remote repository was detected.");
                }
            }

            // --- Release Listing ---
            "release_list" | "list_releases" | "releases" => {
                if gh_available {
                    let report = gh_list_releases(effective_repo.as_deref(), limit_param, &ctx.cwd, &ctx.env).await?;
                    let summary = format_release_list_summary(&report);
                    let data = serde_json::to_value(&report)?;
                    GitHubToolResponse {
                        action: "release_list".to_string(),
                        success: true,
                        repo: Some(report.repo),
                        backend: "gh_cli".to_string(),
                        data,
                        message: format!("Retrieved {} releases", report.total_count),
                        summary: Some(summary),
                    }
                } else if let Some(r) = &effective_repo {
                    let endpoint = format!("repos/{r}/releases?per_page={limit_param}");
                    let api_val = query_github_api(&endpoint, &ctx.env).await?;
                    let mut releases = Vec::new();
                    if let Some(arr) = api_val.as_array() {
                        for item in arr {
                            releases.push(normalize_release_json(item));
                        }
                    }
                    let total_count = releases.len();
                    let report = GitHubReleaseListReport {
                        repo: r.clone(),
                        total_count,
                        releases,
                    };
                    let summary = format_release_list_summary(&report);
                    let data = serde_json::to_value(&report)?;
                    GitHubToolResponse {
                        action: "release_list".to_string(),
                        success: true,
                        repo: Some(r.clone()),
                        backend: "github_api".to_string(),
                        data,
                        message: format!("Retrieved {} releases via GitHub API", total_count),
                        summary: Some(summary),
                    }
                } else {
                    anyhow::bail!("GitHub CLI ('gh') is not installed and no remote repository was detected.");
                }
            }

            // --- Release View ---
            "release_view" | "view_release" => {
                let tag = release_target.ok_or_else(|| {
                    anyhow::anyhow!("Missing required parameter 'tag' or 'target' for release_view")
                })?;

                if gh_available {
                    let release = gh_view_release(effective_repo.as_deref(), &tag, &ctx.cwd, &ctx.env).await?;
                    let data = serde_json::to_value(&release)?;
                    GitHubToolResponse {
                        action: "release_view".to_string(),
                        success: true,
                        repo: effective_repo,
                        backend: "gh_cli".to_string(),
                        data,
                        message: format!("Retrieved release '{}'", release.tag_name),
                        summary: None,
                    }
                } else if let Some(r) = &effective_repo {
                    let endpoint = format!("repos/{r}/releases/tags/{tag}");
                    let api_val = query_github_api(&endpoint, &ctx.env).await?;
                    let release = normalize_release_json(&api_val);
                    let data = serde_json::to_value(&release)?;
                    GitHubToolResponse {
                        action: "release_view".to_string(),
                        success: true,
                        repo: Some(r.clone()),
                        backend: "github_api".to_string(),
                        data,
                        message: format!("Retrieved release '{}' via GitHub API", release.tag_name),
                        summary: None,
                    }
                } else {
                    anyhow::bail!("GitHub CLI ('gh') is not installed and no remote repository was detected.");
                }
            }

            // --- Actions Workflow Runs ---
            "run_list" | "list_runs" | "actions" => {
                if gh_available {
                    let report = gh_list_runs(
                        effective_repo.as_deref(),
                        limit_param,
                        workflow_param,
                        branch_param,
                        &ctx.cwd,
                        &ctx.env,
                    )
                    .await?;

                    let summary = format_workflow_runs_summary(&report);
                    let data = serde_json::to_value(&report)?;
                    GitHubToolResponse {
                        action: "run_list".to_string(),
                        success: true,
                        repo: Some(report.repo),
                        backend: "gh_cli".to_string(),
                        data,
                        message: format!("Retrieved {} workflow runs", report.total_count),
                        summary: Some(summary),
                    }
                } else {
                    anyhow::bail!("'run_list' requires the 'gh' CLI tool.");
                }
            }

            _ => {
                anyhow::bail!("Unknown GitHub action '{action_str}'. Supported actions: 'pr_list', 'pr_view', 'pr_diff', 'pr_commits', 'pr_checks', 'pr_reviews', 'issue_list', 'issue_view', 'issue_comments', 'repo_view', 'release_list', 'release_view', 'run_list'.");
            }
        };

        if format_param == "summary" || format_param == "markdown" {
            if let Some(summary) = &response.summary {
                return Ok(summary.clone());
            }
        }

        // Return clean formatted JSON
        serde_json::to_string_pretty(&response)
            .map_err(|e| anyhow::anyhow!("Failed serializing GitHub response: {e}"))
    }
}

// ============================================================================
// Unit & Integration Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_github_repo_from_remote() {
        assert_eq!(
            parse_github_repo_from_remote("git@github.com:theaungmyatmoe/fusion.git"),
            Some(("theaungmyatmoe".to_string(), "fusion".to_string()))
        );
        assert_eq!(
            parse_github_repo_from_remote("https://github.com/rust-lang/rust.git"),
            Some(("rust-lang".to_string(), "rust".to_string()))
        );
        assert_eq!(
            parse_github_repo_from_remote("https://github.com/tokio-rs/tokio"),
            Some(("tokio-rs".to_string(), "tokio".to_string()))
        );
        assert_eq!(
            parse_github_repo_from_remote("http://github.com/facebook/react.git?auth=token"),
            Some(("facebook".to_string(), "react".to_string()))
        );
        assert_eq!(
            parse_github_repo_from_remote("ssh://git@github.com/torvalds/linux.git"),
            Some(("torvalds".to_string(), "linux".to_string()))
        );
        assert_eq!(
            parse_github_repo_from_remote("https://api.github.com/repos/owner/repo_name"),
            Some(("owner".to_string(), "repo_name".to_string()))
        );
        assert_eq!(
            parse_github_repo_from_remote("git@ghe.company.internal:team-a/backend-service.git"),
            Some(("team-a".to_string(), "backend-service".to_string()))
        );
        assert_eq!(parse_github_repo_from_remote(""), None);
        assert_eq!(parse_github_repo_from_remote("invalid-remote-format"), None);
    }

    #[test]
    fn test_normalize_pr_json() {
        let fixture = json!({
            "number": 142,
            "title": "feat: add GitHub PR and Issue inspection tool",
            "body": "Implements github.rs with full gh CLI and REST API fallback.",
            "state": "OPEN",
            "isDraft": false,
            "url": "https://github.com/theaungmyatmoe/fusion/pull/142",
            "author": {
                "login": "octocat",
                "name": "The Octocat",
                "avatarUrl": "https://avatars.githubusercontent.com/u/583231"
            },
            "baseRefName": "main",
            "headRefName": "feat/github-tool",
            "headRefOid": "abc1234def5678",
            "additions": 450,
            "deletions": 12,
            "changedFiles": 4,
            "labels": [
                { "name": "enhancement", "color": "a2eeef", "description": "New feature" }
            ],
            "assignees": [
                { "login": "theaungmyatmoe" }
            ],
            "reviewRequests": [
                { "login": "reviewer1" }
            ],
            "milestone": {
                "title": "v0.4.0",
                "number": 2,
                "state": "open"
            },
            "mergeable": "MERGEABLE",
            "mergeStateStatus": "CLEAN",
            "createdAt": "2026-09-02T10:00:00Z",
            "updatedAt": "2026-09-02T10:30:00Z",
            "statusCheckRollup": [
                {
                    "name": "ci / test (macos-14)",
                    "status": "COMPLETED",
                    "conclusion": "SUCCESS",
                    "detailsUrl": "https://github.com/theaungmyatmoe/fusion/actions/runs/12345"
                }
            ],
            "files": [
                {
                    "path": "src/tools/github.rs",
                    "additions": 450,
                    "deletions": 0,
                    "status": "added"
                }
            ],
            "commits": [
                {
                    "oid": "abc1234",
                    "messageHeadline": "Add github tool implementation",
                    "authors": [{ "login": "octocat" }]
                }
            ]
        });

        let pr = normalize_pr_json(&fixture);
        assert_eq!(pr.number, 142);
        assert_eq!(pr.title, "feat: add GitHub PR and Issue inspection tool");
        assert_eq!(pr.state, "OPEN");
        assert!(!pr.is_draft);
        assert_eq!(pr.author.unwrap().login, "octocat");
        assert_eq!(pr.base_ref_name, Some("main".to_string()));
        assert_eq!(pr.head_ref_name, Some("feat/github-tool".to_string()));
        assert_eq!(pr.additions, Some(450));
        assert_eq!(pr.deletions, Some(12));
        assert_eq!(pr.changed_files, Some(4));
        assert_eq!(pr.labels.len(), 1);
        assert_eq!(pr.labels[0].name, "enhancement");
        assert_eq!(pr.assignees.len(), 1);
        assert_eq!(pr.assignees[0].login, "theaungmyatmoe");
        assert_eq!(pr.reviewers.len(), 1);
        assert_eq!(pr.reviewers[0].login, "reviewer1");
        assert_eq!(pr.milestone.unwrap().title, "v0.4.0");
        assert_eq!(pr.mergeable, Some("MERGEABLE".to_string()));
        assert_eq!(pr.status_checks.len(), 1);
        assert_eq!(pr.status_checks[0].conclusion, Some("SUCCESS".to_string()));
        assert_eq!(pr.files.len(), 1);
        assert_eq!(pr.files[0].path, "src/tools/github.rs");
        assert_eq!(pr.commits.len(), 1);
        assert_eq!(pr.commits[0].oid, "abc1234");
    }

    #[test]
    fn test_normalize_issue_json() {
        let fixture = json!({
            "number": 88,
            "title": "Bug: memory leak during stream parsing",
            "body": "Observed high memory usage when streaming large JSON chunks.",
            "state": "OPEN",
            "url": "https://github.com/theaungmyatmoe/fusion/issues/88",
            "author": { "login": "reporter" },
            "labels": [{ "name": "bug", "color": "d73a4a" }],
            "assignees": [{ "login": "maintainer" }],
            "comments": [
                {
                    "id": "c1",
                    "author": { "login": "maintainer" },
                    "body": "Investigating the buffer sizing.",
                    "createdAt": "2026-09-02T11:00:00Z"
                }
            ]
        });

        let issue = normalize_issue_json(&fixture);
        assert_eq!(issue.number, 88);
        assert_eq!(issue.title, "Bug: memory leak during stream parsing");
        assert_eq!(issue.state, "OPEN");
        assert_eq!(issue.author.unwrap().login, "reporter");
        assert_eq!(issue.labels.len(), 1);
        assert_eq!(issue.labels[0].name, "bug");
        assert_eq!(issue.assignees.len(), 1);
        assert_eq!(issue.comments.len(), 1);
        assert_eq!(issue.comments[0].body, "Investigating the buffer sizing.");
    }

    #[test]
    fn test_normalize_repo_json() {
        let fixture = json!({
            "name": "fusion",
            "owner": { "login": "theaungmyatmoe" },
            "nameWithOwner": "theaungmyatmoe/fusion",
            "description": "Fast AI coding assistant",
            "defaultBranchRef": { "name": "main" },
            "visibility": "PUBLIC",
            "isPrivate": false,
            "isFork": false,
            "stargazerCount": 1250,
            "forksCount": 85,
            "openIssuesCount": 14,
            "licenseInfo": { "name": "MIT License", "spdxId": "MIT" },
            "repositoryTopics": [{ "name": "rust" }, { "name": "cli" }, { "name": "ai" }],
            "url": "https://github.com/theaungmyatmoe/fusion"
        });

        let repo = normalize_repo_json(&fixture);
        assert_eq!(repo.name, "fusion");
        assert_eq!(repo.owner, "theaungmyatmoe");
        assert_eq!(repo.full_name, "theaungmyatmoe/fusion");
        assert_eq!(repo.default_branch, "main");
        assert_eq!(repo.stars_count, 1250);
        assert_eq!(repo.license, Some("MIT License".to_string()));
        assert_eq!(repo.topics, vec!["rust", "cli", "ai"]);
    }

    #[test]
    fn test_format_summaries() {
        let pr = GitHubPr {
            number: 1,
            title: "Test PR".to_string(),
            body: Some("Test body content".to_string()),
            state: "OPEN".to_string(),
            is_draft: false,
            url: "https://github.com/owner/repo/pull/1".to_string(),
            author: Some(GitHubUser::new("alice")),
            base_ref_name: Some("main".to_string()),
            head_ref_name: Some("feat/x".to_string()),
            additions: Some(10),
            deletions: Some(2),
            changed_files: Some(1),
            ..Default::default()
        };

        let summary = format_pr_view_summary(&pr);
        assert!(summary.contains("# PR #1: Test PR [OPEN]"));
        assert!(summary.contains("**Author**: @alice"));
        assert!(summary.contains("`feat/x` -> `main`"));
        assert!(summary.contains("+10 / -2 across 1 changed files"));
        assert!(summary.contains("Test body content"));

        let report = GitHubPrListReport {
            repo: "owner/repo".to_string(),
            total_count: 1,
            pull_requests: vec![pr],
        };
        let list_summary = format_pr_list_summary(&report);
        assert!(list_summary.contains("# Pull Requests for 'owner/repo' (Total: 1)"));
        assert!(list_summary.contains("- **#1** [OPEN] **Test PR** by @alice"));
    }

    #[tokio::test]
    async fn test_github_tool_parameters_schema() {
        let tool = GitHubTool::new();
        assert_eq!(tool.name(), "github");
        let params = tool.parameters();
        assert_eq!(params["type"], "object");
        assert!(params["properties"]["action"]["enum"].as_array().unwrap().len() >= 10);
        assert_eq!(params["required"][0], "action");
    }

    #[tokio::test]
    async fn test_github_tool_invalid_action() {
        let tool = GitHubTool::new();
        let ctx = ToolContext::default();
        let res = tool.execute(json!({ "action": "invalid_action_xyz" }), &ctx).await;
        assert!(res.is_err());
        assert!(res.unwrap_err().to_string().contains("Unknown GitHub action"));
    }
}
