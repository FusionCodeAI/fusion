import * as fs from "node:fs";
import * as path from "node:path";
import { FusionAgent } from "@fusioncode/sdk";
import { DEFAULT_FUSION_MODEL } from "./models";
import type { TurnStep } from "../types";

export type BridgeEvent = "thought" | "chunk" | "step" | "diff" | "done" | "error";

export interface TurnDoneStats {
  tokens?: number;
  durationMs?: number;
}

export type BridgeEventCallbackMap = {
  thought: (thought: string) => void;
  chunk: (chunk: string) => void;
  step: (step: TurnStep) => void;
  diff: (diff: string) => void;
  done: (stats?: TurnDoneStats) => void;
  error: (error: Error) => void;
};

export interface AgentBridgeOptions {
  binaryPath?: string;
  cwd?: string;
  defaultModel?: string;
  agent?: FusionAgent;
}

/**
 * Resolves the location of the `fusion` binary across common dev and packaged paths.
 */
const fsRecord = fs as Record<string, any>;
const pathRecord = path as Record<string, any>;
const existsSync = (p: string): boolean => {
  try {
    return typeof fsRecord["existsSync"] === "function" ? Boolean(fsRecord["existsSync"](p)) : false;
  } catch {
    return false;
  }
};
const resolve = (...args: string[]): string =>
  typeof pathRecord["resolve"] === "function" ? pathRecord["resolve"](...args) : args.join("/");
const join = (...args: string[]): string =>
  typeof pathRecord["join"] === "function" ? pathRecord["join"](...args) : args.join("/");
const dirname = (p: string): string =>
  typeof pathRecord["dirname"] === "function" ? pathRecord["dirname"](p) : p;

export function resolveFusionBinary(customPath?: string): string | null {
  if (customPath !== undefined) {
    const trimmed = customPath.trim();
    if (!trimmed) return null;
    return existsSync(trimmed) ? resolve(trimmed) : null;
  }

  const envPath = process.env.FUSION_BINARY_PATH;
  if (envPath && existsSync(envPath)) {
    return resolve(envPath);
  }

  const packagedPath = join(dirname(process.execPath), "fusion");
  if (existsSync(packagedPath)) {
    return resolve(packagedPath);
  }

  const cwd = process.cwd();
  const baseDir = import.meta.dir;
  const candidates = [
    resolve(cwd, "../../target/release/fusion"),
    resolve(cwd, "../../target/debug/fusion"),
    resolve(cwd, "target/release/fusion"),
    resolve(cwd, "target/debug/fusion"),
    resolve(baseDir, "../../target/release/fusion"),
    resolve(baseDir, "../../target/debug/fusion"),
    resolve(baseDir, "../../../target/release/fusion"),
    resolve(baseDir, "../../../target/debug/fusion"),
  ];

  for (const candidate of candidates) {
    if (existsSync(candidate)) {
      return candidate;
    }
  }

  return null;
}

/**
 * Formats a human-readable title for tool calls matching Cursor / Cline aesthetics.
 * Examples: `Ran <cmd>`, `Read <file>`, `Edited <file>`, `Searched "<query>"`.
 */
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

  if (tool === "bash" || tool === "sh" || tool === "terminal" || tool === "exec" || tool === "command") {
    const cmd = String(args.command ?? args.cmd ?? "").trim();
    return cmd ? `Ran ${cmd}` : "Ran terminal command";
  }

  if (tool === "read" || tool === "read_file" || tool === "readfile" || tool === "cat") {
    const p = String(args.path ?? args.file ?? args.filePath ?? "").trim();
    const cleanPath = p.replace(/^[./\\]+/, "");
    return cleanPath ? `Read ${cleanPath}` : "Read file";
  }

  if (tool === "edit" || tool === "edit_file" || tool === "editfile" || tool === "patch") {
    const p = String(args.path ?? args.file ?? args.filePath ?? "").trim();
    const cleanPath = p.replace(/^[./\\]+/, "");
    return cleanPath ? `Edited ${cleanPath}` : "Edited file";
  }

  if (tool === "write" || tool === "write_file" || tool === "writefile") {
    const p = String(args.path ?? args.file ?? args.filePath ?? "").trim();
    const cleanPath = p.replace(/^[./\\]+/, "");
    return cleanPath ? `Wrote ${cleanPath}` : "Wrote file";
  }

  if (tool === "grep" || tool === "glob" || tool === "search" || tool === "web_search" || tool === "find") {
    const q = String(args.query ?? args.pattern ?? args.path ?? "").trim();
    return q ? `Searched "${q}"` : "Searched files";
  }

  return `Ran ${name}`;
}

/**
 * Returns true if the string is an internal status or spinner message that should be suppressed.
 */
export function isSpinnerString(text: string): boolean {
  const trimmed = text.trim().toLowerCase();
  if (!trimmed) return true;
  if (trimmed === "thinking..." || trimmed === "thinking") return true;
  if (trimmed.startsWith("waiting for model") || trimmed.startsWith("waiting for response")) return true;
  return /waiting for (?:model\s+)?response\.{0,3}$/i.test(trimmed);
}

/**
 * Parses DeepSeek DSML tool calls `<[|｜]DSML[|｜]...>`, generates clean TurnStep events,
 * and strips all DSML markup from chunks.
 */
export function parseAndStripDsml(text: string): { cleanText: string; steps: TurnStep[] } {
  const steps: TurnStep[] = [];
  if (!text) {
    return { cleanText: "", steps };
  }

  const hasDsml =
    text.includes("DSML") ||
    text.includes("<｜") ||
    text.includes("<|") ||
    text.includes("tool_calls") ||
    text.includes("call:");

  if (!hasDsml) {
    return { cleanText: text, steps };
  }

  // 1. Extract tool calls with parameters, e.g. <|DSML|call:grep><|DSML|parameter name="query">q</|DSML|parameter></|DSML|call>
  const callRegex = /<[|｜]DSML[|｜]call:?([a-zA-Z0-9_-]+)?>([\s\S]*?)<\/[|｜]DSML[|｜]call(?::?[a-zA-Z0-9_-]+)?>/g;
  let match: RegExpExecArray | null;

  while ((match = callRegex.exec(text)) !== null) {
    const toolName = match[1] || "tool";
    const body = match[2];

    const paramRegex = /<[|｜]DSML[|｜]parameter(?:\s+name=["']([^"']+)["'])?[^>]*>([\s\S]*?)<\/[|｜]DSML[|｜]parameter>/g;
    const args: Record<string, unknown> = {};
    let paramMatch: RegExpExecArray | null;
    while ((paramMatch = paramRegex.exec(body)) !== null) {
      const pName = paramMatch[1] ?? "query";
      args[pName] = paramMatch[2].trim();
    }

    const stepId = `step-${Date.now()}-${Math.random().toString(36).slice(2, 7)}`;
    const title = formatToolTitle(toolName, args);
    steps.push({
      id: stepId,
      title,
      status: "completed",
    });
  }

  // 2. Extract standalone parameter queries if no call block was matched: <|DSML|parameter name="query">search term</|DSML|parameter>
  if (steps.length === 0) {
    const queryRegex = /<[|｜]DSML[|｜]parameter(?:\s+name=["']([^"']+)["'])?[^>]*>([\s\S]*?)<\/[|｜]DSML[|｜]parameter>/g;
    let qMatch: RegExpExecArray | null;
    while ((qMatch = queryRegex.exec(text)) !== null) {
      const pName = qMatch[1] || "query";
      const val = qMatch[2].trim();
      if (val) {
        const stepId = `step-${Date.now()}-${Math.random().toString(36).slice(2, 7)}`;
        const title = pName === "command" || pName === "cmd"
          ? `Ran ${val}`
          : pName === "path" || pName === "file"
          ? `Read ${val.replace(/^[./\\]+/, "")}`
          : `Searched "${val}"`;
        steps.push({
          id: stepId,
          title,
          status: "completed",
        });
      }
    }
  }

  // 3. Strip all DSML tags, tool calls containers, parameter blocks, and thought markers
  let clean = text
    .replace(/<[|｜]DSML[|｜]tool_calls>[\s\S]*?<\/[|｜]DSML[|｜]tool_calls>/g, "")
    .replace(/<[|｜]DSML[|｜]call(?::[a-zA-Z0-9_-]+)?>[\s\S]*?<\/[|｜]DSML[|｜]call(?::[a-zA-Z0-9_-]+)?>/g, "")
    .replace(/<[|｜]DSML[|｜]parameter[^>]*>[\s\S]*?<\/[|｜]DSML[|｜]parameter>/g, "")
    .replace(/<[|｜]DSML[|｜][\s\S]*?[|｜]DSML[|｜]tool_calls>/g, "")
    .replace(/<[|｜]DSML[|｜][\s\S]*?>/g, "")
    .replace(/<\/[|｜]DSML[|｜][\s\S]*?>/g, "")
    .replace(/<\/?(tool_call|function_call|parameter)[^>]*>/g, "")
    .replace(/<[|｜](?:begin_of_thought|thought|end_of_thought)[|｜]>/g, "")
    .replace(/<[|｜]call:[^>]*>/g, "")
    .replace(/<\/[|｜]call[^>]*>/g, "");

  return { cleanText: clean, steps };
}

/**
 * High-level Agent Bridge wrapping `@fusioncode/sdk` (`FusionAgent`) and stdio transport.
 */
export class AgentBridge {
  private agent: FusionAgent;
  private isStarted: boolean = false;
  private activeModel: string;
  private listeners: Map<BridgeEvent, Set<Function>> = new Map();

  constructor(options?: AgentBridgeOptions) {
    this.activeModel = options?.defaultModel ?? DEFAULT_FUSION_MODEL.id;

    if (options?.agent) {
      this.agent = options.agent;
    } else {
      const resolvedBinary = resolveFusionBinary(options?.binaryPath) ?? options?.binaryPath ?? "fusion";
      this.agent = new FusionAgent({
        transport: "stdio",
        binaryPath: resolvedBinary,
        cwd: options?.cwd,
        default_model: this.activeModel,
      });
    }
  }

  /**
   * Initializes the agent transport and backend connection.
   */
  async start(): Promise<void> {
    if (this.isStarted) return;
    await this.agent.initialize();
    this.isStarted = true;
  }

  /**
   * Sends a prompt turn to the agent, optionally specifying a model.
   */
  async prompt(text: string, model?: string): Promise<void> {
    if (!this.isStarted) {
      await this.start();
    }

    const targetModel = model ?? this.activeModel;
    if (model && model !== this.activeModel) {
      this.activeModel = model;
      try {
        await this.agent.switchModel(model);
      } catch {
        // best-effort model switch
      }
    }

    try {
      let handledViaCallback = false;

      const stream = await this.agent.prompt(text, {
        model: targetModel,
        onEvent: (event) => {
          handledViaCallback = true;
          this.handleAgentEvent(event);
        },
      });

      if (stream && typeof stream.getReader === "function") {
        const reader = stream.getReader();
        try {
          while (true) {
            const { done, value } = await reader.read();
            if (done) break;
            if (!handledViaCallback && value) {
              this.handleAgentEvent(value);
            }
          }
        } finally {
          reader.releaseLock();
        }
      }

      this.emit("done", {});
    } catch (err) {
      const error = err instanceof Error ? err : new Error(String(err));
      this.emit("error", error);
      throw error;
    }
  }

  /**
   * Cancels any currently executing prompt turn.
   */
  async cancel(): Promise<void> {
    await this.agent.cancel();
  }

  /**
   * Registers an event listener for agent lifecycle events.
   * Supported events: "thought" | "chunk" | "step" | "diff" | "done" | "error"
   * Returns an unsubscribe function.
   */
  on<E extends BridgeEvent>(event: E, cb: BridgeEventCallbackMap[E] | Function): () => void {
    let set = this.listeners.get(event);
    if (!set) {
      set = new Set();
      this.listeners.set(event, set);
    }
    set.add(cb);
    return () => {
      set.delete(cb);
    };
  }

  /**
   * Emits an event to registered listeners.
   */
  emit(event: BridgeEvent, payload?: unknown): void {
    const set = this.listeners.get(event);
    if (!set) return;
    for (const listener of set) {
      try {
        listener(payload);
      } catch (err) {
        console.error(`[AgentBridge] Error in '${event}' event listener:`, err);
      }
    }
  }

  handleAgentEvent(event: unknown): void {
    if (!event || typeof event !== "object") return;

    const record = event as Record<string, unknown>;
    const type = typeof record.type === "string" ? record.type : typeof record.kind === "string" ? record.kind : undefined;

    switch (type) {
      case "text_delta": {
        const text = typeof record.delta === "string" ? record.delta : typeof record.text === "string" ? record.text : "";
        if (!text) break;
        if (isSpinnerString(text)) break;

        const { cleanText, steps } = parseAndStripDsml(text);
        for (const step of steps) {
          this.emit("step", step);
        }

        if (cleanText && cleanText.trim().length > 0 && !isSpinnerString(cleanText)) {
          this.emit("chunk", cleanText);
        }
        break;
      }

      case "thinking_delta": {
        const thought = typeof record.delta === "string" ? record.delta : typeof record.thought === "string" ? record.thought : "";
        if (!thought) break;
        if (isSpinnerString(thought)) break;
        this.emit("thought", thought);
        break;
      }

      case "tool_started": {
        const id = typeof record.id === "string" ? record.id : `step-${Date.now()}`;
        const name = typeof record.name === "string" ? record.name : "tool";
        const args = record.args;
        const title = formatToolTitle(name, args);

        this.emit("step", {
          id,
          title,
          status: "running",
          timestamp: Date.now(),
        });
        break;
      }

      case "tool_finished": {
        const id = typeof record.id === "string" ? record.id : `step-${Date.now()}`;
        const name = typeof record.name === "string" ? record.name : "tool";
        const success = record.success !== false;
        const output = typeof record.output === "string" ? record.output : "";
        const args = record.args;
        const title = formatToolTitle(name, args);

        this.emit("step", {
          id,
          title,
          status: success ? "completed" : "failed",
          details: output,
          timestamp: Date.now(),
        });

        // Check if output contains a diff patch
        if (
          output.includes("--- a/") ||
          output.includes("+++ b/") ||
          output.startsWith("diff --git") ||
          output.includes("@@ -")
        ) {
          this.emit("diff", output);
        }
        break;
      }

      case "diff": {
        const patch = typeof record.patch === "string" ? record.patch : typeof record.diff === "string" ? record.diff : "";
        if (patch) {
          this.emit("diff", patch);
        }
        break;
      }

      case "finished": {
        const usage = record.usage as { total_tokens?: number } | undefined;
        this.emit("done", { tokens: usage?.total_tokens });
        break;
      }

      case "error": {
        const message = typeof record.message === "string" ? record.message : "Agent error";
        this.emit("error", new Error(message));
        break;
      }
    }
  }
}
