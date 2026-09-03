//! Dynamic ANSI table column auto-sizer and responsive formatter for Markdown tables.
//!
//! Designed for high-performance terminal rendering with automatic adaptation
//! to terminal width constraints, mobile/Termux portrait screens, ANSI escape
//! sequences, and multi-line cell wrapping without layout glitching.

use std::fmt::Write as FmtWrite;

/// Minimum allowed column content width (excluding borders and padding).
pub const DEFAULT_MIN_COL_WIDTH: usize = 3;

/// Default padding spaces on left and right sides of each cell.
pub const DEFAULT_CELL_PADDING: usize = 1;

/// Default fallback width when terminal size cannot be detected.
pub const DEFAULT_FALLBACK_TERMINAL_WIDTH: usize = 80;

/// Threshold below which a terminal is considered narrow (e.g. mobile/Termux portrait).
pub const NARROW_TERMINAL_THRESHOLD: usize = 50;

/// Detects the current terminal width in columns, with multiple fallbacks.
pub fn get_terminal_width() -> usize {
    // 1. Try crossterm terminal size
    if let Ok((cols, _)) = crossterm::terminal::size() {
        if cols > 0 {
            return cols as usize;
        }
    }

    // 2. Try COLUMNS environment variable
    if let Ok(cols_str) = std::env::var("COLUMNS") {
        if let Ok(cols) = cols_str.trim().parse::<usize>() {
            if cols > 0 {
                return cols;
            }
        }
    }

    // 3. Detect Android Termux environment
    if std::env::var("TERMUX_VERSION").is_ok()
        || std::env::var("PREFIX")
            .map(|p| p.contains("com.termux"))
            .unwrap_or(false)
    {
        return 60;
    }

    DEFAULT_FALLBACK_TERMINAL_WIDTH
}

/// Computes the visible character width of a string on a monospace terminal.
///
/// Skips ANSI CSI and OSC escape sequences and correctly weights double-width
/// Unicode characters (CJK, emojis) and zero-width characters (combining marks,
/// zero-width spaces).
pub fn visible_width(s: &str) -> usize {
    let mut width = 0;
    let mut chars = s.chars().peekable();

    while let Some(c) = chars.next() {
        // ANSI escape sequence: \x1b[...] (CSI) or \x1b]... (OSC)
        if c == '\x1b' {
            if let Some(&next_c) = chars.peek() {
                if next_c == '[' {
                    // CSI sequence: skip until final byte (0x40..=0x7E)
                    chars.next();
                    while let Some(seq_c) = chars.next() {
                        if ('@'..='~').contains(&seq_c) {
                            break;
                        }
                    }
                    continue;
                } else if next_c == ']' {
                    // OSC sequence: skip until BEL (\x07) or ST (\x1b\)
                    chars.next();
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
                } else {
                    // Two-character escape sequence (e.g. \x1bM, \x1b(B)
                    chars.next();
                    continue;
                }
            }
        }

        // Control characters: 0 width
        if c.is_control() {
            continue;
        }

        // Zero-width combining characters and format codes
        if is_zero_width(c) {
            continue;
        }

        // Wide characters: CJK, full-width forms, emojis
        if is_wide_char(c) {
            width += 2;
        } else {
            width += 1;
        }
    }

    width
}

/// Returns true if character is zero-width (combining mark, format code, zero-width space).
fn is_zero_width(c: char) -> bool {
    matches!(c,
        '\u{00AD}' // Soft hyphen
        | '\u{200B}'..='\u{200F}' // Zero width space, joiners, marks
        | '\u{202A}'..='\u{202E}' // Directional overrides
        | '\u{2060}'..='\u{206F}' // Word joiner, invisible operators
        | '\u{FEFF}' // Zero-width non-breaking space / BOM
        | '\u{0300}'..='\u{036F}' // Combining diacritical marks
        | '\u{1AB0}'..='\u{1AFF}'
        | '\u{1DC0}'..='\u{1DFF}'
        | '\u{20D0}'..='\u{20FF}'
        | '\u{FE20}'..='\u{FE2F}'
    )
}

/// Returns true if character has double-width display on monospace terminal.
fn is_wide_char(c: char) -> bool {
    matches!(c,
        '\u{1100}'..='\u{115F}' // Hangul Jamo
        | '\u{2E80}'..='\u{303E}' // CJK Radicals, Kangxi, CJK Symbols
        | '\u{3040}'..='\u{4DBF}' // Hiragana, Katakana, Bopomofo, CJK Unified Ext A
        | '\u{4E00}'..='\u{9FFF}' // CJK Unified Ideographs
        | '\u{AC00}'..='\u{D7A3}' // Hangul Syllables
        | '\u{F900}'..='\u{FAFF}' // CJK Compatibility Ideographs
        | '\u{FE10}'..='\u{FE19}' // Vertical forms
        | '\u{FE30}'..='\u{FE6F}' // CJK Compatibility Forms
        | '\u{FF01}'..='\u{FF60}' // Fullwidth Forms
        | '\u{FFE0}'..='\u{FFE6}' // Fullwidth Symbol Variants
        | '\u{1F300}'..='\u{1F9FF}' // Miscellaneous Symbols and Pictographs, Emojis
        | '\u{1FA00}'..='\u{1FAFF}' // Symbols and Pictographs Extended-A
    )
}

/// Strips all ANSI escape codes from a string.
pub fn strip_ansi(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();

    while let Some(c) = chars.next() {
        if c == '\x1b' {
            if let Some(&next_c) = chars.peek() {
                if next_c == '[' {
                    chars.next();
                    while let Some(seq_c) = chars.next() {
                        if ('@'..='~').contains(&seq_c) {
                            break;
                        }
                    }
                    continue;
                } else if next_c == ']' {
                    chars.next();
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
                } else {
                    chars.next();
                    continue;
                }
            }
        }
        out.push(c);
    }

    out
}

/// Truncates a string to fit `max_width` visible columns.
///
/// If truncated, appends `ellipsis` and appends `\x1b[0m` if ANSI formatting was active.
pub fn truncate_ansi(s: &str, max_width: usize, ellipsis: &str) -> String {
    let s_width = visible_width(s);
    if s_width <= max_width {
        return s.to_string();
    }

    let ellipsis_width = visible_width(ellipsis);
    let target_width = max_width.saturating_sub(ellipsis_width);

    let mut out = String::new();
    let mut current_width = 0;
    let mut has_ansi = false;
    let mut chars = s.chars().peekable();

    while let Some(c) = chars.next() {
        if c == '\x1b' {
            has_ansi = true;
            out.push(c);
            if let Some(&next_c) = chars.peek() {
                if next_c == '[' {
                    out.push(chars.next().unwrap());
                    while let Some(seq_c) = chars.next() {
                        out.push(seq_c);
                        if ('@'..='~').contains(&seq_c) {
                            break;
                        }
                    }
                    continue;
                } else if next_c == ']' {
                    out.push(chars.next().unwrap());
                    while let Some(seq_c) = chars.next() {
                        out.push(seq_c);
                        if seq_c == '\x07' {
                            break;
                        }
                        if seq_c == '\x1b' && chars.peek() == Some(&'\\') {
                            out.push(chars.next().unwrap());
                            break;
                        }
                    }
                    continue;
                }
            }
            continue;
        }

        let char_w = if is_zero_width(c) {
            0
        } else if is_wide_char(c) {
            2
        } else {
            1
        };

        if current_width + char_w > target_width {
            break;
        }

        out.push(c);
        current_width += char_w;
    }

    out.push_str(ellipsis);
    if has_ansi {
        out.push_str("\x1b[0m");
    }

    out
}

/// Wraps text to fit within `max_width` visible columns.
///
/// Correctly breaks on word boundaries where possible, or character boundaries for long words.
/// Preserves active ANSI styling across wrapped lines so formatting doesn't leak or break.
pub fn wrap_ansi(text: &str, max_width: usize) -> Vec<String> {
    if max_width == 0 {
        return vec![String::new()];
    }

    let trimmed = text.trim();
    if trimmed.is_empty() {
        return vec![String::new()];
    }

    let mut lines = Vec::new();
    let mut current_line = String::new();
    let mut current_width = 0;
    let mut active_ansi = String::new();

    // Split text into tokens (words, spaces, ANSI sequences)
    let tokens = tokenize_text(trimmed);

    for token in tokens {
        match token {
            Token::Ansi(seq) => {
                if seq == "\x1b[0m" || seq == "\x1b[m" {
                    active_ansi.clear();
                } else {
                    active_ansi.push_str(&seq);
                }
                current_line.push_str(&seq);
            }
            Token::Whitespace(ws) => {
                let ws_len = ws.chars().count();
                if current_width > 0 && current_width + ws_len <= max_width {
                    current_line.push_str(&ws);
                    current_width += ws_len;
                }
            }
            Token::Word(word) => {
                let word_width = visible_width(&word);

                // Word fits on current line
                if current_width + word_width <= max_width {
                    current_line.push_str(&word);
                    current_width += word_width;
                } else if current_width == 0 && word_width > max_width {
                    // Word is longer than max_width and line is empty: hard wrap character by character
                    let mut char_buf = String::new();
                    let mut char_width = 0;

                    for c in word.chars() {
                        let c_w = if is_zero_width(c) {
                            0
                        } else if is_wide_char(c) {
                            2
                        } else {
                            1
                        };

                        if char_width + c_w > max_width && !char_buf.is_empty() {
                            if !active_ansi.is_empty() {
                                char_buf.push_str("\x1b[0m");
                            }
                            lines.push(char_buf);
                            char_buf = active_ansi.clone();
                            char_width = 0;
                        }

                        char_buf.push(c);
                        char_width += c_w;
                    }

                    if !char_buf.is_empty() {
                        current_line = char_buf;
                        current_width = char_width;
                    }
                } else {
                    // Flush current line and start a new line with this word
                    if !active_ansi.is_empty() {
                        current_line.push_str("\x1b[0m");
                    }
                    lines.push(current_line);

                    // Start new line with carried-over ANSI styles
                    current_line = active_ansi.clone();

                    if word_width <= max_width {
                        current_line.push_str(&word);
                        current_width = word_width;
                    } else {
                        // Word exceeds full line width on new line: break character by character
                        let mut char_buf = current_line;
                        let mut char_width = 0;

                        for c in word.chars() {
                            let c_w = if is_zero_width(c) {
                                0
                            } else if is_wide_char(c) {
                                2
                            } else {
                                1
                            };

                            if char_width + c_w > max_width && !char_buf.is_empty() {
                                if !active_ansi.is_empty() {
                                    char_buf.push_str("\x1b[0m");
                                }
                                lines.push(char_buf);
                                char_buf = active_ansi.clone();
                                char_width = 0;
                            }

                            char_buf.push(c);
                            char_width += c_w;
                        }

                        current_line = char_buf;
                        current_width = char_width;
                    }
                }
            }
        }
    }

    if !current_line.is_empty() || lines.is_empty() {
        if !active_ansi.is_empty() && !current_line.ends_with("\x1b[0m") {
            current_line.push_str("\x1b[0m");
        }
        lines.push(current_line);
    }

    lines
}

#[derive(Debug, PartialEq, Eq)]
enum Token {
    Word(String),
    Whitespace(String),
    Ansi(String),
}

fn tokenize_text(s: &str) -> Vec<Token> {
    let mut tokens = Vec::new();
    let mut chars = s.chars().peekable();
    let mut current_word = String::new();
    let mut current_ws = String::new();

    while let Some(c) = chars.next() {
        if c == '\x1b' {
            if !current_word.is_empty() {
                tokens.push(Token::Word(std::mem::take(&mut current_word)));
            }
            if !current_ws.is_empty() {
                tokens.push(Token::Whitespace(std::mem::take(&mut current_ws)));
            }

            let mut ansi = String::from("\x1b");
            if let Some(&next_c) = chars.peek() {
                if next_c == '[' {
                    ansi.push(chars.next().unwrap());
                    while let Some(seq_c) = chars.next() {
                        ansi.push(seq_c);
                        if ('@'..='~').contains(&seq_c) {
                            break;
                        }
                    }
                } else if next_c == ']' {
                    ansi.push(chars.next().unwrap());
                    while let Some(seq_c) = chars.next() {
                        ansi.push(seq_c);
                        if seq_c == '\x07' {
                            break;
                        }
                        if seq_c == '\x1b' && chars.peek() == Some(&'\\') {
                            ansi.push(chars.next().unwrap());
                            break;
                        }
                    }
                } else {
                    ansi.push(chars.next().unwrap());
                }
            }
            tokens.push(Token::Ansi(ansi));
        } else if c.is_whitespace() {
            if !current_word.is_empty() {
                tokens.push(Token::Word(std::mem::take(&mut current_word)));
            }
            current_ws.push(c);
        } else {
            if !current_ws.is_empty() {
                tokens.push(Token::Whitespace(std::mem::take(&mut current_ws)));
            }
            current_word.push(c);
        }
    }

    if !current_word.is_empty() {
        tokens.push(Token::Word(current_word));
    }
    if !current_ws.is_empty() {
        tokens.push(Token::Whitespace(current_ws));
    }

    tokens
}

/// Column horizontal alignment.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ColumnAlign {
    #[default]
    Left,
    Center,
    Right,
}

impl ColumnAlign {
    /// Parse alignment from a markdown delimiter cell (e.g. `---`, `:---`, `:---:`, `---:`).
    pub fn from_delimiter(cell: &str) -> Self {
        let trimmed = cell.trim();
        let left = trimmed.starts_with(':');
        let right = trimmed.ends_with(':');

        match (left, right) {
            (true, true) => ColumnAlign::Center,
            (false, true) => ColumnAlign::Right,
            _ => ColumnAlign::Left,
        }
    }
}

/// Table border drawing character set.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TableBorderStyle {
    pub top_left: &'static str,
    pub top_mid: &'static str,
    pub top_right: &'static str,
    pub mid_left: &'static str,
    pub mid_mid: &'static str,
    pub mid_right: &'static str,
    pub bottom_left: &'static str,
    pub bottom_mid: &'static str,
    pub bottom_right: &'static str,
    pub horizontal: &'static str,
    pub vertical: &'static str,
}

impl TableBorderStyle {
    /// Rounded corner borders (modern terminal style).
    pub const ROUNDED: Self = Self {
        top_left: "╭",
        top_mid: "┬",
        top_right: "╮",
        mid_left: "├",
        mid_mid: "┼",
        mid_right: "┤",
        bottom_left: "╰",
        bottom_mid: "┴",
        bottom_right: "╯",
        horizontal: "─",
        vertical: "│",
    };

    /// Standard light box borders.
    pub const LIGHT: Self = Self {
        top_left: "┌",
        top_mid: "┬",
        top_right: "┐",
        mid_left: "├",
        mid_mid: "┼",
        mid_right: "┤",
        bottom_left: "└",
        bottom_mid: "┴",
        bottom_right: "┘",
        horizontal: "─",
        vertical: "│",
    };

    /// Heavy box borders.
    pub const HEAVY: Self = Self {
        top_left: "┏",
        top_mid: "┳",
        top_right: "┓",
        mid_left: "┣",
        mid_mid: "╋",
        mid_right: "┫",
        bottom_left: "┗",
        bottom_mid: "┻",
        bottom_right: "┛",
        horizontal: "━",
        vertical: "┃",
    };

    /// Double line borders.
    pub const DOUBLE: Self = Self {
        top_left: "╔",
        top_mid: "╦",
        top_right: "╗",
        mid_left: "╠",
        mid_mid: "╬",
        mid_right: "╣",
        bottom_left: "╚",
        bottom_mid: "╩",
        bottom_right: "╝",
        horizontal: "═",
        vertical: "║",
    };

    /// Standard ASCII borders for simple or legacy terminals.
    pub const ASCII: Self = Self {
        top_left: "+",
        top_mid: "+",
        top_right: "+",
        mid_left: "+",
        mid_mid: "+",
        mid_right: "+",
        bottom_left: "+",
        bottom_mid: "+",
        bottom_right: "+",
        horizontal: "-",
        vertical: "|",
    };

    /// Markdown-compatible border style.
    pub const MARKDOWN: Self = Self {
        top_left: "|",
        top_mid: "|",
        top_right: "|",
        mid_left: "|",
        mid_mid: "|",
        mid_right: "|",
        bottom_left: "|",
        bottom_mid: "|",
        bottom_right: "|",
        horizontal: "-",
        vertical: "|",
    };
}

impl Default for TableBorderStyle {
    fn default() -> Self {
        Self::LIGHT
    }
}

/// ANSI styling theme for table borders and text elements.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TableTheme {
    /// Border ANSI styling sequence (default: muted gray).
    pub border_color: String,
    /// Header text ANSI styling sequence (default: bold white).
    pub header_style: String,
    /// Regular cell text ANSI styling sequence (default: none / reset).
    pub cell_style: String,
    /// Card title ANSI styling sequence for narrow mobile views.
    pub card_title_style: String,
}

impl Default for TableTheme {
    fn default() -> Self {
        Self {
            border_color: "\x1b[38;5;240m".to_string(),
            header_style: "\x1b[1;37m".to_string(),
            cell_style: "\x1b[0m".to_string(),
            card_title_style: "\x1b[1;36m".to_string(),
        }
    }
}

/// Dynamic column auto-sizer that calculates optimal column widths
/// while respecting terminal boundaries and proportional content length.
#[derive(Debug, Clone)]
pub struct ColumnAutoSizer {
    /// Maximum available table width in columns.
    pub terminal_width: usize,
    /// Minimum width for any column content.
    pub min_col_width: usize,
    /// Horizontal padding per cell (left and right).
    pub padding: usize,
    /// Whether border characters are rendered.
    pub has_borders: bool,
}

impl ColumnAutoSizer {
    /// Create a new auto-sizer with the given terminal width constraint.
    pub fn new(terminal_width: usize) -> Self {
        Self {
            terminal_width,
            min_col_width: DEFAULT_MIN_COL_WIDTH,
            padding: DEFAULT_CELL_PADDING,
            has_borders: true,
        }
    }

    /// Set minimum column width.
    pub fn with_min_col_width(mut self, min: usize) -> Self {
        self.min_col_width = min.max(1);
        self
    }

    /// Set padding spaces per cell.
    pub fn with_padding(mut self, padding: usize) -> Self {
        self.padding = padding;
        self
    }

    /// Set whether borders are included.
    pub fn with_borders(mut self, has_borders: bool) -> Self {
        self.has_borders = has_borders;
        self
    }

    /// Calculate optimal column widths for the given headers and data rows.
    pub fn calculate_widths(&self, headers: &[String], rows: &[Vec<String>]) -> Vec<usize> {
        let num_cols = headers
            .len()
            .max(rows.iter().map(|r| r.len()).max().unwrap_or(0));
        if num_cols == 0 {
            return Vec::new();
        }

        // 1. Calculate natural content width for each column
        let mut natural_widths = vec![0; num_cols];

        for (i, h) in headers.iter().enumerate() {
            natural_widths[i] = natural_widths[i].max(visible_width(h));
        }

        for row in rows {
            for (i, cell) in row.iter().enumerate() {
                if i < num_cols {
                    natural_widths[i] = natural_widths[i].max(visible_width(cell));
                }
            }
        }

        // Ensure minimum 1 width naturally
        for w in &mut natural_widths {
            *w = (*w).max(1);
        }

        // 2. Compute border and padding overhead
        // Overhead = (num_cols + 1) borders + (num_cols * 2 * padding)
        let overhead = if self.has_borders {
            (num_cols + 1) + (num_cols * 2 * self.padding)
        } else {
            (num_cols.saturating_sub(1)) + (num_cols * 2 * self.padding)
        };

        // Available width for column contents
        let available_content_width = self.terminal_width.saturating_sub(overhead);

        let total_natural_width: usize = natural_widths.iter().sum();

        // 3. Case A: Total natural width fits within available width!
        // All columns get their exact natural width.
        if total_natural_width <= available_content_width {
            return natural_widths;
        }

        // 4. Case B: Squeeze required.
        // If available width is extremely constrained (e.g. less than 1 char per column),
        // allocate at least 1 char to as many columns as possible.
        if available_content_width < num_cols {
            let mut widths = vec![1; num_cols];
            for i in available_content_width..num_cols {
                widths[i] = 1;
            }
            return widths;
        }

        // Minimum width per column: min(natural_width, min_col_width)
        let min_widths: Vec<usize> = natural_widths
            .iter()
            .map(|&nw| nw.min(self.min_col_width.max(1)))
            .collect();

        let sum_min_widths: usize = min_widths.iter().sum();

        // If even min_widths exceed available content width, scale proportionally down to 1
        if sum_min_widths > available_content_width {
            let mut allocated = vec![1; num_cols];
            let mut remaining = available_content_width.saturating_sub(num_cols);

            // Distribute remaining characters to columns with larger natural widths
            while remaining > 0 {
                let mut best_col = 0;
                let mut best_ratio = 0.0f64;

                for (i, &nw) in natural_widths.iter().enumerate() {
                    let ratio = (nw as f64) / (allocated[i] as f64);
                    if ratio > best_ratio {
                        best_ratio = ratio;
                        best_col = i;
                    }
                }

                allocated[best_col] += 1;
                remaining -= 1;
            }

            return allocated;
        }

        // 5. Fair-share iterative allocation:
        // Short columns (whose natural width fits within the fair share) get their full
        // natural width, leaving maximum room for wider columns to wrap cleanly on word boundaries.
        let mut final_widths = vec![0; num_cols];
        let mut remaining_budget = available_content_width;
        let mut unassigned: Vec<usize> = (0..num_cols).collect();

        loop {
            if unassigned.is_empty() {
                break;
            }

            let fair_share = remaining_budget / unassigned.len();

            // Find columns whose natural width fits within fair_share
            let mut satisfied = Vec::new();
            for &col in &unassigned {
                if natural_widths[col] <= fair_share {
                    satisfied.push(col);
                }
            }

            if !satisfied.is_empty() {
                for col in satisfied {
                    final_widths[col] = natural_widths[col];
                    remaining_budget = remaining_budget.saturating_sub(natural_widths[col]);
                    unassigned.retain(|&c| c != col);
                }
            } else {
                // All remaining columns need more than fair_share.
                // Divide remaining budget among them proportionally to their natural widths.
                let count = unassigned.len();
                let min_needed_per_col = self.min_col_width.max(1);

                if remaining_budget >= count * min_needed_per_col {
                    // Give each at least min_needed_per_col
                    for &col in &unassigned {
                        final_widths[col] = min_needed_per_col.min(natural_widths[col]);
                    }
                    let used: usize = unassigned.iter().map(|&c| final_widths[c]).sum();
                    let extra_budget = remaining_budget.saturating_sub(used);

                    let weights: Vec<usize> = unassigned
                        .iter()
                        .map(|&c| natural_widths[c].saturating_sub(final_widths[c]))
                        .collect();
                    let total_weight: usize = weights.iter().sum();

                    if total_weight > 0 && extra_budget > 0 {
                        for (idx, &col) in unassigned.iter().enumerate() {
                            let add = (extra_budget * weights[idx]) / total_weight;
                            let capped = add.min(weights[idx]);
                            final_widths[col] += capped;
                        }

                        let now_used: usize = unassigned.iter().map(|&c| final_widths[c]).sum();
                        let mut rem = remaining_budget.saturating_sub(now_used);
                        while rem > 0 {
                            let mut added = false;
                            for &col in &unassigned {
                                if final_widths[col] < natural_widths[col] && rem > 0 {
                                    final_widths[col] += 1;
                                    rem -= 1;
                                    added = true;
                                }
                            }
                            if !added {
                                if let Some(&widest) =
                                    unassigned.iter().max_by_key(|&&c| final_widths[c])
                                {
                                    final_widths[widest] += rem;
                                }
                                break;
                            }
                        }
                    }
                } else {
                    // Severely constrained budget: allocate 1 char per column then distribute leftovers
                    for &col in &unassigned {
                        final_widths[col] = 1;
                    }
                    let mut rem = remaining_budget.saturating_sub(count);
                    while rem > 0 {
                        for &col in &unassigned {
                            if rem > 0 {
                                final_widths[col] += 1;
                                rem -= 1;
                            }
                        }
                    }
                }
                break;
            }
        }

        final_widths
    }
}

/// A responsive Markdown table representation with ANSI formatting,
/// dynamic auto-sizing, multi-line cell wrapping, and mobile card fallback.
#[derive(Debug, Clone)]
pub struct Table {
    /// Header row cells.
    pub headers: Vec<String>,
    /// Alignment per column.
    pub alignments: Vec<ColumnAlign>,
    pub rows: Vec<Vec<String>>,
    /// Explicit terminal width constraint (or auto-detected if None).
    pub terminal_width: Option<usize>,
    /// Border glyph style.
    pub border_style: TableBorderStyle,
    /// ANSI theme.
    pub theme: TableTheme,
    /// Minimum width of each column.
    pub min_col_width: usize,
    /// Horizontal padding per cell.
    pub padding: usize,
    /// Enable responsive stacked card fallback on narrow mobile screens.
    pub responsive_card_fallback: bool,
}

impl Default for Table {
    fn default() -> Self {
        Self::new()
    }
}

impl Table {
    /// Create a new empty table.
    pub fn new() -> Self {
        Self {
            headers: Vec::new(),
            alignments: Vec::new(),
            rows: Vec::new(),
            terminal_width: None,
            border_style: TableBorderStyle::default(),
            theme: TableTheme::default(),
            min_col_width: DEFAULT_MIN_COL_WIDTH,
            padding: DEFAULT_CELL_PADDING,
            responsive_card_fallback: true,
        }
    }

    /// Set table headers.
    pub fn with_headers<I, S>(mut self, headers: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.headers = headers.into_iter().map(Into::into).collect();
        self
    }

    /// Set column alignments.
    pub fn with_alignments<I>(mut self, alignments: I) -> Self
    where
        I: IntoIterator<Item = ColumnAlign>,
    {
        self.alignments = alignments.into_iter().collect();
        self
    }

    /// Add a data row.
    pub fn add_row<I, S>(&mut self, row: I)
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.rows.push(row.into_iter().map(Into::into).collect());
    }

    /// Set terminal width constraint.
    pub fn with_terminal_width(mut self, width: usize) -> Self {
        self.terminal_width = Some(width);
        self
    }

    /// Set border style.
    pub fn with_border_style(mut self, style: TableBorderStyle) -> Self {
        self.border_style = style;
        self
    }

    /// Set theme.
    pub fn with_theme(mut self, theme: TableTheme) -> Self {
        self.theme = theme;
        self
    }

    /// Set minimum column width.
    pub fn with_min_col_width(mut self, min: usize) -> Self {
        self.min_col_width = min.max(1);
        self
    }

    /// Set horizontal cell padding.
    pub fn with_padding(mut self, padding: usize) -> Self {
        self.padding = padding;
        self
    }

    /// Enable or disable responsive stacked card fallback for mobile/narrow viewports.
    pub fn with_card_fallback(mut self, enable: bool) -> Self {
        self.responsive_card_fallback = enable;
        self
    }

    /// Parse a complete markdown table from a string.
    pub fn from_markdown(text: &str) -> Option<Self> {
        let lines: Vec<&str> = text.lines().map(|l| l.trim()).collect();
        Self::from_markdown_lines(lines)
    }

    /// Parse a markdown table from an iterator of line strings.
    pub fn from_markdown_lines<'a, I>(lines: I) -> Option<Self>
    where
        I: IntoIterator<Item = &'a str>,
    {
        let mut table = Table::new();
        let mut lines_iter = lines.into_iter().filter(|l| is_markdown_table_line(l));

        // First table line: Header row
        let first_line = lines_iter.next()?;
        table.headers = parse_markdown_table_row(first_line);
        if table.headers.is_empty() {
            return None;
        }

        // Second table line: Delimiter row
        if let Some(second_line) = lines_iter.next() {
            if is_markdown_delimiter_line(second_line) {
                table.alignments = parse_delimiter_row(second_line);
            } else {
                // Not a delimiter: treat as second row
                table.rows.push(parse_markdown_table_row(second_line));
            }
        }

        // Subsequent lines: Data rows
        for line in lines_iter {
            if !is_markdown_delimiter_line(line) {
                table.rows.push(parse_markdown_table_row(line));
            }
        }

        Some(table)
    }

    /// Render the responsive table to an ANSI-formatted string.
    pub fn render(&self) -> String {
        let term_width = self.terminal_width.unwrap_or_else(get_terminal_width);
        let num_cols = self
            .headers
            .len()
            .max(self.rows.iter().map(|r| r.len()).max().unwrap_or(0));

        if num_cols == 0 {
            return String::new();
        }

        // Check if viewport is too narrow for tabular view
        let min_table_width =
            (num_cols + 1) + (num_cols * 2 * self.padding) + (num_cols * self.min_col_width);

        if self.responsive_card_fallback
            && term_width < min_table_width
            && term_width <= NARROW_TERMINAL_THRESHOLD
        {
            self.render_cards(term_width)
        } else {
            self.render_table(term_width)
        }
    }

    /// Render standard responsive table with auto-sized columns and cell wrapping.
    fn render_table(&self, term_width: usize) -> String {
        let num_cols = self
            .headers
            .len()
            .max(self.rows.iter().map(|r| r.len()).max().unwrap_or(0));
        let auto_sizer = ColumnAutoSizer::new(term_width)
            .with_min_col_width(self.min_col_width)
            .with_padding(self.padding)
            .with_borders(true);

        let widths = auto_sizer.calculate_widths(&self.headers, &self.rows);
        if widths.is_empty() {
            return String::new();
        }

        // Alignments: pad to num_cols with default Left
        let mut aligns = self.alignments.clone();
        while aligns.len() < num_cols {
            aligns.push(ColumnAlign::Left);
        }

        let mut out = String::new();
        let b = &self.border_style;
        let pad_str = " ".repeat(self.padding);

        // 1. Top Border
        write!(out, "{}", self.theme.border_color).unwrap();
        write!(out, "{}", b.top_left).unwrap();
        for (i, &w) in widths.iter().enumerate() {
            let col_total_w = w + (self.padding * 2);
            write!(out, "{}", b.horizontal.repeat(col_total_w)).unwrap();
            if i + 1 < widths.len() {
                write!(out, "{}", b.top_mid).unwrap();
            }
        }
        writeln!(out, "{}\x1b[0m", b.top_right).unwrap();

        // 2. Header Row
        if !self.headers.is_empty() {
            let wrapped_headers: Vec<Vec<String>> = (0..num_cols)
                .map(|i| {
                    let text = self.headers.get(i).map(|s| s.as_str()).unwrap_or("");
                    wrap_ansi(text, widths[i])
                })
                .collect();

            let max_sublines = wrapped_headers
                .iter()
                .map(|lines| lines.len())
                .max()
                .unwrap_or(1);

            for subline_idx in 0..max_sublines {
                write!(out, "{}{}\x1b[0m", self.theme.border_color, b.vertical).unwrap();

                for i in 0..num_cols {
                    let cell_line = wrapped_headers[i]
                        .get(subline_idx)
                        .map(|s| s.as_str())
                        .unwrap_or("");
                    let aligned = align_cell(cell_line, widths[i], aligns[i]);

                    write!(out, "{}", pad_str).unwrap();
                    write!(out, "{}{}\x1b[0m", self.theme.header_style, aligned).unwrap();
                    write!(out, "{}", pad_str).unwrap();
                    write!(out, "{}{}\x1b[0m", self.theme.border_color, b.vertical).unwrap();
                }
                writeln!(out).unwrap();
            }

            // Header Separator
            write!(out, "{}", self.theme.border_color).unwrap();
            write!(out, "{}", b.mid_left).unwrap();
            for (i, &w) in widths.iter().enumerate() {
                let col_total_w = w + (self.padding * 2);
                write!(out, "{}", b.horizontal.repeat(col_total_w)).unwrap();
                if i + 1 < widths.len() {
                    write!(out, "{}", b.mid_mid).unwrap();
                }
            }
            writeln!(out, "{}\x1b[0m", b.mid_right).unwrap();
        }

        // 3. Data Rows
        for (row_idx, row) in self.rows.iter().enumerate() {
            let wrapped_cells: Vec<Vec<String>> = (0..num_cols)
                .map(|i| {
                    let text = row.get(i).map(|s| s.as_str()).unwrap_or("");
                    wrap_ansi(text, widths[i])
                })
                .collect();

            let max_sublines = wrapped_cells
                .iter()
                .map(|lines| lines.len())
                .max()
                .unwrap_or(1);

            for subline_idx in 0..max_sublines {
                write!(out, "{}{}\x1b[0m", self.theme.border_color, b.vertical).unwrap();

                for i in 0..num_cols {
                    let cell_line = wrapped_cells[i]
                        .get(subline_idx)
                        .map(|s| s.as_str())
                        .unwrap_or("");
                    let aligned = align_cell(cell_line, widths[i], aligns[i]);

                    write!(out, "{}", pad_str).unwrap();
                    write!(out, "{}{}\x1b[0m", self.theme.cell_style, aligned).unwrap();
                    write!(out, "{}", pad_str).unwrap();
                    write!(out, "{}{}\x1b[0m", self.theme.border_color, b.vertical).unwrap();
                }
                writeln!(out).unwrap();
            }

            // Optional subtle row separator if needed, or just bottom border at the end
            let is_last_row = row_idx + 1 == self.rows.len();
            if is_last_row {
                // Bottom Border
                write!(out, "{}", self.theme.border_color).unwrap();
                write!(out, "{}", b.bottom_left).unwrap();
                for (i, &w) in widths.iter().enumerate() {
                    let col_total_w = w + (self.padding * 2);
                    write!(out, "{}", b.horizontal.repeat(col_total_w)).unwrap();
                    if i + 1 < widths.len() {
                        write!(out, "{}", b.bottom_mid).unwrap();
                    }
                }
                write!(out, "{}\x1b[0m", b.bottom_right).unwrap();
            }
        }

        // Handle case where table has only headers and no rows
        if self.rows.is_empty() && !self.headers.is_empty() {
            write!(out, "{}", self.theme.border_color).unwrap();
            write!(out, "{}", b.bottom_left).unwrap();
            for (i, &w) in widths.iter().enumerate() {
                let col_total_w = w + (self.padding * 2);
                write!(out, "{}", b.horizontal.repeat(col_total_w)).unwrap();
                if i + 1 < widths.len() {
                    write!(out, "{}", b.bottom_mid).unwrap();
                }
            }
            write!(out, "{}\x1b[0m", b.bottom_right).unwrap();
        }

        out
    }

    /// Render responsive stacked card view for mobile/narrow screens (Termux portrait).
    pub fn render_cards(&self, term_width: usize) -> String {
        let mut out = String::new();
        let b = &self.border_style;
        let card_width = term_width.max(20);
        let inner_width = card_width.saturating_sub(4); // 2 borders + 2 padding

        for (idx, row) in self.rows.iter().enumerate() {
            if idx > 0 {
                out.push('\n');
            }

            // Card Header
            let title = format!(" [Item {}/{}] ", idx + 1, self.rows.len());
            let title_len = visible_width(&title);
            let border_rem = inner_width.saturating_sub(title_len);

            write!(out, "{}", self.theme.border_color).unwrap();
            write!(out, "{}", b.top_left).unwrap();
            write!(out, "{}", b.horizontal).unwrap();
            write!(
                out,
                "{}{}\x1b[0m{}",
                self.theme.card_title_style, title, self.theme.border_color
            )
            .unwrap();
            write!(out, "{}", b.horizontal.repeat(border_rem)).unwrap();
            writeln!(out, "{}\x1b[0m", b.top_right).unwrap();

            // Key-Value rows
            for (col_idx, cell) in row.iter().enumerate() {
                let header_name = self
                    .headers
                    .get(col_idx)
                    .cloned()
                    .unwrap_or_else(|| format!("Col {}", col_idx + 1));

                let prefix = format!("{}: ", header_name);
                let prefix_width = visible_width(&prefix);
                let value_avail_width = inner_width.saturating_sub(prefix_width).max(10);

                let wrapped_val = wrap_ansi(cell, value_avail_width);

                for (val_line_idx, val_line) in wrapped_val.iter().enumerate() {
                    write!(out, "{}{}\x1b[0m ", self.theme.border_color, b.vertical).unwrap();

                    if val_line_idx == 0 {
                        write!(out, "{}{}\x1b[0m", self.theme.header_style, prefix).unwrap();
                        let pad_len = value_avail_width.saturating_sub(visible_width(val_line));
                        write!(out, "{}{}", val_line, " ".repeat(pad_len)).unwrap();
                    } else {
                        let indent = " ".repeat(prefix_width);
                        let pad_len = value_avail_width.saturating_sub(visible_width(val_line));
                        write!(out, "{}{}{}", indent, val_line, " ".repeat(pad_len)).unwrap();
                    }

                    writeln!(out, " {}{}\x1b[0m", self.theme.border_color, b.vertical).unwrap();
                }
            }

            // Card Bottom
            write!(out, "{}", self.theme.border_color).unwrap();
            write!(out, "{}", b.bottom_left).unwrap();
            write!(out, "{}", b.horizontal.repeat(inner_width + 2)).unwrap();
            write!(out, "{}\x1b[0m", b.bottom_right).unwrap();
        }

        out
    }
}

/// Aligns a single line of text within `target_width` columns.
fn align_cell(text: &str, target_width: usize, align: ColumnAlign) -> String {
    let vis_w = visible_width(text);
    if vis_w >= target_width {
        return text.to_string();
    }

    let diff = target_width - vis_w;
    match align {
        ColumnAlign::Left => {
            format!("{}{}", text, " ".repeat(diff))
        }
        ColumnAlign::Right => {
            format!("{}{}", " ".repeat(diff), text)
        }
        ColumnAlign::Center => {
            let left_pad = diff / 2;
            let right_pad = diff - left_pad;
            format!("{}{}{}", " ".repeat(left_pad), text, " ".repeat(right_pad))
        }
    }
}

/// Helper to parse a markdown table row into cells, handling escaped pipes `\|`.
pub fn parse_markdown_table_row(line: &str) -> Vec<String> {
    let trimmed = line.trim();
    let content = if trimmed.starts_with('|') && trimmed.ends_with('|') && trimmed.len() >= 2 {
        &trimmed[1..trimmed.len() - 1]
    } else {
        trimmed
    };

    let mut cells = Vec::new();
    let mut current = String::new();
    let mut chars = content.chars().peekable();

    while let Some(c) = chars.next() {
        if c == '\\' {
            if let Some(&next_c) = chars.peek() {
                if next_c == '|' {
                    current.push('|');
                    chars.next();
                    continue;
                }
            }
            current.push('\\');
        } else if c == '|' {
            cells.push(current.trim().to_string());
            current.clear();
        } else {
            current.push(c);
        }
    }

    cells.push(current.trim().to_string());
    cells
}

/// Checks if a string looks like a markdown table line.
pub fn is_markdown_table_line(line: &str) -> bool {
    let trimmed = line.trim();
    if trimmed.is_empty() {
        return false;
    }
    // Require leading and trailing pipe or at least 2 pipes
    if trimmed.starts_with('|') && trimmed.ends_with('|') && trimmed.len() >= 2 {
        return true;
    }
    trimmed.chars().filter(|&c| c == '|').count() >= 2
}

/// Checks if a string is a markdown delimiter line (e.g. `|---|---|` or `|:---:|---:|`).
pub fn is_markdown_delimiter_line(line: &str) -> bool {
    if !is_markdown_table_line(line) {
        return false;
    }

    let cells = parse_markdown_table_row(line);
    if cells.is_empty() {
        return false;
    }

    cells.iter().all(|c| {
        let t = c.trim();
        !t.is_empty()
            && t.chars()
                .all(|ch| ch == '-' || ch == ':' || ch.is_whitespace())
    })
}

/// Parse column alignments from a markdown delimiter row.
pub fn parse_delimiter_row(line: &str) -> Vec<ColumnAlign> {
    parse_markdown_table_row(line)
        .iter()
        .map(|c| ColumnAlign::from_delimiter(c))
        .collect()
}

/// Streaming parser and formatter for markdown tables.
///
/// Buffers incoming table lines from an LLM or markdown stream, then formats
/// and emits the responsive table with optimal dynamic column sizing.
#[derive(Debug, Clone, Default)]
pub struct MarkdownTableStreamer {
    buffered_lines: Vec<String>,
    terminal_width: Option<usize>,
    border_style: TableBorderStyle,
    theme: TableTheme,
}

impl MarkdownTableStreamer {
    /// Create a new table streamer.
    pub fn new() -> Self {
        Self {
            buffered_lines: Vec::new(),
            terminal_width: None,
            border_style: TableBorderStyle::default(),
            theme: TableTheme::default(),
        }
    }

    /// Set explicit terminal width constraint.
    pub fn with_terminal_width(mut self, width: usize) -> Self {
        self.terminal_width = Some(width);
        self
    }

    /// Set border style.
    pub fn with_border_style(mut self, style: TableBorderStyle) -> Self {
        self.border_style = style;
        self
    }

    /// Set theme.
    pub fn with_theme(mut self, theme: TableTheme) -> Self {
        self.theme = theme;
        self
    }

    /// Feed a line into the table streamer.
    ///
    /// - If the line is a table line, buffers it and returns `None`.
    /// - If the line is a non-table line, and lines were previously buffered,
    ///   flushes and returns `Some(rendered_table)`.
    /// - If no lines were buffered, returns `None`.
    pub fn feed_line(&mut self, line: &str) -> Option<String> {
        if is_markdown_table_line(line) {
            self.buffered_lines.push(line.to_string());
            None
        } else if !self.buffered_lines.is_empty() {
            let rendered = self.flush();
            Some(rendered)
        } else {
            None
        }
    }

    /// Flush and render all buffered table lines into a formatted ANSI table.
    pub fn flush(&mut self) -> String {
        if self.buffered_lines.is_empty() {
            return String::new();
        }

        let lines = std::mem::take(&mut self.buffered_lines);
        if let Some(table) = Table::from_markdown_lines(lines.iter().map(|s| s.as_str())) {
            let mut t = table
                .with_border_style(self.border_style)
                .with_theme(self.theme.clone());
            if let Some(w) = self.terminal_width {
                t = t.with_terminal_width(w);
            }
            t.render()
        } else {
            lines.join("\n")
        }
    }

    /// Check if currently buffering table rows.
    pub fn is_buffering(&self) -> bool {
        !self.buffered_lines.is_empty()
    }

    /// Number of buffered rows.
    pub fn buffered_count(&self) -> usize {
        self.buffered_lines.len()
    }
}

/// Format a complete markdown table string with automatic terminal width detection.
pub fn render_markdown_table(markdown: &str) -> String {
    let width = get_terminal_width();
    render_markdown_table_with_width(markdown, width)
}

/// Format a complete markdown table string constrained to the specified terminal width.
pub fn render_markdown_table_with_width(markdown: &str, terminal_width: usize) -> String {
    if let Some(table) = Table::from_markdown(markdown) {
        table.with_terminal_width(terminal_width).render()
    } else {
        markdown.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_visible_width_plain() {
        assert_eq!(visible_width("hello"), 5);
        assert_eq!(visible_width("foo bar baz"), 11);
        assert_eq!(visible_width(""), 0);
    }

    #[test]
    fn test_visible_width_ansi() {
        assert_eq!(visible_width("\x1b[32mhello\x1b[0m"), 5);
        assert_eq!(visible_width("\x1b[1;36m# Header\x1b[0m"), 8);
        assert_eq!(visible_width("\x1b[38;5;240m│\x1b[0m"), 1);
        assert_eq!(visible_width("\x1b]0;Title\x07hello"), 5);
    }

    #[test]
    fn test_visible_width_cjk_and_emojis() {
        assert_eq!(visible_width("你好"), 4);
        assert_eq!(visible_width("Rust 🦀"), 7); // 5 ASCII + 2 emoji
    }

    #[test]
    fn test_visible_width_zero_width() {
        assert_eq!(visible_width("test\u{200B}"), 4);
        assert_eq!(visible_width("e\u{0301}"), 1); // e + acute accent
    }

    #[test]
    fn test_strip_ansi() {
        assert_eq!(strip_ansi("\x1b[1;32mSuccess\x1b[0m"), "Success");
        assert_eq!(strip_ansi("No ANSI"), "No ANSI");
    }

    #[test]
    fn test_truncate_ansi() {
        let s = "\x1b[32mHello World\x1b[0m";
        let truncated = truncate_ansi(s, 8, "...");
        assert!(visible_width(&truncated) <= 8);
        assert!(truncated.contains("..."));
        assert!(truncated.ends_with("\x1b[0m"));
    }

    #[test]
    fn test_wrap_ansi_plain() {
        let text = "The quick brown fox jumps over the lazy dog";
        let wrapped = wrap_ansi(text, 15);
        assert!(wrapped.len() >= 3);
        for line in &wrapped {
            assert!(
                visible_width(line) <= 15,
                "Line '{}' exceeds 15 width ({})",
                line,
                visible_width(line)
            );
        }
    }

    #[test]
    fn test_wrap_ansi_with_formatting() {
        let text = "\x1b[33mSupercalifragilisticexpialidocious\x1b[0m";
        let wrapped = wrap_ansi(text, 10);
        assert!(wrapped.len() > 1);
        for line in &wrapped {
            assert!(visible_width(line) <= 10);
        }
    }

    #[test]
    fn test_column_align_parsing() {
        assert_eq!(ColumnAlign::from_delimiter("---"), ColumnAlign::Left);
        assert_eq!(ColumnAlign::from_delimiter(":---"), ColumnAlign::Left);
        assert_eq!(ColumnAlign::from_delimiter(":---:"), ColumnAlign::Center);
        assert_eq!(ColumnAlign::from_delimiter("---:"), ColumnAlign::Right);
    }

    #[test]
    fn test_auto_sizer_wide_screen() {
        let headers = vec!["ID".to_string(), "Name".to_string(), "Role".to_string()];
        let rows = vec![
            vec!["1".to_string(), "Alice".to_string(), "Engineer".to_string()],
            vec!["2".to_string(), "Bob".to_string(), "Designer".to_string()],
        ];

        let sizer = ColumnAutoSizer::new(80);
        let widths = sizer.calculate_widths(&headers, &rows);

        assert_eq!(widths.len(), 3);
        assert_eq!(widths[0], 2); // "ID"
        assert_eq!(widths[1], 5); // "Alice"
        assert_eq!(widths[2], 8); // "Engineer"
    }

    #[test]
    fn test_auto_sizer_narrow_screen_squeeze() {
        let headers = vec!["Feature".to_string(), "Description".to_string()];
        let rows = vec![vec![
            "Fusion Engine".to_string(),
            "A very long description that definitely exceeds small width".to_string(),
        ]];

        // Narrow screen of 35 columns
        let sizer = ColumnAutoSizer::new(35);
        let widths = sizer.calculate_widths(&headers, &rows);

        assert_eq!(widths.len(), 2);
        // Total table width = (w1 + w2) + 3 borders + 4 padding = w1 + w2 + 7 <= 35
        let total_table_w = widths.iter().sum::<usize>() + 7;
        assert!(
            total_table_w <= 35,
            "Total table width {} exceeded 35",
            total_table_w
        );
        assert!(widths[0] >= 3);
        assert!(widths[1] >= 3);
    }

    #[test]
    fn test_render_table_no_wrapping_glitches() {
        let md = r#"
| Col A | Col B | Col C |
| :--- | :---: | ---: |
| Hello | World | 123 |
| Short | Long descriptive cell text that must wrap cleanly inside the cell | 456 |
"#;

        let table = Table::from_markdown(md).unwrap().with_terminal_width(50);
        let rendered = table.render();

        for line in rendered.lines() {
            let vis_w = visible_width(line);
            assert!(
                vis_w <= 50,
                "Line '{}' has visible width {} > 50",
                line,
                vis_w
            );
        }

        assert!(rendered.contains("Hello"));
        assert!(rendered.contains("World"));
        assert!(rendered.contains("│"));
    }

    #[test]
    fn test_mobile_card_fallback() {
        let md = r#"
| Name | Age | City | Country | Occupation |
| --- | --- | --- | --- | --- |
| Alice | 30 | London | UK | Architect |
"#;

        // Terminal width 30 is too narrow for 5 columns in tabular mode
        let table = Table::from_markdown(md)
            .unwrap()
            .with_terminal_width(30)
            .with_card_fallback(true);

        let rendered = table.render();
        assert!(rendered.contains("Alice"));
        assert!(rendered.contains("London"));
        assert!(rendered.contains("Item 1/1"));

        for line in rendered.lines() {
            let vis_w = visible_width(line);
            assert!(
                vis_w <= 30,
                "Card line '{}' has visible width {} > 30",
                line,
                vis_w
            );
        }
    }

    #[test]
    fn test_markdown_table_streamer() {
        let mut streamer = MarkdownTableStreamer::new().with_terminal_width(60);

        assert_eq!(streamer.feed_line("| Name | Status |"), None);
        assert_eq!(streamer.feed_line("|---|---|"), None);
        assert_eq!(streamer.feed_line("| Service A | Running |"), None);
        assert!(streamer.is_buffering());

        // Feed non-table line to flush
        let flushed = streamer.feed_line("Some normal paragraph text").unwrap();
        assert!(!streamer.is_buffering());
        assert!(flushed.contains("Service A"));
        assert!(flushed.contains("Running"));
    }
    #[test]
    fn test_escaped_pipes() {
        let md = r#"
| Command | Explanation |
|---|---|
| `cat file \| grep foo` | Pipes output through grep |
"#;
        let table = Table::from_markdown(md).unwrap().with_terminal_width(60);
        let rendered = table.render();
        assert!(rendered.contains("cat file | grep foo"));
        assert!(rendered.contains("Pipes output"));
    }

    #[test]
    fn test_differing_column_counts() {
        let md = r#"
| A | B | C |
|---|---|---|
| 1 | 2 |
| x | y | z | extra |
"#;
        let table = Table::from_markdown(md).unwrap().with_terminal_width(50);
        let rendered = table.render();
        assert!(rendered.contains("1"));
        assert!(rendered.contains("2"));
        assert!(rendered.contains("x"));
        assert!(rendered.contains("extra"));
    }

    #[test]
    fn test_unicode_and_emojis_in_table() {
        let md = r#"
| Status | Symbol | Description |
|---|---|---|
| Running | 🚀 | Fast startup with Tokio |
| Testing | 🧪 | Unit and integration matrix |
| Rust | 🦀 | Pure memory safe architecture |
"#;
        let table = Table::from_markdown(md).unwrap().with_terminal_width(50);
        let rendered = table.render();
        assert!(rendered.contains("🚀"));
        assert!(rendered.contains("🧪"));
        assert!(rendered.contains("🦀"));
        assert!(rendered.contains("Running"));

        for line in rendered.lines() {
            let vis_w = visible_width(line);
            assert!(vis_w <= 50, "Line '{}' width {} > 50", line, vis_w);
        }
    }

    #[test]
    fn test_table_border_styles() {
        let md = r#"
| Key | Value |
|---|---|
| port | 8080 |
"#;
        let table_rounded = Table::from_markdown(md)
            .unwrap()
            .with_border_style(TableBorderStyle::ROUNDED);
        let out_rounded = table_rounded.render();
        assert!(out_rounded.contains("╭"));
        assert!(out_rounded.contains("╰"));

        let table_ascii = Table::from_markdown(md)
            .unwrap()
            .with_border_style(TableBorderStyle::ASCII);
        let out_ascii = table_ascii.render();
        assert!(out_ascii.contains("+"));
        assert!(out_ascii.contains("-"));
        assert!(out_ascii.contains("|"));
    }
}
