import { describe, expect, it } from "bun:test";
import React from "react";
import { renderToString } from "react-dom/server";
import { ThinkingRow } from "../src/components/ThinkingRow";
import { UserMessage } from "../src/components/UserMessage";
import { ToolCallRow } from "../src/components/ToolCallRow";
import { DiffView } from "../src/components/DiffView";
import type { TurnStep } from "../src/types";

describe("Chat UI Components", () => {
  describe("ThinkingRow", () => {
    it("returns empty/null when thought is empty and not generating", () => {
      const html = renderToString(<ThinkingRow thought="" isGenerating={false} />);
      expect(html).toBe("");
    });

    it("renders Thinking... when isGenerating is true and no thought yet", () => {
      const html = renderToString(<ThinkingRow thought="" isGenerating={true} />);
      expect(html).toContain("Thinking...");
      expect(html).not.toContain("Thought briefly");
    });

    it("renders Thought briefly ▾ when thought is present (collapsed by default)", () => {
      const html = renderToString(
        <ThinkingRow thought="Analyzing the database schema..." isGenerating={false} />
      );
      expect(html).toContain("Thought briefly");
      expect(html).not.toContain("Analyzing the database schema...");
    });
  });

  describe("UserMessage", () => {
    it("renders Cline-style message container with correct classes and content", () => {
      const content = "Can you check the performance metrics?";
      const html = renderToString(<UserMessage content={content} />);

      expect(html).toContain("w-full");
      expect(html).toContain("bg-zinc-100/90");
      expect(html).toContain(content);
    });
  });

  describe("ToolCallRow", () => {
    it("renders compact 28px action pill with completed status", () => {
      const step: TurnStep = {
        id: "step-1",
        title: "Ran bun test (340ms)",
        status: "completed",
        details: "66 pass, 0 fail",
      };

      const html = renderToString(<ToolCallRow step={step} />);
      expect(html).toContain("Ran bun test (340ms)");
      expect(html).toContain("text-emerald-600");
      expect(html).toContain("rounded-lg");
      // Collapsed by default
      expect(html).not.toContain("66 pass, 0 fail");
    });

    it("renders running spinner icon for running step", () => {
      const step: TurnStep = {
        id: "step-2",
        title: "Read src/models.ts",
        status: "running",
      };

      const html = renderToString(<ToolCallRow step={step} />);
      expect(html).toContain("Read src/models.ts");
      expect(html).toContain("animate-spin");
    });
  });

  describe("DiffView", () => {
    it("returns empty string when patch is empty", () => {
      const html = renderToString(<DiffView patch="" />);
      expect(html).toBe("");
    });

    it("renders unified git diff with green additions, red deletions, and header styling", () => {
      const patch = `diff --git a/src/types.ts b/src/types.ts
--- a/src/types.ts
+++ b/src/types.ts
@@ -1,3 +1,3 @@
 export interface TurnStep {
-  timestamp: number;
+  timestamp?: number;
 }`;

      const html = renderToString(<DiffView patch={patch} />);
      expect(html).toContain("src/types.ts");
      expect(html).toContain("+1");
      expect(html).toContain("-1");
      expect(html).toContain("bg-emerald-50");
      expect(html).toContain("bg-rose-50");
      expect(html).toContain("timestamp?: number;");
      expect(html).toContain("timestamp: number;");
    });
  });
});
