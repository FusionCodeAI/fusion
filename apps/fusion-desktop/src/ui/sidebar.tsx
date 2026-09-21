import React, { useState, useMemo } from "react";
import type { ChatSession } from "../types";
import { icons } from "./icons";

export interface SidebarProps {
  sessions: readonly ChatSession[];
  activeSessionId: string | null;
  workspaceDir: string;
  userName?: string;
  onNewChat: () => void;
  onSelectSession: (id: string) => void;
  onDeleteSession: (id: string) => void;
  onToggleSidebar?: () => void;
  onSelectWorkspace?: () => void;
  onOpenSettings?: () => void;
}

export function formatRelativeTime(timestamp: number, now: number = Date.now()): string {
  if (!timestamp || isNaN(timestamp) || timestamp <= 0) return "just now";
  const diff = now - timestamp;
  if (diff < 0) return "just now";
  const minute = 60 * 1000;
  const hour = 60 * minute;
  const day = 24 * hour;
  const week = 7 * day;

  if (diff < minute) return "just now";
  if (diff < hour) return `${Math.floor(diff / minute)}m`;
  if (diff < day) return `${Math.floor(diff / hour)}h`;
  if (diff < 2 * day) return "yesterday";
  if (diff < week) return `${Math.floor(diff / day)}d`;
  if (diff < 30 * day) return `${Math.floor(diff / week)}w`;
  const months = Math.floor(diff / (30 * day));
  if (months < 12) return `${months}mo`;
  return `${Math.floor(diff / (365 * day))}y`;
}

export function getWorkspaceFolderName(workspaceDir: string): string {
  if (workspaceDir === "/") return "/";
  if (!workspaceDir) return "workspace";
  const parts = workspaceDir.replace(/[\\/]+$/, "").split(/[\\/]/);
  return parts[parts.length - 1] || "workspace";
}

export function truncatePath(path: string, maxLength: number = 30): string {
  if (!path || path.length <= maxLength) return path;
  return "..." + path.slice(-(maxLength - 3));
}

export function Sidebar({
  sessions,
  activeSessionId,
  workspaceDir,
  userName = "Aung Myat Moe",
  onNewChat,
  onSelectSession,
  onDeleteSession,
  onToggleSidebar,
  onSelectWorkspace,
  onOpenSettings,
}: SidebarProps) {
  const [searchQuery, setSearchQuery] = useState("");
  const [isSearching, setIsSearching] = useState(false);

  const filteredSessions = useMemo(() => {
    const trimmed = searchQuery.trim().toLowerCase();
    if (!trimmed) return sessions;
    return sessions.filter((s) => s.title.toLowerCase().includes(trimmed));
  }, [sessions, searchQuery]);

  const folderName = useMemo(() => getWorkspaceFolderName(workspaceDir), [workspaceDir]);

  return (
    <div
      testId="sidebar"
      style={{
        width: 220,
        height: "100%",
        backgroundColor: "#f7f7f8",
        borderRightWidth: 1,
        borderColor: "#e5e5e8",
        display: "flex",
        flexDirection: "column",
        justifyContent: "space-between",
        flexShrink: 0,
      }}
    >
      {/* Top Container: Header & Session/Project List */}
      <div
        style={{
          display: "flex",
          flexDirection: "column",
          flexGrow: 1,
          overflow: "hidden",
        }}
      >
        {/* Top Header Row: macOS traffic lights clearance (paddingLeft: 78), sidebar toggle and arrows */}
        <div
          style={{
            height: 40,
            paddingLeft: 78, // Comfortable clearance for macOS traffic lights at x: 18, y: 14
            paddingRight: 12,
            display: "flex",
            flexDirection: "row",
            alignItems: "center",
            justifyContent: "space-between",
            borderBottomWidth: 1,
            borderColor: "#f0f0f2",
            flexShrink: 0,
          }}
        >
          {/* Left: Sidebar Toggle */}
          <div
            role="button"
            onClick={onToggleSidebar}
            style={{
              cursor: "pointer",
              padding: 2,
              display: "flex",
              alignItems: "center",
            }}
          >
            <svg source={icons.sidebarToggle} style={{ width: 14, height: 14, color: "#52525b" }} />
          </div>

          {/* Right: History Arrows */}
          <div style={{ display: "flex", flexDirection: "row", alignItems: "center", gap: 8 }}>
            <div style={{ cursor: "pointer", display: "flex", alignItems: "center" }}>
              <svg source={icons.arrowLeft} style={{ width: 13, height: 13, color: "#8e8e93" }} />
            </div>
            <div style={{ cursor: "pointer", display: "flex", alignItems: "center" }}>
              <svg source={icons.arrowRight} style={{ width: 13, height: 13, color: "#8e8e93" }} />
            </div>
          </div>
        </div>

        {/* Primary Navigation Actions */}
        <div
          testId="sidebar-header"
          style={{
            display: "flex",
            flexDirection: "column",
            paddingLeft: 8,
            paddingRight: 8,
            paddingTop: 8,
            paddingBottom: 6,
            gap: 2,
          }}
        >
          {/* New Chat */}
          <div
            testId="sidebar-new-chat-button"
            role="button"
            aria-label="New Chat"
            onClick={onNewChat}
            style={{
              display: "flex",
              flexDirection: "row",
              alignItems: "center",
              gap: 8,
              height: 30,
              borderRadius: 6,
              paddingLeft: 8,
              paddingRight: 8,
              cursor: "pointer",
              hover: { backgroundColor: "#ececee" },
            }}
          >
            <svg source={icons.newChat} style={{ width: 13, height: 13, color: "#52525b" }} />
            <text style={{ fontSize: 13, color: "#27272a", fontWeight: "400" }}>New Chat</text>
          </div>

          {/* Search Input Box */}
          <div
            style={{
              display: "flex",
              flexDirection: "row",
              alignItems: "center",
              gap: 8,
              height: 30,
              borderRadius: 6,
              paddingLeft: 8,
              paddingRight: 8,
              backgroundColor: "#ffffff",
              borderWidth: 1,
              borderColor: "#e5e5e8",
            }}
          >
            <svg source={icons.search} style={{ width: 13, height: 13, color: "#52525b" }} />
            <input
              testId="sidebar-search-input"
              value={searchQuery}
              onChange={(e: { value?: string }) => setSearchQuery(e.value ?? "")}
              placeholder="Search"
              style={{
                flexGrow: 1,
                fontSize: 12,
                color: "#18181b",
                backgroundColor: "transparent",
                borderWidth: 0,
                padding: 0,
              }}
            />
            {searchQuery ? (
              <div
                testId="sidebar-clear-search"
                role="button"
                aria-label="Clear Search"
                onClick={() => setSearchQuery("")}
                style={{ paddingLeft: 4, cursor: "pointer" }}
              >
                <svg source={icons.close} style={{ width: 10, height: 10, color: "#8e8e93" }} />
              </div>
            ) : null}
          </div>

          {/* Automations */}
          <div
            style={{
              display: "flex",
              flexDirection: "row",
              alignItems: "center",
              gap: 8,
              height: 30,
              borderRadius: 6,
              paddingLeft: 8,
              paddingRight: 8,
              cursor: "pointer",
              hover: { backgroundColor: "#ececee" },
            }}
          >
            <svg source={icons.automations} style={{ width: 13, height: 13, color: "#52525b" }} />
            <text style={{ fontSize: 13, color: "#27272a" }}>Automations</text>
          </div>

          {/* Customize */}
          <div
            style={{
              display: "flex",
              flexDirection: "row",
              alignItems: "center",
              gap: 8,
              height: 30,
              borderRadius: 6,
              paddingLeft: 8,
              paddingRight: 8,
              cursor: "pointer",
              hover: { backgroundColor: "#ececee" },
            }}
          >
            <svg source={icons.customize} style={{ width: 13, height: 13, color: "#52525b" }} />
            <text style={{ fontSize: 13, color: "#27272a" }}>Customize</text>
          </div>
        </div>

        {/* Section: Projects */}
        <div
          style={{
            display: "flex",
            flexDirection: "row",
            alignItems: "center",
            justifyContent: "space-between",
            paddingLeft: 14,
            paddingRight: 14,
            paddingTop: 8,
            paddingBottom: 4,
          }}
        >
          <text style={{ fontSize: 11, fontWeight: "500", color: "#8e8e93" }}>Projects</text>
          <div style={{ cursor: "pointer", display: "flex", alignItems: "center" }}>
            <svg source={icons.plus} style={{ width: 10, height: 10, color: "#8e8e93" }} />
          </div>
        </div>

        <div style={{ paddingLeft: 8, paddingRight: 8, paddingBottom: 6 }}>
          <div
            style={{
              display: "flex",
              flexDirection: "row",
              alignItems: "center",
              gap: 8,
              height: 28,
              paddingLeft: 8,
              borderRadius: 6,
              cursor: "pointer",
              hover: { backgroundColor: "#ececee" },
            }}
          >
            <svg source={icons.circleDashed} style={{ width: 12, height: 12, color: "#8e8e93" }} />
            <text style={{ fontSize: 12, color: "#71717a" }}>New Project</text>
          </div>
        </div>

        {/* Section: Repositories */}
        <div
          style={{
            display: "flex",
            flexDirection: "row",
            alignItems: "center",
            justifyContent: "space-between",
            paddingLeft: 14,
            paddingRight: 14,
            paddingTop: 6,
            paddingBottom: 4,
          }}
        >
          <text style={{ fontSize: 11, fontWeight: "500", color: "#8e8e93" }}>Repositories</text>
          <div style={{ display: "flex", flexDirection: "row", alignItems: "center", gap: 8 }}>
            <svg source={icons.filter} style={{ width: 11, height: 11, color: "#8e8e93" }} />
            <svg source={icons.folder} style={{ width: 11, height: 11, color: "#8e8e93" }} />
          </div>
        </div>

        {/* Repositories & Active Conversations */}
        <div
          testId="sidebar-session-list"
          style={{
            display: "flex",
            flexDirection: "column",
            flexGrow: 1,
            overflow: "scroll",
            paddingLeft: 8,
            paddingRight: 8,
            gap: 1,
          }}
        >
          {/* No Repo / Active Project */}
          <div
            testId="sidebar-workspace-chip"
            onClick={onSelectWorkspace}
            style={{
              display: "flex",
              flexDirection: "row",
              alignItems: "center",
              gap: 8,
              height: 28,
              paddingLeft: 8,
              paddingRight: 8,
              borderRadius: 6,
              cursor: "pointer",
              overflow: "hidden",
              hover: { backgroundColor: "#ececee" },
            }}
          >
            <svg source={icons.home} style={{ width: 13, height: 13, color: "#71717a" }} />
            <text
              testId="sidebar-workspace-folder"
              style={{ fontSize: 12, color: "#3f3f46" }}
            >
              {folderName || "No Repo"}
            </text>
          </div>

          {/* Conversations */}
          {filteredSessions.length === 0 ? (
            <div
              testId="sidebar-empty-state"
              style={{
                display: "flex",
                flexDirection: "column",
                alignItems: "center",
                padding: 14,
                gap: 4,
              }}
            >
              <text
                testId="sidebar-empty-title"
                style={{ fontSize: 11, color: "#8e8e93" }}
              >
                {searchQuery ? "No chats found" : "No chats yet"}
              </text>
              <text
                testId="sidebar-empty-subtitle"
                style={{ fontSize: 11, color: "#8e8e93" }}
              >
                Click + New Chat to get started
              </text>
            </div>
          ) : (
            filteredSessions.map((session) => {
              const isActive = session.id === activeSessionId;
              return (
                <div
                  key={session.id}
                  testId={`sidebar-session-${session.id}`}
                  role="button"
                  aria-selected={isActive}
                  style={{
                    display: "flex",
                    flexDirection: "row",
                    alignItems: "center",
                    justifyContent: "space-between",
                    height: 30,
                    borderRadius: 6,
                    paddingLeft: 8,
                    paddingRight: 8,
                    backgroundColor: isActive ? "#ebebec" : "transparent",
                    cursor: "pointer",
                    hover: { backgroundColor: isActive ? "#ebebec" : "#efeff1" },
                  }}
                >
                  <div
                    testId={`sidebar-session-content-${session.id}`}
                    onClick={() => onSelectSession(session.id)}
                    style={{
                      display: "flex",
                      flexDirection: "row",
                      alignItems: "center",
                      gap: 6,
                      flexGrow: 1,
                      overflow: "hidden",
                    }}
                  >
                    <text style={{ fontSize: 11, color: isActive ? "#18181b" : "#8e8e93" }}>
                      •
                    </text>
                    <text
                      testId={`sidebar-session-title-${session.id}`}
                      style={{
                        fontSize: 12,
                        fontWeight: isActive ? "600" : "400",
                        color: isActive ? "#18181b" : "#3f3f46",
                      }}
                    >
                      {session.title.length > 28
                        ? session.title.slice(0, 28) + "..."
                        : session.title}
                    </text>
                  </div>

                  <div style={{ display: "flex", flexDirection: "row", alignItems: "center", gap: 4 }}>
                    <text
                      testId={`sidebar-session-time-${session.id}`}
                      style={{ fontSize: 11, color: "#8e8e93" }}
                    >
                      {formatRelativeTime(session.updatedAt || session.createdAt)}
                    </text>

                    <div
                      testId={`sidebar-session-delete-${session.id}`}
                      onClick={() => onDeleteSession(session.id)}
                      style={{ cursor: "pointer", paddingLeft: 2 }}
                    >
                      <svg source={icons.close} style={{ width: 9, height: 9, color: "#a1a1aa" }} />
                    </div>
                  </div>
                </div>
              );
            })
          )}
        </div>
      </div>

      {/* Footer Container */}
      <div
        testId="sidebar-footer"
        style={{
          display: "flex",
          flexDirection: "column",
          padding: 10,
          gap: 10,
        }}
      >
        {/* Getting Started Card */}
        <div
          style={{
            backgroundColor: "#f0f0f2",
            borderRadius: 8,
            padding: 10,
            display: "flex",
            flexDirection: "column",
            gap: 8,
          }}
        >
          <div
            style={{
              display: "flex",
              flexDirection: "row",
              alignItems: "center",
              justifyContent: "space-between",
            }}
          >
            <text style={{ fontSize: 11, fontWeight: "500", color: "#52525b" }}>
              Getting Started
            </text>
            <text style={{ fontSize: 10, color: "#8e8e93" }}>1/3 ⚪</text>
          </div>

          <div
            style={{
              height: 28,
              borderRadius: 6,
              backgroundColor: "#ffffff",
              borderWidth: 1,
              borderColor: "#e5e5e8",
              display: "flex",
              flexDirection: "row",
              alignItems: "center",
              justifyContent: "center",
              gap: 6,
              cursor: "pointer",
              hover: { backgroundColor: "#fbfbfb" },
            }}
          >
            <svg source={icons.github} style={{ width: 12, height: 12, color: "#18181b" }} />
            <text style={{ fontSize: 11, fontWeight: "500", color: "#18181b" }}>
              Connect GitHub
            </text>
          </div>
        </div>

        {/* User Profile & Settings Row */}
        <div
          style={{
            display: "flex",
            flexDirection: "row",
            alignItems: "center",
            justifyContent: "space-between",
            paddingLeft: 4,
            paddingRight: 4,
          }}
        >
          <div style={{ display: "flex", flexDirection: "row", alignItems: "center", gap: 8 }}>
            <div
              style={{
                width: 22,
                height: 22,
                borderRadius: 11,
                backgroundColor: "#e4e4e7",
                display: "flex",
                alignItems: "center",
                justifyContent: "center",
              }}
            >
              <text
                testId="sidebar-user-initial"
                style={{ fontSize: 11, fontWeight: "600", color: "#18181b" }}
              >
                {userName.charAt(0).toUpperCase()}
              </text>
            </div>
            <text
              testId="sidebar-user-name"
              style={{ fontSize: 12, color: "#18181b", fontWeight: "500" }}
            >
              {userName}
            </text>
          </div>

          <div
            testId="sidebar-settings-button"
            role="button"
            aria-label="Settings"
            onClick={onOpenSettings}
            style={{ cursor: "pointer", padding: 2 }}
          >
            <svg source={icons.settings} style={{ width: 14, height: 14, color: "#71717a" }} />
          </div>
        </div>
      </div>
    </div>
  );
}
