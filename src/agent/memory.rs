//! Persistent Memory Store for Fusion Agent.
//!
//! Stores user coding preferences, project architecture facts, architecture decisions,
//! recurring conventions, and tool states across sessions in `~/.fusion/memory.json`.
//!
//! ## Overview
//!
//! Persistent memory gives Fusion long-term recall across independent sessions and restarts:
//! 1. **User Preferences**: Preferred coding styles, language idioms, test frameworks, linting rules.
//! 2. **Project Facts**: Tech stack choices, module structures, entry points, database models, ports.
//! 3. **Architecture Decisions**: Architectural decisions (ADRs), structural constraints, naming standards.
//! 4. **Tool State**: Dynamic parameter states, cached tool settings, and execution configs.
//!
//! The store automatically saves changes atomically to `~/.fusion/memory.json` and seamlessly
//! formats relevant memories into the agent's system prompt during conversation turns.

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::fs;
use std::path::{Path, PathBuf};
use uuid::Uuid;

use crate::config::Config;
use crate::tools::types::{Tool, ToolContext};

// ---------------------------------------------------------------------------
// Memory Category
// ---------------------------------------------------------------------------

/// Categories of persistent memory entries.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MemoryCategory {
    /// User coding preferences, styling choices, preferred idioms, editor settings, and tool habits.
    #[serde(
        alias = "preference",
        alias = "pref",
        alias = "user_preference",
        alias = "style",
        alias = "user_pref",
        alias = "user_preferences"
    )]
    UserPreference,

    /// Factual knowledge about the project, tech stack, environment, database models, ports, dependencies.
    #[serde(
        alias = "project_fact",
        alias = "fact",
        alias = "general_fact",
        alias = "info",
        alias = "knowledge",
        alias = "project_info",
        alias = "facts"
    )]
    ProjectFact,

    /// Architectural and design decisions, ADRs, invariant patterns, structural rules, and conventions.
    #[serde(
        alias = "architecture_decision",
        alias = "architecture",
        alias = "arch",
        alias = "decision",
        alias = "adr",
        alias = "project_architecture",
        alias = "convention",
        alias = "conventions",
        alias = "rule",
        alias = "rules",
        alias = "standard",
        alias = "standards"
    )]
    ArchitectureDecision,

    /// State, cached configs, credentials metadata, execution modes, or dynamic parameters of tools.
    #[serde(
        alias = "tool_state",
        alias = "tool",
        alias = "state",
        alias = "tool_config",
        alias = "tools",
        alias = "tool_status"
    )]
    ToolState,

    /// Custom or domain-specific user memory category.
    #[serde(untagged)]
    Custom(String),
}

impl MemoryCategory {
    /// Returns the canonical snake_case string identifier for this category.
    pub fn as_str(&self) -> &str {
        match self {
            Self::UserPreference => "user_preference",
            Self::ProjectFact => "project_fact",
            Self::ArchitectureDecision => "architecture_decision",
            Self::ToolState => "tool_state",
            Self::Custom(c) => c.as_str(),
        }
    }

    /// Returns a human-friendly display name.
    pub fn display_name(&self) -> &str {
        match self {
            Self::UserPreference => "User Preference",
            Self::ProjectFact => "Project Fact",
            Self::ArchitectureDecision => "Architecture Decision",
            Self::ToolState => "Tool State",
            Self::Custom(c) => c.as_str(),
        }
    }

    /// Returns an emoji icon associated with this category.
    pub fn emoji(&self) -> &'static str {
        match self {
            Self::UserPreference => "🎨",
            Self::ProjectFact => "💡",
            Self::ArchitectureDecision => "🏗️",
            Self::ToolState => "⚙️",
            Self::Custom(_) => "📌",
        }
    }

    /// Returns a short description of the category's intended purpose.
    pub fn description(&self) -> &'static str {
        match self {
            Self::UserPreference => "User coding preferences, tooling habits, and stylistic tastes",
            Self::ProjectFact => {
                "Project facts, tech stack details, environment settings, and models"
            }
            Self::ArchitectureDecision => {
                "System architecture decisions, design invariants, ADRs, and conventions"
            }
            Self::ToolState => {
                "Persistent tool states, cached parameters, and execution configurations"
            }
            Self::Custom(_) => "Custom user-defined memory domain",
        }
    }

    /// Loosely parses a string into a `MemoryCategory`.
    pub fn from_str_loose(s: &str) -> Self {
        let normalized = s.trim().to_lowercase().replace('-', "_");
        match normalized.as_str() {
            "user_preference" | "user_pref" | "preference" | "pref" | "style"
            | "user_preferences" => Self::UserPreference,
            "project_fact" | "fact" | "facts" | "general_fact" | "general" | "info"
            | "knowledge" | "project_info" => Self::ProjectFact,
            "architecture_decision"
            | "architecture"
            | "arch"
            | "decision"
            | "adr"
            | "project_architecture"
            | "project_arch"
            | "tech_stack"
            | "stack"
            | "convention"
            | "conv"
            | "recurring_convention"
            | "conventions"
            | "rule"
            | "rules"
            | "standard"
            | "standards" => Self::ArchitectureDecision,
            "tool_state" | "tool" | "state" | "tool_config" | "tools" | "tool_status" => {
                Self::ToolState
            }
            other => Self::Custom(other.to_string()),
        }
    }
}

impl std::fmt::Display for MemoryCategory {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.display_name())
    }
}

// ---------------------------------------------------------------------------
// Relevance Scoring & Associative Search Types
// ---------------------------------------------------------------------------

/// Detailed breakdown of the relevance score computed for a memory entry.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RelevanceScore {
    /// Overall composite score combining keyword similarity, importance, recency, and decay.
    pub total_score: f64,

    /// Keyword and semantic match score (key, content, tags, category).
    pub keyword_score: f64,

    /// Score contributed by the memory's configured importance level (1..=5).
    pub importance_score: f64,

    /// Score boost contributed by matching the active workspace.
    pub workspace_score: f64,

    /// Score boost contributed by historical access frequency.
    pub access_score: f64,

    /// Multiplier (0.0..=1.0) applied due to temporal decay.
    pub decay_factor: f64,

    /// Query terms that matched key, content, or tags.
    pub matched_terms: Vec<String>,
}

/// A memory entry paired with its calculated relevance score.
#[derive(Debug, Clone, PartialEq)]
pub struct ScoredMemory<'a> {
    /// Reference to the underlying memory entry.
    pub entry: &'a MemoryEntry,

    /// Computed relevance score breakdown.
    pub score: RelevanceScore,
}

// ---------------------------------------------------------------------------
// Memory Entry
// ---------------------------------------------------------------------------

/// A single persistent memory item.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MemoryEntry {
    /// Unique memory identifier (e.g. `mem_1a2b3c4d`).
    pub id: String,

    /// Category classifying this memory item.
    pub category: MemoryCategory,

    /// Short, descriptive key or slug identifying the memory (e.g. `rust_error_handling`).
    pub key: String,

    /// The factual or prescriptive text content of this memory.
    pub content: String,

    /// Optional workspace path or project identifier this memory applies to.
    /// If `None`, the memory applies globally across all workspaces.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace: Option<String>,

    /// Searchable tags associated with this memory.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,

    /// Importance score from 1 (minor note) to 5 (critical hard invariant). Default: 3.
    #[serde(default = "default_importance")]
    pub importance: u8,

    /// ISO 8601 / RFC 3339 creation timestamp.
    pub created_at: String,

    /// ISO 8601 / RFC 3339 last update timestamp.
    pub updated_at: String,

    /// Number of times this memory was accessed or injected into prompt context.
    #[serde(default)]
    pub access_count: u64,

    /// ISO 8601 / RFC 3339 timestamp of the last access.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_accessed_at: Option<String>,
}

fn default_importance() -> u8 {
    3
}

impl MemoryEntry {
    /// Creates a new `MemoryEntry` with a freshly generated ID and current timestamps.
    pub fn new(
        category: MemoryCategory,
        key: impl Into<String>,
        content: impl Into<String>,
    ) -> Self {
        let now = Utc::now().to_rfc3339();
        let short_id = format!("mem_{}", &Uuid::new_v4().to_string()[..8]);
        Self {
            id: short_id,
            category,
            key: key.into().trim().to_string(),
            content: content.into().trim().to_string(),
            workspace: None,
            tags: Vec::new(),
            importance: 3,
            created_at: now.clone(),
            updated_at: now,
            access_count: 0,
            last_accessed_at: None,
        }
    }

    /// Creates a user preference memory entry.
    pub fn preference(key: impl Into<String>, content: impl Into<String>) -> Self {
        Self::new(MemoryCategory::UserPreference, key, content)
    }

    /// Creates a user preference memory entry (alias).
    pub fn user_preference(key: impl Into<String>, content: impl Into<String>) -> Self {
        Self::new(MemoryCategory::UserPreference, key, content)
    }

    /// Creates a project fact memory entry, optionally tied to a workspace.
    pub fn project_fact(
        workspace: Option<impl Into<String>>,
        key: impl Into<String>,
        content: impl Into<String>,
    ) -> Self {
        let mut entry = Self::new(MemoryCategory::ProjectFact, key, content);
        entry.workspace = workspace.map(|w| w.into());
        entry
    }

    /// Creates a project fact memory entry (backward compatibility alias).
    pub fn fact(
        workspace: Option<impl Into<String>>,
        key: impl Into<String>,
        content: impl Into<String>,
    ) -> Self {
        Self::project_fact(workspace, key, content)
    }

    /// Creates an architecture decision memory entry.
    pub fn architecture_decision(
        workspace: Option<impl Into<String>>,
        key: impl Into<String>,
        content: impl Into<String>,
    ) -> Self {
        let mut entry = Self::new(MemoryCategory::ArchitectureDecision, key, content);
        entry.workspace = workspace.map(|w| w.into());
        entry
    }

    /// Creates an architecture decision memory entry (alias).
    pub fn architecture(
        workspace: Option<impl Into<String>>,
        key: impl Into<String>,
        content: impl Into<String>,
    ) -> Self {
        Self::architecture_decision(workspace, key, content)
    }

    /// Creates a recurring convention memory entry.
    pub fn convention(key: impl Into<String>, content: impl Into<String>) -> Self {
        Self::new(MemoryCategory::ArchitectureDecision, key, content)
    }

    /// Creates a tool state memory entry.
    pub fn tool_state(
        workspace: Option<impl Into<String>>,
        key: impl Into<String>,
        content: impl Into<String>,
    ) -> Self {
        let mut entry = Self::new(MemoryCategory::ToolState, key, content);
        entry.workspace = workspace.map(|w| w.into());
        entry
    }

    /// Creates a custom category memory entry.
    pub fn custom(
        category: impl Into<String>,
        key: impl Into<String>,
        content: impl Into<String>,
    ) -> Self {
        Self::new(MemoryCategory::Custom(category.into()), key, content)
    }

    /// Overrides the memory entry's unique ID.
    pub fn with_id(mut self, id: impl Into<String>) -> Self {
        self.id = id.into();
        self
    }

    /// Scopes this memory to a specific workspace directory or project name.
    pub fn with_workspace(mut self, workspace: impl Into<String>) -> Self {
        self.workspace = Some(workspace.into());
        self
    }

    /// Sets searchable tags.
    pub fn with_tags(mut self, tags: Vec<String>) -> Self {
        self.tags = tags;
        self
    }

    /// Appends a single searchable tag.
    pub fn with_tag(mut self, tag: impl Into<String>) -> Self {
        self.tags.push(tag.into());
        self
    }

    /// Sets importance level (clamped to 1..=5).
    pub fn with_importance(mut self, importance: u8) -> Self {
        self.importance = importance.clamp(1, 5);
        self
    }

    /// Sets creation timestamp (RFC 3339).
    pub fn with_created_at(mut self, created_at: impl Into<String>) -> Self {
        self.created_at = created_at.into();
        self
    }

    /// Sets update timestamp (RFC 3339).
    pub fn with_updated_at(mut self, updated_at: impl Into<String>) -> Self {
        self.updated_at = updated_at.into();
        self
    }

    /// Sets access count.
    pub fn with_access_count(mut self, count: u64) -> Self {
        self.access_count = count;
        self
    }

    /// Sets last accessed timestamp (RFC 3339).
    pub fn with_last_accessed_at(mut self, last_accessed_at: impl Into<String>) -> Self {
        self.last_accessed_at = Some(last_accessed_at.into());
        self
    }

    /// Returns true if this memory entry is global (applies to all workspaces).
    pub fn is_global(&self) -> bool {
        self.workspace.is_none()
    }

    /// Returns true if this memory entry applies to the given workspace target.
    /// Global memories always match any target workspace.
    pub fn matches_workspace(&self, target_workspace: Option<&str>) -> bool {
        match (&self.workspace, target_workspace) {
            (None, _) => true,       // Global applies everywhere
            (Some(_), None) => true, // If no workspace specified, match all
            (Some(entry_ws), Some(target_ws)) => {
                let entry_norm = entry_ws.trim().trim_end_matches(['/', '\\']);
                let target_norm = target_ws.trim().trim_end_matches(['/', '\\']);

                entry_norm.eq_ignore_ascii_case(target_norm)
                    || target_norm.ends_with(entry_norm)
                    || entry_norm.ends_with(target_norm)
            }
        }
    }

    /// Checks if this memory matches a free-form search query.
    pub fn matches_query(&self, query: &str) -> bool {
        let q = query.trim().to_lowercase();
        if q.is_empty() {
            return true;
        }

        self.key.to_lowercase().contains(&q)
            || self.content.to_lowercase().contains(&q)
            || self.category.as_str().contains(&q)
            || self.tags.iter().any(|t| t.to_lowercase().contains(&q))
            || self
                .workspace
                .as_deref()
                .map(|w| w.to_lowercase().contains(&q))
                .unwrap_or(false)
    }

    /// Records an access, updating timestamp and count.
    pub fn touch(&mut self) {
        self.access_count = self.access_count.saturating_add(1);
        self.last_accessed_at = Some(Utc::now().to_rfc3339());
    }

    /// Updates the text content and refreshes `updated_at`.
    pub fn update_content(&mut self, new_content: impl Into<String>) {
        self.content = new_content.into().trim().to_string();
        self.updated_at = Utc::now().to_rfc3339();
    }

    /// Returns the latest timestamp of activity (last_accessed_at, updated_at, or created_at).
    pub fn last_activity_utc(&self) -> DateTime<Utc> {
        let parse_time = |s: &str| -> Option<DateTime<Utc>> {
            DateTime::parse_from_rfc3339(s)
                .ok()
                .map(|dt| dt.with_timezone(&Utc))
        };

        if let Some(accessed) = self.last_accessed_at.as_deref().and_then(parse_time) {
            return accessed;
        }
        if let Some(updated) = parse_time(&self.updated_at) {
            return updated;
        }
        if let Some(created) = parse_time(&self.created_at) {
            return created;
        }

        Utc::now()
    }

    /// Returns the age in days since the memory's last activity relative to `now`.
    pub fn age_days(&self, now: DateTime<Utc>) -> f64 {
        let last_activity = self.last_activity_utc();
        let elapsed_seconds = (now - last_activity).num_seconds().max(0) as f64;
        elapsed_seconds / 86400.0
    }

    /// Computes the temporal decay factor (between 0.05 and 1.0) using half-life decay.
    ///
    /// The effective half-life scales with memory importance:
    /// - Importance 1: 7 days
    /// - Importance 2: 14 days
    /// - Importance 3: 30 days
    /// - Importance 4: 90 days
    /// - Importance 5: 365 days (high permanence)
    pub fn compute_decay_factor(
        &self,
        now: DateTime<Utc>,
        half_life_days_override: Option<f64>,
    ) -> f64 {
        let half_life_days = half_life_days_override.unwrap_or_else(|| match self.importance {
            1 => 7.0,
            2 => 14.0,
            3 => 30.0,
            4 => 90.0,
            _ => 365.0,
        });

        let age = self.age_days(now);
        let raw_decay = 2.0_f64.powf(-age / half_life_days.max(0.1));

        // Retention floor ensures important invariant facts never decay to zero
        let min_floor = 0.05 + (self.importance as f64) * 0.05; // 0.10 for imp 1, 0.30 for imp 5
        raw_decay.max(min_floor).min(1.0)
    }

    /// Formats a single markdown bullet line suitable for prompt injection.
    pub fn format_markdown_bullet(&self) -> String {
        format!("- **{}**: {}", self.key, self.content)
    }

    /// Formats a detailed multi-line human-readable summary.
    pub fn format_detailed(&self) -> String {
        let ws = self.workspace.as_deref().unwrap_or("Global (All projects)");
        let tags_str = if self.tags.is_empty() {
            "none".to_string()
        } else {
            self.tags.join(", ")
        };
        let stars = "★".repeat(self.importance as usize);

        format!(
            "{} [{}] {}\n\
             Key:        {}\n\
             Importance: {} ({}/5)\n\
             Scope:      {}\n\
             Tags:       {}\n\
             Updated:    {}\n\
             Accesses:   {}\n\
             Content:\n  {}",
            self.category.emoji(),
            self.category.display_name(),
            self.id,
            self.key,
            stars,
            self.importance,
            ws,
            tags_str,
            self.updated_at,
            self.access_count,
            self.content.replace('\n', "\n  ")
        )
    }
}

// ---------------------------------------------------------------------------
// Memory Store
// ---------------------------------------------------------------------------

/// Persistent memory store holding user preferences, project facts, architecture decisions, and tool states.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryStore {
    /// Schema version for forward/backward compatibility.
    #[serde(default = "default_schema_version")]
    pub version: u32,

    /// List of stored persistent memory entries.
    pub entries: Vec<MemoryEntry>,

    /// ISO 8601 / RFC 3339 timestamp when the store was last modified.
    #[serde(default = "default_now")]
    pub updated_at: String,
}

fn default_schema_version() -> u32 {
    1
}

fn default_now() -> String {
    Utc::now().to_rfc3339()
}

impl Default for MemoryStore {
    fn default() -> Self {
        Self::new()
    }
}

impl MemoryStore {
    /// Creates a new empty `MemoryStore`.
    pub fn new() -> Self {
        Self {
            version: 1,
            entries: Vec::new(),
            updated_at: Utc::now().to_rfc3339(),
        }
    }

    /// Returns the standard path to `~/.fusion/memory.json`.
    pub fn default_path() -> PathBuf {
        Config::config_dir().join("memory.json")
    }

    /// Loads the persistent memory store from the default location `~/.fusion/memory.json`.
    ///
    /// If the file does not exist, returns a fresh empty `MemoryStore`.
    /// If the file is corrupted or unparseable, logs a warning and returns an empty store.
    pub fn load() -> anyhow::Result<Self> {
        let path = Self::default_path();
        Self::load_from_path(&path)
    }

    /// Loads the memory store from a specific file path.
    pub fn load_from_path(path: &Path) -> anyhow::Result<Self> {
        if !path.exists() {
            return Ok(Self::new());
        }

        let content = match fs::read_to_string(path) {
            Ok(c) => c,
            Err(e) => {
                tracing::warn!(
                    "Failed to read memory file at {}: {}. Starting with empty store.",
                    path.display(),
                    e
                );
                return Ok(Self::new());
            }
        };

        if content.trim().is_empty() {
            return Ok(Self::new());
        }

        match serde_json::from_str::<MemoryStore>(&content) {
            Ok(store) => Ok(store),
            Err(e) => {
                tracing::warn!(
                    "Failed to parse memory JSON at {}: {}. Starting with empty store.",
                    path.display(),
                    e
                );
                Ok(Self::new())
            }
        }
    }

    /// Saves the current memory store atomically to `~/.fusion/memory.json`.
    pub fn save(&self) -> anyhow::Result<PathBuf> {
        let path = Self::default_path();
        self.save_to_path(&path)
    }

    /// Saves the memory store atomically to a specific file path.
    pub fn save_to_path(&self, path: &Path) -> anyhow::Result<PathBuf> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }

        let json_str = serde_json::to_string_pretty(self)?;

        // Safe atomic write using sibling temporary file
        let temp_path = path.with_extension(format!("tmp.{}", Uuid::new_v4()));
        fs::write(&temp_path, json_str.as_bytes())?;
        fs::rename(&temp_path, path)?;

        Ok(path.to_path_buf())
    }

    /// Serializes the memory store to a pretty JSON string.
    pub fn to_json(&self) -> anyhow::Result<String> {
        Ok(serde_json::to_string_pretty(self)?)
    }

    /// Deserializes a memory store from a JSON string.
    pub fn from_json(json_str: &str) -> anyhow::Result<Self> {
        Ok(serde_json::from_str(json_str)?)
    }

    // -----------------------------------------------------------------------
    // Markdown Export & Import
    // -----------------------------------------------------------------------

    /// Exports all persistent memories as a comprehensive, well-structured Markdown document.
    pub fn export_markdown(&self) -> String {
        let mut out = String::with_capacity(4096);

        out.push_str("# 🧠 Fusion Persistent Memory Store\n\n");
        out.push_str(&format!("- **Schema Version**: `{}`\n", self.version));
        out.push_str(&format!("- **Last Modified**: `{}`\n", self.updated_at));
        out.push_str(&format!(
            "- **Total Memories**: `{}`\n\n",
            self.entries.len()
        ));

        let categories = [
            (MemoryCategory::UserPreference, "User Preferences"),
            (MemoryCategory::ProjectFact, "Project Facts"),
            (
                MemoryCategory::ArchitectureDecision,
                "Architecture Decisions",
            ),
            (MemoryCategory::ToolState, "Tool States"),
        ];

        for (cat, title) in &categories {
            let in_cat = self.filter_by_category(cat);
            if !in_cat.is_empty() {
                out.push_str(&format!(
                    "## {} {} ({})\n",
                    cat.emoji(),
                    title,
                    in_cat.len()
                ));
                out.push_str(&format!("*{}*\n\n", cat.description()));

                for entry in in_cat {
                    let ws_str = entry
                        .workspace
                        .as_deref()
                        .unwrap_or("Global (All workspaces)");
                    let stars = "★".repeat(entry.importance as usize);
                    let tags_str = if entry.tags.is_empty() {
                        "none".to_string()
                    } else {
                        entry
                            .tags
                            .iter()
                            .map(|t| format!("`{}`", t))
                            .collect::<Vec<_>>()
                            .join(", ")
                    };

                    out.push_str(&format!("### `{}`\n\n", entry.key));
                    out.push_str(&format!("- **ID**: `{}`\n", entry.id));
                    out.push_str(&format!(
                        "- **Importance**: {} ({}/5)\n",
                        stars, entry.importance
                    ));
                    out.push_str(&format!("- **Scope**: `{}`\n", ws_str));
                    out.push_str(&format!("- **Tags**: {}\n", tags_str));
                    out.push_str(&format!("- **Access Count**: `{}`\n", entry.access_count));
                    out.push_str(&format!(
                        "- **Created**: `{}` | **Updated**: `{}`\n\n",
                        entry.created_at, entry.updated_at
                    ));
                    out.push_str(&format!("{}\n\n---\n\n", entry.content));
                }
            }
        }

        // Custom categories
        let customs: Vec<&MemoryEntry> = self
            .entries
            .iter()
            .filter(|e| matches!(e.category, MemoryCategory::Custom(_)))
            .collect();

        if !customs.is_empty() {
            out.push_str(&format!("## 📌 Custom Categories ({})\n\n", customs.len()));
            for entry in customs {
                let cat_name = entry.category.as_str();
                out.push_str(&format!(
                    "### `{}` (Category: `{}`)\n\n",
                    entry.key, cat_name
                ));
                out.push_str(&format!("- **ID**: `{}`\n", entry.id));
                out.push_str(&format!("- **Importance**: {}/5\n", entry.importance));
                out.push_str(&format!("- **Content**: {}\n\n---\n\n", entry.content));
            }
        }

        out.trim_end().to_string()
    }

    /// Exports memories to a Markdown file atomically.
    pub fn export_markdown_to_path(&self, path: &Path) -> anyhow::Result<PathBuf> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }

        let md_content = self.export_markdown();
        let temp_path = path.with_extension(format!("tmp.{}", Uuid::new_v4()));
        fs::write(&temp_path, md_content.as_bytes())?;
        fs::rename(&temp_path, path)?;

        Ok(path.to_path_buf())
    }

    /// Parses markdown content and merges valid memory bullet items into the store.
    /// Returns the number of entries imported.
    pub fn import_markdown(&mut self, md: &str) -> usize {
        let mut imported = 0;
        let mut current_category = MemoryCategory::ProjectFact;

        for line in md.lines() {
            let trimmed = line.trim();

            if trimmed.starts_with("## ") {
                let header = trimmed.trim_start_matches("## ").to_lowercase();
                if header.contains("preference") {
                    current_category = MemoryCategory::UserPreference;
                } else if header.contains("architecture")
                    || header.contains("decision")
                    || header.contains("convention")
                {
                    current_category = MemoryCategory::ArchitectureDecision;
                } else if header.contains("tool") {
                    current_category = MemoryCategory::ToolState;
                } else if header.contains("fact") {
                    current_category = MemoryCategory::ProjectFact;
                }
                continue;
            }

            // Bullet format: - **key**: content or - `key`: content
            if trimmed.starts_with("- **") || trimmed.starts_with("- `") {
                let rest = trimmed.trim_start_matches('-').trim();
                if let Some((raw_key, content)) = rest.split_once(':') {
                    let key = raw_key.trim().trim_matches('*').trim_matches('`').trim();
                    let clean_content = content.trim();

                    if !key.is_empty() && !clean_content.is_empty() {
                        let entry = MemoryEntry::new(current_category.clone(), key, clean_content);
                        self.add(entry);
                        imported += 1;
                    }
                }
            }
        }

        imported
    }

    // -----------------------------------------------------------------------
    // Core Mutations & CRUD
    // -----------------------------------------------------------------------

    /// Returns the total number of memory entries.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Returns true if the store contains no entries.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Returns a slice of all memory entries.
    pub fn entries(&self) -> &[MemoryEntry] {
        &self.entries
    }

    /// Returns a mutable slice of all memory entries.
    pub fn entries_mut(&mut self) -> &mut Vec<MemoryEntry> {
        &mut self.entries
    }

    /// Adds or updates a memory entry in the store.
    /// If an entry with matching key and workspace already exists, it is updated in-place.
    /// Otherwise, the new entry is appended.
    pub fn add(&mut self, entry: MemoryEntry) -> &MemoryEntry {
        let now = Utc::now().to_rfc3339();
        self.updated_at = now.clone();

        // Check if matching key already exists
        if let Some(idx) = self
            .entries
            .iter()
            .position(|e| e.key.eq_ignore_ascii_case(&entry.key) && e.workspace == entry.workspace)
        {
            self.entries[idx].content = entry.content;
            self.entries[idx].category = entry.category;
            if !entry.tags.is_empty() {
                self.entries[idx].tags = entry.tags;
            }
            self.entries[idx].importance = entry.importance;
            self.entries[idx].updated_at = now;
            &self.entries[idx]
        } else {
            self.entries.push(entry);
            let idx = self.entries.len() - 1;
            &self.entries[idx]
        }
    }

    /// Records or updates a general memory entry by category, key, and content.
    pub fn remember(
        &mut self,
        category: MemoryCategory,
        key: impl Into<String>,
        content: impl Into<String>,
    ) -> &MemoryEntry {
        let entry = MemoryEntry::new(category, key, content);
        self.add(entry)
    }

    /// Records or updates a user coding preference.
    pub fn remember_preference(
        &mut self,
        key: impl Into<String>,
        content: impl Into<String>,
    ) -> &MemoryEntry {
        let entry = MemoryEntry::preference(key, content);
        self.add(entry)
    }

    /// Records or updates a project fact.
    pub fn remember_project_fact(
        &mut self,
        workspace: Option<&str>,
        key: impl Into<String>,
        content: impl Into<String>,
    ) -> &MemoryEntry {
        let entry = MemoryEntry::project_fact(workspace.map(|s| s.to_string()), key, content);
        self.add(entry)
    }

    /// Records or updates a general fact (alias).
    pub fn remember_fact(
        &mut self,
        workspace: Option<&str>,
        key: impl Into<String>,
        content: impl Into<String>,
    ) -> &MemoryEntry {
        self.remember_project_fact(workspace, key, content)
    }

    /// Records or updates an architecture decision.
    pub fn remember_architecture_decision(
        &mut self,
        workspace: Option<&str>,
        key: impl Into<String>,
        content: impl Into<String>,
    ) -> &MemoryEntry {
        let entry =
            MemoryEntry::architecture_decision(workspace.map(|s| s.to_string()), key, content);
        self.add(entry)
    }

    /// Records or updates a project architecture fact (alias).
    pub fn remember_architecture(
        &mut self,
        workspace: Option<&str>,
        key: impl Into<String>,
        content: impl Into<String>,
    ) -> &MemoryEntry {
        self.remember_architecture_decision(workspace, key, content)
    }

    /// Records or updates a recurring coding convention.
    pub fn remember_convention(
        &mut self,
        key: impl Into<String>,
        content: impl Into<String>,
    ) -> &MemoryEntry {
        let entry = MemoryEntry::convention(key, content);
        self.add(entry)
    }

    /// Records or updates a tool state entry.
    pub fn remember_tool_state(
        &mut self,
        workspace: Option<&str>,
        key: impl Into<String>,
        content: impl Into<String>,
    ) -> &MemoryEntry {
        let entry = MemoryEntry::tool_state(workspace.map(|s| s.to_string()), key, content);
        self.add(entry)
    }

    /// Retrieves an entry by its unique ID or key.
    pub fn get(&self, id_or_key: &str) -> Option<&MemoryEntry> {
        let needle = id_or_key.trim();
        self.entries
            .iter()
            .find(|e| e.id.eq_ignore_ascii_case(needle) || e.key.eq_ignore_ascii_case(needle))
    }

    /// Retrieves a mutable reference to an entry by its unique ID or key.
    pub fn get_mut(&mut self, id_or_key: &str) -> Option<&mut MemoryEntry> {
        let needle = id_or_key.trim();
        self.entries
            .iter_mut()
            .find(|e| e.id.eq_ignore_ascii_case(needle) || e.key.eq_ignore_ascii_case(needle))
    }

    /// Touches a memory entry by ID or key, incrementing its access count.
    pub fn touch(&mut self, id_or_key: &str) -> bool {
        if let Some(entry) = self.get_mut(id_or_key) {
            entry.touch();
            true
        } else {
            false
        }
    }

    /// Updates the text content of a memory entry by ID or key.
    pub fn update_content(&mut self, id_or_key: &str, content: impl Into<String>) -> bool {
        if let Some(entry) = self.get_mut(id_or_key) {
            entry.update_content(content);
            self.updated_at = Utc::now().to_rfc3339();
            true
        } else {
            false
        }
    }

    /// Removes a memory entry by its unique ID or key.
    /// Returns true if an entry was found and removed.
    pub fn forget(&mut self, id_or_key: &str) -> bool {
        let needle = id_or_key.trim();
        let initial_len = self.entries.len();
        self.entries
            .retain(|e| !e.id.eq_ignore_ascii_case(needle) && !e.key.eq_ignore_ascii_case(needle));

        let changed = self.entries.len() < initial_len;
        if changed {
            self.updated_at = Utc::now().to_rfc3339();
        }
        changed
    }

    /// Removes all entries belonging to a given category.
    pub fn forget_by_category(&mut self, category: &MemoryCategory) -> usize {
        let initial_len = self.entries.len();
        self.entries.retain(|e| &e.category != category);
        let removed = initial_len.saturating_sub(self.entries.len());
        if removed > 0 {
            self.updated_at = Utc::now().to_rfc3339();
        }
        removed
    }

    /// Removes all entries scoped to a specific workspace.
    pub fn forget_by_workspace(&mut self, workspace: &str) -> usize {
        let initial_len = self.entries.len();
        self.entries.retain(|e| {
            if let Some(ws) = &e.workspace {
                !ws.eq_ignore_ascii_case(workspace)
            } else {
                true
            }
        });
        let removed = initial_len.saturating_sub(self.entries.len());
        if removed > 0 {
            self.updated_at = Utc::now().to_rfc3339();
        }
        removed
    }

    /// Clears all entries from the memory store.
    pub fn clear(&mut self) {
        self.entries.clear();
        self.updated_at = Utc::now().to_rfc3339();
    }

    // -----------------------------------------------------------------------
    // Queries, Filtering & Associative Search
    // -----------------------------------------------------------------------

    /// Returns all user preferences.
    pub fn preferences(&self) -> Vec<&MemoryEntry> {
        self.filter_by_category(&MemoryCategory::UserPreference)
    }

    /// Returns all project facts, optionally filtered for a workspace.
    pub fn project_facts(&self, workspace: Option<&str>) -> Vec<&MemoryEntry> {
        self.entries
            .iter()
            .filter(|e| e.category == MemoryCategory::ProjectFact && e.matches_workspace(workspace))
            .collect()
    }

    /// Returns all architecture decisions, optionally filtered for a workspace.
    pub fn architecture_decisions(&self, workspace: Option<&str>) -> Vec<&MemoryEntry> {
        self.entries
            .iter()
            .filter(|e| {
                e.category == MemoryCategory::ArchitectureDecision && e.matches_workspace(workspace)
            })
            .collect()
    }

    /// Returns all project architecture facts (alias).
    pub fn architecture_facts(&self, workspace: Option<&str>) -> Vec<&MemoryEntry> {
        self.architecture_decisions(workspace)
    }

    /// Returns all recurring conventions (alias).
    pub fn conventions(&self) -> Vec<&MemoryEntry> {
        self.filter_by_category(&MemoryCategory::ArchitectureDecision)
    }

    /// Returns all tool states.
    pub fn tool_states(&self, workspace: Option<&str>) -> Vec<&MemoryEntry> {
        self.entries
            .iter()
            .filter(|e| e.category == MemoryCategory::ToolState && e.matches_workspace(workspace))
            .collect()
    }

    /// Filters entries by category.
    pub fn filter_by_category(&self, category: &MemoryCategory) -> Vec<&MemoryEntry> {
        self.entries
            .iter()
            .filter(|e| &e.category == category)
            .collect()
    }

    /// Filters entries applicable to a given workspace (including global entries).
    pub fn filter_by_workspace(&self, workspace: Option<&str>) -> Vec<&MemoryEntry> {
        self.entries
            .iter()
            .filter(|e| e.matches_workspace(workspace))
            .collect()
    }

    /// Filters entries matching a specific tag.
    pub fn filter_by_tag(&self, tag: &str) -> Vec<&MemoryEntry> {
        let t = tag.trim().to_lowercase();
        self.entries
            .iter()
            .filter(|e| e.tags.iter().any(|entry_tag| entry_tag.to_lowercase() == t))
            .collect()
    }

    /// Searches entries across keys, contents, tags, and categories.
    pub fn search(&self, query: &str) -> Vec<&MemoryEntry> {
        self.entries
            .iter()
            .filter(|e| e.matches_query(query))
            .collect()
    }

    /// Performs associative fact retrieval with multi-factor relevance scoring:
    /// - Keyword match and BM25/TF token ranking
    /// - Key, content, tag, and category weighting
    /// - Importance score weighting (1..=5)
    /// - Workspace specificity boost
    /// - Access recency and frequency
    /// - Temporal exponential decay
    pub fn retrieve_associative(
        &self,
        query: &str,
        workspace: Option<&str>,
        limit: usize,
    ) -> Vec<ScoredMemory<'_>> {
        self.retrieve_associative_with_time(query, workspace, limit, Utc::now())
    }

    /// Performs associative retrieval with an explicit evaluation timestamp (useful for testing decay).
    pub fn retrieve_associative_with_time(
        &self,
        query: &str,
        workspace: Option<&str>,
        limit: usize,
        now: DateTime<Utc>,
    ) -> Vec<ScoredMemory<'_>> {
        let tokens: Vec<String> = query
            .to_lowercase()
            .split(|c: char| !c.is_alphanumeric() && c != '_')
            .filter(|s| !s.is_empty())
            .map(|s| s.to_string())
            .collect();

        let mut scored_entries: Vec<ScoredMemory<'_>> = self
            .entries
            .iter()
            .filter(|e| e.matches_workspace(workspace))
            .filter_map(|entry| {
                let mut keyword_score = 0.0;
                let mut matched_terms = Vec::new();

                let key_lower = entry.key.to_lowercase();
                let content_lower = entry.content.to_lowercase();
                let cat_lower = entry.category.as_str().to_lowercase();

                let query_clean = query.trim().to_lowercase();

                if !query_clean.is_empty() {
                    // Exact substring bonus
                    if key_lower.contains(&query_clean) {
                        keyword_score += 120.0;
                        matched_terms.push(query_clean.clone());
                    } else if content_lower.contains(&query_clean) {
                        keyword_score += 60.0;
                        matched_terms.push(query_clean.clone());
                    }

                    // Token-level matching
                    for tok in &tokens {
                        let mut tok_matched = false;

                        // Key matches
                        if key_lower == *tok {
                            keyword_score += 80.0;
                            tok_matched = true;
                        } else if key_lower.contains(tok) {
                            keyword_score += 40.0;
                            tok_matched = true;
                        }

                        // Tag matches
                        if entry.tags.iter().any(|t| t.to_lowercase() == *tok) {
                            keyword_score += 50.0;
                            tok_matched = true;
                        }

                        // Content token frequency match
                        let occurrences = content_lower.matches(tok).count();
                        if occurrences > 0 {
                            keyword_score += (occurrences as f64).min(5.0) * 15.0;
                            tok_matched = true;
                        }

                        // Category match
                        if cat_lower.contains(tok) {
                            keyword_score += 20.0;
                            tok_matched = true;
                        }

                        if tok_matched && !matched_terms.contains(tok) {
                            matched_terms.push(tok.clone());
                        }
                    }

                    // If query is specified and no keywords matched, exclude from results
                    if keyword_score == 0.0 {
                        return None;
                    }
                }

                // 2. Importance score (20 to 100)
                let importance_score = (entry.importance as f64) * 20.0;

                // 3. Workspace specificity boost (scoped workspace > global)
                let workspace_score = if entry.workspace.is_some() {
                    40.0
                } else {
                    15.0
                };

                // 4. Access frequency boost
                let access_score = (entry.access_count.min(20) as f64) * 3.0;

                // 5. Temporal decay
                let decay_factor = entry.compute_decay_factor(now, None);

                let base_score = keyword_score + importance_score + workspace_score + access_score;
                let total_score = base_score * decay_factor;

                Some(ScoredMemory {
                    entry,
                    score: RelevanceScore {
                        total_score,
                        keyword_score,
                        importance_score,
                        workspace_score,
                        access_score,
                        decay_factor,
                        matched_terms,
                    },
                })
            })
            .collect();

        // Sort descending by total score
        scored_entries.sort_by(|a, b| {
            b.score
                .total_score
                .partial_cmp(&a.score.total_score)
                .unwrap_or(std::cmp::Ordering::Equal)
        });

        if limit > 0 {
            scored_entries.truncate(limit);
        }

        scored_entries
    }

    /// Finds memories associated with a given entry (sharing tags, workspace, category, or keywords).
    pub fn find_associated(&self, id_or_key: &str, limit: usize) -> Vec<ScoredMemory<'_>> {
        let target = match self.get(id_or_key) {
            Some(e) => e,
            None => return Vec::new(),
        };

        let mut query_builder = target.key.clone();
        for tag in &target.tags {
            query_builder.push(' ');
            query_builder.push_str(tag);
        }

        let mut results = self.retrieve_associative(
            &query_builder,
            target.workspace.as_deref(),
            limit.saturating_add(1),
        );

        // Remove the target entry itself
        results.retain(|m| m.entry.id != target.id && m.entry.key != target.key);
        if limit > 0 {
            results.truncate(limit);
        }

        results
    }

    /// Returns the most relevant memories for the active workspace and optional context query.
    /// Backward-compatible helper that returns `Vec<&MemoryEntry>`.
    pub fn relevant_memories(
        &self,
        workspace: Option<&str>,
        query: Option<&str>,
        limit: usize,
    ) -> Vec<&MemoryEntry> {
        let q = query.unwrap_or("");
        let scored = self.retrieve_associative(q, workspace, limit);
        scored.into_iter().map(|sm| sm.entry).collect()
    }

    // -----------------------------------------------------------------------
    // Prompt & Display Formatting
    // -----------------------------------------------------------------------

    /// Formats persistent memories into a structured Markdown section ready for LLM system prompt injection.
    ///
    /// Categorizes memories into:
    /// - User Coding Preferences
    /// - Project Architecture & Design Facts
    /// - Architecture Decisions & Recurring Conventions
    /// - Tool States & Environment Settings
    pub fn format_for_system_prompt(&self, workspace: Option<&str>) -> String {
        let applicable: Vec<&MemoryEntry> = self
            .entries
            .iter()
            .filter(|e| e.matches_workspace(workspace))
            .collect();

        if applicable.is_empty() {
            return String::new();
        }

        let mut out = String::with_capacity(2048);

        // 1. User Preferences
        let prefs: Vec<&MemoryEntry> = applicable
            .iter()
            .copied()
            .filter(|e| e.category == MemoryCategory::UserPreference)
            .collect();
        if !prefs.is_empty() {
            out.push_str("### User Coding Preferences:\n");
            for p in prefs {
                out.push_str(&p.format_markdown_bullet());
                out.push('\n');
            }
            out.push('\n');
        }

        // 2. Project Facts
        let facts: Vec<&MemoryEntry> = applicable
            .iter()
            .copied()
            .filter(|e| e.category == MemoryCategory::ProjectFact)
            .collect();
        if !facts.is_empty() {
            out.push_str("### Project Architecture & Design Facts:\n");
            for f in facts {
                out.push_str(&f.format_markdown_bullet());
                out.push('\n');
            }
            out.push('\n');
        }

        // 3. Architecture Decisions & Conventions
        let arch: Vec<&MemoryEntry> = applicable
            .iter()
            .copied()
            .filter(|e| e.category == MemoryCategory::ArchitectureDecision)
            .collect();
        if !arch.is_empty() {
            out.push_str("### Recurring Conventions & Guidelines:\n");
            for a in arch {
                out.push_str(&a.format_markdown_bullet());
                out.push('\n');
            }
            out.push('\n');
        }

        // 4. Tool State / Custom
        let others: Vec<&MemoryEntry> = applicable
            .iter()
            .copied()
            .filter(|e| {
                e.category == MemoryCategory::ToolState
                    || matches!(e.category, MemoryCategory::Custom(_))
            })
            .collect();
        if !others.is_empty() {
            out.push_str("### Learned Environment & Project Facts:\n");
            for o in others {
                out.push_str(&o.format_markdown_bullet());
                out.push('\n');
            }
            out.push('\n');
        }

        out.trim_end().to_string()
    }

    /// Formats a clean, tabular/bullet overview of all memories for CLI or UI display.
    pub fn format_summary(&self) -> String {
        if self.entries.is_empty() {
            return "No persistent memories stored yet. Use `/memory add` or the agent will learn them automatically.".to_string();
        }

        let mut lines = Vec::new();
        lines.push(format!(
            "🧠 **Fusion Persistent Memory Store** ({} item{}, stored at `{}`)\n",
            self.entries.len(),
            if self.entries.len() == 1 { "" } else { "s" },
            Self::default_path().display()
        ));

        // Group by category
        let categories = [
            MemoryCategory::UserPreference,
            MemoryCategory::ProjectFact,
            MemoryCategory::ArchitectureDecision,
            MemoryCategory::ToolState,
        ];

        for cat in &categories {
            let in_cat: Vec<&MemoryEntry> = self.filter_by_category(cat);
            if !in_cat.is_empty() {
                lines.push(format!(
                    "### {} {} ({}):",
                    cat.emoji(),
                    cat.display_name(),
                    in_cat.len()
                ));
                for entry in in_cat {
                    let scope = entry
                        .workspace
                        .as_deref()
                        .map(|w| format!(" `[{}]`", w))
                        .unwrap_or_default();
                    lines.push(format!(
                        "- `{}`{}: {}{}",
                        entry.key,
                        scope,
                        entry.content,
                        if entry.importance >= 4 { " ⭐️" } else { "" }
                    ));
                }
                lines.push(String::new());
            }
        }

        // Custom categories if any
        let customs: Vec<&MemoryEntry> = self
            .entries
            .iter()
            .filter(|e| matches!(e.category, MemoryCategory::Custom(_)))
            .collect();

        if !customs.is_empty() {
            lines.push(format!("### 📌 Custom Categories ({}):", customs.len()));
            for entry in customs {
                lines.push(format!("- `{}`: {}", entry.key, entry.content));
            }
            lines.push(String::new());
        }

        lines.join("\n").trim_end().to_string()
    }

    /// Formats detailed inspector view for a specific entry.
    pub fn format_detailed(&self, id_or_key: &str) -> Option<String> {
        self.get(id_or_key).map(|e| e.format_detailed())
    }
}

// ---------------------------------------------------------------------------
// Memory Tool for LLM Integration
// ---------------------------------------------------------------------------

/// Tool allowing LLMs to inspect, record, query, export, and forget long-term memories across sessions.
#[derive(Default, Debug, Clone)]
pub struct MemoryTool;

impl MemoryTool {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl Tool for MemoryTool {
    fn name(&self) -> &str {
        "memory"
    }

    fn description(&self) -> &str {
        "Store, recall, search, update, export, or forget persistent cross-session memories \
         (user coding preferences, project facts, architecture decisions, and tool states)."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "action": {
                    "type": "string",
                    "enum": ["store", "remember", "query", "search", "list", "get", "forget", "delete", "export", "clear"],
                    "description": "The action to perform: 'store'/'remember' (save or update fact), 'query'/'search' (find relevant memories), 'list' (list all), 'get' (retrieve by key/id), 'forget'/'delete' (remove fact), 'export' (export as markdown/json), 'clear' (wipe all)."
                },
                "category": {
                    "type": "string",
                    "enum": ["user_preference", "project_fact", "architecture_decision", "tool_state", "custom"],
                    "description": "Category of memory: 'user_preference' (user style/tooling), 'project_fact' (tech stack/environment), 'architecture_decision' (design/ADR/conventions), 'tool_state' (persistent tool config)."
                },
                "key": {
                    "type": "string",
                    "description": "Short unique identifier or slug for the memory (e.g. 'rust_error_handling', 'project_stack', 'commit_style')."
                },
                "content": {
                    "type": "string",
                    "description": "The memory content or fact to store/update."
                },
                "workspace": {
                    "type": "string",
                    "description": "Optional workspace path or project name. If omitted, applies globally to all projects."
                },
                "tags": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "Optional search tags."
                },
                "importance": {
                    "type": "integer",
                    "minimum": 1,
                    "maximum": 5,
                    "description": "Importance level from 1 (minor note) to 5 (critical hard rule). Default: 3."
                },
                "query": {
                    "type": "string",
                    "description": "Search query string when performing 'query' or 'search'."
                },
                "format": {
                    "type": "string",
                    "enum": ["markdown", "json"],
                    "description": "Export format when performing 'export' (default: 'markdown')."
                }
            },
            "required": ["action"]
        })
    }

    async fn execute(&self, args: Value, ctx: &ToolContext) -> anyhow::Result<String> {
        let action = args
            .get("action")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("Missing required 'action' parameter"))?;

        let mut store = MemoryStore::load().unwrap_or_default();
        let ws_str = ctx.cwd.to_string_lossy().to_string();

        match action.to_lowercase().as_str() {
            "store" | "remember" | "save" | "add" => {
                let key = args
                    .get("key")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| anyhow::anyhow!("'key' parameter required for store action"))?;

                let content = args
                    .get("content")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| anyhow::anyhow!("'content' parameter required for store action"))?;

                let category = args
                    .get("category")
                    .and_then(|v| v.as_str())
                    .map(MemoryCategory::from_str_loose)
                    .unwrap_or(MemoryCategory::ProjectFact);

                let workspace = args
                    .get("workspace")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string());

                let tags: Vec<String> = args
                    .get("tags")
                    .and_then(|v| v.as_array())
                    .map(|arr| {
                        arr.iter()
                            .filter_map(|v| v.as_str().map(|s| s.to_string()))
                            .collect()
                    })
                    .unwrap_or_default();

                let importance = args
                    .get("importance")
                    .and_then(|v| v.as_u64())
                    .map(|n| n.clamp(1, 5) as u8)
                    .unwrap_or(3);

                let mut entry = MemoryEntry::new(category.clone(), key, content)
                    .with_tags(tags)
                    .with_importance(importance);

                if let Some(ws) = workspace {
                    entry = entry.with_workspace(ws);
                }

                store.add(entry);
                let save_path = store.save()?;

                Ok(format!(
                    "✓ Successfully remembered {} '{}' in persistent memory ({})",
                    category.display_name(),
                    key,
                    save_path.display()
                ))
            }

            "query" | "search" => {
                let query = args
                    .get("query")
                    .and_then(|v| v.as_str())
                    .or_else(|| args.get("key").and_then(|v| v.as_str()))
                    .unwrap_or("");

                let results = store.retrieve_associative(query, Some(&ws_str), 10);

                if results.is_empty() {
                    Ok(format!("No matching memories found for query: '{}'.", query))
                } else {
                    let mut out = format!("Found {} relevant memor{}:\n\n", results.len(), if results.len() == 1 { "y" } else { "ies" });
                    for r in results {
                        out.push_str(&format!(
                            "[Score: {:.1} | Keyword: {:.1} | Decay: {:.2}]\n{}\n\n---\n\n",
                            r.score.total_score,
                            r.score.keyword_score,
                            r.score.decay_factor,
                            r.entry.format_detailed()
                        ));
                    }
                    Ok(out.trim_end().to_string())
                }
            }

            "list" => {
                let category_filter = args
                    .get("category")
                    .and_then(|v| v.as_str())
                    .map(MemoryCategory::from_str_loose);

                let entries = if let Some(cat) = category_filter {
                    store.filter_by_category(&cat)
                } else {
                    store.entries().iter().collect()
                };

                if entries.is_empty() {
                    Ok("Memory store is currently empty.".to_string())
                } else {
                    let mut out = format!("Persistent Memories ({} total):\n\n", entries.len());
                    for e in entries {
                        out.push_str(&format!(
                            "- [{}] `{}` (importance {}/5): {}\n",
                            e.category.display_name(),
                            e.key,
                            e.importance,
                            e.content
                        ));
                    }
                    Ok(out)
                }
            }

            "get" => {
                let key = args
                    .get("key")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| anyhow::anyhow!("'key' or 'id' required for get action"))?;

                if let Some(entry) = store.get(key) {
                    Ok(entry.format_detailed())
                } else {
                    Ok(format!("No memory entry found with key or ID: '{}'.", key))
                }
            }

            "export" => {
                let fmt = args
                    .get("format")
                    .and_then(|v| v.as_str())
                    .unwrap_or("markdown");

                if fmt.eq_ignore_ascii_case("json") {
                    store.to_json()
                } else {
                    Ok(store.export_markdown())
                }
            }

            "forget" | "delete" | "remove" => {
                let key = args
                    .get("key")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| anyhow::anyhow!("'key' parameter required for forget action"))?;

                let removed = store.forget(key);
                if removed {
                    store.save()?;
                    Ok(format!("✓ Removed memory '{}' from persistent store.", key))
                } else {
                    Ok(format!("No memory found matching '{}'.", key))
                }
            }

            "clear" => {
                let count = store.len();
                store.clear();
                store.save()?;
                Ok(format!("✓ Cleared all {} entries from persistent memory.", count))
            }

            other => Err(anyhow::anyhow!(
                "Unknown memory action: '{}'. Supported: store, query, list, get, export, forget, clear.",
                other
            )),
        }
    }
}

// ---------------------------------------------------------------------------
// Unit Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Duration;
    use tempfile::tempdir;

    #[test]
    fn test_memory_category_loose_parsing() {
        assert_eq!(
            MemoryCategory::from_str_loose("preference"),
            MemoryCategory::UserPreference
        );
        assert_eq!(
            MemoryCategory::from_str_loose("user-pref"),
            MemoryCategory::UserPreference
        );
        assert_eq!(
            MemoryCategory::from_str_loose("style"),
            MemoryCategory::UserPreference
        );
        assert_eq!(
            MemoryCategory::from_str_loose("fact"),
            MemoryCategory::ProjectFact
        );
        assert_eq!(
            MemoryCategory::from_str_loose("project_fact"),
            MemoryCategory::ProjectFact
        );
        assert_eq!(
            MemoryCategory::from_str_loose("architecture"),
            MemoryCategory::ArchitectureDecision
        );
        assert_eq!(
            MemoryCategory::from_str_loose("architecture_decision"),
            MemoryCategory::ArchitectureDecision
        );
        assert_eq!(
            MemoryCategory::from_str_loose("convention"),
            MemoryCategory::ArchitectureDecision
        );
        assert_eq!(
            MemoryCategory::from_str_loose("adr"),
            MemoryCategory::ArchitectureDecision
        );
        assert_eq!(
            MemoryCategory::from_str_loose("tool"),
            MemoryCategory::ToolState
        );
        assert_eq!(
            MemoryCategory::from_str_loose("tool_state"),
            MemoryCategory::ToolState
        );
        assert_eq!(
            MemoryCategory::from_str_loose("custom_domain"),
            MemoryCategory::Custom("custom_domain".to_string())
        );
    }

    #[test]
    fn test_memory_category_display_and_emoji() {
        let pref = MemoryCategory::UserPreference;
        assert_eq!(pref.as_str(), "user_preference");
        assert_eq!(pref.display_name(), "User Preference");
        assert_eq!(pref.emoji(), "🎨");

        let fact = MemoryCategory::ProjectFact;
        assert_eq!(fact.as_str(), "project_fact");
        assert_eq!(fact.display_name(), "Project Fact");
        assert_eq!(fact.emoji(), "💡");

        let arch = MemoryCategory::ArchitectureDecision;
        assert_eq!(arch.as_str(), "architecture_decision");
        assert_eq!(arch.display_name(), "Architecture Decision");
        assert_eq!(arch.emoji(), "🏗️");

        let tool = MemoryCategory::ToolState;
        assert_eq!(tool.as_str(), "tool_state");
        assert_eq!(tool.display_name(), "Tool State");
        assert_eq!(tool.emoji(), "⚙️");
    }

    #[test]
    fn test_memory_entry_creation_and_matching() {
        let entry = MemoryEntry::preference("rust_style", "Prefer iterators and zero allocations")
            .with_tags(vec!["rust".to_string(), "performance".to_string()])
            .with_importance(5);

        assert_eq!(entry.category, MemoryCategory::UserPreference);
        assert_eq!(entry.key, "rust_style");
        assert_eq!(entry.importance, 5);
        assert!(entry.is_global());
        assert!(entry.matches_workspace(Some("/any/path")));
        assert!(entry.matches_query("iterators"));
        assert!(entry.matches_query("performance"));
        assert!(!entry.matches_query("nonexistent_keyword"));
    }

    #[test]
    fn test_memory_entry_temporal_decay() {
        let now = Utc::now();
        let thirty_days_ago = now - Duration::days(30);

        let fresh_entry = MemoryEntry::preference("key_fresh", "content").with_importance(3);
        let fresh_decay = fresh_entry.compute_decay_factor(now, None);
        assert!(
            (fresh_decay - 1.0).abs() < 0.05,
            "Fresh memory decay should be close to 1.0, got {}",
            fresh_decay
        );

        let mut old_entry = MemoryEntry::preference("key_old", "content")
            .with_importance(3)
            .with_created_at(thirty_days_ago.to_rfc3339())
            .with_updated_at(thirty_days_ago.to_rfc3339());
        old_entry.last_accessed_at = None;

        let old_decay = old_entry.compute_decay_factor(now, None);
        // Half life for importance 3 is 30 days -> 2^(-30/30) = 0.5
        assert!(
            (old_decay - 0.5).abs() < 0.05,
            "30-day old imp 3 memory decay should be ~0.5, got {}",
            old_decay
        );
    }

    #[test]
    fn test_memory_entry_importance_decay_resistance() {
        let now = Utc::now();
        let sixty_days_ago = now - Duration::days(60);

        let mut imp1 = MemoryEntry::preference("imp1", "minor note")
            .with_importance(1)
            .with_created_at(sixty_days_ago.to_rfc3339())
            .with_updated_at(sixty_days_ago.to_rfc3339());
        imp1.last_accessed_at = None;

        let mut imp5 = MemoryEntry::preference("imp5", "critical rule")
            .with_importance(5)
            .with_created_at(sixty_days_ago.to_rfc3339())
            .with_updated_at(sixty_days_ago.to_rfc3339());
        imp5.last_accessed_at = None;

        let decay1 = imp1.compute_decay_factor(now, None);
        let decay5 = imp5.compute_decay_factor(now, None);

        assert!(decay5 > decay1, "Critical importance 5 should resist decay much better than importance 1 (decay5: {}, decay1: {})", decay5, decay1);
    }

    #[test]
    fn test_memory_store_crud_in_memory() {
        let mut store = MemoryStore::new();
        assert_eq!(store.len(), 0);
        assert!(store.is_empty());

        // 1. Remember
        store.remember_preference("indentation", "4 spaces, no tabs");
        store.remember_convention("commit_style", "Use Conventional Commits (feat:, fix:)");
        store.remember_project_fact(
            Some("/work/fusion"),
            "core_layer",
            "src/agent is the core orchestration engine",
        );
        store.remember_tool_state(
            None,
            "browser_viewport",
            "{\"width\": 1280, \"height\": 720}",
        );

        assert_eq!(store.len(), 4);
        assert_eq!(store.preferences().len(), 1);
        assert_eq!(store.conventions().len(), 1);
        assert_eq!(store.project_facts(Some("/work/fusion")).len(), 1);
        assert_eq!(store.tool_states(None).len(), 1);

        // 2. Querying
        let pref = store.get("indentation").expect("Should find indentation");
        assert_eq!(pref.content, "4 spaces, no tabs");

        // 3. Update existing key (upsert)
        store.remember_preference("indentation", "2 spaces for yaml, 4 spaces for rust");
        assert_eq!(store.len(), 4); // Length remains 4
        let updated = store.get("indentation").unwrap();
        assert_eq!(updated.content, "2 spaces for yaml, 4 spaces for rust");

        // 4. Touch
        assert!(store.touch("indentation"));
        assert_eq!(store.get("indentation").unwrap().access_count, 1);

        // 5. Forget
        assert!(store.forget("commit_style"));
        assert_eq!(store.len(), 3);
        assert!(store.get("commit_style").is_none());

        // 6. Clear
        store.clear();
        assert_eq!(store.len(), 0);
        assert!(store.is_empty());
    }

    #[test]
    fn test_associative_fact_retrieval_and_keyword_ranking() {
        let mut store = MemoryStore::new();
        store.add(
            MemoryEntry::preference(
                "rust_async",
                "Use tokio mpsc channels for async message passing",
            )
            .with_tags(vec![
                "rust".to_string(),
                "async".to_string(),
                "tokio".to_string(),
            ])
            .with_importance(4),
        );
        store.add(
            MemoryEntry::project_fact(
                Some("/fusion"),
                "database",
                "SQLite embedded database for metadata",
            )
            .with_tags(vec!["db".to_string(), "sqlite".to_string()])
            .with_importance(3),
        );
        store.add(
            MemoryEntry::architecture_decision(
                None::<&str>,
                "error_policy",
                "Return Result with anyhow for all fallible operations",
            )
            .with_tags(vec!["rust".to_string(), "errors".to_string()])
            .with_importance(5),
        );

        // 1. Search for tokio -> should rank rust_async first
        let results = store.retrieve_associative("tokio channels", Some("/fusion"), 10);
        assert!(!results.is_empty());
        assert_eq!(results[0].entry.key, "rust_async");
        assert!(results[0].score.keyword_score > 50.0);
        assert!(results[0]
            .score
            .matched_terms
            .contains(&"tokio".to_string()));

        // 2. Search for sqlite -> should find database
        let results_db = store.retrieve_associative("sqlite", Some("/fusion"), 10);
        assert!(!results_db.is_empty());
        assert_eq!(results_db[0].entry.key, "database");

        // 3. Search for errors -> should find error_policy
        let results_err = store.retrieve_associative("errors anyhow", None, 10);
        assert!(!results_err.is_empty());
        assert_eq!(results_err[0].entry.key, "error_policy");
    }

    #[test]
    fn test_find_associated_memories() {
        let mut store = MemoryStore::new();
        store.add(
            MemoryEntry::preference("rust_style", "Write idiomatic Rust code")
                .with_tags(vec!["rust".to_string(), "idioms".to_string()])
                .with_importance(4),
        );
        store.add(
            MemoryEntry::architecture_decision(
                None::<&str>,
                "rust_errors",
                "Error handling in Rust",
            )
            .with_tags(vec!["rust".to_string(), "errors".to_string()])
            .with_importance(5),
        );
        store.add(
            MemoryEntry::project_fact(None::<&str>, "python_script", "Legacy scripts in Python")
                .with_tags(vec!["python".to_string()])
                .with_importance(2),
        );

        let associated = store.find_associated("rust_style", 5);
        assert!(!associated.is_empty());
        assert_eq!(associated[0].entry.key, "rust_errors");
        assert!(associated.iter().all(|m| m.entry.key != "rust_style"));
    }

    #[test]
    fn test_memory_store_file_persistence_roundtrip() {
        let temp_dir = tempdir().unwrap();
        let memory_file = temp_dir.path().join("sub").join("memory.json");

        let mut store = MemoryStore::new();
        store.remember_preference(
            "error_handling",
            "Use anyhow for application code and thiserror for libraries",
        );
        store.remember_convention(
            "tests",
            "Write unit tests in the same file and integration tests in tests/",
        );
        store.remember_fact(
            None,
            "wasm_support",
            "Fusion compiles to WebAssembly with wasm-bindgen",
        );
        store.remember_tool_state(None, "editor_theme", "dark_modern");

        // Save to disk
        let saved_path = store.save_to_path(&memory_file).unwrap();
        assert!(saved_path.exists());

        // Load back from disk
        let loaded = MemoryStore::load_from_path(&memory_file).unwrap();
        assert_eq!(loaded.len(), 4);

        let err_pref = loaded.get("error_handling").unwrap();
        assert_eq!(err_pref.category, MemoryCategory::UserPreference);
        assert_eq!(
            err_pref.content,
            "Use anyhow for application code and thiserror for libraries"
        );

        let fact = loaded.get("wasm_support").unwrap();
        assert_eq!(fact.category, MemoryCategory::ProjectFact);

        let tool = loaded.get("editor_theme").unwrap();
        assert_eq!(tool.category, MemoryCategory::ToolState);
        assert_eq!(tool.content, "dark_modern");
    }

    #[test]
    fn test_memory_store_json_serialization_roundtrip() {
        let mut store = MemoryStore::new();
        store.remember_preference("pref1", "content1");
        store.remember_project_fact(Some("/ws"), "fact1", "content2");

        let json_str = store.to_json().unwrap();
        let decoded = MemoryStore::from_json(&json_str).unwrap();

        assert_eq!(decoded.len(), 2);
        assert_eq!(decoded.get("pref1").unwrap().content, "content1");
        assert_eq!(decoded.get("fact1").unwrap().content, "content2");
    }

    #[test]
    fn test_memory_store_markdown_export_and_import() {
        let temp_dir = tempdir().unwrap();
        let md_path = temp_dir.path().join("export.md");

        let mut store = MemoryStore::new();
        store.add(
            MemoryEntry::preference("code_style", "Prefer functional idioms")
                .with_tags(vec!["rust".to_string()])
                .with_importance(4),
        );
        store.add(
            MemoryEntry::project_fact(None::<&str>, "server_port", "API runs on port 8080")
                .with_importance(3),
        );
        store.add(
            MemoryEntry::architecture_decision(
                None::<&str>,
                "monorepo_layout",
                "Cargo workspace layout with src/ and crates/",
            )
            .with_importance(5),
        );

        // Export markdown string
        let md_str = store.export_markdown();
        assert!(md_str.contains("# 🧠 Fusion Persistent Memory Store"));
        assert!(md_str.contains("## 🎨 User Preferences"));
        assert!(md_str.contains("Prefer functional idioms"));
        assert!(md_str.contains("API runs on port 8080"));

        // Export to file
        store.export_markdown_to_path(&md_path).unwrap();
        assert!(md_path.exists());

        // Import from markdown text
        let mut fresh_store = MemoryStore::new();
        let count = fresh_store.import_markdown(&md_str);
        assert!(count >= 3);
        assert!(fresh_store.get("code_style").is_some());
        assert!(fresh_store.get("server_port").is_some());
    }

    #[test]
    fn test_memory_store_system_prompt_formatting() {
        let mut store = MemoryStore::new();
        store.remember_preference("naming", "Use descriptive snake_case for functions");
        store.remember_project_fact(
            None,
            "data_flow",
            "Uni-directional event stream via tokio channels",
        );
        store.remember_convention(
            "git_workflow",
            "Never commit directly to main; use feature branches",
        );

        let prompt_text = store.format_for_system_prompt(None);
        assert!(prompt_text.contains("### User Coding Preferences:"));
        assert!(prompt_text.contains("- **naming**: Use descriptive snake_case for functions"));
        assert!(prompt_text.contains("### Project Architecture & Design Facts:"));
        assert!(prompt_text
            .contains("- **data_flow**: Uni-directional event stream via tokio channels"));
        assert!(prompt_text.contains("### Recurring Conventions & Guidelines:"));
        assert!(prompt_text
            .contains("- **git_workflow**: Never commit directly to main; use feature branches"));
    }

    #[test]
    fn test_memory_store_workspace_filtering() {
        let mut store = MemoryStore::new();
        store.remember_project_fact(
            Some("/projects/frontend"),
            "ui_framework",
            "React + Tailwind",
        );
        store.remember_project_fact(Some("/projects/backend"), "db_engine", "PostgreSQL + Sqlx");
        store.remember_preference("editor", "VSCode with Vim mode"); // Global

        let frontend_memories = store.filter_by_workspace(Some("/projects/frontend"));
        assert_eq!(frontend_memories.len(), 2); // ui_framework + editor (global)

        let backend_memories = store.filter_by_workspace(Some("/projects/backend"));
        assert_eq!(backend_memories.len(), 2); // db_engine + editor (global)
    }

    #[test]
    fn test_corrupted_file_recovery() {
        let temp_dir = tempdir().unwrap();
        let memory_file = temp_dir.path().join("corrupted.json");

        // Write invalid JSON
        fs::write(&memory_file, "{ invalid json ... }").unwrap();

        // Should not panic, but gracefully recover with an empty store
        let loaded = MemoryStore::load_from_path(&memory_file).unwrap();
        assert_eq!(loaded.len(), 0);
    }

    #[tokio::test]
    async fn test_memory_tool_execution() {
        let temp_dir = tempdir().unwrap();
        let tool = MemoryTool::new();
        let ctx = ToolContext {
            cwd: temp_dir.path().to_path_buf(),
            env: std::collections::HashMap::new(),
        };

        // 1. Store
        let store_args = json!({
            "action": "store",
            "category": "user_preference",
            "key": "test_pref",
            "content": "Always write doc comments",
            "importance": 4
        });
        let res = tool.execute(store_args, &ctx).await.unwrap();
        assert!(res.contains("Successfully remembered"));

        // 2. Query
        let query_args = json!({
            "action": "query",
            "query": "doc comments"
        });
        let res = tool.execute(query_args, &ctx).await.unwrap();
        assert!(res.contains("test_pref"));

        // 3. Get
        let get_args = json!({
            "action": "get",
            "key": "test_pref"
        });
        let res = tool.execute(get_args, &ctx).await.unwrap();
        assert!(res.contains("Always write doc comments"));

        // 4. Export
        let export_args = json!({
            "action": "export",
            "format": "markdown"
        });
        let res = tool.execute(export_args, &ctx).await.unwrap();
        assert!(res.contains("Fusion Persistent Memory Store"));

        // 5. Forget
        let forget_args = json!({
            "action": "forget",
            "key": "test_pref"
        });
        let res = tool.execute(forget_args, &ctx).await.unwrap();
        assert!(res.contains("Removed memory"));
    }
}
