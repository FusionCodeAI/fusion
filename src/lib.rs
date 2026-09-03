//! # Fusion
//!
//! Fusion is a fast, lightweight, cross-platform AI coding assistant engine written in pure Rust.
//! It features multi-agent orchestration, parallel advisor critiques, an extensible tool
//! sandbox, interactive inline Ratatui UI, Agent Client Protocol (ACP) JSON-RPC support for IDEs,
//! and optional WebAssembly browser bindings.
//!
//! ## Architectural Structure
//!
//! Fusion's codebase is structured as a strict Directed Acyclic Graph (DAG) with clean separation
//! of concerns and zero circular dependencies:
//!
//! ```text
//!              ┌─────────┐
//!              │ config  │ (Self-contained, serde-driven, migrations)
//!              └────┬────┘
//!                   │
//!              ┌────▼────┐
//!              │provider │ (Anthropic, OpenAI, OpenRouter, Ollama, DeepSeek, xAI)
//!              └────┬────┘
//!                   │
//!              ┌────▼────┐
//!              │  tools  │ (Bash, Read, Write, Edit, Grep, Glob, Git, Patch, Guardrails)
//!              └────┬────┘
//!                   │
//!              ┌────▼────┐
//!              │  agent  │ (LoopRunner, Session, Subagents, Mesh, Advisors, Compaction)
//!              └────┬────┘
//!             ┌─────┴─────┐
//!             │           │
//!        ┌────▼────┐ ┌────▼────┐
//!        │   ui    │ │   acp   │ (Inline Ratatui REPL & ACP JSON-RPC IDE server)
//!        └─────────┘ └─────────┘
//!
//!   (cli: Command-line parsing & shell completions, depends on clap/clap_complete)
//!   (wasm: Optional WebAssembly browser bindings under cfg(target_arch = "wasm32"))
//! ```
//!
//! ## Core Modules
//!
//! - [`config`]: Configuration management, environment variable resolution, model aliases, and schema migrations.
//! - [`provider`]: Unified multi-provider LLM client abstractions, streaming SSE parsers, and rate limiting.
//! - [`tools`]: Cross-platform execution sandbox, extensible tool registry, and built-in filesystem, search, git, and guardrail tools.
//! - [`agent`]: Core agent loop runner, session persistence, subagent orchestration, advisor critiques, token estimation, cost tracking, and context compaction.
//! - [`ui`]: Interactive terminal user interface with inline Ratatui rendering, streaming markdown parser, syntax highlighting, spinners, and slash commands.
//! - [`acp`]: Agent Client Protocol (ACP) JSON-RPC 2.0 stdio server adapter for editor and IDE integration (Zed, Neovim, VS Code, JetBrains).
//! - [`cli`]: Command-line interface definitions, shell completions, and argument parsing.
//! - [`wasm`]: Optional WebAssembly browser bindings for in-browser client execution.
//!
//! ## Prelude
//!
//! For ergonomic embedding and extension, import the prelude:
//!
//! ```rust,no_run
//! use fusion::prelude::*;
//! ```

pub mod acp;
pub mod agent;
pub mod cli;
pub mod config;
pub mod provider;
pub mod tools;
#[cfg(not(target_arch = "wasm32"))]
pub mod ui;

#[cfg(any(target_arch = "wasm32", feature = "wasm"))]
pub mod wasm;

// ---------------------------------------------------------------------------
// Top-Level Ergonomic Re-exports
// ---------------------------------------------------------------------------

pub use acp::AcpServer;
pub use agent::{AgentEvent, AgentRunner, Session};
pub use cli::Cli;
pub use config::{Config, ConfigPreset};
pub use provider::LlmClient;
pub use tools::{
    default_registry, CatTool, CreateTool, McpClient, McpManager, McpServerConfig, McpTool,
    StrReplaceEditorTool, TerminalTool, Tool, ToolContext, ToolRegistry, ViewTool,
};

// ---------------------------------------------------------------------------
// Prelude
// ---------------------------------------------------------------------------

/// Commonly used types, traits, and functions for building with or embedding Fusion.
pub mod prelude {
    pub use crate::acp::AcpServer;
    pub use crate::agent::{AgentEvent, AgentRunner, Session};
    pub use crate::cli::Cli;
    pub use crate::config::{Config, ConfigPreset};
    pub use crate::provider::types::{Message, Role, StreamChunk, ToolCall, ToolDefinition};
    pub use crate::provider::LlmClient;
    pub use crate::tools::{
        default_registry, CatTool, CreateTool, McpClient, McpManager, McpServerConfig, McpTool,
        StrReplaceEditorTool, TerminalTool, Tool, ToolContext, ToolRegistry, ViewTool,
    };
    pub use crate::ui::diff_view::{
        run_interactive_diff_viewer, DiffFile, DiffHunk, DiffLine, DiffLineType, DiffViewMode,
        DiffViewResult, DiffViewState, DiffViewerWidget, HunkStatus, SyntaxLanguage,
    };
    pub use crate::ui::side_by_side::{
        render_adaptive, render_diff_with_width, render_side_by_side_ansi, render_unified_ansi,
        AdaptiveDiffConfig, DiffBorderStyle, DiffChangeKind, DiffDisplayMode, DiffStats,
        SideBySideCell, SideBySideDocument, SideBySideHunk, SideBySideRow, SideBySideWidget,
    };
    pub use crate::ui::sound::{SoundConfig, SoundCue, SoundPlayer};
    pub use crate::ui::stats_card::{
        render_stats_card_ansi, render_stats_card_markdown, render_stats_card_plain,
        render_stats_compact_ansi, SessionStats, SessionStatsBuilder, SessionStatsCardState,
        SessionStatsCardWidget, StatsCardBorderStyle, StatsCardConfig, StatsCardLayout,
        ToolExecutionStat,
    };
    #[cfg(any(target_arch = "wasm32", feature = "wasm"))]
    pub use crate::wasm::WasmFusionAgent;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_top_level_re_exports() {
        let cfg = Config::default();
        assert!(!cfg.default_model.is_empty());
        assert!(!cfg.default_provider.is_empty());

        let session = Session::new(&cfg.default_model);
        assert_eq!(session.active_model, cfg.default_model);

        let registry = default_registry();
        assert!(registry.get("bash").is_some());
        assert!(registry.get("read").is_some());
        assert!(registry.get("write").is_some());
        assert!(registry.get("edit").is_some());
        assert!(registry.get("grep").is_some());
        assert!(registry.get("glob").is_some());
    }

    #[test]
    fn test_prelude_imports() {
        use crate::prelude::*;

        let cfg = Config::default();
        let session = Session::new(&cfg.default_model);
        assert_eq!(session.active_model, cfg.default_model);

        let msg = Message::user("Hello, Fusion!");
        assert_eq!(msg.role, Role::User);
        assert_eq!(msg.content, "Hello, Fusion!");
    }

    #[test]
    fn test_tool_context_creation() {
        use std::path::PathBuf;

        let ctx = ToolContext {
            cwd: PathBuf::from("."),
            env: std::collections::HashMap::new(),
        };
        assert_eq!(ctx.cwd, PathBuf::from("."));
        assert!(ctx.env.is_empty());
    }
}
