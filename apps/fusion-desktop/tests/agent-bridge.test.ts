import { describe, expect, it } from "bun:test";
import type { AgentEvent, FusionAgent } from "@fusioncode/sdk";
import {
  FUSION_MODELS,
  DEFAULT_FUSION_MODEL,
  getModelById,
  getFusionModel,
  isValidModelId,
} from "../src/lib/models";
import {
  AgentBridge,
  formatToolTitle,
  isSpinnerString,
  parseAndStripDsml,
  resolveFusionBinary,
} from "../src/lib/agent-bridge";
import type { TurnStep } from "../src/types";

describe("Model Catalog Lookups", () => {
  it("exports 4 canonical Fusion models matching specifications", () => {
    expect(FUSION_MODELS.length).toBe(4);

    const [m1, m2, m3, m4] = FUSION_MODELS;

    expect(m1.id).toBe("deepseek-v4-flash-0731");
    expect(m1.name).toBe("DeepSeek 4 0731 Flash");
    expect(m1.shortName).toBe("DeepSeek 4 Flash");
    expect(m1.badge).toBe("Default · 1M");

    expect(m2.id).toBe("deepseek-v4-flash-0731-fast");
    expect(m2.name).toBe("DeepSeek 4 0731 Flash Fast");
    expect(m2.shortName).toBe("DeepSeek 4 Fast");
    expect(m2.badge).toBe("Speed");

    expect(m3.id).toBe("glm-5.3-flash");
    expect(m3.name).toBe("GLM 5.3 Flash");
    expect(m3.shortName).toBe("GLM 5.3 Flash");
    expect(m3.badge).toBe("1M Context");

    expect(m4.id).toBe("minimax-m2.7");
    expect(m4.name).toBe("MiniMax M2.7");
    expect(m4.shortName).toBe("MiniMax M2.7");
    expect(m4.badge).toBe("Reasoning");
  });

  it("exports DEFAULT_FUSION_MODEL as the first model (deepseek-v4-flash-0731)", () => {
    expect(DEFAULT_FUSION_MODEL).toBe(FUSION_MODELS[0]);
    expect(DEFAULT_FUSION_MODEL.id).toBe("deepseek-v4-flash-0731");
  });

  it("looks up models by exact ID using getModelById", () => {
    expect(getModelById("deepseek-v4-flash-0731")?.shortName).toBe("DeepSeek 4 Flash");
    expect(getModelById("glm-5.3-flash")?.badge).toBe("1M Context");
    expect(getModelById("minimax-m2.7")?.name).toBe("MiniMax M2.7");
    expect(getModelById("non-existent-model")).toBeUndefined();
    expect(getModelById("")).toBeUndefined();
  });

  it("looks up models by shortName or ID, with fallback, using getFusionModel", () => {
    expect(getFusionModel("deepseek-v4-flash-0731-fast").id).toBe("deepseek-v4-flash-0731-fast");
    expect(getFusionModel("DeepSeek 4 Fast").id).toBe("deepseek-v4-flash-0731-fast");
    expect(getFusionModel("GLM 5.3 Flash").id).toBe("glm-5.3-flash");
    expect(getFusionModel("MiniMax M2.7").id).toBe("minimax-m2.7");
    // Fallback on unknown or undefined
    expect(getFusionModel("unknown").id).toBe(DEFAULT_FUSION_MODEL.id);
    expect(getFusionModel().id).toBe(DEFAULT_FUSION_MODEL.id);
  });

  it("validates model IDs using isValidModelId", () => {
    expect(isValidModelId("deepseek-v4-flash-0731")).toBe(true);
    expect(isValidModelId("deepseek-v4-flash-0731-fast")).toBe(true);
    expect(isValidModelId("glm-5.3-flash")).toBe(true);
    expect(isValidModelId("minimax-m2.7")).toBe(true);
    expect(isValidModelId("claude-3-opus")).toBe(false);
    expect(isValidModelId("gpt-4o")).toBe(false);
  });
});

describe("Tool Title Formatting", () => {
  it("formats terminal commands as 'Ran <cmd>'", () => {
    expect(formatToolTitle("bash", { command: "cargo test" })).toBe("Ran cargo test");
    expect(formatToolTitle("bash", { cmd: "npm run build" })).toBe("Ran npm run build");
    expect(formatToolTitle("exec", { command: "git status" })).toBe("Ran git status");
    expect(formatToolTitle("terminal", { command: "ls -la" })).toBe("Ran ls -la");
    expect(formatToolTitle("sh", { command: "bun test" })).toBe("Ran bun test");
    expect(formatToolTitle("bash", {})).toBe("Ran terminal command");
  });

  it("formats read file commands as 'Read <file>'", () => {
    expect(formatToolTitle("read", { path: "src/lib/models.ts" })).toBe("Read src/lib/models.ts");
    expect(formatToolTitle("read_file", { path: "./src/lib/agent-bridge.ts" })).toBe(
      "Read src/lib/agent-bridge.ts"
    );
    expect(formatToolTitle("readfile", { file: "package.json" })).toBe("Read package.json");
    expect(formatToolTitle("cat", { path: "README.md" })).toBe("Read README.md");
    expect(formatToolTitle("read", {})).toBe("Read file");
  });

  it("formats edit file commands as 'Edited <file>'", () => {
    expect(formatToolTitle("edit", { path: "src/lib/models.ts" })).toBe("Edited src/lib/models.ts");
    expect(formatToolTitle("edit_file", { path: "./src/App.tsx" })).toBe("Edited src/App.tsx");
    expect(formatToolTitle("editfile", { file: "vite.config.ts" })).toBe("Edited vite.config.ts");
    expect(formatToolTitle("patch", { path: "Cargo.toml" })).toBe("Edited Cargo.toml");
    expect(formatToolTitle("edit", {})).toBe("Edited file");
  });

  it("formats write file commands as 'Wrote <file>'", () => {
    expect(formatToolTitle("write", { path: "src/lib/models.ts" })).toBe("Wrote src/lib/models.ts");
    expect(formatToolTitle("write_file", { path: "./src/index.css" })).toBe("Wrote src/index.css");
    expect(formatToolTitle("write", {})).toBe("Wrote file");
  });

  it("formats search commands as 'Searched \"<query>\"'", () => {
    expect(formatToolTitle("grep", { query: "FUSION_MODELS" })).toBe('Searched "FUSION_MODELS"');
    expect(formatToolTitle("grep", { pattern: "AgentBridge" })).toBe('Searched "AgentBridge"');
    expect(formatToolTitle("search", { query: "useSessionStore" })).toBe(
      'Searched "useSessionStore"'
    );
    expect(formatToolTitle("glob", { pattern: "*.ts" })).toBe('Searched "*.ts"');
    expect(formatToolTitle("web_search", { query: "Tauri v2" })).toBe('Searched "Tauri v2"');
    expect(formatToolTitle("search", {})).toBe("Searched files");
  });

  it("parses JSON-encoded arguments safely", () => {
    expect(formatToolTitle("bash", '{"command": "cargo check"}')).toBe("Ran cargo check");
    expect(formatToolTitle("read", '{"path": "package.json"}')).toBe("Read package.json");
    expect(formatToolTitle("grep", 'invalid json string')).toBe("grep: invalid json string");
  });

  it("falls back to 'Ran <name>' for unknown tools", () => {
    expect(formatToolTitle("custom_tool")).toBe("Ran custom_tool");
    expect(formatToolTitle("ast_grep", { pattern: "$A" })).toBe("Ran ast_grep");
  });
});

describe("Internal Spinner Suppression", () => {
  it("suppresses standard spinner and waiting messages", () => {
    expect(isSpinnerString("Waiting for model response...")).toBe(true);
    expect(isSpinnerString("waiting for model response...")).toBe(true);
    expect(isSpinnerString("Waiting for model response")).toBe(true);
    expect(isSpinnerString("Waiting for response...")).toBe(true);
    expect(isSpinnerString("Waiting for model...")).toBe(true);
    expect(isSpinnerString("Thinking...")).toBe(true);
    expect(isSpinnerString("Thinking")).toBe(true);
    expect(isSpinnerString("thinking")).toBe(true);
    expect(isSpinnerString("   waiting for model response...   ")).toBe(true);
    expect(isSpinnerString("")).toBe(true);
    expect(isSpinnerString("   ")).toBe(true);
  });

  it("allows substantive thoughts and text to pass through", () => {
    expect(isSpinnerString("Thinking about how to structure the models catalog...")).toBe(false);
    expect(isSpinnerString("Waiting for the user confirmation on changes")).toBe(false);
    expect(isSpinnerString("Here is the solution to your request:")).toBe(false);
    expect(isSpinnerString("I will now read the file src/lib/models.ts")).toBe(false);
  });
});

describe("DSML Parsing and Stripping", () => {
  it("parses DeepSeek full-width pipe DSML tool call and emits clean step", () => {
    const raw =
      'Searching the codebase... <｜DSML｜tool_calls><｜DSML｜call:grep><｜DSML｜parameter name="query">FUSION_MODELS</｜DSML｜parameter></｜DSML｜call></｜DSML｜tool_calls>';

    const { cleanText, steps } = parseAndStripDsml(raw);

    expect(steps.length).toBe(1);
    expect(steps[0].title).toBe('Searched "FUSION_MODELS"');
    expect(steps[0].status).toBe("completed");
    expect(cleanText.trim()).toBe("Searching the codebase...");
    expect(cleanText).not.toContain("DSML");
    expect(cleanText).not.toContain("grep");
    expect(cleanText).not.toContain("FUSION_MODELS");
  });

  it("parses ASCII pipe DSML tool call with search query", () => {
    const raw =
      '<|DSML|call:search><|DSML|parameter name="query">AgentBridge</|DSML|parameter></|DSML|call>';

    const { cleanText, steps } = parseAndStripDsml(raw);

    expect(steps.length).toBe(1);
    expect(steps[0].title).toBe('Searched "AgentBridge"');
    expect(steps[0].status).toBe("completed");
    expect(cleanText).toBe("");
  });

  it("parses DSML tool calls for bash commands", () => {
    const raw =
      '<|DSML|call:bash><|DSML|parameter name="command">bun test</|DSML|parameter></|DSML|call>';

    const { cleanText, steps } = parseAndStripDsml(raw);

    expect(steps.length).toBe(1);
    expect(steps[0].title).toBe("Ran bun test");
    expect(cleanText).toBe("");
  });

  it("parses DSML tool calls for reading and editing files", () => {
    const rawRead =
      '<|DSML|call:read><|DSML|parameter name="path">./src/lib/models.ts</|DSML|parameter></|DSML|call>';
    const resRead = parseAndStripDsml(rawRead);
    expect(resRead.steps.length).toBe(1);
    expect(resRead.steps[0].title).toBe("Read src/lib/models.ts");

    const rawEdit =
      '<|DSML|call:edit><|DSML|parameter name="path">src/App.tsx</|DSML|parameter></|DSML|call>';
    const resEdit = parseAndStripDsml(rawEdit);
    expect(resEdit.steps.length).toBe(1);
    expect(resEdit.steps[0].title).toBe("Edited src/App.tsx");
  });

  it("parses standalone DSML parameters without call container", () => {
    const raw = '<|DSML|parameter name="query">tailwindcss v4</|DSML|parameter>';
    const { cleanText, steps } = parseAndStripDsml(raw);

    expect(steps.length).toBe(1);
    expect(steps[0].title).toBe('Searched "tailwindcss v4"');
    expect(cleanText).toBe("");
  });

  it("preserves regular text that does not contain DSML markup", () => {
    const raw = "This is a regular assistant response that does not contain any markup.";
    const { cleanText, steps } = parseAndStripDsml(raw);

    expect(steps.length).toBe(0);
    expect(cleanText).toBe(raw);
  });

  it("strips residual thought markers and tool container tags", () => {
    const raw =
      "<｜begin_of_thought｜>Let me think<｜end_of_thought｜>Here is your answer.<tool_call>ignore</tool_call>";
    const { cleanText } = parseAndStripDsml(raw);

    expect(cleanText).not.toContain("begin_of_thought");
    expect(cleanText).not.toContain("end_of_thought");
    expect(cleanText).not.toContain("<tool_call>");
    expect(cleanText).toContain("Here is your answer.");
  });
});

describe("AgentBridge Event Emitter Subscriptions & Lifecycle", () => {
  interface MockAgentHandle {
    isInitialized: boolean;
    initializedCalls: number;
    activeModel: string;
    switchModelCalls: string[];
    cancelCalls: number;
    initialize: () => Promise<void>;
    switchModel: (model: string) => Promise<void>;
    cancel: () => Promise<void>;
    prompt: (text: string, options?: { model?: string; onEvent?: (ev: AgentEvent) => void }) => Promise<ReadableStream<AgentEvent>>;
  }

  function createMockAgent(): MockAgentHandle {
    return {
      isInitialized: false,
      initializedCalls: 0,
      activeModel: "deepseek-v4-flash-0731",
      switchModelCalls: [],
      cancelCalls: 0,
      async initialize() {
        this.initializedCalls++;
        this.isInitialized = true;
      },
      async switchModel(model: string) {
        this.switchModelCalls.push(model);
        this.activeModel = model;
      },
      async cancel() {
        this.cancelCalls++;
      },
      async prompt(_text: string, _options?: { model?: string; onEvent?: (ev: AgentEvent) => void }) {
        return new ReadableStream<AgentEvent>({
          start(controller) {
            controller.close();
          },
        });
      },
    };
  }

  it("initializes via start() and delegates to FusionAgent.initialize", async () => {
    const mockAgent = createMockAgent();
    const bridge = new AgentBridge({ agent: mockAgent as unknown as FusionAgent });

    await bridge.start();
    expect(mockAgent.initializedCalls).toBe(1);

    // Idempotent: subsequent start() should not re-initialize
    await bridge.start();
    expect(mockAgent.initializedCalls).toBe(1);
  });

  it("delegates cancel() to FusionAgent.cancel", async () => {
    const mockAgent = createMockAgent();
    const bridge = new AgentBridge({ agent: mockAgent as unknown as FusionAgent });

    await bridge.cancel();
    expect(mockAgent.cancelCalls).toBe(1);
  });

  it("switches model when a different model is provided in prompt()", async () => {
    const mockAgent = createMockAgent();
    const bridge = new AgentBridge({ agent: mockAgent as unknown as FusionAgent });

    await bridge.prompt("Hello world", "glm-5.3-flash");
    expect(mockAgent.switchModelCalls).toContain("glm-5.3-flash");
  });

  it("subscribes and receives thought events, suppressing spinner strings", () => {
    const mockAgent = createMockAgent();
    const bridge = new AgentBridge({ agent: mockAgent as unknown as FusionAgent });

    const receivedThoughts: string[] = [];
    const unsub = bridge.on("thought", (thought) => {
      receivedThoughts.push(thought);
    });

    // Valid thought
    bridge.handleAgentEvent({
      type: "thinking_delta",
      delta: "Considering optimal data structures...",
    });

    // Suppressed spinner thoughts
    bridge.handleAgentEvent({
      type: "thinking_delta",
      delta: "Waiting for model response...",
    });

    bridge.handleAgentEvent({
      type: "thinking_delta",
      delta: "Thinking...",
    });

    // Another valid thought
    bridge.handleAgentEvent({
      type: "thinking_delta",
      delta: "Selecting Map for fast lookups.",
    });

    expect(receivedThoughts).toEqual([
      "Considering optimal data structures...",
      "Selecting Map for fast lookups.",
    ]);

    // Test unsubscribe
    unsub();
    bridge.handleAgentEvent({
      type: "thinking_delta",
      delta: "This should not be captured.",
    });

    expect(receivedThoughts.length).toBe(2);
  });

  it("subscribes and receives chunk events, parsing and stripping DSML", () => {
    const mockAgent = createMockAgent();
    const bridge = new AgentBridge({ agent: mockAgent as unknown as FusionAgent });

    const chunks: string[] = [];
    const steps: TurnStep[] = [];

    bridge.on("chunk", (chunk) => chunks.push(chunk));
    bridge.on("step", (step) => steps.push(step));

    // Chunk with mixed DSML search
    bridge.handleAgentEvent({
      type: "text_delta",
      delta:
        'I will look up the model definitions. <|DSML|call:grep><|DSML|parameter name="query">DEFAULT_FUSION_MODEL</|DSML|parameter></|DSML|call>',
    });

    expect(steps.length).toBe(1);
    expect(steps[0].title).toBe('Searched "DEFAULT_FUSION_MODEL"');
    expect(chunks.length).toBe(1);
    expect(chunks[0].trim()).toBe("I will look up the model definitions.");
    expect(chunks[0]).not.toContain("DSML");
  });

  it("suppresses chunk events that are spinner strings or stripped to empty", () => {
    const mockAgent = createMockAgent();
    const bridge = new AgentBridge({ agent: mockAgent as unknown as FusionAgent });

    const chunks: string[] = [];
    bridge.on("chunk", (chunk) => chunks.push(chunk));

    // Spinner chunk
    bridge.handleAgentEvent({
      type: "text_delta",
      delta: "Waiting for model response...",
    });

    // Pure DSML markup chunk (should emit step, but no chunk)
    bridge.handleAgentEvent({
      type: "text_delta",
      delta: '<|DSML|call:search><|DSML|parameter name="query">test</|DSML|parameter></|DSML|call>',
    });

    expect(chunks.length).toBe(0);
  });

  it("dispatches tool_started and tool_finished as step events with formatted titles", () => {
    const mockAgent = createMockAgent();
    const bridge = new AgentBridge({ agent: mockAgent as unknown as FusionAgent });

    const steps: TurnStep[] = [];
    bridge.on("step", (step) => steps.push(step));

    // Tool start
    bridge.handleAgentEvent({
      type: "tool_started",
      id: "tool-1",
      name: "bash",
      args: { command: "cargo test" },
    });

    expect(steps.length).toBe(1);
    expect(steps[0].id).toBe("tool-1");
    expect(steps[0].title).toBe("Ran cargo test");
    expect(steps[0].status).toBe("running");

    // Tool finish
    bridge.handleAgentEvent({
      type: "tool_finished",
      id: "tool-1",
      name: "bash",
      args: { command: "cargo test" },
      success: true,
      output: "test result: ok. 4 passed",
      duration_ms: 120,
    });

    expect(steps.length).toBe(2);
    expect(steps[1].id).toBe("tool-1");
    expect(steps[1].title).toBe("Ran cargo test");
    expect(steps[1].status).toBe("completed");
    expect(steps[1].details).toBe("test result: ok. 4 passed");
  });

  it("dispatches diff events when tool_finished contains a unified diff or diff event occurs", () => {
    const mockAgent = createMockAgent();
    const bridge = new AgentBridge({ agent: mockAgent as unknown as FusionAgent });

    const diffs: string[] = [];
    bridge.on("diff", (diff) => diffs.push(diff));

    const samplePatch = "--- a/src/lib/models.ts\n+++ b/src/lib/models.ts\n@@ -1,3 +1,3 @@\n-old\n+new";

    // Via tool_finished output
    bridge.handleAgentEvent({
      type: "tool_finished",
      id: "tool-edit-1",
      name: "edit",
      args: { path: "src/lib/models.ts" },
      success: true,
      output: samplePatch,
      duration_ms: 50,
    });

    expect(diffs).toContain(samplePatch);

    // Via explicit diff event
    bridge.handleAgentEvent({
      type: "diff",
      patch: "diff --git a/file b/file",
    });

    expect(diffs).toContain("diff --git a/file b/file");
  });

  it("dispatches done and error events correctly", () => {
    const mockAgent = createMockAgent();
    const bridge = new AgentBridge({ agent: mockAgent as unknown as FusionAgent });

    let doneReceived = false;
    let errorReceived: Error | null = null;

    bridge.on("done", () => {
      doneReceived = true;
    });
    bridge.on("error", (err) => {
      errorReceived = err;
    });

    bridge.handleAgentEvent({
      type: "finished",
      usage: { prompt_tokens: 500, completion_tokens: 1020, total_tokens: 1520 },
    });

    expect(doneReceived).toBe(true);

    bridge.handleAgentEvent({
      type: "error",
      message: "Connection timeout to model endpoint",
    });

    expect(errorReceived).toBeInstanceOf(Error);
    expect((errorReceived as Error | null)?.message).toBe("Connection timeout to model endpoint");
  });

  it("supports multiple listeners and independent unsubscribe", () => {
    const mockAgent = createMockAgent();
    const bridge = new AgentBridge({ agent: mockAgent as unknown as FusionAgent });

    let countA = 0;
    let countB = 0;

    const unsubA = bridge.on("chunk", () => countA++);
    const unsubB = bridge.on("chunk", () => countB++);

    bridge.emit("chunk", "Hello");
    expect(countA).toBe(1);
    expect(countB).toBe(1);

    unsubA();

    bridge.emit("chunk", "World");
    expect(countA).toBe(1);
    expect(countB).toBe(2);

    unsubB();

    bridge.emit("chunk", "!");
    expect(countA).toBe(1);
    expect(countB).toBe(2);
  });
});

describe("Binary Resolution", () => {
  it("resolves existing binary or returns null for non-existent path", () => {
    expect(resolveFusionBinary("/non/existent/path/fusion")).toBeNull();
    expect(resolveFusionBinary("")).toBeNull();

    // Default resolution finds dev candidate if built
    const resolved = resolveFusionBinary();
    if (resolved) {
      expect(typeof resolved).toBe("string");
      expect(resolved).toContain("fusion");
    }
  });
});
