# Design Spec: Fusion Desktop (GPUIX Edition)

- **Date:** 2026-09-21
- **Status:** Approved
- **Audience:** Non-technical and developer users wanting a fast, Cursor-style agent control plane without a heavy IDE.

---

## 1. Objectives & Overview

Build a lightweight, lightning-fast (120 FPS native Metal/GPU) desktop agent control plane powered by **GPUIX** (`https://gpuix.dev/`) that controls the existing Rust **Fusion** engine via **ACP (Agent Client Protocol)**.

### Key Tenets
1. **Zero Installation Overhead:** The standalone `fusion` binary is baked directly into the desktop app bundle as a sidecar. The user never needs to touch a terminal or install CLI tools.
2. **Agent-First Control Plane:** Not an IDE. Focuses on prompts, thought steps, visual diffs, and outcomes rather than manual syntax editing.
3. **Cursor-Style Experience:** Smooth transition from centered hero input on launch to active chat stream with bottom floating composer.

---

## 2. Architecture & Sidecar Lifecycle

```
Fusion.app/Contents/
  MacOS/
    fusion-desktop    <-- Main GUI executable (GPUIX React via Bun/Hermes)
    fusion            <-- Sidecar Agent binary (Fusion Rust ACP engine)
  Resources/
    AppIcon.icns
```

### Sidecar Resolution
- **Packaged App:** Resolves `fusion` executable adjacent to `process.execPath` inside `Contents/MacOS/fusion`.
- **Local Dev (`bun --hot`):** Resolves local build artifacts at `../../target/debug/fusion` or `../../target/release/fusion`.

### Communication: Agent Client Protocol (ACP) over Stdio
- **Transport:** Standard input/output with JSON-RPC 2.0 messages.
- **Client:** `acp-client.ts` spawns the binary with `["--acp", "--cwd", projectDir]`.
- **Events Handled:**
  - `session/prompt`: Dispatches prompt to agent.
  - `turn/step`: Emits reasoning and tool execution state ("Thought 2s", "Searching files...").
  - `turn/chunk`: Streams markdown tokens to the UI in real time.
  - `turn/done`: Signals completion and updates token usage metrics.
  - `session/cancel`: Halts active turn on demand.

---

## 3. UI Layout & State Transitions

### A. View States
1. **Hero Launch State (`isChatActive == false`):**
   - App opens to a centered prompt input ("What should we build?").
   - Quick starter action pills (`Open Folder`, `Fix Bug`, `Ask Question`).
2. **Active Chat State (`isChatActive == true`):**
   - Triggered upon sending a message.
   - Composer smoothly docks to the bottom.
   - Stream area displays conversation transcript with:
     - User message bubble.
     - Collapsible thought pill (`Thought 2s ▾`).
     - Streaming native `<markdown>` assistant response.
     - Inline `<diff>` viewer for file edits.

### B. Minimalist Sidebar
- Top: `[+] New Chat` button, `🔍 Search` bar.
- Middle: Chronological chat history list (click to switch, trash to delete).
- Bottom: Active workspace directory selector (`📁 /path/to/project`) and user settings card.

---

## 4. Component Structure

```
apps/fusion-desktop/
├── package.json               # @gpuix/react, @gpuix/native, react, typescript
├── tsconfig.json              # jsxImportSource: "@gpuix/react"
├── packager.json              # cargo-packager configuration for .app
└── src/
    ├── app.tsx                # Application root, window config & layout
    ├── bridge/
    │   └── acp-client.ts      # Sidecar process manager & ACP JSON-RPC client
    ├── state/
    │   └── session-store.ts   # Chat history persistence & reactive state
    └── ui/
        ├── sidebar.tsx        # New chat, search, and history list
        ├── hero-view.tsx      # Centered initial prompt view
        ├── chat-view.tsx      # Virtualized message list, thoughts, markdown
        └── composer.tsx       # Floating bottom input with model selector
```

---

## 5. Error Handling & Edge Cases
- **Missing Sidecar Binary:** If `fusion` binary is missing in development, UI renders an informative error banner with instructions to run `cargo build`.
- **Agent Process Panic:** If the background agent exits unexpectedly, the UI displays a "Reconnecting to Fusion agent..." badge and attempts an automatic clean restart without dropping chat history.
- **Large Context / Long Output:** Message list leverages GPUIX native `<virtual-list followTail>` to ensure zero frame drops even during extended multi-turn sessions.
