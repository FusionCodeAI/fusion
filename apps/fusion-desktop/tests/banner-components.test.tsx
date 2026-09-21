import { describe, test, expect } from "bun:test";
import React from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { WorkspacePill } from "../src/components/WorkspacePill";
import { ConnectModelBanner } from "../src/components/ConnectModelBanner";

describe("Workspace Pill & Sign In Banner", () => {
  test("renders WorkspacePill with desktop and folder icons", () => {
    const html = renderToStaticMarkup(<WorkspacePill name="workspace" />);
    expect(html).toContain("workspace");
    expect(html).toContain('data-testid="workspace-pill"');
  });

  test("renders Sign In banner when user is not signed in and has NO model settings button", () => {
    const html = renderToStaticMarkup(
      <ConnectModelBanner isSignedIn={false} onSignIn={() => {}} />
    );
    expect(html).toContain("Sign in to start building");
    expect(html).toContain("Sign in");
    expect(html).toContain('data-testid="banner-signin-btn"');
    // Model settings button must be removed per user request
    expect(html).not.toContain("Model settings");
    expect(html).not.toContain('data-testid="banner-settings-btn"');
  });

  test("hides banner completely when user is already signed in", () => {
    const html = renderToStaticMarkup(
      <ConnectModelBanner isSignedIn={true} onSignIn={() => {}} />
    );
    expect(html).toBe("");
  });
});
