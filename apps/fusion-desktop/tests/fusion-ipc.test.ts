import { describe, test, expect } from "bun:test";
import {
  isTauriEnvironment,
  listFusionSessions,
  loadFusionSession,
  saveFusionSession,
  deleteFusionSession,
} from "../src/lib/fusion-ipc";

describe("Native Fusion IPC Bridge", () => {
  test("isTauriEnvironment returns false in standard test/browser environment", () => {
    expect(isTauriEnvironment()).toBe(false);
  });

  test("listFusionSessions returns empty array gracefully when not in Tauri", async () => {
    const sessions = await listFusionSessions();
    expect(Array.isArray(sessions)).toBe(true);
    expect(sessions.length).toBe(0);
  });

  test("loadFusionSession returns null gracefully when not in Tauri", async () => {
    const session = await loadFusionSession("any-id");
    expect(session).toBeNull();
  });

  test("saveFusionSession returns null gracefully when not in Tauri", async () => {
    const res = await saveFusionSession({ id: "test" });
    expect(res).toBeNull();
  });

  test("deleteFusionSession returns false gracefully when not in Tauri", async () => {
    const res = await deleteFusionSession("test");
    expect(res).toBe(false);
  });
});
