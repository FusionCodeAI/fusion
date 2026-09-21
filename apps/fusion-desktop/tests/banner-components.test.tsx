import { describe, test, expect } from "bun:test";
import React from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { WorkspacePill } from "../src/components/WorkspacePill";
import { ConnectModelBanner } from "../src/components/ConnectModelBanner";

describe("Workspace Pill & Connect Model Banner", () => {
  test("renders WorkspacePill with desktop and folder icons", () => {
    const html = renderToStaticMarkup(<WorkspacePill name="workspace" />);
    expect(html).toContain("workspace");
    expect(html).toContain('data-testid="workspace-pill"');
  });

  test("renders ConnectModelBanner with Fusion messaging and violet button", () => {
    const html = renderToStaticMarkup(
      <ConnectModelBanner onConnect={() => {}} onSettings={() => {}} />
    );
    expect(html).toContain("Connect a model to start building");
    expect(html).toContain("Connect a model");
    expect(html).toContain("Model settings");
    expect(html).toContain('data-testid="connect-model-banner"');
  });
});
