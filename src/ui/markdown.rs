use std::io::{stdout, Write};

use super::spinner::SpinnerStyle;

/// Fixed width (in characters) of rendered code-block borders and horizontal rules.
const BORDER_WIDTH: usize = 60;

/// Streaming markdown renderer for the terminal.
/// Supports ANSI-formatted headers, bullet/numbered lists, task lists, bold, italic,
/// inline code, blockquotes, horizontal rules, and bordered code blocks with language tags.
///
/// Chunks fed via [`push`](Self::push) may split lines, ANSI tokens, or table rows at
/// arbitrary byte boundaries; the renderer buffers the trailing partial line until a
/// newline (or [`finish`](Self::finish)) arrives.
#[derive(Debug, Clone)]
pub struct MarkdownRenderer {
    /// Partial line still awaiting its newline.
    buffer: String,
    in_code_block: bool,
    code_lang: String,
    stream_stdout: bool,
    table_streamer: super::table::MarkdownTableStreamer,
    /// Optional inline spinner shown while the model is streaming text.
    spinner: Option<StreamSpinner>,
    /// Frames consumed so far; advanced each time a spinner frame is printed.
    spinner_frame_idx: usize,
    indent: usize,
    mermaid_buffer: Vec<String>,
    line_has_prefix: bool,
}
/// Inline spinner rendered at the start of the current streaming line.
/// Mirrors the visual language of `super::spinner` without spawning a task.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StreamSpinner {
    style: SpinnerStyle,
    message: String,
}

impl StreamSpinner {
    /// Create an inline spinner with a Braille animation and the given message.
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            style: SpinnerStyle::Braille,
            message: message.into(),
        }
    }

    /// Set the animation style.
    pub fn with_style(mut self, style: SpinnerStyle) -> Self {
        self.style = style;
        self
    }

    /// Returns the visible width of this spinner's rendered form.
    fn width(&self) -> usize {
        // frame + space + message
        1 + 1 + self.message.chars().count()
    }
}

impl Default for StreamSpinner {
    fn default() -> Self {
        Self::new("")
    }
}

impl MarkdownRenderer {
    /// Create a new MarkdownRenderer that streams formatted output to stdout.
    pub fn new() -> Self {
        Self {
            buffer: String::new(),
            in_code_block: false,
            code_lang: String::new(),
            stream_stdout: true,
            table_streamer: super::table::MarkdownTableStreamer::new(),
            spinner: None,
            spinner_frame_idx: 0,
            indent: 0,
            mermaid_buffer: Vec::new(),
            line_has_prefix: false,
        }
    }

    /// Create a renderer that buffers rendered output in memory (for testing or string output).
    pub fn buffered() -> Self {
        Self {
            buffer: String::new(),
            in_code_block: false,
            code_lang: String::new(),
            stream_stdout: false,
            table_streamer: super::table::MarkdownTableStreamer::new(),
            spinner: None,
            spinner_frame_idx: 0,
            indent: 0,
            mermaid_buffer: Vec::new(),
            line_has_prefix: false,
        }
    }

    /// Configure indentation (number of leading spaces) for each emitted line.
    pub fn with_indent(mut self, indent: usize) -> Self {
        self.indent = indent;
        self
    }

    /// Returns whether the renderer is currently inside a fenced code block.
    pub fn is_in_code_block(&self) -> bool {
        self.in_code_block
    }

    /// Returns whether the current streaming line has already had its indentation/prefix emitted.
    pub fn line_has_prefix(&self) -> bool {
        self.line_has_prefix
    }

    /// Returns the language identifier of the active code block, if any.
    pub fn code_lang(&self) -> &str {
        &self.code_lang
    }

    /// Returns the buffered (not yet newline-terminated) text.
    pub fn pending(&self) -> &str {
        &self.buffer
    }

    /// Set (or replace) the inline spinner at runtime.
    pub fn set_spinner(&mut self, spinner: Option<StreamSpinner>) {
        if spinner.is_none() {
            self.spinner_frame_idx = 0;
        }
        self.spinner = spinner;
    }

    /// Advance the spinner animation by one frame.
    pub fn tick_spinner(&mut self) {
        if self.spinner.is_some() {
            self.spinner_frame_idx = self.spinner_frame_idx.wrapping_add(1);
        }
    }

    /// Reset the internal state of the renderer.
    pub fn reset(&mut self) {
        self.buffer.clear();
        self.in_code_block = false;
        self.code_lang.clear();
        self.mermaid_buffer.clear();
        self.table_streamer = super::table::MarkdownTableStreamer::new();
        self.spinner = None;
        self.spinner_frame_idx = 0;
        self.line_has_prefix = false;
    }
    /// Emit a fully rendered line: into the output buffer and, when streaming,
    /// straight to stdout with an inline flush.
    fn emit(&mut self, formatted: &str, output: &mut String) {
        if self.indent > 0 {
            let indent_prefix = " ".repeat(self.indent);
            for line in formatted.split('\n') {
                if !line.is_empty() {
                    output.push_str(&indent_prefix);
                    output.push_str(line);
                    if self.stream_stdout {
                        print!("{}{}\r\n", indent_prefix, line);
                    }
                } else if self.stream_stdout {
                    print!("\r\n");
                }
                output.push('\n');
            }
            if self.stream_stdout {
                let _ = stdout().flush();
            }
        } else {
            for line in formatted.split('\n') {
                output.push_str(line);
                output.push('\n');
                if self.stream_stdout {
                    print!("{}\r\n", line);
                }
            }
            if self.stream_stdout {
                let _ = stdout().flush();
            }
        }
    }

    /// Render one buffered line, dispatching tables to the table streamer.
    fn render_buffered_line(&mut self, line: &str, output: &mut String) {
        let trimmed = line.trim();
        if !self.in_code_block && super::table::is_markdown_table_line(trimmed) {
            self.table_streamer.feed_line(line);
            return;
        }
        if self.table_streamer.is_buffering() {
            let table_out = self.table_streamer.flush();
            self.emit(&table_out, output);
        }

        // Handle buffering and rendering of Mermaid diagram code blocks
        if self.in_code_block && self.code_lang.eq_ignore_ascii_case("mermaid") {
            if trimmed.starts_with("```") {
                let mermaid_src = self.mermaid_buffer.join("\n");
                self.mermaid_buffer.clear();
                self.in_code_block = false;
                self.code_lang.clear();

                if let Some(ascii_art) = super::mermaid_ascii::render_mermaid_ascii(&mermaid_src) {
                    self.emit(ascii_art.trim_end(), output);
                } else {
                    for l in mermaid_src.lines() {
                        self.emit(l, output);
                    }
                }
                let end_border = format!("  \x1b[38;5;240m─{}\x1b[0m", "─".repeat(BORDER_WIDTH.saturating_sub(1)));
                self.emit(&end_border, output);
                return;
            } else {
                self.mermaid_buffer.push(line.to_string());
                return;
            }
        }

        let formatted = render_line(line, &mut self.in_code_block, &mut self.code_lang);
        self.emit(&formatted, output);
    }
    /// Feed a streaming token or text chunk into the renderer.
    /// Returns any newly formatted output produced.
    pub fn push(&mut self, chunk: &str) -> String {
        // Normalize CRLF / lone CR to LF so Windows and odd chunk splits do not
        // leak carriage returns into rendered output.
        let normalized: String;
        let chunk = if chunk.contains('\r') {
            normalized = chunk.replace("\r\n", "\n").replace('\r', "\n");
            &normalized
        } else {
            chunk
        };

        self.buffer.push_str(chunk);

        // First real content clears the pending inline spinner (it only shows
        // while nothing has streamed yet).
        if !self.buffer.trim().is_empty() {
            self.spinner = None;
        }

        let mut output = String::new();

        if self.stream_stdout {
            while let Some(newline_pos) = self.buffer.find('\n') {
                let line_content: String = self.buffer.drain(..=newline_pos).collect();
                let line = line_content.trim_end_matches('\n');

                if self.in_code_block {
                    self.render_buffered_line(line, &mut output);
                    self.line_has_prefix = false;
                } else {
                    let is_special = is_special_block_prefix(line) || self.table_streamer.is_buffering();

                    if is_special {
                        self.render_buffered_line(line, &mut output);
                        self.line_has_prefix = false;
                    } else {
                        if !self.line_has_prefix {
                            if self.indent > 0 {
                                print!("{}{}\r\n", " ".repeat(self.indent), line);
                            } else {
                                print!("{}\r\n", line);
                            }
                        } else {
                            print!("{}\r\n", line);
                        }
                        let _ = stdout().flush();
                        self.line_has_prefix = false;
                    }
                }
            }

            // Progressive word streaming for normal paragraph text outside code blocks:
            if !self.in_code_block && !self.buffer.is_empty() {
                let is_special = is_special_block_prefix(&self.buffer) || self.table_streamer.is_buffering();
                if !is_special {
                    if let Some(last_space_idx) = self.buffer.rfind(' ') {
                        let to_print: String = self.buffer.drain(..=last_space_idx).collect();
                        if !self.line_has_prefix {
                            if self.indent > 0 {
                                print!("{}{}", " ".repeat(self.indent), to_print);
                            } else {
                                print!("{}", to_print);
                            }
                            self.line_has_prefix = true;
                        } else {
                            print!("{}", to_print);
                        }
                        let _ = stdout().flush();
                    }
                }
            }
        } else {
            while let Some(newline_pos) = self.buffer.find('\n') {
                // Take the complete line including the newline in one memmove.
                let line: String = self.buffer.drain(..=newline_pos).collect();
                let line = line.trim_end_matches('\n');
                self.render_buffered_line(line, &mut output);
            }
        }

        output
    }

    /// Finalize stream, rendering any remaining buffered text and closing open blocks.
    pub fn finish(&mut self) -> String {
        let mut output = String::new();

        if !self.buffer.is_empty() {
            if self.stream_stdout {
                if self.in_code_block {
                    let line = std::mem::take(&mut self.buffer);
                    self.render_buffered_line(&line, &mut output);
                } else {
                    let is_special = is_special_block_prefix(&self.buffer) || self.table_streamer.is_buffering();
                    if is_special {
                        let line = std::mem::take(&mut self.buffer);
                        self.render_buffered_line(&line, &mut output);
                    } else {
                        let line = std::mem::take(&mut self.buffer);
                        if !self.line_has_prefix {
                            if self.indent > 0 {
                                print!("{}{}\r\n", " ".repeat(self.indent), line);
                            } else {
                                print!("{}\r\n", line);
                            }
                        } else {
                            print!("{}\r\n", line);
                        }
                        let _ = stdout().flush();
                    }
                }
                self.line_has_prefix = false;
            } else {
                let line = std::mem::take(&mut self.buffer);
                self.render_buffered_line(&line, &mut output);
            }
        }

        if self.table_streamer.is_buffering() {
            let table_out = self.table_streamer.flush();
            self.emit(&table_out, &mut output);
        }
        if self.in_code_block && self.code_lang.eq_ignore_ascii_case("mermaid") {
            let mermaid_src = self.mermaid_buffer.join("\n");
            self.mermaid_buffer.clear();
            if let Some(ascii_art) = super::mermaid_ascii::render_mermaid_ascii(&mermaid_src) {
                self.emit(ascii_art.trim_end(), &mut output);
            } else {
                for l in mermaid_src.lines() {
                    self.emit(l, &mut output);
                }
            }
        }

        if self.in_code_block {
            self.in_code_block = false;
            self.code_lang.clear();
            let end_border = format!("  \x1b[38;5;240m─{}\x1b[0m", "─".repeat(BORDER_WIDTH.saturating_sub(1)));
            self.emit(&end_border, &mut output);
        }

        self.line_has_prefix = false;

        output
    }
}

/// Helper to detect if a line starts with special block markdown syntax that should
/// not be progressively word-streamed without full line formatting.
fn is_special_block_prefix(text: &str) -> bool {
    let trimmed = text.trim_start();
    trimmed.starts_with('#')
        || trimmed.starts_with("```")
        || trimmed.starts_with('|')
        || trimmed.starts_with("---")
        || trimmed.starts_with("***")
}

impl Default for MarkdownRenderer {
    fn default() -> Self {
        Self::new()
    }
}
pub fn print_markdown(text: &str) {
    let rendered = render_markdown(text);
    let normalized = rendered.replace("\r\n", "\n").replace('\n', "\r\n");
    print!("{}", normalized);
    let _ = stdout().flush();
}

/// One-shot helper to render a markdown string to an ANSI-colored string.
pub fn render_markdown(text: &str) -> String {
    let mut in_code_block = false;
    let mut code_lang = String::new();
    let mut output = String::new();
    let mut table_lines = Vec::new();

    let flush_table = |output: &mut String, table_lines: &mut Vec<&str>, in_code_block: &mut bool, code_lang: &mut String| {
        if table_lines.is_empty() {
            return;
        }
        let lines: Vec<&str> = std::mem::take(table_lines);
        if lines.len() >= 2 && super::table::is_markdown_delimiter_line(lines[1]) {
            let table_text = lines.join("\n");
            let rendered_table = super::table::render_markdown_table(&table_text);
            output.push_str(&rendered_table);
            output.push('\n');
        } else {
            for l in lines {
                let rendered = render_line(l, in_code_block, code_lang);
                output.push_str(&rendered);
                output.push('\n');
            }
        }
    };

    for line in text.lines() {
        let trimmed = line.trim();
        if !in_code_block && super::table::is_markdown_table_line(trimmed) {
            table_lines.push(line);
        } else {
            flush_table(&mut output, &mut table_lines, &mut in_code_block, &mut code_lang);
            let rendered = render_line(line, &mut in_code_block, &mut code_lang);
            output.push_str(&rendered);
            output.push('\n');
        }
    }

    flush_table(&mut output, &mut table_lines, &mut in_code_block, &mut code_lang);

    if in_code_block {
        output.push_str(&format!("  \x1b[38;5;240m─{}\x1b[0m\n", "─".repeat(BORDER_WIDTH.saturating_sub(1))));
    }

    output
}

/// Formats a single markdown line, managing code block state.
pub fn render_line(line: &str, in_code_block: &mut bool, code_lang: &mut String) -> String {
    let trimmed = line.trim_start();

    // Check for fenced code block toggle
    if trimmed.starts_with("```") {
        if *in_code_block {
            *in_code_block = false;
            code_lang.clear();
            return format!("  \x1b[38;5;240m─{}\x1b[0m", "─".repeat(BORDER_WIDTH.saturating_sub(1)));
        } else {
            *in_code_block = true;
            let lang = trimmed.trim_start_matches('`').trim();
            *code_lang = lang.to_string();
            if lang.is_empty() {
                return format!("  \x1b[38;5;240m─{}\x1b[0m", "─".repeat(BORDER_WIDTH.saturating_sub(1)));
            } else {
                let border_len = BORDER_WIDTH.saturating_sub(lang.len() + 3);
                return format!(
                    "  \x1b[38;5;240m─\x1b[0m \x1b[1;37m{}\x1b[0m \x1b[38;5;240m{}\x1b[0m",
                    lang,
                    "─".repeat(border_len)
                );
            }
        }
    }

    // Inside code block: render diagrams and plain text cleanly without left bar
    if *in_code_block {
        let is_plain = matches!(code_lang.to_lowercase().as_str(), "text" | "ascii" | "mermaid" | "");
        if is_plain {
            return line.to_string();
        }
        let highlighted = highlight_code_line(line, code_lang);
        return format!("\x1b[38;5;240m│\x1b[0m {}", highlighted);
    }

    // Horizontal rules: --- or *** or ___ (3 or more chars)
    let rule_trimmed = trimmed.trim();
    if (rule_trimmed.len() >= 3)
        && (rule_trimmed.chars().all(|c| c == '-')
            || rule_trimmed.chars().all(|c| c == '*')
            || rule_trimmed.chars().all(|c| c == '_'))
    {
        return format!("\x1b[38;5;240m{}\x1b[0m", "─".repeat(BORDER_WIDTH));
    }

    // Headers H1 through H6
    if let Some(rest) = trimmed.strip_prefix("# ") {
        return format!("\x1b[1;36m# {}\x1b[0m", render_inline(rest.trim()));
    }
    if let Some(rest) = trimmed.strip_prefix("## ") {
        return format!("\x1b[1;34m## {}\x1b[0m", render_inline(rest.trim()));
    }
    if let Some(rest) = trimmed.strip_prefix("### ") {
        return format!("\x1b[1;35m### {}\x1b[0m", render_inline(rest.trim()));
    }
    if let Some(rest) = trimmed.strip_prefix("#### ") {
        return format!("\x1b[1;33m#### {}\x1b[0m", render_inline(rest.trim()));
    }
    if let Some(rest) = trimmed.strip_prefix("##### ") {
        return format!("\x1b[1;32m##### {}\x1b[0m", render_inline(rest.trim()));
    }
    if let Some(rest) = trimmed.strip_prefix("###### ") {
        return format!("\x1b[1;90m###### {}\x1b[0m", render_inline(rest.trim()));
    }

    // Blockquotes: > quote or >> nested quote
    if trimmed.starts_with('>') {
        let mut depth = 0;
        let mut cur = trimmed;
        while let Some(stripped) = cur.strip_prefix('>') {
            depth += 1;
            cur = stripped.trim_start();
        }
        let bars = "│ ".repeat(depth);
        let indent = " ".repeat(depth);
        if cur.is_empty() {
            return format!("{}\x1b[38;5;240m{}\x1b[0m", indent, bars.trim_end());
        }
        return format!(
            "{}\x1b[38;5;240m{}\x1b[0m\x1b[3m{}\x1b[0m",
            indent,
            bars,
            render_inline(cur)
        );
    }

    // Task lists: - [ ] or - [x] or - [X] or * [ ] etc.
    if let Some(rest) = trimmed
        .strip_prefix("- [ ] ")
        .or_else(|| trimmed.strip_prefix("* [ ] "))
        .or_else(|| trimmed.strip_prefix("+ [ ] "))
    {
        let indent = " ".repeat(line.len() - trimmed.len());
        return format!("{}  \x1b[38;5;244m[ ]\x1b[0m {}", indent, render_inline(rest));
    }
    if let Some(rest) = trimmed
        .strip_prefix("- [x] ")
        .or_else(|| trimmed.strip_prefix("- [X] "))
        .or_else(|| trimmed.strip_prefix("* [x] "))
        .or_else(|| trimmed.strip_prefix("* [X] "))
        .or_else(|| trimmed.strip_prefix("+ [x] "))
        .or_else(|| trimmed.strip_prefix("+ [X] "))
    {
        let indent = " ".repeat(line.len() - trimmed.len());
        return format!("{}  \x1b[32m[✓]\x1b[0m {}", indent, render_inline(rest));
    }

    // Bullet lists: - item, * item, + item
    if let Some(rest) = trimmed
        .strip_prefix("- ")
        .or_else(|| trimmed.strip_prefix("* "))
        .or_else(|| trimmed.strip_prefix("+ "))
    {
        let indent = " ".repeat(line.len() - trimmed.len());
        return format!("{}  \x1b[36m•\x1b[0m {}", indent, render_inline(rest));
    }

    // Numbered lists: 1. item, 2. item, etc.
    if let Some((num, rest)) = parse_numbered_list(trimmed) {
        let indent = " ".repeat(line.len() - trimmed.len());
        return format!("{}  \x1b[36m{}.\x1b[0m {}", indent, num, render_inline(rest));
    }

    // Markdown tables: | col1 | col2 |
    if let Some(table_row) = render_table_line(trimmed) {
        return table_row;
    }

    // Standard paragraph line with inline markdown
    render_inline(line)
}

/// Helper to parse numbered list item e.g. "1. Hello" -> (1, "Hello")
fn parse_numbered_list(line: &str) -> Option<(usize, &str)> {
    let dot_pos = line.find('.')?;
    let num_str = &line[..dot_pos];
    if num_str.is_empty() {
        return None;
    }
    let num: usize = num_str.parse().ok()?;
    let rest = line[dot_pos + 1..].strip_prefix(' ')?;
    Some((num, rest))
}

/// Helper to format table rows and delimiter rows cleanly.
fn render_table_line(line: &str) -> Option<String> {
    if !line.starts_with('|') || !line.ends_with('|') {
        return None;
    }
    let inner = &line[1..line.len() - 1];
    let parts: Vec<&str> = inner.split('|').collect();
    if parts.is_empty() {
        return None;
    }

    // Check if it is a delimiter row e.g. |---|---|
    let is_delimiter = parts.iter().all(|p| {
        let p_trim = p.trim();
        !p_trim.is_empty()
            && p_trim
                .chars()
                .all(|c| c == '-' || c == ':' || c.is_whitespace())
    });

    if is_delimiter {
        let sep: Vec<String> = parts
            .iter()
            .map(|p| "─".repeat(p.trim().len().max(3)))
            .collect();
        Some(format!(
            "\x1b[38;5;240m├──{}──┤\x1b[0m",
            sep.join("┼")
        ))
    } else {
        let formatted_cells: Vec<String> = parts
            .iter()
            .map(|c| format!(" {} ", render_inline(c.trim())))
            .collect();
        Some(format!(
            "\x1b[38;5;240m│\x1b[0m{}\x1b[38;5;240m│\x1b[0m",
            formatted_cells.join("\x1b[38;5;240m│\x1b[0m")
        ))
    }
}

/// Parse and format inline markdown: bold (** or __), italic (* or _),
/// bold+italic (*** or ___), inline code (` `), strikethrough (~~), and links [text](url).
pub fn render_inline(text: &str) -> String {
    let mut result = String::with_capacity(text.len() * 2);
    let chars: Vec<char> = text.chars().collect();
    let len = chars.len();
    let mut i = 0;

    while i < len {
        // Escape sequence: \char
        if chars[i] == '\\' && i + 1 < len {
            let next = chars[i + 1];
            if next == '*' || next == '_' || next == '`' || next == '~' || next == '[' || next == ']' || next == '#' || next == '\\' {
                result.push(next);
                i += 2;
                continue;
            }
        }

        // Inline code: `code`
        if chars[i] == '`' {
            let mut j = i + 1;
            while j < len && chars[j] != '`' {
                j += 1;
            }
            if j < len {
                let code_content: String = chars[i + 1..j].iter().collect();
                result.push_str("\x1b[33m`");
                result.push_str(&code_content);
                result.push_str("`\x1b[0m");
                i = j + 1;
                continue;
            }
        }

        // Bold + Italic: ***text*** or ___text___
        if i + 2 < len
            && ((chars[i] == '*' && chars[i + 1] == '*' && chars[i + 2] == '*')
                || (chars[i] == '_' && chars[i + 1] == '_' && chars[i + 2] == '_'))
        {
            let delim = chars[i];
            let mut j = i + 3;
            while j + 2 < len && !(chars[j] == delim && chars[j + 1] == delim && chars[j + 2] == delim) {
                j += 1;
            }
            if j + 2 < len {
                let content: String = chars[i + 3..j].iter().collect();
                result.push_str("\x1b[1;3m");
                result.push_str(&render_inline(&content));
                result.push_str("\x1b[22;23m");
                i = j + 3;
                continue;
            }
        }

        // Bold: **text** or __text__
        if i + 1 < len
            && ((chars[i] == '*' && chars[i + 1] == '*')
                || (chars[i] == '_' && chars[i + 1] == '_'))
        {
            let delim = chars[i];
            let mut j = i + 2;
            while j + 1 < len && !(chars[j] == delim && chars[j + 1] == delim) {
                j += 1;
            }
            if j + 1 < len {
                let bold_content: String = chars[i + 2..j].iter().collect();
                result.push_str("\x1b[1m");
                result.push_str(&render_inline(&bold_content));
                result.push_str("\x1b[22m");
                i = j + 2;
                continue;
            }
        }

        // Strikethrough: ~~text~~
        if i + 1 < len && chars[i] == '~' && chars[i + 1] == '~' {
            let mut j = i + 2;
            while j + 1 < len && !(chars[j] == '~' && chars[j + 1] == '~') {
                j += 1;
            }
            if j + 1 < len {
                let strike_content: String = chars[i + 2..j].iter().collect();
                result.push_str("\x1b[9m");
                result.push_str(&render_inline(&strike_content));
                result.push_str("\x1b[29m");
                i = j + 2;
                continue;
            }
        }

        // Italic: *text* or _text_ (single char delimiter)
        if chars[i] == '*'
            || (chars[i] == '_' && (i == 0 || chars[i - 1].is_whitespace() || chars[i - 1].is_ascii_punctuation()))
        {
            let delim = chars[i];
            let mut j = i + 1;
            while j < len && chars[j] != delim && chars[j] != '\n' {
                j += 1;
            }
            if j < len && chars[j] == delim && j > i + 1 {
                let valid_end = if delim == '_' {
                    j + 1 == len || chars[j + 1].is_whitespace() || chars[j + 1].is_ascii_punctuation()
                } else {
                    true
                };
                if valid_end {
                    let italic_content: String = chars[i + 1..j].iter().collect();
                    result.push_str("\x1b[3m");
                    result.push_str(&render_inline(&italic_content));
                    result.push_str("\x1b[23m");
                    i = j + 1;
                    continue;
                }
            }
        }

        // Link: [text](url)
        if chars[i] == '[' {
            if let Some(close_bracket) = chars[i + 1..].iter().position(|&c| c == ']') {
                let close_bracket_idx = i + 1 + close_bracket;
                if close_bracket_idx + 1 < len && chars[close_bracket_idx + 1] == '(' {
                    if let Some(close_paren) = chars[close_bracket_idx + 2..].iter().position(|&c| c == ')') {
                        let close_paren_idx = close_bracket_idx + 2 + close_paren;
                        let link_text: String = chars[i + 1..close_bracket_idx].iter().collect();
                        let link_url: String = chars[close_bracket_idx + 2..close_paren_idx].iter().collect();
                        result.push_str("\x1b[4;34m");
                        result.push_str(&link_text);
                        result.push_str("\x1b[0m (\x1b[38;5;244m");
                        result.push_str(&link_url);
                        result.push_str("\x1b[0m)");
                        i = close_paren_idx + 1;
                        continue;
                    }
                }
            }
        }

        // Autolink: <http://...> or <https://...> or <mailto:...>
        if chars[i] == '<' {
            if let Some(close_angle) = chars[i + 1..].iter().position(|&c| c == '>') {
                let close_idx = i + 1 + close_angle;
                let inner: String = chars[i + 1..close_idx].iter().collect();
                if inner.starts_with("http://") || inner.starts_with("https://") || inner.starts_with("mailto:") {
                    result.push_str("\x1b[4;34m");
                    result.push_str(&inner);
                    result.push_str("\x1b[0m");
                    i = close_idx + 1;
                    continue;
                }
            }
        }

        result.push(chars[i]);
        i += 1;
    }

    result
}

/// Lightweight syntax highlighting for lines inside fenced code blocks.
pub fn highlight_code_line(line: &str, lang: &str) -> String {
    let lang_lower = lang.to_lowercase();
    let lang_clean = lang_lower.trim();

    if lang_clean.is_empty() {
        return format!("\x1b[38;5;252m{}\x1b[0m", line);
    }

    let trimmed = line.trim_start();
    let indent = &line[..line.len() - trimmed.len()];

    // Full-line comments
    if (lang_clean == "rust" || lang_clean == "rs" || lang_clean == "js" || lang_clean == "ts"
        || lang_clean == "javascript" || lang_clean == "typescript" || lang_clean == "go"
        || lang_clean == "c" || lang_clean == "cpp" || lang_clean == "java" || lang_clean == "kotlin"
        || lang_clean == "swift" || lang_clean == "cs" || lang_clean == "csharp")
        && (trimmed.starts_with("//") || trimmed.starts_with("/*") || trimmed.starts_with('*'))
    {
        return format!("{}\x1b[38;5;244;3m{}\x1b[0m", indent, trimmed);
    }

    if (lang_clean == "python" || lang_clean == "py" || lang_clean == "bash" || lang_clean == "sh"
        || lang_clean == "zsh" || lang_clean == "toml" || lang_clean == "yaml" || lang_clean == "yml"
        || lang_clean == "ruby" || lang_clean == "rb" || lang_clean == "dockerfile")
        && trimmed.starts_with('#')
    {
        return format!("{}\x1b[38;5;244;3m{}\x1b[0m", indent, trimmed);
    }

    if (lang_clean == "sql" || lang_clean == "lua" || lang_clean == "hs" || lang_clean == "haskell")
        && trimmed.starts_with("--")
    {
        return format!("{}\x1b[38;5;244;3m{}\x1b[0m", indent, trimmed);
    }

    let is_keyword: fn(&str) -> bool = match lang_clean {
        "rust" | "rs" => is_rust_keyword,
        "python" | "py" => is_python_keyword,
        "js" | "ts" | "javascript" | "typescript" | "jsx" | "tsx" => is_js_keyword,
        "bash" | "sh" | "zsh" | "shell" => is_sh_keyword,
        "json" => is_json_keyword,
        "toml" | "yaml" | "yml" => is_config_keyword,
        "go" => is_go_keyword,
        "c" | "cpp" | "c++" => is_c_keyword,
        "sql" => is_sql_keyword,
        _ => is_generic_keyword,
    };

    let mut out = String::with_capacity(line.len() * 2);
    out.push_str(indent);

    let chars: Vec<char> = trimmed.chars().collect();
    let len = chars.len();
    let mut i = 0;

    while i < len {
        // String literal with double quotes: "..."
        if chars[i] == '"' {
            let mut j = i + 1;
            let mut escaped = false;
            while j < len {
                if escaped {
                    escaped = false;
                } else if chars[j] == '\\' {
                    escaped = true;
                } else if chars[j] == '"' {
                    j += 1;
                    break;
                }
                j += 1;
            }
            let s: String = chars[i..j].iter().collect();
            out.push_str("\x1b[38;5;149m");
            out.push_str(&s);
            out.push_str("\x1b[0m");
            i = j;
            continue;
        }

        // String literal with single quotes: '...'
        if chars[i] == '\'' && (lang_clean != "rust" && lang_clean != "rs" || (i + 2 < len && chars[i + 2] == '\'')) {
            let mut j = i + 1;
            let mut escaped = false;
            while j < len {
                if escaped {
                    escaped = false;
                } else if chars[j] == '\\' {
                    escaped = true;
                } else if chars[j] == '\'' {
                    j += 1;
                    break;
                }
                j += 1;
            }
            let s: String = chars[i..j].iter().collect();
            out.push_str("\x1b[38;5;149m");
            out.push_str(&s);
            out.push_str("\x1b[0m");
            i = j;
            continue;
        }

        // Inline comment //
        if (lang_clean == "rust" || lang_clean == "rs" || lang_clean == "js" || lang_clean == "ts"
            || lang_clean == "go" || lang_clean == "c" || lang_clean == "cpp")
            && i + 1 < len
            && chars[i] == '/'
            && chars[i + 1] == '/'
        {
            let comment: String = chars[i..].iter().collect();
            out.push_str("\x1b[38;5;244;3m");
            out.push_str(&comment);
            out.push_str("\x1b[0m");
            break;
        }

        // Inline comment #
        if (lang_clean == "python" || lang_clean == "py" || lang_clean == "bash" || lang_clean == "sh"
            || lang_clean == "toml" || lang_clean == "yaml")
            && chars[i] == '#'
        {
            let comment: String = chars[i..].iter().collect();
            out.push_str("\x1b[38;5;244;3m");
            out.push_str(&comment);
            out.push_str("\x1b[0m");
            break;
        }

        // Word (identifier / keyword / number / type)
        if chars[i].is_alphanumeric() || chars[i] == '_' {
            let start = i;
            while i < len && (chars[i].is_alphanumeric() || chars[i] == '_') {
                i += 1;
            }
            let word: String = chars[start..i].iter().collect();
            if is_keyword(&word) {
                out.push_str("\x1b[38;5;176m");
                out.push_str(&word);
                out.push_str("\x1b[0m");
            } else if word.chars().next().map_or(false, |c| c.is_ascii_digit()) {
                out.push_str("\x1b[38;5;215m");
                out.push_str(&word);
                out.push_str("\x1b[0m");
            } else if word.chars().next().map_or(false, |c| c.is_uppercase()) {
                out.push_str("\x1b[38;5;110m");
                out.push_str(&word);
                out.push_str("\x1b[0m");
            } else {
                out.push_str("\x1b[38;5;252m");
                out.push_str(&word);
                out.push_str("\x1b[0m");
            }
            continue;
        }

        // Punctuation and operators
        out.push(chars[i]);
        i += 1;
    }

    out
}

fn is_rust_keyword(word: &str) -> bool {
    matches!(
        word,
        "as" | "async" | "await" | "break" | "const" | "continue" | "crate" | "dyn"
            | "else" | "enum" | "extern" | "false" | "fn" | "for" | "if" | "impl"
            | "in" | "let" | "loop" | "match" | "mod" | "move" | "mut" | "pub"
            | "ref" | "return" | "self" | "Self" | "static" | "struct" | "super"
            | "trait" | "true" | "type" | "unsafe" | "use" | "where" | "while"
    )
}

fn is_python_keyword(word: &str) -> bool {
    matches!(
        word,
        "and" | "as" | "assert" | "async" | "await" | "break" | "class" | "continue"
            | "def" | "del" | "elif" | "else" | "except" | "False" | "finally"
            | "for" | "from" | "global" | "if" | "import" | "in" | "is" | "lambda"
            | "None" | "nonlocal" | "not" | "or" | "pass" | "raise" | "return"
            | "self" | "True" | "try" | "while" | "with" | "yield"
    )
}

fn is_js_keyword(word: &str) -> bool {
    matches!(
        word,
        "async" | "await" | "break" | "case" | "catch" | "class" | "const" | "continue"
            | "debugger" | "default" | "delete" | "do" | "else" | "export" | "extends"
            | "false" | "finally" | "for" | "from" | "function" | "if" | "import"
            | "in" | "instanceof" | "interface" | "let" | "new" | "null" | "of"
            | "return" | "super" | "switch" | "this" | "throw" | "true" | "try"
            | "type" | "typeof" | "undefined" | "var" | "void" | "while" | "with" | "yield"
    )
}

fn is_sh_keyword(word: &str) -> bool {
    matches!(
        word,
        "case" | "do" | "done" | "elif" | "else" | "esac" | "exit" | "export"
            | "fi" | "for" | "function" | "if" | "in" | "local" | "return" | "select"
            | "then" | "time" | "until" | "while" | "echo" | "set" | "unset"
    )
}

fn is_json_keyword(word: &str) -> bool {
    matches!(word, "true" | "false" | "null")
}

fn is_config_keyword(word: &str) -> bool {
    matches!(word, "true" | "false" | "null" | "yes" | "no" | "on" | "off")
}

fn is_go_keyword(word: &str) -> bool {
    matches!(
        word,
        "break" | "case" | "chan" | "const" | "continue" | "default" | "defer"
            | "else" | "fallthrough" | "for" | "func" | "go" | "goto" | "if"
            | "import" | "interface" | "map" | "package" | "range" | "return"
            | "select" | "struct" | "switch" | "type" | "var" | "true" | "false" | "nil"
    )
}

fn is_c_keyword(word: &str) -> bool {
    matches!(
        word,
        "auto" | "break" | "case" | "char" | "const" | "continue" | "default" | "do"
            | "double" | "else" | "enum" | "extern" | "float" | "for" | "goto" | "if"
            | "int" | "long" | "register" | "return" | "short" | "signed" | "sizeof"
            | "static" | "struct" | "switch" | "typedef" | "union" | "unsigned" | "void"
            | "volatile" | "while" | "class" | "public" | "private" | "protected"
            | "template" | "typename" | "namespace" | "using" | "virtual" | "bool"
            | "true" | "false" | "nullptr"
    )
}

fn is_sql_keyword(word: &str) -> bool {
    let u = word.to_ascii_uppercase();
    matches!(
        u.as_str(),
        "SELECT" | "FROM" | "WHERE" | "INSERT" | "INTO" | "UPDATE" | "DELETE" | "JOIN"
            | "LEFT" | "RIGHT" | "INNER" | "OUTER" | "ON" | "GROUP" | "BY" | "ORDER"
            | "HAVING" | "LIMIT" | "OFFSET" | "AS" | "AND" | "OR" | "NOT" | "NULL"
            | "CREATE" | "TABLE" | "DROP" | "ALTER" | "INDEX" | "PRIMARY" | "KEY"
            | "FOREIGN" | "REFERENCES" | "DISTINCT" | "UNION" | "ALL" | "VALUES" | "SET"
    )
}

fn is_generic_keyword(word: &str) -> bool {
    matches!(
        word,
        "fn" | "def" | "function" | "func" | "var" | "let" | "const" | "class"
            | "struct" | "interface" | "return" | "if" | "else" | "for" | "while"
            | "import" | "export" | "from" | "pub" | "true" | "false" | "null" | "nil"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_headers() {
        let h1 = render_line("# Title", &mut false, &mut String::new());
        assert!(h1.contains("\x1b[1;36m# Title\x1b[0m"));

        let h2 = render_line("## Subtitle", &mut false, &mut String::new());
        assert!(h2.contains("\x1b[1;34m## Subtitle\x1b[0m"));

        let h3 = render_line("### Section", &mut false, &mut String::new());
        assert!(h3.contains("\x1b[1;35m### Section\x1b[0m"));

        let h4 = render_line("#### Subsection", &mut false, &mut String::new());
        assert!(h4.contains("\x1b[1;33m#### Subsection\x1b[0m"));

        let h5 = render_line("##### Minor", &mut false, &mut String::new());
        assert!(h5.contains("\x1b[1;32m##### Minor\x1b[0m"));

        let h6 = render_line("###### Detail", &mut false, &mut String::new());
        assert!(h6.contains("\x1b[1;90m###### Detail\x1b[0m"));
    }

    #[test]
    fn test_code_block() {
        let mut in_code = false;
        let mut lang = String::new();

        let top = render_line("```rust", &mut in_code, &mut lang);
        assert!(in_code);
        assert_eq!(lang, "rust");
        assert!(top.contains("rust"));
        assert!(top.contains("─"));

        let code_line = render_line("let x = 42;", &mut in_code, &mut lang);
        assert!(code_line.contains("│"));
        assert!(code_line.contains("let"));
        assert!(code_line.contains("42"));

        let bottom = render_line("```", &mut in_code, &mut lang);
        assert!(!in_code);
        assert!(bottom.contains("─"));
    }

    #[test]
    fn test_inline_formatting() {
        let inline = render_inline("This is **bold** and `code` and *italic* and ~~strike~~ test.");
        assert!(inline.contains("\x1b[1mbold\x1b[22m"));
        assert!(inline.contains("\x1b[33m`code`\x1b[0m"));
        assert!(inline.contains("\x1b[3mitalic\x1b[23m"));
        assert!(inline.contains("\x1b[9mstrike\x1b[29m"));
    }

    #[test]
    fn test_bold_italic_and_links() {
        let bi = render_inline("***bold italic*** and [link](https://example.com)");
        assert!(bi.contains("\x1b[1;3mbold italic\x1b[22;23m"));
        assert!(bi.contains("\x1b[4;34mlink\x1b[0m"));
        assert!(bi.contains("https://example.com"));
    }

    #[test]
    fn test_lists() {
        let bullet = render_line("- Item 1", &mut false, &mut String::new());
        assert!(bullet.contains("•"));
        assert!(bullet.contains("Item 1"));

        let numbered = render_line("1. First item", &mut false, &mut String::new());
        assert!(numbered.contains("1."));
        assert!(numbered.contains("First item"));
    }

    #[test]
    fn test_task_lists() {
        let unchecked = render_line("- [ ] Todo item", &mut false, &mut String::new());
        assert!(unchecked.contains("[ ]"));
        assert!(unchecked.contains("Todo item"));

        let checked = render_line("- [x] Done item", &mut false, &mut String::new());
        assert!(checked.contains("[✓]"));
        assert!(checked.contains("Done item"));
    }

    #[test]
    fn test_blockquotes() {
        let bq = render_line("> Simple quote", &mut false, &mut String::new());
        assert!(bq.contains("│"));
        assert!(bq.contains("Simple quote"));

        let nbq = render_line(">> Nested quote", &mut false, &mut String::new());
        assert!(nbq.contains("│ │"));
        assert!(nbq.contains("Nested quote"));
    }

    #[test]
    fn test_horizontal_rules() {
        let hr = render_line("---", &mut false, &mut String::new());
        assert!(hr.contains("─"));
    }

    #[test]
    fn test_tables() {
        let delim = render_line("| Name | Age |", &mut false, &mut String::new());
        assert!(delim.contains("│"));
        assert!(delim.contains("Name"));

        let sep = render_line("| --- | --- |", &mut false, &mut String::new());
        assert!(sep.contains("├──"));
        assert!(sep.contains("┼"));
    }

    #[test]
    fn test_streaming_renderer() {
        let mut renderer = MarkdownRenderer::buffered();
        let chunk1 = renderer.push("# Hello\n\nThis is **streaming");
        assert!(chunk1.contains("# Hello"));

        let chunk2 = renderer.push("** markdown.\n```rust\nfn main() {}\n```\n");
        assert!(chunk2.contains("streaming"));
        assert!(chunk2.contains("─"));
        assert!(chunk2.contains("main"));
        assert!(chunk2.contains("─"));

        let finished = renderer.finish();
        assert_eq!(finished, "");
    }

    #[test]
    fn test_streaming_unclosed_code_block() {
        let mut renderer = MarkdownRenderer::buffered();
        renderer.push("```python\nprint('hello')\n");
        assert!(renderer.is_in_code_block());
        let finish = renderer.finish();
        assert!(!renderer.is_in_code_block());
        assert!(finish.contains("─"));
    }

    #[test]
    fn test_render_markdown_full_document() {
        let doc = "# Main Heading\n\nSome introductory text with **bold**.\n\n- Point A\n- Point B\n\n```sh\necho \"Hello\"\n```\n";
        let rendered = render_markdown(doc);
        assert!(rendered.contains("# Main Heading"));
        assert!(rendered.contains("bold"));
        assert!(rendered.contains("•"));
        assert!(rendered.contains("Point A"));
        assert!(rendered.contains("─"));
        assert!(rendered.contains("echo"));
        assert!(rendered.contains("─"));
    }

    #[test]
    fn test_syntax_highlighting_languages() {
        let py = highlight_code_line("def hello(name):", "python");
        assert!(py.contains("def"));

        let rs = highlight_code_line("pub fn test() -> bool {", "rust");
        assert!(rs.contains("pub"));
        assert!(rs.contains("fn"));

        let sh = highlight_code_line("export PATH=\"/bin:$PATH\"", "bash");
        assert!(sh.contains("export"));

        let sql = highlight_code_line("SELECT id, name FROM users WHERE active = true;", "sql");
        assert!(sql.contains("SELECT"));
        assert!(sql.contains("FROM"));
    }

    #[test]
    fn test_inline_escaping() {
        let escaped = render_inline(r"This is \*not italic\* and \`not code\`.");
        assert_eq!(escaped, "This is *not italic* and `not code`.");
    }

    #[test]
    fn test_autolinks() {
        let auto = render_inline("Visit <https://github.com> for more.");
        assert!(auto.contains("https://github.com"));
        assert!(auto.contains("\x1b[4;34m"));
    }

    #[test]
    fn test_renderer_reset() {
        let mut renderer = MarkdownRenderer::buffered();
        renderer.push("```rust\nlet x = 1;\n");
        assert!(renderer.is_in_code_block());
        renderer.reset();
        assert!(!renderer.is_in_code_block());
        assert_eq!(renderer.code_lang(), "");
    }
    #[test]
    fn test_renderer_indent_and_blank_lines() {
        let mut renderer = MarkdownRenderer::buffered().with_indent(2);
        let output = renderer.push("Line 1\n\nLine 2\n");
        assert_eq!(output, "  Line 1\n\n  Line 2\n");
    }

    #[test]
    fn test_renderer_indent_unclosed_code_block() {
        let mut renderer = MarkdownRenderer::buffered().with_indent(4);
        let chunk = renderer.push("```python\nprint('hi')\n");
        assert!(chunk.starts_with("      \x1b[38;5;240m─"));
        let finished = renderer.finish();
        assert!(finished.starts_with("      \x1b[38;5;240m─"));
    }

    #[test]
    fn test_mermaid_block_renders_ascii_diagram() {
        let mut renderer = MarkdownRenderer::buffered();
        let mut output = renderer.push("```mermaid\ngraph TD\n    A[Start] --> B[Process]\n    B --> C[End]\n```\n");
        output.push_str(&renderer.finish());
        assert!(output.contains("+---------"));
        assert!(output.contains("Start"));
        assert!(output.contains("Process"));
        assert!(output.contains("End"));
        assert!(output.contains("v"));
    }

    #[test]
    fn test_plain_code_blocks_omit_left_border() {
        for lang_name in &["text", "ascii", "mermaid", ""] {
            let mut in_code = true;
            let mut lang = lang_name.to_string();
            let line = "  +---+  ";
            let rendered = render_line(line, &mut in_code, &mut lang);
            assert_eq!(rendered, "  +---+  ", "Language {} should not have left bar", lang_name);
        }
    }

    #[test]
    fn test_progressive_word_streaming_state_transitions() {
        let mut renderer = MarkdownRenderer::new().with_indent(2);
        assert!(!renderer.line_has_prefix());

        // Pushing word without space stays in buffer, prefix not set yet
        renderer.push("Hello");
        assert_eq!(renderer.pending(), "Hello");
        assert!(!renderer.line_has_prefix());

        // Pushing space flushes "Hello " and sets line_has_prefix to true
        renderer.push(" ");
        assert_eq!(renderer.pending(), "");
        assert!(renderer.line_has_prefix());

        // Pushing next word with space flushes and keeps line_has_prefix true
        renderer.push("world! ");
        assert_eq!(renderer.pending(), "");
        assert!(renderer.line_has_prefix());

        // Pushing newline resets line_has_prefix to false
        renderer.push("Done.\n");
        assert_eq!(renderer.pending(), "");
        assert!(!renderer.line_has_prefix());
    }

    #[test]
    fn test_progressive_streaming_special_blocks_buffered_until_newline() {
        let mut renderer = MarkdownRenderer::new().with_indent(2);

        // Header syntax should not stream word-by-word
        renderer.push("# Heading");
        assert_eq!(renderer.pending(), "# Heading");
        assert!(!renderer.line_has_prefix());

        renderer.push(" with spaces ");
        assert_eq!(renderer.pending(), "# Heading with spaces ");
        assert!(!renderer.line_has_prefix());

        renderer.push("\n");
        assert_eq!(renderer.pending(), "");
        assert!(!renderer.line_has_prefix());
    }

    #[test]
    fn test_progressive_streaming_finish_flushes_pending() {
        let mut renderer = MarkdownRenderer::new().with_indent(2);
        renderer.push("Pending without newline");
        assert_eq!(renderer.pending(), "newline");
        assert!(renderer.line_has_prefix());

        renderer.finish();
        assert_eq!(renderer.pending(), "");
        assert!(!renderer.line_has_prefix());
    }

    #[test]
    fn test_progressive_streaming_list_items() {
        let mut renderer = MarkdownRenderer::new().with_indent(2);

        // List items are normal streamable text
        renderer.push("- Item ");
        assert_eq!(renderer.pending(), "");
        assert!(renderer.line_has_prefix());

        renderer.push("one\n");
        assert_eq!(renderer.pending(), "");
        assert!(!renderer.line_has_prefix());
    }

    #[test]
    fn test_progressive_streaming_code_block_transition() {
        let mut renderer = MarkdownRenderer::new().with_indent(2);

        // Start code block
        renderer.push("```rust\n");
        assert!(renderer.is_in_code_block());
        assert!(!renderer.line_has_prefix());

        // Code inside block is not word-streamed
        renderer.push("let x = ");
        assert_eq!(renderer.pending(), "let x = ");
        assert!(!renderer.line_has_prefix());

        renderer.push("42;\n");
        assert_eq!(renderer.pending(), "");
        assert!(!renderer.line_has_prefix());

        // Close code block
        renderer.push("```\n");
        assert!(!renderer.is_in_code_block());
        assert!(!renderer.line_has_prefix());

        // Normal text after code block streams progressively
        renderer.push("Now back to normal ");
        assert_eq!(renderer.pending(), "");
        assert!(renderer.line_has_prefix());
    }

    #[test]
    fn test_progressive_streaming_reset() {
        let mut renderer = MarkdownRenderer::new().with_indent(2);
        renderer.push("Starting word ");
        assert!(renderer.line_has_prefix());

        renderer.reset();
        assert!(!renderer.line_has_prefix());
        assert_eq!(renderer.pending(), "");
    }
}
