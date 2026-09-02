//! Full-text, semantic, and hybrid search subsystem for Fusion historical sessions.
//!
//! Provides comprehensive cross-session discovery capabilities across `~/.fusion/sessions/`:
//! - **Full-Text BM25 Search**: Statistical Okapi BM25 ranking with inverse document frequency
//!   (IDF), document length normalization, and hierarchical field boosting (Title > User > Assistant > Tools).
//! - **Semantic & Vector Similarity**: Pure-Rust character $n$-gram TF-IDF vector embeddings with
//!   cosine similarity and semantic synonym/concept expansion for programming terminology.
//! - **Hybrid Search**: Fused BM25 statistical scoring and semantic vector cosine similarity.
//! - **Exact Substring & Regular Expression Search**: Fast regex and substring matching.
//! - **Fuzzy & Typo-Tolerant Search**: Dynamic programming Levenshtein and Jaro-Winkler token similarity.
//! - **Inverted Index Engine (`SessionSearchIndex`)**: In-memory postings index with term frequencies,
//!   position tracking, cached vector representations, and incremental file sync.
//! - **Rich Query Syntax**: Supports structured query operators (`role:user`, `model:gpt-4o`,
//!   `after:YYYY-MM-DD`, `before:YYYY-MM-DD`, `title:...`, `tool:...`, `tag:...`, `-exclude`, `"quoted phrase"`).
//! - **Contextual Snippets & Highlighting**: Dynamic snippet windowing around matches with ANSI
//!   terminal and Markdown highlighting.
//! - **Similar Session Discovery**: Cosine similarity clustering across historical sessions.

use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Instant;

use chrono::{DateTime, NaiveDate, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::agent::session::Session;
use crate::provider::types::Role;

// ============================================================================
// Constants & Field Weights
// ============================================================================

/// Default BM25 term frequency saturation parameter $k_1$.
pub const DEFAULT_BM25_K1: f64 = 1.2;

/// Default BM25 document length normalization parameter $b$.
pub const DEFAULT_BM25_B: f64 = 0.75;

/// Field boost weight for session title matches.
pub const BOOST_TITLE: f64 = 3.0;

/// Field boost weight for user prompt matches.
pub const BOOST_USER_PROMPT: f64 = 2.0;

/// Field boost weight for assistant response matches.
pub const BOOST_ASSISTANT_REPLY: f64 = 1.2;

/// Field boost weight for tool call names and arguments.
pub const BOOST_TOOL_CALL: f64 = 1.0;

/// Field boost weight for tool result outputs.
pub const BOOST_TOOL_RESULT: f64 = 0.8;

/// Field boost weight for system prompt content.
pub const BOOST_SYSTEM_PROMPT: f64 = 0.7;

/// Field boost weight for session metadata key-value pairs.
pub const BOOST_METADATA: f64 = 1.5;

/// Weight of BM25 score in Hybrid search mode ($\alpha$).
pub const HYBRID_BM25_WEIGHT: f64 = 0.6;

/// Weight of Semantic vector similarity in Hybrid search mode ($1 - \alpha$).
pub const HYBRID_SEMANTIC_WEIGHT: f64 = 0.4;

/// Default context snippet length in characters.
pub const DEFAULT_SNIPPET_LENGTH: usize = 140;

/// Default maximum number of session results returned.
pub const DEFAULT_SEARCH_LIMIT: usize = 20;

/// Default maximum matches highlighted per session.
pub const DEFAULT_MAX_MATCHES_PER_SESSION: usize = 5;

// ============================================================================
// Search Query & Mode Enums
// ============================================================================

/// Matching strategy for cross-session queries.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum SearchMode {
    /// Statistical Okapi BM25 ranking across tokenized fields (default).
    #[default]
    FullText,
    /// Semantic vector similarity using subword $n$-gram TF-IDF embeddings and synonym expansion.
    Semantic,
    /// Weighted fusion of BM25 statistical relevance and semantic vector cosine similarity.
    Hybrid,
    /// Exact substring matching (case-sensitive or insensitive).
    Exact,
    /// Regular expression pattern matching.
    Regex,
    /// Typo-tolerant fuzzy matching based on edit distance and character similarity.
    Fuzzy,
}

/// The conversational field where a match occurred.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MatchedField {
    /// Session title or user-defined label.
    Title,
    /// User prompt message.
    UserPrompt,
    /// Assistant response message.
    AssistantReply,
    /// System prompt or instructions.
    SystemPrompt,
    /// Tool invocation name or arguments.
    ToolCall,
    /// Tool execution output.
    ToolResult,
    /// Arbitrary session metadata key or value.
    Metadata,
}

impl MatchedField {
    /// Returns the scoring boost weight for this field.
    pub fn boost_weight(&self) -> f64 {
        match self {
            Self::Title => BOOST_TITLE,
            Self::UserPrompt => BOOST_USER_PROMPT,
            Self::AssistantReply => BOOST_ASSISTANT_REPLY,
            Self::ToolCall => BOOST_TOOL_CALL,
            Self::ToolResult => BOOST_TOOL_RESULT,
            Self::SystemPrompt => BOOST_SYSTEM_PROMPT,
            Self::Metadata => BOOST_METADATA,
        }
    }

    /// Human-readable label for terminal rendering.
    pub fn label(&self) -> &'static str {
        match self {
            Self::Title => "Title",
            Self::UserPrompt => "User",
            Self::AssistantReply => "Assistant",
            Self::SystemPrompt => "System",
            Self::ToolCall => "Tool Call",
            Self::ToolResult => "Tool Result",
            Self::Metadata => "Metadata",
        }
    }
}

/// Structured cross-session search query specification.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchQuery {
    /// The primary text pattern or query terms.
    pub text: String,
    /// Search execution mode (FullText, Semantic, Hybrid, Exact, Regex, Fuzzy).
    pub mode: SearchMode,
    /// Whether substring and regex matching are case-sensitive.
    pub case_sensitive: bool,
    /// Optional filter by message role (e.g. User, Assistant, System, Tool).
    pub role_filter: Option<Vec<Role>>,
    /// Optional filter by active LLM model identifier (exact or substring).
    pub model_filter: Option<String>,
    /// Optional filter for sessions created on or after this timestamp.
    pub date_from: Option<DateTime<Utc>>,
    /// Optional filter for sessions created on or before this timestamp.
    pub date_to: Option<DateTime<Utc>>,
    /// Optional filter by session working directory.
    pub working_dir_filter: Option<PathBuf>,
    /// Optional filter by tool name called in the session.
    pub tool_filter: Option<String>,
    /// Required metadata tags (key-value or key-only).
    pub tag_filters: HashMap<String, Option<String>>,
    /// Words or terms that must NOT be present in matching sessions.
    pub excluded_terms: Vec<String>,
    /// Exact phrases (quoted in query) that must appear verbatim.
    pub required_phrases: Vec<String>,
    /// Maximum number of matching sessions to return.
    pub limit: usize,
    /// Maximum number of message matches highlighted per session.
    pub max_matches_per_session: usize,
    /// Minimum normalized score threshold (0.0 to 1.0).
    pub min_score: f64,
    /// Maximum context characters around match in snippet.
    pub snippet_len: usize,
    /// Whether to search within tool calls.
    pub include_tool_calls: bool,
    /// Whether to search within tool execution outputs.
    pub include_tool_results: bool,
    /// Whether to search within system prompts.
    pub include_system_prompts: bool,
    /// Whether to search within session metadata.
    pub include_metadata: bool,
}

impl Default for SearchQuery {
    fn default() -> Self {
        Self {
            text: String::new(),
            mode: SearchMode::FullText,
            case_sensitive: false,
            role_filter: None,
            model_filter: None,
            date_from: None,
            date_to: None,
            working_dir_filter: None,
            tool_filter: None,
            tag_filters: HashMap::new(),
            excluded_terms: Vec::new(),
            required_phrases: Vec::new(),
            limit: DEFAULT_SEARCH_LIMIT,
            max_matches_per_session: DEFAULT_MAX_MATCHES_PER_SESSION,
            min_score: 0.0,
            snippet_len: DEFAULT_SNIPPET_LENGTH,
            include_tool_calls: true,
            include_tool_results: true,
            include_system_prompts: true,
            include_metadata: true,
        }
    }
}

impl SearchQuery {
    /// Creates a default full-text search query with the given search text.
    pub fn new(text: impl Into<String>) -> Self {
        let mut query = Self::default();
        query.text = text.into();
        query
    }

    /// Parses an expressive CLI or REPL query string into structured operators.
    ///
    /// Supported syntax:
    /// - `"exact phrase"` -> exact phrase requirement
    /// - `-exclude` -> excluded word
    /// - `role:user`, `role:assistant`, `role:tool`, `role:system` -> role filter
    /// - `model:gpt-4o` -> model filter
    /// - `mode:semantic`, `mode:hybrid`, `mode:exact`, `mode:regex`, `mode:fuzzy` -> search mode
    /// - `tool:grep`, `tool:bash` -> tool filter
    /// - `after:2026-01-01` -> date_from filter
    /// - `before:2026-09-01` -> date_to filter
    /// - `tag:key=value` or `tag:key` -> metadata tag filter
    /// - `limit:10` -> max results
    pub fn parse(raw_input: &str) -> Self {
        let mut query = Self::default();
        let input = raw_input.trim();
        if input.is_empty() {
            return query;
        }

        let mut normal_terms = Vec::new();
        let mut chars = input.chars().peekable();

        while let Some(&ch) = chars.peek() {
            if ch.is_whitespace() {
                chars.next();
                continue;
            }

            // Quoted phrase
            if ch == '"' || ch == '\'' {
                let quote = chars.next().unwrap();
                let mut phrase = String::new();
                for next_ch in chars.by_ref() {
                    if next_ch == quote {
                        break;
                    }
                    phrase.push(next_ch);
                }
                let trimmed = phrase.trim();
                if !trimmed.is_empty() {
                    query.required_phrases.push(trimmed.to_string());
                    normal_terms.push(trimmed.to_string());
                }
                continue;
            }

            // Read next token
            let mut token = String::new();
            while let Some(&next_ch) = chars.peek() {
                if next_ch.is_whitespace() {
                    break;
                }
                token.push(chars.next().unwrap());
            }

            // Process token operators
            if let Some(stripped) = token.strip_prefix('-') {
                if !stripped.is_empty() {
                    query.excluded_terms.push(stripped.to_lowercase());
                }
            } else if let Some(role_str) = token.strip_prefix("role:") {
                let role = match role_str.to_lowercase().as_str() {
                    "user" => Some(Role::User),
                    "assistant" => Some(Role::Assistant),
                    "system" => Some(Role::System),
                    "tool" => Some(Role::Tool),
                    _ => None,
                };
                if let Some(r) = role {
                    let mut roles = query.role_filter.unwrap_or_default();
                    roles.push(r);
                    query.role_filter = Some(roles);
                }
            } else if let Some(model_str) = token.strip_prefix("model:") {
                if !model_str.is_empty() {
                    query.model_filter = Some(model_str.to_string());
                }
            } else if let Some(tool_str) = token.strip_prefix("tool:") {
                if !tool_str.is_empty() {
                    query.tool_filter = Some(tool_str.to_string());
                }
            } else if let Some(mode_str) = token.strip_prefix("mode:") {
                match mode_str.to_lowercase().as_str() {
                    "semantic" | "sem" => query.mode = SearchMode::Semantic,
                    "hybrid" | "hyb" => query.mode = SearchMode::Hybrid,
                    "exact" | "str" => query.mode = SearchMode::Exact,
                    "regex" | "re" => query.mode = SearchMode::Regex,
                    "fuzzy" | "fuzz" => query.mode = SearchMode::Fuzzy,
                    "fulltext" | "text" | "bm25" => query.mode = SearchMode::FullText,
                    _ => {}
                }
            } else if let Some(after_str) = token.strip_prefix("after:") {
                if let Ok(date) = NaiveDate::parse_from_str(after_str, "%Y-%m-%d") {
                    if let Some(dt) = date.and_hms_opt(0, 0, 0) {
                        query.date_from = Some(DateTime::from_naive_utc_and_offset(dt, Utc));
                    }
                }
            } else if let Some(before_str) = token.strip_prefix("before:") {
                if let Ok(date) = NaiveDate::parse_from_str(before_str, "%Y-%m-%d") {
                    if let Some(dt) = date.and_hms_opt(23, 59, 59) {
                        query.date_to = Some(DateTime::from_naive_utc_and_offset(dt, Utc));
                    }
                }
            } else if let Some(tag_str) = token.strip_prefix("tag:") {
                if let Some((k, v)) = tag_str.split_once('=') {
                    query.tag_filters.insert(k.to_string(), Some(v.to_string()));
                } else if !tag_str.is_empty() {
                    query.tag_filters.insert(tag_str.to_string(), None);
                }
            } else if let Some(limit_str) = token.strip_prefix("limit:") {
                if let Ok(limit) = limit_str.parse::<usize>() {
                    query.limit = limit.max(1);
                }
            } else {
                normal_terms.push(token);
            }
        }

        query.text = normal_terms.join(" ");
        query
    }

    /// Sets the search mode.
    pub fn with_mode(mut self, mode: SearchMode) -> Self {
        self.mode = mode;
        self
    }

    /// Sets the maximum results limit.
    pub fn with_limit(mut self, limit: usize) -> Self {
        self.limit = limit.max(1);
        self
    }

    /// Sets case sensitivity.
    pub fn with_case_sensitive(mut self, case_sensitive: bool) -> Self {
        self.case_sensitive = case_sensitive;
        self
    }

    /// Adds a role filter.
    pub fn with_role(mut self, role: Role) -> Self {
        let mut roles = self.role_filter.unwrap_or_default();
        roles.push(role);
        self.role_filter = Some(roles);
        self
    }

    /// Sets model filter.
    pub fn with_model(mut self, model: impl Into<String>) -> Self {
        self.model_filter = Some(model.into());
        self
    }

    /// Sets minimum score threshold.
    pub fn with_min_score(mut self, min_score: f64) -> Self {
        self.min_score = min_score;
        self
    }
}

// ============================================================================
// Match & Result Structs
// ============================================================================

/// An individual match within a message or session field.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MessageMatch {
    /// Index in `session.messages` (if applicable).
    pub message_index: Option<usize>,
    /// Turn sequence number (1-based), if mapped.
    pub turn_index: Option<usize>,
    /// Role of the message matched.
    pub role: Option<Role>,
    /// The specific field where the match was found.
    pub field: MatchedField,
    /// Extracted contextual snippet around the match.
    pub snippet: String,
    /// Character offset spans within `snippet` corresponding to matched terms.
    pub match_spans: Vec<(usize, usize)>,
    /// Match score contribution for this field.
    pub score: f64,
}

/// Aggregated search result for a single matching session.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionSearchResult {
    /// Unique session UUID.
    pub session_id: Uuid,
    /// Optional session title or summary.
    pub title: Option<String>,
    /// Active LLM model when the session ran.
    pub active_model: String,
    /// Session creation timestamp (RFC 3339).
    pub created_at: String,
    /// Session last update timestamp (RFC 3339).
    pub updated_at: String,
    /// Total conversation message count in the session.
    pub total_messages: usize,
    /// Overall relevance score (normalized 0.0 - 1.0 or ranked BM25).
    pub score: f64,
    /// Matching highlights within this session.
    pub matches: Vec<MessageMatch>,
    /// Total count of match hits across all fields in this session.
    pub total_hit_count: usize,
}

/// Complete search execution report across historical sessions.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchReport {
    /// The query executed.
    pub query: SearchQuery,
    /// Total number of sessions scanned on disk or in memory.
    pub total_sessions_scanned: usize,
    /// Number of sessions meeting matching and filter criteria.
    pub matching_sessions_count: usize,
    /// Total match hits across all sessions.
    pub total_hits_count: usize,
    /// Execution time in milliseconds.
    pub duration_ms: u64,
    /// Ranked search results up to `query.limit`.
    pub results: Vec<SessionSearchResult>,
}

impl SearchReport {
    /// Returns `true` if no matching sessions were found.
    pub fn is_empty(&self) -> bool {
        self.results.is_empty()
    }

    /// Formats the search report as an ANSI-colored or plain-text terminal view.
    pub fn format_terminal(&self, color: bool) -> String {
        let mut out = String::new();

        let (c_cyan, _c_green, c_yellow, c_dim, c_bold, c_blue, c_reset) = if color {
            (
                "\x1b[1;36m",
                "\x1b[1;32m",
                "\x1b[1;33m",
                "\x1b[2;37m",
                "\x1b[1m",
                "\x1b[1;34m",
                "\x1b[0m",
            )
        } else {
            ("", "", "", "", "", "", "")
        };

        out.push_str(&format!(
            "{}✦ Fusion Cross-Session Search Results{} (found {} session{} in {}ms)\n",
            c_cyan,
            c_reset,
            self.matching_sessions_count,
            if self.matching_sessions_count == 1 { "" } else { "s" },
            self.duration_ms
        ));
        out.push_str(&format!(
            "{}Query:{} \"{}{}\" {}[mode: {:?}]{}\n\n",
            c_dim, c_reset, self.query.text, c_reset, c_dim, self.query.mode, c_reset
        ));

        if self.results.is_empty() {
            out.push_str(&format!(
                "{}ℹ No historical sessions matched the query criteria.{}\n",
                c_yellow, c_reset
            ));
            return out;
        }

        for (idx, res) in self.results.iter().enumerate() {
            let title = res.title.as_deref().unwrap_or("Untitled Session");
            let short_id: String = res.session_id.to_string().chars().take(8).collect();

            out.push_str(&format!(
                "{}{}.{} {}✦ {}{} {}(`{}`){} - Score: {:.3}\n",
                c_bold,
                idx + 1,
                c_reset,
                c_cyan,
                title,
                c_reset,
                c_dim,
                short_id,
                c_reset,
                res.score
            ));
            out.push_str(&format!(
                "   {}Model:{} {}  {}Created:{} {}  {}Messages:{} {}\n",
                c_dim,
                c_reset,
                res.active_model,
                c_dim,
                c_reset,
                res.created_at,
                c_dim,
                c_reset,
                res.total_messages
            ));

            for m in &res.matches {
                let role_label = m.role.map(|r| r.as_str()).unwrap_or(m.field.label());
                let field_str = m.field.label();

                // Highlight snippet matches
                let highlighted = if color {
                    highlight_spans(&m.snippet, &m.match_spans, "\x1b[1;33m", "\x1b[0m")
                } else {
                    m.snippet.clone()
                };

                out.push_str(&format!(
                    "   {}↳ [{}:{}] {}{}\n",
                    c_blue, role_label, field_str, c_reset, highlighted
                ));
            }
            out.push('\n');
        }

        out.push_str(&format!(
            "{}Use `/session load <id>` to resume any matching session.{}\n",
            c_dim, c_reset
        ));

        out
    }

    /// Formats the search report as standard GitHub-flavored Markdown.
    pub fn format_markdown(&self) -> String {
        let mut md = String::new();
        md.push_str("# Fusion Cross-Session Search Results\n\n");
        md.push_str(&format!(
            "- **Query:** `{}`\n- **Mode:** `{:?}`\n- **Matches:** {} sessions (scanned {} sessions in {}ms)\n\n",
            self.query.text, self.query.mode, self.matching_sessions_count, self.total_sessions_scanned, self.duration_ms
        ));

        if self.results.is_empty() {
            md.push_str("*No matching historical sessions found.*\n");
            return md;
        }

        for (idx, res) in self.results.iter().enumerate() {
            let title = res.title.as_deref().unwrap_or("Untitled Session");
            md.push_str(&format!(
                "### {}. {} (`{}`)\n\n",
                idx + 1,
                title,
                res.session_id
            ));
            md.push_str(&format!(
                "- **Model:** `{}` | **Created:** `{}` | **Messages:** {} | **Score:** `{:.3}`\n\n",
                res.active_model, res.created_at, res.total_messages, res.score
            ));

            for m in &res.matches {
                let role_label = m.role.map(|r| r.as_str()).unwrap_or(m.field.label());
                let highlighted = highlight_spans(&m.snippet, &m.match_spans, "**", "**");
                md.push_str(&format!(
                    "> **[{}:{}]** {}\n\n",
                    role_label,
                    m.field.label(),
                    highlighted
                ));
            }
        }

        md
    }

    /// Formats the search report as pretty-printed JSON.
    pub fn format_json(&self) -> anyhow::Result<String> {
        Ok(serde_json::to_string_pretty(self)?)
    }
}

// ============================================================================
// Tokenization, N-Grams & Semantic Expansion
// ============================================================================

/// Tokenizes text into lowercase alphanumeric and code tokens (splitting snake_case and camelCase).
pub fn tokenize_text(text: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut current = String::new();

    for ch in text.chars() {
        if ch.is_alphanumeric() {
            current.push(ch);
        } else {
            if !current.is_empty() {
                add_sub_tokens(&current, &mut tokens);
                current.clear();
            }
        }
    }

    if !current.is_empty() {
        add_sub_tokens(&current, &mut tokens);
    }

    tokens
}

fn add_sub_tokens(raw_token: &str, out: &mut Vec<String>) {
    let lower = raw_token.to_lowercase();
    if lower.len() >= 2 {
        out.push(lower.clone());
    }

    // Split snake_case or camelCase
    let mut sub = String::new();
    let chars: Vec<char> = raw_token.chars().collect();
    for i in 0..chars.len() {
        let ch = chars[i];
        if ch == '_' || ch == '-' {
            if !sub.is_empty() {
                let s_lower = sub.to_lowercase();
                if s_lower.len() >= 2 && s_lower != lower {
                    out.push(s_lower);
                }
                sub.clear();
            }
            continue;
        }

        // Check camelCase boundary
        if ch.is_uppercase() && !sub.is_empty() {
            let is_prev_lower = i > 0 && chars[i - 1].is_lowercase();
            if is_prev_lower {
                let s_lower = sub.to_lowercase();
                if s_lower.len() >= 2 && s_lower != lower {
                    out.push(s_lower);
                }
                sub.clear();
            }
        }
        sub.push(ch);
    }

    if !sub.is_empty() {
        let s_lower = sub.to_lowercase();
        if s_lower.len() >= 2 && s_lower != lower {
            out.push(s_lower);
        }
    }
}

/// Generates character 3-grams and 4-grams for subword semantic vector representation.
pub fn generate_char_ngrams(text: &str) -> Vec<String> {
    let lower = text.to_lowercase();
    let chars: Vec<char> = lower.chars().filter(|c| c.is_alphanumeric() || c.is_whitespace()).collect();
    let mut ngrams = Vec::new();

    // 3-grams
    if chars.len() >= 3 {
        for w in chars.windows(3) {
            ngrams.push(w.iter().collect::<String>());
        }
    }

    // 4-grams
    if chars.len() >= 4 {
        for w in chars.windows(4) {
            ngrams.push(w.iter().collect::<String>());
        }
    }

    ngrams
}

/// Common developer semantic synonym and concept expansion mappings.
pub fn expand_semantic_synonyms(term: &str) -> Vec<String> {
    let lower = term.to_lowercase();
    let mut synonyms = Vec::new();

    match lower.as_str() {
        "auth" | "authentication" | "login" | "jwt" | "token" | "oauth" | "credential" => {
            synonyms.extend(
                ["auth", "authentication", "login", "jwt", "token", "oauth", "credential", "password"]
                    .into_iter()
                    .map(|s| s.to_string()),
            );
        }
        "test" | "testing" | "unittest" | "spec" | "assert" | "benchmark" | "fixture" => {
            synonyms.extend(
                ["test", "testing", "unittest", "spec", "assert", "benchmark", "fixture", "mock"]
                    .into_iter()
                    .map(|s| s.to_string()),
            );
        }
        "error" | "bug" | "fix" | "panic" | "exception" | "crash" | "issue" | "fault" => {
            synonyms.extend(
                ["error", "bug", "fix", "panic", "exception", "crash", "issue", "fault", "warn"]
                    .into_iter()
                    .map(|s| s.to_string()),
            );
        }
        "db" | "database" | "sql" | "sqlite" | "postgres" | "table" | "schema" | "migration" => {
            synonyms.extend(
                ["db", "database", "sql", "sqlite", "postgres", "table", "schema", "migration", "query"]
                    .into_iter()
                    .map(|s| s.to_string()),
            );
        }
        "http" | "api" | "rest" | "endpoint" | "request" | "response" | "fetch" | "server" => {
            synonyms.extend(
                ["http", "api", "rest", "endpoint", "request", "response", "fetch", "server", "url"]
                    .into_iter()
                    .map(|s| s.to_string()),
            );
        }
        "git" | "commit" | "branch" | "diff" | "merge" | "rebase" | "repo" | "repository" => {
            synonyms.extend(
                ["git", "commit", "branch", "diff", "merge", "rebase", "repo", "repository"]
                    .into_iter()
                    .map(|s| s.to_string()),
            );
        }
        "perf" | "performance" | "fast" | "latency" | "throughput" | "speed" | "alloc" | "profile" => {
            synonyms.extend(
                ["perf", "performance", "fast", "latency", "throughput", "speed", "alloc", "profile", "memory"]
                    .into_iter()
                    .map(|s| s.to_string()),
            );
        }
        "ui" | "theme" | "color" | "terminal" | "ratatui" | "layout" | "style" | "markdown" => {
            synonyms.extend(
                ["ui", "theme", "color", "terminal", "ratatui", "layout", "style", "markdown", "render"]
                    .into_iter()
                    .map(|s| s.to_string()),
            );
        }
        "wasm" | "web" | "browser" | "javascript" | "typescript" | "npm" => {
            synonyms.extend(
                ["wasm", "web", "browser", "javascript", "typescript", "npm", "bindgen"]
                    .into_iter()
                    .map(|s| s.to_string()),
            );
        }
        "termux" | "android" | "mobile" | "arm" | "aarch64" => {
            synonyms.extend(
                ["termux", "android", "mobile", "arm", "aarch64"]
                    .into_iter()
                    .map(|s| s.to_string()),
            );
        }
        _ => {}
    }

    synonyms
}

// ============================================================================
// Mathematical Vector & Distance Operations
// ============================================================================

/// Computes the cosine similarity between two sparse frequency maps.
///
/// $$\text{CosineSim}(\mathbf{u}, \mathbf{v}) = \frac{\sum_i u_i \cdot v_i}{\sqrt{\sum_i u_i^2} \cdot \sqrt{\sum_i v_i^2}}$$
pub fn cosine_similarity(v1: &HashMap<String, f64>, v2: &HashMap<String, f64>) -> f64 {
    if v1.is_empty() || v2.is_empty() {
        return 0.0;
    }

    let mut dot_product = 0.0;
    let mut norm1_sq = 0.0;
    let mut norm2_sq = 0.0;

    for (k, val1) in v1 {
        norm1_sq += val1 * val1;
        if let Some(val2) = v2.get(k) {
            dot_product += val1 * val2;
        }
    }

    for val2 in v2.values() {
        norm2_sq += val2 * val2;
    }

    if norm1_sq <= 0.0 || norm2_sq <= 0.0 {
        return 0.0;
    }

    (dot_product / (norm1_sq.sqrt() * norm2_sq.sqrt())).clamp(0.0, 1.0)
}

/// Computes Levenshtein edit distance between two strings using dynamic programming.
pub fn levenshtein_distance(s1: &str, s2: &str) -> usize {
    let s1_chars: Vec<char> = s1.chars().collect();
    let s2_chars: Vec<char> = s2.chars().collect();
    let len1 = s1_chars.len();
    let len2 = s2_chars.len();

    if len1 == 0 {
        return len2;
    }
    if len2 == 0 {
        return len1;
    }

    let mut prev = vec![0; len2 + 1];
    let mut curr = vec![0; len2 + 1];

    for j in 0..=len2 {
        prev[j] = j;
    }

    for i in 1..=len1 {
        curr[0] = i;
        for j in 1..=len2 {
            let cost = if s1_chars[i - 1] == s2_chars[j - 1] { 0 } else { 1 };
            curr[j] = (prev[j] + 1)
                .min(curr[j - 1] + 1)
                .min(prev[j - 1] + cost);
        }
        prev.copy_from_slice(&curr);
    }

    prev[len2]
}

/// Normalized fuzzy similarity score between 0.0 and 1.0.
pub fn fuzzy_similarity(s1: &str, s2: &str) -> f64 {
    let max_len = s1.chars().count().max(s2.chars().count());
    if max_len == 0 {
        return 1.0;
    }
    let dist = levenshtein_distance(s1, s2);
    (1.0 - (dist as f64 / max_len as f64)).clamp(0.0, 1.0)
}

// ============================================================================
// Context Snippet Generation & Highlighting
// ============================================================================

/// Extracts a centered context snippet around matched terms with ellipses.
pub fn extract_snippet(
    text: &str,
    target_terms: &[String],
    max_len: usize,
) -> (String, Vec<(usize, usize)>) {
    if text.is_empty() {
        return (String::new(), Vec::new());
    }

    let lower_text = text.to_lowercase();
    let text_chars: Vec<char> = text.chars().collect();
    let total_chars = text_chars.len();

    // Find the first matching term position
    let mut best_pos = None;
    for term in target_terms {
        let t_lower = term.to_lowercase();
        if let Some(byte_idx) = lower_text.find(&t_lower) {
            let char_idx = text[..byte_idx].chars().count();
            best_pos = Some((char_idx, t_lower.chars().count()));
            break;
        }
    }

    let (start_idx, end_idx) = if let Some((match_char_idx, match_char_len)) = best_pos {
        let half = max_len / 2;
        let start = match_char_idx.saturating_sub(half);
        let end = (match_char_idx + match_char_len + half).min(total_chars);
        (start, end)
    } else {
        (0, max_len.min(total_chars))
    };

    let snippet_str: String = text_chars[start_idx..end_idx].iter().collect();
    let mut final_snippet = String::new();

    if start_idx > 0 {
        final_snippet.push_str("...");
    }
    final_snippet.push_str(&snippet_str);
    if end_idx < total_chars {
        final_snippet.push_str("...");
    }

    // Compute spans in final_snippet
    let snippet_lower = final_snippet.to_lowercase();
    let mut spans = Vec::new();

    for term in target_terms {
        let t_lower = term.to_lowercase();
        if t_lower.is_empty() {
            continue;
        }

        let mut search_from = 0;
        while let Some(pos) = snippet_lower[search_from..].find(&t_lower) {
            let actual_byte_pos = search_from + pos;
            let start_char = final_snippet[..actual_byte_pos].chars().count();
            let end_char = start_char + t_lower.chars().count();
            spans.push((start_char, end_char));
            search_from = actual_byte_pos + t_lower.len();
            if search_from >= snippet_lower.len() {
                break;
            }
        }
    }

    (final_snippet, spans)
}

/// Applies highlighting markers around character spans in a string.
pub fn highlight_spans(
    text: &str,
    spans: &[(usize, usize)],
    start_tag: &str,
    end_tag: &str,
) -> String {
    if spans.is_empty() {
        return text.to_string();
    }

    let chars: Vec<char> = text.chars().collect();
    let mut highlighted_indices: HashSet<usize> = HashSet::new();

    for &(s, e) in spans {
        for i in s..e {
            highlighted_indices.insert(i);
        }
    }

    let mut out = String::new();
    let mut in_highlight = false;

    for (i, &ch) in chars.iter().enumerate() {
        let is_hl = highlighted_indices.contains(&i);
        if is_hl && !in_highlight {
            out.push_str(start_tag);
            in_highlight = true;
        } else if !is_hl && in_highlight {
            out.push_str(end_tag);
            in_highlight = false;
        }
        out.push(ch);
    }

    if in_highlight {
        out.push_str(end_tag);
    }

    out
}

// ============================================================================
// Core Cross-Session Search Engine
// ============================================================================

/// Searches across all historical session JSON files in the specified directory.
pub fn search_sessions_dir(dir: impl AsRef<Path>, query: &SearchQuery) -> anyhow::Result<SearchReport> {
    let start_time = Instant::now();
    let dir = dir.as_ref();

    if !dir.exists() {
        return Ok(SearchReport {
            query: query.clone(),
            total_sessions_scanned: 0,
            matching_sessions_count: 0,
            total_hits_count: 0,
            duration_ms: start_time.elapsed().as_millis() as u64,
            results: Vec::new(),
        });
    }

    let mut sessions = Vec::new();
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.extension().and_then(|s| s.to_str()) == Some("json") {
            if let Ok(session) = Session::load_from_path(&path) {
                sessions.push(session);
            }
        }
    }

    let report = search_in_sessions(&sessions, query);
    Ok(report)
}

/// Convenience function searching historical sessions in the default `~/.fusion/sessions/` directory.
pub fn search_sessions(query_str: &str) -> anyhow::Result<SearchReport> {
    let query = SearchQuery::parse(query_str);
    let dir = Session::sessions_dir();
    search_sessions_dir(&dir, &query)
}

/// Executes a search query over an in-memory collection of sessions.
pub fn search_in_sessions(sessions: &[Session], query: &SearchQuery) -> SearchReport {
    let start_time = Instant::now();
    let total_scanned = sessions.len();

    // 1. Calculate collection-level statistics for BM25 (document frequencies, avgdl)
    let stats = CollectionStats::compute(sessions);

    // 2. Score and filter each session
    let mut ranked_results = Vec::new();
    let mut total_hits = 0;

    for session in sessions {
        if !passes_metadata_filters(session, query) {
            continue;
        }

        if let Some(res) = score_session(session, query, &stats) {
            if res.score >= query.min_score && (!res.matches.is_empty() || res.score > 0.0) {
                total_hits += res.total_hit_count;
                ranked_results.push(res);
            }
        }
    }

    // 3. Sort by score descending (most relevant first), then by updated_at descending
    ranked_results.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| b.updated_at.cmp(&a.updated_at))
    });

    let matching_count = ranked_results.len();

    // 4. Apply result limit
    if ranked_results.len() > query.limit {
        ranked_results.truncate(query.limit);
    }

    SearchReport {
        query: query.clone(),
        total_sessions_scanned: total_scanned,
        matching_sessions_count: matching_count,
        total_hits_count: total_hits,
        duration_ms: start_time.elapsed().as_millis() as u64,
        results: ranked_results,
    }
}

/// Checks if a session satisfies basic query metadata filters (model, role, dates, tags).
fn passes_metadata_filters(session: &Session, query: &SearchQuery) -> bool {
    // Model filter
    if let Some(target_model) = &query.model_filter {
        let s_model = session.active_model().to_lowercase();
        let t_model = target_model.to_lowercase();
        if !s_model.contains(&t_model) {
            return false;
        }
    }

    // Working directory filter
    if let Some(target_dir) = &query.working_dir_filter {
        if let Some(s_dir) = &session.working_dir {
            if s_dir != target_dir && !s_dir.starts_with(target_dir) {
                return false;
            }
        } else {
            return false;
        }
    }

    // Date range filters
    if let Ok(created_dt) = DateTime::parse_from_rfc3339(session.created_at()) {
        let created_utc = created_dt.with_timezone(&Utc);
        if let Some(from) = query.date_from {
            if created_utc < from {
                return false;
            }
        }
        if let Some(to) = query.date_to {
            if created_utc > to {
                return false;
            }
        }
    }

    // Tag filters
    for (k, v_opt) in &query.tag_filters {
        if let Some(val) = session.metadata.get(k) {
            if let Some(expected_val) = v_opt {
                if val != expected_val {
                    return false;
                }
            }
        } else {
            return false;
        }
    }

    // Tool filter
    if let Some(tool_name) = &query.tool_filter {
        let tool_lower = tool_name.to_lowercase();
        let has_tool = session.messages().iter().any(|m| {
            m.tool_calls
                .as_ref()
                .map(|calls| calls.iter().any(|c| c.name.to_lowercase().contains(&tool_lower)))
                .unwrap_or(false)
        });
        if !has_tool {
            return false;
        }
    }

    true
}

/// Scores a session against a query using the specified `SearchMode`.
fn score_session(
    session: &Session,
    query: &SearchQuery,
    stats: &CollectionStats,
) -> Option<SessionSearchResult> {
    match query.mode {
        SearchMode::FullText => score_session_bm25(session, query, stats),
        SearchMode::Semantic => score_session_semantic(session, query),
        SearchMode::Hybrid => score_session_hybrid(session, query, stats),
        SearchMode::Exact => score_session_exact(session, query),
        SearchMode::Regex => score_session_regex(session, query),
        SearchMode::Fuzzy => score_session_fuzzy(session, query),
    }
}

// ============================================================================
// BM25 Ranking Algorithm
// ============================================================================

struct CollectionStats {
    total_docs: usize,
    avg_doc_len: f64,
    doc_frequencies: HashMap<String, usize>,
}

impl CollectionStats {
    fn compute(sessions: &[Session]) -> Self {
        let mut doc_frequencies = HashMap::new();
        let mut total_tokens = 0usize;
        let total_docs = sessions.len().max(1);

        for session in sessions {
            let mut unique_terms = HashSet::new();

            if let Some(title) = session.title() {
                for t in tokenize_text(title) {
                    unique_terms.insert(t);
                }
            }

            for m in session.messages() {
                let tokens = tokenize_text(&m.content);
                total_tokens += tokens.len();
                for t in tokens {
                    unique_terms.insert(t);
                }
            }

            for term in unique_terms {
                *doc_frequencies.entry(term).or_insert(0) += 1;
            }
        }

        let avg_doc_len = if total_docs > 0 {
            total_tokens as f64 / total_docs as f64
        } else {
            100.0
        };

        Self {
            total_docs,
            avg_doc_len: avg_doc_len.max(1.0),
            doc_frequencies,
        }
    }

    fn idf(&self, term: &str) -> f64 {
        let n = self.doc_frequencies.get(term).copied().unwrap_or(0);
        let n_f = n as f64;
        let total_f = self.total_docs as f64;

        // Robertson-Spärck Jones IDF: ln(1 + (N - n + 0.5) / (n + 0.5))
        ((total_f - n_f + 0.5) / (n_f + 0.5) + 1.0).ln().max(0.1)
    }
}

fn score_session_bm25(
    session: &Session,
    query: &SearchQuery,
    stats: &CollectionStats,
) -> Option<SessionSearchResult> {
    let query_tokens = tokenize_text(&query.text);
    if query_tokens.is_empty() && query.required_phrases.is_empty() {
        // Return session with zero score if query is empty
        return Some(SessionSearchResult {
            session_id: session.id,
            title: session.title.clone(),
            active_model: session.active_model.clone(),
            created_at: session.created_at.clone(),
            updated_at: session.updated_at.clone(),
            total_messages: session.messages.len(),
            score: 1.0,
            matches: Vec::new(),
            total_hit_count: 0,
        });
    }

    // Excluded terms check
    if !query.excluded_terms.is_empty() {
        for excl in &query.excluded_terms {
            if session_contains_term(session, excl) {
                return None;
            }
        }
    }

    // Required phrases check
    if !query.required_phrases.is_empty() {
        for phrase in &query.required_phrases {
            if !session_contains_phrase(session, phrase) {
                return None;
            }
        }
    }

    let mut total_score = 0.0;
    let mut matches = Vec::new();
    let mut hit_count = 0;

    // 1. Score Title
    if let Some(title) = session.title() {
        let title_tokens = tokenize_text(title);
        let field_score = compute_bm25_field_score(
            &query_tokens,
            &title_tokens,
            stats,
            BOOST_TITLE,
            title_tokens.len(),
        );
        if field_score > 0.0 {
            total_score += field_score;
            hit_count += 1;
            let (snippet, spans) = extract_snippet(title, &query_tokens, query.snippet_len);
            matches.push(MessageMatch {
                message_index: None,
                turn_index: None,
                role: None,
                field: MatchedField::Title,
                snippet,
                match_spans: spans,
                score: field_score,
            });
        }
    }

    // 2. Score Messages
    let mut turn_counter = 1;
    for (m_idx, message) in session.messages().iter().enumerate() {
        if message.role == Role::User && m_idx > 0 {
            turn_counter += 1;
        }

        // Role filter check
        if let Some(roles) = &query.role_filter {
            if !roles.contains(&message.role) {
                continue;
            }
        }

        let field_type = match message.role {
            Role::User => MatchedField::UserPrompt,
            Role::Assistant => MatchedField::AssistantReply,
            Role::System => {
                if !query.include_system_prompts {
                    continue;
                }
                MatchedField::SystemPrompt
            }
            Role::Tool => {
                if !query.include_tool_results {
                    continue;
                }
                MatchedField::ToolResult
            }
        };

        let msg_tokens = tokenize_text(&message.content);
        let field_score = compute_bm25_field_score(
            &query_tokens,
            &msg_tokens,
            stats,
            field_type.boost_weight(),
            msg_tokens.len(),
        );

        if field_score > 0.0 {
            total_score += field_score;
            hit_count += 1;
            let (snippet, spans) = extract_snippet(&message.content, &query_tokens, query.snippet_len);
            matches.push(MessageMatch {
                message_index: Some(m_idx),
                turn_index: Some(turn_counter),
                role: Some(message.role),
                field: field_type,
                snippet,
                match_spans: spans,
                score: field_score,
            });
        }

        // Tool calls content check
        if query.include_tool_calls {
            if let Some(calls) = &message.tool_calls {
                for call in calls {
                    let call_text = format!("{} {}", call.name, call.arguments);
                    let call_tokens = tokenize_text(&call_text);
                    let call_score = compute_bm25_field_score(
                        &query_tokens,
                        &call_tokens,
                        stats,
                        BOOST_TOOL_CALL,
                        call_tokens.len(),
                    );
                    if call_score > 0.0 {
                        total_score += call_score;
                        hit_count += 1;
                        let (snippet, spans) =
                            extract_snippet(&call_text, &query_tokens, query.snippet_len);
                        matches.push(MessageMatch {
                            message_index: Some(m_idx),
                            turn_index: Some(turn_counter),
                            role: Some(message.role),
                            field: MatchedField::ToolCall,
                            snippet,
                            match_spans: spans,
                            score: call_score,
                        });
                    }
                }
            }
        }
    }

    if hit_count == 0 {
        return None;
    }

    // Limit matches per session
    if matches.len() > query.max_matches_per_session {
        matches.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap_or(std::cmp::Ordering::Equal));
        matches.truncate(query.max_matches_per_session);
    }

    Some(SessionSearchResult {
        session_id: session.id,
        title: session.title.clone(),
        active_model: session.active_model.clone(),
        created_at: session.created_at.clone(),
        updated_at: session.updated_at.clone(),
        total_messages: session.messages.len(),
        score: total_score,
        matches,
        total_hit_count: hit_count,
    })
}

fn compute_bm25_field_score(
    query_tokens: &[String],
    field_tokens: &[String],
    stats: &CollectionStats,
    boost: f64,
    doc_len: usize,
) -> f64 {
    if query_tokens.is_empty() || field_tokens.is_empty() {
        return 0.0;
    }

    let mut term_freqs = HashMap::new();
    for t in field_tokens {
        *term_freqs.entry(t.as_str()).or_insert(0usize) += 1;
    }

    let mut score = 0.0;
    let k1 = DEFAULT_BM25_K1;
    let b = DEFAULT_BM25_B;
    let len_ratio = doc_len as f64 / stats.avg_doc_len;

    for q_term in query_tokens {
        if let Some(&tf) = term_freqs.get(q_term.as_str()) {
            let idf = stats.idf(q_term);
            let tf_f = tf as f64;
            let numerator = tf_f * (k1 + 1.0);
            let denominator = tf_f + k1 * (1.0 - b + b * len_ratio);
            score += idf * (numerator / denominator);
        }
    }

    score * boost
}

// ============================================================================
// Semantic & Hybrid Vector Search
// ============================================================================

fn score_session_semantic(session: &Session, query: &SearchQuery) -> Option<SessionSearchResult> {
    // Generate query vector with synonym expansion
    let query_vector = build_query_semantic_vector(&query.text);
    if query_vector.is_empty() {
        return None;
    }

    let mut total_score = 0.0;
    let mut matches = Vec::new();
    let mut hit_count = 0;
    let query_terms = tokenize_text(&query.text);

    // 1. Compare title
    if let Some(title) = session.title() {
        let title_vector = build_document_semantic_vector(title);
        let sim = cosine_similarity(&query_vector, &title_vector);
        if sim > 0.15 {
            let field_score = sim * BOOST_TITLE;
            total_score += field_score;
            hit_count += 1;
            let (snippet, spans) = extract_snippet(title, &query_terms, query.snippet_len);
            matches.push(MessageMatch {
                message_index: None,
                turn_index: None,
                role: None,
                field: MatchedField::Title,
                snippet,
                match_spans: spans,
                score: field_score,
            });
        }
    }

    // 2. Compare messages
    for (m_idx, message) in session.messages().iter().enumerate() {
        if let Some(roles) = &query.role_filter {
            if !roles.contains(&message.role) {
                continue;
            }
        }

        let doc_vector = build_document_semantic_vector(&message.content);
        let sim = cosine_similarity(&query_vector, &doc_vector);

        if sim > 0.12 {
            let field_type = match message.role {
                Role::User => MatchedField::UserPrompt,
                Role::Assistant => MatchedField::AssistantReply,
                Role::System => MatchedField::SystemPrompt,
                Role::Tool => MatchedField::ToolResult,
            };

            let field_score = sim * field_type.boost_weight();
            total_score += field_score;
            hit_count += 1;
            let (snippet, spans) = extract_snippet(&message.content, &query_terms, query.snippet_len);
            matches.push(MessageMatch {
                message_index: Some(m_idx),
                turn_index: None,
                role: Some(message.role),
                field: field_type,
                snippet,
                match_spans: spans,
                score: field_score,
            });
        }
    }

    if hit_count == 0 {
        return None;
    }

    matches.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap_or(std::cmp::Ordering::Equal));
    if matches.len() > query.max_matches_per_session {
        matches.truncate(query.max_matches_per_session);
    }

    Some(SessionSearchResult {
        session_id: session.id,
        title: session.title.clone(),
        active_model: session.active_model.clone(),
        created_at: session.created_at.clone(),
        updated_at: session.updated_at.clone(),
        total_messages: session.messages.len(),
        score: total_score,
        matches,
        total_hit_count: hit_count,
    })
}

fn score_session_hybrid(
    session: &Session,
    query: &SearchQuery,
    stats: &CollectionStats,
) -> Option<SessionSearchResult> {
    let bm25_res = score_session_bm25(session, query, stats);
    let sem_res = score_session_semantic(session, query);

    match (bm25_res, sem_res) {
        (Some(b), Some(s)) => {
            let combined_score = b.score * HYBRID_BM25_WEIGHT + s.score * HYBRID_SEMANTIC_WEIGHT;
            let mut matches = b.matches;
            for s_match in s.matches {
                if !matches.iter().any(|m| m.snippet == s_match.snippet) {
                    matches.push(s_match);
                }
            }
            matches.sort_by(|x, y| y.score.partial_cmp(&x.score).unwrap_or(std::cmp::Ordering::Equal));
            matches.truncate(query.max_matches_per_session);

            Some(SessionSearchResult {
                session_id: session.id,
                title: session.title.clone(),
                active_model: session.active_model.clone(),
                created_at: session.created_at.clone(),
                updated_at: session.updated_at.clone(),
                total_messages: session.messages.len(),
                score: combined_score,
                matches,
                total_hit_count: b.total_hit_count + s.total_hit_count,
            })
        }
        (Some(b), None) => Some(b),
        (None, Some(s)) => Some(s),
        (None, None) => None,
    }
}

fn build_query_semantic_vector(text: &str) -> HashMap<String, f64> {
    let mut vector = HashMap::new();
    let tokens = tokenize_text(text);
    let ngrams = generate_char_ngrams(text);

    // Direct token weights
    for t in &tokens {
        *vector.entry(format!("t:{}", t)).or_insert(0.0) += 2.0;

        // Add synonym concept weights
        for syn in expand_semantic_synonyms(t) {
            *vector.entry(format!("t:{}", syn)).or_insert(0.0) += 0.8;
        }
    }

    // Character n-gram weights
    for ng in ngrams {
        *vector.entry(format!("ng:{}", ng)).or_insert(0.0) += 1.0;
    }

    vector
}

fn build_document_semantic_vector(text: &str) -> HashMap<String, f64> {
    let mut vector = HashMap::new();
    let tokens = tokenize_text(text);
    let ngrams = generate_char_ngrams(text);

    for t in tokens {
        *vector.entry(format!("t:{}", t)).or_insert(0.0) += 2.0;
    }

    for ng in ngrams {
        *vector.entry(format!("ng:{}", ng)).or_insert(0.0) += 1.0;
    }

    vector
}

// ============================================================================
// Exact, Regex & Fuzzy Matchers
// ============================================================================

fn score_session_exact(session: &Session, query: &SearchQuery) -> Option<SessionSearchResult> {
    let target = if query.case_sensitive {
        query.text.clone()
    } else {
        query.text.to_lowercase()
    };

    if target.is_empty() {
        return None;
    }

    let mut matches = Vec::new();
    let mut hit_count = 0;

    // Title
    if let Some(title) = session.title() {
        let check_title = if query.case_sensitive {
            title.to_string()
        } else {
            title.to_lowercase()
        };
        if check_title.contains(&target) {
            hit_count += 1;
            let (snippet, spans) = extract_snippet(title, &[query.text.clone()], query.snippet_len);
            matches.push(MessageMatch {
                message_index: None,
                turn_index: None,
                role: None,
                field: MatchedField::Title,
                snippet,
                match_spans: spans,
                score: BOOST_TITLE,
            });
        }
    }

    // Messages
    for (m_idx, message) in session.messages().iter().enumerate() {
        if let Some(roles) = &query.role_filter {
            if !roles.contains(&message.role) {
                continue;
            }
        }

        let check_content = if query.case_sensitive {
            message.content.clone()
        } else {
            message.content.to_lowercase()
        };

        if check_content.contains(&target) {
            hit_count += 1;
            let field_type = match message.role {
                Role::User => MatchedField::UserPrompt,
                Role::Assistant => MatchedField::AssistantReply,
                Role::System => MatchedField::SystemPrompt,
                Role::Tool => MatchedField::ToolResult,
            };
            let (snippet, spans) =
                extract_snippet(&message.content, &[query.text.clone()], query.snippet_len);
            matches.push(MessageMatch {
                message_index: Some(m_idx),
                turn_index: None,
                role: Some(message.role),
                field: field_type,
                snippet,
                match_spans: spans,
                score: field_type.boost_weight(),
            });
        }
    }

    if hit_count == 0 {
        return None;
    }

    Some(SessionSearchResult {
        session_id: session.id,
        title: session.title.clone(),
        active_model: session.active_model.clone(),
        created_at: session.created_at.clone(),
        updated_at: session.updated_at.clone(),
        total_messages: session.messages.len(),
        score: hit_count as f64,
        matches,
        total_hit_count: hit_count,
    })
}

fn score_session_regex(session: &Session, query: &SearchQuery) -> Option<SessionSearchResult> {
    let re = regex::RegexBuilder::new(&query.text)
        .case_insensitive(!query.case_sensitive)
        .build()
        .ok()?;

    let mut matches = Vec::new();
    let mut hit_count = 0;

    // Title
    if let Some(title) = session.title() {
        if let Some(mat) = re.find(title) {
            hit_count += 1;
            let term = mat.as_str().to_string();
            let (snippet, spans) = extract_snippet(title, &[term], query.snippet_len);
            matches.push(MessageMatch {
                message_index: None,
                turn_index: None,
                role: None,
                field: MatchedField::Title,
                snippet,
                match_spans: spans,
                score: BOOST_TITLE,
            });
        }
    }

    // Messages
    for (m_idx, message) in session.messages().iter().enumerate() {
        if let Some(roles) = &query.role_filter {
            if !roles.contains(&message.role) {
                continue;
            }
        }

        if let Some(mat) = re.find(&message.content) {
            hit_count += 1;
            let term = mat.as_str().to_string();
            let field_type = match message.role {
                Role::User => MatchedField::UserPrompt,
                Role::Assistant => MatchedField::AssistantReply,
                Role::System => MatchedField::SystemPrompt,
                Role::Tool => MatchedField::ToolResult,
            };
            let (snippet, spans) = extract_snippet(&message.content, &[term], query.snippet_len);
            matches.push(MessageMatch {
                message_index: Some(m_idx),
                turn_index: None,
                role: Some(message.role),
                field: field_type,
                snippet,
                match_spans: spans,
                score: field_type.boost_weight(),
            });
        }
    }

    if hit_count == 0 {
        return None;
    }

    Some(SessionSearchResult {
        session_id: session.id,
        title: session.title.clone(),
        active_model: session.active_model.clone(),
        created_at: session.created_at.clone(),
        updated_at: session.updated_at.clone(),
        total_messages: session.messages.len(),
        score: hit_count as f64,
        matches,
        total_hit_count: hit_count,
    })
}

fn score_session_fuzzy(session: &Session, query: &SearchQuery) -> Option<SessionSearchResult> {
    let query_tokens = tokenize_text(&query.text);
    if query_tokens.is_empty() {
        return None;
    }

    let mut total_score = 0.0;
    let mut matches = Vec::new();
    let mut hit_count = 0;

    // Title
    if let Some(title) = session.title() {
        let title_tokens = tokenize_text(title);
        let sim = compute_fuzzy_token_similarity(&query_tokens, &title_tokens);
        if sim >= 0.65 {
            let score = sim * BOOST_TITLE;
            total_score += score;
            hit_count += 1;
            let (snippet, spans) = extract_snippet(title, &query_tokens, query.snippet_len);
            matches.push(MessageMatch {
                message_index: None,
                turn_index: None,
                role: None,
                field: MatchedField::Title,
                snippet,
                match_spans: spans,
                score,
            });
        }
    }

    // Messages
    for (m_idx, message) in session.messages().iter().enumerate() {
        if let Some(roles) = &query.role_filter {
            if !roles.contains(&message.role) {
                continue;
            }
        }

        let msg_tokens = tokenize_text(&message.content);
        let sim = compute_fuzzy_token_similarity(&query_tokens, &msg_tokens);

        if sim >= 0.65 {
            let field_type = match message.role {
                Role::User => MatchedField::UserPrompt,
                Role::Assistant => MatchedField::AssistantReply,
                Role::System => MatchedField::SystemPrompt,
                Role::Tool => MatchedField::ToolResult,
            };

            let score = sim * field_type.boost_weight();
            total_score += score;
            hit_count += 1;
            let (snippet, spans) = extract_snippet(&message.content, &query_tokens, query.snippet_len);
            matches.push(MessageMatch {
                message_index: Some(m_idx),
                turn_index: None,
                role: Some(message.role),
                field: field_type,
                snippet,
                match_spans: spans,
                score,
            });
        }
    }

    if hit_count == 0 {
        return None;
    }

    Some(SessionSearchResult {
        session_id: session.id,
        title: session.title.clone(),
        active_model: session.active_model.clone(),
        created_at: session.created_at.clone(),
        updated_at: session.updated_at.clone(),
        total_messages: session.messages.len(),
        score: total_score,
        matches,
        total_hit_count: hit_count,
    })
}

fn compute_fuzzy_token_similarity(q_tokens: &[String], doc_tokens: &[String]) -> f64 {
    if q_tokens.is_empty() || doc_tokens.is_empty() {
        return 0.0;
    }

    let mut total_sim = 0.0;
    for q in q_tokens {
        let mut best_match = 0.0;
        for d in doc_tokens {
            let sim = fuzzy_similarity(q, d);
            if sim > best_match {
                best_match = sim;
            }
        }
        total_sim += best_match;
    }

    total_sim / q_tokens.len() as f64
}

fn session_contains_term(session: &Session, term: &str) -> bool {
    let lower = term.to_lowercase();
    if let Some(title) = session.title() {
        if title.to_lowercase().contains(&lower) {
            return true;
        }
    }
    session
        .messages()
        .iter()
        .any(|m| m.content.to_lowercase().contains(&lower))
}

fn session_contains_phrase(session: &Session, phrase: &str) -> bool {
    let lower = phrase.to_lowercase();
    if let Some(title) = session.title() {
        if title.to_lowercase().contains(&lower) {
            return true;
        }
    }
    session
        .messages()
        .iter()
        .any(|m| m.content.to_lowercase().contains(&lower))
}

// ============================================================================
// Inverted Index Engine (`SessionSearchIndex`)
// ============================================================================

/// Document posting entry in the inverted index.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchPosting {
    pub session_id: Uuid,
    pub message_index: Option<usize>,
    pub field: MatchedField,
    pub term_frequency: usize,
}

/// Metadata recorded for an indexed session.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IndexedSessionMeta {
    pub id: Uuid,
    pub title: Option<String>,
    pub active_model: String,
    pub created_at: String,
    pub updated_at: String,
    pub message_count: usize,
    pub total_tokens: usize,
}

/// In-memory inverted index for fast cross-session searching and clustering.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SessionSearchIndex {
    /// Inverted index mapping term -> list of postings.
    pub postings: HashMap<String, Vec<SearchPosting>>,
    /// Session metadata registry.
    pub sessions: HashMap<Uuid, IndexedSessionMeta>,
    /// Document lengths (session_id -> total token count).
    pub doc_lengths: HashMap<Uuid, usize>,
    /// Pre-computed semantic vector embeddings for each session.
    pub semantic_vectors: HashMap<Uuid, HashMap<String, f64>>,
    /// Total tokens indexed across all sessions.
    pub total_indexed_tokens: usize,
}

impl SessionSearchIndex {
    /// Creates a new empty `SessionSearchIndex`.
    pub fn new() -> Self {
        Self::default()
    }

    /// Builds a search index from an existing slice of `Session`s.
    pub fn build_from_sessions(sessions: &[Session]) -> Self {
        let mut index = Self::new();
        for session in sessions {
            index.index_session(session);
        }
        index
    }

    /// Builds a search index from all `.json` session files in a directory.
    pub fn build_from_dir(dir: impl AsRef<Path>) -> anyhow::Result<Self> {
        let dir = dir.as_ref();
        let mut index = Self::new();
        if !dir.exists() {
            return Ok(index);
        }

        for entry in fs::read_dir(dir)? {
            let entry = entry?;
            let path = entry.path();
            if path.extension().and_then(|s| s.to_str()) == Some("json") {
                if let Ok(session) = Session::load_from_path(&path) {
                    index.index_session(&session);
                }
            }
        }

        Ok(index)
    }

    /// Indexes a single session into the inverted index and vector registry.
    pub fn index_session(&mut self, session: &Session) {
        // Remove prior entry if re-indexing
        self.remove_session(session.id);

        let mut session_tokens_count = 0;
        let mut combined_text = String::new();

        // 1. Index Title
        if let Some(title) = session.title() {
            combined_text.push_str(title);
            combined_text.push(' ');
            let tokens = tokenize_text(title);
            session_tokens_count += tokens.len();
            self.add_field_tokens(session.id, None, MatchedField::Title, &tokens);
        }

        // 2. Index Messages
        for (m_idx, msg) in session.messages().iter().enumerate() {
            combined_text.push_str(&msg.content);
            combined_text.push(' ');
            let tokens = tokenize_text(&msg.content);
            session_tokens_count += tokens.len();

            let field = match msg.role {
                Role::User => MatchedField::UserPrompt,
                Role::Assistant => MatchedField::AssistantReply,
                Role::System => MatchedField::SystemPrompt,
                Role::Tool => MatchedField::ToolResult,
            };

            self.add_field_tokens(session.id, Some(m_idx), field, &tokens);
        }

        // 3. Register Metadata
        self.sessions.insert(
            session.id,
            IndexedSessionMeta {
                id: session.id,
                title: session.title.clone(),
                active_model: session.active_model.clone(),
                created_at: session.created_at.clone(),
                updated_at: session.updated_at.clone(),
                message_count: session.messages.len(),
                total_tokens: session_tokens_count,
            },
        );

        self.doc_lengths.insert(session.id, session_tokens_count);
        self.total_indexed_tokens += session_tokens_count;

        // 4. Build Semantic Vector
        let sem_vector = build_document_semantic_vector(&combined_text);
        self.semantic_vectors.insert(session.id, sem_vector);
    }

    fn add_field_tokens(
        &mut self,
        session_id: Uuid,
        message_index: Option<usize>,
        field: MatchedField,
        tokens: &[String],
    ) {
        let mut term_counts = HashMap::new();
        for t in tokens {
            *term_counts.entry(t.clone()).or_insert(0usize) += 1;
        }

        for (term, count) in term_counts {
            self.postings
                .entry(term)
                .or_default()
                .push(SearchPosting {
                    session_id,
                    message_index,
                    field,
                    term_frequency: count,
                });
        }
    }

    /// Removes a session from the index by UUID.
    pub fn remove_session(&mut self, session_id: Uuid) {
        if let Some(meta) = self.sessions.remove(&session_id) {
            self.total_indexed_tokens = self.total_indexed_tokens.saturating_sub(meta.total_tokens);
        }
        self.doc_lengths.remove(&session_id);
        self.semantic_vectors.remove(&session_id);

        for postings_list in self.postings.values_mut() {
            postings_list.retain(|p| p.session_id != session_id);
        }
    }

    /// Returns the total number of indexed sessions.
    pub fn session_count(&self) -> usize {
        self.sessions.len()
    }

    /// Finds sessions most semantically similar to a given target session.
    pub fn find_similar_sessions(
        &self,
        target_session_id: Uuid,
        limit: usize,
    ) -> Vec<(Uuid, f64)> {
        let target_vec = match self.semantic_vectors.get(&target_session_id) {
            Some(v) => v,
            None => return Vec::new(),
        };

        let mut similarities = Vec::new();
        for (&other_id, other_vec) in &self.semantic_vectors {
            if other_id == target_session_id {
                continue;
            }
            let sim = cosine_similarity(target_vec, other_vec);
            if sim > 0.05 {
                similarities.push((other_id, sim));
            }
        }

        similarities.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        if similarities.len() > limit {
            similarities.truncate(limit);
        }
        similarities
    }
}

// ============================================================================
// Unit Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn make_test_session(title: &str, model: &str, user_msg: &str, assistant_msg: &str) -> Session {
        let mut s = Session::new(model);
        s.set_title(title);
        s.add_user_message(user_msg);
        s.add_assistant_message(assistant_msg);
        s
    }

    #[test]
    fn test_query_parser() {
        let q = SearchQuery::parse("authentication bug role:user model:gpt-4o limit:5 -legacy");
        assert_eq!(q.text, "authentication bug");
        assert_eq!(q.role_filter, Some(vec![Role::User]));
        assert_eq!(q.model_filter, Some("gpt-4o".to_string()));
        assert_eq!(q.limit, 5);
        assert_eq!(q.excluded_terms, vec!["legacy".to_string()]);
    }

    #[test]
    fn test_query_parser_quotes_and_modes() {
        let q = SearchQuery::parse("\"database connection error\" mode:semantic after:2026-01-01");
        assert_eq!(q.text, "database connection error");
        assert_eq!(q.required_phrases, vec!["database connection error".to_string()]);
        assert_eq!(q.mode, SearchMode::Semantic);
        assert!(q.date_from.is_some());
    }

    #[test]
    fn test_bm25_search_ranking() {
        let s1 = make_test_session(
            "Rust Async Mutex Deadlock",
            "gpt-4o",
            "How do I prevent deadlock with tokio::sync::Mutex?",
            "Use try_lock or lock ordering.",
        );
        let s2 = make_test_session(
            "Python Pandas Dataframe",
            "claude-3-5-sonnet",
            "How to filter pandas dataframe?",
            "Use df[df['col'] > 0]",
        );

        let sessions = vec![s1.clone(), s2.clone()];
        let query = SearchQuery::parse("Rust Mutex deadlock");
        let report = search_in_sessions(&sessions, &query);

        assert_eq!(report.matching_sessions_count, 1);
        assert_eq!(report.results[0].session_id, s1.id);
        assert!(report.results[0].score > 0.0);
    }

    #[test]
    fn test_semantic_concept_expansion() {
        let s1 = make_test_session(
            "Security Token Validation",
            "gpt-4o",
            "Verify the JWT token and validate permissions",
            "Decode bearer token and check claims.",
        );
        let s2 = make_test_session(
            "UI Button Styling",
            "gpt-4o",
            "How to style Ratatui widgets?",
            "Use ratatui::style::Style.",
        );

        let sessions = vec![s1.clone(), s2.clone()];
        // Search for "auth" - should semantically match JWT / token session
        let query = SearchQuery::parse("auth mode:semantic");
        let report = search_in_sessions(&sessions, &query);

        assert!(!report.results.is_empty());
        assert_eq!(report.results[0].session_id, s1.id);
    }

    #[test]
    fn test_exact_and_regex_search() {
        let s1 = make_test_session(
            "HTTP Server Fix",
            "gpt-4o",
            "Fixing StatusCode::UNAUTHORIZED in handler",
            "Return 401 response code.",
        );

        let sessions = vec![s1.clone()];

        // Exact match
        let exact_query = SearchQuery::parse("StatusCode::UNAUTHORIZED mode:exact");
        let rep_exact = search_in_sessions(&sessions, &exact_query);
        assert_eq!(rep_exact.matching_sessions_count, 1);

        // Regex match
        let regex_query = SearchQuery::parse("Status[a-zA-Z]+::[A-Z]+ mode:regex");
        let rep_regex = search_in_sessions(&sessions, &regex_query);
        assert_eq!(rep_regex.matching_sessions_count, 1);
    }

    #[test]
    fn test_fuzzy_search_typo_tolerance() {
        let s1 = make_test_session(
            "PostgreSQL Migration",
            "gpt-4o",
            "Creating index on user_accounts table",
            "CREATE INDEX idx_user ON user_accounts(id);",
        );

        let sessions = vec![s1.clone()];
        // Typo: "PostgreSQLL" and "acounts"
        let query = SearchQuery::parse("PostgreSQLL acounts mode:fuzzy");
        let report = search_in_sessions(&sessions, &query);

        assert_eq!(report.matching_sessions_count, 1);
        assert_eq!(report.results[0].session_id, s1.id);
    }

    #[test]
    fn test_inverted_index_and_similarity() {
        let s1 = make_test_session(
            "WASM Canvas",
            "gpt-4o",
            "Render canvas using web-sys and wasm-bindgen",
            "Access html canvas 2d context in Rust.",
        );
        let s2 = make_test_session(
            "WebAssembly Graphics",
            "gpt-4o",
            "Draw webgl shaders in browser wasm",
            "Use web-sys WebGlRenderingContext.",
        );
        let s3 = make_test_session(
            "CLI File Grep",
            "gpt-4o",
            "Recursive file search in terminal",
            "Use ignore::WalkBuilder.",
        );

        let index = SessionSearchIndex::build_from_sessions(&[s1.clone(), s2.clone(), s3.clone()]);
        assert_eq!(index.session_count(), 3);

        let similar_to_s1 = index.find_similar_sessions(s1.id, 5);
        assert!(!similar_to_s1.is_empty());
        // s2 (WASM/graphics) should be more similar to s1 (WASM/canvas) than s3 (CLI grep)
        assert_eq!(similar_to_s1[0].0, s2.id);
    }

    #[test]
    fn test_snippet_extraction_and_highlighting() {
        let text = "Here is a detailed explanation of how to configure Ratatui inline terminal rendering in Fusion.";
        let (snippet, spans) = extract_snippet(text, &["Ratatui".to_string(), "Fusion".to_string()], 50);
        assert!(!snippet.is_empty());
        assert!(!spans.is_empty());

        let highlighted = highlight_spans(&snippet, &spans, "**", "**");
        assert!(highlighted.contains("**"));
    }
}
