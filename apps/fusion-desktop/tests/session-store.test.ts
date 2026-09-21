import { describe, expect, it, beforeEach, afterEach } from "bun:test";
import { mkdtempSync, rmSync, writeFileSync, existsSync } from "node:fs";
import { readFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { SessionStore, isValidMessage, isValidSession } from "../src/state/session-store";
import type { ChatSession, TurnStep } from "../src/types";

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

  it("ensures snapshot immutability so mutating store after snapshot does not mutate prior snapshot's messages or steps", () => {
    const store = new SessionStore({ storagePath });
    const session = store.createSession("Immutability Test");
    store.appendUserMessage("Hello");
    store.appendAssistantChunk("Initial answer");
    store.appendThoughtStep("Step 1: thinking");

    const snap1 = store.getSnapshot();
    const snap1Session = snap1.sessions.find((s) => s.id === session.id)!;
    const snap1AssistantMsg = snap1Session.messages.find((m) => m.role === "assistant")!;
    const snap1Step = snap1AssistantMsg.steps![0];

    // Verify snapshot and nested objects are frozen
    expect(Object.isFrozen(snap1)).toBe(true);
    expect(Object.isFrozen(snap1Session)).toBe(true);
    expect(Object.isFrozen(snap1Session.messages)).toBe(true);
    expect(Object.isFrozen(snap1AssistantMsg)).toBe(true);
    expect(Object.isFrozen(snap1AssistantMsg.steps)).toBe(true);
    expect(Object.isFrozen(snap1Step)).toBe(true);

    const originalContent = snap1AssistantMsg.content;
    const originalStepStatus = snap1Step.status;

    // Mutate the store: append chunk, update step, add diff patch
    store.appendAssistantChunk(" - additional text");
    store.updateTurnStep(snap1Step.id, "completed", "done reasoning");
    store.setDiffPatch("@@ -1,1 +1,1 @@\n-old\n+new");

    const snap2 = store.getSnapshot();
    const snap2Session = snap2.sessions.find((s) => s.id === session.id)!;
    const snap2AssistantMsg = snap2Session.messages.find((m) => m.role === "assistant")!;
    const snap2Step = snap2AssistantMsg.steps![0];

    // Prior snapshot must remain untouched
    expect(snap1AssistantMsg.content).toBe(originalContent);
    expect(snap1Step.status).toBe(originalStepStatus);
    expect(snap1AssistantMsg.diffPatch).toBeUndefined();

    // New snapshot must reflect changes with new object references (for React.memo)
    expect(snap2AssistantMsg).not.toBe(snap1AssistantMsg);
    expect(snap2AssistantMsg.steps).not.toBe(snap1AssistantMsg.steps);
    expect(snap2Step).not.toBe(snap1Step);
    expect(snap2AssistantMsg.content).toBe("Initial answer - additional text");
    expect(snap2Step.status).toBe("completed");
    expect(snap2AssistantMsg.diffPatch).toBe("@@ -1,1 +1,1 @@\n-old\n+new");

    // Direct mutation of snap1 must throw
    expect(() => {
      (snap1AssistantMsg as unknown as { content: string }).content = "mutated";
    }).toThrow();
    expect(() => {
      (snap1Step as unknown as { status: string }).status = "failed";
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

  it("resolves all concurrent save calls after the latest state is persisted to disk", async () => {
    const store = new SessionStore({ storagePath });
    const session = store.createSession("Concurrent Save Test");

    store.appendUserMessage("Message 1");
    const p1 = store.save();

    store.appendUserMessage("Message 2");
    const p2 = store.save();

    store.appendUserMessage("Message 3");
    const p3 = store.save();

    await Promise.all([p1, p2, p3]);

    const raw = await readFile(storagePath, "utf-8");
    const persisted = JSON.parse(raw);

    const persistedSession = persisted.sessions.find((s: Record<string, unknown>) => s.id === session.id);
    expect(persistedSession).toBeDefined();
    expect(persistedSession.messages).toHaveLength(3);
    expect(persistedSession.messages[0].content).toBe("Message 1");
    expect(persistedSession.messages[1].content).toBe("Message 2");
    expect(persistedSession.messages[2].content).toBe("Message 3");
  });

  it("rejects without an infinite loop when performSave fails", async () => {
    const badDir = join(tempDir, "regular-file-blocking-dir");
    writeFileSync(badDir, "not-a-directory");
    const badStoragePath = join(badDir, "sub-dir", "sessions.json");
    const store = new SessionStore({ storagePath: badStoragePath, autoSave: true });
    store.createSession("Fail Session");
    store.appendUserMessage("Will fail to save");

    let waitError: unknown = null;
    try {
      await store.waitForPendingSave();
    } catch (err: unknown) {
      waitError = err;
    }
    expect(waitError).toBeDefined();

    // Calling waitForPendingSave again completes promptly without hanging
    await expect(store.waitForPendingSave()).resolves.toBeUndefined();

    // Explicit save failure also rejects both save and waitForPendingSave
    const store2 = new SessionStore({ storagePath: badStoragePath });
    store2.createSession("Explicit Fail");
    let saveError: unknown = null;
    try {
      await store2.save();
    } catch (err: unknown) {
      saveError = err;
    }
    expect(saveError).toBeDefined();

    let waitError2: unknown = null;
    try {
      await store2.waitForPendingSave();
    } catch (err: unknown) {
      waitError2 = err;
    }
    // Either save caught it or waitForPendingSave caught it
    await expect(store2.waitForPendingSave()).resolves.toBeUndefined();
  });
  it("validates individual message records and rejects corrupted entries", () => {
    const validSession: ChatSession = {
      id: "sess-1",
      title: "Valid Session",
      createdAt: 1000,
      updatedAt: 2000,
      workspaceDir: "/workspace",
      model: "claude-3-5-sonnet",
      messages: [
        {
          id: "msg-1",
          role: "user",
          content: "Valid message",
          timestamp: 1500,
        },
      ],
    };

    expect(isValidSession(validSession)).toBe(true);

    // Corrupted message: missing role
    expect(
      isValidSession({
        ...validSession,
        messages: [{ id: "msg-2", content: "No role", timestamp: 1600 }],
      })
    ).toBe(false);

    // Corrupted message: invalid role
    expect(
      isValidSession({
        ...validSession,
        messages: [{ id: "msg-2", role: "unknown_role", content: "Bad", timestamp: 1600 }],
      })
    ).toBe(false);

    // Corrupted message: non-object item (null)
    expect(
      isValidSession({
        ...validSession,
        messages: [null],
      })
    ).toBe(false);

    // Corrupted message: missing id
    expect(
      isValidSession({
        ...validSession,
        messages: [{ role: "user", content: "No id", timestamp: 1600 }],
      })
    ).toBe(false);

    // Corrupted message: missing timestamp
    expect(
      isValidSession({
        ...validSession,
        messages: [{ id: "msg-3", role: "assistant", content: "No timestamp" }],
      })
    ).toBe(false);
  });

  it("ignores sessions with corrupted message structures during load", async () => {
    const corruptPayload = {
      version: 1,
      sessions: [
        {
          id: "sess-corrupt",
          title: "Corrupt Session",
          createdAt: 1000,
          updatedAt: 2000,
          workspaceDir: "/workspace",
          model: "claude-3-5-sonnet",
          messages: [
            { id: "msg-1", role: "user", content: "OK", timestamp: 1000 },
            { id: "msg-2", role: "invalid-role", content: "Corrupt", timestamp: 1001 },
          ],
        },
      ],
      activeSessionId: "sess-corrupt",
    };
    writeFileSync(storagePath, JSON.stringify(corruptPayload), "utf-8");

    const store = new SessionStore({ storagePath });
    const loaded = await store.load();
    expect(loaded).toBe(true);
    expect(store.sessions).toHaveLength(0);
  });
});
