import { describe, test, expect } from "bun:test";
import React from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { CustomizeView } from "../src/components/CustomizeView";

describe("Cline Customize View Layout & Feature Parity", () => {
  test("renders all 6 Cline CUSTOMIZE_TABS: Tools, Plugins, Skills, Rules, MCP, Hooks", () => {
    const html = renderToStaticMarkup(<CustomizeView />);
    expect(html).toContain("Customize");
    expect(html).toContain("Extend what Cline can do and how it works");
    expect(html).toContain('data-testid="customize-tab-tools"');
    expect(html).toContain('data-testid="customize-tab-plugins"');
    expect(html).toContain('data-testid="customize-tab-skills"');
    expect(html).toContain('data-testid="customize-tab-rules"');
    expect(html).toContain('data-testid="customize-tab-mcp"');
    expect(html).toContain('data-testid="customize-tab-hooks"');
  });

  test("renders section commands and filter search", () => {
    const html = renderToStaticMarkup(<CustomizeView />);
    expect(html).toContain("fusion config tools");
    expect(html).toContain("Filter tools...");
  });

  test("renders close button and back button when onClose is provided", () => {
    let closed = false;
    const html = renderToStaticMarkup(
      <CustomizeView onClose={() => { closed = true; }} />
    );
    expect(html).toContain('data-testid="customize-back-btn"');
    expect(html).toContain('data-testid="customize-close-top-btn"');
  });
});
