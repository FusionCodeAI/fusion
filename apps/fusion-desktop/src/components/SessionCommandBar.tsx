import React, { useState, useEffect, useRef, useMemo } from "react";
import {
  Search,
  Plus,
  Folder,
  PanelLeft,
  Compass,
  Zap,
  Settings,
  Blocks,
  MessageSquare,
  Pin,
} from "lucide-react";
import { type SidebarSessionItem } from "./Sidebar";
import { type ChatSessionRecord } from "../state/session-storage";

export interface SessionCommandBarProps {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  sessions?: readonly SidebarSessionItem[] | ChatSessionRecord[];
  onOpenSession: (sessionId: string) => void;
  onNewChat?: () => void;
  onOpenFolder?: () => void;
  onToggleSidebar?: () => void;
  onOpenSettings?: () => void;
  onOpenCustomize?: () => void;
  onSetMode?: (mode: "plan" | "act") => void;
}

interface PaletteCommand {
  id: string;
  type: "command";
  title: string;
  subtitle: string;
  category: "Action" | "Mode" | "Navigation";
  shortcut?: string;
  icon: string;
  action: () => void;
}

interface PaletteSession {
  id: string;
  type: "session";
  sessionId: string;
  title: string;
  role: string;
  snippet: string;
  isPinned?: boolean;
  updatedAt?: number;
}

type PaletteItem = PaletteCommand | PaletteSession;

function renderPaletteIcon(iconName: string) {
  switch (iconName) {
    case "Plus":
      return <Plus className="w-4 h-4 text-zinc-600" />;
    case "Folder":
      return <Folder className="w-4 h-4 text-zinc-600" />;
    case "PanelLeft":
      return <PanelLeft className="w-4 h-4 text-zinc-600" />;
    case "Compass":
      return <Compass className="w-4 h-4 text-zinc-600" />;
    case "Zap":
      return <Zap className="w-4 h-4 text-zinc-600" />;
    case "Settings":
      return <Settings className="w-4 h-4 text-zinc-600" />;
    case "Blocks":
      return <Blocks className="w-4 h-4 text-zinc-600" />;
    default:
      return <MessageSquare className="w-4 h-4 text-zinc-400" />;
  }
}

export function SessionCommandBar({
  open,
  onOpenChange,
  sessions = [],
  onOpenSession,
  onNewChat,
  onOpenFolder,
  onToggleSidebar,
  onOpenSettings,
  onOpenCustomize,
  onSetMode,
}: SessionCommandBarProps) {
  const [query, setQuery] = useState("");
  const [selectedIndex, setSelectedIndex] = useState(0);
  const inputRef = useRef<HTMLInputElement>(null);
  const listRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    if (open) {
      setQuery("");
      setSelectedIndex(0);
      setTimeout(() => {
        inputRef.current?.focus();
      }, 50);
    }
  }, [open]);

  // Static Action Commands
  const commands: PaletteCommand[] = useMemo(() => {
    const list: PaletteCommand[] = [
      {
        id: "cmd-new-chat",
        type: "command",
        title: "New Session",
        subtitle: "Start a fresh conversation",
        category: "Action",
        shortcut: "Cmd+N",
        icon: "Plus",
        action: () => {
          onNewChat?.();
          onOpenChange(false);
        },
      },
      {
        id: "cmd-open-folder",
        type: "command",
        title: "Open Project Folder...",
        subtitle: "Switch workspace directory",
        category: "Action",
        shortcut: "Cmd+O",
        icon: "Folder",
        action: () => {
          onOpenFolder?.();
          onOpenChange(false);
        },
      },
      {
        id: "cmd-toggle-sidebar",
        type: "command",
        title: "Toggle Sidebar",
        subtitle: "Expand or collapse the sidebar",
        category: "Navigation",
        shortcut: "Cmd+B",
        icon: "PanelLeft",
        action: () => {
          onToggleSidebar?.();
          onOpenChange(false);
        },
      },
      {
        id: "cmd-plan-mode",
        type: "command",
        title: "Switch to Plan Mode",
        subtitle: "Architect mode (designs solution without code edits)",
        category: "Mode",
        icon: "Compass",
        action: () => {
          onSetMode?.("plan");
          onOpenChange(false);
        },
      },
      {
        id: "cmd-act-mode",
        type: "command",
        title: "Switch to Act Mode",
        subtitle: "Execution mode (direct implementation & tool execution)",
        category: "Mode",
        icon: "Zap",
        action: () => {
          onSetMode?.("act");
          onOpenChange(false);
        },
      },
      {
        id: "cmd-settings",
        type: "command",
        title: "Open Settings",
        subtitle: "Manage models, API keys, and preferences",
        category: "Navigation",
        shortcut: "Cmd+,",
        icon: "Settings",
        action: () => {
          onOpenSettings?.();
          onOpenChange(false);
        },
      },
      {
        id: "cmd-customize",
        type: "command",
        title: "Customize & Skills",
        subtitle: "Browse skills and agent marketplace",
        category: "Navigation",
        icon: "Blocks",
        action: () => {
          onOpenCustomize?.();
          onOpenChange(false);
        },
      },
    ];
    return list;
  }, [onNewChat, onOpenFolder, onToggleSidebar, onSetMode, onOpenSettings, onOpenCustomize, onOpenChange]);

  // Filter commands and search sessions
  const combinedItems: PaletteItem[] = useMemo(() => {
    const q = query.trim().toLowerCase();

    // 1. Filter commands
    const matchingCommands = commands.filter((c) => {
      if (!q) return true;
      return (
        c.title.toLowerCase().includes(q) ||
        c.subtitle.toLowerCase().includes(q) ||
        c.category.toLowerCase().includes(q)
      );
    });

    // 2. Filter / Rank sessions
    const matchingSessions: PaletteSession[] = [];
    for (const s of sessions) {
      const titleMatch = s.title.toLowerCase().includes(q);
      const sessionRecord = s as ChatSessionRecord;

      let matchedSnippet = "";
      let matchedRole = "session";

      if (q && sessionRecord.messages && Array.isArray(sessionRecord.messages)) {
        for (const msg of sessionRecord.messages) {
          const content = msg.content || "";
          const thought = msg.thought || "";
          if (content.toLowerCase().includes(q)) {
            matchedRole = msg.role;
            matchedSnippet = content;
            break;
          } else if (thought.toLowerCase().includes(q)) {
            matchedRole = "thought";
            matchedSnippet = thought;
            break;
          }
        }
      }

      if (!q || titleMatch || matchedSnippet) {
        matchingSessions.push({
          id: `session-${s.id}`,
          type: "session",
          sessionId: s.id,
          title: s.title,
          role: matchedRole,
          snippet: matchedSnippet || `Conversation in "${s.title}"`,
          isPinned: (s as SidebarSessionItem).isPinned,
          updatedAt: s.updatedAt || s.createdAt,
        });
      }
    }

    // Sort matching sessions so pinned are first, then chronological
    matchingSessions.sort((a, b) => {
      if (Boolean(a.isPinned) !== Boolean(b.isPinned)) {
        return a.isPinned ? -1 : 1;
      }
      return (b.updatedAt || 0) - (a.updatedAt || 0);
    });

    return [...matchingCommands, ...matchingSessions.slice(0, 25)];
  }, [query, commands, sessions]);

  // Keyboard navigation
  useEffect(() => {
    if (!open) return;

    const handleKeyDown = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        e.preventDefault();
        onOpenChange(false);
      } else if (e.key === "ArrowDown") {
        e.preventDefault();
        setSelectedIndex((prev) => (combinedItems.length > 0 ? (prev + 1) % combinedItems.length : 0));
      } else if (e.key === "ArrowUp") {
        e.preventDefault();
        setSelectedIndex((prev) => (combinedItems.length > 0 ? (prev - 1 + combinedItems.length) % combinedItems.length : 0));
      } else if (e.key === "Enter") {
        e.preventDefault();
        const selected = combinedItems[selectedIndex];
        if (selected) {
          if (selected.type === "command") {
            selected.action();
          } else {
            onOpenSession(selected.sessionId);
            onOpenChange(false);
          }
        }
      }
    };

    window.addEventListener("keydown", handleKeyDown);
    return () => window.removeEventListener("keydown", handleKeyDown);
  }, [open, combinedItems, selectedIndex, onOpenChange, onOpenSession]);

  if (!open) return null;

  return (
    <div
      data-testid="session-command-bar-overlay"
      className="fixed inset-0 z-50 bg-black/40 backdrop-blur-xs flex items-start justify-center pt-20 p-4 animate-in fade-in duration-100"
      onClick={(e) => {
        if (e.target === e.currentTarget) onOpenChange(false);
      }}
    >
      <div
        data-testid="session-command-bar"
        className="w-full max-w-2xl bg-white rounded-2xl border border-zinc-200 shadow-2xl overflow-hidden flex flex-col max-h-[36rem] animate-in zoom-in-95 duration-100"
      >
        {/* Search Input matching Cline CommandInput */}
        <div className="flex items-center px-4 border-b border-zinc-200/80 bg-white">
          <Search className="size-4 text-zinc-400 shrink-0 mr-3" />
          <input
            ref={inputRef}
            data-testid="command-bar-input"
            type="text"
            value={query}
            onChange={(e) => {
              setQuery(e.target.value);
              setSelectedIndex(0);
            }}
            placeholder="Type a command or search sessions..."
            className="w-full py-3.5 text-sm text-zinc-900 bg-transparent outline-none placeholder-zinc-400"
          />
        </div>

        {/* Results List */}
        <div ref={listRef} className="flex-1 overflow-y-auto min-h-[14rem] max-h-[26rem] p-2">
          {combinedItems.length === 0 ? (
            <div className="py-12 text-center text-xs text-zinc-400">
              No matching commands or session history for "{query}".
            </div>
          ) : (
            <div className="space-y-1" data-testid="command-bar-results">
              {combinedItems.map((item, idx) => {
                const isSelected = idx === selectedIndex;
                const isCommand = item.type === "command";

                return (
                  <div
                    key={item.id}
                    data-testid="command-bar-item"
                    onClick={() => {
                      if (item.type === "command") {
                        item.action();
                      } else {
                        onOpenSession(item.sessionId);
                        onOpenChange(false);
                      }
                    }}
                    onMouseEnter={() => setSelectedIndex(idx)}
                    className={`px-3 py-2 rounded-xl cursor-pointer transition-colors flex items-center justify-between gap-3 ${
                      isSelected ? "bg-zinc-100 text-zinc-900 font-medium" : "hover:bg-zinc-50 text-zinc-700"
                    }`}
                  >
                    <div className="min-w-0 flex-1 flex items-center gap-2.5">
                      <div className="p-1 rounded bg-zinc-100 shrink-0">
                        {isCommand ? (
                          renderPaletteIcon(item.icon)
                        ) : item.isPinned ? (
                          <Pin className="w-3.5 h-3.5 text-zinc-800 fill-current rotate-45" />
                        ) : (
                          <MessageSquare className="w-3.5 h-3.5 text-zinc-400" />
                        )}
                      </div>

                      <div className="min-w-0">
                        <div className="flex items-center gap-2">
                          <span className="text-xs font-semibold text-zinc-900 truncate">
                            {item.title}
                          </span>
                          {!isCommand && (
                            <span className="text-[10px] font-mono px-1.5 py-0.2 rounded bg-zinc-100 text-zinc-500">
                              {item.role}
                            </span>
                          )}
                        </div>
                        <p className="text-[11px] text-zinc-500 truncate leading-relaxed">
                          {isCommand ? item.subtitle : item.snippet}
                        </p>
                      </div>
                    </div>

                    <div className="shrink-0 flex items-center gap-2">
                      {isCommand && item.shortcut && (
                        <kbd className="rounded border border-zinc-200 bg-white px-1.5 py-0.5 font-mono text-[10px] text-zinc-500 shadow-2xs">
                          {item.shortcut}
                        </kbd>
                      )}
                      {isSelected && (
                        <div className="text-zinc-700 flex items-center gap-1 text-[11px]">
                          <span>↵</span>
                        </div>
                      )}
                    </div>
                  </div>
                );
              })}
            </div>
          )}
        </div>

        {/* Footer matching Cline CommandBar */}
        <div className="flex items-center justify-between border-t border-zinc-100 bg-zinc-50/60 px-4 py-2 text-[11px] text-zinc-500">
          <div className="flex items-center gap-2">
            <span>Navigate</span>
            <kbd className="rounded border border-zinc-200 bg-white px-1.5 py-0.5 font-mono text-[10px] text-zinc-600">
              ↑↓
            </kbd>
            <span>Select</span>
            <kbd className="rounded border border-zinc-200 bg-white px-1.5 py-0.5 font-mono text-[10px] text-zinc-600">
              ↵
            </kbd>
          </div>
          <div className="flex items-center gap-1.5">
            <span>Command Palette</span>
            <kbd className="rounded border border-zinc-200 bg-white px-1.5 py-0.5 font-mono text-[10px] text-zinc-600 shadow-2xs">
              Cmd+K
            </kbd>
          </div>
        </div>
      </div>
    </div>
  );
}

export default SessionCommandBar;
