use serde::{de::DeserializeOwned, Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;

use crate::provider::types::{StreamChunk, ToolCall};

/// Strategy for handling unclosed object keys when input ends unexpectedly.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum UnclosedKeyStrategy {
    /// Append `: null` to dangling keys (e.g. `{"key"` -> `{"key": null}`)
    #[default]
    Null,
    /// Append `: ""` to dangling keys (e.g. `{"key"` -> `{"key": ""}`)
    EmptyString,
    /// Append `: {}` to dangling keys (e.g. `{"key"` -> `{"key": {}}`)
    EmptyObject,
    /// Omit incomplete keys if possible (e.g. `{"a": 1, "key"` -> `{"a": 1}`)
    Omit,
}

/// Configuration options for partial JSON parsing and repair.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PartialJsonOptions {
    /// Strip markdown code block wrappers (e.g. ```json ... ```)
    pub strip_markdown_codeblocks: bool,
    /// Strategy when encountering a dangling object key without a value
    pub unclosed_key_strategy: UnclosedKeyStrategy,
    /// Automatically complete truncated literals (e.g. `tru` -> `true`, `nul` -> `null`)
    pub autocomplete_literals: bool,
    /// Repair lenient numbers (e.g. `12.` -> `12.0`, `-` -> `0`, `1e` -> `1e0`, `+42` -> `42`)
    pub lenient_numbers: bool,
    /// Allow Python-style literals (`True`, `False`, `None`, `NaN`, `Infinity`)
    pub allow_python_literals: bool,
    /// Allow single-quoted strings (`'key': 'value'`)
    pub allow_single_quotes: bool,
    /// Allow unquoted keys (`{key: "value"}`)
    pub allow_unquoted_keys: bool,
    /// Allow C-style `//` and `/* */` and Python-style `#` comments
    pub allow_comments: bool,
    /// Auto-escape unescaped control characters in strings (newlines, tabs, etc.)
    pub auto_escape_control_chars: bool,
    /// Maximum nesting depth allowed
    pub max_depth: usize,
}

impl Default for PartialJsonOptions {
    fn default() -> Self {
        Self {
            strip_markdown_codeblocks: true,
            unclosed_key_strategy: UnclosedKeyStrategy::Null,
            autocomplete_literals: true,
            lenient_numbers: true,
            allow_python_literals: true,
            allow_single_quotes: true,
            allow_unquoted_keys: true,
            allow_comments: true,
            auto_escape_control_chars: true,
            max_depth: 128,
        }
    }
}

/// Errors that can occur during partial JSON parsing or deserialization.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum PartialJsonError {
    #[error("Empty or whitespace-only JSON input")]
    EmptyInput,
    #[error("Maximum JSON nesting depth ({0}) exceeded")]
    MaxDepthExceeded(usize),
    #[error("Syntax error repairing partial JSON: {0}")]
    SyntaxError(String),
    #[error("Failed to parse repaired JSON as serde_json::Value: {0}")]
    ParseError(String),
    #[error("Failed to deserialize partial JSON into target type: {0}")]
    DeserializationError(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ObjectExpect {
    KeyOrClose,
    Colon,
    Value,
    CommaOrClose,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ArrayExpect {
    ValueOrClose,
    CommaOrClose,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Frame {
    Object(ObjectExpect),
    Array(ArrayExpect),
}

/// Strips markdown code block wrappers (e.g. ````json ... ````) from input.
pub fn strip_markdown(input: &str) -> &str {
    let trimmed = input.trim();
    if let Some(rest) = trimmed.strip_prefix("```") {
        let rest = if let Some(after_json) = rest.strip_prefix("json") {
            after_json
        } else if let Some(after_json) = rest.strip_prefix("JSON") {
            after_json
        } else if let Some(after_json) = rest.strip_prefix("Json") {
            after_json
        } else {
            rest
        };
        let after_line = if let Some(pos) = rest.find('\n') {
            &rest[pos + 1..]
        } else {
            rest.trim_start()
        };
        let mut stripped_end = if let Some(end_pos) = after_line.rfind("```") {
            &after_line[..end_pos]
        } else {
            after_line
        };
        stripped_end = stripped_end.trim();
        // Also strip incomplete trailing fence backticks
        if let Some(rest) = stripped_end.strip_suffix("``") {
            stripped_end = rest.trim();
        } else if let Some(rest) = stripped_end.strip_suffix('`') {
            stripped_end = rest.trim();
        }
        stripped_end
    } else {
        trimmed
    }
}

fn trim_trailing_comma(out: &mut String) {
    let mut len = out.len();
    while len > 0 {
        if let Some(last_char) = out[..len].chars().next_back() {
            if last_char.is_whitespace() {
                len -= last_char.len_utf8();
            } else if last_char == ',' {
                len -= 1;
                out.truncate(len);
                break;
            } else {
                break;
            }
        } else {
            break;
        }
    }
}

fn trim_dangling_key(out: &mut String) {
    let trimmed = out.trim_end();
    if let Some(last_quote) = trimmed.rfind('"') {
        let before_last_quote = &trimmed[..last_quote];
        if let Some(open_quote) = before_last_quote.rfind('"') {
            out.truncate(open_quote);
            trim_trailing_comma(out);
        }
    }
}

fn trim_dangling_key_and_colon(out: &mut String) {
    let mut trimmed = out.trim_end();
    if let Some(rest) = trimmed.strip_suffix(':') {
        trimmed = rest.trim_end();
    }
    if let Some(last_quote) = trimmed.rfind('"') {
        let before_last_quote = &trimmed[..last_quote];
        if let Some(open_quote) = before_last_quote.rfind('"') {
            out.truncate(open_quote);
            trim_trailing_comma(out);
        }
    }
}

fn on_value_completed(stack: &mut [Frame]) {
    if let Some(frame) = stack.last_mut() {
        match frame {
            Frame::Object(expect) => {
                if *expect == ObjectExpect::Value {
                    *expect = ObjectExpect::CommaOrClose;
                }
            }
            Frame::Array(expect) => {
                if *expect == ArrayExpect::ValueOrClose {
                    *expect = ArrayExpect::CommaOrClose;
                }
            }
        }
    }
}

fn repair_number_literal(s: &str) -> String {
    let mut s = s.trim();
    if let Some(rest) = s.strip_prefix('+') {
        s = rest;
    }
    if s.is_empty() || s == "-" {
        return "0".to_string();
    }
    if s == "." || s == "-." {
        return "0".to_string();
    }

    let mut normalized = String::with_capacity(s.len() + 4);
    if s.starts_with('.') {
        normalized.push('0');
        normalized.push_str(s);
    } else if s.starts_with("-.") {
        normalized.push_str("-0.");
        normalized.push_str(&s[2..]);
    } else {
        normalized.push_str(s);
    }

    if normalized.ends_with('.') {
        normalized.push('0');
    } else if normalized.ends_with('e') || normalized.ends_with('E') {
        normalized.push('0');
    } else if normalized.ends_with("e+")
        || normalized.ends_with("E+")
        || normalized.ends_with("e-")
        || normalized.ends_with("E-")
    {
        normalized.push('0');
    }

    normalized
}

/// Repairs an incomplete JSON string into a syntactically valid JSON string.
pub fn repair_partial_json(
    raw: &str,
    options: &PartialJsonOptions,
) -> Result<String, PartialJsonError> {
    let mut input = raw.trim();
    if input.is_empty() {
        return Ok("null".to_string());
    }

    if options.strip_markdown_codeblocks {
        input = strip_markdown(input);
    }
    input = input.trim();
    if input.is_empty() {
        return Ok("null".to_string());
    }

    let mut out = String::with_capacity(input.len() + 32);
    let mut stack: Vec<Frame> = Vec::new();
    let bytes = input.as_bytes();
    let len = bytes.len();
    let mut pos = 0;

    while pos < len {
        // Skip whitespace
        while pos < len
            && (bytes[pos] == b' '
                || bytes[pos] == b'\t'
                || bytes[pos] == b'\n'
                || bytes[pos] == b'\r')
        {
            pos += 1;
        }
        if pos >= len {
            break;
        }

        // Skip comments if enabled
        if options.allow_comments && bytes[pos] == b'/' && pos + 1 < len {
            if bytes[pos + 1] == b'/' {
                pos += 2;
                while pos < len && bytes[pos] != b'\n' {
                    pos += 1;
                }
                continue;
            } else if bytes[pos + 1] == b'*' {
                pos += 2;
                while pos + 1 < len && !(bytes[pos] == b'*' && bytes[pos + 1] == b'/') {
                    pos += 1;
                }
                if pos + 1 < len {
                    pos += 2;
                } else {
                    pos = len;
                }
                continue;
            }
        }

        // Python / bash style comments (#)
        if options.allow_comments && bytes[pos] == b'#' {
            pos += 1;
            while pos < len && bytes[pos] != b'\n' {
                pos += 1;
            }
            continue;
        }

        let b = bytes[pos];

        // Object open
        if b == b'{' {
            if stack.len() >= options.max_depth {
                return Err(PartialJsonError::MaxDepthExceeded(options.max_depth));
            }
            out.push('{');
            pos += 1;
            stack.push(Frame::Object(ObjectExpect::KeyOrClose));
            continue;
        }

        // Object close
        if b == b'}' {
            if let Some(Frame::Object(_)) = stack.last() {
                trim_trailing_comma(&mut out);
                out.push('}');
                pos += 1;
                stack.pop();
                on_value_completed(&mut stack);
                continue;
            } else {
                pos += 1;
                continue;
            }
        }

        // Array open
        if b == b'[' {
            if stack.len() >= options.max_depth {
                return Err(PartialJsonError::MaxDepthExceeded(options.max_depth));
            }
            out.push('[');
            pos += 1;
            stack.push(Frame::Array(ArrayExpect::ValueOrClose));
            continue;
        }

        // Array close
        if b == b']' {
            if let Some(Frame::Array(_)) = stack.last() {
                trim_trailing_comma(&mut out);
                out.push(']');
                pos += 1;
                stack.pop();
                on_value_completed(&mut stack);
                continue;
            } else {
                pos += 1;
                continue;
            }
        }

        // Colon
        if b == b':' {
            if let Some(Frame::Object(expect)) = stack.last_mut() {
                if *expect == ObjectExpect::Colon {
                    out.push(':');
                    pos += 1;
                    *expect = ObjectExpect::Value;
                    continue;
                }
            }
            pos += 1;
            continue;
        }

        // Comma
        if b == b',' {
            if let Some(frame) = stack.last_mut() {
                match frame {
                    Frame::Object(expect) => {
                        if *expect == ObjectExpect::CommaOrClose {
                            out.push(',');
                            pos += 1;
                            *expect = ObjectExpect::KeyOrClose;
                            continue;
                        }
                    }
                    Frame::Array(expect) => {
                        if *expect == ArrayExpect::CommaOrClose {
                            out.push(',');
                            pos += 1;
                            *expect = ArrayExpect::ValueOrClose;
                            continue;
                        }
                    }
                }
            }
            pos += 1;
            continue;
        }

        // Strings
        if b == b'"' || (options.allow_single_quotes && b == b'\'') {
            let quote = b;
            pos += 1;
            out.push('"');

            let mut escaped = false;
            let mut closed = false;

            while pos < len {
                let cb = bytes[pos];
                if escaped {
                    escaped = false;
                    if cb == b'"' {
                        out.push_str("\\\"");
                        pos += 1;
                    } else if cb == b'\\' {
                        out.push_str("\\\\");
                        pos += 1;
                    } else if cb == b'/' {
                        out.push('/');
                        pos += 1;
                    } else if cb == b'n' {
                        out.push_str("\\n");
                        pos += 1;
                    } else if cb == b'r' {
                        out.push_str("\\r");
                        pos += 1;
                    } else if cb == b't' {
                        out.push_str("\\t");
                        pos += 1;
                    } else if cb == b'b' {
                        out.push_str("\\b");
                        pos += 1;
                    } else if cb == b'f' {
                        out.push_str("\\f");
                        pos += 1;
                    } else if cb == b'\'' {
                        out.push('\'');
                        pos += 1;
                    } else if cb == b'u' {
                        pos += 1;
                        let mut hex_buf = String::with_capacity(4);
                        while pos < len && hex_buf.len() < 4 {
                            let h = bytes[pos];
                            if h.is_ascii_hexdigit() {
                                hex_buf.push(h as char);
                                pos += 1;
                            } else {
                                break;
                            }
                        }
                        while hex_buf.len() < 4 {
                            hex_buf.push('0');
                        }

                        if let Ok(code_point) = u16::from_str_radix(&hex_buf, 16) {
                            if (0xD800..=0xDBFF).contains(&code_point) {
                                // High surrogate - check for following low surrogate \uDC00..\uDFFF
                                if pos + 5 < len && bytes[pos] == b'\\' && bytes[pos + 1] == b'u' {
                                    let next_hex = &input[pos + 2..pos + 6];
                                    if let Ok(low_cp) = u16::from_str_radix(next_hex, 16) {
                                        if (0xDC00..=0xDFFF).contains(&low_cp) {
                                            out.push_str("\\u");
                                            out.push_str(&hex_buf);
                                            out.push_str("\\u");
                                            out.push_str(next_hex);
                                            pos += 6;
                                            continue;
                                        }
                                    }
                                }
                                out.push_str("\\uFFFD");
                            } else if (0xDC00..=0xDFFF).contains(&code_point) {
                                out.push_str("\\uFFFD");
                            } else {
                                out.push_str("\\u");
                                out.push_str(&hex_buf);
                            }
                        } else {
                            out.push_str("\\u0000");
                        }
                    } else {
                        // Lenient escape: escape the backslash so invalid escape becomes literal
                        out.push_str("\\\\");
                        let s = &input[pos..];
                        if let Some(ch) = s.chars().next() {
                            out.push(ch);
                            pos += ch.len_utf8();
                        } else {
                            pos += 1;
                        }
                    }
                } else if cb == b'\\' {
                    escaped = true;
                    pos += 1;
                } else if cb == quote {
                    out.push('"');
                    pos += 1;
                    closed = true;
                    break;
                } else if quote == b'\'' && cb == b'"' {
                    // Double quote inside single-quoted string
                    out.push_str("\\\"");
                    pos += 1;
                } else if cb == b'\n' {
                    if options.auto_escape_control_chars {
                        out.push_str("\\n");
                    } else {
                        out.push('\n');
                    }
                    pos += 1;
                } else if cb == b'\r' {
                    if options.auto_escape_control_chars {
                        out.push_str("\\r");
                    } else {
                        out.push('\r');
                    }
                    pos += 1;
                } else if cb == b'\t' {
                    if options.auto_escape_control_chars {
                        out.push_str("\\t");
                    } else {
                        out.push('\t');
                    }
                    pos += 1;
                } else if cb < 0x20 {
                    if options.auto_escape_control_chars {
                        out.push_str(&format!("\\u{:04x}", cb));
                    } else {
                        out.push(cb as char);
                    }
                    pos += 1;
                } else {
                    let s = &input[pos..];
                    if let Some(ch) = s.chars().next() {
                        out.push(ch);
                        pos += ch.len_utf8();
                    } else {
                        pos += 1;
                    }
                }
            }

            if !closed {
                out.push('"');
            }

            if let Some(frame) = stack.last_mut() {
                match frame {
                    Frame::Object(expect) => {
                        if *expect == ObjectExpect::KeyOrClose {
                            *expect = ObjectExpect::Colon;
                        } else if *expect == ObjectExpect::Value {
                            *expect = ObjectExpect::CommaOrClose;
                        }
                    }
                    Frame::Array(expect) => {
                        if *expect == ArrayExpect::ValueOrClose {
                            *expect = ArrayExpect::CommaOrClose;
                        }
                    }
                }
            }
            continue;
        }

        // Numbers (or leading dot like .5)
        if b.is_ascii_digit() || b == b'-' || (options.lenient_numbers && (b == b'+' || b == b'.'))
        {
            if b == b'.' && (pos + 1 >= len || !bytes[pos + 1].is_ascii_digit()) {
                pos += 1;
                continue;
            }

            let start = pos;
            let mut has_dot = b == b'.';
            let mut has_exp = false;

            if bytes[pos] == b'-' || bytes[pos] == b'+' {
                pos += 1;
            }

            while pos < len {
                let cb = bytes[pos];
                if cb.is_ascii_digit() {
                    pos += 1;
                } else if cb == b'.' && !has_dot && !has_exp {
                    has_dot = true;
                    pos += 1;
                } else if (cb == b'e' || cb == b'E') && !has_exp {
                    has_exp = true;
                    pos += 1;
                    if pos < len && (bytes[pos] == b'+' || bytes[pos] == b'-') {
                        pos += 1;
                    }
                } else {
                    break;
                }
            }

            let num_str = &input[start..pos];
            let repaired_num = repair_number_literal(num_str);

            if let Some(Frame::Object(ObjectExpect::KeyOrClose)) = stack.last() {
                out.push('"');
                out.push_str(&repaired_num);
                out.push('"');
                if let Some(Frame::Object(expect)) = stack.last_mut() {
                    *expect = ObjectExpect::Colon;
                }
            } else {
                out.push_str(&repaired_num);
                on_value_completed(&mut stack);
            }
            continue;
        }

        // Literals, booleans, null, identifiers
        if b.is_ascii_alphabetic() || b == b'_' || b == b'$' {
            let start = pos;
            while pos < len
                && (bytes[pos].is_ascii_alphanumeric()
                    || bytes[pos] == b'_'
                    || bytes[pos] == b'$'
                    || bytes[pos] == b'-')
            {
                pos += 1;
            }
            let word = &input[start..pos];
            let lower = word.to_ascii_lowercase();

            if let Some(Frame::Object(ObjectExpect::KeyOrClose)) = stack.last() {
                out.push('"');
                out.push_str(word);
                out.push('"');
                if let Some(Frame::Object(expect)) = stack.last_mut() {
                    *expect = ObjectExpect::Colon;
                }
                continue;
            }

            if options.autocomplete_literals && lower.starts_with('t') && "true".starts_with(&lower)
            {
                out.push_str("true");
                on_value_completed(&mut stack);
            } else if options.autocomplete_literals
                && lower.starts_with('f')
                && "false".starts_with(&lower)
            {
                out.push_str("false");
                on_value_completed(&mut stack);
            } else if (options.autocomplete_literals
                && lower.starts_with('n')
                && "null".starts_with(&lower))
                || lower == "none"
                || lower == "nil"
                || lower == "undefined"
            {
                out.push_str("null");
                on_value_completed(&mut stack);
            } else if options.allow_python_literals && word == "True" {
                out.push_str("true");
                on_value_completed(&mut stack);
            } else if options.allow_python_literals && word == "False" {
                out.push_str("false");
                on_value_completed(&mut stack);
            } else if options.allow_python_literals && word == "None" {
                out.push_str("null");
                on_value_completed(&mut stack);
            } else if lower == "nan" || lower == "infinity" || lower == "-infinity" {
                out.push_str("null");
                on_value_completed(&mut stack);
            } else {
                out.push('"');
                out.push_str(word);
                out.push('"');
                on_value_completed(&mut stack);
            }
            continue;
        }

        // Advance past unhandled character
        pos += 1;
    }

    // Unwind and close remaining open container frames
    while let Some(frame) = stack.pop() {
        match frame {
            Frame::Object(expect) => {
                match expect {
                    ObjectExpect::KeyOrClose => {
                        trim_trailing_comma(&mut out);
                    }
                    ObjectExpect::Colon => match options.unclosed_key_strategy {
                        UnclosedKeyStrategy::Null => out.push_str(": null"),
                        UnclosedKeyStrategy::EmptyString => out.push_str(": \"\""),
                        UnclosedKeyStrategy::EmptyObject => out.push_str(": {}"),
                        UnclosedKeyStrategy::Omit => {
                            trim_dangling_key(&mut out);
                        }
                    },
                    ObjectExpect::Value => match options.unclosed_key_strategy {
                        UnclosedKeyStrategy::Null => {
                            if !out.ends_with(' ') && !out.ends_with(':') {
                                out.push(' ');
                            }
                            out.push_str("null");
                        }
                        UnclosedKeyStrategy::EmptyString => {
                            if !out.ends_with(' ') && !out.ends_with(':') {
                                out.push(' ');
                            }
                            out.push_str("\"\"");
                        }
                        UnclosedKeyStrategy::EmptyObject => {
                            if !out.ends_with(' ') && !out.ends_with(':') {
                                out.push(' ');
                            }
                            out.push_str("{}");
                        }
                        UnclosedKeyStrategy::Omit => {
                            trim_dangling_key_and_colon(&mut out);
                        }
                    },
                    ObjectExpect::CommaOrClose => {}
                }
                out.push('}');
                on_value_completed(&mut stack);
            }
            Frame::Array(expect) => {
                match expect {
                    ArrayExpect::ValueOrClose => {
                        trim_trailing_comma(&mut out);
                    }
                    ArrayExpect::CommaOrClose => {}
                }
                out.push(']');
                on_value_completed(&mut stack);
            }
        }
    }

    if out.trim().is_empty() {
        out = "null".to_string();
    }

    Ok(out)
}

/// Convenience alias for repairing partial JSON with default options.
pub fn repair_json(raw: &str) -> String {
    repair_partial_json(raw, &PartialJsonOptions::default()).unwrap_or_else(|_| "null".to_string())
}

/// Parses partial or complete JSON into a `serde_json::Value` using default options.
pub fn parse_partial_json(raw: &str) -> Result<Value, PartialJsonError> {
    parse_partial_json_with_options(raw, &PartialJsonOptions::default())
}

/// Parses partial or complete JSON into a `serde_json::Value` using specified options.
pub fn parse_partial_json_with_options(
    raw: &str,
    options: &PartialJsonOptions,
) -> Result<Value, PartialJsonError> {
    let repaired = repair_partial_json(raw, options)?;
    serde_json::from_str(&repaired)
        .map_err(|e| PartialJsonError::ParseError(format!("{e} (repaired JSON: {repaired})")))
}

/// Parses partial JSON into a `serde_json::Value`, returning `Value::Null` if unparseable.
pub fn parse_partial_json_lossy(raw: &str) -> Value {
    parse_partial_json(raw).unwrap_or(Value::Null)
}

/// Parses partial JSON into a `serde_json::Value` with custom options, returning `Value::Null` if unparseable.
pub fn parse_partial_json_lossy_with_options(raw: &str, options: &PartialJsonOptions) -> Value {
    parse_partial_json_with_options(raw, options).unwrap_or(Value::Null)
}

/// Deserializes partial JSON into target type `T` using default options.
pub fn deserialize_partial_json<T: DeserializeOwned>(raw: &str) -> Result<T, PartialJsonError> {
    deserialize_partial_json_with_options(raw, &PartialJsonOptions::default())
}

/// Deserializes partial JSON into target type `T` using custom options.
pub fn deserialize_partial_json_with_options<T: DeserializeOwned>(
    raw: &str,
    options: &PartialJsonOptions,
) -> Result<T, PartialJsonError> {
    let value = parse_partial_json_with_options(raw, options)?;
    serde_json::from_value(value).map_err(|e| PartialJsonError::DeserializationError(e.to_string()))
}

/// Streaming partial JSON parser that accumulates SSE chunks and provides live parsed state.
#[derive(Debug, Clone, Default)]
pub struct StreamingJsonParser {
    raw_buffer: String,
    repaired_json: String,
    current_value: Option<Value>,
    is_complete: bool,
    options: PartialJsonOptions,
}

impl StreamingJsonParser {
    /// Creates a new streaming parser with default options.
    pub fn new() -> Self {
        Self::default()
    }

    /// Creates a new streaming parser with custom options.
    pub fn with_options(options: PartialJsonOptions) -> Self {
        Self {
            options,
            ..Default::default()
        }
    }

    /// Appends a new string chunk to the buffer and updates the repaired/parsed state.
    pub fn feed(&mut self, chunk: &str) -> Option<&Value> {
        if chunk.is_empty() && self.current_value.is_some() {
            return self.current_value.as_ref();
        }
        self.raw_buffer.push_str(chunk);
        self.recompute();
        self.current_value.as_ref()
    }

    /// Recomputes the repaired JSON and parsed value from the internal buffer.
    fn recompute(&mut self) {
        let trimmed = self.raw_buffer.trim();
        if trimmed.is_empty() {
            self.repaired_json.clear();
            self.current_value = None;
            self.is_complete = false;
            return;
        }

        let clean_raw = if self.options.strip_markdown_codeblocks {
            strip_markdown(trimmed)
        } else {
            trimmed
        };

        // Check if raw is already valid complete JSON
        if let Ok(val) = serde_json::from_str::<Value>(clean_raw) {
            self.repaired_json = clean_raw.to_string();
            self.current_value = Some(val);
            self.is_complete = true;
            return;
        }

        self.is_complete = false;

        // Repair partial JSON
        if let Ok(repaired) = repair_partial_json(&self.raw_buffer, &self.options) {
            if let Ok(val) = serde_json::from_str::<Value>(&repaired) {
                self.repaired_json = repaired;
                self.current_value = Some(val);
            }
        }
    }

    /// Returns a reference to the latest parsed `Value`, if available.
    pub fn current_value(&self) -> Option<&Value> {
        self.current_value.as_ref()
    }

    /// Returns the raw accumulated string buffer.
    pub fn current_raw(&self) -> &str {
        &self.raw_buffer
    }

    /// Returns the currently repaired JSON string.
    pub fn repaired_json(&self) -> &str {
        &self.repaired_json
    }

    /// Returns true if the buffer forms a complete, valid JSON document without repair.
    pub fn is_complete(&self) -> bool {
        self.is_complete
    }

    /// Returns true if the buffer is empty.
    pub fn is_empty(&self) -> bool {
        self.raw_buffer.is_empty()
    }

    /// Returns the number of bytes in the raw buffer.
    pub fn len(&self) -> usize {
        self.raw_buffer.len()
    }

    /// Resets the parser state and clears all buffers.
    pub fn reset(&mut self) {
        self.raw_buffer.clear();
        self.repaired_json.clear();
        self.current_value = None;
        self.is_complete = false;
    }

    /// Extracts a string field from the top-level parsed object.
    pub fn get_str(&self, key: &str) -> Option<&str> {
        self.current_value
            .as_ref()
            .and_then(|v| v.get(key))
            .and_then(|v| v.as_str())
    }

    /// Extracts a value from the top-level parsed object.
    pub fn get_value(&self, key: &str) -> Option<&Value> {
        self.current_value.as_ref().and_then(|v| v.get(key))
    }

    /// Navigates a nested path in the parsed JSON value.
    pub fn get_path(&self, path: &[&str]) -> Option<&Value> {
        let mut curr = self.current_value.as_ref()?;
        for &segment in path {
            if let Some(obj) = curr.as_object() {
                curr = obj.get(segment)?;
            } else if let Some(arr) = curr.as_array() {
                let idx: usize = segment.parse().ok()?;
                curr = arr.get(idx)?;
            } else {
                return None;
            }
        }
        Some(curr)
    }

    /// Extracts a boolean field from the top-level parsed object.
    pub fn get_bool(&self, key: &str) -> Option<bool> {
        self.get_value(key).and_then(|v| v.as_bool())
    }

    /// Extracts an i64 field from the top-level parsed object.
    pub fn get_i64(&self, key: &str) -> Option<i64> {
        self.get_value(key).and_then(|v| v.as_i64())
    }

    /// Extracts a u64 field from the top-level parsed object.
    pub fn get_u64(&self, key: &str) -> Option<u64> {
        self.get_value(key).and_then(|v| v.as_u64())
    }

    /// Extracts an f64 field from the top-level parsed object.
    pub fn get_f64(&self, key: &str) -> Option<f64> {
        self.get_value(key).and_then(|v| v.as_f64())
    }

    /// Extracts an array reference from the top-level parsed object.
    pub fn get_array(&self, key: &str) -> Option<&Vec<Value>> {
        self.get_value(key).and_then(|v| v.as_array())
    }

    /// Extracts an object reference from the top-level parsed object.
    pub fn get_object(&self, key: &str) -> Option<&serde_json::Map<String, Value>> {
        self.get_value(key).and_then(|v| v.as_object())
    }

    /// Returns a list of top-level keys if the parsed value is an object.
    pub fn keys(&self) -> Vec<String> {
        self.current_value
            .as_ref()
            .and_then(|v| v.as_object())
            .map(|obj| obj.keys().cloned().collect())
            .unwrap_or_default()
    }

    /// Returns true if the top-level object contains the given key.
    pub fn has_key(&self, key: &str) -> bool {
        self.current_value
            .as_ref()
            .and_then(|v| v.as_object())
            .map(|obj| obj.contains_key(key))
            .unwrap_or(false)
    }

    /// Attempts to deserialize the current repaired JSON into type `T`.
    pub fn deserialize<T: DeserializeOwned>(&self) -> Result<T, PartialJsonError> {
        let val = self
            .current_value
            .as_ref()
            .ok_or(PartialJsonError::EmptyInput)?;
        serde_json::from_value(val.clone())
            .map_err(|e| PartialJsonError::DeserializationError(e.to_string()))
    }

    /// Deserializes a specific field from the top-level object into type `T`.
    pub fn extract_field<T: DeserializeOwned>(&self, key: &str) -> Option<T> {
        let val = self.get_value(key)?;
        serde_json::from_value(val.clone()).ok()
    }
}

/// Represents an in-progress or completed tool call accumulated from SSE chunks.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PartialToolCall {
    pub index: usize,
    pub id: Option<String>,
    pub name: Option<String>,
    pub raw_arguments: String,
    pub repaired_arguments: String,
    pub parsed_arguments: Value,
    pub is_complete: bool,
}

impl PartialToolCall {
    /// Returns the string value of an argument key, if present.
    pub fn get_arg_str(&self, key: &str) -> Option<&str> {
        self.parsed_arguments.get(key).and_then(|v| v.as_str())
    }

    /// Returns the JSON value of an argument key, if present.
    pub fn get_arg_value(&self, key: &str) -> Option<&Value> {
        self.parsed_arguments.get(key)
    }

    /// Returns the boolean value of an argument key, if present.
    pub fn get_arg_bool(&self, key: &str) -> Option<bool> {
        self.parsed_arguments.get(key).and_then(|v| v.as_bool())
    }

    /// Returns the i64 value of an argument key, if present.
    pub fn get_arg_i64(&self, key: &str) -> Option<i64> {
        self.parsed_arguments.get(key).and_then(|v| v.as_i64())
    }

    /// Returns the u64 value of an argument key, if present.
    pub fn get_arg_u64(&self, key: &str) -> Option<u64> {
        self.parsed_arguments.get(key).and_then(|v| v.as_u64())
    }

    /// Returns the f64 value of an argument key, if present.
    pub fn get_arg_f64(&self, key: &str) -> Option<f64> {
        self.parsed_arguments.get(key).and_then(|v| v.as_f64())
    }

    /// Returns an array argument reference, if present.
    pub fn get_arg_array(&self, key: &str) -> Option<&Vec<Value>> {
        self.parsed_arguments.get(key).and_then(|v| v.as_array())
    }

    /// Returns an object argument reference, if present.
    pub fn get_arg_object(&self, key: &str) -> Option<&serde_json::Map<String, Value>> {
        self.parsed_arguments.get(key).and_then(|v| v.as_object())
    }

    /// Deserializes the tool call arguments into type `T`.
    pub fn deserialize_args<T: DeserializeOwned>(&self) -> Result<T, PartialJsonError> {
        serde_json::from_value(self.parsed_arguments.clone())
            .map_err(|e| PartialJsonError::DeserializationError(e.to_string()))
    }

    /// Converts this partial tool call into a final `ToolCall`.
    pub fn to_tool_call(&self) -> ToolCall {
        ToolCall {
            id: self.id.clone().unwrap_or_default(),
            name: self.name.clone().unwrap_or_default(),
            arguments: if self.repaired_arguments.is_empty() {
                "{}".to_string()
            } else {
                self.repaired_arguments.clone()
            },
        }
    }
}

/// Accumulator for tracking and parsing multiple streaming tool calls from SSE chunks.
#[derive(Debug, Clone, Default)]
pub struct StreamingToolCallAccumulator {
    parsers: BTreeMap<usize, (Option<String>, Option<String>, StreamingJsonParser)>,
    options: PartialJsonOptions,
}

impl StreamingToolCallAccumulator {
    /// Creates a new accumulator with default options.
    pub fn new() -> Self {
        Self::default()
    }

    /// Creates a new accumulator with custom options.
    pub fn with_options(options: PartialJsonOptions) -> Self {
        Self {
            options,
            parsers: BTreeMap::new(),
        }
    }

    /// Processes an individual tool call delta.
    pub fn process_delta(
        &mut self,
        index: usize,
        id: Option<String>,
        name: Option<String>,
        arguments_delta: &str,
    ) {
        let entry = self.parsers.entry(index).or_insert_with(|| {
            (
                None,
                None,
                StreamingJsonParser::with_options(self.options.clone()),
            )
        });

        if let Some(new_id) = id {
            entry.0 = Some(new_id);
        }
        if let Some(new_name) = name {
            entry.1 = Some(new_name);
        }
        if !arguments_delta.is_empty() {
            entry.2.feed(arguments_delta);
        }
    }

    /// Processes a `StreamChunk`, updating the accumulator if it is a `ToolCallDelta`.
    pub fn process_chunk(&mut self, chunk: &StreamChunk) -> bool {
        if let StreamChunk::ToolCallDelta {
            index,
            id,
            name,
            arguments_delta,
        } = chunk
        {
            self.process_delta(*index, id.clone(), name.clone(), arguments_delta);
            true
        } else {
            false
        }
    }

    /// Returns the partial tool call at the specified index.
    pub fn get_partial_call(&self, index: usize) -> Option<PartialToolCall> {
        let (id, name, parser) = self.parsers.get(&index)?;
        Some(PartialToolCall {
            index,
            id: id.clone(),
            name: name.clone(),
            raw_arguments: parser.current_raw().to_string(),
            repaired_arguments: parser.repaired_json().to_string(),
            parsed_arguments: parser.current_value().cloned().unwrap_or(Value::Null),
            is_complete: parser.is_complete(),
        })
    }

    /// Returns all partial tool calls accumulated so far in ascending index order.
    pub fn get_all_partial_calls(&self) -> Vec<PartialToolCall> {
        self.parsers
            .keys()
            .filter_map(|&idx| self.get_partial_call(idx))
            .collect()
    }

    /// Finds a partial tool call by its tool call ID string.
    pub fn get_call_by_id(&self, target_id: &str) -> Option<PartialToolCall> {
        self.parsers.iter().find_map(|(&idx, (id, _, _))| {
            if id.as_deref() == Some(target_id) {
                self.get_partial_call(idx)
            } else {
                None
            }
        })
    }

    /// Returns the latest active partial tool call (highest index).
    pub fn get_latest_call(&self) -> Option<PartialToolCall> {
        let &max_idx = self.parsers.keys().next_back()?;
        self.get_partial_call(max_idx)
    }

    /// Converts all accumulated tool calls into completed `ToolCall` structs.
    pub fn to_tool_calls(&self) -> Vec<ToolCall> {
        self.get_all_partial_calls()
            .into_iter()
            .map(|p| p.to_tool_call())
            .collect()
    }

    /// Returns true if the tool call arguments at `index` form complete JSON.
    pub fn is_complete(&self, index: usize) -> bool {
        self.parsers
            .get(&index)
            .map(|(_, _, p)| p.is_complete())
            .unwrap_or(false)
    }

    /// Returns true if all accumulated tool calls form complete JSON.
    pub fn is_all_complete(&self) -> bool {
        !self.parsers.is_empty() && self.parsers.values().all(|(_, _, p)| p.is_complete())
    }

    /// Clears all accumulated tool calls and parsers.
    pub fn clear(&mut self) {
        self.parsers.clear();
    }

    /// Returns the number of tool calls currently tracked.
    pub fn len(&self) -> usize {
        self.parsers.len()
    }

    /// Returns true if no tool calls are tracked.
    pub fn is_empty(&self) -> bool {
        self.parsers.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[derive(Debug, Deserialize, PartialEq)]
    struct CatArgs {
        path: String,
        line_numbers: Option<bool>,
    }

    #[derive(Debug, Deserialize, PartialEq)]
    struct ComplexConfig {
        name: String,
        count: u32,
        enabled: bool,
        tags: Vec<String>,
        extra: Option<String>,
    }

    #[test]
    fn test_empty_input() {
        assert_eq!(parse_partial_json("").unwrap(), Value::Null);
        assert_eq!(parse_partial_json("   \n\t ").unwrap(), Value::Null);
        assert_eq!(parse_partial_json_lossy(""), Value::Null);
        assert_eq!(repair_json(""), "null");
    }

    #[test]
    fn test_primitive_values() {
        assert_eq!(parse_partial_json("true").unwrap(), json!(true));
        assert_eq!(parse_partial_json("false").unwrap(), json!(false));
        assert_eq!(parse_partial_json("null").unwrap(), json!(null));
        assert_eq!(parse_partial_json("123").unwrap(), json!(123));
        assert_eq!(parse_partial_json("-456").unwrap(), json!(-456));
        assert_eq!(parse_partial_json("3.1415").unwrap(), json!(3.1415));
        assert_eq!(
            parse_partial_json("\"hello world\"").unwrap(),
            json!("hello world")
        );
    }

    #[test]
    fn test_truncated_primitive_literals() {
        assert_eq!(parse_partial_json("t").unwrap(), json!(true));
        assert_eq!(parse_partial_json("tr").unwrap(), json!(true));
        assert_eq!(parse_partial_json("tru").unwrap(), json!(true));
        assert_eq!(parse_partial_json("f").unwrap(), json!(false));
        assert_eq!(parse_partial_json("fa").unwrap(), json!(false));
        assert_eq!(parse_partial_json("fal").unwrap(), json!(false));
        assert_eq!(parse_partial_json("fals").unwrap(), json!(false));
        assert_eq!(parse_partial_json("n").unwrap(), json!(null));
        assert_eq!(parse_partial_json("nu").unwrap(), json!(null));
        assert_eq!(parse_partial_json("nul").unwrap(), json!(null));
    }

    #[test]
    fn test_python_and_extended_literals() {
        assert_eq!(parse_partial_json("True").unwrap(), json!(true));
        assert_eq!(parse_partial_json("False").unwrap(), json!(false));
        assert_eq!(parse_partial_json("None").unwrap(), json!(null));
        assert_eq!(parse_partial_json("undefined").unwrap(), json!(null));
        assert_eq!(parse_partial_json("nil").unwrap(), json!(null));
        assert_eq!(parse_partial_json("NaN").unwrap(), json!(null));
        assert_eq!(parse_partial_json("Infinity").unwrap(), json!(null));
        assert_eq!(parse_partial_json("-Infinity").unwrap(), json!(null));
    }

    #[test]
    fn test_truncated_and_lenient_numbers() {
        assert_eq!(
            parse_partial_json(r#"{"num": 42."#).unwrap(),
            json!({"num": 42.0})
        );
        assert_eq!(
            parse_partial_json(r#"{"num": 1e"#).unwrap(),
            json!({"num": 1.0})
        );
        assert_eq!(
            parse_partial_json(r#"{"num": 1e+"#).unwrap(),
            json!({"num": 1.0})
        );
        assert_eq!(
            parse_partial_json(r#"{"num": 1e-"#).unwrap(),
            json!({"num": 1.0})
        );
        assert_eq!(
            parse_partial_json(r#"{"num": -"#).unwrap(),
            json!({"num": 0})
        );
        assert_eq!(
            parse_partial_json(r#"{"num": +"#).unwrap(),
            json!({"num": 0})
        );
        assert_eq!(
            parse_partial_json(r#"{"num": +42"#).unwrap(),
            json!({"num": 42})
        );
        assert_eq!(
            parse_partial_json(r#"{"num": .5"#).unwrap(),
            json!({"num": 0.5})
        );
        assert_eq!(
            parse_partial_json(r#"{"num": -.5"#).unwrap(),
            json!({"num": -0.5})
        );
    }

    #[test]
    fn test_basic_object_repair() {
        let raw = r#"{"command": "cargo build"#;
        let val = parse_partial_json(raw).unwrap();
        assert_eq!(val, json!({"command": "cargo build"}));

        let raw = r#"{"command": "cargo build", "release": tr"#;
        let val = parse_partial_json(raw).unwrap();
        assert_eq!(val, json!({"command": "cargo build", "release": true}));

        let raw = r#"{"command": "cargo build", "release": true}"#;
        let val = parse_partial_json(raw).unwrap();
        assert_eq!(val, json!({"command": "cargo build", "release": true}));
    }

    #[test]
    fn test_unclosed_string_repair() {
        let raw = r#"{"query": "SELECT * FROM users WHERE active = 1"#;
        let val = parse_partial_json(raw).unwrap();
        assert_eq!(
            val,
            json!({"query": "SELECT * FROM users WHERE active = 1"})
        );

        let raw = r#"["first", "second unclosed"#;
        let val = parse_partial_json(raw).unwrap();
        assert_eq!(val, json!(["first", "second unclosed"]));
    }

    #[test]
    fn test_escaped_characters() {
        let raw = r#"{"msg": "He said \"hello world\" and then stopped"#;
        let val = parse_partial_json(raw).unwrap();
        assert_eq!(
            val,
            json!({"msg": "He said \"hello world\" and then stopped"})
        );

        let raw = r#"{"path": "C:\\Windows\\System32\\cmd.exe"#;
        let val = parse_partial_json(raw).unwrap();
        assert_eq!(val, json!({"path": "C:\\Windows\\System32\\cmd.exe"}));

        // Escaped newlines and tabs
        let raw = r#"{"text": "line1\nline2\ttabbed"#;
        let val = parse_partial_json(raw).unwrap();
        assert_eq!(val, json!({"text": "line1\nline2\ttabbed"}));

        // Backslash at end of string
        let raw = r#"{"path": "C:\"#;
        let val = parse_partial_json(raw).unwrap();
        assert_eq!(val, json!({"path": "C:"}));
    }

    #[test]
    fn test_unicode_escapes() {
        let raw = r#"{"letter": "\u0041"}"#;
        let val = parse_partial_json(raw).unwrap();
        assert_eq!(val, json!({"letter": "A"}));

        // Truncated unicode escape
        let raw = r#"{"letter": "\u004"#;
        let val = parse_partial_json(raw).unwrap();
        assert_eq!(val, json!({"letter": "@"}));

        let raw = r#"{"letter": "\u"#;
        let val = parse_partial_json(raw).unwrap();
        assert_eq!(val, json!({"letter": "\u{0}"}));

        // Complete surrogate pair (e.g. 😀 U+1F600 = \uD83D\uDE00)
        let raw = r#"{"emoji": "\uD83D\uDE00"}"#;
        let val = parse_partial_json(raw).unwrap();
        assert_eq!(val, json!({"emoji": "😀"}));

        // Truncated surrogate pair (lone high surrogate -> U+FFFD)
        let raw = r#"{"emoji": "\uD83D"#;
        let val = parse_partial_json(raw).unwrap();
        assert_eq!(val, json!({"emoji": "\u{FFFD}"}));
    }

    #[test]
    fn test_single_quoted_strings() {
        let raw = r#"{'name': 'Alice', 'role': 'admin'}"#;
        let val = parse_partial_json(raw).unwrap();
        assert_eq!(val, json!({"name": "Alice", "role": "admin"}));

        let raw = r#"{'quote': "He said 'hello'"}"#;
        let val = parse_partial_json(raw).unwrap();
        assert_eq!(val, json!({"quote": "He said 'hello'"}));

        let raw = r#"{'quote': 'He said "hello"'}"#;
        let val = parse_partial_json(raw).unwrap();
        assert_eq!(val, json!({"quote": "He said \"hello\""}));

        let raw = r#"{'text': 'It\'s working'}"#;
        let val = parse_partial_json(raw).unwrap();
        assert_eq!(val, json!({"text": "It's working"}));
    }

    #[test]
    fn test_unquoted_keys_and_identifiers() {
        let raw = r#"{name: 'Bob', active: True, score: None}"#;
        let val = parse_partial_json(raw).unwrap();
        assert_eq!(val, json!({"name": "Bob", "active": true, "score": null}));

        let raw = r#"{$special_id: 123, user-tag: "primary"}"#;
        let val = parse_partial_json(raw).unwrap();
        assert_eq!(val, json!({"$special_id": 123, "user-tag": "primary"}));

        let raw = r#"{123: "numeric_key"}"#;
        let val = parse_partial_json(raw).unwrap();
        assert_eq!(val, json!({"123": "numeric_key"}));

        let raw = r#"[apple, banana, cherry]"#;
        let val = parse_partial_json(raw).unwrap();
        assert_eq!(val, json!(["apple", "banana", "cherry"]));
    }

    #[test]
    fn test_nested_objects_and_arrays() {
        let raw = r#"{"files": [{"path": "src/main.rs", "lines": [1, 2, 3"#;
        let val = parse_partial_json(raw).unwrap();
        assert_eq!(
            val,
            json!({
                "files": [
                    {
                        "path": "src/main.rs",
                        "lines": [1, 2, 3]
                    }
                ]
            })
        );

        let raw = r#"{"a": {"b": {"c": [1, [2, [3"#;
        let val = parse_partial_json(raw).unwrap();
        assert_eq!(
            val,
            json!({
                "a": {
                    "b": {
                        "c": [1, [2, [3]]]
                    }
                }
            })
        );
    }

    #[test]
    fn test_unclosed_arrays_and_objects_boundary() {
        let raw = r#"{"key": "val", "arr": ["#;
        let val = parse_partial_json(raw).unwrap();
        assert_eq!(val, json!({"key": "val", "arr": []}));

        let raw = r#"{"key": "val", "arr": [1, 2,"#;
        let val = parse_partial_json(raw).unwrap();
        assert_eq!(val, json!({"key": "val", "arr": [1, 2]}));

        let raw = r#"{"key": "val", "obj": {"#;
        let val = parse_partial_json(raw).unwrap();
        assert_eq!(val, json!({"key": "val", "obj": {}}));

        let raw = r#"{"key": "val", "obj": {"nested": 1,"#;
        let val = parse_partial_json(raw).unwrap();
        assert_eq!(val, json!({"key": "val", "obj": {"nested": 1}}));
    }

    #[test]
    fn test_trailing_commas() {
        let raw = r#"{"a": 1, "b": 2, "#;
        let val = parse_partial_json(raw).unwrap();
        assert_eq!(val, json!({"a": 1, "b": 2}));

        let raw = r#"[1, 2, 3, "#;
        let val = parse_partial_json(raw).unwrap();
        assert_eq!(val, json!([1, 2, 3]));
    }

    #[test]
    fn test_unclosed_key_strategies() {
        let raw = r#"{"a": 1, "pending_key"#;

        // Default Null strategy
        let val = parse_partial_json(raw).unwrap();
        assert_eq!(val, json!({"a": 1, "pending_key": null}));

        // EmptyString strategy
        let opts_str = PartialJsonOptions {
            unclosed_key_strategy: UnclosedKeyStrategy::EmptyString,
            ..Default::default()
        };
        let val_str = parse_partial_json_with_options(raw, &opts_str).unwrap();
        assert_eq!(val_str, json!({"a": 1, "pending_key": ""}));

        // EmptyObject strategy
        let opts_obj = PartialJsonOptions {
            unclosed_key_strategy: UnclosedKeyStrategy::EmptyObject,
            ..Default::default()
        };
        let val_obj = parse_partial_json_with_options(raw, &opts_obj).unwrap();
        assert_eq!(val_obj, json!({"a": 1, "pending_key": {}}));

        // Omit strategy
        let opts_omit = PartialJsonOptions {
            unclosed_key_strategy: UnclosedKeyStrategy::Omit,
            ..Default::default()
        };
        let val_omit = parse_partial_json_with_options(raw, &opts_omit).unwrap();
        assert_eq!(val_omit, json!({"a": 1}));

        // Omit with colon
        let raw_colon = r#"{"a": 1, "pending_key": "#;
        let val_omit_colon = parse_partial_json_with_options(raw_colon, &opts_omit).unwrap();
        assert_eq!(val_omit_colon, json!({"a": 1}));
    }

    #[test]
    fn test_comments_stripping() {
        let raw = r#"
        // Configuration object
        {
            /* Author info */
            "author": "Alice", // admin user
            # Secondary tag
            "tag": "prod"
        }
        "#;
        let val = parse_partial_json(raw).unwrap();
        assert_eq!(val, json!({"author": "Alice", "tag": "prod"}));

        // Unclosed block comment at EOF
        let raw = r#"{"a": 1 /* unclosed comment"#;
        let val = parse_partial_json(raw).unwrap();
        assert_eq!(val, json!({"a": 1}));
    }

    #[test]
    fn test_markdown_codeblock_stripping() {
        let raw = "```json\n{\"command\": \"cat Cargo.toml\"}\n```";
        let val = parse_partial_json(raw).unwrap();
        assert_eq!(val, json!({"command": "cat Cargo.toml"}));

        let raw = "```json\n{\"command\": \"cat Cargo.toml";
        let val = parse_partial_json(raw).unwrap();
        assert_eq!(val, json!({"command": "cat Cargo.toml"}));

        let raw = "```\n{\"command\": \"cat Cargo.toml\"}\n```";
        let val = parse_partial_json(raw).unwrap();
        assert_eq!(val, json!({"command": "cat Cargo.toml"}));
    }

    #[test]
    fn test_auto_escape_newlines_in_strings() {
        let raw = "{\"script\": \"echo 1\necho 2\necho 3";
        let val = parse_partial_json(raw).unwrap();
        assert_eq!(val, json!({"script": "echo 1\necho 2\necho 3"}));
    }

    #[test]
    fn test_max_depth_limit() {
        let opts = PartialJsonOptions {
            max_depth: 3,
            ..Default::default()
        };
        let deep = "[[[[1]]]]";
        let err = parse_partial_json_with_options(deep, &opts).unwrap_err();
        assert!(matches!(err, PartialJsonError::MaxDepthExceeded(3)));
    }

    #[test]
    fn test_streaming_parser_step_by_step() {
        let mut parser = StreamingJsonParser::new();

        parser.feed(r#"{"path": ""#);
        assert_eq!(parser.get_str("path"), Some(""));
        assert!(!parser.is_complete());

        parser.feed("src/");
        assert_eq!(parser.get_str("path"), Some("src/"));
        assert!(!parser.is_complete());

        parser.feed("lib.rs\"");
        assert_eq!(parser.get_str("path"), Some("src/lib.rs"));
        assert!(!parser.is_complete());

        parser.feed(r#", "line_numbers": true}"#);
        assert_eq!(parser.get_str("path"), Some("src/lib.rs"));
        assert_eq!(parser.get_bool("line_numbers"), Some(true));
        assert!(parser.is_complete());

        let args: CatArgs = parser.deserialize().unwrap();
        assert_eq!(
            args,
            CatArgs {
                path: "src/lib.rs".to_string(),
                line_numbers: Some(true),
            }
        );
    }

    #[test]
    fn test_streaming_parser_accessors() {
        let mut parser = StreamingJsonParser::new();
        parser.feed(r#"{"name": "Fusion", "count": 42, "score": 98.5, "enabled": true, "tags": ["rust", "ai"], "meta": {"owner": "user"}}"#);

        assert_eq!(parser.get_str("name"), Some("Fusion"));
        assert_eq!(parser.get_i64("count"), Some(42));
        assert_eq!(parser.get_u64("count"), Some(42));
        assert_eq!(parser.get_f64("score"), Some(98.5));
        assert_eq!(parser.get_bool("enabled"), Some(true));
        assert!(parser.has_key("name"));
        assert!(!parser.has_key("nonexistent"));
        assert_eq!(parser.keys().len(), 6);
        assert_eq!(parser.get_path(&["meta", "owner"]), Some(&json!("user")));
        assert_eq!(parser.get_path(&["tags", "0"]), Some(&json!("rust")));
        assert_eq!(parser.get_path(&["tags", "1"]), Some(&json!("ai")));

        let tags = parser.get_array("tags").unwrap();
        assert_eq!(tags.len(), 2);

        let meta = parser.get_object("meta").unwrap();
        assert_eq!(meta.get("owner"), Some(&json!("user")));

        let owner: String = parser.extract_field("name").unwrap();
        assert_eq!(owner, "Fusion");

        parser.reset();
        assert!(parser.is_empty());
        assert_eq!(parser.len(), 0);
        assert!(parser.current_value().is_none());
    }

    #[test]
    fn test_streaming_tool_call_accumulator() {
        let mut acc = StreamingToolCallAccumulator::new();

        acc.process_delta(
            0,
            Some("call_1".into()),
            Some("read_file".into()),
            "{\"path\": \"",
        );
        assert_eq!(acc.len(), 1);
        let partial = acc.get_partial_call(0).unwrap();
        assert_eq!(partial.name.as_deref(), Some("read_file"));
        assert_eq!(partial.get_arg_str("path"), Some(""));
        assert!(!acc.is_all_complete());

        acc.process_delta(0, None, None, "README.md\"}");
        assert!(acc.is_complete(0));
        assert!(acc.is_all_complete());

        let tool_calls = acc.to_tool_calls();
        assert_eq!(tool_calls.len(), 1);
        assert_eq!(tool_calls[0].id, "call_1");
        assert_eq!(tool_calls[0].name, "read_file");
        assert_eq!(tool_calls[0].arguments, "{\"path\": \"README.md\"}");

        // Test multiple tool calls
        acc.process_delta(
            1,
            Some("call_2".into()),
            Some("grep".into()),
            "{\"pattern\": \"TODO\"",
        );
        assert_eq!(acc.len(), 2);
        assert!(!acc.is_all_complete());

        let latest = acc.get_latest_call().unwrap();
        assert_eq!(latest.index, 1);
        assert_eq!(latest.get_arg_str("pattern"), Some("TODO"));

        let by_id = acc.get_call_by_id("call_1").unwrap();
        assert_eq!(by_id.name.as_deref(), Some("read_file"));

        acc.clear();
        assert!(acc.is_empty());
        assert_eq!(acc.len(), 0);
    }

    #[test]
    fn test_partial_tool_call_helpers() {
        let mut parser = StreamingJsonParser::new();
        parser.feed(r#"{"path": "src/main.rs", "count": 10, "ratio": 0.75, "active": true, "items": ["a", "b"], "nested": {"k": "v"}}"#);

        let ptc = PartialToolCall {
            index: 0,
            id: Some("call_123".to_string()),
            name: Some("test_tool".to_string()),
            raw_arguments: parser.current_raw().to_string(),
            repaired_arguments: parser.repaired_json().to_string(),
            parsed_arguments: parser.current_value().cloned().unwrap(),
            is_complete: true,
        };

        assert_eq!(ptc.get_arg_str("path"), Some("src/main.rs"));
        assert_eq!(ptc.get_arg_i64("count"), Some(10));
        assert_eq!(ptc.get_arg_u64("count"), Some(10));
        assert_eq!(ptc.get_arg_f64("ratio"), Some(0.75));
        assert_eq!(ptc.get_arg_bool("active"), Some(true));
        assert_eq!(ptc.get_arg_array("items").map(|a| a.len()), Some(2));
        assert!(ptc.get_arg_object("nested").is_some());

        let tool_call = ptc.to_tool_call();
        assert_eq!(tool_call.id, "call_123");
        assert_eq!(tool_call.name, "test_tool");
    }

    #[test]
    fn test_deserialize_partial_json_helpers() {
        let raw = r#"{"name": "Fusion", "count": 10, "enabled": true, "tags": ["fast", "rust"#;
        let config: ComplexConfig = deserialize_partial_json(raw).unwrap();
        assert_eq!(
            config,
            ComplexConfig {
                name: "Fusion".to_string(),
                count: 10,
                enabled: true,
                tags: vec!["fast".to_string(), "rust".to_string()],
                extra: None,
            }
        );

        let lossy =
            parse_partial_json_lossy_with_options("{invalid: ", &PartialJsonOptions::default());
        assert_eq!(lossy, json!({"invalid": null}));
    }

    #[test]
    fn test_process_chunk_stream() {
        let mut acc = StreamingToolCallAccumulator::new();
        let chunk = StreamChunk::ToolCallDelta {
            index: 0,
            id: Some("call_abc".to_string()),
            name: Some("fetch".to_string()),
            arguments_delta: "{\"url\": \"https://".to_string(),
        };
        assert!(acc.process_chunk(&chunk));

        let non_tool_chunk = StreamChunk::ContentDelta("hello".to_string());
        assert!(!acc.process_chunk(&non_tool_chunk));

        let call = acc.get_partial_call(0).unwrap();
        assert_eq!(call.get_arg_str("url"), Some("https://"));
    }
}
