import React, { useState, useMemo } from "react";
import {
  PanelLeft,
  ArrowLeft,
  ArrowRight,
  SquarePen,
  Search,
  Plus,
  CircleDashed,
  Home,
  Circle,
  GitBranch,
  Settings,
  X,
} from "lucide-react";

export interface SidebarSessionItem {
  id: string;
  title: string;
  createdAt?: number;
  updatedAt?: number;
}

export interface SidebarProps {
  sessions?: readonly SidebarSessionItem[];
  activeSessionId?: string | null;
  workspaceDir?: string;
  userName?: string;
  onNewChat?: () => void;
  onSelectSession?: (id: string) => void;
  onDeleteSession?: (id: string) => void;
  onToggleSidebar?: () => void;
  onHistoryBack?: () => void;
  onHistoryForward?: () => void;
  onNewProject?: () => void;
  onConnectGithub?: () => void;
  onOpenSettings?: () => void;
  className?: string;
}

export function formatRelativeTime(timestamp?: number, now: number = Date.now()): string {
  if (!timestamp || isNaN(timestamp) || timestamp <= 0) return "2h";
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
  const parts = workspaceDir.replace(/[\\/]+$/, "").split(/[\\/]/);
  return parts[parts.length - 1] || "No Repo";
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
  onHistoryBack,
  onHistoryForward,
  onNewProject,
  onConnectGithub,
  onOpenSettings,
  className = "",
}: SidebarProps) {
  const [searchQuery, setSearchQuery] = useState("");

  // Default fallback session if none are passed or empty
  const effectiveSessions = useMemo(() => {
    if (sessions && sessions.length > 0) {
      return sessions;
    }
    return [
      {
        id: "default-general-chat",
        title: "General chat conversation",
        createdAt: Date.now() - 2 * 3600 * 1000,
        updatedAt: Date.now() - 2 * 3600 * 1000,
      },
    ];
  }, [sessions]);

  const filteredSessions = useMemo(() => {
    const trimmed = searchQuery.trim().toLowerCase();
    if (!trimmed) return effectiveSessions;
    return effectiveSessions.filter((s) => s.title.toLowerCase().includes(trimmed));
  }, [effectiveSessions, searchQuery]);

  const effectiveActiveId = activeSessionId ?? effectiveSessions[0]?.id ?? null;

  return (
    <aside
      data-testid="sidebar"
      className={`w-[220px] h-full bg-[#f7f7f8] border-r border-zinc-200/80 flex flex-col justify-between shrink-0 select-none overflow-hidden ${className}`}
    >
      {/* Top Container: Header & Navigation */}
      <div className="flex flex-col flex-1 min-h-0 overflow-hidden">
        {/* Top row: height 40px (h-10) aligned with TopHeader, pl-[78px] macOS traffic clearance */}
        <div
          data-testid="sidebar-top-row"
          data-tauri-drag-region
          className="h-10 pl-[78px] pr-3 flex items-center justify-between border-b border-zinc-200/80 shrink-0"
        >
          {/* Left: Sidebar toggle [|] */}
          <button
            type="button"
            data-testid="sidebar-toggle-button"
            onClick={onToggleSidebar}
            className="p-1 rounded text-zinc-600 hover:text-zinc-900 hover:bg-zinc-200/60 transition-colors cursor-pointer"
            aria-label="Toggle sidebar"
            title="Toggle sidebar (Cmd+B)"
          >
            <PanelLeft className="w-4 h-4" />
          </button>

          {/* Right: History navigation arrows */}
          <div className="flex items-center gap-1">
            <button
              type="button"
              data-testid="sidebar-history-back"
              onClick={onHistoryBack}
              className="p-1 rounded text-zinc-400 hover:text-zinc-700 hover:bg-zinc-200/60 transition-colors cursor-pointer"
              aria-label="Go back"
            >
              <ArrowLeft className="w-3.5 h-3.5" />
            </button>
            <button
              type="button"
              data-testid="sidebar-history-forward"
              onClick={onHistoryForward}
              className="p-1 rounded text-zinc-400 hover:text-zinc-700 hover:bg-zinc-200/60 transition-colors cursor-pointer"
              aria-label="Go forward"
            >
              <ArrowRight className="w-3.5 h-3.5" />
            </button>
          </div>
        </div>

        {/* Menu items list */}
        <div className="flex-1 overflow-y-auto px-2 py-2 flex flex-col gap-1 min-h-0">
          {/* New Chat */}
          <button
            type="button"
            data-testid="sidebar-new-chat"
            onClick={onNewChat}
            className="w-full flex items-center gap-2 px-2 py-1.5 rounded-lg text-xs text-zinc-800 hover:bg-zinc-200/60 transition-colors cursor-pointer font-medium text-left"
          >
            <SquarePen className="w-3.5 h-3.5 text-zinc-600 shrink-0" />
            <span>New Chat</span>
          </button>

          {/* Search (embedded search input box) */}
          <div className="relative flex items-center w-full px-2 py-1 rounded-lg bg-white border border-zinc-200/90 shadow-2xs focus-within:border-zinc-300 transition-colors">
            <Search className="w-3.5 h-3.5 text-zinc-400 shrink-0 mr-1.5" />
            <input
              type="text"
              data-testid="sidebar-search-input"
              value={searchQuery}
              onChange={(e) => setSearchQuery(e.target.value)}
              placeholder="Search"
              className="w-full text-xs text-zinc-800 placeholder-zinc-400 bg-transparent border-none outline-none p-0"
            />
            {searchQuery && (
              <button
                type="button"
                data-testid="sidebar-search-clear"
                onClick={() => setSearchQuery("")}
                className="text-zinc-400 hover:text-zinc-600 cursor-pointer ml-1"
                aria-label="Clear search"
              >
                <X className="w-3 h-3" />
              </button>
            )}
          </div>

          {/* Section: Projects */}
          <div className="flex items-center justify-between px-2 pt-3 pb-0.5">
            <span className="text-[11px] font-medium text-zinc-500 tracking-wider">
              Projects
            </span>
            <button
              type="button"
              data-testid="sidebar-new-project-button"
              onClick={onNewProject}
              className="p-0.5 rounded text-zinc-400 hover:text-zinc-700 hover:bg-zinc-200/60 transition-colors cursor-pointer"
              aria-label="Add project"
            >
              <Plus className="w-3 h-3" />
            </button>
          </div>

          {/* Projects item: ◌ New Project */}
          <button
            type="button"
            data-testid="sidebar-new-project-item"
            onClick={onNewProject}
            className="w-full flex items-center gap-2 px-2 py-1 rounded-lg text-xs text-zinc-600 hover:bg-zinc-200/60 hover:text-zinc-900 transition-colors cursor-pointer text-left"
          >
            <CircleDashed className="w-3.5 h-3.5 text-zinc-400 shrink-0" />
            <span>New Project</span>
          </button>

          {/* Section: Repositories */}
          <div className="flex items-center justify-between px-2 pt-2.5 pb-0.5">
            <span className="text-[11px] font-medium text-zinc-500 tracking-wider">
              Repositories
            </span>
          </div>

          {/* Repositories item: No Repo (home icon) */}
          <div
            data-testid="sidebar-repo-item"
            className="w-full flex items-center gap-2 px-2 py-1 rounded-lg text-xs text-zinc-700 hover:bg-zinc-200/60 transition-colors cursor-pointer text-left"
          >
            <Home className="w-3.5 h-3.5 text-zinc-500 shrink-0" />
            <span className="truncate">{getWorkspaceFolderName(workspaceDir)}</span>
          </div>

          {/* Active session item list (• General chat conversation 2h) */}
          <div data-testid="sidebar-sessions-list" className="flex flex-col gap-0.5 mt-0.5">
            {filteredSessions.map((session) => {
              const isActive = session.id === effectiveActiveId;
              const timeDisplay = formatRelativeTime(session.updatedAt || session.createdAt);

              return (
                <div
                  key={session.id}
                  data-testid={`sidebar-session-${session.id}`}
                  onClick={() => onSelectSession?.(session.id)}
                  className={`flex items-center justify-between px-2 py-1 rounded-lg text-xs transition-colors cursor-pointer group ${
                    isActive
                      ? "bg-zinc-200/70 text-zinc-900 font-medium"
                      : "text-zinc-600 hover:bg-zinc-200/50 hover:text-zinc-900"
                  }`}
                >
                  <div className="flex items-center gap-1.5 truncate min-w-0 pr-1">
                    <span
                      className={`text-[10px] leading-none shrink-0 ${
                        isActive ? "text-zinc-900" : "text-zinc-400"
                      }`}
                    >
                      •
                    </span>
                    <span className="truncate">{session.title}</span>
                  </div>

                  <div className="flex items-center gap-1 shrink-0">
                    <span className="text-[11px] text-zinc-400 tabular-nums">
                      {timeDisplay}
                    </span>
                    {onDeleteSession && (
                      <button
                        type="button"
                        onClick={(e) => {
                          e.stopPropagation();
                          onDeleteSession(session.id);
                        }}
                        className="opacity-0 group-hover:opacity-100 p-0.5 rounded hover:bg-zinc-300/50 text-zinc-400 hover:text-zinc-600 cursor-pointer transition-opacity"
                        aria-label="Delete session"
                      >
                        <X className="w-2.5 h-2.5" />
                      </button>
                    )}
                  </div>
                </div>
              );
            })}
          </div>
        </div>
      </div>

      {/* Bottom Footer Section */}
      <div className="p-2 border-t border-zinc-200/80 flex flex-col gap-1.5 shrink-0 bg-[#f7f7f8]">
        {/* Getting Started 1/3 ⚪ */}
        <div
          data-testid="sidebar-getting-started"
          className="flex items-center justify-between px-2 py-1 text-xs text-zinc-600"
        >
          <div className="flex items-center gap-1.5">
            <span>Getting Started</span>
            <span className="text-[11px] font-medium text-zinc-500">1/3</span>
          </div>
          <Circle className="w-3 h-3 text-zinc-400 shrink-0" />
        </div>

        {/* Connect GitHub button */}
        <button
          type="button"
          data-testid="sidebar-connect-github"
          onClick={onConnectGithub}
          className="w-full flex items-center justify-center gap-1.5 py-1.5 px-2 rounded-lg bg-white hover:bg-zinc-50 border border-zinc-200/90 text-xs font-medium text-zinc-700 shadow-2xs transition-colors cursor-pointer"
        >
          <GitBranch className="w-3.5 h-3.5 text-zinc-800 shrink-0" />
          <span>Connect GitHub</span>
        </button>

        {/* User profile: (A) Aung Myat Moe ⚙ */}
        <div
          data-testid="sidebar-user-profile"
          className="flex items-center justify-between px-2 py-1 text-xs text-zinc-700"
        >
          <div className="flex items-center gap-2 min-w-0">
            <div className="w-5 h-5 rounded-full bg-zinc-200 text-zinc-700 flex items-center justify-center text-[10px] font-semibold shrink-0">
              {userName.charAt(0).toUpperCase()}
            </div>
            <span className="font-medium text-zinc-800 truncate">{userName}</span>
          </div>

          <button
            type="button"
            data-testid="sidebar-settings-button"
            onClick={onOpenSettings}
            className="p-1 rounded text-zinc-400 hover:text-zinc-600 hover:bg-zinc-200/60 transition-colors cursor-pointer shrink-0"
            aria-label="Settings"
          >
            <Settings className="w-3.5 h-3.5" />
          </button>
        </div>
      </div>
    </aside>
  );
}

export default Sidebar;
