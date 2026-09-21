import { describe, expect, it, beforeEach, afterEach } from "bun:test";
import type { ChildProcess } from "node:child_process";
import { PassThrough } from "node:stream";
import { mkdtempSync, mkdirSync, writeFileSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import {
  parseJsonRpcLine,
  resolveFusionBinary,
  AcpClient,
  type TurnStepEvent,
  type TurnDoneEvent,
} from "../src/bridge/acp-client";

describe("ACP Client Protocol - parseJsonRpcLine", () => {
  it("parses valid JSON-RPC 2.0 notification lines", () => {
    const parsed = parseJsonRpcLine('{"jsonrpc":"2.0","method":"turn/chunk","params":{"delta":"hello"}}\n');
    expect(parsed).not.toBeNull();
    expect(parsed?.jsonrpc).toBe("2.0");
    expect(parsed?.method).toBe("turn/chunk");
    const params = parsed?.params as Record<string, unknown> | undefined;
    expect(params?.delta).toBe("hello");
  });

  it("parses valid JSON-RPC 2.0 request lines", () => {
    const parsed = parseJsonRpcLine('{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"workspace":"/tmp"}}\n');
    expect(parsed).not.toBeNull();
    expect(parsed?.id).toBe(1);
    expect(parsed?.method).toBe("initialize");
    const params = parsed?.params as Record<string, unknown> | undefined;
    expect(params?.workspace).toBe("/tmp");
  });

  it("parses valid JSON-RPC 2.0 response lines with result", () => {
    const parsed = parseJsonRpcLine('{"jsonrpc":"2.0","id":42,"result":{"status":"ok"}}\n');
    expect(parsed).not.toBeNull();
    expect(parsed?.id).toBe(42);
    const result = parsed?.result as Record<string, unknown> | undefined;
    expect(result?.status).toBe("ok");
    expect(parsed?.error).toBeUndefined();
  });

  it("parses valid JSON-RPC 2.0 error responses", () => {
    const parsed = parseJsonRpcLine('{"jsonrpc":"2.0","id":42,"error":{"code":-32600,"message":"Invalid request"}}\n');
    expect(parsed).not.toBeNull();
    expect(parsed?.id).toBe(42);
    expect(parsed?.error?.code).toBe(-32600);
    expect(parsed?.error?.message).toBe("Invalid request");
  });

  it("returns null for empty or whitespace-only lines", () => {
    expect(parseJsonRpcLine("")).toBeNull();
    expect(parseJsonRpcLine("   \n\t  ")).toBeNull();
  });

  it("returns null for malformed JSON strings", () => {
    expect(parseJsonRpcLine("not-json")).toBeNull();
    expect(parseJsonRpcLine("{unclosed json")).toBeNull();
    expect(parseJsonRpcLine("undefined")).toBeNull();
  });

  it("returns null for non-object JSON values", () => {
    expect(parseJsonRpcLine("123\n")).toBeNull();
    expect(parseJsonRpcLine('"a string"\n')).toBeNull();
    expect(parseJsonRpcLine("[1, 2, 3]\n")).toBeNull();
    expect(parseJsonRpcLine("true\n")).toBeNull();
    expect(parseJsonRpcLine("null\n")).toBeNull();
  });

  it("returns null when jsonrpc version is not 2.0", () => {
    expect(parseJsonRpcLine('{"jsonrpc":"1.0","id":1,"method":"test"}\n')).toBeNull();
    expect(parseJsonRpcLine('{"id":1,"method":"test"}\n')).toBeNull();
  });
});

describe("ACP Client Protocol - resolveFusionBinary", () => {
  let tempDir: string;

  beforeEach(() => {
    tempDir = mkdtempSync(join(tmpdir(), "fusion-test-"));
  });

  afterEach(() => {
    try {
      rmSync(tempDir, { recursive: true, force: true });
    } catch {
      // ignore
    }
  });

  it("returns customPath when provided and the file exists", () => {
    const fakeBin = join(tempDir, "fusion-custom");
    writeFileSync(fakeBin, "#!/bin/sh\nexit 0\n", { mode: 0o755 });

    const resolved = resolveFusionBinary(fakeBin);
    expect(resolved).toBe(resolve(fakeBin));
  });

  it("returns null when customPath is provided but the file does not exist", () => {
    const missingBin = join(tempDir, "non-existent-binary");
    const resolved = resolveFusionBinary(missingBin);
    expect(resolved).toBeNull();
  });

  it("resolves from FUSION_BINARY_PATH env variable", () => {
    const fakeBin = join(tempDir, "fusion-env");
    writeFileSync(fakeBin, "#!/bin/sh\nexit 0\n", { mode: 0o755 });

    const resolved = resolveFusionBinary(undefined, {
      env: { FUSION_BINARY_PATH: fakeBin },
    });
    expect(resolved).toBe(resolve(fakeBin));
  });

  it("resolves from packaged app adjacent path", () => {
    const fakeAppDir = join(tempDir, "MacOS");
    mkdirSync(fakeAppDir, { recursive: true });
    const fakeExec = join(fakeAppDir, "app-runner");
    const fakeBin = join(fakeAppDir, "fusion");
    writeFileSync(fakeBin, "#!/bin/sh\nexit 0\n", { mode: 0o755 });
    const resolved = resolveFusionBinary(undefined, {
      execPath: fakeExec,
      env: {},
    });
    expect(resolved).toBe(resolve(fakeBin));
  });

  it("resolves from dev paths relative to cwd or baseDir", () => {
    const resolved = resolveFusionBinary(undefined, {
      cwd: tempDir,
      baseDir: tempDir,
      env: {},
      existsSync: (p) => p.includes("target/release/fusion"),
    });
    expect(resolved).not.toBeNull();
    expect(resolved).toContain("target/release/fusion");
  });

  it("returns null when no candidate exists", () => {
    const resolved = resolveFusionBinary(undefined, {
      cwd: tempDir,
      baseDir: tempDir,
      env: {},
      execPath: join(tempDir, "dummy-exec"),
      existsSync: () => false,
    });
    expect(resolved).toBeNull();
  });
});

describe("ACP Client Protocol - AcpClient Request/Response & Lifecycle", () => {
  it("generates incrementing request IDs and resolves with response result", async () => {
    const client = new AcpClient();
    const mockStdin = new PassThrough();
    const mockStdout = new PassThrough();
    let capturedStdin = "";

    mockStdin.on("data", (d: Buffer) => {
      capturedStdin += d.toString();
    });

    client.attachStreams(mockStdout, mockStdin);

    const reqPromise = client.sendRequest<{ greeting: string }>("greet", { name: "Fusion" });

    // Verify request formatted and sent to stdin
    expect(capturedStdin).toContain('"jsonrpc":"2.0"');
    expect(capturedStdin).toContain('"method":"greet"');
    expect(capturedStdin).toContain('"id":1');

    // Simulate server response
    mockStdout.write(JSON.stringify({ jsonrpc: "2.0", id: 1, result: { greeting: "Hello, Fusion!" } }) + "\n");

    const result = await reqPromise;
    expect(result.greeting).toBe("Hello, Fusion!");
  });

  it("rejects request on JSON-RPC error response", async () => {
    const client = new AcpClient();
    const mockStdin = new PassThrough();
    const mockStdout = new PassThrough();

    client.attachStreams(mockStdout, mockStdin);

    const reqPromise = client.sendRequest("unknown_method");

    mockStdout.write(
      JSON.stringify({
        jsonrpc: "2.0",
        id: 1,
        error: { code: -32601, message: "Method not found" },
      }) + "\n"
    );

    expect(reqPromise).rejects.toThrow("Method not found");
  });

  it("times out pending requests when timeoutMs elapses", async () => {
    const client = new AcpClient();
    const mockStdin = new PassThrough();
    const mockStdout = new PassThrough();

    client.attachStreams(mockStdout, mockStdin);

    const reqPromise = client.sendRequest("slow_method", {}, 50);

    expect(reqPromise).rejects.toThrow("timed out");
  });

  it("rejects sendRequest if client stream/process is not connected", async () => {
    const client = new AcpClient();
    expect(client.sendRequest("any")).rejects.toThrow("not connected");
  });

  it("sends notifications without expecting response", () => {
    const client = new AcpClient();
    const mockStdin = new PassThrough();
    const mockStdout = new PassThrough();
    let written = "";

    mockStdin.on("data", (d: Buffer) => {
      written += d.toString();
    });

    client.attachStreams(mockStdout, mockStdin);
    client.sendNotification("session/cancel", { sessionId: "s1" });

    expect(written).toContain('"method":"session/cancel"');
    expect(written).toContain('"sessionId":"s1"');
    expect(written).not.toContain('"id":');
  });

  it("provides prompt and cancel helper methods", async () => {
    const client = new AcpClient();
    const mockStdin = new PassThrough();
    const mockStdout = new PassThrough();
    const sent: string[] = [];

    mockStdin.on("data", (d: Buffer) => {
      sent.push(d.toString());
    });

    client.attachStreams(mockStdout, mockStdin);

    // Call prompt
    const promptPromise = client.prompt("Build a feature", "gpt-4o");
    const parsedPrompt = JSON.parse(sent[0]) as {
      jsonrpc: string;
      id: number;
      method: string;
      params: { prompt: string; model?: string };
    };
    expect(parsedPrompt.method).toBe("session/prompt");
    expect(parsedPrompt.params.prompt).toBe("Build a feature");
    expect(parsedPrompt.params.model).toBe("gpt-4o");

    mockStdout.write(JSON.stringify({ jsonrpc: "2.0", id: parsedPrompt.id, result: { stopReason: "end_turn" } }) + "\n");
    await promptPromise;

    // Call cancel
    const cancelPromise = client.cancel();
    const parsedCancel = JSON.parse(sent[1]) as {
      jsonrpc: string;
      id: number;
      method: string;
      params?: Record<string, unknown>;
    };
    expect(parsedCancel.method).toBe("session/cancel");

    mockStdout.write(JSON.stringify({ jsonrpc: "2.0", id: parsedCancel.id, result: null }) + "\n");
    await cancelPromise;
  });
});

describe("ACP Client Protocol - Notification Dispatching", () => {
  it("dispatches turn/step events and returns unsubscribe handler", () => {
    const client = new AcpClient();
    const mockStdin = new PassThrough();
    const mockStdout = new PassThrough();
    client.attachStreams(mockStdout, mockStdin);

    const steps: TurnStepEvent[] = [];
    const unsub = client.onStep((step) => {
      steps.push(step);
    });

    mockStdout.write(
      JSON.stringify({
        jsonrpc: "2.0",
        method: "turn/step",
        params: { title: "Searching files", status: "running", details: "glob src/**/*.ts" },
      }) + "\n"
    );

    expect(steps).toHaveLength(1);
    expect(steps[0].title).toBe("Searching files");
    expect(steps[0].status).toBe("running");
    expect(steps[0].details).toBe("glob src/**/*.ts");

    unsub();

    mockStdout.write(
      JSON.stringify({
        jsonrpc: "2.0",
        method: "turn/step",
        params: { title: "Reading file", status: "completed" },
      }) + "\n"
    );

    // No new step received after unsubscribe
    expect(steps).toHaveLength(1);
  });

  it("dispatches turn/chunk events and returns unsubscribe handler", () => {
    const client = new AcpClient();
    const mockStdin = new PassThrough();
    const mockStdout = new PassThrough();
    client.attachStreams(mockStdout, mockStdin);

    let received = "";
    const unsub = client.onChunk((delta) => {
      received += delta;
    });

    mockStdout.write(JSON.stringify({ jsonrpc: "2.0", method: "turn/chunk", params: { delta: "Hello " } }) + "\n");
    mockStdout.write(JSON.stringify({ jsonrpc: "2.0", method: "turn/chunk", params: { delta: "World!" } }) + "\n");

    expect(received).toBe("Hello World!");

    unsub();

    mockStdout.write(JSON.stringify({ jsonrpc: "2.0", method: "turn/chunk", params: { delta: " More" } }) + "\n");
    expect(received).toBe("Hello World!");
  });

  it("dispatches turn/done events with token and duration metrics", () => {
    const client = new AcpClient();
    const mockStdin = new PassThrough();
    const mockStdout = new PassThrough();
    client.attachStreams(mockStdout, mockStdin);

    let doneInfo: TurnDoneEvent | undefined = undefined;
    client.onDone((info) => {
      doneInfo = info;
    });

    mockStdout.write(
      JSON.stringify({
        jsonrpc: "2.0",
        method: "turn/done",
        params: { tokens: 1542, durationMs: 2310 },
      }) + "\n"
    );

    const info = doneInfo as TurnDoneEvent | undefined;
    expect(info).toBeDefined();
    expect(info?.tokens).toBe(1542);
    expect(info?.durationMs).toBe(2310);
  });

  it("bridges session/update rich events to turn callbacks", () => {
    const client = new AcpClient();
    const mockStdin = new PassThrough();
    const mockStdout = new PassThrough();
    client.attachStreams(mockStdout, mockStdin);

    let chunkOutput = "";
    const steps: TurnStepEvent[] = [];
    let doneMetrics: TurnDoneEvent | undefined = undefined;

    client.onChunk((c) => (chunkOutput += c));
    client.onStep((s) => steps.push(s));
    client.onDone((d) => (doneMetrics = d));

    // session/update: agent_message_chunk
    mockStdout.write(
      JSON.stringify({
        jsonrpc: "2.0",
        method: "session/update",
        params: {
          sessionId: "s1",
          update: {
            sessionUpdate: "agent_message_chunk",
            content: { type: "text", text: "Streaming message" },
          },
        },
      }) + "\n"
    );
    expect(chunkOutput).toBe("Streaming message");

    // session/update: tool_call
    mockStdout.write(
      JSON.stringify({
        jsonrpc: "2.0",
        method: "session/update",
        params: {
          sessionId: "s1",
          update: {
            sessionUpdate: "tool_call",
            callId: "call-1",
            name: "read_file",
            args: { path: "src/main.rs" },
          },
        },
      }) + "\n"
    );
    expect(steps).toHaveLength(1);
    expect(steps[0].title).toBe("Tool: read_file");
    expect(steps[0].status).toBe("running");

    // session/update: tool_call_result
    mockStdout.write(
      JSON.stringify({
        jsonrpc: "2.0",
        method: "session/update",
        params: {
          sessionId: "s1",
          update: {
            sessionUpdate: "tool_call_result",
            callId: "call-1",
            name: "read_file",
            output: "file contents",
            success: true,
          },
        },
      }) + "\n"
    );
    expect(steps).toHaveLength(2);
    expect(steps[1].title).toBe("Tool: read_file");
    expect(steps[1].status).toBe("completed");

    // session/update: token_stats
    mockStdout.write(
      JSON.stringify({
        jsonrpc: "2.0",
        method: "session/update",
        params: {
          sessionId: "s1",
          update: {
            sessionUpdate: "token_stats",
            totalTokens: 800,
            durationMs: 1200,
          },
        },
      }) + "\n"
    );
    const metrics = doneMetrics as TurnDoneEvent | undefined;
    expect(metrics).toBeDefined();
    expect(metrics?.tokens).toBe(800);
    expect(metrics?.durationMs).toBe(1200);
  });
});

describe("ACP Client Protocol - Mock Stream Parsing", () => {
  it("handles stream data split across multiple chunks", () => {
    const client = new AcpClient();
    const chunks: string[] = [];
    client.onChunk((c) => chunks.push(c));

    const mockStdin = new PassThrough();
    const mockStdout = new PassThrough();
    client.attachStreams(mockStdout, mockStdin);

    // Message split in three parts
    mockStdout.write('{"jsonrpc":"2.0","method"');
    mockStdout.write(':"turn/chunk","params":');
    mockStdout.write('{"delta":"partial-chunk"}}\n');

    expect(chunks).toEqual(["partial-chunk"]);
  });

  it("handles multiple JSON-RPC lines in a single chunk", () => {
    const client = new AcpClient();
    const chunks: string[] = [];
    client.onChunk((c) => chunks.push(c));

    const mockStdin = new PassThrough();
    const mockStdout = new PassThrough();
    client.attachStreams(mockStdout, mockStdin);

    mockStdout.write(
      '{"jsonrpc":"2.0","method":"turn/chunk","params":{"delta":"1"}}\n{"jsonrpc":"2.0","method":"turn/chunk","params":{"delta":"2"}}\n{"jsonrpc":"2.0","method":"turn/chunk","params":{"delta":"3"}}\n'
    );

    expect(chunks).toEqual(["1", "2", "3"]);
  });
});

describe("ACP Client Protocol - Process Lifecycle", () => {
  it("reports isAlive accurately based on stream/process state", () => {
    const client = new AcpClient();
    expect(client.isAlive()).toBe(false);

    const mockStdin = new PassThrough();
    const mockStdout = new PassThrough();
    client.attachStreams(mockStdout, mockStdin);
    expect(client.isAlive()).toBe(true);

    client.kill();
    expect(client.isAlive()).toBe(false);
  });

  it("returns false from spawn when binary is missing and emits error", async () => {
    const client = new AcpClient({ binaryPath: "/non/existent/fusion/binary" });
    let emittedError: Error | null = null;
    client.onError((err) => {
      emittedError = err;
    });

    const success = await client.spawn("/some/workspace");
    expect(success).toBe(false);
    expect(emittedError).not.toBeNull();
    if (emittedError) {
      expect((emittedError as Error).message).toContain("not found");
    }
  });

  it("returns false from isAlive when child process has a signalCode", async () => {
    const client = new AcpClient();
    const mockStdin = new PassThrough();
    const mockStdout = new PassThrough();
    client.attachStreams(mockStdout, mockStdin);

    // Access internals for unit-testing process signal lifecycle
    interface ClientInternals {
      customStreamsActive: boolean;
      streamsConnected: boolean;
      child: {
        killed: boolean;
        exitCode: number | null;
        signalCode: NodeJS.Signals | null;
      } | null;
    }
    const internals = client as unknown as ClientInternals;
    internals.customStreamsActive = false;
    internals.streamsConnected = true;

    const mockProcess = {
      killed: false,
      exitCode: null,
      signalCode: null as NodeJS.Signals | null,
    };
    internals.child = mockProcess;
    expect(client.isAlive()).toBe(true);

    mockProcess.signalCode = "SIGTERM";
    expect(client.isAlive()).toBe(false);
    await expect(client.sendRequest("test/ping")).rejects.toThrow("ACP client is not connected");

    mockProcess.signalCode = "SIGKILL";
    expect(client.isAlive()).toBe(false);

    mockProcess.signalCode = null;
    expect(client.isAlive()).toBe(true);
  });

  it("includes --cwd when workspaceDir is provided in AcpClientOptions", async () => {
    const tempDir = mkdtempSync(join(tmpdir(), "fusion-acp-cwd-"));
    const fakeBin = join(tempDir, "fake-fusion");
    writeFileSync(fakeBin, "#!/bin/sh\nsleep 5\n", { mode: 0o755 });

    try {
      const workspaceDir = join(tempDir, "workspace");
      mkdirSync(workspaceDir, { recursive: true });

      const client = new AcpClient({
        binaryPath: fakeBin,
        workspaceDir,
      });

      const spawned = await client.spawn();
      expect(spawned).toBe(true);
      expect(client.childProcess).not.toBeNull();

      const spawnArgs = client.childProcess?.spawnargs;
      expect(spawnArgs).toContain("--cwd");
      const cwdIndex = spawnArgs.indexOf("--cwd");
      expect(spawnArgs[cwdIndex + 1]).toBe(workspaceDir);

      client.kill();
      expect(client.isAlive()).toBe(false);
    } finally {
      try {
        rmSync(tempDir, { recursive: true, force: true });
      } catch {
        // ignore
      }
    }
  });
});
