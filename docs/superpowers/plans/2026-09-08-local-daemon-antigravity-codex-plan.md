# Local Daemon Free Inference (Antigravity & Codex) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Implement auto-probing and routing for local Antigravity Tools (`127.0.0.1:8045`) and Codex (`~/.codex/auth.json`) to provide zero-config, zero-token-cost inference for Claude Opus, Claude Sonnet, Gemini 3, and GPT-5 models.

**Architecture:**
1. `src/provider/local_daemon.rs`: Core detection and health-checking engine for Antigravity Tools and Codex OAuth credentials.
2. `src/config.rs`: Wire `get_key_and_url` to support `"antigravity"`, `"codex"`, and `"local"` with automatic loopback fallback.
3. `src/provider/catalog.rs`: Auto-fetch local models in `fetch_all` and tag them with `"Local Daemon (Free)"`.

**Tech Stack:** Rust (reqwest, serde, serde_json, tokio).

---

### Task 1: `src/provider/local_daemon.rs` Core Probing Engine

**Files:**
- Create: `src/provider/local_daemon.rs`
- Modify: `src/provider/mod.rs`
- Test: `tests/local_daemon_test.rs`

- [ ] **Step 1: Write integration tests in `tests/local_daemon_test.rs`**
- [ ] **Step 2: Implement `detect_antigravity_daemon() -> Option<LocalDaemonEndpoint>`**
- [ ] **Step 3: Implement `detect_codex_auth() -> Option<LocalDaemonEndpoint>`**
- [ ] **Step 4: Implement `probe_and_fetch_models()`**
- [ ] **Step 5: Verify with tests**

---

### Task 2: Config Integration & Provider Dispatch

**Files:**
- Modify: `src/config.rs`
- Modify: `src/provider/catalog.rs`
- Modify: `src/ui/slash.rs`
- Test: `tests/local_daemon_test.rs`

- [ ] **Step 1: Update `get_key_and_url` in `src/config.rs` to support local daemon**
- [ ] **Step 2: Update `fetch_all` in `src/provider/catalog.rs` to include local models**
- [ ] **Step 3: Add `/provider local` and `/provider antigravity` aliases in `src/ui/slash.rs`**
- [ ] **Step 4: Verify end-to-end with live local daemon**
