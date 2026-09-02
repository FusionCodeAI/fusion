//! Interactive Visual Context Inspector & Token Budget Widget
//!
//! Provides a high-polish, keyboard-driven context window visualization and diagnostic inspector:
//! - **Visual Token Distribution**: Real-time stacked horizontal bar and progress meter displaying
//!   proportions allocated across **System** (prompts/instructions), **History** (messages & turns),
//!   **Tools** (schemas & MCP definitions), and **Free Budget** (unallocated headroom).
//! - **Token Budget & Capacity Tracking**:
//!   1. Displays total context window capacity (e.g. 128k / 200k tokens).
//!   2. Tracks prompt tokens, completion tokens, total session tokens, and turn counts.
//!   3. Monitors provider cache usage (cache read tokens, cache write tokens, hit rates).
//! - **Utilization Progress Bar & Color Alerts**:
//!   - Green: `< 60%` context utilization (Safe / Optimal operating margin).
//!   - Yellow: `60% - 85%` context utilization (Warning / Approaching threshold).
//!   - Red: `> 85%` context utilization (Critical / Immediate compaction recommended).
//!   - Bold Red: `> 100%` context overflow.
//! - **Active System Prompt Breakdown**:
//!   - **Base Instructions**: System role, persona, runtime directives, and output formatting rules.
//!   - **Tool Definitions**: Registered tool schemas, JSON signatures, and execution guidelines.
//!   - **Skills**: Installed agent skills, specialized capabilities, and workflow recipes.
//!   - **Memory**: Workspace context, bookmark state, project metadata, and persistent memory.
//! - **Interactive Multi-Tab TUI**:
//!   1. `[ 1. Overview ]`: Stacked visual bar, 4 category metric cards, token budget & cache stats,
//!      health diagnostics, and top message consumers.
//!   2. `[ 2. System ]`: 4-pillar category summary (Base, Tools, Skills, Memory) and granular section breakdown.
//!   3. `[ 3. History ]`: Scrollable interactive message table with turn index, role badges,
//!      exact/estimated token weights, percentage shares, and preview text.
//!   4. `[ 4. Tools ]`: Registered tool definitions with parameter counts, schema token costs,
//!      categories, and descriptions.
//!   5. `[ 5. Compaction ]`: Real-time simulation comparing Current vs Compacted context with
//!      projected token savings and headroom expansion.
//! - **Multi-Format Rendering**:
//!   - `ContextInspectorWidget`: Full Ratatui widget for rich inline or alternate-screen viewports.
//!   - `ContextBarWidget`: Lightweight embeddable multi-segment bar for status lines and headers.
//!   - `ContextProgressBarWidget`: Single utilization progress bar with color alerts.
//!   - `render_context_inspector_ansi`: Zero-dependency, pure-Rust ANSI terminal string formatter.
//!   - `render_context_bar_ansi` / `render_utilization_bar_ansi`: Compact colored horizontal bar strings.
//! - **Keyboard Controls**: `Tab`/`Shift+Tab`/`1-5` to switch tabs, `↑`/`↓`/`j`/`k` to navigate,
//!   `Enter`/`Space` to expand/inspect, `c` to preview compaction, `?` for help, `Esc`/`q` to close.

use std::cmp::Ordering;
use std::collections::HashMap;
use std::fmt;
use std::io::{stdout, Stdout};

use crossterm::{
    cursor,
    event::{self, Event, KeyCode, KeyEventKind, KeyModifiers},
    execute,
    terminal::{self, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{
    backend::CrosstermBackend,
    buffer::Buffer,
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Clear, Paragraph, Widget, Wrap},
    Frame, Terminal, TerminalOptions, Viewport,
};
use serde::{Deserialize, Serialize};

use crate::agent::compaction::{estimate_messages_tokens, group_into_turns, Compactor};
use crate::agent::session::{Session, TokenStats};
use crate::agent::tokens::{
    estimate_text_tokens, estimate_tool_definition_tokens, estimate_tools_tokens,
    format_token_count, model_context_limit, ContextBreakdown, ContextBudget, TokenCount,
    DEFAULT_RESERVED_COMPLETION, DEFAULT_SAFETY_MARGIN,
};
use crate::provider::types::{Message, Role, ToolDefinition};
use crate::ui::budget::ContextAlertLevel;
use crate::ui::prompt::RawModeGuard;
use crate::ui::theme::Theme;

// ---------------------------------------------------------------------------
// 1. Constants & Thresholds
// ---------------------------------------------------------------------------

/// Default bar width when rendering ASCII/ANSI progress meters.
pub const DEFAULT_CONTEXT_BAR_WIDTH: usize = 36;

/// Default minimum width for full inspector dialog.
pub const MIN_INSPECTOR_WIDTH: u16 = 50;

/// Default minimum height for full inspector dialog.
pub const MIN_INSPECTOR_HEIGHT: u16 = 14;

/// Safe utilization threshold (< 60.0%, Green).
pub const UTILIZATION_SAFE_THRESHOLD: f32 = 60.0;

/// Warning utilization threshold (60.0% to 85.0%, Yellow).
pub const UTILIZATION_WARNING_THRESHOLD: f32 = 85.0;

/// Overflow utilization threshold (> 100.0%, Red / Overflow).
pub const UTILIZATION_OVERFLOW_THRESHOLD: f32 = 100.0;

// Block characters for stacked progress bar segments
pub const BLOCK_FULL: &str = "█";
pub const BLOCK_SEVEN_EIGHTHS: &str = "▉";
pub const BLOCK_THREE_QUARTERS: &str = "▊";
pub const BLOCK_FIVE_EIGHTHS: &str = "▋";
pub const BLOCK_HALF: &str = "▌";
pub const BLOCK_THREE_EIGHTHS: &str = "▍";
pub const BLOCK_ONE_QUARTER: &str = "▎";
pub const BLOCK_ONE_EIGHTH: &str = "▏";
pub const BLOCK_DARK_SHADE: &str = "▓";
pub const BLOCK_MEDIUM_SHADE: &str = "▒";
pub const BLOCK_LIGHT_SHADE: &str = "░";
pub const BLOCK_EMPTY: &str = "░";

// ANSI Terminal Colors
const ANSI_RESET: &str = "\x1b[0m";
const ANSI_BOLD: &str = "\x1b[1m";
const ANSI_DIM: &str = "\x1b[2m";
const ANSI_ITALIC: &str = "\x1b[3m";
const ANSI_UNDERLINE: &str = "\x1b[4m";

// Category Palette (TokyoNight / Modern Dark inspired)
const ANSI_CYAN: &str = "\x1b[36m";
const ANSI_BOLD_CYAN: &str = "\x1b[1;36m";
const ANSI_MAGENTA: &str = "\x1b[35m";
const ANSI_BOLD_MAGENTA: &str = "\x1b[1;35m";
const ANSI_YELLOW: &str = "\x1b[33m";
const ANSI_BOLD_YELLOW: &str = "\x1b[1;33m";
const ANSI_GREEN: &str = "\x1b[32m";
const ANSI_BOLD_GREEN: &str = "\x1b[1;32m";
const ANSI_BLUE: &str = "\x1b[34m";
const ANSI_BOLD_BLUE: &str = "\x1b[1;34m";
const ANSI_RED: &str = "\x1b[31m";
const ANSI_BOLD_RED: &str = "\x1b[1;31m";
const ANSI_GRAY: &str = "\x1b[90m";
const ANSI_WHITE: &str = "\x1b[37m";
const ANSI_BOLD_WHITE: &str = "\x1b[1;37m";

// ---------------------------------------------------------------------------
// 2. Utilization Alert Level (Color Alert Rules: <60% Green, 60-85% Yellow, >85% Red)
// ---------------------------------------------------------------------------

/// Utilization severity alert based on percentage thresholds:
/// - `Safe` (< 60%): Green
/// - `Warning` (60% - 85%): Yellow
/// - `Critical` (> 85%): Red
/// - `Overflow` (> 100%): Bold Red
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum UtilizationAlertLevel {
    /// Token usage is within safe operating margins (< 60%, Green).
    Safe,
    /// Token usage is in warning territory (60% - 85%, Yellow).
    Warning,
    /// Token usage is in critical danger territory (> 85%, Red).
    Critical,
    /// Token usage has exceeded 100% of context window (Bold Red).
    Overflow,
}

impl Default for UtilizationAlertLevel {
    fn default() -> Self {
        Self::Safe
    }
}

impl UtilizationAlertLevel {
    /// Classifies context utilization percentage into an alert level.
    pub fn from_percentage(pct: f32) -> Self {
        if pct >= UTILIZATION_OVERFLOW_THRESHOLD {
            Self::Overflow
        } else if pct > UTILIZATION_WARNING_THRESHOLD {
            Self::Critical
        } else if pct >= UTILIZATION_SAFE_THRESHOLD {
            Self::Warning
        } else {
            Self::Safe
        }
    }

    /// Classifies utilization ratio (0.0 to 1.0+).
    pub fn from_ratio(ratio: f32) -> Self {
        Self::from_percentage(ratio * 100.0)
    }

    /// Returns `true` if this level represents a warning or critical condition.
    #[inline]
    pub fn is_alert(&self) -> bool {
        matches!(self, Self::Warning | Self::Critical | Self::Overflow)
    }

    /// Primary Ratatui color associated with this alert level.
    pub fn color(&self) -> Color {
        match self {
            Self::Safe => Color::Green,
            Self::Warning => Color::Yellow,
            Self::Critical => Color::Red,
            Self::Overflow => Color::Rgb(255, 60, 60),
        }
    }

    /// Primary ANSI color escape sequence associated with this alert level.
    pub fn ansi_color(&self) -> &'static str {
        match self {
            Self::Safe => ANSI_BOLD_GREEN,
            Self::Warning => ANSI_BOLD_YELLOW,
            Self::Critical => ANSI_BOLD_RED,
            Self::Overflow => ANSI_BOLD_RED,
        }
    }

    /// Formatted status badge string for UI headers and banners.
    pub fn badge(&self) -> &'static str {
        match self {
            Self::Safe => "[✓ SAFE <60%]",
            Self::Warning => "[⚠ WARN 60-85%]",
            Self::Critical => "[🔥 DANGER >85%]",
            Self::Overflow => "[🚨 OVERFLOW >100%]",
        }
    }

    /// Human-readable description of this alert status.
    pub fn label(&self) -> &'static str {
        match self {
            Self::Safe => "Safe Operating Margin (<60%)",
            Self::Warning => "Moderate Warning (60-85%)",
            Self::Critical => "Critical Utilization (>85%)",
            Self::Overflow => "Context Limit Overflow (>100%)",
        }
    }
}

// ---------------------------------------------------------------------------
// 3. Context Category Definition
// ---------------------------------------------------------------------------

/// Context window partition category.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ContextCategory {
    /// System prompt, custom instructions, persona, and runtime environment.
    System,
    /// Conversation history, user queries, assistant responses, and tool results.
    History,
    /// Registered tool definitions, JSON schemas, and MCP capabilities.
    Tools,
    /// Remaining unallocated token headroom before context limit.
    Free,
}

impl ContextCategory {
    /// Ordered list of all 4 primary context categories.
    pub const ALL: [ContextCategory; 4] = [
        ContextCategory::System,
        ContextCategory::History,
        ContextCategory::Tools,
        ContextCategory::Free,
    ];

    /// Human-readable category name.
    pub fn name(&self) -> &'static str {
        match self {
            ContextCategory::System => "System",
            ContextCategory::History => "History",
            ContextCategory::Tools => "Tools",
            ContextCategory::Free => "Free Budget",
        }
    }

    /// Short label for compact displays.
    pub fn short_name(&self) -> &'static str {
        match self {
            ContextCategory::System => "Sys",
            ContextCategory::History => "Hist",
            ContextCategory::Tools => "Tools",
            ContextCategory::Free => "Free",
        }
    }

    /// Unicode icon / emoji glyph.
    pub fn icon(&self) -> &'static str {
        match self {
            ContextCategory::System => "🖥️",
            ContextCategory::History => "💬",
            ContextCategory::Tools => "🔧",
            ContextCategory::Free => "🟢",
        }
    }

    /// Short badge tag string (e.g. `[SYS]`).
    pub fn badge(&self) -> &'static str {
        match self {
            ContextCategory::System => "[SYSTEM]",
            ContextCategory::History => "[HISTORY]",
            ContextCategory::Tools => "[TOOLS]",
            ContextCategory::Free => "[FREE]",
        }
    }

    /// Primary Ratatui color associated with this category.
    pub fn ratatui_color(&self, _theme: &Theme) -> Color {
        match self {
            ContextCategory::System => Color::Cyan,
            ContextCategory::History => Color::Magenta,
            ContextCategory::Tools => Color::Yellow,
            ContextCategory::Free => Color::Green,
        }
    }

    /// Secondary / Accent Ratatui color.
    pub fn ratatui_accent_color(&self) -> Color {
        match self {
            ContextCategory::System => Color::Rgb(56, 189, 248),   // Light Sky Blue
            ContextCategory::History => Color::Rgb(192, 132, 252), // Light Purple
            ContextCategory::Tools => Color::Rgb(250, 204, 21),   // Light Amber
            ContextCategory::Free => Color::Rgb(74, 222, 128),    // Light Emerald
        }
    }

    /// ANSI color escape sequence.
    pub fn ansi_color(&self) -> &'static str {
        match self {
            ContextCategory::System => ANSI_BOLD_CYAN,
            ContextCategory::History => ANSI_BOLD_MAGENTA,
            ContextCategory::Tools => ANSI_BOLD_YELLOW,
            ContextCategory::Free => ANSI_BOLD_GREEN,
        }
    }

    /// One-line description of what this category encompasses.
    pub fn description(&self) -> &'static str {
        match self {
            ContextCategory::System => "System instructions, runtime directives, & environment metadata",
            ContextCategory::History => "Conversation turns, user queries, assistant outputs, & tool results",
            ContextCategory::Tools => "Registered tool specifications, JSON schemas, & MCP functions",
            ContextCategory::Free => "Available token capacity before reaching model context limits",
        }
    }
}

// ---------------------------------------------------------------------------
// 4. System Prompt Section Categorization (Base, Tools, Skills, Memory)
// ---------------------------------------------------------------------------

/// Categorization classification for sections within the active system prompt.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum SystemSectionCategory {
    /// Core system instructions, role, persona, directives, and output formatting.
    BaseInstructions,
    /// Tool schemas, parameter definitions, and execution conventions.
    ToolDefinitions,
    /// Installed agent skills, workflow recipes, and domain capabilities.
    Skills,
    /// Workspace context, bookmarks, persistent memories, and project knowledge.
    Memory,
    /// Custom, user-defined, or uncategorized system prompt content.
    Custom,
}

impl SystemSectionCategory {
    /// Ordered list of standard system prompt categories.
    pub const ALL: [SystemSectionCategory; 5] = [
        SystemSectionCategory::BaseInstructions,
        SystemSectionCategory::ToolDefinitions,
        SystemSectionCategory::Skills,
        SystemSectionCategory::Memory,
        SystemSectionCategory::Custom,
    ];

    /// Human-readable category title.
    pub fn name(&self) -> &'static str {
        match self {
            Self::BaseInstructions => "Base Instructions",
            Self::ToolDefinitions => "Tool Definitions",
            Self::Skills => "Skills & Capabilities",
            Self::Memory => "Memory & Workspace",
            Self::Custom => "Custom Sections",
        }
    }

    /// Short badge tag string.
    pub fn badge(&self) -> &'static str {
        match self {
            Self::BaseInstructions => "[BASE]",
            Self::ToolDefinitions => "[TOOLS]",
            Self::Skills => "[SKILLS]",
            Self::Memory => "[MEMORY]",
            Self::Custom => "[CUSTOM]",
        }
    }

    /// Emoji icon.
    pub fn icon(&self) -> &'static str {
        match self {
            Self::BaseInstructions => "📋",
            Self::ToolDefinitions => "🔧",
            Self::Skills => "⚡",
            Self::Memory => "🧠",
            Self::Custom => "📌",
        }
    }

    /// Primary Ratatui color.
    pub fn color(&self) -> Color {
        match self {
            Self::BaseInstructions => Color::Cyan,
            Self::ToolDefinitions => Color::Yellow,
            Self::Skills => Color::Magenta,
            Self::Memory => Color::Green,
            Self::Custom => Color::Blue,
        }
    }

    /// Primary ANSI color escape sequence.
    pub fn ansi_color(&self) -> &'static str {
        match self {
            Self::BaseInstructions => ANSI_BOLD_CYAN,
            Self::ToolDefinitions => ANSI_BOLD_YELLOW,
            Self::Skills => ANSI_BOLD_MAGENTA,
            Self::Memory => ANSI_BOLD_GREEN,
            Self::Custom => ANSI_BOLD_BLUE,
        }
    }

    /// Heuristically classifies a system prompt section title and snippet.
    pub fn classify(title: &str, content: &str) -> Self {
        let t_lower = title.to_lowercase();
        let c_lower = content.to_lowercase();

        // 1. Check for Skills
        if t_lower.contains("skill")
            || t_lower.contains("capability")
            || t_lower.contains("workflow")
            || c_lower.contains("<skills>")
            || c_lower.contains("skill://")
        {
            return Self::Skills;
        }

        // 2. Check for Memory / Workspace
        if t_lower.contains("memory")
            || t_lower.contains("bookmark")
            || t_lower.contains("workspace")
            || t_lower.contains("project context")
            || t_lower.contains("repository")
            || t_lower.contains("working dir")
            || c_lower.contains("<memory>")
            || c_lower.contains("<project_context>")
        {
            return Self::Memory;
        }

        // 3. Check for Tool Definitions
        if t_lower.contains("tool")
            || t_lower.contains("schema")
            || t_lower.contains("function")
            || t_lower.contains("mcp")
            || t_lower.contains("parameter")
            || c_lower.contains("<tools>")
            || c_lower.contains("tool_call")
        {
            return Self::ToolDefinitions;
        }

        // 4. Check for Base Instructions
        if t_lower.contains("instruction")
            || t_lower.contains("directive")
            || t_lower.contains("persona")
            || t_lower.contains("rule")
            || t_lower.contains("guideline")
            || t_lower.contains("role")
            || t_lower.contains("preamble")
            || t_lower.contains("constraint")
            || t_lower.contains("convention")
            || t_lower.contains("format")
        {
            return Self::BaseInstructions;
        }

        // Default to Base Instructions if it looks like general directives, otherwise Custom
        if t_lower.contains("system") || t_lower.is_empty() || t_lower.contains("untitled") {
            Self::BaseInstructions
        } else {
            Self::Custom
        }
    }
}

/// Structured breakdown of active system prompt tokens across the 4 core pillars.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SystemPromptBreakdown {
    /// Total tokens consumed by the active system prompt.
    pub total_tokens: usize,
    /// Tokens consumed by Base Instructions (persona, directives, conventions).
    pub base_instructions_tokens: usize,
    /// Tokens consumed by Tool Definitions (schemas, calling rules).
    pub tool_definitions_tokens: usize,
    /// Tokens consumed by Skills (specialized agents, workflows).
    pub skills_tokens: usize,
    /// Tokens consumed by Memory (workspace context, persistent state).
    pub memory_tokens: usize,
    /// Tokens consumed by custom/other sections.
    pub custom_tokens: usize,
    /// Percentage of system prompt in Base Instructions.
    pub base_instructions_pct: f32,
    /// Percentage of system prompt in Tool Definitions.
    pub tool_definitions_pct: f32,
    /// Percentage of system prompt in Skills.
    pub skills_pct: f32,
    /// Percentage of system prompt in Memory.
    pub memory_pct: f32,
    /// Granular list of extracted sections.
    pub sections: Vec<SystemSectionItem>,
}

impl Default for SystemPromptBreakdown {
    fn default() -> Self {
        Self {
            total_tokens: 0,
            base_instructions_tokens: 0,
            tool_definitions_tokens: 0,
            skills_tokens: 0,
            memory_tokens: 0,
            custom_tokens: 0,
            base_instructions_pct: 0.0,
            tool_definitions_pct: 0.0,
            skills_pct: 0.0,
            memory_pct: 0.0,
            sections: Vec::new(),
        }
    }
}

impl SystemPromptBreakdown {
    /// Builds a `SystemPromptBreakdown` from parsed section items and total token count.
    pub fn from_sections(sections: Vec<SystemSectionItem>, total_tokens: usize) -> Self {
        let mut base_tokens = 0usize;
        let mut tool_tokens = 0usize;
        let mut skill_tokens = 0usize;
        let mut mem_tokens = 0usize;
        let mut custom_tokens = 0usize;

        for sec in &sections {
            match sec.category {
                SystemSectionCategory::BaseInstructions => base_tokens += sec.tokens,
                SystemSectionCategory::ToolDefinitions => tool_tokens += sec.tokens,
                SystemSectionCategory::Skills => skill_tokens += sec.tokens,
                SystemSectionCategory::Memory => mem_tokens += sec.tokens,
                SystemSectionCategory::Custom => custom_tokens += sec.tokens,
            }
        }

        let total_f = total_tokens.max(1) as f32;

        Self {
            total_tokens,
            base_instructions_tokens: base_tokens,
            tool_definitions_tokens: tool_tokens,
            skills_tokens: skill_tokens,
            memory_tokens: mem_tokens,
            custom_tokens,
            base_instructions_pct: (base_tokens as f32 / total_f) * 100.0,
            tool_definitions_pct: (tool_tokens as f32 / total_f) * 100.0,
            skills_pct: (skill_tokens as f32 / total_f) * 100.0,
            memory_pct: (mem_tokens as f32 / total_f) * 100.0,
            sections,
        }
    }
}

// ---------------------------------------------------------------------------
// 5. Category Statistics & Context Items
// ---------------------------------------------------------------------------

/// Granular metrics for a single context category.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ContextCategoryStats {
    /// Category enum.
    pub category: ContextCategory,
    /// Absolute token count consumed by this category.
    pub tokens: usize,
    /// Human-formatted token string (e.g. `"4.2k"`).
    pub formatted_tokens: String,
    /// Percentage of the total context window (0.0% to 100.0%).
    pub pct_of_total: f32,
    /// Percentage of the currently used tokens (0.0% to 100.0%).
    pub pct_of_used: f32,
    /// Number of items (messages, tools, sections) in this category.
    pub item_count: usize,
    /// Pluralized item label (e.g. `"messages"`, `"tools"`, `"sections"`).
    pub item_label: String,
    /// Supplementary detail text.
    pub details: String,
}

/// Information about a single message's footprint in the context window.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MessageContextItem {
    /// Zero-based message index in session.
    pub index: usize,
    /// Conversational turn number (1-based).
    pub turn: usize,
    /// Message role (User, Assistant, System, Tool).
    pub role: Role,
    /// Token count consumed by this message.
    pub tokens: usize,
    /// Formatted token count string (e.g. `"1.2k"`).
    pub formatted_tokens: String,
    /// Percentage of the total conversation history tokens.
    pub pct_of_history: f32,
    /// Percentage of the entire model context window.
    pub pct_of_total: f32,
    /// Raw character count of message content.
    pub char_count: usize,
    /// Short content preview snippet (single line, sanitized).
    pub preview: String,
    /// Whether this message triggered or returned tool calls.
    pub has_tools: bool,
    /// Number of tool calls or tool results attached.
    pub tool_call_count: usize,
    /// Whether this message has a verified exact token count.
    pub is_exact: bool,
}

impl MessageContextItem {
    /// Formats a role badge string (e.g. `[USER]`, `[ASSIST]`, `[TOOL]`).
    pub fn role_badge(&self) -> &'static str {
        match self.role {
            Role::User => "[USER]",
            Role::Assistant => "[ASSIST]",
            Role::System => "[SYSTEM]",
            Role::Tool => "[TOOL]",
        }
    }

    /// Role glyph / emoji.
    pub fn role_icon(&self) -> &'static str {
        match self.role {
            Role::User => "👤",
            Role::Assistant => "🤖",
            Role::System => "⚙️",
            Role::Tool => "🔨",
        }
    }

    /// Ratatui color corresponding to this message role.
    pub fn role_color(&self) -> Color {
        match self.role {
            Role::User => Color::Cyan,
            Role::Assistant => Color::Green,
            Role::System => Color::Blue,
            Role::Tool => Color::Yellow,
        }
    }
}

/// Information about a registered tool's schema footprint.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolContextItem {
    /// Name of the tool (e.g. `"bash"`, `"read"`, `"grep"`).
    pub name: String,
    /// Short description of tool capabilities.
    pub description: String,
    /// Number of parameters declared in JSON schema.
    pub param_count: usize,
    /// Token count consumed by tool specification.
    pub tokens: usize,
    /// Formatted token count string (e.g. `"350"`).
    pub formatted_tokens: String,
    /// Percentage of total tools token budget.
    pub pct_of_tools: f32,
    /// Percentage of model context window.
    pub pct_of_total: f32,
    /// Whether this is an external MCP tool.
    pub is_mcp: bool,
    /// Category / group (e.g. `"Builtin"`, `"Filesystem"`, `"MCP"`).
    pub category: String,
}

/// Information about a section in the system prompt.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SystemSectionItem {
    /// Pillar category (Base Instructions, Tool Definitions, Skills, Memory, Custom).
    pub category: SystemSectionCategory,
    /// Section title or heading name.
    pub title: String,
    /// Number of lines in this section.
    pub line_count: usize,
    /// Token count consumed by this section.
    pub tokens: usize,
    /// Formatted token count string.
    pub formatted_tokens: String,
    /// Percentage of the entire system prompt.
    pub pct_of_system: f32,
    /// Percentage of the model context window.
    pub pct_of_total: f32,
    /// Short snippet preview.
    pub preview: String,
}

/// Prospective compaction simulation metrics.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CompactionPreview {
    /// Current history tokens before compaction.
    pub original_history_tokens: usize,
    /// Projected history tokens after compaction.
    pub compacted_history_tokens: usize,
    /// Number of tokens projected to be saved.
    pub tokens_saved: usize,
    /// Reduction percentage across conversation history (0.0% to 100.0%).
    pub reduction_pct: f32,
    /// Projected total used tokens across all categories post-compaction.
    pub projected_total_tokens: usize,
    /// Projected remaining free tokens post-compaction.
    pub projected_free_tokens: usize,
    /// Projected utilization ratio post-compaction (0.0% to 100.0%).
    pub projected_utilization_pct: f32,
    /// Estimated number of older tool outputs pruned.
    pub pruned_tool_count: usize,
    /// Estimated number of turns consolidated into recap summary.
    pub summarized_turn_count: usize,
}

impl Default for CompactionPreview {
    fn default() -> Self {
        Self {
            original_history_tokens: 0,
            compacted_history_tokens: 0,
            tokens_saved: 0,
            reduction_pct: 0.0,
            projected_total_tokens: 0,
            projected_free_tokens: 0,
            projected_utilization_pct: 0.0,
            pruned_tool_count: 0,
            summarized_turn_count: 0,
        }
    }
}

// ---------------------------------------------------------------------------
// 6. Context Distribution (Core Data Model)
// ---------------------------------------------------------------------------

/// Comprehensive token distribution, capacity, and budget metrics for a session.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ContextDistribution {
    /// Active LLM model identifier (e.g. `"claude-3-5-sonnet"`, `"gpt-4o"`, `"deepseek-chat"`).
    pub model: String,
    /// Maximum context window limit of the model in tokens (e.g. 128,000 or 200,000).
    pub max_context: usize,
    /// Reserved completion buffer (typically 4,096 tokens).
    pub reserved_completion: usize,
    /// Safety margin buffer (typically 1,024 tokens).
    pub safety_margin: usize,
    /// Effective available prompt budget (`max_context - reserved_completion - safety_margin`).
    pub effective_budget: usize,

    // Token counts (Instantaneous context load)
    /// Tokens consumed by system instructions and persona.
    pub system_tokens: usize,
    /// Tokens consumed by conversation message history.
    pub history_tokens: usize,
    /// Tokens consumed by registered tool definitions.
    pub tools_tokens: usize,
    /// Sum of tokens currently in use (`system + history + tools`).
    pub used_tokens: usize,
    /// Remaining unallocated token capacity before reaching max context limit.
    pub free_tokens: usize,

    // Cumulative Session Metrics (Prompt, Completion, Cache Usage)
    /// Total cumulative prompt tokens sent to provider across all turns.
    pub prompt_tokens: u64,
    /// Total cumulative completion tokens generated by model across all turns.
    pub completion_tokens: u64,
    /// Total cumulative session tokens (`prompt_tokens + completion_tokens`).
    pub session_total_tokens: u64,
    /// Tokens read from provider context cache (prompt caching).
    pub cache_read_tokens: u64,
    /// Tokens written to provider context cache.
    pub cache_write_tokens: u64,
    /// Completed conversation request turns in this session.
    pub total_turns: u64,

    // Proportions (Percentages of total context window)
    /// System prompt share of context window (0.0% to 100.0%).
    pub system_pct: f32,
    /// Conversation history share of context window (0.0% to 100.0%).
    pub history_pct: f32,
    /// Tool definitions share of context window (0.0% to 100.0%).
    pub tools_pct: f32,
    /// Unallocated free capacity share of context window (0.0% to 100.0%).
    pub free_pct: f32,
    /// Overall context utilization ratio (0.0% to 100.0+%).
    pub utilization_pct: f32,

    // Classification & Color Alert
    /// Standard alert level classification.
    pub alert_level: ContextAlertLevel,
    /// High-precision utilization alert level (<60% Green, 60-85% Yellow, >85% Red).
    pub utilization_alert: UtilizationAlertLevel,

    // Category Stats
    pub system_stats: ContextCategoryStats,
    pub history_stats: ContextCategoryStats,
    pub tools_stats: ContextCategoryStats,
    pub free_stats: ContextCategoryStats,

    // Active System Prompt Breakdown (Base, Tools, Skills, Memory)
    pub system_breakdown: SystemPromptBreakdown,
    pub system_sections: Vec<SystemSectionItem>,

    // Detailed Item Breakdowns
    pub messages: Vec<MessageContextItem>,
    pub tools: Vec<ToolContextItem>,

    // Compaction Simulation
    pub compaction_preview: CompactionPreview,
}

impl ContextDistribution {
    /// Computes full context distribution from raw parts (model, system prompt, messages, tools).
    pub fn from_parts(
        model: impl Into<String>,
        system_prompt: Option<&str>,
        messages: &[Message],
        tools: &[ToolDefinition],
    ) -> Self {
        let model_str = model.into();
        let max_context = model_context_limit(&model_str);
        let reserved_completion = DEFAULT_RESERVED_COMPLETION;
        let safety_margin = DEFAULT_SAFETY_MARGIN;
        let effective_budget = max_context.saturating_sub(reserved_completion + safety_margin);

        // 1. Calculate System tokens and breakdown
        let system_text = system_prompt.unwrap_or("").trim();
        let system_tokens = if system_text.is_empty() {
            0
        } else {
            // Include message framing overhead (4 tokens)
            4 + estimate_text_tokens(system_text)
        };
        let system_sections = Self::extract_system_sections(system_text, max_context);
        let system_breakdown = SystemPromptBreakdown::from_sections(system_sections.clone(), system_tokens);

        // 2. Calculate History tokens and message items
        let history_tokens = estimate_messages_tokens(messages);
        let message_items = Self::extract_message_items(messages, history_tokens, max_context);

        // 3. Calculate Tool tokens and tool items
        let tools_tokens = estimate_tools_tokens(tools);
        let tool_items = Self::extract_tool_items(tools, tools_tokens, max_context);

        // 4. Totals and Free budget
        let used_tokens = system_tokens + history_tokens + tools_tokens;
        let free_tokens = max_context.saturating_sub(used_tokens);

        let max_f = max_context.max(1) as f32;
        let used_f = used_tokens.max(1) as f32;

        let system_pct = (system_tokens as f32 / max_f) * 100.0;
        let history_pct = (history_tokens as f32 / max_f) * 100.0;
        let tools_pct = (tools_tokens as f32 / max_f) * 100.0;
        let free_pct = (free_tokens as f32 / max_f) * 100.0;
        let utilization_pct = (used_tokens as f32 / max_f) * 100.0;

        let alert_level = ContextAlertLevel::from_utilization(used_tokens as f32 / max_f);
        let utilization_alert = UtilizationAlertLevel::from_percentage(utilization_pct);

        // 5. Category Stats
        let system_stats = ContextCategoryStats {
            category: ContextCategory::System,
            tokens: system_tokens,
            formatted_tokens: format_token_count(system_tokens),
            pct_of_total: system_pct,
            pct_of_used: (system_tokens as f32 / used_f) * 100.0,
            item_count: system_sections.len().max(if system_tokens > 0 { 1 } else { 0 }),
            item_label: "sections".to_string(),
            details: if system_tokens == 0 {
                "No system prompt specified".to_string()
            } else {
                format!(
                    "{} sections (Base: {}, Tools: {}, Skills: {}, Mem: {})",
                    system_sections.len(),
                    format_token_count(system_breakdown.base_instructions_tokens),
                    format_token_count(system_breakdown.tool_definitions_tokens),
                    format_token_count(system_breakdown.skills_tokens),
                    format_token_count(system_breakdown.memory_tokens)
                )
            },
        };

        let history_stats = ContextCategoryStats {
            category: ContextCategory::History,
            tokens: history_tokens,
            formatted_tokens: format_token_count(history_tokens),
            pct_of_total: history_pct,
            pct_of_used: (history_tokens as f32 / used_f) * 100.0,
            item_count: messages.len(),
            item_label: "messages".to_string(),
            details: format!("{} messages across conversation turns", messages.len()),
        };

        let tools_stats = ContextCategoryStats {
            category: ContextCategory::Tools,
            tokens: tools_tokens,
            formatted_tokens: format_token_count(tools_tokens),
            pct_of_total: tools_pct,
            pct_of_used: (tools_tokens as f32 / used_f) * 100.0,
            item_count: tools.len(),
            item_label: "tools".to_string(),
            details: format!("{} registered tool definitions & schemas", tools.len()),
        };

        let free_stats = ContextCategoryStats {
            category: ContextCategory::Free,
            tokens: free_tokens,
            formatted_tokens: format_token_count(free_tokens),
            pct_of_total: free_pct,
            pct_of_used: 0.0,
            item_count: 1,
            item_label: "headroom".to_string(),
            details: if free_tokens > 0 {
                format!("{} available headroom before model limit", format_token_count(free_tokens))
            } else {
                "Context window completely exhausted / overflowing".to_string()
            },
        };

        // 6. Compaction simulation
        let compaction_preview = Self::simulate_compaction(
            messages,
            &model_str,
            max_context,
            system_tokens,
            tools_tokens,
        );

        Self {
            model: model_str,
            max_context,
            reserved_completion,
            safety_margin,
            effective_budget,
            system_tokens,
            history_tokens,
            tools_tokens,
            used_tokens,
            free_tokens,
            prompt_tokens: 0,
            completion_tokens: 0,
            session_total_tokens: 0,
            cache_read_tokens: 0,
            cache_write_tokens: 0,
            total_turns: 0,
            system_pct,
            history_pct,
            tools_pct,
            free_pct,
            utilization_pct,
            alert_level,
            utilization_alert,
            system_stats,
            history_stats,
            tools_stats,
            free_stats,
            system_breakdown,
            system_sections,
            messages: message_items,
            tools: tool_items,
            compaction_preview,
        }
    }

    /// Computes context distribution directly from an active `Session` and registered tools.
    pub fn from_session(session: &Session, tools: &[ToolDefinition]) -> Self {
        let mut dist = Self::from_parts(
            session.active_model(),
            session.system_prompt(),
            session.messages(),
            tools,
        );

        dist.prompt_tokens = session.token_stats.prompt_tokens;
        dist.completion_tokens = session.token_stats.completion_tokens;
        dist.session_total_tokens = session.token_stats.total_tokens;
        dist.cache_read_tokens = session.token_stats.cache_read_tokens;
        dist.cache_write_tokens = session.token_stats.cache_write_tokens;
        dist.total_turns = session.token_stats.total_turns;
        dist
    }

    /// Builder helper to inject custom token stats.
    pub fn with_token_stats(mut self, stats: &TokenStats) -> Self {
        self.prompt_tokens = stats.prompt_tokens;
        self.completion_tokens = stats.completion_tokens;
        self.session_total_tokens = stats.total_tokens;
        self.cache_read_tokens = stats.cache_read_tokens;
        self.cache_write_tokens = stats.cache_write_tokens;
        self.total_turns = stats.total_turns;
        self
    }

    /// Formats total context window capacity (e.g. `"128.0k tokens"` or `"200.0k tokens"`).
    pub fn format_capacity(&self) -> String {
        format!("{} tokens", format_token_count(self.max_context))
    }

    /// Formats prompt tokens count.
    pub fn format_prompt_tokens(&self) -> String {
        format_token_count(self.prompt_tokens as usize)
    }

    /// Formats completion tokens count.
    pub fn format_completion_tokens(&self) -> String {
        format_token_count(self.completion_tokens as usize)
    }

    /// Formats total cumulative session tokens.
    pub fn format_session_tokens(&self) -> String {
        format_token_count(self.session_total_tokens as usize)
    }

    /// Formats cache read tokens count.
    pub fn format_cache_read_tokens(&self) -> String {
        format_token_count(self.cache_read_tokens as usize)
    }

    /// Formats cache write tokens count.
    pub fn format_cache_write_tokens(&self) -> String {
        format_token_count(self.cache_write_tokens as usize)
    }

    /// Calculates cache hit rate percentage (0.0% to 100.0%).
    pub fn cache_hit_rate(&self) -> f32 {
        let total_prompt = self.prompt_tokens.saturating_add(self.cache_read_tokens);
        if total_prompt > 0 {
            (self.cache_read_tokens as f32 / total_prompt as f32) * 100.0
        } else {
            0.0
        }
    }

    /// Formatted cache metrics summary.
    pub fn format_cache_usage(&self) -> String {
        if self.cache_read_tokens > 0 || self.cache_write_tokens > 0 {
            format!(
                "Cache: {} read, {} write (Hit Rate: {:.1}%)",
                self.format_cache_read_tokens(),
                self.format_cache_write_tokens(),
                self.cache_hit_rate()
            )
        } else {
            "Cache: 0 read / 0 written".to_string()
        }
    }

    /// Primary Ratatui color for utilization alerts (Green <60%, Yellow 60-85%, Red >85%).
    pub fn utilization_color(&self) -> Color {
        self.utilization_alert.color()
    }

    /// Primary ANSI color code for utilization alerts.
    pub fn utilization_ansi_color(&self) -> &'static str {
        self.utilization_alert.ansi_color()
    }

    /// Header badge and color for context utilization.
    pub fn utilization_badge(&self) -> (&'static str, Color) {
        (self.utilization_alert.badge(), self.utilization_alert.color())
    }

    /// Helper to extract structured section items and classify them across Base, Tools, Skills, Memory.
    fn extract_system_sections(text: &str, max_context: usize) -> Vec<SystemSectionItem> {
        if text.is_empty() {
            return Vec::new();
        }

        let mut sections = Vec::new();
        let lines: Vec<&str> = text.lines().collect();
        let total_system_tokens = 4 + estimate_text_tokens(text);
        let max_f = max_context.max(1) as f32;
        let sys_f = total_system_tokens.max(1) as f32;

        let mut current_title = "Preamble / System Directives".to_string();
        let mut current_lines: Vec<&str> = Vec::new();

        for line in lines {
            let trimmed = line.trim();
            if trimmed.starts_with('#') {
                // Flush previous section
                if !current_lines.is_empty() {
                    let section_text = current_lines.join("\n");
                    let tokens = estimate_text_tokens(&section_text);
                    let preview = current_lines
                        .iter()
                        .find(|l| !l.trim().is_empty())
                        .map(|s| s.trim().to_string())
                        .unwrap_or_else(|| "Empty section".to_string());

                    let category = SystemSectionCategory::classify(&current_title, &section_text);

                    sections.push(SystemSectionItem {
                        category,
                        title: current_title,
                        line_count: current_lines.len(),
                        tokens,
                        formatted_tokens: format_token_count(tokens),
                        pct_of_system: (tokens as f32 / sys_f) * 100.0,
                        pct_of_total: (tokens as f32 / max_f) * 100.0,
                        preview,
                    });
                    current_lines.clear();
                }

                current_title = trimmed.trim_start_matches('#').trim().to_string();
                if current_title.is_empty() {
                    current_title = "Untitled Section".to_string();
                }
            } else {
                current_lines.push(line);
            }
        }

        // Flush trailing section
        if !current_lines.is_empty() {
            let section_text = current_lines.join("\n");
            let tokens = estimate_text_tokens(&section_text);
            let preview = current_lines
                .iter()
                .find(|l| !l.trim().is_empty())
                .map(|s| s.trim().to_string())
                .unwrap_or_else(|| "Empty section".to_string());

            let category = SystemSectionCategory::classify(&current_title, &section_text);

            sections.push(SystemSectionItem {
                category,
                title: current_title,
                line_count: current_lines.len(),
                tokens,
                formatted_tokens: format_token_count(tokens),
                pct_of_system: (tokens as f32 / sys_f) * 100.0,
                pct_of_total: (tokens as f32 / max_f) * 100.0,
                preview,
            });
        }

        if sections.is_empty() {
            let category = SystemSectionCategory::classify("System Instructions", text);
            sections.push(SystemSectionItem {
                category,
                title: "System Instructions".to_string(),
                line_count: 1,
                tokens: total_system_tokens,
                formatted_tokens: format_token_count(total_system_tokens),
                pct_of_system: 100.0,
                pct_of_total: (total_system_tokens as f32 / max_f) * 100.0,
                preview: text.chars().take(80).collect(),
            });
        }

        sections
    }

    /// Helper to convert conversation messages into structured `MessageContextItem`s.
    fn extract_message_items(
        messages: &[Message],
        history_tokens: usize,
        max_context: usize,
    ) -> Vec<MessageContextItem> {
        let mut items = Vec::with_capacity(messages.len());
        let max_f = max_context.max(1) as f32;
        let hist_f = history_tokens.max(1) as f32;

        let mut turn_counter = 1usize;

        for (idx, msg) in messages.iter().enumerate() {
            let tokens = crate::agent::tokens::estimate_message_tokens(msg);
            let char_count = msg.content.chars().count();

            // Preview sanitize
            let clean_preview = msg
                .content
                .lines()
                .find(|l| !l.trim().is_empty())
                .map(|l| l.trim())
                .unwrap_or(if msg.tool_calls.is_some() {
                    "[Tool Call Invocation]"
                } else if msg.tool_call_id.is_some() {
                    "[Tool Result]"
                } else {
                    "[Empty Message]"
                });

            let preview_snippet: String = clean_preview.chars().take(120).collect();

            let tool_calls_count = msg.tool_calls.as_ref().map(|tc| tc.len()).unwrap_or(0);
            let has_tools = tool_calls_count > 0 || msg.tool_call_id.is_some();

            if msg.role == Role::User && idx > 0 {
                turn_counter += 1;
            }

            items.push(MessageContextItem {
                index: idx,
                turn: turn_counter,
                role: msg.role,
                tokens,
                formatted_tokens: format_token_count(tokens),
                pct_of_history: (tokens as f32 / hist_f) * 100.0,
                pct_of_total: (tokens as f32 / max_f) * 100.0,
                char_count,
                preview: preview_snippet,
                has_tools,
                tool_call_count: tool_calls_count,
                is_exact: false,
            });
        }

        items
    }

    /// Helper to extract structured tool items from registered `ToolDefinition`s.
    fn extract_tool_items(
        tools: &[ToolDefinition],
        tools_tokens: usize,
        max_context: usize,
    ) -> Vec<ToolContextItem> {
        let mut items = Vec::with_capacity(tools.len());
        let max_f = max_context.max(1) as f32;
        let tools_f = tools_tokens.max(1) as f32;

        for tool in tools {
            let tokens = estimate_tool_definition_tokens(tool);
            let param_count = tool
                .parameters
                .get("properties")
                .and_then(|p| p.as_object())
                .map(|o| o.len())
                .unwrap_or(0);

            let is_mcp = tool.name.starts_with("mcp__") || tool.name.contains("__");
            let category = if is_mcp {
                "MCP Server".to_string()
            } else {
                match tool.name.as_str() {
                    "bash" | "process" => "System / Execution",
                    "read" | "write" | "edit" | "patch" => "Filesystem / Editing",
                    "grep" | "glob" | "search" | "symbols" => "Codebase Search",
                    "fetch" | "web_search" => "Network / Web",
                    "sqlite" => "Database",
                    _ => "Core Tools",
                }
                .to_string()
            };

            items.push(ToolContextItem {
                name: tool.name.clone(),
                description: tool.description.clone(),
                param_count,
                tokens,
                formatted_tokens: format_token_count(tokens),
                pct_of_tools: (tokens as f32 / tools_f) * 100.0,
                pct_of_total: (tokens as f32 / max_f) * 100.0,
                is_mcp,
                category,
            });
        }

        // Sort largest consumers first
        items.sort_by(|a, b| b.tokens.cmp(&a.tokens));
        items
    }

    /// Runs heuristic compaction simulation.
    fn simulate_compaction(
        messages: &[Message],
        model: &str,
        max_context: usize,
        system_tokens: usize,
        tools_tokens: usize,
    ) -> CompactionPreview {
        let orig_history_tokens = estimate_messages_tokens(messages);
        if messages.is_empty() {
            return CompactionPreview::default();
        }

        let compactor = Compactor::new(max_context);
        let (compacted_messages, result) = compactor.compact(messages, model);

        let compacted_history_tokens = if result.compacted {
            estimate_messages_tokens(&compacted_messages)
        } else {
            // Heuristic estimation if compactor didn't trigger
            (orig_history_tokens as f32 * 0.45) as usize
        };

        let tokens_saved = orig_history_tokens.saturating_sub(compacted_history_tokens);
        let reduction_pct = if orig_history_tokens > 0 {
            (tokens_saved as f32 / orig_history_tokens as f32) * 100.0
        } else {
            0.0
        };

        let projected_total = system_tokens + compacted_history_tokens + tools_tokens;
        let projected_free = max_context.saturating_sub(projected_total);
        let projected_util = (projected_total as f32 / max_context.max(1) as f32) * 100.0;

        let tool_msgs_count = messages.iter().filter(|m| m.role == Role::Tool).count();
        let pruned_tool_count = if result.compacted {
            result.messages_removed.min(tool_msgs_count)
        } else {
            tool_msgs_count
        };
        let summarized_turns = if result.compacted {
            result.messages_removed.saturating_sub(pruned_tool_count) / 2
        } else {
            messages.len().saturating_sub(4) / 2
        };

        CompactionPreview {
            original_history_tokens: orig_history_tokens,
            compacted_history_tokens,
            tokens_saved,
            reduction_pct,
            projected_total_tokens: projected_total,
            projected_free_tokens: projected_free,
            projected_utilization_pct: projected_util,
            pruned_tool_count: if pruned_tool_count > 0 {
                pruned_tool_count
            } else {
                messages.iter().filter(|m| m.role == Role::Tool).count()
            },
            summarized_turn_count: if summarized_turns > 0 {
                summarized_turns
            } else {
                messages.len().saturating_sub(4) / 2
            },
        }
    }

    /// Access category statistics.
    pub fn get_category_stats(&self, category: ContextCategory) -> &ContextCategoryStats {
        match category {
            ContextCategory::System => &self.system_stats,
            ContextCategory::History => &self.history_stats,
            ContextCategory::Tools => &self.tools_stats,
            ContextCategory::Free => &self.free_stats,
        }
    }

    /// Returns `true` if current utilization is at Warning level or higher.
    pub fn is_alert(&self) -> bool {
        self.utilization_alert.is_alert()
    }

    /// Returns top N largest messages by token count.
    pub fn top_messages_by_size(&self, limit: usize) -> Vec<&MessageContextItem> {
        let mut sorted: Vec<&MessageContextItem> = self.messages.iter().collect();
        sorted.sort_by(|a, b| b.tokens.cmp(&a.tokens));
        sorted.truncate(limit);
        sorted
    }

    /// Formatted single-line text summary.
    pub fn formatted_summary(&self) -> String {
        format!(
            "Capacity: {} • Used: {} ({:.1}%) • Sys: {} (Base: {}, Tools: {}, Skills: {}, Mem: {}) • Hist: {} • Tools: {} • Free: {} • Session: {} prompt, {} comp, {} cache read",
            self.format_capacity(),
            format_token_count(self.used_tokens),
            self.utilization_pct,
            format_token_count(self.system_tokens),
            format_token_count(self.system_breakdown.base_instructions_tokens),
            format_token_count(self.system_breakdown.tool_definitions_tokens),
            format_token_count(self.system_breakdown.skills_tokens),
            format_token_count(self.system_breakdown.memory_tokens),
            format_token_count(self.history_tokens),
            format_token_count(self.tools_tokens),
            format_token_count(self.free_tokens),
            self.format_prompt_tokens(),
            self.format_completion_tokens(),
            self.format_cache_read_tokens(),
        )
    }
}

// ---------------------------------------------------------------------------
// 7. Interactive UI State & Tabs
// ---------------------------------------------------------------------------

/// Categorized tab views for the Context Inspector dialog.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
pub enum ContextInspectorTab {
    /// Visual stacked bar, metric cards, token budget, cache stats, and health analytics.
    #[default]
    Overview,
    /// 4-pillar system prompt breakdown (Base, Tools, Skills, Memory) and section details.
    System,
    /// Interactive scrollable message history table.
    History,
    /// Registered tool definitions and schema weights.
    Tools,
    /// Compaction simulation and prospective savings preview.
    Compaction,
}

impl ContextInspectorTab {
    /// Ordered list of all tabs.
    pub const ALL: [ContextInspectorTab; 5] = [
        ContextInspectorTab::Overview,
        ContextInspectorTab::System,
        ContextInspectorTab::History,
        ContextInspectorTab::Tools,
        ContextInspectorTab::Compaction,
    ];

    pub fn index(&self) -> usize {
        match self {
            Self::Overview => 0,
            Self::System => 1,
            Self::History => 2,
            Self::Tools => 3,
            Self::Compaction => 4,
        }
    }

    pub fn name(&self) -> &'static str {
        match self {
            Self::Overview => "Overview & Budget",
            Self::System => "System Breakdown",
            Self::History => "History",
            Self::Tools => "Tools",
            Self::Compaction => "Compaction Preview",
        }
    }

    pub fn short_name(&self) -> &'static str {
        match self {
            Self::Overview => "Overv",
            Self::System => "Sys",
            Self::History => "Hist",
            Self::Tools => "Tools",
            Self::Compaction => "Comp",
        }
    }

    pub fn icon(&self) -> &'static str {
        match self {
            Self::Overview => "📊",
            Self::System => "🖥️",
            Self::History => "💬",
            Self::Tools => "🔧",
            Self::Compaction => "⚡",
        }
    }

    pub fn next(&self) -> Self {
        match self {
            Self::Overview => Self::System,
            Self::System => Self::History,
            Self::History => Self::Tools,
            Self::Tools => Self::Compaction,
            Self::Compaction => Self::Overview,
        }
    }

    pub fn prev(&self) -> Self {
        match self {
            Self::Overview => Self::Compaction,
            Self::System => Self::Overview,
            Self::History => Self::System,
            Self::Tools => Self::History,
            Self::Compaction => Self::Tools,
        }
    }
}

/// Action result returned when the interactive context inspector exits.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ContextInspectorResult {
    /// User closed inspector without taking action (`Esc` / `q`).
    Closed,
    /// User requested immediate compaction (`c` or Enter on Compaction tab).
    TriggerCompaction,
    /// User selected a message to inspect or copy (`Enter`).
    SelectMessage(usize),
    /// User selected a tool to inspect (`Enter`).
    SelectTool(String),
}

/// Stateful controller for interactive context inspector navigation.
#[derive(Debug, Clone)]
pub struct ContextInspectorState {
    /// Active tab in the inspector dialog.
    pub active_tab: ContextInspectorTab,
    /// Selected index within the active tab's list.
    pub selected_index: usize,
    /// Scroll offset for vertical list scrolling.
    pub scroll_offset: usize,
    /// Whether the detailed item inspection popup/drawer is toggled on.
    pub detail_expanded: bool,
    /// Whether the help keybindings overlay is currently visible.
    pub show_help: bool,
    /// Live search query string when filtering messages or tools.
    pub filter_query: String,
    /// Active visual theme.
    pub theme: Theme,
    /// Whether outer rounded border is rendered.
    pub show_border: bool,
}

impl Default for ContextInspectorState {
    fn default() -> Self {
        Self::new()
    }
}

impl ContextInspectorState {
    /// Creates a new default `ContextInspectorState`.
    pub fn new() -> Self {
        Self {
            active_tab: ContextInspectorTab::Overview,
            selected_index: 0,
            scroll_offset: 0,
            detail_expanded: false,
            show_help: false,
            filter_query: String::new(),
            theme: Theme::auto(),
            show_border: true,
        }
    }

    /// Sets custom theme.
    pub fn with_theme(mut self, theme: Theme) -> Self {
        self.theme = theme;
        self
    }

    /// Sets starting tab.
    pub fn with_tab(mut self, tab: ContextInspectorTab) -> Self {
        self.active_tab = tab;
        self
    }

    /// Switch to next tab (`Tab` / `l`).
    pub fn next_tab(&mut self) {
        self.active_tab = self.active_tab.next();
        self.selected_index = 0;
        self.scroll_offset = 0;
        self.detail_expanded = false;
    }

    /// Switch to previous tab (`Shift+Tab` / `h`).
    pub fn prev_tab(&mut self) {
        self.active_tab = self.active_tab.prev();
        self.selected_index = 0;
        self.scroll_offset = 0;
        self.detail_expanded = false;
    }

    /// Switch directly to a specific tab (`1`..`5`).
    pub fn set_tab(&mut self, tab: ContextInspectorTab) {
        if self.active_tab != tab {
            self.active_tab = tab;
            self.selected_index = 0;
            self.scroll_offset = 0;
            self.detail_expanded = false;
        }
    }

    /// Move selection down (`↓` / `j`).
    pub fn select_next(&mut self, item_count: usize) {
        if item_count == 0 {
            self.selected_index = 0;
            return;
        }
        if self.selected_index + 1 < item_count {
            self.selected_index += 1;
        } else {
            self.selected_index = 0; // Wrap
        }
    }

    /// Move selection up (`↑` / `k`).
    pub fn select_prev(&mut self, item_count: usize) {
        if item_count == 0 {
            self.selected_index = 0;
            return;
        }
        if self.selected_index > 0 {
            self.selected_index -= 1;
        } else {
            self.selected_index = item_count - 1; // Wrap
        }
    }

    /// Move selection page down.
    pub fn select_page_down(&mut self, item_count: usize, page_size: usize) {
        if item_count == 0 {
            return;
        }
        self.selected_index = (self.selected_index + page_size).min(item_count - 1);
    }

    /// Move selection page up.
    pub fn select_page_up(&mut self, page_size: usize) {
        self.selected_index = self.selected_index.saturating_sub(page_size);
    }

    /// Jump to first item.
    pub fn select_first(&mut self) {
        self.selected_index = 0;
    }

    /// Jump to last item.
    pub fn select_last(&mut self, item_count: usize) {
        if item_count > 0 {
            self.selected_index = item_count - 1;
        }
    }

    /// Toggle detailed item view popup.
    pub fn toggle_detail(&mut self) {
        self.detail_expanded = !self.detail_expanded;
    }

    /// Toggle help modal.
    pub fn toggle_help(&mut self) {
        self.show_help = !self.show_help;
    }

    /// Returns current item count based on active tab.
    pub fn active_item_count(&self, dist: &ContextDistribution) -> usize {
        match self.active_tab {
            ContextInspectorTab::Overview => 4, // 4 categories
            ContextInspectorTab::System => dist.system_sections.len(),
            ContextInspectorTab::History => dist.messages.len(),
            ContextInspectorTab::Tools => dist.tools.len(),
            ContextInspectorTab::Compaction => 1,
        }
    }

    /// Handles keyboard events, returning `Some(ContextInspectorResult)` on terminal actions.
    pub fn handle_key(
        &mut self,
        code: KeyCode,
        modifiers: KeyModifiers,
        dist: &ContextDistribution,
    ) -> Option<ContextInspectorResult> {
        // 1. Help Modal toggle or dismissal
        if self.show_help {
            if matches!(code, KeyCode::Char('?') | KeyCode::Char('h') | KeyCode::Esc | KeyCode::Enter) {
                self.show_help = false;
            }
            return None;
        }

        let item_count = self.active_item_count(dist);

        match (code, modifiers) {
            // Exit / Close
            (KeyCode::Esc, _) | (KeyCode::Char('q'), KeyModifiers::NONE) => {
                if self.detail_expanded {
                    self.detail_expanded = false;
                    None
                } else if !self.filter_query.is_empty() {
                    self.filter_query.clear();
                    None
                } else {
                    Some(ContextInspectorResult::Closed)
                }
            }
            (KeyCode::Char('c'), KeyModifiers::CONTROL) => Some(ContextInspectorResult::Closed),

            // Help
            (KeyCode::Char('?'), _) | (KeyCode::Char('h'), KeyModifiers::NONE)
                if self.filter_query.is_empty() =>
            {
                self.toggle_help();
                None
            }

            // Tab Switching: Tab / Shift+Tab
            (KeyCode::Tab, KeyModifiers::NONE) => {
                self.next_tab();
                None
            }
            (KeyCode::BackTab, _) | (KeyCode::Tab, KeyModifiers::SHIFT) => {
                self.prev_tab();
                None
            }

            // Numeric Tab Switching ('1' .. '5')
            (KeyCode::Char('1'), KeyModifiers::NONE) if self.filter_query.is_empty() => {
                self.set_tab(ContextInspectorTab::Overview);
                None
            }
            (KeyCode::Char('2'), KeyModifiers::NONE) if self.filter_query.is_empty() => {
                self.set_tab(ContextInspectorTab::System);
                None
            }
            (KeyCode::Char('3'), KeyModifiers::NONE) if self.filter_query.is_empty() => {
                self.set_tab(ContextInspectorTab::History);
                None
            }
            (KeyCode::Char('4'), KeyModifiers::NONE) if self.filter_query.is_empty() => {
                self.set_tab(ContextInspectorTab::Tools);
                None
            }
            (KeyCode::Char('5'), KeyModifiers::NONE) if self.filter_query.is_empty() => {
                self.set_tab(ContextInspectorTab::Compaction);
                None
            }

            // Vertical Navigation: Down
            (KeyCode::Down, _) | (KeyCode::Char('j'), KeyModifiers::NONE)
                if self.filter_query.is_empty() =>
            {
                self.select_next(item_count);
                None
            }

            // Vertical Navigation: Up
            (KeyCode::Up, _) | (KeyCode::Char('k'), KeyModifiers::NONE)
                if self.filter_query.is_empty() =>
            {
                self.select_prev(item_count);
                None
            }

            // Page Navigation
            (KeyCode::PageDown, _) | (KeyCode::Char('d'), KeyModifiers::CONTROL) => {
                self.select_page_down(item_count, 6);
                None
            }
            (KeyCode::PageUp, _) | (KeyCode::Char('u'), KeyModifiers::CONTROL) => {
                self.select_page_up(6);
                None
            }

            // Home / End
            (KeyCode::Home, _) | (KeyCode::Char('g'), KeyModifiers::NONE)
                if self.filter_query.is_empty() =>
            {
                self.select_first();
                None
            }
            (KeyCode::End, _) | (KeyCode::Char('G'), KeyModifiers::SHIFT) => {
                self.select_last(item_count);
                None
            }

            // Space / Enter: Inspect / Action
            (KeyCode::Enter, KeyModifiers::NONE) | (KeyCode::Char(' '), KeyModifiers::NONE)
                if self.filter_query.is_empty() =>
            {
                match self.active_tab {
                    ContextInspectorTab::Overview => {
                        // Switch to selected category tab
                        match self.selected_index {
                            0 => self.set_tab(ContextInspectorTab::System),
                            1 => self.set_tab(ContextInspectorTab::History),
                            2 => self.set_tab(ContextInspectorTab::Tools),
                            3 => self.set_tab(ContextInspectorTab::Compaction),
                            _ => {}
                        }
                        None
                    }
                    ContextInspectorTab::System => {
                        self.toggle_detail();
                        None
                    }
                    ContextInspectorTab::History => {
                        if self.detail_expanded {
                            self.toggle_detail();
                            None
                        } else if let Some(msg) = dist.messages.get(self.selected_index) {
                            Some(ContextInspectorResult::SelectMessage(msg.index))
                        } else {
                            None
                        }
                    }
                    ContextInspectorTab::Tools => {
                        if let Some(tool) = dist.tools.get(self.selected_index) {
                            Some(ContextInspectorResult::SelectTool(tool.name.clone()))
                        } else {
                            None
                        }
                    }
                    ContextInspectorTab::Compaction => {
                        Some(ContextInspectorResult::TriggerCompaction)
                    }
                }
            }

            // 'c' to trigger or jump to compaction
            (KeyCode::Char('c'), KeyModifiers::NONE) if self.filter_query.is_empty() => {
                if self.active_tab == ContextInspectorTab::Compaction {
                    Some(ContextInspectorResult::TriggerCompaction)
                } else {
                    self.set_tab(ContextInspectorTab::Compaction);
                    None
                }
            }

            // 'd' to toggle detail pane
            (KeyCode::Char('d'), KeyModifiers::NONE) if self.filter_query.is_empty() => {
                self.toggle_detail();
                None
            }

            // Backspace for search query
            (KeyCode::Backspace, _) => {
                if !self.filter_query.is_empty() {
                    self.filter_query.pop();
                    self.selected_index = 0;
                }
                None
            }

            // Search query typing
            (KeyCode::Char(c), KeyModifiers::NONE) | (KeyCode::Char(c), KeyModifiers::SHIFT) => {
                self.filter_query.push(c);
                self.selected_index = 0;
                None
            }

            _ => None,
        }
    }
}

// ---------------------------------------------------------------------------
// 8. Context Bar Widget (Lightweight Standalone Segment Bar)
// ---------------------------------------------------------------------------

/// Visual configuration options for `ContextBarWidget`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ContextBarOptions {
    /// Whether to show text labels above or inside the bar.
    pub show_labels: bool,
    /// Whether to show percentage indicators.
    pub show_percentages: bool,
    /// Whether to render a horizontal legend row beneath the bar.
    pub show_legend: bool,
    /// Compact mode for 1-line status bars.
    pub compact: bool,
}

impl Default for ContextBarOptions {
    fn default() -> Self {
        Self {
            show_labels: true,
            show_percentages: true,
            show_legend: true,
            compact: false,
        }
    }
}

/// Standalone Ratatui widget that renders a multi-colored stacked horizontal bar
/// representing token distribution across System, History, Tools, and Free budget.
pub struct ContextBarWidget<'a> {
    distribution: &'a ContextDistribution,
    options: ContextBarOptions,
    theme: &'a Theme,
}

impl<'a> ContextBarWidget<'a> {
    /// Creates a new `ContextBarWidget`.
    pub fn new(distribution: &'a ContextDistribution, theme: &'a Theme) -> Self {
        Self {
            distribution,
            options: ContextBarOptions::default(),
            theme,
        }
    }

    /// Sets custom display options.
    pub fn with_options(mut self, options: ContextBarOptions) -> Self {
        self.options = options;
        self
    }
}

impl<'a> Widget for ContextBarWidget<'a> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        if area.width < 8 || area.height < 1 {
            return;
        }

        let d = self.distribution;
        let total = d.max_context.max(1) as f32;
        let width = area.width as usize;

        // Calculate discrete column widths for each slice
        let sys_w = ((d.system_tokens as f32 / total) * width as f32).round() as usize;
        let hist_w = ((d.history_tokens as f32 / total) * width as f32).round() as usize;
        let tools_w = ((d.tools_tokens as f32 / total) * width as f32).round() as usize;

        // Ensure visible representation (at least 1 char if tokens > 0)
        let safe_sys_w = if d.system_tokens > 0 && sys_w == 0 { 1 } else { sys_w };
        let safe_hist_w = if d.history_tokens > 0 && hist_w == 0 { 1 } else { hist_w };
        let safe_tools_w = if d.tools_tokens > 0 && tools_w == 0 { 1 } else { tools_w };

        let used_w = safe_sys_w + safe_hist_w + safe_tools_w;
        let _safe_free_w = width.saturating_sub(used_w);

        let bar_y = area.y;
        let mut cur_x = area.x;

        // 1. Render System slice (Cyan)
        let sys_end = (cur_x + safe_sys_w as u16).min(area.right());
        for x in cur_x..sys_end {
            buf[(x, bar_y)]
                .set_symbol(BLOCK_FULL)
                .set_fg(ContextCategory::System.ratatui_color(self.theme));
        }
        cur_x = sys_end;

        // 2. Render History slice (Magenta)
        let hist_end = (cur_x + safe_hist_w as u16).min(area.right());
        for x in cur_x..hist_end {
            buf[(x, bar_y)]
                .set_symbol(BLOCK_FULL)
                .set_fg(ContextCategory::History.ratatui_color(self.theme));
        }
        cur_x = hist_end;

        // 3. Render Tools slice (Yellow)
        let tools_end = (cur_x + safe_tools_w as u16).min(area.right());
        for x in cur_x..tools_end {
            buf[(x, bar_y)]
                .set_symbol(BLOCK_FULL)
                .set_fg(ContextCategory::Tools.ratatui_color(self.theme));
        }
        cur_x = tools_end;

        // 4. Render Free budget slice (Green / Gray Shade)
        let free_symbol = if d.free_tokens > 0 { BLOCK_LIGHT_SHADE } else { BLOCK_DARK_SHADE };
        let free_fg = if d.free_tokens > 0 {
            Color::DarkGray
        } else {
            Color::Red
        };

        for x in cur_x..area.right() {
            buf[(x, bar_y)]
                .set_symbol(free_symbol)
                .set_fg(free_fg);
        }

        // Render Legend Row if height >= 2 and option is enabled
        if self.options.show_legend && area.height >= 2 {
            let legend_y = bar_y + 1;
            let legend_spans = vec![
                Span::styled("█ ", Style::default().fg(Color::Cyan)),
                Span::styled(
                    format!("Sys: {} ", format_token_count(d.system_tokens)),
                    Style::default().fg(Color::White).add_modifier(Modifier::BOLD),
                ),
                Span::styled("█ ", Style::default().fg(Color::Magenta)),
                Span::styled(
                    format!("Hist: {} ", format_token_count(d.history_tokens)),
                    Style::default().fg(Color::White).add_modifier(Modifier::BOLD),
                ),
                Span::styled("█ ", Style::default().fg(Color::Yellow)),
                Span::styled(
                    format!("Tools: {} ", format_token_count(d.tools_tokens)),
                    Style::default().fg(Color::White).add_modifier(Modifier::BOLD),
                ),
                Span::styled("░ ", Style::default().fg(Color::Green)),
                Span::styled(
                    format!("Free: {}", format_token_count(d.free_tokens)),
                    Style::default().fg(Color::Green).add_modifier(Modifier::BOLD),
                ),
                Span::raw(" • "),
                Span::styled(
                    format!("{:.1}% Used ", d.utilization_pct),
                    Style::default().fg(d.utilization_color()).add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    d.utilization_alert.badge(),
                    Style::default().fg(d.utilization_color()),
                ),
            ];

            let line = Line::from(legend_spans);
            buf.set_line(area.x, legend_y, &line, area.width);
        }
    }
}

// ---------------------------------------------------------------------------
// 9. Dedicated Context Progress Bar Widget (Color Alert Meter)
// ---------------------------------------------------------------------------

/// Visual progress meter widget rendering single context utilization with color alert highlights.
pub struct ContextProgressBarWidget<'a> {
    distribution: &'a ContextDistribution,
    show_percentage: bool,
    show_capacity: bool,
}

impl<'a> ContextProgressBarWidget<'a> {
    /// Creates a new `ContextProgressBarWidget`.
    pub fn new(distribution: &'a ContextDistribution) -> Self {
        Self {
            distribution,
            show_percentage: true,
            show_capacity: true,
        }
    }

    /// Whether to append utilization percentage at the end of the bar.
    pub fn with_percentage(mut self, show: bool) -> Self {
        self.show_percentage = show;
        self
    }

    /// Whether to append capacity indicators (e.g. `128k / 200k`).
    pub fn with_capacity(mut self, show: bool) -> Self {
        self.show_capacity = show;
        self
    }
}

impl<'a> Widget for ContextProgressBarWidget<'a> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        if area.width < 12 || area.height < 1 {
            return;
        }

        let d = self.distribution;
        let color = d.utilization_color();

        let suffix = if self.show_capacity && self.show_percentage {
            format!(
                " {:.1}% ({}/{})",
                d.utilization_pct,
                format_token_count(d.used_tokens),
                format_token_count(d.max_context)
            )
        } else if self.show_percentage {
            format!(" {:.1}%", d.utilization_pct)
        } else if self.show_capacity {
            format!(
                " ({}/{})",
                format_token_count(d.used_tokens),
                format_token_count(d.max_context)
            )
        } else {
            String::new()
        };

        let bar_width = (area.width as usize).saturating_sub(suffix.len() + 2).max(4);
        let filled_chars = ((d.utilization_pct / 100.0) * bar_width as f32).round() as usize;
        let filled = filled_chars.min(bar_width);
        let empty = bar_width.saturating_sub(filled);

        let mut spans = Vec::new();
        spans.push(Span::styled("[", Style::default().fg(Color::DarkGray)));
        spans.push(Span::styled(
            BLOCK_FULL.repeat(filled),
            Style::default().fg(color).add_modifier(Modifier::BOLD),
        ));
        spans.push(Span::styled(
            BLOCK_LIGHT_SHADE.repeat(empty),
            Style::default().fg(Color::DarkGray),
        ));
        spans.push(Span::styled("]", Style::default().fg(Color::DarkGray)));

        if !suffix.is_empty() {
            spans.push(Span::styled(
                suffix,
                Style::default().fg(color).add_modifier(Modifier::BOLD),
            ));
        }

        let line = Line::from(spans);
        buf.set_line(area.x, area.y, &line, area.width);
    }
}

// ---------------------------------------------------------------------------
// 10. Full Context Inspector Widget (Ratatui TUI)
// ---------------------------------------------------------------------------

/// Full interactive context inspector dialog widget with tabs, metrics, and message explorer.
pub struct ContextInspectorWidget<'a> {
    distribution: &'a ContextDistribution,
    state: &'a ContextInspectorState,
}

impl<'a> ContextInspectorWidget<'a> {
    /// Creates a new `ContextInspectorWidget`.
    pub fn new(distribution: &'a ContextDistribution, state: &'a ContextInspectorState) -> Self {
        Self {
            distribution,
            state,
        }
    }

    /// Renders the top header banner with Model info, Capacity gauge, and Alert badge.
    fn render_header(&self, area: Rect, buf: &mut Buffer) {
        let d = self.distribution;
        let status_color = d.utilization_color();
        let (status_badge, _) = d.utilization_badge();

        let title_line = Line::from(vec![
            Span::styled(" 🧠 CONTEXT WINDOW & TOKEN BUDGET ", Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)),
            Span::styled(format!("• Model: {} ", d.model), Style::default().fg(Color::White).add_modifier(Modifier::BOLD)),
            Span::styled(
                format!("(Capacity: {} • Used: {} / {:.1}%) ", d.format_capacity(), format_token_count(d.used_tokens), d.utilization_pct),
                Style::default().fg(Color::Gray),
            ),
            Span::styled(format!(" {} ", status_badge), Style::default().fg(Color::Black).bg(status_color).add_modifier(Modifier::BOLD)),
        ]);

        buf.set_line(area.x, area.y, &title_line, area.width);
    }

    /// Renders the tab navigation bar.
    fn render_tab_bar(&self, area: Rect, buf: &mut Buffer) {
        let mut spans = Vec::new();
        spans.push(Span::raw(" "));

        for tab in ContextInspectorTab::ALL {
            let is_active = tab == self.state.active_tab;
            let tab_num = tab.index() + 1;

            if is_active {
                spans.push(Span::styled(
                    format!(" [{}. {} {}] ", tab_num, tab.icon(), tab.name()),
                    Style::default()
                        .fg(Color::Black)
                        .bg(Color::Cyan)
                        .add_modifier(Modifier::BOLD),
                ));
            } else {
                spans.push(Span::styled(
                    format!("  {}. {} {}  ", tab_num, tab.icon(), tab.name()),
                    Style::default().fg(Color::Gray),
                ));
            }
            spans.push(Span::raw(" "));
        }

        let line = Line::from(spans);
        buf.set_line(area.x, area.y, &line, area.width);
    }

    /// Renders Tab 1: Overview (Cards + Token Budget / Cache + Diagnostics + Top consumers).
    fn render_overview_tab(&self, area: Rect, buf: &mut Buffer) {
        let d = self.distribution;
        let theme = &self.state.theme;

        if area.height < 6 {
            return;
        }

        // Split into: (1) Category 4-Card Grid, (2) Token Budget & Cache Panel, (3) Diagnostics & Top Consumers
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(5), // 4 Cards
                Constraint::Min(5),    // Session Token Budget & Diagnostics Grid
            ])
            .split(area);

        // 1. Render 4 Category Cards side-by-side
        let card_chunks = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([
                Constraint::Percentage(25),
                Constraint::Percentage(25),
                Constraint::Percentage(25),
                Constraint::Percentage(25),
            ])
            .split(chunks[0]);

        let categories = [
            (&d.system_stats, ContextCategory::System),
            (&d.history_stats, ContextCategory::History),
            (&d.tools_stats, ContextCategory::Tools),
            (&d.free_stats, ContextCategory::Free),
        ];

        for (i, (stats, cat)) in categories.iter().enumerate() {
            let is_selected = self.state.selected_index == i;
            let border_color = if is_selected {
                Color::White
            } else {
                cat.ratatui_color(theme)
            };

            let card_block = Block::default()
                .title(format!(" {} {} ", cat.icon(), cat.name()))
                .borders(Borders::ALL)
                .border_type(if is_selected { BorderType::Thick } else { BorderType::Rounded })
                .border_style(Style::default().fg(border_color));

            let inner = card_block.inner(card_chunks[i]);
            card_block.render(card_chunks[i], buf);

            if inner.height >= 2 {
                let line1 = Line::from(vec![
                    Span::styled(
                        format!("{}", stats.formatted_tokens),
                        Style::default().fg(cat.ratatui_color(theme)).add_modifier(Modifier::BOLD),
                    ),
                    Span::styled(
                        format!(" ({:.1}%)", stats.pct_of_total),
                        Style::default().fg(Color::Gray),
                    ),
                ]);
                buf.set_line(inner.x, inner.y, &line1, inner.width);

                let line2 = Line::from(vec![
                    Span::styled(
                        format!("{} {}", stats.item_count, stats.item_label),
                        Style::default().fg(Color::DarkGray),
                    ),
                ]);
                buf.set_line(inner.x, inner.y + 1, &line2, inner.width);
            }
        }

        // 2. Render Diagnostics & Session Token Budget & Top Consumers
        if chunks[1].height >= 4 {
            let bottom_chunks = Layout::default()
                .direction(Direction::Horizontal)
                .constraints([
                    Constraint::Percentage(50), // Session Token Budget & Cache Usage
                    Constraint::Percentage(50), // Health Diagnostics & Top Message Consumers
                ])
                .split(chunks[1]);

            // Left Box: Session Token Budget & Cache Usage
            let budget_block = Block::default()
                .title(" 💳 Token Budget & Provider Cache Usage ")
                .borders(Borders::ALL)
                .border_type(BorderType::Rounded)
                .border_style(Style::default().fg(Color::Green));
            let budget_inner = budget_block.inner(bottom_chunks[0]);
            budget_block.render(bottom_chunks[0], buf);

            let mut budget_lines = Vec::new();
            budget_lines.push(Line::from(vec![
                Span::styled("• Total Capacity:     ", Style::default().fg(Color::Gray)),
                Span::styled(d.format_capacity(), Style::default().fg(Color::White).add_modifier(Modifier::BOLD)),
                Span::styled(format!(" ({:.1}% effective budget)", (d.effective_budget as f32 / d.max_context.max(1) as f32) * 100.0), Style::default().fg(Color::DarkGray)),
            ]));
            budget_lines.push(Line::from(vec![
                Span::styled("• Prompt Tokens:      ", Style::default().fg(Color::Gray)),
                Span::styled(format!("{} ({} cumulative)", format_token_count(d.used_tokens), d.format_prompt_tokens()), Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)),
            ]));
            budget_lines.push(Line::from(vec![
                Span::styled("• Completion Tokens:  ", Style::default().fg(Color::Gray)),
                Span::styled(d.format_completion_tokens(), Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)),
                Span::styled(format!(" across {} turn{}", d.total_turns, if d.total_turns == 1 { "" } else { "s" }), Style::default().fg(Color::Gray)),
            ]));
            budget_lines.push(Line::from(vec![
                Span::styled("• Cache Usage:        ", Style::default().fg(Color::Gray)),
                Span::styled(
                    format!("{} Read • {} Written ", d.format_cache_read_tokens(), d.format_cache_write_tokens()),
                    Style::default().fg(Color::Green).add_modifier(Modifier::BOLD),
                ),
                Span::styled(format!("(Hit Rate: {:.1}%)", d.cache_hit_rate()), Style::default().fg(Color::White)),
            ]));
            budget_lines.push(Line::from(vec![
                Span::styled("• Context Alert:      ", Style::default().fg(Color::Gray)),
                Span::styled(d.utilization_alert.label(), Style::default().fg(d.utilization_color()).add_modifier(Modifier::BOLD)),
            ]));

            for (idx, line) in budget_lines.iter().enumerate() {
                if idx < budget_inner.height as usize {
                    buf.set_line(budget_inner.x, budget_inner.y + idx as u16, line, budget_inner.width);
                }
            }

            // Right Box: Top Message Consumers
            let top_block = Block::default()
                .title(" 🏆 Largest Message Consumers ")
                .borders(Borders::ALL)
                .border_type(BorderType::Rounded)
                .border_style(Style::default().fg(Color::Magenta));
            let top_inner = top_block.inner(bottom_chunks[1]);
            top_block.render(bottom_chunks[1], buf);

            let top_msgs = d.top_messages_by_size(4);
            if top_msgs.is_empty() {
                buf.set_line(
                    top_inner.x,
                    top_inner.y,
                    &Line::from(Span::styled("No conversation messages in session", Style::default().fg(Color::DarkGray))),
                    top_inner.width,
                );
            } else {
                for (idx, msg) in top_msgs.iter().enumerate() {
                    if idx < top_inner.height as usize {
                        let line = Line::from(vec![
                            Span::styled(format!("#{:<2} ", msg.index + 1), Style::default().fg(Color::DarkGray)),
                            Span::styled(format!("{:<6} ", msg.role_badge()), Style::default().fg(msg.role_color())),
                            Span::styled(format!("{:<6} ", msg.formatted_tokens), Style::default().fg(Color::White).add_modifier(Modifier::BOLD)),
                            Span::styled(format!("({:.1}%) ", msg.pct_of_history), Style::default().fg(Color::Gray)),
                            Span::styled(msg.preview.chars().take(24).collect::<String>(), Style::default().fg(Color::DarkGray)),
                        ]);
                        buf.set_line(top_inner.x, top_inner.y + idx as u16, &line, top_inner.width);
                    }
                }
            }
        }
    }

    /// Renders Tab 2: System Prompt Breakdown (Base Instructions, Tool Definitions, Skills, Memory).
    fn render_system_tab(&self, area: Rect, buf: &mut Buffer) {
        let d = self.distribution;
        let b = &d.system_breakdown;

        if area.height < 6 {
            return;
        }

        // Layout: (1) 4 Summary Category Cards, (2) Granular Section List
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(4), // 4 Pillar Category Cards
                Constraint::Min(4),    // Section items list
            ])
            .split(area);

        // 1. Pillar Category Cards (Base Instructions, Tool Definitions, Skills, Memory)
        let pillar_chunks = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([
                Constraint::Percentage(25),
                Constraint::Percentage(25),
                Constraint::Percentage(25),
                Constraint::Percentage(25),
            ])
            .split(chunks[0]);

        let pillars = [
            (
                SystemSectionCategory::BaseInstructions,
                b.base_instructions_tokens,
                b.base_instructions_pct,
            ),
            (
                SystemSectionCategory::ToolDefinitions,
                b.tool_definitions_tokens,
                b.tool_definitions_pct,
            ),
            (
                SystemSectionCategory::Skills,
                b.skills_tokens,
                b.skills_pct,
            ),
            (
                SystemSectionCategory::Memory,
                b.memory_tokens,
                b.memory_pct,
            ),
        ];

        for (i, (cat, tokens, pct)) in pillars.iter().enumerate() {
            let card_block = Block::default()
                .title(format!(" {} {} ", cat.icon(), cat.name()))
                .borders(Borders::ALL)
                .border_type(BorderType::Rounded)
                .border_style(Style::default().fg(cat.color()));

            let inner = card_block.inner(pillar_chunks[i]);
            card_block.render(pillar_chunks[i], buf);

            if inner.height >= 1 {
                let line1 = Line::from(vec![
                    Span::styled(
                        format_token_count(*tokens),
                        Style::default().fg(cat.color()).add_modifier(Modifier::BOLD),
                    ),
                    Span::styled(
                        format!(" ({:.1}%)", pct),
                        Style::default().fg(Color::Gray),
                    ),
                ]);
                buf.set_line(inner.x, inner.y, &line1, inner.width);
            }
        }

        // 2. Granular Section Breakdown Table
        let list_block = Block::default()
            .title(format!(
                " 📋 Granular Section Breakdown ({} total tokens, {:.1}% of window) ",
                format_token_count(d.system_tokens),
                d.system_pct
            ))
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(Color::Cyan));

        let list_inner = list_block.inner(chunks[1]);
        list_block.render(chunks[1], buf);

        if d.system_sections.is_empty() {
            buf.set_line(
                list_inner.x,
                list_inner.y,
                &Line::from(Span::styled(
                    "No active system prompt attached to this session.",
                    Style::default().fg(Color::DarkGray),
                )),
                list_inner.width,
            );
            return;
        }

        for (idx, sec) in d.system_sections.iter().enumerate() {
            let y = list_inner.y + (idx * 2) as u16;
            if y + 1 >= list_inner.bottom() {
                break;
            }

            let is_selected = self.state.selected_index == idx;
            let title_style = if is_selected {
                Style::default().fg(Color::Black).bg(Color::Cyan).add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)
            };

            let line1 = Line::from(vec![
                Span::styled(format!(" {} ", sec.category.badge()), Style::default().fg(sec.category.color()).add_modifier(Modifier::BOLD)),
                Span::styled(format!("📌 {} ", sec.title), title_style),
                Span::styled(
                    format!("• {} tokens ({:.1}% of system, {:.2}% total) ", sec.formatted_tokens, sec.pct_of_system, sec.pct_of_total),
                    Style::default().fg(Color::White),
                ),
                Span::styled(format!("• {} lines", sec.line_count), Style::default().fg(Color::Gray)),
            ]);
            buf.set_line(list_inner.x, y, &line1, list_inner.width);

            let line2 = Line::from(vec![
                Span::raw("      "),
                Span::styled(
                    sec.preview.chars().take(list_inner.width.saturating_sub(8) as usize).collect::<String>(),
                    Style::default().fg(Color::DarkGray),
                ),
            ]);
            buf.set_line(list_inner.x, y + 1, &line2, list_inner.width);
        }
    }

    /// Renders Tab 3: History Messages Table.
    fn render_history_tab(&self, area: Rect, buf: &mut Buffer) {
        let d = self.distribution;

        let block = Block::default()
            .title(format!(
                " 💬 Conversation History ({} messages, {} tokens, {:.1}% of window) ",
                d.messages.len(),
                format_token_count(d.history_tokens),
                d.history_pct
            ))
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(Color::Magenta));

        let inner = block.inner(area);
        block.render(area, buf);

        if d.messages.is_empty() {
            buf.set_line(
                inner.x,
                inner.y,
                &Line::from(Span::styled(
                    "No conversation messages in session.",
                    Style::default().fg(Color::DarkGray),
                )),
                inner.width,
            );
            return;
        }

        // Table Header
        if inner.height >= 2 {
            let header = Line::from(vec![
                Span::styled("  #  ", Style::default().fg(Color::Gray).add_modifier(Modifier::BOLD)),
                Span::styled("Role       ", Style::default().fg(Color::Gray).add_modifier(Modifier::BOLD)),
                Span::styled("Tokens    ", Style::default().fg(Color::Gray).add_modifier(Modifier::BOLD)),
                Span::styled("Share   ", Style::default().fg(Color::Gray).add_modifier(Modifier::BOLD)),
                Span::styled("Preview", Style::default().fg(Color::Gray).add_modifier(Modifier::BOLD)),
            ]);
            buf.set_line(inner.x, inner.y, &header, inner.width);
        }

        let visible_rows = inner.height.saturating_sub(1) as usize;
        let start_idx = self.state.scroll_offset;
        let end_idx = (start_idx + visible_rows).min(d.messages.len());

        for (row_offset, idx) in (start_idx..end_idx).enumerate() {
            let y = inner.y + 1 + row_offset as u16;
            let msg = &d.messages[idx];
            let is_selected = self.state.selected_index == idx;

            let line = Line::from(vec![
                Span::styled(
                    format!(" {:<3} ", idx + 1),
                    if is_selected {
                        Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)
                    } else {
                        Style::default().fg(Color::DarkGray)
                    },
                ),
                Span::styled(
                    format!("{:<10} ", msg.role_badge()),
                    Style::default().fg(msg.role_color()).add_modifier(Modifier::BOLD),
                ),
                Span::styled(format!("{:<9} ", msg.formatted_tokens), Style::default().fg(Color::White)),
                Span::styled(format!("{:>4.1}%  ", msg.pct_of_history), Style::default().fg(Color::Gray)),
                Span::styled(
                    msg.preview.chars().take(inner.width.saturating_sub(32) as usize).collect::<String>(),
                    Style::default().fg(if is_selected { Color::White } else { Color::DarkGray }),
                ),
            ]);

            buf.set_line(inner.x, y, &line, inner.width);
        }
    }

    /// Renders Tab 4: Tools Definition Table.
    fn render_tools_tab(&self, area: Rect, buf: &mut Buffer) {
        let d = self.distribution;

        let block = Block::default()
            .title(format!(
                " 🔧 Registered Tool Schemas ({} tools, {} tokens, {:.1}% of window) ",
                d.tools.len(),
                format_token_count(d.tools_tokens),
                d.tools_pct
            ))
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(Color::Yellow));

        let inner = block.inner(area);
        block.render(area, buf);

        if d.tools.is_empty() {
            buf.set_line(
                inner.x,
                inner.y,
                &Line::from(Span::styled(
                    "No tools registered in active session.",
                    Style::default().fg(Color::DarkGray),
                )),
                inner.width,
            );
            return;
        }

        // Header
        if inner.height >= 2 {
            let header = Line::from(vec![
                Span::styled("  Tool Name          ", Style::default().fg(Color::Gray).add_modifier(Modifier::BOLD)),
                Span::styled("Category            ", Style::default().fg(Color::Gray).add_modifier(Modifier::BOLD)),
                Span::styled("Params ", Style::default().fg(Color::Gray).add_modifier(Modifier::BOLD)),
                Span::styled("Tokens    ", Style::default().fg(Color::Gray).add_modifier(Modifier::BOLD)),
                Span::styled("Share   ", Style::default().fg(Color::Gray).add_modifier(Modifier::BOLD)),
                Span::styled("Description", Style::default().fg(Color::Gray).add_modifier(Modifier::BOLD)),
            ]);
            buf.set_line(inner.x, inner.y, &header, inner.width);
        }

        let visible_rows = inner.height.saturating_sub(1) as usize;
        let start_idx = self.state.scroll_offset;
        let end_idx = (start_idx + visible_rows).min(d.tools.len());

        for (row_offset, idx) in (start_idx..end_idx).enumerate() {
            let y = inner.y + 1 + row_offset as u16;
            let tool = &d.tools[idx];
            let is_selected = self.state.selected_index == idx;

            let line = Line::from(vec![
                Span::styled(
                    format!("  {:<18} ", tool.name),
                    if is_selected {
                        Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)
                    } else {
                        Style::default().fg(Color::Cyan)
                    },
                ),
                Span::styled(format!("{:<19} ", tool.category), Style::default().fg(Color::Gray)),
                Span::styled(format!("{:<6} ", tool.param_count), Style::default().fg(Color::White)),
                Span::styled(format!("{:<9} ", tool.formatted_tokens), Style::default().fg(Color::White).add_modifier(Modifier::BOLD)),
                Span::styled(format!("{:>4.1}%  ", tool.pct_of_tools), Style::default().fg(Color::Gray)),
                Span::styled(
                    tool.description.chars().take(inner.width.saturating_sub(65) as usize).collect::<String>(),
                    Style::default().fg(Color::DarkGray),
                ),
            ]);

            buf.set_line(inner.x, y, &line, inner.width);
        }
    }

    /// Renders Tab 5: Compaction Simulation Preview.
    fn render_compaction_tab(&self, area: Rect, buf: &mut Buffer) {
        let d = self.distribution;
        let p = &d.compaction_preview;

        let block = Block::default()
            .title(" ⚡ Compaction Simulation & Token Optimization Preview ")
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(Color::Green));

        let inner = block.inner(area);
        block.render(area, buf);

        if inner.height < 6 {
            return;
        }

        let mut lines = Vec::new();
        lines.push(Line::from(vec![
            Span::styled(
                "Current vs Projected Post-Compaction Context Window:",
                Style::default().fg(Color::White).add_modifier(Modifier::BOLD),
            ),
        ]));

        lines.push(Line::from(vec![
            Span::styled("• Current Used Tokens:    ", Style::default().fg(Color::Gray)),
            Span::styled(
                format!("{} tokens ", format_token_count(d.used_tokens)),
                Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                format!("({:.1}% utilization, {} free headroom)", d.utilization_pct, format_token_count(d.free_tokens)),
                Style::default().fg(Color::Gray),
            ),
        ]));

        lines.push(Line::from(vec![
            Span::styled("• Projected Post-Compact: ", Style::default().fg(Color::Gray)),
            Span::styled(
                format!("{} tokens ", format_token_count(p.projected_total_tokens)),
                Style::default().fg(Color::Green).add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                format!("({:.1}% utilization, {} free headroom)", p.projected_utilization_pct, format_token_count(p.projected_free_tokens)),
                Style::default().fg(Color::Gray),
            ),
        ]));

        lines.push(Line::from(vec![
            Span::styled("• Projected Token Savings:", Style::default().fg(Color::Gray)),
            Span::styled(
                format!("-{} tokens (-{:.1}% reduction in history)", format_token_count(p.tokens_saved), p.reduction_pct),
                Style::default().fg(Color::Green).add_modifier(Modifier::BOLD),
            ),
        ]));

        lines.push(Line::from(vec![
            Span::styled("• Pruning Operations:     ", Style::default().fg(Color::Gray)),
            Span::styled(
                format!("{} older tool execution outputs pruned & {} historical turns summarized", p.pruned_tool_count, p.summarized_turn_count),
                Style::default().fg(Color::White),
            ),
        ]));

        lines.push(Line::raw(""));
        lines.push(Line::from(vec![
            Span::styled("Press ", Style::default().fg(Color::Gray)),
            Span::styled("[ Enter ]", Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)),
            Span::styled(" or ", Style::default().fg(Color::Gray)),
            Span::styled("[ c ]", Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)),
            Span::styled(" to execute compaction immediately in the active session.", Style::default().fg(Color::White)),
        ]));

        for (idx, line) in lines.iter().enumerate() {
            if idx < inner.height as usize {
                buf.set_line(inner.x, inner.y + idx as u16, line, inner.width);
            }
        }
    }

    /// Renders bottom keybindings footer.
    fn render_footer(&self, area: Rect, buf: &mut Buffer) {
        let footer_spans = vec![
            Span::styled(" Tab", Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)),
            Span::styled(":Switch Tab  ", Style::default().fg(Color::DarkGray)),
            Span::styled("↑↓/jk", Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)),
            Span::styled(":Navigate  ", Style::default().fg(Color::DarkGray)),
            Span::styled("Enter/Space", Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)),
            Span::styled(":Inspect  ", Style::default().fg(Color::DarkGray)),
            Span::styled("c", Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)),
            Span::styled(":Compact  ", Style::default().fg(Color::DarkGray)),
            Span::styled("?", Style::default().fg(Color::White).add_modifier(Modifier::BOLD)),
            Span::styled(":Help  ", Style::default().fg(Color::DarkGray)),
            Span::styled("Esc/q", Style::default().fg(Color::Red).add_modifier(Modifier::BOLD)),
            Span::styled(":Close", Style::default().fg(Color::DarkGray)),
        ];

        let line = Line::from(footer_spans);
        buf.set_line(area.x, area.y, &line, area.width);
    }

    /// Renders help overlay modal.
    fn render_help_modal(&self, area: Rect, buf: &mut Buffer) {
        let help_width = (area.width.saturating_sub(10)).min(60);
        let help_height = (area.height.saturating_sub(4)).min(16);

        let help_x = area.x + (area.width.saturating_sub(help_width)) / 2;
        let help_y = area.y + (area.height.saturating_sub(help_height)) / 2;
        let help_rect = Rect::new(help_x, help_y, help_width, help_height);

        Clear.render(help_rect, buf);

        let block = Block::default()
            .title(" ❓ Context Inspector Keybindings ")
            .borders(Borders::ALL)
            .border_type(BorderType::Double)
            .border_style(Style::default().fg(Color::Yellow));

        let inner = block.inner(help_rect);
        block.render(help_rect, buf);

        let help_lines = [
            ("Tab / Shift+Tab", "Cycle through inspector tabs"),
            ("1 .. 5", "Jump directly to specific tab"),
            ("↑ / ↓ or j / k", "Navigate items in active tab"),
            ("PgUp / PgDn", "Scroll one page up / down"),
            ("Home / End", "Jump to top / bottom"),
            ("Enter / Space", "Drill-down / inspect selected item"),
            ("c", "Preview or trigger compaction"),
            ("d", "Toggle item detail drawer"),
            ("?", "Toggle this help modal"),
            ("Esc / q", "Close context inspector"),
        ];

        for (idx, (key, desc)) in help_lines.iter().enumerate() {
            if idx < inner.height as usize {
                let line = Line::from(vec![
                    Span::styled(format!("  {:<16} ", key), Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)),
                    Span::styled(*desc, Style::default().fg(Color::White)),
                ]);
                buf.set_line(inner.x, inner.y + idx as u16, &line, inner.width);
            }
        }
    }
}

impl<'a> Widget for ContextInspectorWidget<'a> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        if area.width < MIN_INSPECTOR_WIDTH || area.height < 6 {
            return;
        }

        // Layout partitions:
        // Row 0: Header banner (Model, Capacity, Alert)
        // Row 1: Stacked visual bar widget
        // Row 2: Tab navigation bar
        // Row 3..N-2: Tab Content Area
        // Row N-1: Footer keybindings
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(1), // Header
                Constraint::Length(1), // Stacked visual bar
                Constraint::Length(1), // Tab Bar
                Constraint::Min(4),    // Content
                Constraint::Length(1), // Footer
            ])
            .split(area);

        // 1. Header
        self.render_header(chunks[0], buf);

        // 2. Visual Stacked Bar
        let bar_widget = ContextBarWidget::new(self.distribution, &self.state.theme)
            .with_options(ContextBarOptions {
                show_labels: false,
                show_percentages: false,
                show_legend: false,
                compact: true,
            });
        bar_widget.render(chunks[1], buf);

        // 3. Tab Bar
        self.render_tab_bar(chunks[2], buf);

        // 4. Tab Content
        match self.state.active_tab {
            ContextInspectorTab::Overview => self.render_overview_tab(chunks[3], buf),
            ContextInspectorTab::System => self.render_system_tab(chunks[3], buf),
            ContextInspectorTab::History => self.render_history_tab(chunks[3], buf),
            ContextInspectorTab::Tools => self.render_tools_tab(chunks[3], buf),
            ContextInspectorTab::Compaction => self.render_compaction_tab(chunks[3], buf),
        }

        // 5. Footer
        self.render_footer(chunks[4], buf);

        // 6. Help Modal overlay
        if self.state.show_help {
            self.render_help_modal(area, buf);
        }
    }
}

// ---------------------------------------------------------------------------
// 11. Pure Rust ANSI Terminal Renderers (Zero Dependency CLI Output)
// ---------------------------------------------------------------------------

/// Renders a colored multi-segment ANSI context distribution bar.
pub fn render_context_bar_ansi(d: &ContextDistribution, bar_width: usize) -> String {
    let width = bar_width.max(12);
    let total = d.max_context.max(1) as f32;

    let sys_w = ((d.system_tokens as f32 / total) * width as f32).round() as usize;
    let hist_w = ((d.history_tokens as f32 / total) * width as f32).round() as usize;
    let tools_w = ((d.tools_tokens as f32 / total) * width as f32).round() as usize;

    let safe_sys_w = if d.system_tokens > 0 && sys_w == 0 { 1 } else { sys_w };
    let safe_hist_w = if d.history_tokens > 0 && hist_w == 0 { 1 } else { hist_w };
    let safe_tools_w = if d.tools_tokens > 0 && tools_w == 0 { 1 } else { tools_w };

    let used_w = safe_sys_w + safe_hist_w + safe_tools_w;
    let free_w = width.saturating_sub(used_w);

    let mut out = String::with_capacity(256);
    out.push('[');

    // System (Cyan)
    if safe_sys_w > 0 {
        out.push_str(ANSI_BOLD_CYAN);
        out.push_str(&BLOCK_FULL.repeat(safe_sys_w));
        out.push_str(ANSI_RESET);
    }

    // History (Magenta)
    if safe_hist_w > 0 {
        out.push_str(ANSI_BOLD_MAGENTA);
        out.push_str(&BLOCK_FULL.repeat(safe_hist_w));
        out.push_str(ANSI_RESET);
    }

    // Tools (Yellow)
    if safe_tools_w > 0 {
        out.push_str(ANSI_BOLD_YELLOW);
        out.push_str(&BLOCK_FULL.repeat(safe_tools_w));
        out.push_str(ANSI_RESET);
    }

    // Free (Green / Light Shade)
    if free_w > 0 {
        out.push_str(ANSI_GREEN);
        out.push_str(&BLOCK_LIGHT_SHADE.repeat(free_w));
        out.push_str(ANSI_RESET);
    }

    out.push(']');
    out
}

/// Renders a single-bar ANSI progress meter with utilization percentage and color alerts.
pub fn render_utilization_bar_ansi(d: &ContextDistribution, bar_width: usize) -> String {
    let width = bar_width.max(12);
    let color_code = d.utilization_ansi_color();
    let filled_chars = ((d.utilization_pct / 100.0) * width as f32).round() as usize;
    let filled = filled_chars.min(width);
    let empty = width.saturating_sub(filled);

    format!(
        "[{}{}{}{}{}] {}{:.1}%{} ({} / {})",
        color_code,
        BLOCK_FULL.repeat(filled),
        ANSI_RESET,
        BLOCK_LIGHT_SHADE.repeat(empty),
        ANSI_RESET,
        color_code,
        d.utilization_pct,
        ANSI_RESET,
        format_token_count(d.used_tokens),
        d.format_capacity(),
    )
}

/// Renders a full multi-line colored ANSI context inspector report.
pub fn render_context_inspector_ansi(d: &ContextDistribution, width: usize) -> String {
    let w = width.max(60);
    let mut out = String::with_capacity(1024);

    let bar = render_context_bar_ansi(d, w.saturating_sub(32));
    let status_str = format!(
        "{}{}{}",
        d.utilization_ansi_color(),
        d.utilization_alert.badge(),
        ANSI_RESET
    );

    out.push_str(&format!(
        "\n{ANSI_BOLD_CYAN}╭────────────────────────────────────────────────────────────────────────────╮{ANSI_RESET}\n"
    ));
    out.push_str(&format!(
        "{ANSI_BOLD_CYAN}│{ANSI_RESET} {ANSI_BOLD}🧠 Context Window Inspector{ANSI_RESET} • Model: {ANSI_CYAN}{}{ANSI_RESET} • Capacity: {ANSI_BOLD}{}{ANSI_RESET}  {} \n",
        d.model, d.format_capacity(), status_str
    ));
    out.push_str(&format!(
        "{ANSI_BOLD_CYAN}├────────────────────────────────────────────────────────────────────────────┤{ANSI_RESET}\n"
    ));

    out.push_str(&format!(
        "  {ANSI_BOLD}Distribution:{ANSI_RESET} {} {ANSI_BOLD}{}{:.1}% used{ANSI_RESET} ({}/{} tokens)\n\n",
        bar,
        d.utilization_ansi_color(),
        d.utilization_pct,
        format_token_count(d.used_tokens),
        format_token_count(d.max_context),
    ));

    // 4 Category breakdown rows
    out.push_str(&format!(
        "  {ANSI_BOLD_CYAN}🖥️  System:{ANSI_RESET}      {:<7} ({:>4.1}%) • Base: {} | Tools: {} | Skills: {} | Mem: {}\n",
        d.system_stats.formatted_tokens,
        d.system_stats.pct_of_total,
        format_token_count(d.system_breakdown.base_instructions_tokens),
        format_token_count(d.system_breakdown.tool_definitions_tokens),
        format_token_count(d.system_breakdown.skills_tokens),
        format_token_count(d.system_breakdown.memory_tokens),
    ));

    out.push_str(&format!(
        "  {ANSI_BOLD_MAGENTA}💬 History:{ANSI_RESET}     {:<7} ({:>4.1}%) • {} {}\n",
        d.history_stats.formatted_tokens,
        d.history_stats.pct_of_total,
        d.history_stats.item_count,
        d.history_stats.item_label,
    ));

    out.push_str(&format!(
        "  {ANSI_BOLD_YELLOW}🔧 Tools:{ANSI_RESET}       {:<7} ({:>4.1}%) • {} {}\n",
        d.tools_stats.formatted_tokens,
        d.tools_stats.pct_of_total,
        d.tools_stats.item_count,
        d.tools_stats.item_label,
    ));

    out.push_str(&format!(
        "  {ANSI_BOLD_GREEN}🟢 Free Budget:{ANSI_RESET} {:<7} ({:>4.1}%) • unallocated headroom\n",
        d.free_stats.formatted_tokens,
        d.free_stats.pct_of_total,
    ));

    // Session Token Budget & Cache line
    out.push_str(&format!(
        "\n  {ANSI_BOLD}💳 Session Tokens:{ANSI_RESET} Prompt: {ANSI_CYAN}{}{ANSI_RESET} • Completion: {ANSI_YELLOW}{}{ANSI_RESET} • Cache: {ANSI_GREEN}{} Read / {} Written{ANSI_RESET} (Hit Rate: {:.1}%)\n",
        d.format_prompt_tokens(),
        d.format_completion_tokens(),
        d.format_cache_read_tokens(),
        d.format_cache_write_tokens(),
        d.cache_hit_rate(),
    ));

    // Compaction recommendation
    if d.is_alert() {
        let p = &d.compaction_preview;
        out.push_str(&format!(
            "\n  {ANSI_BOLD_YELLOW}⚡ Compaction Potential:{ANSI_RESET} -{} tokens (-{:.1}%) via /compact (prunes {} tool outputs)\n",
            format_token_count(p.tokens_saved),
            p.reduction_pct,
            p.pruned_tool_count,
        ));
    }

    out.push_str(&format!(
        "{ANSI_BOLD_CYAN}╰────────────────────────────────────────────────────────────────────────────╯{ANSI_RESET}\n"
    ));

    out
}

/// Renders a concise one-line ANSI summary for status lines.
pub fn render_context_summary_ansi(d: &ContextDistribution) -> String {
    format!(
        "{ANSI_BOLD}Ctx:{ANSI_RESET} {}{}/{} ({:.1}%){ANSI_RESET} [{ANSI_CYAN}Sys: {}{ANSI_RESET} | {ANSI_MAGENTA}Hist: {}{ANSI_RESET} | {ANSI_YELLOW}Tools: {}{ANSI_RESET} | {ANSI_GREEN}Free: {}{ANSI_RESET}]",
        d.utilization_ansi_color(),
        format_token_count(d.used_tokens),
        d.format_capacity(),
        d.utilization_pct,
        d.system_stats.formatted_tokens,
        d.history_stats.formatted_tokens,
        d.tools_stats.formatted_tokens,
        d.free_stats.formatted_tokens,
    )
}

// ---------------------------------------------------------------------------
// 12. Interactive Fullscreen / Inline Terminal Runner
// ---------------------------------------------------------------------------

/// Runs the interactive context inspector in the terminal until dismissed or an action is triggered.
pub fn run_context_inspector(
    session: &Session,
    tools: &[ToolDefinition],
) -> std::io::Result<Option<ContextInspectorResult>> {
    let dist = ContextDistribution::from_session(session, tools);
    let mut state = ContextInspectorState::new();

    let _guard = RawModeGuard::enter()?;
    execute!(stdout(), EnterAlternateScreen, cursor::Hide)?;

    let backend = CrosstermBackend::new(stdout());
    let mut terminal = Terminal::new(backend)?;

    let result = run_context_inspector_loop(&mut terminal, &dist, &mut state);

    execute!(stdout(), LeaveAlternateScreen, cursor::Show)?;
    result
}

/// Internal event loop for the interactive inspector.
fn run_context_inspector_loop(
    terminal: &mut Terminal<CrosstermBackend<Stdout>>,
    dist: &ContextDistribution,
    state: &mut ContextInspectorState,
) -> std::io::Result<Option<ContextInspectorResult>> {
    loop {
        terminal.draw(|f| {
            let widget = ContextInspectorWidget::new(dist, state);
            f.render_widget(widget, f.area());
        })?;

        if event::poll(std::time::Duration::from_millis(100))? {
            if let Event::Key(key_event) = event::read()? {
                if key_event.kind == KeyEventKind::Press {
                    if let Some(action) = state.handle_key(key_event.code, key_event.modifiers, dist) {
                        return Ok(Some(action));
                    }
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// 13. Comprehensive Unit Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::provider::types::Role;

    fn sample_tools() -> Vec<ToolDefinition> {
        vec![
            ToolDefinition {
                name: "bash".to_string(),
                description: "Execute bash shell command".to_string(),
                parameters: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "command": { "type": "string" }
                    },
                    "required": ["command"]
                }),
            },
            ToolDefinition {
                name: "read".to_string(),
                description: "Read file contents".to_string(),
                parameters: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "path": { "type": "string" }
                    },
                    "required": ["path"]
                }),
            },
        ]
    }

    fn sample_messages() -> Vec<Message> {
        vec![
            Message::user("Please analyze src/ui/budget.rs and tell me how it works."),
            Message::assistant("Certainly! Let me inspect the budget warning banners and thresholds."),
            Message::tool_result("call_1", "Contents of budget.rs: 1400 lines of pure Rust code..."),
            Message::assistant("The budget module provides warning banners at 80% and 95% capacity."),
        ]
    }

    #[test]
    fn test_context_distribution_calculation() {
        let tools = sample_tools();
        let messages = sample_messages();
        let sys_prompt = "# Directives\nYou are an expert assistant.\n# Constraints\nPure Rust only.";

        let dist = ContextDistribution::from_parts("claude-3-5-sonnet", Some(sys_prompt), &messages, &tools);

        assert_eq!(dist.model, "claude-3-5-sonnet");
        assert_eq!(dist.max_context, 200_000);
        assert_eq!(dist.format_capacity(), "200.0k tokens");
        assert!(dist.system_tokens > 0);
        assert!(dist.history_tokens > 0);
        assert!(dist.tools_tokens > 0);
        assert_eq!(dist.used_tokens, dist.system_tokens + dist.history_tokens + dist.tools_tokens);
        assert_eq!(dist.free_tokens, dist.max_context.saturating_sub(dist.used_tokens));

        // Proportions
        assert!(dist.system_pct > 0.0);
        assert!(dist.history_pct > 0.0);
        assert!(dist.tools_pct > 0.0);
        assert!(dist.free_pct > 0.0);

        let total_pct = dist.system_pct + dist.history_pct + dist.tools_pct + dist.free_pct;
        assert!((total_pct - 100.0).abs() < 0.5);

        // Sections & Items
        assert!(!dist.system_sections.is_empty());
        assert_eq!(dist.messages.len(), 4);
        assert_eq!(dist.tools.len(), 2);
    }

    #[test]
    fn test_capacity_and_token_stats_tracking() {
        let mut stats = TokenStats::new();
        stats.add(15_420, 3_850);
        stats.record_cache(12_000, 2_400);

        let dist = ContextDistribution::from_parts("claude-3-5-sonnet", None, &[], &[])
            .with_token_stats(&stats);

        assert_eq!(dist.prompt_tokens, 15_420);
        assert_eq!(dist.completion_tokens, 3_850);
        assert_eq!(dist.session_total_tokens, 19_270);
        assert_eq!(dist.cache_read_tokens, 12_000);
        assert_eq!(dist.cache_write_tokens, 2_400);
        assert_eq!(dist.total_turns, 1);

        assert_eq!(dist.format_prompt_tokens(), "15.4k");
        assert_eq!(dist.format_completion_tokens(), "3.9k");
        assert_eq!(dist.format_session_tokens(), "19.3k");
        assert_eq!(dist.format_cache_read_tokens(), "12.0k");
        assert_eq!(dist.format_cache_write_tokens(), "2.4k");

        // Cache hit rate: 12000 / (15420 + 12000) = 43.76%
        let hit_rate = dist.cache_hit_rate();
        assert!(hit_rate > 40.0 && hit_rate < 50.0);
        assert!(dist.format_cache_usage().contains("12.0k read"));
    }

    #[test]
    fn test_utilization_color_alerts_thresholds() {
        // 1. Safe (< 60%, Green)
        let alert_safe = UtilizationAlertLevel::from_percentage(45.0);
        assert_eq!(alert_safe, UtilizationAlertLevel::Safe);
        assert_eq!(alert_safe.color(), Color::Green);
        assert!(!alert_safe.is_alert());
        assert_eq!(alert_safe.badge(), "[✓ SAFE <60%]");

        // 2. Warning (60% - 85%, Yellow)
        let alert_warn1 = UtilizationAlertLevel::from_percentage(60.0);
        assert_eq!(alert_warn1, UtilizationAlertLevel::Warning);
        assert_eq!(alert_warn1.color(), Color::Yellow);
        assert!(alert_warn1.is_alert());

        let alert_warn2 = UtilizationAlertLevel::from_percentage(84.9);
        assert_eq!(alert_warn2, UtilizationAlertLevel::Warning);
        assert_eq!(alert_warn2.color(), Color::Yellow);
        assert_eq!(alert_warn2.badge(), "[⚠ WARN 60-85%]");

        // 3. Critical (> 85%, Red)
        let alert_crit = UtilizationAlertLevel::from_percentage(85.1);
        assert_eq!(alert_crit, UtilizationAlertLevel::Critical);
        assert_eq!(alert_crit.color(), Color::Red);
        assert!(alert_crit.is_alert());
        assert_eq!(alert_crit.badge(), "[🔥 DANGER >85%]");

        // 4. Overflow (> 100%, Red / Overflow)
        let alert_over = UtilizationAlertLevel::from_percentage(105.0);
        assert_eq!(alert_over, UtilizationAlertLevel::Overflow);
        assert!(alert_over.is_alert());
        assert_eq!(alert_over.badge(), "[🚨 OVERFLOW >100%]");
    }

    #[test]
    fn test_system_prompt_breakdown_pillars() {
        let system_text = r#"
# System Directives
You are an advanced AI coding harness for Fusion.

# Tool Definitions & Schema Rules
Follow JSON tool call conventions and avoid unnecessary reads.

# Skills
- `skill://code-simplifier`: Simplifies and refines codebase.
- `skill://scout`: Fast exploratory code scanner.

# Memory & Workspace Context
Working directory is `/workspace/fusion`. Bookmarked 12 symbols.
"#;

        let sections = ContextDistribution::extract_system_sections(system_text, 128_000);
        assert!(!sections.is_empty());

        let breakdown = SystemPromptBreakdown::from_sections(sections.clone(), 500);

        assert!(breakdown.base_instructions_tokens > 0);
        assert!(breakdown.tool_definitions_tokens > 0);
        assert!(breakdown.skills_tokens > 0);
        assert!(breakdown.memory_tokens > 0);

        // Verify that sections are properly tagged
        let categories: Vec<SystemSectionCategory> = sections.iter().map(|s| s.category).collect();
        assert!(categories.contains(&SystemSectionCategory::BaseInstructions));
        assert!(categories.contains(&SystemSectionCategory::ToolDefinitions));
        assert!(categories.contains(&SystemSectionCategory::Skills));
        assert!(categories.contains(&SystemSectionCategory::Memory));
    }

    #[test]
    fn test_context_category_properties() {
        for cat in ContextCategory::ALL {
            assert!(!cat.name().is_empty());
            assert!(!cat.short_name().is_empty());
            assert!(!cat.icon().is_empty());
            assert!(!cat.badge().is_empty());
            assert!(!cat.description().is_empty());
        }
    }

    #[test]
    fn test_compaction_preview_simulation() {
        let messages = sample_messages();
        let dist = ContextDistribution::from_parts("claude-3-5-sonnet", None, &messages, &[]);

        let preview = &dist.compaction_preview;
        assert_eq!(preview.original_history_tokens, dist.history_tokens);
        assert!(preview.compacted_history_tokens <= preview.original_history_tokens);
        assert!(preview.tokens_saved <= preview.original_history_tokens);
        assert!(preview.projected_total_tokens <= dist.used_tokens);
    }

    #[test]
    fn test_ansi_rendering_outputs() {
        let tools = sample_tools();
        let messages = sample_messages();
        let dist = ContextDistribution::from_parts("claude-3-5-sonnet", Some("System prompt"), &messages, &tools);

        let bar_str = render_context_bar_ansi(&dist, 36);
        assert!(bar_str.starts_with('['));
        assert!(bar_str.ends_with(']'));

        let util_bar_str = render_utilization_bar_ansi(&dist, 30);
        assert!(util_bar_str.contains("200.0k"));

        let full_report = render_context_inspector_ansi(&dist, 80);
        assert!(full_report.contains("Context Window Inspector"));
        assert!(full_report.contains("Distribution:"));
        assert!(full_report.contains("System:"));
        assert!(full_report.contains("History:"));
        assert!(full_report.contains("Tools:"));
        assert!(full_report.contains("Free Budget:"));
        assert!(full_report.contains("Session Tokens:"));

        let summary = render_context_summary_ansi(&dist);
        assert!(summary.contains("Ctx:"));
        assert!(summary.contains("Sys:"));
        assert!(summary.contains("Hist:"));
        assert!(summary.contains("Tools:"));
        assert!(summary.contains("Free:"));
    }

    #[test]
    fn test_context_bar_widget_ratatui_buffer() {
        let dist = ContextDistribution::from_parts("claude-3-5-sonnet", Some("System prompt"), &sample_messages(), &sample_tools());
        let theme = Theme::auto();

        let widget = ContextBarWidget::new(&dist, &theme).with_options(ContextBarOptions {
            show_labels: true,
            show_percentages: true,
            show_legend: true,
            compact: false,
        });

        let mut buffer = Buffer::empty(Rect::new(0, 0, 80, 2));
        widget.render(Rect::new(0, 0, 80, 2), &mut buffer);

        // Verify buffer got populated with non-empty cells
        let first_cell = &buffer[(0, 0)];
        assert_eq!(first_cell.symbol(), BLOCK_FULL);
    }

    #[test]
    fn test_context_progress_bar_widget_ratatui_buffer() {
        let dist = ContextDistribution::from_parts("claude-3-5-sonnet", Some("System prompt"), &sample_messages(), &sample_tools());
        let widget = ContextProgressBarWidget::new(&dist).with_percentage(true).with_capacity(true);

        let mut buffer = Buffer::empty(Rect::new(0, 0, 60, 1));
        widget.render(Rect::new(0, 0, 60, 1), &mut buffer);

        let first_cell = &buffer[(0, 0)];
        assert_eq!(first_cell.symbol(), "[");
    }

    #[test]
    fn test_context_inspector_widget_ratatui_buffer_all_tabs() {
        let dist = ContextDistribution::from_parts("claude-3-5-sonnet", Some("# Sys\nRules\n# Skills\nWorkflow\n# Memory\nState"), &sample_messages(), &sample_tools());

        for tab in ContextInspectorTab::ALL {
            let mut state = ContextInspectorState::new();
            state.set_tab(tab);

            let widget = ContextInspectorWidget::new(&dist, &state);
            let mut buffer = Buffer::empty(Rect::new(0, 0, 90, 24));
            widget.render(Rect::new(0, 0, 90, 24), &mut buffer);

            // Header should be rendered in every tab
            let header_str: String = (0..30).map(|x| buffer[(x, 0)].symbol()).collect();
            assert!(header_str.contains("CONTEXT"));
        }
    }

    #[test]
    fn test_state_tab_and_navigation() {
        let dist = ContextDistribution::from_parts("claude-3-5-sonnet", None, &sample_messages(), &sample_tools());
        let mut state = ContextInspectorState::new();

        assert_eq!(state.active_tab, ContextInspectorTab::Overview);
        state.next_tab();
        assert_eq!(state.active_tab, ContextInspectorTab::System);
        state.next_tab();
        assert_eq!(state.active_tab, ContextInspectorTab::History);
        state.next_tab();
        assert_eq!(state.active_tab, ContextInspectorTab::Tools);
        state.next_tab();
        assert_eq!(state.active_tab, ContextInspectorTab::Compaction);
        state.next_tab();
        assert_eq!(state.active_tab, ContextInspectorTab::Overview);

        // Direct selection
        state.set_tab(ContextInspectorTab::History);
        assert_eq!(state.active_tab, ContextInspectorTab::History);

        // Key handling
        let res = state.handle_key(KeyCode::Char('q'), KeyModifiers::NONE, &dist);
        assert_eq!(res, Some(ContextInspectorResult::Closed));
    }

    #[test]
    fn test_message_context_item_role_badge() {
        let user_item = MessageContextItem {
            index: 0,
            turn: 1,
            role: Role::User,
            tokens: 50,
            formatted_tokens: "50".to_string(),
            pct_of_history: 25.0,
            pct_of_total: 0.1,
            char_count: 200,
            preview: "Hello".to_string(),
            has_tools: false,
            tool_call_count: 0,
            is_exact: false,
        };

        assert_eq!(user_item.role_badge(), "[USER]");
        assert_eq!(user_item.role_icon(), "👤");
        assert_eq!(user_item.role_color(), Color::Cyan);
    }
}
