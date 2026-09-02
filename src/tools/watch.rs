use async_trait::async_trait;
use globset::{Glob, GlobSet, GlobSetBuilder};
use ignore::WalkBuilder;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::{HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, LazyLock};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::sync::{broadcast, Notify, RwLock};

use crate::tools::file::resolve_path;
use crate::tools::types::{Tool, ToolContext};

// ---------------------------------------------------------------------------
// ChangeKind & FileChange
// ---------------------------------------------------------------------------

/// The nature of a detected file system change.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChangeKind {
    /// A new file or directory was created.
    Created,
    /// An existing file's contents, size, or metadata was modified.
    Modified,
    /// An existing file or directory was removed.
    Deleted,
}

impl std::fmt::Display for ChangeKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ChangeKind::Created => write!(f, "created"),
            ChangeKind::Modified => write!(f, "modified"),
            ChangeKind::Deleted => write!(f, "deleted"),
        }
    }
}

/// An asynchronous notification of a file system modification.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileChange {
    /// Relative path from the watched root or workspace.
    pub path: String,
    /// Full canonical or resolved filesystem path.
    pub full_path: PathBuf,
    /// The kind of modification detected.
    pub kind: ChangeKind,
    /// RFC 3339 formatted timestamp when the change was observed.
    pub timestamp: String,
    /// Current size in bytes (None for deleted entries).
    pub size: Option<u64>,
    /// Whether the target is a directory.
    pub is_dir: bool,
    /// Optional human-readable details about the change (e.g. size delta, hash delta).
    pub details: Option<String>,
}

impl FileChange {
    /// Returns a compact formatted string representation of this change.
    pub fn format_entry(&self) -> String {
        let tag = match self.kind {
            ChangeKind::Created => "[CREATED] ",
            ChangeKind::Modified => "[MODIFIED]",
            ChangeKind::Deleted => "[DELETED] ",
        };

        let size_str = match self.size {
            Some(s) if !self.is_dir => format!(" ({})", format_size(s)),
            _ if self.is_dir => " (dir)".to_string(),
            _ => String::new(),
        };

        let details_str = match &self.details {
            Some(d) if !d.is_empty() => format!(" - {d}"),
            _ => String::new(),
        };

        format!("{tag} {}{size_str}{details_str}", self.path)
    }
}

/// Formats byte sizes into human-readable strings.
pub fn format_size(bytes: u64) -> String {
    const KB: u64 = 1024;
    const MB: u64 = 1024 * KB;
    const GB: u64 = 1024 * MB;

    if bytes >= GB {
        format!("{:.1} GB", bytes as f64 / GB as f64)
    } else if bytes >= MB {
        format!("{:.1} MB", bytes as f64 / MB as f64)
    } else if bytes >= KB {
        format!("{:.1} KB", bytes as f64 / KB as f64)
    } else {
        format!("{bytes} B")
    }
}

// ---------------------------------------------------------------------------
// Pure-Rust Content Hashing (FNV-1a 64-bit)
// ---------------------------------------------------------------------------

/// Computes a fast 64-bit FNV-1a hash over byte slices.
#[inline]
pub fn fnv1a_hash(bytes: &[u8]) -> u64 {
    const FNV_OFFSET: u64 = 0xcbf29ce484222325;
    const FNV_PRIME: u64 = 0x100000001b3;

    let mut hash = FNV_OFFSET;
    for &byte in bytes {
        hash ^= byte as u64;
        hash = hash.wrapping_mul(FNV_PRIME);
    }
    hash
}

/// Computes a content or metadata hash for a file.
/// For files <= 1MB, reads content and computes FNV-1a.
/// For larger files or unreadable entries, hashes (size, mtime).
pub fn compute_file_hash(path: &Path, size: u64, mtime: SystemTime) -> u64 {
    const MAX_HASH_SIZE: u64 = 1024 * 1024; // 1 MB

    if size <= MAX_HASH_SIZE {
        if let Ok(bytes) = std::fs::read(path) {
            return fnv1a_hash(&bytes);
        }
    }

    // Fallback: fast hash of size and mtime
    let duration = mtime
        .duration_since(UNIX_EPOCH)
        .unwrap_or(Duration::from_secs(0));
    let mut hash = fnv1a_hash(&size.to_le_bytes());
    hash ^= fnv1a_hash(&duration.as_nanos().to_le_bytes());
    hash
}

// ---------------------------------------------------------------------------
// FileRecord & FileSnapshot
// ---------------------------------------------------------------------------

/// Metadata record for a single tracked file or directory.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileRecord {
    pub path: PathBuf,
    pub full_path: PathBuf,
    pub mtime: SystemTime,
    pub size: u64,
    pub is_dir: bool,
    pub hash: u64,
}

/// A point-in-time snapshot of files within a directory tree.
#[derive(Debug, Clone, Default)]
pub struct FileSnapshot {
    pub timestamp: String,
    pub root: PathBuf,
    pub files: HashMap<PathBuf, FileRecord>,
}

impl FileSnapshot {
    /// Captures the current state of files under `root` according to `config`.
    pub fn capture(config: &WatchConfig) -> anyhow::Result<Self> {
        let root = &config.root;
        if !root.exists() {
            anyhow::bail!("Path does not exist: '{}'", root.display());
        }

        let now = chrono::Utc::now().to_rfc3339();
        let mut files = HashMap::new();

        let include_set = build_glob_set(&config.includes)?;
        let exclude_set = build_glob_set(&config.excludes)?;

        // If watching a single file rather than a directory:
        if root.is_file() {
            if let Ok(meta) = root.metadata() {
                let mtime = meta.modified().unwrap_or(UNIX_EPOCH);
                let size = meta.len();
                let hash = compute_file_hash(root, size, mtime);
                let rel = PathBuf::from(root.file_name().unwrap_or_default());
                files.insert(
                    rel.clone(),
                    FileRecord {
                        path: rel,
                        full_path: root.clone(),
                        mtime,
                        size,
                        is_dir: false,
                        hash,
                    },
                );
            }
            return Ok(Self {
                timestamp: now,
                root: root.clone(),
                files,
            });
        }

        let mut builder = WalkBuilder::new(root);
        builder
            .hidden(!config.hidden)
            .git_ignore(config.gitignore)
            .git_global(config.gitignore)
            .git_exclude(config.gitignore);

        if !config.recursive {
            builder.max_depth(Some(1));
        }

        for result in builder.build() {
            let entry = match result {
                Ok(e) => e,
                Err(_) => continue,
            };

            // Skip the search root itself
            if entry.depth() == 0 {
                continue;
            }

            let path = entry.path();
            let is_dir = entry.file_type().map(|ft| ft.is_dir()).unwrap_or(false);

            // Skip directory entries themselves from file-tracking if we only care about leaf changes,
            // but keep them if we want to detect empty dir creation/deletion.
            // We track both files and dirs.
            let rel_path = match path.strip_prefix(root) {
                Ok(p) => p.to_path_buf(),
                Err(_) => path.to_path_buf(),
            };

            let rel_str = rel_path.to_string_lossy().replace('\\', "/");

            // Exclude filter
            if let Some(set) = &exclude_set {
                if set.is_match(&rel_str) {
                    continue;
                }
            }

            // Default hardcoded excludes for common noisy directories
            if is_default_excluded(&rel_str) {
                continue;
            }

            // Include filter (if specified, entry must match at least one pattern)
            if let Some(set) = &include_set {
                if !is_dir && !set.is_match(&rel_str) {
                    continue;
                }
            }

            let meta = match entry.metadata() {
                Ok(m) => m,
                Err(_) => continue,
            };

            let mtime = meta.modified().unwrap_or(UNIX_EPOCH);
            let size = meta.len();
            let hash = if is_dir {
                0
            } else {
                compute_file_hash(path, size, mtime)
            };

            files.insert(
                rel_path.clone(),
                FileRecord {
                    path: rel_path,
                    full_path: path.to_path_buf(),
                    mtime,
                    size,
                    is_dir,
                    hash,
                },
            );
        }

        Ok(Self {
            timestamp: now,
            root: root.clone(),
            files,
        })
    }

    /// Computes differences between this older snapshot (`self`) and a newer snapshot (`current`).
    /// Returns an ordered list of `FileChange` items.
    pub fn diff(&self, current: &FileSnapshot) -> Vec<FileChange> {
        let now = chrono::Utc::now().to_rfc3339();
        let mut changes = Vec::new();

        // Check for Created and Modified entries
        for (rel_path, curr_rec) in &current.files {
            match self.files.get(rel_path) {
                None => {
                    changes.push(FileChange {
                        path: rel_path.to_string_lossy().replace('\\', "/"),
                        full_path: curr_rec.full_path.clone(),
                        kind: ChangeKind::Created,
                        timestamp: now.clone(),
                        size: Some(curr_rec.size),
                        is_dir: curr_rec.is_dir,
                        details: Some(if curr_rec.is_dir {
                            "new directory".to_string()
                        } else {
                            format!("new file ({})", format_size(curr_rec.size))
                        }),
                    });
                }
                Some(prev_rec) => {
                    // Check if modified
                    if prev_rec.is_dir != curr_rec.is_dir {
                        changes.push(FileChange {
                            path: rel_path.to_string_lossy().replace('\\', "/"),
                            full_path: curr_rec.full_path.clone(),
                            kind: ChangeKind::Modified,
                            timestamp: now.clone(),
                            size: Some(curr_rec.size),
                            is_dir: curr_rec.is_dir,
                            details: Some("type changed between directory and file".to_string()),
                        });
                    } else if !curr_rec.is_dir {
                        let size_diff = curr_rec.size != prev_rec.size;
                        let hash_diff = curr_rec.hash != prev_rec.hash;
                        let mtime_diff = curr_rec.mtime != prev_rec.mtime;

                        if size_diff || hash_diff || mtime_diff {
                            let detail = if size_diff {
                                format!(
                                    "size changed: {} -> {}",
                                    format_size(prev_rec.size),
                                    format_size(curr_rec.size)
                                )
                            } else if hash_diff {
                                "content modified".to_string()
                            } else {
                                "metadata/timestamp modified".to_string()
                            };

                            changes.push(FileChange {
                                path: rel_path.to_string_lossy().replace('\\', "/"),
                                full_path: curr_rec.full_path.clone(),
                                kind: ChangeKind::Modified,
                                timestamp: now.clone(),
                                size: Some(curr_rec.size),
                                is_dir: false,
                                details: Some(detail),
                            });
                        }
                    }
                }
            }
        }

        // Check for Deleted entries
        for (rel_path, prev_rec) in &self.files {
            if !current.files.contains_key(rel_path) {
                changes.push(FileChange {
                    path: rel_path.to_string_lossy().replace('\\', "/"),
                    full_path: prev_rec.full_path.clone(),
                    kind: ChangeKind::Deleted,
                    timestamp: now.clone(),
                    size: None,
                    is_dir: prev_rec.is_dir,
                    details: Some(if prev_rec.is_dir {
                        "directory deleted".to_string()
                    } else {
                        format!("file deleted (was {})", format_size(prev_rec.size))
                    }),
                });
            }
        }

        // Sort by path for deterministic, readable output
        changes.sort_by(|a, b| a.path.cmp(&b.path));
        changes
    }
}

/// Checks if a relative path matches common default noisy directories/patterns.
fn is_default_excluded(path_str: &str) -> bool {
    let p = path_str.trim_matches('/');
    p.starts_with(".git/")
        || p == ".git"
        || p.starts_with("target/")
        || p == "target"
        || p.starts_with("node_modules/")
        || p == "node_modules"
        || p.ends_with(".DS_Store")
        || p.ends_with(".swp")
        || p.ends_with('~')
}

/// Compiles a list of glob patterns into a `GlobSet`.
fn build_glob_set(patterns: &[String]) -> anyhow::Result<Option<GlobSet>> {
    if patterns.is_empty() {
        return Ok(None);
    }
    let mut builder = GlobSetBuilder::new();
    for p in patterns {
        let glob = Glob::new(p).map_err(|e| anyhow::anyhow!("Invalid glob pattern '{p}': {e}"))?;
        builder.add(glob);
    }
    let set = builder.build().map_err(|e| anyhow::anyhow!("Failed to build glob set: {e}"))?;
    Ok(Some(set))
}

// ---------------------------------------------------------------------------
// WatchConfig & WorkspaceWatcher
// ---------------------------------------------------------------------------

/// Configuration options for watching a workspace or directory.
#[derive(Debug, Clone)]
pub struct WatchConfig {
    /// Root directory or file path to watch.
    pub root: PathBuf,
    /// Whether to watch recursively through child subdirectories.
    pub recursive: bool,
    /// Polling interval for scanning.
    pub interval: Duration,
    /// Minimum debounce duration before publishing changes.
    pub debounce: Duration,
    /// Optional list of glob patterns to include (e.g. `["*.rs", "*.toml"]`).
    pub includes: Vec<String>,
    /// Optional list of glob patterns to exclude.
    pub excludes: Vec<String>,
    /// Whether to track hidden files/dotfiles.
    pub hidden: bool,
    /// Whether to respect `.gitignore` rules.
    pub gitignore: bool,
    /// Maximum number of historical change events to retain in memory.
    pub max_history: usize,
}

impl Default for WatchConfig {
    fn default() -> Self {
        Self {
            root: PathBuf::from("."),
            recursive: true,
            interval: Duration::from_millis(500),
            debounce: Duration::from_millis(100),
            includes: Vec::new(),
            excludes: Vec::new(),
            hidden: false,
            gitignore: true,
            max_history: 1000,
        }
    }
}

/// Active asynchronous workspace file watcher.
pub struct WorkspaceWatcher {
    id: String,
    config: WatchConfig,
    sender: broadcast::Sender<FileChange>,
    history: Arc<RwLock<VecDeque<FileChange>>>,
    last_snapshot: Arc<RwLock<FileSnapshot>>,
    baseline_snapshot: Arc<FileSnapshot>,
    running: Arc<AtomicBool>,
    handle: Option<tokio::task::JoinHandle<()>>,
    notify: Arc<Notify>,
    started_at: String,
    total_change_count: Arc<AtomicU64>,
}

impl WorkspaceWatcher {
    /// Starts a new background workspace file watcher.
    /// Captures an initial baseline snapshot and spawns an async polling task.
    pub fn start(id: impl Into<String>, config: WatchConfig) -> anyhow::Result<Self> {
        let id_str = id.into();
        let baseline = FileSnapshot::capture(&config)?;
        let baseline_arc = Arc::new(baseline.clone());

        let (sender, _) = broadcast::channel(512);
        let history = Arc::new(RwLock::new(VecDeque::with_capacity(config.max_history)));
        let last_snapshot = Arc::new(RwLock::new(baseline));
        let running = Arc::new(AtomicBool::new(true));
        let notify = Arc::new(Notify::new());
        let total_change_count = Arc::new(AtomicU64::new(0));

        let loop_config = config.clone();
        let loop_running = running.clone();
        let loop_history = history.clone();
        let loop_last_snapshot = last_snapshot.clone();
        let loop_sender = sender.clone();
        let loop_notify = notify.clone();
        let loop_change_count = total_change_count.clone();

        let handle = tokio::spawn(async move {
            while loop_running.load(Ordering::Relaxed) {
                tokio::time::sleep(loop_config.interval).await;
                if !loop_running.load(Ordering::Relaxed) {
                    break;
                }

                // Capture snapshot in blocking thread pool
                let cfg_clone = loop_config.clone();
                let current_res = tokio::task::spawn_blocking(move || {
                    FileSnapshot::capture(&cfg_clone)
                })
                .await;

                let current_snapshot = match current_res {
                    Ok(Ok(snap)) => snap,
                    _ => continue,
                };

                let changes = {
                    let prev = loop_last_snapshot.read().await;
                    prev.diff(&current_snapshot)
                };

                if !changes.is_empty() {
                    // Apply debounce if configured
                    if loop_config.debounce > Duration::ZERO {
                        tokio::time::sleep(loop_config.debounce).await;
                    }

                    // Update last snapshot
                    {
                        let mut last = loop_last_snapshot.write().await;
                        *last = current_snapshot;
                    }

                    // Update change count
                    loop_change_count.fetch_add(changes.len() as u64, Ordering::Relaxed);

                    // Record to bounded history
                    {
                        let mut hist = loop_history.write().await;
                        for change in &changes {
                            if hist.len() >= loop_config.max_history {
                                hist.pop_front();
                            }
                            hist.push_back(change.clone());
                        }
                    }

                    // Broadcast changes to active stream listeners
                    for change in &changes {
                        let _ = loop_sender.send(change.clone());
                    }

                    // Wake any async waiters waiting for changes
                    loop_notify.notify_waiters();
                }
            }
        });

        Ok(Self {
            id: id_str,
            config,
            sender,
            history,
            last_snapshot,
            baseline_snapshot: baseline_arc,
            running,
            handle: Some(handle),
            notify,
            started_at: chrono::Utc::now().to_rfc3339(),
            total_change_count,
        })
    }

    /// Subscribes to real-time asynchronous file change events via a broadcast receiver.
    pub fn subscribe(&self) -> broadcast::Receiver<FileChange> {
        self.sender.subscribe()
    }

    /// Asynchronously waits up to `timeout` for at least one file change event.
    /// Returns all changes that occurred during the wait period.
    pub async fn wait_for_changes(&self, timeout: Duration) -> Vec<FileChange> {
        let mut rx = self.subscribe();
        let mut collected = Vec::new();

        let wait_future = async {
            // First wait for the notify or first broadcast event
            match rx.recv().await {
                Ok(change) => {
                    collected.push(change);
                    // Drain any immediately available follow-up events
                    while let Ok(next) = rx.try_recv() {
                        collected.push(next);
                    }
                }
                Err(_) => {
                    // Fallback to notify trigger
                    self.notify.notified().await;
                }
            }
        };

        let _ = tokio::time::timeout(timeout, wait_future).await;

        if collected.is_empty() {
            // Check if any events arrived in history since call
            while let Ok(next) = rx.try_recv() {
                collected.push(next);
            }
        }

        collected
    }

    /// Retrieves all recorded changes from history, optionally clearing them.
    pub async fn get_changes(&self, clear: bool) -> Vec<FileChange> {
        let mut hist = self.history.write().await;
        if clear {
            hist.drain(..).collect()
        } else {
            hist.iter().cloned().collect()
        }
    }

    /// Compares the current workspace state against the baseline snapshot captured
    /// when the watcher initially started.
    pub async fn diff_from_baseline(&self) -> anyhow::Result<Vec<FileChange>> {
        let cfg = self.config.clone();
        let current = tokio::task::spawn_blocking(move || FileSnapshot::capture(&cfg))
            .await
            .map_err(|e| anyhow::anyhow!("Task join error: {e}"))??;

        Ok(self.baseline_snapshot.diff(&current))
    }

    /// Stops the watcher and halts its background polling task.
    pub fn stop(&mut self) {
        self.running.store(false, Ordering::SeqCst);
        if let Some(h) = self.handle.take() {
            h.abort();
        }
    }

    /// Returns whether the watcher is currently active.
    pub fn is_running(&self) -> bool {
        self.running.load(Ordering::Relaxed)
    }

    /// Unique identifier for this watcher.
    pub fn id(&self) -> &str {
        &self.id
    }

    /// Root directory being watched.
    pub fn root(&self) -> &Path {
        &self.config.root
    }

    /// Total number of individual change events observed since start.
    pub fn total_changes(&self) -> u64 {
        self.total_change_count.load(Ordering::Relaxed)
    }

    /// Number of files currently tracked in baseline.
    pub fn baseline_file_count(&self) -> usize {
        self.baseline_snapshot.files.len()
    }
}

impl Drop for WorkspaceWatcher {
    fn drop(&mut self) {
        self.stop();
    }
}

// ---------------------------------------------------------------------------
// WatcherManager (Global Registry)
// ---------------------------------------------------------------------------

/// Metadata summary of an active watcher.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WatcherInfo {
    pub id: String,
    pub root: String,
    pub is_running: bool,
    pub started_at: String,
    pub total_changes: u64,
    pub baseline_files: usize,
    pub interval_ms: u64,
    pub recursive: bool,
}

/// Thread-safe registry managing active file watcher sessions.
pub struct WatcherManager {
    watchers: Arc<RwLock<HashMap<String, Arc<RwLock<WorkspaceWatcher>>>>>,
    manual_snapshots: Arc<RwLock<HashMap<String, FileSnapshot>>>,
}

impl WatcherManager {
    pub fn new() -> Self {
        Self {
            watchers: Arc::new(RwLock::new(HashMap::new())),
            manual_snapshots: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Starts a new watcher session under `id` (or generates one if omitted).
    pub async fn start_watcher(
        &self,
        id: Option<&str>,
        config: WatchConfig,
    ) -> anyhow::Result<String> {
        let watcher_id = match id {
            Some(i) if !i.trim().is_empty() => i.trim().to_string(),
            _ => format!("watch_{}", uuid::Uuid::new_v4().simple()),
        };

        let mut map = self.watchers.write().await;
        if let Some(existing) = map.get_mut(&watcher_id) {
            let mut w = existing.write().await;
            if w.is_running() {
                w.stop();
            }
        }

        let watcher = WorkspaceWatcher::start(watcher_id.clone(), config)?;
        map.insert(watcher_id.clone(), Arc::new(RwLock::new(watcher)));
        Ok(watcher_id)
    }

    /// Retrieves an active watcher by ID.
    pub async fn get_watcher(&self, id: &str) -> Option<Arc<RwLock<WorkspaceWatcher>>> {
        let map = self.watchers.read().await;
        map.get(id).cloned()
    }

    /// Stops a watcher by ID. Returns true if the watcher was found and stopped.
    pub async fn stop_watcher(&self, id: &str) -> bool {
        let mut map = self.watchers.write().await;
        if let Some(w_arc) = map.remove(id) {
            let mut w = w_arc.write().await;
            w.stop();
            true
        } else {
            false
        }
    }

    /// Lists summaries of all known watchers.
    pub async fn list_watchers(&self) -> Vec<WatcherInfo> {
        let map = self.watchers.read().await;
        let mut list = Vec::new();
        for (id, w_arc) in map.iter() {
            let w = w_arc.read().await;
            list.push(WatcherInfo {
                id: id.clone(),
                root: w.root().to_string_lossy().to_string(),
                is_running: w.is_running(),
                started_at: w.started_at.clone(),
                total_changes: w.total_changes(),
                baseline_files: w.baseline_file_count(),
                interval_ms: w.config.interval.as_millis() as u64,
                recursive: w.config.recursive,
            });
        }
        list.sort_by(|a, b| a.id.cmp(&b.id));
        list
    }

    /// Saves a manual snapshot under `name`.
    pub async fn save_snapshot(&self, name: &str, snapshot: FileSnapshot) {
        let mut map = self.manual_snapshots.write().await;
        map.insert(name.to_string(), snapshot);
    }

    /// Retrieves a manual snapshot by `name`.
    pub async fn get_snapshot(&self, name: &str) -> Option<FileSnapshot> {
        let map = self.manual_snapshots.read().await;
        map.get(name).cloned()
    }

    /// Stops all running watchers.
    pub async fn stop_all(&self) {
        let mut map = self.watchers.write().await;
        for (_, w_arc) in map.drain() {
            let mut w = w_arc.write().await;
            w.stop();
        }
    }
}

/// Global singleton watcher manager.
pub static GLOBAL_WATCHER_MANAGER: LazyLock<WatcherManager> = LazyLock::new(WatcherManager::new);

/// Returns a reference to the global `WatcherManager`.
pub fn global_watcher_manager() -> &'static WatcherManager {
    &GLOBAL_WATCHER_MANAGER
}

// ---------------------------------------------------------------------------
// WatchTool (Tool Implementation)
// ---------------------------------------------------------------------------

/// Tool for watching workspace modifications, querying detected changes, and
/// awaiting asynchronous file change notifications during long tasks.
#[derive(Default, Debug, Clone)]
pub struct WatchTool;

impl WatchTool {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl Tool for WatchTool {
    fn name(&self) -> &str {
        "watch"
    }

    fn description(&self) -> &str {
        "Watch files or directories for changes, track workspace modifications during long tasks, and receive asynchronous file change notifications."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "action": {
                    "type": "string",
                    "enum": ["start", "wait", "changes", "diff", "status", "stop", "snapshot", "list"],
                    "description": "Action to perform: 'start' (begin background watcher), 'wait' (await changes asynchronously up to timeout), 'changes' (poll/drain detected changes), 'diff' (compare current state against baseline), 'status' (get watcher status), 'stop' (halt watcher), 'snapshot' (record instant snapshot), 'list' (list all active watchers)."
                },
                "path": {
                    "type": "string",
                    "description": "Path to the directory or file to watch (relative to workspace or absolute). Defaults to current workspace root."
                },
                "watch_id": {
                    "type": "string",
                    "description": "Identifier for the watch session (optional, auto-generated if omitted on 'start')."
                },
                "interval_ms": {
                    "type": "integer",
                    "description": "Polling interval in milliseconds (optional, default: 500ms)."
                },
                "timeout_ms": {
                    "type": "integer",
                    "description": "Timeout in milliseconds when waiting for changes with 'wait' (optional, default: 5000ms)."
                },
                "recursive": {
                    "type": "boolean",
                    "description": "Whether to watch subdirectories recursively (optional, default: true)."
                },
                "include": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "Optional glob patterns of files to track (e.g. ['*.rs', '*.toml'])."
                },
                "exclude": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "Optional glob patterns of files/directories to ignore."
                },
                "clear": {
                    "type": "boolean",
                    "description": "Whether to clear reported changes after reading them in 'changes' (optional, default: true)."
                },
                "hidden": {
                    "type": "boolean",
                    "description": "Whether to include hidden files (dotfiles) in tracking (optional, default: false)."
                }
            }
        })
    }

    async fn execute(&self, args: Value, ctx: &ToolContext) -> anyhow::Result<String> {
        let mgr = global_watcher_manager();

        let path_str = args
            .get("path")
            .and_then(|v| v.as_str())
            .or_else(|| args.get("dir").and_then(|v| v.as_str()));

        let target_path = match path_str {
            Some(p) => resolve_path(p, &ctx.cwd),
            None => ctx.cwd.clone(),
        };

        let watch_id = args.get("watch_id").and_then(|v| v.as_str());

        let action = args
            .get("action")
            .and_then(|v| v.as_str())
            .unwrap_or_else(|| {
                // Infer smart default
                if args.get("timeout_ms").is_some() {
                    "wait"
                } else if watch_id.is_some() {
                    "changes"
                } else {
                    "start"
                }
            });

        match action {
            "start" => {
                if !target_path.exists() {
                    anyhow::bail!("Cannot watch non-existent path: '{}'", target_path.display());
                }

                let interval_ms = args
                    .get("interval_ms")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(500);

                let recursive = args
                    .get("recursive")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(true);

                let hidden = args
                    .get("hidden")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false);

                let includes: Vec<String> = args
                    .get("include")
                    .and_then(|v| v.as_array())
                    .map(|arr| {
                        arr.iter()
                            .filter_map(|x| x.as_str().map(|s| s.to_string()))
                            .collect()
                    })
                    .unwrap_or_default();

                let excludes: Vec<String> = args
                    .get("exclude")
                    .and_then(|v| v.as_array())
                    .map(|arr| {
                        arr.iter()
                            .filter_map(|x| x.as_str().map(|s| s.to_string()))
                            .collect()
                    })
                    .unwrap_or_default();

                let config = WatchConfig {
                    root: target_path.clone(),
                    recursive,
                    interval: Duration::from_millis(interval_ms.max(50)),
                    debounce: Duration::from_millis(100),
                    includes,
                    excludes,
                    hidden,
                    gitignore: true,
                    max_history: 1000,
                };

                let assigned_id = mgr.start_watcher(watch_id, config).await?;
                let watcher_arc = mgr.get_watcher(&assigned_id).await.unwrap();
                let watcher = watcher_arc.read().await;

                Ok(format!(
                    "Watcher started successfully.\n\
                     - ID: {assigned_id}\n\
                     - Root: {}\n\
                     - Baseline tracked files: {}\n\
                     - Polling interval: {}ms\n\
                     - Recursive: {}\n\
                     Use action 'changes' to poll modifications or 'wait' to receive asynchronous notifications.",
                    target_path.display(),
                    watcher.baseline_file_count(),
                    interval_ms,
                    recursive
                ))
            }

            "wait" => {
                let timeout_ms = args
                    .get("timeout_ms")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(5000);
                let timeout = Duration::from_millis(timeout_ms);

                // Find target watcher
                let watcher_arc = if let Some(id) = watch_id {
                    mgr.get_watcher(id)
                        .await
                        .ok_or_else(|| anyhow::anyhow!("Watcher not found: '{id}'"))?
                } else {
                    // Try to find a watcher matching target_path, or create a temporary one-shot watcher
                    let mut found = None;
                    let watchers = mgr.list_watchers().await;
                    for w_info in watchers {
                        if Path::new(&w_info.root) == target_path {
                            found = mgr.get_watcher(&w_info.id).await;
                            break;
                        }
                    }

                    match found {
                        Some(w) => w,
                        None => {
                            // Start a watcher on target_path
                            let temp_id = format!("watch_wait_{}", uuid::Uuid::new_v4().simple());
                            let config = WatchConfig {
                                root: target_path.clone(),
                                recursive: true,
                                interval: Duration::from_millis(200),
                                debounce: Duration::from_millis(50),
                                includes: Vec::new(),
                                excludes: Vec::new(),
                                hidden: false,
                                gitignore: true,
                                max_history: 1000,
                            };
                            let assigned = mgr.start_watcher(Some(&temp_id), config).await?;
                            mgr.get_watcher(&assigned).await.unwrap()
                        }
                    }
                };

                let changes = {
                    let w = watcher_arc.read().await;
                    w.wait_for_changes(timeout).await
                };

                if changes.is_empty() {
                    Ok(format!(
                        "No file changes detected within {}ms timeout.",
                        timeout_ms
                    ))
                } else {
                    format_changes_response(&changes)
                }
            }

            "changes" => {
                let clear = args
                    .get("clear")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(true);

                let watcher_arc = if let Some(id) = watch_id {
                    mgr.get_watcher(id)
                        .await
                        .ok_or_else(|| anyhow::anyhow!("Watcher not found: '{id}'"))?
                } else {
                    // Find first active watcher matching target_path or return error
                    let mut found = None;
                    let watchers = mgr.list_watchers().await;
                    for w_info in watchers {
                        if Path::new(&w_info.root) == target_path {
                            found = mgr.get_watcher(&w_info.id).await;
                            break;
                        }
                    }
                    found.ok_or_else(|| {
                        anyhow::anyhow!(
                            "No active watcher found for '{}'. Please specify 'watch_id' or start a watcher first.",
                            target_path.display()
                        )
                    })?
                };

                let changes = {
                    let w = watcher_arc.read().await;
                    w.get_changes(clear).await
                };

                if changes.is_empty() {
                    Ok("No new file modifications detected since last query.".to_string())
                } else {
                    format_changes_response(&changes)
                }
            }

            "diff" => {
                // If watch_id provided, diff from its baseline; otherwise instant snapshot diff
                if let Some(id) = watch_id {
                    let watcher_arc = mgr
                        .get_watcher(id)
                        .await
                        .ok_or_else(|| anyhow::anyhow!("Watcher not found: '{id}'"))?;
                    let w = watcher_arc.read().await;
                    let changes = w.diff_from_baseline().await?;

                    if changes.is_empty() {
                        Ok(format!("Workspace is identical to baseline snapshot for '{id}'."))
                    } else {
                        let mut out = format!(
                            "Baseline diff for '{id}' ({} change(s) detected since start):\n",
                            changes.len()
                        );
                        for c in changes {
                            out.push_str(&format!("  {}\n", c.format_entry()));
                        }
                        Ok(out)
                    }
                } else {
                    // Check if a manual snapshot exists for this path
                    let key = target_path.to_string_lossy().to_string();
                    let snap = mgr.get_snapshot(&key).await.ok_or_else(|| {
                        anyhow::anyhow!(
                            "No baseline snapshot found for '{}'. Run action 'snapshot' first or specify 'watch_id'.",
                            target_path.display()
                        )
                    })?;

                    let config = WatchConfig {
                        root: target_path.clone(),
                        ..Default::default()
                    };
                    let current = FileSnapshot::capture(&config)?;
                    let changes = snap.diff(&current);

                    if changes.is_empty() {
                        Ok("No differences from saved snapshot.".to_string())
                    } else {
                        let mut out = format!("Diff from snapshot ({} changes):\n", changes.len());
                        for c in changes {
                            out.push_str(&format!("  {}\n", c.format_entry()));
                        }
                        Ok(out)
                    }
                }
            }

            "snapshot" => {
                let config = WatchConfig {
                    root: target_path.clone(),
                    ..Default::default()
                };
                let snapshot = FileSnapshot::capture(&config)?;
                let file_count = snapshot.files.len();
                let key = watch_id
                    .unwrap_or_else(|| target_path.to_str().unwrap_or("default"))
                    .to_string();

                mgr.save_snapshot(&key, snapshot).await;
                Ok(format!(
                    "Captured snapshot '{key}' with {file_count} tracked files in '{}'.",
                    target_path.display()
                ))
            }

            "status" => {
                let id = watch_id.ok_or_else(|| {
                    anyhow::anyhow!("Missing parameter 'watch_id' required for 'status'")
                })?;
                let watcher_arc = mgr
                    .get_watcher(id)
                    .await
                    .ok_or_else(|| anyhow::anyhow!("Watcher not found: '{id}'"))?;
                let w = watcher_arc.read().await;

                Ok(format!(
                    "Watcher Status:\n\
                     - ID: {}\n\
                     - Active: {}\n\
                     - Root: {}\n\
                     - Started at: {}\n\
                     - Baseline files: {}\n\
                     - Total events captured: {}\n\
                     - Polling interval: {}ms",
                    w.id(),
                    w.is_running(),
                    w.root().display(),
                    w.started_at,
                    w.baseline_file_count(),
                    w.total_changes(),
                    w.config.interval.as_millis()
                ))
            }

            "stop" => {
                let id = watch_id.ok_or_else(|| {
                    anyhow::anyhow!("Missing parameter 'watch_id' required for 'stop'")
                })?;
                let stopped = mgr.stop_watcher(id).await;
                if stopped {
                    Ok(format!("Watcher '{id}' stopped successfully."))
                } else {
                    anyhow::bail!("Watcher '{id}' not found or already stopped.")
                }
            }

            "list" => {
                let watchers = mgr.list_watchers().await;
                if watchers.is_empty() {
                    Ok("No active watchers registered.".to_string())
                } else {
                    let mut out = format!("Active Watchers ({}):\n", watchers.len());
                    for w in watchers {
                        out.push_str(&format!(
                            "- ID: {}\n  Root: {}\n  Running: {}\n  Baseline files: {}\n  Total events: {}\n",
                            w.id, w.root, w.is_running, w.baseline_files, w.total_changes
                        ));
                    }
                    Ok(out)
                }
            }

            unknown => {
                anyhow::bail!(
                    "Unknown action: '{unknown}'. Supported actions are: start, wait, changes, diff, status, stop, snapshot, list."
                );
            }
        }
    }
}

/// Helper function to format an array of `FileChange` items into a clear summary response.
fn format_changes_response(changes: &[FileChange]) -> anyhow::Result<String> {
    let mut created = 0;
    let mut modified = 0;
    let mut deleted = 0;

    for c in changes {
        match c.kind {
            ChangeKind::Created => created += 1,
            ChangeKind::Modified => modified += 1,
            ChangeKind::Deleted => deleted += 1,
        }
    }

    let mut out = format!(
        "Detected {} file change(s) ({} created, {} modified, {} deleted):\n",
        changes.len(),
        created,
        modified,
        deleted
    );

    for c in changes {
        out.push_str(&format!("  {}\n", c.format_entry()));
    }

    Ok(out)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering as AtomicOrdering};

    static TEST_COUNTER: AtomicUsize = AtomicUsize::new(0);

    fn make_temp_dir() -> PathBuf {
        let count = TEST_COUNTER.fetch_add(1, AtomicOrdering::SeqCst);
        let dir = std::env::temp_dir().join(format!(
            "fusion_watch_test_{}_{}_{}",
            std::process::id(),
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0),
            count
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn test_format_size() {
        assert_eq!(format_size(500), "500 B");
        assert_eq!(format_size(2048), "2.0 KB");
        assert_eq!(format_size(1024 * 1024 * 3), "3.0 MB");
    }

    #[test]
    fn test_fnv1a_hash() {
        let h1 = fnv1a_hash(b"hello world");
        let h2 = fnv1a_hash(b"hello world");
        let h3 = fnv1a_hash(b"hello world!");
        assert_eq!(h1, h2);
        assert_ne!(h1, h3);
    }

    #[tokio::test]
    async fn test_snapshot_diff_created_modified_deleted() {
        let dir = make_temp_dir();
        let file_a = dir.join("file_a.txt");
        let file_b = dir.join("file_b.txt");

        std::fs::write(&file_a, "initial a").unwrap();
        std::fs::write(&file_b, "initial b").unwrap();

        let config = WatchConfig {
            root: dir.clone(),
            ..Default::default()
        };

        // Snapshot 1
        let snap1 = FileSnapshot::capture(&config).unwrap();
        assert_eq!(snap1.files.len(), 2);

        // Modify file_a, delete file_b, create file_c
        std::fs::write(&file_a, "modified content of file a with different length").unwrap();
        std::fs::remove_file(&file_b).unwrap();
        let file_c = dir.join("file_c.txt");
        std::fs::write(&file_c, "new file c").unwrap();

        // Snapshot 2
        let snap2 = FileSnapshot::capture(&config).unwrap();
        assert_eq!(snap2.files.len(), 2);

        let diff = snap1.diff(&snap2);
        assert_eq!(diff.len(), 3);

        let created = diff.iter().find(|c| c.kind == ChangeKind::Created).unwrap();
        assert_eq!(created.path, "file_c.txt");

        let modified = diff.iter().find(|c| c.kind == ChangeKind::Modified).unwrap();
        assert_eq!(modified.path, "file_a.txt");

        let deleted = diff.iter().find(|c| c.kind == ChangeKind::Deleted).unwrap();
        assert_eq!(deleted.path, "file_b.txt");

        let _ = std::fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn test_async_workspace_watcher_notifications() {
        let dir = make_temp_dir();
        let initial_file = dir.join("init.txt");
        std::fs::write(&initial_file, "initial").unwrap();

        let config = WatchConfig {
            root: dir.clone(),
            interval: Duration::from_millis(50),
            debounce: Duration::from_millis(10),
            ..Default::default()
        };

        let mut watcher = WorkspaceWatcher::start("test_watch_async", config).unwrap();
        assert_eq!(watcher.baseline_file_count(), 1);

        // Subscribe to real-time asynchronous notifications
        let mut rx = watcher.subscribe();

        // Add a new file
        let new_file = dir.join("added.rs");
        std::fs::write(&new_file, "pub fn test() {}").unwrap();

        // Await asynchronous notification from broadcast channel
        let event = tokio::time::timeout(Duration::from_secs(2), rx.recv())
            .await
            .expect("Should receive notification within timeout")
            .expect("Channel receive success");

        assert_eq!(event.kind, ChangeKind::Created);
        assert_eq!(event.path, "added.rs");

        watcher.stop();
        let _ = std::fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn test_wait_for_changes_timeout_and_detection() {
        let dir = make_temp_dir();
        let config = WatchConfig {
            root: dir.clone(),
            interval: Duration::from_millis(50),
            debounce: Duration::from_millis(10),
            ..Default::default()
        };

        let mut watcher = WorkspaceWatcher::start("test_wait_detector", config).unwrap();

        // 1. Wait with timeout when no changes happen -> empty vec
        let no_changes = watcher.wait_for_changes(Duration::from_millis(100)).await;
        assert!(no_changes.is_empty());

        // 2. Write file while waiting
        let target_file = dir.join("triggered.txt");
        let write_fut = async {
            tokio::time::sleep(Duration::from_millis(50)).await;
            std::fs::write(&target_file, "content").unwrap();
        };

        let wait_fut = watcher.wait_for_changes(Duration::from_secs(2));

        let (_, changes) = tokio::join!(write_fut, wait_fut);
        assert!(!changes.is_empty());
        assert_eq!(changes[0].path, "triggered.txt");
        assert_eq!(changes[0].kind, ChangeKind::Created);

        watcher.stop();
        let _ = std::fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn test_watch_tool_lifecycle() {
        let dir = make_temp_dir();
        let ctx = ToolContext {
            cwd: dir.clone(),
            env: HashMap::new(),
        };

        let tool = WatchTool::new();

        // 1. Start watcher
        let start_res = tool
            .execute(
                json!({
                    "action": "start",
                    "watch_id": "test_session_1",
                    "interval_ms": 50
                }),
                &ctx,
            )
            .await
            .unwrap();

        assert!(start_res.contains("Watcher started successfully"));
        assert!(start_res.contains("test_session_1"));

        // 2. Query status
        let status_res = tool
            .execute(
                json!({
                    "action": "status",
                    "watch_id": "test_session_1"
                }),
                &ctx,
            )
            .await
            .unwrap();

        assert!(status_res.contains("Active: true"));

        // 3. Create a file and poll changes
        let file_path = dir.join("hello.txt");
        std::fs::write(&file_path, "world").unwrap();

        tokio::time::sleep(Duration::from_millis(200)).await;

        let changes_res = tool
            .execute(
                json!({
                    "action": "changes",
                    "watch_id": "test_session_1"
                }),
                &ctx,
            )
            .await
            .unwrap();

        assert!(changes_res.contains("[CREATED]"));
        assert!(changes_res.contains("hello.txt"));

        // 4. Stop watcher
        let stop_res = tool
            .execute(
                json!({
                    "action": "stop",
                    "watch_id": "test_session_1"
                }),
                &ctx,
            )
            .await
            .unwrap();

        assert!(stop_res.contains("stopped successfully"));

        let _ = std::fs::remove_dir_all(dir);
    }
}
