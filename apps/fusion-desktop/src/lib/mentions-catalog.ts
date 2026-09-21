export interface MentionItem {
  id: string;
  label: string;
  value: string;
  description: string;
  category: "context" | "file" | "git" | "diagnostics";
  icon?: string;
}

export const DEFAULT_CONTEXT_MENTIONS: MentionItem[] = [
  {
    id: "mention-file",
    label: "@file",
    value: "@file:",
    description: "Reference a workspace file to provide full file context",
    category: "context",
    icon: "FileCode",
  },
  {
    id: "mention-folder",
    label: "@folder",
    value: "@folder:",
    description: "Reference an entire folder directory tree",
    category: "context",
    icon: "Folder",
  },
  {
    id: "mention-git-diff",
    label: "@git-diff",
    value: "@git-diff",
    description: "Include uncommitted git diffs and modified files",
    category: "git",
    icon: "GitBranch",
  },
  {
    id: "mention-git-commits",
    label: "@commits",
    value: "@commits",
    description: "Include recent git commits log and authors",
    category: "git",
    icon: "GitCommit",
  },
  {
    id: "mention-problems",
    label: "@problems",
    value: "@problems",
    description: "Include workspace compiler errors, warnings, and diagnostics",
    category: "diagnostics",
    icon: "AlertCircle",
  },
  {
    id: "mention-terminal",
    label: "@terminal",
    value: "@terminal",
    description: "Include the latest terminal command output",
    category: "context",
    icon: "Terminal",
  },
  {
    id: "mention-url",
    label: "@url",
    value: "@url:",
    description: "Fetch and read remote web documentation or URL contents",
    category: "context",
    icon: "Globe",
  },
];

export const COMMON_WORKSPACE_FILES = [
  "Cargo.toml",
  "package.json",
  "README.md",
  "tsconfig.json",
  "src/main.rs",
  "src/App.tsx",
  "apps/fusion-desktop/src/App.tsx",
  "apps/fusion-desktop/src/components/Composer.tsx",
  "apps/fusion-desktop/src/components/Sidebar.tsx",
  "apps/fusion-desktop/src/components/ClineHeroView.tsx",
  "apps/fusion-desktop/src-tauri/src/main.rs",
  "crates/fusion-shell/src/lib.rs",
  "crates/fusion-builtins/src/lib.rs",
  "crates/fusion-diff/src/lib.rs",
  "crates/fusion-vcs/src/lib.rs",
  "crates/fusion-ast/src/lib.rs",
];

export function filterMentions(
  query: string,
  customFiles: string[] = []
): MentionItem[] {
  const clean = query.trim().toLowerCase();
  const term = clean.startsWith("@") ? clean.slice(1) : clean;

  const fileItems: MentionItem[] = (customFiles.length > 0 ? customFiles : COMMON_WORKSPACE_FILES).map((path) => {
    const filename = path.split("/").pop() || path;
    return {
      id: `file-${path}`,
      label: `@${filename}`,
      value: `@${path}`,
      description: path,
      category: "file" as const,
      icon: "FileText",
    };
  });

  const allItems = [...DEFAULT_CONTEXT_MENTIONS, ...fileItems];

  if (!term) {
    return allItems;
  }

  return allItems.filter((item) => {
    return (
      item.label.toLowerCase().includes(term) ||
      item.value.toLowerCase().includes(term) ||
      item.description.toLowerCase().includes(term) ||
      item.category.toLowerCase().includes(term)
    );
  });
}
