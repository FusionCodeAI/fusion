use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::io::{BufRead, Write};

use crate::tools::types::{Tool, ToolContext};

/// An option/choice presented to the user.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct QuestionOption {
    /// Display label for this choice.
    pub label: String,
    /// Optional detailed explanation or rationale.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

impl QuestionOption {
    /// Creates a new option with the given label.
    pub fn new(label: impl Into<String>) -> Self {
        Self {
            label: label.into(),
            description: None,
        }
    }

    /// Attaches an optional description to this option.
    pub fn with_description(mut self, description: impl Into<String>) -> Self {
        self.description = Some(description.into());
        self
    }
}

/// A structured question to prompt the user with.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Question {
    /// Unique identifier for this question.
    pub id: String,
    /// The question text to display.
    pub question: String,
    /// Available choices for the user to select from.
    pub options: Vec<QuestionOption>,
    /// Whether multiple choices can be selected (default: false).
    #[serde(default)]
    pub multi: bool,
    /// Optional zero-based index of the recommended option.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub recommended: Option<usize>,
}

/// Interactive `ask` tool for decision making, architectural choices, or confirmation.
#[derive(Debug, Clone, Default)]
pub struct AskTool {
    /// Override interactive mode: `Some(true)` forces interactive, `Some(false)` forces headless/non-interactive, `None` auto-detects.
    interactive_override: Option<bool>,
}

impl AskTool {
    /// Creates a new `AskTool` with auto-detected interactive/headless mode.
    pub fn new() -> Self {
        Self {
            interactive_override: None,
        }
    }

    /// Creates an `AskTool` with an explicit interactive mode setting (useful for tests).
    pub fn with_interactive(mut self, interactive: bool) -> Self {
        self.interactive_override = Some(interactive);
        self
    }

    /// Determines whether interactive prompt mode should be used.
    pub fn is_interactive(&self) -> bool {
        if let Some(forced) = self.interactive_override {
            return forced;
        }
        is_interactive_terminal()
    }

    /// Executes the tool in headless / non-interactive mode.
    pub fn execute_headless(&self, questions: &[Question]) -> anyhow::Result<String> {
        if questions.is_empty() {
            anyhow::bail!("No questions provided to ask tool.");
        }

        let mut selections = Vec::new();
        for q in questions {
            if q.options.is_empty() {
                anyhow::bail!("Question '{}' must contain at least one option.", q.id);
            }

            let def_idx = q.recommended.unwrap_or(0);
            let chosen = q
                .options
                .get(def_idx)
                .or_else(|| q.options.first())
                .unwrap();
            selections.push((q.id.as_str(), chosen.label.as_str()));
        }

        if selections.len() == 1 {
            Ok(format!("Selected option: {}", selections[0].1))
        } else {
            let lines: Vec<String> = selections
                .iter()
                .map(|(id, label)| format!("{}: Selected option: {}", id, label))
                .collect();
            Ok(lines.join("\n"))
        }
    }

    /// Prompts a single question interactively using the given reader and writer.
    pub fn prompt_question<R: BufRead, W: Write>(
        &self,
        q: &Question,
        reader: &mut R,
        writer: &mut W,
    ) -> anyhow::Result<String> {
        if q.options.is_empty() {
            anyhow::bail!("Question '{}' must contain at least one option.", q.id);
        }

        render_question_prompt(writer, q)?;
        read_user_choice(reader, q)
    }

    /// Executes all questions interactively using the provided reader and writer.
    pub fn execute_interactive_with_io<R: BufRead, W: Write>(
        &self,
        questions: &[Question],
        reader: &mut R,
        writer: &mut W,
    ) -> anyhow::Result<String> {
        if questions.is_empty() {
            anyhow::bail!("No questions provided to ask tool.");
        }

        let mut results = Vec::new();
        for q in questions {
            let choice = self.prompt_question(q, reader, writer)?;
            results.push((q.id.as_str(), choice));
        }

        if results.len() == 1 {
            Ok(format!("Selected option: {}", results[0].1))
        } else {
            let lines: Vec<String> = results
                .iter()
                .map(|(id, choice)| format!("{}: Selected option: {}", id, choice))
                .collect();
            Ok(lines.join("\n"))
        }
    }
}

/// Executes all questions interactively via inline Crossterm/Ratatui picker.
pub fn execute_interactive_tui(questions: &[Question]) -> anyhow::Result<String> {
    if questions.is_empty() {
        anyhow::bail!("No questions provided to ask tool.");
    }

    let mut results = Vec::new();
    for q in questions {
        let choice = prompt_question_tui(q)?;
        results.push((q.id.as_str(), choice));
    }

    if results.len() == 1 {
        Ok(format!("Selected option: {}", results[0].1))
    } else {
        let lines: Vec<String> = results
            .iter()
            .map(|(id, choice)| format!("{}: Selected option: {}", id, choice))
            .collect();
        Ok(lines.join("\n"))
    }
}

/// Renders an interactive TUI selection popup for a single question.
pub fn prompt_question_tui(q: &Question) -> anyhow::Result<String> {
    use crossterm::{
        cursor,
        event::{self, Event, KeyCode, KeyEventKind, KeyModifiers},
        execute,
    };
    use std::io::{stdout, Write};

    let mut out = stdout();
    let _ = write!(out, "\r\x1b[2K");
    let _ = out.flush();

    let _guard = crate::ui::prompt::RawModeGuard::enter()?;
    let _ = execute!(out, cursor::Hide);

    let num_opts = q.options.len();
    let mut selected_idx = q.recommended.unwrap_or(0).min(num_opts.saturating_sub(1));
    let mut checked: Vec<bool> = vec![false; num_opts];
    if q.multi {
        if let Some(r) = q.recommended {
            if r < num_opts {
                checked[r] = true;
            }
        }
    }

    let mut last_rendered_lines: usize = 0;
    let term_width = crate::ui::table::get_terminal_width().max(40);
    let menu_width = term_width.saturating_sub(4).min(84);
    let divider = "─".repeat(menu_width);

    loop {
        // 1. Clear previous rendered lines
        if last_rendered_lines > 0 {
            for _ in 0..last_rendered_lines {
                let _ = execute!(out, cursor::MoveToPreviousLine(1), cursor::MoveToColumn(0));
                let _ = write!(out, "\x1b[2K");
            }
            let _ = out.flush();
        }

        // 2. Render frame in slash command dropdown style
        let mut lines = Vec::new();

        // Top divider
        lines.push(format!("\x1b[38;5;240m{}\x1b[0m", divider));

        // Question header
        lines.push(format!("\x1b[1;36m? \x1b[1;37m{}\x1b[0m", q.question.trim()));
        lines.push(String::new());

        // Options
        for (idx, opt) in q.options.iter().enumerate() {
            let is_curr = idx == selected_idx;
            let is_rec = q.recommended == Some(idx);
            let rec_chip = if is_rec { " \x1b[1;32m(Recommended)\x1b[0m" } else { "" };

            let marker = if q.multi {
                let check_mark = if checked[idx] { "✓" } else { " " };
                format!("[{}]", check_mark)
            } else {
                if is_curr {
                    "(•)".to_string()
                } else {
                    "( )".to_string()
                }
            };

            if is_curr {
                lines.push(format!(
                    "\x1b[1;36m┃\x1b[0m \x1b[1;37m{} {}\x1b[0m{}",
                    marker, opt.label, rec_chip
                ));
            } else {
                lines.push(format!(
                    "  \x1b[37m{} {}\x1b[0m{}",
                    marker, opt.label, rec_chip
                ));
            }

            // Description indented below
            if let Some(desc) = &opt.description {
                let d_clean = desc.trim();
                if !d_clean.is_empty() {
                    lines.push(format!("    \x1b[2;37m{}\x1b[0m", d_clean));
                }
            }
        }

        // Bottom divider
        lines.push(format!("\x1b[38;5;240m{}\x1b[0m", divider));

        // Footer key hints
        let hint_text = if q.multi {
            "↑↓ Navigate · Space Toggle · Enter Select · Esc Default · Ctrl+C Cancel"
        } else {
            "↑↓ Navigate · Enter Select · Esc Default · Ctrl+C Cancel"
        };
        lines.push(format!("\x1b[2;37m{}\x1b[0m", hint_text));

        last_rendered_lines = lines.len();
        for l in &lines {
            let _ = write!(out, "\r\x1b[2K{}\r\n", l);
        }
        let _ = out.flush();

        // 3. Read key
        if let Event::Key(key) = event::read()? {
            if key.kind == KeyEventKind::Press {
                if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
                    for _ in 0..last_rendered_lines {
                        let _ = execute!(out, cursor::MoveToPreviousLine(1), cursor::MoveToColumn(0));
                        let _ = write!(out, "\x1b[2K");
                    }
                    let _ = execute!(out, cursor::Show);
                    let _ = crossterm::terminal::enable_raw_mode();
                    anyhow::bail!("Question prompt cancelled by user (Ctrl+C)");
                }

                match key.code {
                    KeyCode::Up | KeyCode::Char('k') => {
                        selected_idx = selected_idx.saturating_sub(1);
                    }
                    KeyCode::Down | KeyCode::Char('j') => {
                        if selected_idx + 1 < num_opts {
                            selected_idx += 1;
                        }
                    }
                    KeyCode::Char(' ') if q.multi => {
                        checked[selected_idx] = !checked[selected_idx];
                    }
                    KeyCode::Enter => {
                        break;
                    }
                    KeyCode::Esc => {
                        let def_idx = q.recommended.unwrap_or(0).min(num_opts.saturating_sub(1));
                        selected_idx = def_idx;
                        if q.multi {
                            checked = vec![false; num_opts];
                            checked[def_idx] = true;
                        }
                        break;
                    }
                    _ => {}
                }
            }
        }
    }

    // 4. Cleanup interactive frame
    for _ in 0..last_rendered_lines {
        let _ = execute!(out, cursor::MoveToPreviousLine(1), cursor::MoveToColumn(0));
        let _ = write!(out, "\x1b[2K");
    }
    let _ = execute!(out, cursor::Show);

    // 5. Build chosen answer & print clean confirmation
    let result_str = if q.multi {
        let mut chosen: Vec<String> = Vec::new();
        for (i, &chk) in checked.iter().enumerate() {
            if chk {
                chosen.push(q.options[i].label.clone());
            }
        }
        if chosen.is_empty() {
            q.options[selected_idx].label.clone()
        } else {
            chosen.join(", ")
        }
    } else {
        q.options[selected_idx].label.clone()
    };

    let _ = write!(
        out,
        "\r\x1b[2K\x1b[1;32m✓ Selected:\x1b[0m \x1b[1;37m{}\x1b[0m\r\n\r\n",
        result_str
    );
    let _ = out.flush();

    // 6. Ensure raw mode stays enabled for outer repl.rs so Esc immediately cancels the turn
    let _ = crossterm::terminal::enable_raw_mode();

    Ok(result_str)
}

/// Checks whether the current process is running in an interactive terminal.
pub fn is_interactive_terminal() -> bool {
    if std::env::var("CI").is_ok() {
        return false;
    }
    if std::env::var("FUSION_NON_INTERACTIVE").is_ok()
        || std::env::var("NON_INTERACTIVE").is_ok()
        || std::env::var("HEADLESS").is_ok()
    {
        return false;
    }

    #[cfg(test)]
    {
        false
    }

    #[cfg(not(test))]
    {
        use std::io::IsTerminal;
        std::io::stdin().is_terminal() && std::io::stdout().is_terminal()
    }
}

/// Renders question and choices with `(Recommended)` chips into the given writer.
pub fn render_question_prompt<W: Write>(writer: &mut W, q: &Question) -> std::io::Result<()> {
    writeln!(writer, "\n━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━")?;
    writeln!(writer, "❓ {}", q.question)?;
    writeln!(writer, "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━")?;

    for (idx, opt) in q.options.iter().enumerate() {
        let is_recommended = q.recommended == Some(idx);
        let chip = if is_recommended {
            " (Recommended)"
        } else {
            ""
        };
        writeln!(writer, "  [{}] {}{}", idx + 1, opt.label, chip)?;
        if let Some(desc) = &opt.description {
            if !desc.trim().is_empty() {
                writeln!(writer, "      {}", desc.trim())?;
            }
        }
    }

    let def_idx = q.recommended.unwrap_or(0);
    if q.multi {
        write!(
            writer,
            "\nSelect option(s) [1-{}, comma-separated] (default: {}): ",
            q.options.len(),
            def_idx + 1
        )?;
    } else {
        write!(
            writer,
            "\nSelect an option [1-{}] (default: {}): ",
            q.options.len(),
            def_idx + 1
        )?;
    }
    writer.flush()?;
    Ok(())
}

/// Reads and resolves user choice from input reader.
pub fn read_user_choice<R: BufRead>(reader: &mut R, q: &Question) -> anyhow::Result<String> {
    let mut line = String::new();
    let bytes_read = reader.read_line(&mut line)?;
    let trimmed = line.trim();

    let def_idx = q.recommended.unwrap_or(0);
    let default_label = q
        .options
        .get(def_idx)
        .or_else(|| q.options.first())
        .map(|o| o.label.clone())
        .unwrap_or_default();

    if bytes_read == 0 || trimmed.is_empty() {
        return Ok(default_label);
    }

    if q.multi {
        let parts: Vec<&str> = if trimmed.contains(',') {
            trimmed
                .split(',')
                .map(|s| s.trim())
                .filter(|s| !s.is_empty())
                .collect()
        } else {
            trimmed.split_whitespace().collect()
        };

        let mut selected_labels = Vec::new();
        for part in parts {
            if let Some(label) = resolve_option(part, &q.options) {
                if !selected_labels.contains(&label) {
                    selected_labels.push(label);
                }
            }
        }

        if selected_labels.is_empty() {
            Ok(default_label)
        } else {
            Ok(selected_labels.join(", "))
        }
    } else {
        if let Some(label) = resolve_option(trimmed, &q.options) {
            Ok(label)
        } else {
            Ok(default_label)
        }
    }
}

/// Matches user input against options by 1-based index, exact name, prefix, or substring.
fn resolve_option(input: &str, options: &[QuestionOption]) -> Option<String> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return None;
    }

    // 1-based index match
    if let Ok(num) = trimmed.parse::<usize>() {
        if num >= 1 && num <= options.len() {
            return Some(options[num - 1].label.clone());
        }
    }

    // Exact match (case-insensitive)
    if let Some(opt) = options
        .iter()
        .find(|o| o.label.eq_ignore_ascii_case(trimmed))
    {
        return Some(opt.label.clone());
    }

    // Prefix match (case-insensitive)
    let lower = trimmed.to_lowercase();
    if let Some(opt) = options
        .iter()
        .find(|o| o.label.to_lowercase().starts_with(&lower))
    {
        return Some(opt.label.clone());
    }

    // Substring match (case-insensitive)
    if let Some(opt) = options
        .iter()
        .find(|o| o.label.to_lowercase().contains(&lower))
    {
        return Some(opt.label.clone());
    }

    None
}

/// Parses questions from tool invocation arguments.
fn parse_questions(args: &Value) -> anyhow::Result<Vec<Question>> {
    if let Some(questions_val) = args.get("questions") {
        let questions: Vec<Question> = serde_json::from_value(questions_val.clone())
            .map_err(|e| anyhow::anyhow!("Failed to parse 'questions' array: {}", e))?;
        return Ok(questions);
    }

    // Fallback: single question object at root level
    if let Some(question_str) = args.get("question").and_then(|v| v.as_str()) {
        let id = args
            .get("id")
            .and_then(|v| v.as_str())
            .unwrap_or("q1")
            .to_string();
        let options: Vec<QuestionOption> = if let Some(opts) = args.get("options") {
            serde_json::from_value(opts.clone())
                .map_err(|e| anyhow::anyhow!("Failed to parse 'options': {}", e))?
        } else {
            anyhow::bail!("Missing 'options' for question '{}'", id);
        };
        let multi = args.get("multi").and_then(|v| v.as_bool()).unwrap_or(false);
        let recommended = args
            .get("recommended")
            .and_then(|v| v.as_u64())
            .map(|v| v as usize);

        return Ok(vec![Question {
            id,
            question: question_str.to_string(),
            options,
            multi,
            recommended,
        }]);
    }

    anyhow::bail!("Missing required parameter: 'questions'")
}

#[async_trait]
impl Tool for AskTool {
    fn name(&self) -> &str {
        "ask"
    }

    fn description(&self) -> &str {
        "Prompts the user with one or more structured questions to resolve architectural decisions, library choices, or destructive action confirmation."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "questions": {
                    "type": "array",
                    "description": "List of questions to prompt the user with.",
                    "items": {
                        "type": "object",
                        "properties": {
                            "id": {
                                "type": "string",
                                "description": "Unique identifier for this question."
                            },
                            "question": {
                                "type": "string",
                                "description": "The question text to display."
                            },
                            "options": {
                                "type": "array",
                                "description": "List of available choices.",
                                "items": {
                                    "type": "object",
                                    "properties": {
                                        "label": {
                                            "type": "string",
                                            "description": "Display label for this option."
                                        },
                                        "description": {
                                            "type": "string",
                                            "description": "Optional explanatory details for this option."
                                        }
                                    },
                                    "required": ["label"]
                                }
                            },
                            "multi": {
                                "type": "boolean",
                                "description": "Whether multiple choices can be selected (default: false)."
                            },
                            "recommended": {
                                "type": "integer",
                                "description": "Zero-based index of the recommended option."
                            }
                        },
                        "required": ["id", "question", "options"]
                    }
                }
            },
            "required": ["questions"]
        })
    }

    async fn execute(&self, args: Value, _ctx: &ToolContext) -> anyhow::Result<String> {
        let questions = parse_questions(&args)?;
        if questions.is_empty() {
            anyhow::bail!("No questions provided to ask tool.");
        }
        if self.is_interactive() {
            #[cfg(not(test))]
            {
                execute_interactive_tui(&questions)
            }
            #[cfg(test)]
            {
                let stdin = std::io::stdin();
                let mut reader = stdin.lock();
                let mut stdout = std::io::stdout();
                self.execute_interactive_with_io(&questions, &mut reader, &mut stdout)
            }
        } else {
            self.execute_headless(&questions)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn test_ask_tool_metadata() {
        let tool = AskTool::new();
        assert_eq!(tool.name(), "ask");
        assert_eq!(
            tool.description(),
            "Prompts the user with one or more structured questions to resolve architectural decisions, library choices, or destructive action confirmation."
        );
        let params = tool.parameters();
        assert!(params.get("properties").unwrap().get("questions").is_some());
    }

    #[test]
    fn test_headless_single_question_default() {
        let tool = AskTool::new().with_interactive(false);
        let q = Question {
            id: "db_choice".into(),
            question: "Which database?".into(),
            options: vec![
                QuestionOption::new("PostgreSQL").with_description("Relational database"),
                QuestionOption::new("SQLite").with_description("Embedded database"),
            ],
            multi: false,
            recommended: None,
        };

        let result = tool.execute_headless(&[q]).unwrap();
        assert_eq!(result, "Selected option: PostgreSQL");
    }

    #[test]
    fn test_headless_single_question_recommended() {
        let tool = AskTool::new().with_interactive(false);
        let q = Question {
            id: "db_choice".into(),
            question: "Which database?".into(),
            options: vec![
                QuestionOption::new("PostgreSQL"),
                QuestionOption::new("SQLite"),
            ],
            multi: false,
            recommended: Some(1),
        };

        let result = tool.execute_headless(&[q]).unwrap();
        assert_eq!(result, "Selected option: SQLite");
    }

    #[test]
    fn test_headless_multiple_questions() {
        let tool = AskTool::new().with_interactive(false);
        let q1 = Question {
            id: "db".into(),
            question: "Database choice".into(),
            options: vec![QuestionOption::new("PostgreSQL"), QuestionOption::new("SQLite")],
            multi: false,
            recommended: Some(0),
        };
        let q2 = Question {
            id: "cache".into(),
            question: "Cache choice".into(),
            options: vec![QuestionOption::new("Redis"), QuestionOption::new("Memcached")],
            multi: false,
            recommended: Some(1),
        };

        let result = tool.execute_headless(&[q1, q2]).unwrap();
        assert!(result.contains("db: Selected option: PostgreSQL"));
        assert!(result.contains("cache: Selected option: Memcached"));
    }

    #[test]
    fn test_interactive_chip_rendering() {
        let q = Question {
            id: "arch".into(),
            question: "Select architecture style".into(),
            options: vec![
                QuestionOption::new("Modular Monolith").with_description("Single deployable unit"),
                QuestionOption::new("Microservices").with_description("Distributed services"),
            ],
            multi: false,
            recommended: Some(0),
        };

        let mut output = Vec::new();
        render_question_prompt(&mut output, &q).unwrap();
        let rendered = String::from_utf8(output).unwrap();

        assert!(rendered.contains("❓ Select architecture style"));
        assert!(rendered.contains("[1] Modular Monolith (Recommended)"));
        assert!(rendered.contains("Single deployable unit"));
        assert!(rendered.contains("[2] Microservices"));
        assert!(!rendered.contains("[2] Microservices (Recommended)"));
    }

    #[test]
    fn test_interactive_selection_by_index() {
        let tool = AskTool::new();
        let q = Question {
            id: "db".into(),
            question: "Pick DB".into(),
            options: vec![QuestionOption::new("PostgreSQL"), QuestionOption::new("SQLite")],
            multi: false,
            recommended: Some(0),
        };

        let mut input = Cursor::new(b"2\n");
        let mut output = Vec::new();
        let choice = tool.prompt_question(&q, &mut input, &mut output).unwrap();
        assert_eq!(choice, "SQLite");
    }

    #[test]
    fn test_interactive_selection_by_label() {
        let tool = AskTool::new();
        let q = Question {
            id: "db".into(),
            question: "Pick DB".into(),
            options: vec![QuestionOption::new("PostgreSQL"), QuestionOption::new("SQLite")],
            multi: false,
            recommended: Some(0),
        };

        let mut input = Cursor::new(b"sqlite\n");
        let mut output = Vec::new();
        let choice = tool.prompt_question(&q, &mut input, &mut output).unwrap();
        assert_eq!(choice, "SQLite");
    }

    #[test]
    fn test_interactive_default_on_empty_input() {
        let tool = AskTool::new();
        let q = Question {
            id: "db".into(),
            question: "Pick DB".into(),
            options: vec![QuestionOption::new("PostgreSQL"), QuestionOption::new("SQLite")],
            multi: false,
            recommended: Some(1),
        };

        let mut input = Cursor::new(b"\n");
        let mut output = Vec::new();
        let choice = tool.prompt_question(&q, &mut input, &mut output).unwrap();
        assert_eq!(choice, "SQLite");
    }

    #[test]
    fn test_interactive_multi_selection() {
        let tool = AskTool::new();
        let q = Question {
            id: "features".into(),
            question: "Select features".into(),
            options: vec![
                QuestionOption::new("Auth"),
                QuestionOption::new("Logging"),
                QuestionOption::new("Metrics"),
            ],
            multi: true,
            recommended: Some(0),
        };

        let mut input = Cursor::new(b"1, 3\n");
        let mut output = Vec::new();
        let choice = tool.prompt_question(&q, &mut input, &mut output).unwrap();
        assert_eq!(choice, "Auth, Metrics");
    }

    #[tokio::test]
    async fn test_tool_execute_json() {
        let tool = AskTool::new().with_interactive(false);
        let ctx = ToolContext::default();
        let args = json!({
            "questions": [
                {
                    "id": "confirm_action",
                    "question": "Apply migration?",
                    "options": [
                        { "label": "Yes", "description": "Apply immediately" },
                        { "label": "No", "description": "Cancel action" }
                    ],
                    "recommended": 0
                }
            ]
        });

        let res = tool.execute(args, &ctx).await.unwrap();
        assert_eq!(res, "Selected option: Yes");
    }
}
