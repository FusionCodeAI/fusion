//! Inter-agent messaging bus and peer-to-peer coordination mesh.
//!
//! Provides a decentralized coordination fabric for autonomous subagents and advisors:
//! - **Pub-Sub Broadcasts**: Subagents broadcast status, progress, discoveries, and alerts.
//! - **Peer-to-Peer Queries**: Direct asynchronous request-response RPC with timeouts and mailboxes.
//! - **Advisor Reviews**: Subagents request architectural, security, or code quality reviews.
//! - **Coordination Channels**: Resource/file locking to prevent merge collisions, shared
//!   blackboard memory for discoveries, and synchronization barriers.
//! - **LLM Mesh Tools**: Tools enabling subagents to broadcast, query peers, request reviews,
//!   and coordinate resources directly during execution turns.

use std::collections::{HashMap, HashSet};
use std::fmt;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use thiserror::Error;
use tokio::sync::{broadcast, mpsc, oneshot, Notify, RwLock};
use tokio::time::timeout;
use uuid::Uuid;

use crate::agent::advisor::{AdvisorCritique, AdvisorEngine, AdvisorRegistry, RiskLevel};
use crate::agent::consensus::{resolve_consensus, ConsensusStrategy};
use crate::agent::subagent::SubagentRole;
use crate::tools::types::{Tool, ToolContext};

/// Default capacity for broadcast channels.
const DEFAULT_BROADCAST_CAPACITY: usize = 1024;
/// Default capacity for per-peer direct message mailboxes.
const DEFAULT_MAILBOX_CAPACITY: usize = 128;
/// Default capacity for per-peer query mailboxes.
const DEFAULT_QUERY_CAPACITY: usize = 64;
/// Default timeout for peer-to-peer queries.
const DEFAULT_QUERY_TIMEOUT: Duration = Duration::from_secs(30);

// ============================================================================
// Errors
// ============================================================================

/// Errors that can occur within the agent coordination mesh.
#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum MeshError {
    #[error("Peer '{0}' not found in mesh")]
    PeerNotFound(String),

    #[error("Peer '{0}' is already registered in mesh")]
    PeerAlreadyRegistered(String),

    #[error("Peer '{0}' has disconnected or mailbox closed")]
    PeerDisconnected(String),

    #[error("Direct query to peer '{to}' timed out (query_id: {query_id})")]
    QueryTimeout { to: String, query_id: String },

    #[error("Query failed: {0}")]
    QueryFailed(String),

    #[error("Resource '{resource}' is already claimed by peer '{claimed_by}'")]
    ResourceAlreadyClaimed {
        resource: String,
        claimed_by: String,
    },

    #[error("Resource '{resource}' is not claimed by peer '{agent_id}'")]
    ResourceNotOwned { resource: String, agent_id: String },

    #[error("Advisor '{0}' is unavailable")]
    AdvisorUnavailable(String),

    #[error("Barrier '{name}' timed out waiting for {expected} peers")]
    BarrierTimeout { name: String, expected: usize },

    #[error("Broadcast channel error: {0}")]
    BroadcastError(String),

    #[error("Channel closed")]
    ChannelClosed,

    #[error("Internal mesh error: {0}")]
    Internal(String),
}

// ============================================================================
// Agent Roles & Statuses
// ============================================================================

/// Role assumed by an agent or subagent within the mesh.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AgentRole {
    /// Read-only search, exploration, and analysis agent.
    Scout,
    /// Code implementation, refactoring, and file modification specialist.
    Coder,
    /// Testing, test runner, and regression specialist.
    Tester,
    /// Code quality, security, and architectural review specialist.
    Reviewer,
    /// Automated or human architectural/security advisor.
    Advisor,
    /// Main session orchestrator or root planning agent.
    Orchestrator,
    /// General-purpose worker subagent.
    General,
    /// Custom named role.
    Custom(String),
}

impl fmt::Display for AgentRole {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            AgentRole::Scout => write!(f, "Scout"),
            AgentRole::Coder => write!(f, "Coder"),
            AgentRole::Tester => write!(f, "Tester"),
            AgentRole::Reviewer => write!(f, "Reviewer"),
            AgentRole::Advisor => write!(f, "Advisor"),
            AgentRole::Orchestrator => write!(f, "Orchestrator"),
            AgentRole::General => write!(f, "General"),
            AgentRole::Custom(name) => write!(f, "{name}"),
        }
    }
}

impl From<&str> for AgentRole {
    fn from(s: &str) -> Self {
        match s.trim().to_lowercase().as_str() {
            "scout" => AgentRole::Scout,
            "coder" => AgentRole::Coder,
            "tester" => AgentRole::Tester,
            "reviewer" => AgentRole::Reviewer,
            "advisor" => AgentRole::Advisor,
            "orchestrator" | "main" => AgentRole::Orchestrator,
            "general" | "worker" => AgentRole::General,
            custom => AgentRole::Custom(custom.to_string()),
        }
    }
}

impl From<SubagentRole> for AgentRole {
    fn from(role: SubagentRole) -> Self {
        match role {
            SubagentRole::Scout => AgentRole::Scout,
            SubagentRole::Coder => AgentRole::Coder,
            SubagentRole::Tester => AgentRole::Tester,
            SubagentRole::Reviewer => AgentRole::Reviewer,
            SubagentRole::General => AgentRole::General,
            SubagentRole::Custom { name, .. } => AgentRole::Custom(name),
        }
    }
}

/// Dynamic execution status of an agent within the mesh.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "lowercase")]
pub enum AgentStatus {
    /// Agent is idle and ready to receive instructions or queries.
    Idle,
    /// Agent is actively executing a task.
    Active { task: String },
    /// Agent reports fine-grained progress.
    Progress {
        step: usize,
        total: Option<usize>,
        message: String,
    },
    /// Agent is blocked waiting on an external dependency, resource, or peer.
    Blocked {
        reason: String,
        waiting_for: Option<String>,
    },
    /// Agent is performing a review or evaluation.
    Reviewing { subject: String },
    /// Agent has completed its current task.
    Completed { result: Option<String> },
    /// Agent has failed or encountered an error.
    Failed { error: String },
    /// Agent has finished and is being decommissioned.
    Terminated,
}

impl fmt::Display for AgentStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            AgentStatus::Idle => write!(f, "Idle"),
            AgentStatus::Active { task } => write!(f, "Active: {task}"),
            AgentStatus::Progress {
                step,
                total,
                message,
            } => {
                if let Some(tot) = total {
                    write!(f, "Progress [{step}/{tot}]: {message}")
                } else {
                    write!(f, "Progress [step {step}]: {message}")
                }
            }
            AgentStatus::Blocked {
                reason,
                waiting_for,
            } => {
                if let Some(target) = waiting_for {
                    write!(f, "Blocked: {reason} (waiting for: {target})")
                } else {
                    write!(f, "Blocked: {reason}")
                }
            }
            AgentStatus::Reviewing { subject } => write!(f, "Reviewing: {subject}"),
            AgentStatus::Completed { result } => {
                if let Some(res) = result {
                    write!(f, "Completed: {res}")
                } else {
                    write!(f, "Completed")
                }
            }
            AgentStatus::Failed { error } => write!(f, "Failed: {error}"),
            AgentStatus::Terminated => write!(f, "Terminated"),
        }
    }
}

/// Returns current UTC timestamp in ISO 8601 / RFC 3339 format.
fn current_timestamp() -> String {
    Utc::now().to_rfc3339()
}

/// Metadata and registration info for a peer in the mesh.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentInfo {
    /// Unique agent identifier (e.g. "Scout-1", "Coder-2", "ArchitectureAdvisor").
    pub id: String,
    /// Role assigned to the agent.
    pub role: AgentRole,
    /// Brief description of the agent's function.
    pub description: String,
    /// Current execution status.
    pub status: AgentStatus,
    /// When the agent registered with the mesh.
    /// When the agent registered with the mesh (RFC 3339).
    pub registered_at: String,
    /// Last recorded heartbeat or activity timestamp (RFC 3339).
    pub last_active: String,
    /// Optional capability tags (e.g. ["rust", "diff", "security", "filesystem"]).
    pub capabilities: Vec<String>,
}

// ============================================================================
// Broadcast Types (Pub-Sub)
// ============================================================================

/// Broadcast topics standardly supported by the mesh.
pub mod topics {
    pub const STATUS: &str = "status";
    pub const DISCOVERY: &str = "discovery";
    pub const ALERT: &str = "alert";
    pub const COORDINATION: &str = "coordination";
    pub const ALL: &str = "*";
}

/// Payload carried by a pub-sub broadcast message.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum BroadcastPayload {
    /// Agent status update (state change, progress).
    Status { status: AgentStatus },
    /// Shared code discovery or knowledge finding.
    Discovery {
        topic: String,
        findings: String,
        file_references: Vec<String>,
    },
    /// System, security, or error alert.
    Alert { severity: String, message: String },
    /// Shared fact update on the blackboard.
    FactUpdate { key: String, value: Value },
    /// Custom application or extension payload.
    Custom { kind: String, data: Value },
}

/// Message broadcast to all interested peers across the mesh.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BroadcastMessage {
    /// Unique message identifier.
    pub id: String,
    /// Sender agent ID.
    pub sender: String,
    /// Topic for broadcast routing (e.g. "status", "discovery", "alert").
    pub topic: String,
    /// The message payload.
    pub payload: BroadcastPayload,
    /// Broadcast timestamp.
    /// Broadcast timestamp (RFC 3339).
    pub timestamp: String,
}

impl BroadcastMessage {
    /// Creates a new broadcast message with a unique ID and current timestamp.
    pub fn new(
        sender: impl Into<String>,
        topic: impl Into<String>,
        payload: BroadcastPayload,
    ) -> Self {
        Self {
            id: Uuid::new_v4().to_string(),
            sender: sender.into(),
            topic: topic.into(),
            payload,
            timestamp: current_timestamp(),
        }
    }
}

// ============================================================================
// Direct Peer-to-Peer Messaging (Direct Messages & RPC Queries)
// ============================================================================

/// A direct point-to-point message sent from one agent to another.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DirectMessage {
    /// Unique message identifier.
    pub id: String,
    /// Sender agent ID.
    pub from: String,
    /// Recipient agent ID.
    pub to: String,
    /// Subject or intent of the message.
    pub subject: String,
    /// Textual content or body.
    pub content: String,
    /// Optional structured JSON payload.
    pub payload: Value,
    /// Optional parent message ID for conversation threading.
    pub reply_to: Option<String>,
    /// Timestamp when sent.
    /// Timestamp when sent (RFC 3339).
    pub timestamp: String,
}

impl DirectMessage {
    /// Creates a new direct message.
    pub fn new(
        from: impl Into<String>,
        to: impl Into<String>,
        subject: impl Into<String>,
        content: impl Into<String>,
    ) -> Self {
        Self {
            id: Uuid::new_v4().to_string(),
            from: from.into(),
            to: to.into(),
            subject: subject.into(),
            content: content.into(),
            payload: Value::Null,
            reply_to: None,
            timestamp: current_timestamp(),
        }
    }

    /// Attaches a structured JSON payload to the message.
    pub fn with_payload(mut self, payload: Value) -> Self {
        self.payload = payload;
        self
    }

    /// Marks this message as a reply to a previous message ID.
    pub fn with_reply_to(mut self, reply_to: impl Into<String>) -> Self {
        self.reply_to = Some(reply_to.into());
        self
    }
}

/// A direct query request sent from one peer to another awaiting an answer.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PeerQuery {
    /// Unique query identifier.
    pub query_id: String,
    /// Sender agent ID.
    pub from: String,
    /// Recipient agent ID.
    pub to: String,
    /// Query question or request text.
    pub query: String,
    /// Optional contextual details or file snippets.
    pub context: Option<String>,
    /// Timestamp when the query was dispatched.
    /// Timestamp when the query was dispatched (RFC 3339).
    pub timestamp: String,
}

/// An answer returned in response to a `PeerQuery`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PeerResponse {
    /// Correlated query identifier.
    pub query_id: String,
    /// Responder agent ID.
    pub from: String,
    /// Original requester agent ID.
    pub to: String,
    /// Answer content.
    pub answer: String,
    /// Whether the query was answered successfully.
    pub success: bool,
    /// Optional structured result data.
    pub data: Option<Value>,
    /// Timestamp when answered.
    /// Timestamp when answered (RFC 3339).
    pub timestamp: String,
}

impl PeerResponse {
    /// Creates a successful response to a query.
    pub fn success(query: &PeerQuery, answer: impl Into<String>) -> Self {
        Self {
            query_id: query.query_id.clone(),
            from: query.to.clone(),
            to: query.from.clone(),
            answer: answer.into(),
            success: true,
            data: None,
            timestamp: current_timestamp(),
        }
    }

    /// Creates an error or failure response to a query.
    pub fn failure(query: &PeerQuery, error: impl Into<String>) -> Self {
        Self {
            query_id: query.query_id.clone(),
            from: query.to.clone(),
            to: query.from.clone(),
            answer: error.into(),
            success: false,
            data: None,
            timestamp: current_timestamp(),
        }
    }

    /// Attaches structured data to the response.
    pub fn with_data(mut self, data: Value) -> Self {
        self.data = Some(data);
        self
    }
}

/// Envelope delivering a query to a recipient peer along with a one-shot response channel.
pub struct PeerQueryEnvelope {
    /// The incoming query.
    pub query: PeerQuery,
    /// Reply channel to resolve the query.
    reply_tx: oneshot::Sender<PeerResponse>,
}

impl PeerQueryEnvelope {
    /// Returns a reference to the inner query.
    pub fn query(&self) -> &PeerQuery {
        &self.query
    }

    /// Sends a successful response back to the requester.
    pub fn respond(self, answer: impl Into<String>) -> Result<(), MeshError> {
        let resp = PeerResponse::success(&self.query, answer);
        self.reply_tx
            .send(resp)
            .map_err(|_| MeshError::ChannelClosed)
    }

    /// Sends a response with structured data back to the requester.
    pub fn respond_with_data(
        self,
        answer: impl Into<String>,
        data: Value,
    ) -> Result<(), MeshError> {
        let resp = PeerResponse::success(&self.query, answer).with_data(data);
        self.reply_tx
            .send(resp)
            .map_err(|_| MeshError::ChannelClosed)
    }

    /// Sends a failure response back to the requester.
    pub fn fail(self, error: impl Into<String>) -> Result<(), MeshError> {
        let resp = PeerResponse::failure(&self.query, error);
        self.reply_tx
            .send(resp)
            .map_err(|_| MeshError::ChannelClosed)
    }
}

// ============================================================================
// Advisor Reviews
// ============================================================================

/// Request submitted by a subagent for architectural, security, or code quality critique.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AdvisorReviewRequest {
    /// Unique review ticket ID.
    pub request_id: String,
    /// Subagent requesting the review.
    pub requester: String,
    /// Target advisor name, or None to consult all available default advisors.
    pub advisor: Option<String>,
    /// Brief subject of what is being reviewed.
    pub subject: String,
    /// The planned action, code diff, command, or proposal.
    pub diff_or_plan: String,
    /// Optional context or motivation.
    pub context: Option<String>,
    /// Timestamp when requested.
    /// Timestamp when requested (RFC 3339).
    pub timestamp: String,
}

impl AdvisorReviewRequest {
    /// Creates a new review request.
    pub fn new(
        requester: impl Into<String>,
        subject: impl Into<String>,
        diff_or_plan: impl Into<String>,
    ) -> Self {
        Self {
            request_id: Uuid::new_v4().to_string(),
            requester: requester.into(),
            advisor: None,
            subject: subject.into(),
            diff_or_plan: diff_or_plan.into(),
            context: None,
            timestamp: current_timestamp(),
        }
    }

    /// Directs the review to a specific named advisor (e.g. "SecurityAdvisor").
    pub fn with_advisor(mut self, advisor: impl Into<String>) -> Self {
        self.advisor = Some(advisor.into());
        self
    }

    /// Supplies contextual background for the review.
    pub fn with_context(mut self, context: impl Into<String>) -> Self {
        self.context = Some(context.into());
        self
    }
}

/// Consolidated advisor critique and assessment returned to the subagent.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AdvisorReviewResponse {
    /// Correlated review ticket ID.
    pub request_id: String,
    /// Whether all consulting advisors approved the proposed action.
    pub approved: bool,
    /// Individual critiques from each consulting advisor.
    pub critiques: Vec<AdvisorCritique>,
    /// Highest risk level assessed among all critiques.
    pub highest_risk: RiskLevel,
    /// Textual summary of critiques and suggestions.
    pub summary: String,
    /// Timestamp when review completed.
    /// Timestamp when review completed (RFC 3339).
    pub reviewed_at: String,
}

impl AdvisorReviewResponse {
    /// Creates a response from a list of `AdvisorCritique`s.
    pub fn from_critiques(request_id: String, critiques: Vec<AdvisorCritique>) -> Self {
        let approved = critiques.is_empty() || critiques.iter().all(|c| c.approved);
        let highest_risk = critiques
            .iter()
            .map(|c| c.risk_level)
            .max()
            .unwrap_or(RiskLevel::Low);

        let summary = if critiques.is_empty() {
            "No critiques generated; auto-approved.".to_string()
        } else {
            let mut lines = Vec::new();
            for c in &critiques {
                let status_icon = if c.approved {
                    "✓ APPROVED"
                } else {
                    "✗ REJECTED"
                };
                lines.push(format!(
                    "[{status_icon}] {} (Risk: {}): {}",
                    c.advisor, c.risk_level, c.critique
                ));
                for s in &c.suggestions {
                    lines.push(format!("  - Suggestion: {s}"));
                }
            }
            lines.join("\n")
        };

        Self {
            request_id,
            approved,
            critiques,
            highest_risk,
            summary,
            reviewed_at: current_timestamp(),
        }
    }

    /// Creates a response from a list of `AdvisorCritique`s using a formal consensus strategy.
    pub fn from_critiques_with_consensus(
        request_id: String,
        critiques: Vec<AdvisorCritique>,
        strategy: ConsensusStrategy,
    ) -> Self {
        let resolution = resolve_consensus(&critiques, strategy);
        Self {
            request_id,
            approved: resolution.approved,
            highest_risk: resolution.highest_risk,
            summary: resolution.summary,
            critiques,
            reviewed_at: current_timestamp(),
        }
    }
}

/// Optional custom asynchronous handler for advisor reviews.
#[async_trait]
pub trait AdvisorReviewHandler: Send + Sync {
    async fn handle_review(
        &self,
        req: &AdvisorReviewRequest,
    ) -> Result<AdvisorReviewResponse, MeshError>;
}

// ============================================================================
// Coordination Primitives: Resource Locks & Blackboard
// ============================================================================

/// Information on a resource claimed by an agent to prevent collision.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResourceClaim {
    /// Resource identifier (e.g. "src/agent/mesh.rs" or "database_schema").
    pub resource: String,
    /// Agent that holds the claim.
    pub owner: String,
    /// When the claim was granted.
    /// When the claim was granted (RFC 3339).
    pub claimed_at: String,
    /// Optional expiration timestamp (RFC 3339).
    pub expires_at: Option<String>,
}

/// A fact or discovery recorded on the shared mesh blackboard.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SharedFact {
    /// Fact identifier / key.
    pub key: String,
    /// Fact value.
    pub value: Value,
    /// Author agent that recorded the fact.
    pub author: String,
    /// Monotonic revision counter.
    pub revision: u64,
    /// When the fact was last modified.
    /// When the fact was last modified (RFC 3339).
    pub updated_at: String,
}

/// Internal state of a multi-agent synchronization barrier.
struct BarrierState {
    expected: usize,
    arrived: HashSet<String>,
    notify: Arc<Notify>,
}

// ============================================================================
// Mesh Inner State & AgentMesh Bus
// ============================================================================

/// Channels assigned to a registered peer for receiving incoming messages.
struct PeerMailbox {
    direct_tx: mpsc::Sender<DirectMessage>,
    query_tx: mpsc::Sender<PeerQueryEnvelope>,
}

/// Internal shared state of the agent mesh.
struct AgentMeshInner {
    /// Active peers indexed by agent ID.
    peers: RwLock<HashMap<String, AgentInfo>>,
    /// Routing mailboxes for active peers.
    mailboxes: RwLock<HashMap<String, PeerMailbox>>,
    /// Global broadcast sender for pub-sub messages.
    broadcast_tx: broadcast::Sender<BroadcastMessage>,
    /// Recent broadcast history buffer.
    recent_broadcasts: RwLock<Vec<BroadcastMessage>>,
    /// Shared blackboard key-value store.
    blackboard: RwLock<HashMap<String, SharedFact>>,
    /// Resource claims (e.g. file lock registry).
    resource_claims: RwLock<HashMap<String, ResourceClaim>>,
    /// Coordination barriers.
    barriers: RwLock<HashMap<String, BarrierState>>,
    /// Optional attached Advisor review handler.
    advisor_handler: RwLock<Option<Arc<dyn AdvisorReviewHandler>>>,
}

/// Decentralized coordination bus connecting subagents, advisors, and orchestrator.
#[derive(Clone)]
pub struct AgentMesh {
    inner: Arc<AgentMeshInner>,
}

impl Default for AgentMesh {
    fn default() -> Self {
        Self::new()
    }
}

impl AgentMesh {
    /// Creates a new `AgentMesh` coordination bus with default capacities.
    pub fn new() -> Self {
        Self::with_capacity(DEFAULT_BROADCAST_CAPACITY)
    }

    /// Creates an `AgentMesh` with a custom broadcast capacity.
    pub fn with_capacity(broadcast_capacity: usize) -> Self {
        let (broadcast_tx, _) = broadcast::channel(broadcast_capacity.max(16));
        Self {
            inner: Arc::new(AgentMeshInner {
                peers: RwLock::new(HashMap::new()),
                mailboxes: RwLock::new(HashMap::new()),
                broadcast_tx,
                recent_broadcasts: RwLock::new(Vec::new()),
                blackboard: RwLock::new(HashMap::new()),
                resource_claims: RwLock::new(HashMap::new()),
                barriers: RwLock::new(HashMap::new()),
                advisor_handler: RwLock::new(None),
            }),
        }
    }

    // ------------------------------------------------------------------------
    // Peer Registration & Lifecycle
    // ------------------------------------------------------------------------

    /// Registers an agent on the mesh, establishing direct and query mailboxes.
    ///
    /// Returns a dedicated `MeshPeerChannel` handle for the registered agent.
    pub async fn register(
        &self,
        agent_id: impl Into<String>,
        role: AgentRole,
        description: impl Into<String>,
    ) -> Result<MeshPeerChannel, MeshError> {
        let id = agent_id.into();
        let desc = description.into();

        let mut peers = self.inner.peers.write().await;
        if peers.contains_key(&id) {
            return Err(MeshError::PeerAlreadyRegistered(id));
        }

        let info = AgentInfo {
            id: id.clone(),
            role: role.clone(),
            description: desc,
            status: AgentStatus::Idle,
            registered_at: current_timestamp(),
            last_active: current_timestamp(),
            capabilities: Vec::new(),
        };
        peers.insert(id.clone(), info);

        let (direct_tx, direct_rx) = mpsc::channel(DEFAULT_MAILBOX_CAPACITY);
        let (query_tx, query_rx) = mpsc::channel(DEFAULT_QUERY_CAPACITY);
        let broadcast_rx = self.inner.broadcast_tx.subscribe();

        let mut mailboxes = self.inner.mailboxes.write().await;
        mailboxes.insert(
            id.clone(),
            PeerMailbox {
                direct_tx,
                query_tx,
            },
        );

        // Broadcast initial status event to all peers
        let _ = self.broadcast_status(&id, AgentStatus::Idle).await;

        Ok(MeshPeerChannel {
            agent_id: id,
            role,
            mesh: self.clone(),
            direct_rx,
            query_rx,
            broadcast_rx,
        })
    }

    /// Deregisters an agent from the mesh and releases any resource claims held by it.
    pub async fn unregister(&self, agent_id: &str) -> Result<(), MeshError> {
        let mut peers = self.inner.peers.write().await;
        if peers.remove(agent_id).is_none() {
            return Err(MeshError::PeerNotFound(agent_id.to_string()));
        }

        let mut mailboxes = self.inner.mailboxes.write().await;
        mailboxes.remove(agent_id);

        // Release any resource claims held by the unregistering peer
        let mut claims = self.inner.resource_claims.write().await;
        claims.retain(|_, claim| claim.owner != agent_id);

        // Broadcast terminated status
        let msg = BroadcastMessage::new(
            agent_id,
            topics::STATUS,
            BroadcastPayload::Status {
                status: AgentStatus::Terminated,
            },
        );
        let _ = self.broadcast(msg).await;

        Ok(())
    }

    /// Updates the execution status of a peer.
    pub async fn update_status(
        &self,
        agent_id: &str,
        status: AgentStatus,
    ) -> Result<(), MeshError> {
        let mut peers = self.inner.peers.write().await;
        let info = peers
            .get_mut(agent_id)
            .ok_or_else(|| MeshError::PeerNotFound(agent_id.to_string()))?;

        info.status = status.clone();
        info.last_active = current_timestamp();

        drop(peers);
        self.broadcast_status(agent_id, status).await?;
        Ok(())
    }

    /// Lists metadata for all currently registered peers.
    pub async fn list_peers(&self) -> Vec<AgentInfo> {
        self.inner.peers.read().await.values().cloned().collect()
    }

    /// Looks up information on a specific peer by ID.
    pub async fn get_peer(&self, agent_id: &str) -> Option<AgentInfo> {
        self.inner.peers.read().await.get(agent_id).cloned()
    }

    /// Finds all registered peers with a matching role.
    pub async fn find_peers_by_role(&self, role: &AgentRole) -> Vec<AgentInfo> {
        self.inner
            .peers
            .read()
            .await
            .values()
            .filter(|p| &p.role == role)
            .cloned()
            .collect()
    }

    // ------------------------------------------------------------------------
    // Pub-Sub Broadcasts
    // ------------------------------------------------------------------------

    /// Broadcasts a message to all subscribed peers across the mesh.
    pub async fn broadcast(&self, message: BroadcastMessage) -> Result<usize, MeshError> {
        // Record in recent broadcasts buffer (capped at 256)
        {
            let mut recent = self.inner.recent_broadcasts.write().await;
            recent.push(message.clone());
            if recent.len() > 256 {
                recent.remove(0);
            }
        }

        // Send over broadcast channel
        self.inner
            .broadcast_tx
            .send(message)
            .map_err(|e| MeshError::BroadcastError(e.to_string()))
    }

    /// Convenience: broadcasts a status update for an agent.
    pub async fn broadcast_status(
        &self,
        agent_id: &str,
        status: AgentStatus,
    ) -> Result<(), MeshError> {
        let msg = BroadcastMessage::new(
            agent_id,
            topics::STATUS,
            BroadcastPayload::Status { status },
        );
        let _ = self.broadcast(msg).await;
        Ok(())
    }

    /// Convenience: broadcasts a discovery or finding made by an agent.
    pub async fn broadcast_discovery(
        &self,
        agent_id: &str,
        topic: &str,
        findings: &str,
        file_references: Vec<String>,
    ) -> Result<(), MeshError> {
        let msg = BroadcastMessage::new(
            agent_id,
            topics::DISCOVERY,
            BroadcastPayload::Discovery {
                topic: topic.to_string(),
                findings: findings.to_string(),
                file_references,
            },
        );
        let _ = self.broadcast(msg).await;
        Ok(())
    }

    /// Returns a new broadcast receiver subscribed to the mesh.
    pub fn subscribe(&self) -> broadcast::Receiver<BroadcastMessage> {
        self.inner.broadcast_tx.subscribe()
    }

    /// Retrieves recent broadcast messages.
    pub async fn recent_broadcasts(&self) -> Vec<BroadcastMessage> {
        self.inner.recent_broadcasts.read().await.clone()
    }

    // ------------------------------------------------------------------------
    // Direct Messaging & Queries
    // ------------------------------------------------------------------------

    /// Sends a direct point-to-point message to a specific peer.
    pub async fn send_direct(&self, msg: DirectMessage) -> Result<(), MeshError> {
        let mailboxes = self.inner.mailboxes.read().await;
        let mailbox = mailboxes
            .get(&msg.to)
            .ok_or_else(|| MeshError::PeerNotFound(msg.to.clone()))?;

        mailbox
            .direct_tx
            .send(msg)
            .await
            .map_err(|_| MeshError::PeerDisconnected(mailbox.direct_tx.capacity().to_string()))
    }

    /// Sends a direct query to a peer and awaits their response within a timeout.
    pub async fn query_peer(
        &self,
        from: impl Into<String>,
        to: impl Into<String>,
        query_text: impl Into<String>,
        context: Option<String>,
        query_timeout: Option<Duration>,
    ) -> Result<PeerResponse, MeshError> {
        let from_id = from.into();
        let to_id = to.into();
        let q_text = query_text.into();
        let t_out = query_timeout.unwrap_or(DEFAULT_QUERY_TIMEOUT);

        let query = PeerQuery {
            query_id: Uuid::new_v4().to_string(),
            from: from_id,
            to: to_id.clone(),
            query: q_text,
            context,
            timestamp: current_timestamp(),
        };

        let (reply_tx, reply_rx) = oneshot::channel();
        let envelope = PeerQueryEnvelope {
            query: query.clone(),
            reply_tx,
        };

        // Locate recipient mailbox
        {
            let mailboxes = self.inner.mailboxes.read().await;
            let mailbox = mailboxes
                .get(&to_id)
                .ok_or_else(|| MeshError::PeerNotFound(to_id.clone()))?;

            mailbox
                .query_tx
                .send(envelope)
                .await
                .map_err(|_| MeshError::PeerDisconnected(to_id.clone()))?;
        }

        // Await response with timeout
        match timeout(t_out, reply_rx).await {
            Ok(Ok(response)) => Ok(response),
            Ok(Err(_)) => Err(MeshError::PeerDisconnected(to_id)),
            Err(_) => Err(MeshError::QueryTimeout {
                to: to_id,
                query_id: query.query_id,
            }),
        }
    }

    // ------------------------------------------------------------------------
    // Advisor Reviews
    // ------------------------------------------------------------------------

    /// Attaches an asynchronous advisor review handler to the mesh.
    pub async fn set_advisor_handler(&self, handler: Arc<dyn AdvisorReviewHandler>) {
        let mut slot = self.inner.advisor_handler.write().await;
        *slot = Some(handler);
    }

    /// Requests an advisor review for a planned action or code diff.
    ///
    /// If an advisor handler is attached, routes to it. Otherwise evaluates using
    /// the built-in heuristic/rule-based advisor engine.
    pub async fn request_advisor_review(
        &self,
        req: AdvisorReviewRequest,
    ) -> Result<AdvisorReviewResponse, MeshError> {
        // 1. Check if a custom advisor handler is registered
        let custom_handler = {
            let slot = self.inner.advisor_handler.read().await;
            slot.clone()
        };

        if let Some(handler) = custom_handler {
            return handler.handle_review(&req).await;
        }

        // 2. Built-in heuristic rule-based advisor review
        let critiques = Self::evaluate_heuristic_advisors(&req);
        Ok(AdvisorReviewResponse::from_critiques(
            req.request_id,
            critiques,
        ))
    }

    /// Internal heuristic rules for offline/built-in advisor evaluations.
    fn evaluate_heuristic_advisors(req: &AdvisorReviewRequest) -> Vec<AdvisorCritique> {
        let mut critiques = Vec::new();
        let content_lower = req.diff_or_plan.to_lowercase();

        // Security Advisor Heuristics
        let mut sec_approved = true;
        let mut sec_risk = RiskLevel::Low;
        let mut sec_critique = String::from("No immediate security vulnerabilities detected.");
        let mut sec_suggestions = Vec::new();

        if content_lower.contains("rm -rf /")
            || content_lower.contains("rm -rf ~")
            || content_lower.contains("format ")
            || content_lower.contains("mkfs")
        {
            sec_approved = false;
            sec_risk = RiskLevel::Critical;
            sec_critique = "Catastrophic command detected: potential destructive deletion of root or filesystem."
                .to_string();
            sec_suggestions.push("Never execute destructive filesystem wipes.".to_string());
        } else if content_lower.contains("api_key")
            || content_lower.contains("password")
            || content_lower.contains("secret")
            || content_lower.contains("id_rsa")
            || content_lower.contains("BEGIN PRIVATE KEY")
        {
            sec_approved = false;
            sec_risk = RiskLevel::High;
            sec_critique =
                "Potential secret or private key leakage detected in proposed diff or parameters."
                    .to_string();
            sec_suggestions
                .push("Sanitize credentials before persisting or executing.".to_string());
        }

        critiques.push(AdvisorCritique {
            advisor: "SecurityAdvisor".to_string(),
            focus: "Command safety, vulnerability prevention, and secret leakage".to_string(),
            approved: sec_approved,
            risk_level: sec_risk,
            critique: sec_critique,
            suggestions: sec_suggestions,
        });

        // Architecture Advisor Heuristics
        let arch_approved = true;
        let mut arch_risk = RiskLevel::Low;
        let mut arch_critique =
            String::from("Architectural design aligns with modularity and separation of concerns.");
        let mut arch_suggestions = Vec::new();

        if content_lower.contains("unwrap()") && !content_lower.contains("// test") {
            arch_risk = RiskLevel::Medium;
            arch_critique =
                "Production code contains unhandled unwraps which may trigger panic.".to_string();
            arch_suggestions.push(
                "Replace unwrap() with idiomatic Result/Option handling (? or ok_or).".to_string(),
            );
        }

        if content_lower.contains("/tmp/") {
            arch_risk = RiskLevel::Medium;
            arch_critique =
                "Hardcoded /tmp path detected: breaks Android/Termux and Windows compatibility."
                    .to_string();
            arch_suggestions
                .push("Use std::env::temp_dir() or workspace-relative scratch paths.".to_string());
        }

        critiques.push(AdvisorCritique {
            advisor: "ArchitectureAdvisor".to_string(),
            focus: "Modularity, separation of concerns, and cross-platform compatibility"
                .to_string(),
            approved: arch_approved,
            risk_level: arch_risk,
            critique: arch_critique,
            suggestions: arch_suggestions,
        });

        // Filter if caller requested a specific advisor
        if let Some(target_advisor) = &req.advisor {
            critiques.retain(|c| c.advisor.eq_ignore_ascii_case(target_advisor));
        }

        critiques
    }

    // ------------------------------------------------------------------------
    // Resource Locking & File Claims
    // ------------------------------------------------------------------------

    /// Attempts to claim exclusive ownership of a resource (e.g. file path).
    pub async fn try_claim_resource(
        &self,
        agent_id: &str,
        resource: &str,
        ttl: Option<Duration>,
    ) -> Result<(), MeshError> {
        let mut claims = self.inner.resource_claims.write().await;
        let now = Utc::now();

        // Check for existing unexpired claim
        if let Some(existing) = claims.get(resource) {
            let expired = existing
                .expires_at
                .as_deref()
                .and_then(|exp| chrono::DateTime::parse_from_rfc3339(exp).ok())
                .map(|exp| exp.with_timezone(&Utc) <= now)
                .unwrap_or(false);
            if !expired && existing.owner != agent_id {
                return Err(MeshError::ResourceAlreadyClaimed {
                    resource: resource.to_string(),
                    claimed_by: existing.owner.clone(),
                });
            }
        }

        let expires_at =
            ttl.map(|d| (now + chrono::Duration::from_std(d).unwrap_or_default()).to_rfc3339());
        claims.insert(
            resource.to_string(),
            ResourceClaim {
                resource: resource.to_string(),
                owner: agent_id.to_string(),
                claimed_at: current_timestamp(),
                expires_at,
            },
        );

        // Broadcast coordination event
        let _ = self
            .broadcast(BroadcastMessage::new(
                agent_id,
                topics::COORDINATION,
                BroadcastPayload::Custom {
                    kind: "resource_claimed".to_string(),
                    data: json!({ "resource": resource, "owner": agent_id }),
                },
            ))
            .await;

        Ok(())
    }

    /// Releases a resource claimed by an agent.
    pub async fn release_resource(
        &self,
        agent_id: &str,
        resource: &str,
    ) -> Result<bool, MeshError> {
        let mut claims = self.inner.resource_claims.write().await;
        if let Some(claim) = claims.get(resource) {
            if claim.owner != agent_id {
                return Err(MeshError::ResourceNotOwned {
                    resource: resource.to_string(),
                    agent_id: agent_id.to_string(),
                });
            }
            claims.remove(resource);

            // Broadcast coordination release event
            let _ = self
                .broadcast(BroadcastMessage::new(
                    agent_id,
                    topics::COORDINATION,
                    BroadcastPayload::Custom {
                        kind: "resource_released".to_string(),
                        data: json!({ "resource": resource, "owner": agent_id }),
                    },
                ))
                .await;

            return Ok(true);
        }
        Ok(false)
    }

    /// Returns a map of all currently active resource claims.
    pub async fn get_resource_claims(&self) -> HashMap<String, ResourceClaim> {
        self.inner.resource_claims.read().await.clone()
    }

    // ------------------------------------------------------------------------
    // Shared Blackboard (Knowledge Store)
    // ------------------------------------------------------------------------

    /// Sets or updates a fact on the shared blackboard.
    pub async fn set_shared_fact(
        &self,
        author: impl Into<String>,
        key: impl Into<String>,
        value: Value,
    ) -> Result<u64, MeshError> {
        let author_str = author.into();
        let key_str = key.into();
        let mut bb = self.inner.blackboard.write().await;

        let revision = match bb.get_mut(&key_str) {
            Some(existing) => {
                existing.value = value.clone();
                existing.author = author_str.clone();
                existing.revision += 1;
                existing.updated_at = current_timestamp();
                existing.revision
            }
            None => {
                let fact = SharedFact {
                    key: key_str.clone(),
                    value: value.clone(),
                    author: author_str.clone(),
                    revision: 1,
                    updated_at: current_timestamp(),
                };
                bb.insert(key_str.clone(), fact);
                1
            }
        };

        // Notify peers of the fact update
        let _ = self
            .broadcast(BroadcastMessage::new(
                &author_str,
                topics::COORDINATION,
                BroadcastPayload::FactUpdate {
                    key: key_str,
                    value,
                },
            ))
            .await;

        Ok(revision)
    }

    /// Retrieves a fact from the shared blackboard by key.
    pub async fn get_shared_fact(&self, key: &str) -> Option<SharedFact> {
        self.inner.blackboard.read().await.get(key).cloned()
    }

    /// Returns all facts recorded on the shared blackboard.
    pub async fn get_all_shared_facts(&self) -> HashMap<String, SharedFact> {
        self.inner.blackboard.read().await.clone()
    }

    // ------------------------------------------------------------------------
    // Coordination Barriers
    // ------------------------------------------------------------------------

    /// Registers a synchronization barrier waiting for `expected` peers.
    pub async fn create_barrier(&self, name: impl Into<String>, expected: usize) {
        let mut barriers = self.inner.barriers.write().await;
        barriers.insert(
            name.into(),
            BarrierState {
                expected,
                arrived: HashSet::new(),
                notify: Arc::new(Notify::new()),
            },
        );
    }

    /// Marks an agent as having arrived at a barrier and waits until all peers arrive.
    pub async fn wait_barrier(
        &self,
        name: &str,
        agent_id: &str,
        wait_timeout: Duration,
    ) -> Result<(), MeshError> {
        let (notify, is_last) = {
            let mut barriers = self.inner.barriers.write().await;
            let barrier = barriers
                .get_mut(name)
                .ok_or_else(|| MeshError::Internal(format!("Barrier '{name}' does not exist")))?;

            barrier.arrived.insert(agent_id.to_string());
            let is_last = barrier.arrived.len() >= barrier.expected;
            (barrier.notify.clone(), is_last)
        };

        if is_last {
            notify.notify_waiters();
            return Ok(());
        }

        match timeout(wait_timeout, notify.notified()).await {
            Ok(_) => Ok(()),
            Err(_) => {
                let barriers = self.inner.barriers.read().await;
                let expected = barriers.get(name).map(|b| b.expected).unwrap_or(0);
                Err(MeshError::BarrierTimeout {
                    name: name.to_string(),
                    expected,
                })
            }
        }
    }
}

// ============================================================================
// Dedicated Peer Channel Endpoint (MeshPeerChannel)
// ============================================================================

/// An agent's private, bidirectional communication handle to the mesh.
pub struct MeshPeerChannel {
    agent_id: String,
    role: AgentRole,
    mesh: AgentMesh,
    direct_rx: mpsc::Receiver<DirectMessage>,
    query_rx: mpsc::Receiver<PeerQueryEnvelope>,
    broadcast_rx: broadcast::Receiver<BroadcastMessage>,
}

impl MeshPeerChannel {
    /// Returns the unique ID of the agent associated with this channel.
    pub fn agent_id(&self) -> &str {
        &self.agent_id
    }

    /// Returns the role of the agent.
    pub fn role(&self) -> &AgentRole {
        &self.role
    }

    /// Returns a reference to the underlying mesh bus.
    pub fn mesh(&self) -> &AgentMesh {
        &self.mesh
    }

    /// Broadcasts an updated status for this agent.
    pub async fn broadcast_status(&self, status: AgentStatus) -> Result<(), MeshError> {
        self.mesh.update_status(&self.agent_id, status).await
    }

    /// Broadcasts a discovery or code finding.
    pub async fn broadcast_discovery(
        &self,
        topic: &str,
        findings: &str,
        file_references: Vec<String>,
    ) -> Result<(), MeshError> {
        self.mesh
            .broadcast_discovery(&self.agent_id, topic, findings, file_references)
            .await
    }

    /// Broadcasts an arbitrary payload on a specified topic.
    pub async fn broadcast(
        &self,
        topic: &str,
        payload: BroadcastPayload,
    ) -> Result<usize, MeshError> {
        let msg = BroadcastMessage::new(&self.agent_id, topic, payload);
        self.mesh.broadcast(msg).await
    }

    /// Sends a direct message to a peer agent.
    pub async fn send_direct(
        &self,
        to: &str,
        subject: &str,
        content: &str,
    ) -> Result<(), MeshError> {
        let msg = DirectMessage::new(&self.agent_id, to, subject, content);
        self.mesh.send_direct(msg).await
    }

    /// Sends a direct query to a peer agent and waits for the response.
    pub async fn ask(
        &self,
        to: &str,
        query: &str,
        context: Option<String>,
        timeout_dur: Option<Duration>,
    ) -> Result<PeerResponse, MeshError> {
        self.mesh
            .query_peer(&self.agent_id, to, query, context, timeout_dur)
            .await
    }

    /// Submits a proposal, plan, or diff for advisor review.
    pub async fn request_review(
        &self,
        subject: &str,
        diff_or_plan: &str,
        advisor: Option<&str>,
    ) -> Result<AdvisorReviewResponse, MeshError> {
        let mut req = AdvisorReviewRequest::new(&self.agent_id, subject, diff_or_plan);
        if let Some(adv) = advisor {
            req = req.with_advisor(adv);
        }
        self.mesh.request_advisor_review(req).await
    }

    /// Claims exclusive ownership of a file or resource.
    pub async fn claim_resource(
        &self,
        resource: &str,
        ttl: Option<Duration>,
    ) -> Result<(), MeshError> {
        self.mesh
            .try_claim_resource(&self.agent_id, resource, ttl)
            .await
    }

    /// Releases ownership of a file or resource.
    pub async fn release_resource(&self, resource: &str) -> Result<bool, MeshError> {
        self.mesh.release_resource(&self.agent_id, resource).await
    }

    /// Records or updates a fact on the shared blackboard.
    pub async fn set_fact(&self, key: &str, value: Value) -> Result<u64, MeshError> {
        self.mesh.set_shared_fact(&self.agent_id, key, value).await
    }

    /// Reads a fact from the shared blackboard.
    pub async fn get_fact(&self, key: &str) -> Option<SharedFact> {
        self.mesh.get_shared_fact(key).await
    }

    /// Awaits the next direct message sent to this agent.
    pub async fn recv_direct(&mut self) -> Option<DirectMessage> {
        self.direct_rx.recv().await
    }

    /// Awaits the next incoming query envelope sent to this agent.
    pub async fn recv_query(&mut self) -> Option<PeerQueryEnvelope> {
        self.query_rx.recv().await
    }

    /// Awaits the next broadcast message.
    pub async fn recv_broadcast(
        &mut self,
    ) -> Result<BroadcastMessage, broadcast::error::RecvError> {
        self.broadcast_rx.recv().await
    }

    /// Deregisters this agent and cleans up its mailboxes.
    pub async fn unregister(self) -> Result<(), MeshError> {
        self.mesh.unregister(&self.agent_id).await
    }
}

// ============================================================================
// LLM Tools for Agent Tool Loops
// ============================================================================

/// Tool enabling an LLM agent to broadcast status updates or discoveries across the mesh.
pub struct MeshBroadcastTool {
    mesh: AgentMesh,
    agent_id: String,
}

impl MeshBroadcastTool {
    pub fn new(mesh: AgentMesh, agent_id: impl Into<String>) -> Self {
        Self {
            mesh,
            agent_id: agent_id.into(),
        }
    }
}

#[async_trait]
impl Tool for MeshBroadcastTool {
    fn name(&self) -> &str {
        "mesh_broadcast"
    }

    fn description(&self) -> &str {
        "Broadcast a status update, finding, or discovery to all peer agents in the coordination mesh."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "topic": {
                    "type": "string",
                    "enum": ["status", "discovery", "alert", "coordination"],
                    "description": "The broadcast topic"
                },
                "message": {
                    "type": "string",
                    "description": "Descriptive message or findings to broadcast"
                },
                "file_references": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "Optional file paths referenced by this finding"
                }
            },
            "required": ["topic", "message"]
        })
    }

    async fn execute(&self, args: Value, _ctx: &ToolContext) -> anyhow::Result<String> {
        let topic = args
            .get("topic")
            .and_then(|v| v.as_str())
            .unwrap_or("status");
        let message = args.get("message").and_then(|v| v.as_str()).unwrap_or("");
        let file_refs: Vec<String> = args
            .get("file_references")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(ToString::to_string))
                    .collect()
            })
            .unwrap_or_default();

        let payload = match topic {
            "discovery" => BroadcastPayload::Discovery {
                topic: "code_inspection".to_string(),
                findings: message.to_string(),
                file_references: file_refs,
            },
            "alert" => BroadcastPayload::Alert {
                severity: "warning".to_string(),
                message: message.to_string(),
            },
            _ => BroadcastPayload::Status {
                status: AgentStatus::Active {
                    task: message.to_string(),
                },
            },
        };

        let msg = BroadcastMessage::new(&self.agent_id, topic, payload);
        self.mesh.broadcast(msg).await?;
        Ok(format!(
            "Broadcast dispatched on topic '{topic}': {message}"
        ))
    }
}

/// Tool enabling an LLM agent to directly query a peer agent and receive an answer.
pub struct MeshQueryPeerTool {
    mesh: AgentMesh,
    agent_id: String,
}

impl MeshQueryPeerTool {
    pub fn new(mesh: AgentMesh, agent_id: impl Into<String>) -> Self {
        Self {
            mesh,
            agent_id: agent_id.into(),
        }
    }
}

#[async_trait]
impl Tool for MeshQueryPeerTool {
    fn name(&self) -> &str {
        "mesh_query_peer"
    }

    fn description(&self) -> &str {
        "Send a direct question or query to another active agent on the mesh and await their response."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "peer_id": {
                    "type": "string",
                    "description": "The target agent ID to query (e.g. 'Scout', 'Reviewer', 'Coder-1')"
                },
                "query": {
                    "type": "string",
                    "description": "The specific question or request for the peer"
                },
                "context": {
                    "type": "string",
                    "description": "Optional context or code snippet to assist the peer"
                },
                "timeout_secs": {
                    "type": "integer",
                    "description": "Optional timeout in seconds (defaults to 30)"
                }
            },
            "required": ["peer_id", "query"]
        })
    }

    async fn execute(&self, args: Value, _ctx: &ToolContext) -> anyhow::Result<String> {
        let peer_id = args
            .get("peer_id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("Missing 'peer_id' argument"))?;
        let query = args
            .get("query")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("Missing 'query' argument"))?;
        let context = args
            .get("context")
            .and_then(|v| v.as_str())
            .map(ToString::to_string);
        let timeout_secs = args
            .get("timeout_secs")
            .and_then(|v| v.as_u64())
            .unwrap_or(30);

        let response = self
            .mesh
            .query_peer(
                &self.agent_id,
                peer_id,
                query,
                context,
                Some(Duration::from_secs(timeout_secs)),
            )
            .await?;

        if response.success {
            Ok(format!(
                "Peer '{}' replied:\n{}",
                response.from, response.answer
            ))
        } else {
            Ok(format!(
                "Peer '{}' reported failure:\n{}",
                response.from, response.answer
            ))
        }
    }
}

/// Tool enabling an LLM agent to request an architectural or security advisor review.
pub struct MeshRequestReviewTool {
    mesh: AgentMesh,
    agent_id: String,
}

impl MeshRequestReviewTool {
    pub fn new(mesh: AgentMesh, agent_id: impl Into<String>) -> Self {
        Self {
            mesh,
            agent_id: agent_id.into(),
        }
    }
}

#[async_trait]
impl Tool for MeshRequestReviewTool {
    fn name(&self) -> &str {
        "mesh_request_review"
    }

    fn description(&self) -> &str {
        "Request an architectural, security, or code quality critique from the advisor system before executing significant changes."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "subject": {
                    "type": "string",
                    "description": "Brief description of what is being reviewed (e.g. 'Database migration plan', 'Delete auth route')"
                },
                "diff_or_plan": {
                    "type": "string",
                    "description": "The planned command, code diff, or architectural design to be reviewed"
                },
                "advisor": {
                    "type": "string",
                    "enum": ["ArchitectureAdvisor", "SecurityAdvisor", "CodeReviewAdvisor", "all"],
                    "description": "Optional specific advisor to consult (defaults to all)"
                }
            },
            "required": ["subject", "diff_or_plan"]
        })
    }

    async fn execute(&self, args: Value, _ctx: &ToolContext) -> anyhow::Result<String> {
        let subject = args
            .get("subject")
            .and_then(|v| v.as_str())
            .unwrap_or("Proposed change");
        let diff_or_plan = args
            .get("diff_or_plan")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let advisor = args
            .get("advisor")
            .and_then(|v| v.as_str())
            .filter(|&a| a != "all");

        let mut req = AdvisorReviewRequest::new(&self.agent_id, subject, diff_or_plan);
        if let Some(adv) = advisor {
            req = req.with_advisor(adv);
        }

        let resp = self.mesh.request_advisor_review(req).await?;
        let status_str = if resp.approved {
            "✓ REVIEW APPROVED"
        } else {
            "✗ REVIEW REJECTED"
        };

        Ok(format!(
            "{status_str} (Highest Risk: {})\n\nCritiques:\n{}",
            resp.highest_risk, resp.summary
        ))
    }
}

/// Tool enabling an LLM agent to inspect active peers in the mesh.
pub struct MeshListPeersTool {
    mesh: AgentMesh,
}

impl MeshListPeersTool {
    pub fn new(mesh: AgentMesh) -> Self {
        Self { mesh }
    }
}

#[async_trait]
impl Tool for MeshListPeersTool {
    fn name(&self) -> &str {
        "mesh_list_peers"
    }

    fn description(&self) -> &str {
        "List all active subagents and advisors registered on the mesh along with their roles and status."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {}
        })
    }

    async fn execute(&self, _args: Value, _ctx: &ToolContext) -> anyhow::Result<String> {
        let peers = self.mesh.list_peers().await;
        if peers.is_empty() {
            return Ok("No peers currently registered in the mesh.".to_string());
        }

        let mut lines = Vec::new();
        lines.push(format!("Active Peers (Total: {}):", peers.len()));
        for p in peers {
            lines.push(format!("- {} [{}] -> Status: {}", p.id, p.role, p.status));
            if !p.description.is_empty() {
                lines.push(format!("  Description: {}", p.description));
            }
        }

        Ok(lines.join("\n"))
    }
}

/// Tool enabling an LLM agent to claim or release exclusive file locks.
pub struct MeshClaimResourceTool {
    mesh: AgentMesh,
    agent_id: String,
}

impl MeshClaimResourceTool {
    pub fn new(mesh: AgentMesh, agent_id: impl Into<String>) -> Self {
        Self {
            mesh,
            agent_id: agent_id.into(),
        }
    }
}

#[async_trait]
impl Tool for MeshClaimResourceTool {
    fn name(&self) -> &str {
        "mesh_claim_resource"
    }

    fn description(&self) -> &str {
        "Claim or release exclusive ownership of a file or resource to prevent conflicts with concurrent peer agents."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "action": {
                    "type": "string",
                    "enum": ["claim", "release", "list"],
                    "description": "The action to perform: 'claim', 'release', or 'list'"
                },
                "resource": {
                    "type": "string",
                    "description": "The resource or file path (e.g. 'src/agent/mesh.rs')"
                },
                "ttl_secs": {
                    "type": "integer",
                    "description": "Optional lease TTL in seconds for a claim"
                }
            },
            "required": ["action"]
        })
    }

    async fn execute(&self, args: Value, _ctx: &ToolContext) -> anyhow::Result<String> {
        let action = args
            .get("action")
            .and_then(|v| v.as_str())
            .unwrap_or("list");

        match action {
            "claim" => {
                let resource = args
                    .get("resource")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| anyhow::anyhow!("Missing 'resource' argument for claim"))?;
                let ttl = args
                    .get("ttl_secs")
                    .and_then(|v| v.as_u64())
                    .map(Duration::from_secs);

                self.mesh
                    .try_claim_resource(&self.agent_id, resource, ttl)
                    .await?;
                Ok(format!(
                    "Resource '{resource}' successfully claimed by agent '{}'.",
                    self.agent_id
                ))
            }
            "release" => {
                let resource = args
                    .get("resource")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| anyhow::anyhow!("Missing 'resource' argument for release"))?;

                let released = self.mesh.release_resource(&self.agent_id, resource).await?;
                if released {
                    Ok(format!(
                        "Resource '{resource}' successfully released by agent '{}'.",
                        self.agent_id
                    ))
                } else {
                    Ok(format!(
                        "Resource '{resource}' was not currently held by '{}'.",
                        self.agent_id
                    ))
                }
            }
            _ => {
                let claims = self.mesh.get_resource_claims().await;
                if claims.is_empty() {
                    return Ok("No active resource claims on the mesh.".to_string());
                }
                let mut lines = Vec::new();
                lines.push(format!("Active Resource Claims (Total: {}):", claims.len()));
                for (res, claim) in claims {
                    lines.push(format!("- {} (Owner: {})", res, claim.owner));
                }
                Ok(lines.join("\n"))
            }
        }
    }
}

// ============================================================================
// Unit and Integration Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_peer_registration_and_discovery() {
        let mesh = AgentMesh::new();

        let scout = mesh
            .register("Scout-1", AgentRole::Scout, "Exploration agent")
            .await
            .expect("register scout");

        let coder = mesh
            .register("Coder-1", AgentRole::Coder, "Implementation agent")
            .await
            .expect("register coder");

        assert_eq!(scout.agent_id(), "Scout-1");
        assert_eq!(coder.agent_id(), "Coder-1");

        let peers = mesh.list_peers().await;
        assert_eq!(peers.len(), 2);

        let scouts = mesh.find_peers_by_role(&AgentRole::Scout).await;
        assert_eq!(scouts.len(), 1);
        assert_eq!(scouts[0].id, "Scout-1");

        // Duplicate registration should fail
        let dup = mesh
            .register("Scout-1", AgentRole::Scout, "Duplicate")
            .await;
        assert!(matches!(dup, Err(MeshError::PeerAlreadyRegistered(_))));

        // Unregister
        scout.unregister().await.expect("unregister");
        let peers_after = mesh.list_peers().await;
        assert_eq!(peers_after.len(), 1);
        assert_eq!(peers_after[0].id, "Coder-1");
    }

    #[tokio::test]
    async fn test_broadcast_pub_sub() {
        let mesh = AgentMesh::new();

        let _peer_a = mesh
            .register("Agent-A", AgentRole::Scout, "Agent A")
            .await
            .unwrap();
        let mut peer_b = mesh
            .register("Agent-B", AgentRole::Coder, "Agent B")
            .await
            .unwrap();

        // Broadcast discovery from A
        mesh.broadcast_discovery(
            "Agent-A",
            "ast_analysis",
            "Found key parser logic in src/parser.rs",
            vec!["src/parser.rs".to_string()],
        )
        .await
        .unwrap();

        // Peer B receives broadcast
        let received = peer_b.recv_broadcast().await.unwrap();
        assert_eq!(received.sender, "Agent-A");
        assert_eq!(received.topic, topics::DISCOVERY);
        if let BroadcastPayload::Discovery {
            findings,
            file_references,
            ..
        } = received.payload
        {
            assert!(findings.contains("parser logic"));
            assert_eq!(file_references, vec!["src/parser.rs"]);
        } else {
            panic!("Expected discovery payload");
        }
    }

    #[tokio::test]
    async fn test_direct_messaging() {
        let mesh = AgentMesh::new();

        let peer_a = mesh
            .register("Agent-A", AgentRole::Scout, "Agent A")
            .await
            .unwrap();
        let mut peer_b = mesh
            .register("Agent-B", AgentRole::Coder, "Agent B")
            .await
            .unwrap();

        peer_a
            .send_direct(
                "Agent-B",
                "Task Handshake",
                "Ready to hand off file modifications.",
            )
            .await
            .unwrap();

        let msg = peer_b.recv_direct().await.expect("recv direct message");
        assert_eq!(msg.from, "Agent-A");
        assert_eq!(msg.to, "Agent-B");
        assert_eq!(msg.subject, "Task Handshake");
        assert!(msg.content.contains("hand off"));
    }

    #[tokio::test]
    async fn test_direct_query_request_response() {
        let mesh = AgentMesh::new();

        let peer_a = mesh
            .register("Requester", AgentRole::Coder, "Code requester")
            .await
            .unwrap();
        let mut peer_b = mesh
            .register("Scout", AgentRole::Scout, "Code searcher")
            .await
            .unwrap();

        // Spawn responder loop for peer B
        tokio::spawn(async move {
            if let Some(envelope) = peer_b.recv_query().await {
                assert_eq!(envelope.query().from, "Requester");
                assert_eq!(envelope.query().query, "Where is the auth handler defined?");
                envelope
                    .respond("Auth handler is in src/auth/handler.rs:42")
                    .expect("reply");
            }
        });

        // Peer A asks peer B
        let resp = peer_a
            .ask(
                "Scout",
                "Where is the auth handler defined?",
                None,
                Some(Duration::from_secs(5)),
            )
            .await
            .expect("ask response");

        assert!(resp.success);
        assert_eq!(resp.from, "Scout");
        assert_eq!(resp.to, "Requester");
        assert!(resp.answer.contains("src/auth/handler.rs:42"));
    }

    #[tokio::test]
    async fn test_direct_query_timeout() {
        let mesh = AgentMesh::new();

        let peer_a = mesh
            .register("Caller", AgentRole::Coder, "Caller")
            .await
            .unwrap();
        let _peer_b = mesh
            .register("SilentPeer", AgentRole::Scout, "Silent")
            .await
            .unwrap();

        // Peer B does not answer; query should timeout
        let result = peer_a
            .ask(
                "SilentPeer",
                "Hello?",
                None,
                Some(Duration::from_millis(50)),
            )
            .await;

        assert!(matches!(result, Err(MeshError::QueryTimeout { .. })));
    }

    #[tokio::test]
    async fn test_advisor_review_heuristics() {
        let mesh = AgentMesh::new();

        let peer = mesh
            .register("Worker", AgentRole::Coder, "Worker")
            .await
            .unwrap();

        // 1. Safe change
        let safe_resp = peer
            .request_review(
                "Add helper function",
                "pub fn add(a: i32, b: i32) -> i32 { a + b }",
                None,
            )
            .await
            .expect("safe review");

        assert!(safe_resp.approved);
        assert_eq!(safe_resp.highest_risk, RiskLevel::Low);

        // 2. Critical security violation (rm -rf /)
        let dangerous_resp = peer
            .request_review(
                "Cleanup scratch directory",
                "bash::execute: rm -rf /",
                Some("SecurityAdvisor"),
            )
            .await
            .expect("dangerous review");

        assert!(!dangerous_resp.approved);
        assert_eq!(dangerous_resp.highest_risk, RiskLevel::Critical);
        assert!(dangerous_resp.summary.contains("Catastrophic command"));
    }

    #[tokio::test]
    async fn test_resource_claim_coordination() {
        let mesh = AgentMesh::new();

        let peer_a = mesh
            .register("Coder-A", AgentRole::Coder, "Coder A")
            .await
            .unwrap();
        let peer_b = mesh
            .register("Coder-B", AgentRole::Coder, "Coder B")
            .await
            .unwrap();

        let file = "src/agent/mesh.rs";

        // Peer A claims file
        peer_a.claim_resource(file, None).await.expect("claim file");

        // Peer B tries to claim the same file -> should fail with ResourceAlreadyClaimed
        let conflict = peer_b.claim_resource(file, None).await;
        assert!(matches!(
            conflict,
            Err(MeshError::ResourceAlreadyClaimed { claimed_by, .. }) if claimed_by == "Coder-A"
        ));

        // Peer A releases file
        let released = peer_a.release_resource(file).await.expect("release file");
        assert!(released);

        // Now Peer B can claim it
        peer_b
            .claim_resource(file, None)
            .await
            .expect("peer b claim");
    }

    #[tokio::test]
    async fn test_blackboard_shared_facts() {
        let mesh = AgentMesh::new();

        let peer_a = mesh
            .register("Scout-A", AgentRole::Scout, "Scout")
            .await
            .unwrap();

        peer_a
            .set_fact(
                "target_architecture",
                json!({"arch": "arm64", "os": "linux"}),
            )
            .await
            .expect("set fact");

        let fact = peer_a
            .get_fact("target_architecture")
            .await
            .expect("get fact");
        assert_eq!(fact.author, "Scout-A");
        assert_eq!(fact.revision, 1);
        assert_eq!(fact.value["arch"], "arm64");

        // Overwrite fact
        peer_a
            .set_fact(
                "target_architecture",
                json!({"arch": "wasm32", "os": "unknown"}),
            )
            .await
            .expect("update fact");

        let updated = peer_a
            .get_fact("target_architecture")
            .await
            .expect("get fact updated");
        assert_eq!(updated.revision, 2);
        assert_eq!(updated.value["arch"], "wasm32");
    }

    #[tokio::test]
    async fn test_mesh_coordination_barrier() {
        let mesh = AgentMesh::new();

        mesh.create_barrier("sync_phase_1", 3).await;

        let m1 = mesh.clone();
        let m2 = mesh.clone();
        let m3 = mesh.clone();

        let h1 = tokio::spawn(async move {
            m1.wait_barrier("sync_phase_1", "Worker-1", Duration::from_secs(2))
                .await
        });
        let h2 = tokio::spawn(async move {
            m2.wait_barrier("sync_phase_1", "Worker-2", Duration::from_secs(2))
                .await
        });
        let h3 = tokio::spawn(async move {
            m3.wait_barrier("sync_phase_1", "Worker-3", Duration::from_secs(2))
                .await
        });

        let (r1, r2, r3) = tokio::join!(h1, h2, h3);
        assert!(r1.unwrap().is_ok());
        assert!(r2.unwrap().is_ok());
        assert!(r3.unwrap().is_ok());
    }

    #[tokio::test]
    async fn test_mesh_tools_execution() {
        let mesh = AgentMesh::new();
        let ctx = ToolContext::default();

        let _scout = mesh
            .register("Scout", AgentRole::Scout, "Scout")
            .await
            .unwrap();

        // 1. MeshListPeersTool
        let list_tool = MeshListPeersTool::new(mesh.clone());
        let list_out = list_tool.execute(json!({}), &ctx).await.unwrap();
        assert!(list_out.contains("Scout"));

        // 2. MeshBroadcastTool
        let broadcast_tool = MeshBroadcastTool::new(mesh.clone(), "Scout");
        let bcast_out = broadcast_tool
            .execute(
                json!({
                    "topic": "discovery",
                    "message": "Found 12 modules in src/",
                    "file_references": ["src/main.rs"]
                }),
                &ctx,
            )
            .await
            .unwrap();
        assert!(bcast_out.contains("discovery"));

        // 3. MeshClaimResourceTool
        let claim_tool = MeshClaimResourceTool::new(mesh.clone(), "Scout");
        let claim_out = claim_tool
            .execute(
                json!({
                    "action": "claim",
                    "resource": "src/lib.rs"
                }),
                &ctx,
            )
            .await
            .unwrap();
        assert!(claim_out.contains("successfully claimed"));

        // 4. MeshRequestReviewTool
        let review_tool = MeshRequestReviewTool::new(mesh.clone(), "Scout");
        let review_out = review_tool
            .execute(
                json!({
                    "subject": "Add helper function",
                    "diff_or_plan": "pub fn test() {}"
                }),
                &ctx,
            )
            .await
            .unwrap();
        assert!(review_out.contains("REVIEW APPROVED"));
    }
}
