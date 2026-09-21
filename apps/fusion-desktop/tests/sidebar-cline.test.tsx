import { describe, test, expect } from "bun:test";
import React from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { Sidebar } from "../src/components/Sidebar";

describe("Sidebar Cline Layout matching Image #1 and #2", () => {
  test("renders 2-row header: traffic light row with navigation arrows, logo row with search icon", () => {
    const html = renderToStaticMarkup(
      <Sidebar
        sessions={[{ id: "s1", title: "hi", createdAt: Date.now() - 3600 * 1000, updatedAt: Date.now() - 3600 * 1000 }]}
        activeSessionId="s1"
        onNewChat={() => {}}
        onSelectSession={() => {}}
      />
    );
    expect(html).toContain('data-testid="sidebar-nav-back"');
    expect(html).toContain('data-testid="sidebar-nav-forward"');
    expect(html).toContain('data-testid="cline-avatar"');
    expect(html).toContain('data-testid="sidebar-search-btn"');
  });

  test("renders main page sidebar without settings items (Image #1)", () => {
    const html = renderToStaticMarkup(
      <Sidebar
        currentView="chat"
        onNewChat={() => {}}
        onCustomize={() => {}}
      />
    );
    expect(html).toContain("Session");
    expect(html).toContain("Schedule");
    expect(html).toContain("Customize");
    expect(html).toContain("Settings");
    // Settings group is NOT shown in main chat view
    expect(html).not.toContain('data-testid="sidebar-settings-general"');
    expect(html).not.toContain('data-testid="sidebar-settings-api"');
  });

  test("renders settings items in sidebar ONLY when in settings view (Image #2)", () => {
    const html = renderToStaticMarkup(
      <Sidebar
        currentView="settings"
        settingsSection="general"
        onNewChat={() => {}}
        onCustomize={() => {}}
      />
    );
    expect(html).toContain('data-testid="sidebar-settings-general"');
    expect(html).toContain('data-testid="sidebar-settings-api"');
    expect(html).toContain("General");
    expect(html).toContain("API Providers");
  });

  test("groups sessions by project with collapsible sections and sort/filter controls", () => {
    const sessions = [
      { id: "s1", title: "fix bug", createdAt: Date.now(), updatedAt: Date.now(), workspaceName: "fusion" },
      { id: "s2", title: "add cart", createdAt: Date.now(), updatedAt: Date.now(), workspaceName: "ecommerce-shop" },
    ];

    const html = renderToStaticMarkup(
      <Sidebar
        sessions={sessions}
        workspaceDir="/Users/aungmyatmoe/TheSpace/fusion"
        onNewChat={() => {}}
      />
    );

    // Project labels
    expect(html).toContain("fusion");
    expect(html).toContain("ecommerce-shop");
    expect(html).toContain('data-testid="sidebar-sort-toggle"');
    expect(html).toContain('data-testid="sidebar-filter-toggle"');
  });
});
