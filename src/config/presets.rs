//! Pre-built configuration presets for Fusion.
//!
//! Two families of presets are provided:
//!
//! 1. [`ConfigPreset`] — full environment profiles (provider + model + advisor
//!    policy + terminal cues) for distinct usage styles:
//!    - `coding-fast`: Ultra-fast response coding preset using Claude 3.5 Sonnet / Haiku.
//!    - `deep-reasoning`: Extended chain-of-thought architecture and reasoning using DeepSeek R1.
//!    - `cheap`: Cost-optimized low-token profile disabling advisors to minimize token burn.
//!    - `offline-ollama`: 100% offline local LLM preset using Ollama (Qwen 2.5 Coder / Llama 3).
//!    - `termux-mobile`: Mobile & Termux optimized preset with compact limits and terminal bell cues.
//!
//! 2. [`ModelPreset`] — named provider+model presets (`deepseek-fast`,
//!    `deepseek-reasoning`, `claude-sonnet`, `claude-haiku`, `gpt-4o`, `grok`,
//!    `ollama-local`, `openrouter-default`) that pin a specific provider/model
//!    pairing with sensible temperature, token, context-window, and advisor
//!    policy defaults.

use serde::{Deserialize, Serialize};
use std::fmt;
use std::path::Path;
use std::str::FromStr;

use crate::config::Config;

/// Supported pre-built configuration presets in Fusion.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ConfigPreset {
    /// High-throughput, low-latency coding profile with full advisor assistance.
    CodingFast,
    /// Deep multi-step reasoning, architectural planning, and math using R1.
    DeepReasoning,
    /// Maximum cost efficiency / budget mode with minimal token expenditure.
    Cheap,
    /// 100% offline local LLM preset using Ollama (Qwen 2.5 Coder / Llama 3).
    OfflineOllama,
    /// Default profile routed through the Fusion API provider.
    FusionDefault,
    /// Mobile environment optimization for Termux / Android phones.
    TermuxMobile,
}

impl ConfigPreset {
    /// Canonical kebab-case identifier for this preset.
    pub const fn id(&self) -> &'static str {
        match self {
            Self::CodingFast => "coding-fast",
            Self::DeepReasoning => "deep-reasoning",
            Self::Cheap => "cheap",
            Self::OfflineOllama => "offline-ollama",
            Self::FusionDefault => "fusion-default",
            Self::TermuxMobile => "termux-mobile",
        }
    }

    /// Target LLM provider for this preset.
    pub const fn provider(&self) -> &'static str {
        match self {
            Self::CodingFast => "anthropic",
            Self::DeepReasoning | Self::Cheap | Self::TermuxMobile => "deepseek",
            Self::OfflineOllama => "ollama",
            Self::FusionDefault => "fusion",
        }
    }

    /// Short human-readable display title.
    pub const fn title(&self) -> &'static str {
        match self {
            Self::CodingFast => "Coding Fast (Low Latency)",
            Self::DeepReasoning => "Deep Reasoning (R1 / Thought Chains)",
            Self::Cheap => "Cheap / Budget (Low Cost)",
            Self::OfflineOllama => "Offline Ollama (Local)",
            Self::FusionDefault => "Fusion Default (API)",
            Self::TermuxMobile => "Termux Mobile (Low RAM / Battery)",
        }
    }

    /// Detailed description of the preset's characteristics and tuning.
    pub const fn description(&self) -> &'static str {
        match self {
            Self::CodingFast => {
                "Ultra-fast coding assistance using Claude 3.5 Sonnet with low latency and balanced 8k token windows."
            }
            Self::DeepReasoning => {
                "Complex architectural analysis and deep reasoning using DeepSeek-R1 with 16k output tokens and deterministic temperature."
            }
            Self::Cheap => {
                "Cost-effective daily driver using DeepSeek-Chat with advisors disabled to minimize API billing and token burn."
            }
            Self::OfflineOllama => {
                "100% offline local LLM preset using Ollama (Qwen 2.5 Coder / Llama 3) with advisors disabled for total privacy."
            }
            Self::FusionDefault => {
                "Balanced default profile routed through the Fusion API endpoint with advisor critiques enabled."
            }
            Self::TermuxMobile => {
                "Resource-conscious mobile profile for Android Termux with compact 2k token limits, advisors disabled, and audible terminal bell."
            }
        }
    }

    /// Target LLM model identifier for this preset.
    pub const fn model(&self) -> &'static str {
        match self {
            Self::CodingFast => "claude-3-5-sonnet-20241022",
            Self::DeepReasoning => "deepseek-reasoner",
            Self::Cheap => "deepseek-chat",
            Self::OfflineOllama => "qwen2.5-coder",
            Self::FusionDefault => "fusion-chat",
            Self::TermuxMobile => "deepseek-chat",
        }
    }

    /// Sampling temperature configured for this preset.
    pub const fn temperature(&self) -> Option<f32> {
        match self {
            Self::CodingFast => Some(0.2),
            Self::DeepReasoning => Some(0.0),
            Self::Cheap => Some(0.1),
            Self::OfflineOllama => Some(0.2),
            Self::FusionDefault => Some(0.2),
            Self::TermuxMobile => Some(0.2),
        }
    }

    /// Maximum generation token limit configured for this preset.
    pub const fn max_tokens(&self) -> Option<u32> {
        match self {
            Self::CodingFast => Some(8192),
            Self::DeepReasoning => Some(16384),
            Self::Cheap => Some(4096),
            Self::OfflineOllama => Some(4096),
            Self::FusionDefault => Some(4096),
            Self::TermuxMobile => Some(2048),
        }
    }

    /// Whether parallel advisor critiques are enabled for this preset.
    pub const fn advisors_enabled(&self) -> bool {
        match self {
            Self::CodingFast => true,
            Self::DeepReasoning => true,
            Self::Cheap => false,
            Self::OfflineOllama => false,
            Self::FusionDefault => false,
            Self::TermuxMobile => false,
        }
    }

    /// Optional specific model override for the advisor critiques.
    pub const fn advisor_model(&self) -> Option<&'static str> {
        match self {
            Self::CodingFast => Some("claude-3-5-haiku-20241022"),
            Self::DeepReasoning => Some("deepseek-reasoner"),
            Self::Cheap => None,
            Self::OfflineOllama => None,
            Self::FusionDefault => None,
            Self::TermuxMobile => None,
        }
    }

    /// Whether terminal audio cues and sound cues are enabled.
    pub const fn sound_enabled(&self) -> bool {
        match self {
            Self::CodingFast => false,
            Self::DeepReasoning => false,
            Self::Cheap => false,
            Self::OfflineOllama => false,
            Self::FusionDefault => false,
            Self::TermuxMobile => true,
        }
    }

    /// Whether terminal bell rings upon successful turn completion.
    pub const fn bell_on_completion(&self) -> bool {
        match self {
            Self::CodingFast => true,
            Self::DeepReasoning => true,
            Self::Cheap => true,
            Self::OfflineOllama => false,
            Self::FusionDefault => false,
            Self::TermuxMobile => true,
        }
    }

    /// Whether terminal bell rings on turn errors or tool failures.
    pub const fn bell_on_error(&self) -> bool {
        match self {
            Self::CodingFast => true,
            Self::DeepReasoning => true,
            Self::Cheap => true,
            Self::OfflineOllama => false,
            Self::FusionDefault => false,
            Self::TermuxMobile => true,
        }
    }

    /// Default Fusion API base URL if relevant.
    pub const fn fusion_base_url(&self) -> Option<&'static str> {
        match self {
            Self::FusionDefault => Some("https://api.fusioncode.app/v1"),
            _ => None,
        }
    }

    /// Default local Ollama base URL if this preset targets Ollama.
    pub const fn ollama_base_url(&self) -> Option<&'static str> {
        match self {
            Self::OfflineOllama => Some("http://localhost:11434"),
            _ => None,
        }
    }

    /// Recommended use cases for this preset.
    pub const fn recommended_for(&self) -> &'static str {
        match self {
            Self::CodingFast => "Full-stack web/systems programming, fast refactoring, day-to-day coding",
            Self::DeepReasoning => "Complex algorithm design, security audits, formal architecture reviews, bug triage",
            Self::Cheap => "Budget-conscious coding, high-volume automation, simple scripts, documentation",
            Self::OfflineOllama => "100% offline local coding, privacy-sensitive work, no cloud dependency",
            Self::FusionDefault => "Default day-to-day coding through the Fusion API provider",
            Self::TermuxMobile => "Android phones/tablets, mobile SSH sessions, low-bandwidth connections, battery saving",
        }
    }

    /// Slice of all available configuration presets.
    pub const fn all() -> &'static [ConfigPreset] {
        &[
            ConfigPreset::CodingFast,
            ConfigPreset::DeepReasoning,
            ConfigPreset::Cheap,
            ConfigPreset::OfflineOllama,
            ConfigPreset::FusionDefault,
            ConfigPreset::TermuxMobile,
        ]
    }

    /// Parses a preset name loosely, tolerating case, hyphens, underscores, and common aliases.
    ///
    /// Examples:
    /// - `"coding-fast"`, `"fast"`, `"coding"`, `"code-fast"` -> `Some(CodingFast)`
    /// - `"deep-reasoning"`, `"reasoning"`, `"deep"`, `"r1"`, `"think"` -> `Some(DeepReasoning)`
    /// - `"cheap"`, `"budget"`, `"low-cost"`, `"mini"`, `"frugal"` -> `Some(Cheap)`
    /// - `"offline-ollama"`, `"ollama"`, `"offline"`, `"local"`, `"local-ollama"` -> `Some(OfflineOllama)`
    /// - `"termux-mobile"`, `"termux"`, `"mobile"`, `"android"`, `"phone"` -> `Some(TermuxMobile)`
    pub fn from_str_loose(s: &str) -> Option<Self> {
        let clean = s.trim().to_lowercase().replace('_', "-");
        match clean.as_str() {
            // Coding Fast
            "coding-fast" | "codingfast" | "code-fast" | "codefast" | "fast" | "coding"
            | "fast-coding" | "speed" => Some(Self::CodingFast),

            // Deep Reasoning
            "deep-reasoning" | "deepreasoning" | "deep" | "reasoning" | "reasoner" | "r1"
            | "deepseek-r1" | "think" | "thinking" | "architect" => Some(Self::DeepReasoning),

            // Cheap / Budget
            "cheap" | "budget" | "low-cost" | "lowcost" | "economical" | "economy" | "mini"
            | "frugal" | "saver" => Some(Self::Cheap),

            // Offline Ollama
            "offline-ollama" | "offlineollama" | "ollama" | "offline" | "local"
            | "local-ollama" => Some(Self::OfflineOllama),

            // Fusion Default
            "fusion-default" | "fusiondefault" | "fusion" | "default" => {
                Some(Self::FusionDefault)
            }

            // Termux Mobile
            "termux-mobile" | "termuxmobile" | "termux" | "mobile" | "android" | "phone"
            | "tablet" | "low-ram" | "battery" => Some(Self::TermuxMobile),

            _ => None,
        }
    }

    /// Converts this preset into a complete [`Config`] instance initialized with its defaults.
    pub fn to_config(&self) -> Config {
        let mut cfg = Config::default();
        self.apply_to(&mut cfg);
        cfg
    }

    /// Applies this preset's settings to an existing [`Config`], preserving existing API keys
    /// and custom provider base URLs.
    pub fn apply_to(&self, config: &mut Config) {
        config.default_provider = self.provider().to_string();
        config.default_model = self.model().to_string();
        config.default_temperature = self.temperature();
        config.max_tokens = self.max_tokens();
        config.advisors_enabled = self.advisors_enabled();
        config.advisor_model = self.advisor_model().map(|s| s.to_string());
        config.sound_enabled = self.sound_enabled();
        config.bell_on_completion = self.bell_on_completion();
        config.bell_on_error = self.bell_on_error();

        if let Some(ollama_url) = self.ollama_base_url() {
            if config.ollama_base_url.is_none() {
                config.ollama_base_url = Some(ollama_url.to_string());
            }
        } else if let Some(fusion_url) = self.fusion_base_url() {
            if config.ollama_base_url.is_none() {
                config.ollama_base_url = Some(fusion_url.to_string());
            }
        }
    }

    /// Loads the active configuration from `~/.fusion/config.json`, applies this preset,
    /// and persists it to disk.
    pub fn apply_and_save(&self) -> Result<Config, PresetError> {
        let mut cfg = Config::load();
        self.apply_to(&mut cfg);
        cfg.save()
            .map_err(|e| PresetError::IoError(format!("Failed to save config: {}", e)))?;
        Ok(cfg)
    }

    /// Loads configuration from a specific file path, applies this preset, and writes back.
    pub fn apply_to_file(path: &Path, preset: ConfigPreset) -> Result<Config, PresetError> {
        let mut cfg = if path.exists() {
            let content = std::fs::read_to_string(path)
                .map_err(|e| PresetError::IoError(format!("Cannot read {}: {}", path.display(), e)))?;
            Config::from_json(&content)
                .map_err(|e| PresetError::ParseError(format!("Cannot parse {}: {}", path.display(), e)))?
        } else {
            Config::default()
        };

        preset.apply_to(&mut cfg);

        let json = serde_json::to_string_pretty(&cfg)
            .map_err(|e| PresetError::ParseError(format!("Serialization error: {}", e)))?;

        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| {
                PresetError::IoError(format!("Cannot create dir {}: {}", parent.display(), e))
            })?;
        }

        std::fs::write(path, json)
            .map_err(|e| PresetError::IoError(format!("Cannot write {}: {}", path.display(), e)))?;

        Ok(cfg)
    }

    /// Returns a structured metadata record for introspection and display.
    pub fn info(&self) -> PresetInfo {
        PresetInfo {
            id: self.id(),
            title: self.title(),
            description: self.description(),
            provider: self.provider(),
            model: self.model(),
            temperature: self.temperature(),
            max_tokens: self.max_tokens(),
            advisors_enabled: self.advisors_enabled(),
            advisor_model: self.advisor_model(),
            sound_enabled: self.sound_enabled(),
            bell_on_completion: self.bell_on_completion(),
            bell_on_error: self.bell_on_error(),
            recommended_for: self.recommended_for(),
        }
    }
}

/// Named provider+model configuration presets.
///
/// Each variant pins a concrete provider/model pairing together with
/// recommended sampling, token-budget, context-window, and advisor-policy
/// defaults. Apply one with [`ModelPreset::apply_to`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ModelPreset {
    /// DeepSeek V3 chat — fast, inexpensive general-purpose coding.
    DeepseekFast,
    /// DeepSeek R1 — extended chain-of-thought reasoning.
    DeepseekReasoning,
    /// Claude Sonnet — balanced flagship coding assistant.
    ClaudeSonnet,
    /// Claude Haiku — lowest-latency Claude tier.
    ClaudeHaiku,
    /// OpenAI GPT-4o — multimodal generalist.
    Gpt4o,
    /// xAI Grok 2 — high-throughput assistant with large context.
    Grok,
    /// Default route through the Fusion API provider.
    FusionDefault,
    /// OpenRouter default route — DeepSeek V3 via OpenRouter.
    OpenrouterDefault,
}

impl ModelPreset {
    /// Canonical kebab-case identifier for this preset.
    pub const fn id(&self) -> &'static str {
        match self {
            Self::DeepseekFast => "deepseek-fast",
            Self::DeepseekReasoning => "deepseek-reasoning",
            Self::ClaudeSonnet => "claude-sonnet",
            Self::ClaudeHaiku => "claude-haiku",
            Self::Gpt4o => "gpt-4o",
            Self::Grok => "grok",
            Self::FusionDefault => "fusion-default",
            Self::OpenrouterDefault => "openrouter-default",
        }
    }

    /// Short human-readable display title.
    pub const fn title(&self) -> &'static str {
        match self {
            Self::DeepseekFast => "DeepSeek Fast (V3 Chat)",
            Self::DeepseekReasoning => "DeepSeek Reasoning (R1)",
            Self::ClaudeSonnet => "Claude Sonnet (3.5)",
            Self::ClaudeHaiku => "Claude Haiku (3.5)",
            Self::Gpt4o => "GPT-4o",
            Self::Grok => "Grok 2 (xAI)",
            Self::FusionDefault => "Fusion Default (API)",
            Self::OpenrouterDefault => "OpenRouter Default (DeepSeek V3)",
        }
    }

    /// Detailed description of the preset's characteristics and tuning.
    pub const fn description(&self) -> &'static str {
        match self {
            Self::DeepseekFast => {
                "DeepSeek V3 chat model tuned for low-latency, low-cost everyday coding with moderate determinism."
            }
            Self::DeepseekReasoning => {
                "DeepSeek R1 reasoner with deterministic temperature and a 16k output budget for multi-step reasoning."
            }
            Self::ClaudeSonnet => {
                "Claude 3.5 Sonnet balanced flagship with a 200k context window and Haiku-powered advisor critiques."
            }
            Self::ClaudeHaiku => {
                "Claude 3.5 Haiku optimized for minimal latency and snappy interactive sessions."
            }
            Self::Gpt4o => {
                "OpenAI GPT-4o multimodal generalist with a 128k context window and deterministic sampling."
            }
            Self::Grok => {
                "xAI Grok 2 with a 128k-class context window and low temperature for factual responses."
            }
            Self::FusionDefault => {
                "Balanced default profile routed through the Fusion API endpoint with advisor critiques enabled."
            }
            Self::OpenrouterDefault => {
                "DeepSeek V3 routed through OpenRouter with conservative tokens and advisors disabled."
            }
        }
    }

    /// Target LLM provider for this preset.
    pub const fn provider(&self) -> &'static str {
        match self {
            Self::DeepseekFast | Self::DeepseekReasoning => "deepseek",
            Self::ClaudeSonnet | Self::ClaudeHaiku => "anthropic",
            Self::Gpt4o => "openai",
            Self::Grok => "xai",
            Self::FusionDefault => "fusion",
            Self::OpenrouterDefault => "openrouter",
        }
    }

    /// Target LLM model identifier for this preset.
    pub const fn model(&self) -> &'static str {
        match self {
            Self::DeepseekFast => "deepseek-chat",
            Self::DeepseekReasoning => "deepseek-reasoner",
            Self::ClaudeSonnet => "claude-3-5-sonnet-20241022",
            Self::ClaudeHaiku => "claude-3-5-haiku-20241022",
            Self::Gpt4o => "gpt-4o",
            Self::Grok => "grok-2-latest",
            Self::FusionDefault => "fusion-chat",
            Self::OpenrouterDefault => "deepseek/deepseek-chat",
        }
    }

    /// Sampling temperature configured for this preset.
    pub const fn temperature(&self) -> Option<f32> {
        match self {
            Self::DeepseekFast => Some(0.3),
            Self::DeepseekReasoning => Some(0.0),
            Self::ClaudeSonnet => Some(0.2),
            Self::ClaudeHaiku => Some(0.2),
            Self::Gpt4o => Some(0.2),
            Self::Grok => Some(0.1),
            Self::FusionDefault => Some(0.2),
            Self::OpenrouterDefault => Some(0.2),
        }
    }

    /// Maximum generation token limit configured for this preset.
    pub const fn max_tokens(&self) -> Option<u32> {
        match self {
            Self::DeepseekFast => Some(8192),
            Self::DeepseekReasoning => Some(16384),
            Self::ClaudeSonnet => Some(8192),
            Self::ClaudeHaiku => Some(4096),
            Self::Gpt4o => Some(16384),
            Self::Grok => Some(8192),
            Self::FusionDefault => Some(4096),
            Self::OpenrouterDefault => Some(8192),
        }
    }

    /// Provider context window in tokens configured for this preset.
    pub const fn context_window(&self) -> Option<u64> {
        match self {
            Self::DeepseekFast | Self::DeepseekReasoning | Self::OpenrouterDefault => Some(64_000),
            Self::ClaudeSonnet | Self::ClaudeHaiku => Some(200_000),
            Self::Gpt4o => Some(128_000),
            Self::Grok => Some(131_072),
            Self::FusionDefault => Some(64_000),
        }
    }

    /// Whether parallel advisor critiques are enabled for this preset.
    pub const fn advisors_enabled(&self) -> bool {
        match self {
            Self::DeepseekFast => true,
            Self::DeepseekReasoning => true,
            Self::ClaudeSonnet => true,
            Self::ClaudeHaiku => false,
            Self::Gpt4o => true,
            Self::Grok => false,
            Self::FusionDefault => false,
            Self::OpenrouterDefault => false,
        }
    }

    /// Optional specific model override for the advisor critiques.
    pub const fn advisor_model(&self) -> Option<&'static str> {
        match self {
            Self::DeepseekFast | Self::DeepseekReasoning => Some("deepseek-chat"),
            Self::ClaudeSonnet => Some("claude-3-5-haiku-20241022"),
            _ => None,
        }
    }

    /// Default Fusion API base URL if relevant.
    pub const fn fusion_base_url(&self) -> Option<&'static str> {
        match self {
            Self::FusionDefault => Some("https://api.fusioncode.app/v1"),
            _ => None,
        }
    }

    /// Recommended use cases for this preset.
    pub const fn recommended_for(&self) -> &'static str {
        match self {
            Self::DeepseekFast => "Everyday coding, refactoring, fast iteration on a budget",
            Self::DeepseekReasoning => "Algorithm design, math, deep multi-step analysis",
            Self::ClaudeSonnet => "Full-stack development, large codebases, agentic workflows",
            Self::ClaudeHaiku => "Interactive autocomplete, quick edits, latency-sensitive UX",
            Self::Gpt4o => "Multimodal tasks, generalist reasoning, mixed-language projects",
            Self::Grok => "High-throughput chat, factual Q&A, long documents",
            Self::FusionDefault => "Default day-to-day coding through the Fusion API provider",
            Self::OpenrouterDefault => "Single-key multi-provider setups, fallback routing",
        }
    }

    /// Slice of all available model presets in canonical order.
    pub const fn all() -> &'static [ModelPreset] {
        &[
            ModelPreset::DeepseekFast,
            ModelPreset::DeepseekReasoning,
            ModelPreset::ClaudeSonnet,
            ModelPreset::ClaudeHaiku,
            ModelPreset::Gpt4o,
            ModelPreset::Grok,
            ModelPreset::FusionDefault,
            ModelPreset::OpenrouterDefault,
        ]
    }

    /// Looks up a model preset by its canonical kebab-case identifier.
    ///
    /// Returns `None` when no preset matches `id` exactly.
    pub fn lookup(id: &str) -> Option<Self> {
        Self::all().iter().copied().find(|p| p.id() == id)
    }

    /// Parses a model preset name loosely, tolerating case, hyphens,
    /// underscores, and common aliases.
    ///
    /// Examples:
    /// - `"deepseek-fast"`, `"deepseek_fast"`, `"ds-fast"`, `"deepseek"` -> `Some(DeepseekFast)`
    /// - `"deepseek-reasoning"`, `"r1"`, `"ds-reasoner"` -> `Some(DeepseekReasoning)`
    /// - `"claude-sonnet"`, `"sonnet"`, `"claude"` -> `Some(ClaudeSonnet)`
    /// - `"claude-haiku"`, `"haiku"` -> `Some(ClaudeHaiku)`
    /// - `"gpt-4o"`, `"gpt4o"`, `"4o"`, `"openai"` -> `Some(Gpt4o)`
    /// - `"grok"`, `"xai"` -> `Some(Grok)`
    /// - `"ollama-local"`, `"ollama"`, `"local"` -> `Some(FusionDefault)`
    /// - `"openrouter-default"`, `"openrouter"` -> `Some(OpenrouterDefault)`
    pub fn from_str_loose(s: &str) -> Option<Self> {
        let clean = s.trim().to_lowercase().replace('_', "-");
        match clean.as_str() {
            // DeepSeek Fast
            "deepseek-fast" | "deepseekfast" | "ds-fast" | "dsfast" | "deepseek" | "deepseek-chat"
            | "ds" => Some(Self::DeepseekFast),

            // DeepSeek Reasoning
            "deepseek-reasoning" | "deepseekreasoning" | "ds-reasoning" | "ds-reasoner"
            | "deepseek-reasoner" | "ds-r1" | "r1" => Some(Self::DeepseekReasoning),

            // Claude Sonnet
            "claude-sonnet" | "claudesonnet" | "sonnet" | "claude" | "claude-3-5-sonnet" => {
                Some(Self::ClaudeSonnet)
            }

            // Claude Haiku
            "claude-haiku" | "claudehaiku" | "haiku" | "claude-3-5-haiku" => Some(Self::ClaudeHaiku),

            // GPT-4o
            "gpt-4o" | "gpt4o" | "4o" | "openai" => Some(Self::Gpt4o),

            // Grok
            "grok" | "grok-2" | "grok2" | "xai" => Some(Self::Grok),

            // Fusion Default
            "fusion-default" | "fusiondefault" | "fusion" | "default" => {
                Some(Self::FusionDefault)
            }

            // OpenRouter Default
            "openrouter-default" | "openrouterdefault" | "openrouter" | "open-router" => {
                Some(Self::OpenrouterDefault)
            }

            _ => None,
        }
    }

    /// Converts this model preset into a complete [`Config`] instance
    /// initialized with its defaults.
    pub fn to_config(&self) -> Config {
        let mut cfg = Config::default();
        self.apply_to(&mut cfg);
        cfg
    }

    /// Applies this model preset's settings to an existing [`Config`],
    /// preserving existing API keys and custom provider base URLs.
    pub fn apply_to(&self, config: &mut Config) {
        config.default_provider = self.provider().to_string();
        config.default_model = self.model().to_string();
        config.default_temperature = self.temperature();
        config.max_tokens = self.max_tokens();
        config.advisors_enabled = self.advisors_enabled();
        config.advisor_model = self.advisor_model().map(|s| s.to_string());

        if let Some(fusion_url) = self.fusion_base_url() {
            if config.ollama_base_url.is_none() {
                config.ollama_base_url = Some(fusion_url.to_string());
            }
        }
    }

    /// Returns a structured metadata record for introspection and display.
    pub fn info(&self) -> ModelPresetInfo {
        ModelPresetInfo {
            id: self.id(),
            title: self.title(),
            description: self.description(),
            provider: self.provider(),
            model: self.model(),
            temperature: self.temperature(),
            max_tokens: self.max_tokens(),
            context_window: self.context_window(),
            advisors_enabled: self.advisors_enabled(),
            advisor_model: self.advisor_model(),
            recommended_for: self.recommended_for(),
        }
    }
}

impl fmt::Display for ModelPreset {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.id())
    }
}

impl FromStr for ModelPreset {
    type Err = PresetError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::from_str_loose(s).ok_or_else(|| PresetError::UnknownPreset {
            name: s.to_string(),
        })
    }
}

/// Metadata information about a named model preset.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ModelPresetInfo {
    pub id: &'static str,
    pub title: &'static str,
    pub description: &'static str,
    pub provider: &'static str,
    pub model: &'static str,
    pub temperature: Option<f32>,
    pub max_tokens: Option<u32>,
    pub context_window: Option<u64>,
    pub advisors_enabled: bool,
    pub advisor_model: Option<&'static str>,
    pub recommended_for: &'static str,
}

/// Returns a comma-separated list of all available model preset identifiers.
pub fn available_model_presets_list() -> String {
    ModelPreset::all()
        .iter()
        .map(|p| p.id())
        .collect::<Vec<_>>()
        .join(", ")
}

/// Formats a clean ASCII summary table of all available model presets.
pub fn format_model_presets_table() -> String {
    let mut out = String::new();
    out.push_str("┌──────────────────────┬────────────┬──────────────────────────┬────────────┬───────────┬──────────┐\n");
    out.push_str("│ Preset ID            │ Provider   │ Default Model            │ Max Tokens │ Context   │ Advisors │\n");
    out.push_str("├──────────────────────┼────────────┼──────────────────────────┼────────────┼───────────┼──────────┤\n");

    for p in ModelPreset::all() {
        let advisors = if p.advisors_enabled() { "Yes" } else { "No" };
        let temp = p
            .temperature()
            .map(|t| format!("{:.1}", t))
            .unwrap_or_else(|| "-".to_string());
        let tokens = p
            .max_tokens()
            .map(|t| format!("{}", t))
            .unwrap_or_else(|| "-".to_string());
        let context = p
            .context_window()
            .map(|c| format_context_tokens(c))
            .unwrap_or_else(|| "-".to_string());

        out.push_str(&format!(
            "│ {:<20} │ {:<10} │ {:<24} │ {:<10} │ {:<9} │ {:<8} │\n",
            p.id(),
            p.provider(),
            p.model(),
            tokens,
            context,
            advisors
        ));
    }

    out.push_str("└──────────────────────┴────────────┴──────────────────────────┴────────────┴───────────┴──────────┘\n");
    out
}

/// Formats a context-window token count as a compact human string (e.g. `200K`, `1M`).
fn format_context_tokens(tokens: u64) -> String {
    if tokens >= 1_000_000 && tokens % 1_000_000 == 0 {
        format!("{}M", tokens / 1_000_000)
    } else if tokens >= 1_000 && tokens % 1_000 == 0 {
        format!("{}K", tokens / 1_000)
    } else {
        format!("{}", tokens)
    }
}

impl fmt::Display for ConfigPreset {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.id())
    }
}

impl FromStr for ConfigPreset {
    type Err = PresetError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::from_str_loose(s).ok_or_else(|| PresetError::UnknownPreset {
            name: s.to_string(),
        })
    }
}

/// Metadata information about a configuration preset.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PresetInfo {
    pub id: &'static str,
    pub title: &'static str,
    pub description: &'static str,
    pub provider: &'static str,
    pub model: &'static str,
    pub temperature: Option<f32>,
    pub max_tokens: Option<u32>,
    pub advisors_enabled: bool,
    pub advisor_model: Option<&'static str>,
    pub sound_enabled: bool,
    pub bell_on_completion: bool,
    pub bell_on_error: bool,
    pub recommended_for: &'static str,
}

/// Errors occurring during preset lookup, parsing, or application.
#[derive(Debug, thiserror::Error, Clone, PartialEq, Eq)]
pub enum PresetError {
    #[error("Unknown configuration preset '{name}'.\nHint: Available presets are: {}", available_presets_list())]
    UnknownPreset { name: String },

    #[error("I/O error during preset operation: {0}")]
    IoError(String),

    #[error("JSON parsing error during preset operation: {0}")]
    ParseError(String),
}

/// Returns a comma-separated list of all available preset identifiers.
pub fn available_presets_list() -> String {
    ConfigPreset::all()
        .iter()
        .map(|p| p.id())
        .collect::<Vec<_>>()
        .join(", ")
}

/// Formats a clean ASCII summary table of all available configuration presets.
pub fn format_presets_table() -> String {
    let mut out = String::new();
    out.push_str("┌──────────────────┬────────────┬─────────────────────────────┬────────────┬──────────┬──────────┐\n");
    out.push_str("│ Preset ID        │ Provider   │ Default Model               │ Max Tokens │ Temp     │ Advisors │\n");
    out.push_str("├──────────────────┼────────────┼─────────────────────────────┼────────────┼──────────┼──────────┤\n");

    for p in ConfigPreset::all() {
        let advisors = if p.advisors_enabled() { "Yes" } else { "No" };
        let temp = p
            .temperature()
            .map(|t| format!("{:.1}", t))
            .unwrap_or_else(|| "-".to_string());
        let tokens = p
            .max_tokens()
            .map(|t| format!("{}", t))
            .unwrap_or_else(|| "-".to_string());

        out.push_str(&format!(
            "│ {:<16} │ {:<10} │ {:<27} │ {:<10} │ {:<8} │ {:<8} │\n",
            p.id(),
            p.provider(),
            p.model(),
            tokens,
            temp,
            advisors
        ));
    }

    out.push_str("└──────────────────┴────────────┴─────────────────────────────┴────────────┴──────────┴──────────┘\n");
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn test_preset_all_count() {
        assert_eq!(ConfigPreset::all().len(), 6);
    }

    #[test]
    fn test_preset_from_str_loose() {
        assert_eq!(
            ConfigPreset::from_str_loose("coding-fast"),
            Some(ConfigPreset::CodingFast)
        );
        assert_eq!(
            ConfigPreset::from_str_loose("coding_fast"),
            Some(ConfigPreset::CodingFast)
        );
        assert_eq!(
            ConfigPreset::from_str_loose("fast"),
            Some(ConfigPreset::CodingFast)
        );
        assert_eq!(
            ConfigPreset::from_str_loose("coding"),
            Some(ConfigPreset::CodingFast)
        );

        assert_eq!(
            ConfigPreset::from_str_loose("deep-reasoning"),
            Some(ConfigPreset::DeepReasoning)
        );
        assert_eq!(
            ConfigPreset::from_str_loose("deep_reasoning"),
            Some(ConfigPreset::DeepReasoning)
        );
        assert_eq!(
            ConfigPreset::from_str_loose("reasoning"),
            Some(ConfigPreset::DeepReasoning)
        );
        assert_eq!(
            ConfigPreset::from_str_loose("r1"),
            Some(ConfigPreset::DeepReasoning)
        );

        assert_eq!(
            ConfigPreset::from_str_loose("cheap"),
            Some(ConfigPreset::Cheap)
        );
        assert_eq!(
            ConfigPreset::from_str_loose("budget"),
            Some(ConfigPreset::Cheap)
        );
        assert_eq!(
            ConfigPreset::from_str_loose("low-cost"),
            Some(ConfigPreset::Cheap)
        );

        assert_eq!(
            ConfigPreset::from_str_loose("offline-ollama"),
            Some(ConfigPreset::OfflineOllama)
        );
        assert_eq!(
            ConfigPreset::from_str_loose("ollama"),
            Some(ConfigPreset::OfflineOllama)
        );
        assert_eq!(
            ConfigPreset::from_str_loose("offline"),
            Some(ConfigPreset::OfflineOllama)
        );
        assert_eq!(
            ConfigPreset::from_str_loose("local"),
            Some(ConfigPreset::OfflineOllama)
        );

        assert_eq!(
            ConfigPreset::from_str_loose("termux-mobile"),
            Some(ConfigPreset::TermuxMobile)
        );
        assert_eq!(
            ConfigPreset::from_str_loose("termux"),
            Some(ConfigPreset::TermuxMobile)
        );
        assert_eq!(
            ConfigPreset::from_str_loose("mobile"),
            Some(ConfigPreset::TermuxMobile)
        );
        assert_eq!(
            ConfigPreset::from_str_loose("android"),
            Some(ConfigPreset::TermuxMobile)
        );

        assert_eq!(ConfigPreset::from_str_loose("unknown-preset-xyz"), None);
    }

    #[test]
    fn test_coding_fast_preset_properties() {
        let preset = ConfigPreset::CodingFast;
        assert_eq!(preset.id(), "coding-fast");
        assert_eq!(preset.provider(), "anthropic");
        assert_eq!(preset.model(), "claude-3-5-sonnet-20241022");
        assert_eq!(preset.temperature(), Some(0.2));
        assert_eq!(preset.max_tokens(), Some(8192));
        assert!(preset.advisors_enabled());
        assert_eq!(preset.advisor_model(), Some("claude-3-5-haiku-20241022"));

        let cfg = preset.to_config();
        assert_eq!(cfg.default_provider, "anthropic");
        assert_eq!(cfg.default_model, "claude-3-5-sonnet-20241022");
        assert_eq!(cfg.default_temperature, Some(0.2));
        assert_eq!(cfg.max_tokens, Some(8192));
        assert!(cfg.advisors_enabled);
    }

    #[test]
    fn test_deep_reasoning_preset_properties() {
        let preset = ConfigPreset::DeepReasoning;
        assert_eq!(preset.id(), "deep-reasoning");
        assert_eq!(preset.provider(), "deepseek");
        assert_eq!(preset.model(), "deepseek-reasoner");
        assert_eq!(preset.temperature(), Some(0.0));
        assert_eq!(preset.max_tokens(), Some(16384));
        assert!(preset.advisors_enabled());

        let cfg = preset.to_config();
        assert_eq!(cfg.default_provider, "deepseek");
        assert_eq!(cfg.default_model, "deepseek-reasoner");
        assert_eq!(cfg.default_temperature, Some(0.0));
        assert_eq!(cfg.max_tokens, Some(16384));
        assert!(cfg.advisors_enabled);
    }

    #[test]
    fn test_cheap_preset_properties() {
        let preset = ConfigPreset::Cheap;
        assert_eq!(preset.id(), "cheap");
        assert_eq!(preset.provider(), "deepseek");
        assert_eq!(preset.model(), "deepseek-chat");
        assert_eq!(preset.temperature(), Some(0.1));
        assert_eq!(preset.max_tokens(), Some(4096));
        assert!(!preset.advisors_enabled());

        let cfg = preset.to_config();
        assert_eq!(cfg.default_provider, "deepseek");
        assert_eq!(cfg.default_model, "deepseek-chat");
        assert_eq!(cfg.default_temperature, Some(0.1));
        assert_eq!(cfg.max_tokens, Some(4096));
        assert!(!cfg.advisors_enabled);
    }

    #[test]
    fn test_offline_ollama_preset_properties() {
        let preset = ConfigPreset::OfflineOllama;
        assert_eq!(preset.id(), "offline-ollama");
        assert_eq!(preset.provider(), "ollama");
        assert_eq!(preset.model(), "qwen2.5-coder");
        assert_eq!(preset.temperature(), Some(0.2));
        assert_eq!(preset.max_tokens(), Some(4096));
        assert!(!preset.advisors_enabled());
        assert_eq!(preset.ollama_base_url(), Some("http://localhost:11434"));

        let cfg = preset.to_config();
        assert_eq!(cfg.default_provider, "ollama");
        assert_eq!(cfg.default_model, "qwen2.5-coder");
        assert_eq!(cfg.default_temperature, Some(0.2));
        assert_eq!(cfg.max_tokens, Some(4096));
        assert!(!cfg.advisors_enabled);
        assert_eq!(
            cfg.ollama_base_url.as_deref(),
            Some("http://localhost:11434")
        );
    }

    #[test]
    fn test_termux_mobile_preset_properties() {
        let preset = ConfigPreset::TermuxMobile;
        assert_eq!(preset.id(), "termux-mobile");
        assert_eq!(preset.provider(), "deepseek");
        assert_eq!(preset.model(), "deepseek-chat");
        assert_eq!(preset.temperature(), Some(0.2));
        assert_eq!(preset.max_tokens(), Some(2048));
        assert!(!preset.advisors_enabled());
        assert!(preset.sound_enabled());
        assert!(preset.bell_on_completion());
        assert!(preset.bell_on_error());

        let cfg = preset.to_config();
        assert_eq!(cfg.default_provider, "deepseek");
        assert_eq!(cfg.default_model, "deepseek-chat");
        assert_eq!(cfg.default_temperature, Some(0.2));
        assert_eq!(cfg.max_tokens, Some(2048));
        assert!(!cfg.advisors_enabled);
        assert!(cfg.sound_enabled);
    }

    #[test]
    fn test_apply_to_preserves_api_keys() {
        let mut cfg = Config::default();
        cfg.openai_api_key = Some("sk-test-openai".to_string());
        cfg.anthropic_api_key = Some("sk-test-anthropic".to_string());
        cfg.deepseek_api_key = Some("sk-test-deepseek".to_string());

        ConfigPreset::DeepReasoning.apply_to(&mut cfg);

        assert_eq!(cfg.default_provider, "deepseek");
        assert_eq!(cfg.default_model, "deepseek-reasoner");
        assert_eq!(cfg.openai_api_key.as_deref(), Some("sk-test-openai"));
        assert_eq!(
            cfg.anthropic_api_key.as_deref(),
            Some("sk-test-anthropic")
        );
        assert_eq!(cfg.deepseek_api_key.as_deref(), Some("sk-test-deepseek"));
    }

    #[test]
    fn test_apply_to_file() {
        let dir = tempdir().unwrap();
        let config_file = dir.path().join("config.json");

        // Write an initial config
        let initial = Config {
            openai_api_key: Some("sk-persisted".to_string()),
            ..Config::default()
        };
        std::fs::write(
            &config_file,
            serde_json::to_string_pretty(&initial).unwrap(),
        )
        .unwrap();

        // Apply preset
        let updated =
            ConfigPreset::apply_to_file(&config_file, ConfigPreset::Cheap).unwrap();
        assert_eq!(updated.default_provider, "deepseek");
        assert_eq!(updated.default_model, "deepseek-chat");
        assert_eq!(updated.max_tokens, Some(4096));
        assert!(!updated.advisors_enabled);
        assert_eq!(updated.openai_api_key.as_deref(), Some("sk-persisted"));

        // Verify disk content
        let reloaded_str = std::fs::read_to_string(&config_file).unwrap();
        let reloaded: Config = serde_json::from_str(&reloaded_str).unwrap();
        assert_eq!(reloaded.default_provider, "deepseek");
        assert_eq!(reloaded.openai_api_key.as_deref(), Some("sk-persisted"));
    }

    #[test]
    fn test_format_presets_table() {
        let table = format_presets_table();
        assert!(table.contains("coding-fast"));
        assert!(table.contains("deep-reasoning"));
        assert!(table.contains("cheap"));
        assert!(table.contains("offline-ollama"));
        assert!(table.contains("termux-mobile"));
    }

    #[test]
    fn test_preset_from_str_trait() {
        let parsed: Result<ConfigPreset, _> = "coding-fast".parse();
        assert_eq!(parsed.unwrap(), ConfigPreset::CodingFast);

        let err: Result<ConfigPreset, _> = "invalid-preset".parse();
        assert!(err.is_err());
    }

    #[test]
    fn test_preset_info_struct() {
        let info = ConfigPreset::CodingFast.info();
        assert_eq!(info.id, "coding-fast");
        assert_eq!(info.provider, "anthropic");
        assert_eq!(info.model, "claude-3-5-sonnet-20241022");
    }

    #[test]
    fn test_model_preset_all_count() {
        assert_eq!(ModelPreset::all().len(), 8);
    }

    #[test]
    fn test_model_preset_lookup_by_id() {
        for preset in ModelPreset::all() {
            assert_eq!(ModelPreset::lookup(preset.id()), Some(*preset));
        }
        assert_eq!(ModelPreset::lookup("deepseek-fast"), Some(ModelPreset::DeepseekFast));
        assert_eq!(
            ModelPreset::lookup("deepseek-reasoning"),
            Some(ModelPreset::DeepseekReasoning)
        );
        assert_eq!(ModelPreset::lookup("claude-sonnet"), Some(ModelPreset::ClaudeSonnet));
        assert_eq!(ModelPreset::lookup("claude-haiku"), Some(ModelPreset::ClaudeHaiku));
        assert_eq!(ModelPreset::lookup("gpt-4o"), Some(ModelPreset::Gpt4o));
        assert_eq!(ModelPreset::lookup("grok"), Some(ModelPreset::Grok));
        assert_eq!(ModelPreset::lookup("fusion-default"), Some(ModelPreset::FusionDefault));
        assert_eq!(
            ModelPreset::lookup("openrouter-default"),
            Some(ModelPreset::OpenrouterDefault)
        );
        assert_eq!(ModelPreset::lookup("nonexistent-preset"), None);
    }

    #[test]
    fn test_model_preset_from_str_loose() {
        assert_eq!(
            ModelPreset::from_str_loose("deepseek-fast"),
            Some(ModelPreset::DeepseekFast)
        );
        assert_eq!(
            ModelPreset::from_str_loose("deepseek_fast"),
            Some(ModelPreset::DeepseekFast)
        );
        assert_eq!(ModelPreset::from_str_loose("deepseek"), Some(ModelPreset::DeepseekFast));

        assert_eq!(
            ModelPreset::from_str_loose("deepseek-reasoning"),
            Some(ModelPreset::DeepseekReasoning)
        );
        assert_eq!(ModelPreset::from_str_loose("r1"), Some(ModelPreset::DeepseekReasoning));

        assert_eq!(
            ModelPreset::from_str_loose("claude-sonnet"),
            Some(ModelPreset::ClaudeSonnet)
        );
        assert_eq!(ModelPreset::from_str_loose("sonnet"), Some(ModelPreset::ClaudeSonnet));

        assert_eq!(
            ModelPreset::from_str_loose("claude-haiku"),
            Some(ModelPreset::ClaudeHaiku)
        );
        assert_eq!(ModelPreset::from_str_loose("haiku"), Some(ModelPreset::ClaudeHaiku));

        assert_eq!(ModelPreset::from_str_loose("gpt-4o"), Some(ModelPreset::Gpt4o));
        assert_eq!(ModelPreset::from_str_loose("gpt4o"), Some(ModelPreset::Gpt4o));

        assert_eq!(ModelPreset::from_str_loose("grok"), Some(ModelPreset::Grok));
        assert_eq!(ModelPreset::from_str_loose("xai"), Some(ModelPreset::Grok));

        assert_eq!(
            ModelPreset::from_str_loose("ollama-local"),
            Some(ModelPreset::FusionDefault)
        );
        assert_eq!(ModelPreset::from_str_loose("fusion"), Some(ModelPreset::FusionDefault));

        assert_eq!(
            ModelPreset::from_str_loose("openrouter-default"),
            Some(ModelPreset::OpenrouterDefault)
        );
        assert_eq!(
            ModelPreset::from_str_loose("openrouter"),
            Some(ModelPreset::OpenrouterDefault)
        );

        assert_eq!(ModelPreset::from_str_loose("unknown-model-xyz"), None);
    }

    #[test]
    fn test_model_preset_deepseek_fast_properties() {
        let preset = ModelPreset::DeepseekFast;
        assert_eq!(preset.id(), "deepseek-fast");
        assert_eq!(preset.provider(), "deepseek");
        assert_eq!(preset.model(), "deepseek-chat");
        assert_eq!(preset.temperature(), Some(0.3));
        assert_eq!(preset.max_tokens(), Some(8192));
        assert_eq!(preset.context_window(), Some(64_000));
        assert!(preset.advisors_enabled());

        let cfg = preset.to_config();
        assert_eq!(cfg.default_provider, "deepseek");
        assert_eq!(cfg.default_model, "deepseek-chat");
        assert_eq!(cfg.default_temperature, Some(0.3));
        assert_eq!(cfg.max_tokens, Some(8192));
        assert!(cfg.advisors_enabled);
    }

    #[test]
    fn test_model_preset_deepseek_reasoning_properties() {
        let preset = ModelPreset::DeepseekReasoning;
        assert_eq!(preset.id(), "deepseek-reasoning");
        assert_eq!(preset.provider(), "deepseek");
        assert_eq!(preset.model(), "deepseek-reasoner");
        assert_eq!(preset.temperature(), Some(0.0));
        assert_eq!(preset.max_tokens(), Some(16384));
        assert_eq!(preset.context_window(), Some(64_000));
        assert!(preset.advisors_enabled());

        let cfg = preset.to_config();
        assert_eq!(cfg.default_provider, "deepseek");
        assert_eq!(cfg.default_model, "deepseek-reasoner");
        assert_eq!(cfg.default_temperature, Some(0.0));
        assert_eq!(cfg.max_tokens, Some(16384));
        assert!(cfg.advisors_enabled);
    }

    #[test]
    fn test_model_preset_claude_sonnet_properties() {
        let preset = ModelPreset::ClaudeSonnet;
        assert_eq!(preset.id(), "claude-sonnet");
        assert_eq!(preset.provider(), "anthropic");
        assert_eq!(preset.model(), "claude-3-5-sonnet-20241022");
        assert_eq!(preset.temperature(), Some(0.2));
        assert_eq!(preset.max_tokens(), Some(8192));
        assert_eq!(preset.context_window(), Some(200_000));
        assert!(preset.advisors_enabled());
        assert_eq!(preset.advisor_model(), Some("claude-3-5-haiku-20241022"));

        let cfg = preset.to_config();
        assert_eq!(cfg.default_provider, "anthropic");
        assert_eq!(cfg.default_model, "claude-3-5-sonnet-20241022");
        assert_eq!(cfg.default_temperature, Some(0.2));
        assert_eq!(cfg.max_tokens, Some(8192));
        assert!(cfg.advisors_enabled);
        assert_eq!(cfg.advisor_model.as_deref(), Some("claude-3-5-haiku-20241022"));
    }

    #[test]
    fn test_model_preset_claude_haiku_properties() {
        let preset = ModelPreset::ClaudeHaiku;
        assert_eq!(preset.id(), "claude-haiku");
        assert_eq!(preset.provider(), "anthropic");
        assert_eq!(preset.model(), "claude-3-5-haiku-20241022");
        assert_eq!(preset.temperature(), Some(0.2));
        assert_eq!(preset.max_tokens(), Some(4096));
        assert_eq!(preset.context_window(), Some(200_000));
        assert!(!preset.advisors_enabled());

        let cfg = preset.to_config();
        assert_eq!(cfg.default_provider, "anthropic");
        assert_eq!(cfg.default_model, "claude-3-5-haiku-20241022");
        assert_eq!(cfg.default_temperature, Some(0.2));
        assert_eq!(cfg.max_tokens, Some(4096));
        assert!(!cfg.advisors_enabled);
    }

    #[test]
    fn test_model_preset_gpt4o_properties() {
        let preset = ModelPreset::Gpt4o;
        assert_eq!(preset.id(), "gpt-4o");
        assert_eq!(preset.provider(), "openai");
        assert_eq!(preset.model(), "gpt-4o");
        assert_eq!(preset.temperature(), Some(0.2));
        assert_eq!(preset.max_tokens(), Some(16384));
        assert_eq!(preset.context_window(), Some(128_000));
        assert!(preset.advisors_enabled());

        let cfg = preset.to_config();
        assert_eq!(cfg.default_provider, "openai");
        assert_eq!(cfg.default_model, "gpt-4o");
        assert_eq!(cfg.default_temperature, Some(0.2));
        assert_eq!(cfg.max_tokens, Some(16384));
        assert!(cfg.advisors_enabled);
    }

    #[test]
    fn test_model_preset_grok_properties() {
        let preset = ModelPreset::Grok;
        assert_eq!(preset.id(), "grok");
        assert_eq!(preset.provider(), "xai");
        assert_eq!(preset.model(), "grok-2-latest");
        assert_eq!(preset.temperature(), Some(0.1));
        assert_eq!(preset.max_tokens(), Some(8192));
        assert_eq!(preset.context_window(), Some(131_072));
        assert!(!preset.advisors_enabled());

        let cfg = preset.to_config();
        assert_eq!(cfg.default_provider, "xai");
        assert_eq!(cfg.default_model, "grok-2-latest");
        assert_eq!(cfg.default_temperature, Some(0.1));
        assert_eq!(cfg.max_tokens, Some(8192));
        assert!(!cfg.advisors_enabled);
    }

    #[test]
    fn test_model_preset_fusion_default_properties() {
        let preset = ModelPreset::FusionDefault;
        assert_eq!(preset.id(), "fusion-default");
        assert_eq!(preset.provider(), "fusion");
        assert_eq!(preset.model(), "fusion-chat");
        assert_eq!(preset.temperature(), Some(0.2));
        assert_eq!(preset.max_tokens(), Some(4096));

        let cfg = preset.to_config();
        assert_eq!(cfg.default_provider, "fusion");
        assert_eq!(cfg.default_model, "fusion-chat");
        assert_eq!(cfg.default_temperature, Some(0.2));
        assert_eq!(cfg.max_tokens, Some(4096));
    }

    #[test]
    fn test_model_preset_openrouter_default_properties() {
        let preset = ModelPreset::OpenrouterDefault;
        assert_eq!(preset.id(), "openrouter-default");
        assert_eq!(preset.provider(), "openrouter");
        assert_eq!(preset.model(), "deepseek/deepseek-chat");
        assert_eq!(preset.temperature(), Some(0.2));
        assert_eq!(preset.max_tokens(), Some(8192));
        assert_eq!(preset.context_window(), Some(64_000));
        assert!(!preset.advisors_enabled());

        let cfg = preset.to_config();
        assert_eq!(cfg.default_provider, "openrouter");
        assert_eq!(cfg.default_model, "deepseek/deepseek-chat");
        assert_eq!(cfg.default_temperature, Some(0.2));
        assert_eq!(cfg.max_tokens, Some(8192));
        assert!(!cfg.advisors_enabled);
    }

    #[test]
    fn test_model_preset_apply_to_preserves_api_keys() {
        let mut cfg = Config::default();
        cfg.openai_api_key = Some("sk-persisted-openai".to_string());
        cfg.anthropic_api_key = Some("sk-persisted-anthropic".to_string());
        cfg.ollama_base_url = Some("http://custom-host:11434".to_string());

        ModelPreset::ClaudeSonnet.apply_to(&mut cfg);

        assert_eq!(cfg.default_provider, "anthropic");
        assert_eq!(cfg.default_model, "claude-3-5-sonnet-20241022");
        assert_eq!(cfg.openai_api_key.as_deref(), Some("sk-persisted-openai"));
        assert_eq!(cfg.anthropic_api_key.as_deref(), Some("sk-persisted-anthropic"));
        // Custom Ollama base URL must not be overwritten by a non-Ollama preset.
        assert_eq!(cfg.ollama_base_url.as_deref(), Some("http://custom-host:11434"));
    }

    #[test]
    fn test_model_preset_ollama_apply_sets_base_url() {
        let mut cfg = Config::default();
        cfg.ollama_base_url = None;

        ModelPreset::FusionDefault.apply_to(&mut cfg);

        assert_eq!(cfg.ollama_base_url.as_deref(), Some("http://localhost:11434"));
    }

    #[test]
    fn test_model_preset_display_and_from_str_trait() {
        let parsed: Result<ModelPreset, _> = "claude-sonnet".parse();
        assert_eq!(parsed.unwrap(), ModelPreset::ClaudeSonnet);

        let err: Result<ModelPreset, _> = "invalid-model-preset".parse();
        assert!(err.is_err());

        for preset in ModelPreset::all() {
            assert_eq!(preset.to_string(), preset.id());
        }
    }

    #[test]
    fn test_model_preset_info_struct() {
        let info = ModelPreset::Gpt4o.info();
        assert_eq!(info.id, "gpt-4o");
        assert_eq!(info.provider, "openai");
        assert_eq!(info.model, "gpt-4o");
        assert_eq!(info.temperature, Some(0.2));
        assert_eq!(info.max_tokens, Some(16384));
        assert_eq!(info.context_window, Some(128_000));
        assert!(info.advisors_enabled);
        assert_eq!(info.advisor_model, None);
    }

    #[test]
    fn test_available_model_presets_list() {
        let list = available_model_presets_list();
        assert!(list.contains("deepseek-fast"));
        assert!(list.contains("deepseek-reasoning"));
        assert!(list.contains("claude-sonnet"));
        assert!(list.contains("claude-haiku"));
        assert!(list.contains("gpt-4o"));
        assert!(list.contains("grok"));
        assert!(list.contains("fusion-default"));
        assert!(list.contains("openrouter-default"));
    }

    #[test]
    fn test_format_model_presets_table() {
        let table = format_model_presets_table();
        for preset in ModelPreset::all() {
            assert!(table.contains(preset.id()));
        }
        assert!(table.contains("Provider"));
        assert!(table.contains("Context"));
    }

    #[test]
    fn test_model_preset_ids_unique() {
        let ids: Vec<_> = ModelPreset::all().iter().map(|p| p.id()).collect();
        for (i, id) in ids.iter().enumerate() {
            for other in &ids[i + 1..] {
                assert_ne!(id, other, "duplicate preset id: {}", id);
            }
        }
    }

    #[test]
    fn test_model_preset_max_tokens_within_context() {
        for preset in ModelPreset::all() {
            if let (Some(ctx), Some(max)) = (preset.context_window(), preset.max_tokens()) {
                assert!(
                    (max as u64) <= ctx,
                    "{}: max_tokens {} exceeds context {}",
                    preset.id(),
                    max,
                    ctx
                );
            }
        }
    }

}
