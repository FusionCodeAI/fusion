import { describe, test, expect, beforeEach } from "bun:test";
import {
  syncNativeFusionSessions,
  persistSessionToNativeDisk,
  loadFullNativeSessionMessages,
  clearMemoryStorage,
  type ChatSessionRecord,
} from "../src/state/session-storage";

describe("Native Fusion Session Synchronization", () => {
  beforeEach(() => {
    clearMemoryStorage();
  });

  test("syncNativeFusionSessions returns current sessions cleanly when not in Tauri", async () => {
    const initial: ChatSessionRecord[] = [
      {
        id: "s1",
        title: "Initial Session",
        createdAt: Date.now(),
        updatedAt: Date.now(),
        model: "deepseek-v4-flash-0731",
        messages: [],
      },
    ];

    const result = await syncNativeFusionSessions(initial);
    expect(result.length).toBe(1);
    expect(result[0].id).toBe("s1");
  });

  test("loadFullNativeSessionMessages returns null gracefully when not in Tauri", async () => {
    const res = await loadFullNativeSessionMessages("any-uuid");
    expect(res).toBeNull();
  });

  test("persistSessionToNativeDisk runs without throwing when not in Tauri", async () => {
    const s: ChatSessionRecord = {
      id: "s2",
      title: "Test Save",
      createdAt: Date.now(),
      updatedAt: Date.now(),
      model: "deepseek-v4-flash-0731",
      messages: [],
    };
    await persistSessionToNativeDisk(s);
  });
});
