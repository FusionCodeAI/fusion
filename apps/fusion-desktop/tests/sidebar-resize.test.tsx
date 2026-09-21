import { describe, test, expect } from "bun:test";
import React from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { Sidebar } from "../src/components/Sidebar";

describe("Sidebar Expandable & Resizable Handle", () => {
  test("renders resize handle on right edge with title and testid", () => {
    const html = renderToStaticMarkup(<Sidebar width={320} />);
    expect(html).toContain('data-testid="sidebar-resize-handle"');
    expect(html).toContain("cursor-col-resize");
    expect(html).toContain('style="width:320px"');
  });

  test("uses default w-64 when custom width is not provided", () => {
    const html = renderToStaticMarkup(<Sidebar />);
    expect(html).toContain("w-64");
  });
});
