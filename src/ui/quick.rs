//! Fast In-Terminal Quick Actions Popup Menu (Triggered by `/` in REPL)
//!
//! Provides a high-performance, keyboard-driven fuzzy action launcher:
//! - Triggered instantly by typing `/` on an empty prompt or via `/palette` / `/quick`.
//! - Pure Rust fuzzy matching across command names, aliases, titles, syntax, and descriptions.
//! - Categorized tab navigation: `[ All ] [ Core ] [ Session ] [ Model ] [ Config ]` (cycled via `Tab` / `Shift+Tab`).
//! - Matched character highlighting in command strings and descriptions.
//! - Rich metadata: icons, syntax signatures, category badges, key shortcuts (e.g. `Ctrl+P`).
//! - Full keyboard controls: `↑`/`↓`/`Ctrl+P`/`Ctrl+N`/`j`/`k` to navigate, `Tab` for tabs,
//!   `Enter` to select and execute, `Esc`/`Ctrl+C` to cancel.
//! - Seamless rendering inside Ratatui inline viewport or full-frame popup.

use crossterm::{
    cursor,
    event::{self, Event, KeyCode, KeyEventKind, KeyModifiers},
    execute,
};
use ratatui::{
    buffer::Buffer,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Paragraph, Widget},
};
use serde::{Deserialize, Serialize};
use std::cmp::Ordering;
use std::io::{stdout, Write};

use crate::ui::inline::InlineTerminal;
use crate::ui::prompt::RawModeGuard;
use crate::ui::slash::{get_command_palette, CommandCategory, CommandDescriptor};

// ---------------------------------------------------------------------------
// Quick Action Categories & Tabs
// ---------------------------------------------------------------------------

/// Categorization tabs for filtering quick actions in the popup menu.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
pub enum QuickActionCategory {
    /// Show all actions across all categories.
    #[default]
    All,
    /// Essential shell navigation, screen control, and exits.
    Core,
    /// Session lifecycle, persistence, branching, and rewinding.
    Session,
    /// LLM model, provider, advisors, and token/cost analytics.
    Model,
    /// Tool registration, system configuration, and diagnostic tracing.
    Config,
}

impl QuickActionCategory {
    /// Ordered list of all category tabs displayed in the header.
    pub const ALL: [QuickActionCategory; 5] = [
        QuickActionCategory::All,
        QuickActionCategory::Core,
        QuickActionCategory::Session,
        QuickActionCategory::Model,
        QuickActionCategory::Config,
    ];

    /// Human-readable tab label.
    pub fn name(&self) -> &'static str {
        match self {
            QuickActionCategory::All => "All",
            QuickActionCategory::Core => "Core",
            QuickActionCategory::Session => "Session",
            QuickActionCategory::Model => "Model",
            QuickActionCategory::Config => "Config",
        }
    }

    /// Short label for compact screens.
    pub fn short_name(&self) -> &'static str {
        match self {
            QuickActionCategory::All => "All",
            QuickActionCategory::Core => "Core",
            QuickActionCategory::Session => "Sess",
            QuickActionCategory::Model => "Modl",
            QuickActionCategory::Config => "Conf",
        }
    }

    /// Category icon / emoji.
    pub fn icon(&self) -> &'static str {
        match self {
            QuickActionCategory::All => "⚡",
            QuickActionCategory::Core => "💻",
            QuickActionCategory::Session => "🌿",
            QuickActionCategory::Model => "🧠",
            QuickActionCategory::Config => "⚙️",
        }
    }

    /// Formatted badge string for row display (e.g. `[Core]`, `[Sess]`).
    pub fn badge(&self) -> &'static str {
        match self {
            QuickActionCategory::All => "[ALL]",
            QuickActionCategory::Core => "[CORE]",
            QuickActionCategory::Session => "[SESS]",
            QuickActionCategory::Model => "[MODL]",
            QuickActionCategory::Config => "[CONF]",
        }
    }

    /// Accent color associated with this category.
    pub fn color(&self) -> Color {
        match self {
            QuickActionCategory::All => Color::Cyan,
            QuickActionCategory::Core => Color::Cyan,
            QuickActionCategory::Session => Color::Green,
            QuickActionCategory::Model => Color::Yellow,
            QuickActionCategory::Config => Color::Magenta,
        }
    }

    /// Check if this category matches a command category.
    pub fn matches(&self, other: &QuickActionCategory) -> bool {
        match self {
            QuickActionCategory::All => true,
            _ => self == other,
        }
    }

    /// Converts from `CommandCategory` in `slash.rs`.
    pub fn from_command_category(cat: CommandCategory) -> Self {
        match cat {
            CommandCategory::Core => QuickActionCategory::Core,
            CommandCategory::Session => QuickActionCategory::Session,
            CommandCategory::Model => QuickActionCategory::Model,
            CommandCategory::Config => QuickActionCategory::Config,
        }
    }

    /// Cycle forward to next tab (`Tab`).
    pub fn next(&self) -> Self {
        match self {
            QuickActionCategory::All => QuickActionCategory::Core,
            QuickActionCategory::Core => QuickActionCategory::Session,
            QuickActionCategory::Session => QuickActionCategory::Model,
            QuickActionCategory::Model => QuickActionCategory::Config,
            QuickActionCategory::Config => QuickActionCategory::All,
        }
    }

    /// Cycle backward to previous tab (`Shift+Tab`).
    pub fn prev(&self) -> Self {
        match self {
            QuickActionCategory::All => QuickActionCategory::Config,
            QuickActionCategory::Core => QuickActionCategory::All,
            QuickActionCategory::Session => QuickActionCategory::Core,
            QuickActionCategory::Model => QuickActionCategory::Session,
            QuickActionCategory::Config => QuickActionCategory::Model,
        }
    }

    /// Parses loose string to category.
    pub fn from_str_loose(s: &str) -> Option<Self> {
        match s.trim().to_lowercase().as_str() {
            "all" | "*" => Some(QuickActionCategory::All),
            "core" | "repl" | "nav" => Some(QuickActionCategory::Core),
            "session" | "sess" | "history" | "fork" => Some(QuickActionCategory::Session),
            "model" | "modl" | "provider" | "stats" | "cost" => Some(QuickActionCategory::Model),
            "config" | "conf" | "cfg" | "tools" | "skills" | "trace" => {
                Some(QuickActionCategory::Config)
            }
            _ => None,
        }
    }
}

// ---------------------------------------------------------------------------
// Quick Action Item Model
// ---------------------------------------------------------------------------

/// Represents an executable action in the quick actions popup menu.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct QuickAction {
    /// Unique identifier for this action (e.g. `"help"`, `"model"`, `"session_new"`).
    pub id: String,
    /// Canonical command string executed when selected (e.g. `"/help"`, `"/model"`, `"/session new"`).
    pub command: String,
    /// Human-friendly display title (e.g. `"Help & Documentation"`, `"Switch LLM Model"`).
    pub title: String,
    /// Parameter syntax / signature guide (e.g. `"/help [command]"`, `"/model [name]"`).
    pub syntax: String,
    /// Concise explanation of what this action does.
    pub description: String,
    /// Category for tab grouping.
    pub category: QuickActionCategory,
    /// Alternative aliases and shortcut tokens.
    pub aliases: Vec<String>,
    /// Visual icon or symbol.
    pub icon: String,
    /// Optional keyboard shortcut label (e.g. `Ctrl+P` for file finder).
    pub shortcut: Option<String>,
    /// Popularity / priority weight for default sort order (higher = listed first).
    pub priority: u32,
    /// Detailed usage examples.
    pub examples: Vec<String>,
}

impl QuickAction {
    /// Create a new `QuickAction`.
    pub fn new(
        id: impl Into<String>,
        command: impl Into<String>,
        title: impl Into<String>,
        category: QuickActionCategory,
        description: impl Into<String>,
    ) -> Self {
        let cmd = command.into();
        Self {
            id: id.into(),
            syntax: cmd.clone(),
            command: cmd,
            title: title.into(),
            description: description.into(),
            category,
            aliases: Vec::new(),
            icon: category.icon().to_string(),
            shortcut: None,
            priority: 100,
            examples: Vec::new(),
        }
    }

    /// Builder: set syntax string.
    pub fn with_syntax(mut self, syntax: impl Into<String>) -> Self {
        self.syntax = syntax.into();
        self
    }

    /// Builder: set icon.
    pub fn with_icon(mut self, icon: impl Into<String>) -> Self {
        self.icon = icon.into();
        self
    }

    /// Builder: add aliases.
    pub fn with_aliases(mut self, aliases: &[&str]) -> Self {
        self.aliases = aliases.iter().map(|s| s.to_string()).collect();
        self
    }

    /// Builder: set shortcut.
    pub fn with_shortcut(mut self, shortcut: impl Into<String>) -> Self {
        self.shortcut = Some(shortcut.into());
        self
    }

    /// Builder: set priority weight.
    pub fn with_priority(mut self, priority: u32) -> Self {
        self.priority = priority;
        self
    }

    /// Builder: set examples.
    pub fn with_examples(mut self, examples: &[&str]) -> Self {
        self.examples = examples.iter().map(|s| s.to_string()).collect();
        self
    }

    /// Creates a `QuickAction` from a `CommandDescriptor` in `slash.rs`.
    pub fn from_descriptor(desc: &CommandDescriptor) -> Self {
        let cat = QuickActionCategory::from_command_category(desc.category);
        let id = desc.name.trim_start_matches('/').to_string();
        let title = match desc.name {
            "/help" => "Help & Documentation",
            "/palette" => "Command Palette",
            "/file" => "Fuzzy File Finder",
            "/clear" => "Clear Screen & History",
            "/status" => "System & Model Status",
            "/quit" => "Save & Exit REPL",
            "/session" => "Session Manager",
            "/fork" => "Branch Session (Fork)",
            "/rewind" => "Undo / Rewind Turns",
            "/compact" => "Compact Token History",
            "/export" => "Export Conversation Transcript",
            "/model" => "Switch LLM Model",
            "/provider" => "Switch LLM Provider",
            "/advisors" => "Toggle Advisors",
            "/stats" => "Token Usage & Cost Analytics",
            "/config" => "View & Set Configuration",
            "/preset" => "Apply Config Preset",
            "/tools" => "List Registered Tools",
            "/trace" => "Export Diagnostic Trace",
            "/skills" => "Manage Domain Skills",
            _ => desc.name,
        };

        let shortcut = match desc.name {
            "/file" => Some("Ctrl+P".to_string()),
            "/clear" => Some("Ctrl+L".to_string()),
            "/quit" => Some("Ctrl+D".to_string()),
            "/palette" => Some("Ctrl+/".to_string()),
            _ => None,
        };

        let priority = match desc.name {
            "/help" => 200,
            "/file" => 195,
            "/model" => 190,
            "/session" => 185,
            "/clear" => 180,
            "/status" => 175,
            "/stats" => 170,
            "/fork" => 165,
            "/rewind" => 160,
            "/preset" => 155,
            "/provider" => 150,
            "/export" => 145,
            "/advisors" => 140,
            "/compact" => 135,
            "/tools" => 130,
            "/config" => 125,
            "/skills" => 120,
            "/trace" => 115,
            "/palette" => 110,
            "/quit" => 100,
            _ => 50,
        };

        Self {
            id,
            command: desc.name.to_string(),
            title: title.to_string(),
            syntax: desc.syntax.to_string(),
            description: desc.description.to_string(),
            category: cat,
            aliases: desc.aliases.iter().map(|s| s.to_string()).collect(),
            icon: cat.icon().to_string(),
            shortcut,
            priority,
            examples: desc.examples.iter().map(|s| s.to_string()).collect(),
        }
    }
}

// ---------------------------------------------------------------------------
// Default Actions Catalogue
// ---------------------------------------------------------------------------

/// Builds the comprehensive default quick action catalog combining all top-level
/// slash commands from `COMMAND_PALETTE` plus frequently-used subcommands.
pub fn default_quick_actions() -> Vec<QuickAction> {
    let mut actions = Vec::new();

    // 1. Ingest base descriptors from slash::COMMAND_PALETTE
    for desc in get_command_palette() {
        actions.push(QuickAction::from_descriptor(desc));
    }

    // 2. Add granular high-value subactions for instant fuzzy discoverability
    actions.push(
        QuickAction::new(
            "session_list",
            "/session list",
            "List Saved Sessions",
            QuickActionCategory::Session,
            "List all persistent sessions saved in ~/.fusion/sessions",
        )
        .with_syntax("/session list")
        .with_aliases(&["/s list", "/sl"])
        .with_priority(184)
        .with_examples(&["/session list"]),
    );

    actions.push(
        QuickAction::new(
            "session_search",
            "/session search",
            "Search Session History",
            QuickActionCategory::Session,
            "Fuzzy search through past conversation transcripts and topics",
        )
        .with_syntax("/session search <query>")
        .with_aliases(&["/s search", "/s find"])
        .with_priority(183)
        .with_examples(&["/session search auth", "/session search \"bug fix\""]),
    );

    actions.push(
        QuickAction::new(
            "session_new",
            "/session new",
            "Start Fresh Session",
            QuickActionCategory::Session,
            "Create and switch to a brand new conversation session",
        )
        .with_syntax("/session new [model]")
        .with_aliases(&["/s new", "/new"])
        .with_priority(182)
        .with_examples(&["/session new", "/session new gpt-4o"]),
    );

    actions.push(
        QuickAction::new(
            "session_save",
            "/session save",
            "Checkpoint Session",
            QuickActionCategory::Session,
            "Explicitly save current session state and messages to disk",
        )
        .with_syntax("/session save")
        .with_aliases(&["/s save", "/save"])
        .with_priority(181)
        .with_examples(&["/session save"]),
    );

    actions.push(
        QuickAction::new(
            "export_md",
            "/export md",
            "Export to Markdown",
            QuickActionCategory::Session,
            "Save clean GitHub-flavored markdown transcript of current turn history",
        )
        .with_syntax("/export md [path]")
        .with_aliases(&["/exp md"])
        .with_priority(144)
        .with_examples(&["/export md", "/export md transcript.md"]),
    );

    actions.push(
        QuickAction::new(
            "export_html",
            "/export html",
            "Export to Standalone HTML",
            QuickActionCategory::Session,
            "Export dark-mode syntax-highlighted HTML conversation transcript",
        )
        .with_syntax("/export html [path]")
        .with_aliases(&["/exp html"])
        .with_priority(143)
        .with_examples(&["/export html", "/export html session.html"]),
    );

    actions.push(
        QuickAction::new(
            "advisors_toggle",
            "/advisors toggle",
            "Toggle Advisors On/Off",
            QuickActionCategory::Model,
            "Toggle parallel security, architecture, and performance advisors",
        )
        .with_syntax("/advisors toggle")
        .with_aliases(&["/adv toggle", "/advisors"])
        .with_priority(139)
        .with_examples(&["/advisors toggle"]),
    );

    actions.push(
        QuickAction::new(
            "preset_coding_fast",
            "/preset coding-fast",
            "Preset: Fast Coding",
            QuickActionCategory::Config,
            "High-speed configuration for rapid prototyping and iteration",
        )
        .with_syntax("/preset coding-fast")
        .with_priority(154)
        .with_examples(&["/preset coding-fast"]),
    );

    actions.push(
        QuickAction::new(
            "preset_deep_reasoning",
            "/preset deep-reasoning",
            "Preset: Deep Reasoning",
            QuickActionCategory::Config,
            "Maximum reasoning depth for complex architectural refactorings",
        )
        .with_syntax("/preset deep-reasoning")
        .with_priority(153)
        .with_examples(&["/preset deep-reasoning"]),
    );

    actions.push(
        QuickAction::new(
            "preset_offline_ollama",
            "/preset offline-ollama",
            "Preset: Offline / Local Ollama",
            QuickActionCategory::Config,
            "100% local offline execution using Ollama models",
        )
        .with_syntax("/preset offline-ollama")
        .with_priority(152)
        .with_examples(&["/preset offline-ollama"]),
    );

    actions.push(
        QuickAction::new(
            "skills_list",
            "/skills list",
            "List Domain Skills",
            QuickActionCategory::Config,
            "View all available domain skills from .fusion/skills/",
        )
        .with_syntax("/skills list")
        .with_aliases(&["/sk list"])
        .with_priority(121)
        .with_examples(&["/skills list"]),
    );

    actions.push(
        QuickAction::new(
            "skills_reload",
            "/skills reload",
            "Reload Domain Skills",
            QuickActionCategory::Config,
            "Rescan and hot-reload all skills from workspace and user directories",
        )
        .with_syntax("/skills reload")
        .with_aliases(&["/sk reload"])
        .with_priority(119)
        .with_examples(&["/skills reload"]),
    );

    // Sort by priority descending initially
    actions.sort_by(|a, b| b.priority.cmp(&a.priority));

    actions
}

// ---------------------------------------------------------------------------
// Fuzzy Matching Algorithm
// ---------------------------------------------------------------------------

/// Result of fuzzy matching a candidate string against a pattern.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FuzzyMatchResult {
    /// Match quality score (higher is better).
    pub score: i64,
    /// 0-based character indices where characters matched.
    pub matched_indices: Vec<usize>,
}

/// Pure-Rust fuzzy matching with subsequence alignment, boundary bonuses,
/// exact prefix bonuses, consecutive match bonuses, and case rewards.
pub fn fuzzy_match(pattern: &str, candidate: &str) -> Option<FuzzyMatchResult> {
    let clean_pattern = pattern.trim();
    if clean_pattern.is_empty() {
        return Some(FuzzyMatchResult {
            score: 0,
            matched_indices: Vec::new(),
        });
    }

    let pattern_chars: Vec<char> = clean_pattern.chars().collect();
    let candidate_chars: Vec<char> = candidate.chars().collect();

    if pattern_chars.len() > candidate_chars.len() {
        return None;
    }

    let pattern_lower: Vec<char> = clean_pattern.to_lowercase().chars().collect();
    let candidate_lower: Vec<char> = candidate.to_lowercase().chars().collect();

    // Fast-path subsequence check
    let mut p_iter = pattern_lower.iter();
    let mut curr_p = p_iter.next();
    for c in &candidate_lower {
        if let Some(target) = curr_p {
            if c == target {
                curr_p = p_iter.next();
            }
        } else {
            break;
        }
    }
    if curr_p.is_some() {
        return None; // Subsequence not found
    }

    // Dynamic scoring with bonuses
    let mut matched_indices = Vec::with_capacity(pattern_chars.len());
    let mut score: i64 = 0;
    let mut cand_idx = 0;

    // Check for exact substring match bonus
    let cand_str_lower: String = candidate_lower.iter().collect();
    let patt_str_lower: String = pattern_lower.iter().collect();
    if let Some(sub_pos) = cand_str_lower.find(&patt_str_lower) {
        score += 80;
        if sub_pos == 0 {
            score += 60; // Exact prefix match
        }
    }

    for (pi, &p_char) in pattern_chars.iter().enumerate() {
        let p_lower = pattern_lower[pi];
        let mut best_idx = None;
        let mut best_local_score = i64::MIN;

        while cand_idx < candidate_chars.len() {
            let c = candidate_chars[cand_idx];
            let c_lower = candidate_lower[cand_idx];

            if c_lower == p_lower {
                let mut char_score: i64 = 10;

                // Consecutive match bonus
                if let Some(&last_idx) = matched_indices.last() {
                    if cand_idx == last_idx + 1 {
                        char_score += 25;
                    }
                }

                // Word boundary bonus (after '/', ' ', '-', '_', ':')
                let is_boundary = if cand_idx == 0 {
                    true
                } else {
                    let prev = candidate_chars[cand_idx - 1];
                    prev == '/' || prev == ' ' || prev == '-' || prev == '_' || prev == ':'
                };

                if is_boundary {
                    char_score += 40;
                }

                // CamelCase boundary bonus
                if cand_idx > 0 {
                    let prev = candidate_chars[cand_idx - 1];
                    if prev.is_ascii_lowercase() && c.is_ascii_uppercase() {
                        char_score += 35;
                    }
                }

                // Exact case match bonus
                if p_char == c {
                    char_score += 8;
                }

                // Start position penalty (earlier matches score higher)
                char_score -= (cand_idx as i64) / 3;

                if char_score > best_local_score {
                    best_local_score = char_score;
                    best_idx = Some(cand_idx);
                }

                if is_boundary {
                    break;
                }
            }

            cand_idx += 1;
        }

        if let Some(chosen_idx) = best_idx {
            matched_indices.push(chosen_idx);
            score += best_local_score;
            cand_idx = chosen_idx + 1;
        } else {
            return None;
        }
    }

    // Length difference penalty
    let len_diff = candidate_chars.len().saturating_sub(pattern_chars.len());
    score -= (len_diff as i64) / 3;

    Some(FuzzyMatchResult {
        score,
        matched_indices,
    })
}

/// Evaluates match quality of an action against a query string.
/// Checks command name, title, aliases, and description with weighting.
pub fn fuzzy_score_action(pattern: &str, action: &QuickAction) -> Option<(i64, Vec<usize>)> {
    let clean_pattern = pattern.trim();
    if clean_pattern.is_empty() {
        return Some((action.priority as i64, Vec::new()));
    }

    // Strip leading slash from query if user typed it or didn't
    let query_no_slash = clean_pattern.trim_start_matches('/');
    let query_with_slash = if clean_pattern.starts_with('/') {
        clean_pattern.to_string()
    } else {
        format!("/{}", clean_pattern)
    };

    let mut best_score = i64::MIN;
    let mut best_indices = Vec::new();

    // 1. Match against command string (e.g. "/session list") - highest weight
    if let Some(res) = fuzzy_match(&query_with_slash, &action.command) {
        let score = res.score + 100 + (action.priority as i64 / 2);
        if score > best_score {
            best_score = score;
            best_indices = res.matched_indices;
        }
    }

    // Also match command without leading slash
    if let Some(res) = fuzzy_match(query_no_slash, action.command.trim_start_matches('/')) {
        let score = res.score + 90 + (action.priority as i64 / 2);
        if score > best_score {
            best_score = score;
            // Shift indices by 1 for command highlighting if slash was stripped
            best_indices = res.matched_indices.iter().map(|i| i + 1).collect();
        }
    }

    // 2. Match against title (e.g. "Switch LLM Model")
    if let Some(res) = fuzzy_match(query_no_slash, &action.title) {
        let score = res.score + 70 + (action.priority as i64 / 3);
        if score > best_score {
            best_score = score;
            // Clear command indices since match was on title
            best_indices = Vec::new();
        }
    }

    // 3. Match against aliases (e.g. "/s", "/undo", "/cost")
    for alias in &action.aliases {
        if let Some(res) = fuzzy_match(clean_pattern, alias) {
            let score = res.score + 80;
            if score > best_score {
                best_score = score;
            }
        }
    }

    // 4. Match against description
    if let Some(res) = fuzzy_match(query_no_slash, &action.description) {
        let score = res.score + 30;
        if score > best_score {
            best_score = score;
            best_indices = Vec::new();
        }
    }

    if best_score > i64::MIN {
        Some((best_score, best_indices))
    } else {
        None
    }
}

// ---------------------------------------------------------------------------
// Quick Action Picker Result
// ---------------------------------------------------------------------------

/// Outcome returned from interactive quick action picker.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum QuickActionResult {
    /// User selected an action (`Enter`).
    Selected(QuickAction),
    /// User selected an action resulting in direct command execution.
    Command(String),
    /// User dismissed or canceled the menu (`Esc` / `Ctrl+C`).
    Cancelled,
}

impl QuickActionResult {
    /// True if an action was selected.
    pub fn is_selected(&self) -> bool {
        matches!(self, QuickActionResult::Selected(_) | QuickActionResult::Command(_))
    }

    /// True if user canceled the menu.
    pub fn is_cancelled(&self) -> bool {
        matches!(self, QuickActionResult::Cancelled)
    }

    /// Command string to execute if selected.
    pub fn command(&self) -> Option<&str> {
        match self {
            QuickActionResult::Selected(a) => Some(&a.command),
            QuickActionResult::Command(c) => Some(c.as_str()),
            QuickActionResult::Cancelled => None,
        }
    }
}

// ---------------------------------------------------------------------------
// Interactive QuickActionsMenu Widget
// ---------------------------------------------------------------------------

/// Interactive Quick Actions Popup Menu Widget matching VS Code / Raycast / Telescope UX.
#[derive(Debug, Clone)]
pub struct QuickActionsMenu {
    /// All registered quick actions.
    actions: Vec<QuickAction>,
    /// Filtered action indices with score and matched char indices: `(action_index, score, matched_indices)`.
    filtered_indices: Vec<(usize, i64, Vec<usize>)>,
    /// Active category tab filter.
    active_tab: QuickActionCategory,
    /// Search query typed by user.
    query: String,
    /// Cursor position inside the search query.
    cursor_pos: usize,
    /// Selection index inside filtered list.
    selected_index: usize,
    /// Scroll offset for viewport list pagination.
    scroll_offset: usize,
    /// Header title.
    title: String,
    /// Whether outer border block is drawn.
    show_border: bool,
    /// Whether preview/detail pane is rendered at the bottom.
    show_preview: bool,
}

impl Default for QuickActionsMenu {
    fn default() -> Self {
        Self::new()
    }
}

impl QuickActionsMenu {
    /// Create a new `QuickActionsMenu` initialized with all default slash actions.
    pub fn new() -> Self {
        Self::from_actions(default_quick_actions())
    }

    /// Create a `QuickActionsMenu` with a custom set of actions.
    pub fn from_actions(actions: Vec<QuickAction>) -> Self {
        let mut menu = Self {
            actions,
            filtered_indices: Vec::new(),
            active_tab: QuickActionCategory::All,
            query: String::new(),
            cursor_pos: 0,
            selected_index: 0,
            scroll_offset: 0,
            title: "Quick Actions".to_string(),
            show_border: true,
            show_preview: true,
        };
        menu.refilter();
        menu
    }

    /// Builder: set custom title.
    pub fn with_title(mut self, title: impl Into<String>) -> Self {
        self.title = title.into();
        self
    }

    /// Builder: toggle outer borders.
    pub fn with_border(mut self, show_border: bool) -> Self {
        self.show_border = show_border;
        self
    }

    /// Builder: toggle preview details pane.
    pub fn with_preview(mut self, show_preview: bool) -> Self {
        self.show_preview = show_preview;
        self
    }

    /// Builder: set initial active category tab.
    pub fn with_category(mut self, category: QuickActionCategory) -> Self {
        self.active_tab = category;
        self.refilter();
        self
    }

    /// Builder: set initial query.
    pub fn with_initial_query(mut self, query: impl Into<String>) -> Self {
        self.set_query(query);
        self
    }

    /// Add a custom action to the menu.
    pub fn add_action(&mut self, action: QuickAction) {
        self.actions.push(action);
        self.refilter();
    }

    // -----------------------------------------------------------------------
    // State Accessors & Mutators
    // -----------------------------------------------------------------------

    /// Current search query.
    pub fn query(&self) -> &str {
        &self.query
    }

    /// Set search query and recompute filtered rankings.
    pub fn set_query(&mut self, query: impl Into<String>) {
        self.query = query.into();
        self.cursor_pos = self.query.chars().count();
        self.refilter();
    }

    /// Clear search query.
    pub fn clear_query(&mut self) {
        self.query.clear();
        self.cursor_pos = 0;
        self.refilter();
    }

    /// Insert character at cursor position in query.
    pub fn insert_char(&mut self, c: char) {
        let mut chars: Vec<char> = self.query.chars().collect();
        if self.cursor_pos <= chars.len() {
            chars.insert(self.cursor_pos, c);
            self.query = chars.into_iter().collect();
            self.cursor_pos += 1;
            self.refilter();
        }
    }

    /// Delete character before cursor in query (Backspace).
    pub fn backspace(&mut self) {
        if self.cursor_pos > 0 {
            let mut chars: Vec<char> = self.query.chars().collect();
            chars.remove(self.cursor_pos - 1);
            self.query = chars.into_iter().collect();
            self.cursor_pos -= 1;
            self.refilter();
        }
    }

    /// Delete character at cursor in query (Delete).
    pub fn delete_forward(&mut self) {
        let mut chars: Vec<char> = self.query.chars().collect();
        if self.cursor_pos < chars.len() {
            chars.remove(self.cursor_pos);
            self.query = chars.into_iter().collect();
            self.refilter();
        }
    }

    /// Delete previous word in query (Ctrl+W).
    pub fn delete_word(&mut self) {
        if self.cursor_pos == 0 {
            return;
        }
        let chars: Vec<char> = self.query.chars().collect();
        let mut new_pos = self.cursor_pos;

        // Skip spaces
        while new_pos > 0 && chars[new_pos - 1].is_whitespace() {
            new_pos -= 1;
        }
        // Skip word chars
        while new_pos > 0 && !chars[new_pos - 1].is_whitespace() {
            new_pos -= 1;
        }

        let mut new_chars = Vec::new();
        new_chars.extend_from_slice(&chars[..new_pos]);
        new_chars.extend_from_slice(&chars[self.cursor_pos..]);

        self.query = new_chars.into_iter().collect();
        self.cursor_pos = new_pos;
        self.refilter();
    }

    /// Set active category tab.
    pub fn set_tab(&mut self, tab: QuickActionCategory) {
        self.active_tab = tab;
        self.refilter();
    }

    /// Cycle to next category tab (`Tab`).
    pub fn next_tab(&mut self) {
        self.set_tab(self.active_tab.next());
    }

    /// Cycle to previous category tab (`Shift+Tab`).
    pub fn prev_tab(&mut self) {
        self.set_tab(self.active_tab.prev());
    }

    /// Active category tab.
    pub fn active_tab(&self) -> QuickActionCategory {
        self.active_tab
    }

    /// Recomputes fuzzy ranking and filters actions by query and category tab.
    pub fn refilter(&mut self) {
        let mut matches: Vec<(usize, i64, Vec<usize>)> = Vec::new();

        for (idx, action) in self.actions.iter().enumerate() {
            // Category tab filter
            if !self.active_tab.matches(&action.category) {
                continue;
            }

            // Fuzzy match query
            if let Some((score, indices)) = fuzzy_score_action(&self.query, action) {
                matches.push((idx, score, indices));
            }
        }

        // Sort descending by score; ties broken by priority then alphabetical
        matches.sort_by(|a, b| {
            b.1.cmp(&a.1)
                .then_with(|| self.actions[b.0].priority.cmp(&self.actions[a.0].priority))
                .then_with(|| self.actions[a.0].command.cmp(&self.actions[b.0].command))
        });

        self.filtered_indices = matches;
        self.selected_index = 0;
        self.scroll_offset = 0;
    }

    /// Total registered actions count.
    pub fn total_actions_count(&self) -> usize {
        self.actions.len()
    }

    /// Number of matching actions.
    pub fn matched_actions_count(&self) -> usize {
        self.filtered_indices.len()
    }

    /// Currently selected action.
    pub fn selected_action(&self) -> Option<&QuickAction> {
        self.filtered_indices
            .get(self.selected_index)
            .map(|(idx, _, _)| &self.actions[*idx])
    }

    /// All filtered actions in current view.
    pub fn filtered_actions(&self) -> Vec<&QuickAction> {
        self.filtered_indices
            .iter()
            .map(|(idx, _, _)| &self.actions[*idx])
            .collect()
    }

    // -----------------------------------------------------------------------
    // Selection Navigation
    // -----------------------------------------------------------------------

    /// Move selection cursor down.
    pub fn select_next(&mut self) {
        if !self.filtered_indices.is_empty() {
            self.selected_index = (self.selected_index + 1).min(self.filtered_indices.len() - 1);
        }
    }

    /// Move selection cursor up.
    pub fn select_prev(&mut self) {
        if self.selected_index > 0 {
            self.selected_index -= 1;
        }
    }

    /// Jump to first result.
    pub fn select_first(&mut self) {
        self.selected_index = 0;
    }

    /// Jump to last result.
    pub fn select_last(&mut self) {
        if !self.filtered_indices.is_empty() {
            self.selected_index = self.filtered_indices.len() - 1;
        }
    }

    /// Page down by N items.
    pub fn page_down(&mut self, page_size: usize) {
        if !self.filtered_indices.is_empty() {
            self.selected_index =
                (self.selected_index + page_size).min(self.filtered_indices.len() - 1);
        }
    }

    /// Page up by N items.
    pub fn page_up(&mut self, page_size: usize) {
        self.selected_index = self.selected_index.saturating_sub(page_size);
    }

    // -----------------------------------------------------------------------
    // Keyboard Event Handling
    // -----------------------------------------------------------------------

    /// Process a single keyboard event. Returns `Some(QuickActionResult)` when interactive session finishes.
    pub fn handle_key(&mut self, code: KeyCode, modifiers: KeyModifiers) -> Option<QuickActionResult> {
        // 1. Global Interruption / Cancellation: Esc / Ctrl+C
        if code == KeyCode::Esc || (code == KeyCode::Char('c') && modifiers.contains(KeyModifiers::CONTROL)) {
            return Some(QuickActionResult::Cancelled);
        }

        // 2. Selection & Execution: Enter
        if code == KeyCode::Enter {
            return if let Some(action) = self.selected_action() {
                Some(QuickActionResult::Selected(action.clone()))
            } else if !self.query.trim().is_empty() {
                // Execute typed command as fallback
                let cmd = if self.query.starts_with('/') {
                    self.query.clone()
                } else {
                    format!("/{}", self.query)
                };
                Some(QuickActionResult::Command(cmd))
            } else {
                Some(QuickActionResult::Cancelled)
            };
        }

        // 3. Tab Navigation: Tab / Shift+Tab
        if code == KeyCode::Tab {
            if modifiers.contains(KeyModifiers::SHIFT) {
                self.prev_tab();
            } else {
                self.next_tab();
            }
            return None;
        }
        if code == KeyCode::BackTab {
            self.prev_tab();
            return None;
        }

        // 4. Line Navigation: Up / Down / Ctrl+P / Ctrl+N
        if code == KeyCode::Up || (code == KeyCode::Char('p') && modifiers.contains(KeyModifiers::CONTROL)) {
            self.select_prev();
            return None;
        }
        if code == KeyCode::Down || (code == KeyCode::Char('n') && modifiers.contains(KeyModifiers::CONTROL)) {
            self.select_next();
            return None;
        }

        // 5. Page Navigation: PageUp / PageDown
        if code == KeyCode::PageUp {
            self.page_up(6);
            return None;
        }
        if code == KeyCode::PageDown {
            self.page_down(6);
            return None;
        }

        // 6. Home / End
        if code == KeyCode::Home {
            self.select_first();
            return None;
        }
        if code == KeyCode::End {
            self.select_last();
            return None;
        }

        // 7. Query Editing Shortcuts
        if code == KeyCode::Char('u') && modifiers.contains(KeyModifiers::CONTROL) {
            self.clear_query();
            return None;
        }
        if code == KeyCode::Char('w') && modifiers.contains(KeyModifiers::CONTROL) {
            self.delete_word();
            return None;
        }
        if code == KeyCode::Char('a') && modifiers.contains(KeyModifiers::CONTROL) {
            self.cursor_pos = 0;
            return None;
        }
        if code == KeyCode::Char('e') && modifiers.contains(KeyModifiers::CONTROL) {
            self.cursor_pos = self.query.chars().count();
            return None;
        }

        // 8. Cursor movements in search query
        if code == KeyCode::Left {
            if self.cursor_pos > 0 {
                self.cursor_pos -= 1;
            }
            return None;
        }
        if code == KeyCode::Right {
            if self.cursor_pos < self.query.chars().count() {
                self.cursor_pos += 1;
            }
            return None;
        }

        // 9. Backspace & Delete
        if code == KeyCode::Backspace {
            self.backspace();
            return None;
        }
        if code == KeyCode::Delete {
            self.delete_forward();
            return None;
        }

        // 10. Character input
        if let KeyCode::Char(c) = code {
            if !modifiers.contains(KeyModifiers::CONTROL) && !modifiers.contains(KeyModifiers::ALT) {
                self.insert_char(c);
                return None;
            }
        }

        None
    }

    // -----------------------------------------------------------------------
    // Interactive Execution Runner
    // -----------------------------------------------------------------------

    /// Runs the interactive quick actions popup menu inside an inline Ratatui viewport.
    pub fn run_interactive(
        &mut self,
        requested_height: Option<u16>,
    ) -> std::io::Result<Option<QuickAction>> {
        let _raw_guard = RawModeGuard::enter()?;
        let _ = execute!(stdout(), cursor::Hide);

        let height = requested_height.unwrap_or(12).clamp(8, 20);
        let mut inline = match InlineTerminal::new(height) {
            Ok(term) => term,
            Err(e) => {
                let _ = execute!(stdout(), cursor::Show);
                return Err(e);
            }
        };

        let outcome = loop {
            inline.draw(|f| {
                let size = f.area();
                let mut buf = f.buffer_mut();
                self.render_buffer(size, &mut buf);
            })?;

            // Poll for keyboard input (40ms polling for smooth responsive UX)
            if event::poll(std::time::Duration::from_millis(40))? {
                if let Event::Key(key) = event::read()? {
                    if key.kind == KeyEventKind::Release {
                        continue;
                    }

                    if let Some(result) = self.handle_key(key.code, key.modifiers) {
                        break match result {
                            QuickActionResult::Selected(action) => Some(action),
                            QuickActionResult::Command(cmd) => {
                                // Find or synthesize action
                                let synthesized = QuickAction::new(
                                    "custom",
                                    &cmd,
                                    &cmd,
                                    QuickActionCategory::Core,
                                    "User typed command",
                                );
                                Some(synthesized)
                            }
                            QuickActionResult::Cancelled => None,
                        };
                    }
                }
            }
        };

        // Clean up inline terminal and restore cursor
        let _ = inline.clear();
        let _ = inline.finish();
        let _ = execute!(stdout(), cursor::Show);
        let _ = stdout().flush();

        Ok(outcome)
    }

    /// Convenience runner returning the command string to execute.
    pub fn run_interactive_command(
        &mut self,
        requested_height: Option<u16>,
    ) -> std::io::Result<Option<String>> {
        self.run_interactive(requested_height)
            .map(|opt| opt.map(|a| a.command))
    }

    // -----------------------------------------------------------------------
    // Ratatui Buffer Rendering
    // -----------------------------------------------------------------------

    /// Renders the entire quick actions menu into the given Ratatui `Buffer`.
    pub fn render_buffer(&self, area: Rect, buf: &mut Buffer) {
        if area.width < 10 || area.height < 4 {
            return;
        }

        // Draw outer block
        let inner_area = if self.show_border {
            let block = Block::default()
                .title(Line::from(vec![
                    Span::styled("⚡ ", Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)),
                    Span::styled(&self.title, Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)),
                    Span::styled(
                        format!(" ({}/{})", self.filtered_indices.len(), self.actions.len()),
                        Style::default().fg(Color::DarkGray),
                    ),
                ]))
                .borders(Borders::ALL)
                .border_type(BorderType::Rounded)
                .border_style(Style::default().fg(Color::Cyan));
            let inner = block.inner(area);
            block.render(area, buf);
            inner
        } else {
            area
        };

        if inner_area.height < 3 {
            return;
        }

        // Layout: Tabs, Search Box, List, Optional Preview, Footer
        let show_preview_pane = self.show_preview && inner_area.height >= 9;

        let constraints = if show_preview_pane {
            vec![
                Constraint::Length(1), // Tabs bar
                Constraint::Length(1), // Search Input box
                Constraint::Min(3),    // Action List
                Constraint::Length(1), // Divider
                Constraint::Length(1), // Preview details pane
                Constraint::Length(1), // Footer key hints
            ]
        } else {
            vec![
                Constraint::Length(1), // Tabs bar
                Constraint::Length(1), // Search Input box
                Constraint::Min(2),    // Action List
                Constraint::Length(1), // Footer key hints
            ]
        };

        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints(constraints)
            .split(inner_area);

        let tabs_area = chunks[0];
        let search_area = chunks[1];
        let list_area = chunks[2];

        // 1. Render Category Tabs
        self.render_tabs(tabs_area, buf);

        // 2. Render Search Input Box
        self.render_search(search_area, buf);

        // 3. Render Actions List
        self.render_list(list_area, buf);

        // 4. Render Preview Details or Footer
        if show_preview_pane {
            let div_area = chunks[3];
            let prev_area = chunks[4];
            let foot_area = chunks[5];

            render_horizontal_divider(buf, div_area, Color::DarkGray);
            self.render_preview(prev_area, buf);
            self.render_footer(foot_area, buf);
        } else {
            let foot_area = chunks[3];
            self.render_footer(foot_area, buf);
        }
    }

    /// Render category tabs bar: `[ All ] [ Core ] [ Session ] [ Model ] [ Config ]`
    fn render_tabs(&self, area: Rect, buf: &mut Buffer) {
        let is_narrow = area.width < 50;
        let mut spans = Vec::new();
        spans.push(Span::raw(" "));

        for (idx, tab) in QuickActionCategory::ALL.iter().enumerate() {
            let is_active = *tab == self.active_tab;
            let label = if is_narrow {
                tab.short_name()
            } else {
                tab.name()
            };

            if is_active {
                spans.push(Span::styled(
                    format!("[{}]", label),
                    Style::default()
                        .fg(tab.color())
                        .add_modifier(Modifier::BOLD),
                ));
            } else {
                spans.push(Span::styled(
                    label.to_string(),
                    Style::default().fg(Color::DarkGray),
                ));
            }

            if idx + 1 < QuickActionCategory::ALL.len() {
                spans.push(Span::raw("  "));
            }
        }

        let paragraph = Paragraph::new(Line::from(spans));
        paragraph.render(area, buf);
    }

    /// Render search input prompt row: `❯ /search query`
    fn render_search(&self, area: Rect, buf: &mut Buffer) {
        let mut spans = Vec::new();
        spans.push(Span::raw(" "));
        spans.push(Span::styled(
            "❯ ",
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ));

        if self.query.is_empty() {
            spans.push(Span::styled(
                "Type a command or filter actions... (/ for slash commands)",
                Style::default().fg(Color::DarkGray),
            ));
        } else {
            let query_chars: Vec<char> = self.query.chars().collect();
            let before: String = query_chars[..self.cursor_pos].iter().collect();
            let cursor_char = if self.cursor_pos < query_chars.len() {
                query_chars[self.cursor_pos].to_string()
            } else {
                " ".to_string()
            };
            let after: String = if self.cursor_pos + 1 < query_chars.len() {
                query_chars[self.cursor_pos + 1..].iter().collect()
            } else {
                String::new()
            };

            spans.push(Span::styled(
                before,
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            ));
            spans.push(Span::styled(
                cursor_char,
                Style::default()
                    .fg(Color::Black)
                    .bg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            ));
            spans.push(Span::styled(
                after,
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            ));
        }

        let paragraph = Paragraph::new(Line::from(spans));
        paragraph.render(area, buf);
    }

    /// Render scrollable action rows.
    fn render_list(&self, area: Rect, buf: &mut Buffer) {
        let visible_rows = area.height as usize;
        if visible_rows == 0 {
            return;
        }

        if self.filtered_indices.is_empty() {
            let msg = if !self.query.is_empty() {
                format!("   No actions matching '{}'", self.query)
            } else {
                format!("   No actions in [{}] category", self.active_tab.name())
            };
            let paragraph = Paragraph::new(Line::from(vec![Span::styled(
                msg,
                Style::default().fg(Color::DarkGray),
            )]));
            paragraph.render(area, buf);
            return;
        }

        // Adjust scroll window
        let mut scroll = self.scroll_offset;
        if self.selected_index < scroll {
            scroll = self.selected_index;
        } else if self.selected_index >= scroll + visible_rows {
            scroll = self.selected_index - visible_rows + 1;
        }

        let end = (scroll + visible_rows).min(self.filtered_indices.len());
        let visible_slice = &self.filtered_indices[scroll..end];
        let width = area.width;
        let mut lines = Vec::new();

        for (rel_idx, (action_idx, _score, matched_indices)) in visible_slice.iter().enumerate() {
            let actual_idx = scroll + rel_idx;
            let is_selected = actual_idx == self.selected_index;
            let action = &self.actions[*action_idx];
            let line = format_action_row(action, is_selected, matched_indices, width);
            lines.push(line);
        }

        let paragraph = Paragraph::new(lines);
        paragraph.render(area, buf);
    }

    /// Render preview / syntax details for selected action.
    fn render_preview(&self, area: Rect, buf: &mut Buffer) {
        if let Some(action) = self.selected_action() {
            let mut spans = Vec::new();
            spans.push(Span::raw(" "));
            spans.push(Span::styled(
                "Syntax: ",
                Style::default().fg(Color::DarkGray),
            ));
            spans.push(Span::styled(
                &action.syntax,
                Style::default().fg(Color::White).add_modifier(Modifier::BOLD),
            ));

            if !action.aliases.is_empty() {
                spans.push(Span::raw("  "));
                spans.push(Span::styled(
                    "Aliases: ",
                    Style::default().fg(Color::DarkGray),
                ));
                spans.push(Span::styled(
                    action.aliases.join(", "),
                    Style::default().fg(Color::Cyan),
                ));
            }

            if let Some(ex) = action.examples.first() {
                spans.push(Span::raw("  "));
                spans.push(Span::styled(
                    "Example: ",
                    Style::default().fg(Color::DarkGray),
                ));
                spans.push(Span::styled(
                    ex.as_str(),
                    Style::default().fg(Color::Green),
                ));
            }

            let paragraph = Paragraph::new(Line::from(spans));
            paragraph.render(area, buf);
        }
    }

    /// Render key hints footer: `↑↓ Navigate  Tab Category  Enter Execute  Esc Close`
    fn render_footer(&self, area: Rect, buf: &mut Buffer) {
        let is_compact = area.width < 50;

        let footer_line = if is_compact {
            Line::from(vec![
                Span::raw(" "),
                Span::styled("↑↓ ", Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)),
                Span::styled("Nav ", Style::default().fg(Color::DarkGray)),
                Span::styled("Tab ", Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)),
                Span::styled("Cat ", Style::default().fg(Color::DarkGray)),
                Span::styled("↵ ", Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)),
                Span::styled("Run ", Style::default().fg(Color::DarkGray)),
                Span::styled("Esc ", Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)),
                Span::styled("Close", Style::default().fg(Color::DarkGray)),
            ])
        } else {
            Line::from(vec![
                Span::raw(" "),
                Span::styled("↑↓/Ctrl+P/N ", Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)),
                Span::styled("Navigate", Style::default().fg(Color::DarkGray)),
                Span::raw("   "),
                Span::styled("Tab ", Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)),
                Span::styled("Category", Style::default().fg(Color::DarkGray)),
                Span::raw("   "),
                Span::styled("Enter ", Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)),
                Span::styled("Execute", Style::default().fg(Color::DarkGray)),
                Span::raw("   "),
                Span::styled("Esc ", Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)),
                Span::styled("Close", Style::default().fg(Color::DarkGray)),
            ])
        };

        let paragraph = Paragraph::new(footer_line);
        paragraph.render(area, buf);
    }
}

// ---------------------------------------------------------------------------
// Ratatui Widget Trait Implementations
// ---------------------------------------------------------------------------

impl Widget for &QuickActionsMenu {
    fn render(self, area: Rect, buf: &mut Buffer) {
        self.render_buffer(area, buf);
    }
}

impl Widget for QuickActionsMenu {
    fn render(self, area: Rect, buf: &mut Buffer) {
        self.render_buffer(area, buf);
    }
}

// ---------------------------------------------------------------------------
// Row Formatting Helpers
// ---------------------------------------------------------------------------

/// Formats a single quick action row with selection cursor, category badge,
/// command with highlighted characters, title, description, and shortcut tag.
pub fn format_action_row<'a>(
    action: &'a QuickAction,
    is_selected: bool,
    matched_indices: &[usize],
    width: u16,
) -> Line<'a> {
    let mut spans = Vec::new();

    // 1. Selection indicator pointer
    if is_selected {
        spans.push(Span::styled(
            "❯ ",
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ));
    } else {
        spans.push(Span::raw("  "));
    }

    // 2. Category badge: `[CORE]`, `[SESS]`, etc.
    let badge_style = if is_selected {
        Style::default()
            .fg(action.category.color())
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(action.category.color())
    };
    spans.push(Span::styled(action.category.badge(), badge_style));
    spans.push(Span::raw(" "));

    // 3. Command string with matched character highlighting
    let cmd_chars: Vec<char> = action.command.chars().collect();
    let mut curr_chunk = String::new();
    let mut curr_matched = false;

    for (idx, &ch) in cmd_chars.iter().enumerate() {
        let is_m = matched_indices.contains(&idx);
        if is_m != curr_matched {
            if !curr_chunk.is_empty() {
                spans.push(create_cmd_span(
                    std::mem::take(&mut curr_chunk),
                    curr_matched,
                    is_selected,
                ));
            }
            curr_matched = is_m;
        }
        curr_chunk.push(ch);
    }

    if !curr_chunk.is_empty() {
        spans.push(create_cmd_span(curr_chunk, curr_matched, is_selected));
    }

    // 4. Separator
    spans.push(Span::raw(" "));
    spans.push(Span::styled(
        "•",
        Style::default().fg(Color::DarkGray),
    ));
    spans.push(Span::raw(" "));

    // 5. Title / Description
    let desc_text = if !action.title.is_empty() {
        format!("{} - {}", action.title, action.description)
    } else {
        action.description.clone()
    };

    let shortcut_width = if let Some(sc) = &action.shortcut {
        sc.chars().count() + 2
    } else {
        0
    };

    let used_width: usize = spans.iter().map(|s| s.content.chars().count()).sum();
    let available_desc = (width as usize).saturating_sub(used_width + shortcut_width + 1);

    if available_desc > 3 {
        let truncated_desc: String = if desc_text.chars().count() > available_desc {
            let mut s: String = desc_text.chars().take(available_desc.saturating_sub(3)).collect();
            s.push_str("...");
            s
        } else {
            desc_text
        };

        let desc_style = if is_selected {
            Style::default().fg(Color::White)
        } else {
            Style::default().fg(Color::DarkGray)
        };
        spans.push(Span::styled(truncated_desc, desc_style));
    }

    // 6. Right-aligned shortcut keybadge (e.g. `Ctrl+P`)
    if let Some(sc) = &action.shortcut {
        let current_len: usize = spans.iter().map(|s| s.content.chars().count()).sum();
        let pad = (width as usize).saturating_sub(current_len + shortcut_width);
        if pad > 0 {
            spans.push(Span::raw(" ".repeat(pad)));
        }
        spans.push(Span::raw(" "));
        spans.push(Span::styled(
            sc.as_str(),
            Style::default()
                .fg(Color::Rgb(120, 120, 120))
                .add_modifier(Modifier::DIM),
        ));
    }

    Line::from(spans)
}

fn create_cmd_span(text: String, is_match: bool, is_selected: bool) -> Span<'static> {
    let style = if is_match {
        Style::default()
            .fg(Color::Yellow)
            .add_modifier(Modifier::BOLD | Modifier::UNDERLINED)
    } else if is_selected {
        Style::default()
            .fg(Color::Cyan)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(Color::Cyan)
    };
    Span::styled(text, style)
}

/// Helper to render a thin horizontal line divider across a Rect.
fn render_horizontal_divider(buf: &mut Buffer, area: Rect, color: Color) {
    if area.height == 0 || area.width == 0 {
        return;
    }
    let divider = "─".repeat(area.width as usize);
    let paragraph = Paragraph::new(Line::from(vec![Span::styled(
        divider,
        Style::default().fg(color),
    )]));
    paragraph.render(area, buf);
}

// ---------------------------------------------------------------------------
// Convenience Helper Functions
// ---------------------------------------------------------------------------

/// Convenience function to launch interactive quick actions popup menu.
pub fn pick_quick_action() -> std::io::Result<Option<QuickAction>> {
    let mut menu = QuickActionsMenu::new();
    menu.run_interactive(None)
}

/// Convenience function to launch interactive quick actions menu and return the chosen slash command string.
pub fn pick_slash_command() -> std::io::Result<Option<String>> {
    let mut menu = QuickActionsMenu::new();
    menu.run_interactive_command(None)
}

// ---------------------------------------------------------------------------
// Unit Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_quick_action_category_cycling() {
        let tab = QuickActionCategory::All;
        assert_eq!(tab.next(), QuickActionCategory::Core);
        assert_eq!(tab.next().next(), QuickActionCategory::Session);
        assert_eq!(tab.next().next().next(), QuickActionCategory::Model);
        assert_eq!(tab.next().next().next().next(), QuickActionCategory::Config);
        assert_eq!(tab.next().next().next().next().next(), QuickActionCategory::All);

        assert_eq!(QuickActionCategory::All.prev(), QuickActionCategory::Config);
        assert_eq!(QuickActionCategory::Config.prev(), QuickActionCategory::Model);
    }

    #[test]
    fn test_category_matching() {
        assert!(QuickActionCategory::All.matches(&QuickActionCategory::Core));
        assert!(QuickActionCategory::All.matches(&QuickActionCategory::Model));
        assert!(QuickActionCategory::Core.matches(&QuickActionCategory::Core));
        assert!(!QuickActionCategory::Core.matches(&QuickActionCategory::Session));
    }

    #[test]
    fn test_default_quick_actions_catalogue() {
        let actions = default_quick_actions();
        assert!(!actions.is_empty());

        let has_help = actions.iter().any(|a| a.command == "/help");
        let has_file = actions.iter().any(|a| a.command == "/file");
        let has_model = actions.iter().any(|a| a.command == "/model");
        let has_session_new = actions.iter().any(|a| a.command == "/session new");

        assert!(has_help, "Should contain /help action");
        assert!(has_file, "Should contain /file action");
        assert!(has_model, "Should contain /model action");
        assert!(has_session_new, "Should contain /session new action");
    }

    #[test]
    fn test_fuzzy_match_exact_and_prefix() {
        let res = fuzzy_match("/model", "/model").expect("Exact match should succeed");
        assert!(res.score > 0);
        assert_eq!(res.matched_indices, vec![0, 1, 2, 3, 4, 5]);

        let res_prefix = fuzzy_match("mod", "/model").expect("Prefix match should succeed");
        assert!(res_prefix.score > 0);
    }

    #[test]
    fn test_fuzzy_match_subsequence() {
        let res = fuzzy_match("hlp", "/help").expect("Subsequence should match");
        assert!(res.score > 0);

        let res_non = fuzzy_match("xyz", "/help");
        assert!(res_non.is_none());
    }

    #[test]
    fn test_fuzzy_score_action() {
        let action = QuickAction::new(
            "model",
            "/model",
            "Switch Model",
            QuickActionCategory::Model,
            "Inspect or switch active LLM completion model",
        );

        let score_cmd = fuzzy_score_action("/model", &action);
        assert!(score_cmd.is_some());

        let score_no_slash = fuzzy_score_action("model", &action);
        assert!(score_no_slash.is_some());

        let score_title = fuzzy_score_action("switch", &action);
        assert!(score_title.is_some());
    }

    #[test]
    fn test_quick_menu_filtering_and_query() {
        let mut menu = QuickActionsMenu::new();
        let total = menu.total_actions_count();
        assert!(total > 0);

        menu.set_query("file");
        assert!(menu.matched_actions_count() > 0);
        let selected = menu.selected_action().expect("Should have match");
        assert!(selected.command.contains("file"));

        menu.clear_query();
        assert_eq!(menu.matched_actions_count(), total);
    }

    #[test]
    fn test_quick_menu_category_filtering() {
        let mut menu = QuickActionsMenu::new();
        menu.set_tab(QuickActionCategory::Session);

        for action in menu.filtered_actions() {
            assert_eq!(action.category, QuickActionCategory::Session);
        }
    }

    #[test]
    fn test_quick_menu_navigation_and_selection() {
        let mut menu = QuickActionsMenu::new();
        assert_eq!(menu.selected_index, 0);

        menu.select_next();
        assert_eq!(menu.selected_index, 1);

        menu.select_prev();
        assert_eq!(menu.selected_index, 0);

        menu.select_prev(); // Clamp at 0
        assert_eq!(menu.selected_index, 0);

        menu.select_last();
        assert_eq!(menu.selected_index, menu.matched_actions_count() - 1);

        menu.select_first();
        assert_eq!(menu.selected_index, 0);
    }

    #[test]
    fn test_quick_menu_keys_handling() {
        let mut menu = QuickActionsMenu::new();

        // Type 'q'
        let res = menu.handle_key(KeyCode::Char('q'), KeyModifiers::NONE);
        assert!(res.is_none());
        assert_eq!(menu.query(), "q");

        // Type 'u'
        menu.handle_key(KeyCode::Char('u'), KeyModifiers::NONE);
        assert_eq!(menu.query(), "qu");

        // Backspace
        menu.handle_key(KeyCode::Backspace, KeyModifiers::NONE);
        assert_eq!(menu.query(), "q");

        // Enter on match
        menu.set_query("quit");
        let res_enter = menu.handle_key(KeyCode::Enter, KeyModifiers::NONE);
        assert!(matches!(res_enter, Some(QuickActionResult::Selected(_))));

        // Esc cancel
        let res_esc = menu.handle_key(KeyCode::Esc, KeyModifiers::NONE);
        assert_eq!(res_esc, Some(QuickActionResult::Cancelled));
    }

    #[test]
    fn test_quick_menu_buffer_render() {
        let menu = QuickActionsMenu::new();
        let area = Rect::new(0, 0, 80, 12);
        let mut buf = Buffer::empty(area);

        menu.render_buffer(area, &mut buf);

        // Verify title rendered in buffer
        let text: String = (0..area.width)
            .map(|x| buf[(x, 0)].symbol().to_string())
            .collect();
        assert!(text.contains("Quick Actions"));
    }

    #[test]
    fn test_format_action_row() {
        let action = QuickAction::new(
            "model",
            "/model",
            "Switch Model",
            QuickActionCategory::Model,
            "Inspect or switch active LLM completion model",
        )
        .with_shortcut("Ctrl+M");

        let line = format_action_row(&action, true, &[1, 2], 80);
        assert!(!line.spans.is_empty());
    }
}
