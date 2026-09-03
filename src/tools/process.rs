use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::{HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, LazyLock};
use std::time::{Duration, Instant};
use tokio::sync::{mpsc, Notify, RwLock};

use crate::tools::bash::BashTool;
use crate::tools::file::resolve_path;
use crate::tools::types::{Tool, ToolContext};

// ---------------------------------------------------------------------------
// OutputStream & OutputLine
// ---------------------------------------------------------------------------

/// Identifies the source stream of a buffered output line.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OutputStream {
    /// Standard output stream.
    Stdout,
    /// Standard error stream.
    Stderr,
    /// Internal system event or process manager notification.
    System,
}

impl std::fmt::Display for OutputStream {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            OutputStream::Stdout => write!(f, "stdout"),
            OutputStream::Stderr => write!(f, "stderr"),
            OutputStream::System => write!(f, "system"),
        }
    }
}

/// A single timestamped line of output captured from a background process.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OutputLine {
    /// Source stream (stdout, stderr, or system).
    pub stream: OutputStream,
    /// The line content (without trailing newline).
    pub line: String,
    /// RFC 3339 formatted timestamp when this line was captured.
    pub timestamp: String,
    /// Monotonically increasing sequence number within the process buffer.
    pub sequence: u64,
}

impl OutputLine {
    /// Formats the line with stream prefix and optional timestamp.
    pub fn format(&self, include_timestamp: bool) -> String {
        let stream_tag = match self.stream {
            OutputStream::Stdout => "[stdout]",
            OutputStream::Stderr => "[stderr]",
            OutputStream::System => "[system]",
        };

        if include_timestamp {
            format!("{} [{}] {}", stream_tag, self.timestamp, self.line)
        } else {
            format!("{} {}", stream_tag, self.line)
        }
    }
}

// ---------------------------------------------------------------------------
// OutputBuffer (Thread-Safe Rolling Log Buffer)
// ---------------------------------------------------------------------------

/// Thread-safe rolling ring buffer for storing stdout, stderr, and system logs.
#[derive(Debug, Clone)]
pub struct OutputBuffer {
    max_lines: usize,
    lines: VecDeque<OutputLine>,
    sequence_counter: u64,
    total_lines_recorded: u64,
    total_bytes_recorded: u64,
}

impl OutputBuffer {
    /// Creates a new output buffer with the specified maximum line capacity.
    pub fn new(max_lines: usize) -> Self {
        Self {
            max_lines: max_lines.max(10),
            lines: VecDeque::new(),
            sequence_counter: 0,
            total_lines_recorded: 0,
            total_bytes_recorded: 0,
        }
    }

    /// Pushes a new line to the buffer, evicting the oldest line if at capacity.
    pub fn push(&mut self, stream: OutputStream, line: String) {
        self.sequence_counter = self.sequence_counter.wrapping_add(1);
        self.total_lines_recorded = self.total_lines_recorded.saturating_add(1);
        self.total_bytes_recorded = self.total_bytes_recorded.saturating_add(line.len() as u64);

        let entry = OutputLine {
            stream,
            line,
            timestamp: chrono::Utc::now().to_rfc3339(),
            sequence: self.sequence_counter,
        };

        if self.lines.len() >= self.max_lines {
            self.lines.pop_front();
        }
        self.lines.push_back(entry);
    }

    /// Retrieves lines matching the given stream filter, text search, offset, limit, and tail constraints.
    pub fn get_lines(
        &self,
        tail: Option<usize>,
        offset: Option<usize>,
        limit: Option<usize>,
        stream_filter: Option<OutputStream>,
        text_filter: Option<&str>,
    ) -> Vec<OutputLine> {
        let filtered: Vec<&OutputLine> = self
            .lines
            .iter()
            .filter(|entry| {
                if let Some(s) = stream_filter {
                    if entry.stream != s {
                        return false;
                    }
                }
                if let Some(pat) = text_filter {
                    if !pat.is_empty() && !entry.line.to_lowercase().contains(&pat.to_lowercase()) {
                        return false;
                    }
                }
                true
            })
            .collect();

        let total_matching = filtered.len();

        let slice = if let Some(t) = tail {
            let start = total_matching.saturating_sub(t);
            &filtered[start..]
        } else {
            let off = offset.unwrap_or(0).min(total_matching);
            let lim = limit.unwrap_or(total_matching);
            let end = (off + lim).min(total_matching);
            &filtered[off..end]
        };

        slice.iter().map(|&x| x.clone()).collect()
    }

    /// Returns combined stdout and stderr text for the last `tail` lines.
    pub fn combined_text(&self, tail: Option<usize>) -> String {
        let lines = self.get_lines(tail, None, None, None, None);
        lines
            .into_iter()
            .map(|l| l.line)
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// Returns stdout-only text for the last `tail` lines.
    pub fn stdout_text(&self, tail: Option<usize>) -> String {
        let lines = self.get_lines(tail, None, None, Some(OutputStream::Stdout), None);
        lines
            .into_iter()
            .map(|l| l.line)
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// Returns stderr-only text for the last `tail` lines.
    pub fn stderr_text(&self, tail: Option<usize>) -> String {
        let lines = self.get_lines(tail, None, None, Some(OutputStream::Stderr), None);
        lines
            .into_iter()
            .map(|l| l.line)
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// Clears all buffered lines while preserving sequence counters.
    pub fn clear(&mut self) {
        self.lines.clear();
    }

    /// Returns `(buffered_lines_count, total_lines_recorded, total_bytes_recorded)`.
    pub fn stats(&self) -> (usize, u64, u64) {
        (
            self.lines.len(),
            self.total_lines_recorded,
            self.total_bytes_recorded,
        )
    }

    /// Returns the maximum line capacity.
    pub fn max_lines(&self) -> usize {
        self.max_lines
    }
}

// ---------------------------------------------------------------------------
// ProcessStatus & ProcessInfo
// ---------------------------------------------------------------------------

/// Lifecycle state of a background process.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ProcessStatus {
    /// The process is currently executing.
    Running { pid: u32 },
    /// The process was stopped by user request.
    Stopped {
        exit_code: Option<i32>,
        signal: Option<String>,
    },
    /// The process completed its execution on its own.
    Exited {
        exit_code: Option<i32>,
        success: bool,
    },
    /// The process failed to start or crashed.
    Failed { error: String },
    /// The process was forcefully killed.
    Killed,
}

impl ProcessStatus {
    /// Returns true if the process is currently running.
    pub fn is_running(&self) -> bool {
        matches!(self, ProcessStatus::Running { .. })
    }

    /// Returns the system PID if running.
    pub fn pid(&self) -> Option<u32> {
        match self {
            ProcessStatus::Running { pid } => Some(*pid),
            _ => None,
        }
    }

    /// Returns the exit code if terminated.
    pub fn exit_code(&self) -> Option<i32> {
        match self {
            ProcessStatus::Exited { exit_code, .. } => *exit_code,
            ProcessStatus::Stopped { exit_code, .. } => *exit_code,
            _ => None,
        }
    }

    /// Returns a concise human-readable label for this status.
    pub fn label(&self) -> &'static str {
        match self {
            ProcessStatus::Running { .. } => "running",
            ProcessStatus::Stopped { .. } => "stopped",
            ProcessStatus::Exited { success: true, .. } => "exited (success)",
            ProcessStatus::Exited { success: false, .. } => "exited (failed)",
            ProcessStatus::Failed { .. } => "failed",
            ProcessStatus::Killed => "killed",
        }
    }
}

/// Comprehensive metadata and status information for a managed background process.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProcessInfo {
    /// Unique identifier for this process (e.g. "proc_1", "proc_2").
    pub id: String,
    /// Optional user-assigned name/label (e.g. "dev-server", "test-watcher").
    pub name: Option<String>,
    /// The command string that was launched.
    pub command: String,
    /// Working directory where the command was spawned.
    pub cwd: String,
    /// System PID if currently running.
    pub pid: Option<u32>,
    /// Current execution state.
    pub status: ProcessStatus,
    /// RFC 3339 formatted timestamp when the process started.
    pub started_at: String,
    /// RFC 3339 formatted timestamp when the process stopped (None if still running).
    pub stopped_at: Option<String>,
    /// Exit code if terminated.
    pub exit_code: Option<i32>,
    /// Total execution duration in seconds.
    pub uptime_secs: u64,
    /// Number of lines currently retained in the output buffer.
    pub buffered_lines: usize,
    /// Total lines produced over the lifetime of the process.
    pub total_lines: u64,
    /// Total bytes of output produced over the lifetime of the process.
    pub total_bytes: u64,
}

impl ProcessInfo {
    /// Returns a formatted single-line summary of the process state.
    pub fn format_summary(&self) -> String {
        let name_str = self
            .name
            .as_deref()
            .map(|n| format!(" ({n})"))
            .unwrap_or_default();
        let pid_str = self.pid.map(|p| format!(" [PID {p}]")).unwrap_or_default();
        let uptime_str = if self.status.is_running() {
            format!(" | uptime: {}s", self.uptime_secs)
        } else {
            String::new()
        };
        let lines_str = format!(" ({} log lines)", self.buffered_lines);

        format!(
            "[{}] {}{}{} - status: {}{}{}",
            self.id,
            self.command,
            name_str,
            pid_str,
            self.status.label(),
            uptime_str,
            lines_str
        )
    }

    /// Returns a detailed multi-line block describing the process.
    pub fn format_detailed(&self) -> String {
        let mut out = String::new();
        out.push_str(&format!("Process ID:   {}\n", self.id));
        if let Some(name) = &self.name {
            out.push_str(&format!("Name:         {}\n", name));
        }
        out.push_str(&format!("Command:      {}\n", self.command));
        out.push_str(&format!("Working Dir:  {}\n", self.cwd));
        if let Some(pid) = self.pid {
            out.push_str(&format!("PID:          {}\n", pid));
        }
        out.push_str(&format!("Status:       {}\n", self.status.label()));
        out.push_str(&format!("Started At:   {}\n", self.started_at));
        if let Some(stopped) = &self.stopped_at {
            out.push_str(&format!("Stopped At:   {}\n", stopped));
        }
        if let Some(code) = self.exit_code {
            out.push_str(&format!("Exit Code:    {}\n", code));
        }
        out.push_str(&format!("Uptime:       {}s\n", self.uptime_secs));
        out.push_str(&format!(
            "Logs:         {} buffered / {} total lines ({} bytes)\n",
            self.buffered_lines, self.total_lines, self.total_bytes
        ));
        out
    }
}

/// Configuration options for launching a background process.
#[derive(Debug, Clone)]
pub struct ProcessConfig {
    /// Shell command string to execute.
    pub command: String,
    /// Working directory for the process.
    pub cwd: PathBuf,
    /// Custom environment variables.
    pub env: HashMap<String, String>,
    /// Optional user-friendly name/label.
    pub name: Option<String>,
    /// Maximum number of log lines to retain in memory (default: 10,000).
    pub max_buffer_lines: usize,
}

impl ProcessConfig {
    /// Creates a new configuration with standard defaults.
    pub fn new(command: impl Into<String>) -> Self {
        Self {
            command: command.into(),
            cwd: std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")),
            env: HashMap::new(),
            name: None,
            max_buffer_lines: 10_000,
        }
    }

    /// Sets the working directory.
    pub fn with_cwd(mut self, cwd: impl Into<PathBuf>) -> Self {
        self.cwd = cwd.into();
        self
    }

    /// Sets custom environment variables.
    pub fn with_env(mut self, env: HashMap<String, String>) -> Self {
        self.env = env;
        self
    }

    /// Sets the friendly name/label.
    pub fn with_name(mut self, name: impl Into<String>) -> Self {
        self.name = Some(name.into());
        self
    }

    /// Sets maximum buffer lines.
    pub fn with_max_buffer_lines(mut self, max_lines: usize) -> Self {
        self.max_buffer_lines = max_lines;
        self
    }
}

/// Log query result containing captured output lines and metadata.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LogOutput {
    /// Process identifier.
    pub id: String,
    /// Optional process name.
    pub name: Option<String>,
    /// Current process status.
    pub status: ProcessStatus,
    /// The retrieved lines.
    pub lines: Vec<OutputLine>,
    /// Total lines produced over process lifetime.
    pub total_lines: u64,
    /// Number of lines currently buffered in memory.
    pub buffered_lines: usize,
    /// Number of lines returned in this query.
    pub returned_lines: usize,
}

impl LogOutput {
    /// Formats the logs into a readable string representation.
    pub fn format_text(&self, include_header: bool) -> String {
        let mut out = String::new();
        if include_header {
            let name_str = self
                .name
                .as_deref()
                .map(|n| format!(" ({n})"))
                .unwrap_or_default();
            out.push_str(&format!(
                "=== Logs for [{}{}] (status: {}, {} lines shown / {} buffered) ===\n",
                self.id,
                name_str,
                self.status.label(),
                self.returned_lines,
                self.buffered_lines
            ));
        }

        if self.lines.is_empty() {
            out.push_str("(no log output recorded)\n");
        } else {
            for line in &self.lines {
                let stream_tag = match line.stream {
                    OutputStream::Stdout => "[stdout]",
                    OutputStream::Stderr => "[stderr]",
                    OutputStream::System => "[system]",
                };
                out.push_str(&format!("{} {}\n", stream_tag, line.line));
            }
        }

        out
    }
}

// ---------------------------------------------------------------------------
// ManagedProcess (Runtime Handle)
// ---------------------------------------------------------------------------

/// Internal runtime handle representing a running or completed background process.
pub struct ManagedProcess {
    pub id: String,
    pub name: Option<String>,
    pub command: String,
    pub cwd: PathBuf,
    pub env: HashMap<String, String>,
    pub pid: Option<u32>,
    pub status: Arc<RwLock<ProcessStatus>>,
    pub started_at_instant: Instant,
    pub started_at: String,
    pub stopped_at: Arc<RwLock<Option<String>>>,
    pub buffer: Arc<RwLock<OutputBuffer>>,
    pub stdin_tx: Arc<tokio::sync::Mutex<Option<mpsc::Sender<String>>>>,
    pub stop_notify: Arc<Notify>,
    pub kill_notify: Arc<Notify>,
    pub exit_notify: Arc<Notify>,
}

impl ManagedProcess {
    /// Returns current snapshot of `ProcessInfo`.
    pub async fn info(&self) -> ProcessInfo {
        let status = self.status.read().await.clone();
        let stopped_at = self.stopped_at.read().await.clone();
        let buf = self.buffer.read().await;
        let (buffered_lines, total_lines, total_bytes) = buf.stats();
        let exit_code = status.exit_code();
        let uptime_secs = self.started_at_instant.elapsed().as_secs();

        ProcessInfo {
            id: self.id.clone(),
            name: self.name.clone(),
            command: self.command.clone(),
            cwd: self.cwd.to_string_lossy().to_string(),
            pid: self.pid,
            status,
            started_at: self.started_at.clone(),
            stopped_at,
            exit_code,
            uptime_secs,
            buffered_lines,
            total_lines,
            total_bytes,
        }
    }
}

// ---------------------------------------------------------------------------
// ProcessManager
// ---------------------------------------------------------------------------

/// Global and instance-level manager for background processes (dev servers, test watchers, etc.).
pub struct ProcessManager {
    processes: Arc<RwLock<HashMap<String, Arc<ManagedProcess>>>>,
    counter: AtomicU64,
    default_buffer_capacity: usize,
}

impl Default for ProcessManager {
    fn default() -> Self {
        Self::new()
    }
}

impl ProcessManager {
    /// Creates a new `ProcessManager`.
    pub fn new() -> Self {
        Self {
            processes: Arc::new(RwLock::new(HashMap::new())),
            counter: AtomicU64::new(1),
            default_buffer_capacity: 10_000,
        }
    }

    /// Creates a new `ProcessManager` with custom default buffer capacity.
    pub fn with_buffer_capacity(capacity: usize) -> Self {
        Self {
            processes: Arc::new(RwLock::new(HashMap::new())),
            counter: AtomicU64::new(1),
            default_buffer_capacity: capacity.max(10),
        }
    }

    /// Spawns a new background process with default configuration.
    pub async fn spawn(
        &self,
        cmd: &str,
        cwd: Option<&Path>,
        env: Option<HashMap<String, String>>,
        name: Option<&str>,
    ) -> anyhow::Result<ProcessInfo> {
        let trimmed_cmd = cmd.trim();
        if trimmed_cmd.is_empty() {
            anyhow::bail!("Command cannot be empty");
        }

        let working_dir = cwd
            .map(|p| p.to_path_buf())
            .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));

        let mut config = ProcessConfig::new(trimmed_cmd)
            .with_cwd(working_dir)
            .with_max_buffer_lines(self.default_buffer_capacity);

        if let Some(e) = env {
            config = config.with_env(e);
        }
        if let Some(n) = name {
            config = config.with_name(n);
        }

        self.spawn_with_config(config).await
    }

    /// Spawns a background process using a `ProcessConfig`.
    pub async fn spawn_with_config(&self, config: ProcessConfig) -> anyhow::Result<ProcessInfo> {
        let id_num = self.counter.fetch_add(1, Ordering::SeqCst);
        let id = format!("proc_{id_num}");
        self.spawn_internal(&id, config).await
    }

    /// Spawns a background process with a specific ID (used for spawn & restart).
    async fn spawn_internal(&self, id: &str, config: ProcessConfig) -> anyhow::Result<ProcessInfo> {
        if !config.cwd.exists() {
            anyhow::bail!("Working directory does not exist: {}", config.cwd.display());
        }

        if !config.cwd.is_dir() {
            anyhow::bail!(
                "Working directory is not a directory: {}",
                config.cwd.display()
            );
        }

        // Build shell command cross-platform
        let mut cmd = BashTool::build_command(&config.command);
        cmd.current_dir(&config.cwd);

        // Environment sanitization & custom variables
        let cleaner = crate::tools::env_cleaner::EnvCleaner::default();
        cleaner.apply_to_tokio_command(&mut cmd, Some(&config.env));

        cmd.stdin(std::process::Stdio::piped());
        cmd.stdout(std::process::Stdio::piped());
        cmd.stderr(std::process::Stdio::piped());

        // Spawn child
        let mut child = match cmd.spawn() {
            Ok(c) => c,
            Err(e) => {
                anyhow::bail!("Failed to spawn process '{}': {}", config.command, e);
            }
        };

        let pid = child.id();
        let stdout = child.stdout.take();
        let stderr = child.stderr.take();
        let stdin = child.stdin.take();

        let buffer = Arc::new(RwLock::new(OutputBuffer::new(config.max_buffer_lines)));
        let initial_status = match pid {
            Some(p) => ProcessStatus::Running { pid: p },
            None => ProcessStatus::Running { pid: 0 },
        };
        let status = Arc::new(RwLock::new(initial_status));
        let started_at = chrono::Utc::now().to_rfc3339();
        let started_at_instant = Instant::now();
        let stopped_at = Arc::new(RwLock::new(None));

        let stop_notify = Arc::new(Notify::new());
        let kill_notify = Arc::new(Notify::new());
        let exit_notify = Arc::new(Notify::new());

        // Log initial system entry
        {
            let mut buf = buffer.write().await;
            let pid_str = pid
                .map(|p| format!("PID {p}"))
                .unwrap_or_else(|| "unknown PID".to_string());
            buf.push(
                OutputStream::System,
                format!("Process started ({pid_str}): {}", config.command),
            );
        }

        // Set up stdin channel
        let (stdin_tx, mut stdin_rx) = mpsc::channel::<String>(64);
        if let Some(mut stdin_stream) = stdin {
            tokio::spawn(async move {
                use tokio::io::AsyncWriteExt;
                while let Some(data) = stdin_rx.recv().await {
                    if stdin_stream.write_all(data.as_bytes()).await.is_err() {
                        break;
                    }
                    if stdin_stream.flush().await.is_err() {
                        break;
                    }
                }
            });
        }

        // Set up stdout reader
        if let Some(stdout_stream) = stdout {
            let buf_clone = buffer.clone();
            tokio::spawn(async move {
                use tokio::io::{AsyncBufReadExt, BufReader};
                let reader = BufReader::new(stdout_stream);
                let mut lines = reader.lines();
                while let Ok(Some(line)) = lines.next_line().await {
                    let mut b = buf_clone.write().await;
                    b.push(OutputStream::Stdout, line);
                }
            });
        }

        // Set up stderr reader
        if let Some(stderr_stream) = stderr {
            let buf_clone = buffer.clone();
            tokio::spawn(async move {
                use tokio::io::{AsyncBufReadExt, BufReader};
                let reader = BufReader::new(stderr_stream);
                let mut lines = reader.lines();
                while let Ok(Some(line)) = lines.next_line().await {
                    let mut b = buf_clone.write().await;
                    b.push(OutputStream::Stderr, line);
                }
            });
        }

        // Set up exit monitoring & termination handler
        let status_clone = status.clone();
        let stopped_at_clone = stopped_at.clone();
        let buffer_clone = buffer.clone();
        let exit_notify_clone = exit_notify.clone();
        let stop_notify_clone = stop_notify.clone();
        let kill_notify_clone = kill_notify.clone();

        tokio::spawn(async move {
            let final_status = tokio::select! {
                exit_res = child.wait() => {
                    match exit_res {
                        Ok(exit_status) => {
                            let code = exit_status.code();
                            let success = exit_status.success();
                            ProcessStatus::Exited { exit_code: code, success }
                        }
                        Err(e) => {
                            ProcessStatus::Failed { error: e.to_string() }
                        }
                    }
                }
                _ = stop_notify_clone.notified() => {
                    let _ = child.start_kill();
                    let res = child.wait().await;
                    let code = res.ok().and_then(|s| s.code());
                    ProcessStatus::Stopped { exit_code: code, signal: Some("SIGTERM".to_string()) }
                }
                _ = kill_notify_clone.notified() => {
                    let _ = child.start_kill();
                    let _ = child.wait().await;
                    ProcessStatus::Killed
                }
            };

            let now = chrono::Utc::now().to_rfc3339();
            {
                let mut st = status_clone.write().await;
                *st = final_status.clone();
            }
            {
                let mut sp = stopped_at_clone.write().await;
                *sp = Some(now.clone());
            }
            {
                let mut buf = buffer_clone.write().await;
                buf.push(
                    OutputStream::System,
                    format!("Process terminated: {}", final_status.label()),
                );
            }
            exit_notify_clone.notify_waiters();
        });

        let managed = Arc::new(ManagedProcess {
            id: id.to_string(),
            name: config.name.clone(),
            command: config.command.clone(),
            cwd: config.cwd.clone(),
            env: config.env.clone(),
            pid,
            status,
            started_at_instant,
            started_at,
            stopped_at,
            buffer,
            stdin_tx: Arc::new(tokio::sync::Mutex::new(Some(stdin_tx))),
            stop_notify,
            kill_notify,
            exit_notify,
        });

        let info = managed.info().await;

        {
            let mut map = self.processes.write().await;
            map.insert(id.to_string(), managed);
        }

        Ok(info)
    }

    /// Looks up a managed process handle by ID or friendly name.
    pub async fn get_managed(&self, id_or_name: &str) -> Option<Arc<ManagedProcess>> {
        let map = self.processes.read().await;

        // Direct ID lookup
        if let Some(p) = map.get(id_or_name) {
            return Some(p.clone());
        }

        // Friendly name lookup
        for p in map.values() {
            if let Some(name) = &p.name {
                if name.eq_ignore_ascii_case(id_or_name) {
                    return Some(p.clone());
                }
            }
        }

        None
    }

    /// Returns process metadata by ID or friendly name.
    pub async fn get(&self, id_or_name: &str) -> Option<ProcessInfo> {
        let managed = self.get_managed(id_or_name).await?;
        Some(managed.info().await)
    }

    /// Lists all tracked background processes.
    pub async fn list(&self) -> Vec<ProcessInfo> {
        let map = self.processes.read().await;
        let mut list = Vec::with_capacity(map.len());
        for p in map.values() {
            list.push(p.info().await);
        }
        list.sort_by(|a, b| a.id.cmp(&b.id));
        list
    }

    /// Reads output logs for a process.
    pub async fn read_logs(
        &self,
        id_or_name: &str,
        tail: Option<usize>,
        offset: Option<usize>,
        limit: Option<usize>,
        filter: Option<&str>,
        stream_name: Option<&str>,
    ) -> anyhow::Result<LogOutput> {
        let proc = self
            .get_managed(id_or_name)
            .await
            .ok_or_else(|| anyhow::anyhow!("Process not found: '{id_or_name}'"))?;

        let stream_filter = match stream_name {
            Some("stdout") => Some(OutputStream::Stdout),
            Some("stderr") => Some(OutputStream::Stderr),
            Some("system") => Some(OutputStream::System),
            _ => None,
        };

        let status = proc.status.read().await.clone();
        let buf = proc.buffer.read().await;
        let (buffered_lines, total_lines, _) = buf.stats();
        let lines = buf.get_lines(tail, offset, limit, stream_filter, filter);
        let returned_lines = lines.len();

        Ok(LogOutput {
            id: proc.id.clone(),
            name: proc.name.clone(),
            status,
            lines,
            total_lines,
            buffered_lines,
            returned_lines,
        })
    }

    /// Stops a running process gracefully with a timeout, falling back to force-kill.
    pub async fn stop(
        &self,
        id_or_name: &str,
        timeout_secs: Option<u64>,
    ) -> anyhow::Result<ProcessInfo> {
        let proc = self
            .get_managed(id_or_name)
            .await
            .ok_or_else(|| anyhow::anyhow!("Process not found: '{id_or_name}'"))?;

        let is_running = proc.status.read().await.is_running();
        if !is_running {
            return Ok(proc.info().await);
        }

        let timeout = Duration::from_secs(timeout_secs.unwrap_or(5).max(1));

        // Attempt soft termination first
        #[cfg(unix)]
        if let Some(pid) = proc.pid {
            let _ = std::process::Command::new("kill")
                .arg("-15")
                .arg(pid.to_string())
                .status();
        }

        #[cfg(windows)]
        if let Some(pid) = proc.pid {
            let _ = std::process::Command::new("taskkill")
                .args(["/PID", &pid.to_string(), "/T"])
                .status();
        }

        proc.stop_notify.notify_one();

        // Wait for exit with timeout
        let wait_res = tokio::time::timeout(timeout, proc.exit_notify.notified()).await;
        if wait_res.is_err() {
            // Force kill if timed out
            proc.kill_notify.notify_one();

            #[cfg(unix)]
            if let Some(pid) = proc.pid {
                let _ = std::process::Command::new("kill")
                    .arg("-9")
                    .arg(pid.to_string())
                    .status();
            }

            #[cfg(windows)]
            if let Some(pid) = proc.pid {
                let _ = std::process::Command::new("taskkill")
                    .args(["/PID", &pid.to_string(), "/T", "/F"])
                    .status();
            }

            // Brief wait for forced termination
            let _ =
                tokio::time::timeout(Duration::from_millis(300), proc.exit_notify.notified()).await;
        }

        Ok(proc.info().await)
    }

    /// Forcefully kills a process immediately.
    pub async fn kill(&self, id_or_name: &str) -> anyhow::Result<ProcessInfo> {
        let proc = self
            .get_managed(id_or_name)
            .await
            .ok_or_else(|| anyhow::anyhow!("Process not found: '{id_or_name}'"))?;

        let is_running = proc.status.read().await.is_running();
        if !is_running {
            return Ok(proc.info().await);
        }

        proc.kill_notify.notify_one();

        #[cfg(unix)]
        if let Some(pid) = proc.pid {
            let _ = std::process::Command::new("kill")
                .arg("-9")
                .arg(pid.to_string())
                .status();
        }

        #[cfg(windows)]
        if let Some(pid) = proc.pid {
            let _ = std::process::Command::new("taskkill")
                .args(["/PID", &pid.to_string(), "/T", "/F"])
                .status();
        }

        let _ = tokio::time::timeout(Duration::from_millis(500), proc.exit_notify.notified()).await;

        Ok(proc.info().await)
    }

    /// Restarts a background process with the same configuration.
    pub async fn restart(&self, id_or_name: &str) -> anyhow::Result<ProcessInfo> {
        let proc = self
            .get_managed(id_or_name)
            .await
            .ok_or_else(|| anyhow::anyhow!("Process not found: '{id_or_name}'"))?;

        let cmd = proc.command.clone();
        let cwd = proc.cwd.clone();
        let env = proc.env.clone();
        let name = proc.name.clone();
        let id = proc.id.clone();
        let max_lines = proc.buffer.read().await.max_lines();

        if proc.status.read().await.is_running() {
            let _ = self.stop(&id, Some(3)).await;
        }

        let config = ProcessConfig::new(cmd)
            .with_cwd(cwd)
            .with_env(env)
            .with_max_buffer_lines(max_lines);

        let config = if let Some(n) = name {
            config.with_name(n)
        } else {
            config
        };

        self.spawn_internal(&id, config).await
    }

    /// Sends input text to the standard input of a running background process.
    pub async fn send_input(&self, id_or_name: &str, input: &str) -> anyhow::Result<()> {
        let proc = self
            .get_managed(id_or_name)
            .await
            .ok_or_else(|| anyhow::anyhow!("Process not found: '{id_or_name}'"))?;

        let is_running = proc.status.read().await.is_running();
        if !is_running {
            anyhow::bail!(
                "Cannot send input to process '{}' because it is not running (status: {})",
                proc.id,
                proc.status.read().await.label()
            );
        }

        let tx_guard = proc.stdin_tx.lock().await;
        if let Some(tx) = tx_guard.as_ref() {
            let payload = if input.ends_with('\n') {
                input.to_string()
            } else {
                format!("{input}\n")
            };

            tx.send(payload)
                .await
                .map_err(|_| anyhow::anyhow!("Process stdin stream is closed"))?;

            let mut buf = proc.buffer.write().await;
            buf.push(
                OutputStream::System,
                format!("Input sent: {}", input.trim_end()),
            );
            Ok(())
        } else {
            anyhow::bail!("Process stdin stream is not available");
        }
    }

    /// Clears completed or stopped processes from manager memory.
    pub async fn clear_completed(&self) -> usize {
        let mut map = self.processes.write().await;
        let mut to_remove = Vec::new();

        for (id, p) in map.iter() {
            if let Ok(st) = p.status.try_read() {
                if !st.is_running() {
                    to_remove.push(id.clone());
                }
            }
        }

        let count = to_remove.len();
        for id in to_remove {
            map.remove(&id);
        }
        count
    }

    /// Removes a process by ID or friendly name, stopping it first if running.
    pub async fn remove(&self, id_or_name: &str) -> anyhow::Result<ProcessInfo> {
        let proc = self
            .get_managed(id_or_name)
            .await
            .ok_or_else(|| anyhow::anyhow!("Process not found: '{id_or_name}'"))?;

        let id = proc.id.clone();
        if proc.status.read().await.is_running() {
            let _ = self.stop(&id, Some(2)).await;
        }

        let info = proc.info().await;
        let mut map = self.processes.write().await;
        map.remove(&id);
        Ok(info)
    }

    /// Stops all running background processes.
    pub async fn stop_all(
        &self,
        timeout_secs: Option<u64>,
    ) -> Vec<(String, Result<ProcessInfo, String>)> {
        let ids: Vec<String> = {
            let map = self.processes.read().await;
            map.keys().cloned().collect()
        };

        let mut results = Vec::new();
        for id in ids {
            let res = self
                .stop(&id, timeout_secs)
                .await
                .map_err(|e| e.to_string());
            results.push((id, res));
        }
        results
    }

    /// Forcefully kills all running background processes.
    pub async fn kill_all(&self) -> Vec<(String, Result<ProcessInfo, String>)> {
        let ids: Vec<String> = {
            let map = self.processes.read().await;
            map.keys().cloned().collect()
        };

        let mut results = Vec::new();
        for id in ids {
            let res = self.kill(&id).await.map_err(|e| e.to_string());
            results.push((id, res));
        }
        results
    }

    /// Returns the number of currently active/running processes.
    pub async fn active_count(&self) -> usize {
        let map = self.processes.read().await;
        let mut count = 0;
        for p in map.values() {
            if p.status.read().await.is_running() {
                count += 1;
            }
        }
        count
    }

    /// Returns the total number of tracked processes.
    pub async fn total_count(&self) -> usize {
        let map = self.processes.read().await;
        map.len()
    }
}

/// Global singleton instance of `ProcessManager`.
pub static GLOBAL_PROCESS_MANAGER: LazyLock<ProcessManager> = LazyLock::new(ProcessManager::new);

/// Returns a reference to the global `ProcessManager`.
pub fn global_process_manager() -> &'static ProcessManager {
    &GLOBAL_PROCESS_MANAGER
}

// ---------------------------------------------------------------------------
// ProcessTool (Tool Implementation)
// ---------------------------------------------------------------------------

/// Tool for managing long-running background processes (e.g. dev servers, build watchers, test runners).
#[derive(Default, Debug, Clone)]
pub struct ProcessTool;

impl ProcessTool {
    /// Creates a new `ProcessTool`.
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl Tool for ProcessTool {
    fn name(&self) -> &str {
        "process"
    }

    fn description(&self) -> &str {
        "Manage long-running background processes (e.g. dev servers, test watchers, daemons). Spawn, list, tail logs, send stdin input, stop, kill, and restart background processes."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "action": {
                    "type": "string",
                    "enum": [
                        "spawn", "start", "run",
                        "list", "ps", "status_all",
                        "logs", "output", "tail", "read",
                        "info", "status", "get",
                        "stop",
                        "kill",
                        "restart",
                        "send", "input", "write", "stdin",
                        "clear", "clean", "prune",
                        "remove", "delete",
                        "stop_all",
                        "kill_all"
                    ],
                    "description": "The action to perform: 'spawn'/'start' (launch background command), 'list'/'ps' (list all processes), 'logs'/'tail' (read output buffer), 'info'/'status' (inspect process metadata), 'stop' (gracefully terminate), 'kill' (force-terminate), 'restart' (re-launch process), 'send'/'input' (write to stdin), 'clear' (prune completed processes), 'remove' (stop and delete process), 'stop_all', 'kill_all'."
                },
                "command": {
                    "type": "string",
                    "description": "The shell command to spawn (required for 'spawn'/'start')."
                },
                "id": {
                    "type": "string",
                    "description": "The process ID (e.g. 'proc_1') or friendly name to target."
                },
                "name": {
                    "type": "string",
                    "description": "Optional human-readable label/name when spawning a process (e.g. 'dev-server')."
                },
                "cwd": {
                    "type": "string",
                    "description": "Working directory for the spawned process (relative to workspace or absolute)."
                },
                "env": {
                    "type": "object",
                    "description": "Optional key-value map of environment variables to set."
                },
                "lines": {
                    "type": "integer",
                    "description": "Number of log lines to retrieve (for 'logs'/'tail', default: 50)."
                },
                "tail": {
                    "type": "integer",
                    "description": "Alias for 'lines': number of lines to tail from log output."
                },
                "offset": {
                    "type": "integer",
                    "description": "Line offset for pagination in logs."
                },
                "limit": {
                    "type": "integer",
                    "description": "Maximum number of lines to return in logs."
                },
                "filter": {
                    "type": "string",
                    "description": "Substring or regex pattern to filter log lines."
                },
                "stream": {
                    "type": "string",
                    "enum": ["all", "stdout", "stderr", "system"],
                    "description": "Stream to read from in logs ('all', 'stdout', 'stderr', or 'system', default: 'all')."
                },
                "input": {
                    "type": "string",
                    "description": "Text to write to the process's standard input (for 'send'/'input')."
                },
                "timeout": {
                    "type": "integer",
                    "description": "Graceful shutdown timeout in seconds for 'stop'/'stop_all' (default: 5)."
                }
            }
        })
    }

    async fn execute(&self, args: Value, ctx: &ToolContext) -> anyhow::Result<String> {
        let mgr = global_process_manager();

        let action = args
            .get("action")
            .and_then(|v| v.as_str())
            .unwrap_or_else(|| {
                if args.get("command").is_some() {
                    "spawn"
                } else if args.get("input").is_some() {
                    "send"
                } else if args.get("id").is_some() || args.get("name").is_some() {
                    if args.get("lines").is_some() || args.get("tail").is_some() {
                        "logs"
                    } else {
                        "info"
                    }
                } else {
                    "list"
                }
            });

        let id_param = args
            .get("id")
            .and_then(|v| v.as_str())
            .or_else(|| args.get("process_id").and_then(|v| v.as_str()))
            .or_else(|| args.get("name").and_then(|v| v.as_str()));

        match action {
            "spawn" | "start" | "run" => {
                let command = args
                    .get("command")
                    .and_then(|v| v.as_str())
                    .or_else(|| args.get("cmd").and_then(|v| v.as_str()))
                    .ok_or_else(|| anyhow::anyhow!("Missing required parameter: 'command'"))?;

                let name = args.get("name").and_then(|v| v.as_str());

                let cwd_str = args
                    .get("cwd")
                    .and_then(|v| v.as_str())
                    .or_else(|| args.get("working_dir").and_then(|v| v.as_str()));

                let working_dir = match cwd_str {
                    Some(p) => resolve_path(p, &ctx.cwd),
                    None => ctx.cwd.clone(),
                };

                let env = args.get("env").and_then(|v| v.as_object()).map(|obj| {
                    obj.iter()
                        .filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), s.to_string())))
                        .collect::<HashMap<String, String>>()
                });

                let info = mgr.spawn(command, Some(&working_dir), env, name).await?;

                let name_line = if let Some(n) = &info.name {
                    format!("  Name:        {}\n", n)
                } else {
                    String::new()
                };

                let pid_str = info
                    .pid
                    .map(|p| p.to_string())
                    .unwrap_or_else(|| "N/A".to_string());

                Ok(format!(
                    "Background process spawned successfully:\n  ID:          {}\n{}  PID:         {}\n  Command:     {}\n  Working Dir: {}\n  Status:      {}\n  Started At:  {}",
                    info.id,
                    name_line,
                    pid_str,
                    info.command,
                    info.cwd,
                    info.status.label(),
                    info.started_at
                ))
            }

            "list" | "ps" | "status_all" => {
                let list = mgr.list().await;
                if list.is_empty() {
                    return Ok("No background processes are currently tracked.".to_string());
                }

                let running_count = list.iter().filter(|p| p.status.is_running()).count();
                let stopped_count = list.len() - running_count;

                let mut out = format!(
                    "Background Processes ({} running, {} stopped/exited):\n\n",
                    running_count, stopped_count
                );

                for p in list {
                    out.push_str(&format!("• {}\n", p.format_summary()));
                }

                Ok(out)
            }

            "logs" | "output" | "tail" | "read" => {
                let target_id = id_param.ok_or_else(|| {
                    anyhow::anyhow!("Missing required parameter 'id' (or 'name') for logs")
                })?;

                let lines_count = args
                    .get("lines")
                    .and_then(|v| v.as_u64())
                    .or_else(|| args.get("tail").and_then(|v| v.as_u64()))
                    .map(|n| n as usize);

                let offset = args
                    .get("offset")
                    .and_then(|v| v.as_u64())
                    .map(|n| n as usize);

                let limit = args
                    .get("limit")
                    .and_then(|v| v.as_u64())
                    .map(|n| n as usize);

                let filter = args
                    .get("filter")
                    .and_then(|v| v.as_str())
                    .or_else(|| args.get("grep").and_then(|v| v.as_str()));

                let stream = args.get("stream").and_then(|v| v.as_str());

                let tail = if offset.is_none() && limit.is_none() {
                    Some(lines_count.unwrap_or(50))
                } else {
                    None
                };

                let logs = mgr
                    .read_logs(target_id, tail, offset, limit, filter, stream)
                    .await?;

                Ok(logs.format_text(true))
            }

            "info" | "status" | "get" => {
                let target_id = id_param.ok_or_else(|| {
                    anyhow::anyhow!("Missing required parameter 'id' (or 'name') for info")
                })?;

                let info = mgr
                    .get(target_id)
                    .await
                    .ok_or_else(|| anyhow::anyhow!("Process not found: '{target_id}'"))?;

                Ok(info.format_detailed())
            }

            "stop" => {
                let target_id = id_param.ok_or_else(|| {
                    anyhow::anyhow!("Missing required parameter 'id' (or 'name') for stop")
                })?;

                let timeout = args.get("timeout").and_then(|v| v.as_u64());

                let info = mgr.stop(target_id, timeout).await?;

                Ok(format!(
                    "Process '{}' stopped.\nStatus:    {}\nExit Code: {:?}",
                    info.id,
                    info.status.label(),
                    info.exit_code
                ))
            }

            "kill" => {
                let target_id = id_param.ok_or_else(|| {
                    anyhow::anyhow!("Missing required parameter 'id' (or 'name') for kill")
                })?;

                let info = mgr.kill(target_id).await?;

                Ok(format!(
                    "Process '{}' killed.\nStatus: {}",
                    info.id,
                    info.status.label()
                ))
            }

            "restart" => {
                let target_id = id_param.ok_or_else(|| {
                    anyhow::anyhow!("Missing required parameter 'id' (or 'name') for restart")
                })?;

                let info = mgr.restart(target_id).await?;

                let pid_str = info
                    .pid
                    .map(|p| p.to_string())
                    .unwrap_or_else(|| "N/A".to_string());

                Ok(format!(
                    "Process '{}' restarted successfully.\nPID:    {}\nStatus: {}",
                    info.id,
                    pid_str,
                    info.status.label()
                ))
            }

            "send" | "input" | "write" | "stdin" => {
                let target_id = id_param.ok_or_else(|| {
                    anyhow::anyhow!("Missing required parameter 'id' (or 'name') for send")
                })?;

                let input = args
                    .get("input")
                    .and_then(|v| v.as_str())
                    .or_else(|| args.get("text").and_then(|v| v.as_str()))
                    .ok_or_else(|| anyhow::anyhow!("Missing required parameter: 'input'"))?;

                mgr.send_input(target_id, input).await?;

                Ok(format!(
                    "Input sent to process '{target_id}': {:?}",
                    input.trim_end()
                ))
            }

            "clear" | "clean" | "prune" => {
                let count = mgr.clear_completed().await;
                Ok(format!(
                    "Cleared {count} completed/stopped background processes."
                ))
            }

            "remove" | "delete" => {
                let target_id = id_param.ok_or_else(|| {
                    anyhow::anyhow!("Missing required parameter 'id' (or 'name') for remove")
                })?;

                let info = mgr.remove(target_id).await?;

                Ok(format!(
                    "Process '{}' ({}) removed from manager.",
                    info.id, info.command
                ))
            }

            "stop_all" => {
                let timeout = args.get("timeout").and_then(|v| v.as_u64());
                let results = mgr.stop_all(timeout).await;

                if results.is_empty() {
                    return Ok("No background processes to stop.".to_string());
                }

                let mut out = format!("Stopped {} background processes:\n", results.len());
                for (id, res) in results {
                    match res {
                        Ok(info) => {
                            out.push_str(&format!("• [{}] status: {}\n", id, info.status.label()));
                        }
                        Err(e) => {
                            out.push_str(&format!("• [{}] error: {}\n", id, e));
                        }
                    }
                }
                Ok(out)
            }

            "kill_all" => {
                let results = mgr.kill_all().await;

                if results.is_empty() {
                    return Ok("No background processes to kill.".to_string());
                }

                let mut out = format!("Killed {} background processes:\n", results.len());
                for (id, res) in results {
                    match res {
                        Ok(info) => {
                            out.push_str(&format!("• [{}] status: {}\n", id, info.status.label()));
                        }
                        Err(e) => {
                            out.push_str(&format!("• [{}] error: {}\n", id, e));
                        }
                    }
                }
                Ok(out)
            }

            other => {
                anyhow::bail!(
                    "Unknown action '{other}'. Valid actions: spawn, list, logs, info, stop, kill, restart, send, clear, remove, stop_all, kill_all"
                );
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Unit Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn test_output_stream_display() {
        assert_eq!(OutputStream::Stdout.to_string(), "stdout");
        assert_eq!(OutputStream::Stderr.to_string(), "stderr");
        assert_eq!(OutputStream::System.to_string(), "system");
    }

    #[test]
    fn test_output_buffer_push_and_capacity() {
        let buf = OutputBuffer::new(5);
        assert_eq!(buf.max_lines(), 10); // Minimum 10 enforcement

        let mut buf2 = OutputBuffer::new(10);
        for i in 1..=15 {
            buf2.push(OutputStream::Stdout, format!("Line {i}"));
        }

        let (count, total_lines, _) = buf2.stats();
        assert_eq!(count, 10);
        assert_eq!(total_lines, 15);

        let lines = buf2.get_lines(None, None, None, None, None);
        assert_eq!(lines.len(), 10);
        assert_eq!(lines[0].line, "Line 6");
        assert_eq!(lines[9].line, "Line 15");
    }

    #[test]
    fn test_output_buffer_filtering() {
        let mut buf = OutputBuffer::new(20);
        buf.push(OutputStream::Stdout, "info: ready on port 3000".to_string());
        buf.push(
            OutputStream::Stderr,
            "warn: deprecated API used".to_string(),
        );
        buf.push(
            OutputStream::Stdout,
            "info: connection accepted".to_string(),
        );
        buf.push(OutputStream::System, "Process started".to_string());

        let stdout_lines = buf.get_lines(None, None, None, Some(OutputStream::Stdout), None);
        assert_eq!(stdout_lines.len(), 2);
        assert_eq!(stdout_lines[0].line, "info: ready on port 3000");

        let stderr_lines = buf.get_lines(None, None, None, Some(OutputStream::Stderr), None);
        assert_eq!(stderr_lines.len(), 1);
        assert_eq!(stderr_lines[0].line, "warn: deprecated API used");

        let filtered = buf.get_lines(None, None, None, None, Some("connection"));
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].line, "info: connection accepted");

        let tail_lines = buf.get_lines(Some(2), None, None, None, None);
        assert_eq!(tail_lines.len(), 2);
        assert_eq!(tail_lines[0].line, "info: connection accepted");
        assert_eq!(tail_lines[1].line, "Process started");
    }

    #[test]
    fn test_output_buffer_combined_and_clear() {
        let mut buf = OutputBuffer::new(10);
        buf.push(OutputStream::Stdout, "hello".to_string());
        buf.push(OutputStream::Stderr, "world".to_string());

        assert_eq!(buf.combined_text(None), "hello\nworld");
        assert_eq!(buf.stdout_text(None), "hello");
        assert_eq!(buf.stderr_text(None), "world");

        buf.clear();
        assert_eq!(buf.stats().0, 0);
        assert_eq!(buf.combined_text(None), "");
    }

    #[test]
    fn test_process_status_methods() {
        let running = ProcessStatus::Running { pid: 1234 };
        assert!(running.is_running());
        assert_eq!(running.pid(), Some(1234));
        assert_eq!(running.exit_code(), None);
        assert_eq!(running.label(), "running");

        let exited = ProcessStatus::Exited {
            exit_code: Some(0),
            success: true,
        };
        assert!(!exited.is_running());
        assert_eq!(exited.pid(), None);
        assert_eq!(exited.exit_code(), Some(0));
        assert_eq!(exited.label(), "exited (success)");

        let stopped = ProcessStatus::Stopped {
            exit_code: None,
            signal: Some("SIGTERM".to_string()),
        };
        assert_eq!(stopped.label(), "stopped");

        let killed = ProcessStatus::Killed;
        assert_eq!(killed.label(), "killed");
    }

    #[test]
    fn test_process_config_builder() {
        let cfg = ProcessConfig::new("echo hello")
            .with_cwd("/tmp")
            .with_name("echo-test")
            .with_max_buffer_lines(500);

        assert_eq!(cfg.command, "echo hello");
        assert_eq!(cfg.cwd, PathBuf::from("/tmp"));
        assert_eq!(cfg.name, Some("echo-test".to_string()));
        assert_eq!(cfg.max_buffer_lines, 500);
    }

    #[tokio::test]
    async fn test_process_manager_spawn_and_exit() {
        let mgr = ProcessManager::new();

        // Spawn a command that prints output and exits
        #[cfg(windows)]
        let cmd = "cmd /C echo hello_fusion";
        #[cfg(not(windows))]
        let cmd = "echo hello_fusion";

        let info = mgr.spawn(cmd, None, None, Some("echo-test")).await.unwrap();
        assert_eq!(info.command, cmd);
        assert_eq!(info.name, Some("echo-test".to_string()));

        // Allow child to run and exit
        tokio::time::sleep(Duration::from_millis(600)).await;

        let updated = mgr.get(&info.id).await.unwrap();
        assert!(!updated.status.is_running());

        let logs = mgr
            .read_logs(&info.id, None, None, None, None, None)
            .await
            .unwrap();
        assert!(!logs.lines.is_empty());
        let stdout_has_hello = logs.lines.iter().any(|l| l.line.contains("hello_fusion"));
        assert!(stdout_has_hello, "Expected logs to contain 'hello_fusion'");
    }

    #[tokio::test]
    async fn test_process_manager_stop_and_kill() {
        let mgr = ProcessManager::new();

        // Spawn a long-running process (e.g. sleep)
        #[cfg(windows)]
        let cmd = "powershell -Command Start-Sleep -Seconds 60";
        #[cfg(not(windows))]
        let cmd = "sleep 60";

        let info = mgr.spawn(cmd, None, None, Some("sleep-job")).await.unwrap();
        assert!(info.status.is_running() || info.pid.is_some());

        // Stop it
        let stopped_info = mgr.stop(&info.id, Some(2)).await.unwrap();
        assert!(!stopped_info.status.is_running());

        // Test kill on another process
        let info2 = mgr.spawn(cmd, None, None, None).await.unwrap();
        let killed_info = mgr.kill(&info2.id).await.unwrap();
        assert!(!killed_info.status.is_running());
    }

    #[tokio::test]
    async fn test_process_manager_list_and_clear() {
        let mgr = ProcessManager::new();

        #[cfg(windows)]
        let cmd = "cmd /C echo quick_exit";
        #[cfg(not(windows))]
        let cmd = "echo quick_exit";

        let _info1 = mgr.spawn(cmd, None, None, Some("p1")).await.unwrap();
        let _info2 = mgr.spawn(cmd, None, None, Some("p2")).await.unwrap();

        let list = mgr.list().await;
        assert_eq!(list.len(), 2);

        // Wait for exit
        tokio::time::sleep(Duration::from_millis(600)).await;

        let cleared = mgr.clear_completed().await;
        assert_eq!(cleared, 2);

        let list_after = mgr.list().await;
        assert_eq!(list_after.len(), 0);
    }

    #[tokio::test]
    async fn test_process_tool_execution() {
        let tool = ProcessTool::new();
        let ctx = ToolContext::default();

        assert_eq!(tool.name(), "process");
        assert!(!tool.description().is_empty());

        // List initially empty or contains existing
        let list_res = tool
            .execute(json!({ "action": "list" }), &ctx)
            .await
            .unwrap();
        assert!(!list_res.is_empty());

        // Spawn a process
        #[cfg(windows)]
        let cmd = "cmd /C echo tool_spawn_test";
        #[cfg(not(windows))]
        let cmd = "echo tool_spawn_test";

        let spawn_res = tool
            .execute(
                json!({
                    "action": "spawn",
                    "command": cmd,
                    "name": "tool-proc"
                }),
                &ctx,
            )
            .await
            .unwrap();

        assert!(spawn_res.contains("Background process spawned successfully"));
        assert!(spawn_res.contains("tool-proc"));

        // Wait a bit for execution
        tokio::time::sleep(Duration::from_millis(600)).await;

        // Fetch logs by name
        let logs_res = tool
            .execute(
                json!({
                    "action": "logs",
                    "id": "tool-proc"
                }),
                &ctx,
            )
            .await
            .unwrap();

        assert!(logs_res.contains("tool_spawn_test"));

        // Fetch info
        let info_res = tool
            .execute(
                json!({
                    "action": "info",
                    "id": "tool-proc"
                }),
                &ctx,
            )
            .await
            .unwrap();

        assert!(info_res.contains("Process ID:"));
        assert!(info_res.contains("tool-proc"));

        // Clear completed
        let clear_res = tool
            .execute(json!({ "action": "clear" }), &ctx)
            .await
            .unwrap();
        assert!(clear_res.contains("Cleared"));
    }

    #[tokio::test]
    async fn test_process_tool_invalid_args() {
        let tool = ProcessTool::new();
        let ctx = ToolContext::default();

        // Spawn without command
        let err = tool.execute(json!({ "action": "spawn" }), &ctx).await;
        assert!(err.is_err());

        // Logs without ID
        let err = tool.execute(json!({ "action": "logs" }), &ctx).await;
        assert!(err.is_err());

        // Stop non-existent process
        let err = tool
            .execute(
                json!({ "action": "stop", "id": "proc_nonexistent_9999" }),
                &ctx,
            )
            .await;
        assert!(err.is_err());
    }
}
