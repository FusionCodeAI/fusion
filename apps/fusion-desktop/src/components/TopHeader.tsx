import React from "react";
import { Folder, ExternalLink, MoreHorizontal, PanelLeft } from "lucide-react";

export interface TopHeaderProps {
  title?: string;
  onToggleSidebar?: () => void;
  onOpenIde?: () => void;
  onMore?: () => void;
  isSidebarOpen?: boolean;
  className?: string;
}

export function TopHeader({
  title = "General chat conversation",
  onToggleSidebar,
  onOpenIde,
  onMore,
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

      {/* Right: Actions (IDE ↗, ..., Sidebar toggle [|]) */}
      <div className="flex items-center gap-2 shrink-0">
        {/* IDE ↗ button */}
        <button
          type="button"
          data-testid="top-header-ide"
          onClick={onOpenIde}
          className="inline-flex items-center gap-1 px-2 py-1 rounded text-xs font-medium text-zinc-600 hover:text-zinc-900 hover:bg-zinc-100 transition-colors cursor-pointer"
          aria-label="Open in IDE"
        >
          <span>IDE</span>
          <ExternalLink className="w-3 h-3 text-zinc-500" />
        </button>

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
