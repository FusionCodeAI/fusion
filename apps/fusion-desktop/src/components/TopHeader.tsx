import React, { useState, useRef, useEffect } from "react";
import {
  Folder,
  MoreHorizontal,
  PanelRight,
  Pin,
  Download,
  FileText,
  Copy,
  Trash2,
  Check,
  RotateCcw,
} from "lucide-react";

export interface TopHeaderProps {
  title?: string;
  isPinned?: boolean;
  onTogglePin?: () => void;
  isRightPanelOpen?: boolean;
  onToggleRightPanel?: () => void;
  onExportMarkdown?: () => void;
  onExportHtml?: () => void;
  onCopyTranscript?: () => void;
  onClearSession?: () => void;
  onDeleteSession?: () => void;
  className?: string;
}

export function TopHeader({
  title = "General chat conversation",
  isPinned = false,
  onTogglePin,
  isRightPanelOpen = false,
  onToggleRightPanel,
  onExportMarkdown,
  onExportHtml,
  onCopyTranscript,
  onClearSession,
  onDeleteSession,
  className = "",
}: TopHeaderProps) {
  const [isMenuOpen, setIsMenuOpen] = useState(false);
  const [copied, setCopied] = useState(false);
  const menuRef = useRef<HTMLDivElement>(null);
  const buttonRef = useRef<HTMLButtonElement>(null);

  // Close dropdown when clicking outside
  useEffect(() => {
    const handleClickOutside = (e: MouseEvent) => {
      const target = e.target as Node;
      if (
        isMenuOpen &&
        menuRef.current &&
        !menuRef.current.contains(target) &&
        buttonRef.current &&
        !buttonRef.current.contains(target)
      ) {
        setIsMenuOpen(false);
      }
    };
    document.addEventListener("mousedown", handleClickOutside);
    return () => document.removeEventListener("mousedown", handleClickOutside);
  }, [isMenuOpen]);

  const handleCopy = () => {
    onCopyTranscript?.();
    setCopied(true);
    setTimeout(() => {
      setCopied(false);
      setIsMenuOpen(false);
    }, 1200);
  };

  return (
    <header
      data-testid="top-header"
      data-tauri-drag-region
      className={`h-10 shrink-0 px-4 border-b border-zinc-200/80 bg-white flex items-center justify-between select-none ${className}`}
    >
      {/* Left: Session Title + Folder Icon */}
      <div className="flex items-center gap-2 min-w-0">
        <span
          data-testid="top-header-title"
          className="text-[13px] font-medium text-zinc-900 truncate tracking-tight"
        >
          {title}
        </span>
        <Folder className="w-3.5 h-3.5 text-zinc-400 shrink-0 cursor-default" />
      </div>

      {/* Right: Actions (Pin, ..., Secondary Panel [|]) */}
      <div className="flex items-center gap-1.5 shrink-0 relative">
        {/* Pin button */}
        {onTogglePin && (
          <button
            type="button"
            data-testid="top-header-pin"
            onClick={onTogglePin}
            className={`p-1 rounded transition-colors cursor-pointer ${
              isPinned
                ? "text-zinc-900 bg-zinc-100"
                : "text-zinc-400 hover:text-zinc-800 hover:bg-zinc-100"
            }`}
            title={isPinned ? "Unpin chat" : "Pin chat"}
            aria-label={isPinned ? "Unpin chat" : "Pin chat"}
          >
            <Pin className={`w-3.5 h-3.5 ${isPinned ? "fill-current" : ""}`} />
          </button>
        )}

        {/* More options (...) */}
        <button
          ref={buttonRef}
          type="button"
          data-testid="top-header-more"
          onClick={() => setIsMenuOpen((prev) => !prev)}
          className={`p-1 rounded transition-colors cursor-pointer ${
            isMenuOpen ? "bg-zinc-100 text-zinc-900" : "text-zinc-500 hover:text-zinc-800 hover:bg-zinc-100"
          }`}
          aria-label="More session actions"
          aria-expanded={isMenuOpen}
        >
          <MoreHorizontal className="w-4 h-4" />
        </button>

        {/* Session Actions Dropdown Menu matching Cline */}
        {isMenuOpen && (
          <div
            ref={menuRef}
            data-testid="top-header-actions-menu"
            className="absolute right-8 top-8 w-56 bg-white rounded-xl border border-zinc-200 shadow-xl py-1 z-50 text-xs text-zinc-700 select-none animate-in fade-in zoom-in-95 duration-100"
          >
            <div className="px-3 py-1 text-[10px] font-medium text-zinc-400 uppercase tracking-wider">
              Session Actions
            </div>

            {/* Copy Transcript */}
            <button
              type="button"
              data-testid="header-action-copy"
              onClick={handleCopy}
              className="w-full text-left px-3 py-1.5 flex items-center justify-between hover:bg-zinc-50 cursor-pointer transition-colors"
            >
              <div className="flex items-center gap-2">
                {copied ? <Check className="w-3.5 h-3.5 text-zinc-900" /> : <Copy className="w-3.5 h-3.5 text-zinc-500" />}
                <span>{copied ? "Copied to clipboard" : "Copy transcript"}</span>
              </div>
            </button>

            {/* Export Markdown */}
            <button
              type="button"
              data-testid="header-action-export-md"
              onClick={() => {
                onExportMarkdown?.();
                setIsMenuOpen(false);
              }}
              className="w-full text-left px-3 py-1.5 flex items-center gap-2 hover:bg-zinc-50 cursor-pointer transition-colors"
            >
              <FileText className="w-3.5 h-3.5 text-zinc-500" />
              <span>Export as Markdown (.md)</span>
            </button>

            {/* Export HTML */}
            <button
              type="button"
              data-testid="header-action-export-html"
              onClick={() => {
                onExportHtml?.();
                setIsMenuOpen(false);
              }}
              className="w-full text-left px-3 py-1.5 flex items-center gap-2 hover:bg-zinc-50 cursor-pointer transition-colors"
            >
              <Download className="w-3.5 h-3.5 text-zinc-500" />
              <span>Export as Standalone HTML</span>
            </button>

            <div className="my-1 border-t border-zinc-100" />

            {/* Clear Conversation */}
            {onClearSession && (
              <button
                type="button"
                data-testid="header-action-clear"
                onClick={() => {
                  onClearSession();
                  setIsMenuOpen(false);
                }}
                className="w-full text-left px-3 py-1.5 flex items-center gap-2 hover:bg-zinc-50 cursor-pointer transition-colors text-zinc-700"
              >
                <RotateCcw className="w-3.5 h-3.5 text-zinc-500" />
                <span>Clear conversation</span>
              </button>
            )}

            {/* Delete Session */}
            {onDeleteSession && (
              <button
                type="button"
                data-testid="header-action-delete"
                onClick={() => {
                  onDeleteSession();
                  setIsMenuOpen(false);
                }}
                className="w-full text-left px-3 py-1.5 flex items-center gap-2 hover:bg-red-50 text-red-600 cursor-pointer transition-colors"
              >
                <Trash2 className="w-3.5 h-3.5 text-red-500" />
                <span>Delete session</span>
              </button>
            )}
          </div>
        )}

        {/* Right Secondary Panel toggle ([|]) matching Cline */}
        {onToggleRightPanel && (
          <button
            type="button"
            data-testid="top-header-right-panel-toggle"
            onClick={onToggleRightPanel}
            className={`p-1 rounded transition-colors cursor-pointer ${
              isRightPanelOpen
                ? "bg-zinc-100 text-zinc-900"
                : "text-zinc-500 hover:text-zinc-800 hover:bg-zinc-100"
            }`}
            aria-label={isRightPanelOpen ? "Collapse secondary panel" : "Expand secondary panel"}
            title={isRightPanelOpen ? "Collapse secondary panel (Cmd+J)" : "Expand secondary panel (Cmd+J)"}
          >
            <PanelRight className="w-4 h-4" />
          </button>
        )}
      </div>
    </header>
  );
}

export default TopHeader;
