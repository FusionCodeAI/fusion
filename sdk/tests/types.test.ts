/**
 * Type & shape tests for sdk/src/types.ts
 *
 * Validates the JSON-RPC 2.0 protocol envelope shapes, agent-mesh message
 * typing, session-state serialization round-trips, and model info parsing.
 *
 * Run:
 *   bun test tests/types.test.ts
 *   # or, after `npm run build`:
 *   node --test tests/types.test.js
 */

import test from 'node:test';
import assert from 'node:assert/strict';

import {
  ACP_PROTOCOL_VERSION,
  JSON_RPC_ERROR_CODES,
  type BroadcastMessage,
  type BroadcastPayload,
  type DirectMessage,
  type JsonRpcError,
  type JsonRpcNotification,
  type JsonRpcRequest,
  type JsonRpcResponse,
  type LoadSessionResult,
  type MeshAgentInfo,
  type Message,
  type ModelInfo,
  type NewSessionResult,
  type PeerQuery,
  type PeerResponse,
  type SessionState,
  type TokenStats
} from '../src/types.js';

// ============================================================================
// Shape-validation helpers (type guards for untrusted wire data)
// ============================================================================

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null && !Array.isArray(value);
}

/** Validate a decoded JSON-RPC 2.0 request envelope. */
export function isJsonRpcRequest(value: unknown): value is JsonRpcRequest {
  if (!isRecord(value)) return false;
  if (value.jsonrpc !== '2.0') return false;
  if (typeof value.method !== 'string') return false;
  if (!('id' in value)) return false;
  const id = value.id;
  const validId = typeof id === 'string' || typeof id === 'number' || id === null;
  if (!validId) return false;
  if ('params' in value && !isRecord(value.params)) return false;
  return true;
}

/** Validate a decoded JSON-RPC 2.0 response envelope (success or error). */
export function isJsonRpcResponse(value: unknown): value is JsonRpcResponse {
  if (!isRecord(value)) return false;
  if (value.jsonrpc !== '2.0') return false;
  const id = value.id;
  if (!(typeof id === 'string' || typeof id === 'number' || id === null)) return false;
  const hasResult = 'result' in value;
  const hasError = 'error' in value;
  if (hasResult === hasError) return false; // exactly one of result / error
  if (hasError && !isJsonRpcError(value.error)) return false;
  return true;
}

/** Validate a JSON-RPC 2.0 error object. */
export function isJsonRpcError(value: unknown): value is JsonRpcError {
  if (!isRecord(value)) return false;
  if (typeof value.code !== 'number' || !Number.isInteger(value.code)) return false;
  if (typeof value.message !== 'string') return false;
  return true;
}

/** Validate a JSON-RPC 2.0 notification (no id, params required). */
export function isJsonRpcNotification(value: unknown): value is JsonRpcNotification {
  if (!isRecord(value)) return false;
  if (value.jsonrpc !== '2.0') return false;
  if (typeof value.method !== 'string') return false;
  if ('id' in value) return false;
  if (!('params' in value) || !isRecord(value.params)) return false;
  return true;
}

/** Validate a model descriptor coming from an untrusted provider listing. */
export function parseModelInfo(value: unknown): ModelInfo | null {
  if (!isRecord(value)) return null;
  if (typeof value.id !== 'string' || value.id.length === 0) return null;
  if (typeof value.name !== 'string') return null;
  if (typeof value.provider !== 'string') return null;
  if ('isDefault' in value && typeof value.isDefault !== 'boolean') return null;
  if ('contextLength' in value && (typeof value.contextLength !== 'number' || !Number.isFinite(value.contextLength))) return null;
  return value as unknown as ModelInfo;
}

/** Validate a mesh broadcast message envelope. */
export function isBroadcastMessage(value: unknown): value is BroadcastMessage {
  if (!isRecord(value)) return false;
  if (typeof value.id !== 'string' || value.id.length === 0) return false;
  if (typeof value.sender !== 'string') return false;
  if (typeof value.topic !== 'string') return false;
  if (typeof value.timestamp !== 'string') return false;
  if (!isBroadcastPayload(value.payload)) return false;
  return true;
}

/** Validate a mesh broadcast payload discriminator. */
export function isBroadcastPayload(value: unknown): value is BroadcastPayload {
  if (!isRecord(value)) return false;
  switch (value.type) {
    case 'status':
      return isRecord(value.status) && typeof (value.status as { state?: unknown }).state === 'string';
    case 'discovery':
      return typeof value.topic === 'string' && typeof value.findings === 'string' && Array.isArray(value.fileReferences);
    case 'alert':
      return typeof value.severity === 'string' && typeof value.message === 'string';
    case 'fact_update':
      return typeof value.key === 'string' && 'value' in value;
    case 'custom':
      return typeof value.kind === 'string' && 'data' in value;
    default:
      return false;
  }
}

/** Validate a point-to-point peer query. */
export function isPeerQuery(value: unknown): value is PeerQuery {
  if (!isRecord(value)) return false;
  return (
    typeof value.queryId === 'string' &&
    value.queryId.length > 0 &&
    typeof value.from === 'string' &&
    typeof value.to === 'string' &&
    typeof value.query === 'string' &&
    typeof value.timestamp === 'string'
  );
}

/** Validate a peer query response. */
export function isPeerResponse(value: unknown): value is PeerResponse {
  if (!isRecord(value)) return false;
  return (
    typeof value.queryId === 'string' &&
    value.queryId.length > 0 &&
    typeof value.from === 'string' &&
    typeof value.to === 'string' &&
    typeof value.answer === 'string' &&
    typeof value.success === 'boolean' &&
    typeof value.timestamp === 'string'
  );
}

// ============================================================================
// 1. JSON-RPC 2.0 constants & request validation
// ============================================================================

test('JSON-RPC constants expose ACP protocol version and standard error codes', () => {
  assert.equal(ACP_PROTOCOL_VERSION, 1);

  assert.equal(JSON_RPC_ERROR_CODES.PARSE_ERROR, -32700);
  assert.equal(JSON_RPC_ERROR_CODES.INVALID_REQUEST, -32600);
  assert.equal(JSON_RPC_ERROR_CODES.METHOD_NOT_FOUND, -32601);
  assert.equal(JSON_RPC_ERROR_CODES.INVALID_PARAMS, -32602);
  assert.equal(JSON_RPC_ERROR_CODES.INTERNAL_ERROR, -32603);

  // Fusion-specific extension codes live in the server-defined range.
  assert.equal(JSON_RPC_ERROR_CODES.SERVER_NOT_INITIALIZED, -32002);
  assert.equal(JSON_RPC_ERROR_CODES.SESSION_NOT_FOUND, -32001);
  assert.equal(JSON_RPC_ERROR_CODES.REQUEST_CANCELLED, -32000);
  assert.equal(JSON_RPC_ERROR_CODES.TOOL_EXECUTION_ERROR, -32010);
  assert.equal(JSON_RPC_ERROR_CODES.MCP_ERROR, -32020);
  assert.equal(JSON_RPC_ERROR_CODES.RATE_LIMIT_EXCEEDED, -32030);
  assert.equal(JSON_RPC_ERROR_CODES.AUTH_ERROR, -32040);
});

test('valid JSON-RPC requests pass shape validation', () => {
  const request: JsonRpcRequest<{ sessionId: string }> = {
    jsonrpc: '2.0',
    id: 42,
    method: 'session/new',
    params: { sessionId: 'abc' }
  };
  assert.equal(isJsonRpcRequest(request), true);

  // String id
  assert.equal(isJsonRpcRequest({ jsonrpc: '2.0', id: 'req-1', method: 'ping' }), true);
  // Null id
  assert.equal(isJsonRpcRequest({ jsonrpc: '2.0', id: null, method: 'ping' }), true);
  // Omitted params
  assert.equal(isJsonRpcRequest({ jsonrpc: '2.0', id: 1, method: 'ping' }), true);
});

test('malformed JSON-RPC requests fail shape validation', () => {
  assert.equal(isJsonRpcRequest(null), false);
  assert.equal(isJsonRpcRequest('request'), false);
  assert.equal(isJsonRpcRequest([]), false);
  // Wrong protocol version
  assert.equal(isJsonRpcRequest({ jsonrpc: '1.0', id: 1, method: 'ping' }), false);
  assert.equal(isJsonRpcRequest({ jsonrpc: 2.0, id: 1, method: 'ping' }), false);
  // Missing method
  assert.equal(isJsonRpcRequest({ jsonrpc: '2.0', id: 1 }), false);
  // Missing id
  assert.equal(isJsonRpcRequest({ jsonrpc: '2.0', method: 'ping' }), false);
  // Invalid id type
  assert.equal(isJsonRpcRequest({ jsonrpc: '2.0', id: { nested: true }, method: 'ping' }), false);
  assert.equal(isJsonRpcRequest({ jsonrpc: '2.0', id: undefined, method: 'ping' }), false);
  // Non-object params
  assert.equal(isJsonRpcRequest({ jsonrpc: '2.0', id: 1, method: 'ping', params: [1, 2] }), false);
});

// ============================================================================
// 2. JSON-RPC 2.0 response & error validation
// ============================================================================

test('successful responses validate with a result payload', () => {
  const response: JsonRpcResponse<NewSessionResult> = {
    jsonrpc: '2.0',
    id: 7,
    result: { sessionId: 'sess-123', models: [] }
  };
  assert.equal(isJsonRpcResponse(response), true);
  assert.equal(isJsonRpcResponse({ jsonrpc: '2.0', id: null, result: undefined as unknown as number }), true);
});

test('error responses validate with an error object', () => {
  const response: JsonRpcResponse<never, { session: string }> = {
    jsonrpc: '2.0',
    id: 'abc',
    error: {
      code: JSON_RPC_ERROR_CODES.SESSION_NOT_FOUND,
      message: 'Session not found',
      data: { session: 'abc' }
    }
  };
  assert.equal(isJsonRpcResponse(response), true);

  // Error without optional data
  assert.equal(
    isJsonRpcResponse({
      jsonrpc: '2.0',
      id: 1,
      error: { code: JSON_RPC_ERROR_CODES.INTERNAL_ERROR, message: 'boom' }
    }),
    true
  );
});

test('error objects reject non-integer codes and missing messages', () => {
  assert.equal(isJsonRpcError({ code: -32600, message: 'bad' }), true);
  assert.equal(isJsonRpcError({ code: -32600.5, message: 'bad' }), false);
  assert.equal(isJsonRpcError({ code: '-32600', message: 'bad' }), false);
  assert.equal(isJsonRpcError({ code: -32600 }), false);
  assert.equal(isJsonRpcError({ message: 'no code' }), false);
  assert.equal(isJsonRpcError('error'), false);
});

test('responses must carry exactly one of result or error', () => {
  // Both present — invalid
  assert.equal(
    isJsonRpcResponse({
      jsonrpc: '2.0',
      id: 1,
      result: 'ok',
      error: { code: -32603, message: 'boom' }
    }),
    false
  );
  // Neither present — invalid
  assert.equal(isJsonRpcResponse({ jsonrpc: '2.0', id: 1 }), false);
  // Invalid error object inside response
  assert.equal(
    isJsonRpcResponse({ jsonrpc: '2.0', id: 1, error: { code: 'oops', message: 'x' } }),
    false
  );
  // Malformed envelopes
  assert.equal(isJsonRpcResponse(null), false);
  assert.equal(isJsonRpcResponse({ id: 1, result: 'x' }), false);
  assert.equal(isJsonRpcResponse({ jsonrpc: '2.0', result: 'x' }), false);
});

// ============================================================================
// 3. JSON-RPC 2.0 notification validation
// ============================================================================

test('session/update notifications validate without an id', () => {
  const notification: JsonRpcNotification<{ sessionId: string; update: { kind: string } }> = {
    jsonrpc: '2.0',
    method: 'session/update',
    params: { sessionId: 'sess-1', update: { kind: 'status' } }
  };
  assert.equal(isJsonRpcNotification(notification), true);
  assert.equal(
    isJsonRpcNotification({ jsonrpc: '2.0', method: 'session/update', params: {} }),
    true
  );
});

test('notifications reject ids and require params', () => {
  // Requests (with id) are not notifications
  assert.equal(isJsonRpcNotification({ jsonrpc: '2.0', id: 1, method: 'm', params: {} }), false);
  // Missing params
  assert.equal(isJsonRpcNotification({ jsonrpc: '2.0', method: 'm' }), false);
  // Wrong version
  assert.equal(isJsonRpcNotification({ jsonrpc: '2.0', method: 'm', params: {}, jsonrpc2: false }), true); // extra keys tolerated
  assert.equal(isJsonRpcNotification({ jsonrpc: '2.0', id: null, method: 'm', params: {} }), false);
  assert.equal(isJsonRpcNotification(undefined), false);
});

// ============================================================================
// 4. Mesh broadcast message typing
// ============================================================================

const STATUS: MeshAgentInfo = {
  id: 'Scout-1',
  role: 'scout',
  description: 'Read-only repo explorer',
  status: { state: 'active', task: 'mapping src/ui' },
  registeredAt: '2026-09-02T10:00:00.000Z',
  lastActive: '2026-09-02T10:05:00.000Z',
  capabilities: ['rust', 'filesystem']
};

test('broadcast messages validate across every standard payload kind', () => {
  const statusBroadcast: BroadcastMessage = {
    id: '11111111-1111-1111-1111-111111111111',
    sender: 'Scout-1',
    topic: 'status',
    payload: { type: 'status', status: STATUS.status },
    timestamp: '2026-09-02T10:06:00.000Z'
  };
  assert.equal(isBroadcastMessage(statusBroadcast), true);

  const discovery: BroadcastMessage = {
    id: '22222222-2222-2222-2222-222222222222',
    sender: 'Scout-1',
    topic: 'discovery',
    payload: {
      type: 'discovery',
      topic: 'src/ui',
      findings: 'Found 4 UI clusters',
      fileReferences: ['src/ui/bench_runner.rs']
    },
    timestamp: '2026-09-02T10:06:30.000Z'
  };
  assert.equal(isBroadcastMessage(discovery), true);

  const alert: BroadcastMessage = {
    id: '33333333-3333-3333-3333-333333333333',
    sender: 'Reviewer-1',
    topic: 'alert',
    payload: { type: 'alert', severity: 'critical', message: 'unresolved merge conflict in main.rs' },
    timestamp: '2026-09-02T10:07:00.000Z'
  };
  assert.equal(isBroadcastMessage(alert), true);

  const factUpdate: BroadcastMessage = {
    id: '44444444-4444-4444-4444-444444444444',
    sender: 'Orchestrator',
    topic: 'coordination',
    payload: { type: 'fact_update', key: 'total_agents', value: 12 },
    timestamp: '2026-09-02T10:07:30.000Z'
  };
  assert.equal(isBroadcastMessage(factUpdate), true);

  const custom: BroadcastMessage = {
    id: '55555555-5555-5555-5555-555555555555',
    sender: 'Tester-2',
    topic: '*',
    payload: { type: 'custom', kind: 'benchmark_result', data: { throughput: 420 } },
    timestamp: '2026-09-02T10:08:00.000Z'
  };
  assert.equal(isBroadcastMessage(custom), true);
});

test('broadcast validation rejects unknown payload discriminators and bad envelopes', () => {
  // Unknown payload discriminator
  assert.equal(
    isBroadcastMessage({
      id: 'id-1',
      sender: 'a',
      topic: 'status',
      payload: { type: 'mystery' },
      timestamp: '2026-09-02T10:00:00.000Z'
    }),
    false
  );
  // Missing fields
  assert.equal(
    isBroadcastMessage({ sender: 'a', topic: 'status', payload: { type: 'alert', severity: 'info', message: 'x' }, timestamp: 't' }),
    false
  );
  assert.equal(
    isBroadcastMessage({ id: 'id-1', topic: 'status', payload: { type: 'alert', severity: 'info', message: 'x' }, timestamp: 't' }),
    false
  );
  // Bad nested payload fields
  assert.equal(
    isBroadcastPayload({ type: 'discovery', topic: 3, findings: 'x', fileReferences: [] }),
    false
  );
  assert.equal(isBroadcastPayload({ type: 'discovery', topic: 't', findings: 'x' }), false);
  assert.equal(isBroadcastPayload({ type: 'fact_update', key: 'k' }), false);
  assert.equal(isBroadcastPayload(null), false);
});

// ============================================================================
// 5. Peer query / response typing
// ============================================================================

test('peer queries and responses validate and correlate by queryId', () => {
  const query: PeerQuery = {
    queryId: 'q-100',
    from: 'Coder-1',
    to: 'Scout-1',
    query: 'Where is the Tool trait defined?',
    context: 'src/tools/mod.rs',
    timestamp: '2026-09-02T11:00:00.000Z'
  };
  assert.equal(isPeerQuery(query), true);

  const response: PeerResponse = {
    queryId: query.queryId,
    from: 'Scout-1',
    to: 'Coder-1',
    answer: 'src/tools/mod.rs, line 42',
    success: true,
    data: { file: 'src/tools/mod.rs', line: 42 },
    timestamp: '2026-09-02T11:00:02.000Z'
  };
  assert.equal(isPeerResponse(response), true);
  assert.equal(response.queryId, query.queryId, 'response correlates to the query');
});

test('peer messages reject missing correlation or sender fields', () => {
  assert.equal(
    isPeerQuery({ from: 'a', to: 'b', query: 'q', timestamp: 't' }), // no queryId
    false
  );
  assert.equal(
    isPeerQuery({ queryId: '', from: 'a', to: 'b', query: 'q', timestamp: 't' }), // empty queryId
    false
  );
  assert.equal(isPeerQuery({ queryId: 'q', to: 'b', query: 'q', timestamp: 't' }), false);
  assert.equal(isPeerQuery(undefined), false);

  assert.equal(
    isPeerResponse({ from: 'a', to: 'b', answer: 'x', success: true, timestamp: 't' }), // no queryId
    false
  );
  assert.equal(
    isPeerResponse({ queryId: 'q', from: 'a', to: 'b', answer: 'x', success: 'yes', timestamp: 't' }),
    false
  );
});

// ============================================================================
// 6. Session state serialization round-trip
// ============================================================================

function sampleTokenStats(): TokenStats {
  return {
    prompt_tokens: 1200,
    completion_tokens: 340,
    total_tokens: 1540,
    cached_tokens: 200,
    estimated_cost_usd: 0.0042
  };
}

function sampleSessionState(): SessionState {
  const userMessage: Message = { role: 'user', content: 'Refactor the SDK loader', timestamp: '2026-09-02T09:00:00.000Z' };
  const assistantMessage: Message = {
    role: 'assistant',
    content: 'On it.',
    reasoning_content: 'Plan: batch loader edits, then run tsc.',
    timestamp: '2026-09-02T09:00:05.000Z'
  };
  return {
    id: 'sess-8f3a2b10',
    activeModel: 'anthropic/claude-sonnet-4.5',
    systemPrompt: 'You are the Fusion agent.',
    messages: [userMessage, assistantMessage],
    tokenStats: sampleTokenStats(),
    turnCounter: 1,
    createdAt: '2026-09-02T09:00:00.000Z',
    updatedAt: '2026-09-02T09:00:05.000Z'
  };
}

test('session state survives a JSON serialization round-trip losslessly', () => {
  const original = sampleSessionState();
  const serialized = JSON.stringify(original);
  const restored = JSON.parse(serialized) as SessionState;

  assert.deepEqual(restored, original);
  assert.equal(restored.id, original.id);
  assert.equal(restored.activeModel, original.activeModel);
  assert.equal(restored.messages.length, 2);
  assert.deepEqual(restored.tokenStats, original.tokenStats);
  assert.equal(restored.turnCounter, original.turnCounter);
});

test('optional session fields survive round-trip when absent', () => {
  const minimal: SessionState = {
    id: 'sess-min',
    activeModel: 'deepseek/deepseek-chat',
    messages: [],
    tokenStats: { prompt_tokens: 0, completion_tokens: 0, total_tokens: 0 },
    turnCounter: 0
  };
  const restored = JSON.parse(JSON.stringify(minimal)) as SessionState;
  assert.deepEqual(restored, minimal);
  assert.equal('systemPrompt' in restored, false);
  assert.equal('createdAt' in restored, false);
});

test('session message roles round-trip through serialization', () => {
  const messages: Message[] = [
    { role: 'system', content: 'sys' },
    { role: 'user', content: 'u' },
    { role: 'assistant', content: 'a', tool_calls: [{ id: 'call-1', name: 'read_file', arguments: JSON.stringify({ path: 'a.rs' }) }] },
    { role: 'tool', content: 'file body', tool_call_id: 'call-1' }
  ];
  const restored = JSON.parse(JSON.stringify(messages)) as Message[];
  assert.deepEqual(restored, messages);
  assert.deepEqual(
    restored.map((m) => m.role),
    ['system', 'user', 'assistant', 'tool']
  );
  assert.deepEqual(restored[2]?.tool_calls, [{ id: 'call-1', name: 'read_file', arguments: JSON.stringify({ path: 'a.rs' }) }]);
  assert.equal(restored[3]?.tool_call_id, 'call-1');
});

test('token stats round-trip preserves numeric precision', () => {
  const stats = sampleTokenStats();
  const restored = JSON.parse(JSON.stringify(stats)) as TokenStats;
  assert.equal(restored.total_tokens, stats.prompt_tokens + stats.completion_tokens);
  assert.equal(restored.estimated_cost_usd, 0.0042);
});

test('corrupted session state is detectable after decoding', () => {
  const restored = JSON.parse(JSON.stringify(sampleSessionState())) as SessionState;
  // Simulate a truncated / tampered transcript
  restored.messages = [];
  assert.notDeepEqual(restored, sampleSessionState());
  assert.equal(restored.messages.length, 0);
  // Tampered token accounting
  restored.tokenStats.total_tokens = 999_999;
  assert.notEqual(
    restored.tokenStats.total_tokens,
    restored.tokenStats.prompt_tokens + restored.tokenStats.completion_tokens
  );
});

// ============================================================================
// 7. Model info parsing
// ============================================================================

test('model info parses complete descriptors from provider listings', () => {
  const raw = {
    id: 'anthropic/claude-sonnet-4.5',
    name: 'Claude Sonnet 4.5',
    provider: 'openrouter',
    isDefault: true,
    contextLength: 200_000,
    pricing: '$3 / M input tokens',
    description: 'Balanced coding model with extended thinking.'
  };
  const model = parseModelInfo(raw);
  assert.notEqual(model, null);
  assert.equal(model?.id, 'anthropic/claude-sonnet-4.5');
  assert.equal(model?.provider, 'openrouter');
  assert.equal(model?.isDefault, true);
  assert.equal(model?.contextLength, 200_000);
});

test('model info parses minimal descriptors with only required fields', () => {
  const model = parseModelInfo({ id: 'llama3:8b', name: 'Llama 3 8B', provider: 'ollama' });
  assert.notEqual(model, null);
  assert.equal(model?.contextLength, undefined);
  assert.equal(model?.isDefault, undefined);
  // Round-trips losslessly through JSON
  const restored = parseModelInfo(JSON.parse(JSON.stringify(model)));
  assert.deepEqual(restored, model);
});

test('model info parsing rejects malformed listings', () => {
  assert.equal(parseModelInfo(null), null);
  assert.equal(parseModelInfo('claude'), null);
  assert.equal(parseModelInfo(42), null);
  assert.equal(parseModelInfo({}), null);
  // Missing id
  assert.equal(parseModelInfo({ name: 'Claude', provider: 'anthropic' }), null);
  // Empty id
  assert.equal(parseModelInfo({ id: '', name: 'Claude', provider: 'anthropic' }), null);
  // Non-string provider
  assert.equal(parseModelInfo({ id: 'm1', name: 'M1', provider: 3 }), null);
  // Bad optional field types
  assert.equal(parseModelInfo({ id: 'm1', name: 'M1', provider: 'p', contextLength: 'huge' }), null);
  assert.equal(parseModelInfo({ id: 'm1', name: 'M1', provider: 'p', contextLength: Number.NaN }), null);
  assert.equal(parseModelInfo({ id: 'm1', name: 'M1', provider: 'p', isDefault: 'yes' }), null);
});

test('session results carrying model catalogs round-trip through JSON', () => {
  const newSession: NewSessionResult = {
    sessionId: 'sess-42',
    models: [
      { id: 'm1', name: 'Model One', provider: 'openrouter', isDefault: true },
      { id: 'm2', name: 'Model Two', provider: 'anthropic' }
    ]
  };
  const restored = JSON.parse(JSON.stringify(newSession)) as NewSessionResult;
  assert.deepEqual(restored, newSession);
  assert.equal(restored.models?.length, 2);
  for (const entry of restored.models ?? []) {
    assert.notEqual(parseModelInfo(entry), null, `every catalog entry re-parses: ${entry.id}`);
  }

  const loadResult: LoadSessionResult = {
    sessionId: 'sess-42',
    activeModel: 'm1',
    messageCount: 7,
    title: 'Refactor SDK loader'
  };
  const restoredLoad = JSON.parse(JSON.stringify(loadResult)) as LoadSessionResult;
  assert.deepEqual(restoredLoad, loadResult);
  assert.equal(restoredLoad.messageCount, 7);
});
