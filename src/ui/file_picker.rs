//! Fast In-Terminal Fuzzy File Finder (Ctrl+P / `/file`)
//!
//! Provides a high-performance, keyboard-driven fuzzy file picker with:
//! - Pure Rust fuzzy matching with subsequence alignment, boundary bonuses,
//!   camelCase detection, filename priority scoring, and exact substring rewards.
//! - Gitignore-aware workspace scanner using `ignore::WalkBuilder`.
//! - Ratatui inline list rendering with matched character highlighting.
//! - Rich file metadata: file types, language badges, formatted file sizes, directory paths.
//! - Full keyboard controls: `Ctrl+P`/`Ctrl+N`/arrows to navigate, `Ctrl+H` for hidden files,
//!   `Ctrl+U` to clear, `Enter` to select, `Esc` to cancel.
//! - Seamless execution inside an inline Ratatui viewport or full-screen frame.

use crossterm::{
    cursor,
    event::{self, Event, KeyCode, KeyEventKind, KeyModifiers},
    execute,
};
use ignore::WalkBuilder;
use ratatui::{
    buffer::Buffer,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Paragraph, Widget},
    Frame,
};
use std::io::{stdout, Write};
use std::path::{Path, PathBuf};

use crate::ui::inline::InlineTerminal;
use crate::ui::prompt::RawModeGuard;

// ---------------------------------------------------------------------------
// File Types & Badges
// ---------------------------------------------------------------------------

/// Detected file category for icon and badge rendering.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FileType {
    Rust,
    TypeScript,
    JavaScript,
    Python,
    Go,
    C,
    Cpp,
    Markdown,
    Config,
    Shell,
    Web,
    Git,
    Sql,
    Directory,
    Other,
}

impl FileType {
    /// Detects file category from file extension and filename.
    pub fn from_path(path: &Path) -> Self {
        if path.is_dir() {
            return FileType::Directory;
        }

        let file_name = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or_default()
            .to_ascii_lowercase();

        if file_name.starts_with(".git") || file_name == ".gitignore" || file_name == ".gitmodules" {
            return FileType::Git;
        }

        if file_name == "cargo.toml"
            || file_name == "cargo.lock"
            || file_name == "package.json"
            || file_name == "tsconfig.json"
            || file_name == "dockerfile"
            || file_name.ends_with(".toml")
            || file_name.ends_with(".json")
            || file_name.ends_with(".yaml")
            || file_name.ends_with(".yml")
            || file_name.ends_with(".ini")
            || file_name.ends_with(".env")
        {
            return FileType::Config;
        }

        let ext = path
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or_default()
            .to_ascii_lowercase();

        Self::from_extension(&ext)
    }

    /// Detects category from extension string.
    pub fn from_extension(ext: &str) -> Self {
        match ext {
            "rs" => FileType::Rust,
            "ts" | "tsx" | "mts" | "cts" => FileType::TypeScript,
            "js" | "jsx" | "mjs" | "cjs" => FileType::JavaScript,
            "py" | "pyi" | "pyw" => FileType::Python,
            "go" => FileType::Go,
            "c" | "h" => FileType::C,
            "cpp" | "cc" | "cxx" | "hpp" | "hh" | "hxx" => FileType::Cpp,
            "md" | "markdown" | "txt" | "rst" | "adoc" => FileType::Markdown,
            "json" | "toml" | "yaml" | "yml" | "xml" | "ini" | "env" | "conf" => FileType::Config,
            "sh" | "bash" | "zsh" | "fish" | "bat" | "cmd" | "ps1" => FileType::Shell,
            "html" | "htm" | "css" | "scss" | "sass" | "less" | "vue" | "svelte" => FileType::Web,
            "sql" | "sqlite" | "db" => FileType::Sql,
            _ => FileType::Other,
        }
    }

    /// Short textual badge formatted for compact terminal display.
    pub fn badge(&self) -> &'static str {
        match self {
            FileType::Rust => "[RS]",
            FileType::TypeScript => "[TS]",
            FileType::JavaScript => "[JS]",
            FileType::Python => "[PY]",
            FileType::Go => "[GO]",
            FileType::C => "[C ]",
            FileType::Cpp => "[C+]",
            FileType::Markdown => "[MD]",
            FileType::Config => "[CF]",
            FileType::Shell => "[SH]",
            FileType::Web => "[UI]",
            FileType::Git => "[GT]",
            FileType::Sql => "[DB]",
            FileType::Directory => "[DIR]",
            FileType::Other => "[--]",
        }
    }

    /// Color associated with this file category.
    pub fn color(&self) -> Color {
        match self {
            FileType::Rust => Color::Rgb(222, 100, 50),     // Rust Orange
            FileType::TypeScript => Color::Rgb(49, 120, 198), // TS Blue
            FileType::JavaScript => Color::Rgb(247, 223, 30), // JS Yellow
            FileType::Python => Color::Rgb(53, 114, 165),   // Python Blue
            FileType::Go => Color::Rgb(0, 173, 216),        // Go Cyan
            FileType::C => Color::Rgb(85, 85, 85),          // C Gray
            FileType::Cpp => Color::Rgb(243, 75, 125),      // C++ Pink/Red
            FileType::Markdown => Color::Rgb(140, 180, 240),// Markdown Light Blue
            FileType::Config => Color::Rgb(220, 170, 70),   // Config Gold
            FileType::Shell => Color::Rgb(78, 186, 111),    // Shell Green
            FileType::Web => Color::Rgb(228, 77, 38),       // HTML/CSS Red
            FileType::Git => Color::Rgb(240, 80, 50),       // Git Red
            FileType::Sql => Color::Rgb(218, 112, 214),     // SQL Orchid
            FileType::Directory => Color::Cyan,
            FileType::Other => Color::DarkGray,
        }
    }
}

// ---------------------------------------------------------------------------
// File Entry Metadata
// ---------------------------------------------------------------------------

/// Metadata representation of a single file in the workspace.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileEntry {
    /// Full filesystem path.
    pub path: PathBuf,
    /// Path relative to the workspace root with standardized forward slashes.
    pub relative_path: String,
    /// Bare filename (e.g. `file_picker.rs`).
    pub file_name: String,
    /// Lowercase file extension without dot.
    pub extension: Option<String>,
    /// File size in bytes.
    pub size_bytes: u64,
    /// Whether this entry is a directory.
    pub is_dir: bool,
    /// Whether this file is hidden (starts with `.`).
    pub is_hidden: bool,
    /// Categorized file type.
    pub file_type: FileType,
}

impl FileEntry {
    /// Creates a new `FileEntry` with metadata derived from path.
    pub fn new(path: PathBuf, base_dir: &Path, is_dir: bool, size_bytes: u64, is_hidden: bool) -> Self {
        let rel_path = path.strip_prefix(base_dir).unwrap_or(&path);
        let relative_path = rel_path.to_string_lossy().replace('\\', "/");
        let file_name = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or_default()
            .to_string();
        let extension = path
            .extension()
            .and_then(|e| e.to_str())
            .map(|s| s.to_ascii_lowercase());
        let file_type = FileType::from_path(&path);

        Self {
            path,
            relative_path,
            file_name,
            extension,
            size_bytes,
            is_dir,
            is_hidden,
            file_type,
        }
    }

    /// Formats the file size as human-readable (e.g. `12.4 KB`, `1.2 MB`).
    pub fn formatted_size(&self) -> String {
        if self.is_dir {
            return "DIR".to_string();
        }

        let bytes = self.size_bytes;
        if bytes < 1024 {
            format!("{} B", bytes)
        } else if bytes < 1024 * 1024 {
            format!("{:.1} KB", bytes as f64 / 1024.0)
        } else if bytes < 1024 * 1024 * 1024 {
            format!("{:.1} MB", bytes as f64 / (1024.0 * 1024.0))
        } else {
            format!("{:.2} GB", bytes as f64 / (1024.0 * 1024.0 * 1024.0))
        }
    }

    /// Returns the parent directory portion of the relative path.
    pub fn parent_dir(&self) -> &str {
        if let Some(pos) = self.relative_path.rfind('/') {
            &self.relative_path[..=pos]
        } else {
            ""
        }
    }

    /// Returns the filename stem without extension.
    pub fn file_stem(&self) -> &str {
        if let Some(pos) = self.file_name.rfind('.') {
            if pos > 0 {
                &self.file_name[..pos]
            } else {
                &self.file_name
            }
        } else {
            &self.file_name
        }
    }
}

// ---------------------------------------------------------------------------
// Pure Rust Fuzzy Match Engine
// ---------------------------------------------------------------------------

/// Result of a fuzzy match query against a candidate string.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FuzzyMatchResult {
    /// Calculated match score (higher is better).
    pub score: i64,
    /// Indices of matched characters in the candidate string for visual highlighting.
    pub matched_indices: Vec<usize>,
}

/// Computes a fuzzy match score and matched character indices between pattern and candidate.
///
/// Features:
/// - Case-insensitive match with case-preservation bonuses.
/// - Exact match, prefix match, and substring rewards.
/// - Word boundary bonuses (following `/`, `_`, `-`, `.`, spaces, or camelCase).
/// - Filename component weighting (matches in filename scored significantly higher than directory paths).
/// - Consecutive character match bonuses.
/// - Start position and length penalties.
pub fn fuzzy_match(pattern: &str, candidate: &str) -> Option<FuzzyMatchResult> {
    if pattern.is_empty() {
        return Some(FuzzyMatchResult {
            score: 0,
            matched_indices: Vec::new(),
        });
    }

    if candidate.is_empty() {
        return None;
    }

    let pattern_chars: Vec<char> = pattern.chars().collect();
    let candidate_chars: Vec<char> = candidate.chars().collect();

    // 1. Fast Subsequence Check (Case-Insensitive)
    let mut p_idx = 0;
    for &c in &candidate_chars {
        if p_idx < pattern_chars.len()
            && c.to_ascii_lowercase() == pattern_chars[p_idx].to_ascii_lowercase()
        {
            p_idx += 1;
        }
    }
    if p_idx < pattern_chars.len() {
        return None; // Pattern is not a subsequence
    }

    // 2. Exact Match Checks
    let pattern_lower = pattern.to_ascii_lowercase();
    let candidate_lower = candidate.to_ascii_lowercase();

    if pattern_lower == candidate_lower {
        let indices = (0..candidate_chars.len()).collect();
        return Some(FuzzyMatchResult {
            score: 500 + (candidate_chars.len() as i64 * 10),
            matched_indices: indices,
        });
    }

    // Find the last path separator to distinguish filename from directory
    let last_slash_idx = candidate.rfind('/').or_else(|| candidate.rfind('\\'));
    let filename_start = last_slash_idx.map(|idx| idx + 1).unwrap_or(0);

    // Exact filename match bonus
    let filename_str = &candidate[filename_start..];
    if filename_str.eq_ignore_ascii_case(pattern) {
        let indices = (filename_start..candidate_chars.len()).collect();
        return Some(FuzzyMatchResult {
            score: 350 + (pattern_chars.len() as i64 * 15),
            matched_indices: indices,
        });
    }

    // Exact Substring Match Bonus
    if let Some(pos) = candidate_lower.find(&pattern_lower) {
        let char_pos = candidate[..pos].chars().count();
        let pat_len = pattern_chars.len();
        let indices: Vec<usize> = (char_pos..char_pos + pat_len).collect();

        let mut score = 200 + (pat_len as i64 * 20);
        if pos == filename_start {
            score += 100; // Exact prefix of filename
        } else if pos == 0 {
            score += 80;  // Exact prefix of entire path
        }
        score -= (char_pos as i64) * 2;
        score -= candidate_chars.len() as i64 - pat_len as i64;

        return Some(FuzzyMatchResult {
            score,
            matched_indices: indices,
        });
    }

    // 3. Optimal Alignment Scoring
    // We compute the best matching indices using greedy boundary-preferring alignment
    let mut matched_indices = Vec::with_capacity(pattern_chars.len());
    let mut cand_idx = 0;
    let mut score: i64 = 0;
    let mut consecutive_count = 0;

    for (pi, &p_char) in pattern_chars.iter().enumerate() {
        let p_lower = p_char.to_ascii_lowercase();
        let mut best_idx = None;
        let mut best_local_score = i64::MIN;

        // Search for matching character in candidate
        while cand_idx < candidate_chars.len() {
            let c = candidate_chars[cand_idx];
            let c_lower = c.to_ascii_lowercase();

            if c_lower == p_lower {
                let mut char_score: i64 = 10;

                // Consecutive match bonus
                if let Some(&last_idx) = matched_indices.last() {
                    if cand_idx == last_idx + 1 {
                        consecutive_count += 1;
                        char_score += 15 * consecutive_count;
                    } else {
                        consecutive_count = 0;
                    }
                }

                // Word boundary bonus
                let is_boundary = if cand_idx == 0 {
                    true
                } else {
                    let prev = candidate_chars[cand_idx - 1];
                    prev == '/' || prev == '\\' || prev == '_' || prev == '-' || prev == '.' || prev == ' ' || prev == ':'
                };

                if is_boundary {
                    char_score += 25;
                }

                // CamelCase boundary bonus
                if cand_idx > 0 {
                    let prev = candidate_chars[cand_idx - 1];
                    if prev.is_ascii_lowercase() && c.is_ascii_uppercase() {
                        char_score += 20;
                    }
                }

                // Filename match bonus
                if cand_idx >= filename_start {
                    char_score += 30;
                }

                // Exact case match bonus
                if p_char == c {
                    char_score += 5;
                }

                // Start position penalty (earlier matches score higher)
                char_score -= (cand_idx as i64) / 4;

                if char_score > best_local_score {
                    best_local_score = char_score;
                    best_idx = Some(cand_idx);
                }

                // If this is a high-value boundary or consecutive, commit immediately
                if is_boundary || (cand_idx >= filename_start && pi == 0) {
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
            // Should not happen if subsequence check passed, but fallback safely
            return None;
        }
    }

    // Final penalties for total length and gap distances
    let len_diff = candidate_chars.len().saturating_sub(pattern_chars.len());
    score -= (len_diff as i64) / 2;

    if let Some(&first_idx) = matched_indices.first() {
        if first_idx >= filename_start {
            score += 40; // Entire match is contained in the filename
        }
    }

    Some(FuzzyMatchResult {
        score,
        matched_indices,
    })
}

/// Searches and ranks a collection of `FileEntry` candidates against a query string.
pub fn fuzzy_search<'a>(
    pattern: &str,
    entries: &'a [FileEntry],
    max_results: usize,
) -> Vec<(&'a FileEntry, FuzzyMatchResult)> {
    if pattern.trim().is_empty() {
        return entries
            .iter()
            .take(max_results)
            .map(|e| {
                (
                    e,
                    FuzzyMatchResult {
                        score: 0,
                        matched_indices: Vec::new(),
                    },
                )
            })
            .collect();
    }

    let mut matches: Vec<(&'a FileEntry, FuzzyMatchResult)> = entries
        .iter()
        .filter_map(|entry| {
            // Check relative path first, then filename
            fuzzy_match(pattern, &entry.relative_path).map(|res| (entry, res))
        })
        .collect();

    // Sort descending by score; ties broken by shorter path then alphabetical
    matches.sort_by(|a, b| {
        b.1.score
            .cmp(&a.1.score)
            .then_with(|| a.0.relative_path.len().cmp(&b.0.relative_path.len()))
            .then_with(|| a.0.relative_path.cmp(&b.0.relative_path))
    });

    if matches.len() > max_results {
        matches.truncate(max_results);
    }

    matches
}

// ---------------------------------------------------------------------------
// File Scanner
// ---------------------------------------------------------------------------

/// Workspace file scanner respecting `.gitignore` and skipping noisy build caches.
#[derive(Debug, Clone)]
pub struct FileScanner;

impl FileScanner {
    /// Recursively scans workspace root directory for files.
    pub fn scan(root: &Path, include_hidden: bool, max_files: usize) -> Vec<FileEntry> {
        let mut builder = WalkBuilder::new(root);
        builder
            .hidden(!include_hidden)
            .git_ignore(true)
            .git_global(true)
            .git_exclude(true)
            .parents(true);

        let mut entries = Vec::new();
        let walker = builder.build();

        for result in walker {
            let dir_entry = match result {
                Ok(e) => e,
                Err(err) => {
                    tracing::debug!("File scanner error: {err}");
                    continue;
                }
            };

            // Skip root directory entry itself
            if dir_entry.depth() == 0 {
                continue;
            }

            let path = dir_entry.path();

            // Filter out common massive build directories even if not in gitignore
            let file_name = path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or_default();

            if !include_hidden {
                if file_name == ".git"
                    || file_name == "node_modules"
                    || file_name == "target"
                    || file_name == "dist"
                    || file_name == "build"
                    || file_name == ".venv"
                    || file_name == "venv"
                    || file_name == "__pycache__"
                    || file_name == ".idea"
                    || file_name == ".vscode"
                {
                    continue;
                }
            }

            let is_dir = dir_entry.file_type().map(|ft| ft.is_dir()).unwrap_or(false);

            // Skip directories in file picker results for cleaner file navigation
            if is_dir {
                continue;
            }

            let size_bytes = dir_entry.metadata().map(|m| m.len()).unwrap_or(0);
            let is_hidden = file_name.starts_with('.');

            let entry = FileEntry::new(
                path.to_path_buf(),
                root,
                is_dir,
                size_bytes,
                is_hidden,
            );

            entries.push(entry);

            if entries.len() >= max_files {
                break;
            }
        }

        // Deterministic sorting: sort by relative path depth and alphabetical
        entries.sort_by(|a, b| {
            let a_depth = a.relative_path.matches('/').count();
            let b_depth = b.relative_path.matches('/').count();
            a_depth
                .cmp(&b_depth)
                .then_with(|| a.relative_path.cmp(&b.relative_path))
        });

        entries
    }
}

// ---------------------------------------------------------------------------
// File Picker Interactive Result
// ---------------------------------------------------------------------------

/// Outcome returned after interactive file picker session.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FilePickerResult {
    /// User selected a file entry (`Enter`).
    Selected(FileEntry),
    /// User canceled the picker without selection (`Esc` / `Ctrl+C`).
    Cancelled,
}

// ---------------------------------------------------------------------------
// Interactive FilePicker Widget
// ---------------------------------------------------------------------------

/// Interactive Fuzzy File Picker widget matching VS Code / Telescope UX.
#[derive(Debug, Clone)]
pub struct FilePicker {
    /// Scanned file catalog.
    entries: Vec<FileEntry>,
    /// Filtered matches containing (entry_index, score, matched_indices).
    filtered_indices: Vec<(usize, i64, Vec<usize>)>,
    /// Search query string typed by user.
    query: String,
    /// Cursor position inside the query.
    cursor_pos: usize,
    /// Selection cursor within filtered results.
    selected_index: usize,
    /// Scroll offset for list viewport pagination.
    scroll_offset: usize,
    /// Search root directory.
    root_dir: PathBuf,
    /// Whether hidden files are included in the search.
    include_hidden: bool,
    /// Whether outer border block is drawn.
    show_border: bool,
    /// Custom title displayed in header.
    title: String,
    /// Whether syntax preview pane is enabled.
    preview_enabled: bool,
}

impl Default for FilePicker {
    fn default() -> Self {
        Self::new()
    }
}

impl FilePicker {
    /// Create a new `FilePicker` scanning the current working directory.
    pub fn new() -> Self {
        let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
        Self::from_directory(&cwd)
    }

    /// Create a `FilePicker` for a specific workspace directory.
    pub fn from_directory(root: impl AsRef<Path>) -> Self {
        let root_buf = root.as_ref().to_path_buf();
        let entries = FileScanner::scan(&root_buf, false, 10_000);
        let mut picker = Self {
            entries,
            filtered_indices: Vec::new(),
            query: String::new(),
            cursor_pos: 0,
            selected_index: 0,
            scroll_offset: 0,
            root_dir: root_buf,
            include_hidden: false,
            show_border: true,
            title: "File Finder".to_string(),
            preview_enabled: false,
        };
        picker.refilter();
        picker
    }

    /// Create a `FilePicker` with pre-populated files.
    pub fn with_files(files: Vec<FileEntry>) -> Self {
        let mut picker = Self {
            entries: files,
            filtered_indices: Vec::new(),
            query: String::new(),
            cursor_pos: 0,
            selected_index: 0,
            scroll_offset: 0,
            root_dir: PathBuf::from("."),
            include_hidden: false,
            show_border: true,
            title: "File Finder".to_string(),
            preview_enabled: false,
        };
        picker.refilter();
        picker
    }

    /// Set root search directory.
    pub fn with_root(mut self, root: impl Into<PathBuf>) -> Self {
        self.root_dir = root.into();
        self.rescan();
        self
    }

    /// Set custom title.
    pub fn with_title(mut self, title: impl Into<String>) -> Self {
        self.title = title.into();
        self
    }

    /// Set whether outer borders are drawn.
    pub fn with_border(mut self, show_border: bool) -> Self {
        self.show_border = show_border;
        self
    }

    /// Set initial hidden files visibility.
    pub fn with_hidden(mut self, include_hidden: bool) -> Self {
        self.include_hidden = include_hidden;
        self.rescan();
        self
    }

    /// Set whether preview panel is shown.
    pub fn with_preview(mut self, preview_enabled: bool) -> Self {
        self.preview_enabled = preview_enabled;
        self
    }

    /// Set initial query string.
    pub fn with_initial_query(mut self, query: impl Into<String>) -> Self {
        self.set_query(query);
        self
    }

    /// Re-scans the directory from disk.
    pub fn rescan(&mut self) {
        self.entries = FileScanner::scan(&self.root_dir, self.include_hidden, 10_000);
        self.refilter();
    }

    /// Returns current query string.
    pub fn query(&self) -> &str {
        &self.query
    }

    /// Update search query and recalculate fuzzy rankings.
    pub fn set_query(&mut self, query: impl Into<String>) {
        self.query = query.into();
        self.cursor_pos = self.query.chars().count();
        self.refilter();
    }

    /// Clears the active search query.
    pub fn clear_query(&mut self) {
        self.query.clear();
        self.cursor_pos = 0;
        self.refilter();
    }

    /// Recomputes fuzzy ranking for all files.
    pub fn refilter(&mut self) {
        if self.query.trim().is_empty() {
            self.filtered_indices = self
                .entries
                .iter()
                .enumerate()
                .map(|(idx, _)| (idx, 0, Vec::new()))
                .collect();
        } else {
            let mut matches: Vec<(usize, i64, Vec<usize>)> = self
                .entries
                .iter()
                .enumerate()
                .filter_map(|(idx, entry)| {
                    fuzzy_match(&self.query, &entry.relative_path)
                        .map(|res| (idx, res.score, res.matched_indices))
                })
                .collect();

            matches.sort_by(|a, b| {
                b.1.cmp(&a.1)
                    .then_with(|| {
                        self.entries[a.0]
                            .relative_path
                            .len()
                            .cmp(&self.entries[b.0].relative_path.len())
                    })
                    .then_with(|| {
                        self.entries[a.0]
                            .relative_path
                            .cmp(&self.entries[b.0].relative_path)
                    })
            });

            self.filtered_indices = matches;
        }

        self.selected_index = 0;
        self.scroll_offset = 0;
    }

    /// Total number of indexed files.
    pub fn total_files_count(&self) -> usize {
        self.entries.len()
    }

    /// Number of matching files for the current query.
    pub fn matched_files_count(&self) -> usize {
        self.filtered_indices.len()
    }

    /// Currently selected `FileEntry`.
    pub fn selected_entry(&self) -> Option<&FileEntry> {
        self.filtered_indices
            .get(self.selected_index)
            .map(|(idx, _, _)| &self.entries[*idx])
    }

    /// All currently filtered entries.
    pub fn filtered_entries(&self) -> Vec<&FileEntry> {
        self.filtered_indices
            .iter()
            .map(|(idx, _, _)| &self.entries[*idx])
            .collect()
    }

    /// Toggle hidden files inclusion.
    pub fn toggle_hidden(&mut self) {
        self.include_hidden = !self.include_hidden;
        self.rescan();
    }

    /// Toggle preview panel.
    pub fn toggle_preview(&mut self) {
        self.preview_enabled = !self.preview_enabled;
    }

    // -----------------------------------------------------------------------
    // Navigation & Selection
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

    /// Move selection down by page size.
    pub fn select_page_down(&mut self, page_size: usize) {
        if !self.filtered_indices.is_empty() {
            self.selected_index = (self.selected_index + page_size).min(self.filtered_indices.len() - 1);
        }
    }

    /// Move selection up by page size.
    pub fn select_page_up(&mut self, page_size: usize) {
        self.selected_index = self.selected_index.saturating_sub(page_size);
    }

    /// Jump selection to first item.
    pub fn select_first(&mut self) {
        self.selected_index = 0;
    }

    /// Jump selection to last item.
    pub fn select_last(&mut self) {
        if !self.filtered_indices.is_empty() {
            self.selected_index = self.filtered_indices.len() - 1;
        }
    }

    // -----------------------------------------------------------------------
    // Query Text Editing
    // -----------------------------------------------------------------------

    /// Insert character into query string at cursor position.
    pub fn insert_char(&mut self, c: char) {
        let mut chars: Vec<char> = self.query.chars().collect();
        if self.cursor_pos <= chars.len() {
            chars.insert(self.cursor_pos, c);
            self.cursor_pos += 1;
            self.query = chars.into_iter().collect();
            self.refilter();
        }
    }

    /// Delete character before cursor (Backspace).
    pub fn delete_char_before_cursor(&mut self) {
        if self.cursor_pos > 0 {
            let mut chars: Vec<char> = self.query.chars().collect();
            if self.cursor_pos <= chars.len() {
                chars.remove(self.cursor_pos - 1);
                self.cursor_pos -= 1;
                self.query = chars.into_iter().collect();
                self.refilter();
            }
        }
    }

    /// Delete character at cursor (Delete).
    pub fn delete_char_at_cursor(&mut self) {
        let mut chars: Vec<char> = self.query.chars().collect();
        if self.cursor_pos < chars.len() {
            chars.remove(self.cursor_pos);
            self.query = chars.into_iter().collect();
            self.refilter();
        }
    }

    /// Delete word before cursor (Ctrl+W).
    pub fn delete_word_before_cursor(&mut self) {
        if self.cursor_pos == 0 {
            return;
        }
        let chars: Vec<char> = self.query.chars().collect();
        let mut pos = self.cursor_pos;

        // Skip trailing spaces
        while pos > 0 && chars[pos - 1].is_whitespace() {
            pos -= 1;
        }
        // Skip word characters
        while pos > 0 && !chars[pos - 1].is_whitespace() && chars[pos - 1] != '/' {
            pos -= 1;
        }

        let mut new_chars = Vec::new();
        new_chars.extend_from_slice(&chars[..pos]);
        new_chars.extend_from_slice(&chars[self.cursor_pos..]);
        self.cursor_pos = pos;
        self.query = new_chars.into_iter().collect();
        self.refilter();
    }

    /// Move cursor left.
    pub fn move_cursor_left(&mut self) {
        if self.cursor_pos > 0 {
            self.cursor_pos -= 1;
        }
    }

    /// Move cursor right.
    pub fn move_cursor_right(&mut self) {
        let char_count = self.query.chars().count();
        if self.cursor_pos < char_count {
            self.cursor_pos += 1;
        }
    }

    /// Move cursor to beginning of query line.
    pub fn move_cursor_home(&mut self) {
        self.cursor_pos = 0;
    }

    /// Move cursor to end of query line.
    pub fn move_cursor_end(&mut self) {
        self.cursor_pos = self.query.chars().count();
    }

    // -----------------------------------------------------------------------
    // Keyboard Event Handling
    // -----------------------------------------------------------------------

    /// Handles a single crossterm key event.
    pub fn handle_key(&mut self, code: KeyCode, modifiers: KeyModifiers) -> Option<FilePickerResult> {
        match (code, modifiers) {
            // Enter: Select current file
            (KeyCode::Enter, _) => {
                if let Some(entry) = self.selected_entry().cloned() {
                    Some(FilePickerResult::Selected(entry))
                } else {
                    None
                }
            }

            // Esc: Clear query or Cancel
            (KeyCode::Esc, _) => {
                if !self.query.is_empty() {
                    self.clear_query();
                    None
                } else {
                    Some(FilePickerResult::Cancelled)
                }
            }

            // Ctrl+C / Ctrl+D on empty query: Cancel
            (KeyCode::Char('c'), KeyModifiers::CONTROL) => Some(FilePickerResult::Cancelled),
            (KeyCode::Char('d'), KeyModifiers::CONTROL) if self.query.is_empty() => {
                Some(FilePickerResult::Cancelled)
            }

            // Up arrow / Ctrl+P / Ctrl+K: Navigate Up
            (KeyCode::Up, _)
            | (KeyCode::Char('p'), KeyModifiers::CONTROL)
            | (KeyCode::Char('k'), KeyModifiers::CONTROL) => {
                self.select_prev();
                None
            }

            // Down arrow / Ctrl+N / Ctrl+J: Navigate Down
            (KeyCode::Down, _)
            | (KeyCode::Char('n'), KeyModifiers::CONTROL)
            | (KeyCode::Char('j'), KeyModifiers::CONTROL) => {
                self.select_next();
                None
            }

            // PageUp / PageDown
            (KeyCode::PageUp, _) => {
                self.select_page_up(8);
                None
            }
            (KeyCode::PageDown, _) => {
                self.select_page_down(8);
                None
            }

            // Home / End
            (KeyCode::Home, KeyModifiers::NONE) | (KeyCode::Char('a'), KeyModifiers::CONTROL) => {
                self.move_cursor_home();
                None
            }
            (KeyCode::End, KeyModifiers::NONE) | (KeyCode::Char('e'), KeyModifiers::CONTROL) => {
                self.move_cursor_end();
                None
            }

            // Left / Right cursor movement
            (KeyCode::Left, KeyModifiers::NONE) => {
                self.move_cursor_left();
                None
            }
            (KeyCode::Right, KeyModifiers::NONE) => {
                self.move_cursor_right();
                None
            }

            // Backspace: Delete character before cursor
            (KeyCode::Backspace, _) => {
                self.delete_char_before_cursor();
                None
            }

            // Delete: Delete character at cursor
            (KeyCode::Delete, _) => {
                self.delete_char_at_cursor();
                None
            }

            // Ctrl+U: Clear entire query
            (KeyCode::Char('u'), KeyModifiers::CONTROL) => {
                self.clear_query();
                None
            }

            // Ctrl+W: Delete word before cursor
            (KeyCode::Char('w'), KeyModifiers::CONTROL) => {
                self.delete_word_before_cursor();
                None
            }

            // Ctrl+H / F2: Toggle hidden files
            (KeyCode::Char('h'), KeyModifiers::CONTROL) | (KeyCode::F(2), _) => {
                self.toggle_hidden();
                None
            }

            // Tab / Ctrl+O / F3: Toggle preview pane
            (KeyCode::Tab, KeyModifiers::NONE)
            | (KeyCode::Char('o'), KeyModifiers::CONTROL)
            | (KeyCode::F(3), _) => {
                self.toggle_preview();
                None
            }

            // Typing standard characters
            (KeyCode::Char(c), KeyModifiers::NONE) | (KeyCode::Char(c), KeyModifiers::SHIFT) => {
                self.insert_char(c);
                None
            }

            _ => None,
        }
    }

    // -----------------------------------------------------------------------
    // Ratatui Widget Rendering
    // -----------------------------------------------------------------------

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
        let (inner_area, _use_border) = if self.show_border && area.height >= 5 && area.width >= 25 {
            let count_info = format!("{}/{} files", self.matched_files_count(), self.total_files_count());
            let title_text = format!(" 🔍 {}  [{}] ", self.title, count_info);
            let block = Block::default()
                .title(title_text)
                .borders(Borders::ALL)
                .border_type(BorderType::Rounded)
                .border_style(Style::default().fg(Color::Cyan));
            let inner = block.inner(area);
            block.render(area, buf);
            (inner, true)
        } else {
            (area, false)
        };

        if inner_area.height < 2 {
            return;
        }

        // Vertical Layout:
        // Row 0: Search input bar
        // Row 1: Top divider (if height >= 6)
        // Rows 2..N-2: File results list
        // Row N-1: Key hints footer (if height >= 5)
        let has_divider = inner_area.height >= 6;
        let has_footer = inner_area.height >= 5;

        let mut constraints = vec![Constraint::Length(1)]; // Search input
        if has_divider {
            constraints.push(Constraint::Length(1)); // Divider
        }
        constraints.push(Constraint::Min(1)); // Files list
        if has_footer {
            constraints.push(Constraint::Length(1)); // Footer
        }

        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints(constraints)
            .split(inner_area);

        let search_area = chunks[0];
        let mut idx = 1;
        let divider_area = if has_divider {
            let a = chunks[idx];
            idx += 1;
            Some(a)
        } else {
            None
        };
        let list_area = chunks[idx];
        idx += 1;
        let footer_area = if has_footer {
            Some(chunks[idx])
        } else {
            None
        };

        // 1. Render Search Input Bar
        self.render_search_bar(search_area, buf);

        // 2. Render Divider
        if let Some(div_rect) = divider_area {
            let div_line = Line::from(vec![Span::styled(
                "─".repeat(div_rect.width as usize),
                Style::default().fg(Color::DarkGray),
            )]);
            Paragraph::new(div_line).render(div_rect, buf);
        }

        // 3. Render File List
        self.render_file_list(list_area, buf);

        // 4. Render Footer Key Hints
        if let Some(foot_rect) = footer_area {
            self.render_footer(foot_rect, buf);
        }
    }

    /// Renders search prompt, current query string, and cursor.
    fn render_search_bar(&self, area: Rect, buf: &mut Buffer) {
        let mut spans = Vec::new();

        spans.push(Span::styled(
            " ❯ ",
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ));

        if self.query.is_empty() {
            spans.push(Span::styled(
                "Type to search files (e.g. 'main.rs', 'picker', 'Cargo')...",
                Style::default().fg(Color::DarkGray),
            ));
        } else {
            let chars: Vec<char> = self.query.chars().collect();
            let before: String = chars.iter().take(self.cursor_pos).collect();
            let cursor_char = chars.get(self.cursor_pos).copied().unwrap_or(' ');
            let after: String = chars.iter().skip(self.cursor_pos + 1).collect();

            spans.push(Span::styled(
                before,
                Style::default()
                    .fg(Color::White)
                    .add_modifier(Modifier::BOLD),
            ));

            // Inverted cursor block
            spans.push(Span::styled(
                cursor_char.to_string(),
                Style::default()
                    .fg(Color::Black)
                    .bg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            ));

            spans.push(Span::styled(
                after,
                Style::default()
                    .fg(Color::White)
                    .add_modifier(Modifier::BOLD),
            ));
        }

        Paragraph::new(Line::from(spans)).render(area, buf);
    }

    /// Renders visible window of filtered file list entries.
    fn render_file_list(&self, area: Rect, buf: &mut Buffer) {
        if area.height == 0 || area.width == 0 {
            return;
        }

        if self.filtered_indices.is_empty() {
            let empty_msg = if self.entries.is_empty() {
                "  No files found in workspace."
            } else {
                "  No matching files."
            };
            Paragraph::new(Span::styled(empty_msg, Style::default().fg(Color::DarkGray)))
                .render(area, buf);
            return;
        }

        let max_visible = area.height as usize;
        let total = self.filtered_indices.len();

        // Adjust scroll offset to keep selected_index in view
        let scroll_offset = if self.selected_index < self.scroll_offset {
            self.selected_index
        } else if self.selected_index >= self.scroll_offset + max_visible {
            self.selected_index + 1 - max_visible
        } else {
            self.scroll_offset
        };

        let visible_range = scroll_offset..(scroll_offset + max_visible).min(total);

        for (row_idx, match_idx) in visible_range.enumerate() {
            let item_y = area.y + row_idx as u16;
            if item_y >= area.y + area.height {
                break;
            }

            let is_selected = match_idx == self.selected_index;
            let (entry_idx, _, ref matched_chars) = self.filtered_indices[match_idx];
            let entry = &self.entries[entry_idx];

            let row_line = format_file_row(entry, is_selected, matched_chars, area.width);
            let row_area = Rect::new(area.x, item_y, area.width, 1);

            Paragraph::new(row_line).render(row_area, buf);
        }
    }

    /// Renders footer containing keyboard shortcuts and workspace status.
    fn render_footer(&self, area: Rect, buf: &mut Buffer) {
        let hidden_tag = if self.include_hidden {
            "[Hidden: On]"
        } else {
            "[Hidden: Off]"
        };

        let spans = if area.width < 50 {
            vec![
                Span::styled("↑↓", Style::default().fg(Color::Cyan)),
                Span::raw(" Nav  "),
                Span::styled("Enter", Style::default().fg(Color::Cyan)),
                Span::raw(" Open  "),
                Span::styled("Esc", Style::default().fg(Color::Cyan)),
                Span::raw(" Close"),
            ]
        } else {
            vec![
                Span::styled("↑↓/Ctrl+P/N", Style::default().fg(Color::Cyan)),
                Span::raw(" Navigate  "),
                Span::styled("Enter", Style::default().fg(Color::Cyan)),
                Span::raw(" Select  "),
                Span::styled("Esc", Style::default().fg(Color::Cyan)),
                Span::raw(" Cancel  "),
                Span::styled("Ctrl+H", Style::default().fg(Color::Cyan)),
                Span::raw(format!(" {}  ", hidden_tag)),
                Span::styled("Ctrl+U", Style::default().fg(Color::Cyan)),
                Span::raw(" Clear"),
            ]
        };

        Paragraph::new(Line::from(spans)).render(area, buf);
    }

    // -----------------------------------------------------------------------
    // Interactive TUI Execution Loop
    // -----------------------------------------------------------------------

    /// Launch interactive TUI loop inside an inline Ratatui viewport.
    ///
    /// Automatically manages raw terminal mode, cursor visibility,
    /// dynamic height clamping, and keyboard event loop.
    pub fn run_interactive(&mut self, requested_height: Option<u16>) -> std::io::Result<Option<FileEntry>> {
        let _raw_guard = RawModeGuard::enter()?;
        let _ = execute!(stdout(), cursor::Hide);

        let (_cols, rows) = InlineTerminal::terminal_size();
        let height = requested_height.unwrap_or_else(|| {
            if rows <= 10 {
                rows.saturating_sub(1).max(4)
            } else if rows <= 20 {
                10
            } else {
                14
            }
        });

        let mut inline = InlineTerminal::new(height)?;

        let outcome = loop {
            // Draw current frame
            inline.draw(|f| {
                let area = f.area();
                f.render_widget(&*self, area);
            })?;

            // Poll for user keyboard input (50ms timeout for smooth response)
            if event::poll(std::time::Duration::from_millis(50))? {
                if let Event::Key(key) = event::read()? {
                    if key.kind == KeyEventKind::Release {
                        continue;
                    }

                    if let Some(result) = self.handle_key(key.code, key.modifiers) {
                        break match result {
                            FilePickerResult::Selected(entry) => Some(entry),
                            FilePickerResult::Cancelled => None,
                        };
                    }
                }
            }
        };

        // Clean up inline viewport and restore terminal cursor
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

impl Widget for &FilePicker {
    fn render(self, area: Rect, buf: &mut Buffer) {
        self.render_buffer(area, buf);
    }
}

impl Widget for FilePicker {
    fn render(self, area: Rect, buf: &mut Buffer) {
        self.render_buffer(area, buf);
    }
}

// ---------------------------------------------------------------------------
// Row Formatting Helper
// ---------------------------------------------------------------------------

/// Formats a single file entry row with cursor, badges, directory path, filename,
/// matched character highlighting, and right-aligned size.
pub fn format_file_row<'a>(
    entry: &'a FileEntry,
    is_selected: bool,
    matched_indices: &[usize],
    width: u16,
) -> Line<'a> {
    let mut spans = Vec::new();

    // 1. Selection indicator
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

    // 2. Type badge (e.g. `[RS]`, `[TS]`, `[MD]`)
    let badge_style = if is_selected {
        Style::default()
            .fg(entry.file_type.color())
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(entry.file_type.color())
    };
    spans.push(Span::styled(entry.file_type.badge(), badge_style));
    spans.push(Span::raw(" "));

    // 3. Path & Filename with matched character highlighting
    let size_str = entry.formatted_size();
    let size_width = size_str.chars().count() + 2; // "+2" for spacing
    let available_path_width = (width as usize).saturating_sub(spans_len(&spans) + size_width);

    let path_str = &entry.relative_path;
    let path_chars: Vec<char> = path_str.chars().collect();
    let parent_len = entry.parent_dir().chars().count();

    // If path is longer than available width on narrow terminals, show tail with ellipsis
    let (start_offset, show_ellipsis) = if path_chars.len() > available_path_width && available_path_width > 10 {
        (path_chars.len() - available_path_width + 3, true)
    } else {
        (0, false)
    };

    if show_ellipsis {
        spans.push(Span::styled("...", Style::default().fg(Color::DarkGray)));
    }

    // Build highlighted path spans
    let mut current_text = String::new();
    let mut current_is_match = false;
    let mut current_is_parent = true;

    for (offset, &ch) in path_chars[start_offset..].iter().enumerate() {
        let orig_idx = start_offset + offset;
        let is_match = matched_indices.contains(&orig_idx);
        let is_parent = orig_idx < parent_len;

        if is_match != current_is_match || is_parent != current_is_parent {
            if !current_text.is_empty() {
                spans.push(create_path_span(
                    std::mem::take(&mut current_text),
                    current_is_match,
                    current_is_parent,
                    is_selected,
                ));
            }
            current_is_match = is_match;
            current_is_parent = is_parent;
        }

        current_text.push(ch);
    }

    if !current_text.is_empty() {
        spans.push(create_path_span(
            current_text,
            current_is_match,
            current_is_parent,
            is_selected,
        ));
    }

    // Truncate spans if exceeding available width
    let current_width = spans_len(&spans);
    let padding = (width as usize).saturating_sub(current_width + size_width);
    if padding > 0 {
        spans.push(Span::raw(" ".repeat(padding)));
    }

    // 4. Right-aligned file size
    let size_style = if is_selected {
        Style::default().fg(Color::DarkGray)
    } else {
        Style::default().fg(Color::Rgb(100, 100, 100))
    };
    spans.push(Span::styled(format!(" {}", size_str), size_style));

    Line::from(spans)
}

fn create_path_span(
    text: String,
    is_match: bool,
    is_parent: bool,
    is_selected: bool,
) -> Span<'static> {
    let style = if is_match {
        // Highlighted matched characters (Bold Yellow/Cyan)
        Style::default()
            .fg(Color::Yellow)
            .add_modifier(Modifier::BOLD | Modifier::UNDERLINED)
    } else if is_parent {
        // Parent directory path (Dim Gray)
        if is_selected {
            Style::default().fg(Color::Rgb(140, 140, 140))
        } else {
            Style::default().fg(Color::DarkGray)
        }
    } else {
        // Filename portion (White / Bold White)
        if is_selected {
            Style::default()
                .fg(Color::White)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(Color::White)
        }
    };
    Span::styled(text, style)
}
fn spans_len(spans: &[Span]) -> usize {
    spans.iter().map(|s| s.content.chars().count()).sum()
}

// ---------------------------------------------------------------------------
// Convenience Helper Functions
// ---------------------------------------------------------------------------

/// Convenience function to launch the interactive fuzzy file picker in the given directory.
pub fn pick_file(root: Option<&Path>) -> std::io::Result<Option<FileEntry>> {
    let mut picker = match root {
        Some(r) => FilePicker::from_directory(r),
        None => FilePicker::new(),
    };
    picker.run_interactive(None)
}

// ---------------------------------------------------------------------------
// Unit Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_file_type_detection() {
        assert_eq!(FileType::from_extension("rs"), FileType::Rust);
        assert_eq!(FileType::from_extension("ts"), FileType::TypeScript);
        assert_eq!(FileType::from_extension("tsx"), FileType::TypeScript);
        assert_eq!(FileType::from_extension("js"), FileType::JavaScript);
        assert_eq!(FileType::from_extension("py"), FileType::Python);
        assert_eq!(FileType::from_extension("go"), FileType::Go);
        assert_eq!(FileType::from_extension("md"), FileType::Markdown);
        assert_eq!(FileType::from_extension("json"), FileType::Config);
        assert_eq!(FileType::from_extension("toml"), FileType::Config);
        assert_eq!(FileType::from_extension("sh"), FileType::Shell);
        assert_eq!(FileType::from_extension("sql"), FileType::Sql);
        assert_eq!(FileType::from_extension("unknown"), FileType::Other);
    }

    #[test]
    fn test_file_entry_formatting() {
        let entry = FileEntry::new(
            PathBuf::from("/workspace/src/ui/file_picker.rs"),
            Path::new("/workspace"),
            false,
            4096,
            false,
        );

        assert_eq!(entry.relative_path, "src/ui/file_picker.rs");
        assert_eq!(entry.file_name, "file_picker.rs");
        assert_eq!(entry.file_stem(), "file_picker");
        assert_eq!(entry.parent_dir(), "src/ui/");
        assert_eq!(entry.extension.as_deref(), Some("rs"));
        assert_eq!(entry.file_type, FileType::Rust);
        assert_eq!(entry.formatted_size(), "4.0 KB");
    }

    #[test]
    fn test_fuzzy_match_exact() {
        let res = fuzzy_match("main.rs", "src/main.rs").expect("Match should succeed");
        assert!(res.score > 200);
        assert!(!res.matched_indices.is_empty());
    }

    #[test]
    fn test_fuzzy_match_subsequence() {
        let res = fuzzy_match("fp", "src/ui/file_picker.rs").expect("Match should succeed");
        assert!(res.score > 0);
        assert_eq!(res.matched_indices.len(), 2);

        // Non-match
        assert!(fuzzy_match("xyz", "src/ui/file_picker.rs").is_none());
    }

    #[test]
    fn test_fuzzy_match_case_insensitivity() {
        let res1 = fuzzy_match("cargo", "Cargo.toml");
        let res2 = fuzzy_match("CARGO", "Cargo.toml");
        assert!(res1.is_some());
        assert!(res2.is_some());
    }

    #[test]
    fn test_fuzzy_search_ranking() {
        let files = vec![
            FileEntry::new(PathBuf::from("src/tools/file.rs"), Path::new("."), false, 100, false),
            FileEntry::new(PathBuf::from("src/ui/file_picker.rs"), Path::new("."), false, 200, false),
            FileEntry::new(PathBuf::from("README.md"), Path::new("."), false, 50, false),
        ];

        let results = fuzzy_search("file_picker", &files, 10);
        assert!(!results.is_empty());
        assert_eq!(results[0].0.file_name, "file_picker.rs");
    }

    #[test]
    fn test_file_picker_query_and_keys() {
        let files = vec![
            FileEntry::new(PathBuf::from("src/main.rs"), Path::new("."), false, 100, false),
            FileEntry::new(PathBuf::from("src/ui/mod.rs"), Path::new("."), false, 200, false),
            FileEntry::new(PathBuf::from("Cargo.toml"), Path::new("."), false, 300, false),
        ];

        let mut picker = FilePicker::with_files(files);
        assert_eq!(picker.matched_files_count(), 3);

        // Type 'main'
        picker.handle_key(KeyCode::Char('m'), KeyModifiers::NONE);
        picker.handle_key(KeyCode::Char('a'), KeyModifiers::NONE);
        picker.handle_key(KeyCode::Char('i'), KeyModifiers::NONE);
        picker.handle_key(KeyCode::Char('n'), KeyModifiers::NONE);

        assert_eq!(picker.query(), "main");
        assert_eq!(picker.matched_files_count(), 1);
        assert_eq!(picker.selected_entry().unwrap().file_name, "main.rs");

        // Press Enter to select
        let res = picker.handle_key(KeyCode::Enter, KeyModifiers::NONE);
        assert_eq!(
            res,
            Some(FilePickerResult::Selected(
                picker.selected_entry().unwrap().clone()
            ))
        );
    }

    #[test]
    fn test_file_picker_navigation_and_cancel() {
        let files = vec![
            FileEntry::new(PathBuf::from("a.rs"), Path::new("."), false, 10, false),
            FileEntry::new(PathBuf::from("b.rs"), Path::new("."), false, 20, false),
            FileEntry::new(PathBuf::from("c.rs"), Path::new("."), false, 30, false),
        ];

        let mut picker = FilePicker::with_files(files);
        assert_eq!(picker.selected_index, 0);

        // Down
        picker.handle_key(KeyCode::Down, KeyModifiers::NONE);
        assert_eq!(picker.selected_index, 1);

        // Ctrl+N
        picker.handle_key(KeyCode::Char('n'), KeyModifiers::CONTROL);
        assert_eq!(picker.selected_index, 2);

        // Up
        picker.handle_key(KeyCode::Up, KeyModifiers::NONE);
        assert_eq!(picker.selected_index, 1);

        // Esc clears or cancels
        let cancel = picker.handle_key(KeyCode::Esc, KeyModifiers::NONE);
        assert_eq!(cancel, Some(FilePickerResult::Cancelled));
    }

    #[test]
    fn test_render_buffer_smoke() {
        let files = vec![
            FileEntry::new(PathBuf::from("src/main.rs"), Path::new("."), false, 1024, false),
            FileEntry::new(PathBuf::from("src/ui/file_picker.rs"), Path::new("."), false, 2048, false),
        ];

        let picker = FilePicker::with_files(files);
        let mut buf = Buffer::empty(Rect::new(0, 0, 80, 10));
        picker.render_buffer(Rect::new(0, 0, 80, 10), &mut buf);
    }
}
