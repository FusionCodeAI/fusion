import { describe, test, expect } from "bun:test";
import React from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { Composer } from "../src/components/Composer";

describe("Composer Cline Layout", () => {
  test("renders Cline-style placeholder, paperclip, billing profile, model picker, and effort selector", () => {
    const html = renderToStaticMarkup(
      <Composer onSend={() => {}} selectedModel="deepseek-4-flash" />
    );
    expect(html).toContain("Ask to make changes, @mention files, reference #PRs, or run /commands.");
    expect(html).toContain("Fusion Usage-Billing");
    expect(html).toContain("DeepSeek 4");
    expect(html).toContain("Low");
    expect(html).toContain('data-testid="composer-paperclip"');
    expect(html).toContain('data-testid="composer-billing-profile"');
    expect(html).toContain('data-testid="composer-effort"');
  });
});
