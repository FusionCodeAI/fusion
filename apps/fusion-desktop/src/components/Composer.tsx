import React, { useState, useRef, useEffect, useMemo } from "react";
import {
  Paperclip,
  ChevronDown,
  Brain,
  ArrowUp,
  Square,
  Check,
  Search,
  X,
  Image as ImageIcon,
} from "lucide-react";
import {
  getAllAvailableModels,
  getFusionModel,
  type FusionModel,
} from "../models";
import type { ChatImageAttachment } from "../types";
import { filterSkills, type SkillItem } from "../lib/skills-catalog";
import { filterMentions, type MentionItem } from "../lib/mentions-catalog";
import { SlashCommandMenu } from "./SlashCommandMenu";
import { AtMentionMenu } from "./AtMentionMenu";

export interface ComposerProps {
  onSend: (text: string, images?: ChatImageAttachment[]) => void;
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
  workspaceFiles?: string[];
  workspaceEntries?: Array<{ path: string; name?: string; is_dir?: boolean } | string>;
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
  workspaceFiles = [],
  workspaceEntries,
}: ComposerProps) {
  const [text, setText] = useState("");
  const [attachedImages, setAttachedImages] = useState<ChatImageAttachment[]>([]);
  const [isDragging, setIsDragging] = useState(false);

  // Popover menus state
  const [isSlashMenuOpen, setIsSlashMenuOpen] = useState(false);
  const [slashQuery, setSlashQuery] = useState("");
  const [slashSelectedIndex, setSlashSelectedIndex] = useState(0);

  const [isMentionMenuOpen, setIsMentionMenuOpen] = useState(false);
  const [mentionQuery, setMentionQuery] = useState("");
  const [mentionSelectedIndex, setMentionSelectedIndex] = useState(0);

  // Model & Effort state
  const [isModelMenuOpen, setIsModelMenuOpen] = useState(false);
  const [isEffortMenuOpen, setIsEffortMenuOpen] = useState(false);
  const [mode, setMode] = useState<"plan" | "agent">("agent");
  const [effort, setEffort] = useState<"Low" | "Medium" | "High">(propEffort);
  const [modelSearch, setModelSearch] = useState("");
  const [availableModels] = useState<FusionModel[]>(() => getAllAvailableModels());

  const textareaRef = useRef<HTMLTextAreaElement>(null);
  const fileInputRef = useRef<HTMLInputElement>(null);
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

  const filteredSkills = useMemo(() => {
    return filterSkills(slashQuery);
  }, [slashQuery]);

  const filteredMentions = useMemo(() => {
    return filterMentions(mentionQuery, workspaceEntries || workspaceFiles);
  }, [mentionQuery, workspaceEntries, workspaceFiles]);
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

  // Helper to attach an image File
  const attachImageFile = (file: File) => {
    if (!file.type.startsWith("image/")) return;
    const reader = new FileReader();
    reader.onload = (loadEvent) => {
      const dataUrl = loadEvent.target?.result as string;
      if (dataUrl) {
        const newAttachment: ChatImageAttachment = {
          id: `img-${Date.now()}-${Math.random().toString(36).slice(2, 7)}`,
          url: dataUrl,
          name: file.name || `Pasted Image ${attachedImages.length + 1}`,
          size: file.size,
        };
        setAttachedImages((prev) => [...prev, newAttachment]);
      }
    };
    reader.readAsDataURL(file);
  };

  // Clipboard Paste listener for images
  const handlePaste = (e: React.ClipboardEvent<HTMLTextAreaElement>) => {
    const items = e.clipboardData?.items;
    if (!items) return;

    let hasImage = false;
    for (let i = 0; i < items.length; i++) {
      const item = items[i];
      if (item.type.indexOf("image") !== -1) {
        hasImage = true;
        const file = item.getAsFile();
        if (file) {
          attachImageFile(file);
        }
      }
    }

    if (hasImage) {
      e.preventDefault();
    }
  };

  // Drag & drop handlers
  const handleDragOver = (e: React.DragEvent) => {
    e.preventDefault();
    setIsDragging(true);
  };

  const handleDragLeave = () => {
    setIsDragging(false);
  };

  const handleDrop = (e: React.DragEvent) => {
    e.preventDefault();
    setIsDragging(false);
    const files = e.dataTransfer?.files;
    if (files) {
      for (let i = 0; i < files.length; i++) {
        if (files[i].type.startsWith("image/")) {
          attachImageFile(files[i]);
        }
      }
    }
  };

  const removeAttachedImage = (id: string) => {
    setAttachedImages((prev) => prev.filter((img) => img.id !== id));
  };

  // Detect slash command or @ mention triggers on change
  const handleTextChange = (e: React.ChangeEvent<HTMLTextAreaElement>) => {
    const val = e.target.value;
    const cursorPos = e.target.selectionStart;
    setText(val);

    const beforeCursor = val.slice(0, cursorPos);

    // 1. Check for slash command at start or after newline
    const slashMatch = beforeCursor.match(/(?:^|\n)\/([a-zA-Z0-9:_-]*)$/);
    if (slashMatch) {
      setIsSlashMenuOpen(true);
      setSlashQuery(slashMatch[1] || "");
      setSlashSelectedIndex(0);
      setIsMentionMenuOpen(false);
      return;
    } else {
      setIsSlashMenuOpen(false);
    }

    // 2. Check for @ mention
    const atMatch = beforeCursor.match(/(?:^|\s)@([a-zA-Z0-9_./-]*)$/);
    if (atMatch) {
      setIsMentionMenuOpen(true);
      setMentionQuery(atMatch[1] || "");
      setMentionSelectedIndex(0);
      return;
    } else {
      setIsMentionMenuOpen(false);
    }
  };

  // Select a slash command
  const handleSelectSkill = (skill: SkillItem) => {
    setIsSlashMenuOpen(false);

    if (skill.trigger === "/plan") {
      setMode("plan");
      setText((prev) => prev.replace(/(?:^|\n)\/[a-zA-Z0-9:_-]*$/, ""));
      return;
    }

    if (skill.trigger === "/act" || skill.trigger === "/agent") {
      setMode("agent");
      setText((prev) => prev.replace(/(?:^|\n)\/[a-zA-Z0-9:_-]*$/, ""));
      return;
    }

    // Replace the slash token with the skill trigger
    setText((prev) => {
      const replaced = prev.replace(/(?:^|\n)\/[a-zA-Z0-9:_-]*$/, `${skill.trigger} `);
      return replaced;
    });

    if (textareaRef.current) {
      textareaRef.current.focus();
    }
  };

  // Select an @ mention
  const handleSelectMention = (mention: MentionItem) => {
    setIsMentionMenuOpen(false);

    setText((prev) => {
      const replaced = prev.replace(/(?:^|\s)@[a-zA-Z0-9_./-]*$/, ` ${mention.value} `);
      return replaced.trimStart();
    });

    if (textareaRef.current) {
      textareaRef.current.focus();
    }
  };

  const handleKeyDown = (e: React.KeyboardEvent<HTMLTextAreaElement>) => {
    // Keyboard navigation in Slash Command Menu
    if (isSlashMenuOpen && filteredSkills.length > 0) {
      if (e.key === "ArrowDown") {
        e.preventDefault();
        setSlashSelectedIndex((prev) => (prev + 1) % filteredSkills.length);
        return;
      }
      if (e.key === "ArrowUp") {
        e.preventDefault();
        setSlashSelectedIndex((prev) => (prev - 1 + filteredSkills.length) % filteredSkills.length);
        return;
      }
      if (e.key === "Enter" || e.key === "Tab") {
        e.preventDefault();
        const target = filteredSkills[slashSelectedIndex];
        if (target) {
          handleSelectSkill(target);
        }
        return;
      }
      if (e.key === "Escape") {
        e.preventDefault();
        setIsSlashMenuOpen(false);
        return;
      }
    }

    // Keyboard navigation in @ Mention Menu
    if (isMentionMenuOpen && filteredMentions.length > 0) {
      if (e.key === "ArrowDown") {
        e.preventDefault();
        setMentionSelectedIndex((prev) => (prev + 1) % filteredMentions.length);
        return;
      }
      if (e.key === "ArrowUp") {
        e.preventDefault();
        setMentionSelectedIndex((prev) => (prev - 1 + filteredMentions.length) % filteredMentions.length);
        return;
      }
      if (e.key === "Enter" || e.key === "Tab") {
        e.preventDefault();
        const target = filteredMentions[mentionSelectedIndex];
        if (target) {
          handleSelectMention(target);
        }
        return;
      }
      if (e.key === "Escape") {
        e.preventDefault();
        setIsMentionMenuOpen(false);
        return;
      }
    }

    // Enter without Shift submits
    if (e.key === "Enter" && !e.shiftKey) {
      e.preventDefault();
      handleSubmit();
    }
  };

  const handleSubmit = () => {
    const trimmed = text.trim();
    if ((!trimmed && attachedImages.length === 0) || isGenerating) return;
    onSend(trimmed, attachedImages.length > 0 ? attachedImages : undefined);
    setText("");
    setAttachedImages([]);
    setIsSlashMenuOpen(false);
    setIsMentionMenuOpen(false);
    if (textareaRef.current) {
      textareaRef.current.style.height = "auto";
    }
  };

  const handleEffortChange = (val: "Low" | "Medium" | "High") => {
    setEffort(val);
    onSelectEffort?.(val);
    setIsEffortMenuOpen(false);
  };

  const canSubmit = (Boolean(text.trim()) || attachedImages.length > 0) && !isGenerating;

  return (
    <div
      data-testid="composer-container"
      onDragOver={handleDragOver}
      onDragLeave={handleDragLeave}
      onDrop={handleDrop}
      className={`relative w-full max-w-[760px] mx-auto rounded-2xl border ${
        isDragging ? "border-zinc-900 ring-2 ring-zinc-300 bg-zinc-50" : "border-zinc-200 bg-white"
      } shadow-sm focus-within:border-zinc-300 focus-within:shadow-md transition-all ${className}`}
    >
      <input
        ref={fileInputRef}
        type="file"
        accept="image/*,.png,.jpg,.jpeg,.webp,.gif"
        multiple
        className="hidden"
        onChange={(e) => {
          const files = e.target.files;
          if (files) {
            for (let i = 0; i < files.length; i++) {
              attachImageFile(files[i]);
            }
          }
          e.target.value = "";
        }}
      />

      {/* Floating Slash Command / Skills Autocomplete Menu */}
      {isSlashMenuOpen && (
        <SlashCommandMenu
          skills={filteredSkills}
          selectedIndex={slashSelectedIndex}
          onSelect={handleSelectSkill}
          onClose={() => setIsSlashMenuOpen(false)}
        />
      )}

      {/* Floating @ Mentions / Context Autocomplete Menu */}
      {isMentionMenuOpen && (
        <AtMentionMenu
          mentions={filteredMentions}
          selectedIndex={mentionSelectedIndex}
          onSelect={handleSelectMention}
          onClose={() => setIsMentionMenuOpen(false)}
        />
      )}

      {/* Attached Images Tray */}
      {attachedImages.length > 0 && (
        <div
          data-testid="composer-attached-images"
          className="px-3 pt-2.5 pb-1 flex flex-wrap gap-2 items-center border-b border-zinc-100"
        >
          {attachedImages.map((img) => (
            <div
              key={img.id}
              data-testid={`attached-image-${img.id}`}
              className="group relative flex items-center gap-2 px-2 py-1 rounded-xl bg-zinc-100/90 border border-zinc-200/80 text-xs text-zinc-700 shadow-2xs pr-7"
            >
              <img
                src={img.url}
                alt={img.name || "Attachment"}
                className="w-7 h-7 rounded-lg object-cover border border-zinc-300/60 bg-white"
              />
              <div className="flex flex-col min-w-0">
                <span className="font-medium max-w-[140px] truncate text-[11px] text-zinc-900">
                  {img.name || "Pasted image"}
                </span>
                {img.size ? (
                  <span className="text-[10px] text-zinc-400">
                    {(img.size / 1024).toFixed(0)} KB
                  </span>
                ) : null}
              </div>
              <button
                type="button"
                onClick={() => removeAttachedImage(img.id)}
                className="absolute right-1.5 p-0.5 rounded-full hover:bg-zinc-200 text-zinc-400 hover:text-zinc-700 transition-colors cursor-pointer"
                title="Remove image"
                aria-label="Remove image"
              >
                <X className="w-3.5 h-3.5" />
              </button>
            </div>
          ))}
        </div>
      )}

      {/* Upper Area: Textarea with bottom-right Send button */}
      <div className="relative p-3 pb-2">
        <textarea
          ref={textareaRef}
          data-testid="composer-textarea"
          value={text}
          onChange={handleTextChange}
          onKeyDown={handleKeyDown}
          onPaste={handlePaste}
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
                  ? "bg-zinc-900 hover:bg-zinc-800 active:scale-95 text-white cursor-pointer shadow-xs"
                  : "bg-zinc-100 text-zinc-300 border border-zinc-200/50 cursor-not-allowed"
              }`}
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
            onClick={() => {
              if (onAttachFile) {
                onAttachFile();
              } else if (fileInputRef.current) {
                fileInputRef.current.click();
              }
            }}
            className="p-1 rounded text-zinc-400 hover:text-zinc-700 hover:bg-zinc-100 transition-colors cursor-pointer"
            title="Attach context or files"
            aria-label="Attach context or files"
          >
            <Paperclip className="w-3.5 h-3.5" />
          </button>

          {/* Plan | Agent Mode Toggle */}
          <div
            data-testid="composer-mode-switcher"
            className="inline-flex items-center rounded-lg border border-zinc-200 bg-zinc-100/90 p-0.5 text-[11px] font-medium select-none"
          >
            <button
              type="button"
              onClick={() => setMode("plan")}
              className={`px-2 py-0.5 rounded-md transition-all cursor-pointer ${
                mode === "plan"
                  ? "bg-white text-zinc-900 shadow-xs font-semibold"
                  : "text-zinc-500 hover:text-zinc-800"
              }`}
            >
              Plan
            </button>
            <button
              onClick={() => setMode("agent")}
              className={`px-2 py-0.5 rounded-md transition-all cursor-pointer ${
                mode === "agent"
                  ? "bg-white text-zinc-900 shadow-xs font-semibold"
                  : "text-zinc-500 hover:text-zinc-800"
              }`}
            >
              Agent
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
              <span>{currentModel.shortName}</span>
              <ChevronDown className="w-3 h-3 text-zinc-400" />
            </button>

            {isModelMenuOpen && (
              <div
                ref={modelMenuRef}
                data-testid="composer-model-menu"
                className="absolute bottom-full left-0 mb-2 w-72 max-h-72 overflow-y-auto rounded-xl border border-zinc-200 bg-white p-1.5 shadow-xl z-50 select-none flex flex-col gap-1"
              >
                {/* Search Bar inside Model Menu */}
                <div className="relative px-1 pt-1 pb-1">
                  <Search className="absolute left-3 top-2.5 w-3.5 h-3.5 text-zinc-400" />
                  <input
                    type="text"
                    data-testid="composer-model-search-input"
                    value={modelSearch}
                    onChange={(e) => setModelSearch(e.target.value)}
                    placeholder="Search model or enter ID..."
                    className="w-full pl-7 pr-2 py-1 text-xs rounded-md bg-zinc-100 border border-zinc-200/60 placeholder-zinc-400 text-zinc-900 outline-none focus:border-zinc-300"
                    autoFocus
                  />
                </div>

                <div className="px-2 py-1 text-[10px] font-medium text-zinc-400 uppercase tracking-wider">
                  Available Models
                </div>

                <div className="space-y-0.5 overflow-y-auto max-h-48">
                  {filteredModels.map((model) => {
                    const isSelected = model.id === currentModel.id;
                    return (
                      <button
                        key={model.id}
                        type="button"
                        data-testid={`composer-model-option-${model.id}`}
                        onClick={() => {
                          onSelectModel?.(model.id);
                          setIsModelMenuOpen(false);
                          setModelSearch("");
                        }}
                        className={`w-full text-left px-2 py-1.5 rounded-lg flex items-center justify-between text-xs transition-colors cursor-pointer ${
                          isSelected
                            ? "bg-zinc-100 text-zinc-900 font-medium"
                            : "hover:bg-zinc-50 text-zinc-700"
                        }`}
                      >
                        <div className="flex flex-col min-w-0 pr-2">
                          <span className="truncate">{model.name}</span>
                          <span className="text-[10px] text-zinc-400 truncate">
                            {model.provider}
                          </span>
                        </div>
                        {isSelected && (
                          <Check className="w-3.5 h-3.5 text-zinc-900 shrink-0" />
                        )}
                      </button>
                    );
                  })}

                  {/* Custom Model ID Entry Option if not matching exactly */}
                  {modelSearch.trim() &&
                    !availableModels.some(
                      (m) => m.id.toLowerCase() === modelSearch.toLowerCase().trim()
                    ) && (
                      <button
                        type="button"
                        data-testid="composer-model-option-custom"
                        onClick={() => {
                          onSelectModel?.(modelSearch.trim());
                          setIsModelMenuOpen(false);
                          setModelSearch("");
                        }}
                        className="w-full text-left px-2 py-1.5 rounded-lg flex items-center justify-between text-xs bg-zinc-50 hover:bg-zinc-100 text-zinc-800 transition-colors cursor-pointer border border-dashed border-zinc-300 mt-1"
                      >
                        <div className="flex flex-col min-w-0">
                          <span className="font-medium truncate">
                            Use custom model: "{modelSearch.trim()}"
                          </span>
                          <span className="text-[10px] text-zinc-400">
                            Arbitrary provider model string
                          </span>
                        </div>
                      </button>
                    )}
                </div>
              </div>
            )}
          </div>

          <span className="text-zinc-200">|</span>

          {/* Effort Selector */}
          <div className="relative">
            <button
              ref={effortButtonRef}
              type="button"
              data-testid="composer-effort"
              onClick={() => setIsEffortMenuOpen((prev) => !prev)}
              className="flex items-center gap-1.5 text-zinc-700 hover:text-zinc-950 px-1 py-0.5 rounded hover:bg-zinc-100 transition-colors cursor-pointer"
              aria-expanded={isEffortMenuOpen}
              aria-haspopup="listbox"
            >
              <Brain className="w-3.5 h-3.5 text-zinc-400" />
              <span className="font-medium text-[12px]">{effort}</span>
              <ChevronDown className="w-3 h-3 text-zinc-400" />
            </button>

            {isEffortMenuOpen && (
              <div
                ref={effortMenuRef}
                data-testid="composer-effort-menu"
                className="absolute bottom-full left-0 mb-2 w-32 rounded-xl border border-zinc-200 bg-white p-1 shadow-xl z-50 select-none flex flex-col gap-0.5 text-xs"
              >
                {(["Low", "Medium", "High"] as const).map((level) => {
                  const isSelected = level === effort;
                  return (
                    <button
                      key={level}
                      type="button"
                      data-testid={`composer-effort-option-${level}`}
                      onClick={() => handleEffortChange(level)}
                      className={`w-full text-left px-2 py-1.5 rounded-lg flex items-center justify-between transition-colors cursor-pointer ${
                        isSelected
                          ? "bg-zinc-100 text-zinc-900 font-medium"
                          : "hover:bg-zinc-50 text-zinc-700"
                      }`}
                    >
                      <span>{level}</span>
                      {isSelected && (
                        <Check className="w-3.5 h-3.5 text-zinc-900" />
                      )}
                    </button>
                  );
                })}
              </div>
            )}
          </div>
        </div>
      </div>
    </div>
  );
}

export default Composer;
