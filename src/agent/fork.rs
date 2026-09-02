//! Session rewind and branching subsystem for Fusion.
//!
//! Provides capabilities to:
//! - Fork an existing session into a new branch (`fork_session`), optionally at a specific historical turn.
//! - Rewind a session by reverting the last N conversation turns (`rewind_session`).
//! - Inspect conversational turn boundaries and previews (`SessionTurn`, `extract_turns`).
//! - Trace branch lineages and discover sibling/child branches.
//! - Compare two sessions to identify divergence points (`diff_session_branches`).

use std::collections::{HashMap, HashSet};

use chrono::Utc;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::agent::session::{Session, SessionSummary};
use crate::provider::types::{Message, Role};

/// Detailed information about a single conversational turn in a session.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionTurn {
    /// 1-based turn sequence number (1, 2, 3...)
    pub turn_index: usize,
    /// Starting index in `session.messages` (inclusive).
    pub start_message_index: usize,
    /// Ending index in `session.messages` (exclusive).
    pub end_message_index: usize,
    /// User message content initiating this turn, if any.
    pub user_message: Option<String>,
    /// Final assistant response content in this turn, if any.
    pub assistant_message: Option<String>,
    /// Number of tool calls executed during this turn.
    pub tool_calls_count: usize,
    /// Total number of messages in this turn (user + assistant + tools).
    pub message_count: usize,
}

impl SessionTurn {
    /// Returns a short human-readable preview of the turn.
    pub fn preview(&self) -> String {
        let user = self
            .user_message
            .as_deref()
            .map(|s| {
                let trimmed = s.trim();
                let chars: String = trimmed.chars().take(40).collect();
                if trimmed.chars().count() > 40 {
                    format!("{}...", chars)
                } else {
                    chars
                }
            })
            .unwrap_or_else(|| "<no user message>".to_string());

        let assistant = self
            .assistant_message
            .as_deref()
            .map(|s| {
                let trimmed = s.trim();
                let chars: String = trimmed.chars().take(40).collect();
                if trimmed.chars().count() > 40 {
                    format!("{}...", chars)
                } else {
                    chars
                }
            })
            .unwrap_or_else(|| "<no response>".to_string());

        format!(
            "Turn {}: User: \"{}\" -> Assistant: \"{}\"",
            self.turn_index, user, assistant
        )
    }
}

/// Differences and divergence point between two session branches.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionBranchDiff {
    /// Number of identical turns shared between both sessions from the beginning.
    pub common_turns: usize,
    /// Number of turns unique to session A after the divergence point.
    pub session_a_exclusive_turns: usize,
    /// Number of turns unique to session B after the divergence point.
    pub session_b_exclusive_turns: usize,
    /// The 1-based turn index where the two sessions diverged, or `None` if identical.
    pub divergence_turn: Option<usize>,
}

/// Identifies and extracts all conversational turns from a list of messages.
///
/// A turn is initiated by a `Role::User` message and includes all subsequent
/// tool calls, tool results, and assistant messages up to the next `Role::User` message.
/// Any messages preceding the first user message (e.g. system instructions) form the prelude.
pub fn extract_turns(messages: &[Message]) -> Vec<SessionTurn> {
    let mut turns = Vec::new();
    let user_indices: Vec<usize> = messages
        .iter()
        .enumerate()
        .filter(|(_, m)| m.role == Role::User)
        .map(|(i, _)| i)
        .collect();

    if user_indices.is_empty() {
        // Check if there are non-system messages (e.g. assistant greeting)
        let has_assistant = messages.iter().any(|m| m.role == Role::Assistant);
        if has_assistant {
            let start = messages
                .iter()
                .position(|m| m.role != Role::System)
                .unwrap_or(0);
            let end = messages.len();
            let slice = &messages[start..end];
            let assistant_msg = slice
                .iter()
                .rev()
                .find(|m| m.role == Role::Assistant)
                .map(|m| m.content.clone());
            let tool_calls_count = slice
                .iter()
                .filter_map(|m| m.tool_calls.as_ref())
                .map(|tc| tc.len())
                .sum();
            turns.push(SessionTurn {
                turn_index: 1,
                start_message_index: start,
                end_message_index: end,
                user_message: None,
                assistant_message: assistant_msg,
                tool_calls_count,
                message_count: end - start,
            });
        }
        return turns;
    }

    for (turn_idx, &start_idx) in user_indices.iter().enumerate() {
        let end_idx = if turn_idx + 1 < user_indices.len() {
            user_indices[turn_idx + 1]
        } else {
            messages.len()
        };

        let slice = &messages[start_idx..end_idx];
        let user_msg = slice.first().map(|m| m.content.clone());
        let assistant_msg = slice
            .iter()
            .rev()
            .find(|m| m.role == Role::Assistant)
            .map(|m| m.content.clone());
        let tool_calls_count = slice
            .iter()
            .filter_map(|m| m.tool_calls.as_ref())
            .map(|tc| tc.len())
            .sum();

        turns.push(SessionTurn {
            turn_index: turn_idx + 1,
            start_message_index: start_idx,
            end_message_index: end_idx,
            user_message: user_msg,
            assistant_message: assistant_msg,
            tool_calls_count,
            message_count: end_idx - start_idx,
        });
    }

    turns
}

/// Returns the total number of turns in the given session.
pub fn count_turns(session: &Session) -> usize {
    extract_turns(session.messages()).len()
}

/// Returns details for a specific 1-based turn in the session, if present.
pub fn get_turn(session: &Session, turn_index: usize) -> Option<SessionTurn> {
    if turn_index == 0 {
        return None;
    }
    let turns = extract_turns(session.messages());
    turns.into_iter().nth(turn_index - 1)
}

// ---------------------------------------------------------------------------
// Rewind Operations
// ---------------------------------------------------------------------------

/// Rewinds an in-memory session in place by reverting the last `turns_to_rewind` turns.
///
/// Returns the actual number of turns reverted.
/// Preserves any prelude/system messages that occurred before the first rewound turn.
pub fn rewind_session_in_place(session: &mut Session, turns_to_rewind: usize) -> usize {
    if turns_to_rewind == 0 {
        return 0;
    }

    let turns = extract_turns(&session.messages);
    let total_turns = turns.len();
    if total_turns == 0 {
        // No turns to rewind; if there are stray non-system messages, clear them
        let first_non_system = session
            .messages
            .iter()
            .position(|m| m.role != Role::System)
            .unwrap_or(session.messages.len());
        if session.messages.len() > first_non_system {
            session.messages.truncate(first_non_system);
            session.touch();
        }
        return 0;
    }

    let turns_reverted = turns_to_rewind.min(total_turns);
    let target_remaining = total_turns - turns_reverted;

    if target_remaining == 0 {
        // Revert all turns: truncate to before the first turn
        let truncate_idx = turns[0].start_message_index;
        session.messages.truncate(truncate_idx);
    } else {
        // Keep turns up to target_remaining
        let truncate_idx = turns[target_remaining - 1].end_message_index;
        session.messages.truncate(truncate_idx);
    }

    // Update token stats total turns
    session.token_stats.total_turns = session
        .token_stats
        .total_turns
        .saturating_sub(turns_reverted as u64);

    // Record rewind in session metadata
    let now = Utc::now().to_rfc3339();
    session.metadata.insert("last_rewound_at".to_string(), now);
    session.metadata.insert(
        "last_rewound_turns".to_string(),
        turns_reverted.to_string(),
    );
    session.touch();

    turns_reverted
}

/// Rewinds a persistent session on disk by UUID, reverting the last `turns` turns,
/// saving the modified session to disk, and returning the updated session.
pub fn rewind_session(session_id: Uuid, turns: usize) -> anyhow::Result<Session> {
    let mut session = Session::load(session_id)?;
    rewind_session_in_place(&mut session, turns);
    session.save()?;
    Ok(session)
}

/// Rewinds a persistent session on disk identified by UUID string or prefix.
pub fn rewind_session_from_str(id_or_prefix: &str, turns: usize) -> anyhow::Result<Session> {
    let mut session = Session::load_from_str(id_or_prefix)?;
    rewind_session_in_place(&mut session, turns);
    session.save()?;
    Ok(session)
}

// ---------------------------------------------------------------------------
// Fork / Branching Operations
// ---------------------------------------------------------------------------

/// Forks an in-memory session into a new branch.
///
/// If `at_turn` is `None`, creates a new branch preserving all existing turns.
/// If `at_turn` is `Some(n)`, truncates the branch history to contain only turns up to `n`.
/// For example, `at_turn = Some(1)` keeps only Turn 1. `at_turn = Some(0)` keeps only the
/// system messages/prelude.
pub fn fork_session_in_memory(session: &Session, at_turn: Option<usize>) -> Session {
    let mut forked = session.clone();
    forked.id = Uuid::new_v4();
    let now = Utc::now().to_rfc3339();
    forked.created_at = now.clone();
    forked.updated_at = now.clone();

    let turns = extract_turns(&session.messages);
    let total_turns = turns.len();

    let effective_turn = match at_turn {
        Some(n) => n.min(total_turns),
        None => total_turns,
    };

    if effective_turn == 0 {
        // Truncate to prelude (before first turn)
        let truncate_idx = if let Some(first) = turns.first() {
            first.start_message_index
        } else {
            0
        };
        forked.messages.truncate(truncate_idx);
        forked.token_stats.total_turns = 0;
    } else if effective_turn < total_turns {
        // Truncate to end of effective_turn
        let truncate_idx = turns[effective_turn - 1].end_message_index;
        forked.messages.truncate(truncate_idx);
        forked.token_stats.total_turns = effective_turn as u64;
    } else {
        // Keep all turns
        forked.token_stats.total_turns = total_turns as u64;
    }

    // Set branch title
    let base_title = session
        .title()
        .map(|t| t.to_string())
        .unwrap_or_else(|| "Session".to_string());

    let new_title = match at_turn {
        Some(n) if n < total_turns => format!("{} (branch @ turn {})", base_title, n),
        _ => format!("{} (branch)", base_title),
    };
    forked.set_title(new_title);

    // Record branch lineage in metadata
    forked
        .metadata
        .insert("forked_from_id".to_string(), session.id.to_string());
    if let Some(t) = session.title() {
        forked
            .metadata
            .insert("forked_from_title".to_string(), t.to_string());
    }
    forked
        .metadata
        .insert("forked_at_turn".to_string(), effective_turn.to_string());
    forked.metadata.insert("forked_at".to_string(), now);

    forked
}

/// Forks a session on disk by UUID, creating a new branched session saved to disk.
///
/// If `at_turn` is specified, the branch will only contain history up to that turn number.
/// If `at_turn` is `None`, the new branch copies the complete existing conversation.
pub fn fork_session(session_id: Uuid, at_turn: Option<usize>) -> anyhow::Result<Session> {
    let original = Session::load(session_id)?;
    let forked = fork_session_in_memory(&original, at_turn);
    forked.save()?;
    Ok(forked)
}

/// Forks a session identified by UUID string or prefix.
pub fn fork_session_from_str(id_or_prefix: &str, at_turn: Option<usize>) -> anyhow::Result<Session> {
    let original = Session::load_from_str(id_or_prefix)?;
    let forked = fork_session_in_memory(&original, at_turn);
    forked.save()?;
    Ok(forked)
}

// ---------------------------------------------------------------------------
// Branch Discovery & Lineage Operations
// ---------------------------------------------------------------------------

/// Lists all sessions on disk that were forked directly from `parent_id`.
pub fn list_branches(parent_id: Uuid) -> anyhow::Result<Vec<SessionSummary>> {
    let parent_str = parent_id.to_string();
    let all = Session::list_sessions()?;
    let mut branches = Vec::new();

    for summary in all {
        if summary.id == parent_id {
            continue;
        }
        if let Ok(sess) = Session::load(summary.id) {
            if sess.metadata.get("forked_from_id").map(|s| s.as_str()) == Some(&parent_str) {
                branches.push(summary);
            }
        }
    }

    Ok(branches)
}

/// Traces the lineage of ancestor sessions for `session_id`, from root ancestor down to the given session.
pub fn get_fork_lineage(session_id: Uuid) -> anyhow::Result<Vec<SessionSummary>> {
    let mut lineage = Vec::new();
    let mut current_id = session_id;
    let mut visited = HashSet::new();

    while visited.insert(current_id) {
        let sess = match Session::load(current_id) {
            Ok(s) => s,
            Err(_) => break,
        };

        let preview = sess
            .messages
            .iter()
            .rev()
            .find(|m| m.role == Role::User || m.role == Role::Assistant)
            .map(|m| {
                let mut p: String = m.content.chars().take(80).collect();
                if m.content.chars().count() > 80 {
                    p.push_str("...");
                }
                p
            })
            .unwrap_or_else(|| "Empty session".to_string());

        lineage.push(SessionSummary {
            id: sess.id,
            created_at: sess.created_at.clone(),
            updated_at: sess.updated_at.clone(),
            active_model: sess.active_model.clone(),
            title: sess.title.clone(),
            message_count: sess.messages.len(),
            preview,
        });

        if let Some(parent_str) = sess.metadata.get("forked_from_id") {
            if let Ok(parent_uuid) = Uuid::parse_str(parent_str) {
                current_id = parent_uuid;
                continue;
            }
        }
        break;
    }

    lineage.reverse();
    Ok(lineage)
}

/// Compares two sessions and calculates common turns and divergence point.
pub fn diff_session_branches(session_a: &Session, session_b: &Session) -> SessionBranchDiff {
    let turns_a = extract_turns(&session_a.messages);
    let turns_b = extract_turns(&session_b.messages);

    let min_turns = turns_a.len().min(turns_b.len());
    let mut common = 0;

    for i in 0..min_turns {
        let ta = &turns_a[i];
        let tb = &turns_b[i];

        let user_match = ta.user_message == tb.user_message;
        let assistant_match = ta.assistant_message == tb.assistant_message;

        if user_match && assistant_match {
            common += 1;
        } else {
            break;
        }
    }

    let divergence_turn = if common < min_turns {
        Some(common + 1)
    } else if turns_a.len() != turns_b.len() {
        Some(min_turns + 1)
    } else {
        None
    };

    SessionBranchDiff {
        common_turns: common,
        session_a_exclusive_turns: turns_a.len().saturating_sub(common),
        session_b_exclusive_turns: turns_b.len().saturating_sub(common),
        divergence_turn,
    }
}

/// Formats a formatted branch tree representation starting from the session's root ancestor.
pub fn preview_branch_tree(session_id: Uuid) -> anyhow::Result<String> {
    let lineage = get_fork_lineage(session_id)?;
    let root_id = lineage.first().map(|s| s.id).unwrap_or(session_id);

    // Map parent_id -> Vec<SessionSummary>
    let all = Session::list_sessions()?;
    let mut children_map: HashMap<Uuid, Vec<SessionSummary>> = HashMap::new();

    for summary in all {
        if let Ok(sess) = Session::load(summary.id) {
            if let Some(p_str) = sess.metadata.get("forked_from_id") {
                if let Ok(p_id) = Uuid::parse_str(p_str) {
                    children_map.entry(p_id).or_default().push(summary);
                }
            }
        }
    }

    let mut output = String::new();
    let root_sess = Session::load(root_id)?;
    let root_title = root_sess.title().unwrap_or("Untitled Session");
    let root_turns = count_turns(&root_sess);
    let is_current = if root_id == session_id { " [*]" } else { "" };

    output.push_str(&format!(
        "{} (\"{}\", {} turns){}\n",
        &root_id.to_string()[..8],
        root_title,
        root_turns,
        is_current
    ));

    fn format_node(
        output: &mut String,
        current_id: Uuid,
        active_id: Uuid,
        children_map: &HashMap<Uuid, Vec<SessionSummary>>,
        prefix: &str,
    ) {
        if let Some(children) = children_map.get(&current_id) {
            for (i, child) in children.iter().enumerate() {
                let is_last = i == children.len() - 1;
                let branch_symbol = if is_last { "└── " } else { "├── " };
                let child_prefix = if is_last { "    " } else { "│   " };

                let child_sess = Session::load(child.id).ok();
                let turns = child_sess
                    .as_ref()
                    .map(|s| count_turns(s))
                    .unwrap_or(0);
                let title = child
                    .title
                    .as_deref()
                    .unwrap_or("Untitled Branch");
                let is_active = if child.id == active_id { " [*]" } else { "" };

                output.push_str(&format!(
                    "{}{}{} (\"{}\", {} turns){}\n",
                    prefix,
                    branch_symbol,
                    &child.id.to_string()[..8],
                    title,
                    turns,
                    is_active
                ));

                format_node(
                    output,
                    child.id,
                    active_id,
                    children_map,
                    &format!("{}{}", prefix, child_prefix),
                );
            }
        }
    }

    format_node(&mut output, root_id, session_id, &children_map, "");
    Ok(output)
}

// ---------------------------------------------------------------------------
// Unit Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::provider::types::ToolCall;

    fn make_test_session() -> Session {
        let mut session = Session::new("test-model");
        session.set_title("Test Conversation");
        session.add_system_message("You are an AI assistant.");

        // Turn 1
        session.add_user_message("Hello, how are you?");
        session.add_assistant_message("I am doing great, thank you!");
        session.record_usage(100, 50);

        // Turn 2
        session.add_user_message("Can you list files?");
        session.add_assistant_with_tools(
            "Let me check.",
            vec![ToolCall {
                id: "call_1".to_string(),
                name: "bash".to_string(),
                arguments: "{\"command\": \"ls\"}".to_string(),
            }],
        );
        session.add_tool_result("call_1", "file1.txt\nfile2.txt");
        session.add_assistant_message("Found file1.txt and file2.txt.");
        session.record_usage(200, 100);

        // Turn 3
        session.add_user_message("Delete file2.txt");
        session.add_assistant_message("Deleted file2.txt.");
        session.record_usage(50, 25);

        session
    }

    #[test]
    fn test_extract_turns() {
        let session = make_test_session();
        let turns = extract_turns(session.messages());

        assert_eq!(turns.len(), 3);

        // Turn 1
        assert_eq!(turns[0].turn_index, 1);
        assert_eq!(turns[0].start_message_index, 1);
        assert_eq!(turns[0].end_message_index, 3);
        assert_eq!(
            turns[0].user_message.as_deref(),
            Some("Hello, how are you?")
        );
        assert_eq!(
            turns[0].assistant_message.as_deref(),
            Some("I am doing great, thank you!")
        );
        assert_eq!(turns[0].tool_calls_count, 0);
        assert_eq!(turns[0].message_count, 2);

        // Turn 2 (with tools)
        assert_eq!(turns[1].turn_index, 2);
        assert_eq!(turns[1].start_message_index, 3);
        assert_eq!(turns[1].end_message_index, 7);
        assert_eq!(
            turns[1].user_message.as_deref(),
            Some("Can you list files?")
        );
        assert_eq!(
            turns[1].assistant_message.as_deref(),
            Some("Found file1.txt and file2.txt.")
        );
        assert_eq!(turns[1].tool_calls_count, 1);
        assert_eq!(turns[1].message_count, 4);

        // Turn 3
        assert_eq!(turns[2].turn_index, 3);
        assert_eq!(turns[2].start_message_index, 7);
        assert_eq!(turns[2].end_message_index, 9);
        assert_eq!(turns[2].tool_calls_count, 0);
        assert_eq!(turns[2].message_count, 2);
    }

    #[test]
    fn test_rewind_in_place_single_turn() {
        let mut session = make_test_session();
        assert_eq!(count_turns(&session), 3);
        assert_eq!(session.messages.len(), 9);

        // Rewind 1 turn (revert Turn 3)
        let reverted = rewind_session_in_place(&mut session, 1);
        assert_eq!(reverted, 1);
        assert_eq!(count_turns(&session), 2);
        assert_eq!(session.messages.len(), 7);

        // Verify last message is Turn 2's assistant response
        assert_eq!(
            session.last_message().unwrap().content,
            "Found file1.txt and file2.txt."
        );
        assert_eq!(session.metadata.get("last_rewound_turns").unwrap(), "1");
    }

    #[test]
    fn test_rewind_in_place_multiple_turns() {
        let mut session = make_test_session();

        // Rewind 2 turns (reverts Turn 3 and Turn 2)
        let reverted = rewind_session_in_place(&mut session, 2);
        assert_eq!(reverted, 2);
        assert_eq!(count_turns(&session), 1);
        assert_eq!(session.messages.len(), 3);

        // Messages left: System, User 1, Assistant 1
        assert_eq!(session.messages[0].role, Role::System);
        assert_eq!(session.messages[1].content, "Hello, how are you?");
        assert_eq!(session.messages[2].content, "I am doing great, thank you!");
    }

    #[test]
    fn test_rewind_in_place_all_turns() {
        let mut session = make_test_session();

        // Rewind 10 turns (more than the 3 existing turns)
        let reverted = rewind_session_in_place(&mut session, 10);
        assert_eq!(reverted, 3);
        assert_eq!(count_turns(&session), 0);

        // Only the initial System message remains!
        assert_eq!(session.messages.len(), 1);
        assert_eq!(session.messages[0].role, Role::System);
        assert_eq!(session.messages[0].content, "You are an AI assistant.");
    }

    #[test]
    fn test_fork_session_at_current_turn() {
        let session = make_test_session();
        let forked = fork_session_in_memory(&session, None);

        assert_ne!(forked.id, session.id);
        assert_eq!(forked.messages.len(), session.messages.len());
        assert_eq!(count_turns(&forked), 3);
        assert_eq!(forked.title().unwrap(), "Test Conversation (branch)");
        assert_eq!(
            forked.metadata.get("forked_from_id").unwrap(),
            &session.id.to_string()
        );
        assert_eq!(forked.metadata.get("forked_at_turn").unwrap(), "3");
    }

    #[test]
    fn test_fork_session_at_historical_turn() {
        let session = make_test_session();

        // Fork at Turn 1 (should only keep Turn 1 messages + System message)
        let forked_t1 = fork_session_in_memory(&session, Some(1));
        assert_ne!(forked_t1.id, session.id);
        assert_eq!(count_turns(&forked_t1), 1);
        assert_eq!(forked_t1.messages.len(), 3); // System + User 1 + Assistant 1
        assert_eq!(
            forked_t1.title().unwrap(),
            "Test Conversation (branch @ turn 1)"
        );
        assert_eq!(forked_t1.metadata.get("forked_at_turn").unwrap(), "1");

        // Fork at Turn 2 (should keep System + Turn 1 + Turn 2)
        let forked_t2 = fork_session_in_memory(&session, Some(2));
        assert_eq!(count_turns(&forked_t2), 2);
        assert_eq!(forked_t2.messages.len(), 7); // System (1) + Turn 1 (2) + Turn 2 (4)
        assert_eq!(
            forked_t2.title().unwrap(),
            "Test Conversation (branch @ turn 2)"
        );
        assert_eq!(forked_t2.metadata.get("forked_at_turn").unwrap(), "2");

        // Fork at Turn 0 (prelude only)
        let forked_t0 = fork_session_in_memory(&session, Some(0));
        assert_eq!(count_turns(&forked_t0), 0);
        assert_eq!(forked_t0.messages.len(), 1); // Only System message
    }

    #[test]
    fn test_diff_session_branches() {
        let original = make_test_session();

        // Fork at Turn 2
        let mut branch = fork_session_in_memory(&original, Some(2));

        // In the branch, add a different Turn 3
        branch.add_user_message("Explain quantum computing");
        branch.add_assistant_message("Quantum computing leverages qubits...");

        let diff = diff_session_branches(&original, &branch);
        assert_eq!(diff.common_turns, 2);
        assert_eq!(diff.session_a_exclusive_turns, 1);
        assert_eq!(diff.session_b_exclusive_turns, 1);
        assert_eq!(diff.divergence_turn, Some(3));
    }

    #[test]
    fn test_disk_fork_and_rewind() {
        let temp_dir = std::env::temp_dir().join(format!("fusion_test_{}", Uuid::new_v4()));
        let _ = std::fs::create_dir_all(&temp_dir);

        let session = make_test_session();
        let session_file = temp_dir.join(format!("{}.json", session.id));
        session.save_to_path(&session_file).unwrap();

        // Test in-memory / file roundtrips
        let loaded = Session::load_from_path(&session_file).unwrap();
        assert_eq!(count_turns(&loaded), 3);

        let forked = fork_session_in_memory(&loaded, Some(2));
        let forked_file = temp_dir.join(format!("{}.json", forked.id));
        forked.save_to_path(&forked_file).unwrap();

        let loaded_fork = Session::load_from_path(&forked_file).unwrap();
        assert_eq!(count_turns(&loaded_fork), 2);
        assert_eq!(
            loaded_fork.metadata.get("forked_from_id").unwrap(),
            &session.id.to_string()
        );

        // Clean up
        let _ = std::fs::remove_dir_all(&temp_dir);
    }
}
