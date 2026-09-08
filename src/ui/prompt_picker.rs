//! Interactive Prompt Template Selector Widget
//!
//! Provides a polished, keyboard-driven prompt template picker for Fusion:
//! - Category tabs: `[All]  Rust  TypeScript  Security  Refactor  Documentation` (cycled via `Tab` / `Shift+Tab` / `1-9`).
//! - Left column: Category tabs, search query input (`/`), and filtered template list with cursor navigation.
//! - Right column: Template preview with parameter placeholder (`{{var}}`) highlighting and metadata.
//! - Status bar: Keyboard hints (`↑↓` navigate, `Tab` switch tab, `Enter` select, `Esc` cancel).
//! - Built-in curated prompt templates for common software engineering workflows.

use crossterm::{
    cursor,
    event::{self, Event, KeyCode, KeyEventKind, KeyModifiers},
    execute,
    terminal::{self, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{
    backend::CrosstermBackend,
    buffer::Buffer,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Widget},
};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::io::{stdout, Write};

// ---------------------------------------------------------------------------
// Prompt Template Data Structure
// ---------------------------------------------------------------------------

/// Representation of an AI prompt template with variable placeholders.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PromptTemplate {
    /// Friendly display name (e.g. `Rust: TDD Fix Loop`)
    pub name: String,
    /// Category classification (e.g. `Rust`, `TypeScript`, `Security`, `Refactor`, `Documentation`)
    pub category: String,
    /// Human-readable summary of what the prompt accomplishes
    pub description: String,
    /// Template text with parameter placeholders in `{{var_name}}` format
    pub template: String,
    /// Extracted variable placeholder names in order of appearance
    pub variables: Vec<String>,
}

impl PromptTemplate {
    /// Creates a new prompt template, automatically extracting unique `{{var}}` placeholders from `template`.
    pub fn new(
        name: impl Into<String>,
        category: impl Into<String>,
        description: impl Into<String>,
        template: impl Into<String>,
    ) -> Self {
        let template_str = template.into();
        let variables = Self::extract_variables(&template_str);
        Self {
            name: name.into(),
            category: category.into(),
            description: description.into(),
            template: template_str,
            variables,
        }
    }

    /// Creates a prompt template with explicitly provided variable names.
    pub fn with_variables(
        name: impl Into<String>,
        category: impl Into<String>,
        description: impl Into<String>,
        template: impl Into<String>,
        variables: Vec<String>,
    ) -> Self {
        Self {
            name: name.into(),
            category: category.into(),
            description: description.into(),
            template: template.into(),
            variables,
        }
    }

    /// Extracts unique variable names from `{{var}}` placeholders in order of appearance.
    /// Trims surrounding whitespace and ignores empty or malformed brackets.
    pub fn extract_variables(template: &str) -> Vec<String> {
        let mut vars = Vec::new();
        let mut remaining = template;

        while let Some(start) = remaining.find("{{") {
            let after_open = &remaining[start + 2..];
            if let Some(end) = after_open.find("}}") {
                let raw_var = &after_open[..end];
                let trimmed = raw_var.trim();
                if !trimmed.is_empty() && !vars.iter().any(|v: &String| v == trimmed) {
                    vars.push(trimmed.to_string());
                }
                remaining = &after_open[end + 2..];
            } else {
                break;
            }
        }

        vars
    }

    /// Renders the template by replacing `{{var}}` placeholders with values from `values`.
    /// Unspecified placeholders remain as original `{{var}}` tokens.
    pub fn render(&self, values: &HashMap<String, String>) -> String {
        let mut result = String::new();
        let mut remaining = self.template.as_str();

        while let Some(start) = remaining.find("{{") {
            result.push_str(&remaining[..start]);
            let after_open = &remaining[start + 2..];
            if let Some(end) = after_open.find("}}") {
                let raw_var = &after_open[..end];
                let trimmed = raw_var.trim();
                if let Some(val) = values.get(trimmed) {
                    result.push_str(val);
                } else {
                    result.push_str(&remaining[start..start + 2 + end + 2]);
                }
                remaining = &after_open[end + 2..];
            } else {
                result.push_str(&remaining[start..]);
                remaining = "";
                break;
            }
        }
        result.push_str(remaining);
        result
    }

    /// Checks if this template matches the search query (case-insensitive across name, category, description, template, variables).
    pub fn matches_filter(&self, query: &str) -> bool {
        let q = query.trim().to_lowercase();
        if q.is_empty() {
            return true;
        }
        self.name.to_lowercase().contains(&q)
            || self.category.to_lowercase().contains(&q)
            || self.description.to_lowercase().contains(&q)
            || self.template.to_lowercase().contains(&q)
            || self.variables.iter().any(|v| v.to_lowercase().contains(&q))
    }

    /// Checks if this template matches the specified category filter (`"All"` matches everything).
    pub fn matches_category(&self, category: &str) -> bool {
        if category.eq_ignore_ascii_case("all") {
            true
        } else {
            self.category.eq_ignore_ascii_case(category)
        }
    }
}

// ---------------------------------------------------------------------------
// Curated Built-in Prompts
// ---------------------------------------------------------------------------

/// Returns curated prompt templates covering high-impact software engineering workflows.
pub fn curated_prompts() -> Vec<PromptTemplate> {
    vec![
        PromptTemplate::new(
            "Rust: TDD Fix Loop",
            "Rust",
            "Autonomous TDD fix loop using compiler feedback and test failures",
            "Execute TDD fix loop for {{test_name}}:\n\
             1. Run `cargo test {{test_name}}` to capture failure diagnostics.\n\
             2. Read the failing test in {{test_file}} and implementation in {{source_file}}.\n\
             3. Identify the root cause and apply minimal surgical edits.\n\
             4. Re-run `cargo test {{test_name}}` to verify the fix passes without regressing other tests.",
        ),
        PromptTemplate::new(
            "Rust: Clippy & Performance",
            "Rust",
            "Audit Rust code for clippy lints, zero-cost abstractions, and allocation hotspots",
            "Audit {{target_file}} for performance and clippy hygiene:\n\
             - Eliminate redundant .clone() calls and unnecessary allocations.\n\
             - Replace heap allocations with borrowed slices or small-vector optimizations.\n\
             - Leverage iterator combinators and zero-cost abstractions.\n\
             - Target benchmark: {{benchmark_metric}}.\n\
             - Ensure zero clippy warnings with `cargo clippy --all-targets -- -D warnings`.",
        ),
        PromptTemplate::new(
            "TypeScript: React Component",
            "TypeScript",
            "Generate production-grade React component with TypeScript, Tailwind, and accessibility",
            "Create a production-grade React component named `{{component_name}}`:\n\
             - Requirements: {{requirements}}\n\
             - Props interface: {{props_interface}}\n\
             - Style using Tailwind CSS with dark mode and responsive variants.\n\
             - Ensure WCAG 2.1 AA accessibility (proper ARIA roles, focus management, keyboard handlers).\n\
             - Include comprehensive unit tests with React Testing Library.",
        ),
        PromptTemplate::new(
            "Security: Vulnerability Audit",
            "Security",
            "Inspect codebase for OWASP Top 10 vulnerabilities, injection flaws, and unsafe blocks",
            "Perform a comprehensive security audit on `{{codebase_path}}`:\n\
             - Threat model focus: {{threat_focus}} (injection, SSRF, broken auth, memory safety).\n\
             - Check all `unsafe` blocks, FFI boundaries, and deserialization routines.\n\
             - Evaluate sanitization of external user input and command execution arguments.\n\
             - Provide severity ratings (CVSS) and concrete remediation diffs.",
        ),
        PromptTemplate::new(
            "Refactor: Simplify & DRY",
            "Refactor",
            "Eliminate duplication, flatten nested logic, and improve readability without changing behavior",
            "Refactor {{target_module}} to improve maintainability and readability:\n\
             - Goal: {{refactor_goal}}\n\
             - Eliminate duplicated logic and extract reusable helper functions.\n\
             - Flatten deeply nested conditionals using guard clauses and early returns.\n\
             - Maintain 100% backward compatibility with existing public API contracts.\n\
             - Verify that all unit and integration tests still pass.",
        ),
        PromptTemplate::new(
            "Doc: API Reference",
            "Documentation",
            "Generate comprehensive API documentation with examples, panic conditions, and error contracts",
            "Generate comprehensive API documentation for `{{symbol_name}}` in `{{module_path}}`:\n\
             - Provide doc comments explaining purpose, design invariants, and performance characteristics.\n\
             - Include runnable `# Examples` with doctests demonstrating common usage.\n\
             - Explicitly document `# Errors`, `# Panics`, and `# Safety` contracts.\n\
             - Adhere to style guidelines: {{style_guide}}.",
        ),
    ]
}

/// Alias for `curated_prompts`.
pub fn builtin_prompts() -> Vec<PromptTemplate> {
    curated_prompts()
}

// ---------------------------------------------------------------------------
// Template Highlighting Helper
// ---------------------------------------------------------------------------

/// Slices a line of template text into styled spans, highlighting `{{...}}` placeholders in bold yellow.
pub fn highlight_template_line(line: &str) -> Vec<Span<'static>> {
    let mut spans = Vec::new();
    let mut remaining = line;

    while let Some(start) = remaining.find("{{") {
        if start > 0 {
            spans.push(Span::styled(
                remaining[..start].to_string(),
                Style::default().fg(Color::White),
            ));
        }
        let after_open = &remaining[start..];
        if let Some(end) = after_open.find("}}") {
            let var_slice = &after_open[..end + 2];
            spans.push(Span::styled(
                var_slice.to_string(),
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            ));
            remaining = &after_open[end + 2..];
        } else {
            spans.push(Span::styled(
                after_open.to_string(),
                Style::default().fg(Color::White),
            ));
            remaining = "";
            break;
        }
    }

    if !remaining.is_empty() {
        spans.push(Span::styled(
            remaining.to_string(),
            Style::default().fg(Color::White),
        ));
    }

    spans
}

// ---------------------------------------------------------------------------
// Prompt Picker Result & State
// ---------------------------------------------------------------------------

/// Result returned from handling keyboard input or running the interactive prompt picker.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PromptPickerResult {
    /// User selected and confirmed a prompt template.
    Selected(PromptTemplate),
    /// User cancelled the prompt picker dialog.
    Cancelled,
}

/// State container for the interactive prompt picker.
#[derive(Debug, Clone)]
pub struct PromptPickerState {
    /// Full catalog of available prompt templates.
    pub prompts: Vec<PromptTemplate>,
    /// Unique categories discovered from prompts (starting with `"All"`).
    pub categories: Vec<String>,
    /// Currently highlighted category tab index.
    pub selected_category_idx: usize,
    /// Currently highlighted template index in the filtered list.
    pub selected_prompt_idx: usize,
    /// Current search filter query string.
    pub search_query: String,
    /// Vertical scroll offset for the template list.
    pub scroll_offset: usize,
}

pub type PromptPicker = PromptPickerState;

impl PromptPickerState {
    /// Creates a new picker state with the given templates and derives category tabs.
    pub fn new(prompts: Vec<PromptTemplate>) -> Self {
        let mut categories = vec!["All".to_string()];
        for p in &prompts {
            if !categories.iter().any(|c| c.eq_ignore_ascii_case(&p.category)) {
                categories.push(p.category.clone());
            }
        }
        Self {
            prompts,
            categories,
            selected_category_idx: 0,
            selected_prompt_idx: 0,
            search_query: String::new(),
            scroll_offset: 0,
        }
    }

    /// Returns the currently active category name.
    pub fn current_category(&self) -> &str {
        self.categories
            .get(self.selected_category_idx)
            .map(|s| s.as_str())
            .unwrap_or("All")
    }

    /// Returns all templates matching the active category tab and current search query.
    pub fn filtered_prompts(&self) -> Vec<&PromptTemplate> {
        let cat = self.current_category();
        self.prompts
            .iter()
            .filter(|p| p.matches_category(cat) && p.matches_filter(&self.search_query))
            .collect()
    }

    /// Returns a reference to the currently selected template in the filtered list.
    pub fn selected_prompt(&self) -> Option<&PromptTemplate> {
        let filtered = self.filtered_prompts();
        filtered.get(self.selected_prompt_idx).copied()
    }

    /// Clamps selection and scroll offset within current filtered bounds.
    pub fn clamp_selection(&mut self) {
        let count = self.filtered_prompts().len();
        if count == 0 {
            self.selected_prompt_idx = 0;
            self.scroll_offset = 0;
        } else if self.selected_prompt_idx >= count {
            self.selected_prompt_idx = count - 1;
        }
    }

    /// Selects the next template in the filtered list (wraps around).
    pub fn select_next(&mut self) {
        let count = self.filtered_prompts().len();
        if count > 0 {
            if self.selected_prompt_idx + 1 < count {
                self.selected_prompt_idx += 1;
            } else {
                self.selected_prompt_idx = 0;
            }
        }
    }

    /// Selects the previous template in the filtered list (wraps around).
    pub fn select_prev(&mut self) {
        let count = self.filtered_prompts().len();
        if count > 0 {
            if self.selected_prompt_idx > 0 {
                self.selected_prompt_idx -= 1;
            } else {
                self.selected_prompt_idx = count - 1;
            }
        }
    }

    /// Selects the first template in the list.
    pub fn select_first(&mut self) {
        self.selected_prompt_idx = 0;
        self.scroll_offset = 0;
    }

    /// Selects the last template in the list.
    pub fn select_last(&mut self) {
        let count = self.filtered_prompts().len();
        if count > 0 {
            self.selected_prompt_idx = count - 1;
        }
    }

    /// Cycles forward to the next category tab (`Tab`).
    pub fn next_category(&mut self) {
        if !self.categories.is_empty() {
            self.selected_category_idx = (self.selected_category_idx + 1) % self.categories.len();
            self.selected_prompt_idx = 0;
            self.scroll_offset = 0;
        }
    }

    /// Cycles backward to the previous category tab (`Shift+Tab` / `BackTab`).
    pub fn prev_category(&mut self) {
        if !self.categories.is_empty() {
            if self.selected_category_idx > 0 {
                self.selected_category_idx -= 1;
            } else {
                self.selected_category_idx = self.categories.len() - 1;
            }
            self.selected_prompt_idx = 0;
            self.scroll_offset = 0;
        }
    }

    /// Sets category by name.
    pub fn set_category(&mut self, category: &str) {
        if let Some(pos) = self
            .categories
            .iter()
            .position(|c| c.eq_ignore_ascii_case(category))
        {
            self.selected_category_idx = pos;
            self.selected_prompt_idx = 0;
            self.scroll_offset = 0;
        }
    }

    /// Sets the search filter query.
    pub fn set_search_query(&mut self, query: impl Into<String>) {
        self.search_query = query.into();
        self.selected_prompt_idx = 0;
        self.scroll_offset = 0;
    }

    /// Clears the search filter query.
    pub fn clear_search(&mut self) {
        self.search_query.clear();
        self.selected_prompt_idx = 0;
        self.scroll_offset = 0;
    }

    /// Processes keyboard events, returning `Some(PromptPickerResult)` on selection/cancellation.
    pub fn handle_key(
        &mut self,
        code: KeyCode,
        modifiers: KeyModifiers,
    ) -> Option<PromptPickerResult> {
        match (code, modifiers) {
            // Enter: Select current template
            (KeyCode::Enter, _) => {
                if let Some(selected) = self.selected_prompt().cloned() {
                    Some(PromptPickerResult::Selected(selected))
                } else {
                    Some(PromptPickerResult::Cancelled)
                }
            }

            // Esc: Clear search filter if active, otherwise cancel
            (KeyCode::Esc, _) => {
                if !self.search_query.is_empty() {
                    self.clear_search();
                    None
                } else {
                    Some(PromptPickerResult::Cancelled)
                }
            }

            // Ctrl+C: Cancel
            (KeyCode::Char('c'), KeyModifiers::CONTROL) => Some(PromptPickerResult::Cancelled),

            // Tab: Next category tab
            (KeyCode::Tab, KeyModifiers::NONE) => {
                self.next_category();
                None
            }

            // Shift+Tab or BackTab: Previous category tab
            (KeyCode::BackTab, _) | (KeyCode::Tab, KeyModifiers::SHIFT) => {
                self.prev_category();
                None
            }

            // Up arrow or Ctrl+P: Navigate up
            (KeyCode::Up, _) | (KeyCode::Char('p'), KeyModifiers::CONTROL) => {
                self.select_prev();
                None
            }

            // Down arrow or Ctrl+N: Navigate down
            (KeyCode::Down, _) | (KeyCode::Char('n'), KeyModifiers::CONTROL) => {
                self.select_next();
                None
            }

            // Left arrow: Prev category when search query is empty
            (KeyCode::Left, KeyModifiers::NONE) if self.search_query.is_empty() => {
                self.prev_category();
                None
            }

            // Right arrow: Next category when search query is empty
            (KeyCode::Right, KeyModifiers::NONE) if self.search_query.is_empty() => {
                self.next_category();
                None
            }

            // Number keys 1-9 jump to category tab when search is empty
            (KeyCode::Char(c @ '1'..='9'), KeyModifiers::NONE) if self.search_query.is_empty() => {
                let idx = (c as usize).saturating_sub('1' as usize);
                if idx < self.categories.len() {
                    self.selected_category_idx = idx;
                    self.selected_prompt_idx = 0;
                    self.scroll_offset = 0;
                }
                None
            }

            // Backspace: Delete character in search filter
            (KeyCode::Backspace, _) => {
                if !self.search_query.is_empty() {
                    self.search_query.pop();
                    self.clamp_selection();
                }
                None
            }

            // Ctrl+U: Clear search query
            (KeyCode::Char('u'), KeyModifiers::CONTROL) => {
                self.clear_search();
                None
            }

            // Ctrl+W: Delete last word in search query
            (KeyCode::Char('w'), KeyModifiers::CONTROL) => {
                if !self.search_query.is_empty() {
                    let trimmed = self.search_query.trim_end();
                    if let Some(pos) = trimmed.rfind(' ') {
                        self.search_query.truncate(pos);
                    } else {
                        self.search_query.clear();
                    }
                    self.clamp_selection();
                }
                None
            }

            // Home / End
            (KeyCode::Home, _) => {
                self.select_first();
                None
            }
            (KeyCode::End, _) => {
                self.select_last();
                None
            }

            // PageUp / PageDown
            (KeyCode::PageUp, _) => {
                for _ in 0..5 {
                    self.select_prev();
                }
                None
            }
            (KeyCode::PageDown, _) => {
                for _ in 0..5 {
                    self.select_next();
                }
                None
            }

            // 'q' cancels when search query is empty
            (KeyCode::Char('q'), KeyModifiers::NONE) if self.search_query.is_empty() => {
                Some(PromptPickerResult::Cancelled)
            }

            // Vim navigation 'k' / 'j' when search is empty
            (KeyCode::Char('k'), KeyModifiers::NONE) if self.search_query.is_empty() => {
                self.select_prev();
                None
            }
            (KeyCode::Char('j'), KeyModifiers::NONE) if self.search_query.is_empty() => {
                self.select_next();
                None
            }

            // Slash '/' to clear / refocus search
            (KeyCode::Char('/'), KeyModifiers::NONE) if self.search_query.is_empty() => {
                self.clear_search();
                None
            }

            // Typing characters: append to search query
            (KeyCode::Char(c), KeyModifiers::NONE) | (KeyCode::Char(c), KeyModifiers::SHIFT) => {
                self.search_query.push(c);
                self.selected_prompt_idx = 0;
                self.scroll_offset = 0;
                None
            }

            _ => None,
        }
    }

    /// Runs interactive TUI dialog inside an alternate terminal screen.
    pub fn run_interactive(&mut self) -> std::io::Result<Option<PromptTemplate>> {
        let _guard = RawModeGuard::enter()?;
        let mut stdout = stdout();
        execute!(stdout, EnterAlternateScreen, cursor::Hide)?;

        let backend = CrosstermBackend::new(stdout);
        let mut terminal = ratatui::Terminal::new(backend)?;

        let outcome = self.event_loop(&mut terminal);

        let _ = execute!(terminal.backend_mut(), cursor::Show, LeaveAlternateScreen);
        let _ = terminal.backend_mut().flush();

        outcome
    }

    fn event_loop<B: ratatui::backend::Backend>(
        &mut self,
        terminal: &mut ratatui::Terminal<B>,
    ) -> std::io::Result<Option<PromptTemplate>> {
        loop {
            terminal.draw(|f| {
                let widget = PromptPickerWidget::new(self);
                f.render_widget(widget, f.area());
            })?;

            if event::poll(std::time::Duration::from_millis(50))? {
                if let Event::Key(key) = event::read()? {
                    if key.kind == KeyEventKind::Release {
                        continue;
                    }
                    if let Some(result) = self.handle_key(key.code, key.modifiers) {
                        match result {
                            PromptPickerResult::Selected(prompt) => return Ok(Some(prompt)),
                            PromptPickerResult::Cancelled => return Ok(None),
                        }
                    }
                }
            }
        }
    }
}

impl Default for PromptPickerState {
    fn default() -> Self {
        Self::new(curated_prompts())
    }
}

// ---------------------------------------------------------------------------
// Terminal RAII Raw Mode Guard
// ---------------------------------------------------------------------------

/// RAII Guard that enables terminal raw mode on creation and disables on drop.
pub struct RawModeGuard;

impl RawModeGuard {
    pub fn enter() -> std::io::Result<Self> {
        terminal::enable_raw_mode()?;
        let _ = execute!(stdout(), cursor::Hide);
        Ok(Self)
    }
}

impl Drop for RawModeGuard {
    fn drop(&mut self) {
        let _ = execute!(stdout(), cursor::Show);
        let _ = terminal::disable_raw_mode();
        let _ = stdout().flush();
    }
}

// ---------------------------------------------------------------------------
// Ratatui Widget Implementation
// ---------------------------------------------------------------------------

/// Ratatui widget that renders the complete prompt picker UI:
/// - Left column: Category tabs & template list with search filter query.
/// - Right column: Template preview with parameter placeholder highlighting.
/// - Status bar with keyboard hints (`↑↓` navigate, `Tab` switch tab, `Enter` select, `Esc` cancel).
pub struct PromptPickerWidget<'a> {
    pub state: &'a PromptPickerState,
    pub title: Option<String>,
}

impl<'a> PromptPickerWidget<'a> {
    /// Constructs a new widget referencing the given picker state.
    pub fn new(state: &'a PromptPickerState) -> Self {
        Self { state, title: None }
    }

    /// Sets an optional title header for the widget.
    pub fn with_title(mut self, title: impl Into<String>) -> Self {
        self.title = Some(title.into());
        self
    }

    /// Renders the complete picker UI into a Ratatui buffer.
    pub fn render_buffer(&self, area: Rect, buf: &mut Buffer) {
        if area.width < 10 || area.height < 3 {
            return;
        }

        // Top-level vertical layout: Content area + Status bar footer
        let has_footer = area.height >= 5;
        let (content_area, footer_area) = if has_footer {
            let chunks = Layout::default()
                .direction(Direction::Vertical)
                .constraints([Constraint::Min(4), Constraint::Length(1)])
                .split(area);
            (chunks[0], chunks[1])
        } else {
            (area, Rect::default())
        };

        // Two-column horizontal split: Left (Tabs + Search + List), Right (Preview)
        let (left_area, right_area) = if content_area.width >= 55 {
            let cols = Layout::default()
                .direction(Direction::Horizontal)
                .constraints([Constraint::Percentage(45), Constraint::Percentage(55)])
                .split(content_area);
            (cols[0], cols[1])
        } else if content_area.height >= 14 {
            let rows = Layout::default()
                .direction(Direction::Vertical)
                .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
                .split(content_area);
            (rows[0], rows[1])
        } else {
            (content_area, Rect::default())
        };

        // Render Left Column
        self.render_left_column(left_area, buf);

        // Render Right Column (Preview)
        if right_area.width > 0 && right_area.height > 0 {
            self.render_right_preview(right_area, buf);
        }

        // Render Status Bar Footer
        if has_footer && footer_area.height > 0 {
            self.render_status_bar(footer_area, buf);
        }
    }

    fn render_left_column(&self, area: Rect, buf: &mut Buffer) {
        if area.width < 5 || area.height < 2 {
            return;
        }

        if area.height >= 10 {
            let chunks = Layout::default()
                .direction(Direction::Vertical)
                .constraints([
                    Constraint::Length(3), // Category tabs block
                    Constraint::Length(3), // Search bar block
                    Constraint::Min(4),    // Template list block
                ])
                .split(area);

            self.render_category_tabs_block(chunks[0], buf);
            self.render_search_bar_block(chunks[1], buf);
            self.render_template_list_block(chunks[2], buf);
        } else if area.height >= 6 {
            let chunks = Layout::default()
                .direction(Direction::Vertical)
                .constraints([
                    Constraint::Length(1), // Category tabs line
                    Constraint::Length(1), // Search bar line
                    Constraint::Min(3),    // Template list
                ])
                .split(area);

            self.render_category_tabs_line(chunks[0], buf);
            self.render_search_bar_line(chunks[1], buf);
            self.render_template_list_block(chunks[2], buf);
        } else {
            self.render_template_list_block(area, buf);
        }
    }

    fn render_category_tabs_block(&self, area: Rect, buf: &mut Buffer) {
        let block = Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(Color::DarkGray))
            .title(" Categories (Tab) ");
        let inner = block.inner(area);
        block.render(area, buf);
        self.render_category_tabs_line(inner, buf);
    }

    fn render_category_tabs_line(&self, area: Rect, buf: &mut Buffer) {
        if area.height == 0 || area.width == 0 {
            return;
        }
        let mut spans = Vec::new();
        for (i, cat) in self.state.categories.iter().enumerate() {
            let is_active = i == self.state.selected_category_idx;
            if is_active {
                spans.push(Span::styled(
                    format!(" [{}] ", cat),
                    Style::default()
                        .fg(Color::Cyan)
                        .add_modifier(Modifier::BOLD),
                ));
            } else {
                spans.push(Span::styled(
                    format!("  {}  ", cat),
                    Style::default().fg(Color::DarkGray),
                ));
            }
        }
        let line = Line::from(spans);
        buf.set_line(area.x, area.y, &line, area.width);
    }

    fn render_search_bar_block(&self, area: Rect, buf: &mut Buffer) {
        let is_filtering = !self.state.search_query.is_empty();
        let border_color = if is_filtering {
            Color::Yellow
        } else {
            Color::DarkGray
        };
        let block = Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(border_color))
            .title(" Search (/ to type) ");
        let inner = block.inner(area);
        block.render(area, buf);
        self.render_search_bar_line(inner, buf);
    }

    fn render_search_bar_line(&self, area: Rect, buf: &mut Buffer) {
        if area.height == 0 || area.width == 0 {
            return;
        }
        let mut spans = Vec::new();
        spans.push(Span::styled("🔎 ", Style::default().fg(Color::Yellow)));

        if self.state.search_query.is_empty() {
            spans.push(Span::styled(
                "Type to filter templates...",
                Style::default().fg(Color::DarkGray),
            ));
        } else {
            spans.push(Span::styled(
                &self.state.search_query,
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            ));
            spans.push(Span::styled(
                "▏",
                Style::default()
                    .fg(Color::White)
                    .add_modifier(Modifier::BOLD),
            ));
        }
        let line = Line::from(spans);
        buf.set_line(area.x, area.y, &line, area.width);
    }

    fn render_template_list_block(&self, area: Rect, buf: &mut Buffer) {
        if area.height == 0 || area.width == 0 {
            return;
        }
        let filtered = self.state.filtered_prompts();
        let title = format!(" Templates ({}) ", filtered.len());
        let block = Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(Color::Blue))
            .title(title);
        let inner = block.inner(area);
        block.render(area, buf);

        if inner.height == 0 || inner.width == 0 {
            return;
        }

        if filtered.is_empty() {
            let empty_line = Line::from(Span::styled(
                "  No templates match query",
                Style::default().fg(Color::DarkGray),
            ));
            buf.set_line(inner.x, inner.y, &empty_line, inner.width);
            return;
        }

        let visible_rows = inner.height as usize;
        let mut scroll = self.state.scroll_offset;

        // Auto-scroll to keep selected item in view
        if self.state.selected_prompt_idx < scroll {
            scroll = self.state.selected_prompt_idx;
        } else if self.state.selected_prompt_idx >= scroll + visible_rows {
            scroll = self.state.selected_prompt_idx.saturating_sub(visible_rows) + 1;
        }

        for i in 0..visible_rows {
            let item_idx = scroll + i;
            if item_idx >= filtered.len() {
                break;
            }
            let prompt = filtered[item_idx];
            let is_selected = item_idx == self.state.selected_prompt_idx;
            let y = inner.y + i as u16;

            let mut spans = Vec::new();
            if is_selected {
                spans.push(Span::styled(
                    "❯ ",
                    Style::default()
                        .fg(Color::Cyan)
                        .add_modifier(Modifier::BOLD),
                ));
                spans.push(Span::styled(
                    &prompt.name,
                    Style::default()
                        .fg(Color::White)
                        .add_modifier(Modifier::BOLD),
                ));
                spans.push(Span::raw(" "));
                spans.push(Span::styled(
                    format!("[{}]", prompt.category),
                    Style::default().fg(Color::Cyan),
                ));
            } else {
                spans.push(Span::raw("  "));
                spans.push(Span::styled(
                    &prompt.name,
                    Style::default().fg(Color::Gray),
                ));
                spans.push(Span::raw(" "));
                spans.push(Span::styled(
                    format!("[{}]", prompt.category),
                    Style::default().fg(Color::DarkGray),
                ));
            }
            let line = Line::from(spans);
            buf.set_line(inner.x, y, &line, inner.width);
        }
    }

    fn render_right_preview(&self, area: Rect, buf: &mut Buffer) {
        if area.height == 0 || area.width == 0 {
            return;
        }

        let preview_title = if let Some(prompt) = self.state.selected_prompt() {
            format!(" Preview: {} ", prompt.name)
        } else {
            " Preview ".to_string()
        };

        let block = Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(Color::Cyan))
            .title(Span::styled(
                preview_title,
                Style::default()
                    .fg(Color::White)
                    .add_modifier(Modifier::BOLD),
            ));
        let inner = block.inner(area);
        block.render(area, buf);

        if inner.height == 0 || inner.width == 0 {
            return;
        }

        if let Some(prompt) = self.state.selected_prompt() {
            let mut cur_y = inner.y;
            let max_y = inner.y + inner.height;

            // 1. Category line
            if cur_y < max_y {
                let cat_line = Line::from(vec![
                    Span::styled("Category: ", Style::default().fg(Color::DarkGray)),
                    Span::styled(
                        &prompt.category,
                        Style::default()
                            .fg(Color::Cyan)
                            .add_modifier(Modifier::BOLD),
                    ),
                ]);
                buf.set_line(inner.x, cur_y, &cat_line, inner.width);
                cur_y += 1;
            }

            // 2. Description line
            if cur_y < max_y {
                let desc_line = Line::from(vec![
                    Span::styled("Description: ", Style::default().fg(Color::DarkGray)),
                    Span::styled(&prompt.description, Style::default().fg(Color::White)),
                ]);
                buf.set_line(inner.x, cur_y, &desc_line, inner.width);
                cur_y += 1;
            }

            // 3. Variables line with highlighted parameter badges
            if cur_y < max_y {
                let mut var_spans = vec![Span::styled(
                    "Variables: ",
                    Style::default().fg(Color::DarkGray),
                )];
                if prompt.variables.is_empty() {
                    var_spans.push(Span::styled(
                        "None",
                        Style::default().fg(Color::DarkGray),
                    ));
                } else {
                    for v in &prompt.variables {
                        var_spans.push(Span::styled(
                            format!("{{{{{}}}}} ", v),
                            Style::default()
                                .fg(Color::Yellow)
                                .add_modifier(Modifier::BOLD),
                        ));
                    }
                }
                let var_line = Line::from(var_spans);
                buf.set_line(inner.x, cur_y, &var_line, inner.width);
                cur_y += 1;
            }

            // 4. Divider line
            if cur_y < max_y {
                let divider = "─".repeat(inner.width as usize);
                let div_line =
                    Line::from(Span::styled(divider, Style::default().fg(Color::DarkGray)));
                buf.set_line(inner.x, cur_y, &div_line, inner.width);
                cur_y += 1;
            }

            // 5. Template body with highlighted parameter placeholders
            for line_str in prompt.template.lines() {
                if cur_y >= max_y {
                    break;
                }
                let spans = highlight_template_line(line_str);
                let line = Line::from(spans);
                buf.set_line(inner.x, cur_y, &line, inner.width);
                cur_y += 1;
            }
        } else {
            let empty_line = Line::from(Span::styled(
                "No prompt template selected",
                Style::default().fg(Color::DarkGray),
            ));
            buf.set_line(inner.x, inner.y, &empty_line, inner.width);
        }
    }

    fn render_status_bar(&self, area: Rect, buf: &mut Buffer) {
        let hints = [
            ("↑↓", "navigate"),
            ("Tab", "switch tab"),
            ("/", "search"),
            ("Enter", "select"),
            ("Esc", "cancel"),
        ];

        let mut spans = Vec::new();
        spans.push(Span::raw(" "));
        for (i, (key, desc)) in hints.iter().enumerate() {
            if i > 0 {
                spans.push(Span::styled("  •  ", Style::default().fg(Color::DarkGray)));
            }
            spans.push(Span::styled(
                *key,
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            ));
            spans.push(Span::raw(" "));
            spans.push(Span::styled(*desc, Style::default().fg(Color::Gray)));
        }
        let line = Line::from(spans);
        buf.set_line(area.x, area.y, &line, area.width);
    }
}

impl<'a> Widget for PromptPickerWidget<'a> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        self.render_buffer(area, buf);
    }
}

impl<'a> Widget for &PromptPickerWidget<'a> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        self.render_buffer(area, buf);
    }
}

impl Widget for &PromptPickerState {
    fn render(self, area: Rect, buf: &mut Buffer) {
        PromptPickerWidget::new(self).render(area, buf);
    }
}

impl Widget for PromptPickerState {
    fn render(self, area: Rect, buf: &mut Buffer) {
        PromptPickerWidget::new(&self).render(area, buf);
    }
}

// ---------------------------------------------------------------------------
// Convenience Entry Point
// ---------------------------------------------------------------------------

/// Convenience helper to pick a prompt template interactively using a TUI dialog.
pub fn pick_prompt_interactive() -> std::io::Result<Option<PromptTemplate>> {
    let mut picker = PromptPickerState::default();
    picker.run_interactive()
}

/// Helper to pick a prompt template interactively from a custom list of templates.
pub fn pick_prompt_from_list(
    prompts: Vec<PromptTemplate>,
) -> std::io::Result<Option<PromptTemplate>> {
    let mut picker = PromptPickerState::new(prompts);
    picker.run_interactive()
}
