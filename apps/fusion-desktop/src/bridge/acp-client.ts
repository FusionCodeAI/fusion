import { spawn as cpSpawn, type ChildProcess } from "node:child_process";
import { existsSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import type { Readable, Writable } from "node:stream";

export interface JsonRpcErrorObject {
  code: number;
  message: string;
  data?: unknown;
}

export interface JsonRpcMessage {
  jsonrpc: "2.0";
  id?: string | number | null;
  method?: string;
  params?: Record<string, unknown> | unknown[];
  result?: unknown;
  error?: JsonRpcErrorObject;
}

export function parseJsonRpcLine(line: string): JsonRpcMessage | null {
  const trimmed = line.trim();
  if (!trimmed) {
    return null;
  }

  try {
    const parsed: unknown = JSON.parse(trimmed);
    if (!parsed || typeof parsed !== "object" || Array.isArray(parsed)) {
      return null;
    }

    const record = parsed as Record<string, unknown>;
    if (record.jsonrpc !== "2.0") {
      return null;
    }

    const msg: JsonRpcMessage = { jsonrpc: "2.0" };

    if ("id" in record && record.id !== undefined) {
      const id = record.id;
      if (typeof id === "string" || typeof id === "number" || id === null) {
        msg.id = id;
      }
    }

    if ("method" in record && typeof record.method === "string") {
      msg.method = record.method;
    }

    if ("params" in record && record.params !== undefined) {
      if (typeof record.params === "object" && record.params !== null) {
        msg.params = record.params as Record<string, unknown> | unknown[];
      }
    }

    if ("result" in record && record.result !== undefined) {
      msg.result = record.result;
    }

    if ("error" in record && record.error !== undefined) {
      if (typeof record.error === "object" && record.error !== null) {
        const errObj = record.error as Record<string, unknown>;
        if (typeof errObj.code === "number" && typeof errObj.message === "string") {
          msg.error = {
            code: errObj.code,
            message: errObj.message,
            ...(errObj.data !== undefined ? { data: errObj.data } : {}),
          };
        }
      }
    }

    return msg;
  } catch {
    return null;
  }
}

export interface ResolveBinaryOptions {
  cwd?: string;
  baseDir?: string;
  execPath?: string;
  env?: Record<string, string | undefined>;
  existsSync?: (path: string) => boolean;
}

export function resolveFusionBinary(
  customPath?: string,
  options?: ResolveBinaryOptions
): string | null {
  const checkExists = options?.existsSync ?? existsSync;

  if (customPath !== undefined) {
    const trimmed = customPath.trim();
    if (!trimmed) {
      return null;
    }
    return checkExists(trimmed) ? resolve(trimmed) : null;
  }

  const env = options?.env ?? process.env;
  const envPath = env.FUSION_BINARY_PATH;
  if (envPath && checkExists(envPath)) {
    return resolve(envPath);
  }

  const execPath = options?.execPath ?? process.execPath;
  const packagedPath = join(dirname(execPath), "fusion");
  if (checkExists(packagedPath)) {
    return resolve(packagedPath);
  }

  const cwd = options?.cwd ?? process.cwd();
  const baseDir = options?.baseDir ?? import.meta.dir;

  const devCandidates = [
    resolve(cwd, "../../target/release/fusion"),
    resolve(cwd, "../../target/debug/fusion"),
    resolve(cwd, "target/release/fusion"),
    resolve(cwd, "target/debug/fusion"),
    resolve(baseDir, "../../target/release/fusion"),
    resolve(baseDir, "../../target/debug/fusion"),
    resolve(baseDir, "../../../target/release/fusion"),
    resolve(baseDir, "../../../target/debug/fusion"),
  ];

  for (const candidate of devCandidates) {
    if (checkExists(candidate)) {
      return resolve(candidate);
    }
  }

  return null;
}

export function formatToolTitle(name: string, argsRaw?: unknown): string {
  let args: Record<string, unknown> = {};
  if (typeof argsRaw === "string") {
    try {
      args = JSON.parse(argsRaw);
    } catch {
      return `${name}: ${argsRaw.slice(0, 32)}`;
    }
  } else if (typeof argsRaw === "object" && argsRaw !== null) {
    args = argsRaw as Record<string, unknown>;
  }

  const tool = name.toLowerCase();
  if (tool === "bash") {
    const cmd = String(args.command ?? args.cmd ?? "").trim();
    const truncated = cmd.length > 36 ? cmd.slice(0, 34) + "..." : cmd;
    return truncated ? `Ran ${truncated}` : "Ran terminal command";
  }
  if (tool === "read" || tool === "read_file") {
    const p = String(args.path ?? args.file ?? "");
    const shortPath = p.replace(/^[./\\]+/, "");
    return shortPath ? `Read ${shortPath}` : "Read file";
  }
  if (tool === "edit" || tool === "edit_file") {
    const p = String(args.path ?? args.file ?? "");
    const shortPath = p.replace(/^[./\\]+/, "");
    return shortPath ? `Edited ${shortPath}` : "Edited file";
  }
  if (tool === "write") {
    const p = String(args.path ?? args.file ?? "");
    const shortPath = p.replace(/^[./\\]+/, "");
    return shortPath ? `Wrote ${shortPath}` : "Created file";
  }
  if (tool === "grep" || tool === "glob" || tool === "search") {
    const q = String(args.pattern ?? args.query ?? args.path ?? "");
    return q ? `Searched "${q.slice(0, 24)}"` : "Searched files";
  }
  return `Ran ${name}`;
}
export interface TurnStepEvent {
  title: string;
  status: "running" | "completed" | "failed";
  details?: string;
}
export interface TurnDoneEvent {
  tokens?: number;
  durationMs?: number;
}
export interface AcpClientOptions {
  binaryPath?: string;
  args?: string[];
  env?: Record<string, string>;
  workspaceDir?: string;
  sessionId?: string;
  defaultTimeoutMs?: number;
}

interface PendingRequest {
  resolve: (val: unknown) => void;
  reject: (err: Error) => void;
  timer?: NodeJS.Timeout;
  method: string;
}

export class AcpClient {
  private options: AcpClientOptions;
  private child: ChildProcess | null = null;
  get childProcess(): ChildProcess | null {
    return this.child;
  }

  set childProcess(proc: ChildProcess | null) {
    this.child = proc;
  }
  private stdinStream: Writable | null = null;
  private stdoutStream: Readable | null = null;
  private streamsConnected = false;
  private customStreamsActive = false;
  private childExited = false;

  private nextRequestId = 0;
  private pendingRequests = new Map<string | number, PendingRequest>();
  private buffer = "";

  private stepListeners = new Set<(step: TurnStepEvent) => void>();
  private thoughtListeners = new Set<(delta: string) => void>();
  private chunkListeners = new Set<(delta: string) => void>();
  private doneListeners = new Set<(info?: TurnDoneEvent) => void>();
  private errorListeners = new Set<(err: Error) => void>();
  private exitListeners = new Set<(code: number | null) => void>();
  private toolCallsArgs = new Map<string, unknown>();

  public sessionId: string;

  constructor(options: AcpClientOptions = {}) {
    this.options = options;
    this.sessionId = options.sessionId ?? `session-${Date.now()}`;
  }

  attachStreams(stdout: Readable, stdin: Writable): void {
    this.stdoutStream = stdout;
    this.stdinStream = stdin;
    this.customStreamsActive = true;
    this.streamsConnected = true;

    stdout.on("data", (chunk: Buffer | string) => {
      this.feedData(chunk);
    });

    stdout.on("error", (err: Error) => {
      this.emitError(err);
    });

    stdin.on("error", (err: unknown) => {
      this.emitError(err instanceof Error ? err : new Error(String(err)));
    });

    stdout.on("close", () => {
      this.streamsConnected = false;
      this.cleanupPending(new Error("Stream closed"));
      const childAlreadyExited = Boolean(
        this.childExited ||
        (this.child && (this.child.exitCode !== null || this.child.signalCode !== null))
      );
      if (!childAlreadyExited) {
        this.emitExit(null);
      }
    });
  }

  async spawn(workspaceDir?: string): Promise<boolean> {
    if (this.isAlive()) {
      const existingChild = this.child;
      this.kill();
      if (
        existingChild &&
        existingChild.exitCode === null &&
        existingChild.signalCode === null &&
        typeof existingChild.once === "function"
      ) {
        await new Promise<void>((resolve) => {
          let finished = false;
          const finish = () => {
            if (!finished) {
              finished = true;
              clearTimeout(timer);
              resolve();
            }
          };
          const timer = setTimeout(() => {
            try {
              if (
                typeof existingChild.kill === "function" &&
                (!existingChild.killed ||
                  (existingChild.exitCode === null &&
                    existingChild.signalCode === null))
              ) {
                existingChild.kill("SIGKILL");
              }
            } catch {
              // ignore
            }
            finish();
          }, 500);

          existingChild.once("exit", finish);
        });
      }
    }

    const effectiveCwd = workspaceDir || this.options.workspaceDir;
    const binPath = resolveFusionBinary(this.options.binaryPath);
    if (!binPath) {
      this.emitError(new Error("Fusion binary not found"));
      return false;
    }

    const args = [...(this.options.args ?? ["--acp"])];
    if (effectiveCwd && !args.includes("--cwd")) {
      args.push("--cwd", effectiveCwd);
    }

    try {
      this.child = cpSpawn(binPath, args, {
        cwd: effectiveCwd || process.cwd(),
        env: { ...process.env, ...this.options.env },
        stdio: ["pipe", "pipe", "pipe"],
      });

      if (!this.child.stdout || !this.child.stdin) {
        this.emitError(new Error("Failed to pipe child process stdio"));
        return false;
      }

      this.childExited = false;
      this.attachStreams(this.child.stdout, this.child.stdin);
      this.customStreamsActive = false;

      const proc = this.child;

      proc.on("error", (err: Error) => {
        if (this.child === proc) {
          this.emitError(err);
        }
      });

      proc.on("exit", (code: number | null) => {
        if (this.child === proc) {
          this.childExited = true;
          this.streamsConnected = false;
          this.cleanupPending(`ACP client exited with code ${code}`);
          this.emitExit(code);
        }
      });

      return true;
    } catch (err) {
      const error = err instanceof Error ? err : new Error(String(err));
      this.emitError(error);
      return false;
    }
  }

  sendRequest<T = unknown>(
    method: string,
    params?: Record<string, unknown>,
    timeoutMs?: number
  ): Promise<T> {
    if (!this.isAlive()) {
      return Promise.reject(new Error("ACP client is not connected"));
    }

    const id = ++this.nextRequestId;
    const effectiveTimeout = timeoutMs ?? this.options.defaultTimeoutMs ?? 30000;

    return new Promise<T>((resolveReq, rejectReq) => {
      let timer: NodeJS.Timeout | undefined;
      if (effectiveTimeout > 0) {
        timer = setTimeout(() => {
          this.pendingRequests.delete(id);
          rejectReq(
            new Error(
              `Request ${method} (id: ${id}) timed out after ${effectiveTimeout}ms`
            )
          );
        }, effectiveTimeout);
      }

      this.pendingRequests.set(id, {
        resolve: (val: unknown) => resolveReq(val as T),
        reject: rejectReq,
        timer,
        method,
      });

      const payload: Record<string, unknown> = {
        jsonrpc: "2.0",
        id,
        method,
      };
      if (params !== undefined) {
        payload.params = params;
      }

      this.writeToStdin(JSON.stringify(payload) + "\n");
    });
  }

  sendNotification(method: string, params?: Record<string, unknown>): void {
    if (!this.isAlive()) {
      this.emitError(new Error("Cannot send notification: ACP client is not connected"));
      return;
    }

    const payload: Record<string, unknown> = {
      jsonrpc: "2.0",
      method,
    };
    if (params !== undefined) {
      payload.params = params;
    }

    this.writeToStdin(JSON.stringify(payload) + "\n");
  }

  async prompt(text: string, model?: string): Promise<void> {
    const params: Record<string, unknown> = {
      prompt: text,
      ...(model ? { model } : {}),
      ...(this.sessionId ? { sessionId: this.sessionId } : {}),
    };
    await this.sendRequest("session/prompt", params);
  }

  async cancel(): Promise<void> {
    const params: Record<string, unknown> = {
      ...(this.sessionId ? { sessionId: this.sessionId } : {}),
    };
    await this.sendRequest("session/cancel", params);
  }

  kill(): void {
    if (this.child && !this.child.killed && typeof this.child.kill === "function") {
      this.child.kill();
    }
    this.streamsConnected = false;
    this.cleanupPending("ACP client process killed");
  }

  isAlive(): boolean {
    if (this.customStreamsActive) {
      return this.streamsConnected;
    }
    return (
      this.childProcess !== null &&
      !this.childProcess.killed &&
      this.childProcess.exitCode === null &&
      this.childProcess.signalCode === null &&
      this.streamsConnected
    );
  }

  onStep(
    cb: (step: TurnStepEvent) => void
  ): () => void {
    this.stepListeners.add(cb);
    return () => {
      this.stepListeners.delete(cb);
    };
  }

  onChunk(cb: (delta: string) => void): () => void {
    this.chunkListeners.add(cb);
    return () => {
      this.chunkListeners.delete(cb);
    };
  }
  onThought(cb: (delta: string) => void): () => void {
    this.thoughtListeners.add(cb);
    return () => {
      this.thoughtListeners.delete(cb);
    };
  }
  onDone(cb: (info?: TurnDoneEvent) => void): () => void {
    this.doneListeners.add(cb);
    return () => {
      this.doneListeners.delete(cb);
    };
  }

  onError(cb: (err: Error) => void): () => void {
    this.errorListeners.add(cb);
    return () => {
      this.errorListeners.delete(cb);
    };
  }

  onExit(cb: (code: number | null) => void): () => void {
    this.exitListeners.add(cb);
    return () => {
      this.exitListeners.delete(cb);
    };
  }

  feedData(chunk: string | Buffer | Uint8Array): void {
    const str = typeof chunk === "string" ? chunk : Buffer.from(chunk).toString("utf-8");
    this.buffer += str;
    const lines = this.buffer.split("\n");
    this.buffer = lines.pop() ?? "";
    for (const line of lines) {
      if (line.trim().length > 0) {
        this.handleLine(line);
      }
    }
  }

  handleLine(line: string): void {
    const msg = parseJsonRpcLine(line);
    if (!msg) {
      return;
    }

    if (msg.id !== undefined && msg.id !== null) {
      const pending = this.pendingRequests.get(msg.id);
      if (pending) {
        this.pendingRequests.delete(msg.id);
        clearTimeout(pending.timer);
        if (msg.error) {
          pending.reject(
            new Error(msg.error.message || `RPC Error code ${msg.error.code}`)
          );
        } else {
          pending.resolve(msg.result);
        }
        return;
      }
    }

    if (msg.method) {
      this.routeNotification(msg.method, msg.params);
    }
  }

  private writeToStdin(data: string): void {
    if (this.stdinStream && !this.stdinStream.destroyed) {
      this.stdinStream.write(data);
    } else if (this.child?.stdin && !this.child.stdin.destroyed) {
      this.child.stdin.write(data);
    } else {
      this.emitError(new Error("Cannot write to ACP client stdin: stream closed"));
    }
  }

  private routeNotification(method: string, rawParams?: unknown): void {
    const params = (rawParams && typeof rawParams === "object" && !Array.isArray(rawParams)
      ? rawParams
      : {}) as Record<string, unknown>;

    switch (method) {
      case "turn/step": {
        const stepObj = (params.step && typeof params.step === "object"
          ? params.step
          : params) as Record<string, unknown>;
        const title = typeof stepObj.title === "string" ? stepObj.title : "Step";
        const statusRaw = stepObj.status;
        const status: "running" | "completed" | "failed" =
          statusRaw === "completed" || statusRaw === "failed" ? statusRaw : "running";
        const details = typeof stepObj.details === "string" ? stepObj.details : undefined;
        this.emitStep({ title, status, details });
        break;
      }
      case "turn/chunk": {
        const delta = (params.delta ?? params.chunk ?? params.text ?? "") as string;
        if (typeof delta === "string" && delta.length > 0) {
          this.emitChunk(delta);
        }
        break;
      }
      case "turn/done": {
        const infoObj = (params.info && typeof params.info === "object"
          ? params.info
          : params) as Record<string, unknown>;
        const tokens = typeof infoObj.tokens === "number" ? infoObj.tokens : undefined;
        const durationMs = typeof infoObj.durationMs === "number" ? infoObj.durationMs : undefined;
        this.emitDone({ tokens, durationMs });
        break;
      }
      case "session/update": {
        this.routeSessionUpdate(params);
        break;
      }
      case "error": {
        const message = typeof params.message === "string" ? params.message : "RPC Error";
        this.emitError(new Error(message));
        break;
      }
    }
  }

  private routeSessionUpdate(params: Record<string, unknown>): void {
    const update = (params.update && typeof params.update === "object"
      ? params.update
      : params) as Record<string, unknown>;
    const kind = (update.sessionUpdate ?? update.kind) as string | undefined;

    switch (kind) {
      case "agent_message_chunk": {
        const content = update.content as Record<string, unknown> | undefined;
        const text = (content?.text ?? update.delta ?? update.text ?? "") as string;
        if (text) {
          this.emitChunk(text);
        }
        break;
      }
      case "agent_thought_chunk": {
        const content = update.content as Record<string, unknown> | undefined;
        const thought = (update.thought ?? content?.text ?? "") as string;
        if (thought) {
          this.emitThought(thought);
        }
        break;
      }
      case "tool_call": {
        const name = typeof update.name === "string" ? update.name : "tool";
        const callId = typeof update.callId === "string" ? update.callId : "";
        if (callId && update.args) {
          this.toolCallsArgs.set(callId, update.args);
        }
        const title = formatToolTitle(name, update.args);
        const args = typeof update.args === "string" ? update.args : JSON.stringify(update.args);
        this.emitStep({
          title,
          status: "running",
          details: args,
        });
        break;
      }
      case "tool_call_result": {
        const name = typeof update.name === "string" ? update.name : "tool";
        const callId = typeof update.callId === "string" ? update.callId : "";
        const args = update.args ?? (callId ? this.toolCallsArgs.get(callId) : undefined);
        const success = update.success !== false;
        const title = formatToolTitle(name, args);
        let details: string | undefined;
        if (typeof update.output === "string") {
          details = update.output;
        } else if (typeof update.output === "object" && update.output !== null) {
          details = JSON.stringify(update.output);
        } else if (typeof update.error === "object" && update.error !== null) {
          details = JSON.stringify(update.error);
        } else if (update.output !== undefined && update.output !== null) {
          details = String(update.output);
        } else if (update.error !== undefined && update.error !== null) {
          details = String(update.error);
        }
        this.emitStep({
          title,
          status: success ? "completed" : "failed",
          details,
        });
        break;
      }
      case "token_stats": {
        const totalTokens = typeof update.totalTokens === "number" ? update.totalTokens : undefined;
        const durationMs = typeof update.durationMs === "number" ? update.durationMs : undefined;
        this.emitDone({ tokens: totalTokens, durationMs });
        break;
      }
      case "status": {
        const message = typeof update.message === "string" ? update.message : "Status";
        const level = update.level as string | undefined;
        this.emitStep({
          title: message,
          status: level === "error" ? "failed" : "running",
        });
        break;
      }
    }
  }

  private emitStep(step: TurnStepEvent): void {
    for (const cb of this.stepListeners) {
      try {
        cb(step);
      } catch (err) {
        this.emitError(err instanceof Error ? err : new Error(String(err)));
      }
    }
  }

  private emitThought(delta: string): void {
    for (const cb of this.thoughtListeners) {
      try {
        cb(delta);
      } catch (err) {
        this.emitError(err instanceof Error ? err : new Error(String(err)));
      }
    }
  }

  private emitChunk(delta: string): void {
    for (const cb of this.chunkListeners) {
      try {
        cb(delta);
      } catch (err) {
        this.emitError(err instanceof Error ? err : new Error(String(err)));
      }
    }
  }

  private emitDone(info?: TurnDoneEvent): void {
    for (const cb of this.doneListeners) {
      try {
        cb(info);
      } catch (err) {
        this.emitError(err instanceof Error ? err : new Error(String(err)));
      }
    }
  }

  private emitError(err: Error): void {
    if (this.errorListeners.size > 0) {
      for (const cb of this.errorListeners) {
        try {
          cb(err);
        } catch {
          // ignore nested error
        }
      }
    }
  }

  private emitExit(code: number | null): void {
    for (const cb of this.exitListeners) {
      try {
        cb(code);
      } catch (err) {
        this.emitError(err instanceof Error ? err : new Error(String(err)));
      }
    }
  }

  private cleanupPending(reason: string | Error): void {
    const error = reason instanceof Error ? reason : new Error(reason);
    for (const pending of this.pendingRequests.values()) {
      clearTimeout(pending.timer);
      pending.reject(error);
    }
    this.pendingRequests.clear();
  }
}
