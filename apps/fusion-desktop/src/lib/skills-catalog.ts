export interface SkillItem {
  id: string;
  name: string;
  trigger: string;
  description: string;
  category: "command" | "skill" | "mode";
  source?: string;
  icon?: string;
}

export const DEFAULT_SKILLS: SkillItem[] = [
  // Core Slash Commands
  {
    id: "cmd-skills",
    name: "skills",
    trigger: "/skills",
    description: "Browse, inspect, and toggle active agent domain skills",
    category: "command",
    icon: "Sparkles",
  },
  {
    id: "cmd-plan",
    name: "plan",
    trigger: "/plan",
    description: "Switch to Architect / Plan mode (designs solution without touching files)",
    category: "mode",
    icon: "Compass",
  },
  {
    id: "cmd-act",
    name: "act",
    trigger: "/act",
    description: "Switch to Code / Act mode (full implementation & tool execution)",
    category: "mode",
    icon: "Zap",
  },
  {
    id: "cmd-clear",
    name: "clear",
    trigger: "/clear",
    description: "Clear current conversation context and start fresh",
    category: "command",
    icon: "Trash2",
  },
  {
    id: "cmd-review",
    name: "review",
    trigger: "/review",
    description: "Review current git working tree changes and uncommitted diffs",
    category: "command",
    icon: "FileCheck",
  },
  {
    id: "cmd-mcp",
    name: "mcp",
    trigger: "/mcp",
    description: "List configured Model Context Protocol servers and active tools",
    category: "command",
    icon: "Cpu",
  },
  {
    id: "cmd-help",
    name: "help",
    trigger: "/help",
    description: "Show interactive command and keyboard shortcuts reference",
    category: "command",
    icon: "HelpCircle",
  },

  // Domain Agent Skills
  {
    id: "skill-systematic-debugging",
    name: "systematic-debugging",
    trigger: "/skill:systematic-debugging",
    description: "Investigate root cause before proposing fixes; reproduce first",
    category: "skill",
    source: "global",
    icon: "Bug",
  },
  {
    id: "skill-brainstorming",
    name: "brainstorming",
    trigger: "/skill:brainstorming",
    description: "Explore intent, requirements, and design specs before implementation",
    category: "skill",
    source: "global",
    icon: "Lightbulb",
  },
  {
    id: "skill-test-driven-development",
    name: "test-driven-development",
    trigger: "/skill:test-driven-development",
    description: "Write failing invariant tests first before implementing code",
    category: "skill",
    source: "global",
    icon: "CheckSquare",
  },
  {
    id: "skill-design-taste-frontend",
    name: "design-taste-frontend",
    trigger: "/skill:design-taste-frontend",
    description: "Create distinctive, polished, production-grade frontend interfaces",
    category: "skill",
    source: "workspace",
    icon: "Palette",
  },
  {
    id: "skill-fixing-gpuix-layout",
    name: "fixing-gpuix-layout",
    trigger: "/skill:fixing-gpuix-layout",
    description: "Fix GPUI/layout rendering, traffic lights clearance & window borders",
    category: "skill",
    source: "workspace",
    icon: "Layout",
  },
  {
    id: "skill-verification-before-completion",
    name: "verification-before-completion",
    trigger: "/skill:verification-before-completion",
    description: "Verify all behaviors with tests and smoke runs before claiming completion",
    category: "skill",
    source: "global",
    icon: "ShieldCheck",
  },
  {
    id: "skill-strace",
    name: "strace",
    trigger: "/skill:strace",
    description: "Analyze multi-agent trajectories and optimize agent prompt routing",
    category: "skill",
    source: "global",
    icon: "Activity",
  },
  {
    id: "skill-cloudflare",
    name: "cloudflare",
    trigger: "/skill:cloudflare",
    description: "Cloudflare Workers, Durable Objects, KV, D1, R2 & Agents SDK",
    category: "skill",
    source: "global",
    icon: "Cloud",
  },
  {
    id: "skill-web-perf",
    name: "web-perf",
    trigger: "/skill:web-perf",
    description: "Audit and optimize Core Web Vitals, LCP, INP, and bundle size",
    category: "skill",
    source: "global",
    icon: "Gauge",
  },
  {
    id: "skill-check-work",
    name: "check-work",
    trigger: "/skill:check-work",
    description: "Comprehensive sanity check against acceptance criteria",
    category: "skill",
    source: "user",
    icon: "CheckCircle",
  },
  {
    id: "skill-code-review",
    name: "code-review",
    trigger: "/skill:code-review",
    description: "Senior engineer code quality and security vulnerability audit",
    category: "skill",
    source: "user",
    icon: "Code2",
  },
  {
    id: "skill-imagine",
    name: "imagine",
    trigger: "/skill:imagine",
    description: "Creative ideation, architecture brainstorming, and novel concept exploration",
    category: "skill",
    source: "user",
    icon: "Eye",
  },
  {
    id: "skill-create-skill",
    name: "create-skill",
    trigger: "/skill:create-skill",
    description: "Guide step-by-step authoring of a new domain SKILL.md",
    category: "skill",
    source: "user",
    icon: "PlusCircle",
  },
];

export function filterSkills(query: string, skills: SkillItem[] = DEFAULT_SKILLS): SkillItem[] {
  const clean = query.trim().toLowerCase();
  if (!clean || clean === "/") {
    return skills;
  }
  const term = clean.startsWith("/") ? clean.slice(1) : clean;

  return skills.filter((s) => {
    return (
      s.name.toLowerCase().includes(term) ||
      s.trigger.toLowerCase().includes(term) ||
      s.description.toLowerCase().includes(term) ||
      s.category.toLowerCase().includes(term)
    );
  });
}
