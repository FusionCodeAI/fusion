# Architecture Overview

Fusion is crafted with clean separation of concerns and zero unnecessary abstraction layers:

```mermaid
graph TD
    subgraph UI_Layer["Interface Layer"]
        CLI["CLI Command Parser (Clap)"]
        REPL["Inline REPL (Ratatui + Crossterm)"]
        ACP["ACP Server (JSON-RPC 2.0 Stdio)"]
        WASM["WebAssembly Browser Engine"]
    end

    subgraph Core_Engine["Core Engine"]
        Runner["AgentRunner Loop"]
        Session["Session State & History Manager"]
        Config["Configuration & Key Resolver"]
        Recovery["Recovery & Correction Engine"]
        Compaction["Context Compaction & Pruner"]
        Cost["Token / Cost Tracking & Budgets"]
    end

    subgraph Intelligence["Intelligence & Guardrails"]
        Mesh["Multi-Agent Mesh (Scout, Coder, Tester, Reviewer)"]
        Advisors["Advisory Committee (Arch, Security, Quality)"]
        Consensus["Consensus Engine (Votes & Vetoes)"]
        Throttle["Rate Throttling & Offline Transition"]
    end

    subgraph Platform_Tools["Tool Registry"]
        Files["File Ops (Read, Write, Edit)"]
        Search["Search Engine (Grep, Glob)"]
        Shell["Execution (Bash, Signals)"]
        Git["Git, SQLite, Fetch, MCP"]
    end

    subgraph Observability["Observability"]
        Trace["OTLP Tracing & Redaction"]
        Notify["Notifications, Voice, Sound"]
        Bench["Benchmark Harness"]
    end

    subgraph Network["Provider & Transport"]
        Client["LlmClient (Pure Rustls Streaming)"]
        Retry["Retry Policies"]
        Providers["DeepSeek • Anthropic • OpenAI • xAI • Ollama • OpenRouter"]
    end

    CLI --> Runner
    REPL --> Runner
    ACP --> Runner
    WASM --> Runner

    Runner --> Session
    Runner --> Config
    Runner --> Recovery
    Runner --> Compaction
    Runner --> Cost

    Runner --> Mesh
    Runner --> Advisors
    Mesh --> Consensus
    Runner --> Throttle

    Runner --> Platform_Tools
    Runner --> Client

    Client --> Retry
    Retry --> Providers

    Runner --> Observability
```

## Pure Rust Dependency Manifesto

- **No C/C++ build dependencies**: Avoids `gcc`, `clang`, `cmake`, and `make` during installation.
- **No OpenSSL**: Eliminates shared object version mismatches across Linux distributions and Android NDK.
- **No Protocol Buffers compiler**: Zero requirement for external `protoc` binaries.

## Source Layout

```text
src/
  acp/          ACP JSON-RPC 2.0 server, event streaming, WebSocket bridge
  agent/        AgentRunner loop, mesh, subagents, advisors, consensus,
                recovery, compaction, pruner, undo, bookmarks, cost,
                tracing (OTLP), throttle, skills, snippets, fork/rewind
  cli/          CLI parsing, shell completion generation
  config/       Config loading, env resolution, migration, presets, workspaces
  provider/     LlmClient, provider adapters, model catalog, retry, offline mode
  tools/        Sandboxed tool registry (files, search, git, sqlite, MCP, ...)
  ui/           Inline REPL, markdown renderer, slash commands, themes, voice
  wasm.rs       WASM bindings, VirtualFs, browser engine
sdk/            TypeScript SDK (@fusion/sdk) — types, xterm adapter, WASM core
web/            Browser playground (xterm.js + WebSocket ACP)
Formula/        Homebrew formula
benches/        Criterion micro-benchmarks & provider comparison harness
scripts/        install.sh, termux-bootstrap.sh, package.sh, verify.sh
tests/          CLI, multi-agent, smoke, and WASM integration tests
```
