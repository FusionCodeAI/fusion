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
  ArrowUpDown,
  Filter,
  Settings,
  X,
} from "lucide-react";
import { ClineAvatar } from "./ClineAvatar";

export interface SidebarSessionItem {
  id: string;
  title: string;
  createdAt?: number;
  updatedAt?: number;
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
  onCustomize?: () => void;
  onOpenSettings?: (section?: SettingsSectionId) => void;
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
  const parts = workspaceDir.replace(/[\\/]+$/, "").split(/[\\/]/);
  return parts[parts.length - 1] || "No Repo";
}

export function Sidebar({
  sessions,
  activeSessionId,
  currentView = "chat",
  settingsSection = "general",
  canNavigateBack = false,
  canNavigateForward = false,
  onNewChat,
  onSelectSession,
  onDeleteSession,
  onHistoryBack,
  onHistoryForward,
  onCustomize,
  onOpenSettings,
  width,
  onResize,
  onResetWidth,
  className = "",
}: SidebarProps) {
  const [searchQuery, setSearchQuery] = useState("");
  const [isSearchOpen, setIsSearchOpen] = useState(false);

  const defaultSessions: SidebarSessionItem[] = useMemo(
    () => [
      {
        id: "session-default",
        title: "hi",
        createdAt: Date.now() - 3600 * 1000,
        updatedAt: Date.now() - 3600 * 1000,
      },
    ],
    []
  );

  const sessionList = sessions && sessions.length > 0 ? sessions : defaultSessions;

  const filteredSessions = useMemo(() => {
    if (!searchQuery.trim()) return sessionList;
    const q = searchQuery.toLowerCase();
    return sessionList.filter((s) => s.title.toLowerCase().includes(q));
  }, [sessionList, searchQuery]);

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
            <ClineAvatar className="w-5 h-5 cursor-pointer" />
          </button>

          <button
            type="button"
            data-testid="sidebar-search-btn"
            onClick={() => setIsSearchOpen((prev) => !prev)}
            className="size-8 rounded-md flex items-center justify-center text-zinc-500 hover:text-zinc-900 hover:bg-zinc-200/60 cursor-pointer"
            title="Search sessions (Cmd/Ctrl+P)"
            aria-label="Search sessions"
          >
            <Search className="w-4 h-4" />
          </button>
        </div>

        {/* Search Bar Input (toggled) */}
        {isSearchOpen && (
          <div className="px-3 pb-2">
            <div className="flex items-center gap-1.5 px-2 py-1 bg-white border border-zinc-200 rounded-md shadow-2xs">
              <Search className="w-3.5 h-3.5 text-zinc-400 shrink-0" />
              <input
                type="text"
                placeholder="Search..."
                value={searchQuery}
                onChange={(e) => setSearchQuery(e.target.value)}
                className="w-full bg-transparent text-xs text-zinc-800 placeholder-zinc-400 outline-none"
                autoFocus
              />
              {searchQuery && (
                <button
                  type="button"
                  onClick={() => setSearchQuery("")}
                  className="p-0.5 text-zinc-400 hover:text-zinc-600 cursor-pointer"
                >
                  <X className="w-3 h-3" />
                </button>
              )}
            </div>
          </div>
        )}

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

        {/* SETTINGS Group Section: rendered ONLY when on settings page matching Image #2 */}
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

        {/* Sessions Section: rendered ONLY when NOT on settings page */}
        {currentView !== "settings" && (
          <>
            <div className="px-3 pt-3 pb-1 flex items-center justify-between text-xs text-zinc-500">
              <span className="font-medium text-zinc-600">Sessions</span>
              <div className="flex items-center gap-1">
                <button
                  type="button"
                  className="p-1 rounded hover:bg-zinc-200/50 hover:text-zinc-800 transition-colors cursor-pointer"
                  title="Sort sessions"
                  aria-label="Sort sessions"
                >
                  <ArrowUpDown className="w-3 h-3" />
                </button>
                <button
                  type="button"
                  className="p-1 rounded hover:bg-zinc-200/50 hover:text-zinc-800 transition-colors cursor-pointer"
                  title="Filter sessions"
                  aria-label="Filter sessions"
                >
                  <Filter className="w-3 h-3" />
                </button>
              </div>
            </div>

            {/* Session List */}
            <div className="flex-1 overflow-y-auto px-2 space-y-0.5">
              {filteredSessions.map((session) => {
                const isActive = session.id === activeSessionId && currentView === "chat";
                return (
                  <div
                    key={session.id}
                    data-testid="sidebar-session-item"
                    data-session-id={session.id}
                    onClick={() => onSelectSession?.(session.id)}
                    className={`group flex items-center justify-between px-3 py-1.5 rounded-lg text-xs cursor-pointer transition-colors ${
                      isActive
                        ? "bg-zinc-200/60 font-medium text-zinc-900"
                        : "hover:bg-zinc-100 text-zinc-700 font-normal"
                    }`}
                  >
                    <span className="truncate pr-2">{session.title}</span>
                    <div className="flex items-center gap-1 shrink-0 text-zinc-400">
                      <span className="text-[11px] tabular-nums">
                        {formatRelativeTime(session.updatedAt || session.createdAt)}
                      </span>
                      {onDeleteSession && (
                        <button
                          type="button"
                          onClick={(e) => {
                            e.stopPropagation();
                            onDeleteSession(session.id);
                          }}
                          className="opacity-0 group-hover:opacity-100 p-0.5 rounded hover:bg-zinc-200 text-zinc-400 hover:text-zinc-700 transition-opacity"
                          title="Delete session"
                          aria-label="Delete session"
                        >
                          <X className="w-3 h-3" />
                        </button>
                      )}
                    </div>
                  </div>
                );
              })}
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
