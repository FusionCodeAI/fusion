import { describe, test, expect } from "bun:test";
import React from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { SessionCommandBar } from "../src/components/SessionCommandBar";

describe("Cline Session Command Bar (Cmd+K) Layout & Feature Parity", () => {
  test("renders closed when open is false", () => {
    const html = renderToStaticMarkup(
      <SessionCommandBar open={false} onOpenChange={() => {}} onOpenSession={() => {}} />
    );
    expect(html).toBe("");
  });

  test("renders search input, empty state, and footer keyboard badges when open", () => {
    const html = renderToStaticMarkup(
      <SessionCommandBar open={true} onOpenChange={() => {}} onOpenSession={() => {}} />
    );
    expect(html).toContain('data-testid="session-command-bar"');
    expect(html).toContain('data-testid="command-bar-input"');
    expect(html).toContain("Type a command or search sessions...");
    expect(html).toContain("Cmd+K");
    expect(html).toContain("New Session");
    expect(html).toContain("Open Project Folder...");
    expect(html).toContain("Toggle Sidebar");
  });

  test("renders matching session results and actions when open", () => {
    const sessions = [
      {
        id: "s-1",
        title: "Build authentication flow",
        createdAt: Date.now(),
        updatedAt: Date.now(),
        messages: [{ id: "m1", role: "user", content: "Can we use JWT tokens?", timestamp: Date.now() }],
      },
    ];

    const html = renderToStaticMarkup(
      <SessionCommandBar
        open={true}
        onOpenChange={() => {}}
        sessions={sessions}
        onOpenSession={() => {}}
      />
    );
    expect(html).toContain("Build authentication flow");
    expect(html).toContain("New Session");
  });
});
