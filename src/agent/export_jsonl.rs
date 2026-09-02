//! JSONL session exporter for LLM fine-tuning datasets and evaluation benchmarks.
//!
//! Provides high-performance, configurable export of conversational agent sessions
//! into standard JSON Lines (`.jsonl`) formats for:
//! - **Supervised Fine-Tuning (SFT)** (OpenAI Chat, ShareGPT, Alpaca, Anthropic Claude, Base Completion)
//! - **Tool-Calling / Agentic Fine-Tuning** (OpenAI Function/Tool Calling schemas & outputs)
//! - **Direct Preference Optimization (DPO / RLHF)** (Pairwise preference formats)
//! - **LLM Evaluation & Benchmarking** (Standardized test cases with inputs, expected outputs, and metadata)
//! - **Offline Auditing & Telemetry** (Turn-by-turn event logs with token metrics)
//!
//! # Features
//! - Multi-format support: OpenAI Chat, OpenAI Tools, ShareGPT, Alpaca, Anthropic, Prompt-Completion, DPO, LLM Evals.
//! - Flexible multi-turn split strategies: Full session, sliding-window every assistant turn, last turn only, user-assistant pairs.
//! - DeepSeek / reasoning `<think>` block extraction, stripping, or preservation into dedicated reasoning fields.
//! - Tool call formatting: native structured tool calls or flattened text representations.
//! - Sensitive data masking: sanitizes API keys, tokens, secrets, and authorization headers.
//! - Built-in fine-tuning format validation against provider specifications.
//! - Deterministic train / validation / test dataset partitioning with customizable ratios.

use std::collections::HashMap;
use std::fmt;
use std::fs;
use std::io::{self, BufRead, BufReader, Write};
use std::path::Path;

use regex::Regex;
use serde::{Deserialize, Serialize};

use crate::agent::session::Session;
use crate::provider::types::{Message, Role, ToolCall, ToolDefinition};

// ============================================================================
// Enums & Formats
// ============================================================================

/// Target format for the exported JSONL lines.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum JsonlFormat {
    /// OpenAI chat fine-tuning format: `{"messages": [{"role": "...", "content": "..."}]}`
    #[default]
    OpenAiChat,
    /// OpenAI tool-calling fine-tuning format with structured tool definitions and calls.
    OpenAiToolCalling,
    /// ShareGPT / Axolotl / LLaMA-Factory format: `{"conversations": [{"from": "human"|"gpt", "value": "..."}]}`
    ShareGpt,
    /// Alpaca instruction dataset format: `{"instruction": "...", "input": "...", "output": "..."}`
    Alpaca,
    /// Anthropic Claude messages format: `{"system": "...", "messages": [{"role": "user"|"assistant", "content": "..."}]}`
    Anthropic,
    /// Legacy / Base model completion format: `{"prompt": "...", "completion": "..."}`
    PromptCompletion,
    /// Direct Preference Optimization (DPO / RLHF) preference format: `{"prompt": "...", "chosen": "...", "rejected": "..."}`
    PreferenceDpo,
    /// Standardized LLM evaluation test case format.
    LlmEvaluation,
    /// Raw chronological turn breakdown for telemetry, logging, and offline analysis.
    RawTurns,
}

impl JsonlFormat {
    /// Returns the standard file extension or identifier for this format.
    pub fn identifier(&self) -> &'static str {
        match self {
            Self::OpenAiChat => "openai_chat",
            Self::OpenAiToolCalling => "openai_tools",
            Self::ShareGpt => "sharegpt",
            Self::Alpaca => "alpaca",
            Self::Anthropic => "anthropic",
            Self::PromptCompletion => "prompt_completion",
            Self::PreferenceDpo => "dpo_preference",
            Self::LlmEvaluation => "llm_eval",
            Self::RawTurns => "raw_turns",
        }
    }

    /// Returns a human-friendly description of the target format.
    pub fn description(&self) -> &'static str {
        match self {
            Self::OpenAiChat => "OpenAI Chat Fine-Tuning Format (messages array)",
            Self::OpenAiToolCalling => "OpenAI Tool Calling Fine-Tuning Format (messages + tools)",
            Self::ShareGpt => "ShareGPT / FastChat / Axolotl Format (conversations array)",
            Self::Alpaca => "Alpaca Instruction Dataset Format (instruction/input/output)",
            Self::Anthropic => "Anthropic Claude Messages Format (system + messages)",
            Self::PromptCompletion => "Prompt-Completion Pair Format (prompt/completion)",
            Self::PreferenceDpo => "DPO / RLHF Preference Pair Format (prompt/chosen/rejected)",
            Self::LlmEvaluation => "LLM Evaluation Benchmark Case Format",
            Self::RawTurns => "Raw Turn Breakdown / Audit Event Format",
        }
    }
}

/// Multi-turn conversation splitting strategy for dataset generation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TurnSplitStrategy {
    /// Export the entire session as a single multi-turn sample line.
    #[default]
    FullSession,
    /// Sliding window: export each assistant turn as an independent sample, with all previous turns as context.
    EveryAssistantTurn,
    /// Export only the final turn exchange of the session.
    LastTurnOnly,
    /// Split into individual 1-turn (user, assistant) message pairs.
    UserAssistantPairs,
}

/// Handling strategy for `<think>...</think>` or `<thought>...</thought>` reasoning blocks.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ThoughtHandling {
    /// Preserve `<think>` tags verbatim in the exported message text.
    #[default]
    Preserve,
    /// Strip `<think>...</think>` blocks entirely from the exported text.
    Strip,
    /// Extract thinking blocks into a separate `"reasoning"` or `"thought"` JSON field.
    ExtractToField,
}

// ============================================================================
// Errors & Results
// ============================================================================

/// Errors that can occur during JSONL session export operations.
#[derive(Debug)]
pub enum JsonlExportError {
    /// I/O error reading from or writing to disk.
    Io(io::Error),
    /// JSON serialization or deserialization failed.
    Serialization(serde_json::Error),
    /// Validation error against target format specification.
    Validation(String),
    /// The session contains no valid messages or failed filtering constraints.
    EmptySession(String),
    /// An invalid option configuration was provided.
    InvalidConfiguration(String),
}

impl fmt::Display for JsonlExportError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(err) => write!(f, "I/O error during JSONL export: {}", err),
            Self::Serialization(err) => write!(f, "JSON serialization error: {}", err),
            Self::Validation(msg) => write!(f, "Dataset validation failed: {}", msg),
            Self::EmptySession(msg) => write!(f, "Empty or invalid session: {}", msg),
            Self::InvalidConfiguration(msg) => write!(f, "Invalid export configuration: {}", msg),
        }
    }
}

impl std::error::Error for JsonlExportError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(err) => Some(err),
            Self::Serialization(err) => Some(err),
            _ => None,
        }
    }
}

impl From<io::Error> for JsonlExportError {
    fn from(err: io::Error) -> Self {
        Self::Io(err)
    }
}

impl From<serde_json::Error> for JsonlExportError {
    fn from(err: serde_json::Error) -> Self {
        Self::Serialization(err)
    }
}

// ============================================================================
// Export Options
// ============================================================================

/// Configuration options for tailoring JSONL exports.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JsonlExportOptions {
    /// Target JSONL schema/format.
    pub format: JsonlFormat,
    /// Multi-turn conversation splitting strategy.
    pub split_strategy: TurnSplitStrategy,
    /// Strategy for handling reasoning / thought blocks.
    pub thought_handling: ThoughtHandling,
    /// Whether to include system prompt messages in the exported samples.
    pub include_system_messages: bool,
    /// Whether to include tool calls and tool outputs.
    pub include_tool_calls: bool,
    /// Whether to flatten tool calls into text representations (e.g. `<tool_call>`).
    pub flatten_tool_calls_to_text: bool,
    /// Optional system prompt override (replaces session system prompt if present).
    pub system_prompt_override: Option<String>,
    /// Whether to automatically mask sensitive information (API keys, tokens, secrets).
    pub mask_sensitive_data: bool,
    /// Minimum number of messages required for a sample to be exported.
    pub min_messages: usize,
    /// Maximum number of messages per exported sample (truncates oldest if exceeded).
    pub max_messages: Option<usize>,
    /// Minimum character length for an assistant turn to be included.
    pub min_assistant_chars: usize,
    /// Maximum character length per message (truncates tail if exceeded).
    pub max_message_chars: Option<usize>,
    /// Optional role filter (only messages with these roles are retained).
    pub filter_roles: Option<Vec<Role>>,
    /// Additional metadata key-value pairs to inject into each sample line.
    pub extra_metadata: HashMap<String, String>,
    /// Deduplicate consecutive messages with the same role.
    pub deduplicate_consecutive_roles: bool,
    /// Enforce strict validation of exported samples against the target format.
    pub validate_output: bool,
    /// Optional tools/functions definitions to include when using `OpenAiToolCalling`.
    pub tool_definitions: Option<Vec<ToolDefinition>>,
}

impl Default for JsonlExportOptions {
    fn default() -> Self {
        Self {
            format: JsonlFormat::OpenAiChat,
            split_strategy: TurnSplitStrategy::FullSession,
            thought_handling: ThoughtHandling::Preserve,
            include_system_messages: true,
            include_tool_calls: true,
            flatten_tool_calls_to_text: false,
            system_prompt_override: None,
            mask_sensitive_data: false,
            min_messages: 2,
            max_messages: None,
            min_assistant_chars: 1,
            max_message_chars: None,
            filter_roles: None,
            extra_metadata: HashMap::new(),
            deduplicate_consecutive_roles: true,
            validate_output: false,
            tool_definitions: None,
        }
    }
}

impl JsonlExportOptions {
    /// Creates a new default `JsonlExportOptions` configured for the specified format.
    pub fn new(format: JsonlFormat) -> Self {
        Self {
            format,
            ..Default::default()
        }
    }

    /// Sets the JSONL export format.
    pub fn with_format(mut self, format: JsonlFormat) -> Self {
        self.format = format;
        self
    }

    /// Sets the multi-turn split strategy.
    pub fn with_split_strategy(mut self, strategy: TurnSplitStrategy) -> Self {
        self.split_strategy = strategy;
        self
    }

    /// Sets the thought/reasoning handling strategy.
    pub fn with_thought_handling(mut self, handling: ThoughtHandling) -> Self {
        self.thought_handling = handling;
        self
    }

    /// Toggles inclusion of system messages.
    pub fn with_system_messages(mut self, include: bool) -> Self {
        self.include_system_messages = include;
        self
    }

    /// Toggles inclusion of tool calls and results.
    pub fn with_tool_calls(mut self, include: bool) -> Self {
        self.include_tool_calls = include;
        self
    }

    /// Toggles flattening tool calls to plain text.
    pub fn with_flattened_tools(mut self, flatten: bool) -> Self {
        self.flatten_tool_calls_to_text = flatten;
        self
    }

    /// Sets a system prompt override.
    pub fn with_system_prompt_override(mut self, prompt: impl Into<String>) -> Self {
        self.system_prompt_override = Some(prompt.into());
        self
    }

    /// Toggles masking of sensitive credentials.
    pub fn with_sensitive_data_masking(mut self, mask: bool) -> Self {
        self.mask_sensitive_data = mask;
        self
    }

    /// Sets minimum messages required per sample.
    pub fn with_min_messages(mut self, min: usize) -> Self {
        self.min_messages = min;
        self
    }

    /// Sets maximum messages per sample.
    pub fn with_max_messages(mut self, max: usize) -> Self {
        self.max_messages = Some(max);
        self
    }

    /// Sets minimum characters required for assistant responses.
    pub fn with_min_assistant_chars(mut self, min_chars: usize) -> Self {
        self.min_assistant_chars = min_chars;
        self
    }

    /// Adds an extra metadata tag to each exported line.
    pub fn with_metadata(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.extra_metadata.insert(key.into(), value.into());
        self
    }

    /// Sets tool definitions for OpenAI tool calling format.
    pub fn with_tool_definitions(mut self, tools: Vec<ToolDefinition>) -> Self {
        self.tool_definitions = Some(tools);
        self
    }

    /// Toggles strict format validation.
    pub fn with_validation(mut self, validate: bool) -> Self {
        self.validate_output = validate;
        self
    }
}

// ============================================================================
// Target Schema Structs (Serialized into JSONL lines)
// ============================================================================

/// Single message in an OpenAI Chat fine-tuning sample.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct OpenAiExportMessage {
    pub role: String,
    pub content: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_calls: Option<Vec<OpenAiExportToolCall>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
}

/// Structured tool call inside an OpenAI message.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct OpenAiExportToolCall {
    pub id: String,
    #[serde(rename = "type")]
    pub call_type: String,
    pub function: OpenAiExportFunctionCall,
}

/// Function call details for an OpenAI tool call.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct OpenAiExportFunctionCall {
    pub name: String,
    pub arguments: String,
}

/// OpenAI Chat fine-tuning sample line.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OpenAiChatSample {
    pub messages: Vec<OpenAiExportMessage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tools: Option<Vec<ToolDefinition>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reasoning: Option<String>,
    #[serde(skip_serializing_if = "HashMap::is_empty", default)]
    pub metadata: HashMap<String, String>,
}

/// Single conversation turn in ShareGPT format.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ShareGptMessage {
    pub from: String,
    pub value: String,
}

/// ShareGPT / Axolotl fine-tuning sample line.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ShareGptSample {
    pub conversations: Vec<ShareGptMessage>,
    #[serde(skip_serializing_if = "HashMap::is_empty", default)]
    pub metadata: HashMap<String, String>,
}

/// Alpaca instruction dataset sample line.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AlpacaSample {
    pub instruction: String,
    pub input: String,
    pub output: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub system: Option<String>,
    #[serde(skip_serializing_if = "HashMap::is_empty", default)]
    pub metadata: HashMap<String, String>,
}

/// Anthropic Claude messages fine-tuning sample line.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AnthropicSample {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub system: Option<String>,
    pub messages: Vec<AnthropicMessage>,
    #[serde(skip_serializing_if = "HashMap::is_empty", default)]
    pub metadata: HashMap<String, String>,
}

/// Single message in Anthropic format.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AnthropicMessage {
    pub role: String,
    pub content: String,
}

/// Prompt-Completion pair sample line.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PromptCompletionSample {
    pub prompt: String,
    pub completion: String,
    #[serde(skip_serializing_if = "HashMap::is_empty", default)]
    pub metadata: HashMap<String, String>,
}

/// DPO preference dataset sample line.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DpoPreferenceSample {
    pub prompt: String,
    pub chosen: String,
    pub rejected: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub system: Option<String>,
    #[serde(skip_serializing_if = "HashMap::is_empty", default)]
    pub metadata: HashMap<String, String>,
}

/// Standardized LLM evaluation test case line.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LlmEvaluationSample {
    pub id: String,
    pub session_id: String,
    pub turn_index: usize,
    pub system_prompt: Option<String>,
    pub messages: Vec<OpenAiExportMessage>,
    pub expected_response: String,
    pub model: String,
    #[serde(skip_serializing_if = "HashMap::is_empty", default)]
    pub metadata: HashMap<String, String>,
}

/// Raw turn event log sample line.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RawTurnSample {
    pub session_id: String,
    pub turn_index: usize,
    pub role: String,
    pub content: String,
    pub has_tool_calls: bool,
    pub tool_call_count: usize,
    pub estimated_tokens: usize,
    pub timestamp: String,
    pub model: String,
    #[serde(skip_serializing_if = "HashMap::is_empty", default)]
    pub metadata: HashMap<String, String>,
}

// ============================================================================
// Statistics & Results
// ============================================================================

/// Summary statistics describing a completed JSONL export operation.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ExportStats {
    /// Number of sessions processed.
    pub total_sessions: usize,
    /// Number of JSONL sample lines successfully generated.
    pub exported_samples: usize,
    /// Number of sessions skipped due to filtering or empty messages.
    pub skipped_sessions: usize,
    /// Number of individual turns skipped.
    pub skipped_turns: usize,
    /// Total number of messages included across all samples.
    pub total_messages: usize,
    /// Estimated total token count across all exported samples.
    pub total_estimated_tokens: usize,
    /// Average estimated tokens per exported sample.
    pub avg_tokens_per_sample: f64,
    /// Total count of structured tool calls exported.
    pub total_tool_calls: usize,
    /// Target JSONL format used.
    pub format: JsonlFormat,
}

impl ExportStats {
    /// Formats a concise human-readable summary of the export statistics.
    pub fn summary(&self) -> String {
        format!(
            "Exported {} samples from {} sessions ({} skipped) in {:?} format. Total messages: {}, Tokens: ~{} (avg {:.1}/sample), Tool calls: {}",
            self.exported_samples,
            self.total_sessions,
            self.skipped_sessions,
            self.format,
            self.total_messages,
            self.total_estimated_tokens,
            self.avg_tokens_per_sample,
            self.total_tool_calls,
        )
    }
}

/// Partitioning results from splitting a dataset into train / val / test sets.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DatasetSplitResult {
    /// Number of training samples.
    pub train_count: usize,
    /// Number of validation samples.
    pub val_count: usize,
    /// Number of test samples.
    pub test_count: usize,
    /// Path where train.jsonl was written.
    pub train_path: String,
    /// Path where val.jsonl was written.
    pub val_path: String,
    /// Path where test.jsonl was written (if test ratio > 0).
    pub test_path: Option<String>,
    /// Overall export statistics.
    pub stats: ExportStats,
}

// ============================================================================
// Core Exporter Implementation
// ============================================================================

/// JSONL Exporter engine for converting sessions to fine-tuning datasets.
#[derive(Debug, Clone)]
pub struct JsonlExporter {
    options: JsonlExportOptions,
}

impl JsonlExporter {
    /// Creates a new `JsonlExporter` with default options for the given format.
    pub fn new(format: JsonlFormat) -> Self {
        Self {
            options: JsonlExportOptions::new(format),
        }
    }

    /// Creates a new `JsonlExporter` with custom options.
    pub fn with_options(options: JsonlExportOptions) -> Self {
        Self { options }
    }

    /// Returns a reference to the active export options.
    pub fn options(&self) -> &JsonlExportOptions {
        &self.options
    }

    /// Returns a mutable reference to the active export options.
    pub fn options_mut(&mut self) -> &mut JsonlExportOptions {
        &mut self.options
    }

    /// Exports a single session to a JSONL string (one or multiple lines depending on split strategy).
    pub fn export_session(&self, session: &Session) -> Result<String, JsonlExportError> {
        let (lines, _) = self.process_session(session)?;
        if lines.is_empty() {
            return Err(JsonlExportError::EmptySession(format!(
                "Session {} produced zero exportable samples under current options",
                session.id
            )));
        }
        Ok(lines.join("\n"))
    }

    /// Exports multiple sessions to a single JSONL formatted string.
    pub fn export_sessions(&self, sessions: &[Session]) -> Result<(String, ExportStats), JsonlExportError> {
        let mut all_lines = Vec::new();
        let mut stats = ExportStats {
            total_sessions: sessions.len(),
            format: self.options.format,
            ..Default::default()
        };

        for session in sessions {
            match self.process_session(session) {
                Ok((lines, session_stats)) => {
                    if lines.is_empty() {
                        stats.skipped_sessions += 1;
                    } else {
                        stats.exported_samples += lines.len();
                        stats.total_messages += session_stats.total_messages;
                        stats.total_estimated_tokens += session_stats.total_estimated_tokens;
                        stats.total_tool_calls += session_stats.total_tool_calls;
                        stats.skipped_turns += session_stats.skipped_turns;
                        all_lines.extend(lines);
                    }
                }
                Err(_) => {
                    stats.skipped_sessions += 1;
                }
            }
        }

        if stats.exported_samples > 0 {
            stats.avg_tokens_per_sample =
                stats.total_estimated_tokens as f64 / stats.exported_samples as f64;
        }

        Ok((all_lines.join("\n"), stats))
    }

    /// Exports sessions directly to a file on disk.
    pub fn export_to_file(
        &self,
        sessions: &[Session],
        path: impl AsRef<Path>,
    ) -> Result<ExportStats, JsonlExportError> {
        let (content, stats) = self.export_sessions(sessions)?;
        let path = path.as_ref();
        if let Some(parent) = path.parent() {
            if !parent.as_os_str().is_empty() {
                fs::create_dir_all(parent)?;
            }
        }
        let mut file = fs::File::create(path)?;
        file.write_all(content.as_bytes())?;
        if !content.is_empty() && !content.ends_with('\n') {
            file.write_all(b"\n")?;
        }
        Ok(stats)
    }

    /// Internal session processor that generates JSONL lines and local statistics.
    fn process_session(&self, session: &Session) -> Result<(Vec<String>, ExportStats), JsonlExportError> {
        let mut raw_messages = self.prepare_session_messages(session);
        if raw_messages.is_empty() {
            return Ok((Vec::new(), ExportStats::default()));
        }

        // Apply filters
        raw_messages = self.apply_message_filters(raw_messages);

        let mut lines = Vec::new();
        let mut session_stats = ExportStats {
            total_sessions: 1,
            format: self.options.format,
            ..Default::default()
        };

        match self.options.split_strategy {
            TurnSplitStrategy::FullSession => {
                if let Some(sample_json) = self.format_messages_to_sample(session, &raw_messages, None)? {
                    session_stats.total_messages += raw_messages.len();
                    session_stats.total_estimated_tokens += estimate_token_count(&sample_json);
                    session_stats.total_tool_calls += count_tool_calls(&raw_messages);
                    lines.push(sample_json);
                } else {
                    session_stats.skipped_turns += 1;
                }
            }
            TurnSplitStrategy::EveryAssistantTurn => {
                let mut current_history: Vec<Message> = Vec::new();
                for msg in &raw_messages {
                    current_history.push(msg.clone());
                    if msg.role == Role::Assistant {
                        if msg.content.trim().len() < self.options.min_assistant_chars && msg.tool_calls.is_none() {
                            session_stats.skipped_turns += 1;
                            continue;
                        }
                        if let Some(sample_json) =
                            self.format_messages_to_sample(session, &current_history, None)?
                        {
                            session_stats.total_messages += current_history.len();
                            session_stats.total_estimated_tokens += estimate_token_count(&sample_json);
                            session_stats.total_tool_calls += count_tool_calls(&current_history);
                            lines.push(sample_json);
                        } else {
                            session_stats.skipped_turns += 1;
                        }
                    }
                }
            }
            TurnSplitStrategy::LastTurnOnly => {
                let mut last_exchange = Vec::new();
                let mut found_assistant = false;
                for msg in raw_messages.iter().rev() {
                    if !found_assistant {
                        if msg.role == Role::Assistant {
                            found_assistant = true;
                            last_exchange.push(msg.clone());
                        }
                    } else {
                        last_exchange.push(msg.clone());
                        if msg.role == Role::User {
                            break;
                        }
                    }
                }
                last_exchange.reverse();
                if self.options.include_system_messages {
                    if let Some(sys) = raw_messages.iter().find(|m| m.role == Role::System) {
                        last_exchange.insert(0, sys.clone());
                    }
                }

                if !last_exchange.is_empty() {
                    if let Some(sample_json) =
                        self.format_messages_to_sample(session, &last_exchange, None)?
                    {
                        session_stats.total_messages += last_exchange.len();
                        session_stats.total_estimated_tokens += estimate_token_count(&sample_json);
                        session_stats.total_tool_calls += count_tool_calls(&last_exchange);
                        lines.push(sample_json);
                    }
                }
            }
            TurnSplitStrategy::UserAssistantPairs => {
                let mut pending_user: Option<Message> = None;
                for msg in &raw_messages {
                    match msg.role {
                        Role::User => {
                            pending_user = Some(msg.clone());
                        }
                        Role::Assistant => {
                            if let Some(user_msg) = pending_user.take() {
                                let mut pair = Vec::new();
                                if self.options.include_system_messages {
                                    if let Some(sys) = raw_messages.iter().find(|m| m.role == Role::System) {
                                        pair.push(sys.clone());
                                    }
                                }
                                pair.push(user_msg);
                                pair.push(msg.clone());
                                if let Some(sample_json) =
                                    self.format_messages_to_sample(session, &pair, None)?
                                {
                                    session_stats.total_messages += pair.len();
                                    session_stats.total_estimated_tokens += estimate_token_count(&sample_json);
                                    session_stats.total_tool_calls += count_tool_calls(&pair);
                                    lines.push(sample_json);
                                }
                            }
                        }
                        _ => {}
                    }
                }
            }
        }

        session_stats.exported_samples = lines.len();
        Ok((lines, session_stats))
    }

    /// Prepares session messages, incorporating system prompt overrides and initial normalizations.
    fn prepare_session_messages(&self, session: &Session) -> Vec<Message> {
        let mut messages = Vec::new();

        // Check if system prompt should be prepended
        let system_prompt = self
            .options
            .system_prompt_override
            .as_deref()
            .or(session.system_prompt());

        if self.options.include_system_messages {
            if let Some(prompt) = system_prompt {
                if !prompt.trim().is_empty() {
                    messages.push(Message::system(prompt.trim()));
                }
            }
        }

        for msg in session.messages() {
            if msg.role == Role::System && !self.options.include_system_messages {
                continue;
            }
            if msg.role == Role::Tool && !self.options.include_tool_calls {
                continue;
            }
            messages.push(msg.clone());
        }

        messages
    }

    /// Applies role filters, character constraints, and deduplication.
    fn apply_message_filters(&self, mut messages: Vec<Message>) -> Vec<Message> {
        // Role filtering
        if let Some(allowed_roles) = &self.options.filter_roles {
            messages.retain(|m| allowed_roles.contains(&m.role));
        }

        // Sensitive data masking & thought handling
        for msg in &mut messages {
            if self.options.mask_sensitive_data {
                msg.content = mask_sensitive_credentials(&msg.content);
                if let Some(tool_calls) = &mut msg.tool_calls {
                    for tc in tool_calls {
                        tc.arguments = mask_sensitive_credentials(&tc.arguments);
                    }
                }
            }

            // Thought handling
            if msg.role == Role::Assistant {
                match self.options.thought_handling {
                    ThoughtHandling::Strip => {
                        msg.content = strip_thought_tags(&msg.content);
                    }
                    ThoughtHandling::Preserve | ThoughtHandling::ExtractToField => {
                        // Keep or extract handled in formatter
                    }
                }
            }

            // Max message characters truncation
            if let Some(max_chars) = self.options.max_message_chars {
                if msg.content.chars().count() > max_chars {
                    let truncated: String = msg.content.chars().take(max_chars).collect();
                    msg.content = truncated;
                }
            }
        }

        // Deduplicate consecutive roles if configured
        if self.options.deduplicate_consecutive_roles && messages.len() > 1 {
            let mut deduplicated: Vec<Message> = Vec::with_capacity(messages.len());
            for msg in messages {
                if let Some(last) = deduplicated.last_mut() {
                    if last.role == msg.role && msg.role != Role::Tool {
                        // Merge contents
                        last.content.push_str("\n\n");
                        last.content.push_str(&msg.content);
                        if let Some(calls) = msg.tool_calls {
                            if let Some(existing_calls) = &mut last.tool_calls {
                                existing_calls.extend(calls);
                            } else {
                                last.tool_calls = Some(calls);
                            }
                        }
                        continue;
                    }
                }
                deduplicated.push(msg);
            }
            messages = deduplicated;
        }

        // Max messages slice
        if let Some(max_msgs) = self.options.max_messages {
            if messages.len() > max_msgs {
                let start = messages.len() - max_msgs;
                messages = messages[start..].to_vec();
            }
        }

        messages
    }

    /// Formats a list of messages into the target JSONL line string.
    fn format_messages_to_sample(
        &self,
        session: &Session,
        messages: &[Message],
        _turn_index: Option<usize>,
    ) -> Result<Option<String>, JsonlExportError> {
        if messages.len() < self.options.min_messages {
            return Ok(None);
        }

        match self.options.format {
            JsonlFormat::OpenAiChat => {
                let mut openai_messages = Vec::new();
                let mut extracted_reasoning: Option<String> = None;

                for msg in messages {
                    let mut content = msg.content.clone();
                    if msg.role == Role::Assistant && self.options.thought_handling == ThoughtHandling::ExtractToField {
                        if let Some((clean_content, reasoning)) = extract_thought_blocks(&content) {
                            content = clean_content;
                            extracted_reasoning = Some(reasoning);
                        }
                    }
                    if self.options.flatten_tool_calls_to_text {
                        if let Some(tool_calls) = &msg.tool_calls {
                            for tc in tool_calls {
                                content.push_str(&format!(
                                    "\n<tool_call>{{\"name\": \"{}\", \"arguments\": {}}}</tool_call>",
                                    tc.name, tc.arguments
                                ));
                            }
                        }
                        if msg.role == Role::Tool {
                            content = format!(
                                "<tool_result id=\"{}\">{}</tool_result>",
                                msg.tool_call_id.as_deref().unwrap_or(""),
                                content
                            );
                        }
                    }

                    let tool_calls = if !self.options.flatten_tool_calls_to_text && self.options.include_tool_calls {
                        msg.tool_calls.as_ref().map(|calls| {
                            calls
                                .iter()
                                .map(|tc| OpenAiExportToolCall {
                                    id: tc.id.clone(),
                                    call_type: "function".to_string(),
                                    function: OpenAiExportFunctionCall {
                                        name: tc.name.clone(),
                                        arguments: tc.arguments.clone(),
                                    },
                                })
                                .collect()
                        })
                    } else {
                        None
                    };

                    let role_str = if self.options.flatten_tool_calls_to_text && msg.role == Role::Tool {
                        "user"
                    } else {
                        role_to_str(msg.role)
                    };

                    openai_messages.push(OpenAiExportMessage {
                        role: role_str.to_string(),
                        content,
                        name: msg.name.clone(),
                        tool_calls,
                        tool_call_id: if !self.options.flatten_tool_calls_to_text && self.options.include_tool_calls {
                            msg.tool_call_id.clone()
                        } else {
                            None
                        },
                    });
                }

                let sample = OpenAiChatSample {
                    messages: openai_messages,
                    tools: None,
                    reasoning: extracted_reasoning,
                    metadata: self.options.extra_metadata.clone(),
                };

                let json = serde_json::to_string(&sample)?;
                if self.options.validate_output {
                    validate_openai_chat_sample(&sample)?;
                }
                Ok(Some(json))
            }

            JsonlFormat::OpenAiToolCalling => {
                let mut openai_messages = Vec::new();
                for msg in messages {
                    let tool_calls = msg.tool_calls.as_ref().map(|calls| {
                        calls
                            .iter()
                            .map(|tc| OpenAiExportToolCall {
                                id: tc.id.clone(),
                                call_type: "function".to_string(),
                                function: OpenAiExportFunctionCall {
                                    name: tc.name.clone(),
                                    arguments: tc.arguments.clone(),
                                },
                            })
                            .collect()
                    });

                    openai_messages.push(OpenAiExportMessage {
                        role: role_to_str(msg.role).to_string(),
                        content: msg.content.clone(),
                        name: msg.name.clone(),
                        tool_calls,
                        tool_call_id: msg.tool_call_id.clone(),
                    });
                }

                let sample = OpenAiChatSample {
                    messages: openai_messages,
                    tools: self.options.tool_definitions.clone(),
                    reasoning: None,
                    metadata: self.options.extra_metadata.clone(),
                };

                let json = serde_json::to_string(&sample)?;
                Ok(Some(json))
            }

            JsonlFormat::ShareGpt => {
                let mut convs = Vec::new();
                for msg in messages {
                    let from = match msg.role {
                        Role::System => "system",
                        Role::User => "human",
                        Role::Assistant => "gpt",
                        Role::Tool => "tool",
                    };
                    convs.push(ShareGptMessage {
                        from: from.to_string(),
                        value: msg.content.clone(),
                    });
                }

                let sample = ShareGptSample {
                    conversations: convs,
                    metadata: self.options.extra_metadata.clone(),
                };
                let json = serde_json::to_string(&sample)?;
                Ok(Some(json))
            }

            JsonlFormat::Alpaca => {
                let system = messages.iter().find(|m| m.role == Role::System).map(|m| m.content.clone());
                let user_msg = messages.iter().rfind(|m| m.role == Role::User);
                let assistant_msg = messages.iter().rfind(|m| m.role == Role::Assistant);

                if let (Some(user), Some(assistant)) = (user_msg, assistant_msg) {
                    let sample = AlpacaSample {
                        instruction: user.content.clone(),
                        input: String::new(),
                        output: assistant.content.clone(),
                        system,
                        metadata: self.options.extra_metadata.clone(),
                    };
                    let json = serde_json::to_string(&sample)?;
                    Ok(Some(json))
                } else {
                    Ok(None)
                }
            }

            JsonlFormat::Anthropic => {
                let system = messages.iter().find(|m| m.role == Role::System).map(|m| m.content.clone());
                let mut anthropic_messages = Vec::new();

                for msg in messages {
                    if msg.role == Role::System {
                        continue;
                    }
                    let role = match msg.role {
                        Role::User | Role::Tool => "user",
                        Role::Assistant => "assistant",
                        _ => "user",
                    };
                    anthropic_messages.push(AnthropicMessage {
                        role: role.to_string(),
                        content: msg.content.clone(),
                    });
                }

                let sample = AnthropicSample {
                    system,
                    messages: anthropic_messages,
                    metadata: self.options.extra_metadata.clone(),
                };
                let json = serde_json::to_string(&sample)?;
                Ok(Some(json))
            }

            JsonlFormat::PromptCompletion => {
                let mut prompt_parts = Vec::new();
                let mut completion = String::new();

                for (idx, msg) in messages.iter().enumerate() {
                    if idx == messages.len() - 1 && msg.role == Role::Assistant {
                        completion = msg.content.clone();
                    } else {
                        let role_tag = match msg.role {
                            Role::System => "### System:\n",
                            Role::User => "### Human:\n",
                            Role::Assistant => "### Assistant:\n",
                            Role::Tool => "### Tool Result:\n",
                        };
                        prompt_parts.push(format!("{}{}\n", role_tag, msg.content));
                    }
                }
                prompt_parts.push("### Assistant:\n".to_string());

                let sample = PromptCompletionSample {
                    prompt: prompt_parts.concat(),
                    completion,
                    metadata: self.options.extra_metadata.clone(),
                };
                let json = serde_json::to_string(&sample)?;
                Ok(Some(json))
            }

            JsonlFormat::PreferenceDpo => {
                let mut prompt_history = Vec::new();
                let mut chosen = String::new();

                for (idx, msg) in messages.iter().enumerate() {
                    if idx == messages.len() - 1 && msg.role == Role::Assistant {
                        chosen = msg.content.clone();
                    } else {
                        prompt_history.push(format!("{}: {}", role_to_str(msg.role), msg.content));
                    }
                }

                let sample = DpoPreferenceSample {
                    prompt: prompt_history.join("\n\n"),
                    chosen,
                    rejected: String::new(), // Placeholder for pairing during DPO curation
                    system: session.system_prompt().map(|s| s.to_string()),
                    metadata: self.options.extra_metadata.clone(),
                };
                let json = serde_json::to_string(&sample)?;
                Ok(Some(json))
            }

            JsonlFormat::LlmEvaluation => {
                let mut eval_messages = Vec::new();
                let mut expected_response = String::new();

                for (idx, msg) in messages.iter().enumerate() {
                    if idx == messages.len() - 1 && msg.role == Role::Assistant {
                        expected_response = msg.content.clone();
                    } else {
                        eval_messages.push(OpenAiExportMessage {
                            role: role_to_str(msg.role).to_string(),
                            content: msg.content.clone(),
                            name: msg.name.clone(),
                            tool_calls: None,
                            tool_call_id: None,
                        });
                    }
                }

                let sample = LlmEvaluationSample {
                    id: format!("{}-turn-{}", session.id, messages.len()),
                    session_id: session.id.to_string(),
                    turn_index: messages.len(),
                    system_prompt: session.system_prompt().map(|s| s.to_string()),
                    messages: eval_messages,
                    expected_response,
                    model: session.active_model.clone(),
                    metadata: self.options.extra_metadata.clone(),
                };
                let json = serde_json::to_string(&sample)?;
                Ok(Some(json))
            }

            JsonlFormat::RawTurns => {
                let mut out_lines = Vec::new();
                for (idx, msg) in messages.iter().enumerate() {
                    let has_tool = msg.tool_calls.as_ref().map_or(false, |c| !c.is_empty());
                    let tool_count = msg.tool_calls.as_ref().map_or(0, |c| c.len());
                    let sample = RawTurnSample {
                        session_id: session.id.to_string(),
                        turn_index: idx,
                        role: role_to_str(msg.role).to_string(),
                        content: msg.content.clone(),
                        has_tool_calls: has_tool,
                        tool_call_count: tool_count,
                        estimated_tokens: estimate_token_count(&msg.content),
                        timestamp: session.updated_at.clone(),
                        model: session.active_model.clone(),
                        metadata: self.options.extra_metadata.clone(),
                    };
                    out_lines.push(serde_json::to_string(&sample)?);
                }
                Ok(Some(out_lines.join("\n")))
            }
        }
    }
}

// ============================================================================
// Dataset Splitter
// ============================================================================

/// Configuration for partitioning a dataset into train / validation / test files.
#[derive(Debug, Clone)]
pub struct DatasetSplitter {
    /// Fraction allocated to training set (e.g. 0.8).
    pub train_ratio: f64,
    /// Fraction allocated to validation set (e.g. 0.1).
    pub val_ratio: f64,
    /// Fraction allocated to testing set (e.g. 0.1).
    pub test_ratio: f64,
    /// Deterministic seed for reproducible dataset splits.
    pub seed: u64,
}

impl Default for DatasetSplitter {
    fn default() -> Self {
        Self {
            train_ratio: 0.8,
            val_ratio: 0.1,
            test_ratio: 0.1,
            seed: 42,
        }
    }
}

impl DatasetSplitter {
    /// Creates a new `DatasetSplitter` with the specified ratios (must sum to ~1.0).
    pub fn new(train_ratio: f64, val_ratio: f64, test_ratio: f64) -> Result<Self, JsonlExportError> {
        let sum = train_ratio + val_ratio + test_ratio;
        if (sum - 1.0).abs() > 0.001 {
            return Err(JsonlExportError::InvalidConfiguration(format!(
                "Split ratios must sum to 1.0 (got train: {}, val: {}, test: {}, sum: {})",
                train_ratio, val_ratio, test_ratio, sum
            )));
        }
        Ok(Self {
            train_ratio,
            val_ratio,
            test_ratio,
            seed: 42,
        })
    }

    /// Sets the pseudo-random seed for deterministic partitioning.
    pub fn with_seed(mut self, seed: u64) -> Self {
        self.seed = seed;
        self
    }

    /// Splits and writes a collection of sessions into `train.jsonl`, `val.jsonl`, and optionally `test.jsonl`.
    pub fn split_and_export(
        &self,
        sessions: &[Session],
        output_dir: impl AsRef<Path>,
        options: &JsonlExportOptions,
    ) -> Result<DatasetSplitResult, JsonlExportError> {
        let output_dir = output_dir.as_ref();
        fs::create_dir_all(output_dir)?;

        let exporter = JsonlExporter::with_options(options.clone());
        let mut train_lines = Vec::new();
        let mut val_lines = Vec::new();
        let mut test_lines = Vec::new();

        let mut total_stats = ExportStats {
            total_sessions: sessions.len(),
            format: options.format,
            ..Default::default()
        };

        for (idx, session) in sessions.iter().enumerate() {
            let (lines, stats) = exporter.process_session(session)?;
            if lines.is_empty() {
                total_stats.skipped_sessions += 1;
                continue;
            }

            total_stats.total_messages += stats.total_messages;
            total_stats.total_estimated_tokens += stats.total_estimated_tokens;
            total_stats.total_tool_calls += stats.total_tool_calls;

            // Deterministic hash assignment
            let score = hash_split_score(session.id.as_bytes(), self.seed, idx);
            if score < self.train_ratio {
                train_lines.extend(lines);
            } else if score < (self.train_ratio + self.val_ratio) {
                val_lines.extend(lines);
            } else {
                test_lines.extend(lines);
            }
        }

        total_stats.exported_samples = train_lines.len() + val_lines.len() + test_lines.len();
        if total_stats.exported_samples > 0 {
            total_stats.avg_tokens_per_sample =
                total_stats.total_estimated_tokens as f64 / total_stats.exported_samples as f64;
        }

        let train_path = output_dir.join("train.jsonl");
        let val_path = output_dir.join("val.jsonl");

        fs::write(&train_path, train_lines.join("\n") + "\n")?;
        fs::write(&val_path, val_lines.join("\n") + "\n")?;

        let test_path_str = if self.test_ratio > 0.0 && !test_lines.is_empty() {
            let test_path = output_dir.join("test.jsonl");
            fs::write(&test_path, test_lines.join("\n") + "\n")?;
            Some(test_path.to_string_lossy().to_string())
        } else {
            None
        };

        Ok(DatasetSplitResult {
            train_count: train_lines.len(),
            val_count: val_lines.len(),
            test_count: test_lines.len(),
            train_path: train_path.to_string_lossy().to_string(),
            val_path: val_path.to_string_lossy().to_string(),
            test_path: test_path_str,
            stats: total_stats,
        })
    }
}

// ============================================================================
// Format Validation
// ============================================================================

/// Fine-tuning dataset validation report.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ValidationReport {
    /// Number of valid JSONL lines.
    pub valid_lines: usize,
    /// Number of invalid JSONL lines.
    pub invalid_lines: usize,
    /// List of fatal validation error messages.
    pub errors: Vec<String>,
    /// List of non-fatal warnings (e.g. excessive context length).
    pub warnings: Vec<String>,
    /// Total estimated tokens across valid samples.
    pub total_tokens: usize,
}

impl ValidationReport {
    /// Returns `true` if the dataset passed validation with zero fatal errors.
    pub fn is_valid(&self) -> bool {
        self.invalid_lines == 0 && self.errors.is_empty()
    }

    /// Formats a concise validation summary.
    pub fn summary(&self) -> String {
        format!(
            "Validation result: {} valid, {} invalid lines. Errors: {}, Warnings: {}, Tokens: ~{}",
            self.valid_lines,
            self.invalid_lines,
            self.errors.len(),
            self.warnings.len(),
            self.total_tokens
        )
    }
}

/// Validates a raw JSONL string against target format rules.
pub fn validate_jsonl_string(jsonl: &str, format: JsonlFormat) -> ValidationReport {
    let mut report = ValidationReport::default();

    for (line_idx, line) in jsonl.lines().enumerate() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }

        match format {
            JsonlFormat::OpenAiChat | JsonlFormat::OpenAiToolCalling => {
                match serde_json::from_str::<OpenAiChatSample>(trimmed) {
                    Ok(sample) => {
                        if let Err(err) = validate_openai_chat_sample(&sample) {
                            report.invalid_lines += 1;
                            report.errors.push(format!("Line {}: {}", line_idx + 1, err));
                        } else {
                            report.valid_lines += 1;
                            let tokens = estimate_token_count(trimmed);
                            report.total_tokens += tokens;
                            if tokens > 32768 {
                                report.warnings.push(format!(
                                    "Line {} has high estimated token count (~{} tokens)",
                                    line_idx + 1,
                                    tokens
                                ));
                            }
                        }
                    }
                    Err(err) => {
                        report.invalid_lines += 1;
                        report.errors.push(format!(
                            "Line {} is not valid OpenAI Chat JSON: {}",
                            line_idx + 1,
                            err
                        ));
                    }
                }
            }
            JsonlFormat::ShareGpt => {
                match serde_json::from_str::<ShareGptSample>(trimmed) {
                    Ok(sample) => {
                        if sample.conversations.is_empty() {
                            report.invalid_lines += 1;
                            report.errors.push(format!("Line {}: empty conversations array", line_idx + 1));
                        } else {
                            report.valid_lines += 1;
                            report.total_tokens += estimate_token_count(trimmed);
                        }
                    }
                    Err(err) => {
                        report.invalid_lines += 1;
                        report.errors.push(format!("Line {} is not valid ShareGPT JSON: {}", line_idx + 1, err));
                    }
                }
            }
            JsonlFormat::Alpaca => {
                match serde_json::from_str::<AlpacaSample>(trimmed) {
                    Ok(sample) => {
                        if sample.instruction.trim().is_empty() || sample.output.trim().is_empty() {
                            report.invalid_lines += 1;
                            report.errors.push(format!("Line {}: instruction or output is empty", line_idx + 1));
                        } else {
                            report.valid_lines += 1;
                            report.total_tokens += estimate_token_count(trimmed);
                        }
                    }
                    Err(err) => {
                        report.invalid_lines += 1;
                        report.errors.push(format!("Line {} is not valid Alpaca JSON: {}", line_idx + 1, err));
                    }
                }
            }
            JsonlFormat::Anthropic => {
                match serde_json::from_str::<AnthropicSample>(trimmed) {
                    Ok(sample) => {
                        if sample.messages.is_empty() {
                            report.invalid_lines += 1;
                            report.errors.push(format!("Line {}: empty messages array", line_idx + 1));
                        } else {
                            report.valid_lines += 1;
                            report.total_tokens += estimate_token_count(trimmed);
                        }
                    }
                    Err(err) => {
                        report.invalid_lines += 1;
                        report.errors.push(format!("Line {} is not valid Anthropic JSON: {}", line_idx + 1, err));
                    }
                }
            }
            JsonlFormat::PromptCompletion => {
                match serde_json::from_str::<PromptCompletionSample>(trimmed) {
                    Ok(sample) => {
                        if sample.prompt.trim().is_empty() || sample.completion.trim().is_empty() {
                            report.invalid_lines += 1;
                            report.errors.push(format!("Line {}: prompt or completion is empty", line_idx + 1));
                        } else {
                            report.valid_lines += 1;
                            report.total_tokens += estimate_token_count(trimmed);
                        }
                    }
                    Err(err) => {
                        report.invalid_lines += 1;
                        report.errors.push(format!("Line {} is not valid Prompt-Completion JSON: {}", line_idx + 1, err));
                    }
                }
            }
            JsonlFormat::PreferenceDpo => {
                match serde_json::from_str::<DpoPreferenceSample>(trimmed) {
                    Ok(sample) => {
                        if sample.prompt.trim().is_empty() || sample.chosen.trim().is_empty() {
                            report.invalid_lines += 1;
                            report.errors.push(format!("Line {}: prompt or chosen response is empty", line_idx + 1));
                        } else {
                            report.valid_lines += 1;
                            report.total_tokens += estimate_token_count(trimmed);
                        }
                    }
                    Err(err) => {
                        report.invalid_lines += 1;
                        report.errors.push(format!("Line {} is not valid DPO JSON: {}", line_idx + 1, err));
                    }
                }
            }
            JsonlFormat::LlmEvaluation => {
                match serde_json::from_str::<LlmEvaluationSample>(trimmed) {
                    Ok(sample) => {
                        if sample.id.is_empty() || sample.expected_response.is_empty() {
                            report.invalid_lines += 1;
                            report.errors.push(format!("Line {}: missing id or expected_response", line_idx + 1));
                        } else {
                            report.valid_lines += 1;
                            report.total_tokens += estimate_token_count(trimmed);
                        }
                    }
                    Err(err) => {
                        report.invalid_lines += 1;
                        report.errors.push(format!("Line {} is not valid LLM Evaluation JSON: {}", line_idx + 1, err));
                    }
                }
            }
            JsonlFormat::RawTurns => {
                match serde_json::from_str::<RawTurnSample>(trimmed) {
                    Ok(_) => {
                        report.valid_lines += 1;
                        report.total_tokens += estimate_token_count(trimmed);
                    }
                    Err(err) => {
                        report.invalid_lines += 1;
                        report.errors.push(format!("Line {} is not valid RawTurn JSON: {}", line_idx + 1, err));
                    }
                }
            }
        }
    }

    report
}

/// Validates a JSONL file on disk.
pub fn validate_jsonl_file(path: impl AsRef<Path>, format: JsonlFormat) -> Result<ValidationReport, JsonlExportError> {
    let file = fs::File::open(path)?;
    let reader = BufReader::new(file);
    let mut content = String::new();
    for line in reader.lines() {
        content.push_str(&line?);
        content.push('\n');
    }
    Ok(validate_jsonl_string(&content, format))
}

/// Validates an individual OpenAI chat sample against OpenAI fine-tuning rules.
fn validate_openai_chat_sample(sample: &OpenAiChatSample) -> Result<(), JsonlExportError> {
    if sample.messages.is_empty() {
        return Err(JsonlExportError::Validation(
            "Messages array cannot be empty".to_string(),
        ));
    }

    let mut has_user = false;
    let mut has_assistant = false;

    for (idx, msg) in sample.messages.iter().enumerate() {
        match msg.role.as_str() {
            "system" => {
                if idx != 0 {
                    // OpenAI advises system message to be at index 0
                }
            }
            "user" => {
                has_user = true;
            }
            "assistant" => {
                has_assistant = true;
                if msg.content.trim().is_empty() && msg.tool_calls.is_none() {
                    return Err(JsonlExportError::Validation(format!(
                        "Assistant message at index {} has empty content and no tool calls",
                        idx
                    )));
                }
            }
            "tool" => {
                if msg.tool_call_id.is_none() {
                    return Err(JsonlExportError::Validation(format!(
                        "Tool message at index {} missing tool_call_id",
                        idx
                    )));
                }
            }
            other => {
                return Err(JsonlExportError::Validation(format!(
                    "Unrecognized message role '{}' at index {}",
                    other, idx
                )));
            }
        }
    }

    if !has_user {
        return Err(JsonlExportError::Validation(
            "Sample must contain at least one user message".to_string(),
        ));
    }
    if !has_assistant {
        return Err(JsonlExportError::Validation(
            "Sample must contain at least one assistant message".to_string(),
        ));
    }

    Ok(())
}

// ============================================================================
// Helper & Utility Functions
// ============================================================================

/// Exports a single session to a JSONL string with default options.
pub fn export_session_to_jsonl(session: &Session, options: &JsonlExportOptions) -> Result<String, JsonlExportError> {
    JsonlExporter::with_options(options.clone()).export_session(session)
}

/// Exports multiple sessions to a single JSONL string with options.
pub fn export_sessions_to_jsonl(
    sessions: &[Session],
    options: &JsonlExportOptions,
) -> Result<(String, ExportStats), JsonlExportError> {
    JsonlExporter::with_options(options.clone()).export_sessions(sessions)
}

/// Exports a single session to a JSONL file on disk.
pub fn export_session_to_jsonl_file(
    session: &Session,
    path: impl AsRef<Path>,
    options: &JsonlExportOptions,
) -> Result<ExportStats, JsonlExportError> {
    JsonlExporter::with_options(options.clone()).export_to_file(&[session.clone()], path)
}

/// Exports multiple sessions to a JSONL file on disk.
pub fn export_sessions_to_jsonl_file(
    sessions: &[Session],
    path: impl AsRef<Path>,
    options: &JsonlExportOptions,
) -> Result<ExportStats, JsonlExportError> {
    JsonlExporter::with_options(options.clone()).export_to_file(sessions, path)
}

/// Helper function to partition a dataset and write split files into an output directory.
pub fn export_dataset_split(
    sessions: &[Session],
    output_dir: impl AsRef<Path>,
    options: &JsonlExportOptions,
    splitter: &DatasetSplitter,
) -> Result<DatasetSplitResult, JsonlExportError> {
    splitter.split_and_export(sessions, output_dir, options)
}

/// Maps internal `Role` enum to standard lowercase string.
pub fn role_to_str(role: Role) -> &'static str {
    match role {
        Role::System => "system",
        Role::User => "user",
        Role::Assistant => "assistant",
        Role::Tool => "tool",
    }
}

/// Estimates token count using a fast character heuristic (~3.8 characters per token).
pub fn estimate_token_count(text: &str) -> usize {
    if text.is_empty() {
        return 0;
    }
    // Heuristic: ~3.8 chars per token + basic whitespace tokenization consideration
    let char_count = text.chars().count();
    let word_count = text.split_whitespace().count();
    let token_est = (char_count as f64 / 3.8).max(word_count as f64 * 1.3);
    token_est.ceil() as usize
}

/// Counts total tool calls in a slice of messages.
pub fn count_tool_calls(messages: &[Message]) -> usize {
    messages
        .iter()
        .map(|m| m.tool_calls.as_ref().map_or(0, |calls| calls.len()))
        .sum()
}

/// Strips `<think>...</think>` or `<thought>...</thought>` blocks from text.
pub fn strip_thought_tags(text: &str) -> String {
    let re = Regex::new(r"(?s)<(think|thought)>.*?</(think|thought)>").unwrap();
    let stripped = re.replace_all(text, "");
    stripped.trim().to_string()
}

/// Extracts reasoning text from `<think>...</think>` blocks and returns `(clean_content, reasoning)`.
pub fn extract_thought_blocks(text: &str) -> Option<(String, String)> {
    let re = Regex::new(r"(?s)<(think|thought)>(.*?)</(think|thought)>").unwrap();
    let mut reasoning_parts = Vec::new();

    for caps in re.captures_iter(text) {
        if let Some(matched) = caps.get(2) {
            reasoning_parts.push(matched.as_str().trim().to_string());
        }
    }

    if reasoning_parts.is_empty() {
        None
    } else {
        let clean = re.replace_all(text, "").trim().to_string();
        let reasoning = reasoning_parts.join("\n\n");
        Some((clean, reasoning))
    }
}

/// Masks API keys, secret tokens, and credentials in text.
pub fn mask_sensitive_credentials(text: &str) -> String {
    let mut result = text.to_string();

    // Mask OpenAI keys
    let openai_re = Regex::new(r"sk-[a-zA-Z0-9]{20,}").unwrap();
    result = openai_re.replace_all(&result, "sk-[REDACTED_API_KEY]").to_string();

    // Mask Anthropic keys
    let anthropic_re = Regex::new(r"sk-ant-[a-zA-Z0-9_\-]{20,}").unwrap();
    result = anthropic_re.replace_all(&result, "sk-ant-[REDACTED_API_KEY]").to_string();

    // Mask GitHub tokens
    let github_re = Regex::new(r"gh[pousr]_[a-zA-Z0-9]{20,}").unwrap();
    result = github_re.replace_all(&result, "ghp_[REDACTED_TOKEN]").to_string();

    // Mask Bearer tokens
    let bearer_re = Regex::new(r"(?i)Bearer\s+[a-zA-Z0-9_\-\.]{20,}").unwrap();
    result = bearer_re.replace_all(&result, "Bearer [REDACTED_TOKEN]").to_string();

    // Mask generic secret/key assignments
    let secret_re = Regex::new(r#"(?i)(api[_-]?key|secret|password|auth_token)\s*[:=]\s*["']([^"']{8,})["']"#).unwrap();
    result = secret_re.replace_all(&result, "$1=\"[REDACTED]\"").to_string();

    result
}

/// Deterministic float in range [0.0, 1.0) for dataset hash partitioning.
fn hash_split_score(bytes: &[u8], seed: u64, salt: usize) -> f64 {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::Hasher;

    let mut hasher = DefaultHasher::new();
    hasher.write(bytes);
    hasher.write_u64(seed);
    hasher.write_usize(salt);
    let hash = hasher.finish();

    (hash as f64) / (u64::MAX as f64)
}

// ============================================================================
// Unit Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::session::Session;
    use crate::provider::types::{Message, ToolCall};
    use tempfile::tempdir;

    fn create_sample_session() -> Session {
        let mut session = Session::new("gpt-4o");
        session.set_title("Code Refactoring Task");
        session.system_prompt = Some("You are an expert Rust programming assistant.".to_string());
        session.messages.push(Message::user("How do I parse JSON in Rust?"));
        session.messages.push(Message::assistant(
            "<think>The user is asking about JSON parsing in Rust. Serde JSON is the standard.</think>Use `serde_json::from_str` with a struct implementing `Deserialize`.",
        ));
        session.messages.push(Message::user("Can you show a code example?"));
        session.messages.push(Message::assistant(
            "```rust\nuse serde::Deserialize;\n\n#[derive(Deserialize)]\nstruct Person {\n    name: String,\n}\n```",
        ));
        session
    }

    fn create_tool_session() -> Session {
        let mut session = Session::new("gpt-4o");
        session.messages.push(Message::user("Check the weather in Tokyo."));
        session.messages.push(Message::assistant_with_tools(
            "Checking weather now...",
            vec![ToolCall {
                id: "call_tokyo_123".to_string(),
                name: "get_weather".to_string(),
                arguments: r#"{"city": "Tokyo"}"#.to_string(),
            }],
        ));
        session.messages.push(Message::tool_result(
            "call_tokyo_123",
            r#"{"temp_c": 22, "condition": "Sunny"}"#,
        ));
        session.messages.push(Message::assistant("The weather in Tokyo is currently sunny and 22°C."));
        session
    }

    #[test]
    fn test_export_openai_chat_format() {
        let session = create_sample_session();
        let options = JsonlExportOptions::new(JsonlFormat::OpenAiChat);
        let jsonl = export_session_to_jsonl(&session, &options).expect("Export failed");

        assert!(!jsonl.is_empty());
        let parsed: OpenAiChatSample = serde_json::from_str(&jsonl).expect("Failed to parse JSON");
        assert_eq!(parsed.messages.len(), 5); // 1 system + 4 conversation turns
        assert_eq!(parsed.messages[0].role, "system");
        assert_eq!(parsed.messages[1].role, "user");
        assert_eq!(parsed.messages[2].role, "assistant");
    }

    #[test]
    fn test_thought_handling_strip() {
        let session = create_sample_session();
        let options = JsonlExportOptions::new(JsonlFormat::OpenAiChat)
            .with_thought_handling(ThoughtHandling::Strip);
        let jsonl = export_session_to_jsonl(&session, &options).expect("Export failed");

        assert!(!jsonl.contains("<think>"));
        assert!(!jsonl.contains("The user is asking about JSON"));
        assert!(jsonl.contains("Use `serde_json::from_str`"));
    }

    #[test]
    fn test_thought_handling_extract() {
        let session = create_sample_session();
        let options = JsonlExportOptions::new(JsonlFormat::OpenAiChat)
            .with_thought_handling(ThoughtHandling::ExtractToField);
        let jsonl = export_session_to_jsonl(&session, &options).expect("Export failed");

        let parsed: OpenAiChatSample = serde_json::from_str(&jsonl).expect("Failed to parse JSON");
        assert!(parsed.reasoning.is_some());
        assert!(parsed.reasoning.unwrap().contains("Serde JSON is the standard"));
    }

    #[test]
    fn test_export_sharegpt_format() {
        let session = create_sample_session();
        let options = JsonlExportOptions::new(JsonlFormat::ShareGpt);
        let jsonl = export_session_to_jsonl(&session, &options).expect("Export failed");

        let parsed: ShareGptSample = serde_json::from_str(&jsonl).expect("Failed to parse ShareGPT JSON");
        assert_eq!(parsed.conversations.len(), 5);
        assert_eq!(parsed.conversations[0].from, "system");
        assert_eq!(parsed.conversations[1].from, "human");
        assert_eq!(parsed.conversations[2].from, "gpt");
    }

    #[test]
    fn test_export_alpaca_format() {
        let session = create_sample_session();
        let options = JsonlExportOptions::new(JsonlFormat::Alpaca);
        let jsonl = export_session_to_jsonl(&session, &options).expect("Export failed");

        let parsed: AlpacaSample = serde_json::from_str(&jsonl).expect("Failed to parse Alpaca JSON");
        assert_eq!(parsed.instruction, "Can you show a code example?");
        assert!(parsed.output.contains("struct Person"));
        assert!(parsed.system.is_some());
    }

    #[test]
    fn test_export_anthropic_format() {
        let session = create_sample_session();
        let options = JsonlExportOptions::new(JsonlFormat::Anthropic);
        let jsonl = export_session_to_jsonl(&session, &options).expect("Export failed");

        let parsed: AnthropicSample = serde_json::from_str(&jsonl).expect("Failed to parse Anthropic JSON");
        assert!(parsed.system.is_some());
        assert_eq!(parsed.messages.len(), 4);
    }

    #[test]
    fn test_export_prompt_completion_format() {
        let session = create_sample_session();
        let options = JsonlExportOptions::new(JsonlFormat::PromptCompletion);
        let jsonl = export_session_to_jsonl(&session, &options).expect("Export failed");

        let parsed: PromptCompletionSample =
            serde_json::from_str(&jsonl).expect("Failed to parse PromptCompletion JSON");
        assert!(parsed.prompt.contains("### Human:\nHow do I parse JSON in Rust?"));
        assert!(parsed.completion.contains("struct Person"));
    }

    #[test]
    fn test_export_llm_evaluation_format() {
        let session = create_sample_session();
        let options = JsonlExportOptions::new(JsonlFormat::LlmEvaluation);
        let jsonl = export_session_to_jsonl(&session, &options).expect("Export failed");

        let parsed: LlmEvaluationSample =
            serde_json::from_str(&jsonl).expect("Failed to parse LlmEvaluation JSON");
        assert_eq!(parsed.model, "gpt-4o");
        assert!(parsed.expected_response.contains("struct Person"));
    }

    #[test]
    fn test_sliding_window_every_assistant_turn() {
        let session = create_sample_session();
        let options = JsonlExportOptions::new(JsonlFormat::OpenAiChat)
            .with_split_strategy(TurnSplitStrategy::EveryAssistantTurn);
        let jsonl = export_session_to_jsonl(&session, &options).expect("Export failed");

        let lines: Vec<&str> = jsonl.lines().collect();
        assert_eq!(lines.len(), 2); // 2 assistant turns = 2 samples

        let first: OpenAiChatSample = serde_json::from_str(lines[0]).unwrap();
        assert_eq!(first.messages.len(), 3); // sys + user1 + asst1

        let second: OpenAiChatSample = serde_json::from_str(lines[1]).unwrap();
        assert_eq!(second.messages.len(), 5); // sys + user1 + asst1 + user2 + asst2
    }

    #[test]
    fn test_tool_calling_export() {
        let session = create_tool_session();
        let options = JsonlExportOptions::new(JsonlFormat::OpenAiChat);
        let jsonl = export_session_to_jsonl(&session, &options).expect("Export failed");

        let parsed: OpenAiChatSample = serde_json::from_str(&jsonl).expect("Failed to parse JSON");
        assert_eq!(parsed.messages.len(), 4);
        let asst_tool = &parsed.messages[1];
        assert!(asst_tool.tool_calls.is_some());
        let tc = &asst_tool.tool_calls.as_ref().unwrap()[0];
        assert_eq!(tc.function.name, "get_weather");
        assert_eq!(parsed.messages[2].role, "tool");
        assert_eq!(parsed.messages[2].tool_call_id.as_deref(), Some("call_tokyo_123"));
    }

    #[test]
    fn test_flatten_tool_calls_to_text() {
        let session = create_tool_session();
        let options = JsonlExportOptions::new(JsonlFormat::OpenAiChat).with_flattened_tools(true);
        let jsonl = export_session_to_jsonl(&session, &options).expect("Export failed");

        let parsed: OpenAiChatSample = serde_json::from_str(&jsonl).expect("Failed to parse JSON");
        assert!(parsed.messages[1].content.contains("<tool_call>{\"name\": \"get_weather\""));
        assert!(parsed.messages[2].content.contains("<tool_result id=\"call_tokyo_123\">"));
        assert_eq!(parsed.messages[2].role, "user");
    }

    #[test]
    fn test_raw_turns_format() {
        let session = create_sample_session();
        let options = JsonlExportOptions::new(JsonlFormat::RawTurns);
        let jsonl = export_session_to_jsonl(&session, &options).expect("Export failed");

        let lines: Vec<&str> = jsonl.lines().collect();
        assert_eq!(lines.len(), 5);
        let first: RawTurnSample = serde_json::from_str(lines[0]).expect("Parse RawTurn failed");
        assert_eq!(first.role, "system");
        assert_eq!(first.turn_index, 0);
    }

    #[test]
    fn test_dpo_preference_format() {
        let session = create_sample_session();
        let options = JsonlExportOptions::new(JsonlFormat::PreferenceDpo);
        let jsonl = export_session_to_jsonl(&session, &options).expect("Export failed");

        let parsed: DpoPreferenceSample =
            serde_json::from_str(&jsonl).expect("Parse DPO failed");
        assert!(parsed.prompt.contains("How do I parse JSON in Rust?"));
        assert!(parsed.chosen.contains("struct Person"));
    }

    #[test]
    fn test_role_filtering() {
        let session = create_sample_session();
        let options = JsonlExportOptions::new(JsonlFormat::OpenAiChat)
            .with_system_messages(false);
        let jsonl = export_session_to_jsonl(&session, &options).expect("Export failed");

        let parsed: OpenAiChatSample = serde_json::from_str(&jsonl).expect("Parse failed");
        assert_eq!(parsed.messages.len(), 4);
        assert_eq!(parsed.messages[0].role, "user");
    }
    #[test]
    fn test_sensitive_credential_masking() {
        let text = "Here is my OpenAI key sk-1234567890abcdef1234567890 and github token ghp_abcdefghijklmnopqrstuvwx and api_key=\"secret_password_123\"";
        let masked = mask_sensitive_credentials(text);

        assert!(!masked.contains("sk-1234567890abcdef1234567890"));
        assert!(masked.contains("sk-[REDACTED_API_KEY]"));
        assert!(!masked.contains("ghp_abcdefghijklmnopqrstuvwx"));
        assert!(masked.contains("ghp_[REDACTED_TOKEN]"));
        assert!(!masked.contains("secret_password_123"));
    }

    #[test]
    fn test_validation_report() {
        let valid_jsonl = r#"{"messages": [{"role": "user", "content": "Hello"}, {"role": "assistant", "content": "Hi there!"}]}"#;
        let report = validate_jsonl_string(valid_jsonl, JsonlFormat::OpenAiChat);
        assert!(report.is_valid());
        assert_eq!(report.valid_lines, 1);
        assert_eq!(report.invalid_lines, 0);

        let invalid_jsonl = r#"{"messages": [{"role": "user", "content": "Hello"}]}"#; // Missing assistant
        let report_invalid = validate_jsonl_string(invalid_jsonl, JsonlFormat::OpenAiChat);
        assert!(!report_invalid.is_valid());
        assert_eq!(report_invalid.invalid_lines, 1);
    }

    #[test]
    fn test_dataset_splitter() {
        let mut sessions = Vec::new();
        for i in 0..10 {
            let mut s = Session::new("gpt-4o");
            s.messages.push(Message::user(format!("Question {}", i)));
            s.messages.push(Message::assistant(format!("Answer {}", i)));
            sessions.push(s);
        }

        let temp_dir = tempdir().expect("Failed to create tempdir");
        let splitter = DatasetSplitter::new(0.8, 0.2, 0.0).expect("Splitter init failed");
        let options = JsonlExportOptions::new(JsonlFormat::OpenAiChat);

        let result = splitter
            .split_and_export(&sessions, temp_dir.path(), &options)
            .expect("Split failed");

        assert_eq!(result.train_count + result.val_count, 10);
        assert!(Path::new(&result.train_path).exists());
        assert!(Path::new(&result.val_path).exists());
    }

    #[test]
    fn test_file_export_roundtrip() {
        let session = create_sample_session();
        let temp_dir = tempdir().expect("Failed to create tempdir");
        let export_path = temp_dir.path().join("dataset.jsonl");

        let options = JsonlExportOptions::new(JsonlFormat::OpenAiChat);
        let stats = export_session_to_jsonl_file(&session, &export_path, &options)
            .expect("File export failed");

        assert_eq!(stats.exported_samples, 1);
        assert!(export_path.exists());

        let validation = validate_jsonl_file(&export_path, JsonlFormat::OpenAiChat)
            .expect("Validation failed");
        assert!(validation.is_valid());
    }
}
