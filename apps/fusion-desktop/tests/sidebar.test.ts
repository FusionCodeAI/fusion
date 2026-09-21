import { describe, expect, it, mock, afterEach } from "bun:test";
import React from "react";
import { createTestRoot, type TestRoot, type TestRenderer, type TestElement } from "@gpuix/react/testing";
import {
  Sidebar,
  formatRelativeTime,
  getWorkspaceFolderName,
  truncatePath,
} from "../src/ui/sidebar";
import type { ChatSession } from "../src/types";

function createMockSession(overrides: Partial<ChatSession> = {}): ChatSession {
  const now = Date.now();
  return {
    id: overrides.id || `session-${Math.random().toString(36).slice(2, 8)}`,
    title: overrides.title ?? "Test Session",
    createdAt: overrides.createdAt ?? now,
    updatedAt: overrides.updatedAt ?? now,
    workspaceDir: overrides.workspaceDir ?? "/test/workspace",
    model: overrides.model ?? "fusion-agent",
    messages: overrides.messages ?? [],
  };
}

function getElementText(renderer: TestRenderer, el: TestElement | undefined): string | null {
  if (!el) return null;
  if (el.text !== null && el.text !== undefined) return el.text;
  for (const childId of el.children) {
    const child = renderer.getElement(childId);
    if (child?.text !== null && child?.text !== undefined) {
      return child.text;
    }
  }
  return null;
}

describe("formatRelativeTime utility", () => {
  const baseTime = 1700000000000;

  it("handles zero, negative, NaN or invalid timestamps", () => {
    expect(formatRelativeTime(0, baseTime)).toBe("just now");
    expect(formatRelativeTime(-1000, baseTime)).toBe("just now");
    expect(formatRelativeTime(NaN, baseTime)).toBe("just now");
  });

  it("handles future timestamps gracefully as 'just now'", () => {
    expect(formatRelativeTime(baseTime + 10000, baseTime)).toBe("just now");
  });

  it("returns 'just now' for diffs under 1 minute", () => {
    expect(formatRelativeTime(baseTime, baseTime)).toBe("just now");
    expect(formatRelativeTime(baseTime - 30 * 1000, baseTime)).toBe("just now");
    expect(formatRelativeTime(baseTime - 59 * 1000, baseTime)).toBe("just now");
  });

  it("returns minutes 'Xm' for diffs under 1 hour", () => {
    expect(formatRelativeTime(baseTime - 60 * 1000, baseTime)).toBe("1m");
    expect(formatRelativeTime(baseTime - 5 * 60 * 1000, baseTime)).toBe("5m");
    expect(formatRelativeTime(baseTime - 59 * 60 * 1000, baseTime)).toBe("59m");
  });

  it("returns hours 'Xh' for diffs under 24 hours", () => {
    expect(formatRelativeTime(baseTime - 60 * 60 * 1000, baseTime)).toBe("1h");
    expect(formatRelativeTime(baseTime - 2 * 60 * 60 * 1000, baseTime)).toBe("2h");
    expect(formatRelativeTime(baseTime - 23 * 60 * 60 * 1000, baseTime)).toBe("23h");
  });

  it("returns 'yesterday' for diffs between 24 and 48 hours", () => {
    expect(formatRelativeTime(baseTime - 24 * 60 * 60 * 1000, baseTime)).toBe("yesterday");
    expect(formatRelativeTime(baseTime - 36 * 60 * 60 * 1000, baseTime)).toBe("yesterday");
    expect(formatRelativeTime(baseTime - 47 * 60 * 60 * 1000, baseTime)).toBe("yesterday");
  });

  it("returns days 'Xd' for diffs between 2 and 7 days", () => {
    expect(formatRelativeTime(baseTime - 2 * 24 * 60 * 60 * 1000, baseTime)).toBe("2d");
    expect(formatRelativeTime(baseTime - 3 * 24 * 60 * 60 * 1000, baseTime)).toBe("3d");
    expect(formatRelativeTime(baseTime - 6 * 24 * 60 * 60 * 1000, baseTime)).toBe("6d");
  });

  it("returns weeks 'Xw' for diffs over 7 days up to 30 days", () => {
    expect(formatRelativeTime(baseTime - 7 * 24 * 60 * 60 * 1000, baseTime)).toBe("1w");
    expect(formatRelativeTime(baseTime - 14 * 24 * 60 * 60 * 1000, baseTime)).toBe("2w");
  });

  it("returns months or years for older diffs", () => {
    expect(formatRelativeTime(baseTime - 60 * 24 * 60 * 60 * 1000, baseTime)).toBe("2mo");
    expect(formatRelativeTime(baseTime - 400 * 24 * 60 * 60 * 1000, baseTime)).toBe("1y");
  });
});

describe("workspace path helpers", () => {
  it("extracts folder name accurately", () => {
    expect(getWorkspaceFolderName("/Users/developer/projects/fusion")).toBe("fusion");
    expect(getWorkspaceFolderName("/Users/developer/projects/fusion/")).toBe("fusion");
    expect(getWorkspaceFolderName("C:\\projects\\fusion-desktop")).toBe("fusion-desktop");
    expect(getWorkspaceFolderName("")).toBe("workspace");
    expect(getWorkspaceFolderName("/")).toBe("/");
  });

  it("truncates long paths while preserving prefix and base folder", () => {
    const shortPath = "/projects/fusion";
    expect(truncatePath(shortPath, 30)).toBe(shortPath);

    const longPath = "/Users/aungmyatmoe/TheSpace/fusion/apps/fusion-desktop";
    const truncated = truncatePath(longPath, 30);
    expect(truncated.length).toBeLessThanOrEqual(30);
    expect(truncated).toContain("fusion-desktop");
    expect(truncated).toContain("...");
  });
});

describe("Sidebar Component", () => {
  let testRoot: TestRoot;

  afterEach(() => {
    testRoot?.unmount();
  });

  it("renders sessions and detects the active session", () => {
    testRoot = createTestRoot();
    const now = Date.now();
    const session1 = createMockSession({
      id: "s1",
      title: "Architecture Review",
      updatedAt: now - 5 * 60 * 1000, // 5m
    });
    const session2 = createMockSession({
      id: "s2",
      title: "Bug Triage",
      updatedAt: now - 2 * 60 * 60 * 1000, // 2h
    });

    testRoot.render(
      React.createElement(Sidebar, {
        sessions: [session1, session2],
        activeSessionId: "s1",
        workspaceDir: "/home/user/code/fusion",
        onNewChat: () => {},
        onSelectSession: () => {},
        onDeleteSession: () => {},
      })
    );
    testRoot.renderer.flush();

    // Verify session 1 title and formatted time
    const title1 = testRoot.renderer.findByTestId("sidebar-session-title-s1");
    expect(title1).toBeDefined();
    expect(getElementText(testRoot.renderer, title1)).toBe("Architecture Review");

    const time1 = testRoot.renderer.findByTestId("sidebar-session-time-s1");
    expect(getElementText(testRoot.renderer, time1)).toBe("5m");

    // Verify session 2 title and formatted time
    const title2 = testRoot.renderer.findByTestId("sidebar-session-title-s2");
    expect(title2).toBeDefined();
    expect(getElementText(testRoot.renderer, title2)).toBe("Bug Triage");

    const time2 = testRoot.renderer.findByTestId("sidebar-session-time-s2");
    expect(getElementText(testRoot.renderer, time2)).toBe("2h");

    // Verify active session highlighting
    const sessionEl1 = testRoot.renderer.findByTestId("sidebar-session-s1");
    const sessionEl2 = testRoot.renderer.findByTestId("sidebar-session-s2");

    expect(sessionEl1?.customProps?.["aria-selected"]).toBe(true);
    expect(sessionEl1?.style?.backgroundColor).toBe("#1e293b");

    expect(sessionEl2?.customProps?.["aria-selected"]).toBe(false);
    expect(sessionEl2?.style?.backgroundColor).toBe("transparent");
  });

  it("renders empty state when there are no sessions", () => {
    testRoot = createTestRoot();
    testRoot.render(
      React.createElement(Sidebar, {
        sessions: [],
        activeSessionId: null,
        workspaceDir: "/projects/empty",
        onNewChat: () => {},
        onSelectSession: () => {},
        onDeleteSession: () => {},
      })
    );
    testRoot.renderer.flush();

    const emptyState = testRoot.renderer.findByTestId("sidebar-empty-state");
    expect(emptyState).toBeDefined();

    const emptyTitle = testRoot.renderer.findByTestId("sidebar-empty-title");
    expect(getElementText(testRoot.renderer, emptyTitle)).toBe("No chats yet");

    const emptySubtitle = testRoot.renderer.findByTestId("sidebar-empty-subtitle");
    expect(getElementText(testRoot.renderer, emptySubtitle)).toBe("Click + New Chat to get started");
  });

  it("filters sessions in real-time based on search input", () => {
    testRoot = createTestRoot();
    const session1 = createMockSession({ id: "s1", title: "React Component Refactor" });
    const session2 = createMockSession({ id: "s2", title: "API Gateway Integration" });
    const session3 = createMockSession({ id: "s3", title: "Database Migration Script" });

    testRoot.render(
      React.createElement(Sidebar, {
        sessions: [session1, session2, session3],
        activeSessionId: null,
        workspaceDir: "/workspace",
        onNewChat: () => {},
        onSelectSession: () => {},
        onDeleteSession: () => {},
      })
    );
    testRoot.renderer.flush();

    // Initially all 3 sessions are rendered
    expect(testRoot.renderer.findByTestId("sidebar-session-s1")).toBeDefined();
    expect(testRoot.renderer.findByTestId("sidebar-session-s2")).toBeDefined();
    expect(testRoot.renderer.findByTestId("sidebar-session-s3")).toBeDefined();

    // Type "gateway" into search input
    const searchInput = testRoot.renderer.findByTestId("sidebar-search-input");
    expect(searchInput).toBeDefined();
    testRoot.renderer.nativeSimulateKeystrokes(searchInput!.id, "gateway");
    testRoot.renderer.flush();

    // Only session 2 should remain visible
    expect(testRoot.renderer.findByTestId("sidebar-session-s1")).toBeUndefined();
    expect(testRoot.renderer.findByTestId("sidebar-session-s2")).toBeDefined();
    expect(testRoot.renderer.findByTestId("sidebar-session-s3")).toBeUndefined();

    // Type query matching nothing
    testRoot.renderer.nativeSimulateKeystrokes(searchInput!.id, "xyz_not_found");
    testRoot.renderer.flush();

    expect(testRoot.renderer.findByTestId("sidebar-empty-state")).toBeDefined();
    const emptyTitle = testRoot.renderer.findByTestId("sidebar-empty-title");
    expect(getElementText(testRoot.renderer, emptyTitle)).toBe("No chats found");

    // Clear search query by clicking clear button
    const clearBtn = testRoot.renderer.findByTestId("sidebar-clear-search");
    expect(clearBtn).toBeDefined();
    const clearBounds = testRoot.renderer.getElementBounds(clearBtn!.id);
    expect(clearBounds).not.toBeNull();
    testRoot.renderer.nativeSimulateClick(
      clearBounds!.x + clearBounds!.width / 2,
      clearBounds!.y + clearBounds!.height / 2
    );
    testRoot.renderer.flush();

    // All sessions should be restored
    expect(testRoot.renderer.findByTestId("sidebar-session-s1")).toBeDefined();
    expect(testRoot.renderer.findByTestId("sidebar-session-s2")).toBeDefined();
    expect(testRoot.renderer.findByTestId("sidebar-session-s3")).toBeDefined();
  });

  it("triggers onNewChat callback when + New Chat button is clicked", () => {
    testRoot = createTestRoot();
    const onNewChat = mock();

    testRoot.render(
      React.createElement(Sidebar, {
        sessions: [],
        activeSessionId: null,
        workspaceDir: "/workspace",
        onNewChat,
        onSelectSession: () => {},
        onDeleteSession: () => {},
      })
    );
    testRoot.renderer.flush();

    const newChatBtn = testRoot.renderer.findByTestId("sidebar-new-chat-button");
    expect(newChatBtn).toBeDefined();

    const bounds = testRoot.renderer.getElementBounds(newChatBtn!.id);
    expect(bounds).not.toBeNull();
    testRoot.renderer.nativeSimulateClick(
      bounds!.x + bounds!.width / 2,
      bounds!.y + bounds!.height / 2
    );

    expect(onNewChat).toHaveBeenCalledTimes(1);
  });

  it("triggers onSelectSession when clicking on a session row content", () => {
    testRoot = createTestRoot();
    const onSelectSession = mock();
    const session = createMockSession({ id: "sess-select-1", title: "Select Me" });

    testRoot.render(
      React.createElement(Sidebar, {
        sessions: [session],
        activeSessionId: null,
        workspaceDir: "/workspace",
        onNewChat: () => {},
        onSelectSession,
        onDeleteSession: () => {},
      })
    );
    testRoot.renderer.flush();

    const contentEl = testRoot.renderer.findByTestId("sidebar-session-content-sess-select-1");
    expect(contentEl).toBeDefined();

    const bounds = testRoot.renderer.getElementBounds(contentEl!.id);
    expect(bounds).not.toBeNull();
    testRoot.renderer.nativeSimulateClick(
      bounds!.x + bounds!.width / 2,
      bounds!.y + bounds!.height / 2
    );

    expect(onSelectSession).toHaveBeenCalledTimes(1);
    expect(onSelectSession).toHaveBeenCalledWith("sess-select-1");
  });

  it("triggers onDeleteSession without triggering onSelectSession", () => {
    testRoot = createTestRoot();
    const onSelectSession = mock();
    const onDeleteSession = mock();
    const session = createMockSession({ id: "sess-del-1", title: "Delete Me" });

    testRoot.render(
      React.createElement(Sidebar, {
        sessions: [session],
        activeSessionId: null,
        workspaceDir: "/workspace",
        onNewChat: () => {},
        onSelectSession,
        onDeleteSession,
      })
    );
    testRoot.renderer.flush();

    const deleteBtn = testRoot.renderer.findByTestId("sidebar-session-delete-sess-del-1");
    expect(deleteBtn).toBeDefined();

    const bounds = testRoot.renderer.getElementBounds(deleteBtn!.id);
    expect(bounds).not.toBeNull();
    testRoot.renderer.nativeSimulateClick(
      bounds!.x + bounds!.width / 2,
      bounds!.y + bounds!.height / 2
    );

    expect(onDeleteSession).toHaveBeenCalledTimes(1);
    expect(onDeleteSession).toHaveBeenCalledWith("sess-del-1");
    expect(onSelectSession).not.toHaveBeenCalled();
  });

  it("triggers onSelectWorkspace when workspace chip is clicked", () => {
    testRoot = createTestRoot();
    const onSelectWorkspace = mock();

    testRoot.render(
      React.createElement(Sidebar, {
        sessions: [],
        activeSessionId: null,
        workspaceDir: "/Users/alex/projects/fusion",
        onNewChat: () => {},
        onSelectSession: () => {},
        onDeleteSession: () => {},
        onSelectWorkspace,
      })
    );
    testRoot.renderer.flush();

    const chip = testRoot.renderer.findByTestId("sidebar-workspace-chip");
    expect(chip).toBeDefined();

    const bounds = testRoot.renderer.getElementBounds(chip!.id);
    expect(bounds).not.toBeNull();
    testRoot.renderer.nativeSimulateClick(
      bounds!.x + bounds!.width / 2,
      bounds!.y + bounds!.height / 2
    );

    expect(onSelectWorkspace).toHaveBeenCalledTimes(1);
  });

  it("triggers onOpenSettings when settings button is clicked", () => {
    testRoot = createTestRoot();
    const onOpenSettings = mock();

    testRoot.render(
      React.createElement(Sidebar, {
        sessions: [],
        activeSessionId: null,
        workspaceDir: "/workspace",
        onNewChat: () => {},
        onSelectSession: () => {},
        onDeleteSession: () => {},
        onOpenSettings,
      })
    );
    testRoot.renderer.flush();

    const settingsBtn = testRoot.renderer.findByTestId("sidebar-settings-button");
    expect(settingsBtn).toBeDefined();

    const bounds = testRoot.renderer.getElementBounds(settingsBtn!.id);
    expect(bounds).not.toBeNull();
    testRoot.renderer.nativeSimulateClick(
      bounds!.x + bounds!.width / 2,
      bounds!.y + bounds!.height / 2
    );

    expect(onOpenSettings).toHaveBeenCalledTimes(1);
  });

  it("renders user profile with default name and initial", () => {
    testRoot = createTestRoot();
    testRoot.render(
      React.createElement(Sidebar, {
        sessions: [],
        activeSessionId: null,
        workspaceDir: "/workspace",
        onNewChat: () => {},
        onSelectSession: () => {},
        onDeleteSession: () => {},
      })
    );
    testRoot.renderer.flush();

    const initialEl = testRoot.renderer.findByTestId("sidebar-user-initial");
    expect(getElementText(testRoot.renderer, initialEl)).toBe("A");

    const nameEl = testRoot.renderer.findByTestId("sidebar-user-name");
    expect(getElementText(testRoot.renderer, nameEl)).toBe("Aung Myat Moe");
  });

  it("renders user profile with custom userName prop", () => {
    testRoot = createTestRoot();
    testRoot.render(
      React.createElement(Sidebar, {
        sessions: [],
        activeSessionId: null,
        workspaceDir: "/workspace",
        userName: "Jane Developer",
        onNewChat: () => {},
        onSelectSession: () => {},
        onDeleteSession: () => {},
      })
    );
    testRoot.renderer.flush();

    const initialEl = testRoot.renderer.findByTestId("sidebar-user-initial");
    expect(getElementText(testRoot.renderer, initialEl)).toBe("J");

    const nameEl = testRoot.renderer.findByTestId("sidebar-user-name");
    expect(getElementText(testRoot.renderer, nameEl)).toBe("Jane Developer");
  });

  it("renders workspace folder name and truncated path correctly", () => {
    testRoot = createTestRoot();
    testRoot.render(
      React.createElement(Sidebar, {
        sessions: [],
        activeSessionId: null,
        workspaceDir: "/Users/dev/repos/my-project",
        onNewChat: () => {},
        onSelectSession: () => {},
        onDeleteSession: () => {},
      })
    );
    testRoot.renderer.flush();

    const folderEl = testRoot.renderer.findByTestId("sidebar-workspace-folder");
    expect(getElementText(testRoot.renderer, folderEl)).toBe("my-project");

    const pathEl = testRoot.renderer.findByTestId("sidebar-workspace-path");
    const pathText = getElementText(testRoot.renderer, pathEl);
    expect(pathText).toContain("📁");
    expect(pathText).toContain("my-project");
  });
});
