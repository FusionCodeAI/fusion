import { describe, expect, it, beforeEach, afterEach } from "bun:test";
import { mkdtempSync, rmSync, writeFileSync, existsSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { SessionStore } from "../src/state/session-store";
import type { TurnStep } from "../src/types";

describe("SessionStore - Session Lifecycle", () => {
  let tempDir: string;
  let storagePath: string;

  beforeEach(() => {
    tempDir = mkdtempSync(join(tmpdir(), "fusion-store-test-"));
    storagePath = join(tempDir, "sessions.json");
  });

  afterEach(() => {
    rmSync(tempDir, { recursive: true, force: true });
  });

  it("initializes with default state", () => {
    const store = new SessionStore({ storagePath });
    expect(store.sessions).toHaveLength(0);
    expect(store.activeSessionId).toBeNull();
    expect(store.getActiveSession()).toBeNull();
    expect(store.isGenerating).toBe(false);
    expect(typeof store.workspaceDir).toBe("string");
    expect(typeof store.selectedModel).toBe("string");
  });

  it("creates a new session and marks it as active", () => {
    const store = new SessionStore({ storagePath });
    const session = store.createSession("Feature Planner");

    expect(session.id).toBeDefined();
    expect(session.title).toBe("Feature Planner");
    expect(session.messages).toHaveLength(0);
    expect(session.createdAt).toBeGreaterThan(0);
    expect(session.updatedAt).toBeGreaterThan(0);
    expect(store.sessions).toHaveLength(1);
    expect(store.activeSessionId).toBe(session.id);
    expect(store.getActiveSession()?.id).toBe(session.id);
  });

  it("creates session with custom workspaceDir and model overrides", () => {
    const store = new SessionStore({
      storagePath,
      workspaceDir: "/default/workspace",
      selectedModel: "claude-3-5-sonnet",
    });

    const session = store.createSession("Custom Env", "/custom/path", "custom-model");
    expect(session.workspaceDir).toBe("/custom/path");
    expect(session.model).toBe("custom-model");
  });

  it("selects an existing session and updates activeSessionId", () => {
    const store = new SessionStore({ storagePath });
    const s1 = store.createSession("Session 1");
    const s2 = store.createSession("Session 2");

    expect(store.activeSessionId).toBe(s2.id);

    const selected = store.selectSession(s1.id);
    expect(selected?.id).toBe(s1.id);
    expect(store.activeSessionId).toBe(s1.id);
    expect(store.getActiveSession()?.id).toBe(s1.id);
  });

  it("returns null when selecting a non-existent session", () => {
    const store = new SessionStore({ storagePath });
    const s1 = store.createSession("Session 1");

    const result = store.selectSession("non-existent-id");
    expect(result).toBeNull();
    expect(store.activeSessionId).toBe(s1.id);
  });

  it("deletes a session and shifts activeSessionId if active was deleted", () => {
    const store = new SessionStore({ storagePath });
    const s1 = store.createSession("Session 1");
    const s2 = store.createSession("Session 2");

    expect(store.activeSessionId).toBe(s2.id);

    const deleted = store.deleteSession(s2.id);
    expect(deleted).toBe(true);
    expect(store.sessions).toHaveLength(1);
    expect(store.sessions[0].id).toBe(s1.id);
    expect(store.activeSessionId).toBe(s1.id);

    const deletedLast = store.deleteSession(s1.id);
    expect(deletedLast).toBe(true);
    expect(store.sessions).toHaveLength(0);
    expect(store.activeSessionId).toBeNull();
    expect(store.getActiveSession()).toBeNull();
  });

  it("returns false when deleting a non-existent session", () => {
    const store = new SessionStore({ storagePath });
    store.createSession("Session 1");
    expect(store.deleteSession("random-id")).toBe(false);
  });

  it("updates session title and updatedAt timestamp", () => {
    const store = new SessionStore({ storagePath });
    const s1 = store.createSession("Old Title");
    const initialUpdatedAt = s1.updatedAt;

    const updated = store.updateSessionTitle(s1.id, "New Title");
    expect(updated).toBe(true);
    expect(s1.title).toBe("New Title");
    expect(s1.updatedAt).toBeGreaterThanOrEqual(initialUpdatedAt);

    const notUpdated = store.updateSessionTitle("unknown-id", "Another Title");
    expect(notUpdated).toBe(false);
  });
});

describe("SessionStore - Message Flow", () => {
  let tempDir: string;
  let storagePath: string;

  beforeEach(() => {
    tempDir = mkdtempSync(join(tmpdir(), "fusion-store-test-"));
    storagePath = join(tempDir, "sessions.json");
  });

  afterEach(() => {
    rmSync(tempDir, { recursive: true, force: true });
  });

  it("appends user message to active session", () => {
    const store = new SessionStore({ storagePath });
    const session = store.createSession("Test Session");

    const msg = store.appendUserMessage("How do I build GPUIX?");
    expect(msg.id).toBeDefined();
    expect(msg.role).toBe("user");
    expect(msg.content).toBe("How do I build GPUIX?");
    expect(session.messages).toHaveLength(1);
    expect(session.messages[0].id).toBe(msg.id);
  });

  it("auto-creates session if appendUserMessage is called without active session", () => {
    const store = new SessionStore({ storagePath });
    expect(store.sessions).toHaveLength(0);

    const msg = store.appendUserMessage("Hello agent");
    expect(store.sessions).toHaveLength(1);
    expect(store.getActiveSession()?.messages).toHaveLength(1);
    expect(store.getActiveSession()?.messages[0].content).toBe("Hello agent");
    expect(msg.content).toBe("Hello agent");
  });

  it("appends assistant chunks streaming into the same assistant message", () => {
    const store = new SessionStore({ storagePath });
    store.createSession("Streaming Test");
    store.appendUserMessage("Count to 3");

    const chunk1 = store.appendAssistantChunk("1, ");
    expect(chunk1.role).toBe("assistant");
    expect(chunk1.content).toBe("1, ");

    const chunk2 = store.appendAssistantChunk("2, ");
    expect(chunk2.id).toBe(chunk1.id);
    expect(chunk2.content).toBe("1, 2, ");

    const chunk3 = store.appendAssistantChunk("3!");
    expect(chunk3.id).toBe(chunk1.id);
    expect(chunk3.content).toBe("1, 2, 3!");

    const msgs = store.getActiveSession()?.messages ?? [];
    expect(msgs).toHaveLength(2);
    expect(msgs[0].role).toBe("user");
    expect(msgs[1].role).toBe("assistant");
    expect(msgs[1].content).toBe("1, 2, 3!");
  });

  it("appends thought steps and integrates with assistant message", () => {
    const store = new SessionStore({ storagePath });
    store.createSession("Thought Chain");
    store.appendUserMessage("Fix authentication bug");

    store.appendThoughtStep("Analyzing codebase...");
    store.appendAssistantChunk("I am ready");

    const msgs = store.getActiveSession()?.messages ?? [];
    expect(msgs.length).toBe(2);
    expect(msgs[1].content).toBe("I am ready");
    expect(msgs[1].thought).toContain("Analyzing codebase...");
    expect(msgs[1].steps).toBeDefined();
    expect(msgs[1].steps?.length).toBe(1);
    expect(msgs[1].steps?.[0].title).toBe("Analyzing codebase...");
  });

  it("appends structured TurnStep objects and updates them", () => {
    const store = new SessionStore({ storagePath });
    store.createSession("Step Tracking");
    store.appendUserMessage("Refactor bridge");

    const step: TurnStep = {
      id: "step-42",
      title: "Reading acp-client.ts",
      status: "running",
      timestamp: Date.now(),
      details: "Loading file contents",
    };

    store.appendThoughtStep(step);
    const session = store.getActiveSession();
    expect(session?.messages[1].steps?.[0].status).toBe("running");

    store.updateTurnStep("step-42", "completed", "File loaded successfully");
    const updatedStep = session?.messages[1].steps?.[0];
    expect(updatedStep?.status).toBe("completed");
    expect(updatedStep?.details).toBe("File loaded successfully");
  });

  it("sets diff patch on the active assistant message", () => {
    const store = new SessionStore({ storagePath });
    store.createSession("Patch Test");
    store.appendUserMessage("Generate patch");
    store.appendAssistantChunk("Here is the patch:");

    const patch = "--- a/file.ts\n+++ b/file.ts\n@@ -1 +1 @@\n-old\n+new\n";
    store.setDiffPatch(patch);

    const activeMsg = store.getActiveSession()?.messages[1];
    expect(activeMsg?.diffPatch).toBe(patch);
  });

  it("routes message mutations to explicit sessionId if provided", () => {
    const store = new SessionStore({ storagePath });
    const s1 = store.createSession("Session 1");
    const s2 = store.createSession("Session 2");

    expect(store.activeSessionId).toBe(s2.id);

    store.appendUserMessage("Message for session 1", s1.id);
    expect(s1.messages).toHaveLength(1);
    expect(s2.messages).toHaveLength(0);

    store.appendAssistantChunk("Chunk for session 1", s1.id);
    expect(s1.messages).toHaveLength(2);
    expect(s1.messages[1].content).toBe("Chunk for session 1");
  });
});

describe("SessionStore - Global State & Configuration", () => {
  let tempDir: string;
  let storagePath: string;

  beforeEach(() => {
    tempDir = mkdtempSync(join(tmpdir(), "fusion-store-test-"));
    storagePath = join(tempDir, "sessions.json");
  });

  afterEach(() => {
    rmSync(tempDir, { recursive: true, force: true });
  });

  it("updates workspaceDir and notifies listeners", () => {
    const store = new SessionStore({ storagePath });
    let notified = false;
    store.subscribe(() => {
      notified = true;
    });

    store.setWorkspaceDir("/new/workspace/dir");
    expect(store.workspaceDir).toBe("/new/workspace/dir");
    expect(notified).toBe(true);
  });

  it("updates selectedModel and notifies listeners", () => {
    const store = new SessionStore({ storagePath });
    let notified = false;
    store.subscribe(() => {
      notified = true;
    });

    store.setSelectedModel("claude-3-7-sonnet");
    expect(store.selectedModel).toBe("claude-3-7-sonnet");
    expect(notified).toBe(true);
  });

  it("updates isGenerating flag and notifies listeners", () => {
    const store = new SessionStore({ storagePath });
    let count = 0;
    store.subscribe(() => {
      count++;
    });

    store.setGenerating(true);
    expect(store.isGenerating).toBe(true);
    expect(count).toBe(1);

    store.setGenerating(false);
    expect(store.isGenerating).toBe(false);
    expect(count).toBe(2);
  });
});

describe("SessionStore - React Integration & Reactivity", () => {
  let tempDir: string;
  let storagePath: string;

  beforeEach(() => {
    tempDir = mkdtempSync(join(tmpdir(), "fusion-store-test-"));
    storagePath = join(tempDir, "sessions.json");
  });

  afterEach(() => {
    rmSync(tempDir, { recursive: true, force: true });
  });

  it("notifies listeners on mutations and supports unsubscribe", () => {
    const store = new SessionStore({ storagePath });
    let callCount = 0;
    const unsubscribe = store.subscribe(() => {
      callCount++;
    });

    store.createSession("Session 1");
    expect(callCount).toBe(1);

    store.appendUserMessage("Hi");
    expect(callCount).toBe(2);

    unsubscribe();
    store.appendUserMessage("After unsubscribe");
    expect(callCount).toBe(2);
  });

  it("provides immutable snapshot via getSnapshot with referential stability", () => {
    const store = new SessionStore({ storagePath });
    const snap1 = store.getSnapshot();
    const snap2 = store.getSnapshot();

    // Referential equality when no mutation occurred
    expect(snap1).toBe(snap2);
    expect(Object.isFrozen(snap1)).toBe(true);

    store.createSession("Test");
    const snap3 = store.getSnapshot();
    expect(snap3).not.toBe(snap1);
    expect(snap3.sessions).toHaveLength(1);
    expect(Object.isFrozen(snap3)).toBe(true);

    // Verify snapshot immutability prevents property mutation
    expect(() => {
      const mutableTarget = snap3 as unknown as { activeSessionId: string | null };
      mutableTarget.activeSessionId = "modified";
    }).toThrow();
  });
});

describe("SessionStore - Disk Persistence", () => {
  let tempDir: string;
  let storagePath: string;

  beforeEach(() => {
    tempDir = mkdtempSync(join(tmpdir(), "fusion-store-test-"));
    storagePath = join(tempDir, "sessions.json");
  });

  afterEach(() => {
    rmSync(tempDir, { recursive: true, force: true });
  });

  it("saves state atomically and loads it back", async () => {
    const store1 = new SessionStore({
      storagePath,
      workspaceDir: "/project/app",
      selectedModel: "fusion-agent",
    });

    const s1 = store1.createSession("Session 1");
    store1.appendUserMessage("User prompt 1");
    store1.appendAssistantChunk("Assistant answer 1");

    const s2 = store1.createSession("Session 2");
    store1.appendUserMessage("User prompt 2");

    await store1.save();
    expect(existsSync(storagePath)).toBe(true);

    // Read back in fresh store
    const store2 = new SessionStore({ storagePath });
    const loaded = await store2.load();
    expect(loaded).toBe(true);

    expect(store2.sessions).toHaveLength(2);
    expect(store2.activeSessionId).toBe(s2.id);
    expect(store2.workspaceDir).toBe("/project/app");
    expect(store2.selectedModel).toBe("fusion-agent");

    const loadedS1 = store2.sessions.find((s) => s.id === s1.id);
    expect(loadedS1).toBeDefined();
    expect(loadedS1?.title).toBe("Session 1");
    expect(loadedS1?.messages).toHaveLength(2);
    expect(loadedS1?.messages[0].content).toBe("User prompt 1");
    expect(loadedS1?.messages[1].content).toBe("Assistant answer 1");
  });

  it("returns false gracefully when file does not exist", async () => {
    const store = new SessionStore({ storagePath: join(tempDir, "does-not-exist.json") });
    const loaded = await store.load();
    expect(loaded).toBe(false);
    expect(store.sessions).toHaveLength(0);
  });

  it("returns false and recovers gracefully when file is corrupted JSON", async () => {
    writeFileSync(storagePath, "{ corrupted json: null, [", "utf-8");

    const store = new SessionStore({ storagePath });
    const loaded = await store.load();
    expect(loaded).toBe(false);
    expect(store.sessions).toHaveLength(0);
  });

  it("automatically persists mutations when autoSave is enabled", async () => {
    const store = new SessionStore({ storagePath, autoSave: true });
    store.createSession("Auto Saved Session");
    store.appendUserMessage("Auto message");

    // Wait for deterministic in-flight auto-save to settle
    await store.waitForPendingSave();
    expect(existsSync(storagePath)).toBe(true);

    const store2 = new SessionStore({ storagePath });
    const loaded = await store2.load();
    expect(loaded).toBe(true);
    expect(store2.sessions).toHaveLength(1);
    expect(store2.sessions[0].title).toBe("Auto Saved Session");
    expect(store2.sessions[0].messages[0].content).toBe("Auto message");
  });
});
