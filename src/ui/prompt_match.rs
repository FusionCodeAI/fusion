//! Prompt history fuzzy search and autocompletion matching previous prompts as you type.
//!
//! Provides a high-performance, pure-Rust prompt matching and completion engine for the interactive UI:
//! - **Inline Ghost Autocompletion**: Real-time fish-shell / zsh style dimmed ghost suggestions
//!   as the user types in the prompt buffer, with full-line (`Tab` / `Right`) or word-by-word (`Alt+Right`) acceptance.
//! - **Typo-Tolerant Fuzzy History Search**: Subsequence alignment, camelCase/snake_case boundary bonuses,
//!   exact prefix bonuses, consecutive match bonuses, and multi-word token overlap scoring.
//! - **Frecency Ranking**: Combines frequency (usage count) and exponential recency decay
//!   to prioritize frequently used and recently entered prompts.
//! - **Interactive Completion Menu & Popup**: Keyboard-navigable popup menu displaying
//!   ranked candidate prompts with ANSI matched-character highlighting and category badges.
//! - **Reverse History Search (Ctrl+R)**: Interactive reverse search through full prompt history
//!   with live filtering and selection.
//! - **Built-in Assistant Templates & Slash Presets**: Curated coding assistant prompts
//!   (e.g. `/review`, `/test`, `/refactor`, `/fix`, `/explain`, `/doc`) integrated directly into completions.

use serde::{Deserialize, Serialize};
use std::cmp::Ordering;
use std::collections::{HashMap, HashSet};
use std::fmt;

// ---------------------------------------------------------------------------
// Prompt Categories & Metadata
// ---------------------------------------------------------------------------

/// Category classification for prompt history entries and templates.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PromptCategory {
    /// General free-form user message or instruction.
    General,
    /// Slash command invocation (e.g. `/model`, `/file`, `/skills`).
    Command,
    /// Code snippet or coding query.
    Code,
    /// Predefined or user-saved prompt snippet / template.
    Snippet,
    /// Code review request (e.g. `/review`, "Review these changes").
    Review,
    /// Unit test generation request (e.g. `/test`, "Write unit tests for").
    Test,
    /// Code refactoring request (e.g. `/refactor`).
    Refactor,
    /// Bug debugging / troubleshooting request (e.g. `/fix`).
    Debug,
    /// Documentation generation request (e.g. `/doc`).
    Doc,
    /// Custom user-defined category.
    Custom(String),
}

impl Default for PromptCategory {
    fn default() -> Self {
        Self::General
    }
}

impl fmt::Display for PromptCategory {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::General => write!(f, "General"),
            Self::Command => write!(f, "Command"),
            Self::Code => write!(f, "Code"),
            Self::Snippet => write!(f, "Snippet"),
            Self::Review => write!(f, "Review"),
            Self::Test => write!(f, "Test"),
            Self::Refactor => write!(f, "Refactor"),
            Self::Debug => write!(f, "Debug"),
            Self::Doc => write!(f, "Doc"),
            Self::Custom(name) => write!(f, "{}", name),
        }
    }
}

impl PromptCategory {
    /// Detect category automatically from prompt text.
    pub fn detect_from_text(text: &str) -> Self {
        let trimmed = text.trim();
        if trimmed.starts_with('/') {
            let cmd = trimmed.split_whitespace().next().unwrap_or(trimmed);
            match cmd {
                "/review" => Self::Review,
                "/test" => Self::Test,
                "/refactor" => Self::Refactor,
                "/fix" | "/debug" => Self::Debug,
                "/doc" | "/docs" => Self::Doc,
                _ => Self::Command,
            }
        } else if trimmed.starts_with("```") || trimmed.contains("fn ") || trimmed.contains("def ") || trimmed.contains("pub struct ") {
            Self::Code
        } else if trimmed.to_lowercase().starts_with("review ") || trimmed.to_lowercase().contains("code review") {
            Self::Review
        } else if trimmed.to_lowercase().starts_with("write test") || trimmed.to_lowercase().starts_with("test ") {
            Self::Test
        } else if trimmed.to_lowercase().starts_with("refactor ") {
            Self::Refactor
        } else if trimmed.to_lowercase().starts_with("fix ") || trimmed.to_lowercase().starts_with("debug ") {
            Self::Debug
        } else if trimmed.to_lowercase().starts_with("document ") || trimmed.to_lowercase().starts_with("explain ") {
            Self::Doc
        } else {
            Self::General
        }
    }

    /// Short icon or glyph representing the category for UI rendering.
    pub fn icon(&self) -> &'static str {
        match self {
            Self::General => "💬",
            Self::Command => "⚡",
            Self::Code => "💻",
            Self::Snippet => "📋",
            Self::Review => "🔍",
            Self::Test => "🧪",
            Self::Refactor => "🛠️",
            Self::Debug => "🐛",
            Self::Doc => "📝",
            Self::Custom(_) => "🏷️",
        }
    }

    /// Color style badge for category indicator.
    pub fn badge_ansi(&self) -> &'static str {
        match self {
            Self::General => "\x1b[38;5;246m[Gen]\x1b[0m",
            Self::Command => "\x1b[38;5;39m[Cmd]\x1b[0m",
            Self::Code => "\x1b[38;5;75m[Code]\x1b[0m",
            Self::Snippet => "\x1b[38;5;141m[Snip]\x1b[0m",
            Self::Review => "\x1b[38;5;214m[Rev]\x1b[0m",
            Self::Test => "\x1b[38;5;42m[Test]\x1b[0m",
            Self::Refactor => "\x1b[38;5;178m[Ref]\x1b[0m",
            Self::Debug => "\x1b[38;5;203m[Fix]\x1b[0m",
            Self::Doc => "\x1b[38;5;117m[Doc]\x1b[0m",
            Self::Custom(_) => "\x1b[38;5;250m[Tag]\x1b[0m",
        }
    }
}

// ---------------------------------------------------------------------------
// Prompt History Entry Item
// ---------------------------------------------------------------------------

/// A recorded prompt entry in history with metadata for ranking and filtering.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PromptHistoryItem {
    /// Full prompt text.
    pub text: String,
    /// Initial creation timestamp (Unix epoch seconds).
    pub timestamp: i64,
    /// Number of times this prompt has been submitted or chosen.
    pub use_count: u32,
    /// Last used timestamp (Unix epoch seconds).
    pub last_used: i64,
    /// Classification category.
    pub category: PromptCategory,
    /// Optional user or system tags.
    pub tags: Vec<String>,
    /// Optional additional metadata (e.g. model used, session id).
    pub metadata: HashMap<String, String>,
}

impl PromptHistoryItem {
    /// Create a new history entry with current timestamp and default values.
    pub fn new(text: impl Into<String>) -> Self {
        let text_str = text.into();
        let now = current_epoch_secs();
        let category = PromptCategory::detect_from_text(&text_str);
        Self {
            text: text_str,
            timestamp: now,
            use_count: 1,
            last_used: now,
            category,
            tags: Vec::new(),
            metadata: HashMap::new(),
        }
    }

    /// Builder: Set custom category.
    pub fn with_category(mut self, category: PromptCategory) -> Self {
        self.category = category;
        self
    }

    /// Builder: Set initial usage count.
    pub fn with_use_count(mut self, count: u32) -> Self {
        self.use_count = count;
        self
    }

    /// Builder: Set custom timestamp.
    pub fn with_timestamp(mut self, timestamp: i64) -> Self {
        self.timestamp = timestamp;
        self.last_used = timestamp;
        self
    }

    /// Builder: Attach a tag.
    pub fn with_tag(mut self, tag: impl Into<String>) -> Self {
        self.tags.push(tag.into());
        self
    }

    /// Builder: Attach key-value metadata.
    pub fn with_metadata(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.metadata.insert(key.into(), value.into());
        self
    }

    /// Record an additional use of this item (increments count and refreshes `last_used`).
    pub fn record_use(&mut self) {
        self.use_count = self.use_count.saturating_add(1);
        self.last_used = current_epoch_secs();
    }
}

impl From<&str> for PromptHistoryItem {
    fn from(s: &str) -> Self {
        Self::new(s)
    }
}

impl From<String> for PromptHistoryItem {
    fn from(s: String) -> Self {
        Self::new(s)
    }
}

// ---------------------------------------------------------------------------
// Match Result & Match Kind
// ---------------------------------------------------------------------------

/// Type of match determined during scoring.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum MatchKind {
    /// Exact match (buffer identical to history item).
    Exact,
    /// Exact prefix match (buffer is a strict prefix of candidate).
    Prefix,
    /// High-confidence ghost autocompletion suggestion.
    GhostSuggestion,
    /// Fuzzy subsequence match with word boundary alignment.
    Subsequence,
    /// Token-level overlap match (words match regardless of order).
    TokenOverlap,
    /// Typo-tolerant edit-distance match.
    Fuzzy,
    /// Predefined template or snippet.
    SnippetTemplate,
}

impl MatchKind {
    /// Base score priority bonus for sorting.
    pub fn base_priority(&self) -> i64 {
        match self {
            Self::Exact => 1000,
            Self::Prefix => 800,
            Self::GhostSuggestion => 750,
            Self::SnippetTemplate => 600,
            Self::Subsequence => 400,
            Self::TokenOverlap => 300,
            Self::Fuzzy => 150,
        }
    }
}

/// Raw result of fuzzy matching a pattern against a candidate text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FuzzyMatchResult {
    /// Match quality score (higher is better).
    pub score: i64,
    /// 0-based character indices in candidate that matched pattern characters.
    pub matched_indices: Vec<usize>,
    /// Whether the pattern is an exact match.
    pub is_exact: bool,
    /// Whether the candidate starts with the pattern (case-insensitive).
    pub is_prefix: bool,
    /// Classification of the match.
    pub match_kind: MatchKind,
}

/// A ranked match candidate with full context, scores, and ghost text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PromptMatch {
    /// Matched history item.
    pub item: PromptHistoryItem,
    /// Calculated composite score (fuzzy match score + frecency bonus + category priority).
    pub score: i64,
    /// 0-based character indices in candidate string that matched.
    pub matched_indices: Vec<usize>,
    /// Match classification.
    pub match_kind: MatchKind,
    /// Remaining suffix text to display as inline ghost completion, if candidate starts with prefix.
    pub ghost_suffix: Option<String>,
    /// Pre-rendered ANSI string with matched characters highlighted.
    pub highlighted_text: Option<String>,
}

impl Ord for PromptMatch {
    fn cmp(&self, other: &Self) -> Ordering {
        // Higher scores sort first
        self.score.cmp(&other.score).reverse()
    }
}

impl PartialOrd for PromptMatch {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

// ---------------------------------------------------------------------------
// Ghost Completion Representation
// ---------------------------------------------------------------------------

/// Ghost autocompletion result showing inline ghost suggestion text after the cursor.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GhostCompletion {
    /// Full matched candidate prompt.
    pub full_text: String,
    /// Current prefix typed by user.
    pub prefix: String,
    /// Dimmed ghost suffix to display after cursor.
    pub ghost_suffix: String,
    /// Next single word token of the ghost suffix (for `Alt+Right` word-by-word accept).
    pub next_word: String,
    /// Composite match score.
    pub score: i64,
    /// Match kind.
    pub match_kind: MatchKind,
}

impl GhostCompletion {
    /// Construct a new GhostCompletion from prefix and candidate.
    pub fn new(prefix: &str, candidate: &str, score: i64, match_kind: MatchKind) -> Option<Self> {
        let prefix_chars: Vec<char> = prefix.chars().collect();
        let cand_chars: Vec<char> = candidate.chars().collect();

        if prefix_chars.len() >= cand_chars.len() {
            return None;
        }

        // Case-insensitive prefix verification
        let prefix_lower: String = prefix.to_lowercase();
        let cand_lower: String = candidate.to_lowercase();
        if !cand_lower.starts_with(&prefix_lower) {
            return None;
        }

        let ghost_suffix: String = cand_chars[prefix_chars.len()..].iter().collect();
        if ghost_suffix.is_empty() {
            return None;
        }

        let next_word = extract_next_word_token(&ghost_suffix);

        Some(Self {
            full_text: candidate.to_string(),
            prefix: prefix.to_string(),
            ghost_suffix,
            next_word,
            score,
            match_kind,
        })
    }

    /// Return the full text when accepted in whole.
    pub fn accept_all(&self) -> &str {
        &self.full_text
    }

    /// Return the updated buffer text when accepting the next word only.
    pub fn accept_next_word(&self) -> String {
        format!("{}{}", self.prefix, self.next_word)
    }

    /// Format full prompt line with dimmed ANSI ghost text.
    pub fn render_inline_ansi(&self) -> String {
        format!("{}\x1b[2;37m{}\x1b[0m", self.prefix, self.ghost_suffix)
    }
}

// ---------------------------------------------------------------------------
// Configuration
// ---------------------------------------------------------------------------

/// Configuration options for prompt history matching and autocompletion.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PromptMatchConfig {
    /// Minimum prefix character length required to trigger ghost completions (default: 1).
    pub min_prefix_len: usize,
    /// Maximum number of suggestions to return in search / menu (default: 8).
    pub max_suggestions: usize,
    /// Half-life in seconds for exponential recency decay in frecency calculation (default: 86400 = 1 day).
    pub frecency_half_life_secs: f64,
    /// Weight multiplier for frecency bonus (default: 1.0).
    pub frecency_weight: f64,
    /// Bonus score for exact prefix matches (default: 120).
    pub exact_prefix_bonus: i64,
    /// Bonus score for word boundary matches (default: 45).
    pub word_boundary_bonus: i64,
    /// Bonus score for consecutive matched characters (default: 30).
    pub consecutive_bonus: i64,
    /// Bonus score for matching case (default: 10).
    pub case_bonus: i64,
    /// Minimum score threshold for returning a match (default: 10).
    pub min_score_threshold: i64,
    /// Whether matching should be case-sensitive (default: false).
    pub case_sensitive: bool,
    /// Enable multi-word token overlap matching (default: true).
    pub enable_token_overlap: bool,
    /// Enable subsequence fuzzy matching (default: true).
    pub enable_subsequence: bool,
    /// Enable frecency boost (default: true).
    pub enable_frecency: bool,
    /// Deduplicate results having identical prompt text (default: true).
    pub deduplicate_by_text: bool,
}

impl Default for PromptMatchConfig {
    fn default() -> Self {
        Self {
            min_prefix_len: 1,
            max_suggestions: 8,
            frecency_half_life_secs: 86400.0, // 24 hours
            frecency_weight: 1.0,
            exact_prefix_bonus: 120,
            word_boundary_bonus: 45,
            consecutive_bonus: 30,
            case_bonus: 10,
            min_score_threshold: 10,
            case_sensitive: false,
            enable_token_overlap: true,
            enable_subsequence: true,
            enable_frecency: true,
            deduplicate_by_text: true,
        }
    }
}

// ---------------------------------------------------------------------------
// Pure-Rust Fuzzy Matching Core
// ---------------------------------------------------------------------------

/// Computes a fuzzy match score, matched indices, and match kind between pattern and candidate.
///
/// Features:
/// - Case-insensitive match with case-preservation bonuses.
/// - Exact prefix bonus (critical for ghost autocompletion).
/// - Word boundary bonuses (after space, `/`, `_`, `-`, `.`, `:`, `(`, `[`, `{`).
/// - CamelCase transition bonuses.
/// - Consecutive character streak bonuses.
/// - Start position penalties and candidate length penalties.
pub fn fuzzy_match(pattern: &str, candidate: &str) -> Option<FuzzyMatchResult> {
    fuzzy_match_with_config(pattern, candidate, &PromptMatchConfig::default())
}

/// Computes a fuzzy match score with custom configuration parameters.
pub fn fuzzy_match_with_config(
    pattern: &str,
    candidate: &str,
    config: &PromptMatchConfig,
) -> Option<FuzzyMatchResult> {
    let clean_pattern = if config.case_sensitive {
        pattern.trim().to_string()
    } else {
        pattern.trim().to_string()
    };

    if clean_pattern.is_empty() {
        return Some(FuzzyMatchResult {
            score: 0,
            matched_indices: Vec::new(),
            is_exact: candidate.is_empty(),
            is_prefix: true,
            match_kind: MatchKind::Exact,
        });
    }

    let pattern_chars: Vec<char> = clean_pattern.chars().collect();
    let candidate_chars: Vec<char> = candidate.chars().collect();

    if pattern_chars.len() > candidate_chars.len() {
        return None;
    }

    let pattern_lower: Vec<char> = clean_pattern.to_lowercase().chars().collect();
    let candidate_lower: Vec<char> = candidate.to_lowercase().chars().collect();

    // 1. Exact match check
    if pattern_lower == candidate_lower {
        let indices: Vec<usize> = (0..candidate_chars.len()).collect();
        return Some(FuzzyMatchResult {
            score: 500 + (candidate_chars.len() as i64 * 5),
            matched_indices: indices,
            is_exact: true,
            is_prefix: true,
            match_kind: MatchKind::Exact,
        });
    }

    // 2. Exact prefix check (Ghost Autocompletion Fast-Path)
    let is_prefix = candidate_lower.starts_with(&pattern_lower);
    if is_prefix {
        let indices: Vec<usize> = (0..pattern_chars.len()).collect();
        let mut score: i64 = 300 + config.exact_prefix_bonus + (pattern_chars.len() as i64 * 15);
        // Bonus if first char matches case
        if pattern_chars.first() == candidate_chars.first() {
            score += config.case_bonus;
        }
        // Small length penalty to favor shorter candidates when prefix is identical
        let diff = candidate_chars.len().saturating_sub(pattern_chars.len());
        score -= (diff as i64) / 4;

        return Some(FuzzyMatchResult {
            score,
            matched_indices: indices,
            is_exact: false,
            is_prefix: true,
            match_kind: MatchKind::Prefix,
        });
    }

    if !config.enable_subsequence {
        return None;
    }

    // 3. Fast-path subsequence verification
    let mut p_iter = pattern_lower.iter();
    let mut curr_p = p_iter.next();
    for c in &candidate_lower {
        if let Some(target) = curr_p {
            if c == target {
                curr_p = p_iter.next();
            }
        } else {
            break;
        }
    }
    if curr_p.is_some() {
        // Pattern chars are not a subsequence in order
        return None;
    }

    // 4. Dynamic scoring alignment with boundary and streak bonuses
    let mut matched_indices = Vec::with_capacity(pattern_chars.len());
    let mut score: i64 = 0;
    let mut cand_idx = 0;

    // Substring bonus if candidate contains pattern as a continuous substring
    let cand_str_lower: String = candidate_lower.iter().collect();
    let patt_str_lower: String = pattern_lower.iter().collect();
    if let Some(sub_pos) = cand_str_lower.find(&patt_str_lower) {
        score += 70;
        if sub_pos == 0 {
            score += config.exact_prefix_bonus;
        }
    }

    for (pi, &p_char) in pattern_chars.iter().enumerate() {
        let p_lower = pattern_lower[pi];
        let mut best_idx = None;
        let mut best_local_score = i64::MIN;

        while cand_idx < candidate_chars.len() {
            let c = candidate_chars[cand_idx];
            let c_lower = candidate_lower[cand_idx];

            if c_lower == p_lower {
                let mut char_score: i64 = 12;

                // Consecutive match streak bonus
                if let Some(&last_idx) = matched_indices.last() {
                    if cand_idx == last_idx + 1 {
                        char_score += config.consecutive_bonus;
                    }
                }

                // Word boundary bonus
                let is_boundary = if cand_idx == 0 {
                    true
                } else {
                    let prev = candidate_chars[cand_idx - 1];
                    is_word_boundary_char(prev)
                };

                if is_boundary {
                    char_score += config.word_boundary_bonus;
                }

                // CamelCase transition bonus
                if cand_idx > 0 {
                    let prev = candidate_chars[cand_idx - 1];
                    if prev.is_ascii_lowercase() && c.is_ascii_uppercase() {
                        char_score += config.word_boundary_bonus - 5;
                    }
                }

                // Case match bonus
                if p_char == c {
                    char_score += config.case_bonus;
                }

                // Earlier positions score higher
                char_score -= (cand_idx as i64) / 4;

                if char_score > best_local_score {
                    best_local_score = char_score;
                    best_idx = Some(cand_idx);
                }

                if is_boundary {
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
            return None;
        }
    }

    // Candidate length penalty
    let len_diff = candidate_chars.len().saturating_sub(pattern_chars.len());
    score -= (len_diff as i64) / 3;

    Some(FuzzyMatchResult {
        score: score.max(1),
        matched_indices,
        is_exact: false,
        is_prefix: false,
        match_kind: MatchKind::Subsequence,
    })
}

/// Check if a character represents a word boundary separator.
#[inline]
pub fn is_word_boundary_char(c: char) -> bool {
    matches!(
        c,
        ' ' | '\t' | '\n' | '/' | '\\' | '_' | '-' | '.' | ':' | ';' | ',' | '(' | ')' | '[' | ']' | '{' | '}' | '<' | '>' | '|' | '!' | '?' | '=' | '+' | '*' | '&' | '%' | '$' | '#' | '@' | '"' | '\'' | '`'
    )
}

// ---------------------------------------------------------------------------
// Multi-Word Token Overlap Matching
// ---------------------------------------------------------------------------

/// Computes multi-word token overlap score for prompts with words in arbitrary order.
pub fn match_token_overlap(pattern: &str, candidate: &str) -> Option<PromptMatch> {
    let q_tokens: Vec<&str> = pattern.split_whitespace().collect();
    if q_tokens.len() <= 1 {
        return None;
    }

    let cand_lower = candidate.to_lowercase();
    let mut total_score: i64 = 100;
    let mut matched_indices = Vec::new();
    let mut matched_tokens = 0;

    for token in &q_tokens {
        let t_lower = token.to_lowercase();
        if let Some(pos) = cand_lower.find(&t_lower) {
            matched_tokens += 1;
            total_score += 40 + (t_lower.len() as i64 * 5);
            let char_start = candidate[..pos].chars().count();
            let token_char_len = token.chars().count();
            for i in char_start..(char_start + token_char_len) {
                matched_indices.push(i);
            }
        } else if let Some(sub_res) = fuzzy_match(&t_lower, &cand_lower) {
            matched_tokens += 1;
            total_score += sub_res.score / 2;
            matched_indices.extend(sub_res.matched_indices);
        }
    }

    if matched_tokens == q_tokens.len() {
        matched_indices.sort_unstable();
        matched_indices.dedup();
        Some(PromptMatch {
            item: PromptHistoryItem::new(candidate),
            score: total_score,
            matched_indices,
            match_kind: MatchKind::TokenOverlap,
            ghost_suffix: None,
            highlighted_text: None,
        })
    } else {
        None
    }
}

// ---------------------------------------------------------------------------
// Frecency Scoring
// ---------------------------------------------------------------------------

/// Computes a frecency score combining usage count and exponential recency decay.
///
/// Formula: `score = (use_count ^ 0.75) * (0.5 ^ (age_seconds / half_life_seconds)) * 100.0`
pub fn frecency_score(use_count: u32, last_used_epoch_secs: i64, now_epoch_secs: i64, half_life_secs: f64) -> f64 {
    if use_count == 0 {
        return 0.0;
    }

    let age_secs = (now_epoch_secs - last_used_epoch_secs).max(0) as f64;
    let decay = 0.5f64.powf(age_secs / half_life_secs.max(1.0));
    let freq_factor = (use_count as f64).powf(0.75);

    freq_factor * decay * 100.0
}

/// Helper to get current Unix epoch in seconds.
pub fn current_epoch_secs() -> i64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

// ---------------------------------------------------------------------------
// Token Extraction Helpers
// ---------------------------------------------------------------------------

/// Extracts the first next word token (including any leading space/delimiter) from a ghost suffix.
pub fn extract_next_word_token(suffix: &str) -> String {
    if suffix.is_empty() {
        return String::new();
    }

    let chars: Vec<char> = suffix.chars().collect();
    let mut idx = 0;

    // 1. Consume leading spaces/punctuation if any
    while idx < chars.len() && is_word_boundary_char(chars[idx]) {
        idx += 1;
    }

    // 2. Consume word characters until next boundary
    while idx < chars.len() && !is_word_boundary_char(chars[idx]) {
        idx += 1;
    }

    chars[..idx].iter().collect()
}

// ---------------------------------------------------------------------------
// Built-in Presets & Coding Assistant Snippet Templates
// ---------------------------------------------------------------------------

/// Returns curated prompt templates for coding assistants.
pub fn default_prompt_templates() -> Vec<PromptHistoryItem> {
    vec![
        PromptHistoryItem::new("/review Review the current code changes for bugs, performance issues, and edge cases")
            .with_category(PromptCategory::Review)
            .with_use_count(10),
        PromptHistoryItem::new("/test Write comprehensive unit tests covering happy paths, edge cases, and errors")
            .with_category(PromptCategory::Test)
            .with_use_count(10),
        PromptHistoryItem::new("/refactor Refactor this function for better readability, modularity, and maintainability")
            .with_category(PromptCategory::Refactor)
            .with_use_count(8),
        PromptHistoryItem::new("/fix Analyze and fix the compilation/runtime error, explaining root cause")
            .with_category(PromptCategory::Debug)
            .with_use_count(9),
        PromptHistoryItem::new("/explain Explain how this component and architecture work step by step")
            .with_category(PromptCategory::Doc)
            .with_use_count(7),
        PromptHistoryItem::new("/doc Generate thorough documentation comments and usage examples")
            .with_category(PromptCategory::Doc)
            .with_use_count(6),
        PromptHistoryItem::new("/model Switch active LLM model or inspect available providers")
            .with_category(PromptCategory::Command)
            .with_use_count(5),
        PromptHistoryItem::new("/file Interactive fuzzy file finder and workspace browser")
            .with_category(PromptCategory::Command)
            .with_use_count(5),
        PromptHistoryItem::new("/skills Search, list, and manage extensible agent capabilities")
            .with_category(PromptCategory::Command)
            .with_use_count(4),
        PromptHistoryItem::new("/session Manage, export, or resume saved conversation sessions")
            .with_category(PromptCategory::Command)
            .with_use_count(4),
        PromptHistoryItem::new("Write a pure-Rust implementation of")
            .with_category(PromptCategory::Code)
            .with_use_count(6),
        PromptHistoryItem::new("Optimize memory allocations and algorithmic complexity in")
            .with_category(PromptCategory::Refactor)
            .with_use_count(5),
        PromptHistoryItem::new("Add comprehensive docstrings and rustdoc examples for all public items")
            .with_category(PromptCategory::Doc)
            .with_use_count(5),
        PromptHistoryItem::new("Benchmark this implementation and compare throughput vs latency")
            .with_category(PromptCategory::Code)
            .with_use_count(4),
    ]
}

// ---------------------------------------------------------------------------
// PromptAutocompleter (Main Helper)
// ---------------------------------------------------------------------------

/// Stateful prompt history manager and autocompletion matching helper.
#[derive(Debug, Clone)]
pub struct PromptAutocompleter {
    /// History items.
    history: Vec<PromptHistoryItem>,
    /// Curated prompt templates and presets.
    templates: Vec<PromptHistoryItem>,
    /// Matcher configuration.
    config: PromptMatchConfig,
    /// Fast lookup index of history texts.
    known_texts: HashSet<String>,
}

impl Default for PromptAutocompleter {
    fn default() -> Self {
        Self::new()
    }
}

impl PromptAutocompleter {
    /// Create a new autocompleter initialized with default templates.
    pub fn new() -> Self {
        let templates = default_prompt_templates();
        let mut known_texts = HashSet::new();
        for t in &templates {
            known_texts.insert(t.text.clone());
        }

        Self {
            history: Vec::new(),
            templates,
            config: PromptMatchConfig::default(),
            known_texts,
        }
    }

    /// Builder: Set custom matcher configuration.
    pub fn with_config(mut self, config: PromptMatchConfig) -> Self {
        self.config = config;
        self
    }

    /// Builder: Initialize with history strings.
    pub fn with_history_strings(mut self, history: &[String]) -> Self {
        self.import_history_slice(history);
        self
    }

    /// Return reference to configuration.
    pub fn config(&self) -> &PromptMatchConfig {
        &self.config
    }

    /// Mutable reference to configuration.
    pub fn config_mut(&mut self) -> &mut PromptMatchConfig {
        &mut self.config
    }

    /// Return slice of recorded history entries.
    pub fn history(&self) -> &[PromptHistoryItem] {
        &self.history
    }

    /// Return slice of templates.
    pub fn templates(&self) -> &[PromptHistoryItem] {
        &self.templates
    }

    /// Append a single history string.
    pub fn add_history(&mut self, text: impl Into<String>) {
        let text_str = text.into();
        let trimmed = text_str.trim();
        if trimmed.is_empty() {
            return;
        }

        if let Some(existing) = self.history.iter_mut().find(|h| h.text == trimmed) {
            existing.record_use();
        } else {
            let item = PromptHistoryItem::new(trimmed);
            self.known_texts.insert(trimmed.to_string());
            self.history.push(item);
        }
    }

    /// Import a slice of string history entries.
    pub fn import_history_slice(&mut self, history: &[String]) {
        for entry in history {
            self.add_history(entry.clone());
        }
    }

    /// Export all recorded prompt strings in chronological order.
    pub fn export_history_strings(&self) -> Vec<String> {
        self.history.iter().map(|h| h.text.clone()).collect()
    }

    /// Clear all history entries (templates are preserved).
    pub fn clear_history(&mut self) {
        self.history.clear();
        self.known_texts.clear();
        for t in &self.templates {
            self.known_texts.insert(t.text.clone());
        }
    }

    /// Find the highest-ranked inline ghost autocompletion suggestion for the given buffer.
    ///
    /// Returns `Some(GhostCompletion)` if a candidate starts with the current buffer prefix.
    pub fn get_ghost_suggestion(&self, buffer: &str, _cursor_pos: usize) -> Option<GhostCompletion> {
        let trimmed = buffer.trim_start();
        if trimmed.chars().count() < self.config.min_prefix_len {
            return None;
        }

        let now = current_epoch_secs();
        let mut best_ghost: Option<GhostCompletion> = None;
        let mut best_score: i64 = i64::MIN;

        // Search history first (most relevant), then templates
        let candidates = self.history.iter().chain(self.templates.iter());

        for item in candidates {
            if let Some(fuzzy_res) = fuzzy_match_with_config(buffer, &item.text, &self.config) {
                if fuzzy_res.is_prefix && !fuzzy_res.is_exact {
                    let mut score = fuzzy_res.score;

                    // Apply frecency boost
                    if self.config.enable_frecency {
                        let frec = frecency_score(
                            item.use_count,
                            item.last_used,
                            now,
                            self.config.frecency_half_life_secs,
                        );
                        score += (frec * self.config.frecency_weight) as i64;
                    }

                    if score > best_score {
                        if let Some(ghost) = GhostCompletion::new(
                            buffer,
                            &item.text,
                            score,
                            fuzzy_res.match_kind,
                        ) {
                            best_score = score;
                            best_ghost = Some(ghost);
                        }
                    }
                }
            }
        }

        best_ghost
    }

    /// Perform fuzzy search across history and templates, returning ranked matches.
    pub fn fuzzy_search(&self, query: &str, max_results: usize) -> Vec<PromptMatch> {
        let trimmed = query.trim();
        let now = current_epoch_secs();
        let limit = if max_results == 0 {
            self.config.max_suggestions
        } else {
            max_results
        };

        if trimmed.is_empty() {
            // Return most recent / popular items when query is empty
            let mut recent: Vec<PromptMatch> = self
                .history
                .iter()
                .rev()
                .take(limit)
                .map(|item| {
                    let frec = frecency_score(
                        item.use_count,
                        item.last_used,
                        now,
                        self.config.frecency_half_life_secs,
                    );
                    PromptMatch {
                        item: item.clone(),
                        score: frec as i64,
                        matched_indices: Vec::new(),
                        match_kind: MatchKind::Exact,
                        ghost_suffix: None,
                        highlighted_text: Some(item.text.clone()),
                    }
                })
                .collect();

            if recent.len() < limit {
                for t in self.templates.iter().take(limit - recent.len()) {
                    recent.push(PromptMatch {
                        item: t.clone(),
                        score: 50,
                        matched_indices: Vec::new(),
                        match_kind: MatchKind::SnippetTemplate,
                        ghost_suffix: None,
                        highlighted_text: Some(t.text.clone()),
                    });
                }
            }
            return recent;
        }

        let mut matches: Vec<PromptMatch> = Vec::new();
        let mut seen_texts = HashSet::new();

        // Search history and templates
        let candidates = self.history.iter().chain(self.templates.iter());

        for item in candidates {
            if self.config.deduplicate_by_text && seen_texts.contains(&item.text) {
                continue;
            }

            let mut matched = false;

            // 1. Try fuzzy match
            if let Some(fuzzy_res) = fuzzy_match_with_config(trimmed, &item.text, &self.config) {
                let mut score = fuzzy_res.score;

                // Frecency boost
                if self.config.enable_frecency {
                    let frec = frecency_score(
                        item.use_count,
                        item.last_used,
                        now,
                        self.config.frecency_half_life_secs,
                    );
                    score += (frec * self.config.frecency_weight) as i64;
                }

                // Category bonus
                score += item.category.icon().len() as i64;

                let ghost_suffix = if fuzzy_res.is_prefix && !fuzzy_res.is_exact {
                    let p_len = trimmed.chars().count();
                    let c_chars: Vec<char> = item.text.chars().collect();
                    if p_len < c_chars.len() {
                        Some(c_chars[p_len..].iter().collect())
                    } else {
                        None
                    }
                } else {
                    None
                };

                let highlighted = highlight_matched_chars(&item.text, &fuzzy_res.matched_indices, "\x1b[1;33m");

                matches.push(PromptMatch {
                    item: item.clone(),
                    score,
                    matched_indices: fuzzy_res.matched_indices,
                    match_kind: fuzzy_res.match_kind,
                    ghost_suffix,
                    highlighted_text: Some(highlighted),
                });
                seen_texts.insert(item.text.clone());
                matched = true;
            }

            // 2. Try token overlap if fuzzy match didn't match and token overlap enabled
            if !matched && self.config.enable_token_overlap {
                if let Some(mut token_match) = match_token_overlap(trimmed, &item.text) {
                    if self.config.enable_frecency {
                        let frec = frecency_score(
                            item.use_count,
                            item.last_used,
                            now,
                            self.config.frecency_half_life_secs,
                        );
                        token_match.score += (frec * self.config.frecency_weight) as i64;
                    }
                    token_match.highlighted_text = Some(highlight_matched_chars(
                        &item.text,
                        &token_match.matched_indices,
                        "\x1b[1;33m",
                    ));
                    matches.push(token_match);
                    seen_texts.insert(item.text.clone());
                }
            }
        }

        // Sort descending by score
        matches.sort();
        matches.truncate(limit);
        matches
    }

    /// Return prompt suggestions filtered by category.
    pub fn filter_by_category(&self, category: &PromptCategory) -> Vec<PromptHistoryItem> {
        self.history
            .iter()
            .chain(self.templates.iter())
            .filter(|item| &item.category == category)
            .cloned()
            .collect()
    }

    /// Return the top N most frequently used prompt items.
    pub fn top_frequent(&self, limit: usize) -> Vec<PromptHistoryItem> {
        let mut items = self.history.clone();
        items.sort_by(|a, b| b.use_count.cmp(&a.use_count));
        items.truncate(limit);
        items
    }

    /// Return the top N most recently used prompt items.
    pub fn top_recent(&self, limit: usize) -> Vec<PromptHistoryItem> {
        let mut items = self.history.clone();
        items.sort_by(|a, b| b.last_used.cmp(&a.last_used));
        items.truncate(limit);
        items
    }
}

// ---------------------------------------------------------------------------
// Interactive Prompt Autocompletion State Machine
// ---------------------------------------------------------------------------

/// Mode of display for autocompletion UI.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompletionUIMode {
    /// Inline ghost suggestion text only.
    InlineGhost,
    /// Floating popup completion menu.
    PopupMenu,
    /// Reverse interactive history search (`Ctrl+R`).
    ReverseSearch,
}

/// Interactive state tracking buffer input, ghost completions, and popup dropdown.
#[derive(Debug, Clone)]
pub struct PromptCompletionState {
    /// Whether completion UI is actively displaying a dropdown or reverse search.
    pub is_open: bool,
    /// Current display mode.
    pub mode: CompletionUIMode,
    /// Current ranked candidate matches.
    pub matches: Vec<PromptMatch>,
    /// Currently selected index in `matches`.
    pub selected_index: usize,
    /// Current inline ghost autocompletion.
    pub ghost: Option<GhostCompletion>,
    /// Last query string.
    pub last_query: String,
}

impl Default for PromptCompletionState {
    fn default() -> Self {
        Self::new()
    }
}

impl PromptCompletionState {
    /// Create new empty state.
    pub fn new() -> Self {
        Self {
            is_open: false,
            mode: CompletionUIMode::InlineGhost,
            matches: Vec::new(),
            selected_index: 0,
            ghost: None,
            last_query: String::new(),
        }
    }

    /// Update matching state as user types in the prompt buffer.
    pub fn on_buffer_changed(
        &mut self,
        completer: &PromptAutocompleter,
        buffer: &str,
        cursor_pos: usize,
    ) {
        self.last_query = buffer.to_string();

        // 1. Update inline ghost suggestion
        self.ghost = completer.get_ghost_suggestion(buffer, cursor_pos);

        // 2. If popup or reverse search is open, update matches
        if self.is_open {
            self.matches = completer.fuzzy_search(buffer, completer.config().max_suggestions);
            if self.selected_index >= self.matches.len() {
                self.selected_index = 0;
            }
        }
    }

    /// Open popup completion menu.
    pub fn open_popup(&mut self, completer: &PromptAutocompleter, buffer: &str) {
        self.is_open = true;
        self.mode = CompletionUIMode::PopupMenu;
        self.matches = completer.fuzzy_search(buffer, completer.config().max_suggestions);
        self.selected_index = 0;
    }

    /// Open reverse history search mode (`Ctrl+R`).
    pub fn open_reverse_search(&mut self, completer: &PromptAutocompleter, query: &str) {
        self.is_open = true;
        self.mode = CompletionUIMode::ReverseSearch;
        self.matches = completer.fuzzy_search(query, completer.config().max_suggestions);
        self.selected_index = 0;
    }

    /// Close popup or reverse search menu.
    pub fn close(&mut self) {
        self.is_open = false;
        self.mode = CompletionUIMode::InlineGhost;
        self.matches.clear();
        self.selected_index = 0;
    }

    /// Navigate to next candidate item.
    pub fn select_next(&mut self) {
        if !self.matches.is_empty() {
            self.selected_index = (self.selected_index + 1) % self.matches.len();
        }
    }

    /// Navigate to previous candidate item.
    pub fn select_prev(&mut self) {
        if !self.matches.is_empty() {
            if self.selected_index == 0 {
                self.selected_index = self.matches.len() - 1;
            } else {
                self.selected_index -= 1;
            }
        }
    }

    /// Return reference to currently selected match.
    pub fn selected_match(&self) -> Option<&PromptMatch> {
        self.matches.get(self.selected_index)
    }

    /// Accept inline ghost completion in whole, updating the buffer.
    pub fn accept_ghost_full(&mut self, buffer: &mut String) -> bool {
        if let Some(ghost) = &self.ghost {
            *buffer = ghost.full_text.clone();
            self.ghost = None;
            true
        } else {
            false
        }
    }

    /// Accept the next word token of the ghost completion, updating the buffer.
    pub fn accept_ghost_next_word(&mut self, buffer: &mut String) -> bool {
        if let Some(ghost) = &self.ghost {
            *buffer = ghost.accept_next_word();
            true
        } else {
            false
        }
    }

    /// Accept currently selected popup match, updating buffer and closing popup.
    pub fn accept_selected_match(&mut self, buffer: &mut String) -> bool {
        if let Some(m) = self.selected_match() {
            *buffer = m.item.text.clone();
            self.close();
            true
        } else {
            false
        }
    }
}

// ---------------------------------------------------------------------------
// ANSI Rendering & Highlighting Helpers
// ---------------------------------------------------------------------------

/// Highlights characters at `matched_indices` with the given ANSI color escape sequence.
pub fn highlight_matched_chars(text: &str, matched_indices: &[usize], highlight_ansi: &str) -> String {
    if matched_indices.is_empty() {
        return text.to_string();
    }

    let char_indices_set: HashSet<usize> = matched_indices.iter().copied().collect();
    let mut out = String::with_capacity(text.len() + (matched_indices.len() * 12));
    let mut in_highlight = false;

    for (i, c) in text.chars().enumerate() {
        if char_indices_set.contains(&i) {
            if !in_highlight {
                out.push_str(highlight_ansi);
                in_highlight = true;
            }
            out.push(c);
        } else {
            if in_highlight {
                out.push_str("\x1b[0m");
                in_highlight = false;
            }
            out.push(c);
        }
    }

    if in_highlight {
        out.push_str("\x1b[0m");
    }

    out
}

/// Renders a completion popup dropdown menu as an ANSI string.
pub fn render_completion_popup(
    matches: &[PromptMatch],
    selected_idx: usize,
    max_width: usize,
) -> String {
    if matches.is_empty() {
        return String::new();
    }

    let width = max_width.clamp(40, 100);
    let mut lines = Vec::new();

    let border_color = "\x1b[38;5;238m";
    let reset = "\x1b[0m";

    // Top border
    lines.push(format!("{}┌{}┐{}", border_color, "─".repeat(width.saturating_sub(2)), reset));

    for (idx, m) in matches.iter().enumerate() {
        let is_selected = idx == selected_idx;
        let prefix = if is_selected {
            "\x1b[1;36m❯ "
        } else {
            "  "
        };

        let badge = m.item.category.badge_ansi();
        let icon = m.item.category.icon();

        // Truncate text if needed
        let max_text_len = width.saturating_sub(18);
        let display_text = if let Some(hl) = &m.highlighted_text {
            hl.clone()
        } else {
            truncate_str(&m.item.text, max_text_len)
        };

        let row_bg = if is_selected {
            "\x1b[48;5;236m"
        } else {
            ""
        };

        let line = format!(
            "{row_bg}{prefix}{icon} {badge} {display_text}{reset}"
        );
        lines.push(line);
    }

    // Bottom border with helper tips
    let tip = "\x1b[38;5;244m[Tab/Enter] Accept  [↑/↓] Navigate  [Esc] Dismiss\x1b[0m";
    lines.push(format!("{}└─ {} ─{}┘{}", border_color, tip, "─".repeat(width.saturating_sub(55)), reset));

    lines.join("\n")
}

/// Renders reverse history search banner (`Ctrl+R`) as an ANSI string.
pub fn render_reverse_history_search(
    query: &str,
    matches: &[PromptMatch],
    selected_idx: usize,
    total_count: usize,
    max_width: usize,
) -> String {
    let width = max_width.clamp(40, 100);
    let reset = "\x1b[0m";
    let mut lines = Vec::new();

    // Header search prompt
    lines.push(format!(
        "\x1b[1;35mbck-i-search\x1b[0m: \x1b[1;37m{}\x1b[0m_ \x1b[38;5;244m({}/{} matches)\x1b[0m",
        query,
        matches.len(),
        total_count
    ));

    let max_text_len = width.saturating_sub(14);
    for (idx, m) in matches.iter().enumerate() {
        let is_selected = idx == selected_idx;
        let prefix = if is_selected {
            "\x1b[1;36m❯ "
        } else {
            "  "
        };
        let bg = if is_selected {
            "\x1b[48;5;236m"
        } else {
            ""
        };
        let text = if let Some(hl) = &m.highlighted_text {
            hl.clone()
        } else {
            truncate_str(&m.item.text, max_text_len)
        };
        let badge = m.item.category.badge_ansi();
        lines.push(format!("{bg}{prefix}{badge} {text}{reset}"));
    }
    lines.join("\n")
}

/// Helper function to truncate strings to a visible character length.
fn truncate_str(s: &str, max_chars: usize) -> String {
    let chars: Vec<char> = s.chars().collect();
    if chars.len() <= max_chars {
        s.to_string()
    } else {
        let truncated: String = chars[..max_chars.saturating_sub(3)].iter().collect();
        format!("{}...", truncated)
    }
}

// ---------------------------------------------------------------------------
// Standalone Top-Level Convenience Functions
// ---------------------------------------------------------------------------

/// Suggest prompt autocompletion text for a given user input buffer from a history slice.
///
/// Returns `Some(completion_text)` if a candidate starts with `buffer`.
pub fn suggest_completion(buffer: &str, history: &[String]) -> Option<String> {
    let completer = PromptAutocompleter::new().with_history_strings(history);
    completer
        .get_ghost_suggestion(buffer, buffer.chars().count())
        .map(|g| g.full_text)
}

/// Suggest detailed inline ghost completion with word-acceptance metadata.
pub fn suggest_ghost_completion(buffer: &str, history: &[String]) -> Option<GhostCompletion> {
    let completer = PromptAutocompleter::new().with_history_strings(history);
    completer.get_ghost_suggestion(buffer, buffer.chars().count())
}

/// Fuzzy search across a slice of prompt history strings.
pub fn fuzzy_search_history(
    query: &str,
    history: &[String],
    max_results: usize,
) -> Vec<PromptMatch> {
    let completer = PromptAutocompleter::new().with_history_strings(history);
    completer.fuzzy_search(query, max_results)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_exact_and_prefix_fuzzy_match() {
        let exact = fuzzy_match("cargo build", "cargo build");
        assert!(exact.is_some());
        let res = exact.unwrap();
        assert!(res.is_exact);
        assert!(res.is_prefix);
        assert_eq!(res.match_kind, MatchKind::Exact);

        let prefix = fuzzy_match("cargo", "cargo build --release");
        assert!(prefix.is_some());
        let p_res = prefix.unwrap();
        assert!(!p_res.is_exact);
        assert!(p_res.is_prefix);
        assert_eq!(p_res.match_kind, MatchKind::Prefix);
        assert_eq!(p_res.matched_indices, vec![0, 1, 2, 3, 4]);
    }

    #[test]
    fn test_subsequence_fuzzy_match() {
        let res = fuzzy_match("cb", "cargo build");
        assert!(res.is_some());
        let m = res.unwrap();
        assert_eq!(m.match_kind, MatchKind::Subsequence);
        assert_eq!(m.matched_indices, vec![0, 6]);

        let non_match = fuzzy_match("xyz", "cargo build");
        assert!(non_match.is_none());
    }

    #[test]
    fn test_ghost_completion_extraction() {
        let ghost = GhostCompletion::new("cargo b", "cargo build --release", 100, MatchKind::Prefix);
        assert!(ghost.is_some());
        let g = ghost.unwrap();
        assert_eq!(g.prefix, "cargo b");
        assert_eq!(g.ghost_suffix, "uild --release");
        assert_eq!(g.next_word, "uild");
        assert_eq!(g.accept_next_word(), "cargo build");
        assert_eq!(g.accept_all(), "cargo build --release");
    }

    #[test]
    fn test_token_overlap_match() {
        let overlap = match_token_overlap("release cargo", "cargo build --release");
        assert!(overlap.is_some());
        let m = overlap.unwrap();
        assert_eq!(m.match_kind, MatchKind::TokenOverlap);
    }

    #[test]
    fn test_frecency_scoring() {
        let now = 1_000_000;
        let score_recent_frequent = frecency_score(10, now - 3600, now, 86400.0);
        let score_old_frequent = frecency_score(10, now - (86400 * 7), now, 86400.0);
        let score_recent_infrequent = frecency_score(1, now - 3600, now, 86400.0);

        assert!(score_recent_frequent > score_old_frequent);
        assert!(score_recent_frequent > score_recent_infrequent);
    }

    #[test]
    fn test_autocompleter_history_and_ghost() {
        let mut completer = PromptAutocompleter::new();
        completer.add_history("git status");
        completer.add_history("git commit -m 'feat: add prompt match'");
        completer.add_history("git push origin main");

        let ghost = completer.get_ghost_suggestion("git com", 7);
        assert!(ghost.is_some());
        let g = ghost.unwrap();
        assert_eq!(g.full_text, "git commit -m 'feat: add prompt match'");
        assert_eq!(g.ghost_suffix, "mit -m 'feat: add prompt match'");
        assert_eq!(g.next_word, "mit");
    }

    #[test]
    fn test_prompt_completion_state_navigation() {
        let mut completer = PromptAutocompleter::new();
        completer.add_history("docker run -it ubuntu");
        completer.add_history("docker ps -a");
        completer.add_history("docker compose up");

        let mut state = PromptCompletionState::new();
        state.open_popup(&completer, "docker");
        assert!(state.is_open);
        assert_eq!(state.matches.len(), 3);
        assert_eq!(state.selected_index, 0);

        state.select_next();
        assert_eq!(state.selected_index, 1);

        state.select_next();
        assert_eq!(state.selected_index, 2);

        state.select_next();
        assert_eq!(state.selected_index, 0); // Wrap around

        state.select_prev();
        assert_eq!(state.selected_index, 2); // Wrap backwards
    }

    #[test]
    fn test_highlight_matched_chars() {
        let text = "cargo build";
        let indices = vec![0, 1, 2, 3, 4]; // "cargo"
        let highlighted = highlight_matched_chars(text, &indices, "\x1b[1;33m");
        assert!(highlighted.starts_with("\x1b[1;33mcargo\x1b[0m build"));
    }

    #[test]
    fn test_prompt_category_detection() {
        assert_eq!(PromptCategory::detect_from_text("/review diff"), PromptCategory::Review);
        assert_eq!(PromptCategory::detect_from_text("/test src/ui"), PromptCategory::Test);
        assert_eq!(PromptCategory::detect_from_text("/fix panic"), PromptCategory::Debug);
        assert_eq!(PromptCategory::detect_from_text("fn test_fn() {}"), PromptCategory::Code);
        assert_eq!(PromptCategory::detect_from_text("How do I use this?"), PromptCategory::General);
    }

    #[test]
    fn test_standalone_suggest_completion() {
        let history = vec![
            "cargo test --all".to_string(),
            "cargo check".to_string(),
            "cargo clippy".to_string(),
        ];

        let suggestion = suggest_completion("cargo t", &history);
        assert_eq!(suggestion, Some("cargo test --all".to_string()));
    }

    #[test]
    fn test_render_completion_popup() {
        let matches = vec![
            PromptMatch {
                item: PromptHistoryItem::new("git status").with_category(PromptCategory::Command),
                score: 250,
                matched_indices: vec![0, 1, 2],
                match_kind: MatchKind::Prefix,
                ghost_suffix: Some(" status".to_string()),
                highlighted_text: Some("\x1b[1;33mgit\x1b[0m status".to_string()),
            },
        ];
        let popup = render_completion_popup(&matches, 0, 60);
        assert!(popup.contains("git"));
        assert!(popup.contains("Cmd"));
    }

    #[test]
    fn test_render_reverse_history_search() {
        let matches = vec![
            PromptMatch {
                item: PromptHistoryItem::new("cargo run --bin fusion"),
                score: 300,
                matched_indices: vec![0, 1, 2, 3, 4],
                match_kind: MatchKind::Prefix,
                ghost_suffix: None,
                highlighted_text: None,
            },
        ];
        let search_view = render_reverse_history_search("cargo", &matches, 0, 10, 70);
        assert!(search_view.contains("bck-i-search"));
        assert!(search_view.contains("cargo run --bin fusion"));
    }

    #[test]
    fn test_word_by_word_ghost_acceptance() {
        let mut completer = PromptAutocompleter::new();
        completer.add_history("npm run build:prod -- --verbose");

        let mut state = PromptCompletionState::new();
        state.on_buffer_changed(&completer, "npm", 3);
        assert!(state.ghost.is_some());

        let mut buf = "npm".to_string();
        assert!(state.accept_ghost_next_word(&mut buf));
        assert_eq!(buf, "npm run");

        state.on_buffer_changed(&completer, &buf, buf.chars().count());
        assert!(state.accept_ghost_next_word(&mut buf));
        assert_eq!(buf, "npm run build");

        state.on_buffer_changed(&completer, &buf, buf.chars().count());
        assert!(state.accept_ghost_full(&mut buf));
        assert_eq!(buf, "npm run build:prod -- --verbose");
    }

    #[test]
    fn test_top_frequent_and_recent() {
        let mut completer = PromptAutocompleter::new();
        completer.add_history("prompt A");
        completer.add_history("prompt B");
        completer.add_history("prompt A"); // use_count = 2

        let top = completer.top_frequent(1);
        assert_eq!(top.len(), 1);
        assert_eq!(top[0].text, "prompt A");
        assert_eq!(top[0].use_count, 2);

        let exported = completer.export_history_strings();
        assert_eq!(exported.len(), 2);
    }
}
