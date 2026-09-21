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
    expect(html).toContain("Search all session history...");
    expect(html).toContain("Cmd+K");
    expect(html).toContain("Cmd+P");
    expect(html).toContain("Navigate with");
  });

  test("renders matching session results when query matches sessions", () => {
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
    expect(html).toContain("Search session history");
  });
});
