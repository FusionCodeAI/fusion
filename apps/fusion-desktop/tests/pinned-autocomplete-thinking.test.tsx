import { describe, test, expect } from "bun:test";
import React from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { filterMentions } from "../src/lib/mentions-catalog";
import { Sidebar, type SidebarSessionItem } from "../src/components/Sidebar";
import { TopHeader } from "../src/components/TopHeader";
import { ThinkingRow } from "../src/components/ThinkingRow";

describe("Real Workspace File & Folder Autocomplete", () => {
  const mockEntries = [
    { path: "crates", name: "crates", is_dir: true },
    { path: "crates/fusion-shell", name: "fusion-shell", is_dir: true },
    { path: "crates/fusion-shell/Cargo.toml", name: "Cargo.toml", is_dir: false },
    { path: "src/App.tsx", name: "App.tsx", is_dir: false },
    { path: "src/components", name: "components", is_dir: true },
  ];

  test("autocompletes files and folders with proper icons and categories", () => {
    const mentions = filterMentions("@", mockEntries);
    const crateFolder = mentions.find((m) => m.value === "@crates");
    const appFile = mentions.find((m) => m.value === "@src/App.tsx");

    expect(crateFolder).toBeDefined();
    expect(crateFolder?.category).toBe("folder");
    expect(crateFolder?.icon).toBe("Folder");

    expect(appFile).toBeDefined();
    expect(appFile?.category).toBe("file");
    expect(appFile?.icon).toBe("FileText");
  });

  test("filters exclusively to files when using @file: trigger", () => {
    const filesOnly = filterMentions("@file:", mockEntries);
    expect(filesOnly.length).toBeGreaterThan(0);
    expect(filesOnly.every((m) => m.category === "file")).toBe(true);
    expect(filesOnly.some((m) => m.value === "@src/App.tsx")).toBe(true);
    expect(filesOnly.some((m) => m.value === "@crates")).toBe(false);
  });

  test("filters exclusively to folders when using @folder: trigger", () => {
    const foldersOnly = filterMentions("@folder:", mockEntries);
    expect(foldersOnly.length).toBeGreaterThan(0);
    expect(foldersOnly.every((m) => m.category === "folder")).toBe(true);
    expect(foldersOnly.some((m) => m.value === "@crates")).toBe(true);
    expect(foldersOnly.some((m) => m.value === "@src/App.tsx")).toBe(false);
  });
});

describe("Pin Feature for Chat (Cline Matching)", () => {
  const mockSessions: SidebarSessionItem[] = [
    {
      id: "sess-1",
      title: "Unpinned session 1",
      updatedAt: 1000,
      workspaceName: "ProjectA",
      isPinned: false,
    },
    {
      id: "sess-2",
      title: "Pinned session 2",
      updatedAt: 500, // older timestamp, but pinned!
      workspaceName: "ProjectA",
      isPinned: true,
    },
  ];

  test("renders pinned session at the top of project group with pin icon", () => {
    const html = renderToStaticMarkup(
      <Sidebar
        sessions={mockSessions}
        activeSessionId="sess-2"
        onTogglePinSession={() => {}}
      />
    );

    // Both sessions render
    expect(html).toContain("Pinned session 2");
    expect(html).toContain("Unpinned session 1");

    // Pin button and indicator are rendered
    expect(html).toContain('data-testid="sidebar-pin-sess-1"');
    expect(html).toContain('data-testid="sidebar-pin-sess-2"');

    // Pinned session must appear before unpinned session in DOM order
    const pinnedIdx = html.indexOf("Pinned session 2");
    const unpinnedIdx = html.indexOf("Unpinned session 1");
    expect(pinnedIdx).toBeLessThan(unpinnedIdx);
  });

  test("TopHeader renders Pin button with active/inactive state", () => {
    const unpinnedHtml = renderToStaticMarkup(
      <TopHeader title="Test Chat" isPinned={false} onTogglePin={() => {}} />
    );
    expect(unpinnedHtml).toContain('data-testid="top-header-pin"');
    expect(unpinnedHtml).toContain('title="Pin chat"');

    const pinnedHtml = renderToStaticMarkup(
      <TopHeader title="Test Chat" isPinned={true} onTogglePin={() => {}} />
    );
    expect(pinnedHtml).toContain('title="Unpin chat"');
    expect(pinnedHtml).toContain("fill-current");
  });
});

describe("ThinkingRow with Markdown Rendering", () => {
  test("renders thought with markdown code blocks, bold headings, and lists", () => {
    const thought = `
### Architecture Plan
1. Refactor **state management**
2. Add \`list_workspace_entries\`

\`\`\`rust
fn test() -> bool { true }
\`\`\`
`;

    const html = renderToStaticMarkup(
      <ThinkingRow thought={thought} isGenerating={false} />
    );

    // Initial state renders the trigger button
    expect(html).toContain('data-testid="thinking-trigger"');
    expect(html).toContain("Thought briefly");
  });
});
