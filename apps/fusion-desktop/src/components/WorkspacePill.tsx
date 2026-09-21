import React from "react";
import { Monitor, Folder } from "lucide-react";

export interface WorkspacePillProps {
  name?: string;
  className?: string;
  onClick?: () => void;
}

export function WorkspacePill({
  name = "workspace",
  className = "",
  onClick,
}: WorkspacePillProps) {
  return (
    <button
      type="button"
      data-testid="workspace-pill"
      onClick={onClick}
      className={`inline-flex items-center gap-2 px-2.5 py-1 rounded-lg border border-zinc-200/80 bg-white hover:bg-zinc-50 shadow-2xs text-xs font-medium text-zinc-700 transition-colors cursor-pointer ${className}`}
    >
      <Monitor className="w-3.5 h-3.5 text-zinc-500" />
      <div className="flex items-center gap-1.5 pl-1.5 border-l border-zinc-200/80">
        <Folder className="w-3.5 h-3.5 text-zinc-500" />
        <span>{name}</span>
      </div>
    </button>
  );
}

export default WorkspacePill;
