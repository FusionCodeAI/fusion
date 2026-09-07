# Specification: Skills & Internal URIs, MCP Extensibility, and Goal Mode

- **Status:** Approved / In Implementation
- **Date:** 2026-09-08
- **Target Version:** `v2.0.0-alpha.5`
- **Execution Architecture:** Sequential Phases with Parallel Subagent Dispatch

---

## 1. Overview & Objectives

This specification delivers the three remaining flagship capabilities for the Fusion agent harness:
1. **Skills & Internal URI Routing:** Enable the `read` tool to auto-resolve `skill://<name>` and `rule://<name>` URIs to their respective markdown sources, and add `/skills` palette to the TUI.
2. **MCP Extensibility & Diagnostics:** Broaden MCP server discovery beyond local `.fusion/mcp.json` to include global `~/.fusion/mcp.json` and legacy IDE configs (`.claude.json`, `.cursor/mcp.json`), and add the `/mcp` command.
3. **Goal & Autonomous Plan Mode:** Surface `TaskDecomposer` and `SubagentDag` via `/goal <task>` and `/plan`, enabling multi-phase autonomous execution with subagent workers.

---

## 2. Architecture & Subsystems

### Phase 1: Skills & Internal URI Schemes
- **`src/tools/file.rs` (Internal URI Router):**
  - Intercepts paths starting with `skill://<name>`: queries `SkillRegistry::scan_default()`, locates the skill's file path, and reads it.
  - Intercepts paths starting with `rule://<name>`: checks `.fusion/rules/`, `.cursor/rules/`, or `.claude/`.
- **`src/ui/slash.rs` (`/skills` command):**
  - `/skills` (or `/skills list`): renders a clean table of all discovered project and global skills with trigger keywords.
  - `/skills show <name>`: displays the full skill instructions in markdown scrollback.
  - `/skills enable <name>` / `/skills disable <name>`: toggles active status.

### Phase 2: MCP Extensibility & `/mcp` Command
- **`src/tools/mcp_bridge.rs` (Multi-Source MCP Discovery):**
  - Priority chain:
    1. `<cwd>/.fusion/mcp.json`
    2. `<cwd>/.cursor/mcp.json` / `<cwd>/.claude.json`
    3. `~/.fusion/mcp.json`
    4. `~/.claude.json`
- **`src/ui/slash.rs` (`/mcp` command):**
  - `/mcp` or `/mcp list`: lists connected servers, protocol version, and exposed tools.
  - `/mcp reload`: disconnects and re-initializes all MCP servers live without restarting the REPL.
  - `/mcp status`: tests server process health and latency.

### Phase 3: Goal Mode & Phased Plan Execution
- **`src/agent/plan_runner.rs` (Autonomous Goal Driver):**
  - Takes a `SubagentDag` created by `TaskDecomposer::decompose(goal)`.
  - Executes stages in topological order:
    - Stage 1: Parallel `Scout` subagents
    - Stage 2: Parallel `Coder` subagents
    - Stage 3: `Tester` & `Reviewer` verification
- **`src/ui/slash.rs` (`/goal` and `/plan` commands):**
  - `/goal <description>`: decomposes the goal and previews the phased DAG.
  - `/plan run`: runs the active DAG autonomously until completion or verification failure.
  - `/plan status`: prints live ASCII DAG execution status.

---

## 3. Subagent Parallel Decomposition

| Slice | Ownership Files | Subagent Role | Concurrency |
| :--- | :--- | :--- | :---: |
| **Slice A: URI Router & `/skills`** | `src/tools/file.rs`, `src/tools/uri_router.rs`, `src/ui/slash.rs` | `Coder` | Parallel with B |
| **Slice B: MCP Multi-Config & `/mcp`** | `src/tools/mcp_bridge.rs`, `src/tools/mcp.rs`, `src/ui/slash.rs` | `Coder` | Parallel with A |
| **Slice C: `/goal` & Phased Driver** | `src/agent/plan_runner.rs`, `src/agent/plan.rs`, `src/ui/slash.rs` | `Coder` | Sequential after A & B |
