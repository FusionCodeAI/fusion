//! Internal URI router for resolving virtual schemes like `skill://` and `rule://`.
//!
//! Schemes supported:
//! - `skill://<name>`: Resolves to the skill markdown instructions (`SKILL.md` or `<name>.md`).
//! - `skill://<name>/<subpath>`: Resolves to a file within the skill directory.
//! - `rule://<name>`: Resolves to rule definition files in workspace or configuration paths
//!   (`.fusion/rules/<name>.md`, `.cursor/rules/<name>.mdc`, `.claude/rules/<name>.md`).

use std::path::{Path, PathBuf};

use crate::agent::skills::SkillRegistry;

/// Returns `true` if the raw path or URI begins with an HTTP or HTTPS scheme.
pub fn is_http_url(raw_path: &str) -> bool {
    let trimmed = raw_path.trim();
    trimmed.starts_with("http://") || trimmed.starts_with("https://")
}

/// Resolves an internal URI (`skill://...` or `rule://...`) to a concrete filesystem `PathBuf`.
///
/// Returns `Some(PathBuf)` if the URI matches a supported scheme and resolves to a path.
/// Returns `None` if the scheme is unrecognized or no matching file/directory is found.
/// HTTP and HTTPS URLs are recognized as external network URLs and pass through cleanly as `None`.
pub fn resolve_internal_uri(raw_path: &str, workspace_root: Option<&Path>) -> Option<PathBuf> {
    let trimmed = raw_path.trim();

    if is_http_url(trimmed) {
        return None;
    }

    if trimmed.starts_with("skill://") {
        resolve_skill_uri(trimmed, workspace_root)
    } else if trimmed.starts_with("rule://") {
        resolve_rule_uri(trimmed, workspace_root)
    } else if trimmed.starts_with("agent://") || trimmed.starts_with("artifact://") {
        resolve_agent_uri(trimmed, workspace_root)
    } else {
        None
    }
}

/// Resolves a `skill://...` URI.
///
/// Format:
/// - `skill://<name>`
/// - `skill://<name>/<subpath>`
fn resolve_skill_uri(raw_path: &str, workspace_root: Option<&Path>) -> Option<PathBuf> {
    let rest = raw_path.strip_prefix("skill://")?.trim();
    let rest = rest.trim_start_matches('/');
    if rest.is_empty() {
        return None;
    }

    let (skill_name, subpath) = match rest.split_once('/') {
        Some((name, sub)) => {
            let s = sub.trim().trim_start_matches('/');
            let sub_opt = if s.is_empty() { None } else { Some(s) };
            (name.trim(), sub_opt)
        }
        None => (rest.trim(), None),
    };

    if skill_name.is_empty() {
        return None;
    }

    // 1. Try resolving via SkillRegistry::scan_default(workspace_root)
    let registry = SkillRegistry::scan_default(workspace_root);
    let skill = registry.get(skill_name).or_else(|| {
        registry
            .list()
            .into_iter()
            .find(|s| s.name().eq_ignore_ascii_case(skill_name))
    });

    if let Some(skill) = skill {
        if let Some(path) = &skill.path {
            match subpath {
                None => {
                    return Some(path.clone());
                }
                Some(sub) => {
                    let base_dir = if path.is_file() {
                        path.parent().unwrap_or(path)
                    } else {
                        path.as_path()
                    };
                    let candidate = base_dir.join(sub);
                    if candidate.exists() {
                        return Some(candidate);
                    }
                }
            }
        }
    }

    // 2. Fallback checks:
    //    - <workspace>/.fusion/skills/<name>/SKILL.md (or <workspace>/.fusion/skills/<name>/<sub>)
    //    - <workspace>/.fusion/skills/<name>.md
    //    - <home>/.fusion/skills/<name>/SKILL.md (or <home>/.fusion/skills/<name>/<sub>)
    //    - <home>/.fusion/skills/<name>.md
    if let Some(ws) = workspace_root {
        if let Some(sub) = subpath {
            let candidate = ws.join(".fusion").join("skills").join(skill_name).join(sub);
            if candidate.exists() {
                return Some(candidate);
            }
        } else {
            let c1 = ws
                .join(".fusion")
                .join("skills")
                .join(skill_name)
                .join("SKILL.md");
            if c1.exists() {
                return Some(c1);
            }
            let c1_lower = ws
                .join(".fusion")
                .join("skills")
                .join(skill_name)
                .join("skill.md");
            if c1_lower.exists() {
                return Some(c1_lower);
            }
            let c2 = ws
                .join(".fusion")
                .join("skills")
                .join(format!("{skill_name}.md"));
            if c2.exists() {
                return Some(c2);
            }
        }
    }

    if let Some(home) = dirs::home_dir() {
        if let Some(sub) = subpath {
            let candidate = home
                .join(".fusion")
                .join("skills")
                .join(skill_name)
                .join(sub);
            if candidate.exists() {
                return Some(candidate);
            }
        } else {
            let c3 = home
                .join(".fusion")
                .join("skills")
                .join(skill_name)
                .join("SKILL.md");
            if c3.exists() {
                return Some(c3);
            }
            let c3_lower = home
                .join(".fusion")
                .join("skills")
                .join(skill_name)
                .join("skill.md");
            if c3_lower.exists() {
                return Some(c3_lower);
            }
            let c4 = home
                .join(".fusion")
                .join("skills")
                .join(format!("{skill_name}.md"));
            if c4.exists() {
                return Some(c4);
            }
        }
    }

    None
}
/// Resolves an `agent://<id_or_name>` or `artifact://<id>` URI to its persisted output file path.
fn resolve_agent_uri(raw_path: &str, workspace_root: Option<&Path>) -> Option<PathBuf> {
    let rest = raw_path
        .strip_prefix("agent://")
        .or_else(|| raw_path.strip_prefix("artifact://"))?
        .trim();
    let rest = rest.trim_start_matches('/').trim();
    if rest.is_empty() {
        return None;
    }

    let mut search_dirs = Vec::new();
    if let Some(root) = workspace_root {
        search_dirs.push(root.join(".fusion").join("artifacts"));
    }
    if let Ok(cwd) = std::env::current_dir() {
        search_dirs.push(cwd.join(".fusion").join("artifacts"));
    }
    search_dirs.push(crate::config::Config::config_dir().join("artifacts"));

    let candidates = [
        format!("{}.txt", rest),
        format!("{}.md", rest),
        rest.to_string(),
    ];

    for dir in &search_dirs {
        for candidate in &candidates {
            let path = dir.join(candidate);
            if path.exists() && path.is_file() {
                return Some(path);
            }
        }
    }

    None
}

/// Resolves a `rule://...` URI.
///
/// Checks in order:
/// 1. `<workspace>/.fusion/rules/<name>.md`
/// 2. `<workspace>/.cursor/rules/<name>.mdc`
/// 3. `<workspace>/.claude/rules/<name>.md`
fn resolve_rule_uri(raw_path: &str, workspace_root: Option<&Path>) -> Option<PathBuf> {
    let rest = raw_path.strip_prefix("rule://")?.trim();
    let name = rest.trim_start_matches('/');
    if name.is_empty() {
        return None;
    }

    let base_name = name
        .strip_suffix(".md")
        .or_else(|| name.strip_suffix(".mdc"))
        .unwrap_or(name);

    if let Some(ws) = workspace_root {
        // 1. <workspace>/.fusion/rules/<name>.md
        let fusion_rule = ws.join(".fusion").join("rules").join(format!("{base_name}.md"));
        if fusion_rule.exists() {
            return Some(fusion_rule);
        }

        // 2. <workspace>/.cursor/rules/<name>.mdc
        let cursor_rule = ws.join(".cursor").join("rules").join(format!("{base_name}.mdc"));
        if cursor_rule.exists() {
            return Some(cursor_rule);
        }

        // 3. <workspace>/.claude/rules/<name>.md
        let claude_rule = ws.join(".claude").join("rules").join(format!("{base_name}.md"));
        if claude_rule.exists() {
            return Some(claude_rule);
        }

        // Direct filename match if explicit extension was provided
        if name != base_name {
            let direct_fusion = ws.join(".fusion").join("rules").join(name);
            if direct_fusion.exists() {
                return Some(direct_fusion);
            }
            let direct_cursor = ws.join(".cursor").join("rules").join(name);
            if direct_cursor.exists() {
                return Some(direct_cursor);
            }
            let direct_claude = ws.join(".claude").join("rules").join(name);
            if direct_claude.exists() {
                return Some(direct_claude);
            }
        }
    }

    // Global / home fallback if workspace didn't match
    if let Some(home) = dirs::home_dir() {
        let home_fusion_rule = home
            .join(".fusion")
            .join("rules")
            .join(format!("{base_name}.md"));
        if home_fusion_rule.exists() {
            return Some(home_fusion_rule);
        }
    }

    None
}
