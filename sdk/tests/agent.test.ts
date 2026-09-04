#!/usr/bin/env -S node --experimental-strip-types --test
/**
 * FusionAgent integration tests — real SDK code over a scripted fake ACP
 * process. hooks.mjs intercepts the `child_process` module that
 * StdioTransport.connect() dynamically imports; each test drives the real
 * FusionAgent against the fake server with deterministic awaits.
 *
 * Covered: initialize handshake, prompt streaming (text_delta,
 * thinking_delta, tool_started, tool_finished, finished), cancel,
 * switchModel, listTools, close, JSON-RPC errors, and spawn failure.
 */
import { register } from 'node:module';
import { fusionMockState } from './hooks.mjs';
import { describe, it, beforeEach } from 'node:test';
import assert from 'node:assert/strict';

// ---------------------------------------------------------------------------
// 1. child_process interception — hooks.mjs resolves the bare specifier
//    `child_process` (the one StdioTransport.connect() awaits) to an
//    in-memory fake ACP server.
// ---------------------------------------------------------------------------

try {
  register(new URL('./hooks.mjs', import.meta.url).href, import.meta.url);
} catch {}

const g = globalThis as unknown as Record<string, unknown>;
if (!g.__fusionMock) {
  g.__fusionMock = fusionMockState;
}
if (!g.__fusionHooksState) {
  g.__fusionHooksState = fusionMockState;
}
const mockState = (g.__fusionMock || fusionMockState) as typeof fusionMockState;

interface AcpMessage {
  id?: number | string;
  method?: string;
  params?: Record<string, unknown>;
}

/** Await the fake server receiving a request with the given method. */
function awaitedRequest(srv: typeof mockState.current, method: string): Promise<AcpMessage> {
  return new Promise((resolve) => srv.once('request:' + method, resolve));
}

/** Drain a ReadableStream to its end. */
async function readAll(stream: ReadableStream<unknown>): Promise<unknown[]> {
  const events: unknown[] = [];
  const reader = stream.getReader();
  for (;;) {
    const { done, value } = await reader.read();
    if (done) break;
    events.push(value);
  }
  return events;
}

/** Build a FusionAgent wired to the fake stdio transport. */
async function makeAgent(opts: Record<string, unknown> = {}) {
  const sdk = await import('../src/agent.js');
  const agent = new sdk.FusionAgent({
    transport: 'stdio',
    binaryPath: 'fusion-fake',
    args: ['--acp'],
    cwd: '/tmp/fusion-test',
    ...opts
  } as never);
  return agent as {
    initialize(): Promise<void>;
    prompt(text: string, opts?: Record<string, unknown>): Promise<ReadableStream<unknown>>;
    promptTurn(text: string): Promise<string>;
    cancel(): Promise<void>;
    switchModel(model: string): Promise<void>;
    listTools(): Promise<Array<{ name: string; description: string; parameters: { type: string; required: string[] } }>>;
    close(): Promise<void>;
    getSessionId(): string;
    getActiveModel(): string;
    subscribe(cb: (e: unknown) => void): () => void;
    getTransport(): { isConnected: boolean };
  };
}

/** Agent + fresh fake server, fully handshaken. */
async function makeHandshakenAgent(opts: Record<string, unknown> = {}) {
  const agent = await makeAgent(opts);
  const initDone = awaitedRequest(mockState.current, 'initialize');
  const sessionDone = awaitedRequest(mockState.current, 'session/new');
  const handshake = agent.initialize();
  await Promise.all([initDone, sessionDone]);
  await handshake;
  return agent;
}

beforeEach(() => {
  // Fresh fake server state per test.
  mockState.servers.length = 0;
  mockState.createServerCalls.length = 0;
  mockState.current = null;
});

// ---------------------------------------------------------------------------
// 2. Initialize handshake
// ---------------------------------------------------------------------------

describe('FusionAgent.initialize — ACP handshake', () => {
  it('spawns the fake fusion binary and completes initialize → initialized → session/new', async () => {
    const agent = await makeAgent();
    const initializeRequest = awaitedRequest(mockState.current, 'initialize');
    const newSessionRequest = awaitedRequest(mockState.current, 'session/new');
    const handshake = agent.initialize();

    const initMsg = await initializeRequest;
    await newSessionRequest;
    await handshake;

    // One spawn of the fake binary with ACP args.
    assert.equal(mockState.createServerCalls.length, 1);
    const spawnCall = mockState.createServerCalls[0];
    assert.equal(spawnCall.command, 'fusion-fake');
    assert.deepEqual(spawnCall.args, ['--acp']);

    // initialize request payload.
    assert.equal(initMsg.method, 'initialize');
    assert.equal(initMsg.params?.protocolVersion, 1);
    const clientInfo = initMsg.params?.clientInfo as { name: string };
    assert.equal(clientInfo.name, '@fusioncode/sdk');

    // Session adopted from session/new result.
    assert.equal(agent.getSessionId(), 'sess-mock-001');
  });

  it('is idempotent — a second initialize does not spawn another process', async () => {
    const agent = await makeAgent();
    const initializeRequest = awaitedRequest(mockState.current, 'initialize');
    const newSessionRequest = awaitedRequest(mockState.current, 'session/new');
    const handshake = agent.initialize();
    await Promise.all([initializeRequest, newSessionRequest]);
    await handshake;

    await agent.initialize();
    assert.equal(mockState.createServerCalls.length, 1);
  });

  it('marks the transport connected and adopts the default model from session/new', async () => {
    const agent = await makeAgent();
    const initializeRequest = awaitedRequest(mockState.current, 'initialize');
    const newSessionRequest = awaitedRequest(mockState.current, 'session/new');
    const handshake = agent.initialize();
    await Promise.all([initializeRequest, newSessionRequest]);
    await handshake;

    assert.equal(agent.getTransport().isConnected, true);
    assert.equal(agent.getActiveModel(), 'anthropic/claude-3.5-sonnet');
  });
});

// ---------------------------------------------------------------------------
// 3. Prompt streaming
// ---------------------------------------------------------------------------

describe('FusionAgent.prompt — streaming', () => {
  it('streams text_delta → thinking_delta → tool events → finished in order', async () => {
    const agent = await makeHandshakenAgent();
    const srv = mockState.current;

    srv.updates = [
      { kind: 'agent_message_chunk', content: { role: 'assistant', content: [{ text: 'Hello' }] } },
      { kind: 'agent_message_chunk', content: { role: 'assistant', content: [{ text: ', world' }] } },
      { kind: 'agent_thought_chunk', thought: 'planning…' },
      { kind: 'tool_call', callId: 'call_1', name: 'read', args: { path: 'src/main.rs' } },
      { kind: 'tool_call_result', callId: 'call_1', name: 'read', output: 'fn main() {}', success: true, duration_ms: 5 }
    ];

    const promptDone = awaitedRequest(srv, 'session/prompt');
    const stream = await agent.prompt('Write a fibonacci function');
    await promptDone;

    const events = await readAll(stream);
    const types = events.map((e) => (e as { type: string }).type);
    assert.deepEqual(types, [
      'text_delta', 'text_delta', 'thinking_delta', 'tool_started', 'tool_finished', 'finished'
    ]);

    assert.equal(events[0].delta, 'Hello');
    assert.equal(events[1].delta, ', world');
    assert.equal(events[2].delta, 'planning…');
    assert.deepEqual(events[3], { type: 'tool_started', id: 'call_1', name: 'read', args: { path: 'src/main.rs' } });
    assert.deepEqual(events[4], {
      type: 'tool_finished', id: 'call_1', name: 'read', success: true, output: 'fn main() {}', duration_ms: 5
    });
    assert.deepEqual(events[5].usage, { prompt_tokens: 12, completion_tokens: 34, total_tokens: 46 });
  });

  it('writes the prompt text to the transport as a session/prompt request', async () => {
    const agent = await makeHandshakenAgent();
    const srv = mockState.current;

    const promptDone = awaitedRequest(srv, 'session/prompt');
    const stream = await agent.prompt('echo me');
    await promptDone;
    await readAll(stream);

    const lines = srv.written.map((l) => JSON.parse(l));
    const req = lines.find((m) => m.method === 'session/prompt');
    assert.ok(req, 'session/prompt request written to transport');
    assert.equal(req.params.prompt, 'echo me');
    assert.equal(req.params.sessionId, 'sess-mock-001');
  });

  it('invokes onEvent for every streamed event', async () => {
    const agent = await makeHandshakenAgent();
    const srv = mockState.current;

    srv.updates = [{ kind: 'agent_message_chunk', content: 'chunk-one' }];

    const seen: unknown[] = [];
    const promptDone = awaitedRequest(srv, 'session/prompt');
    const stream = await agent.prompt('ping', {
      onEvent: (e: unknown) => seen.push(e)
    });
    await promptDone;
    await readAll(stream);

    assert.deepEqual(
      seen.map((e) => (e as { type: string }).type),
      ['text_delta', 'finished']
    );
  });

  it('accumulates text deltas in promptTurn and returns the full response', async () => {
    const agent = await makeHandshakenAgent();
    const srv = mockState.current;

    srv.updates = [
      { kind: 'agent_message_chunk', content: { role: 'assistant', content: [{ text: 'Rust ' }] } },
      { kind: 'agent_message_chunk', content: { role: 'assistant', content: [{ text: 'rocks' }] } },
      { kind: 'agent_thought_chunk', thought: 'not part of the answer' }
    ];

    const promptDone = awaitedRequest(srv, 'session/prompt');
    const turn = agent.promptTurn('say it');
    await promptDone;

    assert.equal(await turn, 'Rust rocks');
  });

  it('delivers events to subscribed global listeners', async () => {
    const agent = await makeHandshakenAgent();
    const srv = mockState.current;

    srv.updates = [{ kind: 'agent_message_chunk', content: 'sub-test' }];

    const seen: unknown[] = [];
    const unsubscribe = agent.subscribe((e: unknown) => seen.push(e));
    const promptDone = awaitedRequest(srv, 'session/prompt');
    const stream = await agent.prompt('broadcast');
    await promptDone;
    await readAll(stream);
    unsubscribe();

    assert.ok(seen.length >= 2, 'listener saw text + finished events');
  });
});

// ---------------------------------------------------------------------------
// 4. Cancel
// ---------------------------------------------------------------------------

describe('FusionAgent.cancel', () => {
  it('sends session/cancel for the active session', async () => {
    const agent = await makeHandshakenAgent();
    const srv = mockState.current;

    const cancelDone = awaitedRequest(srv, 'session/cancel');
    const cancel = agent.cancel();
    await cancelDone;
    await cancel;

    const lines = srv.written.map((l) => JSON.parse(l));
    const req = lines.find((m) => m.method === 'session/cancel');
    assert.ok(req, 'session/cancel written');
    assert.equal(req.params.sessionId, 'sess-mock-001');
  });

  it('is a no-op before initialize — no process spawned', async () => {
    const agent = await makeAgent();
    await agent.cancel();
    assert.equal(mockState.createServerCalls.length, 0);
  });
});

// ---------------------------------------------------------------------------
// 5. switchModel
// ---------------------------------------------------------------------------

describe('FusionAgent.switchModel', () => {
  it('switches the active model on the stdio transport', async () => {
    const agent = await makeHandshakenAgent();
    await agent.switchModel('deepseek/deepseek-chat');
    assert.equal(agent.getActiveModel(), 'deepseek/deepseek-chat');
  });

  it('model override in the session/new request selects the requested model', async () => {
    const agent = await makeAgent({ default_model: 'deepseek/deepseek-chat' });
    const initializeRequest = awaitedRequest(mockState.current, 'initialize');
    const newSessionRequest = awaitedRequest(mockState.current, 'session/new');
    const handshake = agent.initialize();
    await Promise.all([initializeRequest, newSessionRequest]);
    await handshake;

    // hooks.mjs echoes the requested model as the default in its catalog.
    assert.equal(agent.getActiveModel(), 'deepseek/deepseek-chat');
  });
});

// ---------------------------------------------------------------------------
// 6. listTools
// ---------------------------------------------------------------------------

describe('FusionAgent.listTools', () => {
  it('returns the workspace tool catalog without spawning a process', async () => {
    const agent = await makeAgent();
    const tools = await agent.listTools();

    assert.deepEqual(
      tools.map((t) => t.name),
      ['read', 'write', 'edit', 'grep', 'glob', 'bash']
    );
    assert.equal(mockState.createServerCalls.length, 0);

    for (const tool of tools) {
      assert.equal(typeof tool.description, 'string');
      assert.ok(tool.parameters && tool.parameters.type === 'object');
      assert.ok(Array.isArray(tool.parameters.required));
    }

    const write = tools.find((t) => t.name === 'write');
    assert.ok(write?.parameters?.required?.includes('path'));
    assert.ok(write?.parameters?.required?.includes('content'));
  });
});

// ---------------------------------------------------------------------------
// 7. close
// ---------------------------------------------------------------------------

describe('FusionAgent.close', () => {
  it('sends session/close, kills the fake process, and disconnects', async () => {
    const agent = await makeHandshakenAgent();
    const srv = mockState.current;

    const closeDone = awaitedRequest(srv, 'session/close');
    const closing = agent.close();
    await closeDone;
    await closing;

    const lines = srv.written.map((l) => JSON.parse(l));
    assert.ok(lines.some((m) => m.method === 'session/close'), 'session/close written');
    assert.equal(srv.killed, true, 'fake process killed');
    assert.equal(agent.getTransport().isConnected, false);
  });

  it('close before initialize does not spawn a process', async () => {
    const agent = await makeAgent();
    await agent.close();
    assert.equal(mockState.createServerCalls.length, 0);
  });
});

// ---------------------------------------------------------------------------
// 8. Error handling
// ---------------------------------------------------------------------------

describe('FusionAgent error handling', () => {
  it('initialize rejects when the fake server returns a JSON-RPC error', async () => {
    const agent = await makeAgent();
    const srv = mockState.current;

    srv.behavior.set('initialize', (msg: AcpMessage) => srv.rpcError(msg.id, -32000, 'handshake refused by mock'));
    const initializeRequest = awaitedRequest(srv, 'initialize');
    const handshake = agent.initialize();
    await initializeRequest;

    await assert.rejects(handshake, /handshake refused by mock/);
  });

  it('prompt yields an error event when the server rejects session/prompt', async () => {
    const agent = await makeHandshakenAgent();
    const srv = mockState.current;

    srv.behavior.set('session/prompt', (msg: AcpMessage) => srv.rpcError(msg.id, -32001, 'model overloaded (mock)'));

    const promptDone = awaitedRequest(srv, 'session/prompt');
    const stream = await agent.prompt('trigger error');
    await promptDone;

    const events = await readAll(stream);
    assert.equal(events.length, 1);
    const first = events[0] as { type: string; message: string };
    assert.equal(first.type, 'error');
    assert.match(first.message, /model overloaded/);
  });

  it('rejects the stream reader when the fake process crashes mid-request', async () => {
    const agent = await makeHandshakenAgent();
    const srv = mockState.current;

    srv.behavior.set('session/prompt', () => {
      srv.crash(137);
    });

    const promptDone = awaitedRequest(srv, 'session/prompt');
    const stream = await agent.prompt('crash me');
    await promptDone;

    await assert.rejects(readAll(stream), /exited with code 137|process exited/);
  });

  it('rejects the stream reader on a stdio-level process error', async () => {
    const agent = await makeHandshakenAgent();
    const srv = mockState.current;

    srv.behavior.set('session/prompt', () => {
      srv.stdioError(new Error('ECONNRESET (mock stdio)'));
    });

    const promptDone = awaitedRequest(srv, 'session/prompt');
    const stream = await agent.prompt('stdio failure');
    await promptDone;

    await assert.rejects(readAll(stream), /process error|ECONNRESET/);
  });

  it('pre-aborted signal fails the prompt before execution', async () => {
    const agent = await makeHandshakenAgent();

    const controller = new AbortController();
    controller.abort();

    const stream = await agent.prompt('never runs', { signal: controller.signal });
    await assert.rejects(readAll(stream), /aborted before execution/);
  });

  it('non-JSON stdout lines are ignored without breaking the transport', async () => {
    const agent = await makeHandshakenAgent();
    const srv = mockState.current;

    srv.behavior.set('session/prompt', (msg: AcpMessage) => {
      srv.send('WARN: fusion is chatty (not JSON)');
      srv.send('');
      srv.send(JSON.stringify({
        jsonrpc: '2.0', method: 'session/update',
        params: { sessionId: 'sess-mock-001', update: { kind: 'agent_message_chunk', content: { role: 'assistant', content: [{ text: 'still alive' }] } } }
      }));
      srv.respond(msg.id, { stopReason: 'end_turn', stats: { promptTokens: 1, completionTokens: 2, totalTokens: 3 } });
    });

    const promptDone = awaitedRequest(srv, 'session/prompt');
    const stream = await agent.prompt('chatty');
    await promptDone;

    const events = await readAll(stream);
    assert.deepEqual(events.map((e) => (e as { type: string }).type), ['text_delta', 'finished']);
    assert.equal(events[0].delta, 'still alive');
  });

  it('spawn failure (ENOENT-equivalent from the fake) surfaces as a spawn error', async () => {
    const agent = await makeAgent();
    const srv = mockState.current;
    srv.behavior.set('initialize', () => srv.stdioError(new Error('spawn fusion-fake ENOENT (mock)')));

    const initializeRequest = awaitedRequest(srv, 'initialize');
    const handshake = agent.initialize();
    await initializeRequest;

    await assert.rejects(handshake, /process error|ENOENT/);
  });
});
