import { describe, test, expect } from "bun:test";
import React from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { SettingsView } from "../src/components/SettingsView";

describe("Cline Settings View Layout with Fusion API", () => {
  test("renders all 4 tabs: General, Fusion API & Models, Notifications, Account", () => {
    const html = renderToStaticMarkup(<SettingsView />);
    expect(html).toContain("Settings");
    expect(html).toContain("Manage desktop preferences for this browser and CLI environment.");
    expect(html).toContain('data-testid="settings-tab-general"');
    expect(html).toContain('data-testid="settings-tab-api"');
    expect(html).toContain('data-testid="settings-tab-notifications"');
    expect(html).toContain('data-testid="settings-tab-account"');
  });

  test("renders General controls: dark mode, font size, accent color swatches", () => {
    const html = renderToStaticMarkup(<SettingsView />);
    expect(html).toContain("Dark mode");
    expect(html).toContain("Font size");
    expect(html).toContain("Accent color");
    expect(html).toContain('data-testid="settings-darkmode-toggle"');
    expect(html).toContain('data-testid="settings-font-decrease"');
    expect(html).toContain('data-testid="settings-font-increase"');
    expect(html).toContain('data-testid="settings-font-display"');
  });

  test("renders back and close buttons when onClose is provided", () => {
    let closed = false;
    const html = renderToStaticMarkup(<SettingsView onClose={() => { closed = true; }} />);
    expect(html).toContain('data-testid="settings-back-btn"');
    expect(html).toContain('data-testid="settings-close-top-btn"');
  });
});
