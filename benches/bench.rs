//! # Fusion v2 Comprehensive Subsystem Benchmark Suite
//!
//! High-throughput, precision Criterion benchmarks for core Fusion subsystems:
//! 1. **Cold Startup**: Config parsing, tool registry construction, and initial system prompt assembly.
//! 2. **Rendering Throughput**: Markdown & ANSI streaming throughput (tokens/sec and frame rendering times).
//! 3. **Tool Execution Latency**: Grep filter, file globbing, search indexing, and file manipulation.
//! 4. **Subagent Mesh & Routing**: Message dispatch, pub-sub broadcast routing, and peer coordination.
//!
//! Run benchmarks via:
//! `cargo bench --bench bench`

use std::collections::HashMap;
use std::fs::File;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::sync::Arc;
use std::time::Duration;

use criterion::{
    black_box, criterion_group, criterion_main, Criterion, Throughput,
};
use ratatui::backend::TestBackend;
use ratatui::layout::Rect;
use ratatui::Terminal;
use serde_json::json;

use fusion::agent::mesh::{
    topics, AgentMesh, AgentRole, AgentStatus, BroadcastMessage, BroadcastPayload, PeerResponse,
};
use fusion::agent::prompts::{PromptPreset, SystemPromptBuilder, RUST_SYSTEM_PROMPT};
use fusion::agent::search::{search_in_sessions, SearchQuery, SessionSearchIndex};
use fusion::agent::session::Session;
use fusion::agent::tokens::estimate_text_tokens;
use fusion::config::env_loader::expand_variables;
use fusion::config::migration::migrate_str;
use fusion::config::presets::{available_presets_list, ConfigPreset};
use fusion::config::Config;
use fusion::provider::anthropic::AnthropicStreamAccumulator;
use fusion::provider::types::Message;
use fusion::tools::bash::BashTool;
use fusion::tools::default_registry;
use fusion::tools::edit::{
    apply_exact_edit, compute_diff_stats, generate_colorized_diff, generate_unified_diff,
};
use fusion::tools::file::ReadFileTool;
use fusion::tools::glob::GlobTool;
use fusion::tools::grep::GrepTool;
use fusion::tools::grep_filter::{
    FileTypeRegistry, FilterableGrepEngine, GrepOptions, PathFilterBuilder,
};
use fusion::tools::symbols::SymbolScanner;
use fusion::tools::types::{Tool, ToolContext, ToolRegistry};
use fusion::ui::budget::{
    render_budget_banner_ansi, render_budget_status_pill_ansi, BannerBoxStyle, ContextAlert,
};
use fusion::ui::inline::{
    render_card_themed, render_critique_card_themed, render_status_bar_themed, StatusInfo,
};
use fusion::ui::markdown::{
    highlight_code_line, render_inline, render_line, render_markdown, MarkdownRenderer,
};
use fusion::ui::table::{
    render_markdown_table, strip_ansi, truncate_ansi, visible_width, wrap_ansi,
    MarkdownTableStreamer,
};
use fusion::ui::theme::Theme;
// ============================================================================
// Test Environment Fixtures & Synthetic Datasets
// ============================================================================

/// Helper RAII struct for managing benchmark test directories and files.
struct BenchFixture {
    temp_dir: tempfile::TempDir,
}

impl BenchFixture {
    fn new() -> Self {
        let temp_dir = tempfile::Builder::new()
            .prefix("fusion_bench_")
            .tempdir()
            .expect("failed to create temporary benchmark directory");
        Self { temp_dir }
    }

    fn path(&self) -> &Path {
        self.temp_dir.path()
    }

    fn context(&self) -> ToolContext {
        ToolContext {
            cwd: self.path().to_path_buf(),
            env: std::env::vars().collect(),
        }
    }

    /// Create a file with the given relative name and content.
    fn create_file(&self, rel_path: &str, content: &str) -> PathBuf {
        let full_path = self.path().join(rel_path);
        if let Some(parent) = full_path.parent() {
            std::fs::create_dir_all(parent).expect("failed to create parent directories");
        }
        let mut file = File::create(&full_path).expect("failed to create fixture file");
        file.write_all(content.as_bytes())
            .expect("failed to write fixture file content");
        full_path
    }

    /// Populate a multi-directory tree suitable for grep, glob, and symbol benchmarks.
    fn populate_search_tree(&self) {
        // Root files
        self.create_file(
            "Cargo.toml",
            "[package]\nname = \"bench_fixture\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
        );
        self.create_file(
            "README.md",
            "# Bench Fixture\nThis is a test repository for benchmarks.\nTARGET_KEYWORD found here.\n",
        );
        self.create_file(".gitignore", "target/\n*.tmp\n.fusion/\n");

        // src/ tree
        self.create_file(
            "src/main.rs",
            r#"
use bench_fixture::service::Service;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    println!("Hello bench!");
    let svc = Service::new("http://127.0.0.1:8080");
    svc.run().await?;
    let x = bench_fixture::math::calculate_val(10);
    println!("Result: {x}");
    Ok(())
}
"#,
        );
        self.create_file(
            "src/lib.rs",
            r#"
pub mod math;
pub mod util;
pub mod service;

pub fn add(a: i32, b: i32) -> i32 {
    a + b
}

pub struct Config {
    pub host: String,
    pub port: u16,
}
"#,
        );
        self.create_file(
            "src/math.rs",
            r#"
pub fn calculate_val(n: usize) -> usize {
    (0..n).map(|x| x * 2).sum()
}

pub fn multiply(a: i64, b: i64) -> i64 {
    a * b
}

pub fn divide(a: f64, b: f64) -> Option<f64> {
    if b == 0.0 { None } else { Some(a / b) }
}
"#,
        );
        self.create_file(
            "src/util.rs",
            r#"
pub fn format_greeting(name: &str) -> String {
    format!("Hello, {}!", name)
}

// TARGET_KEYWORD appears in comment
pub fn sanitize_input(raw: &str) -> String {
    raw.trim().to_lowercase()
}
"#,
        );
        self.create_file(
            "src/service/mod.rs",
            r#"
pub mod client;

pub struct Service {
    endpoint: String,
}

impl Service {
    pub fn new(endpoint: impl Into<String>) -> Self {
        Self { endpoint: endpoint.into() }
    }

    pub async fn run(&self) -> anyhow::Result<()> {
        Ok(())
    }
}
"#,
        );
        self.create_file(
            "src/service/client.rs",
            r#"
pub struct Client {
    pub endpoint: String,
    pub timeout_ms: u64,
}

impl Client {
    pub fn new(endpoint: String) -> Self {
        Self { endpoint, timeout_ms: 5000 }
    }
}

// Another TARGET_KEYWORD in client
"#,
        );

        // tests/ tree
        self.create_file(
            "tests/integration_test.rs",
            r#"
#[test]
fn test_fixture_math() {
    assert_eq!(2 + 2, 4);
}
"#,
        );
        self.create_file(
            "tests/common/mod.rs",
            r#"
pub fn setup_test_env() {
    // Test setup logic
}
"#,
        );

        // docs/ tree
        self.create_file(
            "docs/architecture.md",
            "# Architecture\nOverview of the system design.\nTARGET_KEYWORD in documentation.\n",
        );
        self.create_file(
            "docs/api/v1.md",
            "# API v1\nEndpoints specification and interface guidelines.\n",
        );
    }
}

/// Generate synthetic code content with a specified number of lines.
fn generate_code_content(lines: usize) -> String {
    let mut out = String::with_capacity(lines * 45);
    out.push_str("// Generated source file for benchmarks\n");
    out.push_str("use std::collections::HashMap;\n\n");
    for i in 1..=lines {
        if i % 10 == 0 {
            out.push_str(&format!("pub fn function_marker_{i}() -> usize {{ {i} }}\n"));
        } else if i % 5 == 0 {
            out.push_str(&format!("    let var_{i} = \"value_{i}\";\n"));
        } else {
            out.push_str(&format!("    // Line {i}: standard statement and processing logic\n"));
        }
    }
    out
}

/// Generate a synthetic multi-turn session with realistic message payloads.
fn generate_test_session(title: &str, model: &str, turn_count: usize) -> Session {
    let mut session = Session::new(model);
    session.set_title(title);
    session.system_prompt = Some(RUST_SYSTEM_PROMPT.to_string());

    for i in 1..=turn_count {
        session.messages.push(Message::user(format!(
            "Turn {i}: How do I optimize lock contention in Tokio and prevent deadlocks with Mutex in Rust?"
        )));
        session.messages.push(Message::assistant(format!(
            "Turn {i}: To minimize lock contention, prefer `tokio::sync::RwLock` for read-heavy workloads, \
             keep critical sections small, and avoid holding locks across `.await` points. \
             Consider message-passing channels (`mpsc`, `broadcast`) or sharded data structures."
        )));
    }
    session
}

const SHORT_MARKDOWN: &str = r#"
### Quick Overview
This is **bold** text and *italic* text, plus `inline_code()` and [Fusion Link](https://github.com/theaungmyatmoe/fusion).
- Bullet item 1
- Bullet item 2
"#;

const MEDIUM_MARKDOWN: &str = r#"
# Project Specification & Overview

Welcome to the **Fusion** assistant guide. This engine provides fast terminal AI interactions.

## Key Features
- **Speed**: Optimized Rust binary with pure-Rust dependencies.
- **Portability**: Runs on Linux, macOS, Windows, and Android (Termux).
- **Subagents**: Parallel task execution mesh with advisory system.

### Tasks List
- [x] Implement tool benchmarks
- [x] Stream parser validation
- [ ] Complete documentation

### Code Sample
```rust
use std::collections::HashMap;

pub fn process_data(input: &[String]) -> HashMap<String, usize> {
    let mut map = HashMap::new();
    for item in input {
        *map.entry(item.clone()).or_insert(0) += 1;
    }
    map
}
```

> "Make it work, make it right, make it fast." — Kent Beck

### Quick Table
| Component | Status | Latency |
|:---|:---:|---:|
| Bash Tool | Active | < 5ms |
| Grep Tool | Active | < 10ms |
| SSE Parser | Active | < 1ms |
"#;

fn generate_large_markdown() -> String {
    let mut doc = String::with_capacity(35 * 1024);
    doc.push_str("# Comprehensive Architecture Document\n\n");
    doc.push_str("Detailed analysis and technical documentation across multiple subsections.\n\n");

    for section in 1..=15 {
        doc.push_str(&format!("## Section {section}: Subsystem Architecture\n\n"));
        doc.push_str("This module handles core communication, serialization, and stream decoding.\n");
        doc.push_str("Key considerations include **zero-allocation** paths, *cache-friendly* layouts, and robust error recovery.\n\n");

        doc.push_str("### Implementation Details\n");
        doc.push_str("- Point 1: Handle input validation and sanitization.\n");
        doc.push_str("- Point 2: Dispatch async background tasks using `tokio::spawn`.\n");
        doc.push_str("- Point 3: Record performance metrics and execution traces.\n\n");

        if section % 2 == 0 {
            doc.push_str("```rust\n");
            doc.push_str("pub async fn execute_step(id: u64, name: &str) -> Result<String, Error> {\n");
            doc.push_str("    let client = HttpClient::new();\n");
            doc.push_str("    let res = client.get(&format!(\"/api/{id}\")).await?;\n");
            doc.push_str("    Ok(res.text().await?)\n");
            doc.push_str("}\n");
            doc.push_str("```\n\n");
        } else {
            doc.push_str("```python\n");
            doc.push_str("def process_records(records):\n");
            doc.push_str("    return [r for r in records if r.get('active', False)]\n");
            doc.push_str("```\n\n");
        }

        doc.push_str("| Metric | Target | Actual |\n");
        doc.push_str("|:---|:---:|---:|\n");
        doc.push_str(&format!("| Latency S{section} | < 10ms | 3.2ms |\n"));
        doc.push_str(&format!("| Memory S{section} | < 50MB | 12.4MB |\n\n"));
    }

    doc
}

// ============================================================================
// 1. Cold Startup Benchmarks
// ============================================================================

fn bench_cold_startup(c: &mut Criterion) {
    let mut group = c.benchmark_group("cold_startup");

    // 1.1 Config Parsing Benchmarks
    let minimal_config_json = r#"{
        "version": 2,
        "default_provider": "deepseek",
        "default_model": "deepseek-chat"
    }"#;

    let full_config_json = r#"{
        "version": 2,
        "default_provider": "anthropic",
        "default_model": "claude-3-7-sonnet-20250219",
        "default_temperature": 0.2,
        "max_tokens": 8192,
        "openai_api_key": "sk-proj-bench-key-sample-value",
        "openai_base_url": "https://api.openai.com/v1",
        "anthropic_api_key": "sk-ant-bench-key-sample-value",
        "anthropic_base_url": "https://api.anthropic.com",
        "deepseek_api_key": "sk-deepseek-sample-key",
        "deepseek_base_url": "https://api.deepseek.com",
        "xai_api_key": "xai-sample-key",
        "openrouter_api_key": "sk-or-sample-key",
        "ollama_base_url": "http://localhost:11434",
        "advisors_enabled": true,
        "advisor_model": "deepseek-reasoner",
        "sound_enabled": false,
        "bell_on_completion": true,
        "bell_on_error": true,
        "notify_enabled": true,
        "notify_on_completion": true,
        "notify_on_error": true,
        "notify_min_duration_secs": 3.0
    }"#;

    group.throughput(Throughput::Bytes(full_config_json.len() as u64));
    group.bench_function("config_parsing/default_struct", |b| {
        b.iter(|| {
            let cfg = Config::default();
            black_box(cfg);
        });
    });

    group.bench_function("config_parsing/json_minimal", |b| {
        b.iter(|| {
            let cfg: Config = serde_json::from_str(black_box(minimal_config_json)).unwrap();
            black_box(cfg);
        });
    });

    group.bench_function("config_parsing/json_full_multi_provider", |b| {
        b.iter(|| {
            let cfg: Config = serde_json::from_str(black_box(full_config_json)).unwrap();
            black_box(cfg);
        });
    });

    let mut env_context = HashMap::new();
    env_context.insert("API_HOST".to_string(), "api.deepseek.com".to_string());
    env_context.insert("PORT".to_string(), "443".to_string());
    let env_template = "https://${API_HOST:-localhost}:${PORT:-8080}/v1/chat/completions";

    group.bench_function("config_parsing/env_variable_expansion", |b| {
        b.iter(|| {
            let expanded = expand_variables(black_box(env_template), black_box(&env_context)).unwrap();
            black_box(expanded);
        });
    });

    group.bench_function("config_parsing/preset_resolution", |b| {
        b.iter(|| {
            let p1 = ConfigPreset::from_str(black_box("coding-fast")).unwrap();
            let p2 = ConfigPreset::from_str(black_box("deep-reasoning")).unwrap();
            let p3 = ConfigPreset::from_str(black_box("cheap")).unwrap();
            let p4 = ConfigPreset::from_str(black_box("offline-ollama")).unwrap();
            let p5 = ConfigPreset::from_str(black_box("termux-mobile")).unwrap();
            let all = available_presets_list();
            black_box((p1, p2, p3, p4, p5, all));
        });
    });

    let legacy_v1_json = r#"{"model_provider": "openai", "model": "gpt-4o", "temperature": 0.5}"#;
    group.bench_function("config_parsing/schema_migration", |b| {
        b.iter(|| {
            let (migrated, outcome) = migrate_str(black_box(legacy_v1_json)).unwrap();
            black_box((migrated, outcome));
        });
    });

    // 1.2 Tool Registry Construction Benchmarks
    group.bench_function("tool_registry/manual_register_builtins", |b| {
        b.iter(|| {
            let mut registry = ToolRegistry::new();
            registry.register(Arc::new(BashTool::new()));
            registry.register(Arc::new(ReadFileTool::new()));
            registry.register(Arc::new(WriteFileTool::new()));
            registry.register(Arc::new(EditFileTool::new()));
            registry.register(Arc::new(GrepTool::new()));
            registry.register(Arc::new(GlobTool::new()));
            black_box(registry);
        });
    });

    group.bench_function("tool_registry/default_factory", |b| {
        b.iter(|| {
            let reg = default_registry();
            black_box(reg);
        });
    });

    let registry = default_registry();
    group.bench_function("tool_registry/lookup_and_alias_resolution", |b| {
        b.iter(|| {
            let t1 = registry.get(black_box("bash"));
            let t2 = registry.get(black_box("read"));
            let t3 = registry.get(black_box("read_file"));
            let t4 = registry.get(black_box("grep"));
            let t5 = registry.get(black_box("glob"));
            let t6 = registry.get(black_box("edit"));
            let t7 = registry.get(black_box("edit_file"));
            let t8 = registry.get(black_box("diff"));
            let t9 = registry.get(black_box("status"));
            let t10 = registry.get(black_box("symbols"));
            black_box((t1, t2, t3, t4, t5, t6, t7, t8, t9, t10));
        });
    });

    group.bench_function("tool_registry/definitions_serialization", |b| {
        b.iter(|| {
            let defs = registry.definitions();
            black_box(defs);
        });
    });

    // 1.3 Initial System Prompt Assembly Benchmarks
    group.bench_function("system_prompt/builder_general", |b| {
        b.iter(|| {
            let prompt = SystemPromptBuilder::new().build();
            black_box(prompt);
        });
    });

    let workspace_ctx = "Project: Fusion AI Coding Assistant\nArchitecture: Pure Rust CLI + TUI + Subagents\nTarget OS: macOS, Linux, Windows, Android Termux";
    let tool_instructions = "Available tools: bash, read, write, edit, grep, glob, git_diff, patch, symbols, process.";
    let advisor_critique = "Architecture Advisor: Approved. Ensure zero-allocation inner loops and clean error handling.";

    group.bench_function("system_prompt/builder_domain_rust_full", |b| {
        b.iter(|| {
            let prompt = SystemPromptBuilder::new()
                .with_preset(black_box(PromptPreset::Rust))
                .with_workspace_context(black_box(workspace_ctx))
                .with_tool_instructions(black_box(tool_instructions))
                .with_advisor_critiques(black_box(advisor_critique))
                .build();
            black_box(prompt);
        });
    });

    group.bench_function("system_prompt/builder_domain_typescript", |b| {
        b.iter(|| {
            let prompt = SystemPromptBuilder::new()
                .with_preset(black_box(PromptPreset::TypeScript))
                .with_workspace_context(black_box(workspace_ctx))
                .with_tool_instructions(black_box(tool_instructions))
                .build();
            black_box(prompt);
        });
    });

    group.bench_function("system_prompt/builder_domain_termux_mobile", |b| {
        b.iter(|| {
            let prompt = SystemPromptBuilder::new()
                .with_preset(black_box(PromptPreset::Termux))
                .with_termux(black_box(true))
                .with_workspace_context(black_box(workspace_ctx))
                .build();
            black_box(prompt);
        });
    });

    group.bench_function("system_prompt/manifest_auto_detection", |b| {
        b.iter(|| {
            let p1 = PromptPreset::from_manifest_filename(black_box("Cargo.toml"));
            let p2 = PromptPreset::from_manifest_filename(black_box("package.json"));
            let p3 = PromptPreset::from_manifest_filename(black_box("pyproject.toml"));
            let p4 = PromptPreset::from_manifest_filename(black_box("go.mod"));
            let p5 = PromptPreset::from_manifest_filename(black_box("termux.properties"));
            black_box((p1, p2, p3, p4, p5));
        });
    });

    let assembled_prompt = SystemPromptBuilder::new()
        .with_preset(PromptPreset::Rust)
        .with_workspace_context(workspace_ctx)
        .with_tool_instructions(tool_instructions)
        .with_advisor_critiques(advisor_critique)
        .build();

    group.throughput(Throughput::Bytes(assembled_prompt.len() as u64));
    group.bench_function("system_prompt/token_estimation_bpe", |b| {
        b.iter(|| {
            let tokens = estimate_text_tokens(black_box(&assembled_prompt));
            black_box(tokens);
        });
    });

    group.finish();
}

// ============================================================================
// 2. Markdown & ANSI Streaming Rendering Throughput Benchmarks
// ============================================================================

fn bench_rendering_throughput(c: &mut Criterion) {
    let mut group = c.benchmark_group("rendering_throughput");

    // 2.1 Streaming Markdown Tokens / Throughput
    let short_tokens = vec![
        "### ", "Quick ", "Overview\n",
        "This ", "is ", "**bold** ", "and ", "*italic* ", "text, ", "plus ", "`code()`.\n",
        "- ", "Item ", "1\n",
        "- ", "Item ", "2\n",
    ];
    let short_bytes: usize = short_tokens.iter().map(|t| t.len()).sum();
    group.throughput(Throughput::Bytes(short_bytes as u64));
    group.bench_function("streaming_markdown/push_short_tokens", |b| {
        let mut renderer = MarkdownRenderer::buffered();
        b.iter(|| {
            renderer.reset();
            let mut out = String::with_capacity(short_bytes * 2);
            for &tok in &short_tokens {
                let piece = renderer.push(black_box(tok));
                out.push_str(&piece);
            }
            let fin = renderer.finish();
            out.push_str(&fin);
            black_box(out);
        });
    });

    let medium_tokens: Vec<&str> = MEDIUM_MARKDOWN.split_inclusive(' ').collect();
    let medium_bytes: usize = medium_tokens.iter().map(|t| t.len()).sum();
    group.throughput(Throughput::Bytes(medium_bytes as u64));
    group.bench_function("streaming_markdown/push_medium_tokens", |b| {
        let mut renderer = MarkdownRenderer::buffered();
        b.iter(|| {
            renderer.reset();
            let mut out = String::with_capacity(medium_bytes * 2);
            for &tok in &medium_tokens {
                let piece = renderer.push(black_box(tok));
                out.push_str(&piece);
            }
            let fin = renderer.finish();
            out.push_str(&fin);
            black_box(out);
        });
    });

    let large_markdown = generate_large_markdown();
    let large_lines: Vec<&str> = large_markdown.lines().collect();
    group.throughput(Throughput::Bytes(large_markdown.len() as u64));
    group.bench_function("streaming_markdown/line_by_line_stream", |b| {
        let mut renderer = MarkdownRenderer::buffered();
        b.iter(|| {
            renderer.reset();
            let mut out = String::with_capacity(large_markdown.len() * 2);
            for &line in &large_lines {
                let mut chunk = line.to_string();
                chunk.push('\n');
                let piece = renderer.push(black_box(&chunk));
                out.push_str(&piece);
            }
            let fin = renderer.finish();
            out.push_str(&fin);
            black_box(out);
        });
    });

    let table_lines = vec![
        "| Module | Subsystem | Status | Latency | Overhead |",
        "|:---|:---|:---:|---:|---:|",
        "| config | env_loader | Active | 0.12ms | Low |",
        "| agent | subagent_mesh | Active | 0.45ms | Minimal |",
        "| ui | inline_ratatui | Active | 1.10ms | Zero-copy |",
        "| tools | grep_filter | Active | 2.30ms | Cache-friendly |",
        "| provider | stream_sse | Active | 0.08ms | Pure-Rust |",
    ];
    group.bench_function("streaming_markdown/table_streamer_chunked", |b| {
        b.iter(|| {
            let mut streamer = MarkdownTableStreamer::new();
            for &line in &table_lines {
                streamer.feed_line(black_box(line));
            }
            let flushed = streamer.flush();
            black_box(flushed);
        });
    });

    // 2.2 Full Document Markdown Rendering
    group.throughput(Throughput::Bytes(SHORT_MARKDOWN.len() as u64));
    group.bench_function("markdown_render/short_document", |b| {
        b.iter(|| {
            let rendered = render_markdown(black_box(SHORT_MARKDOWN));
            black_box(rendered);
        });
    });

    group.throughput(Throughput::Bytes(MEDIUM_MARKDOWN.len() as u64));
    group.bench_function("markdown_render/medium_document", |b| {
        b.iter(|| {
            let rendered = render_markdown(black_box(MEDIUM_MARKDOWN));
            black_box(rendered);
        });
    });

    group.throughput(Throughput::Bytes(large_markdown.len() as u64));
    group.bench_function("markdown_render/large_document", |b| {
        b.iter(|| {
            let rendered = render_markdown(black_box(&large_markdown));
            black_box(rendered);
        });
    });

    let inline_sample = "Format: **bold**, *italic*, ***bold-italic***, `code()`, ~~strike~~, and [link](https://github.com).";
    group.bench_function("markdown_render/inline_formatting", |b| {
        b.iter(|| {
            let formatted = render_inline(black_box(inline_sample));
            black_box(formatted);
        });
    });

    let line_header = "## 3. Advanced Optimization Strategy for Concurrency";
    group.bench_function("markdown_render/line_parser", |b| {
        let mut in_code = false;
        let mut lang = String::new();
        b.iter(|| {
            let res = render_line(black_box(line_header), &mut in_code, &mut lang);
            black_box(res);
        });
    });

    // 2.3 Syntax Highlighting Throughput
    let rust_code = "pub async fn handle_request(req: Request<Body>) -> Result<Response<Body>, StatusCode> {";
    group.bench_function("syntax_highlighting/rust", |b| {
        b.iter(|| {
            let highlighted = highlight_code_line(black_box(rust_code), black_box("rust"));
            black_box(highlighted);
        });
    });

    let python_code = "async def fetch_user_data(user_id: int, session: aiohttp.ClientSession) -> dict:";
    group.bench_function("syntax_highlighting/python", |b| {
        b.iter(|| {
            let highlighted = highlight_code_line(black_box(python_code), black_box("python"));
            black_box(highlighted);
        });
    });

    let ts_code = "const response: ApiResponse<User> = await client.query({ id: 'u_123', timeout: 5000 });";
    group.bench_function("syntax_highlighting/typescript", |b| {
        b.iter(|| {
            let highlighted = highlight_code_line(black_box(ts_code), black_box("typescript"));
            black_box(highlighted);
        });
    });

    let shell_code = "cargo test --release --all-features -- --nocapture && ./scripts/verify_pipeline.sh";
    group.bench_function("syntax_highlighting/shell", |b| {
        b.iter(|| {
            let highlighted = highlight_code_line(black_box(shell_code), black_box("sh"));
            black_box(highlighted);
        });
    });

    let json_code = r#"{"id": "agent-01", "role": "coder", "active": true, "turn": 42}"#;
    group.bench_function("syntax_highlighting/json", |b| {
        b.iter(|| {
            let highlighted = highlight_code_line(black_box(json_code), black_box("json"));
            black_box(highlighted);
        });
    });

    let go_code = "func (s *Server) HandleQuery(ctx context.Context, req *QueryRequest) (*QueryResponse, error) {";
    group.bench_function("syntax_highlighting/go", |b| {
        b.iter(|| {
            let highlighted = highlight_code_line(black_box(go_code), black_box("go"));
            black_box(highlighted);
        });
    });

    // 2.4 ANSI String Processing & Terminal Auto-Sizing
    let ansi_sample = "\x1b[1m\x1b[38;2;122;162;247mFusion Agent\x1b[0m: \x1b[32mActive\x1b[0m \x1b[33m[turn 5]\x1b[0m — \x1b[4mEditing src/lib.rs\x1b[0m";
    group.bench_function("ansi_processing/strip_escapes", |b| {
        b.iter(|| {
            let clean = strip_ansi(black_box(ansi_sample));
            black_box(clean);
        });
    });

    group.bench_function("ansi_processing/visible_width_calculation", |b| {
        b.iter(|| {
            let width = visible_width(black_box(ansi_sample));
            black_box(width);
        });
    });

    let long_ansi = format!("{} | {} | {}", ansi_sample, ansi_sample, ansi_sample);
    group.bench_function("ansi_processing/wrap_ansi_columns", |b| {
        b.iter(|| {
            let wrapped = wrap_ansi(black_box(&long_ansi), black_box(40));
            black_box(wrapped);
        });
    });

    group.bench_function("ansi_processing/truncate_ansi_width", |b| {
        b.iter(|| {
            let truncated = truncate_ansi(black_box(ansi_sample), black_box(25));
            black_box(truncated);
        });
    });

    // 2.5 Frame & Widget Rendering Times (Ratatui Backend & ANSI Banners)
    let backend_status = TestBackend::new(80, 2);
    let mut terminal_status = Terminal::new(backend_status).unwrap();
    let status_info = StatusInfo::new("deepseek", "deepseek-chat")
        .with_agent("Coder")
        .with_advisor("ArchitectureAdvisor")
        .with_tokens("3.8k tokens / $0.008")
        .with_status("Compiling subagent mesh...");
    let theme_tokyo = Theme::tokyo_night();

    group.bench_function("frame_rendering/status_bar_widget", |b| {
        b.iter(|| {
            terminal_status
                .draw(|f| {
                    let area = f.area();
                    render_status_bar_themed(f, area, black_box(&status_info), black_box(&theme_tokyo));
                })
                .unwrap();
        });
    });

    let backend_card = TestBackend::new(70, 5);
    let mut terminal_card = Terminal::new(backend_card).unwrap();
    let card_title = "Subagent Mesh Telemetry";
    let card_content = "Dispatched 4 subagents across DAG. All locks acquired without deadlock. Review passed.";

    group.bench_function("frame_rendering/card_widget", |b| {
        b.iter(|| {
            terminal_card
                .draw(|f| {
                    let area = f.area();
                    render_card_themed(
                        f,
                        area,
                        black_box(card_title),
                        black_box(card_content),
                        black_box(&theme_tokyo),
                        None,
                    );
                })
                .unwrap();
        });
    });

    let backend_critique = TestBackend::new(75, 5);
    let mut terminal_critique = Terminal::new(backend_critique).unwrap();
    let critique_theme = Theme::dracula();

    group.bench_function("frame_rendering/critique_card_widget", |b| {
        b.iter(|| {
            terminal_critique
                .draw(|f| {
                    let area = f.area();
                    render_critique_card_themed(
                        f,
                        area,
                        black_box("ArchitectureAdvisor"),
                        black_box(true),
                        black_box("Verified modular DAG design. No circular dependencies detected."),
                        black_box(&critique_theme),
                    );
                })
                .unwrap();
        });
    });

    let budget_alert = ContextAlert::new("claude-3-7-sonnet", 165_000, 200_000);
    group.bench_function("frame_rendering/budget_warning_banner_ansi", |b| {
        b.iter(|| {
            let banner = render_budget_banner_ansi(
                black_box(&budget_alert),
                black_box(BannerBoxStyle::Rounded),
                black_box(72),
            );
            black_box(banner);
        });
    });

    group.bench_function("frame_rendering/budget_status_pill_ansi", |b| {
        b.iter(|| {
            let pill = render_budget_status_pill_ansi(black_box(&budget_alert));
            black_box(pill);
        });
    });

    let markdown_table_src = r#"| Subsystem | Latency (p50) | Latency (p99) | Status |
|:---|:---:|:---:|:---:|
| Cold Startup | 0.45ms | 0.82ms | PASS |
| Markdown Parser | 0.18ms | 0.35ms | PASS |
| Grep Filter | 1.12ms | 2.45ms | PASS |
| Subagent Mesh | 0.22ms | 0.51ms | PASS |"#;

    group.bench_function("frame_rendering/markdown_table_formatter", |b| {
        b.iter(|| {
            let formatted = render_markdown_table(black_box(markdown_table_src), black_box(80));
            black_box(formatted);
        });
    });

    // 2.6 SSE Stream Parser Throughput

    let anthropic_text_delta = json!({
        "type": "content_block_delta",
        "index": 0,
        "delta": {
            "type": "text_delta",
            "text": " Optimized solution using zero-cost iterator combinators."
        }
    });

    group.bench_function("sse_parser/anthropic_text_delta", |b| {
        let mut accumulator = AnthropicStreamAccumulator::new();
        b.iter(|| {
            let chunks = accumulator.process_event(black_box(&anthropic_text_delta));
            black_box(chunks);
        });
    });

    group.finish();
}

// ============================================================================
// 3. Tool Execution Latency Benchmarks
// ============================================================================

fn bench_tool_execution(c: &mut Criterion) {
    let mut group = c.benchmark_group("tool_execution");
    let rt = tokio::runtime::Runtime::new().unwrap();

    let fixture = BenchFixture::new();
    fixture.populate_search_tree();
    let ctx = fixture.context();

    // 3.1 Grep Filter & Content Search
    let grep_tool = GrepTool::new();

    group.bench_function("grep/literal_keyword", |b| {
        let args = json!({
            "pattern": "TARGET_KEYWORD",
            "case_sensitive": true
        });
        b.to_async(&rt).iter(|| async {
            let res = grep_tool.execute(black_box(args.clone()), &ctx).await.unwrap();
            black_box(res);
        });
    });

    group.bench_function("grep/regex_pattern", |b| {
        let args = json!({
            "pattern": r"pub\s+(async\s+)?fn\s+[a-z_]+",
            "case_sensitive": true
        });
        b.to_async(&rt).iter(|| async {
            let res = grep_tool.execute(black_box(args.clone()), &ctx).await.unwrap();
            black_box(res);
        });
    });

    group.bench_function("grep/case_insensitive", |b| {
        let args = json!({
            "pattern": "target_keyword",
            "case_sensitive": false
        });
        b.to_async(&rt).iter(|| async {
            let res = grep_tool.execute(black_box(args.clone()), &ctx).await.unwrap();
            black_box(res);
        });
    });

    let grep_opts_rust = GrepOptions {
        pattern: "TARGET_KEYWORD".to_string(),
        search_path: fixture.path().to_path_buf(),
        cwd: fixture.path().to_path_buf(),
        file_types: vec!["rust".to_string()],
        ..Default::default()
    };

    group.bench_function("grep_filter/engine_file_type_filtered", |b| {
        let engine = FilterableGrepEngine::new(grep_opts_rust.clone()).unwrap();
        b.iter(|| {
            let search_result = engine.search().unwrap();
            black_box(search_result);
        });
    });

    let grep_opts_globs = GrepOptions {
        pattern: "TARGET_KEYWORD".to_string(),
        search_path: fixture.path().to_path_buf(),
        cwd: fixture.path().to_path_buf(),
        include_globs: vec!["src/**/*.rs".to_string()],
        exclude_globs: vec!["tests/**".to_string()],
        ..Default::default()
    };

    group.bench_function("grep_filter/engine_glob_includes_excludes", |b| {
        let engine = FilterableGrepEngine::new(grep_opts_globs.clone()).unwrap();
        b.iter(|| {
            let search_result = engine.search().unwrap();
            black_box(search_result);
        });
    });

    group.bench_function("grep_filter/path_filter_builder_compile", |b| {
        b.iter(|| {
            let filter = PathFilterBuilder::default()
                .includes(vec!["src/**/*.rs".to_string(), "docs/*.md".to_string()])
                .excludes(vec!["target/**".to_string(), "*.tmp".to_string()])
                .file_types(vec!["rust".to_string(), "markdown".to_string()])
                .build()
                .unwrap();
            black_box(filter);
        });
    });

    group.bench_function("grep_filter/file_type_registry_lookup", |b| {
        let registry = FileTypeRegistry::default();
        b.iter(|| {
            let r1 = registry.resolve_extensions(black_box("rust"));
            let r2 = registry.resolve_extensions(black_box("typescript"));
            let r3 = registry.resolve_extensions(black_box("python"));
            let r4 = registry.resolve_extensions(black_box("go"));
            let r5 = registry.resolve_extensions(black_box("docker"));
            black_box((r1, r2, r3, r4, r5));
        });
    });

    // 3.2 File Globbing
    let glob_tool = GlobTool::new();

    group.bench_function("glob/shallow_pattern", |b| {
        let args = json!({ "pattern": "*.toml" });
        b.to_async(&rt).iter(|| async {
            let res = glob_tool.execute(black_box(args.clone()), &ctx).await.unwrap();
            black_box(res);
        });
    });

    group.bench_function("glob/recursive_all_rs", |b| {
        let args = json!({ "pattern": "**/*.rs" });
        b.to_async(&rt).iter(|| async {
            let res = glob_tool.execute(black_box(args.clone()), &ctx).await.unwrap();
            black_box(res);
        });
    });

    group.bench_function("glob/scoped_directory_pattern", |b| {
        let args = json!({
            "pattern": "*.rs",
            "path": "src"
        });
        b.to_async(&rt).iter(|| async {
            let res = glob_tool.execute(black_box(args.clone()), &ctx).await.unwrap();
            black_box(res);
        });
    });

    // 3.3 Search Indexing & Querying
    let s1 = generate_test_session("Rust Concurrency & Mutex Optimization", "gpt-4o", 5);
    let s2 = generate_test_session("TypeScript React State Management", "claude-3-5-sonnet", 5);
    let s3 = generate_test_session("Python FastAPI Asynchronous Endpoints", "deepseek-chat", 5);
    let s4 = generate_test_session("Go Goroutine Channels and Context", "gpt-4o", 5);
    let s5 = generate_test_session("Android Termux Rust Build & Toolchain", "claude-3-5-haiku", 5);
    let sessions = vec![s1.clone(), s2.clone(), s3.clone(), s4.clone(), s5.clone()];

    group.bench_function("search_indexing/build_session_index", |b| {
        b.iter(|| {
            let index = SessionSearchIndex::build_from_sessions(black_box(&sessions));
            black_box(index);
        });
    });

    let bm25_query = SearchQuery::parse("Rust Mutex deadlock contention");
    group.bench_function("search_indexing/bm25_text_query", |b| {
        b.iter(|| {
            let report = search_in_sessions(black_box(&sessions), black_box(&bm25_query));
            black_box(report);
        });
    });

    let semantic_query = SearchQuery::parse("mode:semantic concurrency synchronization locks");
    group.bench_function("search_indexing/semantic_vector_query", |b| {
        b.iter(|| {
            let report = search_in_sessions(black_box(&sessions), black_box(&semantic_query));
            black_box(report);
        });
    });

    let hybrid_query = SearchQuery::parse("mode:hybrid Tokio async channel performance");
    group.bench_function("search_indexing/hybrid_search_query", |b| {
        b.iter(|| {
            let report = search_in_sessions(black_box(&sessions), black_box(&hybrid_query));
            black_box(report);
        });
    });

    let index = SessionSearchIndex::build_from_sessions(&sessions);
    group.bench_function("search_indexing/session_similarity_discovery", |b| {
        b.iter(|| {
            let similar = index.find_similar_sessions(black_box(s1.id), black_box(3));
            black_box(similar);
        });
    });

    let sample_rs_file = generate_code_content(300);
    let symbol_scanner = SymbolScanner::new();
    group.bench_function("search_indexing/workspace_symbol_scanning", |b| {
        b.iter(|| {
            let symbols = symbol_scanner.scan_content(black_box(&sample_rs_file), black_box("src/lib.rs"));
            black_box(symbols);
        });
    });

    // 3.4 File Reading & Surgical Editing
    let read_tool = ReadFileTool::new();
    let medium_code = generate_code_content(500);
    fixture.create_file("medium_code.rs", &medium_code);

    group.bench_function("file_tools/read_numbered_medium", |b| {
        let args = json!({ "path": "medium_code.rs" });
        b.to_async(&rt).iter(|| async {
            let res = read_tool.execute(black_box(args.clone()), &ctx).await.unwrap();
            black_box(res);
        });
    });

    group.bench_function("file_tools/read_windowed_slice", |b| {
        let args = json!({ "path": "medium_code.rs", "offset": 100, "limit": 50 });
        b.to_async(&rt).iter(|| async {
            let res = read_tool.execute(black_box(args.clone()), &ctx).await.unwrap();
            black_box(res);
        });
    });

    let old_text = generate_code_content(200);
    let target_needle = "pub fn function_marker_50() -> usize { 50 }";
    let replacement = "pub fn function_marker_50() -> usize {\n    // Replaced during bench\n    100\n}";
    let new_text = old_text.replace(target_needle, replacement);

    group.bench_function("diff_tools/apply_exact_edit_algorithm", |b| {
        b.iter(|| {
            let res = apply_exact_edit(
                black_box(&old_text),
                black_box(target_needle),
                black_box(replacement),
                black_box("test_file.rs"),
            )
            .unwrap();
            black_box(res);
        });
    });

    group.bench_function("diff_tools/compute_diff_stats", |b| {
        b.iter(|| {
            let stats = compute_diff_stats(black_box(&old_text), black_box(&new_text));
            black_box(stats);
        });
    });

    group.bench_function("diff_tools/generate_unified_diff", |b| {
        b.iter(|| {
            let diff = generate_unified_diff(
                black_box(&old_text),
                black_box(&new_text),
                black_box("src/sample.rs"),
                black_box(3),
            );
            black_box(diff);
        });
    });

    group.bench_function("diff_tools/generate_colorized_diff", |b| {
        b.iter(|| {
            let diff = generate_colorized_diff(
                black_box(&old_text),
                black_box(&new_text),
                black_box("src/sample.rs"),
                black_box(3),
            );
            black_box(diff);
        });
    });

    let bash_tool = BashTool::new();
    group.bench_function("bash_tool/simple_echo", |b| {
        let args = json!({ "command": "echo 'fusion_bench_echo'" });
        b.to_async(&rt).iter(|| async {
            let res = bash_tool.execute(black_box(args.clone()), &ctx).await.unwrap();
            black_box(res);
        });
    });

    group.finish();
}

// ============================================================================
// 4. Subagent Message Dispatch & Pub-Sub Routing Benchmarks
// ============================================================================

fn bench_subagent_mesh(c: &mut Criterion) {
    let mut group = c.benchmark_group("subagent_mesh");
    let rt = tokio::runtime::Runtime::new().unwrap();

    // 4.1 Subagent Peer Lifecycle Benchmarks
    group.bench_function("peer_lifecycle/mesh_creation", |b| {
        b.iter(|| {
            let mesh = AgentMesh::with_capacity(black_box(1024));
            black_box(mesh);
        });
    });

    group.bench_function("peer_lifecycle/register_peers_batch", |b| {
        b.to_async(&rt).iter(|| async {
            let mesh = AgentMesh::new();
            let c1 = mesh.register("scout-01", AgentRole::Scout, "Exploration agent").await.unwrap();
            let c2 = mesh.register("coder-01", AgentRole::Coder, "Implementation agent").await.unwrap();
            let c3 = mesh.register("tester-01", AgentRole::Tester, "Test runner").await.unwrap();
            let c4 = mesh.register("reviewer-01", AgentRole::Reviewer, "Quality reviewer").await.unwrap();
            black_box((c1, c2, c3, c4));
        });
    });

    group.bench_function("peer_lifecycle/deregister_peer", |b| {
        b.to_async(&rt).iter(|| async {
            let mesh = AgentMesh::new();
            let _c = mesh.register("temp-agent", AgentRole::General, "Temporary agent").await.unwrap();
            mesh.unregister("temp-agent").await.unwrap();
        });
    });

    let active_mesh = rt.block_on(async {
        let mesh = AgentMesh::new();
        let _c1 = mesh.register("scout-01", AgentRole::Scout, "Scout").await.unwrap();
        let _c2 = mesh.register("coder-01", AgentRole::Coder, "Coder").await.unwrap();
        let _c3 = mesh.register("tester-01", AgentRole::Tester, "Tester").await.unwrap();
        let _c4 = mesh.register("reviewer-01", AgentRole::Reviewer, "Reviewer").await.unwrap();
        mesh
    });

    group.bench_function("peer_lifecycle/list_peers_snapshot", |b| {
        b.to_async(&rt).iter(|| async {
            let peers = active_mesh.list_peers().await;
            black_box(peers);
        });
    });

    // 4.2 Pub-Sub Broadcast Routing Overhead
    let (broadcast_mesh, mut sender_channel, mut subscriber_5_channels) = rt.block_on(async {
        let mesh = AgentMesh::with_capacity(2048);
        let sender = mesh.register("sender-agent", AgentRole::Orchestrator, "Sender").await.unwrap();
        let mut subs = Vec::new();
        for i in 1..=5 {
            let sub = mesh
                .register(format!("sub-5-{i}"), AgentRole::General, "Subscriber")
                .await
                .unwrap();
            subs.push(sub);
        }
        (mesh, sender, subs)
    });

    group.bench_function("pubsub_broadcast/status_update", |b| {
        let ch = &sender_channel;
        b.to_async(&rt).iter(|| async {
            ch.broadcast_status(black_box(AgentStatus::Active {
                task: "Running parallel code refactoring".to_string(),
            }))
            .await
            .unwrap();
        });
    });

    group.bench_function("pubsub_broadcast/discovery_finding", |b| {
        let ch = &sender_channel;
        b.to_async(&rt).iter(|| async {
            ch.broadcast_discovery(
                black_box("performance"),
                black_box("Found unindexed query pattern in sessions storage"),
                black_box(vec!["src/agent/search.rs".to_string()]),
            )
            .await
            .unwrap();
        });
    });

    group.bench_function("pubsub_broadcast/fanout_5_subscribers", |b| {
        let ch = &sender_channel;
        let subs = &mut subscriber_5_channels;
        b.to_async(&rt).iter(|| async {
            let msg = BroadcastMessage::new(
                "sender-agent",
                topics::STATUS,
                BroadcastPayload::Status {
                    status: AgentStatus::Progress {
                        step: 3,
                        total: Some(10),
                        message: "Processing AST batch".to_string(),
                    },
                },
            );
            broadcast_mesh.broadcast(black_box(msg)).await.unwrap();

            for sub in subs.iter_mut() {
                let received = sub.recv_broadcast().await.unwrap();
                black_box(received);
            }
        });
    });

    let (mesh_20, mut subs_20) = rt.block_on(async {
        let mesh = AgentMesh::with_capacity(4096);
        let mut subs = Vec::new();
        for i in 1..=20 {
            let sub = mesh
                .register(format!("sub-20-{i}"), AgentRole::General, "Sub")
                .await
                .unwrap();
            subs.push(sub);
        }
        (mesh, subs)
    });

    group.bench_function("pubsub_broadcast/fanout_20_subscribers", |b| {
        let subs = &mut subs_20;
        b.to_async(&rt).iter(|| async {
            let msg = BroadcastMessage::new(
                "orchestrator",
                topics::COORDINATION,
                BroadcastPayload::Custom {
                    kind: "barrier_pulse".to_string(),
                    data: json!({ "phase": "validation", "timestamp": 1710000000 }),
                },
            );
            mesh_20.broadcast(black_box(msg)).await.unwrap();

            for sub in subs.iter_mut() {
                let received = sub.recv_broadcast().await.unwrap();
                black_box(received);
            }
        });
    });

    // 4.3 Direct Peer-to-Peer Message Dispatch
    let (direct_mesh, mut direct_sender, mut direct_receiver) = rt.block_on(async {
        let mesh = AgentMesh::new();
        let sender = mesh.register("direct-alice", AgentRole::Coder, "Alice").await.unwrap();
        let receiver = mesh.register("direct-bob", AgentRole::Tester, "Bob").await.unwrap();
        (mesh, sender, receiver)
    });

    group.bench_function("direct_messaging/point_to_point_send", |b| {
        let sender = &direct_sender;
        b.to_async(&rt).iter(|| async {
            sender
                .send_direct(
                    black_box("direct-bob"),
                    black_box("test_dispatch"),
                    black_box("Run integration tests for module X"),
                )
                .await
                .unwrap();
        });
    });

    group.bench_function("direct_messaging/send_and_recv_roundtrip", |b| {
        let sender = &direct_sender;
        let receiver = &mut direct_receiver;
        b.to_async(&rt).iter(|| async {
            sender
                .send_direct(
                    "direct-bob",
                    "ping",
                    "ping payload content",
                )
                .await
                .unwrap();

            let msg = receiver.recv_direct().await.unwrap();
            black_box(msg);
        });
    });

    // 4.4 Request-Response Peer RPC Queries
    let (rpc_mesh, mut rpc_requester, mut rpc_responder) = rt.block_on(async {
        let mesh = AgentMesh::new();
        let req = mesh.register("requester-agent", AgentRole::Coder, "Requester").await.unwrap();
        let resp = mesh.register("responder-agent", AgentRole::Reviewer, "Responder").await.unwrap();
        (mesh, req, resp)
    });

    group.bench_function("peer_rpc/ask_and_reply_roundtrip", |b| {
        let requester = &rpc_requester;
        let responder = &mut rpc_responder;
        b.to_async(&rt).iter(|| async {
            // Spawn async responder loop
            let responder_future = async {
                if let Some(envelope) = responder.recv_query().await {
                    let response = PeerResponse::success(
                        envelope.query.query_id.clone(),
                        "responder-agent",
                        "Review approved: lock safety verified.",
                    );
                    let _ = envelope.reply.send(response);
                }
            };

            let ask_future = requester.ask(
                "responder-agent",
                "Is lock ordering invariant maintained?",
                Some("src/agent/mesh.rs".to_string()),
                Some(Duration::from_secs(5)),
            );

            let (_, res) = tokio::join!(responder_future, ask_future);
            let response = res.unwrap();
            black_box(response);
        });
    });

    // 4.5 Resource Claiming, Locking, & Blackboard Coordination
    let (coord_mesh, coord_agent) = rt.block_on(async {
        let mesh = AgentMesh::new();
        let agent = mesh.register("lock-agent", AgentRole::Coder, "Lock Agent").await.unwrap();
        (mesh, agent)
    });

    group.bench_function("coordination/resource_claim_and_release", |b| {
        let agent = &coord_agent;
        b.to_async(&rt).iter(|| async {
            agent
                .claim_resource(black_box("src/agent/mesh.rs"), black_box(None))
                .await
                .unwrap();

            let released = agent.release_resource(black_box("src/agent/mesh.rs")).await.unwrap();
            black_box(released);
        });
    });

    group.bench_function("coordination/shared_fact_blackboard", |b| {
        let agent = &coord_agent;
        b.to_async(&rt).iter(|| async {
            let version = agent
                .set_fact(
                    black_box("architecture_status"),
                    black_box(json!({ "status": "approved", "timestamp": 1710000000 })),
                )
                .await
                .unwrap();

            let fact = agent.get_fact(black_box("architecture_status")).await;
            black_box((version, fact));
        });
    });

    group.finish();
}

// ============================================================================
// Criterion Benchmark Groups & Main Entrypoint
// ============================================================================

criterion_group!(
    cold_startup,
    bench_cold_startup,
);

criterion_group!(
    rendering_throughput,
    bench_rendering_throughput,
);

criterion_group!(
    tool_execution,
    bench_tool_execution,
);

criterion_group!(
    subagent_mesh,
    bench_subagent_mesh,
);

criterion_main!(
    cold_startup,
    rendering_throughput,
    tool_execution,
    subagent_mesh,
);
