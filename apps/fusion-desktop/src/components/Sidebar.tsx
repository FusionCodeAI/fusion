import React, { useState, useMemo } from "react";
import {
  ArrowLeft,
  ArrowRight,
  Search,
  Plus,
  Blocks,
  Clock,
  Sliders,
  Cable,
  CircleUser,
  FolderTree,
  ChevronDown,
  Filter,
  Settings,
  X,
  Pin,
} from "lucide-react";
import { ClineAvatar } from "./ClineAvatar";
import { FusionMascot } from "./FusionMascot";

export interface SidebarSessionItem {
  id: string;
  title: string;
  createdAt?: number;
  updatedAt?: number;
  workspace?: string;
  workspaceName?: string;
  isPinned?: boolean;
}
export type SettingsSectionId = "general" | "api" | "account";

export interface SidebarProps {
  sessions?: readonly SidebarSessionItem[];
  activeSessionId?: string | null;
  workspaceDir?: string;
  userName?: string;
  currentView?: "chat" | "customize" | "settings";
  settingsSection?: SettingsSectionId;
  canNavigateBack?: boolean;
  canNavigateForward?: boolean;
  onNewChat?: () => void;
  onSelectSession?: (id: string) => void;
  onDeleteSession?: (id: string) => void;
  onToggleSidebar?: () => void;
  onHistoryBack?: () => void;
  onHistoryForward?: () => void;
  onOpenSearch?: () => void;
  onCustomize?: () => void;
  onOpenSettings?: (section?: SettingsSectionId) => void;
  onTogglePinSession?: (id: string) => void;
  width?: number;
  onResize?: (width: number) => void;
  onResetWidth?: () => void;
  className?: string;
}

export function formatRelativeTime(timestamp?: number, now: number = Date.now()): string {
  if (!timestamp || isNaN(timestamp) || timestamp <= 0) return "1h";
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

export function getWorkspaceFolderName(workspaceDir?: string): string {
  if (!workspaceDir || workspaceDir === "/") return "No Repo";
  if (workspaceDir === ".") return "fusion";
  const parts = workspaceDir.replace(/[\\/]+$/, "").split(/[\\/]/);
  return parts[parts.length - 1] || "No Repo";
}

export function Sidebar({
  sessions,
  activeSessionId,
  workspaceDir = ".",
  currentView = "chat",
  settingsSection = "general",
  canNavigateBack = false,
  canNavigateForward = false,
  onNewChat,
  onSelectSession,
  onDeleteSession,
  onOpenSearch,
  onHistoryBack,
  onHistoryForward,
  onCustomize,
  onOpenSettings,
  onTogglePinSession,
  width,
  onResize,
  onResetWidth,
  className = "",
}: SidebarProps) {
  const [sortMode, setSortMode] = useState<"time" | "project">("project");
  const [isFilterCurrentWorkspace, setIsFilterCurrentWorkspace] = useState(false);
  const [collapsedProjects, setCollapsedProjects] = useState<Set<string>>(() => new Set());

  const defaultSessions: SidebarSessionItem[] = useMemo(
    () => [
      {
        id: "session-default",
        title: "General chat conversation",
        createdAt: Date.now() - 3600 * 1000,
        updatedAt: Date.now() - 3600 * 1000,
        workspaceName: getWorkspaceFolderName(workspaceDir),
      },
    ],
    [workspaceDir]
  );

  const sessionList = sessions && sessions.length > 0 ? sessions : defaultSessions;
  const currentWorkspaceName = getWorkspaceFolderName(workspaceDir).toLowerCase();

  // Filter sessions if current workspace filter is active
  const displayedSessions = useMemo(() => {
    if (!isFilterCurrentWorkspace) return sessionList;
    return sessionList.filter((s) => {
      const sWs = (s.workspaceName || "").toLowerCase();
      const sPath = (s.workspace || "").toLowerCase();
      return (
        sWs === currentWorkspaceName ||
        sPath.includes(currentWorkspaceName) ||
        (!s.workspace && currentWorkspaceName === "fusion")
      );
    });
  }, [sessionList, isFilterCurrentWorkspace, currentWorkspaceName]);

  // Group by project when sortMode === "project"
  const projectGroups = useMemo(() => {
    const groups = new Map<string, { label: string; workspacePath: string; sessions: SidebarSessionItem[] }>();

    for (const session of displayedSessions) {
      const wsPath = session.workspace || "";
      const wsName = session.workspaceName || (wsPath ? getWorkspaceFolderName(wsPath) : "General Chat");
      const key = wsName.toLowerCase();
      const existing = groups.get(key);
      if (existing) {
        existing.sessions.push(session);
      } else {
        groups.set(key, { label: wsName, workspacePath: wsPath, sessions: [session] });
      }
    }

    // Sort sessions in each group so pinned sessions appear at the top
    for (const group of groups.values()) {
      group.sessions.sort((a, b) => {
        if (Boolean(a.isPinned) !== Boolean(b.isPinned)) {
          return a.isPinned ? -1 : 1;
        }
        return (b.updatedAt || b.createdAt || 0) - (a.updatedAt || a.createdAt || 0);
      });
    }

    // Sort projects so active workspace project is first, then alphabetical
    return Array.from(groups.values()).sort((a, b) => {
      if (a.label.toLowerCase() === currentWorkspaceName) return -1;
      if (b.label.toLowerCase() === currentWorkspaceName) return 1;
      return a.label.localeCompare(b.label);
    });
  }, [displayedSessions, currentWorkspaceName]);

  const toggleProjectCollapse = (label: string) => {
    setCollapsedProjects((prev) => {
      const next = new Set(prev);
      if (next.has(label)) next.delete(label);
      else next.add(label);
      return next;
    });
  };

  const handleMouseDown = (e: React.MouseEvent) => {
    e.preventDefault();
    const startX = e.clientX;
    const initialWidth = width ?? 256;

    const onMouseMove = (moveEvent: MouseEvent) => {
      const delta = moveEvent.clientX - startX;
      const newWidth = Math.min(Math.max(initialWidth + delta, 180), 520);
      onResize?.(newWidth);
    };

    const onMouseUp = () => {
      document.removeEventListener("mousemove", onMouseMove);
      document.removeEventListener("mouseup", onMouseUp);
    };

    document.addEventListener("mousemove", onMouseMove);
    document.addEventListener("mouseup", onMouseUp);
  };

  const renderSessionRow = (session: SidebarSessionItem) => {
    const isActive = session.id === activeSessionId && currentView === "chat";
    const isPinned = Boolean(session.isPinned);
    return (
      <div
        key={session.id}
        data-testid="sidebar-session-item"
        data-session-id={session.id}
        onClick={() => onSelectSession?.(session.id)}
        className={`group flex items-center justify-between px-2.5 py-1.5 rounded-lg text-xs cursor-pointer transition-colors ${
          isActive
            ? "bg-zinc-200/70 font-medium text-zinc-900"
            : "hover:bg-zinc-100 text-zinc-700 font-normal"
        }`}
      >
        <div className="flex items-center gap-1.5 truncate pr-2 min-w-0">
          {isPinned && (
            <Pin className="w-3 h-3 text-zinc-800 shrink-0 fill-current rotate-45" />
          )}
          <span className="truncate">{session.title}</span>
        </div>
        <div className="flex items-center gap-1 shrink-0 text-zinc-400">
          <span className="text-[11px] tabular-nums">
            {formatRelativeTime(session.updatedAt || session.createdAt)}
          </span>
          {onTogglePinSession && (
            <button
              type="button"
              data-testid={`sidebar-pin-${session.id}`}
              onClick={(e) => {
                e.stopPropagation();
                onTogglePinSession(session.id);
              }}
              className={`${
                isPinned ? "opacity-100 text-zinc-800" : "opacity-0 group-hover:opacity-100 text-zinc-400 hover:text-zinc-800"
              } p-0.5 rounded hover:bg-zinc-200 transition-opacity cursor-pointer`}
              title={isPinned ? "Unpin session" : "Pin session"}
              aria-label={isPinned ? "Unpin session" : "Pin session"}
            >
              <Pin className={`w-3 h-3 ${isPinned ? "fill-current" : ""}`} />
            </button>
          )}
          {onDeleteSession && (
            <button
              type="button"
              onClick={(e) => {
                e.stopPropagation();
                onDeleteSession(session.id);
              }}
              className="opacity-0 group-hover:opacity-100 p-0.5 rounded hover:bg-zinc-200 text-zinc-400 hover:text-zinc-700 transition-opacity cursor-pointer"
              title="Delete session"
              aria-label="Delete session"
            >
              <X className="w-3 h-3" />
            </button>
          )}
        </div>
      </div>
    );
  };

  return (
    <aside
      data-testid="sidebar"
      style={width ? { width: `${width}px` } : undefined}
      className={`relative ${width ? "" : "w-64"} h-full shrink-0 flex flex-col justify-between border-r border-zinc-200/80 bg-[#fbfbfb] select-none text-zinc-800 ${className}`}
    >
      {/* Right Edge Resize Handle */}
      <div
        data-testid="sidebar-resize-handle"
        onMouseDown={handleMouseDown}
        onDoubleClick={onResetWidth}
        title="Drag to resize sidebar (double-click to reset)"
        className="absolute top-0 right-0 bottom-0 w-1 cursor-col-resize hover:w-1.5 hover:bg-[#5100cd]/40 active:bg-[#5100cd] transition-all z-20"
      />

      {/* Top Section */}
      <div className="flex flex-col min-h-0">
        {/* Row 1: Traffic lights clearance on left, navigation arrows on right matching Cline Image #1 & #2 */}
        <div
          data-tauri-drag-region
          className="h-12 pl-[76px] pr-2 flex items-center justify-end gap-0.5 border-b border-zinc-200/40"
        >
          <button
            type="button"
            data-testid="sidebar-nav-back"
            onClick={onHistoryBack}
            disabled={!canNavigateBack}
            className="size-8 rounded-md flex items-center justify-center text-zinc-500 hover:text-zinc-900 hover:bg-zinc-200/60 disabled:opacity-30 disabled:hover:bg-transparent cursor-pointer"
            title="Previous page"
            aria-label="Previous page"
          >
            <ArrowLeft className="w-4 h-4" />
          </button>
          <button
            type="button"
            data-testid="sidebar-nav-forward"
            onClick={onHistoryForward}
            disabled={!canNavigateForward}
            className="size-8 rounded-md flex items-center justify-center text-zinc-500 hover:text-zinc-900 hover:bg-zinc-200/60 disabled:opacity-30 disabled:hover:bg-transparent cursor-pointer"
            title="Next page"
            aria-label="Next page"
          >
            <ArrowRight className="w-4 h-4" />
          </button>
        </div>

        {/* Row 2: Logo on left, Search icon on right matching Cline Image #1 & #2 */}
        <div className="h-10 px-2 flex items-center justify-between">
          <button
            type="button"
            onClick={onNewChat}
            className="size-8 rounded-md flex items-center justify-center hover:bg-zinc-200/60 cursor-pointer"
            title="Home"
          >
            <FusionMascot className="w-5 h-5 cursor-pointer" animated={true} />
          </button>

          <button
            type="button"
            data-testid="sidebar-search-btn"
            onClick={onOpenSearch}
            className="size-8 rounded-md flex items-center justify-center text-zinc-500 hover:text-zinc-900 hover:bg-zinc-200/60 cursor-pointer"
            title="Search sessions (Cmd/Ctrl+K)"
            aria-label="Search sessions"
          >
            <Search className="w-4 h-4" />
          </button>
        </div>

        {/* Primary Action Buttons: Session, Schedule, Customize matching Image #1 & #2 */}
        <div className="px-2 pt-1 pb-1 space-y-0.5">
          {/* + Session */}
          <button
            type="button"
            data-testid="sidebar-new-chat"
            onClick={onNewChat}
            className={`w-full flex items-center gap-2 px-3 py-1.5 rounded-lg text-xs font-medium transition-colors cursor-pointer ${
              currentView === "chat" ? "bg-zinc-200/70 text-zinc-900 font-semibold" : "text-zinc-700 hover:bg-zinc-100"
            }`}
          >
            <Plus className="w-3.5 h-3.5 text-zinc-700" />
            <span>Session</span>
          </button>

          {/* Schedule */}
          <button
            type="button"
            data-testid="sidebar-schedule-item"
            onClick={() => onOpenSettings?.("general")}
            className="w-full flex items-center gap-2 px-3 py-1.5 rounded-lg hover:bg-zinc-100 text-xs font-normal text-zinc-700 transition-colors cursor-pointer"
          >
            <Clock className="w-3.5 h-3.5 text-zinc-500" />
            <span>Schedule</span>
          </button>

          {/* Customize */}
          <button
            type="button"
            data-testid="sidebar-customize-item"
            onClick={onCustomize}
            className={`w-full flex items-center gap-2 px-3 py-1.5 rounded-lg text-xs transition-colors cursor-pointer ${
              currentView === "customize" ? "bg-zinc-200/70 text-zinc-900 font-medium" : "text-zinc-700 hover:bg-zinc-100 font-normal"
            }`}
          >
            <Blocks className="w-3.5 h-3.5 text-zinc-500" />
            <span>Customize</span>
          </button>

          {/* Indented sub-items when Customize is open matching Image #1 */}
          {currentView === "customize" && (
            <div className="space-y-0.5 pl-5">
              <button
                type="button"
                className="w-full flex items-center px-3 py-1 rounded-md bg-zinc-200/80 text-xs font-medium text-zinc-900 cursor-pointer"
              >
                <span>Installed</span>
              </button>
              <button
                type="button"
                className="w-full flex items-center px-3 py-1 rounded-md hover:bg-zinc-100 text-xs font-normal text-zinc-600 transition-colors cursor-pointer"
              >
                <span>Marketplace</span>
              </button>
            </div>
          )}
        </div>

        {/* SETTINGS Group Section: rendered ONLY when on settings page */}
        {currentView === "settings" && (
          <div className="px-2 pt-2 pb-1 space-y-0.5 border-t border-zinc-200/50">
            <div className="px-3 py-1 text-[11px] font-medium text-zinc-400 uppercase tracking-wider">
              Settings
            </div>

            <button
              type="button"
              data-testid="sidebar-settings-general"
              onClick={() => onOpenSettings?.("general")}
              className={`w-full flex items-center gap-2 px-3 py-1.5 rounded-lg text-xs transition-colors cursor-pointer ${
                settingsSection === "general" ? "bg-zinc-200/70 text-zinc-900 font-medium" : "text-zinc-700 hover:bg-zinc-100 font-normal"
              }`}
            >
              <Sliders className="w-3.5 h-3.5 text-zinc-500" />
              <span>General</span>
            </button>

            <button
              type="button"
              data-testid="sidebar-settings-api"
              onClick={() => onOpenSettings?.("api")}
              className={`w-full flex items-center gap-2 px-3 py-1.5 rounded-lg text-xs transition-colors cursor-pointer ${
                settingsSection === "api" ? "bg-zinc-200/70 text-zinc-900 font-medium" : "text-zinc-700 hover:bg-zinc-100 font-normal"
              }`}
            >
              <Cable className="w-3.5 h-3.5 text-zinc-500" />
              <span>API Providers</span>
            </button>

            <button
              type="button"
              data-testid="sidebar-settings-account"
              onClick={() => onOpenSettings?.("account")}
              className={`w-full flex items-center gap-2 px-3 py-1.5 rounded-lg text-xs transition-colors cursor-pointer ${
                settingsSection === "account" ? "bg-zinc-200/70 text-zinc-900 font-medium" : "text-zinc-700 hover:bg-zinc-100 font-normal"
              }`}
            >
              <CircleUser className="w-3.5 h-3.5 text-zinc-500" />
              <span>Account</span>
            </button>
          </div>
        )}

        {/* Sessions Section Header & List: ONLY shown when NOT on settings page */}
        {currentView !== "settings" && (
          <>
            <div className="px-3 pt-3 pb-1 text-[11px] font-semibold text-zinc-400 uppercase tracking-wider">
              Projects
            </div>

            {/* Session List: grouped by Project or flat by Time */}
            <div className="flex-1 overflow-y-auto px-2 space-y-0.5">
              {sortMode === "project" ? (
                projectGroups.map((group) => {
                  const isCollapsed = collapsedProjects.has(group.label);
                  return (
                    <div key={group.label} className="mb-2 min-w-0">
                      <button
                        type="button"
                        onClick={() => toggleProjectCollapse(group.label)}
                        className="flex h-7 w-full min-w-0 items-center gap-1.5 rounded-md px-1.5 text-left text-xs font-semibold text-zinc-600 hover:bg-zinc-100 transition-colors cursor-pointer"
                        title={group.label}
                      >
                        <ChevronDown
                          className={`size-3.5 shrink-0 transition-transform ${
                            isCollapsed ? "-rotate-90 text-zinc-400" : "text-zinc-500"
                          }`}
                        />
                        <span className="truncate">{group.label}</span>
                        <span className="text-[11px] text-zinc-400 font-normal ml-auto tabular-nums">
                          {group.sessions.length}
                        </span>
                      </button>

                      {!isCollapsed && (
                        <div className="pl-3 space-y-0.5 mt-0.5">
                          {group.sessions.map((session) => renderSessionRow(session))}
                        </div>
                      )}
                    </div>
                  );
                })
              ) : (
                displayedSessions.map((session) => renderSessionRow(session))
              )}
            </div>
          </>
        )}
      </div>

      {/* Bottom Row: Settings Button matching Cline Image #1 & #2 */}
      <div className="p-2 border-t border-zinc-200/60">
        <button
          type="button"
          data-testid="sidebar-settings"
          onClick={() => onOpenSettings?.()}
          className="w-full flex items-center gap-2 px-3 py-1.5 rounded-lg hover:bg-zinc-100 text-xs font-normal text-zinc-700 transition-colors cursor-pointer"
        >
          <Settings className="w-3.5 h-3.5 text-zinc-500" />
          <span>Settings</span>
        </button>
      </div>
    </aside>
  );
}

export default Sidebar;
