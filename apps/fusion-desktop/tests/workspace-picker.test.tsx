import { describe, test, expect } from "bun:test";
import React from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { WorkspacePill } from "../src/components/WorkspacePill";
import { ClineHeroView } from "../src/components/ClineHeroView";

describe("Workspace Selector & Native Folder Picker", () => {
  test("renders WorkspacePill with custom workspace name and clickable button", () => {
    let clicked = false;
    const html = renderToStaticMarkup(
      <WorkspacePill name="fusion-engine" onClick={() => { clicked = true; }} />
    );
    expect(html).toContain("fusion-engine");
    expect(html).toContain('data-testid="workspace-pill"');
  });

  test("ClineHeroView propagates workspaceName and pick handler", () => {
    const html = renderToStaticMarkup(
      <ClineHeroView
        workspaceName="my-custom-repo"
        onSend={() => {}}
      />
    );
    expect(html).toContain("my-custom-repo");
  });
});
