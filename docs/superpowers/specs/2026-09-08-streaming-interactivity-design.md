# Specification: Streaming Interactivity & Subagent Scaling

- **Status:** Approved / In Design
- **Date:** 2026-09-08
- **Authors:** Fusion Engineering Team
- **Target Version:** `v2.0.0-alpha.4`

---

## 1. Objective

Enhance the live interactivity and responsiveness of the Fusion CLI during long-horizon tasks and complex multi-agent workflows without sacrificing token economics or introducing artificial tool round-trips.

Specifically:
1. **Live Thinking Streaming in Scrollback:** Stream reasoning/thought chunks (`AgentEvent::ThinkingDelta`) live to the terminal in muted, italic ANSI text (`\x1b[2;3m`), committing finalized thought lines into the native terminal scrollback.
2. **Subagent Event Streaming:** Forward subagent progress (`SubagentProgress`) from worker tasks directly into the primary REPL event loop, rendering real-time indented execution trees in scrollback.
3. **Subagent Concurrency Scaling:** Increase the default subagent concurrency semaphore from 8 to 16 (configurable up to 32) for high-fanout parallel tasks (`Scout`, `Coder`, `Tester`).

---

## 2. Architecture & Data Flow

### 2.1 Thinking Stream Pipeline

```
LLM Provider Stream
       │
       ▼
AgentEvent::ThinkingDelta(chunk)
       │
       ▼
REPL Event Handler (src/ui/repl.rs)
  ├── 1. Clear prompt frame
  ├── 2. Format with Dim + Italic: "\x1b[2;3m" + chunk + "\x1b[0m"
  ├── 3. Write and flush directly to stdout
  └── 4. On transition to TextDelta / ToolStarted:
         • Reset ANSI styling
         • Emit subtle thought-separator rule: "\x1b[2;37m───\x1b[0m\r\n"
         • Resume normal markdown/tool rendering
```

### 2.2 Subagent Event Forwarding

```
Coordinator Agent
       │
       ├──> spawn_subagents_batch([Task 1, Task 2, ...])
       │           │
       │           ▼
       │     SubagentManager
       │           │ (mpsc::channel forwarder)
       │           ▼
       │     AgentEvent::SubagentProgressEvent { id, name, role, progress }
       │           │
       ▼           ▼
REPL Event Handler (src/ui/repl.rs)
  ├── ┌─ 🤖 [Role: Name] Task description
  ├── │  ⠋ Tool started: <tool> <args>
  ├── │  ✓ Tool finished: <output summary>
  └── └─ ✓ Completed in <N> turns (<duration>, <tokens>)
```

---

## 3. Detailed Component Specifications

### 3.1 Thinking Stream Formatter (`src/ui/repl.rs`)

1. **Active State Tracking:**
   - Add `is_streaming_thinking: bool` to the REPL turn state.
   - When `AgentEvent::ThinkingDelta(delta)` arrives:
     - Clear active prompt frame.
     - Emit `\x1b[2;3m{delta}\x1b[0m`.
     - Update `output_tokens` counter using `estimate_text_tokens(&delta)`.
2. **Thought Transition Barrier:**
   - When transitioning from `is_streaming_thinking == true` to `AgentEvent::TextDelta` or `AgentEvent::ToolStarted`:
     - Flush trailing newline.
     - Render subtle divider rule `\x1b[2;37m───\x1b[0m\r\n\r\n`.
     - Set `is_streaming_thinking = false`.

### 3.2 Subagent Progress Bridging (`src/agent/subagent.rs` & `src/agent/loop_runner.rs`)

1. **Event Types:**
   - Add `AgentEvent::SubagentProgressEvent(SubagentProgress)` to `src/agent/loop_runner.rs`.
2. **Progress Forwarding:**
   - In `SubagentManager::spawn`: Subscribe an `mpsc` listener to worker progress streams and forward events to the parent `event_tx` channel.
3. **REPL Tree Rendering:**
   - Render hierarchical branches:
     - `Started`: `┌─ 🤖 {role} ({name}) - {task}`
     - `ToolStarted`: `│  ⠋ {tool} {args_preview}`
     - `ToolCompleted`: `│  ✓ {tool} ({duration})`
     - `Completed`: `└─ ✓ {name} finished in {turns} turns ({duration})`
     - `Failed`: `└─ ❌ {name} failed: {error}`

### 3.3 Concurrency Scaling (`src/agent/subagent.rs`)

1. Upgrade default semaphore capacity:
   - `DEFAULT_MAX_CONCURRENT_SUBAGENTS: usize = 16`
   - Configurable up to 32 via `Config::max_concurrent_subagents`.

---

## 4. Verification & Testing Strategy

1. **Unit Tests:**
   - Verify thinking stream formatting and escape sequences.
   - Verify subagent event forwarding over Tokio channels.
   - Verify concurrency semaphore scaling under 16 concurrent workers.
2. **Integration & Smoke Test:**
   - Run a live reasoning prompt (e.g. Claude with extended thinking or DeepSeek-R1).
   - Confirm thinking text appears in dimmed italic, commits to scrollback, and cleanly transitions to final output.
   - Run a batch subagent delegation and confirm tree progress lines stream live.
