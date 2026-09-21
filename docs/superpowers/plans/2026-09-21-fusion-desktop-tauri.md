# Fusion Desktop (Tauri v2 + Cline UI Edition) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build a production-grade, 100% pixel-perfect desktop agent control plane powered by **Tauri v2 + React 19 + Tailwind v4 + Lucide icons**, adapting the battle-tested UI patterns from Cline (`reference/cline/apps/vscode/webview-ui`) and connecting to the standalone `fusion --acp` Rust engine sidecar via `@fusioncode/sdk`.

**Architecture:** Tauri v2 native desktop shell wrapping a Vite + React 19 + Tailwind v4 web frontend. The backend agent runs as a bundled native sidecar (`externalBin: ["fusion"]`) communicating over ACP (JSON-RPC 2.0 on stdio) via the official `@fusioncode/sdk`.

**Tech Stack:** Tauri v2, Rust, Vite, React 19, Tailwind CSS v4, Lucide React (`lucide-react`), `@fusioncode/sdk`.

---

### Task 1: Scaffold Tauri v2 + React 19 + Tailwind v4 Workspace

**Files:**
- Modify: `apps/fusion-desktop/package.json`
- Create: `apps/fusion-desktop/vite.config.ts`
- Create: `apps/fusion-desktop/index.html`
- Create: `apps/fusion-desktop/src-tauri/Cargo.toml`
- Create: `apps/fusion-desktop/src-tauri/tauri.conf.json`
- Create: `apps/fusion-desktop/src-tauri/src/main.rs`
- Create: `apps/fusion-desktop/src/index.css`

- [ ] **Step 1: Setup package.json and vite.config.ts**
  Configure React 19, `@tailwindcss/vite`, `lucide-react`, `@fusioncode/sdk`, `@tauri-apps/api`, and `@tauri-apps/plugin-shell`.
- [ ] **Step 2: Setup src-tauri Cargo.toml, tauri.conf.json, and main.rs**
  Configure transparent titlebar, traffic light positioning, sidecar `externalBin: ["fusion"]`.
- [ ] **Step 3: Run bun install and verify build**
  Run: `cd apps/fusion-desktop && bun install && bun run build`
  Expected: PASS
- [ ] **Step 4: Commit**
  `git commit -m "feat(desktop): scaffold tauri v2 + react 19 + tailwind v4 workspace"`

---

### Task 2: Implement Agent Bridge using `@fusioncode/sdk`

**Files:**
- Create: `apps/fusion-desktop/src/lib/agent-bridge.ts`
- Create: `apps/fusion-desktop/src/lib/models.ts`
- Test: `apps/fusion-desktop/tests/agent-bridge.test.ts`

- [ ] **Step 1: Write agent bridge tests**
- [ ] **Step 2: Implement AgentBridge using FusionAgent / ACP stdio**
  Connect `FusionAgent` from `@fusioncode/sdk` to the Tauri sidecar, handling `thought`, `chunk`, `tool_call`, `diff`, and `done` events.
- [ ] **Step 3: Verify tests**
  Run: `cd apps/fusion-desktop && bun test tests/agent-bridge.test.ts`
  Expected: PASS
- [ ] **Step 4: Commit**
  `git commit -m "feat(desktop): implement agent bridge with @fusioncode/sdk and models catalog"`

---

### Task 3: Adapt Chat Message & Thinking Components (Cline Style)

**Files:**
- Create: `apps/fusion-desktop/src/components/ThinkingRow.tsx`
- Create: `apps/fusion-desktop/src/components/UserMessage.tsx`
- Create: `apps/fusion-desktop/src/components/ToolCallRow.tsx`
- Create: `apps/fusion-desktop/src/components/DiffView.tsx`
- Test: `apps/fusion-desktop/tests/chat-components.test.ts`

- [ ] **Step 1: Implement ThinkingRow with unboxed markdown styling**
- [ ] **Step 2: Implement UserMessage as a right-aligned pill**
- [ ] **Step 3: Implement ToolCallRow as a compact collapsible 28px action pill**
- [ ] **Step 4: Implement DiffView using syntax-highlighted unified diffs**
- [ ] **Step 5: Run tests and commit**
  `git commit -m "feat(desktop): adapt cline chat components with tailwind v4"`

---

### Task 4: Adapt Composer & Model Selector

**Files:**
- Create: `apps/fusion-desktop/src/components/Composer.tsx`
- Test: `apps/fusion-desktop/tests/composer.test.ts`

- [ ] **Step 1: Implement Composer with multi-line auto-resizing textarea**
  Native dark caret, no text clipping, `Enter` to send, `Shift+Enter` for newlines.
- [ ] **Step 2: Add Fusion AI model picker dropdown**
  DeepSeek 4 Flash, DeepSeek 4 Fast, GLM 5.3 Flash, MiniMax M2.7.
- [ ] **Step 3: Add bottom sub-row with branch and context limit meter**
- [ ] **Step 4: Verify tests and commit**
  `git commit -m "feat(desktop): implement elevated composer card and model picker"`

---

### Task 5: Build Top Header, Sidebar & App Shell

**Files:**
- Create: `apps/fusion-desktop/src/components/TopHeader.tsx`
- Create: `apps/fusion-desktop/src/components/Sidebar.tsx`
- Modify: `apps/fusion-desktop/src/App.tsx`
- Test: `apps/fusion-desktop/tests/app-shell.test.ts`

- [ ] **Step 1: Implement TopHeader matching 40px unified strip**
  Left: session title + drawer icon. Right: `...` and `[|]`.
- [ ] **Step 2: Implement Sidebar matching 1:1 reference**
  Traffic light clearance, `New Chat`, `Search`, `Projects (+)` with `◌ New Project`, user profile.
- [ ] **Step 3: Wire Cmd+B sidebar toggle and centering**
- [ ] **Step 4: Run tests and commit**
  `git commit -m "feat(desktop): implement top header, sidebar, and app shell"`

---

### Task 6: End-to-End Integration & Verification

- [ ] **Step 1: Build frontend and verify typecheck**
  Run: `cd apps/fusion-desktop && bun run typecheck && bun run build`
  Expected: PASS
- [ ] **Step 2: Launch and verify Tauri window**
- [ ] **Step 3: Final commit**
