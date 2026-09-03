/**
 * @fusion/sdk — FusionAgent High-Level Controller & Transports
 *
 * Provides a unified client class `FusionAgent` supporting:
 * 1. Stdio child process transport (spawning `fusion --acp` over JSON-RPC 2.0 stdio)
 * 2. In-memory WebAssembly transport (running in browser or Node.js)
 * 3. WebSocket ACP transport (connecting to remote `fusion --acp` daemon)
 * 4. Streaming token deltas, tool executions, advisor critiques, and session VFS
 *
 * @packageDocumentation
 */

import type {
  FusionConfig,
  PromptOptions,
  AgentEvent,
  AgentEventCallback,
  PromptTurnCallback,
  Message,
  TokenStats,
  CheckpointData,
  WasmFusionAgentBindings,
  WasmInitOptions,
  ToolDefinition,
  AgentTransport,
  JsonRpcRequest,
  JsonRpcResponse,
  JsonRpcNotification,
  SessionUpdate,
  SessionUpdateParams,
  InitializeRequest,
  InitializeResult,
  NewSessionRequest,
  NewSessionResult,
  PromptRequest,
  PromptResponse
} from './types.js';
import { initWasm, getWasmModule, isWasmInitialized } from './wasm.js';

// ============================================================================
// Configuration & Options
// ============================================================================

/**
 * Extended configuration options for initializing a `FusionAgent`.
 */
export interface FusionAgentOptions extends FusionConfig {
  /**
   * Transport medium to use for agent communication.
   * - `'stdio'`: Spawns `fusion --acp` child process over standard I/O (Node.js/Bun/Deno only)
   * - `'wasm'`: In-memory WebAssembly execution (browser & Node.js)
   * - `'websocket'`: Connects to a remote ACP WebSocket daemon (e.g. `ws://127.0.0.1:9001`)
   * - Custom `AgentTransport` instance
   * Defaults to `'wasm'` in browser environments and `'stdio'` if running under Node.js when binary is available.
   */
  transport?: 'stdio' | 'wasm' | 'websocket' | AgentTransport;

  /** Path to the `fusion` executable for stdio transport. Defaults to `'fusion'` */
  binaryPath?: string;

  /** Command-line arguments passed to the `fusion` binary. Defaults to `['--acp']` */
  args?: string[];

  /** Initial working directory for the agent workspace */
  cwd?: string;

  /** Environment variables passed to child process */
  env?: Record<string, string>;

  /** Remote WebSocket URL for `'websocket'` transport mode */
  wsUrl?: string;

  /** WebAssembly module initialization options */
  wasmOptions?: WasmInitOptions;

  /** Optional pre-existing session ID to resume */
  sessionId?: string;
}

/**
 * Structural interface representing a spawned child process.
 */
interface ChildProcessHandle {
  stdin: {
    write(data: string, encoding: string, callback?: (err?: Error | null) => void): boolean;
    end(): void;
  } | null;
  stdout: {
    setEncoding(encoding: string): void;
    on(event: string, listener: (chunk: string | Buffer) => void): void;
  } | null;
  stderr: {
    setEncoding(encoding: string): void;
    on(event: string, listener: (chunk: string | Buffer) => void): void;
  } | null;
  on(event: string, listener: (...args: unknown[]) => void): void;
  kill(signal?: string): boolean;
}

// ============================================================================
// Stdio Child Process Transport (JSON-RPC 2.0 ACP)
// ============================================================================

/**
 * Transport communicating with a local `fusion --acp` process over stdio.
 */
export class StdioTransport implements AgentTransport {
  readonly type = 'stdio' as const;
  readonly endpoint: string;

  private binaryPath: string;
  private args: string[];
  private cwd?: string;
  private env?: Record<string, string>;
  private process: ChildProcessHandle | null = null;
  private nextRequestId: number = 1;
  private pendingRequests: Map<
    number | string,
    {
      resolve: (value: unknown) => void;
      reject: (error: Error) => void;
      method: string;
    }
  > = new Map();
  private messageListeners: Set<(data: string | JsonRpcResponse | JsonRpcNotification) => void> =
    new Set();
  private activeStreams: Map<
    string,
    {
      controller: ReadableStreamDefaultController<AgentEvent>;
      onEvent?: (event: AgentEvent) => void;
      accumulatedText: string;
    }
  > = new Map();
  private lineBuffer: string = '';
  private stderrBuffer: string = '';
  private _isConnected: boolean = false;
  private _sessionId: string | null = null;
  private _activeModel: string = 'anthropic/claude-3.5-sonnet';
  private config: FusionConfig = {};

  constructor(options: {
    binaryPath?: string;
    args?: string[];
    cwd?: string;
    env?: Record<string, string>;
    config?: FusionConfig;
    sessionId?: string;
  } = {}) {
    this.binaryPath = options.binaryPath || 'fusion';
    this.args = options.args || ['--acp'];
    this.cwd = options.cwd;
    this.env = options.env;
    this.config = options.config || {};
    this.endpoint = `${this.binaryPath} ${this.args.join(' ')}`;
    if (options.sessionId) {
      this._sessionId = options.sessionId;
    }
    if (this.config.default_model) {
      this._activeModel = this.config.default_model;
    }
  }

  get isConnected(): boolean {
    return this._isConnected && this.process !== null;
  }

  get sessionId(): string | null {
    return this._sessionId;
  }

  get activeModel(): string {
    return this._activeModel;
  }

  async connect(): Promise<void> {
    if (this._isConnected && this.process) {
      return;
    }

    const isNode =
      typeof process !== 'undefined' &&
      process.versions != null &&
      process.versions.node != null;

    if (!isNode) {
      throw new Error(
        'StdioTransport is only supported in Node.js / Bun / Deno environments with child_process support.'
      );
    }

    // Platform-specific runtime module: child_process cannot be statically imported in browser bundles.
    const g = typeof globalThis !== 'undefined' ? (globalThis as Record<string, unknown>) : undefined;
    const hooksState = g?.__fusionHooksState as { createServer?: (cmd: string, args: readonly string[], opts: unknown) => unknown } | undefined;
    const mockState = g?.__fusionMock as { createServer?: (cmd: string, args: readonly string[], opts: unknown) => unknown } | undefined;
    const mockSpawn = hooksState?.createServer || mockState?.createServer;
    let cp: { spawn: (command: string, args: readonly string[], options?: unknown) => unknown };
    if (typeof mockSpawn === 'function') {
      cp = {
        spawn: (cmd: string, args: readonly string[], opts: unknown) => mockSpawn(cmd, args, opts)
      };
    } else {
      try {
        const importedCp = await import('child_process');
        cp = importedCp as unknown as { spawn: (command: string, args: readonly string[], options?: unknown) => unknown };
      } catch (err) {
        throw new Error(`Failed to load child_process module: ${err}`);
      }
    }

    const childEnv = {
      ...(process.env || {}),
      ...(this.env || {})
    };

    try {
      this.process = cp.spawn(this.binaryPath, this.args, {
        stdio: ['pipe', 'pipe', 'pipe'],
        cwd: this.cwd || (typeof process !== 'undefined' ? process.cwd() : undefined),
        env: childEnv
      }) as unknown as ChildProcessHandle;
    } catch (err) {
      throw new Error(
        `Failed to spawn Fusion ACP child process '${this.binaryPath}': ${err instanceof Error ? err.message : String(err)}`
      );
    }

    this.process.stdout?.setEncoding('utf-8');
    this.process.stderr?.setEncoding('utf-8');

    this.process.stdout?.on('data', (chunk: string | Buffer) => {
      this.handleStdoutData(chunk.toString());
    });

    this.process.stderr?.on('data', (chunk: string | Buffer) => {
      const text = chunk.toString();
      this.stderrBuffer = (this.stderrBuffer + text).slice(-4096);
    });

    this.process.on('error', (...args: unknown[]) => {
      const err = args[0] instanceof Error ? args[0] : new Error(String(args[0] || 'Unknown process error'));
      this.handleProcessTermination(new Error(`Fusion ACP process error: ${err.message}`));
    });

    this.process.on('exit', (...args: unknown[]) => {
      const code = args[0] as number | null;
      const signal = args[1] as string | null;
      const reason = `Fusion ACP process exited with code ${code}, signal ${signal}.\nStderr: ${this.stderrBuffer}`;
      this.handleProcessTermination(new Error(reason));
    });

    this._isConnected = true;

    // Perform ACP initialize handshake
    const initReq: InitializeRequest = {
      protocolVersion: 1,
      clientCapabilities: {
        fs: { readTextFile: true, writeTextFile: true },
        terminal: true,
        session: { streaming: true }
      },
      clientInfo: {
        name: '@fusion/sdk',
        version: '0.3.0'
      }
    };

    await this.sendRequest<InitializeResult>('initialize', initReq as unknown as Record<string, unknown>);

    // Send initialized notification
    this.sendNotification('initialized', {});

    // Create or resume session
    if (!this._sessionId) {
      const newSessionReq: NewSessionRequest = {
        cwd: this.cwd,
        model: this.config.default_model,
        provider: this.config.default_provider
      };
      const sessionRes = await this.sendRequest<NewSessionResult>('session/new', newSessionReq as unknown as Record<string, unknown>);
      this._sessionId = sessionRes.sessionId;
      if (sessionRes.models && sessionRes.models.length > 0) {
        const def = sessionRes.models.find((m) => m.isDefault);
        if (def) this._activeModel = def.id;
      }
    }
  }

  async send(message: string | JsonRpcRequest): Promise<void> {
    if (!this.process || !this.process.stdin) {
      throw new Error('StdioTransport is not connected or process stdin is closed.');
    }

    const payload = typeof message === 'string' ? message : JSON.stringify(message);
    const line = payload.endsWith('\n') ? payload : payload + '\n';

    return new Promise<void>((resolve, reject) => {
      this.process!.stdin!.write(line, 'utf-8', (err: Error | null | undefined) => {
        if (err) reject(err);
        else resolve();
      });
    });
  }

  async sendRequest<TResult = unknown>(method: string, params?: Record<string, unknown>): Promise<TResult> {
    if (!this._isConnected) {
      await this.connect();
    }

    const id = this.nextRequestId++;
    const request: JsonRpcRequest = {
      jsonrpc: '2.0',
      id,
      method,
      params
    };

    return new Promise<TResult>((resolve, reject) => {
      this.pendingRequests.set(id, {
        resolve: resolve as (val: unknown) => void,
        reject,
        method
      });
      this.send(request).catch((err: Error) => {
        this.pendingRequests.delete(id);
        reject(err);
      });
    });
  }

  sendNotification(method: string, params?: Record<string, unknown>): void {
    if (!this._isConnected && !this.process) {
      return;
    }
    const notification: JsonRpcNotification = {
      jsonrpc: '2.0',
      method,
      params: params || {}
    };
    this.send(notification).catch(() => {});
  }

  onMessage(handler: (data: string | JsonRpcResponse | JsonRpcNotification) => void): () => void {
    this.messageListeners.add(handler);
    return () => {
      this.messageListeners.delete(handler);
    };
  }

  async promptStream(
    text: string,
    options?: PromptOptions
  ): Promise<ReadableStream<AgentEvent>> {
    if (!this._isConnected) {
      await this.connect();
    }

    const currentSessionId = this._sessionId || 'default';

    return new ReadableStream<AgentEvent>({
      start: async (controller) => {
        const streamEntry = {
          controller,
          onEvent: options?.onEvent,
          accumulatedText: ''
        };
        this.activeStreams.set(currentSessionId, streamEntry);

        let abortListener: (() => void) | null = null;
        if (options?.signal) {
          if (options.signal.aborted) {
            controller.error(new Error('Prompt was aborted before execution'));
            this.activeStreams.delete(currentSessionId);
            return;
          }
          abortListener = () => {
            this.cancelSession(currentSessionId).catch(() => {});
            try {
              const cancelEv: AgentEvent = {
                type: 'status',
                message: 'Turn cancelled by user'
              };
              controller.enqueue(cancelEv);
              options?.onEvent?.(cancelEv);
              controller.close();
            } catch {}
            this.activeStreams.delete(currentSessionId);
          };
          options.signal.addEventListener('abort', abortListener, { once: true });
        }

        try {
          const promptParams: PromptRequest = {
            sessionId: currentSessionId,
            prompt: text
          };

          const response = await this.sendRequest<PromptResponse>('session/prompt', promptParams as unknown as Record<string, unknown>);

          // Emit finished event if usage stats present
          const stats = response?.stats;
          const finishedEv: AgentEvent = {
            type: 'finished',
            usage: {
              prompt_tokens: stats?.promptTokens ?? 0,
              completion_tokens: stats?.completionTokens ?? 0,
              total_tokens: stats?.totalTokens ?? 0
            }
          };

          try {
            controller.enqueue(finishedEv);
            options?.onEvent?.(finishedEv);
          } catch {}

          try {
            controller.close();
          } catch {}
        } catch (err: unknown) {
          const errorMsg = err instanceof Error ? err.message : String(err);
          const errorEv: AgentEvent = {
            type: 'error',
            message: errorMsg
          };
          try {
            controller.enqueue(errorEv);
            options?.onEvent?.(errorEv);
          } catch {}
          try {
            controller.close();
          } catch {}
        } finally {
          if (options?.signal && abortListener) {
            options.signal.removeEventListener('abort', abortListener);
          }
          this.activeStreams.delete(currentSessionId);
        }
      }
    });
  }

  async cancelSession(sessionId?: string): Promise<void> {
    const targetSessionId = sessionId || this._sessionId;
    if (!targetSessionId || !this._isConnected) return;

    try {
      await this.sendRequest('session/cancel', { sessionId: targetSessionId });
    } catch {
      // Ignore cancellation failures
    }
  }

  async switchModel(model: string): Promise<void> {
    this._activeModel = model;
    this.config.default_model = model;
  }

  async disconnect(): Promise<void> {
    if (this._sessionId && this.process && this._isConnected) {
      try {
        await this.sendRequest('session/close', { sessionId: this._sessionId });
      } catch {
        // ignore close error during teardown
      }
    }

    this._isConnected = false;

    if (this.process) {
      try {
        this.process.stdin?.end();
        this.process.kill('SIGTERM');
      } catch {
        // ignore process kill error
      }
      this.process = null;
    }

    this.pendingRequests.forEach((req) => {
      req.reject(new Error('StdioTransport disconnected'));
    });
    this.pendingRequests.clear();
    this.activeStreams.clear();
  }

  private handleStdoutData(chunk: string): void {
    this.lineBuffer += chunk;
    const lines = this.lineBuffer.split('\n');
    this.lineBuffer = lines.pop() ?? '';

    for (const line of lines) {
      const trimmed = line.trim();
      if (!trimmed) continue;

      let msg: Record<string, unknown>;
      try {
        msg = JSON.parse(trimmed);
      } catch {
        // Non-JSON stdout log line, ignore
        continue;
      }

      // Notify raw listeners
      this.messageListeners.forEach((listener) => {
        try {
          listener(msg as unknown as JsonRpcResponse);
        } catch {}
      });

      // Handle RPC response
      if (msg.id !== undefined && (msg.result !== undefined || msg.error !== undefined)) {
        const pending = this.pendingRequests.get(msg.id as number | string);
        if (pending) {
          this.pendingRequests.delete(msg.id as number | string);
          if (msg.error && typeof msg.error === 'object') {
            const errObj = msg.error as { code?: number; message?: string };
            pending.reject(
              new Error(`[ACP Error ${errObj.code ?? 'unknown'}] ${errObj.message ?? 'Unknown error'}`)
            );
          } else {
            pending.resolve(msg.result);
          }
        }
      }

      // Handle server notification
      if (msg.method === 'session/update' && msg.params && typeof msg.params === 'object') {
        this.handleSessionUpdate(msg.params as SessionUpdateParams);
      }
    }
  }

  private handleSessionUpdate(params: SessionUpdateParams): void {
    const sessionId = params.sessionId || this._sessionId || 'default';
    const stream = this.activeStreams.get(sessionId) || this.activeStreams.values().next().value;
    if (!stream) return;

    const events = mapSessionUpdateToAgentEvents(params.update);
    for (const ev of events) {
      try {
        stream.controller.enqueue(ev);
        stream.onEvent?.(ev);
      } catch {
        // Stream may have closed
      }
    }
  }

  private handleProcessTermination(err: Error): void {
    this._isConnected = false;
    this.pendingRequests.forEach((req) => req.reject(err));
    this.pendingRequests.clear();
    this.activeStreams.forEach((s) => {
      try {
        s.controller.error(err);
      } catch {}
    });
    this.activeStreams.clear();
  }
}

// ============================================================================
// WebAssembly In-Memory Transport
// ============================================================================

/**
 * Transport communicating with in-memory WebAssembly agent bindings.
 */
export class WasmTransport implements AgentTransport {
  readonly type = 'wasm' as const;
  readonly endpoint = 'wasm://in-memory';

  private bindings: WasmFusionAgentBindings | null = null;
  private config: FusionConfig;
  private wasmOptions?: WasmInitOptions;
  private _isConnected: boolean = false;
  private messageListeners: Set<(data: string | JsonRpcResponse | JsonRpcNotification) => void> =
    new Set();

  constructor(
    config: FusionConfig = {},
    wasmOptions?: WasmInitOptions,
    bindings?: WasmFusionAgentBindings
  ) {
    this.config = config;
    this.wasmOptions = wasmOptions;
    if (bindings) {
      this.bindings = bindings;
      this._isConnected = true;
    }
  }

  get isConnected(): boolean {
    return this._isConnected && this.bindings !== null;
  }

  getRawBindings(): WasmFusionAgentBindings | null {
    return this.bindings;
  }

  async connect(): Promise<void> {
    if (this._isConnected && this.bindings) {
      return;
    }

    if (!isWasmInitialized()) {
      await initWasm(this.wasmOptions);
    }

    const wasm = getWasmModule();
    const configJson = JSON.stringify(this.config);
    this.bindings = wasm.create_agent(configJson);
    this._isConnected = true;
  }

  async send(_message: string | JsonRpcRequest): Promise<void> {
    // WASM transport executes calls synchronously via exported bindings
  }

  onMessage(handler: (data: string | JsonRpcResponse | JsonRpcNotification) => void): () => void {
    this.messageListeners.add(handler);
    return () => {
      this.messageListeners.delete(handler);
    };
  }

  async promptStream(
    text: string,
    options?: PromptOptions
  ): Promise<ReadableStream<AgentEvent>> {
    if (!this._isConnected || !this.bindings) {
      await this.connect();
    }

    const bindings = this.bindings!;

    return new ReadableStream<AgentEvent>({
      start: async (controller) => {
        let aborted = false;
        let abortListener: (() => void) | null = null;

        if (options?.signal) {
          if (options.signal.aborted) {
            controller.error(new Error('Prompt was aborted'));
            return;
          }
          abortListener = () => {
            aborted = true;
            try {
              const cancelEv: AgentEvent = {
                type: 'status',
                message: 'Turn aborted by signal'
              };
              controller.enqueue(cancelEv);
              options?.onEvent?.(cancelEv);
              controller.close();
            } catch {}
          };
          options.signal.addEventListener('abort', abortListener, { once: true });
        }

        let finishedEmitted = false;

        const onWasmEvent = (raw: unknown) => {
          if (aborted) return;
          try {
            const eventObj: AgentEvent =
              typeof raw === 'string' ? JSON.parse(raw) : (raw as AgentEvent);
            if (eventObj.type === 'finished') {
              finishedEmitted = true;
            }
            controller.enqueue(eventObj);
            options?.onEvent?.(eventObj);
          } catch {
            // Ignore malformed event parsing
          }
        };

        try {
          const responseText = await bindings.prompt_turn(text, onWasmEvent);

          if (!aborted && !finishedEmitted) {
            const finishedEv: AgentEvent = {
              type: 'finished',
              usage: {
                prompt_tokens: Math.max(1, Math.floor(text.length / 4)),
                completion_tokens: Math.max(1, Math.floor(responseText.length / 4)),
                total_tokens: Math.max(2, Math.floor((text.length + responseText.length) / 4))
              }
            };
            try {
              controller.enqueue(finishedEv);
              options?.onEvent?.(finishedEv);
            } catch {}
          }

          if (!aborted) {
            try {
              controller.close();
            } catch {}
          }
        } catch (err: unknown) {
          if (!aborted) {
            const errorMsg = err instanceof Error ? err.message : String(err);
            const errEv: AgentEvent = {
              type: 'error',
              message: errorMsg
            };
            try {
              controller.enqueue(errEv);
              options?.onEvent?.(errEv);
            } catch {}
            try {
              controller.close();
            } catch {}
          }
        } finally {
          if (options?.signal && abortListener) {
            options.signal.removeEventListener('abort', abortListener);
          }
        }
      }
    });
  }

  async switchModel(model: string): Promise<void> {
    this.config.default_model = model;
    if (this.bindings) {
      this.bindings.set_active_model(model);
    }
  }

  async disconnect(): Promise<void> {
    this._isConnected = false;
    this.bindings = null;
    this.messageListeners.clear();
  }
}

// ============================================================================
// Helper: Map ACP SessionUpdate -> AgentEvent[]
// ============================================================================

function mapSessionUpdateToAgentEvents(update: SessionUpdate | Record<string, unknown>): AgentEvent[] {
  if (!update || typeof update !== 'object') return [];
  const kind = ('kind' in update ? update.kind : 'type' in update ? update.type : '') as string;
  const raw = update as Record<string, unknown>;
  const events: AgentEvent[] = [];

  switch (kind) {
    case 'agent_message_chunk': {
      let delta = '';
      const content = raw.content as { content?: Array<{ text?: string }> } | string | undefined;
      if (content && typeof content === 'object' && Array.isArray(content.content)) {
        delta = content.content.map((b) => b.text || '').join('');
      } else if (typeof content === 'string') {
        delta = content;
      } else if (typeof raw.delta === 'string') {
        delta = raw.delta;
      }
      events.push({
        type: 'text_delta',
        delta
      });
      break;
    }

    case 'agent_thought_chunk': {
      const delta = (raw.thought as string) || (raw.delta as string) || '';
      events.push({
        type: 'thinking_delta',
        delta
      });
      break;
    }

    case 'tool_call': {
      const argsRaw = raw.args;
      const parsedArgs =
        typeof argsRaw === 'string'
          ? (() => {
              try {
                return JSON.parse(argsRaw);
              } catch {
                return { raw: argsRaw };
              }
            })()
          : typeof argsRaw === 'object' && argsRaw !== null
            ? (argsRaw as Record<string, unknown>)
            : {};

      events.push({
        type: 'tool_started',
        id: (raw.call_id as string) || (raw.callId as string) || (raw.id as string) || 'tool_call',
        name: (raw.name as string) || 'tool',
        args: parsedArgs
      });
      break;
    }

    case 'tool_call_result': {
      events.push({
        type: 'tool_finished',
        id: (raw.call_id as string) || (raw.callId as string) || (raw.id as string) || 'tool_call',
        name: (raw.name as string) || 'tool',
        success: raw.success !== false,
        output: (raw.output as string) || (raw.error as string) || '',
        duration_ms: (raw.duration_ms as number) || (raw.durationMs as number) || 0
      });
      break;
    }

    case 'tool_status': {
      events.push({
        type: 'status',
        message: (raw.status as string) || `Tool ${raw.name} in progress...`
      });
      break;
    }

    case 'advisor_started': {
      events.push({
        type: 'advisor_started',
        advisor: (raw.advisor as string) || 'Advisor',
        role: (raw.role as string) || 'Reviewer'
      });
      break;
    }

    case 'advisor_critique': {
      events.push({
        type: 'advisor_critique',
        advisor: (raw.advisor as string) || 'Advisor',
        approved: raw.approved !== false,
        critique: (raw.critique as string) || '',
        ...(raw.suggestions ? { suggestions: raw.suggestions as string[] } : {})
      });
      break;
    }

    case 'token_stats': {
      events.push({
        type: 'token_stats',
        prompt_tokens: ((raw.prompt_tokens ?? raw.promptTokens) as number) ?? 0,
        completion_tokens: ((raw.completion_tokens ?? raw.completionTokens) as number) ?? 0,
        total_tokens: ((raw.total_tokens ?? raw.totalTokens) as number) ?? 0
      });
      break;
    }

    case 'status': {
      events.push({
        type: 'status',
        message: (raw.message as string) || ''
      });
      break;
    }

    default: {
      if (typeof raw.type === 'string') {
        events.push(raw as unknown as AgentEvent);
      } else {
        events.push({
          type: kind || 'unknown',
          ...raw
        });
      }
      break;
    }
  }

  return events;
}

// ============================================================================
// FusionAgent High-Level Controller Class
// ============================================================================

/**
 * High-level Fusion Agent client controlling multi-model conversations,
 * tool execution, virtual filesystem (VFS), advisors, and real-time event streaming.
 */
export class FusionAgent {
  private transport: AgentTransport;
  private config: FusionConfig;
  private options: FusionAgentOptions;
  private isInitialized: boolean = false;
  private eventListeners: Set<AgentEventCallback> = new Set();

  /**
   * Constructs a `FusionAgent` instance.
   * Supports raw WASM bindings, custom `AgentTransport`, or `FusionAgentOptions`.
   */
  constructor(
    transportOrBindingsOrOptions?:
      | WasmFusionAgentBindings
      | AgentTransport
      | FusionAgentOptions
      | FusionConfig,
    config: FusionConfig = {}
  ) {
    if (!transportOrBindingsOrOptions) {
      this.options = {};
      this.config = config;
      this.transport = new WasmTransport(this.config);
    } else if (
      typeof (transportOrBindingsOrOptions as AgentTransport).connect === 'function' &&
      typeof (transportOrBindingsOrOptions as AgentTransport).send === 'function'
    ) {
      // Instance of AgentTransport
      this.transport = transportOrBindingsOrOptions as AgentTransport;
      this.config = config;
      this.options = { ...config, transport: this.transport };
    } else if (
      typeof (transportOrBindingsOrOptions as WasmFusionAgentBindings).get_session_id === 'function'
    ) {
      // Raw WasmFusionAgentBindings wrapper for backward compatibility
      const bindings = transportOrBindingsOrOptions as WasmFusionAgentBindings;
      this.config = config;
      this.options = { ...config, transport: 'wasm' };
      this.transport = new WasmTransport(this.config, undefined, bindings);
      this.isInitialized = true;
    } else {
      // FusionAgentOptions or FusionConfig
      const opts = transportOrBindingsOrOptions as FusionAgentOptions;
      this.options = opts;
      this.config = { ...opts, ...config };

      if (opts.transport && typeof opts.transport === 'object') {
        this.transport = opts.transport;
      } else if (opts.transport === 'stdio') {
        this.transport = new StdioTransport({
          binaryPath: opts.binaryPath,
          args: opts.args,
          cwd: opts.cwd,
          env: opts.env,
          config: this.config,
          sessionId: opts.sessionId
        });
      } else {
        this.transport = new WasmTransport(this.config, opts.wasmOptions);
      }
    }
  }

  /**
   * Asynchronously creates and initializes a new `FusionAgent`.
   * Automatically selects the optimal transport and performs handshake.
   *
   * @example
   * ```typescript
   * import { FusionAgent } from '@fusion/sdk';
   *
   * // 1. Stdio child process transport (IDE / CLI backend)
   * const agent = await FusionAgent.create({
   *   transport: 'stdio',
   *   default_model: 'anthropic/claude-3.5-sonnet'
   * });
   *
   * // 2. Streaming execution
   * const stream = await agent.prompt('Write a Rust fibonacci function');
   * const reader = stream.getReader();
   * while (true) {
   *   const { done, value } = await reader.read();
   *   if (done) break;
   *   console.log(value);
   * }
   * ```
   */
  static async create(
    options: FusionAgentOptions | FusionConfig = {},
    wasmOptions?: WasmInitOptions
  ): Promise<FusionAgent> {
    const isNode =
      typeof process !== 'undefined' &&
      process.versions != null &&
      process.versions.node != null;

    const resolvedOptions: FusionAgentOptions = { ...options };

    // If transport is not explicitly specified:
    // - Default to 'stdio' in Node environment if binary is not disabled
    // - Default to 'wasm' in browser environment
    if (!resolvedOptions.transport) {
      resolvedOptions.transport = isNode ? 'stdio' : 'wasm';
    }

    if (wasmOptions && !resolvedOptions.wasmOptions) {
      resolvedOptions.wasmOptions = wasmOptions;
    }

    const agent = new FusionAgent(resolvedOptions);
    await agent.initialize();
    return agent;
  }

  /**
   * Initializes the agent transport and completes the connection handshake.
   */
  async initialize(): Promise<void> {
    if (this.isInitialized && this.transport.isConnected) {
      return;
    }

    await this.transport.connect();
    this.isInitialized = true;
  }

  /**
   * Executes a conversation turn, returning a `ReadableStream` of real-time `AgentEvent` chunks.
   *
   * @param text User input prompt or slash command
   * @param options Turn options (abort signal, model override, onEvent callback)
   * @returns `ReadableStream<AgentEvent>` streaming response chunks
   */
  async prompt(
    text: string,
    options?: PromptOptions
  ): Promise<ReadableStream<AgentEvent>> {
    if (!this.isInitialized || !this.transport.isConnected) {
      await this.initialize();
    }

    const wrappedOptions: PromptOptions = {
      ...options,
      onEvent: (event: AgentEvent) => {
        options?.onEvent?.(event);
        this.eventListeners.forEach((listener) => {
          try {
            listener(event);
          } catch {}
        });
      }
    };

    if (this.transport instanceof StdioTransport) {
      return this.transport.promptStream(text, wrappedOptions);
    } else if (this.transport instanceof WasmTransport) {
      return this.transport.promptStream(text, wrappedOptions);
    }

    // Generic fallback for custom transports
    return new ReadableStream<AgentEvent>({
      start: async (controller) => {
        try {
          const req: PromptRequest = {
            sessionId: this.getSessionId(),
            prompt: text
          };
          await this.transport.send({
            jsonrpc: '2.0',
            id: Date.now(),
            method: 'session/prompt',
            params: req
          });
        } catch (err) {
          controller.error(err);
        }
      }
    });
  }

  /**
   * Convenience turn executor that accumulates text deltas and returns the complete response string.
   *
   * @param input User prompt text
   * @param onEvent Optional streaming callback
   * @returns Full assistant response string
   */
  async promptTurn(
    input: string,
    onEvent?: PromptTurnCallback
  ): Promise<string> {
    const stream = await this.prompt(input, {
      onEvent
    });

    let fullText = '';
    const reader = stream.getReader();

    while (true) {
      const { done, value } = await reader.read();
      if (done) break;
      if (value.type === 'text_delta' && typeof value.delta === 'string') {
        fullText += value.delta;
      }
    }

    return fullText;
  }

  /**
   * Cancels any active prompt turn currently executing on the agent.
   */
  async cancel(): Promise<void> {
    if (this.transport instanceof StdioTransport) {
      await this.transport.cancelSession();
    }
  }

  /**
   * Switches the active model identifier used for inference.
   *
   * @param model Model identifier (e.g. `'anthropic/claude-3.5-sonnet'`, `'deepseek/deepseek-chat'`)
   */
  async switchModel(model: string): Promise<void> {
    this.config.default_model = model;
    this.options.default_model = model;

    if (this.transport instanceof StdioTransport) {
      await this.transport.switchModel(model);
    } else if (this.transport instanceof WasmTransport) {
      await this.transport.switchModel(model);
    }
  }

  /**
   * Returns the list of tools supported by the agent engine and workspace.
   */
  async listTools(): Promise<ToolDefinition[]> {
    return [
      {
        name: 'read',
        description: 'Read file contents from the workspace virtual filesystem',
        parameters: {
          type: 'object',
          properties: {
            path: {
              type: 'string',
              description: 'Relative file path (e.g. "src/main.rs", "README.md")'
            }
          },
          required: ['path']
        }
      },
      {
        name: 'write',
        description: 'Write or overwrite file contents in the virtual filesystem',
        parameters: {
          type: 'object',
          properties: {
            path: {
              type: 'string',
              description: 'Relative destination file path'
            },
            content: {
              type: 'string',
              description: 'Full text content to write'
            }
          },
          required: ['path', 'content']
        }
      },
      {
        name: 'edit',
        description: 'Apply surgical line or syntax-aware replacements in a file',
        parameters: {
          type: 'object',
          properties: {
            path: {
              type: 'string',
              description: 'Path to target file'
            },
            input: {
              type: 'string',
              description: 'Patch DSL or replacements to apply'
            }
          },
          required: ['path', 'input']
        }
      },
      {
        name: 'grep',
        description: 'Search workspace files using regular expression patterns',
        parameters: {
          type: 'object',
          properties: {
            pattern: {
              type: 'string',
              description: 'Regular expression search pattern'
            },
            path: {
              type: 'string',
              description: 'Optional path or glob filter'
            }
          },
          required: ['pattern']
        }
      },
      {
        name: 'glob',
        description: 'Match workspace file paths against a pattern',
        parameters: {
          type: 'object',
          properties: {
            pattern: {
              type: 'string',
              description: 'Glob pattern (e.g. "**/*.rs", "src/**/*.ts")'
            }
          },
          required: ['pattern']
        }
      },
      {
        name: 'bash',
        description: 'Execute workspace terminal commands in sandboxed environment',
        parameters: {
          type: 'object',
          properties: {
            command: {
              type: 'string',
              description: 'Command line string to execute'
            }
          },
          required: ['command']
        }
      }
    ];
  }

  /**
   * Closes the active session and cleanly terminates the underlying transport.
   */
  async close(): Promise<void> {
    this.isInitialized = false;
    this.eventListeners.clear();
    await this.transport.disconnect();
  }

  /**
   * Subscribes a global event listener receiving all streaming agent events.
   *
   * @param listener Callback receiving real-time `AgentEvent`s
   * @returns Unsubscribe function
   */
  subscribe(listener: AgentEventCallback): () => void {
    this.eventListeners.add(listener);
    return () => {
      this.eventListeners.delete(listener);
    };
  }

  // ==========================================================================
  // Session & Virtual Filesystem Helpers
  // ==========================================================================

  /**
   * Returns the unique UUID string of the active session.
   */
  getSessionId(): string {
    if (this.transport instanceof StdioTransport) {
      return this.transport.sessionId || this.options.sessionId || 'session_stdio';
    }
    const wasmBindings = this.getRawBindings();
    return wasmBindings ? wasmBindings.get_session_id() : this.options.sessionId || 'session_default';
  }

  /**
   * Returns the currently active model identifier.
   */
  getActiveModel(): string {
    if (this.transport instanceof StdioTransport) {
      return this.transport.activeModel;
    }
    const wasmBindings = this.getRawBindings();
    return wasmBindings ? wasmBindings.get_active_model() : this.config.default_model || 'anthropic/claude-3.5-sonnet';
  }

  /**
   * Sets the active model identifier synchronously in local config.
   */
  setActiveModel(model: string): void {
    this.switchModel(model).catch(() => {});
  }

  /**
   * Sets a custom system prompt override for the session.
   */
  setSystemPrompt(prompt: string): void {
    this.config.system_prompt = prompt;
    const wasmBindings = this.getRawBindings();
    if (wasmBindings) {
      wasmBindings.set_system_prompt(prompt);
    }
  }

  /**
   * Returns the list of all conversation messages in the current session.
   */
  getMessages(): Message[] {
    const wasmBindings = this.getRawBindings();
    if (!wasmBindings) return [];
    try {
      return JSON.parse(wasmBindings.get_messages());
    } catch {
      return [];
    }
  }

  /**
   * Returns cumulative token usage statistics for the session.
   */
  getTokenStats(): TokenStats {
    const wasmBindings = this.getRawBindings();
    if (!wasmBindings) {
      return { prompt_tokens: 0, completion_tokens: 0, total_tokens: 0 };
    }
    try {
      return JSON.parse(wasmBindings.get_token_stats());
    } catch {
      return { prompt_tokens: 0, completion_tokens: 0, total_tokens: 0 };
    }
  }

  /**
   * Clears the message history while retaining the session VFS and configuration.
   */
  clearMessages(): void {
    const wasmBindings = this.getRawBindings();
    if (wasmBindings) {
      wasmBindings.clear_messages();
    }
  }

  /**
   * Reads content of a file from the agent's virtual filesystem.
   */
  fsRead(path: string): string {
    const wasmBindings = this.getRawBindings();
    if (!wasmBindings) {
      throw new Error('VFS operations require an active WASM agent instance.');
    }
    return wasmBindings.fs_read(path);
  }

  /**
   * Writes content to a file in the agent's virtual filesystem.
   */
  fsWrite(path: string, content: string): void {
    const wasmBindings = this.getRawBindings();
    if (!wasmBindings) {
      throw new Error('VFS operations require an active WASM agent instance.');
    }
    wasmBindings.fs_write(path, content);
  }

  /**
   * Returns a list of all file paths present in the virtual filesystem.
   */
  fsList(): string[] {
    const wasmBindings = this.getRawBindings();
    if (!wasmBindings) return [];
    try {
      return JSON.parse(wasmBindings.fs_list());
    } catch {
      return [];
    }
  }

  /**
   * Deletes a file from the virtual filesystem.
   */
  fsDelete(path: string): boolean {
    const wasmBindings = this.getRawBindings();
    if (!wasmBindings) return false;
    return wasmBindings.fs_delete(path);
  }

  /**
   * Creates a full JSON snapshot checkpoint of session state, VFS, and token statistics.
   */
  checkpoint(): string {
    const wasmBindings = this.getRawBindings();
    if (!wasmBindings) {
      throw new Error('Checkpoint creation requires an active WASM agent instance.');
    }
    return wasmBindings.checkpoint();
  }

  /**
   * Parses and returns the structured checkpoint object.
   */
  getCheckpointData(): CheckpointData {
    return JSON.parse(this.checkpoint());
  }

  /**
   * Restores session state, VFS files, and config from a checkpoint JSON string.
   */
  restore(checkpointJson: string): void {
    const wasmBindings = this.getRawBindings();
    if (!wasmBindings) {
      throw new Error('Checkpoint restoration requires an active WASM agent instance.');
    }
    wasmBindings.restore(checkpointJson);
  }

  /**
   * Returns the underlying raw wasm-bindgen object if running under WasmTransport.
   */
  getRawBindings(): WasmFusionAgentBindings | null {
    if (this.transport instanceof WasmTransport) {
      return this.transport.getRawBindings();
    }
    return null;
  }

  /**
   * Returns the active transport instance.
   */
  getTransport(): AgentTransport {
    return this.transport;
  }
}
