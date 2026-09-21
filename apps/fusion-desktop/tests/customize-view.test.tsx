import { describe, test, expect } from "bun:test";
import React from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { CustomizeView } from "../src/components/CustomizeView";

describe("Cline Customize View Layout & Feature Parity", () => {
  test("renders all 6 Cline CUSTOMIZE_TABS: Tools 10, Plugins 0, Skills 17, Rules 0, MCP 0, Hooks 0", () => {
    const html = renderToStaticMarkup(<CustomizeView />);
    expect(html).toContain("Customize");
    expect(html).toContain("Extend what Cline can do and how it works");
    expect(html).toContain('data-testid="customize-tab-tools"');
    expect(html).toContain('data-testid="customize-tab-plugins"');
    expect(html).toContain('data-testid="customize-tab-skills"');
    expect(html).toContain('data-testid="customize-tab-rules"');
    expect(html).toContain('data-testid="customize-tab-mcp"');
    expect(html).toContain('data-testid="customize-tab-hooks"');
    expect(html).toContain("10"); // Tools 10
    expect(html).toContain("17"); // Skills 17
  });

  test("renders Marketplace action button and NO inline back button in header", () => {
    const html = renderToStaticMarkup(<CustomizeView />);
    expect(html).toContain('data-testid="customize-marketplace-btn"');
    expect(html).toContain("Marketplace");
    // Back button belongs to the sidebar header row, not inside page header
    expect(html).not.toContain('data-testid="customize-back-btn"');
  });

  test("renders search box, Disable all checkbox, and Image #1 tools", () => {
    const html = renderToStaticMarkup(<CustomizeView />);
    expect(html).toContain("Search tools");
    expect(html).toContain("BuiltIn Tools");
    expect(html).toContain("Disable all");
    expect(html).toContain("ask_question");
    expect(html).toContain("editor");
    expect(html).toContain("fetch_web_content");
    expect(html).toContain("read_files");
    expect(html).toContain("run_commands");
    expect(html).toContain("search_codebase");
  });
});
