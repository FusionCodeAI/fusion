//! Interactive Tabbed Model Selector Widget
//!
//! Provides a polished, keyboard-driven model picker matching the fx.sh UX:
//! - Provider tabs: `[All] Anthropic OpenAI DeepSeek xAI Ollama OpenRouter` (cycled via `Tab` / `1-7`).
//! - Model rows showing: Model ID, Context Window (e.g. `200K ctx`, `1M context`), Output Tokens (`128K out`),
//!   Pricing (`$3/$15`, `Free`), Speed (`⚡ Fast`), and Capability Badges (`[Reasoning]`, `[Vision]`).
//! - Selected model info bar displaying full metadata and description.
//! - Key hints footer: `↑↓ Navigate  Tab Provider  1-7 Jump  / Search  Enter Use  Esc Close`.
//! - Grouping helpers by provider for multi-provider overviews.
//! - Renders seamlessly inside Ratatui inline viewport or full-screen viewports.

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
    Frame,
};
use serde::{Deserialize, Serialize};
use std::io::{stdout, Write};

use crate::ui::inline::InlineTerminal;
use crate::ui::prompt::RawModeGuard;

// ---------------------------------------------------------------------------
// Provider Tabs
// ---------------------------------------------------------------------------

/// Provider tabs selectable along the top header bar.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ProviderTab {
    All,
    Fusion,
}

impl ProviderTab {
    /// Ordered list of all tabs.
    pub const ALL: [ProviderTab; 2] = [ProviderTab::All, ProviderTab::Fusion];

    /// Human-readable tab label.
    pub fn name(&self) -> &'static str {
        match self {
            ProviderTab::All => "All",
            ProviderTab::Fusion => "Fusion",
        }
    }

    /// Short abbreviation for narrow screens.
    pub fn short_name(&self) -> &'static str {
        match self {
            ProviderTab::All => "All",
            ProviderTab::Fusion => "Fus",
        }
    }

    /// Check if a model's provider string matches this tab.
    pub fn matches_provider(&self, provider: &str) -> bool {
        match self {
            ProviderTab::All => true,
            ProviderTab::Fusion => provider.eq_ignore_ascii_case("fusion"),
        }
    }

    /// Cycle forward to the next tab (`Tab`).
    pub fn next(&self) -> Self {
        match self {
            ProviderTab::All => ProviderTab::Fusion,
            ProviderTab::Fusion => ProviderTab::All,
        }
    }

    /// Cycle backward to the previous tab (`Shift+Tab` / `BackTab`).
    pub fn prev(&self) -> Self {
        self.next()
    }

    /// Get zero-based index of this tab.
    pub fn index(&self) -> usize {
        match self {
            ProviderTab::All => 0,
            ProviderTab::Fusion => 1,
        }
    }

    /// Construct tab from zero-based index.
    pub fn from_index(idx: usize) -> Option<Self> {
        Self::ALL.get(idx).copied()
    }
}

// ---------------------------------------------------------------------------
// Model Entry
// ---------------------------------------------------------------------------

/// Representation of an LLM model in the picker.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelEntry {
    /// Canonical model identifier (e.g. `claude-3-7-sonnet-20250219`, `gpt-4o`)
    pub id: String,
    /// Friendly display name
    pub name: String,
    /// Provider name (`anthropic`, `openai`, `deepseek`, `xai`, `ollama`, `openrouter`)
    pub provider: String,
    /// Raw context window in tokens (e.g. 1_000_000, 200_000)
    pub context_window: Option<u64>,
    /// Raw maximum output tokens (e.g. 128_000, 16_384, 8_192)
    pub max_output_tokens: Option<u64>,
    /// Formatted context window string (e.g. `"1M context"`, `"200K context"`)
    pub context_display: String,
    /// Formatted output tokens string (e.g. `"128K output"`, `"16K output"`)
    pub output_display: String,
    /// Estimated prompt input pricing in USD per 1M tokens (e.g. 3.0 for $3.00/1M)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input_cost_per_m: Option<f64>,
    /// Estimated completion output pricing in USD per 1M tokens (e.g. 15.0 for $15.00/1M)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_cost_per_m: Option<f64>,
    /// Formatted pricing string (e.g. `"$3/$15"`, `"$0.14/$0.28"`, `"Free"`)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pricing_display: Option<String>,
    /// Speed indicator (e.g. `"Fast"`, `"Very Fast"`, `"Instant"`, `"Reasoning"`, `"Local"`)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub speed: Option<String>,
    /// Feature and category badges (e.g. `["Fast", "Vision", "Reasoning"]`)
    pub badges: Vec<String>,
    /// Optional short description of model capabilities
    pub description: Option<String>,
}

impl PartialEq for ModelEntry {
    fn eq(&self, other: &Self) -> bool {
        self.id == other.id
            && self.name == other.name
            && self.provider == other.provider
            && self.context_window == other.context_window
            && self.max_output_tokens == other.max_output_tokens
            && self.context_display == other.context_display
            && self.output_display == other.output_display
            && self.input_cost_per_m.map(|f| f.to_bits())
                == other.input_cost_per_m.map(|f| f.to_bits())
            && self.output_cost_per_m.map(|f| f.to_bits())
                == other.output_cost_per_m.map(|f| f.to_bits())
            && self.pricing_display == other.pricing_display
            && self.speed == other.speed
            && self.badges == other.badges
            && self.description == other.description
    }
}

impl Eq for ModelEntry {}

impl ModelEntry {
    /// Create a new model entry with explicit string displays.
    pub fn new(
        id: impl Into<String>,
        provider: impl Into<String>,
        context_display: impl Into<String>,
        output_display: impl Into<String>,
        badges: Vec<impl Into<String>>,
    ) -> Self {
        let id_str = id.into();
        Self {
            name: id_str.clone(),
            id: id_str,
            provider: provider.into(),
            context_window: None,
            max_output_tokens: None,
            context_display: context_display.into(),
            output_display: output_display.into(),
            input_cost_per_m: None,
            output_cost_per_m: None,
            pricing_display: None,
            speed: None,
            badges: badges.into_iter().map(Into::into).collect(),
            description: None,
        }
    }

    /// Create a model entry with numeric token values, formatting them automatically.
    pub fn with_tokens(
        id: impl Into<String>,
        provider: impl Into<String>,
        context_tokens: u64,
        output_tokens: u64,
        badges: Vec<impl Into<String>>,
    ) -> Self {
        let id_str = id.into();
        Self {
            name: id_str.clone(),
            id: id_str,
            provider: provider.into(),
            context_window: Some(context_tokens),
            max_output_tokens: Some(output_tokens),
            context_display: format_context(context_tokens),
            output_display: format_output(output_tokens),
            input_cost_per_m: None,
            output_cost_per_m: None,
            pricing_display: None,
            speed: None,
            badges: badges.into_iter().map(Into::into).collect(),
            description: None,
        }
    }

    /// Builder method to set friendly name.
    pub fn with_name(mut self, name: impl Into<String>) -> Self {
        self.name = name.into();
        self
    }

    /// Builder method to set description.
    pub fn with_description(mut self, desc: impl Into<String>) -> Self {
        self.description = Some(desc.into());
        self
    }

    /// Builder method to set input and output pricing in USD per 1M tokens.
    pub fn with_pricing(mut self, input_per_m: f64, output_per_m: f64) -> Self {
        self.input_cost_per_m = Some(input_per_m);
        self.output_cost_per_m = Some(output_per_m);
        self.pricing_display = Some(format_pricing_pair(input_per_m, output_per_m));
        self
    }

    /// Builder method to set free model pricing.
    pub fn with_free_pricing(mut self) -> Self {
        self.input_cost_per_m = Some(0.0);
        self.output_cost_per_m = Some(0.0);
        self.pricing_display = Some("Free".to_string());
        self
    }

    /// Builder method to set custom pricing display string.
    pub fn with_pricing_display(mut self, display: impl Into<String>) -> Self {
        self.pricing_display = Some(display.into());
        self
    }

    /// Builder method to set model speed rating.
    pub fn with_speed(mut self, speed: impl Into<String>) -> Self {
        self.speed = Some(speed.into());
        self
    }

    /// Returns human-readable pricing string (e.g. `"$3/$15"`, `"$0.14/$0.28"`, `"Free"`, `"-"`).
    pub fn formatted_pricing(&self) -> String {
        if let Some(disp) = &self.pricing_display {
            return disp.clone();
        }
        match (self.input_cost_per_m, self.output_cost_per_m) {
            (Some(inp), Some(out)) => format_pricing_pair(inp, out),
            (Some(inp), None) => format!("${:.2}/M", inp),
            (None, Some(out)) => format!("${:.2}/M", out),
            (None, None) => "-".to_string(),
        }
    }

    /// Returns model speed string, inferring from badges if not explicitly configured.
    pub fn formatted_speed(&self) -> String {
        if let Some(spd) = &self.speed {
            return spd.clone();
        }
        if self.badges.iter().any(|b| b.eq_ignore_ascii_case("fast")) {
            "Fast".to_string()
        } else if self.badges.iter().any(|b| b.eq_ignore_ascii_case("local")) {
            "Local".to_string()
        } else if self
            .badges
            .iter()
            .any(|b| b.eq_ignore_ascii_case("reasoning") || b.eq_ignore_ascii_case("r1"))
        {
            "Reasoning".to_string()
        } else {
            "Standard".to_string()
        }
    }
}

impl From<crate::provider::catalog::CatalogModel> for ModelEntry {
    fn from(cm: crate::provider::catalog::CatalogModel) -> Self {
        let ctx_display = cm.formatted_context_window();
        let out_display = cm.formatted_max_output();
        Self {
            id: cm.id,
            name: cm.name,
            provider: cm.provider,
            context_window: cm.context_window,
            max_output_tokens: cm.max_output_tokens,
            context_display: if ctx_display == "-" {
                String::new()
            } else {
                ctx_display
            },
            output_display: if out_display == "-" {
                String::new()
            } else {
                out_display
            },
            input_cost_per_m: None,
            output_cost_per_m: None,
            pricing_display: None,
            speed: None,
            badges: cm.badges,
            description: cm.description,
        }
    }
}

impl From<ModelEntry> for crate::provider::catalog::CatalogModel {
    fn from(m: ModelEntry) -> Self {
        crate::provider::catalog::CatalogModel {
            id: m.id,
            name: m.name,
            provider: m.provider,
            context_window: m.context_window,
            max_output_tokens: m.max_output_tokens,
            input_cost_per_m: m.input_cost_per_m,
            output_cost_per_m: m.output_cost_per_m,
            knowledge_cutoff: None,
            capabilities: Default::default(),
            badges: m.badges,
            description: m.description,
        }
    }
}

/// Format token count into standard notation (e.g. `1M`, `200K`, `128K`, `8K`).
pub fn format_tokens(tokens: u64) -> String {
    if tokens >= 1_000_000 {
        let millions = tokens as f64 / 1_000_000.0;
        if millions.fract() == 0.0 {
            format!("{:.0}M", millions)
        } else {
            format!("{:.1}M", millions)
        }
    } else if tokens >= 1_000 {
        let thousands = tokens as f64 / 1_000.0;
        if thousands.fract() == 0.0 {
            format!("{:.0}K", thousands)
        } else {
            format!("{:.0}K", thousands.round())
        }
    } else {
        tokens.to_string()
    }
}

/// Format token count as context string (e.g. `"1M context"`, `"200K context"`).
pub fn format_context(tokens: u64) -> String {
    format!("{} context", format_tokens(tokens))
}

/// Format token count as output string (e.g. `"128K output"`, `"8K output"`).
pub fn format_output(tokens: u64) -> String {
    format!("{} output", format_tokens(tokens))
}

/// Format input/output pricing pair per 1M tokens into compact notation.
pub fn format_pricing_pair(input_per_m: f64, output_per_m: f64) -> String {
    if input_per_m == 0.0 && output_per_m == 0.0 {
        return "Free".to_string();
    }

    let format_val = |v: f64| -> String {
        if v == 0.0 {
            "$0".to_string()
        } else if v < 1.0 {
            format!("${:.2}", v)
        } else if v.fract() == 0.0 {
            format!("${:.0}", v)
        } else {
            format!("${:.2}", v)
        }
    };

    format!("{}/{}", format_val(input_per_m), format_val(output_per_m))
}

/// Group a slice of models by provider tab.
pub fn group_by_provider<'a>(models: &'a [ModelEntry]) -> Vec<(ProviderTab, Vec<&'a ModelEntry>)> {
    let mut groups = Vec::new();
    for tab in ProviderTab::ALL.iter().filter(|t| **t != ProviderTab::All) {
        let group: Vec<&'a ModelEntry> = models
            .iter()
            .filter(|m| tab.matches_provider(&m.provider))
            .collect();
        if !group.is_empty() {
            groups.push((*tab, group));
        }
    }
    groups
}

// ---------------------------------------------------------------------------
// Model Picker Result
// ---------------------------------------------------------------------------

/// Outcome returned from user interaction with the model picker.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ModelPickerResult {
    /// User selected a model (Enter).
    Selected(ModelEntry),
    /// User closed the picker without selection (Esc / Ctrl+C / q).
    Cancelled,
}

// ---------------------------------------------------------------------------
// Interactive ModelPicker Widget
// ---------------------------------------------------------------------------

/// Interactive Tabbed Model Selector widget matching the fx.sh UX.
#[derive(Debug, Clone)]
pub struct ModelPicker {
    /// Complete catalog of all registered models.
    models: Vec<ModelEntry>,
    /// Currently active provider tab.
    active_tab: ProviderTab,
    /// Selection cursor within filtered model list.
    selected_index: usize,
    /// Scroll offset for vertical list pagination.
    scroll_offset: usize,
    /// Search / filter query string typed by user.
    filter_query: String,
    /// Whether to render outer rounded border block.
    show_border: bool,
    /// Custom title displayed in the card header.
    title: String,
}

impl Default for ModelPicker {
    fn default() -> Self {
        Self::new()
    }
}

impl ModelPicker {
    /// Create a new ModelPicker populated with curated flagship defaults.
    pub fn new() -> Self {
        Self {
            models: default_models(),
            active_tab: ProviderTab::All,
            selected_index: 0,
            scroll_offset: 0,
            filter_query: String::new(),
            show_border: true,
            title: "Model Selector".to_string(),
        }
    }

    /// Create a ModelPicker with a custom model catalog.
    pub fn with_models(models: Vec<ModelEntry>) -> Self {
        Self {
            models,
            active_tab: ProviderTab::All,
            selected_index: 0,
            scroll_offset: 0,
            filter_query: String::new(),
            show_border: true,
            title: "Model Selector".to_string(),
        }
    }

    /// Create a ModelPicker using the provider catalog (checking disk cache and static catalog).
    pub fn from_catalog() -> Self {
        let catalog = crate::provider::catalog::get_catalog();
        let models: Vec<ModelEntry> = catalog.models.into_iter().map(ModelEntry::from).collect();
        if models.is_empty() {
            Self::new()
        } else {
            Self::with_models(models)
        }
    }

    /// Set initial active provider tab.
    pub fn with_active_tab(mut self, tab: ProviderTab) -> Self {
        self.set_tab(tab);
        self
    }

    /// Pre-select a specific model by ID if present.
    pub fn with_selected_id(mut self, id: &str) -> Self {
        let filtered = self.filtered_indices();
        if let Some(pos) = filtered.iter().position(|&idx| self.models[idx].id == id) {
            self.selected_index = pos;
        }
        self
    }

    /// Set whether outer borders are drawn.
    pub fn with_border(mut self, show_border: bool) -> Self {
        self.show_border = show_border;
        self
    }

    /// Set custom title.
    pub fn with_title(mut self, title: impl Into<String>) -> Self {
        self.title = title.into();
        self
    }

    /// Returns currently active provider tab.
    pub fn active_tab(&self) -> ProviderTab {
        self.active_tab
    }

    /// Set active provider tab and reset selection/scrolling safely.
    pub fn set_tab(&mut self, tab: ProviderTab) {
        self.active_tab = tab;
        self.selected_index = 0;
        self.scroll_offset = 0;
    }

    /// Cycle forward to next provider tab (`Tab`).
    pub fn next_tab(&mut self) {
        self.set_tab(self.active_tab.next());
    }

    /// Cycle backward to previous provider tab (`BackTab` / `Shift+Tab`).
    pub fn prev_tab(&mut self) {
        self.set_tab(self.active_tab.prev());
    }

    /// Returns reference to all models.
    pub fn all_models(&self) -> &[ModelEntry] {
        &self.models
    }

    /// Set new list of models.
    pub fn set_models(&mut self, models: Vec<ModelEntry>) {
        self.models = models;
        self.clamp_selection();
    }

    /// Append a single model to catalog.
    pub fn add_model(&mut self, model: ModelEntry) {
        self.models.push(model);
    }

    /// Current filter query text.
    pub fn filter_query(&self) -> &str {
        &self.filter_query
    }

    /// Set search filter query.
    pub fn set_filter_query(&mut self, query: impl Into<String>) {
        self.filter_query = query.into();
        self.selected_index = 0;
        self.scroll_offset = 0;
    }

    /// Clear search filter query.
    pub fn clear_filter(&mut self) {
        self.filter_query.clear();
        self.selected_index = 0;
        self.scroll_offset = 0;
    }

    /// Indices of models matching current active tab and search filter query.
    fn filtered_indices(&self) -> Vec<usize> {
        let query = self.filter_query.trim().to_lowercase();
        self.models
            .iter()
            .enumerate()
            .filter(|(_, m)| {
                // Tab filter
                if !self.active_tab.matches_provider(&m.provider) {
                    return false;
                }
                // Text search filter
                if !query.is_empty() {
                    let id_match = m.id.to_lowercase().contains(&query);
                    let name_match = m.name.to_lowercase().contains(&query);
                    let provider_match = m.provider.to_lowercase().contains(&query);
                    let badge_match = m.badges.iter().any(|b| b.to_lowercase().contains(&query));
                    let speed_match = m
                        .speed
                        .as_deref()
                        .map(|s| s.to_lowercase().contains(&query))
                        .unwrap_or(false);
                    let desc_match = m
                        .description
                        .as_deref()
                        .map(|d| d.to_lowercase().contains(&query))
                        .unwrap_or(false);
                    let price_match = m
                        .pricing_display
                        .as_deref()
                        .map(|p| p.to_lowercase().contains(&query))
                        .unwrap_or(false);

                    if !id_match
                        && !name_match
                        && !provider_match
                        && !badge_match
                        && !speed_match
                        && !desc_match
                        && !price_match
                    {
                        return false;
                    }
                }
                true
            })
            .map(|(idx, _)| idx)
            .collect()
    }

    /// Returns references to all models matching current tab and filter query.
    pub fn filtered_models(&self) -> Vec<&ModelEntry> {
        self.filtered_indices()
            .into_iter()
            .map(|idx| &self.models[idx])
            .collect()
    }

    /// Count of models matching current filters.
    pub fn filtered_count(&self) -> usize {
        self.filtered_indices().len()
    }

    /// Returns count of models belonging to a specific provider.
    pub fn provider_count(&self, tab: ProviderTab) -> usize {
        self.models
            .iter()
            .filter(|m| tab.matches_provider(&m.provider))
            .count()
    }

    /// Returns filtered models grouped by provider tab.
    pub fn grouped_filtered_models(&self) -> Vec<(ProviderTab, Vec<&ModelEntry>)> {
        let filtered = self.filtered_models();
        let mut groups = Vec::new();
        for tab in ProviderTab::ALL.iter().filter(|t| **t != ProviderTab::All) {
            let group: Vec<&ModelEntry> = filtered
                .iter()
                .copied()
                .filter(|m| tab.matches_provider(&m.provider))
                .collect();
            if !group.is_empty() {
                groups.push((*tab, group));
            }
        }
        groups
    }

    /// Returns currently selected model if available.
    pub fn selected_model(&self) -> Option<&ModelEntry> {
        let indices = self.filtered_indices();
        indices
            .get(self.selected_index)
            .map(|&idx| &self.models[idx])
    }

    /// Current selection index within filtered list.
    pub fn selected_index(&self) -> usize {
        self.selected_index
    }

    /// Move selection down (`↓` or `j` or `Ctrl+N`).
    pub fn select_next(&mut self) {
        let count = self.filtered_count();
        if count == 0 {
            self.selected_index = 0;
            return;
        }
        if self.selected_index + 1 < count {
            self.selected_index += 1;
        } else {
            // Wrap to top
            self.selected_index = 0;
        }
    }

    /// Move selection up (`↑` or `k` or `Ctrl+P`).
    pub fn select_prev(&mut self) {
        let count = self.filtered_count();
        if count == 0 {
            self.selected_index = 0;
            return;
        }
        if self.selected_index > 0 {
            self.selected_index -= 1;
        } else {
            // Wrap to bottom
            self.selected_index = count - 1;
        }
    }

    /// Move selection forward by a page height.
    pub fn select_page_down(&mut self, page_size: usize) {
        let count = self.filtered_count();
        if count == 0 {
            return;
        }
        self.selected_index = (self.selected_index + page_size).min(count - 1);
    }

    /// Move selection backward by a page height.
    pub fn select_page_up(&mut self, page_size: usize) {
        self.selected_index = self.selected_index.saturating_sub(page_size);
    }

    /// Jump selection to top item.
    pub fn select_first(&mut self) {
        self.selected_index = 0;
    }

    /// Jump selection to bottom item.
    pub fn select_last(&mut self) {
        let count = self.filtered_count();
        if count > 0 {
            self.selected_index = count - 1;
        }
    }

    /// Ensure selection index and scroll offset stay within valid bounds.
    fn clamp_selection(&mut self) {
        let count = self.filtered_count();
        if count == 0 {
            self.selected_index = 0;
            self.scroll_offset = 0;
        } else if self.selected_index >= count {
            self.selected_index = count - 1;
        }
    }

    /// Process a keyboard event. Returns `Some(result)` on terminal actions (Enter/Esc),
    /// or `None` if the event modified picker state.
    pub fn handle_key(
        &mut self,
        code: KeyCode,
        modifiers: KeyModifiers,
    ) -> Option<ModelPickerResult> {
        match (code, modifiers) {
            // Enter: Select and Use
            (KeyCode::Enter, _) => {
                if let Some(selected) = self.selected_model().cloned() {
                    return Some(ModelPickerResult::Selected(selected));
                }
                Some(ModelPickerResult::Cancelled)
            }

            // Esc: Clear Filter if active, else Close
            (KeyCode::Esc, _) => {
                if !self.filter_query.is_empty() {
                    self.clear_filter();
                    None
                } else {
                    Some(ModelPickerResult::Cancelled)
                }
            }

            // Ctrl+C: Cancel
            (KeyCode::Char('c'), KeyModifiers::CONTROL) => Some(ModelPickerResult::Cancelled),

            // Tab: Cycle provider forward
            (KeyCode::Tab, KeyModifiers::NONE) => {
                self.next_tab();
                None
            }

            // Shift+Tab / BackTab: Cycle provider backward
            (KeyCode::BackTab, _) | (KeyCode::Tab, KeyModifiers::SHIFT) => {
                self.prev_tab();
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

            // Left arrow: Previous provider tab when search is empty
            (KeyCode::Left, KeyModifiers::NONE) if self.filter_query.is_empty() => {
                self.prev_tab();
                None
            }

            // Right arrow: Next provider tab when search is empty
            (KeyCode::Right, KeyModifiers::NONE) if self.filter_query.is_empty() => {
                self.next_tab();
                None
            }

            // Numeric provider jump keys: 1-8 when search is empty
            (KeyCode::Char(c @ '1'..='8'), KeyModifiers::NONE) if self.filter_query.is_empty() => {
                let idx = (c as usize).saturating_sub('1' as usize);
                if let Some(tab) = ProviderTab::from_index(idx) {
                    self.set_tab(tab);
                }
                None
            }

            // PageUp / PageDown
            (KeyCode::PageUp, _) => {
                self.select_page_up(6);
                None
            }
            (KeyCode::PageDown, _) => {
                self.select_page_down(6);
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

            // Backspace: Delete filter character
            (KeyCode::Backspace, _) => {
                if !self.filter_query.is_empty() {
                    self.filter_query.pop();
                    self.clamp_selection();
                }
                None
            }

            // Ctrl+U: Clear entire filter query
            (KeyCode::Char('u'), KeyModifiers::CONTROL) => {
                if !self.filter_query.is_empty() {
                    self.clear_filter();
                }
                None
            }

            // Ctrl+W: Delete last word in filter query
            (KeyCode::Char('w'), KeyModifiers::CONTROL) => {
                if !self.filter_query.is_empty() {
                    let trimmed = self.filter_query.trim_end();
                    if let Some(last_space) = trimmed.rfind(' ') {
                        self.filter_query.truncate(last_space);
                    } else {
                        self.filter_query.clear();
                    }
                    self.clamp_selection();
                }
                None
            }

            // 'q' to close when not in active search
            (KeyCode::Char('q'), KeyModifiers::NONE) if self.filter_query.is_empty() => {
                Some(ModelPickerResult::Cancelled)
            }

            // Vim navigation keys when filter is empty
            (KeyCode::Char('k'), KeyModifiers::NONE) if self.filter_query.is_empty() => {
                self.select_prev();
                None
            }
            (KeyCode::Char('j'), KeyModifiers::NONE) if self.filter_query.is_empty() => {
                self.select_next();
                None
            }

            // Slash '/' to start search or clear filter
            (KeyCode::Char('/'), KeyModifiers::NONE) if self.filter_query.is_empty() => {
                self.filter_query.clear();
                None
            }

            // Character typing: Filter query
            (KeyCode::Char(c), KeyModifiers::NONE) | (KeyCode::Char(c), KeyModifiers::SHIFT) => {
                self.filter_query.push(c);
                self.selected_index = 0;
                self.scroll_offset = 0;
                None
            }

            _ => None,
        }
    }

    /// Render widget onto a Ratatui Frame within specified area.
    pub fn render_frame(&mut self, f: &mut Frame, area: Rect) {
        f.render_widget(&*self, area);
    }

    /// Internal rendering to a Ratatui Buffer.
    pub fn render_buffer(&self, area: Rect, buf: &mut Buffer) {
        if area.width < 10 || area.height < 3 {
            return;
        }

        // Determine layout based on available height and show_border option
        let (inner_area, _use_border) = if self.show_border && area.height >= 7 && area.width >= 30
        {
            let block = Block::default()
                .title(format!(" ✦ {} ", self.title))
                .borders(Borders::ALL)
                .border_type(BorderType::Rounded)
                .border_style(Style::default().fg(Color::Cyan));
            let inner = block.inner(area);
            block.render(area, buf);
            (inner, true)
        } else {
            (area, false)
        };

        if inner_area.height < 3 {
            return;
        }

        let show_info_line = inner_area.height >= 9;
        let has_dividers = inner_area.height >= 7;

        let constraints = if show_info_line {
            vec![
                Constraint::Length(1), // Tabs
                Constraint::Length(1), // Top Divider
                Constraint::Min(2),    // Models list
                Constraint::Length(1), // Bottom Divider
                Constraint::Length(1), // Selected Model Info Line
                Constraint::Length(1), // Key hints footer
            ]
        } else if has_dividers {
            vec![
                Constraint::Length(1), // Tabs
                Constraint::Length(1), // Top Divider
                Constraint::Min(2),    // Models list
                Constraint::Length(1), // Bottom Divider
                Constraint::Length(1), // Key hints footer
            ]
        } else {
            vec![
                Constraint::Length(1), // Tabs
                Constraint::Min(1),    // Models list
                Constraint::Length(1), // Key hints footer
            ]
        };

        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints(constraints)
            .split(inner_area);

        let tabs_area = chunks[0];

        if show_info_line {
            let div1_area = chunks[1];
            let list_area = chunks[2];
            let div2_area = chunks[3];
            let info_area = chunks[4];
            let foot_area = chunks[5];

            // Render thin horizontal dividers
            render_horizontal_divider(buf, div1_area, Color::DarkGray);
            render_horizontal_divider(buf, div2_area, Color::DarkGray);

            // 1. Render Provider Tabs
            self.render_tabs(tabs_area, buf);
            // 2. Render Models List
            self.render_models(list_area, buf);
            // 3. Render Selected Model Info Summary
            self.render_model_info(info_area, buf);
            // 4. Render Key Hints Footer
            self.render_footer(foot_area, buf);
        } else if has_dividers {
            let div1_area = chunks[1];
            let list_area = chunks[2];
            let div2_area = chunks[3];
            let foot_area = chunks[4];

            render_horizontal_divider(buf, div1_area, Color::DarkGray);
            render_horizontal_divider(buf, div2_area, Color::DarkGray);

            self.render_tabs(tabs_area, buf);
            self.render_models(list_area, buf);
            self.render_footer(foot_area, buf);
        } else {
            let list_area = chunks[1];
            let foot_area = chunks[2];

            self.render_tabs(tabs_area, buf);
            self.render_models(list_area, buf);
            self.render_footer(foot_area, buf);
        }
    }

    /// Render top provider tabs bar: `[All] Anthropic OpenAI DeepSeek xAI Ollama OpenRouter`.
    fn render_tabs(&self, area: Rect, buf: &mut Buffer) {
        let is_narrow = area.width < 56;
        let mut spans = Vec::new();
        spans.push(Span::raw(" "));

        for (idx, tab) in ProviderTab::ALL.iter().enumerate() {
            let is_active = *tab == self.active_tab;
            let label = if is_narrow {
                tab.short_name()
            } else {
                tab.name()
            };

            if is_active {
                // Active tab matching fx.sh UX: [TabName] in Bold Cyan
                spans.push(Span::styled(
                    format!("[{}]", label),
                    Style::default()
                        .fg(Color::Cyan)
                        .add_modifier(Modifier::BOLD),
                ));
            } else {
                // Inactive tabs
                spans.push(Span::styled(
                    label.to_string(),
                    Style::default().fg(Color::DarkGray),
                ));
            }

            if idx + 1 < ProviderTab::ALL.len() {
                spans.push(Span::raw("  "));
            }
        }

        // Show search filter status on right side if active
        if !self.filter_query.is_empty() {
            spans.push(Span::raw("  "));
            spans.push(Span::styled(
                format!("(/\"{}\")", self.filter_query),
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            ));
        }

        let paragraph = Paragraph::new(Line::from(spans));
        paragraph.render(area, buf);
    }

    /// Render visible model rows.
    fn render_models(&self, area: Rect, buf: &mut Buffer) {
        let filtered = self.filtered_models();
        let visible_rows = area.height as usize;

        if filtered.is_empty() {
            let msg = if !self.filter_query.is_empty() {
                format!("  No models matching '{}'", self.filter_query)
            } else {
                format!("  No models available for tab [{}]", self.active_tab.name())
            };
            let paragraph = Paragraph::new(Line::from(vec![Span::styled(
                msg,
                Style::default().fg(Color::DarkGray),
            )]));
            paragraph.render(area, buf);
            return;
        }

        // Compute active scroll window
        let mut scroll = self.scroll_offset;
        if self.selected_index < scroll {
            scroll = self.selected_index;
        } else if self.selected_index >= scroll + visible_rows {
            scroll = self.selected_index - visible_rows + 1;
        }

        let end = (scroll + visible_rows).min(filtered.len());
        let visible_slice = &filtered[scroll..end];

        let width = area.width;
        let mut lines = Vec::new();

        for (rel_idx, model) in visible_slice.iter().enumerate() {
            let actual_idx = scroll + rel_idx;
            let is_selected = actual_idx == self.selected_index;
            let line = format_model_row(model, is_selected, width);
            lines.push(line);
        }

        let paragraph = Paragraph::new(lines);
        paragraph.render(area, buf);
    }

    /// Render selected model information line when vertical space permits.
    fn render_model_info(&self, area: Rect, buf: &mut Buffer) {
        if let Some(model) = self.selected_model() {
            let mut spans = Vec::new();
            spans.push(Span::raw(" "));
            spans.push(Span::styled(
                "ℹ ",
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            ));

            if let Some(desc) = &model.description {
                let max_desc_len = (area.width as usize).saturating_sub(6);
                let truncated_desc = truncate_pad(desc, max_desc_len);
                spans.push(Span::styled(
                    truncated_desc,
                    Style::default().fg(Color::Gray),
                ));
            } else {
                let pricing = model.formatted_pricing();
                let speed = model.formatted_speed();
                let info_text = format!(
                    "{} • {} • {} • ⚡ {}",
                    model.id, model.context_display, pricing, speed
                );
                let max_len = (area.width as usize).saturating_sub(6);
                spans.push(Span::styled(
                    truncate_pad(&info_text, max_len),
                    Style::default().fg(Color::Gray),
                ));
            }

            let paragraph = Paragraph::new(Line::from(spans));
            paragraph.render(area, buf);
        }
    }

    /// Render footer matching prompt requirements:
    /// `↑↓ Navigate  Tab Provider  1-7 Jump  Enter Use  Esc Close`
    fn render_footer(&self, area: Rect, buf: &mut Buffer) {
        let width = area.width;
        let is_compact = width < 56;

        let footer_line = if is_compact {
            Line::from(vec![
                Span::styled(
                    "↑↓ ",
                    Style::default()
                        .fg(Color::Cyan)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled("Nav ", Style::default().fg(Color::DarkGray)),
                Span::styled(
                    "Tab ",
                    Style::default()
                        .fg(Color::Cyan)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled("Prov ", Style::default().fg(Color::DarkGray)),
                Span::styled(
                    "↵ ",
                    Style::default()
                        .fg(Color::Cyan)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled("Use ", Style::default().fg(Color::DarkGray)),
                Span::styled(
                    "Esc ",
                    Style::default()
                        .fg(Color::Cyan)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled("Quit", Style::default().fg(Color::DarkGray)),
            ])
        } else {
            Line::from(vec![
                Span::raw(" "),
                Span::styled(
                    "↑↓ ",
                    Style::default()
                        .fg(Color::Cyan)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled("Navigate", Style::default().fg(Color::DarkGray)),
                Span::raw("  "),
                Span::styled(
                    "Tab ",
                    Style::default()
                        .fg(Color::Cyan)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled("Provider", Style::default().fg(Color::DarkGray)),
                Span::raw("  "),
                Span::styled(
                    "1-8 ",
                    Style::default()
                        .fg(Color::Cyan)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled("Jump", Style::default().fg(Color::DarkGray)),
                Span::raw("  "),
                Span::styled(
                    "Enter ",
                    Style::default()
                        .fg(Color::Cyan)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled("Use", Style::default().fg(Color::DarkGray)),
                Span::raw("  "),
                Span::styled(
                    "Esc ",
                    Style::default()
                        .fg(Color::Cyan)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled("Close", Style::default().fg(Color::DarkGray)),
            ])
        };

        let paragraph = Paragraph::new(footer_line);
        paragraph.render(area, buf);
    }

    /// Launch interactive TUI loop inside an inline Ratatui viewport.
    ///
    /// Clamps height safely for Termux and narrow mobile viewports.
    /// Restores terminal mode and cursor on exit or cancel.
    pub fn run_interactive(
        &mut self,
        requested_height: Option<u16>,
    ) -> std::io::Result<Option<ModelEntry>> {
        let _raw_guard = RawModeGuard::enter()?;
        let _ = execute!(stdout(), cursor::Hide);

        let (_cols, rows) = InlineTerminal::terminal_size();
        let height = requested_height.unwrap_or_else(|| {
            if rows <= 10 {
                rows.saturating_sub(1).max(3)
            } else if rows <= 18 {
                8
            } else {
                12
            }
        });

        let mut inline = InlineTerminal::new(height)?;

        let outcome = loop {
            // Draw current frame
            inline.draw(|f| {
                let area = f.area();
                f.render_widget(&*self, area);
            })?;

            // Poll for user keyboard input
            if event::poll(std::time::Duration::from_millis(50))? {
                if let Event::Key(key) = event::read()? {
                    if key.kind == KeyEventKind::Release {
                        continue;
                    }

                    if let Some(result) = self.handle_key(key.code, key.modifiers) {
                        break match result {
                            ModelPickerResult::Selected(m) => Some(m),
                            ModelPickerResult::Cancelled => None,
                        };
                    }
                }
            }
        };

        // Clean up inline viewport and restore cursor
        let _ = inline.clear();
        let _ = inline.finish();
        let _ = execute!(stdout(), cursor::Show);
        let _ = stdout().flush();

        Ok(outcome)
    }
}

// ---------------------------------------------------------------------------
// Ratatui Widget Implementations
// ---------------------------------------------------------------------------

impl Widget for &ModelPicker {
    fn render(self, area: Rect, buf: &mut Buffer) {
        self.render_buffer(area, buf);
    }
}

impl Widget for ModelPicker {
    fn render(self, area: Rect, buf: &mut Buffer) {
        self.render_buffer(area, buf);
    }
}

// ---------------------------------------------------------------------------
// Row Formatting Helper
// ---------------------------------------------------------------------------

/// Format a single model row with:
/// - Cursor indicator (`❯ ` or `  `)
/// - Model ID
/// - Context window (e.g. `200K ctx`, `1M context`)
/// - Pricing (e.g. `$3/$15`, `Free`, `$0.14/$0.28`)
/// - Speed (e.g. `⚡ Fast`, `⚡ Reas`)
/// - Capability Badges (`[Fast]`, `[Reasoning]`, `[Vision]`)
pub fn format_model_row<'a>(model: &'a ModelEntry, is_selected: bool, width: u16) -> Line<'a> {
    let mut spans = Vec::new();

    // 1. Cursor prefix
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

    // Dynamic column width allocation based on screen width
    if width >= 95 {
        // Wide display (95+ cols): ID, Context, Output, Pricing, Speed, Badges
        let id_width = 28;
        let ctx_width = 13;
        let out_width = 11;
        let price_width = 11;
        let speed_width = 9;

        // Model ID
        let truncated_id = truncate_pad(&model.id, id_width);
        let id_style = if is_selected {
            Style::default()
                .fg(Color::White)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(Color::White)
        };
        spans.push(Span::styled(truncated_id, id_style));
        spans.push(Span::raw(" "));

        // Context Window
        let truncated_ctx = truncate_pad(&model.context_display, ctx_width);
        let ctx_style = if is_selected {
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(Color::Cyan)
        };
        spans.push(Span::styled(truncated_ctx, ctx_style));
        spans.push(Span::raw(" "));

        // Output tokens
        let truncated_out = truncate_pad(&model.output_display, out_width);
        let out_style = if is_selected {
            Style::default()
                .fg(Color::Magenta)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(Color::DarkGray)
        };
        spans.push(Span::styled(truncated_out, out_style));
        spans.push(Span::raw(" "));

        // Pricing
        let pricing = model.formatted_pricing();
        let truncated_price = truncate_pad(&pricing, price_width);
        let price_style = if pricing == "Free" {
            Style::default().fg(Color::Green)
        } else if is_selected {
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(Color::Yellow)
        };
        spans.push(Span::styled(truncated_price, price_style));
        spans.push(Span::raw(" "));

        // Speed
        let speed = format!("⚡ {}", model.formatted_speed());
        let truncated_speed = truncate_pad(&speed, speed_width);
        let speed_style = if is_selected {
            Style::default()
                .fg(Color::Green)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(Color::Green)
        };
        spans.push(Span::styled(truncated_speed, speed_style));
        spans.push(Span::raw(" "));

        // Badges
        for badge in &model.badges {
            let (fg_color, is_bold) = badge_style(badge);
            let mut style = Style::default().fg(fg_color);
            if is_bold || is_selected {
                style = style.add_modifier(Modifier::BOLD);
            }
            spans.push(Span::styled(format!("[{}] ", badge), style));
        }
    } else if width >= 70 {
        // Medium display (70..94 cols): ID, Context, Pricing, Badges
        let id_width = 24;
        let ctx_width = 11;
        let price_width = 10;

        let truncated_id = truncate_pad(&model.id, id_width);
        let id_style = if is_selected {
            Style::default()
                .fg(Color::White)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(Color::White)
        };
        spans.push(Span::styled(truncated_id, id_style));
        spans.push(Span::raw(" "));

        let ctx_display = model.context_display.replace(" context", " ctx");
        let truncated_ctx = truncate_pad(&ctx_display, ctx_width);
        let ctx_style = if is_selected {
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(Color::Cyan)
        };
        spans.push(Span::styled(truncated_ctx, ctx_style));
        spans.push(Span::raw(" "));

        let pricing = model.formatted_pricing();
        let truncated_price = truncate_pad(&pricing, price_width);
        let price_style = if pricing == "Free" {
            Style::default().fg(Color::Green)
        } else if is_selected {
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(Color::Yellow)
        };
        spans.push(Span::styled(truncated_price, price_style));
        spans.push(Span::raw(" "));

        for badge in &model.badges {
            let (fg_color, is_bold) = badge_style(badge);
            let mut style = Style::default().fg(fg_color);
            if is_bold || is_selected {
                style = style.add_modifier(Modifier::BOLD);
            }
            spans.push(Span::styled(format!("[{}] ", badge), style));
        }
    } else {
        // Narrow display (< 70 cols, Termux / mobile): Compact ID, Context, Badges
        let id_width = if width < 50 { 18 } else { 22 };
        let ctx_width = if width < 50 { 8 } else { 10 };

        let truncated_id = truncate_pad(&model.id, id_width);
        let id_style = if is_selected {
            Style::default()
                .fg(Color::White)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(Color::White)
        };
        spans.push(Span::styled(truncated_id, id_style));
        spans.push(Span::raw(" "));

        let ctx_display = model.context_display.replace(" context", " ctx");
        let truncated_ctx = truncate_pad(&ctx_display, ctx_width);
        let ctx_style = if is_selected {
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(Color::Cyan)
        };
        spans.push(Span::styled(truncated_ctx, ctx_style));
        spans.push(Span::raw(" "));

        for badge in &model.badges {
            let (fg_color, is_bold) = badge_style(badge);
            let mut style = Style::default().fg(fg_color);
            if is_bold || is_selected {
                style = style.add_modifier(Modifier::BOLD);
            }
            spans.push(Span::styled(format!("[{}] ", badge), style));
        }
    }

    Line::from(spans)
}

/// Helper to render a thin horizontal line across a Rect area.
fn render_horizontal_divider(buf: &mut Buffer, area: Rect, color: Color) {
    if area.height == 0 || area.width == 0 {
        return;
    }
    let divider_char = "─";
    for x in area.left()..area.right() {
        if let Some(cell) = buf.cell_mut((x, area.top())) {
            cell.set_symbol(divider_char);
            cell.set_fg(color);
        }
    }
}

/// Pad or truncate string to exact visible character width.
fn truncate_pad(s: &str, width: usize) -> String {
    let char_count = s.chars().count();
    if char_count == width {
        s.to_string()
    } else if char_count < width {
        let padding = " ".repeat(width - char_count);
        format!("{}{}", s, padding)
    } else if width > 2 {
        let mut result: String = s.chars().take(width - 1).collect();
        result.push('…');
        result
    } else {
        s.chars().take(width).collect()
    }
}

/// Map badge name to color and bold modifier.
fn badge_style(badge: &str) -> (Color, bool) {
    let lower = badge.to_lowercase();
    if lower.contains("fast") || lower.contains("instant") {
        (Color::Green, true)
    } else if lower.contains("reason") || lower.contains("r1") || lower.contains("cot") {
        (Color::Magenta, true)
    } else if lower.contains("vision") || lower.contains("omni") || lower.contains("multimodal") {
        (Color::Cyan, false)
    } else if lower.contains("code") || lower.contains("coding") {
        (Color::Yellow, true)
    } else if lower.contains("local") {
        (Color::Blue, false)
    } else if lower.contains("1m") || lower.contains("200k") {
        (Color::Cyan, true)
    } else if lower.contains("opensource") || lower.contains("openrouter") {
        (Color::LightCyan, false)
    } else if lower.contains("cost-efficient") || lower.contains("free") {
        (Color::Green, false)
    } else {
        (Color::LightCyan, false)
    }
}

// ---------------------------------------------------------------------------
// Curated Default Catalog
// ---------------------------------------------------------------------------

/// Curated flagship and popular models matching fx.sh catalog across all 6 providers.
pub fn default_models() -> Vec<ModelEntry> {
    vec![
        // Fusion
        ModelEntry::with_tokens(
            "deepseek-ai/DeepSeek-V4-Flash-0731",
            "fusion",
            1_048_576,
            8_192,
            vec!["Fast", "Default", "1M context"],
        )
        .with_speed("Ultra-Fast")
        .with_description("Fusion gateway high-speed 1M context flash model"),
        ModelEntry::with_tokens(
            "MiniMaxAI/MiniMax-M2.7",
            "fusion",
            204_800,
            8_192,
            vec!["Reasoning", "200K context"],
        )
        .with_speed("Fast")
        .with_description("MiniMax M2.7 frontier coding and reasoning model"),
    ]
}

/// Convenience helper to pick a model interactively.
/// Checks provider catalog (disk cache + static catalog), falling back to curated defaults.
pub fn pick_model() -> std::io::Result<Option<ModelEntry>> {
    let mut picker = ModelPicker::from_catalog();
    picker.run_interactive(None)
}

// ---------------------------------------------------------------------------
// Unit Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;

    #[test]
    fn test_provider_tab_cycling() {
        let tab = ProviderTab::All;
        assert_eq!(tab.name(), "All");
        assert_eq!(tab.index(), 0);
        assert_eq!(tab.next(), ProviderTab::Fusion);
        assert_eq!(tab.next().next(), ProviderTab::All);

        assert_eq!(ProviderTab::All.prev(), ProviderTab::Fusion);
        assert_eq!(ProviderTab::Fusion.prev(), ProviderTab::All);
    }

    #[test]
    fn test_provider_tab_matching() {
        assert!(ProviderTab::All.matches_provider("fusion"));
        assert!(ProviderTab::All.matches_provider("anything"));
        assert!(ProviderTab::Fusion.matches_provider("fusion"));
        assert!(!ProviderTab::Fusion.matches_provider("other"));
    }

    #[test]
    fn test_provider_tab_from_index() {
        assert_eq!(ProviderTab::from_index(0), Some(ProviderTab::All));
        assert_eq!(ProviderTab::from_index(1), Some(ProviderTab::Fusion));
        assert_eq!(ProviderTab::from_index(2), None);
    }

    #[test]
    fn test_token_formatting_helpers() {
        assert_eq!(format_tokens(1_000_000), "1M");
        assert_eq!(format_tokens(200_000), "200K");
        assert_eq!(format_tokens(128_000), "128K");
        assert_eq!(format_tokens(16_384), "16K");
        assert_eq!(format_tokens(8_192), "8K");
        assert_eq!(format_tokens(4_096), "4K");
        assert_eq!(format_tokens(500), "500");

        assert_eq!(format_context(1_000_000), "1M context");
        assert_eq!(format_context(200_000), "200K context");
        assert_eq!(format_output(128_000), "128K output");
        assert_eq!(format_output(8_192), "8K output");
    }

    #[test]
    fn test_pricing_formatting_helpers() {
        assert_eq!(format_pricing_pair(0.0, 0.0), "Free");
        assert_eq!(format_pricing_pair(3.0, 15.0), "$3/$15");
        assert_eq!(format_pricing_pair(0.14, 0.28), "$0.14/$0.28");
        assert_eq!(format_pricing_pair(2.50, 10.0), "$2.50/$10");
        assert_eq!(format_pricing_pair(0.15, 0.60), "$0.15/$0.60");
    }

    #[test]
    fn test_model_entry_builders_and_getters() {
        let entry = ModelEntry::with_tokens(
            "test-model",
            "anthropic",
            200_000,
            8_192,
            vec!["Fast", "Vision"],
        )
        .with_name("Test Friendly")
        .with_description("A testing model")
        .with_pricing(3.0, 15.0)
        .with_speed("Very Fast");

        assert_eq!(entry.name, "Test Friendly");
        assert_eq!(entry.formatted_pricing(), "$3/$15");
        assert_eq!(entry.formatted_speed(), "Very Fast");
        assert_eq!(entry.context_display, "200K context");
        assert_eq!(entry.output_display, "8K output");
        assert_eq!(entry.description.as_deref(), Some("A testing model"));

        let free_entry = ModelEntry::new(
            "local-mod",
            "ollama",
            "32K context",
            "8K output",
            vec!["Local"],
        )
        .with_free_pricing();
        assert_eq!(free_entry.formatted_pricing(), "Free");
        assert_eq!(free_entry.formatted_speed(), "Local");
    }

    #[test]
    fn test_default_models_catalog() {
        let models = default_models();
        assert!(!models.is_empty());
        assert!(models.iter().any(|m| m.id == "MiniMaxAI/MiniMax-M2.7"));
        assert!(models
            .iter()
            .any(|m| m.id == "deepseek-ai/DeepSeek-V4-Flash-0731"));
        for m in &models {
            assert_eq!(m.provider, "fusion");
        }
    }

    #[test]
    fn test_picker_tab_filtering() {
        let mut picker = ModelPicker::new();
        assert_eq!(picker.active_tab(), ProviderTab::All);
        let all_count = picker.filtered_count();
        assert!(all_count >= 2);

        picker.set_tab(ProviderTab::Fusion);
        assert_eq!(picker.active_tab(), ProviderTab::Fusion);
        for m in picker.filtered_models() {
            assert_eq!(m.provider, "fusion");
        }
    }

    #[test]
    fn test_picker_group_by_provider() {
        let models = default_models();
        let groups = group_by_provider(&models);
        assert!(!groups.is_empty());
        assert!(groups.iter().any(|(tab, _)| *tab == ProviderTab::Fusion));
    }

    #[test]
    fn test_picker_search_filtering() {
        let mut picker = ModelPicker::new();
        picker.set_filter_query("minimax");
        for m in picker.filtered_models() {
            assert!(
                m.id.to_lowercase().contains("minimax")
                    || m.name.to_lowercase().contains("minimax")
            );
        }

        picker.set_filter_query("reasoning");
        for m in picker.filtered_models() {
            let has_badge = m
                .badges
                .iter()
                .any(|b| b.to_lowercase().contains("reasoning"));
            let has_id = m.id.to_lowercase().contains("reasoning");
            let has_desc = m
                .description
                .as_deref()
                .map(|d| d.to_lowercase().contains("reasoning"))
                .unwrap_or(false);
            let has_speed = m
                .speed
                .as_deref()
                .map(|s| s.to_lowercase().contains("reasoning"))
                .unwrap_or(false);
            assert!(has_badge || has_id || has_desc || has_speed);
        }

        picker.set_filter_query("nonexistent_model_xyz");
        assert_eq!(picker.filtered_count(), 0);

        picker.clear_filter();
        assert!(picker.filtered_count() > 0);
    }

    #[test]
    fn test_picker_navigation() {
        let mut picker = ModelPicker::new();
        assert_eq!(picker.selected_index(), 0);

        picker.select_next();
        assert_eq!(picker.selected_index(), 1);

        picker.select_prev();
        assert_eq!(picker.selected_index(), 0);

        // Wrapping up from top goes to last
        picker.select_prev();
        assert_eq!(picker.selected_index(), picker.filtered_count() - 1);

        // Wrapping down from last goes to 0
        picker.select_next();
        assert_eq!(picker.selected_index(), 0);

        // Page navigation (clamped to max index)
        picker.select_page_down(5);
        assert_eq!(picker.selected_index(), picker.filtered_count() - 1);

        picker.select_page_up(1);
        assert_eq!(picker.selected_index(), 0);
        picker.select_last();
        assert_eq!(picker.selected_index(), picker.filtered_count() - 1);

        picker.select_first();
        assert_eq!(picker.selected_index(), 0);
    }

    #[test]
    fn test_picker_key_handling() {
        let mut picker = ModelPicker::new();

        // Down arrow
        assert_eq!(picker.handle_key(KeyCode::Down, KeyModifiers::NONE), None);
        assert_eq!(picker.selected_index(), 1);

        // Ctrl+P / Ctrl+N
        assert_eq!(
            picker.handle_key(KeyCode::Char('p'), KeyModifiers::CONTROL),
            None
        );
        assert_eq!(picker.selected_index(), 0);
        assert_eq!(
            picker.handle_key(KeyCode::Char('n'), KeyModifiers::CONTROL),
            None
        );
        assert_eq!(picker.selected_index(), 1);

        // Tab switches provider
        assert_eq!(picker.handle_key(KeyCode::Tab, KeyModifiers::NONE), None);
        assert_eq!(picker.active_tab(), ProviderTab::Fusion);

        // Enter selects model
        let res = picker.handle_key(KeyCode::Enter, KeyModifiers::NONE);
        match res {
            Some(ModelPickerResult::Selected(m)) => {
                assert_eq!(m.provider, "fusion");
            }
            _ => panic!("Expected ModelPickerResult::Selected"),
        }

        // Typing query and Backspace / Ctrl+U
        picker.handle_key(KeyCode::Char('c'), KeyModifiers::NONE);
        picker.handle_key(KeyCode::Char('h'), KeyModifiers::NONE);
        picker.handle_key(KeyCode::Char('a'), KeyModifiers::NONE);
        picker.handle_key(KeyCode::Char('t'), KeyModifiers::NONE);
        assert_eq!(picker.filter_query(), "chat");

        picker.handle_key(KeyCode::Backspace, KeyModifiers::NONE);
        assert_eq!(picker.filter_query(), "cha");

        picker.handle_key(KeyCode::Char('u'), KeyModifiers::CONTROL);
        assert_eq!(picker.filter_query(), "");

        // Esc clears search if query non-empty
        picker.handle_key(KeyCode::Char('x'), KeyModifiers::NONE);
        assert_eq!(picker.filter_query(), "x");
        let res_esc_clear = picker.handle_key(KeyCode::Esc, KeyModifiers::NONE);
        assert_eq!(res_esc_clear, None);
        assert_eq!(picker.filter_query(), "");

        // Esc cancels when filter is empty
        let res_esc = picker.handle_key(KeyCode::Esc, KeyModifiers::NONE);
        assert_eq!(res_esc, Some(ModelPickerResult::Cancelled));

        // Ctrl+C cancels
        let res_ctrl_c = picker.handle_key(KeyCode::Char('c'), KeyModifiers::CONTROL);
        assert_eq!(res_ctrl_c, Some(ModelPickerResult::Cancelled));
    }

    #[test]
    fn test_ratatui_render_buffer_standard() {
        let backend = TestBackend::new(85, 14);
        let mut terminal = Terminal::new(backend).unwrap();

        let picker = ModelPicker::new();

        terminal
            .draw(|f| {
                let area = f.area();
                f.render_widget(&picker, area);
            })
            .unwrap();

        let buffer = terminal.backend().buffer();
        let text = buffer_to_string(buffer);

        // Check that tabs are present in buffer
        assert!(text.contains("[All]"));
        assert!(text.contains("Fusion"));
        // Check footer hints
        assert!(text.contains("Navigate"));
        assert!(text.contains("Provider"));
        assert!(text.contains("Use"));
        assert!(text.contains("Close"));
    }

    #[test]
    fn test_ratatui_render_buffer_wide() {
        let backend = TestBackend::new(120, 16);
        let mut terminal = Terminal::new(backend).unwrap();

        let picker = ModelPicker::new();

        terminal
            .draw(|f| {
                let area = f.area();
                f.render_widget(&picker, area);
            })
            .unwrap();

        let buffer = terminal.backend().buffer();
        let text = buffer_to_string(buffer);

        assert!(text.contains("[All]"));
        assert!(text.contains("DeepSeek") || text.contains("MiniMax"));
    }

    #[test]
    fn test_ratatui_render_buffer_compact() {
        let backend = TestBackend::new(50, 6);
        let mut terminal = Terminal::new(backend).unwrap();

        let picker = ModelPicker::new();

        terminal
            .draw(|f| {
                let area = f.area();
                f.render_widget(&picker, area);
            })
            .unwrap();

        let buffer = terminal.backend().buffer();
        let text = buffer_to_string(buffer);
        assert!(text.contains("[All]"));
    }

    #[test]
    fn test_truncate_pad() {
        assert_eq!(truncate_pad("abc", 5), "abc  ");
        assert_eq!(truncate_pad("abcde", 5), "abcde");
        assert_eq!(truncate_pad("abcdef", 5), "abcd…");
    }

    #[test]
    fn test_badge_style() {
        let (fg, bold) = badge_style("Fast");
        assert_eq!(fg, Color::Green);
        assert!(bold);

        let (fg_r, bold_r) = badge_style("Reasoning");
        assert_eq!(fg_r, Color::Magenta);
        assert!(bold_r);

        let (fg_v, _) = badge_style("Vision");
        assert_eq!(fg_v, Color::Cyan);
    }

    #[test]
    fn test_model_entry_serde_roundtrip() {
        let entry =
            ModelEntry::with_tokens("gpt-4o", "openai", 128_000, 16_384, vec!["Fast", "Vision"])
                .with_pricing(2.50, 10.0)
                .with_speed("Fast")
                .with_description("Omni multimodal flagship");

        let json = serde_json::to_string(&entry).unwrap();
        let deserialized: ModelEntry = serde_json::from_str(&json).unwrap();
        assert_eq!(entry, deserialized);
    }

    /// Helper to convert a Ratatui test buffer into plain string for assertions.
    fn buffer_to_string(buffer: &Buffer) -> String {
        let mut res = String::new();
        for y in buffer.area.top()..buffer.area.bottom() {
            for x in buffer.area.left()..buffer.area.right() {
                if let Some(cell) = buffer.cell((x, y)) {
                    res.push_str(cell.symbol());
                }
            }
            res.push('\n');
        }
        res
    }
}
