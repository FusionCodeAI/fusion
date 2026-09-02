//! Standalone markdown-to-styled-HTML documentation page generator.
//!
//! Provides a zero-dependency, cross-platform documentation engine that converts
//! markdown into rich, modern, responsive standalone HTML pages and multi-page sites.
//! Features include:
//! - Full markdown rendering (headings, tables, lists, task lists, code blocks, callouts/admonitions)
//! - Syntax highlighting for multiple programming languages (Rust, Python, JS/TS, Go, C/C++, Shell, JSON, SQL, etc.)
//! - GitHub-style alert callouts (`[!NOTE]`, `[!TIP]`, `[!WARNING]`, `[!IMPORTANT]`, `[!CAUTION]`)
//! - Auto-generated table of contents (TOC) with sticky sidebar navigation and anchor links
//! - Responsive CSS Grid layout with light, dark, auto, and custom themes
//! - Embedded offline vanilla JS for search/filtering, code copy buttons, theme toggling, and mobile menu
//! - Multi-page doc site generation with navigation trees and breadcrumbs

use std::collections::HashMap;

// ============================================================================
// Enums & Configurations
// ============================================================================

/// Built-in color themes for generated documentation.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum DocTheme {
    /// Follows user's system OS preference (prefers-color-scheme).
    #[default]
    Auto,
    /// Light theme with clean contrast.
    Light,
    /// Dark theme with slate/charcoal tones.
    Dark,
    /// Arctic, north-bluish clean theme.
    Nord,
    /// High-contrast neon-accented cyberpunk theme.
    Cyberpunk,
    /// Precision colors for machines and people (Light).
    SolarizedLight,
    /// Precision colors for machines and people (Dark).
    SolarizedDark,
    /// Custom color palette.
    Custom(ThemeColors),
}

/// Custom color configuration for themes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ThemeColors {
    pub bg_primary: String,
    pub bg_secondary: String,
    pub bg_card: String,
    pub bg_code: String,
    pub text_primary: String,
    pub text_secondary: String,
    pub text_muted: String,
    pub text_link: String,
    pub accent: String,
    pub accent_hover: String,
    pub border: String,
    pub border_subtle: String,
}

impl Default for ThemeColors {
    fn default() -> Self {
        Self {
            bg_primary: "#0f172a".to_string(),
            bg_secondary: "#1e293b".to_string(),
            bg_card: "#1e293b".to_string(),
            bg_code: "#0d1117".to_string(),
            text_primary: "#f8fafc".to_string(),
            text_secondary: "#cbd5e1".to_string(),
            text_muted: "#64748b".to_string(),
            text_link: "#38bdf8".to_string(),
            accent: "#6366f1".to_string(),
            accent_hover: "#4f46e5".to_string(),
            border: "#334155".to_string(),
            border_subtle: "#1e293b".to_string(),
        }
    }
}

/// Navigation item in the sidebar.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NavItem {
    pub title: String,
    pub url: String,
    pub active: bool,
    pub icon: Option<String>,
    pub badge: Option<String>,
}

impl NavItem {
    pub fn new(title: impl Into<String>, url: impl Into<String>) -> Self {
        Self {
            title: title.into(),
            url: url.into(),
            active: false,
            icon: None,
            badge: None,
        }
    }

    pub fn with_active(mut self, active: bool) -> Self {
        self.active = active;
        self
    }

    pub fn with_icon(mut self, icon: impl Into<String>) -> Self {
        self.icon = Some(icon.into());
        self
    }

    pub fn with_badge(mut self, badge: impl Into<String>) -> Self {
        self.badge = Some(badge.into());
        self
    }
}

/// Section of grouped navigation items in the sidebar.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NavSection {
    pub title: String,
    pub items: Vec<NavItem>,
}

impl NavSection {
    pub fn new(title: impl Into<String>, items: Vec<NavItem>) -> Self {
        Self {
            title: title.into(),
            items,
        }
    }
}

/// Configuration options for generating documentation pages.
#[derive(Debug, Clone)]
pub struct DocConfig {
    pub title: String,
    pub subtitle: Option<String>,
    pub description: Option<String>,
    pub author: Option<String>,
    pub version: Option<String>,
    pub logo: Option<String>,
    pub favicon: Option<String>,
    pub repo_url: Option<String>,
    pub theme: DocTheme,
    pub show_toc: bool,
    pub show_sidebar: bool,
    pub show_search: bool,
    pub show_copy_button: bool,
    pub show_back_to_top: bool,
    pub show_theme_toggle: bool,
    pub show_footer: bool,
    pub footer_text: Option<String>,
    pub syntax_highlighting: bool,
    pub custom_css: Option<String>,
    pub custom_js: Option<String>,
    pub sidebar_nav: Vec<NavSection>,
    pub breadcrumbs: Vec<(String, String)>,
}

impl Default for DocConfig {
    fn default() -> Self {
        Self {
            title: "Documentation".to_string(),
            subtitle: None,
            description: Some("Generated by Fusion Documentation Generator".to_string()),
            author: None,
            version: None,
            logo: None,
            favicon: None,
            repo_url: None,
            theme: DocTheme::Auto,
            show_toc: true,
            show_sidebar: true,
            show_search: true,
            show_copy_button: true,
            show_back_to_top: true,
            show_theme_toggle: true,
            show_footer: true,
            footer_text: None,
            syntax_highlighting: true,
            custom_css: None,
            custom_js: None,
            sidebar_nav: Vec::new(),
            breadcrumbs: Vec::new(),
        }
    }
}

impl DocConfig {
    pub fn builder() -> DocConfigBuilder {
        DocConfigBuilder::default()
    }

    pub fn new(title: impl Into<String>) -> Self {
        Self {
            title: title.into(),
            ..Default::default()
        }
    }
}

/// Builder for `DocConfig`.
#[derive(Debug, Default)]
pub struct DocConfigBuilder {
    config: DocConfig,
}

impl DocConfigBuilder {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn title(mut self, title: impl Into<String>) -> Self {
        self.config.title = title.into();
        self
    }

    pub fn subtitle(mut self, subtitle: impl Into<String>) -> Self {
        self.config.subtitle = Some(subtitle.into());
        self
    }

    pub fn description(mut self, description: impl Into<String>) -> Self {
        self.config.description = Some(description.into());
        self
    }

    pub fn author(mut self, author: impl Into<String>) -> Self {
        self.config.author = Some(author.into());
        self
    }

    pub fn version(mut self, version: impl Into<String>) -> Self {
        self.config.version = Some(version.into());
        self
    }

    pub fn logo(mut self, logo: impl Into<String>) -> Self {
        self.config.logo = Some(logo.into());
        self
    }

    pub fn favicon(mut self, favicon: impl Into<String>) -> Self {
        self.config.favicon = Some(favicon.into());
        self
    }

    pub fn repo_url(mut self, repo_url: impl Into<String>) -> Self {
        self.config.repo_url = Some(repo_url.into());
        self
    }

    pub fn theme(mut self, theme: DocTheme) -> Self {
        self.config.theme = theme;
        self
    }

    pub fn show_toc(mut self, show_toc: bool) -> Self {
        self.config.show_toc = show_toc;
        self
    }

    pub fn show_sidebar(mut self, show_sidebar: bool) -> Self {
        self.config.show_sidebar = show_sidebar;
        self
    }

    pub fn show_search(mut self, show_search: bool) -> Self {
        self.config.show_search = show_search;
        self
    }

    pub fn show_copy_button(mut self, show_copy_button: bool) -> Self {
        self.config.show_copy_button = show_copy_button;
        self
    }

    pub fn show_back_to_top(mut self, show_back_to_top: bool) -> Self {
        self.config.show_back_to_top = show_back_to_top;
        self
    }

    pub fn show_theme_toggle(mut self, show_theme_toggle: bool) -> Self {
        self.config.show_theme_toggle = show_theme_toggle;
        self
    }

    pub fn show_footer(mut self, show_footer: bool) -> Self {
        self.config.show_footer = show_footer;
        self
    }

    pub fn footer_text(mut self, footer_text: impl Into<String>) -> Self {
        self.config.footer_text = Some(footer_text.into());
        self
    }

    pub fn syntax_highlighting(mut self, syntax_highlighting: bool) -> Self {
        self.config.syntax_highlighting = syntax_highlighting;
        self
    }

    pub fn custom_css(mut self, custom_css: impl Into<String>) -> Self {
        self.config.custom_css = Some(custom_css.into());
        self
    }

    pub fn custom_js(mut self, custom_js: impl Into<String>) -> Self {
        self.config.custom_js = Some(custom_js.into());
        self
    }

    pub fn add_nav_section(mut self, section: NavSection) -> Self {
        self.config.sidebar_nav.push(section);
        self
    }

    pub fn add_breadcrumb(mut self, label: impl Into<String>, url: impl Into<String>) -> Self {
        self.config.breadcrumbs.push((label.into(), url.into()));
        self
    }

    pub fn build(self) -> DocConfig {
        self.config
    }
}

/// Represents a single documentation page in a multi-page site.
#[derive(Debug, Clone)]
pub struct DocPage {
    pub id: String,
    pub title: String,
    pub content: String,
    pub category: Option<String>,
    pub order: usize,
}

impl DocPage {
    pub fn new(id: impl Into<String>, title: impl Into<String>, content: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            title: title.into(),
            content: content.into(),
            category: None,
            order: 0,
        }
    }

    pub fn with_category(mut self, category: impl Into<String>) -> Self {
        self.category = Some(category.into());
        self
    }

    pub fn with_order(mut self, order: usize) -> Self {
        self.order = order;
        self
    }
}

/// Extracted heading for table-of-contents.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HeadingItem {
    pub level: usize,
    pub title: String,
    pub id: String,
}

// ============================================================================
// HTML Utilities & Escaping
// ============================================================================

/// Escapes standard HTML special characters.
pub fn escape_html(input: &str) -> String {
    let mut output = String::with_capacity(input.len());
    for c in input.chars() {
        match c {
            '&' => output.push_str("&amp;"),
            '<' => output.push_str("&lt;"),
            '>' => output.push_str("&gt;"),
            '"' => output.push_str("&quot;"),
            '\'' => output.push_str("&#39;"),
            _ => output.push(c),
        }
    }
    output
}

/// Transforms text into a clean URL-friendly slug ID for headings.
pub fn slugify(text: &str) -> String {
    let mut slug = String::new();
    let mut prev_dash = false;

    for c in text.chars() {
        if c.is_alphanumeric() {
            slug.push(c.to_ascii_lowercase());
            prev_dash = false;
        } else if (c == ' ' || c == '-' || c == '_' || c == '.') && !prev_dash && !slug.is_empty() {
            slug.push('-');
            prev_dash = true;
        }
    }

    while slug.ends_with('-') {
        slug.pop();
    }

    if slug.is_empty() {
        "section".to_string()
    } else {
        slug
    }
}

// ============================================================================
// Markdown Extraction & Parsing
// ============================================================================

/// Extracts all headings from markdown source.
pub fn extract_headings(markdown: &str) -> Vec<HeadingItem> {
    let mut headings = Vec::new();
    let mut used_ids: HashMap<String, usize> = HashMap::new();
    let mut in_code_block = false;

    for line in markdown.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("```") || trimmed.starts_with("~~~") {
            in_code_block = !in_code_block;
            continue;
        }
        if in_code_block {
            continue;
        }

        if let Some((level, raw_title)) = parse_heading_line(trimmed) {
            let plain_title = strip_inline_markdown(raw_title);
            let base_id = slugify(&plain_title);
            let count = used_ids.entry(base_id.clone()).or_insert(0);
            let id = if *count == 0 {
                base_id
            } else {
                format!("{}-{}", base_id, count)
            };
            *count += 1;

            headings.push(HeadingItem {
                level,
                title: plain_title,
                id,
            });
        }
    }

    headings
}

fn parse_heading_line(line: &str) -> Option<(usize, &str)> {
    if !line.starts_with('#') {
        return None;
    }
    let bytes = line.as_bytes();
    let mut level = 0;
    while level < bytes.len() && bytes[level] == b'#' && level < 6 {
        level += 1;
    }
    if level > 0 && level < bytes.len() && bytes[level] == b' ' {
        Some((level, line[level + 1..].trim()))
    } else {
        None
    }
}

/// Strips formatting tags and markdown syntax to produce plain text.
pub fn strip_inline_markdown(text: &str) -> String {
    let mut out = String::new();
    let mut chars = text.chars().peekable();

    while let Some(c) = chars.next() {
        match c {
            '`' | '*' | '_' | '~' => {
                // skip markdown markup symbols
            }
            '[' => {
                // Link text: [text](url) -> keep text
                let mut link_text = String::new();
                for inner in chars.by_ref() {
                    if inner == ']' {
                        break;
                    }
                    link_text.push(inner);
                }
                if chars.peek() == Some(&'(') {
                    chars.next();
                    for inner in chars.by_ref() {
                        if inner == ')' {
                            break;
                        }
                    }
                }
                out.push_str(&link_text);
            }
            '!' if chars.peek() == Some(&'[') => {
                // Image: ![alt](url) -> skip or keep alt
                chars.next(); // consume '['
                for inner in chars.by_ref() {
                    if inner == ']' {
                        break;
                    }
                }
                if chars.peek() == Some(&'(') {
                    chars.next();
                    for inner in chars.by_ref() {
                        if inner == ')' {
                            break;
                        }
                    }
                }
            }
            _ => out.push(c),
        }
    }
    out
}

// ============================================================================
// Syntax Highlighting Engine
// ============================================================================

/// Lightweight pure-Rust syntax highlighter producing semantic HTML span tags.
pub fn highlight_code_html(code: &str, lang: &str) -> String {
    let lang_lower = lang.trim().to_lowercase();
    let mut result = String::with_capacity(code.len() * 2);

    for (line_idx, line) in code.lines().enumerate() {
        if line_idx > 0 {
            result.push('\n');
        }
        highlight_line_html(line, &lang_lower, &mut result);
    }

    result
}

fn highlight_line_html(line: &str, lang: &str, out: &mut String) {
    if line.is_empty() {
        return;
    }

    let trimmed = line.trim_start();
    let indent_len = line.len() - trimmed.len();
    out.push_str(&escape_html(&line[..indent_len]));

    // Line comment detection
    let is_comment = match lang {
        "rs" | "rust" | "js" | "javascript" | "ts" | "typescript" | "go" | "c" | "cpp" | "csharp"
        | "cs" | "java" | "kotlin" | "swift" | "php" => trimmed.starts_with("//"),
        "py" | "python" | "sh" | "bash" | "zsh" | "yaml" | "yml" | "toml" | "ruby" | "rb"
        | "dockerfile" | "makefile" => trimmed.starts_with('#'),
        "sql" | "lua" => trimmed.starts_with("--"),
        "html" | "xml" => trimmed.starts_with("<!--"),
        "css" => trimmed.starts_with("/*"),
        _ => false,
    };

    if is_comment {
        out.push_str("<span class=\"hl-comment\">");
        out.push_str(&escape_html(trimmed));
        out.push_str("</span>");
        return;
    }

    let chars: Vec<char> = trimmed.chars().collect();
    let mut i = 0;

    while i < chars.len() {
        let c = chars[i];

        // Inline comments: // or # or --
        if (c == '/' && i + 1 < chars.len() && chars[i + 1] == '/')
            && matches!(lang, "rs" | "rust" | "js" | "ts" | "go" | "c" | "cpp" | "cs" | "java")
        {
            out.push_str("<span class=\"hl-comment\">");
            let rem: String = chars[i..].iter().collect();
            out.push_str(&escape_html(&rem));
            out.push_str("</span>");
            break;
        }

        if c == '#' && matches!(lang, "py" | "python" | "sh" | "bash" | "yaml" | "toml" | "rb") {
            out.push_str("<span class=\"hl-comment\">");
            let rem: String = chars[i..].iter().collect();
            out.push_str(&escape_html(&rem));
            out.push_str("</span>");
            break;
        }

        // Strings
        if c == '"' || c == '\'' || (c == '`' && matches!(lang, "js" | "ts" | "go")) {
            let quote = c;
            out.push_str("<span class=\"hl-str\">");
            out.push(quote);
            i += 1;
            let mut escaped = false;
            while i < chars.len() {
                let cur = chars[i];
                if escaped {
                    out.push_str(&escape_html(&cur.to_string()));
                    escaped = false;
                } else if cur == '\\' {
                    escaped = true;
                    out.push('\\');
                } else if cur == quote {
                    out.push(quote);
                    i += 1;
                    break;
                } else {
                    out.push_str(&escape_html(&cur.to_string()));
                }
                i += 1;
            }
            out.push_str("</span>");
            continue;
        }

        // Numbers
        if c.is_ascii_digit() && (i == 0 || !chars[i - 1].is_alphanumeric() && chars[i - 1] != '_') {
            out.push_str("<span class=\"hl-num\">");
            while i < chars.len()
                && (chars[i].is_alphanumeric() || chars[i] == '.' || chars[i] == '_')
            {
                out.push(chars[i]);
                i += 1;
            }
            out.push_str("</span>");
            continue;
        }

        // Identifiers & Keywords
        if c.is_alphabetic() || c == '_' {
            let start = i;
            while i < chars.len() && (chars[i].is_alphanumeric() || chars[i] == '_') {
                i += 1;
            }
            let word: String = chars[start..i].iter().collect();

            // Function call check (followed by '(')
            let mut next_idx = i;
            while next_idx < chars.len() && chars[next_idx].is_whitespace() {
                next_idx += 1;
            }
            let is_fn_call = next_idx < chars.len() && chars[next_idx] == '(';

            if is_keyword(&word, lang) {
                out.push_str("<span class=\"hl-kw\">");
                out.push_str(&escape_html(&word));
                out.push_str("</span>");
            } else if is_type_name(&word) {
                out.push_str("<span class=\"hl-type\">");
                out.push_str(&escape_html(&word));
                out.push_str("</span>");
            } else if is_builtin(&word) {
                out.push_str("<span class=\"hl-builtin\">");
                out.push_str(&escape_html(&word));
                out.push_str("</span>");
            } else if is_fn_call {
                out.push_str("<span class=\"hl-fn\">");
                out.push_str(&escape_html(&word));
                out.push_str("</span>");
            } else {
                out.push_str(&escape_html(&word));
            }
            continue;
        }

        // Punctuation & operators
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            _ => out.push(c),
        }
        i += 1;
    }
}

fn is_keyword(word: &str, lang: &str) -> bool {
    match lang {
        "rs" | "rust" => matches!(
            word,
            "as" | "async" | "await" | "break" | "const" | "continue" | "crate" | "dyn" | "else"
                | "enum" | "extern" | "false" | "fn" | "for" | "if" | "impl" | "in" | "let"
                | "loop" | "match" | "mod" | "move" | "mut" | "pub" | "ref" | "return" | "self"
                | "Self" | "static" | "struct" | "super" | "trait" | "true" | "type" | "unsafe"
                | "use" | "where" | "while"
        ),
        "py" | "python" => matches!(
            word,
            "and" | "as" | "assert" | "async" | "await" | "break" | "class" | "continue" | "def"
                | "del" | "elif" | "else" | "except" | "finally" | "for" | "from" | "global"
                | "if" | "import" | "in" | "is" | "lambda" | "None" | "nonlocal" | "not" | "or"
                | "pass" | "raise" | "return" | "True" | "False" | "try" | "while" | "with"
                | "yield" | "self"
        ),
        "js" | "javascript" | "ts" | "typescript" => matches!(
            word,
            "abstract" | "arguments" | "async" | "await" | "boolean" | "break" | "byte" | "case"
                | "catch" | "class" | "const" | "continue" | "debugger" | "default" | "delete"
                | "do" | "else" | "enum" | "export" | "extends" | "false" | "final" | "finally"
                | "for" | "from" | "function" | "goto" | "if" | "implements" | "import" | "in"
                | "instanceof" | "interface" | "let" | "new" | "null" | "of" | "package"
                | "private" | "protected" | "public" | "return" | "static" | "super" | "switch"
                | "this" | "throw" | "true" | "try" | "type" | "typeof" | "undefined" | "var"
                | "void" | "while" | "with" | "yield"
        ),
        "go" => matches!(
            word,
            "break" | "case" | "chan" | "const" | "continue" | "default" | "defer" | "else"
                | "fallthrough" | "for" | "func" | "go" | "goto" | "if" | "import" | "interface"
                | "map" | "package" | "range" | "return" | "select" | "struct" | "switch"
                | "type" | "var" | "true" | "false" | "nil" | "iota"
        ),
        "c" | "cpp" | "csharp" | "cs" | "java" => matches!(
            word,
            "auto" | "break" | "case" | "catch" | "class" | "const" | "continue" | "default"
                | "delete" | "do" | "else" | "enum" | "explicit" | "export" | "extern" | "false"
                | "for" | "friend" | "goto" | "if" | "inline" | "namespace" | "new" | "operator"
                | "private" | "protected" | "public" | "return" | "sizeof" | "static" | "struct"
                | "switch" | "template" | "this" | "throw" | "true" | "try" | "typedef"
                | "typename" | "using" | "virtual" | "void" | "volatile" | "while" | "nullptr"
                | "override" | "final"
        ),
        "sh" | "bash" | "zsh" => matches!(
            word,
            "if" | "then" | "else" | "elif" | "fi" | "case" | "esac" | "for" | "select" | "while"
                | "until" | "do" | "done" | "in" | "function" | "time" | "source" | "export"
                | "local" | "readonly" | "return" | "exit" | "set" | "unset"
        ),
        "sql" => matches!(
            word,
            "SELECT" | "select" | "FROM" | "from" | "WHERE" | "where" | "INSERT" | "insert"
                | "INTO" | "into" | "VALUES" | "values" | "UPDATE" | "update" | "SET" | "set"
                | "DELETE" | "delete" | "CREATE" | "create" | "TABLE" | "table" | "DROP" | "drop"
                | "ALTER" | "alter" | "INDEX" | "index" | "JOIN" | "join" | "LEFT" | "left"
                | "RIGHT" | "right" | "INNER" | "inner" | "OUTER" | "outer" | "ON" | "on"
                | "GROUP" | "group" | "BY" | "by" | "ORDER" | "order" | "HAVING" | "having"
                | "LIMIT" | "limit" | "OFFSET" | "offset" | "UNION" | "union" | "ALL" | "all"
                | "AS" | "as" | "DISTINCT" | "distinct" | "AND" | "and" | "OR" | "or" | "NOT"
                | "not" | "NULL" | "null" | "TRUE" | "true" | "FALSE" | "false" | "PRIMARY"
                | "primary" | "KEY" | "key"
        ),
        _ => matches!(
            word,
            "fn" | "func" | "function" | "def" | "class" | "struct" | "interface" | "enum"
                | "let" | "var" | "const" | "val" | "if" | "else" | "for" | "while" | "return"
                | "import" | "export" | "pub" | "public" | "private" | "true" | "false" | "null"
                | "nil"
        ),
    }
}

fn is_type_name(word: &str) -> bool {
    if word.is_empty() {
        return false;
    }
    // Primitive and common type identifiers
    if matches!(
        word,
        "i8" | "i16"
            | "i32"
            | "i64"
            | "i128"
            | "isize"
            | "u8"
            | "u16"
            | "u32"
            | "u64"
            | "u128"
            | "usize"
            | "f32"
            | "f64"
            | "bool"
            | "char"
            | "str"
            | "String"
            | "Vec"
            | "Option"
            | "Result"
            | "Box"
            | "Rc"
            | "Arc"
            | "HashMap"
            | "HashSet"
            | "int"
            | "float"
            | "double"
            | "long"
            | "short"
            | "byte"
            | "string"
            | "boolean"
            | "number"
            | "any"
            | "void"
            | "never"
            | "unknown"
            | "int32"
            | "int64"
            | "uint32"
            | "uint64"
            | "float32"
            | "float64"
            | "int8"
            | "rune"
            | "error"
    ) {
        return true;
    }
    // PascalCase convention for structs / classes
    let first = word.chars().next().unwrap();
    first.is_ascii_uppercase() && word.chars().any(|c| c.is_ascii_lowercase())
}

fn is_builtin(word: &str) -> bool {
    matches!(
        word,
        "println"
            | "print"
            | "eprintln"
            | "eprint"
            | "format"
            | "panic"
            | "vec"
            | "len"
            | "range"
            | "append"
            | "make"
            | "new"
            | "console"
            | "document"
            | "window"
            | "JSON"
            | "Math"
            | "Promise"
            | "Array"
            | "Object"
            | "printf"
            | "scanf"
            | "malloc"
            | "free"
            | "sizeof"
    )
}

// ============================================================================
// Markdown to HTML Conversion
// ============================================================================

/// Converts an inline markdown snippet to HTML.
pub fn render_inline_html(text: &str) -> String {
    let mut out = String::with_capacity(text.len() * 2);
    let chars: Vec<char> = text.chars().collect();
    let mut i = 0;

    while i < chars.len() {
        let c = chars[i];

        // Inline code: `code`
        if c == '`' {
            let mut end = i + 1;
            while end < chars.len() && chars[end] != '`' {
                end += 1;
            }
            if end < chars.len() {
                let code_content: String = chars[i + 1..end].iter().collect();
                out.push_str("<code>");
                out.push_str(&escape_html(&code_content));
                out.push_str("</code>");
                i = end + 1;
                continue;
            }
        }

        // Highlight: ==marked==
        if c == '=' && i + 1 < chars.len() && chars[i + 1] == '=' {
            let mut end = i + 2;
            while end + 1 < chars.len() && !(chars[end] == '=' && chars[end + 1] == '=') {
                end += 1;
            }
            if end + 1 < chars.len() {
                let mark_content: String = chars[i + 2..end].iter().collect();
                out.push_str("<mark>");
                out.push_str(&render_inline_html(&mark_content));
                out.push_str("</mark>");
                i = end + 2;
                continue;
            }
        }

        // Strikethrough: ~~del~~
        if c == '~' && i + 1 < chars.len() && chars[i + 1] == '~' {
            let mut end = i + 2;
            while end + 1 < chars.len() && !(chars[end] == '~' && chars[end + 1] == '~') {
                end += 1;
            }
            if end + 1 < chars.len() {
                let del_content: String = chars[i + 2..end].iter().collect();
                out.push_str("<del>");
                out.push_str(&render_inline_html(&del_content));
                out.push_str("</del>");
                i = end + 2;
                continue;
            }
        }

        // Bold & Italic combinations: *** or ___
        if (c == '*' || c == '_')
            && i + 2 < chars.len()
            && chars[i + 1] == c
            && chars[i + 2] == c
        {
            let mut end = i + 3;
            while end + 2 < chars.len()
                && !(chars[end] == c && chars[end + 1] == c && chars[end + 2] == c)
            {
                end += 1;
            }
            if end + 2 < chars.len() {
                let inner: String = chars[i + 3..end].iter().collect();
                out.push_str("<strong><em>");
                out.push_str(&render_inline_html(&inner));
                out.push_str("</em></strong>");
                i = end + 3;
                continue;
            }
        }

        // Bold: ** or __
        if (c == '*' || c == '_') && i + 1 < chars.len() && chars[i + 1] == c {
            let mut end = i + 2;
            while end + 1 < chars.len() && !(chars[end] == c && chars[end + 1] == c) {
                end += 1;
            }
            if end + 1 < chars.len() {
                let inner: String = chars[i + 2..end].iter().collect();
                out.push_str("<strong>");
                out.push_str(&render_inline_html(&inner));
                out.push_str("</strong>");
                i = end + 2;
                continue;
            }
        }

        // Italic: * or _ (must not be inside word for _)
        if c == '*' || (c == '_' && (i == 0 || !chars[i - 1].is_alphanumeric())) {
            let mut end = i + 1;
            while end < chars.len() && chars[end] != c {
                end += 1;
            }
            if end < chars.len() && end > i + 1 {
                let inner: String = chars[i + 1..end].iter().collect();
                out.push_str("<em>");
                out.push_str(&render_inline_html(&inner));
                out.push_str("</em>");
                i = end + 1;
                continue;
            }
        }

        // Images: ![alt](url "title")
        if c == '!' && i + 1 < chars.len() && chars[i + 1] == '[' {
            let mut close_bracket = i + 2;
            while close_bracket < chars.len() && chars[close_bracket] != ']' {
                close_bracket += 1;
            }
            if close_bracket + 1 < chars.len() && chars[close_bracket + 1] == '(' {
                let mut close_paren = close_bracket + 2;
                while close_paren < chars.len() && chars[close_paren] != ')' {
                    close_paren += 1;
                }
                if close_paren < chars.len() {
                    let alt: String = chars[i + 2..close_bracket].iter().collect();
                    let url_part: String =
                        chars[close_bracket + 2..close_paren].iter().collect();
                    let (url, title) = parse_link_url_title(&url_part);
                    out.push_str("<img src=\"");
                    out.push_str(&escape_html(&url));
                    out.push_str("\" alt=\"");
                    out.push_str(&escape_html(&alt));
                    out.push('\"');
                    if let Some(t) = title {
                        out.push_str(" title=\"");
                        out.push_str(&escape_html(&t));
                        out.push('\"');
                    }
                    out.push_str(" loading=\"lazy\" />");
                    i = close_paren + 1;
                    continue;
                }
            }
        }

        // Links: [text](url)
        if c == '[' {
            let mut close_bracket = i + 1;
            while close_bracket < chars.len() && chars[close_bracket] != ']' {
                close_bracket += 1;
            }
            if close_bracket + 1 < chars.len() && chars[close_bracket + 1] == '(' {
                let mut close_paren = close_bracket + 2;
                while close_paren < chars.len() && chars[close_paren] != ')' {
                    close_paren += 1;
                }
                if close_paren < chars.len() {
                    let text_part: String = chars[i + 1..close_bracket].iter().collect();
                    let url_part: String =
                        chars[close_bracket + 2..close_paren].iter().collect();
                    let (url, title) = parse_link_url_title(&url_part);

                    out.push_str("<a href=\"");
                    out.push_str(&escape_html(&url));
                    out.push('\"');
                    if let Some(t) = title {
                        out.push_str(" title=\"");
                        out.push_str(&escape_html(&t));
                        out.push('\"');
                    }
                    if !url.starts_with('#') && !url.starts_with('/') {
                        out.push_str(" target=\"_blank\" rel=\"noopener noreferrer\"");
                    }
                    out.push('>');
                    out.push_str(&render_inline_html(&text_part));
                    out.push_str("</a>");
                    i = close_paren + 1;
                    continue;
                }
            }
        }

        // Raw HTML or characters
        match c {
            '&' => out.push_str("&amp;"),
            '<' => {
                // Check if it's an inline safe HTML tag like <kbd> or <br> or <span>
                let rem: String = chars[i..].iter().collect();
                if let Some((tag_html, advance)) = parse_safe_inline_html(&rem) {
                    out.push_str(&tag_html);
                    i += advance;
                    continue;
                } else {
                    out.push_str("&lt;");
                }
            }
            '>' => out.push_str("&gt;"),
            _ => out.push(c),
        }
        i += 1;
    }

    out
}

fn parse_link_url_title(url_part: &str) -> (String, Option<String>) {
    let trimmed = url_part.trim();
    if let Some(pos) = trimmed.find('"') {
        let url = trimmed[..pos].trim();
        let title_part = &trimmed[pos + 1..];
        let title = title_part.strip_suffix('"').unwrap_or(title_part).trim();
        (url.to_string(), Some(title.to_string()))
    } else {
        (trimmed.to_string(), None)
    }
}

fn parse_safe_inline_html(s: &str) -> Option<(String, usize)> {
    let safe_tags = [
        "<kbd>", "</kbd>", "<mark>", "</mark>", "<br>", "<br/>", "<br />", "<b>", "</b>", "<i>",
        "</i>", "<u>", "</u>", "<span>", "</span>", "<small>", "</small>", "<sub>", "</sub>",
        "<sup>", "</sup>", "<code>", "</code>",
    ];

    for tag in safe_tags {
        if s.starts_with(tag) {
            return Some((tag.to_string(), tag.chars().count()));
        }
    }

    if s.starts_with("<span class=\"") {
        if let Some(end) = s.find('>') {
            let tag = &s[..=end];
            return Some((tag.to_string(), tag.chars().count()));
        }
    }

    None
}

/// Converts a full markdown document into an HTML body fragment.
pub fn markdown_to_html(markdown: &str) -> String {
    let mut html = String::with_capacity(markdown.len() * 2);
    let mut lines = markdown.lines().peekable();
    let mut used_ids: HashMap<String, usize> = HashMap::new();

    while let Some(line) = lines.next() {
        let trimmed = line.trim();

        if trimmed.is_empty() {
            continue;
        }

        // Fenced code blocks
        if trimmed.starts_with("```") || trimmed.starts_with("~~~") {
            let fence_char = trimmed.chars().next().unwrap();
            let fence_len = trimmed.chars().take_while(|&c| c == fence_char).count();
            let lang = trimmed[fence_len..].trim();

            let mut code_lines = Vec::new();
            while let Some(next_line) = lines.peek() {
                let next_trimmed = next_line.trim();
                if next_trimmed.starts_with(&trimmed[..fence_len]) {
                    lines.next();
                    break;
                }
                code_lines.push(lines.next().unwrap());
            }

            let code_content = code_lines.join("\n");
            let highlighted = if !lang.is_empty() {
                highlight_code_html(&code_content, lang)
            } else {
                escape_html(&code_content)
            };

            let lang_badge = if !lang.is_empty() {
                format!("<span class=\"code-lang-badge\">{}</span>", escape_html(lang))
            } else {
                String::new()
            };

            html.push_str("<div class=\"code-block-wrapper\">\n");
            html.push_str("  <div class=\"code-header\">\n");
            html.push_str(&format!("    {}\n", lang_badge));
            html.push_str("    <button class=\"copy-code-btn\" title=\"Copy code\" onclick=\"copyCode(this)\">\n");
            html.push_str("      <svg class=\"copy-icon\" width=\"14\" height=\"14\" viewBox=\"0 0 24 24\" fill=\"none\" stroke=\"currentColor\" stroke-width=\"2\"><rect x=\"9\" y=\"9\" width=\"13\" height=\"13\" rx=\"2\" ry=\"2\"></rect><path d=\"M5 15H4a2 2 0 0 1-2-2V4a2 2 0 0 1 2-2h9a2 2 0 0 1 2 2v1\"></path></svg>\n");
            html.push_str("      <span class=\"copy-label\">Copy</span>\n");
            html.push_str("    </button>\n");
            html.push_str("  </div>\n");
            html.push_str(&format!(
                "  <pre><code class=\"language-{}\">{}</code></pre>\n",
                escape_html(lang),
                highlighted
            ));
            html.push_str("</div>\n");
            continue;
        }

        // Headings (# .. ######)
        if let Some((level, raw_title)) = parse_heading_line(trimmed) {
            let inline_html = render_inline_html(raw_title);
            let plain_title = strip_inline_markdown(raw_title);
            let base_id = slugify(&plain_title);
            let count = used_ids.entry(base_id.clone()).or_insert(0);
            let id = if *count == 0 {
                base_id
            } else {
                format!("{}-{}", base_id, count)
            };
            *count += 1;

            html.push_str(&format!(
                "<h{} id=\"{}\" class=\"doc-heading\">\n  <a class=\"heading-anchor\" href=\"#{}\" aria-hidden=\"true\">#</a>\n  {}\n</h{}>\n",
                level, id, id, inline_html, level
            ));
            continue;
        }

        // Horizontal Rules
        if is_horizontal_rule(trimmed) {
            html.push_str("<hr />\n");
            continue;
        }

        // Callouts / Admonitions / Blockquotes
        if trimmed.starts_with('>') {
            let mut quote_lines = vec![line];
            while let Some(next_line) = lines.peek() {
                if next_line.trim().starts_with('>') || (!next_line.trim().is_empty() && !next_line.starts_with('#') && !next_line.starts_with("```")) {
                    quote_lines.push(lines.next().unwrap());
                } else {
                    break;
                }
            }

            let rendered_quote = render_blockquote_or_callout(&quote_lines);
            html.push_str(&rendered_quote);
            continue;
        }

        // Tables: | Header | Header |
        if trimmed.starts_with('|') && trimmed.contains('|') {
            let mut table_lines = vec![trimmed];
            while let Some(next_line) = lines.peek() {
                let next_trim = next_line.trim();
                if next_trim.starts_with('|') {
                    table_lines.push(lines.next().unwrap().trim());
                } else {
                    break;
                }
            }

            if let Some(table_html) = render_table(&table_lines) {
                html.push_str(&table_html);
                continue;
            }
        }

        // Lists: Unordered, Ordered, Task lists
        if is_list_item(trimmed) {
            let mut list_lines = vec![line];
            while let Some(next_line) = lines.peek() {
                let next_trim = next_line.trim();
                if is_list_item(next_trim) || (!next_trim.is_empty() && next_line.starts_with("  ")) {
                    list_lines.push(lines.next().unwrap());
                } else {
                    break;
                }
            }

            html.push_str(&render_list(&list_lines));
            continue;
        }

        // Details / Collapsible summary
        if trimmed.starts_with("<details>") {
            html.push_str("<details class=\"doc-details\">\n");
            continue;
        }
        if trimmed.starts_with("</details>") {
            html.push_str("</details>\n");
            continue;
        }
        if trimmed.starts_with("<summary>") && trimmed.ends_with("</summary>") {
            let inner = trimmed.strip_prefix("<summary>").unwrap().strip_suffix("</summary>").unwrap();
            html.push_str(&format!("<summary>{}</summary>\n", render_inline_html(inner)));
            continue;
        }

        // Paragraphs
        let mut para_lines = vec![trimmed];
        while let Some(next_line) = lines.peek() {
            let next_trim = next_line.trim();
            if next_trim.is_empty()
                || next_trim.starts_with('#')
                || next_trim.starts_with("```")
                || next_trim.starts_with('>')
                || next_trim.starts_with('|')
                || is_list_item(next_trim)
                || is_horizontal_rule(next_trim)
                || next_trim.starts_with("<details>")
                || next_trim.starts_with("</details>")
            {
                break;
            }
            para_lines.push(lines.next().unwrap().trim());
        }

        let para_content = para_lines.join(" ");
        html.push_str(&format!("<p>{}</p>\n", render_inline_html(&para_content)));
    }

    html
}

fn is_horizontal_rule(s: &str) -> bool {
    if s.len() < 3 {
        return false;
    }
    let first = s.chars().next().unwrap();
    if first != '-' && first != '*' && first != '_' {
        return false;
    }
    s.chars().all(|c| c == first || c.is_whitespace())
}

fn is_list_item(s: &str) -> bool {
    let trimmed = s.trim_start();
    if trimmed.starts_with("- ") || trimmed.starts_with("* ") || trimmed.starts_with("+ ") {
        return true;
    }
    // Ordered: 1. 2. etc
    if let Some(pos) = trimmed.find('.') {
        let num_part = &trimmed[..pos];
        if !num_part.is_empty() && num_part.chars().all(|c| c.is_ascii_digit()) {
            let rest = &trimmed[pos + 1..];
            return rest.starts_with(' ');
        }
    }
    false
}

fn render_list(lines: &[&str]) -> String {
    let mut out = String::new();
    let mut is_ordered = false;

    // Check first item kind
    if let Some(first) = lines.first() {
        let trim = first.trim_start();
        if let Some(pos) = trim.find('.') {
            if trim[..pos].chars().all(|c| c.is_ascii_digit()) {
                is_ordered = true;
            }
        }
    }

    let tag = if is_ordered { "ol" } else { "ul" };
    out.push_str(&format!("<{}>\n", tag));

    for line in lines {
        let trim = line.trim();
        let content = if trim.starts_with("- ") || trim.starts_with("* ") || trim.starts_with("+ ") {
            &trim[2..]
        } else if let Some(pos) = trim.find(". ") {
            if trim[..pos].chars().all(|c| c.is_ascii_digit()) {
                &trim[pos + 2..]
            } else {
                trim
            }
        } else {
            trim
        };

        // Task list items: [ ] or [x]
        if content.starts_with("[ ] ") {
            out.push_str("  <li class=\"task-list-item\"><input type=\"checkbox\" disabled /> ");
            out.push_str(&render_inline_html(&content[4..]));
            out.push_str("</li>\n");
        } else if content.starts_with("[x] ") || content.starts_with("[X] ") {
            out.push_str("  <li class=\"task-list-item\"><input type=\"checkbox\" checked disabled /> ");
            out.push_str(&render_inline_html(&content[4..]));
            out.push_str("</li>\n");
        } else {
            out.push_str("  <li>");
            out.push_str(&render_inline_html(content));
            out.push_str("</li>\n");
        }
    }

    out.push_str(&format!("</{}>\n", tag));
    out
}

fn render_blockquote_or_callout(lines: &[&str]) -> String {
    let mut cleaned_lines = Vec::new();
    for line in lines {
        let trimmed = line.trim();
        if let Some(stripped) = trimmed.strip_prefix('>') {
            cleaned_lines.push(stripped.trim());
        } else {
            cleaned_lines.push(trimmed);
        }
    }

    if cleaned_lines.is_empty() {
        return String::new();
    }

    let first = cleaned_lines[0];
    let callouts = [
        ("[!NOTE]", "note", "Note", "ℹ️"),
        ("[!TIP]", "tip", "Tip", "💡"),
        ("[!IMPORTANT]", "important", "Important", "🔔"),
        ("[!WARNING]", "warning", "Warning", "⚠️"),
        ("[!CAUTION]", "danger", "Caution", "🚨"),
        ("[!DANGER]", "danger", "Danger", "🚨"),
        ("[!INFO]", "info", "Info", "ℹ️"),
    ];

    for (tag, class_name, default_title, icon) in callouts {
        if first.starts_with(tag) {
            let custom_title = first[tag.len()..].trim();
            let title = if custom_title.is_empty() {
                default_title
            } else {
                custom_title
            };

            let body_lines = &cleaned_lines[1..];
            let body = body_lines.join(" ");

            return format!(
                "<div class=\"admonition admonition-{}\">\n  <div class=\"admonition-title\"><span class=\"admonition-icon\">{}</span> {}</div>\n  <div class=\"admonition-content\">\n    <p>{}</p>\n  </div>\n</div>\n",
                class_name, icon, escape_html(title), render_inline_html(&body)
            );
        }
    }

    // Standard blockquote
    let body = cleaned_lines.join(" ");
    format!("<blockquote><p>{}</p></blockquote>\n", render_inline_html(&body))
}

fn render_table(lines: &[&str]) -> Option<String> {
    if lines.len() < 2 {
        return None;
    }

    let header_line = lines[0];
    let delimiter_line = lines[1];

    let headers = parse_table_row(header_line);
    let delimiters = parse_table_row(delimiter_line);

    if headers.is_empty() || delimiters.len() != headers.len() {
        return None;
    }

    let alignments: Vec<&str> = delimiters
        .iter()
        .map(|d| {
            let trim = d.trim();
            let left = trim.starts_with(':');
            let right = trim.ends_with(':');
            if left && right {
                "center"
            } else if right {
                "right"
            } else {
                "left"
            }
        })
        .collect();

    let mut out = String::new();
    out.push_str("<div class=\"table-container\">\n  <table class=\"doc-table\">\n    <thead>\n      <tr>\n");

    for (i, h) in headers.iter().enumerate() {
        let align = alignments.get(i).unwrap_or(&"left");
        out.push_str(&format!(
            "        <th style=\"text-align: {}\">{}</th>\n",
            align,
            render_inline_html(h)
        ));
    }

    out.push_str("      </tr>\n    </thead>\n    <tbody>\n");

    for row_line in &lines[2..] {
        let cells = parse_table_row(row_line);
        if cells.is_empty() {
            continue;
        }
        out.push_str("      <tr>\n");
        for (i, cell) in cells.iter().enumerate() {
            let align = alignments.get(i).unwrap_or(&"left");
            out.push_str(&format!(
                "        <td style=\"text-align: {}\">{}</td>\n",
                align,
                render_inline_html(cell)
            ));
        }
        out.push_str("      </tr>\n");
    }

    out.push_str("    </tbody>\n  </table>\n</div>\n");
    Some(out)
}

fn parse_table_row(line: &str) -> Vec<String> {
    let trimmed = line.trim();
    let content = if trimmed.starts_with('|') && trimmed.ends_with('|') && trimmed.len() >= 2 {
        &trimmed[1..trimmed.len() - 1]
    } else {
        trimmed
    };

    content
        .split('|')
        .map(|c| c.trim().to_string())
        .collect()
}

// ============================================================================
// Table of Contents Generator
// ============================================================================

/// Generates an HTML Table of Contents widget from a list of headings.
pub fn generate_toc_html(headings: &[HeadingItem]) -> String {
    if headings.is_empty() {
        return String::new();
    }

    let mut out = String::new();
    out.push_str("<nav class=\"doc-toc\" aria-label=\"Table of Contents\">\n");
    out.push_str("  <div class=\"toc-header\">On this page</div>\n");
    out.push_str("  <ul class=\"toc-list\">\n");

    for h in headings {
        let indent_class = match h.level {
            1 => "toc-h1",
            2 => "toc-h2",
            3 => "toc-h3",
            4 => "toc-h4",
            5 => "toc-h5",
            _ => "toc-h6",
        };

        out.push_str(&format!(
            "    <li class=\"toc-item {}\"><a href=\"#{}\" class=\"toc-link\">{}</a></li>\n",
            indent_class,
            escape_html(&h.id),
            escape_html(&h.title)
        ));
    }

    out.push_str("  </ul>\n");
    out.push_str("</nav>\n");
    out
}

// ============================================================================
// CSS Stylesheets & Theming
// ============================================================================

/// Generates the embedded modern CSS for document layout, typography, themes, and responsive design.
pub fn generate_embedded_css(config: &DocConfig) -> String {
    let theme_vars: std::borrow::Cow<'_, str> = match &config.theme {
        DocTheme::Light => generate_theme_variables_light().into(),
        DocTheme::Dark => generate_theme_variables_dark().into(),
        DocTheme::Nord => generate_theme_variables_nord().into(),
        DocTheme::Cyberpunk => generate_theme_variables_cyberpunk().into(),
        DocTheme::SolarizedLight => generate_theme_variables_solarized_light().into(),
        DocTheme::SolarizedDark => generate_theme_variables_solarized_dark().into(),
        DocTheme::Custom(custom) => generate_theme_variables_custom(custom).into(),
        DocTheme::Auto => generate_theme_variables_auto().into(),
    };

    let custom_user_css = config.custom_css.as_deref().unwrap_or("");

    format!(
        r#"
/* Fusion Standalone Documentation Engine CSS */
:root {{
{theme_vars}
  --font-sans: -apple-system, BlinkMacSystemFont, "Segoe UI", Roboto, Oxygen, Ubuntu, Cantarell, "Helvetica Neue", sans-serif;
  --font-mono: ui-monospace, SFMono-Regular, "SF Mono", Menlo, Consolas, "Liberation Mono", monospace;
  --sidebar-width: 280px;
  --toc-width: 240px;
  --header-height: 60px;
  --radius-sm: 4px;
  --radius-md: 8px;
  --radius-lg: 12px;
  --shadow-sm: 0 1px 2px 0 rgba(0, 0, 0, 0.05);
  --shadow-md: 0 4px 6px -1px rgba(0, 0, 0, 0.1), 0 2px 4px -1px rgba(0, 0, 0, 0.06);
  --shadow-lg: 0 10px 15px -3px rgba(0, 0, 0, 0.1), 0 4px 6px -2px rgba(0, 0, 0, 0.05);
  --transition: all 0.2s cubic-bezier(0.4, 0, 0.2, 1);
}}

*, *::before, *::after {{
  box-sizing: border-box;
  margin: 0;
  padding: 0;
}}

html {{
  font-family: var(--font-sans);
  font-size: 16px;
  line-height: 1.7;
  color: var(--doc-text-primary);
  background-color: var(--doc-bg-primary);
  scroll-behavior: smooth;
  text-rendering: optimizeLegibility;
  -webkit-font-smoothing: antialiased;
}}

body {{
  min-height: 100vh;
  display: flex;
  flex-direction: column;
}}

/* Top Navigation Bar */
.doc-header {{
  position: sticky;
  top: 0;
  z-index: 50;
  height: var(--header-height);
  background: var(--doc-bg-primary);
  border-bottom: 1px solid var(--doc-border);
  backdrop-filter: blur(8px);
  display: flex;
  align-items: center;
  justify-content: space-between;
  padding: 0 1.5rem;
}}

.header-brand {{
  display: flex;
  align-items: center;
  gap: 0.75rem;
  text-decoration: none;
  color: var(--doc-text-primary);
  font-weight: 700;
  font-size: 1.15rem;
}}

.header-logo {{
  height: 28px;
  width: auto;
}}

.header-actions {{
  display: flex;
  align-items: center;
  gap: 1rem;
}}

.search-box {{
  position: relative;
  display: flex;
  align-items: center;
}}

.search-input {{
  background: var(--doc-bg-secondary);
  border: 1px solid var(--doc-border);
  border-radius: var(--radius-md);
  padding: 0.4rem 0.8rem 0.4rem 2.2rem;
  color: var(--doc-text-primary);
  font-size: 0.875rem;
  width: 220px;
  transition: var(--transition);
}}

.search-input:focus {{
  outline: none;
  border-color: var(--doc-accent);
  width: 280px;
  box-shadow: 0 0 0 2px var(--doc-accent-subtle);
}}

.search-icon {{
  position: absolute;
  left: 0.75rem;
  color: var(--doc-text-muted);
  pointer-events: none;
}}

.btn-icon {{
  background: transparent;
  border: 1px solid var(--doc-border);
  color: var(--doc-text-secondary);
  cursor: pointer;
  padding: 0.45rem;
  border-radius: var(--radius-md);
  display: inline-flex;
  align-items: center;
  justify-content: center;
  transition: var(--transition);
}}

.btn-icon:hover {{
  color: var(--doc-text-primary);
  background: var(--doc-bg-secondary);
  border-color: var(--doc-border-hover);
}}

/* Main Documentation Layout */
.doc-layout {{
  display: flex;
  flex: 1;
  max-width: 1440px;
  margin: 0 auto;
  width: 100%;
}}

/* Sidebar Navigation */
.doc-sidebar {{
  width: var(--sidebar-width);
  flex-shrink: 0;
  position: sticky;
  top: var(--header-height);
  height: calc(100vh - var(--header-height));
  overflow-y: auto;
  border-right: 1px solid var(--doc-border);
  padding: 1.5rem 1rem;
}}

.nav-section {{
  margin-bottom: 1.5rem;
}}

.nav-section-title {{
  font-size: 0.75rem;
  font-weight: 700;
  text-transform: uppercase;
  letter-spacing: 0.05em;
  color: var(--doc-text-muted);
  margin-bottom: 0.5rem;
  padding: 0 0.75rem;
}}

.nav-list {{
  list-style: none;
}}

.nav-item-link {{
  display: flex;
  align-items: center;
  justify-content: space-between;
  padding: 0.45rem 0.75rem;
  border-radius: var(--radius-md);
  color: var(--doc-text-secondary);
  text-decoration: none;
  font-size: 0.9rem;
  font-weight: 500;
  transition: var(--transition);
}}

.nav-item-link:hover {{
  color: var(--doc-text-primary);
  background: var(--doc-bg-secondary);
}}

.nav-item-link.active {{
  color: var(--doc-accent);
  background: var(--doc-accent-subtle);
  font-weight: 600;
}}

.nav-badge {{
  font-size: 0.7rem;
  padding: 0.15rem 0.45rem;
  border-radius: 999px;
  background: var(--doc-accent);
  color: #ffffff;
  font-weight: 600;
}}

/* Main Content Area */
.doc-main {{
  flex: 1;
  min-width: 0;
  padding: 2rem 3rem;
}}

.doc-breadcrumbs {{
  display: flex;
  align-items: center;
  gap: 0.5rem;
  font-size: 0.85rem;
  color: var(--doc-text-muted);
  margin-bottom: 1.5rem;
}}

.doc-breadcrumbs a {{
  color: var(--doc-text-muted);
  text-decoration: none;
}}

.doc-breadcrumbs a:hover {{
  color: var(--doc-accent);
}}

.doc-article {{
  max-width: 820px;
}}

/* Right Rail TOC */
.doc-aside {{
  width: var(--toc-width);
  flex-shrink: 0;
  position: sticky;
  top: var(--header-height);
  height: calc(100vh - var(--header-height));
  overflow-y: auto;
  padding: 2rem 1rem;
}}

.doc-toc {{
  font-size: 0.85rem;
}}

.toc-header {{
  font-weight: 600;
  color: var(--doc-text-primary);
  margin-bottom: 0.75rem;
  text-transform: uppercase;
  letter-spacing: 0.05em;
  font-size: 0.75rem;
}}

.toc-list {{
  list-style: none;
  border-left: 2px solid var(--doc-border);
}}

.toc-item {{
  margin: 0.35rem 0;
}}

.toc-link {{
  display: block;
  padding: 0.2rem 0 0.2rem 0.75rem;
  color: var(--doc-text-secondary);
  text-decoration: none;
  transition: var(--transition);
  border-left: 2px solid transparent;
  margin-left: -2px;
}}

.toc-link:hover, .toc-link.active {{
  color: var(--doc-accent);
  border-left-color: var(--doc-accent);
}}

.toc-h2 {{ padding-left: 0.5rem; }}
.toc-h3 {{ padding-left: 1.25rem; font-size: 0.8rem; }}
.toc-h4 {{ padding-left: 2rem; font-size: 0.75rem; }}

/* Typography & Markdown Styles */
h1, h2, h3, h4, h5, h6 {{
  color: var(--doc-text-primary);
  font-weight: 700;
  line-height: 1.3;
  margin-top: 2rem;
  margin-bottom: 1rem;
  position: relative;
}}

h1 {{ font-size: 2.25rem; margin-top: 0; border-bottom: 1px solid var(--doc-border); padding-bottom: 0.5rem; }}
h2 {{ font-size: 1.6rem; border-bottom: 1px solid var(--doc-border-subtle); padding-bottom: 0.4rem; }}
h3 {{ font-size: 1.25rem; }}
h4 {{ font-size: 1.05rem; }}

.heading-anchor {{
  position: absolute;
  left: -1.25rem;
  opacity: 0;
  text-decoration: none;
  color: var(--doc-accent);
  font-weight: 400;
  transition: var(--transition);
}}

.doc-heading:hover .heading-anchor {{
  opacity: 1;
}}

p, ul, ol, blockquote, .table-container, .code-block-wrapper, .admonition {{
  margin-bottom: 1.25rem;
}}

a {{
  color: var(--doc-text-link);
  text-decoration: none;
  transition: var(--transition);
}}

a:hover {{
  text-decoration: underline;
}}

strong {{ font-weight: 600; color: var(--doc-text-primary); }}
em {{ font-style: italic; }}
mark {{ background: var(--doc-mark-bg); color: var(--doc-text-primary); padding: 0.1rem 0.3rem; border-radius: var(--radius-sm); }}
del {{ text-decoration: line-through; opacity: 0.7; }}

code {{
  font-family: var(--font-mono);
  font-size: 0.875em;
  background: var(--doc-code-inline-bg);
  color: var(--doc-accent);
  padding: 0.2em 0.4em;
  border-radius: var(--radius-sm);
}}

kbd {{
  font-family: var(--font-mono);
  font-size: 0.8em;
  background: var(--doc-bg-secondary);
  border: 1px solid var(--doc-border);
  box-shadow: 0 1px 0 var(--doc-border);
  padding: 0.15em 0.4em;
  border-radius: var(--radius-sm);
}}

hr {{
  border: 0;
  height: 1px;
  background: var(--doc-border);
  margin: 2.5rem 0;
}}

/* Lists */
ul, ol {{
  padding-left: 1.75rem;
}}

li {{
  margin-bottom: 0.4rem;
}}

.task-list-item {{
  list-style: none;
  margin-left: -1.25rem;
  display: flex;
  align-items: center;
  gap: 0.5rem;
}}

.task-list-item input[type="checkbox"] {{
  accent-color: var(--doc-accent);
  cursor: pointer;
}}

/* Code Blocks */
.code-block-wrapper {{
  background: var(--doc-code-bg);
  border: 1px solid var(--doc-border);
  border-radius: var(--radius-lg);
  overflow: hidden;
  box-shadow: var(--shadow-sm);
}}

.code-header {{
  display: flex;
  align-items: center;
  justify-content: space-between;
  padding: 0.5rem 1rem;
  background: rgba(0, 0, 0, 0.15);
  border-bottom: 1px solid var(--doc-border);
}}

.code-lang-badge {{
  font-family: var(--font-mono);
  font-size: 0.75rem;
  font-weight: 600;
  text-transform: uppercase;
  color: var(--doc-text-muted);
}}

.copy-code-btn {{
  background: transparent;
  border: 1px solid var(--doc-border);
  color: var(--doc-text-secondary);
  border-radius: var(--radius-sm);
  padding: 0.25rem 0.5rem;
  font-size: 0.75rem;
  cursor: pointer;
  display: flex;
  align-items: center;
  gap: 0.35rem;
  transition: var(--transition);
}}

.copy-code-btn:hover {{
  color: var(--doc-text-primary);
  background: rgba(255, 255, 255, 0.05);
}}

.code-block-wrapper pre {{
  padding: 1.25rem;
  overflow-x: auto;
  font-family: var(--font-mono);
  font-size: 0.9rem;
  line-height: 1.6;
  color: var(--doc-text-primary);
}}

.code-block-wrapper pre code {{
  background: transparent;
  color: inherit;
  padding: 0;
  font-size: inherit;
}}

/* Syntax Highlighting Colors */
.hl-kw {{ color: var(--hl-kw); font-weight: 600; }}
.hl-str {{ color: var(--hl-str); }}
.hl-num {{ color: var(--hl-num); }}
.hl-comment {{ color: var(--hl-comment); font-style: italic; }}
.hl-type {{ color: var(--hl-type); }}
.hl-fn {{ color: var(--hl-fn); }}
.hl-builtin {{ color: var(--hl-builtin); }}

/* Admonitions / Callouts */
.admonition {{
  border-left: 4px solid;
  border-radius: var(--radius-md);
  padding: 1rem 1.25rem;
  background: var(--doc-bg-secondary);
}}

.admonition-title {{
  font-weight: 700;
  font-size: 0.95rem;
  margin-bottom: 0.5rem;
  display: flex;
  align-items: center;
  gap: 0.5rem;
}}

.admonition-note {{ border-color: #3b82f6; background: rgba(59, 130, 246, 0.08); }}
.admonition-note .admonition-title {{ color: #3b82f6; }}

.admonition-tip {{ border-color: #10b981; background: rgba(16, 185, 129, 0.08); }}
.admonition-tip .admonition-title {{ color: #10b981; }}

.admonition-important {{ border-color: #8b5cf6; background: rgba(139, 92, 246, 0.08); }}
.admonition-important .admonition-title {{ color: #8b5cf6; }}

.admonition-warning {{ border-color: #f59e0b; background: rgba(245, 158, 11, 0.08); }}
.admonition-warning .admonition-title {{ color: #f59e0b; }}

.admonition-danger {{ border-color: #ef4444; background: rgba(239, 68, 68, 0.08); }}
.admonition-danger .admonition-title {{ color: #ef4444; }}

.admonition-info {{ border-color: #06b6d4; background: rgba(6, 182, 212, 0.08); }}
.admonition-info .admonition-title {{ color: #06b6d4; }}

/* Blockquotes */
blockquote {{
  border-left: 4px solid var(--doc-accent);
  padding: 0.75rem 1.25rem;
  background: var(--doc-bg-secondary);
  border-radius: var(--radius-sm);
  color: var(--doc-text-secondary);
  font-style: italic;
}}

/* Tables */
.table-container {{
  overflow-x: auto;
  border: 1px solid var(--doc-border);
  border-radius: var(--radius-lg);
  box-shadow: var(--shadow-sm);
}}

.doc-table {{
  width: 100%;
  border-collapse: collapse;
  font-size: 0.95rem;
  text-align: left;
}}

.doc-table th, .doc-table td {{
  padding: 0.75rem 1rem;
  border-bottom: 1px solid var(--doc-border);
}}

.doc-table th {{
  background: var(--doc-bg-secondary);
  font-weight: 600;
  color: var(--doc-text-primary);
}}

.doc-table tr:last-child td {{
  border-bottom: none;
}}

.doc-table tbody tr:hover {{
  background: rgba(255, 255, 255, 0.02);
}}

/* Details / Summary */
.doc-details {{
  background: var(--doc-bg-secondary);
  border: 1px solid var(--doc-border);
  border-radius: var(--radius-md);
  padding: 0.75rem 1rem;
  margin-bottom: 1.25rem;
}}

.doc-details summary {{
  font-weight: 600;
  cursor: pointer;
  outline: none;
}}

/* Footer */
.doc-footer {{
  margin-top: auto;
  border-top: 1px solid var(--doc-border);
  padding: 2rem 1.5rem;
  text-align: center;
  font-size: 0.85rem;
  color: var(--doc-text-muted);
  background: var(--doc-bg-secondary);
}}

/* Back to top button */
.back-to-top {{
  position: fixed;
  bottom: 2rem;
  right: 2rem;
  background: var(--doc-accent);
  color: #ffffff;
  border: none;
  border-radius: 999px;
  width: 42px;
  height: 42px;
  display: flex;
  align-items: center;
  justify-content: center;
  box-shadow: var(--shadow-lg);
  cursor: pointer;
  opacity: 0;
  pointer-events: none;
  transition: var(--transition);
  z-index: 40;
}}

.back-to-top.visible {{
  opacity: 1;
  pointer-events: auto;
}}

/* Responsive Design */
@media (max-width: 1024px) {{
  .doc-aside {{ display: none; }}
  .doc-main {{ padding: 2rem 1.5rem; }}
}}

@media (max-width: 768px) {{
  .doc-sidebar {{
    position: fixed;
    left: -100%;
    top: var(--header-height);
    z-index: 45;
    background: var(--doc-bg-primary);
    transition: left 0.3s ease;
    box-shadow: var(--shadow-lg);
  }}
  .doc-sidebar.open {{
    left: 0;
  }}
  .search-input {{ width: 160px; }}
  .search-input:focus {{ width: 200px; }}
}}

{custom_user_css}
"#
    )
}

fn generate_theme_variables_auto() -> &'static str {
    r#"
  /* Auto theme: defaults to Dark, overrides for Light */
  --doc-bg-primary: #0f172a;
  --doc-bg-secondary: #1e293b;
  --doc-code-bg: #0b1120;
  --doc-code-inline-bg: #1e293b;
  --doc-text-primary: #f8fafc;
  --doc-text-secondary: #cbd5e1;
  --doc-text-muted: #64748b;
  --doc-text-link: #38bdf8;
  --doc-accent: #6366f1;
  --doc-accent-hover: #4f46e5;
  --doc-accent-subtle: rgba(99, 102, 241, 0.15);
  --doc-border: #334155;
  --doc-border-subtle: #1e293b;
  --doc-border-hover: #475569;
  --doc-mark-bg: rgba(234, 179, 8, 0.25);
  
  --hl-kw: #f43f5e;
  --hl-str: #34d399;
  --hl-num: #fb923c;
  --hl-comment: #64748b;
  --hl-type: #38bdf8;
  --hl-fn: #818cf8;
  --hl-builtin: #a78bfa;
}

@media (prefers-color-scheme: light) {
  :root {
    --doc-bg-primary: #ffffff;
    --doc-bg-secondary: #f8fafc;
    --doc-code-bg: #f1f5f9;
    --doc-code-inline-bg: #f1f5f9;
    --doc-text-primary: #0f172a;
    --doc-text-secondary: #334155;
    --doc-text-muted: #64748b;
    --doc-text-link: #2563eb;
    --doc-accent: #4f46e5;
    --doc-accent-hover: #4338ca;
    --doc-accent-subtle: rgba(79, 70, 229, 0.1);
    --doc-border: #e2e8f0;
    --doc-border-subtle: #f1f5f9;
    --doc-border-hover: #cbd5e1;
    --doc-mark-bg: rgba(254, 240, 138, 0.7);
    
    --hl-kw: #d946ef;
    --hl-str: #059669;
    --hl-num: #d97706;
    --hl-comment: #94a3b8;
    --hl-type: #0284c7;
    --hl-fn: #4f46e5;
    --hl-builtin: #7c3aed;
  }
}

[data-theme="light"] {
  --doc-bg-primary: #ffffff;
  --doc-bg-secondary: #f8fafc;
  --doc-code-bg: #f1f5f9;
  --doc-code-inline-bg: #f1f5f9;
  --doc-text-primary: #0f172a;
  --doc-text-secondary: #334155;
  --doc-text-muted: #64748b;
  --doc-text-link: #2563eb;
  --doc-accent: #4f46e5;
  --doc-accent-hover: #4338ca;
  --doc-accent-subtle: rgba(79, 70, 229, 0.1);
  --doc-border: #e2e8f0;
  --doc-border-subtle: #f1f5f9;
  --doc-border-hover: #cbd5e1;
  --doc-mark-bg: rgba(254, 240, 138, 0.7);
  
  --hl-kw: #d946ef;
  --hl-str: #059669;
  --hl-num: #d97706;
  --hl-comment: #94a3b8;
  --hl-type: #0284c7;
  --hl-fn: #4f46e5;
  --hl-builtin: #7c3aed;
}

[data-theme="dark"] {
  --doc-bg-primary: #0f172a;
  --doc-bg-secondary: #1e293b;
  --doc-code-bg: #0b1120;
  --doc-code-inline-bg: #1e293b;
  --doc-text-primary: #f8fafc;
  --doc-text-secondary: #cbd5e1;
  --doc-text-muted: #64748b;
  --doc-text-link: #38bdf8;
  --doc-accent: #6366f1;
  --doc-accent-hover: #4f46e5;
  --doc-accent-subtle: rgba(99, 102, 241, 0.15);
  --doc-border: #334155;
  --doc-border-subtle: #1e293b;
  --doc-border-hover: #475569;
  --doc-mark-bg: rgba(234, 179, 8, 0.25);
  
  --hl-kw: #f43f5e;
  --hl-str: #34d399;
  --hl-num: #fb923c;
  --hl-comment: #64748b;
  --hl-type: #38bdf8;
  --hl-fn: #818cf8;
  --hl-builtin: #a78bfa;
"#
}

fn generate_theme_variables_light() -> &'static str {
    r#"
  --doc-bg-primary: #ffffff;
  --doc-bg-secondary: #f8fafc;
  --doc-code-bg: #f1f5f9;
  --doc-code-inline-bg: #f1f5f9;
  --doc-text-primary: #0f172a;
  --doc-text-secondary: #334155;
  --doc-text-muted: #64748b;
  --doc-text-link: #2563eb;
  --doc-accent: #4f46e5;
  --doc-accent-hover: #4338ca;
  --doc-accent-subtle: rgba(79, 70, 229, 0.1);
  --doc-border: #e2e8f0;
  --doc-border-subtle: #f1f5f9;
  --doc-border-hover: #cbd5e1;
  --doc-mark-bg: rgba(254, 240, 138, 0.7);
  
  --hl-kw: #d946ef;
  --hl-str: #059669;
  --hl-num: #d97706;
  --hl-comment: #94a3b8;
  --hl-type: #0284c7;
  --hl-fn: #4f46e5;
  --hl-builtin: #7c3aed;
"#
}

fn generate_theme_variables_dark() -> &'static str {
    r#"
  --doc-bg-primary: #0f172a;
  --doc-bg-secondary: #1e293b;
  --doc-code-bg: #0b1120;
  --doc-code-inline-bg: #1e293b;
  --doc-text-primary: #f8fafc;
  --doc-text-secondary: #cbd5e1;
  --doc-text-muted: #64748b;
  --doc-text-link: #38bdf8;
  --doc-accent: #6366f1;
  --doc-accent-hover: #4f46e5;
  --doc-accent-subtle: rgba(99, 102, 241, 0.15);
  --doc-border: #334155;
  --doc-border-subtle: #1e293b;
  --doc-border-hover: #475569;
  --doc-mark-bg: rgba(234, 179, 8, 0.25);
  
  --hl-kw: #f43f5e;
  --hl-str: #34d399;
  --hl-num: #fb923c;
  --hl-comment: #64748b;
  --hl-type: #38bdf8;
  --hl-fn: #818cf8;
  --hl-builtin: #a78bfa;
"#
}

fn generate_theme_variables_nord() -> &'static str {
    r#"
  --doc-bg-primary: #2e3440;
  --doc-bg-secondary: #3b4252;
  --doc-code-bg: #242933;
  --doc-code-inline-bg: #3b4252;
  --doc-text-primary: #eceff4;
  --doc-text-secondary: #e5e9f0;
  --doc-text-muted: #7b88a1;
  --doc-text-link: #88c0d0;
  --doc-accent: #88c0d0;
  --doc-accent-hover: #81a1c1;
  --doc-accent-subtle: rgba(136, 192, 208, 0.15);
  --doc-border: #434c5e;
  --doc-border-subtle: #3b4252;
  --doc-border-hover: #4c566a;
  --doc-mark-bg: rgba(235, 203, 139, 0.25);
  
  --hl-kw: #81a1c1;
  --hl-str: #a3be8c;
  --hl-num: #b48ead;
  --hl-comment: #616e88;
  --hl-type: #8fbcbb;
  --hl-fn: #88c0d0;
  --hl-builtin: #d08770;
"#
}

fn generate_theme_variables_cyberpunk() -> &'static str {
    r#"
  --doc-bg-primary: #05050d;
  --doc-bg-secondary: #0d0d1e;
  --doc-code-bg: #000000;
  --doc-code-inline-bg: #14142b;
  --doc-text-primary: #00ffcc;
  --doc-text-secondary: #e0f2fe;
  --doc-text-muted: #6272a4;
  --doc-text-link: #ff007f;
  --doc-accent: #ff007f;
  --doc-accent-hover: #ff3399;
  --doc-accent-subtle: rgba(255, 0, 127, 0.15);
  --doc-border: #2a2b4a;
  --doc-border-subtle: #191a2e;
  --doc-border-hover: #ff007f;
  --doc-mark-bg: rgba(255, 255, 0, 0.3);
  
  --hl-kw: #ff007f;
  --hl-str: #00ffcc;
  --hl-num: #ffff00;
  --hl-comment: #6272a4;
  --hl-type: #ff79c6;
  --hl-fn: #00e5ff;
  --hl-builtin: #bd93f9;
"#
}

fn generate_theme_variables_solarized_light() -> &'static str {
    r#"
  --doc-bg-primary: #fdf6e3;
  --doc-bg-secondary: #eee8d5;
  --doc-code-bg: #eee8d5;
  --doc-code-inline-bg: #eee8d5;
  --doc-text-primary: #657b83;
  --doc-text-secondary: #586e75;
  --doc-text-muted: #93a1a1;
  --doc-text-link: #268bd2;
  --doc-accent: #268bd2;
  --doc-accent-hover: #2aa198;
  --doc-accent-subtle: rgba(38, 139, 210, 0.15);
  --doc-border: #d33682;
  --doc-border-subtle: #e0d9c4;
  --doc-border-hover: #b58900;
  --doc-mark-bg: rgba(181, 137, 0, 0.25);
  
  --hl-kw: #859900;
  --hl-str: #2aa198;
  --hl-num: #d33682;
  --hl-comment: #93a1a1;
  --hl-type: #b58900;
  --hl-fn: #268bd2;
  --hl-builtin: #6c71c4;
"#
}

fn generate_theme_variables_solarized_dark() -> &'static str {
    r#"
  --doc-bg-primary: #002b36;
  --doc-bg-secondary: #073642;
  --doc-code-bg: #073642;
  --doc-code-inline-bg: #073642;
  --doc-text-primary: #839496;
  --doc-text-secondary: #93a1a1;
  --doc-text-muted: #586e75;
  --doc-text-link: #268bd2;
  --doc-accent: #2aa198;
  --doc-accent-hover: #268bd2;
  --doc-accent-subtle: rgba(42, 161, 152, 0.15);
  --doc-border: #073642;
  --doc-border-subtle: #00212b;
  --doc-border-hover: #586e75;
  --doc-mark-bg: rgba(181, 137, 0, 0.25);
  
  --hl-kw: #859900;
  --hl-str: #2aa198;
  --hl-num: #d33682;
  --hl-comment: #586e75;
  --hl-type: #b58900;
  --hl-fn: #268bd2;
  --hl-builtin: #6c71c4;
"#
}

fn generate_theme_variables_custom(c: &ThemeColors) -> String {
    format!(
        r#"
  --doc-bg-primary: {};
  --doc-bg-secondary: {};
  --doc-code-bg: {};
  --doc-code-inline-bg: {};
  --doc-text-primary: {};
  --doc-text-secondary: {};
  --doc-text-muted: {};
  --doc-text-link: {};
  --doc-accent: {};
  --doc-accent-hover: {};
  --doc-accent-subtle: rgba(99, 102, 241, 0.15);
  --doc-border: {};
  --doc-border-subtle: {};
  --doc-border-hover: {};
  --doc-mark-bg: rgba(234, 179, 8, 0.25);
  
  --hl-kw: #f43f5e;
  --hl-str: #34d399;
  --hl-num: #fb923c;
  --hl-comment: #64748b;
  --hl-type: #38bdf8;
  --hl-fn: #818cf8;
  --hl-builtin: #a78bfa;
"#,
        c.bg_primary,
        c.bg_secondary,
        c.bg_code,
        c.bg_secondary,
        c.text_primary,
        c.text_secondary,
        c.text_muted,
        c.text_link,
        c.accent,
        c.accent_hover,
        c.border,
        c.border_subtle,
        c.border
    )
}

// ============================================================================
// Embedded Vanilla JavaScript
// ============================================================================

/// Generates the embedded offline vanilla JavaScript for interaction.
pub fn generate_embedded_js(config: &DocConfig) -> String {
    let custom_user_js = config.custom_js.as_deref().unwrap_or("");

    format!(
        r#"
// Fusion Documentation Engine Runtime JS
function copyCode(btn) {{
  const pre = btn.closest('.code-block-wrapper').querySelector('pre');
  const code = pre.innerText;
  navigator.clipboard.writeText(code).then(() => {{
    const label = btn.querySelector('.copy-label');
    const originalText = label.innerText;
    label.innerText = 'Copied!';
    btn.classList.add('copied');
    setTimeout(() => {{
      label.innerText = originalText;
      btn.classList.remove('copied');
    }}, 2000);
  }}).catch(err => {{
    console.error('Failed to copy: ', err);
  }});
}}

function toggleTheme() {{
  const root = document.documentElement;
  const currentTheme = root.getAttribute('data-theme');
  const newTheme = currentTheme === 'dark' ? 'light' : 'dark';
  root.setAttribute('data-theme', newTheme);
  try {{
    localStorage.setItem('fusion-doc-theme', newTheme);
  }} catch (e) {{}}
}}

function toggleMobileSidebar() {{
  const sidebar = document.querySelector('.doc-sidebar');
  if (sidebar) {{
    sidebar.classList.toggle('open');
  }}
}}

function scrollToTop() {{
  window.scrollTo({{ top: 0, behavior: 'smooth' }});
}}

// Initialize DOM listeners
document.addEventListener('DOMContentLoaded', () => {{
  // Restore theme preference
  try {{
    const savedTheme = localStorage.getItem('fusion-doc-theme');
    if (savedTheme) {{
      document.documentElement.setAttribute('data-theme', savedTheme);
    }}
  }} catch (e) {{}}

  // Back to top scroll listener
  const backToTopBtn = document.querySelector('.back-to-top');
  if (backToTopBtn) {{
    window.addEventListener('scroll', () => {{
      if (window.scrollY > 300) {{
        backToTopBtn.classList.add('visible');
      }} else {{
        backToTopBtn.classList.remove('visible');
      }}
    }});
  }}

  // Active heading spy for TOC
  const headings = document.querySelectorAll('.doc-heading');
  const tocLinks = document.querySelectorAll('.toc-link');
  if (headings.length > 0 && tocLinks.length > 0) {{
    const observer = new IntersectionObserver((entries) => {{
      entries.forEach(entry => {{
        if (entry.isIntersecting) {{
          const id = entry.target.getAttribute('id');
          tocLinks.forEach(link => {{
            if (link.getAttribute('href') === '#' + id) {{
              link.classList.add('active');
            }} else {{
              link.classList.remove('active');
            }}
          }});
        }}
      }});
    }}, {{ rootMargin: '0px 0px -70% 0px' }});

    headings.forEach(h => observer.observe(h));
  }}

  // In-page live search filter
  const searchInput = document.querySelector('.search-input');
  if (searchInput) {{
    searchInput.addEventListener('input', (e) => {{
      const query = e.target.value.toLowerCase().trim();
      const articles = document.querySelectorAll('.doc-article p, .doc-article h1, .doc-article h2, .doc-article h3, .doc-article li');
      if (!query) {{
        articles.forEach(el => el.style.opacity = '1');
        return;
      }}
      articles.forEach(el => {{
        if (el.innerText.toLowerCase().includes(query)) {{
          el.style.opacity = '1';
        }} else {{
          el.style.opacity = '0.3';
        }}
      }});
    }});
  }}
}});

{custom_user_js}
"#
    )
}

// ============================================================================
// Top-Level Document & Site Generators
// ============================================================================

/// Renders a full standalone HTML documentation page from markdown string.
pub fn generate_doc_page(markdown: &str, config: &DocConfig) -> String {
    let headings = extract_headings(markdown);
    let html_body = markdown_to_html(markdown);
    let toc_html = if config.show_toc {
        generate_toc_html(&headings)
    } else {
        String::new()
    };
    let css = generate_embedded_css(config);
    let js = generate_embedded_js(config);

    let mut doc = String::with_capacity(markdown.len() * 4);

    doc.push_str("<!DOCTYPE html>\n");
    doc.push_str("<html lang=\"en\">\n<head>\n");
    doc.push_str("  <meta charset=\"UTF-8\" />\n");
    doc.push_str("  <meta name=\"viewport\" content=\"width=device-width, initial-scale=1.0\" />\n");
    doc.push_str(&format!("  <title>{}</title>\n", escape_html(&config.title)));

    if let Some(desc) = &config.description {
        doc.push_str(&format!(
            "  <meta name=\"description\" content=\"{}\" />\n",
            escape_html(desc)
        ));
    }

    if let Some(author) = &config.author {
        doc.push_str(&format!(
            "  <meta name=\"author\" content=\"{}\" />\n",
            escape_html(author)
        ));
    }

    if let Some(favicon) = &config.favicon {
        doc.push_str(&format!(
            "  <link rel=\"icon\" href=\"{}\" />\n",
            escape_html(favicon)
        ));
    }

    doc.push_str(&format!("  <style>\n{}\n  </style>\n", css));
    doc.push_str("</head>\n<body>\n");

    // Header bar
    doc.push_str("  <header class=\"doc-header\">\n");
    doc.push_str("    <div style=\"display: flex; align-items: center; gap: 0.75rem;\">\n");
    if config.show_sidebar && !config.sidebar_nav.is_empty() {
        doc.push_str("      <button class=\"btn-icon mobile-menu-toggle\" onclick=\"toggleMobileSidebar()\" title=\"Toggle navigation\">\n");
        doc.push_str("        <svg width=\"20\" height=\"20\" viewBox=\"0 0 24 24\" fill=\"none\" stroke=\"currentColor\" stroke-width=\"2\"><line x1=\"3\" y1=\"12\" x2=\"21\" y2=\"12\"></line><line x1=\"3\" y1=\"6\" x2=\"21\" y2=\"6\"></line><line x1=\"3\" y1=\"18\" x2=\"21\" y2=\"18\"></line></svg>\n");
        doc.push_str("      </button>\n");
    }
    doc.push_str("      <a href=\"#\" class=\"header-brand\">\n");
    if let Some(logo) = &config.logo {
        doc.push_str(&format!(
            "        <img src=\"{}\" alt=\"Logo\" class=\"header-logo\" />\n",
            escape_html(logo)
        ));
    }
    doc.push_str(&format!("        <span>{}</span>\n", escape_html(&config.title)));
    if let Some(ver) = &config.version {
        doc.push_str(&format!(
            "        <span class=\"nav-badge\">v{}</span>\n",
            escape_html(ver)
        ));
    }
    doc.push_str("      </a>\n");
    doc.push_str("    </div>\n");

    doc.push_str("    <div class=\"header-actions\">\n");
    if config.show_search {
        doc.push_str("      <div class=\"search-box\">\n");
        doc.push_str("        <svg class=\"search-icon\" width=\"14\" height=\"14\" viewBox=\"0 0 24 24\" fill=\"none\" stroke=\"currentColor\" stroke-width=\"2\"><circle cx=\"11\" cy=\"11\" r=\"8\"></circle><line x1=\"21\" y1=\"21\" x2=\"16.65\" y2=\"16.65\"></line></svg>\n");
        doc.push_str("        <input type=\"text\" class=\"search-input\" placeholder=\"Filter doc content...\" />\n");
        doc.push_str("      </div>\n");
    }

    if let Some(repo) = &config.repo_url {
        doc.push_str(&format!(
            "      <a href=\"{}\" class=\"btn-icon\" target=\"_blank\" rel=\"noopener noreferrer\" title=\"Repository\">\n        <svg width=\"18\" height=\"18\" viewBox=\"0 0 24 24\" fill=\"currentColor\"><path d=\"M12 0c-6.626 0-12 5.373-12 12 0 5.302 3.438 9.8 8.207 11.387.599.111.793-.261.793-.577v-2.234c-3.338.726-4.033-1.416-4.033-1.416-.546-1.387-1.333-1.756-1.333-1.756-1.089-.745.083-.729.083-.729 1.205.084 1.839 1.237 1.839 1.237 1.07 1.834 2.807 1.304 3.492.997.107-.775.418-1.305.762-1.604-2.665-.305-5.467-1.334-5.467-5.931 0-1.311.469-2.381 1.236-3.221-.124-.303-.535-1.524.117-3.176 0 0 1.008-.322 3.301 1.23.957-.266 1.983-.399 3.003-.404 1.02.005 2.047.138 3.006.404 2.291-1.552 3.297-1.23 3.297-1.23.653 1.653.242 2.874.118 3.176.77.84 1.235 1.911 1.235 3.221 0 4.609-2.807 5.624-5.479 5.921.43.372.823 1.102.823 2.222v3.293c0 .319.192.694.801.576 4.765-1.589 8.199-6.086 8.199-11.386 0-6.627-5.373-12-12-12z\"/></svg>\n      </a>\n",
            escape_html(repo)
        ));
    }

    if config.show_theme_toggle {
        doc.push_str("      <button class=\"btn-icon\" onclick=\"toggleTheme()\" title=\"Toggle theme\">\n");
        doc.push_str("        <svg width=\"18\" height=\"18\" viewBox=\"0 0 24 24\" fill=\"none\" stroke=\"currentColor\" stroke-width=\"2\"><path d=\"M21 12.79A9 9 0 1 1 11.21 3 7 7 0 0 0 21 12.79z\"></path></svg>\n");
        doc.push_str("      </button>\n");
    }

    doc.push_str("    </div>\n");
    doc.push_str("  </header>\n\n");

    // Layout
    doc.push_str("  <div class=\"doc-layout\">\n");

    // Sidebar
    if config.show_sidebar && !config.sidebar_nav.is_empty() {
        doc.push_str("    <aside class=\"doc-sidebar\">\n");
        for section in &config.sidebar_nav {
            doc.push_str("      <div class=\"nav-section\">\n");
            doc.push_str(&format!(
                "        <div class=\"nav-section-title\">{}</div>\n",
                escape_html(&section.title)
            ));
            doc.push_str("        <ul class=\"nav-list\">\n");
            for item in &section.items {
                let active_class = if item.active { " active" } else { "" };
                doc.push_str(&format!(
                    "          <li><a href=\"{}\" class=\"nav-item-link{}\"><span>{}</span>",
                    escape_html(&item.url),
                    active_class,
                    escape_html(&item.title)
                ));
                if let Some(b) = &item.badge {
                    doc.push_str(&format!(
                        "<span class=\"nav-badge\">{}</span>",
                        escape_html(b)
                    ));
                }
                doc.push_str("</a></li>\n");
            }
            doc.push_str("        </ul>\n");
            doc.push_str("      </div>\n");
        }
        doc.push_str("    </aside>\n");
    }

    // Main content
    doc.push_str("    <main class=\"doc-main\">\n");

    // Breadcrumbs
    if !config.breadcrumbs.is_empty() {
        doc.push_str("      <nav class=\"doc-breadcrumbs\">\n");
        for (i, (label, url)) in config.breadcrumbs.iter().enumerate() {
            if i > 0 {
                doc.push_str("        <span>/</span>\n");
            }
            doc.push_str(&format!(
                "        <a href=\"{}\">{}</a>\n",
                escape_html(url),
                escape_html(label)
            ));
        }
        doc.push_str("      </nav>\n");
    }

    doc.push_str("      <article class=\"doc-article\">\n");
    doc.push_str(&html_body);
    doc.push_str("      </article>\n");
    doc.push_str("    </main>\n");

    // Aside / TOC
    if config.show_toc && !headings.is_empty() {
        doc.push_str("    <aside class=\"doc-aside\">\n");
        doc.push_str(&toc_html);
        doc.push_str("    </aside>\n");
    }

    doc.push_str("  </div>\n\n");

    // Back to top button
    if config.show_back_to_top {
        doc.push_str("  <button class=\"back-to-top\" onclick=\"scrollToTop()\" title=\"Back to top\">\n");
        doc.push_str("    <svg width=\"20\" height=\"20\" viewBox=\"0 0 24 24\" fill=\"none\" stroke=\"currentColor\" stroke-width=\"2\"><polyline points=\"18 15 12 9 6 15\"></polyline></svg>\n");
        doc.push_str("  </button>\n");
    }

    // Footer
    if config.show_footer {
        let footer_msg = config.footer_text.as_deref().unwrap_or("Documentation generated with Fusion Doc Render Engine");
        doc.push_str("  <footer class=\"doc-footer\">\n");
        doc.push_str(&format!("    <p>{}</p>\n", escape_html(footer_msg)));
        doc.push_str("  </footer>\n");
    }

    // JS
    doc.push_str(&format!("  <script>\n{}\n  </script>\n", js));
    doc.push_str("</body>\n</html>\n");

    doc
}

/// One-liner helper to render markdown to a standalone styled HTML doc page with default options.
pub fn render_doc_page(markdown: &str, title: &str) -> String {
    let config = DocConfig::new(title);
    generate_doc_page(markdown, &config)
}

/// Generates a multi-page documentation website from an array of `DocPage` items.
/// Returns a map of `filename -> HTML content`.
pub fn generate_doc_site(pages: &[DocPage], site_config: &DocConfig) -> HashMap<String, String> {
    let mut site = HashMap::new();

    // Group pages by category into NavSections
    let mut categorized: HashMap<String, Vec<&DocPage>> = HashMap::new();
    let mut uncategorized = Vec::new();

    for page in pages {
        if let Some(cat) = &page.category {
            categorized.entry(cat.clone()).or_default().push(page);
        } else {
            uncategorized.push(page);
        }
    }

    for page in pages {
        let mut page_config = site_config.clone();
        page_config.title = format!("{} - {}", page.title, site_config.title);

        let mut nav_sections = Vec::new();

        if !uncategorized.is_empty() {
            let items: Vec<NavItem> = uncategorized
                .iter()
                .map(|p| {
                    let filename = format!("{}.html", p.id);
                    NavItem::new(&p.title, &filename).with_active(p.id == page.id)
                })
                .collect();
            nav_sections.push(NavSection::new("Overview", items));
        }

        let mut cat_keys: Vec<String> = categorized.keys().cloned().collect();
        cat_keys.sort();

        for cat in cat_keys {
            let mut cat_pages = categorized[&cat].clone();
            cat_pages.sort_by_key(|p| p.order);

            let items: Vec<NavItem> = cat_pages
                .iter()
                .map(|p| {
                    let filename = format!("{}.html", p.id);
                    NavItem::new(&p.title, &filename).with_active(p.id == page.id)
                })
                .collect();
            nav_sections.push(NavSection::new(cat, items));
        }

        page_config.sidebar_nav = nav_sections;
        page_config.breadcrumbs = vec![
            ("Docs".to_string(), "index.html".to_string()),
            (page.title.clone(), format!("{}.html", page.id)),
        ];

        let html = generate_doc_page(&page.content, &page_config);
        let filename = format!("{}.html", page.id);
        site.insert(filename, html);
    }

    site
}

// ============================================================================
// Terminal Markdown & Documentation Renderer
// ============================================================================

/// ANSI escape codes for terminal styling and syntax highlighting.
pub mod ansi {
    pub const RESET: &str = "\x1b[0m";
    pub const BOLD: &str = "\x1b[1m";
    pub const DIM: &str = "\x1b[2m";
    pub const ITALIC: &str = "\x1b[3m";
    pub const UNDERLINE: &str = "\x1b[4m";
    pub const STRIKETHROUGH: &str = "\x1b[9m";

    // Standard foreground colors
    pub const BLACK: &str = "\x1b[30m";
    pub const RED: &str = "\x1b[31m";
    pub const GREEN: &str = "\x1b[32m";
    pub const YELLOW: &str = "\x1b[33m";
    pub const BLUE: &str = "\x1b[34m";
    pub const MAGENTA: &str = "\x1b[35m";
    pub const CYAN: &str = "\x1b[36m";
    pub const WHITE: &str = "\x1b[37m";

    // Bright foreground colors
    pub const BRIGHT_BLACK: &str = "\x1b[90m";
    pub const BRIGHT_RED: &str = "\x1b[91m";
    pub const BRIGHT_GREEN: &str = "\x1b[92m";
    pub const BRIGHT_YELLOW: &str = "\x1b[93m";
    pub const BRIGHT_BLUE: &str = "\x1b[94m";
    pub const BRIGHT_MAGENTA: &str = "\x1b[95m";
    pub const BRIGHT_CYAN: &str = "\x1b[96m";
    pub const BRIGHT_WHITE: &str = "\x1b[97m";

    // Background colors
    pub const BG_DARK: &str = "\x1b[48;5;236m";
    pub const BG_CODE: &str = "\x1b[48;5;235m";
    pub const BG_HIGHLIGHT: &str = "\x1b[48;5;226m\x1b[30m";

    // Syntax highlighting colors
    pub const HL_KW: &str = "\x1b[38;5;171m\x1b[1m";      // Magenta bold
    pub const HL_TYPE: &str = "\x1b[38;5;81m";             // Cyan
    pub const HL_BUILTIN: &str = "\x1b[38;5;75m";          // Blue
    pub const HL_FN: &str = "\x1b[38;5;117m";              // Sky blue
    pub const HL_STR: &str = "\x1b[38;5;114m";             // Green
    pub const HL_NUM: &str = "\x1b[38;5;215m";             // Orange / peach
    pub const HL_COMMENT: &str = "\x1b[38;5;245m\x1b[3m";  // Dim gray italic
    pub const HL_MACRO: &str = "\x1b[38;5;220m";           // Yellow
    pub const HL_INLINE_CODE: &str = "\x1b[38;5;222m\x1b[48;5;236m"; // Amber on dark bg
}

/// Strips all ANSI escape codes (CSI and OSC sequences) from a string.
pub fn strip_ansi(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut chars = input.chars().peekable();

    while let Some(c) = chars.next() {
        if c == '\x1b' {
            if let Some(&next_c) = chars.peek() {
                if next_c == '[' {
                    chars.next(); // consume '['
                    while let Some(&seq_c) = chars.peek() {
                        chars.next();
                        if (seq_c >= 'a' && seq_c <= 'z')
                            || (seq_c >= 'A' && seq_c <= 'Z')
                            || seq_c == '~'
                            || seq_c == '@'
                        {
                            break;
                        }
                    }
                    continue;
                } else if next_c == ']' {
                    chars.next(); // consume ']'
                    while let Some(seq_c) = chars.next() {
                        if seq_c == '\x07' {
                            break;
                        }
                        if seq_c == '\x1b' && chars.peek() == Some(&'\\') {
                            chars.next();
                            break;
                        }
                    }
                    continue;
                }
            }
        }
        out.push(c);
    }

    out
}

/// Returns the visible display column width of a single unicode character.
pub fn char_width(c: char) -> usize {
    if c == '\t' {
        return 4;
    }
    if c < ' ' || (c >= '\x7F' && c < '\u{A0}') {
        return 0;
    }
    if c == '\u{200B}' || c == '\u{200C}' || c == '\u{200D}' || c == '\u{FEFF}' {
        return 0;
    }
    match c {
        '\u{1100}'..='\u{115F}'
        | '\u{2329}'..='\u{232A}'
        | '\u{2E80}'..='\u{303E}'
        | '\u{3040}'..='\u{A4CF}'
        | '\u{AC00}'..='\u{D7A3}'
        | '\u{F900}'..='\u{FAFF}'
        | '\u{FE10}'..='\u{FE19}'
        | '\u{FE30}'..='\u{FE6F}'
        | '\u{FF00}'..='\u{FF60}'
        | '\u{FFE0}'..='\u{FFE6}'
        | '\u{1F300}'..='\u{1F64F}'
        | '\u{1F680}'..='\u{1F6FF}'
        | '\u{1F900}'..='\u{1F9FF}'
        | '\u{2600}'..='\u{26FF}'
        | '\u{2700}'..='\u{27BF}' => 2,
        _ => 1,
    }
}

/// Calculates the visible terminal column width of a string (excluding ANSI escape codes).
pub fn visible_width(s: &str) -> usize {
    let clean = strip_ansi(s);
    clean.chars().map(char_width).sum()
}

#[derive(Debug, Clone)]
enum LineToken {
    Ansi(String),
    Whitespace(String, usize),
    Word(String, usize),
}

fn tokenize_ansi_line(line: &str) -> Vec<LineToken> {
    let mut tokens = Vec::new();
    let chars: Vec<char> = line.chars().collect();
    let mut i = 0;

    while i < chars.len() {
        let c = chars[i];
        if c == '\x1b' {
            let start = i;
            i += 1;
            if i < chars.len() && chars[i] == '[' {
                i += 1;
                while i < chars.len() {
                    let sc = chars[i];
                    i += 1;
                    if (sc >= 'a' && sc <= 'z')
                        || (sc >= 'A' && sc <= 'Z')
                        || sc == '~'
                        || sc == '@'
                    {
                        break;
                    }
                }
            } else if i < chars.len() && chars[i] == ']' {
                i += 1;
                while i < chars.len() {
                    let sc = chars[i];
                    i += 1;
                    if sc == '\x07' {
                        break;
                    }
                    if sc == '\x1b' && i < chars.len() && chars[i] == '\\' {
                        i += 1;
                        break;
                    }
                }
            }
            let raw: String = chars[start..i].iter().collect();
            tokens.push(LineToken::Ansi(raw));
            continue;
        }

        if c.is_whitespace() {
            let start = i;
            let mut w = 0;
            while i < chars.len() && chars[i].is_whitespace() && chars[i] != '\x1b' {
                w += char_width(chars[i]);
                i += 1;
            }
            let raw: String = chars[start..i].iter().collect();
            tokens.push(LineToken::Whitespace(raw, w));
            continue;
        }

        let mut raw = String::new();
        let mut w = 0;
        while i < chars.len() && !chars[i].is_whitespace() {
            if chars[i] == '\x1b' {
                let start = i;
                i += 1;
                if i < chars.len() && chars[i] == '[' {
                    i += 1;
                    while i < chars.len() {
                        let sc = chars[i];
                        i += 1;
                        if (sc >= 'a' && sc <= 'z')
                            || (sc >= 'A' && sc <= 'Z')
                            || sc == '~'
                            || sc == '@'
                        {
                            break;
                        }
                    }
                } else if i < chars.len() && chars[i] == ']' {
                    i += 1;
                    while i < chars.len() {
                        let sc = chars[i];
                        i += 1;
                        if sc == '\x07' {
                            break;
                        }
                        if sc == '\x1b' && i < chars.len() && chars[i] == '\\' {
                            i += 1;
                            break;
                        }
                    }
                }
                let esc: String = chars[start..i].iter().collect();
                raw.push_str(&esc);
                continue;
            }

            raw.push(chars[i]);
            w += char_width(chars[i]);
            i += 1;
        }
        tokens.push(LineToken::Word(raw, w));
    }

    tokens
}

fn update_active_styles(styles: &mut Vec<String>, esc: &str) {
    if esc == "\x1b[0m" || esc == "\x1b[m" {
        styles.clear();
    } else if esc.starts_with("\x1b[") && esc.ends_with('m') {
        if esc == "\x1b[22m" {
            styles.retain(|s| s != "\x1b[1m" && s != "\x1b[2m");
        } else if esc == "\x1b[23m" {
            styles.retain(|s| s != "\x1b[3m");
        } else if esc == "\x1b[24m" {
            styles.retain(|s| s != "\x1b[4m");
        } else if esc == "\x1b[29m" {
            styles.retain(|s| s != "\x1b[9m");
        } else {
            styles.push(esc.to_string());
        }
    }
}

fn split_long_word(word: &str, max_width: usize, active_styles: &mut Vec<String>) -> Vec<String> {
    let mut chunks = Vec::new();
    let mut cur_chunk = String::new();
    let mut cur_w = 0;
    let chars: Vec<char> = word.chars().collect();
    let mut i = 0;

    while i < chars.len() {
        let c = chars[i];
        if c == '\x1b' {
            let start = i;
            i += 1;
            if i < chars.len() && chars[i] == '[' {
                i += 1;
                while i < chars.len() {
                    let sc = chars[i];
                    i += 1;
                    if (sc >= 'a' && sc <= 'z')
                        || (sc >= 'A' && sc <= 'Z')
                        || sc == '~'
                        || sc == '@'
                    {
                        break;
                    }
                }
            } else if i < chars.len() && chars[i] == ']' {
                i += 1;
                while i < chars.len() {
                    let sc = chars[i];
                    i += 1;
                    if sc == '\x07' {
                        break;
                    }
                    if sc == '\x1b' && i < chars.len() && chars[i] == '\\' {
                        i += 1;
                        break;
                    }
                }
            }
            let esc: String = chars[start..i].iter().collect();
            update_active_styles(active_styles, &esc);
            cur_chunk.push_str(&esc);
            continue;
        }

        let cw = char_width(c);
        if cur_w + cw > max_width && cur_w > 0 {
            chunks.push(cur_chunk);
            cur_chunk = String::new();
            cur_w = 0;
        }

        cur_chunk.push(c);
        cur_w += cw;
        i += 1;
    }

    if !cur_chunk.is_empty() {
        chunks.push(cur_chunk);
    }

    chunks
}

fn wrap_single_line_ansi(line: &str, max_width: usize) -> Vec<String> {
    let tokens = tokenize_ansi_line(line);
    let mut lines = Vec::new();
    let mut current_line = String::new();
    let mut current_width = 0;
    let mut active_styles = Vec::new();
    let mut pending_space = String::new();
    let mut pending_space_width = 0;

    for token in tokens {
        match token {
            LineToken::Ansi(esc) => {
                update_active_styles(&mut active_styles, &esc);
                current_line.push_str(&esc);
            }
            LineToken::Whitespace(ws, w) => {
                if current_width > 0 {
                    pending_space = ws;
                    pending_space_width = w;
                }
            }
            LineToken::Word(raw, word_w) => {
                if word_w == 0 {
                    current_line.push_str(&raw);
                    continue;
                }

                if current_width + pending_space_width + word_w <= max_width {
                    if current_width > 0 && pending_space_width > 0 {
                        current_line.push_str(&pending_space);
                        current_width += pending_space_width;
                    }
                    current_line.push_str(&raw);
                    current_width += word_w;
                    pending_space.clear();
                    pending_space_width = 0;
                } else if word_w <= max_width {
                    if current_width > 0 {
                        if !active_styles.is_empty() && !current_line.ends_with(ansi::RESET) {
                            current_line.push_str(ansi::RESET);
                        }
                        lines.push(current_line);
                        current_line = active_styles.concat();
                        current_width = 0;
                    }
                    current_line.push_str(&raw);
                    current_width = word_w;
                    pending_space.clear();
                    pending_space_width = 0;
                } else {
                    if current_width > 0 {
                        if !active_styles.is_empty() && !current_line.ends_with(ansi::RESET) {
                            current_line.push_str(ansi::RESET);
                        }
                        lines.push(current_line);
                        current_line = active_styles.concat();
                        current_width = 0;
                    }
                    pending_space.clear();
                    pending_space_width = 0;

                    let word_chunks = split_long_word(&raw, max_width, &mut active_styles);
                    for (idx, chunk) in word_chunks.into_iter().enumerate() {
                        let chunk_w = visible_width(&chunk);
                        if idx > 0 {
                            if !active_styles.is_empty() && !current_line.ends_with(ansi::RESET) {
                                current_line.push_str(ansi::RESET);
                            }
                            lines.push(current_line);
                            current_line = active_styles.concat();
                            current_width = 0;
                        }
                        current_line.push_str(&chunk);
                        current_width += chunk_w;
                    }
                }
            }
        }
    }

    if !current_line.is_empty() {
        if !active_styles.is_empty() && !current_line.ends_with(ansi::RESET) {
            current_line.push_str(ansi::RESET);
        }
        lines.push(current_line);
    }

    if lines.is_empty() {
        lines.push(String::new());
    }

    lines
}

/// Wraps text containing ANSI escape sequences to fit within `max_width` columns
/// without breaking ANSI escape codes or losing active color styling across wrapped lines.
pub fn wrap_ansi(text: &str, max_width: usize) -> Vec<String> {
    let target_width = max_width.max(1);
    let mut out = Vec::new();

    for line in text.split('\n') {
        if line.is_empty() {
            out.push(String::new());
        } else if visible_width(line) <= target_width {
            out.push(line.to_string());
        } else {
            let wrapped_sublines = wrap_single_line_ansi(line, target_width);
            out.extend(wrapped_sublines);
        }
    }

    out
}

/// Wraps text containing ANSI escape sequences and joins with newlines.
pub fn wrap_ansi_lines(text: &str, max_width: usize) -> String {
    wrap_ansi(text, max_width).join("\n")
}

/// Lightweight pure-Rust syntax highlighter producing ANSI colored terminal output.
pub fn highlight_code_terminal(code: &str, lang: &str) -> String {
    highlight_code_terminal_opt(code, lang, true)
}

/// Highlights code with ANSI colors, or returns plain code if `colored` is false.
pub fn highlight_code_terminal_opt(code: &str, lang: &str, colored: bool) -> String {
    if !colored {
        return code.to_string();
    }
    let lang_lower = lang.trim().to_lowercase();
    let mut result = String::with_capacity(code.len() * 2);

    for (line_idx, line) in code.lines().enumerate() {
        if line_idx > 0 {
            result.push('\n');
        }
        highlight_line_terminal(line, &lang_lower, &mut result);
    }

    result
}

fn highlight_line_terminal(line: &str, lang: &str, out: &mut String) {
    if line.is_empty() {
        return;
    }

    let trimmed = line.trim_start();
    let indent_len = line.len() - trimmed.len();
    out.push_str(&line[..indent_len]);

    // Full-line comments
    let is_comment = match lang {
        "rs" | "rust" | "js" | "javascript" | "ts" | "typescript" | "go" | "c" | "cpp"
        | "csharp" | "cs" | "java" | "kotlin" | "swift" | "php" => trimmed.starts_with("//"),
        "py" | "python" | "sh" | "bash" | "zsh" | "yaml" | "yml" | "toml" | "ruby" | "rb"
        | "dockerfile" | "makefile" => trimmed.starts_with('#'),
        "sql" | "lua" => trimmed.starts_with("--"),
        "html" | "xml" => trimmed.starts_with("<!--"),
        "css" => trimmed.starts_with("/*"),
        _ => false,
    };

    if is_comment {
        out.push_str(ansi::HL_COMMENT);
        out.push_str(trimmed);
        out.push_str(ansi::RESET);
        return;
    }

    let chars: Vec<char> = trimmed.chars().collect();
    let mut i = 0;

    while i < chars.len() {
        let c = chars[i];

        // Inline comments: // or # or --
        if (c == '/' && i + 1 < chars.len() && chars[i + 1] == '/')
            && matches!(lang, "rs" | "rust" | "js" | "ts" | "go" | "c" | "cpp" | "cs" | "java")
        {
            out.push_str(ansi::HL_COMMENT);
            let rem: String = chars[i..].iter().collect();
            out.push_str(&rem);
            out.push_str(ansi::RESET);
            break;
        }

        if c == '#' && matches!(lang, "py" | "python" | "sh" | "bash" | "yaml" | "toml" | "rb") {
            out.push_str(ansi::HL_COMMENT);
            let rem: String = chars[i..].iter().collect();
            out.push_str(&rem);
            out.push_str(ansi::RESET);
            break;
        }

        if c == '-' && i + 1 < chars.len() && chars[i + 1] == '-' && matches!(lang, "sql" | "lua") {
            out.push_str(ansi::HL_COMMENT);
            let rem: String = chars[i..].iter().collect();
            out.push_str(&rem);
            out.push_str(ansi::RESET);
            break;
        }

        // Rust attributes: #[derive(...)]
        if c == '#' && i + 1 < chars.len() && chars[i + 1] == '[' && matches!(lang, "rs" | "rust") {
            out.push_str(ansi::HL_MACRO);
            out.push('#');
            out.push('[');
            i += 2;
            while i < chars.len() && chars[i] != ']' {
                out.push(chars[i]);
                i += 1;
            }
            if i < chars.len() {
                out.push(']');
                i += 1;
            }
            out.push_str(ansi::RESET);
            continue;
        }

        // Decorators: @decorator
        if c == '@' && matches!(lang, "py" | "python" | "ts" | "typescript" | "java") {
            out.push_str(ansi::HL_MACRO);
            out.push('@');
            i += 1;
            while i < chars.len() && (chars[i].is_alphanumeric() || chars[i] == '_') {
                out.push(chars[i]);
                i += 1;
            }
            out.push_str(ansi::RESET);
            continue;
        }

        // Strings
        if c == '"' || c == '\'' || (c == '`' && matches!(lang, "js" | "ts" | "go")) {
            let quote = c;
            out.push_str(ansi::HL_STR);
            out.push(quote);
            i += 1;
            let mut escaped = false;
            while i < chars.len() {
                let cur = chars[i];
                if escaped {
                    out.push(cur);
                    escaped = false;
                } else if cur == '\\' {
                    escaped = true;
                    out.push('\\');
                } else if cur == quote {
                    out.push(quote);
                    i += 1;
                    break;
                } else {
                    out.push(cur);
                }
                i += 1;
            }
            out.push_str(ansi::RESET);
            continue;
        }

        // Numbers
        if c.is_ascii_digit() && (i == 0 || (!chars[i - 1].is_alphanumeric() && chars[i - 1] != '_')) {
            out.push_str(ansi::HL_NUM);
            while i < chars.len()
                && (chars[i].is_alphanumeric() || chars[i] == '.' || chars[i] == '_')
            {
                out.push(chars[i]);
                i += 1;
            }
            out.push_str(ansi::RESET);
            continue;
        }

        // Identifiers & Keywords & Types & Functions
        if c.is_alphabetic() || c == '_' {
            let start = i;
            while i < chars.len() && (chars[i].is_alphanumeric() || chars[i] == '_') {
                i += 1;
            }
            let word: String = chars[start..i].iter().collect();

            let is_rust_macro = matches!(lang, "rs" | "rust") && i < chars.len() && chars[i] == '!';
            if is_rust_macro {
                out.push_str(ansi::HL_MACRO);
                out.push_str(&word);
                out.push('!');
                out.push_str(ansi::RESET);
                i += 1;
                continue;
            }

            let mut next_idx = i;
            while next_idx < chars.len() && chars[next_idx].is_whitespace() {
                next_idx += 1;
            }
            let is_fn_call = next_idx < chars.len() && chars[next_idx] == '(';

            if is_keyword(&word, lang) {
                out.push_str(ansi::HL_KW);
                out.push_str(&word);
                out.push_str(ansi::RESET);
            } else if is_type_name(&word) {
                out.push_str(ansi::HL_TYPE);
                out.push_str(&word);
                out.push_str(ansi::RESET);
            } else if is_builtin(&word) {
                out.push_str(ansi::HL_BUILTIN);
                out.push_str(&word);
                out.push_str(ansi::RESET);
            } else if is_fn_call {
                out.push_str(ansi::HL_FN);
                out.push_str(&word);
                out.push_str(ansi::RESET);
            } else {
                out.push_str(&word);
            }
            continue;
        }

        out.push(c);
        i += 1;
    }
}

/// Converts inline markdown formatting to ANSI-styled terminal text.
pub fn render_inline_terminal(text: &str, colored: bool) -> String {
    if !colored {
        return strip_inline_markdown(text);
    }

    let mut out = String::with_capacity(text.len() * 2);
    let chars: Vec<char> = text.chars().collect();
    let mut i = 0;

    while i < chars.len() {
        let c = chars[i];

        // Escaped characters: \* \_ \` etc.
        if c == '\\' && i + 1 < chars.len() {
            out.push(chars[i + 1]);
            i += 2;
            continue;
        }

        // Inline code: `code`
        if c == '`' {
            let start = i + 1;
            if let Some(end) = chars[start..].iter().position(|&x| x == '`') {
                let code_content: String = chars[start..start + end].iter().collect();
                out.push_str(ansi::HL_INLINE_CODE);
                out.push_str(&code_content);
                out.push_str(ansi::RESET);
                i = start + end + 1;
                continue;
            }
        }

        // Bold + Italic: ***text*** or ___text___
        if (c == '*' || c == '_') && i + 2 < chars.len() && chars[i + 1] == c && chars[i + 2] == c {
            let delim = c;
            let start = i + 3;
            let mut end_idx = None;
            let mut j = start;
            while j + 2 < chars.len() {
                if chars[j] == delim && chars[j + 1] == delim && chars[j + 2] == delim {
                    end_idx = Some(j);
                    break;
                }
                j += 1;
            }
            if let Some(end) = end_idx {
                let inner: String = chars[start..end].iter().collect();
                out.push_str(ansi::BOLD);
                out.push_str(ansi::ITALIC);
                out.push_str(&render_inline_terminal(&inner, true));
                out.push_str(ansi::RESET);
                i = end + 3;
                continue;
            }
        }

        // Bold: **text** or __text__
        if (c == '*' || c == '_') && i + 1 < chars.len() && chars[i + 1] == c {
            let delim = c;
            let start = i + 2;
            let mut end_idx = None;
            let mut j = start;
            while j + 1 < chars.len() {
                if chars[j] == delim && chars[j + 1] == delim {
                    end_idx = Some(j);
                    break;
                }
                j += 1;
            }
            if let Some(end) = end_idx {
                let inner: String = chars[start..end].iter().collect();
                out.push_str(ansi::BOLD);
                out.push_str(&render_inline_terminal(&inner, true));
                out.push_str(ansi::RESET);
                i = end + 2;
                continue;
            }
        }

        // Italic: *text* or _text_
        if c == '*' || c == '_' {
            let delim = c;
            let start = i + 1;
            let is_in_word = delim == '_' && i > 0 && chars[i - 1].is_alphanumeric();
            if !is_in_word {
                let mut end_idx = None;
                let mut j = start;
                while j < chars.len() {
                    if chars[j] == delim {
                        if delim == '_' && j + 1 < chars.len() && chars[j + 1].is_alphanumeric() {
                            j += 1;
                            continue;
                        }
                        end_idx = Some(j);
                        break;
                    }
                    j += 1;
                }
                if let Some(end) = end_idx {
                    let inner: String = chars[start..end].iter().collect();
                    out.push_str(ansi::ITALIC);
                    out.push_str(&render_inline_terminal(&inner, true));
                    out.push_str(ansi::RESET);
                    i = end + 1;
                    continue;
                }
            }
        }

        // Strikethrough: ~~text~~
        if c == '~' && i + 1 < chars.len() && chars[i + 1] == '~' {
            let start = i + 2;
            let mut end_idx = None;
            let mut j = start;
            while j + 1 < chars.len() {
                if chars[j] == '~' && chars[j + 1] == '~' {
                    end_idx = Some(j);
                    break;
                }
                j += 1;
            }
            if let Some(end) = end_idx {
                let inner: String = chars[start..end].iter().collect();
                out.push_str(ansi::STRIKETHROUGH);
                out.push_str(&render_inline_terminal(&inner, true));
                out.push_str(ansi::RESET);
                i = end + 2;
                continue;
            }
        }

        // Highlight: ==text==
        if c == '=' && i + 1 < chars.len() && chars[i + 1] == '=' {
            let start = i + 2;
            let mut end_idx = None;
            let mut j = start;
            while j + 1 < chars.len() {
                if chars[j] == '=' && chars[j + 1] == '=' {
                    end_idx = Some(j);
                    break;
                }
                j += 1;
            }
            if let Some(end) = end_idx {
                let inner: String = chars[start..end].iter().collect();
                out.push_str(ansi::BG_HIGHLIGHT);
                out.push_str(&inner);
                out.push_str(ansi::RESET);
                i = end + 2;
                continue;
            }
        }

        // Images: ![alt](url)
        if c == '!' && i + 1 < chars.len() && chars[i + 1] == '[' {
            let start = i + 2;
            if let Some(close_bracket) = chars[start..].iter().position(|&x| x == ']') {
                let alt: String = chars[start..start + close_bracket].iter().collect();
                let url_start = start + close_bracket + 1;
                if url_start < chars.len() && chars[url_start] == '(' {
                    if let Some(close_paren) = chars[url_start + 1..].iter().position(|&x| x == ')') {
                        let url: String = chars[url_start + 1..url_start + 1 + close_paren].iter().collect();
                        out.push_str("\x1b[38;5;178m[Image: \x1b[1m");
                        out.push_str(&alt);
                        out.push_str("\x1b[22m]\x1b[0m \x1b[90m(");
                        out.push_str(&url);
                        out.push_str(")\x1b[0m");
                        i = url_start + 1 + close_paren + 1;
                        continue;
                    }
                }
            }
        }

        // Links: [text](url)
        if c == '[' {
            let start = i + 1;
            if let Some(close_bracket) = chars[start..].iter().position(|&x| x == ']') {
                let link_text: String = chars[start..start + close_bracket].iter().collect();
                let url_start = start + close_bracket + 1;
                if url_start < chars.len() && chars[url_start] == '(' {
                    if let Some(close_paren) = chars[url_start + 1..].iter().position(|&x| x == ')') {
                        let url_raw: String = chars[url_start + 1..url_start + 1 + close_paren].iter().collect();
                        let (url, _title) = parse_link_url_title(&url_raw);
                        out.push_str("\x1b[38;5;75m\x1b[4m");
                        out.push_str(&link_text);
                        out.push_str("\x1b[24m\x1b[0m \x1b[90m(\x1b[4m\x1b[38;5;111m");
                        out.push_str(&url);
                        out.push_str("\x1b[24m\x1b[90m)\x1b[0m");
                        i = url_start + 1 + close_paren + 1;
                        continue;
                    }
                }
            }
        }

        out.push(c);
        i += 1;
    }

    out
}

/// Alignment of columns within a Markdown table.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TableAlign {
    #[default]
    Left,
    Center,
    Right,
}

/// Renders a markdown table with styled Unicode box borders and aligned cells for the terminal.
pub fn render_table_terminal(lines: &[&str], max_width: usize, colored: bool) -> Option<String> {
    if lines.len() < 2 {
        return None;
    }

    let header_cells = parse_table_row(lines[0]);
    if header_cells.is_empty() {
        return None;
    }

    let num_cols = header_cells.len();
    let alignments: Vec<TableAlign> = parse_table_alignments(lines[1], num_cols);

    let mut body_rows: Vec<Vec<String>> = Vec::new();
    for line in &lines[2..] {
        let mut row = parse_table_row(line);
        while row.len() < num_cols {
            row.push(String::new());
        }
        row.truncate(num_cols);
        body_rows.push(row);
    }

    let formatted_headers: Vec<String> = header_cells
        .iter()
        .map(|h| render_inline_terminal(h, colored))
        .collect();

    let mut formatted_body_rows: Vec<Vec<String>> = Vec::new();
    for row in &body_rows {
        let formatted_row: Vec<String> = row
            .iter()
            .map(|cell| render_inline_terminal(cell, colored))
            .collect();
        formatted_body_rows.push(formatted_row);
    }

    let mut col_widths: Vec<usize> = vec![3; num_cols];
    for (c, h) in formatted_headers.iter().enumerate() {
        col_widths[c] = col_widths[c].max(visible_width(h));
    }
    for row in &formatted_body_rows {
        for (c, cell) in row.iter().enumerate() {
            col_widths[c] = col_widths[c].max(visible_width(cell));
        }
    }

    let total_needed: usize = col_widths.iter().map(|w| w + 2).sum::<usize>() + num_cols + 1;
    if total_needed > max_width && max_width > num_cols * 4 + 1 {
        let available_cell_space = max_width.saturating_sub(num_cols * 3 + 1);
        let sum_raw: usize = col_widths.iter().sum();
        if sum_raw > 0 {
            for w in col_widths.iter_mut() {
                *w = ((*w * available_cell_space) / sum_raw).max(3);
            }
        }
    }

    let mut out = String::new();
    let cyan_border = if colored { "\x1b[36m" } else { "" };
    let reset = if colored { ansi::RESET } else { "" };
    let bold_header = if colored { "\x1b[1;96m" } else { "" };

    // Top border: ┌──────┬──────┐
    out.push_str(cyan_border);
    out.push('┌');
    for (i, w) in col_widths.iter().enumerate() {
        if i > 0 {
            out.push('┬');
        }
        out.push_str(&"─".repeat(*w + 2));
    }
    out.push_str("┐\n");
    out.push_str(reset);

    // Header Row
    out.push_str(cyan_border);
    out.push('│');
    out.push_str(reset);
    for (i, (h, w)) in formatted_headers.iter().zip(&col_widths).enumerate() {
        let align = alignments.get(i).copied().unwrap_or(TableAlign::Left);
        let cell_str = format!("{}{}{}", bold_header, h, reset);
        let padded = pad_cell(&cell_str, *w, align);
        out.push(' ');
        out.push_str(&padded);
        out.push(' ');
        out.push_str(cyan_border);
        out.push('│');
        out.push_str(reset);
    }
    out.push('\n');

    // Header Separator: ├──────┼──────┤
    out.push_str(cyan_border);
    out.push('├');
    for (i, w) in col_widths.iter().enumerate() {
        if i > 0 {
            out.push('┼');
        }
        out.push_str(&"─".repeat(*w + 2));
    }
    out.push_str("┤\n");
    out.push_str(reset);

    // Body Rows
    for row in &formatted_body_rows {
        let mut wrapped_cells: Vec<Vec<String>> = Vec::new();
        let mut max_sublines = 1;

        for (cell, w) in row.iter().zip(&col_widths) {
            let sublines = wrap_ansi(cell, *w);
            max_sublines = max_sublines.max(sublines.len());
            wrapped_cells.push(sublines);
        }

        for sub_idx in 0..max_sublines {
            out.push_str(cyan_border);
            out.push('│');
            out.push_str(reset);
            for (i, w) in col_widths.iter().enumerate() {
                let align = alignments.get(i).copied().unwrap_or(TableAlign::Left);
                let cell_line = wrapped_cells
                    .get(i)
                    .and_then(|lines| lines.get(sub_idx))
                    .map(|s| s.as_str())
                    .unwrap_or("");
                let padded = pad_cell(cell_line, *w, align);
                out.push(' ');
                out.push_str(&padded);
                out.push(' ');
                out.push_str(cyan_border);
                out.push('│');
                out.push_str(reset);
            }
            out.push('\n');
        }
    }

    // Bottom border: └──────┴──────┘
    out.push_str(cyan_border);
    out.push('└');
    for (i, w) in col_widths.iter().enumerate() {
        if i > 0 {
            out.push('┴');
        }
        out.push_str(&"─".repeat(*w + 2));
    }
    out.push_str("┘\n");
    out.push_str(reset);

    Some(out)
}

fn parse_table_alignments(delim_line: &str, count: usize) -> Vec<TableAlign> {
    let parts = parse_table_row(delim_line);
    let mut aligns = Vec::new();
    for part in parts {
        let trimmed = part.trim();
        let starts = trimmed.starts_with(':');
        let ends = trimmed.ends_with(':');
        let align = match (starts, ends) {
            (true, true) => TableAlign::Center,
            (false, true) => TableAlign::Right,
            _ => TableAlign::Left,
        };
        aligns.push(align);
    }
    while aligns.len() < count {
        aligns.push(TableAlign::Left);
    }
    aligns.truncate(count);
    aligns
}

fn pad_cell(content: &str, width: usize, align: TableAlign) -> String {
    let vis_w = visible_width(content);
    if vis_w >= width {
        return content.to_string();
    }
    let total_pad = width - vis_w;
    match align {
        TableAlign::Left => {
            format!("{}{}", content, " ".repeat(total_pad))
        }
        TableAlign::Right => {
            format!("{}{}", " ".repeat(total_pad), content)
        }
        TableAlign::Center => {
            let left_pad = total_pad / 2;
            let right_pad = total_pad - left_pad;
            format!("{}{}{}", " ".repeat(left_pad), content, " ".repeat(right_pad))
        }
    }
}

/// Terminal Markdown heading rendering style.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum HeadingStyle {
    #[default]
    Banner,
    Underline,
    Prefix,
}

/// Border styling for terminal code blocks.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CodeBorderStyle {
    #[default]
    Boxed,
    LeftRail,
    Minimal,
    Plain,
}

/// Options for terminal documentation rendering.
#[derive(Debug, Clone)]
pub struct TerminalDocOptions {
    pub width: usize,
    pub colored: bool,
    pub syntax_highlighting: bool,
    pub code_border_style: CodeBorderStyle,
    pub heading_style: HeadingStyle,
    pub line_numbers_in_code: bool,
    pub osc8_links: bool,
    pub tab_width: usize,
}

impl Default for TerminalDocOptions {
    fn default() -> Self {
        Self {
            width: 80,
            colored: true,
            syntax_highlighting: true,
            code_border_style: CodeBorderStyle::Boxed,
            heading_style: HeadingStyle::Banner,
            line_numbers_in_code: true,
            osc8_links: false,
            tab_width: 4,
        }
    }
}

fn render_code_block_terminal(
    code: &str,
    lang: &str,
    options: &TerminalDocOptions,
) -> String {
    let highlighted = if options.syntax_highlighting {
        highlight_code_terminal_opt(code, lang, options.colored)
    } else {
        code.to_string()
    };

    let border_color = if options.colored { "\x1b[38;5;242m" } else { "" };
    let lang_color = if options.colored { "\x1b[1;36m" } else { "" };
    let line_no_color = if options.colored { "\x1b[38;5;240m" } else { "" };
    let reset = if options.colored { ansi::RESET } else { "" };

    let lines: Vec<&str> = highlighted.lines().collect();
    let total_lines = lines.len();
    let line_no_width = if options.line_numbers_in_code && total_lines > 0 {
        format!("{}", total_lines).len().max(2)
    } else {
        0
    };

    let max_line_len = lines.iter().map(|l| visible_width(l)).max().unwrap_or(20);
    let lang_display = if lang.is_empty() { "text" } else { lang };
    let min_box_width = visible_width(lang_display) + 6 + line_no_width;
    let box_width = (max_line_len + line_no_width + 6).clamp(min_box_width, options.width);

    match options.code_border_style {
        CodeBorderStyle::Boxed => {
            let mut out = String::new();
            let lang_badge = format!("─ {}{}{} ", lang_color, lang_display, reset);
            let badge_vis_w = visible_width(&lang_badge);
            let bar_len = box_width.saturating_sub(badge_vis_w + 2);
            out.push_str(border_color);
            out.push('╭');
            out.push_str(&lang_badge);
            out.push_str(border_color);
            out.push_str(&"─".repeat(bar_len));
            out.push_str("╮\n");
            out.push_str(reset);

            for (idx, line) in lines.iter().enumerate() {
                out.push_str(border_color);
                out.push('│');
                out.push_str(reset);
                out.push(' ');
                if options.line_numbers_in_code {
                    let num_str = format!("{:>width$}", idx + 1, width = line_no_width);
                    out.push_str(line_no_color);
                    out.push_str(&num_str);
                    out.push_str(reset);
                    out.push_str(border_color);
                    out.push_str(" │ ");
                    out.push_str(reset);
                }
                out.push_str(line);
                out.push('\n');
            }

            out.push_str(border_color);
            out.push('╰');
            out.push_str(&"─".repeat(box_width.saturating_sub(2)));
            out.push_str("╯\n");
            out.push_str(reset);
            out
        }
        CodeBorderStyle::LeftRail => {
            let mut out = String::new();
            out.push_str(&format!("{}{}─── {} ───{}{}\n", border_color, lang_color, lang_display, border_color, reset));
            for (idx, line) in lines.iter().enumerate() {
                out.push_str(border_color);
                out.push('│');
                out.push_str(reset);
                out.push(' ');
                if options.line_numbers_in_code {
                    let num_str = format!("{:>width$}", idx + 1, width = line_no_width);
                    out.push_str(&format!("{}{}{} │ ", line_no_color, num_str, reset));
                }
                out.push_str(line);
                out.push('\n');
            }
            out
        }
        CodeBorderStyle::Minimal => {
            let mut out = String::new();
            out.push_str(&format!("{}[{}]-----------------------------------{}\n", border_color, lang_display, reset));
            for line in lines {
                out.push_str(line);
                out.push('\n');
            }
            out.push_str(&format!("{}--------------------------------------{}\n", border_color, reset));
            out
        }
        CodeBorderStyle::Plain => {
            let mut out = String::new();
            for line in lines {
                out.push_str(line);
                out.push('\n');
            }
            out
        }
    }
}

fn render_callout_terminal(
    callout_type: &str,
    lines: &[&str],
    options: &TerminalDocOptions,
) -> String {
    let (icon, title, color_code) = match callout_type.to_uppercase().as_str() {
        "NOTE" => ("ℹ", "NOTE", "\x1b[38;5;39m"),
        "TIP" => ("💡", "TIP", "\x1b[38;5;42m"),
        "WARNING" => ("⚠", "WARNING", "\x1b[38;5;214m"),
        "IMPORTANT" => ("⚡", "IMPORTANT", "\x1b[38;5;171m"),
        "CAUTION" => ("🛑", "CAUTION", "\x1b[38;5;196m"),
        _ => ("💬", callout_type, "\x1b[36m"),
    };

    let col = if options.colored { color_code } else { "" };
    let bold = if options.colored { ansi::BOLD } else { "" };
    let reset = if options.colored { ansi::RESET } else { "" };

    let mut out = String::new();
    let box_width = options.width.min(76);
    let title_badge = format!("─ {} {}{} ", icon, bold, title);
    let vis_badge = visible_width(&title_badge);
    let bar_len = box_width.saturating_sub(vis_badge + 2);

    out.push_str(col);
    out.push('╭');
    out.push_str(&title_badge);
    out.push_str(col);
    out.push_str(&"─".repeat(bar_len));
    out.push_str("╮\n");
    out.push_str(reset);

    let content_width = box_width.saturating_sub(4);
    for line in lines {
        let rendered = render_inline_terminal(line, options.colored);
        let wrapped = wrap_ansi(&rendered, content_width);
        for wline in wrapped {
            out.push_str(col);
            out.push('│');
            out.push_str(reset);
            out.push(' ');
            out.push_str(&wline);
            out.push('\n');
        }
    }

    out.push_str(col);
    out.push('╰');
    out.push_str(&"─".repeat(box_width.saturating_sub(2)));
    out.push_str("╯\n");
    out.push_str(reset);

    out
}

fn render_list_terminal(
    lines: &[&str],
    options: &TerminalDocOptions,
) -> String {
    let mut out = String::new();

    for line in lines {
        let trimmed_start = line.trim_start();
        let leading_spaces = line.len() - trimmed_start.len();
        let indent_level = leading_spaces / 2;
        let indent_str = "  ".repeat(indent_level);

        // Task list
        if trimmed_start.starts_with("- [ ] ") || trimmed_start.starts_with("* [ ] ") {
            let item_text = &trimmed_start[6..];
            let rendered = render_inline_terminal(item_text, options.colored);
            let box_glyph = if options.colored { "\x1b[90m☐\x1b[0m" } else { "[ ]" };
            let prefix = format!("{}  {} ", indent_str, box_glyph);
            let prefix_w = visible_width(&prefix);
            let hanging_indent = " ".repeat(prefix_w);
            let avail_w = options.width.saturating_sub(prefix_w).max(10);
            let wrapped = wrap_ansi(&rendered, avail_w);
            for (idx, wline) in wrapped.iter().enumerate() {
                if idx == 0 {
                    out.push_str(&format!("{}{}\n", prefix, wline));
                } else {
                    out.push_str(&format!("{}{}\n", hanging_indent, wline));
                }
            }
            continue;
        }

        if trimmed_start.starts_with("- [x] ")
            || trimmed_start.starts_with("- [X] ")
            || trimmed_start.starts_with("* [x] ")
            || trimmed_start.starts_with("* [X] ")
        {
            let item_text = &trimmed_start[6..];
            let rendered = render_inline_terminal(item_text, options.colored);
            let box_glyph = if options.colored { "\x1b[32m☑\x1b[0m" } else { "[x]" };
            let prefix = format!("{}  {} ", indent_str, box_glyph);
            let prefix_w = visible_width(&prefix);
            let hanging_indent = " ".repeat(prefix_w);
            let avail_w = options.width.saturating_sub(prefix_w).max(10);
            let wrapped = wrap_ansi(&rendered, avail_w);
            for (idx, wline) in wrapped.iter().enumerate() {
                if idx == 0 {
                    out.push_str(&format!("{}{}\n", prefix, wline));
                } else {
                    out.push_str(&format!("{}{}\n", hanging_indent, wline));
                }
            }
            continue;
        }

        // Unordered list (*, -, +)
        if trimmed_start.starts_with("* ")
            || trimmed_start.starts_with("- ")
            || trimmed_start.starts_with("+ ")
        {
            let item_text = &trimmed_start[2..];
            let rendered = render_inline_terminal(item_text, options.colored);
            let bullet_glyph = match indent_level % 4 {
                0 => if options.colored { "\x1b[36m•\x1b[0m" } else { "•" },
                1 => if options.colored { "\x1b[34m○\x1b[0m" } else { "○" },
                2 => if options.colored { "\x1b[35m▪\x1b[0m" } else { "▪" },
                _ => if options.colored { "\x1b[33m▸\x1b[0m" } else { "▸" },
            };
            let prefix = format!("{}  {} ", indent_str, bullet_glyph);
            let prefix_w = visible_width(&prefix);
            let hanging_indent = " ".repeat(prefix_w);
            let avail_w = options.width.saturating_sub(prefix_w).max(10);
            let wrapped = wrap_ansi(&rendered, avail_w);
            for (idx, wline) in wrapped.iter().enumerate() {
                if idx == 0 {
                    out.push_str(&format!("{}{}\n", prefix, wline));
                } else {
                    out.push_str(&format!("{}{}\n", hanging_indent, wline));
                }
            }
            continue;
        }

        // Ordered list (1., 2.)
        let chars: Vec<char> = trimmed_start.chars().collect();
        let mut num_end = 0;
        while num_end < chars.len() && chars[num_end].is_ascii_digit() {
            num_end += 1;
        }
        if num_end > 0 && num_end < chars.len() && chars[num_end] == '.' && (num_end + 1 == chars.len() || chars[num_end + 1] == ' ') {
            let num_str: String = chars[..num_end].iter().collect();
            let start_item = if num_end + 1 < chars.len() { num_end + 2 } else { num_end + 1 };
            let item_text: String = chars[start_item.min(chars.len())..].iter().collect();
            let rendered = render_inline_terminal(&item_text, options.colored);
            let num_glyph = if options.colored {
                format!("\x1b[1;36m{}.\x1b[0m", num_str)
            } else {
                format!("{}.", num_str)
            };
            let prefix = format!("{}  {} ", indent_str, num_glyph);
            let prefix_w = visible_width(&prefix);
            let hanging_indent = " ".repeat(prefix_w);
            let avail_w = options.width.saturating_sub(prefix_w).max(10);
            let wrapped = wrap_ansi(&rendered, avail_w);
            for (idx, wline) in wrapped.iter().enumerate() {
                if idx == 0 {
                    out.push_str(&format!("{}{}\n", prefix, wline));
                } else {
                    out.push_str(&format!("{}{}\n", hanging_indent, wline));
                }
            }
            continue;
        }

        // Continuation line inside list
        let rendered = render_inline_terminal(trimmed_start, options.colored);
        let prefix = format!("{}    ", indent_str);
        let prefix_w = visible_width(&prefix);
        let avail_w = options.width.saturating_sub(prefix_w).max(10);
        let wrapped = wrap_ansi(&rendered, avail_w);
        for wline in wrapped {
            out.push_str(&format!("{}{}\n", prefix, wline));
        }
    }

    out
}

/// Renders a full markdown document to ANSI-styled terminal text according to the provided options.
pub fn render_markdown_terminal_styled(markdown: &str, options: &TerminalDocOptions) -> String {
    let mut out = String::with_capacity(markdown.len() * 2);
    let lines: Vec<&str> = markdown.lines().collect();
    let mut i = 0;

    while i < lines.len() {
        let line = lines[i];
        let trimmed = line.trim();

        // 1. Fenced code blocks
        if trimmed.starts_with("```") {
            let lang = trimmed.trim_start_matches('`').trim();
            i += 1;
            let mut code_lines = Vec::new();
            while i < lines.len() && !lines[i].trim().starts_with("```") {
                code_lines.push(lines[i]);
                i += 1;
            }
            if i < lines.len() {
                i += 1; // skip closing ```
            }
            let code_str = code_lines.join("\n");
            let rendered_block = render_code_block_terminal(&code_str, lang, options);
            out.push('\n');
            out.push_str(&rendered_block);
            out.push('\n');
            continue;
        }

        // 2. Horizontal rules: ---, ***, ___
        if is_horizontal_rule(trimmed) {
            let hr_color = if options.colored { "\x1b[90m" } else { "" };
            let reset = if options.colored { ansi::RESET } else { "" };
            let bar_len = options.width.min(60);
            out.push_str(&format!("\n{}{}{}\n\n", hr_color, "─".repeat(bar_len), reset));
            i += 1;
            continue;
        }

        // 3. Headings: #, ##, ###, ####, #####, ######
        if let Some((level, heading_text)) = parse_heading_line(trimmed) {
            let rendered_heading = render_inline_terminal(heading_text, options.colored);
            let reset = if options.colored { ansi::RESET } else { "" };

            match level {
                1 => {
                    let col = if options.colored { "\x1b[1;96m" } else { "" };
                    let bar_col = if options.colored { "\x1b[36m" } else { "" };
                    let bar_len = (visible_width(&rendered_heading) + 4).clamp(10, options.width);
                    out.push_str(&format!(
                        "\n{}# {}{}\n{}{}{}\n",
                        col, rendered_heading, reset,
                        bar_col, "═".repeat(bar_len), reset
                    ));
                }
                2 => {
                    let col = if options.colored { "\x1b[1;94m" } else { "" };
                    let bar_col = if options.colored { "\x1b[34m" } else { "" };
                    let bar_len = (visible_width(&rendered_heading) + 3).clamp(8, options.width);
                    out.push_str(&format!(
                        "\n{}## {}{}\n{}{}{}\n",
                        col, rendered_heading, reset,
                        bar_col, "─".repeat(bar_len), reset
                    ));
                }
                3 => {
                    let col = if options.colored { "\x1b[1;95m" } else { "" };
                    out.push_str(&format!("\n{}◆ {}{}\n", col, rendered_heading, reset));
                }
                4 => {
                    let col = if options.colored { "\x1b[1;93m" } else { "" };
                    out.push_str(&format!("\n{}▸ {}{}\n", col, rendered_heading, reset));
                }
                5 => {
                    let col = if options.colored { "\x1b[1;97m" } else { "" };
                    out.push_str(&format!("\n{}• {}{}\n", col, rendered_heading, reset));
                }
                _ => {
                    let col = if options.colored { "\x1b[2;37m" } else { "" };
                    out.push_str(&format!("\n{}  {}{}\n", col, rendered_heading, reset));
                }
            }
            i += 1;
            continue;
        }

        // 4. Tables
        if is_table_header(trimmed) && i + 1 < lines.len() && is_table_delimiter(lines[i + 1].trim()) {
            let mut table_lines = Vec::new();
            while i < lines.len() && (lines[i].trim().starts_with('|') || lines[i].trim().ends_with('|')) {
                table_lines.push(lines[i]);
                i += 1;
            }
            if let Some(rendered_table) = render_table_terminal(&table_lines, options.width, options.colored) {
                out.push('\n');
                out.push_str(&rendered_table);
                out.push('\n');
            }
            continue;
        }

        // 5. Blockquotes & Callouts
        if trimmed.starts_with('>') {
            let mut quote_lines = Vec::new();
            while i < lines.len() && lines[i].trim().starts_with('>') {
                let qline = lines[i].trim().trim_start_matches('>').trim_start();
                quote_lines.push(qline);
                i += 1;
            }

            if !quote_lines.is_empty() && quote_lines[0].starts_with("[!") && quote_lines[0].contains(']') {
                let header = quote_lines[0];
                if let Some(close_idx) = header.find(']') {
                    let ctype = &header[2..close_idx];
                    let rest = header[close_idx + 1..].trim();
                    let mut body_lines = Vec::new();
                    if !rest.is_empty() {
                        body_lines.push(rest);
                    }
                    body_lines.extend_from_slice(&quote_lines[1..]);
                    let rendered_callout = render_callout_terminal(ctype, &body_lines, options);
                    out.push('\n');
                    out.push_str(&rendered_callout);
                    out.push('\n');
                    continue;
                }
            }

            let col = if options.colored { "\x1b[36m" } else { "" };
            let italic = if options.colored { "\x1b[3m" } else { "" };
            let reset = if options.colored { ansi::RESET } else { "" };
            out.push('\n');
            for qline in quote_lines {
                let rendered = render_inline_terminal(qline, options.colored);
                let wrapped = wrap_ansi(&rendered, options.width.saturating_sub(4).max(10));
                for wline in wrapped {
                    out.push_str(&format!("{}│{} {}{}{}\n", col, reset, italic, wline, reset));
                }
            }
            out.push('\n');
            continue;
        }

        // 6. Lists
        if is_list_item(trimmed) {
            let mut list_lines = Vec::new();
            while i < lines.len() {
                let l = lines[i];
                let lt = l.trim();
                if is_list_item(lt) || (l.starts_with("  ") && !lt.is_empty()) {
                    list_lines.push(l);
                    i += 1;
                } else {
                    break;
                }
            }
            let rendered_list = render_list_terminal(&list_lines, options);
            out.push('\n');
            out.push_str(&rendered_list);
            out.push('\n');
            continue;
        }

        // 7. Empty lines
        if trimmed.is_empty() {
            out.push('\n');
            i += 1;
            continue;
        }

        // 8. Paragraphs
        let mut paragraph_lines = Vec::new();
        while i < lines.len() {
            let l = lines[i];
            let lt = l.trim();
            if lt.is_empty()
                || lt.starts_with('#')
                || lt.starts_with('>')
                || lt.starts_with("```")
                || is_horizontal_rule(lt)
                || is_list_item(lt)
                || (is_table_header(lt) && i + 1 < lines.len() && is_table_delimiter(lines[i + 1].trim()))
            {
                break;
            }
            paragraph_lines.push(l);
            i += 1;
        }

        let combined_paragraph = paragraph_lines.join(" ");
        let rendered_para = render_inline_terminal(&combined_paragraph, options.colored);
        let wrapped_para = wrap_ansi(&rendered_para, options.width);
        for wline in wrapped_para {
            out.push_str(&wline);
            out.push('\n');
        }
    }

    out
}

fn is_table_header(line: &str) -> bool {
    line.starts_with('|') && line.ends_with('|') && line.matches('|').count() >= 2
}

fn is_table_delimiter(line: &str) -> bool {
    line.starts_with('|')
        && line.ends_with('|')
        && line.chars().all(|c| c == '|' || c == '-' || c == ':' || c == ' ')
        && line.contains('-')
}

/// Renders a markdown string to ANSI-styled terminal text with word wrapping.
pub fn render_markdown_terminal(markdown: &str, width: usize) -> String {
    let mut options = TerminalDocOptions::default();
    options.width = width.max(20);
    render_markdown_terminal_styled(markdown, &options)
}

/// Helper to render markdown to terminal with default 80-column width.
pub fn render_doc_terminal(markdown: &str) -> String {
    render_markdown_terminal(markdown, 80)
}

/// Creates a paginated viewport / interactive pager for the rendered documentation.
pub fn render_doc_terminal_paged(markdown: &str, width: usize, height: usize) -> DocPager {
    DocPager::new(markdown, width, height)
}

/// Action to take following a pager key command.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PagerAction {
    Continue,
    Quit,
    Redraw,
    SearchPrompt,
}

/// Interactive and programmatic pager for viewing long documentation pages in the terminal.
#[derive(Debug, Clone)]
pub struct DocPager {
    lines: Vec<String>,
    plain_lines: Vec<String>,
    scroll_offset: usize,
    viewport_width: usize,
    viewport_height: usize,
    title: Option<String>,
    search_query: Option<String>,
    search_matches: Vec<usize>,
    current_match_idx: Option<usize>,
}

impl DocPager {
    /// Creates a new `DocPager` by rendering the markdown at `width` columns and setting the viewport height.
    pub fn new(markdown: &str, width: usize, height: usize) -> Self {
        let rendered = render_markdown_terminal(markdown, width);
        let lines: Vec<String> = rendered.lines().map(|s| s.to_string()).collect();
        Self::from_rendered_lines(lines, width, height)
    }

    /// Creates a `DocPager` directly from pre-rendered terminal lines.
    pub fn from_rendered_lines(lines: Vec<String>, width: usize, height: usize) -> Self {
        let plain_lines: Vec<String> = lines.iter().map(|l| strip_ansi(l)).collect();
        Self {
            lines,
            plain_lines,
            scroll_offset: 0,
            viewport_width: width.max(20),
            viewport_height: height.max(5),
            title: None,
            search_query: None,
            search_matches: Vec::new(),
            current_match_idx: None,
        }
    }

    /// Sets an optional title displayed in the pager header.
    pub fn set_title(&mut self, title: impl Into<String>) {
        self.title = Some(title.into());
    }

    /// Current scroll offset (line index at top of viewport).
    pub fn scroll_offset(&self) -> usize {
        self.scroll_offset
    }

    /// Total lines in rendered document.
    pub fn total_lines(&self) -> usize {
        self.lines.len()
    }

    /// Viewport width in columns.
    pub fn viewport_width(&self) -> usize {
        self.viewport_width
    }

    /// Viewport height in rows.
    pub fn viewport_height(&self) -> usize {
        self.viewport_height
    }

    /// Progress percentage through the document (0.0 to 100.0).
    pub fn progress_percentage(&self) -> f32 {
        if self.lines.is_empty() {
            return 100.0;
        }
        let end = (self.scroll_offset + self.viewport_height).min(self.lines.len());
        (end as f32 / self.lines.len() as f32) * 100.0
    }

    /// Scrolls down by `n` lines.
    pub fn scroll_down(&mut self, n: usize) {
        let max_scroll = self.lines.len().saturating_sub(self.viewport_height);
        self.scroll_offset = (self.scroll_offset + n).min(max_scroll);
    }

    /// Scrolls up by `n` lines.
    pub fn scroll_up(&mut self, n: usize) {
        self.scroll_offset = self.scroll_offset.saturating_sub(n);
    }

    /// Scrolls down by one page (viewport height minus 1 line).
    pub fn page_down(&mut self) {
        let page_step = self.viewport_height.saturating_sub(1).max(1);
        self.scroll_down(page_step);
    }

    /// Scrolls up by one page (viewport height minus 1 line).
    pub fn page_up(&mut self) {
        let page_step = self.viewport_height.saturating_sub(1).max(1);
        self.scroll_up(page_step);
    }

    /// Scrolls to top of document.
    pub fn scroll_to_top(&mut self) {
        self.scroll_offset = 0;
    }

    /// Scrolls to bottom of document.
    pub fn scroll_to_bottom(&mut self) {
        self.scroll_offset = self.lines.len().saturating_sub(self.viewport_height);
    }

    /// Scrolls to a specific line number.
    pub fn scroll_to_line(&mut self, line: usize) {
        let max_scroll = self.lines.len().saturating_sub(self.viewport_height);
        self.scroll_offset = line.min(max_scroll);
    }

    /// Searches for a text pattern and jumps to the first match. Returns match count.
    pub fn search(&mut self, query: &str) -> usize {
        let q_lower = query.trim().to_lowercase();
        self.search_matches.clear();
        self.search_query = Some(query.to_string());
        if q_lower.is_empty() {
            self.current_match_idx = None;
            return 0;
        }

        for (idx, line) in self.plain_lines.iter().enumerate() {
            if line.to_lowercase().contains(&q_lower) {
                self.search_matches.push(idx);
            }
        }

        if !self.search_matches.is_empty() {
            self.current_match_idx = Some(0);
            self.scroll_to_line(self.search_matches[0]);
        } else {
            self.current_match_idx = None;
        }

        self.search_matches.len()
    }

    /// Jumps to next match in search results.
    pub fn next_match(&mut self) -> Option<usize> {
        if self.search_matches.is_empty() {
            return None;
        }
        let cur = self.current_match_idx.unwrap_or(0);
        let next_idx = (cur + 1) % self.search_matches.len();
        self.current_match_idx = Some(next_idx);
        let target_line = self.search_matches[next_idx];
        self.scroll_to_line(target_line);
        Some(target_line)
    }

    /// Jumps to previous match in search results.
    pub fn prev_match(&mut self) -> Option<usize> {
        if self.search_matches.is_empty() {
            return None;
        }
        let cur = self.current_match_idx.unwrap_or(0);
        let prev_idx = if cur == 0 {
            self.search_matches.len() - 1
        } else {
            cur - 1
        };
        self.current_match_idx = Some(prev_idx);
        let target_line = self.search_matches[prev_idx];
        self.scroll_to_line(target_line);
        Some(target_line)
    }

    /// Clears active search.
    pub fn clear_search(&mut self) {
        self.search_query = None;
        self.search_matches.clear();
        self.current_match_idx = None;
    }

    /// Returns the slice of lines currently visible in the viewport.
    pub fn render_viewport_lines(&self) -> Vec<String> {
        if self.lines.is_empty() {
            return vec![String::new(); self.viewport_height];
        }
        let start = self.scroll_offset;
        let end = (start + self.viewport_height).min(self.lines.len());
        let mut slice = self.lines[start..end].to_vec();
        while slice.len() < self.viewport_height {
            slice.push(String::new());
        }
        slice
    }

    /// Renders the pager status bar string.
    pub fn render_status_bar(&self) -> String {
        let total = self.lines.len();
        let start = if total == 0 { 0 } else { self.scroll_offset + 1 };
        let end = (self.scroll_offset + self.viewport_height).min(total);
        let pct = self.progress_percentage();

        let search_info = if let Some(query) = &self.search_query {
            if let Some(cur) = self.current_match_idx {
                format!(" [{}/{} matches for '{}']", cur + 1, self.search_matches.len(), query)
            } else {
                format!(" [0 matches for '{}']", query)
            }
        } else {
            String::new()
        };

        format!(
            "\x1b[7m [Lines {}-{}/{} ({:.0}%){} ]  'j/k': scroll | 'd/u': page | '/': search | 'q': quit \x1b[0m",
            start, end, total, pct, search_info
        )
    }

    /// Renders the complete view (optional title header, content lines, status bar).
    pub fn render_view(&self) -> String {
        let mut out = String::new();
        if let Some(title) = &self.title {
            let title_line = format!("\x1b[1;96m=== {} ===\x1b[0m\n", title);
            out.push_str(&title_line);
        }
        for line in self.render_viewport_lines() {
            out.push_str(&line);
            out.push('\n');
        }
        out.push_str(&self.render_status_bar());
        out
    }

    /// Total page count.
    pub fn total_pages(&self) -> usize {
        if self.lines.is_empty() {
            1
        } else {
            (self.lines.len() + self.viewport_height - 1) / self.viewport_height
        }
    }

    /// Renders a specific page (0-indexed).
    pub fn render_page(&self, page_index: usize) -> String {
        let page_size = self.viewport_height;
        let start = (page_index * page_size).min(self.lines.len());
        let end = (start + page_size).min(self.lines.len());
        let mut out = String::new();
        if start < self.lines.len() {
            for line in &self.lines[start..end] {
                out.push_str(line);
                out.push('\n');
            }
        }
        let total_p = self.total_pages();
        out.push_str(&format!(
            "\x1b[7m [Page {}/{}]  'j/k': scroll | 'd/u': page | 'q': quit \x1b[0m",
            page_index + 1,
            total_p
        ));
        out
    }

    /// Dispatches a key command string (e.g. "j", "k", "d", "u", "g", "G", "q", "/search").
    pub fn handle_key_command(&mut self, command: &str) -> PagerAction {
        let cmd = command.trim();
        match cmd {
            "j" | "down" | "s" => {
                self.scroll_down(1);
                PagerAction::Continue
            }
            "k" | "up" | "w" => {
                self.scroll_up(1);
                PagerAction::Continue
            }
            "d" | "pagedown" | " " => {
                self.page_down();
                PagerAction::Continue
            }
            "u" | "pageup" | "b" => {
                self.page_up();
                PagerAction::Continue
            }
            "g" | "home" | "top" => {
                self.scroll_to_top();
                PagerAction::Continue
            }
            "G" | "end" | "bottom" => {
                self.scroll_to_bottom();
                PagerAction::Continue
            }
            "n" => {
                self.next_match();
                PagerAction::Continue
            }
            "N" => {
                self.prev_match();
                PagerAction::Continue
            }
            "q" | "quit" | "exit" | "esc" => PagerAction::Quit,
            "r" | "redraw" => PagerAction::Redraw,
            s if s.starts_with('/') => {
                let query = &s[1..];
                self.search(query);
                PagerAction::Continue
            }
            _ => PagerAction::Continue,
        }
    }
}

/// Configurable builder for terminal markdown rendering.
#[derive(Debug, Clone, Default)]
pub struct TerminalDocRenderer {
    options: TerminalDocOptions,
}

impl TerminalDocRenderer {
    /// Creates a new `TerminalDocRenderer` with default settings.
    pub fn new() -> Self {
        Self::default()
    }

    /// Sets terminal column width.
    pub fn with_width(mut self, width: usize) -> Self {
        self.options.width = width;
        self
    }

    /// Enables or disables ANSI colors.
    pub fn with_colored(mut self, colored: bool) -> Self {
        self.options.colored = colored;
        self
    }

    /// Enables or disables syntax highlighting.
    pub fn with_syntax_highlighting(mut self, syntax_highlighting: bool) -> Self {
        self.options.syntax_highlighting = syntax_highlighting;
        self
    }

    /// Sets code block border style.
    pub fn with_code_border_style(mut self, style: CodeBorderStyle) -> Self {
        self.options.code_border_style = style;
        self
    }

    /// Sets whether to show line numbers in code blocks.
    pub fn with_line_numbers(mut self, line_numbers: bool) -> Self {
        self.options.line_numbers_in_code = line_numbers;
        self
    }

    /// Renders markdown string to terminal string.
    pub fn render(&self, markdown: &str) -> String {
        render_markdown_terminal_styled(markdown, &self.options)
    }

    /// Renders markdown and creates a `DocPager`.
    pub fn render_paged(&self, markdown: &str, height: usize) -> DocPager {
        let rendered = self.render(markdown);
        let lines: Vec<String> = rendered.lines().map(|s| s.to_string()).collect();
        DocPager::from_rendered_lines(lines, self.options.width, height)
    }
}

// ============================================================================
// Unit Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_escape_html() {
        assert_eq!(escape_html("<div class=\"test\">&'</div>"), "&lt;div class=&quot;test&quot;&gt;&amp;&#39;&lt;/div&gt;");
    }

    #[test]
    fn test_slugify() {
        assert_eq!(slugify("Hello World!"), "hello-world");
        assert_eq!(slugify("API Reference: v2.0 (Beta)"), "api-reference-v2-0-beta");
        assert_eq!(slugify("   "), "section");
    }

    #[test]
    fn test_extract_headings() {
        let md = r#"
# Introduction
Some text here.
## Getting Started
### Installation
```rust
# Ignore this heading inside code block
fn main() {}
```
## Getting Started
"#;
        let headings = extract_headings(md);
        assert_eq!(headings.len(), 4);
        assert_eq!(headings[0].title, "Introduction");
        assert_eq!(headings[0].level, 1);
        assert_eq!(headings[0].id, "introduction");

        assert_eq!(headings[1].title, "Getting Started");
        assert_eq!(headings[1].level, 2);
        assert_eq!(headings[1].id, "getting-started");

        assert_eq!(headings[2].title, "Installation");
        assert_eq!(headings[2].level, 3);
        assert_eq!(headings[2].id, "installation");

        // Duplicate heading ID incremented
        assert_eq!(headings[3].title, "Getting Started");
        assert_eq!(headings[3].id, "getting-started-1");
    }

    #[test]
    fn test_inline_formatting() {
        let md = "**bold** and *italic* and ***bold-italic*** with `code` and ~~strike~~ and ==mark==";
        let html = render_inline_html(md);
        assert!(html.contains("<strong>bold</strong>"));
        assert!(html.contains("<em>italic</em>"));
        assert!(html.contains("<strong><em>bold-italic</em></strong>"));
        assert!(html.contains("<code>code</code>"));
        assert!(html.contains("<del>strike</del>"));
        assert!(html.contains("<mark>mark</mark>"));
    }

    #[test]
    fn test_links_and_images() {
        let link_md = "[Fusion Assistant](https://github.com/theaungmyatmoe/fusion \"Official Repo\")";
        let link_html = render_inline_html(link_md);
        assert!(link_html.contains("<a href=\"https://github.com/theaungmyatmoe/fusion\" title=\"Official Repo\" target=\"_blank\" rel=\"noopener noreferrer\">Fusion Assistant</a>"));

        let img_md = "![Banner Logo](/assets/logo.png \"Banner\")";
        let img_html = render_inline_html(img_md);
        assert!(img_html.contains("<img src=\"/assets/logo.png\" alt=\"Banner Logo\" title=\"Banner\" loading=\"lazy\" />"));
    }

    #[test]
    fn test_code_block_highlighting() {
        let md = "```rust\npub fn hello_world() -> bool {\n    println!(\"Hello\");\n    true\n}\n```";
        let html = markdown_to_html(md);
        assert!(html.contains("<div class=\"code-block-wrapper\">"));
        assert!(html.contains("<span class=\"code-lang-badge\">rust</span>"));
        assert!(html.contains("<button class=\"copy-code-btn\""));
        assert!(html.contains("<span class=\"hl-kw\">pub</span>"));
        assert!(html.contains("<span class=\"hl-kw\">fn</span>"));
        assert!(html.contains("<span class=\"hl-fn\">hello_world</span>"));
        assert!(html.contains("<span class=\"hl-type\">bool</span>"));
    }

    #[test]
    fn test_callouts_admonitions() {
        let md = "> [!NOTE] Custom Note Title\n> This is a helpful note message.";
        let html = markdown_to_html(md);
        assert!(html.contains("<div class=\"admonition admonition-note\">"));
        assert!(html.contains("Custom Note Title"));
        assert!(html.contains("This is a helpful note message."));

        let tip_md = "> [!TIP]\n> Pro-tip here.";
        let tip_html = markdown_to_html(tip_md);
        assert!(tip_html.contains("<div class=\"admonition admonition-tip\">"));
        assert!(tip_html.contains("Tip"));
    }

    #[test]
    fn test_lists_and_tasks() {
        let md = "- Item 1\n- Item 2\n- [ ] Task unfinished\n- [x] Task finished";
        let html = markdown_to_html(md);
        assert!(html.contains("<ul>"));
        assert!(html.contains("<li>Item 1</li>"));
        assert!(html.contains("<li class=\"task-list-item\"><input type=\"checkbox\" disabled /> Task unfinished</li>"));
        assert!(html.contains("<li class=\"task-list-item\"><input type=\"checkbox\" checked disabled /> Task finished</li>"));
    }

    #[test]
    fn test_tables() {
        let md = r#"
| Command | Description | Shortcut |
| :--- | :---: | ---: |
| `/help` | Display help | `Ctrl+H` |
| `/clear` | Clear screen | `Ctrl+L` |
"#;
        let html = markdown_to_html(md);
        assert!(html.contains("<table class=\"doc-table\">"));
        assert!(html.contains("<th style=\"text-align: left\">Command</th>"));
        assert!(html.contains("<th style=\"text-align: center\">Description</th>"));
        assert!(html.contains("<th style=\"text-align: right\">Shortcut</th>"));
        assert!(html.contains("<td style=\"text-align: left\"><code>/help</code></td>"));
    }

    #[test]
    fn test_standalone_doc_page_generator() {
        let md = "# Documentation Title\n\nWelcome to Fusion Docs.\n\n## Quick Start\n\nRun `fusion` in terminal.";
        let config = DocConfig::builder()
            .title("Fusion API")
            .version("0.3.0")
            .author("Fusion Team")
            .theme(DocTheme::Nord)
            .show_toc(true)
            .show_sidebar(true)
            .show_search(true)
            .add_nav_section(NavSection::new("Guides", vec![NavItem::new("Quick Start", "quickstart.html").with_active(true)]))
            .build();

        let page_html = generate_doc_page(md, &config);
        assert!(page_html.contains("<!DOCTYPE html>"));
        assert!(page_html.contains("<title>Fusion API</title>"));
        assert!(page_html.contains("v0.3.0"));
        assert!(page_html.contains("--doc-bg-primary: #2e3440;")); // Nord theme
        assert!(page_html.contains("id=\"documentation-title\""));
        assert!(page_html.contains("id=\"quick-start\""));
        assert!(page_html.contains("class=\"doc-toc\""));
        assert!(page_html.contains("Fusion Documentation Engine Runtime JS"));
    }

    #[test]
    fn test_render_doc_page_helper() {
        let md = "# Simple Doc\nContent here.";
        let html = render_doc_page(md, "My Simple Doc");
        assert!(html.contains("<!DOCTYPE html>"));
        assert!(html.contains("<title>My Simple Doc</title>"));
        assert!(html.contains("Simple Doc"));
    }

    #[test]
    fn test_generate_doc_site() {
        let pages = vec![
            DocPage::new("index", "Home", "# Welcome to Docs\nIntroductory content.")
                .with_category("Getting Started")
                .with_order(0),
            DocPage::new("config", "Configuration", "# Config Guide\nSettings details.")
                .with_category("Getting Started")
                .with_order(1),
            DocPage::new("api", "API Reference", "# API Reference\nEndpoints details.")
                .with_category("Reference")
                .with_order(0),
        ];

        let config = DocConfig::new("Fusion Assistant Site");
        let site = generate_doc_site(&pages, &config);

        assert_eq!(site.len(), 3);
        assert!(site.contains_key("index.html"));
        assert!(site.contains_key("config.html"));
        assert!(site.contains_key("api.html"));

        let index_html = &site["index.html"];
        assert!(index_html.contains("Home - Fusion Assistant Site"));
        assert!(index_html.contains("Getting Started"));
        assert!(index_html.contains("Reference"));
    }

    #[test]
    fn test_multilingual_code_highlighting() {
        let py_code = "```python\ndef greet(name: str) -> None:\n    # print greeting\n    print(f\"Hello {name}\")\n```";
        let py_html = markdown_to_html(py_code);
        assert!(py_html.contains("<span class=\"hl-kw\">def</span>"));
        assert!(py_html.contains("<span class=\"hl-comment\"># print greeting</span>"));

        let js_code = "```javascript\nconst compute = async (x) => {\n    // compute result\n    return x * 2;\n};\n```";
        let js_html = markdown_to_html(js_code);
        assert!(js_html.contains("<span class=\"hl-kw\">const</span>"));
        assert!(js_html.contains("<span class=\"hl-kw\">async</span>"));

        let sql_code = "```sql\nSELECT id, name FROM users WHERE active = 1;\n```";
        let sql_html = markdown_to_html(sql_code);
        assert!(sql_html.contains("<span class=\"hl-kw\">SELECT</span>"));
        assert!(sql_html.contains("<span class=\"hl-kw\">FROM</span>"));
    }

    #[test]
    fn test_details_summary() {
        let md = "<details>\n<summary>Click to view details</summary>\nHidden content revealed.\n</details>";
        let html = markdown_to_html(md);
        assert!(html.contains("<details class=\"doc-details\">"));
        assert!(html.contains("<summary>Click to view details</summary>"));
        assert!(html.contains("Hidden content revealed."));
    }

    #[test]
    fn test_custom_theme() {
        let custom = ThemeColors {
            bg_primary: "#121212".to_string(),
            bg_secondary: "#1e1e1e".to_string(),
            bg_card: "#2a2a2a".to_string(),
            bg_code: "#000000".to_string(),
            text_primary: "#ffffff".to_string(),
            text_secondary: "#cccccc".to_string(),
            text_muted: "#888888".to_string(),
            text_link: "#4dabf7".to_string(),
            accent: "#e599f7".to_string(),
            accent_hover: "#da77f2".to_string(),
            border: "#333333".to_string(),
            border_subtle: "#222222".to_string(),
        };
        let config = DocConfig::builder()
            .title("Custom Themed Doc")
            .theme(DocTheme::Custom(custom))
            .build();
        let html = generate_doc_page("# Hello Custom", &config);
        assert!(html.contains("--doc-bg-primary: #121212;"));
        assert!(html.contains("--doc-accent: #e599f7;"));
    }

    #[test]
    fn test_all_themes_render() {
        let themes = vec![
            DocTheme::Auto,
            DocTheme::Light,
            DocTheme::Dark,
            DocTheme::Nord,
            DocTheme::Cyberpunk,
            DocTheme::SolarizedLight,
            DocTheme::SolarizedDark,
        ];
        for t in themes {
            let config = DocConfig::builder().title("Theme Test").theme(t).build();
            let html = generate_doc_page("# Title", &config);
            assert!(html.contains("<!DOCTYPE html>"));
        }
    }

    #[test]
    fn test_empty_and_special_chars() {
        let empty_html = markdown_to_html("");
        assert_eq!(empty_html, "");

        let special_md = "# <b>Dangerous & Unsafe</b>\n<script>alert(1)</script>";
        let special_html = markdown_to_html(special_md);
        assert!(!special_html.contains("<script>"));
        assert!(special_html.contains("&lt;script&gt;"));
    }

    #[test]
    fn test_breadcrumbs_and_repo_link() {
        let config = DocConfig::builder()
            .title("Repo Doc")
            .repo_url("https://github.com/theaungmyatmoe/fusion")
            .add_breadcrumb("Home", "index.html")
            .add_breadcrumb("Section", "section.html")
            .build();
        let html = generate_doc_page("# Content", &config);
        assert!(html.contains("https://github.com/theaungmyatmoe/fusion"));
        assert!(html.contains("class=\"doc-breadcrumbs\""));
        assert!(html.contains("index.html"));
        assert!(html.contains("section.html"));
    }

    // ========================================================================
    // Terminal Rendering & Pager Tests
    // ========================================================================

    #[test]
    fn test_strip_ansi() {
        assert_eq!(strip_ansi("Hello World"), "Hello World");
        assert_eq!(strip_ansi("\x1b[31mRed Text\x1b[0m"), "Red Text");
        assert_eq!(strip_ansi("\x1b[1;32;48;5;236mStyled\x1b[0m"), "Styled");
        assert_eq!(
            strip_ansi("\x1b]8;;https://example.com\x07Click Here\x1b]8;;\x07"),
            "Click Here"
        );
        assert_eq!(strip_ansi(""), "");
    }

    #[test]
    fn test_visible_width() {
        assert_eq!(visible_width("Hello"), 5);
        assert_eq!(visible_width("\x1b[31;1mHello\x1b[0m"), 5);
        assert_eq!(visible_width("Hello\t"), 9); // tab = 4
        // Unicode characters: CJK characters take 2 columns
        assert_eq!(visible_width("世界"), 4);
        assert_eq!(visible_width("\x1b[32m🦀 Rust 世界\x1b[0m"), 2 + 1 + 4 + 1 + 4);
        // Zero width
        assert_eq!(visible_width("\u{200B}"), 0);
    }

    #[test]
    fn test_wrap_ansi_plain() {
        let text = "The quick brown fox jumps over the lazy dog";
        let wrapped = wrap_ansi(text, 15);
        assert_eq!(wrapped.len(), 3);
        assert_eq!(wrapped[0], "The quick brown");
        assert_eq!(wrapped[1], "fox jumps over");
        assert_eq!(wrapped[2], "the lazy dog");
        for line in &wrapped {
            assert!(visible_width(line) <= 15);
        }
    }

    #[test]
    fn test_wrap_ansi_colored() {
        let text = "\x1b[31mThe quick brown fox jumps over the lazy dog\x1b[0m";
        let wrapped = wrap_ansi(text, 15);
        assert_eq!(wrapped.len(), 3);
        for line in &wrapped {
            assert!(visible_width(line) <= 15);
            // Line must retain color
            assert!(line.contains("\x1b[31m"));
        }
        assert_eq!(strip_ansi(&wrapped[0]), "The quick brown");
        assert_eq!(strip_ansi(&wrapped[1]), "fox jumps over");
        assert_eq!(strip_ansi(&wrapped[2]), "the lazy dog");
    }

    #[test]
    fn test_wrap_ansi_long_word() {
        let text = "https://example.com/very/long/unbroken/url/that/exceeds/terminal/width";
        let wrapped = wrap_ansi(text, 20);
        assert!(wrapped.len() >= 3);
        for line in &wrapped {
            assert!(visible_width(line) <= 20);
        }
        let joined = wrapped.concat();
        assert_eq!(joined, text);
    }

    #[test]
    fn test_terminal_headers() {
        let md = "# Level 1\n## Level 2\n### Level 3\n#### Level 4\n##### Level 5\n###### Level 6";
        let rendered = render_markdown_terminal(md, 80);
        assert!(rendered.contains("# Level 1"));
        assert!(rendered.contains("══"));
        assert!(rendered.contains("## Level 2"));
        assert!(rendered.contains("──"));
        assert!(rendered.contains("◆ Level 3"));
        assert!(rendered.contains("▸ Level 4"));
        assert!(rendered.contains("• Level 5"));
        assert!(rendered.contains("Level 6"));
    }

    #[test]
    fn test_terminal_inline_formatting() {
        let md = "**bold** and *italic* and ***bold-italic*** and `code` and ~~strike~~ and ==highlight==";
        let rendered = render_inline_terminal(md, true);
        assert!(rendered.contains(ansi::BOLD));
        assert!(rendered.contains(ansi::ITALIC));
        assert!(rendered.contains(ansi::STRIKETHROUGH));
        assert!(rendered.contains(ansi::BG_HIGHLIGHT));
        assert!(rendered.contains(ansi::HL_INLINE_CODE));
        assert!(rendered.contains("bold"));
        assert!(rendered.contains("italic"));
        assert!(rendered.contains("code"));

        // Uncolored version strips markup
        let plain = render_inline_terminal(md, false);
        assert_eq!(plain, "bold and italic and bold-italic and code and strike and highlight");
    }

    #[test]
    fn test_terminal_syntax_highlighting() {
        let rust_code = r#"// A comment
pub fn main() {
    let mut count: i32 = 42;
    println!("Hello, count: {}", count);
}"#;
        let highlighted = highlight_code_terminal(rust_code, "rust");
        assert!(highlighted.contains(ansi::HL_COMMENT));
        assert!(highlighted.contains(ansi::HL_KW)); // fn, pub, let, mut
        assert!(highlighted.contains(ansi::HL_TYPE)); // i32
        assert!(highlighted.contains(ansi::HL_STR)); // "Hello, count: {}"
        assert!(highlighted.contains(ansi::HL_NUM)); // 42
        assert!(highlighted.contains(ansi::HL_MACRO)); // println!
    }

    #[test]
    fn test_terminal_code_block_frame() {
        let md = "```rust\nfn main() {\n    println!(\"hi\");\n}\n```";
        let rendered = render_markdown_terminal(md, 60);
        assert!(rendered.contains("╭─"));
        assert!(rendered.contains("rust"));
        assert!(rendered.contains("│"));
        assert!(rendered.contains("1"));
        assert!(rendered.contains("2"));
        assert!(rendered.contains("3"));
        assert!(rendered.contains("╰─"));
    }

    #[test]
    fn test_terminal_table_rendering() {
        let table_md = r#"| Option | Type | Default | Description |
| :--- | :---: | ---: | :--- |
| width | usize | 80 | Terminal column width |
| colored | bool | true | Enable ANSI colors |"#;
        let rendered = render_markdown_terminal(table_md, 80);
        assert!(rendered.contains('┌'));
        assert!(rendered.contains('┬'));
        assert!(rendered.contains('┐'));
        assert!(rendered.contains('├'));
        assert!(rendered.contains('┼'));
        assert!(rendered.contains('┤'));
        assert!(rendered.contains('└'));
        assert!(rendered.contains('┴'));
        assert!(rendered.contains('┘'));
        assert!(rendered.contains("Option"));
        assert!(rendered.contains("Type"));
        assert!(rendered.contains("Default"));
        assert!(rendered.contains("Description"));
        assert!(rendered.contains("width"));
        assert!(rendered.contains("colored"));
    }

    #[test]
    fn test_terminal_callouts() {
        let md = "> [!NOTE]\n> This is an important note.\n\n> [!WARNING]\n> Watch out!";
        let rendered = render_markdown_terminal(md, 60);
        assert!(rendered.contains("NOTE"));
        assert!(rendered.contains("ℹ"));
        assert!(rendered.contains("This is an important note."));
        assert!(rendered.contains("WARNING"));
        assert!(rendered.contains("⚠"));
        assert!(rendered.contains("Watch out!"));
    }

    #[test]
    fn test_terminal_lists() {
        let md = r#"- Item 1
- Item 2
  - Nested item
- [ ] Incomplete task
- [x] Completed task
1. First numbered
2. Second numbered"#;
        let rendered = render_markdown_terminal(md, 60);
        assert!(rendered.contains("•"));
        assert!(rendered.contains("Item 1"));
        assert!(rendered.contains("Item 2"));
        assert!(rendered.contains("○"));
        assert!(rendered.contains("Nested item"));
        assert!(rendered.contains("☐"));
        assert!(rendered.contains("Incomplete task"));
        assert!(rendered.contains("☑"));
        assert!(rendered.contains("Completed task"));
        assert!(rendered.contains("1."));
        assert!(rendered.contains("First numbered"));
        assert!(rendered.contains("2."));
        assert!(rendered.contains("Second numbered"));
    }

    #[test]
    fn test_doc_pager_scrolling_and_navigation() {
        let md = "# Title\n\nLine 1\n\nLine 2\n\nLine 3\n\nLine 4\n\nLine 5\n\nLine 6\n\nLine 7\n\nLine 8\n\nLine 9\n\nLine 10";
        let mut pager = render_doc_terminal_paged(md, 60, 5);
        pager.set_title("API Docs");

        assert_eq!(pager.scroll_offset(), 0);
        assert!(pager.total_lines() > 10);

        pager.scroll_down(3);
        assert_eq!(pager.scroll_offset(), 3);

        pager.scroll_up(2);
        assert_eq!(pager.scroll_offset(), 1);

        pager.page_down();
        assert_eq!(pager.scroll_offset(), 5);

        pager.page_up();
        assert_eq!(pager.scroll_offset(), 1);

        pager.scroll_to_bottom();
        let max_scroll = pager.total_lines().saturating_sub(5);
        assert_eq!(pager.scroll_offset(), max_scroll);

        pager.scroll_to_top();
        assert_eq!(pager.scroll_offset(), 0);

        let view = pager.render_view();
        assert!(view.contains("API Docs"));
        assert!(view.contains("Lines 1-5"));
    }

    #[test]
    fn test_doc_pager_search_and_matches() {
        let md = "First paragraph\n\nSecond paragraph target here\n\nThird paragraph\n\nFourth target here";
        let mut pager = DocPager::new(md, 60, 5);

        let match_count = pager.search("target");
        assert_eq!(match_count, 2);
        assert_eq!(pager.scroll_offset(), pager.search_matches[0]);

        let next_line = pager.next_match();
        assert!(next_line.is_some());
        assert_eq!(next_line, Some(pager.search_matches[1]));

        let prev_line = pager.prev_match();
        assert_eq!(prev_line, Some(pager.search_matches[0]));

        pager.clear_search();
        assert_eq!(pager.search("nonexistent_term_xyz"), 0);
    }

    #[test]
    fn test_doc_pager_key_commands() {
        let md = "Line A\nLine B\nLine C\nLine D\nLine E\nLine F\nLine G\nLine H\nLine I\nLine J";
        let mut pager = DocPager::new(md, 60, 4);

        assert_eq!(pager.handle_key_command("j"), PagerAction::Continue);
        assert_eq!(pager.scroll_offset(), 1);

        assert_eq!(pager.handle_key_command("k"), PagerAction::Continue);
        assert_eq!(pager.scroll_offset(), 0);

        assert_eq!(pager.handle_key_command("d"), PagerAction::Continue);
        assert_eq!(pager.scroll_offset(), 3);

        assert_eq!(pager.handle_key_command("u"), PagerAction::Continue);
        assert_eq!(pager.scroll_offset(), 0);

        assert_eq!(pager.handle_key_command("G"), PagerAction::Continue);
        assert!(pager.scroll_offset() > 0);

        assert_eq!(pager.handle_key_command("g"), PagerAction::Continue);
        assert_eq!(pager.scroll_offset(), 0);

        assert_eq!(pager.handle_key_command("q"), PagerAction::Quit);
        assert_eq!(pager.handle_key_command("/Line"), PagerAction::Continue);
    }

    #[test]
    fn test_terminal_doc_renderer_builder() {
        let renderer = TerminalDocRenderer::new()
            .with_width(50)
            .with_colored(false)
            .with_code_border_style(CodeBorderStyle::Plain)
            .with_line_numbers(false);

        let md = "# Title\n\nSome paragraph text that will be wrapped cleanly.\n\n```rust\nlet x = 10;\n```";
        let rendered = renderer.render(md);
        assert!(rendered.contains("Title"));
        assert!(rendered.contains("let x = 10;"));
        // Uncolored output contains no ESC characters
        assert!(!rendered.contains('\x1b'));

        let pager = renderer.render_paged(md, 10);
        assert_eq!(pager.viewport_width(), 50);
        assert_eq!(pager.viewport_height(), 10);
    }
}
