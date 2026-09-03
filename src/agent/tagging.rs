//! Session tagging and categorization subsystem for Fusion.
//!
//! Provides a flexible, colorful, and queryable tagging mechanism for organizing,
//! categorizing, and filtering conversational sessions.
//!
//! # Capabilities
//! - Tag active or saved sessions with `/tag add <name>` (or multiple tags at once).
//! - Inspect session tags with `/tag list` (active session or global directory).
//! - Filter saved historical sessions with `/tag filter <name>`.
//! - Assign deterministic or custom ANSI colors to tags for instant visual recognition.
//! - Search, suggest, rename, and delete tags across all sessions.
//! - Persist structured tag metadata inside `Session::metadata` under `fusion:tags`.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::fmt;
use std::fs;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};

use chrono::Utc;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::agent::session::{Session, SessionSummary};
use crate::config::Config;

/// Well-known metadata key used to persist structured tags in `Session::metadata`.
pub const TAGS_METADATA_KEY: &str = "fusion:tags";

/// Fallback legacy metadata key for simple comma-separated tag strings.
pub const LEGACY_TAGS_KEY: &str = "tags";

/// Minimum permitted length of a normalized tag name.
pub const MIN_TAG_NAME_LENGTH: usize = 1;

/// Maximum permitted length of a normalized tag name.
pub const MAX_TAG_NAME_LENGTH: usize = 50;

/// Default ANSI color used when none is explicitly configured.
pub const DEFAULT_TAG_COLOR: &str = "cyan";

/// Curated color palette for deterministic tag color hashing.
pub const TAG_COLOR_PALETTE: &[&str] = &[
    "cyan",
    "magenta",
    "green",
    "yellow",
    "blue",
    "red",
    "bright_cyan",
    "bright_green",
    "bright_yellow",
    "bright_magenta",
    "bright_blue",
];

// ============================================================================
// Errors & Status
// ============================================================================

/// Error types occurring during session tagging operations.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TaggingError {
    /// Provided tag name is invalid (e.g. empty, too long, invalid characters).
    InvalidTagName(String),
    /// Requested tag was not found on the target session.
    TagNotFound(String),
    /// Tag is already present on the session.
    TagAlreadyExists(String),
    /// Target session could not be found or loaded.
    SessionNotFound(String),
    /// Failed to serialize or deserialize tag metadata.
    Serialization(String),
    /// File system or I/O error occurred while modifying saved sessions.
    Io(String),
    /// No tags were provided for a batch operation.
    EmptyTags,
}

impl fmt::Display for TaggingError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidTagName(msg) => write!(f, "Invalid tag name: {}", msg),
            Self::TagNotFound(tag) => write!(f, "Tag '{}' not found on session", tag),
            Self::TagAlreadyExists(tag) => write!(f, "Tag '{}' already exists on session", tag),
            Self::SessionNotFound(id) => write!(f, "Session '{}' not found", id),
            Self::Serialization(msg) => write!(f, "Tag serialization error: {}", msg),
            Self::Io(msg) => write!(f, "Tag I/O error: {}", msg),
            Self::EmptyTags => write!(f, "No tags provided"),
        }
    }
}

impl std::error::Error for TaggingError {}

// ============================================================================
// Tag Data Structures
// ============================================================================

/// Represents a single tag attached to a conversational session.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionTag {
    /// Normalized tag slug (lowercase, trimmed, URL/identifier safe).
    pub name: String,
    /// Optional human-friendly display label (preserves original casing).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
    /// Optional custom ANSI color label (e.g. "cyan", "magenta", "green", "blue").
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
    /// RFC 3339 timestamp when the tag was attached to the session.
    pub created_at: String,
    /// Optional descriptive note explaining the tag's purpose.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// Optional high-level category grouping (e.g. "project", "topic", "status", "feature").
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub category: Option<String>,
}

impl SessionTag {
    /// Creates a new `SessionTag` with the given name, normalized, and timestamped to now.
    pub fn new(name: impl Into<String>) -> Self {
        let raw_name = name.into();
        let normalized = normalize_tag_name(&raw_name).unwrap_or_else(|_| raw_name.to_lowercase());
        Self {
            name: normalized,
            display_name: None,
            color: None,
            created_at: Utc::now().to_rfc3339(),
            description: None,
            category: None,
        }
    }

    /// Configures an explicit ANSI color for this tag.
    pub fn with_color(mut self, color: impl Into<String>) -> Self {
        self.color = Some(color.into());
        self
    }

    /// Configures an explanatory description for this tag.
    pub fn with_description(mut self, desc: impl Into<String>) -> Self {
        self.description = Some(desc.into());
        self
    }

    /// Configures a categorical grouping.
    pub fn with_category(mut self, cat: impl Into<String>) -> Self {
        self.category = Some(cat.into());
        self
    }

    /// Configures an explicit display name.
    pub fn with_display_name(mut self, display_name: impl Into<String>) -> Self {
        self.display_name = Some(display_name.into());
        self
    }

    /// Returns the effective color of this tag, falling back to deterministic hashing.
    pub fn effective_color(&self) -> &str {
        self.color
            .as_deref()
            .unwrap_or_else(|| deterministic_tag_color(&self.name))
    }

    /// Returns ANSI foreground code for the tag's color.
    pub fn ansi_fg(&self) -> &'static str {
        tag_ansi_fg(self.effective_color())
    }

    /// Returns ANSI background code for the tag's color.
    pub fn ansi_bg(&self) -> &'static str {
        tag_ansi_bg(self.effective_color())
    }

    /// Renders a rich ANSI badge for terminal display (e.g. `\x1b[36m#rust\x1b[0m`).
    pub fn display_badge(&self) -> String {
        let label = self.display_name.as_deref().unwrap_or(&self.name);
        format!("{}{}{}\x1b[0m", self.ansi_fg(), "\x1b[1m#", label)
    }

    /// Renders a filled pill-style ANSI badge (e.g. `[ rust ]`).
    pub fn display_pill(&self) -> String {
        let label = self.display_name.as_deref().unwrap_or(&self.name);
        format!("{}\x1b[30;1m {} \x1b[0m", self.ansi_bg(), label)
    }

    /// Formats a concise string representation of the tag.
    pub fn format_short(&self) -> String {
        format!("#{}", self.name)
    }
}

/// In-memory collection of tags attached to a session.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionTagCollection {
    pub tags: Vec<SessionTag>,
}

impl SessionTagCollection {
    /// Creates an empty collection.
    pub fn new() -> Self {
        Self { tags: Vec::new() }
    }

    /// Adds a tag to the collection. Returns `true` if added, `false` if updated.
    pub fn add(&mut self, tag: SessionTag) -> bool {
        if let Some(existing) = self.tags.iter_mut().find(|t| t.name == tag.name) {
            *existing = tag;
            false
        } else {
            self.tags.push(tag);
            true
        }
    }

    /// Removes a tag by name. Returns `true` if a tag was removed.
    pub fn remove(&mut self, name: &str) -> bool {
        let norm = normalize_tag_name(name).unwrap_or_else(|_| name.to_lowercase());
        let initial_len = self.tags.len();
        self.tags.retain(|t| t.name != norm);
        self.tags.len() < initial_len
    }

    /// Checks if a tag is present.
    pub fn has(&self, name: &str) -> bool {
        let norm = normalize_tag_name(name).unwrap_or_else(|_| name.to_lowercase());
        self.tags.iter().any(|t| t.name == norm)
    }

    /// Gets a reference to a tag by name.
    pub fn get(&self, name: &str) -> Option<&SessionTag> {
        let norm = normalize_tag_name(name).unwrap_or_else(|_| name.to_lowercase());
        self.tags.iter().find(|t| t.name == norm)
    }

    /// Returns list of normalized tag names.
    pub fn tag_names(&self) -> Vec<String> {
        self.tags.iter().map(|t| t.name.clone()).collect()
    }

    /// Returns true if empty.
    pub fn is_empty(&self) -> bool {
        self.tags.is_empty()
    }

    /// Returns number of tags.
    pub fn len(&self) -> usize {
        self.tags.len()
    }

    /// Clears all tags.
    pub fn clear(&mut self) {
        self.tags.clear();
    }
}

/// Lightweight summary of a session including attached tags.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TaggedSessionSummary {
    /// Session UUID.
    pub id: Uuid,
    /// User-defined or auto-generated session title.
    pub title: Option<String>,
    /// RFC 3339 creation timestamp.
    pub created_at: String,
    /// RFC 3339 last update timestamp.
    pub updated_at: String,
    /// Active LLM model identifier.
    pub active_model: String,
    /// Total conversation message count.
    pub message_count: usize,
    /// Truncated snippet of the last user/assistant message.
    pub preview: String,
    /// List of tags attached to this session.
    pub tags: Vec<SessionTag>,
}

impl TaggedSessionSummary {
    /// Checks if this session has a tag by name.
    pub fn has_tag(&self, name: &str) -> bool {
        let norm = normalize_tag_name(name).unwrap_or_else(|_| name.to_lowercase());
        self.tags.iter().any(|t| t.name == norm)
    }

    /// Checks if this session contains all of the specified tags (AND matching).
    pub fn has_all_tags(&self, names: &[&str]) -> bool {
        names.iter().all(|&name| self.has_tag(name))
    }

    /// Checks if this session contains at least one of the specified tags (OR matching).
    pub fn has_any_tag(&self, names: &[&str]) -> bool {
        names.iter().any(|&name| self.has_tag(name))
    }

    /// Returns the list of normalized tag names for this session.
    pub fn tag_names(&self) -> Vec<String> {
        self.tags.iter().map(|t| t.name.clone()).collect()
    }
}

/// Aggregated frequency information for a single tag across historical sessions.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TagFrequency {
    /// Tag slug name.
    pub tag: String,
    /// Number of sessions tagged with this tag.
    pub count: usize,
    /// ANSI color associated with the tag.
    pub color: Option<String>,
    /// Session UUIDs associated with this tag.
    pub session_ids: Vec<Uuid>,
    /// Most recent update timestamp of any session with this tag.
    pub last_used: String,
}

/// Filtering mode for multi-tag queries.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TagFilterMode {
    /// Match sessions having at least one of the specified tags (OR).
    Any,
    /// Match sessions having all of the specified tags (AND).
    All,
    /// Match sessions having exactly the specified tags.
    Exact,
}

/// Multi-criteria search query for filtering tagged sessions.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TagFilterQuery {
    /// List of tag names to filter on.
    pub tags: Vec<String>,
    /// Filter mode (Any, All, Exact).
    pub mode: TagFilterMode,
    /// Optional model filter.
    pub model: Option<String>,
    /// Optional full-text substring search on title / preview.
    pub query_text: Option<String>,
    /// Optional limit on the number of returned sessions.
    pub limit: Option<usize>,
}

impl TagFilterQuery {
    /// Creates a query for a single tag.
    pub fn single(tag: impl Into<String>) -> Self {
        Self {
            tags: vec![tag.into()],
            mode: TagFilterMode::Any,
            model: None,
            query_text: None,
            limit: None,
        }
    }

    /// Creates a query matching any of the given tags (OR).
    pub fn any_of(tags: Vec<String>) -> Self {
        Self {
            tags,
            mode: TagFilterMode::Any,
            model: None,
            query_text: None,
            limit: None,
        }
    }

    /// Creates a query matching all of the given tags (AND).
    pub fn all_of(tags: Vec<String>) -> Self {
        Self {
            tags,
            mode: TagFilterMode::All,
            model: None,
            query_text: None,
            limit: None,
        }
    }

    /// Sets an optional model filter.
    pub fn with_model(mut self, model: impl Into<String>) -> Self {
        self.model = Some(model.into());
        self
    }

    /// Sets an optional query text filter.
    pub fn with_query_text(mut self, text: impl Into<String>) -> Self {
        self.query_text = Some(text.into());
        self
    }

    /// Sets an optional result count limit.
    pub fn with_limit(mut self, limit: usize) -> Self {
        self.limit = Some(limit);
        self
    }
}

/// Comprehensive report of tag usage statistics across all saved sessions.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TagStatsReport {
    /// Total number of unique tags across all sessions.
    pub total_unique_tags: usize,
    /// Total number of sessions that have at least one tag.
    pub total_tagged_sessions: usize,
    /// Total number of sessions scanned.
    pub total_sessions_scanned: usize,
    /// All tag frequencies sorted by usage count descending.
    pub tag_frequencies: Vec<TagFrequency>,
    /// Top most used tags.
    pub top_tags: Vec<TagFrequency>,
    /// Most recently updated tags.
    pub recent_tags: Vec<TagFrequency>,
}

// ============================================================================
// Normalization, Validation & Color Utilities
// ============================================================================

/// Normalizes and validates a tag name string.
///
/// Strips leading `#`, trims whitespace, converts to lowercase, and verifies
/// that the tag contains valid identifier characters.
pub fn normalize_tag_name(name: &str) -> Result<String, TaggingError> {
    let trimmed = name.trim();
    let stripped = trimmed.strip_prefix('#').unwrap_or(trimmed).trim();

    if stripped.len() < MIN_TAG_NAME_LENGTH {
        return Err(TaggingError::InvalidTagName(
            "Tag name cannot be empty".to_string(),
        ));
    }

    if stripped.len() > MAX_TAG_NAME_LENGTH {
        return Err(TaggingError::InvalidTagName(format!(
            "Tag name exceeds maximum length of {} characters (got {})",
            MAX_TAG_NAME_LENGTH,
            stripped.len()
        )));
    }

    // Check allowed characters: alphanumeric, dash, underscore, colon, dot, slash
    for ch in stripped.chars() {
        if !ch.is_alphanumeric() && ch != '-' && ch != '_' && ch != ':' && ch != '.' && ch != '/' {
            return Err(TaggingError::InvalidTagName(format!(
                "Tag name contains invalid character '{}'. Allowed: alphanumeric, -, _, :, ., /",
                ch
            )));
        }
    }

    Ok(stripped.to_lowercase())
}

/// Validates a tag name without returning the normalized string.
pub fn validate_tag_name(name: &str) -> Result<(), TaggingError> {
    normalize_tag_name(name).map(|_| ())
}

/// Computes a deterministic ANSI color for a tag name based on its hash.
pub fn deterministic_tag_color(tag_name: &str) -> &'static str {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    tag_name.hash(&mut hasher);
    let hash = hasher.finish() as usize;
    TAG_COLOR_PALETTE[hash % TAG_COLOR_PALETTE.len()]
}

/// Returns ANSI foreground escape code for a named color.
pub fn tag_ansi_fg(color: &str) -> &'static str {
    match color.to_lowercase().as_str() {
        "cyan" => "\x1b[36m",
        "magenta" | "purple" => "\x1b[35m",
        "green" => "\x1b[32m",
        "yellow" => "\x1b[33m",
        "blue" => "\x1b[34m",
        "red" => "\x1b[31m",
        "bright_cyan" => "\x1b[96m",
        "bright_green" => "\x1b[92m",
        "bright_yellow" => "\x1b[93m",
        "bright_magenta" => "\x1b[95m",
        "bright_blue" => "\x1b[94m",
        "white" => "\x1b[37m",
        "gray" | "grey" => "\x1b[90m",
        _ => "\x1b[36m", // default cyan
    }
}

/// Returns ANSI background escape code for a named color.
pub fn tag_ansi_bg(color: &str) -> &'static str {
    match color.to_lowercase().as_str() {
        "cyan" => "\x1b[46m",
        "magenta" | "purple" => "\x1b[45m",
        "green" => "\x1b[42m",
        "yellow" => "\x1b[43m",
        "blue" => "\x1b[44m",
        "red" => "\x1b[41m",
        "bright_cyan" => "\x1b[106m",
        "bright_green" => "\x1b[102m",
        "bright_yellow" => "\x1b[103m",
        "bright_magenta" => "\x1b[105m",
        "bright_blue" => "\x1b[104m",
        "white" => "\x1b[47m",
        "gray" | "grey" => "\x1b[100m",
        _ => "\x1b[46m",
    }
}

// ============================================================================
// Session Metadata Operations
// ============================================================================

/// Retrieves all structured tags attached to a `Session`.
///
/// Supports parsing from primary `fusion:tags` metadata key, with fallbacks to
/// JSON arrays of strings or legacy comma-separated tag lists.
pub fn get_session_tags(session: &Session) -> Vec<SessionTag> {
    extract_tags_from_session(session)
}

/// Extracts tags from a session with multi-format fallback parsing.
pub fn extract_tags_from_session(session: &Session) -> Vec<SessionTag> {
    // 1. Primary: JSON Vec<SessionTag> in "fusion:tags"
    if let Some(json_str) = session.get_metadata(TAGS_METADATA_KEY) {
        if let Ok(tags) = serde_json::from_str::<Vec<SessionTag>>(json_str) {
            return tags;
        }
        // Fallback: JSON Vec<String>
        if let Ok(raw_names) = serde_json::from_str::<Vec<String>>(json_str) {
            return raw_names.into_iter().map(SessionTag::new).collect();
        }
    }

    // 2. Legacy fallback: comma-separated list in "tags" metadata key
    if let Some(legacy_str) = session.get_metadata(LEGACY_TAGS_KEY) {
        let mut tags = Vec::new();
        for item in legacy_str.split(',') {
            let trimmed = item.trim();
            if !trimmed.is_empty() {
                tags.push(SessionTag::new(trimmed));
            }
        }
        if !tags.is_empty() {
            return tags;
        }
    }

    Vec::new()
}

/// Returns the normalized tag names attached to a session.
pub fn get_session_tag_names(session: &Session) -> Vec<String> {
    get_session_tags(session)
        .into_iter()
        .map(|t| t.name)
        .collect()
}

/// Checks if a session is tagged with the given tag name.
pub fn has_tag(session: &Session, tag_name: &str) -> bool {
    if let Ok(normalized) = normalize_tag_name(tag_name) {
        get_session_tags(session)
            .iter()
            .any(|t| t.name == normalized)
    } else {
        false
    }
}

/// Overwrites the full list of tags on a session and updates its timestamp.
pub fn set_session_tags(session: &mut Session, tags: &[SessionTag]) -> Result<(), TaggingError> {
    if tags.is_empty() {
        session.metadata.remove(TAGS_METADATA_KEY);
        session.metadata.remove(LEGACY_TAGS_KEY);
    } else {
        let json_str =
            serde_json::to_string(tags).map_err(|e| TaggingError::Serialization(e.to_string()))?;
        session.set_metadata(TAGS_METADATA_KEY, json_str);
    }
    session.touch();
    Ok(())
}

/// Adds a single tag to a session.
pub fn add_tag(session: &mut Session, tag_name: &str) -> Result<SessionTag, TaggingError> {
    add_tag_with_details(session, tag_name, None, None, None)
}

/// Adds a tag with optional custom color, description, and category.
pub fn add_tag_with_details(
    session: &mut Session,
    tag_name: &str,
    color: Option<&str>,
    desc: Option<&str>,
    cat: Option<&str>,
) -> Result<SessionTag, TaggingError> {
    let normalized = normalize_tag_name(tag_name)?;
    let mut tags = get_session_tags(session);

    if tags.iter().any(|t| t.name == normalized) {
        return Err(TaggingError::TagAlreadyExists(normalized));
    }

    let mut new_tag = SessionTag::new(&normalized);
    if let Some(c) = color {
        new_tag = new_tag.with_color(c);
    }
    if let Some(d) = desc {
        new_tag = new_tag.with_description(d);
    }
    if let Some(cat_name) = cat {
        new_tag = new_tag.with_category(cat_name);
    }

    tags.push(new_tag.clone());
    set_session_tags(session, &tags)?;
    Ok(new_tag)
}

/// Adds multiple tags to a session in one batch. Skips duplicates.
pub fn add_tags(
    session: &mut Session,
    tag_names: &[&str],
) -> Result<Vec<SessionTag>, TaggingError> {
    if tag_names.is_empty() {
        return Err(TaggingError::EmptyTags);
    }

    let mut tags = get_session_tags(session);
    let mut added = Vec::new();

    for &raw_name in tag_names {
        let normalized = normalize_tag_name(raw_name)?;
        if !tags.iter().any(|t| t.name == normalized) {
            let tag = SessionTag::new(&normalized);
            tags.push(tag.clone());
            added.push(tag);
        }
    }

    if !added.is_empty() {
        set_session_tags(session, &tags)?;
    }

    Ok(added)
}

/// Removes a tag from a session by name. Returns `true` if a tag was removed.
pub fn remove_tag(session: &mut Session, tag_name: &str) -> Result<bool, TaggingError> {
    let normalized = normalize_tag_name(tag_name)?;
    let mut tags = get_session_tags(session);
    let initial_len = tags.len();

    tags.retain(|t| t.name != normalized);

    if tags.len() < initial_len {
        set_session_tags(session, &tags)?;
        Ok(true)
    } else {
        Err(TaggingError::TagNotFound(normalized))
    }
}

/// Removes multiple tags from a session. Returns the number of tags removed.
pub fn remove_tags(session: &mut Session, tag_names: &[&str]) -> Result<usize, TaggingError> {
    let mut tags = get_session_tags(session);
    let initial_len = tags.len();

    let to_remove: HashSet<String> = tag_names
        .iter()
        .filter_map(|&name| normalize_tag_name(name).ok())
        .collect();

    tags.retain(|t| !to_remove.contains(&t.name));
    let removed_count = initial_len - tags.len();

    if removed_count > 0 {
        set_session_tags(session, &tags)?;
    }

    Ok(removed_count)
}

/// Clears all tags from a session. Returns the number of removed tags.
pub fn clear_tags(session: &mut Session) -> usize {
    let count = get_session_tags(session).len();
    session.metadata.remove(TAGS_METADATA_KEY);
    session.metadata.remove(LEGACY_TAGS_KEY);
    session.touch();
    count
}

/// Renames a tag on the active session.
pub fn rename_tag(
    session: &mut Session,
    old_name: &str,
    new_name: &str,
) -> Result<bool, TaggingError> {
    let old_norm = normalize_tag_name(old_name)?;
    let new_norm = normalize_tag_name(new_name)?;

    let mut tags = get_session_tags(session);
    let mut modified = false;

    // Verify new tag does not already exist (unless it's the same normalized name)
    if old_norm != new_norm && tags.iter().any(|t| t.name == new_norm) {
        return Err(TaggingError::TagAlreadyExists(new_norm));
    }

    for tag in tags.iter_mut() {
        if tag.name == old_norm {
            tag.name = new_norm.clone();
            tag.display_name = Some(new_name.trim().to_string());
            modified = true;
            break;
        }
    }

    if modified {
        set_session_tags(session, &tags)?;
        Ok(true)
    } else {
        Err(TaggingError::TagNotFound(old_norm))
    }
}

// ============================================================================
// Multi-Session & Disk Management (`SessionTagManager`)
// ============================================================================

/// Session tag manager responsible for disk-level queries, global listings,
/// cross-session filtering, and bulk tag management.
#[derive(Debug, Clone)]
pub struct SessionTagManager {
    sessions_dir: PathBuf,
}

impl Default for SessionTagManager {
    fn default() -> Self {
        Self::new()
    }
}

impl SessionTagManager {
    /// Creates a new manager targeting the default Fusion sessions directory (`~/.fusion/sessions`).
    pub fn new() -> Self {
        Self {
            sessions_dir: Session::sessions_dir(),
        }
    }

    /// Creates a manager targeting a custom sessions directory.
    pub fn with_dir(dir: impl Into<PathBuf>) -> Self {
        Self {
            sessions_dir: dir.into(),
        }
    }

    /// Returns the active sessions directory.
    pub fn sessions_dir(&self) -> &Path {
        &self.sessions_dir
    }

    /// Scans all saved sessions in the directory and returns all unique tags with frequencies.
    pub fn list_all_tags(&self) -> anyhow::Result<Vec<TagFrequency>> {
        list_all_tags_in_dir(&self.sessions_dir)
    }

    /// Filters saved sessions matching the specified tag name.
    pub fn filter_sessions(&self, tag_name: &str) -> anyhow::Result<Vec<TaggedSessionSummary>> {
        filter_sessions_in_dir(&self.sessions_dir, tag_name)
    }

    /// Advanced multi-criteria search and filter across saved sessions.
    pub fn filter_sessions_advanced(
        &self,
        query: &TagFilterQuery,
    ) -> anyhow::Result<Vec<TaggedSessionSummary>> {
        filter_sessions_multi(&self.sessions_dir, query)
    }

    /// Loads a saved session by ID, adds a tag, and saves it back to disk.
    pub fn tag_session(&self, session_id: Uuid, tag_name: &str) -> anyhow::Result<SessionTag> {
        let path = self.sessions_dir.join(format!("{}.json", session_id));
        if !path.exists() {
            anyhow::bail!("Session not found: {}", session_id);
        }
        let mut session = Session::load_from_path(&path)?;
        let tag = add_tag(&mut session, tag_name).map_err(|e| anyhow::anyhow!("{}", e))?;
        session.save_to_path(&path)?;
        Ok(tag)
    }

    /// Loads a saved session by ID, removes a tag, and saves it back to disk.
    pub fn untag_session(&self, session_id: Uuid, tag_name: &str) -> anyhow::Result<bool> {
        let path = self.sessions_dir.join(format!("{}.json", session_id));
        if !path.exists() {
            anyhow::bail!("Session not found: {}", session_id);
        }
        let mut session = Session::load_from_path(&path)?;
        let removed = remove_tag(&mut session, tag_name).map_err(|e| anyhow::anyhow!("{}", e))?;
        session.save_to_path(&path)?;
        Ok(removed)
    }

    /// Renames a tag globally across all saved sessions in the directory.
    pub fn rename_tag_globally(&self, old_tag: &str, new_tag: &str) -> anyhow::Result<usize> {
        let old_norm = normalize_tag_name(old_tag).map_err(|e| anyhow::anyhow!("{}", e))?;
        let new_norm = normalize_tag_name(new_tag).map_err(|e| anyhow::anyhow!("{}", e))?;

        if !self.sessions_dir.exists() {
            return Ok(0);
        }

        let mut modified_count = 0;
        for entry in fs::read_dir(&self.sessions_dir)? {
            let entry = entry?;
            let path = entry.path();
            if path.extension().and_then(|s| s.to_str()) == Some("json") {
                if let Ok(mut session) = Session::load_from_path(&path) {
                    let mut tags = get_session_tags(&session);
                    let mut changed = false;

                    for tag in tags.iter_mut() {
                        if tag.name == old_norm {
                            tag.name = new_norm.clone();
                            tag.display_name = Some(new_tag.trim().to_string());
                            changed = true;
                        }
                    }

                    if changed {
                        set_session_tags(&mut session, &tags)
                            .map_err(|e| anyhow::anyhow!("{}", e))?;
                        session.save_to_path(&path)?;
                        modified_count += 1;
                    }
                }
            }
        }

        Ok(modified_count)
    }

    /// Deletes a tag globally across all saved sessions in the directory.
    pub fn delete_tag_globally(&self, tag_name: &str) -> anyhow::Result<usize> {
        let norm = normalize_tag_name(tag_name).map_err(|e| anyhow::anyhow!("{}", e))?;

        if !self.sessions_dir.exists() {
            return Ok(0);
        }

        let mut modified_count = 0;
        for entry in fs::read_dir(&self.sessions_dir)? {
            let entry = entry?;
            let path = entry.path();
            if path.extension().and_then(|s| s.to_str()) == Some("json") {
                if let Ok(mut session) = Session::load_from_path(&path) {
                    let mut tags = get_session_tags(&session);
                    let initial_len = tags.len();
                    tags.retain(|t| t.name != norm);

                    if tags.len() < initial_len {
                        set_session_tags(&mut session, &tags)
                            .map_err(|e| anyhow::anyhow!("{}", e))?;
                        session.save_to_path(&path)?;
                        modified_count += 1;
                    }
                }
            }
        }

        Ok(modified_count)
    }

    /// Suggests tag names matching a prefix for autocompletion.
    pub fn suggest_tags(&self, prefix: &str, limit: usize) -> anyhow::Result<Vec<String>> {
        let prefix_norm = prefix
            .trim()
            .strip_prefix('#')
            .unwrap_or(prefix)
            .to_lowercase();
        let freqs = self.list_all_tags()?;
        let suggestions: Vec<String> = freqs
            .into_iter()
            .filter(|f| f.tag.starts_with(&prefix_norm) || f.tag.contains(&prefix_norm))
            .take(limit)
            .map(|f| f.tag)
            .collect();
        Ok(suggestions)
    }

    /// Computes full tag statistics across all saved sessions.
    pub fn tag_stats(&self) -> anyhow::Result<TagStatsReport> {
        let freqs = self.list_all_tags()?;
        let mut total_tagged_sessions_set = HashSet::new();
        let mut total_scanned = 0;

        if self.sessions_dir.exists() {
            for entry in fs::read_dir(&self.sessions_dir)? {
                let entry = entry?;
                if entry.path().extension().and_then(|s| s.to_str()) == Some("json") {
                    total_scanned += 1;
                }
            }
        }

        for f in &freqs {
            for &id in &f.session_ids {
                total_tagged_sessions_set.insert(id);
            }
        }

        let total_unique_tags = freqs.len();
        let total_tagged_sessions = total_tagged_sessions_set.len();

        let mut top_tags = freqs.clone();
        top_tags.sort_by(|a, b| b.count.cmp(&a.count));
        top_tags.truncate(10);

        let mut recent_tags = freqs.clone();
        recent_tags.sort_by(|a, b| b.last_used.cmp(&a.last_used));
        recent_tags.truncate(10);

        Ok(TagStatsReport {
            total_unique_tags,
            total_tagged_sessions,
            total_sessions_scanned: total_scanned,
            tag_frequencies: freqs,
            top_tags,
            recent_tags,
        })
    }
}

// ============================================================================
// Standalone Functions
// ============================================================================

/// Lists all unique tags and usage frequencies from the default sessions directory.
pub fn list_all_tags() -> anyhow::Result<Vec<TagFrequency>> {
    list_all_tags_in_dir(&Session::sessions_dir())
}

/// Lists all unique tags and usage frequencies from a specific directory.
pub fn list_all_tags_in_dir(dir: &Path) -> anyhow::Result<Vec<TagFrequency>> {
    if !dir.exists() {
        return Ok(Vec::new());
    }

    let mut tag_map: BTreeMap<String, (usize, Option<String>, Vec<Uuid>, String)> = BTreeMap::new();

    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.extension().and_then(|s| s.to_str()) == Some("json") {
            if let Ok(session) = Session::load_from_path(&path) {
                let tags = get_session_tags(&session);
                for tag in tags {
                    let entry = tag_map.entry(tag.name).or_insert((
                        0,
                        tag.color,
                        Vec::new(),
                        session.updated_at.clone(),
                    ));
                    entry.0 += 1;
                    entry.2.push(session.id);
                    if session.updated_at > entry.3 {
                        entry.3 = session.updated_at.clone();
                    }
                }
            }
        }
    }

    let mut result: Vec<TagFrequency> = tag_map
        .into_iter()
        .map(
            |(tag, (count, color, session_ids, last_used))| TagFrequency {
                tag,
                count,
                color,
                session_ids,
                last_used,
            },
        )
        .collect();

    // Sort by count descending, then alphabetical
    result.sort_by(|a, b| b.count.cmp(&a.count).then_with(|| a.tag.cmp(&b.tag)));
    Ok(result)
}

/// Filters saved sessions matching a tag name from the default directory.
pub fn filter_sessions_by_tag(tag_name: &str) -> anyhow::Result<Vec<TaggedSessionSummary>> {
    filter_sessions_in_dir(&Session::sessions_dir(), tag_name)
}

/// Filters saved sessions matching a tag name from a specific directory.
pub fn filter_sessions_in_dir(
    dir: &Path,
    tag_name: &str,
) -> anyhow::Result<Vec<TaggedSessionSummary>> {
    let normalized = normalize_tag_name(tag_name).map_err(|e| anyhow::anyhow!("{}", e))?;

    let query = TagFilterQuery::single(normalized);
    filter_sessions_multi(dir, &query)
}

/// Advanced multi-criteria search and filter in a specific directory.
pub fn filter_sessions_multi(
    dir: &Path,
    query: &TagFilterQuery,
) -> anyhow::Result<Vec<TaggedSessionSummary>> {
    if !dir.exists() {
        return Ok(Vec::new());
    }

    let normalized_tags: Vec<String> = query
        .tags
        .iter()
        .filter_map(|t| normalize_tag_name(t).ok())
        .collect();

    let mut matches = Vec::new();

    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.extension().and_then(|s| s.to_str()) == Some("json") {
            if let Ok(session) = Session::load_from_path(&path) {
                let tags = get_session_tags(&session);
                let tag_names: HashSet<String> = tags.iter().map(|t| t.name.clone()).collect();

                // Check tag filter
                let tag_match = if normalized_tags.is_empty() {
                    true
                } else {
                    match query.mode {
                        TagFilterMode::Any => {
                            normalized_tags.iter().any(|req| tag_names.contains(req))
                        }
                        TagFilterMode::All => {
                            normalized_tags.iter().all(|req| tag_names.contains(req))
                        }
                        TagFilterMode::Exact => {
                            let req_set: HashSet<String> =
                                normalized_tags.iter().cloned().collect();
                            tag_names == req_set
                        }
                    }
                };

                if !tag_match {
                    continue;
                }

                // Check model filter
                if let Some(req_model) = &query.model {
                    if !session
                        .active_model
                        .to_lowercase()
                        .contains(&req_model.to_lowercase())
                    {
                        continue;
                    }
                }

                // Check text query filter
                if let Some(text) = &query.query_text {
                    let text_lower = text.to_lowercase();
                    let title_match = session
                        .title
                        .as_deref()
                        .map(|t| t.to_lowercase().contains(&text_lower))
                        .unwrap_or(false);
                    let msg_match = session
                        .messages
                        .iter()
                        .any(|m| m.content.to_lowercase().contains(&text_lower));

                    if !title_match && !msg_match {
                        continue;
                    }
                }

                // Extract preview
                let preview = session
                    .messages
                    .iter()
                    .rev()
                    .find(|m| {
                        m.role == crate::provider::types::Role::User
                            || m.role == crate::provider::types::Role::Assistant
                    })
                    .map(|m| {
                        let mut p: String = m.content.chars().take(80).collect();
                        if m.content.chars().count() > 80 {
                            p.push_str("...");
                        }
                        p
                    })
                    .unwrap_or_else(|| "Empty session".to_string());

                matches.push(TaggedSessionSummary {
                    id: session.id,
                    title: session.title,
                    created_at: session.created_at,
                    updated_at: session.updated_at,
                    active_model: session.active_model,
                    message_count: session.messages.len(),
                    preview,
                    tags,
                });
            }
        }
    }

    // Sort descending by updated_at (most recent first)
    matches.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));

    if let Some(limit) = query.limit {
        matches.truncate(limit);
    }

    Ok(matches)
}

/// Tags a saved session on disk by UUID.
pub fn tag_saved_session(session_id: Uuid, tag_name: &str) -> anyhow::Result<SessionTag> {
    SessionTagManager::new().tag_session(session_id, tag_name)
}

/// Untags a saved session on disk by UUID.
pub fn untag_saved_session(session_id: Uuid, tag_name: &str) -> anyhow::Result<bool> {
    SessionTagManager::new().untag_session(session_id, tag_name)
}

// ============================================================================
// Slash Command Handler (`/tag`)
// ============================================================================

/// Top-level command handler for interactive `/tag` slash commands.
///
/// Supported subcommands:
/// - `/tag` (or `/tag help`) -> Show active tags and usage help.
/// - `/tag add <name> [tag2...] [--color <color>] [--desc <desc>]` -> Add tag(s) to active session.
/// - `/tag remove <name>` / `/tag rm <name>` -> Remove tag from active session.
/// - `/tag list` (or `/tag list --all`) -> List active tags or all global tags on disk.
/// - `/tag filter <name>` (or `/tag find <name>`) -> Filter historical sessions by tag.
/// - `/tag clear` -> Clear all tags from active session.
/// - `/tag rename <old> <new> [--global]` -> Rename a tag in active session or globally.
/// - `/tag stats` -> Display tag usage analytics and frequencies.
/// - `/tag set <tag1> <tag2> ...` -> Replace all tags on active session.
/// - `/tag session <id> <add|rm|list> <tag>` -> Manage tags for another session on disk.
pub fn handle_tag_command(args: &[String], session: &mut Session) -> String {
    if args.is_empty() {
        let active_tags = get_session_tags(session);
        if active_tags.is_empty() {
            return format_tag_welcome_and_help();
        } else {
            return format_active_session_tags(session);
        }
    }

    let subcmd = args[0].to_lowercase();
    match subcmd.as_str() {
        "help" | "-h" | "--help" => format_tag_help(),

        "add" | "+" | "new" => {
            if args.len() < 2 {
                return "\x1b[1;31mUsage:\x1b[0m /tag add <name> [more_tags...] [--color <color>] [--desc <description>]".to_string();
            }

            let mut tag_names = Vec::new();
            let mut custom_color: Option<String> = None;
            let mut custom_desc: Option<String> = None;
            let mut custom_cat: Option<String> = None;

            let mut idx = 1;
            while idx < args.len() {
                match args[idx].as_str() {
                    "--color" | "-c" if idx + 1 < args.len() => {
                        custom_color = Some(args[idx + 1].clone());
                        idx += 2;
                    }
                    "--desc" | "-d" if idx + 1 < args.len() => {
                        custom_desc = Some(args[idx + 1].clone());
                        idx += 2;
                    }
                    "--cat" if idx + 1 < args.len() => {
                        custom_cat = Some(args[idx + 1].clone());
                        idx += 2;
                    }
                    _ => {
                        tag_names.push(&args[idx]);
                        idx += 1;
                    }
                }
            }

            if tag_names.is_empty() {
                return "\x1b[1;31mError:\x1b[0m No tag names specified.".to_string();
            }

            let mut added_badges = Vec::new();
            let mut errors = Vec::new();

            for &raw_name in &tag_names {
                match add_tag_with_details(
                    session,
                    raw_name,
                    custom_color.as_deref(),
                    custom_desc.as_deref(),
                    custom_cat.as_deref(),
                ) {
                    Ok(tag) => added_badges.push(tag.display_badge()),
                    Err(TaggingError::TagAlreadyExists(name)) => {
                        errors.push(format!("Tag '{}' is already attached", name));
                    }
                    Err(e) => errors.push(e.to_string()),
                }
            }

            let mut out = String::new();
            if !added_badges.is_empty() {
                out.push_str(&format!(
                    "\x1b[1;32mTagged active session:\x1b[0m {}\n",
                    added_badges.join(" ")
                ));
            }
            if !errors.is_empty() {
                for err in errors {
                    out.push_str(&format!("\x1b[1;33mNotice:\x1b[0m {}\n", err));
                }
            }
            if out.is_empty() {
                out.push_str("\x1b[1;33mNo tags added.\x1b[0m\n");
            }
            out.trim_end().to_string()
        }

        "remove" | "rm" | "del" | "delete" | "-" | "untag" => {
            if args.len() < 2 {
                return "\x1b[1;31mUsage:\x1b[0m /tag remove <name> [more_tags...]".to_string();
            }

            let mut removed_count = 0;
            let mut not_found = Vec::new();

            for tag_name in &args[1..] {
                match remove_tag(session, tag_name) {
                    Ok(_) => removed_count += 1,
                    Err(TaggingError::TagNotFound(name)) => not_found.push(name),
                    Err(e) => not_found.push(format!("{}: {}", tag_name, e)),
                }
            }

            let mut out = String::new();
            if removed_count > 0 {
                out.push_str(&format!(
                    "\x1b[1;32mRemoved {} tag(s) from active session.\x1b[0m\n",
                    removed_count
                ));
            }
            if !not_found.is_empty() {
                out.push_str(&format!(
                    "\x1b[1;33mNot found:\x1b[0m {}\n",
                    not_found.join(", ")
                ));
            }
            out.trim_end().to_string()
        }

        "list" | "ls" | "show" => {
            let is_all = args
                .iter()
                .skip(1)
                .any(|a| a == "--all" || a == "-a" || a == "all" || a == "global");

            if is_all {
                match list_all_tags() {
                    Ok(freqs) => format_global_tags_table(&freqs),
                    Err(e) => format!("\x1b[1;31mFailed to list tags:\x1b[0m {}", e),
                }
            } else {
                let tags = get_session_tags(session);
                if tags.is_empty() {
                    let mut out = "\x1b[1;33mActive session has no tags.\x1b[0m\n".to_string();
                    out.push_str("Add tags with \x1b[1m/tag add <name>\x1b[0m or view all saved tags with \x1b[1m/tag list --all\x1b[0m\n");
                    if let Ok(freqs) = list_all_tags() {
                        if !freqs.is_empty() {
                            out.push_str("\n\x1b[1mTop Tags in Historical Sessions:\x1b[0m\n");
                            let badges: Vec<String> = freqs
                                .iter()
                                .take(8)
                                .map(|f| {
                                    let badge = format_tag_badge_str(&f.tag, f.color.as_deref());
                                    format!("{} ({})", badge, f.count)
                                })
                                .collect();
                            out.push_str(&format!("  {}\n", badges.join("  ")));
                        }
                    }
                    out.trim_end().to_string()
                } else {
                    format_active_session_tags(session)
                }
            }
        }

        "filter" | "find" | "search" | "query" => {
            if args.len() < 2 {
                return "\x1b[1;31mUsage:\x1b[0m /tag filter <name> [tag2...] [--all]".to_string();
            }

            let mut filter_tags = Vec::new();
            let mut match_all = false;

            for arg in &args[1..] {
                if arg == "--all" || arg == "-a" || arg == "--and" {
                    match_all = true;
                } else if !arg.starts_with('-') {
                    filter_tags.push(arg.clone());
                }
            }

            if filter_tags.is_empty() {
                return "\x1b[1;31mError:\x1b[0m Please specify at least one tag to filter on."
                    .to_string();
            }

            let mode = if match_all {
                TagFilterMode::All
            } else {
                TagFilterMode::Any
            };

            let query = TagFilterQuery {
                tags: filter_tags.clone(),
                mode,
                model: None,
                query_text: None,
                limit: Some(50),
            };

            match SessionTagManager::new().filter_sessions_advanced(&query) {
                Ok(sessions) => {
                    let label = filter_tags.join(if match_all { " + " } else { " | " });
                    format_tagged_sessions_table(&label, &sessions)
                }
                Err(e) => format!("\x1b[1;31mFailed to filter sessions:\x1b[0m {}", e),
            }
        }

        "clear" | "reset" => {
            let removed = clear_tags(session);
            format!(
                "\x1b[1;32mCleared {} tag(s) from active session.\x1b[0m",
                removed
            )
        }

        "set" => {
            if args.len() < 2 {
                return "\x1b[1;31mUsage:\x1b[0m /tag set <tag1> [tag2...]".to_string();
            }

            let mut new_tags = Vec::new();
            for raw in &args[1..] {
                match normalize_tag_name(raw) {
                    Ok(norm) => new_tags.push(SessionTag::new(norm)),
                    Err(e) => return format!("\x1b[1;31mError:\x1b[0m {}", e),
                }
            }

            match set_session_tags(session, &new_tags) {
                Ok(_) => {
                    let badges: Vec<String> = new_tags.iter().map(|t| t.display_badge()).collect();
                    format!("\x1b[1;32mSet session tags to:\x1b[0m {}", badges.join(" "))
                }
                Err(e) => format!("\x1b[1;31mFailed to set tags:\x1b[0m {}", e),
            }
        }

        "rename" | "mv" => {
            if args.len() < 3 {
                return "\x1b[1;31mUsage:\x1b[0m /tag rename <old_name> <new_name> [--global]"
                    .to_string();
            }

            let old_name = &args[1];
            let new_name = &args[2];
            let is_global = args.iter().skip(3).any(|a| a == "--global" || a == "-g");

            if is_global {
                match SessionTagManager::new().rename_tag_globally(old_name, new_name) {
                    Ok(count) => format!(
                        "\x1b[1;32mRenamed tag globally:\x1b[0m #{} → #{} across {} session(s)",
                        old_name, new_name, count
                    ),
                    Err(e) => format!("\x1b[1;31mGlobal rename failed:\x1b[0m {}", e),
                }
            } else {
                match rename_tag(session, old_name, new_name) {
                    Ok(_) => format!(
                        "\x1b[1;32mRenamed tag:\x1b[0m #{} → #{} on active session",
                        old_name, new_name
                    ),
                    Err(e) => format!("\x1b[1;31mRename failed:\x1b[0m {}", e),
                }
            }
        }

        "stats" | "analytics" => match SessionTagManager::new().tag_stats() {
            Ok(report) => format_tag_stats_report(&report),
            Err(e) => format!("\x1b[1;31mFailed to compute tag stats:\x1b[0m {}", e),
        },

        "session" => {
            if args.len() < 4 {
                return "\x1b[1;31mUsage:\x1b[0m /tag session <session_id> <add|rm> <tag>"
                    .to_string();
            }
            let session_id_str = &args[1];
            let action = args[2].to_lowercase();
            let tag_name = &args[3];

            let session_id = match Uuid::parse_str(session_id_str) {
                Ok(id) => id,
                Err(_) => {
                    // Try prefix lookup
                    match Session::find_by_prefix(session_id_str) {
                        Ok(Some(s)) => s.id,
                        _ => {
                            return format!(
                                "\x1b[1;31mInvalid session ID or prefix:\x1b[0m {}",
                                session_id_str
                            )
                        }
                    }
                }
            };

            let manager = SessionTagManager::new();
            match action.as_str() {
                "add" | "+" => match manager.tag_session(session_id, tag_name) {
                    Ok(t) => format!(
                        "\x1b[1;32mTagged session {}:\x1b[0m {}",
                        session_id,
                        t.display_badge()
                    ),
                    Err(e) => format!("\x1b[1;31mFailed to tag session:\x1b[0m {}", e),
                },
                "remove" | "rm" | "del" | "-" => {
                    match manager.untag_session(session_id, tag_name) {
                        Ok(_) => format!(
                            "\x1b[1;32mRemoved tag #{} from session {}\x1b[0m",
                            tag_name, session_id
                        ),
                        Err(e) => format!("\x1b[1;31mFailed to untag session:\x1b[0m {}", e),
                    }
                }
                _ => "\x1b[1;31mInvalid session action.\x1b[0m Use 'add' or 'rm'.".to_string(),
            }
        }

        // Shortcut / default fallback:
        _ => {
            // If argument starts with '?', treat as `/tag filter <tag>`
            if let Some(query) = args[0].strip_prefix('?') {
                return handle_tag_command(&["filter".to_string(), query.to_string()], session);
            }
            // If 1 argument given and it looks like a tag name, treat as `/tag add <name>`
            if args.len() == 1 {
                return handle_tag_command(&["add".to_string(), args[0].clone()], session);
            }
            format!(
                "\x1b[1;31mUnknown tag subcommand:\x1b[0m '{}'. Use \x1b[1m/tag help\x1b[0m for usage.",
                args[0]
            )
        }
    }
}

// ============================================================================
// Formatting Helpers
// ============================================================================

/// Renders a tag badge string with explicit color.
pub fn format_tag_badge_str(name: &str, color: Option<&str>) -> String {
    let eff_color = color.unwrap_or_else(|| deterministic_tag_color(name));
    let fg = tag_ansi_fg(eff_color);
    format!("{}{}{}\x1b[0m", fg, "\x1b[1m#", name)
}

/// Renders a single `SessionTag` badge.
pub fn format_tag_badge(tag: &SessionTag) -> String {
    tag.display_badge()
}

/// Formats the active session's tags into a rich terminal block.
pub fn format_active_session_tags(session: &Session) -> String {
    let tags = get_session_tags(session);
    let mut out = String::new();
    out.push_str("\x1b[1;36m🏷️  Active Session Tags\x1b[0m\n");
    out.push_str(&format!(
        "   Session: \x1b[1m{}\x1b[0m ({} messages)\n",
        session.title.as_deref().unwrap_or("Untitled Session"),
        session.messages.len()
    ));

    if tags.is_empty() {
        out.push_str(
            "   \x1b[90m(No tags assigned yet. Use `/tag add <name>` to attach tags)\x1b[0m\n",
        );
    } else {
        out.push_str("   Tags: ");
        let badges: Vec<String> = tags.iter().map(|t| t.display_badge()).collect();
        out.push_str(&badges.join("  "));
        out.push('\n');

        // Detailed view
        out.push_str("\n\x1b[1mDetailed Tag Breakdown:\x1b[0m\n");
        for tag in &tags {
            let desc = tag.description.as_deref().unwrap_or("");
            let cat = tag
                .category
                .as_deref()
                .map(|c| format!(" [{}]", c))
                .unwrap_or_default();
            out.push_str(&format!(
                "   • {} {}{}\n",
                tag.display_badge(),
                cat,
                if desc.is_empty() {
                    String::new()
                } else {
                    format!(" - \x1b[90m{}\x1b[0m", desc)
                }
            ));
        }
    }

    out.push_str("\n\x1b[90mCommands: /tag add <name> | /tag rm <name> | /tag filter <name> | /tag list --all\x1b[0m\n");
    out.trim_end().to_string()
}

/// Formats a directory table of all tags across historical sessions.
pub fn format_global_tags_table(frequencies: &[TagFrequency]) -> String {
    let mut out = String::new();
    out.push_str("\x1b[1;36m🏷️  Global Session Tags Directory\x1b[0m\n\n");

    if frequencies.is_empty() {
        out.push_str("   \x1b[90mNo tagged sessions found on disk.\x1b[0m\n");
        out.push_str("   Tag your current session with \x1b[1m/tag add <name>\x1b[0m\n");
        return out.trim_end().to_string();
    }

    out.push_str(&format!(
        "   Found \x1b[1m{}\x1b[0m unique tag(s) across saved sessions:\n\n",
        frequencies.len()
    ));

    out.push_str(&format!(
        "   {:<24} {:<10} {:<24}\n",
        "\x1b[1mTag\x1b[0m", "\x1b[1mSessions\x1b[0m", "\x1b[1mLast Active\x1b[0m"
    ));
    out.push_str(&format!("   {:-<24} {:-<10} {:-<24}\n", "", "", ""));

    for f in frequencies {
        let badge = format_tag_badge_str(&f.tag, f.color.as_deref());
        let last_time = f.last_used.split('T').next().unwrap_or(&f.last_used);
        out.push_str(&format!(
            "   {:<33} {:<10} {:<24}\n",
            badge, f.count, last_time
        ));
    }

    out.push_str("\n\x1b[90mFilter sessions by tag using: /tag filter <name>\x1b[0m\n");
    out.trim_end().to_string()
}

/// Formats filtered sessions into a readable table.
pub fn format_tagged_sessions_table(
    filter_label: &str,
    sessions: &[TaggedSessionSummary],
) -> String {
    let mut out = String::new();
    out.push_str(&format!(
        "\x1b[1;36m🔍 Sessions Tagged with \x1b[1m[{}]\x1b[0m\n\n",
        filter_label
    ));

    if sessions.is_empty() {
        out.push_str(&format!(
            "   \x1b[90mNo saved sessions found matching tag '{}'.\x1b[0m\n",
            filter_label
        ));
        out.push_str("   View all available tags with \x1b[1m/tag list --all\x1b[0m\n");
        return out.trim_end().to_string();
    }

    out.push_str(&format!(
        "   Found \x1b[1m{}\x1b[0m matching session(s):\n\n",
        sessions.len()
    ));

    for (idx, session) in sessions.iter().enumerate() {
        let short_id: String = session.id.to_string().chars().take(8).collect();
        let title = session.title.as_deref().unwrap_or("Untitled Session");
        let date = session
            .updated_at
            .split('T')
            .next()
            .unwrap_or(&session.updated_at);
        let tag_badges: Vec<String> = session.tags.iter().map(|t| t.display_badge()).collect();

        out.push_str(&format!(
            "   \x1b[1;33m{:>2}.\x1b[0m \x1b[1m{}\x1b[0m (\x1b[36m{}\x1b[0m)\n",
            idx + 1,
            title,
            short_id
        ));
        out.push_str(&format!(
            "       Model: \x1b[90m{}\x1b[0m | Messages: {} | Updated: {}\n",
            session.active_model, session.message_count, date
        ));
        out.push_str(&format!("       Tags: {}\n", tag_badges.join(" ")));
        if !session.preview.is_empty() && session.preview != "Empty session" {
            out.push_str(&format!("       \x1b[90m\"{}\"\x1b[0m\n", session.preview));
        }
        out.push('\n');
    }

    out.push_str(&format!(
        "\x1b[90mResume any session with: /session load <id>\x1b[0m\n"
    ));
    out.trim_end().to_string()
}

/// Formats tag statistics and analytics.
pub fn format_tag_stats_report(report: &TagStatsReport) -> String {
    let mut out = String::new();
    out.push_str("\x1b[1;36m📊 Session Tag Analytics & Statistics\x1b[0m\n\n");

    out.push_str(&format!(
        "   • \x1b[1mTotal Unique Tags:\x1b[0m      {}\n",
        report.total_unique_tags
    ));
    out.push_str(&format!(
        "   • \x1b[1mTagged Sessions:\x1b[0m        {} / {} scanned\n",
        report.total_tagged_sessions, report.total_sessions_scanned
    ));

    let pct = if report.total_sessions_scanned > 0 {
        (report.total_tagged_sessions as f64 / report.total_sessions_scanned as f64) * 100.0
    } else {
        0.0
    };
    out.push_str(&format!(
        "   • \x1b[1mTagging Coverage:\x1b[0m       {:.1}%\n\n",
        pct
    ));

    if !report.top_tags.is_empty() {
        out.push_str("\x1b[1mTop Tags by Session Count:\x1b[0m\n");
        for (i, t) in report.top_tags.iter().take(5).enumerate() {
            let badge = format_tag_badge_str(&t.tag, t.color.as_deref());
            let bar_len = (t.count * 2).min(20);
            let bar: String = "█".repeat(bar_len);
            out.push_str(&format!(
                "   {:>2}. {:<25} {:>3} sessions \x1b[36m{}\x1b[0m\n",
                i + 1,
                badge,
                t.count,
                bar
            ));
        }
        out.push('\n');
    }

    out.trim_end().to_string()
}

/// Formats introductory welcome and help text when `/tag` is run on an untagged session.
pub fn format_tag_welcome_and_help() -> String {
    let mut out = String::new();
    out.push_str("\x1b[1;36m🏷️  Fusion Session Tagging System\x1b[0m\n\n");
    out.push_str(
        "Organize, categorize, and quickly retrieve conversational sessions using tags.\n\n",
    );
    out.push_str("\x1b[1mQuick Start:\x1b[0m\n");
    out.push_str("   • \x1b[1m/tag add <name>\x1b[0m               Tag active session (e.g. `/tag add rust-backend`)\n");
    out.push_str(
        "   • \x1b[1m/tag add tag1 tag2 tag3\x1b[0m       Tag active session with multiple tags\n",
    );
    out.push_str("   • \x1b[1m/tag list\x1b[0m                     View tags attached to the active session\n");
    out.push_str("   • \x1b[1m/tag list --all\x1b[0m               View directory of all tags across saved sessions\n");
    out.push_str(
        "   • \x1b[1m/tag filter <name>\x1b[0m            Find all saved sessions with this tag\n",
    );
    out.push_str(
        "   • \x1b[1m/tag rm <name>\x1b[0m                Remove a tag from the active session\n\n",
    );
    out.push_str("\x1b[90mRun `/tag help` for the complete command reference.\x1b[0m\n");
    out.trim_end().to_string()
}

/// Formats the full comprehensive `/tag` command help manual.
pub fn format_tag_help() -> String {
    let mut out = String::new();
    out.push_str("\x1b[1;36m🏷️  Session Tagging Subsystem - Command Reference\x1b[0m\n\n");
    out.push_str(
        "Organize, categorize, list, and filter conversational sessions by custom tags.\n\n",
    );

    out.push_str("\x1b[1mCOMMANDS:\x1b[0m\n");
    out.push_str("   \x1b[1m/tag add <name> [tag2...] [--color <color>] [--desc <text>]\x1b[0m\n");
    out.push_str("       Attach one or more tags to the active session. Color and description are optional.\n\n");

    out.push_str("   \x1b[1m/tag list\x1b[0m (aliases: `/tag ls`, `/tag show`)\n");
    out.push_str("       Display tags attached to the currently active session.\n\n");

    out.push_str("   \x1b[1m/tag list --all\x1b[0m (aliases: `/tag all`, `/tag list -a`)\n");
    out.push_str("       Scan all historical sessions on disk and show a global tag frequency directory.\n\n");

    out.push_str("   \x1b[1m/tag filter <name> [tag2...] [--all]\x1b[0m (aliases: `/tag find`, `/tag search`)\n");
    out.push_str("       Find and display all saved sessions tagged with the given tag(s).\n");
    out.push_str(
        "       By default matches sessions having ANY tag; use `--all` to require ALL tags.\n\n",
    );

    out.push_str("   \x1b[1m/tag remove <name> [tag2...]\x1b[0m (aliases: `/tag rm`, `/tag del`, `/tag untag`)\n");
    out.push_str("       Remove one or more tags from the currently active session.\n\n");

    out.push_str("   \x1b[1m/tag clear\x1b[0m\n");
    out.push_str("       Clear all tags from the currently active session.\n\n");

    out.push_str("   \x1b[1m/tag set <tag1> <tag2> ...\x1b[0m\n");
    out.push_str("       Replace all tags on the active session with the specified set.\n\n");

    out.push_str(
        "   \x1b[1m/tag rename <old_name> <new_name> [--global]\x1b[0m (alias: `/tag mv`)\n",
    );
    out.push_str("       Rename a tag in the active session, or across all saved sessions on disk with `--global`.\n\n");

    out.push_str("   \x1b[1m/tag session <id> <add|rm> <tag>\x1b[0m\n");
    out.push_str("       Add or remove tags on a specific saved session by UUID or prefix.\n\n");

    out.push_str("   \x1b[1m/tag stats\x1b[0m (alias: `/tag analytics`)\n");
    out.push_str("       Display aggregate tag metrics, distribution, and top tags.\n\n");

    out.push_str("\x1b[1mEXAMPLES:\x1b[0m\n");
    out.push_str("   /tag add rust compiler\n");
    out.push_str("   /tag add backend --color magenta --desc \"API refactoring\"\n");
    out.push_str("   /tag filter rust\n");
    out.push_str("   /tag filter rust backend --all\n");
    out.push_str("   /tag list --all\n");
    out.push_str("   /tag rename old-name new-name --global\n\n");

    out.push_str("\x1b[90mSupported Colors: cyan, magenta, green, yellow, blue, red, bright_cyan, bright_green, etc.\x1b[0m\n");
    out.trim_end().to_string()
}

// ============================================================================
// Unit Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::provider::types::Message;
    use tempfile::tempdir;

    fn create_test_session() -> Session {
        let mut session = Session::new("gpt-4o");
        session.set_title("Compiler Optimization Research");
        session.add_user_message("How do we optimize LLVM IR passes?");
        session
            .add_assistant_message("By utilizing loop vectorization and inline threshold tuning.");
        session
    }

    #[test]
    fn test_tag_name_normalization() {
        assert_eq!(normalize_tag_name("rust").unwrap(), "rust");
        assert_eq!(normalize_tag_name("  #Rust  ").unwrap(), "rust");
        assert_eq!(normalize_tag_name("#backend-v2").unwrap(), "backend-v2");
        assert_eq!(normalize_tag_name("FEATURE:auth").unwrap(), "feature:auth");
        assert_eq!(normalize_tag_name("project/sub").unwrap(), "project/sub");
        assert_eq!(normalize_tag_name("v1.2.3").unwrap(), "v1.2.3");

        // Errors
        assert!(normalize_tag_name("").is_err());
        assert!(normalize_tag_name("   ").is_err());
        assert!(normalize_tag_name("#").is_err());
        assert!(normalize_tag_name("invalid tag with spaces").is_err());
        assert!(normalize_tag_name("bad!char").is_err());
        assert!(normalize_tag_name("bad@char").is_err());

        let too_long = "a".repeat(51);
        assert!(normalize_tag_name(&too_long).is_err());
    }

    #[test]
    fn test_add_and_get_session_tags() {
        let mut session = create_test_session();
        assert!(get_session_tags(&session).is_empty());
        assert!(!has_tag(&session, "rust"));

        let tag = add_tag(&mut session, "rust").unwrap();
        assert_eq!(tag.name, "rust");
        assert!(has_tag(&session, "rust"));
        assert!(has_tag(&session, "#Rust")); // Case and hash-insensitive query

        let tag2 = add_tag_with_details(
            &mut session,
            "compiler",
            Some("magenta"),
            Some("LLVM passes"),
            Some("tech"),
        )
        .unwrap();
        assert_eq!(tag2.name, "compiler");
        assert_eq!(tag2.color.as_deref(), Some("magenta"));
        assert_eq!(tag2.category.as_deref(), Some("tech"));

        let all_tags = get_session_tags(&session);
        assert_eq!(all_tags.len(), 2);
        let names = get_session_tag_names(&session);
        assert_eq!(names, vec!["rust".to_string(), "compiler".to_string()]);

        // Duplicate rejection
        let dup_res = add_tag(&mut session, "rust");
        assert!(matches!(dup_res, Err(TaggingError::TagAlreadyExists(_))));
    }

    #[test]
    fn test_add_multiple_tags() {
        let mut session = create_test_session();
        let added = add_tags(&mut session, &["rust", "llvm", "backend"]).unwrap();
        assert_eq!(added.len(), 3);
        assert_eq!(get_session_tags(&session).len(), 3);

        // Adding more with some overlaps
        let added_more = add_tags(&mut session, &["rust", "ml"]).unwrap();
        assert_eq!(added_more.len(), 1);
        assert_eq!(added_more[0].name, "ml");
        assert_eq!(get_session_tags(&session).len(), 4);
    }

    #[test]
    fn test_remove_and_clear_tags() {
        let mut session = create_test_session();
        add_tags(&mut session, &["rust", "compiler", "llvm"]).unwrap();

        assert!(remove_tag(&mut session, "compiler").unwrap());
        assert!(!has_tag(&session, "compiler"));
        assert_eq!(get_session_tags(&session).len(), 2);

        // Remove non-existent
        assert!(matches!(
            remove_tag(&mut session, "nonexistent"),
            Err(TaggingError::TagNotFound(_))
        ));

        // Remove multiple
        let removed = remove_tags(&mut session, &["rust", "other"]).unwrap();
        assert_eq!(removed, 1);
        assert_eq!(get_session_tags(&session).len(), 1);
        assert!(has_tag(&session, "llvm"));

        // Clear
        let cleared = clear_tags(&mut session);
        assert_eq!(cleared, 1);
        assert!(get_session_tags(&session).is_empty());
    }

    #[test]
    fn test_rename_tag() {
        let mut session = create_test_session();
        add_tags(&mut session, &["rust", "llvm"]).unwrap();

        assert!(rename_tag(&mut session, "llvm", "llvm-ir").unwrap());
        assert!(!has_tag(&session, "llvm"));
        assert!(has_tag(&session, "llvm-ir"));

        // Rename to existing should fail
        assert!(matches!(
            rename_tag(&mut session, "llvm-ir", "rust"),
            Err(TaggingError::TagAlreadyExists(_))
        ));
    }

    #[test]
    fn test_legacy_tags_fallback() {
        let mut session = create_test_session();
        session.set_metadata("tags", "rust, backend, architecture");

        let tags = get_session_tags(&session);
        assert_eq!(tags.len(), 3);
        assert_eq!(tags[0].name, "rust");
        assert_eq!(tags[1].name, "backend");
        assert_eq!(tags[2].name, "architecture");
    }

    #[test]
    fn test_tag_collection_helper() {
        let mut col = SessionTagCollection::new();
        assert!(col.is_empty());

        assert!(col.add(SessionTag::new("rust")));
        assert!(col.add(SessionTag::new("wasm")));
        assert_eq!(col.len(), 2);
        assert!(col.has("rust"));

        assert!(col.remove("rust"));
        assert_eq!(col.len(), 1);
        assert!(!col.has("rust"));
    }

    #[test]
    fn test_disk_filtering_and_manager() {
        let tmp = tempdir().unwrap();
        let dir = tmp.path().to_path_buf();
        let manager = SessionTagManager::with_dir(dir.clone());

        // Create 3 sessions on disk
        let mut s1 = Session::new("gpt-4o");
        s1.set_title("Session One");
        add_tags(&mut s1, &["rust", "backend"]).unwrap();
        s1.save_to_path(dir.join(format!("{}.json", s1.id)))
            .unwrap();

        let mut s2 = Session::new("claude-3-5-sonnet");
        s2.set_title("Session Two");
        add_tags(&mut s2, &["rust", "frontend", "wasm"]).unwrap();
        s2.save_to_path(dir.join(format!("{}.json", s2.id)))
            .unwrap();

        let mut s3 = Session::new("gpt-4o");
        s3.set_title("Session Three");
        add_tags(&mut s3, &["python", "data"]).unwrap();
        s3.save_to_path(dir.join(format!("{}.json", s3.id)))
            .unwrap();

        // 1. List all tags
        let all_tags = manager.list_all_tags().unwrap();
        assert_eq!(all_tags.len(), 6); // rust(2), backend(1), frontend(1), wasm(1), python(1), data(1)
        let rust_freq = all_tags.iter().find(|t| t.tag == "rust").unwrap();
        assert_eq!(rust_freq.count, 2);

        // 2. Filter by single tag
        let rust_sessions = manager.filter_sessions("rust").unwrap();
        assert_eq!(rust_sessions.len(), 2);

        let python_sessions = manager.filter_sessions("python").unwrap();
        assert_eq!(python_sessions.len(), 1);
        assert_eq!(python_sessions[0].title.as_deref(), Some("Session Three"));

        // 3. Multi-tag filter AND
        let query_and = TagFilterQuery::all_of(vec!["rust".to_string(), "backend".to_string()]);
        let and_res = manager.filter_sessions_advanced(&query_and).unwrap();
        assert_eq!(and_res.len(), 1);
        assert_eq!(and_res[0].id, s1.id);

        // 4. Multi-tag filter OR
        let query_or = TagFilterQuery::any_of(vec!["backend".to_string(), "data".to_string()]);
        let or_res = manager.filter_sessions_advanced(&query_or).unwrap();
        assert_eq!(or_res.len(), 2);

        // 5. Suggest tags
        let suggestions = manager.suggest_tags("ru", 5).unwrap();
        assert_eq!(suggestions, vec!["rust".to_string()]);

        // 6. Global tag rename
        let renamed = manager.rename_tag_globally("backend", "server").unwrap();
        assert_eq!(renamed, 1);
        assert_eq!(manager.filter_sessions("backend").unwrap().len(), 0);
        assert_eq!(manager.filter_sessions("server").unwrap().len(), 1);

        // 7. Global tag delete
        let deleted = manager.delete_tag_globally("wasm").unwrap();
        assert_eq!(deleted, 1);
        assert_eq!(manager.filter_sessions("wasm").unwrap().len(), 0);

        // 8. Tag saved session directly
        let added_tag = manager.tag_session(s3.id, "machine-learning").unwrap();
        assert_eq!(added_tag.name, "machine-learning");
        let s3_loaded = Session::load_from_path(dir.join(format!("{}.json", s3.id))).unwrap();
        assert!(has_tag(&s3_loaded, "machine-learning"));

        // 9. Stats report
        let stats = manager.tag_stats().unwrap();
        assert_eq!(stats.total_sessions_scanned, 3);
        assert_eq!(stats.total_tagged_sessions, 3);
    }

    #[test]
    fn test_handle_tag_slash_commands() {
        let mut session = create_test_session();

        // 1. Tag help
        let help = handle_tag_command(&["help".to_string()], &mut session);
        assert!(help.contains("Session Tagging Subsystem"));

        // 2. Initial empty list / welcome
        let initial = handle_tag_command(&[], &mut session);
        assert!(
            initial.contains("Fusion Session Tagging System")
                || initial.contains("Active Session Tags")
        );

        // 3. /tag add
        let add_out = handle_tag_command(
            &["add".to_string(), "rust".to_string(), "backend".to_string()],
            &mut session,
        );
        assert!(add_out.contains("Tagged active session"));
        assert!(has_tag(&session, "rust"));
        assert!(has_tag(&session, "backend"));

        // 4. /tag list
        let list_out = handle_tag_command(&["list".to_string()], &mut session);
        assert!(list_out.contains("#rust"));
        assert!(list_out.contains("#backend"));

        // 5. /tag rename
        let rename_out = handle_tag_command(
            &[
                "rename".to_string(),
                "backend".to_string(),
                "api".to_string(),
            ],
            &mut session,
        );
        assert!(rename_out.contains("Renamed tag"));
        assert!(!has_tag(&session, "backend"));
        assert!(has_tag(&session, "api"));

        // 6. /tag remove
        let rm_out = handle_tag_command(&["rm".to_string(), "api".to_string()], &mut session);
        assert!(rm_out.contains("Removed 1 tag(s)"));
        assert!(!has_tag(&session, "api"));

        // 7. /tag set
        let set_out = handle_tag_command(
            &["set".to_string(), "alpha".to_string(), "beta".to_string()],
            &mut session,
        );
        assert!(set_out.contains("Set session tags to"));
        assert_eq!(get_session_tag_names(&session), vec!["alpha", "beta"]);

        // 8. /tag clear
        let clear_out = handle_tag_command(&["clear".to_string()], &mut session);
        assert!(clear_out.contains("Cleared 2 tag(s)"));
        assert!(get_session_tags(&session).is_empty());

        // 9. Shortcut /tag <single_name> -> adds tag
        let shortcut_out = handle_tag_command(&["quick-tag".to_string()], &mut session);
        assert!(shortcut_out.contains("Tagged active session"));
        assert!(has_tag(&session, "quick-tag"));
    }

    #[test]
    fn test_ansi_badges_and_colors() {
        let tag = SessionTag::new("rust").with_color("magenta");
        let badge = tag.display_badge();
        assert!(badge.contains("\x1b[35m"));
        assert!(badge.contains("#rust"));

        let pill = tag.display_pill();
        assert!(pill.contains("\x1b[45m"));
        assert!(pill.contains("rust"));

        let default_color = deterministic_tag_color("my-custom-tag");
        assert!(TAG_COLOR_PALETTE.contains(&default_color));
    }
}
