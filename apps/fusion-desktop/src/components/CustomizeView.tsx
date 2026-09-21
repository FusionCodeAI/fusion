import React, { useState, useMemo, useEffect } from "react";
import {
  Wrench,
  Puzzle,
  Sparkles,
  BookOpen,
  Server,
  Zap,
  Store,
  Search,
  Check,
  X,
  Copy,
  Plus,
  ArrowLeft,
  ExternalLink,
} from "lucide-react";

export type CustomizeTab =
  | "tools"
  | "plugins"
  | "skills"
  | "rules"
  | "mcp"
  | "hooks";

export interface CustomizeViewProps {
  onClose?: () => void;
  onOpenMarketplace?: () => void;
  className?: string;
}

interface CustomItem {
  id: string;
  name: string;
  description: string;
  scope: "Built-in" | "Project" | "Global";
  enabled: boolean;
  command?: string;
  path?: string;
}

const SECTION_DESCRIPTIONS: Record<CustomizeTab, string> = {
  tools: "Inspect built-in tools and tools contributed by plugins.",
  plugins: "Manage installed plugins and add new plugins from the marketplace.",
  skills: "Manage installed skills and add new skills from the marketplace.",
  rules: "Review project and global rule files that shape agent behavior.",
  mcp: "Manage installed MCP servers and add new servers from the marketplace.",
  hooks: "Inspect hook configuration and recent execution status.",
};

const SECTION_COMMANDS: Record<CustomizeTab, string> = {
  tools: "fusion config tools",
  plugins: "fusion plugin",
  skills: "fusion skill",
  rules: "fusion config rules",
  mcp: "fusion mcp install",
  hooks: "fusion config hooks",
};

const INITIAL_TOOLS: CustomItem[] = [
  { id: "read", name: "read", description: "Read files, directories, SQLite tables, images, and documents.", scope: "Built-in", enabled: true, command: "fusion tool read" },
  { id: "edit", name: "edit", description: "Line-anchored surgical patch language for existing codebases.", scope: "Built-in", enabled: true, command: "fusion tool edit" },
  { id: "write", name: "write", description: "Create or overwrite files, configurations, and archives.", scope: "Built-in", enabled: true, command: "fusion tool write" },
  { id: "bash", name: "bash", description: "Run commands, test suites, and shell pipelines in a persistent process.", scope: "Built-in", enabled: true, command: "fusion tool bash" },
  { id: "grep", name: "grep", description: "Fast Rust PCRE2-compatible regex search across workspace files.", scope: "Built-in", enabled: true, command: "fusion tool grep" },
  { id: "glob", name: "glob", description: "Fast pattern matching for files and directories.", scope: "Built-in", enabled: true, command: "fusion tool glob" },
  { id: "web_search", name: "web_search", description: "Fetch live web documentation and references.", scope: "Built-in", enabled: true, command: "fusion tool web_search" },
  { id: "eval", name: "eval", description: "Run incremental JavaScript (Bun) or Python in persistent runtime kernels.", scope: "Built-in", enabled: true, command: "fusion tool eval" },
];

const INITIAL_PLUGINS: CustomItem[] = [
  { id: "fusion-git", name: "Git Control Plane", description: "High-level VCS branch management, commit generator, and merge conflict resolution.", scope: "Built-in", enabled: true },
  { id: "fusion-ast", name: "AST Engine", description: "Tree-sitter structural syntax parsing and symbol definition lookups.", scope: "Built-in", enabled: true },
  { id: "fusion-linter", name: "Diagnostic Linter", description: "Automated compiler diagnostic extraction and auto-fix loop.", scope: "Project", enabled: true },
];

const INITIAL_SKILLS: CustomItem[] = [
  { id: "brainstorming", name: "Brainstorming", description: "Explores user intent, requirements, and design specs before implementation.", scope: "Global", enabled: true, path: "skill://brainstorming" },
  { id: "systematic-debugging", name: "Systematic Debugging", description: "Root-cause investigation before proposing or applying code fixes.", scope: "Global", enabled: true, path: "skill://systematic-debugging" },
  { id: "test-driven-development", name: "TDD Loop", description: "Red-Green-Refactor test suites defending invariants.", scope: "Global", enabled: true, path: "skill://test-driven-development" },
  { id: "frontend-design", name: "Frontend Design", description: "Tailwind CSS and React UI component refinement with high aesthetic taste.", scope: "Global", enabled: true, path: "skill://frontend-design" },
  { id: "verification-before-completion", name: "Verification", description: "Runs test commands and collects concrete output before claiming completion.", scope: "Global", enabled: true, path: "skill://verification-before-completion" },
];

const INITIAL_RULES: CustomItem[] = [
  { id: "fusionrules", name: ".fusionrules", description: "Project-level instructions loaded into every turn prompt.", scope: "Project", enabled: true, path: "./.fusionrules" },
  { id: "agents-md", name: "AGENTS.md", description: "Repository developer guidelines and team conventions.", scope: "Project", enabled: true, path: "./AGENTS.md" },
  { id: "global-rules", name: "Global System Instructions", description: "User preferences applied across all projects and sessions.", scope: "Global", enabled: true, path: "~/.fusion/rules.md" },
];

const INITIAL_MCP: CustomItem[] = [
  { id: "memory", name: "mcp-server-memory", description: "Persistent knowledge graph memory across agent runs.", scope: "Global", enabled: true, command: "npx -y @modelcontextprotocol/server-memory" },
  { id: "sqlite", name: "mcp-server-sqlite", description: "Inspect and query local SQLite database tables.", scope: "Project", enabled: true, command: "npx -y @modelcontextprotocol/server-sqlite" },
];

const INITIAL_HOOKS: CustomItem[] = [
  { id: "post-task", name: "postTask", description: "Executes lint and test verification after completing an agent task.", scope: "Project", enabled: true, path: ".fusion/hooks/post-task.sh" },
  { id: "pre-tool", name: "preToolCall", description: "Validates sensitive terminal commands before execution.", scope: "Global", enabled: true, path: "~/.fusion/hooks/pre-tool.sh" },
];

const DEFAULT_RULE_TEXT = `# Project Rules & System Instructions
- Always prioritize correctness, elegance, and maintainability.
- Write clean, self-documenting code with TypeScript strict types.
- Follow existing repository conventions and test with bun test.
`;

export function CustomizeView({
  onClose,
  onOpenMarketplace,
  className = "",
}: CustomizeViewProps) {
  const [tab, setTab] = useState<CustomizeTab>("tools");
  const [searchQuery, setSearchQuery] = useState("");

  const [tools, setTools] = useState<CustomItem[]>(INITIAL_TOOLS);
  const [plugins, setPlugins] = useState<CustomItem[]>(INITIAL_PLUGINS);
  const [skills, setSkills] = useState<CustomItem[]>(INITIAL_SKILLS);
  const [rules, setRules] = useState<CustomItem[]>(INITIAL_RULES);
  const [mcp, setMcp] = useState<CustomItem[]>(INITIAL_MCP);
  const [hooks, setHooks] = useState<CustomItem[]>(INITIAL_HOOKS);

  const [ruleEditorText, setRuleEditorText] = useState<string>(() => {
    if (typeof window !== "undefined" && window.localStorage) {
      return window.localStorage.getItem("fusion_desktop_custom_rules") || DEFAULT_RULE_TEXT;
    }
    return DEFAULT_RULE_TEXT;
  });
  const [isSavedRule, setIsSavedRule] = useState(false);

  const counts: Record<CustomizeTab, number> = useMemo(() => ({
    tools: tools.length,
    plugins: plugins.length,
    skills: skills.length,
    rules: rules.length,
    mcp: mcp.length,
    hooks: hooks.length,
  }), [tools, plugins, skills, rules, mcp, hooks]);

  const currentItems = useMemo(() => {
    let list: CustomItem[] = [];
    if (tab === "tools") list = tools;
    else if (tab === "plugins") list = plugins;
    else if (tab === "skills") list = skills;
    else if (tab === "rules") list = rules;
    else if (tab === "mcp") list = mcp;
    else if (tab === "hooks") list = hooks;

    if (!searchQuery.trim()) return list;
    const q = searchQuery.toLowerCase().trim();
    return list.filter(
      (item) =>
        item.name.toLowerCase().includes(q) ||
        item.description.toLowerCase().includes(q) ||
        item.id.toLowerCase().includes(q)
    );
  }, [tab, tools, plugins, skills, rules, mcp, hooks, searchQuery]);

  const toggleItem = (id: string) => {
    const updater = (list: CustomItem[]) =>
      list.map((it) => (it.id === id ? { ...it, enabled: !it.enabled } : it));

    if (tab === "tools") setTools(updater);
    else if (tab === "plugins") setPlugins(updater);
    else if (tab === "skills") setSkills(updater);
    else if (tab === "rules") setRules(updater);
    else if (tab === "mcp") setMcp(updater);
    else if (tab === "hooks") setHooks(updater);
  };

  const handleSaveRuleText = () => {
    if (typeof window !== "undefined" && window.localStorage) {
      window.localStorage.setItem("fusion_desktop_custom_rules", ruleEditorText);
    }
    setIsSavedRule(true);
    setTimeout(() => setIsSavedRule(false), 2000);
  };

  const getTabIcon = (t: CustomizeTab) => {
    switch (t) {
      case "tools": return <Wrench className="size-4 text-[#5100cd]" />;
      case "plugins": return <Puzzle className="size-4 text-[#5100cd]" />;
      case "skills": return <Sparkles className="size-4 text-[#5100cd]" />;
      case "rules": return <BookOpen className="size-4 text-[#5100cd]" />;
      case "mcp": return <Server className="size-4 text-[#5100cd]" />;
      case "hooks": return <Zap className="size-4 text-[#5100cd]" />;
    }
  };

  return (
    <div
      data-testid="customize-view"
      className={`h-full w-full overflow-y-auto bg-white text-zinc-900 ${className}`}
    >
      {/* PageFrame matching Cline px-18 py-10 */}
      <div className="max-w-5xl mx-auto px-8 md:px-12 py-8">
        {/* PageHeader matching Cline */}
        <section className="mb-6 flex items-start justify-between gap-6 max-[860px]:flex-col">
          <div className="min-w-0">
            <div className="flex items-center gap-3">
              {onClose && (
                <button
                  type="button"
                  data-testid="customize-back-btn"
                  onClick={onClose}
                  className="p-1.5 rounded-lg border border-zinc-200 hover:bg-zinc-100 text-zinc-600 transition-colors cursor-pointer mr-1"
                  title="Back to conversation"
                >
                  <ArrowLeft className="size-4" />
                </button>
              )}
              <h1 className="truncate text-2xl md:text-3xl font-semibold tracking-tight text-zinc-900">
                Customize
              </h1>
            </div>
            <p className="mt-2 text-sm text-zinc-500 max-w-2xl leading-relaxed">
              Extend what Cline can do and how it works. Explore the marketplace for more options.
            </p>
          </div>

          <div className="flex shrink-0 items-center gap-2">
            {onOpenMarketplace && (
              <button
                type="button"
                onClick={onOpenMarketplace}
                className="inline-flex items-center gap-2 px-3.5 py-1.5 rounded-lg border border-zinc-200/80 bg-white hover:bg-zinc-50 text-xs font-medium text-zinc-700 shadow-2xs transition-colors cursor-pointer"
              >
                <Store className="size-3.5" />
                <span>Marketplace</span>
              </button>
            )}
            {onClose && (
              <button
                type="button"
                data-testid="customize-close-top-btn"
                onClick={onClose}
                className="p-1.5 rounded-lg text-zinc-400 hover:text-zinc-700 hover:bg-zinc-100 transition-colors cursor-pointer"
                title="Close"
              >
                <X className="size-4" />
              </button>
            )}
          </div>
        </section>

        {/* Sub-tabs with live counts matching Cline CUSTOMIZE_TABS */}
        <div className="mb-6 flex items-center gap-0 border-b border-zinc-200">
          {(["tools", "plugins", "skills", "rules", "mcp", "hooks"] as const).map((tabId) => {
            const active = tab === tabId;
            const count = counts[tabId];
            const label = tabId.charAt(0).toUpperCase() + tabId.slice(1);

            return (
              <button
                key={tabId}
                type="button"
                data-testid={`customize-tab-${tabId}`}
                aria-current={active ? "page" : undefined}
                onClick={() => setTab(tabId)}
                className={`relative px-4 py-2.5 text-xs md:text-sm font-medium transition-colors cursor-pointer flex items-center gap-1.5 ${
                  active
                    ? "text-zinc-900 font-semibold"
                    : "text-zinc-500 hover:text-zinc-900"
                }`}
              >
                <span>{label}</span>
                <span
                  className={`text-xs tabular-nums px-1.5 py-0.2 rounded-full ${
                    active ? "bg-purple-100 text-[#5100cd]" : "bg-zinc-100 text-zinc-500"
                  }`}
                >
                  {count}
                </span>
                {active && (
                  <span className="absolute inset-x-0 -bottom-px h-0.5 bg-[#5100cd]" />
                )}
              </button>
            );
          })}
        </div>

        {/* Section Subheader with CommandBadge & Search */}
        <div className="mb-6 flex flex-col md:flex-row md:items-center justify-between gap-4">
          <div>
            <p className="text-xs text-zinc-600">
              {SECTION_DESCRIPTIONS[tab]}
            </p>
            <div className="mt-1 flex items-center gap-2">
              <span className="rounded-md border border-zinc-200 bg-zinc-50 px-2 py-0.5 font-mono text-[11px] text-zinc-600">
                {SECTION_COMMANDS[tab]}
              </span>
            </div>
          </div>

          <div className="relative w-full md:w-64">
            <Search className="absolute left-2.5 top-2.5 size-3.5 text-zinc-400" />
            <input
              type="text"
              value={searchQuery}
              onChange={(e) => setSearchQuery(e.target.value)}
              placeholder={`Filter ${tab}...`}
              className="w-full pl-8 pr-3 py-1.5 text-xs rounded-lg border border-zinc-200 bg-zinc-50/50 outline-none focus:bg-white focus:border-[#5100cd] transition-colors"
            />
          </div>
        </div>

        {/* Section Content */}
        {tab === "rules" ? (
          <div className="space-y-6">
            {/* Rule items */}
            <div className="space-y-2">
              {currentItems.map((item) => (
                <div
                  key={item.id}
                  className="p-3.5 rounded-xl border border-zinc-200/80 bg-white hover:border-zinc-300 transition-all flex items-center justify-between gap-4"
                >
                  <div className="flex items-center gap-3 min-w-0">
                    <div className="w-8 h-8 rounded-lg bg-purple-50 text-[#5100cd] flex items-center justify-center shrink-0">
                      <BookOpen className="size-4" />
                    </div>
                    <div className="min-w-0">
                      <div className="flex items-center gap-2">
                        <span className="text-xs font-semibold text-zinc-900 truncate">{item.name}</span>
                        <span className="text-[10px] font-medium px-1.5 py-0.2 rounded bg-zinc-100 text-zinc-600">
                          {item.scope}
                        </span>
                        {item.path && (
                          <span className="text-[11px] font-mono text-zinc-400 truncate">
                            {item.path}
                          </span>
                        )}
                      </div>
                      <p className="text-[11px] text-zinc-500 mt-0.5 truncate">{item.description}</p>
                    </div>
                  </div>

                  {/* Toggle Switch */}
                  <button
                    type="button"
                    onClick={() => toggleItem(item.id)}
                    className={`w-8 h-4.5 rounded-full transition-colors relative cursor-pointer shrink-0 ${
                      item.enabled ? "bg-[#5100cd]" : "bg-zinc-300"
                    }`}
                  >
                    <span
                      className={`absolute top-0.5 left-0.5 w-3.5 h-3.5 rounded-full bg-white transition-transform ${
                        item.enabled ? "translate-x-3.5" : "translate-x-0"
                      }`}
                    />
                  </button>
                </div>
              ))}
            </div>

            {/* Custom Rules Live Editor */}
            <div className="p-4 rounded-xl border border-zinc-200 bg-zinc-50/50 space-y-3">
              <div className="flex items-center justify-between">
                <div>
                  <h3 className="text-xs font-semibold text-zinc-900">Custom Rules & Instructions Editor</h3>
                  <p className="text-[11px] text-zinc-500 mt-0.5">
                    Changes here take effect immediately on all subsequent agent turns.
                  </p>
                </div>
                <button
                  type="button"
                  onClick={handleSaveRuleText}
                  className="inline-flex items-center gap-1.5 px-3 py-1.5 text-xs font-medium bg-[#5100cd] hover:bg-[#4300a8] text-white rounded-lg transition-colors cursor-pointer shadow-2xs"
                >
                  {isSavedRule ? (
                    <>
                      <Check className="size-3.5 text-emerald-300" />
                      <span>Saved!</span>
                    </>
                  ) : (
                    <span>Save Rules</span>
                  )}
                </button>
              </div>

              <textarea
                rows={8}
                value={ruleEditorText}
                onChange={(e) => setRuleEditorText(e.target.value)}
                className="w-full p-3 font-mono text-xs text-zinc-900 bg-white border border-zinc-200 rounded-lg outline-none focus:border-[#5100cd] leading-relaxed resize-y"
              />
            </div>
          </div>
        ) : (
          <div className="space-y-2">
            {currentItems.length === 0 ? (
              <div className="p-8 text-center border border-dashed border-zinc-200 rounded-xl text-xs text-zinc-500">
                No items match your filter query.
              </div>
            ) : (
              currentItems.map((item) => (
                <div
                  key={item.id}
                  className="p-3.5 rounded-xl border border-zinc-200/80 bg-white hover:border-zinc-300 transition-all flex items-center justify-between gap-4"
                >
                  <div className="flex items-center gap-3 min-w-0">
                    <div className="w-8 h-8 rounded-lg bg-zinc-100 text-zinc-600 flex items-center justify-center shrink-0">
                      {getTabIcon(tab)}
                    </div>
                    <div className="min-w-0">
                      <div className="flex items-center gap-2">
                        <span className="text-xs font-semibold text-zinc-900 truncate">{item.name}</span>
                        <span className="text-[10px] font-medium px-1.5 py-0.2 rounded bg-zinc-100 text-zinc-600">
                          {item.scope}
                        </span>
                        {item.command && (
                          <span className="text-[10px] font-mono text-zinc-400 bg-zinc-50 px-1.5 py-0.5 rounded border border-zinc-200 truncate">
                            {item.command}
                          </span>
                        )}
                        {item.path && (
                          <span className="text-[10px] font-mono text-purple-600 bg-purple-50 px-1.5 py-0.5 rounded truncate">
                            {item.path}
                          </span>
                        )}
                      </div>
                      <p className="text-[11px] text-zinc-500 mt-0.5 truncate">{item.description}</p>
                    </div>
                  </div>

                  {/* Toggle Switch */}
                  <button
                    type="button"
                    onClick={() => toggleItem(item.id)}
                    className={`w-8 h-4.5 rounded-full transition-colors relative cursor-pointer shrink-0 ${
                      item.enabled ? "bg-[#5100cd]" : "bg-zinc-300"
                    }`}
                  >
                    <span
                      className={`absolute top-0.5 left-0.5 w-3.5 h-3.5 rounded-full bg-white transition-transform ${
                        item.enabled ? "translate-x-3.5" : "translate-x-0"
                      }`}
                    />
                  </button>
                </div>
              ))
            )}
          </div>
        )}
      </div>
    </div>
  );
}

export default CustomizeView;
