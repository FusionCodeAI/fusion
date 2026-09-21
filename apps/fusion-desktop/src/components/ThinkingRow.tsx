import React, { useState } from "react";
import { Loader2, ChevronDown, ChevronUp } from "lucide-react";

export interface ThinkingRowProps {
  thought: string;
  isGenerating?: boolean;
}

export function ThinkingRow({ thought, isGenerating }: ThinkingRowProps) {
  const [isExpanded, setIsExpanded] = useState(false);

  // If there's no thought and not generating, render nothing
  if (!thought && !isGenerating) {
    return null;
  }

  const isThinkingWithoutThought = Boolean(isGenerating && !thought);
  const cleanParagraphs = thought
    ? thought
        .split("\n")
        .map((line) => line.trim())
        .filter((line) => line.length > 0)
    : [];

  return (
    <div className="w-full py-1 select-none">
      <button
        type="button"
        disabled={isThinkingWithoutThought}
        onClick={() => {
          if (!isThinkingWithoutThought) {
            setIsExpanded((prev) => !prev);
          }
        }}
        className="group inline-flex items-center gap-1.5 text-xs text-zinc-500 hover:text-zinc-700 transition-colors cursor-pointer disabled:cursor-default"
      >
        {isThinkingWithoutThought ? (
          <>
            <Loader2 className="w-3 h-3 animate-spin text-zinc-400 shrink-0" />
            <span className="font-medium text-zinc-500">Thinking...</span>
          </>
        ) : (
          <span className="font-medium inline-flex items-center gap-1">
            <span>Thought briefly</span>
            {isExpanded ? (
              <ChevronUp className="w-3 h-3 text-zinc-400" />
            ) : (
              <ChevronDown className="w-3 h-3 text-zinc-400" />
            )}
          </span>
        )}
      </button>

      {isExpanded && cleanParagraphs.length > 0 && (
        <div className="mt-2 text-[13px] text-zinc-600 leading-relaxed font-sans space-y-1.5 whitespace-pre-wrap break-words">
          {cleanParagraphs.map((paragraph, index) => (
            <p key={index}>{paragraph}</p>
          ))}
        </div>
      )}
    </div>
  );
}

export default ThinkingRow;
