# @fusioncode/sdk — SDK

> **Official TypeScript & WebAssembly SDK for the Fusion AI Coding Assistant**
> A pure-Rust agent engine compiled to WebAssembly (or driven over a stdio JSON-RPC
> process) with an in-memory Virtual File System (VFS), streaming event turns,
> multi-agent advisors, session checkpoints, and a turnkey xterm.js terminal adapter.

[![npm version](https://img.shields.io/npm/v/@fusioncode/sdk.svg?style=flat-square)](https://www.npmjs.com/package/@fusioncode/sdk)
[![License](https://img.shields.io/badge/License-MIT-blue.svg?style=flat-square)](LICENSE)
[![TypeScript](https://img.shields.io/badge/TypeScript-5.6%2B-blue.svg?style=flat-square)](#type-reference)
[![WebAssembly](https://img.shields.io/badge/Core-Pure%20Rust%20WASM-purple.svg?style=flat-square)](#cross-environment-notes)
[![Engines](https://img.shields.io/badge/node-%E2%89%A518-green.svg?style=flat-square)](#cross-environment-notes)

---

## Table of Contents

- [Features](#features)
- [Installation](#installation)
- [Quick Start](#quick-start)
  - [WASM agent (browser & Node)](#1-wasm-agent-browser--node)
  - [Stdio agent (Node CLI backend)](#2-stdio-agent-node-cli-backend)
- [Streaming Events](#streaming-events)
- [Virtual File System (VFS)](#virtual-file-system-vfs)
- [xterm.js Terminal Adapter](#xtermjs-terminal-adapter)
- [Model Switching](#model-switching)
- [Sessions & Checkpoints](#sessions--checkpoints)
- [Configuration Reference](#configuration-reference)
- [Type Reference](#type-reference)
- [Cross-Environment Notes (Node vs Browser)](#cross-environment-notes-node-vs-browser)
- [Framework Integration](#framework-integration)
- [License](#license)

---

## Features

- 🦀 **Pure-Rust WebAssembly core** — zero C/C++ dependencies or native binaries; runs in every modern browser and Node.js ≥ 18.
- 🖥️ **Dual transports** — in-memory WASM (`WasmTransport`) in browsers, and a JSON-RPC 2.0 / ACP child-process transport (`StdioTransport`) spawning `fusion --acp` in Node.js, Bun, or Deno. A custom `AgentTransport` plugs in for anything else (e.g. WebSocket daemons).
- 📁 **Sandboxed in-memory VFS** — `read`, `write`, `edit`, `grep`, `glob`, and simulated `bash` tools operating on a fully isolated virtual workspace.
- 🌊 **Streaming event pipeline** — typed deltas for text, thinking traces, tool lifecycle, advisor critiques, token stats, and errors.
- 🧑‍⚖️ **Multi-agent advisors** — built-in Architect / Security / Performance critics evaluate plans before and after tool calls.
- 💻 **xterm.js terminal adapter** — REPL line editing, history navigation, ANSI themes, spinners, slash commands, and three pluggable backends (WASM / WebSocket / Mock).
- 💾 **Session checkpoints** — one-call JSON snapshot and restore of messages, config, token stats, and the entire VFS.
- 🌐 **Universal model support** — OpenRouter, Anthropic, OpenAI, DeepSeek, and local Ollama out of the box; custom endpoints via `custom_base_url`.

---

## Installation

```bash
# npm
npm install @fusioncode/sdk

# pnpm
pnpm add @fusioncode/sdk

# bun
bun add @fusioncode/sdk
```

The package ships ESM only (`"type": "module"`) with TypeScript declarations. Node.js ≥ 18 is required (`engines` field). If you use the in-browser terminal adapter, install [`@xterm/xterm`](https://www.npmjs.com/package/@xterm/xterm) as an optional peer:

```bash
npm install @xterm/xterm        # ^5.0.0 or ^6.0.0
npm install @xterm/addon-fit    # optional, for auto-fitting
```

Subpath exports:

| Import path | Contents |
| :--- | :--- |
| `@fusioncode/sdk` | `FusionAgent`, WASM loader, xterm adapter, all types |
| `@fusioncode/sdk/wasm` | Low-level WASM loader bridge (`loadFusionWasm`, `initWasm`, …) |
| `@fusioncode/sdk/xterm` | `XtermAdapter` and terminal utilities |
| `@fusioncode/sdk/types` | Type-only definitions |

The Fusion CLI binary itself (needed for the stdio transport) is distributed separately — via [Homebrew](https://fusioncode.app) (`brew install fusion`), `cargo install fusion`, or a GitHub release download.

---

## Quick Start

### 1. WASM agent (browser & Node)

`FusionAgent.create()` auto-detects the runtime: **`wasm`** in browsers, **`stdio`** under Node.js. Force the in-memory WASM engine explicitly with `transport: 'wasm'`:

```typescript
import { FusionAgent } from '@fusioncode/sdk';

const agent = await FusionAgent.create({
  transport: 'wasm',
  default_provider: 'openrouter',
  default_model: 'anthropic/claude-3.5-sonnet',
  openrouter_api_key: process.env.OPENROUTER_API_KEY
});

// One-shot convenience: accumulates text deltas, returns the full response.
const response = await agent.promptTurn(
  'List the starter files, then read package.json and summarize the scripts.',
  (event) => {
    switch (event.type) {
      case 'status':
        console.log(`[status] ${event.message}`);
        break;
      case 'tool_started':
        console.log(`[tool] ${event.name}`, event.args);
        break;
      case 'tool_finished':
        console.log(`[tool] ${event.name} ok=${event.success} (${event.duration_ms ?? 0} ms)`);
        break;
      case 'text_delta':
        process.stdout.write(event.delta);
        break;
      case 'finished':
        console.log(`\n[done] tokens: ${event.usage?.total_tokens ?? 0}`);
        break;
      case 'error':
        console.error(`[error] ${event.message}`);
        break;
    }
  }
);

console.log('\nFinal response:\n', response);
```

Prefer manual stream control? `agent.prompt()` returns a standard `ReadableStream<AgentEvent>`:

```typescript
const stream = await agent.prompt('Write a Rust fibonacci function');
const reader = stream.getReader();
for (;;) {
  const { done, value } = await reader.read();
  if (done) break;
  if (value.type === 'text_delta') process.stdout.write(value.delta);
}
```

Cleanup when finished:

```typescript
await agent.close(); // ends the session and disconnects the transport
```

### 2. Stdio agent (Node CLI backend)

On Node.js the default transport spawns a `fusion --acp` child process and speaks
JSON-RPC 2.0 (Agent Client Protocol) over stdio — initialize handshake,
`session/new`, streamed `session/update` notifications, and `session/prompt`:

```typescript
import { FusionAgent } from '@fusioncode/sdk';

const agent = await FusionAgent.create({
  transport: 'stdio',          // default under Node.js; explicit here for clarity
  binaryPath: '/usr/local/bin/fusion', // default: 'fusion' (resolved from PATH)
  args: ['--acp'],             // default ACP server arguments
  cwd: '/path/to/project',     // workspace root handed to the agent
  env: { FUSION_LOG: 'debug' },// extra environment for the child process
  default_model: 'anthropic/claude-3.5-sonnet'
});

for await (const chunk of (await agent.prompt('Explain the module layout')).getReader() as any) {
  // (or consume via promptTurn as above)
}
```

> **Note** — `StdioTransport` requires `child_process` and therefore only works in
> Node.js / Bun / Deno. Browser bundles must use the WASM (or a WebSocket) transport.

---

## Streaming Events

Every turn dispatches a typed, discriminated-union event stream
(`FusionEvent`, re-exported as `AgentEvent` for adapter use):

| Event | Payload | Emitted when |
| :--- | :--- | :--- |
| `status` | `{ message }` | Lifecycle milestones (connecting, planning, cancelling, …) |
| `text_delta` | `{ delta }` | Assistant text chunks stream in |
| `thinking_delta` | `{ delta }` | Model reasoning/contemplation chunks stream in |
| `tool_started` | `{ id, name, args? }` | A tool call begins executing in the workspace |
| `tool_finished` | `{ id, name, success, output, duration_ms? }` | The tool call completes |
| `advisor_started` | `{ advisor, role }` | An advisor agent begins its review |
| `advisor_critique` | `{ advisor, approved, critique, suggestions? }` | An advisor returns its verdict |
| `finished` | `{ usage: { prompt_tokens, completion_tokens, total_tokens } }` | The turn completed successfully |
| `error` | `{ message, code? }` | An unrecoverable error occurred |

```typescript
import type { FusionEvent } from '@fusioncode/sdk';

const render = (event: FusionEvent) => {
  if (event.type === 'thinking_delta') {
    // Style reasoning traces differently from final text
    appendToPane('reasoning', event.delta);
  } else if (event.type === 'text_delta') {
    appendToPane('answer', event.delta);
  } else if (event.type === 'tool_started') {
    showToolSpinner(event.name, event.args);
  }
};

await agent.promptTurn('Refactor the config parser', render);
```

### Global subscription (transport-independent)

Instead of a per-turn callback, subscribe once and receive **every** event from
all turns:

```typescript
const unsubscribe = agent.subscribe((event) => {
  if (event.type === 'token_stats') updateMeter(event);
});

// ... later
unsubscribe();
```

### Async-iterator bridge (low-level WASM path)

`WasmEventBridge` wraps raw WASM callbacks (JSON strings, pointers, or objects)
into an `AsyncIterable<FusionEvent>` and normalizes everything into typed events:

```typescript
import { createWasmEventBridge } from '@fusioncode/sdk/wasm';

const bridge = createWasmEventBridge({ signal: controller.signal });

for await (const event of bridge) {
  if (event.type === 'text_delta') process.stdout.write(event.delta);
}
```

---

## Virtual File System (VFS)

The agent operates in an isolated, in-memory virtual workspace. Seed it before
turns, inspect it after, and persist it with checkpoints.

### Via `FusionAgent` (WASM transport)

```typescript
agent.fsWrite('src/app.ts', 'export const greeting = "Hello from Fusion!";\n');
agent.fsWrite('config.json', JSON.stringify({ theme: 'dark', port: 3000 }, null, 2));

const code = agent.fsRead('src/app.ts');
const files = agent.fsList();        // ['config.json', 'package.json', 'README.md', 'src/app.ts', ...]
const deleted = agent.fsDelete('config.json'); // true

// Richer access via the standalone bridge (grep / glob / edit / events / bash)
import { VirtualFileSystem } from '@fusioncode/sdk/wasm';

const vfs = new VirtualFileSystem('memory');           // or 'localstorage' in browsers
vfs.writeFile('src/main.rs', 'fn main() {}\n');
vfs.editFile('src/main.rs', 'fn main() {}', 'fn main() { println!("hi"); }');
vfs.grep('println', 'src/');                           // VfsGrepMatch[]
vfs.glob('src/**/*.rs');                               // ['src/main.rs']
vfs.executeVirtualBash('ls -la');                      // { success, output } — sandboxed simulation
vfs.onChange((e) => console.log('VFS changed:', e));   // live change events

// Two-way sync between the JS-side VFS and the WASM agent instance
const bindings = agent.getRawBindings();
if (bindings) {
  vfs.syncToAgent(bindings);            // push files into the agent
  const unsub = vfs.bindToAgent(bindings); // keep them in sync on every change
}
```

VFS operations require an **active WASM agent instance** — they throw on the
stdio transport, where the real filesystem of the spawned `fusion` process is
used instead.

---

## xterm.js Terminal Adapter

`XtermAdapter` turns any xterm.js terminal into a full Fusion console: REPL line
editing with history, ANSI themes, inline spinners, streaming token insertion,
and slash commands. Bind it to either the WASM engine or a remote WebSocket ACP
server.

```typescript
import { Terminal } from '@xterm/xterm';
import { FusionAgent, XtermAdapter } from '@fusioncode/sdk';
import { FitAddon } from '@xterm/addon-fit';
import '@xterm/xterm/css/xterm.css';

const term = new Terminal({
  cursorBlink: true,
  fontFamily: 'Fira Code, monospace',
  fontSize: 14,
  theme: { background: '#1a1b26', foreground: '#c0caf5', cursor: '#58a6ff' }
});
term.open(document.getElementById('terminal-container')!);

const fitAddon = new FitAddon();
term.loadAddon(fitAddon);

const agent = await FusionAgent.create({
  transport: 'wasm',
  default_model: 'anthropic/claude-3.5-sonnet'
});

const adapter = new XtermAdapter({
  terminal: term,
  fitAddon,
  autoFit: true,
  backend: agent.getTransport() as any,   // WasmTransport implements AgentBackend;
  model: 'anthropic/claude-3.5-sonnet',   // alternatively omit `backend` for the zero-config MockAgentBackend
  theme: 'tokyoNight',                    // preset name or Partial<TerminalTheme>
  welcomeBanner: true,
  enableSlashCommands: true,
  historyLimit: 200,
  historyStorageKey: 'fusion:history',    // persist history across reloads
  allowMultiline: true,                   // Shift+Enter / Ctrl+J continuation
  sanitizePaste: true
});
```

Adapter essentials:

```typescript
adapter.printBanner();                     // branded welcome box
adapter.write / writeln / clear / reset;   // raw terminal I/O
adapter.streamToken(delta, 'thinking');    // styled streaming insertion ('text' | 'thinking' | 'status')
adapter.startSpinner('Working...');        // inline spinner (dots|line|pulse|arrows|bounce|moon)
adapter.stopSpinner(true);
adapter.submitPrompt('Fix the failing test');  // programmatic turn submission
adapter.abortTurn();                       // cancel the running turn
adapter.setTheme('gruvbox');               // switch theme at runtime
adapter.stats;                             // Readonly<SessionStats> — tokens, turns, estimated cost
adapter.dispose();                         // detach + disconnect backend
```

### Pluggable backends

| Backend | Import | Use case |
| :--- | :--- | :--- |
| `WasmAgentBackend` | `@fusioncode/sdk/xterm` | Drive the adapter from a raw `WasmFusionAgent` instance |
| `WebSocketAgentBackend` | `@fusioncode/sdk/xterm` | Connect to a remote Fusion ACP daemon (`ws://127.0.0.1:3000/acp`, auto-reconnect) |
| `MockAgentBackend` | `@fusioncode/sdk/xterm` | Zero-config demo / offline UI testing (default when no backend given) |

```typescript
import { WebSocketAgentBackend } from '@fusioncode/sdk/xterm';

const adapter = new XtermAdapter({
  terminal: term,
  backend: new WebSocketAgentBackend('wss://fusion.example.com/acp')
});
```

### Slash commands

Built-ins (disable with `enableSlashCommands: false`):

| Command | Description |
| :--- | :--- |
| `/help` | Display available slash commands |
| `/clear` | Clear the terminal screen buffer |
| `/version` | Show Fusion harness and runtime version |
| `/theme [name]` | Switch theme (`deepOcean`, `cyberpunk`, `monokai`, `nord`, `gruvbox`, `tokyoNight`, `catppuccinMocha`, `dracula`, `githubDark`, `highContrastDark`) |
| `/model [id]` | View or switch the active model |
| `/cost` | Show session token usage and estimated cost |
| `/checkpoint` | Export a serialized session checkpoint |
| `/restore <json>` | Restore session state from checkpoint JSON |

Register your own:

```typescript
adapter.registerCommand({
  name: '/deploy',
  usage: '[env]',
  description: 'Kick off a deployment from the chat',
  aliases: ['/ship'],
  handler: async (args, adapter) => {
    adapter.writeln(`Deploying to ${args || 'staging'}...`);
    await adapter.submitPrompt(`Deploy the workspace to ${args || 'staging'}`);
  }
});
```

---

## Model Switching

Switch models per session or per turn:

```typescript
// Session-wide switch (updates config and notifies the backend)
await agent.switchModel('deepseek/deepseek-chat');
console.log(agent.getActiveModel());   // 'deepseek/deepseek-chat'

// Per-turn override without touching session state
await agent.prompt('Quick summary, please', {
  model: 'anthropic/claude-3.5-haiku',
  temperature: 0.7,
  maxTokens: 1024
});
```

Valid identifiers follow `provider/model` conventions, e.g.
`anthropic/claude-3.5-sonnet`, `openai/gpt-4o`, `deepseek/deepseek-chat`, or any
local Ollama tag. Build model-picker UIs from `ModelCatalogEntry` shapes
(`id`, `name`, `provider`, `category`, `context`, `pricing`, `description`).

---

## Sessions & Checkpoints

`checkpoint()` serializes the full session — conversation history, active model,
system prompt, token stats, config, and the entire VFS — into portable JSON.

```typescript
// Snapshot and persist (browser)
const snapshot = agent.checkpoint();              // JSON string
localStorage.setItem('fusion_session', snapshot);

// Structured access
const data = agent.getCheckpointData();
console.log(data.version, data.session.active_model, data.turn_counter, Object.keys(data.vfs.files));

// Restore later — same tab, another tab, or a fresh page
const agent2 = await FusionAgent.create({ transport: 'wasm' });
agent2.restore(localStorage.getItem('fusion_session')!);
console.log(agent2.getActiveModel(), agent2.fsList());
```

Checkpoint shape (`CheckpointData`):

```typescript
interface CheckpointData {
  version: string;                  // engine version
  session: {
    id: string;
    active_model: string;
    system_prompt?: string;
    messages: Message[];
    token_stats: TokenStats;
  };
  config: FusionConfig;
  vfs: { files: Record<string, string> };
  turn_counter: number;
}
```

Other session utilities:

```typescript
agent.getSessionId();      // UUID of the active session
agent.getMessages();       // Message[] — full conversation history
agent.getTokenStats();     // { prompt_tokens, completion_tokens, total_tokens }
agent.clearMessages();     // wipe history, keep VFS + config
agent.setSystemPrompt('You are a Rust refactor specialist.');
await agent.cancel();      // cancel the in-flight turn (stdio transport)
await agent.close();       // end session + disconnect transport
```

---

## Configuration Reference

`FusionAgent.create(options)` accepts `FusionAgentOptions` (a superset of `FusionConfig`):

```typescript
interface FusionConfig {
  /** Provider backend: 'openrouter' (default) | 'anthropic' | 'openai' | 'ollama' | 'custom' */
  default_provider?: ProviderType;
  /** Model identifier, e.g. 'anthropic/claude-3.5-sonnet', 'deepseek/deepseek-chat' */
  default_model?: string;
  /** Custom system prompt override */
  system_prompt?: string;
  /** Sampling temperature, 0.0–2.0 (default 0.2) */
  default_temperature?: number;
  /** Max generation tokens (default 4096) */
  max_tokens?: number;
  /** Provider API keys */
  openrouter_api_key?: string;
  anthropic_api_key?: string;
  openai_api_key?: string;
  /** Ollama base URL (default 'http://localhost:11434') */
  ollama_base_url?: string;
  /** Multi-agent advisor critiques: Architect, Security, Performance (default true) */
  advisors_enabled?: boolean;
  /** Custom/self-hosted LLM endpoint URL */
  custom_base_url?: string;
  /** Extra headers for custom endpoints */
  custom_headers?: Record<string, string>;
  /** Request timeout in milliseconds */
  timeout_ms?: number;
}

interface FusionAgentOptions extends FusionConfig {
  /** 'stdio' | 'wasm' | 'websocket' | custom AgentTransport — auto: 'wasm' in browser, 'stdio' in Node */
  transport?: 'stdio' | 'wasm' | 'websocket' | AgentTransport;
  binaryPath?: string;      // stdio: path to fusion binary (default 'fusion')
  args?: string[];          // stdio: child argv (default ['--acp'])
  cwd?: string;             // stdio: workspace root
  env?: Record<string, string>; // stdio: child env vars
  wsUrl?: string;           // websocket: remote ACP daemon URL
  wasmOptions?: WasmInitOptions; // wasm: { wasmUrl?, wasmBinary? }
  sessionId?: string;       // resume an existing session
}
```

Per-turn options (`PromptOptions`):

```typescript
interface PromptOptions {
  signal?: AbortSignal;     // cancel the turn
  model?: string;           // model override
  temperature?: number;     // temperature override
  maxTokens?: number;       // token limit override
  systemPrompt?: string;    // system prompt override
  tools?: ToolDefinition[]; // extra tools for this turn
  onEvent?: PromptTurnCallback;
}
```

---

## Type Reference

All types are exported from the root package and `@fusioncode/sdk/types`.

### Core

| Type | Kind | Summary |
| :--- | :--- | :--- |
| `FusionAgent` | class | High-level controller: prompts, streaming, VFS, checkpoints |
| `FusionAgentOptions` / `FusionConfig` | interface | Creation & runtime configuration (above) |
| `PromptOptions` | interface | Per-turn overrides (above) |
| `AgentTransport` | interface | `connect / disconnect / send / onMessage / promptStream / isConnected / type` — implement to plug custom transports |
| `StdioTransport` | class | JSON-RPC 2.0 / ACP child-process transport |
| `WasmTransport` | class | In-memory WASM bindings transport |
| `FusionEvent` | union | `status \| text_delta \| thinking_delta \| tool_started \| tool_finished \| advisor_started \| advisor_critique \| finished \| error` |
| `PromptTurnCallback` / `AgentEventCallback` | type | `(event) => void` streaming callbacks |

### Protocol (JSON-RPC 2.0 / ACP)

| Type | Summary |
| :--- | :--- |
| `JsonRpcRequest` / `JsonRpcResponse` / `JsonRpcNotification` / `JsonRpcError` | Envelope types |
| `JSON_RPC_ERROR_CODES` / `JsonRpcErrorCode` | Standard error-code constants |
| `InitializeRequest` / `InitializeResult`, `ClientCapabilities`, `ClientInfo`, `AgentInfo`, `AgentCapabilities` | ACP handshake |
| `NewSessionRequest` / `NewSessionResult`, `LoadSessionRequest` / `LoadSessionResult`, `ListSessionsRequest` / `ListSessionsResult`, `SessionSummaryItem`, `CloseSessionRequest`, `CancelSessionRequest` | Session lifecycle |
| `PromptRequest` / `PromptResponse`, `PromptInput`, `ContentBlock`, `ContentType`, `StopReason`, `TokenStatsInfo` | Prompt dispatch & results |
| `SessionUpdate` / `SessionUpdateKind` / `SessionUpdateParams` | Streamed `session/update` notifications (`agent_message_chunk`, `agent_thought_chunk`, `tool_call`, `tool_call_result`, `advisor_started`, `advisor_critique`, `token_stats`, `status`, `plan`, `subagent_update`) |
| `ModelInfo` | Model descriptor advertised by the agent |

### Agents, tools & state

| Type | Summary |
| :--- | :--- |
| `ProviderType` | `'openrouter' \| 'anthropic' \| 'openai' \| 'ollama' \| 'custom'` (open set) |
| `Message` / `MessageRole` | Conversation history entries (`system \| user \| assistant \| tool`) |
| `ToolDefinition` / `ToolInfo` / `ToolCall` / `ToolResult` / `ToolParameterSchema` / `ToolParameterProperty` | Tool registry & JSON-Schema shapes |
| `TokenStats` / `SessionStats` | Usage counters |
| `VirtualFile` / `VirtualFileSystemState` | VFS snapshots |
| `SessionState` | In-memory session representation |
| `CheckpointData` | Checkpoint shape (above) |
| `SubagentRole` / `SubagentStatus` / `SubagentProgress` | Subagent lifecycle |
| `AgentRole` / `AgentStatus` / `MeshAgentInfo` / `MeshTopic` / `BroadcastMessage` / `DirectMessage` / `PeerQuery` / `PeerResponse` | Agent-mesh pub-sub & RPC |
| `AdvisorCritique` / `AdvisorReviewRequest` / `AdvisorReviewResponse` / `RiskLevel` | Advisor review pipeline |
| `ModelCatalogEntry` | Model-picker metadata |
| `WasmFusionAgentBindings` | Raw wasm-bindgen surface (`prompt_turn`, `fs_*`, `checkpoint`, `restore`, …) |
| `WasmInitOptions` / `WasmSourceInput` | WASM loading configuration |

### Terminal adapter

| Type | Summary |
| :--- | :--- |
| `XtermAdapter` / `createXtermAdapter` / `FusionTerminalAdapter` | Main adapter class & factory |
| `XtermAdapterOptions` | Full option set (terminal, container, fitAddon, backend, theme, history, slash commands, …) |
| `AgentBackend` | Adapter-side backend contract: `promptTurn(prompt, onEvent, signal?)`, optional `connect/disconnect/cancelTurn/resizeTerminal/checkpoint/restore` |
| `WasmAgentBackend` / `WebSocketAgentBackend` / `MockAgentBackend` | Built-in backends |
| `SlashCommand` | `{ name, description, usage?, aliases?, handler(args, adapter) }` |
| `SessionStats` / `TerminalTheme` / `SpinnerStyle` / `SpinnerOptions` / `StreamTokenOptions` | Stats, theming, spinner, streaming |
| `XtermInstance` / `FitAddonInstance` | Structural xterm.js interfaces (v5/v6 compatible) |
| `AnsiParser` / `AnsiFormatter` / `KeyEncoder` / `ANSI` / `THEMES` / `TerminalSpinner` | Formatting, keys, and styling utilities |

### WASM bridge

| Export | Summary |
| :--- | :--- |
| `loadFusionWasm(source?)` | Cross-environment loader — accepts a URL/path, base64 data URI, `ArrayBuffer`/`Uint8Array`, `Response`, or pre-compiled `WebAssembly.Module`; auto-detects runtime and falls back to a pure-JS in-memory engine |
| `initWasm(options?)` | Backward-compatible alias |
| `getWasmModule()` / `isWasmInitialized()` / `resetWasmModule()` | Cached module access |
| `VirtualFileSystem` | Standalone VFS with `readFile`, `writeFile`, `editFile`, `grep`, `glob`, `listFiles`, `stat`, `copyFile`, `moveFile`, `deleteFile`, `exists`, `exportJson`, `importJson`, `onChange`, `executeVirtualBash`, `syncToAgent` / `syncFromAgent` / `bindToAgent` |
| `InMemoryStorageBackend` / `LocalStorageBackend` / `VfsStorageBackend` | Pluggable VFS storage |
| `WasmEventBridge` / `createWasmEventBridge` | Callback → `AsyncIterable<FusionEvent>` normalization with `AbortSignal` support |
| `Subscription` / `Observer<T>` | Minimal observable primitives |

---

## Cross-Environment Notes (Node vs Browser)

| Capability | Node.js ≥ 18 / Bun / Deno | Browser |
| :--- | :--- | :--- |
| Default transport (via `FusionAgent.create()`) | `stdio` (`fusion --acp` child process) | `wasm` (in-memory) |
| WASM transport | ✔ (auto-falls back to pure-JS engine if no bundle found) | ✔ |
| `StdioTransport` | ✔ (`child_process`) | ✖ throws — use WASM/WebSocket |
| `WebSocketAgentBackend` | ✔ | ✔ |
| Standalone `VirtualFileSystem` with `'localstorage'` backend | falls back to memory | ✔ |
| Adapter history persistence (`historyStorageKey`) | memory only | ✔ (`localStorage`) |
| WASM loading | `node:fs/promises` read + `WebAssembly.instantiate` | `fetch` + `WebAssembly.instantiateStreaming` |

Practical guidance:

- **Bundlers (Vite/webpack/esbuild):** import from the root package; the stdio path uses a dynamic `import('child_process')` so browser bundles never pull Node builtins. If your bundler still tries to resolve it, alias `child_process` to an empty module for browser builds.
- **Custom WASM bundles:** pass `wasmOptions: { wasmUrl: new URL('/wasm/fusion.wasm', import.meta.url) }` or a pre-fetched `wasmBinary`/`Response` to `FusionAgent.create()` — useful for strict CSPs or pinned assets.
- **`process.env` in browsers:** guard API-key reads (`typeof process !== 'undefined' && process.env.KEY`) or inject keys at build time; never ship secrets in client bundles — proxy through the WebSocket backend instead.
- **SSR (Next.js/Nuxt/SvelteKit):** the adapter touches `document`/`localStorage`; load terminal components client-only (`dynamic(..., { ssr: false })` in Next.js).

---

## Framework Integration

### React / Vite

```tsx
import React, { useEffect, useRef } from 'react';
import { Terminal } from '@xterm/xterm';
import { FusionAgent, XtermAdapter } from '@fusioncode/sdk';
import '@xterm/xterm/css/xterm.css';

export function FusionTerminalComponent() {
  const containerRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    if (!containerRef.current) return;

    const term = new Terminal({ cursorBlink: true });
    term.open(containerRef.current);

    let adapter: XtermAdapter | undefined;
    let cancelled = false;

    FusionAgent.create({ transport: 'wasm', default_model: 'anthropic/claude-3.5-sonnet' })
      .then((agent) => {
        if (cancelled) return;
        adapter = new XtermAdapter({ terminal: term, agent: agent as never });
      });

    return () => {
      cancelled = true;
      adapter?.dispose();
      term.dispose();
    };
  }, []);

  return <div ref={containerRef} style={{ width: '100%', height: '500px' }} />;
}
```

### Next.js (App Router, client-only)

```tsx
'use client';

import dynamic from 'next/dynamic';

const FusionTerminal = dynamic(
  () => import('./FusionTerminalComponent').then((m) => m.FusionTerminalComponent),
  { ssr: false }
);

export default function Page() {
  return (
    <main>
      <h1>Fusion In-Browser Assistant</h1>
      <FusionTerminal />
    </main>
  );
}
```

---

## License

Licensed under [MIT](LICENSE).
