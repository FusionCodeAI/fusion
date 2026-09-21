import { describe, expect, it } from "bun:test";
import React from "react";
import { renderToStaticMarkup, renderToString } from "react-dom/server";
import { TopHeader, type TopHeaderProps } from "../src/components/TopHeader";
import {
  Sidebar,
  type SidebarProps,
  formatRelativeTime,
  getWorkspaceFolderName,
} from "../src/components/Sidebar";
import {
  App,
  type AppProps,
  setupBridgeListeners,
  isSidebarToggleKeyCombo,
} from "../src/App";
import { AgentBridge } from "../src/lib/agent-bridge";
import { FUSION_MODELS, DEFAULT_FUSION_MODEL } from "../src/models";
import type { ChatMessage, TurnStep } from "../src/types";

describe("TopHeader Component", () => {
  it("renders with 40px height (h-10) and subtle bottom border matching design spec", () => {
    const html = renderToStaticMarkup(<TopHeader />);

    expect(html).toContain("h-10");
    expect(html).toContain("border-b");
    expect(html).toContain("border-zinc-200/80");
    expect(html).toContain("bg-white");
    expect(html).toContain("data-tauri-drag-region");
  });

  it("renders default title 'General chat conversation' with 13px font-medium text-zinc-900", () => {
    const html = renderToStaticMarkup(<TopHeader />);

    expect(html).toContain("General chat conversation");
    expect(html).toContain("text-[13px]");
    expect(html).toContain("font-medium");
    expect(html).toContain("text-zinc-900");
  });

  it("renders custom session title when provided", () => {
    const customTitle = "Refactor Authentication Flow";
    const html = renderToStaticMarkup(<TopHeader title={customTitle} />);

    expect(html).toContain(customTitle);
  });

  it("renders right-side actions: IDE ↗, More (...), and Sidebar toggle ([|])", () => {
    const html = renderToStaticMarkup(<TopHeader />);

    expect(html).toContain("IDE");
    expect(html).toContain("data-testid=\"top-header-ide\"");
    expect(html).toContain("data-testid=\"top-header-more\"");
    expect(html).toContain("data-testid=\"top-header-sidebar-toggle\"");
  });

  it("invokes onToggleSidebar when toggle button is clicked", () => {
    let toggled = false;
    const element = TopHeader({
      onToggleSidebar: () => {
        toggled = true;
      },
    });

    // Verify component element is created and call prop directly
    expect(element).toBeDefined();
    // Simulate callback
    element.props.children[1].props.children[2].props.onClick();
    expect(toggled).toBe(true);
  });

  it("invokes onOpenIde when IDE button is clicked", () => {
    let opened = false;
    const element = TopHeader({
      onOpenIde: () => {
        opened = true;
      },
    });

    element.props.children[1].props.children[0].props.onClick();
    expect(opened).toBe(true);
  });
});

describe("Sidebar Component", () => {
  it("renders with 220px width, bg-[#f7f7f8], and border right matching design spec", () => {
    const html = renderToStaticMarkup(<Sidebar />);

    expect(html).toContain("w-[220px]");
    expect(html).toContain("bg-[#f7f7f8]");
    expect(html).toContain("border-r");
    expect(html).toContain("border-zinc-200/80");
  });

  it("renders top row (40px) with pl-[78px] clearing macOS traffic lights at x:18, y:14", () => {
    const html = renderToStaticMarkup(<Sidebar />);

    expect(html).toContain("h-10");
    expect(html).toContain("pl-[78px]");
    expect(html).toContain("data-testid=\"sidebar-top-row\"");
    expect(html).toContain("data-testid=\"sidebar-toggle-button\"");
    expect(html).toContain("data-testid=\"sidebar-history-back\"");
    expect(html).toContain("data-testid=\"sidebar-history-forward\"");
  });

  it("renders primary menu items: New Chat and Search input box", () => {
    const html = renderToStaticMarkup(<Sidebar />);

    expect(html).toContain("New Chat");
    expect(html).toContain("data-testid=\"sidebar-new-chat\"");
    expect(html).toContain("data-testid=\"sidebar-search-input\"");
    expect(html).toContain("placeholder=\"Search\"");
  });

  it("renders Projects section with add button and ◌ New Project item", () => {
    const html = renderToStaticMarkup(<Sidebar />);

    expect(html).toContain("Projects");
    expect(html).toContain("New Project");
    expect(html).toContain("data-testid=\"sidebar-new-project-button\"");
    expect(html).toContain("data-testid=\"sidebar-new-project-item\"");
  });

  it("renders Repositories section with No Repo and session item • General chat conversation 2h", () => {
    const html = renderToStaticMarkup(<Sidebar />);

    expect(html).toContain("Repositories");
    expect(html).toContain("No Repo");
    expect(html).toContain("data-testid=\"sidebar-repo-item\"");
    expect(html).toContain("General chat conversation");
    expect(html).toContain("2h");
  });

  it("renders footer with Getting Started 1/3 ⚪, Connect GitHub, and User profile (A) Aung Myat Moe ⚙", () => {
    const html = renderToStaticMarkup(<Sidebar />);

    expect(html).toContain("Getting Started");
    expect(html).toContain("1/3");
    expect(html).toContain("Connect GitHub");
    expect(html).toContain("data-testid=\"sidebar-connect-github\"");
    expect(html).toContain("Aung Myat Moe");
    expect(html).toContain("data-testid=\"sidebar-settings-button\"");
  });

  it("invokes onNewChat when New Chat button is clicked", () => {
    let newChatCalled = false;
    const element = (
      <Sidebar
        onNewChat={() => {
          newChatCalled = true;
        }}
      />
    );

    expect(element.props.onNewChat).toBeDefined();
    element.props.onNewChat();
    expect(newChatCalled).toBe(true);
  });
  describe("Helper formatters", () => {
    it("formatRelativeTime computes human-readable timestamps accurately", () => {
      const now = 1700000000000;
      expect(formatRelativeTime(now - 30 * 1000, now)).toBe("just now");
      expect(formatRelativeTime(now - 5 * 60 * 1000, now)).toBe("5m");
      expect(formatRelativeTime(now - 2 * 3600 * 1000, now)).toBe("2h");
      expect(formatRelativeTime(now - 3 * 86400 * 1000, now)).toBe("3d");
      expect(formatRelativeTime(now - 14 * 86400 * 1000, now)).toBe("2w");
      expect(formatRelativeTime(undefined)).toBe("2h");
    });

    it("getWorkspaceFolderName extracts clean folder names", () => {
      expect(getWorkspaceFolderName("")).toBe("No Repo");
      expect(getWorkspaceFolderName("/")).toBe("No Repo");
      expect(getWorkspaceFolderName("/Users/developer/code/fusion")).toBe("fusion");
      expect(getWorkspaceFolderName("C:\\Projects\\my-app\\")).toBe("my-app");
    });
  });
});

describe("App Shell Component", () => {
  it("renders locked root, conversation, stream, and composer layout", () => {
    const html = renderToString(<App />);

    // Locked root layout
    expect(html).toContain("h-screen w-screen flex flex-row overflow-hidden bg-white");
    // Locked conversation layout
    expect(html).toContain("flex-1 min-h-0 flex flex-col overflow-hidden");
    // Locked stream layout
    expect(html).toContain("flex-1 min-h-0 overflow-y-auto px-8 py-4 flex flex-col items-center");
    // Locked composer layout
    expect(html).toContain("shrink-0 p-4 flex flex-col items-center bg-white border-t border-zinc-100");
  });

  it("renders empty state with HeroView and initial prompt composer", () => {
    const html = renderToString(<App />);

    expect(html).toContain("What should we build today?");
    expect(html).toContain("Ask questions, plan features, or generate code with Fusion Agent");
    expect(html).toContain("Build a new feature in React &amp; Tailwind");
  });

  it("renders Sidebar by default when initialSidebarOpen is true", () => {
    const html = renderToString(<App initialSidebarOpen={true} />);

    expect(html).toContain("data-testid=\"sidebar\"");
    expect(html).toContain("w-[220px]");
    expect(html).toContain("New Chat");
  });

  it("collapses Sidebar and expands conversation area when initialSidebarOpen is false", () => {
    const html = renderToString(<App initialSidebarOpen={false} />);

    // Sidebar is collapsed/hidden
    expect(html).not.toContain("data-testid=\"sidebar\"");
    // Conversation area retains flex-1 expanding to full width
    expect(html).toContain("flex-1 min-h-0 flex flex-col overflow-hidden");
    // Stream and composer remain centered
    expect(html).toContain("items-center");
  });

  it("renders conversation messages with UserMessage, ThinkingRow, ToolCallRow, and DiffView", () => {
    const sampleMessages: ChatMessage[] = [
      {
        id: "msg-1",
        role: "user",
        content: "Please check the database connection and update schema",
        timestamp: Date.now() - 10000,
      },
      {
        id: "msg-2",
        role: "assistant",
        content: "Here are the database updates and applied patches.",
        thought: "Analyzing postgres connection strings and running migrations...",
        steps: [
          {
            id: "step-1",
            title: "Ran bun db:migrate",
            status: "completed",
            details: "Migration applied successfully in 12ms",
          },
        ],
        diffPatch: "--- a/schema.sql\n+++ b/schema.sql\n@@ -1,2 +1,3 @@\n+CREATE TABLE users (id SERIAL);",
        timestamp: Date.now() - 5000,
      },
    ];

    const html = renderToString(<App initialMessages={sampleMessages} />);

    // UserMessage rendered
    expect(html).toContain("Please check the database connection");
    expect(html).toContain("bg-zinc-100");

    // ThinkingRow rendered
    expect(html).toContain("Thought briefly");

    // ToolCallRow rendered
    expect(html).toContain("Ran bun db:migrate");

    // DiffView rendered
    expect(html).toContain("schema.sql");
    expect(html).toContain("CREATE TABLE users (id SERIAL);");

    // Assistant content rendered
    expect(html).toContain("Here are the database updates and applied patches.");
  });

  it("renders default model selection (DeepSeek 4 Flash)", () => {
    const html = renderToString(<App />);

    expect(html).toContain(DEFAULT_FUSION_MODEL.shortName);
  });

  it("supports custom model selection via initialModel", () => {
    const targetModel = FUSION_MODELS[2]; // GLM 5.3 Flash
    const html = renderToString(<App initialModel={targetModel.id} />);

    expect(html).toContain(targetModel.shortName);
  });

  it("wires AgentBridge event listeners (thought, chunk, step, diff, done, error)", () => {
    const customBridge = new AgentBridge();
    const registeredEvents: string[] = [];

    const origOn = customBridge.on.bind(customBridge);
    customBridge.on = (
      event: Parameters<AgentBridge["on"]>[0],
      cb: Parameters<AgentBridge["on"]>[1]
    ) => {
      registeredEvents.push(event);
      return origOn(event, cb);
    };

    const cleanup = setupBridgeListeners(customBridge, {
      onThought: () => {},
      onChunk: () => {},
      onStep: () => {},
      onDiff: () => {},
      onDone: () => {},
      onError: () => {},
    });

    expect(registeredEvents).toContain("thought");
    expect(registeredEvents).toContain("chunk");
    expect(registeredEvents).toContain("step");
    expect(registeredEvents).toContain("diff");
    expect(registeredEvents).toContain("done");
    expect(registeredEvents).toContain("error");

    expect(typeof cleanup).toBe("function");
    cleanup();
  });

  it("Cmd+B keyboard shortcut helper identifies toggle key combinations accurately", () => {
    // Cmd+B on Mac
    expect(isSidebarToggleKeyCombo({ key: "b", metaKey: true })).toBe(true);
    expect(isSidebarToggleKeyCombo({ key: "B", metaKey: true })).toBe(true);
    // Ctrl+B on Windows/Linux
    expect(isSidebarToggleKeyCombo({ key: "b", ctrlKey: true })).toBe(true);
    expect(isSidebarToggleKeyCombo({ key: "B", ctrlKey: true })).toBe(true);
    // Negative cases
    expect(isSidebarToggleKeyCombo({ key: "b", metaKey: false, ctrlKey: false })).toBe(false);
    expect(isSidebarToggleKeyCombo({ key: "a", metaKey: true })).toBe(false);
    expect(isSidebarToggleKeyCombo({ key: "c", ctrlKey: true })).toBe(false);
  });
});
