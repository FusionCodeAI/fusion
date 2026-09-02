//! Session bookmarks and turn checkpointing subsystem for Fusion.
//!
//! Provides capabilities to:
//! - Pin important conversation turns with `/bookmark <name>`.
//! - Save named checkpoints with snapshots of conversation history.
//! - Recall bookmark metadata, turn previews, token statistics, and session drift.
//! - Restore/rewind active sessions back to bookmarked points.
//! - Fork new branched sessions originating from specific bookmarks.
//! - Tag, search, filter, export, and import session bookmarks.
//! - Manage codebase bookmarks and annotations (file path, lines, snippet, tags, notes).
//! - Search/filter codebase bookmarks by tag, keyword, or path prefix.
//! - Export codebase bookmarks to Markdown summaries.

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use chrono::Utc;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::agent::fork::{count_turns, extract_turns, rewind_session_in_place, SessionTurn};
use crate::agent::session::Session;
use crate::config::Config;
use crate::provider::types::Message;

/// Well-known metadata key used to persist session bookmarks inside `Session::metadata`.
pub const BOOKMARKS_METADATA_KEY: &str = "fusion:bookmarks";

/// Well-known metadata key used to persist codebase bookmarks inside `Session::metadata`.
pub const CODE_BOOKMARKS_METADATA_KEY: &str = "fusion:code_bookmarks";

// ============================================================================
// Core Session Bookmark Data Structures
// ============================================================================

/// Categorization of a session bookmark.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BookmarkKind {
    /// Standard bookmark referencing a specific conversation turn.
    Turn,
    /// Full checkpoint capturing a restorable snapshot of conversation state.
    Checkpoint,
    /// Pinned turn highlighted in transcripts and protected during compaction.
    Pinned,
    /// User-annotated note attached to a specific turn.
    Note,
}

impl BookmarkKind {
    /// Short string representation of the bookmark kind.
    pub fn as_str(&self) -> &'static str {
        match self {
            BookmarkKind::Turn => "turn",
            BookmarkKind::Checkpoint => "checkpoint",
            BookmarkKind::Pinned => "pinned",
            BookmarkKind::Note => "note",
        }
    }

    /// Human-friendly display label with icon.
    pub fn display_label(&self) -> &'static str {
        match self {
            BookmarkKind::Turn => "🔖 Turn",
            BookmarkKind::Checkpoint => "💾 Checkpoint",
            BookmarkKind::Pinned => "📌 Pinned",
            BookmarkKind::Note => "📝 Note",
        }
    }

    /// Parse kind loosely from string.
    pub fn from_str_loose(s: &str) -> Option<Self> {
        match s.trim().to_lowercase().as_str() {
            "turn" | "t" | "bookmark" => Some(BookmarkKind::Turn),
            "checkpoint" | "cp" | "save" | "snapshot" => Some(BookmarkKind::Checkpoint),
            "pinned" | "pin" | "p" => Some(BookmarkKind::Pinned),
            "note" | "memo" | "n" => Some(BookmarkKind::Note),
            _ => None,
        }
    }
}

/// A snapshot of conversation state saved within a checkpoint bookmark.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BookmarkSnapshot {
    /// Messages in the session at the time the snapshot was captured.
    pub messages: Vec<Message>,
    /// Active model identifier when snapshotted.
    pub active_model: String,
    /// Token usage stats at the time of snapshot.
    pub token_stats: crate::agent::session::TokenStats,
    /// System prompt active at the time of snapshot.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub system_prompt: Option<String>,
    /// 1-based turn index corresponding to this snapshot.
    pub turn_index: usize,
    /// Timestamp when snapshot was captured.
    pub timestamp: String,
}

/// Represents a pinned conversation turn, checkpoint, or annotated bookmark.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Bookmark {
    /// Unique identifier for this bookmark (UUID v4).
    pub id: Uuid,
    /// Human-readable bookmark label/name (unique within session, case-insensitive).
    pub name: String,
    /// ID of the session containing this bookmark.
    pub session_id: Uuid,
    /// 1-based conversational turn index (1, 2, 3...).
    pub turn_index: usize,
    /// Index in `session.messages` corresponding to the start of this turn.
    pub message_index: usize,
    /// Total number of messages in session at the time of bookmarking.
    pub message_count: usize,
    /// Classification of bookmark (Turn, Checkpoint, Pinned, Note).
    pub kind: BookmarkKind,
    /// Optional user memo or description.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    /// RFC 3339 timestamp when the bookmark was created.
    pub created_at: String,
    /// RFC 3339 timestamp when the bookmark was last updated.
    pub updated_at: String,
    /// Optional list of categorizing tags (e.g. `["refactor", "bugfix"]`).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
    /// Truncated snippet of the user query at this turn.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user_preview: Option<String>,
    /// Truncated snippet of the assistant response at this turn.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub assistant_preview: Option<String>,
    /// Estimated cumulative token count in session up to this bookmark point.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tokens_at_bookmark: Option<usize>,
    /// Full message history snapshot (populated for Checkpoint bookmarks).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub snapshot: Option<BookmarkSnapshot>,
    /// Arbitrary bookmark metadata key-value pairs.
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub metadata: HashMap<String, String>,
}

impl Bookmark {
    /// Creates a new Bookmark on a specific turn.
    pub fn new(
        session: &Session,
        name: impl Into<String>,
        turn_index: usize,
        kind: BookmarkKind,
    ) -> Self {
        let now = Utc::now().to_rfc3339();
        let turns = extract_turns(session.messages());
        let total_turns = turns.len();

        let (msg_idx, msg_cnt, user_prev, asst_prev) = if turn_index > 0 && turn_index <= total_turns {
            let t = &turns[turn_index - 1];
            let u_prev = t.user_message.as_deref().map(|s| truncate_string(s, 80));
            let a_prev = t.assistant_message.as_deref().map(|s| truncate_string(s, 80));
            (t.start_message_index, t.end_message_index, u_prev, a_prev)
        } else {
            let count = session.total_messages();
            (count.saturating_sub(1), count, None, None)
        };

        // Estimate tokens up to message_count
        let msgs_slice = &session.messages()[..msg_cnt.min(session.total_messages())];
        let tokens = Some(crate::agent::tokens::estimate_messages_tokens(msgs_slice));

        Self {
            id: Uuid::new_v4(),
            name: name.into().trim().to_string(),
            session_id: session.id(),
            turn_index: if turn_index == 0 { 1 } else { turn_index },
            message_index: msg_idx,
            message_count: msg_cnt,
            kind,
            note: None,
            created_at: now.clone(),
            updated_at: now,
            tags: Vec::new(),
            user_preview: user_prev,
            assistant_preview: asst_prev,
            tokens_at_bookmark: tokens,
            snapshot: None,
            metadata: HashMap::new(),
        }
    }

    /// Attaches an optional note/memo to this bookmark.
    pub fn with_note(mut self, note: impl Into<String>) -> Self {
        let n = note.into().trim().to_string();
        if !n.is_empty() {
            self.note = Some(n);
        }
        self
    }

    /// Attaches tags to this bookmark.
    pub fn with_tags(mut self, tags: Vec<String>) -> Self {
        self.tags = tags;
        self
    }

    /// Captures full snapshot for restorable checkpointing.
    pub fn with_snapshot(mut self, session: &Session) -> Self {
        let now = Utc::now().to_rfc3339();
        let end_idx = self.message_count.min(session.total_messages());
        let msgs = session.messages()[..end_idx].to_vec();

        self.snapshot = Some(BookmarkSnapshot {
            messages: msgs,
            active_model: session.active_model().to_string(),
            token_stats: *session.token_stats(),
            system_prompt: session.system_prompt().map(|s| s.to_string()),
            turn_index: self.turn_index,
            timestamp: now,
        });
        self
    }

    /// Formats a concise single-line summary of this bookmark.
    pub fn summary_line(&self) -> String {
        let note_part = self
            .note
            .as_deref()
            .map(|n| format!(" - \"{}\"", truncate_string(n, 40)))
            .unwrap_or_default();

        let tags_part = if self.tags.is_empty() {
            String::new()
        } else {
            format!(" [{}]", self.tags.join(", "))
        };

        format!(
            "{} \x1b[1m{}\x1b[0m (Turn {}){}{}",
            self.kind.display_label(),
            self.name,
            self.turn_index,
            note_part,
            tags_part
        )
    }
}

/// Detailed recall report for a requested bookmark, comparing it to current session state.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BookmarkRecall {
    /// The retrieved bookmark record.
    pub bookmark: Bookmark,
    /// Details about the turn at the time of bookmarking.
    pub turn: Option<SessionTurn>,
    /// Turn index of bookmark (1-based).
    pub turn_index: usize,
    /// Total turns currently in the active session.
    pub current_total_turns: usize,
    /// Number of turns completed after this bookmark (0 if currently at this turn).
    pub turns_behind: usize,
    /// Number of messages added after this bookmark.
    pub messages_behind: usize,
    /// Estimated token count when the bookmark was placed.
    pub tokens_at_bookmark: usize,
    /// Current estimated token count in active session.
    pub current_tokens: usize,
    /// Human-readable multi-line preview text.
    pub preview: String,
    /// Whether this bookmark can be cleanly restored or rewound to.
    pub is_restorable: bool,
}

/// Filter criteria for querying session bookmarks.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct BookmarkFilter {
    /// Specific bookmark kind to match.
    pub kind: Option<BookmarkKind>,
    /// Tag that must be present.
    pub tag: Option<String>,
    /// Case-insensitive search query matching name, note, or turn preview.
    pub search_query: Option<String>,
    /// Minimum turn index.
    pub min_turn: Option<usize>,
    /// Maximum turn index.
    pub max_turn: Option<usize>,
}

// ============================================================================
// Core Session Bookmark Operations on Session
// ============================================================================

/// Reads all bookmarks currently saved in a session's metadata.
pub fn list_bookmarks(session: &Session) -> Vec<Bookmark> {
    if let Some(json_str) = session.get_metadata(BOOKMARKS_METADATA_KEY) {
        if let Ok(bookmarks) = serde_json::from_str::<Vec<Bookmark>>(json_str) {
            return bookmarks;
        }
    }
    Vec::new()
}

/// Persists a list of bookmarks into a session's metadata.
pub fn save_bookmarks_to_session(session: &mut Session, bookmarks: &[Bookmark]) -> anyhow::Result<()> {
    let json_str = serde_json::to_string(bookmarks)?;
    session.set_metadata(BOOKMARKS_METADATA_KEY, json_str);
    Ok(())
}

/// Pins or bookmarks the latest completed conversational turn in the session.
pub fn bookmark_turn(
    session: &mut Session,
    name: &str,
    note: Option<&str>,
    kind: BookmarkKind,
) -> anyhow::Result<Bookmark> {
    let turns = extract_turns(session.messages());
    let turn_idx = if turns.is_empty() { 1 } else { turns.len() };
    bookmark_specific_turn(session, turn_idx, name, note, kind)
}

/// Pins or bookmarks a specific 1-based conversational turn in the session.
pub fn bookmark_specific_turn(
    session: &mut Session,
    turn_index: usize,
    name: &str,
    note: Option<&str>,
    kind: BookmarkKind,
) -> anyhow::Result<Bookmark> {
    let clean_name = name.trim();
    if clean_name.is_empty() {
        anyhow::bail!("Bookmark name cannot be empty");
    }

    let mut bookmarks = list_bookmarks(session);

    // Check if bookmark name already exists (case-insensitive)
    if let Some(pos) = bookmarks.iter().position(|b| b.name.eq_ignore_ascii_case(clean_name)) {
        // Update existing bookmark in place
        let mut b = Bookmark::new(session, clean_name, turn_index, kind);
        if let Some(n) = note {
            b = b.with_note(n);
        }
        if kind == BookmarkKind::Checkpoint {
            b = b.with_snapshot(session);
        }
        bookmarks[pos] = b.clone();
        save_bookmarks_to_session(session, &bookmarks)?;
        return Ok(b);
    }

    let mut bookmark = Bookmark::new(session, clean_name, turn_index, kind);
    if let Some(n) = note {
        bookmark = bookmark.with_note(n);
    }
    if kind == BookmarkKind::Checkpoint {
        bookmark = bookmark.with_snapshot(session);
    }

    bookmarks.push(bookmark.clone());
    save_bookmarks_to_session(session, &bookmarks)?;
    Ok(bookmark)
}

/// Creates a full restorable Checkpoint bookmark with state snapshot.
pub fn bookmark_checkpoint(
    session: &mut Session,
    name: &str,
    note: Option<&str>,
) -> anyhow::Result<Bookmark> {
    bookmark_turn(session, name, note, BookmarkKind::Checkpoint)
}

/// Pins a specific turn in the session to protect it from compaction and highlight it.
pub fn pin_turn(session: &mut Session, turn_index: usize, name: &str) -> anyhow::Result<Bookmark> {
    bookmark_specific_turn(session, turn_index, name, None, BookmarkKind::Pinned)
}

/// Unpins a specific turn if a pinned bookmark exists for it.
pub fn unpin_turn(session: &mut Session, turn_index: usize) -> bool {
    let mut bookmarks = list_bookmarks(session);
    let orig_len = bookmarks.len();
    bookmarks.retain(|b| !(b.turn_index == turn_index && b.kind == BookmarkKind::Pinned));
    if bookmarks.len() != orig_len {
        let _ = save_bookmarks_to_session(session, &bookmarks);
        true
    } else {
        false
    }
}

/// Checks if a specific 1-based turn index is currently pinned.
pub fn is_turn_pinned(session: &Session, turn_index: usize) -> bool {
    list_bookmarks(session)
        .iter()
        .any(|b| b.turn_index == turn_index && b.kind == BookmarkKind::Pinned)
}

/// Returns a list of all pinned 1-based turn indices in the session.
pub fn get_pinned_turns(session: &Session) -> Vec<usize> {
    let mut pinned: Vec<usize> = list_bookmarks(session)
        .into_iter()
        .filter(|b| b.kind == BookmarkKind::Pinned)
        .map(|b| b.turn_index)
        .collect();
    pinned.sort_unstable();
    pinned.dedup();
    pinned
}

/// Finds a bookmark by name (case-insensitive) or full UUID / prefix.
pub fn get_bookmark(session: &Session, name_or_id: &str) -> Option<Bookmark> {
    let query = name_or_id.trim();
    let bookmarks = list_bookmarks(session);

    // 1. Match exact or case-insensitive name
    if let Some(b) = bookmarks.iter().find(|b| b.name.eq_ignore_ascii_case(query)) {
        return Some(b.clone());
    }

    // 2. Match full UUID
    if let Ok(uuid) = Uuid::parse_str(query) {
        if let Some(b) = bookmarks.iter().find(|b| b.id == uuid) {
            return Some(b.clone());
        }
    }

    // 3. Match UUID prefix (at least 4 chars)
    if query.len() >= 4 {
        let query_lower = query.to_lowercase();
        if let Some(b) = bookmarks
            .iter()
            .find(|b| b.id.to_string().to_lowercase().starts_with(&query_lower))
        {
            return Some(b.clone());
        }
    }

    None
}

/// Finds a bookmark by turn index.
pub fn get_bookmark_by_turn(session: &Session, turn_index: usize) -> Option<Bookmark> {
    list_bookmarks(session)
        .into_iter()
        .find(|b| b.turn_index == turn_index)
}

/// Deletes a bookmark by name or UUID.
pub fn delete_bookmark(session: &mut Session, name_or_id: &str) -> Option<Bookmark> {
    let query = name_or_id.trim();
    let mut bookmarks = list_bookmarks(session);
    let pos = bookmarks.iter().position(|b| {
        b.name.eq_ignore_ascii_case(query)
            || b.id.to_string().eq_ignore_ascii_case(query)
            || (query.len() >= 4 && b.id.to_string().starts_with(query))
    })?;

    let removed = bookmarks.remove(pos);
    let _ = save_bookmarks_to_session(session, &bookmarks);
    Some(removed)
}

/// Clears all bookmarks from the session. Returns number of removed bookmarks.
pub fn clear_bookmarks(session: &mut Session) -> usize {
    let count = list_bookmarks(session).len();
    session.metadata.remove(BOOKMARKS_METADATA_KEY);
    session.touch();
    count
}

/// Renames an existing bookmark.
pub fn rename_bookmark(
    session: &mut Session,
    old_name: &str,
    new_name: &str,
) -> anyhow::Result<()> {
    let clean_new = new_name.trim();
    if clean_new.is_empty() {
        anyhow::bail!("New bookmark name cannot be empty");
    }

    let mut bookmarks = list_bookmarks(session);
    if bookmarks.iter().any(|b| b.name.eq_ignore_ascii_case(clean_new)) {
        anyhow::bail!("A bookmark named '{}' already exists", clean_new);
    }

    let pos = bookmarks
        .iter()
        .position(|b| b.name.eq_ignore_ascii_case(old_name.trim()))
        .ok_or_else(|| anyhow::anyhow!("Bookmark '{}' not found", old_name))?;

    bookmarks[pos].name = clean_new.to_string();
    bookmarks[pos].updated_at = Utc::now().to_rfc3339();
    save_bookmarks_to_session(session, &bookmarks)?;
    Ok(())
}

/// Adds a tag to a session bookmark.
pub fn tag_bookmark(session: &mut Session, name_or_id: &str, tag: &str) -> anyhow::Result<()> {
    let clean_tag = tag.trim().to_lowercase();
    if clean_tag.is_empty() {
        anyhow::bail!("Tag cannot be empty");
    }

    let mut bookmarks = list_bookmarks(session);
    let pos = bookmarks
        .iter()
        .position(|b| b.name.eq_ignore_ascii_case(name_or_id.trim()) || b.id.to_string() == name_or_id.trim())
        .ok_or_else(|| anyhow::anyhow!("Bookmark '{}' not found", name_or_id))?;

    if !bookmarks[pos].tags.iter().any(|t| t.eq_ignore_ascii_case(&clean_tag)) {
        bookmarks[pos].tags.push(clean_tag);
        bookmarks[pos].updated_at = Utc::now().to_rfc3339();
        save_bookmarks_to_session(session, &bookmarks)?;
    }
    Ok(())
}

/// Removes a tag from a session bookmark.
pub fn untag_bookmark(session: &mut Session, name_or_id: &str, tag: &str) -> anyhow::Result<()> {
    let clean_tag = tag.trim();
    let mut bookmarks = list_bookmarks(session);
    let pos = bookmarks
        .iter()
        .position(|b| b.name.eq_ignore_ascii_case(name_or_id.trim()) || b.id.to_string() == name_or_id.trim())
        .ok_or_else(|| anyhow::anyhow!("Bookmark '{}' not found", name_or_id))?;

    bookmarks[pos].tags.retain(|t| !t.eq_ignore_ascii_case(clean_tag));
    bookmarks[pos].updated_at = Utc::now().to_rfc3339();
    save_bookmarks_to_session(session, &bookmarks)?;
    Ok(())
}

/// Updates the note/memo on a session bookmark.
pub fn update_bookmark_note(
    session: &mut Session,
    name_or_id: &str,
    note: &str,
) -> anyhow::Result<()> {
    let mut bookmarks = list_bookmarks(session);
    let pos = bookmarks
        .iter()
        .position(|b| b.name.eq_ignore_ascii_case(name_or_id.trim()) || b.id.to_string() == name_or_id.trim())
        .ok_or_else(|| anyhow::anyhow!("Bookmark '{}' not found", name_or_id))?;

    let clean = note.trim();
    bookmarks[pos].note = if clean.is_empty() {
        None
    } else {
        Some(clean.to_string())
    };
    bookmarks[pos].updated_at = Utc::now().to_rfc3339();
    save_bookmarks_to_session(session, &bookmarks)?;
    Ok(())
}

// ============================================================================
// Session Bookmark Recall & Restoration
// ============================================================================

/// Recalls a bookmark and produces a comprehensive analysis comparing it to current session state.
pub fn recall_bookmark(session: &Session, name_or_id: &str) -> anyhow::Result<BookmarkRecall> {
    let bookmark = get_bookmark(session, name_or_id)
        .ok_or_else(|| anyhow::anyhow!("Bookmark '{}' not found in this session", name_or_id))?;

    let turns = extract_turns(session.messages());
    let current_total_turns = turns.len();
    let current_messages = session.total_messages();
    let current_tokens = crate::agent::tokens::estimate_messages_tokens(session.messages());

    let turn = if bookmark.turn_index > 0 && bookmark.turn_index <= current_total_turns {
        Some(turns[bookmark.turn_index - 1].clone())
    } else {
        None
    };

    let turns_behind = current_total_turns.saturating_sub(bookmark.turn_index);
    let messages_behind = current_messages.saturating_sub(bookmark.message_count);
    let tokens_at_bm = bookmark.tokens_at_bookmark.unwrap_or(0);

    let mut preview_lines = Vec::new();
    preview_lines.push(format!(
        "🔖 Bookmark: \x1b[1m{}\x1b[0m ({})",
        bookmark.name,
        bookmark.kind.display_label()
    ));
    preview_lines.push(format!(
        "   Turn {} of {} | Created: {}",
        bookmark.turn_index, current_total_turns, bookmark.created_at
    ));

    if let Some(note) = &bookmark.note {
        preview_lines.push(format!("   Note: \"{}\"", note));
    }

    if !bookmark.tags.is_empty() {
        preview_lines.push(format!("   Tags: {}", bookmark.tags.join(", ")));
    }

    if let Some(turn_info) = &turn {
        if let Some(u) = &turn_info.user_message {
            preview_lines.push(format!("   👤 User: \"{}\"", truncate_string(u, 100)));
        }
        if let Some(a) = &turn_info.assistant_message {
            preview_lines.push(format!("   🤖 Assistant: \"{}\"", truncate_string(a, 100)));
        }
        if turn_info.tool_calls_count > 0 {
            preview_lines.push(format!("   ⚙️ Tools: {} calls", turn_info.tool_calls_count));
        }
    } else if let Some(snap) = &bookmark.snapshot {
        preview_lines.push(format!(
            "   💾 Restorable Snapshot: {} messages saved",
            snap.messages.len()
        ));
    }

    if turns_behind > 0 {
        preview_lines.push(format!(
            "   ⏳ Drift: {} turns behind current head ({} messages, +{} tokens)",
            turns_behind,
            messages_behind,
            current_tokens.saturating_sub(tokens_at_bm)
        ));
    } else {
        preview_lines.push("   📍 Status: Bookmark is at the current conversation head".to_string());
    }

    let is_restorable = bookmark.snapshot.is_some() || bookmark.turn_index <= current_total_turns;
    let turn_index = bookmark.turn_index;
    Ok(BookmarkRecall {
        bookmark,
        turn,
        turn_index,
        current_total_turns,
        turns_behind,
        messages_behind,
        tokens_at_bookmark: tokens_at_bm,
        current_tokens,
        preview: preview_lines.join("\n"),
        is_restorable,
    })
}

/// Restores or rewinds the active session in-place back to the bookmarked turn or checkpoint snapshot.
///
/// Returns the number of turns reverted.
pub fn restore_to_bookmark(session: &mut Session, name_or_id: &str) -> anyhow::Result<usize> {
    let recall = recall_bookmark(session, name_or_id)?;

    // If bookmark has a full state snapshot (Checkpoint), restore exact messages
    if let Some(snap) = &recall.bookmark.snapshot {
        session.messages = snap.messages.clone();
        session.active_model = snap.active_model.clone();
        session.token_stats = snap.token_stats;
        if let Some(sp) = &snap.system_prompt {
            session.system_prompt = Some(sp.clone());
        }
        session.touch();
        return Ok(recall.turns_behind);
    }

    // Otherwise rewind turns in place
    if recall.turns_behind == 0 {
        return Ok(0);
    }

    let reverted = rewind_session_in_place(session, recall.turns_behind);
    Ok(reverted)
}

/// Forks a new branched session starting from a bookmarked turn.
pub fn fork_from_bookmark(
    session: &Session,
    name_or_id: &str,
    new_title: Option<&str>,
) -> anyhow::Result<Session> {
    let bookmark = get_bookmark(session, name_or_id)
        .ok_or_else(|| anyhow::anyhow!("Bookmark '{}' not found", name_or_id))?;

    let title = new_title
        .map(|s| s.to_string())
        .or_else(|| {
            session
                .title()
                .map(|t| format!("{} (from bookmark {})", t, bookmark.name))
        })
        .unwrap_or_else(|| format!("Branch from {}", bookmark.name));

    let mut forked = crate::agent::fork::fork_session_in_memory(session, Some(bookmark.turn_index));
    forked.set_title(title);
    Ok(forked)
}

// ============================================================================
// Session Bookmark Searching, Filtering & Querying
// ============================================================================

/// Searches session bookmarks by matching a query string against name, note, previews, and tags.
pub fn search_bookmarks(session: &Session, query: &str) -> Vec<Bookmark> {
    let q = query.trim().to_lowercase();
    if q.is_empty() {
        return list_bookmarks(session);
    }

    list_bookmarks(session)
        .into_iter()
        .filter(|b| {
            b.name.to_lowercase().contains(&q)
                || b.note
                    .as_deref()
                    .map(|n| n.to_lowercase().contains(&q))
                    .unwrap_or(false)
                || b.tags.iter().any(|t| t.to_lowercase().contains(&q))
                || b.user_preview
                    .as_deref()
                    .map(|u| u.to_lowercase().contains(&q))
                    .unwrap_or(false)
                || b.assistant_preview
                    .as_deref()
                    .map(|a| a.to_lowercase().contains(&q))
                    .unwrap_or(false)
        })
        .collect()
}

/// Filters session bookmarks according to a structured `BookmarkFilter`.
pub fn filter_bookmarks(session: &Session, filter: &BookmarkFilter) -> Vec<Bookmark> {
    list_bookmarks(session)
        .into_iter()
        .filter(|b| {
            if let Some(kind) = filter.kind {
                if b.kind != kind {
                    return false;
                }
            }
            if let Some(tag) = &filter.tag {
                if !b.tags.iter().any(|t| t.eq_ignore_ascii_case(tag)) {
                    return false;
                }
            }
            if let Some(min_t) = filter.min_turn {
                if b.turn_index < min_t {
                    return false;
                }
            }
            if let Some(max_t) = filter.max_turn {
                if b.turn_index > max_t {
                    return false;
                }
            }
            if let Some(q) = &filter.search_query {
                let q_lower = q.to_lowercase();
                let matches_name = b.name.to_lowercase().contains(&q_lower);
                let matches_note = b
                    .note
                    .as_deref()
                    .map(|n| n.to_lowercase().contains(&q_lower))
                    .unwrap_or(false);
                let matches_tag = b.tags.iter().any(|t| t.to_lowercase().contains(&q_lower));
                if !matches_name && !matches_note && !matches_tag {
                    return false;
                }
            }
            true
        })
        .collect()
}

/// Filters session bookmarks with a specific tag.
pub fn filter_bookmarks_by_tag(session: &Session, tag: &str) -> Vec<Bookmark> {
    filter_bookmarks(
        session,
        &BookmarkFilter {
            tag: Some(tag.to_string()),
            ..Default::default()
        },
    )
}

// ============================================================================
// Session Bookmark Formatting & Export
// ============================================================================

/// Formats a list of session bookmarks into a clean ANSI-colored CLI table.
pub fn format_bookmarks_table(bookmarks: &[Bookmark]) -> String {
    if bookmarks.is_empty() {
        return "\x1b[2;37mNo bookmarks saved in this session.\x1b[0m\n\
                Use \x1b[1;36m/bookmark <name>\x1b[0m to bookmark the current turn."
            .to_string();
    }

    let mut out = String::new();
    out.push_str("\x1b[1;36mSession Bookmarks & Checkpoints:\x1b[0m\n");
    out.push_str(&format!(
        "  {:<18} {:<14} {:<6} {:<24} {:<20}\n",
        "Name", "Type", "Turn", "Note / Preview", "Tags"
    ));
    out.push_str(&format!("  {}\n", "─".repeat(84)));

    for b in bookmarks {
        let note_or_preview = if let Some(n) = &b.note {
            truncate_string(n, 22)
        } else if let Some(u) = &b.user_preview {
            truncate_string(u, 22)
        } else {
            "-".to_string()
        };

        let tags_str = if b.tags.is_empty() {
            "-".to_string()
        } else {
            truncate_string(&b.tags.join(", "), 18)
        };

        let type_color = match b.kind {
            BookmarkKind::Checkpoint => "\x1b[1;32m", // Green
            BookmarkKind::Pinned => "\x1b[1;33m",     // Yellow
            BookmarkKind::Turn => "\x1b[1;34m",       // Blue
            BookmarkKind::Note => "\x1b[1;35m",       // Magenta
        };

        out.push_str(&format!(
            "  \x1b[1m{:<18}\x1b[0m {}{:<14}\x1b[0m {:<6} {:<24} {:<20}\n",
            truncate_string(&b.name, 17),
            type_color,
            b.kind.display_label(),
            format!("T{}", b.turn_index),
            note_or_preview,
            tags_str
        ));
    }

    out
}

/// Formats detailed bookmark recall information for display in the REPL.
pub fn format_bookmark_detail(recall: &BookmarkRecall) -> String {
    let mut out = String::new();
    out.push_str(&recall.preview);
    out.push_str("\n\n\x1b[2;37mActions:\x1b[0m\n");
    out.push_str(&format!(
        "  • Restore session:  \x1b[1;36m/bookmark restore {}\x1b[0m\n",
        recall.bookmark.name
    ));
    out.push_str(&format!(
        "  • Fork into branch: \x1b[1;36m/bookmark fork {}\x1b[0m\n",
        recall.bookmark.name
    ));
    out.push_str(&format!(
        "  • Delete bookmark:  \x1b[1;36m/bookmark delete {}\x1b[0m",
        recall.bookmark.name
    ));
    out
}

/// Exports all session bookmarks into a Markdown document.
pub fn export_bookmarks_markdown(session: &Session) -> String {
    let bookmarks = list_bookmarks(session);
    let title = session.title().unwrap_or("Fusion Session");
    let mut md = String::new();

    md.push_str(&format!("# Bookmarks for Session: {}\n\n", title));
    md.push_str(&format!("**Session ID:** `{}`  \n", session.id()));
    md.push_str(&format!("**Total Bookmarks:** {}  \n\n", bookmarks.len()));

    if bookmarks.is_empty() {
        md.push_str("*No bookmarks recorded for this session.*\n");
        return md;
    }

    md.push_str("| Name | Type | Turn | Created | Note | Tags |\n");
    md.push_str("| --- | --- | --- | --- | --- | --- |\n");

    for b in &bookmarks {
        let note = b.note.as_deref().unwrap_or("-").replace('|', "\\|");
        let tags = if b.tags.is_empty() {
            "-".to_string()
        } else {
            b.tags.join(", ")
        };
        md.push_str(&format!(
            "| **{}** | {} | Turn {} | {} | {} | {} |\n",
            b.name,
            b.kind.display_label(),
            b.turn_index,
            b.created_at,
            note,
            tags
        ));
    }

    md.push_str("\n---\n\n");
    for b in &bookmarks {
        md.push_str(&format!("### 🔖 {}\n\n", b.name));
        md.push_str(&format!("- **Kind:** {}\n", b.kind.display_label()));
        md.push_str(&format!("- **Turn Index:** {}\n", b.turn_index));
        md.push_str(&format!("- **Created:** {}\n", b.created_at));
        if let Some(n) = &b.note {
            md.push_str(&format!("- **Note:** {}\n", n));
        }
        if let Some(u) = &b.user_preview {
            md.push_str(&format!("- **User Prompt:** {}\n", u));
        }
        if let Some(a) = &b.assistant_preview {
            md.push_str(&format!("- **Assistant Response:** {}\n", a));
        }
        md.push('\n');
    }

    md
}

/// Exports session bookmarks as formatted JSON.
pub fn export_bookmarks_json(session: &Session) -> anyhow::Result<String> {
    let bookmarks = list_bookmarks(session);
    Ok(serde_json::to_string_pretty(&bookmarks)?)
}

/// Imports session bookmarks from JSON and adds them into session metadata.
pub fn import_bookmarks_json(session: &mut Session, json_str: &str) -> anyhow::Result<usize> {
    let imported: Vec<Bookmark> = serde_json::from_str(json_str)?;
    let count = imported.len();
    let mut existing = list_bookmarks(session);

    for new_bm in imported {
        if let Some(pos) = existing.iter().position(|b| b.name.eq_ignore_ascii_case(&new_bm.name)) {
            existing[pos] = new_bm;
        } else {
            existing.push(new_bm);
        }
    }

    save_bookmarks_to_session(session, &existing)?;
    Ok(count)
}

// ============================================================================
// Disk Persistence Helpers for Session Bookmarks
// ============================================================================

/// Returns directory where standalone bookmark snapshots are stored: `~/.fusion/bookmarks`
pub fn bookmarks_dir() -> PathBuf {
    Config::config_dir().join("bookmarks")
}

/// Saves standalone session bookmarks file to `~/.fusion/bookmarks/<session_id>.json`.
pub fn save_bookmarks_to_disk(session: &Session) -> anyhow::Result<PathBuf> {
    let dir = bookmarks_dir();
    if !dir.exists() {
        fs::create_dir_all(&dir)?;
    }
    let path = dir.join(format!("{}.bookmarks.json", session.id()));
    let json = export_bookmarks_json(session)?;
    fs::write(&path, json)?;
    Ok(path)
}

/// Loads session bookmarks from standalone disk file if available.
pub fn load_bookmarks_from_disk(session_id: Uuid) -> anyhow::Result<Vec<Bookmark>> {
    let path = bookmarks_dir().join(format!("{}.bookmarks.json", session_id));
    if !path.exists() {
        return Ok(Vec::new());
    }
    let content = fs::read_to_string(path)?;
    let bookmarks: Vec<Bookmark> = serde_json::from_str(&content)?;
    Ok(bookmarks)
}

// ============================================================================
// Interactive Slash Command Handler
// ============================================================================

/// Handles interactive `/bookmark` slash command execution.
///
/// Supported forms:
/// - `/bookmark <name>` -> bookmarks current turn with `<name>`
/// - `/bookmark <name> <note>` -> bookmarks current turn with note
/// - `/bookmark turn <turn_idx> <name>` -> bookmarks specific turn
/// - `/bookmark checkpoint <name>` -> creates full restorable checkpoint
/// - `/bookmark pin [turn_idx]` -> pins turn
/// - `/bookmark list` / `/bookmark ls` -> lists all bookmarks
/// - `/bookmark recall <name>` / `/bookmark show <name>` -> detailed recall
/// - `/bookmark restore <name>` / `/bookmark jump <name>` -> restores session
/// - `/bookmark fork <name> [new_title]` -> forks session from bookmark
/// - `/bookmark tag <name> <tag>` -> tags bookmark
/// - `/bookmark untag <name> <tag>` -> untags bookmark
/// - `/bookmark delete <name>` / `/bookmark rm <name>` -> deletes bookmark
/// - `/bookmark clear` -> clears all bookmarks
/// - `/bookmark export [path]` -> exports bookmarks
/// - `/bookmark help` -> shows help
pub fn handle_bookmark_command(args: &[String], session: &mut Session) -> String {
    if args.is_empty() {
        let bookmarks = list_bookmarks(session);
        return format_bookmarks_table(&bookmarks);
    }

    let subcmd = args[0].to_lowercase();
    match subcmd.as_str() {
        "list" | "ls" => {
            let bookmarks = list_bookmarks(session);
            format_bookmarks_table(&bookmarks)
        }
        "help" | "-h" | "--help" => format_bookmark_help(),
        "clear" => {
            let removed = clear_bookmarks(session);
            format!("\x1b[1;32mCleared {} bookmark(s) from session.\x1b[0m", removed)
        }
        "checkpoint" | "cp" => {
            if args.len() < 2 {
                return "\x1b[1;31mUsage:\x1b[0m /bookmark checkpoint <name> [note]".to_string();
            }
            let name = &args[1];
            let note = if args.len() > 2 {
                Some(args[2..].join(" "))
            } else {
                None
            };
            match bookmark_checkpoint(session, name, note.as_deref()) {
                Ok(b) => format!(
                    "\x1b[1;32mCheckpoint saved:\x1b[0m 💾 \x1b[1m{}\x1b[0m (Turn {}) with full snapshot",
                    b.name, b.turn_index
                ),
                Err(e) => format!("\x1b[1;31mFailed to create checkpoint:\x1b[0m {}", e),
            }
        }
        "pin" => {
            let turns = count_turns(session);
            let (turn_idx, name) = if args.len() >= 3 {
                let t = args[1].parse::<usize>().unwrap_or(turns);
                (t, args[2].clone())
            } else if args.len() == 2 {
                if let Ok(t) = args[1].parse::<usize>() {
                    (t, format!("pinned-turn-{}", t))
                } else {
                    (turns, args[1].clone())
                }
            } else {
                (turns, format!("pinned-turn-{}", turns))
            };

            match pin_turn(session, turn_idx, &name) {
                Ok(b) => format!(
                    "\x1b[1;32mPinned turn:\x1b[0m 📌 \x1b[1m{}\x1b[0m (Turn {})",
                    b.name, b.turn_index
                ),
                Err(e) => format!("\x1b[1;31mFailed to pin turn:\x1b[0m {}", e),
            }
        }
        "turn" => {
            if args.len() < 3 {
                return "\x1b[1;31mUsage:\x1b[0m /bookmark turn <turn_number> <name> [note]".to_string();
            }
            let turn_idx = match args[1].parse::<usize>() {
                Ok(n) if n > 0 => n,
                _ => return "\x1b[1;31mInvalid turn number.\x1b[0m Must be a positive integer.".to_string(),
            };
            let name = &args[2];
            let note = if args.len() > 3 {
                Some(args[3..].join(" "))
            } else {
                None
            };

            match bookmark_specific_turn(session, turn_idx, name, note.as_deref(), BookmarkKind::Turn) {
                Ok(b) => format!(
                    "\x1b[1;32mBookmark created:\x1b[0m 🔖 \x1b[1m{}\x1b[0m on Turn {}",
                    b.name, b.turn_index
                ),
                Err(e) => format!("\x1b[1;31mFailed to create bookmark:\x1b[0m {}", e),
            }
        }
        "recall" | "show" | "view" | "info" => {
            if args.len() < 2 {
                return "\x1b[1;31mUsage:\x1b[0m /bookmark recall <name>".to_string();
            }
            let name = &args[1];
            match recall_bookmark(session, name) {
                Ok(recall) => format_bookmark_detail(&recall),
                Err(e) => format!("\x1b[1;31mRecall failed:\x1b[0m {}", e),
            }
        }
        "restore" | "jump" | "rewind" => {
            if args.len() < 2 {
                return "\x1b[1;31mUsage:\x1b[0m /bookmark restore <name>".to_string();
            }
            let name = &args[1];
            match restore_to_bookmark(session, name) {
                Ok(reverted) => format!(
                    "\x1b[1;32mSession restored to bookmark:\x1b[0m \x1b[1m{}\x1b[0m (reverted {} turn(s))",
                    name, reverted
                ),
                Err(e) => format!("\x1b[1;31mFailed to restore session:\x1b[0m {}", e),
            }
        }
        "fork" | "branch" => {
            if args.len() < 2 {
                return "\x1b[1;31mUsage:\x1b[0m /bookmark fork <name> [new_title]".to_string();
            }
            let name = &args[1];
            let title = if args.len() > 2 {
                Some(args[2..].join(" "))
            } else {
                None
            };
            match fork_from_bookmark(session, name, title.as_deref()) {
                Ok(forked) => {
                    let _ = forked.save();
                    format!(
                        "\x1b[1;32mForked new branch session:\x1b[0m \x1b[1m{}\x1b[0m (ID: `{}`)",
                        forked.title().unwrap_or("Untitled Branch"),
                        forked.id()
                    )
                }
                Err(e) => format!("\x1b[1;31mFailed to fork from bookmark:\x1b[0m {}", e),
            }
        }
        "tag" => {
            if args.len() < 3 {
                return "\x1b[1;31mUsage:\x1b[0m /bookmark tag <name> <tag>".to_string();
            }
            let name = &args[1];
            let tag = &args[2];
            match tag_bookmark(session, name, tag) {
                Ok(_) => format!("\x1b[1;32mTagged bookmark:\x1b[0m \x1b[1m{}\x1b[0m with \x1b[1;33m[{}]\x1b[0m", name, tag),
                Err(e) => format!("\x1b[1;31mTagging failed:\x1b[0m {}", e),
            }
        }
        "untag" => {
            if args.len() < 3 {
                return "\x1b[1;31mUsage:\x1b[0m /bookmark untag <name> <tag>".to_string();
            }
            let name = &args[1];
            let tag = &args[2];
            match untag_bookmark(session, name, tag) {
                Ok(_) => format!("\x1b[1;32mRemoved tag:\x1b[0m \x1b[1;33m[{}]\x1b[0m from \x1b[1m{}\x1b[0m", tag, name),
                Err(e) => format!("\x1b[1;31mUntagging failed:\x1b[0m {}", e),
            }
        }
        "delete" | "del" | "rm" | "remove" => {
            if args.len() < 2 {
                return "\x1b[1;31mUsage:\x1b[0m /bookmark delete <name>".to_string();
            }
            let name = &args[1];
            match delete_bookmark(session, name) {
                Some(b) => format!("\x1b[1;32mDeleted bookmark:\x1b[0m \x1b[1m{}\x1b[0m", b.name),
                None => format!("\x1b[1;31mBookmark '{}' not found.\x1b[0m", name),
            }
        }
        "export" => {
            let path = if args.len() > 1 {
                Some(args[1].clone())
            } else {
                None
            };
            match path {
                Some(p) => {
                    let content = export_bookmarks_markdown(session);
                    if let Err(e) = fs::write(&p, content) {
                        format!("\x1b[1;31mFailed to export bookmarks:\x1b[0m {}", e)
                    } else {
                        format!("\x1b[1;32mBookmarks exported to:\x1b[0m {}", p)
                    }
                }
                None => export_bookmarks_markdown(session),
            }
        }
        // Default: `/bookmark <name> [note]` -> bookmarks current turn!
        name => {
            let note = if args.len() > 1 {
                Some(args[1..].join(" "))
            } else {
                None
            };
            match bookmark_turn(session, name, note.as_deref(), BookmarkKind::Turn) {
                Ok(b) => format!(
                    "\x1b[1;32mTurn bookmarked:\x1b[0m 🔖 \x1b[1m{}\x1b[0m (Turn {})",
                    b.name, b.turn_index
                ),
                Err(e) => format!("\x1b[1;31mFailed to bookmark turn:\x1b[0m {}", e),
            }
        }
    }
}

/// Formats the help manual for the `/bookmark` command.
fn format_bookmark_help() -> String {
    "\x1b[1;36m/bookmark - Session Turn Bookmarks & Checkpoints\x1b[0m\n\n\
     \x1b[1mUsage:\x1b[0m\n\
     • \x1b[1;36m/bookmark <name> [note]\x1b[0m           Pin current turn with a name and optional note\n\
     • \x1b[1;36m/bookmark list\x1b[0m                    List all bookmarks in this session\n\
     • \x1b[1;36m/bookmark recall <name>\x1b[0m           View bookmark details, turn preview, and drift\n\
     • \x1b[1;36m/bookmark checkpoint <name>\x1b[0m       Save a full restorable state snapshot checkpoint\n\
     • \x1b[1;36m/bookmark restore <name>\x1b[0m          Rewind / restore session back to this bookmark\n\
     • \x1b[1;36m/bookmark fork <name> [title]\x1b[0m     Fork session at bookmark into an independent branch\n\
     • \x1b[1;36m/bookmark pin [turn]\x1b[0m              Pin turn to protect it from compaction\n\
     • \x1b[1;36m/bookmark turn <turn> <name>\x1b[0m      Bookmark a specific historical turn\n\
     • \x1b[1;36m/bookmark tag <name> <tag>\x1b[0m        Add category tag to bookmark\n\
     • \x1b[1;36m/bookmark untag <name> <tag>\x1b[0m      Remove tag from bookmark\n\
     • \x1b[1;36m/bookmark delete <name>\x1b[0m         Delete bookmark from session\n\
     • \x1b[1;36m/bookmark clear\x1b[0m                 Clear all bookmarks from session\n\
     • \x1b[1;36m/bookmark export [path]\x1b[0m         Export bookmarks to Markdown file"
        .to_string()
}

/// Helper function to truncate strings with ellipsis.
fn truncate_string(s: &str, max_chars: usize) -> String {
    let trimmed = s.trim();
    let char_count = trimmed.chars().count();
    if char_count <= max_chars {
        trimmed.to_string()
    } else {
        let mut result = String::new();
        for (i, c) in trimmed.chars().enumerate() {
            if i >= max_chars.saturating_sub(3) {
                break;
            }
            result.push(c);
        }
        result.push_str("...");
        result
    }
}

// ============================================================================
// Codebase Bookmark & Annotation Manager
// ============================================================================

/// Categorization kind of a codebase bookmark or code annotation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CodeBookmarkKind {
    /// Standard reference bookmark to an important code location.
    Bookmark,
    /// Pending task, TODO, or work item in the codebase.
    Todo,
    /// Known bug, defect, or broken logic annotation.
    Bug,
    /// Code requiring refactoring or architecture cleanup.
    Refactor,
    /// Security audit note, potential vulnerability, or sanitization check.
    Security,
    /// Question, investigation point, or item needing clarification.
    Question,
    /// General developer note, comment, or architectural memo.
    Note,
    /// Performance optimization target or hotspot.
    Performance,
}

impl CodeBookmarkKind {
    /// Short string representation of the codebase bookmark kind.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Bookmark => "bookmark",
            Self::Todo => "todo",
            Self::Bug => "bug",
            Self::Refactor => "refactor",
            Self::Security => "security",
            Self::Question => "question",
            Self::Note => "note",
            Self::Performance => "performance",
        }
    }

    /// Human-friendly display label with emoji.
    pub fn display_label(&self) -> &'static str {
        match self {
            Self::Bookmark => "🔖 Bookmark",
            Self::Todo => "📝 TODO",
            Self::Bug => "🐛 Bug",
            Self::Refactor => "🔨 Refactor",
            Self::Security => "🔒 Security",
            Self::Question => "❓ Question",
            Self::Note => "📌 Note",
            Self::Performance => "⚡ Performance",
        }
    }

    /// Returns the emoji symbol associated with this kind.
    pub fn emoji(&self) -> &'static str {
        match self {
            Self::Bookmark => "🔖",
            Self::Todo => "📝",
            Self::Bug => "🐛",
            Self::Refactor => "🔨",
            Self::Security => "🔒",
            Self::Question => "❓",
            Self::Note => "📌",
            Self::Performance => "⚡",
        }
    }

    /// Parses kind loosely from a user string.
    pub fn from_str_loose(s: &str) -> Option<Self> {
        match s.trim().to_lowercase().as_str() {
            "bookmark" | "mark" | "bm" => Some(Self::Bookmark),
            "todo" | "task" => Some(Self::Todo),
            "bug" | "fixme" | "issue" | "defect" => Some(Self::Bug),
            "refactor" | "cleanup" | "rewrite" => Some(Self::Refactor),
            "security" | "sec" | "vuln" | "audit" => Some(Self::Security),
            "question" | "ask" | "help" | "clarify" => Some(Self::Question),
            "note" | "memo" | "comment" => Some(Self::Note),
            "perf" | "performance" | "opt" | "speed" => Some(Self::Performance),
            _ => None,
        }
    }
}

/// Represents a codebase bookmark or code annotation pointing to a specific file and line range.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CodeBookmark {
    /// Unique identifier for this code bookmark (UUID v4).
    pub id: Uuid,
    /// Path to the bookmarked file (relative or absolute).
    pub file_path: String,
    /// 1-based start line number.
    pub line_start: usize,
    /// Optional 1-based end line number for multi-line selections/ranges.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub line_end: Option<usize>,
    /// Optional code snippet or extract.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub snippet: Option<String>,
    /// Categorization kind of this bookmark (Bookmark, Todo, Bug, Refactor, Security, etc.).
    pub kind: CodeBookmarkKind,
    /// Optional title or short description.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    /// User notes, explanations, or analysis attached to this code location.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    /// Categorization tags (e.g. `["auth", "jwt", "urgent"]`).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
    /// Optional author or agent name who created this annotation.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub author: Option<String>,
    /// RFC 3339 timestamp when the bookmark was created.
    pub created_at: String,
    /// RFC 3339 timestamp when the bookmark was last modified.
    pub updated_at: String,
    /// Arbitrary user or tool metadata.
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub metadata: HashMap<String, String>,
}

impl CodeBookmark {
    /// Creates a new `CodeBookmark` pointing to a single line or start line.
    pub fn new(file_path: impl Into<String>, line_start: usize) -> Self {
        let now = Utc::now().to_rfc3339();
        Self {
            id: Uuid::new_v4(),
            file_path: file_path.into().trim().to_string(),
            line_start: if line_start == 0 { 1 } else { line_start },
            line_end: None,
            snippet: None,
            kind: CodeBookmarkKind::Bookmark,
            title: None,
            note: None,
            tags: Vec::new(),
            author: None,
            created_at: now.clone(),
            updated_at: now,
            metadata: HashMap::new(),
        }
    }

    /// Creates a new `CodeBookmark` with a line range.
    pub fn with_range(file_path: impl Into<String>, line_start: usize, line_end: usize) -> Self {
        let start = if line_start == 0 { 1 } else { line_start };
        let end = line_end.max(start);
        Self::new(file_path, start).with_line_end(end)
    }

    /// Sets the end line for a multi-line range.
    pub fn with_line_end(mut self, line_end: usize) -> Self {
        if line_end >= self.line_start {
            self.line_end = Some(line_end);
        } else {
            self.line_end = Some(self.line_start);
        }
        self
    }

    /// Sets the code snippet.
    pub fn with_snippet(mut self, snippet: impl Into<String>) -> Self {
        let s = snippet.into();
        self.snippet = if s.trim().is_empty() { None } else { Some(s) };
        self
    }

    /// Sets the bookmark kind.
    pub fn with_kind(mut self, kind: CodeBookmarkKind) -> Self {
        self.kind = kind;
        self
    }

    /// Sets an optional title.
    pub fn with_title(mut self, title: impl Into<String>) -> Self {
        let t = title.into().trim().to_string();
        self.title = if t.is_empty() { None } else { Some(t) };
        self
    }

    /// Sets the user note / annotation.
    pub fn with_note(mut self, note: impl Into<String>) -> Self {
        let n = note.into().trim().to_string();
        self.note = if n.is_empty() { None } else { Some(n) };
        self
    }

    /// Adds a single tag (normalized to lowercase).
    pub fn with_tag(mut self, tag: impl Into<String>) -> Self {
        let t = tag.into().trim().to_lowercase();
        if !t.is_empty() && !self.tags.contains(&t) {
            self.tags.push(t);
        }
        self
    }

    /// Sets multiple tags.
    pub fn with_tags(mut self, tags: impl IntoIterator<Item = impl Into<String>>) -> Self {
        for t in tags {
            let clean = t.into().trim().to_lowercase();
            if !clean.is_empty() && !self.tags.contains(&clean) {
                self.tags.push(clean);
            }
        }
        self
    }

    /// Sets author name.
    pub fn with_author(mut self, author: impl Into<String>) -> Self {
        let a = author.into().trim().to_string();
        self.author = if a.is_empty() { None } else { Some(a) };
        self
    }

    /// Sets metadata key-value.
    pub fn with_metadata(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.metadata.insert(key.into(), value.into());
        self
    }

    /// Formats the line range as a human-readable string, e.g. "L42" or "L42-L55".
    pub fn line_display(&self) -> String {
        match self.line_end {
            Some(end) if end > self.line_start => format!("L{}-L{}", self.line_start, end),
            _ => format!("L{}", self.line_start),
        }
    }

    /// Formats the location as "path:L42" or "path:L42-L55".
    pub fn location_display(&self) -> String {
        format!("{}:{}", self.file_path, self.line_display())
    }

    /// Checks if a given line is included in this bookmark's range.
    pub fn contains_line(&self, line: usize) -> bool {
        let end = self.line_end.unwrap_or(self.line_start);
        line >= self.line_start && line <= end
    }

    /// Checks if this bookmark has a specific tag (case-insensitive).
    pub fn has_tag(&self, tag: &str) -> bool {
        let clean = tag.trim().to_lowercase();
        self.tags.iter().any(|t| t == &clean)
    }

    /// Checks if this bookmark matches a keyword query (in path, title, note, snippet, or tags).
    pub fn matches_keyword(&self, query: &str) -> bool {
        let q = query.trim().to_lowercase();
        if q.is_empty() {
            return true;
        }

        if self.file_path.to_lowercase().contains(&q) {
            return true;
        }
        if let Some(t) = &self.title {
            if t.to_lowercase().contains(&q) {
                return true;
            }
        }
        if let Some(n) = &self.note {
            if n.to_lowercase().contains(&q) {
                return true;
            }
        }
        if let Some(s) = &self.snippet {
            if s.to_lowercase().contains(&q) {
                return true;
            }
        }
        if self.tags.iter().any(|t| t.to_lowercase().contains(&q)) {
            return true;
        }
        if self.kind.as_str().contains(&q) {
            return true;
        }
        false
    }

    /// Exports this single bookmark to Markdown format.
    pub fn to_markdown(&self) -> String {
        let mut md = String::new();
        md.push_str(&format!("### {} `{}`\n\n", self.kind.emoji(), self.location_display()));

        if let Some(t) = &self.title {
            md.push_str(&format!("**Title:** {}\n\n", t));
        }

        md.push_str(&format!("- **Location:** `{}` ({})\n", self.file_path, self.line_display()));
        md.push_str(&format!("- **Kind:** {}\n", self.kind.display_label()));

        if !self.tags.is_empty() {
            let tags_str = self
                .tags
                .iter()
                .map(|t| format!("`#{}`", t))
                .collect::<Vec<_>>()
                .join(" ");
            md.push_str(&format!("- **Tags:** {}\n", tags_str));
        }

        if let Some(author) = &self.author {
            md.push_str(&format!("- **Author:** {}\n", author));
        }
        md.push_str(&format!("- **Created:** {}\n", self.created_at));

        if let Some(note) = &self.note {
            md.push_str(&format!("\n**Note:**\n> {}\n", note.replace('\n', "\n> ")));
        }

        if let Some(snippet) = &self.snippet {
            let lang = detect_language_from_path(&self.file_path);
            md.push_str(&format!("\n```{}\n{}\n```\n", lang, snippet.trim_end()));
        }

        md.push('\n');
        md
    }
}

/// Query filter criteria for searching and filtering codebase bookmarks.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CodeBookmarkFilter {
    /// Filter by specific tag (case-insensitive).
    pub tag: Option<String>,
    /// Filter by matching any of these tags.
    pub any_tags: Vec<String>,
    /// Filter by path prefix (e.g. `src/agent` or `tests/`).
    pub path_prefix: Option<String>,
    /// Exact file path match.
    pub file_path: Option<String>,
    /// Keyword search matching path, note, title, snippet, or tags.
    pub keyword: Option<String>,
    /// Filter by specific bookmark kind.
    pub kind: Option<CodeBookmarkKind>,
    /// Filter by minimum line number.
    pub min_line: Option<usize>,
    /// Filter by maximum line number.
    pub max_line: Option<usize>,
    /// Filter by author.
    pub author: Option<String>,
}

impl CodeBookmarkFilter {
    /// Creates an empty filter matching all bookmarks.
    pub fn new() -> Self {
        Self::default()
    }

    /// Sets tag filter.
    pub fn with_tag(mut self, tag: impl Into<String>) -> Self {
        self.tag = Some(tag.into());
        self
    }

    /// Sets path prefix filter.
    pub fn with_path_prefix(mut self, prefix: impl Into<String>) -> Self {
        self.path_prefix = Some(prefix.into());
        self
    }

    /// Sets exact file path filter.
    pub fn with_file_path(mut self, file_path: impl Into<String>) -> Self {
        self.file_path = Some(file_path.into());
        self
    }

    /// Sets keyword filter.
    pub fn with_keyword(mut self, keyword: impl Into<String>) -> Self {
        self.keyword = Some(keyword.into());
        self
    }

    /// Sets kind filter.
    pub fn with_kind(mut self, kind: CodeBookmarkKind) -> Self {
        self.kind = Some(kind);
        self
    }

    /// Sets line range bounds.
    pub fn with_line_range(mut self, min: usize, max: usize) -> Self {
        self.min_line = Some(min);
        self.max_line = Some(max);
        self
    }

    /// Sets author filter.
    pub fn with_author(mut self, author: impl Into<String>) -> Self {
        self.author = Some(author.into());
        self
    }

    /// Evaluates whether a bookmark satisfies all active filter criteria.
    pub fn matches(&self, bookmark: &CodeBookmark) -> bool {
        if let Some(tag) = &self.tag {
            if !bookmark.has_tag(tag) {
                return false;
            }
        }

        if !self.any_tags.is_empty() {
            let matched_any = self.any_tags.iter().any(|t| bookmark.has_tag(t));
            if !matched_any {
                return false;
            }
        }

        if let Some(prefix) = &self.path_prefix {
            let clean_prefix = prefix.trim();
            if !bookmark.file_path.starts_with(clean_prefix)
                && !Path::new(&bookmark.file_path).starts_with(clean_prefix)
            {
                return false;
            }
        }

        if let Some(file) = &self.file_path {
            if !bookmark.file_path.eq_ignore_ascii_case(file.trim()) {
                return false;
            }
        }

        if let Some(kw) = &self.keyword {
            if !bookmark.matches_keyword(kw) {
                return false;
            }
        }

        if let Some(kind) = self.kind {
            if bookmark.kind != kind {
                return false;
            }
        }

        if let Some(min) = self.min_line {
            let end = bookmark.line_end.unwrap_or(bookmark.line_start);
            if end < min {
                return false;
            }
        }

        if let Some(max) = self.max_line {
            if bookmark.line_start > max {
                return false;
            }
        }

        if let Some(author) = &self.author {
            match &bookmark.author {
                Some(a) => {
                    if !a.eq_ignore_ascii_case(author.trim()) {
                        return false;
                    }
                }
                None => return false,
            }
        }

        true
    }
}

/// Persistent and in-memory store for managing codebase bookmarks and annotations.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CodeBookmarkStore {
    /// In-memory list of codebase bookmarks.
    bookmarks: Vec<CodeBookmark>,
}

impl CodeBookmarkStore {
    /// Creates a new empty `CodeBookmarkStore`.
    pub fn new() -> Self {
        Self {
            bookmarks: Vec::new(),
        }
    }

    /// Adds a bookmark to the store. Returns its ID.
    pub fn add(&mut self, bookmark: CodeBookmark) -> Uuid {
        let id = bookmark.id;
        if let Some(pos) = self.bookmarks.iter().position(|b| b.id == id) {
            self.bookmarks[pos] = bookmark;
        } else {
            self.bookmarks.push(bookmark);
        }
        id
    }

    /// Helper to create and add a new bookmark in one call.
    pub fn create(
        &mut self,
        file_path: impl Into<String>,
        line_start: usize,
        line_end: Option<usize>,
        note: Option<String>,
    ) -> &CodeBookmark {
        let mut b = CodeBookmark::new(file_path, line_start);
        if let Some(end) = line_end {
            b = b.with_line_end(end);
        }
        if let Some(n) = note {
            b = b.with_note(n);
        }
        let id = b.id;
        self.bookmarks.push(b);
        self.get(&id).unwrap_or_else(|| &self.bookmarks[self.bookmarks.len() - 1])
    }

    /// Retrieves a bookmark by its unique ID.
    pub fn get(&self, id: &Uuid) -> Option<&CodeBookmark> {
        self.bookmarks.iter().find(|b| &b.id == id)
    }

    /// Retrieves a mutable reference to a bookmark by its unique ID.
    pub fn get_mut(&mut self, id: &Uuid) -> Option<&mut CodeBookmark> {
        self.bookmarks.iter_mut().find(|b| &b.id == id)
    }

    /// Finds a bookmark by ID prefix or exact string representation.
    pub fn find_by_id_str(&self, id_str: &str) -> Option<&CodeBookmark> {
        let clean = id_str.trim();
        if let Ok(uuid) = Uuid::parse_str(clean) {
            return self.get(&uuid);
        }
        if clean.len() >= 4 {
            let lower = clean.to_lowercase();
            return self
                .bookmarks
                .iter()
                .find(|b| b.id.to_string().to_lowercase().starts_with(&lower));
        }
        None
    }

    /// Removes a bookmark by its ID.
    pub fn remove(&mut self, id: &Uuid) -> Option<CodeBookmark> {
        if let Some(pos) = self.bookmarks.iter().position(|b| &b.id == id) {
            Some(self.bookmarks.remove(pos))
        } else {
            None
        }
    }

    /// Removes a bookmark by ID string or prefix.
    pub fn remove_by_id_str(&mut self, id_str: &str) -> Option<CodeBookmark> {
        let clean = id_str.trim();
        let target_id = if let Ok(uuid) = Uuid::parse_str(clean) {
            Some(uuid)
        } else if clean.len() >= 4 {
            let lower = clean.to_lowercase();
            self.bookmarks
                .iter()
                .find(|b| b.id.to_string().to_lowercase().starts_with(&lower))
                .map(|b| b.id)
        } else {
            None
        }?;

        self.remove(&target_id)
    }

    /// Adds a tag to a bookmark by ID.
    pub fn add_tag(&mut self, id: &Uuid, tag: &str) -> bool {
        if let Some(b) = self.get_mut(id) {
            let clean = tag.trim().to_lowercase();
            if !clean.is_empty() && !b.tags.contains(&clean) {
                b.tags.push(clean);
                b.updated_at = Utc::now().to_rfc3339();
                return true;
            }
        }
        false
    }

    /// Removes a tag from a bookmark by ID.
    pub fn remove_tag(&mut self, id: &Uuid, tag: &str) -> bool {
        if let Some(b) = self.get_mut(id) {
            let clean = tag.trim().to_lowercase();
            let orig = b.tags.len();
            b.tags.retain(|t| t != &clean);
            if b.tags.len() != orig {
                b.updated_at = Utc::now().to_rfc3339();
                return true;
            }
        }
        false
    }

    /// Updates the note of a bookmark by ID.
    pub fn update_note(&mut self, id: &Uuid, note: Option<String>) -> bool {
        if let Some(b) = self.get_mut(id) {
            b.note = note.map(|n| n.trim().to_string()).filter(|n| !n.is_empty());
            b.updated_at = Utc::now().to_rfc3339();
            true
        } else {
            false
        }
    }

    /// Updates the snippet of a bookmark by ID.
    pub fn update_snippet(&mut self, id: &Uuid, snippet: Option<String>) -> bool {
        if let Some(b) = self.get_mut(id) {
            b.snippet = snippet.filter(|s| !s.trim().is_empty());
            b.updated_at = Utc::now().to_rfc3339();
            true
        } else {
            false
        }
    }

    /// Updates the line range of a bookmark by ID.
    pub fn update_lines(&mut self, id: &Uuid, line_start: usize, line_end: Option<usize>) -> bool {
        if let Some(b) = self.get_mut(id) {
            b.line_start = if line_start == 0 { 1 } else { line_start };
            b.line_end = line_end.map(|e| e.max(b.line_start));
            b.updated_at = Utc::now().to_rfc3339();
            true
        } else {
            false
        }
    }

    /// Returns the count of bookmarks.
    pub fn len(&self) -> usize {
        self.bookmarks.len()
    }

    /// Returns true if the store has no bookmarks.
    pub fn is_empty(&self) -> bool {
        self.bookmarks.is_empty()
    }

    /// Clears all bookmarks from the store. Returns the number of removed bookmarks.
    pub fn clear(&mut self) -> usize {
        let count = self.bookmarks.len();
        self.bookmarks.clear();
        count
    }

    /// Returns all bookmarks as a slice.
    pub fn list(&self) -> &[CodeBookmark] {
        &self.bookmarks
    }

    /// Returns a vector of references to all bookmarks.
    pub fn all(&self) -> Vec<&CodeBookmark> {
        self.bookmarks.iter().collect()
    }

    /// Searches bookmarks using keyword matching against file paths, notes, snippets, and tags.
    pub fn search(&self, keyword: &str) -> Vec<&CodeBookmark> {
        let q = keyword.trim();
        if q.is_empty() {
            return self.all();
        }
        self.bookmarks.iter().filter(|b| b.matches_keyword(q)).collect()
    }

    /// Filters bookmarks based on a `CodeBookmarkFilter`.
    pub fn filter(&self, filter: &CodeBookmarkFilter) -> Vec<&CodeBookmark> {
        self.bookmarks.iter().filter(|b| filter.matches(b)).collect()
    }

    /// Finds all bookmarks for a specific file path.
    pub fn find_by_file(&self, path: &str) -> Vec<&CodeBookmark> {
        let clean = path.trim();
        self.bookmarks
            .iter()
            .filter(|b| b.file_path.eq_ignore_ascii_case(clean))
            .collect()
    }

    /// Finds all bookmarks under a directory path prefix.
    pub fn find_by_path_prefix(&self, prefix: &str) -> Vec<&CodeBookmark> {
        let clean = prefix.trim();
        self.bookmarks
            .iter()
            .filter(|b| b.file_path.starts_with(clean) || Path::new(&b.file_path).starts_with(clean))
            .collect()
    }

    /// Finds all bookmarks with a specific tag (case-insensitive).
    pub fn find_by_tag(&self, tag: &str) -> Vec<&CodeBookmark> {
        let clean = tag.trim().to_lowercase();
        self.bookmarks.iter().filter(|b| b.has_tag(&clean)).collect()
    }

    /// Finds all bookmarks of a specific kind.
    pub fn find_by_kind(&self, kind: CodeBookmarkKind) -> Vec<&CodeBookmark> {
        self.bookmarks.iter().filter(|b| b.kind == kind).collect()
    }

    /// Exports all bookmarks to a Markdown document summary.
    pub fn export_markdown(&self, title: Option<&str>) -> String {
        let doc_title = title.unwrap_or("Codebase Bookmarks & Annotations");
        let mut md = String::new();
        md.push_str(&format!("# {}\n\n", doc_title));

        if self.bookmarks.is_empty() {
            md.push_str("_No codebase bookmarks recorded._\n");
            return md;
        }

        // Summary Statistics
        let total_count = self.bookmarks.len();
        let mut kind_counts: HashMap<CodeBookmarkKind, usize> = HashMap::new();
        let mut file_map: HashMap<String, Vec<&CodeBookmark>> = HashMap::new();

        for b in &self.bookmarks {
            *kind_counts.entry(b.kind).or_default() += 1;
            file_map.entry(b.file_path.clone()).or_default().push(b);
        }

        md.push_str("## 📊 Summary\n\n");
        md.push_str(&format!("- **Total Bookmarks:** {}\n", total_count));
        md.push_str(&format!("- **Files Referenced:** {}\n", file_map.len()));
        md.push_str("- **Breakdown:** ");

        let mut breakdown_parts = Vec::new();
        let mut kinds_sorted: Vec<CodeBookmarkKind> = kind_counts.keys().cloned().collect();
        kinds_sorted.sort_by_key(|k| k.as_str());

        for kind in kinds_sorted {
            if let Some(count) = kind_counts.get(&kind) {
                breakdown_parts.push(format!("{} {} ({})", kind.emoji(), kind.as_str(), count));
            }
        }
        md.push_str(&breakdown_parts.join(", "));
        md.push_str("\n\n---\n\n");

        // Overview Table
        md.push_str("## 📑 Index Table\n\n");
        md.push_str("| Location | Type | Tags | Note |\n");
        md.push_str("| --- | --- | --- | --- |\n");

        let mut sorted_all: Vec<&CodeBookmark> = self.bookmarks.iter().collect();
        sorted_all.sort_by(|a, b| {
            a.file_path
                .cmp(&b.file_path)
                .then(a.line_start.cmp(&b.line_start))
        });

        for bm in sorted_all {
            let note = bm
                .note
                .as_deref()
                .map(|n| truncate_string(n, 40))
                .unwrap_or_else(|| "-".to_string())
                .replace('|', "\\|");
            let tags = if bm.tags.is_empty() {
                "-".to_string()
            } else {
                bm.tags.iter().map(|t| format!("`#{}`", t)).collect::<Vec<_>>().join(" ")
            };
            md.push_str(&format!(
                "| `{}` | {} | {} | {} |\n",
                bm.location_display(),
                bm.kind.display_label(),
                tags,
                note
            ));
        }

        md.push_str("\n---\n\n");

        // Grouped by File
        md.push_str("## 📁 Bookmarks by File\n\n");

        let mut sorted_files: Vec<String> = file_map.keys().cloned().collect();
        sorted_files.sort();

        for file in sorted_files {
            if let Some(mut file_bms) = file_map.remove(&file) {
                file_bms.sort_by_key(|b| b.line_start);
                md.push_str(&format!("### 📄 `{}`\n\n", file));

                for bm in file_bms {
                    let header_title = bm
                        .title
                        .as_deref()
                        .unwrap_or_else(|| bm.kind.as_str());

                    md.push_str(&format!(
                        "#### {} `{}` - {}\n\n",
                        bm.kind.emoji(),
                        bm.line_display(),
                        header_title
                    ));

                    if !bm.tags.is_empty() {
                        let tags_str = bm
                            .tags
                            .iter()
                            .map(|t| format!("`#{}`", t))
                            .collect::<Vec<_>>()
                            .join(" ");
                        md.push_str(&format!("- **Tags:** {}\n", tags_str));
                    }

                    if let Some(author) = &bm.author {
                        md.push_str(&format!("- **Author:** {}\n", author));
                    }

                    if let Some(note) = &bm.note {
                        md.push_str(&format!("\n**Note:**\n> {}\n\n", note.replace('\n', "\n> ")));
                    }

                    if let Some(snippet) = &bm.snippet {
                        let lang = detect_language_from_path(&bm.file_path);
                        md.push_str(&format!("```{}\n{}\n```\n\n", lang, snippet.trim_end()));
                    }
                }
                md.push_str("---\n\n");
            }
        }

        md
    }

    /// Exports bookmarks to formatted JSON string.
    pub fn export_json(&self) -> anyhow::Result<String> {
        Ok(serde_json::to_string_pretty(&self.bookmarks)?)
    }

    /// Imports bookmarks from JSON string, merging with existing entries.
    pub fn import_json(&mut self, json_str: &str) -> anyhow::Result<usize> {
        let imported: Vec<CodeBookmark> = serde_json::from_str(json_str)?;
        let count = imported.len();
        for b in imported {
            self.add(b);
        }
        Ok(count)
    }

    /// Saves the store to a JSON file on disk.
    pub fn save_to_disk(&self, path: impl AsRef<Path>) -> anyhow::Result<()> {
        let path = path.as_ref();
        if let Some(parent) = path.parent() {
            if !parent.exists() {
                fs::create_dir_all(parent)?;
            }
        }
        let json = self.export_json()?;
        fs::write(path, json)?;
        Ok(())
    }

    /// Loads the store from a JSON file on disk.
    pub fn load_from_disk(path: impl AsRef<Path>) -> anyhow::Result<Self> {
        let path = path.as_ref();
        if !path.exists() {
            return Ok(Self::new());
        }
        let content = fs::read_to_string(path)?;
        let bookmarks: Vec<CodeBookmark> = serde_json::from_str(&content)?;
        Ok(Self { bookmarks })
    }

    /// Returns the default path for persistent codebase bookmarks: `~/.fusion/code_bookmarks.json`.
    pub fn default_disk_path() -> PathBuf {
        Config::config_dir().join("code_bookmarks.json")
    }

    /// Saves to the default persistent disk location `~/.fusion/code_bookmarks.json`.
    pub fn save_to_default_disk(&self) -> anyhow::Result<PathBuf> {
        let path = Self::default_disk_path();
        self.save_to_disk(&path)?;
        Ok(path)
    }

    /// Loads from the default persistent disk location `~/.fusion/code_bookmarks.json`.
    pub fn load_from_default_disk() -> anyhow::Result<Self> {
        Self::load_from_disk(Self::default_disk_path())
    }
}

// ============================================================================
// Codebase Bookmark Session Metadata Integration
// ============================================================================

/// Reads codebase bookmarks from session metadata.
pub fn list_code_bookmarks(session: &Session) -> Vec<CodeBookmark> {
    if let Some(json_str) = session.get_metadata(CODE_BOOKMARKS_METADATA_KEY) {
        if let Ok(bookmarks) = serde_json::from_str::<Vec<CodeBookmark>>(json_str) {
            return bookmarks;
        }
    }
    Vec::new()
}

/// Saves codebase bookmarks into session metadata.
pub fn save_code_bookmarks_to_session(
    session: &mut Session,
    bookmarks: &[CodeBookmark],
) -> anyhow::Result<()> {
    let json_str = serde_json::to_string(bookmarks)?;
    session.set_metadata(CODE_BOOKMARKS_METADATA_KEY, json_str);
    Ok(())
}

/// Loads a `CodeBookmarkStore` populated with bookmarks from session metadata.
pub fn load_code_bookmark_store(session: &Session) -> CodeBookmarkStore {
    let bookmarks = list_code_bookmarks(session);
    CodeBookmarkStore { bookmarks }
}

/// Saves a `CodeBookmarkStore` into session metadata.
pub fn save_code_bookmark_store(
    session: &mut Session,
    store: &CodeBookmarkStore,
) -> anyhow::Result<()> {
    save_code_bookmarks_to_session(session, store.list())
}

/// Formats and exports codebase bookmarks in session metadata to Markdown.
pub fn export_code_bookmarks_markdown(session: &Session, title: Option<&str>) -> String {
    let store = load_code_bookmark_store(session);
    store.export_markdown(title)
}

// ============================================================================
// Snippet and Syntax Utilities
// ============================================================================

/// Extracts code snippet lines from source text for given 1-based start and end lines.
pub fn extract_snippet_from_content(
    content: &str,
    line_start: usize,
    line_end: Option<usize>,
) -> Option<String> {
    if line_start == 0 {
        return None;
    }
    let lines: Vec<&str> = content.lines().collect();
    if line_start > lines.len() {
        return None;
    }
    let start_idx = line_start - 1;
    let end_line = line_end.unwrap_or(line_start).max(line_start);
    let end_idx = end_line.min(lines.len());

    let slice = &lines[start_idx..end_idx];
    if slice.is_empty() {
        None
    } else {
        Some(slice.join("\n"))
    }
}

/// Reads file and extracts code snippet lines for given 1-based start and end lines.
pub fn extract_snippet_from_file(
    path: impl AsRef<Path>,
    line_start: usize,
    line_end: Option<usize>,
) -> std::io::Result<Option<String>> {
    let path = path.as_ref();
    if !path.exists() {
        return Ok(None);
    }
    let content = fs::read_to_string(path)?;
    Ok(extract_snippet_from_content(&content, line_start, line_end))
}

/// Detects markdown code block syntax language identifier from a file path extension.
pub fn detect_language_from_path(path: &str) -> &'static str {
    let p = Path::new(path);
    match p.extension().and_then(|ext| ext.to_str()).unwrap_or("").to_ascii_lowercase().as_str() {
        "rs" => "rust",
        "ts" => "typescript",
        "tsx" => "tsx",
        "js" | "mjs" | "cjs" => "javascript",
        "jsx" => "jsx",
        "py" | "pyi" => "python",
        "go" => "go",
        "c" | "h" => "c",
        "cpp" | "cc" | "cxx" | "hpp" | "hh" => "cpp",
        "java" => "java",
        "kt" | "kts" => "kotlin",
        "swift" => "swift",
        "rb" => "ruby",
        "php" => "php",
        "sh" | "bash" | "zsh" => "bash",
        "json" => "json",
        "toml" => "toml",
        "yaml" | "yml" => "yaml",
        "html" | "htm" => "html",
        "css" => "css",
        "scss" | "sass" => "scss",
        "md" | "markdown" => "markdown",
        "sql" => "sql",
        "proto" => "protobuf",
        "wasm" | "wat" => "wat",
        "lua" => "lua",
        "zig" => "zig",
        _ => "",
    }
}

// ============================================================================
// Unit Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn create_sample_session() -> Session {
        let mut session = Session::new("test-model");
        session.set_title("Compiler Optimization Discussion");

        // Turn 1
        session.add_user_message("What is LLVM SSA form?");
        session.add_assistant_message("SSA stands for Static Single Assignment form...");

        // Turn 2
        session.add_user_message("Can you show a Rust example of constant propagation?");
        session.add_assistant_message("Here is how constant propagation works in Rust: ...");

        // Turn 3
        session.add_user_message("Now explain dead code elimination.");
        session.add_assistant_message("Dead code elimination removes unreachable code blocks.");

        session
    }

    #[test]
    fn test_bookmark_turn_creation_and_listing() {
        let mut session = create_sample_session();
        assert_eq!(list_bookmarks(&session).len(), 0);

        // Bookmark current (latest = Turn 3)
        let b1 = bookmark_turn(&mut session, "dce-step", Some("Dead code elimination turn"), BookmarkKind::Turn)
            .expect("should bookmark turn");

        assert_eq!(b1.name, "dce-step");
        assert_eq!(b1.turn_index, 3);
        assert_eq!(b1.kind, BookmarkKind::Turn);
        assert_eq!(b1.note.as_deref(), Some("Dead code elimination turn"));

        let list = list_bookmarks(&session);
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].name, "dce-step");

        // Bookmark specific turn (Turn 1)
        let b2 = bookmark_specific_turn(
            &mut session,
            1,
            "intro-ssa",
            Some("LLVM SSA intro"),
            BookmarkKind::Turn,
        )
        .expect("should bookmark turn 1");

        assert_eq!(b2.name, "intro-ssa");
        assert_eq!(b2.turn_index, 1);

        let list2 = list_bookmarks(&session);
        assert_eq!(list2.len(), 2);
    }

    #[test]
    fn test_bookmark_retrieval_and_recall() {
        let mut session = create_sample_session();
        bookmark_specific_turn(&mut session, 2, "const-prop", Some("Rust constant folding"), BookmarkKind::Turn)
            .unwrap();

        // Retrieval by name (case-insensitive)
        let found = get_bookmark(&session, "CONST-PROP").expect("should find bookmark");
        assert_eq!(found.name, "const-prop");
        assert_eq!(found.turn_index, 2);

        // Recall analysis
        let recall = recall_bookmark(&session, "const-prop").expect("should recall bookmark");
        assert_eq!(recall.bookmark.name, "const-prop");
        assert_eq!(recall.turn_index, 2);
        assert_eq!(recall.current_total_turns, 3);
        assert_eq!(recall.turns_behind, 1); // 1 turn behind current head
        assert!(recall.preview.contains("const-prop"));
        assert!(recall.preview.contains("Rust constant folding"));
    }

    #[test]
    fn test_bookmark_checkpoint_and_restoration() {
        let mut session = create_sample_session();

        // Checkpoint at Turn 3
        let cp = bookmark_checkpoint(&mut session, "v1-complete", Some("Before refactoring"))
            .expect("should create checkpoint");
        assert_eq!(cp.kind, BookmarkKind::Checkpoint);
        assert!(cp.snapshot.is_some());

        // Add 2 more turns to session
        session.add_user_message("Turn 4 user message");
        session.add_assistant_message("Turn 4 assistant response");
        session.add_user_message("Turn 5 user message");
        session.add_assistant_message("Turn 5 assistant response");

        assert_eq!(count_turns(&session), 5);

        // Recall shows drift
        let recall = recall_bookmark(&session, "v1-complete").unwrap();
        assert_eq!(recall.turns_behind, 2);

        // Restore back to checkpoint
        let reverted = restore_to_bookmark(&mut session, "v1-complete").expect("restore should succeed");
        assert_eq!(reverted, 2);
        assert_eq!(count_turns(&session), 3);
    }

    #[test]
    fn test_pinned_turns_protection() {
        let mut session = create_sample_session();
        assert!(!is_turn_pinned(&session, 2));

        pin_turn(&mut session, 2, "important-decision").unwrap();
        assert!(is_turn_pinned(&session, 2));
        assert_eq!(get_pinned_turns(&session), vec![2]);

        unpin_turn(&mut session, 2);
        assert!(!is_turn_pinned(&session, 2));
    }

    #[test]
    fn test_bookmark_tagging_and_filtering() {
        let mut session = create_sample_session();
        bookmark_specific_turn(&mut session, 1, "bm1", None, BookmarkKind::Turn).unwrap();
        bookmark_specific_turn(&mut session, 2, "bm2", None, BookmarkKind::Checkpoint).unwrap();
        bookmark_specific_turn(&mut session, 3, "bm3", None, BookmarkKind::Note).unwrap();

        tag_bookmark(&mut session, "bm1", "compiler").unwrap();
        tag_bookmark(&mut session, "bm2", "compiler").unwrap();
        tag_bookmark(&mut session, "bm3", "docs").unwrap();

        let compiler_bms = filter_bookmarks_by_tag(&session, "compiler");
        assert_eq!(compiler_bms.len(), 2);

        let docs_bms = filter_bookmarks_by_tag(&session, "docs");
        assert_eq!(docs_bms.len(), 1);

        untag_bookmark(&mut session, "bm1", "compiler").unwrap();
        let compiler_bms_after = filter_bookmarks_by_tag(&session, "compiler");
        assert_eq!(compiler_bms_after.len(), 1);
    }

    #[test]
    fn test_bookmark_searching() {
        let mut session = create_sample_session();
        bookmark_specific_turn(&mut session, 1, "ssa-intro", Some("Core compiler architecture"), BookmarkKind::Turn).unwrap();
        bookmark_specific_turn(&mut session, 2, "prop-fold", Some("Optimization phase"), BookmarkKind::Turn).unwrap();

        let res = search_bookmarks(&session, "compiler");
        assert_eq!(res.len(), 1);
        assert_eq!(res[0].name, "ssa-intro");

        let res2 = search_bookmarks(&session, "prop");
        assert_eq!(res2.len(), 1);
        assert_eq!(res2[0].name, "prop-fold");
    }

    #[test]
    fn test_bookmark_forking() {
        let mut session = create_sample_session();
        bookmark_specific_turn(&mut session, 2, "fork-point", None, BookmarkKind::Turn).unwrap();

        let forked = fork_from_bookmark(&session, "fork-point", Some("Branch from Prop Fold")).unwrap();
        assert_ne!(forked.id(), session.id());
        assert_eq!(forked.title(), Some("Branch from Prop Fold"));
        assert_eq!(count_turns(&forked), 2);
    }

    #[test]
    fn test_bookmark_deletion_and_clearing() {
        let mut session = create_sample_session();
        bookmark_turn(&mut session, "to-delete", None, BookmarkKind::Turn).unwrap();
        assert_eq!(list_bookmarks(&session).len(), 1);

        let deleted = delete_bookmark(&mut session, "to-delete");
        assert!(deleted.is_some());
        assert_eq!(list_bookmarks(&session).len(), 0);

        bookmark_turn(&mut session, "bm-a", None, BookmarkKind::Turn).unwrap();
        bookmark_turn(&mut session, "bm-b", None, BookmarkKind::Turn).unwrap();
        assert_eq!(list_bookmarks(&session).len(), 2);

        let cleared = clear_bookmarks(&mut session);
        assert_eq!(cleared, 2);
        assert_eq!(list_bookmarks(&session).len(), 0);
    }

    #[test]
    fn test_bookmark_json_and_markdown_export() {
        let mut session = create_sample_session();
        bookmark_specific_turn(&mut session, 1, "step1", Some("Note on step 1"), BookmarkKind::Turn).unwrap();
        tag_bookmark(&mut session, "step1", "core").unwrap();

        // Markdown export
        let md = export_bookmarks_markdown(&session);
        assert!(md.contains("# Bookmarks for Session:"));
        assert!(md.contains("step1"));
        assert!(md.contains("Note on step 1"));
        assert!(md.contains("core"));

        // JSON export & import
        let json = export_bookmarks_json(&session).unwrap();
        assert!(json.contains("step1"));

        let mut new_session = Session::new("other-model");
        let imported = import_bookmarks_json(&mut new_session, &json).unwrap();
        assert_eq!(imported, 1);
        assert_eq!(list_bookmarks(&new_session).len(), 1);
        assert_eq!(list_bookmarks(&new_session)[0].name, "step1");
    }

    #[test]
    fn test_handle_bookmark_command_cli() {
        let mut session = create_sample_session();

        // 1. /bookmark my-mark (current turn)
        let out = handle_bookmark_command(&["my-mark".to_string()], &mut session);
        assert!(out.contains("Turn bookmarked"));
        assert!(out.contains("my-mark"));

        // 2. /bookmark list
        let out_list = handle_bookmark_command(&["list".to_string()], &mut session);
        assert!(out_list.contains("my-mark"));

        // 3. /bookmark recall my-mark
        let out_recall = handle_bookmark_command(&["recall".to_string(), "my-mark".to_string()], &mut session);
        assert!(out_recall.contains("Bookmark:"));
        assert!(out_recall.contains("my-mark"));

        // 4. /bookmark tag my-mark v1
        let out_tag = handle_bookmark_command(&["tag".to_string(), "my-mark".to_string(), "v1".to_string()], &mut session);
        assert!(out_tag.contains("Tagged bookmark"));

        // 5. /bookmark checkpoint cp1
        let out_cp = handle_bookmark_command(&["checkpoint".to_string(), "cp1".to_string()], &mut session);
        assert!(out_cp.contains("Checkpoint saved"));

        // 6. /bookmark restore cp1
        let out_restore = handle_bookmark_command(&["restore".to_string(), "cp1".to_string()], &mut session);
        assert!(out_restore.contains("Session restored"));

        // 7. /bookmark delete my-mark
        let out_del = handle_bookmark_command(&["delete".to_string(), "my-mark".to_string()], &mut session);
        assert!(out_del.contains("Deleted bookmark"));

        // 8. /bookmark clear
        let out_clr = handle_bookmark_command(&["clear".to_string()], &mut session);
        assert!(out_clr.contains("Cleared"));
    }

    // ========================================================================
    // Codebase Bookmark & Annotation Manager Tests
    // ========================================================================

    #[test]
    fn test_code_bookmark_creation_and_fields() {
        let bm = CodeBookmark::new("src/agent/bookmark.rs", 42)
            .with_kind(CodeBookmarkKind::Todo)
            .with_title("Refactor parser")
            .with_note("Need to optimize regex matching here")
            .with_tag("refactor")
            .with_tag("perf")
            .with_author("dev-alice");

        assert_eq!(bm.file_path, "src/agent/bookmark.rs");
        assert_eq!(bm.line_start, 42);
        assert_eq!(bm.line_end, None);
        assert_eq!(bm.kind, CodeBookmarkKind::Todo);
        assert_eq!(bm.title.as_deref(), Some("Refactor parser"));
        assert_eq!(bm.note.as_deref(), Some("Need to optimize regex matching here"));
        assert_eq!(bm.tags, vec!["refactor", "perf"]);
        assert_eq!(bm.author.as_deref(), Some("dev-alice"));
        assert_eq!(bm.line_display(), "L42");
        assert_eq!(bm.location_display(), "src/agent/bookmark.rs:L42");
        assert!(bm.contains_line(42));
        assert!(!bm.contains_line(43));
    }

    #[test]
    fn test_code_bookmark_range_and_location() {
        let bm = CodeBookmark::with_range("src/main.rs", 10, 25)
            .with_kind(CodeBookmarkKind::Bug)
            .with_snippet("fn handle_connection() {\n    panic!(\"not implemented\");\n}");

        assert_eq!(bm.line_start, 10);
        assert_eq!(bm.line_end, Some(25));
        assert_eq!(bm.line_display(), "L10-L25");
        assert_eq!(bm.location_display(), "src/main.rs:L10-L25");
        assert!(bm.contains_line(10));
        assert!(bm.contains_line(18));
        assert!(bm.contains_line(25));
        assert!(!bm.contains_line(9));
        assert!(!bm.contains_line(26));
        assert!(bm.snippet.as_deref().unwrap().contains("handle_connection"));
    }

    #[test]
    fn test_code_bookmark_kind_parsing() {
        assert_eq!(CodeBookmarkKind::from_str_loose("todo"), Some(CodeBookmarkKind::Todo));
        assert_eq!(CodeBookmarkKind::from_str_loose("bug"), Some(CodeBookmarkKind::Bug));
        assert_eq!(CodeBookmarkKind::from_str_loose("fixme"), Some(CodeBookmarkKind::Bug));
        assert_eq!(CodeBookmarkKind::from_str_loose("refactor"), Some(CodeBookmarkKind::Refactor));
        assert_eq!(CodeBookmarkKind::from_str_loose("security"), Some(CodeBookmarkKind::Security));
        assert_eq!(CodeBookmarkKind::from_str_loose("perf"), Some(CodeBookmarkKind::Performance));
        assert_eq!(CodeBookmarkKind::from_str_loose("bookmark"), Some(CodeBookmarkKind::Bookmark));
        assert_eq!(CodeBookmarkKind::from_str_loose("unknown_value"), None);
    }

    #[test]
    fn test_code_bookmark_store_crud() {
        let mut store = CodeBookmarkStore::new();
        assert!(store.is_empty());
        assert_eq!(store.len(), 0);

        let id1 = store.add(
            CodeBookmark::new("src/auth.rs", 50)
                .with_kind(CodeBookmarkKind::Security)
                .with_note("Verify token expiration"),
        );

        let id2 = store.add(
            CodeBookmark::with_range("src/db.rs", 100, 120)
                .with_kind(CodeBookmarkKind::Performance)
                .with_note("Add index for user_id"),
        );

        assert_eq!(store.len(), 2);
        assert!(!store.is_empty());

        // Get by ID
        let b1 = store.get(&id1).expect("should find id1");
        assert_eq!(b1.file_path, "src/auth.rs");
        assert_eq!(b1.kind, CodeBookmarkKind::Security);

        // Find by ID prefix
        let prefix = &id2.to_string()[..8];
        let b2 = store.find_by_id_str(prefix).expect("should find id2 by prefix");
        assert_eq!(b2.file_path, "src/db.rs");

        // Update note
        assert!(store.update_note(&id1, Some("Updated security note".to_string())));
        assert_eq!(store.get(&id1).unwrap().note.as_deref(), Some("Updated security note"));

        // Update snippet
        assert!(store.update_snippet(&id1, Some("let token = parse();".to_string())));
        assert_eq!(store.get(&id1).unwrap().snippet.as_deref(), Some("let token = parse();"));

        // Update lines
        assert!(store.update_lines(&id1, 55, Some(60)));
        assert_eq!(store.get(&id1).unwrap().line_start, 55);
        assert_eq!(store.get(&id1).unwrap().line_end, Some(60));

        // Tagging
        assert!(store.add_tag(&id1, "jwt"));
        assert!(store.get(&id1).unwrap().has_tag("jwt"));
        assert!(store.remove_tag(&id1, "jwt"));
        assert!(!store.get(&id1).unwrap().has_tag("jwt"));

        // Remove
        let removed = store.remove(&id1);
        assert!(removed.is_some());
        assert_eq!(store.len(), 1);
        assert!(store.get(&id1).is_none());

        // Clear
        let cleared = store.clear();
        assert_eq!(cleared, 1);
        assert!(store.is_empty());
    }

    #[test]
    fn test_code_bookmark_search_and_filtering() {
        let mut store = CodeBookmarkStore::new();

        store.add(
            CodeBookmark::new("src/auth/jwt.rs", 30)
                .with_kind(CodeBookmarkKind::Security)
                .with_title("JWT Validation")
                .with_note("Verify RSA signature format")
                .with_tag("auth")
                .with_tag("crypto"),
        );

        store.add(
            CodeBookmark::with_range("src/auth/session.rs", 10, 20)
                .with_kind(CodeBookmarkKind::Todo)
                .with_title("Session Expiry")
                .with_note("Add sliding window expiration")
                .with_tag("auth")
                .with_tag("session"),
        );

        store.add(
            CodeBookmark::new("src/db/pool.rs", 80)
                .with_kind(CodeBookmarkKind::Performance)
                .with_title("Connection Pool Limit")
                .with_note("Optimize max pool size")
                .with_tag("database")
                .with_tag("perf"),
        );

        // 1. Keyword search (auth)
        let auth_search = store.search("auth");
        assert_eq!(auth_search.len(), 2);

        // 2. Keyword search in notes (sliding)
        let note_search = store.search("sliding");
        assert_eq!(note_search.len(), 1);
        assert_eq!(note_search[0].file_path, "src/auth/session.rs");

        // 3. Find by path prefix
        let prefix_bms = store.find_by_path_prefix("src/auth");
        assert_eq!(prefix_bms.len(), 2);

        let db_prefix = store.find_by_path_prefix("src/db");
        assert_eq!(db_prefix.len(), 1);

        // 4. Find by tag
        let crypto_bms = store.find_by_tag("crypto");
        assert_eq!(crypto_bms.len(), 1);
        assert_eq!(crypto_bms[0].file_path, "src/auth/jwt.rs");

        // 5. Find by kind
        let sec_bms = store.find_by_kind(CodeBookmarkKind::Security);
        assert_eq!(sec_bms.len(), 1);

        // 6. Complex filter
        let filter = CodeBookmarkFilter::new()
            .with_path_prefix("src/auth")
            .with_kind(CodeBookmarkKind::Security)
            .with_tag("auth");

        let filtered = store.filter(&filter);
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].title.as_deref(), Some("JWT Validation"));
    }

    #[test]
    fn test_code_bookmark_markdown_export() {
        let mut store = CodeBookmarkStore::new();

        store.add(
            CodeBookmark::with_range("src/crypto.rs", 15, 25)
                .with_kind(CodeBookmarkKind::Security)
                .with_title("Constant Time Comparison")
                .with_note("Prevent timing attacks on token hashes")
                .with_snippet("pub fn secure_compare(a: &[u8], b: &[u8]) -> bool {\n    subtle::ConstantTimeEq::ct_eq(a, b).into()\n}")
                .with_tag("security")
                .with_author("sec-team"),
        );

        store.add(
            CodeBookmark::new("src/main.rs", 5)
                .with_kind(CodeBookmarkKind::Todo)
                .with_title("CLI Config Parsing")
                .with_note("Add support for --config flag")
                .with_tag("cli"),
        );

        let md = store.export_markdown(Some("Project Audit Bookmarks"));

        assert!(md.contains("# Project Audit Bookmarks"));
        assert!(md.contains("## 📊 Summary"));
        assert!(md.contains("Total Bookmarks:** 2"));
        assert!(md.contains("Files Referenced:** 2"));
        assert!(md.contains("## 📑 Index Table"));
        assert!(md.contains("## 📁 Bookmarks by File"));
        assert!(md.contains("src/crypto.rs"));
        assert!(md.contains("src/main.rs"));
        assert!(md.contains("Constant Time Comparison"));
        assert!(md.contains("Prevent timing attacks on token hashes"));
        assert!(md.contains("```rust"));
        assert!(md.contains("secure_compare"));
        assert!(md.contains("`#security`"));
    }

    #[test]
    fn test_code_bookmark_single_markdown() {
        let bm = CodeBookmark::new("src/engine.rs", 100)
            .with_kind(CodeBookmarkKind::Performance)
            .with_title("JIT Loop Unrolling")
            .with_note("Unroll small loops for 2x speedup")
            .with_snippet("for i in 0..4 { process(i); }")
            .with_tag("jit")
            .with_author("compiler-dev");

        let single_md = bm.to_markdown();
        assert!(single_md.contains("src/engine.rs:L100"));
        assert!(single_md.contains("JIT Loop Unrolling"));
        assert!(single_md.contains("Unroll small loops"));
        assert!(single_md.contains("```rust"));
        assert!(single_md.contains("`#jit`"));
    }

    #[test]
    fn test_code_bookmark_json_serialization() {
        let mut store = CodeBookmarkStore::new();
        store.add(
            CodeBookmark::new("src/lib.rs", 1)
                .with_kind(CodeBookmarkKind::Note)
                .with_note("Entry point of library"),
        );

        let json = store.export_json().expect("should export json");
        assert!(json.contains("src/lib.rs"));

        let mut store2 = CodeBookmarkStore::new();
        let count = store2.import_json(&json).expect("should import json");
        assert_eq!(count, 1);
        assert_eq!(store2.len(), 1);
        assert_eq!(store2.list()[0].file_path, "src/lib.rs");
    }

    #[test]
    fn test_code_bookmark_session_integration() {
        let mut session = create_sample_session();
        let mut store = CodeBookmarkStore::new();

        store.add(
            CodeBookmark::new("src/parser.rs", 120)
                .with_kind(CodeBookmarkKind::Refactor)
                .with_note("Simplify recursive descent"),
        );

        save_code_bookmark_store(&mut session, &store).expect("save should succeed");

        let loaded_store = load_code_bookmark_store(&session);
        assert_eq!(loaded_store.len(), 1);
        assert_eq!(loaded_store.list()[0].file_path, "src/parser.rs");

        let md = export_code_bookmarks_markdown(&session, Some("Session Code Bookmarks"));
        assert!(md.contains("src/parser.rs"));
        assert!(md.contains("Simplify recursive descent"));
    }

    #[test]
    fn test_extract_snippet_from_content() {
        let code = "line 1\nline 2\nline 3\nline 4\nline 5";

        let snip1 = extract_snippet_from_content(code, 2, Some(4)).expect("snippet 2-4");
        assert_eq!(snip1, "line 2\nline 3\nline 4");

        let snip_single = extract_snippet_from_content(code, 3, None).expect("snippet line 3");
        assert_eq!(snip_single, "line 3");

        let snip_invalid = extract_snippet_from_content(code, 10, None);
        assert!(snip_invalid.is_none());

        let snip_zero = extract_snippet_from_content(code, 0, None);
        assert!(snip_zero.is_none());
    }

    #[test]
    fn test_detect_language_from_path() {
        assert_eq!(detect_language_from_path("src/lib.rs"), "rust");
        assert_eq!(detect_language_from_path("index.ts"), "typescript");
        assert_eq!(detect_language_from_path("app.tsx"), "tsx");
        assert_eq!(detect_language_from_path("script.py"), "python");
        assert_eq!(detect_language_from_path("main.go"), "go");
        assert_eq!(detect_language_from_path("config.toml"), "toml");
        assert_eq!(detect_language_from_path("data.json"), "json");
        assert_eq!(detect_language_from_path("style.css"), "css");
        assert_eq!(detect_language_from_path("readme.md"), "markdown");
        assert_eq!(detect_language_from_path("file.unknown_ext"), "");
    }

    #[test]
    fn test_code_bookmark_empty_store_markdown() {
        let store = CodeBookmarkStore::new();
        let md = store.export_markdown(None);
        assert!(md.contains("# Codebase Bookmarks & Annotations"));
        assert!(md.contains("No codebase bookmarks recorded."));
    }
}
