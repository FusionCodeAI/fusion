import { describe, test, expect } from "bun:test";
import React from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { App } from "../src/App";

describe("App Integration with Cline Layout", () => {
  test("renders FusionWatermark, WorkspacePill, ConnectModelBanner, and Composer in initial state", () => {
    const html = renderToStaticMarkup(<App initialMessages={[]} />);
    expect(html).toContain('data-testid="fusion-watermark"');
    expect(html).toContain('data-testid="workspace-pill"');
    expect(html).toContain('data-testid="connect-model-banner"');
    expect(html).toContain("Fusion Usage-Billing");
    expect(html).toContain("Connect a model to start building");
  });
});
