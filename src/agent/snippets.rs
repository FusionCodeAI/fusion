//! Code snippet manager and recall subsystem for Fusion.
//!
//! Provides capabilities to:
//! - Save reusable code snippets to global (`~/.fusion/snippets/`) or project-local (`.fusion/snippets/`) storage.
//! - Recall, inspect, and insert code snippets into active sessions via `/snippet save <name>` and `/snippet insert <name>`.
//! - Automatically detect programming languages and extract code blocks from previous conversation turns.
//! - Search, filter by tag or language, export, and import code snippet collections.

use std::collections::HashMap;
use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};

use chrono::Utc;
use regex::Regex;
use serde::{Deserialize, Serialize};

use crate::agent::session::Session;
use crate::config::Config;
use crate::provider::types::{Message, Role};

// ============================================================================
// Errors
// ============================================================================

/// Errors occurring during snippet operations.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SnippetError {
    /// Requested snippet was not found in storage.
    NotFound(String),
    /// Snippet name is empty or contains only whitespace.
    EmptyName,
    /// Snippet body/content is empty or contains only whitespace.
    EmptyContent,
    /// Snippet name contains invalid characters.
    InvalidName(String),
    /// File system or I/O error occurred.
    Io(String),
    /// Serialization or deserialization error occurred.
    Serialization(String),
    /// Validation error occurred.
    Validation(String),
}

impl fmt::Display for SnippetError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotFound(name) => write!(f, "Snippet '{}' not found in storage", name),
            Self::EmptyName => write!(f, "Snippet name cannot be empty"),
            Self::EmptyContent => write!(f, "Snippet content cannot be empty"),
            Self::InvalidName(name) => write!(f, "Invalid snippet name '{}'", name),
            Self::Io(msg) => write!(f, "Snippet I/O error: {}", msg),
            Self::Serialization(msg) => write!(f, "Snippet serialization error: {}", msg),
            Self::Validation(msg) => write!(f, "Snippet validation error: {}", msg),
        }
    }
}

impl std::error::Error for SnippetError {}

impl From<std::io::Error> for SnippetError {
    fn from(err: std::io::Error) -> Self {
        SnippetError::Io(err.to_string())
    }
}

impl From<serde_json::Error> for SnippetError {
    fn from(err: serde_json::Error) -> Self {
        SnippetError::Serialization(err.to_string())
    }
}

// ============================================================================
// Snippet Data Model
// ============================================================================

/// A reusable, persisted code or text snippet.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Snippet {
    /// Unique identifier / slug name (e.g. `auth-middleware`, `fibonacci`, `tokio-main`).
    pub name: String,
    /// The code or text snippet content.
    pub content: String,
    /// Optional short description or memo.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// Optional programming language identifier (e.g. `rust`, `python`, `typescript`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub language: Option<String>,
    /// Categorical tags for indexing and search.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
    /// RFC 3339 timestamp when snippet was created.
    pub created_at: String,
    /// RFC 3339 timestamp when snippet was last modified.
    pub updated_at: String,
    /// Additional metadata key-value pairs.
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub metadata: HashMap<String, String>,
}

impl Snippet {
    /// Creates a new snippet with the specified name and content.
    pub fn new(name: impl Into<String>, content: impl Into<String>) -> Self {
        let name_str = name.into();
        let content_str = content.into();
        let now = Utc::now().to_rfc3339();
        let auto_lang = detect_code_language(&content_str);

        Self {
            name: name_str,
            content: content_str,
            description: None,
            language: auto_lang,
            tags: Vec::new(),
            created_at: now.clone(),
            updated_at: now,
            metadata: HashMap::new(),
        }
    }

    /// Sets an optional description for this snippet.
    pub fn with_description(mut self, desc: impl Into<String>) -> Self {
        self.description = Some(desc.into());
        self
    }

    /// Sets an explicit programming language for this snippet.
    pub fn with_language(mut self, lang: impl Into<String>) -> Self {
        self.language = Some(lang.into().to_lowercase());
        self
    }

    /// Appends tags to this snippet.
    pub fn with_tags(mut self, tags: &[&str]) -> Self {
        self.tags = tags.iter().map(|s| s.trim().to_lowercase()).filter(|s| !s.is_empty()).collect();
        self
    }

    /// Adds a metadata key-value pair.
    pub fn with_metadata(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.metadata.insert(key.into(), value.into());
        self
    }

    /// Returns the number of lines in the snippet content.
    pub fn line_count(&self) -> usize {
        if self.content.is_empty() {
            0
        } else {
            self.content.lines().count()
        }
    }

    /// Returns the byte size of the snippet content.
    pub fn byte_size(&self) -> usize {
        self.content.len()
    }

    /// Returns true if the snippet has more than one line.
    pub fn is_multiline(&self) -> bool {
        self.content.contains('\n')
    }

    /// Returns the language or detected fallback language.
    pub fn detect_or_language(&self) -> String {
        self.language
            .clone()
            .or_else(|| detect_code_language(&self.content))
            .unwrap_or_else(|| "text".to_string())
    }

    /// Formats the snippet as a Markdown code block ready for insertion into a conversation turn.
    pub fn format_for_insertion(&self) -> String {
        let lang = self.detect_or_language();
        format!(
            "Code snippet `{}`:\n```{}\n{}\n```",
            self.name,
            lang,
            self.content.trim_end()
        )
    }

    /// Updates the snippet content and updates timestamp.
    pub fn update_content(&mut self, new_content: impl Into<String>) {
        self.content = new_content.into();
        self.updated_at = Utc::now().to_rfc3339();
        if self.language.is_none() {
            self.language = detect_code_language(&self.content);
        }
    }

    /// Validates snippet properties.
    pub fn validate(&self) -> Result<(), SnippetError> {
        let trimmed_name = self.name.trim();
        if trimmed_name.is_empty() {
            return Err(SnippetError::EmptyName);
        }
        if !is_valid_snippet_name(trimmed_name) {
            return Err(SnippetError::InvalidName(trimmed_name.to_string()));
        }
        if self.content.trim().is_empty() {
            return Err(SnippetError::EmptyContent);
        }
        Ok(())
    }
}

// ============================================================================
// Directory & Path Helpers
// ============================================================================

/// Returns the global snippet storage directory (`~/.fusion/snippets/`).
pub fn snippets_dir() -> PathBuf {
    Config::config_dir().join("snippets")
}

/// Returns the project-local snippet storage directory (`.fusion/snippets/`).
pub fn project_snippets_dir() -> PathBuf {
    PathBuf::from(".fusion").join("snippets")
}

/// Checks whether a snippet name is valid (alphanumeric, dash, underscore, dot).
pub fn is_valid_snippet_name(name: &str) -> bool {
    let trimmed = name.trim();
    if trimmed.is_empty() || trimmed.len() > 128 {
        return false;
    }
    trimmed.chars().all(|c| c.is_alphanumeric() || c == '-' || c == '_' || c == '.')
}

/// Sanitizes a snippet name for safe filesystem storage.
pub fn sanitize_snippet_name(name: &str) -> String {
    let sanitized: String = name
        .trim()
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '-' || c == '_' || c == '.' {
                c
            } else {
                '-'
            }
        })
        .collect();

    let clean = sanitized.trim_matches('-');
    if clean.is_empty() {
        "unnamed-snippet".to_string()
    } else {
        clean.to_string()
    }
}

/// Resolves the filesystem path for a snippet JSON file inside the specified directory.
pub fn snippet_file_path(dir: &Path, name: &str) -> PathBuf {
    let filename = format!("{}.json", sanitize_snippet_name(name));
    dir.join(filename)
}

// ============================================================================
// Standalone Storage Functions
// ============================================================================

/// Saves a snippet to global storage (`~/.fusion/snippets/`).
pub fn save_snippet(snippet: &Snippet) -> Result<PathBuf, SnippetError> {
    save_snippet_to(&snippets_dir(), snippet)
}

/// Saves a snippet to a designated directory.
pub fn save_snippet_to(dir: &Path, snippet: &Snippet) -> Result<PathBuf, SnippetError> {
    snippet.validate()?;
    if !dir.exists() {
        fs::create_dir_all(dir).map_err(|e| {
            SnippetError::Io(format!("Failed to create snippet directory '{}': {}", dir.display(), e))
        })?;
    }

    let path = snippet_file_path(dir, &snippet.name);
    let json_data = serde_json::to_string_pretty(snippet)?;
    fs::write(&path, json_data).map_err(|e| {
        SnippetError::Io(format!("Failed to write snippet file '{}': {}", path.display(), e))
    })?;

    Ok(path)
}

/// Loads a snippet by name from global storage (`~/.fusion/snippets/`).
pub fn load_snippet(name: &str) -> Result<Snippet, SnippetError> {
    load_snippet_from(&snippets_dir(), name)
}

/// Loads a snippet from a designated directory.
pub fn load_snippet_from(dir: &Path, name: &str) -> Result<Snippet, SnippetError> {
    let path = snippet_file_path(dir, name);
    if !path.exists() {
        return Err(SnippetError::NotFound(name.to_string()));
    }

    let content = fs::read_to_string(&path).map_err(|e| {
        SnippetError::Io(format!("Failed to read snippet file '{}': {}", path.display(), e))
    })?;

    let snippet: Snippet = serde_json::from_str(&content)?;
    Ok(snippet)
}

/// Gets an optional snippet by name from global storage.
pub fn get_snippet(name: &str) -> Result<Option<Snippet>, SnippetError> {
    match load_snippet(name) {
        Ok(s) => Ok(Some(s)),
        Err(SnippetError::NotFound(_)) => Ok(None),
        Err(e) => Err(e),
    }
}

/// Deletes a snippet from global storage.
pub fn delete_snippet(name: &str) -> Result<bool, SnippetError> {
    delete_snippet_from(&snippets_dir(), name)
}

/// Deletes a snippet from a designated directory.
pub fn delete_snippet_from(dir: &Path, name: &str) -> Result<bool, SnippetError> {
    let path = snippet_file_path(dir, name);
    if path.exists() {
        fs::remove_file(&path).map_err(|e| {
            SnippetError::Io(format!("Failed to delete snippet file '{}': {}", path.display(), e))
        })?;
        Ok(true)
    } else {
        Ok(false)
    }
}

/// Lists all snippets from global storage, sorted alphabetically by name.
pub fn list_snippets() -> Result<Vec<Snippet>, SnippetError> {
    list_snippets_from(&snippets_dir())
}

/// Lists all snippets in a designated directory.
pub fn list_snippets_from(dir: &Path) -> Result<Vec<Snippet>, SnippetError> {
    if !dir.exists() {
        return Ok(Vec::new());
    }

    let mut snippets = Vec::new();
    let entries = fs::read_dir(dir).map_err(|e| {
        SnippetError::Io(format!("Failed to read snippet directory '{}': {}", dir.display(), e))
    })?;

    for entry_res in entries {
        let entry = match entry_res {
            Ok(e) => e,
            Err(_) => continue,
        };

        let path = entry.path();
        if path.is_file() && path.extension().and_then(|s| s.to_str()) == Some("json") {
            if let Ok(content) = fs::read_to_string(&path) {
                if let Ok(snippet) = serde_json::from_str::<Snippet>(&content) {
                    snippets.push(snippet);
                }
            }
        }
    }

    snippets.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
    Ok(snippets)
}

/// Recalls a snippet by name from global storage.
pub fn recall_snippet(name: &str) -> Result<Snippet, SnippetError> {
    load_snippet(name)
}

/// Recalls a snippet and inserts it into the active session conversation history.
pub fn insert_snippet(name: &str, session: &mut Session) -> Result<String, SnippetError> {
    let snippet = load_snippet(name)?;
    let formatted = snippet.format_for_insertion();
    session.add_user_message(&formatted);
    Ok(formatted)
}

/// Searches snippets in global storage matching the query across name, description, tags, or content.
pub fn search_snippets(query: &str) -> Result<Vec<Snippet>, SnippetError> {
    let all = list_snippets()?;
    let q = query.trim().to_lowercase();
    if q.is_empty() {
        return Ok(all);
    }

    let results = all
        .into_iter()
        .filter(|s| {
            s.name.to_lowercase().contains(&q)
                || s.description
                    .as_deref()
                    .map(|d| d.to_lowercase().contains(&q))
                    .unwrap_or(false)
                || s.language
                    .as_deref()
                    .map(|l| l.to_lowercase().contains(&q))
                    .unwrap_or(false)
                || s.tags.iter().any(|t| t.to_lowercase().contains(&q))
                || s.content.to_lowercase().contains(&q)
        })
        .collect();

    Ok(results)
}

/// Clears all snippets from global storage.
pub fn clear_snippets() -> Result<usize, SnippetError> {
    let dir = snippets_dir();
    if !dir.exists() {
        return Ok(0);
    }

    let snippets = list_snippets_from(&dir)?;
    let count = snippets.len();
    for s in snippets {
        delete_snippet_from(&dir, &s.name)?;
    }
    Ok(count)
}

/// Exports all snippets from global storage into a single JSON file.
pub fn export_snippets_json(path: &Path) -> Result<usize, SnippetError> {
    let snippets = list_snippets()?;
    let count = snippets.len();
    if let Some(parent) = path.parent() {
        if !parent.exists() {
            fs::create_dir_all(parent).map_err(|e| {
                SnippetError::Io(format!("Failed to create export directory '{}': {}", parent.display(), e))
            })?;
        }
    }

    let data = serde_json::to_string_pretty(&snippets)?;
    fs::write(path, data).map_err(|e| {
        SnippetError::Io(format!("Failed to write export file '{}': {}", path.display(), e))
    })?;

    Ok(count)
}

/// Imports snippets from a JSON file into global storage.
pub fn import_snippets_json(path: &Path) -> Result<usize, SnippetError> {
    if !path.exists() {
        return Err(SnippetError::Io(format!("Import file not found: {}", path.display())));
    }

    let content = fs::read_to_string(path).map_err(|e| {
        SnippetError::Io(format!("Failed to read import file '{}': {}", path.display(), e))
    })?;

    let snippets: Vec<Snippet> = serde_json::from_str(&content)?;
    let mut imported = 0;
    for snippet in snippets {
        if save_snippet(&snippet).is_ok() {
            imported += 1;
        }
    }

    Ok(imported)
}

// ============================================================================
// SnippetManager Struct
// ============================================================================

/// Manages snippet storage, caching, recall, and persistence across global and local directories.
#[derive(Debug, Clone)]
pub struct SnippetManager {
    global_dir: PathBuf,
    local_dir: Option<PathBuf>,
    cache: HashMap<String, Snippet>,
}

impl Default for SnippetManager {
    fn default() -> Self {
        Self::new()
    }
}

impl SnippetManager {
    /// Creates a new `SnippetManager` using standard `~/.fusion/snippets/` and `.fusion/snippets/` paths.
    pub fn new() -> Self {
        let mut mgr = Self {
            global_dir: snippets_dir(),
            local_dir: Some(project_snippets_dir()),
            cache: HashMap::new(),
        };
        let _ = mgr.load_all();
        mgr
    }

    /// Creates a `SnippetManager` isolated to a specific custom directory (useful for unit tests).
    pub fn with_dir(dir: PathBuf) -> Self {
        let mut mgr = Self {
            global_dir: dir,
            local_dir: None,
            cache: HashMap::new(),
        };
        let _ = mgr.load_all();
        mgr
    }

    /// Creates a `SnippetManager` with designated global and optional local directories.
    pub fn with_dirs(global_dir: PathBuf, local_dir: Option<PathBuf>) -> Self {
        let mut mgr = Self {
            global_dir,
            local_dir,
            cache: HashMap::new(),
        };
        let _ = mgr.load_all();
        mgr
    }

    /// Loads all snippets from disk into cache.
    pub fn load_all(&mut self) -> Result<usize, SnippetError> {
        self.cache.clear();

        // Load global snippets
        if let Ok(snippets) = list_snippets_from(&self.global_dir) {
            for s in snippets {
                self.cache.insert(s.name.to_lowercase(), s);
            }
        }

        // Local snippets override global ones
        if let Some(local) = &self.local_dir {
            if let Ok(snippets) = list_snippets_from(local) {
                for s in snippets {
                    self.cache.insert(s.name.to_lowercase(), s);
                }
            }
        }

        Ok(self.cache.len())
    }

    /// Saves a snippet to global storage and updates cache.
    pub fn save(&mut self, snippet: Snippet) -> Result<PathBuf, SnippetError> {
        snippet.validate()?;
        let path = save_snippet_to(&self.global_dir, &snippet)?;
        self.cache.insert(snippet.name.to_lowercase(), snippet);
        Ok(path)
    }

    /// Saves a snippet to project-local storage (`.fusion/snippets/`) and updates cache.
    pub fn save_local(&mut self, snippet: Snippet) -> Result<PathBuf, SnippetError> {
        snippet.validate()?;
        let dir = self.local_dir.clone().unwrap_or_else(project_snippets_dir);
        let path = save_snippet_to(&dir, &snippet)?;
        self.cache.insert(snippet.name.to_lowercase(), snippet);
        Ok(path)
    }

    /// Retrieves a snippet from cache or disk.
    pub fn get(&self, name: &str) -> Option<&Snippet> {
        self.cache.get(&name.trim().to_lowercase())
    }

    /// Retrieves a mutable reference to a snippet from cache.
    pub fn get_mut(&mut self, name: &str) -> Option<&mut Snippet> {
        self.cache.get_mut(&name.trim().to_lowercase())
    }

    /// Recalls a snippet by name, returning a clone.
    pub fn recall(&self, name: &str) -> Result<Snippet, SnippetError> {
        self.get(name)
            .cloned()
            .ok_or_else(|| SnippetError::NotFound(name.to_string()))
    }

    /// Inserts a recalled snippet into the specified session.
    pub fn insert(&self, name: &str, session: &mut Session) -> Result<String, SnippetError> {
        let snippet = self.recall(name)?;
        let formatted = snippet.format_for_insertion();
        session.add_user_message(&formatted);
        Ok(formatted)
    }

    /// Deletes a snippet by name from storage and cache.
    pub fn delete(&mut self, name: &str) -> Result<bool, SnippetError> {
        let key = name.trim().to_lowercase();
        let mut deleted = false;

        if delete_snippet_from(&self.global_dir, name)? {
            deleted = true;
        }

        if let Some(local) = &self.local_dir {
            if delete_snippet_from(local, name)? {
                deleted = true;
            }
        }

        self.cache.remove(&key);
        Ok(deleted)
    }

    /// Lists all cached snippets, sorted alphabetically by name.
    pub fn list(&self) -> Vec<&Snippet> {
        let mut list: Vec<&Snippet> = self.cache.values().collect();
        list.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
        list
    }

    /// Searches snippets matching a query string.
    pub fn search(&self, query: &str) -> Vec<&Snippet> {
        let q = query.trim().to_lowercase();
        if q.is_empty() {
            return self.list();
        }

        let mut matches: Vec<&Snippet> = self
            .cache
            .values()
            .filter(|s| {
                s.name.to_lowercase().contains(&q)
                    || s.description
                        .as_deref()
                        .map(|d| d.to_lowercase().contains(&q))
                        .unwrap_or(false)
                    || s.language
                        .as_deref()
                        .map(|l| l.to_lowercase().contains(&q))
                        .unwrap_or(false)
                    || s.tags.iter().any(|t| t.to_lowercase().contains(&q))
                    || s.content.to_lowercase().contains(&q)
            })
            .collect();

        matches.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
        matches
    }

    /// Filters snippets by tag.
    pub fn filter_by_tag(&self, tag: &str) -> Vec<&Snippet> {
        let target_tag = tag.trim().to_lowercase();
        let mut matches: Vec<&Snippet> = self
            .cache
            .values()
            .filter(|s| s.tags.iter().any(|t| t.to_lowercase() == target_tag))
            .collect();
        matches.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
        matches
    }

    /// Filters snippets by language.
    pub fn filter_by_language(&self, lang: &str) -> Vec<&Snippet> {
        let target_lang = lang.trim().to_lowercase();
        let mut matches: Vec<&Snippet> = self
            .cache
            .values()
            .filter(|s| {
                s.language
                    .as_deref()
                    .map(|l| l.to_lowercase() == target_lang)
                    .unwrap_or(false)
            })
            .collect();
        matches.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
        matches
    }

    /// Clears all snippets from memory and disk.
    pub fn clear(&mut self) -> Result<usize, SnippetError> {
        let count = self.cache.len();
        for s in self.cache.values() {
            let _ = delete_snippet_from(&self.global_dir, &s.name);
            if let Some(local) = &self.local_dir {
                let _ = delete_snippet_from(local, &s.name);
            }
        }
        self.cache.clear();
        Ok(count)
    }

    /// Exports all snippets to a JSON file.
    pub fn export_json(&self, path: &Path) -> Result<usize, SnippetError> {
        let snippets = self.list();
        let count = snippets.len();
        if let Some(parent) = path.parent() {
            if !parent.exists() {
                fs::create_dir_all(parent).map_err(|e| {
                    SnippetError::Io(format!("Failed to create directory '{}': {}", parent.display(), e))
                })?;
            }
        }

        let data = serde_json::to_string_pretty(&snippets)?;
        fs::write(path, data).map_err(|e| {
            SnippetError::Io(format!("Failed to write export file '{}': {}", path.display(), e))
        })?;

        Ok(count)
    }

    /// Imports snippets from a JSON file into manager storage.
    pub fn import_json(&mut self, path: &Path) -> Result<usize, SnippetError> {
        if !path.exists() {
            return Err(SnippetError::Io(format!("File not found: {}", path.display())));
        }

        let content = fs::read_to_string(path).map_err(|e| {
            SnippetError::Io(format!("Failed to read import file '{}': {}", path.display(), e))
        })?;

        let snippets: Vec<Snippet> = serde_json::from_str(&content)?;
        let mut imported = 0;
        for s in snippets {
            if self.save(s).is_ok() {
                imported += 1;
            }
        }

        Ok(imported)
    }

    /// Returns the total number of cached snippets.
    pub fn len(&self) -> usize {
        self.cache.len()
    }

    /// Returns true if there are no snippets.
    pub fn is_empty(&self) -> bool {
        self.cache.is_empty()
    }
}

// ============================================================================
// Code Language Detection & Extraction
// ============================================================================

/// Detects the programming language of a snippet using heuristic pattern matching.
pub fn detect_code_language(code: &str) -> Option<String> {
    let trimmed = code.trim();
    if trimmed.is_empty() {
        return None;
    }

    // Rust
    if trimmed.contains("fn main(")
        || trimmed.contains("pub struct ")
        || trimmed.contains("pub enum ")
        || trimmed.contains("impl ")
        || trimmed.contains("let mut ")
        || trimmed.contains("use std::")
        || trimmed.contains("#[derive(")
    {
        return Some("rust".to_string());
    }

    // Python
    if trimmed.contains("def ")
        || trimmed.contains("import ")
        || trimmed.contains("from ")
        || trimmed.contains("class ")
        || trimmed.contains("elif ")
        || trimmed.contains("__main__")
        || trimmed.starts_with("#!/usr/bin/env python")
    {
        return Some("python".to_string());
    }

    // TypeScript / JavaScript
    if trimmed.contains("export interface ")
        || trimmed.contains("export type ")
        || trimmed.contains("const ")
        || trimmed.contains("let ")
        || trimmed.contains("console.log(")
        || trimmed.contains("function ")
        || trimmed.contains("async () =>")
        || trimmed.contains("import React")
        || trimmed.contains("module.exports")
    {
        if trimmed.contains(": string")
            || trimmed.contains(": number")
            || trimmed.contains(": boolean")
            || trimmed.contains("<T>")
            || trimmed.contains("interface ")
        {
            return Some("typescript".to_string());
        }
        return Some("javascript".to_string());
    }

    // Go
    if trimmed.contains("func ")
        || trimmed.contains("package ")
        || trimmed.contains("import (")
        || trimmed.contains("fmt.Print")
    {
        return Some("go".to_string());
    }

    // C / C++
    if trimmed.contains("#include <")
        || trimmed.contains("int main(")
        || trimmed.contains("std::cout")
        || trimmed.contains("printf(")
    {
        if trimmed.contains("std::") || trimmed.contains("class ") || trimmed.contains("template<") {
            return Some("cpp".to_string());
        }
        return Some("c".to_string());
    }

    // SQL
    let upper = trimmed.to_uppercase();
    if upper.starts_with("SELECT ")
        || upper.starts_with("INSERT INTO ")
        || upper.starts_with("UPDATE ")
        || upper.starts_with("CREATE TABLE ")
        || upper.starts_with("ALTER TABLE ")
        || upper.starts_with("DELETE FROM ")
    {
        return Some("sql".to_string());
    }

    // JSON
    if (trimmed.starts_with('{') && trimmed.ends_with('}'))
        || (trimmed.starts_with('[') && trimmed.ends_with(']'))
    {
        if serde_json::from_str::<serde_json::Value>(trimmed).is_ok() {
            return Some("json".to_string());
        }
    }

    // Shell / Bash
    if trimmed.starts_with("#!/bin/bash")
        || trimmed.starts_with("#!/bin/sh")
        || trimmed.contains("echo ")
        || trimmed.contains("export ")
        || trimmed.contains("chmod +x")
    {
        return Some("bash".to_string());
    }

    // HTML / XML
    if trimmed.starts_with("<!DOCTYPE")
        || trimmed.starts_with("<html")
        || (trimmed.starts_with('<') && trimmed.ends_with('>') && trimmed.contains("</"))
    {
        return Some("html".to_string());
    }

    // CSS
    if trimmed.contains("margin:") || trimmed.contains("padding:") || trimmed.contains("color:") {
        return Some("css".to_string());
    }

    None
}

/// Extracts all fenced code blocks (```lang ... ```) from a text string.
pub fn extract_code_blocks(text: &str) -> Vec<(Option<String>, String)> {
    let mut blocks = Vec::new();
    let re = Regex::new(r"(?s)```([a-zA-Z0-9_-]*)\r?\n(.*?)\r?\n```").unwrap();

    for cap in re.captures_iter(text) {
        let lang = cap.get(1).map(|m| m.as_str().trim().to_string()).filter(|s| !s.is_empty());
        let code = cap.get(2).map(|m| m.as_str().to_string()).unwrap_or_default();
        if !code.trim().is_empty() {
            blocks.push((lang, code));
        }
    }

    blocks
}

/// Extracts the last code snippet from recent assistant or user messages in a session.
pub fn extract_last_code_snippet(session: &Session) -> Option<(Option<String>, String)> {
    // Scan backwards through messages to find the most recent message with a code block
    for msg in session.messages().iter().rev() {
        if msg.role == Role::Assistant || msg.role == Role::User {
            let blocks = extract_code_blocks(&msg.content);
            if let Some(last_block) = blocks.into_iter().last() {
                return Some(last_block);
            }
        }
    }
    None
}

// ============================================================================
// Formatting Helpers for CLI / REPL
// ============================================================================

/// Formats a list of snippets into a clean, formatted terminal table.
pub fn format_snippet_table(snippets: &[Snippet]) -> String {
    if snippets.is_empty() {
        return "\x1b[1;33mNo code snippets saved.\x1b[0m\n\
                \x1b[2;37mUse \x1b[1;36m/snippet save <name> [content...]\x1b[0m\x1b[2;37m to save your first snippet.\x1b[0m"
            .to_string();
    }

    let mut out = String::new();
    out.push_str(&format!(
        "\n\x1b[1;36mCode Snippets\x1b[0m \x1b[2;37m(~/.fusion/snippets/)\x1b[0m ({} items)\n\n",
        snippets.len()
    ));

    out.push_str(&format!(
        "  \x1b[1;37m{:<20} {:<12} {:>8}  {:<16} {}\x1b[0m\n",
        "NAME", "LANGUAGE", "LINES", "UPDATED", "DESCRIPTION / TAGS"
    ));
    out.push_str(&format!(
        "  \x1b[2;37m{:<20} {:<12} {:>8}  {:<16} {}\x1b[0m\n",
        "--------------------", "------------", "--------", "----------------", "------------------------------"
    ));

    for s in snippets {
        let lang = s.detect_or_language();
        let lines = s.line_count();
        let updated_short = s.updated_at.split('T').next().unwrap_or(&s.updated_at);
        let mut desc_tags = s.description.clone().unwrap_or_default();
        if !s.tags.is_empty() {
            let tag_str = s.tags.iter().map(|t| format!("#{}", t)).collect::<Vec<_>>().join(" ");
            if desc_tags.is_empty() {
                desc_tags = format!("\x1b[2;37m{}\x1b[0m", tag_str);
            } else {
                desc_tags = format!("{} \x1b[2;37m({})\x1b[0m", desc_tags, tag_str);
            }
        }

        out.push_str(&format!(
            "  \x1b[1;36m{:<20}\x1b[0m \x1b[1;33m{:<12}\x1b[0m {:>8}  \x1b[2;37m{:<16}\x1b[0m {}\n",
            s.name, lang, lines, updated_short, desc_tags
        ));
    }

    out.push_str("\n\x1b[2;37mCommands:\x1b[0m \x1b[1;36m/snippet insert <name>\x1b[0m  •  \x1b[1;36m/snippet show <name>\x1b[0m  •  \x1b[1;36m/snippet delete <name>\x1b[0m\n");
    out
}

/// Formats detailed information and code content for a single snippet.
pub fn format_snippet_detail(snippet: &Snippet) -> String {
    let mut out = String::new();
    let lang = snippet.detect_or_language();

    out.push_str(&format!("\n\x1b[1;36mSnippet:\x1b[0m \x1b[1;37m{}\x1b[0m\n", snippet.name));
    out.push_str(&format!("  \x1b[2;37m• Language:\x1b[0m \x1b[1;33m{}\x1b[0m\n", lang));
    out.push_str(&format!("  \x1b[2;37m• Lines:\x1b[0m    {}\n", snippet.line_count()));
    out.push_str(&format!("  \x1b[2;37m• Bytes:\x1b[0m    {} B\n", snippet.byte_size()));
    out.push_str(&format!("  \x1b[2;37m• Updated:\x1b[0m  {}\n", snippet.updated_at));

    if let Some(desc) = &snippet.description {
        out.push_str(&format!("  \x1b[2;37m• Memo:\x1b[0m     {}\n", desc));
    }
    if !snippet.tags.is_empty() {
        out.push_str(&format!(
            "  \x1b[2;37m• Tags:\x1b[0m     {}\n",
            snippet.tags.join(", ")
        ));
    }

    out.push_str("\n\x1b[2;37m--------------------------------------------------\x1b[0m\n");
    out.push_str(&snippet.content);
    if !snippet.content.ends_with('\n') {
        out.push('\n');
    }
    out.push_str("\x1b[2;37m--------------------------------------------------\x1b[0m\n");

    out
}

/// Formats the help and usage overview for `/snippet`.
pub fn format_snippet_help() -> String {
    r#"
# Slash Command: `/snippet <subcommand>`

Manage and recall reusable code snippets persisted in `~/.fusion/snippets/`.

### Subcommands
- `/snippet save <name> [content...]` - Save code snippet (auto-extracts from last turn if content omitted).
- `/snippet insert <name>` - Recall and inject code snippet into active conversation.
- `/snippet recall <name>` or `/snippet show <name>` - Display snippet details and code.
- `/snippet list [filter]` - List all saved snippets.
- `/snippet search <query>` - Search snippets by name, tags, description, or content.
- `/snippet delete <name>` - Delete a snippet from storage.
- `/snippet export [path]` - Export all snippets to a JSON file.
- `/snippet import <path>` - Import snippets from a JSON file.
- `/snippet clear` - Delete all snippets.
- `/snippet help` - Show this help guide.

### Examples
- `/snippet save auth-handler fn authenticate(token: &str) -> bool { ... }`
- `/snippet save tokio-main` (extracts recent code block from conversation)
- `/snippet insert auth-handler`
- `/snippet show auth-handler`
- `/snippet list`
- `/snippet delete auth-handler`
"#
    .trim()
    .to_string()
}

// ============================================================================
// Slash Command Handler
// ============================================================================

/// Handles interactive `/snippet` slash commands.
pub fn handle_snippet_command(args: &[String], session: &mut Session) -> String {
    if args.is_empty() {
        match list_snippets() {
            Ok(snippets) => return format_snippet_table(&snippets),
            Err(e) => return format!("\x1b[1;31mError listing snippets:\x1b[0m {}", e),
        }
    }

    let subcmd = args[0].to_lowercase();
    match subcmd.as_str() {
        "list" | "ls" => {
            let filter = args.get(1).map(|s| s.as_str());
            match list_snippets() {
                Ok(snippets) => {
                    let filtered: Vec<Snippet> = if let Some(f) = filter {
                        let q = f.to_lowercase();
                        snippets
                            .into_iter()
                            .filter(|s| {
                                s.name.to_lowercase().contains(&q)
                                    || s.detect_or_language().to_lowercase().contains(&q)
                                    || s.tags.iter().any(|t| t.to_lowercase().contains(&q))
                            })
                            .collect()
                    } else {
                        snippets
                    };
                    format_snippet_table(&filtered)
                }
                Err(e) => format!("\x1b[1;31mError listing snippets:\x1b[0m {}", e),
            }
        }
        "save" | "add" | "new" => {
            if args.len() < 2 {
                return "\x1b[1;31mUsage:\x1b[0m /snippet save <name> [content...]".to_string();
            }
            let name = &args[1];

            // Determine content
            let (lang_opt, content) = if args.len() > 2 {
                let inline_content = args[2..].join(" ");
                (detect_code_language(&inline_content), inline_content)
            } else {
                // Try extracting last code block from session
                match extract_last_code_snippet(session) {
                    Some((lang, code)) => (lang, code),
                    None => {
                        return format!(
                            "\x1b[1;31mError:\x1b[0m No code snippet content provided and no code block found in recent turns.\n\
                             \x1b[2;37mUsage:\x1b[0m \x1b[1;36m/snippet save {} <content...>\x1b[0m",
                            name
                        );
                    }
                }
            };

            let mut snippet = Snippet::new(name, content);
            if let Some(lang) = lang_opt {
                snippet = snippet.with_language(lang);
            }

            match save_snippet(&snippet) {
                Ok(path) => {
                    format!(
                        "\x1b[1;32m✓ Snippet saved:\x1b[0m 📝 \x1b[1;37m{}\x1b[0m ({} lines, {}) in \x1b[2;37m{}\x1b[0m",
                        snippet.name,
                        snippet.line_count(),
                        snippet.detect_or_language(),
                        path.display()
                    )
                }
                Err(e) => format!("\x1b[1;31mFailed to save snippet:\x1b[0m {}", e),
            }
        }
        "insert" | "load" | "use" | "paste" => {
            if args.len() < 2 {
                return "\x1b[1;31mUsage:\x1b[0m /snippet insert <name>".to_string();
            }
            let name = &args[1];
            match insert_snippet(name, session) {
                Ok(_) => {
                    format!(
                        "\x1b[1;32m✓ Injected snippet:\x1b[0m \x1b[1;37m{}\x1b[0m into active session prompt.",
                        name
                    )
                }
                Err(e) => format!("\x1b[1;31mFailed to insert snippet:\x1b[0m {}", e),
            }
        }
        "recall" | "show" | "view" | "get" | "cat" => {
            if args.len() < 2 {
                return "\x1b[1;31mUsage:\x1b[0m /snippet show <name>".to_string();
            }
            let name = &args[1];
            match recall_snippet(name) {
                Ok(snippet) => format_snippet_detail(&snippet),
                Err(e) => format!("\x1b[1;31mRecall failed:\x1b[0m {}", e),
            }
        }
        "delete" | "del" | "rm" | "remove" => {
            if args.len() < 2 {
                return "\x1b[1;31mUsage:\x1b[0m /snippet delete <name>".to_string();
            }
            let name = &args[1];
            match delete_snippet(name) {
                Ok(true) => format!("\x1b[1;32m✓ Deleted snippet:\x1b[0m \x1b[1;37m{}\x1b[0m", name),
                Ok(false) => format!("\x1b[1;33mSnippet not found:\x1b[0m \x1b[1;37m{}\x1b[0m", name),
                Err(e) => format!("\x1b[1;31mFailed to delete snippet:\x1b[0m {}", e),
            }
        }
        "search" | "find" | "grep" => {
            if args.len() < 2 {
                return "\x1b[1;31mUsage:\x1b[0m /snippet search <query>".to_string();
            }
            let query = args[1..].join(" ");
            match search_snippets(&query) {
                Ok(results) => format_snippet_table(&results),
                Err(e) => format!("\x1b[1;31mSearch failed:\x1b[0m {}", e),
            }
        }
        "clear" => {
            match clear_snippets() {
                Ok(count) => format!("\x1b[1;32m✓ Cleared {} snippet(s) from storage.\x1b[0m", count),
                Err(e) => format!("\x1b[1;31mFailed to clear snippets:\x1b[0m {}", e),
            }
        }
        "export" => {
            let export_path = if args.len() > 1 {
                PathBuf::from(&args[1])
            } else {
                Config::config_dir().join("exports").join("snippets.json")
            };

            match export_snippets_json(&export_path) {
                Ok(count) => format!(
                    "\x1b[1;32m✓ Exported {} snippet(s) to:\x1b[0m \x1b[1;37m{}\x1b[0m",
                    count,
                    export_path.display()
                ),
                Err(e) => format!("\x1b[1;31mFailed to export snippets:\x1b[0m {}", e),
            }
        }
        "import" => {
            if args.len() < 2 {
                return "\x1b[1;31mUsage:\x1b[0m /snippet import <path>".to_string();
            }
            let import_path = PathBuf::from(&args[1]);
            match import_snippets_json(&import_path) {
                Ok(count) => format!(
                    "\x1b[1;32m✓ Imported {} snippet(s) from:\x1b[0m \x1b[1;37m{}\x1b[0m",
                    count,
                    import_path.display()
                ),
                Err(e) => format!("\x1b[1;31mFailed to import snippets:\x1b[0m {}", e),
            }
        }
        "help" | "-h" | "--help" => format_snippet_help(),
        other => {
            // Check if user ran `/snippet <name>` as shorthand for recall/show
            if let Ok(Some(snippet)) = get_snippet(other) {
                format_snippet_detail(&snippet)
            } else {
                format!(
                    "\x1b[1;31mUnknown snippet subcommand or snippet not found:\x1b[0m \x1b[1;37m{}\x1b[0m\n\
                     \x1b[2;37mType \x1b[1;36m/snippet help\x1b[0m\x1b[2;37m for available commands.\x1b[0m",
                    other
                )
            }
        }
    }
}

// ============================================================================
// Unit Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn test_snippet_creation_and_builder() {
        let snippet = Snippet::new("auth-fn", "fn authenticate(token: &str) -> bool {\n    !token.is_empty()\n}")
            .with_description("Simple token validator")
            .with_language("rust")
            .with_tags(&["auth", "security"])
            .with_metadata("author", "fusion");

        assert_eq!(snippet.name, "auth-fn");
        assert_eq!(snippet.language.as_deref(), Some("rust"));
        assert_eq!(snippet.description.as_deref(), Some("Simple token validator"));
        assert_eq!(snippet.tags, vec!["auth", "security"]);
        assert_eq!(snippet.metadata.get("author").unwrap(), "fusion");
        assert_eq!(snippet.line_count(), 3);
        assert!(snippet.is_multiline());
        assert_eq!(snippet.detect_or_language(), "rust");
    }

    #[test]
    fn test_detect_code_language() {
        assert_eq!(detect_code_language("pub struct User { id: u64 }"), Some("rust".to_string()));
        assert_eq!(detect_code_language("def calculate_sum(a, b):\n    return a + b"), Some("python".to_string()));
        assert_eq!(detect_code_language("const greet = (name: string): void => console.log(name);"), Some("typescript".to_string()));
        assert_eq!(detect_code_language("func main() {\n    fmt.Println(\"Hello\")\n}"), Some("go".to_string()));
        assert_eq!(detect_code_language("#include <iostream>\nint main() { return 0; }"), Some("cpp".to_string()));
        assert_eq!(detect_code_language("SELECT * FROM users WHERE active = 1;"), Some("sql".to_string()));
        assert_eq!(detect_code_language("{\"key\": \"value\", \"count\": 42}"), Some("json".to_string()));
        assert_eq!(detect_code_language("#!/bin/bash\necho 'hello world'"), Some("bash".to_string()));
    }

    #[test]
    fn test_extract_code_blocks() {
        let text = r#"
Here is a function in Rust:
```rust
fn add(x: i32, y: i32) -> i32 {
    x + y
}
```

And in Python:
```python
def add(x, y):
    return x + y
```
"#;
        let blocks = extract_code_blocks(text);
        assert_eq!(blocks.len(), 2);
        assert_eq!(blocks[0].0.as_deref(), Some("rust"));
        assert!(blocks[0].1.contains("fn add"));
        assert_eq!(blocks[1].0.as_deref(), Some("python"));
        assert!(blocks[1].1.contains("def add"));
    }

    #[test]
    fn test_extract_last_code_snippet_from_session() {
        let mut session = Session::new("gpt-4o");
        session.add_user_message("Can you write a binary search algorithm?");
        session.add_assistant_message("Sure! Here is the implementation:\n```rust\nfn bsearch(arr: &[i32], target: i32) -> Option<usize> {\n    arr.binary_search(&target).ok()\n}\n```");

        let extracted = extract_last_code_snippet(&session);
        assert!(extracted.is_some());
        let (lang, code) = extracted.unwrap();
        assert_eq!(lang.as_deref(), Some("rust"));
        assert!(code.contains("fn bsearch"));
    }

    #[test]
    fn test_snippet_save_load_delete_in_dir() {
        let temp = tempdir().unwrap();
        let dir = temp.path().to_path_buf();

        let s1 = Snippet::new("quick-sort", "fn quick_sort(arr: &mut [i32]) {}").with_language("rust");
        let path = save_snippet_to(&dir, &s1).expect("save snippet");
        assert!(path.exists());

        let loaded = load_snippet_from(&dir, "quick-sort").expect("load snippet");
        assert_eq!(loaded.name, "quick-sort");
        assert_eq!(loaded.content, s1.content);

        let list = list_snippets_from(&dir).expect("list snippets");
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].name, "quick-sort");

        let deleted = delete_snippet_from(&dir, "quick-sort").expect("delete snippet");
        assert!(deleted);

        assert!(load_snippet_from(&dir, "quick-sort").is_err());
    }

    #[test]
    fn test_snippet_manager_lifecycle() {
        let temp = tempdir().unwrap();
        let mut mgr = SnippetManager::with_dir(temp.path().to_path_buf());

        assert!(mgr.is_empty());
        assert_eq!(mgr.len(), 0);

        let s1 = Snippet::new("fib", "def fib(n):\n    return n if n <= 1 else fib(n-1) + fib(n-2)")
            .with_description("Recursive fibonacci")
            .with_tags(&["math", "recursion"]);
        let s2 = Snippet::new("merge-sort", "fn merge_sort(slice: &mut [i32]) {}")
            .with_tags(&["algorithm", "sorting"]);

        mgr.save(s1).unwrap();
        mgr.save(s2).unwrap();

        assert_eq!(mgr.len(), 2);
        assert!(mgr.get("fib").is_some());
        assert!(mgr.get("FIB").is_some()); // case-insensitive lookup
        assert!(mgr.get("merge-sort").is_some());

        // Recall
        let recalled = mgr.recall("fib").unwrap();
        assert_eq!(recalled.name, "fib");

        // Search
        let search_math = mgr.search("math");
        assert_eq!(search_math.len(), 1);
        assert_eq!(search_math[0].name, "fib");

        let search_algo = mgr.search("algorithm");
        assert_eq!(search_algo.len(), 1);
        assert_eq!(search_algo[0].name, "merge-sort");

        // Filter by tag
        let tag_math = mgr.filter_by_tag("math");
        assert_eq!(tag_math.len(), 1);

        // Export and import
        let export_file = temp.path().join("backup.json");
        let exported_count = mgr.export_json(&export_file).unwrap();
        assert_eq!(exported_count, 2);
        assert!(export_file.exists());

        // Clear
        let cleared = mgr.clear().unwrap();
        assert_eq!(cleared, 2);
        assert_eq!(mgr.len(), 0);

        // Import back
        let imported_count = mgr.import_json(&export_file).unwrap();
        assert_eq!(imported_count, 2);
        assert_eq!(mgr.len(), 2);
    }

    #[test]
    fn test_snippet_insertion_into_session() {
        let temp = tempdir().unwrap();
        let mut mgr = SnippetManager::with_dir(temp.path().to_path_buf());

        let snippet = Snippet::new("tokio-server", "async fn run_server() -> Result<(), Box<dyn Error>> {\n    Ok(())\n}")
            .with_language("rust");
        mgr.save(snippet).unwrap();

        let mut session = Session::new("gpt-4o");
        let formatted = mgr.insert("tokio-server", &mut session).expect("insert snippet");

        assert!(formatted.contains("tokio-server"));
        assert!(formatted.contains("async fn run_server"));
        assert_eq!(session.total_messages(), 1);
        assert_eq!(session.messages()[0].role, Role::User);
        assert!(session.messages()[0].content.contains("tokio-server"));
    }

    #[test]
    fn test_handle_snippet_command_dispatch() {
        let temp = tempdir().unwrap();
        let _ = fs::create_dir_all(temp.path());

        let mut session = Session::new("gpt-4o");
        session.add_assistant_message("Here is the code:\n```rust\nfn test_helper() -> i32 { 42 }\n```");

        // 1. Save with auto-extracted code
        let save_args = vec!["save".to_string(), "test-snippet".to_string()];
        let save_res = handle_snippet_command(&save_args, &mut session);
        assert!(save_res.contains("Snippet saved") || save_res.contains("test-snippet"));

        // 2. List
        let list_args = vec!["list".to_string()];
        let list_res = handle_snippet_command(&list_args, &mut session);
        assert!(list_res.contains("test-snippet"));

        // 3. Show / Recall
        let show_args = vec!["show".to_string(), "test-snippet".to_string()];
        let show_res = handle_snippet_command(&show_args, &mut session);
        assert!(show_res.contains("test_helper"));

        // 4. Insert
        let insert_args = vec!["insert".to_string(), "test-snippet".to_string()];
        let insert_res = handle_snippet_command(&insert_args, &mut session);
        assert!(insert_res.contains("Injected snippet"));
        assert!(session.messages().iter().any(|m| m.content.contains("test-snippet")));

        // 5. Search
        let search_args = vec!["search".to_string(), "helper".to_string()];
        let search_res = handle_snippet_command(&search_args, &mut session);
        assert!(search_res.contains("test-snippet"));

        // 6. Delete
        let del_args = vec!["delete".to_string(), "test-snippet".to_string()];
        let del_res = handle_snippet_command(&del_args, &mut session);
        assert!(del_res.contains("Deleted snippet"));

        // 7. Help
        let help_args = vec!["help".to_string()];
        let help_res = handle_snippet_command(&help_args, &mut session);
        assert!(help_res.contains("/snippet save"));
    }
}
