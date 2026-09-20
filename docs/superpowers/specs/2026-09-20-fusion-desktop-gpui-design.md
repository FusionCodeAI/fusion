# Fusion Desktop: Native GPUI Control Plane for Fusion Coding Agent

**Date**: 2026-09-20  
**Status**: Draft / Proposed  
**Author**: Fusion Authors & Antigravity  
**Target Crate**: `apps/fusion-desktop` (Workspace Member)

---

## 1. Executive Summary & Problem Statement

Fusion is a fast, lightweight, cross-platform AI coding assistant written in Rust. While power users and terminal veterans thrive in the current CLI/TUI environment, older users and non-technical developers experience significant friction:
1. **Command & Syntax Anxiety**: Remembering slash commands, CLI flags, or working directory arguments creates hesitation.
2. **Visual & Accessibility Barriers**: Monospace terminal fonts are often too small, lack proper line-height scaling, and provide poor visual hierarchy for older eyes.
3. **Raw Terminal & Diff Overload**: Unformatted terminal diffs and rapid text scrolling make it difficult to understand what files the agent is reading, modifying, or deleting.
4. **Lack of Mouse Affordances**: Older users expect obvious, large click targets, native folder open dialogs, and unambiguous **[ Approve ]** and **[ Reject ]** buttons rather than terminal keystroke combinations.

### The Solution
We propose **Fusion Desktop** (`apps/fusion-desktop`): a native, GPU-accelerated desktop application built in Rust using Zed's **GPUI** framework (incorporating proven architectural patterns from **Zeron**). Fusion Desktop serves as a friendly, zero-CLI control plane that:
- Runs Fusion's core engine in-process over its existing Agent Client Protocol (ACP) JSON-RPC 2.0 interface.
- Provides an accessible, high-legibility interface with point-and-click project selection, one-click starter actions, and interactive diff cards.
- Implements an extensible `AgentHarness` abstraction from day one, allowing other coding agent CLIs (Claude Code, Codex, Cursor) to be orchestrated from the same desktop interface in future releases.

---

## 2. Goals & Non-Goals

### Goals
- **Dedicated Workspace Crate**: Add `apps/fusion-desktop` to the existing Cargo workspace without adding GPUI dependencies to the core CLI `fusion` binary.
- **Zero Terminal Knowledge Required**:
  - Native OS folder picker dialog (`rfd`) for opening projects.
  - One-click prompt pills (e.g. `[ 💡 Explain Project ]`, `[ 🔍 Find Errors ]`, `[ 🧪 Run Tests ]`).
  - Text-size zoom controls (`[-]` / `[+]`, 100%–175%) and high-contrast, large typography.
  - Clear visual tool cards showing agent actions in plain language.
  - One-click **[ ✓ Approve ]** and **[ ✕ Reject ]** buttons for file mutations.
- **Decoupled `AgentHarness` Architecture**:
  - Trait-based interface separating the UI viewport from the agent runtime.
  - Phase 1: `FusionAcpHarness` connecting to Fusion's in-process `AcpServer` via in-memory duplex channel.
  - Phase 2 Ready: Pre-designed extension points for Claude Code (`stream-json`) and Codex (`app-server`).
- **Single Native Binary**: Standalone executable on macOS and Windows with fast startup (<100ms) and minimal RAM footprint (~30–40 MB).

### Non-Goals
- Modifying or breaking the existing terminal CLI/TUI (`fusion`) — the CLI remains completely independent and unmodified.
- Cloudflare edge multi-device synchronization in Phase 1 (Zeron-style remote sync can be added in a later milestone; Phase 1 is strictly local-first).
- Implementing a full-blown text editor/IDE — code review is performed via diff inspection cards, with an optional "Open in VS Code / Cursor" button.

---

## 3. High-Level Architecture

```
┌─────────────────────────────────────────────────────────────────────────────┐
│                         apps/fusion-desktop (GPUI)                          │
│                                                                             │
│  ┌───────────────────────────┐         ┌─────────────────────────────────┐  │
│  │   Sidebar (Projects/Spaces)│         │     Transcript & Diff View      │  │
│  │   • Visual Folder Picker  │         │     • Virtualized ListState     │  │
│  │   • Past Sessions List    │         │     • Streaming Markdown        │  │
│  │   • Text Zoom Controls    │         │     • Visual Tool & Diff Cards  │  │
│  └─────────────┬─────────────┘         └────────────────▲────────────────┘  │
│                │                                        │                   │
│                └───────────────────┬────────────────────┘                   │
│                                    │                                        │
│                            GPUI State Models                                │
│                                    ▲                                        │
│                                    │                                        │
│                         AgentHarness Abstraction                            │
│                                    │                                        │
└────────────────────────────────────┼────────────────────────────────────────┘
                                     │
           Bidirectional JSON-RPC 2.0 (ACP over in-memory duplex)
                                     │
┌────────────────────────────────────▼────────────────────────────────────────┐
│                        Fusion Core Engine (`src/acp/`)                      │
│                                                                             │
│  • `AcpServer`: Session management & JSON-RPC handler                       │
│  • `AgentRunner`: Main agent loop & LLM streaming                           │
│  • `ToolRegistry`: Native tools (read, edit, bash, glob, grep)              │
│  • Runs in-process on an asynchronous background Tokio thread               │
└─────────────────────────────────────────────────────────────────────────────┘
```

---

## 4. Module Structure (`apps/fusion-desktop`)

```
apps/fusion-desktop/
├── Cargo.toml
└── src/
    ├── main.rs                  # Application entry point, window bootstrap
    ├── app.rs                   # Main layout container (Sidebar + Workspace View)
    ├── harness/
    │   ├── mod.rs               # AgentHarness trait, SessionHandle, Event types
    │   └── fusion.rs            # FusionAcpHarness implementation (bridges to AcpServer)
    ├── state/
    │   ├── session.rs           # Active session state, messages, tool status
    │   └── settings.rs          # UI settings (font scale, theme, recent folders)
    └── ui/
        ├── sidebar.rs           # Project folder button, session history list
        ├── transcript.rs        # Virtualized GPUI list, message rendering
        ├── tool_card.rs         # Friendly visual cards for tool actions
        ├── diff_card.rs         # Syntax-colored diff viewer + Approve/Reject buttons
        ├── composer.rs          # Large input box, starter prompts, Send/Stop buttons
        └── theme.rs             # Accessible high-contrast colors, font-size scales
```

---

## 5. Detailed Component Specifications

### 5.1 Project & Session Management (`ui/sidebar.rs`)
- **Visual Folder Picker**:
  - Large button at top of sidebar: `[ 📁 Open Project Folder ]`.
  - Triggers native file dialog via `rfd::AsyncFileDialog`.
  - Displays the current active directory name prominently with a subtle path subtitle.
- **Session History**:
  - Lists past conversations in reverse chronological order (Today, Yesterday, Older).
  - Clicking a session restores the conversation transcript and active working directory.
- **Accessibility Header**:
  - Global font size stepper: `[ - ]` `125%` `[ + ]`.
  - Instantly recalculates rem/pixel dimensions across the entire GPUI tree.

### 5.2 Accessible Conversation Transcript (`ui/transcript.rs`)
- **Virtualized List Rendering**:
  - Uses GPUI's `ListState` with stick-to-bottom behavior during active streaming.
  - Rows are memoized by message block to prevent full-transcript re-renders.
- **Visual Hierarchy**:
  - User messages: Distinct rounded bubble aligned to the right or styled with an accent background.
  - Assistant messages: Clean full-width reading cards with generous line height (1.6) and high-contrast text.
  - Streaming Indicator: Gentle pulsing indicator showing the agent is actively thinking.

### 5.3 Tool Cards & Approval Controls (`ui/tool_card.rs` & `ui/diff_card.rs`)
- **Plain-English Tool Summaries**:
  - `read_file` → `📖 Reading src/config.rs`
  - `grep` → `🔍 Searching for "auth" across project`
  - `bash` → `⚙ Running test suite (cargo test)`
  - Collapsible details toggle (`[Show Details ▾]`) for curious users, hidden by default to avoid visual clutter.
- **Interactive Diff Review Cards**:
  - Line-by-line diff with soft red backgrounds for deleted lines and soft green for added lines.
  - Action Footer:
    - Big green button: `[ ✓ Approve & Apply Changes ]` (minimum 44px height, high-contrast text).
    - Gray outline button: `[ ✕ Reject Changes ]`.
    - Clicking an option immediately resolves the pending ACP permission request.

### 5.4 The Composer (`ui/composer.rs`)
- **Generous Text Area**:
  - Minimum height of 80px, auto-growing up to 240px.
  - Large placeholder: `"What would you like Fusion to help with? Type here..."`.
- **Starter Prompt Pills**:
  - Horizontal scroll/wrap row of suggestions:
    - `[ 💡 Explain this codebase ]`
    - `[ 🔍 Check for bugs ]`
    - `[ 🧪 Run tests & fix failures ]`
    - `[ 📝 Write documentation ]`
  - Clicking a pill populates the input field and focuses it.
- **Primary Controls**:
  - When idle: Blue `[ Send ➔ ]` button.
  - While running: Red `[ ⏹ Stop Agent ]` button to abort execution safely.

---

## 6. Data Flow & Concurrency Model

### 6.1 In-Process ACP Bridge
1. When `fusion-desktop` launches:
   ```rust
   let (client_io, server_io) = tokio::io::duplex(64 * 1024);
   let server = AcpServer::new(config, client, tools, tool_ctx);
   tokio::spawn(async move {
       server.run(server_io).await;
   });
   ```
2. The `FusionAcpHarness` owns `client_io`, framing JSON-RPC 2.0 messages over standard buffers.
3. The UI thread communicates with the harness via GPUI's async task spawner (`cx.spawn`), ensuring UI rendering remains at a rock-solid 120 FPS on Metal/DirectX.

### 6.2 The `AgentHarness` Trait
```rust
#[async_trait]
pub trait AgentHarness: Send + Sync {
    async fn start_session(&self, cwd: PathBuf) -> anyhow::Result<Box<dyn HarnessSession>>;
}

#[async_trait]
pub trait HarnessSession: Send + Sync {
    async fn send_prompt(&mut self, text: String) -> anyhow::Result<()>;
    async fn cancel(&mut self) -> anyhow::Result<()>;
    async fn respond_approval(&mut self, request_id: String, approved: bool) -> anyhow::Result<()>;
    fn event_stream(&mut self) -> Pin<Box<dyn Stream<Item = HarnessEvent> + Send>>;
}
```
This trait guarantees that when we add Claude Code or Codex later, we only need to implement a new `HarnessSession` struct; zero UI code changes are required.

---

## 7. Error Handling & Safety

1. **User-Friendly Error Cards**:
   - Network errors display a clear notice: `"Unable to reach AI provider. Please check your internet connection."` with a `[ Retry ]` button.
   - Missing API keys surface an inline setup card prompting for the key, which writes directly to Fusion's config upon entry.
2. **Safe Workspace Operations**:
   - Every file edit operation is checked against git status before execution.
   - If a change produces unexpected results, the user can click an **[ Undo Changes ]** button that runs a clean checkout for the affected file.

---

## 8. Verification & Testing Plan

1. **Automated Unit Tests**:
   - Test `FusionAcpHarness` in-memory duplex communication: verify session initialization, prompt dispatching, event parsing (`TextDelta`, `ToolStart`, `ApprovalRequest`), and response framing.
   - Test zoom level scaling calculations and theme color contrast ratios.
2. **Interactive Smoke Verification**:
   - Build and launch `fusion-desktop`.
   - Open a test workspace using the visual folder picker.
   - Execute a prompt (e.g. `"Explain main.rs"`): verify streaming text renders smoothly without UI lag.
   - Request a file edit: verify the interactive diff card appears, click `[ Approve ]`, and confirm the file is safely modified on disk.
   - Test text scaling: adjust zoom from 100% to 150% and ensure all text and buttons scale gracefully.
