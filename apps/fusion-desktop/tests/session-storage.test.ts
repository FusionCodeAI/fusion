import { describe, test, expect, beforeEach } from "bun:test";
import {
  loadAllSessions,
  saveSession,
  deleteSessionFromStorage,
  getStoredActiveSessionId,
  setStoredActiveSessionId,
  clearMemoryStorage,
  type ChatSessionRecord,
} from "../src/state/session-storage";

describe("Session Storage Persistence Layer", () => {
  beforeEach(() => {
    clearMemoryStorage();
  });

  test("loads default session when storage is empty", () => {
    const sessions = loadAllSessions();
    expect(sessions.length).toBeGreaterThan(0);
    expect(sessions[0].id).toBeDefined();
    expect(sessions[0].title).toBeDefined();
  });

  test("saves and updates sessions accurately", () => {
    const customSession: ChatSessionRecord = {
      id: "session-test-1",
      title: "My Feature Implementation",
      createdAt: Date.now(),
      updatedAt: Date.now(),
      model: "deepseek-v4-flash-0731",
      messages: [
        {
          id: "m1",
          role: "user",
          content: "Hello agent",
          timestamp: Date.now(),
        },
      ],
    };

    saveSession(customSession);
    const sessions = loadAllSessions();
    const found = sessions.find((s) => s.id === "session-test-1");
    expect(found).toBeDefined();
    expect(found?.title).toBe("My Feature Implementation");
    expect(found?.messages.length).toBe(1);
  });

  test("deletes session and preserves at least one fallback session", () => {
    const s1: ChatSessionRecord = {
      id: "s-delete-me",
      title: "Temporary session",
      createdAt: Date.now(),
      updatedAt: Date.now(),
      model: "deepseek-v4-flash-0731",
      messages: [],
    };

    saveSession(s1);
    const afterSave = loadAllSessions();
    expect(afterSave.some((s) => s.id === "s-delete-me")).toBe(true);

    const remaining = deleteSessionFromStorage("s-delete-me");
    expect(remaining.some((s) => s.id === "s-delete-me")).toBe(false);
    expect(remaining.length).toBeGreaterThan(0);
  });

  test("persists and retrieves active session identifier", () => {
    setStoredActiveSessionId("session-active-xyz");
    expect(getStoredActiveSessionId()).toBe("session-active-xyz");
  });
});
