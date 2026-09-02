//! Regular expression evaluation, testing, capture inspection, syntax validation,
//! and construct explanation tool.
//!
//! Provides rich regex testing and analysis capabilities:
//! - **Match & Capture Group Extraction**: Evaluates regex patterns against single or multiple
//!   test strings, reporting all matches, numbered capture groups (`$1`, `$2`, ...), and named
//!   capture groups (`(?P<name>...)`).
//! - **Accurate Span Tracking**: Calculates 0-indexed byte ranges, 0-indexed char ranges,
//!   and 1-indexed (line, column) locations for matches and every capture group, with full
//!   UTF-8 multi-byte support.
//! - **Flag & Option Customization**: Supports case-insensitivity (`i`), multi-line mode (`m`),
//!   dot-matches-all / single-line mode (`s`), ignore whitespace / extended (`x`), swap greed (`U`),
//!   and unicode mode (`u`).
//! - **Replacement & Substitution Testing**: Evaluates `$1` / `${name}` replacement templates against
//!   matching text (first occurrence or all occurrences).
//! - **Splitting Testing**: Evaluates string splitting based on regex delimiters.
//! - **Construct Explanation & Breakdown**: Tokenizes and provides human-readable explanations of
//!   quantifiers, character classes, anchors, capture groups, lookarounds, and escape sequences.
//! - **Syntax Diagnostics & Lints**: Detects regex compilation errors with precise diagnostics and
//!   offers lint warnings for common pitfalls (unescaped dots in domains/filenames, unsupported lookarounds,
//!   backreferences, ReDoS risk).

use async_trait::async_trait;
use regex::{Regex, RegexBuilder};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashMap;

use crate::tools::file::resolve_path;
use crate::tools::types::{Tool, ToolContext};

// ===========================================================================
// Data Models: Spans, Capture Groups, Matches, Explanations, Reports
// ===========================================================================

/// Source location span in byte offsets, character offsets, and 1-based (line, column) numbers.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Span {
    /// 0-indexed start byte offset.
    pub start: usize,
    /// 0-indexed end byte offset (exclusive).
    pub end: usize,
    /// 0-indexed start character offset.
    pub char_start: usize,
    /// 0-indexed end character offset (exclusive).
    pub char_end: usize,
    /// 1-indexed starting line number.
    pub start_line: usize,
    /// 1-indexed starting column number (character offset within line).
    pub start_col: usize,
    /// 1-indexed ending line number.
    pub end_line: usize,
    /// 1-indexed ending column number (character offset within line).
    pub end_col: usize,
}

impl Span {
    /// Human-readable location description, e.g. "L1:C5-L1:C12 (bytes 4..11)".
    pub fn format_location(&self) -> String {
        if self.start_line == self.end_line {
            format!(
                "L{}:C{}-C{} (bytes {}..{})",
                self.start_line, self.start_col, self.end_col, self.start, self.end
            )
        } else {
            format!(
                "L{}:C{}-L{}:C{} (bytes {}..{})",
                self.start_line, self.start_col, self.end_line, self.end_col, self.start, self.end
            )
        }
    }
}

/// A captured group within a regex match.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CaptureGroup {
    /// 0-indexed capture group index (0 is the full match, 1 is the first group, etc.).
    pub index: usize,
    /// Optional capture group name if defined as `(?P<name>...)` or `(?<name>...)`.
    pub name: Option<String>,
    /// Matched substring text for this group.
    pub text: String,
    /// Location span of this capture group.
    pub span: Span,
}

/// A single regular expression match occurrence in an input string.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MatchItem {
    /// 0-indexed sequence index of this match in the input.
    pub match_index: usize,
    /// Full matched substring.
    pub text: String,
    /// Full match location span.
    pub span: Span,
    /// All capture groups for this match (including group 0 full match).
    pub groups: Vec<CaptureGroup>,
    /// Map of named capture groups (`group_name` -> `captured_text`).
    pub named_groups: HashMap<String, String>,
}

/// Result of evaluating the regular expression against a single test input string.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TestStringResult {
    /// 0-indexed index of the input string in the test batch.
    pub input_index: usize,
    /// Original test string content.
    pub input: String,
    /// Whether any match was found.
    pub matched: bool,
    /// Total number of matches found in this test string.
    pub match_count: usize,
    /// List of match occurrences.
    pub matches: Vec<MatchItem>,
    /// Substituted string result if replacement was requested.
    pub replacement: Option<String>,
    /// Split string segments if split was requested.
    pub splits: Option<Vec<String>>,
}

/// Configuration flags for regex compilation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RegexFlags {
    /// Case-sensitive matching (default: true).
    pub case_sensitive: bool,
    /// Multiline mode: `^` and `$` match line starts/ends (default: false).
    pub multiline: bool,
    /// Singleline / Dot-matches-all: `.` matches `\n` (default: false).
    pub dot_matches_all: bool,
    /// Extended mode: ignore pattern whitespace and allow `#` comments (default: false).
    pub ignore_whitespace: bool,
    /// Swap greediness of `*`, `+`, `?` quantifiers (default: false).
    pub swap_greed: bool,
    /// Unicode character class support (default: true).
    pub unicode: bool,
}

impl Default for RegexFlags {
    fn default() -> Self {
        Self {
            case_sensitive: true,
            multiline: false,
            dot_matches_all: false,
            ignore_whitespace: false,
            swap_greed: false,
            unicode: true,
        }
    }
}

impl RegexFlags {
    /// Parses flags from a flag character string like `"imsxU"`.
    pub fn parse_flag_str(&mut self, flags_str: &str) {
        for ch in flags_str.chars() {
            match ch {
                'i' | 'I' => self.case_sensitive = false,
                'm' | 'M' => self.multiline = true,
                's' | 'S' => self.dot_matches_all = true,
                'x' | 'X' => self.ignore_whitespace = true,
                'U' => self.swap_greed = true,
                'u' => self.unicode = true,
                _ => {}
            }
        }
    }

    /// Formats active flags into a standard flag string (e.g. `"im"`).
    pub fn to_flag_string(&self) -> String {
        let mut s = String::new();
        if !self.case_sensitive {
            s.push('i');
        }
        if self.multiline {
            s.push('m');
        }
        if self.dot_matches_all {
            s.push('s');
        }
        if self.ignore_whitespace {
            s.push('x');
        }
        if self.swap_greed {
            s.push('U');
        }
        if !self.unicode {
            s.push_str("(no-unicode)");
        }
        s
    }
}

/// Metadata about the compiled regex pattern.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RegexMetadata {
    /// The original pattern string.
    pub pattern: String,
    /// Display summary of pattern with flags, e.g. `"/pattern/im"`.
    pub pattern_display: String,
    /// Active flags.
    pub flags: RegexFlags,
    /// Active flag string, e.g. `"im"`.
    pub flags_str: String,
    /// Total number of capture groups in pattern (excluding full match 0).
    pub capture_group_count: usize,
    /// Names of named capture groups in pattern.
    pub named_group_names: Vec<String>,
    /// All capture group names (None for anonymous indexed groups).
    pub all_group_names: Vec<Option<String>>,
    /// Helpful lint warnings or observations.
    pub warnings: Vec<String>,
}

/// Detailed error diagnostic when a regex fails to compile.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RegexErrorDetail {
    /// Primary error message.
    pub message: String,
    /// Specific suggestion or hint to fix the pattern.
    pub suggestion: Option<String>,
}

/// Category kind for an individual regex construct.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ConstructKind {
    Anchor,
    CharacterClass,
    Quantifier,
    Group,
    Lookaround,
    Alternation,
    Escape,
    Literal,
    Comment,
}

/// Breakdown of an individual regex construct within the pattern.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RegexConstruct {
    /// The exact substring in the pattern for this construct.
    pub raw: String,
    /// Classification kind of this construct.
    pub kind: ConstructKind,
    /// Human-readable explanation of what this construct does.
    pub description: String,
    /// 0-indexed byte offset start in pattern.
    pub start: usize,
    /// 0-indexed byte offset end in pattern (exclusive).
    pub end: usize,
    /// Additional detailed notes or compatibility warnings.
    pub notes: Option<String>,
}

/// Structured explanation of all constructs comprising the regex pattern.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RegexExplanation {
    /// Full pattern being explained.
    pub pattern: String,
    /// High-level summary of the pattern structure.
    pub summary: String,
    /// Sequential breakdown of constructs in the pattern.
    pub constructs: Vec<RegexConstruct>,
    /// Flags affecting the interpretation of constructs.
    pub flags: RegexFlags,
}

/// Overall report summarizing the regex evaluation results.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RegexEvaluationReport {
    /// Whether the regex compiled successfully.
    pub valid: bool,
    /// The tested pattern.
    pub pattern: String,
    /// Error detail if compilation failed.
    pub error: Option<RegexErrorDetail>,
    /// Pattern metadata if valid.
    pub metadata: Option<RegexMetadata>,
    /// Human-readable explanation of regex constructs.
    pub explanation: Option<RegexExplanation>,
    /// Evaluation results per test string input.
    pub results: Vec<TestStringResult>,
    /// Total number of input strings evaluated.
    pub total_inputs: usize,
    /// Number of input strings with at least one match.
    pub matched_inputs: usize,
    /// Total match count across all input strings.
    pub total_matches: usize,
}

// ===========================================================================
// Source Location Mapper (UTF-8 Aware)
// ===========================================================================

/// Fast, exact mapping from byte offsets to 1-based (line, column) and character offsets.
pub struct SourceMap<'a> {
    text: &'a str,
    line_starts: Vec<usize>,
}

impl<'a> SourceMap<'a> {
    pub fn new(text: &'a str) -> Self {
        let mut line_starts = vec![0];
        for (byte_idx, b) in text.bytes().enumerate() {
            if b == b'\n' {
                line_starts.push(byte_idx + 1);
            }
        }
        Self { text, line_starts }
    }

    /// Computes (line, column, char_offset) for a given byte offset safely.
    pub fn lookup(&self, byte_offset: usize) -> (usize, usize, usize) {
        let mut clamped_offset = byte_offset.min(self.text.len());
        while clamped_offset > 0 && !self.text.is_char_boundary(clamped_offset) {
            clamped_offset = clamped_offset.saturating_sub(1);
        }

        // Find line index (0-based) via binary search
        let line_idx = match self.line_starts.binary_search(&clamped_offset) {
            Ok(idx) => idx,
            Err(idx) => idx.saturating_sub(1),
        };

        let line_byte_start = self.line_starts.get(line_idx).copied().unwrap_or(0).min(clamped_offset);
        let line_slice = self.text.get(line_byte_start..clamped_offset).unwrap_or("");
        let col = line_slice.chars().count() + 1; // 1-indexed column

        let total_prefix = self.text.get(..clamped_offset).unwrap_or("");
        let char_offset = total_prefix.chars().count(); // 0-indexed char offset

        (line_idx + 1, col, char_offset)
    }

    /// Creates a complete `Span` for a byte range `start..end`.
    pub fn span(&self, start: usize, end: usize) -> Span {
        let (start_line, start_col, char_start) = self.lookup(start);
        let (end_line, end_col, char_end) = self.lookup(end);
        Span {
            start,
            end,
            char_start,
            char_end,
            start_line,
            start_col,
            end_line,
            end_col,
        }
    }
}

// ===========================================================================
// Regex Construct Explainer
// ===========================================================================

/// Parses and breaks down a regex pattern into human-readable construct explanations.
pub fn explain_pattern(pattern: &str, flags: &RegexFlags) -> RegexExplanation {
    let mut constructs = Vec::new();
    let bytes = pattern.as_bytes();
    let len = pattern.len();
    let mut i = 0;
    let mut group_counter = 0;
    let mut group_stack: Vec<(&'static str, usize)> = Vec::new();

    while i < len {
        let b = bytes[i];
        let start = i;

        // Check if extended mode whitespace or comments
        if flags.ignore_whitespace {
            if b.is_ascii_whitespace() {
                let ws_start = i;
                while i < len && bytes[i].is_ascii_whitespace() {
                    i += 1;
                }
                let raw = pattern.get(ws_start..i).unwrap_or("").to_string();
                constructs.push(RegexConstruct {
                    raw,
                    kind: ConstructKind::Comment,
                    description: "Ignored whitespace (extended mode 'x')".to_string(),
                    start: ws_start,
                    end: i,
                    notes: None,
                });
                continue;
            }
            if b == b'#' {
                let comment_start = i;
                while i < len && bytes[i] != b'\n' {
                    i += 1;
                }
                let raw = pattern.get(comment_start..i).unwrap_or("").to_string();
                constructs.push(RegexConstruct {
                    raw,
                    kind: ConstructKind::Comment,
                    description: "Comment (extended mode 'x')".to_string(),
                    start: comment_start,
                    end: i,
                    notes: None,
                });
                continue;
            }
        }

        match b {
            b'^' => {
                i += 1;
                let desc = if flags.multiline {
                    "Anchor: Beginning of a line (multiline mode 'm')"
                } else {
                    "Anchor: Beginning of the entire string"
                };
                constructs.push(RegexConstruct {
                    raw: "^".to_string(),
                    kind: ConstructKind::Anchor,
                    description: desc.to_string(),
                    start,
                    end: i,
                    notes: None,
                });
            }
            b'$' => {
                i += 1;
                let desc = if flags.multiline {
                    "Anchor: End of a line (multiline mode 'm')"
                } else {
                    "Anchor: End of the entire string"
                };
                constructs.push(RegexConstruct {
                    raw: "$".to_string(),
                    kind: ConstructKind::Anchor,
                    description: desc.to_string(),
                    start,
                    end: i,
                    notes: None,
                });
            }
            b'.' => {
                i += 1;
                let desc = if flags.dot_matches_all {
                    "Character class: Matches ANY character including newline '\\n' (dot-matches-all mode 's')"
                } else {
                    "Character class: Matches any character EXCEPT newline '\\n'"
                };
                constructs.push(RegexConstruct {
                    raw: ".".to_string(),
                    kind: ConstructKind::CharacterClass,
                    description: desc.to_string(),
                    start,
                    end: i,
                    notes: None,
                });
            }
            b'|' => {
                i += 1;
                constructs.push(RegexConstruct {
                    raw: "|".to_string(),
                    kind: ConstructKind::Alternation,
                    description: "Alternation (OR): Matches expression on left OR expression on right".to_string(),
                    start,
                    end: i,
                    notes: None,
                });
            }
            b'\\' => {
                i += 1;
                if i >= len {
                    constructs.push(RegexConstruct {
                        raw: "\\".to_string(),
                        kind: ConstructKind::Escape,
                        description: "Trailing backslash escape (incomplete)".to_string(),
                        start,
                        end: i,
                        notes: Some("Pattern ends with a dangling backslash".to_string()),
                    });
                    break;
                }

                let esc_ch = pattern.get(i..).and_then(|s| s.chars().next()).unwrap_or('\\');
                let esc_len = esc_ch.len_utf8();
                i += esc_len;

                let (kind, desc, notes) = match esc_ch {
                    'b' => (
                        ConstructKind::Anchor,
                        "Word boundary: Matches at transition between word (\\w) and non-word (\\W) characters".to_string(),
                        None,
                    ),
                    'B' => (
                        ConstructKind::Anchor,
                        "Non-word boundary: Matches at any position that is not a word boundary".to_string(),
                        None,
                    ),
                    'A' => (
                        ConstructKind::Anchor,
                        "Absolute start of text: Matches beginning of text regardless of multiline mode".to_string(),
                        None,
                    ),
                    'z' => (
                        ConstructKind::Anchor,
                        "Absolute end of text: Matches end of text regardless of multiline mode".to_string(),
                        None,
                    ),
                    'd' => (
                        ConstructKind::CharacterClass,
                        "Digit character class: Matches any ASCII digit [0-9]".to_string(),
                        None,
                    ),
                    'D' => (
                        ConstructKind::CharacterClass,
                        "Non-digit character class: Matches any character except ASCII digits [^0-9]".to_string(),
                        None,
                    ),
                    'w' => (
                        ConstructKind::CharacterClass,
                        if flags.unicode {
                            "Word character class: Matches ASCII [a-zA-Z0-9_] and Unicode word characters".to_string()
                        } else {
                            "Word character class: Matches ASCII letters, digits, and underscore [a-zA-Z0-9_]".to_string()
                        },
                        None,
                    ),
                    'W' => (
                        ConstructKind::CharacterClass,
                        "Non-word character class: Matches any character except word characters [^a-zA-Z0-9_]".to_string(),
                        None,
                    ),
                    's' => (
                        ConstructKind::CharacterClass,
                        "Whitespace character class: Matches space, tab, newline, carriage return, form feed".to_string(),
                        None,
                    ),
                    'S' => (
                        ConstructKind::CharacterClass,
                        "Non-whitespace character class: Matches any non-whitespace character".to_string(),
                        None,
                    ),
                    'n' => (
                        ConstructKind::Escape,
                        "Newline character escape (LF, \\n, U+000A)".to_string(),
                        None,
                    ),
                    'r' => (
                        ConstructKind::Escape,
                        "Carriage return escape (CR, \\r, U+000D)".to_string(),
                        None,
                    ),
                    't' => (
                        ConstructKind::Escape,
                        "Horizontal tab escape (HT, \\t, U+0009)".to_string(),
                        None,
                    ),
                    '0' => (
                        ConstructKind::Escape,
                        "Null character escape (NUL, U+0000)".to_string(),
                        None,
                    ),
                    'p' | 'P' => {
                        if i < len && bytes[i] == b'{' {
                            let prop_start = i + 1;
                            while i < len && bytes[i] != b'}' {
                                i += 1;
                            }
                            if i < len && bytes[i] == b'}' {
                                i += 1; // consume '}'
                                let prop_name = pattern.get(prop_start..i - 1).unwrap_or("");
                                let is_negated = esc_ch == 'P';
                                let desc = if is_negated {
                                    format!("Negated Unicode property: Matches characters NOT in property '{}'", prop_name)
                                } else {
                                    format!("Unicode property: Matches characters with property '{}'", prop_name)
                                };
                                (ConstructKind::CharacterClass, desc, None)
                            } else {
                                (ConstructKind::CharacterClass, "Unclosed Unicode property escape".to_string(), Some("Missing closing '}'".to_string()))
                            }
                        } else {
                            (ConstructKind::Escape, format!("Literal escaped character '{}'", esc_ch), None)
                        }
                    }
                    'x' => {
                        let hex_start = i;
                        while i < len && i - hex_start < 2 && pattern.get(i..).and_then(|s| s.chars().next()).map_or(false, |c| c.is_ascii_hexdigit()) {
                            i += 1;
                        }
                        let hex_val = pattern.get(hex_start..i).unwrap_or("");
                        (ConstructKind::Escape, format!("Hexadecimal ASCII byte escape \\x{}", hex_val), None)
                    }
                    'u' => {
                        if i < len && bytes[i] == b'{' {
                            let u_start = i + 1;
                            while i < len && bytes[i] != b'}' {
                                i += 1;
                            }
                            if i < len && bytes[i] == b'}' {
                                i += 1;
                                let u_val = pattern.get(u_start..i - 1).unwrap_or("");
                                (ConstructKind::Escape, format!("Unicode code point escape \\u{{{}}}", u_val), None)
                            } else {
                                (ConstructKind::Escape, "Unclosed Unicode code point escape".to_string(), Some("Missing closing '}'".to_string()))
                            }
                        } else {
                            let u_start = i;
                            while i < len && i - u_start < 4 && pattern.get(i..).and_then(|s| s.chars().next()).map_or(false, |c| c.is_ascii_hexdigit()) {
                                i += 1;
                            }
                            let u_val = pattern.get(u_start..i).unwrap_or("");
                            (ConstructKind::Escape, format!("Unicode escape \\u{}", u_val), None)
                        }
                    }
                    '1'..='9' => {
                        (
                            ConstructKind::Escape,
                            format!("Backreference to group \\{} (Note: Not supported in standard Rust regex)", esc_ch),
                            Some("Standard Rust regex does not support backreferences".to_string()),
                        )
                    }
                    '.' | '*' | '+' | '?' | '(' | ')' | '[' | ']' | '{' | '}' | '|' | '^' | '$' | '\\' | '/' | '-' => {
                        (
                            ConstructKind::Literal,
                            format!("Escaped literal character '{}'", esc_ch),
                            None,
                        )
                    }
                    other => {
                        (
                            ConstructKind::Escape,
                            format!("Escaped character '\\{}'", other),
                            None,
                        )
                    }
                };

                let raw = pattern.get(start..i).unwrap_or("").to_string();
                constructs.push(RegexConstruct {
                    raw,
                    kind,
                    description: desc,
                    start,
                    end: i,
                    notes,
                });
            }
            b'[' => {
                i += 1;
                let is_negated = if i < len && bytes[i] == b'^' {
                    i += 1;
                    true
                } else {
                    false
                };

                if i < len && bytes[i] == b']' {
                    i += 1;
                }

                while i < len {
                    if bytes[i] == b'\\' {
                        i += 1;
                        if i < len {
                            if let Some(ch) = pattern.get(i..).and_then(|s| s.chars().next()) {
                                i += ch.len_utf8();
                            }
                        }
                    } else if bytes[i] == b']' {
                        i += 1;
                        break;
                    } else if let Some(ch) = pattern.get(i..).and_then(|s| s.chars().next()) {
                        i += ch.len_utf8();
                    } else {
                        i += 1;
                    }
                }

                let raw = pattern.get(start..i).unwrap_or("").to_string();
                let inner = if is_negated {
                    if raw.len() > 3 && raw.ends_with(']') {
                        raw.get(2..raw.len() - 1).unwrap_or("")
                    } else {
                        raw.get(2..).unwrap_or("")
                    }
                } else if raw.len() > 2 && raw.ends_with(']') {
                    raw.get(1..raw.len() - 1).unwrap_or("")
                } else {
                    raw.get(1..).unwrap_or("")
                };

                let class_desc = explain_character_class_inner(inner, is_negated);
                let notes = if !raw.ends_with(']') {
                    Some("Unclosed character class: missing closing ']'".to_string())
                } else {
                    None
                };

                constructs.push(RegexConstruct {
                    raw,
                    kind: ConstructKind::CharacterClass,
                    description: class_desc,
                    start,
                    end: i,
                    notes,
                });
            }
            b'(' => {
                if i + 1 < len && bytes[i + 1] == b'?' {
                    if i + 2 < len {
                        match bytes[i + 2] {
                            b'=' => {
                                i += 3;
                                constructs.push(RegexConstruct {
                                    raw: "(?=".to_string(),
                                    kind: ConstructKind::Lookaround,
                                    description: "Positive lookahead: Asserts that following text matches pattern without consuming characters".to_string(),
                                    start,
                                    end: i,
                                    notes: Some("Lookaheads are NOT supported in standard linear-time Rust regex".to_string()),
                                });
                                group_stack.push(("positive_lookahead", start));
                            }
                            b'!' => {
                                i += 3;
                                constructs.push(RegexConstruct {
                                    raw: "(?!".to_string(),
                                    kind: ConstructKind::Lookaround,
                                    description: "Negative lookahead: Asserts that following text does NOT match pattern without consuming characters".to_string(),
                                    start,
                                    end: i,
                                    notes: Some("Lookaheads are NOT supported in standard linear-time Rust regex".to_string()),
                                });
                                group_stack.push(("negative_lookahead", start));
                            }
                            b'<' => {
                                if i + 3 < len && bytes[i + 3] == b'=' {
                                    i += 4;
                                    constructs.push(RegexConstruct {
                                        raw: "(?<=".to_string(),
                                        kind: ConstructKind::Lookaround,
                                        description: "Positive lookbehind: Asserts that preceding text matches pattern without consuming characters".to_string(),
                                        start,
                                        end: i,
                                        notes: Some("Lookbehinds are NOT supported in standard linear-time Rust regex".to_string()),
                                    });
                                    group_stack.push(("positive_lookbehind", start));
                                } else if i + 3 < len && bytes[i + 3] == b'!' {
                                    i += 4;
                                    constructs.push(RegexConstruct {
                                        raw: "(?<!".to_string(),
                                        kind: ConstructKind::Lookaround,
                                        description: "Negative lookbehind: Asserts that preceding text does NOT match pattern without consuming characters".to_string(),
                                        start,
                                        end: i,
                                        notes: Some("Lookbehinds are NOT supported in standard linear-time Rust regex".to_string()),
                                    });
                                    group_stack.push(("negative_lookbehind", start));
                                } else {
                                    let name_start = i + 3;
                                    let mut name_end = name_start;
                                    while name_end < len && bytes[name_end] != b'>' && bytes[name_end] != b')' {
                                        name_end += 1;
                                    }
                                    let name = if name_end < len && bytes[name_end] == b'>' {
                                        let n = pattern.get(name_start..name_end).unwrap_or("unknown");
                                        i = name_end + 1;
                                        n
                                    } else {
                                        i = name_end;
                                        "unknown"
                                    };
                                    group_counter += 1;
                                    let raw = pattern.get(start..i).unwrap_or("").to_string();
                                    constructs.push(RegexConstruct {
                                        raw,
                                        kind: ConstructKind::Group,
                                        description: format!("Named capture group '{}' (#{}): Captures matched text", name, group_counter),
                                        start,
                                        end: i,
                                        notes: None,
                                    });
                                    group_stack.push(("named_group", start));
                                }
                            }
                            b'P' => {
                                if i + 3 < len && bytes[i + 3] == b'<' {
                                    let name_start = i + 4;
                                    let mut name_end = name_start;
                                    while name_end < len && bytes[name_end] != b'>' && bytes[name_end] != b')' {
                                        name_end += 1;
                                    }
                                    let name = if name_end < len && bytes[name_end] == b'>' {
                                        let n = pattern.get(name_start..name_end).unwrap_or("unknown");
                                        i = name_end + 1;
                                        n
                                    } else {
                                        i = name_end;
                                        "unknown"
                                    };
                                    group_counter += 1;
                                    let raw = pattern.get(start..i).unwrap_or("").to_string();
                                    constructs.push(RegexConstruct {
                                        raw,
                                        kind: ConstructKind::Group,
                                        description: format!("Named capture group '{}' (#{}): Captures matched text", name, group_counter),
                                        start,
                                        end: i,
                                        notes: None,
                                    });
                                    group_stack.push(("named_group", start));
                                } else {
                                    i += 3;
                                    constructs.push(RegexConstruct {
                                        raw: "(?P".to_string(),
                                        kind: ConstructKind::Group,
                                        description: "Malformed named group syntax".to_string(),
                                        start,
                                        end: i,
                                        notes: Some("Expected '<name>' after (?P".to_string()),
                                    });
                                }
                            }
                            b':' => {
                                i += 3;
                                constructs.push(RegexConstruct {
                                    raw: "(?:".to_string(),
                                    kind: ConstructKind::Group,
                                    description: "Non-capturing group: Groups subexpressions for quantifiers without saving a capture".to_string(),
                                    start,
                                    end: i,
                                    notes: None,
                                });
                                group_stack.push(("non_capturing_group", start));
                            }
                            b'>' => {
                                i += 3;
                                constructs.push(RegexConstruct {
                                    raw: "(?>".to_string(),
                                    kind: ConstructKind::Group,
                                    description: "Atomic group: Disallows backtracking once matched".to_string(),
                                    start,
                                    end: i,
                                    notes: Some("Atomic groups are not supported in standard Rust regex".to_string()),
                                });
                                group_stack.push(("atomic_group", start));
                            }
                            _ => {
                                i += 2;
                                let flag_start = i;
                                while i < len && bytes[i] != b')' && bytes[i] != b':' {
                                    i += 1;
                                }
                                let flag_chars = pattern.get(flag_start..i).unwrap_or("");
                                if i < len && bytes[i] == b':' {
                                    i += 1;
                                    let raw = pattern.get(start..i).unwrap_or("").to_string();
                                    constructs.push(RegexConstruct {
                                        raw,
                                        kind: ConstructKind::Group,
                                        description: format!("Non-capturing group with inline flags '{}'", flag_chars),
                                        start,
                                        end: i,
                                        notes: None,
                                    });
                                    group_stack.push(("flag_group", start));
                                } else if i < len && bytes[i] == b')' {
                                    i += 1;
                                    let raw = pattern.get(start..i).unwrap_or("").to_string();
                                    constructs.push(RegexConstruct {
                                        raw,
                                        kind: ConstructKind::Group,
                                        description: format!("Inline flag modifier: Sets flags '{}' for remaining pattern", flag_chars),
                                        start,
                                        end: i,
                                        notes: None,
                                    });
                                } else {
                                    let raw = pattern.get(start..i).unwrap_or("").to_string();
                                    constructs.push(RegexConstruct {
                                        raw,
                                        kind: ConstructKind::Group,
                                        description: format!("Inline flags '(?{}'", flag_chars),
                                        start,
                                        end: i,
                                        notes: None,
                                    });
                                }
                            }
                        }
                    } else {
                        i += 2;
                        constructs.push(RegexConstruct {
                            raw: "(?".to_string(),
                            kind: ConstructKind::Group,
                            description: "Incomplete special group construct".to_string(),
                            start,
                            end: i,
                            notes: None,
                        });
                    }
                } else {
                    i += 1;
                    group_counter += 1;
                    constructs.push(RegexConstruct {
                        raw: "(".to_string(),
                        kind: ConstructKind::Group,
                        description: format!("Numbered capture group #{}: Captures matched subexpression", group_counter),
                        start,
                        end: i,
                        notes: None,
                    });
                    group_stack.push(("numbered_group", start));
                }
            }
            b')' => {
                i += 1;
                let group_info = group_stack.pop();
                let desc = match group_info {
                    Some((gtype, _)) => format!("End of group ({})", gtype.replace('_', " ")),
                    None => "Unmatched closing parenthesis ')'".to_string(),
                };
                constructs.push(RegexConstruct {
                    raw: ")".to_string(),
                    kind: ConstructKind::Group,
                    description: desc,
                    start,
                    end: i,
                    notes: if group_info.is_none() {
                        Some("No matching opening '(' found".to_string())
                    } else {
                        None
                    },
                });
            }
            b'*' | b'+' | b'?' => {
                i += 1;
                let is_lazy = if i < len && bytes[i] == b'?' {
                    i += 1;
                    true
                } else {
                    flags.swap_greed
                };

                let desc = match b {
                    b'*' => if is_lazy {
                        "Quantifier: Matches preceding element 0 or more times (lazy / non-greedy)"
                    } else {
                        "Quantifier: Matches preceding element 0 or more times (greedy)"
                    },
                    b'+' => if is_lazy {
                        "Quantifier: Matches preceding element 1 or more times (lazy / non-greedy)"
                    } else {
                        "Quantifier: Matches preceding element 1 or more times (greedy)"
                    },
                    b'?' => if is_lazy {
                        "Quantifier: Matches preceding element 0 or 1 time (optional, lazy)"
                    } else {
                        "Quantifier: Matches preceding element 0 or 1 time (optional, greedy)"
                    },
                    _ => "Quantifier",
                };

                let raw = pattern.get(start..i).unwrap_or("").to_string();
                constructs.push(RegexConstruct {
                    raw,
                    kind: ConstructKind::Quantifier,
                    description: desc.to_string(),
                    start,
                    end: i,
                    notes: None,
                });
            }
            b'{' => {
                i += 1;
                let q_start = i;
                while i < len && bytes[i] != b'}' && (bytes[i].is_ascii_digit() || bytes[i] == b',' || bytes[i].is_ascii_whitespace()) {
                    i += 1;
                }
                if i < len && bytes[i] == b'}' {
                    i += 1;
                    let is_lazy = if i < len && bytes[i] == b'?' {
                        i += 1;
                        true
                    } else {
                        flags.swap_greed
                    };

                    let inner_end = if is_lazy && pattern.get(start..i).unwrap_or("").ends_with('?') {
                        i.saturating_sub(2)
                    } else {
                        i.saturating_sub(1)
                    };
                    let inner_q = pattern.get(q_start..inner_end).unwrap_or("").trim();
                    let desc = explain_brace_quantifier(inner_q, is_lazy);

                    let raw = pattern.get(start..i).unwrap_or("").to_string();
                    constructs.push(RegexConstruct {
                        raw,
                        kind: ConstructKind::Quantifier,
                        description: desc,
                        start,
                        end: i,
                        notes: None,
                    });
                } else {
                    constructs.push(RegexConstruct {
                        raw: "{".to_string(),
                        kind: ConstructKind::Literal,
                        description: "Literal character '{'".to_string(),
                        start,
                        end: start + 1,
                        notes: None,
                    });
                    i = start + 1;
                }
            }
            _ => {
                let lit_start = i;
                while i < len {
                    let next_b = bytes[i];
                    if matches!(next_b, b'^' | b'$' | b'.' | b'|' | b'\\' | b'[' | b']' | b'(' | b')' | b'*' | b'+' | b'?' | b'{') {
                        break;
                    }
                    if flags.ignore_whitespace && (next_b.is_ascii_whitespace() || next_b == b'#') {
                        break;
                    }
                    if let Some(ch) = pattern.get(i..).and_then(|s| s.chars().next()) {
                        i += ch.len_utf8();
                    } else {
                        i += 1;
                    }
                }

                if i > lit_start {
                    let raw = pattern.get(lit_start..i).unwrap_or("").to_string();
                    let desc = if raw.chars().count() == 1 {
                        format!("Literal character '{}'", raw)
                    } else {
                        format!("Literal text sequence '{}'", raw)
                    };
                    constructs.push(RegexConstruct {
                        raw,
                        kind: ConstructKind::Literal,
                        description: desc,
                        start: lit_start,
                        end: i,
                        notes: None,
                    });
                } else {
                    i += 1;
                }
            }
        }
    }

    let summary = generate_pattern_summary(pattern, &constructs, flags);

    RegexExplanation {
        pattern: pattern.to_string(),
        summary,
        constructs,
        flags: flags.clone(),
    }
}

/// Helper explaining the inner content of a character class `[...]`.
fn explain_character_class_inner(inner: &str, is_negated: bool) -> String {
    let mut parts = Vec::new();

    if inner.contains("a-z") {
        parts.push("lowercase letters a-z");
    }
    if inner.contains("A-Z") {
        parts.push("uppercase letters A-Z");
    }
    if inner.contains("0-9") || inner.contains(r"\d") {
        parts.push("digits 0-9");
    }
    if inner.contains(r"\w") {
        parts.push("word characters");
    }
    if inner.contains(r"\s") {
        parts.push("whitespace");
    }

    let prefix = if is_negated {
        "Negated character class: Matches any character EXCEPT "
    } else {
        "Character class: Matches any character in "
    };

    if !parts.is_empty() {
        format!("{}[{}] ({})", prefix, inner, parts.join(", "))
    } else {
        format!("{}[{}]", prefix, inner)
    }
}

/// Helper explaining brace quantifiers `{n}`, `{n,}`, `{n,m}`.
fn explain_brace_quantifier(inner: &str, is_lazy: bool) -> String {
    let lazy_suffix = if is_lazy { " (lazy / non-greedy)" } else { " (greedy)" };
    if let Some((min_str, max_str)) = inner.split_once(',') {
        let min_val = min_str.trim();
        let max_val = max_str.trim();
        if max_val.is_empty() {
            format!("Quantifier: Matches preceding element at least {} times{}", min_val, lazy_suffix)
        } else {
            format!("Quantifier: Matches preceding element between {} and {} times{}", min_val, max_val, lazy_suffix)
        }
    } else {
        format!("Quantifier: Matches preceding element exactly {} times", inner.trim())
    }
}

/// Generates a high-level summary of the parsed regex pattern.
fn generate_pattern_summary(pattern: &str, constructs: &[RegexConstruct], flags: &RegexFlags) -> String {
    let is_anchored_start = constructs.first().map_or(false, |c| c.kind == ConstructKind::Anchor && c.raw == "^");
    let is_anchored_end = constructs.last().map_or(false, |c| c.kind == ConstructKind::Anchor && c.raw == "$");

    let group_count = constructs.iter().filter(|c| c.kind == ConstructKind::Group && c.raw.starts_with('(')).count();
    let quant_count = constructs.iter().filter(|c| c.kind == ConstructKind::Quantifier).count();
    let class_count = constructs.iter().filter(|c| c.kind == ConstructKind::CharacterClass).count();

    let mut summary_parts = Vec::new();

    if is_anchored_start && is_anchored_end {
        summary_parts.push("Full-string anchored pattern".to_string());
    } else if is_anchored_start {
        summary_parts.push("Prefix-anchored pattern".to_string());
    } else if is_anchored_end {
        summary_parts.push("Suffix-anchored pattern".to_string());
    } else {
        summary_parts.push("Unanchored search pattern".to_string());
    }

    if group_count > 0 {
        summary_parts.push(format!("{} group{}", group_count, if group_count == 1 { "" } else { "s" }));
    }
    if quant_count > 0 {
        summary_parts.push(format!("{} quantifier{}", quant_count, if quant_count == 1 { "" } else { "s" }));
    }
    if class_count > 0 {
        summary_parts.push(format!("{} character class{}", class_count, if class_count == 1 { "" } else { "es" }));
    }

    if !flags.to_flag_string().is_empty() {
        summary_parts.push(format!("flags: /{}/{}", pattern, flags.to_flag_string()));
    }

    summary_parts.join(", ")
}

// ===========================================================================
// Regex Evaluator & Linter
// ===========================================================================

/// Options passed into the regex evaluator.
#[derive(Debug, Clone)]
pub struct RegexTestOptions {
    pub pattern: String,
    pub flags: RegexFlags,
    pub inputs: Vec<String>,
    pub replacement: Option<String>,
    pub replace_all: bool,
    pub split: bool,
    pub split_limit: Option<usize>,
    pub max_matches: usize,
    pub explain: bool,
    pub format: String,
}

impl Default for RegexTestOptions {
    fn default() -> Self {
        Self {
            pattern: String::new(),
            flags: RegexFlags::default(),
            inputs: Vec::new(),
            replacement: None,
            replace_all: true,
            split: false,
            split_limit: None,
            max_matches: 100,
            explain: true,
            format: "detailed".to_string(),
        }
    }
}

/// Helper to analyze patterns for common pitfalls and construct helpful warnings.
pub fn lint_regex_pattern(pattern: &str) -> Vec<String> {
    let mut warnings = Vec::new();

    // Check for unescaped dot before common file extensions or domains
    let common_extensions = [
        ".rs", ".ts", ".js", ".json", ".toml", ".yaml", ".yml", ".md", ".txt", ".html", ".css",
        ".com", ".org", ".net", ".io", ".dev",
    ];
    for ext in common_extensions {
        if pattern.contains(ext) && !pattern.contains(&format!("\\{}", ext)) {
            warnings.push(format!(
                "Pattern contains unescaped '{ext}'. The unescaped dot '.' matches any character. Use '\\{ext}' if matching a literal dot.",
            ));
        }
    }

    // Check for lookaround syntax which is unsupported in standard linear-time Rust regex engine
    if pattern.contains("(?=") || pattern.contains("(?!") {
        warnings.push(
            "Pattern contains lookahead syntax '(?=' or '(?!'. Standard Rust regex uses linear-time DFA and does not support lookarounds. Consider matching adjacent tokens or using capture groups.".to_string()
        );
    }
    if pattern.contains("(?<=") || pattern.contains("(?<!") {
        warnings.push(
            "Pattern contains lookbehind syntax '(?<=' or '(?<!'. Standard Rust regex does not support lookbehinds. Consider matching preceding context and extracting with a capture group.".to_string()
        );
    }

    // Check for backreference syntax like \1, \2
    if regex_contains_backreference(pattern) {
        warnings.push(
            "Pattern contains backreference syntax '\\1', '\\2'. Backreferences are not supported in standard linear-time Rust regex. Consider matching components separately.".to_string()
        );
    }

    // Check for nested quantifiers / ReDoS patterns
    if detect_nested_quantifiers(pattern) {
        warnings.push(
            "Pattern contains nested quantifiers (e.g. '(a+)+'). While Rust regex guarantees linear time, this pattern can cause catastrophic backtracking in PCRE/JS/Python engines.".to_string()
        );
    }

    warnings
}

/// Simple heuristic to detect potential backreferences (e.g. `\1` where not preceded by another slash).
fn regex_contains_backreference(pattern: &str) -> bool {
    let bytes = pattern.as_bytes();
    for i in 0..bytes.len().saturating_sub(1) {
        if bytes[i] == b'\\' && bytes[i + 1].is_ascii_digit() && bytes[i + 1] != b'0' {
            let mut slash_count = 0;
            let mut j = i;
            while j > 0 && bytes[j - 1] == b'\\' {
                slash_count += 1;
                j -= 1;
            }
            if slash_count % 2 == 0 {
                return true;
            }
        }
    }
    false
}

/// Detects nested quantifiers like `(a+)+` or `(\d+)*`.
fn detect_nested_quantifiers(pattern: &str) -> bool {
    let bytes = pattern.as_bytes();
    for i in 0..bytes.len().saturating_sub(1) {
        if bytes[i] == b')' && (bytes[i + 1] == b'+' || bytes[i + 1] == b'*') {
            let mut depth = 1;
            let mut j = i;
            while j > 0 && depth > 0 {
                j -= 1;
                if bytes[j] == b')' {
                    depth += 1;
                } else if bytes[j] == b'(' {
                    depth -= 1;
                }
            }
            if depth == 0 {
                if let Some(inner) = pattern.get(j + 1..i) {
                    if inner.contains('+') || inner.contains('*') {
                        return true;
                    }
                }
            }
        }
    }
    false
}

/// Core regex evaluation engine.
pub struct RegexEvaluator;

impl RegexEvaluator {
    /// Compiles a regex with specified flags and returns the compiled regex or a formatted error.
    pub fn compile(pattern: &str, flags: &RegexFlags) -> Result<Regex, RegexErrorDetail> {
        RegexBuilder::new(pattern)
            .case_insensitive(!flags.case_sensitive)
            .multi_line(flags.multiline)
            .dot_matches_new_line(flags.dot_matches_all)
            .ignore_whitespace(flags.ignore_whitespace)
            .swap_greed(flags.swap_greed)
            .unicode(flags.unicode)
            .build()
            .map_err(|err| {
                let msg = err.to_string();
                let mut suggestion = None;

                if msg.contains("look-around") || pattern.contains("(?=") || pattern.contains("(?<=") {
                    suggestion = Some(
                        "Rust regex engine does not support lookarounds ((?=...), (?<=...)). Consider matching the surrounding text and using capture groups instead.".to_string()
                    );
                } else if msg.contains("unclosed") {
                    suggestion = Some("Check for unclosed parentheses '(', brackets '[', or braces '{'.".to_string());
                } else if msg.contains("repetition") {
                    suggestion = Some("Ensure quantifiers (+, *, ?, {n,m}) follow a valid expression or character class.".to_string());
                } else if msg.contains("escape") {
                    suggestion = Some("Ensure escape sequences are valid, or use raw string / double-backslash in code.".to_string());
                }

                RegexErrorDetail {
                    message: msg,
                    suggestion,
                }
            })
    }

    /// Evaluates a compiled regex against a single input string.
    pub fn evaluate_input(
        re: &Regex,
        input: &str,
        input_index: usize,
        options: &RegexTestOptions,
    ) -> TestStringResult {
        let source_map = SourceMap::new(input);
        let capture_names: Vec<Option<&str>> = re.capture_names().collect();

        let mut matches = Vec::new();

        for (match_idx, caps) in re.captures_iter(input).enumerate() {
            if match_idx >= options.max_matches {
                break;
            }

            let full_match = match caps.get(0) {
                Some(m) => m,
                None => continue,
            };

            let full_span = source_map.span(full_match.start(), full_match.end());
            let mut groups = Vec::new();
            let mut named_groups = HashMap::new();

            for (group_idx, group_match_opt) in caps.iter().enumerate() {
                if let Some(gm) = group_match_opt {
                    let group_span = source_map.span(gm.start(), gm.end());
                    let name = capture_names.get(group_idx).and_then(|&n| n).map(|s| s.to_string());

                    if let Some(group_name) = &name {
                        named_groups.insert(group_name.clone(), gm.as_str().to_string());
                    }

                    groups.push(CaptureGroup {
                        index: group_idx,
                        name,
                        text: gm.as_str().to_string(),
                        span: group_span,
                    });
                }
            }

            matches.push(MatchItem {
                match_index: match_idx,
                text: full_match.as_str().to_string(),
                span: full_span,
                groups,
                named_groups,
            });
        }

        // Replacement testing if requested
        let replacement = options.replacement.as_ref().map(|rep_template| {
            if options.replace_all {
                re.replace_all(input, rep_template.as_str()).to_string()
            } else {
                re.replace(input, rep_template.as_str()).to_string()
            }
        });

        // Split testing if requested
        let splits = if options.split {
            let segs: Vec<String> = match options.split_limit {
                Some(limit) if limit > 0 => re.splitn(input, limit).map(|s| s.to_string()).collect(),
                _ => re.split(input).map(|s| s.to_string()).collect(),
            };
            Some(segs)
        } else {
            None
        };

        let match_count = matches.len();
        let matched = match_count > 0;

        TestStringResult {
            input_index,
            input: input.to_string(),
            matched,
            match_count,
            matches,
            replacement,
            splits,
        }
    }

    /// Runs complete evaluation across all input strings and gathers report.
    pub fn evaluate(options: &RegexTestOptions) -> RegexEvaluationReport {
        let pattern = &options.pattern;
        let flags = &options.flags;

        let explanation = if options.explain {
            Some(explain_pattern(pattern, flags))
        } else {
            None
        };

        let compiled = match Self::compile(pattern, flags) {
            Ok(re) => re,
            Err(err) => {
                return RegexEvaluationReport {
                    valid: false,
                    pattern: pattern.clone(),
                    error: Some(err),
                    metadata: None,
                    explanation,
                    results: Vec::new(),
                    total_inputs: options.inputs.len(),
                    matched_inputs: 0,
                    total_matches: 0,
                };
            }
        };

        // Extract metadata
        let capture_names: Vec<Option<String>> = compiled
            .capture_names()
            .map(|opt| opt.map(|s| s.to_string()))
            .collect();

        let named_group_names: Vec<String> = capture_names
            .iter()
            .filter_map(|opt| opt.clone())
            .collect();

        let capture_group_count = capture_names.len().saturating_sub(1);
        let flags_str = flags.to_flag_string();
        let pattern_display = if flags_str.is_empty() {
            format!("/{}/", pattern)
        } else {
            format!("/{}/{}", pattern, flags_str)
        };

        let warnings = lint_regex_pattern(pattern);

        let metadata = RegexMetadata {
            pattern: pattern.clone(),
            pattern_display,
            flags: flags.clone(),
            flags_str,
            capture_group_count,
            named_group_names,
            all_group_names: capture_names,
            warnings,
        };

        let mut results = Vec::new();
        let mut matched_inputs = 0;
        let mut total_matches = 0;

        for (idx, input) in options.inputs.iter().enumerate() {
            let res = Self::evaluate_input(&compiled, input, idx, options);
            if res.matched {
                matched_inputs += 1;
            }
            total_matches += res.match_count;
            results.push(res);
        }

        RegexEvaluationReport {
            valid: true,
            pattern: pattern.clone(),
            error: None,
            metadata: Some(metadata),
            explanation,
            results,
            total_inputs: options.inputs.len(),
            matched_inputs,
            total_matches,
        }
    }
}

// ===========================================================================
// Report Formatting (Text, Detailed, JSON, Compact, Explain)
// ===========================================================================

pub fn render_report(report: &RegexEvaluationReport, format: &str) -> String {
    match format.to_lowercase().as_str() {
        "json" => serde_json::to_string_pretty(report).unwrap_or_else(|_| "{}".to_string()),
        "compact" => render_compact_report(report),
        "explain" | "explanation" => render_explanation_view(report),
        _ => render_detailed_report(report),
    }
}

fn render_compact_report(report: &RegexEvaluationReport) -> String {
    let mut out = String::new();
    if !report.valid {
        out.push_str(&format!("❌ Invalid Regex: {}\n", report.pattern));
        if let Some(err) = &report.error {
            out.push_str(&format!("Error: {}\n", err.message));
        }
        return out;
    }

    if let Some(meta) = &report.metadata {
        out.push_str(&format!("✓ Regex: {}\n", meta.pattern_display));
    } else {
        out.push_str(&format!("✓ Regex: /{}/\n", report.pattern));
    }

    out.push_str(&format!(
        "Matches: {} across {}/{} inputs\n",
        report.total_matches, report.matched_inputs, report.total_inputs
    ));

    for res in &report.results {
        let prefix = if report.results.len() > 1 {
            format!("[Input #{}] ", res.input_index + 1)
        } else {
            String::new()
        };

        if !res.matched {
            out.push_str(&format!("{}No match\n", prefix));
        } else {
            for m in &res.matches {
                out.push_str(&format!(
                    "{}Match #{}: {:?} @ {}\n",
                    prefix,
                    m.match_index + 1,
                    m.text,
                    m.span.format_location()
                ));
            }
        }
        if let Some(rep) = &res.replacement {
            out.push_str(&format!("{}Replaced: {:?}\n", prefix, rep));
        }
    }
    out
}

fn render_explanation_view(report: &RegexEvaluationReport) -> String {
    let mut out = String::new();
    out.push_str("╔═══════════════════════════════════════════════════════════════╗\n");
    out.push_str("║                   REGEX CONSTRUCT EXPLANATION                 ║\n");
    out.push_str("╚═══════════════════════════════════════════════════════════════╝\n\n");

    out.push_str(&format!("Pattern:         /{}/\n", report.pattern));
    if let Some(meta) = &report.metadata {
        if !meta.flags_str.is_empty() {
            out.push_str(&format!("Flags:           {}\n", meta.flags_str));
        }
    }

    if let Some(exp) = &report.explanation {
        out.push_str(&format!("Summary:         {}\n\n", exp.summary));
        out.push_str("Constructs Breakdown:\n");
        out.push_str("─────────────────────────────────────────────────────────────────\n");
        for (idx, c) in exp.constructs.iter().enumerate() {
            let kind_label = match c.kind {
                ConstructKind::Anchor => "Anchor",
                ConstructKind::CharacterClass => "CharClass",
                ConstructKind::Quantifier => "Quantifier",
                ConstructKind::Group => "Group",
                ConstructKind::Lookaround => "Lookaround",
                ConstructKind::Alternation => "Alternation",
                ConstructKind::Escape => "Escape",
                ConstructKind::Literal => "Literal",
                ConstructKind::Comment => "Comment",
            };

            out.push_str(&format!(
                "{:>3}. {:<14} {:<12} [bytes {:>2}..{:<2}] {}\n",
                idx + 1,
                format!("`{}`", c.raw),
                format!("[{}]", kind_label),
                c.start,
                c.end,
                c.description
            ));

            if let Some(note) = &c.notes {
                out.push_str(&format!("     ↳ ℹ️  {}\n", note));
            }
        }
        out.push_str("─────────────────────────────────────────────────────────────────\n");
    }

    if let Some(err) = &report.error {
        out.push_str(&format!("\n❌ Compilation Error:\n  {}\n", err.message));
        if let Some(sugg) = &err.suggestion {
            out.push_str(&format!("  💡 Suggestion: {}\n", sugg));
        }
    }

    out
}

fn render_detailed_report(report: &RegexEvaluationReport) -> String {
    let mut out = String::new();

    if !report.valid {
        out.push_str("╔═══════════════════════════════════════════════════════════════╗\n");
        out.push_str("║                   REGEX COMPILATION ERROR                     ║\n");
        out.push_str("╚═══════════════════════════════════════════════════════════════╝\n\n");
        out.push_str(&format!("Pattern: {}\n", report.pattern));
        if let Some(err) = &report.error {
            out.push_str(&format!("\nError Diagnostic:\n  {}\n", err.message));
            if let Some(sugg) = &err.suggestion {
                out.push_str(&format!("\nSuggestion:\n  💡 {}\n", sugg));
            }
        }
        if let Some(exp) = &report.explanation {
            out.push_str(&format!("\nConstruct Analysis:\n  {}\n", exp.summary));
        }
        return out;
    }

    out.push_str("╔═══════════════════════════════════════════════════════════════╗\n");
    out.push_str("║                    REGEX EVALUATION REPORT                    ║\n");
    out.push_str("╚═══════════════════════════════════════════════════════════════╝\n\n");

    if let Some(meta) = &report.metadata {
        out.push_str(&format!("Pattern:         {}\n", meta.pattern_display));
        out.push_str(&format!("Raw Pattern:     {}\n", meta.pattern));
        out.push_str(&format!(
            "Active Flags:    {} (case_sensitive: {}, multiline: {}, dot_matches_all: {}, ignore_whitespace: {}, unicode: {})\n",
            if meta.flags_str.is_empty() { "(none)".to_string() } else { meta.flags_str.clone() },
            meta.flags.case_sensitive,
            meta.flags.multiline,
            meta.flags.dot_matches_all,
            meta.flags.ignore_whitespace,
            meta.flags.unicode
        ));
        out.push_str(&format!("Capture Groups:  {}\n", meta.capture_group_count));
        if !meta.named_group_names.is_empty() {
            out.push_str(&format!(
                "Named Groups:    [{}]\n",
                meta.named_group_names.join(", ")
            ));
        }

        if !meta.warnings.is_empty() {
            out.push_str("\nLint Warnings:\n");
            for warn in &meta.warnings {
                out.push_str(&format!("  ⚠️  {}\n", warn));
            }
        }
    }

    if let Some(exp) = &report.explanation {
        out.push_str(&format!("\nPattern Summary: {}\n", exp.summary));
    }

    out.push_str("\n─────────────────────────────────────────────────────────────────\n");
    out.push_str(&format!(
        "Evaluation Summary: {} matches found across {}/{} test inputs\n",
        report.total_matches, report.matched_inputs, report.total_inputs
    ));
    out.push_str("─────────────────────────────────────────────────────────────────\n");

    if report.results.is_empty() {
        out.push_str("\n(No test inputs provided for evaluation. Pattern syntax validated successfully.)\n");
        return out;
    }

    for (idx, res) in report.results.iter().enumerate() {
        out.push_str(&format!("\n▶ Input #{}:\n", idx + 1));
        let display_input = if res.input.len() > 300 {
            format!("{}... (truncated, total {} chars)", &res.input[..300], res.input.len())
        } else {
            res.input.clone()
        };

        for line in display_input.lines() {
            out.push_str(&format!("  │ {}\n", line));
        }

        if !res.matched {
            out.push_str("  └─ ❌ No match found.\n");
        } else {
            out.push_str(&format!(
                "  └─ ✓ {} match{} found:\n\n",
                res.match_count,
                if res.match_count == 1 { "" } else { "es" }
            ));

            for m in &res.matches {
                out.push_str(&format!(
                    "     Match #{}: {:?}\n",
                    m.match_index + 1,
                    m.text
                ));
                out.push_str(&format!("     Location: {}\n", m.span.format_location()));

                if m.groups.len() > 1 {
                    out.push_str("     Capture Groups:\n");
                    for g in &m.groups {
                        if g.index == 0 {
                            continue; // Skip group 0 (full match) in groups display
                        }
                        let name_tag = match &g.name {
                            Some(name) => format!(" (?P<{}>)", name),
                            None => String::new(),
                        };
                        out.push_str(&format!(
                            "       - Group #{}{}: {:?} at {}\n",
                            g.index,
                            name_tag,
                            g.text,
                            g.span.format_location()
                        ));
                    }
                }
                out.push('\n');
            }
        }

        if let Some(rep) = &res.replacement {
            out.push_str(&format!("  Substitution Result: {:?}\n", rep));
        }

        if let Some(splits) = &res.splits {
            out.push_str(&format!("  Split Segments ({}) : {:?}\n", splits.len(), splits));
        }
    }

    out
}

// ===========================================================================
// Tool Implementation: RegexTestTool
// ===========================================================================

/// Tool that evaluates regular expressions against test strings or files.
#[derive(Default, Debug, Clone)]
pub struct RegexTestTool;

impl RegexTestTool {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl Tool for RegexTestTool {
    fn name(&self) -> &str {
        "regex_test"
    }

    fn description(&self) -> &str {
        "Evaluate, test, validate, and explain regular expressions against input strings or files. Inspect match groups, named captures, byte and line/column spans, perform substitution testing, string splitting, construct explanations, and regex syntax diagnostics."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "pattern": {
                    "type": "string",
                    "description": "The regular expression pattern to test (e.g. '^(\\w+): (?P<msg>.*)$')."
                },
                "input": {
                    "description": "Single test string or an array of test strings to evaluate against the regex pattern.",
                    "oneOf": [
                        { "type": "string" },
                        { "type": "array", "items": { "type": "string" } }
                    ]
                },
                "test_strings": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "Optional array of test strings to evaluate."
                },
                "file": {
                    "type": "string",
                    "description": "Optional path to a file whose contents should be used as the test input string."
                },
                "flags": {
                    "type": "string",
                    "description": "Optional regex flag string (e.g. 'i' for case-insensitive, 'm' for multiline, 's' for dot-matches-all, 'x' for ignore-whitespace, 'U' for swap-greed)."
                },
                "case_sensitive": {
                    "type": "boolean",
                    "description": "Whether matching should be case-sensitive (optional, default: true)."
                },
                "multiline": {
                    "type": "boolean",
                    "description": "Whether '^' and '$' match line beginnings and endings (optional, default: false)."
                },
                "dot_matches_all": {
                    "type": "boolean",
                    "description": "Whether '.' matches newline characters (singleline mode) (optional, default: false)."
                },
                "ignore_whitespace": {
                    "type": "boolean",
                    "description": "Whether whitespace and '#' comments inside pattern should be ignored (extended mode) (optional, default: false)."
                },
                "swap_greed": {
                    "type": "boolean",
                    "description": "Whether to swap greediness of quantifiers like '*' and '+' (optional, default: false)."
                },
                "unicode": {
                    "type": "boolean",
                    "description": "Whether unicode character properties are enabled (optional, default: true)."
                },
                "replacement": {
                    "type": "string",
                    "description": "Optional replacement template string (e.g. '$1-$2' or '${msg}') to test substitution."
                },
                "replace_all": {
                    "type": "boolean",
                    "description": "When replacement is specified, whether to replace all matches or only the first (optional, default: true)."
                },
                "split": {
                    "type": "boolean",
                    "description": "Whether to split input string(s) using the regex pattern as a delimiter (optional, default: false)."
                },
                "split_limit": {
                    "type": "integer",
                    "description": "Optional maximum number of split segments."
                },
                "max_matches": {
                    "type": "integer",
                    "description": "Maximum number of matches to collect per test string (optional, default: 100)."
                },
                "explain": {
                    "type": "boolean",
                    "description": "Whether to generate a human-readable breakdown and explanation of regex constructs (optional, default: true)."
                },
                "format": {
                    "type": "string",
                    "enum": ["detailed", "compact", "json", "explain", "text"],
                    "description": "Output format style (optional, default: 'detailed')."
                }
            },
            "required": ["pattern"]
        })
    }

    async fn execute(&self, args: Value, ctx: &ToolContext) -> anyhow::Result<String> {
        let pattern = match args.get("pattern").and_then(|v| v.as_str()) {
            Some(p) => p.to_string(),
            None => {
                anyhow::bail!("Missing required parameter 'pattern'");
            }
        };

        let mut flags = RegexFlags::default();

        if let Some(cs) = args.get("case_sensitive").and_then(|v| v.as_bool()) {
            flags.case_sensitive = cs;
        } else if let Some(ci) = args.get("case_insensitive").and_then(|v| v.as_bool()) {
            flags.case_sensitive = !ci;
        }

        if let Some(m) = args.get("multiline").and_then(|v| v.as_bool()) {
            flags.multiline = m;
        }

        if let Some(s) = args.get("dot_matches_all").and_then(|v| v.as_bool())
            .or_else(|| args.get("dot_matches_new_line").and_then(|v| v.as_bool()))
            .or_else(|| args.get("singleline").and_then(|v| v.as_bool()))
        {
            flags.dot_matches_all = s;
        }

        if let Some(x) = args.get("ignore_whitespace").and_then(|v| v.as_bool())
            .or_else(|| args.get("extended").and_then(|v| v.as_bool()))
        {
            flags.ignore_whitespace = x;
        }

        if let Some(u) = args.get("swap_greed").and_then(|v| v.as_bool()) {
            flags.swap_greed = u;
        }

        if let Some(uni) = args.get("unicode").and_then(|v| v.as_bool()) {
            flags.unicode = uni;
        }

        if let Some(flags_str) = args.get("flags").and_then(|v| v.as_str()) {
            flags.parse_flag_str(flags_str);
        }

        let mut inputs = Vec::new();

        // 1. Check "input" field (single string or array)
        if let Some(inp) = args.get("input").or_else(|| args.get("text")) {
            if let Some(single_str) = inp.as_str() {
                inputs.push(single_str.to_string());
            } else if let Some(arr) = inp.as_array() {
                for item in arr {
                    if let Some(s) = item.as_str() {
                        inputs.push(s.to_string());
                    }
                }
            }
        }

        // 2. Check "test_strings" or "inputs" array
        if let Some(arr) = args.get("test_strings").or_else(|| args.get("inputs")).and_then(|v| v.as_array()) {
            for item in arr {
                if let Some(s) = item.as_str() {
                    inputs.push(s.to_string());
                }
            }
        }

        // 3. Check "file" path if provided
        if let Some(file_val) = args.get("file").and_then(|v| v.as_str()) {
            let file_path = resolve_path(file_val, &ctx.cwd);
            if file_path.exists() {
                let file_content = tokio::fs::read_to_string(&file_path).await
                    .map_err(|e| anyhow::anyhow!("Failed to read test input file '{}': {}", file_val, e))?;
                inputs.push(file_content);
            } else {
                anyhow::bail!("Input file does not exist: {}", file_val);
            }
        }

        let replacement = args.get("replacement")
            .or_else(|| args.get("replace"))
            .or_else(|| args.get("substitute"))
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());

        let replace_all = args.get("replace_all")
            .or_else(|| args.get("all"))
            .and_then(|v| v.as_bool())
            .unwrap_or(true);

        let split = args.get("split").and_then(|v| v.as_bool()).unwrap_or(false);
        let split_limit = args.get("split_limit").and_then(|v| v.as_u64()).map(|u| u as usize);
        let max_matches = args.get("max_matches").and_then(|v| v.as_u64()).map(|u| u as usize).unwrap_or(100);
        let explain = args.get("explain").and_then(|v| v.as_bool()).unwrap_or(true);
        let format = args.get("format").and_then(|v| v.as_str()).unwrap_or("detailed").to_string();

        let options = RegexTestOptions {
            pattern,
            flags,
            inputs,
            replacement,
            replace_all,
            split,
            split_limit,
            max_matches,
            explain,
            format: format.clone(),
        };

        let report = RegexEvaluator::evaluate(&options);
        Ok(render_report(&report, &format))
    }
}

// ===========================================================================
// Unit Tests
// ===========================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_basic_match_and_spans() {
        let pattern = r"\b[A-Za-z0-9._%+-]+@[A-Za-z0-9.-]+\.[A-Za-z]{2,}\b";
        let text = "Contact us at support@example.com or admin@fusion.dev for help.";

        let options = RegexTestOptions {
            pattern: pattern.to_string(),
            inputs: vec![text.to_string()],
            ..Default::default()
        };

        let report = RegexEvaluator::evaluate(&options);
        assert!(report.valid);
        assert_eq!(report.total_matches, 2);
        assert_eq!(report.results.len(), 1);

        let res = &report.results[0];
        assert!(res.matched);
        assert_eq!(res.matches.len(), 2);

        let m1 = &res.matches[0];
        assert_eq!(m1.text, "support@example.com");
        assert_eq!(m1.span.start, 14);
        assert_eq!(m1.span.end, 33);
        assert_eq!(m1.span.start_line, 1);
        assert_eq!(m1.span.start_col, 15);

        let m2 = &res.matches[1];
        assert_eq!(m2.text, "admin@fusion.dev");
    }

    #[test]
    fn test_named_capture_groups() {
        let pattern = r"(?P<year>\d{4})-(?P<month>\d{2})-(?P<day>\d{2})";
        let text = "Release date: 2026-09-02, Previous: 2025-12-31";

        let options = RegexTestOptions {
            pattern: pattern.to_string(),
            inputs: vec![text.to_string()],
            ..Default::default()
        };

        let report = RegexEvaluator::evaluate(&options);
        assert!(report.valid);
        assert_eq!(report.total_matches, 2);

        let res = &report.results[0];
        let m1 = &res.matches[0];
        assert_eq!(m1.text, "2026-09-02");
        assert_eq!(m1.named_groups.get("year").map(|s| s.as_str()), Some("2026"));
        assert_eq!(m1.named_groups.get("month").map(|s| s.as_str()), Some("09"));
        assert_eq!(m1.named_groups.get("day").map(|s| s.as_str()), Some("02"));

        assert_eq!(m1.groups.len(), 4); // 0 (full) + 1 (year) + 2 (month) + 3 (day)
        assert_eq!(m1.groups[1].name.as_deref(), Some("year"));
        assert_eq!(m1.groups[1].text, "2026");
        assert_eq!(m1.groups[1].span.start, 14);
        assert_eq!(m1.groups[1].span.end, 18);
    }

    #[test]
    fn test_multiline_and_source_map() {
        let pattern = r"^fn\s+(?P<func_name>\w+)\s*\(";
        let text = "// Module\nfn calculate(x: i32) -> i32 {\n    x * 2\n}\n\nfn process() {\n}\n";

        let mut flags = RegexFlags::default();
        flags.multiline = true;

        let options = RegexTestOptions {
            pattern: pattern.to_string(),
            flags,
            inputs: vec![text.to_string()],
            ..Default::default()
        };

        let report = RegexEvaluator::evaluate(&options);
        assert!(report.valid);
        assert_eq!(report.total_matches, 2);

        let m1 = &report.results[0].matches[0];
        assert_eq!(m1.span.start_line, 2);
        assert_eq!(m1.span.start_col, 1);
        assert_eq!(m1.named_groups.get("func_name").map(|s| s.as_str()), Some("calculate"));

        let m2 = &report.results[0].matches[1];
        assert_eq!(m2.span.start_line, 6);
        assert_eq!(m2.span.start_col, 1);
        assert_eq!(m2.named_groups.get("func_name").map(|s| s.as_str()), Some("process"));
    }

    #[test]
    fn test_utf8_multibyte_spans() {
        let pattern = r"🦀\s+(?P<word>\w+)";
        let text = "Hello 🦀 Ferris and 🦀 Rustaceans!";

        let options = RegexTestOptions {
            pattern: pattern.to_string(),
            inputs: vec![text.to_string()],
            ..Default::default()
        };

        let report = RegexEvaluator::evaluate(&options);
        assert!(report.valid);
        assert_eq!(report.total_matches, 2);

        let m1 = &report.results[0].matches[0];
        assert_eq!(m1.text, "🦀 Ferris");
        assert_eq!(m1.span.char_start, 6);
        assert_eq!(m1.span.start_col, 7);
        assert_eq!(m1.named_groups.get("word").map(|s| s.as_str()), Some("Ferris"));
    }

    #[test]
    fn test_replacement_and_substitution() {
        let pattern = r"(\d{4})-(\d{2})-(\d{2})";
        let text = "Start: 2026-09-02, End: 2026-10-15";

        let options = RegexTestOptions {
            pattern: pattern.to_string(),
            inputs: vec![text.to_string()],
            replacement: Some("$2/$3/$1".to_string()),
            replace_all: true,
            ..Default::default()
        };

        let report = RegexEvaluator::evaluate(&options);
        assert!(report.valid);
        let res = &report.results[0];
        assert_eq!(res.replacement.as_deref(), Some("Start: 09/02/2026, End: 10/15/2026"));
    }

    #[test]
    fn test_split() {
        let pattern = r"\s*,\s*|\s*;\s*";
        let text = "apple, banana ; cherry, date; elderberry";

        let options = RegexTestOptions {
            pattern: pattern.to_string(),
            inputs: vec![text.to_string()],
            split: true,
            ..Default::default()
        };

        let report = RegexEvaluator::evaluate(&options);
        assert!(report.valid);
        let splits = report.results[0].splits.as_ref().unwrap();
        assert_eq!(splits, &["apple", "banana", "cherry", "date", "elderberry"]);
    }

    #[test]
    fn test_invalid_regex_diagnostics() {
        let pattern = r"(unclosed group";

        let options = RegexTestOptions {
            pattern: pattern.to_string(),
            inputs: vec!["test".to_string()],
            ..Default::default()
        };

        let report = RegexEvaluator::evaluate(&options);
        assert!(!report.valid);
        assert!(report.error.is_some());
        let err = report.error.as_ref().unwrap();
        assert!(err.message.contains("unclosed") || err.suggestion.is_some());
    }

    #[test]
    fn test_lint_warnings() {
        let warnings = lint_regex_pattern("user@domain.com");
        assert!(!warnings.is_empty());
        assert!(warnings[0].contains("unescaped '.com'"));

        let lookahead_warnings = lint_regex_pattern("foo(?=bar)");
        assert!(lookahead_warnings.iter().any(|w| w.contains("lookahead")));

        let redos_warnings = lint_regex_pattern("^(a+)+$");
        assert!(redos_warnings.iter().any(|w| w.contains("nested quantifiers")));
    }

    #[test]
    fn test_construct_explanation() {
        let pattern = r"^(?P<user>[a-zA-Z0-9_]+)@(?P<domain>[a-zA-Z0-9.-]+\.[a-zA-Z]{2,4})$";
        let flags = RegexFlags::default();
        let explanation = explain_pattern(pattern, &flags);

        assert!(!explanation.constructs.is_empty());
        assert!(explanation.summary.contains("anchored"));

        let kinds: Vec<ConstructKind> = explanation.constructs.iter().map(|c| c.kind).collect();
        assert!(kinds.contains(&ConstructKind::Anchor));
        assert!(kinds.contains(&ConstructKind::Group));
        assert!(kinds.contains(&ConstructKind::CharacterClass));
        assert!(kinds.contains(&ConstructKind::Quantifier));

        let user_group = explanation.constructs.iter().find(|c| c.raw.contains("user"));
        assert!(user_group.is_some());
        assert!(user_group.unwrap().description.contains("Named capture group 'user'"));
    }

    #[test]
    fn test_lookaround_explanation() {
        let pattern = r"(?=.*[A-Z])(?=.*\d)(?!.*admin)";
        let flags = RegexFlags::default();
        let explanation = explain_pattern(pattern, &flags);

        assert!(explanation.constructs.iter().any(|c| c.kind == ConstructKind::Lookaround));
        let pos_lookahead = explanation.constructs.iter().find(|c| c.raw == "(?=");
        assert!(pos_lookahead.is_some());
        assert!(pos_lookahead.unwrap().description.contains("Positive lookahead"));

        let neg_lookahead = explanation.constructs.iter().find(|c| c.raw == "(?!");
        assert!(neg_lookahead.is_some());
        assert!(neg_lookahead.unwrap().description.contains("Negative lookahead"));
    }

    #[test]
    fn test_brace_quantifiers_explanation() {
        let pattern = r"\d{4}-\d{2,4}-\d{1,}";
        let flags = RegexFlags::default();
        let explanation = explain_pattern(pattern, &flags);

        let exact = explanation.constructs.iter().find(|c| c.raw == "{4}");
        assert!(exact.is_some());
        assert!(exact.unwrap().description.contains("exactly 4 times"));

        let range = explanation.constructs.iter().find(|c| c.raw == "{2,4}");
        assert!(range.is_some());
        assert!(range.unwrap().description.contains("between 2 and 4 times"));

        let at_least = explanation.constructs.iter().find(|c| c.raw == "{1,}");
        assert!(at_least.is_some());
        assert!(at_least.unwrap().description.contains("at least 1 times"));
    }

    #[tokio::test]
    async fn test_tool_execute_json_and_text() {
        let tool = RegexTestTool::new();
        let ctx = ToolContext::default();

        let args = json!({
            "pattern": "(?P<key>\\w+)=(?P<val>\\w+)",
            "input": "foo=bar baz=qux",
            "format": "json"
        });

        let output = tool.execute(args, &ctx).await.unwrap();
        let parsed: Value = serde_json::from_str(&output).unwrap();
        assert_eq!(parsed["valid"], true);
        assert_eq!(parsed["total_matches"], 2);
        assert!(parsed["explanation"].is_object());

        let args_text = json!({
            "pattern": "\\d+",
            "input": "123 456",
            "format": "detailed"
        });
        let output_text = tool.execute(args_text, &ctx).await.unwrap();
        assert!(output_text.contains("REGEX EVALUATION REPORT"));
        assert!(output_text.contains("2 matches found"));

        let args_explain = json!({
            "pattern": "^[a-z0-9]+$",
            "format": "explain"
        });
        let output_explain = tool.execute(args_explain, &ctx).await.unwrap();
        assert!(output_explain.contains("REGEX CONSTRUCT EXPLANATION"));
        assert!(output_explain.contains("Anchor"));
        assert!(output_explain.contains("CharClass"));
    }
}
