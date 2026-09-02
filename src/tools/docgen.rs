//! Code documentation generator tool.
//!
//! Extracts signatures, parameters, return types, and modifiers from source code
//! across multiple programming languages (Rust, TypeScript, JavaScript, Python, Go,
//! C/C++, Java, C#, Kotlin, Swift, Ruby, PHP, Zig, Dart, Lua, Shell, SQL),
//! and drafts idiomatic doc comments (Rustdoc, JSDoc, Python Google/Sphinx/NumPy,
//! GoDoc, JavaDoc, Doxygen, Markdown), audits doc coverage, and generates API references.

use async_trait::async_trait;
use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::path::Path;
use std::sync::LazyLock;

use crate::tools::file::resolve_path;
use crate::tools::symbols::SymbolKind;
use crate::tools::types::{Tool, ToolContext};

// ---------------------------------------------------------------------------
// Documentation Styles & Actions
// ---------------------------------------------------------------------------

/// Supported documentation comment styles.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DocStyle {
    Rustdoc,
    Jsdoc,
    Google,
    Sphinx,
    Numpy,
    Godoc,
    Javadoc,
    Doxygen,
    Markdown,
    Auto,
}

impl DocStyle {
    pub fn as_str(&self) -> &'static str {
        match self {
            DocStyle::Rustdoc => "rustdoc",
            DocStyle::Jsdoc => "jsdoc",
            DocStyle::Google => "google",
            DocStyle::Sphinx => "sphinx",
            DocStyle::Numpy => "numpy",
            DocStyle::Godoc => "godoc",
            DocStyle::Javadoc => "javadoc",
            DocStyle::Doxygen => "doxygen",
            DocStyle::Markdown => "markdown",
            DocStyle::Auto => "auto",
        }
    }

    pub fn from_str_loose(s: &str) -> Self {
        match s.trim().to_lowercase().as_str() {
            "rust" | "rustdoc" | "rs" => DocStyle::Rustdoc,
            "js" | "ts" | "jsdoc" | "tsdoc" | "javascript" | "typescript" => DocStyle::Jsdoc,
            "google" | "google-docstring" | "py-google" => DocStyle::Google,
            "sphinx" | "rest" | "rst" | "py-sphinx" => DocStyle::Sphinx,
            "numpy" | "numpydoc" | "scipy" => DocStyle::Numpy,
            "go" | "godoc" | "golang" => DocStyle::Godoc,
            "java" | "javadoc" | "kdoc" | "kotlin" => DocStyle::Javadoc,
            "doxygen" | "c" | "cpp" | "c++" => DocStyle::Doxygen,
            "md" | "markdown" | "api" => DocStyle::Markdown,
            _ => DocStyle::Auto,
        }
    }
}

/// Action modes supported by the docgen tool.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DocgenAction {
    /// Draft doc comments for symbols or file (default).
    Draft,
    /// Extract structured signature metadata without drafting comments.
    Extract,
    /// Audit doc coverage across symbols in file or workspace.
    Audit,
    /// Insert drafted doc comments into the source code at exact locations.
    Apply,
    /// Generate complete Markdown API reference manual.
    Markdown,
}

impl DocgenAction {
    pub fn from_str_loose(s: &str) -> Self {
        match s.trim().to_lowercase().as_str() {
            "extract" | "signatures" | "inspect" => DocgenAction::Extract,
            "audit" | "coverage" | "check" | "stats" => DocgenAction::Audit,
            "apply" | "insert" | "write" | "patch" => DocgenAction::Apply,
            "markdown" | "md" | "api_doc" | "doc" | "docs" => DocgenAction::Markdown,
            _ => DocgenAction::Draft,
        }
    }
}

// ---------------------------------------------------------------------------
// Supported Languages
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DocLanguage {
    Rust,
    TypeScript,
    JavaScript,
    Python,
    Go,
    Cpp,
    C,
    Java,
    CSharp,
    Kotlin,
    Swift,
    Ruby,
    Php,
    Zig,
    Dart,
    Lua,
    Shell,
    Sql,
    Generic,
}

impl DocLanguage {
    pub fn as_str(&self) -> &'static str {
        match self {
            DocLanguage::Rust => "rust",
            DocLanguage::TypeScript => "typescript",
            DocLanguage::JavaScript => "javascript",
            DocLanguage::Python => "python",
            DocLanguage::Go => "go",
            DocLanguage::Cpp => "cpp",
            DocLanguage::C => "c",
            DocLanguage::Java => "java",
            DocLanguage::CSharp => "csharp",
            DocLanguage::Kotlin => "kotlin",
            DocLanguage::Swift => "swift",
            DocLanguage::Ruby => "ruby",
            DocLanguage::Php => "php",
            DocLanguage::Zig => "zig",
            DocLanguage::Dart => "dart",
            DocLanguage::Lua => "lua",
            DocLanguage::Shell => "shell",
            DocLanguage::Sql => "sql",
            DocLanguage::Generic => "generic",
        }
    }

    pub fn from_extension(ext: &str) -> Self {
        let e = ext.to_lowercase();
        match e.as_str() {
            "rs" => DocLanguage::Rust,
            "ts" | "tsx" | "mts" | "cts" => DocLanguage::TypeScript,
            "js" | "jsx" | "mjs" | "cjs" => DocLanguage::JavaScript,
            "py" | "pyi" | "pyw" => DocLanguage::Python,
            "go" => DocLanguage::Go,
            "cpp" | "cxx" | "cc" | "hpp" | "hxx" | "hh" => DocLanguage::Cpp,
            "c" | "h" => DocLanguage::C,
            "java" => DocLanguage::Java,
            "cs" => DocLanguage::CSharp,
            "kt" | "kts" => DocLanguage::Kotlin,
            "swift" => DocLanguage::Swift,
            "rb" | "rake" => DocLanguage::Ruby,
            "php" | "phtml" => DocLanguage::Php,
            "zig" => DocLanguage::Zig,
            "dart" => DocLanguage::Dart,
            "lua" => DocLanguage::Lua,
            "sh" | "bash" | "zsh" => DocLanguage::Shell,
            "sql" => DocLanguage::Sql,
            _ => DocLanguage::Generic,
        }
    }

    pub fn from_path_or_name(s: &str) -> Self {
        let clean = s.trim().to_lowercase();
        if let Some(ext) = Path::new(&clean).extension().and_then(|e| e.to_str()) {
            return Self::from_extension(ext);
        }
        let stripped = clean.strip_prefix('.').unwrap_or(&clean);
        match stripped {
            "rs" | "rust" => DocLanguage::Rust,
            "ts" | "tsx" | "typescript" => DocLanguage::TypeScript,
            "js" | "jsx" | "javascript" | "node" => DocLanguage::JavaScript,
            "py" | "pyi" | "python" => DocLanguage::Python,
            "go" | "golang" => DocLanguage::Go,
            "cpp" | "cxx" | "cc" | "c++" => DocLanguage::Cpp,
            "c" | "h" => DocLanguage::C,
            "java" => DocLanguage::Java,
            "cs" | "csharp" | "c#" => DocLanguage::CSharp,
            "kt" | "kts" | "kotlin" => DocLanguage::Kotlin,
            "swift" => DocLanguage::Swift,
            "rb" | "ruby" => DocLanguage::Ruby,
            "php" => DocLanguage::Php,
            "zig" => DocLanguage::Zig,
            "dart" => DocLanguage::Dart,
            "lua" => DocLanguage::Lua,
            "sh" | "bash" | "zsh" | "shell" => DocLanguage::Shell,
            "sql" => DocLanguage::Sql,
            _ => Self::from_extension(stripped),
        }
    }

    pub fn detect_from_code(code: &str) -> Self {
        if code.contains("fn ") || code.contains("pub fn ") || code.contains("impl ") || code.contains("use std::") || code.contains("//!") || code.contains("///") || code.contains("pub struct ") || code.contains("pub enum ") || code.contains("pub mod ") || code.contains("mod ") {
            DocLanguage::Rust
        } else if code.contains("def ") && (code.contains("import ") || code.contains("self") || code.contains(":") && !code.contains("{")) {
            DocLanguage::Python
        } else if code.contains("func ") && (code.contains("package ") || code.contains("import (")) {
            DocLanguage::Go
        } else if code.contains("interface ") || code.contains("export ") || code.contains(": string") || code.contains(": number") {
            DocLanguage::TypeScript
        } else if code.contains("function ") || code.contains("const ") || code.contains("let ") || code.contains("var ") {
            DocLanguage::JavaScript
        } else if code.contains("#include ") || code.contains("std::") {
            DocLanguage::Cpp
        } else if code.contains("public class ") || code.contains("private void ") {
            DocLanguage::Java
        } else {
            DocLanguage::Generic
        }
    }

    pub fn default_style(&self) -> DocStyle {
        match self {
            DocLanguage::Rust => DocStyle::Rustdoc,
            DocLanguage::TypeScript | DocLanguage::JavaScript => DocStyle::Jsdoc,
            DocLanguage::Python => DocStyle::Google,
            DocLanguage::Go => DocStyle::Godoc,
            DocLanguage::Cpp | DocLanguage::C => DocStyle::Doxygen,
            DocLanguage::Java | DocLanguage::Kotlin | DocLanguage::CSharp => DocStyle::Javadoc,
            DocLanguage::Swift => DocStyle::Rustdoc,
            DocLanguage::Php => DocStyle::Jsdoc,
            DocLanguage::Ruby => DocStyle::Jsdoc,
            _ => DocStyle::Rustdoc,
        }
    }
}

// ---------------------------------------------------------------------------
// Data Models
// ---------------------------------------------------------------------------

/// Extracted function/method parameter metadata.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ParamInfo {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub type_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_value: Option<String>,
    pub is_self: bool,
    pub is_variadic: bool,
    pub is_optional: bool,
    pub description: String,
}

/// Extracted return type metadata.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReturnInfo {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub type_name: Option<String>,
    pub is_result_or_error: bool,
    pub is_option_or_nullable: bool,
    pub description: String,
}

/// Comprehensive extracted code signature metadata.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SignatureInfo {
    pub name: String,
    pub kind: SymbolKind,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub visibility: Option<String>,
    pub is_async: bool,
    pub is_unsafe: bool,
    pub is_const: bool,
    pub is_static: bool,
    pub is_generator: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub generics: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub container: Option<String>,
    pub params: Vec<ParamInfo>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub return_type: Option<ReturnInfo>,
    pub throws: Vec<String>,
    pub raw_signature: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub existing_doc: Option<String>,
    pub line_start: usize,
    pub sig_line: usize,
    pub sig_end_line: usize,
    pub indent: String,
}

impl SignatureInfo {
    pub fn is_documented(&self) -> bool {
        self.existing_doc
            .as_ref()
            .map(|d| !d.trim().is_empty())
            .unwrap_or(false)
    }

    pub fn is_public(&self) -> bool {
        if let Some(vis) = &self.visibility {
            let v = vis.to_lowercase();
            v == "pub" || v.starts_with("pub(") || v == "public" || v == "export" || v == "open"
        } else {
            // In Go, uppercase initial letter is public
            if let Some(first) = self.name.chars().next() {
                if first.is_ascii_uppercase() {
                    return true;
                }
            }
            // In Python, non-underscore is considered public API
            !self.name.starts_with('_')
        }
    }
}

/// Status of a symbol in documentation coverage analysis.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SymbolDocStatus {
    pub name: String,
    pub kind: String,
    pub line: usize,
    pub is_documented: bool,
    pub is_public: bool,
    pub existing_doc_lines: usize,
    pub missing_param_docs: Vec<String>,
    pub missing_return_doc: bool,
}

/// Documentation coverage report for a file or workspace.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DocCoverageReport {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target: Option<String>,
    pub language: String,
    pub total_symbols: usize,
    pub documented_symbols: usize,
    pub undocumented_symbols: usize,
    pub public_total: usize,
    pub public_documented: usize,
    pub coverage_percentage: f64,
    pub public_coverage_percentage: f64,
    pub symbols: Vec<SymbolDocStatus>,
    pub summary: String,
}

/// A drafted documentation comment for a symbol.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DraftResult {
    pub symbol_name: String,
    pub kind: String,
    pub line: usize,
    pub doc_comment: String,
    pub combined_preview: String,
}

/// General tool output payload.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DocgenOutput {
    pub action: String,
    pub language: String,
    pub style: String,
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub drafts: Vec<DraftResult>,
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub signatures: Vec<SignatureInfo>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub coverage: Option<DocCoverageReport>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modified_code: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub markdown_doc: Option<String>,
    pub total_symbols: usize,
}

// ---------------------------------------------------------------------------
// Signature Extraction Engine
// ---------------------------------------------------------------------------

pub struct SignatureExtractor;

impl SignatureExtractor {
    /// Extract all function, class, struct, trait, and enum signatures from code.
    pub fn extract(code: &str, lang: DocLanguage) -> Vec<SignatureInfo> {
        let lines: Vec<&str> = code.lines().collect();
        let mut results = Vec::new();
        let mut idx = 0;

        while idx < lines.len() {
            let line = lines[idx];
            let trimmed = line.trim();

            // Skip blank lines
            if trimmed.is_empty() {
                idx += 1;
                continue;
            }

            // Check if this line starts a doc comment or declaration
            let (doc_comment, doc_start_line, next_idx) = Self::collect_leading_doc(&lines, idx, lang);
            if next_idx >= lines.len() {
                break;
            }

            let decl_line = lines[next_idx];
            let indent = Self::get_indent(decl_line);

            // Try to extract signature starting at next_idx
            if let Some((sig_info, end_idx)) = Self::parse_declaration_at(&lines, next_idx, lang, indent, doc_comment, doc_start_line) {
                results.push(sig_info);
                idx = end_idx + 1;
            } else {
                idx = next_idx + 1;
            }
        }

        results
    }

    fn get_indent(line: &str) -> String {
        let mut indent = String::new();
        for ch in line.chars() {
            if ch == ' ' || ch == '\t' {
                indent.push(ch);
            } else {
                break;
            }
        }
        indent
    }

    fn collect_leading_doc(lines: &[&str], start: usize, lang: DocLanguage) -> (Option<String>, usize, usize) {
        let mut doc_lines = Vec::new();
        let mut curr = start;
        let doc_start = curr + 1; // 1-based

        match lang {
            DocLanguage::Rust => {
                while curr < lines.len() {
                    let trimmed = lines[curr].trim();
                    if trimmed.starts_with("///") || trimmed.starts_with("//!") {
                        let content = trimmed.strip_prefix("///").or_else(|| trimmed.strip_prefix("//!")).unwrap_or("");
                        let content = content.strip_prefix(' ').unwrap_or(content);
                        doc_lines.push(content);
                        curr += 1;
                    } else if trimmed.starts_with("/**") {
                        // Multi-line block doc comment
                        let mut block_doc = Vec::new();
                        let first = trimmed.strip_prefix("/**").unwrap_or("").trim_end_matches("*/").trim();
                        if !first.is_empty() {
                            block_doc.push(first);
                        }
                        if trimmed.ends_with("*/") && trimmed.len() > 4 {
                            curr += 1;
                            doc_lines.extend(block_doc);
                            continue;
                        }
                        curr += 1;
                        while curr < lines.len() {
                            let b_trimmed = lines[curr].trim();
                            if b_trimmed.ends_with("*/") {
                                let inner = b_trimmed.strip_suffix("*/").unwrap_or("").trim_start_matches('*').trim();
                                if !inner.is_empty() {
                                    block_doc.push(inner);
                                }
                                curr += 1;
                                break;
                            } else {
                                let inner = b_trimmed.trim_start_matches('*').trim();
                                block_doc.push(inner);
                                curr += 1;
                            }
                        }
                        doc_lines.extend(block_doc);
                    } else {
                        break;
                    }
                }
            }
            DocLanguage::TypeScript | DocLanguage::JavaScript | DocLanguage::Java | DocLanguage::Cpp | DocLanguage::C | DocLanguage::Php => {
                while curr < lines.len() {
                    let trimmed = lines[curr].trim();
                    if trimmed.starts_with("/**") {
                        let mut block_doc = Vec::new();
                        let first = trimmed.strip_prefix("/**").unwrap_or("").trim_end_matches("*/").trim();
                        if !first.is_empty() {
                            block_doc.push(first);
                        }
                        if trimmed.ends_with("*/") && trimmed.len() > 4 {
                            curr += 1;
                            doc_lines.extend(block_doc);
                            continue;
                        }
                        curr += 1;
                        while curr < lines.len() {
                            let b_trimmed = lines[curr].trim();
                            if b_trimmed.ends_with("*/") {
                                let inner = b_trimmed.strip_suffix("*/").unwrap_or("").trim_start_matches('*').trim();
                                if !inner.is_empty() {
                                    block_doc.push(inner);
                                }
                                curr += 1;
                                break;
                            } else {
                                let inner = b_trimmed.trim_start_matches('*').trim();
                                block_doc.push(inner);
                                curr += 1;
                            }
                        }
                        doc_lines.extend(block_doc);
                    } else if trimmed.starts_with("///") {
                        let content = trimmed.strip_prefix("///").unwrap_or("").trim();
                        doc_lines.push(content);
                        curr += 1;
                    } else {
                        break;
                    }
                }
            }
            DocLanguage::Python => {
                while curr < lines.len() {
                    let trimmed = lines[curr].trim();
                    if trimmed.starts_with('#') {
                        let content = trimmed.strip_prefix('#').unwrap_or("").trim();
                        doc_lines.push(content);
                        curr += 1;
                    } else {
                        break;
                    }
                }
            }
            DocLanguage::Go => {
                while curr < lines.len() {
                    let trimmed = lines[curr].trim();
                    if trimmed.starts_with("//") {
                        let content = trimmed.strip_prefix("//").unwrap_or("").trim();
                        doc_lines.push(content);
                        curr += 1;
                    } else {
                        break;
                    }
                }
            }
            _ => {
                while curr < lines.len() {
                    let trimmed = lines[curr].trim();
                    if trimmed.starts_with("//") || trimmed.starts_with('#') {
                        let content = trimmed.trim_start_matches(['/', '#']).trim();
                        doc_lines.push(content);
                        curr += 1;
                    } else {
                        break;
                    }
                }
            }
        }

        if doc_lines.is_empty() {
            (None, start + 1, curr)
        } else {
            (Some(doc_lines.join("\n")), doc_start, curr)
        }
    }

    fn parse_declaration_at(
        lines: &[&str],
        start_idx: usize,
        lang: DocLanguage,
        indent: String,
        existing_doc: Option<String>,
        doc_start_line: usize,
    ) -> Option<(SignatureInfo, usize)> {
        match lang {
            DocLanguage::Rust => Self::parse_rust_declaration(lines, start_idx, indent, existing_doc, doc_start_line),
            DocLanguage::TypeScript | DocLanguage::JavaScript => {
                Self::parse_ts_js_declaration(lines, start_idx, lang, indent, existing_doc, doc_start_line)
            }
            DocLanguage::Python => Self::parse_python_declaration(lines, start_idx, indent, existing_doc, doc_start_line),
            DocLanguage::Go => Self::parse_go_declaration(lines, start_idx, indent, existing_doc, doc_start_line),
            DocLanguage::Cpp | DocLanguage::C => Self::parse_cpp_declaration(lines, start_idx, indent, existing_doc, doc_start_line),
            DocLanguage::Java | DocLanguage::Kotlin | DocLanguage::CSharp => {
                Self::parse_java_csharp_declaration(lines, start_idx, lang, indent, existing_doc, doc_start_line)
            }
            _ => Self::parse_generic_declaration(lines, start_idx, indent, existing_doc, doc_start_line),
        }
    }

    // -----------------------------------------------------------------------
    // Rust Parsing
    // -----------------------------------------------------------------------

    fn parse_rust_declaration(
        lines: &[&str],
        start_idx: usize,
        indent: String,
        existing_doc: Option<String>,
        doc_start_line: usize,
    ) -> Option<(SignatureInfo, usize)> {
        let first_line = lines[start_idx].trim();

        // Check for struct / enum / trait / type / const / macro
        static RUST_STRUCT_RE: LazyLock<Regex> = LazyLock::new(|| {
            Regex::new(r"^(pub(?:\s*\([^)]+\))?\s+)?struct\s+([A-Za-z0-9_]+)(?:<([^>]+)>)?").unwrap()
        });
        static RUST_ENUM_RE: LazyLock<Regex> = LazyLock::new(|| {
            Regex::new(r"^(pub(?:\s*\([^)]+\))?\s+)?enum\s+([A-Za-z0-9_]+)(?:<([^>]+)>)?").unwrap()
        });
        static RUST_TRAIT_RE: LazyLock<Regex> = LazyLock::new(|| {
            Regex::new(r"^(pub(?:\s*\([^)]+\))?\s+)?(?:unsafe\s+)?trait\s+([A-Za-z0-9_]+)(?:<([^>]+)>)?").unwrap()
        });
        static RUST_TYPE_RE: LazyLock<Regex> = LazyLock::new(|| {
            Regex::new(r"^(pub(?:\s*\([^)]+\))?\s+)?type\s+([A-Za-z0-9_]+)(?:<([^>]+)>)?\s*=").unwrap()
        });
        static RUST_CONST_RE: LazyLock<Regex> = LazyLock::new(|| {
            Regex::new(r"^(pub(?:\s*\([^)]+\))?\s+)?const\s+([A-Za-z0-9_]+)\s*:\s*([^=;]+)").unwrap()
        });
        static RUST_MACRO_RE: LazyLock<Regex> = LazyLock::new(|| {
            Regex::new(r"^macro_rules!\s+([A-Za-z0-9_]+)").unwrap()
        });

        // 1. Rust Struct
        if let Some(caps) = RUST_STRUCT_RE.captures(first_line) {
            let vis = caps.get(1).map(|m| m.as_str().trim().to_string());
            let name = caps.get(2).map(|m| m.as_str().to_string())?;
            let generics = caps.get(3).map(|m| format!("<{}>", m.as_str().trim()));
            return Some((
                SignatureInfo {
                    name,
                    kind: SymbolKind::Struct,
                    visibility: vis,
                    is_async: false,
                    is_unsafe: false,
                    is_const: false,
                    is_static: false,
                    is_generator: false,
                    generics,
                    container: None,
                    params: Vec::new(),
                    return_type: None,
                    throws: Vec::new(),
                    raw_signature: first_line.to_string(),
                    existing_doc,
                    line_start: doc_start_line,
                    sig_line: start_idx + 1,
                    sig_end_line: start_idx + 1,
                    indent,
                },
                start_idx,
            ));
        }

        // 2. Rust Enum
        if let Some(caps) = RUST_ENUM_RE.captures(first_line) {
            let vis = caps.get(1).map(|m| m.as_str().trim().to_string());
            let name = caps.get(2).map(|m| m.as_str().to_string())?;
            let generics = caps.get(3).map(|m| format!("<{}>", m.as_str().trim()));
            return Some((
                SignatureInfo {
                    name,
                    kind: SymbolKind::Enum,
                    visibility: vis,
                    is_async: false,
                    is_unsafe: false,
                    is_const: false,
                    is_static: false,
                    is_generator: false,
                    generics,
                    container: None,
                    params: Vec::new(),
                    return_type: None,
                    throws: Vec::new(),
                    raw_signature: first_line.to_string(),
                    existing_doc,
                    line_start: doc_start_line,
                    sig_line: start_idx + 1,
                    sig_end_line: start_idx + 1,
                    indent,
                },
                start_idx,
            ));
        }

        // 3. Rust Trait
        if let Some(caps) = RUST_TRAIT_RE.captures(first_line) {
            let vis = caps.get(1).map(|m| m.as_str().trim().to_string());
            let name = caps.get(2).map(|m| m.as_str().to_string())?;
            let generics = caps.get(3).map(|m| format!("<{}>", m.as_str().trim()));
            return Some((
                SignatureInfo {
                    name,
                    kind: SymbolKind::Trait,
                    visibility: vis,
                    is_async: false,
                    is_unsafe: first_line.contains("unsafe trait"),
                    is_const: false,
                    is_static: false,
                    is_generator: false,
                    generics,
                    container: None,
                    params: Vec::new(),
                    return_type: None,
                    throws: Vec::new(),
                    raw_signature: first_line.to_string(),
                    existing_doc,
                    line_start: doc_start_line,
                    sig_line: start_idx + 1,
                    sig_end_line: start_idx + 1,
                    indent,
                },
                start_idx,
            ));
        }

        // 4. Rust Type Alias
        if let Some(caps) = RUST_TYPE_RE.captures(first_line) {
            let vis = caps.get(1).map(|m| m.as_str().trim().to_string());
            let name = caps.get(2).map(|m| m.as_str().to_string())?;
            let generics = caps.get(3).map(|m| format!("<{}>", m.as_str().trim()));
            return Some((
                SignatureInfo {
                    name,
                    kind: SymbolKind::TypeAlias,
                    visibility: vis,
                    is_async: false,
                    is_unsafe: false,
                    is_const: false,
                    is_static: false,
                    is_generator: false,
                    generics,
                    container: None,
                    params: Vec::new(),
                    return_type: None,
                    throws: Vec::new(),
                    raw_signature: first_line.to_string(),
                    existing_doc,
                    line_start: doc_start_line,
                    sig_line: start_idx + 1,
                    sig_end_line: start_idx + 1,
                    indent,
                },
                start_idx,
            ));
        }

        // 5. Rust Const
        if let Some(caps) = RUST_CONST_RE.captures(first_line) {
            let vis = caps.get(1).map(|m| m.as_str().trim().to_string());
            let name = caps.get(2).map(|m| m.as_str().to_string())?;
            let type_name = caps.get(3).map(|m| m.as_str().trim().to_string());
            return Some((
                SignatureInfo {
                    name,
                    kind: SymbolKind::Constant,
                    visibility: vis,
                    is_async: false,
                    is_unsafe: false,
                    is_const: true,
                    is_static: false,
                    is_generator: false,
                    generics: None,
                    container: None,
                    params: Vec::new(),
                    return_type: type_name.map(|t| ReturnInfo {
                        type_name: Some(t),
                        is_result_or_error: false,
                        is_option_or_nullable: false,
                        description: String::new(),
                    }),
                    throws: Vec::new(),
                    raw_signature: first_line.to_string(),
                    existing_doc,
                    line_start: doc_start_line,
                    sig_line: start_idx + 1,
                    sig_end_line: start_idx + 1,
                    indent,
                },
                start_idx,
            ));
        }

        // 6. Rust Macro
        if let Some(caps) = RUST_MACRO_RE.captures(first_line) {
            let name = caps.get(1).map(|m| m.as_str().to_string())?;
            return Some((
                SignatureInfo {
                    name,
                    kind: SymbolKind::Macro,
                    visibility: Some("pub".to_string()),
                    is_async: false,
                    is_unsafe: false,
                    is_const: false,
                    is_static: false,
                    is_generator: false,
                    generics: None,
                    container: None,
                    params: Vec::new(),
                    return_type: None,
                    throws: Vec::new(),
                    raw_signature: first_line.to_string(),
                    existing_doc,
                    line_start: doc_start_line,
                    sig_line: start_idx + 1,
                    sig_end_line: start_idx + 1,
                    indent,
                },
                start_idx,
            ));
        }

        // 7. Rust Function / Method
        // Multi-line accumulator for fn signatures
        static RUST_FN_START_RE: LazyLock<Regex> = LazyLock::new(|| {
            Regex::new(r"^(pub(?:\s*\([^)]+\))?\s+)?(?:(async|const|unsafe|extern(?:\s+[\x22][^\x22]+[\x22])?)\s+)*(?:fn)\s+([A-Za-z0-9_]+)(?:<([^>]+)>)?\s*\(").unwrap()
        });

        if let Some(caps) = RUST_FN_START_RE.captures(first_line) {
            let vis = caps.get(1).map(|m| m.as_str().trim().to_string());
            let name = caps.get(3).map(|m| m.as_str().to_string())?;
            let generics = caps.get(4).map(|m| format!("<{}>", m.as_str().trim()));

            // Accumulate multi-line signature until `{` or `;` or balanced parentheses and return type
            let mut sig_lines = Vec::new();
            let mut curr = start_idx;
            let mut paren_count = 0;
            let mut found_open = false;
            let mut end_idx = start_idx;

            while curr < lines.len() {
                let l = lines[curr];
                sig_lines.push(l.trim());
                for ch in l.chars() {
                    if ch == '(' {
                        paren_count += 1;
                        found_open = true;
                    } else if ch == ')' {
                        paren_count -= 1;
                    }
                }
                if found_open && paren_count <= 0 && (l.contains('{') || l.contains(';') || l.contains("where") || curr > start_idx + 10) {
                    end_idx = curr;
                    break;
                }
                curr += 1;
            }

            let full_sig = sig_lines.join(" ");
            let is_async = full_sig.contains("async ");
            let is_unsafe = full_sig.contains("unsafe ");
            let is_const = full_sig.contains("const fn");

            // Extract parameter list and return type
            let (params, return_type) = Self::parse_rust_fn_params_and_return(&full_sig, &name);

            let mut throws = Vec::new();
            if let Some(ret) = &return_type {
                if ret.is_result_or_error {
                    throws.push("Returns an error if the operation fails.".to_string());
                }
            }

            return Some((
                SignatureInfo {
                    name,
                    kind: SymbolKind::Function,
                    visibility: vis,
                    is_async,
                    is_unsafe,
                    is_const,
                    is_static: false,
                    is_generator: false,
                    generics,
                    container: None,
                    params,
                    return_type,
                    throws,
                    raw_signature: full_sig.trim_end_matches(['{', ';', ' ']).trim().to_string(),
                    existing_doc,
                    line_start: doc_start_line,
                    sig_line: start_idx + 1,
                    sig_end_line: end_idx + 1,
                    indent,
                },
                end_idx,
            ));
        }

        None
    }

    fn parse_rust_fn_params_and_return(sig: &str, fn_name: &str) -> (Vec<ParamInfo>, Option<ReturnInfo>) {
        let mut params = Vec::new();
        let mut return_type = None;

        if let Some(open_paren) = sig.find('(') {
            if let Some(close_paren) = sig.rfind(')') {
                let param_str = &sig[open_paren + 1..close_paren].trim();
                if !param_str.is_empty() {
                    let raw_params = Self::split_params_by_comma(param_str);
                    for p in raw_params {
                        let p = p.trim();
                        if p.is_empty() {
                            continue;
                        }

                        if p == "self" || p == "&self" || p == "&mut self" || p == "mut self" {
                            params.push(ParamInfo {
                                name: p.to_string(),
                                type_name: Some("Self".to_string()),
                                default_value: None,
                                is_self: true,
                                is_variadic: false,
                                is_optional: false,
                                description: "Self reference".to_string(),
                            });
                            continue;
                        }

                        // name: Type
                        let parts: Vec<&str> = p.splitn(2, ':').collect();
                        if parts.len() == 2 {
                            let raw_name = parts[0].trim().trim_start_matches("mut ").trim();
                            let raw_type = parts[1].trim();
                            let is_opt = raw_type.starts_with("Option<");
                            let desc = DocDescriptionGenerator::generate_param_desc(raw_name, Some(raw_type));

                            params.push(ParamInfo {
                                name: raw_name.to_string(),
                                type_name: Some(raw_type.to_string()),
                                default_value: None,
                                is_self: false,
                                is_variadic: false,
                                is_optional: is_opt,
                                description: desc,
                            });
                        } else {
                            params.push(ParamInfo {
                                name: p.to_string(),
                                type_name: None,
                                default_value: None,
                                is_self: false,
                                is_variadic: false,
                                is_optional: false,
                                description: DocDescriptionGenerator::generate_param_desc(p, None),
                            });
                        }
                    }
                }

                // Parse return type after `->`
                let rest = &sig[close_paren + 1..];
                if let Some(arrow_idx) = rest.find("->") {
                    let ret_part = rest[arrow_idx + 2..]
                        .split(['{', ';', 'w'])
                        .next()
                        .unwrap_or("")
                        .trim();
                    if !ret_part.is_empty() {
                        let is_result = ret_part.starts_with("Result<") || ret_part.contains("Result<") || ret_part == "Result";
                        let is_option = ret_part.starts_with("Option<") || ret_part == "Option";
                        let desc = DocDescriptionGenerator::generate_return_desc(ret_part, fn_name);

                        return_type = Some(ReturnInfo {
                            type_name: Some(ret_part.to_string()),
                            is_result_or_error: is_result,
                            is_option_or_nullable: is_option,
                            description: desc,
                        });
                    }
                }
            }
        }

        (params, return_type)
    }

    // -----------------------------------------------------------------------
    // TypeScript / JavaScript Parsing
    // -----------------------------------------------------------------------

    fn parse_ts_js_declaration(
        lines: &[&str],
        start_idx: usize,
        _lang: DocLanguage,
        indent: String,
        existing_doc: Option<String>,
        doc_start_line: usize,
    ) -> Option<(SignatureInfo, usize)> {
        let first_line = lines[start_idx].trim();

        // 1. Class / Interface / Type / Enum
        static TS_CLASS_RE: LazyLock<Regex> = LazyLock::new(|| {
            Regex::new(r"^(export(?:\s+default)?\s+)?(?:abstract\s+)?class\s+([A-Za-z0-9_$]+)(?:<([^>]+)>)?").unwrap()
        });
        static TS_INTERFACE_RE: LazyLock<Regex> = LazyLock::new(|| {
            Regex::new(r"^(export\s+)?interface\s+([A-Za-z0-9_$]+)(?:<([^>]+)>)?").unwrap()
        });
        static TS_TYPE_RE: LazyLock<Regex> = LazyLock::new(|| {
            Regex::new(r"^(export\s+)?type\s+([A-Za-z0-9_$]+)(?:<([^>]+)>)?\s*=").unwrap()
        });
        static TS_ENUM_RE: LazyLock<Regex> = LazyLock::new(|| {
            Regex::new(r"^(export(?:\s+const)?\s+)?enum\s+([A-Za-z0-9_$]+)").unwrap()
        });

        if let Some(caps) = TS_CLASS_RE.captures(first_line) {
            let vis = caps.get(1).map(|m| m.as_str().trim().to_string());
            let name = caps.get(2).map(|m| m.as_str().to_string())?;
            let generics = caps.get(3).map(|m| format!("<{}>", m.as_str().trim()));
            return Some((
                SignatureInfo {
                    name,
                    kind: SymbolKind::Class,
                    visibility: vis,
                    is_async: false,
                    is_unsafe: false,
                    is_const: false,
                    is_static: false,
                    is_generator: false,
                    generics,
                    container: None,
                    params: Vec::new(),
                    return_type: None,
                    throws: Vec::new(),
                    raw_signature: first_line.to_string(),
                    existing_doc,
                    line_start: doc_start_line,
                    sig_line: start_idx + 1,
                    sig_end_line: start_idx + 1,
                    indent,
                },
                start_idx,
            ));
        }

        if let Some(caps) = TS_INTERFACE_RE.captures(first_line) {
            let vis = caps.get(1).map(|m| m.as_str().trim().to_string());
            let name = caps.get(2).map(|m| m.as_str().to_string())?;
            let generics = caps.get(3).map(|m| format!("<{}>", m.as_str().trim()));
            return Some((
                SignatureInfo {
                    name,
                    kind: SymbolKind::Interface,
                    visibility: vis,
                    is_async: false,
                    is_unsafe: false,
                    is_const: false,
                    is_static: false,
                    is_generator: false,
                    generics,
                    container: None,
                    params: Vec::new(),
                    return_type: None,
                    throws: Vec::new(),
                    raw_signature: first_line.to_string(),
                    existing_doc,
                    line_start: doc_start_line,
                    sig_line: start_idx + 1,
                    sig_end_line: start_idx + 1,
                    indent,
                },
                start_idx,
            ));
        }

        if let Some(caps) = TS_TYPE_RE.captures(first_line) {
            let vis = caps.get(1).map(|m| m.as_str().trim().to_string());
            let name = caps.get(2).map(|m| m.as_str().to_string())?;
            let generics = caps.get(3).map(|m| format!("<{}>", m.as_str().trim()));
            return Some((
                SignatureInfo {
                    name,
                    kind: SymbolKind::TypeAlias,
                    visibility: vis,
                    is_async: false,
                    is_unsafe: false,
                    is_const: false,
                    is_static: false,
                    is_generator: false,
                    generics,
                    container: None,
                    params: Vec::new(),
                    return_type: None,
                    throws: Vec::new(),
                    raw_signature: first_line.to_string(),
                    existing_doc,
                    line_start: doc_start_line,
                    sig_line: start_idx + 1,
                    sig_end_line: start_idx + 1,
                    indent,
                },
                start_idx,
            ));
        }

        if let Some(caps) = TS_ENUM_RE.captures(first_line) {
            let vis = caps.get(1).map(|m| m.as_str().trim().to_string());
            let name = caps.get(2).map(|m| m.as_str().to_string())?;
            return Some((
                SignatureInfo {
                    name,
                    kind: SymbolKind::Enum,
                    visibility: vis,
                    is_async: false,
                    is_unsafe: false,
                    is_const: false,
                    is_static: false,
                    is_generator: false,
                    generics: None,
                    container: None,
                    params: Vec::new(),
                    return_type: None,
                    throws: Vec::new(),
                    raw_signature: first_line.to_string(),
                    existing_doc,
                    line_start: doc_start_line,
                    sig_line: start_idx + 1,
                    sig_end_line: start_idx + 1,
                    indent,
                },
                start_idx,
            ));
        }

        // 2. Functions & Arrow Functions & Methods
        static TS_FN_RE: LazyLock<Regex> = LazyLock::new(|| {
            Regex::new(r"^(export(?:\s+default)?\s+)?(?:(async)\s+)?function(?:\s*\*|\s+)?\s*([A-Za-z0-9_$]+)?(?:<([^>]+)>)?\s*\(").unwrap()
        });
        static TS_ARROW_RE: LazyLock<Regex> = LazyLock::new(|| {
            Regex::new(r"^(export\s+)?(?:const|let|var)\s+([A-Za-z0-9_$]+)\s*=\s*(?:(async)\s*)?(?:<([^>]+)>)?\s*\(").unwrap()
        });
        static TS_METHOD_RE: LazyLock<Regex> = LazyLock::new(|| {
            Regex::new(r"^(public|private|protected)?\s*(static\s+)?(async\s+)?([A-Za-z0-9_$]+)(?:<([^>]+)>)?\s*\(").unwrap()
        });

        let mut matched_fn_name = None;
        let mut matched_vis = None;
        let mut matched_async = false;
        let mut matched_generics = None;
        let mut matched_static = false;

        if let Some(caps) = TS_FN_RE.captures(first_line) {
            matched_vis = caps.get(1).map(|m| m.as_str().trim().to_string());
            matched_async = caps.get(2).is_some() || first_line.contains("async ");
            matched_fn_name = caps.get(3).map(|m| m.as_str().to_string()).or_else(|| Some("anonymous".to_string()));
            matched_generics = caps.get(4).map(|m| format!("<{}>", m.as_str().trim()));
        } else if let Some(caps) = TS_ARROW_RE.captures(first_line) {
            matched_vis = caps.get(1).map(|m| m.as_str().trim().to_string());
            matched_fn_name = caps.get(2).map(|m| m.as_str().to_string());
            matched_async = caps.get(3).is_some() || first_line.contains("async ");
            matched_generics = caps.get(4).map(|m| format!("<{}>", m.as_str().trim()));
        } else if let Some(caps) = TS_METHOD_RE.captures(first_line) {
            let name_candidate = caps.get(4).map(|m| m.as_str().to_string())?;
            if name_candidate != "if" && name_candidate != "for" && name_candidate != "while" && name_candidate != "switch" && name_candidate != "catch" {
                matched_vis = caps.get(1).map(|m| m.as_str().trim().to_string());
                matched_static = caps.get(2).is_some();
                matched_async = caps.get(3).is_some();
                matched_fn_name = Some(name_candidate);
                matched_generics = caps.get(5).map(|m| format!("<{}>", m.as_str().trim()));
            }
        }

        if let Some(name) = matched_fn_name {
            // Collect multi-line signature
            let mut sig_lines = Vec::new();
            let mut curr = start_idx;
            let mut paren_count = 0;
            let mut found_open = false;
            let mut end_idx = start_idx;

            while curr < lines.len() {
                let l = lines[curr];
                sig_lines.push(l.trim());
                for ch in l.chars() {
                    if ch == '(' {
                        paren_count += 1;
                        found_open = true;
                    } else if ch == ')' {
                        paren_count -= 1;
                    }
                }
                if found_open && paren_count <= 0 && (l.contains('{') || l.contains("=>") || l.contains(';') || curr > start_idx + 10) {
                    end_idx = curr;
                    break;
                }
                curr += 1;
            }

            let full_sig = sig_lines.join(" ");
            let (params, return_type) = Self::parse_ts_fn_params_and_return(&full_sig, &name);

            return Some((
                SignatureInfo {
                    name,
                    kind: SymbolKind::Function,
                    visibility: matched_vis,
                    is_async: matched_async || full_sig.contains("Promise<"),
                    is_unsafe: false,
                    is_const: false,
                    is_static: matched_static,
                    is_generator: full_sig.contains("function*"),
                    generics: matched_generics,
                    container: None,
                    params,
                    return_type,
                    throws: Vec::new(),
                    raw_signature: full_sig.trim_end_matches(['{', ';', ' ']).trim().to_string(),
                    existing_doc,
                    line_start: doc_start_line,
                    sig_line: start_idx + 1,
                    sig_end_line: end_idx + 1,
                    indent,
                },
                end_idx,
            ));
        }

        None
    }

    fn parse_ts_fn_params_and_return(sig: &str, fn_name: &str) -> (Vec<ParamInfo>, Option<ReturnInfo>) {
        let mut params = Vec::new();
        let mut return_type = None;

        if let Some(open_paren) = sig.find('(') {
            if let Some(close_paren) = sig.rfind(')') {
                let param_str = &sig[open_paren + 1..close_paren].trim();
                if !param_str.is_empty() {
                    let raw_params = Self::split_params_by_comma(param_str);
                    for p in raw_params {
                        let p = p.trim();
                        if p.is_empty() {
                            continue;
                        }

                        let is_variadic = p.starts_with("...");
                        let clean_p = if is_variadic { p.strip_prefix("...").unwrap_or(p).trim() } else { p };

                        let (raw_name, raw_type, default_val, is_opt) = if let Some(colon_idx) = clean_p.find(':') {
                            let name_part = clean_p[..colon_idx].trim();
                            let is_optional = name_part.ends_with('?');
                            let clean_name = name_part.trim_end_matches('?').trim();
                            let rest = clean_p[colon_idx + 1..].trim();

                            let (type_part, def_part) = if let Some(eq_idx) = rest.find('=') {
                                (rest[..eq_idx].trim().to_string(), Some(rest[eq_idx + 1..].trim().to_string()))
                            } else {
                                (rest.to_string(), None)
                            };

                            (clean_name, Some(type_part), def_part, is_optional)
                        } else if let Some(eq_idx) = clean_p.find('=') {
                            let name_part = clean_p[..eq_idx].trim();
                            let def_part = clean_p[eq_idx + 1..].trim();
                            (name_part, None, Some(def_part.to_string()), true)
                        } else {
                            (clean_p, None, None, false)
                        };

                        let desc = DocDescriptionGenerator::generate_param_desc(raw_name, raw_type.as_deref());

                        params.push(ParamInfo {
                            name: raw_name.to_string(),
                            type_name: raw_type,
                            default_value: default_val,
                            is_self: raw_name == "this",
                            is_variadic,
                            is_optional: is_opt,
                            description: desc,
                        });
                    }
                }

                // Return type after `): Type` or `): Promise<Type>`
                let rest = &sig[close_paren + 1..];
                if let Some(colon_idx) = rest.find(':') {
                    let ret_part = rest[colon_idx + 1..]
                        .split(['{', '=', ';'])
                        .next()
                        .unwrap_or("")
                        .trim();
                    if !ret_part.is_empty() {
                        let is_promise = ret_part.starts_with("Promise<");
                        let desc = DocDescriptionGenerator::generate_return_desc(ret_part, fn_name);

                        return_type = Some(ReturnInfo {
                            type_name: Some(ret_part.to_string()),
                            is_result_or_error: is_promise,
                            is_option_or_nullable: ret_part.contains("null") || ret_part.contains("undefined"),
                            description: desc,
                        });
                    }
                }
            }
        }

        (params, return_type)
    }

    // -----------------------------------------------------------------------
    // Python Parsing
    // -----------------------------------------------------------------------

    fn parse_python_declaration(
        lines: &[&str],
        start_idx: usize,
        indent: String,
        existing_doc: Option<String>,
        doc_start_line: usize,
    ) -> Option<(SignatureInfo, usize)> {
        let first_line = lines[start_idx].trim();

        // 1. Python Class
        static PY_CLASS_RE: LazyLock<Regex> = LazyLock::new(|| {
            Regex::new(r"^class\s+([A-Za-z0-9_]+)(?:\(([^)]*)\))?\s*:").unwrap()
        });

        if let Some(caps) = PY_CLASS_RE.captures(first_line) {
            let name = caps.get(1).map(|m| m.as_str().to_string())?;
            let base_classes = caps.get(2).map(|m| m.as_str().trim().to_string());
            return Some((
                SignatureInfo {
                    name,
                    kind: SymbolKind::Class,
                    visibility: None,
                    is_async: false,
                    is_unsafe: false,
                    is_const: false,
                    is_static: false,
                    is_generator: false,
                    generics: base_classes,
                    container: None,
                    params: Vec::new(),
                    return_type: None,
                    throws: Vec::new(),
                    raw_signature: first_line.to_string(),
                    existing_doc,
                    line_start: doc_start_line,
                    sig_line: start_idx + 1,
                    sig_end_line: start_idx + 1,
                    indent,
                },
                start_idx,
            ));
        }

        // 2. Python Def / Async Def
        static PY_DEF_RE: LazyLock<Regex> = LazyLock::new(|| {
            Regex::new(r"^(async\s+)?def\s+([A-Za-z0-9_]+)\s*\(").unwrap()
        });

        if let Some(caps) = PY_DEF_RE.captures(first_line) {
            let is_async = caps.get(1).is_some();
            let name = caps.get(2).map(|m| m.as_str().to_string())?;

            // Accumulate multi-line python def signature until `:`
            let mut sig_lines = Vec::new();
            let mut curr = start_idx;
            let mut end_idx = start_idx;

            while curr < lines.len() {
                let l = lines[curr];
                sig_lines.push(l.trim());
                if l.trim_end().ends_with(':') || curr > start_idx + 10 {
                    end_idx = curr;
                    break;
                }
                curr += 1;
            }

            let full_sig = sig_lines.join(" ");
            let (params, return_type) = Self::parse_python_params_and_return(&full_sig, &name);

            // Check if there is an existing docstring right below the signature
            let mut doc = existing_doc;
            if doc.is_none() && end_idx + 1 < lines.len() {
                let next_line = lines[end_idx + 1].trim();
                if next_line.starts_with("\"\"\"") || next_line.starts_with("'''") {
                    let quote = if next_line.starts_with("\"\"\"") { "\"\"\"" } else { "'''" };
                    let mut docstring_lines = Vec::new();
                    let first = next_line.strip_prefix(quote).unwrap_or("");
                    if first.ends_with(quote) && first.len() >= quote.len() {
                        let inner = first.strip_suffix(quote).unwrap_or("");
                        doc = Some(inner.trim().to_string());
                    } else {
                        docstring_lines.push(first);
                        let mut d_curr = end_idx + 2;
                        while d_curr < lines.len() {
                            let dl = lines[d_curr];
                            if dl.contains(quote) {
                                let inner = dl.split(quote).next().unwrap_or("");
                                docstring_lines.push(inner);
                                break;
                            } else {
                                docstring_lines.push(dl);
                            }
                            d_curr += 1;
                        }
                        doc = Some(docstring_lines.join("\n"));
                    }
                }
            }

            return Some((
                SignatureInfo {
                    name,
                    kind: SymbolKind::Function,
                    visibility: None,
                    is_async,
                    is_unsafe: false,
                    is_const: false,
                    is_static: false,
                    is_generator: full_sig.contains("yield"),
                    generics: None,
                    container: None,
                    params,
                    return_type,
                    throws: Vec::new(),
                    raw_signature: full_sig.trim_end_matches(':').trim().to_string(),
                    existing_doc: doc,
                    line_start: doc_start_line,
                    sig_line: start_idx + 1,
                    sig_end_line: end_idx + 1,
                    indent,
                },
                end_idx,
            ));
        }

        None
    }

    fn parse_python_params_and_return(sig: &str, fn_name: &str) -> (Vec<ParamInfo>, Option<ReturnInfo>) {
        let mut params = Vec::new();
        let mut return_type = None;

        if let Some(open_paren) = sig.find('(') {
            if let Some(close_paren) = sig.rfind(')') {
                let param_str = &sig[open_paren + 1..close_paren].trim();
                if !param_str.is_empty() {
                    let raw_params = Self::split_params_by_comma(param_str);
                    for p in raw_params {
                        let p = p.trim();
                        if p.is_empty() || p == "/" || p == "*" {
                            continue;
                        }

                        let is_variadic = p.starts_with('*');
                        let is_self = p == "self" || p == "cls";

                        let (raw_name, raw_type, default_val) = if let Some(colon_idx) = p.find(':') {
                            let name_part = p[..colon_idx].trim();
                            let rest = p[colon_idx + 1..].trim();
                            let (type_part, def_part) = if let Some(eq_idx) = rest.find('=') {
                                (rest[..eq_idx].trim().to_string(), Some(rest[eq_idx + 1..].trim().to_string()))
                            } else {
                                (rest.to_string(), None)
                            };
                            (name_part, Some(type_part), def_part)
                        } else if let Some(eq_idx) = p.find('=') {
                            let name_part = p[..eq_idx].trim();
                            let def_part = p[eq_idx + 1..].trim();
                            (name_part, None, Some(def_part.to_string()))
                        } else {
                            (p, None, None)
                        };

                        let clean_name = raw_name.trim_start_matches('*');
                        let desc = if is_self {
                            "Self instance reference.".to_string()
                        } else {
                            DocDescriptionGenerator::generate_param_desc(clean_name, raw_type.as_deref())
                        };

                        params.push(ParamInfo {
                            name: raw_name.to_string(),
                            type_name: raw_type,
                            default_value: default_val,
                            is_self,
                            is_variadic,
                            is_optional: p.contains('=') || p.contains("Optional[") || p.contains("None"),
                            description: desc,
                        });
                    }
                }

                // Return type after `->`
                let rest = &sig[close_paren + 1..];
                if let Some(arrow_idx) = rest.find("->") {
                    let ret_part = rest[arrow_idx + 2..].trim().trim_end_matches(':').trim();
                    if !ret_part.is_empty() {
                        let is_opt = ret_part.starts_with("Optional[") || ret_part.contains("None");
                        let desc = DocDescriptionGenerator::generate_return_desc(ret_part, fn_name);

                        return_type = Some(ReturnInfo {
                            type_name: Some(ret_part.to_string()),
                            is_result_or_error: false,
                            is_option_or_nullable: is_opt,
                            description: desc,
                        });
                    }
                }
            }
        }

        (params, return_type)
    }

    // -----------------------------------------------------------------------
    // Go Parsing
    // -----------------------------------------------------------------------

    fn parse_go_declaration(
        lines: &[&str],
        start_idx: usize,
        indent: String,
        existing_doc: Option<String>,
        doc_start_line: usize,
    ) -> Option<(SignatureInfo, usize)> {
        let first_line = lines[start_idx].trim();

        // 1. Go Struct / Interface / Type
        static GO_TYPE_RE: LazyLock<Regex> = LazyLock::new(|| {
            Regex::new(r"^type\s+([A-Za-z0-9_]+)\s+(struct|interface)").unwrap()
        });

        if let Some(caps) = GO_TYPE_RE.captures(first_line) {
            let name = caps.get(1).map(|m| m.as_str().to_string())?;
            let kind_str = caps.get(2).map(|m| m.as_str()).unwrap_or("struct");
            let kind = if kind_str == "interface" { SymbolKind::Interface } else { SymbolKind::Struct };
            return Some((
                SignatureInfo {
                    name,
                    kind,
                    visibility: None,
                    is_async: false,
                    is_unsafe: false,
                    is_const: false,
                    is_static: false,
                    is_generator: false,
                    generics: None,
                    container: None,
                    params: Vec::new(),
                    return_type: None,
                    throws: Vec::new(),
                    raw_signature: first_line.to_string(),
                    existing_doc,
                    line_start: doc_start_line,
                    sig_line: start_idx + 1,
                    sig_end_line: start_idx + 1,
                    indent,
                },
                start_idx,
            ));
        }

        // 2. Go Function / Method
        static GO_FN_RE: LazyLock<Regex> = LazyLock::new(|| {
            Regex::new(r"^func\s+(?:\(([^)]+)\)\s+)?([A-Za-z0-9_]+)\s*\(").unwrap()
        });

        if let Some(caps) = GO_FN_RE.captures(first_line) {
            let receiver = caps.get(1).map(|m| m.as_str().trim().to_string());
            let name = caps.get(2).map(|m| m.as_str().to_string())?;

            let mut sig_lines = Vec::new();
            let mut curr = start_idx;
            let mut end_idx = start_idx;

            while curr < lines.len() {
                let l = lines[curr];
                sig_lines.push(l.trim());
                if l.contains('{') || curr > start_idx + 8 {
                    end_idx = curr;
                    break;
                }
                curr += 1;
            }

            let full_sig = sig_lines.join(" ");
            let (params, return_type) = Self::parse_go_params_and_return(&full_sig, &name);

            return Some((
                SignatureInfo {
                    name,
                    kind: SymbolKind::Function,
                    visibility: None,
                    is_async: false,
                    is_unsafe: false,
                    is_const: false,
                    is_static: false,
                    is_generator: false,
                    generics: None,
                    container: receiver,
                    params,
                    return_type,
                    throws: Vec::new(),
                    raw_signature: full_sig.trim_end_matches(['{', ' ']).trim().to_string(),
                    existing_doc,
                    line_start: doc_start_line,
                    sig_line: start_idx + 1,
                    sig_end_line: end_idx + 1,
                    indent,
                },
                end_idx,
            ));
        }

        None
    }

    fn parse_go_params_and_return(sig: &str, fn_name: &str) -> (Vec<ParamInfo>, Option<ReturnInfo>) {
        let mut params = Vec::new();
        let mut return_type = None;

        // Find the parameter parenthesis (skip receiver parenthesis if present)
        let param_start_idx = if sig.starts_with("func (") {
            sig[5..].find('(').map(|i| i + 5)
        } else {
            sig.find('(')
        };

        if let Some(open_paren) = param_start_idx {
            if let Some(close_paren) = sig[open_paren..].find(')').map(|i| i + open_paren) {
                let param_str = &sig[open_paren + 1..close_paren].trim();
                if !param_str.is_empty() {
                    let raw_params = Self::split_params_by_comma(param_str);
                    for p in raw_params {
                        let p = p.trim();
                        if p.is_empty() {
                            continue;
                        }

                        let parts: Vec<&str> = p.split_whitespace().collect();
                        let (name, type_name) = if parts.len() >= 2 {
                            (parts[0].to_string(), Some(parts[1..].join(" ")))
                        } else {
                            (p.to_string(), None)
                        };

                        let is_variadic = type_name.as_ref().map(|t| t.starts_with("...")).unwrap_or(false);
                        let desc = DocDescriptionGenerator::generate_param_desc(&name, type_name.as_deref());

                        params.push(ParamInfo {
                            name,
                            type_name,
                            default_value: None,
                            is_self: false,
                            is_variadic,
                            is_optional: false,
                            description: desc,
                        });
                    }
                }

                // Return type is everything between `)` and `{`
                let rest = sig[close_paren + 1..].trim_end_matches(['{', ' ']).trim();
                if !rest.is_empty() {
                    let is_error = rest.contains("error");
                    let desc = DocDescriptionGenerator::generate_return_desc(rest, fn_name);

                    return_type = Some(ReturnInfo {
                        type_name: Some(rest.to_string()),
                        is_result_or_error: is_error,
                        is_option_or_nullable: rest.starts_with('*'),
                        description: desc,
                    });
                }
            }
        }

        (params, return_type)
    }

    // -----------------------------------------------------------------------
    // C / C++ Parsing
    // -----------------------------------------------------------------------

    fn parse_cpp_declaration(
        lines: &[&str],
        start_idx: usize,
        indent: String,
        existing_doc: Option<String>,
        doc_start_line: usize,
    ) -> Option<(SignatureInfo, usize)> {
        let first_line = lines[start_idx].trim();

        // 1. C++ Class / Struct
        static CPP_CLASS_RE: LazyLock<Regex> = LazyLock::new(|| {
            Regex::new(r"^(?:template\s*<[^>]+>\s*)?(class|struct)\s+([A-Za-z0-9_]+)").unwrap()
        });

        if let Some(caps) = CPP_CLASS_RE.captures(first_line) {
            let kind_str = caps.get(1).map(|m| m.as_str()).unwrap_or("class");
            let name = caps.get(2).map(|m| m.as_str().to_string())?;
            let kind = if kind_str == "struct" { SymbolKind::Struct } else { SymbolKind::Class };
            return Some((
                SignatureInfo {
                    name,
                    kind,
                    visibility: None,
                    is_async: false,
                    is_unsafe: false,
                    is_const: false,
                    is_static: false,
                    is_generator: false,
                    generics: None,
                    container: None,
                    params: Vec::new(),
                    return_type: None,
                    throws: Vec::new(),
                    raw_signature: first_line.to_string(),
                    existing_doc,
                    line_start: doc_start_line,
                    sig_line: start_idx + 1,
                    sig_end_line: start_idx + 1,
                    indent,
                },
                start_idx,
            ));
        }

        // 2. C/C++ Function
        static CPP_FN_RE: LazyLock<Regex> = LazyLock::new(|| {
            Regex::new(r"^(?:(?:static|virtual|inline|constexpr|explicit|friend)\s+)*([A-Za-z0-9_:<>&*~]+)\s+([A-Za-z0-9_~]+)\s*\(").unwrap()
        });

        if let Some(caps) = CPP_FN_RE.captures(first_line) {
            let ret_type = caps.get(1).map(|m| m.as_str().to_string());
            let name = caps.get(2).map(|m| m.as_str().to_string())?;

            if name != "if" && name != "while" && name != "for" && name != "switch" {
                let mut sig_lines = Vec::new();
                let mut curr = start_idx;
                let mut end_idx = start_idx;

                while curr < lines.len() {
                    let l = lines[curr];
                    sig_lines.push(l.trim());
                    if l.contains('{') || l.contains(';') || curr > start_idx + 8 {
                        end_idx = curr;
                        break;
                    }
                    curr += 1;
                }

                let full_sig = sig_lines.join(" ");
                let (params, _) = Self::parse_cpp_params(&full_sig);
                let desc = ret_type.as_ref().map(|rt| DocDescriptionGenerator::generate_return_desc(rt, &name));

                return Some((
                    SignatureInfo {
                        name,
                        kind: SymbolKind::Function,
                        visibility: None,
                        is_async: false,
                        is_unsafe: false,
                        is_const: full_sig.contains(") const"),
                        is_static: full_sig.contains("static "),
                        is_generator: false,
                        generics: None,
                        container: None,
                        params,
                        return_type: ret_type.map(|rt| ReturnInfo {
                            type_name: Some(rt),
                            is_result_or_error: false,
                            is_option_or_nullable: false,
                            description: desc.unwrap_or_default(),
                        }),
                        throws: Vec::new(),
                        raw_signature: full_sig.trim_end_matches(['{', ';', ' ']).trim().to_string(),
                        existing_doc,
                        line_start: doc_start_line,
                        sig_line: start_idx + 1,
                        sig_end_line: end_idx + 1,
                        indent,
                    },
                    end_idx,
                ));
            }
        }

        None
    }

    fn parse_cpp_params(sig: &str) -> (Vec<ParamInfo>, Option<ReturnInfo>) {
        let mut params = Vec::new();
        if let Some(open_paren) = sig.find('(') {
            if let Some(close_paren) = sig.rfind(')') {
                let param_str = &sig[open_paren + 1..close_paren].trim();
                if !param_str.is_empty() && *param_str != "void" {
                    let raw_params = Self::split_params_by_comma(param_str);
                    for p in raw_params {
                        let p = p.trim();
                        if p.is_empty() {
                            continue;
                        }

                        let (type_and_name, def_val) = if let Some(eq_idx) = p.find('=') {
                            (p[..eq_idx].trim(), Some(p[eq_idx + 1..].trim().to_string()))
                        } else {
                            (p, None)
                        };

                        let parts: Vec<&str> = type_and_name.split_whitespace().collect();
                        let (name, type_name) = if parts.len() >= 2 {
                            let name = parts.last().unwrap().trim_start_matches(['*', '&']).to_string();
                            let type_str = parts[..parts.len() - 1].join(" ");
                            (name, Some(type_str))
                        } else {
                            (type_and_name.to_string(), None)
                        };

                        let desc = DocDescriptionGenerator::generate_param_desc(&name, type_name.as_deref());

                        params.push(ParamInfo {
                            name,
                            type_name,
                            default_value: def_val,
                            is_self: false,
                            is_variadic: false,
                            is_optional: p.contains('='),
                            description: desc,
                        });
                    }
                }
            }
        }
        (params, None)
    }

    // -----------------------------------------------------------------------
    // Java / C# / Kotlin Parsing
    // -----------------------------------------------------------------------

    fn parse_java_csharp_declaration(
        lines: &[&str],
        start_idx: usize,
        _lang: DocLanguage,
        indent: String,
        existing_doc: Option<String>,
        doc_start_line: usize,
    ) -> Option<(SignatureInfo, usize)> {
        let first_line = lines[start_idx].trim();

        // 1. Class / Interface / Enum
        static JAVA_TYPE_RE: LazyLock<Regex> = LazyLock::new(|| {
            Regex::new(r"^(public|private|protected|internal)?\s*(?:(?:abstract|static|final|sealed)\s+)*(class|interface|enum|record)\s+([A-Za-z0-9_]+)(?:<([^>]+)>)?").unwrap()
        });

        if let Some(caps) = JAVA_TYPE_RE.captures(first_line) {
            let vis = caps.get(1).map(|m| m.as_str().to_string());
            let kind_str = caps.get(2).map(|m| m.as_str()).unwrap_or("class");
            let name = caps.get(3).map(|m| m.as_str().to_string())?;
            let generics = caps.get(4).map(|m| format!("<{}>", m.as_str().trim()));

            let kind = match kind_str {
                "interface" => SymbolKind::Interface,
                "enum" => SymbolKind::Enum,
                "struct" => SymbolKind::Struct,
                _ => SymbolKind::Class,
            };

            return Some((
                SignatureInfo {
                    name,
                    kind,
                    visibility: vis,
                    is_async: false,
                    is_unsafe: false,
                    is_const: false,
                    is_static: first_line.contains("static "),
                    is_generator: false,
                    generics,
                    container: None,
                    params: Vec::new(),
                    return_type: None,
                    throws: Vec::new(),
                    raw_signature: first_line.to_string(),
                    existing_doc,
                    line_start: doc_start_line,
                    sig_line: start_idx + 1,
                    sig_end_line: start_idx + 1,
                    indent,
                },
                start_idx,
            ));
        }

        // 2. Method
        static JAVA_METHOD_RE: LazyLock<Regex> = LazyLock::new(|| {
            Regex::new(r"^(public|private|protected|internal)?\s*(?:(?:static|final|abstract|synchronized|async|override|virtual)\s+)*([A-Za-z0-9_<>,\[\]]+)\s+([A-Za-z0-9_]+)\s*\(").unwrap()
        });

        if let Some(caps) = JAVA_METHOD_RE.captures(first_line) {
            let vis = caps.get(1).map(|m| m.as_str().to_string());
            let ret_type = caps.get(2).map(|m| m.as_str().to_string());
            let name = caps.get(3).map(|m| m.as_str().to_string())?;

            if name != "if" && name != "for" && name != "while" && name != "switch" && name != "catch" {
                let mut sig_lines = Vec::new();
                let mut curr = start_idx;
                let mut end_idx = start_idx;

                while curr < lines.len() {
                    let l = lines[curr];
                    sig_lines.push(l.trim());
                    if l.contains('{') || l.contains(';') || curr > start_idx + 8 {
                        end_idx = curr;
                        break;
                    }
                    curr += 1;
                }

                let full_sig = sig_lines.join(" ");
                let (params, _) = Self::parse_cpp_params(&full_sig);
                let desc = ret_type.as_ref().map(|rt| DocDescriptionGenerator::generate_return_desc(rt, &name));

                return Some((
                    SignatureInfo {
                        name,
                        kind: SymbolKind::Function,
                        visibility: vis,
                        is_async: full_sig.contains("async ") || full_sig.contains("Task<") || full_sig.contains("CompletableFuture<"),
                        is_unsafe: full_sig.contains("unsafe "),
                        is_const: false,
                        is_static: full_sig.contains("static "),
                        is_generator: false,
                        generics: None,
                        container: None,
                        params,
                        return_type: ret_type.map(|rt| ReturnInfo {
                            type_name: Some(rt),
                            is_result_or_error: false,
                            is_option_or_nullable: false,
                            description: desc.unwrap_or_default(),
                        }),
                        throws: Vec::new(),
                        raw_signature: full_sig.trim_end_matches(['{', ';', ' ']).trim().to_string(),
                        existing_doc,
                        line_start: doc_start_line,
                        sig_line: start_idx + 1,
                        sig_end_line: end_idx + 1,
                        indent,
                    },
                    end_idx,
                ));
            }
        }

        None
    }

    // -----------------------------------------------------------------------
    // Generic / Fallback Parsing
    // -----------------------------------------------------------------------

    fn parse_generic_declaration(
        lines: &[&str],
        start_idx: usize,
        indent: String,
        existing_doc: Option<String>,
        doc_start_line: usize,
    ) -> Option<(SignatureInfo, usize)> {
        let first_line = lines[start_idx].trim();

        static GENERIC_FN_RE: LazyLock<Regex> = LazyLock::new(|| {
            Regex::new(r"^(?:function|def|sub|fn)\s+([A-Za-z0-9_]+)\s*(?:\(([^)]*)\))?").unwrap()
        });

        if let Some(caps) = GENERIC_FN_RE.captures(first_line) {
            let name = caps.get(1).map(|m| m.as_str().to_string())?;
            let param_str = caps.get(2).map(|m| m.as_str()).unwrap_or("");

            let mut params = Vec::new();
            if !param_str.trim().is_empty() {
                for p in param_str.split(',') {
                    let clean = p.trim();
                    if !clean.is_empty() {
                        params.push(ParamInfo {
                            name: clean.to_string(),
                            type_name: None,
                            default_value: None,
                            is_self: false,
                            is_variadic: false,
                            is_optional: false,
                            description: DocDescriptionGenerator::generate_param_desc(clean, None),
                        });
                    }
                }
            }

            return Some((
                SignatureInfo {
                    name,
                    kind: SymbolKind::Function,
                    visibility: None,
                    is_async: false,
                    is_unsafe: false,
                    is_const: false,
                    is_static: false,
                    is_generator: false,
                    generics: None,
                    container: None,
                    params,
                    return_type: None,
                    throws: Vec::new(),
                    raw_signature: first_line.to_string(),
                    existing_doc,
                    line_start: doc_start_line,
                    sig_line: start_idx + 1,
                    sig_end_line: start_idx + 1,
                    indent,
                },
                start_idx,
            ));
        }

        None
    }

    /// Split parameter strings by comma while respecting generic angle brackets `<...>` and parentheses `(...)`.
    fn split_params_by_comma(s: &str) -> Vec<String> {
        let mut result = Vec::new();
        let mut curr = String::new();
        let mut angle_depth = 0;
        let mut paren_depth = 0;
        let mut bracket_depth = 0;

        for ch in s.chars() {
            match ch {
                '<' => angle_depth += 1,
                '>' => if angle_depth > 0 { angle_depth -= 1 },
                '(' => paren_depth += 1,
                ')' => if paren_depth > 0 { paren_depth -= 1 },
                '[' => bracket_depth += 1,
                ']' => if bracket_depth > 0 { bracket_depth -= 1 },
                ',' if angle_depth == 0 && paren_depth == 0 && bracket_depth == 0 => {
                    let item = curr.trim().to_string();
                    if !item.is_empty() {
                        result.push(item);
                    }
                    curr.clear();
                    continue;
                }
                _ => {}
            }
            curr.push(ch);
        }

        let item = curr.trim().to_string();
        if !item.is_empty() {
            result.push(item);
        }

        result
    }
}

// ---------------------------------------------------------------------------
// Natural Language Description Generator
// ---------------------------------------------------------------------------

pub struct DocDescriptionGenerator;

impl DocDescriptionGenerator {
    /// Split identifier name into human-readable words.
    pub fn split_identifier_words(name: &str) -> Vec<String> {
        let clean = name.trim_start_matches('_');
        let mut words = Vec::new();
        let mut curr = String::new();
        let chars: Vec<char> = clean.chars().collect();

        for i in 0..chars.len() {
            let ch = chars[i];
            if ch == '_' || ch == '-' {
                if !curr.is_empty() {
                    words.push(curr.to_lowercase());
                    curr.clear();
                }
            } else if ch.is_uppercase() {
                if !curr.is_empty() {
                    let prev_is_lower = i > 0 && chars[i - 1].is_lowercase();
                    let next_is_lower = i + 1 < chars.len() && chars[i + 1].is_lowercase();
                    if prev_is_lower || (curr.len() > 1 && next_is_lower) {
                        words.push(curr.to_lowercase());
                        curr.clear();
                    }
                }
                curr.push(ch);
            } else {
                curr.push(ch);
            }
        }

        if !curr.is_empty() {
            words.push(curr.to_lowercase());
        }

        words
    }

    /// Generate concise summary phrase for a symbol based on its name and kind.
    pub fn generate_summary(sig: &SignatureInfo, _lang: DocLanguage) -> String {
        let words = Self::split_identifier_words(&sig.name);
        if words.is_empty() {
            return format!("The `{}` symbol.", sig.name);
        }

        match sig.kind {
            SymbolKind::Struct => {
                if sig.name.ends_with("Config") || sig.name.ends_with("Options") || sig.name.ends_with("Settings") {
                    format!("Configuration options for {}.", words[..words.len().saturating_sub(1)].join(" "))
                } else if sig.name.ends_with("Error") || sig.name.ends_with("Exception") {
                    format!("Error type representing {} failure conditions.", words[..words.len().saturating_sub(1)].join(" "))
                } else if sig.name.ends_with("Builder") {
                    format!("Builder for constructing [`{}`] instances.", sig.name.strip_suffix("Builder").unwrap_or(&sig.name))
                } else {
                    format!("Represents {}.", words.join(" "))
                }
            }
            SymbolKind::Class => {
                if sig.name.ends_with("Manager") || sig.name.ends_with("Handler") || sig.name.ends_with("Controller") || sig.name.ends_with("Service") {
                    format!("Coordinates and manages {} operations.", words[..words.len().saturating_sub(1)].join(" "))
                } else {
                    format!("Represents a {} entity.", words.join(" "))
                }
            }
            SymbolKind::Interface | SymbolKind::Trait => {
                format!("Defines the interface and contract for {}.", words.join(" "))
            }
            SymbolKind::Enum => {
                format!("Enumeration of possible {} variants.", words.join(" "))
            }
            SymbolKind::TypeAlias => {
                format!("Type alias for {}.", words.join(" "))
            }
            SymbolKind::Constant => {
                format!("Constant value for {}.", words.join(" "))
            }
            SymbolKind::Macro => {
                format!("Macro helper for {}.", words.join(" "))
            }
            SymbolKind::Function | SymbolKind::Module | SymbolKind::Variable => {
                Self::generate_function_verb_summary(&words, &sig.name)
            }
        }
    }

    fn generate_function_verb_summary(words: &[String], name: &str) -> String {
        if words.is_empty() {
            return format!("Executes `{name}`.");
        }

        let first = &words[0];
        let rest = words[1..].join(" ");

        match first.as_str() {
            "is" | "has" | "can" | "should" => {
                if rest.is_empty() {
                    format!("Checks whether `{name}` is true.")
                } else {
                    format!("Returns `true` if {rest}; otherwise `false`.")
                }
            }
            "get" | "fetch" | "find" | "load" | "read" | "lookup" | "query" => {
                if rest.is_empty() {
                    "Retrieves the requested value.".to_string()
                } else {
                    format!("Retrieves the {rest}.")
                }
            }
            "set" | "update" | "write" | "save" | "store" | "put" => {
                if rest.is_empty() {
                    "Updates the value.".to_string()
                } else {
                    format!("Sets or updates the {rest}.")
                }
            }
            "new" | "create" | "build" | "make" | "init" | "initialize" | "spawn" | "construct" => {
                if rest.is_empty() {
                    "Creates and initializes a new instance.".to_string()
                } else {
                    format!("Creates and initializes a new {rest}.")
                }
            }
            "delete" | "remove" | "clear" | "drop" | "purge" | "cleanup" | "clean" => {
                if rest.is_empty() {
                    "Removes the target item.".to_string()
                } else {
                    format!("Removes the specified {rest}.")
                }
            }
            "calculate" | "compute" | "count" | "estimate" | "sum" => {
                if rest.is_empty() {
                    "Calculates the result value.".to_string()
                } else {
                    format!("Calculates the computed {rest}.")
                }
            }
            "parse" | "decode" | "deserialize" => {
                if rest.is_empty() {
                    "Parses the input into structured data.".to_string()
                } else {
                    format!("Parses the {rest} from the given input.")
                }
            }
            "format" | "encode" | "serialize" | "stringify" | "to" | "as" | "into" => {
                if rest.is_empty() {
                    "Formats and converts the value.".to_string()
                } else {
                    format!("Converts and formats into {rest}.")
                }
            }
            "validate" | "verify" | "check" | "assert" | "ensure" => {
                if rest.is_empty() {
                    "Validates the input parameters.".to_string()
                } else {
                    format!("Validates the {rest}.")
                }
            }
            "handle" | "process" | "dispatch" | "on" => {
                if rest.is_empty() {
                    "Handles the event or action.".to_string()
                } else {
                    format!("Processes and handles the {rest}.")
                }
            }
            "send" | "post" | "emit" | "publish" | "broadcast" | "transmit" => {
                if rest.is_empty() {
                    "Transmits the message.".to_string()
                } else {
                    format!("Sends the {rest} to the designated target.")
                }
            }
            "receive" | "poll" | "listen" | "subscribe" | "consume" => {
                if rest.is_empty() {
                    "Receives incoming data.".to_string()
                } else {
                    format!("Listens for and receives {rest}.")
                }
            }
            "execute" | "run" | "start" | "launch" => {
                if rest.is_empty() {
                    "Executes the operation.".to_string()
                } else {
                    format!("Executes the {rest} task.")
                }
            }
            "stop" | "cancel" | "abort" | "terminate" | "close" | "shutdown" => {
                if rest.is_empty() {
                    "Terminates the active operation.".to_string()
                } else {
                    format!("Stops and terminates the {rest}.")
                }
            }
            "register" | "add" | "insert" | "attach" | "bind" => {
                if rest.is_empty() {
                    "Registers the item.".to_string()
                } else {
                    format!("Registers the specified {rest}.")
                }
            }
            "unregister" | "detach" | "unbind" => {
                if rest.is_empty() {
                    "Unregisters the item.".to_string()
                } else {
                    format!("Unregisters the specified {rest}.")
                }
            }
            "filter" | "search" | "scan" | "match" => {
                if rest.is_empty() {
                    "Filters items according to criteria.".to_string()
                } else {
                    format!("Searches for and filters {rest}.")
                }
            }
            "connect" | "disconnect" => {
                if rest.is_empty() {
                    format!("{first}s the endpoint.")
                } else {
                    format!("{first}s to the {rest}.")
                }
            }
            _ => {
                let capitalized = format!("{}{}", first[..1].to_uppercase(), &first[1..]);
                if rest.is_empty() {
                    format!("{capitalized}s the operation.")
                } else {
                    format!("{capitalized}s the {rest}.")
                }
            }
        }
    }

    /// Generate contextual parameter description from parameter name and type.
    pub fn generate_param_desc(name: &str, type_name: Option<&str>) -> String {
        let clean = name.trim_start_matches(['*', '&']).trim();
        let words = Self::split_identifier_words(clean);
        let joined = words.join(" ");

        match clean {
            "path" | "file_path" | "filepath" | "src" | "dest" | "target" | "dir" | "dir_path" => {
                "Path to the target file or directory.".to_string()
            }
            "timeout" | "timeout_ms" | "timeout_secs" | "duration" => {
                "Timeout duration for the operation.".to_string()
            }
            "query" | "search" | "pattern" | "filter" => {
                "Search query pattern or filtering criteria.".to_string()
            }
            "max_results" | "limit" | "count" | "size" | "capacity" => {
                "Maximum number of items to return or process.".to_string()
            }
            "offset" | "skip" | "page" | "index" => {
                "Zero-based offset or page index for pagination.".to_string()
            }
            "ctx" | "context" | "cx" => {
                "Context for execution, cancellation, and metadata propagation.".to_string()
            }
            "buf" | "buffer" | "data" | "bytes" | "content" | "payload" | "body" => {
                "Input data buffer or payload to process.".to_string()
            }
            "cb" | "callback" | "handler" | "listener" | "on_complete" => {
                "Callback function invoked upon event or completion.".to_string()
            }
            "options" | "config" | "settings" | "opts" | "params" => {
                "Configuration options and parameters for the operation.".to_string()
            }
            "id" | "uuid" | "key" | "name" | "identifier" => {
                "Unique identifier of the entity.".to_string()
            }
            "req" | "request" => "The incoming request object.".to_string(),
            "res" | "response" => "The outgoing response object.".to_string(),
            "err" | "error" => "The error or exception instance.".to_string(),
            _ => {
                if clean.starts_with("is_") || clean.starts_with("has_") || clean.starts_with("enable_") || clean == "force" || clean == "recursive" || clean == "exact" || clean == "case_sensitive" {
                    format!("Whether to enable {joined}.")
                } else if let Some(tn) = type_name {
                    format!("The {joined} (`{tn}`).")
                } else {
                    format!("The {joined} to use.")
                }
            }
        }
    }

    /// Generate return value description.
    pub fn generate_return_desc(ret_type: &str, fn_name: &str) -> String {
        let clean = ret_type.trim();
        if clean == "()" || clean == "void" || clean == "None" || clean.is_empty() {
            return "Nothing.".to_string();
        }

        if clean == "bool" || clean == "boolean" {
            let words = Self::split_identifier_words(fn_name);
            let rest = words.join(" ");
            return format!("`true` if {rest} succeeded or matches condition; otherwise `false`.");
        }

        if clean.starts_with("Result<") || clean.contains("Result<") {
            return format!("A Result containing the output if successful, or an Error if the operation fails.");
        }

        if clean.starts_with("Option<") || clean.starts_with("Optional[") {
            return "An Option containing the value if present, or `None` otherwise.".to_string();
        }

        if clean.starts_with("Promise<") {
            return "A Promise resolving to the resulting payload.".to_string();
        }

        format!("The resulting `{clean}` value.")
    }
}

// ---------------------------------------------------------------------------
// Doc Comment Formatters
// ---------------------------------------------------------------------------

pub struct DocFormatter;

impl DocFormatter {
    /// Format doc comment for a signature according to the requested style.
    pub fn format(sig: &SignatureInfo, style: DocStyle, lang: DocLanguage, include_examples: bool) -> String {
        let effective_style = if style == DocStyle::Auto {
            lang.default_style()
        } else {
            style
        };

        match effective_style {
            DocStyle::Rustdoc => Self::format_rustdoc(sig, include_examples),
            DocStyle::Jsdoc => Self::format_jsdoc(sig, include_examples),
            DocStyle::Google => Self::format_google(sig, include_examples),
            DocStyle::Sphinx => Self::format_sphinx(sig),
            DocStyle::Numpy => Self::format_numpy(sig),
            DocStyle::Godoc => Self::format_godoc(sig),
            DocStyle::Javadoc => Self::format_javadoc(sig),
            DocStyle::Doxygen => Self::format_doxygen(sig),
            DocStyle::Markdown => Self::format_markdown_block(sig, include_examples),
            DocStyle::Auto => Self::format_rustdoc(sig, include_examples),
        }
    }

    /// Format Rustdoc (`///` triple slash markdown).
    pub fn format_rustdoc(sig: &SignatureInfo, include_examples: bool) -> String {
        let mut lines = Vec::new();
        let summary = DocDescriptionGenerator::generate_summary(sig, DocLanguage::Rust);

        lines.push(format!("/// {summary}"));

        // Extended description or container
        if let Some(container) = &sig.container {
            lines.push("///".to_string());
            lines.push(format!("/// Associated with [`{container}`]."));
        }

        // Arguments section
        let non_self_params: Vec<&ParamInfo> = sig.params.iter().filter(|p| !p.is_self).collect();
        if !non_self_params.is_empty() {
            lines.push("///".to_string());
            lines.push("/// # Arguments".to_string());
            lines.push("///".to_string());
            for p in non_self_params {
                lines.push(format!("/// * `{}` - {}", p.name, p.description));
            }
        }

        // Returns section
        if let Some(ret) = &sig.return_type {
            if let Some(tn) = &ret.type_name {
                if tn != "()" && tn != "void" {
                    lines.push("///".to_string());
                    lines.push("/// # Returns".to_string());
                    lines.push("///".to_string());
                    lines.push(format!("/// {}", ret.description));
                }
            }
        }

        // Errors section
        if let Some(ret) = &sig.return_type {
            if ret.is_result_or_error {
                lines.push("///".to_string());
                lines.push("/// # Errors".to_string());
                lines.push("///".to_string());
                lines.push("/// This function will return an error if the underlying operation fails.".to_string());
            }
        }

        // Panics section (if unsafe or explicitly declared)
        if sig.is_unsafe {
            lines.push("///".to_string());
            lines.push("/// # Safety".to_string());
            lines.push("///".to_string());
            lines.push("/// Caller must ensure memory invariants and valid pointer references.".to_string());
        }

        // Examples section
        if include_examples && (sig.kind == SymbolKind::Function || sig.kind == SymbolKind::Struct) {
            lines.push("///".to_string());
            lines.push("/// # Examples".to_string());
            lines.push("///".to_string());
            lines.push("/// ```rust".to_string());
            if sig.kind == SymbolKind::Struct {
                lines.push(format!("/// let item = {}::default();", sig.name));
            } else {
                let call_args: Vec<String> = sig.params.iter().filter(|p| !p.is_self).map(|p| {
                    if let Some(tn) = &p.type_name {
                        if tn.contains("str") || tn.contains("String") {
                            "\"example\"".to_string()
                        } else if tn.contains("usize") || tn.contains("u64") || tn.contains("i32") {
                            "42".to_string()
                        } else if tn.contains("bool") {
                            "true".to_string()
                        } else {
                            "Default::default()".to_string()
                        }
                    } else {
                        "Default::default()".to_string()
                    }
                }).collect();

                let is_res = sig.return_type.as_ref().map(|r| r.is_result_or_error).unwrap_or(false);
                let suffix = if is_res { "?" } else { "" };
                let is_async = sig.is_async;
                let prefix = if is_async { "let result = " } else { "let result = " };
                let await_str = if is_async { ".await" } else { "" };

                lines.push(format!("/// {prefix}{}({}){await_str}{suffix};", sig.name, call_args.join(", ")));
            }
            lines.push("/// ```".to_string());
        }

        lines.join("\n")
    }

    /// Format JSDoc (`/** ... */`).
    pub fn format_jsdoc(sig: &SignatureInfo, include_examples: bool) -> String {
        let mut lines = Vec::new();
        let summary = DocDescriptionGenerator::generate_summary(sig, DocLanguage::TypeScript);

        lines.push("/**".to_string());
        lines.push(format!(" * {summary}"));

        let non_self_params: Vec<&ParamInfo> = sig.params.iter().filter(|p| !p.is_self).collect();
        if !non_self_params.is_empty() {
            lines.push(" *".to_string());
            for p in non_self_params {
                let type_tag = p.type_name.as_ref().map(|t| format!("{{{t}}} ")).unwrap_or_default();
                let name_tag = if p.is_optional {
                    if let Some(def) = &p.default_value {
                        format!("[{}={}]", p.name, def)
                    } else {
                        format!("[{}]", p.name)
                    }
                } else {
                    p.name.clone()
                };
                lines.push(format!(" * @param {type_tag}{name_tag} - {}", p.description));
            }
        }

        if let Some(ret) = &sig.return_type {
            if let Some(tn) = &ret.type_name {
                if tn != "void" && tn != "undefined" {
                    lines.push(" *".to_string());
                    lines.push(format!(" * @returns {{{tn}}} {}", ret.description));
                }
            }
        }

        if sig.is_async || !sig.throws.is_empty() {
            lines.push(" * @throws {Error} When the operation encounters an unrecoverable failure.".to_string());
        }

        if include_examples && sig.kind == SymbolKind::Function {
            lines.push(" *".to_string());
            lines.push(" * @example".to_string());
            lines.push(" * ```ts".to_string());
            let call_args: Vec<String> = sig.params.iter().filter(|p| !p.is_self).map(|p| {
                if let Some(tn) = &p.type_name {
                    if tn.contains("string") {
                        "\"example\"".to_string()
                    } else if tn.contains("number") {
                        "42".to_string()
                    } else if tn.contains("boolean") {
                        "true".to_string()
                    } else {
                        "{}".to_string()
                    }
                } else {
                    "{}".to_string()
                }
            }).collect();
            let await_str = if sig.is_async { "await " } else { "" };
            lines.push(format!(" * const res = {await_str}{}({});", sig.name, call_args.join(", ")));
            lines.push(" * ```".to_string());
        }

        lines.push(" */".to_string());
        lines.join("\n")
    }

    /// Format Python Google docstring (`"""..."""`).
    pub fn format_google(sig: &SignatureInfo, include_examples: bool) -> String {
        let mut lines = Vec::new();
        let summary = DocDescriptionGenerator::generate_summary(sig, DocLanguage::Python);

        lines.push(format!("\"\"\"{summary}"));

        let non_self_params: Vec<&ParamInfo> = sig.params.iter().filter(|p| !p.is_self).collect();
        if !non_self_params.is_empty() {
            lines.push(String::new());
            lines.push("Args:".to_string());
            for p in non_self_params {
                let type_part = if let Some(t) = &p.type_name {
                    if p.is_optional {
                        format!(" ({t}, optional)")
                    } else {
                        format!(" ({t})")
                    }
                } else {
                    String::new()
                };
                let def_part = if let Some(d) = &p.default_value {
                    format!(" Defaults to {d}.")
                } else {
                    String::new()
                };
                lines.push(format!("    {}{type_part}: {}{def_part}", p.name, p.description));
            }
        }

        if let Some(ret) = &sig.return_type {
            if let Some(tn) = &ret.type_name {
                if tn != "None" && tn != "void" {
                    lines.push(String::new());
                    lines.push("Returns:".to_string());
                    lines.push(format!("    {tn}: {}", ret.description));
                }
            }
        }

        if sig.is_async {
            lines.push(String::new());
            lines.push("Raises:".to_string());
            lines.push("    RuntimeError: If execution fails during the async task.".to_string());
        }

        if include_examples && sig.kind == SymbolKind::Function {
            lines.push(String::new());
            lines.push("Example:".to_string());
            let call_args: Vec<String> = sig.params.iter().filter(|p| !p.is_self).map(|p| {
                if let Some(d) = &p.default_value {
                    format!("{}={d}", p.name)
                } else {
                    format!("\"value\"")
                }
            }).collect();
            lines.push(format!("    >>> {}({})", sig.name, call_args.join(", ")));
            lines.push("    'result'".to_string());
        }

        lines.push("\"\"\"".to_string());
        lines.join("\n")
    }

    /// Format Python Sphinx / reST docstring (`"""..."""`).
    pub fn format_sphinx(sig: &SignatureInfo) -> String {
        let mut lines = Vec::new();
        let summary = DocDescriptionGenerator::generate_summary(sig, DocLanguage::Python);

        lines.push(format!("\"\"\"{summary}"));
        lines.push(String::new());

        let non_self_params: Vec<&ParamInfo> = sig.params.iter().filter(|p| !p.is_self).collect();
        for p in non_self_params {
            lines.push(format!(":param {}: {}", p.name, p.description));
            if let Some(t) = &p.type_name {
                let opt = if p.is_optional { ", optional" } else { "" };
                lines.push(format!(":type {}: {t}{opt}", p.name));
            }
        }

        if let Some(ret) = &sig.return_type {
            if let Some(tn) = &ret.type_name {
                if tn != "None" {
                    lines.push(format!(":returns: {}", ret.description));
                    lines.push(format!(":rtype: {tn}"));
                }
            }
        }

        lines.push("\"\"\"".to_string());
        lines.join("\n")
    }

    /// Format Python NumPy docstring (`"""..."""`).
    pub fn format_numpy(sig: &SignatureInfo) -> String {
        let mut lines = Vec::new();
        let summary = DocDescriptionGenerator::generate_summary(sig, DocLanguage::Python);

        lines.push(format!("\"\"\"{summary}"));

        let non_self_params: Vec<&ParamInfo> = sig.params.iter().filter(|p| !p.is_self).collect();
        if !non_self_params.is_empty() {
            lines.push(String::new());
            lines.push("Parameters".to_string());
            lines.push("----------".to_string());
            for p in non_self_params {
                let type_part = p.type_name.as_deref().unwrap_or("object");
                let opt = if p.is_optional { ", optional" } else { "" };
                lines.push(format!("{} : {type_part}{opt}", p.name));
                lines.push(format!("    {}", p.description));
            }
        }

        if let Some(ret) = &sig.return_type {
            if let Some(tn) = &ret.type_name {
                if tn != "None" {
                    lines.push(String::new());
                    lines.push("Returns".to_string());
                    lines.push("-------".to_string());
                    lines.push(tn.clone());
                    lines.push(format!("    {}", ret.description));
                }
            }
        }

        lines.push("\"\"\"".to_string());
        lines.join("\n")
    }

    /// Format GoDoc (`// FunctionName does ...`).
    pub fn format_godoc(sig: &SignatureInfo) -> String {
        let mut lines = Vec::new();
        let summary = DocDescriptionGenerator::generate_summary(sig, DocLanguage::Go);

        lines.push(format!("// {} {}", sig.name, summary.trim_end_matches('.')));

        let non_self_params: Vec<&ParamInfo> = sig.params.iter().filter(|p| !p.is_self).collect();
        if !non_self_params.is_empty() {
            lines.push("//".to_string());
            lines.push("// Parameters:".to_string());
            for p in non_self_params {
                lines.push(format!("//   - {}: {}", p.name, p.description));
            }
        }

        if let Some(ret) = &sig.return_type {
            if let Some(tn) = &ret.type_name {
                lines.push("//".to_string());
                lines.push(format!("// Returns {tn}: {}", ret.description));
            }
        }

        lines.join("\n")
    }

    /// Format JavaDoc (`/** ... */`).
    pub fn format_javadoc(sig: &SignatureInfo) -> String {
        let mut lines = Vec::new();
        let summary = DocDescriptionGenerator::generate_summary(sig, DocLanguage::Java);

        lines.push("/**".to_string());
        lines.push(format!(" * {summary}"));

        let non_self_params: Vec<&ParamInfo> = sig.params.iter().filter(|p| !p.is_self).collect();
        if !non_self_params.is_empty() {
            lines.push(" *".to_string());
            for p in non_self_params {
                lines.push(format!(" * @param {} {}", p.name, p.description));
            }
        }

        if let Some(ret) = &sig.return_type {
            if let Some(tn) = &ret.type_name {
                if tn != "void" {
                    lines.push(format!(" * @return {}", ret.description));
                }
            }
        }

        lines.push(" */".to_string());
        lines.join("\n")
    }

    /// Format C/C++ Doxygen (`/** \brief ... */`).
    pub fn format_doxygen(sig: &SignatureInfo) -> String {
        let mut lines = Vec::new();
        let summary = DocDescriptionGenerator::generate_summary(sig, DocLanguage::Cpp);

        lines.push("/**".to_string());
        lines.push(format!(" * \\brief {summary}"));

        let non_self_params: Vec<&ParamInfo> = sig.params.iter().filter(|p| !p.is_self).collect();
        if !non_self_params.is_empty() {
            lines.push(" *".to_string());
            for p in non_self_params {
                lines.push(format!(" * \\param {} {}", p.name, p.description));
            }
        }

        if let Some(ret) = &sig.return_type {
            if let Some(tn) = &ret.type_name {
                if tn != "void" {
                    lines.push(format!(" * \\return {}", ret.description));
                }
            }
        }

        lines.push(" */".to_string());
        lines.join("\n")
    }

    /// Format Markdown block for documentation notes.
    pub fn format_markdown_block(sig: &SignatureInfo, _include_examples: bool) -> String {
        let mut lines = Vec::new();
        let summary = DocDescriptionGenerator::generate_summary(sig, DocLanguage::Generic);

        lines.push(format!("### `{}` ({})", sig.name, sig.kind.as_str()));
        lines.push(String::new());
        lines.push(summary);
        lines.push(String::new());

        lines.push("```".to_string());
        lines.push(sig.raw_signature.clone());
        lines.push("```".to_string());

        let non_self_params: Vec<&ParamInfo> = sig.params.iter().filter(|p| !p.is_self).collect();
        if !non_self_params.is_empty() {
            lines.push(String::new());
            lines.push("| Parameter | Type | Default | Description |".to_string());
            lines.push("| :--- | :--- | :--- | :--- |".to_string());
            for p in non_self_params {
                let t = p.type_name.as_deref().unwrap_or("-");
                let d = p.default_value.as_deref().unwrap_or("-");
                lines.push(format!("| `{}` | `{t}` | `{d}` | {} |", p.name, p.description));
            }
        }

        if let Some(ret) = &sig.return_type {
            if let Some(tn) = &ret.type_name {
                if tn != "void" && tn != "()" {
                    lines.push(String::new());
                    lines.push(format!("**Returns:** `{tn}` - {}", ret.description));
                }
            }
        }

        lines.join("\n")
    }
}

// ---------------------------------------------------------------------------
// Doc Coverage Auditor & Applier
// ---------------------------------------------------------------------------

pub struct DocAuditor;

impl DocAuditor {
    /// Compute documentation coverage metrics for the extracted signatures.
    pub fn audit(signatures: &[SignatureInfo], target: Option<String>, lang: DocLanguage) -> DocCoverageReport {
        let total_symbols = signatures.len();
        let mut documented_symbols = 0;
        let mut public_total = 0;
        let mut public_documented = 0;
        let mut symbol_statuses = Vec::new();

        for sig in signatures {
            let is_doc = sig.is_documented();
            let is_pub = sig.is_public();

            if is_doc {
                documented_symbols += 1;
            }
            if is_pub {
                public_total += 1;
                if is_doc {
                    public_documented += 1;
                }
            }

            let doc_lines_count = sig.existing_doc.as_ref().map(|d| d.lines().count()).unwrap_or(0);

            // Check if existing doc misses parameter tags
            let mut missing_param_docs = Vec::new();
            if let Some(doc_text) = &sig.existing_doc {
                for p in sig.params.iter().filter(|p| !p.is_self) {
                    if !doc_text.contains(&p.name) {
                        missing_param_docs.push(p.name.clone());
                    }
                }
            }

            let missing_return = if let Some(doc_text) = &sig.existing_doc {
                if let Some(ret) = &sig.return_type {
                    if ret.type_name.as_deref() != Some("()") && ret.type_name.as_deref() != Some("void") {
                        !doc_text.contains("return") && !doc_text.contains("Returns") && !doc_text.contains("@return")
                    } else {
                        false
                    }
                } else {
                    false
                }
            } else {
                false
            };

            symbol_statuses.push(SymbolDocStatus {
                name: sig.name.clone(),
                kind: sig.kind.as_str().to_string(),
                line: sig.sig_line,
                is_documented: is_doc,
                is_public: is_pub,
                existing_doc_lines: doc_lines_count,
                missing_param_docs,
                missing_return_doc: missing_return,
            });
        }

        let undocumented_symbols = total_symbols.saturating_sub(documented_symbols);
        let coverage_percentage = if total_symbols > 0 {
            (documented_symbols as f64 / total_symbols as f64) * 100.0
        } else {
            100.0
        };

        let public_coverage_percentage = if public_total > 0 {
            (public_documented as f64 / public_total as f64) * 100.0
        } else {
            100.0
        };

        let summary = format!(
            "Total symbols: {total_symbols}, Documented: {documented_symbols} ({coverage_percentage:.1}%), Undocumented: {undocumented_symbols}. Public: {public_documented}/{public_total} ({public_coverage_percentage:.1}%)."
        );

        DocCoverageReport {
            target,
            language: lang.as_str().to_string(),
            total_symbols,
            documented_symbols,
            undocumented_symbols,
            public_total,
            public_documented,
            coverage_percentage,
            public_coverage_percentage,
            symbols: symbol_statuses,
            summary,
        }
    }
}

pub struct DocApplier;

impl DocApplier {
    /// Apply/insert drafted doc comments into the source code above target declarations.
    pub fn apply(code: &str, signatures: &[SignatureInfo], style: DocStyle, lang: DocLanguage, public_only: bool) -> String {
        let lines: Vec<&str> = code.lines().collect();
        let mut modified_lines = Vec::new();
        let mut sig_map: HashMap<usize, &SignatureInfo> = HashMap::new();

        for sig in signatures {
            if public_only && !sig.is_public() {
                continue;
            }
            // Only apply if currently undocumented
            if !sig.is_documented() {
                sig_map.insert(sig.sig_line, sig);
            }
        }

        for (idx, line) in lines.iter().enumerate() {
            let line_num = idx + 1; // 1-based
            if let Some(sig) = sig_map.get(&line_num) {
                let doc_comment = DocFormatter::format(sig, style, lang, true);
                for doc_line in doc_comment.lines() {
                    modified_lines.push(format!("{}{doc_line}", sig.indent));
                }
            }
            modified_lines.push(line.to_string());
        }

        modified_lines.join("\n")
    }
}

pub struct MarkdownApiGenerator;

impl MarkdownApiGenerator {
    /// Generate a full GitHub-flavored Markdown API reference manual for source code.
    pub fn generate_markdown_api(
        title: &str,
        signatures: &[SignatureInfo],
        lang: DocLanguage,
        coverage: &DocCoverageReport,
    ) -> String {
        let mut md = Vec::new();

        md.push(format!("# API Reference: `{title}`"));
        md.push(String::new());
        md.push(format!("**Language:** {} | **Coverage:** {:.1}% ({}/{} documented)", lang.as_str(), coverage.coverage_percentage, coverage.documented_symbols, coverage.total_symbols));
        md.push(String::new());

        // Table of Contents
        md.push("## Table of Contents".to_string());
        md.push(String::new());

        let mut structs = Vec::new();
        let mut traits = Vec::new();
        let mut enums = Vec::new();
        let mut functions = Vec::new();
        let mut others = Vec::new();

        for sig in signatures {
            match sig.kind {
                SymbolKind::Struct | SymbolKind::Class => structs.push(sig),
                SymbolKind::Trait | SymbolKind::Interface => traits.push(sig),
                SymbolKind::Enum => enums.push(sig),
                SymbolKind::Function => functions.push(sig),
                _ => others.push(sig),
            }
        }

        if !structs.is_empty() {
            md.push("- **Structures & Classes**".to_string());
            for s in &structs {
                md.push(format!("  - [`{}`](#{})", s.name, s.name.to_lowercase()));
            }
        }
        if !traits.is_empty() {
            md.push("- **Traits & Interfaces**".to_string());
            for t in &traits {
                md.push(format!("  - [`{}`](#{})", t.name, t.name.to_lowercase()));
            }
        }
        if !enums.is_empty() {
            md.push("- **Enums**".to_string());
            for e in &enums {
                md.push(format!("  - [`{}`](#{})", e.name, e.name.to_lowercase()));
            }
        }
        if !functions.is_empty() {
            md.push("- **Functions & Methods**".to_string());
            for f in &functions {
                md.push(format!("  - [`{}`](#{})", f.name, f.name.to_lowercase()));
            }
        }
        if !others.is_empty() {
            md.push("- **Types & Constants**".to_string());
            for o in &others {
                md.push(format!("  - [`{}`](#{})", o.name, o.name.to_lowercase()));
            }
        }

        md.push(String::new());
        md.push("---".to_string());
        md.push(String::new());

        // Detailed sections
        let sections = [
            ("Structures & Classes", structs),
            ("Traits & Interfaces", traits),
            ("Enums", enums),
            ("Functions & Methods", functions),
            ("Types & Constants", others),
        ];

        for (sec_title, sig_list) in sections {
            if sig_list.is_empty() {
                continue;
            }

            md.push(format!("## {sec_title}"));
            md.push(String::new());

            for sig in sig_list {
                md.push(DocFormatter::format_markdown_block(sig, true));
                md.push(String::new());
                md.push("---".to_string());
                md.push(String::new());
            }
        }

        md.join("\n")
    }
}

// ---------------------------------------------------------------------------
// Tool Implementation
// ---------------------------------------------------------------------------

/// Tool for extracting code signatures and drafting documentation comments.
#[derive(Default, Debug, Clone)]
pub struct DocgenTool;

impl DocgenTool {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl Tool for DocgenTool {
    fn name(&self) -> &str {
        "docgen"
    }

    fn description(&self) -> &str {
        "Extract code signatures, draft doc comments (Rustdoc, JSDoc, Python Google/Sphinx/NumPy, GoDoc, JavaDoc), audit doc coverage, and generate Markdown API references."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "Path to the file to generate documentation for or audit."
                },
                "code": {
                    "type": "string",
                    "description": "Direct source code snippet to analyze or document (optional if path is provided)."
                },
                "action": {
                    "type": "string",
                    "enum": ["draft", "extract", "audit", "apply", "markdown"],
                    "description": "Operation mode: 'draft' (generate doc comments, default), 'extract' (structured signatures), 'audit' (doc coverage check), 'apply' (insert comments into source), or 'markdown' (generate full API manual)."
                },
                "symbol": {
                    "type": "string",
                    "description": "Specific symbol name to target (optional, analyzes all symbols if omitted)."
                },
                "style": {
                    "type": "string",
                    "enum": ["auto", "rustdoc", "jsdoc", "google", "sphinx", "numpy", "godoc", "javadoc", "doxygen", "markdown"],
                    "description": "Doc comment style: 'auto' (default, matches language), 'rustdoc', 'jsdoc', 'google', 'sphinx', 'numpy', 'godoc', 'javadoc', 'doxygen', or 'markdown'."
                },
                "language": {
                    "type": "string",
                    "description": "Programming language or extension override (e.g. 'rust', 'ts', 'python', 'go', 'rs', 'py')."
                },
                "public_only": {
                    "type": "boolean",
                    "description": "Whether to only target public / exported symbols (default: false)."
                },
                "include_examples": {
                    "type": "boolean",
                    "description": "Whether to include code examples in drafted documentation (default: true)."
                },
                "min_coverage": {
                    "type": "number",
                    "description": "For audit action: minimum required documentation coverage percentage (e.g. 80.0). Fails if below threshold."
                },
                "format": {
                    "type": "string",
                    "enum": ["text", "markdown", "json"],
                    "description": "Output format: 'text' (default human-readable), 'markdown', or 'json' (raw structured data)."
                }
            }
        })
    }

    async fn execute(&self, args: Value, ctx: &ToolContext) -> anyhow::Result<String> {
        let action_str = args
            .get("action")
            .and_then(|v| v.as_str())
            .or_else(|| args.get("mode").and_then(|v| v.as_str()))
            .unwrap_or("draft");
        let action = DocgenAction::from_str_loose(action_str);

        let style_str = args
            .get("style")
            .and_then(|v| v.as_str())
            .unwrap_or("auto");
        let style = DocStyle::from_str_loose(style_str);

        let target_symbol = args
            .get("symbol")
            .and_then(|v| v.as_str())
            .or_else(|| args.get("name").and_then(|v| v.as_str()))
            .map(|s| s.to_string());

        let public_only = args
            .get("public_only")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        let include_examples = args
            .get("include_examples")
            .and_then(|v| v.as_bool())
            .unwrap_or(true);

        let min_coverage = args
            .get("min_coverage")
            .and_then(|v| v.as_f64());

        let format = args
            .get("format")
            .and_then(|v| v.as_str())
            .unwrap_or("text");

        let lang_override = args
            .get("language")
            .and_then(|v| v.as_str())
            .or_else(|| args.get("lang").and_then(|v| v.as_str()));

        // Resolve code and language
        let (code, language, target_path_str) = if let Some(code_val) = args.get("code").and_then(|v| v.as_str()) {
            let lang = if let Some(lo) = lang_override {
                DocLanguage::from_path_or_name(lo)
            } else {
                DocLanguage::detect_from_code(code_val)
            };
            (code_val.to_string(), lang, None)
        } else if let Some(path_val) = args.get("path").and_then(|v| v.as_str()).or_else(|| args.get("file").and_then(|v| v.as_str())) {
            let resolved = resolve_path(path_val, &ctx.cwd);
            if !resolved.exists() {
                anyhow::bail!("Target file does not exist: '{}'", resolved.display());
            }
            let file_content = tokio::fs::read_to_string(&resolved).await
                .map_err(|e| anyhow::anyhow!("Failed to read file '{}': {e}", resolved.display()))?;

            let lang = if let Some(lo) = lang_override {
                DocLanguage::from_path_or_name(lo)
            } else {
                DocLanguage::from_path_or_name(path_val)
            };
            (file_content, lang, Some(resolved.display().to_string()))
        } else {
            anyhow::bail!("Either 'path' or 'code' must be specified for docgen.");
        };

        // Extract signatures
        let mut signatures = SignatureExtractor::extract(&code, language);

        // Filter by target symbol if specified
        if let Some(sym_name) = &target_symbol {
            signatures.retain(|s| s.name == *sym_name || s.name.contains(sym_name.as_str()));
        }

        // Filter by public_only if specified
        if public_only {
            signatures.retain(|s| s.is_public());
        }

        // Execute requested action
        match action {
            DocgenAction::Extract => {
                if format == "json" {
                    return Ok(serde_json::to_string_pretty(&signatures)?);
                }
                let mut out = Vec::new();
                out.push(format!("Extracted {} signatures ({})", signatures.len(), language.as_str()));
                out.push(String::new());
                for sig in &signatures {
                    let vis = sig.visibility.as_deref().unwrap_or("private");
                    let doc_tag = if sig.is_documented() { "[documented]" } else { "[undocumented]" };
                    out.push(format!("- {} `{}` (line {}): {} {}", sig.kind.as_str(), sig.name, sig.sig_line, vis, doc_tag));
                    out.push(format!("  Signature: {}", sig.raw_signature));
                    if !sig.params.is_empty() {
                        let params_str: Vec<String> = sig.params.iter().map(|p| {
                            let t = p.type_name.as_deref().unwrap_or("?");
                            format!("{}: {t}", p.name)
                        }).collect();
                        out.push(format!("  Params: {}", params_str.join(", ")));
                    }
                    if let Some(ret) = &sig.return_type {
                        out.push(format!("  Returns: {}", ret.type_name.as_deref().unwrap_or("()")));
                    }
                    out.push(String::new());
                }
                Ok(out.join("\n"))
            }
            DocgenAction::Audit => {
                let coverage = DocAuditor::audit(&signatures, target_path_str, language);

                if let Some(min_cov) = min_coverage {
                    if coverage.coverage_percentage < min_cov {
                        anyhow::bail!(
                            "Documentation coverage {:.1}% is below required minimum threshold {:.1}%. Undocumented symbols: {}",
                            coverage.coverage_percentage,
                            min_cov,
                            coverage.symbols.iter().filter(|s| !s.is_documented).map(|s| s.name.as_str()).collect::<Vec<_>>().join(", ")
                        );
                    }
                }

                if format == "json" {
                    return Ok(serde_json::to_string_pretty(&coverage)?);
                }

                let mut out = Vec::new();
                out.push("=== Documentation Coverage Audit ===".to_string());
                out.push(coverage.summary.clone());
                out.push(String::new());

                let undocumented: Vec<&SymbolDocStatus> = coverage.symbols.iter().filter(|s| !s.is_documented).collect();
                if !undocumented.is_empty() {
                    out.push("Undocumented Symbols:".to_string());
                    for s in undocumented {
                        let pub_tag = if s.is_public { "(public)" } else { "(private)" };
                        out.push(format!("  - Line {:4} | {:8} | {} {}", s.line, s.kind, s.name, pub_tag));
                    }
                    out.push(String::new());
                }

                let documented: Vec<&SymbolDocStatus> = coverage.symbols.iter().filter(|s| s.is_documented).collect();
                if !documented.is_empty() {
                    out.push("Documented Symbols:".to_string());
                    for s in documented {
                        let miss_p = if !s.missing_param_docs.is_empty() {
                            format!(" (missing param tags: {})", s.missing_param_docs.join(", "))
                        } else {
                            String::new()
                        };
                        out.push(format!("  - Line {:4} | {:8} | {} [{} lines]{miss_p}", s.line, s.kind, s.name, s.existing_doc_lines));
                    }
                }

                Ok(out.join("\n"))
            }
            DocgenAction::Apply => {
                let modified = DocApplier::apply(&code, &signatures, style, language, public_only);

                // If path was provided and user asked to apply, write back to file
                if let Some(path_val) = args.get("path").and_then(|v| v.as_str()).or_else(|| args.get("file").and_then(|v| v.as_str())) {
                    let resolved = resolve_path(path_val, &ctx.cwd);
                    tokio::fs::write(&resolved, &modified).await
                        .map_err(|e| anyhow::anyhow!("Failed to write updated file '{path_val}': {e}"))?;
                    Ok(format!("Successfully inserted documentation comments into '{}'. Total symbols documented: {}.", resolved.display(), signatures.len()))
                } else {
                    if format == "json" {
                        let res = json!({
                            "action": "apply",
                            "language": language.as_str(),
                            "style": style.as_str(),
                            "modified_code": modified,
                            "symbols_count": signatures.len()
                        });
                        Ok(serde_json::to_string_pretty(&res)?)
                    } else {
                        Ok(modified)
                    }
                }
            }
            DocgenAction::Markdown => {
                let coverage = DocAuditor::audit(&signatures, target_path_str.clone(), language);
                let title = target_path_str.as_deref().unwrap_or("Code Snippet");
                let md = MarkdownApiGenerator::generate_markdown_api(title, &signatures, language, &coverage);
                Ok(md)
            }
            DocgenAction::Draft => {
                let mut drafts = Vec::new();
                for sig in &signatures {
                    let doc_comment = DocFormatter::format(sig, style, language, include_examples);
                    let preview = format!("{}\n{}", doc_comment, sig.raw_signature);
                    drafts.push(DraftResult {
                        symbol_name: sig.name.clone(),
                        kind: sig.kind.as_str().to_string(),
                        line: sig.sig_line,
                        doc_comment,
                        combined_preview: preview,
                    });
                }

                if format == "json" {
                    let out_obj = DocgenOutput {
                        action: "draft".to_string(),
                        language: language.as_str().to_string(),
                        style: style.as_str().to_string(),
                        drafts,
                        signatures: Vec::new(),
                        coverage: None,
                        modified_code: None,
                        markdown_doc: None,
                        total_symbols: signatures.len(),
                    };
                    return Ok(serde_json::to_string_pretty(&out_obj)?);
                }

                let mut out = Vec::new();
                out.push(format!("Drafted documentation for {} symbols (Language: {}, Style: {}):", drafts.len(), language.as_str(), style.as_str()));
                out.push(String::new());

                for draft in &drafts {
                    out.push(format!("=== [{}] {} (line {}) ===", draft.kind, draft.symbol_name, draft.line));
                    out.push(draft.doc_comment.clone());
                    out.push(String::new());
                }

                Ok(out.join("\n"))
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Unit Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_rust_fn_signature_extraction() {
        let code = r#"
/// Calculates the checksum for a given slice.
pub async fn calculate_checksum(data: &[u8], timeout_ms: u64) -> Result<String, std::io::Error> {
    Ok("hash".to_string())
}
"#;
        let sigs = SignatureExtractor::extract(code, DocLanguage::Rust);
        assert_eq!(sigs.len(), 1);
        let sig = &sigs[0];
        assert_eq!(sig.name, "calculate_checksum");
        assert_eq!(sig.kind, SymbolKind::Function);
        assert_eq!(sig.visibility.as_deref(), Some("pub"));
        assert!(sig.is_async);
        assert_eq!(sig.params.len(), 2);
        assert_eq!(sig.params[0].name, "data");
        assert_eq!(sig.params[0].type_name.as_deref(), Some("&[u8]"));
        assert_eq!(sig.params[1].name, "timeout_ms");
        assert_eq!(sig.params[1].type_name.as_deref(), Some("u64"));
        assert!(sig.return_type.is_some());
        assert!(sig.return_type.as_ref().unwrap().is_result_or_error);
        assert!(sig.is_documented());
    }

    #[test]
    fn test_rust_struct_enum_extraction() {
        let code = r#"
pub struct UserConfig {
    pub name: String,
    pub timeout: u64,
}

pub enum LogLevel {
    Info,
    Warn,
    Error,
}
"#;
        let sigs = SignatureExtractor::extract(code, DocLanguage::Rust);
        assert_eq!(sigs.len(), 2);
        assert_eq!(sigs[0].name, "UserConfig");
        assert_eq!(sigs[0].kind, SymbolKind::Struct);
        assert_eq!(sigs[1].name, "LogLevel");
        assert_eq!(sigs[1].kind, SymbolKind::Enum);
    }

    #[test]
    fn test_ts_signature_extraction() {
        let code = r#"
export async function fetchUserData(userId: string, options?: FetchOptions): Promise<User> {
    return await api.get(userId);
}

export class SessionManager {
    public static getInstance(): SessionManager {
        return new SessionManager();
    }
}
"#;
        let sigs = SignatureExtractor::extract(code, DocLanguage::TypeScript);
        assert_eq!(sigs.len(), 2);
        assert_eq!(sigs[0].name, "fetchUserData");
        assert_eq!(sigs[0].kind, SymbolKind::Function);
        assert!(sigs[0].is_async);
        assert_eq!(sigs[0].params.len(), 2);
        assert_eq!(sigs[0].params[0].name, "userId");
        assert_eq!(sigs[0].params[1].name, "options");
        assert!(sigs[0].params[1].is_optional);

        assert_eq!(sigs[1].name, "SessionManager");
        assert_eq!(sigs[1].kind, SymbolKind::Class);
    }

    #[test]
    fn test_python_signature_extraction() {
        let code = r#"
async def process_batch(items: list[str], max_workers: int = 4) -> bool:
    """Processes batch items concurrently."""
    return True
"#;
        let sigs = SignatureExtractor::extract(code, DocLanguage::Python);
        assert_eq!(sigs.len(), 1);
        let sig = &sigs[0];
        assert_eq!(sig.name, "process_batch");
        assert!(sig.is_async);
        assert_eq!(sig.params.len(), 2);
        assert_eq!(sig.params[0].name, "items");
        assert_eq!(sig.params[1].name, "max_workers");
        assert_eq!(sig.params[1].default_value.as_deref(), Some("4"));
        assert!(sig.return_type.is_some());
        assert!(sig.is_documented());
    }

    #[test]
    fn test_go_signature_extraction() {
        let code = r#"
// HandleRequest processes the incoming HTTP payload.
func (s *Server) HandleRequest(ctx context.Context, payload []byte) (int, error) {
    return 200, nil
}
"#;
        let sigs = SignatureExtractor::extract(code, DocLanguage::Go);
        assert_eq!(sigs.len(), 1);
        let sig = &sigs[0];
        assert_eq!(sig.name, "HandleRequest");
        assert_eq!(sig.container.as_deref(), Some("s *Server"));
        assert_eq!(sig.params.len(), 2);
        assert_eq!(sig.params[0].name, "ctx");
        assert_eq!(sig.params[1].name, "payload");
        assert!(sig.return_type.is_some());
        assert!(sig.return_type.as_ref().unwrap().is_result_or_error);
        assert!(sig.is_documented());
    }

    #[test]
    fn test_doc_formatters() {
        let sig = SignatureInfo {
            name: "calculate_hash".to_string(),
            kind: SymbolKind::Function,
            visibility: Some("pub".to_string()),
            is_async: true,
            is_unsafe: false,
            is_const: false,
            is_static: false,
            is_generator: false,
            generics: None,
            container: None,
            params: vec![
                ParamInfo {
                    name: "path".to_string(),
                    type_name: Some("&Path".to_string()),
                    default_value: None,
                    is_self: false,
                    is_variadic: false,
                    is_optional: false,
                    description: "Path to the target file.".to_string(),
                },
                ParamInfo {
                    name: "timeout_ms".to_string(),
                    type_name: Some("u64".to_string()),
                    default_value: Some("5000".to_string()),
                    is_self: false,
                    is_variadic: false,
                    is_optional: true,
                    description: "Timeout duration.".to_string(),
                },
            ],
            return_type: Some(ReturnInfo {
                type_name: Some("Result<String, Error>".to_string()),
                is_result_or_error: true,
                is_option_or_nullable: false,
                description: "The hash string.".to_string(),
            }),
            throws: vec!["Errors if file missing".to_string()],
            raw_signature: "pub async fn calculate_hash(path: &Path, timeout_ms: u64) -> Result<String, Error>".to_string(),
            existing_doc: None,
            line_start: 1,
            sig_line: 1,
            sig_end_line: 1,
            indent: String::new(),
        };

        // Rustdoc
        let rustdoc = DocFormatter::format_rustdoc(&sig, true);
        assert!(rustdoc.contains("///"));
        assert!(rustdoc.contains("# Arguments"));
        assert!(rustdoc.contains("`path`"));
        assert!(rustdoc.contains("# Returns"));
        assert!(rustdoc.contains("# Errors"));

        // JSDoc
        let jsdoc = DocFormatter::format_jsdoc(&sig, true);
        assert!(jsdoc.contains("/**"));
        assert!(jsdoc.contains("@param"));
        assert!(jsdoc.contains("@returns"));
        assert!(jsdoc.contains("*/"));

        // Google docstring
        let google = DocFormatter::format_google(&sig, true);
        assert!(google.starts_with("\"\"\""));
        assert!(google.contains("Args:"));
        assert!(google.contains("Returns:"));
        assert!(google.ends_with("\"\"\""));

        // GoDoc
        let godoc = DocFormatter::format_godoc(&sig);
        assert!(godoc.contains("// calculate_hash"));
        assert!(godoc.contains("// Parameters:"));
    }

    #[test]
    fn test_doc_coverage_audit() {
        let code = r#"
/// Documented function
pub fn func_a() {}

pub fn func_b(x: i32) -> bool { true }

pub struct MyStruct;
"#;
        let sigs = SignatureExtractor::extract(code, DocLanguage::Rust);
        assert_eq!(sigs.len(), 3);
        let audit = DocAuditor::audit(&sigs, None, DocLanguage::Rust);
        assert_eq!(audit.total_symbols, 3);
        assert_eq!(audit.documented_symbols, 1);
        assert_eq!(audit.undocumented_symbols, 2);
        assert!((audit.coverage_percentage - 33.33).abs() < 1.0);
    }

    #[test]
    fn test_doc_applier() {
        let code = r#"pub fn add_numbers(a: i32, b: i32) -> i32 {
    a + b
}"#;
        let sigs = SignatureExtractor::extract(code, DocLanguage::Rust);
        let applied = DocApplier::apply(code, &sigs, DocStyle::Rustdoc, DocLanguage::Rust, false);
        assert!(applied.contains("///"));
        assert!(applied.contains("# Arguments"));
        assert!(applied.contains("pub fn add_numbers"));
    }

    #[test]
    fn test_markdown_api_generator() {
        let code = r#"
pub struct ServiceConfig {
    pub port: u16,
}

pub fn start_server(config: &ServiceConfig) -> Result<(), Error> {
    Ok(())
}
"#;
        let sigs = SignatureExtractor::extract(code, DocLanguage::Rust);
        let audit = DocAuditor::audit(&sigs, None, DocLanguage::Rust);
        let md = MarkdownApiGenerator::generate_markdown_api("server.rs", &sigs, DocLanguage::Rust, &audit);
        assert!(md.contains("# API Reference: `server.rs`"));
        assert!(md.contains("## Table of Contents"));
        assert!(md.contains("## Structures & Classes"));
        assert!(md.contains("## Functions & Methods"));
    }

    #[tokio::test]
    async fn test_docgen_tool_execute() {
        let tool = DocgenTool::new();
        let ctx = ToolContext::default();

        let code = r#"
pub fn compute_sum(a: i32, b: i32) -> i32 {
    a + b
}
"#;
        // 1. Test draft
        let res = tool.execute(json!({
            "code": code,
            "language": "rust",
            "action": "draft"
        }), &ctx).await.unwrap();
        assert!(res.contains("Drafted documentation for 1 symbols"));
        assert!(res.contains("compute_sum"));

        // 2. Test audit
        let audit_res = tool.execute(json!({
            "code": code,
            "language": "rust",
            "action": "audit"
        }), &ctx).await.unwrap();
        assert!(audit_res.contains("Documentation Coverage Audit"));

        // 3. Test apply
        let applied_res = tool.execute(json!({
            "code": code,
            "language": "rust",
            "action": "apply"
        }), &ctx).await.unwrap();
        assert!(applied_res.contains("///"));
        assert!(applied_res.contains("compute_sum"));

        // 4. Test markdown
        let md_res = tool.execute(json!({
            "code": code,
            "language": "rust",
            "action": "markdown"
        }), &ctx).await.unwrap();
        assert!(md_res.contains("# API Reference:"));
    }
}
