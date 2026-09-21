import React, { useState, useRef, useEffect } from "react";
import {
  Plus,
  ChevronDown,
  Mic,
  ArrowUp,
  Square,
  GitBranch,
  Gauge,
  Check,
} from "lucide-react";
import { FUSION_MODELS, DEFAULT_FUSION_MODEL } from "../models";

export interface ComposerProps {
  onSend: (text: string) => void;
  onCancel?: () => void;
  isGenerating?: boolean;
  selectedModel?: string;
  onSelectModel?: (modelId: string) => void;
  className?: string;
  placeholder?: string;
  autoFocus?: boolean;
}

export function Composer({
  onSend,
  onCancel,
  isGenerating = false,
  selectedModel,
  onSelectModel,
  className = "",
  placeholder = "Ask anything, type @ to mention, or / for commands...",
  autoFocus = false,
}: ComposerProps) {
  const [text, setText] = useState("");
  const [isModelMenuOpen, setIsModelMenuOpen] = useState(false);
  const textareaRef = useRef<HTMLTextAreaElement>(null);
  const modelMenuRef = useRef<HTMLDivElement>(null);
  const modelButtonRef = useRef<HTMLButtonElement>(null);

  const currentModel =
    FUSION_MODELS.find((m) => m.id === selectedModel) ?? DEFAULT_FUSION_MODEL;

  // Auto-resize textarea height
  useEffect(() => {
    const textarea = textareaRef.current;
    if (!textarea) return;
    textarea.style.height = "auto";
    const scrollHeight = textarea.scrollHeight;
    const clampedHeight = Math.min(Math.max(scrollHeight, 48), 200);
    textarea.style.height = `${clampedHeight}px`;
  }, [text]);

  // Autofocus when requested
  useEffect(() => {
    if (autoFocus && textareaRef.current) {
      textareaRef.current.focus();
    }
  }, [autoFocus]);

  // Dismiss model picker dialog when clicking outside
  useEffect(() => {
    if (!isModelMenuOpen) return;
    const handleClickOutside = (event: MouseEvent) => {
      const target = event.target as Node;
      if (
        modelMenuRef.current &&
        !modelMenuRef.current.contains(target) &&
        modelButtonRef.current &&
        !modelButtonRef.current.contains(target)
      ) {
        setIsModelMenuOpen(false);
      }
    };
    document.addEventListener("mousedown", handleClickOutside);
    return () => {
      document.removeEventListener("mousedown", handleClickOutside);
    };
  }, [isModelMenuOpen]);

  const handleSend = () => {
    if (isGenerating) {
      onCancel?.();
      return;
    }
    const trimmed = text.trim();
    if (!trimmed) return;
    onSend(trimmed);
    setText("");
    if (textareaRef.current) {
      textareaRef.current.style.height = "auto";
    }
  };

  const handleKeyDown = (e: React.KeyboardEvent<HTMLTextAreaElement>) => {
    // Typing automatically dismisses model picker dialog if open
    if (isModelMenuOpen) {
      setIsModelMenuOpen(false);
    }

    if (e.key === "Enter" && !e.shiftKey && !e.nativeEvent.isComposing) {
      e.preventDefault();
      handleSend();
    }
  };

  const handleTextareaClick = () => {
    // Clicking input automatically dismisses model picker dialog if open
    if (isModelMenuOpen) {
      setIsModelMenuOpen(false);
    }
  };

  const handleTextareaChange = (e: React.ChangeEvent<HTMLTextAreaElement>) => {
    // Typing automatically dismisses model picker dialog if open
    if (isModelMenuOpen) {
      setIsModelMenuOpen(false);
    }
    setText(e.target.value);
  };

  return (
    <div className={`w-full max-w-[680px] flex flex-col gap-1.5 ${className}`}>
      {/* Elevated Card */}
      <div className="w-full max-w-[680px] bg-white border border-zinc-200 shadow-sm rounded-2xl p-3 flex flex-col gap-2 transition-shadow focus-within:border-zinc-300 focus-within:shadow-md relative">
        {/* Auto-resizing multi-line textarea */}
        <textarea
          ref={textareaRef}
          value={text}
          onChange={handleTextareaChange}
          onClick={handleTextareaClick}
          onKeyDown={handleKeyDown}
          placeholder={placeholder}
          rows={1}
          className="min-h-[48px] max-h-[200px] text-[14px] leading-relaxed text-zinc-900 placeholder:text-zinc-400 bg-transparent border-none outline-none resize-none w-full p-1"
        />

        {/* Bottom Toolbar inside card */}
        <div className="flex items-center justify-between pt-1 relative">
          {/* Left: + button in soft gray rounded square, then Model Selector pill */}
          <div className="flex items-center gap-2">
            <button
              type="button"
              className="p-1.5 rounded-lg bg-zinc-100 hover:bg-zinc-200 text-zinc-600 transition-colors cursor-pointer"
              title="Add context or attachment"
              aria-label="Add context"
            >
              <Plus className="w-4 h-4" />
            </button>

            <div className="relative">
              <button
                ref={modelButtonRef}
                type="button"
                onClick={() => setIsModelMenuOpen((prev) => !prev)}
                className="flex items-center gap-1.5 px-2.5 py-1 rounded-lg text-xs font-medium text-zinc-700 hover:bg-zinc-100 transition-colors border border-zinc-200/80 cursor-pointer"
                aria-haspopup="listbox"
                aria-expanded={isModelMenuOpen}
              >
                <span>{currentModel.shortName}</span>
                <ChevronDown
                  className={`w-3.5 h-3.5 text-zinc-400 transition-transform duration-150 ${
                    isModelMenuOpen ? "rotate-180" : ""
                  }`}
                />
              </button>

              {/* Model popover menu */}
              {isModelMenuOpen && (
                <div
                  ref={modelMenuRef}
                  className="absolute bottom-full mb-2 left-0 w-72 bg-white border border-zinc-200 rounded-xl shadow-lg p-1.5 z-50 flex flex-col gap-1"
                  role="listbox"
                >
                  <div className="px-2 py-1 text-[10px] font-semibold text-zinc-400 uppercase tracking-wider">
                    Fusion Models
                  </div>
                  {FUSION_MODELS.map((model) => {
                    const isSelected = model.id === currentModel.id;
                    return (
                      <button
                        key={model.id}
                        type="button"
                        onClick={() => {
                          onSelectModel?.(model.id);
                          setIsModelMenuOpen(false);
                        }}
                        className={`w-full flex items-center justify-between px-2.5 py-2 rounded-lg text-left text-xs transition-colors cursor-pointer ${
                          isSelected
                            ? "bg-zinc-100 text-zinc-900 font-medium"
                            : "text-zinc-700 hover:bg-zinc-50"
                        }`}
                        role="option"
                        aria-selected={isSelected}
                      >
                        <div className="flex flex-col min-w-0 pr-2">
                          <div className="flex items-center gap-1.5">
                            <span className="font-medium text-zinc-900 truncate">
                              {model.shortName}
                            </span>
                            <span className="px-1.5 py-0.5 text-[10px] font-normal bg-zinc-100 text-zinc-600 rounded border border-zinc-200/60 shrink-0">
                              {model.badge}
                            </span>
                          </div>
                          <span className="text-[11px] text-zinc-400 truncate mt-0.5">
                            {model.description}
                          </span>
                        </div>
                        {isSelected && (
                          <Check className="w-4 h-4 text-zinc-800 shrink-0" />
                        )}
                      </button>
                    );
                  })}
                </div>
              )}
            </div>
          </div>

          {/* Right: Mic button + circular Send button */}
          <div className="flex items-center gap-1.5">
            <button
              type="button"
              className="p-1.5 rounded-lg text-zinc-400 hover:text-zinc-600 hover:bg-zinc-100 transition-colors cursor-pointer"
              title="Voice input"
              aria-label="Voice input"
            >
              <Mic className="w-4 h-4" />
            </button>

            <button
              type="button"
              onClick={handleSend}
              disabled={!isGenerating && !text.trim()}
              className={`w-8 h-8 rounded-full flex items-center justify-center transition-all ${
                isGenerating
                  ? "bg-zinc-900 text-white hover:bg-zinc-800 cursor-pointer shadow-sm"
                  : text.trim()
                    ? "bg-zinc-900 text-white hover:bg-zinc-800 cursor-pointer shadow-sm"
                    : "bg-zinc-100 text-zinc-400 cursor-not-allowed"
              }`}
              aria-label={isGenerating ? "Cancel generation" : "Send prompt"}
            >
              {isGenerating ? (
                <Square className="w-3.5 h-3.5 fill-current" />
              ) : (
                <ArrowUp className="w-4 h-4 stroke-[2.5]" />
              )}
            </button>
          </div>
        </div>
      </div>

      {/* Sub-row below card */}
      <div className="flex items-center justify-between px-2 text-[11px] text-zinc-500">
        {/* Left: Lucide GitBranch icon + main ⌵ */}
        <div className="flex items-center gap-1 hover:text-zinc-700 cursor-pointer transition-colors">
          <GitBranch className="w-3.5 h-3.5" />
          <span>main</span>
          <ChevronDown className="w-3 h-3 text-zinc-400" />
        </div>

        {/* Right: Context limit gauge icon in text-zinc-500 */}
        <div
          className="flex items-center gap-1 hover:text-zinc-700 cursor-pointer transition-colors text-zinc-500"
          title="Context limit"
        >
          <Gauge className="w-3.5 h-3.5" />
        </div>
      </div>
    </div>
  );
}
