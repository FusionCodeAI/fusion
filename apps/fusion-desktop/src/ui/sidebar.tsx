import React, { useState, useMemo } from "react";
import type { ChatSession } from "../types";

export interface SidebarProps {
  sessions: ChatSession[];
  activeSessionId: string | null;
  workspaceDir: string;
  userName?: string;
  onNewChat: () => void;
  onSelectSession: (id: string) => void;
  onDeleteSession: (id: string) => void;
  onSelectWorkspace?: () => void;
  onOpenSettings?: () => void;
}

/**
 * Formats a timestamp into a clean relative time string.
 * ("just now", "5m", "2h", "yesterday", "3d", "2w", etc.)
 */
export function formatRelativeTime(timestamp: number, now: number = Date.now()): string {
  if (!timestamp || isNaN(timestamp) || timestamp <= 0) {
    return "just now";
  }

  const diff = now - timestamp;
  if (diff < 0) {
    return "just now";
  }

  const minute = 60 * 1000;
  const hour = 60 * minute;
  const day = 24 * hour;
  const week = 7 * day;

  if (diff < minute) {
    return "just now";
  }
  if (diff < hour) {
    return `${Math.floor(diff / minute)}m`;
  }
  if (diff < day) {
    return `${Math.floor(diff / hour)}h`;
  }
  if (diff < 2 * day) {
    return "yesterday";
  }
  if (diff < week) {
    return `${Math.floor(diff / day)}d`;
  }
  if (diff < 30 * day) {
    return `${Math.floor(diff / week)}w`;
  }
  const months = Math.floor(diff / (30 * day));
  if (months < 12) {
    return `${months}mo`;
  }
  return `${Math.floor(diff / (365 * day))}y`;
}

/**
 * Extracts folder name from a workspace directory path.
 */
export function getWorkspaceFolderName(workspaceDir: string): string {
  if (!workspaceDir || workspaceDir === "/" || workspaceDir === ".") {
    return workspaceDir || "workspace";
  }
  const normalized = workspaceDir.replace(/[/\\]+$/, "");
  const parts = normalized.split(/[/\\]/).filter(Boolean);
  return parts[parts.length - 1] || workspaceDir;
}

/**
 * Truncates path for compact display in the sidebar.
 */
export function truncatePath(path: string, maxLength: number = 30): string {
  if (!path) return "";
  if (path.length <= maxLength) return path;

  const parts = path.split(/[/\\]/).filter(Boolean);
  if (parts.length <= 2) {
    return path.slice(0, maxLength - 3) + "...";
  }

  const last = parts[parts.length - 1];
  const first = parts[0];
  const isAbsolute = path.startsWith("/");
  const prefix = isAbsolute ? `/${first}` : first;

  const candidate = `${prefix}/.../${last}`;
  if (candidate.length <= maxLength) {
    return candidate;
  }

  return path.slice(0, 12) + "..." + path.slice(-12);
}

// Crisp inline SVGs for GPUIX <svg source="..." />
const PLUS_ICON_SVG = `<svg xmlns="http://www.w3.org/2000/svg" width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="#ffffff" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round"><line x1="12" y1="5" x2="12" y2="19"/><line x1="5" y1="12" x2="19" y2="12"/></svg>`;

const SEARCH_ICON_SVG = `<svg xmlns="http://www.w3.org/2000/svg" width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="#64748b" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><circle cx="11" cy="11" r="8"/><line x1="21" y1="21" x2="16.65" y2="16.65"/></svg>`;

const TRASH_ICON_SVG = `<svg xmlns="http://www.w3.org/2000/svg" width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="#94a3b8" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><polyline points="3 6 5 6 21 6"/><path d="M19 6v14a2 2 0 0 1-2 2H7a2 2 0 0 1-2-2V6m3 0V4a2 2 0 0 1 2-2h4a2 2 0 0 1 2 2v2"/><line x1="10" y1="11" x2="10" y2="17"/><line x1="14" y1="11" x2="14" y2="17"/></svg>`;

const GEAR_ICON_SVG = `<svg xmlns="http://www.w3.org/2000/svg" width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="#94a3b8" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><circle cx="12" cy="12" r="3"/><path d="M19.4 15a1.65 1.65 0 0 0 .33 1.82l.06.06a2 2 0 0 1 0 2.83 2 2 0 0 1-2.83 0l-.06-.06a1.65 1.65 0 0 0-1.82-.33 1.65 1.65 0 0 0-1 1.51V21a2 2 0 0 1-2 2 2 2 0 0 1-2-2v-.09A1.65 1.65 0 0 0 9 19.4a1.65 1.65 0 0 0-1.82.33l-.06.06a2 2 0 0 1-2.83 0 2 2 0 0 1 0-2.83l.06-.06a1.65 1.65 0 0 0 .33-1.82 1.65 1.65 0 0 0-1.51-1H3a2 2 0 0 1-2-2 2 2 0 0 1 2-2h.09A1.65 1.65 0 0 0 4.6 9a1.65 1.65 0 0 0-.33-1.82l-.06-.06a2 2 0 0 1 0-2.83 2 2 0 0 1 2.83 0l.06.06a1.65 1.65 0 0 0 1.82.33H9a1.65 1.65 0 0 0 1-1.51V3a2 2 0 0 1 2-2 2 2 0 0 1 2 2v.09a1.65 1.65 0 0 0 1 1.51 1.65 1.65 0 0 0 1.82-.33l.06-.06a2 2 0 0 1 2.83 0 2 2 0 0 1 0 2.83l-.06.06a1.65 1.65 0 0 0-.33 1.82V9a1.65 1.65 0 0 0 1.51 1H21a2 2 0 0 1 2 2 2 2 0 0 1-2 2h-.09a1.65 1.65 0 0 0-1.51 1z"/></svg>`;

const CHAT_ICON_SVG = `<svg xmlns="http://www.w3.org/2000/svg" width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="#64748b" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M21 15a2 2 0 0 1-2 2H7l-4 4V5a2 2 0 0 1 2-2h14a2 2 0 0 1 2 2z"/></svg>`;

const EMPTY_CHAT_SVG = `<svg xmlns="http://www.w3.org/2000/svg" width="28" height="28" viewBox="0 0 24 24" fill="none" stroke="#475569" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round"><path d="M21 15a2 2 0 0 1-2 2H7l-4 4V5a2 2 0 0 1 2-2h14a2 2 0 0 1 2 2z"/></svg>`;

export function Sidebar({
  sessions,
  activeSessionId,
  workspaceDir,
  userName = "Aung Myat Moe",
  onNewChat,
  onSelectSession,
  onDeleteSession,
  onSelectWorkspace,
  onOpenSettings,
}: SidebarProps): React.JSX.Element {
  const [searchQuery, setSearchQuery] = useState("");

  const filteredSessions = useMemo(() => {
    const query = searchQuery.trim().toLowerCase();
    if (!query) {
      return sessions;
    }
    return sessions.filter((session) => session.title.toLowerCase().includes(query));
  }, [sessions, searchQuery]);

  const folderName = useMemo(() => getWorkspaceFolderName(workspaceDir), [workspaceDir]);
  const truncatedPath = useMemo(() => truncatePath(workspaceDir), [workspaceDir]);
  const displayPath = useMemo(() => {
    return truncatedPath.startsWith("📁") ? truncatedPath : `📁 ${truncatedPath}`;
  }, [truncatedPath]);

  const displayName = userName.trim() || "Aung Myat Moe";
  const userInitial = displayName.charAt(0).toUpperCase() || "A";

  return (
    <div
      testId="sidebar"
      style={{
        width: 280,
        height: "100%",
        backgroundColor: "#0f172a",
        borderRightWidth: 1,
        borderColor: "#1e293b",
        display: "flex",
        flexDirection: "column",
        justifyContent: "space-between",
      }}
    >
      {/* Top Container: Header and Session List */}
      <div
        style={{
          display: "flex",
          flexDirection: "column",
          flexGrow: 1,
          overflow: "hidden",
        }}
      >
        {/* Header */}
        <div
          testId="sidebar-header"
          style={{
            display: "flex",
            flexDirection: "column",
            padding: 12,
            gap: 10,
            borderBottomWidth: 1,
            borderColor: "#1e293b",
          }}
        >
          {/* + New Chat Button */}
          <div
            testId="sidebar-new-chat-button"
            role="button"
            aria-label="New Chat"
            onClick={onNewChat}
            style={{
              display: "flex",
              flexDirection: "row",
              alignItems: "center",
              justifyContent: "center",
              gap: 8,
              height: 38,
              borderRadius: 8,
              backgroundColor: "#2563eb",
              paddingLeft: 14,
              paddingRight: 14,
              cursor: "pointer",
              hover: {
                backgroundColor: "#1d4ed8",
              },
              active: {
                backgroundColor: "#1e40af",
              },
            }}
          >
            <svg source={PLUS_ICON_SVG} style={{ width: 14, height: 14 }} />
            <text
              style={{
                color: "#ffffff",
                fontSize: 13,
                fontWeight: 600,
              }}
            >
              + New Chat
            </text>
          </div>

          {/* Search Input */}
          <div
            testId="sidebar-search-container"
            style={{
              display: "flex",
              flexDirection: "row",
              alignItems: "center",
              gap: 8,
              height: 34,
              borderRadius: 6,
              backgroundColor: "#1e293b",
              borderWidth: 1,
              borderColor: "#334155",
              paddingLeft: 10,
              paddingRight: 10,
            }}
          >
            <svg source={SEARCH_ICON_SVG} style={{ width: 14, height: 14 }} />
            <input
              testId="sidebar-search-input"
              placeholder="Search chats..."
              value={searchQuery}
              onChange={(e) => setSearchQuery(e.value ?? "")}
              style={{
                flexGrow: 1,
                height: 30,
                backgroundColor: "transparent",
                color: "#f1f5f9",
                fontSize: 12,
              }}
            />
            {searchQuery.length > 0 && (
              <div
                testId="sidebar-clear-search"
                role="button"
                aria-label="Clear search"
                onClick={() => setSearchQuery("")}
                style={{
                  cursor: "pointer",
                  padding: 2,
                  hover: {
                    opacity: 0.8,
                  },
                }}
              >
                <text style={{ color: "#94a3b8", fontSize: 11 }}>✕</text>
              </div>
            )}
          </div>
        </div>

        {/* Session List */}
        <div
          testId="sidebar-session-list"
          style={{
            display: "flex",
            flexGrow: 1,
            flexDirection: "column",
            overflow: "scroll",
            padding: 8,
            gap: 4,
          }}
        >
          {filteredSessions.length === 0 ? (
            /* Empty State */
            <div
              testId="sidebar-empty-state"
              style={{
                display: "flex",
                flexDirection: "column",
                alignItems: "center",
                justifyContent: "center",
                paddingTop: 36,
                paddingBottom: 36,
                paddingLeft: 16,
                paddingRight: 16,
                gap: 8,
              }}
            >
              <svg source={EMPTY_CHAT_SVG} style={{ width: 28, height: 28 }} />
              <text
                testId="sidebar-empty-title"
                style={{
                  color: "#94a3b8",
                  fontSize: 13,
                  fontWeight: 600,
                  textAlign: "center",
                }}
              >
                {sessions.length === 0 ? "No chats yet" : "No chats found"}
              </text>
              <text
                testId="sidebar-empty-subtitle"
                style={{
                  color: "#64748b",
                  fontSize: 11,
                  textAlign: "center",
                }}
              >
                {sessions.length === 0
                  ? "Click + New Chat to get started"
                  : `No chats matching "${searchQuery}"`}
              </text>
            </div>
          ) : (
            filteredSessions.map((session) => {
              const isActive = session.id === activeSessionId;
              const timeFormatted = formatRelativeTime(session.updatedAt || session.createdAt);

              return (
                <div
                  key={session.id}
                  testId={`sidebar-session-${session.id}`}
                  aria-selected={isActive}
                  style={{
                    display: "flex",
                    flexDirection: "row",
                    alignItems: "center",
                    height: 44,
                    borderRadius: 6,
                    backgroundColor: isActive ? "#1e293b" : "transparent",
                    borderLeftWidth: isActive ? 3 : 0,
                    borderColor: isActive ? "#3b82f6" : "transparent",
                    paddingLeft: isActive ? 7 : 10,
                    paddingRight: 8,
                    gap: 8,
                    hover: {
                      backgroundColor: isActive ? "#273549" : "#172033",
                    },
                  }}
                >
                  {/* Clickable Session Content */}
                  <div
                    testId={`sidebar-session-content-${session.id}`}
                    role="button"
                    aria-label={`Select ${session.title}`}
                    onClick={() => onSelectSession(session.id)}
                    style={{
                      display: "flex",
                      flexGrow: 1,
                      flexDirection: "row",
                      alignItems: "center",
                      gap: 8,
                      overflow: "hidden",
                      cursor: "pointer",
                      height: 40,
                    }}
                  >
                    <svg source={CHAT_ICON_SVG} style={{ width: 14, height: 14 }} />

                    <div
                      style={{
                        display: "flex",
                        flexDirection: "column",
                        flexGrow: 1,
                        overflow: "hidden",
                        gap: 2,
                      }}
                    >
                      <div
                        style={{
                          display: "flex",
                          flexDirection: "row",
                          alignItems: "center",
                          justifyContent: "space-between",
                          gap: 6,
                        }}
                      >
                        <text
                          testId={`sidebar-session-title-${session.id}`}
                          style={{
                            color: isActive ? "#ffffff" : "#cbd5e1",
                            fontSize: 13,
                            fontWeight: isActive ? 600 : 400,
                            textOverflow: "ellipsis",
                            whiteSpace: "nowrap",
                            flexGrow: 1,
                          }}
                        >
                          {session.title || "Untitled Chat"}
                        </text>
                        <text
                          testId={`sidebar-session-time-${session.id}`}
                          style={{
                            color: isActive ? "#93c5fd" : "#64748b",
                            fontSize: 11,
                            whiteSpace: "nowrap",
                          }}
                        >
                          {timeFormatted}
                        </text>
                      </div>
                    </div>
                  </div>

                  {/* Delete Button */}
                  <div
                    testId={`sidebar-session-delete-${session.id}`}
                    role="button"
                    aria-label={`Delete ${session.title}`}
                    onClick={() => onDeleteSession(session.id)}
                    style={{
                      display: "flex",
                      width: 24,
                      height: 24,
                      borderRadius: 4,
                      alignItems: "center",
                      justifyContent: "center",
                      cursor: "pointer",
                      hover: {
                        backgroundColor: "#ef444420",
                      },
                    }}
                  >
                    <svg source={TRASH_ICON_SVG} style={{ width: 13, height: 13 }} />
                  </div>
                </div>
              );
            })
          )}
        </div>
      </div>

      {/* Footer: Workspace Directory Chip & User Profile Card */}
      <div
        testId="sidebar-footer"
        style={{
          display: "flex",
          flexDirection: "column",
          padding: 10,
          gap: 8,
          borderTopWidth: 1,
          borderColor: "#1e293b",
        }}
      >
        {/* Workspace Directory Indicator Chip */}
        <div
          testId="sidebar-workspace-chip"
          role="button"
          aria-label={`Workspace: ${folderName}`}
          onClick={onSelectWorkspace}
          style={{
            display: "flex",
            flexDirection: "column",
            gap: 3,
            padding: 8,
            borderRadius: 6,
            backgroundColor: "#1e293b",
            borderWidth: 1,
            borderColor: "#334155",
            cursor: onSelectWorkspace ? "pointer" : "default",
            hover: onSelectWorkspace
              ? {
                  backgroundColor: "#273549",
                  borderColor: "#475569",
                }
              : undefined,
          }}
        >
          <div
            style={{
              display: "flex",
              flexDirection: "row",
              alignItems: "center",
              gap: 6,
            }}
          >
            <text style={{ color: "#94a3b8", fontSize: 12 }}>📁</text>
            <text
              testId="sidebar-workspace-folder"
              style={{
                color: "#f1f5f9",
                fontSize: 12,
                fontWeight: 600,
                textOverflow: "ellipsis",
                whiteSpace: "nowrap",
              }}
            >
              {folderName}
            </text>
          </div>
          <text
            testId="sidebar-workspace-path"
            style={{
              color: "#64748b",
              fontSize: 11,
              textOverflow: "ellipsis",
              whiteSpace: "nowrap",
            }}
          >
            {displayPath}
          </text>
        </div>

        {/* User Profile Card */}
        <div
          testId="sidebar-user-profile"
          style={{
            display: "flex",
            flexDirection: "row",
            alignItems: "center",
            justifyContent: "space-between",
            padding: 8,
            borderRadius: 8,
            backgroundColor: "#1e293b",
            borderWidth: 1,
            borderColor: "#334155",
          }}
        >
          <div
            style={{
              display: "flex",
              flexDirection: "row",
              alignItems: "center",
              gap: 10,
              flexGrow: 1,
              overflow: "hidden",
            }}
          >
            {/* User Avatar Initial */}
            <div
              testId="sidebar-user-avatar"
              style={{
                display: "flex",
                width: 30,
                height: 30,
                borderRadius: 15,
                backgroundColor: "#3b82f6",
                alignItems: "center",
                justifyContent: "center",
              }}
            >
              <text
                testId="sidebar-user-initial"
                style={{
                  color: "#ffffff",
                  fontSize: 13,
                  fontWeight: 600,
                }}
              >
                {userInitial}
              </text>
            </div>

            {/* Display Name */}
            <text
              testId="sidebar-user-name"
              style={{
                color: "#f8fafc",
                fontSize: 13,
                fontWeight: 500,
                textOverflow: "ellipsis",
                whiteSpace: "nowrap",
                flexGrow: 1,
              }}
            >
              {displayName}
            </text>
          </div>

          {/* Settings Gear Button */}
          <div
            testId="sidebar-settings-button"
            role="button"
            aria-label="Settings"
            onClick={onOpenSettings}
            style={{
              display: "flex",
              width: 28,
              height: 28,
              borderRadius: 6,
              alignItems: "center",
              justifyContent: "center",
              cursor: onOpenSettings ? "pointer" : "default",
              hover: {
                backgroundColor: "#334155",
              },
            }}
          >
            <svg source={GEAR_ICON_SVG} style={{ width: 16, height: 16 }} />
          </div>
        </div>
      </div>
    </div>
  );
}

export default Sidebar;
