//! Codebase Dependency Auditor Tool.
//!
//! Scans, parses, and audits project dependency manifests (`Cargo.toml`, `package.json`,
//! `requirements.txt`, `pyproject.toml`, `Pipfile`, `go.mod`, `Gemfile`) across workspaces.
//! Compares declared and resolved versions against upstream registries (crates.io, npm,
//! PyPI, Go proxy, RubyGems) to identify outdated packages (major, minor, patch bumps),
//! detect known security advisories, and compute workspace dependency health scores.
//!
//! Features:
//! - Multi-ecosystem manifest parsing (Rust Cargo, Node.js npm, Python pip/poetry, Go modules, Ruby gems)
//! - Lockfile resolution (`Cargo.lock`, `package-lock.json`)
//! - Pure-Rust SemVer parsing and semver-diff categorization (Major/Minor/Patch)
//! - Online registry querying with in-memory caching and graceful offline fallback
//! - Curated static security advisory / vulnerability database
//! - Dependency health score rating (A through F)
//! - Multiple output formats: `table` (ASCII/colored), `markdown`, `json`, `summary`

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use crate::tools::file::resolve_path;
use crate::tools::types::{Tool, ToolContext};

// ===========================================================================
// Core Data Models & Enums
// ===========================================================================

/// Package ecosystem / package manager type.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Ecosystem {
    Cargo,
    Npm,
    PyPI,
    Go,
    Gem,
    Generic,
}

impl Ecosystem {
    pub fn as_str(&self) -> &'static str {
        match self {
            Ecosystem::Cargo => "cargo",
            Ecosystem::Npm => "npm",
            Ecosystem::PyPI => "pypi",
            Ecosystem::Go => "go",
            Ecosystem::Gem => "gem",
            Ecosystem::Generic => "generic",
        }
    }

    pub fn display_name(&self) -> &'static str {
        match self {
            Ecosystem::Cargo => "Rust (Cargo)",
            Ecosystem::Npm => "Node.js (npm)",
            Ecosystem::PyPI => "Python (PyPI)",
            Ecosystem::Go => "Go (Modules)",
            Ecosystem::Gem => "Ruby (Gems)",
            Ecosystem::Generic => "Generic",
        }
    }

    pub fn default_manifest(&self) -> &'static str {
        match self {
            Ecosystem::Cargo => "Cargo.toml",
            Ecosystem::Npm => "package.json",
            Ecosystem::PyPI => "requirements.txt",
            Ecosystem::Go => "go.mod",
            Ecosystem::Gem => "Gemfile",
            Ecosystem::Generic => "dependencies.txt",
        }
    }

    pub fn from_str_loose(s: &str) -> Option<Self> {
        let clean = s.trim().to_ascii_lowercase();
        match clean.as_str() {
            "cargo" | "rust" | "crate" | "crates" | "cargo.toml" => Some(Ecosystem::Cargo),
            "npm" | "node" | "javascript" | "typescript" | "js" | "ts" | "package.json" => {
                Some(Ecosystem::Npm)
            }
            "pypi" | "python" | "pip" | "pipfile" | "poetry" | "requirements.txt" | "pyproject.toml" => {
                Some(Ecosystem::PyPI)
            }
            "go" | "golang" | "go.mod" => Some(Ecosystem::Go),
            "gem" | "ruby" | "rubygems" | "gemfile" => Some(Ecosystem::Gem),
            "all" | "*" | "" => None,
            _ => Some(Ecosystem::Generic),
        }
    }
}

/// Dependency category (e.g. production runtime vs test/dev dependency).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DependencyType {
    Normal,
    Dev,
    Build,
    Peer,
    Optional,
    Workspace,
}

impl DependencyType {
    pub fn as_str(&self) -> &'static str {
        match self {
            DependencyType::Normal => "prod",
            DependencyType::Dev => "dev",
            DependencyType::Build => "build",
            DependencyType::Peer => "peer",
            DependencyType::Optional => "optional",
            DependencyType::Workspace => "workspace",
        }
    }

    pub fn matches_filter(&self, filter: &str) -> bool {
        let clean = filter.trim().to_ascii_lowercase();
        match clean.as_str() {
            "all" | "*" | "" => true,
            "prod" | "production" | "normal" | "runtime" => *self == DependencyType::Normal,
            "dev" | "development" | "test" => *self == DependencyType::Dev,
            "build" => *self == DependencyType::Build,
            "peer" => *self == DependencyType::Peer,
            "optional" => *self == DependencyType::Optional,
            "workspace" => *self == DependencyType::Workspace,
            _ => true,
        }
    }
}

/// Categorization of semantic version upgrade step.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum BumpType {
    Major,
    Minor,
    Patch,
    Prerelease,
    Other,
}

impl BumpType {
    pub fn as_str(&self) -> &'static str {
        match self {
            BumpType::Major => "major",
            BumpType::Minor => "minor",
            BumpType::Patch => "patch",
            BumpType::Prerelease => "prerelease",
            BumpType::Other => "other",
        }
    }

    pub fn badge(&self) -> &'static str {
        match self {
            BumpType::Major => "[MAJOR]",
            BumpType::Minor => "[MINOR]",
            BumpType::Patch => "[PATCH]",
            BumpType::Prerelease => "[PRE]",
            BumpType::Other => "[UPDATE]",
        }
    }
}

/// Status of dependency freshness compared to upstream.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "status", content = "bump", rename_all = "snake_case")]
pub enum OutdatedStatus {
    UpToDate,
    Outdated(BumpType),
    Unknown,
    NotFound,
    ConstraintOnly,
}

impl OutdatedStatus {
    pub fn is_outdated(&self) -> bool {
        matches!(self, OutdatedStatus::Outdated(_))
    }

    pub fn badge(&self) -> &'static str {
        match self {
            OutdatedStatus::UpToDate => "[OK]",
            OutdatedStatus::Outdated(b) => b.badge(),
            OutdatedStatus::Unknown => "[?]",
            OutdatedStatus::NotFound => "[NOT FOUND]",
            OutdatedStatus::ConstraintOnly => "[RANGE]",
        }
    }
}

/// Known security advisory metadata.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SecurityAdvisory {
    pub id: String,
    pub title: String,
    pub severity: String,
    pub affected_range: String,
    pub patched_version: String,
}

/// A parsed package dependency.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Dependency {
    pub name: String,
    pub requirement: String,
    pub resolved_version: Option<String>,
    pub latest_version: Option<String>,
    pub dep_type: DependencyType,
    pub ecosystem: Ecosystem,
    pub manifest_path: String,
    pub status: OutdatedStatus,
    pub description: Option<String>,
    pub homepage: Option<String>,
    pub advisories: Vec<SecurityAdvisory>,
}

impl Dependency {
    pub fn new(
        name: impl Into<String>,
        requirement: impl Into<String>,
        dep_type: DependencyType,
        ecosystem: Ecosystem,
        manifest_path: impl Into<String>,
    ) -> Self {
        Self {
            name: name.into(),
            requirement: requirement.into(),
            resolved_version: None,
            latest_version: None,
            dep_type,
            ecosystem,
            manifest_path: manifest_path.into(),
            status: OutdatedStatus::Unknown,
            description: None,
            homepage: None,
            advisories: Vec::new(),
        }
    }

    pub fn is_outdated(&self) -> bool {
        self.status.is_outdated()
    }

    pub fn is_vulnerable(&self) -> bool {
        !self.advisories.is_empty()
    }
}

/// Scan and audit result for a single manifest file.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ManifestScanResult {
    pub path: String,
    pub ecosystem: Ecosystem,
    pub dependencies: Vec<Dependency>,
    pub total_count: usize,
    pub up_to_date_count: usize,
    pub outdated_count: usize,
    pub major_count: usize,
    pub minor_count: usize,
    pub patch_count: usize,
    pub vulnerable_count: usize,
    pub health_score: u32,
}

/// Overall multi-manifest audit report.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuditReport {
    pub workspace_root: String,
    pub manifests: Vec<ManifestScanResult>,
    pub total_dependencies: usize,
    pub total_outdated: usize,
    pub total_major: usize,
    pub total_minor: usize,
    pub total_patch: usize,
    pub total_vulnerable: usize,
    pub overall_health_score: u32,
    pub health_rating: String,
    pub recommendations: Vec<String>,
}

// ===========================================================================
// Lightweight Pure-Rust SemVer Implementation
// ===========================================================================

/// Pure-Rust Semantic Version parser and comparator.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct SemVer {
    pub major: u64,
    pub minor: u64,
    pub patch: u64,
    pub pre: Option<String>,
    pub build: Option<String>,
    pub raw: String,
}

impl SemVer {
    /// Attempt to parse a semantic version string.
    pub fn parse(input: &str) -> Option<Self> {
        let clean = input.trim();
        if clean.is_empty() {
            return None;
        }

        // Strip leading constraint / syntax characters
        let s = clean
            .trim_start_matches(|c: char| c == 'v' || c == '=' || c == '^' || c == '~' || c == '>' || c == '<' || c == '@' || c == ' ')
            .trim();

        if s.is_empty() {
            return None;
        }

        // Split build metadata (+...)
        let (without_build, build) = if let Some(idx) = s.find('+') {
            (&s[..idx], Some(s[idx + 1..].to_string()))
        } else {
            (s, None)
        };

        // Split prerelease (-...)
        let (core, pre) = if let Some(idx) = without_build.find('-') {
            (&without_build[..idx], Some(without_build[idx + 1..].to_string()))
        } else {
            (without_build, None)
        };

        let parts: Vec<&str> = core.split('.').collect();
        if parts.is_empty() || parts.len() > 3 {
            return None;
        }

        let major = parts[0].parse::<u64>().ok()?;
        let minor = if parts.len() > 1 {
            parts[1].parse::<u64>().ok()?
        } else {
            0
        };
        let patch = if parts.len() > 2 {
            parts[2].parse::<u64>().ok()?
        } else {
            0
        };

        Some(Self {
            major,
            minor,
            patch,
            pre,
            build,
            raw: clean.to_string(),
        })
    }

    /// Compare two SemVers. Returns std::cmp::Ordering.
    pub fn cmp_version(&self, other: &SemVer) -> std::cmp::Ordering {
        if self.major != other.major {
            return self.major.cmp(&other.major);
        }
        if self.minor != other.minor {
            return self.minor.cmp(&other.minor);
        }
        if self.patch != other.patch {
            return self.patch.cmp(&other.patch);
        }

        // Prereleases have lower precedence than normal releases (e.g. 1.0.0-alpha < 1.0.0)
        match (&self.pre, &other.pre) {
            (None, None) => std::cmp::Ordering::Equal,
            (Some(_), None) => std::cmp::Ordering::Less,
            (None, Some(_)) => std::cmp::Ordering::Greater,
            (Some(a), Some(b)) => a.cmp(b),
        }
    }

    /// Calculate the bump type from self (current/declared) to latest.
    pub fn bump_to(&self, latest: &SemVer, is_cargo_semantics: bool) -> Option<BumpType> {
        if self.cmp_version(latest) == std::cmp::Ordering::Greater || self == latest {
            return None;
        }

        // Cargo semantics for 0.x.y:
        // 0.2.0 -> 0.3.0 is a breaking (Major) bump.
        // 0.0.1 -> 0.0.2 is a breaking (Major) bump.
        if is_cargo_semantics && self.major == 0 {
            if self.minor == 0 {
                if self.patch < latest.patch || latest.minor > 0 || latest.major > 0 {
                    return Some(BumpType::Major);
                }
            } else if self.minor < latest.minor || latest.major > 0 {
                return Some(BumpType::Major);
            } else if self.patch < latest.patch {
                return Some(BumpType::Patch);
            }
        }

        if self.major < latest.major {
            Some(BumpType::Major)
        } else if self.minor < latest.minor {
            Some(BumpType::Minor)
        } else if self.patch < latest.patch {
            Some(BumpType::Patch)
        } else if self.pre.is_some() && latest.pre.is_none() {
            Some(BumpType::Prerelease)
        } else {
            Some(BumpType::Other)
        }
    }
}

/// Extract base / minimum version from requirement string.
pub fn extract_base_version(req: &str) -> Option<String> {
    let clean = req.trim();
    if clean.is_empty() || clean == "*" || clean == "latest" || clean == "workspace" {
        return None;
    }

    // Split multiple clauses (e.g. ">= 1.2.0, < 2.0.0")
    let first_clause = clean.split(',').next().unwrap_or(clean).trim();

    // Find first digit
    if let Some(pos) = first_clause.find(|c: char| c.is_ascii_digit()) {
        let candidate = &first_clause[pos..];
        // Stop at whitespace or semicolon
        let end = candidate
            .find(|c: char| c.is_whitespace() || c == ';' || c == ',')
            .unwrap_or(candidate.len());
        let ver = candidate[..end].trim();
        if !ver.is_empty() {
            return Some(ver.to_string());
        }
    }

    None
}

// ===========================================================================
// Manifest Parsers
// ===========================================================================

/// Parse `Cargo.toml` dependencies.
pub fn parse_cargo_toml(content: &str, path: &str) -> Vec<Dependency> {
    let mut deps = Vec::new();
    let mut current_section: Option<(&str, DependencyType)> = None;

    for line in content.lines() {
        let trimmed = line.trim();

        // Skip empty lines and comments
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }

        // Section header
        if trimmed.starts_with('[') && trimmed.ends_with(']') {
            let section = trimmed.trim_start_matches('[').trim_end_matches(']').trim();

            if section == "dependencies" || section.ends_with(".dependencies") && !section.contains("dev-dependencies") && !section.contains("build-dependencies") {
                current_section = Some((section, DependencyType::Normal));
            } else if section == "dev-dependencies" || section.ends_with(".dev-dependencies") {
                current_section = Some((section, DependencyType::Dev));
            } else if section == "build-dependencies" || section.ends_with(".build-dependencies") {
                current_section = Some((section, DependencyType::Build));
            } else if section == "workspace.dependencies" {
                current_section = Some((section, DependencyType::Workspace));
            } else {
                current_section = None;
            }
            continue;
        }

        let Some((_, dep_type)) = current_section else {
            continue;
        };

        // Parse key = value line
        if let Some((key_part, val_part)) = trimmed.split_once('=') {
            let name = key_part.trim().trim_matches('"').trim_matches('\'').to_string();
            if name.is_empty() {
                continue;
            }

            let val = val_part.split('#').next().unwrap_or(val_part).trim();

            // Case 1: Simple string `serde = "1.0.100"`
            if val.starts_with('"') && val.ends_with('"') || (val.starts_with('\'') && val.ends_with('\'')) {
                let req = val.trim_matches('"').trim_matches('\'').to_string();
                let mut dep = Dependency::new(name, req.clone(), dep_type, Ecosystem::Cargo, path);
                if let Some(base) = extract_base_version(&req) {
                    dep.resolved_version = Some(base);
                }
                deps.push(dep);
            }
            // Case 2: Inline table `tokio = { version = "1.0", features = [...] }`
            else if val.starts_with('{') {
                let mut version_req = String::new();
                let mut is_workspace = false;
                let mut path_or_git = false;

                // Extract `version = "..."`
                if let Some(v_idx) = val.find("version") {
                    let rest = &val[v_idx + 7..];
                    if let Some(eq_idx) = rest.find('=') {
                        let v_val = rest[eq_idx + 1..].trim();
                        if let Some(start_quote) = v_val.find(|c| c == '"' || c == '\'') {
                            let quote_char = v_val.chars().nth(start_quote).unwrap();
                            let after_quote = &v_val[start_quote + 1..];
                            if let Some(end_quote) = after_quote.find(quote_char) {
                                version_req = after_quote[..end_quote].to_string();
                            }
                        }
                    }
                }

                if val.contains("workspace = true") || val.contains("workspace=true") {
                    is_workspace = true;
                }
                if (val.contains("path =") || val.contains("path=")) && version_req.is_empty() {
                    path_or_git = true;
                }

                let final_req = if !version_req.is_empty() {
                    version_req
                } else if is_workspace {
                    "workspace".to_string()
                } else if path_or_git {
                    "path".to_string()
                } else {
                    "*".to_string()
                };

                let mut dep = Dependency::new(name, final_req.clone(), dep_type, Ecosystem::Cargo, path);
                if let Some(base) = extract_base_version(&final_req) {
                    dep.resolved_version = Some(base);
                }
                deps.push(dep);
            }
        }
    }

    deps
}

/// Parse `Cargo.lock` to extract exact resolved versions.
pub fn parse_cargo_lock(content: &str) -> HashMap<String, String> {
    let mut resolved = HashMap::new();
    let mut current_name: Option<String> = None;

    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed == "[[package]]" {
            current_name = None;
            continue;
        }

        if let Some((k, v)) = trimmed.split_once('=') {
            let key = k.trim();
            let val = v.trim().trim_matches('"').trim_matches('\'');
            if key == "name" {
                current_name = Some(val.to_string());
            } else if key == "version" {
                if let Some(name) = &current_name {
                    resolved.insert(name.clone(), val.to_string());
                }
            }
        }
    }

    resolved
}

/// Parse `package.json` dependencies.
pub fn parse_package_json(content: &str, path: &str) -> Vec<Dependency> {
    let mut deps = Vec::new();
    let Ok(json_val): Result<Value, _> = serde_json::from_str(content) else {
        return deps;
    };

    let parse_section = |map_key: &str, dep_type: DependencyType, target: &mut Vec<Dependency>| {
        if let Some(obj) = json_val.get(map_key).and_then(|v| v.as_object()) {
            for (name, ver_val) in obj {
                if let Some(ver_str) = ver_val.as_str() {
                    let mut dep = Dependency::new(
                        name.clone(),
                        ver_str.to_string(),
                        dep_type,
                        Ecosystem::Npm,
                        path,
                    );
                    if let Some(base) = extract_base_version(ver_str) {
                        dep.resolved_version = Some(base);
                    }
                    target.push(dep);
                }
            }
        }
    };

    parse_section("dependencies", DependencyType::Normal, &mut deps);
    parse_section("devDependencies", DependencyType::Dev, &mut deps);
    parse_section("peerDependencies", DependencyType::Peer, &mut deps);
    parse_section("optionalDependencies", DependencyType::Optional, &mut deps);

    deps
}

/// Parse `package-lock.json` for exact resolved versions.
pub fn parse_package_lock_json(content: &str) -> HashMap<String, String> {
    let mut resolved = HashMap::new();
    let Ok(val): Result<Value, _> = serde_json::from_str(content) else {
        return resolved;
    };

    // Format v2 / v3: `packages["node_modules/foo"].version`
    if let Some(packages) = val.get("packages").and_then(|p| p.as_object()) {
        for (pkg_path, pkg_info) in packages {
            if let Some(ver) = pkg_info.get("version").and_then(|v| v.as_str()) {
                if let Some(name) = pkg_path.strip_prefix("node_modules/") {
                    resolved.insert(name.to_string(), ver.to_string());
                }
            }
        }
    }

    // Format v1: `dependencies.foo.version`
    if let Some(dependencies) = val.get("dependencies").and_then(|d| d.as_object()) {
        for (name, dep_info) in dependencies {
            if let Some(ver) = dep_info.get("version").and_then(|v| v.as_str()) {
                resolved.entry(name.clone()).or_insert_with(|| ver.to_string());
            }
        }
    }

    resolved
}

/// Parse Python `requirements.txt`.
pub fn parse_requirements_txt(content: &str, path: &str) -> Vec<Dependency> {
    let mut deps = Vec::new();

    for raw_line in content.lines() {
        let mut line = raw_line.trim();

        // Strip comments
        if let Some(idx) = line.find('#') {
            line = line[..idx].trim();
        }

        if line.is_empty() || line.starts_with('-') {
            // Ignore flags (-r, -i, -f, etc.)
            continue;
        }

        // Split environment markers (e.g. `importlib-metadata>=0.12;python_version<"3.8"`)
        let pkg_part = line.split(';').next().unwrap_or(line).trim();

        // Split extras (e.g. `requests[security]>=2.20.0`)
        let (name_part, spec_part) = if let Some(op_idx) = pkg_part.find(|c: char| {
            c == '=' || c == '>' || c == '<' || c == '~' || c == '!' || c == '@'
        }) {
            (&pkg_part[..op_idx], &pkg_part[op_idx..])
        } else {
            (pkg_part, "*")
        };

        let mut name = name_part.trim();
        if let Some(bracket_idx) = name.find('[') {
            name = name[..bracket_idx].trim();
        }

        if name.is_empty() {
            continue;
        }

        let requirement = spec_part.trim().to_string();
        let mut dep = Dependency::new(name, requirement.clone(), DependencyType::Normal, Ecosystem::PyPI, path);
        if let Some(base) = extract_base_version(&requirement) {
            dep.resolved_version = Some(base);
        }
        deps.push(dep);
    }

    deps
}

/// Parse Python `pyproject.toml` (PEP 621 and Poetry formats).
pub fn parse_pyproject_toml(content: &str, path: &str) -> Vec<Dependency> {
    let mut deps = Vec::new();
    let mut in_project_deps = false;
    let mut in_poetry_deps = false;
    let mut in_poetry_dev_deps = false;

    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }

        if trimmed.starts_with('[') && trimmed.ends_with(']') {
            let section = trimmed.trim_matches(|c| c == '[' || c == ']').trim();
            in_project_deps = section == "project.dependencies" || section == "project";
            in_poetry_deps = section == "tool.poetry.dependencies";
            in_poetry_dev_deps = section == "tool.poetry.dev-dependencies" || section == "tool.poetry.group.dev.dependencies";
            continue;
        }

        if in_project_deps {
            // Check for list element `"requests>=2.28.0",`
            if trimmed.starts_with('"') || trimmed.starts_with('\'') {
                let clean_item = trimmed.trim_matches(',').trim_matches('"').trim_matches('\'').trim();
                let parsed = parse_requirements_txt(clean_item, path);
                for d in parsed {
                    deps.push(d);
                }
            }
        } else if in_poetry_deps || in_poetry_dev_deps {
            let dep_type = if in_poetry_dev_deps {
                DependencyType::Dev
            } else {
                DependencyType::Normal
            };

            if let Some((k, v)) = trimmed.split_once('=') {
                let name = k.trim().trim_matches('"').trim_matches('\'').to_string();
                if name.is_empty() || name == "python" {
                    continue;
                }

                let val = v.split('#').next().unwrap_or(v).trim().trim_matches('"').trim_matches('\'').to_string();
                let mut dep = Dependency::new(name, val.clone(), dep_type, Ecosystem::PyPI, path);
                if let Some(base) = extract_base_version(&val) {
                    dep.resolved_version = Some(base);
                }
                deps.push(dep);
            }
        }
    }

    deps
}

/// Parse Python `Pipfile`.
pub fn parse_pipfile(content: &str, path: &str) -> Vec<Dependency> {
    let mut deps = Vec::new();
    let mut current_type = DependencyType::Normal;
    let mut in_packages = false;

    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }

        if trimmed.starts_with('[') && trimmed.ends_with(']') {
            let section = trimmed.trim_matches(|c| c == '[' || c == ']').trim();
            if section == "packages" {
                in_packages = true;
                current_type = DependencyType::Normal;
            } else if section == "dev-packages" {
                in_packages = true;
                current_type = DependencyType::Dev;
            } else {
                in_packages = false;
            }
            continue;
        }

        if in_packages {
            if let Some((k, v)) = trimmed.split_once('=') {
                let name = k.trim().trim_matches('"').trim_matches('\'').to_string();
                let val = v.split('#').next().unwrap_or(v).trim().trim_matches('"').trim_matches('\'').to_string();
                let mut dep = Dependency::new(name, val.clone(), current_type, Ecosystem::PyPI, path);
                if let Some(base) = extract_base_version(&val) {
                    dep.resolved_version = Some(base);
                }
                deps.push(dep);
            }
        }
    }

    deps
}

/// Parse Go `go.mod`.
pub fn parse_go_mod(content: &str, path: &str) -> Vec<Dependency> {
    let mut deps = Vec::new();
    let mut in_require_block = false;

    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with("//") {
            continue;
        }

        if trimmed.starts_with("require (") {
            in_require_block = true;
            continue;
        }

        if in_require_block {
            if trimmed == ")" {
                in_require_block = false;
                continue;
            }

            let parts: Vec<&str> = trimmed.split_whitespace().collect();
            if parts.len() >= 2 {
                let name = parts[0].to_string();
                let ver = parts[1].to_string();
                let dep_type = if trimmed.contains("// indirect") {
                    DependencyType::Optional
                } else {
                    DependencyType::Normal
                };
                let mut dep = Dependency::new(name, ver.clone(), dep_type, Ecosystem::Go, path);
                if let Some(base) = extract_base_version(&ver) {
                    dep.resolved_version = Some(base);
                }
                deps.push(dep);
            }
        } else if trimmed.starts_with("require ") {
            let rest = trimmed.strip_prefix("require ").unwrap_or("").trim();
            let parts: Vec<&str> = rest.split_whitespace().collect();
            if parts.len() >= 2 {
                let name = parts[0].to_string();
                let ver = parts[1].to_string();
                let mut dep = Dependency::new(name, ver.clone(), DependencyType::Normal, Ecosystem::Go, path);
                if let Some(base) = extract_base_version(&ver) {
                    dep.resolved_version = Some(base);
                }
                deps.push(dep);
            }
        }
    }

    deps
}

/// Parse Ruby `Gemfile`.
pub fn parse_gemfile(content: &str, path: &str) -> Vec<Dependency> {
    let mut deps = Vec::new();

    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }

        if trimmed.starts_with("gem ") {
            let rest = trimmed.strip_prefix("gem ").unwrap_or("").trim();
            let parts: Vec<&str> = rest.split(',').collect();
            if !parts.is_empty() {
                let name = parts[0].trim().trim_matches('"').trim_matches('\'').to_string();
                let req = if parts.len() > 1 {
                    parts[1].trim().trim_matches('"').trim_matches('\'').to_string()
                } else {
                    "*".to_string()
                };

                let mut dep = Dependency::new(name, req.clone(), DependencyType::Normal, Ecosystem::Gem, path);
                if let Some(base) = extract_base_version(&req) {
                    dep.resolved_version = Some(base);
                }
                deps.push(dep);
            }
        }
    }

    deps
}

// ===========================================================================
// Static Curated Security Advisory Database
// ===========================================================================

/// Lookup known security vulnerabilities for a package.
pub fn check_security_advisories(ecosystem: Ecosystem, name: &str, version_str: &str) -> Vec<SecurityAdvisory> {
    let mut advisories = Vec::new();
    let current_ver = SemVer::parse(version_str);

    let check_vuln = |id: &str, title: &str, sev: &str, affected_below: &str, patched: &str| -> Option<SecurityAdvisory> {
        let max_affected = SemVer::parse(affected_below)?;
        if let Some(ver) = &current_ver {
            if ver.cmp_version(&max_affected) == std::cmp::Ordering::Less {
                return Some(SecurityAdvisory {
                    id: id.to_string(),
                    title: title.to_string(),
                    severity: sev.to_string(),
                    affected_range: format!("< {}", affected_below),
                    patched_version: patched.to_string(),
                });
            }
        }
        None
    };

    match ecosystem {
        Ecosystem::Cargo => {
            match name {
                "crossbeam-channel" => {
                    if let Some(a) = check_vuln("RUSTSEC-2020-0052", "Memory corruption in crossbeam-channel", "high", "0.5.2", "0.5.2") {
                        advisories.push(a);
                    }
                }
                "smallvec" => {
                    if let Some(a) = check_vuln("RUSTSEC-2021-0003", "Use-after-free in SmallVec::into_inner", "critical", "1.6.1", "1.6.1") {
                        advisories.push(a);
                    }
                }
                "tokio" => {
                    if let Some(a) = check_vuln("RUSTSEC-2021-0124", "Data race in tokio join/select handles", "medium", "0.2.22", "0.2.22") {
                        advisories.push(a);
                    }
                }
                "time" => {
                    if let Some(a) = check_vuln("RUSTSEC-2020-0071", "Potential segfault in time formatting", "medium", "0.2.23", "0.2.23") {
                        advisories.push(a);
                    }
                }
                "openssl" => {
                    if let Some(a) = check_vuln("RUSTSEC-2023-0022", "OpenSSL certificate validation bypass", "critical", "0.10.48", "0.10.48") {
                        advisories.push(a);
                    }
                }
                "hyper" => {
                    if let Some(a) = check_vuln("RUSTSEC-2023-0034", "HTTP/2 Rapid Reset attack vulnerability", "high", "0.14.27", "0.14.27") {
                        advisories.push(a);
                    }
                }
                "idna" => {
                    if let Some(a) = check_vuln("RUSTSEC-2024-0336", "Punycode domain spoofing vulnerability", "medium", "0.4.0", "0.4.0") {
                        advisories.push(a);
                    }
                }
                _ => {}
            }
        }
        Ecosystem::Npm => {
            match name {
                "lodash" => {
                    if let Some(a) = check_vuln("GHSA-p6mc-m468-83gw", "Prototype Pollution in lodash", "high", "4.17.21", "4.17.21") {
                        advisories.push(a);
                    }
                }
                "axios" => {
                    if let Some(a) = check_vuln("GHSA-wf5p-g6vw-rhxx", "Server-Side Request Forgery in axios", "high", "1.6.0", "1.6.0") {
                        advisories.push(a);
                    }
                }
                "minimist" => {
                    if let Some(a) = check_vuln("GHSA-xvch-5gv4-984h", "Prototype Pollution in minimist", "critical", "1.2.6", "1.2.6") {
                        advisories.push(a);
                    }
                }
                "semver" => {
                    if let Some(a) = check_vuln("GHSA-c2qf-rxjj-qqgw", "Regular Expression Denial of Service in semver", "medium", "7.5.2", "7.5.2") {
                        advisories.push(a);
                    }
                }
                "express" => {
                    if let Some(a) = check_vuln("GHSA-qw6h-v8gh-w3fs", "Express open redirect vulnerability", "medium", "4.19.2", "4.19.2") {
                        advisories.push(a);
                    }
                }
                "ws" => {
                    if let Some(a) = check_vuln("GHSA-3h5v-q93c-6h6q", "Denial of Service in ws server", "high", "8.17.1", "8.17.1") {
                        advisories.push(a);
                    }
                }
                "tar" => {
                    if let Some(a) = check_vuln("GHSA-9r2w-394v-53qc", "Arbitrary File Creation/Overwrite in node-tar", "high", "6.2.1", "6.2.1") {
                        advisories.push(a);
                    }
                }
                _ => {}
            }
        }
        Ecosystem::PyPI => {
            match name {
                "requests" => {
                    if let Some(a) = check_vuln("CVE-2023-32681", "Unintended leak of Proxy-Authorization header in requests", "medium", "2.31.0", "2.31.0") {
                        advisories.push(a);
                    }
                }
                "urllib3" => {
                    if let Some(a) = check_vuln("CVE-2023-45803", "Cookie leak in urllib3 redirect handling", "high", "2.0.7", "2.0.7") {
                        advisories.push(a);
                    }
                }
                "flask" => {
                    if let Some(a) = check_vuln("CVE-2023-30861", "High risk session cookie disclosure in Flask", "high", "2.2.5", "2.2.5") {
                        advisories.push(a);
                    }
                }
                "django" => {
                    if let Some(a) = check_vuln("CVE-2024-45230", "Denial of Service in django.utils.html.urlize", "high", "4.2.16", "4.2.16") {
                        advisories.push(a);
                    }
                }
                "cryptography" => {
                    if let Some(a) = check_vuln("CVE-2023-49083", "NULL pointer dereference in PKCS7 parsing", "medium", "42.0.4", "42.0.4") {
                        advisories.push(a);
                    }
                }
                "aiohttp" => {
                    if let Some(a) = check_vuln("CVE-2024-27306", "HTTP request smuggling in aiohttp", "high", "3.9.4", "3.9.4") {
                        advisories.push(a);
                    }
                }
                "jinja2" => {
                    if let Some(a) = check_vuln("CVE-2024-34064", "HTML attribute injection in Jinja2", "medium", "3.1.4", "3.1.4") {
                        advisories.push(a);
                    }
                }
                _ => {}
            }
        }
        Ecosystem::Go => {
            if name.contains("golang.org/x/net") {
                if let Some(a) = check_vuln("GO-2023-2102", "HTTP/2 Rapid Reset in x/net", "high", "0.17.0", "v0.17.0") {
                    advisories.push(a);
                }
            } else if name.contains("gin-gonic/gin") {
                if let Some(a) = check_vuln("GO-2023-1737", "Context bypass in gin-gonic/gin", "medium", "1.9.1", "v1.9.1") {
                    advisories.push(a);
                }
            }
        }
        _ => {}
    }

    advisories
}

// ===========================================================================
// Registry Client & In-Memory Cache
// ===========================================================================

/// Fetched registry package metadata.
#[derive(Debug, Clone)]
pub struct RegistryMetadata {
    pub latest_version: String,
    pub description: Option<String>,
    pub homepage: Option<String>,
}

/// In-memory cache for fetched package versions.
#[derive(Clone, Default)]
pub struct RegistryCache {
    cache: Arc<Mutex<HashMap<(Ecosystem, String), Option<RegistryMetadata>>>>,
}

impl RegistryCache {
    pub fn new() -> Self {
        Self {
            cache: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    pub fn get(&self, eco: Ecosystem, name: &str) -> Option<Option<RegistryMetadata>> {
        let guard = self.cache.lock().ok()?;
        guard.get(&(eco, name.to_string())).cloned()
    }

    pub fn insert(&self, eco: Ecosystem, name: &str, meta: Option<RegistryMetadata>) {
        if let Ok(mut guard) = self.cache.lock() {
            guard.insert((eco, name.to_string()), meta);
        }
    }
}

/// Fetch latest package version from official registries.
pub async fn fetch_latest_version(
    client: &reqwest::Client,
    cache: &RegistryCache,
    ecosystem: Ecosystem,
    name: &str,
) -> Option<RegistryMetadata> {
    if let Some(cached) = cache.get(ecosystem, name) {
        return cached;
    }

    let user_agent = "Fusion-Dependency-Auditor/0.3 (github.com/theaungmyatmoe/fusion)";

    let res = match ecosystem {
        Ecosystem::Cargo => {
            let url = format!("https://crates.io/api/v1/crates/{}", name);
            let resp = client
                .get(&url)
                .header("User-Agent", user_agent)
                .timeout(Duration::from_secs(5))
                .send()
                .await
                .ok()?;

            if !resp.status().is_success() {
                cache.insert(ecosystem, name, None);
                return None;
            }

            let val: Value = resp.json().await.ok()?;
            let crate_obj = val.get("crate")?;
            let latest_version = crate_obj
                .get("max_stable_version")
                .and_then(|v| v.as_str())
                .or_else(|| crate_obj.get("max_version").and_then(|v| v.as_str()))?
                .to_string();

            let description = crate_obj.get("description").and_then(|v| v.as_str()).map(|s| s.to_string());
            let homepage = crate_obj.get("homepage").and_then(|v| v.as_str()).map(|s| s.to_string());

            Some(RegistryMetadata {
                latest_version,
                description,
                homepage,
            })
        }
        Ecosystem::Npm => {
            let url = format!("https://registry.npmjs.org/{}", name);
            let resp = client
                .get(&url)
                .header("User-Agent", user_agent)
                .timeout(Duration::from_secs(5))
                .send()
                .await
                .ok()?;

            if !resp.status().is_success() {
                cache.insert(ecosystem, name, None);
                return None;
            }

            let val: Value = resp.json().await.ok()?;
            let latest_version = val
                .get("dist-tags")
                .and_then(|dt| dt.get("latest"))
                .and_then(|v| v.as_str())?
                .to_string();

            let description = val.get("description").and_then(|v| v.as_str()).map(|s| s.to_string());
            let homepage = val.get("homepage").and_then(|v| v.as_str()).map(|s| s.to_string());

            Some(RegistryMetadata {
                latest_version,
                description,
                homepage,
            })
        }
        Ecosystem::PyPI => {
            let url = format!("https://pypi.org/pypi/{}/json", name);
            let resp = client
                .get(&url)
                .header("User-Agent", user_agent)
                .timeout(Duration::from_secs(5))
                .send()
                .await
                .ok()?;

            if !resp.status().is_success() {
                cache.insert(ecosystem, name, None);
                return None;
            }

            let val: Value = resp.json().await.ok()?;
            let info = val.get("info")?;
            let latest_version = info.get("version").and_then(|v| v.as_str())?.to_string();
            let description = info.get("summary").and_then(|v| v.as_str()).map(|s| s.to_string());
            let homepage = info.get("home_page").and_then(|v| v.as_str()).map(|s| s.to_string());

            Some(RegistryMetadata {
                latest_version,
                description,
                homepage,
            })
        }
        Ecosystem::Go => {
            let url = format!("https://proxy.golang.org/{}/@latest", name);
            let resp = client
                .get(&url)
                .header("User-Agent", user_agent)
                .timeout(Duration::from_secs(5))
                .send()
                .await
                .ok()?;

            if !resp.status().is_success() {
                cache.insert(ecosystem, name, None);
                return None;
            }

            let val: Value = resp.json().await.ok()?;
            let latest_version = val.get("Version").and_then(|v| v.as_str())?.to_string();

            Some(RegistryMetadata {
                latest_version,
                description: None,
                homepage: None,
            })
        }
        Ecosystem::Gem => {
            let url = format!("https://rubygems.org/api/v1/gems/{}.json", name);
            let resp = client
                .get(&url)
                .header("User-Agent", user_agent)
                .timeout(Duration::from_secs(5))
                .send()
                .await
                .ok()?;

            if !resp.status().is_success() {
                cache.insert(ecosystem, name, None);
                return None;
            }

            let val: Value = resp.json().await.ok()?;
            let latest_version = val.get("version").and_then(|v| v.as_str())?.to_string();
            let description = val.get("info").and_then(|v| v.as_str()).map(|s| s.to_string());
            let homepage = val.get("homepage_uri").and_then(|v| v.as_str()).map(|s| s.to_string());

            Some(RegistryMetadata {
                latest_version,
                description,
                homepage,
            })
        }
        Ecosystem::Generic => None,
    };

    cache.insert(ecosystem, name, res.clone());
    res
}

// ===========================================================================
// Workspace Discovery & Dependency Auditing Engine
// ===========================================================================

/// Find all supported dependency manifests in a directory tree.
pub fn find_manifests(root: &Path, max_depth: usize, filter_eco: Option<Ecosystem>) -> Vec<(PathBuf, Ecosystem)> {
    let mut found = Vec::new();
    find_manifests_recursive(root, 0, max_depth, filter_eco, &mut found);
    found
}

fn find_manifests_recursive(
    dir: &Path,
    current_depth: usize,
    max_depth: usize,
    filter_eco: Option<Ecosystem>,
    results: &mut Vec<(PathBuf, Ecosystem)>,
) {
    if current_depth > max_depth {
        return;
    }

    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };

    let mut subdirs = Vec::new();

    for entry in entries.flatten() {
        let path = entry.path();
        let file_name = entry.file_name();
        let name_str = file_name.to_string_lossy();

        if path.is_file() {
            let eco = match name_str.as_ref() {
                "Cargo.toml" => Some(Ecosystem::Cargo),
                "package.json" => Some(Ecosystem::Npm),
                "requirements.txt" => Some(Ecosystem::PyPI),
                "pyproject.toml" => Some(Ecosystem::PyPI),
                "Pipfile" => Some(Ecosystem::PyPI),
                "go.mod" => Some(Ecosystem::Go),
                "Gemfile" => Some(Ecosystem::Gem),
                _ => None,
            };

            if let Some(e) = eco {
                if filter_eco.is_none() || filter_eco == Some(e) {
                    results.push((path, e));
                }
            }
        } else if path.is_dir() {
            // Skip common build / ignored directories
            if !name_str.starts_with('.')
                && name_str != "target"
                && name_str != "node_modules"
                && name_str != "venv"
                && name_str != ".venv"
                && name_str != "dist"
                && name_str != "build"
                && name_str != "__pycache__"
                && name_str != "vendor"
            {
                subdirs.push(path);
            }
        }
    }

    for sub in subdirs {
        find_manifests_recursive(&sub, current_depth + 1, max_depth, filter_eco, results);
    }
}

/// Audit a single manifest file.
pub async fn audit_manifest(
    manifest_path: &Path,
    ecosystem: Ecosystem,
    client: Option<&reqwest::Client>,
    cache: &RegistryCache,
    check_online: bool,
    dep_filter: &str,
) -> anyhow::Result<ManifestScanResult> {
    let content = std::fs::read_to_string(manifest_path)
        .map_err(|e| anyhow::anyhow!("Failed to read {}: {}", manifest_path.display(), e))?;

    let path_str = manifest_path.display().to_string();

    let mut raw_deps = match ecosystem {
        Ecosystem::Cargo => parse_cargo_toml(&content, &path_str),
        Ecosystem::Npm => parse_package_json(&content, &path_str),
        Ecosystem::PyPI => {
            let filename = manifest_path
                .file_name()
                .map(|f| f.to_string_lossy().to_string())
                .unwrap_or_default();
            if filename == "pyproject.toml" {
                parse_pyproject_toml(&content, &path_str)
            } else if filename == "Pipfile" {
                parse_pipfile(&content, &path_str)
            } else {
                parse_requirements_txt(&content, &path_str)
            }
        }
        Ecosystem::Go => parse_go_mod(&content, &path_str),
        Ecosystem::Gem => parse_gemfile(&content, &path_str),
        Ecosystem::Generic => parse_requirements_txt(&content, &path_str),
    };

    // Filter dependency types if requested
    raw_deps.retain(|d| d.dep_type.matches_filter(dep_filter));

    // Try reading sibling lockfiles to resolve exact installed versions
    if let Some(parent) = manifest_path.parent() {
        if ecosystem == Ecosystem::Cargo {
            let lock_path = parent.join("Cargo.lock");
            if let Ok(lock_content) = std::fs::read_to_string(&lock_path) {
                let locked = parse_cargo_lock(&lock_content);
                for d in &mut raw_deps {
                    if let Some(v) = locked.get(&d.name) {
                        d.resolved_version = Some(v.clone());
                    }
                }
            }
        } else if ecosystem == Ecosystem::Npm {
            let lock_path = parent.join("package-lock.json");
            if let Ok(lock_content) = std::fs::read_to_string(&lock_path) {
                let locked = parse_package_lock_json(&lock_content);
                for d in &mut raw_deps {
                    if let Some(v) = locked.get(&d.name) {
                        d.resolved_version = Some(v.clone());
                    }
                }
            }
        }
    }

    // Check vulnerabilities against static database
    for d in &mut raw_deps {
        let ver_to_check = d.resolved_version.as_deref().unwrap_or(&d.requirement);
        d.advisories = check_security_advisories(ecosystem, &d.name, ver_to_check);
    }

    // Query online registries if requested
    if check_online && client.is_some() {
        let http_client = client.unwrap();

        // Process dependencies concurrently
        let mut futures = Vec::new();
        for d in &raw_deps {
            let name = d.name.clone();
            let eco = d.ecosystem;
            let cache_ref = cache.clone();
            let client_ref = http_client.clone();
            futures.push(async move {
                fetch_latest_version(&client_ref, &cache_ref, eco, &name).await
            });
        }

        let results = futures::future::join_all(futures).await;

        for (d, res) in raw_deps.iter_mut().zip(results.into_iter()) {
            if let Some(meta) = res {
                d.latest_version = Some(meta.latest_version.clone());
                if d.description.is_none() {
                    d.description = meta.description;
                }
                if d.homepage.is_none() {
                    d.homepage = meta.homepage;
                }
            }
        }
    }

    // Determine outdated statuses
    for d in &mut raw_deps {
        let is_cargo = d.ecosystem == Ecosystem::Cargo;

        if let Some(latest_str) = &d.latest_version {
            let current_str = d.resolved_version.as_deref().unwrap_or(&d.requirement);
            if let (Some(cur_sem), Some(lat_sem)) = (SemVer::parse(current_str), SemVer::parse(latest_str)) {
                if let Some(bump) = cur_sem.bump_to(&lat_sem, is_cargo) {
                    d.status = OutdatedStatus::Outdated(bump);
                } else {
                    d.status = OutdatedStatus::UpToDate;
                }
            } else if current_str == "*" || current_str.is_empty() {
                d.status = OutdatedStatus::ConstraintOnly;
            } else {
                d.status = OutdatedStatus::Unknown;
            }
        } else if d.resolved_version.is_none() && (d.requirement == "*" || d.requirement == "workspace") {
            d.status = OutdatedStatus::ConstraintOnly;
        } else {
            d.status = OutdatedStatus::Unknown;
        }
    }

    let total_count = raw_deps.len();
    let mut up_to_date_count = 0;
    let mut outdated_count = 0;
    let mut major_count = 0;
    let mut minor_count = 0;
    let mut patch_count = 0;
    let mut vulnerable_count = 0;

    for d in &raw_deps {
        if d.is_vulnerable() {
            vulnerable_count += 1;
        }
        match d.status {
            OutdatedStatus::UpToDate => up_to_date_count += 1,
            OutdatedStatus::Outdated(b) => {
                outdated_count += 1;
                match b {
                    BumpType::Major => major_count += 1,
                    BumpType::Minor => minor_count += 1,
                    BumpType::Patch => patch_count += 1,
                    _ => {}
                }
            }
            _ => {}
        }
    }

    // Compute Health Score: 100 base, deductions for outdated and vulnerable
    let mut score = 100i32;
    score -= (major_count as i32) * 10;
    score -= (minor_count as i32) * 4;
    score -= (patch_count as i32) * 1;
    score -= (vulnerable_count as i32) * 20;
    let health_score = score.clamp(0, 100) as u32;

    Ok(ManifestScanResult {
        path: path_str,
        ecosystem,
        dependencies: raw_deps,
        total_count,
        up_to_date_count,
        outdated_count,
        major_count,
        minor_count,
        patch_count,
        vulnerable_count,
        health_score,
    })
}

/// Audit entire workspace or target file.
pub async fn audit_workspace(
    target_path: &Path,
    ecosystem_filter: Option<Ecosystem>,
    outdated_only: bool,
    check_online: bool,
    dep_filter: &str,
    max_depth: usize,
) -> anyhow::Result<AuditReport> {
    let client = if check_online {
        Some(
            reqwest::Client::builder()
                .timeout(Duration::from_secs(10))
                .build()
                .unwrap_or_default(),
        )
    } else {
        None
    };

    let cache = RegistryCache::new();

    let manifests_to_audit = if target_path.is_file() {
        let filename = target_path
            .file_name()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_default();
        let eco = match filename.as_str() {
            "Cargo.toml" => Ecosystem::Cargo,
            "package.json" => Ecosystem::Npm,
            "requirements.txt" | "pyproject.toml" | "Pipfile" => Ecosystem::PyPI,
            "go.mod" => Ecosystem::Go,
            "Gemfile" => Ecosystem::Gem,
            _ => ecosystem_filter.unwrap_or(Ecosystem::Generic),
        };
        vec![(target_path.to_path_buf(), eco)]
    } else {
        find_manifests(target_path, max_depth, ecosystem_filter)
    };

    let mut scanned_manifests = Vec::new();

    for (path, eco) in manifests_to_audit {
        match audit_manifest(&path, eco, client.as_ref(), &cache, check_online, dep_filter).await {
            Ok(mut res) => {
                if outdated_only {
                    res.dependencies.retain(|d| d.is_outdated() || d.is_vulnerable());
                }
                scanned_manifests.push(res);
            }
            Err(e) => {
                tracing::warn!("Failed auditing manifest at {}: {}", path.display(), e);
            }
        }
    }

    let mut total_dependencies = 0;
    let mut total_outdated = 0;
    let mut total_major = 0;
    let mut total_minor = 0;
    let mut total_patch = 0;
    let mut total_vulnerable = 0;
    let mut score_sum = 0u64;

    for m in &scanned_manifests {
        total_dependencies += m.total_count;
        total_outdated += m.outdated_count;
        total_major += m.major_count;
        total_minor += m.minor_count;
        total_patch += m.patch_count;
        total_vulnerable += m.vulnerable_count;
        score_sum += m.health_score as u64;
    }

    let overall_health_score = if scanned_manifests.is_empty() {
        100
    } else {
        (score_sum / scanned_manifests.len() as u64) as u32
    };

    let health_rating = match overall_health_score {
        90..=100 => "A (Excellent)".to_string(),
        75..=89 => "B (Good)".to_string(),
        60..=74 => "C (Fair)".to_string(),
        40..=59 => "D (Poor)".to_string(),
        _ => "F (Critical)".to_string(),
    };

    let mut recommendations = Vec::new();
    if total_vulnerable > 0 {
        recommendations.push(format!(
            "CRITICAL: {} security vulnerabilities detected! Review advisory patches immediately.",
            total_vulnerable
        ));
    }
    if total_major > 0 {
        recommendations.push(format!(
            "{} major updates available. Review breaking change release notes before upgrading.",
            total_major
        ));
    }
    if total_minor > 0 || total_patch > 0 {
        recommendations.push(format!(
            "{} minor/patch updates available. Run package update commands (e.g. `cargo update`, `npm update`).",
            total_minor + total_patch
        ));
    }
    if total_outdated == 0 && total_vulnerable == 0 && total_dependencies > 0 {
        recommendations.push("All dependencies are up to date! Great job maintaining freshness.".to_string());
    }

    Ok(AuditReport {
        workspace_root: target_path.display().to_string(),
        manifests: scanned_manifests,
        total_dependencies,
        total_outdated,
        total_major,
        total_minor,
        total_patch,
        total_vulnerable,
        overall_health_score,
        health_rating,
        recommendations,
    })
}

// ===========================================================================
// Formatters (Table, Markdown, JSON, Summary)
// ===========================================================================

/// Format audit report as pretty text table.
pub fn format_table(report: &AuditReport) -> String {
    let mut out = String::new();

    out.push_str("=== Dependency Audit Report ===\n\n");

    if report.manifests.is_empty() {
        out.push_str("No supported dependency manifests found.\n");
        return out;
    }

    for m in &report.manifests {
        out.push_str(&format!(
            "Manifest: {} ({})\n",
            m.path,
            m.ecosystem.display_name()
        ));
        out.push_str(&format!(
            "Health Score: {}/100 | Total: {} | Outdated: {} (Major: {}, Minor: {}, Patch: {})\n",
            m.health_score, m.total_count, m.outdated_count, m.major_count, m.minor_count, m.patch_count
        ));

        if m.dependencies.is_empty() {
            out.push_str("  (No dependencies declared)\n\n");
            continue;
        }

        out.push_str(&format!(
            "{:<25} {:<8} {:<15} {:<15} {:<15} {:<10}\n",
            "PACKAGE", "TYPE", "DECLARED", "CURRENT", "LATEST", "STATUS"
        ));
        out.push_str(&format!("{:-<90}\n", ""));

        for d in &m.dependencies {
            let current_str = d.resolved_version.as_deref().unwrap_or("-");
            let latest_str = d.latest_version.as_deref().unwrap_or("-");
            let status_badge = d.status.badge();

            out.push_str(&format!(
                "{:<25} {:<8} {:<15} {:<15} {:<15} {:<10}\n",
                d.name,
                d.dep_type.as_str(),
                d.requirement,
                current_str,
                latest_str,
                status_badge
            ));

            for adv in &d.advisories {
                out.push_str(&format!(
                    "  -> [VULN: {}] {} (Severity: {}, Patched in: {})\n",
                    adv.id, adv.title, adv.severity, adv.patched_version
                ));
            }
        }
        out.push('\n');
    }

    out.push_str("=== Summary ===\n");
    out.push_str(&format!("Scanned Manifests:    {}\n", report.manifests.len()));
    out.push_str(&format!("Total Dependencies:   {}\n", report.total_dependencies));
    out.push_str(&format!(
        "Outdated Packages:    {} (Major: {}, Minor: {}, Patch: {})\n",
        report.total_outdated, report.total_major, report.total_minor, report.total_patch
    ));
    out.push_str(&format!("Vulnerabilities:      {}\n", report.total_vulnerable));
    out.push_str(&format!(
        "Overall Health Score: {}/100 [{}]\n",
        report.overall_health_score, report.health_rating
    ));

    if !report.recommendations.is_empty() {
        out.push_str("\nRecommendations:\n");
        for rec in &report.recommendations {
            out.push_str(&format!("- {}\n", rec));
        }
    }

    out
}

/// Format audit report as Markdown.
pub fn format_markdown(report: &AuditReport) -> String {
    let mut out = String::new();

    out.push_str("# Dependency Audit Report\n\n");
    out.push_str(&format!(
        "> **Health Score:** `{}/100` ({}) | **Dependencies:** `{}` | **Outdated:** `{}` | **Vulnerabilities:** `{}`\n\n",
        report.overall_health_score, report.health_rating, report.total_dependencies, report.total_outdated, report.total_vulnerable
    ));

    if report.manifests.is_empty() {
        out.push_str("_No supported dependency manifests discovered._\n");
        return out;
    }

    for m in &report.manifests {
        out.push_str(&format!("## `{}` ({})\n\n", m.path, m.ecosystem.display_name()));
        out.push_str(&format!(
            "Health: **{}/100** | Total: **{}** | Outdated: **{}** (Major: {}, Minor: {}, Patch: {})\n\n",
            m.health_score, m.total_count, m.outdated_count, m.major_count, m.minor_count, m.patch_count
        ));

        if m.dependencies.is_empty() {
            out.push_str("_No dependencies declared in this manifest._\n\n");
            continue;
        }

        out.push_str("| Package | Type | Declared | Current | Latest | Status |\n");
        out.push_str("| :--- | :--- | :--- | :--- | :--- | :--- |\n");

        for d in &m.dependencies {
            let cur = d.resolved_version.as_deref().unwrap_or("-");
            let lat = d.latest_version.as_deref().unwrap_or("-");
            let badge = d.status.badge();

            out.push_str(&format!(
                "| **{}** | `{}` | `{}` | `{}` | `{}` | {} |\n",
                d.name,
                d.dep_type.as_str(),
                d.requirement,
                cur,
                lat,
                badge
            ));

            for adv in &d.advisories {
                out.push_str(&format!(
                    "| | | ⚠️ **{}**: {} (Patched: `{}`) | | | |\n",
                    adv.id, adv.title, adv.patched_version
                ));
            }
        }
        out.push('\n');
    }

    if !report.recommendations.is_empty() {
        out.push_str("### Recommendations\n\n");
        for rec in &report.recommendations {
            out.push_str(&format!("- {}\n", rec));
        }
    }

    out
}

/// Format audit report as concise summary.
pub fn format_summary(report: &AuditReport) -> String {
    format!(
        "Dependency Audit: {} manifest(s), {} dependencies. Outdated: {} (Major: {}, Minor: {}, Patch: {}). Vulnerabilities: {}. Health Score: {}/100 [{}]",
        report.manifests.len(),
        report.total_dependencies,
        report.total_outdated,
        report.total_major,
        report.total_minor,
        report.total_patch,
        report.total_vulnerable,
        report.overall_health_score,
        report.health_rating
    )
}

// ===========================================================================
// Tool Implementation
// ===========================================================================

/// Tool for auditing project dependencies across Cargo, npm, PyPI, and more.
#[derive(Default, Debug, Clone)]
pub struct DepsTool;

impl DepsTool {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl Tool for DepsTool {
    fn name(&self) -> &str {
        "deps"
    }

    fn description(&self) -> &str {
        "Audit codebase dependencies across Cargo.toml, package.json, requirements.txt, pyproject.toml, Pipfile, go.mod, and Gemfile. Identifies outdated packages, major/minor/patch upgrades, security advisories, and dependency health."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "Path to specific manifest file (e.g. 'Cargo.toml', 'package.json') or workspace root directory. Defaults to workspace root ('.')."
                },
                "ecosystem": {
                    "type": "string",
                    "description": "Filter by package ecosystem: 'cargo' / 'rust', 'npm' / 'node', 'pypi' / 'python', 'go', 'gem', or 'all' (optional)."
                },
                "outdated_only": {
                    "type": "boolean",
                    "description": "Whether to only list outdated or vulnerable dependencies (default: false)."
                },
                "check_online": {
                    "type": "boolean",
                    "description": "Whether to query upstream registries (crates.io, npm, PyPI) for the latest versions (default: true)."
                },
                "dep_type": {
                    "type": "string",
                    "description": "Filter by dependency type: 'all', 'prod' / 'normal', 'dev', 'build', 'peer', 'optional' (default: 'all')."
                },
                "format": {
                    "type": "string",
                    "enum": ["table", "text", "markdown", "json", "summary"],
                    "description": "Output format: 'table' (default formatted text), 'markdown', 'json' (structured data), or 'summary'."
                },
                "max_depth": {
                    "type": "integer",
                    "description": "Maximum directory traversal depth when searching for manifests (default: 4)."
                }
            }
        })
    }

    async fn execute(&self, args: Value, ctx: &ToolContext) -> anyhow::Result<String> {
        let path_arg = args
            .get("path")
            .and_then(|v| v.as_str())
            .or_else(|| args.get("file").and_then(|v| v.as_str()))
            .unwrap_or(".");

        let ecosystem_filter = args
            .get("ecosystem")
            .and_then(|v| v.as_str())
            .or_else(|| args.get("manifest_type").and_then(|v| v.as_str()))
            .or_else(|| args.get("eco").and_then(|v| v.as_str()))
            .and_then(Ecosystem::from_str_loose);

        let outdated_only = args
            .get("outdated_only")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        let check_online = args
            .get("check_online")
            .and_then(|v| v.as_bool())
            .unwrap_or(true);

        let dep_filter = args
            .get("dep_type")
            .and_then(|v| v.as_str())
            .unwrap_or("all");

        let format = args
            .get("format")
            .and_then(|v| v.as_str())
            .unwrap_or("table");

        let max_depth = args
            .get("max_depth")
            .and_then(|v| v.as_u64())
            .unwrap_or(4) as usize;

        let target_path = resolve_path(path_arg, &ctx.cwd);

        if !target_path.exists() {
            return Ok(format!("Error: Path does not exist: {}", target_path.display()));
        }

        let report = audit_workspace(
            &target_path,
            ecosystem_filter,
            outdated_only,
            check_online,
            dep_filter,
            max_depth,
        )
        .await?;

        match format.to_ascii_lowercase().as_str() {
            "json" => Ok(serde_json::to_string_pretty(&report)?),
            "markdown" | "md" => Ok(format_markdown(&report)),
            "summary" => Ok(format_summary(&report)),
            "table" | "text" | _ => Ok(format_table(&report)),
        }
    }
}

// ===========================================================================
// Unit Tests
// ===========================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_semver_parsing() {
        let v1 = SemVer::parse("1.2.3").unwrap();
        assert_eq!(v1.major, 1);
        assert_eq!(v1.minor, 2);
        assert_eq!(v1.patch, 3);
        assert_eq!(v1.pre, None);

        let v2 = SemVer::parse("v2.0.0-alpha.1+build.123").unwrap();
        assert_eq!(v2.major, 2);
        assert_eq!(v2.minor, 0);
        assert_eq!(v2.patch, 0);
        assert_eq!(v2.pre.as_deref(), Some("alpha.1"));
        assert_eq!(v2.build.as_deref(), Some("build.123"));

        let v3 = SemVer::parse("^0.4").unwrap();
        assert_eq!(v3.major, 0);
        assert_eq!(v3.minor, 4);
        assert_eq!(v3.patch, 0);

        let v4 = SemVer::parse(">= 3.10.2").unwrap();
        assert_eq!(v4.major, 3);
        assert_eq!(v4.minor, 10);
        assert_eq!(v4.patch, 2);
    }

    #[test]
    fn test_semver_comparison() {
        let v1 = SemVer::parse("1.2.3").unwrap();
        let v2 = SemVer::parse("1.2.4").unwrap();
        let v3 = SemVer::parse("1.3.0").unwrap();
        let v4 = SemVer::parse("2.0.0").unwrap();
        let v_pre = SemVer::parse("2.0.0-beta").unwrap();

        assert_eq!(v1.cmp_version(&v2), std::cmp::Ordering::Less);
        assert_eq!(v2.cmp_version(&v1), std::cmp::Ordering::Greater);
        assert_eq!(v2.cmp_version(&v3), std::cmp::Ordering::Less);
        assert_eq!(v3.cmp_version(&v4), std::cmp::Ordering::Less);
        assert_eq!(v_pre.cmp_version(&v4), std::cmp::Ordering::Less);
    }

    #[test]
    fn test_semver_bump_types() {
        let v1 = SemVer::parse("1.2.3").unwrap();
        let v_patch = SemVer::parse("1.2.4").unwrap();
        let v_minor = SemVer::parse("1.3.0").unwrap();
        let v_major = SemVer::parse("2.0.0").unwrap();

        assert_eq!(v1.bump_to(&v_patch, false), Some(BumpType::Patch));
        assert_eq!(v1.bump_to(&v_minor, false), Some(BumpType::Minor));
        assert_eq!(v1.bump_to(&v_major, false), Some(BumpType::Major));
        assert_eq!(v1.bump_to(&v1, false), None);

        // Cargo 0.x.y semantics
        let c02 = SemVer::parse("0.2.1").unwrap();
        let c03 = SemVer::parse("0.3.0").unwrap();
        let c022 = SemVer::parse("0.2.2").unwrap();
        assert_eq!(c02.bump_to(&c03, true), Some(BumpType::Major));
        assert_eq!(c02.bump_to(&c022, true), Some(BumpType::Patch));
    }

    #[test]
    fn test_parse_cargo_toml() {
        let content = r#"
[package]
name = "my-project"
version = "0.1.0"

[dependencies]
serde = "1.0.197"
tokio = { version = "1.36.0", features = ["full"] }
tracing = { workspace = true }
local-lib = { path = "../local-lib" }

[dev-dependencies]
tempfile = "3.10"
criterion = { version = "0.5.1", features = ["async_tokio"] }

[build-dependencies]
cc = "1.0"
"#;
        let deps = parse_cargo_toml(content, "Cargo.toml");
        assert_eq!(deps.len(), 7);

        let serde_dep = deps.iter().find(|d| d.name == "serde").unwrap();
        assert_eq!(serde_dep.requirement, "1.0.197");
        assert_eq!(serde_dep.resolved_version.as_deref(), Some("1.0.197"));
        assert_eq!(serde_dep.dep_type, DependencyType::Normal);

        let tokio_dep = deps.iter().find(|d| d.name == "tokio").unwrap();
        assert_eq!(tokio_dep.requirement, "1.36.0");

        let tracing_dep = deps.iter().find(|d| d.name == "tracing").unwrap();
        assert_eq!(tracing_dep.requirement, "workspace");

        let tempfile_dep = deps.iter().find(|d| d.name == "tempfile").unwrap();
        assert_eq!(tempfile_dep.dep_type, DependencyType::Dev);

        let cc_dep = deps.iter().find(|d| d.name == "cc").unwrap();
        assert_eq!(cc_dep.dep_type, DependencyType::Build);
    }

    #[test]
    fn test_parse_cargo_lock() {
        let content = r#"
version = 3

[[package]]
name = "serde"
version = "1.0.198"
source = "registry+https://github.com/rust-lang/crates.io-index"

[[package]]
name = "tokio"
version = "1.36.1"
"#;
        let locked = parse_cargo_lock(content);
        assert_eq!(locked.get("serde").map(|s| s.as_str()), Some("1.0.198"));
        assert_eq!(locked.get("tokio").map(|s| s.as_str()), Some("1.36.1"));
    }

    #[test]
    fn test_parse_package_json() {
        let content = r#"{
  "name": "my-app",
  "version": "1.0.0",
  "dependencies": {
    "react": "^18.2.0",
    "lodash": "~4.17.21"
  },
  "devDependencies": {
    "typescript": "^5.3.3",
    "@types/react": "^18.2.0"
  },
  "peerDependencies": {
    "react-dom": ">=18.0.0"
  }
}"#;
        let deps = parse_package_json(content, "package.json");
        assert_eq!(deps.len(), 5);

        let react_dep = deps.iter().find(|d| d.name == "react").unwrap();
        assert_eq!(react_dep.requirement, "^18.2.0");
        assert_eq!(react_dep.resolved_version.as_deref(), Some("18.2.0"));
        assert_eq!(react_dep.dep_type, DependencyType::Normal);

        let ts_dep = deps.iter().find(|d| d.name == "typescript").unwrap();
        assert_eq!(ts_dep.dep_type, DependencyType::Dev);

        let peer_dep = deps.iter().find(|d| d.name == "react-dom").unwrap();
        assert_eq!(peer_dep.dep_type, DependencyType::Peer);
    }

    #[test]
    fn test_parse_requirements_txt() {
        let content = r#"
# Core packages
requests==2.31.0
flask>=2.0.0,<3.0.0
pytest~=7.4.0
importlib-metadata>=0.12;python_version<"3.8"
celery[redis]>=5.2.0
-r other-requirements.txt
-i https://pypi.org/simple
"#;
        let deps = parse_requirements_txt(content, "requirements.txt");
        assert_eq!(deps.len(), 5);

        let req_dep = deps.iter().find(|d| d.name == "requests").unwrap();
        assert_eq!(req_dep.requirement, "==2.31.0");
        assert_eq!(req_dep.resolved_version.as_deref(), Some("2.31.0"));

        let flask_dep = deps.iter().find(|d| d.name == "flask").unwrap();
        assert_eq!(flask_dep.requirement, ">=2.0.0,<3.0.0");
        assert_eq!(flask_dep.resolved_version.as_deref(), Some("2.0.0"));

        let celery_dep = deps.iter().find(|d| d.name == "celery").unwrap();
        assert_eq!(celery_dep.requirement, ">=5.2.0");
    }

    #[test]
    fn test_parse_pyproject_toml() {
        let content = r#"
[project]
name = "my-python-app"
dependencies = [
    "httpx>=0.24.0",
    "pydantic>=2.0.0",
]

[tool.poetry.dependencies]
python = "^3.11"
fastapi = "^0.100.0"

[tool.poetry.dev-dependencies]
ruff = "^0.1.0"
"#;
        let deps = parse_pyproject_toml(content, "pyproject.toml");
        assert_eq!(deps.len(), 4);

        let httpx_dep = deps.iter().find(|d| d.name == "httpx").unwrap();
        assert_eq!(httpx_dep.requirement, ">=0.24.0");

        let fastapi_dep = deps.iter().find(|d| d.name == "fastapi").unwrap();
        assert_eq!(fastapi_dep.dep_type, DependencyType::Normal);

        let ruff_dep = deps.iter().find(|d| d.name == "ruff").unwrap();
        assert_eq!(ruff_dep.dep_type, DependencyType::Dev);
    }

    #[test]
    fn test_parse_go_mod() {
        let content = r#"
module my/awesome/module

go 1.21

require (
    github.com/gin-gonic/gin v1.9.1
    github.com/stretchr/testify v1.8.4 // indirect
)

require rsc.io/quote v1.5.2
"#;
        let deps = parse_go_mod(content, "go.mod");
        assert_eq!(deps.len(), 3);

        let gin_dep = deps.iter().find(|d| d.name == "github.com/gin-gonic/gin").unwrap();
        assert_eq!(gin_dep.requirement, "v1.9.1");
        assert_eq!(gin_dep.dep_type, DependencyType::Normal);

        let testify_dep = deps.iter().find(|d| d.name == "github.com/stretchr/testify").unwrap();
        assert_eq!(testify_dep.dep_type, DependencyType::Optional);
    }

    #[test]
    fn test_parse_gemfile() {
        let content = r#"
source 'https://rubygems.org'

gem 'rails', '~> 7.1.0'
gem 'pg', '>= 1.5'
gem 'puma'
"#;
        let deps = parse_gemfile(content, "Gemfile");
        assert_eq!(deps.len(), 3);

        let rails_dep = deps.iter().find(|d| d.name == "rails").unwrap();
        assert_eq!(rails_dep.requirement, "~> 7.1.0");
        assert_eq!(rails_dep.resolved_version.as_deref(), Some("7.1.0"));
    }

    #[test]
    fn test_security_advisories() {
        let advs = check_security_advisories(Ecosystem::Npm, "lodash", "4.17.15");
        assert!(!advs.is_empty());
        assert_eq!(advs[0].severity, "high");

        let clean_advs = check_security_advisories(Ecosystem::Npm, "lodash", "4.17.21");
        assert!(clean_advs.is_empty());

        let rust_advs = check_security_advisories(Ecosystem::Cargo, "smallvec", "1.6.0");
        assert!(!rust_advs.is_empty());
        assert_eq!(rust_advs[0].severity, "critical");
    }

    #[tokio::test]
    async fn test_deps_tool_execution() {
        let tool = DepsTool::new();
        let ctx = ToolContext::default();

        let res = tool
            .execute(
                json!({
                    "path": "Cargo.toml",
                    "check_online": false,
                    "format": "json"
                }),
                &ctx,
            )
            .await
            .unwrap();

        let val: Value = serde_json::from_str(&res).unwrap();
        assert!(val.get("manifests").is_some());
        let manifests = val.get("manifests").unwrap().as_array().unwrap();
        assert!(!manifests.is_empty());
    }

    #[test]
    fn test_formatters() {
        let mut dep = Dependency::new("tokio", "1.20.0", DependencyType::Normal, Ecosystem::Cargo, "Cargo.toml");
        dep.resolved_version = Some("1.20.0".to_string());
        dep.latest_version = Some("1.36.0".to_string());
        dep.status = OutdatedStatus::Outdated(BumpType::Minor);

        let manifest_res = ManifestScanResult {
            path: "Cargo.toml".to_string(),
            ecosystem: Ecosystem::Cargo,
            dependencies: vec![dep],
            total_count: 1,
            up_to_date_count: 0,
            outdated_count: 1,
            major_count: 0,
            minor_count: 1,
            patch_count: 0,
            vulnerable_count: 0,
            health_score: 96,
        };

        let report = AuditReport {
            workspace_root: ".".to_string(),
            manifests: vec![manifest_res],
            total_dependencies: 1,
            total_outdated: 1,
            total_major: 0,
            total_minor: 1,
            total_patch: 0,
            total_vulnerable: 0,
            overall_health_score: 96,
            health_rating: "A (Excellent)".to_string(),
            recommendations: vec!["1 minor update available.".to_string()],
        };

        let table_out = format_table(&report);
        assert!(table_out.contains("tokio"));
        assert!(table_out.contains("[MINOR]"));
        assert!(table_out.contains("96/100"));

        let md_out = format_markdown(&report);
        assert!(md_out.contains("# Dependency Audit Report"));
        assert!(md_out.contains("| **tokio** |"));

        let summary_out = format_summary(&report);
        assert!(summary_out.contains("Dependency Audit: 1 manifest(s)"));
        assert!(summary_out.contains("96/100"));
    }
}
