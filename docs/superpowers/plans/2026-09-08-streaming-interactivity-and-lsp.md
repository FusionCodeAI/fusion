# Streaming Interactivity, Subagent Scaling & Native LSP Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Implement real-time muted italic thinking stream in terminal scrollback, live subagent progress event streaming, 16-worker concurrency scaling, and a pure-Rust native LSP client tool ported from Oh My Pi's blueprints.

**Architecture:** 
1. `src/ui/repl.rs` handles `AgentEvent::ThinkingDelta` by outputting `\x1b[2;3m` directly to stdout and committing completed lines to native scrollback, separating thoughts from action with a subtle rule.
2. `SubagentManager` bridges live `SubagentProgress` events into the turn's `event_tx`, streaming indented execution tree branches in the REPL.
3. A new `src/tools/lsp/` module implements a pure-Rust stdio JSON-RPC client over `tokio::process`, embedding Oh My Pi's `defaults.json` server definitions and exposing semantic code intelligence (`definition`, `references`, `rename`, `diagnostics`).

**Tech Stack:** Rust (Tokio, reqwest, serde, serde_json, crossterm).

---

### Task 1: Live Thinking Stream in Native Scrollback (`src/ui/repl.rs`)

**Files:**
- Modify: `src/ui/repl.rs:730-810`
- Test: `tests/repl_test.rs` (or `tests/tui_fx_style_test.rs`)

- [ ] **Step 1: Write a test verifying thinking ANSI escape sequence formatting**

Add to `tests/tui_fx_style_test.rs`:
```rust
#[test]
fn test_thinking_ansi_formatting() {
    let raw_chunk = "Analyzing function signature in src/lib.rs";
    let formatted = format!("\x1b[2;3m{}\x1b[0m", raw_chunk);
    assert!(formatted.starts_with("\x1b[2;3m"));
    assert!(formatted.ends_with("\x1b[0m"));
}
```

- [ ] **Step 2: Run test to verify it passes**

Run: `cargo test --test tui_fx_style_test test_thinking_ansi_formatting`
Expected: PASS

- [ ] **Step 3: Update `handle_agent_event` in `src/ui/repl.rs` to stream thinking deltas**

Modify `src/ui/repl.rs` in `AgentEvent::ThinkingDelta(th)`:
- If not currently in thinking mode, print a dimmed thought prompt `\x1b[2;3m💭 ` and set `is_thinking = true`.
- Print `th` with `\x1b[2;3m` and flush stdout directly to preserve native scrollback.
- When transitioning from thinking to `AgentEvent::TextDelta` or `AgentEvent::ToolStarted`:
  - Reset ANSI styles (`\x1b[0m`).
  - Print a subtle separator `\r\n\x1b[2;37m───\x1b[0m\r\n\r\n` and flush stdout.
  - Reset `is_thinking = false`.

- [ ] **Step 4: Run compilation check**

Run: `cargo check`
Expected: PASS with no errors.

- [ ] **Step 5: Commit**

```bash
git add src/ui/repl.rs tests/tui_fx_style_test.rs
git commit -m "feat(ui): stream reasoning thoughts live in muted italic scrollback"
```

---

### Task 2: Subagent Progress Event Forwarding & REPL Tree Rendering

**Files:**
- Modify: `src/agent/loop_runner.rs:18-65`
- Modify: `src/agent/subagent.rs:440-520`
- Modify: `src/ui/repl.rs:730-870`
- Test: `tests/subagent_test.rs`

- [ ] **Step 1: Add `SubagentProgressEvent` to `AgentEvent` in `src/agent/loop_runner.rs`**

```rust
pub enum AgentEvent {
    // Existing variants...
    SubagentProgressEvent {
        id: String,
        name: String,
        role: SubagentRole,
        progress: crate::agent::subagent::SubagentProgress,
    },
}
```

- [ ] **Step 2: Forward `SubagentProgress` in `src/agent/subagent.rs`**

In `SubagentManager::spawn_with_channel`, subscribe a forwarder task that sends progress events to the active `event_tx`:
```rust
let progress_event = AgentEvent::SubagentProgressEvent {
    id: id.clone(),
    name: name.clone(),
    role: role.clone(),
    progress: event.clone(),
};
let _ = turn_event_tx.send(progress_event);
```

- [ ] **Step 3: Render subagent progress tree branches in `src/ui/repl.rs`**

Handle `AgentEvent::SubagentProgressEvent` in `handle_agent_event`:
- `Started`: Print `\r\n\x1b[1;36m┌─ 🤖 {role} ({name})\x1b[0m: \x1b[2;37m{task}\x1b[0m\r\n`
- `ToolStarted`: Print `\x1b[1;36m│\x1b[0m  \x1b[33m⠋ {tool}\x1b[0m \x1b[2;37m{args_preview}\x1b[0m\r\n`
- `ToolCompleted`: Print `\x1b[1;36m│\x1b[0m  \x1b[32m✓ {tool}\x1b[0m\r\n`
- `Completed`: Print `\x1b[1;36m└─\x1b[0m \x1b[32m✓ {name} completed in {turns} turns\x1b[0m\r\n\r\n`

- [ ] **Step 4: Verify with subagent test**

Run: `cargo test --test strace_test`
Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add src/agent/loop_runner.rs src/agent/subagent.rs src/ui/repl.rs
git commit -m "feat(subagent): stream subagent execution tree events live in REPL"
```

---

### Task 3: Subagent Concurrency Semaphore Scaling

**Files:**
- Modify: `src/agent/subagent.rs:440-480`
- Modify: `src/config.rs:180-220`
- Test: `tests/subagent_test.rs`

- [ ] **Step 1: Increase default concurrency from 8 to 16**

In `src/agent/subagent.rs`:
```rust
pub const DEFAULT_MAX_CONCURRENT_SUBAGENTS: usize = 16;
```
Allow configuration via `config.max_concurrent_subagents.unwrap_or(DEFAULT_MAX_CONCURRENT_SUBAGENTS)`.

- [ ] **Step 2: Run tests to verify concurrency semaphore initialization**

Run: `cargo test --lib agent::subagent`
Expected: PASS

- [ ] **Step 3: Commit**

```bash
git add src/agent/subagent.rs src/config.rs
git commit -m "feat(subagent): scale default subagent concurrency semaphore to 16"
```

---

### Task 4: Port LSP `defaults.json` Configurations

**Files:**
- Create: `src/tools/lsp/defaults.json`
- Create: `src/tools/lsp/config.rs`
- Test: `tests/lsp_config_test.rs`

- [ ] **Step 1: Copy `defaults.json` from `reference/oh-my-pi`**

Copy `reference/oh-my-pi/packages/coding-agent/src/lsp/defaults.json` into `src/tools/lsp/defaults.json`.

- [ ] **Step 2: Implement `ServerConfig` and loader in `src/tools/lsp/config.rs`**

Parse the embedded JSON:
```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LspServerDef {
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub file_types: Vec<String>,
    #[serde(default)]
    pub root_markers: Vec<String>,
    #[serde(default)]
    pub settings: serde_json::Value,
}

pub fn get_default_servers() -> HashMap<String, LspServerDef> {
    let data = include_str!("defaults.json");
    serde_json::from_str(data).unwrap_or_default()
}
```

- [ ] **Step 3: Write test verifying `rust-analyzer` and `gopls` definitions parse cleanly**

Create `tests/lsp_config_test.rs`:
```rust
#[test]
fn test_default_lsp_servers_parse() {
    let servers = fusion::tools::lsp::config::get_default_servers();
    assert!(servers.contains_key("rust-analyzer"));
    assert!(servers.contains_key("gopls"));
    let ra = &servers["rust-analyzer"];
    assert_eq!(ra.command, "rust-analyzer");
    assert!(ra.file_types.contains(&".rs".to_string()));
}
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test --test lsp_config_test`
Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add src/tools/lsp/ tests/lsp_config_test.rs
git commit -m "feat(lsp): embed standard language server defaults from OMP"
```

---

### Task 5: Pure-Rust Tokio LSP JSON-RPC Client

**Files:**
- Create: `src/tools/lsp/client.rs`
- Modify: `src/tools/lsp/mod.rs`
- Test: `tests/lsp_client_test.rs`

- [ ] **Step 1: Write header framing and JSON-RPC message structs**

Implement `encode_message` and `decode_message`:
- Format: `Content-Length: <len>\r\n\r\n<json_payload>`
- Support `id`, `method`, `params`, `result`, `error`.

- [ ] **Step 2: Implement `LspClient` lifecycle over `tokio::process`**

- `LspClient::spawn(command, args, cwd)`: Spawns subprocess with piped stdin/stdout.
- `initialize(root_path)`: Sends `initialize` request and `initialized` notification.
- `did_open(path, language_id, text)`: Sends `textDocument/didOpen`.
- `definition(path, line, col)`: Sends `textDocument/definition`.
- `references(path, line, col)`: Sends `textDocument/references`.
- `shutdown()`: Sends `shutdown` followed by `exit`.

- [ ] **Step 3: Write test for message framing and JSON-RPC serialization**

Add unit tests in `tests/lsp_client_test.rs` testing frame encoding/decoding.

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test --test lsp_client_test`
Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add src/tools/lsp/client.rs src/tools/lsp/mod.rs tests/lsp_client_test.rs
git commit -m "feat(lsp): implement async tokio stdio JSON-RPC LSP client"
```

---

### Task 6: Implement `LspTool` in Tool Registry

**Files:**
- Create: `src/tools/lsp/tool.rs`
- Modify: `src/tools/mod.rs:98-135`
- Test: `tests/lsp_tool_test.rs`

- [ ] **Step 1: Implement `Tool` trait for `LspTool`**

Register operations:
- `definition`: Jumps to symbol definition using LSP, with fallback to regex `symbols`.
- `references`: Finds all cross-file usages.
- `type_definition`: Locates type declaration.
- `diagnostics`: Reads live compiler/linter diagnostics.

- [ ] **Step 2: Register `LspTool` in `default_registry()` in `src/tools/mod.rs`**

```rust
registry.register(Arc::new(crate::tools::lsp::LspTool::new()));
```

- [ ] **Step 3: Write test verifying tool registration and schema validation**

Create `tests/lsp_tool_test.rs`:
```rust
#[test]
fn test_lsp_tool_registration() {
    let reg = fusion::tools::default_registry();
    assert!(reg.contains("lsp"));
}
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test --test lsp_tool_test`
Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add src/tools/lsp/tool.rs src/tools/mod.rs tests/lsp_tool_test.rs
git commit -m "feat(tools): register native lsp tool in tool registry"
```

---

### Task 7: Full System Verification & Binary Build

**Files:**
- All touched files
- Workspace binary target

- [ ] **Step 1: Run all tests in the workspace**

Run: `cargo test --lib && cargo test --test tui_fx_style_test && cargo test --test strace_test && cargo test --test lsp_config_test && cargo test --test lsp_client_test && cargo test --test lsp_tool_test`
Expected: All tests pass.

- [ ] **Step 2: Run `cargo fmt --all --check` and `cargo check`**

Run: `cargo fmt --all --check && cargo check`
Expected: Clean with zero formatting issues.

- [ ] **Step 3: Build release binary and update symlink**

Run: `cargo build --release --bin fusion && ln -sf /Users/aungmyatmoe/TheSpace/fusion/target/release/fusion /Users/aungmyatmoe/.local/bin/fusion`
Expected: Binary compiles cleanly and updates symlink.

- [ ] **Step 4: Verify with smoke test**

Run: `fusion --version`
Expected: Outputs valid version.
