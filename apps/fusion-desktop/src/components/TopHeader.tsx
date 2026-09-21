import React from "react";
import { Folder, MoreHorizontal, PanelLeft, Pin } from "lucide-react";

export interface TopHeaderProps {
  title?: string;
  onToggleSidebar?: () => void;
  onMore?: () => void;
  isPinned?: boolean;
  onTogglePin?: () => void;
  isSidebarOpen?: boolean;
  className?: string;
}

export function TopHeader({
  title = "General chat conversation",
  onToggleSidebar,
  onMore,
  isPinned = false,
  onTogglePin,
  isSidebarOpen = true,
  className = "",
}: TopHeaderProps) {
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
        <Folder className="w-3.5 h-3.5 text-zinc-500 shrink-0 cursor-default" />
      </div>

      {/* Right: Actions (Pin, ..., Sidebar toggle [|]) */}
      <div className="flex items-center gap-1.5 shrink-0">
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
          type="button"
          data-testid="top-header-more"
          onClick={onMore}
          className="p-1 rounded text-zinc-500 hover:text-zinc-800 hover:bg-zinc-100 transition-colors cursor-pointer"
          aria-label="More options"
        >
          <MoreHorizontal className="w-4 h-4" />
        </button>

        {/* Sidebar toggle ([|]) */}
        <button
          type="button"
          data-testid="top-header-sidebar-toggle"
          onClick={onToggleSidebar}
          className="p-1 rounded text-zinc-500 hover:text-zinc-800 hover:bg-zinc-100 transition-colors cursor-pointer"
          aria-label={isSidebarOpen ? "Collapse sidebar" : "Expand sidebar"}
          title={isSidebarOpen ? "Collapse sidebar (Cmd+B)" : "Expand sidebar (Cmd+B)"}
        >
          <PanelLeft className="w-4 h-4" />
        </button>
      </div>
    </header>
  );
}

export default TopHeader;
