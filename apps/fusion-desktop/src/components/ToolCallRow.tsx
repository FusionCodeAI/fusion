import React, { useState } from "react";
import { Check, Loader2, AlertCircle } from "lucide-react";
import type { TurnStep } from "../types";

export interface ToolCallRowProps {
  step: TurnStep;
}

export function ToolCallRow({ step }: ToolCallRowProps) {
  const [isExpanded, setIsExpanded] = useState(false);

  const inspectionContent =
    step.details && step.details.trim().length > 0
      ? step.details
      : step.title;

  return (
    <div className="flex flex-col items-start py-0.5 max-w-full">
      <button
        type="button"
        onClick={() => setIsExpanded((prev) => !prev)}
        className="inline-flex items-center gap-2 px-2.5 py-1 rounded-lg bg-zinc-100/90 hover:bg-zinc-200/80 border border-zinc-200/80 text-[12px] text-zinc-700 cursor-pointer transition-colors max-w-full"
      >
        {step.status === "completed" ? (
          <Check className="w-3.5 h-3.5 text-emerald-600 shrink-0" />
        ) : step.status === "running" ? (
          <Loader2 className="w-3.5 h-3.5 text-zinc-500 animate-spin shrink-0" />
        ) : (
          <AlertCircle className="w-3.5 h-3.5 text-rose-500 shrink-0" />
        )}

        <span className="truncate font-medium">{step.title}</span>
      </button>

      {isExpanded && (
        <pre className="max-h-40 overflow-y-auto font-mono text-[11px] bg-zinc-50 border border-zinc-200 rounded-lg p-2.5 text-zinc-600 mt-1 w-full whitespace-pre-wrap break-all">
          {inspectionContent}
        </pre>
      )}
    </div>
  );
}

export default ToolCallRow;
