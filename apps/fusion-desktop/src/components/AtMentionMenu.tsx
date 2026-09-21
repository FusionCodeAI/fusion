import React, { useEffect, useRef } from "react";
import {
  FileCode,
  Folder,
  GitBranch,
  GitCommit,
  AlertCircle,
  Terminal,
  Globe,
  FileText,
  AtSign,
} from "lucide-react";
import type { MentionItem } from "../lib/mentions-catalog";

export interface AtMentionMenuProps {
  mentions: MentionItem[];
  selectedIndex: number;
  onSelect: (mention: MentionItem) => void;
  onClose: () => void;
  className?: string;
}

function renderMentionIcon(iconName?: string) {
  switch (iconName) {
    case "FileCode":
      return <FileCode className="w-3.5 h-3.5 text-zinc-500" />;
    case "Folder":
      return <Folder className="w-3.5 h-3.5 text-zinc-500" />;
    case "GitBranch":
      return <GitBranch className="w-3.5 h-3.5 text-zinc-500" />;
    case "GitCommit":
      return <GitCommit className="w-3.5 h-3.5 text-zinc-500" />;
    case "AlertCircle":
      return <AlertCircle className="w-3.5 h-3.5 text-zinc-500" />;
    case "Terminal":
      return <Terminal className="w-3.5 h-3.5 text-zinc-500" />;
    case "Globe":
      return <Globe className="w-3.5 h-3.5 text-zinc-500" />;
    default:
      return <FileText className="w-3.5 h-3.5 text-zinc-400" />;
  }
}

export function AtMentionMenu({
  mentions,
  selectedIndex,
  onSelect,
  onClose,
  className = "",
}: AtMentionMenuProps) {
  const containerRef = useRef<HTMLDivElement>(null);
  const activeItemRef = useRef<HTMLButtonElement>(null);

  useEffect(() => {
    if (activeItemRef.current) {
      activeItemRef.current.scrollIntoView({
        block: "nearest",
        behavior: "smooth",
      });
    }
  }, [selectedIndex]);

  if (mentions.length === 0) {
    return null;
  }

  return (
    <div
      ref={containerRef}
      data-testid="at-mention-menu"
      className={`absolute bottom-full left-0 right-0 mb-2 max-h-64 overflow-y-auto bg-white rounded-xl border border-zinc-200 shadow-lg z-50 p-1 select-none ${className}`}
    >
      <div className="space-y-0.5">
        {mentions.map((item, index) => {
          const isSelected = index === selectedIndex;

          return (
            <button
              key={item.id}
              ref={isSelected ? activeItemRef : null}
              type="button"
              data-testid={`mention-item-${item.label}`}
              onClick={() => onSelect(item)}
              onMouseDown={(e) => {
                e.preventDefault();
                onSelect(item);
              }}
              className={`w-full text-left px-2.5 py-1.5 rounded-lg flex items-center justify-between gap-2 text-xs transition-colors cursor-pointer ${
                isSelected
                  ? "bg-zinc-100 text-zinc-900 font-medium"
                  : "hover:bg-zinc-50 text-zinc-700 font-normal"
              }`}
            >
              <div className="flex items-center gap-2 min-w-0">
                <div className="shrink-0 text-zinc-500">
                  {renderMentionIcon(item.icon)}
                </div>
                <span className="font-mono font-medium text-zinc-900 text-xs">
                  {item.label}
                </span>
                <span className="text-zinc-400 text-xs truncate font-mono">
                  {item.description}
                </span>
              </div>

              <div className="shrink-0 text-[11px] text-zinc-400 font-mono">
                {item.category}
              </div>
            </button>
          );
        })}
      </div>
    </div>
  );
}

export default AtMentionMenu;
