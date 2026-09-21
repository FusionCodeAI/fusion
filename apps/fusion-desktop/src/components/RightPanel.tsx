import React, { useState, useMemo, useRef, useEffect } from "react";
import {
  FileCode,
  Folder,
  FolderTree,
  Terminal as TerminalIcon,
  X,
  Search,
  ChevronRight,
  FileText,
  Copy,
  Check,
  RotateCcw,
  Play,
  CornerDownLeft,
  Loader2,
  GitBranch,
} from "lucide-react";
import { DiffView } from "./DiffView";
import {
  executeTerminalCommand,
  getWorkspaceGitDiff,
  type WorkspaceEntry,
  type GitFileChange,
  type WorkspaceGitStatus,
} from "../lib/fusion-ipc";

export type RightPanelTab = "changes" | "files" | "terminal";

export interface TerminalEntry {
  command: string;
  stdout?: string;
  stderr?: string;
  exit_code?: number;
  isRunning?: boolean;
}

export interface RightPanelProps {
  open: boolean;
  onClose: () => void;
  width?: number;
  onResize?: (width: number) => void;
  activeTab?: RightPanelTab;
  onSelectTab?: (tab: RightPanelTab) => void;
  diffPatch?: string;
  workspaceDir?: string;
  workspaceEntries?: WorkspaceEntry[];
  externalLogs?: Array<{ command: string; output?: string; status?: "running" | "completed" | "failed" }>;
  terminalLogs?: Array<{ command: string; output?: string; status?: "running" | "completed" | "failed" }>;
  onInsertMention?: (path: string) => void;
  className?: string;
}

export function RightPanel({
  open,
  onClose,
  width = 380,
  onResize,
  activeTab: controlledTab,
  onSelectTab,
  diffPatch: initialPatch = "",
  workspaceDir,
  workspaceEntries = [],
  externalLogs = [],
  terminalLogs: propTerminalLogs,
  onInsertMention,
  className = "",
}: RightPanelProps) {
  const [internalTab, setInternalTab] = useState<RightPanelTab>("changes");
  const [fileSearch, setFileSearch] = useState("");
  const [copied, setCopied] = useState(false);

  // Live Git Status
  const [gitStatus, setGitStatus] = useState<WorkspaceGitStatus | null>(null);
  const [selectedFileDiff, setSelectedFileDiff] = useState<GitFileChange | null>(null);

  // Live Terminal State
  const [terminalHistory, setTerminalHistory] = useState<TerminalEntry[]>(() => {
    const logs = propTerminalLogs || externalLogs;
    if (logs && logs.length > 0) {
      return logs.map((l) => ({
        command: l.command,
        stdout: l.output,
        exit_code: l.status === "failed" ? 1 : 0,
        isRunning: l.status === "running",
      }));
    }
    return [
      {
        command: "pwd",
        stdout: workspaceDir || ".",
        exit_code: 0,
      },
    ];
  });
  const currentTab = controlledTab ?? internalTab;
  const setTab = onSelectTab ?? setInternalTab;
  const [terminalInput, setTerminalInput] = useState("");
  const [isTerminalExecuting, setIsTerminalExecuting] = useState(false);
  const terminalScrollRef = useRef<HTMLDivElement>(null);
  const terminalInputRef = useRef<HTMLInputElement>(null);

  // Poll / fetch live git status when RightPanel is open
  useEffect(() => {
    if (!open) return;

    let isMounted = true;
    const fetchGit = async () => {
      try {
        const res = await getWorkspaceGitDiff(workspaceDir);
        if (isMounted && res) {
          setGitStatus(res);
        }
      } catch (err) {
        console.warn("[RightPanel] git diff error:", err);
      }
    };

    fetchGit();
    const interval = setInterval(fetchGit, 3500);
    return () => {
      isMounted = false;
      clearInterval(interval);
    };
  }, [open, workspaceDir]);

  // Sync external logs into terminal history
  useEffect(() => {
    const logs = propTerminalLogs || externalLogs;
    if (logs.length === 0) return;
    setTerminalHistory((prev) => {
      const newItems: TerminalEntry[] = [];
      for (const log of logs) {
        newItems.push({
          command: log.command,
          stdout: log.output,
          exit_code: log.status === "failed" ? 1 : 0,
          isRunning: log.status === "running",
        });
      }
      if (newItems.length > 0) {
        return [...prev, ...newItems];
      }
      return prev;
    });
  }, [externalLogs, propTerminalLogs]);

  // Auto-scroll terminal
  useEffect(() => {
    if (currentTab === "terminal" && terminalScrollRef.current) {
      terminalScrollRef.current.scrollTop = terminalScrollRef.current.scrollHeight;
    }
  }, [terminalHistory, currentTab]);

  // Resize drag handler
  const handleMouseDown = (e: React.MouseEvent) => {
    e.preventDefault();
    const startX = e.clientX;
    const initialWidth = width;

    const onMouseMove = (moveEvent: MouseEvent) => {
      const delta = startX - moveEvent.clientX;
      const newWidth = Math.min(Math.max(initialWidth + delta, 280), 680);
      onResize?.(newWidth);
    };

    const onMouseUp = () => {
      document.removeEventListener("mousemove", onMouseMove);
      document.removeEventListener("mouseup", onMouseUp);
    };

    document.addEventListener("mousemove", onMouseMove);
    document.addEventListener("mouseup", onMouseUp);
  };

  const handleCopyLogs = () => {
    const text = terminalHistory
      .map((l) => `$ ${l.command}\n${l.stdout || ""}${l.stderr ? `\n${l.stderr}` : ""}`)
      .join("\n\n");
    navigator.clipboard.writeText(text);
    setCopied(true);
    setTimeout(() => setCopied(false), 1500);
  };

  // Execute terminal command
  const handleTerminalSubmit = async (e: React.FormEvent) => {
    e.preventDefault();
    const cmd = terminalInput.trim();
    if (!cmd || isTerminalExecuting) return;

    setTerminalInput("");
    setIsTerminalExecuting(true);

    const pendingEntry: TerminalEntry = {
      command: cmd,
      isRunning: true,
    };
    setTerminalHistory((prev) => [...prev, pendingEntry]);

    try {
      const result = await executeTerminalCommand(cmd, workspaceDir);
      setTerminalHistory((prev) => {
        const next = [...prev];
        next[next.length - 1] = {
          command: cmd,
          stdout: result.stdout,
          stderr: result.stderr,
          exit_code: result.exit_code,
          isRunning: false,
        };
        return next;
      });
    } catch (err) {
      setTerminalHistory((prev) => {
        const next = [...prev];
        next[next.length - 1] = {
          command: cmd,
          stderr: String(err),
          exit_code: 1,
          isRunning: false,
        };
        return next;
      });
    } finally {
      setIsTerminalExecuting(false);
      setTimeout(() => {
        terminalInputRef.current?.focus();
      }, 50);
    }
  };

  // Map of changed files for badges in Files tab
  const gitChangesMap = useMemo(() => {
    const map = new Map<string, GitFileChange>();
    if (gitStatus?.changes) {
      for (const change of gitStatus.changes) {
        map.set(change.path, change);
      }
    }
    return map;
  }, [gitStatus]);

  const filteredEntries = useMemo(() => {
    if (!fileSearch.trim()) return workspaceEntries;
    const q = fileSearch.toLowerCase().trim();
    return workspaceEntries.filter(
      (e) => e.name.toLowerCase().includes(q) || e.path.toLowerCase().includes(q)
    );
  }, [workspaceEntries, fileSearch]);

  const activePatch = selectedFileDiff
    ? selectedFileDiff.patch
    : gitStatus?.full_diff || initialPatch;

  if (!open) return null;

  return (
    <aside
      data-testid="right-panel"
      style={{ width: `${width}px` }}
      className={`relative h-full shrink-0 flex flex-col border-l border-zinc-200 bg-white select-none text-zinc-800 z-10 ${className}`}
    >
      {/* Left Edge Resize Handle */}
      <div
        data-testid="right-panel-resize-handle"
        onMouseDown={handleMouseDown}
        title="Drag to resize right panel"
        className="absolute top-0 left-0 bottom-0 w-1 cursor-col-resize hover:w-1.5 hover:bg-zinc-400 active:bg-zinc-900 transition-all z-20"
      />

      {/* Panel Top Header matching Cline Right Panel */}
      <div className="h-10 px-3 border-b border-zinc-200 flex items-center justify-between bg-zinc-50/70">
        {/* Tab Switcher: Changes | Files | Terminal */}
        <div className="flex items-center gap-1 p-0.5 rounded-lg bg-zinc-200/60 text-xs font-medium">
          <button
            type="button"
            data-testid="right-panel-tab-changes"
            onClick={() => {
              setTab("changes");
              setSelectedFileDiff(null);
            }}
            className={`flex items-center gap-1.5 px-2.5 py-1 rounded-md transition-all cursor-pointer ${
              currentTab === "changes"
                ? "bg-white text-zinc-900 shadow-2xs font-semibold"
                : "text-zinc-600 hover:text-zinc-900"
            }`}
          >
            <FileCode className="w-3.5 h-3.5" />
            <span>Changes</span>
            {gitStatus?.changes && gitStatus.changes.length > 0 && (
              <span className="text-[10px] px-1 py-0.2 rounded-full bg-zinc-900 text-white font-mono tabular-nums">
                {gitStatus.changes.length}
              </span>
            )}
          </button>

          <button
            type="button"
            data-testid="right-panel-tab-files"
            onClick={() => setTab("files")}
            className={`flex items-center gap-1.5 px-2.5 py-1 rounded-md transition-all cursor-pointer ${
              currentTab === "files"
                ? "bg-white text-zinc-900 shadow-2xs font-semibold"
                : "text-zinc-600 hover:text-zinc-900"
            }`}
          >
            <FolderTree className="w-3.5 h-3.5" />
            <span>Files</span>
          </button>

          <button
            type="button"
            data-testid="right-panel-tab-terminal"
            onClick={() => setTab("terminal")}
            className={`flex items-center gap-1.5 px-2.5 py-1 rounded-md transition-all cursor-pointer ${
              currentTab === "terminal"
                ? "bg-white text-zinc-900 shadow-2xs font-semibold"
                : "text-zinc-600 hover:text-zinc-900"
            }`}
          >
            <TerminalIcon className="w-3.5 h-3.5" />
            <span>Terminal</span>
          </button>
        </div>

        {/* Close Button */}
        <button
          type="button"
          data-testid="right-panel-close"
          onClick={onClose}
          className="p-1 rounded hover:bg-zinc-200 text-zinc-500 hover:text-zinc-800 transition-colors cursor-pointer"
          title="Close panel (Cmd+J)"
          aria-label="Close panel"
        >
          <X className="w-4 h-4" />
        </button>
      </div>

      {/* Tab 1: Changes (Git Diffs) */}
      {currentTab === "changes" && (
        <div data-testid="right-panel-changes" className="flex-1 overflow-y-auto p-3 space-y-3">
          {/* Changes file list header */}
          {gitStatus?.changes && gitStatus.changes.length > 0 && (
            <div className="space-y-1 pb-2 border-b border-zinc-100">
              <div className="flex items-center justify-between text-xs text-zinc-500 px-1">
                <span className="font-semibold uppercase text-[10px] tracking-wider text-zinc-400">
                  Modified Files ({gitStatus.changes.length})
                </span>
                {selectedFileDiff && (
                  <button
                    type="button"
                    onClick={() => setSelectedFileDiff(null)}
                    className="text-[11px] text-zinc-500 hover:text-zinc-900 cursor-pointer font-medium"
                  >
                    View all changes
                  </button>
                )}
              </div>

              <div className="space-y-0.5">
                {gitStatus.changes.map((change) => {
                  const isSelected = selectedFileDiff?.path === change.path;
                  return (
                    <button
                      key={change.path}
                      type="button"
                      data-testid={`changed-file-${change.path}`}
                      onClick={() => setSelectedFileDiff(change)}
                      className={`w-full text-left px-2 py-1.5 rounded-lg flex items-center justify-between gap-2 text-xs transition-colors cursor-pointer ${
                        isSelected
                          ? "bg-zinc-100 text-zinc-900 font-medium"
                          : "hover:bg-zinc-50 text-zinc-700"
                      }`}
                    >
                      <div className="flex items-center gap-2 truncate min-w-0">
                        <span
                          className={`text-[10px] font-mono px-1 rounded font-bold ${
                            change.status.includes("D")
                              ? "bg-red-100 text-red-700"
                              : change.status.includes("A") || change.status.includes("?")
                              ? "bg-emerald-100 text-emerald-700"
                              : "bg-amber-100 text-amber-800"
                          }`}
                        >
                          {change.status}
                        </span>
                        <span className="truncate font-mono text-[11px]">{change.path}</span>
                      </div>

                      <div className="flex items-center gap-1.5 shrink-0 text-[10px] font-mono tabular-nums">
                        {change.additions > 0 && (
                          <span className="text-emerald-600 font-semibold">+{change.additions}</span>
                        )}
                        {change.deletions > 0 && (
                          <span className="text-rose-600 font-semibold">-{change.deletions}</span>
                        )}
                      </div>
                    </button>
                  );
                })}
              </div>
            </div>
          )}

          {activePatch.trim().length > 0 ? (
            <div className="space-y-2">
              <DiffView patch={activePatch} />
            </div>
          ) : (
            <div className="flex flex-col items-center justify-center py-20 text-center text-zinc-400 space-y-2">
              <FileCode className="w-8 h-8 stroke-1 text-zinc-300" />
              <p className="text-xs font-medium text-zinc-600">No uncommitted changes</p>
              <p className="text-[11px] text-zinc-400 max-w-[220px]">
                Working tree is clean. When files are modified, live git diffs will stream here.
              </p>
            </div>
          )}
        </div>
      )}

      {/* Tab 2: Files (Workspace Explorer with Git Status Badges) */}
      {currentTab === "files" && (
        <div data-testid="right-panel-files" className="flex-1 flex flex-col min-h-0">
          {/* File Search */}
          <div className="p-2 border-b border-zinc-100">
            <div className="relative">
              <Search className="absolute left-2.5 top-2 w-3.5 h-3.5 text-zinc-400" />
              <input
                type="text"
                data-testid="right-panel-file-search"
                value={fileSearch}
                onChange={(e) => setFileSearch(e.target.value)}
                placeholder="Search files in workspace..."
                className="w-full pl-8 pr-2 py-1 text-xs rounded-md bg-zinc-100 border border-zinc-200/80 outline-none placeholder-zinc-400 text-zinc-900 focus:border-zinc-300"
              />
            </div>
          </div>

          {/* Files List */}
          <div className="flex-1 overflow-y-auto p-2 space-y-0.5 text-xs">
            {filteredEntries.length === 0 ? (
              <div className="py-12 text-center text-xs text-zinc-400">
                No files found matching "{fileSearch}"
              </div>
            ) : (
              filteredEntries.map((entry) => {
                const isDir = entry.is_dir;
                const change = gitChangesMap.get(entry.path);

                return (
                  <div
                    key={entry.path}
                    data-testid={`workspace-file-${entry.path}`}
                    onClick={() => {
                      if (change) {
                        setSelectedFileDiff(change);
                        setTab("changes");
                      } else {
                        onInsertMention?.(entry.path);
                      }
                    }}
                    className="flex items-center justify-between px-2 py-1.5 rounded-md hover:bg-zinc-100 cursor-pointer transition-colors text-zinc-700 hover:text-zinc-900 group"
                    title={entry.path}
                  >
                    <div className="flex items-center gap-2 truncate min-w-0">
                      {isDir ? (
                        <Folder className="w-3.5 h-3.5 text-zinc-500 shrink-0" />
                      ) : (
                        <FileText className="w-3.5 h-3.5 text-zinc-400 shrink-0" />
                      )}
                      <span className="truncate font-mono text-[12px]">{entry.path}</span>
                    </div>

                    <div className="flex items-center gap-1.5 shrink-0 font-mono text-[10px]">
                      {change && (
                        <span
                          className={`px-1 py-0.2 rounded font-bold ${
                            change.status.includes("D")
                              ? "bg-red-100 text-red-700"
                              : change.status.includes("A") || change.status.includes("?")
                              ? "bg-emerald-100 text-emerald-700"
                              : "bg-amber-100 text-amber-800"
                          }`}
                        >
                          {change.status}
                        </span>
                      )}
                      <span className="opacity-0 group-hover:opacity-100 text-zinc-400 transition-opacity">
                        @insert
                      </span>
                    </div>
                  </div>
                );
              })
            )}
          </div>
        </div>
      )}

      {/* Tab 3: Terminal (REAL LIVE INTERACTIVE SHELL) */}
      {currentTab === "terminal" && (
        <div data-testid="right-panel-terminal" className="flex-1 flex flex-col min-h-0 bg-zinc-950 text-zinc-200 font-mono text-[12px]">
          {/* Terminal Toolbar */}
          <div className="h-8 px-3 border-b border-zinc-800 flex items-center justify-between bg-zinc-900/90 text-zinc-400 text-[11px]">
            <span className="flex items-center gap-1.5">
              <span className="size-2 rounded-full bg-emerald-500 inline-block" />
              <span>Embedded Shell Output</span>
            </span>

            <div className="flex items-center gap-3">
              <button
                type="button"
                onClick={() => setTerminalHistory([])}
                className="hover:text-zinc-200 transition-colors cursor-pointer"
                title="Clear terminal history"
              >
                Clear
              </button>
              <button
                type="button"
                onClick={handleCopyLogs}
                className="flex items-center gap-1 hover:text-zinc-200 transition-colors cursor-pointer"
                title="Copy terminal logs"
              >
                {copied ? <Check className="w-3 h-3 text-emerald-400" /> : <Copy className="w-3 h-3" />}
                <span>{copied ? "Copied" : "Copy"}</span>
              </button>
            </div>
          </div>

          {/* Terminal History Window */}
          <div ref={terminalScrollRef} className="flex-1 overflow-y-auto p-3 space-y-3 select-text leading-relaxed">
            {terminalHistory.map((item, i) => (
              <div key={i} className="space-y-1">
                <div className="flex items-center gap-2 text-zinc-400">
                  <span className="text-emerald-400 font-bold">$</span>
                  <span className="text-zinc-100">{item.command}</span>
                  {item.isRunning && (
                    <Loader2 className="w-3 h-3 text-amber-400 animate-spin" />
                  )}
                  {!item.isRunning && item.exit_code !== undefined && item.exit_code !== 0 && (
                    <span className="text-[10px] text-rose-400">[exit {item.exit_code}]</span>
                  )}
                </div>

                {item.stdout && (
                  <pre className="text-zinc-300 whitespace-pre-wrap break-all text-[11px] bg-zinc-900/50 p-2 rounded border border-zinc-800/60">
                    {item.stdout}
                  </pre>
                )}

                {item.stderr && (
                  <pre className="text-rose-400 whitespace-pre-wrap break-all text-[11px] bg-rose-950/20 p-2 rounded border border-rose-900/40">
                    {item.stderr}
                  </pre>
                )}
              </div>
            ))}
          </div>

          {/* Interactive Shell Input Line matching genuine terminal */}
          <form
            onSubmit={handleTerminalSubmit}
            className="border-t border-zinc-800 p-2 bg-zinc-900/90 flex items-center gap-2"
          >
            <span className="text-emerald-400 font-bold pl-1">$</span>
            <input
              ref={terminalInputRef}
              data-testid="terminal-interactive-input"
              type="text"
              value={terminalInput}
              onChange={(e) => setTerminalInput(e.target.value)}
              placeholder="Run command in workspace..."
              disabled={isTerminalExecuting}
              className="flex-1 bg-transparent text-zinc-100 placeholder-zinc-500 outline-none text-xs font-mono disabled:opacity-50"
            />
            <button
              type="submit"
              disabled={!terminalInput.trim() || isTerminalExecuting}
              className="px-2 py-0.5 rounded bg-zinc-800 hover:bg-zinc-700 text-zinc-300 disabled:opacity-30 cursor-pointer text-[10px] font-mono"
            >
              Run
            </button>
          </form>
        </div>
      )}
    </aside>
  );
}

export default RightPanel;
