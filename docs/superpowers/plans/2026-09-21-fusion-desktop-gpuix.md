# Fusion Desktop (GPUIX Edition) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build a high-performance native desktop agent control plane powered by GPUIX (`https://gpuix.dev/`) that integrates the standalone `fusion` Rust engine as a baked-in ACP sidecar, featuring a Cursor-inspired transition from hero input to active chat stream.

**Architecture:** A standalone React 19 application rendered via GPUIX into Zed's Metal/GPU engine. The UI spawns and manages `fusion --acp` over standard I/O using JSON-RPC 2.0 to stream thought reasoning, tool execution, markdown responses, and diffs.

**Tech Stack:** TypeScript, Bun, React 19, `@gpuix/react`, `@gpuix/native`, `fusion` ACP (JSON-RPC 2.0).

---

### Task 1: Scaffold GPUIX Desktop Application Workspace

**Files:**
- Create: `apps/fusion-desktop/package.json`
- Create: `apps/fusion-desktop/tsconfig.json`
- Create: `apps/fusion-desktop/src/types.ts`
- Test: `apps/fusion-desktop/tests/smoke.test.ts`

- [ ] **Step 1: Write initial types and smoke test**

```typescript
// apps/fusion-desktop/tests/smoke.test.ts
import { describe, expect, it } from "bun:test";

describe("GPUIX Desktop Environment", () => {
  it("verifies Bun runtime environment", () => {
    expect(typeof Bun).toBe("object");
  });
});
```

- [ ] **Step 2: Run test to verify environment**

Run: `cd apps/fusion-desktop && bun test tests/smoke.test.ts`
Expected: PASS

- [ ] **Step 3: Create package.json and tsconfig.json**

Configure dependencies and set `"jsxImportSource": "@gpuix/react"`.

```json
{
  "name": "fusion-desktop",
  "version": "2.0.0",
  "private": true,
  "type": "module",
  "scripts": {
    "dev": "bun --hot src/app.tsx",
    "test": "bun test",
    "typecheck": "tsc --noEmit"
  },
  "dependencies": {
    "@gpuix/native": "0.0.8",
    "@gpuix/react": "0.0.8",
    "react": "^19.0.0"
  },
  "devDependencies": {
    "@types/bun": "latest",
    "@types/react": "^19.0.0",
    "typescript": "^5.7.0"
  }
}
```

- [ ] **Step 4: Run package install and typecheck**

Run: `cd apps/fusion-desktop && bun install && bun test`
Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add apps/fusion-desktop
git commit -m "feat(desktop): scaffold apps/fusion-desktop workspace with GPUIX dependencies"
```

---

### Task 2: Build ACP JSON-RPC Sidecar Client Bridge

**Files:**
- Create: `apps/fusion-desktop/src/bridge/acp-client.ts`
- Test: `apps/fusion-desktop/tests/acp-client.test.ts`

- [ ] **Step 1: Write failing ACP client tests**

Test binary resolution, JSON-RPC 2.0 message framing, request/response matching, and event dispatching (`turn/step`, `turn/chunk`, `turn/done`).

```typescript
// apps/fusion-desktop/tests/acp-client.test.ts
import { describe, expect, it } from "bun:test";
import { AcpClient, parseJsonRpcLine } from "../src/bridge/acp-client";

describe("ACP Client Protocol", () => {
  it("parses valid JSON-RPC 2.0 lines", () => {
    const parsed = parseJsonRpcLine('{"jsonrpc":"2.0","method":"turn/chunk","params":{"delta":"hello"}}\n');
    expect(parsed?.method).toBe("turn/chunk");
    expect(parsed?.params?.delta).toBe("hello");
  });

  it("handles malformed JSON lines safely", () => {
    const parsed = parseJsonRpcLine('not-json\n');
    expect(parsed).toBeNull();
  });
});
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cd apps/fusion-desktop && bun test tests/acp-client.test.ts`
Expected: FAIL (modules not found)

- [ ] **Step 3: Implement AcpClient with sidecar resolution and stdio streaming**

Implement `AcpClient` with:
- `resolveBinaryPath()`: searches packaged `.app/Contents/MacOS/fusion`, local `target/release/fusion`, and `target/debug/fusion`.
- `spawn(workspaceDir: string)`: launches child process with piped stdio.
- `sendRequest(method: string, params?: unknown)`: formats JSON-RPC with incrementing IDs.
- Event listeners for `onStep`, `onChunk`, `onDone`, `onError`.

- [ ] **Step 4: Run test to verify it passes**

Run: `cd apps/fusion-desktop && bun test tests/acp-client.test.ts`
Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add apps/fusion-desktop/src/bridge/acp-client.ts apps/fusion-desktop/tests/acp-client.test.ts
git commit -m "feat(desktop): implement ACP JSON-RPC sidecar client bridge"
```

---

### Task 3: Build Session State Store & Persistence

**Files:**
- Create: `apps/fusion-desktop/src/state/session-store.ts`
- Test: `apps/fusion-desktop/tests/session-store.test.ts`

- [ ] **Step 1: Write failing session store tests**

```typescript
// apps/fusion-desktop/tests/session-store.test.ts
import { describe, expect, it } from "bun:test";
import { SessionStore } from "../src/state/session-store";

describe("SessionStore", () => {
  it("creates a new active session", () => {
    const store = new SessionStore();
    const session = store.createSession("New Task");
    expect(store.getActiveSession()?.id).toBe(session.id);
    expect(session.messages.length).toBe(0);
  });

  it("appends user and assistant messages with streaming chunks", () => {
    const store = new SessionStore();
    const session = store.createSession("Test");
    store.appendUserMessage("Hello agent");
    expect(store.getActiveSession()?.messages.length).toBe(1);
    store.appendThoughtStep("Analyzing codebase...");
    store.appendAssistantChunk("I am ready");
    const msgs = store.getActiveSession()?.messages ?? [];
    expect(msgs.length).toBe(2);
    expect(msgs[1].content).toBe("I am ready");
    expect(msgs[1].thought).toContain("Analyzing codebase...");
  });
});
```

- [ ] **Step 2: Run test to verify failure**

Run: `cd apps/fusion-desktop && bun test tests/session-store.test.ts`
Expected: FAIL

- [ ] **Step 3: Implement SessionStore**

Implement state management with active session, message list, thought chains, model selection, and disk persistence to `~/.fusion/desktop-sessions.json`.

- [ ] **Step 4: Run test to verify it passes**

Run: `cd apps/fusion-desktop && bun test tests/session-store.test.ts`
Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add apps/fusion-desktop/src/state/session-store.ts apps/fusion-desktop/tests/session-store.test.ts
git commit -m "feat(desktop): implement session state store and message persistence"
```

---

### Task 4: Build Sidebar Component

**Files:**
- Create: `apps/fusion-desktop/src/ui/sidebar.tsx`
- Test: `apps/fusion-desktop/tests/sidebar.test.ts`

- [ ] **Step 1: Write sidebar test**

Verify session list rendering, active indicator, and folder selection event handlers.

- [ ] **Step 2: Implement Sidebar component**

Features:
- Header: `+ New Chat` button, `🔍 Search` filter input.
- Session list: displays titles, timestamps, active highlight, delete button.
- Footer: current workspace folder (`📁 /path/to/project`) and user profile chip.

- [ ] **Step 3: Verify tests**

Run: `cd apps/fusion-desktop && bun test tests/sidebar.test.ts`
Expected: PASS

- [ ] **Step 4: Commit**

```bash
git add apps/fusion-desktop/src/ui/sidebar.tsx apps/fusion-desktop/tests/sidebar.test.ts
git commit -m "feat(desktop): implement clean sidebar with history and folder selector"
```

---

### Task 5: Build Hero Initial State View

**Files:**
- Create: `apps/fusion-desktop/src/ui/hero-view.tsx`
- Test: `apps/fusion-desktop/tests/hero-view.test.ts`

- [ ] **Step 1: Write hero view test**

Verify initial centered layout, prompt input, and suggestion pill click triggers.

- [ ] **Step 2: Implement HeroView component**

Features:
- Centered typography: "What should we build?"
- Prominent input box with placeholder, attachment button, and submit action.
- Action pills: `[📁 Open Folder]`, `[⚡ Fix Issue]`, `[💬 Ask Question]`.

- [ ] **Step 3: Verify tests**

Run: `cd apps/fusion-desktop && bun test tests/hero-view.test.ts`
Expected: PASS

- [ ] **Step 4: Commit**

```bash
git add apps/fusion-desktop/src/ui/hero-view.tsx apps/fusion-desktop/tests/hero-view.test.ts
git commit -m "feat(desktop): implement hero initial prompt view"
```

---

### Task 6: Build Active Chat Stream View

**Files:**
- Create: `apps/fusion-desktop/src/ui/chat-view.tsx`
- Test: `apps/fusion-desktop/tests/chat-view.test.ts`

- [ ] **Step 1: Write chat view test**

Verify message mapping, thought accordion toggling, and native component integration.

- [ ] **Step 2: Implement ChatView component**

Features:
- `<virtual-list followTail estimatedItemHeight={140}>` container.
- User message bubble with rounded pill styling.
- Collapsible thought container (`Thought 2s ▾`) displaying model reasoning.
- Native `<markdown source={msg.content} />` for streaming markdown.
- Inline `<diff patch={patch} wordDiff />` for file modifications.

- [ ] **Step 3: Verify tests**

Run: `cd apps/fusion-desktop && bun test tests/chat-view.test.ts`
Expected: PASS

- [ ] **Step 4: Commit**

```bash
git add apps/fusion-desktop/src/ui/chat-view.tsx apps/fusion-desktop/tests/chat-view.test.ts
git commit -m "feat(desktop): implement active chat stream with virtual list and markdown"
```

---

### Task 7: Build Bottom Floating Composer

**Files:**
- Create: `apps/fusion-desktop/src/ui/composer.tsx`
- Test: `apps/fusion-desktop/tests/composer.test.ts`

- [ ] **Step 1: Write composer test**

Verify input submission, model switching, and stop button states.

- [ ] **Step 2: Implement Composer component**

Features:
- Floating docked pill input at the bottom of the active chat view.
- Native `<textarea onSubmit={...} />` with Enter-to-send and Shift+Enter for newlines.
- Attachment `(+)` button, model picker (`[Auto ⌵]`), and voice/mic icon.
- Sub-bar with execution target (`💻 This Mac ⌵`) and abort/stop generation button when running.

- [ ] **Step 3: Verify tests**

Run: `cd apps/fusion-desktop && bun test tests/composer.test.ts`
Expected: PASS

- [ ] **Step 4: Commit**

```bash
git add apps/fusion-desktop/src/ui/composer.tsx apps/fusion-desktop/tests/composer.test.ts
git commit -m "feat(desktop): implement bottom floating composer with model selector"
```

---

### Task 8: Wire App Root, Packaging Configuration, and End-to-End Verification

**Files:**
- Create: `apps/fusion-desktop/src/app.tsx`
- Create: `apps/fusion-desktop/packager.json`
- Test: `apps/fusion-desktop/tests/app.e2e.test.ts`

- [ ] **Step 1: Wire all components into app.tsx**

Manage the two primary view states (`isChatActive`):
- When empty: render `HeroView`.
- When user sends prompt: switch to `ChatView` + `Composer`.
- Connect `AcpClient` events to `SessionStore`.

- [ ] **Step 2: Setup packager.json for cargo-packager**

Configure the standalone `.app` bundle with `fusion` binary bundled as a sidecar:

```json
{
  "productName": "Fusion",
  "version": "2.0.0",
  "identifier": "ai.fusion.desktop",
  "binariesDir": "dist",
  "outDir": "bundle",
  "binaries": [
    { "path": "fusion-desktop", "main": true },
    { "path": "fusion", "main": false }
  ],
  "formats": ["app"]
}
```

- [ ] **Step 3: Write and run end-to-end integration test**

Verify application bootstrap, ACP handshake, session creation, prompt dispatch, and state transition.

Run: `cd apps/fusion-desktop && bun test`
Expected: ALL PASS

- [ ] **Step 4: Final commit**

```bash
git add apps/fusion-desktop/src/app.tsx apps/fusion-desktop/packager.json apps/fusion-desktop/tests/app.e2e.test.ts
git commit -m "feat(desktop): complete fusion desktop gpuix integration with acp sidecar"
```
