//! Interactive TUI Diff Viewer and Hunk Reviewer
//!
//! Provides a visual diff viewer and interactive hunk staging/rejection tool
//! matching git-add -p, lazygit, and fx.sh UX:
//! - **Unified & Side-by-Side (Split) View Modes**: Toggle seamlessly via `Tab` / `t`.
//! - **Syntax Highlighting**: Pure-Rust multi-language syntax highlighter (Rust, TS/JS, Python, Go, C/C++, JSON, YAML, TOML, Markdown, HTML/CSS, Shell, SQL, etc.).
//! - **Hunk Controls**: Stage (`s`/`Space`), Reject (`r`/`x`), Reset (`u`), Stage File (`S`), Reject File (`R`), Stage All (`A`), Reject All (`X`).
//! - **Navigation**: Jump to next/prev hunk (`n`/`p` or `]`/`[`), next/prev file (`>`/`<`), scroll line-by-line (`j`/`k`) or page (`Ctrl+D`/`Ctrl+U`).
//! - **Collapsible Sidebar**: File tree / changed files list with staging progress indicators (`[2/3 staged]`).
//! - **Help Modal**: Comprehensive keybinding overlay (`?` / `h`).
//! - **Patch Reconstruction**: Generates accurate staged patches or reconstructed file contents from staged decisions.

use crossterm::{
    cursor,
    event::{self, Event, KeyCode, KeyEventKind, KeyModifiers},
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{
    backend::CrosstermBackend,
    buffer::Buffer,
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{
        Block, BorderType, Borders, Clear, List, ListItem, Paragraph, Widget,
    },
    Terminal,
};
use serde::{Deserialize, Serialize};
use similar::{ChangeTag, TextDiff};
use std::io::{stdout, Stdout};
use std::path::Path;

use crate::tools::patch::{FilePatch, HunkLine as PatchHunkLine};
use crate::ui::prompt::RawModeGuard;
use crate::ui::theme::Theme;

// ============================================================================
// Data Types & Enums
// ============================================================================

/// Staging status of a single hunk or file.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
pub enum HunkStatus {
    /// Decision pending (default).
    #[default]
    Pending,
    /// Hunk will be staged / applied.
    Staged,
    /// Hunk will be rejected / discarded.
    Rejected,
    /// Mixed staging status (for files with some hunks staged and some rejected/pending).
    PartiallyStaged,
}

impl HunkStatus {
    pub fn is_staged(&self) -> bool {
        matches!(self, Self::Staged)
    }

    pub fn is_rejected(&self) -> bool {
        matches!(self, Self::Rejected)
    }

    pub fn is_pending(&self) -> bool {
        matches!(self, Self::Pending)
    }

    /// Short status badge string.
    pub fn badge(&self) -> &'static str {
        match self {
            Self::Pending => "[• PENDING]",
            Self::Staged => "[✓ STAGED]",
            Self::Rejected => "[✗ REJECTED]",
            Self::PartiallyStaged => "[~ PARTIAL]",
        }
    }

    /// Short glyph.
    pub fn icon(&self) -> &'static str {
        match self {
            Self::Pending => "•",
            Self::Staged => "✓",
            Self::Rejected => "✗",
            Self::PartiallyStaged => "~",
        }
    }

    /// Style for this status based on the theme.
    pub fn style(&self, theme: &Theme) -> Style {
        match self {
            Self::Pending => Style::default().fg(theme.warning).add_modifier(Modifier::BOLD),
            Self::Staged => Style::default().fg(theme.success).add_modifier(Modifier::BOLD),
            Self::Rejected => Style::default().fg(theme.error).add_modifier(Modifier::BOLD),
            Self::PartiallyStaged => Style::default().fg(theme.accent).add_modifier(Modifier::BOLD),
        }
    }
}

/// Diff display presentation mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
pub enum DiffViewMode {
    /// Unified diff view (linear with line numbers).
    #[default]
    Unified,
    /// Side-by-side (split) diff view (Original | Modified).
    SideBySide,
}

impl DiffViewMode {
    pub fn toggle(&self) -> Self {
        match self {
            Self::Unified => Self::SideBySide,
            Self::SideBySide => Self::Unified,
        }
    }

    pub fn name(&self) -> &'static str {
        match self {
            Self::Unified => "Unified",
            Self::SideBySide => "Side-by-Side",
        }
    }
}

/// Type of line within a diff hunk.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum DiffLineType {
    /// Unchanged context line
    Context,
    /// Added line (+)
    Addition,
    /// Removed line (-)
    Deletion,
    /// Hunk header (@@ -a,b +c,d @@)
    HunkHeader,
    /// File header line (--- / +++ / diff)
    FileHeader,
    /// Padding/empty line used in Side-by-Side alignment
    Empty,
}

/// A single rendered line within a diff view.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiffLine {
    pub line_type: DiffLineType,
    pub old_lineno: Option<usize>,
    pub new_lineno: Option<usize>,
    pub content: String,
    pub staged: bool,
}

impl DiffLine {
    pub fn context(old_no: usize, new_no: usize, content: impl Into<String>) -> Self {
        Self {
            line_type: DiffLineType::Context,
            old_lineno: Some(old_no),
            new_lineno: Some(new_no),
            content: content.into(),
            staged: true,
        }
    }

    pub fn addition(new_no: usize, content: impl Into<String>) -> Self {
        Self {
            line_type: DiffLineType::Addition,
            old_lineno: None,
            new_lineno: Some(new_no),
            content: content.into(),
            staged: true,
        }
    }

    pub fn deletion(old_no: usize, content: impl Into<String>) -> Self {
        Self {
            line_type: DiffLineType::Deletion,
            old_lineno: Some(old_no),
            new_lineno: None,
            content: content.into(),
            staged: true,
        }
    }

    pub fn hunk_header(header: impl Into<String>) -> Self {
        Self {
            line_type: DiffLineType::HunkHeader,
            old_lineno: None,
            new_lineno: None,
            content: header.into(),
            staged: true,
        }
    }

    pub fn empty() -> Self {
        Self {
            line_type: DiffLineType::Empty,
            old_lineno: None,
            new_lineno: None,
            content: String::new(),
            staged: true,
        }
    }
}

/// A parsed diff hunk containing lines and staging status.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiffHunk {
    pub index: usize,
    pub old_start: usize,
    pub old_lines: usize,
    pub new_start: usize,
    pub new_lines: usize,
    pub header: String,
    pub lines: Vec<DiffLine>,
    pub status: HunkStatus,
    pub additions_count: usize,
    pub deletions_count: usize,
}

impl DiffHunk {
    pub fn new(
        index: usize,
        old_start: usize,
        old_lines: usize,
        new_start: usize,
        new_lines: usize,
        header: impl Into<String>,
        lines: Vec<DiffLine>,
    ) -> Self {
        let mut adds = 0;
        let mut dels = 0;
        for l in &lines {
            match l.line_type {
                DiffLineType::Addition => adds += 1,
                DiffLineType::Deletion => dels += 1,
                _ => {}
            }
        }

        Self {
            index,
            old_start,
            old_lines,
            new_start,
            new_lines,
            header: header.into(),
            lines,
            status: HunkStatus::Pending,
            additions_count: adds,
            deletions_count: dels,
        }
    }

    /// Mark hunk as staged.
    pub fn stage(&mut self) {
        self.status = HunkStatus::Staged;
    }

    /// Mark hunk as rejected.
    pub fn reject(&mut self) {
        self.status = HunkStatus::Rejected;
    }

    /// Reset hunk to pending.
    pub fn reset(&mut self) {
        self.status = HunkStatus::Pending;
    }

    /// Toggle hunk status (Pending/Rejected -> Staged -> Rejected).
    pub fn toggle(&mut self) {
        self.status = match self.status {
            HunkStatus::Pending => HunkStatus::Staged,
            HunkStatus::Staged => HunkStatus::Rejected,
            HunkStatus::Rejected => HunkStatus::Pending,
            HunkStatus::PartiallyStaged => HunkStatus::Staged,
        };
    }

    /// Expected lines from the original file for this hunk.
    pub fn expected_old_lines(&self) -> Vec<&str> {
        self.lines
            .iter()
            .filter_map(|l| match l.line_type {
                DiffLineType::Context | DiffLineType::Deletion => Some(l.content.as_str()),
                _ => None,
            })
            .collect()
    }

    /// Expected lines for the modified file for this hunk.
    pub fn expected_new_lines(&self) -> Vec<&str> {
        self.lines
            .iter()
            .filter_map(|l| match l.line_type {
                DiffLineType::Context | DiffLineType::Addition => Some(l.content.as_str()),
                _ => None,
            })
            .collect()
    }

    /// Formats this hunk as a unified diff hunk string.
    pub fn format_unified(&self) -> String {
        let mut out = format!(
            "@@ -{},{} +{},{} @@ {}\n",
            self.old_start,
            self.old_lines,
            self.new_start,
            self.new_lines,
            self.header
        );
        for l in &self.lines {
            match l.line_type {
                DiffLineType::Context => {
                    out.push(' ');
                    out.push_str(&l.content);
                    out.push('\n');
                }
                DiffLineType::Addition => {
                    out.push('+');
                    out.push_str(&l.content);
                    out.push('\n');
                }
                DiffLineType::Deletion => {
                    out.push('-');
                    out.push_str(&l.content);
                    out.push('\n');
                }
                _ => {}
            }
        }
        out
    }
}

/// A file changed in the diff set.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiffFile {
    pub path: String,
    pub old_path: Option<String>,
    pub new_path: Option<String>,
    pub is_new: bool,
    pub is_deleted: bool,
    pub is_renamed: bool,
    pub is_binary: bool,
    pub hunks: Vec<DiffHunk>,
    pub active_hunk_idx: usize,
    pub original_content: Option<String>,
    pub modified_content: Option<String>,
}

impl DiffFile {
    pub fn new(path: impl Into<String>) -> Self {
        let p = path.into();
        Self {
            path: p.clone(),
            old_path: Some(p.clone()),
            new_path: Some(p),
            is_new: false,
            is_deleted: false,
            is_renamed: false,
            is_binary: false,
            hunks: Vec::new(),
            active_hunk_idx: 0,
            original_content: None,
            modified_content: None,
        }
    }

    /// Total additions across all hunks.
    pub fn additions(&self) -> usize {
        self.hunks.iter().map(|h| h.additions_count).sum()
    }

    /// Total deletions across all hunks.
    pub fn deletions(&self) -> usize {
        self.hunks.iter().map(|h| h.deletions_count).sum()
    }

    /// Number of staged hunks.
    pub fn staged_hunks_count(&self) -> usize {
        self.hunks.iter().filter(|h| h.status.is_staged()).count()
    }

    /// Number of rejected hunks.
    pub fn rejected_hunks_count(&self) -> usize {
        self.hunks.iter().filter(|h| h.status.is_rejected()).count()
    }

    /// Number of pending hunks.
    pub fn pending_hunks_count(&self) -> usize {
        self.hunks.iter().filter(|h| h.status.is_pending()).count()
    }

    /// Overall status of this file.
    pub fn status(&self) -> HunkStatus {
        if self.hunks.is_empty() {
            return HunkStatus::Pending;
        }
        let staged = self.staged_hunks_count();
        let rejected = self.rejected_hunks_count();
        let total = self.hunks.len();

        if staged == total {
            HunkStatus::Staged
        } else if rejected == total {
            HunkStatus::Rejected
        } else if staged > 0 || rejected > 0 {
            HunkStatus::PartiallyStaged
        } else {
            HunkStatus::Pending
        }
    }

    /// Stage all hunks in this file.
    pub fn stage_all(&mut self) {
        for h in &mut self.hunks {
            h.stage();
        }
    }

    /// Reject all hunks in this file.
    pub fn reject_all(&mut self) {
        for h in &mut self.hunks {
            h.reject();
        }
    }

    /// Reset all hunks in this file to pending.
    pub fn reset_all(&mut self) {
        for h in &mut self.hunks {
            h.reset();
        }
    }

    /// Toggle all hunks in this file.
    pub fn toggle_all(&mut self) {
        let next_status = match self.status() {
            HunkStatus::Staged => HunkStatus::Rejected,
            HunkStatus::Rejected => HunkStatus::Pending,
            _ => HunkStatus::Staged,
        };
        for h in &mut self.hunks {
            h.status = next_status;
        }
    }

    /// Active hunk reference if valid.
    pub fn active_hunk(&self) -> Option<&DiffHunk> {
        self.hunks.get(self.active_hunk_idx)
    }

    /// Active hunk mutable reference if valid.
    pub fn active_hunk_mut(&mut self) -> Option<&mut DiffHunk> {
        self.hunks.get_mut(self.active_hunk_idx)
    }

    /// Move to next hunk.
    pub fn next_hunk(&mut self) -> bool {
        if !self.hunks.is_empty() && self.active_hunk_idx + 1 < self.hunks.len() {
            self.active_hunk_idx += 1;
            true
        } else {
            false
        }
    }

    /// Move to previous hunk.
    pub fn prev_hunk(&mut self) -> bool {
        if self.active_hunk_idx > 0 {
            self.active_hunk_idx -= 1;
            true
        } else {
            false
        }
    }

    /// Reconstruct the resulting file content based on staged hunks.
    ///
    /// If original content is present, applies staged hunks on top of it.
    /// If only original content and hunks are known, reconstructs line-by-line.
    pub fn reconstruct_staged_content(&self) -> Option<String> {
        if let Some(orig) = &self.original_content {
            let orig_lines: Vec<&str> = orig.lines().collect();
            let mut result_lines = Vec::new();
            let mut orig_idx = 0;

            for hunk in &self.hunks {
                // Determine 1-based old line start (0-indexed in array)
                let hunk_start = if hunk.old_start > 0 { hunk.old_start - 1 } else { 0 };

                // Copy unmodified lines before this hunk
                while orig_idx < hunk_start && orig_idx < orig_lines.len() {
                    result_lines.push(orig_lines[orig_idx].to_string());
                    orig_idx += 1;
                }

                if hunk.status.is_staged() {
                    // Apply staged changes
                    for l in &hunk.lines {
                        match l.line_type {
                            DiffLineType::Context => {
                                if orig_idx < orig_lines.len() {
                                    result_lines.push(orig_lines[orig_idx].to_string());
                                    orig_idx += 1;
                                } else {
                                    result_lines.push(l.content.clone());
                                }
                            }
                            DiffLineType::Addition => {
                                result_lines.push(l.content.clone());
                            }
                            DiffLineType::Deletion => {
                                if orig_idx < orig_lines.len() {
                                    orig_idx += 1;
                                }
                            }
                            _ => {}
                        }
                    }
                } else {
                    // Rejected or Pending: keep original unchanged lines
                    let hunk_end = hunk_start + hunk.old_lines;
                    while orig_idx < hunk_end && orig_idx < orig_lines.len() {
                        result_lines.push(orig_lines[orig_idx].to_string());
                        orig_idx += 1;
                    }
                }
            }

            // Copy any remaining lines after all hunks
            while orig_idx < orig_lines.len() {
                result_lines.push(orig_lines[orig_idx].to_string());
                orig_idx += 1;
            }

            let mut out = result_lines.join("\n");
            if orig.ends_with('\n') {
                out.push('\n');
            }
            Some(out)
        } else if let Some(modified) = &self.modified_content {
            if self.hunks.iter().all(|h| h.status.is_staged()) {
                Some(modified.clone())
            } else {
                None
            }
        } else {
            None
        }
    }

    /// Generate unified diff string containing only staged hunks.
    pub fn generate_staged_patch(&self) -> String {
        let staged_hunks: Vec<&DiffHunk> = self.hunks.iter().filter(|h| h.status.is_staged()).collect();
        if staged_hunks.is_empty() {
            return String::new();
        }

        let old_p = self.old_path.as_deref().unwrap_or(&self.path);
        let new_p = self.new_path.as_deref().unwrap_or(&self.path);

        let mut patch = format!("--- a/{old_p}\n+++ b/{new_p}\n");
        for hunk in staged_hunks {
            patch.push_str(&hunk.format_unified());
        }
        patch
    }
}

// ============================================================================
// Syntax Highlighting Engine (Pure Rust)
// ============================================================================

/// Programming languages supported for syntax highlighting.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SyntaxLanguage {
    Rust,
    JavaScript,
    TypeScript,
    Python,
    Go,
    C,
    Cpp,
    Json,
    Yaml,
    Toml,
    Markdown,
    Html,
    Css,
    Shell,
    Sql,
    Diff,
    PlainText,
}

impl SyntaxLanguage {
    /// Detect language from file path or extension.
    pub fn from_path(path: &str) -> Self {
        let p = Path::new(path);
        if let Some(ext) = p.extension().and_then(|e| e.to_str()) {
            match ext.to_ascii_lowercase().as_str() {
                "rs" => Self::Rust,
                "js" | "mjs" | "cjs" | "jsx" => Self::JavaScript,
                "ts" | "mts" | "cts" | "tsx" => Self::TypeScript,
                "py" | "pyi" | "pyw" => Self::Python,
                "go" => Self::Go,
                "c" | "h" => Self::C,
                "cpp" | "cc" | "cxx" | "hpp" | "hxx" => Self::Cpp,
                "json" | "jsonc" | "json5" => Self::Json,
                "yaml" | "yml" => Self::Yaml,
                "toml" => Self::Toml,
                "md" | "markdown" => Self::Markdown,
                "html" | "htm" | "xml" | "svg" => Self::Html,
                "css" | "scss" | "sass" | "less" => Self::Css,
                "sh" | "bash" | "zsh" | "fish" | "env" => Self::Shell,
                "sql" => Self::Sql,
                "diff" | "patch" => Self::Diff,
                _ => Self::PlainText,
            }
        } else {
            let filename = p.file_name().and_then(|n| n.to_str()).unwrap_or("");
            match filename.to_ascii_lowercase().as_str() {
                "dockerfile" | "containerfile" => Self::Shell,
                "makefile" | "justfile" => Self::Shell,
                "cargo.lock" => Self::Toml,
                _ => Self::PlainText,
            }
        }
    }
}

/// Token types recognized during syntax highlighting.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TokenKind {
    Keyword,
    Type,
    Function,
    StringLiteral,
    NumberLiteral,
    Comment,
    Punctuation,
    Operator,
    Attribute,
    Variable,
    Plain,
}

/// Syntax token with text slice and semantic kind.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SyntaxToken<'a> {
    pub kind: TokenKind,
    pub text: &'a str,
}

/// Tokenizes a single line of code according to the detected language.
pub fn tokenize_line<'a>(line: &'a str, lang: SyntaxLanguage) -> Vec<SyntaxToken<'a>> {
    if line.is_empty() {
        return Vec::new();
    }

    match lang {
        SyntaxLanguage::Rust => tokenize_rust(line),
        SyntaxLanguage::JavaScript | SyntaxLanguage::TypeScript => tokenize_js_ts(line),
        SyntaxLanguage::Python => tokenize_python(line),
        SyntaxLanguage::Go => tokenize_go(line),
        SyntaxLanguage::Json | SyntaxLanguage::Toml | SyntaxLanguage::Yaml => tokenize_data_format(line),
        SyntaxLanguage::Shell => tokenize_shell(line),
        SyntaxLanguage::Html | SyntaxLanguage::Css => tokenize_markup(line),
        _ => tokenize_generic(line),
    }
}

fn tokenize_rust<'a>(line: &'a str) -> Vec<SyntaxToken<'a>> {
    let mut tokens = Vec::new();
    let bytes = line.as_bytes();
    let len = bytes.len();
    let mut i = 0;

    while i < len {
        // Line comments
        if i + 1 < len && bytes[i] == b'/' && bytes[i + 1] == b'/' {
            tokens.push(SyntaxToken {
                kind: TokenKind::Comment,
                text: &line[i..],
            });
            break;
        }

        // Attributes / Macros: #[derive(...)] or #![...]
        if bytes[i] == b'#' {
            let start = i;
            i += 1;
            while i < len && !bytes[i].is_ascii_whitespace() && bytes[i] != b'(' && bytes[i] != b'[' {
                i += 1;
            }
            tokens.push(SyntaxToken {
                kind: TokenKind::Attribute,
                text: &line[start..i],
            });
            continue;
        }

        // Strings: "..." or r#"..."#
        if bytes[i] == b'"' || (i + 1 < len && bytes[i] == b'r' && (bytes[i + 1] == b'"' || bytes[i + 1] == b'#')) {
            let start = i;
            if bytes[i] == b'r' {
                i += 1;
                while i < len && bytes[i] == b'#' {
                    i += 1;
                }
                if i < len && bytes[i] == b'"' {
                    i += 1;
                }
            } else {
                i += 1;
            }
            let mut escaped = false;
            while i < len {
                if !escaped && bytes[i] == b'"' {
                    i += 1;
                    break;
                }
                escaped = !escaped && bytes[i] == b'\\';
                i += 1;
            }
            tokens.push(SyntaxToken {
                kind: TokenKind::StringLiteral,
                text: &line[start..i],
            });
            continue;
        }

        // Numbers: 0x123, 123.45, 42u64
        if bytes[i].is_ascii_digit() {
            let start = i;
            while i < len
                && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'.' || bytes[i] == b'_')
            {
                i += 1;
            }
            tokens.push(SyntaxToken {
                kind: TokenKind::NumberLiteral,
                text: &line[start..i],
            });
            continue;
        }

        // Identifiers & Keywords
        if bytes[i].is_ascii_alphabetic() || bytes[i] == b'_' {
            let start = i;
            while i < len && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_') {
                i += 1;
            }
            let word = &line[start..i];

            // Check if followed by `(` -> Function
            let is_fn_call = i < len && bytes[i] == b'(';

            let kind = match word {
                "fn" | "let" | "mut" | "pub" | "struct" | "enum" | "trait" | "impl" | "use"
                | "mod" | "async" | "await" | "match" | "if" | "else" | "return" | "for"
                | "in" | "while" | "loop" | "break" | "continue" | "const" | "static"
                | "type" | "where" | "as" | "ref" | "move" | "unsafe" | "dyn" => {
                    TokenKind::Keyword
                }
                "Self" | "String" | "str" | "bool" | "u8" | "u16" | "u32" | "u64" | "u128"
                | "usize" | "i8" | "i16" | "i32" | "i64" | "i128" | "isize" | "f32" | "f64"
                | "Option" | "Some" | "None" | "Result" | "Ok" | "Err" | "Vec" | "Box"
                | "Rc" | "Arc" => TokenKind::Type,
                "true" | "false" => TokenKind::Keyword,
                _ if is_fn_call => TokenKind::Function,
                _ => TokenKind::Plain,
            };

            tokens.push(SyntaxToken { kind, text: word });
            continue;
        }

        // Whitespace and symbols
        let start = i;
        while i < len
            && !bytes[i].is_ascii_alphanumeric()
            && bytes[i] != b'_'
            && bytes[i] != b'"'
            && bytes[i] != b'/'
            && bytes[i] != b'#'
        {
            i += 1;
        }
        if i > start {
            tokens.push(SyntaxToken {
                kind: TokenKind::Punctuation,
                text: &line[start..i],
            });
        }
    }

    tokens
}

fn tokenize_js_ts<'a>(line: &'a str) -> Vec<SyntaxToken<'a>> {
    let mut tokens = Vec::new();
    let bytes = line.as_bytes();
    let len = bytes.len();
    let mut i = 0;

    while i < len {
        // Line comments
        if i + 1 < len && bytes[i] == b'/' && bytes[i + 1] == b'/' {
            tokens.push(SyntaxToken {
                kind: TokenKind::Comment,
                text: &line[i..],
            });
            break;
        }

        // Strings: "..." or '...' or `...`
        if bytes[i] == b'"' || bytes[i] == b'\'' || bytes[i] == b'`' {
            let quote = bytes[i];
            let start = i;
            i += 1;
            let mut escaped = false;
            while i < len {
                if !escaped && bytes[i] == quote {
                    i += 1;
                    break;
                }
                escaped = !escaped && bytes[i] == b'\\';
                i += 1;
            }
            tokens.push(SyntaxToken {
                kind: TokenKind::StringLiteral,
                text: &line[start..i],
            });
            continue;
        }

        // Numbers
        if bytes[i].is_ascii_digit() {
            let start = i;
            while i < len && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'.') {
                i += 1;
            }
            tokens.push(SyntaxToken {
                kind: TokenKind::NumberLiteral,
                text: &line[start..i],
            });
            continue;
        }

        // Identifiers / Keywords
        if bytes[i].is_ascii_alphabetic() || bytes[i] == b'_' || bytes[i] == b'$' {
            let start = i;
            while i < len && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_' || bytes[i] == b'$') {
                i += 1;
            }
            let word = &line[start..i];
            let is_fn = i < len && bytes[i] == b'(';

            let kind = match word {
                "const" | "let" | "var" | "function" | "async" | "await" | "return" | "if"
                | "else" | "switch" | "case" | "default" | "for" | "while" | "do" | "break"
                | "continue" | "import" | "export" | "from" | "class" | "extends" | "new"
                | "this" | "super" | "interface" | "type" | "enum" | "implements" => {
                    TokenKind::Keyword
                }
                "true" | "false" | "null" | "undefined" | "NaN" => TokenKind::Keyword,
                "string" | "number" | "boolean" | "any" | "void" | "never" | "unknown"
                | "Promise" | "Array" | "Object" | "Map" | "Set" => TokenKind::Type,
                _ if is_fn => TokenKind::Function,
                _ => TokenKind::Plain,
            };

            tokens.push(SyntaxToken { kind, text: word });
            continue;
        }

        let start = i;
        while i < len
            && !bytes[i].is_ascii_alphanumeric()
            && bytes[i] != b'_'
            && bytes[i] != b'$'
            && bytes[i] != b'"'
            && bytes[i] != b'\''
            && bytes[i] != b'`'
            && bytes[i] != b'/'
        {
            i += 1;
        }
        if i > start {
            tokens.push(SyntaxToken {
                kind: TokenKind::Punctuation,
                text: &line[start..i],
            });
        }
    }

    tokens
}

fn tokenize_python<'a>(line: &'a str) -> Vec<SyntaxToken<'a>> {
    let mut tokens = Vec::new();
    let bytes = line.as_bytes();
    let len = bytes.len();
    let mut i = 0;

    while i < len {
        // Comment
        if bytes[i] == b'#' {
            tokens.push(SyntaxToken {
                kind: TokenKind::Comment,
                text: &line[i..],
            });
            break;
        }

        // Decorators: @decorator
        if bytes[i] == b'@' {
            let start = i;
            i += 1;
            while i < len && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_' || bytes[i] == b'.') {
                i += 1;
            }
            tokens.push(SyntaxToken {
                kind: TokenKind::Attribute,
                text: &line[start..i],
            });
            continue;
        }

        // Strings: "..." or '...' or f"..." or r"..."
        if bytes[i] == b'"' || bytes[i] == b'\'' || ((bytes[i] == b'f' || bytes[i] == b'r' || bytes[i] == b'b') && i + 1 < len && (bytes[i + 1] == b'"' || bytes[i + 1] == b'\'')) {
            let start = i;
            if bytes[i] == b'f' || bytes[i] == b'r' || bytes[i] == b'b' {
                i += 1;
            }
            let quote = bytes[i];
            i += 1;
            let mut escaped = false;
            while i < len {
                if !escaped && bytes[i] == quote {
                    i += 1;
                    break;
                }
                escaped = !escaped && bytes[i] == b'\\';
                i += 1;
            }
            tokens.push(SyntaxToken {
                kind: TokenKind::StringLiteral,
                text: &line[start..i],
            });
            continue;
        }

        // Numbers
        if bytes[i].is_ascii_digit() {
            let start = i;
            while i < len && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'.') {
                i += 1;
            }
            tokens.push(SyntaxToken {
                kind: TokenKind::NumberLiteral,
                text: &line[start..i],
            });
            continue;
        }

        // Identifiers / Keywords
        if bytes[i].is_ascii_alphabetic() || bytes[i] == b'_' {
            let start = i;
            while i < len && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_') {
                i += 1;
            }
            let word = &line[start..i];
            let is_fn = i < len && bytes[i] == b'(';

            let kind = match word {
                "def" | "class" | "import" | "from" | "as" | "return" | "yield" | "async"
                | "await" | "if" | "elif" | "else" | "for" | "while" | "in" | "not" | "and"
                | "or" | "is" | "with" | "try" | "except" | "finally" | "raise" | "pass"
                | "break" | "continue" | "lambda" => TokenKind::Keyword,
                "True" | "False" | "None" | "self" | "cls" => TokenKind::Keyword,
                "int" | "str" | "float" | "bool" | "list" | "dict" | "set" | "tuple"
                | "Optional" | "List" | "Dict" | "Any" => TokenKind::Type,
                _ if is_fn => TokenKind::Function,
                _ => TokenKind::Plain,
            };

            tokens.push(SyntaxToken { kind, text: word });
            continue;
        }

        let start = i;
        while i < len
            && !bytes[i].is_ascii_alphanumeric()
            && bytes[i] != b'_'
            && bytes[i] != b'#'
            && bytes[i] != b'@'
            && bytes[i] != b'"'
            && bytes[i] != b'\''
        {
            i += 1;
        }
        if i > start {
            tokens.push(SyntaxToken {
                kind: TokenKind::Punctuation,
                text: &line[start..i],
            });
        }
    }

    tokens
}

fn tokenize_go<'a>(line: &'a str) -> Vec<SyntaxToken<'a>> {
    let mut tokens = Vec::new();
    let bytes = line.as_bytes();
    let len = bytes.len();
    let mut i = 0;

    while i < len {
        if i + 1 < len && bytes[i] == b'/' && bytes[i + 1] == b'/' {
            tokens.push(SyntaxToken {
                kind: TokenKind::Comment,
                text: &line[i..],
            });
            break;
        }

        if bytes[i] == b'"' || bytes[i] == b'`' {
            let quote = bytes[i];
            let start = i;
            i += 1;
            let mut escaped = false;
            while i < len {
                if !escaped && bytes[i] == quote {
                    i += 1;
                    break;
                }
                escaped = !escaped && bytes[i] == b'\\';
                i += 1;
            }
            tokens.push(SyntaxToken {
                kind: TokenKind::StringLiteral,
                text: &line[start..i],
            });
            continue;
        }

        if bytes[i].is_ascii_digit() {
            let start = i;
            while i < len && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'.') {
                i += 1;
            }
            tokens.push(SyntaxToken {
                kind: TokenKind::NumberLiteral,
                text: &line[start..i],
            });
            continue;
        }

        if bytes[i].is_ascii_alphabetic() || bytes[i] == b'_' {
            let start = i;
            while i < len && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_') {
                i += 1;
            }
            let word = &line[start..i];
            let is_fn = i < len && bytes[i] == b'(';

            let kind = match word {
                "func" | "package" | "import" | "return" | "if" | "else" | "for" | "range"
                | "switch" | "case" | "default" | "go" | "chan" | "select" | "var" | "const"
                | "type" | "struct" | "interface" | "defer" => TokenKind::Keyword,
                "nil" | "true" | "false" | "iota" => TokenKind::Keyword,
                "string" | "int" | "int64" | "uint" | "uint64" | "bool" | "byte" | "error"
                | "any" => TokenKind::Type,
                _ if is_fn => TokenKind::Function,
                _ => TokenKind::Plain,
            };

            tokens.push(SyntaxToken { kind, text: word });
            continue;
        }

        let start = i;
        while i < len
            && !bytes[i].is_ascii_alphanumeric()
            && bytes[i] != b'_'
            && bytes[i] != b'"'
            && bytes[i] != b'`'
            && bytes[i] != b'/'
        {
            i += 1;
        }
        if i > start {
            tokens.push(SyntaxToken {
                kind: TokenKind::Punctuation,
                text: &line[start..i],
            });
        }
    }

    tokens
}

fn tokenize_data_format<'a>(line: &'a str) -> Vec<SyntaxToken<'a>> {
    let mut tokens = Vec::new();
    let bytes = line.as_bytes();
    let len = bytes.len();
    let mut i = 0;

    while i < len {
        // Comment (# or //)
        if bytes[i] == b'#' || (i + 1 < len && bytes[i] == b'/' && bytes[i + 1] == b'/') {
            tokens.push(SyntaxToken {
                kind: TokenKind::Comment,
                text: &line[i..],
            });
            break;
        }

        // Strings
        if bytes[i] == b'"' || bytes[i] == b'\'' {
            let quote = bytes[i];
            let start = i;
            i += 1;
            let mut escaped = false;
            while i < len {
                if !escaped && bytes[i] == quote {
                    i += 1;
                    break;
                }
                escaped = !escaped && bytes[i] == b'\\';
                i += 1;
            }
            tokens.push(SyntaxToken {
                kind: TokenKind::StringLiteral,
                text: &line[start..i],
            });
            continue;
        }

        // Numbers
        if bytes[i].is_ascii_digit() {
            let start = i;
            while i < len && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'.' || bytes[i] == b'-') {
                i += 1;
            }
            tokens.push(SyntaxToken {
                kind: TokenKind::NumberLiteral,
                text: &line[start..i],
            });
            continue;
        }

        // Keys / Words
        if bytes[i].is_ascii_alphabetic() || bytes[i] == b'_' || bytes[i] == b'-' {
            let start = i;
            while i < len && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_' || bytes[i] == b'-') {
                i += 1;
            }
            let word = &line[start..i];
            let kind = match word {
                "true" | "false" | "null" | "nil" => TokenKind::Keyword,
                _ => TokenKind::Attribute,
            };
            tokens.push(SyntaxToken { kind, text: word });
            continue;
        }

        let start = i;
        while i < len
            && !bytes[i].is_ascii_alphanumeric()
            && bytes[i] != b'_'
            && bytes[i] != b'-'
            && bytes[i] != b'"'
            && bytes[i] != b'\''
            && bytes[i] != b'#'
            && bytes[i] != b'/'
        {
            i += 1;
        }
        if i > start {
            tokens.push(SyntaxToken {
                kind: TokenKind::Punctuation,
                text: &line[start..i],
            });
        }
    }

    tokens
}

fn tokenize_shell<'a>(line: &'a str) -> Vec<SyntaxToken<'a>> {
    let mut tokens = Vec::new();
    let bytes = line.as_bytes();
    let len = bytes.len();
    let mut i = 0;

    while i < len {
        if bytes[i] == b'#' {
            tokens.push(SyntaxToken {
                kind: TokenKind::Comment,
                text: &line[i..],
            });
            break;
        }

        if bytes[i] == b'$' {
            let start = i;
            i += 1;
            while i < len && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_' || bytes[i] == b'{' || bytes[i] == b'}') {
                i += 1;
            }
            tokens.push(SyntaxToken {
                kind: TokenKind::Variable,
                text: &line[start..i],
            });
            continue;
        }

        if bytes[i] == b'"' || bytes[i] == b'\'' {
            let quote = bytes[i];
            let start = i;
            i += 1;
            while i < len && bytes[i] != quote {
                i += 1;
            }
            if i < len {
                i += 1;
            }
            tokens.push(SyntaxToken {
                kind: TokenKind::StringLiteral,
                text: &line[start..i],
            });
            continue;
        }

        if bytes[i].is_ascii_alphabetic() || bytes[i] == b'_' {
            let start = i;
            while i < len && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_' || bytes[i] == b'-') {
                i += 1;
            }
            let word = &line[start..i];
            let kind = match word {
                "if" | "then" | "else" | "elif" | "fi" | "for" | "in" | "do" | "done"
                | "case" | "esac" | "while" | "until" | "function" | "export" | "set"
                | "local" | "return" | "exit" => TokenKind::Keyword,
                _ => TokenKind::Plain,
            };
            tokens.push(SyntaxToken { kind, text: word });
            continue;
        }

        let start = i;
        while i < len && !bytes[i].is_ascii_alphanumeric() && bytes[i] != b'_' && bytes[i] != b'$' && bytes[i] != b'#' && bytes[i] != b'"' && bytes[i] != b'\'' {
            i += 1;
        }
        if i > start {
            tokens.push(SyntaxToken {
                kind: TokenKind::Punctuation,
                text: &line[start..i],
            });
        }
    }

    tokens
}

fn tokenize_markup<'a>(line: &'a str) -> Vec<SyntaxToken<'a>> {
    let mut tokens = Vec::new();
    let bytes = line.as_bytes();
    let len = bytes.len();
    let mut i = 0;

    while i < len {
        if bytes[i] == b'<' {
            let start = i;
            i += 1;
            while i < len && bytes[i] != b'>' {
                i += 1;
            }
            if i < len {
                i += 1;
            }
            tokens.push(SyntaxToken {
                kind: TokenKind::Keyword,
                text: &line[start..i],
            });
            continue;
        }

        if bytes[i] == b'"' || bytes[i] == b'\'' {
            let quote = bytes[i];
            let start = i;
            i += 1;
            while i < len && bytes[i] != quote {
                i += 1;
            }
            if i < len {
                i += 1;
            }
            tokens.push(SyntaxToken {
                kind: TokenKind::StringLiteral,
                text: &line[start..i],
            });
            continue;
        }

        let start = i;
        while i < len && bytes[i] != b'<' && bytes[i] != b'"' && bytes[i] != b'\'' {
            i += 1;
        }
        if i > start {
            tokens.push(SyntaxToken {
                kind: TokenKind::Plain,
                text: &line[start..i],
            });
        }
    }

    tokens
}

fn tokenize_generic<'a>(line: &'a str) -> Vec<SyntaxToken<'a>> {
    vec![SyntaxToken {
        kind: TokenKind::Plain,
        text: line,
    }]
}

/// Convert tokens to styled Ratatui Spans based on line type and active theme.
pub fn highlight_tokens_to_spans<'a>(
    tokens: &[SyntaxToken<'a>],
    line_type: DiffLineType,
    theme: &Theme,
) -> Vec<Span<'a>> {
    let base_bg = match line_type {
        DiffLineType::Addition => Some(Color::Rgb(20, 45, 30)),
        DiffLineType::Deletion => Some(Color::Rgb(50, 20, 25)),
        _ => None,
    };

    let mut spans = Vec::with_capacity(tokens.len());
    for tok in tokens {
        let mut style = match tok.kind {
            TokenKind::Keyword => Style::default().fg(theme.primary).add_modifier(Modifier::BOLD),
            TokenKind::Type => Style::default().fg(theme.secondary).add_modifier(Modifier::BOLD),
            TokenKind::Function => Style::default().fg(theme.accent),
            TokenKind::StringLiteral => Style::default().fg(theme.success),
            TokenKind::NumberLiteral => Style::default().fg(theme.warning),
            TokenKind::Comment => Style::default().fg(theme.muted).add_modifier(Modifier::ITALIC),
            TokenKind::Attribute => Style::default().fg(theme.warning).add_modifier(Modifier::BOLD),
            TokenKind::Variable => Style::default().fg(theme.info),
            TokenKind::Punctuation | TokenKind::Operator => Style::default().fg(theme.muted),
            TokenKind::Plain => match line_type {
                DiffLineType::Addition => Style::default().fg(theme.success),
                DiffLineType::Deletion => Style::default().fg(theme.error),
                _ => Style::default().fg(theme.foreground),
            },
        };

        if let Some(bg) = base_bg {
            style = style.bg(bg);
        }

        spans.push(Span::styled(tok.text, style));
    }

    spans
}

// ============================================================================
// Diff View State & Controller
// ============================================================================

/// Full interactive state for the Diff View TUI.
#[derive(Debug, Clone)]
pub struct DiffViewState {
    /// Changed files loaded into the viewer.
    pub files: Vec<DiffFile>,
    /// Currently active file index.
    pub active_file_idx: usize,
    /// Current view presentation mode.
    pub view_mode: DiffViewMode,
    /// Collapsible sidebar visibility.
    pub show_sidebar: bool,
    /// Keybindings help modal visibility.
    pub show_help: bool,
    /// Line numbers visibility.
    pub show_line_numbers: bool,
    /// Syntax highlighting toggle.
    pub syntax_highlighting: bool,
    /// Vertical scroll offset.
    pub scroll_y: usize,
    /// Horizontal scroll offset.
    pub scroll_x: usize,
    /// Theme for rendering colors and styles.
    pub theme: Theme,
    /// Transient status or notification message.
    pub status_message: Option<String>,
    /// Search/filter query for filtering files.
    pub filter_query: String,
    /// Filter mode active.
    pub is_filtering: bool,
}

impl Default for DiffViewState {
    fn default() -> Self {
        Self::new(Vec::new())
    }
}

impl DiffViewState {
    pub fn new(files: Vec<DiffFile>) -> Self {
        Self {
            files,
            active_file_idx: 0,
            view_mode: DiffViewMode::Unified,
            show_sidebar: true,
            show_help: false,
            show_line_numbers: true,
            syntax_highlighting: true,
            scroll_y: 0,
            scroll_x: 0,
            theme: Theme::auto(),
            status_message: None,
            filter_query: String::new(),
            is_filtering: false,
        }
    }

    /// Construct diff view state from two raw file strings.
    pub fn from_strings(
        old_content: &str,
        new_content: &str,
        file_path: &str,
        context_radius: usize,
    ) -> Self {
        let diff = TextDiff::from_lines(old_content, new_content);
        let mut hunks = Vec::new();
        let mut hunk_idx = 0;

        for group in diff.grouped_ops(context_radius) {
            if group.is_empty() {
                continue;
            }

            let mut lines = Vec::new();
            let mut old_start = 0;
            let mut old_count = 0;
            let mut new_start = 0;
            let mut new_count = 0;
            let mut is_first = true;

            for op in &group {
                if is_first {
                    old_start = op.old_range().start + 1;
                    new_start = op.new_range().start + 1;
                    is_first = false;
                }
                old_count += op.old_range().len();
                new_count += op.new_range().len();

                for change in diff.iter_changes(op) {
                    match change.tag() {
                        ChangeTag::Equal => {
                            lines.push(DiffLine::context(
                                change.old_index().unwrap_or(0) + 1,
                                change.new_index().unwrap_or(0) + 1,
                                change.value().trim_end_matches(['\r', '\n']),
                            ));
                        }
                        ChangeTag::Delete => {
                            lines.push(DiffLine::deletion(
                                change.old_index().unwrap_or(0) + 1,
                                change.value().trim_end_matches(['\r', '\n']),
                            ));
                        }
                        ChangeTag::Insert => {
                            lines.push(DiffLine::addition(
                                change.new_index().unwrap_or(0) + 1,
                                change.value().trim_end_matches(['\r', '\n']),
                            ));
                        }
                    }
                }
            }

            let header = format!("Hunk {}", hunk_idx + 1);
            let hunk = DiffHunk::new(
                hunk_idx,
                old_start,
                old_count,
                new_start,
                new_count,
                header,
                lines,
            );
            hunks.push(hunk);
            hunk_idx += 1;
        }

        let mut file = DiffFile::new(file_path);
        file.original_content = Some(old_content.to_string());
        file.modified_content = Some(new_content.to_string());
        file.hunks = hunks;

        Self::new(vec![file])
    }

    /// Construct diff view state from a unified diff patch string.
    pub fn from_unified_diff(diff_str: &str) -> anyhow::Result<Self> {
        let file_patches = crate::tools::patch::parse_unified_diff(diff_str)?;
        Ok(Self::from_file_patches(&file_patches))
    }

    /// Construct from parsed `FilePatch` structs.
    pub fn from_file_patches(patches: &[FilePatch]) -> Self {
        let mut files = Vec::new();

        for p in patches {
            let path = p
                .new_path
                .as_deref()
                .or(p.old_path.as_deref())
                .unwrap_or("unknown")
                .trim_start_matches("a/")
                .trim_start_matches("b/")
                .to_string();

            let mut diff_file = DiffFile::new(path);
            diff_file.old_path = p.old_path.clone();
            diff_file.new_path = p.new_path.clone();
            diff_file.is_new = p.is_new;
            diff_file.is_deleted = p.is_deleted;

            let mut hunks = Vec::new();
            for (idx, h) in p.hunks.iter().enumerate() {
                let mut lines = Vec::new();
                let mut old_lineno = h.old_start;
                let mut new_lineno = h.new_start;

                for l in &h.lines {
                    match l {
                        PatchHunkLine::Context(s) => {
                            lines.push(DiffLine::context(old_lineno, new_lineno, s.clone()));
                            old_lineno += 1;
                            new_lineno += 1;
                        }
                        PatchHunkLine::Add(s) => {
                            lines.push(DiffLine::addition(new_lineno, s.clone()));
                            new_lineno += 1;
                        }
                        PatchHunkLine::Remove(s) => {
                            lines.push(DiffLine::deletion(old_lineno, s.clone()));
                            old_lineno += 1;
                        }
                    }
                }

                let diff_hunk = DiffHunk::new(
                    idx,
                    h.old_start,
                    h.old_lines,
                    h.new_start,
                    h.new_lines,
                    h.header.clone(),
                    lines,
                );
                hunks.push(diff_hunk);
            }

            diff_file.hunks = hunks;
            files.push(diff_file);
        }

        Self::new(files)
    }

    // ------------------------------------------------------------------------
    // Query & Metric Methods
    // ------------------------------------------------------------------------

    pub fn total_files(&self) -> usize {
        self.files.len()
    }

    pub fn total_hunks(&self) -> usize {
        self.files.iter().map(|f| f.hunks.len()).sum()
    }

    pub fn total_staged_hunks(&self) -> usize {
        self.files.iter().map(|f| f.staged_hunks_count()).sum()
    }

    pub fn total_rejected_hunks(&self) -> usize {
        self.files.iter().map(|f| f.rejected_hunks_count()).sum()
    }

    pub fn total_pending_hunks(&self) -> usize {
        self.files.iter().map(|f| f.pending_hunks_count()).sum()
    }

    pub fn total_additions(&self) -> usize {
        self.files.iter().map(|f| f.additions()).sum()
    }

    pub fn total_deletions(&self) -> usize {
        self.files.iter().map(|f| f.deletions()).sum()
    }

    pub fn active_file(&self) -> Option<&DiffFile> {
        self.files.get(self.active_file_idx)
    }

    pub fn active_file_mut(&mut self) -> Option<&mut DiffFile> {
        self.files.get_mut(self.active_file_idx)
    }

    // ------------------------------------------------------------------------
    // Staging & Hunk Control Operations
    // ------------------------------------------------------------------------

    /// Stage the currently active hunk and advance to the next hunk.
    pub fn stage_current_hunk(&mut self) {
        let mut msg = None;
        if let Some(file) = self.files.get_mut(self.active_file_idx) {
            if let Some(hunk) = file.active_hunk_mut() {
                hunk.stage();
                let idx = hunk.index + 1;
                let total = file.hunks.len();
                msg = Some(format!("✓ Hunk {idx}/{total} staged"));
            }
            file.next_hunk();
        }
        if let Some(m) = msg {
            self.status_message = Some(m);
        }
    }

    /// Reject the currently active hunk and advance to the next hunk.
    pub fn reject_current_hunk(&mut self) {
        let mut msg = None;
        if let Some(file) = self.files.get_mut(self.active_file_idx) {
            if let Some(hunk) = file.active_hunk_mut() {
                hunk.reject();
                let idx = hunk.index + 1;
                let total = file.hunks.len();
                msg = Some(format!("✗ Hunk {idx}/{total} rejected"));
            }
            file.next_hunk();
        }
        if let Some(m) = msg {
            self.status_message = Some(m);
        }
    }

    /// Reset current hunk to pending.
    pub fn reset_current_hunk(&mut self) {
        let mut msg = None;
        if let Some(file) = self.files.get_mut(self.active_file_idx) {
            if let Some(hunk) = file.active_hunk_mut() {
                hunk.reset();
                let idx = hunk.index + 1;
                let total = file.hunks.len();
                msg = Some(format!("• Hunk {idx}/{total} reset to pending"));
            }
        }
        if let Some(m) = msg {
            self.status_message = Some(m);
        }
    }

    /// Toggle status of currently active hunk.
    pub fn toggle_current_hunk(&mut self) {
        let mut msg = None;
        if let Some(file) = self.files.get_mut(self.active_file_idx) {
            if let Some(hunk) = file.active_hunk_mut() {
                hunk.toggle();
                let idx = hunk.index + 1;
                let badge = hunk.status.badge();
                msg = Some(format!("Hunk {idx} set to {badge}"));
            }
        }
        if let Some(m) = msg {
            self.status_message = Some(m);
        }
    }

    /// Stage all hunks in current file.
    pub fn stage_current_file(&mut self) {
        let mut msg = None;
        if let Some(file) = self.files.get_mut(self.active_file_idx) {
            file.stage_all();
            let path = file.path.clone();
            msg = Some(format!("✓ All hunks staged in '{path}'"));
        }
        if let Some(m) = msg {
            self.status_message = Some(m);
        }
    }

    /// Reject all hunks in current file.
    pub fn reject_current_file(&mut self) {
        let mut msg = None;
        if let Some(file) = self.files.get_mut(self.active_file_idx) {
            file.reject_all();
            let path = file.path.clone();
            msg = Some(format!("✗ All hunks rejected in '{path}'"));
        }
        if let Some(m) = msg {
            self.status_message = Some(m);
        }
    }

    /// Stage all hunks across all files.
    pub fn stage_all(&mut self) {
        for f in &mut self.files {
            f.stage_all();
        }
        self.status_message = Some("✓ Staged all hunks in all files".to_string());
    }

    /// Reject all hunks across all files.
    pub fn reject_all(&mut self) {
        for f in &mut self.files {
            f.reject_all();
        }
        self.status_message = Some("✗ Rejected all hunks in all files".to_string());
    }

    /// Reset all hunks to pending.
    pub fn reset_all(&mut self) {
        for f in &mut self.files {
            f.reset_all();
        }
        self.status_message = Some("• Reset all hunks to pending".to_string());
    }

    // ------------------------------------------------------------------------
    // Navigation & UI Toggles
    // ------------------------------------------------------------------------

    pub fn next_hunk(&mut self) {
        if let Some(file) = self.active_file_mut() {
            if !file.next_hunk() && self.active_file_idx + 1 < self.files.len() {
                self.active_file_idx += 1;
                if let Some(next_file) = self.active_file_mut() {
                    next_file.active_hunk_idx = 0;
                }
            }
        }
        self.scroll_y = 0;
    }

    pub fn prev_hunk(&mut self) {
        if let Some(file) = self.active_file_mut() {
            if !file.prev_hunk() && self.active_file_idx > 0 {
                self.active_file_idx -= 1;
                if let Some(prev_file) = self.active_file_mut() {
                    prev_file.active_hunk_idx = prev_file.hunks.len().saturating_sub(1);
                }
            }
        }
        self.scroll_y = 0;
    }

    pub fn next_file(&mut self) {
        if !self.files.is_empty() && self.active_file_idx + 1 < self.files.len() {
            self.active_file_idx += 1;
            self.scroll_y = 0;
        }
    }

    pub fn prev_file(&mut self) {
        if self.active_file_idx > 0 {
            self.active_file_idx -= 1;
            self.scroll_y = 0;
        }
    }

    pub fn scroll_down(&mut self, lines: usize) {
        self.scroll_y = self.scroll_y.saturating_add(lines);
    }

    pub fn scroll_up(&mut self, lines: usize) {
        self.scroll_y = self.scroll_y.saturating_sub(lines);
    }

    pub fn toggle_view_mode(&mut self) {
        self.view_mode = self.view_mode.toggle();
        self.status_message = Some(format!("View mode: {}", self.view_mode.name()));
    }

    pub fn toggle_sidebar(&mut self) {
        self.show_sidebar = !self.show_sidebar;
    }

    pub fn toggle_help(&mut self) {
        self.show_help = !self.show_help;
    }

    pub fn toggle_syntax(&mut self) {
        self.syntax_highlighting = !self.syntax_highlighting;
        self.status_message = Some(format!(
            "Syntax highlighting: {}",
            if self.syntax_highlighting { "ON" } else { "OFF" }
        ));
    }

    // ------------------------------------------------------------------------
    // Patch / Result Generation
    // ------------------------------------------------------------------------

    /// Generate complete unified diff string containing all staged hunks.
    pub fn get_staged_diff(&self) -> String {
        let mut diff = String::new();
        for file in &self.files {
            let patch = file.generate_staged_patch();
            if !patch.is_empty() {
                diff.push_str(&patch);
            }
        }
        diff
    }

    /// Reconstruct staged files as a list of `(path, content)`.
    pub fn get_staged_files(&self) -> Vec<(String, String)> {
        let mut out = Vec::new();
        for file in &self.files {
            if let Some(content) = file.reconstruct_staged_content() {
                out.push((file.path.clone(), content));
            }
        }
        out
    }
}

/// Final result returned when exiting interactive diff view.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DiffViewResult {
    /// Changes accepted with staged diff and modified file contents.
    Applied {
        staged_diff: String,
        files: Vec<(String, String)>,
    },
    /// All changes rejected / discarded.
    Rejected,
    /// Cancelled / closed without making changes.
    Cancelled,
}

// ============================================================================
// Ratatui Widget & UI Rendering
// ============================================================================

/// Ratatui widget that renders the complete visual diff reviewer UI.
pub struct DiffViewerWidget<'a> {
    state: &'a DiffViewState,
}

impl<'a> DiffViewerWidget<'a> {
    pub fn new(state: &'a DiffViewState) -> Self {
        Self { state }
    }
}

impl<'a> Widget for DiffViewerWidget<'a> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let _theme = &self.state.theme;

        // Top-level vertical layout: Header bar, Main area, Footer status bar
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(3), // Header & progress
                Constraint::Min(5),    // Main diff / split view
                Constraint::Length(2), // Footer & keybindings
            ])
            .split(area);

        self.render_header(chunks[0], buf);

        // Split main area between optional sidebar and diff viewer
        if self.state.show_sidebar && !self.state.files.is_empty() {
            let main_chunks = Layout::default()
                .direction(Direction::Horizontal)
                .constraints([
                    Constraint::Length(30), // Sidebar
                    Constraint::Min(20),    // Diff area
                ])
                .split(chunks[1]);

            self.render_sidebar(main_chunks[0], buf);
            self.render_diff_area(main_chunks[1], buf);
        } else {
            self.render_diff_area(chunks[1], buf);
        }

        self.render_footer(chunks[2], buf);

        if self.state.show_help {
            self.render_help_modal(area, buf);
        }
    }
}

impl<'a> DiffViewerWidget<'a> {
    fn render_header(&self, area: Rect, buf: &mut Buffer) {
        let theme = &self.state.theme;

        let total_files = self.state.total_files();
        let total_hunks = self.state.total_hunks();
        let staged_hunks = self.state.total_staged_hunks();
        let rejected_hunks = self.state.total_rejected_hunks();
        let additions = self.state.total_additions();
        let deletions = self.state.total_deletions();

        let title = Span::styled(
            " ⚡ FUSION DIFF REVIEWER ",
            Style::default()
                .fg(theme.primary)
                .add_modifier(Modifier::BOLD),
        );

        let view_badge = Span::styled(
            format!("[{}] ", self.state.view_mode.name()),
            Style::default().fg(theme.accent),
        );

        let stats_badge = Span::styled(
            format!("+{} -{} ", additions, deletions),
            Style::default().fg(theme.success).add_modifier(Modifier::BOLD),
        );

        let progress_badge = Span::styled(
            format!(
                "Hunks: {}/{} staged ({} rejected) ",
                staged_hunks, total_hunks, rejected_hunks
            ),
            Style::default().fg(theme.info),
        );

        let block = Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(theme.border_focused))
            .title(title)
            .title_alignment(Alignment::Left);

        let inner = block.inner(area);
        block.render(area, buf);

        let header_line = Line::from(vec![
            view_badge,
            Span::raw(" | "),
            stats_badge,
            Span::raw(" | "),
            progress_badge,
            Span::raw(" | "),
            Span::styled(
                format!("Files: {}/{}", self.state.active_file_idx + 1, total_files.max(1)),
                Style::default().fg(theme.foreground),
            ),
        ]);

        let paragraph = Paragraph::new(header_line);
        paragraph.render(inner, buf);
    }

    fn render_sidebar(&self, area: Rect, buf: &mut Buffer) {
        let theme = &self.state.theme;

        let block = Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(theme.border))
            .title(" Changed Files ")
            .title_alignment(Alignment::Left);

        let inner = block.inner(area);
        block.render(area, buf);

        if self.state.files.is_empty() {
            let empty = Paragraph::new("No changed files").style(Style::default().fg(theme.muted));
            empty.render(inner, buf);
            return;
        }

        let mut items = Vec::new();
        for (idx, file) in self.state.files.iter().enumerate() {
            let is_active = idx == self.state.active_file_idx;
            let status = file.status();

            let status_span = Span::styled(
                format!("{} ", status.icon()),
                status.style(theme),
            );

            let prefix = if is_active { "▶ " } else { "  " };

            let name_style = if is_active {
                Style::default()
                    .fg(theme.primary)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(theme.foreground)
            };

            let adds = file.additions();
            let dels = file.deletions();
            let stats_span = Span::styled(
                format!(" +{adds}/-{dels}"),
                Style::default().fg(theme.muted),
            );

            let hunk_progress = Span::styled(
                format!(" [{}/{}]", file.staged_hunks_count(), file.hunks.len()),
                Style::default().fg(theme.info),
            );

            let line = Line::from(vec![
                Span::styled(prefix, name_style),
                status_span,
                Span::styled(&file.path, name_style),
                stats_span,
                hunk_progress,
            ]);

            items.push(ListItem::new(line));
        }

        let list = List::new(items);
        list.render(inner, buf);
    }

    fn render_diff_area(&self, area: Rect, buf: &mut Buffer) {
        match self.state.view_mode {
            DiffViewMode::Unified => self.render_unified_diff(area, buf),
            DiffViewMode::SideBySide => self.render_side_by_side_diff(area, buf),
        }
    }

    fn render_unified_diff(&self, area: Rect, buf: &mut Buffer) {
        let theme = &self.state.theme;

        let active_file = match self.state.active_file() {
            Some(f) => f,
            None => {
                let empty = Paragraph::new("No active file or diff").style(Style::default().fg(theme.muted));
                empty.render(area, buf);
                return;
            }
        };

        let lang = SyntaxLanguage::from_path(&active_file.path);
        let block = Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(theme.border_focused))
            .title(format!(" File: {} ({:?}) ", active_file.path, lang))
            .title_alignment(Alignment::Left);

        let inner = block.inner(area);
        block.render(area, buf);

        if active_file.hunks.is_empty() {
            let p = Paragraph::new("No diff hunks in this file").style(Style::default().fg(theme.muted));
            p.render(inner, buf);
            return;
        }

        let mut lines = Vec::new();
        for (h_idx, hunk) in active_file.hunks.iter().enumerate() {
            let is_active_hunk = h_idx == active_file.active_hunk_idx;

            // Hunk Header Line
            let hunk_badge = hunk.status.badge();
            let hunk_badge_style = hunk.status.style(theme);
            let active_marker = if is_active_hunk { "▶ [ACTIVE HUNK] " } else { "  " };

            let header_spans = vec![
                Span::styled(
                    active_marker,
                    if is_active_hunk {
                        Style::default().fg(theme.warning).add_modifier(Modifier::BOLD)
                    } else {
                        Style::default().fg(theme.muted)
                    },
                ),
                Span::styled(hunk_badge, hunk_badge_style),
                Span::styled(
                    format!(
                        " @@ -{},{} +{},{} @@ {}",
                        hunk.old_start, hunk.old_lines, hunk.new_start, hunk.new_lines, hunk.header
                    ),
                    Style::default().fg(theme.accent).add_modifier(Modifier::BOLD),
                ),
            ];
            lines.push(Line::from(header_spans));

            // Hunk Code Lines
            for l in &hunk.lines {
                let mut line_spans = Vec::new();

                // Line numbers gutter
                if self.state.show_line_numbers {
                    let old_str = l.old_lineno.map(|n| format!("{:4} ", n)).unwrap_or_else(|| "     ".to_string());
                    let new_str = l.new_lineno.map(|n| format!("{:4} ", n)).unwrap_or_else(|| "     ".to_string());

                    line_spans.push(Span::styled(old_str, Style::default().fg(theme.muted)));
                    line_spans.push(Span::styled(new_str, Style::default().fg(theme.muted)));
                    line_spans.push(Span::styled("│ ", Style::default().fg(theme.border)));
                }

                // Change tag indicator
                let (indicator, ind_style) = match l.line_type {
                    DiffLineType::Addition => ("+ ", Style::default().fg(theme.success).add_modifier(Modifier::BOLD)),
                    DiffLineType::Deletion => ("- ", Style::default().fg(theme.error).add_modifier(Modifier::BOLD)),
                    _ => ("  ", Style::default().fg(theme.muted)),
                };
                line_spans.push(Span::styled(indicator, ind_style));

                // Syntax-highlighted code content
                if self.state.syntax_highlighting {
                    let tokens = tokenize_line(&l.content, lang);
                    let highlighted = highlight_tokens_to_spans(&tokens, l.line_type, theme);
                    line_spans.extend(highlighted);
                } else {
                    let code_style = match l.line_type {
                        DiffLineType::Addition => Style::default().fg(theme.success),
                        DiffLineType::Deletion => Style::default().fg(theme.error),
                        _ => Style::default().fg(theme.foreground),
                    };
                    line_spans.push(Span::styled(&l.content, code_style));
                }

                lines.push(Line::from(line_spans));
            }

            // Separator between hunks
            lines.push(Line::from(Span::styled(
                "─".repeat(inner.width.saturating_sub(2) as usize),
                Style::default().fg(theme.border),
            )));
        }

        // Apply vertical scrolling
        let visible_lines: Vec<Line> = lines.into_iter().skip(self.state.scroll_y).collect();
        let paragraph = Paragraph::new(visible_lines);
        paragraph.render(inner, buf);
    }

    fn render_side_by_side_diff(&self, area: Rect, buf: &mut Buffer) {
        let theme = &self.state.theme;

        let active_file = match self.state.active_file() {
            Some(f) => f,
            None => {
                let empty = Paragraph::new("No active file").style(Style::default().fg(theme.muted));
                empty.render(area, buf);
                return;
            }
        };

        let lang = SyntaxLanguage::from_path(&active_file.path);
        let block = Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(theme.border_focused))
            .title(format!(" Side-by-Side: {} ", active_file.path))
            .title_alignment(Alignment::Left);

        let inner = block.inner(area);
        block.render(area, buf);

        // Split horizontally into Left (Original) and Right (Modified) columns
        let split = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
            .split(inner);

        let left_area = split[0];
        let right_area = split[1];

        let mut left_lines = Vec::new();
        let mut right_lines = Vec::new();

        for (h_idx, hunk) in active_file.hunks.iter().enumerate() {
            let is_active = h_idx == active_file.active_hunk_idx;
            let badge = hunk.status.badge();

            // Header for both sides
            left_lines.push(Line::from(vec![
                Span::styled(if is_active { "▶ " } else { "  " }, Style::default().fg(theme.warning)),
                Span::styled(badge, hunk.status.style(theme)),
                Span::styled(format!(" @@ -{},{} @@", hunk.old_start, hunk.old_lines), Style::default().fg(theme.accent)),
            ]));

            right_lines.push(Line::from(vec![
                Span::styled(if is_active { "▶ " } else { "  " }, Style::default().fg(theme.warning)),
                Span::styled(badge, hunk.status.style(theme)),
                Span::styled(format!(" @@ +{},{} @@", hunk.new_start, hunk.new_lines), Style::default().fg(theme.accent)),
            ]));

            // Group additions and deletions
            for l in &hunk.lines {
                match l.line_type {
                    DiffLineType::Context => {
                        let tokens = tokenize_line(&l.content, lang);
                        let left_spans = highlight_tokens_to_spans(&tokens, DiffLineType::Context, theme);
                        let right_spans = highlight_tokens_to_spans(&tokens, DiffLineType::Context, theme);

                        let mut l_row = vec![
                            Span::styled(format!("{:4} │ ", l.old_lineno.unwrap_or(0)), Style::default().fg(theme.muted)),
                        ];
                        l_row.extend(left_spans);
                        left_lines.push(Line::from(l_row));

                        let mut r_row = vec![
                            Span::styled(format!("{:4} │ ", l.new_lineno.unwrap_or(0)), Style::default().fg(theme.muted)),
                        ];
                        r_row.extend(right_spans);
                        right_lines.push(Line::from(r_row));
                    }
                    DiffLineType::Deletion => {
                        let tokens = tokenize_line(&l.content, lang);
                        let spans = highlight_tokens_to_spans(&tokens, DiffLineType::Deletion, theme);

                        let mut row = vec![
                            Span::styled(format!("{:4} │-", l.old_lineno.unwrap_or(0)), Style::default().fg(theme.error)),
                        ];
                        row.extend(spans);
                        left_lines.push(Line::from(row));
                        right_lines.push(Line::from(Span::styled("     │", Style::default().fg(theme.border))));
                    }
                    DiffLineType::Addition => {
                        let tokens = tokenize_line(&l.content, lang);
                        let spans = highlight_tokens_to_spans(&tokens, DiffLineType::Addition, theme);

                        let mut row = vec![
                            Span::styled(format!("{:4} │+", l.new_lineno.unwrap_or(0)), Style::default().fg(theme.success)),
                        ];
                        row.extend(spans);
                        left_lines.push(Line::from(Span::styled("     │", Style::default().fg(theme.border))));
                        right_lines.push(Line::from(row));
                    }
                    _ => {}
                }
            }

            left_lines.push(Line::from(Span::styled("─".repeat(left_area.width as usize), Style::default().fg(theme.border))));
            right_lines.push(Line::from(Span::styled("─".repeat(right_area.width as usize), Style::default().fg(theme.border))));
        }

        let left_visible: Vec<Line> = left_lines.into_iter().skip(self.state.scroll_y).collect();
        let right_visible: Vec<Line> = right_lines.into_iter().skip(self.state.scroll_y).collect();

        Paragraph::new(left_visible).render(left_area, buf);
        Paragraph::new(right_visible).render(right_area, buf);
    }

    fn render_footer(&self, area: Rect, buf: &mut Buffer) {
        let theme = &self.state.theme;

        let status_text = self.state.status_message.as_deref().unwrap_or("Ready");

        let hints = vec![
            Span::styled("[s/Space] ", Style::default().fg(theme.success).add_modifier(Modifier::BOLD)),
            Span::raw("Stage  "),
            Span::styled("[r/x] ", Style::default().fg(theme.error).add_modifier(Modifier::BOLD)),
            Span::raw("Reject  "),
            Span::styled("[u] ", Style::default().fg(theme.warning)),
            Span::raw("Reset  "),
            Span::styled("[Tab] ", Style::default().fg(theme.accent)),
            Span::raw("View  "),
            Span::styled("[n/p] ", Style::default().fg(theme.info)),
            Span::raw("Hunk  "),
            Span::styled("[>/<] ", Style::default().fg(theme.info)),
            Span::raw("File  "),
            Span::styled("[?] ", Style::default().fg(theme.muted)),
            Span::raw("Help  "),
            Span::styled("[Enter] ", Style::default().fg(theme.primary).add_modifier(Modifier::BOLD)),
            Span::raw("Apply  "),
            Span::styled("[q/Esc] ", Style::default().fg(theme.muted)),
            Span::raw("Quit"),
        ];

        let mut spans = vec![
            Span::styled(format!(" Status: {} ", status_text), Style::default().fg(theme.foreground)),
            Span::raw(" | "),
        ];
        spans.extend(hints);
        let paragraph = Paragraph::new(Line::from(spans));
        paragraph.render(area, buf);
    }

    fn render_help_modal(&self, area: Rect, buf: &mut Buffer) {
        let theme = &self.state.theme;

        let modal_area = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Percentage(15),
                Constraint::Percentage(70),
                Constraint::Percentage(15),
            ])
            .split(area)[1];

        let modal_area = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([
                Constraint::Percentage(20),
                Constraint::Percentage(60),
                Constraint::Percentage(20),
            ])
            .split(modal_area)[1];

        Clear.render(modal_area, buf);

        let block = Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Double)
            .border_style(Style::default().fg(theme.primary).add_modifier(Modifier::BOLD))
            .title(" Interactive Diff Reviewer Keybindings ")
            .title_alignment(Alignment::Center);

        let inner = block.inner(modal_area);
        block.render(modal_area, buf);

        let help_text = vec![
            Line::from(vec![
                Span::styled("  Hunk Staging Controls:", Style::default().fg(theme.primary).add_modifier(Modifier::BOLD)),
            ]),
            Line::from("    s / Space / y    Stage current hunk"),
            Line::from("    r / n / x        Reject current hunk"),
            Line::from("    u                Reset current hunk to pending"),
            Line::from("    S / a            Stage entire file"),
            Line::from("    R                Reject entire file"),
            Line::from("    A                Stage all hunks in all files"),
            Line::from("    X                Reject all hunks in all files"),
            Line::from(""),
            Line::from(vec![
                Span::styled("  Navigation & Views:", Style::default().fg(theme.secondary).add_modifier(Modifier::BOLD)),
            ]),
            Line::from("    n / ] / J        Next hunk"),
            Line::from("    p / [ / K        Previous hunk"),
            Line::from("    > / Ctrl+N       Next file"),
            Line::from("    < / Ctrl+P       Previous file"),
            Line::from("    Tab / t          Toggle Unified / Side-by-Side view"),
            Line::from("    b / F            Toggle File Sidebar"),
            Line::from("    h / S            Toggle Syntax Highlighting"),
            Line::from("    j / k / Arrows   Scroll up / down"),
            Line::from("    Ctrl+D / Ctrl+U  Page down / page up"),
            Line::from(""),
            Line::from(vec![
                Span::styled("  Actions:", Style::default().fg(theme.success).add_modifier(Modifier::BOLD)),
            ]),
            Line::from("    Enter            Apply staged changes & exit"),
            Line::from("    q / Esc          Cancel & exit"),
            Line::from("    ? / h            Toggle this help modal"),
        ];

        let p = Paragraph::new(help_text);
        p.render(inner, buf);
    }
}

// ============================================================================
// Interactive Event Loop Runner
// ============================================================================

/// Run the interactive full-screen TUI diff viewer.
pub fn run_interactive_diff_viewer(state: &mut DiffViewState) -> std::io::Result<DiffViewResult> {
    let mut stdout = stdout();
    execute!(stdout, EnterAlternateScreen, cursor::Hide)?;
    let _raw_mode = RawModeGuard::enter()?;

    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let result = run_diff_view_event_loop(&mut terminal, state);

    // Clean teardown
    let _ = execute!(terminal.backend_mut(), cursor::Show, LeaveAlternateScreen);
    result
}

fn run_diff_view_event_loop(
    terminal: &mut Terminal<CrosstermBackend<Stdout>>,
    state: &mut DiffViewState,
) -> std::io::Result<DiffViewResult> {
    loop {
        terminal.draw(|f| {
            let widget = DiffViewerWidget::new(state);
            f.render_widget(widget, f.area());
        })?;

        if let Event::Key(key) = event::read()? {
            if key.kind != KeyEventKind::Press {
                continue;
            }

            // If help is active, dismiss on Esc or ?
            if state.show_help {
                if matches!(key.code, KeyCode::Esc | KeyCode::Char('?') | KeyCode::Char('q')) {
                    state.show_help = false;
                }
                continue;
            }

            match key.code {
                // Key modifier checks first
                KeyCode::Char('s') if key.modifiers.contains(KeyModifiers::SHIFT) => {
                    state.stage_current_file();
                }
                KeyCode::Char('r') if key.modifiers.contains(KeyModifiers::SHIFT) => {
                    state.reject_current_file();
                }
                KeyCode::Char('a') if key.modifiers.contains(KeyModifiers::SHIFT) => {
                    state.stage_all();
                }
                KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                    state.scroll_up(10);
                }
                KeyCode::Char('d') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                    state.scroll_down(10);
                }

                // Staging actions
                KeyCode::Char('s') | KeyCode::Char('y') | KeyCode::Char(' ') => {
                    state.stage_current_hunk();
                }
                KeyCode::Char('r') | KeyCode::Char('x') => {
                    state.reject_current_hunk();
                }
                KeyCode::Char('u') => {
                    state.reset_current_hunk();
                }
                KeyCode::Char('a') => {
                    state.stage_current_file();
                }
                KeyCode::Char('X') => {
                    state.reject_all();
                }

                // Navigation
                KeyCode::Char('n') | KeyCode::Char(']') | KeyCode::Char('J') => {
                    state.next_hunk();
                }
                KeyCode::Char('p') | KeyCode::Char('[') | KeyCode::Char('K') => {
                    state.prev_hunk();
                }
                KeyCode::Char('>') | KeyCode::Right => {
                    state.next_file();
                }
                KeyCode::Char('<') | KeyCode::Left => {
                    state.prev_file();
                }
                KeyCode::Char('j') | KeyCode::Down => {
                    state.scroll_down(1);
                }
                KeyCode::Char('k') | KeyCode::Up => {
                    state.scroll_up(1);
                }

                // View Modes & UI toggles
                KeyCode::Tab | KeyCode::Char('t') => {
                    state.toggle_view_mode();
                }
                KeyCode::Char('b') | KeyCode::Char('F') => {
                    state.toggle_sidebar();
                }
                KeyCode::Char('?') | KeyCode::Char('h') => {
                    state.toggle_help();
                }

                // Finish / Apply
                KeyCode::Enter => {
                    let staged_diff = state.get_staged_diff();
                    let files = state.get_staged_files();
                    return Ok(DiffViewResult::Applied { staged_diff, files });
                }

                // Quit / Cancel
                KeyCode::Esc | KeyCode::Char('q') => {
                    return Ok(DiffViewResult::Cancelled);
                }

                _ => {}
            }
        }
    }
}

// ============================================================================
// Unit Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_hunk_status_transitions() {
        let mut hunk = DiffHunk::new(
            0,
            1,
            3,
            1,
            4,
            "fn test()",
            vec![
                DiffLine::context(1, 1, "line 1"),
                DiffLine::deletion(2, "line 2 old"),
                DiffLine::addition(2, "line 2 new"),
                DiffLine::addition(3, "line 3 added"),
            ],
        );

        assert_eq!(hunk.status, HunkStatus::Pending);
        assert_eq!(hunk.additions_count, 2);
        assert_eq!(hunk.deletions_count, 1);

        hunk.stage();
        assert_eq!(hunk.status, HunkStatus::Staged);
        assert!(hunk.status.is_staged());

        hunk.reject();
        assert_eq!(hunk.status, HunkStatus::Rejected);
        assert!(hunk.status.is_rejected());

        hunk.reset();
        assert_eq!(hunk.status, HunkStatus::Pending);

        hunk.toggle();
        assert_eq!(hunk.status, HunkStatus::Staged);
        hunk.toggle();
        assert_eq!(hunk.status, HunkStatus::Rejected);
    }

    #[test]
    fn test_diff_state_from_strings() {
        let old = "fn main() {\n    println!(\"old\");\n}\n";
        let new = "fn main() {\n    println!(\"new\");\n    println!(\"extra\");\n}\n";

        let state = DiffViewState::from_strings(old, new, "src/main.rs", 3);
        assert_eq!(state.total_files(), 1);
        assert_eq!(state.total_hunks(), 1);
        assert_eq!(state.total_additions(), 2);
        assert_eq!(state.total_deletions(), 1);

        let file = state.active_file().unwrap();
        assert_eq!(file.path, "src/main.rs");
        assert_eq!(file.hunks.len(), 1);
    }

    #[test]
    fn test_stage_and_reconstruct_file() {
        let old = "line 1\nline 2\nline 3\n";
        let new = "line 1\nline 2 modified\nline 3\n";

        let mut state = DiffViewState::from_strings(old, new, "test.txt", 3);
        assert_eq!(state.total_staged_hunks(), 0);

        state.stage_current_hunk();
        assert_eq!(state.total_staged_hunks(), 1);

        let staged_files = state.get_staged_files();
        assert_eq!(staged_files.len(), 1);
        assert_eq!(staged_files[0].0, "test.txt");
        assert_eq!(staged_files[0].1, "line 1\nline 2 modified\nline 3\n");
    }

    #[test]
    fn test_reject_and_reconstruct_file() {
        let old = "line 1\nline 2\nline 3\n";
        let new = "line 1\nline 2 modified\nline 3\n";

        let mut state = DiffViewState::from_strings(old, new, "test.txt", 3);
        state.reject_current_hunk();
        assert_eq!(state.total_rejected_hunks(), 1);

        let staged_files = state.get_staged_files();
        assert_eq!(staged_files.len(), 1);
        assert_eq!(staged_files[0].1, "line 1\nline 2\nline 3\n");
    }

    #[test]
    fn test_syntax_tokenizer_rust() {
        let line = "pub async fn compute(val: u64) -> Result<String, Error> { // doc";
        let tokens = tokenize_line(line, SyntaxLanguage::Rust);

        assert!(!tokens.is_empty());
        assert!(tokens.iter().any(|t| t.kind == TokenKind::Keyword && t.text == "pub"));
        assert!(tokens.iter().any(|t| t.kind == TokenKind::Keyword && t.text == "async"));
        assert!(tokens.iter().any(|t| t.kind == TokenKind::Keyword && t.text == "fn"));
        assert!(tokens.iter().any(|t| t.kind == TokenKind::Function && t.text == "compute"));
        assert!(tokens.iter().any(|t| t.kind == TokenKind::Type && t.text == "u64"));
        assert!(tokens.iter().any(|t| t.kind == TokenKind::Type && t.text == "Result"));
        assert!(tokens.iter().any(|t| t.kind == TokenKind::Comment));
    }

    #[test]
    fn test_syntax_tokenizer_python() {
        let line = "def process_data(items: list) -> bool: # python function";
        let tokens = tokenize_line(line, SyntaxLanguage::Python);

        assert!(tokens.iter().any(|t| t.kind == TokenKind::Keyword && t.text == "def"));
        assert!(tokens.iter().any(|t| t.kind == TokenKind::Function && t.text == "process_data"));
        assert!(tokens.iter().any(|t| t.kind == TokenKind::Comment));
    }

    #[test]
    fn test_parse_unified_diff_into_state() {
        let diff = r#"--- a/src/foo.rs
+++ b/src/foo.rs
@@ -1,3 +1,4 @@
 fn foo() {
-    let x = 1;
+    let x = 2;
+    let y = 3;
 }
"#;
        let state = DiffViewState::from_unified_diff(diff).expect("Failed to parse diff");
        assert_eq!(state.total_files(), 1);
        assert_eq!(state.total_hunks(), 1);
        assert_eq!(state.total_additions(), 2);
        assert_eq!(state.total_deletions(), 1);
    }

    #[test]
    fn test_generate_staged_patch() {
        let diff = r#"--- a/src/foo.rs
+++ b/src/foo.rs
@@ -1,3 +1,4 @@
 fn foo() {
-    let x = 1;
+    let x = 2;
+    let y = 3;
 }
"#;
        let mut state = DiffViewState::from_unified_diff(diff).unwrap();
        assert_eq!(state.get_staged_diff(), "");

        state.stage_all();
        let staged_diff = state.get_staged_diff();
        assert!(staged_diff.contains("--- a/src/foo.rs"));
        assert!(staged_diff.contains("+++ b/src/foo.rs"));
        assert!(staged_diff.contains("+    let x = 2;"));
    }

    #[test]
    fn test_navigation_and_mode_toggles() {
        let mut state = DiffViewState::new(vec![
            DiffFile::new("file1.rs"),
            DiffFile::new("file2.rs"),
        ]);

        assert_eq!(state.active_file_idx, 0);
        state.next_file();
        assert_eq!(state.active_file_idx, 1);
        state.prev_file();
        assert_eq!(state.active_file_idx, 0);

        assert_eq!(state.view_mode, DiffViewMode::Unified);
        state.toggle_view_mode();
        assert_eq!(state.view_mode, DiffViewMode::SideBySide);

        assert!(state.show_sidebar);
        state.toggle_sidebar();
        assert!(!state.show_sidebar);

        assert!(!state.show_help);
        state.toggle_help();
        assert!(state.show_help);

        assert!(state.syntax_highlighting);
        state.toggle_syntax();
        assert!(!state.syntax_highlighting);
    }

    #[test]
    fn test_syntax_tokenizer_javascript_and_go() {
        let js = "const handle = async (req: Request): Promise<Response> => { return null; };";
        let js_tokens = tokenize_line(js, SyntaxLanguage::TypeScript);
        assert!(js_tokens.iter().any(|t| t.kind == TokenKind::Keyword && t.text == "const"));
        assert!(js_tokens.iter().any(|t| t.kind == TokenKind::Keyword && t.text == "async"));
        assert!(js_tokens.iter().any(|t| t.kind == TokenKind::Type && t.text == "Promise"));

        let go = "func main() { fmt.Println(\"Hello World\") }";
        let go_tokens = tokenize_line(go, SyntaxLanguage::Go);
        assert!(go_tokens.iter().any(|t| t.kind == TokenKind::Keyword && t.text == "func"));
        assert!(go_tokens.iter().any(|t| t.kind == TokenKind::StringLiteral));
    }

    #[test]
    fn test_syntax_tokenizer_shell_and_json() {
        let sh = "export FUSION_HOME=\"/tmp/fusion\" # env setup";
        let sh_tokens = tokenize_line(sh, SyntaxLanguage::Shell);
        assert!(sh_tokens.iter().any(|t| t.kind == TokenKind::Keyword && t.text == "export"));
        assert!(sh_tokens.iter().any(|t| t.kind == TokenKind::Comment));

        let json = "{\"name\": \"fusion\", \"version\": 100, \"active\": true}";
        let json_tokens = tokenize_line(json, SyntaxLanguage::Json);
        assert!(json_tokens.iter().any(|t| t.kind == TokenKind::StringLiteral));
        assert!(json_tokens.iter().any(|t| t.kind == TokenKind::NumberLiteral));
        assert!(json_tokens.iter().any(|t| t.kind == TokenKind::Keyword && t.text == "true"));
    }

    #[test]
    fn test_partial_staging_multiple_hunks() {
        let diff = r#"--- a/src/app.rs
+++ b/src/app.rs
@@ -1,5 +1,6 @@
 fn init() {
-    setup_logging();
+    setup_enhanced_logging();
+    setup_metrics();
 }
@@ -10,4 +11,5 @@
 fn shutdown() {
-    flush();
+    flush_async();
 }
"#;
        let mut state = DiffViewState::from_unified_diff(diff).unwrap();
        assert_eq!(state.total_hunks(), 2);
        assert_eq!(state.active_file().unwrap().status(), HunkStatus::Pending);

        // Stage first hunk only
        state.stage_current_hunk();
        assert_eq!(state.total_staged_hunks(), 1);
        assert_eq!(state.active_file().unwrap().status(), HunkStatus::PartiallyStaged);

        // Check generated staged patch has only first hunk
        let patch = state.get_staged_diff();
        assert!(patch.contains("setup_enhanced_logging"));
        assert!(!patch.contains("flush_async"));

        // Reject second hunk
        state.reject_current_hunk();
        assert_eq!(state.total_rejected_hunks(), 1);
        assert_eq!(state.active_file().unwrap().status(), HunkStatus::PartiallyStaged);
    }

    #[test]
    fn test_widget_rendering_buffer() {
        let diff = r#"--- a/src/foo.rs
+++ b/src/foo.rs
@@ -1,3 +1,4 @@
 fn foo() {
-    let x = 1;
+    let x = 2;
 }
"#;
        let mut state = DiffViewState::from_unified_diff(diff).unwrap();
        let widget = DiffViewerWidget::new(&state);
        let area = Rect::new(0, 0, 80, 24);
        let mut buffer = Buffer::empty(area);

        // Render unified view
        widget.render(area, &mut buffer);
        assert!(buffer.content().iter().any(|c| c.symbol() == "F" || c.symbol() == "f"));

        // Render side-by-side view
        state.toggle_view_mode();
        let widget_sbs = DiffViewerWidget::new(&state);
        let mut buffer_sbs = Buffer::empty(area);
        widget_sbs.render(area, &mut buffer_sbs);
    }
}
