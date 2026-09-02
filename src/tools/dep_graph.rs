//! Cargo Workspace Dependency Graph Tool.
//!
//! Runs `cargo metadata --format-version 1 --no-deps` (and optionally with
//! `--all-features`) in the target directory, parses the JSON output, and
//! renders an ASCII tree or Mermaid flowchart of inter-crate dependencies.
//!
//! The tool never touches `Cargo.lock` or the network beyond what `cargo
//! metadata` itself does; all graph logic is pure Rust.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet, HashMap, VecDeque};
use std::path::{Path, PathBuf};

use crate::tools::file::resolve_path;
use crate::tools::types::{Tool, ToolContext};

// ============================================================================
// cargo metadata JSON structures (subset we need)
// ============================================================================

/// A single crate/package from `cargo metadata`.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct MetaPackage {
    pub id: String,
    pub name: String,
    pub version: String,
    pub manifest_path: String,
    pub dependencies: Vec<MetaDependency>,
}

/// A dependency entry inside a `MetaPackage`.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct MetaDependency {
    pub name: String,
    pub req: String,
    #[serde(default)]
    pub kind: Option<String>, // null = normal, "dev", "build"
    #[serde(default)]
    pub optional: bool,
}

/// Top-level `cargo metadata` JSON output.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct CargoMetadata {
    pub packages: Vec<MetaPackage>,
    pub workspace_members: Vec<String>,
    #[serde(default)]
    pub workspace_root: String,
}

// ============================================================================
// Graph representation
// ============================================================================

/// A single node in the resolved dependency graph.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GraphNode {
    /// Package id (unique).
    pub id: String,
    /// Human-readable `name@version`.
    pub label: String,
    /// Whether this crate is a workspace member.
    pub is_workspace_member: bool,
}

/// Resolved dependency graph: adjacency list (id → sorted dep ids).
#[derive(Debug, Clone)]
pub struct DepGraph {
    /// All nodes, keyed by package id.
    pub nodes: BTreeMap<String, GraphNode>,
    /// Edges: from id → set of dependency ids.
    pub edges: BTreeMap<String, BTreeSet<String>>,
    /// Workspace member ids in declaration order.
    pub workspace_members: Vec<String>,
}

impl DepGraph {
    /// Build a `DepGraph` from parsed `CargoMetadata`.
    ///
    /// `dep_kinds` controls which dependency kinds are included.
    pub fn from_metadata(meta: &CargoMetadata, dep_kinds: DepKindFilter) -> Self {
        let member_set: BTreeSet<&str> =
            meta.workspace_members.iter().map(String::as_str).collect();

        // Index packages by id.
        let by_id: HashMap<&str, &MetaPackage> =
            meta.packages.iter().map(|p| (p.id.as_str(), p)).collect();

        // Build name → id index for resolving dependency names.
        let name_to_ids: HashMap<&str, Vec<&str>> = {
            let mut m: HashMap<&str, Vec<&str>> = HashMap::new();
            for p in &meta.packages {
                m.entry(p.name.as_str()).or_default().push(p.id.as_str());
            }
            m
        };

        let mut nodes = BTreeMap::new();
        let mut edges: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();

        for pkg in &meta.packages {
            let node = GraphNode {
                id: pkg.id.clone(),
                label: format!("{}@{}", pkg.name, pkg.version),
                is_workspace_member: member_set.contains(pkg.id.as_str()),
            };
            nodes.insert(pkg.id.clone(), node);
            edges.entry(pkg.id.clone()).or_default();

            for dep in &pkg.dependencies {
                // Apply kind filter.
                let kind_str = dep.kind.as_deref().unwrap_or("normal");
                if !dep_kinds.includes(kind_str) {
                    continue;
                }

                // Resolve dependency name to an id present in our package list.
                if let Some(ids) = name_to_ids.get(dep.name.as_str()) {
                    // Prefer the id that is itself a member; otherwise pick first.
                    let target_id = ids
                        .iter()
                        .find(|&&id| member_set.contains(id))
                        .or_else(|| ids.first())
                        .copied();

                    if let Some(tid) = target_id {
                        // Only add edge if the target package is actually in our list.
                        if by_id.contains_key(tid) && tid != pkg.id.as_str() {
                            edges
                                .entry(pkg.id.clone())
                                .or_default()
                                .insert(tid.to_string());
                        }
                    }
                }
            }
        }

        let workspace_members = meta.workspace_members.clone();

        Self {
            nodes,
            edges,
            workspace_members,
        }
    }

    /// Roots: workspace members with no incoming edges from other workspace members.
    pub fn workspace_roots(&self) -> Vec<&str> {
        let member_set: BTreeSet<&str> = self
            .workspace_members
            .iter()
            .map(String::as_str)
            .collect();

        // Collect all workspace-member ids that appear as targets of edges from
        // other workspace members.
        let mut has_incoming: BTreeSet<&str> = BTreeSet::new();
        for (from_id, deps) in &self.edges {
            if !member_set.contains(from_id.as_str()) {
                continue;
            }
            for dep_id in deps {
                if member_set.contains(dep_id.as_str()) {
                    has_incoming.insert(dep_id.as_str());
                }
            }
        }

        let mut roots: Vec<&str> = self
            .workspace_members
            .iter()
            .map(String::as_str)
            .filter(|id| !has_incoming.contains(id))
            .collect();

        // Stable sort by label.
        roots.sort_by_key(|id| {
            self.nodes.get(*id).map(|n| n.label.as_str()).unwrap_or(*id)
        });
        roots
    }
}

// ============================================================================
// Dependency kind filter
// ============================================================================

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DepKindFilter {
    /// Normal runtime deps only.
    Normal,
    /// Normal + dev deps.
    NormalAndDev,
    /// All (normal + dev + build).
    All,
}

impl DepKindFilter {
    fn from_str(s: &str) -> Self {
        match s.to_ascii_lowercase().as_str() {
            "dev" | "all_dev" => Self::NormalAndDev,
            "all" | "full" => Self::All,
            _ => Self::Normal,
        }
    }

    fn includes(self, kind: &str) -> bool {
        match self {
            Self::Normal => kind == "normal",
            Self::NormalAndDev => matches!(kind, "normal" | "dev"),
            Self::All => true,
        }
    }
}

// ============================================================================
// Output format
// ============================================================================

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GraphFormat {
    Ascii,
    Mermaid,
}

impl GraphFormat {
    fn from_str(s: &str) -> Self {
        match s.to_ascii_lowercase().as_str() {
            "mermaid" | "mmd" => Self::Mermaid,
            _ => Self::Ascii,
        }
    }
}

// ============================================================================
// ASCII tree renderer
// ============================================================================

/// Render the graph as an ASCII tree.
///
/// Traversal is BFS from workspace roots; each node is printed once.
/// When a node has already been visited its children are replaced by `(…)`.
pub fn render_ascii(graph: &DepGraph, max_depth: usize) -> String {
    let roots = graph.workspace_roots();
    if roots.is_empty() {
        // Fall back to all workspace members.
        if graph.workspace_members.is_empty() {
            return "(no workspace members found)".to_string();
        }
    }

    let root_ids: Vec<&str> = if roots.is_empty() {
        graph.workspace_members.iter().map(String::as_str).collect()
    } else {
        roots
    };

    let mut out = String::new();
    let mut visited: BTreeSet<String> = BTreeSet::new();

    for (i, root_id) in root_ids.iter().enumerate() {
        let is_last_root = i == root_ids.len() - 1;
        render_node_ascii(
            graph,
            root_id,
            "",
            is_last_root,
            0,
            max_depth,
            &mut visited,
            &mut out,
        );
    }

    // Summary line.
    let member_count = graph.workspace_members.len();
    let edge_count: usize = graph.edges.values().map(|s| s.len()).sum();
    out.push_str(&format!(
        "\n{} workspace member(s), {} dependency edge(s)\n",
        member_count, edge_count
    ));

    out
}

#[allow(clippy::too_many_arguments)]
fn render_node_ascii(
    graph: &DepGraph,
    id: &str,
    prefix: &str,
    is_last: bool,
    depth: usize,
    max_depth: usize,
    visited: &mut BTreeSet<String>,
    out: &mut String,
) {
    let connector = if depth == 0 {
        ""
    } else if is_last {
        "└── "
    } else {
        "├── "
    };

    let label = graph
        .nodes
        .get(id)
        .map(|n| {
            if n.is_workspace_member {
                format!("[{}]", n.label)
            } else {
                n.label.clone()
            }
        })
        .unwrap_or_else(|| id.to_string());

    let already_visited = visited.contains(id);
    let display_label = if already_visited {
        format!("{} (*)", label)
    } else {
        label
    };

    out.push_str(&format!("{}{}{}\n", prefix, connector, display_label));

    if already_visited || depth >= max_depth {
        return;
    }
    visited.insert(id.to_string());

    let empty = BTreeSet::new();
    let children: Vec<&str> = graph
        .edges
        .get(id)
        .unwrap_or(&empty)
        .iter()
        .map(String::as_str)
        .collect();

    let child_prefix = if depth == 0 {
        prefix.to_string()
    } else if is_last {
        format!("{}    ", prefix)
    } else {
        format!("{}│   ", prefix)
    };

    for (i, child_id) in children.iter().enumerate() {
        let is_last_child = i == children.len() - 1;
        render_node_ascii(
            graph,
            child_id,
            &child_prefix,
            is_last_child,
            depth + 1,
            max_depth,
            visited,
            out,
        );
    }
}

// ============================================================================
// Mermaid renderer
// ============================================================================

/// Render the graph as a Mermaid `graph TD` diagram.
pub fn render_mermaid(graph: &DepGraph) -> String {
    let mut out = String::from("```mermaid\ngraph TD\n");

    // Emit node definitions with sanitized ids.
    for (id, node) in &graph.nodes {
        let mid = mermaid_id(id);
        let shape = if node.is_workspace_member {
            format!("{}[\"{}\"]", mid, node.label)
        } else {
            format!("{}(\"{}\")", mid, node.label)
        };
        out.push_str(&format!("    {}\n", shape));
    }

    out.push('\n');

    // Emit edges.
    let mut seen_edges: BTreeSet<(String, String)> = BTreeSet::new();
    for (from_id, deps) in &graph.edges {
        for dep_id in deps {
            let key = (from_id.clone(), dep_id.clone());
            if seen_edges.insert(key) {
                out.push_str(&format!(
                    "    {} --> {}\n",
                    mermaid_id(from_id),
                    mermaid_id(dep_id)
                ));
            }
        }
    }

    out.push_str("```\n");
    out
}

/// Produce a safe Mermaid node id from a package id string.
fn mermaid_id(id: &str) -> String {
    // Replace non-alphanumeric chars with underscores; prefix with 'n' if
    // the id starts with a digit.
    let mut s: String = id
        .chars()
        .map(|c| if c.is_alphanumeric() { c } else { '_' })
        .collect();
    if s.starts_with(|c: char| c.is_ascii_digit()) {
        s.insert(0, 'n');
    }
    s
}

// ============================================================================
// cargo metadata runner
// ============================================================================

/// Run `cargo metadata` in `dir` and return parsed output.
///
/// `all_features`: pass `--all-features`; when false passes `--no-default-features`
/// to avoid feature-resolution errors in incomplete workspaces.
pub async fn run_cargo_metadata(
    dir: &Path,
    include_transitive: bool,
) -> anyhow::Result<CargoMetadata> {
    use tokio::process::Command;

    let mut cmd = Command::new("cargo");
    cmd.arg("metadata")
        .arg("--format-version")
        .arg("1")
        .arg("--color")
        .arg("never");

    if !include_transitive {
        cmd.arg("--no-deps");
    }

    cmd.current_dir(dir);

    let output = cmd.output().await?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!(
            "cargo metadata failed (exit {}): {}",
            output.status,
            stderr.trim()
        );
    }

    let stdout = String::from_utf8(output.stdout)?;
    let meta: CargoMetadata = serde_json::from_str(&stdout)
        .map_err(|e| anyhow::anyhow!("Failed to parse cargo metadata output: {}", e))?;

    Ok(meta)
}

// ============================================================================
// Tool Implementation
// ============================================================================

/// Renders a dependency graph for the Cargo workspace rooted at the target path.
#[derive(Default, Debug, Clone)]
pub struct DepGraphTool;

impl DepGraphTool {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl Tool for DepGraphTool {
    fn name(&self) -> &str {
        "dep_graph"
    }

    fn description(&self) -> &str {
        "Build and render a dependency graph for the Cargo workspace at the given path. \
         Runs `cargo metadata` to discover workspace crates and their relationships, then \
         renders an ASCII tree (default) or Mermaid flowchart. Workspace members are shown in \
         square brackets; external crates in parentheses. Nodes marked (*) were already printed \
         higher in the tree."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "Path to the Cargo workspace root (directory containing Cargo.toml). Defaults to '.'."
                },
                "format": {
                    "type": "string",
                    "enum": ["ascii", "mermaid"],
                    "description": "Output format: 'ascii' (default) renders an indented tree; 'mermaid' renders a Mermaid flowchart block."
                },
                "deps": {
                    "type": "string",
                    "enum": ["normal", "dev", "all"],
                    "description": "Which dependency kinds to include: 'normal' (default, runtime deps only), 'dev' (normal + dev), 'all' (normal + dev + build)."
                },
                "transitive": {
                    "type": "boolean",
                    "description": "Whether to include transitive (non-workspace) dependencies. When false (default), only intra-workspace relationships are shown, which is much faster."
                },
                "max_depth": {
                    "type": "integer",
                    "description": "Maximum tree depth for ASCII output (default 10). Has no effect on mermaid output."
                }
            }
        })
    }

    async fn execute(&self, args: Value, ctx: &ToolContext) -> anyhow::Result<String> {
        let path_arg = args
            .get("path")
            .and_then(|v| v.as_str())
            .unwrap_or(".");

        let format = GraphFormat::from_str(
            args.get("format").and_then(|v| v.as_str()).unwrap_or("ascii"),
        );

        let dep_kinds = DepKindFilter::from_str(
            args.get("deps").and_then(|v| v.as_str()).unwrap_or("normal"),
        );

        let include_transitive = args
            .get("transitive")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        let max_depth = args
            .get("max_depth")
            .and_then(|v| v.as_u64())
            .unwrap_or(10) as usize;

        let target_path = resolve_path(path_arg, &ctx.cwd);

        if !target_path.exists() {
            return Ok(format!(
                "Error: path does not exist: {}",
                target_path.display()
            ));
        }

        // Locate the Cargo.toml so we run metadata in the right directory.
        let workspace_dir = if target_path.is_dir() {
            target_path.clone()
        } else if target_path.file_name().map(|n| n == "Cargo.toml").unwrap_or(false) {
            target_path
                .parent()
                .unwrap_or(Path::new("."))
                .to_path_buf()
        } else {
            target_path.clone()
        };

        // Check cargo is available.
        let cargo_check = tokio::process::Command::new("cargo")
            .arg("--version")
            .output()
            .await;

        if cargo_check.is_err() || !cargo_check.unwrap().status.success() {
            return Ok("Error: `cargo` is not available in PATH. Install Rust from https://rustup.rs".to_string());
        }

        let meta = match run_cargo_metadata(&workspace_dir, include_transitive).await {
            Ok(m) => m,
            Err(e) => return Ok(format!("Error: {}", e)),
        };

        if meta.workspace_members.is_empty() {
            return Ok(format!(
                "No workspace members found at {}. Is this a Cargo workspace?",
                workspace_dir.display()
            ));
        }

        let graph = DepGraph::from_metadata(&meta, dep_kinds);

        let rendered = match format {
            GraphFormat::Ascii => render_ascii(&graph, max_depth),
            GraphFormat::Mermaid => render_mermaid(&graph),
        };

        // Prepend a brief header.
        let header = format!(
            "Cargo dependency graph — {}\n{}\n\n",
            meta.workspace_root
                .split(['/', '\\'])
                .filter(|s| !s.is_empty())
                .last()
                .unwrap_or(&meta.workspace_root),
            "─".repeat(60)
        );

        Ok(format!("{}{}", header, rendered))
    }
}

// ============================================================================
// Unit Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn make_meta(packages: Vec<MetaPackage>, members: Vec<String>) -> CargoMetadata {
        CargoMetadata {
            packages,
            workspace_members: members,
            workspace_root: "/test".to_string(),
        }
    }

    fn pkg(id: &str, name: &str, version: &str, deps: Vec<(&str, &str)>) -> MetaPackage {
        MetaPackage {
            id: id.to_string(),
            name: name.to_string(),
            version: version.to_string(),
            manifest_path: format!("/test/{}/Cargo.toml", name),
            dependencies: deps
                .into_iter()
                .map(|(n, r)| MetaDependency {
                    name: n.to_string(),
                    req: r.to_string(),
                    kind: None,
                    optional: false,
                })
                .collect(),
        }
    }

    #[test]
    fn test_graph_from_simple_workspace() {
        // alpha → beta
        let packages = vec![
            pkg("alpha 0.1.0", "alpha", "0.1.0", vec![("beta", "^0.1")]),
            pkg("beta 0.1.0", "beta", "0.1.0", vec![]),
        ];
        let members = vec!["alpha 0.1.0".to_string(), "beta 0.1.0".to_string()];
        let meta = make_meta(packages, members);
        let graph = DepGraph::from_metadata(&meta, DepKindFilter::Normal);

        assert_eq!(graph.nodes.len(), 2);
        assert!(graph.edges["alpha 0.1.0"].contains("beta 0.1.0"));
        assert!(graph.edges["beta 0.1.0"].is_empty());
    }

    #[test]
    fn test_workspace_roots_single_root() {
        // alpha → beta  =>  alpha is root
        let packages = vec![
            pkg("alpha 0.1.0", "alpha", "0.1.0", vec![("beta", "^0.1")]),
            pkg("beta 0.1.0", "beta", "0.1.0", vec![]),
        ];
        let members = vec!["alpha 0.1.0".to_string(), "beta 0.1.0".to_string()];
        let meta = make_meta(packages, members);
        let graph = DepGraph::from_metadata(&meta, DepKindFilter::Normal);
        let roots = graph.workspace_roots();
        assert_eq!(roots, vec!["alpha 0.1.0"]);
    }

    #[test]
    fn test_ascii_tree_contains_labels() {
        let packages = vec![
            pkg("alpha 0.1.0", "alpha", "0.1.0", vec![("beta", "^0.1")]),
            pkg("beta 0.1.0", "beta", "0.1.0", vec![]),
        ];
        let members = vec!["alpha 0.1.0".to_string(), "beta 0.1.0".to_string()];
        let meta = make_meta(packages, members);
        let graph = DepGraph::from_metadata(&meta, DepKindFilter::Normal);
        let output = render_ascii(&graph, 10);

        assert!(output.contains("alpha@0.1.0"), "missing alpha: {}", output);
        assert!(output.contains("beta@0.1.0"), "missing beta: {}", output);
        assert!(output.contains("└──") || output.contains("├──"), "missing tree connector: {}", output);
    }

    #[test]
    fn test_mermaid_output_format() {
        let packages = vec![
            pkg("alpha 0.1.0", "alpha", "0.1.0", vec![("beta", "^0.1")]),
            pkg("beta 0.1.0", "beta", "0.1.0", vec![]),
        ];
        let members = vec!["alpha 0.1.0".to_string(), "beta 0.1.0".to_string()];
        let meta = make_meta(packages, members);
        let graph = DepGraph::from_metadata(&meta, DepKindFilter::Normal);
        let output = render_mermaid(&graph);

        assert!(output.starts_with("```mermaid\ngraph TD\n"), "bad header: {}", output);
        assert!(output.contains("-->"), "missing edge: {}", output);
        assert!(output.ends_with("```\n"), "bad footer: {}", output);
    }

    #[test]
    fn test_dev_deps_filtered() {
        let packages = vec![
            pkg("alpha 0.1.0", "alpha", "0.1.0", vec![]),
            MetaPackage {
                id: "alpha 0.1.0".to_string(),
                name: "alpha".to_string(),
                version: "0.1.0".to_string(),
                manifest_path: "/test/alpha/Cargo.toml".to_string(),
                dependencies: vec![MetaDependency {
                    name: "beta".to_string(),
                    req: "^0.1".to_string(),
                    kind: Some("dev".to_string()),
                    optional: false,
                }],
            },
            pkg("beta 0.1.0", "beta", "0.1.0", vec![]),
        ];
        let packages = vec![
            MetaPackage {
                id: "alpha 0.1.0".to_string(),
                name: "alpha".to_string(),
                version: "0.1.0".to_string(),
                manifest_path: "/test/alpha/Cargo.toml".to_string(),
                dependencies: vec![MetaDependency {
                    name: "beta".to_string(),
                    req: "^0.1".to_string(),
                    kind: Some("dev".to_string()),
                    optional: false,
                }],
            },
            pkg("beta 0.1.0", "beta", "0.1.0", vec![]),
        ];
        let members = vec!["alpha 0.1.0".to_string(), "beta 0.1.0".to_string()];
        let meta = make_meta(packages, members);

        // Normal filter: dev dep excluded → no edge.
        let graph_normal = DepGraph::from_metadata(&meta, DepKindFilter::Normal);
        assert!(
            graph_normal.edges["alpha 0.1.0"].is_empty(),
            "normal filter should exclude dev dep"
        );

        // NormalAndDev filter: dev dep included → edge present.
        let graph_dev = DepGraph::from_metadata(&meta, DepKindFilter::NormalAndDev);
        assert!(
            graph_dev.edges["alpha 0.1.0"].contains("beta 0.1.0"),
            "dev filter should include dev dep"
        );
    }

    #[test]
    fn test_mermaid_id_sanitization() {
        assert_eq!(mermaid_id("foo bar"), "foo_bar");
        assert_eq!(mermaid_id("0foo"), "n0foo");
        assert_eq!(mermaid_id("my-crate 1.0.0 (registry+…)"), "my_crate_1_0_0__registry____");
    }

    #[test]
    fn test_already_visited_marker() {
        // diamond: root → a, root → b, a → shared, b → shared
        let packages = vec![
            pkg("root 0.1.0", "root", "0.1.0", vec![("a", "^0.1"), ("b", "^0.1")]),
            pkg("a 0.1.0", "a", "0.1.0", vec![("shared", "^0.1")]),
            pkg("b 0.1.0", "b", "0.1.0", vec![("shared", "^0.1")]),
            pkg("shared 0.1.0", "shared", "0.1.0", vec![]),
        ];
        let members = vec![
            "root 0.1.0".to_string(),
            "a 0.1.0".to_string(),
            "b 0.1.0".to_string(),
            "shared 0.1.0".to_string(),
        ];
        let meta = make_meta(packages, members);
        let graph = DepGraph::from_metadata(&meta, DepKindFilter::Normal);
        let output = render_ascii(&graph, 10);
        // "shared" should appear once without (*) and once with (*).
        let count_plain = output.matches("shared@0.1.0]").count();
        let count_visited = output.matches("shared@0.1.0] (*)").count();
        assert_eq!(
            count_plain - count_visited,
            1,
            "shared should appear once without (*): {}",
            output
        );
        assert!(count_visited >= 1, "shared should appear at least once with (*): {}", output);
    }
}
