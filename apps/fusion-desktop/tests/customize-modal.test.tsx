import { describe, test, expect } from "bun:test";
import React from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { CustomizeModal } from "../src/components/CustomizeModal";

describe("Cline Customize Modal Feature Parity", () => {
  test("renders closed when isOpen is false", () => {
    const html = renderToStaticMarkup(
      <CustomizeModal isOpen={false} onClose={() => {}} />
    );
    expect(html).toBe("");
  });

  test("renders all 4 tabs when open: Rules, Skills, Tools, MCP", () => {
    const html = renderToStaticMarkup(
      <CustomizeModal isOpen={true} onClose={() => {}} />
    );
    expect(html).toContain("Customize Fusion");
    expect(html).toContain('data-testid="customize-tab-rules"');
    expect(html).toContain('data-testid="customize-tab-skills"');
    expect(html).toContain('data-testid="customize-tab-tools"');
    expect(html).toContain('data-testid="customize-tab-mcp"');
  });

  test("renders rules textarea and save button by default", () => {
    const html = renderToStaticMarkup(
      <CustomizeModal isOpen={true} onClose={() => {}} />
    );
    expect(html).toContain('data-testid="customize-rules-textarea"');
    expect(html).toContain('data-testid="customize-rules-save"');
    expect(html).toContain("Done");
  });
});
