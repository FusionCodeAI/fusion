import React, { useState, useEffect } from "react";
import { X, BookOpen, Wrench, Sparkles, Server, Check } from "lucide-react";

export interface CustomizeModalProps {
  isOpen: boolean;
  onClose: () => void;
}

export type CustomizeTab = "rules" | "skills" | "tools" | "mcp";

const DEFAULT_RULES = `# Project Rules & System Instructions
- Always prioritize correctness and maintainability.
- Write clean, self-documenting code with TypeScript strict types.
- Follow existing repository conventions and test with bun test.
`;

const BUILTIN_TOOLS = [
  { id: "read", name: "File Reader", desc: "Inspect files, directories, and codebases safely." },
  { id: "edit", name: "Line Editor", desc: "Make precision surgical edits to existing source files." },
  { id: "write", name: "File Writer", desc: "Create new files, configurations, and test suites." },
  { id: "bash", name: "Terminal Execution", desc: "Run builds, tests, git commands, and shell pipelines." },
  { id: "grep", name: "Regex Search", desc: "Ultra-fast regex code search across the workspace." },
  { id: "glob", name: "Glob Pattern Match", desc: "Pattern match files, directories, and workspace paths." },
  { id: "web_search", name: "Web Search", desc: "Fetch up-to-date documentation and online resources." },
];

const BUILTIN_SKILLS = [
  { id: "brainstorming", name: "Brainstorming", desc: "Explores user intent and design specs before code implementation." },
  { id: "systematic-debugging", name: "Systematic Debugging", desc: "Root-cause investigation before proposing or applying fixes." },
  { id: "test-driven-development", name: "TDD Loop", desc: "Red-Green-Refactor test suites defending invariants." },
  { id: "frontend-design", name: "Frontend Design", desc: "Tailwind v4 and React UI refinement with high aesthetic taste." },
  { id: "verification-before-completion", name: "Verification", desc: "Executes test commands and evidence capture before completion." },
];

export function CustomizeModal({ isOpen, onClose }: CustomizeModalProps) {
  const [activeTab, setActiveTab] = useState<CustomizeTab>("rules");
  const [rulesText, setRulesText] = useState<string>(() => {
    if (typeof window !== "undefined" && window.localStorage) {
      return window.localStorage.getItem("fusion_desktop_custom_rules") || DEFAULT_RULES;
    }
    return DEFAULT_RULES;
  });
  const [savedSuccess, setSavedSuccess] = useState(false);

  useEffect(() => {
    const handleKeyDown = (e: KeyboardEvent) => {
      if (e.key === "Escape" && isOpen) {
        onClose();
      }
    };
    window.addEventListener("keydown", handleKeyDown);
    return () => window.removeEventListener("keydown", handleKeyDown);
  }, [isOpen, onClose]);

  if (!isOpen) return null;

  const handleSaveRules = () => {
    if (typeof window !== "undefined" && window.localStorage) {
      window.localStorage.setItem("fusion_desktop_custom_rules", rulesText);
    }
    setSavedSuccess(true);
    setTimeout(() => setSavedSuccess(false), 2000);
  };

  return (
    <div
      data-testid="customize-modal-overlay"
      className="fixed inset-0 z-50 bg-black/40 backdrop-blur-xs flex items-center justify-center p-4 animate-in fade-in duration-150"
      onClick={(e) => {
        if (e.target === e.currentTarget) onClose();
      }}
    >
      <div
        data-testid="customize-modal"
        className="w-full max-w-2xl bg-white rounded-2xl border border-zinc-200 shadow-2xl overflow-hidden flex flex-col max-h-[85vh] animate-in zoom-in-95 duration-150"
      >
        {/* Header */}
        <div className="px-5 py-4 border-b border-zinc-100 flex items-center justify-between select-none">
          <div>
            <h2 className="text-base font-semibold text-zinc-900">Customize Fusion</h2>
            <p className="text-xs text-zinc-500 mt-0.5">
              Configure system instructions, agent skills, tools, and MCP servers.
            </p>
          </div>
          <button
            type="button"
            data-testid="customize-close-btn"
            onClick={onClose}
            className="p-1 rounded-lg text-zinc-400 hover:text-zinc-700 hover:bg-zinc-100 transition-colors cursor-pointer"
            aria-label="Close"
          >
            <X className="w-4 h-4" />
          </button>
        </div>

        {/* Tab Strip matching Cline CustomizeView */}
        <div className="px-5 pt-2 border-b border-zinc-100 flex items-center gap-1 select-none">
          <button
            type="button"
            data-testid="customize-tab-rules"
            onClick={() => setActiveTab("rules")}
            className={`inline-flex items-center gap-1.5 px-3 py-2 text-xs font-medium border-b-2 transition-all cursor-pointer ${
              activeTab === "rules"
                ? "border-[#5100cd] text-[#5100cd]"
                : "border-transparent text-zinc-500 hover:text-zinc-800"
            }`}
          >
            <BookOpen className="w-3.5 h-3.5" />
            <span>Rules</span>
          </button>

          <button
            type="button"
            data-testid="customize-tab-skills"
            onClick={() => setActiveTab("skills")}
            className={`inline-flex items-center gap-1.5 px-3 py-2 text-xs font-medium border-b-2 transition-all cursor-pointer ${
              activeTab === "skills"
                ? "border-[#5100cd] text-[#5100cd]"
                : "border-transparent text-zinc-500 hover:text-zinc-800"
            }`}
          >
            <Sparkles className="w-3.5 h-3.5" />
            <span>Skills ({BUILTIN_SKILLS.length})</span>
          </button>

          <button
            type="button"
            data-testid="customize-tab-tools"
            onClick={() => setActiveTab("tools")}
            className={`inline-flex items-center gap-1.5 px-3 py-2 text-xs font-medium border-b-2 transition-all cursor-pointer ${
              activeTab === "tools"
                ? "border-[#5100cd] text-[#5100cd]"
                : "border-transparent text-zinc-500 hover:text-zinc-800"
            }`}
          >
            <Wrench className="w-3.5 h-3.5" />
            <span>Tools ({BUILTIN_TOOLS.length})</span>
          </button>

          <button
            type="button"
            data-testid="customize-tab-mcp"
            onClick={() => setActiveTab("mcp")}
            className={`inline-flex items-center gap-1.5 px-3 py-2 text-xs font-medium border-b-2 transition-all cursor-pointer ${
              activeTab === "mcp"
                ? "border-[#5100cd] text-[#5100cd]"
                : "border-transparent text-zinc-500 hover:text-zinc-800"
            }`}
          >
            <Server className="w-3.5 h-3.5" />
            <span>MCP</span>
          </button>
        </div>

        {/* Body Content */}
        <div className="flex-1 overflow-y-auto p-5">
          {activeTab === "rules" && (
            <div className="space-y-3">
              <div className="text-xs text-zinc-500">
                Custom instructions are injected into every prompt turn across all sessions.
              </div>
              <textarea
                data-testid="customize-rules-textarea"
                rows={10}
                value={rulesText}
                onChange={(e) => setRulesText(e.target.value)}
                placeholder="Enter custom instructions or project guidelines..."
                className="w-full p-3 font-mono text-xs text-zinc-900 bg-zinc-50 border border-zinc-200 rounded-xl outline-none focus:border-[#5100cd] leading-relaxed resize-y"
              />
              <div className="flex items-center justify-between">
                <span className="text-[11px] text-zinc-400">
                  Stored locally and synced with Fusion agent prompt context.
                </span>
                <button
                  type="button"
                  data-testid="customize-rules-save"
                  onClick={handleSaveRules}
                  className="inline-flex items-center gap-1.5 px-3 py-1.5 text-xs font-medium bg-[#5100cd] hover:bg-[#4300a8] text-white rounded-lg transition-colors cursor-pointer shadow-xs"
                >
                  {savedSuccess ? (
                    <>
                      <Check className="w-3.5 h-3.5 text-emerald-300" />
                      <span>Saved!</span>
                    </>
                  ) : (
                    <span>Save Rules</span>
                  )}
                </button>
              </div>
            </div>
          )}

          {activeTab === "skills" && (
            <div className="space-y-2">
              {BUILTIN_SKILLS.map((skill) => (
                <div
                  key={skill.id}
                  className="p-3 rounded-xl border border-zinc-100 hover:border-zinc-200 bg-zinc-50/50 flex items-start justify-between gap-3"
                >
                  <div>
                    <div className="text-xs font-semibold text-zinc-900 flex items-center gap-2">
                      <span>{skill.name}</span>
                      <span className="text-[10px] font-normal px-1.5 py-0.2 rounded bg-purple-50 text-[#5100cd] font-mono">
                        skill://{skill.id}
                      </span>
                    </div>
                    <p className="text-[11px] text-zinc-500 mt-0.5">{skill.desc}</p>
                  </div>
                  <span className="text-[10px] font-medium text-emerald-600 bg-emerald-50 px-2 py-0.5 rounded-full shrink-0">
                    Active
                  </span>
                </div>
              ))}
            </div>
          )}

          {activeTab === "tools" && (
            <div className="space-y-2">
              {BUILTIN_TOOLS.map((tool) => (
                <div
                  key={tool.id}
                  className="p-3 rounded-xl border border-zinc-100 hover:border-zinc-200 bg-zinc-50/50 flex items-start justify-between gap-3"
                >
                  <div>
                    <div className="text-xs font-semibold text-zinc-900 flex items-center gap-2">
                      <span>{tool.name}</span>
                      <span className="text-[10px] font-normal px-1.5 py-0.2 rounded bg-zinc-100 text-zinc-600 font-mono">
                        {tool.id}
                      </span>
                    </div>
                    <p className="text-[11px] text-zinc-500 mt-0.5">{tool.desc}</p>
                  </div>
                  <span className="text-[10px] font-medium text-emerald-600 bg-emerald-50 px-2 py-0.5 rounded-full shrink-0">
                    Enabled
                  </span>
                </div>
              ))}
            </div>
          )}

          {activeTab === "mcp" && (
            <div className="space-y-3 text-center py-6">
              <Server className="w-8 h-8 text-[#5100cd] mx-auto opacity-75" />
              <h3 className="text-xs font-semibold text-zinc-900">Model Context Protocol (MCP) Hub</h3>
              <p className="text-xs text-zinc-500 max-w-sm mx-auto">
                Connect external tool servers, SQLite databases, and browser harnesses via standard MCP endpoints.
              </p>
              <div className="p-3 rounded-xl bg-zinc-50 border border-zinc-200 text-left text-xs font-mono text-zinc-600">
                {"// ~/.fusion/mcp_servers.json\n{\n  \"servers\": {\n    \"memory\": { \"command\": \"npx\", \"args\": [\"-y\", \"@modelcontextprotocol/server-memory\"] }\n  }\n}"}
              </div>
            </div>
          )}
        </div>

        {/* Footer */}
        <div className="px-5 py-3 border-t border-zinc-100 bg-zinc-50/50 flex justify-end">
          <button
            type="button"
            onClick={onClose}
            className="px-3 py-1.5 text-xs font-medium text-zinc-700 hover:text-zinc-900 hover:bg-zinc-200/60 rounded-lg transition-colors cursor-pointer"
          >
            Done
          </button>
        </div>
      </div>
    </div>
  );
}

export default CustomizeModal;
