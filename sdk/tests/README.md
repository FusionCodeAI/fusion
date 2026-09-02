# SDK Tests

Tests for the TypeScript SDK (`sdk/src`). These exercise the real `FusionAgent`
client by driving a mock ACP child-process transport over fake stdio — no
actual `fusion` binary or network access is required.

## Running

From the repository root:

```sh
npm --prefix sdk install          # installs typescript + @types/node
npm --prefix sdk run test         # or: node --test sdk/tests/*.test.ts
```

`agent.test.ts` uses Node's built-in test runner (`node:test`, Node >= 18)
and TypeScript type stripping (`--experimental-strip-types`), which is
enabled by default on Node 22.6+/23. On older Node versions, run the
suite through `tsx`:

```sh
npx tsx --test sdk/tests/agent.test.ts
```

## Files

| File            | Covers                                                                    |
| --------------- | ------------------------------------------------------------------------- |
| `agent.test.ts` | `FusionAgent` end-to-end over a mock stdio transport: initialize handshake, prompt streaming (text / thinking / tool events), cancel, `switchModel`, `listTools`, `close`, and error handling. |

## How the mock transport works

`MockTransport` implements the SDK's `AgentTransport` interface in-memory:

- `connect()` resolves after a simulated handshake (initialize → session/new).
- `promptStream()` returns a `ReadableStream<AgentEvent>` fed by scripted
  events (text deltas, thinking deltas, tool start/finish, usage) or errors.
- Writes are recorded so tests can assert exactly what `FusionAgent` sent.
- An `error` hook lets tests inject failures (connect errors, prompt errors,
  disconnect errors) without real processes.
- A simulated `session/update` notification channel exercises the same
  `SessionUpdate` → `AgentEvent` mapping the real `StdioTransport` uses
  (`mapSessionUpdateToAgentEvents` semantics).

## Scope

These tests are transport-level and intentionally avoid the WebAssembly
transport (`WasmTransport`), which requires the compiled `fusion.wasm`
artifact and is covered separately by the Rust integration tests
(`tests/wasm_test.rs`).