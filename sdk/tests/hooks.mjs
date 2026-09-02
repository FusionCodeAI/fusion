/**
 * child_process interception hooks for the FusionAgent test suite.
 *
 * Loaded via `register()` from agent.test.ts. Two cooperating halves:
 *
 * 1. `resolve`/`load` hooks map the bare specifier `child_process` (the one
 *    StdioTransport.connect() dynamically imports) to an in-memory fake
 *    module whose `spawn` creates a scripted fake ACP server.
 * 2. The hooks module graph has its own `globalThis` (separate from the
 *    main/test graph), so shared state is passed via the register() `data`
 *    option: hooks.mjs receives `state` (its FakeAcpServer factory and
 *    rendezvous object) and the fake `spawn` served from FAKE_CP_SOURCE
 *    closes over it. agent.test.ts holds the same `state` reference.
 *
 * The fake server mimics a `fusion --acp` process: JSON-RPC 2.0 over stdio
 * with newline framing. It answers initialize, session/new, session/cancel,
 * and session/close out of the box, streams scripted `session/update`
 * notifications before the session/prompt result, and exposes per-method
 * behavior overrides so tests can re-script any method (JSON-RPC errors,
 * crashes, stdio failures, never-answering prompts).
 */
import { EventEmitter } from 'node:events';

/**
 * In-memory source for the fake `child_process` module. The register() data
 * option is re-exposed on globalThis inside this module graph so the served
 * source can read the shared state.
 */
const FAKE_CP_SOURCE = `
const g = globalThis;
export function spawn(command, args, opts) {
  return g.__fusionHooksState.createServer(command, args, opts);
}
export default { spawn };
`;
const FAKE_CP_URL = 'fusion-mock:child_process';

/** Stdio stream stand-in: setEncoding is a no-op, on('data') works. */
class FakeStdio extends EventEmitter {
  setEncoding() {}
}

/**
 * Scripted fake `fusion --acp` server. Doubles as the child process handle
 * (stdin/stdout/stderr/on/kill) that StdioTransport stores.
 */
class FakeAcpServer extends EventEmitter {
  /** Lines written by the transport, in order (newline-framed JSON). */
  written = [];
  /** Session updates streamed before the session/prompt result. */
  updates = [];
  /** Per-method overrides; tests set these to re-script responses. */
  behavior = new Map();
  killed = false;
  exited = false;
  stdinClosed = false;

  command = '';
  args = [];
  opts = undefined;

  stdin = {
    write: (data, _encoding, callback) => {
      this.ingest(String(data));
      if (callback) callback(null);
      return true;
    },
    end: () => {
      this.stdinClosed = true;
    }
  };

  stdout = new FakeStdio();
  stderr = new FakeStdio();

  constructor(command, args, opts) {
    super();
    this.command = command;
    this.args = args;
    this.opts = opts;
  }

  kill(signal) {
    this.killed = true;
    if (!this.exited) {
      this.exited = true;
      this.emit('exit', null, signal ?? 'SIGTERM');
    }
    return true;
  }

  /** Emit a fake stdout line from the "process" to the transport. */
  send(line) {
    this.stdout.emit('data', line.endsWith('\n') ? line : line + '\n');
  }

  respond(id, value) {
    this.send(JSON.stringify({ jsonrpc: '2.0', id, result: value }));
  }

  rpcError(id, code, message) {
    this.send(JSON.stringify({ jsonrpc: '2.0', id, error: { code, message } }));
  }

  /** Simulate abrupt process death with the given exit code. */
  crash(code = 137) {
    if (!this.exited) {
      this.exited = true;
      this.emit('exit', code, null);
    }
  }

  /** Simulate a stdio-level process error (ECONNRESET, spawn race, …). */
  stdioError(err) {
    this.emit('error', err instanceof Error ? err : new Error(String(err)));
  }

  /**
   * Route an incoming newline-framed line from the transport: record it,
   * emit request/request:<method> events for deterministic test awaits,
   * then dispatch to the scripted behavior.
   */
  ingest(data) {
    for (const line of data.split('\n')) {
      if (!line.trim()) continue;
      this.written.push(line);
      let msg;
      try {
        msg = JSON.parse(line);
      } catch {
        continue;
      }
      if (msg.method === undefined) continue;
      this.emit('request', msg);
      this.emit('request:' + msg.method, msg);
      this.dispatch(msg);
    }
  }

  dispatch(msg) {
    const override = this.behavior.get(msg.method);
    if (override) {
      override(msg);
      return;
    }

    switch (msg.method) {
      case 'initialize':
        this.respond(msg.id, {
          protocolVersion: 1,
          agentInfo: { name: 'fusion-mock', version: '0.3.0' },
          agentCapabilities: { loadSession: true, prompt: true, cancel: true }
        });
        break;

      case 'session/new': {
        const requested = msg.params?.model;
        const models = requested
          ? [
              { id: requested, isDefault: true },
              { id: 'anthropic/claude-3.5-sonnet', isDefault: false }
            ]
          : [
              { id: 'anthropic/claude-3.5-sonnet', isDefault: true },
              { id: 'deepseek/deepseek-chat', isDefault: false }
            ];
        this.respond(msg.id, { sessionId: 'sess-mock-001', models });
        break;
      }

      case 'session/prompt': {
        for (const update of this.updates) {
          this.send(
            JSON.stringify({
              jsonrpc: '2.0',
              method: 'session/update',
              params: { sessionId: 'sess-mock-001', update }
            })
          );
        }
        this.respond(msg.id, {
          stopReason: 'end_turn',
          stats: { promptTokens: 12, completionTokens: 34, totalTokens: 46 }
        });
        break;
      }

      case 'session/cancel':
        this.respond(msg.id, { stopReason: 'cancelled' });
        break;

      case 'session/close':
        this.respond(msg.id, {});
        break;

      default:
        this.respond(msg.id, {});
        break;
    }
  }
}

/**
 * Shared rendezvous. register() passes this object as `data`; the hooks
 * graph exposes it on its own globalThis for FAKE_CP_SOURCE to reach, and
 * agent.test.ts holds the same reference.
 */
export const fusionMockState = {
  servers: [],
  current: null,
  createServerCalls: [],
  createServer(command, args, opts) {
    const server = new FakeAcpServer(command, args, opts);
    fusionMockState.servers.push(server);
    fusionMockState.current = server;
    fusionMockState.createServerCalls.push({ command, args, opts });
    return server;
  }
};

export async function initialize(data) {
  const g = /** @type {any} */ (globalThis);
  g.__fusionHooksState = data ?? fusionMockState;
}

export async function resolve(specifier, context, nextResolve) {
  if (specifier === 'child_process') {
    return { url: FAKE_CP_URL, shortCircuit: true };
  }
  return nextResolve(specifier, context);
}

export async function load(url, context, nextLoad) {
  if (url === FAKE_CP_URL) {
    return { format: 'module', source: FAKE_CP_SOURCE, shortCircuit: true };
  }
  return nextLoad(url, context);
}
