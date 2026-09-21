import React, { useEffect, useRef } from "react";
import {
  Sparkles,
  Compass,
  Zap,
  Trash2,
  FileCheck,
  Cpu,
  HelpCircle,
  Command,
  BookOpen,
} from "lucide-react";
import type { SkillItem } from "../lib/skills-catalog";

export interface SlashCommandMenuProps {
  skills: SkillItem[];
  selectedIndex: number;
  onSelect: (skill: SkillItem) => void;
  onClose: () => void;
  className?: string;
}

function renderSkillIcon(iconName?: string) {
  switch (iconName) {
    case "Sparkles":
      return <Sparkles className="w-3.5 h-3.5 text-zinc-500" />;
    case "Compass":
      return <Compass className="w-3.5 h-3.5 text-zinc-500" />;
    case "Zap":
      return <Zap className="w-3.5 h-3.5 text-zinc-500" />;
    case "Trash2":
      return <Trash2 className="w-3.5 h-3.5 text-zinc-500" />;
    case "FileCheck":
      return <FileCheck className="w-3.5 h-3.5 text-zinc-500" />;
    case "Cpu":
      return <Cpu className="w-3.5 h-3.5 text-zinc-500" />;
    case "HelpCircle":
      return <HelpCircle className="w-3.5 h-3.5 text-zinc-500" />;
    default:
      return <Command className="w-3.5 h-3.5 text-zinc-400" />;
  }
}

export function SlashCommandMenu({
  skills,
  selectedIndex,
  onSelect,
  onClose,
  className = "",
}: SlashCommandMenuProps) {
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

  if (skills.length === 0) {
    return null;
  }

  return (
    <div
      ref={containerRef}
      data-testid="slash-command-menu"
      className={`absolute bottom-full left-0 right-0 mb-2 max-h-64 overflow-y-auto bg-white rounded-xl border border-zinc-200 shadow-lg z-50 p-1 select-none ${className}`}
    >
      <div className="space-y-0.5">
        {skills.map((skill, index) => {
          const isSelected = index === selectedIndex;

          return (
            <button
              key={skill.id}
              ref={isSelected ? activeItemRef : null}
              type="button"
              data-testid={`slash-command-item-${skill.name}`}
              onClick={() => onSelect(skill)}
              onMouseDown={(e) => {
                e.preventDefault();
                onSelect(skill);
              }}
              className={`w-full text-left px-2.5 py-1.5 rounded-lg flex items-center justify-between gap-2 text-xs transition-colors cursor-pointer ${
                isSelected
                  ? "bg-zinc-100 text-zinc-900 font-medium"
                  : "hover:bg-zinc-50 text-zinc-700 font-normal"
              }`}
            >
              <div className="flex items-center gap-2 min-w-0">
                <div className="shrink-0 text-zinc-500">
                  {renderSkillIcon(skill.icon)}
                </div>
                <span className="font-mono font-medium text-zinc-900 text-xs">
                  {skill.trigger}
                </span>
                <span className="text-zinc-400 text-xs truncate">
                  {skill.description}
                </span>
              </div>

              <div className="shrink-0 text-[11px] text-zinc-400 font-mono">
                {skill.category}
              </div>
            </button>
          );
        })}
      </div>
    </div>
  );
}

export default SlashCommandMenu;
