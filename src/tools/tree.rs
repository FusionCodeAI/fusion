//! Directory tree generator tool.
//!
//! Generates clean, human-readable directory trees in pure Rust with:
//! - Configurable depth limit (`max_depth` / `depth`)
//! - Configurable `.gitignore`, `.ignore`, and custom ignore patterns (`ignore_patterns` / `exclude`)
//! - Hidden files toggle (`show_hidden` / `hidden`)
//! - Rich size reporting with human-readable bytes (`show_size` / `sizes`)
//! - ASCII (`|-- `, `\-- `) and Unicode (`├── `, `└── `) branch styles
//! - Directories-only filtering (`dirs_only`)
//! - Flexible sorting (directories first, alphabetical, size, size ascending, modification time)
//! - Output formats: text tree and structured JSON
//! - Truncation safeguards for huge directory hierarchies
//! - Symlink detection, reporting, and loop prevention

use async_trait::async_trait;
use globset::{GlobBuilder, GlobMatcher};
use ignore::gitignore::{Gitignore, GitignoreBuilder};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use crate::tools::file::resolve_path;
use crate::tools::types::{Tool, ToolContext};

// ===========================================================================
// Tree Formatting & Charset
// ===========================================================================

/// Character set used for rendering tree branches.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum TreeCharset {
    /// Modern Unicode box-drawing characters (`├── `, `└── `, `│   `).
    #[default]
    Unicode,
    /// Standard ASCII characters (`|-- `, `\-- `, `|   `).
    Ascii,
}

impl TreeCharset {
    /// The connector branch for non-last child nodes.
    #[inline]
    pub fn branch(&self) -> &'static str {
        match self {
            TreeCharset::Unicode => "├── ",
            TreeCharset::Ascii => "|-- ",
        }
    }

    /// The connector branch for the last child node.
    #[inline]
    pub fn last_branch(&self) -> &'static str {
        match self {
            TreeCharset::Unicode => "└── ",
            TreeCharset::Ascii => "\\-- ",
        }
    }

    /// The vertical line prefix for child levels.
    #[inline]
    pub fn vertical(&self) -> &'static str {
        match self {
            TreeCharset::Unicode => "│   ",
            TreeCharset::Ascii => "|   ",
        }
    }

    /// The empty space prefix for child levels following a last branch.
    #[inline]
    pub fn empty(&self) -> &'static str {
        "    "
    }
}

// ===========================================================================
// Tree Sort Mode
// ===========================================================================

/// Sort order for directory entries.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum TreeSort {
    /// Directories first (alphabetical), then files (alphabetical).
    #[default]
    DirsFirst,
    /// Pure alphabetical sorting by name regardless of type.
    Name,
    /// Sort by size descending (largest first).
    Size,
    /// Sort by size ascending (smallest first).
    SizeAsc,
    /// Sort by last modification time descending (newest first).
    Modified,
}

impl TreeSort {
    pub fn from_str_loose(s: &str) -> Self {
        match s.trim().to_lowercase().as_str() {
            "name" | "alpha" | "alphabetical" => TreeSort::Name,
            "size" | "size_desc" | "sizedesc" | "largest" => TreeSort::Size,
            "size_asc" | "sizeasc" | "smallest" => TreeSort::SizeAsc,
            "modified" | "time" | "date" | "recent" => TreeSort::Modified,
            _ => TreeSort::DirsFirst,
        }
    }
}

// ===========================================================================
// Tree Format
// ===========================================================================

/// Output format for tree generator results.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum TreeFormat {
    /// Human-readable ASCII/Unicode formatted text.
    #[default]
    Text,
    /// Structured JSON hierarchy.
    Json,
}

// ===========================================================================
// Tree Configuration Options
// ===========================================================================

/// Full configuration options for generating a directory tree.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TreeOptions {
    /// Root directory to generate tree for.
    pub root: PathBuf,
    /// Maximum recursion depth (None = unlimited).
    pub max_depth: Option<usize>,
    /// Whether to include hidden files and directories (dotfiles).
    pub show_hidden: bool,
    /// Whether to respect `.gitignore` and `.ignore` files.
    pub respect_gitignore: bool,
    /// Additional custom glob ignore patterns.
    pub ignore_patterns: Vec<String>,
    /// Only include directories, omit regular files.
    pub dirs_only: bool,
    /// Report human-readable file and directory sizes.
    pub show_size: bool,
    /// Display charset style (Unicode or ASCII).
    pub charset: TreeCharset,
    /// Sorting mode for entries.
    pub sort: TreeSort,
    /// Maximum number of total displayed entries before truncating.
    pub max_entries: usize,
    /// Output representation format.
    pub format: TreeFormat,
    /// Whether to follow directory symlinks.
    pub follow_symlinks: bool,
    /// Optional glob filter pattern for filenames.
    pub pattern: Option<String>,
    /// Whether to display full paths or relative paths.
    pub show_full_path: bool,
}

impl Default for TreeOptions {
    fn default() -> Self {
        Self {
            root: PathBuf::from("."),
            max_depth: None,
            show_hidden: false,
            respect_gitignore: true,
            ignore_patterns: Vec::new(),
            dirs_only: false,
            show_size: true,
            charset: TreeCharset::Unicode,
            sort: TreeSort::DirsFirst,
            max_entries: 1000,
            format: TreeFormat::Text,
            follow_symlinks: false,
            pattern: None,
            show_full_path: false,
        }
    }
}

// ===========================================================================
// Tree Node & Stats
// ===========================================================================

/// A single node in the directory tree.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TreeNode {
    /// File or directory base name.
    pub name: String,
    /// Canonical or absolute path.
    pub path: PathBuf,
    /// Path relative to the root directory.
    pub rel_path: PathBuf,
    /// Whether this node is a directory.
    pub is_dir: bool,
    /// Whether this node is a symbolic link.
    pub is_symlink: bool,
    /// Symlink target path string if applicable.
    pub symlink_target: Option<String>,
    /// File size in bytes, or total aggregated directory size.
    pub size: u64,
    /// Human-readable formatted size string (e.g. `4.2 KB`).
    pub size_human: String,
    /// Last modified timestamp if available.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modified: Option<u64>,
    /// Sub-nodes if this node is a directory.
    pub children: Vec<TreeNode>,
    /// Node depth relative to root (root = 0).
    pub depth: usize,
    /// Error message if directory reading failed.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// Aggregated statistics from tree traversal.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TreeStats {
    /// Total number of directories traversed (excluding root).
    pub directories: usize,
    /// Total number of files traversed.
    pub files: usize,
    /// Total combined size in bytes.
    pub total_size: u64,
    /// Total combined size formatted human-readably.
    pub total_size_human: String,
    /// Whether output was truncated due to `max_entries` limit.
    pub truncated: bool,
    /// Total entries encountered before truncation.
    pub total_entries: usize,
}

/// Complete tree generator result.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TreeResult {
    /// Root directory display name.
    pub root_name: String,
    /// Absolute root path.
    pub root_path: PathBuf,
    /// Hierarchical root node.
    pub tree: TreeNode,
    /// Summary statistics.
    pub stats: TreeStats,
    /// Formatted text tree rendering.
    pub formatted_text: String,
}

// ===========================================================================
// Human-Readable Size Formatter
// ===========================================================================

/// Formats a byte count into a clean, human-readable string (e.g. `120 B`, `4.2 KB`, `1.5 MB`).
pub fn format_size(bytes: u64) -> String {
    const KB: u64 = 1024;
    const MB: u64 = KB * 1024;
    const GB: u64 = MB * 1024;
    const TB: u64 = GB * 1024;

    if bytes < KB {
        format!("{bytes} B")
    } else if bytes < MB {
        let kb = bytes as f64 / KB as f64;
        if kb < 10.0 {
            format!("{kb:.1} KB")
        } else {
            format!("{kb:.0} KB")
        }
    } else if bytes < GB {
        let mb = bytes as f64 / MB as f64;
        if mb < 10.0 {
            format!("{mb:.1} MB")
        } else {
            format!("{mb:.0} MB")
        }
    } else if bytes < TB {
        let gb = bytes as f64 / GB as f64;
        if gb < 10.0 {
            format!("{gb:.2} GB")
        } else {
            format!("{gb:.1} GB")
        }
    } else {
        let tb = bytes as f64 / TB as f64;
        format!("{tb:.2} TB")
    }
}

/// Formats size with fixed width bracket padding for clean tree alignment: `[ 4.2 KB]`.
pub fn format_size_aligned(bytes: u64) -> String {
    let s = format_size(bytes);
    format!("[{s:>8}]")
}

// ===========================================================================
// Tree Scanner Implementation
// ===========================================================================

/// Internal directory scanner and tree builder.
pub struct TreeScanner<'a> {
    options: &'a TreeOptions,
    custom_matchers: Vec<GlobMatcher>,
    pattern_matcher: Option<GlobMatcher>,
    visited_symlinks: HashSet<PathBuf>,
    entry_counter: usize,
    truncated: bool,
    dir_count: usize,
    file_count: usize,
    total_size: u64,
}

impl<'a> TreeScanner<'a> {
    pub fn new(options: &'a TreeOptions) -> anyhow::Result<Self> {
        let mut custom_matchers = Vec::new();
        for pat in &options.ignore_patterns {
            let glob = GlobBuilder::new(pat)
                .case_insensitive(true)
                .literal_separator(false)
                .build()
                .map_err(|e| anyhow::anyhow!("Invalid ignore glob pattern '{pat}': {e}"))?;
            custom_matchers.push(glob.compile_matcher());
        }

        let pattern_matcher = if let Some(p) = &options.pattern {
            let glob = GlobBuilder::new(p)
                .case_insensitive(true)
                .literal_separator(false)
                .build()
                .map_err(|e| anyhow::anyhow!("Invalid filter pattern '{p}': {e}"))?;
            Some(glob.compile_matcher())
        } else {
            None
        };

        Ok(Self {
            options,
            custom_matchers,
            pattern_matcher,
            visited_symlinks: HashSet::new(),
            entry_counter: 0,
            truncated: false,
            dir_count: 0,
            file_count: 0,
            total_size: 0,
        })
    }

    /// Builds the root Gitignore matcher starting at the root path.
    fn build_root_gitignore(&self, root: &Path) -> Option<Gitignore> {
        if !self.options.respect_gitignore {
            return None;
        }

        let mut builder = GitignoreBuilder::new(root);

        // Add git info exclude if present
        let git_exclude = root.join(".git").join("info").join("exclude");
        if git_exclude.is_file() {
            builder.add(&git_exclude);
        }

        // Add root .gitignore if present
        let root_gitignore = root.join(".gitignore");
        if root_gitignore.is_file() {
            builder.add(&root_gitignore);
        }

        // Add root .ignore if present
        let root_ignore = root.join(".ignore");
        if root_ignore.is_file() {
            builder.add(&root_ignore);
        }

        builder.build().ok()
    }

    /// Checks if a file/dir name or path should be ignored.
    fn is_ignored(
        &self,
        path: &Path,
        rel_path: &Path,
        file_name: &str,
        is_dir: bool,
        gitignore_stack: &[Gitignore],
    ) -> bool {
        // 1. Hidden file check (dotfiles)
        if !self.options.show_hidden
            && file_name.starts_with('.')
            && file_name != "."
            && file_name != ".."
        {
            return true;
        }

        // Always ignore .git directory internals unless show_hidden is true
        if !self.options.show_hidden
            && (file_name == ".git" || file_name == ".hg" || file_name == ".svn")
        {
            return true;
        }

        // 2. Custom ignore glob patterns
        let path_str = path.to_string_lossy().replace('\\', "/");
        let rel_str = rel_path.to_string_lossy().replace('\\', "/");
        for matcher in &self.custom_matchers {
            if matcher.is_match(file_name)
                || matcher.is_match(&path_str)
                || matcher.is_match(&rel_str)
            {
                return true;
            }
        }

        // 3. Gitignore check across hierarchical active stack (innermost to outermost)
        for gi in gitignore_stack.iter().rev() {
            let m = gi.matched_path_or_any_parents(path, is_dir);
            if m.is_whitelist() {
                return false;
            }
            if m.is_ignore() {
                return true;
            }
        }

        false
    }

    /// Recursively scans directory contents and builds the `TreeNode` hierarchy.
    pub fn scan_dir(
        &mut self,
        current_path: &Path,
        rel_path: &Path,
        depth: usize,
        gitignore_stack: &mut Vec<Gitignore>,
    ) -> TreeNode {
        let name = current_path
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| current_path.to_string_lossy().to_string());

        let metadata = fs::symlink_metadata(current_path).ok();
        let is_symlink = metadata
            .as_ref()
            .map(|m| m.file_type().is_symlink())
            .unwrap_or(false);
        let mut symlink_target = None;

        if is_symlink {
            if let Ok(target) = fs::read_link(current_path) {
                symlink_target = Some(target.to_string_lossy().to_string());
            }
        }

        let is_dir = if is_symlink && !self.options.follow_symlinks {
            false
        } else if let Ok(meta) = fs::metadata(current_path) {
            meta.is_dir()
        } else {
            metadata.as_ref().map(|m| m.is_dir()).unwrap_or(false)
        };

        let modified = metadata
            .as_ref()
            .and_then(|m| m.modified().ok())
            .and_then(|t| {
                t.duration_since(SystemTime::UNIX_EPOCH)
                    .ok()
                    .map(|d| d.as_secs())
            });

        // If it's a file or non-followed symlink
        if !is_dir {
            let file_size = metadata.map(|m| m.len()).unwrap_or(0);
            self.file_count += 1;
            self.total_size += file_size;
            self.entry_counter += 1;

            return TreeNode {
                name,
                path: current_path.to_path_buf(),
                rel_path: rel_path.to_path_buf(),
                is_dir: false,
                is_symlink,
                symlink_target,
                size: file_size,
                size_human: format_size(file_size),
                modified,
                children: Vec::new(),
                depth,
                error: None,
            };
        }

        // It is a directory
        self.dir_count += 1;
        self.entry_counter += 1;

        // Check symlink loop detection
        if is_symlink && self.options.follow_symlinks {
            if let Ok(canonical) = fs::canonicalize(current_path) {
                if !self.visited_symlinks.insert(canonical) {
                    return TreeNode {
                        name,
                        path: current_path.to_path_buf(),
                        rel_path: rel_path.to_path_buf(),
                        is_dir: true,
                        is_symlink: true,
                        symlink_target,
                        size: 0,
                        size_human: format_size(0),
                        modified,
                        children: Vec::new(),
                        depth,
                        error: Some("Recursive symlink detected".to_string()),
                    };
                }
            }
        }

        // Push local .gitignore / .ignore if present at this directory depth
        let mut pushed_local_gi = false;
        if self.options.respect_gitignore && depth > 0 {
            let local_gi_file = current_path.join(".gitignore");
            let local_ignore_file = current_path.join(".ignore");

            if local_gi_file.is_file() || local_ignore_file.is_file() {
                let mut builder = GitignoreBuilder::new(current_path);
                if local_gi_file.is_file() {
                    builder.add(&local_gi_file);
                }
                if local_ignore_file.is_file() {
                    builder.add(&local_ignore_file);
                }
                if let Ok(gi) = builder.build() {
                    gitignore_stack.push(gi);
                    pushed_local_gi = true;
                }
            }
        }

        // Check if max_depth reached
        if let Some(max_depth) = self.options.max_depth {
            if depth >= max_depth {
                if pushed_local_gi {
                    gitignore_stack.pop();
                }
                return TreeNode {
                    name,
                    path: current_path.to_path_buf(),
                    rel_path: rel_path.to_path_buf(),
                    is_dir: true,
                    is_symlink,
                    symlink_target,
                    size: 0,
                    size_human: format_size(0),
                    modified,
                    children: Vec::new(),
                    depth,
                    error: None,
                };
            }
        }

        // Read directory entries
        let read_dir_result = fs::read_dir(current_path);
        let entries = match read_dir_result {
            Ok(rd) => rd,
            Err(e) => {
                if pushed_local_gi {
                    gitignore_stack.pop();
                }
                return TreeNode {
                    name,
                    path: current_path.to_path_buf(),
                    rel_path: rel_path.to_path_buf(),
                    is_dir: true,
                    is_symlink,
                    symlink_target,
                    size: 0,
                    size_human: format_size(0),
                    modified,
                    children: Vec::new(),
                    depth,
                    error: Some(format!("Permission denied / Error: {e}")),
                };
            }
        };

        let mut child_nodes = Vec::new();
        let mut raw_entries = Vec::new();

        for entry_res in entries {
            if self.entry_counter >= self.options.max_entries {
                self.truncated = true;
                break;
            }

            let entry = match entry_res {
                Ok(e) => e,
                Err(_) => continue,
            };

            let entry_path = entry.path();
            let entry_name = entry.file_name().to_string_lossy().to_string();

            let file_type = entry.file_type().ok();
            let is_child_dir = file_type.map(|ft| ft.is_dir()).unwrap_or(false);

            let child_rel_path = if rel_path == Path::new(".") {
                PathBuf::from(&entry_name)
            } else {
                rel_path.join(&entry_name)
            };

            // Check ignore rules
            if self.is_ignored(
                &entry_path,
                &child_rel_path,
                &entry_name,
                is_child_dir,
                gitignore_stack,
            ) {
                continue;
            }

            // Check dirs_only filter
            if self.options.dirs_only && !is_child_dir {
                continue;
            }

            // Check pattern filter on files
            if !is_child_dir {
                if let Some(matcher) = &self.pattern_matcher {
                    let entry_path_str = entry_path.to_string_lossy().replace('\\', "/");
                    let entry_rel_str = child_rel_path.to_string_lossy().replace('\\', "/");
                    if !matcher.is_match(&entry_name)
                        && !matcher.is_match(&entry_path_str)
                        && !matcher.is_match(&entry_rel_str)
                    {
                        continue;
                    }
                }
            }

            raw_entries.push((entry_path, child_rel_path, entry_name, is_child_dir));
        }

        // Sort child entries
        self.sort_raw_entries(&mut raw_entries);

        let mut dir_total_size = 0u64;

        for (child_path, child_rel, _child_name, _is_child_dir) in raw_entries {
            if self.entry_counter >= self.options.max_entries {
                self.truncated = true;
                break;
            }

            let child_node = self.scan_dir(&child_path, &child_rel, depth + 1, gitignore_stack);

            dir_total_size += child_node.size;
            child_nodes.push(child_node);
        }

        // Post-sort child nodes to accurately sort by aggregate directory size if needed
        self.sort_child_nodes(&mut child_nodes);

        if pushed_local_gi {
            gitignore_stack.pop();
        }

        TreeNode {
            name,
            path: current_path.to_path_buf(),
            rel_path: rel_path.to_path_buf(),
            is_dir: true,
            is_symlink,
            symlink_target,
            size: dir_total_size,
            size_human: format_size(dir_total_size),
            modified,
            children: child_nodes,
            depth,
            error: None,
        }
    }

    /// Sorts raw entries before recursive traversal.
    fn sort_raw_entries(&self, entries: &mut [(PathBuf, PathBuf, String, bool)]) {
        match self.options.sort {
            TreeSort::DirsFirst => {
                entries.sort_by(|a, b| {
                    b.3.cmp(&a.3)
                        .then_with(|| a.2.to_lowercase().cmp(&b.2.to_lowercase()))
                        .then_with(|| a.2.cmp(&b.2))
                });
            }
            TreeSort::Name => {
                entries.sort_by(|a, b| {
                    a.2.to_lowercase()
                        .cmp(&b.2.to_lowercase())
                        .then_with(|| a.2.cmp(&b.2))
                });
            }
            TreeSort::Size => {
                entries.sort_by(|a, b| {
                    let size_a = fs::metadata(&a.0).map(|m| m.len()).unwrap_or(0);
                    let size_b = fs::metadata(&b.0).map(|m| m.len()).unwrap_or(0);
                    size_b
                        .cmp(&size_a)
                        .then_with(|| a.2.to_lowercase().cmp(&b.2.to_lowercase()))
                        .then_with(|| a.2.cmp(&b.2))
                });
            }
            TreeSort::SizeAsc => {
                entries.sort_by(|a, b| {
                    let size_a = fs::metadata(&a.0).map(|m| m.len()).unwrap_or(0);
                    let size_b = fs::metadata(&b.0).map(|m| m.len()).unwrap_or(0);
                    size_a
                        .cmp(&size_b)
                        .then_with(|| a.2.to_lowercase().cmp(&b.2.to_lowercase()))
                        .then_with(|| a.2.cmp(&b.2))
                });
            }
            TreeSort::Modified => {
                entries.sort_by(|a, b| {
                    let time_a = fs::metadata(&a.0)
                        .and_then(|m| m.modified())
                        .unwrap_or(SystemTime::UNIX_EPOCH);
                    let time_b = fs::metadata(&b.0)
                        .and_then(|m| m.modified())
                        .unwrap_or(SystemTime::UNIX_EPOCH);
                    time_b
                        .cmp(&time_a)
                        .then_with(|| a.2.to_lowercase().cmp(&b.2.to_lowercase()))
                        .then_with(|| a.2.cmp(&b.2))
                });
            }
        }
    }

    /// Sorts scanned child nodes with computed aggregate sizes.
    fn sort_child_nodes(&self, nodes: &mut [TreeNode]) {
        match self.options.sort {
            TreeSort::DirsFirst => {
                nodes.sort_by(|a, b| {
                    b.is_dir
                        .cmp(&a.is_dir)
                        .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
                        .then_with(|| a.name.cmp(&b.name))
                });
            }
            TreeSort::Name => {
                nodes.sort_by(|a, b| {
                    a.name
                        .to_lowercase()
                        .cmp(&b.name.to_lowercase())
                        .then_with(|| a.name.cmp(&b.name))
                });
            }
            TreeSort::Size => {
                nodes.sort_by(|a, b| {
                    b.size
                        .cmp(&a.size)
                        .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
                        .then_with(|| a.name.cmp(&b.name))
                });
            }
            TreeSort::SizeAsc => {
                nodes.sort_by(|a, b| {
                    a.size
                        .cmp(&b.size)
                        .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
                        .then_with(|| a.name.cmp(&b.name))
                });
            }
            TreeSort::Modified => {
                nodes.sort_by(|a, b| {
                    let time_a = a.modified.unwrap_or(0);
                    let time_b = b.modified.unwrap_or(0);
                    time_b
                        .cmp(&time_a)
                        .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
                        .then_with(|| a.name.cmp(&b.name))
                });
            }
        }
    }

    /// Runs the complete scan and produces `TreeResult`.
    pub fn execute_scan(mut self) -> TreeResult {
        let root = &self.options.root;
        let root_name = root
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| root.to_string_lossy().to_string());

        let mut gitignore_stack = Vec::new();
        if let Some(root_gi) = self.build_root_gitignore(root) {
            gitignore_stack.push(root_gi);
        }

        let root_node = self.scan_dir(root, Path::new("."), 0, &mut gitignore_stack);

        let final_dir_count = if self.dir_count > 0 && root_node.is_dir {
            self.dir_count - 1
        } else {
            0
        };

        let stats = TreeStats {
            directories: final_dir_count,
            files: self.file_count,
            total_size: self.total_size,
            total_size_human: format_size(self.total_size),
            truncated: self.truncated,
            total_entries: self.entry_counter,
        };

        let formatted_text = render_tree_text(&root_node, self.options, &stats);

        TreeResult {
            root_name,
            root_path: root.clone(),
            tree: root_node,
            stats,
            formatted_text,
        }
    }
}

// ===========================================================================
// Tree Text Renderer
// ===========================================================================

/// Renders a `TreeNode` hierarchy into a formatted text tree.
pub fn render_tree_text(root: &TreeNode, options: &TreeOptions, stats: &TreeStats) -> String {
    let mut out = String::new();

    // Root directory header
    let root_display = if options.show_full_path {
        root.path.to_string_lossy().to_string()
    } else {
        root.name.clone()
    };

    if root.is_dir {
        if options.show_size {
            out.push_str(&format!("{root_display}/  [{}]\n", root.size_human));
        } else {
            out.push_str(&format!("{root_display}/\n"));
        }
    } else if options.show_size {
        out.push_str(&format!("{root_display}  [{}]\n", root.size_human));
    } else {
        out.push_str(&format!("{root_display}\n"));
    }

    // Render children recursively
    let mut prefix_stack = Vec::new();
    render_children(&root.children, options, &mut prefix_stack, &mut out);

    // Summary footer
    out.push('\n');
    let dir_label = if stats.directories == 1 {
        "directory"
    } else {
        "directories"
    };
    let file_label = if stats.files == 1 { "file" } else { "files" };

    if options.dirs_only {
        out.push_str(&format!("{} {}", stats.directories, dir_label));
    } else {
        out.push_str(&format!(
            "{} {}, {} {}",
            stats.directories, dir_label, stats.files, file_label
        ));
    }

    if options.show_size {
        out.push_str(&format!(" (total: {})", stats.total_size_human));
    }

    if stats.truncated {
        out.push_str(&format!(" [truncated at {} entries]", options.max_entries));
    }

    out.push('\n');
    out
}

/// Helper function to recursively render child nodes.
fn render_children(
    children: &[TreeNode],
    options: &TreeOptions,
    prefix_stack: &mut Vec<&'static str>,
    out: &mut String,
) {
    let count = children.len();
    for (i, child) in children.iter().enumerate() {
        let is_last = i == count - 1;

        // Add accumulated prefix stack
        for prefix in prefix_stack.iter() {
            out.push_str(prefix);
        }

        // Add current node's branch connector
        if is_last {
            out.push_str(options.charset.last_branch());
        } else {
            out.push_str(options.charset.branch());
        }

        // Size badge (if enabled)
        if options.show_size {
            out.push_str(&format_size_aligned(child.size));
            out.push(' ');
        }

        // Name and suffix
        let display_name = if options.show_full_path {
            child.rel_path.to_string_lossy().to_string()
        } else {
            child.name.clone()
        };

        if child.is_dir {
            out.push_str(&display_name);
            out.push('/');
        } else {
            out.push_str(&display_name);
        }

        // Symlink indicator
        if child.is_symlink {
            if let Some(target) = &child.symlink_target {
                out.push_str(&format!(" -> {target}"));
            }
        }

        // Error indicator
        if let Some(err) = &child.error {
            out.push_str(&format!(" [{err}]"));
        }

        out.push('\n');

        // Recurse into directory children
        if child.is_dir && !child.children.is_empty() {
            let next_prefix = if is_last {
                options.charset.empty()
            } else {
                options.charset.vertical()
            };

            prefix_stack.push(next_prefix);
            render_children(&child.children, options, prefix_stack, out);
            prefix_stack.pop();
        }
    }
}

// ===========================================================================
// Public Generator Entry Point
// ===========================================================================

/// Generates a directory tree with the given options.
pub fn generate_tree(options: TreeOptions) -> anyhow::Result<TreeResult> {
    if !options.root.exists() {
        anyhow::bail!("Path not found: '{}'", options.root.display());
    }

    let scanner = TreeScanner::new(&options)?;
    Ok(scanner.execute_scan())
}

// ===========================================================================
// TreeTool Implementation
// ===========================================================================

/// Pure-Rust directory tree generator tool with configurable depth, ignore rules, and size reporting.
#[derive(Default, Debug, Clone)]
pub struct TreeTool;

impl TreeTool {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl Tool for TreeTool {
    fn name(&self) -> &str {
        "tree"
    }

    fn description(&self) -> &str {
        "Generate a directory tree with configurable depth, ignore rules, size reporting, and ASCII/Unicode styles."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "Root directory path to generate the tree from (optional, defaults to workspace root)."
                },
                "max_depth": {
                    "type": "integer",
                    "description": "Maximum directory depth to traverse (optional, e.g. 1 for direct children, 2 for two levels)."
                },
                "depth": {
                    "type": "integer",
                    "description": "Alias for max_depth."
                },
                "show_hidden": {
                    "type": "boolean",
                    "description": "Whether to include hidden files and directories (dotfiles). Default: false."
                },
                "respect_gitignore": {
                    "type": "boolean",
                    "description": "Whether to respect .gitignore and .ignore rules. Default: true."
                },
                "ignore_patterns": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "Additional glob patterns to ignore (e.g. ['target', 'node_modules', '*.lock'])."
                },
                "dirs_only": {
                    "type": "boolean",
                    "description": "Only show directories, omit regular files. Default: false."
                },
                "show_size": {
                    "type": "boolean",
                    "description": "Show formatted human-readable file and directory sizes. Default: true."
                },
                "charset": {
                    "type": "string",
                    "enum": ["unicode", "ascii"],
                    "description": "Character set for tree branches: 'unicode' (├──, └──) or 'ascii' (|--, \\--). Default: 'unicode'."
                },
                "ascii": {
                    "type": "boolean",
                    "description": "Shorthand for charset: 'ascii'. If true, uses ASCII tree characters."
                },
                "sort": {
                    "type": "string",
                    "enum": ["dirs_first", "name", "size", "size_asc", "modified"],
                    "description": "Sort order for tree entries. Default: 'dirs_first'."
                },
                "max_entries": {
                    "type": "integer",
                    "description": "Maximum number of total entries to display before truncating. Default: 1000."
                },
                "format": {
                    "type": "string",
                    "enum": ["text", "json"],
                    "description": "Output format: 'text' (formatted tree string) or 'json' (structured JSON tree). Default: 'text'."
                },
                "follow_symlinks": {
                    "type": "boolean",
                    "description": "Whether to follow directory symlinks. Default: false."
                },
                "pattern": {
                    "type": "string",
                    "description": "Optional glob pattern to filter files (e.g. '*.rs', '*.{ts,js}')."
                },
                "show_full_path": {
                    "type": "boolean",
                    "description": "Whether to display full paths instead of relative names. Default: false."
                }
            }
        })
    }

    async fn execute(&self, args: Value, ctx: &ToolContext) -> anyhow::Result<String> {
        let mut options = TreeOptions::default();

        // 1. Path resolution
        let path_str = args
            .get("path")
            .and_then(|v| v.as_str())
            .or_else(|| args.get("dir").and_then(|v| v.as_str()))
            .or_else(|| args.get("root").and_then(|v| v.as_str()));

        options.root = match path_str {
            Some(p) => resolve_path(p, &ctx.cwd),
            None => ctx.cwd.clone(),
        };

        if !options.root.exists() {
            anyhow::bail!("Path not found: '{}'", options.root.display());
        }

        // 2. Max depth
        let max_depth = args
            .get("max_depth")
            .and_then(|v| v.as_u64())
            .or_else(|| args.get("depth").and_then(|v| v.as_u64()))
            .or_else(|| args.get("level").and_then(|v| v.as_u64()))
            .or_else(|| args.get("maxDepth").and_then(|v| v.as_u64()))
            .map(|d| d as usize);

        options.max_depth = max_depth;

        // 3. Hidden files
        if let Some(hidden) = args
            .get("show_hidden")
            .and_then(|v| v.as_bool())
            .or_else(|| args.get("hidden").and_then(|v| v.as_bool()))
            .or_else(|| args.get("all").and_then(|v| v.as_bool()))
            .or_else(|| args.get("showHidden").and_then(|v| v.as_bool()))
        {
            options.show_hidden = hidden;
        }

        // 4. Respect gitignore
        if let Some(gi) = args
            .get("respect_gitignore")
            .and_then(|v| v.as_bool())
            .or_else(|| args.get("gitignore").and_then(|v| v.as_bool()))
            .or_else(|| args.get("respectGitignore").and_then(|v| v.as_bool()))
        {
            options.respect_gitignore = gi;
        }

        // 5. Ignore patterns
        if let Some(patterns) = args
            .get("ignore_patterns")
            .and_then(|v| v.as_array())
            .or_else(|| args.get("ignore").and_then(|v| v.as_array()))
            .or_else(|| args.get("exclude").and_then(|v| v.as_array()))
            .or_else(|| args.get("ignorePatterns").and_then(|v| v.as_array()))
        {
            for pat in patterns {
                if let Some(s) = pat.as_str() {
                    options.ignore_patterns.push(s.to_string());
                }
            }
        } else if let Some(single_pat) = args
            .get("ignore")
            .and_then(|v| v.as_str())
            .or_else(|| args.get("exclude").and_then(|v| v.as_str()))
        {
            options.ignore_patterns.push(single_pat.to_string());
        }

        // 6. Dirs only
        if let Some(dirs_only) = args
            .get("dirs_only")
            .and_then(|v| v.as_bool())
            .or_else(|| args.get("directories_only").and_then(|v| v.as_bool()))
            .or_else(|| args.get("dirsOnly").and_then(|v| v.as_bool()))
            .or_else(|| args.get("only_dirs").and_then(|v| v.as_bool()))
        {
            options.dirs_only = dirs_only;
        }

        // 7. Show size
        if let Some(show_size) = args
            .get("show_size")
            .and_then(|v| v.as_bool())
            .or_else(|| args.get("sizes").and_then(|v| v.as_bool()))
            .or_else(|| args.get("size").and_then(|v| v.as_bool()))
            .or_else(|| args.get("showSize").and_then(|v| v.as_bool()))
            .or_else(|| args.get("report_size").and_then(|v| v.as_bool()))
        {
            options.show_size = show_size;
        }

        // 8. Charset (Unicode vs ASCII)
        let ascii_flag = args
            .get("ascii")
            .and_then(|v| v.as_bool())
            .or_else(|| args.get("ascii_only").and_then(|v| v.as_bool()))
            .or_else(|| args.get("asciiOnly").and_then(|v| v.as_bool()))
            .unwrap_or(false);

        if ascii_flag {
            options.charset = TreeCharset::Ascii;
        } else if let Some(charset_str) = args
            .get("charset")
            .and_then(|v| v.as_str())
            .or_else(|| args.get("style").and_then(|v| v.as_str()))
        {
            if charset_str.eq_ignore_ascii_case("ascii") {
                options.charset = TreeCharset::Ascii;
            } else {
                options.charset = TreeCharset::Unicode;
            }
        }

        // 9. Sort mode
        if let Some(sort_str) = args
            .get("sort")
            .and_then(|v| v.as_str())
            .or_else(|| args.get("sort_by").and_then(|v| v.as_str()))
            .or_else(|| args.get("sortBy").and_then(|v| v.as_str()))
        {
            options.sort = TreeSort::from_str_loose(sort_str);
        }

        // 10. Max entries
        if let Some(limit) = args
            .get("max_entries")
            .and_then(|v| v.as_u64())
            .or_else(|| args.get("limit").and_then(|v| v.as_u64()))
            .or_else(|| args.get("maxEntries").and_then(|v| v.as_u64()))
            .or_else(|| args.get("max_files").and_then(|v| v.as_u64()))
        {
            options.max_entries = limit as usize;
        }

        // 11. Format (Text vs JSON)
        if let Some(fmt_str) = args
            .get("format")
            .and_then(|v| v.as_str())
            .or_else(|| args.get("output_format").and_then(|v| v.as_str()))
            .or_else(|| args.get("outputFormat").and_then(|v| v.as_str()))
        {
            if fmt_str.eq_ignore_ascii_case("json") {
                options.format = TreeFormat::Json;
            } else {
                options.format = TreeFormat::Text;
            }
        }

        // 12. Follow symlinks
        if let Some(follow) = args
            .get("follow_symlinks")
            .and_then(|v| v.as_bool())
            .or_else(|| args.get("follow").and_then(|v| v.as_bool()))
            .or_else(|| args.get("followSymlinks").and_then(|v| v.as_bool()))
        {
            options.follow_symlinks = follow;
        }

        // 13. File pattern
        if let Some(pat) = args
            .get("pattern")
            .and_then(|v| v.as_str())
            .or_else(|| args.get("filter").and_then(|v| v.as_str()))
            .or_else(|| args.get("match").and_then(|v| v.as_str()))
        {
            options.pattern = Some(pat.to_string());
        }

        // 14. Show full path
        if let Some(full_path) = args
            .get("show_full_path")
            .and_then(|v| v.as_bool())
            .or_else(|| args.get("full_path").and_then(|v| v.as_bool()))
            .or_else(|| args.get("fullPath").and_then(|v| v.as_bool()))
        {
            options.show_full_path = full_path;
        }

        let is_json = options.format == TreeFormat::Json;

        // Execute blocking traversal in tokio spawn_blocking
        let result = tokio::task::spawn_blocking(move || generate_tree(options))
            .await
            .map_err(|e| anyhow::anyhow!("Tree generation task failed: {e}"))??;

        if is_json {
            serde_json::to_string_pretty(&result)
                .map_err(|e| anyhow::anyhow!("Failed to serialize tree JSON: {e}"))
        } else {
            Ok(result.formatted_text)
        }
    }
}

// ===========================================================================
// Unit Tests
// ===========================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs::{self, File};
    use std::io::Write;
    use tempfile::tempdir;

    fn create_test_directory() -> tempfile::TempDir {
        let dir = tempdir().expect("Failed to create tempdir");
        let root = dir.path();

        // Create subdirectories
        fs::create_dir_all(root.join("src").join("utils")).unwrap();
        fs::create_dir_all(root.join("tests")).unwrap();
        fs::create_dir_all(root.join(".hidden_dir")).unwrap();
        fs::create_dir_all(root.join("target").join("debug")).unwrap();

        // Create files
        let mut f1 = File::create(root.join("Cargo.toml")).unwrap();
        f1.write_all(b"[package]\nname = \"test\"\n").unwrap();

        let mut f2 = File::create(root.join("README.md")).unwrap();
        f2.write_all(b"# Test Project\n").unwrap();

        let mut f3 = File::create(root.join("src").join("main.rs")).unwrap();
        f3.write_all(b"fn main() {}\n").unwrap();

        let mut f4 = File::create(root.join("src").join("utils").join("helper.rs")).unwrap();
        f4.write_all(b"pub fn help() {}\n").unwrap();

        let mut f5 = File::create(root.join("tests").join("test_basic.rs")).unwrap();
        f5.write_all(b"#[test] fn t() {}\n").unwrap();

        let mut f6 = File::create(root.join(".hidden_file")).unwrap();
        f6.write_all(b"secret\n").unwrap();

        let mut f7 = File::create(root.join("target").join("debug").join("app")).unwrap();
        f7.write_all(b"binary data").unwrap();

        // Create .gitignore
        let mut gi = File::create(root.join(".gitignore")).unwrap();
        gi.write_all(b"target/\n*.tmp\n").unwrap();

        dir
    }

    #[test]
    fn test_format_size() {
        assert_eq!(format_size(0), "0 B");
        assert_eq!(format_size(500), "500 B");
        assert_eq!(format_size(1024), "1.0 KB");
        assert_eq!(format_size(4500), "4.4 KB");
        assert_eq!(format_size(1024 * 1024), "1.0 MB");
        assert_eq!(format_size(1024 * 1024 * 1024), "1.00 GB");
    }

    #[test]
    fn test_format_size_aligned() {
        assert_eq!(format_size_aligned(500), "[   500 B]");
        assert_eq!(format_size_aligned(1024), "[  1.0 KB]");
    }

    #[test]
    fn test_tree_unicode_generation() {
        let dir = create_test_directory();
        let options = TreeOptions {
            root: dir.path().to_path_buf(),
            show_size: true,
            charset: TreeCharset::Unicode,
            ..Default::default()
        };

        let result = generate_tree(options).expect("Failed to generate tree");
        let text = result.formatted_text;

        // Verify unicode branches
        assert!(text.contains("├── ") || text.contains("└── "));
        assert!(text.contains("src/"));
        assert!(text.contains("Cargo.toml"));
        assert!(text.contains("README.md"));
        assert!(text.contains("main.rs"));
        assert!(text.contains("helper.rs"));

        // Verify target/ is ignored due to .gitignore
        assert!(!text.contains("target/"));
        assert!(!text.contains("binary data"));

        // Verify hidden files are not included by default
        assert!(!text.contains(".hidden_file"));
        assert!(!text.contains(".hidden_dir"));

        // Verify footer summary
        assert!(text.contains("directories") || text.contains("directory"));
        assert!(text.contains("files") || text.contains("file"));
        assert!(text.contains("total:"));
    }

    #[test]
    fn test_tree_ascii_generation() {
        let dir = create_test_directory();
        let options = TreeOptions {
            root: dir.path().to_path_buf(),
            show_size: true,
            charset: TreeCharset::Ascii,
            ..Default::default()
        };

        let result = generate_tree(options).expect("Failed to generate tree");
        let text = result.formatted_text;

        // Verify ASCII branch characters
        assert!(text.contains("|-- ") || text.contains("\\-- "));
        // Verify no Unicode box-drawing characters in ASCII mode
        assert!(!text.contains("├── "));
        assert!(!text.contains("└── "));
        assert!(!text.contains("│   "));

        assert!(text.contains("src/"));
        assert!(text.contains("Cargo.toml"));
    }

    #[test]
    fn test_tree_max_depth() {
        let dir = create_test_directory();
        let options = TreeOptions {
            root: dir.path().to_path_buf(),
            max_depth: Some(1),
            ..Default::default()
        };

        let result = generate_tree(options).expect("Failed to generate tree");
        let text = result.formatted_text;

        // Depth 1: direct children are shown, but deeper contents of src/ are not
        assert!(text.contains("src/"));
        assert!(text.contains("Cargo.toml"));
        assert!(text.contains("README.md"));
        assert!(!text.contains("main.rs"));
        assert!(!text.contains("helper.rs"));
    }

    #[test]
    fn test_tree_hidden_files() {
        let dir = create_test_directory();
        let options = TreeOptions {
            root: dir.path().to_path_buf(),
            show_hidden: true,
            ..Default::default()
        };

        let result = generate_tree(options).expect("Failed to generate tree");
        let text = result.formatted_text;

        // Hidden files should now be visible
        assert!(text.contains(".hidden_file"));
        assert!(text.contains(".hidden_dir"));
    }

    #[test]
    fn test_tree_dirs_only() {
        let dir = create_test_directory();
        let options = TreeOptions {
            root: dir.path().to_path_buf(),
            dirs_only: true,
            ..Default::default()
        };

        let result = generate_tree(options).expect("Failed to generate tree");
        let text = result.formatted_text;

        // Only directories should appear
        assert!(text.contains("src/"));
        assert!(text.contains("utils/"));
        assert!(!text.contains("Cargo.toml"));
        assert!(!text.contains("README.md"));
        assert!(!text.contains("main.rs"));
        assert!(text.contains("directories") || text.contains("directory"));
        assert!(!text.contains("files"));
    }

    #[test]
    fn test_tree_custom_ignore_patterns() {
        let dir = create_test_directory();
        let options = TreeOptions {
            root: dir.path().to_path_buf(),
            ignore_patterns: vec!["*.md".to_string(), "tests".to_string()],
            ..Default::default()
        };

        let result = generate_tree(options).expect("Failed to generate tree");
        let text = result.formatted_text;

        assert!(!text.contains("README.md"));
        assert!(!text.contains("tests/"));
        assert!(text.contains("Cargo.toml"));
        assert!(text.contains("src/"));
    }

    #[test]
    fn test_tree_nested_gitignore() {
        let dir = tempdir().expect("Failed to create tempdir");
        let root = dir.path();

        fs::create_dir_all(root.join("packages").join("pkg-a")).unwrap();
        fs::create_dir_all(root.join("packages").join("pkg-b")).unwrap();

        let mut f1 = File::create(root.join("packages").join("pkg-a").join("ignored.txt")).unwrap();
        f1.write_all(b"ignore me").unwrap();

        let mut f2 = File::create(root.join("packages").join("pkg-a").join("kept.txt")).unwrap();
        f2.write_all(b"keep me").unwrap();

        let mut f3 = File::create(root.join("packages").join("pkg-b").join("ignored.txt")).unwrap();
        f3.write_all(b"ignore me too").unwrap();

        // Local gitignore in pkg-a
        let mut gi = File::create(root.join("packages").join("pkg-a").join(".gitignore")).unwrap();
        gi.write_all(b"ignored.txt\n").unwrap();

        let options = TreeOptions {
            root: root.to_path_buf(),
            respect_gitignore: true,
            ..Default::default()
        };

        let result = generate_tree(options).expect("Failed to generate tree");
        let text = result.formatted_text;

        // pkg-a/ignored.txt is ignored by local gitignore
        // pkg-a/kept.txt is kept
        // pkg-b/ignored.txt is kept because pkg-b has no gitignore
        assert!(text.contains("kept.txt"));
        assert!(text.contains("packages/"));
    }

    #[test]
    fn test_tree_pattern_filter() {
        let dir = create_test_directory();
        let options = TreeOptions {
            root: dir.path().to_path_buf(),
            pattern: Some("*.rs".to_string()),
            ..Default::default()
        };

        let result = generate_tree(options).expect("Failed to generate tree");
        let text = result.formatted_text;

        assert!(text.contains("main.rs"));
        assert!(text.contains("helper.rs"));
        assert!(!text.contains("Cargo.toml"));
        assert!(!text.contains("README.md"));
    }

    #[test]
    fn test_tree_sorting_modes() {
        let dir = create_test_directory();

        // Sort by name
        let options_name = TreeOptions {
            root: dir.path().to_path_buf(),
            sort: TreeSort::Name,
            ..Default::default()
        };
        let res_name = generate_tree(options_name).unwrap();
        assert!(!res_name.formatted_text.is_empty());

        // Sort by size
        let options_size = TreeOptions {
            root: dir.path().to_path_buf(),
            sort: TreeSort::Size,
            ..Default::default()
        };
        let res_size = generate_tree(options_size).unwrap();
        assert!(!res_size.formatted_text.is_empty());

        // Sort by size ascending
        let options_size_asc = TreeOptions {
            root: dir.path().to_path_buf(),
            sort: TreeSort::SizeAsc,
            ..Default::default()
        };
        let res_size_asc = generate_tree(options_size_asc).unwrap();
        assert!(!res_size_asc.formatted_text.is_empty());

        // Sort by modified
        let options_mod = TreeOptions {
            root: dir.path().to_path_buf(),
            sort: TreeSort::Modified,
            ..Default::default()
        };
        let res_mod = generate_tree(options_mod).unwrap();
        assert!(!res_mod.formatted_text.is_empty());
    }

    #[test]
    fn test_tree_truncation_limit() {
        let dir = create_test_directory();
        let options = TreeOptions {
            root: dir.path().to_path_buf(),
            max_entries: 3,
            ..Default::default()
        };

        let result = generate_tree(options).expect("Failed to generate tree");
        assert!(result.stats.truncated);
        assert!(result.formatted_text.contains("[truncated at 3 entries]"));
    }

    #[test]
    fn test_tree_empty_directory() {
        let dir = tempdir().expect("Failed to create tempdir");
        let options = TreeOptions {
            root: dir.path().to_path_buf(),
            ..Default::default()
        };

        let result = generate_tree(options).expect("Failed to generate tree");
        assert_eq!(result.stats.directories, 0);
        assert_eq!(result.stats.files, 0);
        assert!(result.formatted_text.contains("0 directories, 0 files"));
    }

    #[tokio::test]
    async fn test_tree_tool_execute() {
        let dir = create_test_directory();
        let tool = TreeTool::new();
        let ctx = ToolContext {
            cwd: dir.path().to_path_buf(),
            env: std::collections::HashMap::new(),
        };

        // Execute default tree
        let res = tool
            .execute(json!({}), &ctx)
            .await
            .expect("Tool execution failed");
        assert!(res.contains("src/"));
        assert!(res.contains("Cargo.toml"));

        // Execute ASCII tree
        let ascii_res = tool
            .execute(json!({ "ascii": true }), &ctx)
            .await
            .expect("ASCII tree failed");
        assert!(ascii_res.contains("|-- ") || ascii_res.contains("\\-- "));
        assert!(!ascii_res.contains("├── "));

        // Execute with depth limit
        let depth_res = tool
            .execute(json!({ "max_depth": 1 }), &ctx)
            .await
            .expect("Depth tree failed");
        assert!(depth_res.contains("src/"));
        assert!(!depth_res.contains("helper.rs"));

        // Execute with JSON format
        let json_res = tool
            .execute(json!({ "format": "json" }), &ctx)
            .await
            .expect("JSON tree failed");
        let parsed: Value = serde_json::from_str(&json_res).expect("Invalid JSON returned");
        assert!(parsed.get("tree").is_some());
        assert!(parsed.get("stats").is_some());
    }
}
