import React, { useState, useMemo, useRef } from "react";
import {
  FileCode,
  Folder,
  FolderTree,
  Terminal,
  X,
  Search,
  ChevronRight,
  ChevronDown,
  FileText,
  Copy,
  Check,
  RotateCcw,
  Sparkles,
} from "lucide-react";
import { DiffView } from "./DiffView";
import type { WorkspaceEntry } from "../lib/fusion-ipc";

export type RightPanelTab = "changes" | "files" | "terminal";

export interface RightPanelProps {
  open: boolean;
  onClose: () => void;
  width?: number;
  onResize?: (width: number) => void;
  activeTab?: RightPanelTab;
  onSelectTab?: (tab: RightPanelTab) => void;
  diffPatch?: string;
  workspaceEntries?: WorkspaceEntry[];
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
  diffPatch = "",
  workspaceEntries = [],
  terminalLogs = [],
  onInsertMention,
  className = "",
}: RightPanelProps) {
  const [internalTab, setInternalTab] = useState<RightPanelTab>("changes");
  const [fileSearch, setFileSearch] = useState("");
  const [copied, setCopied] = useState(false);
  const [collapsedFolders, setCollapsedFolders] = useState<Set<string>>(() => new Set());

  const currentTab = controlledTab ?? internalTab;
  const setTab = onSelectTab ?? setInternalTab;

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
    const text = terminalLogs
      .map((l) => `$ ${l.command}\n${l.output || ""}`)
      .join("\n\n");
    navigator.clipboard.writeText(text);
    setCopied(true);
    setTimeout(() => setCopied(false), 1500);
  };

  const filteredEntries = useMemo(() => {
    if (!fileSearch.trim()) return workspaceEntries;
    const q = fileSearch.toLowerCase().trim();
    return workspaceEntries.filter(
      (e) => e.name.toLowerCase().includes(q) || e.path.toLowerCase().includes(q)
    );
  }, [workspaceEntries, fileSearch]);

  const toggleFolder = (folderPath: string) => {
    setCollapsedFolders((prev) => {
      const next = new Set(prev);
      if (next.has(folderPath)) next.delete(folderPath);
      else next.add(folderPath);
      return next;
    });
  };

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
            onClick={() => setTab("changes")}
            className={`flex items-center gap-1.5 px-2.5 py-1 rounded-md transition-all cursor-pointer ${
              currentTab === "changes"
                ? "bg-white text-zinc-900 shadow-2xs font-semibold"
                : "text-zinc-600 hover:text-zinc-900"
            }`}
          >
            <FileCode className="w-3.5 h-3.5" />
            <span>Changes</span>
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
            <Terminal className="w-3.5 h-3.5" />
            <span>Terminal</span>
          </button>
        </div>

        {/* Close Button */}
        <button
          type="button"
          data-testid="right-panel-close"
          onClick={onClose}
          className="p-1 rounded hover:bg-zinc-200 text-zinc-500 hover:text-zinc-800 transition-colors cursor-pointer"
          title="Close panel"
          aria-label="Close panel"
        >
          <X className="w-4 h-4" />
        </button>
      </div>

      {/* Tab 1: Changes (Git Diffs) */}
      {currentTab === "changes" && (
        <div data-testid="right-panel-changes" className="flex-1 overflow-y-auto p-3 space-y-3">
          {diffPatch.trim().length > 0 ? (
            <div className="space-y-3">
              <div className="flex items-center justify-between text-xs text-zinc-500">
                <span className="font-semibold uppercase text-[11px] tracking-wider text-zinc-400">
                  Active Modifications
                </span>
              </div>
              <DiffView patch={diffPatch} />
            </div>
          ) : (
            <div className="flex flex-col items-center justify-center py-20 text-center text-zinc-400 space-y-2">
              <FileCode className="w-8 h-8 stroke-1 text-zinc-300" />
              <p className="text-xs font-medium text-zinc-600">No active file changes</p>
              <p className="text-[11px] text-zinc-400 max-w-[220px]">
                When the agent edits files, unified git diffs will appear here in real time.
              </p>
            </div>
          )}
        </div>
      )}

      {/* Tab 2: Files (Workspace Explorer) */}
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
                return (
                  <div
                    key={entry.path}
                    data-testid={`workspace-file-${entry.path}`}
                    onClick={() => onInsertMention?.(entry.path)}
                    className="flex items-center justify-between px-2 py-1 rounded-md hover:bg-zinc-100 cursor-pointer transition-colors text-zinc-700 hover:text-zinc-900 group"
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

                    <span className="opacity-0 group-hover:opacity-100 text-[10px] text-zinc-400 font-mono transition-opacity">
                      @insert
                    </span>
                  </div>
                );
              })
            )}
          </div>
        </div>
      )}

      {/* Tab 3: Terminal (Live Console Output) */}
      {currentTab === "terminal" && (
        <div data-testid="right-panel-terminal" className="flex-1 flex flex-col min-h-0 bg-zinc-950 text-zinc-200 font-mono text-[12px]">
          {/* Terminal Toolbar */}
          <div className="h-8 px-3 border-b border-zinc-800 flex items-center justify-between bg-zinc-900/90 text-zinc-400 text-[11px]">
            <span className="flex items-center gap-1.5">
              <span className="size-2 rounded-full bg-emerald-500 inline-block" />
              <span>Embedded Shell Output</span>
            </span>

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

          {/* Terminal Logs Window */}
          <div className="flex-1 overflow-y-auto p-3 space-y-3 select-text leading-relaxed">
            {terminalLogs.length === 0 ? (
              <div className="text-zinc-600 py-12 text-center text-xs">
                No active command executions yet.
              </div>
            ) : (
              terminalLogs.map((log, i) => (
                <div key={i} className="space-y-1">
                  <div className="flex items-center gap-2 text-zinc-400">
                    <span className="text-emerald-400 font-bold">$</span>
                    <span className="text-zinc-200">{log.command}</span>
                    {log.status === "running" && (
                      <span className="text-[10px] text-amber-400 animate-pulse">[running]</span>
                    )}
                  </div>
                  {log.output && (
                    <pre className="text-zinc-400 whitespace-pre-wrap break-all text-[11px] bg-zinc-900/60 p-2 rounded border border-zinc-800/80">
                      {log.output}
                    </pre>
                  )}
                </div>
              ))
            )}
          </div>
        </div>
      )}
    </aside>
  );
}

export default RightPanel;
