import { DEFAULT_FUSION_MODEL } from "../models";
import type { ChatMessage, TurnStep } from "../types";
import {
  isTauriEnvironment,
  listFusionSessions,
  loadFusionSession,
  saveFusionSession,
  deleteFusionSession,
} from "../lib/fusion-ipc";

export interface ChatSessionRecord {
  id: string;
  title: string;
  createdAt: number;
  updatedAt: number;
  model: string;
  messages: ChatMessage[];
}

const STORAGE_KEY = "fusion_desktop_sessions_v2";
const ACTIVE_SESSION_KEY = "fusion_desktop_active_session_v2";

let memoryStorage: Record<string, string> = {};

function getStorageItem(key: string): string | null {
  if (typeof window !== "undefined" && window.localStorage) {
    return window.localStorage.getItem(key);
  }
  return memoryStorage[key] ?? null;
}

function setStorageItem(key: string, value: string): void {
  if (typeof window !== "undefined" && window.localStorage) {
    window.localStorage.setItem(key, value);
    return;
  }
  memoryStorage[key] = value;
}

export function clearMemoryStorage(): void {
  memoryStorage = {};
  if (typeof window !== "undefined" && window.localStorage) {
    window.localStorage.clear();
  }
}

export function getInitialDefaultSession(): ChatSessionRecord {
  const now = Date.now();
  return {
    id: "session-default",
    title: "General chat conversation",
    createdAt: now - 3600 * 1000,
    updatedAt: now - 3600 * 1000,
    model: DEFAULT_FUSION_MODEL.id,
    messages: [],
  };
}

/**
 * Loads all chat sessions from storage.
 * If empty or invalid, initializes with a clean default session.
 */
export function loadAllSessions(): ChatSessionRecord[] {
  try {
    const raw = getStorageItem(STORAGE_KEY);
    if (!raw) {
      const defaultSession = getInitialDefaultSession();
      saveAllSessions([defaultSession]);
      return [defaultSession];
    }

    const parsed = JSON.parse(raw);
    if (!Array.isArray(parsed) || parsed.length === 0) {
      const defaultSession = getInitialDefaultSession();
      saveAllSessions([defaultSession]);
      return [defaultSession];
    }

    // Sanitize sessions
    const sanitized: ChatSessionRecord[] = parsed.map((s) => ({
      id: typeof s.id === "string" ? s.id : `session-${Date.now()}`,
      title: typeof s.title === "string" ? s.title : "Untitled conversation",
      createdAt: typeof s.createdAt === "number" ? s.createdAt : Date.now(),
      updatedAt: typeof s.updatedAt === "number" ? s.updatedAt : Date.now(),
      model: typeof s.model === "string" ? s.model : DEFAULT_FUSION_MODEL.id,
      messages: Array.isArray(s.messages) ? s.messages : [],
    }));

    return sanitized;
  } catch (err) {
    console.warn("[session-storage] Failed to load sessions, resetting to default:", err);
    const defaultSession = getInitialDefaultSession();
    return [defaultSession];
  }
}

/**
 * Persists the entire array of chat sessions.
 */
export function saveAllSessions(sessions: ChatSessionRecord[]): void {
  try {
    setStorageItem(STORAGE_KEY, JSON.stringify(sessions));
  } catch (err) {
    console.error("[session-storage] Failed to save sessions:", err);
  }
}

/**
 * Saves or updates a single session in storage.
 */
export function saveSession(session: ChatSessionRecord): void {
  const current = loadAllSessions();
  const index = current.findIndex((s) => s.id === session.id);

  let updated: ChatSessionRecord[];
  if (index >= 0) {
    updated = [...current];
    updated[index] = { ...session, updatedAt: Date.now() };
  } else {
    updated = [session, ...current];
  }

  saveAllSessions(updated);

  // Background sync to native ~/.fusion/sessions/ if in Tauri
  if (isTauriEnvironment()) {
    persistSessionToNativeDisk(session).catch((e) => {
      console.warn("[session-storage] Native disk persist failed:", e);
    });
  }
}

/**
 * Serializes and writes session JSON directly to ~/.fusion/sessions/<id>.json.
 */
export async function persistSessionToNativeDisk(session: ChatSessionRecord): Promise<void> {
  if (!isTauriEnvironment()) return;

  const nativePayload = {
    id: session.id,
    created_at: new Date(session.createdAt).toISOString(),
    updated_at: new Date(session.updatedAt).toISOString(),
    active_model: session.model,
    title: session.title,
    messages: session.messages.map((m) => ({
      role: m.role,
      content: m.content,
      thought: m.thought,
      steps: m.steps,
      diff_patch: m.diffPatch,
    })),
    token_stats: {
      prompt_tokens: 0,
      completion_tokens: 0,
      total_tokens: 0,
      cache_read_tokens: 0,
      cache_write_tokens: 0,
      total_turns: session.messages.filter((m) => m.role === "user").length,
    },
  };

  await saveFusionSession(nativePayload);
}

/**
 * Syncs the frontend session list with native ~/.fusion/sessions/*.json on disk.
 */
export async function syncNativeFusionSessions(
  currentSessions: ChatSessionRecord[]
): Promise<ChatSessionRecord[]> {
  if (!isTauriEnvironment()) {
    return currentSessions;
  }

  try {
    const nativeList = await listFusionSessions();
    if (!nativeList || nativeList.length === 0) {
      return currentSessions;
    }

    const mergedMap = new Map<string, ChatSessionRecord>();

    // 1. Add current sessions
    for (const s of currentSessions) {
      mergedMap.set(s.id, s);
    }

    // 2. Add or update native sessions
    for (const n of nativeList) {
      const existing = mergedMap.get(n.id);
      const parsedUpdated = n.updated_at ? new Date(n.updated_at).getTime() : Date.now();
      const parsedCreated = n.created_at ? new Date(n.created_at).getTime() : Date.now();

      if (existing) {
        mergedMap.set(n.id, {
          ...existing,
          title: n.title || existing.title,
          updatedAt: Math.max(existing.updatedAt, isNaN(parsedUpdated) ? 0 : parsedUpdated),
          model: n.model || existing.model,
        });
      } else {
        mergedMap.set(n.id, {
          id: n.id,
          title: n.title || "General chat conversation",
          createdAt: isNaN(parsedCreated) ? Date.now() : parsedCreated,
          updatedAt: isNaN(parsedUpdated) ? Date.now() : parsedUpdated,
          model: n.model || DEFAULT_FUSION_MODEL.id,
          messages: [],
        });
      }
    }

    const result = Array.from(mergedMap.values());
    result.sort((a, b) => b.updatedAt - a.updatedAt);
    saveAllSessions(result);
    return result;
  } catch (err) {
    console.warn("[session-storage] syncNativeFusionSessions failed:", err);
    return currentSessions;
  }
}

/**
 * Loads full messages for a session from ~/.fusion/sessions/<id>.json if available.
 */
export async function loadFullNativeSessionMessages(id: string): Promise<ChatMessage[] | null> {
  if (!isTauriEnvironment()) {
    return null;
  }

  try {
    const raw = await loadFusionSession(id);
    if (!raw) return null;

    const rawMessages = raw.messages;
    if (!Array.isArray(rawMessages)) return [];

    const parsedMessages: ChatMessage[] = rawMessages.map((m, idx) => {
      const rec = m as Record<string, unknown>;
      return {
        id: typeof rec.id === "string" ? rec.id : `msg-${id}-${idx}`,
        role: (rec.role === "user" ? "user" : "assistant") as "user" | "assistant",
        content: typeof rec.content === "string" ? rec.content : "",
        thought: typeof rec.thought === "string" ? rec.thought : undefined,
        steps: Array.isArray(rec.steps) ? (rec.steps as unknown as TurnStep[]) : undefined,
        timestamp: typeof rec.timestamp === "number" ? rec.timestamp : Date.now(),
      };
    });

    return parsedMessages;
  } catch (err) {
    console.warn(`[session-storage] loadFullNativeSessionMessages failed for '${id}':`, err);
    return null;
  }
}

/**
 * Deletes a session by identifier both locally and from ~/.fusion/sessions/ on disk.
 */
export function deleteSessionFromStorage(sessionId: string): ChatSessionRecord[] {
  const current = loadAllSessions();
  const filtered = current.filter((s) => s.id !== sessionId);

  // Always retain at least one session
  const finalSessions = filtered.length > 0 ? filtered : [getInitialDefaultSession()];
  saveAllSessions(finalSessions);

  if (isTauriEnvironment()) {
    deleteFusionSession(sessionId).catch((err: unknown) => {
      console.warn("[session-storage] deleteFusionSession failed:", err);
    });
  }

  return finalSessions;
}

/**
 * Returns the currently active session ID.
 */
export function getStoredActiveSessionId(): string {
  return getStorageItem(ACTIVE_SESSION_KEY) || "session-default";
}

/**
 * Stores the active session ID.
 */
export function setStoredActiveSessionId(sessionId: string): void {
  try {
    setStorageItem(ACTIVE_SESSION_KEY, sessionId);
  } catch {
    // Ignore storage quota errors
  }
}
