//! Local multi-process peer-to-peer agent mesh network over Unix domain sockets.
//!
//! Provides inter-process discovery, heartbeat health checks, task delegation,
//! and broadcast messaging for concurrent local `fusion` instances.
//!
//! ## Architecture
//!
//! - **Transport**: Unix domain sockets on Unix platforms (`~/.fusion/run/mesh_<pid>.sock`),
//!   with automatic TCP localhost fallback on Windows or when explicitly configured.
//! - **Peer Discovery**: Scans the local runtime run directory for active socket/port files,
//!   pings candidate nodes, and establishes peer identities.
//! - **Peer Registry**: Thread-safe registry of active local peers with 15-second TTL expiry.
//! - **Messaging Protocol**: Length/newline-delimited JSON messages (`MeshMessage`) supporting
//!   pings, task offers, task acceptances, and execution results.

use std::collections::HashMap;
use std::fmt;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use thiserror::Error;
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, BufReader};
use tokio::sync::{broadcast, watch, RwLock};
use uuid::Uuid;

#[cfg(unix)]
use std::os::unix::fs::FileTypeExt;
#[cfg(unix)]
use tokio::net::{UnixListener, UnixStream};

use tokio::net::{TcpListener, TcpStream};

/// Default time-to-live for active peer discovery entries (15 seconds).
pub const DEFAULT_PEER_TTL: Duration = Duration::from_secs(15);

/// Default capacity for incoming broadcast message channels.
pub const DEFAULT_MESSAGE_CHANNEL_CAPACITY: usize = 256;

/// Default timeout for ping / query network requests.
pub const DEFAULT_REQUEST_TIMEOUT: Duration = Duration::from_secs(5);

/// Counter for generating unique simulated PIDs in testing environments.
static TEST_PID_COUNTER: AtomicU32 = AtomicU32::new(70000);

// ============================================================================
// Errors
// ============================================================================

/// Errors that can occur within the mesh network layer.
#[derive(Debug, Error)]
pub enum MeshNetworkError {
    #[error("Peer '{0}' not found in mesh registry")]
    PeerNotFound(String),

    #[error("Peer '{0}' is expired or inactive")]
    PeerExpired(String),

    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Serialization error: {0}")]
    Serialization(#[from] serde_json::Error),

    #[error("Request timed out after {0:?}")]
    Timeout(Duration),

    #[error("Unexpected response: expected {expected}, got {got}")]
    UnexpectedResponse { expected: String, got: String },

    #[error("Connection closed unexpectedly")]
    ConnectionClosed,

    #[error("Socket listener bind error: {0}")]
    BindError(String),

    #[error("Transport error: {0}")]
    Transport(String),

    #[error("Internal mesh network error: {0}")]
    Other(String),
}

// ============================================================================
// Peer Identifier & Status
// ============================================================================

/// Unique UUID string identifying an autonomous agent peer on the local mesh.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, PartialOrd, Ord)]
#[serde(transparent)]
pub struct PeerId(pub String);

impl PeerId {
    /// Generates a new random UUID v4 peer identifier.
    pub fn new() -> Self {
        Self(Uuid::new_v4().to_string())
    }

    /// Returns the string slice representation of this peer ID.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl Default for PeerId {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Display for PeerId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl From<String> for PeerId {
    fn from(s: String) -> Self {
        Self(s)
    }
}

impl From<&str> for PeerId {
    fn from(s: &str) -> Self {
        Self(s.to_string())
    }
}

impl From<Uuid> for PeerId {
    fn from(u: Uuid) -> Self {
        Self(u.to_string())
    }
}

impl From<PeerId> for String {
    fn from(p: PeerId) -> Self {
        p.0
    }
}

impl std::ops::Deref for PeerId {
    type Target = str;
    fn deref(&self) -> &str {
        &self.0
    }
}

impl AsRef<str> for PeerId {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

impl std::borrow::Borrow<str> for PeerId {
    fn borrow(&self) -> &str {
        &self.0
    }
}

/// Operational state of a local agent node.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum PeerStatus {
    /// Agent is idle and available for task delegation or queries.
    #[default]
    #[serde(alias = "Idle", alias = "idle", alias = "IDLE")]
    Idle,
    /// Agent is actively processing tasks or executing tools.
    #[serde(alias = "Busy", alias = "busy", alias = "BUSY")]
    Busy,
    /// Agent is in an error or degraded state.
    #[serde(alias = "Error", alias = "error", alias = "ERROR")]
    Error,
}

impl PeerStatus {
    /// Returns true if the peer is ready to accept work.
    pub fn is_idle(&self) -> bool {
        matches!(self, PeerStatus::Idle)
    }

    /// Returns true if the peer is currently executing tasks.
    pub fn is_busy(&self) -> bool {
        matches!(self, PeerStatus::Busy)
    }

    /// Returns true if the peer has encountered an unrecovered error.
    pub fn is_error(&self) -> bool {
        matches!(self, PeerStatus::Error)
    }
}

impl fmt::Display for PeerStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PeerStatus::Idle => write!(f, "idle"),
            PeerStatus::Busy => write!(f, "busy"),
            PeerStatus::Error => write!(f, "error"),
        }
    }
}

// ============================================================================
// Mesh Messages
// ============================================================================

/// Messages exchanged across local fusion processes via Unix domain sockets or TCP.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum MeshMessage {
    /// Discovery ping sent to probe candidate local peer sockets.
    #[serde(alias = "Ping", alias = "ping")]
    Ping {
        peer_id: String,
    },

    /// Health and identity response confirming active peer status and model.
    #[serde(alias = "Pong", alias = "pong")]
    Pong {
        peer_id: String,
        active_model: String,
        status: PeerStatus,
    },

    /// Task delegation offer broadcast or targeted to an idle peer.
    #[serde(alias = "TaskOffer", alias = "task_offer")]
    TaskOffer {
        task_id: String,
        description: String,
        required_skills: Vec<String>,
    },

    /// Task acceptance confirmation returned by a worker peer.
    #[serde(alias = "TaskAccept", alias = "task_accept")]
    TaskAccept {
        task_id: String,
        worker_id: String,
    },

    /// Final outcome and output returned after task execution completes.
    #[serde(alias = "TaskResult", alias = "task_result")]
    TaskResult {
        task_id: String,
        worker_id: String,
        output: String,
        success: bool,
    },
}

impl MeshMessage {
    /// Creates a new `Ping` message.
    pub fn ping(peer_id: impl Into<String>) -> Self {
        Self::Ping {
            peer_id: peer_id.into(),
        }
    }

    /// Creates a new `Pong` message.
    pub fn pong(
        peer_id: impl Into<String>,
        active_model: impl Into<String>,
        status: PeerStatus,
    ) -> Self {
        Self::Pong {
            peer_id: peer_id.into(),
            active_model: active_model.into(),
            status,
        }
    }

    /// Creates a new `TaskOffer` message.
    pub fn task_offer(
        task_id: impl Into<String>,
        description: impl Into<String>,
        required_skills: Vec<String>,
    ) -> Self {
        Self::TaskOffer {
            task_id: task_id.into(),
            description: description.into(),
            required_skills,
        }
    }

    /// Creates a new `TaskAccept` message.
    pub fn task_accept(task_id: impl Into<String>, worker_id: impl Into<String>) -> Self {
        Self::TaskAccept {
            task_id: task_id.into(),
            worker_id: worker_id.into(),
        }
    }

    /// Creates a new `TaskResult` message.
    pub fn task_result(
        task_id: impl Into<String>,
        worker_id: impl Into<String>,
        output: impl Into<String>,
        success: bool,
    ) -> Self {
        Self::TaskResult {
            task_id: task_id.into(),
            worker_id: worker_id.into(),
            output: output.into(),
            success,
        }
    }

    /// Returns a short human-readable name of the message variant.
    pub fn message_type(&self) -> &'static str {
        match self {
            MeshMessage::Ping { .. } => "ping",
            MeshMessage::Pong { .. } => "pong",
            MeshMessage::TaskOffer { .. } => "task_offer",
            MeshMessage::TaskAccept { .. } => "task_accept",
            MeshMessage::TaskResult { .. } => "task_result",
        }
    }
}

// ============================================================================
// Network Endpoints & Streaming Abstraction
// ============================================================================

/// Physical network endpoint addressing a local peer (Unix domain socket or TCP socket).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum PeerEndpoint {
    /// Unix domain socket file path.
    Unix(PathBuf),
    /// Localhost TCP socket address.
    Tcp(SocketAddr),
}

impl PeerEndpoint {
    /// Connects to this endpoint asynchronously.
    pub async fn connect(&self) -> std::io::Result<NetworkStream> {
        match self {
            #[cfg(unix)]
            PeerEndpoint::Unix(path) => {
                let stream = UnixStream::connect(path).await?;
                Ok(NetworkStream::Unix(stream))
            }
            #[cfg(not(unix))]
            PeerEndpoint::Unix(_) => Err(std::io::Error::new(
                std::io::ErrorKind::Unsupported,
                "Unix domain sockets not supported on this platform",
            )),
            PeerEndpoint::Tcp(addr) => {
                let stream = TcpStream::connect(addr).await?;
                Ok(NetworkStream::Tcp(stream))
            }
        }
    }

    /// Inspects a path in `run_dir` to determine whether it is a Unix socket or a TCP port spec.
    pub fn from_file(path: &Path) -> Result<Self, MeshNetworkError> {
        #[cfg(unix)]
        {
            if let Ok(meta) = std::fs::symlink_metadata(path) {
                if meta.file_type().is_socket() {
                    return Ok(PeerEndpoint::Unix(path.to_path_buf()));
                }
            }
        }

        // If regular file, check if it specifies a TCP port (e.g. "tcp:127.0.0.1:54321" or "127.0.0.1:54321")
        if let Ok(content) = std::fs::read_to_string(path) {
            let trimmed = content.trim();
            let addr_str = trimmed
                .strip_prefix("tcp://")
                .or_else(|| trimmed.strip_prefix("tcp:"))
                .unwrap_or(trimmed);

            if let Ok(addr) = addr_str.parse::<SocketAddr>() {
                return Ok(PeerEndpoint::Tcp(addr));
            }
        }

        #[cfg(unix)]
        return Ok(PeerEndpoint::Unix(path.to_path_buf()));

        #[cfg(not(unix))]
        Err(MeshNetworkError::Transport(format!(
            "Could not parse valid socket address from {}",
            path.display()
        )))
    }
}

impl fmt::Display for PeerEndpoint {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PeerEndpoint::Unix(path) => write!(f, "unix:{}", path.display()),
            PeerEndpoint::Tcp(addr) => write!(f, "tcp:{}", addr),
        }
    }
}

/// Unified async stream wrapping either a Unix domain socket or a TCP socket stream.
pub enum NetworkStream {
    #[cfg(unix)]
    Unix(UnixStream),
    Tcp(TcpStream),
}

impl AsyncRead for NetworkStream {
    fn poll_read(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &mut tokio::io::ReadBuf<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        match self.get_mut() {
            #[cfg(unix)]
            NetworkStream::Unix(s) => std::pin::Pin::new(s).poll_read(cx, buf),
            NetworkStream::Tcp(s) => std::pin::Pin::new(s).poll_read(cx, buf),
        }
    }
}

impl AsyncWrite for NetworkStream {
    fn poll_write(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &[u8],
    ) -> std::task::Poll<std::io::Result<usize>> {
        match self.get_mut() {
            #[cfg(unix)]
            NetworkStream::Unix(s) => std::pin::Pin::new(s).poll_write(cx, buf),
            NetworkStream::Tcp(s) => std::pin::Pin::new(s).poll_write(cx, buf),
        }
    }

    fn poll_flush(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        match self.get_mut() {
            #[cfg(unix)]
            NetworkStream::Unix(s) => std::pin::Pin::new(s).poll_flush(cx),
            NetworkStream::Tcp(s) => std::pin::Pin::new(s).poll_flush(cx),
        }
    }

    fn poll_shutdown(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        match self.get_mut() {
            #[cfg(unix)]
            NetworkStream::Unix(s) => std::pin::Pin::new(s).poll_shutdown(cx),
            NetworkStream::Tcp(s) => std::pin::Pin::new(s).poll_shutdown(cx),
        }
    }
}

/// Unified listener wrapping Unix domain socket listener or TCP listener.
pub enum NetworkListener {
    #[cfg(unix)]
    Unix(UnixListener),
    Tcp(TcpListener),
}

impl NetworkListener {
    /// Accepts an incoming connection.
    pub async fn accept(&self) -> std::io::Result<NetworkStream> {
        match self {
            #[cfg(unix)]
            NetworkListener::Unix(listener) => {
                let (stream, _) = listener.accept().await?;
                Ok(NetworkStream::Unix(stream))
            }
            NetworkListener::Tcp(listener) => {
                let (stream, _) = listener.accept().await?;
                Ok(NetworkStream::Tcp(stream))
            }
        }
    }
}

// ============================================================================
// Wire Framing Helpers
// ============================================================================

/// Serializes and writes a newline-delimited `MeshMessage` to an async writer.
pub async fn send_message<W: AsyncWriteExt + Unpin>(
    writer: &mut W,
    message: &MeshMessage,
) -> std::io::Result<()> {
    let mut payload = serde_json::to_vec(message)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
    payload.push(b'\n');
    writer.write_all(&payload).await?;
    writer.flush().await?;
    Ok(())
}

/// Reads a single newline-delimited `MeshMessage` from an async buffered reader.
pub async fn read_message<R: AsyncBufReadExt + Unpin>(
    reader: &mut R,
) -> std::io::Result<Option<MeshMessage>> {
    let mut line = String::new();
    let n = reader.read_line(&mut line).await?;
    if n == 0 {
        return Ok(None);
    }
    let trimmed = line.trim();
    if trimmed.is_empty() {
        return Ok(None);
    }
    let msg: MeshMessage = serde_json::from_str(trimmed).map_err(|e| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            format!("Invalid MeshMessage JSON: {e} (raw: {trimmed})"),
        )
    })?;
    Ok(Some(msg))
}

// ============================================================================
// Peer Registry & Peer Record
// ============================================================================

/// Information about an active peer registered in the local mesh.
#[derive(Debug, Clone)]
pub struct PeerRecord {
    /// Unique peer ID.
    pub peer_id: String,
    /// Model currently active on this peer.
    pub active_model: String,
    /// Current operational status.
    pub status: PeerStatus,
    /// Network connection endpoint.
    pub endpoint: PeerEndpoint,
    /// Operating system process ID of the peer.
    pub pid: Option<u32>,
    /// Timestamp when this peer was last observed or pinged.
    pub last_seen: Instant,
    /// Time-to-live for this peer record.
    pub ttl: Duration,
}

impl PeerRecord {
    /// Creates a new peer record with specified TTL.
    pub fn new(
        peer_id: impl Into<String>,
        active_model: impl Into<String>,
        status: PeerStatus,
        endpoint: PeerEndpoint,
        pid: Option<u32>,
        ttl: Duration,
    ) -> Self {
        Self {
            peer_id: peer_id.into(),
            active_model: active_model.into(),
            status,
            endpoint,
            pid,
            last_seen: Instant::now(),
            ttl,
        }
    }

    /// Returns true if this peer record has exceeded its TTL without being touched.
    pub fn is_expired(&self) -> bool {
        self.last_seen.elapsed() > self.ttl
    }

    /// Refreshes the last-seen timestamp to current time.
    pub fn touch(&mut self) {
        self.last_seen = Instant::now();
    }
}

/// Thread-safe registry maintaining known active local peers and enforcing TTL expiry.
#[derive(Debug, Clone)]
pub struct PeerRegistry {
    peers: Arc<RwLock<HashMap<String, PeerRecord>>>,
    default_ttl: Duration,
}

impl PeerRegistry {
    /// Creates an empty registry with specified default TTL.
    pub fn new(default_ttl: Duration) -> Self {
        Self {
            peers: Arc::new(RwLock::new(HashMap::new())),
            default_ttl,
        }
    }

    /// Registers or updates a peer record.
    pub async fn register(
        &self,
        peer_id: impl Into<String>,
        active_model: impl Into<String>,
        status: PeerStatus,
        endpoint: PeerEndpoint,
        pid: Option<u32>,
    ) {
        let peer_id = peer_id.into();
        let mut map = self.peers.write().await;
        if let Some(record) = map.get_mut(&peer_id) {
            record.active_model = active_model.into();
            record.status = status;
            record.endpoint = endpoint;
            record.pid = pid;
            record.touch();
        } else {
            let record = PeerRecord::new(
                peer_id.clone(),
                active_model,
                status,
                endpoint,
                pid,
                self.default_ttl,
            );
            map.insert(peer_id, record);
        }
    }

    /// Touches a peer's last seen timestamp to keep it alive.
    pub async fn touch(&self, peer_id: &str) -> bool {
        let mut map = self.peers.write().await;
        if let Some(record) = map.get_mut(peer_id) {
            record.touch();
            true
        } else {
            false
        }
    }

    /// Updates the status of a known peer.
    pub async fn update_status(&self, peer_id: &str, status: PeerStatus) -> bool {
        let mut map = self.peers.write().await;
        if let Some(record) = map.get_mut(peer_id) {
            record.status = status;
            record.touch();
            true
        } else {
            false
        }
    }

    /// Returns a snapshot of all active, non-expired peers.
    pub async fn active_peers(&self) -> Vec<PeerRecord> {
        let mut map = self.peers.write().await;
        map.retain(|_, record| !record.is_expired());
        map.values().cloned().collect()
    }

    /// Returns a specific peer by ID if present and unexpired.
    pub async fn get_peer(&self, peer_id: &str) -> Option<PeerRecord> {
        let mut map = self.peers.write().await;
        if let Some(record) = map.get(peer_id) {
            if record.is_expired() {
                map.remove(peer_id);
                None
            } else {
                Some(record.clone())
            }
        } else {
            None
        }
    }

    /// Removes expired peers from the registry, returning count of purged entries.
    pub async fn prune_expired(&self) -> usize {
        let mut map = self.peers.write().await;
        let before = map.len();
        map.retain(|_, record| !record.is_expired());
        before - map.len()
    }

    /// Returns the number of known active peers.
    pub async fn len(&self) -> usize {
        self.active_peers().await.len()
    }

    /// Returns true if no active peers are known.
    pub async fn is_empty(&self) -> bool {
        self.len().await == 0
    }

    /// Explicitly removes a peer from the registry.
    pub async fn remove(&self, peer_id: &str) -> Option<PeerRecord> {
        self.peers.write().await.remove(peer_id)
    }
}

// ============================================================================
// MeshNode Configuration & Builder
// ============================================================================

/// Configuration options for initializing a `MeshNode`.
#[derive(Debug, Clone)]
pub struct MeshNodeOptions {
    /// Explicit peer identifier (defaults to a fresh UUID v4).
    pub peer_id: Option<PeerId>,
    /// Explicit process identifier for socket naming (defaults to `std::process::id()`).
    pub pid: Option<u32>,
    /// Custom run directory for socket files (defaults to `~/.fusion/run`).
    pub run_dir: Option<PathBuf>,
    /// Active AI model name exposed during discovery (e.g. `claude-3-5-sonnet`).
    pub active_model: String,
    /// Initial operational status.
    pub status: PeerStatus,
    /// Force TCP localhost binding instead of Unix domain socket.
    pub force_tcp: bool,
    /// Automatically accept incoming `TaskOffer`s when in `Idle` state.
    pub auto_accept: bool,
    /// TTL for active peer records (defaults to 15 seconds).
    pub peer_ttl: Duration,
}

impl Default for MeshNodeOptions {
    fn default() -> Self {
        Self {
            peer_id: None,
            pid: None,
            run_dir: None,
            active_model: "default".to_string(),
            status: PeerStatus::Idle,
            force_tcp: false,
            auto_accept: false,
            peer_ttl: DEFAULT_PEER_TTL,
        }
    }
}

// ============================================================================
// MeshNode Implementation
// ============================================================================

struct MeshNodeInner {
    peer_id: PeerId,
    pid: u32,
    run_dir: PathBuf,
    socket_path: PathBuf,
    endpoint: PeerEndpoint,
    active_model: Arc<RwLock<String>>,
    status: Arc<RwLock<PeerStatus>>,
    auto_accept: Arc<RwLock<bool>>,
    registry: PeerRegistry,
    message_tx: broadcast::Sender<MeshMessage>,
    shutdown_tx: watch::Sender<bool>,
    _listener_handle: Option<tokio::task::JoinHandle<()>>,
}

impl Drop for MeshNodeInner {
    fn drop(&mut self) {
        let _ = self.shutdown_tx.send(true);
        if self.socket_path.exists() {
            let _ = std::fs::remove_file(&self.socket_path);
        }
    }
}

/// Local autonomous agent mesh node providing discovery, messaging, and task orchestration.
#[derive(Clone)]
pub struct MeshNode {
    inner: Arc<MeshNodeInner>,
}

impl MeshNode {
    /// Resolves the default runtime directory `~/.fusion/run`.
    pub fn default_run_dir() -> PathBuf {
        if let Some(home) = dirs::home_dir() {
            home.join(".fusion").join("run")
        } else {
            std::env::temp_dir().join(".fusion").join("run")
        }
    }

    /// Initializes and starts a new `MeshNode` with standard configuration in `~/.fusion/run`.
    pub async fn new(active_model: impl Into<String>) -> Result<Self, MeshNetworkError> {
        let options = MeshNodeOptions {
            active_model: active_model.into(),
            ..Default::default()
        };
        Self::with_options(options).await
    }

    /// Initializes and starts a `MeshNode` bound in a specific directory (useful for tests).
    pub async fn in_dir(
        run_dir: impl AsRef<Path>,
        active_model: impl Into<String>,
    ) -> Result<Self, MeshNetworkError> {
        let pid = TEST_PID_COUNTER.fetch_add(1, Ordering::Relaxed);
        Self::in_dir_with_pid(run_dir, pid, active_model).await
    }

    /// Initializes and starts a `MeshNode` bound in a specific directory with an explicit PID.
    pub async fn in_dir_with_pid(
        run_dir: impl AsRef<Path>,
        pid: u32,
        active_model: impl Into<String>,
    ) -> Result<Self, MeshNetworkError> {
        let options = MeshNodeOptions {
            run_dir: Some(run_dir.as_ref().to_path_buf()),
            pid: Some(pid),
            active_model: active_model.into(),
            ..Default::default()
        };
        Self::with_options(options).await
    }

    /// Initializes and starts a `MeshNode` with full configuration options.
    pub async fn with_options(options: MeshNodeOptions) -> Result<Self, MeshNetworkError> {
        let peer_id = options.peer_id.unwrap_or_else(PeerId::new);
        let pid = options.pid.unwrap_or_else(std::process::id);
        let run_dir = options.run_dir.unwrap_or_else(Self::default_run_dir);

        std::fs::create_dir_all(&run_dir)?;

        let socket_path = run_dir.join(format!("mesh_{pid}.sock"));

        // Determine transport: Unix domain socket on Unix unless force_tcp or non-Unix.
        #[cfg(unix)]
        let use_tcp = options.force_tcp;
        #[cfg(not(unix))]
        let use_tcp = true;

        let (listener, endpoint) = if use_tcp {
            let listener = TcpListener::bind("127.0.0.1:0").await?;
            let addr = listener.local_addr()?;
            // Write TCP port spec file so peers can discover us
            std::fs::write(&socket_path, format!("tcp:{}\n", addr))?;
            (NetworkListener::Tcp(listener), PeerEndpoint::Tcp(addr))
        } else {
            #[cfg(unix)]
            {
                // Clean up stale socket file if it exists and nothing is listening
                if socket_path.exists() {
                    if UnixStream::connect(&socket_path).await.is_err() {
                        let _ = std::fs::remove_file(&socket_path);
                    } else {
                        return Err(MeshNetworkError::BindError(format!(
                            "Socket {} already bound by another process",
                            socket_path.display()
                        )));
                    }
                }
                let listener = UnixListener::bind(&socket_path)?;
                (
                    NetworkListener::Unix(listener),
                    PeerEndpoint::Unix(socket_path.clone()),
                )
            }
            #[cfg(not(unix))]
            unreachable!()
        };

        let active_model = Arc::new(RwLock::new(options.active_model));
        let status = Arc::new(RwLock::new(options.status));
        let auto_accept = Arc::new(RwLock::new(options.auto_accept));
        let registry = PeerRegistry::new(options.peer_ttl);
        let (message_tx, _) = broadcast::channel(DEFAULT_MESSAGE_CHANNEL_CAPACITY);
        let (shutdown_tx, shutdown_rx) = watch::channel(false);

        // Spawn accept loop
        let listener_handle = {
            let peer_id_str = peer_id.to_string();
            let active_model = active_model.clone();
            let status = status.clone();
            let auto_accept = auto_accept.clone();
            let message_tx = message_tx.clone();
            let registry = registry.clone();

            tokio::spawn(async move {
                Self::run_accept_loop(
                    listener,
                    peer_id_str,
                    active_model,
                    status,
                    auto_accept,
                    message_tx,
                    registry,
                    shutdown_rx,
                )
                .await;
            })
        };

        let inner = MeshNodeInner {
            peer_id,
            pid,
            run_dir,
            socket_path,
            endpoint,
            active_model,
            status,
            auto_accept,
            registry,
            message_tx,
            shutdown_tx,
            _listener_handle: Some(listener_handle),
        };

        Ok(Self {
            inner: Arc::new(inner),
        })
    }

    /// Background listener accept loop dispatching incoming connections.
    #[allow(clippy::too_many_arguments)]
    async fn run_accept_loop(
        listener: NetworkListener,
        peer_id: String,
        active_model: Arc<RwLock<String>>,
        status: Arc<RwLock<PeerStatus>>,
        auto_accept: Arc<RwLock<bool>>,
        message_tx: broadcast::Sender<MeshMessage>,
        registry: PeerRegistry,
        mut shutdown_rx: watch::Receiver<bool>,
    ) {
        loop {
            tokio::select! {
                _ = shutdown_rx.changed() => {
                    if *shutdown_rx.borrow() {
                        break;
                    }
                }
                res = listener.accept() => {
                    match res {
                        Ok(stream) => {
                            let peer_id = peer_id.clone();
                            let active_model = active_model.clone();
                            let status = status.clone();
                            let auto_accept = auto_accept.clone();
                            let message_tx = message_tx.clone();
                            let registry = registry.clone();

                            tokio::spawn(async move {
                                Self::handle_connection(
                                    stream,
                                    peer_id,
                                    active_model,
                                    status,
                                    auto_accept,
                                    message_tx,
                                    registry,
                                ).await;
                            });
                        }
                        Err(_) => {
                            tokio::time::sleep(Duration::from_millis(50)).await;
                        }
                    }
                }
            }
        }
    }

    /// Handles a single incoming peer connection.
    async fn handle_connection(
        stream: NetworkStream,
        my_id: String,
        active_model: Arc<RwLock<String>>,
        status: Arc<RwLock<PeerStatus>>,
        auto_accept: Arc<RwLock<bool>>,
        message_tx: broadcast::Sender<MeshMessage>,
        registry: PeerRegistry,
    ) {
        let (reader, mut writer) = tokio::io::split(stream);
        let mut buf_reader = BufReader::new(reader);

        while let Ok(Some(msg)) = read_message(&mut buf_reader).await {
            match &msg {
                MeshMessage::Ping { peer_id: remote_id } => {
                    let model = active_model.read().await.clone();
                    let st = *status.read().await;
                    let pong = MeshMessage::Pong {
                        peer_id: my_id.clone(),
                        active_model: model,
                        status: st,
                    };
                    if send_message(&mut writer, &pong).await.is_err() {
                        break;
                    }
                    registry.touch(remote_id).await;
                    let _ = message_tx.send(msg);
                }
                MeshMessage::Pong {
                    peer_id: remote_id,
                    active_model: model,
                    status: st,
                } => {
                    registry.update_status(remote_id, *st).await;
                    if let Some(mut record) = registry.get_peer(remote_id).await {
                        record.active_model = model.clone();
                        record.status = *st;
                    }
                    let _ = message_tx.send(msg);
                }
                MeshMessage::TaskOffer {
                    task_id,
                    description: _,
                    required_skills: _,
                } => {
                    let is_auto = *auto_accept.read().await;
                    let cur_status = *status.read().await;

                    if is_auto && cur_status == PeerStatus::Idle {
                        *status.write().await = PeerStatus::Busy;
                        let accept = MeshMessage::TaskAccept {
                            task_id: task_id.clone(),
                            worker_id: my_id.clone(),
                        };
                        let _ = send_message(&mut writer, &accept).await;
                    }
                    let _ = message_tx.send(msg);
                }
                MeshMessage::TaskAccept {
                    task_id: _,
                    worker_id,
                } => {
                    registry.update_status(worker_id, PeerStatus::Busy).await;
                    let _ = message_tx.send(msg);
                }
                MeshMessage::TaskResult {
                    task_id: _,
                    worker_id,
                    output: _,
                    success: _,
                } => {
                    registry.update_status(worker_id, PeerStatus::Idle).await;
                    let _ = message_tx.send(msg);
                }
            }
        }
    }

    // ========================================================================
    // Property Accessors
    // ========================================================================

    /// Returns this node's unique peer ID.
    pub fn peer_id(&self) -> &str {
        self.inner.peer_id.as_str()
    }

    /// Returns a reference to the strongly-typed `PeerId`.
    pub fn peer_id_typed(&self) -> &PeerId {
        &self.inner.peer_id
    }

    /// Returns the OS process ID associated with this node.
    pub fn pid(&self) -> u32 {
        self.inner.pid
    }

    /// Returns the path to this node's listening socket.
    pub fn socket_path(&self) -> &Path {
        &self.inner.socket_path
    }

    /// Returns the runtime run directory containing peer sockets.
    pub fn run_dir(&self) -> &Path {
        &self.inner.run_dir
    }

    /// Returns the network endpoint of this node.
    pub fn endpoint(&self) -> &PeerEndpoint {
        &self.inner.endpoint
    }

    /// Returns this node's active AI model name.
    pub async fn active_model(&self) -> String {
        self.inner.active_model.read().await.clone()
    }

    /// Updates this node's active AI model name.
    pub async fn set_active_model(&self, model: impl Into<String>) {
        *self.inner.active_model.write().await = model.into();
    }

    /// Returns this node's current operational status.
    pub async fn status(&self) -> PeerStatus {
        *self.inner.status.read().await
    }

    /// Updates this node's operational status.
    pub async fn set_status(&self, status: PeerStatus) {
        *self.inner.status.write().await = status;
    }

    /// Returns whether auto-accept of task offers is enabled.
    pub async fn auto_accept(&self) -> bool {
        *self.inner.auto_accept.read().await
    }

    /// Enables or disables automatic acceptance of incoming task offers.
    pub async fn set_auto_accept(&self, enable: bool) {
        *self.inner.auto_accept.write().await = enable;
    }

    /// Returns a reference to the underlying `PeerRegistry`.
    pub fn registry(&self) -> &PeerRegistry {
        &self.inner.registry
    }

    // ========================================================================
    // Peer Discovery
    // ========================================================================

    /// Scans the run directory for active peer sockets, performing `Ping`/`Pong`
    /// handshakes and populating the registry of known active local peers.
    pub async fn discover_peers(&self) -> Result<Vec<PeerRecord>, MeshNetworkError> {
        let mut discovered = Vec::new();

        if !self.inner.run_dir.exists() {
            return Ok(discovered);
        }

        let entries = std::fs::read_dir(&self.inner.run_dir)?;
        for entry in entries.flatten() {
            let path = entry.path();
            let file_name = match path.file_name().and_then(|s| s.to_str()) {
                Some(name) => name,
                None => continue,
            };

            // Match mesh_<pid>.sock
            let target_pid = if let Some(rest) = file_name.strip_prefix("mesh_") {
                if let Some(pid_str) = rest.strip_suffix(".sock") {
                    match pid_str.parse::<u32>() {
                        Ok(p) => p,
                        Err(_) => continue,
                    }
                } else {
                    continue;
                }
            } else {
                continue;
            };

            // Skip probing our own node
            if target_pid == self.inner.pid {
                continue;
            }

            let endpoint = match PeerEndpoint::from_file(&path) {
                Ok(ep) => ep,
                Err(_) => continue,
            };

            // Ping candidate peer socket
            match self
                .request_from_endpoint(
                    &endpoint,
                    &MeshMessage::ping(self.peer_id()),
                    DEFAULT_REQUEST_TIMEOUT,
                )
                .await
            {
                Ok(MeshMessage::Pong {
                    peer_id,
                    active_model,
                    status,
                }) => {
                    self.inner
                        .registry
                        .register(
                            peer_id.clone(),
                            active_model.clone(),
                            status,
                            endpoint.clone(),
                            Some(target_pid),
                        )
                        .await;

                    let record = PeerRecord::new(
                        peer_id,
                        active_model,
                        status,
                        endpoint,
                        Some(target_pid),
                        self.inner.registry.default_ttl,
                    );
                    discovered.push(record);
                }
                Ok(other) => {
                    tracing::warn!("Expected Pong from {path:?}, got {other:?}");
                }
                Err(e) => {
                    tracing::debug!("Peer at {path:?} unreachable: {e}");
                    // Clean up dead/stale socket file if connection was refused
                    #[cfg(unix)]
                    if let MeshNetworkError::Io(io_err) = &e {
                        if io_err.kind() == std::io::ErrorKind::ConnectionRefused {
                            let _ = std::fs::remove_file(&path);
                        }
                    }
                }
            }
        }

        // Prune any stale peers exceeding TTL
        self.inner.registry.prune_expired().await;

        Ok(discovered)
    }

    /// Prunes expired peer records from the local registry.
    pub async fn prune_expired(&self) -> usize {
        self.inner.registry.prune_expired().await
    }

    /// Returns all known active peers that have not expired.
    pub async fn active_peers(&self) -> Vec<PeerRecord> {
        self.inner.registry.active_peers().await
    }

    /// Looks up an active peer by its peer ID.
    pub async fn get_peer(&self, peer_id: &str) -> Option<PeerRecord> {
        self.inner.registry.get_peer(peer_id).await
    }

    // ========================================================================
    // Direct & Broadcast Messaging
    // ========================================================================

    /// Sends a ping to a known peer and awaits their `Pong`.
    pub async fn ping(&self, peer_id: &str) -> Result<MeshMessage, MeshNetworkError> {
        let msg = MeshMessage::ping(self.peer_id());
        self.direct_request(peer_id, &msg, DEFAULT_REQUEST_TIMEOUT)
            .await
    }

    /// Sends a one-way message directly to a known active peer.
    pub async fn direct_send(
        &self,
        peer_id: &str,
        msg: &MeshMessage,
    ) -> Result<(), MeshNetworkError> {
        let endpoint = {
            let peer = self
                .inner
                .registry
                .get_peer(peer_id)
                .await
                .ok_or_else(|| MeshNetworkError::PeerNotFound(peer_id.to_string()))?;
            peer.endpoint
        };
        self.send_to_endpoint(&endpoint, msg).await
    }

    /// Sends a request directly to a peer and awaits their response message on the same stream.
    pub async fn direct_request(
        &self,
        peer_id: &str,
        msg: &MeshMessage,
        timeout_dur: Duration,
    ) -> Result<MeshMessage, MeshNetworkError> {
        let endpoint = {
            let peer = self
                .inner
                .registry
                .get_peer(peer_id)
                .await
                .ok_or_else(|| MeshNetworkError::PeerNotFound(peer_id.to_string()))?;
            peer.endpoint
        };
        self.request_from_endpoint(&endpoint, msg, timeout_dur)
            .await
    }

    /// Connects to a raw endpoint and sends a one-way message.
    pub async fn send_to_endpoint(
        &self,
        endpoint: &PeerEndpoint,
        msg: &MeshMessage,
    ) -> Result<(), MeshNetworkError> {
        let stream = endpoint.connect().await?;
        let (_, mut writer) = tokio::io::split(stream);
        send_message(&mut writer, msg).await?;
        Ok(())
    }

    /// Connects to a raw endpoint, sends a message, and reads the reply message.
    pub async fn request_from_endpoint(
        &self,
        endpoint: &PeerEndpoint,
        msg: &MeshMessage,
        timeout_dur: Duration,
    ) -> Result<MeshMessage, MeshNetworkError> {
        tokio::time::timeout(timeout_dur, async {
            let stream = endpoint.connect().await?;
            let (reader, mut writer) = tokio::io::split(stream);
            let mut buf_reader = BufReader::new(reader);

            send_message(&mut writer, msg).await?;
            let resp = read_message(&mut buf_reader)
                .await?
                .ok_or(MeshNetworkError::ConnectionClosed)?;
            Ok(resp)
        })
        .await
        .map_err(|_| MeshNetworkError::Timeout(timeout_dur))?
    }

    /// Asynchronously broadcasts a message to all known active local peers.
    pub async fn broadcast(&self, msg: &MeshMessage) -> Vec<(String, Result<(), MeshNetworkError>)> {
        let peers = self.active_peers().await;
        let mut futures = Vec::with_capacity(peers.len());

        for peer in peers {
            if peer.peer_id == self.peer_id() {
                continue;
            }
            let peer_id = peer.peer_id.clone();
            let endpoint = peer.endpoint.clone();
            let msg = msg.clone();

            futures.push(async move {
                let res = match endpoint.connect().await {
                    Ok(stream) => {
                        let (_, mut writer) = tokio::io::split(stream);
                        send_message(&mut writer, &msg)
                            .await
                            .map_err(MeshNetworkError::from)
                    }
                    Err(e) => Err(MeshNetworkError::from(e)),
                };
                (peer_id, res)
            });
        }

        futures::future::join_all(futures).await
    }

    // ========================================================================
    // Task Orchestration Helpers
    // ========================================================================

    /// Offers a task to a designated peer and awaits acceptance confirmation.
    pub async fn offer_task(
        &self,
        peer_id: &str,
        task_id: impl Into<String>,
        description: impl Into<String>,
        required_skills: Vec<String>,
        timeout_dur: Duration,
    ) -> Result<MeshMessage, MeshNetworkError> {
        let offer = MeshMessage::task_offer(task_id, description, required_skills);
        self.direct_request(peer_id, &offer, timeout_dur).await
    }

    /// Confirms acceptance of an offered task, switching node status to `Busy`.
    pub async fn accept_task(
        &self,
        requester_id: &str,
        task_id: impl Into<String>,
    ) -> Result<(), MeshNetworkError> {
        self.set_status(PeerStatus::Busy).await;
        let accept = MeshMessage::task_accept(task_id, self.peer_id());
        self.direct_send(requester_id, &accept).await
    }

    /// Reports completed task output and status back to the requester, switching status to `Idle`.
    pub async fn send_task_result(
        &self,
        requester_id: &str,
        task_id: impl Into<String>,
        output: impl Into<String>,
        success: bool,
    ) -> Result<(), MeshNetworkError> {
        self.set_status(PeerStatus::Idle).await;
        let result = MeshMessage::task_result(task_id, self.peer_id(), output, success);
        self.direct_send(requester_id, &result).await
    }

    /// Subscribes to all incoming messages received by this node.
    pub fn subscribe(&self) -> broadcast::Receiver<MeshMessage> {
        self.inner.message_tx.subscribe()
    }

    /// Helper waiting for the next incoming message with a timeout.
    pub async fn next_message(
        &self,
        rx: &mut broadcast::Receiver<MeshMessage>,
        timeout_dur: Duration,
    ) -> Result<MeshMessage, MeshNetworkError> {
        tokio::time::timeout(timeout_dur, async {
            loop {
                match rx.recv().await {
                    Ok(msg) => return Ok(msg),
                    Err(broadcast::error::RecvError::Lagged(_)) => continue,
                    Err(broadcast::error::RecvError::Closed) => {
                        return Err(MeshNetworkError::ConnectionClosed);
                    }
                }
            }
        })
        .await
        .map_err(|_| MeshNetworkError::Timeout(timeout_dur))?
    }

    /// Gracefully stops the listener loop and cleans up the socket file.
    pub async fn shutdown(&self) {
        let _ = self.inner.shutdown_tx.send(true);
        if self.inner.socket_path.exists() {
            let _ = std::fs::remove_file(&self.inner.socket_path);
        }
    }
}
