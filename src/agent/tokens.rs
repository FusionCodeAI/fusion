//! Fast heuristic and exact token estimator for context budgeting, streaming, and compaction.
//!
//! Provides:
//! 1. Model-aware token heuristics (cl100k_base, o200k_base, Claude tokenizer rules, Llama 3, DeepSeek, Gemini).
//! 2. Fast zero-allocation byte scanner approximating Byte Pair Encoding (BPE) behavior (~4 chars/token).
//! 3. Streaming token count estimators for prompt and completion buffers (with reasoning / thinking support).
//! 4. Tool call argument token overhead and detailed schema token calculations.
//! 5. Message and conversation token estimation with role formatting overhead.
//! 6. Exact token counts and tracker when available from provider usage reports.
//! 7. Comprehensive model context limits catalog with up-to-date defaults.
//! 8. Context budget calculations, threshold monitoring, and truncation planning.

use std::collections::HashMap;
use std::fmt;

use serde::{Deserialize, Serialize};

use crate::provider::types::{Message, Role, StreamChunk, ToolCall, ToolDefinition};

/// Average characters per token in standard English / source code (BPE approximation).
pub const APPROX_CHARS_PER_TOKEN: f32 = 4.0;

/// Default context window limit when a model is unrecognized (128k tokens).
pub const DEFAULT_CONTEXT_WINDOW: usize = 128_000;

/// Default safety margin (in tokens) subtracted from context window to prevent edge overflows.
pub const DEFAULT_SAFETY_MARGIN: usize = 1_024;

/// Default completion token reserve.
pub const DEFAULT_RESERVED_COMPLETION: usize = 4_096;

/// Default warning threshold (80% of context window).
pub const DEFAULT_WARNING_THRESHOLD: f32 = 0.80;

/// Default danger threshold (95% of context window).
pub const DEFAULT_DANGER_THRESHOLD: f32 = 0.95;

/// Overhead tokens per message in OpenAI / Anthropic chat formats:
/// `<|im_start|>{role}\n{content}<|im_end|>\n`
pub const MESSAGE_BASE_TOKENS: usize = 4;

/// Prime tokens added to prompt for assistant turn initiation (`<|im_start|>assistant\n`).
pub const ASSISTANT_PRIMING_TOKENS: usize = 3;

// ---------------------------------------------------------------------------
// 1. Tokenizer Family Classification
// ---------------------------------------------------------------------------

/// Categorizes LLM tokenizers to apply family-specific BPE rules, vocab densities,
/// and message/tool framing overheads.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum TokenizerFamily {
    /// OpenAI cl100k_base (GPT-4, GPT-4-turbo, GPT-3.5-turbo, text-embedding-ada-002)
    Cl100kBase,
    /// OpenAI o200k_base (GPT-4o, GPT-4o-mini, o1, o3, o3-mini)
    O200kBase,
    /// Anthropic Claude tokenizer (Claude 3, 3.5 Sonnet, 3.7 Sonnet, Opus, Haiku)
    Claude,
    /// Meta Llama 3 / 3.1 / 3.2 / 3.3 tokenizer (128k tiktoken-like BPE)
    Llama3,
    /// DeepSeek V2 / V3 / R1 tokenizer (128k bilingual BPE)
    DeepSeek,
    /// Google Gemini tokenizer (SentencePiece 256k vocab)
    Gemini,
    /// Generic BPE heuristic (~4 chars/token fallback)
    GenericBpe,
}

impl TokenizerFamily {
    /// Infers the appropriate tokenizer family from an LLM model identifier string.
    pub fn from_model(model: &str) -> Self {
        let m = model.trim().to_lowercase();

        if m.contains("gpt-4o") || m.starts_with("o1") || m.starts_with("o3") {
            TokenizerFamily::O200kBase
        } else if m.contains("claude") {
            TokenizerFamily::Claude
        } else if m.contains("deepseek") {
            TokenizerFamily::DeepSeek
        } else if m.contains("llama-3") || m.contains("llama3") {
            TokenizerFamily::Llama3
        } else if m.contains("gemini") {
            TokenizerFamily::Gemini
        } else if m.starts_with("gpt-4") || m.starts_with("gpt-3.5") || m.contains("cl100k") {
            TokenizerFamily::Cl100kBase
        } else {
            TokenizerFamily::GenericBpe
        }
    }

    /// Returns the human-readable canonical name of the tokenizer family.
    #[inline]
    pub fn name(&self) -> &'static str {
        match self {
            TokenizerFamily::Cl100kBase => "cl100k_base",
            TokenizerFamily::O200kBase => "o200k_base",
            TokenizerFamily::Claude => "claude",
            TokenizerFamily::Llama3 => "llama3",
            TokenizerFamily::DeepSeek => "deepseek",
            TokenizerFamily::Gemini => "gemini",
            TokenizerFamily::GenericBpe => "generic_bpe",
        }
    }

    /// Approximate average characters per token for English / standard prose.
    #[inline]
    pub fn approx_chars_per_token(&self) -> f32 {
        match self {
            TokenizerFamily::Cl100kBase => 4.0,
            TokenizerFamily::O200kBase => 4.2,
            TokenizerFamily::Claude => 3.5,
            TokenizerFamily::Llama3 => 4.0,
            TokenizerFamily::DeepSeek => 3.8,
            TokenizerFamily::Gemini => 4.0,
            TokenizerFamily::GenericBpe => 4.0,
        }
    }

    /// Base framing tokens per chat message in this family's protocol.
    #[inline]
    pub fn message_base_tokens(&self) -> usize {
        match self {
            TokenizerFamily::Claude => 3, // \n\nHuman: / \n\nAssistant:
            TokenizerFamily::O200kBase | TokenizerFamily::Cl100kBase => 4, // <|im_start|>{role}\n{content}<|im_end|>\n
            TokenizerFamily::Llama3 => 4, // <|start_header_id|>{role}<|end_header_id|>\n\n{content}<|eot_id|>
            TokenizerFamily::DeepSeek => 4,
            TokenizerFamily::Gemini => 3,
            TokenizerFamily::GenericBpe => MESSAGE_BASE_TOKENS,
        }
    }

    /// Priming tokens added to initiate assistant turn generation.
    #[inline]
    pub fn assistant_priming_tokens(&self) -> usize {
        match self {
            TokenizerFamily::Claude => 2, // \n\nAssistant:
            TokenizerFamily::O200kBase | TokenizerFamily::Cl100kBase => 3, // <|im_start|>assistant\n
            TokenizerFamily::Llama3 => 3, // <|start_header_id|>assistant<|end_header_id|>\n\n
            TokenizerFamily::DeepSeek => 3,
            TokenizerFamily::Gemini => 2,
            TokenizerFamily::GenericBpe => ASSISTANT_PRIMING_TOKENS,
        }
    }

    /// Base structural overhead tokens for assistant tool call invocations.
    #[inline]
    pub fn tool_call_overhead(&self) -> usize {
        match self {
            TokenizerFamily::Claude => 6, // <tool_use><id>...</id><name>...</name><arguments>...</arguments></tool_use>
            TokenizerFamily::O200kBase => 3,
            TokenizerFamily::Cl100kBase => 4,
            TokenizerFamily::Llama3 => 5,
            TokenizerFamily::DeepSeek => 4,
            TokenizerFamily::Gemini => 4,
            TokenizerFamily::GenericBpe => 4,
        }
    }

    /// Base framing overhead tokens when injecting tool definitions into context.
    #[inline]
    pub fn tool_definition_overhead(&self) -> usize {
        match self {
            TokenizerFamily::Claude => 12, // <tools><tool_description>...</tool_description></tools>
            TokenizerFamily::O200kBase => 6,
            TokenizerFamily::Cl100kBase => 8,
            TokenizerFamily::Llama3 => 8,
            TokenizerFamily::DeepSeek => 8,
            TokenizerFamily::Gemini => 8,
            TokenizerFamily::GenericBpe => 8,
        }
    }
}

// ---------------------------------------------------------------------------
// 2. Fast Heuristic & Model-Aware Token Estimator
// ---------------------------------------------------------------------------

/// Fast heuristic estimator approximating Byte Pair Encoding (BPE) behavior (~4 chars/token).
///
/// Designed for high throughput (multi-gigabytes per second) while providing
/// substantially better accuracy than naive `len / 4`:
/// - Indentation / space sequences (1-4 spaces = 1 token, 5-8 = 2 tokens, etc.)
/// - Tab characters (1 token each)
/// - Newlines (1 token each, consecutive newlines accounted for)
/// - CamelCase and snake_case sub-word splitting
/// - Common multi-character code operators (`::`, `->`, `=>`, `==`, `!=`, `<=`, `>=`, `&&`, `||`, etc.)
/// - Multi-byte UTF-8 characters (CJK characters, Cyrillic, accented letters, emojis)
/// - Numbers and numeric groupings
#[inline]
pub fn estimate_text_tokens(text: &str) -> usize {
    estimate_text_tokens_with_family(text, TokenizerFamily::GenericBpe)
}

/// Convenience alias for `estimate_text_tokens`.
#[inline]
pub fn estimate_tokens(text: &str) -> usize {
    estimate_text_tokens(text)
}

/// Estimates token count for a text string using model-aware tokenizer heuristics.
#[inline]
pub fn estimate_text_tokens_for_model(text: &str, model: &str) -> usize {
    let family = TokenizerFamily::from_model(model);
    estimate_text_tokens_with_family(text, family)
}

/// Core zero-allocation token counting engine parameterized by `TokenizerFamily`.
#[inline]
pub fn estimate_text_tokens_with_family(text: &str, family: TokenizerFamily) -> usize {
    if text.is_empty() {
        return 0;
    }

    let bytes = text.as_bytes();
    let len = bytes.len();

    if len <= 3 {
        return 1;
    }

    let mut tokens = 0usize;
    let mut i = 0;

    // Configurable tokenizer parameters based on family
    let (max_spaces_per_token, number_digits_per_token, cjk_token_multiplier) = match family {
        TokenizerFamily::O200kBase => (8usize, 4usize, 1.0f32),
        TokenizerFamily::Cl100kBase => (4usize, 3usize, 2.0f32),
        TokenizerFamily::Claude => (4usize, 3usize, 1.3f32),
        TokenizerFamily::Llama3 => (4usize, 3usize, 1.2f32),
        TokenizerFamily::DeepSeek => (6usize, 3usize, 1.0f32),
        TokenizerFamily::Gemini => (4usize, 3usize, 1.1f32),
        TokenizerFamily::GenericBpe => (4usize, 4usize, 1.0f32),
    };

    while i < len {
        let b = bytes[i];

        // 1. Whitespace: Spaces
        if b == b' ' {
            let mut spaces = 0usize;
            while i < len && bytes[i] == b' ' {
                spaces += 1;
                i += 1;
            }
            // In BPE tokenizers, a single space before a word/token merges into the following token (e.g. " word")
            let has_following_token = i < len && bytes[i] != b'\n' && bytes[i] != b'\r';
            let effective_spaces = if has_following_token && spaces > 0 {
                spaces - 1
            } else {
                spaces
            };

            if effective_spaces > 0 {
                tokens += (effective_spaces + (max_spaces_per_token - 1)) / max_spaces_per_token;
            }
            continue;
        }

        // 2. Whitespace: Tabs
        if b == b'\t' {
            let mut tabs = 0usize;
            while i < len && bytes[i] == b'\t' {
                tabs += 1;
                i += 1;
            }
            tokens += tabs;
            continue;
        }

        // 3. Whitespace: Newlines
        if b == b'\n' || b == b'\r' {
            let mut newlines = 0usize;
            while i < len && (bytes[i] == b'\n' || bytes[i] == b'\r') {
                if bytes[i] == b'\r' && i + 1 < len && bytes[i + 1] == b'\n' {
                    i += 2;
                } else {
                    i += 1;
                }
                newlines += 1;
            }
            // In BPE, 1 or 2 newlines typically collapses to 1 token ("\n" or "\n\n")
            tokens += (newlines + 1) / 2;
            continue;
        }

        // 4. Claude XML tag heuristic (<tag_name> or </tag_name>)
        if family == TokenizerFamily::Claude && b == b'<' && i + 1 < len {
            let is_close = bytes[i + 1] == b'/';
            let tag_start = if is_close { i + 2 } else { i + 1 };
            if tag_start < len && (bytes[tag_start].is_ascii_alphabetic() || bytes[tag_start] == b'_') {
                let mut j = tag_start;
                while j < len && (bytes[j].is_ascii_alphanumeric() || bytes[j] == b'_' || bytes[j] == b'-') {
                    j += 1;
                }
                if j < len && bytes[j] == b'>' {
                    // Whole XML tag matched (<tag> or </tag>) -> 1 token in Claude
                    tokens += 1;
                    i = j + 1;
                    continue;
                }
            }
        }

        // 5. Numbers
        if b.is_ascii_digit() {
            let mut digits = 0usize;
            while i < len && bytes[i].is_ascii_digit() {
                digits += 1;
                i += 1;
            }
            tokens += (digits + (number_digits_per_token - 1)) / number_digits_per_token;
            continue;
        }

        // 6. ASCII Contractions ('s, 't, 're, 've, 'm, 'll, 'd)
        if b == b'\'' && i + 1 < len {
            let next = bytes[i + 1];
            if matches!(next, b's' | b't' | b'm' | b'd' | b'S' | b'T' | b'M' | b'D') {
                tokens += 1;
                i += 2;
                continue;
            } else if i + 2 < len {
                let pair = [bytes[i + 1], bytes[i + 2]];
                if matches!(&pair, b"re" | b"ve" | b"ll" | b"RE" | b"VE" | b"LL") {
                    tokens += 1;
                    i += 3;
                    continue;
                }
            }
        }

        // 7. ASCII Alphanumerics & Identifiers
        if b.is_ascii_alphabetic() || b == b'_' {
            let start = i;
            let mut char_count = 0usize;

            while i < len && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_') {
                // CamelCase boundary: lowercase immediately followed by uppercase
                if i > start && bytes[i].is_ascii_uppercase() && bytes[i - 1].is_ascii_lowercase() {
                    tokens += ((char_count + 2) / 4).max(1);
                    char_count = 0;
                }
                char_count += 1;
                i += 1;
            }

            if char_count > 0 {
                tokens += ((char_count + 2) / 4).max(1);
            }
            continue;
        }

        // 8. ASCII Punctuation and Operators
        if b.is_ascii_punctuation() {
            // Check for 3-character operators (e.g. ===, !==, ..., <<=, >>=, ```)
            if i + 2 < len && bytes[i + 1].is_ascii_punctuation() && bytes[i + 2].is_ascii_punctuation() {
                let triplet = [b, bytes[i + 1], bytes[i + 2]];
                if matches!(
                    &triplet,
                    b"===" | b"!==" | b"..." | b"<<=" | b">>=" | b"```" | b"\"\"\"" | b"'''"
                ) {
                    tokens += 1;
                    i += 3;
                    continue;
                }
            }

            // Check for 2-character operators
            if i + 1 < len && bytes[i + 1].is_ascii_punctuation() {
                let pair = [b, bytes[i + 1]];
                if matches!(
                    &pair,
                    b"::" | b"->" | b"=>" | b"==" | b"!=" | b"<=" | b">=" | b"&&" | b"||"
                        | b"//" | b"/*" | b"*/" | b"+=" | b"-=" | b"*=" | b"/=" | b"<<" | b">>"
                        | b"??" | b"?." | b"##" | b"**" | b"~~"
                ) {
                    tokens += 1;
                    i += 2;
                    continue;
                }
            }
            tokens += 1;
            i += 1;
            continue;
        }

        // 9. Multi-byte UTF-8 sequences (Non-ASCII)
        if b >= 0x80 {
            let char_len = if b & 0xE0 == 0xC0 {
                2
            } else if b & 0xF0 == 0xE0 {
                3
            } else if b & 0xF8 == 0xF0 {
                4
            } else {
                1
            };

            if char_len >= 4 {
                tokens += 2; // Emojis and supplementary planes (often 2 tokens)
            } else if char_len == 3 {
                // CJK and 3-byte unicode characters
                let cjk_tok = (cjk_token_multiplier + 0.5) as usize;
                tokens += cjk_tok.max(1);
            } else {
                tokens += 1; // 2-byte UTF-8 (Cyrillic, Greek, Arabic, Accented Latin)
            }

            i += char_len.min(len - i);
            continue;
        }

        // 10. Fallback for control characters
        tokens += 1;
        i += 1;
    }

    tokens.max(1)
}

/// Simple fallback character-count based token estimate `(chars + 3) / 4`.
#[inline]
pub fn estimate_tokens_simple(text: &str) -> usize {
    if text.is_empty() {
        0
    } else {
        (text.chars().count() + 3) / 4
    }
}

// ---------------------------------------------------------------------------
// 3. Tool Call & Definition Token Overhead Calculations
// ---------------------------------------------------------------------------

/// Detailed breakdown of tokens consumed by a `ToolDefinition`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolDefinitionTokens {
    /// Tokens in the tool name.
    pub name_tokens: usize,
    /// Tokens in the tool description.
    pub description_tokens: usize,
    /// Tokens in the JSON Schema parameters.
    pub parameters_tokens: usize,
    /// Structural JSON/XML framing overhead.
    pub framing_overhead: usize,
    /// Total tokens consumed.
    pub total: usize,
}

/// Detailed breakdown of tokens consumed by a `ToolCall` invocation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolCallTokens {
    /// Tokens in the call ID (e.g. `call_123456`).
    pub id_tokens: usize,
    /// Tokens in the tool name.
    pub name_tokens: usize,
    /// Tokens in the JSON argument payload.
    pub arguments_tokens: usize,
    /// Framing syntax overhead (JSON envelope / tags).
    pub syntax_overhead: usize,
    /// Total tokens consumed.
    pub total: usize,
}

/// Detailed breakdown of tokens consumed by a tool execution result.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolResultTokens {
    /// Tokens in the call ID reference.
    pub id_tokens: usize,
    /// Tokens in the tool output content.
    pub content_tokens: usize,
    /// Message and role framing overhead.
    pub framing_overhead: usize,
    /// Total tokens consumed.
    pub total: usize,
}

/// Estimates tokens consumed by an assistant `ToolCall`.
pub fn estimate_tool_call_tokens(call: &ToolCall) -> usize {
    estimate_tool_call_tokens_with_family(call, TokenizerFamily::GenericBpe)
}

/// Estimates tokens consumed by a `ToolCall` under a specific `TokenizerFamily`.
pub fn estimate_tool_call_tokens_with_family(call: &ToolCall, family: TokenizerFamily) -> usize {
    let detailed = estimate_tool_call_detailed(call, family);
    detailed.total
}

/// Detailed calculation of tokens for a `ToolCall`.
pub fn estimate_tool_call_detailed(call: &ToolCall, family: TokenizerFamily) -> ToolCallTokens {
    let id_tokens = estimate_text_tokens_with_family(&call.id, family);
    let name_tokens = estimate_text_tokens_with_family(&call.name, family);
    let arguments_tokens = estimate_text_tokens_with_family(&call.arguments, family);
    let syntax_overhead = family.tool_call_overhead();

    let total = id_tokens + name_tokens + arguments_tokens + syntax_overhead;

    ToolCallTokens {
        id_tokens,
        name_tokens,
        arguments_tokens,
        syntax_overhead,
        total,
    }
}

/// Estimates tokens consumed by a `ToolDefinition` when injected into the LLM system prompt / tool list.
pub fn estimate_tool_definition_tokens(tool: &ToolDefinition) -> usize {
    estimate_tool_definition_tokens_with_family(tool, TokenizerFamily::GenericBpe)
}

/// Estimates tokens consumed by a `ToolDefinition` under a specific `TokenizerFamily`.
pub fn estimate_tool_definition_tokens_with_family(tool: &ToolDefinition, family: TokenizerFamily) -> usize {
    let detailed = estimate_tool_definition_detailed(tool, family);
    detailed.total
}

/// Detailed calculation of tokens for a `ToolDefinition`.
pub fn estimate_tool_definition_detailed(tool: &ToolDefinition, family: TokenizerFamily) -> ToolDefinitionTokens {
    let name_tokens = estimate_text_tokens_with_family(&tool.name, family);
    let description_tokens = estimate_text_tokens_with_family(&tool.description, family);
    let params_str = tool.parameters.to_string();
    let parameters_tokens = estimate_text_tokens_with_family(&params_str, family);
    let framing_overhead = family.tool_definition_overhead();

    let total = name_tokens + description_tokens + parameters_tokens + framing_overhead;

    ToolDefinitionTokens {
        name_tokens,
        description_tokens,
        parameters_tokens,
        framing_overhead,
        total,
    }
}

/// Estimates detailed tokens consumed by a tool execution result message.
pub fn estimate_tool_result_detailed(tool_call_id: &str, content: &str, family: TokenizerFamily) -> ToolResultTokens {
    let id_tokens = estimate_text_tokens_with_family(tool_call_id, family);
    let content_tokens = estimate_text_tokens_with_family(content, family);
    let framing_overhead = family.message_base_tokens() + 1;

    let total = id_tokens + content_tokens + framing_overhead;

    ToolResultTokens {
        id_tokens,
        content_tokens,
        framing_overhead,
        total,
    }
}

/// Estimates total tokens consumed by a collection of `ToolDefinition`s.
pub fn estimate_tools_tokens(tools: &[ToolDefinition]) -> usize {
    estimate_tools_tokens_with_family(tools, TokenizerFamily::GenericBpe)
}

/// Estimates total tokens consumed by a collection of `ToolDefinition`s with a specific `TokenizerFamily`.
pub fn estimate_tools_tokens_with_family(tools: &[ToolDefinition], family: TokenizerFamily) -> usize {
    if tools.is_empty() {
        return 0;
    }
    let mut total = 10; // Tool collection wrapper header
    for tool in tools {
        total += estimate_tool_definition_tokens_with_family(tool, family);
    }
    total
}

/// Estimates syntax and framing overhead for raw JSON tool call arguments.
pub fn estimate_tool_arguments_overhead(arguments_json: &str, family: TokenizerFamily) -> usize {
    let raw_tokens = estimate_text_tokens_with_family(arguments_json, family);
    let structural_tokens = family.tool_call_overhead();
    raw_tokens + structural_tokens
}

// ---------------------------------------------------------------------------
// 4. Message and Conversation Token Estimation
// ---------------------------------------------------------------------------

/// Estimates tokens consumed by a single `Message`, including roles, names, content, and tool calls.
pub fn estimate_message_tokens(msg: &Message) -> usize {
    estimate_message_tokens_with_family(msg, TokenizerFamily::GenericBpe)
}

/// Estimates tokens consumed by a `Message` under a specific `TokenizerFamily`.
pub fn estimate_message_tokens_with_family(msg: &Message, family: TokenizerFamily) -> usize {
    let mut tokens = family.message_base_tokens();

    // Optional sender name: e.g. `<|im_start|>tool name=bash\n`
    if let Some(name) = &msg.name {
        tokens += estimate_text_tokens_with_family(name, family) + 1;
    }

    // Main content
    if !msg.content.is_empty() {
        tokens += estimate_text_tokens_with_family(&msg.content, family);
    }

    // Role-specific tool call ID
    if let Some(tool_call_id) = &msg.tool_call_id {
        tokens += estimate_text_tokens_with_family(tool_call_id, family) + 1;
    }

    // Assistant tool calls
    if let Some(calls) = &msg.tool_calls {
        for call in calls {
            tokens += estimate_tool_call_tokens_with_family(call, family);
        }
    }

    tokens
}

/// Estimates tokens consumed by a `Message` tailored to a specific model.
pub fn estimate_message_tokens_for_model(msg: &Message, model: &str) -> usize {
    let family = TokenizerFamily::from_model(model);
    estimate_message_tokens_with_family(msg, family)
}

/// Estimates total tokens for a sequence of conversation messages.
///
/// Includes per-message formatting overhead and assistant completion priming.
pub fn estimate_messages_tokens(messages: &[Message]) -> usize {
    estimate_messages_tokens_with_family(messages, TokenizerFamily::GenericBpe)
}

/// Estimates total tokens for messages under a specific `TokenizerFamily`.
pub fn estimate_messages_tokens_with_family(messages: &[Message], family: TokenizerFamily) -> usize {
    if messages.is_empty() {
        return 0;
    }

    let mut total = family.assistant_priming_tokens();
    for msg in messages {
        total += estimate_message_tokens_with_family(msg, family);
    }
    total
}

/// Estimates total tokens for messages tailored to a specific model.
pub fn estimate_messages_tokens_for_model(messages: &[Message], model: &str) -> usize {
    let family = TokenizerFamily::from_model(model);
    estimate_messages_tokens_with_family(messages, family)
}

/// Estimates total tokens for messages including an optional system prompt.
pub fn estimate_messages_tokens_with_system(
    messages: &[Message],
    system_prompt: Option<&str>,
) -> usize {
    estimate_messages_tokens_with_system_and_family(messages, system_prompt, TokenizerFamily::GenericBpe)
}

/// Estimates total tokens for messages including an optional system prompt with a specific family.
pub fn estimate_messages_tokens_with_system_and_family(
    messages: &[Message],
    system_prompt: Option<&str>,
    family: TokenizerFamily,
) -> usize {
    let mut total = estimate_messages_tokens_with_family(messages, family);
    if let Some(system) = system_prompt {
        if !system.is_empty() {
            total += family.message_base_tokens() + estimate_text_tokens_with_family(system, family);
        }
    }
    total
}

// ---------------------------------------------------------------------------
// 5. Streaming Token Count Estimators
// ---------------------------------------------------------------------------

/// Zero-allocation incremental token counter for streaming chunks.
///
/// Handles cross-chunk token boundary splits (e.g. partial words, operators, numbers,
/// multi-byte UTF-8 sequences) without re-scanning historical text.
#[derive(Debug, Clone)]
pub struct StreamingTokenEstimator {
    family: TokenizerFamily,
    total_tokens: usize,
    /// Small fixed stack buffer to hold trailing incomplete token bytes across chunk boundaries.
    tail_buf: [u8; 16],
    tail_len: usize,
}

impl StreamingTokenEstimator {
    /// Creates a new `StreamingTokenEstimator` for the specified tokenizer family.
    pub fn new(family: TokenizerFamily) -> Self {
        Self {
            family,
            total_tokens: 0,
            tail_buf: [0u8; 16],
            tail_len: 0,
        }
    }

    /// Creates a new `StreamingTokenEstimator` for a given model identifier.
    pub fn for_model(model: &str) -> Self {
        Self::new(TokenizerFamily::from_model(model))
    }

    /// Returns the active `TokenizerFamily`.
    pub fn family(&self) -> TokenizerFamily {
        self.family
    }

    /// Returns the current running token count.
    pub fn current_tokens(&self) -> usize {
        self.total_tokens
    }

    /// Feeds an incremental text chunk from a stream.
    ///
    /// Returns the count of newly finalized tokens in this chunk.
    pub fn feed_chunk(&mut self, chunk: &str) -> usize {
        if chunk.is_empty() {
            return 0;
        }

        let chunk_bytes = chunk.as_bytes();
        let chunk_len = chunk_bytes.len();
        let old_total = self.total_tokens;

        // If we have trailing bytes from the previous chunk, combine them in a small stack buffer
        if self.tail_len > 0 {
            let mut combo = [0u8; 64];
            let copy_from_tail = self.tail_len.min(16);
            combo[..copy_from_tail].copy_from_slice(&self.tail_buf[..copy_from_tail]);

            let take_from_chunk = chunk_len.min(64 - copy_from_tail);
            combo[copy_from_tail..copy_from_tail + take_from_chunk]
                .copy_from_slice(&chunk_bytes[..take_from_chunk]);

            let combo_len = copy_from_tail + take_from_chunk;

            // Find a clean boundary in combo
            if let Ok(combo_str) = std::str::from_utf8(&combo[..combo_len]) {
                // If the entire chunk was absorbed into the small buffer
                if take_from_chunk == chunk_len {
                    // Check if chunk ended on whitespace or punctuation boundary
                    let last_b = chunk_bytes[chunk_len - 1];
                    if last_b.is_ascii_whitespace() || last_b == b'\n' {
                        let toks = estimate_text_tokens_with_family(combo_str, self.family);
                        self.total_tokens += toks;
                        self.tail_len = 0;
                    } else {
                        // Still incomplete trailing tail, update tail buffer
                        let copy_tail = combo_len.min(16);
                        self.tail_buf[..copy_tail].copy_from_slice(&combo[combo_len - copy_tail..combo_len]);
                        self.tail_len = copy_tail;
                    }
                    return self.total_tokens - old_total;
                }

                // Estimate the combined prefix
                let prefix_toks = estimate_text_tokens_with_family(combo_str, self.family);
                self.total_tokens += prefix_toks;
                self.tail_len = 0;

                // Process remainder of chunk
                let rem_bytes = &chunk_bytes[take_from_chunk..];
                if !rem_bytes.is_empty() {
                    self.process_slice(rem_bytes);
                }
                return self.total_tokens - old_total;
            }
        }

        // Fast path: no prior tail
        self.process_slice(chunk_bytes);
        self.total_tokens - old_total
    }

    /// Internal slice scanner that splits complete tokens and stores trailing segment in `tail_buf`.
    fn process_slice(&mut self, bytes: &[u8]) {
        let len = bytes.len();
        if len == 0 {
            return;
        }

        // Check if slice ends cleanly on whitespace/newline
        let last_byte = bytes[len - 1];
        if last_byte.is_ascii_whitespace() || last_byte == b'\n' || last_byte == b';' || last_byte == b'}' {
            if let Ok(valid_str) = std::str::from_utf8(bytes) {
                let toks = estimate_text_tokens_with_family(valid_str, self.family);
                self.total_tokens += toks;
                self.tail_len = 0;
                return;
            }
        }

        // Find last clean word/whitespace boundary within the last 16 bytes
        let search_start = len.saturating_sub(16);
        let mut split_idx = len;

        for i in (search_start..len).rev() {
            let b = bytes[i];
            if b.is_ascii_whitespace() || b == b'\n' || b == b'.' || b == b',' || b == b';' {
                split_idx = i + 1;
                break;
            }
        }

        if split_idx < len && split_idx > 0 {
            if let Ok(prefix_str) = std::str::from_utf8(&bytes[..split_idx]) {
                let toks = estimate_text_tokens_with_family(prefix_str, self.family);
                self.total_tokens += toks;
            }
            let tail_slice = &bytes[split_idx..];
            let copy_len = tail_slice.len().min(16);
            self.tail_buf[..copy_len].copy_from_slice(&tail_slice[..copy_len]);
            self.tail_len = copy_len;
        } else if len <= 16 {
            self.tail_buf[..len].copy_from_slice(bytes);
            self.tail_len = len;
        } else {
            // Long segment without standard punctuation, estimate directly
            if let Ok(valid_str) = std::str::from_utf8(bytes) {
                let toks = estimate_text_tokens_with_family(valid_str, self.family);
                self.total_tokens += toks;
                self.tail_len = 0;
            }
        }
    }

    /// Flushes any pending trailing tokens and returns final token count.
    pub fn finish(&mut self) -> usize {
        if self.tail_len > 0 {
            if let Ok(tail_str) = std::str::from_utf8(&self.tail_buf[..self.tail_len]) {
                if !tail_str.is_empty() {
                    let toks = estimate_text_tokens_with_family(tail_str, self.family);
                    self.total_tokens += toks;
                }
            }
            self.tail_len = 0;
        }
        self.total_tokens
    }

    /// Resets the streaming estimator state to zero.
    pub fn reset(&mut self) {
        self.total_tokens = 0;
        self.tail_len = 0;
    }
}

/// Streaming estimator for tracking LLM completion chunks (including reasoning / thinking tokens).
#[derive(Debug, Clone)]
pub struct StreamingCompletionEstimator {
    family: TokenizerFamily,
    content_estimator: StreamingTokenEstimator,
    thinking_estimator: StreamingTokenEstimator,
    content_tokens: usize,
    reasoning_tokens: usize,
    tool_call_tokens: usize,
    exact_completion_tokens: Option<usize>,
}

impl StreamingCompletionEstimator {
    /// Creates a new `StreamingCompletionEstimator` for the given tokenizer family.
    pub fn new(family: TokenizerFamily) -> Self {
        Self {
            family,
            content_estimator: StreamingTokenEstimator::new(family),
            thinking_estimator: StreamingTokenEstimator::new(family),
            content_tokens: 0,
            reasoning_tokens: 0,
            tool_call_tokens: 0,
            exact_completion_tokens: None,
        }
    }

    /// Creates a new `StreamingCompletionEstimator` for a model name.
    pub fn for_model(model: &str) -> Self {
        Self::new(TokenizerFamily::from_model(model))
    }

    /// Ingests a provider `StreamChunk` and updates token estimates.
    pub fn feed_chunk(&mut self, chunk: &StreamChunk) -> usize {
        match chunk {
            StreamChunk::ContentDelta(delta) => self.feed_content(delta),
            StreamChunk::ThinkingDelta(delta) => self.feed_thinking(delta),
            StreamChunk::ToolCallDelta {
                id,
                name,
                arguments_delta,
                ..
            } => self.feed_tool_call_delta(
                id.as_deref(),
                name.as_deref(),
                arguments_delta,
            ),
            StreamChunk::Done {
                completion_tokens,
                ..
            } => {
                if let Some(tokens) = completion_tokens {
                    self.exact_completion_tokens = Some(*tokens as usize);
                }
                0
            }
            StreamChunk::Error(_) => 0,
        }
    }

    /// Feeds standard assistant content delta.
    pub fn feed_content(&mut self, delta: &str) -> usize {
        let added = self.content_estimator.feed_chunk(delta);
        self.content_tokens += added;
        added
    }

    /// Feeds reasoning / thinking model delta (e.g. DeepSeek-R1 / o1 / Claude thinking).
    pub fn feed_thinking(&mut self, delta: &str) -> usize {
        let added = self.thinking_estimator.feed_chunk(delta);
        self.reasoning_tokens += added;
        added
    }

    /// Feeds tool call streaming delta.
    pub fn feed_tool_call_delta(
        &mut self,
        id: Option<&str>,
        name: Option<&str>,
        args_delta: &str,
    ) -> usize {
        let mut added = 0;
        if let Some(id_str) = id {
            added += estimate_text_tokens_with_family(id_str, self.family);
        }
        if let Some(name_str) = name {
            added += estimate_text_tokens_with_family(name_str, self.family) + self.family.tool_call_overhead();
        }
        if !args_delta.is_empty() {
            added += estimate_text_tokens_with_family(args_delta, self.family);
        }
        self.tool_call_tokens += added;
        added
    }

    /// Flushes all pending tokens in internal estimators.
    pub fn finish(&mut self) -> usize {
        let final_content = self.content_estimator.finish();
        let final_thinking = self.thinking_estimator.finish();
        self.content_tokens = final_content;
        self.reasoning_tokens = final_thinking;

        if let Some(exact) = self.exact_completion_tokens {
            exact
        } else {
            self.total_tokens()
        }
    }

    /// Returns standard content tokens.
    pub fn content_tokens(&self) -> usize {
        self.content_tokens
    }

    /// Returns reasoning / thinking tokens.
    pub fn reasoning_tokens(&self) -> usize {
        self.reasoning_tokens
    }

    /// Returns assistant tool call tokens.
    pub fn tool_call_tokens(&self) -> usize {
        self.tool_call_tokens
    }

    /// Returns total completion tokens (content + reasoning + tool calls).
    pub fn total_tokens(&self) -> usize {
        if let Some(exact) = self.exact_completion_tokens {
            exact
        } else {
            self.content_tokens + self.reasoning_tokens + self.tool_call_tokens
        }
    }

    /// Resets all completion counters.
    pub fn reset(&mut self) {
        self.content_estimator.reset();
        self.thinking_estimator.reset();
        self.content_tokens = 0;
        self.reasoning_tokens = 0;
        self.tool_call_tokens = 0;
        self.exact_completion_tokens = None;
    }
}

/// Streaming estimator for incrementally building prompt context buffers.
#[derive(Debug, Clone)]
pub struct StreamingPromptEstimator {
    family: TokenizerFamily,
    system_tokens: usize,
    tools_tokens: usize,
    messages_tokens: usize,
    current_message_estimator: StreamingTokenEstimator,
    current_message_tokens: usize,
}

impl StreamingPromptEstimator {
    /// Creates a new `StreamingPromptEstimator`.
    pub fn new(family: TokenizerFamily) -> Self {
        Self {
            family,
            system_tokens: 0,
            tools_tokens: 0,
            messages_tokens: family.assistant_priming_tokens(),
            current_message_estimator: StreamingTokenEstimator::new(family),
            current_message_tokens: 0,
        }
    }

    /// Creates a new `StreamingPromptEstimator` for a model.
    pub fn for_model(model: &str) -> Self {
        Self::new(TokenizerFamily::from_model(model))
    }

    /// Sets or updates the system prompt.
    pub fn set_system_prompt(&mut self, system: &str) {
        if system.is_empty() {
            self.system_tokens = 0;
        } else {
            self.system_tokens = self.family.message_base_tokens()
                + estimate_text_tokens_with_family(system, self.family);
        }
    }

    /// Ingests tool definitions.
    pub fn set_tools(&mut self, tools: &[ToolDefinition]) {
        self.tools_tokens = estimate_tools_tokens_with_family(tools, self.family);
    }

    /// Ingests a complete message.
    pub fn add_message(&mut self, msg: &Message) {
        self.messages_tokens += estimate_message_tokens_with_family(msg, self.family);
    }

    /// Begins an incremental message.
    pub fn begin_message(&mut self, role: Role, name: Option<&str>) {
        self.current_message_estimator.reset();
        self.current_message_tokens = self.family.message_base_tokens();

        let _ = role;
        if let Some(n) = name {
            self.current_message_tokens += estimate_text_tokens_with_family(n, self.family) + 1;
        }
    }

    /// Feeds content delta to the current message being constructed.
    pub fn feed_message_chunk(&mut self, chunk: &str) -> usize {
        let added = self.current_message_estimator.feed_chunk(chunk);
        self.current_message_tokens += added;
        added
    }

    /// Concludes the current incremental message and commits its tokens to the total.
    pub fn end_message(&mut self) -> usize {
        let final_content = self.current_message_estimator.finish();
        self.current_message_tokens += final_content;
        self.messages_tokens += self.current_message_tokens;
        let committed = self.current_message_tokens;
        self.current_message_tokens = 0;
        self.current_message_estimator.reset();
        committed
    }

    /// Returns the sum of all prompt tokens (system + tools + messages).
    pub fn total_tokens(&self) -> usize {
        self.system_tokens + self.tools_tokens + self.messages_tokens + self.current_message_tokens
    }
}

// ---------------------------------------------------------------------------
// 6. Exact Token Counts When Available
// ---------------------------------------------------------------------------

/// Represents a token count that is either exact (from API usage stats or tokenizer)
/// or estimated via the BPE heuristic.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", content = "value")]
pub enum TokenCount {
    /// Exact count reported by the LLM provider or tokenizer.
    Exact(usize),
    /// Heuristic approximation based on character and subword scanner.
    Estimated(usize),
}

impl TokenCount {
    /// Returns the numerical token count regardless of whether it is exact or estimated.
    #[inline]
    pub fn count(&self) -> usize {
        match self {
            TokenCount::Exact(n) => *n,
            TokenCount::Estimated(n) => *n,
        }
    }

    /// Returns `true` if this token count is verified exact.
    #[inline]
    pub fn is_exact(&self) -> bool {
        matches!(self, TokenCount::Exact(_))
    }

    /// Returns `true` if this token count is an estimate.
    #[inline]
    pub fn is_estimated(&self) -> bool {
        matches!(self, TokenCount::Estimated(_))
    }
}

impl fmt::Display for TokenCount {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TokenCount::Exact(n) => write!(f, "{}", format_token_count(*n)),
            TokenCount::Estimated(n) => write!(f, "{} (est.)", format_token_count(*n)),
        }
    }
}

/// Tracks exact token counts for messages when available from API provider usage reports,
/// falling back to heuristic estimates for unverified or pending messages.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TokenTracker {
    /// Map of message index in session to exact token count.
    exact_message_counts: HashMap<usize, usize>,
    /// Last verified prompt tokens from API provider response.
    last_exact_prompt_tokens: Option<usize>,
    /// Last verified completion tokens from API provider response.
    last_exact_completion_tokens: Option<usize>,
}

impl TokenTracker {
    /// Creates a new empty `TokenTracker`.
    pub fn new() -> Self {
        Self::default()
    }

    /// Records an exact token count for a message at a specific index.
    pub fn set_exact_message_tokens(&mut self, index: usize, count: usize) {
        self.exact_message_counts.insert(index, count);
    }

    /// Records the exact prompt and completion tokens from an API response chunk or turn.
    pub fn record_provider_usage(&mut self, prompt_tokens: usize, completion_tokens: usize) {
        self.last_exact_prompt_tokens = Some(prompt_tokens);
        self.last_exact_completion_tokens = Some(completion_tokens);
    }

    /// Clears recorded exact message counts.
    pub fn clear(&mut self) {
        self.exact_message_counts.clear();
        self.last_exact_prompt_tokens = None;
        self.last_exact_completion_tokens = None;
    }

    /// Returns `true` if the message at the given index has an exact count recorded.
    pub fn has_exact(&self, index: usize) -> bool {
        self.exact_message_counts.contains_key(&index)
    }

    /// Returns the exact count for the message at the given index, if available.
    pub fn get_exact(&self, index: usize) -> Option<usize> {
        self.exact_message_counts.get(&index).copied()
    }

    /// Computes the token count for a single message, using the exact count if recorded,
    /// or falling back to `estimate_message_tokens`.
    pub fn message_tokens(&self, index: usize, msg: &Message) -> TokenCount {
        if let Some(&exact) = self.exact_message_counts.get(&index) {
            TokenCount::Exact(exact)
        } else {
            TokenCount::Estimated(estimate_message_tokens(msg))
        }
    }

    /// Computes total token count across messages, utilizing exact counts wherever available.
    ///
    /// If all messages have exact counts and last prompt count is known, returns `TokenCount::Exact`.
    /// Otherwise returns `TokenCount::Estimated`.
    pub fn total_tokens(&self, messages: &[Message]) -> TokenCount {
        if messages.is_empty() {
            return TokenCount::Exact(0);
        }

        let mut total = ASSISTANT_PRIMING_TOKENS;
        let mut all_exact = true;

        for (idx, msg) in messages.iter().enumerate() {
            match self.message_tokens(idx, msg) {
                TokenCount::Exact(c) => total += c,
                TokenCount::Estimated(c) => {
                    total += c;
                    all_exact = false;
                }
            }
        }

        if all_exact {
            TokenCount::Exact(total)
        } else {
            TokenCount::Estimated(total)
        }
    }

    /// Sum of all verified exact token counts currently in the tracker.
    pub fn exact_tokens_sum(&self) -> usize {
        self.exact_message_counts.values().sum()
    }
}

// ---------------------------------------------------------------------------
// 7. Model Context Limits Catalog
// ---------------------------------------------------------------------------

/// Returns the maximum context window size (in tokens) for a given LLM model name.
///
/// Recognizes popular model identifiers (OpenAI, Anthropic, DeepSeek, Google, Meta Llama, Mistral, Qwen)
/// and applies sensible, up-to-date defaults.
pub fn model_context_limit(model: &str) -> usize {
    let m = model.trim().to_lowercase();

    // 1. Google Gemini models (1M to 2M tokens)
    if m.contains("gemini-1.5-pro") || m.contains("gemini-2.0-pro") {
        return 2_097_152;
    }
    if m.contains("gemini") {
        return 1_048_576;
    }

    // 2. Anthropic Claude models (200k tokens)
    if m.contains("claude-3") || m.contains("claude-sonnet") || m.contains("claude-opus") || m.contains("claude-haiku") {
        return 200_000;
    }
    if m.contains("claude") {
        return 200_000;
    }

    // 3. OpenAI o1 / o3 reasoning models (200k tokens)
    if m.starts_with("o1") || m.starts_with("o3") {
        return 200_000;
    }

    // 4. OpenAI GPT-4o / GPT-4o-mini / GPT-4-turbo (128k tokens)
    if m.contains("gpt-4o") || m.contains("gpt-4-turbo") || m.contains("gpt-4-1106") || m.contains("gpt-4-0125") {
        return 128_000;
    }

    // 5. Classic GPT-4
    if m.contains("gpt-4-32k") {
        return 32_768;
    }
    if m.starts_with("gpt-4") {
        return 8_192;
    }

    // 6. GPT-3.5 Turbo
    if m.contains("gpt-3.5-turbo-16k") {
        return 16_385;
    }
    if m.contains("gpt-3.5-turbo") {
        return 16_385;
    }

    // 7. DeepSeek models (V3, R1, Coder)
    if m.contains("deepseek") {
        return 128_000;
    }

    // 8. Meta Llama models
    if m.contains("llama-3.1") || m.contains("llama-3.2") || m.contains("llama-3.3") {
        return 128_000;
    }
    if m.contains("llama-3") {
        return 8_192;
    }
    if m.contains("llama-2") {
        return 4_096;
    }

    // 9. Mistral & Codestral
    if m.contains("codestral") || m.contains("mistral-large") {
        return 128_000;
    }
    if m.contains("mistral-nemo") || m.contains("mistral-small") || m.contains("mixtral-8x22b") {
        return 128_000;
    }
    if m.contains("mistral") || m.contains("mixtral") {
        return 32_768;
    }

    // 10. Qwen models
    if m.contains("qwen-2.5") || m.contains("qwen2.5") {
        return 128_000;
    }
    if m.contains("qwen") {
        return 32_768;
    }

    // Default fallback
    DEFAULT_CONTEXT_WINDOW
}

/// Checks if a current token count exceeds a specified threshold ratio of the model's context limit.
///
/// E.g. `is_context_overflow(105_000, "gpt-4o", 0.8)` checks if 105k >= 80% of 128k (102.4k) -> returns `true`.
#[inline]
pub fn is_context_overflow(current_tokens: usize, model: &str, threshold: f32) -> bool {
    let limit = model_context_limit(model);
    let trigger_tokens = (limit as f32 * threshold) as usize;
    current_tokens >= trigger_tokens
}

// ---------------------------------------------------------------------------
// 8. Context Budget and Status
// ---------------------------------------------------------------------------

/// Status indicator for context budget usage.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum BudgetStatus {
    /// Usage is comfortably within normal thresholds.
    Normal,
    /// Usage has crossed the warning threshold (e.g. >= 80%).
    Warning { utilization: f32 },
    /// Usage is critically close to limit (e.g. >= 95%).
    Danger { utilization: f32 },
    /// Usage has exceeded available prompt tokens.
    Overflow { excess_tokens: usize },
}

impl BudgetStatus {
    /// Returns true if compaction or pruning is recommended.
    pub fn should_compact(&self) -> bool {
        matches!(
            self,
            BudgetStatus::Warning { .. } | BudgetStatus::Danger { .. } | BudgetStatus::Overflow { .. }
        )
    }

    /// Returns true if the budget is strictly overflowing.
    pub fn is_overflow(&self) -> bool {
        matches!(self, BudgetStatus::Overflow { .. })
    }
}

/// Comprehensive context budget calculator for managing token windows and preventing overflows.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ContextBudget {
    /// Model identifier.
    pub model: String,
    /// Tokenizer family associated with this model.
    pub tokenizer_family: TokenizerFamily,
    /// Maximum context window for the model (in tokens).
    pub max_context_tokens: usize,
    /// Tokens reserved for completion/generation.
    pub reserved_completion_tokens: usize,
    /// Safety margin to protect against tokenizer discrepancies.
    pub safety_margin_tokens: usize,
    /// Warning threshold ratio (default: 0.80).
    pub warning_threshold: f32,
    /// Danger threshold ratio (default: 0.95).
    pub danger_threshold: f32,
}

impl ContextBudget {
    /// Creates a new `ContextBudget` tailored to the specified model with standard defaults.
    pub fn new(model: impl Into<String>) -> Self {
        let model_str = model.into();
        let max_context = model_context_limit(&model_str);
        let family = TokenizerFamily::from_model(&model_str);

        Self {
            model: model_str,
            tokenizer_family: family,
            max_context_tokens: max_context,
            reserved_completion_tokens: DEFAULT_RESERVED_COMPLETION,
            safety_margin_tokens: DEFAULT_SAFETY_MARGIN,
            warning_threshold: DEFAULT_WARNING_THRESHOLD,
            danger_threshold: DEFAULT_DANGER_THRESHOLD,
        }
    }

    /// Overrides the tokenizer family.
    pub fn with_tokenizer_family(mut self, family: TokenizerFamily) -> Self {
        self.tokenizer_family = family;
        self
    }

    /// Overrides the maximum context limit.
    pub fn with_max_context(mut self, max: usize) -> Self {
        self.max_context_tokens = max;
        self
    }

    /// Overrides the reserved completion tokens.
    pub fn with_reserved_completion(mut self, reserved: usize) -> Self {
        self.reserved_completion_tokens = reserved;
        self
    }

    /// Overrides the safety margin tokens.
    pub fn with_safety_margin(mut self, margin: usize) -> Self {
        self.safety_margin_tokens = margin;
        self
    }

    /// Overrides the warning threshold ratio.
    pub fn with_warning_threshold(mut self, threshold: f32) -> Self {
        self.warning_threshold = threshold.clamp(0.1, 0.99);
        self
    }

    /// Overrides the danger threshold ratio.
    pub fn with_danger_threshold(mut self, threshold: f32) -> Self {
        self.danger_threshold = threshold.clamp(0.1, 0.99);
        self
    }

    /// Total budget available for prompts (messages, system prompt, and tools),
    /// after subtracting reserved completion and safety margin.
    #[inline]
    pub fn available_prompt_tokens(&self) -> usize {
        self.max_context_tokens
            .saturating_sub(self.reserved_completion_tokens)
            .saturating_sub(self.safety_margin_tokens)
    }

    /// Calculates remaining tokens before hitting the available prompt budget.
    /// Returns a negative integer if over budget.
    #[inline]
    pub fn remaining_tokens(&self, current_tokens: usize) -> isize {
        self.available_prompt_tokens() as isize - current_tokens as isize
    }

    /// Calculates context utilization ratio against total context limit (0.0 to 1.0+).
    #[inline]
    pub fn utilization(&self, current_tokens: usize) -> f32 {
        if self.max_context_tokens == 0 {
            return 1.0;
        }
        current_tokens as f32 / self.max_context_tokens as f32
    }

    /// Evaluates current token usage and returns a `BudgetStatus`.
    pub fn evaluate_status(&self, current_tokens: usize) -> BudgetStatus {
        let available = self.available_prompt_tokens();
        if current_tokens > available {
            BudgetStatus::Overflow {
                excess_tokens: current_tokens - available,
            }
        } else {
            let util = self.utilization(current_tokens);
            if util >= self.danger_threshold {
                BudgetStatus::Danger { utilization: util }
            } else if util >= self.warning_threshold {
                BudgetStatus::Warning { utilization: util }
            } else {
                BudgetStatus::Normal
            }
        }
    }

    /// Calculates a detailed breakdown of context consumption across system prompt,
    /// messages, and tools.
    pub fn calculate_breakdown(
        &self,
        messages: &[Message],
        system_prompt: Option<&str>,
        tools: &[ToolDefinition],
    ) -> ContextBreakdown {
        let system_tokens = system_prompt
            .filter(|s| !s.is_empty())
            .map(|s| self.tokenizer_family.message_base_tokens() + estimate_text_tokens_with_family(s, self.tokenizer_family))
            .unwrap_or(0);

        let messages_tokens = estimate_messages_tokens_with_family(messages, self.tokenizer_family);
        let tools_tokens = estimate_tools_tokens_with_family(tools, self.tokenizer_family);
        let total_tokens = system_tokens + messages_tokens + tools_tokens;
        let available_budget = self.available_prompt_tokens();
        let remaining = self.remaining_tokens(total_tokens);
        let utilization_pct = self.utilization(total_tokens) * 100.0;
        let status = self.evaluate_status(total_tokens);

        ContextBreakdown {
            system_tokens,
            messages_tokens,
            tools_tokens,
            total_tokens,
            max_context: self.max_context_tokens,
            available_budget,
            remaining_tokens: remaining,
            utilization_pct,
            status,
        }
    }
}

/// Detailed breakdown of context token usage.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ContextBreakdown {
    pub system_tokens: usize,
    pub messages_tokens: usize,
    pub tools_tokens: usize,
    pub total_tokens: usize,
    pub max_context: usize,
    pub available_budget: usize,
    pub remaining_tokens: isize,
    pub utilization_pct: f32,
    pub status: BudgetStatus,
}

impl ContextBreakdown {
    /// Formats a concise summary string of context breakdown.
    pub fn format_summary(&self) -> String {
        format!(
            "Context: {} / {} tokens ({:.1}%) [Sys: {}, Msgs: {}, Tools: {}] - {}",
            format_token_count(self.total_tokens),
            format_token_count(self.max_context),
            self.utilization_pct,
            format_token_count(self.system_tokens),
            format_token_count(self.messages_tokens),
            format_token_count(self.tools_tokens),
            match self.status {
                BudgetStatus::Normal => "OK".to_string(),
                BudgetStatus::Warning { utilization } =>
                    format!("WARN ({:.0}%)", utilization * 100.0),
                BudgetStatus::Danger { utilization } =>
                    format!("DANGER ({:.0}%)", utilization * 100.0),
                BudgetStatus::Overflow { excess_tokens } =>
                    format!("OVERFLOW (+{})", format_token_count(excess_tokens)),
            }
        )
    }
}

// ---------------------------------------------------------------------------
// 9. Truncation and Pruning Assistance
// ---------------------------------------------------------------------------

/// Computes the start and end message index range to fit within a target token budget.
///
/// Ensures that `preserve_head` messages (e.g. user instructions / original goal)
/// and as many of the most recent tail messages as possible are retained.
pub fn suggest_truncation_window(
    messages: &[Message],
    target_tokens: usize,
    preserve_head: usize,
) -> (usize, usize) {
    if messages.is_empty() || target_tokens == 0 {
        return (0, 0);
    }

    let n = messages.len();
    let head_count = preserve_head.min(n);

    // Calculate tokens consumed by preserved head messages
    let mut head_tokens = ASSISTANT_PRIMING_TOKENS;
    for i in 0..head_count {
        head_tokens += estimate_message_tokens(&messages[i]);
    }

    if head_tokens >= target_tokens {
        // Head messages consume full budget; no tail messages fit
        return (n, n);
    }

    let remaining_budget = target_tokens - head_tokens;

    // Scan backwards from the tail to fit as many recent messages as possible
    let mut tail_tokens = 0usize;
    let mut tail_start = n;

    for i in (head_count..n).rev() {
        let msg_tokens = estimate_message_tokens(&messages[i]);
        if tail_tokens + msg_tokens > remaining_budget {
            break;
        }
        tail_tokens += msg_tokens;
        tail_start = i;
    }

    (tail_start, n)
}

// ---------------------------------------------------------------------------
// 10. Number Formatting Helpers
// ---------------------------------------------------------------------------

/// Formats a token count into a human-friendly compact string (e.g. 1.2k, 128k, 1.5M).
pub fn format_token_count(tokens: usize) -> String {
    if tokens >= 1_000_000 {
        let val = tokens as f64 / 1_000_000.0;
        format!("{:.1}M", val)
    } else if tokens >= 10_000 {
        format!("{}k", tokens / 1_000)
    } else if tokens >= 1_000 {
        let val = tokens as f64 / 1_000.0;
        format!("{:.1}k", val)
    } else {
        tokens.to_string()
    }
}

// ---------------------------------------------------------------------------
// 11. Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_estimate_empty_and_short() {
        assert_eq!(estimate_text_tokens(""), 0);
        assert_eq!(estimate_text_tokens("a"), 1);
        assert_eq!(estimate_text_tokens("hi"), 1);
        assert_eq!(estimate_text_tokens("hey"), 1);
    }

    #[test]
    fn test_estimate_english_prose() {
        let text = "The quick brown fox jumps over the lazy dog.";
        let tokens = estimate_text_tokens(text);
        assert!(tokens >= 8 && tokens <= 14, "got {} tokens", tokens);
    }

    #[test]
    fn test_estimate_code_and_operators() {
        let code = r#"
fn calculate_sum(a: i32, b: i32) -> i32 {
    if a == b && b != 0 {
        return a * 2;
    }
    a + b
}
"#;
        let tokens = estimate_text_tokens(code);
        assert!(tokens >= 20 && tokens <= 50, "got {} tokens", tokens);
    }

    #[test]
    fn test_estimate_whitespace_indentation() {
        let indented = "        x = 1;\n        y = 2;\n";
        let tokens = estimate_text_tokens(indented);
        assert!(tokens >= 6 && tokens <= 15, "got {} tokens", tokens);
    }

    #[test]
    fn test_estimate_unicode_cjk() {
        let cjk = "你好世界，人工智能编程助手";
        let tokens = estimate_text_tokens(cjk);
        assert!(tokens >= 12 && tokens <= 35, "got {} tokens for CJK", tokens);
    }

    #[test]
    fn test_tokenizer_family_heuristics() {
        let cjk = "人工智能编程助手";
        let tok_o200k = estimate_text_tokens_with_family(cjk, TokenizerFamily::O200kBase);
        let tok_cl100k = estimate_text_tokens_with_family(cjk, TokenizerFamily::Cl100kBase);

        // o200k has a more compact CJK representation than cl100k
        assert!(tok_o200k <= tok_cl100k, "o200k ({}) should be <= cl100k ({})", tok_o200k, tok_cl100k);

        // Test Claude XML tag recognition
        let xml_text = "<thinking>\nLet me analyze this code.\n</thinking>";
        let tok_claude = estimate_text_tokens_with_family(xml_text, TokenizerFamily::Claude);
        let tok_generic = estimate_text_tokens_with_family(xml_text, TokenizerFamily::GenericBpe);
        assert!(tok_claude <= tok_generic + 2);
    }

    #[test]
    fn test_model_inference_to_family() {
        assert_eq!(TokenizerFamily::from_model("gpt-4o"), TokenizerFamily::O200kBase);
        assert_eq!(TokenizerFamily::from_model("gpt-4o-mini"), TokenizerFamily::O200kBase);
        assert_eq!(TokenizerFamily::from_model("o1-mini"), TokenizerFamily::O200kBase);
        assert_eq!(TokenizerFamily::from_model("o3-mini"), TokenizerFamily::O200kBase);
        assert_eq!(TokenizerFamily::from_model("claude-3-5-sonnet-20241022"), TokenizerFamily::Claude);
        assert_eq!(TokenizerFamily::from_model("claude-3-7-sonnet"), TokenizerFamily::Claude);
        assert_eq!(TokenizerFamily::from_model("deepseek-chat"), TokenizerFamily::DeepSeek);
        assert_eq!(TokenizerFamily::from_model("deepseek-r1"), TokenizerFamily::DeepSeek);
        assert_eq!(TokenizerFamily::from_model("llama-3.1-70b"), TokenizerFamily::Llama3);
        assert_eq!(TokenizerFamily::from_model("gemini-1.5-pro"), TokenizerFamily::Gemini);
        assert_eq!(TokenizerFamily::from_model("gpt-4-0613"), TokenizerFamily::Cl100kBase);
    }

    #[test]
    fn test_streaming_token_estimator_parity() {
        let full_text = "fn main() {\n    let answer = 42;\n    println!(\"The answer is {}\", answer);\n}\n";
        let batch_tokens = estimate_text_tokens(full_text);

        // Stream chunk by chunk (simulating 5-character chunks)
        let mut stream_estimator = StreamingTokenEstimator::new(TokenizerFamily::GenericBpe);
        for chunk in full_text.as_bytes().chunks(5) {
            let chunk_str = std::str::from_utf8(chunk).unwrap();
            stream_estimator.feed_chunk(chunk_str);
        }
        let stream_tokens = stream_estimator.finish();

        // Parity within small threshold
        let diff = (batch_tokens as isize - stream_tokens as isize).abs();
        assert!(diff <= 2, "batch was {}, stream was {}", batch_tokens, stream_tokens);
    }

    #[test]
    fn test_streaming_completion_estimator() {
        let mut estimator = StreamingCompletionEstimator::for_model("deepseek-r1");

        estimator.feed_thinking("Thinking through the problem step by step...\n");
        assert!(estimator.reasoning_tokens() > 0);

        estimator.feed_content("Here is the solution to your question:\n");
        assert!(estimator.content_tokens() > 0);

        estimator.feed_tool_call_delta(
            Some("call_abc123"),
            Some("read_file"),
            "{\"path\": \"src/lib.rs\"}",
        );
        assert!(estimator.tool_call_tokens() > 0);

        let total = estimator.finish();
        assert_eq!(total, estimator.content_tokens() + estimator.reasoning_tokens() + estimator.tool_call_tokens());
    }

    #[test]
    fn test_streaming_prompt_estimator() {
        let mut prompt_est = StreamingPromptEstimator::for_model("gpt-4o");
        prompt_est.set_system_prompt("You are an expert Rust programming assistant.");
        assert!(prompt_est.total_tokens() > 10);

        prompt_est.begin_message(Role::User, None);
        prompt_est.feed_message_chunk("Can you ");
        prompt_est.feed_message_chunk("write a ");
        prompt_est.feed_message_chunk("quicksort algorithm in Rust?");
        let msg_tokens = prompt_est.end_message();
        assert!(msg_tokens > 5);

        assert!(prompt_est.total_tokens() > msg_tokens);
    }

    #[test]
    fn test_tool_call_and_definition_detailed() {
        let tool = ToolDefinition {
            name: "execute_command".to_string(),
            description: "Executes a shell command in the workspace.".to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "command": { "type": "string" },
                    "cwd": { "type": "string" }
                },
                "required": ["command"]
            }),
        };

        let def_tokens = estimate_tool_definition_detailed(&tool, TokenizerFamily::O200kBase);
        assert!(def_tokens.name_tokens > 0);
        assert!(def_tokens.description_tokens > 0);
        assert!(def_tokens.parameters_tokens > 0);
        assert!(def_tokens.framing_overhead > 0);
        assert_eq!(
            def_tokens.total,
            def_tokens.name_tokens + def_tokens.description_tokens + def_tokens.parameters_tokens + def_tokens.framing_overhead
        );

        let call = ToolCall {
            id: "call_987xyz".to_string(),
            name: "execute_command".to_string(),
            arguments: json!({ "command": "cargo check", "cwd": "/home/user" }).to_string(),
        };

        let call_tokens = estimate_tool_call_detailed(&call, TokenizerFamily::O200kBase);
        assert!(call_tokens.id_tokens > 0);
        assert!(call_tokens.name_tokens > 0);
        assert!(call_tokens.arguments_tokens > 0);
        assert_eq!(
            call_tokens.total,
            call_tokens.id_tokens + call_tokens.name_tokens + call_tokens.arguments_tokens + call_tokens.syntax_overhead
        );

        let result_tokens = estimate_tool_result_detailed("call_987xyz", "Build succeeded with 0 warnings.", TokenizerFamily::O200kBase);
        assert!(result_tokens.total > 0);
    }

    #[test]
    fn test_estimate_message_tokens() {
        let msg = Message::user("Can you explain how Rust lifetimes work?");
        let tokens = estimate_message_tokens(&msg);
        assert!(tokens > 5, "got {} tokens", tokens);

        let assistant_msg = Message::assistant_with_tools(
            "I will check the files.",
            vec![ToolCall {
                id: "call_123".to_string(),
                name: "read_file".to_string(),
                arguments: json!({ "path": "src/main.rs" }).to_string(),
            }],
        );
        let call_tokens = estimate_message_tokens(&assistant_msg);
        assert!(call_tokens > tokens, "tool call should have more tokens");
    }

    #[test]
    fn test_estimate_messages_tokens() {
        let msgs = vec![
            Message::system("You are a helpful assistant."),
            Message::user("Hello!"),
            Message::assistant("Hi there! How can I help you today?"),
        ];
        let total = estimate_messages_tokens(&msgs);
        assert!(total >= 20 && total <= 50, "total was {}", total);
    }

    #[test]
    fn test_token_tracker_exact_and_estimated() {
        let mut tracker = TokenTracker::new();
        let msgs = vec![
            Message::user("First prompt"),
            Message::assistant("First response"),
        ];

        // Before exact counts: both estimated
        let t1 = tracker.total_tokens(&msgs);
        assert!(t1.is_estimated());

        // Set exact count for first message
        tracker.set_exact_message_tokens(0, 5);
        let t2 = tracker.message_tokens(0, &msgs[0]);
        assert_eq!(t2, TokenCount::Exact(5));

        // Total still has estimated second message
        let t3 = tracker.total_tokens(&msgs);
        assert!(t3.is_estimated());

        // Set exact for second message
        tracker.set_exact_message_tokens(1, 8);
        let t4 = tracker.total_tokens(&msgs);
        assert!(t4.is_exact());
        // 5 + 8 + ASSISTANT_PRIMING_TOKENS (3) = 16
        assert_eq!(t4.count(), 16);
    }

    #[test]
    fn test_model_context_limits() {
        assert_eq!(model_context_limit("gpt-4o"), 128_000);
        assert_eq!(model_context_limit("claude-3-5-sonnet-20241022"), 200_000);
        assert_eq!(model_context_limit("o1"), 200_000);
        assert_eq!(model_context_limit("deepseek-chat"), 128_000);
        assert_eq!(model_context_limit("gemini-1.5-pro"), 2_097_152);
        assert_eq!(model_context_limit("llama-3.1-70b"), 128_000);
        assert_eq!(model_context_limit("llama-3-8b"), 8_192);
        assert_eq!(model_context_limit("unknown-custom-model"), DEFAULT_CONTEXT_WINDOW);
    }

    #[test]
    fn test_is_context_overflow() {
        // gpt-4o has 128k limit
        // 80% is 102,400 tokens
        assert!(!is_context_overflow(100_000, "gpt-4o", 0.8));
        assert!(is_context_overflow(103_000, "gpt-4o", 0.8));
    }

    #[test]
    fn test_context_budget() {
        let budget = ContextBudget::new("gpt-4o")
            .with_reserved_completion(4000)
            .with_safety_margin(1000);

        assert_eq!(budget.max_context_tokens, 128_000);
        assert_eq!(budget.available_prompt_tokens(), 123_000);

        let msgs = vec![
            Message::user("Short message"),
            Message::assistant("Short reply"),
        ];

        let breakdown = budget.calculate_breakdown(&msgs, Some("System prompt"), &[]);
        assert!(breakdown.total_tokens > 0);
        assert!(breakdown.remaining_tokens > 0);
        assert_eq!(breakdown.status, BudgetStatus::Normal);
    }

    #[test]
    fn test_format_token_count() {
        assert_eq!(format_token_count(42), "42");
        assert_eq!(format_token_count(999), "999");
        assert_eq!(format_token_count(1_200), "1.2k");
        assert_eq!(format_token_count(15_400), "15k");
        assert_eq!(format_token_count(128_000), "128k");
        assert_eq!(format_token_count(2_000_000), "2.0M");
    }

    #[test]
    fn test_suggest_truncation_window() {
        let msgs = vec![
            Message::user("Task instructions: please build an awesome CLI application."),
            Message::assistant("Sure, I can help with that."),
            Message::user("Turn 1"),
            Message::assistant("Turn 1 reply"),
            Message::user("Turn 2"),
            Message::assistant("Turn 2 reply"),
        ];

        // Large budget should keep all messages
        let (start, end) = suggest_truncation_window(&msgs, 10_000, 1);
        assert_eq!(start, 1);
        assert_eq!(end, msgs.len());

        // Tiny budget should keep only the latest turns or head
        let (start_small, _) = suggest_truncation_window(&msgs, 30, 1);
        assert!(start_small >= 1);
    }
}
