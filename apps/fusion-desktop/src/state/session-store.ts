import { existsSync } from "node:fs";
import { mkdir, readFile, rename, rm, writeFile } from "node:fs/promises";
import { homedir } from "node:os";
import { dirname, join } from "node:path";
import { randomUUID } from "node:crypto";
import type { ChatMessage, ChatSession, TurnStep } from "../types";

export interface SessionStoreSnapshot {
  readonly sessions: readonly ChatSession[];
  readonly activeSessionId: string | null;
  readonly workspaceDir: string;
  readonly selectedModel: string;
  readonly isGenerating: boolean;
}

export interface SessionStoreOptions {
  storagePath?: string;
  autoSave?: boolean;
  workspaceDir?: string;
  selectedModel?: string;
}

interface PersistedSessionData {
  version: number;
  sessions: ChatSession[];
  activeSessionId: string | null;
  workspaceDir?: string;
  selectedModel?: string;
}

const VALID_ROLES: Record<string, true> = {
  user: true,
  assistant: true,
  system: true,
};

export function isValidMessage(candidate: unknown): candidate is ChatMessage {
  if (!candidate || typeof candidate !== "object") {
    return false;
  }
  const msg = candidate as Record<string, unknown>;
  if (
    typeof msg.id !== "string" ||
    typeof msg.role !== "string" ||
    !VALID_ROLES[msg.role] ||
    typeof msg.content !== "string" ||
    typeof msg.timestamp !== "number"
  ) {
    return false;
  }

  if (msg.steps !== undefined) {
    if (!Array.isArray(msg.steps)) {
      return false;
    }
    for (const step of msg.steps) {
      if (!step || typeof step !== "object") {
        return false;
      }
      const s = step as Record<string, unknown>;
      if (
        typeof s.id !== "string" ||
        typeof s.title !== "string" ||
        typeof s.status !== "string" ||
        !["running", "completed", "failed"].includes(s.status) ||
        typeof s.timestamp !== "number"
      ) {
        return false;
      }
    }
  }

  return true;
}

export function isValidSession(candidate: unknown): candidate is ChatSession {
  if (!candidate || typeof candidate !== "object") {
    return false;
  }
  const session = candidate as Record<string, unknown>;
  return (
    typeof session.id === "string" &&
    typeof session.title === "string" &&
    typeof session.createdAt === "number" &&
    typeof session.updatedAt === "number" &&
    typeof session.workspaceDir === "string" &&
    typeof session.model === "string" &&
    Array.isArray(session.messages) &&
    session.messages.every(isValidMessage)
  );
}

export class SessionStore {
  public sessions: ChatSession[] = [];
  public activeSessionId: string | null = null;
  public workspaceDir: string;
  public selectedModel: string;
  public isGenerating = false;
  public autoSave: boolean;
  public readonly storagePath: string;

  private listeners = new Set<() => void>();
  private snapshot!: SessionStoreSnapshot;

  private savePromise: Promise<void> | null = null;
  private pendingWaiters: Array<{
    resolve: () => void;
    reject: (err: unknown) => void;
  }> = [];
  private lastSaveError: unknown = null;
  private needsSave = false;

  constructor(options: SessionStoreOptions = {}) {
    this.storagePath = options.storagePath ?? join(homedir(), ".fusion", "desktop-sessions.json");
    this.autoSave = options.autoSave ?? false;
    this.workspaceDir = options.workspaceDir ?? process.cwd();
    this.selectedModel = options.selectedModel ?? "claude-3-5-sonnet";

    this.updateSnapshot();
  }

  // --- React useSyncExternalStore Integration ---

  subscribe = (listener: () => void): (() => void) => {
    this.listeners.add(listener);
    return () => {
      this.listeners.delete(listener);
    };
  };

  getSnapshot = (): SessionStoreSnapshot => {
    return this.snapshot;
  };
  private updateSnapshot(): void {
    this.snapshot = Object.freeze({
      sessions: Object.freeze(
        this.sessions.map((s) =>
          Object.freeze({
            ...s,
            messages: Object.freeze(
              s.messages.map((m) =>
                Object.freeze({
                  ...m,
                  steps: m.steps
                    ? Object.freeze(m.steps.map((step) => Object.freeze({ ...step })))
                    : undefined,
                })
              )
            ),
          })
        )
      ),
      activeSessionId: this.activeSessionId,
      workspaceDir: this.workspaceDir,
      selectedModel: this.selectedModel,
      isGenerating: this.isGenerating,
    }) as unknown as SessionStoreSnapshot;
  }

  private notify(): void {
    for (const listener of this.listeners) {
      try {
        listener();
      } catch (err: unknown) {
        console.error("[SessionStore] Listener callback threw:", err);
      }
    }
  }

  // --- Lifecycle & Mutation Methods ---

  createSession(title?: string, workspaceDir?: string, model?: string): ChatSession {
    const now = Date.now();
    const session: ChatSession = {
      id: randomUUID(),
      title: title?.trim() || "New Session",
      createdAt: now,
      updatedAt: now,
      workspaceDir: workspaceDir || this.workspaceDir,
      model: model || this.selectedModel,
      messages: [],
    };

    this.sessions.push(session);
    this.activeSessionId = session.id;

    this.updateSnapshot();
    this.notify();
    this.triggerAutoSave();
    return session;
  }

  selectSession(id: string): ChatSession | null {
    const target = this.sessions.find((s) => s.id === id);
    if (!target) {
      return null;
    }

    if (this.activeSessionId !== id) {
      this.activeSessionId = id;
      this.updateSnapshot();
      this.notify();
      this.triggerAutoSave();
    }
    return target;
  }

  getActiveSession(): ChatSession | null {
    if (!this.activeSessionId) {
      return null;
    }
    return this.sessions.find((s) => s.id === this.activeSessionId) ?? null;
  }

  deleteSession(id: string): boolean {
    const index = this.sessions.findIndex((s) => s.id === id);
    if (index === -1) {
      return false;
    }

    this.sessions.splice(index, 1);
    if (this.activeSessionId === id) {
      this.activeSessionId = this.sessions.length > 0 ? this.sessions[0].id : null;
    }

    this.updateSnapshot();
    this.notify();
    this.triggerAutoSave();
    return true;
  }

  updateSessionTitle(id: string, title: string): boolean {
    const target = this.sessions.find((s) => s.id === id);
    if (!target) {
      return false;
    }

    target.title = title;
    target.updatedAt = Date.now();

    this.updateSnapshot();
    this.notify();
    this.triggerAutoSave();
    return true;
  }

  appendUserMessage(content: string, sessionId?: string): ChatMessage {
    const session = this.resolveSession(sessionId);
    const message: ChatMessage = {
      id: randomUUID(),
      role: "user",
      content,
      timestamp: Date.now(),
    };

    session.messages.push(message);
    session.updatedAt = Date.now();

    this.updateSnapshot();
    this.notify();
    this.triggerAutoSave();
    return message;
  }

  appendAssistantChunk(delta: string, sessionId?: string): ChatMessage {
    const session = this.resolveSession(sessionId);
    const lastMessage = session.messages[session.messages.length - 1];
    let targetMessage: ChatMessage;

    if (lastMessage && lastMessage.role === "assistant") {
      targetMessage = {
        ...lastMessage,
        content: lastMessage.content + delta,
        timestamp: Date.now(),
        steps: lastMessage.steps ? [...lastMessage.steps] : undefined,
      };
      session.messages[session.messages.length - 1] = targetMessage;
    } else {
      targetMessage = {
        id: randomUUID(),
        role: "assistant",
        content: delta,
        timestamp: Date.now(),
        steps: [],
      };
      session.messages.push(targetMessage);
    }

    session.updatedAt = Date.now();
    this.updateSnapshot();
    this.notify();
    this.triggerAutoSave();
    return targetMessage;
  }

  appendThoughtChunk(delta: string, sessionId?: string): void {
    const session = this.resolveSession(sessionId);
    const lastMessage = session.messages[session.messages.length - 1];
    const now = Date.now();

    if (lastMessage && lastMessage.role === "assistant") {
      session.messages[session.messages.length - 1] = {
        ...lastMessage,
        thought: (lastMessage.thought || "") + delta,
        timestamp: now,
      };
    } else {
      session.messages.push({
        id: randomUUID(),
        role: "assistant",
        content: "",
        thought: delta,
        timestamp: now,
      });
    }

    session.updatedAt = now;
    this.updateSnapshot();
    this.notify();
    this.triggerAutoSave();
  }

  appendThoughtStep(step: TurnStep | string, sessionId?: string): void {
    const session = this.resolveSession(sessionId);
    const now = Date.now();
    const turnStep: TurnStep =
      typeof step === "string"
        ? {
            id: randomUUID(),
            title: step,
            status: "running",
            timestamp: now,
          }
        : { ...step };

    const lastMessage = session.messages[session.messages.length - 1];
    let assistantMessage: ChatMessage;
    const stepSummary = turnStep.title + (turnStep.details ? `\n${turnStep.details}` : "");

    if (lastMessage && lastMessage.role === "assistant") {
      const existingSteps = lastMessage.steps ? [...lastMessage.steps] : [];
      existingSteps.push(turnStep);
      const thought = lastMessage.thought
        ? `${lastMessage.thought}\n${stepSummary}`
        : stepSummary;

      assistantMessage = {
        ...lastMessage,
        thought,
        steps: existingSteps,
        timestamp: now,
      };
      session.messages[session.messages.length - 1] = assistantMessage;
    } else {
      assistantMessage = {
        id: randomUUID(),
        role: "assistant",
        content: "",
        thought: stepSummary,
        steps: [turnStep],
        timestamp: now,
      };
      session.messages.push(assistantMessage);
    }

    session.updatedAt = now;
    this.updateSnapshot();
    this.notify();
    this.triggerAutoSave();
  }

  updateTurnStep(
    stepId: string,
    status: "completed" | "failed",
    details?: string,
    sessionId?: string
  ): void {
    const session = this.resolveSession(sessionId);
    for (let i = session.messages.length - 1; i >= 0; i--) {
      const msg = session.messages[i];
      if (msg.steps) {
        const stepIndex = msg.steps.findIndex((s) => s.id === stepId);
        if (stepIndex !== -1) {
          const oldStep = msg.steps[stepIndex];
          const updatedStep: TurnStep = {
            ...oldStep,
            status,
            ...(details !== undefined ? { details } : {}),
          };
          const updatedSteps = [...msg.steps];
          updatedSteps[stepIndex] = updatedStep;
          const updatedMsg: ChatMessage = {
            ...msg,
            steps: updatedSteps,
          };
          session.messages[i] = updatedMsg;
          session.updatedAt = Date.now();
          this.updateSnapshot();
          this.notify();
          this.triggerAutoSave();
          return;
        }
      }
    }
  }

  setDiffPatch(diffPatch: string, sessionId?: string): void {
    const session = this.resolveSession(sessionId);
    const lastMessage = session.messages[session.messages.length - 1];
    const now = Date.now();

    if (lastMessage && lastMessage.role === "assistant") {
      const updatedMsg: ChatMessage = {
        ...lastMessage,
        diffPatch,
        timestamp: now,
        steps: lastMessage.steps ? [...lastMessage.steps] : undefined,
      };
      session.messages[session.messages.length - 1] = updatedMsg;
    } else {
      const newMsg: ChatMessage = {
        id: randomUUID(),
        role: "assistant",
        content: "",
        diffPatch,
        timestamp: now,
      };
      session.messages.push(newMsg);
    }

    session.updatedAt = now;
    this.updateSnapshot();
    this.notify();
    this.triggerAutoSave();
  }

  setGenerating(generating: boolean): void {
    if (this.isGenerating !== generating) {
      this.isGenerating = generating;
      this.updateSnapshot();
      this.notify();
    }
  }

  setWorkspaceDir(dir: string): void {
    if (this.workspaceDir !== dir) {
      this.workspaceDir = dir;
      this.updateSnapshot();
      this.notify();
      this.triggerAutoSave();
    }
  }

  setSelectedModel(model: string): void {
    if (this.selectedModel !== model) {
      this.selectedModel = model;
      this.updateSnapshot();
      this.notify();
      this.triggerAutoSave();
    }
  }

  private resolveSession(sessionId?: string): ChatSession {
    if (sessionId) {
      const target = this.sessions.find((s) => s.id === sessionId);
      if (target) {
        return target;
      }
    }
    const active = this.getActiveSession();
    if (active) {
      return active;
    }
    return this.createSession();
  }

  // --- Disk Persistence ---

  private triggerAutoSave(): void {
    if (this.autoSave) {
      void this.save().catch((err: unknown) => {
        console.error("[SessionStore] Auto-save error:", err);
      });
    }
  }

  async save(): Promise<void> {
    if (this.savePromise) {
      this.needsSave = true;
      return new Promise<void>((resolve, reject) => {
        this.pendingWaiters.push({ resolve, reject });
      });
    }

    this.savePromise = this.runSaveQueue();
    return this.savePromise;
  }

  private async runSaveQueue(): Promise<void> {
    let currentBatch: Array<{
      resolve: () => void;
      reject: (err: unknown) => void;
    }> = [];

    try {
      this.lastSaveError = null;
      while (true) {
        currentBatch = this.pendingWaiters;
        this.pendingWaiters = [];
        this.needsSave = false;

        await this.performSave();

        for (const waiter of currentBatch) {
          waiter.resolve();
        }
        currentBatch = [];

        if (this.pendingWaiters.length === 0) {
          break;
        }
      }
    } catch (err: unknown) {
      this.lastSaveError = err;
      const toReject = [...currentBatch, ...this.pendingWaiters];
      currentBatch = [];
      this.pendingWaiters = [];
      for (const waiter of toReject) {
        waiter.reject(err);
      }
      throw err;
    } finally {
      this.needsSave = false;
      this.savePromise = null;
    }
  }

  async waitForPendingSave(): Promise<void> {
    try {
      while (this.savePromise) {
        await this.savePromise;
      }
      if (this.lastSaveError) {
        const err = this.lastSaveError;
        this.lastSaveError = null;
        throw err;
      }
    } catch (err: unknown) {
      this.lastSaveError = null;
      throw err;
    } finally {
      this.needsSave = false;
    }
  }
  private async performSave(): Promise<void> {
    const dir = dirname(this.storagePath);
    await mkdir(dir, { recursive: true });

    const tempPath = join(
      dir,
      `.sessions-${Date.now()}-${Math.random().toString(36).slice(2)}.tmp`
    );

    const payload: PersistedSessionData = {
      version: 1,
      sessions: this.sessions,
      activeSessionId: this.activeSessionId,
      workspaceDir: this.workspaceDir,
      selectedModel: this.selectedModel,
    };

    const serialized = JSON.stringify(payload, null, 2);
    try {
      await writeFile(tempPath, serialized, "utf-8");
      await rename(tempPath, this.storagePath);
    } catch (err: unknown) {
      try {
        await rm(tempPath, { force: true });
      } catch {
        // Ignore temp file cleanup error
      }
      throw err;
    }
  }

  async load(): Promise<boolean> {
    try {
      if (!existsSync(this.storagePath)) {
        return false;
      }

      const content = await readFile(this.storagePath, "utf-8");
      const data: unknown = JSON.parse(content);
      if (!data || typeof data !== "object") {
        return false;
      }

      const record = data as Record<string, unknown>;
      if (!Array.isArray(record.sessions)) {
        return false;
      }

      const loadedSessions: ChatSession[] = [];
      for (const item of record.sessions) {
        if (isValidSession(item)) {
          loadedSessions.push(item);
        }
      }

      this.sessions = loadedSessions;
      if (
        typeof record.activeSessionId === "string" &&
        loadedSessions.some((s) => s.id === record.activeSessionId)
      ) {
        this.activeSessionId = record.activeSessionId;
      } else {
        this.activeSessionId = loadedSessions.length > 0 ? loadedSessions[0].id : null;
      }

      if (typeof record.workspaceDir === "string" && record.workspaceDir.trim().length > 0) {
        this.workspaceDir = record.workspaceDir;
      }
      if (typeof record.selectedModel === "string" && record.selectedModel.trim().length > 0) {
        this.selectedModel = record.selectedModel;
      }

      this.updateSnapshot();
      this.notify();
      return true;
    } catch {
      return false;
    }
  }
}
