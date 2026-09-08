# Specification: Local Daemon Free Inference (Antigravity & Codex)

- **Status:** Approved / In Implementation
- **Date:** 2026-09-08
- **Target Version:** `v2.0.0-alpha.5`
- **Goal:** Auto-detect and route inference to local subscription daemons (`127.0.0.1:8045` for Antigravity Tools and `~/.codex/auth.json` for Codex) with zero manual API key configuration.

---

## 1. Objectives & User Benefits

1. **Zero-Token-Cost Inference:** Users with existing Claude Opus / Sonnet / Gemini subscriptions via Antigravity Tools or ChatGPT Plus/Pro via Codex can use Fusion completely free of charge.
2. **Zero-Configuration Discovery:** Fusion automatically reads `~/.antigravity_tools/gui_config.json` and `~/.codex/auth.json`, probes the local loopback port, and activates the models without requiring `export API_KEY=...` or manual `/login`.
3. **Live Model Enumeration:** Live models (`claude-opus-4-6`, `claude-sonnet-4-6`, `gemini-3.8-flash-high`) are probed and displayed in `/model` and `/provider` with clear badges.

---

## 2. Architecture & Components

```
┌────────────────────────────────────────────────────────┐
│ Fusion Agent Runner (src/agent/loop_runner.rs)         │
└──────────────────────────┬─────────────────────────────┘
                           │
                           ▼
┌────────────────────────────────────────────────────────┐
│ Config & Provider Dispatch (src/config.rs)             │
│  ├── Check env vars (FUSION_API_KEY, etc.)             │
│  └── If absent -> Local Daemon Probing                 │
└──────────────────────────┬─────────────────────────────┘
                           │
        ┌──────────────────┴──────────────────┐
        ▼                                     ▼
┌──────────────────────────────┐    ┌──────────────────────────────┐
│ Antigravity Daemon Provider  │    │ Codex Auth Provider          │
│ • ~/.antigravity_tools/      │    │ • ~/.codex/auth.json         │
│   gui_config.json            │    │ • access_token & account_id  │
│ • http://127.0.0.1:8045/v1   │    │ • Lazy OAuth token refresh   │
│ • OpenAI-compatible SSE      │    │ • chatgpt.com/backend-api    │
│ • Claude Opus, Sonnet, Gemini│    │ • GPT-5.3-Codex, GPT-5.4     │
└──────────────────────────────┘    └──────────────────────────────┘
```

### 2.1 Antigravity Daemon Discovery (`src/provider/local_daemon.rs`)
- Parses `~/.antigravity_tools/gui_config.json`:
  - `proxy.port` (defaults to 8045 if missing)
  - `proxy.api_key` (e.g. `sk-...`)
  - `proxy.enabled`
- Health check: `GET http://127.0.0.1:{port}/health` with 800ms timeout.
- Model discovery: `GET http://127.0.0.1:{port}/v1/models` returning catalog entries.

### 2.2 Codex Auth Discovery & Lazy Refresh
- Parses `~/.codex/auth.json`:
  - `tokens.access_token`
  - `tokens.refresh_token`
  - `tokens.account_id`
- If the access token returns 401, auto-refreshes using `https://auth.openai.com/oauth/token` with client ID `app_EMoamEEZ73f0CkXaXp7hrann` and updates the file on disk.

### 2.3 Fallback Precedence in `src/config.rs`
1. Explicit CLI flag: `--provider` / `--model`
2. Environment variables: `ANTIGRAVITY_API_KEY`, `FUSION_API_KEY`, `OPENAI_API_KEY`
3. Local Antigravity Tools daemon on `127.0.0.1:8045`
4. Local Codex credentials in `~/.codex/auth.json`
5. Fusion cloud API fallback
