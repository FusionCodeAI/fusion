import { describe, test, expect } from "bun:test";
import React from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { App } from "../src/App";

describe("App Integration with Cline Layout", () => {
  test("renders Sign In banner when user is not signed in", () => {
    const html = renderToStaticMarkup(<App initialMessages={[]} initialIsSignedIn={false} />);
    expect(html).toContain('data-testid="cline-watermark"');
    expect(html).toContain('data-testid="workspace-pill"');
    expect(html).toContain('data-testid="connect-model-banner"');
    expect(html).toContain("Sign in to start building");
    expect(html).toContain("Sign in");
    expect(html).not.toContain("Model settings");
  });

  test("hides banner completely when user is already signed in", () => {
    const html = renderToStaticMarkup(<App initialMessages={[]} initialIsSignedIn={true} />);
    expect(html).not.toContain('data-testid="connect-model-banner"');
    expect(html).not.toContain("Sign in to start building");
    expect(html).toContain('data-testid="workspace-pill"');
  });
});
