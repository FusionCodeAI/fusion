//! Interactive In-TUI Multi-Line Text Editor Widget
//!
//! Provides an interactive, pure-Rust multi-line text editor with:
//! - Full cursor navigation (arrows, Home, End, PageUp, PageDown)
//! - Text editing (insertion, newline splitting, backspace/delete line joining)
//! - Syntax-conscious highlighting for major programming languages and plain text
//! - Line numbers gutter with dimmed grey styling
//! - Status bar with filename, modified flag `[+]`, line/column coordinates, and shortcut help
//! - Clean terminal lifecycle management with RAII raw-mode and alternate screen guards

use crossterm::{
    cursor,
    event::{self, Event, KeyCode, KeyEventKind, KeyModifiers},
    execute,
    terminal::{self, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{
    backend::CrosstermBackend,
    buffer::Buffer,
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::Widget,
    Terminal,
};
use std::io::{self, stdout};
use std::path::{Path, PathBuf};

// ============================================================================
// Cursor Movement
// ============================================================================

/// Directions and targets for moving the editor cursor.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CursorMove {
    /// Move cursor one row up.
    Up,
    /// Move cursor one row down.
    Down,
    /// Move cursor one character left (wrapping to previous line if at start).
    Left,
    /// Move cursor one character right (wrapping to next line if at end).
    Right,
    /// Move cursor to the beginning of the current line.
    Home,
    /// Move cursor to the end of the current line.
    End,
    /// Move cursor one page (10 lines) up.
    PageUp,
    /// Move cursor one page (10 lines) down.
    PageDown,
    /// Move cursor to the very beginning of the buffer (row 0, col 0).
    BufferStart,
    /// Move cursor to the very end of the buffer.
    BufferEnd,
}

// ============================================================================
// Editor Buffer
// ============================================================================

/// In-memory 2D text buffer tracking cursor coordinates, scroll viewports, and modifications.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EditorBuffer {
    /// Text lines stored in the buffer. Always contains at least one line.
    pub lines: Vec<String>,
    /// 0-indexed cursor row position.
    pub cursor_row: usize,
    /// 0-indexed character column position on the current row.
    pub cursor_col: usize,
    /// 0-indexed vertical scroll offset in lines.
    pub scroll_offset_row: usize,
    /// 0-indexed horizontal scroll offset in characters.
    pub scroll_offset_col: usize,
    /// True if the buffer text has been modified since loading/saving.
    pub is_modified: bool,
    /// Optional filesystem path associated with this buffer.
    pub file_path: Option<PathBuf>,
}

impl Default for EditorBuffer {
    fn default() -> Self {
        Self::new()
    }
}

impl EditorBuffer {
    /// Creates a new empty editor buffer.
    pub fn new() -> Self {
        Self::from_text("")
    }

    /// Initializes an editor buffer from raw text, normalizing line endings.
    pub fn from_text(text: &str) -> Self {
        let normalized = text.replace("\r\n", "\n").replace('\r', "\n");
        let mut lines: Vec<String> = normalized.split('\n').map(String::from).collect();
        if lines.is_empty() {
            lines.push(String::new());
        }
        Self {
            lines,
            cursor_row: 0,
            cursor_col: 0,
            scroll_offset_row: 0,
            scroll_offset_col: 0,
            is_modified: false,
            file_path: None,
        }
    }

    /// Reads and initializes an editor buffer from a file on disk.
    pub fn from_file(path: impl AsRef<Path>) -> io::Result<Self> {
        let path_ref = path.as_ref();
        let content = std::fs::read_to_string(path_ref)?;
        let mut buf = Self::from_text(&content);
        buf.file_path = Some(path_ref.to_path_buf());
        buf.is_modified = false;
        Ok(buf)
    }

    /// Exports the buffer content as a single newline-separated string.
    pub fn to_text(&self) -> String {
        self.lines.join("\n")
    }

    /// Inserts a single character at the current cursor position.
    pub fn insert_char(&mut self, c: char) {
        if c == '\n' {
            self.insert_newline();
            return;
        }

        self.ensure_valid_state();
        let line = &mut self.lines[self.cursor_row];
        let byte_offset = line
            .char_indices()
            .nth(self.cursor_col)
            .map(|(idx, _)| idx)
            .unwrap_or(line.len());

        line.insert(byte_offset, c);
        self.cursor_col += 1;
        self.is_modified = true;
    }

    /// Inserts a string slice at the current cursor position.
    pub fn insert_str(&mut self, s: &str) {
        for c in s.chars() {
            self.insert_char(c);
        }
    }

    /// Splits the current line at the cursor, creating a new line below and advancing the cursor.
    pub fn insert_newline(&mut self) {
        self.ensure_valid_state();
        let current_line = &mut self.lines[self.cursor_row];
        let byte_offset = current_line
            .char_indices()
            .nth(self.cursor_col)
            .map(|(idx, _)| idx)
            .unwrap_or(current_line.len());

        let remainder = current_line[byte_offset..].to_string();
        current_line.truncate(byte_offset);

        self.cursor_row += 1;
        self.cursor_col = 0;
        self.lines.insert(self.cursor_row, remainder);
        self.is_modified = true;
    }

    /// Deletes the character before the cursor, or joins the current line to the previous line.
    pub fn delete_backwards(&mut self) {
        self.ensure_valid_state();
        if self.cursor_col > 0 {
            let target_char = self.cursor_col - 1;
            let line = &mut self.lines[self.cursor_row];
            if let Some((byte_start, ch)) = line.char_indices().nth(target_char) {
                let byte_end = byte_start + ch.len_utf8();
                line.drain(byte_start..byte_end);
                self.cursor_col -= 1;
                self.is_modified = true;
            }
        } else if self.cursor_row > 0 {
            // Join current line to previous line
            let current_line = self.lines.remove(self.cursor_row);
            self.cursor_row -= 1;
            let prev_line = &mut self.lines[self.cursor_row];
            self.cursor_col = prev_line.chars().count();
            prev_line.push_str(&current_line);
            self.is_modified = true;
        }
    }

    /// Deletes the character at the cursor, or joins the next line to the current line.
    pub fn delete_forwards(&mut self) {
        self.ensure_valid_state();
        let cur_len = self.lines[self.cursor_row].chars().count();
        if self.cursor_col < cur_len {
            let line = &mut self.lines[self.cursor_row];
            if let Some((byte_start, ch)) = line.char_indices().nth(self.cursor_col) {
                let byte_end = byte_start + ch.len_utf8();
                line.drain(byte_start..byte_end);
                self.is_modified = true;
            }
        } else if self.cursor_row + 1 < self.lines.len() {
            let next_line = self.lines.remove(self.cursor_row + 1);
            self.lines[self.cursor_row].push_str(&next_line);
            self.is_modified = true;
        }
    }

    /// Moves the cursor according to the specified `CursorMove` direction.
    pub fn move_cursor(&mut self, direction: CursorMove) {
        self.ensure_valid_state();
        match direction {
            CursorMove::Up => {
                if self.cursor_row > 0 {
                    self.cursor_row -= 1;
                }
                self.clamp_cursor_col();
            }
            CursorMove::Down => {
                if self.cursor_row + 1 < self.lines.len() {
                    self.cursor_row += 1;
                }
                self.clamp_cursor_col();
            }
            CursorMove::Left => {
                if self.cursor_col > 0 {
                    self.cursor_col -= 1;
                } else if self.cursor_row > 0 {
                    self.cursor_row -= 1;
                    self.cursor_col = self.lines[self.cursor_row].chars().count();
                }
            }
            CursorMove::Right => {
                let cur_len = self.lines[self.cursor_row].chars().count();
                if self.cursor_col < cur_len {
                    self.cursor_col += 1;
                } else if self.cursor_row + 1 < self.lines.len() {
                    self.cursor_row += 1;
                    self.cursor_col = 0;
                }
            }
            CursorMove::Home => {
                self.move_to_line_start();
            }
            CursorMove::End => {
                self.move_to_line_end();
            }
            CursorMove::PageUp => {
                self.page_up(10);
            }
            CursorMove::PageDown => {
                self.page_down(10);
            }
            CursorMove::BufferStart => {
                self.move_to_buffer_start();
            }
            CursorMove::BufferEnd => {
                self.move_to_buffer_end();
            }
        }
    }

    /// Moves cursor to column 0 on the current line.
    pub fn move_to_line_start(&mut self) {
        self.cursor_col = 0;
    }

    /// Moves cursor to the end of the current line.
    pub fn move_to_line_end(&mut self) {
        self.ensure_valid_state();
        self.cursor_col = self.lines[self.cursor_row].chars().count();
    }

    /// Moves cursor to the very start of the buffer (row 0, col 0).
    pub fn move_to_buffer_start(&mut self) {
        self.cursor_row = 0;
        self.cursor_col = 0;
    }

    /// Moves cursor to the very end of the buffer.
    pub fn move_to_buffer_end(&mut self) {
        self.ensure_valid_state();
        self.cursor_row = self.lines.len().saturating_sub(1);
        self.cursor_col = self.lines[self.cursor_row].chars().count();
    }

    /// Pages up by the given number of rows.
    pub fn page_up(&mut self, rows: usize) {
        self.cursor_row = self.cursor_row.saturating_sub(rows);
        self.clamp_cursor_col();
    }

    /// Pages down by the given number of rows.
    pub fn page_down(&mut self, rows: usize) {
        if !self.lines.is_empty() {
            self.cursor_row = (self.cursor_row + rows).min(self.lines.len() - 1);
        }
        self.clamp_cursor_col();
    }

    /// Saves the current buffer content to a file, resetting `is_modified` on success.
    pub fn save_to_file(&mut self, path: Option<&Path>) -> io::Result<()> {
        let target_path = match path {
            Some(p) => p.to_path_buf(),
            None => match &self.file_path {
                Some(p) => p.clone(),
                None => {
                    return Err(io::Error::new(
                        io::ErrorKind::NotFound,
                        "No file path specified for saving",
                    ));
                }
            },
        };

        if let Some(parent) = target_path.parent() {
            if !parent.as_os_str().is_empty() && !parent.exists() {
                std::fs::create_dir_all(parent)?;
            }
        }

        std::fs::write(&target_path, self.to_text())?;
        self.file_path = Some(target_path);
        self.is_modified = false;
        Ok(())
    }

    /// Clamps cursor coordinates so they remain strictly within the buffer boundaries.
    pub fn clamp_cursor(&mut self) {
        self.ensure_valid_state();
        if self.cursor_row >= self.lines.len() {
            self.cursor_row = self.lines.len().saturating_sub(1);
        }
        self.clamp_cursor_col();
    }

    /// Adjusts viewport scroll offsets so that the cursor is visible inside the given viewport dimensions.
    pub fn adjust_scroll(&mut self, viewport_width: usize, viewport_height: usize) {
        self.clamp_cursor();

        if viewport_height > 0 {
            if self.cursor_row < self.scroll_offset_row {
                self.scroll_offset_row = self.cursor_row;
            } else if self.cursor_row >= self.scroll_offset_row + viewport_height {
                self.scroll_offset_row = self.cursor_row.saturating_sub(viewport_height - 1);
            }
        }

        if viewport_width > 0 {
            if self.cursor_col < self.scroll_offset_col {
                self.scroll_offset_col = self.cursor_col;
            } else if self.cursor_col >= self.scroll_offset_col + viewport_width {
                self.scroll_offset_col = self.cursor_col.saturating_sub(viewport_width - 1);
            }
        }
    }

    /// Returns the number of lines in the buffer.
    pub fn line_count(&self) -> usize {
        self.lines.len()
    }

    /// Returns the contents of the line currently under the cursor.
    pub fn current_line(&self) -> &str {
        if self.cursor_row < self.lines.len() {
            &self.lines[self.cursor_row]
        } else {
            ""
        }
    }

    /// Returns true if the buffer has no text (single empty line).
    pub fn is_empty(&self) -> bool {
        self.lines.len() <= 1 && self.lines.first().map_or(true, |l| l.is_empty())
    }

    fn clamp_cursor_col(&mut self) {
        if self.cursor_row < self.lines.len() {
            let max_col = self.lines[self.cursor_row].chars().count();
            if self.cursor_col > max_col {
                self.cursor_col = max_col;
            }
        } else {
            self.cursor_col = 0;
        }
    }

    fn ensure_valid_state(&mut self) {
        if self.lines.is_empty() {
            self.lines.push(String::new());
        }
        if self.cursor_row >= self.lines.len() {
            self.cursor_row = self.lines.len() - 1;
        }
    }
}

// ============================================================================
// Syntax Tokenizer & Highlighting
// ============================================================================

/// Supported language categories for syntax highlighting.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Language {
    Rust,
    Python,
    JavaScript,
    TypeScript,
    Json,
    Toml,
    Markdown,
    Shell,
    C,
    Cpp,
    Go,
    Plain,
}

/// Detects programming language from a file path extension.
pub fn detect_language(path: Option<&Path>) -> Language {
    let ext = path
        .and_then(|p| p.extension())
        .and_then(|e| e.to_str())
        .map(|s| s.to_ascii_lowercase())
        .unwrap_or_default();

    match ext.as_str() {
        "rs" => Language::Rust,
        "py" | "pyw" => Language::Python,
        "js" | "jsx" | "mjs" | "cjs" => Language::JavaScript,
        "ts" | "tsx" | "mts" | "cts" => Language::TypeScript,
        "json" => Language::Json,
        "toml" => Language::Toml,
        "md" | "markdown" => Language::Markdown,
        "sh" | "bash" | "zsh" => Language::Shell,
        "c" | "h" => Language::C,
        "cpp" | "cc" | "cxx" | "hpp" => Language::Cpp,
        "go" => Language::Go,
        _ => Language::Plain,
    }
}

/// Syntax token classification.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TokenKind {
    Keyword,
    Type,
    Function,
    StringLiteral,
    NumberLiteral,
    Comment,
    Attribute,
    Operator,
    Punctuation,
    Plain,
}

/// A token slice with its semantic category.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SyntaxToken<'a> {
    pub kind: TokenKind,
    pub text: &'a str,
}

/// Tokenizes a single line of text according to language rules.
pub fn tokenize_line<'a>(line: &'a str, lang: Language) -> Vec<SyntaxToken<'a>> {
    if line.is_empty() {
        return Vec::new();
    }

    if lang == Language::Plain {
        return vec![SyntaxToken {
            kind: TokenKind::Plain,
            text: line,
        }];
    }

    let bytes = line.as_bytes();
    let len = bytes.len();
    let mut tokens = Vec::new();
    let mut i = 0;

    while i < len {
        // Comments
        if matches!(
            lang,
            Language::Rust | Language::C | Language::Cpp | Language::JavaScript | Language::TypeScript | Language::Go
        ) && i + 1 < len && bytes[i] == b'/' && bytes[i + 1] == b'/'
        {
            tokens.push(SyntaxToken {
                kind: TokenKind::Comment,
                text: &line[i..],
            });
            break;
        }

        if matches!(lang, Language::Python | Language::Shell | Language::Toml) && bytes[i] == b'#' {
            tokens.push(SyntaxToken {
                kind: TokenKind::Comment,
                text: &line[i..],
            });
            break;
        }

        // Strings
        if bytes[i] == b'"' || bytes[i] == b'\'' || bytes[i] == b'`' {
            let quote = bytes[i];
            let start = i;
            i += 1;
            while i < len {
                if bytes[i] == b'\\' && i + 1 < len {
                    i += 2;
                } else if bytes[i] == quote {
                    i += 1;
                    break;
                } else {
                    i += 1;
                }
            }
            tokens.push(SyntaxToken {
                kind: TokenKind::StringLiteral,
                text: &line[start..i],
            });
            continue;
        }

        // Numbers
        if bytes[i].is_ascii_digit()
            || (bytes[i] == b'.' && i + 1 < len && bytes[i + 1].is_ascii_digit())
        {
            let start = i;
            if i + 1 < len && bytes[i] == b'0' && (bytes[i + 1] == b'x' || bytes[i + 1] == b'b' || bytes[i + 1] == b'o') {
                i += 2;
                while i < len && (bytes[i].is_ascii_hexdigit() || bytes[i] == b'_') {
                    i += 1;
                }
            } else {
                while i < len
                    && (bytes[i].is_ascii_alphanumeric()
                        || bytes[i] == b'.'
                        || bytes[i] == b'_')
                {
                    i += 1;
                }
            }
            tokens.push(SyntaxToken {
                kind: TokenKind::NumberLiteral,
                text: &line[start..i],
            });
            continue;
        }

        // Attributes (e.g. #[derive(...)] in Rust or @decorator in Python/TS)
        if (lang == Language::Rust && bytes[i] == b'#' && i + 1 < len && bytes[i + 1] == b'[')
            || (matches!(lang, Language::Python | Language::TypeScript) && bytes[i] == b'@')
        {
            let start = i;
            i += 1;
            while i < len && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_' || bytes[i] == b'[') {
                i += 1;
            }
            tokens.push(SyntaxToken {
                kind: TokenKind::Attribute,
                text: &line[start..i],
            });
            continue;
        }

        // Identifiers / Keywords / Types / Functions
        if bytes[i].is_ascii_alphabetic() || bytes[i] == b'_' {
            let start = i;
            while i < len && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_') {
                i += 1;
            }
            let word = &line[start..i];

            let is_kw = is_keyword(word, lang);
            let kind = if is_kw {
                TokenKind::Keyword
            } else {
                // Check if followed by '(' -> function call
                let mut peek = i;
                while peek < len && bytes[peek].is_ascii_whitespace() {
                    peek += 1;
                }
                if peek < len && bytes[peek] == b'(' {
                    TokenKind::Function
                } else if is_type_name(word) {
                    TokenKind::Type
                } else {
                    TokenKind::Plain
                }
            };

            tokens.push(SyntaxToken { kind, text: word });
            continue;
        }

        // Operators & Punctuation
        let start = i;
        if b"+-*/%=&|^!<>?:.~".contains(&bytes[i]) {
            while i < len && b"+-*/%=&|^!<>?:.~".contains(&bytes[i]) {
                i += 1;
            }
            tokens.push(SyntaxToken {
                kind: TokenKind::Operator,
                text: &line[start..i],
            });
        } else if b"()[]{},;".contains(&bytes[i]) {
            tokens.push(SyntaxToken {
                kind: TokenKind::Punctuation,
                text: &line[start..start + 1],
            });
            i += 1;
        } else {
            // Whitespace or unhandled byte
            while i < len
                && !bytes[i].is_ascii_alphanumeric()
                && bytes[i] != b'_'
                && bytes[i] != b'"'
                && bytes[i] != b'\''
                && bytes[i] != b'`'
                && bytes[i] != b'/'
                && bytes[i] != b'#'
                && !b"+-*/%=&|^!<>?:.~()[]{},;".contains(&bytes[i])
            {
                i += 1;
            }
            tokens.push(SyntaxToken {
                kind: TokenKind::Plain,
                text: &line[start..i],
            });
        }
    }

    tokens
}

fn is_keyword(word: &str, lang: Language) -> bool {
    match lang {
        Language::Rust => matches!(
            word,
            "as" | "async" | "await" | "break" | "const" | "continue" | "crate" | "dyn"
                | "else" | "enum" | "extern" | "false" | "fn" | "for" | "if" | "impl"
                | "in" | "let" | "loop" | "match" | "mod" | "move" | "mut" | "pub"
                | "ref" | "return" | "self" | "Self" | "static" | "struct" | "super"
                | "trait" | "true" | "type" | "unsafe" | "use" | "where" | "while"
        ),
        Language::Python => matches!(
            word,
            "and" | "as" | "assert" | "async" | "await" | "break" | "class" | "continue"
                | "def" | "del" | "elif" | "else" | "except" | "False" | "finally" | "for"
                | "from" | "global" | "if" | "import" | "in" | "is" | "lambda" | "None"
                | "nonlocal" | "not" | "or" | "pass" | "raise" | "return" | "True" | "try"
                | "while" | "with" | "yield" | "self"
        ),
        Language::JavaScript | Language::TypeScript => matches!(
            word,
            "async" | "await" | "break" | "case" | "catch" | "class" | "const" | "continue"
                | "debugger" | "default" | "delete" | "do" | "else" | "export" | "extends"
                | "false" | "finally" | "for" | "from" | "function" | "if" | "import"
                | "in" | "instanceof" | "let" | "new" | "null" | "return" | "super"
                | "switch" | "this" | "throw" | "true" | "try" | "typeof" | "var"
                | "void" | "while" | "with" | "yield" | "type" | "interface"
        ),
        Language::Go => matches!(
            word,
            "break" | "case" | "chan" | "const" | "continue" | "default" | "defer"
                | "else" | "fallthrough" | "for" | "func" | "go" | "goto" | "if"
                | "import" | "interface" | "map" | "package" | "range" | "return"
                | "select" | "struct" | "switch" | "type" | "var" | "true" | "false" | "nil"
        ),
        Language::C | Language::Cpp => matches!(
            word,
            "auto" | "break" | "case" | "char" | "const" | "continue" | "default"
                | "do" | "double" | "else" | "enum" | "extern" | "float" | "for"
                | "goto" | "if" | "int" | "long" | "register" | "return" | "short"
                | "signed" | "sizeof" | "static" | "struct" | "switch" | "typedef"
                | "union" | "unsigned" | "void" | "volatile" | "while" | "class"
                | "public" | "private" | "protected" | "virtual" | "override" | "namespace"
        ),
        Language::Shell => matches!(
            word,
            "if" | "then" | "else" | "elif" | "fi" | "case" | "esac" | "for"
                | "select" | "while" | "until" | "do" | "done" | "in" | "function"
                | "time" | "return" | "exit"
        ),
        _ => matches!(
            word,
            "fn" | "let" | "def" | "if" | "else" | "for" | "while" | "return"
                | "function" | "class" | "import" | "export" | "true" | "false"
        ),
    }
}

fn is_type_name(word: &str) -> bool {
    if word.is_empty() {
        return false;
    }
    // Primitive types
    matches!(
        word,
        "bool"
            | "char"
            | "str"
            | "u8"
            | "u16"
            | "u32"
            | "u64"
            | "u128"
            | "usize"
            | "i8"
            | "i16"
            | "i32"
            | "i64"
            | "i128"
            | "isize"
            | "f32"
            | "f64"
            | "String"
            | "Vec"
            | "Option"
            | "Result"
            | "Box"
            | "Rc"
            | "Arc"
            | "int"
            | "float"
            | "boolean"
            | "number"
            | "string"
            | "any"
            | "void"
    ) || (word.starts_with(|c: char| c.is_ascii_uppercase()) && word.chars().any(|c| c.is_ascii_lowercase()))
}

fn tokens_to_spans<'a>(tokens: &[SyntaxToken<'a>]) -> Vec<Span<'a>> {
    tokens
        .iter()
        .map(|tok| {
            let style = match tok.kind {
                TokenKind::Keyword => Style::default()
                    .fg(Color::Magenta)
                    .add_modifier(Modifier::BOLD),
                TokenKind::Type => Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
                TokenKind::Function => Style::default().fg(Color::Green),
                TokenKind::StringLiteral => Style::default().fg(Color::Yellow),
                TokenKind::NumberLiteral => Style::default().fg(Color::LightYellow),
                TokenKind::Comment => Style::default()
                    .fg(Color::DarkGray)
                    .add_modifier(Modifier::ITALIC),
                TokenKind::Attribute => Style::default().fg(Color::Magenta),
                TokenKind::Operator => Style::default().fg(Color::LightBlue),
                TokenKind::Punctuation => Style::default().fg(Color::Gray),
                TokenKind::Plain => Style::default().fg(Color::White),
            };
            Span::styled(tok.text, style)
        })
        .collect()
}

fn slice_spans<'a>(spans: Vec<Span<'a>>, skip: usize, take: usize) -> Vec<Span<'a>> {
    if take == 0 {
        return Vec::new();
    }
    let mut result = Vec::new();
    let mut current_skip = skip;
    let mut remaining_take = take;

    for span in spans {
        if remaining_take == 0 {
            break;
        }

        let char_count = span.content.chars().count();
        if current_skip >= char_count {
            current_skip -= char_count;
            continue;
        }

        let start_char = current_skip;
        current_skip = 0;

        let chars_to_take = (char_count - start_char).min(remaining_take);
        remaining_take -= chars_to_take;

        let byte_start = span
            .content
            .char_indices()
            .nth(start_char)
            .map(|(i, _)| i)
            .unwrap_or(0);
        let byte_end = span
            .content
            .char_indices()
            .nth(start_char + chars_to_take)
            .map(|(i, _)| i)
            .unwrap_or(span.content.len());

        let sub_slice = &span.content[byte_start..byte_end];
        result.push(Span::styled(sub_slice.to_string(), span.style));
    }

    result
}

// ============================================================================
// Editor Widget
// ============================================================================

/// Ratatui widget rendering the text editor, line numbers gutter, and status bar.
pub struct EditorWidget<'a> {
    pub buffer: &'a EditorBuffer,
    pub show_line_numbers: bool,
    pub show_status_bar: bool,
    pub status_message: Option<String>,
}

impl<'a> EditorWidget<'a> {
    /// Creates a new editor widget bound to the given buffer.
    pub fn new(buffer: &'a EditorBuffer) -> Self {
        Self {
            buffer,
            show_line_numbers: true,
            show_status_bar: true,
            status_message: None,
        }
    }

    /// Toggles the line numbers gutter.
    pub fn show_line_numbers(mut self, show: bool) -> Self {
        self.show_line_numbers = show;
        self
    }

    /// Toggles the bottom status bar.
    pub fn show_status_bar(mut self, show: bool) -> Self {
        self.show_status_bar = show;
        self
    }

    /// Attaches an optional status message displayed in the status bar.
    pub fn with_status_message(mut self, msg: impl Into<String>) -> Self {
        self.status_message = Some(msg.into());
        self
    }

    fn render_editor_content(&self, area: Rect, buf: &mut Buffer) {
        if area.width == 0 || area.height == 0 {
            return;
        }

        let total_lines = self.buffer.lines.len();
        let gutter_digits = total_lines.to_string().len().max(3);
        let gutter_width = if self.show_line_numbers {
            gutter_digits + 3 // e.g. "  1 │ "
        } else {
            0
        };

        let content_width = area.width.saturating_sub(gutter_width as u16) as usize;
        let visible_rows = area.height as usize;
        let lang = detect_language(self.buffer.file_path.as_deref());

        for row_idx in 0..visible_rows {
            let y = area.y + row_idx as u16;
            let line_idx = self.buffer.scroll_offset_row + row_idx;

            let mut line_spans = Vec::new();

            // Gutter rendering
            if self.show_line_numbers {
                let gutter_style = Style::default().fg(Color::DarkGray);
                if line_idx < total_lines {
                    let num_str = format!("{:>width$} │ ", line_idx + 1, width = gutter_digits);
                    line_spans.push(Span::styled(num_str, gutter_style));
                } else {
                    let num_str = format!("{:>width$} │ ", "~", width = gutter_digits);
                    line_spans.push(Span::styled(num_str, gutter_style));
                }
            }

            // Text rendering
            if line_idx < total_lines {
                let raw_line = &self.buffer.lines[line_idx];
                let tokens = tokenize_line(raw_line, lang);
                let styled_spans = tokens_to_spans(&tokens);
                let sliced = slice_spans(styled_spans, self.buffer.scroll_offset_col, content_width);
                line_spans.extend(sliced);
            }

            let line = Line::from(line_spans);
            buf.set_line(area.x, y, &line, area.width);
        }
    }

    fn render_status_bar(&self, area: Rect, buf: &mut Buffer) {
        if area.width == 0 || area.height == 0 {
            return;
        }

        let status_bg = Color::Rgb(36, 40, 48);
        let status_style = Style::default().bg(status_bg).fg(Color::White);

        // Fill background
        for x in area.left()..area.right() {
            if let Some(cell) = buf.cell_mut((x, area.y)) {
                cell.set_char(' ');
                cell.set_style(status_style);
            }
        }

        let file_display = self
            .buffer
            .file_path
            .as_ref()
            .and_then(|p| p.file_name())
            .and_then(|f| f.to_str())
            .unwrap_or(
                self.buffer
                    .file_path
                    .as_ref()
                    .and_then(|p| p.to_str())
                    .unwrap_or("[No Name]"),
            );

        let mut left_spans = Vec::new();
        left_spans.push(Span::styled(
            format!(" {}", file_display),
            Style::default()
                .bg(status_bg)
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ));

        if self.buffer.is_modified {
            left_spans.push(Span::styled(
                " [+] ",
                Style::default()
                    .bg(status_bg)
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            ));
        } else {
            left_spans.push(Span::styled(" ", Style::default().bg(status_bg)));
        }

        if let Some(msg) = &self.status_message {
            left_spans.push(Span::styled(
                format!("| {} ", msg),
                Style::default().bg(status_bg).fg(Color::Green),
            ));
        }

        let right_text = format!(
            "Ln {}, Col {} | Ctrl+S: Save | Esc: Exit ",
            self.buffer.cursor_row + 1,
            self.buffer.cursor_col + 1
        );
        let right_spans = vec![Span::styled(
            right_text,
            Style::default().bg(status_bg).fg(Color::White),
        )];

        let left_line = Line::from(left_spans);
        let right_line = Line::from(right_spans);

        let left_width = left_line.width() as u16;
        let right_width = right_line.width() as u16;

        if area.width >= left_width + right_width {
            buf.set_line(area.x, area.y, &left_line, left_width);
            buf.set_line(
                area.x + area.width - right_width,
                area.y,
                &right_line,
                right_width,
            );
        } else if area.width >= left_width {
            buf.set_line(area.x, area.y, &left_line, left_width);
        } else {
            buf.set_line(area.x, area.y, &left_line, area.width);
        }
    }
}

impl<'a> Widget for EditorWidget<'a> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        (&self).render(area, buf);
    }
}

impl<'a> Widget for &EditorWidget<'a> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        if area.width == 0 || area.height == 0 {
            return;
        }

        let (editor_area, status_area) = if self.show_status_bar && area.height >= 2 {
            (
                Rect {
                    x: area.x,
                    y: area.y,
                    width: area.width,
                    height: area.height - 1,
                },
                Some(Rect {
                    x: area.x,
                    y: area.y + area.height - 1,
                    width: area.width,
                    height: 1,
                }),
            )
        } else {
            (area, None)
        };

        self.render_editor_content(editor_area, buf);

        if let Some(status_rect) = status_area {
            self.render_status_bar(status_rect, buf);
        }
    }
}

// ============================================================================
// Interactive Terminal Loop
// ============================================================================

/// RAII guard ensuring terminal attributes are restored even on panic or early return.
struct TerminalRestoreGuard;

impl Drop for TerminalRestoreGuard {
    fn drop(&mut self) {
        let _ = terminal::disable_raw_mode();
        let _ = execute!(stdout(), LeaveAlternateScreen, cursor::Show);
    }
}

/// Runs an interactive in-terminal text editor session in raw mode and alternate screen.
///
/// Handles keypresses:
/// - Typing printable characters
/// - Backspace / Delete
/// - Enter (newline insertion)
/// - Navigation arrows, Home, End, PageUp, PageDown
/// - `Ctrl+S` to save (to file if path is known or in memory)
/// - `Esc` to exit
///
/// Returns `Ok(Some(text))` if saved or confirmed, or `Ok(None)` if cancelled without saving.
pub fn edit_text_interactive(
    initial_text: &str,
    file_path: Option<&Path>,
) -> io::Result<Option<String>> {
    let mut buffer = EditorBuffer::from_text(initial_text);
    if let Some(p) = file_path {
        buffer.file_path = Some(p.to_path_buf());
    }

    let _guard = TerminalRestoreGuard;
    terminal::enable_raw_mode()?;
    let mut out = stdout();
    execute!(out, EnterAlternateScreen, cursor::Show)?;

    let backend = CrosstermBackend::new(out);
    let mut terminal = Terminal::new(backend)?;

    let mut status_msg: Option<String> = None;
    let mut saved = false;

    loop {
        terminal.draw(|f| {
            let area = f.area();
            let total_lines = buffer.lines.len();
            let gutter_digits = total_lines.to_string().len().max(3);
            let gutter_w = gutter_digits + 3;
            let viewport_w = (area.width.saturating_sub(gutter_w as u16)) as usize;
            let viewport_h = (area.height.saturating_sub(1)) as usize;

            buffer.adjust_scroll(viewport_w, viewport_h);

            let mut widget = EditorWidget::new(&buffer);
            if let Some(msg) = &status_msg {
                widget = widget.with_status_message(msg);
            }
            f.render_widget(widget, area);

            // Position hardware cursor
            let screen_row = buffer.cursor_row as isize - buffer.scroll_offset_row as isize;
            let screen_col = buffer.cursor_col as isize - buffer.scroll_offset_col as isize;
            if screen_row >= 0
                && screen_row < viewport_h as isize
                && screen_col >= 0
                && screen_col < viewport_w as isize
            {
                let cursor_x = area.x + gutter_w as u16 + screen_col as u16;
                let cursor_y = area.y + screen_row as u16;
                f.set_cursor_position((cursor_x, cursor_y));
            }
        })?;

        if let Event::Key(key) = event::read()? {
            if key.kind != KeyEventKind::Press {
                continue;
            }

            let prev_msg_was_save = status_msg.is_some();

            match (key.modifiers, key.code) {
                // Ctrl+S: Save
                (KeyModifiers::CONTROL, KeyCode::Char('s')) => {
                    if buffer.file_path.is_some() {
                        match buffer.save_to_file(None) {
                            Ok(()) => {
                                status_msg = Some("Saved to file".to_string());
                                saved = true;
                            }
                            Err(e) => {
                                status_msg = Some(format!("Error saving: {}", e));
                            }
                        }
                    } else {
                        buffer.is_modified = false;
                        saved = true;
                        status_msg = Some("Saved (in memory)".to_string());
                    }
                    continue;
                }

                // Esc: Exit
                (_, KeyCode::Esc) => {
                    break;
                }

                // Ctrl+C / Ctrl+Q: Exit
                (KeyModifiers::CONTROL, KeyCode::Char('c'))
                | (KeyModifiers::CONTROL, KeyCode::Char('q')) => {
                    break;
                }

                // Navigation
                (_, KeyCode::Up) => {
                    buffer.move_cursor(CursorMove::Up);
                }
                (_, KeyCode::Down) => {
                    buffer.move_cursor(CursorMove::Down);
                }
                (_, KeyCode::Left) => {
                    buffer.move_cursor(CursorMove::Left);
                }
                (_, KeyCode::Right) => {
                    buffer.move_cursor(CursorMove::Right);
                }
                (_, KeyCode::Home) | (KeyModifiers::CONTROL, KeyCode::Char('a')) => {
                    buffer.move_to_line_start();
                }
                (_, KeyCode::End) | (KeyModifiers::CONTROL, KeyCode::Char('e')) => {
                    buffer.move_to_line_end();
                }
                (_, KeyCode::PageUp) => {
                    buffer.move_cursor(CursorMove::PageUp);
                }
                (_, KeyCode::PageDown) => {
                    buffer.move_cursor(CursorMove::PageDown);
                }

                // Editing
                (_, KeyCode::Enter) => {
                    buffer.insert_newline();
                }
                (_, KeyCode::Backspace) => {
                    buffer.delete_backwards();
                }
                (_, KeyCode::Delete) => {
                    buffer.delete_forwards();
                }
                (_, KeyCode::Tab) => {
                    buffer.insert_str("    ");
                }
                (modifiers, KeyCode::Char(c))
                    if modifiers.is_empty() || modifiers == KeyModifiers::SHIFT =>
                {
                    buffer.insert_char(c);
                }

                _ => {}
            }

            if prev_msg_was_save {
                status_msg = None;
            }
        }
    }

    // Explicit teardown
    let _ = terminal::disable_raw_mode();
    let _ = execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        cursor::Show
    );

    if saved || !buffer.is_modified {
        Ok(Some(buffer.to_text()))
    } else {
        Ok(None)
    }
}
