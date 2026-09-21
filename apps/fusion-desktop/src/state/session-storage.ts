import { DEFAULT_FUSION_MODEL } from "../models";
import type { ChatMessage } from "../types";

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
}

/**
 * Deletes a session by identifier.
 */
export function deleteSessionFromStorage(sessionId: string): ChatSessionRecord[] {
  const current = loadAllSessions();
  const filtered = current.filter((s) => s.id !== sessionId);

  // Always retain at least one session
  const finalSessions = filtered.length > 0 ? filtered : [getInitialDefaultSession()];
  saveAllSessions(finalSessions);
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
