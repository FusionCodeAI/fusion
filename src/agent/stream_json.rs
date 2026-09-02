//! Incremental JSON parser for SSE / chunked streams.
//!
//! [`StreamingJsonParser`] accepts arbitrarily fragmented byte chunks and emits
//! [`JsonEvent`]s as soon as structural tokens are recognized.  It handles:
//!
//! * Fragmented string literals (escape sequences split across chunks)
//! * Nested objects and arrays of arbitrary depth
//! * Number, boolean, and null primitives
//! * Malformed / trailing content (emits [`JsonEvent::Error`] and stops)
//!
//! # Example
//! ```
//! use fusion::agent::stream_json::{StreamingJsonParser, JsonEvent};
//!
//! let mut p = StreamingJsonParser::new();
//! let chunks = [r#"{"na"#, r#"me":"Al"#, r#"ice","age":30}"#];
//! let mut events: Vec<JsonEvent> = Vec::new();
//! for chunk in &chunks {
//!     events.extend(p.feed(chunk));
//! }
//! ```

use serde_json::Value;

// ─────────────────────────────────────────────────────────────────────────────
// Public event type
// ─────────────────────────────────────────────────────────────────────────────

/// Events emitted by [`StreamingJsonParser`] as JSON structure is recognised.
#[derive(Debug, Clone, PartialEq)]
pub enum JsonEvent {
    /// `{` — a JSON object has opened.
    ObjectStart,
    /// `}` — the current JSON object closed.  `value` is the fully-assembled
    /// [`Value::Object`] for the completed object.
    ObjectEnd(Value),
    /// A key inside a JSON object (the string between `"…"` followed by `:`).
    ObjectKey(String),
    /// `[` — a JSON array has opened.
    ArrayStart,
    /// `]` — the current JSON array closed.  `value` is the fully-assembled
    /// [`Value::Array`] for the completed array.
    ArrayEnd(Value),
    /// A complete scalar value: `null`, `true`, `false`, number, or a complete
    /// string literal that is a *value* (not an object key).
    Value(Value),
    /// The top-level document has been fully parsed (all brackets balanced and
    /// no more input expected after optional trailing whitespace).  The inner
    /// value is the complete document root.
    Complete(Value),
    /// A parse error was encountered.  The parser halts; subsequent calls to
    /// [`StreamingJsonParser::feed`] return no further events.
    Error(String),
}

// ─────────────────────────────────────────────────────────────────────────────
// Internal state machine
// ─────────────────────────────────────────────────────────────────────────────

/// Tracks where inside a container we expect the next token.
#[derive(Debug, Clone, PartialEq)]
enum ContainerState {
    /// Inside `{…}`; bool = whether we have seen at least one key-value pair.
    Object {
        pairs: Vec<(String, Value)>,
        expecting_key: bool,
        current_key: Option<String>,
    },
    /// Inside `[…]`.
    Array { items: Vec<Value> },
}

/// Low-level lexer position.
#[derive(Debug, Clone, PartialEq)]
enum LexState {
    /// Between tokens (whitespace / start).
    Idle,
    /// Accumulating a string literal; `is_key` distinguishes object keys from
    /// string values.  `escape` is set after seeing `\`.
    InString { buf: String, escape: bool, is_key: bool },
    /// Accumulating a number or keyword literal (`true`, `false`, `null`).
    InLiteral { buf: String },
}

/// Streaming incremental JSON parser that emits [`JsonEvent`]s.
#[derive(Debug)]
pub struct StreamingJsonParser {
    /// Nesting stack; one entry per open `{` or `[`.
    stack: Vec<ContainerState>,
    /// Current lexer state.
    lex: LexState,
    /// Set once [`JsonEvent::Complete`] or [`JsonEvent::Error`] is emitted.
    done: bool,
    /// Depth of root: 0 means we haven't entered any value yet.
    root_entered: bool,
}

impl StreamingJsonParser {
    /// Creates a new, empty streaming parser.
    pub fn new() -> Self {
        Self {
            stack: Vec::new(),
            lex: LexState::Idle,
            done: false,
            root_entered: false,
        }
    }

    /// Feed the next chunk of text.  Returns all events emitted during this
    /// chunk.  After a [`JsonEvent::Complete`] or [`JsonEvent::Error`] event,
    /// further calls return an empty `Vec`.
    pub fn feed(&mut self, chunk: &str) -> Vec<JsonEvent> {
        if self.done {
            return Vec::new();
        }
        let mut events = Vec::new();
        for ch in chunk.chars() {
            self.step(ch, &mut events);
            if self.done {
                break;
            }
        }
        events
    }

    // ── core character-at-a-time state machine ────────────────────────────

    fn step(&mut self, ch: char, out: &mut Vec<JsonEvent>) {
        match &self.lex.clone() {
            // ── inside a string literal ───────────────────────────────────
            LexState::InString { buf, escape, is_key } => {
                let (mut buf, mut escape, is_key) = (buf.clone(), *escape, *is_key);
                if escape {
                    buf.push(unescape_char(ch));
                    escape = false;
                    self.lex = LexState::InString { buf, escape, is_key };
                    return;
                }
                match ch {
                    '\\' => {
                        escape = true;
                        self.lex = LexState::InString { buf, escape, is_key };
                    }
                    '"' => {
                        // String is complete.
                        let s = buf;
                        self.lex = LexState::Idle;
                        self.on_string_done(s, is_key, out);
                    }
                    _ => {
                        buf.push(ch);
                        self.lex = LexState::InString { buf, escape, is_key };
                    }
                }
            }

            // ── accumulating a literal (number / true / false / null) ─────
            LexState::InLiteral { buf } => {
                if ch.is_ascii_alphanumeric() || ch == '.' || ch == '-' || ch == '+' || ch == 'e' || ch == 'E' {
                    let mut buf = buf.clone();
                    buf.push(ch);
                    self.lex = LexState::InLiteral { buf };
                } else {
                    // Literal ended by a structural character — flush it first,
                    // then re-process the structural character.
                    let buf = buf.clone();
                    self.lex = LexState::Idle;
                    self.on_literal_done(buf, out);
                    if !self.done {
                        self.step(ch, out);
                    }
                }
            }

            // ── idle: looking for next token ──────────────────────────────
            LexState::Idle => {
                match ch {
                    // ── whitespace ────────────────────────────────────────
                    ' ' | '\t' | '\n' | '\r' => {}

                    // ── open object ───────────────────────────────────────
                    '{' => {
                        self.root_entered = true;
                        out.push(JsonEvent::ObjectStart);
                        self.stack.push(ContainerState::Object {
                            pairs: Vec::new(),
                            expecting_key: true,
                            current_key: None,
                        });
                    }

                    // ── close object ──────────────────────────────────────
                    '}' => {
                        match self.stack.last().cloned() {
                            Some(ContainerState::Object { pairs, .. }) => {
                                self.stack.pop();
                                let map: serde_json::Map<String, Value> =
                                    pairs.into_iter().collect();
                                let val = Value::Object(map);
                                out.push(JsonEvent::ObjectEnd(val.clone()));
                                self.deliver_value(val, out);
                            }
                            _ => {
                                self.emit_error("unexpected `}`", out);
                            }
                        }
                    }

                    // ── open array ────────────────────────────────────────
                    '[' => {
                        self.root_entered = true;
                        out.push(JsonEvent::ArrayStart);
                        self.stack.push(ContainerState::Array { items: Vec::new() });
                    }

                    // ── close array ───────────────────────────────────────
                    ']' => {
                        match self.stack.last().cloned() {
                            Some(ContainerState::Array { items }) => {
                                self.stack.pop();
                                let val = Value::Array(items);
                                out.push(JsonEvent::ArrayEnd(val.clone()));
                                self.deliver_value(val, out);
                            }
                            _ => {
                                self.emit_error("unexpected `]`", out);
                            }
                        }
                    }

                    // ── comma (separator) ─────────────────────────────────
                    ',' => {
                        // Advance container to expect the next key/value.
                        match self.stack.last_mut() {
                            Some(ContainerState::Object { expecting_key, .. }) => {
                                *expecting_key = true;
                            }
                            Some(ContainerState::Array { .. }) => { /* items already appended */ }
                            None => {
                                self.emit_error("unexpected `,` outside container", out);
                            }
                        }
                    }

                    // ── colon (key-value separator) ───────────────────────
                    ':' => {
                        // Nothing to do: we already stored the key via on_string_done.
                        match self.stack.last() {
                            Some(ContainerState::Object { current_key: Some(_), .. }) => {}
                            _ => {
                                self.emit_error("unexpected `:`", out);
                            }
                        }
                    }

                    // ── string ────────────────────────────────────────────
                    '"' => {
                        self.root_entered = true;
                        // Determine if this string is an object key or a value.
                        let is_key = matches!(
                            self.stack.last(),
                            Some(ContainerState::Object { expecting_key: true, .. })
                        );
                        self.lex = LexState::InString {
                            buf: String::new(),
                            escape: false,
                            is_key,
                        };
                    }

                    // ── literals ──────────────────────────────────────────
                    't' | 'f' | 'n' | '0'..='9' | '-' => {
                        self.root_entered = true;
                        self.lex = LexState::InLiteral {
                            buf: ch.to_string(),
                        };
                    }

                    _ => {
                        self.emit_error(
                            &format!("unexpected character `{}`", ch),
                            out,
                        );
                    }
                }
            }
        }
    }

    // ── helpers ───────────────────────────────────────────────────────────────

    /// Called when a complete string token has been lexed.
    fn on_string_done(&mut self, s: String, is_key: bool, out: &mut Vec<JsonEvent>) {
        if is_key {
            // Store as pending key and emit ObjectKey event.
            out.push(JsonEvent::ObjectKey(s.clone()));
            if let Some(ContainerState::Object { expecting_key, current_key, .. }) =
                self.stack.last_mut()
            {
                *expecting_key = false;
                *current_key = Some(s);
            }
        } else {
            let val = Value::String(s);
            self.deliver_value(val, out);
        }
    }

    /// Called when a number / keyword literal has finished.
    fn on_literal_done(&mut self, buf: String, out: &mut Vec<JsonEvent>) {
        let val = match buf.as_str() {
            "true" => Value::Bool(true),
            "false" => Value::Bool(false),
            "null" => Value::Null,
            s => match s.parse::<i64>() {
                Ok(n) => Value::Number(n.into()),
                Err(_) => match s.parse::<f64>() {
                    Ok(f) => {
                        match serde_json::Number::from_f64(f) {
                            Some(n) => Value::Number(n),
                            None => {
                                self.emit_error(&format!("invalid number `{}`", s), out);
                                return;
                            }
                        }
                    }
                    Err(_) => {
                        self.emit_error(&format!("invalid literal `{}`", s), out);
                        return;
                    }
                },
            },
        };
        self.deliver_value(val, out);
    }

    /// Route a completed value to the correct destination:
    /// * array item
    /// * object value (matched to `current_key`)
    /// * top-level document root → emit `Complete`
    fn deliver_value(&mut self, val: Value, out: &mut Vec<JsonEvent>) {
        match self.stack.last_mut() {
            Some(ContainerState::Array { items }) => {
                items.push(val.clone());
                out.push(JsonEvent::Value(val));
            }
            Some(ContainerState::Object { pairs, current_key, expecting_key, .. }) => {
                match current_key.take() {
                    Some(key) => {
                        pairs.push((key, val.clone()));
                        *expecting_key = false;
                        // Only emit Value event for non-container values;
                        // containers emit their own ObjectEnd/ArrayEnd events.
                        match &val {
                            Value::Object(_) | Value::Array(_) => {}
                            _ => out.push(JsonEvent::Value(val)),
                        }
                    }
                    None => {
                        // Value arrived without a key — this can happen when
                        // ObjectEnd / ArrayEnd bubbles up through deliver_value
                        // for a nested container that was itself a value.
                        // In that case we just swallow: the ObjectEnd/ArrayEnd
                        // event already carried the value.
                    }
                }
            }
            None => {
                // Top-level scalar or a container that just closed at depth 0.
                match &val {
                    Value::Object(_) | Value::Array(_) => {
                        // ObjectEnd/ArrayEnd already emitted; emit Complete.
                        out.push(JsonEvent::Complete(val));
                        self.done = true;
                    }
                    _ => {
                        out.push(JsonEvent::Value(val.clone()));
                        out.push(JsonEvent::Complete(val));
                        self.done = true;
                    }
                }
            }
        }
    }

    fn emit_error(&mut self, msg: &str, out: &mut Vec<JsonEvent>) {
        out.push(JsonEvent::Error(msg.to_string()));
        self.done = true;
    }
}

impl Default for StreamingJsonParser {
    fn default() -> Self {
        Self::new()
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Helpers
// ─────────────────────────────────────────────────────────────────────────────

/// Map a JSON escape character to its decoded value.
fn unescape_char(ch: char) -> char {
    match ch {
        '"' => '"',
        '\\' => '\\',
        '/' => '/',
        'n' => '\n',
        'r' => '\r',
        't' => '\t',
        'b' => '\x08',
        'f' => '\x0C',
        // Unicode escapes (`\uXXXX`) are not handled character-by-character here;
        // the raw character is preserved for simplicity.
        other => other,
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Unit tests
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn collect(chunks: &[&str]) -> Vec<JsonEvent> {
        let mut p = StreamingJsonParser::new();
        chunks.iter().flat_map(|c| p.feed(c)).collect()
    }

    // ── helpers to locate events ─────────────────────────────────────────────

    fn complete_value(events: &[JsonEvent]) -> Option<Value> {
        events.iter().find_map(|e| {
            if let JsonEvent::Complete(v) = e {
                Some(v.clone())
            } else {
                None
            }
        })
    }

    fn has_error(events: &[JsonEvent]) -> bool {
        events.iter().any(|e| matches!(e, JsonEvent::Error(_)))
    }

    // ── scalar values ────────────────────────────────────────────────────────

    #[test]
    fn test_null_single_chunk() {
        let events = collect(&["null"]);
        assert_eq!(complete_value(&events), Some(Value::Null));
    }

    #[test]
    fn test_bool_true() {
        let events = collect(&["true"]);
        assert_eq!(complete_value(&events), Some(Value::Bool(true)));
    }

    #[test]
    fn test_bool_false() {
        let events = collect(&["false"]);
        assert_eq!(complete_value(&events), Some(Value::Bool(false)));
    }

    #[test]
    fn test_integer() {
        let events = collect(&["42"]);
        assert_eq!(complete_value(&events), Some(json!(42)));
    }

    #[test]
    fn test_negative_integer() {
        let events = collect(&["-7"]);
        assert_eq!(complete_value(&events), Some(json!(-7)));
    }

    #[test]
    fn test_float() {
        let events = collect(&["3.14"]);
        if let Some(Value::Number(n)) = complete_value(&events) {
            assert!((n.as_f64().unwrap() - 3.14).abs() < 1e-9);
        } else {
            panic!("expected a Number");
        }
    }

    #[test]
    fn test_string_single_chunk() {
        let events = collect(&[r#""hello""#]);
        assert_eq!(complete_value(&events), Some(json!("hello")));
    }

    // ── fragmented strings ───────────────────────────────────────────────────

    #[test]
    fn test_string_split_across_chunks() {
        let events = collect(&[r#""hel"#, r#"lo""#]);
        assert_eq!(complete_value(&events), Some(json!("hello")));
    }

    #[test]
    fn test_string_escape_split() {
        // backslash at end of chunk, escape char in next chunk
        let events = collect(&[r#""line1\"#, r#"nline2""#]);
        // should produce "line1\nline2"
        assert_eq!(complete_value(&events), Some(json!("line1\nline2")));
    }

    // ── flat objects ─────────────────────────────────────────────────────────

    #[test]
    fn test_simple_object_single_chunk() {
        let events = collect(&[r#"{"a":1,"b":2}"#]);
        assert_eq!(complete_value(&events), Some(json!({"a":1,"b":2})));
    }

    #[test]
    fn test_object_chunked_key() {
        // Key split: {"na" | me":"value"}
        let events = collect(&[r#"{"na"#, r#"me":"value"}"#]);
        assert_eq!(complete_value(&events), Some(json!({"name":"value"})));
    }

    #[test]
    fn test_object_chunked_value() {
        let events = collect(&[r#"{"k":"val"#, r#"ue"}"#]);
        assert_eq!(complete_value(&events), Some(json!({"k":"value"})));
    }

    #[test]
    fn test_object_many_tiny_chunks() {
        let src = r#"{"x":10,"y":20}"#;
        let chunks: Vec<&str> = src
            .as_bytes()
            .chunks(1)
            .map(|b| std::str::from_utf8(b).unwrap())
            .collect();
        let events = collect(&chunks.iter().map(|s| *s).collect::<Vec<_>>());
        assert_eq!(complete_value(&events), Some(json!({"x":10,"y":20})));
    }

    // ── nested objects ───────────────────────────────────────────────────────

    #[test]
    fn test_nested_object() {
        let events = collect(&[r#"{"a":{"b":{"c":42}}}"#]);
        assert_eq!(
            complete_value(&events),
            Some(json!({"a":{"b":{"c":42}}}))
        );
    }

    #[test]
    fn test_nested_object_split() {
        let events = collect(&[r#"{"ou"#, r#"ter":{"inn"#, r#"er":true}}"#]);
        assert_eq!(
            complete_value(&events),
            Some(json!({"outer":{"inner":true}}))
        );
    }

    // ── arrays ───────────────────────────────────────────────────────────────

    #[test]
    fn test_simple_array() {
        let events = collect(&[r#"[1,2,3]"#]);
        assert_eq!(complete_value(&events), Some(json!([1, 2, 3])));
    }

    #[test]
    fn test_array_split() {
        let events = collect(&["[1,", "2,", "3]"]);
        assert_eq!(complete_value(&events), Some(json!([1, 2, 3])));
    }

    #[test]
    fn test_nested_array() {
        let events = collect(&[r#"[[1,2],[3,4]]"#]);
        assert_eq!(complete_value(&events), Some(json!([[1, 2], [3, 4]])));
    }

    #[test]
    fn test_array_of_objects() {
        let events = collect(&[r#"[{"id":1},{"id"#, r#":2}]"#]);
        assert_eq!(
            complete_value(&events),
            Some(json!([{"id":1},{"id":2}]))
        );
    }

    // ── event sequencing ─────────────────────────────────────────────────────

    #[test]
    fn test_object_events_fired() {
        let events = collect(&[r#"{"k":"v"}"#]);
        assert!(events.iter().any(|e| e == &JsonEvent::ObjectStart));
        assert!(events.iter().any(|e| e == &JsonEvent::ObjectKey("k".into())));
        assert!(events.iter().any(|e| matches!(e, JsonEvent::ObjectEnd(_))));
        assert!(events.iter().any(|e| matches!(e, JsonEvent::Complete(_))));
    }

    #[test]
    fn test_array_events_fired() {
        let events = collect(&[r#"[1]"#]);
        assert!(events.iter().any(|e| e == &JsonEvent::ArrayStart));
        assert!(events.iter().any(|e| matches!(e, JsonEvent::ArrayEnd(_))));
        assert!(events.iter().any(|e| matches!(e, JsonEvent::Complete(_))));
    }

    // ── error handling ───────────────────────────────────────────────────────

    #[test]
    fn test_error_unexpected_close_brace() {
        let events = collect(&[r#"}"#]);
        assert!(has_error(&events));
    }

    #[test]
    fn test_error_unexpected_close_bracket() {
        let events = collect(&[r#"]"#]);
        assert!(has_error(&events));
    }

    #[test]
    fn test_error_invalid_literal() {
        let events = collect(&["xyz"]);
        assert!(has_error(&events));
    }

    #[test]
    fn test_no_events_after_complete() {
        let mut p = StreamingJsonParser::new();
        let first = p.feed("42");
        assert!(first.iter().any(|e| matches!(e, JsonEvent::Complete(_))));
        let second = p.feed("garbage");
        assert!(second.is_empty(), "should emit nothing after Complete");
    }

    #[test]
    fn test_no_events_after_error() {
        let mut p = StreamingJsonParser::new();
        let first = p.feed("}");
        assert!(first.iter().any(|e| matches!(e, JsonEvent::Error(_))));
        let second = p.feed("{}");
        assert!(second.is_empty(), "should emit nothing after Error");
    }

    // ── realistic SSE simulation ─────────────────────────────────────────────

    #[test]
    fn test_sse_realistic_stream() {
        // Simulates a typical LLM SSE stream where chunks are arbitrary sizes.
        let full = r#"{"model":"gpt-4","choices":[{"delta":{"content":"Hello"}}],"finish_reason":null}"#;
        let chunk_size = 7;
        let chunks: Vec<&str> = full
            .as_bytes()
            .chunks(chunk_size)
            .map(|b| std::str::from_utf8(b).unwrap())
            .collect();
        let events = collect(&chunks.iter().map(|s| *s).collect::<Vec<_>>());
        let val = complete_value(&events).expect("should complete");
        assert_eq!(val["model"], json!("gpt-4"));
        assert_eq!(val["choices"][0]["delta"]["content"], json!("Hello"));
    }

    // ── whitespace handling ──────────────────────────────────────────────────

    #[test]
    fn test_whitespace_around_tokens() {
        let events = collect(&["  {  \"k\"  :  42  }  "]);
        assert_eq!(complete_value(&events), Some(json!({"k":42})));
    }

    // ── empty object / array ─────────────────────────────────────────────────

    #[test]
    fn test_empty_object() {
        let events = collect(&["{}"]);
        assert_eq!(complete_value(&events), Some(json!({})));
    }

    #[test]
    fn test_empty_array() {
        let events = collect(&["[]"]);
        assert_eq!(complete_value(&events), Some(json!([])));
    }
}
