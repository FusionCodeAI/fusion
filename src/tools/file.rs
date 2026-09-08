use async_trait::async_trait;
use fusion_walker::WalkRequest;
use fusion_ast::summary::{summarize_code, SummaryOptions};
use fusion_ast::SupportLang;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

use crate::tools::types::{Tool, ToolContext};

pub fn resolve_path(path_str: &str, cwd: &Path) -> PathBuf {
    let p = Path::new(path_str);
    if p.is_absolute() {
        p.to_path_buf()
    } else {
        cwd.join(p)
    }
}

/// Create a high-throughput, parallel multi-threaded directory walk request
/// rooted at `path`, respecting `.gitignore` and skipping `.git` repositories.
pub fn walk_dir(path: impl Into<PathBuf>, hidden: bool) -> WalkRequest {
    WalkRequest::new(path)
        .hidden(hidden)
        .gitignore(true)
        .skip_git(true)
        .emit_root(false)
}

// ---------------------------------------------------------------------------
// Shared file helpers
// ---------------------------------------------------------------------------

/// Default maximum number of lines returned by the `read` tool when no
/// explicit `limit` is provided. Prevents unbounded output on very large
/// files; the truncation footer reports the remaining lines.
pub const DEFAULT_READ_LIMIT: usize = 2000;

/// Number of leading bytes sniffed for NUL bytes to detect binary files.
pub const BINARY_SNIFF_BYTES: usize = 8192;

/// Parameters describing a windowed, streaming read of a text file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReadWindow {
    /// 1-based line number to start reading from (minimum 1).
    pub offset: usize,
    /// Maximum number of lines to return; `None` means `DEFAULT_READ_LIMIT`.
    pub limit: Option<usize>,
    /// Whether to prefix each output line with `"{n:>6} | "`.
    pub line_numbers: bool,
}

/// Parse `offset` / `limit` / `line_numbers` tool arguments into a
/// [`ReadWindow`]. `offset` is clamped to a minimum of 1.
pub fn parse_read_window(args: &Value) -> ReadWindow {
    let offset = args
        .get("offset")
        .and_then(Value::as_u64)
        .unwrap_or(1)
        .max(1) as usize;

    let limit = args
        .get("limit")
        .and_then(Value::as_u64)
        .map(|l| l as usize);

    let line_numbers = args
        .get("line_numbers")
        .and_then(Value::as_bool)
        .unwrap_or(true);

    ReadWindow {
        offset,
        limit,
        line_numbers,
    }
}

/// Format a selected window of lines for output.
///
/// * `selected` — the lines inside the window, in order.
/// * `start_line` — 1-based line number of `selected[0]` in the source file.
/// * `total_lines` — total line count of the source file.
///
/// A truncation footer is appended when the window does not reach the end
/// of the file.
pub fn format_read_output(
    window: &ReadWindow,
    selected: &[String],
    start_line: usize,
    total_lines: usize,
) -> String {
    use std::fmt::Write as _;

    let mut output = String::new();
    for (idx, line) in selected.iter().enumerate() {
        if window.line_numbers {
            let _ = write!(output, "{:6} | {}\n", start_line + idx, line);
        } else {
            output.push_str(line);
            output.push('\n');
        }
    }

    let end = start_line.saturating_sub(1) + selected.len();
    if window.line_numbers && end < total_lines {
        let remaining = total_lines - end;
        output.push_str(&format!(
            "\n... [{} more lines in file (total: {})]\n",
            remaining, total_lines
        ));
    }

    output
}

/// Atomically write `contents` to `path`.
///
/// The bytes are written to a unique sibling temporary file, flushed to
/// stable storage, then renamed over `path`, so readers never observe a
/// partially written file. Parent directories are created as needed. On
/// failure the temporary file is removed and any pre-existing file at
/// `path` is left untouched.
pub async fn atomic_write(path: &Path, contents: &[u8]) -> anyhow::Result<()> {
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            tokio::fs::create_dir_all(parent).await.map_err(|e| {
                anyhow::anyhow!(
                    "Failed to create parent directory for '{}': {e}",
                    path.display()
                )
            })?;
        }
    }

    let mut tmp_name = path.as_os_str().to_os_string();
    tmp_name.push(format!(".tmp.{}", uuid::Uuid::new_v4()).as_str());
    let tmp_path = PathBuf::from(tmp_name);

    let result = write_tmp_and_rename(&tmp_path, path, contents).await;

    if result.is_err() {
        // Best-effort cleanup; the temp file may already have been renamed.
        let _ = tokio::fs::remove_file(&tmp_path).await;
    }

    result
}

async fn write_tmp_and_rename(tmp_path: &Path, dest: &Path, contents: &[u8]) -> anyhow::Result<()> {
    let mut file = tokio::fs::File::create(tmp_path)
        .await
        .map_err(|e| anyhow::anyhow!("Failed to create temp file '{}': {e}", tmp_path.display()))?;
    file.write_all(contents)
        .await
        .map_err(|e| anyhow::anyhow!("Failed to write temp file '{}': {e}", tmp_path.display()))?;
    file.sync_all()
        .await
        .map_err(|e| anyhow::anyhow!("Failed to flush temp file '{}': {e}", tmp_path.display()))?;
    drop(file);

    tokio::fs::rename(tmp_path, dest)
        .await
        .map_err(|e| anyhow::anyhow!("Failed to atomically replace '{}': {e}", dest.display()))
}

// ---------------------------------------------------------------------------
// ReadFileTool
/// Parsed inline selector components from a path string (e.g. `src/main.rs:10-40:raw`).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct PathSelector {
    /// 1-based inclusive slice range: (start, optional end)
    pub range: Option<(usize, Option<usize>)>,
    /// Skip line number headers and return verbatim content
    pub raw: bool,
    /// Use fusion_ast to elide function and struct bodies (...)
    pub defs: bool,
    /// Whether any explicit selector (:range, :raw, :defs) was present in path
    pub has_explicit_selector: bool,
}

/// Parse inline selectors from a path string, returning the clean path without
/// selectors and the parsed [`PathSelector`].
pub fn parse_path_selector(path_str: &str) -> (String, PathSelector) {
    let mut current_path = path_str;
    let mut selector = PathSelector::default();

    loop {
        let Some(last_colon_idx) = current_path.rfind(':') else {
            break;
        };

        // Don't treat Windows drive letters (e.g. C:\ or C:/) as a selector
        if last_colon_idx == 1 {
            let first_char = current_path.chars().next().unwrap_or('\0');
            if first_char.is_ascii_alphabetic() {
                break;
            }
        }

        let suffix = &current_path[last_colon_idx + 1..];

        if suffix == "raw" {
            selector.raw = true;
            selector.has_explicit_selector = true;
            current_path = &current_path[..last_colon_idx];
        } else if suffix == "defs" {
            selector.defs = true;
            selector.has_explicit_selector = true;
            current_path = &current_path[..last_colon_idx];
        } else if let Some((start, end)) = parse_range_selector(suffix) {
            selector.range = Some((start, end));
            selector.has_explicit_selector = true;
            current_path = &current_path[..last_colon_idx];
        } else {
            break;
        }
    }

    (current_path.to_string(), selector)
}

fn parse_range_selector(suffix: &str) -> Option<(usize, Option<usize>)> {
    if suffix.is_empty() {
        return None;
    }

    if let Some((start_s, end_s)) = suffix.split_once('-') {
        let start = start_s.parse::<usize>().ok()?.max(1);
        if end_s.is_empty() {
            Some((start, None))
        } else {
            let end = end_s.parse::<usize>().ok()?.max(1);
            if end < start {
                return None;
            }
            Some((start, Some(end)))
        }
    } else {
        let start = suffix.parse::<usize>().ok()?.max(1);
        Some((start, None))
    }
}

fn render_defs_outline(content: &str, path: &Path, line_numbers: bool) -> anyhow::Result<String> {
    use std::fmt::Write as _;

    let content_lines: Vec<&str> = content.lines().collect();
    let options = SummaryOptions {
        code: content.to_string(),
        lang: None,
        path: Some(path.to_string_lossy().to_string()),
        min_body_lines: Some(2),
        min_comment_lines: None,
        unfold_until_lines: None,
        unfold_limit_lines: None,
    };

    let summary_opt = summarize_code(options).ok();
    let mut output = String::new();
    if let Some(summary) = summary_opt {
        if summary.parsed {
            for segment in &summary.segments {
                if segment.kind == "kept" {
                    for line_num in segment.start_line..=segment.end_line {
                        let line = content_lines
                            .get((line_num - 1) as usize)
                            .copied()
                            .unwrap_or("");
                        if line_numbers {
                            let _ = write!(output, "{:6} | {}\n", line_num, line);
                        } else {
                            output.push_str(line);
                            output.push('\n');
                        }
                    }
                } else if segment.kind == "elided" {
                    if line_numbers {
                        let _ = write!(output, "{:>6} | ...\n", "...");
                    } else {
                        output.push_str("...\n");
                    }
                }
            }
        } else {
            for (idx, line) in content_lines.iter().enumerate() {
                let line_num = idx + 1;
                if line_numbers {
                    let _ = write!(output, "{:6} | {}\n", line_num, line);
                } else {
                    output.push_str(line);
                    output.push('\n');
                }
            }
        }
    } else {
        for (idx, line) in content_lines.iter().enumerate() {
            let line_num = idx + 1;
            if line_numbers {
                let _ = write!(output, "{:6} | {}\n", line_num, line);
            } else {
                output.push_str(line);
                output.push('\n');
            }
        }
    }

    Ok(output)
}

fn render_oversized_summary(
    content: &str,
    path: &Path,
    line_numbers: bool,
) -> anyhow::Result<String> {
    use std::fmt::Write as _;

    let content_lines: Vec<&str> = content.lines().collect();
    let total_lines = content_lines.len();

    let options = SummaryOptions {
        code: content.to_string(),
        lang: None,
        path: Some(path.to_string_lossy().to_string()),
        min_body_lines: Some(2),
        min_comment_lines: None,
        unfold_until_lines: None,
        unfold_limit_lines: None,
    };

    let summary_opt = summarize_code(options).ok();
    let mut output = String::new();
    let mut elided_lines_count = 0usize;

    if let Some(summary) = summary_opt {
        if summary.parsed && summary.elided {
        for segment in &summary.segments {
            if segment.kind == "kept" {
                for line_num in segment.start_line..=segment.end_line {
                    let line = content_lines
                        .get((line_num - 1) as usize)
                        .copied()
                        .unwrap_or("");
                    if line_numbers {
                        let _ = write!(output, "{:6} | {}\n", line_num, line);
                    } else {
                        output.push_str(line);
                        output.push('\n');
                    }
                }
            } else if segment.kind == "elided" {
                let span = (segment.end_line.saturating_sub(segment.start_line) + 1) as usize;
                elided_lines_count += span;
                if line_numbers {
                    let _ = write!(output, "{:>6} | ...\n", "...");
                } else {
                    output.push_str("...\n");
                }
            }
        }
        }
    }

    if elided_lines_count == 0 {
        output.clear();
        let head_count = 50.min(total_lines);
        let tail_count = 50.min(total_lines.saturating_sub(head_count));
        let elided_span = total_lines.saturating_sub(head_count + tail_count);
        elided_lines_count = elided_span;

        for line_num in 1..=head_count {
            let line = content_lines.get(line_num - 1).copied().unwrap_or("");
            if line_numbers {
                let _ = write!(output, "{:6} | {}\n", line_num, line);
            } else {
                output.push_str(line);
                output.push('\n');
            }
        }

        if elided_span > 0 {
            if line_numbers {
                let _ = write!(output, "{:>6} | ...\n", "...");
            } else {
                output.push_str("...\n");
            }
        }

        let tail_start = total_lines.saturating_sub(tail_count) + 1;
        for line_num in tail_start..=total_lines {
            let line = content_lines.get(line_num - 1).copied().unwrap_or("");
            if line_numbers {
                let _ = write!(output, "{:6} | {}\n", line_num, line);
            } else {
                output.push_str(line);
                output.push('\n');
            }
        }
    }

    output.push_str(&format!(
        "\nSummary: {} lines elided; re-issue with line range selector (e.g. :50-100)\n",
        elided_lines_count
    ));

    Ok(output)
}

/// Parse inline selectors from a URL, respecting ports and paths.
pub fn parse_url_selector(raw_url: &str) -> (String, PathSelector) {
    let mut url = raw_url;
    let mut selector = PathSelector::default();

    if let Some(stripped) = url.strip_suffix(":raw") {
        selector.raw = true;
        selector.has_explicit_selector = true;
        url = stripped;
    } else if let Some(stripped) = url.strip_suffix(":defs") {
        selector.defs = true;
        selector.has_explicit_selector = true;
        url = stripped;
    }

    (url.to_string(), selector)
}

/// Strips HTML `<script>`, `<style>`, `<nav>`, `<header>`, `<footer>` blocks
/// (and comments, `<noscript>`, `<svg>`, `<template>`), discarding their tags and inner contents.
pub fn strip_html_blocks(html: &str) -> String {
    let skip_tag_names = [
        "script", "style", "nav", "header", "footer", "noscript", "svg", "template",
    ];
    let mut result = String::with_capacity(html.len());
    let mut chars = html.chars().peekable();

    while let Some(ch) = chars.next() {
        if ch == '<' {
            // Check for HTML comment <!-- ... -->
            if chars.peek() == Some(&'!') {
                let mut lookahead = chars.clone();
                lookahead.next(); // '!'
                if lookahead.next() == Some('-') && lookahead.next() == Some('-') {
                    chars.next(); // '!'
                    chars.next(); // '-'
                    chars.next(); // '-'
                    while let Some(c) = chars.next() {
                        if c == '-' {
                            let mut la = chars.clone();
                            if la.next() == Some('-') && la.next() == Some('>') {
                                chars.next(); // '-'
                                chars.next(); // '>'
                                break;
                            }
                        }
                    }
                    continue;
                }
            }

            // Read tag
            let mut tag_buf = String::new();
            let mut is_closing = false;
            if chars.peek() == Some(&'/') {
                is_closing = true;
                chars.next();
            }

            let mut tag_name = String::new();
            while let Some(&c) = chars.peek() {
                if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                    tag_name.push(chars.next().unwrap());
                } else {
                    break;
                }
            }

            // Read remainder of tag until '>'
            let mut is_self_closing = false;
            while let Some(c) = chars.next() {
                tag_buf.push(c);
                if c == '>' {
                    if tag_buf.trim_end_matches('>').trim_end().ends_with('/') {
                        is_self_closing = true;
                    }
                    break;
                }
            }

            let tag_lower = tag_name.to_ascii_lowercase();
            if !is_closing && !is_self_closing && skip_tag_names.contains(&tag_lower.as_str()) {
                // Skip everything until matching closing tag </tag_lower>
                let mut depth = 1usize;
                while depth > 0 && chars.peek().is_some() {
                    if let Some(c) = chars.next() {
                        if c == '<' {
                            if chars.peek() == Some(&'!') {
                                let mut lookahead = chars.clone();
                                lookahead.next();
                                if lookahead.next() == Some('-') && lookahead.next() == Some('-') {
                                    chars.next();
                                    chars.next();
                                    chars.next();
                                    while let Some(cc) = chars.next() {
                                        if cc == '-' {
                                            let mut la = chars.clone();
                                            if la.next() == Some('-') && la.next() == Some('>') {
                                                chars.next();
                                                chars.next();
                                                break;
                                            }
                                        }
                                    }
                                    continue;
                                }
                            }

                            let mut inner_is_closing = false;
                            if chars.peek() == Some(&'/') {
                                inner_is_closing = true;
                                chars.next();
                            }
                            let mut inner_tag_name = String::new();
                            while let Some(&ic) = chars.peek() {
                                if ic.is_ascii_alphanumeric() || ic == '-' || ic == '_' {
                                    inner_tag_name.push(chars.next().unwrap());
                                } else {
                                    break;
                                }
                            }
                            let mut inner_self_closing = false;
                            let mut inner_buf = String::new();
                            while let Some(ic) = chars.next() {
                                inner_buf.push(ic);
                                if ic == '>' {
                                    if inner_buf.trim_end_matches('>').trim_end().ends_with('/') {
                                        inner_self_closing = true;
                                    }
                                    break;
                                }
                            }

                            if inner_tag_name.eq_ignore_ascii_case(&tag_lower) {
                                if inner_is_closing {
                                    depth = depth.saturating_sub(1);
                                } else if !inner_self_closing {
                                    depth += 1;
                                }
                            }
                        }
                    }
                }
                result.push('\n');
                continue;
            }

            // Normal tag: emit back out
            result.push('<');
            if is_closing {
                result.push('/');
            }
            result.push_str(&tag_name);
            result.push_str(&tag_buf);
        } else {
            result.push(ch);
        }
    }

    result
}

fn extract_attribute(tag: &str, attr_name: &str) -> Option<String> {
    let lower_tag = tag.to_ascii_lowercase();
    let lower_attr = attr_name.to_ascii_lowercase();

    let mut search_from = 0;
    while let Some(pos) = lower_tag[search_from..].find(&lower_attr) {
        let abs_pos = search_from + pos;
        let before_ok = abs_pos == 0
            || tag.as_bytes()[abs_pos - 1].is_ascii_whitespace()
            || tag.as_bytes()[abs_pos - 1] == b'<'
            || tag.as_bytes()[abs_pos - 1] == b'/';
        let after_pos = abs_pos + lower_attr.len();
        let after_slice = tag[after_pos..].trim_start();

        if before_ok && after_slice.starts_with('=') {
            let val_slice = after_slice[1..].trim_start();
            if let Some(quote) = val_slice.chars().next() {
                if quote == '"' || quote == '\'' {
                    let inside = &val_slice[1..];
                    if let Some(end_quote) = inside.find(quote) {
                        return Some(inside[..end_quote].to_string());
                    }
                } else {
                    let end_pos = val_slice
                        .find(|c: char| c.is_ascii_whitespace() || c == '>')
                        .unwrap_or(val_slice.len());
                    return Some(val_slice[..end_pos].to_string());
                }
            }
        }
        search_from = after_pos;
    }
    None
}

fn decode_entity_owned(entity: &str) -> Option<String> {
    let s = match entity.to_ascii_lowercase().as_str() {
        "amp" => Some("&"),
        "lt" => Some("<"),
        "gt" => Some(">"),
        "quot" => Some("\""),
        "apos" | "#39" => Some("'"),
        "nbsp" => Some(" "),
        "copy" => Some("©"),
        "mdash" => Some("—"),
        "ndash" => Some("–"),
        "hellip" => Some("…"),
        "bull" => Some("•"),
        "trade" => Some("™"),
        "reg" => Some("®"),
        _ => None,
    };
    if let Some(decoded) = s {
        return Some(decoded.to_string());
    }

    if let Some(num_str) = entity.strip_prefix('#') {
        let code = if let Some(hex_str) = num_str.strip_prefix('x').or_else(|| num_str.strip_prefix('X')) {
            u32::from_str_radix(hex_str, 16).ok()
        } else {
            num_str.parse::<u32>().ok()
        };
        if let Some(c) = code.and_then(char::from_u32) {
            return Some(c.to_string());
        }
    }

    None
}

struct HtmlToMarkdownParser<'a> {
    input: &'a str,
    output: String,
    link_stack: Vec<Option<String>>,
    list_depth: usize,
    in_pre: bool,
    in_code: bool,
}

impl<'a> HtmlToMarkdownParser<'a> {
    fn new(input: &'a str) -> Self {
        Self {
            input,
            output: String::with_capacity(input.len()),
            link_stack: Vec::new(),
            list_depth: 0,
            in_pre: false,
            in_code: false,
        }
    }

    fn ensure_newline(&mut self) {
        if !self.output.is_empty() && !self.output.ends_with('\n') {
            self.output.push('\n');
        }
    }

    fn ensure_blank_line(&mut self) {
        if !self.output.is_empty() {
            if !self.output.ends_with('\n') {
                self.output.push_str("\n\n");
            } else if !self.output.ends_with("\n\n") {
                self.output.push('\n');
            }
        }
    }

    fn parse(&mut self) -> String {
        let mut chars = self.input.chars().peekable();

        while let Some(ch) = chars.next() {
            if ch == '<' {
                let mut tag_buf = String::new();
                for c in chars.by_ref() {
                    if c == '>' {
                        break;
                    }
                    tag_buf.push(c);
                }
                self.handle_tag(&tag_buf);
            } else if ch == '&' {
                let mut entity = String::new();
                let mut found_semicolon = false;
                for _ in 0..12 {
                    match chars.peek() {
                        Some(';') => {
                            chars.next();
                            found_semicolon = true;
                            break;
                        }
                        Some(&c) if c.is_ascii_alphanumeric() || c == '#' => {
                            entity.push(chars.next().unwrap());
                        }
                        _ => break,
                    }
                }

                if found_semicolon {
                    if let Some(decoded) = decode_entity_owned(&entity) {
                        self.output.push_str(&decoded);
                        continue;
                    }
                }

                self.output.push('&');
                self.output.push_str(&entity);
                if found_semicolon {
                    self.output.push(';');
                }
            } else {
                self.output.push(ch);
            }
        }

        std::mem::take(&mut self.output)
    }

    fn handle_tag(&mut self, raw_tag: &str) {
        let trimmed = raw_tag.trim();
        if trimmed.is_empty() {
            return;
        }

        let is_closing = trimmed.starts_with('/');
        let tag_content = if is_closing { &trimmed[1..] } else { trimmed };

        let tag_name = tag_content
            .split_whitespace()
            .next()
            .unwrap_or("")
            .trim_end_matches('/')
            .to_ascii_lowercase();

        match (tag_name.as_str(), is_closing) {
            ("h1", false) => {
                self.ensure_blank_line();
                self.output.push_str("# ");
            }
            ("h1", true) => self.ensure_blank_line(),

            ("h2", false) => {
                self.ensure_blank_line();
                self.output.push_str("## ");
            }
            ("h2", true) => self.ensure_blank_line(),

            ("h3", false) => {
                self.ensure_blank_line();
                self.output.push_str("### ");
            }
            ("h3", true) => self.ensure_blank_line(),

            ("h4", false) => {
                self.ensure_blank_line();
                self.output.push_str("#### ");
            }
            ("h4", true) => self.ensure_blank_line(),

            ("h5", false) => {
                self.ensure_blank_line();
                self.output.push_str("##### ");
            }
            ("h5", true) => self.ensure_blank_line(),

            ("h6", false) => {
                self.ensure_blank_line();
                self.output.push_str("###### ");
            }
            ("h6", true) => self.ensure_blank_line(),

            ("p", false) => self.ensure_blank_line(),
            ("p", true) => self.ensure_blank_line(),

            ("br", _) => self.output.push('\n'),

            ("hr", _) => {
                self.ensure_blank_line();
                self.output.push_str("---\n\n");
            }

            ("ul" | "ol", false) => {
                self.ensure_newline();
                self.list_depth += 1;
            }
            ("ul" | "ol", true) => {
                self.list_depth = self.list_depth.saturating_sub(1);
                self.ensure_newline();
            }

            ("li", false) => {
                self.ensure_newline();
                let indent = "  ".repeat(self.list_depth.saturating_sub(1));
                self.output.push_str(&format!("{}- ", indent));
            }
            ("li", true) => self.ensure_newline(),

            ("a", false) => {
                let href = extract_attribute(tag_content, "href");
                let valid_href = href.filter(|u| {
                    let trimmed = u.trim();
                    !trimmed.is_empty()
                        && !trimmed.starts_with('#')
                        && !trimmed.starts_with("javascript:")
                });
                if valid_href.is_some() {
                    self.output.push('[');
                }
                self.link_stack.push(valid_href);
            }
            ("a", true) => {
                if let Some(Some(href)) = self.link_stack.pop() {
                    if self.output.ends_with('[') {
                        self.output.pop();
                        self.output.push_str(&href);
                    } else {
                        self.output.push_str(&format!("]({})", href));
                    }
                }
            }

            ("pre", false) => {
                self.ensure_blank_line();
                self.in_pre = true;
                self.output.push_str("```\n");
            }
            ("pre", true) => {
                self.ensure_newline();
                self.output.push_str("```\n\n");
                self.in_pre = false;
            }

            ("code", false) => {
                if !self.in_pre {
                    self.output.push('`');
                    self.in_code = true;
                }
            }
            ("code", true) => {
                if !self.in_pre && self.in_code {
                    self.output.push('`');
                    self.in_code = false;
                }
            }

            ("strong" | "b", false | true) => {
                if !self.in_pre {
                    self.output.push_str("**");
                }
            }
            ("em" | "i", false | true) => {
                if !self.in_pre {
                    self.output.push('*');
                }
            }

            ("blockquote", false) => {
                self.ensure_blank_line();
                self.output.push_str("> ");
            }
            ("blockquote", true) => self.ensure_blank_line(),

            ("div" | "article" | "section" | "main" | "aside", _) => {
                self.ensure_newline();
            }

            _ => {}
        }
    }
}

fn normalize_markdown(raw: &str) -> String {
    let mut lines: Vec<String> = Vec::new();
    let mut in_code_block = false;

    for line in raw.lines() {
        let trimmed_end = line.trim_end();
        if trimmed_end.starts_with("```") {
            in_code_block = !in_code_block;
            lines.push(trimmed_end.to_string());
            continue;
        }

        if in_code_block {
            lines.push(line.to_string());
        } else {
            if trimmed_end.trim().is_empty() {
                if lines.last().map(|l| !l.is_empty()).unwrap_or(false) {
                    lines.push(String::new());
                }
            } else {
                lines.push(trimmed_end.to_string());
            }
        }
    }

    while lines.first().map(|l| l.is_empty()).unwrap_or(false) {
        lines.remove(0);
    }
    while lines.last().map(|l| l.is_empty()).unwrap_or(false) {
        lines.pop();
    }

    lines.join("\n")
}

/// Converts an HTML string into clean Markdown, stripping script, style, nav, header,
/// and footer blocks, and converting headings, paragraphs, line breaks, lists, and links.
pub fn html_to_markdown(html: &str) -> String {
    let clean_html = strip_html_blocks(html);
    let mut parser = HtmlToMarkdownParser::new(&clean_html);
    let raw = parser.parse();
    normalize_markdown(&raw)
}

fn is_html_content(content: &str, content_type: Option<&str>) -> bool {
    if let Some(ct) = content_type {
        let ct_lower = ct.to_ascii_lowercase();
        if ct_lower.contains("text/html")
            || ct_lower.contains("application/xhtml+xml")
            || ct_lower.contains("text/xml")
        {
            return true;
        }
        if ct_lower.contains("text/plain")
            || ct_lower.contains("text/markdown")
            || ct_lower.contains("application/json")
        {
            return false;
        }
    }

    let lower = content.trim_start().to_ascii_lowercase();
    lower.starts_with("<!doctype")
        || lower.starts_with("<html")
        || lower.contains("<body")
        || (lower.contains("<p") && lower.contains("</p>"))
        || (lower.contains("<div") && lower.contains("</div>"))
        || lower.contains("<script")
        || lower.contains("<style")
        || lower.contains("<header")
        || lower.contains("<nav")
        || lower.contains("<footer")
}
// ---------------------------------------------------------------------------

#[derive(Default, Debug, Clone)]
pub struct ReadFileTool;

impl ReadFileTool {
    pub fn new() -> Self {
        Self
    }

    async fn read_url(&self, path: &str, args: &Value) -> anyhow::Result<String> {
        let (clean_url, selector) = parse_url_selector(path);

        const BROWSER_USER_AGENT: &str =
            "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/124.0.0.0 Safari/537.36";

        let client = reqwest::Client::builder()
            .use_rustls_tls()
            .timeout(std::time::Duration::from_secs(15))
            .user_agent(BROWSER_USER_AGENT)
            .build()
            .unwrap_or_else(|_| reqwest::Client::new());

        let resp = client.get(&clean_url).send().await.map_err(|e| {
            anyhow::anyhow!("Failed to read URL '{}': {e}", path)
        })?;

        let status = resp.status();
        if status != reqwest::StatusCode::OK {
            return Ok(format!("HTTP Error {}: Failed to read URL {}", status, path));
        }

        let content_type = resp
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .map(|s| s.to_string());

        let body = resp.text().await.map_err(|e| {
            anyhow::anyhow!("Failed to read response body from '{}': {e}", clean_url)
        })?;

        if selector.raw {
            return Ok(body);
        }

        let content = if is_html_content(&body, content_type.as_deref()) {
            html_to_markdown(&body)
        } else {
            body
        };

        if args.get("line_numbers") == Some(&Value::Bool(true)) {
            let window = parse_read_window(args);
            let lines: Vec<String> = content.lines().map(|s| s.to_string()).collect();
            let total = lines.len();
            Ok(format_read_output(&window, &lines, 1, total))
        } else {
            Ok(content)
        }
    }
}

pub type FileReadTool = ReadFileTool;

#[async_trait]
impl Tool for ReadFileTool {
    fn name(&self) -> &str {
        "read"
    }

    fn description(&self) -> &str {
        "Read the text content of a file with optional line offset, limit, and line numbering."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "path": {
                    "description": "Path to the file to read (relative to workspace or absolute). Supports inline selectors: ':start-end' (e.g. ':10-40'), ':start' (e.g. ':50'), ':raw', and ':defs'."
                },
                "offset": {
                    "type": "integer",
                    "description": "1-based line number to start reading from (optional, default: 1)."
                },
                "limit": {
                    "type": "integer",
                    "description": "Maximum number of lines to read (optional)."
                },
                "line_numbers": {
                    "type": "boolean",
                    "description": "Whether to prefix each line with its 1-based line number (optional, default: true)."
                }
            },
            "required": ["path"]
        })
    }

    async fn execute(&self, args: Value, ctx: &ToolContext) -> anyhow::Result<String> {
        let path_str = args
            .get("path")
            .and_then(|v| v.as_str())
            .or_else(|| args.get("file_path").and_then(|v| v.as_str()))
            .ok_or_else(|| anyhow::anyhow!("Missing required parameter: path"))?;

        let trimmed_path = path_str.trim();
        if trimmed_path.starts_with("http://") || trimmed_path.starts_with("https://") {
            return self.read_url(trimmed_path, &args).await;
        }
        let (clean_path, selector) = parse_path_selector(path_str);
        if clean_path.is_empty() {
            anyhow::bail!("Missing file path in: '{path_str}'");
        }

        let full_path = if let Some(resolved) =
            crate::tools::uri_router::resolve_internal_uri(&clean_path, Some(&ctx.cwd))
        {
            resolved
        } else {
            resolve_path(&clean_path, &ctx.cwd)
        };

        if !full_path.exists() {
            anyhow::bail!("File not found: '{}'", full_path.display());
        }

        if full_path.is_dir() {
            anyhow::bail!("Path is a directory, not a file: '{}'", full_path.display());
        }

        let mut window = parse_read_window(&args);

        if let Some((start, end_opt)) = selector.range {
            window.offset = start;
            if let Some(end) = end_opt {
                window.limit = Some(end.saturating_sub(start) + 1);
            }
        }

        if selector.raw {
            window.line_numbers = false;
        }

        let line_numbers = window.line_numbers;

        // If :defs was explicitly requested, return AST outline
        if selector.defs {
            let bytes = tokio::fs::read(&full_path).await.map_err(|e| {
                anyhow::anyhow!("Failed to read file '{}': {e}", full_path.display())
            })?;
            if bytes.is_empty() {
                return Ok("(empty file)".to_string());
            }
            let sniff_len = bytes.len().min(BINARY_SNIFF_BYTES);
            if bytes[..sniff_len].contains(&0) {
                anyhow::bail!("Cannot read binary file '{}'", full_path.display());
            }
            let content = String::from_utf8(bytes).map_err(|_| {
                anyhow::anyhow!("File '{}' is not valid UTF-8", full_path.display())
            })?;
            return render_defs_outline(&content, &full_path, line_numbers);
        }

        // Automatic outline/summary for oversized files read without explicit selector or limit
        let no_explicit_selector_or_limit = !selector.has_explicit_selector
            && args.get("limit").is_none()
            && args
                .get("offset")
                .map(|v| v.as_u64() == Some(1))
                .unwrap_or(true);

        if no_explicit_selector_or_limit && SupportLang::from_path(&full_path).is_some() {
            let bytes = tokio::fs::read(&full_path).await.map_err(|e| {
                anyhow::anyhow!("Failed to read file '{}': {e}", full_path.display())
            })?;
            if bytes.is_empty() {
                return Ok("(empty file)".to_string());
            }
            let sniff_len = bytes.len().min(BINARY_SNIFF_BYTES);
            if bytes[..sniff_len].contains(&0) {
                anyhow::bail!("Cannot read binary file '{}'", full_path.display());
            }
            let content = String::from_utf8(bytes).map_err(|_| {
                anyhow::anyhow!("File '{}' is not valid UTF-8", full_path.display())
            })?;

            let total_lines = content.lines().count();
            if total_lines > 500 {
                return render_oversized_summary(&content, &full_path, line_numbers);
            }

            let lines: Vec<String> = content.lines().map(|s| s.to_string()).collect();
            let output = format_read_output(&window, &lines, 1, total_lines);
            return Ok(output);
        }

        // Standard windowed/streaming read
        let file = tokio::fs::File::open(&full_path)
            .await
            .map_err(|e| anyhow::anyhow!("Failed to read file '{}': {e}", full_path.display()))?;
        let mut reader = BufReader::with_capacity(64 * 1024, file);

        let mut buf = Vec::with_capacity(256);
        let mut selected: Vec<String> = Vec::new();
        let mut total_lines = 0usize;
        let start_idx = window.offset.saturating_sub(1);
        let max_take = window.limit.unwrap_or(DEFAULT_READ_LIMIT);
        let mut empty = true;

        loop {
            buf.clear();
            let n = reader.read_until(b'\n', &mut buf).await.map_err(|e| {
                anyhow::anyhow!("Failed to read file '{}': {e}", full_path.display())
            })?;
            if n == 0 {
                break;
            }
            empty = false;

            if total_lines < BINARY_SNIFF_BYTES && buf.contains(&0) {
                anyhow::bail!("Cannot read binary file '{}'", full_path.display());
            }

            total_lines += 1;

            if total_lines > start_idx && selected.len() < max_take {
                // Trim trailing newline; strip a lone trailing \r from CRLF
                // files while preserving interior carriage returns.
                if buf.last() == Some(&b'\n') {
                    buf.pop();
                    if buf.last() == Some(&b'\r') {
                        buf.pop();
                    }
                }
                match String::from_utf8(buf.clone()) {
                    Ok(line) => selected.push(line),
                    Err(_) => anyhow::bail!(
                        "File '{}' is not valid UTF-8 (line {})",
                        full_path.display(),
                        total_lines
                    ),
                }
            }

            // Stop early once the window is full AND we have counted one more
            // line to know whether more content remains.
            if selected.len() >= max_take && total_lines > start_idx + max_take {
                break;
            }
        }

        if empty {
            return Ok("(empty file)".to_string());
        }

        // total_lines was only counted up to the early-exit point; when we
        // broke out early the file has more lines than counted, which the
        // footer below reports correctly via `more_remaining`.
        if window.offset > total_lines {
            anyhow::bail!(
                "Offset {} is beyond total lines ({}) in '{}'",
                window.offset,
                total_lines,
                path_str
            );
        }

        let start_line = start_idx + 1;
        let mut output = format_read_output(&window, &selected, start_line, total_lines);

        // When we broke out early (window full + one lookahead line), the
        // counted total is a lower bound; rewrite the footer so remaining
        // counts are never understated.
        if window.line_numbers
            && selected.len() >= max_take
            && total_lines == start_idx + max_take + 1
        {
            let more_marker = "\n... [";
            if let Some(pos) = output.rfind(more_marker) {
                output.truncate(pos);
                output.push_str(&format!(
                    "\n... [more lines in file (read lines {}-{}, stopped early; total: {}+)]\n",
                    start_line,
                    start_line + selected.len().saturating_sub(1),
                    total_lines
                ));
            }
        }

        Ok(output)
    }
}

/// Stream-read a text file line-by-line, returning the requested window
/// plus the total number of lines scanned. Exposed for reuse and testing.
pub async fn read_window_lines(
    path: &Path,
    window: &ReadWindow,
) -> anyhow::Result<(Vec<String>, usize)> {
    let file = tokio::fs::File::open(path)
        .await
        .map_err(|e| anyhow::anyhow!("Failed to read file '{}': {e}", path.display()))?;
    let mut reader = BufReader::with_capacity(64 * 1024, file);

    let mut buf = Vec::with_capacity(256);
    let mut selected: Vec<String> = Vec::new();
    let mut total = 0usize;
    let start_idx = window.offset.saturating_sub(1);
    let max_take = window.limit.unwrap_or(DEFAULT_READ_LIMIT);

    while total <= start_idx + max_take {
        buf.clear();
        let n = reader
            .read_until(b'\n', &mut buf)
            .await
            .map_err(|e| anyhow::anyhow!("Failed to read file '{}': {e}", path.display()))?;
        if n == 0 {
            break;
        }

        if buf.contains(&0) {
            anyhow::bail!("Cannot read binary file '{}'", path.display());
        }

        total += 1;
        if total > start_idx && selected.len() < max_take {
            if buf.last() == Some(&b'\n') {
                buf.pop();
                if buf.last() == Some(&b'\r') {
                    buf.pop();
                }
            }
            let line = String::from_utf8(buf.clone()).map_err(|e| {
                anyhow::anyhow!("File '{}' is not valid UTF-8: {e}", path.display())
            })?;
            selected.push(line);
        }
    }

    Ok((selected, total))
}

// ---------------------------------------------------------------------------
// WriteFileTool
// ---------------------------------------------------------------------------

#[derive(Default, Debug, Clone)]
pub struct WriteFileTool;

impl WriteFileTool {
    pub fn new() -> Self {
        Self
    }
}

pub type FileWriteTool = WriteFileTool;

#[async_trait]
impl Tool for WriteFileTool {
    fn name(&self) -> &str {
        "write"
    }

    fn description(&self) -> &str {
        "Write content to a file. Automatically creates parent directories if they do not exist."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "Path to the file to write (relative to workspace or absolute)."
                },
                "content": {
                    "type": "string",
                    "description": "Content to write into the file."
                }
            },
            "required": ["path", "content"]
        })
    }

    async fn execute(&self, args: Value, ctx: &ToolContext) -> anyhow::Result<String> {
        let path_str = args
            .get("path")
            .and_then(|v| v.as_str())
            .or_else(|| args.get("file_path").and_then(|v| v.as_str()))
            .ok_or_else(|| anyhow::anyhow!("Missing required parameter: path"))?;

        let content = args
            .get("content")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("Missing required parameter: content"))?;

        let full_path = resolve_path(path_str, &ctx.cwd);

        atomic_write(&full_path, content.as_bytes())
            .await
            .map_err(|e| anyhow::anyhow!("Failed to write file '{}': {e}", full_path.display()))?;

        let lines_count = content.lines().count();
        let bytes_count = content.len();

        Ok(format!(
            "Successfully wrote {} bytes ({} lines) to '{}'",
            bytes_count, lines_count, path_str
        ))
    }
}

// Re-export EditFileTool from the dedicated edit module
pub use crate::tools::edit::EditFileTool;

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    static COUNTER: AtomicUsize = AtomicUsize::new(0);

    fn temp_test_dir() -> PathBuf {
        let count = COUNTER.fetch_add(1, Ordering::SeqCst);
        let dir = std::env::temp_dir().join(format!(
            "fusion_file_test_{}_{}_{}",
            std::process::id(),
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0),
            count
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[tokio::test]
    async fn test_resolve_path() {
        let cwd = PathBuf::from("/workspace/fusion");
        assert_eq!(
            resolve_path("src/main.rs", &cwd),
            PathBuf::from("/workspace/fusion/src/main.rs")
        );
        #[cfg(unix)]
        assert_eq!(
            resolve_path("/etc/hosts", &cwd),
            PathBuf::from("/etc/hosts")
        );
    }

    #[tokio::test]
    async fn test_write_and_read_file_tool() {
        let dir = temp_test_dir();
        let ctx = ToolContext {
            cwd: dir.clone(),
            env: std::collections::HashMap::new(),
        };

        let write_tool = WriteFileTool::new();
        let read_tool = ReadFileTool::new();

        // 1. Write file with parent dir creation
        let write_res = write_tool
            .execute(
                json!({
                    "path": "nested/sub/dir/test.txt",
                    "content": "Line 1\nLine 2\nLine 3\nLine 4\nLine 5\n"
                }),
                &ctx,
            )
            .await;
        assert!(write_res.is_ok());
        assert!(write_res.unwrap().contains("Successfully wrote"));

        // Verify file was written
        let file_path = dir.join("nested/sub/dir/test.txt");
        assert!(file_path.exists());

        // 2. Read whole file with line numbers
        let read_res = read_tool
            .execute(
                json!({
                    "path": "nested/sub/dir/test.txt"
                }),
                &ctx,
            )
            .await
            .unwrap();
        assert!(read_res.contains("     1 | Line 1"));
        assert!(read_res.contains("     5 | Line 5"));

        // 3. Read without line numbers
        let raw_read = read_tool
            .execute(
                json!({
                    "path": "nested/sub/dir/test.txt",
                    "line_numbers": false
                }),
                &ctx,
            )
            .await
            .unwrap();
        assert_eq!(raw_read, "Line 1\nLine 2\nLine 3\nLine 4\nLine 5\n");

        // 4. Read with offset and limit
        let slice_read = read_tool
            .execute(
                json!({
                    "path": "nested/sub/dir/test.txt",
                    "offset": 2,
                    "limit": 2,
                    "line_numbers": true
                }),
                &ctx,
            )
            .await
            .unwrap();
        assert!(slice_read.contains("     2 | Line 2"));
        assert!(slice_read.contains("     3 | Line 3"));
        assert!(!slice_read.contains("     1 | Line 1"));
        assert!(!slice_read.contains("     4 | Line 4"));
        assert!(slice_read.contains("more lines in file"));

        // Cleanup
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn test_read_errors() {
        let dir = temp_test_dir();
        let ctx = ToolContext {
            cwd: dir.clone(),
            env: std::collections::HashMap::new(),
        };
        let read_tool = ReadFileTool::new();

        // File not found
        let err = read_tool
            .execute(json!({ "path": "nonexistent.txt" }), &ctx)
            .await
            .unwrap_err();
        assert!(err.to_string().contains("File not found"));

        // Path is a directory
        let sub_dir = dir.join("some_dir");
        std::fs::create_dir_all(&sub_dir).unwrap();
        let err = read_tool
            .execute(json!({ "path": "some_dir" }), &ctx)
            .await
            .unwrap_err();
        assert!(err.to_string().contains("is a directory"));

        // Binary file
        let bin_path = dir.join("binary.bin");
        std::fs::write(&bin_path, &[0x00, 0x01, 0x02, 0xFF]).unwrap();
        let err = read_tool
            .execute(json!({ "path": "binary.bin" }), &ctx)
            .await
            .unwrap_err();
        assert!(err.to_string().contains("Cannot read binary file"));

        // Invalid UTF-8 (without null bytes)
        let non_utf8_path = dir.join("invalid_utf8.txt");
        std::fs::write(&non_utf8_path, &[0x80, 0x81, 0x82, 0x83]).unwrap();
        let err = read_tool
            .execute(json!({ "path": "invalid_utf8.txt" }), &ctx)
            .await
            .unwrap_err();
        assert!(err.to_string().contains("not valid UTF-8"));

        // Offset beyond total lines
        let text_path = dir.join("short.txt");
        std::fs::write(&text_path, "one\ntwo\n").unwrap();
        let err = read_tool
            .execute(json!({ "path": "short.txt", "offset": 10 }), &ctx)
            .await
            .unwrap_err();
        assert!(err.to_string().contains("beyond total lines"));

        // Empty file
        let empty_path = dir.join("empty.txt");
        std::fs::write(&empty_path, "").unwrap();
        let empty_res = read_tool
            .execute(json!({ "path": "empty.txt" }), &ctx)
            .await
            .unwrap();
        assert_eq!(empty_res, "(empty file)");

        // Cleanup
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn test_atomic_write_creates_parents_and_overwrites() {
        let dir = temp_test_dir();
        let target = dir.join("a/b/c/target.txt");

        // 1. Write with parent creation
        atomic_write(&target, b"first\n").await.unwrap();
        assert_eq!(std::fs::read(&target).unwrap(), b"first\n");

        // 2. Overwrite leaves no temp siblings behind
        atomic_write(&target, b"second content\n").await.unwrap();
        assert_eq!(std::fs::read(&target).unwrap(), b"second content\n");

        let entries: Vec<_> = std::fs::read_dir(target.parent().unwrap())
            .unwrap()
            .filter_map(|e| e.ok())
            .collect();
        assert_eq!(entries.len(), 1, "temp file leaked: {:?}", entries);

        // 3. Empty write is valid
        atomic_write(&target, b"").await.unwrap();
        assert_eq!(std::fs::read(&target).unwrap(), b"");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn test_atomic_write_failure_preserves_existing() {
        let dir = temp_test_dir();
        let target = dir.join("keep.txt");
        atomic_write(&target, b"original\n").await.unwrap();

        // Writing over a directory path must fail without clobbering the
        // original file content.
        let sub_dir = dir.join("sub");
        std::fs::create_dir_all(&sub_dir).unwrap();
        assert!(atomic_write(&sub_dir, b"nope\n").await.is_err());

        assert_eq!(std::fs::read(&target).unwrap(), b"original\n");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_parse_read_window_defaults_and_clamps() {
        let w = parse_read_window(&json!({}));
        assert_eq!(w.offset, 1);
        assert_eq!(w.limit, None);
        assert!(w.line_numbers);

        let w = parse_read_window(&json!({ "offset": 0, "limit": 10, "line_numbers": false }));
        assert_eq!(w.offset, 1, "offset 0 must clamp to 1");
        assert_eq!(w.limit, Some(10));
        assert!(!w.line_numbers);
    }

    #[test]
    fn test_format_read_output_numbering_and_footer() {
        let window = ReadWindow {
            offset: 3,
            limit: Some(2),
            line_numbers: true,
        };
        let selected = vec!["three".to_string(), "four".to_string()];
        let out = format_read_output(&window, &selected, 3, 10);

        assert!(out.contains("     3 | three\n"));
        assert!(out.contains("     4 | four\n"));
        assert!(!out.contains("     5 | "));
        assert!(out.contains("[6 more lines in file (total: 10)]"));
    }

    #[test]
    fn test_format_read_output_no_footer_at_eof() {
        let window = ReadWindow {
            offset: 1,
            limit: Some(5),
            line_numbers: false,
        };
        let selected = vec!["a".to_string(), "b".to_string()];
        let out = format_read_output(&window, &selected, 1, 2);

        assert_eq!(out, "a\nb\n", "no footer expected at EOF, got: {out:?}");
    }

    #[tokio::test]
    async fn test_read_window_lines_streaming() {
        let dir = temp_test_dir();
        let path = dir.join("stream.txt");
        let content = (1..=500).map(|i| format!("line {i}\n")).collect::<String>();
        std::fs::write(&path, &content).unwrap();

        // Window in the middle
        let window = ReadWindow {
            offset: 100,
            limit: Some(3),
            line_numbers: true,
        };
        let (selected, total) = read_window_lines(&path, &window).await.unwrap();
        assert_eq!(selected.len(), 3);
        assert_eq!(selected[0], "line 100");
        assert_eq!(selected[2], "line 102");
        assert_eq!(total, 103);

        // Window past EOF
        let window = ReadWindow {
            offset: 600,
            limit: Some(5),
            line_numbers: false,
        };
        let (selected, total) = read_window_lines(&path, &window).await.unwrap();
        assert!(selected.is_empty());
        assert_eq!(total, 500);

        // Binary file must bail
        let bin_path = dir.join("binary.bin");
        std::fs::write(&bin_path, &[0x00, 0x01, 0x02]).unwrap();
        let window = ReadWindow {
            offset: 1,
            limit: None,
            line_numbers: false,
        };
        let err = read_window_lines(&bin_path, &window).await.unwrap_err();
        assert!(err.to_string().contains("binary"));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn test_read_tool_default_limit_truncates() {
        let dir = temp_test_dir();
        let ctx = ToolContext {
            cwd: dir.clone(),
            env: std::collections::HashMap::new(),
        };
        let read_tool = ReadFileTool::new();

        // More lines than DEFAULT_READ_LIMIT
        let path = dir.join("big.txt");
        let content = (1..=(DEFAULT_READ_LIMIT + 50))
            .map(|i| format!("L{i}\n"))
            .collect::<String>();
        std::fs::write(&path, &content).unwrap();

        let res = read_tool
            .execute(json!({ "path": "big.txt" }), &ctx)
            .await
            .unwrap();
        assert!(res.contains(&format!("{:6} | L1\n", 1)));
        assert!(res.contains("more lines in file"));
        assert!(!res.contains(&format!(
            "{:6} | L{}\n",
            DEFAULT_READ_LIMIT + 1,
            DEFAULT_READ_LIMIT + 1
        )));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn test_walk_dir_helper() {
        let dir = temp_test_dir();
        std::fs::write(dir.join("visible.txt"), "hello").unwrap();
        std::fs::write(dir.join(".hidden.txt"), "secret").unwrap();
        let sub = dir.join("sub");
        std::fs::create_dir_all(&sub).unwrap();
        std::fs::write(sub.join("nested.txt"), "world").unwrap();
        std::fs::write(dir.join(".gitignore"), "sub/\n").unwrap();

        let request = walk_dir(&dir, false);
        let outcome = request.collect().expect("walk should succeed");
        let paths: Vec<_> = outcome.entries.into_iter().map(|e| e.path).collect();

        assert!(paths.contains(&"visible.txt".to_string()));
        assert!(!paths.contains(&".hidden.txt".to_string()));
        assert!(!paths.contains(&"sub/nested.txt".to_string()));

        let _ = std::fs::remove_dir_all(&dir);
    }
    #[test]
    fn test_parse_path_selector_variants() {
        let (path, sel) = parse_path_selector("src/main.rs:10-40");
        assert_eq!(path, "src/main.rs");
        assert_eq!(sel.range, Some((10, Some(40))));
        assert!(!sel.raw);
        assert!(!sel.defs);
        assert!(sel.has_explicit_selector);

        let (path, sel) = parse_path_selector("src/main.rs:50");
        assert_eq!(path, "src/main.rs");
        assert_eq!(sel.range, Some((50, None)));
        assert!(!sel.raw);
        assert!(sel.has_explicit_selector);

        let (path, sel) = parse_path_selector("src/main.rs:raw");
        assert_eq!(path, "src/main.rs");
        assert!(sel.raw);
        assert!(sel.has_explicit_selector);

        let (path, sel) = parse_path_selector("src/main.rs:defs");
        assert_eq!(path, "src/main.rs");
        assert!(sel.defs);
        assert!(sel.has_explicit_selector);

        let (path, sel) = parse_path_selector("src/main.rs:10-40:raw");
        assert_eq!(path, "src/main.rs");
        assert_eq!(sel.range, Some((10, Some(40))));
        assert!(sel.raw);
        assert!(sel.has_explicit_selector);

        let (path, sel) = parse_path_selector("src/main.rs");
        assert_eq!(path, "src/main.rs");
        assert!(!sel.has_explicit_selector);
        assert_eq!(sel.range, None);
    }

    #[tokio::test]
    async fn test_read_tool_line_selectors() {
        let dir = temp_test_dir();
        let ctx = ToolContext {
            cwd: dir.clone(),
            env: std::collections::HashMap::new(),
        };
        let read_tool = ReadFileTool::new();
        let file_path = dir.join("test_lines.txt");
        let content = (1..=10).map(|i| format!("Line {i}\n")).collect::<String>();
        std::fs::write(&file_path, content).unwrap();

        // 1. :2-4 slicing
        let res = read_tool
            .execute(json!({ "path": "test_lines.txt:2-4" }), &ctx)
            .await
            .unwrap();
        assert!(res.contains("     2 | Line 2"));
        assert!(res.contains("     4 | Line 4"));
        assert!(!res.contains("Line 1"));
        assert!(!res.contains("Line 5"));

        // 2. :8 offset
        let res = read_tool
            .execute(json!({ "path": "test_lines.txt:8" }), &ctx)
            .await
            .unwrap();
        assert!(res.contains("     8 | Line 8"));
        assert!(res.contains("    10 | Line 10"));
        assert!(!res.contains("Line 7"));

        // 3. :raw
        let res = read_tool
            .execute(json!({ "path": "test_lines.txt:raw" }), &ctx)
            .await
            .unwrap();
        assert_eq!(res, (1..=10).map(|i| format!("Line {i}\n")).collect::<String>());

        // 4. :2-4:raw
        let res = read_tool
            .execute(json!({ "path": "test_lines.txt:2-4:raw" }), &ctx)
            .await
            .unwrap();
        assert_eq!(res, "Line 2\nLine 3\nLine 4\n");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn test_read_tool_defs_and_oversized_summary() {
        let dir = temp_test_dir();
        let ctx = ToolContext {
            cwd: dir.clone(),
            env: std::collections::HashMap::new(),
        };
        let read_tool = ReadFileTool::new();

        // 1. Test :defs on Rust code
        let code_path = dir.join("lib.rs");
        let code = "pub fn add(a: i32, b: i32) -> i32 {\n    let sum = a + b;\n    sum\n}\n\npub struct Item {\n    pub id: u32,\n}\n";
        std::fs::write(&code_path, code).unwrap();

        let res = read_tool
            .execute(json!({ "path": "lib.rs:defs" }), &ctx)
            .await
            .unwrap();
        assert!(res.contains("pub fn add"));
        assert!(res.contains("pub struct Item"));
        assert!(res.contains("..."));

        // 2. Test >500 lines automatic outline summary
        let big_code_path = dir.join("big_code.rs");
        let mut big_code = String::new();
        for i in 0..110 {
            big_code.push_str(&format!(
                "pub fn func_{i}() -> usize {{\n    let a = {i};\n    let b = a + 1;\n    b\n}}\n\n"
            ));
        }
        assert!(big_code.lines().count() > 500);
        std::fs::write(&big_code_path, &big_code).unwrap();

        let res = read_tool
            .execute(json!({ "path": "big_code.rs" }), &ctx)
            .await
            .unwrap();
        assert!(res.contains("Summary:"));
        assert!(res.contains("lines elided; re-issue with line range selector"));
        assert!(res.contains("..."));

        let _ = std::fs::remove_dir_all(&dir);
    }
}
