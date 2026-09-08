use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;

/// Definition of an LSP language server configuration.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LspServerDef {
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(rename = "fileTypes", default)]
    pub file_types: Vec<String>,
    #[serde(rename = "rootMarkers", default)]
    pub root_markers: Vec<String>,
    #[serde(default)]
    pub settings: serde_json::Value,
    #[serde(rename = "initOptions", default)]
    pub init_options: serde_json::Value,
    #[serde(rename = "isLinter", default)]
    pub is_linter: bool,
}

impl Default for LspServerDef {
    fn default() -> Self {
        Self {
            command: String::new(),
            args: Vec::new(),
            file_types: Vec::new(),
            root_markers: Vec::new(),
            settings: serde_json::Value::Null,
            init_options: serde_json::Value::Null,
            is_linter: false,
        }
    }
}

/// Canonical default priority order of servers based on OMP defaults.json.
pub const DEFAULT_SERVER_ORDER: &[&str] = &[
    "rust-analyzer",
    "tlaplus",
    "clangd",
    "zls",
    "gopls",
    "typescript-language-server",
    "typescript-native",
    "biome",
    "eslint",
    "denols",
    "vscode-html-language-server",
    "vscode-css-language-server",
    "vscode-json-language-server",
    "tailwindcss",
    "svelte",
    "vue-language-server",
    "astro",
    "pyright",
    "basedpyright",
    "pylsp",
    "ty",
    "ruff",
    "jdtls",
    "kotlin-lsp",
    "metals",
    "hls",
    "ocamllsp",
    "elixirls",
    "expert",
    "erlangls",
    "gleam",
    "solargraph",
    "ruby-lsp",
    "rubocop",
    "bashls",
    "lua-language-server",
    "intelephense",
    "phpactor",
    "omnisharp",
    "yamlls",
    "terraformls",
    "dockerls",
    "helm-ls",
    "nixd",
    "nil",
    "ols",
    "dartls",
    "marksman",
    "texlab",
    "graphql",
    "prismals",
    "vimls",
    "emmet-language-server",
    "sourcekit-lsp",
    "swiftlint",
];

/// Load and parse the embedded default language server configurations from `defaults.json`.
pub fn get_default_servers() -> HashMap<String, LspServerDef> {
    serde_json::from_str(include_str!("defaults.json"))
        .expect("embedded defaults.json should be valid JSON and match LspServerDef schema")
}

/// Helper to find a matching server from an arbitrary server map.
pub fn find_server_in_map(
    servers: &HashMap<String, LspServerDef>,
    path: &Path,
) -> Option<LspServerDef> {
    let path_str = path.to_string_lossy().to_lowercase();
    let file_name = path
        .file_name()
        .and_then(|n| n.to_str())
        .map(|s| s.to_lowercase());
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .map(|s| s.to_lowercase());
    let ext_with_dot = ext.as_ref().map(|e| format!(".{e}"));

    let mut matches: Vec<(&String, &LspServerDef)> = Vec::new();

    for (name, server) in servers {
        let supports_file = server.file_types.iter().any(|ft| {
            let ft_lower = ft.to_lowercase();
            let ft_no_dot = ft_lower.strip_prefix('.').unwrap_or(&ft_lower);

            ext_with_dot.as_ref().map_or(false, |dot_ext| {
                &ft_lower == dot_ext || ft_lower == ext.as_deref().unwrap_or_default()
            }) || file_name.as_ref().map_or(false, |fname| {
                &ft_lower == fname || ft_no_dot == fname.as_str()
            }) || path_str == ft_lower
                || path_str.ends_with(&format!(".{ft_no_dot}"))
        });

        if supports_file {
            matches.push((name, server));
        }
    }

    matches.sort_by(|(name_a, server_a), (name_b, server_b)| {
        let linter_cmp = server_a.is_linter.cmp(&server_b.is_linter);
        if linter_cmp != std::cmp::Ordering::Equal {
            return linter_cmp;
        }

        let order_a = DEFAULT_SERVER_ORDER
            .iter()
            .position(|&s| s == name_a.as_str())
            .unwrap_or(usize::MAX);
        let order_b = DEFAULT_SERVER_ORDER
            .iter()
            .position(|&s| s == name_b.as_str())
            .unwrap_or(usize::MAX);

        let order_cmp = order_a.cmp(&order_b);
        if order_cmp != std::cmp::Ordering::Equal {
            return order_cmp;
        }

        name_a.cmp(name_b)
    });

    matches.first().map(|(_, server)| (*server).clone())
}

/// Find the most suitable default LSP server definition for a given file path.
///
/// Matches against `fileTypes` (handling `.ext`, `ext`, and full file names like `Dockerfile`),
/// preferring primary non-linter servers over linters, and respecting standard server precedence.
pub fn find_server_for_file(path: &Path) -> Option<LspServerDef> {
    let servers = get_default_servers();
    find_server_in_map(&servers, path)
}
