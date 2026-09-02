use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

use crate::tools::file::resolve_path;
use crate::tools::git::{find_git_root, run_git_command};
use crate::tools::types::{Tool, ToolContext};

// ============================================================================
// Data Types
// ============================================================================

/// Represents a change to an individual file in a Git commit.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GitLogFileChange {
    /// Target or current relative file path.
    pub path: String,
    /// Previous path if the file was renamed or copied.
    pub old_path: Option<String>,
    /// Status description: "added", "modified", "deleted", "renamed", "copied", or "binary".
    pub status: String,
    /// Number of lines added/inserted (0 for binary files or deletions).
    pub insertions: usize,
    /// Number of lines removed/deleted (0 for binary files or additions).
    pub deletions: usize,
    /// Whether this file is recognized as a binary asset by Git.
    pub is_binary: bool,
}

/// Rich metadata and file change summary for a single Git commit.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GitCommitInfo {
    /// Full 40-character (or 64-character SHA-256) hexadecimal commit hash.
    pub hash: String,
    /// Short 7-to-12 character abbreviated commit hash.
    pub short_hash: String,
    /// Author's display name.
    pub author_name: String,
    /// Author's email address.
    pub author_email: String,
    /// Strict ISO 8601 formatted timestamp of the author date.
    pub author_date: String,
    /// Relative author date (e.g. "2 hours ago", "3 days ago").
    pub author_date_relative: String,
    /// Committer's display name.
    pub committer_name: String,
    /// Committer's email address.
    pub committer_email: String,
    /// Strict ISO 8601 formatted timestamp of the committer date.
    pub committer_date: String,
    /// Relative committer date (e.g. "2 hours ago").
    pub committer_date_relative: String,
    /// First line of the commit message (subject).
    pub subject: String,
    /// Detailed commit body message (excluding the subject line).
    pub body: String,
    /// List of parent commit hashes (len > 1 indicates a merge commit).
    pub parents: Vec<String>,
    /// Ref names and decorations (e.g., `HEAD -> main`, `origin/main`, `tag: v1.0.0`).
    pub refs: Vec<String>,
    /// List of files changed in this commit.
    pub files: Vec<GitLogFileChange>,
    /// Total insertions across all changed files in this commit.
    pub total_insertions: usize,
    /// Total deletions across all changed files in this commit.
    pub total_deletions: usize,
    /// Total number of changed files in this commit.
    pub total_files_changed: usize,
}

impl GitCommitInfo {
    /// Returns whether this commit is a merge commit (has more than 1 parent).
    pub fn is_merge(&self) -> bool {
        self.parents.len() > 1
    }
}

/// Output formatting mode for Git log presentation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum LogFormat {
    /// Comprehensive multi-line format with author, date, subject, body, and changed files.
    #[default]
    Detailed,
    /// Compact single-line overview with hash, refs, subject, author, relative date, and stats.
    Compact,
    /// Extremely concise single-line format: short hash + subject.
    Oneline,
    /// High-level overview showing commit header + file change summary table.
    Stat,
    /// Raw structured JSON serialization.
    Json,
}

impl LogFormat {
    pub fn from_str_opt(s: Option<&str>) -> Self {
        match s.map(|v| v.trim().to_lowercase()).as_deref() {
            Some("compact") | Some("short") => Self::Compact,
            Some("oneline") => Self::Oneline,
            Some("stat") | Some("stats") => Self::Stat,
            Some("json") => Self::Json,
            _ => Self::Detailed,
        }
    }
}

/// Query and filter options for Git log inspection.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GitLogOptions {
    /// Subdirectory or repository root to inspect.
    pub path: Option<String>,
    /// Target file path to filter history for a single file.
    pub file_path: Option<String>,
    /// Maximum number of commits to retrieve (default: 10, max: 1000).
    pub max_count: usize,
    /// Number of commits to skip from the start (pagination offset).
    pub skip: usize,
    /// Specific revision range or branch (e.g. "HEAD~5..HEAD", "main", "v1.0..v2.0").
    pub revision: Option<String>,
    /// Filter commits by author name or email pattern.
    pub author: Option<String>,
    /// Filter commits whose commit message matches a regex or string pattern.
    pub grep: Option<String>,
    /// Show commits more recent than a specific date/time (e.g. "2026-01-01", "2 weeks ago").
    pub since: Option<String>,
    /// Show commits older than a specific date/time.
    pub until: Option<String>,
    /// Whether to inspect and list changed files for each commit (default: true).
    pub show_files: bool,
    /// Filter to show only merge commits.
    pub merges_only: bool,
    /// Filter to exclude merge commits.
    pub no_merges: bool,
    /// Follow only the first parent commit upon seeing a merge commit.
    pub first_parent: bool,
    /// Output commits in reverse chronological order.
    pub reverse: bool,
    /// Chosen presentation format.
    pub format: LogFormat,
}

impl Default for GitLogOptions {
    fn default() -> Self {
        Self {
            path: None,
            file_path: None,
            max_count: 10,
            skip: 0,
            revision: None,
            author: None,
            grep: None,
            since: None,
            until: None,
            show_files: true,
            merges_only: false,
            no_merges: false,
            first_parent: false,
            reverse: false,
            format: LogFormat::Detailed,
        }
    }
}

/// Complete report of a Git log query.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GitLogReport {
    /// List of retrieved commit records.
    pub commits: Vec<GitCommitInfo>,
    /// Number of commits included in this report.
    pub total_commits: usize,
    /// Absolute filesystem path to the root of the Git repository.
    pub repo_root: String,
    /// Active branch name if known.
    pub current_branch: Option<String>,
    /// Applied revision range if specified.
    pub revision_range: Option<String>,
    /// Applied path filter if specified.
    pub filter_path: Option<String>,
    /// Applied author filter if specified.
    pub filter_author: Option<String>,
    /// Applied commit message search filter if specified.
    pub filter_grep: Option<String>,
    /// Applied since filter if specified.
    pub filter_since: Option<String>,
    /// Applied until filter if specified.
    pub filter_until: Option<String>,
    /// Pagination limit used.
    pub max_count: usize,
    /// Pagination offset used.
    pub skip: usize,
}

// ============================================================================
// Parsing Logic
// ============================================================================

const RECORD_DELIMITER: &str = "\x1e";
const FIELD_DELIMITER: char = '\x1f';
const COMMIT_MARKER: &str = "COMMIT_RECORD";

/// Parse a Git rename or copy path string from `--numstat` output.
///
/// Handles all standard Git rename notations:
/// - `"old => new"` -> `("new", Some("old"))`
/// - `"src/{old => new}/file.rs"` -> `("src/new/file.rs", Some("src/old/file.rs"))`
/// - `"{old => new}.txt"` -> `("new.txt", Some("old.txt"))`
/// - `"dir/{ => sub}/file.txt"` -> `("dir/sub/file.txt", Some("dir/file.txt"))`
/// - `"dir/{sub => }/file.txt"` -> `("dir/file.txt", Some("dir/sub/file.txt"))`
/// - `"plain/path.rs"` -> `("plain/path.rs", None)`
pub fn parse_rename_path(raw: &str) -> (String, Option<String>) {
    let raw = raw.trim();
    if let (Some(start), Some(end)) = (raw.find('{'), raw.find('}')) {
        if start < end {
            let prefix = &raw[..start];
            let inner = &raw[start + 1..end];
            let suffix = &raw[end + 1..];
            if let Some((old_part, new_part)) = inner.split_once(" => ") {
                let old_full = format!("{}{}{}", prefix, old_part, suffix);
                let new_full = format!("{}{}{}", prefix, new_part, suffix);
                let clean_old = old_full.replace("//", "/");
                let clean_new = new_full.replace("//", "/");
                return (clean_new, Some(clean_old));
            }
        }
    } else if let Some((old_part, new_part)) = raw.split_once(" => ") {
        return (new_part.trim().to_string(), Some(old_part.trim().to_string()));
    }
    (raw.to_string(), None)
}

/// Parse a single line of `git log --numstat` output.
///
/// Format: `<insertions>\t<deletions>\t<path>`
/// Binary files output: `-\t-\t<path>`
pub fn parse_numstat_line(line: &str) -> Option<GitLogFileChange> {
    let line = line.trim();
    if line.is_empty() {
        return None;
    }

    let parts: Vec<&str> = line.split('\t').collect();
    if parts.len() < 3 {
        return None;
    }

    let is_binary = parts[0] == "-" || parts[1] == "-";
    let insertions = if is_binary {
        0
    } else {
        parts[0].parse::<usize>().unwrap_or(0)
    };
    let deletions = if is_binary {
        0
    } else {
        parts[1].parse::<usize>().unwrap_or(0)
    };

    let raw_path = parts[2..].join("\t");
    let (path, old_path) = parse_rename_path(&raw_path);

    let status = if is_binary {
        "binary".to_string()
    } else if old_path.is_some() {
        "renamed".to_string()
    } else if insertions > 0 && deletions == 0 {
        "added".to_string()
    } else if insertions == 0 && deletions > 0 {
        "deleted".to_string()
    } else {
        "modified".to_string()
    };

    Some(GitLogFileChange {
        path,
        old_path,
        status,
        insertions,
        deletions,
        is_binary,
    })
}

/// Parse the raw output generated by `git log` with custom delimiter format and `--numstat`.
pub fn parse_git_log_output(raw: &str) -> Vec<GitCommitInfo> {
    let mut commits = Vec::new();

    // Split on the commit record start marker
    let chunks = raw.split(COMMIT_MARKER);

    for chunk in chunks {
        let chunk = chunk.trim_start();
        if chunk.is_empty() {
            continue;
        }

        // Each chunk starts with FIELD_DELIMITER, then fields up to RECORD_DELIMITER,
        // then the trailing numstat file list.
        let (metadata_section, numstat_section) = if let Some(idx) = chunk.find(RECORD_DELIMITER) {
            (&chunk[..idx], &chunk[idx + RECORD_DELIMITER.len()..])
        } else {
            (chunk, "")
        };

        let trimmed_meta = metadata_section.strip_prefix(FIELD_DELIMITER).unwrap_or(metadata_section);
        let fields: Vec<&str> = trimmed_meta.split(FIELD_DELIMITER).collect();

        if fields.is_empty() || fields[0].trim().is_empty() {
            continue;
        }

        let hash = fields.first().copied().unwrap_or("").trim().to_string();
        let short_hash = fields.get(1).copied().unwrap_or("").trim().to_string();
        let author_name = fields.get(2).copied().unwrap_or("").trim().to_string();
        let author_email = fields.get(3).copied().unwrap_or("").trim().to_string();
        let author_date = fields.get(4).copied().unwrap_or("").trim().to_string();
        let author_date_relative = fields.get(5).copied().unwrap_or("").trim().to_string();
        let committer_name = fields.get(6).copied().unwrap_or("").trim().to_string();
        let committer_email = fields.get(7).copied().unwrap_or("").trim().to_string();
        let committer_date = fields.get(8).copied().unwrap_or("").trim().to_string();
        let committer_date_relative = fields.get(9).copied().unwrap_or("").trim().to_string();
        let parents_raw = fields.get(10).copied().unwrap_or("").trim();
        let refs_raw = fields.get(11).copied().unwrap_or("").trim();
        let subject = fields.get(12).copied().unwrap_or("").trim().to_string();
        let body = fields.get(13).copied().unwrap_or("").trim().to_string();

        let parents = if parents_raw.is_empty() {
            Vec::new()
        } else {
            parents_raw.split_whitespace().map(|s| s.to_string()).collect()
        };

        let refs = if refs_raw.is_empty() {
            Vec::new()
        } else {
            refs_raw
                .split(',')
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect()
        };

        // Parse numstat file changes
        let mut files = Vec::new();
        let mut total_insertions = 0;
        let mut total_deletions = 0;

        for line in numstat_section.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with(COMMIT_MARKER) {
                continue;
            }
            if let Some(change) = parse_numstat_line(line) {
                total_insertions += change.insertions;
                total_deletions += change.deletions;
                files.push(change);
            }
        }

        let total_files_changed = files.len();

        commits.push(GitCommitInfo {
            hash,
            short_hash,
            author_name,
            author_email,
            author_date,
            author_date_relative,
            committer_name,
            committer_email,
            committer_date,
            committer_date_relative,
            subject,
            body,
            parents,
            refs,
            files,
            total_insertions,
            total_deletions,
            total_files_changed,
        });
    }

    commits
}

// ============================================================================
// Formatting
// ============================================================================

/// Colorize unified log output lines with ANSI codes.
pub fn colorize_log(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 512);
    for line in text.lines() {
        if line.starts_with("commit ") || line.starts_with("Commit: ") {
            out.push_str("\x1b[1;33m");
            out.push_str(line);
            out.push_str("\x1b[0m\n");
        } else if line.starts_with("Author: ") || line.starts_with("Committer: ") {
            out.push_str("\x1b[1;36m");
            out.push_str(line);
            out.push_str("\x1b[0m\n");
        } else if line.starts_with("Date: ") {
            out.push_str("\x1b[34m");
            out.push_str(line);
            out.push_str("\x1b[0m\n");
        } else if line.starts_with("  + ") || line.contains("(+") {
            out.push_str("\x1b[32m");
            out.push_str(line);
            out.push_str("\x1b[0m\n");
        } else if line.contains("(-") {
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

/// Format the `GitLogReport` into human-readable text according to the chosen format.
pub fn format_log_report(report: &GitLogReport, format: LogFormat) -> String {
    match format {
        LogFormat::Json => {
            serde_json::to_string_pretty(report).unwrap_or_else(|_| "{}".to_string())
        }
        LogFormat::Oneline => {
            if report.commits.is_empty() {
                return "No commits found matching query criteria.\n".to_string();
            }
            let mut out = String::new();
            for c in &report.commits {
                let refs_str = if c.refs.is_empty() {
                    String::new()
                } else {
                    format!(" ({})", c.refs.join(", "))
                };
                out.push_str(&format!("{} {}{}\n", c.short_hash, c.subject, refs_str));
            }
            out
        }
        LogFormat::Compact => {
            if report.commits.is_empty() {
                return "No commits found matching query criteria.\n".to_string();
            }
            let mut out = String::new();
            for c in &report.commits {
                let refs_str = if c.refs.is_empty() {
                    String::new()
                } else {
                    format!(" ({})", c.refs.join(", "))
                };

                let stat_str = if c.total_files_changed > 0 {
                    format!(
                        " [{} file{}, +{}, -{}]",
                        c.total_files_changed,
                        if c.total_files_changed == 1 { "" } else { "s" },
                        c.total_insertions,
                        c.total_deletions
                    )
                } else {
                    String::new()
                };

                let author_date = if !c.author_date_relative.is_empty() {
                    format!(" - {} ({})", c.author_name, c.author_date_relative)
                } else {
                    format!(" - {}", c.author_name)
                };

                out.push_str(&format!(
                    "* {}{}: {}{}{}\n",
                    c.short_hash, refs_str, c.subject, author_date, stat_str
                ));
            }
            out
        }
        LogFormat::Stat => {
            if report.commits.is_empty() {
                return "No commits found matching query criteria.\n".to_string();
            }
            let mut out = String::new();
            for c in &report.commits {
                let refs_str = if c.refs.is_empty() {
                    String::new()
                } else {
                    format!(" ({})", c.refs.join(", "))
                };
                out.push_str(&format!("commit {}{}\n", c.hash, refs_str));
                out.push_str(&format!("Author: {} <{}>\n", c.author_name, c.author_email));
                out.push_str(&format!(
                    "Date:   {} ({})\n",
                    c.author_date, c.author_date_relative
                ));
                out.push_str(&format!("\n    {}\n\n", c.subject));

                if !c.files.is_empty() {
                    for f in &c.files {
                        let path_display = if let Some(old) = &f.old_path {
                            format!("{} => {}", old, f.path)
                        } else {
                            f.path.clone()
                        };

                        if f.is_binary {
                            out.push_str(&format!(" {:<40} | Bin\n", path_display));
                        } else {
                            let total = f.insertions + f.deletions;
                            let plus = "+".repeat(f.insertions.min(30));
                            let minus = "-".repeat(f.deletions.min(30));
                            out.push_str(&format!(
                                " {:<40} | {:>4} {}{}\n",
                                path_display, total, plus, minus
                            ));
                        }
                    }
                    out.push_str(&format!(
                        " {} file{} changed, {} insertion{}(+), {} deletion{}(-)\n\n",
                        c.total_files_changed,
                        if c.total_files_changed == 1 { "" } else { "s" },
                        c.total_insertions,
                        if c.total_insertions == 1 { "" } else { "s" },
                        c.total_deletions,
                        if c.total_deletions == 1 { "" } else { "s" },
                    ));
                } else {
                    out.push('\n');
                }
            }
            out
        }
        LogFormat::Detailed => {
            if report.commits.is_empty() {
                return "No commits found matching query criteria.\n".to_string();
            }
            let mut out = String::new();

            // Header summary if filtered
            let mut filter_tags = Vec::new();
            if let Some(rev) = &report.revision_range {
                filter_tags.push(format!("range: {rev}"));
            }
            if let Some(path) = &report.filter_path {
                filter_tags.push(format!("path: {path}"));
            }
            if let Some(author) = &report.filter_author {
                filter_tags.push(format!("author: {author}"));
            }
            if let Some(grep) = &report.filter_grep {
                filter_tags.push(format!("grep: {grep}"));
            }
            if let Some(since) = &report.filter_since {
                filter_tags.push(format!("since: {since}"));
            }
            if let Some(until) = &report.filter_until {
                filter_tags.push(format!("until: {until}"));
            }

            if !filter_tags.is_empty() {
                out.push_str(&format!(
                    "Showing {} commit{} [{}]\n\n",
                    report.total_commits,
                    if report.total_commits == 1 { "" } else { "s" },
                    filter_tags.join(", ")
                ));
            }

            for (i, c) in report.commits.iter().enumerate() {
                if i > 0 {
                    out.push_str("\n--------------------------------------------------------------------------------\n\n");
                }

                let refs_str = if c.refs.is_empty() {
                    String::new()
                } else {
                    format!(" ({})", c.refs.join(", "))
                };

                out.push_str(&format!("Commit:   {}{}\n", c.hash, refs_str));
                out.push_str(&format!("Author:   {} <{}>\n", c.author_name, c.author_email));
                if !c.author_date.is_empty() {
                    if !c.author_date_relative.is_empty() {
                        out.push_str(&format!(
                            "Date:     {} ({})\n",
                            c.author_date, c.author_date_relative
                        ));
                    } else {
                        out.push_str(&format!("Date:     {}\n", c.author_date));
                    }
                }

                if c.committer_name != c.author_name || c.committer_email != c.author_email {
                    out.push_str(&format!(
                        "Commit:   {} <{}>\n",
                        c.committer_name, c.committer_email
                    ));
                }

                if !c.parents.is_empty() {
                    let parent_shorts: Vec<String> = c
                        .parents
                        .iter()
                        .map(|p| p.chars().take(8).collect())
                        .collect();
                    out.push_str(&format!(
                        "Parents:  {}{}\n",
                        parent_shorts.join(" "),
                        if c.is_merge() { " (merge)" } else { "" }
                    ));
                }

                out.push_str(&format!("\n    {}\n", c.subject));
                if !c.body.is_empty() {
                    out.push('\n');
                    for line in c.body.lines() {
                        out.push_str(&format!("    {}\n", line));
                    }
                }

                if !c.files.is_empty() {
                    out.push_str(&format!(
                        "\nChanged files ({} file{}, +{}, -{}):\n",
                        c.total_files_changed,
                        if c.total_files_changed == 1 { "" } else { "s" },
                        c.total_insertions,
                        c.total_deletions
                    ));

                    for f in &c.files {
                        let status_code = match f.status.as_str() {
                            "added" => "A",
                            "deleted" => "D",
                            "renamed" => "R",
                            "copied" => "C",
                            "binary" => "B",
                            _ => "M",
                        };

                        let stat_info = if f.is_binary {
                            "binary".to_string()
                        } else {
                            format!("+{}, -{}", f.insertions, f.deletions)
                        };

                        if let Some(old) = &f.old_path {
                            out.push_str(&format!(
                                "  {}  {} => {} ({})\n",
                                status_code, old, f.path, stat_info
                            ));
                        } else {
                            out.push_str(&format!(
                                "  {}  {} ({})\n",
                                status_code, f.path, stat_info
                            ));
                        }
                    }
                }
            }

            out
        }
    }
}

// ============================================================================
// Execution Engine
// ============================================================================

/// Execute a Git log query and return a structured `GitLogReport`.
pub async fn get_git_log(
    repo_dir: &Path,
    options: &GitLogOptions,
) -> anyhow::Result<GitLogReport> {
    let repo_root = find_git_root(repo_dir).ok_or_else(|| {
        anyhow::anyhow!(
            "Not a git repository (or any parent directory): {}",
            repo_dir.display()
        )
    })?;

    // Format specifier:
    // COMMIT_RECORD\x1f%H\x1f%h\x1f%an\x1f%ae\x1f%aI\x1f%ar\x1f%cn\x1f%ce\x1f%cI\x1f%cr\x1f%P\x1f%D\x1f%s\x1f%b\x1e
    let format_arg = format!(
        "--format=format:{COMMIT_MARKER}{FIELD_DELIMITER}%H{FIELD_DELIMITER}%h{FIELD_DELIMITER}%an{FIELD_DELIMITER}%ae{FIELD_DELIMITER}%aI{FIELD_DELIMITER}%ar{FIELD_DELIMITER}%cn{FIELD_DELIMITER}%ce{FIELD_DELIMITER}%cI{FIELD_DELIMITER}%cr{FIELD_DELIMITER}%P{FIELD_DELIMITER}%D{FIELD_DELIMITER}%s{FIELD_DELIMITER}%b{RECORD_DELIMITER}"
    );

    let max_count_str = options.max_count.clamp(1, 2000).to_string();
    let skip_str = options.skip.to_string();

    let mut git_args: Vec<String> = vec![
        "log".to_string(),
        format_arg,
        "-n".to_string(),
        max_count_str,
    ];

    if options.skip > 0 {
        git_args.push(format!("--skip={skip_str}"));
    }

    if options.show_files {
        git_args.push("--numstat".to_string());
    }

    if options.merges_only {
        git_args.push("--merges".to_string());
    } else if options.no_merges {
        git_args.push("--no-merges".to_string());
    }

    if options.first_parent {
        git_args.push("--first-parent".to_string());
    }

    if options.reverse {
        git_args.push("--reverse".to_string());
    }

    if let Some(author) = &options.author {
        if !author.trim().is_empty() {
            git_args.push(format!("--author={}", author.trim()));
        }
    }

    if let Some(grep) = &options.grep {
        if !grep.trim().is_empty() {
            git_args.push(format!("--grep={}", grep.trim()));
        }
    }

    if let Some(since) = &options.since {
        if !since.trim().is_empty() {
            git_args.push(format!("--since={}", since.trim()));
        }
    }

    if let Some(until) = &options.until {
        if !until.trim().is_empty() {
            git_args.push(format!("--until={}", until.trim()));
        }
    }

    if let Some(rev) = &options.revision {
        if !rev.trim().is_empty() {
            git_args.push(rev.trim().to_string());
        }
    }

    // Determine path filter
    let path_filter_str = options
        .file_path
        .as_ref()
        .or(options.path.as_ref())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());

    let mut relative_filter_path = None;

    if let Some(path_str) = &path_filter_str {
        let abs_path = resolve_path(path_str, repo_dir);
        if let Ok(rel) = abs_path.strip_prefix(&repo_root) {
            let s = rel.to_string_lossy().to_string();
            if !s.is_empty() && s != "." {
                git_args.push("--".to_string());
                git_args.push(s.clone());
                relative_filter_path = Some(s);
            }
        } else {
            git_args.push("--".to_string());
            git_args.push(path_str.clone());
            relative_filter_path = Some(path_str.clone());
        }
    }

    let str_args: Vec<&str> = git_args.iter().map(|s| s.as_str()).collect();
    let output = run_git_command(&str_args, &repo_root, 30).await?;

    if !output.success {
        let err_msg = output.stderr.trim();
        // Check for empty repository / unborn branch
        if err_msg.contains("does not have any commits yet")
            || err_msg.contains("fatal: your current branch")
            || err_msg.contains("bad default revision 'HEAD'")
        {
            return Ok(GitLogReport {
                commits: Vec::new(),
                total_commits: 0,
                repo_root: repo_root.to_string_lossy().to_string(),
                current_branch: None,
                revision_range: options.revision.clone(),
                filter_path: relative_filter_path,
                filter_author: options.author.clone(),
                filter_grep: options.grep.clone(),
                filter_since: options.since.clone(),
                filter_until: options.until.clone(),
                max_count: options.max_count,
                skip: options.skip,
            });
        }
        anyhow::bail!("git log failed: {err_msg}");
    }

    let commits = parse_git_log_output(&output.stdout);
    let total_commits = commits.len();

    // Query active branch name
    let branch_output = run_git_command(&["branch", "--show-current"], &repo_root, 5).await;
    let current_branch = branch_output
        .ok()
        .filter(|o| o.success)
        .map(|o| o.stdout.trim().to_string())
        .filter(|b| !b.is_empty());

    Ok(GitLogReport {
        commits,
        total_commits,
        repo_root: repo_root.to_string_lossy().to_string(),
        current_branch,
        revision_range: options.revision.clone(),
        filter_path: relative_filter_path,
        filter_author: options.author.clone(),
        filter_grep: options.grep.clone(),
        filter_since: options.since.clone(),
        filter_until: options.until.clone(),
        max_count: options.max_count,
        skip: options.skip,
    })
}

// ============================================================================
// Tool Implementation
// ============================================================================

#[derive(Default, Debug, Clone)]
pub struct GitLogTool;

impl GitLogTool {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl Tool for GitLogTool {
    fn name(&self) -> &str {
        "git_log"
    }

    fn description(&self) -> &str {
        "Inspect Git commit history showing recent commits, authors, commit dates, subjects, bodies, and changed files with diff statistics."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "action": {
                    "type": "string",
                    "enum": ["list", "diff", "blame"],
                    "description": "Action to perform: 'list' (default) shows recent commits, 'diff' shows the full diff of a specific commit, 'blame' shows line-by-line authorship for a file range."
                },
                "path": {
                    "type": "string",
                    "description": "Optional repository path or directory to inspect (defaults to current working directory)."
                },
                "file_path": {
                    "type": "string",
                    "description": "File path to filter history for, or the file to blame (required for 'blame' action)."
                },
                "commit": {
                    "type": "string",
                    "description": "Commit hash or ref to show the diff for (required for 'diff' action)."
                },
                "start_line": {
                    "type": "integer",
                    "description": "First line number to annotate (inclusive, 1-based). Used with 'blame' action."
                },
                "end_line": {
                    "type": "integer",
                    "description": "Last line number to annotate (inclusive, 1-based). Used with 'blame' action."
                },
                "max_count": {
                    "type": "integer",
                    "description": "Maximum number of commits to return (default: 10, max: 1000)."
                },
                "limit": {
                    "type": "integer",
                    "description": "Alias for max_count."
                },
                "skip": {
                    "type": "integer",
                    "description": "Number of commits to skip from the beginning (pagination offset, default: 0)."
                },
                "revision": {
                    "type": "string",
                    "description": "Specific revision, branch, or range (e.g. 'HEAD~5..HEAD', 'main', 'feat..main')."
                },
                "author": {
                    "type": "string",
                    "description": "Filter commits by author name or email pattern."
                },
                "grep": {
                    "type": "string",
                    "description": "Filter commits by matching text in the commit message subject/body."
                },
                "since": {
                    "type": "string",
                    "description": "Show commits more recent than a specific date/time (e.g. '2026-01-01', '2 weeks ago')."
                },
                "until": {
                    "type": "string",
                    "description": "Show commits older than a specific date/time."
                },
                "show_files": {
                    "type": "boolean",
                    "description": "Whether to list changed files and diff stats for each commit (default: true)."
                },
                "merges_only": {
                    "type": "boolean",
                    "description": "If true, show only merge commits."
                },
                "no_merges": {
                    "type": "boolean",
                    "description": "If true, exclude merge commits."
                },
                "first_parent": {
                    "type": "boolean",
                    "description": "If true, follow only the first parent commit on merge commits."
                },
                "reverse": {
                    "type": "boolean",
                    "description": "If true, output commits in reverse chronological order."
                },
                "oneline": {
                    "type": "boolean",
                    "description": "If true, output concise one-line commit summaries."
                },
                "format": {
                    "type": "string",
                    "enum": ["detailed", "compact", "oneline", "stat", "json"],
                    "description": "Output formatting style (default: 'detailed')."
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

        let file_path = args
            .get("file_path")
            .or_else(|| args.get("file"))
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());

        let max_count = args
            .get("max_count")
            .or_else(|| args.get("limit"))
            .or_else(|| args.get("count"))
            .or_else(|| args.get("n"))
            .and_then(|v| v.as_u64())
            .map(|n| n as usize)
            .unwrap_or(10);

        let skip = args
            .get("skip")
            .or_else(|| args.get("offset"))
            .and_then(|v| v.as_u64())
            .map(|n| n as usize)
            .unwrap_or(0);

        let revision = args
            .get("revision")
            .or_else(|| args.get("rev"))
            .or_else(|| args.get("range"))
            .or_else(|| args.get("branch"))
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());

        let author = args.get("author").and_then(|v| v.as_str()).map(|s| s.to_string());
        let grep = args
            .get("grep")
            .or_else(|| args.get("query"))
            .or_else(|| args.get("search"))
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());

        let since = args
            .get("since")
            .or_else(|| args.get("after"))
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());

        let until = args
            .get("until")
            .or_else(|| args.get("before"))
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());

        let show_files = args
            .get("show_files")
            .or_else(|| args.get("files"))
            .or_else(|| args.get("stat"))
            .and_then(|v| v.as_bool())
            .unwrap_or(true);

        let merges_only = args
            .get("merges_only")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        let no_merges = args
            .get("no_merges")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        let first_parent = args
            .get("first_parent")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        let reverse = args
            .get("reverse")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        let oneline = args
            .get("oneline")
            .or_else(|| args.get("compact"))
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        let format_str = args.get("format").and_then(|v| v.as_str());
        let format = if oneline && format_str.is_none() {
            LogFormat::Oneline
        } else {
            LogFormat::from_str_opt(format_str)
        };

        let options = GitLogOptions {
            path: args.get("path").and_then(|v| v.as_str()).map(|s| s.to_string()),
            file_path,
            max_count,
            skip,
            revision,
            author,
            grep,
            since,
            until,
            show_files,
            merges_only,
            no_merges,
            first_parent,
            reverse,
            format,
        };

        let report = get_git_log(&target_dir, &options).await?;
        Ok(format_log_report(&report, format))
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
            let path = std::env::temp_dir().join(format!("fusion_gitlog_test_{}", uuid::Uuid::new_v4()));
            fs::create_dir_all(&path).unwrap();

            Command::new("git")
                .args(["init", "-b", "main"])
                .current_dir(&path)
                .status()
                .expect("git init");
            Command::new("git")
                .args(["config", "user.name", "Test User"])
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
    fn test_parse_rename_path() {
        // Plain path
        let (p1, old1) = parse_rename_path("src/tools/git_log.rs");
        assert_eq!(p1, "src/tools/git_log.rs");
        assert_eq!(old1, None);

        // Simple rename
        let (p2, old2) = parse_rename_path("old.rs => new.rs");
        assert_eq!(p2, "new.rs");
        assert_eq!(old2, Some("old.rs".to_string()));

        // Curly brace rename in directory
        let (p3, old3) = parse_rename_path("src/{tools => internal}/git_log.rs");
        assert_eq!(p3, "src/internal/git_log.rs");
        assert_eq!(old3, Some("src/tools/git_log.rs".to_string()));

        // Curly brace rename with prefix & suffix
        let (p4, old4) = parse_rename_path("{old => new}.txt");
        assert_eq!(p4, "new.txt");
        assert_eq!(old4, Some("old.txt".to_string()));

        // Curly brace moving into subfolder
        let (p5, old5) = parse_rename_path("dir/{ => sub}/file.txt");
        assert_eq!(p5, "dir/sub/file.txt");
        assert_eq!(old5, Some("dir/file.txt".to_string()));
    }

    #[test]
    fn test_parse_numstat_line() {
        // Normal modification
        let change1 = parse_numstat_line("12\t4\tsrc/main.rs").unwrap();
        assert_eq!(change1.path, "src/main.rs");
        assert_eq!(change1.old_path, None);
        assert_eq!(change1.insertions, 12);
        assert_eq!(change1.deletions, 4);
        assert_eq!(change1.status, "modified");
        assert!(!change1.is_binary);

        // Addition
        let change2 = parse_numstat_line("45\t0\tsrc/new.rs").unwrap();
        assert_eq!(change2.status, "added");
        assert_eq!(change2.insertions, 45);
        assert_eq!(change2.deletions, 0);

        // Deletion
        let change3 = parse_numstat_line("0\t20\tsrc/deleted.rs").unwrap();
        assert_eq!(change3.status, "deleted");
        assert_eq!(change3.insertions, 0);
        assert_eq!(change3.deletions, 20);

        // Binary
        let change4 = parse_numstat_line("-\t-\tassets/logo.png").unwrap();
        assert_eq!(change4.path, "assets/logo.png");
        assert_eq!(change4.status, "binary");
        assert!(change4.is_binary);

        // Rename
        let change5 = parse_numstat_line("5\t2\tsrc/{old => new}.rs").unwrap();
        assert_eq!(change5.path, "src/new.rs");
        assert_eq!(change5.old_path, Some("src/old.rs".to_string()));
        assert_eq!(change5.status, "renamed");
    }

    #[test]
    fn test_parse_git_log_output() {
        let sample = format!(
            "{COMMIT_MARKER}{FIELD_DELIMITER}1111111111111111111111111111111111111111{FIELD_DELIMITER}1111111{FIELD_DELIMITER}Alice Smith{FIELD_DELIMITER}alice@example.com{FIELD_DELIMITER}2026-09-02T12:00:00Z{FIELD_DELIMITER}2 hours ago{FIELD_DELIMITER}Alice Smith{FIELD_DELIMITER}alice@example.com{FIELD_DELIMITER}2026-09-02T12:00:00Z{FIELD_DELIMITER}2 hours ago{FIELD_DELIMITER}0000000{FIELD_DELIMITER}HEAD -> main, tag: v1.0{FIELD_DELIMITER}feat: initial commit{FIELD_DELIMITER}Detailed explanation here.{RECORD_DELIMITER}\n10\t0\tsrc/lib.rs\n5\t2\tsrc/main.rs\n"
        );

        let commits = parse_git_log_output(&sample);
        assert_eq!(commits.len(), 1);

        let c = &commits[0];
        assert_eq!(c.hash, "1111111111111111111111111111111111111111");
        assert_eq!(c.short_hash, "1111111");
        assert_eq!(c.author_name, "Alice Smith");
        assert_eq!(c.author_email, "alice@example.com");
        assert_eq!(c.author_date, "2026-09-02T12:00:00Z");
        assert_eq!(c.author_date_relative, "2 hours ago");
        assert_eq!(c.subject, "feat: initial commit");
        assert_eq!(c.body, "Detailed explanation here.");
        assert_eq!(c.parents, vec!["0000000"]);
        assert_eq!(c.refs, vec!["HEAD -> main", "tag: v1.0"]);
        assert_eq!(c.files.len(), 2);
        assert_eq!(c.total_insertions, 15);
        assert_eq!(c.total_deletions, 2);
        assert_eq!(c.total_files_changed, 2);
    }

    #[test]
    fn test_format_modes() {
        let commit = GitCommitInfo {
            hash: "abcdef1234567890abcdef1234567890abcdef12".to_string(),
            short_hash: "abcdef1".to_string(),
            author_name: "Bob Builder".to_string(),
            author_email: "bob@example.com".to_string(),
            author_date: "2026-09-01T10:00:00Z".to_string(),
            author_date_relative: "1 day ago".to_string(),
            committer_name: "Bob Builder".to_string(),
            committer_email: "bob@example.com".to_string(),
            committer_date: "2026-09-01T10:00:00Z".to_string(),
            committer_date_relative: "1 day ago".to_string(),
            subject: "fix: resolve edge case in parser".to_string(),
            body: "Fixes parsing when inputs contain delimiters.".to_string(),
            parents: vec!["1111111".to_string()],
            refs: vec!["HEAD -> main".to_string()],
            files: vec![GitLogFileChange {
                path: "src/parser.rs".to_string(),
                old_path: None,
                status: "modified".to_string(),
                insertions: 8,
                deletions: 2,
                is_binary: false,
            }],
            total_insertions: 8,
            total_deletions: 2,
            total_files_changed: 1,
        };

        let report = GitLogReport {
            commits: vec![commit],
            total_commits: 1,
            repo_root: "/path/to/repo".to_string(),
            current_branch: Some("main".to_string()),
            revision_range: None,
            filter_path: None,
            filter_author: None,
            filter_grep: None,
            filter_since: None,
            filter_until: None,
            max_count: 10,
            skip: 0,
        };

        // Test oneline format
        let oneline = format_log_report(&report, LogFormat::Oneline);
        assert!(oneline.contains("abcdef1 fix: resolve edge case in parser (HEAD -> main)"));

        // Test compact format
        let compact = format_log_report(&report, LogFormat::Compact);
        assert!(compact.contains("* abcdef1 (HEAD -> main): fix: resolve edge case in parser"));
        assert!(compact.contains("Bob Builder"));
        assert!(compact.contains("[1 file, +8, -2]"));

        // Test detailed format
        let detailed = format_log_report(&report, LogFormat::Detailed);
        assert!(detailed.contains("Commit:   abcdef1234567890abcdef1234567890abcdef12 (HEAD -> main)"));
        assert!(detailed.contains("Author:   Bob Builder <bob@example.com>"));
        assert!(detailed.contains("fix: resolve edge case in parser"));
        assert!(detailed.contains("Fixes parsing when inputs contain delimiters."));
        assert!(detailed.contains("Changed files (1 file, +8, -2):"));
        assert!(detailed.contains("M  src/parser.rs (+8, -2)"));

        // Test JSON format
        let json_str = format_log_report(&report, LogFormat::Json);
        let parsed_json: Value = serde_json::from_str(&json_str).unwrap();
        assert_eq!(parsed_json["total_commits"], 1);
        assert_eq!(parsed_json["commits"][0]["short_hash"], "abcdef1");
    }

    #[tokio::test]
    async fn test_git_log_tool_live() {
        let repo = TempGitRepo::new();
        let tool = GitLogTool::new();
        let ctx = ToolContext {
            cwd: repo.path.clone(),
            env: Default::default(),
        };

        // 1. Initial commit
        fs::write(repo.path.join("file1.txt"), "hello world\nline 2\n").unwrap();
        Command::new("git")
            .args(["add", "file1.txt"])
            .current_dir(&repo.path)
            .status()
            .unwrap();
        Command::new("git")
            .args(["commit", "-m", "feat: first commit"])
            .current_dir(&repo.path)
            .status()
            .unwrap();

        // 2. Second commit
        fs::write(repo.path.join("file2.txt"), "second file\n").unwrap();
        fs::write(repo.path.join("file1.txt"), "hello world modified\n").unwrap();
        Command::new("git")
            .args(["add", "."])
            .current_dir(&repo.path)
            .status()
            .unwrap();
        Command::new("git")
            .args(["commit", "-m", "docs: update file1 and add file2\n\nMore details in body."])
            .current_dir(&repo.path)
            .status()
            .unwrap();

        // Execute default log
        let res = tool.execute(json!({}), &ctx).await.unwrap();
        assert!(res.contains("docs: update file1 and add file2"));
        assert!(res.contains("feat: first commit"));
        assert!(res.contains("Test User"));
        assert!(res.contains("file1.txt"));
        assert!(res.contains("file2.txt"));

        // Execute compact log
        let compact_res = tool.execute(json!({"format": "compact"}), &ctx).await.unwrap();
        assert!(compact_res.contains("docs: update file1 and add file2"));
        assert!(compact_res.contains("feat: first commit"));

        // Execute oneline log
        let oneline_res = tool.execute(json!({"oneline": true}), &ctx).await.unwrap();
        assert!(oneline_res.contains("docs: update file1 and add file2"));

        // Execute filtered by limit
        let limit_res = tool.execute(json!({"max_count": 1}), &ctx).await.unwrap();
        assert!(limit_res.contains("docs: update file1 and add file2"));
        assert!(!limit_res.contains("feat: first commit"));

        // Execute filtered by grep
        let grep_res = tool.execute(json!({"grep": "first commit"}), &ctx).await.unwrap();
        assert!(grep_res.contains("feat: first commit"));
        assert!(!grep_res.contains("docs: update file1"));

        // Execute filtered by file_path
        let file_res = tool.execute(json!({"file_path": "file2.txt"}), &ctx).await.unwrap();
        assert!(file_res.contains("docs: update file1 and add file2"));
        assert!(!file_res.contains("feat: first commit"));
    }
}
