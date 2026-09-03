//! Advanced path include/exclude glob filters, streaming log/text filter, and filterable grep engine.
//!
//! Provides comprehensive streaming and file searching capabilities:
//! - Streaming grep filter for large text streams and log files with constant memory overhead
//! - Context lines before and after matches (`-B`, `-A`, `-C`) with circular streaming window
//! - Inverted matching (`invert_match` / `-v`)
//! - Literal / fixed strings matching (`fixed_strings` / `-F`)
//! - Line numbers and match byte span tracking
//! - Resource limits: maximum match limit (`max_matches`) and output byte limit (`max_bytes`)
//! - Long line truncation safety limits (`max_line_length`)
//! - Binary file probe and fast skip
//! - Include and exclude glob patterns (e.g. `*.rs`, `src/**/*.ts`, `!*.test.js`)
//! - File type shortcuts and extension groups (e.g. `rust`, `python`, `typescript`)
//! - Size bounds (`min_file_size`, `max_file_size`) and directory depth constraints (`max_depth`)
//! - Structured search output, plain text formatting, and JSON summaries
//! - Full integration with the `Tool` trait for agent tool execution

use async_trait::async_trait;
use globset::{GlobBuilder, GlobSet, GlobSetBuilder};
use ignore::overrides::OverrideBuilder;
use ignore::WalkBuilder;
use regex::{Regex, RegexBuilder};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet, VecDeque};
use std::fs::{self, File};
use std::io::{BufRead, BufReader, Cursor};
use std::path::{Path, PathBuf};

use crate::tools::types::{Tool, ToolContext};

// ---------------------------------------------------------------------------
// File Type Registry
// ---------------------------------------------------------------------------

/// Maps language / type aliases to known file extensions.
#[derive(Debug, Clone)]
pub struct FileTypeRegistry {
    type_map: HashMap<String, Vec<String>>,
}

impl Default for FileTypeRegistry {
    fn default() -> Self {
        let mut type_map = HashMap::new();

        type_map.insert("rust".to_string(), vec!["rs".to_string()]);
        type_map.insert(
            "python".to_string(),
            vec!["py".to_string(), "pyi".to_string(), "pyx".to_string()],
        );
        type_map.insert(
            "javascript".to_string(),
            vec![
                "js".to_string(),
                "mjs".to_string(),
                "cjs".to_string(),
                "jsx".to_string(),
            ],
        );
        type_map.insert(
            "js".to_string(),
            vec![
                "js".to_string(),
                "mjs".to_string(),
                "cjs".to_string(),
                "jsx".to_string(),
            ],
        );
        type_map.insert(
            "typescript".to_string(),
            vec![
                "ts".to_string(),
                "mts".to_string(),
                "cts".to_string(),
                "tsx".to_string(),
            ],
        );
        type_map.insert(
            "ts".to_string(),
            vec![
                "ts".to_string(),
                "mts".to_string(),
                "cts".to_string(),
                "tsx".to_string(),
            ],
        );
        type_map.insert("go".to_string(), vec!["go".to_string()]);
        type_map.insert("c".to_string(), vec!["c".to_string(), "h".to_string()]);
        type_map.insert(
            "cpp".to_string(),
            vec![
                "cpp".to_string(),
                "cc".to_string(),
                "cxx".to_string(),
                "hpp".to_string(),
                "hh".to_string(),
                "hxx".to_string(),
            ],
        );
        type_map.insert(
            "c++".to_string(),
            vec![
                "cpp".to_string(),
                "cc".to_string(),
                "cxx".to_string(),
                "hpp".to_string(),
                "hh".to_string(),
                "hxx".to_string(),
            ],
        );
        type_map.insert("csharp".to_string(), vec!["cs".to_string()]);
        type_map.insert("cs".to_string(), vec!["cs".to_string()]);
        type_map.insert("java".to_string(), vec!["java".to_string()]);
        type_map.insert(
            "kotlin".to_string(),
            vec!["kt".to_string(), "kts".to_string()],
        );
        type_map.insert("swift".to_string(), vec!["swift".to_string()]);
        type_map.insert(
            "ruby".to_string(),
            vec!["rb".to_string(), "rake".to_string()],
        );
        type_map.insert(
            "php".to_string(),
            vec!["php".to_string(), "phtml".to_string()],
        );
        type_map.insert(
            "shell".to_string(),
            vec!["sh".to_string(), "bash".to_string(), "zsh".to_string()],
        );
        type_map.insert(
            "sh".to_string(),
            vec!["sh".to_string(), "bash".to_string(), "zsh".to_string()],
        );
        type_map.insert(
            "bash".to_string(),
            vec!["sh".to_string(), "bash".to_string(), "zsh".to_string()],
        );
        type_map.insert(
            "json".to_string(),
            vec!["json".to_string(), "jsonc".to_string(), "json5".to_string()],
        );
        type_map.insert(
            "yaml".to_string(),
            vec!["yaml".to_string(), "yml".to_string()],
        );
        type_map.insert(
            "yml".to_string(),
            vec!["yaml".to_string(), "yml".to_string()],
        );
        type_map.insert("toml".to_string(), vec!["toml".to_string()]);
        type_map.insert(
            "markdown".to_string(),
            vec!["md".to_string(), "markdown".to_string()],
        );
        type_map.insert(
            "md".to_string(),
            vec!["md".to_string(), "markdown".to_string()],
        );
        type_map.insert(
            "html".to_string(),
            vec!["html".to_string(), "htm".to_string()],
        );
        type_map.insert(
            "css".to_string(),
            vec![
                "css".to_string(),
                "scss".to_string(),
                "sass".to_string(),
                "less".to_string(),
            ],
        );
        type_map.insert("sql".to_string(), vec!["sql".to_string()]);
        type_map.insert(
            "xml".to_string(),
            vec!["xml".to_string(), "svg".to_string(), "plist".to_string()],
        );
        type_map.insert(
            "docker".to_string(),
            vec!["dockerfile".to_string(), "dockerignore".to_string()],
        );

        Self { type_map }
    }
}

impl FileTypeRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Resolve a type name or extension to a list of file extensions (lowercase, without leading dot).
    pub fn resolve_extensions(&self, type_or_ext: &str) -> Vec<String> {
        let normalized = type_or_ext.trim().to_ascii_lowercase();
        let stripped = normalized.strip_prefix('.').unwrap_or(&normalized);

        if let Some(exts) = self.type_map.get(stripped) {
            exts.clone()
        } else {
            vec![stripped.to_string()]
        }
    }

    /// Register a custom file type mapping.
    pub fn register_type(&mut self, type_name: &str, extensions: Vec<String>) {
        let exts = extensions
            .into_iter()
            .map(|ext| {
                let lower = ext.trim().to_ascii_lowercase();
                lower.strip_prefix('.').unwrap_or(&lower).to_string()
            })
            .collect();
        self.type_map.insert(type_name.to_ascii_lowercase(), exts);
    }
}

// ---------------------------------------------------------------------------
// PathFilter & PathFilterBuilder
// ---------------------------------------------------------------------------

/// Filter configuration for paths during grep operations.
#[derive(Debug, Clone)]
pub struct PathFilter {
    include_set: Option<GlobSet>,
    exclude_set: Option<GlobSet>,
    allowed_extensions: Option<HashSet<String>>,
    include_hidden: bool,
    min_file_size: Option<usize>,
    max_file_size: Option<usize>,
    max_depth: Option<usize>,
    raw_includes: Vec<String>,
    raw_excludes: Vec<String>,
}

/// Alias for `PathFilter`.
pub type GrepPathFilter = PathFilter;

/// Alias for `PathFilter`.
pub type GrepFilter = PathFilter;

/// Builder for constructing a `PathFilter`.
#[derive(Debug, Default, Clone)]
pub struct PathFilterBuilder {
    includes: Vec<String>,
    excludes: Vec<String>,
    file_types: Vec<String>,
    include_hidden: bool,
    case_sensitive: bool,
    min_file_size: Option<usize>,
    max_file_size: Option<usize>,
    max_depth: Option<usize>,
}

impl PathFilterBuilder {
    pub fn new() -> Self {
        Self {
            case_sensitive: true,
            ..Default::default()
        }
    }

    /// Add an include glob pattern (e.g. `*.rs`, `src/**/*.ts`).
    pub fn include(mut self, pattern: impl Into<String>) -> Self {
        let pat = pattern.into().trim().to_string();
        if !pat.is_empty() {
            self.includes.push(pat);
        }
        self
    }

    /// Add multiple include glob patterns.
    pub fn includes(mut self, patterns: impl IntoIterator<Item = String>) -> Self {
        for p in patterns {
            let pat = p.trim().to_string();
            if !pat.is_empty() {
                self.includes.push(pat);
            }
        }
        self
    }

    /// Add an exclude glob pattern (e.g. `target/**`, `node_modules/**`).
    pub fn exclude(mut self, pattern: impl Into<String>) -> Self {
        let pat = pattern.into().trim().to_string();
        if !pat.is_empty() {
            self.excludes.push(pat);
        }
        self
    }

    /// Add multiple exclude glob patterns.
    pub fn excludes(mut self, patterns: impl IntoIterator<Item = String>) -> Self {
        for p in patterns {
            let pat = p.trim().to_string();
            if !pat.is_empty() {
                self.excludes.push(pat);
            }
        }
        self
    }

    /// Filter by file type shortcut (e.g. "rust", "python", "json").
    pub fn file_type(mut self, file_type: impl Into<String>) -> Self {
        let ft = file_type.into().trim().to_string();
        if !ft.is_empty() {
            self.file_types.push(ft);
        }
        self
    }

    /// Filter by multiple file type shortcuts.
    pub fn file_types(mut self, file_types: impl IntoIterator<Item = String>) -> Self {
        for ft in file_types {
            let s = ft.trim().to_string();
            if !s.is_empty() {
                self.file_types.push(s);
            }
        }
        self
    }

    /// Include or ignore hidden files and directories.
    pub fn include_hidden(mut self, include_hidden: bool) -> Self {
        self.include_hidden = include_hidden;
        self
    }

    /// Set case-sensitivity for glob matching.
    pub fn case_sensitive(mut self, case_sensitive: bool) -> Self {
        self.case_sensitive = case_sensitive;
        self
    }

    /// Set minimum file size bound in bytes.
    pub fn min_file_size(mut self, size: usize) -> Self {
        self.min_file_size = Some(size);
        self
    }

    /// Set maximum file size bound in bytes.
    pub fn max_file_size(mut self, size: usize) -> Self {
        self.max_file_size = Some(size);
        self
    }

    /// Set maximum directory traversal depth.
    pub fn max_depth(mut self, depth: usize) -> Self {
        self.max_depth = Some(depth);
        self
    }

    /// Build the configured `PathFilter`.
    pub fn build(self) -> anyhow::Result<PathFilter> {
        let registry = FileTypeRegistry::new();
        let mut extensions = HashSet::new();

        for ft in &self.file_types {
            for ext in registry.resolve_extensions(ft) {
                extensions.insert(ext);
            }
        }

        let allowed_extensions = if extensions.is_empty() {
            None
        } else {
            Some(extensions)
        };

        // Build include GlobSet
        let include_set = if self.includes.is_empty() {
            None
        } else {
            let mut builder = GlobSetBuilder::new();
            for pat in &self.includes {
                let normalized = normalize_glob_pattern(pat);
                let glob = GlobBuilder::new(&normalized)
                    .case_insensitive(!self.case_sensitive)
                    .literal_separator(false)
                    .build()
                    .map_err(|e| anyhow::anyhow!("Invalid include glob '{}': {e}", pat))?;
                builder.add(glob);
            }
            Some(builder.build()?)
        };

        // Build exclude GlobSet
        let exclude_set = if self.excludes.is_empty() {
            None
        } else {
            let mut builder = GlobSetBuilder::new();
            for pat in &self.excludes {
                let normalized = normalize_glob_pattern(pat);
                let glob = GlobBuilder::new(&normalized)
                    .case_insensitive(!self.case_sensitive)
                    .literal_separator(false)
                    .build()
                    .map_err(|e| anyhow::anyhow!("Invalid exclude glob '{}': {e}", pat))?;
                builder.add(glob);
            }
            Some(builder.build()?)
        };

        Ok(PathFilter {
            include_set,
            exclude_set,
            allowed_extensions,
            include_hidden: self.include_hidden,
            min_file_size: self.min_file_size,
            max_file_size: self.max_file_size,
            max_depth: self.max_depth,
            raw_includes: self.includes,
            raw_excludes: self.excludes,
        })
    }
}

impl PathFilter {
    pub fn builder() -> PathFilterBuilder {
        PathFilterBuilder::new()
    }

    /// Check if a given file path meets all filter criteria.
    pub fn matches_file(
        &self,
        path: &Path,
        base_dir: &Path,
        metadata: Option<&fs::Metadata>,
    ) -> bool {
        // Hidden check
        if !self.include_hidden && is_hidden_path(path, base_dir) {
            return false;
        }

        // Depth check
        if let Some(max_depth) = self.max_depth {
            if let Ok(rel) = path.strip_prefix(base_dir) {
                let depth = rel.components().count();
                if depth > max_depth {
                    return false;
                }
            }
        }

        // Extension check
        if let Some(allowed) = &self.allowed_extensions {
            let ext = path
                .extension()
                .and_then(|e| e.to_str())
                .map(|e| e.to_ascii_lowercase())
                .unwrap_or_default();

            if !allowed.contains(&ext) {
                return false;
            }
        }

        // File size check
        if self.min_file_size.is_some() || self.max_file_size.is_some() {
            let size = if let Some(meta) = metadata {
                Some(meta.len() as usize)
            } else {
                fs::metadata(path).ok().map(|m| m.len() as usize)
            };

            if let Some(s) = size {
                if let Some(min_s) = self.min_file_size {
                    if s < min_s {
                        return false;
                    }
                }
                if let Some(max_s) = self.max_file_size {
                    if s > max_s {
                        return false;
                    }
                }
            }
        }

        // Glob matching relative to base_dir
        let rel_path = path.strip_prefix(base_dir).unwrap_or(path);

        // Check excludes first
        if let Some(exc) = &self.exclude_set {
            if exc.is_match(rel_path) || exc.is_match(path) {
                return false;
            }
        }

        // Check includes
        if let Some(inc) = &self.include_set {
            if !inc.is_match(rel_path) && !inc.is_match(path) {
                return false;
            }
        }

        true
    }

    /// Check if a directory path should be traversed or pruned based on excludes and depth.
    pub fn matches_dir(&self, path: &Path, base_dir: &Path) -> bool {
        if !self.include_hidden && is_hidden_path(path, base_dir) {
            return false;
        }

        if let Some(max_depth) = self.max_depth {
            if let Ok(rel) = path.strip_prefix(base_dir) {
                let depth = rel.components().count();
                if depth > max_depth {
                    return false;
                }
            }
        }

        let rel_path = path.strip_prefix(base_dir).unwrap_or(path);

        if let Some(exc) = &self.exclude_set {
            if exc.is_match(rel_path) || exc.is_match(path) {
                return false;
            }
        }

        true
    }

    pub fn raw_includes(&self) -> &[String] {
        &self.raw_includes
    }

    pub fn raw_excludes(&self) -> &[String] {
        &self.raw_excludes
    }

    pub fn allowed_extensions(&self) -> Option<&HashSet<String>> {
        self.allowed_extensions.as_ref()
    }
}

// ---------------------------------------------------------------------------
// Helpers for Path Normalization, Spans, and Filtering
// ---------------------------------------------------------------------------

fn normalize_glob_pattern(pattern: &str) -> String {
    let mut pat = pattern.trim().replace('\\', "/");
    if pat.starts_with("./") {
        pat = pat[2..].to_string();
    }
    pat
}

fn is_hidden_path(path: &Path, base_dir: &Path) -> bool {
    let rel = path.strip_prefix(base_dir).unwrap_or(path);
    for comp in rel.components() {
        if let std::path::Component::Normal(os_str) = comp {
            if let Some(s) = os_str.to_str() {
                if s.starts_with('.') && s != "." && s != ".." {
                    return true;
                }
            }
        }
    }
    false
}

/// Check if a byte slice contains null bytes (binary probe).
pub fn is_binary_content(bytes: &[u8]) -> bool {
    bytes.iter().take(4096).any(|&b| b == 0)
}

/// Truncate long line at UTF-8 character boundary.
pub fn truncate_line(line: &str, max_len: usize) -> String {
    if line.len() <= max_len {
        line.to_string()
    } else {
        let end = line
            .char_indices()
            .map(|(idx, _)| idx)
            .take_while(|&idx| idx <= max_len)
            .last()
            .unwrap_or(0);
        format!("{}...", &line[..end])
    }
}

/// Find all byte offset spans for a literal substring within a haystack.
pub fn find_substring_spans(
    haystack: &str,
    needle: &str,
    case_sensitive: bool,
) -> Vec<(usize, usize)> {
    if needle.is_empty() {
        return Vec::new();
    }
    let mut spans = Vec::new();
    if case_sensitive {
        let mut start = 0;
        while start <= haystack.len() {
            if let Some(pos) = haystack[start..].find(needle) {
                let actual_start = start + pos;
                let actual_end = actual_start + needle.len();
                spans.push((actual_start, actual_end));
                start = actual_end;
                if start >= haystack.len() {
                    break;
                }
            } else {
                break;
            }
        }
    } else {
        let lower_haystack = haystack.to_lowercase();
        let lower_needle = needle.to_lowercase();
        let mut start = 0;
        while start <= lower_haystack.len() {
            if let Some(pos) = lower_haystack[start..].find(&lower_needle) {
                let actual_start = start + pos;
                let actual_end = actual_start + lower_needle.len();
                spans.push((actual_start, actual_end));
                start = actual_end;
                if start >= lower_haystack.len() {
                    break;
                }
            } else {
                break;
            }
        }
    }
    spans
}

// ---------------------------------------------------------------------------
// Streaming Grep Filter Engine & Types
// ---------------------------------------------------------------------------

/// Options for configuring a streaming grep filter operation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StreamGrepOptions {
    pub pattern: String,
    pub case_sensitive: bool,
    pub invert_match: bool,
    pub fixed_strings: bool,
    pub multiline: bool,
    pub dot_matches_all: bool,
    pub line_numbers: bool,
    pub context_before: usize,
    pub context_after: usize,
    pub max_matches: usize,
    pub max_bytes: usize,
    pub max_line_length: usize,
    pub count_only: bool,
    pub source_name: Option<String>,
}

impl Default for StreamGrepOptions {
    fn default() -> Self {
        Self {
            pattern: String::new(),
            case_sensitive: true,
            invert_match: false,
            fixed_strings: false,
            multiline: false,
            dot_matches_all: false,
            line_numbers: true,
            context_before: 0,
            context_after: 0,
            max_matches: 200,
            max_bytes: 2 * 1024 * 1024, // 2 MB safety limit
            max_line_length: 1000,
            count_only: false,
            source_name: None,
        }
    }
}

impl StreamGrepOptions {
    pub fn new(pattern: impl Into<String>) -> Self {
        Self {
            pattern: pattern.into(),
            ..Default::default()
        }
    }

    pub fn case_sensitive(mut self, val: bool) -> Self {
        self.case_sensitive = val;
        self
    }

    pub fn invert_match(mut self, val: bool) -> Self {
        self.invert_match = val;
        self
    }

    pub fn fixed_strings(mut self, val: bool) -> Self {
        self.fixed_strings = val;
        self
    }

    pub fn multiline(mut self, val: bool) -> Self {
        self.multiline = val;
        self
    }

    pub fn dot_matches_all(mut self, val: bool) -> Self {
        self.dot_matches_all = val;
        self
    }

    pub fn line_numbers(mut self, val: bool) -> Self {
        self.line_numbers = val;
        self
    }

    pub fn context(mut self, val: usize) -> Self {
        self.context_before = val;
        self.context_after = val;
        self
    }

    pub fn context_before(mut self, val: usize) -> Self {
        self.context_before = val;
        self
    }

    pub fn context_after(mut self, val: usize) -> Self {
        self.context_after = val;
        self
    }

    pub fn max_matches(mut self, val: usize) -> Self {
        self.max_matches = val;
        self
    }

    pub fn max_bytes(mut self, val: usize) -> Self {
        self.max_bytes = val;
        self
    }

    pub fn max_line_length(mut self, val: usize) -> Self {
        self.max_line_length = val;
        self
    }

    pub fn count_only(mut self, val: bool) -> Self {
        self.count_only = val;
        self
    }

    pub fn source_name(mut self, val: impl Into<String>) -> Self {
        self.source_name = Some(val.into());
        self
    }

    /// Parse `StreamGrepOptions` from JSON parameters.
    pub fn from_json(args: &Value) -> anyhow::Result<Self> {
        let pattern = args
            .get("pattern")
            .and_then(|v| v.as_str())
            .or_else(|| args.get("regex").and_then(|v| v.as_str()))
            .or_else(|| args.get("query").and_then(|v| v.as_str()))
            .unwrap_or_default();

        let mut opts = Self::new(pattern);

        if let Some(cs) = args.get("case_sensitive").and_then(|v| v.as_bool()) {
            opts.case_sensitive = cs;
        } else if let Some(i) = args.get("i").and_then(|v| v.as_bool()) {
            opts.case_sensitive = !i;
        }

        if let Some(inv) = args
            .get("invert_match")
            .or_else(|| args.get("invert"))
            .or_else(|| args.get("v"))
            .and_then(|v| v.as_bool())
        {
            opts.invert_match = inv;
        }

        if let Some(f) = args
            .get("fixed_strings")
            .or_else(|| args.get("literal"))
            .or_else(|| args.get("F"))
            .and_then(|v| v.as_bool())
        {
            opts.fixed_strings = f;
        }

        if let Some(m) = args
            .get("multiline")
            .or_else(|| args.get("m"))
            .and_then(|v| v.as_bool())
        {
            opts.multiline = m;
        }

        if let Some(s) = args
            .get("dot_matches_all")
            .or_else(|| args.get("s"))
            .and_then(|v| v.as_bool())
        {
            opts.dot_matches_all = s;
        }

        if let Some(ln) = args
            .get("line_numbers")
            .or_else(|| args.get("show_line_numbers"))
            .or_else(|| args.get("n"))
            .and_then(|v| v.as_bool())
        {
            opts.line_numbers = ln;
        }

        if let Some(c) = args
            .get("context")
            .or_else(|| args.get("C"))
            .and_then(|v| v.as_u64())
        {
            opts.context_before = c as usize;
            opts.context_after = c as usize;
        }

        if let Some(b) = args
            .get("context_before")
            .or_else(|| args.get("before_context"))
            .or_else(|| args.get("B"))
            .and_then(|v| v.as_u64())
        {
            opts.context_before = b as usize;
        }

        if let Some(a) = args
            .get("context_after")
            .or_else(|| args.get("after_context"))
            .or_else(|| args.get("A"))
            .and_then(|v| v.as_u64())
        {
            opts.context_after = a as usize;
        }

        if let Some(mr) = args
            .get("max_results")
            .or_else(|| args.get("max_matches"))
            .or_else(|| args.get("limit"))
            .and_then(|v| v.as_u64())
        {
            opts.max_matches = mr as usize;
        }

        if let Some(mb) = args
            .get("max_bytes")
            .or_else(|| args.get("byte_limit"))
            .and_then(|v| v.as_u64())
        {
            opts.max_bytes = mb as usize;
        }

        if let Some(mll) = args
            .get("max_line_length")
            .or_else(|| args.get("line_limit"))
            .and_then(|v| v.as_u64())
        {
            opts.max_line_length = mll as usize;
        }

        if let Some(co) = args
            .get("count_only")
            .or_else(|| args.get("count"))
            .or_else(|| args.get("c"))
            .and_then(|v| v.as_bool())
        {
            opts.count_only = co;
        }

        if let Some(src) = args
            .get("source_name")
            .or_else(|| args.get("source"))
            .and_then(|v| v.as_str())
        {
            opts.source_name = Some(src.to_string());
        }

        Ok(opts)
    }
}

/// A single filtered line or separator in a stream filter result.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StreamFilteredLine {
    pub source: Option<String>,
    pub line_number: usize,
    pub content: String,
    pub is_match: bool,
    pub is_separator: bool,
    pub match_spans: Vec<(usize, usize)>,
}

impl StreamFilteredLine {
    /// Format this single line with optional line numbers and source name prefix.
    pub fn format_line(&self, show_line_numbers: bool, show_source: bool) -> String {
        if self.is_separator {
            return "--".to_string();
        }

        let sep = if self.is_match { ":" } else { "-" };
        let mut prefix = String::new();

        if show_source {
            if let Some(src) = &self.source {
                prefix.push_str(src);
                prefix.push_str(sep);
            }
        }

        if show_line_numbers && self.line_number > 0 {
            prefix.push_str(&self.line_number.to_string());
            prefix.push_str(sep);
        }

        if !prefix.is_empty() {
            format!("{} {}", prefix, self.content)
        } else {
            self.content.clone()
        }
    }
}

/// The result of executing a streaming grep filter operation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StreamFilterResult {
    pub lines: Vec<StreamFilteredLine>,
    pub total_matches: usize,
    pub lines_scanned: usize,
    pub bytes_scanned: usize,
    pub truncated_by_matches: bool,
    pub truncated_by_bytes: bool,
    pub max_matches: usize,
    pub max_bytes: usize,
    pub pattern: String,
    pub is_binary: bool,
}

impl StreamFilterResult {
    /// Format stream filter results into a clean string suitable for display.
    pub fn format_output(&self, options: &StreamGrepOptions) -> String {
        if self.is_binary {
            return format!(
                "Binary file or stream matches pattern '{}' (content omitted)",
                self.pattern
            );
        }

        if options.count_only {
            return format!(
                "Total matches: {} (scanned {} lines, {} bytes)",
                self.total_matches, self.lines_scanned, self.bytes_scanned
            );
        }

        if self.lines.is_empty() {
            return format!(
                "No matches found for pattern '{}' (scanned {} lines, {} bytes)",
                self.pattern, self.lines_scanned, self.bytes_scanned
            );
        }

        let show_source =
            options.source_name.is_some() || self.lines.iter().any(|l| l.source.is_some());
        let mut out = String::new();

        for line in &self.lines {
            if line.is_separator {
                out.push_str("--\n");
                continue;
            }

            let sep = if line.is_match { ":" } else { "-" };
            let mut prefix = String::new();

            if show_source {
                if let Some(src) = &line.source {
                    prefix.push_str(src);
                    prefix.push_str(sep);
                }
            }

            if options.line_numbers && line.line_number > 0 {
                prefix.push_str(&line.line_number.to_string());
                prefix.push_str(sep);
            }

            if !prefix.is_empty() {
                out.push_str(&prefix);
                out.push(' ');
            }
            out.push_str(&line.content);
            out.push('\n');
        }

        if self.truncated_by_matches {
            out.push_str(&format!(
                "\n... [matches limit reached; max_matches={}]\n",
                self.max_matches
            ));
        }

        if self.truncated_by_bytes {
            out.push_str(&format!(
                "\n... [output byte limit reached; max_bytes={}]\n",
                self.max_bytes
            ));
        }

        out.trim_end().to_string()
    }

    /// Format results into structured JSON value.
    pub fn to_json_value(&self) -> Value {
        json!({
            "pattern": self.pattern,
            "total_matches": self.total_matches,
            "lines_scanned": self.lines_scanned,
            "bytes_scanned": self.bytes_scanned,
            "truncated_by_matches": self.truncated_by_matches,
            "truncated_by_bytes": self.truncated_by_bytes,
            "max_matches": self.max_matches,
            "max_bytes": self.max_bytes,
            "is_binary": self.is_binary,
            "lines": self.lines.iter().map(|l| {
                json!({
                    "source": l.source,
                    "line_number": l.line_number,
                    "content": l.content,
                    "is_match": l.is_match,
                    "is_separator": l.is_separator,
                    "match_spans": l.match_spans,
                })
            }).collect::<Vec<_>>()
        })
    }
}

/// High-performance streaming grep filter for text streams, reader buffers, and files.
pub struct StreamGrepFilter {
    options: StreamGrepOptions,
    regex: Regex,
}

pub type StreamFilter = StreamGrepFilter;

impl StreamGrepFilter {
    /// Construct a new `StreamGrepFilter` from `StreamGrepOptions`.
    pub fn new(options: StreamGrepOptions) -> anyhow::Result<Self> {
        let regex_pattern = if options.fixed_strings {
            regex::escape(&options.pattern)
        } else {
            options.pattern.clone()
        };

        let regex = RegexBuilder::new(&regex_pattern)
            .case_insensitive(!options.case_sensitive)
            .multi_line(options.multiline)
            .dot_matches_new_line(options.dot_matches_all)
            .build()
            .map_err(|e| {
                anyhow::anyhow!("Invalid regular expression '{}': {e}", options.pattern)
            })?;

        Ok(Self { options, regex })
    }

    pub fn options(&self) -> &StreamGrepOptions {
        &self.options
    }

    /// Filter an arbitrary `BufRead` stream, outputting matches and context lines.
    pub fn filter_reader<R: BufRead>(
        &self,
        mut reader: R,
        source_name: Option<&str>,
    ) -> anyhow::Result<StreamFilterResult> {
        let mut byte_buffer = Vec::new();
        let mut line_number = 0;
        let mut total_matches = 0;
        let mut lines_scanned = 0;
        let mut bytes_scanned = 0;
        let mut output_bytes = 0;
        let mut truncated_by_matches = false;
        let mut truncated_by_bytes = false;
        let mut is_binary = false;

        let before = self.options.context_before;
        let after = self.options.context_after;
        let mut before_buffer: VecDeque<(usize, String)> = VecDeque::with_capacity(before.max(1));
        let mut after_remaining: usize = 0;
        let mut last_emitted_line: Option<usize> = None;
        let mut emitted_lines: Vec<StreamFilteredLine> = Vec::new();

        let src_string = source_name
            .or(self.options.source_name.as_deref())
            .map(str::to_string);

        while reader.read_until(b'\n', &mut byte_buffer)? > 0 {
            line_number += 1;
            lines_scanned += 1;
            let raw_len = byte_buffer.len();
            bytes_scanned += raw_len;

            // Probe first 4096 bytes for null bytes
            if (line_number == 1 || bytes_scanned <= 4096) && byte_buffer.contains(&0) {
                is_binary = true;
                break;
            }

            let lossy_str = String::from_utf8_lossy(&byte_buffer);
            let trimmed_line = lossy_str.trim_end_matches(&['\r', '\n'][..]);

            let is_match = if self.options.fixed_strings {
                if self.options.case_sensitive {
                    trimmed_line.contains(&self.options.pattern)
                } else {
                    trimmed_line
                        .to_lowercase()
                        .contains(&self.options.pattern.to_lowercase())
                }
            } else {
                self.regex.is_match(trimmed_line)
            };

            let effective_match = if self.options.invert_match {
                !is_match
            } else {
                is_match
            };

            let truncated_content = truncate_line(trimmed_line, self.options.max_line_length);

            if effective_match {
                total_matches += 1;

                if !self.options.count_only {
                    let match_spans = if !self.options.invert_match {
                        if self.options.fixed_strings {
                            find_substring_spans(
                                trimmed_line,
                                &self.options.pattern,
                                self.options.case_sensitive,
                            )
                        } else {
                            self.regex
                                .find_iter(trimmed_line)
                                .map(|m| (m.start(), m.end()))
                                .collect()
                        }
                    } else {
                        Vec::new()
                    };

                    // Emit pending before-context lines
                    if before > 0 {
                        for (b_num, b_line) in before_buffer.drain(..) {
                            let is_already_emitted =
                                last_emitted_line.map(|l| b_num <= l).unwrap_or(false);
                            if !is_already_emitted {
                                let has_gap =
                                    last_emitted_line.map(|l| b_num > l + 1).unwrap_or(false);
                                if has_gap && !emitted_lines.is_empty() && (before > 0 || after > 0)
                                {
                                    emitted_lines.push(StreamFilteredLine {
                                        source: src_string.clone(),
                                        line_number: 0,
                                        content: "--".to_string(),
                                        is_match: false,
                                        is_separator: true,
                                        match_spans: Vec::new(),
                                    });
                                    output_bytes += 3;
                                }

                                let b_trunc = truncate_line(&b_line, self.options.max_line_length);
                                output_bytes += b_trunc.len() + 20;
                                emitted_lines.push(StreamFilteredLine {
                                    source: src_string.clone(),
                                    line_number: b_num,
                                    content: b_trunc,
                                    is_match: false,
                                    is_separator: false,
                                    match_spans: Vec::new(),
                                });
                                last_emitted_line = Some(b_num);
                            }
                        }
                    }

                    // Emit match line
                    let has_gap = last_emitted_line
                        .map(|l| line_number > l + 1)
                        .unwrap_or(false);
                    if has_gap && !emitted_lines.is_empty() && (before > 0 || after > 0) {
                        emitted_lines.push(StreamFilteredLine {
                            source: src_string.clone(),
                            line_number: 0,
                            content: "--".to_string(),
                            is_match: false,
                            is_separator: true,
                            match_spans: Vec::new(),
                        });
                        output_bytes += 3;
                    }

                    output_bytes += truncated_content.len() + 20;
                    emitted_lines.push(StreamFilteredLine {
                        source: src_string.clone(),
                        line_number,
                        content: truncated_content,
                        is_match: true,
                        is_separator: false,
                        match_spans,
                    });
                    last_emitted_line = Some(line_number);
                    after_remaining = after;

                    if output_bytes >= self.options.max_bytes {
                        truncated_by_bytes = true;
                        break;
                    }

                    if total_matches >= self.options.max_matches {
                        truncated_by_matches = true;
                        break;
                    }
                } else if total_matches >= self.options.max_matches {
                    truncated_by_matches = true;
                    break;
                }
            } else if !self.options.count_only {
                if after_remaining > 0 {
                    after_remaining -= 1;
                    let has_gap = last_emitted_line
                        .map(|l| line_number > l + 1)
                        .unwrap_or(false);
                    if has_gap && !emitted_lines.is_empty() && (before > 0 || after > 0) {
                        emitted_lines.push(StreamFilteredLine {
                            source: src_string.clone(),
                            line_number: 0,
                            content: "--".to_string(),
                            is_match: false,
                            is_separator: true,
                            match_spans: Vec::new(),
                        });
                        output_bytes += 3;
                    }

                    output_bytes += truncated_content.len() + 20;
                    emitted_lines.push(StreamFilteredLine {
                        source: src_string.clone(),
                        line_number,
                        content: truncated_content.clone(),
                        is_match: false,
                        is_separator: false,
                        match_spans: Vec::new(),
                    });
                    last_emitted_line = Some(line_number);

                    if output_bytes >= self.options.max_bytes {
                        truncated_by_bytes = true;
                        break;
                    }
                }

                if before > 0 {
                    if before_buffer.len() >= before {
                        before_buffer.pop_front();
                    }
                    before_buffer.push_back((line_number, trimmed_line.to_string()));
                }
            }

            byte_buffer.clear();
        }

        Ok(StreamFilterResult {
            lines: emitted_lines,
            total_matches,
            lines_scanned,
            bytes_scanned,
            truncated_by_matches,
            truncated_by_bytes,
            max_matches: self.options.max_matches,
            max_bytes: self.options.max_bytes,
            pattern: self.options.pattern.clone(),
            is_binary,
        })
    }

    /// Filter an in-memory string slice.
    pub fn filter_str(
        &self,
        input: &str,
        source_name: Option<&str>,
    ) -> anyhow::Result<StreamFilterResult> {
        let cursor = Cursor::new(input.as_bytes());
        self.filter_reader(cursor, source_name)
    }

    /// Filter a single file on disk using buffered reading.
    pub fn filter_file(&self, path: &Path) -> anyhow::Result<StreamFilterResult> {
        let source_name = path.to_string_lossy().to_string();
        self.filter_file_with_source(path, Some(&source_name))
    }

    /// Filter a single file on disk with custom source name prefix.
    pub fn filter_file_with_source(
        &self,
        path: &Path,
        source_name: Option<&str>,
    ) -> anyhow::Result<StreamFilterResult> {
        let file = File::open(path)
            .map_err(|e| anyhow::anyhow!("Failed to open file '{}': {e}", path.display()))?;
        let reader = BufReader::new(file);
        self.filter_reader(reader, source_name)
    }

    /// Filter multiple file paths in sequence, merging results and tracking limits.
    pub fn filter_paths(
        &self,
        paths: &[PathBuf],
        base_dir: Option<&Path>,
    ) -> anyhow::Result<StreamFilterResult> {
        let mut combined_lines = Vec::new();
        let mut total_matches = 0;
        let mut lines_scanned = 0;
        let mut bytes_scanned = 0;
        let mut truncated_by_matches = false;
        let mut truncated_by_bytes = false;
        let mut is_binary = false;

        for path in paths {
            let source_name = if let Some(base) = base_dir {
                path.strip_prefix(base)
                    .unwrap_or(path)
                    .to_string_lossy()
                    .to_string()
            } else {
                path.to_string_lossy().to_string()
            };

            let file_res = match self.filter_file_with_source(path, Some(&source_name)) {
                Ok(res) => res,
                Err(e) => {
                    tracing::debug!("Skip unreadable file {}: {e}", path.display());
                    continue;
                }
            };

            lines_scanned += file_res.lines_scanned;
            bytes_scanned += file_res.bytes_scanned;
            total_matches += file_res.total_matches;

            if file_res.is_binary {
                is_binary = true;
            }

            combined_lines.extend(file_res.lines);

            if file_res.truncated_by_matches || total_matches >= self.options.max_matches {
                truncated_by_matches = true;
                break;
            }
            if file_res.truncated_by_bytes {
                truncated_by_bytes = true;
                break;
            }
        }

        Ok(StreamFilterResult {
            lines: combined_lines,
            total_matches,
            lines_scanned,
            bytes_scanned,
            truncated_by_matches,
            truncated_by_bytes,
            max_matches: self.options.max_matches,
            max_bytes: self.options.max_bytes,
            pattern: self.options.pattern.clone(),
            is_binary,
        })
    }
}

// ---------------------------------------------------------------------------
// Grep Options & Engine
// ---------------------------------------------------------------------------

/// Options for configuring a grep search operation.
#[derive(Debug, Clone)]
pub struct GrepOptions {
    pub pattern: String,
    pub search_path: PathBuf,
    pub cwd: PathBuf,
    pub case_sensitive: bool,
    pub include_hidden: bool,
    pub max_results: usize,
    pub include_globs: Vec<String>,
    pub exclude_globs: Vec<String>,
    pub file_types: Vec<String>,
    pub min_file_size: Option<usize>,
    pub max_file_size: Option<usize>,
    pub max_depth: Option<usize>,
    pub context_before: usize,
    pub context_after: usize,
    pub invert_match: bool,
    pub fixed_strings: bool,
    pub multiline: bool,
    pub line_numbers: bool,
    pub count_only: bool,
    pub files_with_matches: bool,
    pub files_without_matches: bool,
}

impl Default for GrepOptions {
    fn default() -> Self {
        Self {
            pattern: String::new(),
            search_path: PathBuf::from("."),
            cwd: PathBuf::from("."),
            case_sensitive: true,
            include_hidden: false,
            max_results: 200,
            include_globs: Vec::new(),
            exclude_globs: Vec::new(),
            file_types: Vec::new(),
            min_file_size: None,
            max_file_size: Some(20 * 1024 * 1024), // 20 MB default limit
            max_depth: None,
            context_before: 0,
            context_after: 0,
            invert_match: false,
            fixed_strings: false,
            multiline: false,
            line_numbers: true,
            count_only: false,
            files_with_matches: false,
            files_without_matches: false,
        }
    }
}

impl GrepOptions {
    pub fn new(pattern: impl Into<String>) -> Self {
        Self {
            pattern: pattern.into(),
            ..Default::default()
        }
    }

    /// Parse GrepOptions from JSON parameters.
    pub fn from_json(args: &Value, cwd: &Path) -> anyhow::Result<Self> {
        let pattern = args
            .get("pattern")
            .and_then(|v| v.as_str())
            .or_else(|| args.get("regex").and_then(|v| v.as_str()))
            .or_else(|| args.get("query").and_then(|v| v.as_str()))
            .ok_or_else(|| anyhow::anyhow!("Missing required parameter: pattern"))?;

        if pattern.is_empty() {
            anyhow::bail!("Search pattern cannot be empty");
        }

        let mut opts = Self::new(pattern);
        opts.cwd = cwd.to_path_buf();

        // Search path
        let path_str = args
            .get("path")
            .and_then(|v| v.as_str())
            .or_else(|| args.get("dir").and_then(|v| v.as_str()))
            .or_else(|| args.get("file").and_then(|v| v.as_str()));

        opts.search_path = match path_str {
            Some(p) => crate::tools::file::resolve_path(p, cwd),
            None => cwd.to_path_buf(),
        };

        if let Some(cs) = args.get("case_sensitive").and_then(|v| v.as_bool()) {
            opts.case_sensitive = cs;
        }

        if let Some(h) = args.get("hidden").and_then(|v| v.as_bool()) {
            opts.include_hidden = h;
        }

        if let Some(mr) = args.get("max_results").and_then(|v| v.as_u64()) {
            opts.max_results = mr as usize;
        }

        // Includes: supports string or array of strings (keys: include, includes, glob, filter)
        if let Some(inc_val) = args
            .get("include")
            .or_else(|| args.get("includes"))
            .or_else(|| args.get("glob"))
            .or_else(|| args.get("filter"))
        {
            if let Some(s) = inc_val.as_str() {
                opts.include_globs.push(s.to_string());
            } else if let Some(arr) = inc_val.as_array() {
                for item in arr {
                    if let Some(s) = item.as_str() {
                        opts.include_globs.push(s.to_string());
                    }
                }
            }
        }

        // Excludes: supports string or array of strings (keys: exclude, excludes, ignore)
        if let Some(exc_val) = args
            .get("exclude")
            .or_else(|| args.get("excludes"))
            .or_else(|| args.get("ignore"))
        {
            if let Some(s) = exc_val.as_str() {
                opts.exclude_globs.push(s.to_string());
            } else if let Some(arr) = exc_val.as_array() {
                for item in arr {
                    if let Some(s) = item.as_str() {
                        opts.exclude_globs.push(s.to_string());
                    }
                }
            }
        }

        // File types: (keys: type, types, file_type, file_types)
        if let Some(ft_val) = args
            .get("type")
            .or_else(|| args.get("types"))
            .or_else(|| args.get("file_type"))
            .or_else(|| args.get("file_types"))
        {
            if let Some(s) = ft_val.as_str() {
                opts.file_types.push(s.to_string());
            } else if let Some(arr) = ft_val.as_array() {
                for item in arr {
                    if let Some(s) = item.as_str() {
                        opts.file_types.push(s.to_string());
                    }
                }
            }
        }

        // Context lines
        if let Some(c) = args
            .get("context")
            .or_else(|| args.get("C"))
            .and_then(|v| v.as_u64())
        {
            opts.context_before = c as usize;
            opts.context_after = c as usize;
        }
        if let Some(b) = args
            .get("context_before")
            .or_else(|| args.get("before_context"))
            .or_else(|| args.get("B"))
            .and_then(|v| v.as_u64())
        {
            opts.context_before = b as usize;
        }
        if let Some(a) = args
            .get("context_after")
            .or_else(|| args.get("after_context"))
            .or_else(|| args.get("A"))
            .and_then(|v| v.as_u64())
        {
            opts.context_after = a as usize;
        }

        // Invert match
        if let Some(inv) = args
            .get("invert_match")
            .or_else(|| args.get("invert"))
            .or_else(|| args.get("v"))
            .and_then(|v| v.as_bool())
        {
            opts.invert_match = inv;
        }

        // Fixed strings
        if let Some(f) = args
            .get("fixed_strings")
            .or_else(|| args.get("literal"))
            .or_else(|| args.get("F"))
            .and_then(|v| v.as_bool())
        {
            opts.fixed_strings = f;
        }

        // Multiline
        if let Some(m) = args.get("multiline").and_then(|v| v.as_bool()) {
            opts.multiline = m;
        }

        // Max depth
        if let Some(d) = args
            .get("max_depth")
            .or_else(|| args.get("depth"))
            .and_then(|v| v.as_u64())
        {
            opts.max_depth = Some(d as usize);
        }

        // File size constraints
        if let Some(max_s) = args.get("max_file_size").and_then(|v| v.as_u64()) {
            opts.max_file_size = Some(max_s as usize);
        }
        if let Some(min_s) = args.get("min_file_size").and_then(|v| v.as_u64()) {
            opts.min_file_size = Some(min_s as usize);
        }

        // Output mode flags
        if let Some(c) = args
            .get("count_only")
            .or_else(|| args.get("count"))
            .and_then(|v| v.as_bool())
        {
            opts.count_only = c;
        }
        if let Some(f) = args
            .get("files_with_matches")
            .or_else(|| args.get("files_only"))
            .or_else(|| args.get("l"))
            .and_then(|v| v.as_bool())
        {
            opts.files_with_matches = f;
        }
        if let Some(f) = args
            .get("files_without_matches")
            .or_else(|| args.get("L"))
            .and_then(|v| v.as_bool())
        {
            opts.files_without_matches = f;
        }

        Ok(opts)
    }
}

/// A matched line or context line from a grep search.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct GrepMatch {
    pub file: String,
    pub line_number: usize,
    pub line: String,
    pub is_context: bool,
}

/// The result of executing a filterable grep search.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GrepSearchResult {
    pub matches: Vec<GrepMatch>,
    pub files_searched: usize,
    pub files_matched: usize,
    pub total_matches: usize,
    pub truncated: bool,
    pub max_results: usize,
    pub matched_files: Vec<String>,
}

impl GrepSearchResult {
    /// Format search results into a clean string suitable for tool execution outputs.
    pub fn format_output(
        &self,
        search_path_display: &str,
        pattern: &str,
        options: &GrepOptions,
    ) -> String {
        if options.files_with_matches {
            if self.matched_files.is_empty() {
                return format!(
                    "No files matched pattern '{}' in '{}'",
                    pattern, search_path_display
                );
            }
            return self.matched_files.join("\n");
        }

        if options.files_without_matches {
            if self.matched_files.is_empty() {
                return format!(
                    "All searched files matched pattern '{}' in '{}'",
                    pattern, search_path_display
                );
            }
            return self.matched_files.join("\n");
        }

        if options.count_only {
            return format!(
                "Total matches: {} across {} files",
                self.total_matches, self.files_matched
            );
        }

        if self.matches.is_empty() {
            return format!(
                "No matches found for regex '{}' in '{}'",
                pattern, search_path_display
            );
        }

        let mut lines = Vec::new();
        let mut last_file: Option<&str> = None;
        let mut last_line_num: usize = 0;

        for m in &self.matches {
            let file_changed = last_file.map(|f| f != m.file.as_str()).unwrap_or(true);
            let has_gap = !file_changed
                && (m.line_number > last_line_num + 1)
                && (options.context_before > 0 || options.context_after > 0);

            if has_gap && !lines.is_empty() {
                lines.push("--".to_string());
            }

            let separator = if m.is_context { "-" } else { ":" };
            let formatted_line = truncate_line(&m.line, 400);

            if options.line_numbers {
                lines.push(format!(
                    "{}{}{}: {}",
                    m.file, separator, m.line_number, formatted_line
                ));
            } else {
                lines.push(format!("{}{}: {}", m.file, separator, formatted_line));
            }

            last_file = Some(&m.file);
            last_line_num = m.line_number;
        }

        let mut output = lines.join("\n");
        if self.truncated {
            output.push_str(&format!(
                "\n\n... [{} additional matches truncated; max_results={}]",
                self.total_matches
                    .saturating_sub(self.matches.iter().filter(|m| !m.is_context).count()),
                self.max_results
            ));
        }

        output
    }

    /// Format results into structured JSON.
    pub fn to_json_value(&self) -> Value {
        json!({
            "matches": self.matches,
            "files_searched": self.files_searched,
            "files_matched": self.files_matched,
            "total_matches": self.total_matches,
            "truncated": self.truncated,
            "matched_files": self.matched_files,
        })
    }
}

// ---------------------------------------------------------------------------
// FilterableGrepEngine Implementation
// ---------------------------------------------------------------------------

/// Filterable grep engine that applies advanced path include/exclude glob filters.
pub struct FilterableGrepEngine {
    options: GrepOptions,
    filter: PathFilter,
    regex: Regex,
}

impl FilterableGrepEngine {
    /// Create a new engine instance from `GrepOptions`.
    pub fn new(options: GrepOptions) -> anyhow::Result<Self> {
        let mut filter_builder = PathFilter::builder()
            .case_sensitive(options.case_sensitive)
            .include_hidden(options.include_hidden)
            .includes(options.include_globs.clone())
            .excludes(options.exclude_globs.clone())
            .file_types(options.file_types.clone());

        if let Some(min_s) = options.min_file_size {
            filter_builder = filter_builder.min_file_size(min_s);
        }
        if let Some(max_s) = options.max_file_size {
            filter_builder = filter_builder.max_file_size(max_s);
        }
        if let Some(d) = options.max_depth {
            filter_builder = filter_builder.max_depth(d);
        }

        let filter = filter_builder.build()?;

        let regex_pattern = if options.fixed_strings {
            regex::escape(&options.pattern)
        } else {
            options.pattern.clone()
        };

        let regex = RegexBuilder::new(&regex_pattern)
            .case_insensitive(!options.case_sensitive)
            .multi_line(options.multiline)
            .build()
            .map_err(|e| {
                anyhow::anyhow!("Invalid regular expression '{}': {e}", options.pattern)
            })?;

        Ok(Self {
            options,
            filter,
            regex,
        })
    }

    /// Execute search synchronously across the configured path.
    pub fn search(&self) -> anyhow::Result<GrepSearchResult> {
        if !self.options.search_path.exists() {
            anyhow::bail!("Path not found: '{}'", self.options.search_path.display());
        }

        let mut matches = Vec::new();
        let mut matched_files_set = HashSet::new();
        let mut non_matched_files = Vec::new();
        let mut files_searched = 0;
        let mut total_matches = 0;
        let max_results = self.options.max_results;

        if self.options.search_path.is_file() {
            let metadata = fs::metadata(&self.options.search_path).ok();
            if self.filter.matches_file(
                &self.options.search_path,
                &self.options.cwd,
                metadata.as_ref(),
            ) {
                files_searched += 1;
                let (file_matches, count) = self.search_file(&self.options.search_path);
                total_matches += count;
                if count > 0 {
                    let rel = self
                        .options
                        .search_path
                        .strip_prefix(&self.options.cwd)
                        .unwrap_or(&self.options.search_path);
                    matched_files_set.insert(rel.to_string_lossy().to_string());
                    matches.extend(file_matches);
                } else {
                    let rel = self
                        .options
                        .search_path
                        .strip_prefix(&self.options.cwd)
                        .unwrap_or(&self.options.search_path);
                    non_matched_files.push(rel.to_string_lossy().to_string());
                }
            }
        } else {
            // Build WalkBuilder with ignore rules and overrides
            let mut walk_builder = WalkBuilder::new(&self.options.search_path);
            walk_builder
                .hidden(!self.options.include_hidden)
                .git_ignore(true)
                .git_global(true)
                .git_exclude(true)
                .require_git(false)
                .parents(true);

            if let Some(depth) = self.options.max_depth {
                walk_builder.max_depth(Some(depth));
            }

            // Attempt to install fast overrides for walk prune if include/exclude globs are present
            if !self.filter.raw_includes().is_empty() || !self.filter.raw_excludes().is_empty() {
                let mut ov = OverrideBuilder::new(&self.options.search_path);
                for inc in self.filter.raw_includes() {
                    let _ = ov.add(inc);
                }
                for exc in self.filter.raw_excludes() {
                    let _ = ov.add(&format!("!{}", exc));
                }
                if let Ok(overrides) = ov.build() {
                    walk_builder.overrides(overrides);
                }
            }

            for entry_result in walk_builder.build() {
                let entry = match entry_result {
                    Ok(e) => e,
                    Err(e) => {
                        tracing::debug!("Grep walk error: {e}");
                        continue;
                    }
                };

                let path = entry.path();
                let is_dir = entry.file_type().map(|ft| ft.is_dir()).unwrap_or(false);

                if is_dir {
                    continue;
                }

                let metadata = entry.metadata().ok();

                // Check filter rules
                if !self
                    .filter
                    .matches_file(path, &self.options.cwd, metadata.as_ref())
                {
                    continue;
                }

                files_searched += 1;
                let (file_matches, count) = self.search_file(path);
                total_matches += count;

                let rel = path.strip_prefix(&self.options.cwd).unwrap_or(path);
                let rel_str = rel.to_string_lossy().to_string();

                if count > 0 {
                    matched_files_set.insert(rel_str);
                    for m in file_matches {
                        if matches.iter().filter(|x| !x.is_context).count() < max_results
                            || m.is_context
                        {
                            matches.push(m);
                        }
                    }
                } else {
                    non_matched_files.push(rel_str);
                }

                // If we've collected enough primary matches and don't need all counts or file lists, break early
                if !self.options.count_only
                    && !self.options.files_with_matches
                    && !self.options.files_without_matches
                {
                    if matches.iter().filter(|m| !m.is_context).count() >= max_results {
                        break;
                    }
                }
            }
        }

        let primary_matches = matches.iter().filter(|m| !m.is_context).count();
        let truncated = total_matches > primary_matches && primary_matches >= max_results;

        let mut matched_files: Vec<String> = if self.options.files_without_matches {
            non_matched_files
        } else {
            matched_files_set.into_iter().collect()
        };
        matched_files.sort();

        Ok(GrepSearchResult {
            matches,
            files_searched,
            files_matched: matched_files.len(),
            total_matches,
            truncated,
            max_results,
            matched_files,
        })
    }

    /// Search a single file, extracting matches and optional before/after context lines.
    fn search_file(&self, file_path: &Path) -> (Vec<GrepMatch>, usize) {
        let bytes = match fs::read(file_path) {
            Ok(b) => b,
            Err(_) => return (Vec::new(), 0),
        };

        if is_binary_content(&bytes) {
            return (Vec::new(), 0);
        }

        let text = match std::str::from_utf8(&bytes) {
            Ok(s) => s,
            Err(_) => return (Vec::new(), 0),
        };

        let rel_path = file_path
            .strip_prefix(&self.options.cwd)
            .unwrap_or(file_path);
        let rel_path_str = rel_path.to_string_lossy().to_string();

        let lines: Vec<&str> = text.lines().collect();
        let mut match_line_indices = Vec::new();

        for (idx, line) in lines.iter().enumerate() {
            let is_match = self.regex.is_match(line);
            let effective_match = if self.options.invert_match {
                !is_match
            } else {
                is_match
            };

            if effective_match {
                match_line_indices.push(idx);
            }
        }

        let total_count = match_line_indices.len();
        if total_count == 0 {
            return (Vec::new(), 0);
        }

        // If no context lines are needed, directly construct matches
        if self.options.context_before == 0 && self.options.context_after == 0 {
            let mut result_matches = Vec::with_capacity(match_line_indices.len());
            for &idx in &match_line_indices {
                result_matches.push(GrepMatch {
                    file: rel_path_str.clone(),
                    line_number: idx + 1,
                    line: lines[idx].to_string(),
                    is_context: false,
                });
            }
            return (result_matches, total_count);
        }

        // Context line tracking
        let before = self.options.context_before;
        let after = self.options.context_after;
        let mut included_indices = HashSet::new();

        for &idx in &match_line_indices {
            let start = idx.saturating_sub(before);
            let end = (idx + after + 1).min(lines.len());
            for i in start..end {
                included_indices.insert(i);
            }
        }

        let mut sorted_indices: Vec<usize> = included_indices.into_iter().collect();
        sorted_indices.sort_unstable();

        let match_set: HashSet<usize> = match_line_indices.iter().copied().collect();
        let mut result_matches = Vec::with_capacity(sorted_indices.len());

        for idx in sorted_indices {
            let is_match = match_set.contains(&idx);
            result_matches.push(GrepMatch {
                file: rel_path_str.clone(),
                line_number: idx + 1,
                line: lines[idx].to_string(),
                is_context: !is_match,
            });
        }

        (result_matches, total_count)
    }
}

// ---------------------------------------------------------------------------
// GrepFilterTool Implementation
// ---------------------------------------------------------------------------

/// Streaming grep filter tool for agents.
#[derive(Default, Debug, Clone)]
pub struct GrepFilterTool;

pub type StreamGrepTool = GrepFilterTool;

impl GrepFilterTool {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl Tool for GrepFilterTool {
    fn name(&self) -> &str {
        "grep_filter"
    }

    fn description(&self) -> &str {
        "High-performance streaming grep filter for large text streams, log files, and codebase searching. Supports regular expressions, context lines (-A, -B, -C), inverted matching (-v), line numbers, file type and glob filtering, and output truncation safety limits."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "pattern": {
                    "type": "string",
                    "description": "Regular expression pattern or literal text to search for."
                },
                "input": {
                    "type": "string",
                    "description": "Raw text or log stream content to filter directly (optional)."
                },
                "text": {
                    "type": "string",
                    "description": "Alias for input text content (optional)."
                },
                "path": {
                    "type": "string",
                    "description": "Directory or file path to search within (optional, defaults to workspace root)."
                },
                "file": {
                    "type": "string",
                    "description": "Alias for path (optional)."
                },
                "include": {
                    "description": "Glob pattern or array of glob patterns to include (e.g. '*.rs', ['src/**/*.ts', 'tests/**/*.ts']).",
                    "oneOf": [
                        { "type": "string" },
                        { "type": "array", "items": { "type": "string" } }
                    ]
                },
                "exclude": {
                    "description": "Glob pattern or array of glob patterns to exclude (e.g. 'target/**', ['*.min.js', 'vendor/**']).",
                    "oneOf": [
                        { "type": "string" },
                        { "type": "array", "items": { "type": "string" } }
                    ]
                },
                "type": {
                    "description": "File type shortcut or array of types to filter (e.g. 'rust', 'python', 'typescript', 'json', 'toml').",
                    "oneOf": [
                        { "type": "string" },
                        { "type": "array", "items": { "type": "string" } }
                    ]
                },
                "case_sensitive": {
                    "type": "boolean",
                    "description": "Whether the search is case-sensitive (optional, default: true)."
                },
                "invert_match": {
                    "type": "boolean",
                    "description": "Invert match: select non-matching lines (optional, default: false)."
                },
                "fixed_strings": {
                    "type": "boolean",
                    "description": "Treat pattern as a literal fixed string instead of a regular expression (optional, default: false)."
                },
                "context": {
                    "type": "integer",
                    "description": "Number of lines of context before and after each match (optional, default: 0)."
                },
                "context_before": {
                    "type": "integer",
                    "description": "Number of lines of context before each match (optional, default: 0)."
                },
                "context_after": {
                    "type": "integer",
                    "description": "Number of lines of context after each match (optional, default: 0)."
                },
                "line_numbers": {
                    "type": "boolean",
                    "description": "Whether to include line numbers in the output (optional, default: true)."
                },
                "max_results": {
                    "type": "integer",
                    "description": "Maximum number of matching lines to return (optional, default: 200)."
                },
                "max_bytes": {
                    "type": "integer",
                    "description": "Maximum output size in bytes before truncating (optional, default: 2097152 / 2MB)."
                },
                "max_line_length": {
                    "type": "integer",
                    "description": "Maximum character length for individual output lines (optional, default: 1000)."
                },
                "count_only": {
                    "type": "boolean",
                    "description": "Only output total match count (optional, default: false)."
                },
                "format": {
                    "type": "string",
                    "enum": ["plain", "json"],
                    "description": "Output format: 'plain' text or structured 'json' (optional, default: 'plain')."
                }
            },
            "required": ["pattern"]
        })
    }

    async fn execute(&self, args: Value, ctx: &ToolContext) -> anyhow::Result<String> {
        let pattern = args
            .get("pattern")
            .and_then(|v| v.as_str())
            .or_else(|| args.get("regex").and_then(|v| v.as_str()))
            .or_else(|| args.get("query").and_then(|v| v.as_str()))
            .ok_or_else(|| anyhow::anyhow!("Missing required parameter: pattern"))?;

        if pattern.is_empty() {
            anyhow::bail!("Search pattern cannot be empty");
        }

        let input_text = args
            .get("input")
            .and_then(|v| v.as_str())
            .or_else(|| args.get("text").and_then(|v| v.as_str()))
            .map(str::to_string);

        let path_opt = args
            .get("path")
            .and_then(|v| v.as_str())
            .or_else(|| args.get("file").and_then(|v| v.as_str()))
            .or_else(|| args.get("dir").and_then(|v| v.as_str()));

        let format_json = args
            .get("format")
            .and_then(|v| v.as_str())
            .map(|f| f.eq_ignore_ascii_case("json"))
            .unwrap_or(false);

        let stream_opts = StreamGrepOptions::from_json(&args)?;

        // If raw input text was provided directly, filter stream in memory
        if let Some(text) = input_text {
            let opts_clone = stream_opts.clone();
            let res = tokio::task::spawn_blocking(move || -> anyhow::Result<StreamFilterResult> {
                let filter = StreamGrepFilter::new(opts_clone)?;
                filter.filter_str(&text, None)
            })
            .await
            .map_err(|e| anyhow::anyhow!("Stream filter task join failed: {e}"))??;

            if format_json {
                return Ok(serde_json::to_string_pretty(&res.to_json_value())?);
            } else {
                return Ok(res.format_output(&stream_opts));
            }
        }

        // Otherwise, resolve search path
        let target_path = match path_opt {
            Some(p) => crate::tools::file::resolve_path(p, &ctx.cwd),
            None => ctx.cwd.clone(),
        };

        if !target_path.exists() {
            anyhow::bail!("Path not found: '{}'", target_path.display());
        }

        // If single file, perform high-speed streaming file filter
        if target_path.is_file() {
            let target_path_clone = target_path.clone();
            let mut opts_clone = stream_opts.clone();
            let rel_display = target_path
                .strip_prefix(&ctx.cwd)
                .unwrap_or(&target_path)
                .to_string_lossy()
                .to_string();
            opts_clone.source_name = Some(rel_display);

            let res = tokio::task::spawn_blocking(move || -> anyhow::Result<StreamFilterResult> {
                let filter = StreamGrepFilter::new(opts_clone)?;
                filter.filter_file(&target_path_clone)
            })
            .await
            .map_err(|e| anyhow::anyhow!("File stream filter task join failed: {e}"))??;

            if format_json {
                return Ok(serde_json::to_string_pretty(&res.to_json_value())?);
            } else {
                return Ok(res.format_output(&stream_opts));
            }
        }

        // If directory, use FilterableGrepEngine with full path filter capabilities
        let grep_opts = GrepOptions::from_json(&args, &ctx.cwd)?;
        let display_path = grep_opts.search_path.display().to_string();
        let pattern_str = grep_opts.pattern.clone();
        let grep_opts_clone = grep_opts.clone();

        let search_result =
            tokio::task::spawn_blocking(move || -> anyhow::Result<GrepSearchResult> {
                let engine = FilterableGrepEngine::new(grep_opts_clone)?;
                engine.search()
            })
            .await
            .map_err(|e| anyhow::anyhow!("Directory grep filter task join failed: {e}"))??;

        if format_json {
            Ok(serde_json::to_string_pretty(
                &search_result.to_json_value(),
            )?)
        } else {
            Ok(search_result.format_output(&display_path, &pattern_str, &grep_opts))
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    struct TestWorkspace {
        dir: PathBuf,
    }

    impl TestWorkspace {
        fn new(name: &str) -> Self {
            let id = uuid::Uuid::new_v4();
            let dir = std::env::temp_dir().join(format!("fusion_grep_filter_{}_{}", name, id));
            fs::create_dir_all(&dir).unwrap();
            Self { dir }
        }

        fn create_file(&self, rel: &str, content: &[u8]) -> PathBuf {
            let full = self.dir.join(rel);
            if let Some(parent) = full.parent() {
                fs::create_dir_all(parent).unwrap();
            }
            let mut f = fs::File::create(&full).unwrap();
            f.write_all(content).unwrap();
            full
        }
    }

    impl Drop for TestWorkspace {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.dir);
        }
    }

    #[test]
    fn test_file_type_registry() {
        let reg = FileTypeRegistry::new();
        assert_eq!(reg.resolve_extensions("rust"), vec!["rs".to_string()]);
        assert_eq!(reg.resolve_extensions(".rs"), vec!["rs".to_string()]);
        assert!(reg
            .resolve_extensions("typescript")
            .contains(&"ts".to_string()));
        assert!(reg
            .resolve_extensions("typescript")
            .contains(&"tsx".to_string()));
        assert_eq!(reg.resolve_extensions("custom"), vec!["custom".to_string()]);
    }

    #[test]
    fn test_path_filter_include_exclude_globs() {
        let ws = TestWorkspace::new("filter_globs");
        let f_rs = ws.create_file("src/main.rs", b"fn main() {}");
        let f_ts = ws.create_file("src/index.ts", b"console.log('hi');");
        let f_test = ws.create_file("tests/main_test.rs", b"#[test] fn test() {}");
        let f_node = ws.create_file("node_modules/pkg/index.js", b"module.exports = {};");

        // Include only *.rs
        let filter = PathFilter::builder().include("*.rs").build().unwrap();

        assert!(filter.matches_file(&f_rs, &ws.dir, None));
        assert!(filter.matches_file(&f_test, &ws.dir, None));
        assert!(!filter.matches_file(&f_ts, &ws.dir, None));
        assert!(!filter.matches_file(&f_node, &ws.dir, None));

        // Include *.rs but exclude tests/**
        let filter_ex = PathFilter::builder()
            .include("*.rs")
            .exclude("tests/**")
            .build()
            .unwrap();

        assert!(filter_ex.matches_file(&f_rs, &ws.dir, None));
        assert!(!filter_ex.matches_file(&f_test, &ws.dir, None));
        assert!(!filter_ex.matches_file(&f_ts, &ws.dir, None));
    }

    #[test]
    fn test_path_filter_file_types() {
        let ws = TestWorkspace::new("filter_types");
        let f_rs = ws.create_file("lib.rs", b"pub fn a() {}");
        let f_py = ws.create_file("script.py", b"print('py')");
        let f_json = ws.create_file("data.json", b"{}");

        let filter = PathFilter::builder()
            .file_type("rust")
            .file_type("json")
            .build()
            .unwrap();

        assert!(filter.matches_file(&f_rs, &ws.dir, None));
        assert!(filter.matches_file(&f_json, &ws.dir, None));
        assert!(!filter.matches_file(&f_py, &ws.dir, None));
    }

    #[test]
    fn test_filterable_grep_engine_search() {
        let ws = TestWorkspace::new("engine_search");
        ws.create_file(
            "src/app.rs",
            b"use std::sync::Arc;\n// TARGET_TOKEN in app\nfn run() {}\n",
        );
        ws.create_file(
            "src/util.ts",
            b"export const x = 1;\n// TARGET_TOKEN in util\n",
        );
        ws.create_file("src/ignored.rs", b"// TARGET_TOKEN in ignored\n");
        ws.create_file("docs/readme.md", b"# Info\nTARGET_TOKEN in docs\n");

        let mut opts = GrepOptions::new("TARGET_TOKEN");
        opts.search_path = ws.dir.clone();
        opts.cwd = ws.dir.clone();
        opts.include_globs = vec!["*.rs".to_string()];
        opts.exclude_globs = vec!["src/ignored.rs".to_string()];

        let engine = FilterableGrepEngine::new(opts).unwrap();
        let res = engine.search().unwrap();

        assert_eq!(res.files_matched, 1);
        assert_eq!(res.matches.len(), 1);
        assert_eq!(res.matches[0].file, "src/app.rs");
        assert_eq!(res.matches[0].line_number, 2);
    }

    #[test]
    fn test_filterable_grep_context_lines() {
        let ws = TestWorkspace::new("context_lines");
        ws.create_file(
            "src/example.rs",
            b"line 1\nline 2\nTARGET line 3\nline 4\nline 5\n",
        );

        let mut opts = GrepOptions::new("TARGET");
        opts.search_path = ws.dir.clone();
        opts.cwd = ws.dir.clone();
        opts.context_before = 1;
        opts.context_after = 1;

        let engine = FilterableGrepEngine::new(opts.clone()).unwrap();
        let res = engine.search().unwrap();

        assert_eq!(res.matches.len(), 3);
        assert_eq!(res.matches[0].line_number, 2);
        assert!(res.matches[0].is_context);
        assert_eq!(res.matches[1].line_number, 3);
        assert!(!res.matches[1].is_context);
        assert_eq!(res.matches[2].line_number, 4);
        assert!(res.matches[2].is_context);

        let output = res.format_output(&ws.dir.display().to_string(), "TARGET", &opts);
        assert!(output.contains("src/example.rs-2: line 2"));
        assert!(output.contains("src/example.rs:3: TARGET line 3"));
        assert!(output.contains("src/example.rs-4: line 4"));
    }

    #[test]
    fn test_filterable_grep_invert_match() {
        let ws = TestWorkspace::new("invert_match");
        ws.create_file("test.txt", b"apple\nbanana\ncherry\n");

        let mut opts = GrepOptions::new("banana");
        opts.search_path = ws.dir.clone();
        opts.cwd = ws.dir.clone();
        opts.invert_match = true;

        let engine = FilterableGrepEngine::new(opts).unwrap();
        let res = engine.search().unwrap();

        assert_eq!(res.matches.len(), 2);
        assert_eq!(res.matches[0].line, "apple");
        assert_eq!(res.matches[1].line, "cherry");
    }

    #[test]
    fn test_filterable_grep_fixed_strings() {
        let ws = TestWorkspace::new("fixed_strings");
        ws.create_file("code.rs", b"let x = regex[0];\nlet y = regex.len();\n");

        let mut opts = GrepOptions::new("regex[0]");
        opts.search_path = ws.dir.clone();
        opts.cwd = ws.dir.clone();
        opts.fixed_strings = true;

        let engine = FilterableGrepEngine::new(opts).unwrap();
        let res = engine.search().unwrap();

        assert_eq!(res.matches.len(), 1);
        assert_eq!(res.matches[0].line_number, 1);
    }

    // -----------------------------------------------------------------------
    // Streaming Grep Filter Unit Tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_stream_grep_filter_basic() {
        let text = "alpha\nbeta\ngamma\nbeta delta\nepsilon";
        let opts = StreamGrepOptions::new("beta");
        let filter = StreamGrepFilter::new(opts.clone()).unwrap();
        let res = filter.filter_str(text, None).unwrap();

        assert_eq!(res.total_matches, 2);
        assert_eq!(res.lines.len(), 2);
        assert_eq!(res.lines[0].line_number, 2);
        assert_eq!(res.lines[0].content, "beta");
        assert_eq!(res.lines[1].line_number, 4);
        assert_eq!(res.lines[1].content, "beta delta");

        let output = res.format_output(&opts);
        assert!(output.contains("2: beta"));
        assert!(output.contains("4: beta delta"));
    }

    #[test]
    fn test_stream_grep_filter_case_sensitivity() {
        let text = "INFO: system ready\nwarn: high memory\nERROR: timeout\ninfo: reconnect";
        let opts_sensitive = StreamGrepOptions::new("INFO").case_sensitive(true);
        let filter_s = StreamGrepFilter::new(opts_sensitive).unwrap();
        let res_s = filter_s.filter_str(text, None).unwrap();
        assert_eq!(res_s.total_matches, 1);

        let opts_insensitive = StreamGrepOptions::new("info").case_sensitive(false);
        let filter_i = StreamGrepFilter::new(opts_insensitive).unwrap();
        let res_i = filter_i.filter_str(text, None).unwrap();
        assert_eq!(res_i.total_matches, 2);
    }

    #[test]
    fn test_stream_grep_filter_invert_match() {
        let text = "DEBUG: line 1\nERROR: line 2\nDEBUG: line 3\nINFO: line 4";
        let opts = StreamGrepOptions::new("DEBUG").invert_match(true);
        let filter = StreamGrepFilter::new(opts).unwrap();
        let res = filter.filter_str(text, None).unwrap();

        assert_eq!(res.total_matches, 2);
        assert_eq!(res.lines[0].line_number, 2);
        assert_eq!(res.lines[0].content, "ERROR: line 2");
        assert_eq!(res.lines[1].line_number, 4);
        assert_eq!(res.lines[1].content, "INFO: line 4");
    }

    #[test]
    fn test_stream_grep_filter_fixed_strings() {
        let text = "calc(x + y)\ncalc[0]\ncalc.run()";
        let opts = StreamGrepOptions::new("calc[0]").fixed_strings(true);
        let filter = StreamGrepFilter::new(opts).unwrap();
        let res = filter.filter_str(text, None).unwrap();

        assert_eq!(res.total_matches, 1);
        assert_eq!(res.lines[0].line_number, 2);
        assert_eq!(res.lines[0].content, "calc[0]");
        assert_eq!(res.lines[0].match_spans, vec![(0, 7)]);
    }

    #[test]
    fn test_stream_grep_filter_context_before_after() {
        let text = "L1: start\nL2: prepare\nL3: TARGET here\nL4: cleanup\nL5: end";
        let opts = StreamGrepOptions::new("TARGET")
            .context_before(1)
            .context_after(1);
        let filter = StreamGrepFilter::new(opts.clone()).unwrap();
        let res = filter.filter_str(text, None).unwrap();

        assert_eq!(res.total_matches, 1);
        assert_eq!(res.lines.len(), 3);

        assert_eq!(res.lines[0].line_number, 2);
        assert!(!res.lines[0].is_match);
        assert_eq!(res.lines[0].content, "L2: prepare");

        assert_eq!(res.lines[1].line_number, 3);
        assert!(res.lines[1].is_match);
        assert_eq!(res.lines[1].content, "L3: TARGET here");

        assert_eq!(res.lines[2].line_number, 4);
        assert!(!res.lines[2].is_match);
        assert_eq!(res.lines[2].content, "L4: cleanup");

        let output = res.format_output(&opts);
        assert!(output.contains("2- L2: prepare"));
        assert!(output.contains("3: L3: TARGET here"));
        assert!(output.contains("4- L4: cleanup"));
    }

    #[test]
    fn test_stream_grep_filter_overlapping_context() {
        let text = "line 1\nline 2 (TARGET 1)\nline 3\nline 4 (TARGET 2)\nline 5\nline 6\nline 7\nline 8 (TARGET 3)\nline 9";
        let opts = StreamGrepOptions::new("TARGET")
            .context_before(1)
            .context_after(1);
        let filter = StreamGrepFilter::new(opts.clone()).unwrap();
        let res = filter.filter_str(text, None).unwrap();

        assert_eq!(res.total_matches, 3);
        // Lines 1, 2(match), 3(context), 4(match), 5(context), separator, 7(context), 8(match), 9(context)
        let output = res.format_output(&opts);
        assert!(output.contains("--"));
        assert!(output.contains("2: line 2 (TARGET 1)"));
        assert!(output.contains("4: line 4 (TARGET 2)"));
        assert!(output.contains("8: line 8 (TARGET 3)"));
    }

    #[test]
    fn test_stream_grep_filter_max_matches_limit() {
        let text = "match 1\nmatch 2\nmatch 3\nmatch 4\nmatch 5";
        let opts = StreamGrepOptions::new("match").max_matches(2);
        let filter = StreamGrepFilter::new(opts.clone()).unwrap();
        let res = filter.filter_str(text, None).unwrap();

        assert_eq!(res.total_matches, 2);
        assert!(res.truncated_by_matches);
        let output = res.format_output(&opts);
        assert!(output.contains("matches limit reached"));
    }

    #[test]
    fn test_stream_grep_filter_max_bytes_limit() {
        let text = "A".repeat(500) + "\n" + &"B".repeat(500) + "\n" + &"C".repeat(500);
        let opts = StreamGrepOptions::new(".").max_bytes(600);
        let filter = StreamGrepFilter::new(opts.clone()).unwrap();
        let res = filter.filter_str(&text, None).unwrap();

        assert!(res.truncated_by_bytes);
        let output = res.format_output(&opts);
        assert!(output.contains("output byte limit reached"));
    }

    #[test]
    fn test_stream_grep_filter_max_line_length_truncation() {
        let long_line = "START_".to_string() + &"X".repeat(200) + "_END";
        let opts = StreamGrepOptions::new("START").max_line_length(20);
        let filter = StreamGrepFilter::new(opts).unwrap();
        let res = filter.filter_str(&long_line, None).unwrap();

        assert_eq!(res.total_matches, 1);
        assert!(res.lines[0].content.ends_with("..."));
        assert!(res.lines[0].content.len() <= 24);
    }

    #[test]
    fn test_stream_grep_filter_count_only() {
        let text = "apple\nbanana\napricot\navocado\nberry";
        let opts = StreamGrepOptions::new("^a").count_only(true);
        let filter = StreamGrepFilter::new(opts.clone()).unwrap();
        let res = filter.filter_str(text, None).unwrap();

        assert_eq!(res.total_matches, 3);
        assert!(res.lines.is_empty());
        let output = res.format_output(&opts);
        assert!(output.contains("Total matches: 3"));
    }

    #[test]
    fn test_stream_grep_filter_binary_probe() {
        let binary_data = b"hello\x00world\nnext line with target\n";
        let opts = StreamGrepOptions::new("target");
        let filter = StreamGrepFilter::new(opts.clone()).unwrap();
        let res = filter
            .filter_reader(Cursor::new(binary_data), None)
            .unwrap();

        assert!(res.is_binary);
        let output = res.format_output(&opts);
        assert!(output.contains("Binary file or stream matches"));
    }

    #[test]
    fn test_stream_grep_filter_file_reading() {
        let ws = TestWorkspace::new("stream_file");
        let path = ws.create_file(
            "app.log",
            b"2026-09-02 [INFO] Booting system\n2026-09-02 [ERROR] Database connection lost\n2026-09-02 [INFO] Retrying\n",
        );

        let opts = StreamGrepOptions::new("ERROR").context(1);
        let filter = StreamGrepFilter::new(opts).unwrap();
        let res = filter.filter_file(&path).unwrap();

        assert_eq!(res.total_matches, 1);
        assert_eq!(res.lines.len(), 3);
        assert_eq!(res.lines[1].line_number, 2);
        assert_eq!(
            res.lines[1].content,
            "2026-09-02 [ERROR] Database connection lost"
        );
    }

    #[test]
    fn test_stream_grep_filter_json_output() {
        let text = "match line";
        let opts = StreamGrepOptions::new("match");
        let filter = StreamGrepFilter::new(opts).unwrap();
        let res = filter.filter_str(text, Some("test_stream")).unwrap();

        let json_val = res.to_json_value();
        assert_eq!(json_val["total_matches"], 1);
        assert_eq!(json_val["lines"][0]["source"], "test_stream");
        assert_eq!(json_val["lines"][0]["content"], "match line");
        assert_eq!(json_val["lines"][0]["is_match"], true);
    }

    #[tokio::test]
    async fn test_grep_filter_tool_execute_text() {
        let tool = GrepFilterTool::new();
        let ctx = ToolContext::default();

        let args = json!({
            "pattern": "ERROR",
            "input": "2026-09-02 INFO init\n2026-09-02 ERROR failed\n2026-09-02 INFO done",
            "context": 1
        });

        let output = tool.execute(args, &ctx).await.unwrap();
        assert!(output.contains("2: 2026-09-02 ERROR failed"));
        assert!(output.contains("1- 2026-09-02 INFO init"));
        assert!(output.contains("3- 2026-09-02 INFO done"));
    }

    #[tokio::test]
    async fn test_grep_filter_tool_execute_file() {
        let ws = TestWorkspace::new("tool_file");
        let file_path = ws.create_file("service.log", b"trace 1\nPANIC: out of memory\ntrace 2\n");
        let tool = GrepFilterTool::new();
        let ctx = ToolContext {
            cwd: ws.dir.clone(),
            env: HashMap::new(),
        };

        let args = json!({
            "pattern": "PANIC",
            "path": file_path.to_string_lossy().to_string(),
            "context": 1
        });

        let output = tool.execute(args, &ctx).await.unwrap();
        assert!(output.contains("PANIC: out of memory"));
    }

    #[tokio::test]
    async fn test_grep_filter_tool_execute_directory() {
        let ws = TestWorkspace::new("tool_dir");
        ws.create_file("src/a.rs", b"fn test_a() { let token = 1; }\n");
        ws.create_file("src/b.rs", b"fn test_b() { let token = 2; }\n");
        ws.create_file("docs/c.md", b"# token in docs\n");

        let tool = GrepFilterTool::new();
        let ctx = ToolContext {
            cwd: ws.dir.clone(),
            env: HashMap::new(),
        };

        let args = json!({
            "pattern": "token",
            "include": "*.rs"
        });

        let output = tool.execute(args, &ctx).await.unwrap();
        assert!(output.contains("src/a.rs"));
        assert!(output.contains("src/b.rs"));
        assert!(!output.contains("docs/c.md"));
    }
}
