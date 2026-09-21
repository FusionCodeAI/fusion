export interface MentionItem {
  id: string;
  label: string;
  value: string;
  description: string;
  category: "context" | "file" | "folder" | "git" | "diagnostics";
  icon?: string;
}

export interface WorkspaceEntryLike {
  path: string;
  name?: string;
  is_dir?: boolean;
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

export const COMMON_WORKSPACE_FILES: WorkspaceEntryLike[] = [
  { path: "Cargo.toml", name: "Cargo.toml", is_dir: false },
  { path: "package.json", name: "package.json", is_dir: false },
  { path: "README.md", name: "README.md", is_dir: false },
  { path: "tsconfig.json", name: "tsconfig.json", is_dir: false },
  { path: "src", name: "src", is_dir: true },
  { path: "src/main.rs", name: "main.rs", is_dir: false },
  { path: "src/App.tsx", name: "App.tsx", is_dir: false },
  { path: "apps/fusion-desktop/src/App.tsx", name: "App.tsx", is_dir: false },
  { path: "apps/fusion-desktop/src/components/Composer.tsx", name: "Composer.tsx", is_dir: false },
  { path: "apps/fusion-desktop/src/components/Sidebar.tsx", name: "Sidebar.tsx", is_dir: false },
  { path: "apps/fusion-desktop/src/components/ClineHeroView.tsx", name: "ClineHeroView.tsx", is_dir: false },
  { path: "apps/fusion-desktop/src-tauri/src/main.rs", name: "main.rs", is_dir: false },
  { path: "crates", name: "crates", is_dir: true },
  { path: "crates/fusion-shell/src/lib.rs", name: "lib.rs", is_dir: false },
  { path: "crates/fusion-builtins/src/lib.rs", name: "lib.rs", is_dir: false },
  { path: "crates/fusion-diff/src/lib.rs", name: "lib.rs", is_dir: false },
  { path: "crates/fusion-vcs/src/lib.rs", name: "lib.rs", is_dir: false },
  { path: "crates/fusion-ast/src/lib.rs", name: "lib.rs", is_dir: false },
];

export function filterMentions(
  query: string,
  entries: Array<WorkspaceEntryLike | string> = []
): MentionItem[] {
  const clean = query.trim().toLowerCase();
  let term = clean.startsWith("@") ? clean.slice(1) : clean;

  const isFilterOnlyFiles = term.startsWith("file:") || term === "file";
  const isFilterOnlyFolders = term.startsWith("folder:") || term === "folder";

  if (isFilterOnlyFiles) {
    term = term.replace(/^file:?/, "").trim();
  } else if (isFilterOnlyFolders) {
    term = term.replace(/^folder:?/, "").trim();
  }

  const rawEntries = entries.length > 0 ? entries : COMMON_WORKSPACE_FILES;

  const entryItems: MentionItem[] = rawEntries.map((item) => {
    if (typeof item === "string") {
      const filename = item.split("/").pop() || item;
      const isDir = item.endsWith("/") || !item.includes(".");
      return {
        id: `entry-${item}`,
        label: `@${filename}`,
        value: `@${item}`,
        description: item,
        category: isDir ? "folder" : "file",
        icon: isDir ? "Folder" : "FileText",
      };
    }

    const isDir = Boolean(item.is_dir);
    const filename = item.name || item.path.split("/").pop() || item.path;
    return {
      id: `entry-${item.path}`,
      label: `@${filename}`,
      value: `@${item.path}`,
      description: item.path,
      category: isDir ? "folder" : "file",
      icon: isDir ? "Folder" : "FileText",
    };
  });

  // If user typed `@file:` or `@file`, only show files
  if (isFilterOnlyFiles) {
    return entryItems
      .filter((e) => e.category === "file")
      .filter((e) => !term || e.label.toLowerCase().includes(term) || e.description.toLowerCase().includes(term));
  }

  // If user typed `@folder:` or `@folder`, only show folders
  if (isFilterOnlyFolders) {
    return entryItems
      .filter((e) => e.category === "folder")
      .filter((e) => !term || e.label.toLowerCase().includes(term) || e.description.toLowerCase().includes(term));
  }

  const allItems = [...DEFAULT_CONTEXT_MENTIONS, ...entryItems];

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
