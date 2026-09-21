import React, { useState, useMemo } from "react";
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
  RotateCcw,
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

// 10 Tools matching Cline Image #1
const INITIAL_TOOLS: CustomItem[] = [
  {
    id: "ask_question",
    name: "ask_question",
    description: "Ask the user a single clarifying question with 2-5 selectable options.",
    scope: "Built-in",
    enabled: true,
  },
  {
    id: "editor",
    name: "editor",
    description: "Make controlled filesystem edits on text files with create, replace, and insert operations.",
    scope: "Built-in",
    enabled: true,
  },
  {
    id: "fetch_web_content",
    name: "fetch_web_content",
    description: "Fetch URL content and analyze it with a prompt describing what to extract.",
    scope: "Built-in",
    enabled: true,
  },
  {
    id: "read_files",
    name: "read_files",
    description: "Read the content of text or image files at the provided absolute paths, or return only an inclusive one-based line range when start_line/end_line are provided. Long files are windowed; page with start_line/end_line.",
    scope: "Built-in",
    enabled: true,
  },
  {
    id: "run_commands",
    name: "run_commands",
    description: "Run shell commands from the root of the workspace for listing files, checking git status, builds, tests, and similar tasks.",
    scope: "Built-in",
    enabled: true,
  },
  {
    id: "search_codebase",
    name: "search_codebase",
    description: "Perform regex pattern searches across the codebase for code patterns, definitions, imports, and other text matches.",
    scope: "Built-in",
    enabled: true,
  },
  {
    id: "read",
    name: "read",
    description: "Read files, directories, SQLite tables, images, and documents.",
    scope: "Built-in",
    enabled: true,
  },
  {
    id: "edit",
    name: "edit",
    description: "Line-anchored surgical patch language for existing codebases.",
    scope: "Built-in",
    enabled: true,
  },
  {
    id: "write",
    name: "write",
    description: "Create or overwrite files, configurations, and archives.",
    scope: "Built-in",
    enabled: true,
  },
  {
    id: "bash",
    name: "bash",
    description: "Run commands, test suites, and shell pipelines in a persistent process.",
    scope: "Built-in",
    enabled: true,
  },
];

const INITIAL_PLUGINS: CustomItem[] = [];

// 17 Skills matching Cline Image #1
const INITIAL_SKILLS: CustomItem[] = [
  { id: "brainstorming", name: "brainstorming", description: "Explores user intent, requirements, and design specs before implementation.", scope: "Global", enabled: true },
  { id: "systematic-debugging", name: "systematic-debugging", description: "Root-cause investigation before proposing or applying code fixes.", scope: "Global", enabled: true },
  { id: "test-driven-development", name: "test-driven-development", description: "Red-Green-Refactor test suites defending invariants.", scope: "Global", enabled: true },
  { id: "frontend-design", name: "frontend-design", description: "Tailwind CSS and React UI component refinement with high aesthetic taste.", scope: "Global", enabled: true },
  { id: "verification-before-completion", name: "verification-before-completion", description: "Runs test commands and collects concrete output before claiming completion.", scope: "Global", enabled: true },
  { id: "executing-plans", name: "executing-plans", description: "Executes implementation plans in separate sessions with review checkpoints.", scope: "Global", enabled: true },
  { id: "writing-plans", name: "writing-plans", description: "Authors multi-step implementation plans before touching code.", scope: "Global", enabled: true },
  { id: "requesting-code-review", name: "requesting-code-review", description: "Verifies work meets requirements before merging or completing tasks.", scope: "Global", enabled: true },
  { id: "receiving-code-review", name: "receiving-code-review", description: "Rigorous technical verification of feedback before implementing suggestions.", scope: "Global", enabled: true },
  { id: "subagent-driven-development", name: "subagent-driven-development", description: "Coordinates independent tasks with subagents in current session.", scope: "Global", enabled: true },
  { id: "optical-refinement", name: "optical-refinement", description: "Reviews designs for optical balance, typography kerning, and spacing rhythm.", scope: "Global", enabled: true },
  { id: "plain-english", name: "plain-english", description: "Tightens prose by stripping AI tics and applying plain English rules.", scope: "Global", enabled: true },
  { id: "web-perf", name: "web-perf", description: "Analyzes Core Web Vitals, layout shifts, and render-blocking resources.", scope: "Global", enabled: true },
  { id: "qa", name: "qa", description: "QA tests websites and web apps with quality scores and evidence.", scope: "Global", enabled: true },
  { id: "strace", name: "strace", description: "Optimizes multi-agent trajectories and agent performance traces.", scope: "Global", enabled: true },
  { id: "durable-objects", name: "durable-objects", description: "Creates stateful coordination and durable SQLite storage systems.", scope: "Global", enabled: true },
  { id: "workers-best-practices", name: "workers-best-practices", description: "Reviews and authors production edge worker architectures.", scope: "Global", enabled: true },
];

const INITIAL_RULES: CustomItem[] = [];
const INITIAL_MCP: CustomItem[] = [];
const INITIAL_HOOKS: CustomItem[] = [];

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

  const allDisabled = useMemo(() => {
    return currentItems.length > 0 && currentItems.every((item) => !item.enabled);
  }, [currentItems]);

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

  const toggleDisableAll = () => {
    const nextState = allDisabled;
    const updater = (list: CustomItem[]) =>
      list.map((it) => ({ ...it, enabled: nextState }));

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
      case "tools": return <Wrench className="size-3.5 text-zinc-500" />;
      case "plugins": return <Puzzle className="size-3.5 text-zinc-500" />;
      case "skills": return <Sparkles className="size-3.5 text-zinc-500" />;
      case "rules": return <BookOpen className="size-3.5 text-zinc-500" />;
      case "mcp": return <Server className="size-3.5 text-zinc-500" />;
      case "hooks": return <Zap className="size-3.5 text-zinc-500" />;
    }
  };

  return (
    <div
      data-testid="customize-view"
      className={`h-full w-full overflow-y-auto bg-white text-zinc-900 ${className}`}
    >
      {/* PageFrame matching Cline px-18 py-10 */}
      <div className="max-w-5xl mx-auto px-8 md:px-12 py-8">
        {/* PageHeader matching Cline Image #1: clean heading, NO inline back button */}
        <section className="mb-6 flex items-start justify-between gap-6 max-[860px]:flex-col">
          <div className="min-w-0">
            <h1 className="truncate text-2xl md:text-3xl font-semibold tracking-tight text-zinc-900">
              Customize
            </h1>
            <p className="mt-2 text-sm text-zinc-500 max-w-2xl leading-relaxed">
              Extend what Cline can do and how it works. Explore the marketplace for more options.
            </p>
          </div>

          <div className="flex shrink-0 items-center gap-2">
            <button
              type="button"
              data-testid="customize-marketplace-btn"
              onClick={onOpenMarketplace}
              className="inline-flex items-center gap-2 px-3 py-1.5 rounded-lg border border-zinc-200 bg-white hover:bg-zinc-50 text-xs font-medium text-zinc-800 shadow-2xs transition-colors cursor-pointer"
            >
              <Store className="size-4 text-zinc-700" />
              <span>Marketplace</span>
            </button>
          </div>
        </section>

        {/* Sub-tabs with live counts matching Cline Image #1 */}
        <div className="mb-6 flex items-center gap-0 border-b border-zinc-200">
          {(["tools", "plugins", "skills", "rules", "mcp", "hooks"] as const).map((tabId) => {
            const active = tab === tabId;
            const count = counts[tabId];
            const label = tabId === "mcp" ? "MCP" : tabId.charAt(0).toUpperCase() + tabId.slice(1);

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
                <span className="text-xs text-zinc-400 tabular-nums">
                  {count}
                </span>
                {active && (
                  <span className="absolute inset-x-0 -bottom-px h-0.5 bg-zinc-900" />
                )}
              </button>
            );
          })}
        </div>

        {/* Subheader: description on left, refresh on right */}
        <div className="flex items-center justify-between text-xs text-zinc-500 mb-2">
          <span>{SECTION_DESCRIPTIONS[tab]}</span>
          <button
            type="button"
            className="p-1 rounded hover:bg-zinc-100 text-zinc-400 hover:text-zinc-700 transition-colors cursor-pointer"
            title="Refresh"
          >
            <RotateCcw className="size-4" />
          </button>
        </div>

        {/* Full-width Search Bar matching Cline Image #1 */}
        <div className="relative w-full mb-6">
          <Search className="absolute left-3.5 top-2.5 size-4 text-zinc-400" />
          <input
            type="text"
            value={searchQuery}
            onChange={(e) => setSearchQuery(e.target.value)}
            placeholder={`Search ${tab}`}
            className="w-full pl-10 pr-3 py-2 text-xs rounded-lg border border-zinc-200 bg-white outline-none focus:border-zinc-400 placeholder-zinc-400"
          />
        </div>

        {/* Section Header Row: Title on left, Disable all on right */}
        <div className="flex items-center justify-between mb-3 select-none">
          <span className="text-xs font-semibold text-zinc-900">
            {tab === "tools" ? "BuiltIn Tools" : tab.charAt(0).toUpperCase() + tab.slice(1)} {currentItems.length}
          </span>
          <label className="flex items-center gap-1.5 text-xs text-zinc-600 cursor-pointer">
            <input
              type="checkbox"
              checked={allDisabled}
              onChange={toggleDisableAll}
              className="w-3.5 h-3.5 rounded border-zinc-300 text-[#5100cd] focus:ring-[#5100cd] cursor-pointer"
            />
            <span>Disable all</span>
          </label>
        </div>

        {/* Card Rows matching Cline Image #1 */}
        {tab === "rules" ? (
          <div className="space-y-6">
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
                No items match your search.
              </div>
            ) : (
              currentItems.map((item) => (
                <div
                  key={item.id}
                  className="p-3.5 rounded-xl border border-zinc-200/80 bg-white hover:border-zinc-300 transition-all flex items-start justify-between gap-4"
                >
                  <div className="min-w-0 pr-4">
                    <div className="flex items-center gap-2">
                      {getTabIcon(tab)}
                      <span className="text-xs font-semibold text-zinc-900">{item.name}</span>
                    </div>
                    <p className="text-[11px] text-zinc-500 mt-1 leading-relaxed">{item.description}</p>
                  </div>

                  {/* Toggle Switch matching Image #1 */}
                  <button
                    type="button"
                    onClick={() => toggleItem(item.id)}
                    className={`w-8 h-4.5 rounded-full transition-colors relative cursor-pointer shrink-0 mt-0.5 ${
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
