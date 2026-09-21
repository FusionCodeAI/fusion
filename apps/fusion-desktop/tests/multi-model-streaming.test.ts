import { describe, test, expect } from "bun:test";
import { AgentBridge, createModelResponse } from "../src/lib/agent-bridge";
import { FUSION_MODELS } from "../src/models";

describe("Multi-Model Resilient Streaming Engine", () => {
  for (const model of FUSION_MODELS) {
    test(`generates valid model response for ${model.name} (${model.id})`, () => {
      const res = createModelResponse("hi", model.id);
      expect(res.thought).toBeDefined();
      expect(res.thought.length).toBeGreaterThan(10);
      expect(res.content).toContain("Fusion Agent");
      expect(res.content.length).toBeGreaterThan(20);
    });
  }
  test("AgentBridge streams thought, chunk, and done events reliably without hanging", async () => {
    const bridge = new AgentBridge({ forceLocalEngine: true });
    const thoughts: string[] = [];
    const chunks: string[] = [];
    let doneCalled = false;

    bridge.on("thought", (t) => thoughts.push(t));
    bridge.on("chunk", (c) => chunks.push(c));
    bridge.on("done", () => {
      doneCalled = true;
    });

    await bridge.prompt("hi", "deepseek-v4-flash-0731");

    expect(doneCalled).toBe(true);
    expect(thoughts.length).toBeGreaterThan(0);
    expect(chunks.length).toBeGreaterThan(0);
    expect(chunks.join("")).toContain("Fusion Agent");
  });
});
