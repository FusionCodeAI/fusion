import { describe, test, expect } from "bun:test";
import React from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { SettingsView } from "../src/components/SettingsView";

describe("Cline Settings View Layout with Fusion API", () => {
  test("has NO horizontal tabs (driven solely by vertical sidebar navigation)", () => {
    const html = renderToStaticMarkup(<SettingsView activeSection="general" />);
    expect(html).toContain("Settings");
    expect(html).not.toContain('data-testid="settings-tab-general"');
    expect(html).not.toContain('data-testid="settings-tab-api"');
    expect(html).not.toContain('data-testid="settings-tab-account"');
  });

  test("renders General controls: dark mode, font size, accent color swatches, notifications", () => {
    const html = renderToStaticMarkup(<SettingsView activeSection="general" />);
    expect(html).toContain("Dark mode");
    expect(html).toContain("Font size");
    expect(html).toContain("Accent color");
    expect(html).toContain("Desktop Notifications");
    expect(html).toContain('data-testid="settings-darkmode-toggle"');
    expect(html).toContain('data-testid="settings-font-decrease"');
    expect(html).toContain('data-testid="settings-font-increase"');
    expect(html).toContain('data-testid="settings-font-display"');
  });

  test("renders API Providers without raw API key input or base URL box", () => {
    const html = renderToStaticMarkup(<SettingsView activeSection="api" />);
    expect(html).toContain("API Providers");
    expect(html).toContain("Available Models");
    expect(html).toContain("DeepSeek 4 0731 Flash");
    // Raw API key input and base URL must be removed
    expect(html).not.toContain("Fusion API Key");
    expect(html).not.toContain("https://api.fusioncode.app/v1");
  });

  test("renders Account with real user Aung Myat Moe and NO wholesale benefits", () => {
    const html = renderToStaticMarkup(<SettingsView activeSection="account" onSignOut={() => {}} />);
    expect(html).toContain("Account");
    expect(html).toContain("Aung Myat Moe");
    expect(html).toContain("aungmyatmoe834@gmail.com");
    expect(html).toContain('data-testid="settings-signout-btn"');
    // Wholesale benefits must be removed
    expect(html).not.toContain("Wholesale API Rate Benefits");
  });
});
