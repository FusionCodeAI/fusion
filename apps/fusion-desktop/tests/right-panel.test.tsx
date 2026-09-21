import { describe, test, expect } from "bun:test";
import React from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { RightPanel } from "../src/components/RightPanel";

describe("RightPanel (Secondary Panel matching Cline)", () => {
  test("renders closed when open is false", () => {
    const html = renderToStaticMarkup(
      <RightPanel open={false} onClose={() => {}} />
    );
    expect(html).toBe("");
  });

  test("renders header tabs: Changes, Files, Terminal, and resize handle when open", () => {
    const html = renderToStaticMarkup(
      <RightPanel open={true} onClose={() => {}} />
    );

    expect(html).toContain('data-testid="right-panel"');
    expect(html).toContain('data-testid="right-panel-resize-handle"');
    expect(html).toContain('data-testid="right-panel-tab-changes"');
    expect(html).toContain('data-testid="right-panel-tab-files"');
    expect(html).toContain('data-testid="right-panel-tab-terminal"');
    expect(html).toContain("Changes");
    expect(html).toContain("Files");
    expect(html).toContain("Terminal");
  });

  test("renders unified diff in changes tab when patch is present", () => {
    const patch = `diff --git a/src/auth.ts b/src/auth.ts
--- a/src/auth.ts
+++ b/src/auth.ts
@@ -1,3 +1,4 @@
+export const JWT_SECRET = "secret";
`;

    const html = renderToStaticMarkup(
      <RightPanel open={true} activeTab="changes" diffPatch={patch} onClose={() => {}} />
    );

    expect(html).toContain('data-testid="right-panel-changes"');
    expect(html).toContain("JWT_SECRET");
  });

  test("renders file tree in files tab", () => {
    const mockFiles = [
      { path: "src/App.tsx", name: "App.tsx", is_dir: false },
      { path: "src/components", name: "components", is_dir: true },
    ];

    const html = renderToStaticMarkup(
      <RightPanel
        open={true}
        activeTab="files"
        workspaceEntries={mockFiles}
        onClose={() => {}}
      />
    );

    expect(html).toContain('data-testid="right-panel-files"');
    expect(html).toContain('data-testid="right-panel-file-search"');
    expect(html).toContain("src/App.tsx");
  });

  test("renders terminal console in terminal tab", () => {
    const html = renderToStaticMarkup(
      <RightPanel open={true} activeTab="terminal" onClose={() => {}} />
    );

    expect(html).toContain('data-testid="right-panel-terminal"');
    expect(html).toContain('data-testid="open-native-terminal"');
    expect(html).toContain("Native macOS Terminal");
  });
});
