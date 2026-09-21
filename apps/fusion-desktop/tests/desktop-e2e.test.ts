import { describe, expect, it } from "bun:test";
import React from "react";
import { renderToStaticMarkup, renderToString } from "react-dom/server";
import type { AgentEvent, FusionAgent } from "@fusioncode/sdk";

// Models & Types
import {
  FUSION_MODELS,
  DEFAULT_FUSION_MODEL,
  getModelById,
  getFusionModel,
  isValidModelId,
  type FusionModel,
} from "../src/models";
import type { TurnStep, ChatMessage } from "../src/types";

// Agent Bridge & Helpers
import {
  AgentBridge,
  formatToolTitle,
  isSpinnerString,
  parseAndStripDsml,
  resolveFusionBinary,
  type BridgeEvent,
  type TurnDoneStats,
} from "../src/lib/agent-bridge";

// UI Components
import { ThinkingRow } from "../src/components/ThinkingRow";
import { UserMessage } from "../src/components/UserMessage";
import { ToolCallRow } from "../src/components/ToolCallRow";
import { DiffView } from "../src/components/DiffView";
import { Composer } from "../src/components/Composer";
import { HeroView } from "../src/components/HeroView";

// ============================================================================
// Helper: Mock FusionAgent for AgentBridge Testing
// ============================================================================
interface MockAgentState {
  initialized: boolean;
  initializeCount: number;
  activeModel: string;
  switchedModels: string[];
  cancelCount: number;
  promptsReceived: Array<{ text: string; options?: { model?: string; onEvent?: (ev: AgentEvent) => void } }>;
  eventsToEmitOnPrompt?: AgentEvent[];
}

function createMockAgent(initialEvents?: AgentEvent[]) {
  const state: MockAgentState = {
    initialized: false,
    initializeCount: 0,
    activeModel: DEFAULT_FUSION_MODEL.id,
    switchedModels: [],
    cancelCount: 0,
    promptsReceived: [],
    eventsToEmitOnPrompt: initialEvents,
  };

  const agent: Partial<FusionAgent> = {
    async initialize() {
      state.initializeCount++;
      state.initialized = true;
    },
    async switchModel(model: string) {
      state.switchedModels.push(model);
      state.activeModel = model;
    },
    async cancel() {
      state.cancelCount++;
    },
    async prompt(text: string, options?: { model?: string; onEvent?: (ev: AgentEvent) => void }) {
      state.promptsReceived.push({ text, options });

      const events = state.eventsToEmitOnPrompt ?? [];

      return new ReadableStream<AgentEvent>({
        start(controller) {
          for (const ev of events) {
            if (options?.onEvent) {
              options.onEvent(ev);
            }
            controller.enqueue(ev);
          }
          controller.close();
        },
      });
    },
  };

  return { agent: agent as FusionAgent, state };
}

// ============================================================================
// 1. FUSION_MODELS Test Suite
// ============================================================================
describe("Desktop E2E: FUSION_MODELS Catalog", () => {
  it("defines exactly 4 canonical models matching backend catalog", () => {
    expect(FUSION_MODELS).toBeDefined();
    expect(Array.isArray(FUSION_MODELS)).toBe(true);
    expect(FUSION_MODELS.length).toBe(4);
  });

  it("includes all 4 canonical models with exact IDs, names, and badges", () => {
    const expectedModels: Array<{
      id: string;
      name: string;
      shortName: string;
      badge: string;
      contextWindow: string;
    }> = [
      {
        id: "deepseek-v4-flash-0731",
        name: "DeepSeek 4 0731 Flash",
        shortName: "DeepSeek 4 Flash",
        badge: "Default · 1M",
        contextWindow: "1M context",
      },
      {
        id: "deepseek-v4-flash-0731-fast",
        name: "DeepSeek 4 0731 Flash Fast",
        shortName: "DeepSeek 4 Fast",
        badge: "Speed",
        contextWindow: "1M context",
      },
      {
        id: "glm-5.3-flash",
        name: "GLM 5.3 Flash",
        shortName: "GLM 5.3 Flash",
        badge: "1M Context",
        contextWindow: "1M context",
      },
      {
        id: "minimax-m2.7",
        name: "MiniMax M2.7",
        shortName: "MiniMax M2.7",
        badge: "Reasoning",
        contextWindow: "200K context",
      },
    ];

    for (const expected of expectedModels) {
      const model = FUSION_MODELS.find((m) => m.id === expected.id);
      expect(model).toBeDefined();
      expect(model?.id).toBe(expected.id);
      expect(model?.name).toBe(expected.name);
      expect(model?.shortName).toBe(expected.shortName);
      expect(model?.badge).toBe(expected.badge);
      expect(model?.contextWindow).toBe(expected.contextWindow);
      expect(typeof model?.description).toBe("string");
      expect(model!.description.length).toBeGreaterThan(0);
    }
  });

  it("configures DEFAULT_FUSION_MODEL as DeepSeek 4 Flash (deepseek-v4-flash-0731)", () => {
    expect(DEFAULT_FUSION_MODEL).toBeDefined();
    expect(DEFAULT_FUSION_MODEL.id).toBe("deepseek-v4-flash-0731");
    expect(DEFAULT_FUSION_MODEL.name).toBe("DeepSeek 4 0731 Flash");
    expect(DEFAULT_FUSION_MODEL.badge).toBe("Default · 1M");
    expect(DEFAULT_FUSION_MODEL).toBe(FUSION_MODELS[0]);
  });

  it("looks up models by exact ID using getModelById", () => {
    const glm = getModelById("glm-5.3-flash");
    expect(glm).toBeDefined();
    expect(glm?.name).toBe("GLM 5.3 Flash");
    expect(glm?.badge).toBe("1M Context");

    const unknown = getModelById("non-existent-model");
    expect(unknown).toBeUndefined();
  });

  it("looks up models flexibly by ID or shortName with fallback using getFusionModel", () => {
    // Exact ID
    expect(getFusionModel("minimax-m2.7").id).toBe("minimax-m2.7");

    // Case-insensitive short name
    expect(getFusionModel("deepseek 4 fast").id).toBe("deepseek-v4-flash-0731-fast");
    expect(getFusionModel("GLM 5.3 Flash").id).toBe("glm-5.3-flash");

    // Undefined or empty fallback
    expect(getFusionModel(undefined).id).toBe(DEFAULT_FUSION_MODEL.id);
    expect(getFusionModel("").id).toBe(DEFAULT_FUSION_MODEL.id);
    expect(getFusionModel("unknown-model").id).toBe(DEFAULT_FUSION_MODEL.id);
  });

  it("correctly validates model IDs using isValidModelId", () => {
    expect(isValidModelId("deepseek-v4-flash-0731")).toBe(true);
    expect(isValidModelId("deepseek-v4-flash-0731-fast")).toBe(true);
    expect(isValidModelId("glm-5.3-flash")).toBe(true);
    expect(isValidModelId("minimax-m2.7")).toBe(true);

    expect(isValidModelId("gpt-4o")).toBe(false);
    expect(isValidModelId("claude-3-5-sonnet")).toBe(false);
    expect(isValidModelId("")).toBe(false);
  });
});

// ============================================================================
// 2. AgentBridge Test Suite
// ============================================================================
describe("Desktop E2E: AgentBridge", () => {
  describe("Mock Start & Lifecycle", () => {
    it("delegates start() to agent.initialize()", async () => {
      const { agent, state } = createMockAgent();
      const bridge = new AgentBridge({ agent });

      expect(state.initialized).toBe(false);
      expect(state.initializeCount).toBe(0);

      await bridge.start();

      expect(state.initialized).toBe(true);
      expect(state.initializeCount).toBe(1);
    });

    it("ensures start() is idempotent and does not re-initialize", async () => {
      const { agent, state } = createMockAgent();
      const bridge = new AgentBridge({ agent });

      await bridge.start();
      await bridge.start();
      await bridge.start();

      expect(state.initializeCount).toBe(1);
    });

    it("auto-starts agent on prompt() if start() was not explicitly called", async () => {
      const { agent, state } = createMockAgent();
      const bridge = new AgentBridge({ agent });

      expect(state.initialized).toBe(false);
      await bridge.prompt("Hello initial auto-start");

      expect(state.initialized).toBe(true);
      expect(state.initializeCount).toBe(1);
    });

    it("delegates cancel() to agent.cancel()", async () => {
      const { agent, state } = createMockAgent();
      const bridge = new AgentBridge({ agent });

      await bridge.cancel();
      expect(state.cancelCount).toBe(1);

      await bridge.cancel();
      expect(state.cancelCount).toBe(2);
    });
  });

  describe("Prompt Transmission & Model Switching", () => {
    it("transmits prompt text and active model to underlying agent", async () => {
      const { agent, state } = createMockAgent();
      const bridge = new AgentBridge({ agent, defaultModel: "deepseek-v4-flash-0731" });

      await bridge.prompt("Write a binary search in Rust");

      expect(state.promptsReceived.length).toBe(1);
      expect(state.promptsReceived[0].text).toBe("Write a binary search in Rust");
      expect(state.promptsReceived[0].options?.model).toBe("deepseek-v4-flash-0731");
    });

    it("switches model when a custom model is passed to prompt()", async () => {
      const { agent, state } = createMockAgent();
      const bridge = new AgentBridge({ agent, defaultModel: "deepseek-v4-flash-0731" });

      await bridge.prompt("Analyze this algorithm", "minimax-m2.7");

      expect(state.switchedModels).toContain("minimax-m2.7");
      expect(state.promptsReceived[0].options?.model).toBe("minimax-m2.7");

      // Consecutive prompt without model parameter retains the switched active model
      await bridge.prompt("Now optimize it");
      expect(state.promptsReceived[1].options?.model).toBe("minimax-m2.7");
    });
  });

  describe("Event Routing: thought, chunk, step, diff, done, error", () => {
    it("routes thought events and suppresses internal spinner strings", () => {
      const { agent } = createMockAgent();
      const bridge = new AgentBridge({ agent });

      const thoughts: string[] = [];
      const unsub = bridge.on("thought", (th) => thoughts.push(th));

      // Real thought
      bridge.handleAgentEvent({
        type: "thinking_delta",
        delta: "Formulating architectural plan for the desktop UI...",
      });

      // Internal spinner message that should be suppressed
      bridge.handleAgentEvent({
        type: "thinking_delta",
        delta: "Thinking...",
      });
      bridge.handleAgentEvent({
        type: "thinking_delta",
        delta: "Waiting for model response...",
      });
      // Another real thought
      bridge.handleAgentEvent({
        type: "thinking_delta",
        delta: "Deciding on responsive Tailwind layout.",
      });

      expect(thoughts).toEqual([
        "Formulating architectural plan for the desktop UI...",
        "Deciding on responsive Tailwind layout.",
      ]);

      unsub();
      bridge.handleAgentEvent({
        type: "thinking_delta",
        delta: "After unsub, should not receive",
      });
      expect(thoughts.length).toBe(2);
    });

    it("routes chunk events and suppresses spinner strings", () => {
      const { agent } = createMockAgent();
      const bridge = new AgentBridge({ agent });

      const chunks: string[] = [];
      bridge.on("chunk", (c) => chunks.push(c));

      // Valid text chunks
      bridge.handleAgentEvent({
        type: "text_delta",
        delta: "Here is the implementation of the desktop client.",
      });

      // Spinner string suppression
      bridge.handleAgentEvent({
        type: "text_delta",
        delta: "Waiting for model response...",
      });

      bridge.handleAgentEvent({
        type: "text_delta",
        delta: "\n\n```typescript\nexport const ready = true;\n```",
      });

      expect(chunks.length).toBe(2);
      expect(chunks[0]).toBe("Here is the implementation of the desktop client.");
      expect(chunks[1]).toBe("\n\n```typescript\nexport const ready = true;\n```");
    });

    it("routes tool_started and tool_finished to step events with human-friendly titles", () => {
      const { agent } = createMockAgent();
      const bridge = new AgentBridge({ agent });

      const steps: TurnStep[] = [];
      bridge.on("step", (s) => steps.push(s));

      // Tool started: bash
      bridge.handleAgentEvent({
        type: "tool_started",
        id: "step-101",
        name: "bash",
        args: { command: "bun test" },
      });

      expect(steps.length).toBe(1);
      expect(steps[0].id).toBe("step-101");
      expect(steps[0].title).toBe("Ran bun test");
      expect(steps[0].status).toBe("running");

      // Tool finished: bash completed
      bridge.handleAgentEvent({
        type: "tool_finished",
        id: "step-101",
        name: "bash",
        args: { command: "bun test" },
        success: true,
        output: "109 pass, 0 fail",
      });

      expect(steps.length).toBe(2);
      expect(steps[1].id).toBe("step-101");
      expect(steps[1].title).toBe("Ran bun test");
      expect(steps[1].status).toBe("completed");
      expect(steps[1].details).toBe("109 pass, 0 fail");

      // Tool finished with failure
      bridge.handleAgentEvent({
        type: "tool_finished",
        id: "step-102",
        name: "read_file",
        args: { path: "nonexistent.ts" },
        success: false,
        output: "ENOENT: file not found",
      });

      expect(steps.length).toBe(3);
      expect(steps[2].id).toBe("step-102");
      expect(steps[2].title).toBe("Read nonexistent.ts");
      expect(steps[2].status).toBe("failed");
      expect(steps[2].details).toBe("ENOENT: file not found");
    });

    it("routes diff events directly and extracts diff patches from tool_finished", () => {
      const { agent } = createMockAgent();
      const bridge = new AgentBridge({ agent });

      const diffs: string[] = [];
      bridge.on("diff", (d) => diffs.push(d));

      const samplePatch = [
        "diff --git a/src/App.tsx b/src/App.tsx",
        "--- a/src/App.tsx",
        "+++ b/src/App.tsx",
        "@@ -1,4 +1,4 @@",
        "-const v = 1;",
        "+const v = 2;",
      ].join("\n");

      // 1. Through explicit "diff" event
      bridge.handleAgentEvent({
        type: "diff",
        patch: samplePatch,
      });

      expect(diffs.length).toBe(1);
      expect(diffs[0]).toBe(samplePatch);

      // 2. Through tool_finished output containing git diff
      bridge.handleAgentEvent({
        type: "tool_finished",
        id: "step-edit-1",
        name: "edit_file",
        args: { path: "src/App.tsx" },
        success: true,
        output: samplePatch,
      });

      expect(diffs.length).toBe(2);
      expect(diffs[1]).toBe(samplePatch);
    });

    it("routes finished events to done callback with token stats", () => {
      const { agent } = createMockAgent();
      const bridge = new AgentBridge({ agent });

      let receivedStats: TurnDoneStats | undefined;
      bridge.on("done", (stats) => {
        receivedStats = stats;
      });

      bridge.handleAgentEvent({
        type: "finished",
        usage: { total_tokens: 3420 },
      });

      expect(receivedStats).toBeDefined();
      expect(receivedStats?.tokens).toBe(3420);
    });

    it("routes error events and emits standard Error objects", () => {
      const { agent } = createMockAgent();
      const bridge = new AgentBridge({ agent });

      let caughtError: Error | null = null;
      bridge.on("error", (err) => {
        caughtError = err;
      });

      bridge.handleAgentEvent({
        type: "error",
        message: "RPC sidecar disconnected unexpectedly",
      });

      expect(caughtError).toBeInstanceOf(Error);
      expect((caughtError as unknown as Error)?.message).toBe("RPC sidecar disconnected unexpectedly");
    });

    it("executes end-to-end prompt() stream, dispatching all event categories and completing with done", async () => {
      const mockEvents: AgentEvent[] = [
        {
          type: "thinking_delta",
          delta: "Checking workspace structure",
        } as unknown as AgentEvent,
        {
          type: "tool_started",
          id: "step-1",
          name: "read_file",
          args: { path: "package.json" },
        } as unknown as AgentEvent,
        {
          type: "tool_finished",
          id: "step-1",
          name: "read_file",
          args: { path: "package.json" },
          success: true,
          output: '{"name": "fusion-desktop"}',
        } as unknown as AgentEvent,
        {
          type: "text_delta",
          delta: "I found package.json for fusion-desktop.",
        } as unknown as AgentEvent,
        {
          type: "finished",
          usage: { total_tokens: 280 },
        } as unknown as AgentEvent,
      ];

      const { agent } = createMockAgent(mockEvents);
      const bridge = new AgentBridge({ agent });

      const thoughts: string[] = [];
      const chunks: string[] = [];
      const steps: TurnStep[] = [];
      let doneFired = false;

      bridge.on("thought", (t) => thoughts.push(t));
      bridge.on("chunk", (c) => chunks.push(c));
      bridge.on("step", (s) => steps.push(s));
      bridge.on("done", () => {
        doneFired = true;
      });

      await bridge.prompt("What package is this?");

      expect(thoughts).toContain("Checking workspace structure");
      expect(steps.length).toBe(2); // started + completed
      expect(steps[0].title).toBe("Read package.json");
      expect(steps[0].status).toBe("running");
      expect(steps[1].status).toBe("completed");
      expect(chunks).toContain("I found package.json for fusion-desktop.");
      expect(doneFired).toBe(true);
    });
  });

  describe("DSML Markup Parsing and Stripping", () => {
    it("strips DSML call tags and parameters from text", () => {
      const rawText =
        'Checking file. <|DSML|call:read_file><|DSML|parameter name="path">src/main.rs</|DSML|parameter></|DSML|call> Done reading.';
      const { cleanText, steps } = parseAndStripDsml(rawText);

      expect(cleanText.replace(/\s+/g, " ").trim()).toBe("Checking file. Done reading.");
      expect(cleanText).not.toContain("DSML");
      expect(cleanText).not.toContain("read_file");

      expect(steps.length).toBe(1);
      expect(steps[0].title).toBe("Read src/main.rs");
      expect(steps[0].status).toBe("completed");
    });

    it("supports full-width pipe DSML variants (<｜DSML｜...>)", () => {
      const rawText =
        'Searching files: <｜DSML｜call:grep><｜DSML｜parameter name="query">FusionAgent</｜DSML｜parameter></｜DSML｜call>';
      const { cleanText, steps } = parseAndStripDsml(rawText);

      expect(cleanText.trim()).toBe("Searching files:");
      expect(cleanText).not.toContain("DSML");

      expect(steps.length).toBe(1);
      expect(steps[0].title).toBe('Searched "FusionAgent"');
    });

    it("parses bash command tool calls from DSML", () => {
      const rawText =
        '<|DSML|call:bash><|DSML|parameter name="command">cargo check --workspace</|DSML|parameter></|DSML|call>';
      const { cleanText, steps } = parseAndStripDsml(rawText);

      expect(cleanText).toBe("");
      expect(steps.length).toBe(1);
      expect(steps[0].title).toBe("Ran cargo check --workspace");
    });

    it("parses edit_file tool calls from DSML", () => {
      const rawText =
        '<|DSML|call:edit_file><|DSML|parameter name="path">src/App.tsx</|DSML|parameter><|DSML|parameter name="patch">@@ ...</|DSML|parameter></|DSML|call>';
      const { cleanText, steps } = parseAndStripDsml(rawText);

      expect(cleanText).toBe("");
      expect(steps.length).toBe(1);
      expect(steps[0].title).toBe("Edited src/App.tsx");
    });

    it("integrates DSML stripping in AgentBridge text_delta events", () => {
      const { agent } = createMockAgent();
      const bridge = new AgentBridge({ agent });

      const chunks: string[] = [];
      const steps: TurnStep[] = [];

      bridge.on("chunk", (c) => chunks.push(c));
      bridge.on("step", (s) => steps.push(s));

      // Emitting text_delta with embedded DSML
      bridge.handleAgentEvent({
        type: "text_delta",
        delta:
          'Let me inspect the dependencies. <|DSML|call:read_file><|DSML|parameter name="path">package.json</|DSML|parameter></|DSML|call>',
      });

      expect(steps.length).toBe(1);
      expect(steps[0].title).toBe("Read package.json");
      expect(chunks.length).toBe(1);
      expect(chunks[0].trim()).toBe("Let me inspect the dependencies.");
      expect(chunks[0]).not.toContain("DSML");
    });

    it("suppresses chunk emission when text is purely DSML markup", () => {
      const { agent } = createMockAgent();
      const bridge = new AgentBridge({ agent });

      const chunks: string[] = [];
      const steps: TurnStep[] = [];

      bridge.on("chunk", (c) => chunks.push(c));
      bridge.on("step", (s) => steps.push(s));

      bridge.handleAgentEvent({
        type: "text_delta",
        delta:
          '<|DSML|call:bash><|DSML|parameter name="command">git status</|DSML|parameter></|DSML|call>',
      });

      expect(steps.length).toBe(1);
      expect(steps[0].title).toBe("Ran git status");
      expect(chunks.length).toBe(0); // Suppressed empty clean text
    });
  });
});

// ============================================================================
// 3. Component Sanity Test Suite
// ============================================================================
describe("Desktop E2E: Component Sanity (Render & Instantiation)", () => {
  it("imports all 6 canonical chat and prompt components without error", () => {
    expect(ThinkingRow).toBeDefined();
    expect(UserMessage).toBeDefined();
    expect(ToolCallRow).toBeDefined();
    expect(DiffView).toBeDefined();
    expect(Composer).toBeDefined();
    expect(HeroView).toBeDefined();
  });

  describe("ThinkingRow", () => {
    it("instantiates and renders null/empty when not generating and thought is empty", () => {
      const html = renderToStaticMarkup(
        React.createElement(ThinkingRow, {
          thought: "",
          isGenerating: false,
        })
      );
      expect(html).toBe("");
    });

    it("instantiates and renders 'Thinking...' when isGenerating is true without thought", () => {
      const html = renderToStaticMarkup(
        React.createElement(ThinkingRow, {
          thought: "",
          isGenerating: true,
        })
      );
      expect(html).toContain("Thinking...");
      expect(html).toContain("animate-spin");
    });

    it("instantiates and renders collapsed thought toggle when thought is provided", () => {
      const thoughtText = "Evaluating asymptotic complexity of candidate algorithms";
      const html = renderToStaticMarkup(
        React.createElement(ThinkingRow, {
          thought: thoughtText,
          isGenerating: false,
        })
      );
      expect(html).toContain("Thought briefly");
      expect(html).toContain("▾");
    });
  });

  describe("UserMessage", () => {
    it("instantiates and renders user prompt bubble with neutral styling", () => {
      const prompt = "Can you create an e2e test for the Tauri desktop workspace?";
      const html = renderToStaticMarkup(
        React.createElement(UserMessage, {
          content: prompt,
        })
      );

      expect(html).toContain(prompt);
      expect(html).toContain("bg-zinc-100");
      expect(html).toContain("text-zinc-900");
      expect(html).toContain("rounded-2xl");
      expect(html).toContain("justify-end");
    });
  });

  describe("ToolCallRow", () => {
    it("instantiates and renders running step with spinner icon", () => {
      const runningStep: TurnStep = {
        id: "step-r1",
        title: "Ran cargo test --lib",
        status: "running",
      };

      const html = renderToStaticMarkup(
        React.createElement(ToolCallRow, {
          step: runningStep,
        })
      );

      expect(html).toContain("Ran cargo test --lib");
      expect(html).toContain("animate-spin");
      expect(html).toContain("bg-zinc-100/90");
    });

    it("instantiates and renders completed step with check icon", () => {
      const completedStep: TurnStep = {
        id: "step-c1",
        title: "Read apps/fusion-desktop/package.json",
        status: "completed",
        details: '{\n  "name": "fusion-desktop"\n}',
      };

      const html = renderToStaticMarkup(
        React.createElement(ToolCallRow, {
          step: completedStep,
        })
      );

      expect(html).toContain("Read apps/fusion-desktop/package.json");
      expect(html).toContain("text-emerald-600");
    });

    it("instantiates and renders failed step with alert icon", () => {
      const failedStep: TurnStep = {
        id: "step-f1",
        title: "Ran invalid command",
        status: "failed",
        details: "command not found",
      };

      const html = renderToStaticMarkup(
        React.createElement(ToolCallRow, {
          step: failedStep,
        })
      );

      expect(html).toContain("Ran invalid command");
      expect(html).toContain("text-rose-500");
    });
  });

  describe("DiffView", () => {
    it("instantiates and renders empty string for empty patch", () => {
      const html = renderToStaticMarkup(
        React.createElement(DiffView, {
          patch: "",
        })
      );
      expect(html).toBe("");
    });

    it("instantiates and renders unified diff patch with file header and additions/deletions", () => {
      const patch = [
        "diff --git a/src/index.ts b/src/index.ts",
        "--- a/src/index.ts",
        "+++ b/src/index.ts",
        "@@ -1,3 +1,4 @@",
        " import React from 'react';",
        "-export const version = '1.0';",
        "+export const version = '2.0';",
        "+export const ready = true;",
      ].join("\n");

      const html = renderToStaticMarkup(
        React.createElement(DiffView, {
          patch,
        })
      );

      expect(html).toContain("src/index.ts");
      expect(html).toContain("+2");
      expect(html).toContain("-1");
      expect(html).toContain("text-emerald-600");
      expect(html).toContain("text-rose-600");
      expect(html).toContain("export const version = &#x27;2.0&#x27;;");
    });
  });

  describe("Composer", () => {
    it("instantiates and renders elevated card with default model and action buttons", () => {
      let sentText = "";
      const html = renderToStaticMarkup(
        React.createElement(Composer, {
          onSend: (text) => {
            sentText = text;
          },
        })
      );

      // Card container classes
      expect(html).toContain("max-w-[680px]");
      expect(html).toContain("rounded-2xl");
      expect(html).toContain("bg-white");
      expect(html).toContain("border-zinc-200");

      // Default model badge
      expect(html).toContain(DEFAULT_FUSION_MODEL.shortName);

      // Context info and send button
      expect(html).toContain("main");
      expect(html).toContain("Ask anything, type @ to mention");
    });

    it("instantiates and renders Composer with selected model", () => {
      const targetModel = FUSION_MODELS[3]; // MiniMax M2.7
      const html = renderToStaticMarkup(
        React.createElement(Composer, {
          onSend: () => {},
          selectedModel: targetModel.id,
        })
      );

      expect(html).toContain(targetModel.shortName);
    });

    it("instantiates and renders Composer in generating state", () => {
      let canceled = false;
      const html = renderToStaticMarkup(
        React.createElement(Composer, {
          onSend: () => {},
          onCancel: () => {
            canceled = true;
          },
          isGenerating: true,
        })
      );

      // In generating state, send button transitions to cancel/stop button with Square icon
      expect(html).toBeDefined();
      expect(html.length).toBeGreaterThan(0);
    });
  });

  describe("HeroView", () => {
    it("instantiates and renders HeroView with prompt headline and embedded Composer", () => {
      const html = renderToStaticMarkup(
        React.createElement(HeroView, {
          onSend: () => {},
        })
      );

      // Hero header
      expect(html).toContain("What should we build today?");
      expect(html).toContain("Ask questions, plan features, or generate code with Fusion Agent.");

      // Quick prompt pills
      expect(html).toContain("Build a new feature in React &amp; Tailwind");
      expect(html).toContain("Explain workspace code architecture");

      // Embedded composer
      expect(html).toContain("max-w-[680px]");
      expect(html).toContain(DEFAULT_FUSION_MODEL.shortName);
    });

    it("instantiates and renders HeroView with custom selected model", () => {
      const html = renderToStaticMarkup(
        React.createElement(HeroView, {
          onSend: () => {},
          selectedModel: "glm-5.3-flash",
        })
      );

      expect(html).toContain("GLM 5.3 Flash");
    });
  });

  describe("Integrated Layout Sanity", () => {
    it("renders full turn with UserMessage, ThinkingRow, ToolCallRow, DiffView, and Composer simultaneously", () => {
      const step: TurnStep = {
        id: "step-integration-1",
        title: "Edited src/models.ts",
        status: "completed",
      };

      const patch = [
        "diff --git a/src/models.ts b/src/models.ts",
        "--- a/src/models.ts",
        "+++ b/src/models.ts",
        "@@ -1,2 +1,3 @@",
        "+// Canonical models",
      ].join("\n");

      const integratedTree = React.createElement(
        "div",
        { className: "flex flex-col h-full bg-[#f7f7f8]" },
        React.createElement(UserMessage, { content: "Add comments to models catalog" }),
        React.createElement(ThinkingRow, { thought: "Analyzing file structure...", isGenerating: false }),
        React.createElement(ToolCallRow, { step }),
        React.createElement(DiffView, { patch }),
        React.createElement(Composer, { onSend: () => {} })
      );

      const html = renderToString(integratedTree);

      expect(html).toContain("Add comments to models catalog");
      expect(html).toContain("Thought briefly");
      expect(html).toContain("Edited src/models.ts");
      expect(html).toContain("src/models.ts");
      expect(html).toContain(DEFAULT_FUSION_MODEL.shortName);
    });
  });
});
