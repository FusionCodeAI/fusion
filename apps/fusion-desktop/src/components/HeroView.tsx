import React from "react";
import { Sparkles } from "lucide-react";
import { FusionMascot } from "./FusionMascot";
import { Composer, type ComposerProps } from "./Composer";
import type { ChatImageAttachment } from "../types";

export interface HeroViewProps {
  onSend: (text: string, images?: ChatImageAttachment[]) => void;
  onCancel?: () => void;
  isGenerating?: boolean;
  selectedModel?: string;
  onSelectModel?: (modelId: string) => void;
  className?: string;
}

const QUICK_PROMPTS = [
  "Build a new feature in React & Tailwind",
  "Explain workspace code architecture",
  "Refactor components for cleaner state",
  "Find and resolve failing tests",
];

export function HeroView({
  onSend,
  onCancel,
  isGenerating = false,
  selectedModel,
  onSelectModel,
  className = "",
}: HeroViewProps) {
  return (
    <div
      className={`flex flex-col items-center justify-center min-h-full w-full max-w-3xl mx-auto px-4 py-12 select-none ${className}`}
    >
      {/* Header section */}
      <div className="flex flex-col items-center text-center mb-8">
        <div className="w-16 h-16 rounded-2xl bg-purple-50/60 border border-purple-200/50 flex items-center justify-center mb-4 text-zinc-700 shadow-xs p-1">
          <FusionMascot className="w-14 h-14" animated={true} />
        </div>
        <h1 className="text-3xl sm:text-4xl font-semibold tracking-tight text-zinc-900">
          What should we build today?
        </h1>
        <p className="text-sm text-zinc-500 mt-2.5 max-w-md leading-relaxed">
          Ask questions, plan features, or generate code with Fusion Agent.
        </p>
      </div>

      {/* Elevated prompt card */}
      <Composer
        onSend={onSend}
        onCancel={onCancel}
        isGenerating={isGenerating}
        selectedModel={selectedModel}
        onSelectModel={onSelectModel}
        autoFocus
      />

      {/* Suggestion Chips */}
      <div className="flex flex-wrap items-center justify-center gap-2 mt-6 max-w-[680px]">
        {QUICK_PROMPTS.map((prompt) => (
          <button
            key={prompt}
            type="button"
            onClick={() => onSend(prompt)}
            className="px-3 py-1.5 text-xs text-zinc-600 bg-zinc-50 hover:bg-zinc-100 border border-zinc-200/80 rounded-full transition-colors cursor-pointer"
          >
            {prompt}
          </button>
        ))}
      </div>
    </div>
  );
}
