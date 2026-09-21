import React, { memo, useCallback, useEffect, useRef, useState } from "react";
import { ChevronDown, ChevronRight } from "lucide-react";
import { MarkdownRenderer } from "./MarkdownRenderer";
export interface ThinkingRowProps {
  thought?: string;
  isGenerating?: boolean;
  durationMs?: number;
}

export const ThinkingRow = memo(function ThinkingRow({
  thought,
  isGenerating = false,
  durationMs,
}: ThinkingRowProps) {
  const [isExpanded, setIsExpanded] = useState(false);
  const [canScrollUp, setCanScrollUp] = useState(false);
  const [canScrollDown, setCanScrollDown] = useState(false);
  const [seconds, setSeconds] = useState(0);

  const scrollRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    if (!isGenerating) return;
    const start = Date.now();
    const timer = setInterval(() => {
      setSeconds(Math.max(1, Math.round((Date.now() - start) / 1000)));
    }, 1000);
    return () => clearInterval(timer);
  }, [isGenerating]);

  const checkScrollable = useCallback(() => {
    if (scrollRef.current) {
      const { scrollTop, scrollHeight, clientHeight } = scrollRef.current;
      setCanScrollUp(scrollTop > 1);
      setCanScrollDown(scrollTop + clientHeight < scrollHeight - 1);
    }
  }, []);

  useEffect(() => {
    if (scrollRef.current && isGenerating) {
      scrollRef.current.scrollTop = scrollRef.current.scrollHeight;
    }
    checkScrollable();
  }, [thought, isGenerating, checkScrollable]);

  const cleanThought = thought?.trim();

  // If not generating and no reasoning content, do not render anything
  if (!isGenerating && !cleanThought) {
    return null;
  }

  const durationSec = durationMs
    ? Math.max(1, Math.round(durationMs / 1000))
    : seconds;
  const title = isGenerating
    ? seconds > 0
      ? `Thinking (${seconds}s)...`
      : "Thinking..."
    : durationSec > 1
    ? `Thought for ${durationSec}s`
    : "Thought briefly";

  return (
    <div className="w-full my-1 pl-0 select-none">
      {/* Trigger Button matching Cline ThinkingRow */}
      <button
        type="button"
        data-testid="thinking-trigger"
        onClick={() => setIsExpanded((prev) => !prev)}
        className="inline-flex items-center gap-1 text-left text-xs font-medium text-zinc-500 hover:text-zinc-800 transition-colors cursor-pointer"
      >
        <span
          className={
            isGenerating
              ? "animate-pulse text-zinc-800 font-semibold"
              : "text-zinc-500 font-medium"
          }
        >
          {title}
        </span>
        {isExpanded ? (
          <ChevronDown className="w-3.5 h-3.5 text-zinc-400" />
        ) : (
          <ChevronRight className="w-3.5 h-3.5 text-zinc-400" />
        )}
      </button>

      {isExpanded && cleanThought ? (
        <div className="relative mt-2 rounded-lg border-l-2 border-zinc-300 pl-3 py-1.5 bg-zinc-50/70 text-zinc-700 text-xs">
          <div
            ref={scrollRef}
            onScroll={checkScrollable}
            className="max-h-[220px] overflow-y-auto pr-2 text-zinc-700 select-text leading-relaxed"
          >
            <MarkdownRenderer content={cleanThought} />
          </div>

          {canScrollUp && (
            <div className="absolute top-0 left-0 right-0 h-4 pointer-events-none bg-gradient-to-b from-zinc-50/90 to-transparent" />
          )}
          {canScrollDown && (
            <div className="absolute bottom-0 left-0 right-0 h-4 pointer-events-none bg-gradient-to-t from-zinc-50/90 to-transparent" />
          )}
        </div>
      ) : null}
    </div>
  );
});

export default ThinkingRow;
