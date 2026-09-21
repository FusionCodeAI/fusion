import React, { useState, useEffect } from "react";
import { Brain, ChevronDown, ChevronUp } from "lucide-react";

export interface ThinkingRowProps {
  thought: string;
  isGenerating?: boolean;
}

export function ThinkingRow({ thought, isGenerating }: ThinkingRowProps) {
  const [isExpanded, setIsExpanded] = useState(false);
  const [elapsedSec, setElapsedSec] = useState(0);

  useEffect(() => {
    if (!isGenerating) return;
    const start = Date.now();
    const timer = setInterval(() => {
      setElapsedSec(Math.max(1, Math.floor((Date.now() - start) / 1000)));
    }, 1000);
    return () => clearInterval(timer);
  }, [isGenerating]);

  // If there's no thought and not currently generating, do not render a thinking row
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

  if (isThinkingWithoutThought) {
    return (
      <div className="w-full py-1 select-none flex items-center gap-1.5 text-xs text-zinc-500">
        <Brain className="w-3.5 h-3.5 text-[#5100cd] animate-pulse shrink-0" />
        <span className="font-medium text-zinc-600 animate-pulse">
          {elapsedSec > 0 ? `Thinking (${elapsedSec}s)...` : "Thinking..."}
        </span>
      </div>
    );
  }

  // If generation finished and there are no thoughts, hide the row
  if (!isGenerating && cleanParagraphs.length === 0) {
    return null;
  }

  return (
    <div className="w-full py-1 select-none">
      <button
        type="button"
        onClick={() => setIsExpanded((prev) => !prev)}
        className="group inline-flex items-center gap-1.5 text-xs text-zinc-500 hover:text-zinc-800 transition-colors cursor-pointer"
      >
        <Brain className="w-3.5 h-3.5 text-zinc-400 group-hover:text-zinc-600 shrink-0" />
        <span className="font-medium inline-flex items-center gap-1">
          <span>{elapsedSec > 1 ? `Thought for ${elapsedSec}s` : "Thought briefly"}</span>
          {isExpanded ? (
            <ChevronUp className="w-3 h-3 text-zinc-400" />
          ) : (
            <ChevronDown className="w-3 h-3 text-zinc-400" />
          )}
        </span>
      </button>

      {isExpanded && cleanParagraphs.length > 0 && (
        <div className="mt-2 text-xs text-zinc-600 leading-relaxed font-sans space-y-1 border-l-2 border-purple-200/80 pl-3 my-1">
          {cleanParagraphs.map((paragraph, index) => (
            <p key={index}>{paragraph}</p>
          ))}
        </div>
      )}
    </div>
  );
}

export default ThinkingRow;
