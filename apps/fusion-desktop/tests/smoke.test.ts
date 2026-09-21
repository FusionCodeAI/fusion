import { describe, expect, it } from "bun:test";
import type { MessageRole, TurnStep, ChatMessage, ChatSession, AcpEvent } from "../src/types";

describe("smoke test", () => {
  it("verifies bun runtime environment", () => {
    expect(typeof Bun).toBe("object");
    expect(typeof Bun.version).toBe("string");
  });

  it("verifies core types compile and behave correctly", () => {
    const role: MessageRole = "assistant";
    const step: TurnStep = {
      id: "step-1",
      title: "Analyzing repository",
      status: "running",
      timestamp: 1700000000000,
      details: "Inspecting files",
    };
    const message: ChatMessage = {
      id: "msg-1",
      role,
      content: "Ready to assist.",
      thought: "Processing user request",
      steps: [step],
      diffPatch: "--- a/file\n+++ b/file",
      timestamp: 1700000000000,
    };
    const session: ChatSession = {
      id: "session-1",
      title: "Desktop Control Plane",
      createdAt: 1700000000000,
      updatedAt: 1700000000100,
      workspaceDir: "/workspace",
      model: "fusion-agent",
      messages: [message],
    };
    const event: AcpEvent = {
      method: "session/update",
      params: {
        sessionId: session.id,
        status: "active",
      },
    };

    expect(role).toBe("assistant");
    expect(step.status).toBe("running");
    expect(message.steps?.[0].title).toBe("Analyzing repository");
    expect(session.messages).toHaveLength(1);
    expect(event.method).toBe("session/update");
    expect(event.params.sessionId).toBe("session-1");
  });
});
