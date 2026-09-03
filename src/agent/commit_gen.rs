//! Conventional commit generator subsystem for Fusion.
//!
//! # Overview
//!
//! Analyzes unified git diffs across modified, added, renamed, or deleted files,
//! extracts syntactic and semantic change signatures, infers appropriate conventional
//! commit types (`feat`, `fix`, `refactor`, `docs`, `test`, `perf`, `build`, `ci`, `chore`),
//! determines component scopes, detects breaking changes, and formats standard
//! Conventional Commits 1.0.0 compliant messages (`feat(scope): description`).
//!
//! # Key Capabilities
//!
//! 1. **Diff Syntax & Structure Parsing**: Parses unified git diffs, hunk headers,
//!    file headers, binary indicators, additions, and deletions.
//! 2. **Scope Inference Heuristics**: Automatically infers component scopes from directory
//!    hierarchies, file extensions, and project architecture layouts.
//! 3. **Semantic Type Classification**: Classifies change intent using keyword analysis,
//!    AST signature modifications (functions, structs, traits, APIs), and file category rules.
//! 4. **Breaking Change Detection**: Identifies public API removals, breaking renames,
//!    major configuration overhauls, and `!` / `BREAKING CHANGE:` specifications.
//! 5. **Multi-Candidate Generation**: Provides prioritized commit message candidates
//!    ranging from concise single-line headers to rich multi-paragraph descriptions.
//! 6. **Full Round-Trip Parsing & Validation**: Parses and validates existing commit
//!    messages against the Conventional Commits specification.
//! 7. **LLM Prompt Formatting**: Produces structured prompts for LLM-assisted refinement.

use std::collections::HashMap;
use std::fmt;
use std::path::Path;
use std::str::FromStr;

use serde::{Deserialize, Serialize};

// ============================================================================
// Conventional Commit Types & Footers
// ============================================================================

/// Conventional Commit types according to the Conventional Commits v1.0.0 specification
/// and standard Angular / Gitmoji conventions.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CommitType {
    /// A new feature or capability (`feat`)
    Feat,
    /// A bug fix (`fix`)
    Fix,
    /// Documentation only changes (`docs`)
    Docs,
    /// Code style, formatting, missing semicolons, whitespace (`style`)
    Style,
    /// Code changes that neither fix a bug nor add a feature (`refactor`)
    Refactor,
    /// Code change that improves execution or memory performance (`perf`)
    Perf,
    /// Adding missing tests or correcting existing tests (`test`)
    Test,
    /// Changes that affect the build system or external dependencies (`build`)
    Build,
    /// Changes to CI/CD configuration files and scripts (`ci`)
    Ci,
    /// Other changes that do not modify src or test files (`chore`)
    Chore,
    /// Reverts a previous commit (`revert`)
    Revert,
    /// Custom user-defined commit type
    Custom(String),
}

impl CommitType {
    /// Returns the standard lowercase string representation.
    pub fn as_str(&self) -> &str {
        match self {
            Self::Feat => "feat",
            Self::Fix => "fix",
            Self::Docs => "docs",
            Self::Style => "style",
            Self::Refactor => "refactor",
            Self::Perf => "perf",
            Self::Test => "test",
            Self::Build => "build",
            Self::Ci => "ci",
            Self::Chore => "chore",
            Self::Revert => "revert",
            Self::Custom(s) => s.as_str(),
        }
    }

    /// Human-readable description of this commit type.
    pub fn description(&self) -> &str {
        match self {
            Self::Feat => "A new feature",
            Self::Fix => "A bug fix",
            Self::Docs => "Documentation only changes",
            Self::Style => "Changes that do not affect code logic (formatting, whitespace)",
            Self::Refactor => "A code change that neither fixes a bug nor adds a feature",
            Self::Perf => "A code change that improves performance",
            Self::Test => "Adding missing tests or correcting existing tests",
            Self::Build => "Changes that affect the build system or external dependencies",
            Self::Ci => "Changes to CI/CD configuration files and automation",
            Self::Chore => "Miscellaneous tasks, maintenance, or internal tooling",
            Self::Revert => "Reverts a previous commit",
            Self::Custom(_) => "Custom change type",
        }
    }

    /// Gitmoji or visual emoji associated with this commit type.
    pub fn emoji(&self) -> &str {
        match self {
            Self::Feat => "✨",
            Self::Fix => "🐛",
            Self::Docs => "📝",
            Self::Style => "💄",
            Self::Refactor => "♻️",
            Self::Perf => "⚡",
            Self::Test => "✅",
            Self::Build => "📦",
            Self::Ci => "👷",
            Self::Chore => "🔧",
            Self::Revert => "⏪",
            Self::Custom(_) => "📌",
        }
    }

    /// Parse a type string loosely (case-insensitive, trimming).
    pub fn from_str_loose(s: &str) -> Self {
        let clean = s.trim().to_lowercase();
        match clean.as_str() {
            "feat" | "feature" | "add" => Self::Feat,
            "fix" | "bugfix" | "bug" | "patch" => Self::Fix,
            "docs" | "doc" | "documentation" => Self::Docs,
            "style" | "format" | "formatting" => Self::Style,
            "refactor" | "refac" | "clean" | "cleanup" => Self::Refactor,
            "perf" | "performance" | "optimize" | "opt" => Self::Perf,
            "test" | "tests" | "testing" => Self::Test,
            "build" | "deps" | "dependency" | "dependencies" => Self::Build,
            "ci" | "cd" | "workflow" | "actions" => Self::Ci,
            "chore" | "maint" | "maintenance" => Self::Chore,
            "revert" | "rollback" => Self::Revert,
            other => {
                if !other.is_empty() {
                    Self::Custom(other.to_string())
                } else {
                    Self::Chore
                }
            }
        }
    }
}

impl fmt::Display for CommitType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

impl FromStr for CommitType {
    type Err = std::convert::Infallible;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(Self::from_str_loose(s))
    }
}

/// A structured footer element in a conventional commit (e.g. `BREAKING CHANGE: ...`, `Closes #123`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CommitFooter {
    /// Footer token (e.g. `BREAKING CHANGE`, `BREAKING-CHANGE`, `Fixes`, `Closes`, `Refs`, `Co-authored-by`).
    pub token: String,
    /// Value or message associated with the token.
    pub value: String,
}

impl CommitFooter {
    /// Create a new commit footer.
    pub fn new(token: impl Into<String>, value: impl Into<String>) -> Self {
        Self {
            token: token.into(),
            value: value.into(),
        }
    }

    /// Create a `BREAKING CHANGE` footer.
    pub fn breaking_change(description: impl Into<String>) -> Self {
        Self {
            token: "BREAKING CHANGE".to_string(),
            value: description.into(),
        }
    }

    /// Create a `Co-authored-by` footer.
    pub fn co_authored_by(author: impl Into<String>) -> Self {
        Self {
            token: "Co-authored-by".to_string(),
            value: author.into(),
        }
    }

    /// Create an issue close/fix footer.
    pub fn closes(issue: impl Into<String>) -> Self {
        Self {
            token: "Closes".to_string(),
            value: issue.into(),
        }
    }

    /// Format footer as a standard line.
    pub fn format(&self) -> String {
        format!("{}: {}", self.token, self.value)
    }

    /// Returns `true` if this footer represents a breaking change.
    pub fn is_breaking(&self) -> bool {
        self.token.eq_ignore_ascii_case("BREAKING CHANGE")
            || self.token.eq_ignore_ascii_case("BREAKING-CHANGE")
    }
}

impl fmt::Display for CommitFooter {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.token, self.value)
    }
}

// ============================================================================
// Conventional Commit Representation
// ============================================================================

/// Represents a structured Conventional Commit message.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConventionalCommit {
    /// The primary commit type (`feat`, `fix`, etc.).
    pub commit_type: CommitType,
    /// Optional scope indicating the affected module/component.
    pub scope: Option<String>,
    /// Indicates whether this change contains breaking changes (`!`).
    pub is_breaking: bool,
    /// Short, concise summary description in imperative mood.
    pub description: String,
    /// Optional longer multi-line body explaining motivation and details.
    pub body: Option<String>,
    /// Structured footers (e.g., `BREAKING CHANGE: ...`, `Closes #123`).
    pub footers: Vec<CommitFooter>,
}

impl ConventionalCommit {
    /// Create a new builder for constructing a conventional commit.
    pub fn builder() -> ConventionalCommitBuilder {
        ConventionalCommitBuilder::default()
    }

    /// Format the commit message summary line (the first line / header).
    ///
    /// Example: `feat(agent): add conventional commit generator` or `feat(api)!: drop legacy endpoint`
    pub fn format_summary(&self) -> String {
        let mut out = String::new();
        out.push_str(self.commit_type.as_str());

        if let Some(scope) = &self.scope {
            let clean_scope = scope.trim();
            if !clean_scope.is_empty() {
                out.push('(');
                out.push_str(clean_scope);
                out.push(')');
            }
        }

        if self.is_breaking {
            out.push('!');
        }

        out.push_str(": ");
        out.push_str(self.description.trim());
        out
    }

    /// Format the complete commit message following the Conventional Commits v1.0.0 specification.
    pub fn format(&self) -> String {
        let mut out = self.format_summary();

        if let Some(body) = &self.body {
            let trimmed = body.trim();
            if !trimmed.is_empty() {
                out.push_str("\n\n");
                out.push_str(trimmed);
            }
        }

        if !self.footers.is_empty() {
            out.push_str("\n\n");
            for (idx, footer) in self.footers.iter().enumerate() {
                if idx > 0 {
                    out.push('\n');
                }
                out.push_str(&footer.format());
            }
        }

        out
    }

    /// Format the summary with gitmoji icon prefix.
    pub fn format_with_emoji(&self) -> String {
        format!("{} {}", self.commit_type.emoji(), self.format_summary())
    }

    /// Validate this commit message against conventional commit rules and recommendations.
    ///
    /// Returns `Ok(())` if valid, or a list of warning/error messages.
    pub fn validate(&self) -> Result<(), Vec<String>> {
        let mut errors = Vec::new();

        if self.description.trim().is_empty() {
            errors.push("Commit description must not be empty".to_string());
        }

        if self.description.ends_with('.') {
            errors.push("Commit description should not end with a period".to_string());
        }

        let summary = self.format_summary();
        if summary.len() > 100 {
            errors.push(format!(
                "Commit header line exceeds 100 characters (length: {})",
                summary.len()
            ));
        }

        if let Some(scope) = &self.scope {
            if scope.contains(' ') {
                errors.push(
                    "Scope should not contain spaces (use hyphens or underscores)".to_string(),
                );
            }
        }

        if errors.is_empty() {
            Ok(())
        } else {
            Err(errors)
        }
    }

    /// Parse a raw git commit message string into a structured `ConventionalCommit`.
    pub fn parse(raw: &str) -> Result<Self, CommitParseError> {
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            return Err(CommitParseError::EmptyInput);
        }

        let mut lines = trimmed.lines();
        let header = lines.next().unwrap_or("").trim();
        if header.is_empty() {
            return Err(CommitParseError::EmptyInput);
        }

        // Parse header: `<type>[(<scope>)][!]: <description>`
        let colon_pos = header.find(':').ok_or_else(|| {
            CommitParseError::InvalidHeaderFormat(
                "Missing colon separator after commit type/scope".to_string(),
            )
        })?;

        let type_scope_part = header[..colon_pos].trim();
        let description = header[colon_pos + 1..].trim().to_string();

        if description.is_empty() {
            return Err(CommitParseError::InvalidHeaderFormat(
                "Missing description after colon".to_string(),
            ));
        }

        let is_breaking_header = type_scope_part.ends_with('!');
        let type_scope_core = if is_breaking_header {
            &type_scope_part[..type_scope_part.len() - 1]
        } else {
            type_scope_part
        };

        let (type_str, scope) = if let Some(open_paren) = type_scope_core.find('(') {
            let close_paren = type_scope_core.find(')').ok_or_else(|| {
                CommitParseError::InvalidHeaderFormat("Unclosed scope parenthesis".to_string())
            })?;

            let commit_type = &type_scope_core[..open_paren];
            let scope_val = &type_scope_core[open_paren + 1..close_paren];
            (commit_type.trim(), Some(scope_val.trim().to_string()))
        } else {
            (type_scope_core.trim(), None)
        };

        if type_str.is_empty() {
            return Err(CommitParseError::InvalidHeaderFormat(
                "Empty commit type".to_string(),
            ));
        }

        let commit_type = CommitType::from_str_loose(type_str);

        // Parse remaining lines into body and footers
        let mut body_paragraphs: Vec<String> = Vec::new();
        let mut footers: Vec<CommitFooter> = Vec::new();
        let mut current_block: Vec<String> = Vec::new();
        let mut in_footers = false;

        for line in lines {
            let line_str = line.to_string();
            if line_str.trim().is_empty() {
                if !current_block.is_empty() {
                    let block_text = current_block.join("\n");
                    if in_footers {
                        // try parsing block as footers
                        parse_footer_block(&block_text, &mut footers);
                    } else if is_potential_footer_block(&block_text) {
                        in_footers = true;
                        parse_footer_block(&block_text, &mut footers);
                    } else {
                        body_paragraphs.push(block_text);
                    }
                    current_block.clear();
                }
            } else {
                current_block.push(line_str);
            }
        }

        if !current_block.is_empty() {
            let block_text = current_block.join("\n");
            if in_footers || is_potential_footer_block(&block_text) {
                parse_footer_block(&block_text, &mut footers);
            } else {
                body_paragraphs.push(block_text);
            }
        }

        let body = if body_paragraphs.is_empty() {
            None
        } else {
            Some(body_paragraphs.join("\n\n"))
        };

        let has_breaking_footer = footers.iter().any(|f| f.is_breaking());
        let is_breaking = is_breaking_header || has_breaking_footer;

        Ok(Self {
            commit_type,
            scope,
            is_breaking,
            description,
            body,
            footers,
        })
    }
}

impl fmt::Display for ConventionalCommit {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.format())
    }
}

/// Helper to detect if a text block contains conventional commit footers.
fn is_potential_footer_block(text: &str) -> bool {
    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("BREAKING CHANGE:")
            || trimmed.starts_with("BREAKING-CHANGE:")
            || trimmed.starts_with("Closes:")
            || trimmed.starts_with("Fixes:")
            || trimmed.starts_with("Co-authored-by:")
            || trimmed.starts_with("Signed-off-by:")
            || trimmed.starts_with("Reviewed-by:")
            || trimmed.starts_with("Refs:")
            || (trimmed.starts_with("Closes #") || trimmed.starts_with("Fixes #"))
        {
            return true;
        }
    }
    false
}

/// Helper to parse a footer block into structured `CommitFooter` elements.
fn parse_footer_block(text: &str, footers: &mut Vec<CommitFooter>) {
    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }

        if let Some(pos) = trimmed.find(':') {
            let token = trimmed[..pos].trim().to_string();
            let value = trimmed[pos + 1..].trim().to_string();
            footers.push(CommitFooter::new(token, value));
        } else if trimmed.starts_with("Closes #") {
            let value = trimmed["Closes".len()..].trim().to_string();
            footers.push(CommitFooter::new("Closes", value));
        } else if trimmed.starts_with("Fixes #") {
            let value = trimmed["Fixes".len()..].trim().to_string();
            footers.push(CommitFooter::new("Fixes", value));
        }
    }
}

/// Errors occurring when parsing conventional commit strings.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum CommitParseError {
    #[error("Empty input commit message")]
    EmptyInput,
    #[error("Invalid commit header format: {0}")]
    InvalidHeaderFormat(String),
}

/// Builder for constructing `ConventionalCommit` instances fluently.
#[derive(Debug, Clone, Default)]
pub struct ConventionalCommitBuilder {
    commit_type: Option<CommitType>,
    scope: Option<String>,
    is_breaking: bool,
    description: Option<String>,
    body: Option<String>,
    footers: Vec<CommitFooter>,
}

impl ConventionalCommitBuilder {
    pub fn commit_type(mut self, commit_type: CommitType) -> Self {
        self.commit_type = Some(commit_type);
        self
    }

    pub fn feat(self) -> Self {
        self.commit_type(CommitType::Feat)
    }

    pub fn fix(self) -> Self {
        self.commit_type(CommitType::Fix)
    }

    pub fn refactor(self) -> Self {
        self.commit_type(CommitType::Refactor)
    }

    pub fn docs(self) -> Self {
        self.commit_type(CommitType::Docs)
    }

    pub fn test(self) -> Self {
        self.commit_type(CommitType::Test)
    }

    pub fn perf(self) -> Self {
        self.commit_type(CommitType::Perf)
    }

    pub fn chore(self) -> Self {
        self.commit_type(CommitType::Chore)
    }

    pub fn ci(self) -> Self {
        self.commit_type(CommitType::Ci)
    }

    pub fn build(self) -> Self {
        self.commit_type(CommitType::Build)
    }

    pub fn scope(mut self, scope: impl Into<String>) -> Self {
        let s = scope.into();
        let trimmed = s.trim();
        if !trimmed.is_empty() {
            self.scope = Some(trimmed.to_string());
        } else {
            self.scope = None;
        }
        self
    }

    pub fn breaking(mut self, breaking: bool) -> Self {
        self.is_breaking = breaking;
        self
    }

    pub fn description(mut self, description: impl Into<String>) -> Self {
        self.description = Some(description.into());
        self
    }

    pub fn body(mut self, body: impl Into<String>) -> Self {
        let b = body.into();
        let trimmed = b.trim();
        if !trimmed.is_empty() {
            self.body = Some(trimmed.to_string());
        } else {
            self.body = None;
        }
        self
    }

    pub fn footer(mut self, footer: CommitFooter) -> Self {
        self.footers.push(footer);
        self
    }

    pub fn breaking_change(mut self, explanation: impl Into<String>) -> Self {
        self.is_breaking = true;
        self.footers
            .push(CommitFooter::breaking_change(explanation));
        self
    }

    pub fn closes_issue(mut self, issue: impl Into<String>) -> Self {
        self.footers.push(CommitFooter::closes(issue));
        self
    }

    pub fn co_authored_by(mut self, author: impl Into<String>) -> Self {
        self.footers.push(CommitFooter::co_authored_by(author));
        self
    }

    pub fn build_commit(self) -> Result<ConventionalCommit, String> {
        let commit_type = self.commit_type.unwrap_or(CommitType::Chore);
        let description = self
            .description
            .ok_or_else(|| "Commit description is required".to_string())?;

        Ok(ConventionalCommit {
            commit_type,
            scope: self.scope,
            is_breaking: self.is_breaking,
            description,
            body: self.body,
            footers: self.footers,
        })
    }
}

// ============================================================================
// Git Diff Analysis Data Structures
// ============================================================================

/// Type of file system change observed in the git diff.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FileChangeKind {
    Added,
    Modified,
    Deleted,
    Renamed { from: String },
    Copied { from: String },
    TypeChanged,
}

/// Analysis summary for an individual file modified in a git diff.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileDiffSummary {
    /// Target file path (new path).
    pub path: String,
    /// Original file path (if renamed/copied).
    pub old_path: Option<String>,
    /// Type of change (added, modified, deleted, renamed).
    pub change_kind: FileChangeKind,
    /// Number of added lines.
    pub additions: usize,
    /// Number of deleted lines.
    pub deletions: usize,
    /// Added code lines (sampled / non-header).
    pub added_lines: Vec<String>,
    /// Deleted code lines (sampled / non-header).
    pub deleted_lines: Vec<String>,
    /// Detected symbols modified or introduced (e.g. functions, structs, traits).
    pub detected_symbols: Vec<String>,
    /// Inferred component scope for this file.
    pub suggested_scope: Option<String>,
    /// Inferred primary commit type for this file.
    pub suggested_type: Option<CommitType>,
}

impl FileDiffSummary {
    /// Returns total line churn (additions + deletions).
    pub fn churn(&self) -> usize {
        self.additions + self.deletions
    }

    /// Returns `true` if this file is a test file.
    pub fn is_test(&self) -> bool {
        let p = self.path.to_lowercase();
        p.contains("test")
            || p.contains("spec")
            || p.ends_with("_test.rs")
            || p.ends_with(".test.ts")
    }

    /// Returns `true` if this file is a documentation file.
    pub fn is_doc(&self) -> bool {
        let p = self.path.to_lowercase();
        p.ends_with(".md")
            || p.ends_with(".txt")
            || p.ends_with(".rst")
            || p.starts_with("docs/")
            || p.starts_with("doc/")
            || p == "readme.md"
            || p == "changelog.md"
    }

    /// Returns `true` if this file is a CI/CD configuration.
    pub fn is_ci(&self) -> bool {
        let p = self.path.to_lowercase();
        p.starts_with(".github/")
            || p.starts_with(".gitlab-ci")
            || p.contains("workflow")
            || p.ends_with(".travis.yml")
            || p.ends_with("azure-pipelines.yml")
    }

    /// Returns `true` if this file is build or dependency configuration.
    pub fn is_build(&self) -> bool {
        let p = self.path.to_lowercase();
        p == "cargo.toml"
            || p == "cargo.lock"
            || p == "package.json"
            || p == "package-lock.json"
            || p == "pnpm-lock.yaml"
            || p == "yarn.lock"
            || p == "go.mod"
            || p == "go.sum"
            || p == "pom.xml"
            || p == "build.gradle"
            || p == "makefile"
            || p == "cmakelists.txt"
            || p == "dockerfile"
            || p.starts_with("docker/")
            || p.ends_with(".lock")
    }

    /// Returns `true` if this file is a chore / tooling configuration file.
    pub fn is_chore(&self) -> bool {
        let p = self.path.to_lowercase();
        p == ".gitignore"
            || p == ".gitattributes"
            || p == ".editorconfig"
            || p == ".prettierrc"
            || p == ".prettierignore"
            || p == ".eslintrc"
            || p == ".eslintignore"
            || p == "license"
            || p == "license.md"
            || p == "license.txt"
            || p.ends_with(".prettierrc.json")
            || p.ends_with(".eslintrc.json")
            || p.ends_with(".toml") && !p.contains("cargo")
    }
}

/// Complete aggregated analysis of a git diff.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiffAnalysis {
    /// Summaries for each changed file.
    pub files: Vec<FileDiffSummary>,
    /// Total lines added across the entire diff.
    pub total_additions: usize,
    /// Total lines deleted across the entire diff.
    pub total_deletions: usize,
    /// Top inferred commit type across all files.
    pub inferred_type: CommitType,
    /// Top inferred component scope.
    pub inferred_scope: Option<String>,
    /// Whether any breaking changes were detected.
    pub inferred_breaking: bool,
    /// Explanations of why a breaking change was detected.
    pub breaking_reasons: Vec<String>,
    /// Key changes and modifications identified in the diff.
    pub key_changes: Vec<String>,
    /// Distinct component scopes identified.
    pub affected_components: Vec<String>,
    /// Whether the diff is completely empty.
    pub is_empty: bool,
}

impl DiffAnalysis {
    /// Returns a short one-line summary of file stats.
    pub fn stats_summary(&self) -> String {
        format!(
            "{} file(s) changed, +{} -{}",
            self.files.len(),
            self.total_additions,
            self.total_deletions
        )
    }

    /// Format a bulleted markdown summary of the files and detected symbols.
    pub fn format_markdown_breakdown(&self) -> String {
        let mut out = String::new();
        out.push_str(&format!("### Diff Summary ({})\n\n", self.stats_summary()));

        for f in &self.files {
            let kind_label = match &f.change_kind {
                FileChangeKind::Added => "added",
                FileChangeKind::Modified => "modified",
                FileChangeKind::Deleted => "deleted",
                FileChangeKind::Renamed { from } => &format!("renamed from `{}`", from),
                FileChangeKind::Copied { from } => &format!("copied from `{}`", from),
                FileChangeKind::TypeChanged => "type changed",
            };

            out.push_str(&format!(
                "- **`{}`** ({}): +{} -{}\n",
                f.path, kind_label, f.additions, f.deletions
            ));

            if !f.detected_symbols.is_empty() {
                out.push_str(&format!("  - Symbols: {}\n", f.detected_symbols.join(", ")));
            }
        }

        if !self.breaking_reasons.is_empty() {
            out.push_str("\n**Breaking Changes:**\n");
            for reason in &self.breaking_reasons {
                out.push_str(&format!("- ⚠️ {}\n", reason));
            }
        }

        out
    }
}

// ============================================================================
// Commit Generator Configuration & Engine
// ============================================================================

/// Configuration options for conventional commit generation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommitGeneratorConfig {
    /// Maximum character length for the header/subject line (standard: 72 or 50).
    pub max_subject_length: usize,
    /// Whether to generate an explanatory body section in addition to the header.
    pub include_body: bool,
    /// Whether to include file change stats in the generated body.
    pub include_diff_stats: bool,
    /// Whether to infer and attach scopes (e.g. `feat(agent): ...` vs `feat: ...`).
    pub include_scope: bool,
    /// Enforce lowercase description in conventional commit style.
    pub enforce_lower_case_subject: bool,
    /// Explicit fallback or override scope.
    pub default_scope: Option<String>,
    /// Allowed custom scopes list (if restricted).
    pub allowed_scopes: Vec<String>,
    /// Issue identifiers to attach as footers (e.g. `["#123", "PROJ-45"]`).
    pub issue_references: Vec<String>,
    /// Co-authors to include as `Co-authored-by:` footers.
    pub co_authors: Vec<String>,
    /// Whether to format with Gitmoji emoji prefix.
    pub include_emojis: bool,
}

impl Default for CommitGeneratorConfig {
    fn default() -> Self {
        Self {
            max_subject_length: 72,
            include_body: true,
            include_diff_stats: false,
            include_scope: true,
            enforce_lower_case_subject: true,
            default_scope: None,
            allowed_scopes: Vec::new(),
            issue_references: Vec::new(),
            co_authors: Vec::new(),
            include_emojis: false,
        }
    }
}

/// Conventional Commit Generator engine.
#[derive(Debug, Clone)]
pub struct CommitGenerator {
    config: CommitGeneratorConfig,
}

impl Default for CommitGenerator {
    fn default() -> Self {
        Self::new(CommitGeneratorConfig::default())
    }
}

impl CommitGenerator {
    /// Create a new commit generator with the given configuration.
    pub fn new(config: CommitGeneratorConfig) -> Self {
        Self { config }
    }

    /// Access the generator's active configuration.
    pub fn config(&self) -> &CommitGeneratorConfig {
        &self.config
    }

    /// Analyze a raw unified git diff string.
    pub fn analyze_diff(&self, diff_text: &str) -> DiffAnalysis {
        parse_git_diff(diff_text)
    }

    /// Generate a primary conventional commit message from a raw git diff.
    pub fn generate(&self, diff_text: &str) -> ConventionalCommit {
        let analysis = self.analyze_diff(diff_text);
        self.generate_from_analysis(&analysis)
    }

    /// Generate a conventional commit message from an existing `DiffAnalysis`.
    pub fn generate_from_analysis(&self, analysis: &DiffAnalysis) -> ConventionalCommit {
        if analysis.is_empty {
            return ConventionalCommit {
                commit_type: CommitType::Chore,
                scope: self.config.default_scope.clone(),
                is_breaking: false,
                description: "empty commit or no staged changes".to_string(),
                body: None,
                footers: Vec::new(),
            };
        }

        let commit_type = analysis.inferred_type.clone();

        let scope = if self.config.include_scope {
            if let Some(def_scope) = &self.config.default_scope {
                Some(def_scope.clone())
            } else {
                analysis.inferred_scope.clone()
            }
        } else {
            None
        };

        // Synthesize concise subject
        let raw_description = synthesize_subject_description(analysis, &commit_type);
        let mut description = if self.config.enforce_lower_case_subject {
            to_lower_first_char(&raw_description)
        } else {
            raw_description
        };

        // Trim trailing dot
        if description.ends_with('.') {
            description.pop();
        }

        // Clamp description length if needed
        let type_and_scope_len = commit_type.as_str().len()
            + scope.as_ref().map(|s| s.len() + 2).unwrap_or(0)
            + if analysis.inferred_breaking { 1 } else { 0 }
            + 2; // ": "

        if type_and_scope_len + description.len() > self.config.max_subject_length {
            let max_desc_len = self
                .config
                .max_subject_length
                .saturating_sub(type_and_scope_len);
            if max_desc_len > 10 && description.len() > max_desc_len {
                description.truncate(max_desc_len.saturating_sub(3));
                description.push_str("...");
            }
        }

        // Synthesize body if requested
        let body = if self.config.include_body {
            synthesize_commit_body(analysis, self.config.include_diff_stats)
        } else {
            None
        };

        // Synthesize footers
        let mut footers = Vec::new();

        if analysis.inferred_breaking && !analysis.breaking_reasons.is_empty() {
            footers.push(CommitFooter::breaking_change(
                analysis.breaking_reasons.join("; "),
            ));
        }

        for issue in &self.config.issue_references {
            footers.push(CommitFooter::closes(issue.clone()));
        }

        for author in &self.config.co_authors {
            footers.push(CommitFooter::co_authored_by(author.clone()));
        }

        ConventionalCommit {
            commit_type,
            scope,
            is_breaking: analysis.inferred_breaking,
            description,
            body,
            footers,
        }
    }

    /// Generate multiple ranked candidates (e.g. concise, detailed, alternate types).
    pub fn generate_candidates(&self, diff_text: &str, count: usize) -> Vec<ConventionalCommit> {
        let analysis = self.analyze_diff(diff_text);
        if analysis.is_empty {
            return vec![self.generate_from_analysis(&analysis)];
        }

        let primary = self.generate_from_analysis(&analysis);
        let mut candidates = Vec::with_capacity(count.max(1));
        candidates.push(primary.clone());

        if count <= 1 {
            return candidates;
        }

        // Candidate 2: Alternate concise description or different inferred type
        let secondary_type = match primary.commit_type {
            CommitType::Feat => CommitType::Refactor,
            CommitType::Refactor => CommitType::Feat,
            CommitType::Fix => CommitType::Refactor,
            CommitType::Chore => CommitType::Build,
            _ => CommitType::Chore,
        };

        let mut candidate_2 = primary.clone();
        candidate_2.commit_type = secondary_type;
        candidate_2.description =
            synthesize_alternate_description(&analysis, &candidate_2.commit_type);
        if !candidates.contains(&candidate_2) {
            candidates.push(candidate_2);
        }

        // Candidate 3: File-specific or symbol-focused description
        if candidates.len() < count && !analysis.files.is_empty() {
            if let Some(first_file) = analysis.files.first() {
                let mut candidate_3 = primary.clone();
                let file_scope = first_file
                    .suggested_scope
                    .clone()
                    .or_else(|| primary.scope.clone());
                candidate_3.scope = file_scope;

                if let Some(first_symbol) = first_file.detected_symbols.first() {
                    candidate_3.description =
                        format!("update {} in {}", first_symbol, first_file.path);
                } else {
                    candidate_3.description = format!("update {}", first_file.path);
                }

                if !candidates.contains(&candidate_3) {
                    candidates.push(candidate_3);
                }
            }
        }

        // Candidate 4: Ultra-concise single-line candidate (no body)
        if candidates.len() < count {
            let mut candidate_4 = primary.clone();
            candidate_4.body = None;
            if !candidates.contains(&candidate_4) {
                candidates.push(candidate_4);
            }
        }

        candidates.truncate(count);
        candidates
    }

    /// Format a structured prompt string suitable for asking an LLM to generate or refine
    /// a conventional commit message.
    pub fn format_prompt_for_llm(&self, diff_text: &str, analysis: &DiffAnalysis) -> String {
        format!(
            "Generate a conventional git commit message adhering to Conventional Commits v1.0.0 for the following diff.\n\n\
             ### Git Diff Stats:\n\
             - Files Changed: {}\n\
             - Total Additions: +{}\n\
             - Total Deletions: -{}\n\
             - Suggested Type: {}\n\
             - Suggested Scope: {}\n\
             - Breaking Changes: {}\n\n\
             ### Detailed Breakdown:\n{}\n\n\
             ### Raw Git Diff (truncated if large):\n```diff\n{}\n```\n\n\
             Respond ONLY with the formatted commit message in standard `<type>(<scope>): <description>` format.",
            analysis.files.len(),
            analysis.total_additions,
            analysis.total_deletions,
            analysis.inferred_type,
            analysis.inferred_scope.as_deref().unwrap_or("none"),
            if analysis.inferred_breaking { "YES" } else { "NO" },
            analysis.format_markdown_breakdown(),
            diff_text.lines().take(200).collect::<Vec<_>>().join("\n")
        )
    }

    /// Fetch the active git diff from a repository directory and generate a commit message.
    pub async fn generate_from_repo(
        repo_path: &Path,
        cached: bool,
    ) -> anyhow::Result<ConventionalCommit> {
        let diff_text = get_repo_git_diff(repo_path, cached).await?;
        let generator = CommitGenerator::default();
        Ok(generator.generate(&diff_text))
    }
}

// ============================================================================
// Git CLI Runner & Helpers
// ============================================================================

/// Query `git diff` or `git diff --cached` in the given repository path.
pub async fn get_repo_git_diff(repo_path: &Path, cached: bool) -> anyhow::Result<String> {
    let mut cmd = tokio::process::Command::new("git");
    cmd.current_dir(repo_path);
    cmd.arg("diff");
    if cached {
        cmd.arg("--cached");
    }

    let output = cmd.output().await?;
    if !output.status.success() {
        let err_msg = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!("git diff failed: {}", err_msg);
    }

    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    Ok(stdout)
}

// ============================================================================
// Git Diff Parser & Inference Engine
// ============================================================================

/// Parse a raw unified git diff into a structured `DiffAnalysis`.
pub fn parse_git_diff(diff_text: &str) -> DiffAnalysis {
    let trimmed = diff_text.trim();
    if trimmed.is_empty() {
        return DiffAnalysis {
            files: Vec::new(),
            total_additions: 0,
            total_deletions: 0,
            inferred_type: CommitType::Chore,
            inferred_scope: None,
            inferred_breaking: false,
            breaking_reasons: Vec::new(),
            key_changes: Vec::new(),
            affected_components: Vec::new(),
            is_empty: true,
        };
    }

    let mut files = Vec::new();
    let mut current_file: Option<FileDiffSummary> = None;

    let mut total_additions = 0;
    let mut total_deletions = 0;
    let mut breaking_reasons = Vec::new();
    let mut key_changes = Vec::new();

    for line in diff_text.lines() {
        if line.starts_with("diff --git ") {
            if let Some(f) = current_file.take() {
                files.push(f);
            }

            let parts: Vec<&str> = line.split_whitespace().collect();
            let (old_p, new_p) = if parts.len() >= 4 {
                let a_path = parts[2].strip_prefix("a/").unwrap_or(parts[2]);
                let b_path = parts[3].strip_prefix("b/").unwrap_or(parts[3]);
                (Some(a_path.to_string()), b_path.to_string())
            } else {
                (None, "unknown".to_string())
            };

            let scope = infer_scope_from_path(&new_p);
            current_file = Some(FileDiffSummary {
                path: new_p,
                old_path: old_p,
                change_kind: FileChangeKind::Modified,
                additions: 0,
                deletions: 0,
                added_lines: Vec::new(),
                deleted_lines: Vec::new(),
                detected_symbols: Vec::new(),
                suggested_scope: scope,
                suggested_type: None,
            });
            continue;
        }

        if let Some(f) = &mut current_file {
            if line.starts_with("new file mode") {
                f.change_kind = FileChangeKind::Added;
                continue;
            }
            if line.starts_with("deleted file mode") {
                f.change_kind = FileChangeKind::Deleted;
                continue;
            }
            if line.starts_with("rename from ") {
                let from = line["rename from ".len()..].trim().to_string();
                f.old_path = Some(from.clone());
                f.change_kind = FileChangeKind::Renamed { from };
                continue;
            }
            if line.starts_with("rename to ") {
                let to = line["rename to ".len()..].trim().to_string();
                f.path = to;
                f.suggested_scope = infer_scope_from_path(&f.path);
                continue;
            }
            if line.starts_with("similarity index ") {
                continue;
            }
            if line.starts_with("Binary files ") && line.contains(" differ") {
                f.additions += 1;
                total_additions += 1;
                continue;
            }
            if line.starts_with("GIT binary patch") {
                f.additions += 1;
                total_additions += 1;
                continue;
            }

            // Ignore diff headers
            if line.starts_with("--- ")
                || line.starts_with("+++ ")
                || line.starts_with("index ")
                || line.starts_with("@@ ")
            {
                continue;
            }

            // Added line
            if line.starts_with('+') && !line.starts_with("+++") {
                let content = &line[1..];
                f.additions += 1;
                total_additions += 1;
                if f.added_lines.len() < 50 {
                    f.added_lines.push(content.to_string());
                }

                // Detect symbols / signatures introduced
                if let Some(sym) = extract_symbol_signature(content) {
                    if !f.detected_symbols.contains(&sym) {
                        f.detected_symbols.push(sym.clone());
                        key_changes.push(format!("Add `{}` in `{}`", sym, f.path));
                    }
                }
                continue;
            }

            // Deleted line
            if line.starts_with('-') && !line.starts_with("---") {
                let content = &line[1..];
                f.deletions += 1;
                total_deletions += 1;
                if f.deleted_lines.len() < 50 {
                    f.deleted_lines.push(content.to_string());
                }

                // Check for potential breaking change in removed public APIs
                if is_breaking_deletion(content) {
                    let reason = format!("Removed public API in `{}`: {}", f.path, content.trim());
                    if !breaking_reasons.contains(&reason) {
                        breaking_reasons.push(reason);
                    }
                }
                continue;
            }
        }
    }

    if let Some(f) = current_file.take() {
        files.push(f);
    }

    // Infer primary commit type and scope across all files
    let (inferred_type, inferred_scope, affected_components) =
        infer_overall_type_and_scope(&mut files, &breaking_reasons);

    let inferred_breaking = !breaking_reasons.is_empty();

    DiffAnalysis {
        files,
        total_additions,
        total_deletions,
        inferred_type,
        inferred_scope,
        inferred_breaking,
        breaking_reasons,
        key_changes,
        affected_components,
        is_empty: false,
    }
}

/// Infer component scope string from a file path.
///
/// Examples:
/// - `src/agent/commit_gen.rs` -> `Some("agent")`
/// - `src/tools/git.rs` -> `Some("git")`
/// - `ui/slash.rs` -> `Some("ui")`
/// - `docs/index.md` -> `Some("docs")`
/// - `.github/workflows/ci.yml` -> `Some("ci")`
/// - `Cargo.toml` -> `Some("deps")`
pub fn infer_scope_from_path(path: &str) -> Option<String> {
    let clean = path.replace('\\', "/");
    let parts: Vec<&str> = clean.split('/').collect();

    if clean == "Cargo.toml"
        || clean == "Cargo.lock"
        || clean == "package.json"
        || clean == "pnpm-lock.yaml"
        || clean == "yarn.lock"
    {
        return Some("deps".to_string());
    }

    if clean.starts_with(".github/") {
        return Some("ci".to_string());
    }

    if clean.starts_with("docs/") || clean.starts_with("doc/") {
        return Some("docs".to_string());
    }

    if clean.starts_with("benches/") || clean.starts_with("benchmarks/") {
        return Some("bench".to_string());
    }

    if clean.starts_with("tests/") || clean.starts_with("test/") {
        return Some("test".to_string());
    }

    // Monorepo support: `crates/fusion-cli/src/...` or `packages/sdk/src/...`
    if (parts.len() >= 3)
        && (parts[0] == "crates"
            || parts[0] == "packages"
            || parts[0] == "apps"
            || parts[0] == "services"
            || parts[0] == "libs")
    {
        return Some(parts[1].to_string());
    }

    // `src/foo/bar.rs` -> `foo` or `bar`
    if parts.len() >= 3 && parts[0] == "src" {
        let component = parts[1];
        let file_stem = Path::new(parts[2])
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("");

        if file_stem != "mod" && file_stem != "lib" && file_stem != "main" && parts.len() == 3 {
            // For `src/tools/git.rs`, return `git`
            return Some(file_stem.to_string());
        }

        return Some(component.to_string());
    }

    if parts.len() == 2 && parts[0] == "src" {
        let file_stem = Path::new(parts[1])
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("");

        if file_stem != "lib" && file_stem != "main" {
            return Some(file_stem.to_string());
        }
    }

    if parts.len() >= 2 {
        let dir = parts[0];
        if dir != "src" {
            return Some(dir.to_string());
        }
    }

    None
}

/// Extract function, struct, trait, or class signatures from added code lines.
fn extract_symbol_signature(line: &str) -> Option<String> {
    let trimmed = line.trim();

    // Rust: `pub fn foo(...)`, `fn foo(...)`, `pub async fn foo(...)`, `async fn foo(...)`
    if let Some(idx) = trimmed.find("fn ") {
        if idx == 0
            || trimmed.starts_with("pub ")
            || trimmed.starts_with("async ")
            || trimmed.starts_with("pub async ")
        {
            let after_fn = trimmed[idx + 3..].trim();
            if let Some(paren) = after_fn.find('(') {
                let name = after_fn[..paren].trim();
                if is_valid_identifier(name) {
                    return Some(format!("{}()", name));
                }
            }
        }
    }

    // Rust: `pub struct Foo` or `struct Foo`
    if let Some(idx) = trimmed.find("struct ") {
        if idx == 0 || trimmed.starts_with("pub ") {
            let after = trimmed[idx + 7..].trim();
            let name = after
                .split(|c: char| !c.is_alphanumeric() && c != '_')
                .next()
                .unwrap_or("");
            if is_valid_identifier(name) {
                return Some(name.to_string());
            }
        }
    }

    // Rust: `pub enum Foo` or `enum Foo`
    if let Some(idx) = trimmed.find("enum ") {
        if idx == 0 || trimmed.starts_with("pub ") {
            let after = trimmed[idx + 5..].trim();
            let name = after
                .split(|c: char| !c.is_alphanumeric() && c != '_')
                .next()
                .unwrap_or("");
            if is_valid_identifier(name) {
                return Some(name.to_string());
            }
        }
    }

    // Rust: `pub trait Foo` or `trait Foo`
    if let Some(idx) = trimmed.find("trait ") {
        if idx == 0 || trimmed.starts_with("pub ") {
            let after = trimmed[idx + 6..].trim();
            let name = after
                .split(|c: char| !c.is_alphanumeric() && c != '_')
                .next()
                .unwrap_or("");
            if is_valid_identifier(name) {
                return Some(name.to_string());
            }
        }
    }

    // TypeScript/JS: `export function foo`, `function foo`
    if trimmed.starts_with("export function ") || trimmed.starts_with("function ") {
        let after = trimmed
            .strip_prefix("export function ")
            .unwrap_or_else(|| trimmed.strip_prefix("function ").unwrap_or(""));
        if let Some(paren) = after.find('(') {
            let name = after[..paren].trim();
            if is_valid_identifier(name) {
                return Some(format!("{}()", name));
            }
        }
    }

    // TypeScript/JS: `export class Foo`, `class Foo`
    if trimmed.starts_with("export class ") || trimmed.starts_with("class ") {
        let after = trimmed
            .strip_prefix("export class ")
            .unwrap_or_else(|| trimmed.strip_prefix("class ").unwrap_or(""));
        let name = after
            .split(|c: char| !c.is_alphanumeric() && c != '_')
            .next()
            .unwrap_or("");
        if is_valid_identifier(name) {
            return Some(name.to_string());
        }
    }

    // TypeScript: `export interface Foo`, `interface Foo`
    if trimmed.starts_with("export interface ") || trimmed.starts_with("interface ") {
        let after = trimmed
            .strip_prefix("export interface ")
            .unwrap_or_else(|| trimmed.strip_prefix("interface ").unwrap_or(""));
        let name = after
            .split(|c: char| !c.is_alphanumeric() && c != '_')
            .next()
            .unwrap_or("");
        if is_valid_identifier(name) {
            return Some(name.to_string());
        }
    }

    // TypeScript: `export type Foo =`
    if trimmed.starts_with("export type ") || trimmed.starts_with("type ") {
        let after = trimmed
            .strip_prefix("export type ")
            .unwrap_or_else(|| trimmed.strip_prefix("type ").unwrap_or(""));
        let name = after
            .split(|c: char| !c.is_alphanumeric() && c != '_')
            .next()
            .unwrap_or("");
        if is_valid_identifier(name) {
            return Some(name.to_string());
        }
    }

    // Go: `func Foo(` or `func (r *Recv) Foo(`
    if trimmed.starts_with("func ") {
        let after = trimmed["func ".len()..].trim();
        let after_recv = if after.starts_with('(') {
            if let Some(close_p) = after.find(')') {
                after[close_p + 1..].trim()
            } else {
                after
            }
        } else {
            after
        };
        if let Some(paren) = after_recv.find('(') {
            let name = after_recv[..paren].trim();
            if is_valid_identifier(name) {
                return Some(format!("{}()", name));
            }
        }
    }

    // Python: `def foo(...):` or `class Foo:`
    if trimmed.starts_with("def ") || trimmed.starts_with("async def ") {
        let after = trimmed
            .strip_prefix("async def ")
            .unwrap_or_else(|| trimmed.strip_prefix("def ").unwrap_or(""))
            .trim();
        if let Some(paren) = after.find('(') {
            let name = after[..paren].trim();
            if is_valid_identifier(name) {
                return Some(format!("{}()", name));
            }
        }
    }

    if trimmed.starts_with("class ") {
        let after = trimmed["class ".len()..].trim();
        let name = after
            .split(|c: char| !c.is_alphanumeric() && c != '_')
            .next()
            .unwrap_or("");
        if is_valid_identifier(name) {
            return Some(name.to_string());
        }
    }

    None
}

/// Check whether a name is a valid programming identifier.
fn is_valid_identifier(name: &str) -> bool {
    if name.is_empty() {
        return false;
    }
    let mut chars = name.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    if !first.is_alphabetic() && first != '_' {
        return false;
    }
    chars.all(|c| c.is_alphanumeric() || c == '_')
}

/// Detect if a deleted line represents a removed public API (breaking change candidate).
fn is_breaking_deletion(line: &str) -> bool {
    let trimmed = line.trim();
    if trimmed.starts_with("pub fn ")
        || trimmed.starts_with("pub async fn ")
        || trimmed.starts_with("pub struct ")
        || trimmed.starts_with("pub enum ")
        || trimmed.starts_with("pub trait ")
        || trimmed.starts_with("pub type ")
        || trimmed.starts_with("pub const ")
        || trimmed.starts_with("export function ")
        || trimmed.starts_with("export async function ")
        || trimmed.starts_with("export class ")
        || trimmed.starts_with("export interface ")
        || trimmed.starts_with("export type ")
        || trimmed.starts_with("export const ")
        || trimmed.starts_with("export enum ")
    {
        return true;
    }
    false
}

/// Infer primary commit type, overall scope, and affected components across files.
fn infer_overall_type_and_scope(
    files: &mut [FileDiffSummary],
    breaking_reasons: &[String],
) -> (CommitType, Option<String>, Vec<String>) {
    let mut type_scores: HashMap<CommitType, usize> = HashMap::new();
    let mut scope_counts: HashMap<String, usize> = HashMap::new();
    let mut components = Vec::new();

    for file in files.iter_mut() {
        let file_type = infer_file_commit_type(file);
        file.suggested_type = Some(file_type.clone());

        let weight = (file.additions + file.deletions).max(1);
        *type_scores.entry(file_type).or_insert(0) += weight;

        if let Some(scope) = &file.suggested_scope {
            *scope_counts.entry(scope.clone()).or_insert(0) += weight;
            if !components.contains(scope) {
                components.push(scope.clone());
            }
        }
    }

    // Determine highest scoring scope
    let inferred_scope = scope_counts
        .into_iter()
        .max_by_key(|(_, score)| *score)
        .map(|(scope, _)| scope);

    // Determine highest scoring type
    let inferred_type = if !breaking_reasons.is_empty() {
        // Breaking changes are typically feat or refactor
        type_scores
            .iter()
            .max_by_key(|(_, score)| *score)
            .map(|(t, _)| t.clone())
            .unwrap_or(CommitType::Feat)
    } else {
        type_scores
            .into_iter()
            .max_by_key(|(_, score)| *score)
            .map(|(t, _)| t)
            .unwrap_or(CommitType::Chore)
    };

    (inferred_type, inferred_scope, components)
}

/// Infer commit type for a single file based on path, additions, and deletions.
fn infer_file_commit_type(file: &FileDiffSummary) -> CommitType {
    if file.is_test() {
        return CommitType::Test;
    }

    if file.is_doc() {
        return CommitType::Docs;
    }

    if file.is_ci() {
        return CommitType::Ci;
    }

    if file.is_build() {
        return CommitType::Build;
    }

    if file.is_chore() {
        return CommitType::Chore;
    }
    // Scan code content for cues (bug fix keywords vs new feature vs refactor)
    let mut fix_score = 0;
    let mut feat_score = 0;
    let mut perf_score = 0;
    let mut refactor_score = 0;

    for line in &file.added_lines {
        let lower = line.to_lowercase();
        if lower.contains("fix")
            || lower.contains("bug")
            || lower.contains("patch")
            || lower.contains("error")
        {
            fix_score += 2;
        }
        if lower.contains("pub fn ")
            || lower.contains("struct ")
            || lower.contains("enum ")
            || lower.contains("feature")
        {
            feat_score += 3;
        }
        if lower.contains("perf")
            || lower.contains("cache")
            || lower.contains("optimize")
            || lower.contains("speed")
        {
            perf_score += 3;
        }
        if lower.contains("refactor") || lower.contains("rename") || lower.contains("cleanup") {
            refactor_score += 2;
        }
    }

    if matches!(file.change_kind, FileChangeKind::Added) {
        feat_score += 5;
    }

    if matches!(file.change_kind, FileChangeKind::Renamed { .. }) {
        refactor_score += 5;
    }

    let mut scores = vec![
        (CommitType::Feat, feat_score),
        (CommitType::Fix, fix_score),
        (CommitType::Perf, perf_score),
        (CommitType::Refactor, refactor_score),
    ];
    scores.sort_by(|a, b| b.1.cmp(&a.1));

    if let Some((top_type, score)) = scores.first() {
        if *score > 0 {
            return top_type.clone();
        }
    }

    if file.additions > file.deletions * 2 {
        CommitType::Feat
    } else if file.deletions > file.additions * 2 {
        CommitType::Refactor
    } else {
        CommitType::Feat
    }
}

/// Synthesize a primary subject line description based on analysis and commit type.
fn synthesize_subject_description(analysis: &DiffAnalysis, commit_type: &CommitType) -> String {
    // 1. If key symbols detected, mention them
    if !analysis.key_changes.is_empty() {
        if let Some(first_file) = analysis.files.first() {
            if !first_file.detected_symbols.is_empty() {
                let symbols = first_file.detected_symbols.join(", ");
                return match commit_type {
                    CommitType::Feat => format!("add {}", symbols),
                    CommitType::Fix => format!("fix issue in {}", symbols),
                    CommitType::Refactor => format!("refactor {}", symbols),
                    CommitType::Perf => format!("optimize {}", symbols),
                    CommitType::Test => format!("add tests for {}", symbols),
                    _ => format!("update {}", symbols),
                };
            }
        }
    }

    // 2. Pure renames or deletions
    if analysis.files.len() == 1 {
        let f = &analysis.files[0];
        match &f.change_kind {
            FileChangeKind::Renamed { from } => {
                return format!("rename `{}` to `{}`", from, f.path);
            }
            FileChangeKind::Added => {
                return format!("add `{}`", f.path);
            }
            FileChangeKind::Deleted => {
                return format!("remove `{}`", f.path);
            }
            _ => {}
        }
    }

    // 3. Fallback based on commit type and scope
    let component_desc = analysis.inferred_scope.as_deref().unwrap_or("codebase");

    match commit_type {
        CommitType::Feat => format!("add support for {}", component_desc),
        CommitType::Fix => format!("fix error in {}", component_desc),
        CommitType::Docs => format!("update {} documentation", component_desc),
        CommitType::Style => format!("format and clean up {}", component_desc),
        CommitType::Refactor => format!("refactor {} implementation", component_desc),
        CommitType::Perf => format!("improve performance in {}", component_desc),
        CommitType::Test => format!("add test coverage for {}", component_desc),
        CommitType::Build => "update dependencies and build configuration".to_string(),
        CommitType::Ci => "update CI/CD workflows and automation".to_string(),
        CommitType::Chore => format!("maintenance updates for {}", component_desc),
        CommitType::Revert => format!("revert changes in {}", component_desc),
        CommitType::Custom(custom) => format!("apply {} changes to {}", custom, component_desc),
    }
}

/// Synthesize an alternative description for multi-candidate generation.
fn synthesize_alternate_description(analysis: &DiffAnalysis, commit_type: &CommitType) -> String {
    let file_count = analysis.files.len();
    if file_count == 1 {
        let file = &analysis.files[0];
        return format!(
            "update `{}` (+{} -{})",
            file.path, file.additions, file.deletions
        );
    }

    format!(
        "{} changes across {} files",
        commit_type.as_str(),
        file_count
    )
}

/// Synthesize a multi-line body summarizing the diff and changes.
fn synthesize_commit_body(analysis: &DiffAnalysis, include_stats: bool) -> Option<String> {
    if analysis.files.is_empty() {
        return None;
    }

    let mut body_lines = Vec::new();

    // Group files by change kind
    let mut added_files = Vec::new();
    let mut modified_files = Vec::new();
    let mut deleted_files = Vec::new();
    let mut renamed_files = Vec::new();

    for f in &analysis.files {
        match &f.change_kind {
            FileChangeKind::Added => added_files.push(f.path.as_str()),
            FileChangeKind::Modified => modified_files.push(f.path.as_str()),
            FileChangeKind::Deleted => deleted_files.push(f.path.as_str()),
            FileChangeKind::Renamed { from } => {
                renamed_files.push((from.as_str(), f.path.as_str()))
            }
            _ => modified_files.push(f.path.as_str()),
        }
    }

    if !added_files.is_empty() {
        body_lines.push(format!("- Added: {}", added_files.join(", ")));
    }
    if !modified_files.is_empty() {
        body_lines.push(format!("- Modified: {}", modified_files.join(", ")));
    }
    if !deleted_files.is_empty() {
        body_lines.push(format!("- Deleted: {}", deleted_files.join(", ")));
    }
    for (from, to) in renamed_files {
        body_lines.push(format!("- Renamed: `{}` -> `{}`", from, to));
    }

    if include_stats {
        body_lines.push(format!(
            "\nDiff statistics: +{} additions, -{} deletions across {} files.",
            analysis.total_additions,
            analysis.total_deletions,
            analysis.files.len()
        ));
    }

    if body_lines.is_empty() {
        None
    } else {
        Some(body_lines.join("\n"))
    }
}

/// Convert the first character of a string to lowercase.
fn to_lower_first_char(s: &str) -> String {
    let mut chars = s.chars();
    match chars.next() {
        None => String::new(),
        Some(first) => first.to_lowercase().collect::<String>() + chars.as_str(),
    }
}

// ============================================================================
// Unit Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_conventional_commit_format_simple() {
        let commit = ConventionalCommit {
            commit_type: CommitType::Feat,
            scope: Some("agent".to_string()),
            is_breaking: false,
            description: "add conventional commit generator".to_string(),
            body: None,
            footers: Vec::new(),
        };

        assert_eq!(
            commit.format_summary(),
            "feat(agent): add conventional commit generator"
        );
        assert_eq!(
            commit.format(),
            "feat(agent): add conventional commit generator"
        );
    }

    #[test]
    fn test_conventional_commit_format_breaking() {
        let commit = ConventionalCommit {
            commit_type: CommitType::Feat,
            scope: Some("api".to_string()),
            is_breaking: true,
            description: "drop legacy v1 endpoints".to_string(),
            body: Some("Remove deprecated endpoints that were replaced in v2.".to_string()),
            footers: vec![CommitFooter::breaking_change("v1 endpoints removed")],
        };

        assert_eq!(
            commit.format_summary(),
            "feat(api)!: drop legacy v1 endpoints"
        );
        let formatted = commit.format();
        assert!(formatted.starts_with("feat(api)!: drop legacy v1 endpoints\n\n"));
        assert!(formatted.contains("Remove deprecated endpoints"));
        assert!(formatted.contains("BREAKING CHANGE: v1 endpoints removed"));
    }

    #[test]
    fn test_conventional_commit_parse_roundtrip() {
        let raw = "fix(parser)!: handle malformed json tokens\n\nCorrect edge case when streaming partial inputs.\n\nCloses #42\nBREAKING CHANGE: parser returns Result instead of Option";

        let parsed = ConventionalCommit::parse(raw).expect("Failed to parse valid commit");
        assert_eq!(parsed.commit_type, CommitType::Fix);
        assert_eq!(parsed.scope, Some("parser".to_string()));
        assert!(parsed.is_breaking);
        assert_eq!(parsed.description, "handle malformed json tokens");
        assert!(parsed
            .body
            .as_ref()
            .map(|b| b.contains("Correct edge case"))
            .unwrap_or(false));
        assert_eq!(parsed.footers.len(), 2);
    }

    #[test]
    fn test_conventional_commit_parse_errors() {
        assert!(matches!(
            ConventionalCommit::parse(""),
            Err(CommitParseError::EmptyInput)
        ));
        assert!(matches!(
            ConventionalCommit::parse("   "),
            Err(CommitParseError::EmptyInput)
        ));
        assert!(matches!(
            ConventionalCommit::parse("feat missing colon"),
            Err(CommitParseError::InvalidHeaderFormat(_))
        ));
        assert!(matches!(
            ConventionalCommit::parse("feat(unclosed: missing paren"),
            Err(CommitParseError::InvalidHeaderFormat(_))
        ));
        assert!(matches!(
            ConventionalCommit::parse("feat(scope):"),
            Err(CommitParseError::InvalidHeaderFormat(_))
        ));
    }

    #[test]
    fn test_conventional_commit_validate() {
        let valid = ConventionalCommit {
            commit_type: CommitType::Feat,
            scope: Some("auth".to_string()),
            is_breaking: false,
            description: "add oauth token refresh".to_string(),
            body: None,
            footers: Vec::new(),
        };
        assert!(valid.validate().is_ok());

        let with_period = ConventionalCommit {
            commit_type: CommitType::Feat,
            scope: Some("auth".to_string()),
            is_breaking: false,
            description: "add oauth token refresh.".to_string(),
            body: None,
            footers: Vec::new(),
        };
        let errs = with_period.validate().unwrap_err();
        assert!(errs.iter().any(|e| e.contains("period")));

        let space_scope = ConventionalCommit {
            commit_type: CommitType::Fix,
            scope: Some("auth module".to_string()),
            is_breaking: false,
            description: "fix token leak".to_string(),
            body: None,
            footers: Vec::new(),
        };
        let errs = space_scope.validate().unwrap_err();
        assert!(errs.iter().any(|e| e.contains("spaces")));
    }

    #[test]
    fn test_builder_interface() {
        let commit = ConventionalCommit::builder()
            .feat()
            .scope("ui")
            .description("add diff side-by-side view")
            .body("Renders unified and split views.")
            .closes_issue("#101")
            .co_authored_by("Jane Doe <jane@example.com>")
            .build_commit()
            .expect("Failed to build commit");

        assert_eq!(commit.commit_type, CommitType::Feat);
        assert_eq!(commit.scope.as_deref(), Some("ui"));
        assert_eq!(commit.description, "add diff side-by-side view");
        assert_eq!(commit.footers.len(), 2);
    }

    #[test]
    fn test_parse_git_diff_rust_feature() {
        let diff = r#"diff --git a/src/agent/commit_gen.rs b/src/agent/commit_gen.rs
new file mode 100644
index 0000000..1234567
--- /dev/null
+++ b/src/agent/commit_gen.rs
@@ -0,0 +1,25 @@
+pub struct CommitGenerator {
+    config: CommitGeneratorConfig,
+}
+
+pub fn generate_commit() -> String {
+    "feat: initial commit".to_string()
+}
"#;

        let generator = CommitGenerator::default();
        let commit = generator.generate(diff);

        assert_eq!(commit.commit_type, CommitType::Feat);
        assert_eq!(commit.scope.as_deref(), Some("commit_gen"));
        assert!(
            commit.description.contains("CommitGenerator")
                || commit.description.contains("generate_commit")
        );
    }

    #[test]
    fn test_parse_git_diff_fix() {
        let diff = r#"diff --git a/src/agent/search.rs b/src/agent/search.rs
index 1234567..89abcdef 100644
--- a/src/agent/search.rs
+++ b/src/agent/search.rs
@@ -10,4 +10,6 @@ pub fn search_sessions() {
     let q = raw_query.trim();
+    // Fix crash when query is empty string or malformed
+    if q.is_empty() { return vec![]; }
 }
"#;

        let generator = CommitGenerator::default();
        let commit = generator.generate(diff);

        assert_eq!(commit.commit_type, CommitType::Fix);
        assert_eq!(commit.scope.as_deref(), Some("search"));
    }

    #[test]
    fn test_parse_git_diff_refactor() {
        let diff = r#"diff --git a/src/utils/format.rs b/src/utils/format.rs
index 1234567..89abcdef 100644
--- a/src/utils/format.rs
+++ b/src/utils/format.rs
@@ -1,10 +1,5 @@
-fn legacy_format_helper(a: &str, b: &str) -> String {
-    let mut s = String::new();
-    s.push_str(a);
-    s.push_str(b);
-    s
-}
+// Refactor and clean up formatting helper
+fn format_helper(a: &str, b: &str) -> String {
+    format!("{}{}", a, b)
+}
"#;

        let generator = CommitGenerator::default();
        let commit = generator.generate(diff);

        assert_eq!(commit.commit_type, CommitType::Refactor);
        assert_eq!(commit.scope.as_deref(), Some("format"));
    }

    #[test]
    fn test_parse_git_diff_perf() {
        let diff = r#"diff --git a/src/cache/lru.rs b/src/cache/lru.rs
index 1234567..89abcdef 100644
--- a/src/cache/lru.rs
+++ b/src/cache/lru.rs
@@ -15,3 +15,6 @@ impl LruCache {
     pub fn get(&mut self, key: &str) -> Option<&Value> {
+        // Optimize cache lookup speed with index table
+        self.hot_entries.get(key)
     }
 }
"#;

        let generator = CommitGenerator::default();
        let commit = generator.generate(diff);

        assert_eq!(commit.commit_type, CommitType::Perf);
        assert_eq!(commit.scope.as_deref(), Some("lru"));
    }

    #[test]
    fn test_parse_git_diff_docs_change() {
        let diff = r#"diff --git a/docs/README.md b/docs/README.md
index 1111111..2222222 100644
--- a/docs/README.md
+++ b/docs/README.md
@@ -1,3 +1,5 @@
 # Documentation
+
+Updated guide for running subagents.
"#;

        let generator = CommitGenerator::default();
        let commit = generator.generate(diff);

        assert_eq!(commit.commit_type, CommitType::Docs);
        assert_eq!(commit.scope.as_deref(), Some("docs"));
    }

    #[test]
    fn test_parse_git_diff_ci_workflow() {
        let diff = r#"diff --git a/.github/workflows/ci.yml b/.github/workflows/ci.yml
index 1111111..2222222 100644
--- a/.github/workflows/ci.yml
+++ b/.github/workflows/ci.yml
@@ -10,3 +10,4 @@ jobs:
     steps:
       - uses: actions/checkout@v4
+      - run: cargo clippy
"#;

        let generator = CommitGenerator::default();
        let commit = generator.generate(diff);

        assert_eq!(commit.commit_type, CommitType::Ci);
        assert_eq!(commit.scope.as_deref(), Some("ci"));
    }

    #[test]
    fn test_parse_git_diff_build_deps() {
        let diff = r#"diff --git a/Cargo.toml b/Cargo.toml
index 1111111..2222222 100644
--- a/Cargo.toml
+++ b/Cargo.toml
@@ -20,3 +20,4 @@ tokio = { version = "1", features = ["full"] }
+serde_json = "1.0"
"#;

        let generator = CommitGenerator::default();
        let commit = generator.generate(diff);

        assert_eq!(commit.commit_type, CommitType::Build);
        assert_eq!(commit.scope.as_deref(), Some("deps"));
    }

    #[test]
    fn test_parse_git_diff_tests() {
        let diff = r#"diff --git a/tests/integration_test.rs b/tests/integration_test.rs
new file mode 100644
index 0000000..1234567
--- /dev/null
+++ b/tests/integration_test.rs
@@ -0,0 +1,10 @@
+#[test]
+fn test_agent_integration() {
+    assert!(true);
+}
"#;

        let generator = CommitGenerator::default();
        let commit = generator.generate(diff);

        assert_eq!(commit.commit_type, CommitType::Test);
        assert_eq!(commit.scope.as_deref(), Some("test"));
    }

    #[test]
    fn test_parse_git_diff_chore() {
        let diff = r#"diff --git a/.gitignore b/.gitignore
index 1111111..2222222 100644
--- a/.gitignore
+++ b/.gitignore
@@ -5,3 +5,4 @@ target/
+.DS_Store
"#;

        let generator = CommitGenerator::default();
        let commit = generator.generate(diff);

        assert_eq!(commit.commit_type, CommitType::Chore);
    }

    #[test]
    fn test_parse_git_diff_breaking_change() {
        let diff = r#"diff --git a/src/api/client.rs b/src/api/client.rs
index 1234567..89abcdef 100644
--- a/src/api/client.rs
+++ b/src/api/client.rs
@@ -1,5 +1,1 @@
-pub fn connect_v1(endpoint: &str) -> Client {
-    Client::new(endpoint)
-}
"#;

        let generator = CommitGenerator::default();
        let commit = generator.generate(diff);

        assert!(commit.is_breaking);
        assert!(commit.format_summary().contains('!'));
        assert!(commit.footers.iter().any(|f| f.is_breaking()));
    }

    #[test]
    fn test_parse_git_diff_renamed_file() {
        let diff = r#"diff --git a/src/old_name.rs b/src/new_name.rs
similarity index 100%
rename from src/old_name.rs
rename to src/new_name.rs
"#;

        let generator = CommitGenerator::default();
        let commit = generator.generate(diff);

        assert_eq!(commit.commit_type, CommitType::Refactor);
        assert!(commit.description.contains("rename"));
    }

    #[test]
    fn test_parse_git_diff_deleted_file() {
        let diff = r#"diff --git a/src/deprecated.rs b/src/deprecated.rs
deleted file mode 100644
index 1234567..0000000
--- a/src/deprecated.rs
+++ /dev/null
@@ -1,5 +0,0 @@
-// Obsolete code
"#;

        let generator = CommitGenerator::default();
        let commit = generator.generate(diff);

        assert!(
            commit.description.contains("remove `src/deprecated.rs`")
                || commit.commit_type == CommitType::Refactor
                || commit.commit_type == CommitType::Feat
        );
    }

    #[test]
    fn test_parse_git_diff_binary_file() {
        let diff = r#"diff --git a/assets/logo.png b/assets/logo.png
new file mode 100644
index 0000000..1234567
Binary files /dev/null and b/assets/logo.png differ
"#;

        let generator = CommitGenerator::default();
        let analysis = generator.analyze_diff(diff);
        assert_eq!(analysis.files.len(), 1);
        assert_eq!(analysis.files[0].path, "assets/logo.png");
    }

    #[test]
    fn test_parse_git_diff_empty_and_whitespace() {
        let generator = CommitGenerator::default();
        let commit = generator.generate("");
        assert_eq!(commit.commit_type, CommitType::Chore);
        assert!(commit.description.contains("empty"));

        let commit_spaces = generator.generate("   \n\n  ");
        assert_eq!(commit_spaces.commit_type, CommitType::Chore);
    }

    #[test]
    fn test_scope_inference_various_paths() {
        assert_eq!(
            infer_scope_from_path("src/agent/commit_gen.rs"),
            Some("commit_gen".to_string())
        );
        assert_eq!(
            infer_scope_from_path("src/tools/git.rs"),
            Some("git".to_string())
        );
        assert_eq!(
            infer_scope_from_path("src/ui/slash.rs"),
            Some("slash".to_string())
        );
        assert_eq!(
            infer_scope_from_path("Cargo.toml"),
            Some("deps".to_string())
        );
        assert_eq!(
            infer_scope_from_path(".github/workflows/ci.yml"),
            Some("ci".to_string())
        );
        assert_eq!(
            infer_scope_from_path("benches/bench.rs"),
            Some("bench".to_string())
        );
        assert_eq!(
            infer_scope_from_path("crates/fusion-cli/src/main.rs"),
            Some("fusion-cli".to_string())
        );
        assert_eq!(
            infer_scope_from_path("packages/sdk/src/index.ts"),
            Some("sdk".to_string())
        );
    }

    #[test]
    fn test_generate_candidates() {
        let diff = r#"diff --git a/src/agent/search.rs b/src/agent/search.rs
index 1234567..89abcdef 100644
--- a/src/agent/search.rs
+++ b/src/agent/search.rs
@@ -10,4 +10,6 @@ pub fn search_sessions() {
     let q = raw_query.trim();
+    // Fixed search indexing bug
+    let query = clean_query(q);
 }
"#;

        let generator = CommitGenerator::default();
        let candidates = generator.generate_candidates(diff, 3);
        assert!(candidates.len() >= 2);
    }

    #[test]
    fn test_format_prompt_for_llm() {
        let diff = r#"diff --git a/src/agent/search.rs b/src/agent/search.rs
index 1234567..89abcdef 100644
--- a/src/agent/search.rs
+++ b/src/agent/search.rs
@@ -10,4 +10,6 @@ pub fn search_sessions() {
+    pub fn quick_search() {}
 }
"#;

        let generator = CommitGenerator::default();
        let analysis = generator.analyze_diff(diff);
        let prompt = generator.format_prompt_for_llm(diff, &analysis);

        assert!(prompt.contains("Conventional Commits v1.0.0"));
        assert!(prompt.contains("quick_search"));
        assert!(prompt.contains("```diff"));
    }

    #[test]
    fn test_emoji_formatting() {
        let commit = ConventionalCommit {
            commit_type: CommitType::Feat,
            scope: Some("agent".to_string()),
            is_breaking: false,
            description: "add commit generator".to_string(),
            body: None,
            footers: Vec::new(),
        };
        assert_eq!(
            commit.format_with_emoji(),
            "✨ feat(agent): add commit generator"
        );
    }

    #[test]
    fn test_custom_config_options() {
        let config = CommitGeneratorConfig {
            max_subject_length: 50,
            include_body: false,
            include_diff_stats: false,
            include_scope: true,
            enforce_lower_case_subject: true,
            default_scope: Some("core".to_string()),
            allowed_scopes: vec!["core".to_string()],
            issue_references: vec!["#123".to_string()],
            co_authors: vec!["Alice <alice@example.com>".to_string()],
            include_emojis: false,
        };

        let generator = CommitGenerator::new(config);
        let diff = r#"diff --git a/src/main.rs b/src/main.rs
index 1234567..89abcdef 100644
--- a/src/main.rs
+++ b/src/main.rs
@@ -1,3 +1,4 @@
+pub fn run_app() {}
"#;

        let commit = generator.generate(diff);
        assert_eq!(commit.scope.as_deref(), Some("core"));
        assert!(commit.body.is_none());
        assert_eq!(commit.footers.len(), 2);
        assert_eq!(commit.footers[0].format(), "Closes: #123");
        assert_eq!(
            commit.footers[1].format(),
            "Co-authored-by: Alice <alice@example.com>"
        );
    }
}
