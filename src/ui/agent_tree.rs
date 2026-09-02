//! Unicode Tree Diagram and Hierarchy Renderer for Subagents & Multi-Agent Mesh
//!
//! Provides a visual tree renderer and interactive Ratatui widget for inspecting active and completed
//! subagents, lead coordinators, specialized worker subagents (Scout, Coder, Tester, Reviewer),
//! and Advisors across a multi-agent mesh architecture:
//! - **Hierarchical Tree View**: Visualizes parent-child agent delegations using crisp Unicode box-drawing glyphs (`├──`, `└──`, `│`).
//! - **Role Identification**: Distinct role icons and badges for `Lead Agent` (👑), `Scout` (🔍), `Coder` (⚡), `Tester` (🧪), `Reviewer` (🛡️), `Advisor` (💡), and `General` (🤖).
//! - **Animated Status Indicators**: Dynamic spinning frame (`⠋`, `⠙`, `⠹`, `⠸`, etc.) for `Running` agents, green checkmark (`✓`) for `Completed`, red cross (`✗`) for `Failed`, warning (`⊘`) for `Cancelled`, and hourglass (`⏳`) for `Pending`.
//! - **Token & Execution Metrics**: Tracks prompt/completion tokens, total token usage, execution durations, turn limits, USD costs, and throughput (tokens/sec) per subagent node.
//! - **Interactive TUI Widget**: Ratatui [`Widget`] and [`StatefulWidget`] implementations with collapsible/expandable nodes, search filtering, split-pane inspector panel, and keyboard navigation.
//! - **Multiple Glyph Presets**: Unicode Box, Unicode Rounded, Unicode Bold, Unicode Double, Compact, and ASCII fallback styles.
//! - **ANSI & Plaintext Output**: Standalone ANSI string and plaintext generators for REPL commands, CLI logs, and terminal diagnostics.

use std::collections::{HashMap, HashSet};
use std::fmt;
use std::time::Duration;

use chrono::{DateTime, Utc};
use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use ratatui::{
    buffer::Buffer,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Paragraph, StatefulWidget, Widget, Wrap},
};
use serde::{Deserialize, Serialize};

use crate::agent::subagent::{SubagentInfo, SubagentProgress, SubagentRole, SubagentStatus, SubagentTask};
use crate::ui::spinner::BRAILLE_FRAMES;
use crate::ui::table::{get_terminal_width, visible_width};
use crate::ui::theme::Theme;

// ============================================================================
// 1. Data Structures & Hierarchy Models
// ============================================================================

/// Represents an individual agent or subagent node within the execution hierarchy.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentTreeNode {
    /// Unique agent identifier (e.g. "agent-001", "scout-3", UUID).
    pub id: String,
    /// Human-readable display name (e.g. "Main Coordinator", "Scout-Backend", "CodeReviewer").
    pub name: String,
    /// Specialized functional role.
    pub role: SubagentRole,
    /// Current execution lifecycle status.
    pub status: SubagentStatus,
    /// Assigned task description, goal, or user prompt.
    pub task: String,
    /// Optional parent agent identifier if this agent was spawned as a child delegate.
    pub parent_id: Option<String>,
    /// Child subagents spawned by this agent.
    pub children: Vec<AgentTreeNode>,
    /// Timestamp when this agent was started.
    pub started_at: Option<DateTime<Utc>>,
    /// Timestamp when this agent completed or failed.
    pub completed_at: Option<DateTime<Utc>>,
    /// Measured execution duration.
    pub duration: Option<Duration>,
    /// Current turn index or total completed turns.
    pub turns: usize,
    /// Maximum allowed turns before forced termination.
    pub max_turns: Option<usize>,
    /// Name of the currently executing tool (if running).
    pub current_tool: Option<String>,
    /// Total tokens consumed by this agent (prompt + completion).
    pub tokens_used: Option<usize>,
    /// Prompt tokens consumed.
    pub prompt_tokens: Option<usize>,
    /// Completion tokens consumed.
    pub completion_tokens: Option<usize>,
    /// Estimated cost in USD incurred by this agent.
    pub cost_usd: Option<f64>,
    /// Whether this node is expanded in the tree visualization.
    pub expanded: bool,
    /// Optional category tags (e.g. "lead", "frontend", "security", "fast-path").
    pub tags: Vec<String>,
    /// Custom key-value attributes for diagnostic inspection.
    pub custom_attributes: HashMap<String, String>,
}

impl AgentTreeNode {
    /// Creates a new subagent tree node.
    pub fn new(id: impl Into<String>, name: impl Into<String>, role: SubagentRole, task: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            name: name.into(),
            role,
            status: SubagentStatus::Pending,
            task: task.into(),
            parent_id: None,
            children: Vec::new(),
            started_at: Some(Utc::now()),
            completed_at: None,
            duration: None,
            turns: 0,
            max_turns: None,
            current_tool: None,
            tokens_used: None,
            prompt_tokens: None,
            completion_tokens: None,
            cost_usd: None,
            expanded: true,
            tags: Vec::new(),
            custom_attributes: HashMap::new(),
        }
    }

    /// Creates a Lead / Coordinator agent node.
    pub fn lead(id: impl Into<String>, name: impl Into<String>, task: impl Into<String>) -> Self {
        Self::new(id, name, SubagentRole::General, task)
            .with_tag("lead")
    }

    /// Creates a Scout exploration subagent node.
    pub fn scout(id: impl Into<String>, name: impl Into<String>, task: impl Into<String>) -> Self {
        Self::new(id, name, SubagentRole::Scout, task)
            .with_tag("scout")
    }

    /// Creates a Coder implementation subagent node.
    pub fn coder(id: impl Into<String>, name: impl Into<String>, task: impl Into<String>) -> Self {
        Self::new(id, name, SubagentRole::Coder, task)
            .with_tag("coder")
    }

    /// Creates a Tester verification subagent node.
    pub fn tester(id: impl Into<String>, name: impl Into<String>, task: impl Into<String>) -> Self {
        Self::new(id, name, SubagentRole::Tester, task)
            .with_tag("tester")
    }

    /// Creates a Reviewer / Security subagent node.
    pub fn reviewer(id: impl Into<String>, name: impl Into<String>, task: impl Into<String>) -> Self {
        Self::new(id, name, SubagentRole::Reviewer, task)
            .with_tag("reviewer")
    }

    /// Creates an Advisor critique node.
    pub fn advisor(id: impl Into<String>, name: impl Into<String>, focus: impl Into<String>, task: impl Into<String>) -> Self {
        let focus_str = focus.into();
        Self::new(
            id,
            name,
            SubagentRole::Custom {
                name: format!("Advisor: {focus_str}"),
                prompt: String::new(),
            },
            task,
        )
        .with_tag("advisor")
        .with_attribute("focus", focus_str)
    }

    /// Creates a general-purpose worker subagent node.
    pub fn general(id: impl Into<String>, name: impl Into<String>, task: impl Into<String>) -> Self {
        Self::new(id, name, SubagentRole::General, task)
    }

    /// Sets the node execution status.
    pub fn with_status(mut self, status: SubagentStatus) -> Self {
        self.status = status;
        self
    }

    /// Sets the parent agent identifier.
    pub fn with_parent(mut self, parent_id: impl Into<String>) -> Self {
        self.parent_id = Some(parent_id.into());
        self
    }

    /// Adds a child subagent node.
    pub fn with_child(mut self, child: AgentTreeNode) -> Self {
        self.children.push(child);
        self
    }

    /// Adds multiple child subagent nodes.
    pub fn with_children(mut self, children: Vec<AgentTreeNode>) -> Self {
        self.children.extend(children);
        self
    }

    /// Sets the turn count and optional turn limit.
    pub fn with_turns(mut self, turns: usize, max_turns: Option<usize>) -> Self {
        self.turns = turns;
        self.max_turns = max_turns;
        self
    }

    /// Sets the currently active tool.
    pub fn with_tool(mut self, tool: impl Into<String>) -> Self {
        self.current_tool = Some(tool.into());
        self
    }

    /// Sets the execution duration.
    pub fn with_duration(mut self, duration: Duration) -> Self {
        self.duration = Some(duration);
        self
    }

    /// Sets the token usage.
    pub fn with_tokens(mut self, tokens: usize) -> Self {
        self.tokens_used = Some(tokens);
        self
    }

    /// Sets the detailed prompt and completion token counts.
    pub fn with_token_breakdown(mut self, prompt: usize, completion: usize) -> Self {
        self.prompt_tokens = Some(prompt);
        self.completion_tokens = Some(completion);
        self.tokens_used = Some(prompt + completion);
        self
    }

    /// Sets the estimated cost in USD.
    pub fn with_cost(mut self, cost: f64) -> Self {
        self.cost_usd = Some(cost);
        self
    }

    /// Sets whether the node is initially expanded.
    pub fn with_expanded(mut self, expanded: bool) -> Self {
        self.expanded = expanded;
        self
    }

    /// Adds a category tag.
    pub fn with_tag(mut self, tag: impl Into<String>) -> Self {
        self.tags.push(tag.into());
        self
    }

    /// Adds a custom attribute.
    pub fn with_attribute(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.custom_attributes.insert(key.into(), value.into());
        self
    }

    /// Appends a child subagent node directly.
    pub fn add_child(&mut self, child: AgentTreeNode) {
        self.children.push(child);
    }

    /// Returns `true` if this agent is the lead / root coordinator.
    pub fn is_lead(&self) -> bool {
        self.tags.iter().any(|t| t == "lead") || self.name.to_lowercase().contains("lead") || self.name.to_lowercase().contains("coordinator")
    }

    /// Returns `true` if this agent is an advisor.
    pub fn is_advisor(&self) -> bool {
        self.tags.iter().any(|t| t == "advisor")
            || matches!(&self.role, SubagentRole::Custom { name, .. } if name.to_lowercase().contains("advisor"))
    }

    /// Returns `true` if this agent is actively running.
    pub fn is_running(&self) -> bool {
        matches!(self.status, SubagentStatus::Running { .. })
    }

    /// Returns `true` if this agent completed successfully.
    pub fn is_completed(&self) -> bool {
        matches!(self.status, SubagentStatus::Completed { .. })
    }

    /// Returns `true` if this agent failed with an error.
    pub fn is_failed(&self) -> bool {
        matches!(self.status, SubagentStatus::Failed { .. })
    }

    /// Returns `true` if this agent was cancelled.
    pub fn is_cancelled(&self) -> bool {
        matches!(self.status, SubagentStatus::Cancelled)
    }

    /// Returns `true` if this agent is in pending/queued state.
    pub fn is_pending(&self) -> bool {
        matches!(self.status, SubagentStatus::Pending)
    }

    /// Returns `true` if this agent is active (pending or running).
    pub fn is_active(&self) -> bool {
        self.is_pending() || self.is_running()
    }

    /// Returns a distinct Unicode emoji or icon for this agent's role.
    pub fn role_icon(&self) -> &'static str {
        if self.is_lead() {
            return "👑";
        }
        if self.is_advisor() {
            return "💡";
        }
        match &self.role {
            SubagentRole::Scout => "🔍",
            SubagentRole::Coder => "⚡",
            SubagentRole::Tester => "🧪",
            SubagentRole::Reviewer => "🛡️",
            SubagentRole::General => "🤖",
            SubagentRole::Custom { .. } => "✨",
        }
    }

    /// Returns a short label bracket for the role.
    pub fn role_badge(&self) -> String {
        if self.is_lead() {
            return "[Lead]".to_string();
        }
        if self.is_advisor() {
            return "[Advisor]".to_string();
        }
        match &self.role {
            SubagentRole::Scout => "[Scout]".to_string(),
            SubagentRole::Coder => "[Coder]".to_string(),
            SubagentRole::Tester => "[Tester]".to_string(),
            SubagentRole::Reviewer => "[Reviewer]".to_string(),
            SubagentRole::General => "[Worker]".to_string(),
            SubagentRole::Custom { name, .. } => format!("[{name}]"),
        }
    }

    /// Returns a static status indicator symbol.
    pub fn status_icon(&self) -> &'static str {
        match &self.status {
            SubagentStatus::Pending => "⏳",
            SubagentStatus::Running { .. } => "⚡",
            SubagentStatus::Completed { .. } => "✓",
            SubagentStatus::Failed { .. } => "✗",
            SubagentStatus::Cancelled => "⊘",
        }
    }

    /// Returns an animated status indicator symbol based on animation tick index.
    pub fn animated_status_icon(&self, tick: usize) -> &'static str {
        match &self.status {
            SubagentStatus::Pending => "⏳",
            SubagentStatus::Running { .. } => {
                if BRAILLE_FRAMES.is_empty() {
                    "⚡"
                } else {
                    BRAILLE_FRAMES[tick % BRAILLE_FRAMES.len()]
                }
            }
            SubagentStatus::Completed { .. } => "✓",
            SubagentStatus::Failed { .. } => "✗",
            SubagentStatus::Cancelled => "⊘",
        }
    }

    /// Returns a short uppercase status label.
    pub fn status_label(&self) -> &'static str {
        match &self.status {
            SubagentStatus::Pending => "PENDING",
            SubagentStatus::Running { .. } => "RUNNING",
            SubagentStatus::Completed { .. } => "DONE",
            SubagentStatus::Failed { .. } => "FAILED",
            SubagentStatus::Cancelled => "CANCELLED",
        }
    }

    /// Formats a concise status summary string including turns and active tools.
    pub fn status_summary(&self) -> String {
        match &self.status {
            SubagentStatus::Pending => "queued".to_string(),
            SubagentStatus::Running { turn, current_tool } => {
                let tool_part = current_tool
                    .as_deref()
                    .or(self.current_tool.as_deref())
                    .map(|t| format!(" tool: {t}"))
                    .unwrap_or_default();
                if let Some(max) = self.max_turns {
                    format!("turn {turn}/{max}{tool_part}")
                } else {
                    format!("turn {turn}{tool_part}")
                }
            }
            SubagentStatus::Completed { turns, .. } => {
                let dur_part = self
                    .effective_duration()
                    .map(|d| format!(", {:.1}s", d.as_secs_f32()))
                    .unwrap_or_default();
                format!("{turns} turns{dur_part}")
            }
            SubagentStatus::Failed { error } => {
                let err_preview: String = error.chars().take(30).collect();
                if error.chars().count() > 30 {
                    format!("err: {err_preview}…")
                } else {
                    format!("err: {err_preview}")
                }
            }
            SubagentStatus::Cancelled => "cancelled".to_string(),
        }
    }

    /// Computes effective execution duration from explicit duration or timestamps.
    pub fn effective_duration(&self) -> Option<Duration> {
        if let Some(d) = self.duration {
            return Some(d);
        }
        if let (Some(start), Some(end)) = (self.started_at, self.completed_at) {
            if let Ok(d) = end.signed_duration_since(start).to_std() {
                return Some(d);
            }
        } else if let Some(start) = self.started_at {
            if self.is_running() {
                if let Ok(d) = Utc::now().signed_duration_since(start).to_std() {
                    return Some(d);
                }
            }
        }
        None
    }

    /// Formats the execution duration into a clean string representation.
    pub fn formatted_duration(&self) -> Option<String> {
        self.effective_duration().map(|dur| {
            if dur.as_secs() >= 60 {
                format!("{}m {}s", dur.as_secs() / 60, dur.as_secs() % 60)
            } else if dur.as_millis() < 1000 {
                format!("{}ms", dur.as_millis())
            } else {
                format!("{:.1}s", dur.as_secs_f32())
            }
        })
    }

    /// Formats the token usage count.
    pub fn formatted_tokens(&self) -> Option<String> {
        self.tokens_used.map(|tok| {
            if tok >= 1_000_000 {
                format!("{:.1}M tok", tok as f64 / 1_000_000.0)
            } else if tok >= 1_000 {
                format!("{:.1}k tok", tok as f64 / 1_000.0)
            } else {
                format!("{tok} tok")
            }
        })
    }

    /// Calculates token throughput per second if duration is known and > 0.
    pub fn tokens_per_second(&self) -> Option<f64> {
        if let (Some(tok), Some(dur)) = (self.tokens_used, self.effective_duration()) {
            let secs = dur.as_secs_f64();
            if secs > 0.05 {
                return Some(tok as f64 / secs);
            }
        }
        None
    }

    /// Formats performance metrics (tokens, duration, cost) into a compact pill.
    pub fn metrics_summary(&self) -> Option<String> {
        let mut parts = Vec::new();
        if let Some(tok_str) = self.formatted_tokens() {
            parts.push(tok_str);
        }
        if let Some(dur_str) = self.formatted_duration() {
            parts.push(dur_str);
        }
        if let Some(cost) = self.cost_usd {
            if cost > 0.0 {
                parts.push(format!("${:.3}", cost));
            }
        }

        if parts.is_empty() {
            None
        } else {
            Some(format!("[{}]", parts.join(" | ")))
        }
    }

    /// Recursively counts all descendant nodes including self.
    pub fn count_all(&self) -> usize {
        1 + self.children.iter().map(|c| c.count_all()).sum::<usize>()
    }
}

// ---------------------------------------------------------------------------
// Conversion from Agent Framework Types
// ---------------------------------------------------------------------------

impl From<&SubagentInfo> for AgentTreeNode {
    fn from(info: &SubagentInfo) -> Self {
        let mut node = AgentTreeNode::new(&info.id, &info.name, info.role.clone(), &info.task)
            .with_status(info.status.clone())
            .with_turns(info.turns, None);

        if let Ok(started) = DateTime::parse_from_rfc3339(&info.started_at) {
            node.started_at = Some(started.with_timezone(&Utc));
        }
        if let Some(completed_str) = &info.completed_at {
            if let Ok(completed) = DateTime::parse_from_rfc3339(completed_str) {
                node.completed_at = Some(completed.with_timezone(&Utc));
                if let Some(started) = node.started_at {
                    if let Ok(dur) = completed.with_timezone(&Utc).signed_duration_since(started).to_std() {
                        node.duration = Some(dur);
                    }
                }
            }
        }

        node
    }
}

impl From<&SubagentTask> for AgentTreeNode {
    fn from(task: &SubagentTask) -> Self {
        AgentTreeNode::new(&task.id, &task.name, task.role.clone(), &task.task)
            .with_turns(0, Some(task.max_turns))
    }
}

// ============================================================================
// 2. Tree Hierarchy Collection & Operations
// ============================================================================

/// Represents a collection of agent execution trees (a single root coordinator or a forest of roots).
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct AgentTree {
    /// Root-level agent nodes.
    pub roots: Vec<AgentTreeNode>,
    /// Optional overall tree title or session identifier.
    pub title: Option<String>,
}

impl AgentTree {
    /// Creates an empty agent tree.
    pub fn new() -> Self {
        Self {
            roots: Vec::new(),
            title: None,
        }
    }

    /// Creates an agent tree with a single root coordinator.
    pub fn with_root(root: AgentTreeNode) -> Self {
        Self {
            roots: vec![root],
            title: None,
        }
    }

    /// Creates an agent tree with multiple root agents.
    pub fn with_roots(roots: Vec<AgentTreeNode>) -> Self {
        Self {
            roots,
            title: None,
        }
    }

    /// Sets the tree title.
    pub fn with_title(mut self, title: impl Into<String>) -> Self {
        self.title = Some(title.into());
        self
    }

    /// Reconstructs a hierarchical tree from a flat list of nodes using `parent_id` pointers.
    ///
    /// If nodes have unresolved or cyclic parent references, they are safely promoted to root nodes.
    pub fn from_flat_nodes(nodes: Vec<AgentTreeNode>) -> Self {
        let mut children_map: HashMap<String, Vec<AgentTreeNode>> = HashMap::new();
        let mut root_candidates: Vec<AgentTreeNode> = Vec::new();

        // 1. Separate roots and index child candidates
        for node in nodes {
            if let Some(parent) = &node.parent_id {
                children_map.entry(parent.clone()).or_default().push(node);
            } else {
                root_candidates.push(node);
            }
        }

        // 2. Recursive helper to attach children from the map
        fn attach_children(node: &mut AgentTreeNode, map: &mut HashMap<String, Vec<AgentTreeNode>>) {
            if let Some(mut kids) = map.remove(&node.id) {
                for kid in &mut kids {
                    attach_children(kid, map);
                }
                node.children.extend(kids);
            }
        }

        for root in &mut root_candidates {
            attach_children(root, &mut children_map);
        }

        // 3. Any leftover nodes in `children_map` whose parents were missing are promoted to roots
        for (_, orphaned_kids) in children_map {
            for mut orphan in orphaned_kids {
                orphan.parent_id = None;
                root_candidates.push(orphan);
            }
        }

        Self {
            roots: root_candidates,
            title: None,
        }
    }

    /// Builds a tree from a slice of `SubagentInfo` snapshots.
    pub fn from_subagent_infos(infos: &[SubagentInfo]) -> Self {
        let nodes: Vec<AgentTreeNode> = infos.iter().map(AgentTreeNode::from).collect();
        Self::from_flat_nodes(nodes)
    }

    /// Finds a reference to a node by its unique ID anywhere in the hierarchy.
    pub fn find_node(&self, id: &str) -> Option<&AgentTreeNode> {
        fn search<'a>(node: &'a AgentTreeNode, id: &str) -> Option<&'a AgentTreeNode> {
            if node.id == id {
                return Some(node);
            }
            for child in &node.children {
                if let Some(found) = search(child, id) {
                    return Some(found);
                }
            }
            None
        }

        for root in &self.roots {
            if let Some(found) = search(root, id) {
                return Some(found);
            }
        }
        None
    }

    /// Finds a mutable reference to a node by its unique ID.
    pub fn find_node_mut(&mut self, id: &str) -> Option<&mut AgentTreeNode> {
        fn search_mut<'a>(node: &'a mut AgentTreeNode, id: &str) -> Option<&'a mut AgentTreeNode> {
            if node.id == id {
                return Some(node);
            }
            for child in &mut node.children {
                if let Some(found) = search_mut(child, id) {
                    return Some(found);
                }
            }
            None
        }

        for root in &mut self.roots {
            if let Some(found) = search_mut(root, id) {
                return Some(found);
            }
        }
        None
    }

    /// Adds a node under the given parent ID, or as a new root if `parent_id` is None or not found.
    pub fn add_node(&mut self, parent_id: Option<&str>, node: AgentTreeNode) -> bool {
        if let Some(p_id) = parent_id {
            if let Some(parent) = self.find_node_mut(p_id) {
                parent.children.push(node);
                return true;
            }
        }
        self.roots.push(node);
        false
    }

    /// Removes a node by ID and returns it if found.
    pub fn remove_node(&mut self, id: &str) -> Option<AgentTreeNode> {
        if let Some(pos) = self.roots.iter().position(|r| r.id == id) {
            return Some(self.roots.remove(pos));
        }

        fn remove_child(parent: &mut AgentTreeNode, id: &str) -> Option<AgentTreeNode> {
            if let Some(pos) = parent.children.iter().position(|c| c.id == id) {
                return Some(parent.children.remove(pos));
            }
            for child in &mut parent.children {
                if let Some(removed) = remove_child(child, id) {
                    return Some(removed);
                }
            }
            None
        }

        for root in &mut self.roots {
            if let Some(removed) = remove_child(root, id) {
                return Some(removed);
            }
        }
        None
    }

    /// Updates the status of an agent by ID. Returns `true` if the node was found.
    pub fn update_status(&mut self, id: &str, status: SubagentStatus) -> bool {
        if let Some(node) = self.find_node_mut(id) {
            node.status = status;
            true
        } else {
            false
        }
    }

    /// Updates the currently active tool of an agent by ID.
    pub fn update_tool(&mut self, id: &str, tool: Option<String>) -> bool {
        if let Some(node) = self.find_node_mut(id) {
            node.current_tool = tool;
            true
        } else {
            false
        }
    }

    /// Updates the node according to a received `SubagentProgress` lifecycle event.
    pub fn update_progress(&mut self, progress: &SubagentProgress) -> bool {
        let id = progress.id();
        let Some(node) = self.find_node_mut(id) else {
            return false;
        };

        match progress {
            SubagentProgress::Started { name, role, task, .. } => {
                node.name = name.clone();
                node.role = role.clone();
                node.task = task.clone();
                node.status = SubagentStatus::Running {
                    turn: 1,
                    current_tool: None,
                };
                node.started_at = Some(Utc::now());
            }
            SubagentProgress::TurnStarted { turn, max_turns, .. } => {
                node.turns = *turn;
                node.max_turns = Some(*max_turns);
                node.status = SubagentStatus::Running {
                    turn: *turn,
                    current_tool: node.current_tool.clone(),
                };
            }
            SubagentProgress::ToolStarted { tool, .. } => {
                node.current_tool = Some(tool.clone());
                if let SubagentStatus::Running { turn, .. } = node.status {
                    node.status = SubagentStatus::Running {
                        turn,
                        current_tool: Some(tool.clone()),
                    };
                }
            }
            SubagentProgress::ToolCompleted { .. } => {
                node.current_tool = None;
                if let SubagentStatus::Running { turn, .. } = node.status {
                    node.status = SubagentStatus::Running {
                        turn,
                        current_tool: None,
                    };
                }
            }
            SubagentProgress::Completed { output, turns_taken, .. } => {
                node.current_tool = None;
                node.turns = *turns_taken;
                node.completed_at = Some(Utc::now());
                if let Some(started) = node.started_at {
                    if let Ok(d) = Utc::now().signed_duration_since(started).to_std() {
                        node.duration = Some(d);
                    }
                }
                node.status = SubagentStatus::Completed {
                    output: output.clone(),
                    turns: *turns_taken,
                };
            }
            SubagentProgress::Failed { error, .. } => {
                node.current_tool = None;
                node.completed_at = Some(Utc::now());
                if let Some(started) = node.started_at {
                    if let Ok(d) = Utc::now().signed_duration_since(started).to_std() {
                        node.duration = Some(d);
                    }
                }
                node.status = SubagentStatus::Failed {
                    error: error.clone(),
                };
            }
            SubagentProgress::Cancelled { .. } => {
                node.current_tool = None;
                node.completed_at = Some(Utc::now());
                node.status = SubagentStatus::Cancelled;
            }
            _ => {}
        }
        true
    }

    /// Expands all nodes in the tree.
    pub fn expand_all(&mut self) {
        fn expand(node: &mut AgentTreeNode) {
            node.expanded = true;
            for child in &mut node.children {
                expand(child);
            }
        }
        for root in &mut self.roots {
            expand(root);
        }
    }

    /// Collapses all non-root nodes in the tree.
    pub fn collapse_all(&mut self) {
        fn collapse(node: &mut AgentTreeNode, is_root: bool) {
            if !is_root {
                node.expanded = false;
            }
            for child in &mut node.children {
                collapse(child, false);
            }
        }
        for root in &mut self.roots {
            collapse(root, true);
        }
    }

    /// Toggles the expanded/collapsed state of a node by ID.
    pub fn toggle_collapse(&mut self, id: &str) -> bool {
        if let Some(node) = self.find_node_mut(id) {
            node.expanded = !node.expanded;
            true
        } else {
            false
        }
    }

    /// Total number of agents across all trees.
    pub fn total_agents(&self) -> usize {
        self.roots.iter().map(|r| r.count_all()).sum()
    }

    /// Count of currently running agents.
    pub fn running_count(&self) -> usize {
        self.count_matching(|n| n.is_running())
    }

    /// Count of successfully completed agents.
    pub fn completed_count(&self) -> usize {
        self.count_matching(|n| n.is_completed())
    }

    /// Count of failed agents.
    pub fn failed_count(&self) -> usize {
        self.count_matching(|n| n.is_failed())
    }

    /// Count of pending agents.
    pub fn pending_count(&self) -> usize {
        self.count_matching(|n| n.is_pending())
    }

    /// Total token consumption across all nodes.
    pub fn total_tokens(&self) -> usize {
        fn sum_tokens(node: &AgentTreeNode) -> usize {
            node.tokens_used.unwrap_or(0) + node.children.iter().map(sum_tokens).sum::<usize>()
        }
        self.roots.iter().map(sum_tokens).sum()
    }

    /// Total prompt tokens consumed across all nodes.
    pub fn total_prompt_tokens(&self) -> usize {
        fn sum_prompt(node: &AgentTreeNode) -> usize {
            node.prompt_tokens.unwrap_or(0) + node.children.iter().map(sum_prompt).sum::<usize>()
        }
        self.roots.iter().map(sum_prompt).sum()
    }

    /// Total completion tokens consumed across all nodes.
    pub fn total_completion_tokens(&self) -> usize {
        fn sum_comp(node: &AgentTreeNode) -> usize {
            node.completion_tokens.unwrap_or(0) + node.children.iter().map(sum_comp).sum::<usize>()
        }
        self.roots.iter().map(sum_comp).sum()
    }

    /// Total cumulative execution duration across all nodes.
    pub fn total_duration(&self) -> Duration {
        fn sum_dur(node: &AgentTreeNode) -> Duration {
            let self_dur = node.effective_duration().unwrap_or(Duration::ZERO);
            self_dur + node.children.iter().map(sum_dur).sum::<Duration>()
        }
        self.roots.iter().map(sum_dur).sum()
    }

    /// Maximum single execution duration among all nodes.
    pub fn max_duration(&self) -> Duration {
        fn max_dur(node: &AgentTreeNode) -> Duration {
            let self_dur = node.effective_duration().unwrap_or(Duration::ZERO);
            let child_max = node.children.iter().map(max_dur).max().unwrap_or(Duration::ZERO);
            self_dur.max(child_max)
        }
        self.roots.iter().map(max_dur).max().unwrap_or(Duration::ZERO)
    }

    /// Total turn count across all nodes.
    pub fn total_turns(&self) -> usize {
        fn sum_turns(node: &AgentTreeNode) -> usize {
            node.turns + node.children.iter().map(sum_turns).sum::<usize>()
        }
        self.roots.iter().map(sum_turns).sum()
    }

    /// Total estimated cost across all nodes.
    pub fn total_cost(&self) -> f64 {
        fn sum_cost(node: &AgentTreeNode) -> f64 {
            node.cost_usd.unwrap_or(0.0) + node.children.iter().map(sum_cost).sum::<f64>()
        }
        self.roots.iter().map(sum_cost).sum()
    }

    /// Helper to count nodes matching a predicate.
    pub fn count_matching(&self, pred: impl Fn(&AgentTreeNode) -> bool) -> usize {
        fn count<'a>(node: &'a AgentTreeNode, pred: &impl Fn(&'a AgentTreeNode) -> bool) -> usize {
            let self_count = if pred(node) { 1 } else { 0 };
            self_count + node.children.iter().map(|c| count(c, pred)).sum::<usize>()
        }
        self.roots.iter().map(|r| count(r, &pred)).sum()
    }

    /// Flattens only visible (expanded) nodes into a linear list of tree rows for rendering.
    pub fn flatten_visible(&self) -> Vec<FlattenedTreeRow> {
        let mut rows = Vec::new();
        let total_roots = self.roots.len();
        for (idx, root) in self.roots.iter().enumerate() {
            let is_last = idx + 1 == total_roots;
            self.flatten_recursive(root, 0, is_last, &[], true, &mut rows);
        }
        rows
    }

    /// Flattens all nodes into a linear list regardless of collapse state.
    pub fn flatten_all(&self) -> Vec<FlattenedTreeRow> {
        let mut rows = Vec::new();
        let total_roots = self.roots.len();
        for (idx, root) in self.roots.iter().enumerate() {
            let is_last = idx + 1 == total_roots;
            self.flatten_recursive(root, 0, is_last, &[], false, &mut rows);
        }
        rows
    }

    fn flatten_recursive(
        &self,
        node: &AgentTreeNode,
        depth: usize,
        is_last_child: bool,
        ancestors_are_last: &[bool],
        respect_expansion: bool,
        output: &mut Vec<FlattenedTreeRow>,
    ) {
        let has_children = !node.children.is_empty();
        output.push(FlattenedTreeRow {
            node: node.clone(),
            depth,
            is_last_child,
            ancestors_are_last: ancestors_are_last.to_vec(),
            has_children,
            is_expanded: node.expanded,
        });

        if !respect_expansion || node.expanded {
            let mut next_ancestors = ancestors_are_last.to_vec();
            next_ancestors.push(is_last_child);

            let total_kids = node.children.len();
            for (idx, child) in node.children.iter().enumerate() {
                let kid_is_last = idx + 1 == total_kids;
                self.flatten_recursive(
                    child,
                    depth + 1,
                    kid_is_last,
                    &next_ancestors,
                    respect_expansion,
                    output,
                );
            }
        }
    }

    /// Searches for agents matching a query string in name, ID, role, task, or tags.
    pub fn search(&self, query: &str) -> Vec<&AgentTreeNode> {
        let q = query.trim().to_lowercase();
        if q.is_empty() {
            return Vec::new();
        }

        let mut matches = Vec::new();
        fn collect<'a>(node: &'a AgentTreeNode, q: &str, matches: &mut Vec<&'a AgentTreeNode>) {
            if node.name.to_lowercase().contains(q)
                || node.id.to_lowercase().contains(q)
                || node.task.to_lowercase().contains(q)
                || node.role.default_name().to_lowercase().contains(q)
                || node.tags.iter().any(|t| t.to_lowercase().contains(q))
                || node.custom_attributes.iter().any(|(k, v)| k.to_lowercase().contains(q) || v.to_lowercase().contains(q))
            {
                matches.push(node);
            }
            for child in &node.children {
                collect(child, q, matches);
            }
        }

        for root in &self.roots {
            collect(root, &q, &mut matches);
        }
        matches
    }

    /// Generates a rich mock multi-agent mesh tree containing Lead Agent, Scout, Coder, Tester, and Advisors.
    pub fn mesh_demo_tree() -> Self {
        // 1. Scout subagent
        let scout = AgentTreeNode::scout("scout-01", "ArchitectureScout", "Scan src/ directory, identify module graph and layout dependencies")
            .with_status(SubagentStatus::Completed {
                output: "Discovered 32 modules with complete cross-references".to_string(),
                turns: 3,
            })
            .with_turns(3, Some(8))
            .with_duration(Duration::from_millis(1250))
            .with_token_breakdown(1800, 1400)
            .with_cost(0.0064);

        // 2. Tester subagent
        let tester = AgentTreeNode::tester("tester-01", "RegressionTester", "Run isolated unit tests for widget buffer rendering and state transitions")
            .with_status(SubagentStatus::Running {
                turn: 2,
                current_tool: Some("bash".to_string()),
            })
            .with_turns(2, Some(10))
            .with_tool("bash")
            .with_token_breakdown(1200, 950)
            .with_cost(0.0045);

        // 3. Reviewer subagent
        let reviewer = AgentTreeNode::reviewer("review-01", "SecurityReviewer", "Audit buffer slicing for out-of-bounds panics and Unicode width alignment")
            .with_status(SubagentStatus::Completed {
                output: "Zero buffer overflows or ANSI injections found".to_string(),
                turns: 2,
            })
            .with_turns(2, Some(5))
            .with_duration(Duration::from_millis(850))
            .with_token_breakdown(2100, 1100)
            .with_cost(0.0058);

        // 4. Coder subagent with nested Tester and Reviewer children
        let coder = AgentTreeNode::coder("coder-01", "WidgetCoder", "Implement Ratatui AgentTreeWidget with animated status spinners and split-pane layout")
            .with_status(SubagentStatus::Running {
                turn: 4,
                current_tool: Some("edit".to_string()),
            })
            .with_turns(4, Some(12))
            .with_tool("edit")
            .with_token_breakdown(4800, 3600)
            .with_cost(0.0195)
            .with_child(tester)
            .with_child(reviewer);

        // 5. Advisors
        let sec_advisor = AgentTreeNode::advisor("adv-sec", "SecurityAdvisor", "Security", "Evaluate execution safety and sanitize tool invocation arguments")
            .with_status(SubagentStatus::Completed {
                output: "Risk assessed: LOW. Tool sandboxing verified.".to_string(),
                turns: 1,
            })
            .with_duration(Duration::from_millis(600))
            .with_token_breakdown(1100, 800)
            .with_cost(0.0035);

        let arch_advisor = AgentTreeNode::advisor("adv-arch", "ArchAdvisor", "Architecture", "Review component boundaries and state transition invariants")
            .with_status(SubagentStatus::Completed {
                output: "Clean separation between State, Widget, and ANSI formatters.".to_string(),
                turns: 1,
            })
            .with_duration(Duration::from_millis(520))
            .with_token_breakdown(950, 750)
            .with_cost(0.0031);

        // 6. Lead Coordinator Agent (Root)
        let lead = AgentTreeNode::lead("lead-coord", "Lead Coordinator", "Orchestrate multi-agent swarm for high-speed parallel development")
            .with_status(SubagentStatus::Running {
                turn: 5,
                current_tool: Some("task".to_string()),
            })
            .with_turns(5, Some(20))
            .with_token_breakdown(9200, 6800)
            .with_cost(0.0385)
            .with_child(scout)
            .with_child(coder)
            .with_child(sec_advisor)
            .with_child(arch_advisor);

        Self::with_root(lead).with_title("Multi-Agent Swarm Execution Mesh")
    }

    /// Backward-compatible alias for `mesh_demo_tree()`.
    pub fn demo_tree() -> Self {
        Self::mesh_demo_tree()
    }
}

// ============================================================================
// 3. Tree Glyphs & Formats
// ============================================================================

/// Flattened representation of a tree node ready for line-by-line rendering.
#[derive(Debug, Clone)]
pub struct FlattenedTreeRow {
    /// Node snapshot data.
    pub node: AgentTreeNode,
    /// Depth level from root (0 = root).
    pub depth: usize,
    /// True if this node is the last child of its parent.
    pub is_last_child: bool,
    /// Boolean flags for each ancestor level: true if ancestor was the last child at that level.
    pub ancestors_are_last: Vec<bool>,
    /// True if this node has child delegates.
    pub has_children: bool,
    /// True if this node is currently expanded.
    pub is_expanded: bool,
}

/// Customizable Unicode and ASCII box-drawing glyph character sets.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct TreeGlyphSet {
    /// Branch connector for intermediate children (`├── `).
    pub branch: &'static str,
    /// Branch connector for the final child of a parent (`└── `).
    pub last_branch: &'static str,
    /// Vertical guide pipe continuing through intermediate children (`│   `).
    pub vertical: &'static str,
    /// Indentation spaces when ancestor guide pipe has terminated (`    `).
    pub space: &'static str,
    /// Indicator badge for expanded parent nodes (`▼ `).
    pub expanded_badge: &'static str,
    /// Indicator badge for collapsed parent nodes (`▶ `).
    pub collapsed_badge: &'static str,
    /// Indicator badge for leaf nodes without children (`● `).
    pub leaf_badge: &'static str,
    /// Horizontal divider bar character (`─`).
    pub horizontal_bar: &'static str,
}

impl Default for TreeGlyphSet {
    fn default() -> Self {
        Self::unicode()
    }
}

impl TreeGlyphSet {
    /// Standard crisp Unicode box-drawing characters (default).
    pub const fn unicode() -> Self {
        Self {
            branch: "├── ",
            last_branch: "└── ",
            vertical: "│   ",
            space: "    ",
            expanded_badge: "▼ ",
            collapsed_badge: "▶ ",
            leaf_badge: "● ",
            horizontal_bar: "─",
        }
    }

    /// Smooth rounded Unicode box-drawing characters.
    pub const fn rounded() -> Self {
        Self {
            branch: "├── ",
            last_branch: "╰── ",
            vertical: "│   ",
            space: "    ",
            expanded_badge: "▾ ",
            collapsed_badge: "▸ ",
            leaf_badge: "• ",
            horizontal_bar: "─",
        }
    }

    /// Heavy bold Unicode box-drawing characters.
    pub const fn bold() -> Self {
        Self {
            branch: "┣━━ ",
            last_branch: "┗━━ ",
            vertical: "┃   ",
            space: "    ",
            expanded_badge: "▼ ",
            collapsed_badge: "▶ ",
            leaf_badge: "◆ ",
            horizontal_bar: "━",
        }
    }

    /// Double-line Unicode box-drawing characters.
    pub const fn double() -> Self {
        Self {
            branch: "╠══ ",
            last_branch: "╚══ ",
            vertical: "║   ",
            space: "    ",
            expanded_badge: "▼ ",
            collapsed_badge: "▶ ",
            leaf_badge: "■ ",
            horizontal_bar: "═",
        }
    }

    /// Pure ASCII fallback glyphs for strict 7-bit environments or raw log files.
    pub const fn ascii() -> Self {
        Self {
            branch: "|-- ",
            last_branch: "\\-- ",
            vertical: "|   ",
            space: "    ",
            expanded_badge: "[-] ",
            collapsed_badge: "[+] ",
            leaf_badge: "* ",
            horizontal_bar: "-",
        }
    }

    /// Ultra-compact glyphs for narrow screens and mobile Termux displays.
    pub const fn compact() -> Self {
        Self {
            branch: "├ ",
            last_branch: "└ ",
            vertical: "│ ",
            space: "  ",
            expanded_badge: "v ",
            collapsed_badge: "> ",
            leaf_badge: "- ",
            horizontal_bar: "─",
        }
    }

    /// Formats the tree guide prefix string for a flattened row.
    pub fn format_prefix(&self, row: &FlattenedTreeRow, show_expansion: bool) -> String {
        let mut prefix = String::new();

        if row.depth == 0 {
            if show_expansion {
                if row.has_children {
                    prefix.push_str(if row.is_expanded {
                        self.expanded_badge
                    } else {
                        self.collapsed_badge
                    });
                } else {
                    prefix.push_str(self.leaf_badge);
                }
            }
            return prefix;
        }

        // Ancestor vertical guides (skip root level at index 0)
        for &ancestor_last in row.ancestors_are_last.iter().skip(1) {
            if ancestor_last {
                prefix.push_str(self.space);
            } else {
                prefix.push_str(self.vertical);
            }
        }

        // Current branch connector
        if row.is_last_child {
            prefix.push_str(self.last_branch);
        } else {
            prefix.push_str(self.branch);
        }

        // Child expand / leaf indicator
        if show_expansion {
            if row.has_children {
                prefix.push_str(if row.is_expanded {
                    self.expanded_badge
                } else {
                    self.collapsed_badge
                });
            } else {
                prefix.push_str(self.leaf_badge);
            }
        }

        prefix
    }

    /// Formats the indent prefix for secondary aligned lines.
    pub fn format_subline_prefix(&self, row: &FlattenedTreeRow) -> String {
        let mut prefix = String::new();

        if row.depth == 0 {
            prefix.push_str(self.space);
            return prefix;
        }

        for &ancestor_last in row.ancestors_are_last.iter().skip(1) {
            if ancestor_last {
                prefix.push_str(self.space);
            } else {
                prefix.push_str(self.vertical);
            }
        }

        if row.is_last_child {
            prefix.push_str(self.space);
        } else {
            prefix.push_str(self.vertical);
        }

        prefix.push_str(self.space);
        prefix
    }
}

// ============================================================================
// 4. Render Configuration & ANSI Formatting
// ============================================================================

/// Rendering options and switches for tree generation.
#[derive(Debug, Clone)]
pub struct TreeRenderOptions {
    /// Glyph preset to use.
    pub glyphs: TreeGlyphSet,
    /// Whether to display assigned task snippets.
    pub show_task: bool,
    /// Maximum character length for inline task truncation.
    pub task_max_length: usize,
    /// Whether to display performance metrics (tokens, duration, cost).
    pub show_metrics: bool,
    /// Whether to display role and status badges.
    pub show_badges: bool,
    /// Whether to display the header title.
    pub show_header: bool,
    /// Whether to display the summary footer line.
    pub show_footer_summary: bool,
    /// Explicit terminal width limit (defaults to automatic terminal width).
    pub max_width: Option<usize>,
    /// Whether to output ANSI color escape codes.
    pub use_colors: bool,
    /// Whether to wrap long task descriptions onto secondary aligned lines.
    pub multiline_task: bool,
    /// Whether to include expansion indicator symbols (`▼`/`▶`).
    pub show_expand_indicators: bool,
    /// Animation tick index for dynamic spinner rendering.
    pub anim_tick: usize,
}

impl Default for TreeRenderOptions {
    fn default() -> Self {
        Self {
            glyphs: TreeGlyphSet::unicode(),
            show_task: true,
            task_max_length: 50,
            show_metrics: true,
            show_badges: true,
            show_header: true,
            show_footer_summary: true,
            max_width: None,
            use_colors: true,
            multiline_task: false,
            show_expand_indicators: true,
            anim_tick: 0,
        }
    }
}

impl TreeRenderOptions {
    /// Creates options configured for plain text output without ANSI colors.
    pub fn plain() -> Self {
        Self {
            use_colors: false,
            ..Default::default()
        }
    }

    /// Creates options configured for compact single-line embedding.
    pub fn compact() -> Self {
        Self {
            glyphs: TreeGlyphSet::compact(),
            show_metrics: false,
            task_max_length: 30,
            show_header: false,
            show_footer_summary: false,
            ..Default::default()
        }
    }

    /// Creates options with multi-line formatted task display.
    pub fn detailed() -> Self {
        Self {
            multiline_task: true,
            task_max_length: 120,
            ..Default::default()
        }
    }

    /// Sets the animation tick frame.
    pub fn with_tick(mut self, tick: usize) -> Self {
        self.anim_tick = tick;
        self
    }
}

// ---------------------------------------------------------------------------
// Standalone Tree Rendering Functions
// ---------------------------------------------------------------------------

/// Renders an `AgentTree` into an ANSI-colored string suitable for terminal display.
pub fn render_tree_ansi(tree: &AgentTree, options: &TreeRenderOptions, theme: &Theme) -> String {
    let mut out = String::new();
    let term_width = options.max_width.unwrap_or_else(get_terminal_width);

    // 1. Header
    if options.show_header {
        let title = tree.title.as_deref().unwrap_or("Active Subagent Hierarchy");
        if options.use_colors {
            out.push_str(&format!(
                "\x1b[1;38;2;{};{};{}m┌─ {} ─{}\x1b[0m\n",
                theme.primary.r(),
                theme.primary.g(),
                theme.primary.b(),
                title,
                "─".repeat(term_width.saturating_sub(visible_width(title) + 5))
            ));
        } else {
            out.push_str(&format!(
                "┌─ {} ─{}\n",
                title,
                "─".repeat(term_width.saturating_sub(title.chars().count() + 5))
            ));
        }
    }

    // 2. Tree rows
    let rows = tree.flatten_visible();
    if rows.is_empty() {
        if options.use_colors {
            out.push_str("  \x1b[2;37m(no active or completed subagents)\x1b[0m\n");
        } else {
            out.push_str("  (no active or completed subagents)\n");
        }
    } else {
        for row in &rows {
            render_tree_row_ansi(row, options, theme, &mut out);
        }
    }

    // 3. Summary Footer
    if options.show_footer_summary && !rows.is_empty() {
        let total = tree.total_agents();
        let running = tree.running_count();
        let completed = tree.completed_count();
        let failed = tree.failed_count();
        let pending = tree.pending_count();
        let tokens = tree.total_tokens();
        let turns = tree.total_turns();
        let duration = tree.max_duration();

        let mut summary_parts = Vec::new();
        if running > 0 {
            let spin = if BRAILLE_FRAMES.is_empty() { "⚡" } else { BRAILLE_FRAMES[options.anim_tick % BRAILLE_FRAMES.len()] };
            summary_parts.push(if options.use_colors {
                format!("\x1b[1;36m{spin} {running} running\x1b[0m")
            } else {
                format!("{spin} {running} running")
            });
        }
        if completed > 0 {
            summary_parts.push(if options.use_colors {
                format!("\x1b[1;32m✓ {completed} done\x1b[0m")
            } else {
                format!("✓ {completed} done")
            });
        }
        if failed > 0 {
            summary_parts.push(if options.use_colors {
                format!("\x1b[1;31m✗ {failed} failed\x1b[0m")
            } else {
                format!("✗ {failed} failed")
            });
        }
        if pending > 0 {
            summary_parts.push(if options.use_colors {
                format!("\x1b[2;37m⏳ {pending} pending\x1b[0m")
            } else {
                format!("⏳ {pending} pending")
            });
        }
        if tokens > 0 {
            let tok_str = if tokens >= 1_000_000 {
                format!("{:.1}M", tokens as f64 / 1_000_000.0)
            } else if tokens >= 1_000 {
                format!("{:.1}k", tokens as f64 / 1_000.0)
            } else {
                tokens.to_string()
            };
            summary_parts.push(if options.use_colors {
                format!("\x1b[1;33m{tok_str} tok\x1b[0m")
            } else {
                format!("{tok_str} tok")
            });
        }
        if duration > Duration::ZERO {
            let dur_str = if duration.as_secs() >= 60 {
                format!("{}m {}s", duration.as_secs() / 60, duration.as_secs() % 60)
            } else {
                format!("{:.1}s", duration.as_secs_f32())
            };
            summary_parts.push(dur_str);
        }
        if turns > 0 {
            summary_parts.push(format!("{turns} turns"));
        }

        let summary_line = format!("Total: {total} agents [{}]", summary_parts.join(" | "));
        if options.use_colors {
            out.push_str(&format!(
                "\x1b[1;38;2;{};{};{}m└─\x1b[0m \x1b[2;37m{}\x1b[0m\n",
                theme.primary.r(),
                theme.primary.g(),
                theme.primary.b(),
                summary_line
            ));
        } else {
            out.push_str(&format!("└─ {}\n", summary_line));
        }
    }

    out
}

/// Renders an `AgentTree` into a plain-text tree string without ANSI escapes.
pub fn render_tree_plain(tree: &AgentTree, options: &TreeRenderOptions) -> String {
    let mut plain_opts = options.clone();
    plain_opts.use_colors = false;
    let dummy_theme = Theme::default();
    render_tree_ansi(tree, &plain_opts, &dummy_theme)
}

/// Quick helper to render a standard Unicode tree diagram.
pub fn render_tree_diagram(tree: &AgentTree) -> String {
    let options = TreeRenderOptions::default();
    let theme = Theme::default();
    render_tree_ansi(tree, &options, &theme)
}

/// Helper to render a single row in ANSI format.
fn render_tree_row_ansi(
    row: &FlattenedTreeRow,
    options: &TreeRenderOptions,
    theme: &Theme,
    out: &mut String,
) {
    let prefix = options.glyphs.format_prefix(row, options.show_expand_indicators);
    let node = &row.node;
    let status_icon = node.animated_status_icon(options.anim_tick);

    if options.use_colors {
        // Guide prefix in muted border color
        out.push_str(&format!(
            "\x1b[38;2;{};{};{}m{}\x1b[0m",
            theme.border.r(),
            theme.border.g(),
            theme.border.b(),
            prefix
        ));

        // Role icon and name
        let (role_r, role_g, role_b) = role_color(&node.role, node.is_lead(), node.is_advisor(), theme);
        out.push_str(&format!(
            "{} \x1b[1;38;2;{};{};{}m{}\x1b[0m ",
            node.role_icon(),
            role_r,
            role_g,
            role_b,
            node.name
        ));

        // Role badge
        if options.show_badges {
            out.push_str(&format!(
                "\x1b[2;38;2;{};{};{}m{}\x1b[0m ",
                role_r,
                role_g,
                role_b,
                node.role_badge()
            ));
        }

        // Status indicator & label & summary
        let (status_r, status_g, status_b) = status_color(&node.status, theme);
        out.push_str(&format!(
            "\x1b[1;38;2;{};{};{}m{} [{}]\x1b[0m \x1b[38;2;{};{};{}m{}\x1b[0m",
            status_r,
            status_g,
            status_b,
            status_icon,
            node.status_label(),
            status_r,
            status_g,
            status_b,
            node.status_summary()
        ));

        // Metrics pill
        if options.show_metrics {
            if let Some(metrics) = node.metrics_summary() {
                out.push_str(&format!(" \x1b[2;37m{metrics}\x1b[0m"));
            }
        }

        // Task description
        if options.show_task && !node.task.is_empty() {
            if options.multiline_task {
                out.push('\n');
                let sub_prefix = options.glyphs.format_subline_prefix(row);
                out.push_str(&format!(
                    "\x1b[38;2;{};{};{}m{}\x1b[0m\x1b[38;2;{};{};{}mTask: {}\x1b[0m",
                    theme.border.r(),
                    theme.border.g(),
                    theme.border.b(),
                    sub_prefix,
                    theme.foreground.r(),
                    theme.foreground.g(),
                    theme.foreground.b(),
                    node.task
                ));
            } else {
                let task_snippet = truncate_str(&node.task, options.task_max_length);
                out.push_str(&format!(
                    " \x1b[2;38;2;{};{};{}m— \"{}\"\x1b[0m",
                    theme.muted.r(),
                    theme.muted.g(),
                    theme.muted.b(),
                    task_snippet
                ));
            }
        }
    } else {
        // Plain text rendering without ANSI escape sequences
        out.push_str(&prefix);
        out.push_str(node.role_icon());
        out.push(' ');
        out.push_str(&node.name);
        out.push(' ');

        if options.show_badges {
            out.push_str(&node.role_badge());
            out.push(' ');
        }

        out.push_str(status_icon);
        out.push(' ');
        out.push('[');
        out.push_str(node.status_label());
        out.push_str("] ");
        out.push_str(&node.status_summary());

        if options.show_metrics {
            if let Some(metrics) = node.metrics_summary() {
                out.push(' ');
                out.push_str(&metrics);
            }
        }

        if options.show_task && !node.task.is_empty() {
            if options.multiline_task {
                out.push('\n');
                let sub_prefix = options.glyphs.format_subline_prefix(row);
                out.push_str(&sub_prefix);
                out.push_str("Task: ");
                out.push_str(&node.task);
            } else {
                let task_snippet = truncate_str(&node.task, options.task_max_length);
                out.push_str(" — \"");
                out.push_str(&task_snippet);
                out.push('"');
            }
        }
    }
    out.push('\n');
}

// ---------------------------------------------------------------------------
// Color Resolution Helpers
// ---------------------------------------------------------------------------

trait ColorRgbExt {
    fn r(&self) -> u8;
    fn g(&self) -> u8;
    fn b(&self) -> u8;
}

impl ColorRgbExt for Color {
    fn r(&self) -> u8 {
        match self {
            Color::Rgb(r, _, _) => *r,
            Color::Red => 255,
            Color::Green => 0,
            Color::Yellow => 255,
            Color::Blue => 0,
            Color::Magenta => 255,
            Color::Cyan => 0,
            Color::White => 255,
            _ => 180,
        }
    }

    fn g(&self) -> u8 {
        match self {
            Color::Rgb(_, g, _) => *g,
            Color::Red => 0,
            Color::Green => 255,
            Color::Yellow => 255,
            Color::Blue => 0,
            Color::Magenta => 0,
            Color::Cyan => 255,
            Color::White => 255,
            _ => 180,
        }
    }

    fn b(&self) -> u8 {
        match self {
            Color::Rgb(_, _, b) => *b,
            Color::Red => 0,
            Color::Green => 0,
            Color::Yellow => 0,
            Color::Blue => 255,
            Color::Magenta => 255,
            Color::Cyan => 255,
            Color::White => 255,
            _ => 180,
        }
    }
}

fn role_color(role: &SubagentRole, is_lead: bool, is_advisor: bool, theme: &Theme) -> (u8, u8, u8) {
    if is_lead {
        return (theme.primary.r(), theme.primary.g(), theme.primary.b());
    }
    if is_advisor {
        return (theme.advisor.r(), theme.advisor.g(), theme.advisor.b());
    }
    match role {
        SubagentRole::Scout => (theme.info.r(), theme.info.g(), theme.info.b()),
        SubagentRole::Coder => (theme.primary.r(), theme.primary.g(), theme.primary.b()),
        SubagentRole::Tester => (theme.accent.r(), theme.accent.g(), theme.accent.b()),
        SubagentRole::Reviewer => (theme.warning.r(), theme.warning.g(), theme.warning.b()),
        SubagentRole::General => (theme.foreground.r(), theme.foreground.g(), theme.foreground.b()),
        SubagentRole::Custom { .. } => (theme.secondary.r(), theme.secondary.g(), theme.secondary.b()),
    }
}

fn status_color(status: &SubagentStatus, theme: &Theme) -> (u8, u8, u8) {
    match status {
        SubagentStatus::Pending => (theme.muted.r(), theme.muted.g(), theme.muted.b()),
        SubagentStatus::Running { .. } => (theme.info.r(), theme.info.g(), theme.info.b()),
        SubagentStatus::Completed { .. } => (theme.success.r(), theme.success.g(), theme.success.b()),
        SubagentStatus::Failed { .. } => (theme.error.r(), theme.error.g(), theme.error.b()),
        SubagentStatus::Cancelled => (theme.warning.r(), theme.warning.g(), theme.warning.b()),
    }
}

fn truncate_str(s: &str, max_chars: usize) -> String {
    if max_chars == 0 {
        return String::new();
    }
    let chars: Vec<char> = s.chars().collect();
    if chars.len() <= max_chars {
        s.to_string()
    } else {
        let snippet: String = chars[..max_chars.saturating_sub(1)].iter().collect();
        format!("{snippet}…")
    }
}

// ============================================================================
// 5. Interactive Ratatui Widget & State
// ============================================================================

/// Presentation layout mode for the Ratatui tree widget.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum TreeViewMode {
    /// Pure hierarchical tree diagram occupying the full widget area.
    #[default]
    TreeOnly,
    /// Split view: tree hierarchy on the left (55%), selected agent inspector on the right (45%).
    SplitWithDetails,
    /// Ultra-compact single-line per node mode for embedding inside dashboards.
    Compact,
}

impl TreeViewMode {
    /// Cycles to the next presentation mode.
    pub fn next(&self) -> Self {
        match self {
            Self::TreeOnly => Self::SplitWithDetails,
            Self::SplitWithDetails => Self::Compact,
            Self::Compact => Self::TreeOnly,
        }
    }
}

/// Action resulting from interactive keyboard handling.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AgentTreeAction {
    /// No operation / key consumed.
    None,
    /// An agent node was selected or clicked.
    Selected(String),
    /// A node fold state was toggled.
    ToggledFold(String),
    /// View mode was changed.
    ViewModeChanged(TreeViewMode),
    /// Search filter was updated.
    SearchQueryChanged(String),
    /// Exit / close requested.
    Close,
}

/// Interactive state for the `AgentTreeWidget`.
#[derive(Debug, Clone, Default)]
pub struct AgentTreeState {
    /// Currently selected row index in the flattened visible list.
    pub selected_index: usize,
    /// Vertical scroll offset for tree list.
    pub scroll_offset: usize,
    /// Active search / filter query string.
    pub search_query: String,
    /// Current view presentation mode.
    pub view_mode: TreeViewMode,
    /// Manually collapsed node IDs.
    pub collapsed_ids: HashSet<String>,
    /// Dynamic animation frame / tick counter for running status spinners.
    pub tick: usize,
}

impl AgentTreeState {
    /// Creates a new initial tree state.
    pub fn new() -> Self {
        Self::default()
    }

    /// Advances the animation tick by 1.
    pub fn tick(&mut self) {
        self.tick = self.tick.wrapping_add(1);
    }

    /// Sets the animation tick counter.
    pub fn with_tick(mut self, tick: usize) -> Self {
        self.tick = tick;
        self
    }

    /// Returns the active Braille spinner frame corresponding to the current state tick.
    pub fn spinner_frame(&self) -> &'static str {
        if BRAILLE_FRAMES.is_empty() {
            "⚡"
        } else {
            BRAILLE_FRAMES[self.tick % BRAILLE_FRAMES.len()]
        }
    }

    /// Selects the next visible row.
    pub fn select_next(&mut self, total_rows: usize) {
        if total_rows > 0 {
            if self.selected_index + 1 < total_rows {
                self.selected_index += 1;
            }
        }
    }

    /// Selects the previous visible row.
    pub fn select_prev(&mut self) {
        if self.selected_index > 0 {
            self.selected_index -= 1;
        }
    }

    /// Selects the first row.
    pub fn select_first(&mut self) {
        self.selected_index = 0;
        self.scroll_offset = 0;
    }

    /// Selects the last visible row.
    pub fn select_last(&mut self, total_rows: usize) {
        if total_rows > 0 {
            self.selected_index = total_rows.saturating_sub(1);
        }
    }

    /// Scrolls down by a page.
    pub fn scroll_page_down(&mut self, page_size: usize, total_rows: usize) {
        if total_rows > 0 {
            self.selected_index = (self.selected_index + page_size).min(total_rows.saturating_sub(1));
        }
    }

    /// Scrolls up by a page.
    pub fn scroll_page_up(&mut self, page_size: usize) {
        self.selected_index = self.selected_index.saturating_sub(page_size);
    }

    /// Returns the ID of the currently selected node from the flattened rows.
    pub fn selected_node_id<'a>(&self, rows: &'a [FlattenedTreeRow]) -> Option<&'a str> {
        rows.get(self.selected_index).map(|r| r.node.id.as_str())
    }

    /// Handles keyboard events for tree navigation, folding, and actions.
    pub fn handle_key(&mut self, key: KeyEvent, tree: &mut AgentTree) -> AgentTreeAction {
        if key.kind == KeyEventKind::Release {
            return AgentTreeAction::None;
        }

        let rows = tree.flatten_visible();
        let total = rows.len();

        match key.code {
            KeyCode::Char('q') | KeyCode::Esc => AgentTreeAction::Close,
            KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => AgentTreeAction::Close,

            // Navigation
            KeyCode::Up | KeyCode::Char('k') => {
                self.select_prev();
                AgentTreeAction::None
            }
            KeyCode::Down | KeyCode::Char('j') => {
                self.select_next(total);
                AgentTreeAction::None
            }
            KeyCode::Home | KeyCode::Char('g') => {
                self.select_first();
                AgentTreeAction::None
            }
            KeyCode::End | KeyCode::Char('G') => {
                self.select_last(total);
                AgentTreeAction::None
            }
            KeyCode::PageUp => {
                self.scroll_page_up(10);
                AgentTreeAction::None
            }
            KeyCode::PageDown => {
                self.scroll_page_down(10, total);
                AgentTreeAction::None
            }

            // Expanding / Collapsing
            KeyCode::Char(' ') => {
                if let Some(id) = self.selected_node_id(&rows) {
                    let id_str = id.to_string();
                    tree.toggle_collapse(&id_str);
                    AgentTreeAction::ToggledFold(id_str)
                } else {
                    AgentTreeAction::None
                }
            }
            KeyCode::Left | KeyCode::Char('h') => {
                if let Some(row) = rows.get(self.selected_index) {
                    if row.has_children && row.is_expanded {
                        let id = row.node.id.clone();
                        tree.toggle_collapse(&id);
                        return AgentTreeAction::ToggledFold(id);
                    }
                }
                AgentTreeAction::None
            }
            KeyCode::Right | KeyCode::Char('l') => {
                if let Some(row) = rows.get(self.selected_index) {
                    if row.has_children && !row.is_expanded {
                        let id = row.node.id.clone();
                        tree.toggle_collapse(&id);
                        return AgentTreeAction::ToggledFold(id);
                    }
                }
                AgentTreeAction::None
            }
            KeyCode::Char('e') => {
                tree.expand_all();
                AgentTreeAction::None
            }
            KeyCode::Char('c') => {
                tree.collapse_all();
                AgentTreeAction::None
            }

            // Selection & Inspection
            KeyCode::Enter => {
                if let Some(id) = self.selected_node_id(&rows) {
                    AgentTreeAction::Selected(id.to_string())
                } else {
                    AgentTreeAction::None
                }
            }

            // Mode Switching
            KeyCode::Tab => {
                self.view_mode = self.view_mode.next();
                AgentTreeAction::ViewModeChanged(self.view_mode)
            }

            _ => AgentTreeAction::None,
        }
    }
}

// ---------------------------------------------------------------------------
// Ratatui Widget Implementation
// ---------------------------------------------------------------------------

/// Ratatui widget for rendering the agent tree diagram inside interactive TUIs.
pub struct AgentTreeWidget<'a> {
    tree: &'a AgentTree,
    theme: &'a Theme,
    options: TreeRenderOptions,
    block: Option<Block<'a>>,
}

impl<'a> AgentTreeWidget<'a> {
    /// Creates a new `AgentTreeWidget`.
    pub fn new(tree: &'a AgentTree, theme: &'a Theme) -> Self {
        Self {
            tree,
            theme,
            options: TreeRenderOptions::default(),
            block: None,
        }
    }

    /// Sets custom render options.
    pub fn with_options(mut self, options: TreeRenderOptions) -> Self {
        self.options = options;
        self
    }

    /// Sets the wrapping border block.
    pub fn block(mut self, block: Block<'a>) -> Self {
        self.block = Some(block);
        self
    }
}

impl<'a> Widget for AgentTreeWidget<'a> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let mut state = AgentTreeState::default();
        StatefulWidget::render(self, area, buf, &mut state);
    }
}

impl<'a> StatefulWidget for AgentTreeWidget<'a> {
    type State = AgentTreeState;

    fn render(self, area: Rect, buf: &mut Buffer, state: &mut Self::State) {
        if area.width < 10 || area.height < 3 {
            return;
        }

        // Render wrapping block if present
        let inner_area = if let Some(ref block) = self.block {
            let inner = block.inner(area);
            block.render(area, buf);
            inner
        } else {
            let title = self.tree.title.as_deref().unwrap_or("Active Subagents Mesh");
            let default_block = Block::default()
                .borders(Borders::ALL)
                .border_type(BorderType::Rounded)
                .border_style(Style::default().fg(self.theme.border))
                .title(Span::styled(
                    format!(" {title} "),
                    Style::default()
                        .fg(self.theme.primary)
                        .add_modifier(Modifier::BOLD),
                ));
            let inner = default_block.inner(area);
            default_block.render(area, buf);
            inner
        };

        if inner_area.height == 0 || inner_area.width == 0 {
            return;
        }

        // Dispatch based on view mode
        match state.view_mode {
            TreeViewMode::TreeOnly | TreeViewMode::Compact => {
                self.render_tree_pane(inner_area, buf, state);
            }
            TreeViewMode::SplitWithDetails => {
                let chunks = Layout::default()
                    .direction(Direction::Horizontal)
                    .constraints([Constraint::Percentage(55), Constraint::Percentage(45)])
                    .split(inner_area);

                self.render_tree_pane(chunks[0], buf, state);
                self.render_detail_pane(chunks[1], buf, state);
            }
        }
    }
}

impl<'a> AgentTreeWidget<'a> {
    fn render_tree_pane(&self, area: Rect, buf: &mut Buffer, state: &mut AgentTreeState) {
        let rows = self.tree.flatten_visible();
        if rows.is_empty() {
            let empty_p = Paragraph::new(Span::styled(
                "No active or completed subagents.",
                Style::default().fg(self.theme.muted),
            ));
            empty_p.render(area, buf);
            return;
        }

        // Keep selection in bounds
        if state.selected_index >= rows.len() {
            state.selected_index = rows.len().saturating_sub(1);
        }

        // Adjust scroll offset
        let visible_height = area.height as usize;
        if state.selected_index < state.scroll_offset {
            state.scroll_offset = state.selected_index;
        } else if state.selected_index >= state.scroll_offset + visible_height {
            state.scroll_offset = state.selected_index.saturating_sub(visible_height - 1);
        }

        for (i, row_idx) in (state.scroll_offset..rows.len()).take(visible_height).enumerate() {
            let y = area.y + i as u16;
            if y >= area.bottom() {
                break;
            }

            let row = &rows[row_idx];
            let is_selected = row_idx == state.selected_index;
            let spans = self.build_row_spans(row, is_selected, area.width as usize, state.tick);

            let line = Line::from(spans);
            buf.set_line(area.x, y, &line, area.width);

            // Highlight full selected row background
            if is_selected {
                for x in area.x..area.right() {
                    let cell = buf.cell_mut((x, y));
                    if let Some(c) = cell {
                        c.set_style(c.style().bg(self.theme.selection));
                    }
                }
            }
        }
    }

    fn build_row_spans(&self, row: &FlattenedTreeRow, is_selected: bool, max_width: usize, tick: usize) -> Vec<Span<'a>> {
        let mut spans = Vec::new();
        let node = &row.node;
        let prefix = self.options.glyphs.format_prefix(row, self.options.show_expand_indicators);

        // 1. Tree Guide Prefix
        spans.push(Span::styled(
            prefix,
            Style::default().fg(self.theme.border),
        ));

        // 2. Role Icon & Name
        let role_color = role_to_ratatui_color(&node.role, node.is_lead(), node.is_advisor(), self.theme);
        let role_style = Style::default()
            .fg(role_color)
            .add_modifier(if is_selected { Modifier::BOLD } else { Modifier::empty() });

        spans.push(Span::styled(format!("{} ", node.role_icon()), Style::default()));
        spans.push(Span::styled(format!("{} ", node.name), role_style));

        // 3. Status Badge with Animated Icon
        let status_color = status_to_ratatui_color(&node.status, self.theme);
        let status_style = Style::default()
            .fg(status_color)
            .add_modifier(Modifier::BOLD);

        let status_glyph = node.animated_status_icon(tick);
        spans.push(Span::styled(format!("{status_glyph} [{}] ", node.status_label()), status_style));

        // 4. Status summary
        let summary_style = Style::default().fg(status_color);
        spans.push(Span::styled(format!("{} ", node.status_summary()), summary_style));

        // 5. Metrics (Token usage & Execution time)
        if self.options.show_metrics {
            if let Some(metrics) = node.metrics_summary() {
                spans.push(Span::styled(
                    format!("{metrics} "),
                    Style::default().fg(self.theme.muted),
                ));
            }
        }

        // 6. Task snippet
        if self.options.show_task && !node.task.is_empty() {
            let current_len: usize = spans.iter().map(|s| s.content.chars().count()).sum();
            let remaining = max_width.saturating_sub(current_len + 5);
            let snippet = truncate_str(&node.task, remaining.min(self.options.task_max_length));
            if !snippet.is_empty() {
                spans.push(Span::styled(
                    format!("— \"{snippet}\""),
                    Style::default().fg(self.theme.muted),
                ));
            }
        }

        spans
    }

    fn render_detail_pane(&self, area: Rect, buf: &mut Buffer, state: &AgentTreeState) {
        let rows = self.tree.flatten_visible();
        let Some(selected_row) = rows.get(state.selected_index) else {
            return;
        };
        let node = &selected_row.node;

        let detail_block = Block::default()
            .borders(Borders::LEFT)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(self.theme.border))
            .title(Span::styled(
                format!(" Agent Details: {} ", node.name),
                Style::default()
                    .fg(self.theme.accent)
                    .add_modifier(Modifier::BOLD),
            ));

        let inner = detail_block.inner(area);
        detail_block.render(area, buf);

        if inner.height == 0 || inner.width == 0 {
            return;
        }

        let mut lines = Vec::new();

        // 1. Header Identifiers
        let role_col = role_to_ratatui_color(&node.role, node.is_lead(), node.is_advisor(), self.theme);
        lines.push(Line::from(vec![
            Span::styled("ID: ", Style::default().fg(self.theme.muted)),
            Span::styled(&node.id, Style::default().fg(self.theme.foreground).add_modifier(Modifier::BOLD)),
            Span::styled("  Role: ", Style::default().fg(self.theme.muted)),
            Span::styled(
                format!("{} {}", node.role_icon(), node.role_badge()),
                Style::default().fg(role_col).add_modifier(Modifier::BOLD),
            ),
        ]));

        // 2. Status & Tool
        let status_col = status_to_ratatui_color(&node.status, self.theme);
        let anim_icon = node.animated_status_icon(state.tick);
        lines.push(Line::from(vec![
            Span::styled("Status: ", Style::default().fg(self.theme.muted)),
            Span::styled(
                format!("{anim_icon} {}", node.status_label()),
                Style::default()
                    .fg(status_col)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(format!(" ({})", node.status_summary()), Style::default().fg(self.theme.muted)),
        ]));

        if let Some(tool) = &node.current_tool {
            lines.push(Line::from(vec![
                Span::styled("Active Tool: ", Style::default().fg(self.theme.muted)),
                Span::styled(tool, Style::default().fg(self.theme.warning).add_modifier(Modifier::BOLD)),
            ]));
        }

        // 3. Execution & Token Metrics
        let mut metric_spans = Vec::new();
        metric_spans.push(Span::styled("Turns: ", Style::default().fg(self.theme.muted)));
        if let Some(max) = node.max_turns {
            metric_spans.push(Span::styled(format!("{}/{}  ", node.turns, max), Style::default().fg(self.theme.foreground)));
        } else {
            metric_spans.push(Span::styled(format!("{}  ", node.turns), Style::default().fg(self.theme.foreground)));
        }

        if let Some(dur_str) = node.formatted_duration() {
            metric_spans.push(Span::styled("Duration: ", Style::default().fg(self.theme.muted)));
            metric_spans.push(Span::styled(format!("{dur_str}  "), Style::default().fg(self.theme.info)));
        }

        if let Some(tokens) = node.tokens_used {
            metric_spans.push(Span::styled("Tokens: ", Style::default().fg(self.theme.muted)));
            let tok_detail = match (node.prompt_tokens, node.completion_tokens) {
                (Some(p), Some(c)) => format!("{tokens} (p:{p}, c:{c})  "),
                _ => format!("{tokens}  "),
            };
            metric_spans.push(Span::styled(tok_detail, Style::default().fg(self.theme.foreground)));
        }

        if let Some(tps) = node.tokens_per_second() {
            metric_spans.push(Span::styled("Speed: ", Style::default().fg(self.theme.muted)));
            metric_spans.push(Span::styled(format!("{tps:.1} tok/s  "), Style::default().fg(self.theme.primary)));
        }

        if let Some(cost) = node.cost_usd {
            metric_spans.push(Span::styled("Cost: ", Style::default().fg(self.theme.muted)));
            metric_spans.push(Span::styled(format!("${cost:.4}  "), Style::default().fg(self.theme.success)));
        }

        lines.push(Line::from(metric_spans));

        // 4. Tags & Custom Attributes
        if !node.tags.is_empty() || !node.custom_attributes.is_empty() {
            let mut meta_spans = Vec::new();
            if !node.tags.is_empty() {
                let tag_str = node.tags.iter().map(|t| format!("#{t}")).collect::<Vec<_>>().join(" ");
                meta_spans.push(Span::styled("Tags: ", Style::default().fg(self.theme.muted)));
                meta_spans.push(Span::styled(format!("{tag_str}  "), Style::default().fg(self.theme.info)));
            }
            for (k, v) in &node.custom_attributes {
                meta_spans.push(Span::styled(format!("{k}: "), Style::default().fg(self.theme.muted)));
                meta_spans.push(Span::styled(format!("{v}  "), Style::default().fg(self.theme.foreground)));
            }
            lines.push(Line::from(meta_spans));
        }

        lines.push(Line::from(""));

        // 5. Assigned Task Prompt
        lines.push(Line::from(Span::styled(
            "── Assigned Task ──",
            Style::default().fg(self.theme.primary).add_modifier(Modifier::BOLD),
        )));
        lines.push(Line::from(Span::styled(
            &node.task,
            Style::default().fg(self.theme.foreground),
        )));

        // 6. Output or Error Detail
        if let SubagentStatus::Completed { output, .. } = &node.status {
            lines.push(Line::from(""));
            lines.push(Line::from(Span::styled(
                "── Output Summary ──",
                Style::default().fg(self.theme.success).add_modifier(Modifier::BOLD),
            )));
            lines.push(Line::from(Span::styled(
                output,
                Style::default().fg(self.theme.foreground),
            )));
        } else if let SubagentStatus::Failed { error } = &node.status {
            lines.push(Line::from(""));
            lines.push(Line::from(Span::styled(
                "── Error Detail ──",
                Style::default().fg(self.theme.error).add_modifier(Modifier::BOLD),
            )));
            lines.push(Line::from(Span::styled(
                error,
                Style::default().fg(self.theme.error),
            )));
        }

        let p = Paragraph::new(lines).wrap(Wrap { trim: true });
        p.render(inner, buf);
    }
}

fn role_to_ratatui_color(role: &SubagentRole, is_lead: bool, is_advisor: bool, theme: &Theme) -> Color {
    if is_lead {
        return theme.primary;
    }
    if is_advisor {
        return theme.advisor;
    }
    match role {
        SubagentRole::Scout => theme.info,
        SubagentRole::Coder => theme.primary,
        SubagentRole::Tester => theme.accent,
        SubagentRole::Reviewer => theme.warning,
        SubagentRole::General => theme.foreground,
        SubagentRole::Custom { .. } => theme.secondary,
    }
}

fn status_to_ratatui_color(status: &SubagentStatus, theme: &Theme) -> Color {
    match status {
        SubagentStatus::Pending => theme.muted,
        SubagentStatus::Running { .. } => theme.info,
        SubagentStatus::Completed { .. } => theme.success,
        SubagentStatus::Failed { .. } => theme.error,
        SubagentStatus::Cancelled => theme.warning,
    }
}

// ============================================================================
// 6. Unit Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_node_creation_and_builders() {
        let node = AgentTreeNode::new("a1", "Worker-1", SubagentRole::Coder, "Refactor database models")
            .with_status(SubagentStatus::Running {
                turn: 2,
                current_tool: Some("edit".to_string()),
            })
            .with_turns(2, Some(10))
            .with_token_breakdown(1000, 500)
            .with_cost(0.003)
            .with_duration(Duration::from_millis(1500))
            .with_tag("backend");

        assert_eq!(node.id, "a1");
        assert_eq!(node.name, "Worker-1");
        assert_eq!(node.role, SubagentRole::Coder);
        assert!(node.is_running());
        assert_eq!(node.turns, 2);
        assert_eq!(node.max_turns, Some(10));
        assert_eq!(node.tokens_used, Some(1500));
        assert_eq!(node.prompt_tokens, Some(1000));
        assert_eq!(node.completion_tokens, Some(500));
        assert_eq!(node.cost_usd, Some(0.003));
        assert_eq!(node.tags, vec!["backend"]);
        assert_eq!(node.role_icon(), "⚡");
        assert_eq!(node.status_label(), "RUNNING");
        assert!(node.status_summary().contains("turn 2/10 tool: edit"));
        assert_eq!(node.formatted_duration().as_deref(), Some("1.5s"));
        assert_eq!(node.formatted_tokens().as_deref(), Some("1.5k tok"));
        assert!(node.tokens_per_second().is_some());
    }

    #[test]
    fn test_specialized_role_constructors_and_icons() {
        let lead = AgentTreeNode::lead("l1", "Coordinator", "Manage swarm");
        assert!(lead.is_lead());
        assert_eq!(lead.role_icon(), "👑");
        assert_eq!(lead.role_badge(), "[Lead]");

        let scout = AgentTreeNode::scout("s1", "Finder", "Find files");
        assert_eq!(scout.role_icon(), "🔍");
        assert_eq!(scout.role_badge(), "[Scout]");

        let coder = AgentTreeNode::coder("c1", "Builder", "Write code");
        assert_eq!(coder.role_icon(), "⚡");
        assert_eq!(coder.role_badge(), "[Coder]");

        let tester = AgentTreeNode::tester("t1", "Verifier", "Run tests");
        assert_eq!(tester.role_icon(), "🧪");
        assert_eq!(tester.role_badge(), "[Tester]");

        let reviewer = AgentTreeNode::reviewer("r1", "Auditor", "Review code");
        assert_eq!(reviewer.role_icon(), "🛡️");
        assert_eq!(reviewer.role_badge(), "[Reviewer]");

        let advisor = AgentTreeNode::advisor("a1", "SecAdvisor", "Security", "Review security");
        assert!(advisor.is_advisor());
        assert_eq!(advisor.role_icon(), "💡");
        assert_eq!(advisor.role_badge(), "[Advisor]");

        let general = AgentTreeNode::general("g1", "Worker", "Do general work");
        assert_eq!(general.role_icon(), "🤖");
        assert_eq!(general.role_badge(), "[Worker]");
    }

    #[test]
    fn test_animated_status_indicators() {
        let running_node = AgentTreeNode::coder("c1", "Builder", "Compile").with_status(SubagentStatus::Running {
            turn: 1,
            current_tool: None,
        });

        assert_eq!(running_node.animated_status_icon(0), "⠋");
        assert_eq!(running_node.animated_status_icon(1), "⠙");
        assert_eq!(running_node.animated_status_icon(2), "⠹");

        let done_node = AgentTreeNode::tester("t1", "Tester", "Pass").with_status(SubagentStatus::Completed {
            output: "Passed".to_string(),
            turns: 1,
        });
        assert_eq!(done_node.animated_status_icon(0), "✓");
        assert_eq!(done_node.animated_status_icon(5), "✓");

        let fail_node = AgentTreeNode::tester("t2", "Tester", "Fail").with_status(SubagentStatus::Failed {
            error: "Broken".to_string(),
        });
        assert_eq!(fail_node.animated_status_icon(0), "✗");

        let cancel_node = AgentTreeNode::general("c1", "Worker", "Cancel").with_status(SubagentStatus::Cancelled);
        assert_eq!(cancel_node.animated_status_icon(0), "⊘");

        let pending_node = AgentTreeNode::general("g1", "Worker", "Wait");
        assert_eq!(pending_node.animated_status_icon(0), "⏳");
    }

    #[test]
    fn test_tree_hierarchy_reconstruction_from_flat() {
        let root = AgentTreeNode::lead("root", "MainCoordinator", "Top level goal");
        let child1 = AgentTreeNode::scout("c1", "Scout", "Search files").with_parent("root");
        let child2 = AgentTreeNode::coder("c2", "Coder", "Write code").with_parent("root");
        let grandchild = AgentTreeNode::tester("gc1", "Tester", "Verify changes").with_parent("c2");

        let flat = vec![root, child1, child2, grandchild];
        let tree = AgentTree::from_flat_nodes(flat);

        assert_eq!(tree.roots.len(), 1);
        let root_node = &tree.roots[0];
        assert_eq!(root_node.id, "root");
        assert_eq!(root_node.children.len(), 2);

        let coder_node = root_node.children.iter().find(|c| c.id == "c2").unwrap();
        assert_eq!(coder_node.children.len(), 1);
        assert_eq!(coder_node.children[0].id, "gc1");

        assert_eq!(tree.total_agents(), 4);
    }

    #[test]
    fn test_multi_agent_mesh_demo_tree() {
        let tree = AgentTree::mesh_demo_tree();
        assert_eq!(tree.roots.len(), 1);
        let lead = &tree.roots[0];
        assert!(lead.is_lead());
        assert_eq!(lead.children.len(), 4);

        assert!(tree.find_node("scout-01").is_some());
        assert!(tree.find_node("coder-01").is_some());
        assert!(tree.find_node("tester-01").is_some());
        assert!(tree.find_node("review-01").is_some());
        assert!(tree.find_node("adv-sec").is_some());
        assert!(tree.find_node("adv-arch").is_some());

        assert!(tree.total_tokens() > 0);
        assert!(tree.total_prompt_tokens() > 0);
        assert!(tree.total_completion_tokens() > 0);
        assert!(tree.total_cost() > 0.0);
        assert!(tree.total_turns() > 0);
    }

    #[test]
    fn test_flatten_visible_prefixes_and_box_drawing() {
        let grandchild = AgentTreeNode::reviewer("gc", "Reviewer", "Review code");
        let child1 = AgentTreeNode::scout("c1", "Scout", "Search code");
        let child2 = AgentTreeNode::coder("c2", "Coder", "Implement feature")
            .with_child(grandchild);

        let root = AgentTreeNode::lead("root", "Coordinator", "Manage project")
            .with_child(child1)
            .with_child(child2);

        let tree = AgentTree::with_root(root);
        let rows = tree.flatten_visible();

        assert_eq!(rows.len(), 4);

        let glyphs = TreeGlyphSet::unicode();
        let p0 = glyphs.format_prefix(&rows[0], true);
        let p1 = glyphs.format_prefix(&rows[1], true);
        let p2 = glyphs.format_prefix(&rows[2], true);
        let p3 = glyphs.format_prefix(&rows[3], true);

        // Root has children and is expanded: "▼ "
        assert_eq!(p0, "▼ ");
        // Child 1 is intermediate: "├── ● "
        assert_eq!(p1, "├── ● ");
        // Child 2 is last child of root, and expanded: "└── ▼ "
        assert_eq!(p2, "└── ▼ ");
        // Grandchild is child of last child of root: "    └── ● "
        assert_eq!(p3, "    └── ● ");
    }

    #[test]
    fn test_tree_plain_and_ansi_rendering() {
        let tree = AgentTree::mesh_demo_tree();
        let options = TreeRenderOptions::default().with_tick(2);

        let plain = render_tree_plain(&tree, &options);
        assert!(!plain.is_empty());
        assert!(plain.contains("Lead Coordinator"));
        assert!(plain.contains("ArchitectureScout"));
        assert!(plain.contains("WidgetCoder"));
        assert!(plain.contains("├──"));
        assert!(plain.contains("└──"));
        assert!(plain.contains("Total:"));

        let theme = Theme::default();
        let ansi = render_tree_ansi(&tree, &options, &theme);
        assert!(!ansi.is_empty());
        assert!(ansi.contains("\x1b["));
    }

    #[test]
    fn test_tree_glyph_presets() {
        let rounded = TreeGlyphSet::rounded();
        assert_eq!(rounded.last_branch, "╰── ");

        let bold = TreeGlyphSet::bold();
        assert_eq!(bold.branch, "┣━━ ");
        assert_eq!(bold.last_branch, "┗━━ ");

        let double = TreeGlyphSet::double();
        assert_eq!(double.branch, "╠══ ");
        assert_eq!(double.last_branch, "╚══ ");

        let ascii = TreeGlyphSet::ascii();
        assert_eq!(ascii.branch, "|-- ");
        assert_eq!(ascii.last_branch, "\\-- ");

        let compact = TreeGlyphSet::compact();
        assert_eq!(compact.branch, "├ ");
        assert_eq!(compact.last_branch, "└ ");
    }

    #[test]
    fn test_tree_search_and_filtering() {
        let tree = AgentTree::mesh_demo_tree();
        let matches = tree.search("scout");
        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0].name, "ArchitectureScout");

        let coder_matches = tree.search("widgetcoder");
        assert_eq!(coder_matches.len(), 1);
        assert_eq!(coder_matches[0].name, "WidgetCoder");

        let adv_matches = tree.search("advisor");
        assert_eq!(adv_matches.len(), 2);

        let empty = tree.search("nonexistent");
        assert!(empty.is_empty());
    }

    #[test]
    fn test_state_navigation_and_collapse_toggle() {
        let mut tree = AgentTree::mesh_demo_tree();
        let mut state = AgentTreeState::new();

        let initial_rows = tree.flatten_visible();
        let initial_count = initial_rows.len();
        assert!(initial_count > 1);

        state.select_next(initial_count);
        assert_eq!(state.selected_index, 1);

        state.select_prev();
        assert_eq!(state.selected_index, 0);

        state.select_last(initial_count);
        assert_eq!(state.selected_index, initial_count - 1);

        state.select_first();
        assert_eq!(state.selected_index, 0);

        // Toggle collapse on root
        let root_id = tree.roots[0].id.clone();
        assert!(tree.toggle_collapse(&root_id));

        let collapsed_rows = tree.flatten_visible();
        assert_eq!(collapsed_rows.len(), 1);

        tree.expand_all();
        let expanded_rows = tree.flatten_visible();
        assert_eq!(expanded_rows.len(), initial_count);
    }

    #[test]
    fn test_state_tick_and_view_mode_cycle() {
        let mut state = AgentTreeState::new();
        assert_eq!(state.tick, 0);
        assert_eq!(state.spinner_frame(), "⠋");

        state.tick();
        assert_eq!(state.tick, 1);
        assert_eq!(state.spinner_frame(), "⠙");

        assert_eq!(state.view_mode, TreeViewMode::TreeOnly);
        state.view_mode = state.view_mode.next();
        assert_eq!(state.view_mode, TreeViewMode::SplitWithDetails);
        state.view_mode = state.view_mode.next();
        assert_eq!(state.view_mode, TreeViewMode::Compact);
        state.view_mode = state.view_mode.next();
        assert_eq!(state.view_mode, TreeViewMode::TreeOnly);
    }

    #[test]
    fn test_ratatui_widget_tree_only_rendering() {
        let tree = AgentTree::mesh_demo_tree();
        let theme = Theme::default();
        let widget = AgentTreeWidget::new(&tree, &theme);

        let area = Rect::new(0, 0, 90, 25);
        let mut buf = Buffer::empty(area);

        ratatui::widgets::Widget::render(widget, area, &mut buf);

        let mut text = String::new();
        for y in 0..area.height {
            for x in 0..area.width {
                if let Some(cell) = buf.cell((x, y)) {
                    text.push_str(cell.symbol());
                }
            }
            text.push('\n');
        }

        assert!(text.contains("Multi-Agent Swarm Execution Mesh") || text.contains("Mesh"));
        assert!(text.contains("Lead Coordinator"));
        assert!(text.contains("ArchitectureScout"));
    }

    #[test]
    fn test_ratatui_widget_split_view_rendering() {
        let tree = AgentTree::mesh_demo_tree();
        let theme = Theme::default();
        let widget = AgentTreeWidget::new(&tree, &theme);

        let mut state = AgentTreeState::new();
        state.view_mode = TreeViewMode::SplitWithDetails;

        let area = Rect::new(0, 0, 100, 25);
        let mut buf = Buffer::empty(area);

        StatefulWidget::render(widget, area, &mut buf, &mut state);

        let mut text = String::new();
        for y in 0..area.height {
            for x in 0..area.width {
                if let Some(cell) = buf.cell((x, y)) {
                    text.push_str(cell.symbol());
                }
            }
            text.push('\n');
        }

        assert!(text.contains("Agent Details:"));
        assert!(text.contains("Assigned Task"));
    }

    #[test]
    fn test_widget_zero_and_minimal_area_safety() {
        let tree = AgentTree::mesh_demo_tree();
        let theme = Theme::default();
        let widget = AgentTreeWidget::new(&tree, &theme);

        let area_zero = Rect::new(0, 0, 0, 0);
        let mut buf_zero = Buffer::empty(area_zero);
        ratatui::widgets::Widget::render(widget, area_zero, &mut buf_zero);

        let area_tiny = Rect::new(0, 0, 5, 2);
        let mut buf_tiny = Buffer::empty(area_tiny);
        let widget2 = AgentTreeWidget::new(&tree, &theme);
        ratatui::widgets::Widget::render(widget2, area_tiny, &mut buf_tiny);
    }

    #[test]
    fn test_progress_event_updates() {
        let mut tree = AgentTree::with_root(AgentTreeNode::coder("worker-1", "Worker", "Initial task"));

        let start_ev = SubagentProgress::Started {
            id: "worker-1".to_string(),
            name: "LiveWorker".to_string(),
            role: SubagentRole::Coder,
            task: "Updated task description".to_string(),
        };
        assert!(tree.update_progress(&start_ev));
        let node = tree.find_node("worker-1").unwrap();
        assert_eq!(node.name, "LiveWorker");
        assert_eq!(node.role, SubagentRole::Coder);
        assert!(node.is_running());

        let tool_ev = SubagentProgress::ToolStarted {
            id: "worker-1".to_string(),
            tool: "grep".to_string(),
            args: serde_json::json!({}),
        };
        assert!(tree.update_progress(&tool_ev));
        let node = tree.find_node("worker-1").unwrap();
        assert_eq!(node.current_tool.as_deref(), Some("grep"));

        let done_ev = SubagentProgress::Completed {
            id: "worker-1".to_string(),
            output: "All done successfully".to_string(),
            turns_taken: 4,
        };
        assert!(tree.update_progress(&done_ev));
        let node = tree.find_node("worker-1").unwrap();
        assert!(node.is_completed());
        assert_eq!(node.turns, 4);
    }

    #[test]
    fn test_ratatui_widget_compact_view_rendering() {
        let tree = AgentTree::mesh_demo_tree();
        let theme = Theme::default();
        let widget = AgentTreeWidget::new(&tree, &theme);

        let mut state = AgentTreeState::new();
        state.view_mode = TreeViewMode::Compact;

        let area = Rect::new(0, 0, 80, 20);
        let mut buf = Buffer::empty(area);

        StatefulWidget::render(widget, area, &mut buf, &mut state);

        let mut text = String::new();
        for y in 0..area.height {
            for x in 0..area.width {
                if let Some(cell) = buf.cell((x, y)) {
                    text.push_str(cell.symbol());
                }
            }
            text.push('\n');
        }

        assert!(text.contains("Lead Coordinator"));
    }

    #[test]
    fn test_keyboard_event_handling() {
        let mut tree = AgentTree::mesh_demo_tree();
        let mut state = AgentTreeState::new();

        let down_key = KeyEvent::new(KeyCode::Down, KeyModifiers::empty());
        let action = state.handle_key(down_key, &mut tree);
        assert_eq!(action, AgentTreeAction::None);
        assert_eq!(state.selected_index, 1);

        let up_key = KeyEvent::new(KeyCode::Up, KeyModifiers::empty());
        state.handle_key(up_key, &mut tree);
        assert_eq!(state.selected_index, 0);

        let tab_key = KeyEvent::new(KeyCode::Tab, KeyModifiers::empty());
        let action = state.handle_key(tab_key, &mut tree);
        assert_eq!(action, AgentTreeAction::ViewModeChanged(TreeViewMode::SplitWithDetails));

        let enter_key = KeyEvent::new(KeyCode::Enter, KeyModifiers::empty());
        let action = state.handle_key(enter_key, &mut tree);
        assert_eq!(action, AgentTreeAction::Selected("lead-coord".to_string()));

        let esc_key = KeyEvent::new(KeyCode::Esc, KeyModifiers::empty());
        let action = state.handle_key(esc_key, &mut tree);
        assert_eq!(action, AgentTreeAction::Close);
    }

    #[test]
    fn test_tokens_and_cost_aggregation() {
        let tree = AgentTree::mesh_demo_tree();
        assert!(tree.total_tokens() > 10_000);
        assert!(tree.total_prompt_tokens() > 5_000);
        assert!(tree.total_completion_tokens() > 5_000);
        assert!(tree.total_cost() > 0.05);
        assert!(tree.total_duration() > Duration::from_millis(1000));
        assert!(tree.max_duration() >= Duration::from_millis(1250));
    }
}
