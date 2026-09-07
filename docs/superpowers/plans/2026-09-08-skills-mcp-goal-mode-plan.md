# Skills & Internal URIs, MCP Multi-Discovery, and Goal Mode Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Implement `skill://` & `rule://` URI resolution in the file read tool, `/skills` and `/mcp` management slash commands, multi-source MCP discovery (`~/.fusion/mcp.json`, `.cursor/mcp.json`, `.claude.json`), and the `/goal` & `/plan` autonomous phased execution engine.

**Architecture:**
1. `src/tools/uri_router.rs`: Pure-Rust URI interceptor for `skill://<name>` and `rule://<name>` wired into `FileReadTool`.
2. `src/tools/mcp_bridge.rs`: Multi-directory configuration resolver scanning workspace, user global, Cursor, and Claude MCP configs.
3. `src/agent/plan_runner.rs`: Autonomous DAG coordinator executing topological stages using `SubagentManager`.
4. `src/ui/slash.rs`: Registers `/skills`, `/mcp`, `/goal`, and `/plan` commands with rich TUI formatting.

**Tech Stack:** Rust (Tokio, serde, serde_json, crossterm).

---

### Task 1: Internal URI Router & `skill://` Resolution in `read` Tool

**Files:**
- Create: `src/tools/uri_router.rs`
- Modify: `src/tools/mod.rs`
- Modify: `src/tools/file.rs`
- Test: `tests/uri_router_test.rs`

- [ ] **Step 1: Write integration test for URI resolution**
- [ ] **Step 2: Implement `resolve_internal_uri` in `src/tools/uri_router.rs`**
- [ ] **Step 3: Wire into `FileReadTool::execute` in `src/tools/file.rs`**
- [ ] **Step 4: Run tests and verify**

---

### Task 2: Multi-Source MCP Discovery & `/mcp` Slash Command

**Files:**
- Modify: `src/tools/mcp_bridge.rs`
- Modify: `src/ui/slash.rs`
- Test: `tests/mcp_bridge_test.rs`

- [ ] **Step 1: Add global and legacy config discovery in `mcp_bridge.rs`**
- [ ] **Step 2: Add `/mcp [list|reload|status]` to `src/ui/slash.rs`**
- [ ] **Step 3: Verify with unit tests**

---

### Task 3: `/skills` Slash Command & Skill Management

**Files:**
- Modify: `src/ui/slash.rs`
- Test: `tests/skills_test.rs`

- [ ] **Step 1: Add `/skills [list|show|enable|disable]` in `src/ui/slash.rs`**
- [ ] **Step 2: Connect to `SkillRegistry` for interactive inspection**
- [ ] **Step 3: Verify with tests**

---

### Task 4: Autonomous Goal & Plan Execution Mode (`/goal` & `/plan`)

**Files:**
- Create: `src/agent/plan_runner.rs`
- Modify: `src/agent/mod.rs`
- Modify: `src/ui/slash.rs`
- Test: `tests/plan_runner_test.rs`

- [ ] **Step 1: Implement `PlanRunner` DAG execution loop in `src/agent/plan_runner.rs`**
- [ ] **Step 2: Add `/goal <prompt>` and `/plan [run|status|cancel]` to `src/ui/slash.rs`**
- [ ] **Step 3: Verify end-to-end with tests**
