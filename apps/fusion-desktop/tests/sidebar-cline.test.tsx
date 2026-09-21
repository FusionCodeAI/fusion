import { describe, test, expect } from "bun:test";
import React from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { Sidebar } from "../src/components/Sidebar";

describe("Sidebar Cline Layout", () => {
  test("renders traffic light clearance, FusionLogo, navigation controls, and Cline actions", () => {
    const html = renderToStaticMarkup(
      <Sidebar
        sessions={[{ id: "s1", title: "hi", createdAt: Date.now() - 3600 * 1000, updatedAt: Date.now() - 3600 * 1000 }]}
        activeSessionId="s1"
        onNewChat={() => {}}
        onSelectSession={() => {}}
      />
    );
    expect(html).toContain('data-testid="fusion-logo"');
    expect(html).toContain("Session");
    expect(html).toContain("Schedule");
    expect(html).toContain("Customize");
    expect(html).toContain("Sessions");
    expect(html).toContain("Settings");
    expect(html).toContain("hi");
    expect(html).toContain("1h");
  });
});
