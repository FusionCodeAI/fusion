import React, { useState, useRef, useEffect, useMemo } from "react";
import {
  Paperclip,
  ChevronDown,
  Globe,
  ArrowUp,
  Square,
  Check,
  Search,
} from "lucide-react";
import {
  getAllAvailableModels,
  getFusionModel,
  type FusionModel,
} from "../models";

export interface ComposerProps {
  onSend: (text: string) => void;
  onCancel?: () => void;
  isGenerating?: boolean;
  selectedModel?: string;
  onSelectModel?: (modelId: string) => void;
  className?: string;
  placeholder?: string;
  autoFocus?: boolean;
  billingProfile?: string;
  effort?: "Low" | "Medium" | "High";
  onSelectEffort?: (effort: "Low" | "Medium" | "High") => void;
  onAttachFile?: () => void;
}

export function Composer({
  onSend,
  onCancel,
  isGenerating = false,
  selectedModel,
  onSelectModel,
  className = "",
  placeholder = "Ask to make changes, @mention files, reference #PRs, or run /commands.",
  autoFocus = false,
  effort: propEffort = "Low",
  onSelectEffort,
  onAttachFile,
}: ComposerProps) {
  const [text, setText] = useState("");
  const [isModelMenuOpen, setIsModelMenuOpen] = useState(false);
  const [isEffortMenuOpen, setIsEffortMenuOpen] = useState(false);
  const [mode, setMode] = useState<"plan" | "act">("act");
  const [effort, setEffort] = useState<"Low" | "Medium" | "High">(propEffort);
  const [modelSearch, setModelSearch] = useState("");
  const [availableModels] = useState<FusionModel[]>(() => getAllAvailableModels());

  const textareaRef = useRef<HTMLTextAreaElement>(null);
  const modelMenuRef = useRef<HTMLDivElement>(null);
  const modelButtonRef = useRef<HTMLButtonElement>(null);
  const effortMenuRef = useRef<HTMLDivElement>(null);
  const effortButtonRef = useRef<HTMLButtonElement>(null);

  const currentModel = getFusionModel(selectedModel);

  const filteredModels = useMemo(() => {
    if (!modelSearch.trim()) return availableModels;
    const q = modelSearch.toLowerCase().trim();
    return availableModels.filter(
      (m) =>
        m.name.toLowerCase().includes(q) ||
        m.id.toLowerCase().includes(q) ||
        m.provider.toLowerCase().includes(q) ||
        m.badge.toLowerCase().includes(q)
    );
  }, [availableModels, modelSearch]);

  // Auto-resize textarea height
  useEffect(() => {
    const textarea = textareaRef.current;
    if (!textarea) return;
    textarea.style.height = "auto";
    const scrollHeight = textarea.scrollHeight;
    const clampedHeight = Math.min(Math.max(scrollHeight, 44), 180);
    textarea.style.height = `${clampedHeight}px`;
  }, [text]);

  // Autofocus when requested
  useEffect(() => {
    if (autoFocus && textareaRef.current) {
      textareaRef.current.focus();
    }
  }, [autoFocus]);

  // Dismiss dropdowns when clicking outside
  useEffect(() => {
    const handleClickOutside = (event: MouseEvent) => {
      const target = event.target as Node;
      if (
        isModelMenuOpen &&
        modelMenuRef.current &&
        !modelMenuRef.current.contains(target) &&
        modelButtonRef.current &&
        !modelButtonRef.current.contains(target)
      ) {
        setIsModelMenuOpen(false);
      }
      if (
        isEffortMenuOpen &&
        effortMenuRef.current &&
        !effortMenuRef.current.contains(target) &&
        effortButtonRef.current &&
        !effortButtonRef.current.contains(target)
      ) {
        setIsEffortMenuOpen(false);
      }
    };
    document.addEventListener("mousedown", handleClickOutside);
    return () => {
      document.removeEventListener("mousedown", handleClickOutside);
    };
  }, [isModelMenuOpen, isEffortMenuOpen]);

  const handleKeyDown = (e: React.KeyboardEvent<HTMLTextAreaElement>) => {
    if (e.key === "Enter" && !e.shiftKey) {
      e.preventDefault();
      handleSubmit();
    }
  };

  const handleSubmit = () => {
    const trimmed = text.trim();
    if (!trimmed || isGenerating) return;
    onSend(trimmed);
    setText("");
    if (textareaRef.current) {
      textareaRef.current.style.height = "auto";
    }
  };

  const handleEffortChange = (val: "Low" | "Medium" | "High") => {
    setEffort(val);
    onSelectEffort?.(val);
    setIsEffortMenuOpen(false);
  };

  const canSubmit = Boolean(text.trim()) && !isGenerating;

  return (
    <div
      data-testid="composer-container"
      className={`w-full max-w-[760px] mx-auto rounded-2xl border border-zinc-200 bg-white shadow-sm focus-within:border-zinc-300 focus-within:shadow-md transition-all ${className}`}
    >
      {/* Upper Area: Textarea with bottom-right Send button */}
      <div className="relative p-3 pb-2">
        <textarea
          ref={textareaRef}
          data-testid="composer-textarea"
          value={text}
          onChange={(e) => setText(e.target.value)}
          onKeyDown={handleKeyDown}
          placeholder={placeholder}
          rows={1}
          className="w-full pr-10 bg-transparent text-[13px] text-zinc-900 placeholder-zinc-400 outline-none resize-none leading-relaxed min-h-[44px]"
        />

        {/* Send / Stop Button in bottom-right corner of input area */}
        <div className="absolute right-3 bottom-3">
          {isGenerating ? (
            <button
              type="button"
              data-testid="composer-cancel"
              onClick={onCancel}
              className="w-8 h-8 rounded-full bg-zinc-900 hover:bg-zinc-800 text-white flex items-center justify-center transition-all active:scale-95 cursor-pointer shadow-xs"
              title="Stop generation"
              aria-label="Stop generation"
            >
              <Square className="w-3.5 h-3.5 fill-current" />
            </button>
          ) : (
            <button
              type="button"
              data-testid="composer-send"
              onClick={handleSubmit}
              disabled={!canSubmit}
              className={`w-8 h-8 rounded-full flex items-center justify-center transition-all ${
                canSubmit
                  ? "bg-[#5100cd] hover:bg-[#4300a8] active:scale-95 text-white cursor-pointer shadow-sm ring-2 ring-purple-500/20"
                  : "bg-purple-100/70 text-purple-400 border border-purple-200/50 cursor-not-allowed opacity-75"
              }`}
              title={canSubmit ? "Send message (Enter)" : "Type a message to send"}
              aria-label="Send prompt"
            >
              <ArrowUp className="w-4 h-4 stroke-[2.5]" />
            </button>
          )}
        </div>
      </div>

      {/* Lower Toolbar: Paperclip | Mode Switcher | Model Selector | Effort */}
      <div className="border-t border-zinc-100 px-3 py-1.5 flex items-center justify-between text-xs text-zinc-600 select-none">
        <div className="flex items-center gap-2 relative">
          {/* Paperclip */}
          <button
            type="button"
            data-testid="composer-paperclip"
            onClick={onAttachFile}
            className="p-1 rounded text-zinc-400 hover:text-zinc-700 hover:bg-zinc-100 transition-colors cursor-pointer"
            title="Attach context or files"
            aria-label="Attach context or files"
          >
            <Paperclip className="w-3.5 h-3.5" />
          </button>

          {/* Plan | Act Mode Toggle matching Cline */}
          <div
            data-testid="composer-mode-switcher"
            className="inline-flex items-center rounded-lg border border-zinc-200 bg-zinc-100/90 p-0.5 text-[11px] font-medium select-none"
          >
            <button
              type="button"
              onClick={() => setMode("plan")}
              className={`px-2 py-0.5 rounded-md transition-all cursor-pointer ${
                mode === "plan"
                  ? "bg-amber-100 text-amber-900 shadow-2xs font-semibold"
                  : "text-zinc-500 hover:text-zinc-800"
              }`}
            >
              Plan
            </button>
            <button
              type="button"
              onClick={() => setMode("act")}
              className={`px-2 py-0.5 rounded-md transition-all cursor-pointer ${
                mode === "act"
                  ? "bg-purple-100 text-[#5100cd] shadow-2xs font-semibold"
                  : "text-zinc-500 hover:text-zinc-800"
              }`}
            >
              Act
            </button>
          </div>

          <span className="text-zinc-200">|</span>

          {/* Model Selector Dropdown with Search & Custom Model ID */}
          <div className="relative">
            <button
              ref={modelButtonRef}
              type="button"
              data-testid="composer-model-picker-toggle"
              onClick={() => setIsModelMenuOpen((prev) => !prev)}
              className="flex items-center gap-1 text-[12px] font-medium text-zinc-800 hover:text-zinc-950 px-1 py-0.5 rounded hover:bg-zinc-100 transition-colors cursor-pointer"
              aria-expanded={isModelMenuOpen}
              aria-haspopup="listbox"
            >
              <span>{currentModel.shortName || currentModel.name}</span>
              <ChevronDown className="w-3 h-3 text-zinc-400" />
            </button>

            {isModelMenuOpen && (
              <div
                ref={modelMenuRef}
                data-testid="composer-model-menu"
                className="absolute bottom-full left-0 mb-2 w-80 bg-white rounded-xl shadow-xl border border-zinc-200/90 py-1.5 z-50 animate-in fade-in zoom-in-95 duration-100 flex flex-col"
                role="listbox"
              >
                {/* Search Header matching Cline ModelPicker */}
                <div className="px-2.5 pt-1 pb-2 border-b border-zinc-100">
                  <div className="flex items-center gap-1.5 px-2.5 py-1 bg-zinc-50 border border-zinc-200/80 rounded-lg">
                    <Search className="w-3.5 h-3.5 text-zinc-400 shrink-0" />
                    <input
                      type="text"
                      data-testid="composer-model-search"
                      placeholder="Search models or providers..."
                      value={modelSearch}
                      onChange={(e) => setModelSearch(e.target.value)}
                      className="w-full bg-transparent text-xs text-zinc-800 placeholder-zinc-400 outline-none"
                      autoFocus
                    />
                  </div>
                </div>

                {/* Scrollable Model List */}
                <div className="max-h-60 overflow-y-auto py-1">
                  {filteredModels.length === 0 ? (
                    <div className="px-3 py-3 text-center text-xs text-zinc-400">
                      No matching models found.
                    </div>
                  ) : (
                    filteredModels.map((model) => {
                      const isSelected = model.id === currentModel.id;
                      return (
                        <button
                          key={model.id}
                          type="button"
                          data-testid={`composer-model-option-${model.id}`}
                          onClick={() => {
                            onSelectModel?.(model.id);
                            setIsModelMenuOpen(false);
                          }}
                          className={`w-full px-3 py-2 text-left flex items-start justify-between hover:bg-zinc-50 transition-colors cursor-pointer ${
                            isSelected ? "bg-zinc-50/80" : ""
                          }`}
                          role="option"
                          aria-selected={isSelected}
                        >
                          <div className="min-w-0 pr-2">
                            <div className="flex items-center gap-1.5 flex-wrap">
                              <span className="text-xs font-medium text-zinc-900 truncate">
                                {model.name}
                              </span>
                              {model.badge && (
                                <span className="text-[10px] px-1.5 py-0.2 rounded bg-zinc-100 text-zinc-600 font-normal">
                                  {model.badge}
                                </span>
                              )}
                              <span className="text-[10px] px-1.5 py-0.2 rounded uppercase tracking-wider text-zinc-400 font-mono">
                                {model.provider}
                              </span>
                            </div>
                            {model.description && (
                              <div className="text-[11px] text-zinc-500 mt-0.5 truncate">
                                {model.description}
                              </div>
                            )}
                          </div>
                          {isSelected && (
                            <Check className="w-3.5 h-3.5 text-[#5100cd] shrink-0 mt-0.5" />
                          )}
                        </button>
                      );
                    })
                  )}
                </div>
              </div>
            )}
          </div>

          <span className="text-zinc-200">|</span>

          {/* Reasoning Effort Dropdown */}
          <div className="relative">
            <button
              ref={effortButtonRef}
              type="button"
              data-testid="composer-effort"
              onClick={() => setIsEffortMenuOpen((prev) => !prev)}
              className="flex items-center gap-1 text-[12px] text-zinc-600 hover:text-zinc-900 px-1 py-0.5 rounded hover:bg-zinc-100 transition-colors cursor-pointer"
              aria-expanded={isEffortMenuOpen}
            >
              <Globe className="w-3.5 h-3.5 text-zinc-400" />
              <span>{effort}</span>
              <ChevronDown className="w-3 h-3 text-zinc-400" />
            </button>

            {isEffortMenuOpen && (
              <div
                ref={effortMenuRef}
                data-testid="composer-effort-menu"
                className="absolute bottom-full left-0 mb-2 w-36 bg-white rounded-xl shadow-lg border border-zinc-200/90 py-1 z-50 animate-in fade-in duration-100"
              >
                <div className="px-2.5 py-1 text-[10px] font-semibold text-zinc-400 uppercase tracking-wider">
                  Thinking Effort
                </div>
                {(["Low", "Medium", "High"] as const).map((lvl) => (
                  <button
                    key={lvl}
                    type="button"
                    onClick={() => handleEffortChange(lvl)}
                    className={`w-full px-2.5 py-1.5 text-left text-xs flex items-center justify-between hover:bg-zinc-50 cursor-pointer ${
                      effort === lvl ? "font-medium text-[#5100cd]" : "text-zinc-700"
                    }`}
                  >
                    <span>{lvl}</span>
                    {effort === lvl && <Check className="w-3 h-3 text-[#5100cd]" />}
                  </button>
                ))}
              </div>
            )}
          </div>
        </div>
      </div>
    </div>
  );
}

export default Composer;
