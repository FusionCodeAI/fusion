import React, { useState, useMemo } from "react";
import {
  ArrowLeft,
  ArrowRight,
  Search,
  Plus,
  Clock,
  LayoutGrid,
  ArrowUpDown,
  Filter,
  Settings,
  X,
} from "lucide-react";
import { FusionLogo } from "./FusionLogo";

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
  onSchedule?: () => void;
  onCustomize?: () => void;
  onOpenSettings?: () => void;
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

export function Sidebar({
  sessions,
  activeSessionId,
  onNewChat,
  onSelectSession,
  onDeleteSession,
  onHistoryBack,
  onHistoryForward,
  onSchedule,
  onCustomize,
  onOpenSettings,
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

  return (
    <aside
      data-testid="sidebar"
      className={`w-64 h-full shrink-0 flex flex-col justify-between border-r border-zinc-200/80 bg-[#fbfbfb] select-none text-zinc-800 ${className}`}
    >
      {/* Top Section */}
      <div className="flex flex-col min-h-0">
        {/* Top Header Row with Traffic Lights clearance */}
        <div
          data-tauri-drag-region
          className="h-10 pl-[78px] pr-3 flex items-center justify-between border-b border-zinc-200/60"
        >
          {/* Fusion Brand Logo */}
          <div className="flex items-center">
            <FusionLogo className="w-4 h-4 text-[#5100cd]" fill="#5100cd" />
          </div>

          {/* Navigation Controls: Back, Forward, Search */}
          <div className="flex items-center gap-1 text-zinc-500">
            <button
              type="button"
              onClick={onHistoryBack}
              className="p-1 rounded hover:text-zinc-900 hover:bg-zinc-200/60 transition-colors cursor-pointer"
              title="Go back"
              aria-label="Go back"
            >
              <ArrowLeft className="w-3.5 h-3.5" />
            </button>
            <button
              type="button"
              onClick={onHistoryForward}
              className="p-1 rounded hover:text-zinc-900 hover:bg-zinc-200/60 transition-colors cursor-pointer"
              title="Go forward"
              aria-label="Go forward"
            >
              <ArrowRight className="w-3.5 h-3.5" />
            </button>
            <button
              type="button"
              onClick={() => setIsSearchOpen((prev) => !prev)}
              className="p-1 rounded hover:text-zinc-900 hover:bg-zinc-200/60 transition-colors cursor-pointer"
              title="Search sessions"
              aria-label="Search sessions"
            >
              <Search className="w-3.5 h-3.5" />
            </button>
          </div>
        </div>

        {/* Search Bar Input (toggled) */}
        {isSearchOpen && (
          <div className="px-3 pt-2">
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

        {/* Primary Action Buttons: + Session, Schedule, Customize */}
        <div className="p-2 space-y-1">
          {/* + Session */}
          <button
            type="button"
            data-testid="sidebar-new-chat"
            onClick={onNewChat}
            className="w-full flex items-center gap-2 px-3 py-1.5 rounded-lg bg-zinc-200/70 hover:bg-zinc-200 text-xs font-medium text-zinc-900 transition-colors cursor-pointer"
          >
            <Plus className="w-3.5 h-3.5 text-zinc-700" />
            <span>+ Session</span>
          </button>

          {/* Schedule */}
          <button
            type="button"
            onClick={onSchedule}
            className="w-full flex items-center gap-2 px-3 py-1.5 rounded-lg hover:bg-zinc-100 text-xs font-normal text-zinc-700 transition-colors cursor-pointer"
          >
            <Clock className="w-3.5 h-3.5 text-zinc-500" />
            <span>Schedule</span>
          </button>

          {/* Customize */}
          <button
            type="button"
            onClick={onCustomize}
            className="w-full flex items-center gap-2 px-3 py-1.5 rounded-lg hover:bg-zinc-100 text-xs font-normal text-zinc-700 transition-colors cursor-pointer"
          >
            <LayoutGrid className="w-3.5 h-3.5 text-zinc-500" />
            <span>Customize</span>
          </button>
        </div>

        {/* Sessions Section Header */}
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
            const isActive = session.id === activeSessionId;
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
                      data-testid={`delete-session-${session.id}`}
                      onClick={(e) => {
                        e.stopPropagation();
                        onDeleteSession(session.id);
                      }}
                      className="opacity-0 group-hover:opacity-100 hover:text-zinc-800 p-0.5 rounded transition-opacity"
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
      </div>

      {/* Sidebar Bottom Footer: Settings */}
      <div className="p-2 border-t border-zinc-200/60">
        <button
          type="button"
          data-testid="sidebar-settings"
          onClick={onOpenSettings}
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
