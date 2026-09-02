use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use similar::{ChangeTag, TextDiff};
use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::tools::file::resolve_path;
use crate::tools::types::{Tool, ToolContext};

/// Output of an executed git process.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GitProcessOutput {
    pub stdout: String,
    pub stderr: String,
    pub exit_code: Option<i32>,
    pub success: bool,
}

/// Find the enclosing git repository root containing `start_path`.
/// Inspects `.git` as either a directory or a file (for submodules/worktrees).
pub fn find_git_root(start_path: &Path) -> Option<PathBuf> {
    let mut current = if start_path.is_file() {
        start_path.parent()?.to_path_buf()
    } else {
        start_path.to_path_buf()
    };

    loop {
        let git_marker = current.join(".git");
        if git_marker.exists() {
            return Some(current);
        }
        if !current.pop() {
            break;
        }
    }
    None
}

/// Execute a raw git command within the given repository directory.
pub async fn run_git_command(
    args: &[&str],
    repo_dir: &Path,
    timeout_secs: u64,
) -> anyhow::Result<GitProcessOutput> {
    if !repo_dir.exists() {
        anyhow::bail!("Directory does not exist: {}", repo_dir.display());
    }

    let mut cmd = tokio::process::Command::new("git");
    cmd.current_dir(repo_dir);
    cmd.args(args);
    cmd.stdin(std::process::Stdio::null());
    cmd.stdout(std::process::Stdio::piped());
    cmd.stderr(std::process::Stdio::piped());
    cmd.kill_on_drop(true);

    let child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            anyhow::bail!(
                "Git binary ('git') not found in PATH. Please install git to use git tools."
            );
        }
        Err(e) => {
            anyhow::bail!("Failed to spawn git process: {e}");
        }
    };

    let timeout_duration = Duration::from_secs(timeout_secs.max(1));
    let output = match tokio::time::timeout(timeout_duration, child.wait_with_output()).await {
        Ok(Ok(out)) => out,
        Ok(Err(e)) => anyhow::bail!("Failed reading git output: {e}"),
        Err(_) => anyhow::bail!("Git command timed out after {timeout_secs}s: git {}", args.join(" ")),
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

// ============================================================================
// Git Status Data Types & Parsing
// ============================================================================

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GitFileChange {
    pub path: String,
    pub original_path: Option<String>,
    pub staged_code: Option<char>,
    pub unstaged_code: Option<char>,
    pub is_untracked: bool,
    pub is_conflict: bool,
}

impl GitFileChange {
    pub fn staged_description(&self) -> Option<&'static str> {
        match self.staged_code {
            Some('M') => Some("modified"),
            Some('A') => Some("new file"),
            Some('D') => Some("deleted"),
            Some('R') => Some("renamed"),
            Some('C') => Some("copied"),
            Some('U') => Some("unmerged"),
            _ => None,
        }
    }

    pub fn unstaged_description(&self) -> Option<&'static str> {
        match self.unstaged_code {
            Some('M') => Some("modified"),
            Some('D') => Some("deleted"),
            Some('T') => Some("type change"),
            Some('U') => Some("unmerged"),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct GitStatusReport {
    pub branch: String,
    pub upstream: Option<String>,
    pub ahead: usize,
    pub behind: usize,
    pub staged: Vec<GitFileChange>,
    pub unstaged: Vec<GitFileChange>,
    pub untracked: Vec<String>,
    pub conflicts: Vec<String>,
}

impl GitStatusReport {
    pub fn is_clean(&self) -> bool {
        self.staged.is_empty()
            && self.unstaged.is_empty()
            && self.untracked.is_empty()
            && self.conflicts.is_empty()
    }

    pub fn total_changed_files(&self) -> usize {
        self.staged.len() + self.unstaged.len() + self.untracked.len() + self.conflicts.len()
    }
}

/// Parse `git status --porcelain=v1 -b -u` output into a `GitStatusReport`.
pub fn parse_porcelain_status(raw: &str) -> GitStatusReport {
    let mut report = GitStatusReport::default();

    for line in raw.lines() {
        if line.is_empty() {
            continue;
        }

        if let Some(branch_info) = line.strip_prefix("## ") {
            parse_branch_header(branch_info, &mut report);
            continue;
        }

        if line.len() < 3 {
            continue;
        }

        let mut chars = line.chars();
        let x = chars.next().unwrap_or(' ');
        let y = chars.next().unwrap_or(' ');
        let rest = line[2..].trim_start();

        // Check for untracked file
        if x == '?' && y == '?' {
            report.untracked.push(rest.to_string());
            continue;
        }

        // Check for conflicts (DD, AU, UD, UA, DU, AA, UU)
        let is_conflict = matches!(
            (x, y),
            ('D', 'D')
                | ('A', 'U')
                | ('U', 'D')
                | ('U', 'A')
                | ('D', 'U')
                | ('A', 'A')
                | ('U', 'U')
        );

        let (path, orig_path) = if let Some((orig, target)) = rest.split_once(" -> ") {
            (target.to_string(), Some(orig.to_string()))
        } else {
            (rest.to_string(), None)
        };

        if is_conflict {
            report.conflicts.push(path.clone());
        }

        let item = GitFileChange {
            path: path.clone(),
            original_path: orig_path,
            staged_code: if x != ' ' && x != '?' && x != '!' {
                Some(x)
            } else {
                None
            },
            unstaged_code: if y != ' ' && y != '?' && y != '!' {
                Some(y)
            } else {
                None
            },
            is_untracked: false,
            is_conflict,
        };

        if item.staged_code.is_some() && !is_conflict {
            report.staged.push(item.clone());
        }

        if item.unstaged_code.is_some() && !is_conflict {
            report.unstaged.push(item);
        }
    }

    report
}

fn parse_branch_header(header: &str, report: &mut GitStatusReport) {
    // Examples:
    // "main"
    // "main...origin/main"
    // "main...origin/main [ahead 1]"
    // "main...origin/main [behind 2]"
    // "main...origin/main [ahead 1, behind 2]"
    // "Initial commit on main" / "No commits yet on main"
    // "HEAD (no branch)"
    let header = header.trim();
    if header.starts_with("No commits yet on ") {
        report.branch = header.trim_start_matches("No commits yet on ").to_string();
        return;
    }
    if header.starts_with("Initial commit on ") {
        report.branch = header.trim_start_matches("Initial commit on ").to_string();
        return;
    }

    let (branch_part, tracking_part) = if let Some(bracket_idx) = header.find(" [") {
        let branch_section = &header[..bracket_idx];
        let track_section = &header[bracket_idx + 2..header.len().saturating_sub(1)];
        (branch_section, Some(track_section))
    } else {
        (header, None)
    };

    if let Some((local, remote)) = branch_part.split_once("...") {
        report.branch = local.to_string();
        report.upstream = Some(remote.to_string());
    } else {
        report.branch = branch_part.to_string();
    }

    if let Some(track) = tracking_part {
        for part in track.split(',') {
            let p = part.trim();
            if let Some(ahead_str) = p.strip_prefix("ahead ") {
                if let Ok(n) = ahead_str.parse::<usize>() {
                    report.ahead = n;
                }
            } else if let Some(behind_str) = p.strip_prefix("behind ") {
                if let Ok(n) = behind_str.parse::<usize>() {
                    report.behind = n;
                }
            }
        }
    }
}

/// Format a `GitStatusReport` into human- and LLM-friendly text.
pub fn format_status_report(report: &GitStatusReport) -> String {
    let mut out = String::new();

    let branch_desc = if report.branch.is_empty() {
        "HEAD (detached)".to_string()
    } else {
        report.branch.clone()
    };

    out.push_str(&format!("On branch: {branch_desc}\n"));

    if let Some(upstream) = &report.upstream {
        let mut track_note = format!("Tracking: {upstream}");
        if report.ahead > 0 && report.behind > 0 {
            track_note.push_str(&format!(" [ahead {}, behind {}]", report.ahead, report.behind));
        } else if report.ahead > 0 {
            track_note.push_str(&format!(" [ahead {}]", report.ahead));
        } else if report.behind > 0 {
            track_note.push_str(&format!(" [behind {}]", report.behind));
        } else {
            track_note.push_str(" [up to date]");
        }
        out.push_str(&format!("{track_note}\n"));
    }

    out.push('\n');

    if report.is_clean() {
        out.push_str("nothing to commit, working tree clean\n");
        return out;
    }

    if !report.conflicts.is_empty() {
        out.push_str("Unmerged paths (conflicts):\n");
        for c in &report.conflicts {
            out.push_str(&format!("  both modified:   {c}\n"));
        }
        out.push('\n');
    }

    if !report.staged.is_empty() {
        out.push_str("Changes to be committed (staged):\n");
        for item in &report.staged {
            let desc = item.staged_description().unwrap_or("modified");
            if let Some(orig) = &item.original_path {
                out.push_str(&format!("  {:12} {} -> {}\n", desc, orig, item.path));
            } else {
                out.push_str(&format!("  {:12} {}\n", desc, item.path));
            }
        }
        out.push('\n');
    }

    if !report.unstaged.is_empty() {
        out.push_str("Changes not staged for commit (unstaged):\n");
        for item in &report.unstaged {
            let desc = item.unstaged_description().unwrap_or("modified");
            out.push_str(&format!("  {:12} {}\n", desc, item.path));
        }
        out.push('\n');
    }

    if !report.untracked.is_empty() {
        out.push_str("Untracked files:\n");
        for f in &report.untracked {
            out.push_str(&format!("  {f}\n"));
        }
        out.push('\n');
    }

    out
}

// ============================================================================
// GitStatusTool
// ============================================================================

#[derive(Default, Debug, Clone)]
pub struct GitStatusTool;

impl GitStatusTool {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl Tool for GitStatusTool {
    fn name(&self) -> &str {
        "git_status"
    }

    fn description(&self) -> &str {
        "Inspect the git status of the workspace, listing staged, unstaged, and untracked modified files and branch tracking info."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "Optional repository or subdirectory path (defaults to current workspace directory)."
                },
                "short": {
                    "type": "boolean",
                    "description": "If true, return short porcelain status format. Defaults to false (structured human-readable summary)."
                },
                "untracked": {
                    "type": "boolean",
                    "description": "Whether to list untracked files (defaults to true)."
                },
                "staged_only": {
                    "type": "boolean",
                    "description": "If true, filter to show only staged files."
                },
                "unstaged_only": {
                    "type": "boolean",
                    "description": "If true, filter to show only unstaged modified files."
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

        let untracked = args
            .get("untracked")
            .and_then(|v| v.as_bool())
            .unwrap_or(true);
        let short = args.get("short").and_then(|v| v.as_bool()).unwrap_or(false);
        let staged_only = args
            .get("staged_only")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        let unstaged_only = args
            .get("unstaged_only")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        let mut git_args = vec!["status", "--porcelain=v1", "-b"];
        if untracked {
            git_args.push("-uall");
        } else {
            git_args.push("-uno");
        }

        let output = run_git_command(&git_args, &repo_root, 30).await?;
        if !output.success {
            let err = if !output.stderr.trim().is_empty() {
                output.stderr.trim()
            } else {
                "git status failed with unknown error"
            };
            anyhow::bail!("{err}");
        }

        if short && !staged_only && !unstaged_only {
            return Ok(output.stdout);
        }

        let mut report = parse_porcelain_status(&output.stdout);

        if staged_only {
            report.unstaged.clear();
            report.untracked.clear();
            report.conflicts.clear();
        } else if unstaged_only {
            report.staged.clear();
            report.untracked.clear();
            report.conflicts.clear();
        }

        if short {
            // Filtered short view
            let mut lines = Vec::new();
            lines.push(format!("## {}", report.branch));
            for s in &report.staged {
                lines.push(format!("{}  {}", s.staged_code.unwrap_or('M'), s.path));
            }
            for u in &report.unstaged {
                lines.push(format!(" {} {}", u.unstaged_code.unwrap_or('M'), u.path));
            }
            for unt in &report.untracked {
                lines.push(format!("?? {}", unt));
            }
            Ok(lines.join("\n") + "\n")
        } else {
            Ok(format_status_report(&report))
        }
    }
}

// ============================================================================
// GitDiffTool
// ============================================================================

/// Colorize unified diff lines using standard ANSI escape codes.
pub fn colorize_diff(diff: &str) -> String {
    let mut out = String::with_capacity(diff.len() + 256);
    for line in diff.lines() {
        if line.starts_with("diff --git") || line.starts_with("index ") {
            out.push_str("\x1b[1m");
            out.push_str(line);
            out.push_str("\x1b[0m\n");
        } else if line.starts_with("--- ") || line.starts_with("+++ ") {
            out.push_str("\x1b[1;37m");
            out.push_str(line);
            out.push_str("\x1b[0m\n");
        } else if line.starts_with("@@ ") {
            out.push_str("\x1b[36m");
            out.push_str(line);
            out.push_str("\x1b[0m\n");
        } else if line.starts_with('+') && !line.starts_with("+++") {
            out.push_str("\x1b[32m");
            out.push_str(line);
            out.push_str("\x1b[0m\n");
        } else if line.starts_with('-') && !line.starts_with("---") {
            out.push_str("\x1b[31m");
            out.push_str(line);
            out.push_str("\x1b[0m\n");
        } else {
            out.push_str(line);
            out.push('\n');
        }
    }
    out
}

#[derive(Default, Debug, Clone)]
pub struct GitDiffTool;

impl GitDiffTool {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl Tool for GitDiffTool {
    fn name(&self) -> &str {
        "git_diff"
    }

    fn description(&self) -> &str {
        "Generate unified diffs of workspace changes (staged, unstaged, specific files, commits, or diffstat summaries)."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "Optional file or directory path to limit the diff to (relative to workspace or absolute)."
                },
                "staged": {
                    "type": "boolean",
                    "description": "If true, show diff of staged changes (`--cached` / `--staged`). Defaults to false."
                },
                "commit": {
                    "type": "string",
                    "description": "Optional commit, revision, or branch to compare against (e.g. 'HEAD~1', 'main', 'abc1234')."
                },
                "base": {
                    "type": "string",
                    "description": "Optional base commit/branch when comparing a range (e.g. base: 'main', commit: 'feature')."
                },
                "context_lines": {
                    "type": "integer",
                    "description": "Number of context lines in unified diff (defaults to 3)."
                },
                "stat": {
                    "type": "boolean",
                    "description": "If true, output a diffstat summary instead of full line-by-line patch."
                },
                "name_only": {
                    "type": "boolean",
                    "description": "If true, list only names of changed files."
                },
                "color": {
                    "type": "boolean",
                    "description": "If true, format diff with ANSI color codes (green additions, red deletions, cyan hunks)."
                },
                "ignore_whitespace": {
                    "type": "boolean",
                    "description": "If true, ignore whitespace changes (-w)."
                }
            }
        })
    }

    async fn execute(&self, args: Value, ctx: &ToolContext) -> anyhow::Result<String> {
        let repo_root = find_git_root(&ctx.cwd).ok_or_else(|| {
            anyhow::anyhow!(
                "Not a git repository (or any parent directory): {}",
                ctx.cwd.display()
            )
        })?;

        let staged = args.get("staged").and_then(|v| v.as_bool()).unwrap_or(false);
        let stat = args.get("stat").and_then(|v| v.as_bool()).unwrap_or(false);
        let name_only = args
            .get("name_only")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        let color = args.get("color").and_then(|v| v.as_bool()).unwrap_or(false);
        let ignore_whitespace = args
            .get("ignore_whitespace")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        let context_lines = args
            .get("context_lines")
            .and_then(|v| v.as_u64())
            .unwrap_or(3);

        let context_arg = format!("-U{}", context_lines);

        let mut git_args: Vec<String> = vec!["diff".to_string(), context_arg];

        if staged {
            git_args.push("--staged".to_string());
        }

        if stat {
            git_args.push("--stat".to_string());
        }

        if name_only {
            git_args.push("--name-only".to_string());
        }

        if ignore_whitespace {
            git_args.push("-w".to_string());
        }

        // Handle commit / revision comparison
        let commit_opt = args.get("commit").and_then(|v| v.as_str());
        let base_opt = args.get("base").and_then(|v| v.as_str());

        if let (Some(base), Some(commit)) = (base_opt, commit_opt) {
            git_args.push(format!("{base}..{commit}"));
        } else if let Some(commit) = commit_opt {
            git_args.push(commit.to_string());
        }

        // Target file or directory path
        let specified_path = args.get("path").and_then(|v| v.as_str());
        let mut rel_path_string = None;

        if let Some(path_str) = specified_path {
            let abs_path = resolve_path(path_str, &ctx.cwd);
            // Relativize against repo root if possible
            if let Ok(rel) = abs_path.strip_prefix(&repo_root) {
                let s = rel.to_string_lossy().to_string();
                git_args.push("--".to_string());
                git_args.push(s.clone());
                rel_path_string = Some(s);
            } else {
                git_args.push("--".to_string());
                git_args.push(path_str.to_string());
                rel_path_string = Some(path_str.to_string());
            }
        }

        let str_args: Vec<&str> = git_args.iter().map(|s| s.as_str()).collect();
        let output = run_git_command(&str_args, &repo_root, 30).await?;

        if !output.success {
            let err = if !output.stderr.trim().is_empty() {
                output.stderr.trim()
            } else {
                "git diff failed"
            };
            anyhow::bail!("{err}");
        }

        let mut diff_text = output.stdout;

        // If diff is empty and a specific file was requested, check if it's an untracked file.
        // For untracked new files, generate a unified diff against an empty string so the caller
        // sees the proposed additions.
        if diff_text.trim().is_empty() && !staged && commit_opt.is_none() {
            if let Some(path_str) = specified_path {
                let abs_path = resolve_path(path_str, &ctx.cwd);
                if abs_path.is_file() {
                    if let Ok(content) = tokio::fs::read_to_string(&abs_path).await {
                        let display_path = rel_path_string.as_deref().unwrap_or(path_str);
                        if stat {
                            let lines = content.lines().count();
                            diff_text = format!(" {} | {} +++++++\n 1 file changed, {} insertions(+)\n", display_path, lines, lines);
                        } else if name_only {
                            diff_text = format!("{display_path}\n");
                        } else {
                            let diff = TextDiff::from_lines("", &content);
                            diff_text = diff
                                .unified_diff()
                                .context_radius(context_lines as usize)
                                .header("/dev/null", &format!("b/{}", display_path))
                                .to_string();
                        }
                    }
                }
            }
        }

        if diff_text.trim().is_empty() {
            return Ok("No differences found (working tree clean or no changes match the criteria).\n".to_string());
        }

        if color && !stat && !name_only {
            Ok(colorize_diff(&diff_text))
        } else {
            Ok(diff_text)
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
            path.push(format!("fusion_git_test_{id}"));
            fs::create_dir_all(&path).expect("failed to create temp git dir");

            // Initialize git repository
            let status = Command::new("git")
                .arg("init")
                .current_dir(&path)
                .status()
                .expect("failed to execute git init");
            assert!(status.success(), "git init must succeed");

            // Configure git user for commits in test
            Command::new("git")
                .args(["config", "user.name", "Fusion Test"])
                .current_dir(&path)
                .status()
                .expect("git config user.name");
            Command::new("git")
                .args(["config", "user.email", "test@fusion.local"])
                .current_dir(&path)
                .status()
                .expect("git config user.email");

            Self { path }
        }
    }

    impl Drop for TempGitRepo {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.path);
        }
    }

    #[test]
    fn test_find_git_root() {
        let repo = TempGitRepo::new();
        let sub = repo.path.join("a").join("b");
        fs::create_dir_all(&sub).unwrap();

        let found = find_git_root(&sub);
        assert_eq!(found, Some(repo.path.clone()));

        let found_root = find_git_root(&repo.path);
        assert_eq!(found_root, Some(repo.path.clone()));
    }

    #[test]
    fn test_parse_porcelain_status() {
        let sample = "\
## main...origin/main [ahead 2, behind 1]
M  staged_file.rs
 M unstaged_file.rs
MM both_file.rs
A  new_staged.rs
?? untracked.txt
UU conflict.rs
";
        let report = parse_porcelain_status(sample);
        assert_eq!(report.branch, "main");
        assert_eq!(report.upstream, Some("origin/main".to_string()));
        assert_eq!(report.ahead, 2);
        assert_eq!(report.behind, 1);

        assert_eq!(report.staged.len(), 3); // M, MM, A
        assert_eq!(report.unstaged.len(), 2); // M, MM
        assert_eq!(report.untracked.len(), 1);
        assert_eq!(report.untracked[0], "untracked.txt");
        assert_eq!(report.conflicts.len(), 1);
        assert_eq!(report.conflicts[0], "conflict.rs");

        let formatted = format_status_report(&report);
        assert!(formatted.contains("On branch: main"));
        assert!(formatted.contains("Tracking: origin/main [ahead 2, behind 1]"));
        assert!(formatted.contains("Changes to be committed (staged):"));
        assert!(formatted.contains("staged_file.rs"));
        assert!(formatted.contains("Changes not staged for commit (unstaged):"));
        assert!(formatted.contains("unstaged_file.rs"));
        assert!(formatted.contains("Untracked files:"));
        assert!(formatted.contains("untracked.txt"));
        assert!(formatted.contains("Unmerged paths (conflicts):"));
    }

    #[test]
    fn test_colorize_diff() {
        let diff = "\
diff --git a/hello.rs b/hello.rs
index 1234567..89abcdef 100644
--- a/hello.rs
+++ b/hello.rs
@@ -1,3 +1,3 @@
-fn old() {}
+fn new() {}
";
        let colored = colorize_diff(diff);
        assert!(colored.contains("\x1b[31m-fn old() {}\x1b[0m"));
        assert!(colored.contains("\x1b[32m+fn new() {}\x1b[0m"));
        assert!(colored.contains("\x1b[36m@@ -1,3 +1,3 @@\x1b[0m"));
    }

    #[tokio::test]
    async fn test_git_status_tool_live() {
        let repo = TempGitRepo::new();
        let tool = GitStatusTool::new();
        let ctx = ToolContext {
            cwd: repo.path.clone(),
            env: Default::default(),
        };

        // 1. Initially clean repo (initial commit or empty)
        let res = tool.execute(json!({}), &ctx).await.unwrap();
        assert!(res.contains("working tree clean") || res.contains("nothing to commit"));

        // 2. Add an untracked file
        fs::write(repo.path.join("file1.txt"), "hello world\n").unwrap();
        let res2 = tool.execute(json!({}), &ctx).await.unwrap();
        assert!(res2.contains("Untracked files:"));
        assert!(res2.contains("file1.txt"));

        // 3. Stage the file
        Command::new("git")
            .args(["add", "file1.txt"])
            .current_dir(&repo.path)
            .status()
            .unwrap();

        let res3 = tool.execute(json!({}), &ctx).await.unwrap();
        assert!(res3.contains("Changes to be committed (staged):"));
        assert!(res3.contains("file1.txt"));

        // 4. Test short format
        let res_short = tool.execute(json!({"short": true}), &ctx).await.unwrap();
        assert!(res_short.contains("A  file1.txt"));
    }

    #[tokio::test]
    async fn test_git_diff_tool_live() {
        let repo = TempGitRepo::new();
        let diff_tool = GitDiffTool::new();
        let ctx = ToolContext {
            cwd: repo.path.clone(),
            env: Default::default(),
        };

        // Initial commit
        let test_file = repo.path.join("test.txt");
        fs::write(&test_file, "Line 1\nLine 2\nLine 3\n").unwrap();
        Command::new("git")
            .args(["add", "test.txt"])
            .current_dir(&repo.path)
            .status()
            .unwrap();
        Command::new("git")
            .args(["commit", "-m", "Initial commit"])
            .current_dir(&repo.path)
            .status()
            .unwrap();

        // No diff initially
        let res = diff_tool.execute(json!({}), &ctx).await.unwrap();
        assert!(res.contains("No differences found"));

        // Modify file
        fs::write(&test_file, "Line 1\nLine 2 modified\nLine 3\n").unwrap();

        // Unstaged diff
        let res_unstaged = diff_tool.execute(json!({}), &ctx).await.unwrap();
        assert!(res_unstaged.contains("-Line 2"));
        assert!(res_unstaged.contains("+Line 2 modified"));

        // Test stat
        let res_stat = diff_tool.execute(json!({"stat": true}), &ctx).await.unwrap();
        assert!(res_stat.contains("test.txt"));
        assert!(res_stat.contains("changed"));

        // Test name_only
        let res_names = diff_tool.execute(json!({"name_only": true}), &ctx).await.unwrap();
        assert_eq!(res_names.trim(), "test.txt");

        // Test staged diff
        Command::new("git")
            .args(["add", "test.txt"])
            .current_dir(&repo.path)
            .status()
            .unwrap();

        let res_staged = diff_tool.execute(json!({"staged": true}), &ctx).await.unwrap();
        assert!(res_staged.contains("+Line 2 modified"));

        // Test untracked file diff fallback
        let new_untracked = repo.path.join("new_file.txt");
        fs::write(&new_untracked, "Brand new file\n").unwrap();

        let res_new = diff_tool.execute(json!({"path": "new_file.txt"}), &ctx).await.unwrap();
        assert!(res_new.contains("+Brand new file"));
    }
}
