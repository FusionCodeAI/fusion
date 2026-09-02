//! Pure-Rust syntax, brace, bracket, quotation, and indentation balance checker.
//!
//! Provides fast, zero-dependency validation for multiple programming and markup languages:
//! - **Brace & Delimiter Balancing**: Comprehensive tracking for `()`, `{}`, `[]`, angle brackets,
//!   handling raw strings, template literals, char literals, lifetimes, and nested comments.
//! - **Indentation Analysis**: Mixed tabs/spaces detection, inconsistent indentation step detection,
//!   and Python-specific block/indent stack verification (e.g. `IndentationError` detection).
//! - **Language-Specific Parsers**:
//!   - Rust (lifetimes vs chars, raw strings `r#"..."#`, nested `/* /* */ */` block comments)
//!   - Python (triple quotes, f-strings, colon block indent/unindent verification)
//!   - JavaScript / TypeScript / JSX / TSX (template literals `` `...${...}...` ``, regex literals)
//!   - JSON (strict validation with line/column diagnostic and trailing comma/single quote detection)
//!   - HTML / XML (tag hierarchy balancing, void elements, self-closing tags)
//!   - TOML, YAML, Shell/Bash, SQL, CSS/SCSS, C/C++, Java, Go, Ruby, Lua.
//! - **Rich Diagnostics**: Rustc-style visual error formatting with source line context and column carets.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashSet;
use std::path::Path;

use crate::tools::file::resolve_path;
use crate::tools::types::{Tool, ToolContext};

// ---------------------------------------------------------------------------
// Types and Data Models
// ---------------------------------------------------------------------------

/// Severity level of a syntax validation diagnostic.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum IssueSeverity {
    Error,
    Warning,
    Info,
}

impl std::fmt::Display for IssueSeverity {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            IssueSeverity::Error => write!(f, "error"),
            IssueSeverity::Warning => write!(f, "warning"),
            IssueSeverity::Info => write!(f, "info"),
        }
    }
}

/// A single diagnostic issue detected during syntax validation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SyntaxIssue {
    pub severity: IssueSeverity,
    pub rule: String,
    pub message: String,
    pub line: usize,
    pub column: usize,
    pub end_line: Option<usize>,
    pub end_column: Option<usize>,
    pub suggestion: Option<String>,
}

impl SyntaxIssue {
    pub fn error(rule: impl Into<String>, message: impl Into<String>, line: usize, column: usize) -> Self {
        Self {
            severity: IssueSeverity::Error,
            rule: rule.into(),
            message: message.into(),
            line,
            column,
            end_line: None,
            end_column: None,
            suggestion: None,
        }
    }

    pub fn warning(rule: impl Into<String>, message: impl Into<String>, line: usize, column: usize) -> Self {
        Self {
            severity: IssueSeverity::Warning,
            rule: rule.into(),
            message: message.into(),
            line,
            column,
            end_line: None,
            end_column: None,
            suggestion: None,
        }
    }

    pub fn with_suggestion(mut self, suggestion: impl Into<String>) -> Self {
        self.suggestion = Some(suggestion.into());
        self
    }
}

/// Overall report summarizing the results of syntax validation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SyntaxValidationReport {
    pub valid: bool,
    pub language: String,
    pub file_path: Option<String>,
    pub total_lines: usize,
    pub total_chars: usize,
    pub error_count: usize,
    pub warning_count: usize,
    pub issues: Vec<SyntaxIssue>,
}

impl SyntaxValidationReport {
    pub fn new(language: String, file_path: Option<String>, content: &str, issues: Vec<SyntaxIssue>) -> Self {
        let error_count = issues.iter().filter(|i| i.severity == IssueSeverity::Error).count();
        let warning_count = issues.iter().filter(|i| i.severity == IssueSeverity::Warning).count();
        let total_lines = content.lines().count().max(if content.is_empty() { 0 } else { 1 });
        let total_chars = content.chars().count();

        Self {
            valid: error_count == 0,
            language,
            file_path,
            total_lines,
            total_chars,
            error_count,
            warning_count,
            issues,
        }
    }
}

/// Options controlling which checks are executed.
#[derive(Debug, Clone)]
pub struct SyntaxCheckOptions {
    pub check_indentation: bool,
    pub check_brackets: bool,
    pub check_quotes: bool,
    pub language_override: Option<String>,
}

impl Default for SyntaxCheckOptions {
    fn default() -> Self {
        Self {
            check_indentation: true,
            check_brackets: true,
            check_quotes: true,
            language_override: None,
        }
    }
}

// ---------------------------------------------------------------------------
// Supported Languages & Detection
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SupportedLanguage {
    Rust,
    Python,
    JavaScript,
    TypeScript,
    Jsx,
    Tsx,
    Json,
    Toml,
    Yaml,
    Html,
    Xml,
    Css,
    Scss,
    C,
    Cpp,
    CSharp,
    Java,
    Go,
    Ruby,
    Lua,
    Shell,
    Sql,
    Markdown,
    Unknown,
}

impl SupportedLanguage {
    pub fn as_str(&self) -> &'static str {
        match self {
            SupportedLanguage::Rust => "rust",
            SupportedLanguage::Python => "python",
            SupportedLanguage::JavaScript => "javascript",
            SupportedLanguage::TypeScript => "typescript",
            SupportedLanguage::Jsx => "jsx",
            SupportedLanguage::Tsx => "tsx",
            SupportedLanguage::Json => "json",
            SupportedLanguage::Toml => "toml",
            SupportedLanguage::Yaml => "yaml",
            SupportedLanguage::Html => "html",
            SupportedLanguage::Xml => "xml",
            SupportedLanguage::Css => "css",
            SupportedLanguage::Scss => "scss",
            SupportedLanguage::C => "c",
            SupportedLanguage::Cpp => "cpp",
            SupportedLanguage::CSharp => "csharp",
            SupportedLanguage::Java => "java",
            SupportedLanguage::Go => "go",
            SupportedLanguage::Ruby => "ruby",
            SupportedLanguage::Lua => "lua",
            SupportedLanguage::Shell => "shell",
            SupportedLanguage::Sql => "sql",
            SupportedLanguage::Markdown => "markdown",
            SupportedLanguage::Unknown => "unknown",
        }
    }

    pub fn display_name(&self) -> &'static str {
        match self {
            SupportedLanguage::Rust => "Rust",
            SupportedLanguage::Python => "Python",
            SupportedLanguage::JavaScript => "JavaScript",
            SupportedLanguage::TypeScript => "TypeScript",
            SupportedLanguage::Jsx => "JSX",
            SupportedLanguage::Tsx => "TSX",
            SupportedLanguage::Json => "JSON",
            SupportedLanguage::Toml => "TOML",
            SupportedLanguage::Yaml => "YAML",
            SupportedLanguage::Html => "HTML",
            SupportedLanguage::Xml => "XML",
            SupportedLanguage::Css => "CSS",
            SupportedLanguage::Scss => "SCSS",
            SupportedLanguage::C => "C",
            SupportedLanguage::Cpp => "C++",
            SupportedLanguage::CSharp => "C#",
            SupportedLanguage::Java => "Java",
            SupportedLanguage::Go => "Go",
            SupportedLanguage::Ruby => "Ruby",
            SupportedLanguage::Lua => "Lua",
            SupportedLanguage::Shell => "Shell/Bash",
            SupportedLanguage::Sql => "SQL",
            SupportedLanguage::Markdown => "Markdown",
            SupportedLanguage::Unknown => "Plain Text",
        }
    }

    pub fn from_str_hint(s: &str) -> Self {
        let clean = s.trim().to_lowercase();
        match clean.as_str() {
            "rs" | "rust" => SupportedLanguage::Rust,
            "py" | "python" | "pyw" | "pyi" => SupportedLanguage::Python,
            "js" | "javascript" | "mjs" | "cjs" => SupportedLanguage::JavaScript,
            "ts" | "typescript" | "mts" | "cts" => SupportedLanguage::TypeScript,
            "jsx" => SupportedLanguage::Jsx,
            "tsx" => SupportedLanguage::Tsx,
            "json" | "jsonc" | "json5" => SupportedLanguage::Json,
            "toml" => SupportedLanguage::Toml,
            "yaml" | "yml" => SupportedLanguage::Yaml,
            "html" | "htm" | "xhtml" => SupportedLanguage::Html,
            "xml" | "svg" | "plist" | "xaml" => SupportedLanguage::Xml,
            "css" => SupportedLanguage::Css,
            "scss" | "sass" | "less" => SupportedLanguage::Scss,
            "c" | "h" => SupportedLanguage::C,
            "cpp" | "cxx" | "cc" | "hpp" | "hxx" | "hh" | "c++" => SupportedLanguage::Cpp,
            "cs" | "csharp" => SupportedLanguage::CSharp,
            "java" | "class" => SupportedLanguage::Java,
            "go" | "golang" => SupportedLanguage::Go,
            "rb" | "ruby" => SupportedLanguage::Ruby,
            "lua" => SupportedLanguage::Lua,
            "sh" | "bash" | "zsh" | "shell" => SupportedLanguage::Shell,
            "sql" => SupportedLanguage::Sql,
            "md" | "markdown" => SupportedLanguage::Markdown,
            _ => SupportedLanguage::Unknown,
        }
    }
}

/// Detect language from file path, explicit hint, or content inspection.
pub fn detect_language(path: Option<&Path>, content: &str, hint: Option<&str>) -> SupportedLanguage {
    if let Some(h) = hint {
        if !h.trim().is_empty() && h != "auto" {
            let parsed = SupportedLanguage::from_str_hint(h);
            if parsed != SupportedLanguage::Unknown {
                return parsed;
            }
        }
    }

    if let Some(p) = path {
        if let Some(file_name) = p.file_name().and_then(|f| f.to_str()) {
            let lower_name = file_name.to_lowercase();
            if lower_name == "cargo.toml" || lower_name == "pyproject.toml" {
                return SupportedLanguage::Toml;
            }
            if lower_name == "package.json" || lower_name == "tsconfig.json" {
                return SupportedLanguage::Json;
            }
            if lower_name == "dockerfile" || lower_name == "makefile" {
                return SupportedLanguage::Shell;
            }
        }

        if let Some(ext) = p.extension().and_then(|e| e.to_str()) {
            let parsed = SupportedLanguage::from_str_hint(ext);
            if parsed != SupportedLanguage::Unknown {
                return parsed;
            }
        }
    }

    // Inspect content heuristics (shebang / tags / json markers)
    let trimmed = content.trim_start();
    if trimmed.starts_with("#!/") {
        let first_line = trimmed.lines().next().unwrap_or("");
        if first_line.contains("python") {
            return SupportedLanguage::Python;
        }
        if first_line.contains("node") || first_line.contains("deno") || first_line.contains("bun") {
            return SupportedLanguage::JavaScript;
        }
        if first_line.contains("sh") || first_line.contains("bash") || first_line.contains("zsh") {
            return SupportedLanguage::Shell;
        }
        if first_line.contains("ruby") {
            return SupportedLanguage::Ruby;
        }
    }

    if trimmed.starts_with("<?xml") || trimmed.starts_with("<svg") {
        return SupportedLanguage::Xml;
    }
    if trimmed.starts_with("<!DOCTYPE html") || trimmed.starts_with("<html") {
        return SupportedLanguage::Html;
    }
    if (trimmed.starts_with('{') && trimmed.ends_with('}')) || (trimmed.starts_with('[') && trimmed.ends_with(']')) {
        if serde_json::from_str::<serde_json::Value>(content).is_ok() {
            return SupportedLanguage::Json;
        }
    }

    SupportedLanguage::Unknown
}

// ---------------------------------------------------------------------------
// Delimiter / Bracket / Quote Balance Engine
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
struct OpenDelimiter {
    delimiter: char,
    expected_close: char,
    line: usize,
    column: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum JsState {
    Normal,
    InTemplate,
    InTemplateExpr(usize), // nested brace counter inside ${ ... }
}

/// Validates bracket, brace, parenthesis, and string quotation balance.
pub fn check_delimiters(
    content: &str,
    lang: SupportedLanguage,
    options: &SyntaxCheckOptions,
    issues: &mut Vec<SyntaxIssue>,
) {
    if !options.check_brackets && !options.check_quotes {
        return;
    }

    let chars: Vec<char> = content.chars().collect();
    let len = chars.len();
    let mut pos = 0;
    let mut line = 1;
    let mut col = 1;

    let mut delimiter_stack: Vec<OpenDelimiter> = Vec::new();
    let mut js_states: Vec<JsState> = Vec::new();

    while pos < len {
        let ch = chars[pos];

        // -------------------------------------------------------------------
        // 1. Newline tracking
        // -------------------------------------------------------------------
        if ch == '\n' {
            line += 1;
            col = 1;
            pos += 1;
            continue;
        }

        // -------------------------------------------------------------------
        // 2. Comments
        // -------------------------------------------------------------------
        // Single-line // comments (C-family, Rust, JS, TS, Go, Java, C#, CSS...)
        if ch == '/' && pos + 1 < len && chars[pos + 1] == '/' {
            if matches!(
                lang,
                SupportedLanguage::Rust
                    | SupportedLanguage::JavaScript
                    | SupportedLanguage::TypeScript
                    | SupportedLanguage::Jsx
                    | SupportedLanguage::Tsx
                    | SupportedLanguage::C
                    | SupportedLanguage::Cpp
                    | SupportedLanguage::CSharp
                    | SupportedLanguage::Java
                    | SupportedLanguage::Go
                    | SupportedLanguage::Css
                    | SupportedLanguage::Scss
                    | SupportedLanguage::Unknown
            ) {
                while pos < len && chars[pos] != '\n' {
                    pos += 1;
                    col += 1;
                }
                continue;
            }
        }

        // Block /* ... */ comments
        if ch == '/' && pos + 1 < len && chars[pos + 1] == '*' {
            if matches!(
                lang,
                SupportedLanguage::Rust
                    | SupportedLanguage::JavaScript
                    | SupportedLanguage::TypeScript
                    | SupportedLanguage::Jsx
                    | SupportedLanguage::Tsx
                    | SupportedLanguage::C
                    | SupportedLanguage::Cpp
                    | SupportedLanguage::CSharp
                    | SupportedLanguage::Java
                    | SupportedLanguage::Go
                    | SupportedLanguage::Css
                    | SupportedLanguage::Scss
                    | SupportedLanguage::Sql
                    | SupportedLanguage::Unknown
            ) {
                let start_line = line;
                let start_col = col;
                pos += 2;
                col += 2;

                if lang == SupportedLanguage::Rust {
                    // Rust supports nested block comments: /* /* */ */
                    let mut depth = 1;
                    while pos < len && depth > 0 {
                        if chars[pos] == '\n' {
                            line += 1;
                            col = 1;
                            pos += 1;
                        } else if chars[pos] == '/' && pos + 1 < len && chars[pos + 1] == '*' {
                            depth += 1;
                            pos += 2;
                            col += 2;
                        } else if chars[pos] == '*' && pos + 1 < len && chars[pos + 1] == '/' {
                            depth -= 1;
                            pos += 2;
                            col += 2;
                        } else {
                            pos += 1;
                            col += 1;
                        }
                    }
                    if depth > 0 && options.check_quotes {
                        issues.push(
                            SyntaxIssue::error(
                                "unclosed-comment",
                                "Unclosed block comment '/*' (reached end of file without matching '*/')",
                                start_line,
                                start_col,
                            )
                            .with_suggestion("Add '*/' to close the block comment"),
                        );
                    }
                } else {
                    // Non-nested block comments
                    let mut closed = false;
                    while pos < len {
                        if chars[pos] == '\n' {
                            line += 1;
                            col = 1;
                            pos += 1;
                        } else if chars[pos] == '*' && pos + 1 < len && chars[pos + 1] == '/' {
                            pos += 2;
                            col += 2;
                            closed = true;
                            break;
                        } else {
                            pos += 1;
                            col += 1;
                        }
                    }
                    if !closed && options.check_quotes {
                        issues.push(
                            SyntaxIssue::error(
                                "unclosed-comment",
                                "Unclosed block comment '/*' (reached end of file without matching '*/')",
                                start_line,
                                start_col,
                            )
                            .with_suggestion("Add '*/' to close the block comment"),
                        );
                    }
                }
                continue;
            }
        }

        // Single-line # comments (Python, Ruby, Shell, YAML, TOML)
        if ch == '#' && matches!(
            lang,
            SupportedLanguage::Python
                | SupportedLanguage::Ruby
                | SupportedLanguage::Shell
                | SupportedLanguage::Yaml
                | SupportedLanguage::Toml
        ) {
            while pos < len && chars[pos] != '\n' {
                pos += 1;
                col += 1;
            }
            continue;
        }

        // Single-line -- comments (SQL, Lua)
        if ch == '-' && pos + 1 < len && chars[pos + 1] == '-' && matches!(lang, SupportedLanguage::Sql | SupportedLanguage::Lua) {
            // Lua block comment check: --[[ ... ]]
            if lang == SupportedLanguage::Lua && pos + 3 < len && chars[pos + 2] == '[' && chars[pos + 3] == '[' {
                let start_line = line;
                let start_col = col;
                pos += 4;
                col += 4;
                let mut closed = false;
                while pos < len {
                    if chars[pos] == '\n' {
                        line += 1;
                        col = 1;
                        pos += 1;
                    } else if chars[pos] == ']' && pos + 1 < len && chars[pos + 1] == ']' {
                        pos += 2;
                        col += 2;
                        closed = true;
                        break;
                    } else {
                        pos += 1;
                        col += 1;
                    }
                }
                if !closed && options.check_quotes {
                    issues.push(SyntaxIssue::error(
                        "unclosed-comment",
                        "Unclosed Lua block comment '--[['",
                        start_line,
                        start_col,
                    ));
                }
                continue;
            }

            while pos < len && chars[pos] != '\n' {
                pos += 1;
                col += 1;
            }
            continue;
        }

        // HTML/XML comments: <!-- ... -->
        if ch == '<' && pos + 3 < len && chars[pos + 1] == '!' && chars[pos + 2] == '-' && chars[pos + 3] == '-' {
            let start_line = line;
            let start_col = col;
            pos += 4;
            col += 4;
            let mut closed = false;
            while pos < len {
                if chars[pos] == '\n' {
                    line += 1;
                    col = 1;
                    pos += 1;
                } else if chars[pos] == '-' && pos + 2 < len && chars[pos + 1] == '-' && chars[pos + 2] == '>' {
                    pos += 3;
                    col += 3;
                    closed = true;
                    break;
                } else {
                    pos += 1;
                    col += 1;
                }
            }
            if !closed && options.check_quotes {
                issues.push(SyntaxIssue::error(
                    "unclosed-comment",
                    "Unclosed HTML comment '<!--' (missing '-->')",
                    start_line,
                    start_col,
                ));
            }
            continue;
        }

        // -------------------------------------------------------------------
        // 3. String Literals & Quotations
        // -------------------------------------------------------------------
        // Rust Raw String Literals: r"...", r#"..."#, r##"..."##, br#"..."#, cr#"..."#
        if lang == SupportedLanguage::Rust {
            let mut is_raw = false;
            let mut raw_start_offset = 0;

            if ch == 'r' {
                is_raw = true;
                raw_start_offset = 1;
            } else if (ch == 'b' || ch == 'c') && pos + 1 < len && chars[pos + 1] == 'r' {
                is_raw = true;
                raw_start_offset = 2;
            }

            if is_raw {
                let mut p = pos + raw_start_offset;
                let mut hash_count = 0;
                while p < len && chars[p] == '#' {
                    hash_count += 1;
                    p += 1;
                }
                if p < len && chars[p] == '"' {
                    // Valid raw string start
                    let start_line = line;
                    let start_col = col;
                    let prefix_len = raw_start_offset + hash_count + 1;
                    pos += prefix_len;
                    col += prefix_len;

                    let mut matched = false;
                    while pos < len {
                        if chars[pos] == '\n' {
                            line += 1;
                            col = 1;
                            pos += 1;
                        } else if chars[pos] == '"' {
                            // Check if followed by hash_count '#'
                            let mut h = 0;
                            while pos + 1 + h < len && chars[pos + 1 + h] == '#' && h < hash_count {
                                h += 1;
                            }
                            if h == hash_count {
                                pos += 1 + hash_count;
                                col += 1 + hash_count;
                                matched = true;
                                break;
                            } else {
                                pos += 1;
                                col += 1;
                            }
                        } else {
                            pos += 1;
                            col += 1;
                        }
                    }

                    if !matched && options.check_quotes {
                        issues.push(SyntaxIssue::error(
                            "unclosed-raw-string",
                            format!(
                                "Unclosed Rust raw string literal (missing '\"{}')",
                                "#".repeat(hash_count)
                            ),
                            start_line,
                            start_col,
                        ));
                    }
                    continue;
                }
            }
        }

        // Python Triple Quotes: """ or ''' (with optional prefix r, u, f, b, fr, rf)
        if lang == SupportedLanguage::Python {
            let quote_char = if ch == '"' && pos + 2 < len && chars[pos + 1] == '"' && chars[pos + 2] == '"' {
                Some('"')
            } else if ch == '\'' && pos + 2 < len && chars[pos + 1] == '\'' && chars[pos + 2] == '\'' {
                Some('\'')
            } else {
                None
            };

            if let Some(qc) = quote_char {
                let start_line = line;
                let start_col = col;
                pos += 3;
                col += 3;

                let mut matched = false;
                while pos < len {
                    if chars[pos] == '\n' {
                        line += 1;
                        col = 1;
                        pos += 1;
                    } else if chars[pos] == '\\' && pos + 1 < len {
                        if chars[pos + 1] == '\n' {
                            line += 1;
                            col = 1;
                        } else {
                            col += 2;
                        }
                        pos += 2;
                    } else if chars[pos] == qc && pos + 2 < len && chars[pos + 1] == qc && chars[pos + 2] == qc {
                        pos += 3;
                        col += 3;
                        matched = true;
                        break;
                    } else {
                        pos += 1;
                        col += 1;
                    }
                }

                if !matched && options.check_quotes {
                    issues.push(SyntaxIssue::error(
                        "unclosed-triple-quote",
                        format!("Unclosed Python triple-quoted string ({}{}{})", qc, qc, qc),
                        start_line,
                        start_col,
                    ));
                }
                continue;
            }
        }

        // JavaScript / TypeScript Template Literals: `...` and `${...}`
        if (lang == SupportedLanguage::JavaScript
            || lang == SupportedLanguage::TypeScript
            || lang == SupportedLanguage::Jsx
            || lang == SupportedLanguage::Tsx)
            && ch == '`'
        {
            let start_line = line;
            let start_col = col;
            pos += 1;
            col += 1;
            js_states.push(JsState::InTemplate);

            let mut matched = false;
            while pos < len {
                if chars[pos] == '\n' {
                    line += 1;
                    col = 1;
                    pos += 1;
                } else if chars[pos] == '\\' && pos + 1 < len {
                    if chars[pos + 1] == '\n' {
                        line += 1;
                        col = 1;
                    } else {
                        col += 2;
                    }
                    pos += 2;
                } else if chars[pos] == '`' {
                    pos += 1;
                    col += 1;
                    js_states.pop();
                    matched = true;
                    break;
                } else if chars[pos] == '$' && pos + 1 < len && chars[pos + 1] == '{' {
                    // Template expression interpolation: `${ ... }`
                    pos += 2;
                    col += 2;
                    delimiter_stack.push(OpenDelimiter {
                        delimiter: '{',
                        expected_close: '}',
                        line,
                        column: col - 2,
                    });
                    js_states.push(JsState::InTemplateExpr(1));
                    matched = true; // Handled as active syntax stream
                    break;
                } else {
                    pos += 1;
                    col += 1;
                }
            }

            if !matched && options.check_quotes {
                issues.push(SyntaxIssue::error(
                    "unclosed-template-literal",
                    "Unclosed JavaScript template literal (missing '`')",
                    start_line,
                    start_col,
                ));
            }
            continue;
        }

        // Rust Lifetime vs Character Literal: 'a' vs 'a, 'static, 'label:
        if lang == SupportedLanguage::Rust && ch == '\'' {
            let start_line = line;
            let start_col = col;

            // Check if this is a lifetime or label (e.g. 'a, 'static, 'de)
            // Lifetimes start with ' followed by ident, but NOT closed with '
            let mut p = pos + 1;
            let mut is_ident_start = false;
            if p < len && (chars[p].is_alphabetic() || chars[p] == '_') {
                is_ident_start = true;
                p += 1;
                while p < len && (chars[p].is_alphanumeric() || chars[p] == '_') {
                    p += 1;
                }
            }

            if is_ident_start && (p >= len || chars[p] != '\'') {
                // This is a lifetime or label, e.g. `'a`, `'static`, `'loop:`
                let ident_len = p - pos;
                pos += ident_len;
                col += ident_len;
                continue;
            }

            // Otherwise, it is a char literal 'c', '\n', '\'', '\x7f', '\u{1234}'
            pos += 1;
            col += 1;
            let mut closed = false;

            if pos < len && chars[pos] == '\\' {
                pos += 1;
                col += 1;
                if pos < len && chars[pos] == 'u' && pos + 1 < len && chars[pos + 1] == '{' {
                    // Unicode escape '\u{1f600}'
                    pos += 2;
                    col += 2;
                    while pos < len && chars[pos] != '}' && chars[pos] != '\n' {
                        pos += 1;
                        col += 1;
                    }
                    if pos < len && chars[pos] == '}' {
                        pos += 1;
                        col += 1;
                    }
                } else if pos < len {
                    pos += 1;
                    col += 1;
                }
            } else if pos < len && chars[pos] != '\'' && chars[pos] != '\n' {
                pos += 1;
                col += 1;
            }

            if pos < len && chars[pos] == '\'' {
                pos += 1;
                col += 1;
                closed = true;
            }

            if !closed && options.check_quotes {
                issues.push(SyntaxIssue::error(
                    "unclosed-char-literal",
                    "Unclosed character literal (missing closing \"'\")",
                    start_line,
                    start_col,
                ));
            }
            continue;
        }

        // Standard Double Quote "..." or Single Quote '...'
        if ch == '"' || ch == '\'' {
            let quote_char = ch;
            let start_line = line;
            let start_col = col;
            pos += 1;
            col += 1;

            let mut matched = false;
            let allows_multiline = matches!(
                lang,
                SupportedLanguage::Html
                    | SupportedLanguage::Xml
                    | SupportedLanguage::Css
                    | SupportedLanguage::Scss
                    | SupportedLanguage::Sql
            );

            while pos < len {
                let curr = chars[pos];
                if curr == '\n' {
                    if !allows_multiline {
                        // In most languages, single/double quotes cannot span unescaped newlines
                        if options.check_quotes {
                            issues.push(SyntaxIssue::error(
                                "unclosed-string-literal",
                                format!("Unclosed string literal (line break without closing '{quote_char}')"),
                                start_line,
                                start_col,
                            ));
                        }
                        matched = true; // Avoid runaway loop
                        break;
                    }
                    line += 1;
                    col = 1;
                    pos += 1;
                } else if curr == '\\' && pos + 1 < len {
                    if chars[pos + 1] == '\n' {
                        line += 1;
                        col = 1;
                    } else {
                        col += 2;
                    }
                    pos += 2;
                } else if curr == quote_char {
                    // SQL allows '' escape for single quote
                    if lang == SupportedLanguage::Sql && quote_char == '\'' && pos + 1 < len && chars[pos + 1] == '\'' {
                        pos += 2;
                        col += 2;
                        continue;
                    }
                    pos += 1;
                    col += 1;
                    matched = true;
                    break;
                } else {
                    pos += 1;
                    col += 1;
                }
            }

            if !matched && options.check_quotes {
                issues.push(SyntaxIssue::error(
                    "unclosed-string-literal",
                    format!("Unclosed string literal '{quote_char}' (reached end of file)"),
                    start_line,
                    start_col,
                ));
            }
            continue;
        }

        // -------------------------------------------------------------------
        // 4. Bracket / Delimiter Balance Checking
        // -------------------------------------------------------------------
        if options.check_brackets {
            match ch {
                '(' => {
                    delimiter_stack.push(OpenDelimiter {
                        delimiter: '(',
                        expected_close: ')',
                        line,
                        column: col,
                    });
                }
                '[' => {
                    delimiter_stack.push(OpenDelimiter {
                        delimiter: '[',
                        expected_close: ']',
                        line,
                        column: col,
                    });
                }
                '{' => {
                    delimiter_stack.push(OpenDelimiter {
                        delimiter: '{',
                        expected_close: '}',
                        line,
                        column: col,
                    });
                }
                ')' | ']' | '}' => {
                    if let Some(top) = delimiter_stack.pop() {
                        if top.expected_close != ch {
                            issues.push(
                                SyntaxIssue::error(
                                    "mismatched-delimiter",
                                    format!(
                                        "Mismatched closing delimiter '{}' at line {}:{}; expected '{}' to match '{}' opened at line {}:{}",
                                        ch, line, col, top.expected_close, top.delimiter, top.line, top.column
                                    ),
                                    line,
                                    col,
                                )
                                .with_suggestion(format!("Replace '{}' with '{}' or fix enclosing scope", ch, top.expected_close)),
                            );
                        } else {
                            // If closing a template expression in JS/TS:
                            if let Some(JsState::InTemplateExpr(cnt)) = js_states.last_mut() {
                                *cnt = cnt.saturating_sub(1);
                                if *cnt == 0 {
                                    js_states.pop();
                                }
                            }
                        }
                    } else {
                        issues.push(
                            SyntaxIssue::error(
                                "unmatched-delimiter",
                                format!(
                                    "Unmatched closing delimiter '{}' at line {}:{} (no matching opening delimiter)",
                                    ch, line, col
                                ),
                                line,
                                col,
                            )
                            .with_suggestion(format!("Remove extra '{}' or add missing opening delimiter", ch)),
                        );
                    }
                }
                _ => {}
            }
        }

        pos += 1;
        col += 1;
    }

    // -----------------------------------------------------------------------
    // 5. Check for unclosed delimiters at EOF
    // -----------------------------------------------------------------------
    if options.check_brackets {
        for open in delimiter_stack {
            issues.push(
                SyntaxIssue::error(
                    "unclosed-delimiter",
                    format!(
                        "Unclosed delimiter '{}' opened at line {}:{} (missing '{}' at end of file)",
                        open.delimiter, open.line, open.column, open.expected_close
                    ),
                    open.line,
                    open.column,
                )
                .with_suggestion(format!("Add '{}' before end of file to close '{}'", open.expected_close, open.delimiter)),
            );
        }
    }
}

// ---------------------------------------------------------------------------
// Indentation & Layout Engine
// ---------------------------------------------------------------------------

/// Validates indentation consistency, mixed tabs/spaces, and Python indentation stacks.
pub fn check_indentation(
    content: &str,
    lang: SupportedLanguage,
    options: &SyntaxCheckOptions,
    issues: &mut Vec<SyntaxIssue>,
) {
    if !options.check_indentation {
        return;
    }

    let mut py_indent_stack: Vec<usize> = vec![0];
    let mut py_paren_depth: usize = 0;
    let mut prev_non_empty_line: Option<(usize, String, usize)> = None; // (line_idx, trimmed_content, indent_size)

    let mut has_tab_indent = false;
    let mut has_space_indent = false;

    for (line_idx_0, raw_line) in content.lines().enumerate() {
        let line_num = line_idx_0 + 1;
        let trimmed_line = raw_line.trim();

        // Count leading whitespace
        let leading_spaces = raw_line.chars().take_while(|&c| c == ' ').count();
        let leading_tabs = raw_line.chars().take_while(|&c| c == '\t').count();
        let leading_ws: String = raw_line.chars().take_while(|c| c.is_whitespace()).collect();

        // 1. Mixed tabs and spaces on the same indentation prefix
        if leading_ws.contains(' ') && leading_ws.contains('\t') {
            issues.push(
                SyntaxIssue::warning(
                    "mixed-indentation",
                    format!("Line {line_num} mixes tabs and spaces in leading indentation"),
                    line_num,
                    1,
                )
                .with_suggestion("Convert indentation to consistent all-spaces or all-tabs"),
            );
        }

        if !trimmed_line.is_empty() {
            if leading_tabs > 0 {
                has_tab_indent = true;
            }
            if leading_spaces > 0 {
                has_space_indent = true;
            }

            // YAML bans tab indentation entirely
            if lang == SupportedLanguage::Yaml && leading_ws.contains('\t') {
                issues.push(
                    SyntaxIssue::error(
                        "yaml-tab-indentation",
                        format!("YAML forbids tabs for indentation at line {line_num}"),
                        line_num,
                        1,
                    )
                    .with_suggestion("Replace tabs with 2 or 4 spaces"),
                );
            }
        }

        // 2. Python specific indentation verification
        if lang == SupportedLanguage::Python {
            // Update paren/bracket depth to ignore continuation line indents inside expressions
            for c in raw_line.chars() {
                match c {
                    '(' | '[' | '{' => py_paren_depth += 1,
                    ')' | ']' | '}' => py_paren_depth = py_paren_depth.saturating_sub(1),
                    _ => {}
                }
            }

            // Skip indentation check if inside open parentheses/brackets or if line is comment/empty
            if py_paren_depth == 0 && !trimmed_line.is_empty() && !trimmed_line.starts_with('#') {
                let current_indent = leading_spaces + (leading_tabs * 4);

                // Check if previous line opened a new block with ':'
                if let Some((prev_line_num, prev_trimmed, prev_indent)) = &prev_non_empty_line {
                    let prev_ends_with_colon = prev_trimmed.ends_with(':')
                        || prev_trimmed.split('#').next().unwrap_or("").trim_end().ends_with(':');

                    if prev_ends_with_colon && current_indent <= *prev_indent {
                        issues.push(
                            SyntaxIssue::error(
                                "expected-indented-block",
                                format!(
                                    "Expected an indented block at line {line_num} after ':' at line {prev_line_num}"
                                ),
                                line_num,
                                current_indent + 1,
                            )
                            .with_suggestion(format!("Indent line {line_num} with {} spaces", prev_indent + 4)),
                        );
                    }
                }

                let current_top = *py_indent_stack.last().unwrap_or(&0);
                if current_indent > current_top {
                    // Increased indent
                    if let Some((_, prev_trimmed, _)) = &prev_non_empty_line {
                        let prev_ends_with_colon = prev_trimmed.ends_with(':')
                            || prev_trimmed.split('#').next().unwrap_or("").trim_end().ends_with(':');

                        if !prev_ends_with_colon {
                            issues.push(
                                SyntaxIssue::warning(
                                    "unexpected-indent",
                                    format!("Unexpected indentation at line {line_num} without prior ':' block opener"),
                                    line_num,
                                    current_indent + 1,
                                ),
                            );
                        }
                    }
                    py_indent_stack.push(current_indent);
                } else if current_indent < current_top {
                    // Dedent - must match an existing stack level
                    while let Some(&top) = py_indent_stack.last() {
                        if top > current_indent {
                            py_indent_stack.pop();
                        } else {
                            break;
                        }
                    }

                    if py_indent_stack.last() != Some(&current_indent) {
                        issues.push(
                            SyntaxIssue::error(
                                "unindent-mismatch",
                                format!(
                                    "Unindent at line {line_num} ({current_indent} spaces) does not match any outer indentation level (valid levels: {:?})",
                                    py_indent_stack
                                ),
                                line_num,
                                current_indent + 1,
                            )
                            .with_suggestion("Align indentation with the matching outer block"),
                        );
                    }
                }

                prev_non_empty_line = Some((line_num, trimmed_line.to_string(), current_indent));
            }
        }
    }

    // 3. Whole-file inconsistent indentation style (mixing tabs and spaces across different lines)
    if has_tab_indent && has_space_indent && lang != SupportedLanguage::Unknown {
        issues.push(
            SyntaxIssue::info(
                "inconsistent-indent-style",
                "File contains inconsistent indentation style (some lines use tabs, others use spaces)",
                1,
                1,
            )
            .with_suggestion("Adopt a uniform indentation style throughout the entire file"),
        );
    }
}

impl SyntaxIssue {
    pub fn info(rule: impl Into<String>, message: impl Into<String>, line: usize, column: usize) -> Self {
        Self {
            severity: IssueSeverity::Info,
            rule: rule.into(),
            message: message.into(),
            line,
            column,
            end_line: None,
            end_column: None,
            suggestion: None,
        }
    }
}

// ---------------------------------------------------------------------------
// Specialized Parsers: JSON, HTML/XML
// ---------------------------------------------------------------------------

/// Validates strict JSON syntax, reporting exact line, column, and hints.
pub fn check_json(content: &str, issues: &mut Vec<SyntaxIssue>) {
    if content.trim().is_empty() {
        issues.push(SyntaxIssue::error("empty-json", "JSON document is empty", 1, 1));
        return;
    }

    if let Err(e) = serde_json::from_str::<Value>(content) {
        let err_line = e.line();
        let err_col = e.column();
        let err_str = e.to_string();

        let mut suggestion = None;
        if err_str.contains("trailing comma") || err_str.contains("expected value") {
            suggestion = Some("Remove trailing comma before '}' or ']'".to_string());
        } else if err_str.contains("single quote") || err_str.contains("key must be a string") {
            suggestion = Some("Wrap object keys and string values in double quotes (\"...\")".to_string());
        }

        let mut issue = SyntaxIssue::error("invalid-json", format!("JSON syntax error: {err_str}"), err_line, err_col);
        if let Some(sug) = suggestion {
            issue = issue.with_suggestion(sug);
        }
        issues.push(issue);
    }
}

/// HTML5 Void Elements which do not have a closing tag.
fn html_void_tags() -> HashSet<&'static str> {
    [
        "area", "base", "br", "col", "embed", "hr", "img", "input", "link", "meta", "param", "source", "track",
        "wbr", "!doctype",
    ]
    .into_iter()
    .collect()
}

#[derive(Debug, Clone)]
struct OpenTag {
    name: String,
    line: usize,
    column: usize,
}

/// Validates HTML and XML tag hierarchy and self-closing structure.
pub fn check_html_xml(content: &str, is_xml: bool, issues: &mut Vec<SyntaxIssue>) {
    let void_tags = html_void_tags();
    let chars: Vec<char> = content.chars().collect();
    let len = chars.len();
    let mut pos = 0;
    let mut line = 1;
    let mut col = 1;

    let mut tag_stack: Vec<OpenTag> = Vec::new();

    while pos < len {
        let ch = chars[pos];
        if ch == '\n' {
            line += 1;
            col = 1;
            pos += 1;
            continue;
        }

        // Check for tag start '<'
        if ch == '<' && pos + 1 < len {
            let start_line = line;
            let start_col = col;
            pos += 1;
            col += 1;

            // Skip comments: <!-- ... -->
            if pos + 2 < len && chars[pos] == '!' && chars[pos + 1] == '-' && chars[pos + 2] == '-' {
                pos += 3;
                col += 3;
                while pos < len {
                    if chars[pos] == '\n' {
                        line += 1;
                        col = 1;
                        pos += 1;
                    } else if chars[pos] == '-' && pos + 2 < len && chars[pos + 1] == '-' && chars[pos + 2] == '>' {
                        pos += 3;
                        col += 3;
                        break;
                    } else {
                        pos += 1;
                        col += 1;
                    }
                }
                continue;
            }

            // Skip CDATA: <![CDATA[ ... ]]>
            if pos + 7 < len && chars[pos..pos + 7].iter().collect::<String>() == "![CDATA[" {
                pos += 7;
                col += 7;
                while pos < len {
                    if chars[pos] == '\n' {
                        line += 1;
                        col = 1;
                        pos += 1;
                    } else if chars[pos] == ']' && pos + 2 < len && chars[pos + 1] == ']' && chars[pos + 2] == '>' {
                        pos += 3;
                        col += 3;
                        break;
                    } else {
                        pos += 1;
                        col += 1;
                    }
                }
                continue;
            }

            // Skip DOCTYPE: <!DOCTYPE ...> or <?xml ... ?>
            if chars[pos] == '!' || chars[pos] == '?' {
                while pos < len && chars[pos] != '>' && chars[pos] != '\n' {
                    pos += 1;
                    col += 1;
                }
                if pos < len && chars[pos] == '>' {
                    pos += 1;
                    col += 1;
                }
                continue;
            }

            // Closing tag: </tag_name>
            let is_closing = if chars[pos] == '/' {
                pos += 1;
                col += 1;
                true
            } else {
                false
            };

            // Read tag name
            let mut tag_name = String::new();
            while pos < len && (chars[pos].is_alphanumeric() || chars[pos] == '-' || chars[pos] == '_' || chars[pos] == ':') {
                tag_name.push(chars[pos]);
                pos += 1;
                col += 1;
            }

            if tag_name.is_empty() {
                continue;
            }

            // Scan attributes until '>'
            let mut is_self_closing = false;
            let mut in_attr_quote: Option<char> = None;

            while pos < len {
                let curr = chars[pos];
                if curr == '\n' {
                    line += 1;
                    col = 1;
                    pos += 1;
                    continue;
                }

                if let Some(qc) = in_attr_quote {
                    if curr == qc {
                        in_attr_quote = None;
                    }
                } else if curr == '"' || curr == '\'' {
                    in_attr_quote = Some(curr);
                } else if curr == '/' && pos + 1 < len && chars[pos + 1] == '>' {
                    is_self_closing = true;
                    pos += 1;
                    col += 1;
                } else if curr == '>' {
                    pos += 1;
                    col += 1;
                    break;
                }
                pos += 1;
                col += 1;
            }

            let tag_lower = tag_name.to_lowercase();

            if is_closing {
                if let Some(top) = tag_stack.pop() {
                    let top_lower = top.name.to_lowercase();
                    if top_lower != tag_lower {
                        issues.push(
                            SyntaxIssue::error(
                                "tag-mismatch",
                                format!(
                                    "Mismatched closing tag '</{tag_name}>' at line {start_line}:{start_col}; expected '</{}>' to close '<{}>' opened at line {}:{}",
                                    top.name, top.name, top.line, top.column
                                ),
                                start_line,
                                start_col,
                            )
                            .with_suggestion(format!("Replace '</{tag_name}>' with '</{}>'", top.name)),
                        );
                    }
                } else {
                    issues.push(
                        SyntaxIssue::error(
                            "unmatched-closing-tag",
                            format!("Unmatched closing tag '</{tag_name}>' at line {start_line}:{start_col} (no opening tag)"),
                            start_line,
                            start_col,
                        )
                        .with_suggestion(format!("Remove '</{tag_name}>' or add opening '<{tag_name}>'")),
                    );
                }
            } else if !is_self_closing {
                if !is_xml && void_tags.contains(tag_lower.as_str()) {
                    // HTML void elements do not need a closing tag
                } else {
                    tag_stack.push(OpenTag {
                        name: tag_name,
                        line: start_line,
                        column: start_col,
                    });
                }
            }
            continue;
        }

        pos += 1;
        col += 1;
    }

    for open in tag_stack {
        issues.push(
            SyntaxIssue::error(
                "unclosed-tag",
                format!(
                    "Unclosed tag '<{}>' opened at line {}:{} (missing closing '</{}>')",
                    open.name, open.line, open.column, open.name
                ),
                open.line,
                open.column,
            )
            .with_suggestion(format!("Add '</{}>' to close '<{}>'", open.name, open.name)),
        );
    }
}

// ---------------------------------------------------------------------------
// Main Validator Coordinator
// ---------------------------------------------------------------------------

/// Performs complete multi-pass syntax validation on code content.
pub fn validate_syntax(
    content: &str,
    lang: SupportedLanguage,
    path: Option<&Path>,
    options: &SyntaxCheckOptions,
) -> SyntaxValidationReport {
    let mut issues = Vec::new();

    match lang {
        SupportedLanguage::Json => {
            check_json(content, &mut issues);
            // Also check brackets / indentation if requested
            check_indentation(content, lang, options, &mut issues);
        }
        SupportedLanguage::Html => {
            check_html_xml(content, false, &mut issues);
            check_delimiters(content, lang, options, &mut issues);
            check_indentation(content, lang, options, &mut issues);
        }
        SupportedLanguage::Xml => {
            check_html_xml(content, true, &mut issues);
            check_delimiters(content, lang, options, &mut issues);
            check_indentation(content, lang, options, &mut issues);
        }
        _ => {
            check_delimiters(content, lang, options, &mut issues);
            check_indentation(content, lang, options, &mut issues);
        }
    }

    // Sort issues by line then column
    issues.sort_by_key(|i| (i.line, i.column));

    let file_path_str = path.map(|p| p.to_string_lossy().to_string());
    SyntaxValidationReport::new(lang.display_name().to_string(), file_path_str, content, issues)
}

// ---------------------------------------------------------------------------
// Diagnostic Rendering & Formatting
// ---------------------------------------------------------------------------

/// Formats a single diagnostic issue with visual source code context and column pointer.
pub fn format_diagnostic(issue: &SyntaxIssue, content: &str, file_name: Option<&str>) -> String {
    let mut out = String::new();
    let file = file_name.unwrap_or("<input>");

    let prefix = match issue.severity {
        IssueSeverity::Error => "\x1b[1;31merror\x1b[0m",
        IssueSeverity::Warning => "\x1b[1;33mwarning\x1b[0m",
        IssueSeverity::Info => "\x1b[1;34minfo\x1b[0m",
    };

    out.push_str(&format!(
        "{prefix}[{}]: {}\n  --> {}:{}:{}\n",
        issue.rule, issue.message, file, issue.line, issue.column
    ));

    let lines: Vec<&str> = content.lines().collect();
    if issue.line > 0 && issue.line <= lines.len() {
        let line_num_str = issue.line.to_string();
        let pad = " ".repeat(line_num_str.len());

        out.push_str(&format!("   {pad} |\n"));
        out.push_str(&format!(
            "   \x1b[1;36m{line_num_str}\x1b[0m | {}\n",
            lines[issue.line - 1]
        ));

        let caret_pad = " ".repeat(issue.column.saturating_sub(1));
        let caret_color = match issue.severity {
            IssueSeverity::Error => "\x1b[1;31m^\x1b[0m",
            IssueSeverity::Warning => "\x1b[1;33m^\x1b[0m",
            IssueSeverity::Info => "\x1b[1;34m^\x1b[0m",
        };

        out.push_str(&format!("   {pad} | {caret_pad}{caret_color}"));

        if let Some(sug) = &issue.suggestion {
            out.push_str(&format!(" \x1b[1mhelp: {}\x1b[0m", sug));
        }
        out.push('\n');
    }

    out
}

/// Formats a complete syntax validation report into human-readable terminal text.
pub fn format_report_text(report: &SyntaxValidationReport, content: &str) -> String {
    let mut out = String::new();
    let target = report
        .file_path
        .as_deref()
        .map(|p| format!("`{p}`"))
        .unwrap_or_else(|| "provided code".to_string());

    if report.valid {
        out.push_str(&format!(
            "\x1b[1;32m✓\x1b[0m Syntax validation passed for {target} ({}, {} lines, {} chars)\n",
            report.language, report.total_lines, report.total_chars
        ));
        if report.warning_count > 0 {
            out.push_str(&format!(
                "  ({} warning{} detected)\n\n",
                report.warning_count,
                if report.warning_count == 1 { "" } else { "s" }
            ));
            for issue in &report.issues {
                out.push_str(&format_diagnostic(issue, content, report.file_path.as_deref()));
                out.push('\n');
            }
        }
    } else {
        out.push_str(&format!(
            "\x1b[1;31m✗\x1b[0m Syntax validation failed for {target} ({}, {} lines) — {} error{}, {} warning{}\n\n",
            report.language,
            report.total_lines,
            report.error_count,
            if report.error_count == 1 { "" } else { "s" },
            report.warning_count,
            if report.warning_count == 1 { "" } else { "s" }
        ));

        for issue in &report.issues {
            out.push_str(&format_diagnostic(issue, content, report.file_path.as_deref()));
            out.push('\n');
        }
    }

    out
}

// ---------------------------------------------------------------------------
// Pure-Rust ANSI Syntax Highlighter Engine
// ---------------------------------------------------------------------------

/// Semantic category of a syntax token.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum SyntaxTokenKind {
    /// Language reserved keywords (e.g. `fn`, `let`, `def`, `class`, `func`, `import`).
    Keyword,
    /// Type identifiers and primitives (e.g. `String`, `u32`, `int`, `boolean`, `Promise`).
    Type,
    /// Function/method definitions and invocations (e.g. `calculate()`, `println!`).
    Function,
    /// String literals, raw strings, template literals, and char literals.
    String,
    /// Numeric literals (integers, floats, hex, bin, scientific notation, unit numbers).
    Number,
    /// Comments (single-line, block, docstrings).
    Comment,
    /// Operators (arithmetic, logical, bitwise, assignment, arrow `->`, `=>`).
    Operator,
    /// Punctuation and delimiters (parentheses, braces, brackets, commas, semicolons).
    Punctuation,
    /// Markup tags (HTML, XML, JSX tags).
    Tag,
    /// Attributes, property keys, object keys (HTML attributes, JSON/YAML/TOML keys).
    Attribute,
    /// Variables, identifiers, and special references (`$VAR`, `self`, `this`).
    Variable,
    /// Constants and literal values (`true`, `false`, `null`, `None`, uppercase consts).
    Constant,
    /// Macros, preprocessor directives, and decorators (`#[derive]`, `#include`, `@deco`).
    Macro,
    /// Unstyled plain text or whitespace.
    Plain,
}

impl SyntaxTokenKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            SyntaxTokenKind::Keyword => "keyword",
            SyntaxTokenKind::Type => "type",
            SyntaxTokenKind::Function => "function",
            SyntaxTokenKind::String => "string",
            SyntaxTokenKind::Number => "number",
            SyntaxTokenKind::Comment => "comment",
            SyntaxTokenKind::Operator => "operator",
            SyntaxTokenKind::Punctuation => "punctuation",
            SyntaxTokenKind::Tag => "tag",
            SyntaxTokenKind::Attribute => "attribute",
            SyntaxTokenKind::Variable => "variable",
            SyntaxTokenKind::Constant => "constant",
            SyntaxTokenKind::Macro => "macro",
            SyntaxTokenKind::Plain => "plain",
        }
    }

    pub fn is_keyword(&self) -> bool {
        matches!(self, SyntaxTokenKind::Keyword)
    }

    pub fn is_type(&self) -> bool {
        matches!(self, SyntaxTokenKind::Type)
    }

    pub fn is_function(&self) -> bool {
        matches!(self, SyntaxTokenKind::Function)
    }

    pub fn is_string(&self) -> bool {
        matches!(self, SyntaxTokenKind::String)
    }

    pub fn is_number(&self) -> bool {
        matches!(self, SyntaxTokenKind::Number)
    }

    pub fn is_comment(&self) -> bool {
        matches!(self, SyntaxTokenKind::Comment)
    }
}

impl std::fmt::Display for SyntaxTokenKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

/// A highlighted syntax token with byte span and semantic kind.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SyntaxToken {
    pub kind: SyntaxTokenKind,
    pub text: String,
    pub start: usize,
    pub end: usize,
}

impl SyntaxToken {
    pub fn new(kind: SyntaxTokenKind, text: impl Into<String>, start: usize, end: usize) -> Self {
        Self {
            kind,
            text: text.into(),
            start,
            end,
        }
    }

    pub fn len(&self) -> usize {
        self.text.len()
    }

    pub fn is_empty(&self) -> bool {
        self.text.is_empty()
    }
}

/// 24-bit RGB color representation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct RgbColor {
    pub r: u8,
    pub g: u8,
    pub b: u8,
}

impl RgbColor {
    pub const fn new(r: u8, g: u8, b: u8) -> Self {
        Self { r, g, b }
    }

    /// Parse hex color string `#RRGGBB` or `#RGB`.
    pub fn from_hex(hex: &str) -> Option<Self> {
        let clean = hex.trim().trim_start_matches('#');
        if clean.len() == 6 {
            let r = u8::from_str_radix(&clean[0..2], 16).ok()?;
            let g = u8::from_str_radix(&clean[2..4], 16).ok()?;
            let b = u8::from_str_radix(&clean[4..6], 16).ok()?;
            Some(Self::new(r, g, b))
        } else if clean.len() == 3 {
            let r = u8::from_str_radix(&clean[0..1], 16).ok()?;
            let g = u8::from_str_radix(&clean[1..2], 16).ok()?;
            let b = u8::from_str_radix(&clean[2..3], 16).ok()?;
            Some(Self::new(r * 17, g * 17, b * 17))
        } else {
            None
        }
    }

    /// Convert 24-bit RGB color to the closest standard ANSI 256 color index (0-255).
    pub fn to_ansi256(&self) -> u8 {
        let palette = ansi_256_palette();
        let mut best_idx = 0u8;
        let mut min_dist = u32::MAX;

        for (idx, &(pr, pg, pb)) in palette.iter().enumerate() {
            let dr = (self.r as i32) - (pr as i32);
            let dg = (self.g as i32) - (pg as i32);
            let db = (self.b as i32) - (pb as i32);
            let dist = (dr * dr + dg * dg + db * db) as u32;
            if dist < min_dist {
                min_dist = dist;
                best_idx = idx as u8;
                if dist == 0 {
                    break;
                }
            }
        }
        best_idx
    }

    /// Convert 24-bit RGB color to closest ANSI 16 foreground code (30-37 or 90-97).
    pub fn to_ansi16_fg(&self) -> u8 {
        let idx = self.to_ansi256();
        match idx {
            0 => 30, // black
            1 => 31, // red
            2 => 32, // green
            3 => 33, // yellow
            4 => 34, // blue
            5 => 35, // magenta
            6 => 36, // cyan
            7 => 37, // white
            8 => 90, // bright black
            9 => 91, // bright red
            10 => 92, // bright green
            11 => 93, // bright yellow
            12 => 94, // bright blue
            13 => 95, // bright magenta
            14 => 96, // bright cyan
            15 => 97, // bright white
            _ => {
                let r = self.r > 128;
                let g = self.g > 128;
                let b = self.b > 128;
                let bright = self.r > 192 || self.g > 192 || self.b > 192;
                let base = match (r, g, b) {
                    (false, false, false) => 30,
                    (true, false, false) => 31,
                    (false, true, false) => 32,
                    (true, true, false) => 33,
                    (false, false, true) => 34,
                    (true, false, true) => 35,
                    (false, true, true) => 36,
                    (true, true, true) => 37,
                };
                if bright && base < 37 {
                    base + 60
                } else {
                    base
                }
            }
        }
    }

    /// Convert to 24-bit TrueColor foreground ANSI escape sequence.
    pub fn to_truecolor_fg(&self) -> String {
        format!("\x1b[38;2;{};{};{}m", self.r, self.g, self.b)
    }

    /// Convert to 24-bit TrueColor background ANSI escape sequence.
    pub fn to_truecolor_bg(&self) -> String {
        format!("\x1b[48;2;{};{};{}m", self.r, self.g, self.b)
    }

    /// Convert to 256-color foreground ANSI escape sequence.
    pub fn to_ansi256_fg(&self) -> String {
        format!("\x1b[38;5;{}m", self.to_ansi256())
    }

    /// Convert to 256-color background ANSI escape sequence.
    pub fn to_ansi256_bg(&self) -> String {
        format!("\x1b[48;5;{}m", self.to_ansi256())
    }
}

/// Static 256-color ANSI reference palette for Euclidean nearest color matching.
fn ansi_256_palette() -> &'static [(u8, u8, u8); 256] {
    static PALETTE: std::sync::OnceLock<[(u8, u8, u8); 256]> = std::sync::OnceLock::new();
    PALETTE.get_or_init(|| {
        let mut p = [(0u8, 0u8, 0u8); 256];
        let std16: [(u8, u8, u8); 16] = [
            (0, 0, 0),       // 0
            (128, 0, 0),     // 1
            (0, 128, 0),     // 2
            (128, 128, 0),   // 3
            (0, 0, 128),     // 4
            (128, 0, 128),   // 5
            (0, 128, 128),   // 6
            (192, 192, 192), // 7
            (128, 128, 128), // 8
            (255, 0, 0),     // 9
            (0, 255, 0),     // 10
            (255, 255, 0),   // 11
            (0, 0, 255),     // 12
            (255, 0, 255),   // 13
            (0, 255, 255),   // 14
            (255, 255, 255), // 15
        ];
        p[0..16].copy_from_slice(&std16);

        let steps = [0u8, 95, 135, 175, 215, 255];
        for r in 0..6 {
            for g in 0..6 {
                for b in 0..6 {
                    let idx = 16 + 36 * r + 6 * g + b;
                    p[idx] = (steps[r], steps[g], steps[b]);
                }
            }
        }

        for i in 0..24 {
            let v = (8 + i * 10) as u8;
            p[232 + i] = (v, v, v);
        }

        p
    })
}

/// Target color formatting mode for ANSI terminal output.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum ColorMode {
    /// 24-bit truecolor (`\x1b[38;2;R;G;Bm`).
    #[default]
    TrueColor,
    /// 256-color palette (`\x1b[38;5;Nm`).
    Ansi256,
    /// Standard 16-color ANSI (`\x1b[31m` .. `\x1b[37m`).
    Ansi16,
    /// Clean plain text with all ANSI escape sequences stripped.
    Plain,
}

impl ColorMode {
    pub fn from_str_hint(s: &str) -> Self {
        match s.trim().to_lowercase().as_str() {
            "truecolor" | "24bit" | "rgb" => ColorMode::TrueColor,
            "256" | "ansi256" | "8bit" => ColorMode::Ansi256,
            "16" | "ansi16" | "4bit" | "basic" => ColorMode::Ansi16,
            "none" | "plain" | "raw" | "off" => ColorMode::Plain,
            _ => ColorMode::TrueColor,
        }
    }
}

/// Visual style for syntax tokens including foreground, background, and text attributes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct HighlightStyle {
    pub fg: Option<RgbColor>,
    pub bg: Option<RgbColor>,
    pub bold: bool,
    pub italic: bool,
    pub underline: bool,
    pub dim: bool,
}

impl HighlightStyle {
    pub const fn new() -> Self {
        Self {
            fg: None,
            bg: None,
            bold: false,
            italic: false,
            underline: false,
            dim: false,
        }
    }

    pub const fn fg(mut self, color: RgbColor) -> Self {
        self.fg = Some(color);
        self
    }

    pub const fn bg(mut self, color: RgbColor) -> Self {
        self.bg = Some(color);
        self
    }

    pub const fn bold(mut self) -> Self {
        self.bold = true;
        self
    }

    pub const fn italic(mut self) -> Self {
        self.italic = true;
        self
    }

    pub const fn underline(mut self) -> Self {
        self.underline = true;
        self
    }

    pub const fn dim(mut self) -> Self {
        self.dim = true;
        self
    }

    /// Formats a text snippet with the configured style in the given ColorMode.
    pub fn format_text(&self, text: &str, mode: ColorMode) -> String {
        if text.is_empty() || mode == ColorMode::Plain {
            return text.to_string();
        }

        let mut ansi = String::new();

        match mode {
            ColorMode::TrueColor => {
                if let Some(fg) = self.fg {
                    ansi.push_str(&fg.to_truecolor_fg());
                }
                if let Some(bg) = self.bg {
                    ansi.push_str(&bg.to_truecolor_bg());
                }
            }
            ColorMode::Ansi256 => {
                if let Some(fg) = self.fg {
                    ansi.push_str(&fg.to_ansi256_fg());
                }
                if let Some(bg) = self.bg {
                    ansi.push_str(&bg.to_ansi256_bg());
                }
            }
            ColorMode::Ansi16 => {
                if let Some(fg) = self.fg {
                    ansi.push_str(&format!("\x1b[{}m", fg.to_ansi16_fg()));
                }
            }
            ColorMode::Plain => {}
        }

        if self.bold {
            ansi.push_str("\x1b[1m");
        }
        if self.dim {
            ansi.push_str("\x1b[2m");
        }
        if self.italic {
            ansi.push_str("\x1b[3m");
        }
        if self.underline {
            ansi.push_str("\x1b[4m");
        }

        if ansi.is_empty() {
            text.to_string()
        } else {
            format!("{}{}\x1b[0m", ansi, text)
        }
    }
}

/// Color and style theme for syntax highlighting.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HighlightTheme {
    pub name: String,
    pub keyword: HighlightStyle,
    pub type_style: HighlightStyle,
    pub function: HighlightStyle,
    pub string: HighlightStyle,
    pub number: HighlightStyle,
    pub comment: HighlightStyle,
    pub operator: HighlightStyle,
    pub punctuation: HighlightStyle,
    pub tag: HighlightStyle,
    pub attribute: HighlightStyle,
    pub variable: HighlightStyle,
    pub constant: HighlightStyle,
    pub macro_style: HighlightStyle,
    pub plain: HighlightStyle,
}

impl HighlightTheme {
    /// Dracula / Modern Dark theme (default).
    pub fn dark() -> Self {
        Self {
            name: "dark".to_string(),
            keyword: HighlightStyle::new().fg(RgbColor::new(255, 121, 198)).bold(),
            type_style: HighlightStyle::new().fg(RgbColor::new(139, 233, 253)),
            function: HighlightStyle::new().fg(RgbColor::new(80, 250, 123)),
            string: HighlightStyle::new().fg(RgbColor::new(241, 250, 140)),
            number: HighlightStyle::new().fg(RgbColor::new(189, 147, 249)),
            comment: HighlightStyle::new().fg(RgbColor::new(98, 114, 164)).italic(),
            operator: HighlightStyle::new().fg(RgbColor::new(255, 121, 198)),
            punctuation: HighlightStyle::new().fg(RgbColor::new(248, 248, 242)),
            tag: HighlightStyle::new().fg(RgbColor::new(255, 121, 198)).bold(),
            attribute: HighlightStyle::new().fg(RgbColor::new(80, 250, 123)).italic(),
            variable: HighlightStyle::new().fg(RgbColor::new(248, 248, 242)),
            constant: HighlightStyle::new().fg(RgbColor::new(189, 147, 249)).bold(),
            macro_style: HighlightStyle::new().fg(RgbColor::new(255, 184, 108)),
            plain: HighlightStyle::new().fg(RgbColor::new(248, 248, 242)),
        }
    }

    /// Monokai Pro vibrant theme.
    pub fn monokai() -> Self {
        Self {
            name: "monokai".to_string(),
            keyword: HighlightStyle::new().fg(RgbColor::new(249, 38, 114)).bold(),
            type_style: HighlightStyle::new().fg(RgbColor::new(102, 217, 239)).italic(),
            function: HighlightStyle::new().fg(RgbColor::new(166, 226, 46)),
            string: HighlightStyle::new().fg(RgbColor::new(230, 219, 116)),
            number: HighlightStyle::new().fg(RgbColor::new(174, 129, 255)),
            comment: HighlightStyle::new().fg(RgbColor::new(117, 113, 94)).italic(),
            operator: HighlightStyle::new().fg(RgbColor::new(249, 38, 114)),
            punctuation: HighlightStyle::new().fg(RgbColor::new(248, 248, 242)),
            tag: HighlightStyle::new().fg(RgbColor::new(249, 38, 114)),
            attribute: HighlightStyle::new().fg(RgbColor::new(166, 226, 46)),
            variable: HighlightStyle::new().fg(RgbColor::new(253, 151, 31)),
            constant: HighlightStyle::new().fg(RgbColor::new(174, 129, 255)),
            macro_style: HighlightStyle::new().fg(RgbColor::new(253, 151, 31)),
            plain: HighlightStyle::new().fg(RgbColor::new(248, 248, 242)),
        }
    }

    /// Atom One Dark balanced theme.
    pub fn one_dark() -> Self {
        Self {
            name: "one_dark".to_string(),
            keyword: HighlightStyle::new().fg(RgbColor::new(198, 120, 221)).bold(),
            type_style: HighlightStyle::new().fg(RgbColor::new(229, 192, 123)),
            function: HighlightStyle::new().fg(RgbColor::new(97, 175, 239)),
            string: HighlightStyle::new().fg(RgbColor::new(152, 195, 121)),
            number: HighlightStyle::new().fg(RgbColor::new(209, 154, 102)),
            comment: HighlightStyle::new().fg(RgbColor::new(92, 99, 112)).italic(),
            operator: HighlightStyle::new().fg(RgbColor::new(86, 182, 194)),
            punctuation: HighlightStyle::new().fg(RgbColor::new(171, 178, 191)),
            tag: HighlightStyle::new().fg(RgbColor::new(224, 108, 117)),
            attribute: HighlightStyle::new().fg(RgbColor::new(209, 154, 102)),
            variable: HighlightStyle::new().fg(RgbColor::new(224, 108, 117)),
            constant: HighlightStyle::new().fg(RgbColor::new(209, 154, 102)),
            macro_style: HighlightStyle::new().fg(RgbColor::new(97, 175, 239)),
            plain: HighlightStyle::new().fg(RgbColor::new(171, 178, 191)),
        }
    }

    /// GitHub Light / Crisp theme.
    pub fn light() -> Self {
        Self {
            name: "light".to_string(),
            keyword: HighlightStyle::new().fg(RgbColor::new(215, 58, 73)).bold(),
            type_style: HighlightStyle::new().fg(RgbColor::new(0, 92, 197)),
            function: HighlightStyle::new().fg(RgbColor::new(111, 66, 193)),
            string: HighlightStyle::new().fg(RgbColor::new(3, 47, 98)),
            number: HighlightStyle::new().fg(RgbColor::new(0, 92, 197)),
            comment: HighlightStyle::new().fg(RgbColor::new(106, 115, 125)).italic(),
            operator: HighlightStyle::new().fg(RgbColor::new(215, 58, 73)),
            punctuation: HighlightStyle::new().fg(RgbColor::new(36, 41, 46)),
            tag: HighlightStyle::new().fg(RgbColor::new(34, 134, 58)),
            attribute: HighlightStyle::new().fg(RgbColor::new(111, 66, 193)),
            variable: HighlightStyle::new().fg(RgbColor::new(227, 98, 9)),
            constant: HighlightStyle::new().fg(RgbColor::new(0, 92, 197)),
            macro_style: HighlightStyle::new().fg(RgbColor::new(215, 58, 73)),
            plain: HighlightStyle::new().fg(RgbColor::new(36, 41, 46)),
        }
    }

    /// Nord Arctic cold theme.
    pub fn nord() -> Self {
        Self {
            name: "nord".to_string(),
            keyword: HighlightStyle::new().fg(RgbColor::new(129, 161, 193)).bold(),
            type_style: HighlightStyle::new().fg(RgbColor::new(143, 188, 187)),
            function: HighlightStyle::new().fg(RgbColor::new(136, 192, 208)),
            string: HighlightStyle::new().fg(RgbColor::new(163, 190, 140)),
            number: HighlightStyle::new().fg(RgbColor::new(180, 142, 173)),
            comment: HighlightStyle::new().fg(RgbColor::new(76, 86, 106)).italic(),
            operator: HighlightStyle::new().fg(RgbColor::new(129, 161, 193)),
            punctuation: HighlightStyle::new().fg(RgbColor::new(236, 239, 244)),
            tag: HighlightStyle::new().fg(RgbColor::new(129, 161, 193)),
            attribute: HighlightStyle::new().fg(RgbColor::new(143, 188, 187)),
            variable: HighlightStyle::new().fg(RgbColor::new(216, 222, 233)),
            constant: HighlightStyle::new().fg(RgbColor::new(180, 142, 173)),
            macro_style: HighlightStyle::new().fg(RgbColor::new(235, 203, 139)),
            plain: HighlightStyle::new().fg(RgbColor::new(216, 222, 233)),
        }
    }

    /// Solarized Dark theme.
    pub fn solarized_dark() -> Self {
        Self {
            name: "solarized_dark".to_string(),
            keyword: HighlightStyle::new().fg(RgbColor::new(133, 153, 0)).bold(),
            type_style: HighlightStyle::new().fg(RgbColor::new(181, 137, 0)),
            function: HighlightStyle::new().fg(RgbColor::new(38, 139, 210)),
            string: HighlightStyle::new().fg(RgbColor::new(42, 161, 152)),
            number: HighlightStyle::new().fg(RgbColor::new(211, 54, 130)),
            comment: HighlightStyle::new().fg(RgbColor::new(88, 110, 117)).italic(),
            operator: HighlightStyle::new().fg(RgbColor::new(133, 153, 0)),
            punctuation: HighlightStyle::new().fg(RgbColor::new(131, 148, 150)),
            tag: HighlightStyle::new().fg(RgbColor::new(38, 139, 210)),
            attribute: HighlightStyle::new().fg(RgbColor::new(181, 137, 0)),
            variable: HighlightStyle::new().fg(RgbColor::new(203, 75, 22)),
            constant: HighlightStyle::new().fg(RgbColor::new(211, 54, 130)),
            macro_style: HighlightStyle::new().fg(RgbColor::new(203, 75, 22)),
            plain: HighlightStyle::new().fg(RgbColor::new(131, 148, 150)),
        }
    }

    pub fn from_name(name: &str) -> Self {
        match name.trim().to_lowercase().as_str() {
            "monokai" => Self::monokai(),
            "one_dark" | "onedark" | "atom" => Self::one_dark(),
            "light" | "github_light" | "github" => Self::light(),
            "nord" => Self::nord(),
            "solarized" | "solarized_dark" => Self::solarized_dark(),
            _ => Self::dark(),
        }
    }

    pub fn style_for(&self, kind: SyntaxTokenKind) -> HighlightStyle {
        match kind {
            SyntaxTokenKind::Keyword => self.keyword,
            SyntaxTokenKind::Type => self.type_style,
            SyntaxTokenKind::Function => self.function,
            SyntaxTokenKind::String => self.string,
            SyntaxTokenKind::Number => self.number,
            SyntaxTokenKind::Comment => self.comment,
            SyntaxTokenKind::Operator => self.operator,
            SyntaxTokenKind::Punctuation => self.punctuation,
            SyntaxTokenKind::Tag => self.tag,
            SyntaxTokenKind::Attribute => self.attribute,
            SyntaxTokenKind::Variable => self.variable,
            SyntaxTokenKind::Constant => self.constant,
            SyntaxTokenKind::Macro => self.macro_style,
            SyntaxTokenKind::Plain => self.plain,
        }
    }

    pub fn format_token(&self, token: &SyntaxToken, mode: ColorMode) -> String {
        let style = self.style_for(token.kind);
        style.format_text(&token.text, mode)
    }
}

impl Default for HighlightTheme {
    fn default() -> Self {
        Self::dark()
    }
}

/// Configurable syntax highlighter instance.
#[derive(Debug, Clone)]
pub struct SyntaxHighlighter {
    pub theme: HighlightTheme,
    pub color_mode: ColorMode,
    pub line_numbers: bool,
    pub start_line_number: usize,
}

impl SyntaxHighlighter {
    pub fn new() -> Self {
        Self {
            theme: HighlightTheme::default(),
            color_mode: ColorMode::default(),
            line_numbers: false,
            start_line_number: 1,
        }
    }

    pub fn with_theme(mut self, theme: HighlightTheme) -> Self {
        self.theme = theme;
        self
    }

    pub fn with_color_mode(mut self, mode: ColorMode) -> Self {
        self.color_mode = mode;
        self
    }

    pub fn with_line_numbers(mut self, enabled: bool) -> Self {
        self.line_numbers = enabled;
        self
    }

    pub fn with_start_line_number(mut self, start: usize) -> Self {
        self.start_line_number = start;
        self
    }

    /// Highlight complete source code string.
    pub fn highlight(&self, content: &str, lang: SupportedLanguage) -> String {
        if self.line_numbers {
            highlight_with_line_numbers(
                content,
                lang,
                self.start_line_number,
                &self.theme,
                self.color_mode,
            )
        } else {
            highlight_with_theme(content, lang, &self.theme, self.color_mode)
        }
    }

    /// Highlight source code line-by-line.
    pub fn highlight_lines(&self, content: &str, lang: SupportedLanguage) -> Vec<String> {
        content
            .lines()
            .map(|line| highlight_line_with_theme(line, lang, &self.theme, self.color_mode))
            .collect()
    }
}

impl Default for SyntaxHighlighter {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Tokenizer Engine & Lexers
// ---------------------------------------------------------------------------

/// Internal streaming character scanner for zero-allocation lexing.
struct SourceScanner<'a> {
    source: &'a str,
    chars: Vec<char>,
    char_to_byte: Vec<usize>,
    pos: usize,
}

impl<'a> SourceScanner<'a> {
    fn new(source: &'a str) -> Self {
        let mut chars = Vec::with_capacity(source.len());
        let mut char_to_byte = Vec::with_capacity(source.len() + 1);
        for (byte_idx, ch) in source.char_indices() {
            chars.push(ch);
            char_to_byte.push(byte_idx);
        }
        char_to_byte.push(source.len());
        Self {
            source,
            chars,
            char_to_byte,
            pos: 0,
        }
    }

    #[inline]
    fn is_eof(&self) -> bool {
        self.pos >= self.chars.len()
    }

    #[inline]
    fn peek(&self) -> Option<char> {
        self.chars.get(self.pos).copied()
    }

    #[inline]
    fn peek_nth(&self, n: usize) -> Option<char> {
        self.chars.get(self.pos + n).copied()
    }

    #[inline]
    fn advance(&mut self) -> Option<char> {
        if self.pos < self.chars.len() {
            let ch = self.chars[self.pos];
            self.pos += 1;
            Some(ch)
        } else {
            None
        }
    }

    #[inline]
    fn advance_n(&mut self, n: usize) {
        self.pos = (self.pos + n).min(self.chars.len());
    }

    fn starts_with_str(&self, s: &str) -> bool {
        let mut offset = 0;
        for expected in s.chars() {
            if self.pos + offset >= self.chars.len() || self.chars[self.pos + offset] != expected {
                return false;
            }
            offset += 1;
        }
        true
    }

    fn make_token(&self, kind: SyntaxTokenKind, start_char: usize, end_char: usize) -> Option<SyntaxToken> {
        if start_char >= end_char {
            return None;
        }
        let start_byte = self.char_to_byte[start_char];
        let end_byte = self.char_to_byte[end_char];
        let text = self.source[start_byte..end_byte].to_string();
        Some(SyntaxToken {
            kind,
            text,
            start: start_byte,
            end: end_byte,
        })
    }

    fn consume_whitespace(&mut self) -> Option<SyntaxToken> {
        let start = self.pos;
        while let Some(c) = self.peek() {
            if c.is_whitespace() {
                self.advance();
            } else {
                break;
            }
        }
        self.make_token(SyntaxTokenKind::Plain, start, self.pos)
    }

    fn consume_digits_and_number(&mut self) -> Option<SyntaxToken> {
        let start = self.pos;
        if self.peek() == Some('0') {
            if let Some(next) = self.peek_nth(1) {
                if next == 'x' || next == 'X' {
                    self.advance_n(2);
                    while let Some(c) = self.peek() {
                        if c.is_ascii_hexdigit() || c == '_' {
                            self.advance();
                        } else {
                            break;
                        }
                    }
                    self.consume_ident_suffix();
                    return self.make_token(SyntaxTokenKind::Number, start, self.pos);
                } else if next == 'b' || next == 'B' {
                    self.advance_n(2);
                    while let Some(c) = self.peek() {
                        if c == '0' || c == '1' || c == '_' {
                            self.advance();
                        } else {
                            break;
                        }
                    }
                    self.consume_ident_suffix();
                    return self.make_token(SyntaxTokenKind::Number, start, self.pos);
                } else if next == 'o' || next == 'O' {
                    self.advance_n(2);
                    while let Some(c) = self.peek() {
                        if (c >= '0' && c <= '7') || c == '_' {
                            self.advance();
                        } else {
                            break;
                        }
                    }
                    self.consume_ident_suffix();
                    return self.make_token(SyntaxTokenKind::Number, start, self.pos);
                }
            }
        }

        while let Some(c) = self.peek() {
            if c.is_ascii_digit() || c == '_' {
                self.advance();
            } else {
                break;
            }
        }

        if self.peek() == Some('.') && self.peek_nth(1).map_or(false, |c| c.is_ascii_digit()) {
            self.advance();
            while let Some(c) = self.peek() {
                if c.is_ascii_digit() || c == '_' {
                    self.advance();
                } else {
                    break;
                }
            }
        }

        if let Some(c) = self.peek() {
            if c == 'e' || c == 'E' {
                self.advance();
                if let Some(sign) = self.peek() {
                    if sign == '+' || sign == '-' {
                        self.advance();
                    }
                }
                while let Some(digit) = self.peek() {
                    if digit.is_ascii_digit() || digit == '_' {
                        self.advance();
                    } else {
                        break;
                    }
                }
            }
        }

        self.consume_ident_suffix();
        self.make_token(SyntaxTokenKind::Number, start, self.pos)
    }

    fn consume_ident_suffix(&mut self) {
        while let Some(c) = self.peek() {
            if c.is_alphanumeric() || c == '_' {
                self.advance();
            } else {
                break;
            }
        }
    }

    fn peek_next_non_whitespace(&self) -> Option<char> {
        let mut offset = 0;
        while let Some(c) = self.chars.get(self.pos + offset) {
            if !c.is_whitespace() {
                return Some(*c);
            }
            offset += 1;
        }
        None
    }
}

const RUST_KEYWORDS: &[&str] = &[
    "as", "async", "await", "break", "const", "continue", "crate", "dyn", "else",
    "enum", "extern", "false", "fn", "for", "if", "impl", "in", "let", "loop",
    "match", "mod", "move", "mut", "pub", "ref", "return", "self", "Self",
    "static", "struct", "super", "trait", "true", "type", "unsafe", "use",
    "where", "while", "yield",
];

const RUST_TYPES: &[&str] = &[
    "bool", "char", "str", "u8", "u16", "u32", "u64", "u128", "usize",
    "i8", "i16", "i32", "i64", "i128", "isize", "f32", "f64",
    "Option", "Result", "Some", "None", "Ok", "Err",
    "Vec", "String", "Box", "Rc", "Arc", "Cell", "RefCell", "Mutex", "RwLock",
    "HashMap", "HashSet", "BTreeMap", "BTreeSet", "VecDeque", "BinaryHeap",
    "Pin", "Future", "Send", "Sync", "Clone", "Copy", "Debug", "Default",
    "Display", "Error", "From", "Into", "Iterator", "IntoIterator",
    "Path", "PathBuf", "File", "Duration", "Instant", "Ordering",
];

fn tokenize_rust(content: &str) -> Vec<SyntaxToken> {
    let mut s = SourceScanner::new(content);
    let mut tokens = Vec::new();

    while !s.is_eof() {
        if let Some(ws) = s.consume_whitespace() {
            tokens.push(ws);
            continue;
        }

        // Line comment
        if s.starts_with_str("//") {
            let start = s.pos;
            s.advance_n(2);
            while let Some(c) = s.peek() {
                if c == '\n' {
                    break;
                }
                s.advance();
            }
            if let Some(tok) = s.make_token(SyntaxTokenKind::Comment, start, s.pos) {
                tokens.push(tok);
            }
            continue;
        }

        // Block comment (with nested tracking)
        if s.starts_with_str("/*") {
            let start = s.pos;
            s.advance_n(2);
            let mut depth = 1usize;
            while !s.is_eof() && depth > 0 {
                if s.starts_with_str("/*") {
                    depth += 1;
                    s.advance_n(2);
                } else if s.starts_with_str("*/") {
                    depth -= 1;
                    s.advance_n(2);
                } else {
                    s.advance();
                }
            }
            if let Some(tok) = s.make_token(SyntaxTokenKind::Comment, start, s.pos) {
                tokens.push(tok);
            }
            continue;
        }

        // Raw string r#"..."# or br#"..."#
        let is_raw_str = s.starts_with_str("r\"")
            || s.starts_with_str("r#")
            || s.starts_with_str("br\"")
            || s.starts_with_str("br#");
        if is_raw_str {
            let start = s.pos;
            if s.starts_with_str("br") {
                s.advance_n(2);
            } else {
                s.advance(); // 'r'
            }
            let mut hashes = 0usize;
            while s.peek() == Some('#') {
                hashes += 1;
                s.advance();
            }
            if s.peek() == Some('"') {
                s.advance();
                let mut closing = String::with_capacity(hashes + 1);
                closing.push('"');
                for _ in 0..hashes {
                    closing.push('#');
                }
                while !s.is_eof() {
                    if s.starts_with_str(&closing) {
                        s.advance_n(closing.chars().count());
                        break;
                    }
                    s.advance();
                }
                if let Some(tok) = s.make_token(SyntaxTokenKind::String, start, s.pos) {
                    tokens.push(tok);
                }
                continue;
            }
        }

        // Standard string literal "..." or b"..."
        if s.starts_with_str("b\"") || s.peek() == Some('"') {
            let start = s.pos;
            if s.starts_with_str("b\"") {
                s.advance_n(2);
            } else {
                s.advance();
            }
            let mut escaped = false;
            while let Some(c) = s.peek() {
                s.advance();
                if escaped {
                    escaped = false;
                } else if c == '\\' {
                    escaped = true;
                } else if c == '"' {
                    break;
                }
            }
            if let Some(tok) = s.make_token(SyntaxTokenKind::String, start, s.pos) {
                tokens.push(tok);
            }
            continue;
        }

        // Character literal vs Lifetime
        if s.starts_with_str("b'") || s.peek() == Some('\'') {
            let start = s.pos;
            let is_byte = s.starts_with_str("b'");
            if is_byte {
                s.advance_n(2);
            } else {
                s.advance();
            }
            // Check if lifetime (e.g. 'a, 'static, '_)
            if !is_byte && s.peek().map_or(false, |c| c.is_alphabetic() || c == '_') {
                let mut len = 0;
                while let Some(c) = s.peek_nth(len) {
                    if c.is_alphanumeric() || c == '_' {
                        len += 1;
                    } else {
                        break;
                    }
                }
                // If not followed by closing quote, it's a lifetime
                if s.peek_nth(len) != Some('\'') {
                    s.advance_n(len);
                    if let Some(tok) = s.make_token(SyntaxTokenKind::Type, start, s.pos) {
                        tokens.push(tok);
                    }
                    continue;
                }
            }
            // Char literal
            let mut escaped = false;
            while let Some(c) = s.peek() {
                s.advance();
                if escaped {
                    escaped = false;
                } else if c == '\\' {
                    escaped = true;
                } else if c == '\'' {
                    break;
                } else if c == '\n' {
                    break;
                }
            }
            if let Some(tok) = s.make_token(SyntaxTokenKind::String, start, s.pos) {
                tokens.push(tok);
            }
            continue;
        }

        // Attributes: #[derive(...)] or #![no_std]
        if s.starts_with_str("#[") || s.starts_with_str("#![") {
            let start = s.pos;
            if s.starts_with_str("#![") {
                s.advance_n(3);
            } else {
                s.advance_n(2);
            }
            let mut depth = 1usize;
            while let Some(c) = s.peek() {
                s.advance();
                if c == '[' {
                    depth += 1;
                } else if c == ']' {
                    depth -= 1;
                    if depth == 0 {
                        break;
                    }
                } else if c == '\n' {
                    break;
                }
            }
            if let Some(tok) = s.make_token(SyntaxTokenKind::Macro, start, s.pos) {
                tokens.push(tok);
            }
            continue;
        }

        // Numbers
        if s.peek().map_or(false, |c| c.is_ascii_digit()) {
            if let Some(num_tok) = s.consume_digits_and_number() {
                tokens.push(num_tok);
                continue;
            }
        }

        // Identifiers, Keywords, Types, Macros, Functions
        if s.peek().map_or(false, |c| c.is_alphabetic() || c == '_') {
            let start = s.pos;
            while let Some(c) = s.peek() {
                if c.is_alphanumeric() || c == '_' {
                    s.advance();
                } else {
                    break;
                }
            }
            let ident_text = &s.source[s.char_to_byte[start]..s.char_to_byte[s.pos]];

            // Macro invocation like println!, vec!
            if s.peek() == Some('!') && s.peek_nth(1) != Some('=') {
                s.advance(); // consume '!'
                if let Some(tok) = s.make_token(SyntaxTokenKind::Macro, start, s.pos) {
                    tokens.push(tok);
                }
                continue;
            }

            let kind = if ident_text == "true" || ident_text == "false" || ident_text == "Some" || ident_text == "None" || ident_text == "Ok" || ident_text == "Err" {
                SyntaxTokenKind::Constant
            } else if ident_text == "self" {
                SyntaxTokenKind::Variable
            } else if ident_text == "Self" {
                SyntaxTokenKind::Type
            } else if RUST_KEYWORDS.contains(&ident_text) {
                SyntaxTokenKind::Keyword
            } else if RUST_TYPES.contains(&ident_text) {
                SyntaxTokenKind::Type
            } else if ident_text.chars().all(|c| c.is_uppercase() || c == '_' || c.is_ascii_digit()) && ident_text.len() >= 2 {
                SyntaxTokenKind::Constant
            } else if s.peek_next_non_whitespace() == Some('(') {
                SyntaxTokenKind::Function
            } else if ident_text.chars().next().map_or(false, |c| c.is_uppercase()) {
                SyntaxTokenKind::Type
            } else {
                SyntaxTokenKind::Plain
            };

            if let Some(tok) = s.make_token(kind, start, s.pos) {
                tokens.push(tok);
            }
            continue;
        }

        // Multi-character operators
        let multi_ops = &[
            "::", "->", "=>", "==", "!=", "<=", ">=", "&&", "||", "+=", "-=", "*=", "/=", "%=",
            "&=", "|=", "^=", "<<=", ">>=", "..=", "..",
        ];
        let mut matched_op = false;
        for op in multi_ops {
            if s.starts_with_str(op) {
                let start = s.pos;
                s.advance_n(op.chars().count());
                if let Some(tok) = s.make_token(SyntaxTokenKind::Operator, start, s.pos) {
                    tokens.push(tok);
                }
                matched_op = true;
                break;
            }
        }
        if matched_op {
            continue;
        }

        // Single character operators and punctuation
        let start = s.pos;
        let ch = s.advance();
        let kind = match ch {
            Some('+' | '-' | '*' | '/' | '%' | '&' | '|' | '^' | '<' | '>' | '=' | '!' | '?') => {
                SyntaxTokenKind::Operator
            }
            Some('(' | ')' | '{' | '}' | '[' | ']' | ';' | ',' | '.' | ':') => {
                SyntaxTokenKind::Punctuation
            }
            _ => SyntaxTokenKind::Plain,
        };

        if let Some(tok) = s.make_token(kind, start, s.pos) {
            tokens.push(tok);
        }
    }

    tokens
}

const JS_KEYWORDS: &[&str] = &[
    "abstract", "any", "as", "async", "await", "boolean", "break", "case", "catch",
    "class", "const", "continue", "debugger", "declare", "default", "delete", "do",
    "else", "enum", "export", "extends", "false", "finally", "for", "from", "function",
    "get", "if", "implements", "import", "in", "instanceof", "interface", "is",
    "keyof", "let", "namespace", "never", "new", "null", "number", "object", "of",
    "override", "package", "private", "protected", "public", "readonly", "require",
    "return", "satisfies", "set", "static", "string", "super", "switch", "symbol",
    "this", "throw", "true", "try", "type", "typeof", "undefined", "unknown", "var",
    "void", "while", "with", "yield",
];

const JS_TYPES: &[&str] = &[
    "Array", "Boolean", "Date", "Error", "Function", "Map", "Number", "Object",
    "Promise", "RegExp", "Set", "String", "Symbol", "BigInt", "Record", "Partial",
    "Required", "Readonly", "Pick", "Omit", "Exclude", "Extract", "NonNullable",
    "Parameters", "ReturnType", "InstanceType", "Console", "console", "document",
    "window", "process", "Math", "JSON", "Node", "Element", "Event",
];

fn tokenize_javascript_typescript(content: &str, _lang: SupportedLanguage) -> Vec<SyntaxToken> {
    let mut s = SourceScanner::new(content);
    let mut tokens = Vec::new();

    while !s.is_eof() {
        if let Some(ws) = s.consume_whitespace() {
            tokens.push(ws);
            continue;
        }

        // Line comment
        if s.starts_with_str("//") {
            let start = s.pos;
            s.advance_n(2);
            while let Some(c) = s.peek() {
                if c == '\n' {
                    break;
                }
                s.advance();
            }
            if let Some(tok) = s.make_token(SyntaxTokenKind::Comment, start, s.pos) {
                tokens.push(tok);
            }
            continue;
        }

        // Block comment
        if s.starts_with_str("/*") {
            let start = s.pos;
            s.advance_n(2);
            while !s.is_eof() {
                if s.starts_with_str("*/") {
                    s.advance_n(2);
                    break;
                }
                s.advance();
            }
            if let Some(tok) = s.make_token(SyntaxTokenKind::Comment, start, s.pos) {
                tokens.push(tok);
            }
            continue;
        }

        // Template string `...`
        if s.peek() == Some('`') {
            let start = s.pos;
            s.advance();
            let mut escaped = false;
            while let Some(c) = s.peek() {
                s.advance();
                if escaped {
                    escaped = false;
                } else if c == '\\' {
                    escaped = true;
                } else if c == '`' {
                    break;
                }
            }
            if let Some(tok) = s.make_token(SyntaxTokenKind::String, start, s.pos) {
                tokens.push(tok);
            }
            continue;
        }

        // Normal string literal "..." or '...'
        if s.peek() == Some('"') || s.peek() == Some('\'') {
            let quote = s.advance().unwrap_or('"');
            let start = s.pos - 1;
            let mut escaped = false;
            while let Some(c) = s.peek() {
                s.advance();
                if escaped {
                    escaped = false;
                } else if c == '\\' {
                    escaped = true;
                } else if c == quote {
                    break;
                } else if c == '\n' {
                    break;
                }
            }
            if let Some(tok) = s.make_token(SyntaxTokenKind::String, start, s.pos) {
                tokens.push(tok);
            }
            continue;
        }

        // Decorators @name
        if s.peek() == Some('@') && s.peek_nth(1).map_or(false, |c| c.is_alphabetic() || c == '_') {
            let start = s.pos;
            s.advance();
            while let Some(c) = s.peek() {
                if c.is_alphanumeric() || c == '_' {
                    s.advance();
                } else {
                    break;
                }
            }
            if let Some(tok) = s.make_token(SyntaxTokenKind::Macro, start, s.pos) {
                tokens.push(tok);
            }
            continue;
        }

        // JSX closing tags </Tag> or opening <Tag>
        if s.starts_with_str("</") && s.peek_nth(2).map_or(false, |c| c.is_alphabetic()) {
            let start = s.pos;
            s.advance_n(2);
            while let Some(c) = s.peek() {
                if c.is_alphanumeric() || c == '-' || c == '_' || c == '.' {
                    s.advance();
                } else {
                    break;
                }
            }
            if let Some(tok) = s.make_token(SyntaxTokenKind::Tag, start, s.pos) {
                tokens.push(tok);
            }
            continue;
        }

        // Numbers
        if s.peek().map_or(false, |c| c.is_ascii_digit()) {
            if let Some(num_tok) = s.consume_digits_and_number() {
                tokens.push(num_tok);
                continue;
            }
        }

        // Identifiers, Keywords, Types, Functions
        if s.peek().map_or(false, |c| c.is_alphabetic() || c == '_' || c == '$') {
            let start = s.pos;
            while let Some(c) = s.peek() {
                if c.is_alphanumeric() || c == '_' || c == '$' {
                    s.advance();
                } else {
                    break;
                }
            }
            let ident_text = &s.source[s.char_to_byte[start]..s.char_to_byte[s.pos]];

            let kind = if ident_text == "true" || ident_text == "false" || ident_text == "null" || ident_text == "undefined" || ident_text == "NaN" || ident_text == "Infinity" {
                SyntaxTokenKind::Constant
            } else if ident_text == "this" || ident_text == "super" {
                SyntaxTokenKind::Variable
            } else if ident_text == "any" || ident_text == "boolean" || ident_text == "never" || ident_text == "number" || ident_text == "object" || ident_text == "string" || ident_text == "symbol" || ident_text == "unknown" || ident_text == "void" {
                SyntaxTokenKind::Type
            } else if JS_KEYWORDS.contains(&ident_text) {
                SyntaxTokenKind::Keyword
            } else if JS_TYPES.contains(&ident_text) {
                SyntaxTokenKind::Type
            } else if ident_text.chars().all(|c| c.is_uppercase() || c == '_' || c.is_ascii_digit()) && ident_text.len() >= 2 {
                SyntaxTokenKind::Constant
            } else if s.peek_next_non_whitespace() == Some('(') {
                SyntaxTokenKind::Function
            } else if ident_text.chars().next().map_or(false, |c| c.is_uppercase()) {
                SyntaxTokenKind::Type
            } else {
                SyntaxTokenKind::Plain
            };

            if let Some(tok) = s.make_token(kind, start, s.pos) {
                tokens.push(tok);
            }
            continue;
        }

        // Multi-char operators
        let multi_ops = &[
            "===", "!==", "==", "!=", "<=", ">=", "=>", "&&", "||", "??", "?.",
            "+=", "-=", "*=", "/=", "%=", "**=", "**", "++", "--", "<<=", ">>=",
        ];
        let mut matched_op = false;
        for op in multi_ops {
            if s.starts_with_str(op) {
                let start = s.pos;
                s.advance_n(op.chars().count());
                if let Some(tok) = s.make_token(SyntaxTokenKind::Operator, start, s.pos) {
                    tokens.push(tok);
                }
                matched_op = true;
                break;
            }
        }
        if matched_op {
            continue;
        }

        // Single character operators / punctuation
        let start = s.pos;
        let ch = s.advance();
        let kind = match ch {
            Some('+' | '-' | '*' | '/' | '%' | '=' | '<' | '>' | '!' | '&' | '|' | '^' | '~' | '?') => {
                SyntaxTokenKind::Operator
            }
            Some('(' | ')' | '{' | '}' | '[' | ']' | ';' | ',' | '.' | ':') => {
                SyntaxTokenKind::Punctuation
            }
            _ => SyntaxTokenKind::Plain,
        };

        if let Some(tok) = s.make_token(kind, start, s.pos) {
            tokens.push(tok);
        }
    }

    tokens
}

const PYTHON_KEYWORDS: &[&str] = &[
    "and", "as", "assert", "async", "await", "break", "case", "class", "continue",
    "def", "del", "elif", "else", "except", "finally", "for", "from", "global",
    "if", "import", "in", "is", "lambda", "match", "nonlocal", "not", "or",
    "pass", "raise", "return", "try", "while", "with", "yield",
];

const PYTHON_BUILTINS: &[&str] = &[
    "int", "float", "str", "bool", "list", "dict", "set", "tuple", "bytes", "bytearray",
    "object", "type", "range", "len", "print", "input", "open", "isinstance", "issubclass",
    "enumerate", "zip", "map", "filter", "any", "all", "min", "max", "sum", "abs",
    "repr", "id", "dir", "vars", "callable", "iter", "next", "super", "property",
    "staticmethod", "classmethod", "Optional", "Union", "List", "Dict", "Set",
    "Tuple", "Any", "Callable", "Iterable", "Iterator", "Sequence", "Mapping", "Generator",
];

fn tokenize_python(content: &str) -> Vec<SyntaxToken> {
    let mut s = SourceScanner::new(content);
    let mut tokens = Vec::new();

    while !s.is_eof() {
        if let Some(ws) = s.consume_whitespace() {
            tokens.push(ws);
            continue;
        }

        // Comment
        if s.peek() == Some('#') {
            let start = s.pos;
            while let Some(c) = s.peek() {
                if c == '\n' {
                    break;
                }
                s.advance();
            }
            if let Some(tok) = s.make_token(SyntaxTokenKind::Comment, start, s.pos) {
                tokens.push(tok);
            }
            continue;
        }

        // Triple quoted strings: """...""" or '''...'''
        let prefixes = &["f\"\"\"", "r\"\"\"", "b\"\"\"", "u\"\"\"", "fr\"\"\"", "rf\"\"\"", "\"\"\"",
                         "f'''", "r'''", "b'''", "u'''", "fr'''", "rf'''", "'''"];
        let mut matched_triple = false;
        for p in prefixes {
            if s.starts_with_str(p) {
                let start = s.pos;
                let quote_char = if p.contains('"') { "\"\"\"" } else { "'''" };
                s.advance_n(p.chars().count());
                while !s.is_eof() {
                    if s.starts_with_str(quote_char) {
                        s.advance_n(3);
                        break;
                    }
                    s.advance();
                }
                if let Some(tok) = s.make_token(SyntaxTokenKind::String, start, s.pos) {
                    tokens.push(tok);
                }
                matched_triple = true;
                break;
            }
        }
        if matched_triple {
            continue;
        }

        // Single/double quoted strings with optional prefix
        let str_prefixes = &["f\"", "r\"", "b\"", "u\"", "fr\"", "rf\"", "\"",
                            "f'", "r'", "b'", "u'", "fr'", "rf'", "'"];
        let mut matched_str = false;
        for p in str_prefixes {
            if s.starts_with_str(p) {
                let start = s.pos;
                let quote = if p.ends_with('"') { '"' } else { '\'' };
                s.advance_n(p.chars().count());
                let mut escaped = false;
                while let Some(c) = s.peek() {
                    s.advance();
                    if escaped {
                        escaped = false;
                    } else if c == '\\' {
                        escaped = true;
                    } else if c == quote {
                        break;
                    } else if c == '\n' {
                        break;
                    }
                }
                if let Some(tok) = s.make_token(SyntaxTokenKind::String, start, s.pos) {
                    tokens.push(tok);
                }
                matched_str = true;
                break;
            }
        }
        if matched_str {
            continue;
        }

        // Decorator @deco
        if s.peek() == Some('@') && s.peek_nth(1).map_or(false, |c| c.is_alphabetic() || c == '_') {
            let start = s.pos;
            s.advance();
            while let Some(c) = s.peek() {
                if c.is_alphanumeric() || c == '_' || c == '.' {
                    s.advance();
                } else {
                    break;
                }
            }
            if let Some(tok) = s.make_token(SyntaxTokenKind::Macro, start, s.pos) {
                tokens.push(tok);
            }
            continue;
        }

        // Numbers
        if s.peek().map_or(false, |c| c.is_ascii_digit()) {
            if let Some(num_tok) = s.consume_digits_and_number() {
                tokens.push(num_tok);
                continue;
            }
        }

        // Identifiers, Keywords, Builtins, Functions
        if s.peek().map_or(false, |c| c.is_alphabetic() || c == '_') {
            let start = s.pos;
            while let Some(c) = s.peek() {
                if c.is_alphanumeric() || c == '_' {
                    s.advance();
                } else {
                    break;
                }
            }
            let ident_text = &s.source[s.char_to_byte[start]..s.char_to_byte[s.pos]];

            let kind = if ident_text == "True" || ident_text == "False" || ident_text == "None" || ident_text == "Ellipsis" || ident_text == "NotImplemented" {
                SyntaxTokenKind::Constant
            } else if ident_text == "self" || ident_text == "cls" {
                SyntaxTokenKind::Variable
            } else if PYTHON_KEYWORDS.contains(&ident_text) {
                SyntaxTokenKind::Keyword
            } else if PYTHON_BUILTINS.contains(&ident_text) {
                SyntaxTokenKind::Type
            } else if ident_text.chars().all(|c| c.is_uppercase() || c == '_' || c.is_ascii_digit()) && ident_text.len() >= 2 {
                SyntaxTokenKind::Constant
            } else if s.peek_next_non_whitespace() == Some('(') {
                SyntaxTokenKind::Function
            } else if ident_text.chars().next().map_or(false, |c| c.is_uppercase()) {
                SyntaxTokenKind::Type
            } else {
                SyntaxTokenKind::Plain
            };

            if let Some(tok) = s.make_token(kind, start, s.pos) {
                tokens.push(tok);
            }
            continue;
        }

        // Multi-character operators
        let multi_ops = &[
            "->", "==", "!=", "<=", ">=", "+=", "-=", "*=", "/=", "//=", "%=",
            "**=", "**", "//", ":=", "<<=", ">>=",
        ];
        let mut matched_op = false;
        for op in multi_ops {
            if s.starts_with_str(op) {
                let start = s.pos;
                s.advance_n(op.chars().count());
                if let Some(tok) = s.make_token(SyntaxTokenKind::Operator, start, s.pos) {
                    tokens.push(tok);
                }
                matched_op = true;
                break;
            }
        }
        if matched_op {
            continue;
        }

        // Single character operators and punctuation
        let start = s.pos;
        let ch = s.advance();
        let kind = match ch {
            Some('+' | '-' | '*' | '/' | '%' | '=' | '<' | '>' | '&' | '|' | '^' | '~' | '@') => {
                SyntaxTokenKind::Operator
            }
            Some('(' | ')' | '{' | '}' | '[' | ']' | ':' | ',' | '.' | ';') => {
                SyntaxTokenKind::Punctuation
            }
            _ => SyntaxTokenKind::Plain,
        };

        if let Some(tok) = s.make_token(kind, start, s.pos) {
            tokens.push(tok);
        }
    }

    tokens
}

const GO_KEYWORDS: &[&str] = &[
    "break", "case", "chan", "const", "continue", "default", "defer", "else",
    "fallthrough", "for", "func", "go", "goto", "if", "import", "interface",
    "map", "package", "range", "return", "select", "struct", "switch", "type", "var",
];

const GO_TYPES: &[&str] = &[
    "bool", "byte", "complex64", "complex128", "error", "float32", "float64",
    "int", "int8", "int16", "int32", "int64", "rune", "string",
    "uint", "uint8", "uint16", "uint32", "uint64", "uintptr", "any",
    "append", "cap", "close", "complex", "copy", "delete", "imag", "len",
    "make", "new", "panic", "print", "println", "real", "recover",
];

fn tokenize_go(content: &str) -> Vec<SyntaxToken> {
    let mut s = SourceScanner::new(content);
    let mut tokens = Vec::new();

    while !s.is_eof() {
        if let Some(ws) = s.consume_whitespace() {
            tokens.push(ws);
            continue;
        }

        // Line comment
        if s.starts_with_str("//") {
            let start = s.pos;
            s.advance_n(2);
            while let Some(c) = s.peek() {
                if c == '\n' {
                    break;
                }
                s.advance();
            }
            if let Some(tok) = s.make_token(SyntaxTokenKind::Comment, start, s.pos) {
                tokens.push(tok);
            }
            continue;
        }

        // Block comment
        if s.starts_with_str("/*") {
            let start = s.pos;
            s.advance_n(2);
            while !s.is_eof() {
                if s.starts_with_str("*/") {
                    s.advance_n(2);
                    break;
                }
                s.advance();
            }
            if let Some(tok) = s.make_token(SyntaxTokenKind::Comment, start, s.pos) {
                tokens.push(tok);
            }
            continue;
        }

        // Raw backtick string
        if s.peek() == Some('`') {
            let start = s.pos;
            s.advance();
            while let Some(c) = s.peek() {
                s.advance();
                if c == '`' {
                    break;
                }
            }
            if let Some(tok) = s.make_token(SyntaxTokenKind::String, start, s.pos) {
                tokens.push(tok);
            }
            continue;
        }

        // Double-quoted string or char literal
        if s.peek() == Some('"') || s.peek() == Some('\'') {
            let quote = s.advance().unwrap_or('"');
            let start = s.pos - 1;
            let mut escaped = false;
            while let Some(c) = s.peek() {
                s.advance();
                if escaped {
                    escaped = false;
                } else if c == '\\' {
                    escaped = true;
                } else if c == quote {
                    break;
                } else if c == '\n' {
                    break;
                }
            }
            if let Some(tok) = s.make_token(SyntaxTokenKind::String, start, s.pos) {
                tokens.push(tok);
            }
            continue;
        }

        // Numbers
        if s.peek().map_or(false, |c| c.is_ascii_digit()) {
            if let Some(num_tok) = s.consume_digits_and_number() {
                tokens.push(num_tok);
                continue;
            }
        }

        // Identifiers, Keywords, Types, Functions
        if s.peek().map_or(false, |c| c.is_alphabetic() || c == '_') {
            let start = s.pos;
            while let Some(c) = s.peek() {
                if c.is_alphanumeric() || c == '_' {
                    s.advance();
                } else {
                    break;
                }
            }
            let ident_text = &s.source[s.char_to_byte[start]..s.char_to_byte[s.pos]];

            let kind = if ident_text == "true" || ident_text == "false" || ident_text == "iota" || ident_text == "nil" {
                SyntaxTokenKind::Constant
            } else if GO_KEYWORDS.contains(&ident_text) {
                SyntaxTokenKind::Keyword
            } else if GO_TYPES.contains(&ident_text) {
                SyntaxTokenKind::Type
            } else if ident_text.chars().all(|c| c.is_uppercase() || c == '_' || c.is_ascii_digit()) && ident_text.len() >= 2 {
                SyntaxTokenKind::Constant
            } else if s.peek_next_non_whitespace() == Some('(') {
                SyntaxTokenKind::Function
            } else if ident_text.chars().next().map_or(false, |c| c.is_uppercase()) {
                SyntaxTokenKind::Type
            } else {
                SyntaxTokenKind::Plain
            };

            if let Some(tok) = s.make_token(kind, start, s.pos) {
                tokens.push(tok);
            }
            continue;
        }

        // Multi-char operators
        let multi_ops = &[
            ":=", "<-", "==", "!=", "<=", ">=", "&&", "||", "++", "--", "+=", "-=",
            "*=", "/=", "%=", "&=", "|=", "^=", "<<=", ">>=", "...",
        ];
        let mut matched_op = false;
        for op in multi_ops {
            if s.starts_with_str(op) {
                let start = s.pos;
                s.advance_n(op.chars().count());
                if let Some(tok) = s.make_token(SyntaxTokenKind::Operator, start, s.pos) {
                    tokens.push(tok);
                }
                matched_op = true;
                break;
            }
        }
        if matched_op {
            continue;
        }

        // Single character operators and punctuation
        let start = s.pos;
        let ch = s.advance();
        let kind = match ch {
            Some('+' | '-' | '*' | '/' | '%' | '&' | '|' | '^' | '<' | '>' | '=' | '!') => {
                SyntaxTokenKind::Operator
            }
            Some('(' | ')' | '{' | '}' | '[' | ']' | ';' | ',' | '.' | ':') => {
                SyntaxTokenKind::Punctuation
            }
            _ => SyntaxTokenKind::Plain,
        };

        if let Some(tok) = s.make_token(kind, start, s.pos) {
            tokens.push(tok);
        }
    }

    tokens
}

const C_CPP_KEYWORDS: &[&str] = &[
    "alignas", "alignof", "and", "and_eq", "asm", "auto", "bitand", "bitor",
    "bool", "break", "case", "catch", "char", "char8_t", "char16_t", "char32_t",
    "class", "compl", "concept", "const", "consteval", "constexpr", "constinit",
    "const_cast", "continue", "co_await", "co_return", "co_yield", "decltype",
    "default", "delete", "do", "double", "dynamic_cast", "else", "enum", "explicit",
    "export", "extern", "false", "float", "for", "friend", "goto", "if", "inline",
    "int", "long", "mutable", "namespace", "new", "noexcept", "not", "not_eq",
    "nullptr", "operator", "or", "or_eq", "override", "private", "protected",
    "public", "reflexpr", "register", "reinterpret_cast", "requires", "return",
    "short", "signed", "sizeof", "static", "static_assert", "static_cast",
    "struct", "switch", "template", "this", "thread_local", "throw", "true",
    "try", "typedef", "typeid", "typename", "union", "unsigned", "using",
    "virtual", "void", "volatile", "wchar_t", "while", "xor", "xor_eq",
];

const C_CPP_TYPES: &[&str] = &[
    "int8_t", "int16_t", "int32_t", "int64_t", "uint8_t", "uint16_t", "uint32_t", "uint64_t",
    "size_t", "ssize_t", "intptr_t", "uintptr_t", "ptrdiff_t",
    "string", "wstring", "vector", "map", "unordered_map", "set", "unordered_set",
    "pair", "tuple", "shared_ptr", "unique_ptr", "weak_ptr", "make_shared", "make_unique",
    "std", "cout", "cin", "cerr", "endl", "FILE",
];

fn tokenize_c_cpp_family(content: &str, _lang: SupportedLanguage) -> Vec<SyntaxToken> {
    let mut s = SourceScanner::new(content);
    let mut tokens = Vec::new();

    while !s.is_eof() {
        if let Some(ws) = s.consume_whitespace() {
            tokens.push(ws);
            continue;
        }

        // Line comment
        if s.starts_with_str("//") {
            let start = s.pos;
            s.advance_n(2);
            while let Some(c) = s.peek() {
                if c == '\n' {
                    break;
                }
                s.advance();
            }
            if let Some(tok) = s.make_token(SyntaxTokenKind::Comment, start, s.pos) {
                tokens.push(tok);
            }
            continue;
        }

        // Block comment
        if s.starts_with_str("/*") {
            let start = s.pos;
            s.advance_n(2);
            while !s.is_eof() {
                if s.starts_with_str("*/") {
                    s.advance_n(2);
                    break;
                }
                s.advance();
            }
            if let Some(tok) = s.make_token(SyntaxTokenKind::Comment, start, s.pos) {
                tokens.push(tok);
            }
            continue;
        }

        // Preprocessor directives #include, #define, #ifdef, etc.
        if s.peek() == Some('#') {
            let start = s.pos;
            s.advance();
            while let Some(c) = s.peek() {
                if c.is_alphanumeric() || c == '_' {
                    s.advance();
                } else {
                    break;
                }
            }
            if let Some(tok) = s.make_token(SyntaxTokenKind::Macro, start, s.pos) {
                tokens.push(tok);
            }
            continue;
        }

        // Strings "..." or 'c'
        if s.peek() == Some('"') || s.peek() == Some('\'') {
            let quote = s.advance().unwrap_or('"');
            let start = s.pos - 1;
            let mut escaped = false;
            while let Some(c) = s.peek() {
                s.advance();
                if escaped {
                    escaped = false;
                } else if c == '\\' {
                    escaped = true;
                } else if c == quote {
                    break;
                } else if c == '\n' {
                    break;
                }
            }
            if let Some(tok) = s.make_token(SyntaxTokenKind::String, start, s.pos) {
                tokens.push(tok);
            }
            continue;
        }

        // Numbers
        if s.peek().map_or(false, |c| c.is_ascii_digit()) {
            if let Some(num_tok) = s.consume_digits_and_number() {
                tokens.push(num_tok);
                continue;
            }
        }

        // Identifiers, Keywords, Types, Functions
        if s.peek().map_or(false, |c| c.is_alphabetic() || c == '_') {
            let start = s.pos;
            while let Some(c) = s.peek() {
                if c.is_alphanumeric() || c == '_' {
                    s.advance();
                } else {
                    break;
                }
            }
            let ident_text = &s.source[s.char_to_byte[start]..s.char_to_byte[s.pos]];

            let kind = if ident_text == "true" || ident_text == "false" || ident_text == "nullptr" || ident_text == "NULL" {
                SyntaxTokenKind::Constant
            } else if ident_text == "this" {
                SyntaxTokenKind::Variable
            } else if C_CPP_KEYWORDS.contains(&ident_text) {
                SyntaxTokenKind::Keyword
            } else if C_CPP_TYPES.contains(&ident_text) {
                SyntaxTokenKind::Type
            } else if ident_text.chars().all(|c| c.is_uppercase() || c == '_' || c.is_ascii_digit()) && ident_text.len() >= 2 {
                SyntaxTokenKind::Constant
            } else if s.peek_next_non_whitespace() == Some('(') {
                SyntaxTokenKind::Function
            } else if ident_text.chars().next().map_or(false, |c| c.is_uppercase()) {
                SyntaxTokenKind::Type
            } else {
                SyntaxTokenKind::Plain
            };

            if let Some(tok) = s.make_token(kind, start, s.pos) {
                tokens.push(tok);
            }
            continue;
        }

        // Multi-char operators
        let multi_ops = &[
            "::", "->*", "->", "==", "!=", "<=", ">=", "&&", "||", "++", "--",
            "+=", "-=", "*=", "/=", "%=", "<<=", ">>=", "<<", ">>",
        ];
        let mut matched_op = false;
        for op in multi_ops {
            if s.starts_with_str(op) {
                let start = s.pos;
                s.advance_n(op.chars().count());
                if let Some(tok) = s.make_token(SyntaxTokenKind::Operator, start, s.pos) {
                    tokens.push(tok);
                }
                matched_op = true;
                break;
            }
        }
        if matched_op {
            continue;
        }

        // Single character operators and punctuation
        let start = s.pos;
        let ch = s.advance();
        let kind = match ch {
            Some('+' | '-' | '*' | '/' | '%' | '&' | '|' | '^' | '~' | '<' | '>' | '=' | '!' | '?') => {
                SyntaxTokenKind::Operator
            }
            Some('(' | ')' | '{' | '}' | '[' | ']' | ';' | ',' | '.' | ':') => {
                SyntaxTokenKind::Punctuation
            }
            _ => SyntaxTokenKind::Plain,
        };

        if let Some(tok) = s.make_token(kind, start, s.pos) {
            tokens.push(tok);
        }
    }

    tokens
}

fn tokenize_html_xml(content: &str, _is_xml: bool) -> Vec<SyntaxToken> {
    let mut s = SourceScanner::new(content);
    let mut tokens = Vec::new();

    while !s.is_eof() {
        if let Some(ws) = s.consume_whitespace() {
            tokens.push(ws);
            continue;
        }

        // Comments <!-- ... -->
        if s.starts_with_str("<!--") {
            let start = s.pos;
            s.advance_n(4);
            while !s.is_eof() {
                if s.starts_with_str("-->") {
                    s.advance_n(3);
                    break;
                }
                s.advance();
            }
            if let Some(tok) = s.make_token(SyntaxTokenKind::Comment, start, s.pos) {
                tokens.push(tok);
            }
            continue;
        }

        // Doctype / XML header
        if s.starts_with_str("<!DOCTYPE") || s.starts_with_str("<!doctype") || s.starts_with_str("<?xml") {
            let start = s.pos;
            while let Some(c) = s.peek() {
                s.advance();
                if c == '>' {
                    break;
                }
            }
            if let Some(tok) = s.make_token(SyntaxTokenKind::Macro, start, s.pos) {
                tokens.push(tok);
            }
            continue;
        }

        // HTML / XML tag: <tag or </tag
        if s.starts_with_str("</") || s.peek() == Some('<') {
            let start = s.pos;
            if s.starts_with_str("</") {
                s.advance_n(2);
            } else {
                s.advance();
            }
            // Tag name
            while let Some(c) = s.peek() {
                if c.is_alphanumeric() || c == '-' || c == '_' || c == ':' {
                    s.advance();
                } else {
                    break;
                }
            }
            if let Some(tok) = s.make_token(SyntaxTokenKind::Tag, start, s.pos) {
                tokens.push(tok);
            }

            // Tag attributes until '>' or '/>'
            while !s.is_eof() {
                if let Some(ws) = s.consume_whitespace() {
                    tokens.push(ws);
                    continue;
                }
                if s.starts_with_str("/>") {
                    let start_close = s.pos;
                    s.advance_n(2);
                    if let Some(tok) = s.make_token(SyntaxTokenKind::Tag, start_close, s.pos) {
                        tokens.push(tok);
                    }
                    break;
                }
                if s.peek() == Some('>') {
                    let start_close = s.pos;
                    s.advance();
                    if let Some(tok) = s.make_token(SyntaxTokenKind::Tag, start_close, s.pos) {
                        tokens.push(tok);
                    }
                    break;
                }
                if s.peek() == Some('"') || s.peek() == Some('\'') {
                    let quote = s.advance().unwrap_or('"');
                    let start_str = s.pos - 1;
                    while let Some(c) = s.peek() {
                        s.advance();
                        if c == quote {
                            break;
                        }
                    }
                    if let Some(tok) = s.make_token(SyntaxTokenKind::String, start_str, s.pos) {
                        tokens.push(tok);
                    }
                    continue;
                }
                if s.peek() == Some('=') {
                    let start_eq = s.pos;
                    s.advance();
                    if let Some(tok) = s.make_token(SyntaxTokenKind::Operator, start_eq, s.pos) {
                        tokens.push(tok);
                    }
                    continue;
                }
                // Attribute name
                let start_attr = s.pos;
                while let Some(c) = s.peek() {
                    if c.is_alphanumeric() || c == '-' || c == '_' || c == ':' || c == '@' || c == '#' || c == '.' {
                        s.advance();
                    } else {
                        break;
                    }
                }
                if start_attr < s.pos {
                    if let Some(tok) = s.make_token(SyntaxTokenKind::Attribute, start_attr, s.pos) {
                        tokens.push(tok);
                    }
                } else {
                    // Unknown single char inside tag
                    let start_char = s.pos;
                    s.advance();
                    if let Some(tok) = s.make_token(SyntaxTokenKind::Plain, start_char, s.pos) {
                        tokens.push(tok);
                    }
                }
            }
            continue;
        }

        // HTML entities &amp; &#123;
        if s.peek() == Some('&') {
            let start = s.pos;
            s.advance();
            while let Some(c) = s.peek() {
                if c.is_alphanumeric() || c == '#' {
                    s.advance();
                } else {
                    break;
                }
            }
            if s.peek() == Some(';') {
                s.advance();
                if let Some(tok) = s.make_token(SyntaxTokenKind::Constant, start, s.pos) {
                    tokens.push(tok);
                }
                continue;
            }
        }

        // Plain text content
        let start = s.pos;
        while let Some(c) = s.peek() {
            if c == '<' || c == '&' || c.is_whitespace() {
                break;
            }
            s.advance();
        }
        if start < s.pos {
            if let Some(tok) = s.make_token(SyntaxTokenKind::Plain, start, s.pos) {
                tokens.push(tok);
            }
        } else {
            let start_char = s.pos;
            s.advance();
            if let Some(tok) = s.make_token(SyntaxTokenKind::Plain, start_char, s.pos) {
                tokens.push(tok);
            }
        }
    }

    tokens
}

fn tokenize_css(content: &str) -> Vec<SyntaxToken> {
    let mut s = SourceScanner::new(content);
    let mut tokens = Vec::new();

    while !s.is_eof() {
        if let Some(ws) = s.consume_whitespace() {
            tokens.push(ws);
            continue;
        }

        // Block comment
        if s.starts_with_str("/*") {
            let start = s.pos;
            s.advance_n(2);
            while !s.is_eof() {
                if s.starts_with_str("*/") {
                    s.advance_n(2);
                    break;
                }
                s.advance();
            }
            if let Some(tok) = s.make_token(SyntaxTokenKind::Comment, start, s.pos) {
                tokens.push(tok);
            }
            continue;
        }

        // Line comment (SCSS)
        if s.starts_with_str("//") {
            let start = s.pos;
            s.advance_n(2);
            while let Some(c) = s.peek() {
                if c == '\n' {
                    break;
                }
                s.advance();
            }
            if let Some(tok) = s.make_token(SyntaxTokenKind::Comment, start, s.pos) {
                tokens.push(tok);
            }
            continue;
        }

        // At-rules @media, @import, @keyframes
        if s.peek() == Some('@') {
            let start = s.pos;
            s.advance();
            while let Some(c) = s.peek() {
                if c.is_alphanumeric() || c == '-' || c == '_' {
                    s.advance();
                } else {
                    break;
                }
            }
            if let Some(tok) = s.make_token(SyntaxTokenKind::Macro, start, s.pos) {
                tokens.push(tok);
            }
            continue;
        }

        // Hex colors #fff, #123456
        if s.peek() == Some('#') {
            let start = s.pos;
            s.advance();
            let mut hex_len = 0;
            while let Some(c) = s.peek() {
                if c.is_ascii_hexdigit() {
                    hex_len += 1;
                    s.advance();
                } else {
                    break;
                }
            }
            if hex_len == 3 || hex_len == 4 || hex_len == 6 || hex_len == 8 {
                if let Some(tok) = s.make_token(SyntaxTokenKind::Constant, start, s.pos) {
                    tokens.push(tok);
                }
                continue;
            } else {
                // ID selector
                while let Some(c) = s.peek() {
                    if c.is_alphanumeric() || c == '-' || c == '_' {
                        s.advance();
                    } else {
                        break;
                    }
                }
                if let Some(tok) = s.make_token(SyntaxTokenKind::Keyword, start, s.pos) {
                    tokens.push(tok);
                }
                continue;
            }
        }

        // Class selector .class_name
        if s.peek() == Some('.') && s.peek_nth(1).map_or(false, |c| c.is_alphabetic() || c == '-' || c == '_') {
            let start = s.pos;
            s.advance();
            while let Some(c) = s.peek() {
                if c.is_alphanumeric() || c == '-' || c == '_' {
                    s.advance();
                } else {
                    break;
                }
            }
            if let Some(tok) = s.make_token(SyntaxTokenKind::Keyword, start, s.pos) {
                tokens.push(tok);
            }
            continue;
        }

        // Strings
        if s.peek() == Some('"') || s.peek() == Some('\'') {
            let quote = s.advance().unwrap_or('"');
            let start = s.pos - 1;
            while let Some(c) = s.peek() {
                s.advance();
                if c == quote {
                    break;
                }
            }
            if let Some(tok) = s.make_token(SyntaxTokenKind::String, start, s.pos) {
                tokens.push(tok);
            }
            continue;
        }

        // Numbers with units: 10px, 1.5rem, 100%
        if s.peek().map_or(false, |c| c.is_ascii_digit()) {
            if let Some(num_tok) = s.consume_digits_and_number() {
                tokens.push(num_tok);
                continue;
            }
        }

        // Important keyword !important
        if s.starts_with_str("!important") {
            let start = s.pos;
            s.advance_n(10);
            if let Some(tok) = s.make_token(SyntaxTokenKind::Keyword, start, s.pos) {
                tokens.push(tok);
            }
            continue;
        }

        // Identifiers (properties, tag names, pseudo-classes)
        if s.peek().map_or(false, |c| c.is_alphabetic() || c == '-' || c == '_') {
            let start = s.pos;
            while let Some(c) = s.peek() {
                if c.is_alphanumeric() || c == '-' || c == '_' {
                    s.advance();
                } else {
                    break;
                }
            }
            let kind = if s.peek_next_non_whitespace() == Some(':') {
                SyntaxTokenKind::Attribute
            } else if s.peek_next_non_whitespace() == Some('(') {
                SyntaxTokenKind::Function
            } else {
                SyntaxTokenKind::Plain
            };

            if let Some(tok) = s.make_token(kind, start, s.pos) {
                tokens.push(tok);
            }
            continue;
        }

        // Punctuation & Operators
        let start = s.pos;
        let ch = s.advance();
        let kind = match ch {
            Some('{' | '}' | '(' | ')' | '[' | ']' | ';' | ':' | ',') => SyntaxTokenKind::Punctuation,
            Some('+' | '>' | '~' | '*' | '=' | '!') => SyntaxTokenKind::Operator,
            _ => SyntaxTokenKind::Plain,
        };
        if let Some(tok) = s.make_token(kind, start, s.pos) {
            tokens.push(tok);
        }
    }

    tokens
}

fn tokenize_json(content: &str) -> Vec<SyntaxToken> {
    let mut s = SourceScanner::new(content);
    let mut tokens = Vec::new();

    while !s.is_eof() {
        if let Some(ws) = s.consume_whitespace() {
            tokens.push(ws);
            continue;
        }

        // Strings / Keys
        if s.peek() == Some('"') {
            let start = s.pos;
            s.advance();
            let mut escaped = false;
            while let Some(c) = s.peek() {
                s.advance();
                if escaped {
                    escaped = false;
                } else if c == '\\' {
                    escaped = true;
                } else if c == '"' {
                    break;
                }
            }
            let kind = if s.peek_next_non_whitespace() == Some(':') {
                SyntaxTokenKind::Attribute
            } else {
                SyntaxTokenKind::String
            };
            if let Some(tok) = s.make_token(kind, start, s.pos) {
                tokens.push(tok);
            }
            continue;
        }

        // Numbers
        if s.peek().map_or(false, |c| c.is_ascii_digit() || c == '-') {
            if let Some(num_tok) = s.consume_digits_and_number() {
                tokens.push(num_tok);
                continue;
            }
        }

        // Constants: true, false, null
        if s.peek().map_or(false, |c| c.is_alphabetic()) {
            let start = s.pos;
            while let Some(c) = s.peek() {
                if c.is_alphabetic() {
                    s.advance();
                } else {
                    break;
                }
            }
            if let Some(tok) = s.make_token(SyntaxTokenKind::Constant, start, s.pos) {
                tokens.push(tok);
            }
            continue;
        }

        // Punctuation
        let start = s.pos;
        let ch = s.advance();
        let kind = match ch {
            Some('{' | '}' | '[' | ']' | ':' | ',') => SyntaxTokenKind::Punctuation,
            _ => SyntaxTokenKind::Plain,
        };
        if let Some(tok) = s.make_token(kind, start, s.pos) {
            tokens.push(tok);
        }
    }

    tokens
}

fn tokenize_yaml(content: &str) -> Vec<SyntaxToken> {
    let mut s = SourceScanner::new(content);
    let mut tokens = Vec::new();

    while !s.is_eof() {
        if let Some(ws) = s.consume_whitespace() {
            tokens.push(ws);
            continue;
        }

        // Comment
        if s.peek() == Some('#') {
            let start = s.pos;
            while let Some(c) = s.peek() {
                if c == '\n' {
                    break;
                }
                s.advance();
            }
            if let Some(tok) = s.make_token(SyntaxTokenKind::Comment, start, s.pos) {
                tokens.push(tok);
            }
            continue;
        }

        // Directives --- or ...
        if s.starts_with_str("---") || s.starts_with_str("...") {
            let start = s.pos;
            s.advance_n(3);
            if let Some(tok) = s.make_token(SyntaxTokenKind::Macro, start, s.pos) {
                tokens.push(tok);
            }
            continue;
        }

        // Strings "..." or '...'
        if s.peek() == Some('"') || s.peek() == Some('\'') {
            let quote = s.advance().unwrap_or('"');
            let start = s.pos - 1;
            let mut escaped = false;
            while let Some(c) = s.peek() {
                s.advance();
                if escaped {
                    escaped = false;
                } else if c == '\\' {
                    escaped = true;
                } else if c == quote {
                    break;
                }
            }
            let kind = if s.peek_next_non_whitespace() == Some(':') {
                SyntaxTokenKind::Attribute
            } else {
                SyntaxTokenKind::String
            };
            if let Some(tok) = s.make_token(kind, start, s.pos) {
                tokens.push(tok);
            }
            continue;
        }

        // Anchors & Aliases &anchor, *alias
        if s.peek() == Some('&') || s.peek() == Some('*') {
            let start = s.pos;
            s.advance();
            while let Some(c) = s.peek() {
                if c.is_alphanumeric() || c == '_' || c == '-' {
                    s.advance();
                } else {
                    break;
                }
            }
            if let Some(tok) = s.make_token(SyntaxTokenKind::Variable, start, s.pos) {
                tokens.push(tok);
            }
            continue;
        }

        // Numbers
        if s.peek().map_or(false, |c| c.is_ascii_digit()) {
            if let Some(num_tok) = s.consume_digits_and_number() {
                tokens.push(num_tok);
                continue;
            }
        }

        // Identifiers / Keys / Constants
        if s.peek().map_or(false, |c| c.is_alphanumeric() || c == '_' || c == '-') {
            let start = s.pos;
            while let Some(c) = s.peek() {
                if c.is_alphanumeric() || c == '_' || c == '-' {
                    s.advance();
                } else {
                    break;
                }
            }
            let ident_text = &s.source[s.char_to_byte[start]..s.char_to_byte[s.pos]];
            let lower = ident_text.to_lowercase();

            let kind = if s.peek_next_non_whitespace() == Some(':') {
                SyntaxTokenKind::Attribute
            } else if lower == "true" || lower == "false" || lower == "yes" || lower == "no" || lower == "null" || lower == "on" || lower == "off" {
                SyntaxTokenKind::Constant
            } else {
                SyntaxTokenKind::Plain
            };

            if let Some(tok) = s.make_token(kind, start, s.pos) {
                tokens.push(tok);
            }
            continue;
        }

        // Operators & Punctuation
        let start = s.pos;
        let ch = s.advance();
        let kind = match ch {
            Some('-' | '|' | '>' | '?') => SyntaxTokenKind::Operator,
            Some(':' | '{' | '}' | '[' | ']' | ',') => SyntaxTokenKind::Punctuation,
            _ => SyntaxTokenKind::Plain,
        };
        if let Some(tok) = s.make_token(kind, start, s.pos) {
            tokens.push(tok);
        }
    }

    tokens
}

fn tokenize_toml(content: &str) -> Vec<SyntaxToken> {
    let mut s = SourceScanner::new(content);
    let mut tokens = Vec::new();

    while !s.is_eof() {
        if let Some(ws) = s.consume_whitespace() {
            tokens.push(ws);
            continue;
        }

        // Comment
        if s.peek() == Some('#') {
            let start = s.pos;
            while let Some(c) = s.peek() {
                if c == '\n' {
                    break;
                }
                s.advance();
            }
            if let Some(tok) = s.make_token(SyntaxTokenKind::Comment, start, s.pos) {
                tokens.push(tok);
            }
            continue;
        }

        // Table headers [[array]] or [section]
        if s.peek() == Some('[') {
            let start = s.pos;
            while let Some(c) = s.peek() {
                s.advance();
                if c == ']' {
                    if s.peek() == Some(']') {
                        s.advance();
                    }
                    break;
                } else if c == '\n' {
                    break;
                }
            }
            if let Some(tok) = s.make_token(SyntaxTokenKind::Tag, start, s.pos) {
                tokens.push(tok);
            }
            continue;
        }

        // Multi-line strings """...""" or '''...'''
        if s.starts_with_str("\"\"\"") || s.starts_with_str("'''") {
            let start = s.pos;
            let delimiter = if s.starts_with_str("\"\"\"") { "\"\"\"" } else { "'''" };
            s.advance_n(3);
            while !s.is_eof() {
                if s.starts_with_str(delimiter) {
                    s.advance_n(3);
                    break;
                }
                s.advance();
            }
            if let Some(tok) = s.make_token(SyntaxTokenKind::String, start, s.pos) {
                tokens.push(tok);
            }
            continue;
        }

        // Normal strings "..." or '...'
        if s.peek() == Some('"') || s.peek() == Some('\'') {
            let quote = s.advance().unwrap_or('"');
            let start = s.pos - 1;
            let mut escaped = false;
            while let Some(c) = s.peek() {
                s.advance();
                if escaped {
                    escaped = false;
                } else if c == '\\' {
                    escaped = true;
                } else if c == quote {
                    break;
                }
            }
            if let Some(tok) = s.make_token(SyntaxTokenKind::String, start, s.pos) {
                tokens.push(tok);
            }
            continue;
        }

        // Numbers / Dates
        if s.peek().map_or(false, |c| c.is_ascii_digit()) {
            if let Some(num_tok) = s.consume_digits_and_number() {
                tokens.push(num_tok);
                continue;
            }
        }

        // Keys / Constants
        if s.peek().map_or(false, |c| c.is_alphanumeric() || c == '_' || c == '-') {
            let start = s.pos;
            while let Some(c) = s.peek() {
                if c.is_alphanumeric() || c == '_' || c == '-' {
                    s.advance();
                } else {
                    break;
                }
            }
            let ident_text = &s.source[s.char_to_byte[start]..s.char_to_byte[s.pos]];
            let kind = if ident_text == "true" || ident_text == "false" {
                SyntaxTokenKind::Constant
            } else if s.peek_next_non_whitespace() == Some('=') || s.peek_next_non_whitespace() == Some('.') {
                SyntaxTokenKind::Attribute
            } else {
                SyntaxTokenKind::Plain
            };
            if let Some(tok) = s.make_token(kind, start, s.pos) {
                tokens.push(tok);
            }
            continue;
        }

        // Operators and punctuation
        let start = s.pos;
        let ch = s.advance();
        let kind = match ch {
            Some('=') => SyntaxTokenKind::Operator,
            Some('{' | '}' | '[' | ']' | ',' | '.') => SyntaxTokenKind::Punctuation,
            _ => SyntaxTokenKind::Plain,
        };
        if let Some(tok) = s.make_token(kind, start, s.pos) {
            tokens.push(tok);
        }
    }

    tokens
}

const SHELL_KEYWORDS: &[&str] = &[
    "if", "then", "else", "elif", "fi", "for", "in", "do", "done", "while",
    "until", "case", "esac", "function", "select", "time",
];

const SHELL_COMMANDS: &[&str] = &[
    "echo", "cd", "pwd", "export", "set", "unset", "source", "alias", "eval",
    "exec", "exit", "read", "shift", "test", "local", "declare", "typeset",
    "readonly", "trap", "kill", "wait", "cat", "grep", "sed", "awk", "find",
    "mkdir", "rm", "cp", "mv", "ls", "chmod", "chown", "curl", "wget", "git",
    "sudo", "docker", "cargo", "npm", "yarn", "pnpm", "bun", "node", "python", "pip",
];

fn tokenize_shell(content: &str) -> Vec<SyntaxToken> {
    let mut s = SourceScanner::new(content);
    let mut tokens = Vec::new();

    while !s.is_eof() {
        if let Some(ws) = s.consume_whitespace() {
            tokens.push(ws);
            continue;
        }

        // Shebang #!/bin/bash
        if s.starts_with_str("#!") {
            let start = s.pos;
            while let Some(c) = s.peek() {
                if c == '\n' {
                    break;
                }
                s.advance();
            }
            if let Some(tok) = s.make_token(SyntaxTokenKind::Macro, start, s.pos) {
                tokens.push(tok);
            }
            continue;
        }

        // Comment
        if s.peek() == Some('#') {
            let start = s.pos;
            while let Some(c) = s.peek() {
                if c == '\n' {
                    break;
                }
                s.advance();
            }
            if let Some(tok) = s.make_token(SyntaxTokenKind::Comment, start, s.pos) {
                tokens.push(tok);
            }
            continue;
        }

        // Variables: $VAR, ${VAR}, $1, $@, $?, $$
        if s.peek() == Some('$') {
            let start = s.pos;
            s.advance();
            if s.peek() == Some('{') {
                s.advance();
                while let Some(c) = s.peek() {
                    s.advance();
                    if c == '}' {
                        break;
                    }
                }
            } else if s.peek() == Some('(') {
                // Subshell command $(command)
                s.advance();
                let mut depth = 1usize;
                while let Some(c) = s.peek() {
                    s.advance();
                    if c == '(' {
                        depth += 1;
                    } else if c == ')' {
                        depth -= 1;
                        if depth == 0 {
                            break;
                        }
                    }
                }
            } else {
                while let Some(c) = s.peek() {
                    if c.is_alphanumeric() || c == '_' || c == '?' || c == '!' || c == '@' || c == '*' || c == '#' || c == '$' {
                        s.advance();
                    } else {
                        break;
                    }
                }
            }
            if let Some(tok) = s.make_token(SyntaxTokenKind::Variable, start, s.pos) {
                tokens.push(tok);
            }
            continue;
        }

        // Strings "..." or '...'
        if s.peek() == Some('"') || s.peek() == Some('\'') {
            let quote = s.advance().unwrap_or('"');
            let start = s.pos - 1;
            let mut escaped = false;
            while let Some(c) = s.peek() {
                s.advance();
                if escaped {
                    escaped = false;
                } else if c == '\\' {
                    escaped = true;
                } else if c == quote {
                    break;
                }
            }
            if let Some(tok) = s.make_token(SyntaxTokenKind::String, start, s.pos) {
                tokens.push(tok);
            }
            continue;
        }

        // Numbers
        if s.peek().map_or(false, |c| c.is_ascii_digit()) {
            let start = s.pos;
            while let Some(c) = s.peek() {
                if c.is_ascii_digit() {
                    s.advance();
                } else {
                    break;
                }
            }
            if let Some(tok) = s.make_token(SyntaxTokenKind::Number, start, s.pos) {
                tokens.push(tok);
            }
            continue;
        }

        // Identifiers, Keywords, Builtins, Commands
        if s.peek().map_or(false, |c| c.is_alphabetic() || c == '_' || c == '-') {
            let start = s.pos;
            while let Some(c) = s.peek() {
                if c.is_alphanumeric() || c == '_' || c == '-' {
                    s.advance();
                } else {
                    break;
                }
            }
            let ident_text = &s.source[s.char_to_byte[start]..s.char_to_byte[s.pos]];
            let kind = if SHELL_KEYWORDS.contains(&ident_text) {
                SyntaxTokenKind::Keyword
            } else if SHELL_COMMANDS.contains(&ident_text) {
                SyntaxTokenKind::Function
            } else if ident_text.starts_with('-') {
                SyntaxTokenKind::Attribute // CLI flags e.g. -rf, --help
            } else {
                SyntaxTokenKind::Plain
            };
            if let Some(tok) = s.make_token(kind, start, s.pos) {
                tokens.push(tok);
            }
            continue;
        }

        // Multi-character operators
        let multi_ops = &["||", "&&", ">>", "<<", "2>&1", ";;", "=="];
        let mut matched_op = false;
        for op in multi_ops {
            if s.starts_with_str(op) {
                let start = s.pos;
                s.advance_n(op.chars().count());
                if let Some(tok) = s.make_token(SyntaxTokenKind::Operator, start, s.pos) {
                    tokens.push(tok);
                }
                matched_op = true;
                break;
            }
        }
        if matched_op {
            continue;
        }

        // Single character operators and punctuation
        let start = s.pos;
        let ch = s.advance();
        let kind = match ch {
            Some('|' | '&' | '>' | '<' | '=' | '!') => SyntaxTokenKind::Operator,
            Some('(' | ')' | '{' | '}' | '[' | ']' | ';' | ',') => SyntaxTokenKind::Punctuation,
            _ => SyntaxTokenKind::Plain,
        };
        if let Some(tok) = s.make_token(kind, start, s.pos) {
            tokens.push(tok);
        }
    }

    tokens
}

const SQL_KEYWORDS: &[&str] = &[
    "select", "from", "where", "insert", "into", "values", "update", "set", "delete",
    "join", "inner", "left", "right", "full", "outer", "cross", "on", "group", "by",
    "having", "order", "asc", "desc", "limit", "offset", "union", "all", "distinct",
    "create", "table", "drop", "alter", "index", "view", "primary", "key", "foreign",
    "references", "constraint", "default", "check", "unique", "not", "null", "and",
    "or", "in", "is", "like", "ilike", "between", "exists", "case", "when", "then",
    "else", "end", "cast", "as", "with", "recursive", "truncate", "begin", "commit",
    "rollback", "transaction",
];

const SQL_TYPES: &[&str] = &[
    "int", "integer", "bigint", "smallint", "varchar", "text", "char", "boolean",
    "date", "timestamp", "time", "float", "double", "decimal", "numeric", "blob",
    "json", "jsonb", "serial",
];

const SQL_FUNCTIONS: &[&str] = &[
    "count", "sum", "avg", "min", "max", "coalesce", "now", "length", "concat",
    "lower", "upper", "substring",
];

fn tokenize_sql(content: &str) -> Vec<SyntaxToken> {
    let mut s = SourceScanner::new(content);
    let mut tokens = Vec::new();

    while !s.is_eof() {
        if let Some(ws) = s.consume_whitespace() {
            tokens.push(ws);
            continue;
        }

        // Line comment --
        if s.starts_with_str("--") {
            let start = s.pos;
            s.advance_n(2);
            while let Some(c) = s.peek() {
                if c == '\n' {
                    break;
                }
                s.advance();
            }
            if let Some(tok) = s.make_token(SyntaxTokenKind::Comment, start, s.pos) {
                tokens.push(tok);
            }
            continue;
        }

        // Block comment /* ... */
        if s.starts_with_str("/*") {
            let start = s.pos;
            s.advance_n(2);
            while !s.is_eof() {
                if s.starts_with_str("*/") {
                    s.advance_n(2);
                    break;
                }
                s.advance();
            }
            if let Some(tok) = s.make_token(SyntaxTokenKind::Comment, start, s.pos) {
                tokens.push(tok);
            }
            continue;
        }

        // String literals '...'
        if s.peek() == Some('\'') || s.peek() == Some('"') {
            let quote = s.advance().unwrap_or('\'');
            let start = s.pos - 1;
            while let Some(c) = s.peek() {
                s.advance();
                if c == quote {
                    if s.peek() == Some(quote) {
                        s.advance(); // escaped quote ''
                    } else {
                        break;
                    }
                }
            }
            if let Some(tok) = s.make_token(SyntaxTokenKind::String, start, s.pos) {
                tokens.push(tok);
            }
            continue;
        }

        // Numbers
        if s.peek().map_or(false, |c| c.is_ascii_digit()) {
            if let Some(num_tok) = s.consume_digits_and_number() {
                tokens.push(num_tok);
                continue;
            }
        }

        // Identifiers, Keywords, Types, Functions
        if s.peek().map_or(false, |c| c.is_alphabetic() || c == '_') {
            let start = s.pos;
            while let Some(c) = s.peek() {
                if c.is_alphanumeric() || c == '_' {
                    s.advance();
                } else {
                    break;
                }
            }
            let ident_text = &s.source[s.char_to_byte[start]..s.char_to_byte[s.pos]];
            let lower = ident_text.to_lowercase();

            let kind = if SQL_KEYWORDS.contains(&lower.as_str()) {
                SyntaxTokenKind::Keyword
            } else if SQL_TYPES.contains(&lower.as_str()) {
                SyntaxTokenKind::Type
            } else if SQL_FUNCTIONS.contains(&lower.as_str()) {
                SyntaxTokenKind::Function
            } else {
                SyntaxTokenKind::Plain
            };
            if let Some(tok) = s.make_token(kind, start, s.pos) {
                tokens.push(tok);
            }
            continue;
        }

        // Punctuation & Operators
        let start = s.pos;
        let ch = s.advance();
        let kind = match ch {
            Some('=' | '<' | '>' | '!' | '+' | '-' | '*' | '/' | '%') => SyntaxTokenKind::Operator,
            Some('(' | ')' | ',' | ';' | '.') => SyntaxTokenKind::Punctuation,
            _ => SyntaxTokenKind::Plain,
        };
        if let Some(tok) = s.make_token(kind, start, s.pos) {
            tokens.push(tok);
        }
    }

    tokens
}

fn tokenize_markdown(content: &str) -> Vec<SyntaxToken> {
    let mut s = SourceScanner::new(content);
    let mut tokens = Vec::new();

    while !s.is_eof() {
        if let Some(ws) = s.consume_whitespace() {
            tokens.push(ws);
            continue;
        }

        // Headings: # ...
        if s.peek() == Some('#') {
            let start = s.pos;
            while let Some(c) = s.peek() {
                if c == '\n' {
                    break;
                }
                s.advance();
            }
            if let Some(tok) = s.make_token(SyntaxTokenKind::Keyword, start, s.pos) {
                tokens.push(tok);
            }
            continue;
        }

        // Code fence ```...```
        if s.starts_with_str("```") {
            let start = s.pos;
            s.advance_n(3);
            while !s.is_eof() {
                if s.starts_with_str("```") {
                    s.advance_n(3);
                    break;
                }
                s.advance();
            }
            if let Some(tok) = s.make_token(SyntaxTokenKind::String, start, s.pos) {
                tokens.push(tok);
            }
            continue;
        }

        // Inline code `...`
        if s.peek() == Some('`') {
            let start = s.pos;
            s.advance();
            while let Some(c) = s.peek() {
                s.advance();
                if c == '`' {
                    break;
                }
            }
            if let Some(tok) = s.make_token(SyntaxTokenKind::String, start, s.pos) {
                tokens.push(tok);
            }
            continue;
        }

        // Blockquotes >
        if s.peek() == Some('>') {
            let start = s.pos;
            while let Some(c) = s.peek() {
                if c == '\n' {
                    break;
                }
                s.advance();
            }
            if let Some(tok) = s.make_token(SyntaxTokenKind::Comment, start, s.pos) {
                tokens.push(tok);
            }
            continue;
        }

        // Normal word or punctuation
        let start = s.pos;
        let ch = s.advance();
        let kind = match ch {
            Some('*' | '_' | '~') => SyntaxTokenKind::Constant,
            Some('[' | ']' | '(' | ')') => SyntaxTokenKind::Punctuation,
            _ => SyntaxTokenKind::Plain,
        };
        if let Some(tok) = s.make_token(kind, start, s.pos) {
            tokens.push(tok);
        }
    }

    tokens
}

fn tokenize_generic(content: &str) -> Vec<SyntaxToken> {
    let mut s = SourceScanner::new(content);
    let mut tokens = Vec::new();

    while !s.is_eof() {
        if let Some(ws) = s.consume_whitespace() {
            tokens.push(ws);
            continue;
        }

        // Comments
        if s.starts_with_str("//") || s.starts_with_str("#") || s.starts_with_str("--") {
            let start = s.pos;
            while let Some(c) = s.peek() {
                if c == '\n' {
                    break;
                }
                s.advance();
            }
            if let Some(tok) = s.make_token(SyntaxTokenKind::Comment, start, s.pos) {
                tokens.push(tok);
            }
            continue;
        }

        // Strings
        if s.peek() == Some('"') || s.peek() == Some('\'') {
            let quote = s.advance().unwrap_or('"');
            let start = s.pos - 1;
            let mut escaped = false;
            while let Some(c) = s.peek() {
                s.advance();
                if escaped {
                    escaped = false;
                } else if c == '\\' {
                    escaped = true;
                } else if c == quote {
                    break;
                }
            }
            if let Some(tok) = s.make_token(SyntaxTokenKind::String, start, s.pos) {
                tokens.push(tok);
            }
            continue;
        }

        // Numbers
        if s.peek().map_or(false, |c| c.is_ascii_digit()) {
            if let Some(num_tok) = s.consume_digits_and_number() {
                tokens.push(num_tok);
                continue;
            }
        }

        // Identifiers
        if s.peek().map_or(false, |c| c.is_alphabetic() || c == '_') {
            let start = s.pos;
            while let Some(c) = s.peek() {
                if c.is_alphanumeric() || c == '_' {
                    s.advance();
                } else {
                    break;
                }
            }
            if let Some(tok) = s.make_token(SyntaxTokenKind::Plain, start, s.pos) {
                tokens.push(tok);
            }
            continue;
        }

        let start = s.pos;
        let ch = s.advance();
        let kind = match ch {
            Some('+' | '-' | '*' | '/' | '=' | '<' | '>') => SyntaxTokenKind::Operator,
            Some('(' | ')' | '{' | '}' | '[' | ']' | ';' | ',' | '.' | ':') => SyntaxTokenKind::Punctuation,
            _ => SyntaxTokenKind::Plain,
        };
        if let Some(tok) = s.make_token(kind, start, s.pos) {
            tokens.push(tok);
        }
    }

    tokens
}

/// Tokenize source code for any supported programming or markup language.
pub fn tokenize(content: &str, lang: SupportedLanguage) -> Vec<SyntaxToken> {
    match lang {
        SupportedLanguage::Rust => tokenize_rust(content),
        SupportedLanguage::JavaScript
        | SupportedLanguage::TypeScript
        | SupportedLanguage::Jsx
        | SupportedLanguage::Tsx => tokenize_javascript_typescript(content, lang),
        SupportedLanguage::Python => tokenize_python(content),
        SupportedLanguage::Go => tokenize_go(content),
        SupportedLanguage::C | SupportedLanguage::Cpp | SupportedLanguage::CSharp | SupportedLanguage::Java => {
            tokenize_c_cpp_family(content, lang)
        }
        SupportedLanguage::Html | SupportedLanguage::Xml => {
            tokenize_html_xml(content, lang == SupportedLanguage::Xml)
        }
        SupportedLanguage::Css | SupportedLanguage::Scss => tokenize_css(content),
        SupportedLanguage::Json => tokenize_json(content),
        SupportedLanguage::Yaml => tokenize_yaml(content),
        SupportedLanguage::Toml => tokenize_toml(content),
        SupportedLanguage::Shell => tokenize_shell(content),
        SupportedLanguage::Sql => tokenize_sql(content),
        SupportedLanguage::Markdown => tokenize_markdown(content),
        SupportedLanguage::Ruby | SupportedLanguage::Lua | SupportedLanguage::Unknown => {
            tokenize_generic(content)
        }
    }
}

/// Tokenize a single line of code.
pub fn tokenize_line(line: &str, lang: SupportedLanguage) -> Vec<SyntaxToken> {
    tokenize(line, lang)
}

/// High-level pure-Rust syntax highlighter producing TrueColor ANSI terminal text.
pub fn highlight(content: &str, lang: SupportedLanguage) -> String {
    highlight_with_theme(content, lang, &HighlightTheme::dark(), ColorMode::TrueColor)
}

/// Highlights code string given a language hint name (e.g. `"rust"`, `"python"`, `"json"`).
pub fn highlight_code(content: &str, lang_hint: &str) -> String {
    let lang = SupportedLanguage::from_str_hint(lang_hint);
    highlight(content, lang)
}

/// Highlights code string with specific theme and color mode.
pub fn highlight_with_theme(
    content: &str,
    lang: SupportedLanguage,
    theme: &HighlightTheme,
    mode: ColorMode,
) -> String {
    if mode == ColorMode::Plain || content.is_empty() {
        return content.to_string();
    }

    let tokens = tokenize(content, lang);
    let mut out = String::with_capacity(content.len() * 2);
    for token in &tokens {
        out.push_str(&theme.format_token(token, mode));
    }
    out
}

/// Highlights a single line of source code with default dark theme and TrueColor ANSI.
pub fn highlight_line(line: &str, lang: SupportedLanguage) -> String {
    highlight_line_with_theme(line, lang, &HighlightTheme::dark(), ColorMode::TrueColor)
}

/// Highlights a single line of source code with custom theme and color mode.
pub fn highlight_line_with_theme(
    line: &str,
    lang: SupportedLanguage,
    theme: &HighlightTheme,
    mode: ColorMode,
) -> String {
    if mode == ColorMode::Plain || line.is_empty() {
        return line.to_string();
    }

    let tokens = tokenize_line(line, lang);
    let mut out = String::with_capacity(line.len() * 2);
    for token in &tokens {
        out.push_str(&theme.format_token(token, mode));
    }
    out
}

/// Formats code with line numbers and syntax highlighting.
pub fn highlight_with_line_numbers(
    content: &str,
    lang: SupportedLanguage,
    start_line: usize,
    theme: &HighlightTheme,
    mode: ColorMode,
) -> String {
    let lines: Vec<&str> = content.lines().collect();
    let total_lines = lines.len();
    let max_line_no = start_line + total_lines;
    let width = max_line_no.to_string().len().max(3);

    let mut out = String::with_capacity(content.len() * 2);
    for (idx, line) in lines.iter().enumerate() {
        let line_no = start_line + idx;
        let highlighted = highlight_line_with_theme(line, lang, theme, mode);
        let gutter = match mode {
            ColorMode::Plain => format!("{:>width$} │ ", line_no, width = width),
            ColorMode::TrueColor | ColorMode::Ansi256 | ColorMode::Ansi16 => {
                format!("\x1b[90m{:>width$} │\x1b[0m ", line_no, width = width)
            }
        };
        out.push_str(&gutter);
        out.push_str(&highlighted);
        if idx + 1 < total_lines || content.ends_with('\n') {
            out.push('\n');
        }
    }
    out
}

/// Automatically detects language from file path or content and renders highlighted ANSI output.
pub fn highlight_auto(content: &str, file_path: Option<&Path>, hint: Option<&str>) -> String {
    let lang = detect_language(file_path, content, hint);
    highlight(content, lang)
}

// ---------------------------------------------------------------------------
// SyntaxCheckTool Tool Implementation
// ---------------------------------------------------------------------------

#[derive(Default, Debug, Clone)]
pub struct SyntaxCheckTool;

impl SyntaxCheckTool {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl Tool for SyntaxCheckTool {
    fn name(&self) -> &str {
        "syntax_check"
    }

    fn description(&self) -> &str {
        "Validate code syntax, bracket/brace/parenthesis balancing, quotation integrity, indentation consistency, and generate pure-Rust ANSI syntax highlighted terminal output."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "Path to the source file to validate (optional if content is supplied)."
                },
                "content": {
                    "type": "string",
                    "description": "Raw source code content to validate directly (optional if path is supplied)."
                },
                "language": {
                    "type": "string",
                    "description": "Language hint (e.g. 'rust', 'python', 'javascript', 'typescript', 'json', 'yaml', 'toml', 'html', 'c', 'cpp', 'auto')."
                },
                "highlight": {
                    "type": "boolean",
                    "description": "Whether to return ANSI syntax-highlighted code output (optional, default: false)."
                },
                "theme": {
                    "type": "string",
                    "enum": ["dark", "monokai", "one_dark", "light", "nord", "solarized_dark"],
                    "description": "Color theme for syntax highlighting (optional, default: 'dark')."
                },
                "color_mode": {
                    "type": "string",
                    "enum": ["truecolor", "ansi256", "ansi16", "plain"],
                    "description": "Terminal color format mode (optional, default: 'truecolor')."
                },
                "line_numbers": {
                    "type": "boolean",
                    "description": "Whether to display line numbers with highlighted code (optional, default: false)."
                },
                "check_indentation": {
                    "type": "boolean",
                    "description": "Whether to check indentation consistency and mixed tabs/spaces (optional, default: true)."
                },
                "check_brackets": {
                    "type": "boolean",
                    "description": "Whether to verify bracket, brace, and parenthesis balance (optional, default: true)."
                },
                "check_quotes": {
                    "type": "boolean",
                    "description": "Whether to check string literals and quotation marks (optional, default: true)."
                },
                "format": {
                    "type": "string",
                    "enum": ["text", "json", "highlight", "ansi"],
                    "description": "Output report format: 'text' (default), 'json', 'highlight', or 'ansi'."
                }
            }
        })
    }

    async fn execute(&self, args: Value, ctx: &ToolContext) -> anyhow::Result<String> {
        let path_opt = args
            .get("path")
            .and_then(|v| v.as_str())
            .or_else(|| args.get("file_path").and_then(|v| v.as_str()));

        let content_opt = args.get("content").and_then(|v| v.as_str());

        let (content, resolved_path) = match (path_opt, content_opt) {
            (Some(path_str), None) => {
                let full_path = resolve_path(path_str, &ctx.cwd);
                if !full_path.exists() {
                    anyhow::bail!("File not found: '{}'", full_path.display());
                }
                if full_path.is_dir() {
                    anyhow::bail!("Path is a directory, not a file: '{}'", full_path.display());
                }
                let bytes = tokio::fs::read(&full_path)
                    .await
                    .map_err(|e| anyhow::anyhow!("Failed to read file '{}': {e}", full_path.display()))?;
                let text = String::from_utf8_lossy(&bytes).to_string();
                (text, Some(full_path))
            }
            (_, Some(raw_text)) => {
                let p = path_opt.map(|ps| resolve_path(ps, &ctx.cwd));
                (raw_text.to_string(), p)
            }
            (None, None) => {
                anyhow::bail!("Either 'path' or 'content' must be provided to syntax_check");
            }
        };

        let lang_hint = args.get("language").and_then(|v| v.as_str());
        let lang = detect_language(resolved_path.as_deref(), &content, lang_hint);

        let format = args.get("format").and_then(|v| v.as_str()).unwrap_or("text");
        let should_highlight = args.get("highlight").and_then(|v| v.as_bool()).unwrap_or(false)
            || format == "highlight"
            || format == "ansi";

        if should_highlight {
            let theme_str = args.get("theme").and_then(|v| v.as_str()).unwrap_or("dark");
            let theme = HighlightTheme::from_name(theme_str);
            let mode_str = args.get("color_mode").and_then(|v| v.as_str()).unwrap_or("truecolor");
            let color_mode = ColorMode::from_str_hint(mode_str);
            let line_numbers = args.get("line_numbers").and_then(|v| v.as_bool()).unwrap_or(false);

            let highlighter = SyntaxHighlighter::new()
                .with_theme(theme)
                .with_color_mode(color_mode)
                .with_line_numbers(line_numbers);

            return Ok(highlighter.highlight(&content, lang));
        }

        let options = SyntaxCheckOptions {
            check_indentation: args.get("check_indentation").and_then(|v| v.as_bool()).unwrap_or(true),
            check_brackets: args.get("check_brackets").and_then(|v| v.as_bool()).unwrap_or(true),
            check_quotes: args.get("check_quotes").and_then(|v| v.as_bool()).unwrap_or(true),
            language_override: lang_hint.map(|s| s.to_string()),
        };

        let report = validate_syntax(&content, lang, resolved_path.as_deref(), &options);

        if format == "json" {
            serde_json::to_string_pretty(&report)
                .map_err(|e| anyhow::anyhow!("Failed to serialize report: {e}"))
        } else {
            Ok(format_report_text(&report, &content))
        }
    }
}
// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_rust_bracket_balance_valid() {
        let code = r#"
fn main() {
    let items = vec![1, 2, 3];
    if items.is_empty() {
        println!("empty: {}", items.len());
    }
}
"#;
        let report = validate_syntax(code, SupportedLanguage::Rust, None, &SyntaxCheckOptions::default());
        assert!(report.valid, "Report should be valid: {:?}", report.issues);
        assert_eq!(report.error_count, 0);
    }

    #[test]
    fn test_rust_mismatched_brackets() {
        let code = "fn test() { let arr = [1, 2, 3}; }";
        let report = validate_syntax(code, SupportedLanguage::Rust, None, &SyntaxCheckOptions::default());
        assert!(!report.valid);
        assert_eq!(report.error_count, 1);
        assert_eq!(report.issues[0].rule, "mismatched-delimiter");
    }

    #[test]
    fn test_rust_unclosed_delimiter() {
        let code = "fn test() { let arr = [1, 2, 3; }";
        let report = validate_syntax(code, SupportedLanguage::Rust, None, &SyntaxCheckOptions::default());
        assert!(!report.valid);
        assert!(report.issues.iter().any(|i| i.rule == "mismatched-delimiter" || i.rule == "unclosed-delimiter"));
    }

    #[test]
    fn test_rust_raw_strings_and_lifetimes() {
        let code = r##"
fn lifetime<'a>(s: &'a str) -> &'a str {
    let raw = r#"hello "world" { brackets inside } #"#;
    let ch = 'x';
    s
}
"##;
        let report = validate_syntax(code, SupportedLanguage::Rust, None, &SyntaxCheckOptions::default());
        assert!(report.valid, "Report should handle raw strings and lifetimes: {:?}", report.issues);
    }

    #[test]
    fn test_rust_nested_block_comments() {
        let code = r#"
/* Outer comment
   /* Nested comment */
   Still comment */
fn valid() {}
"#;
        let report = validate_syntax(code, SupportedLanguage::Rust, None, &SyntaxCheckOptions::default());
        assert!(report.valid, "Nested comments should be supported in Rust");
    }

    #[test]
    fn test_python_indentation_block() {
        let valid_py = r#"
def calculate(a, b):
    if a > b:
        return a - b
    else:
        return b - a
"#;
        let report = validate_syntax(valid_py, SupportedLanguage::Python, None, &SyntaxCheckOptions::default());
        assert!(report.valid, "Valid Python indentation failed: {:?}", report.issues);

        let invalid_py = r#"
def calculate(a, b):
if a > b:
    return a
"#;
        let report2 = validate_syntax(invalid_py, SupportedLanguage::Python, None, &SyntaxCheckOptions::default());
        assert!(!report2.valid);
        assert!(report2.issues.iter().any(|i| i.rule == "expected-indented-block"));
    }

    #[test]
    fn test_python_unindent_mismatch() {
        let py = r#"
def foo():
    if True:
        x = 1
      y = 2
"#;
        let report = validate_syntax(py, SupportedLanguage::Python, None, &SyntaxCheckOptions::default());
        assert!(!report.valid);
        assert!(report.issues.iter().any(|i| i.rule == "unindent-mismatch"));
    }

    #[test]
    fn test_python_triple_quotes() {
        let py = r#"
def doc():
    """This is a docstring
    with (parentheses and {braces} inside)
    """
    return 42
"#;
        let report = validate_syntax(py, SupportedLanguage::Python, None, &SyntaxCheckOptions::default());
        assert!(report.valid, "Python triple quotes should not leak braces: {:?}", report.issues);
    }

    #[test]
    fn test_javascript_template_literals() {
        let js = r#"
const msg = `Hello ${user.getName({ formal: true })}!`;
console.log(msg);
"#;
        let report = validate_syntax(js, SupportedLanguage::JavaScript, None, &SyntaxCheckOptions::default());
        assert!(report.valid, "JS template literal with nested expr failed: {:?}", report.issues);
    }

    #[test]
    fn test_json_validation() {
        let valid_json = r#"{"name": "fusion", "version": 2, "tools": ["syntax", "read"]}"#;
        let report = validate_syntax(valid_json, SupportedLanguage::Json, None, &SyntaxCheckOptions::default());
        assert!(report.valid);

        let invalid_json = r#"{"name": "fusion", "trailing": 1, }"#;
        let report2 = validate_syntax(invalid_json, SupportedLanguage::Json, None, &SyntaxCheckOptions::default());
        assert!(!report2.valid);
        assert!(report2.issues.iter().any(|i| i.rule == "invalid-json"));
    }

    #[test]
    fn test_html_tag_balancing() {
        let valid_html = r#"
<div class="container">
    <img src="logo.png" />
    <br>
    <p>Hello <span>World</span></p>
</div>
"#;
        let report = validate_syntax(valid_html, SupportedLanguage::Html, None, &SyntaxCheckOptions::default());
        assert!(report.valid, "Valid HTML failed: {:?}", report.issues);

        let mismatched_html = r#"
<div>
    <span>Mismatched</div>
</span>
"#;
        let report2 = validate_syntax(mismatched_html, SupportedLanguage::Html, None, &SyntaxCheckOptions::default());
        assert!(!report2.valid);
        assert!(report2.issues.iter().any(|i| i.rule == "tag-mismatch"));
    }

    #[test]
    fn test_yaml_tab_error() {
        let yaml_with_tabs = "key:\n\tvalue: 1\n";
        let report = validate_syntax(yaml_with_tabs, SupportedLanguage::Yaml, None, &SyntaxCheckOptions::default());
        assert!(!report.valid);
        assert!(report.issues.iter().any(|i| i.rule == "yaml-tab-indentation"));
    }

    #[test]
    fn test_mixed_indentation_warning() {
        let mixed = "fn main() {\n \tlet x = 1;\n}\n";
        let report = validate_syntax(mixed, SupportedLanguage::Rust, None, &SyntaxCheckOptions::default());
        assert!(report.warning_count > 0);
        assert!(report.issues.iter().any(|i| i.rule == "mixed-indentation"));
    }

    // -----------------------------------------------------------------------
    // Tokenization Tests
    // -----------------------------------------------------------------------

    /// Helper: collect (kind, text) pairs, filtering whitespace-only Plain tokens.
    fn tok_pairs(tokens: &[SyntaxToken]) -> Vec<(SyntaxTokenKind, &str)> {
        tokens
            .iter()
            .filter(|t| !(t.kind == SyntaxTokenKind::Plain && t.text.trim().is_empty()))
            .map(|t| (t.kind, t.text.as_str()))
            .collect()
    }

    /// Helper: assert a specific kind appears in the token stream for a given text.
    fn assert_has_token(tokens: &[SyntaxToken], kind: SyntaxTokenKind, text: &str) {
        assert!(
            tokens.iter().any(|t| t.kind == kind && t.text == text),
            "Expected token ({:?}, {:?}) not found in: {:?}",
            kind,
            text,
            tok_pairs(tokens),
        );
    }

    #[test]
    fn test_tokenize_rust_keywords_and_types() {
        let code = "fn main() { let x: u32 = 42; }";
        let tokens = tokenize(code, SupportedLanguage::Rust);
        assert_has_token(&tokens, SyntaxTokenKind::Keyword, "fn");
        assert_has_token(&tokens, SyntaxTokenKind::Keyword, "let");
        assert_has_token(&tokens, SyntaxTokenKind::Type, "u32");
        assert_has_token(&tokens, SyntaxTokenKind::Number, "42");
        assert_has_token(&tokens, SyntaxTokenKind::Function, "main");
    }

    #[test]
    fn test_tokenize_rust_strings_and_comments() {
        let code = r#"let s = "hello"; // line comment"#;
        let tokens = tokenize(code, SupportedLanguage::Rust);
        assert_has_token(&tokens, SyntaxTokenKind::String, "\"hello\"");
        assert!(
            tokens.iter().any(|t| t.kind == SyntaxTokenKind::Comment && t.text.contains("line comment")),
            "Line comment not found"
        );
    }

    #[test]
    fn test_tokenize_rust_macro() {
        let code = "println!(\"hi\");";
        let tokens = tokenize(code, SupportedLanguage::Rust);
        assert!(
            tokens.iter().any(|t| t.kind == SyntaxTokenKind::Macro && t.text.contains("println")),
            "Macro not found in {:?}", tok_pairs(&tokens)
        );
    }

    #[test]
    fn test_tokenize_javascript_keywords_and_strings() {
        let code = r#"const name = "Alice"; let x = 3.14;"#;
        let tokens = tokenize(code, SupportedLanguage::JavaScript);
        assert_has_token(&tokens, SyntaxTokenKind::Keyword, "const");
        assert_has_token(&tokens, SyntaxTokenKind::Keyword, "let");
        assert_has_token(&tokens, SyntaxTokenKind::String, "\"Alice\"");
        assert_has_token(&tokens, SyntaxTokenKind::Number, "3.14");
    }

    #[test]
    fn test_tokenize_typescript_types() {
        let code = "function greet(name: string): void {}";
        let tokens = tokenize(code, SupportedLanguage::TypeScript);
        assert_has_token(&tokens, SyntaxTokenKind::Keyword, "function");
        assert_has_token(&tokens, SyntaxTokenKind::Function, "greet");
        // `string` and `void` should be recognized as types in TS
        assert!(
            tokens.iter().any(|t| t.kind == SyntaxTokenKind::Type && (t.text == "string" || t.text == "void")),
            "TS built-in type not found"
        );
    }

    #[test]
    fn test_tokenize_python_keywords_and_comments() {
        let code = "def calculate(a, b):\n    # sum them\n    return a + b";
        let tokens = tokenize(code, SupportedLanguage::Python);
        assert_has_token(&tokens, SyntaxTokenKind::Keyword, "def");
        assert_has_token(&tokens, SyntaxTokenKind::Keyword, "return");
        assert_has_token(&tokens, SyntaxTokenKind::Function, "calculate");
        assert!(
            tokens.iter().any(|t| t.kind == SyntaxTokenKind::Comment && t.text.contains("sum them")),
            "Python comment not found"
        );
    }

    #[test]
    fn test_tokenize_python_triple_quote_string() {
        let code = "s = \"\"\"multi\nline\nstring\"\"\"";
        let tokens = tokenize(code, SupportedLanguage::Python);
        assert!(
            tokens.iter().any(|t| t.kind == SyntaxTokenKind::String && t.text.contains("multi")),
            "Triple-quote string not tokenized"
        );
    }

    #[test]
    fn test_tokenize_go_keywords_and_types() {
        let code = "func main() { var x int = 42 }";
        let tokens = tokenize(code, SupportedLanguage::Go);
        assert_has_token(&tokens, SyntaxTokenKind::Keyword, "func");
        assert_has_token(&tokens, SyntaxTokenKind::Keyword, "var");
        assert_has_token(&tokens, SyntaxTokenKind::Type, "int");
        assert_has_token(&tokens, SyntaxTokenKind::Number, "42");
        assert_has_token(&tokens, SyntaxTokenKind::Function, "main");
    }

    #[test]
    fn test_tokenize_c_keywords_and_preprocessor() {
        let code = "#include <stdio.h>\nint main() { return 0; }";
        let tokens = tokenize(code, SupportedLanguage::C);
        assert_has_token(&tokens, SyntaxTokenKind::Keyword, "return");
        assert_has_token(&tokens, SyntaxTokenKind::Type, "int");
        assert_has_token(&tokens, SyntaxTokenKind::Number, "0");
        assert!(
            tokens.iter().any(|t| t.kind == SyntaxTokenKind::Macro && t.text.contains("include")),
            "C preprocessor directive not found"
        );
    }

    #[test]
    fn test_tokenize_cpp_block_comment() {
        let code = "/* block\n   comment */ int x = 5;";
        let tokens = tokenize(code, SupportedLanguage::Cpp);
        assert!(
            tokens.iter().any(|t| t.kind == SyntaxTokenKind::Comment && t.text.contains("block")),
            "Block comment not found"
        );
        assert_has_token(&tokens, SyntaxTokenKind::Type, "int");
        assert_has_token(&tokens, SyntaxTokenKind::Number, "5");
    }

    #[test]
    fn test_tokenize_html_tags_and_attributes() {
        let code = r#"<div class="main"><p>Hello</p></div>"#;
        let tokens = tokenize(code, SupportedLanguage::Html);
        assert!(
            tokens.iter().any(|t| t.kind == SyntaxTokenKind::Tag && t.text.contains("div")),
            "HTML tag 'div' not found"
        );
        assert!(
            tokens.iter().any(|t| t.kind == SyntaxTokenKind::Attribute && t.text.contains("class")),
            "HTML attribute 'class' not found"
        );
        assert!(
            tokens.iter().any(|t| t.kind == SyntaxTokenKind::String && t.text.contains("main")),
            "HTML attribute value string not found"
        );
    }

    #[test]
    fn test_tokenize_css_selectors_and_properties() {
        let code = "body { color: red; font-size: 14px; }";
        let tokens = tokenize(code, SupportedLanguage::Css);
        // CSS should tokenize property names and values
        assert!(
            !tokens.is_empty(),
            "CSS tokenizer produced no tokens"
        );
        // Numbers like 14px should be recognized
        assert!(
            tokens.iter().any(|t| t.kind == SyntaxTokenKind::Number && t.text.contains("14")),
            "CSS number not found in: {:?}", tok_pairs(&tokens)
        );
    }

    #[test]
    fn test_tokenize_json_keys_and_values() {
        let code = r#"{"name": "fusion", "version": 2, "active": true}"#;
        let tokens = tokenize(code, SupportedLanguage::Json);
        assert!(
            tokens.iter().any(|t| t.kind == SyntaxTokenKind::Attribute && t.text.contains("name")),
            "JSON key not found"
        );
        assert!(
            tokens.iter().any(|t| t.kind == SyntaxTokenKind::String && t.text.contains("fusion")),
            "JSON string value not found"
        );
        assert_has_token(&tokens, SyntaxTokenKind::Number, "2");
        assert!(
            tokens.iter().any(|t| t.kind == SyntaxTokenKind::Constant && t.text == "true"),
            "JSON boolean not found"
        );
    }

    #[test]
    fn test_tokenize_yaml_keys_and_comments() {
        let code = "name: fusion\n# comment\nversion: 2";
        let tokens = tokenize(code, SupportedLanguage::Yaml);
        assert!(
            tokens.iter().any(|t| t.kind == SyntaxTokenKind::Attribute && t.text.contains("name")),
            "YAML key not found in: {:?}", tok_pairs(&tokens)
        );
        assert!(
            tokens.iter().any(|t| t.kind == SyntaxTokenKind::Comment && t.text.contains("comment")),
            "YAML comment not found"
        );
    }

    #[test]
    fn test_tokenize_toml_keys_and_sections() {
        let code = "[package]\nname = \"fusion\"\nversion = 1";
        let tokens = tokenize(code, SupportedLanguage::Toml);
        assert!(
            tokens.iter().any(|t| (t.kind == SyntaxTokenKind::Tag || t.kind == SyntaxTokenKind::Attribute) && t.text.contains("package")),
            "TOML section header not found in: {:?}", tok_pairs(&tokens)
        );
        assert!(
            tokens.iter().any(|t| t.kind == SyntaxTokenKind::String && t.text.contains("fusion")),
            "TOML string value not found"
        );
    }

    #[test]
    fn test_tokenize_shell_keywords_and_variables() {
        let code = "if [ -f \"$HOME/.bashrc\" ]; then\n  echo \"found\"\nfi";
        let tokens = tokenize(code, SupportedLanguage::Shell);
        assert_has_token(&tokens, SyntaxTokenKind::Keyword, "if");
        assert_has_token(&tokens, SyntaxTokenKind::Keyword, "then");
        assert_has_token(&tokens, SyntaxTokenKind::Keyword, "fi");
        assert!(
            tokens.iter().any(|t| t.kind == SyntaxTokenKind::Variable && t.text.contains("HOME")),
            "Shell variable not found in: {:?}", tok_pairs(&tokens)
        );
    }

    #[test]
    fn test_tokenize_shell_comment() {
        let code = "# This is a comment\necho hello";
        let tokens = tokenize(code, SupportedLanguage::Shell);
        assert!(
            tokens.iter().any(|t| t.kind == SyntaxTokenKind::Comment && t.text.contains("This is a comment")),
            "Shell comment not found"
        );
    }

    #[test]
    fn test_tokenize_empty_input() {
        let tokens = tokenize("", SupportedLanguage::Rust);
        assert!(tokens.is_empty());
    }

    #[test]
    fn test_tokenize_preserves_full_content() {
        // Concatenating all token texts should reproduce the original code.
        let code = "fn main() { let x = 42; }";
        let tokens = tokenize(code, SupportedLanguage::Rust);
        let reconstructed: String = tokens.iter().map(|t| t.text.as_str()).collect();
        assert_eq!(reconstructed, code, "Token texts don't reconstruct original input");
    }

    #[test]
    fn test_tokenize_contiguous_spans() {
        // Byte spans should be contiguous and cover the full input.
        let code = "let x: i32 = 10;";
        let tokens = tokenize(code, SupportedLanguage::Rust);
        assert!(!tokens.is_empty());
        assert_eq!(tokens[0].start, 0, "First token should start at 0");
        for pair in tokens.windows(2) {
            assert_eq!(
                pair[0].end, pair[1].start,
                "Gap between tokens: {:?} and {:?}",
                pair[0], pair[1]
            );
        }
        assert_eq!(tokens.last().unwrap().end, code.len(), "Last token should end at content length");
    }

    #[test]
    fn test_tokenize_python_preserves_content() {
        let code = "def foo(x):\n    return x * 2";
        let tokens = tokenize(code, SupportedLanguage::Python);
        let reconstructed: String = tokens.iter().map(|t| t.text.as_str()).collect();
        assert_eq!(reconstructed, code);
    }

    #[test]
    fn test_tokenize_go_preserves_content() {
        let code = "package main\n\nimport \"fmt\"\n\nfunc main() {\n\tfmt.Println(\"hello\")\n}";
        let tokens = tokenize(code, SupportedLanguage::Go);
        let reconstructed: String = tokens.iter().map(|t| t.text.as_str()).collect();
        assert_eq!(reconstructed, code);
    }

    #[test]
    fn test_tokenize_json_preserves_content() {
        let code = r#"{"a": 1, "b": [true, null]}"#;
        let tokens = tokenize(code, SupportedLanguage::Json);
        let reconstructed: String = tokens.iter().map(|t| t.text.as_str()).collect();
        assert_eq!(reconstructed, code);
    }

    // -----------------------------------------------------------------------
    // Color Rendering Tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_rgb_truecolor_fg() {
        let c = RgbColor::new(255, 0, 128);
        assert_eq!(c.to_truecolor_fg(), "\x1b[38;2;255;0;128m");
    }

    #[test]
    fn test_rgb_truecolor_bg() {
        let c = RgbColor::new(0, 255, 64);
        assert_eq!(c.to_truecolor_bg(), "\x1b[48;2;0;255;64m");
    }

    #[test]
    fn test_rgb_ansi256_fg_format() {
        let c = RgbColor::new(255, 0, 0);
        let s = c.to_ansi256_fg();
        assert!(s.starts_with("\x1b[38;5;"), "256-color FG should start with ESC[38;5;");
        assert!(s.ends_with('m'));
    }

    #[test]
    fn test_rgb_ansi256_bg_format() {
        let c = RgbColor::new(0, 0, 255);
        let s = c.to_ansi256_bg();
        assert!(s.starts_with("\x1b[48;5;"), "256-color BG should start with ESC[48;5;");
        assert!(s.ends_with('m'));
    }

    #[test]
    fn test_rgb_from_hex() {
        let c = RgbColor::from_hex("#FF79C6").unwrap();
        assert_eq!(c, RgbColor::new(255, 121, 198));

        let c2 = RgbColor::from_hex("#abc").unwrap();
        assert_eq!(c2, RgbColor::new(0xAA, 0xBB, 0xCC));

        assert!(RgbColor::from_hex("xyz").is_none());
    }

    #[test]
    fn test_highlight_style_format_text_plain_mode() {
        let style = HighlightStyle::new().fg(RgbColor::new(255, 0, 0)).bold();
        let out = style.format_text("hello", ColorMode::Plain);
        assert_eq!(out, "hello", "Plain mode should produce unstyled text");
    }

    #[test]
    fn test_highlight_style_format_text_truecolor() {
        let style = HighlightStyle::new().fg(RgbColor::new(255, 121, 198)).bold();
        let out = style.format_text("fn", ColorMode::TrueColor);
        assert!(out.contains("\x1b[38;2;255;121;198m"), "Should contain truecolor fg");
        assert!(out.contains("\x1b[1m"), "Should contain bold");
        assert!(out.contains("fn"), "Should contain the text");
        assert!(out.ends_with("\x1b[0m"), "Should end with reset");
    }

    #[test]
    fn test_highlight_style_format_text_ansi256() {
        let style = HighlightStyle::new().fg(RgbColor::new(80, 250, 123)).italic();
        let out = style.format_text("green", ColorMode::Ansi256);
        assert!(out.contains("\x1b[38;5;"), "Should contain 256-color fg");
        assert!(out.contains("\x1b[3m"), "Should contain italic");
        assert!(out.ends_with("\x1b[0m"));
    }

    #[test]
    fn test_highlight_style_format_text_empty() {
        let style = HighlightStyle::new().fg(RgbColor::new(255, 0, 0));
        let out = style.format_text("", ColorMode::TrueColor);
        assert_eq!(out, "", "Empty text should return empty string");
    }

    #[test]
    fn test_highlight_style_dim_and_underline() {
        let style = HighlightStyle::new()
            .fg(RgbColor::new(100, 100, 100))
            .dim()
            .underline();
        let out = style.format_text("dimmed", ColorMode::TrueColor);
        assert!(out.contains("\x1b[2m"), "Should contain dim");
        assert!(out.contains("\x1b[4m"), "Should contain underline");
    }

    #[test]
    fn test_highlight_style_bg_truecolor() {
        let style = HighlightStyle::new()
            .fg(RgbColor::new(0, 0, 0))
            .bg(RgbColor::new(255, 255, 0));
        let out = style.format_text("highlighted", ColorMode::TrueColor);
        assert!(out.contains("\x1b[48;2;255;255;0m"), "Should contain truecolor bg");
    }

    // -----------------------------------------------------------------------
    // Theme Tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_theme_style_for_returns_correct_styles() {
        let theme = HighlightTheme::dark();
        let kw_style = theme.style_for(SyntaxTokenKind::Keyword);
        assert!(kw_style.bold, "Keyword should be bold in dark theme");
        assert!(kw_style.fg.is_some(), "Keyword should have fg color");

        let comment_style = theme.style_for(SyntaxTokenKind::Comment);
        assert!(comment_style.italic, "Comment should be italic in dark theme");
    }

    #[test]
    fn test_theme_from_name() {
        assert_eq!(HighlightTheme::from_name("dark").name, "dark");
        assert_eq!(HighlightTheme::from_name("monokai").name, "monokai");
        assert_eq!(HighlightTheme::from_name("one_dark").name, "one_dark");
        assert_eq!(HighlightTheme::from_name("onedark").name, "one_dark");
        assert_eq!(HighlightTheme::from_name("light").name, "light");
        assert_eq!(HighlightTheme::from_name("nord").name, "nord");
        assert_eq!(HighlightTheme::from_name("solarized").name, "solarized_dark");
        // Unknown falls back to dark
        assert_eq!(HighlightTheme::from_name("unknown_theme").name, "dark");
    }

    #[test]
    fn test_theme_format_token() {
        let theme = HighlightTheme::dark();
        let token = SyntaxToken::new(SyntaxTokenKind::Keyword, "fn", 0, 2);
        let out = theme.format_token(&token, ColorMode::TrueColor);
        assert!(out.contains("fn"));
        assert!(out.contains("\x1b["), "Should contain ANSI escape");
        assert!(out.ends_with("\x1b[0m"));
    }

    // -----------------------------------------------------------------------
    // End-to-End Highlight Function Tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_highlight_rust_code() {
        let code = "fn main() { let x = 42; }";
        let out = highlight(code, SupportedLanguage::Rust);
        assert!(out.contains("\x1b["), "Output should contain ANSI escapes");
        assert!(out.contains("\x1b[0m"), "Output should contain resets");
        // Stripping escapes should recover readable text
        let stripped = strip_ansi(&out);
        assert_eq!(stripped, code, "Stripped output should match original");
    }

    #[test]
    fn test_highlight_code_by_hint() {
        let code = "import os\nprint(os.getcwd())";
        let out = highlight_code(code, "python");
        assert!(out.contains("\x1b["), "Should be highlighted");
        let stripped = strip_ansi(&out);
        assert_eq!(stripped, code);
    }

    #[test]
    fn test_highlight_with_theme_plain_passthrough() {
        let code = "fn main() {}";
        let out = highlight_with_theme(code, SupportedLanguage::Rust, &HighlightTheme::dark(), ColorMode::Plain);
        assert_eq!(out, code, "Plain mode should return original code");
    }

    #[test]
    fn test_highlight_empty_input() {
        let out = highlight("", SupportedLanguage::Rust);
        assert_eq!(out, "");
    }

    #[test]
    fn test_highlight_line_single() {
        let line = "let x: i32 = 10;";
        let out = highlight_line(line, SupportedLanguage::Rust);
        assert!(out.contains("\x1b["));
        let stripped = strip_ansi(&out);
        assert_eq!(stripped, line);
    }

    #[test]
    fn test_highlight_with_line_numbers() {
        let code = "fn main() {\n    println!(\"hi\");\n}";
        let out = highlight_with_line_numbers(code, SupportedLanguage::Rust, 1, &HighlightTheme::dark(), ColorMode::TrueColor);
        // Should contain line numbers
        let stripped = strip_ansi(&out);
        assert!(stripped.contains("1"), "Should have line number 1");
        assert!(stripped.contains("2"), "Should have line number 2");
        assert!(stripped.contains("│"), "Should have gutter separator");
    }

    #[test]
    fn test_highlight_auto_detects_language() {
        let code = "fn main() { }";
        let path = std::path::Path::new("test.rs");
        let out = highlight_auto(code, Some(path), None);
        assert!(out.contains("\x1b["), "Auto-detected Rust should be highlighted");
    }

    #[test]
    fn test_syntax_highlighter_builder() {
        let hl = SyntaxHighlighter::new()
            .with_theme(HighlightTheme::monokai())
            .with_color_mode(ColorMode::Ansi256)
            .with_line_numbers(true)
            .with_start_line_number(10);

        assert_eq!(hl.theme.name, "monokai");
        assert_eq!(hl.color_mode, ColorMode::Ansi256);
        assert!(hl.line_numbers);
        assert_eq!(hl.start_line_number, 10);

        let code = "let x = 1;";
        let out = hl.highlight(code, SupportedLanguage::Rust);
        let stripped = strip_ansi(&out);
        assert!(stripped.contains("10"), "Should start at line 10");
    }

    #[test]
    fn test_syntax_highlighter_highlight_lines() {
        let hl = SyntaxHighlighter::new();
        let code = "fn a() {}\nfn b() {}";
        let lines = hl.highlight_lines(code, SupportedLanguage::Rust);
        assert_eq!(lines.len(), 2);
        for line in &lines {
            assert!(line.contains("\x1b["));
        }
    }

    // -----------------------------------------------------------------------
    // ColorMode Tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_color_mode_truecolor_vs_256() {
        let code = "fn main() {}";
        let tc = highlight_with_theme(code, SupportedLanguage::Rust, &HighlightTheme::dark(), ColorMode::TrueColor);
        let a256 = highlight_with_theme(code, SupportedLanguage::Rust, &HighlightTheme::dark(), ColorMode::Ansi256);
        // Both should produce ANSI output but with different escape formats
        assert!(tc.contains("\x1b[38;2;"), "TrueColor should use 38;2 sequences");
        assert!(a256.contains("\x1b[38;5;"), "Ansi256 should use 38;5 sequences");
    }

    #[test]
    fn test_color_mode_ansi16() {
        let code = "fn main() {}";
        let out = highlight_with_theme(code, SupportedLanguage::Rust, &HighlightTheme::dark(), ColorMode::Ansi16);
        assert!(out.contains("\x1b["), "Ansi16 should produce escapes");
        // Should NOT contain 38;2 or 38;5
        assert!(!out.contains("\x1b[38;2;"), "Ansi16 should not use truecolor");
        assert!(!out.contains("\x1b[38;5;"), "Ansi16 should not use 256-color");
    }

    // -----------------------------------------------------------------------
    // SyntaxTokenKind Tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_syntax_token_kind_as_str() {
        assert_eq!(SyntaxTokenKind::Keyword.as_str(), "keyword");
        assert_eq!(SyntaxTokenKind::Type.as_str(), "type");
        assert_eq!(SyntaxTokenKind::Function.as_str(), "function");
        assert_eq!(SyntaxTokenKind::String.as_str(), "string");
        assert_eq!(SyntaxTokenKind::Number.as_str(), "number");
        assert_eq!(SyntaxTokenKind::Comment.as_str(), "comment");
        assert_eq!(SyntaxTokenKind::Operator.as_str(), "operator");
        assert_eq!(SyntaxTokenKind::Punctuation.as_str(), "punctuation");
        assert_eq!(SyntaxTokenKind::Tag.as_str(), "tag");
        assert_eq!(SyntaxTokenKind::Attribute.as_str(), "attribute");
        assert_eq!(SyntaxTokenKind::Variable.as_str(), "variable");
        assert_eq!(SyntaxTokenKind::Constant.as_str(), "constant");
        assert_eq!(SyntaxTokenKind::Macro.as_str(), "macro");
        assert_eq!(SyntaxTokenKind::Plain.as_str(), "plain");
    }

    #[test]
    fn test_syntax_token_kind_predicates() {
        assert!(SyntaxTokenKind::Keyword.is_keyword());
        assert!(!SyntaxTokenKind::Keyword.is_type());
        assert!(SyntaxTokenKind::Type.is_type());
        assert!(SyntaxTokenKind::Function.is_function());
        assert!(SyntaxTokenKind::String.is_string());
        assert!(SyntaxTokenKind::Number.is_number());
        assert!(SyntaxTokenKind::Comment.is_comment());
    }

    #[test]
    fn test_syntax_token_kind_display() {
        assert_eq!(format!("{}", SyntaxTokenKind::Keyword), "keyword");
        assert_eq!(format!("{}", SyntaxTokenKind::Plain), "plain");
    }

    // -----------------------------------------------------------------------
    // SyntaxToken Tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_syntax_token_new_and_accessors() {
        let t = SyntaxToken::new(SyntaxTokenKind::Keyword, "fn", 0, 2);
        assert_eq!(t.kind, SyntaxTokenKind::Keyword);
        assert_eq!(t.text, "fn");
        assert_eq!(t.start, 0);
        assert_eq!(t.end, 2);
        assert_eq!(t.len(), 2);
        assert!(!t.is_empty());
    }

    #[test]
    fn test_syntax_token_empty() {
        let t = SyntaxToken::new(SyntaxTokenKind::Plain, "", 5, 5);
        assert!(t.is_empty());
        assert_eq!(t.len(), 0);
    }

    // -----------------------------------------------------------------------
    // Language Detection Tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_detect_language_from_path() {
        assert_eq!(detect_language(Some(Path::new("main.rs")), "", None), SupportedLanguage::Rust);
        assert_eq!(detect_language(Some(Path::new("app.js")), "", None), SupportedLanguage::JavaScript);
        assert_eq!(detect_language(Some(Path::new("app.ts")), "", None), SupportedLanguage::TypeScript);
        assert_eq!(detect_language(Some(Path::new("script.py")), "", None), SupportedLanguage::Python);
        assert_eq!(detect_language(Some(Path::new("main.go")), "", None), SupportedLanguage::Go);
        assert_eq!(detect_language(Some(Path::new("main.c")), "", None), SupportedLanguage::C);
        assert_eq!(detect_language(Some(Path::new("main.cpp")), "", None), SupportedLanguage::Cpp);
        assert_eq!(detect_language(Some(Path::new("index.html")), "", None), SupportedLanguage::Html);
        assert_eq!(detect_language(Some(Path::new("style.css")), "", None), SupportedLanguage::Css);
        assert_eq!(detect_language(Some(Path::new("data.json")), "", None), SupportedLanguage::Json);
        assert_eq!(detect_language(Some(Path::new("config.yaml")), "", None), SupportedLanguage::Yaml);
        assert_eq!(detect_language(Some(Path::new("Cargo.toml")), "", None), SupportedLanguage::Toml);
        assert_eq!(detect_language(Some(Path::new("run.sh")), "", None), SupportedLanguage::Shell);
    }

    #[test]
    fn test_detect_language_from_hint() {
        assert_eq!(detect_language(None, "", Some("rust")), SupportedLanguage::Rust);
        assert_eq!(detect_language(None, "", Some("python")), SupportedLanguage::Python);
        assert_eq!(detect_language(None, "", Some("javascript")), SupportedLanguage::JavaScript);
    }

    // -----------------------------------------------------------------------
    // ANSI Stripping Utility for Test Assertions
    // -----------------------------------------------------------------------

    /// Strip ANSI escape sequences from a string for content comparison.
    fn strip_ansi(s: &str) -> String {
        let mut out = String::with_capacity(s.len());
        let mut in_escape = false;
        for ch in s.chars() {
            if in_escape {
                if ch.is_ascii_alphabetic() {
                    in_escape = false;
                }
            } else if ch == '\x1b' {
                in_escape = true;
            } else {
                out.push(ch);
            }
        }
        out
    }
}
