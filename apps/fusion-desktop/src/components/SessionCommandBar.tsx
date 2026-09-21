import React, { useState, useEffect, useRef, useMemo } from "react";
import { Search, Loader2, MessageSquare, ArrowRight, CornerDownLeft } from "lucide-react";
import { type SidebarSessionItem } from "./Sidebar";
import { type ChatSessionRecord } from "../state/session-storage";

export interface SessionCommandBarProps {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  sessions?: readonly SidebarSessionItem[] | ChatSessionRecord[];
  onOpenSession: (sessionId: string) => void;
}

interface SearchResultItem {
  sessionId: string;
  title: string;
  role: string;
  snippet: string;
  updatedAt?: number;
}

export function SessionCommandBar({
  open,
  onOpenChange,
  sessions = [],
  onOpenSession,
}: SessionCommandBarProps) {
  const [query, setQuery] = useState("");
  const [selectedIndex, setSelectedIndex] = useState(0);
  const inputRef = useRef<HTMLInputElement>(null);

  useEffect(() => {
    if (open) {
      setQuery("");
      setSelectedIndex(0);
      setTimeout(() => {
        inputRef.current?.focus();
      }, 50);
    }
  }, [open]);

  // Search across session titles and message contents
  const results: SearchResultItem[] = useMemo(() => {
    const q = query.trim().toLowerCase();
    if (!q) return [];

    const hits: SearchResultItem[] = [];

    for (const s of sessions) {
      const titleMatch = s.title.toLowerCase().includes(q);
      const sessionRecord = s as ChatSessionRecord;

      let matchedSnippet = "";
      let matchedRole = "session";

      if (sessionRecord.messages && Array.isArray(sessionRecord.messages)) {
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

      if (titleMatch || matchedSnippet) {
        hits.push({
          sessionId: s.id,
          title: s.title,
          role: matchedRole,
          snippet: matchedSnippet || `Conversation in session "${s.title}"`,
          updatedAt: s.updatedAt || s.createdAt,
        });
      }
    }

    return hits.slice(0, 30);
  }, [query, sessions]);

  // Keyboard navigation
  useEffect(() => {
    if (!open) return;

    const handleKeyDown = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        e.preventDefault();
        onOpenChange(false);
      } else if (e.key === "ArrowDown") {
        e.preventDefault();
        setSelectedIndex((prev) => (results.length > 0 ? (prev + 1) % results.length : 0));
      } else if (e.key === "ArrowUp") {
        e.preventDefault();
        setSelectedIndex((prev) => (results.length > 0 ? (prev - 1 + results.length) % results.length : 0));
      } else if (e.key === "Enter") {
        e.preventDefault();
        if (results.length > 0 && results[selectedIndex]) {
          onOpenSession(results[selectedIndex].sessionId);
          onOpenChange(false);
        }
      }
    };

    window.addEventListener("keydown", handleKeyDown);
    return () => window.removeEventListener("keydown", handleKeyDown);
  }, [open, results, selectedIndex, onOpenChange, onOpenSession]);

  if (!open) return null;

  return (
    <div
      data-testid="session-command-bar-overlay"
      className="fixed inset-0 z-50 bg-black/40 backdrop-blur-xs flex items-start justify-center pt-24 p-4 animate-in fade-in duration-100"
      onClick={(e) => {
        if (e.target === e.currentTarget) onOpenChange(false);
      }}
    >
      <div
        data-testid="session-command-bar"
        className="w-full max-w-2xl bg-white rounded-2xl border border-zinc-200/90 shadow-2xl overflow-hidden flex flex-col max-h-[36rem] animate-in zoom-in-95 duration-100"
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
            placeholder="Search all session history..."
            className="w-full py-3.5 text-sm text-zinc-900 bg-transparent outline-none placeholder-zinc-400"
          />
        </div>

        {/* Results List matching Cline CommandList */}
        <div className="flex-1 overflow-y-auto min-h-[14rem] max-h-[26rem] p-2">
          {!query.trim() ? (
            <div className="flex flex-col items-center justify-center py-12 text-center text-zinc-400 space-y-1">
              <MessageSquare className="w-8 h-8 stroke-1 text-zinc-300 mb-1" />
              <p className="text-xs font-medium text-zinc-600">Search session history</p>
              <p className="text-[11px] text-zinc-400 max-w-xs">
                Search messages, commands, errors, and files across all conversations.
              </p>
            </div>
          ) : results.length === 0 ? (
            <div className="py-12 text-center text-xs text-zinc-400">
              No matching session history for "{query}".
            </div>
          ) : (
            <div className="space-y-1" data-testid="command-bar-results">
              <div className="px-2 py-1 text-[11px] font-semibold text-zinc-400 uppercase tracking-wider">
                Session history ({results.length})
              </div>
              {results.map((hit, idx) => {
                const isSelected = idx === selectedIndex;
                return (
                  <div
                    key={`${hit.sessionId}-${idx}`}
                    data-testid="command-bar-item"
                    onClick={() => {
                      onOpenSession(hit.sessionId);
                      onOpenChange(false);
                    }}
                    onMouseEnter={() => setSelectedIndex(idx)}
                    className={`px-3 py-2.5 rounded-xl cursor-pointer transition-colors flex items-start justify-between gap-3 ${
                      isSelected ? "bg-[#5100cd]/8 text-zinc-900" : "hover:bg-zinc-50 text-zinc-700"
                    }`}
                  >
                    <div className="min-w-0 flex-1">
                      <div className="flex items-center gap-2">
                        <span className="text-[10px] font-medium uppercase px-1.5 py-0.2 rounded bg-zinc-100 text-zinc-600">
                          {hit.role}
                        </span>
                        <span className="text-xs font-semibold text-zinc-900 truncate">
                          {hit.title}
                        </span>
                      </div>
                      <p className="text-xs text-zinc-500 mt-1 line-clamp-2 leading-relaxed">
                        {hit.snippet}
                      </p>
                    </div>

                    {isSelected && (
                      <div className="shrink-0 mt-1 text-[#5100cd] flex items-center gap-1 text-[11px] font-medium">
                        <span>Open</span>
                        <CornerDownLeft className="w-3.5 h-3.5" />
                      </div>
                    )}
                  </div>
                );
              })}
            </div>
          )}
        </div>

        {/* Footer matching Cline CommandBar */}
        <div className="flex items-center justify-between border-t border-zinc-100 bg-zinc-50/60 px-4 py-2 text-[11px] text-zinc-500">
          <div className="flex items-center gap-2">
            <span>Navigate with</span>
            <kbd className="rounded border border-zinc-200 bg-white px-1.5 py-0.5 font-mono text-[10px] text-zinc-600">
              ↑↓
            </kbd>
            <span>and open with</span>
            <kbd className="rounded border border-zinc-200 bg-white px-1.5 py-0.5 font-mono text-[10px] text-zinc-600">
              ↵
            </kbd>
          </div>
          <div className="flex items-center gap-1.5">
            <kbd className="rounded border border-zinc-200 bg-white px-1.5 py-0.5 font-mono text-[10px] text-zinc-600 shadow-2xs">
              Cmd+K
            </kbd>
            <span>or</span>
            <kbd className="rounded border border-zinc-200 bg-white px-1.5 py-0.5 font-mono text-[10px] text-zinc-600 shadow-2xs">
              Cmd+P
            </kbd>
          </div>
        </div>
      </div>
    </div>
  );
}

export default SessionCommandBar;
