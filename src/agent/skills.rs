//! Extensible Domain Skill Loader and Prompt Injector for Fusion.
//!
//! Scans `.fusion/skills/` (workspace) and `~/.fusion/skills/` (global user)
//! for domain-specific `SKILL.md` instruction files and dynamically injects
//! relevant skill guidelines into system prompts based on trigger words,
//! active workspace files, and explicit user invocations (`@skill:<name>` or `skill://<name>`).
//!
//! ## Skill File Structure
//!
//! Skills can be defined in two formats:
//!
//! ### 1. Directory with `SKILL.md` (recommended):
//! ```text
//! .fusion/skills/
//! └── cloudflare-workers/
//!     ├── SKILL.md
//!     └── reference.json (optional auxiliary files)
//! ```
//!
//! ### 2. Single-file `.md`:
//! ```text
//! .fusion/skills/
//! └── cloudflare-workers.md
//! ```
//!
//! ### `SKILL.md` Content Format:
//! ```markdown
//! ---
//! name: cloudflare-workers
//! description: Best practices for Cloudflare Workers & Durable Objects
//! triggers: ["wrangler.jsonc", "wrangler.toml", "cloudflare", "worker", "durable object"]
//! tags: ["cloud", "serverless", "edge"]
//! always: false
//! enabled: true
//! version: "1.0.0"
//! author: "Fusion Team"
//! ---
//!
//! # Cloudflare Workers Engineering Guidelines
//!
//! 1. Always use standard Web standard APIs (fetch, Request, Response, Headers, ReadableStream).
//! 2. Keep cold start overhead minimal by avoiding heavy synchronous modules.
//! 3. Use `wrangler.jsonc` configuration with strict schema validation.
//! ```

use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};

use crate::config::Config;

/// Source location where a skill was discovered or registered.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "type", content = "path")]
pub enum SkillSource {
    /// Workspace-local project skill (e.g. `.fusion/skills/my-skill/SKILL.md`).
    Project(PathBuf),
    /// User-global configuration skill (e.g. `~/.fusion/skills/my-skill/SKILL.md`).
    Global(PathBuf),
    /// Programmatically registered or custom-injected skill.
    Custom(String),
    /// Built-in pre-packaged skill.
    Builtin,
}

impl fmt::Display for SkillSource {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Project(p) => write!(f, "project:{}", p.display()),
            Self::Global(p) => write!(f, "global:{}", p.display()),
            Self::Custom(name) => write!(f, "custom:{}", name),
            Self::Builtin => write!(f, "builtin"),
        }
    }
}

/// Metadata extracted from a `SKILL.md` frontmatter or header section.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SkillMetadata {
    /// Canonical skill identifier (e.g. `cloudflare-workers`, `docker-compose`).
    pub name: String,
    /// Short summary of what this skill teaches or enforces.
    pub description: String,
    /// Keywords, filenames, or glob patterns that trigger this skill.
    pub triggers: Vec<String>,
    /// Categorical tags (e.g. `backend`, `database`, `frontend`).
    pub tags: Vec<String>,
    /// Whether this skill is active.
    pub enabled: bool,
    /// If true, this skill is always included regardless of trigger matching.
    pub always_active: bool,
    /// Optional semantic version string (e.g. `1.2.0`).
    pub version: Option<String>,
    /// Optional author or creator info.
    pub author: Option<String>,
}

impl Default for SkillMetadata {
    fn default() -> Self {
        Self {
            name: String::new(),
            description: String::new(),
            triggers: Vec::new(),
            tags: Vec::new(),
            enabled: true,
            always_active: false,
            version: None,
            author: None,
        }
    }
}

/// A parsed, executable domain skill ready for prompt injection.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Skill {
    /// Metadata attributes.
    pub metadata: SkillMetadata,
    /// Markdown instructional guidelines to inject into system prompt.
    pub instructions: String,
    /// Origin of this skill.
    pub source: SkillSource,
    /// File path to `SKILL.md` if loaded from disk.
    pub path: Option<PathBuf>,
}

impl Skill {
    /// Creates a new Skill with explicit components.
    pub fn new(
        name: impl Into<String>,
        description: impl Into<String>,
        triggers: Vec<String>,
        instructions: impl Into<String>,
        source: SkillSource,
    ) -> Self {
        let name_str = name.into();
        Self {
            metadata: SkillMetadata {
                name: name_str.clone(),
                description: description.into(),
                triggers,
                tags: Vec::new(),
                enabled: true,
                always_active: false,
                version: None,
                author: None,
            },
            instructions: instructions.into(),
            source,
            path: None,
        }
    }

    /// Builder method to set tags.
    pub fn with_tags(mut self, tags: Vec<String>) -> Self {
        self.metadata.tags = tags;
        self
    }

    /// Builder method to set always-active status.
    pub fn with_always_active(mut self, always: bool) -> Self {
        self.metadata.always_active = always;
        self
    }

    /// Builder method to set enabled status.
    pub fn with_enabled(mut self, enabled: bool) -> Self {
        self.metadata.enabled = enabled;
        self
    }

    /// Builder method to set file path.
    pub fn with_path(mut self, path: impl Into<PathBuf>) -> Self {
        self.path = Some(path.into());
        self
    }

    /// Returns the skill name.
    pub fn name(&self) -> &str {
        &self.metadata.name
    }

    /// Returns the skill description.
    pub fn description(&self) -> &str {
        &self.metadata.description
    }

    /// Returns the skill trigger keywords/patterns.
    pub fn triggers(&self) -> &[String] {
        &self.metadata.triggers
    }

    /// Returns the skill categorical tags.
    pub fn tags(&self) -> &[String] {
        &self.metadata.tags
    }

    /// Returns whether the skill is currently enabled.
    pub fn is_enabled(&self) -> bool {
        self.metadata.enabled
    }

    /// Returns whether the skill is configured to always be active.
    pub fn is_always_active(&self) -> bool {
        self.metadata.always_active
    }

    /// Returns the markdown instructional content.
    pub fn instructions(&self) -> &str {
        &self.instructions
    }

    /// Parses a `SKILL.md` markdown string into a `Skill`.
    ///
    /// Supports YAML/frontmatter metadata delimited by `---` as well as
    /// markdown header sections (`# Skill: <Name>`, `## Triggers`, `## Instructions`).
    pub fn parse_markdown(
        content: &str,
        default_name: Option<&str>,
        source: SkillSource,
        path: Option<PathBuf>,
    ) -> Result<Self, String> {
        let trimmed = content.trim();
        if trimmed.is_empty() {
            return Err("Skill content is empty".to_string());
        }

        let mut metadata = SkillMetadata::default();
        if let Some(def) = default_name {
            metadata.name = def.to_string();
        }

        let mut instructions_text;

        // 1. Check for YAML frontmatter between `---`
        if trimmed.starts_with("---") {
            let after_first = &trimmed[3..];
            if let Some(end_idx) = after_first.find("\n---") {
                let frontmatter = &after_first[..end_idx];
                let rest_idx = end_idx + 4; // Skip `\n---`
                let rest = if rest_idx < after_first.len() {
                    &after_first[rest_idx..]
                } else {
                    ""
                };

                parse_frontmatter_lines(frontmatter, &mut metadata);
                instructions_text = rest.trim().to_string();
            } else {
                // Malformed frontmatter (no closing `---`)
                instructions_text = trimmed.to_string();
            }
        } else {
            // 2. Parse Markdown header structures
            let (parsed_meta, parsed_body) = parse_markdown_headers(trimmed, &metadata.name);
            metadata = parsed_meta;
            instructions_text = parsed_body;
        }

        // Fallback name if still empty
        if metadata.name.trim().is_empty() {
            metadata.name = default_name.unwrap_or("unnamed-skill").trim().to_string();
        }

        // If instructions are empty, fallback to entire trimmed content
        if instructions_text.trim().is_empty() {
            instructions_text = trimmed.to_string();
        }

        // Normalise name (lowercase, kebab-case friendly)
        metadata.name = metadata.name.trim().to_string();

        Ok(Skill {
            metadata,
            instructions: instructions_text,
            source,
            path,
        })
    }

    /// Formats the skill instructions into a clean markdown block for prompt injection.
    pub fn format_prompt_block(&self) -> String {
        let mut out = String::with_capacity(self.instructions.len() + 256);
        out.push_str(&format!("### Skill: {}\n", self.name()));
        if !self.description().is_empty() {
            out.push_str(&format!("*{}*\n\n", self.description()));
        }
        out.push_str(self.instructions.trim());
        out
    }
}

/// Helper to parse frontmatter key-values line by line.
fn parse_frontmatter_lines(frontmatter: &str, meta: &mut SkillMetadata) {
    let mut current_list_key: Option<String> = None;

    for raw_line in frontmatter.lines() {
        let line = raw_line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }

        // Handle multiline list item (`- item`)
        if line.starts_with('-') || line.starts_with('*') {
            let item = line.trim_start_matches(['-', '*']).trim();
            let unquoted = strip_quotes(item);
            if !unquoted.is_empty() {
                if let Some(key) = &current_list_key {
                    match key.as_str() {
                        "triggers" | "keywords" | "patterns" | "files" => {
                            meta.triggers.push(unquoted);
                        }
                        "tags" | "categories" => {
                            meta.tags.push(unquoted);
                        }
                        _ => {}
                    }
                }
            }
            continue;
        }

        if let Some((key_part, val_part)) = line.split_once(':') {
            let key = key_part.trim().to_ascii_lowercase();
            let raw_val = val_part.trim();

            match key.as_str() {
                "name" | "id" | "skill" => {
                    let v = strip_quotes(raw_val);
                    if !v.is_empty() {
                        meta.name = v;
                    }
                    current_list_key = None;
                }
                "description" | "desc" | "summary" => {
                    let v = strip_quotes(raw_val);
                    meta.description = v;
                    current_list_key = None;
                }
                "triggers" | "keywords" | "patterns" | "files" | "matches" => {
                    if raw_val.is_empty() {
                        current_list_key = Some(key);
                    } else {
                        // Array format `[a, b, c]` or comma-separated `a, b`
                        let items = parse_string_list(raw_val);
                        meta.triggers.extend(items);
                        current_list_key = None;
                    }
                }
                "tags" | "categories" => {
                    if raw_val.is_empty() {
                        current_list_key = Some(key);
                    } else {
                        let items = parse_string_list(raw_val);
                        meta.tags.extend(items);
                        current_list_key = None;
                    }
                }
                "always" | "always_active" | "global" | "auto_load" => {
                    let v = strip_quotes(raw_val).to_ascii_lowercase();
                    meta.always_active = v == "true" || v == "1" || v == "yes";
                    current_list_key = None;
                }
                "enabled" | "active" => {
                    let v = strip_quotes(raw_val).to_ascii_lowercase();
                    meta.enabled = v != "false" && v != "0" && v != "no";
                    current_list_key = None;
                }
                "version" => {
                    meta.version = Some(strip_quotes(raw_val));
                    current_list_key = None;
                }
                "author" => {
                    meta.author = Some(strip_quotes(raw_val));
                    current_list_key = None;
                }
                _ => {
                    current_list_key = None;
                }
            }
        }
    }
}

/// Helper to parse markdown headers into a `SkillMetadata` and body text.
fn parse_markdown_headers(content: &str, default_name: &str) -> (SkillMetadata, String) {
    let mut meta = SkillMetadata::default();
    meta.name = default_name.to_string();

    let mut body_lines = Vec::new();
    let mut in_triggers_section = false;
    let mut in_tags_section = false;
    let mut in_description_section = false;

    for line in content.lines() {
        let trimmed = line.trim();

        if trimmed.starts_with("# ") {
            let title = trimmed.trim_start_matches("# ").trim();
            // E.g. "# Skill: Docker" -> "Docker"
            let extracted = if let Some(stripped) = title.strip_prefix("Skill:") {
                stripped.trim()
            } else if let Some(stripped) = title.strip_prefix("skill:") {
                stripped.trim()
            } else {
                title
            };
            if !extracted.is_empty() && meta.name.is_empty() {
                meta.name = extracted.to_string();
            }
            in_triggers_section = false;
            in_tags_section = false;
            in_description_section = false;
            continue;
        }

        if trimmed.starts_with("## Triggers")
            || trimmed.starts_with("## Keywords")
            || trimmed.starts_with("## Patterns")
        {
            in_triggers_section = true;
            in_tags_section = false;
            in_description_section = false;
            continue;
        }

        if trimmed.starts_with("## Tags") || trimmed.starts_with("## Categories") {
            in_tags_section = true;
            in_triggers_section = false;
            in_description_section = false;
            continue;
        }

        if trimmed.starts_with("## Description") || trimmed.starts_with("## Summary") {
            in_description_section = true;
            in_triggers_section = false;
            in_tags_section = false;
            continue;
        }

        if trimmed.starts_with("## ") {
            // Any other section header (e.g. `## Instructions`, `## Guidelines`)
            in_triggers_section = false;
            in_tags_section = false;
            in_description_section = false;
            body_lines.push(line);
            continue;
        }

        if in_triggers_section {
            if trimmed.starts_with('-') || trimmed.starts_with('*') {
                let item = trimmed.trim_start_matches(['-', '*']).trim();
                let unquoted = strip_quotes(item);
                if !unquoted.is_empty() {
                    meta.triggers.push(unquoted);
                }
            } else if !trimmed.is_empty() {
                meta.triggers.extend(parse_string_list(trimmed));
            }
        } else if in_tags_section {
            if trimmed.starts_with('-') || trimmed.starts_with('*') {
                let item = trimmed.trim_start_matches(['-', '*']).trim();
                let unquoted = strip_quotes(item);
                if !unquoted.is_empty() {
                    meta.tags.push(unquoted);
                }
            } else if !trimmed.is_empty() {
                meta.tags.extend(parse_string_list(trimmed));
            }
        } else if in_description_section {
            if !trimmed.is_empty() {
                if !meta.description.is_empty() {
                    meta.description.push(' ');
                }
                meta.description
                    .push_str(trimmed.trim_start_matches('>').trim());
            }
        } else {
            // Capture blockquote right under top header as description if empty
            if meta.description.is_empty() && trimmed.starts_with('>') && body_lines.is_empty() {
                meta.description = trimmed.trim_start_matches('>').trim().to_string();
            } else {
                body_lines.push(line);
            }
        }
    }

    (meta, body_lines.join("\n").trim().to_string())
}

/// Helper to strip leading and trailing quotes or brackets.
fn strip_quotes(s: &str) -> String {
    let trimmed = s.trim();
    let unquoted = if (trimmed.starts_with('"') && trimmed.ends_with('"'))
        || (trimmed.starts_with('\'') && trimmed.ends_with('\''))
        || (trimmed.starts_with('`') && trimmed.ends_with('`'))
    {
        if trimmed.len() >= 2 {
            &trimmed[1..trimmed.len() - 1]
        } else {
            trimmed
        }
    } else {
        trimmed
    };
    unquoted.trim().to_string()
}

/// Helper to parse array `[a, b, c]` or comma-separated list `a, b, c`.
fn parse_string_list(val: &str) -> Vec<String> {
    let trimmed = val.trim();
    let inner = if trimmed.starts_with('[') && trimmed.ends_with(']') {
        &trimmed[1..trimmed.len() - 1]
    } else {
        trimmed
    };

    inner
        .split(',')
        .map(|item| strip_quotes(item.trim()))
        .filter(|item| !item.is_empty())
        .collect()
}

/// Result of matching a skill against a query or workspace context.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SkillMatch {
    /// Matched skill.
    pub skill: Skill,
    /// Relevance score between 0.0 and 1.0 (higher = more relevant).
    pub score: f32,
    /// Specific triggers that matched the query or workspace.
    pub matched_triggers: Vec<String>,
    /// Reason explaining why the skill was matched.
    pub reason: String,
}

/// Skill loader responsible for scanning `.fusion/skills` and `~/.fusion/skills`.
#[derive(Debug, Clone, Default)]
pub struct SkillLoader {
    /// Custom search directories in addition to default locations.
    extra_dirs: Vec<PathBuf>,
}

impl SkillLoader {
    /// Creates a new SkillLoader instance.
    pub fn new() -> Self {
        Self::default()
    }

    /// Appends an additional custom directory to scan for skills.
    pub fn with_extra_dir(mut self, dir: impl Into<PathBuf>) -> Self {
        self.extra_dirs.push(dir.into());
        self
    }

    /// Returns the standard workspace skill directory: `<workspace>/.fusion/skills`.
    pub fn project_skills_dir(workspace_root: &Path) -> PathBuf {
        workspace_root.join(".fusion").join("skills")
    }

    /// Returns the global user skill directory: `~/.fusion/skills`.
    pub fn global_skills_dir() -> PathBuf {
        Config::config_dir().join("skills")
    }

    /// Loads a single skill from a file path.
    pub fn load_file(path: &Path, source: SkillSource) -> Result<Skill, String> {
        if !path.exists() {
            return Err(format!("Skill file does not exist: {}", path.display()));
        }

        let content = fs::read_to_string(path)
            .map_err(|e| format!("Failed to read skill file {}: {}", path.display(), e))?;

        // Determine fallback name from parent dir if named SKILL.md or from file stem
        let default_name = if let Some(file_name) = path.file_name().and_then(|n| n.to_str()) {
            if file_name.eq_ignore_ascii_case("SKILL.md") || file_name.eq_ignore_ascii_case("SKILL")
            {
                path.parent()
                    .and_then(|p| p.file_name())
                    .and_then(|n| n.to_str())
            } else {
                path.file_stem().and_then(|s| s.to_str())
            }
        } else {
            None
        };

        Skill::parse_markdown(&content, default_name, source, Some(path.to_path_buf()))
    }

    /// Scans a directory for skills.
    ///
    /// Discovers:
    /// 1. `<dir>/<skill_name>/SKILL.md` (case-insensitive `SKILL.md`, `skill.md`, `Skill.md`)
    /// 2. `<dir>/<skill_name>.md`
    pub fn scan_directory(
        &self,
        dir: &Path,
        source_kind: impl Fn(&Path) -> SkillSource,
    ) -> Vec<Skill> {
        let mut skills = Vec::new();
        if !dir.exists() || !dir.is_dir() {
            return skills;
        }

        let entries = match fs::read_dir(dir) {
            Ok(entries) => entries,
            Err(err) => {
                tracing::debug!("Failed to read skills directory {}: {}", dir.display(), err);
                return skills;
            }
        };

        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                // Check for `SKILL.md` or `skill.md` inside subdirectory
                let skill_candidates = ["SKILL.md", "skill.md", "Skill.md", "SKILL", "skill"];
                for candidate in skill_candidates {
                    let candidate_path = path.join(candidate);
                    if candidate_path.is_file() {
                        let source = source_kind(&candidate_path);
                        match Self::load_file(&candidate_path, source) {
                            Ok(skill) => {
                                skills.push(skill);
                                break;
                            }
                            Err(err) => {
                                tracing::warn!(
                                    "Failed to parse skill at {}: {}",
                                    candidate_path.display(),
                                    err
                                );
                            }
                        }
                    }
                }
            } else if path.is_file() {
                // Single-file skill (e.g. `docker.md`)
                if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
                    if ext.eq_ignore_ascii_case("md") || ext.eq_ignore_ascii_case("markdown") {
                        let source = source_kind(&path);
                        match Self::load_file(&path, source) {
                            Ok(skill) => skills.push(skill),
                            Err(err) => {
                                tracing::warn!(
                                    "Failed to parse skill file {}: {}",
                                    path.display(),
                                    err
                                );
                            }
                        }
                    }
                }
            }
        }

        // Sort skills alphabetically by name for deterministic ordering
        skills.sort_by(|a, b| a.name().cmp(b.name()));
        skills
    }

    /// Scans workspace-local skills (`<workspace_root>/.fusion/skills/`).
    pub fn scan_project(&self, workspace_root: &Path) -> Vec<Skill> {
        let dir = Self::project_skills_dir(workspace_root);
        self.scan_directory(&dir, |p| SkillSource::Project(p.to_path_buf()))
    }

    /// Scans global user skills (`~/.fusion/skills/`).
    pub fn scan_global(&self) -> Vec<Skill> {
        let dir = Self::global_skills_dir();
        self.scan_directory(&dir, |p| SkillSource::Global(p.to_path_buf()))
    }

    /// Scans both global and workspace-local skills, with workspace skills
    /// taking precedence over global skills with the same name.
    pub fn scan_all(&self, workspace_root: Option<&Path>) -> SkillRegistry {
        let mut registry = SkillRegistry::new();

        // 1. Load global skills (~/.fusion/skills/ and ~/.claude/skills/)
        for skill in self.scan_global() {
            registry.register(skill);
        }
        let global_claude = dirs::home_dir()
            .unwrap_or_default()
            .join(".claude")
            .join("skills");
        for skill in self.scan_directory(&global_claude, |p| SkillSource::Global(p.to_path_buf())) {
            registry.register(skill);
        }

        // 2. Load custom extra directories
        for extra in &self.extra_dirs {
            for skill in
                self.scan_directory(extra, |p| SkillSource::Custom(p.display().to_string()))
            {
                registry.register(skill);
            }
        }
        // 3. Load workspace project skills (overrides global skills with same name)
        if let Some(root) = workspace_root {
            for skill in self.scan_project(root) {
                registry.register(skill);
            }
            // 4. Also scan .claude/skills/ for compatibility with Claude Code skills
            let claude_dir = root.join(".claude").join("skills");
            for skill in self.scan_directory(&claude_dir, |p| SkillSource::Project(p.to_path_buf()))
            {
                registry.register(skill);
            }
        }

        registry
    }
}

/// Registry holding all active, disabled, and discovered skills.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SkillRegistry {
    skills: HashMap<String, Skill>,
}

impl SkillRegistry {
    /// Creates an empty SkillRegistry.
    pub fn new() -> Self {
        Self {
            skills: HashMap::new(),
        }
    }

    /// Convenience constructor to scan and populate registry from default global
    /// and optional workspace directories.
    pub fn scan_default(workspace_root: Option<&Path>) -> Self {
        let loader = SkillLoader::new();
        loader.scan_all(workspace_root)
    }

    /// Registers a skill in the registry.
    ///
    /// If a skill with the same name already exists, it is replaced.
    pub fn register(&mut self, skill: Skill) {
        self.skills.insert(skill.name().to_string(), skill);
    }

    /// Unregisters and removes a skill by name.
    pub fn unregister(&mut self, name: &str) -> Option<Skill> {
        self.skills.remove(name)
    }

    /// Retrieves a skill by name.
    pub fn get(&self, name: &str) -> Option<&Skill> {
        self.skills.get(name)
    }

    /// Retrieves a mutable reference to a skill by name.
    pub fn get_mut(&mut self, name: &str) -> Option<&mut Skill> {
        self.skills.get_mut(name)
    }

    /// Returns a list of references to all registered skills, sorted by name.
    pub fn list(&self) -> Vec<&Skill> {
        let mut list: Vec<&Skill> = self.skills.values().collect();
        list.sort_by(|a, b| a.name().cmp(b.name()));
        list
    }

    /// Returns a list of all enabled skills.
    pub fn list_enabled(&self) -> Vec<&Skill> {
        let mut list: Vec<&Skill> = self.skills.values().filter(|s| s.is_enabled()).collect();
        list.sort_by(|a, b| a.name().cmp(b.name()));
        list
    }

    /// Returns the number of registered skills.
    pub fn len(&self) -> usize {
        self.skills.len()
    }

    /// Returns true if no skills are registered.
    pub fn is_empty(&self) -> bool {
        self.skills.is_empty()
    }

    /// Enables or disables a skill by name. Returns true if the skill was found.
    pub fn set_enabled(&mut self, name: &str, enabled: bool) -> bool {
        if let Some(skill) = self.skills.get_mut(name) {
            skill.metadata.enabled = enabled;
            true
        } else {
            false
        }
    }

    /// Finds relevant skills based on user prompt text, active files, and workspace context.
    ///
    /// Matching criteria:
    /// - Explicit skill invocation: `@skill:<name>`, `@<name>`, or `skill://<name>` (Score: 1.0)
    /// - Always active skills: `always_active == true` (Score: 0.95)
    /// - Trigger keyword / file match in user prompt (Score: 0.8)
    /// - File existence in workspace matching triggers (Score: 0.6)
    /// - Category tag match (Score: 0.5)
    ///
    /// Results are filtered to enabled skills and sorted by descending relevance score.
    pub fn find_relevant(&self, query: &str, workspace_root: Option<&Path>) -> Vec<SkillMatch> {
        let mut matches = Vec::new();
        let query_lower = query.to_lowercase();
        let query_tokens: HashSet<String> = tokenize_words(&query_lower);

        for skill in self.skills.values() {
            if !skill.is_enabled() {
                continue;
            }

            let mut score: f32 = 0.0;
            let mut matched_triggers = Vec::new();
            let mut reason = String::new();

            let skill_name_lower = skill.name().to_lowercase();

            // 1. Check for explicit invocation (`@skill:name`, `skill://name`, `@name`)
            let explicit_patterns = [
                format!("@skill:{}", skill_name_lower),
                format!("skill://{}", skill_name_lower),
                format!("@{}", skill_name_lower),
            ];

            let is_explicit = explicit_patterns
                .iter()
                .any(|pat| query_lower.contains(pat));
            if is_explicit {
                score = 1.0;
                matched_triggers.push(format!("explicit:{}", skill.name()));
                reason = "Explicitly invoked in user prompt".to_string();
            } else if skill.is_always_active() {
                // 2. Always active skills
                score = 0.95;
                matched_triggers.push("always_active".to_string());
                reason = "Configured as always active".to_string();
            } else {
                // 3. Exact skill name mentioned in prompt
                if query_tokens.contains(&skill_name_lower)
                    || query_lower.contains(&skill_name_lower)
                {
                    score = score.max(0.85);
                    matched_triggers.push(skill.name().to_string());
                    reason = format!("Skill name '{}' mentioned in prompt", skill.name());
                }

                // 4. Trigger keywords matching
                for trigger in skill.triggers() {
                    let trigger_lower = trigger.trim().to_lowercase();
                    if trigger_lower.is_empty() {
                        continue;
                    }

                    // Check if trigger is a single word or phrase in the query
                    if trigger_lower.contains(' ')
                        || trigger_lower.contains('.')
                        || trigger_lower.contains('/')
                    {
                        if query_lower.contains(&trigger_lower) {
                            score = score.max(0.80);
                            matched_triggers.push(trigger.clone());
                            if reason.is_empty() {
                                reason = format!("Trigger phrase '{}' matched in prompt", trigger);
                            }
                        }
                    } else if query_tokens.contains(&trigger_lower)
                        || query_lower.contains(&trigger_lower)
                    {
                        score = score.max(0.75);
                        matched_triggers.push(trigger.clone());
                        if reason.is_empty() {
                            reason = format!("Trigger keyword '{}' matched in prompt", trigger);
                        }
                    }

                    // Check workspace file presence for file-like triggers (e.g. `wrangler.toml`, `Dockerfile`, `Cargo.toml`)
                    if let Some(root) = workspace_root {
                        if is_file_like_trigger(&trigger_lower) {
                            let file_candidate = root.join(&trigger_lower);
                            if file_candidate.exists() {
                                score = score.max(0.65);
                                matched_triggers.push(format!("file:{}", trigger));
                                if reason.is_empty() {
                                    reason = format!("Workspace file '{}' exists", trigger);
                                }
                            }
                        }
                    }
                }

                // 5. Categorical tags matching
                for tag in skill.tags() {
                    let tag_lower = tag.trim().to_lowercase();
                    if !tag_lower.is_empty() && query_tokens.contains(&tag_lower) {
                        score = score.max(0.50);
                        matched_triggers.push(format!("tag:{}", tag));
                        if reason.is_empty() {
                            reason = format!("Category tag '{}' matched in prompt", tag);
                        }
                    }
                }
            }

            // If score is above minimal threshold, record match
            if score >= 0.40 {
                matches.push(SkillMatch {
                    skill: skill.clone(),
                    score,
                    matched_triggers,
                    reason,
                });
            }
        }

        // Sort descending by score, then alphabetically by name
        matches.sort_by(|a, b| {
            b.score
                .partial_cmp(&a.score)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| a.skill.name().cmp(b.skill.name()))
        });

        matches
    }

    /// Formats all relevant skills matching the query into a Markdown addendum
    /// suitable for direct system prompt injection.
    ///
    /// Limits injection to top `max_skills` (defaults to 4 if not specified) to
    /// prevent context window bloat.
    pub fn inject_relevant_skills(
        &self,
        query: &str,
        workspace_root: Option<&Path>,
        max_skills: Option<usize>,
    ) -> Option<String> {
        let matches = self.find_relevant(query, workspace_root);
        if matches.is_empty() {
            return None;
        }

        let limit = max_skills.unwrap_or(4);
        let selected_matches: Vec<&SkillMatch> = matches.iter().take(limit).collect();

        let mut block = String::with_capacity(2048);
        block.push_str("Active Domain Skills:\n");
        block.push_str("Follow these specialized guidelines for tasks touching their domains:\n\n");

        for (idx, m) in selected_matches.iter().enumerate() {
            if idx > 0 {
                block.push_str("\n\n---\n\n");
            }
            block.push_str(&m.skill.format_prompt_block());
        }

        Some(block)
    }

    /// Formats a complete markdown table summarizing all available skills.
    pub fn format_catalog_markdown(&self) -> String {
        let skills = self.list();
        if skills.is_empty() {
            return "No skills currently registered. Place `SKILL.md` files in `.fusion/skills/` or `~/.fusion/skills/`.".to_string();
        }

        let mut md = String::with_capacity(1024);
        md.push_str("| Status | Skill | Source | Triggers | Description |\n");
        md.push_str("|:------:|:------|:-------|:---------|:------------|\n");

        for skill in skills {
            let status = if skill.is_enabled() {
                "🟢 Active"
            } else {
                "⚪ Disabled"
            };
            let source_str = match &skill.source {
                SkillSource::Project(_) => "Project",
                SkillSource::Global(_) => "Global",
                SkillSource::Custom(_) => "Custom",
                SkillSource::Builtin => "Builtin",
            };
            let triggers_str = if skill.triggers().is_empty() {
                "-".to_string()
            } else {
                skill.triggers().join(", ")
            };
            let desc_str = if skill.description().is_empty() {
                "-"
            } else {
                skill.description()
            };

            md.push_str(&format!(
                "| {} | `{}` | {} | {} | {} |\n",
                status,
                skill.name(),
                source_str,
                triggers_str,
                desc_str
            ));
        }

        md
    }
}

/// Helper to tokenize input text into clean lowercase words.
fn tokenize_words(text: &str) -> HashSet<String> {
    text.split(|c: char| !c.is_alphanumeric() && c != '-' && c != '_' && c != '.')
        .map(|w| w.trim().to_lowercase())
        .filter(|w| w.len() >= 2)
        .collect()
}

/// Helper to check if a trigger string resembles a filename or extension.
fn is_file_like_trigger(trigger: &str) -> bool {
    trigger.contains('.')
        || trigger.eq_ignore_ascii_case("dockerfile")
        || trigger.eq_ignore_ascii_case("makefile")
        || trigger.eq_ignore_ascii_case("gemfile")
        || trigger.eq_ignore_ascii_case("procfile")
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn test_parse_frontmatter_yaml() {
        let md = r#"---
name: cloudflare-workers
description: Guidelines for Cloudflare Workers & Durable Objects
triggers: ["wrangler.jsonc", "wrangler.toml", "cloudflare", "worker"]
tags: ["cloud", "serverless"]
always: false
enabled: true
version: "1.0.0"
author: "Fusion Team"
---

# Cloudflare Guidelines
1. Use Web Standard APIs.
2. Avoid heavy cold-start dependencies.
"#;

        let skill = Skill::parse_markdown(md, None, SkillSource::Builtin, None).unwrap();
        assert_eq!(skill.name(), "cloudflare-workers");
        assert_eq!(
            skill.description(),
            "Guidelines for Cloudflare Workers & Durable Objects"
        );
        assert_eq!(
            skill.triggers(),
            &["wrangler.jsonc", "wrangler.toml", "cloudflare", "worker"]
        );
        assert_eq!(skill.tags(), &["cloud", "serverless"]);
        assert!(skill.is_enabled());
        assert!(!skill.is_always_active());
        assert_eq!(skill.metadata.version.as_deref(), Some("1.0.0"));
        assert_eq!(skill.metadata.author.as_deref(), Some("Fusion Team"));
        assert!(skill.instructions().contains("Use Web Standard APIs."));
    }

    #[test]
    fn test_parse_frontmatter_multiline_list() {
        let md = r#"---
name: docker-expert
description: Best practices for Dockerfiles
triggers:
  - Dockerfile
  - docker-compose.yml
  - compose.yaml
  - container
tags:
  - devops
  - containerization
---
Always leverage multi-stage builds.
"#;

        let skill = Skill::parse_markdown(md, None, SkillSource::Builtin, None).unwrap();
        assert_eq!(skill.name(), "docker-expert");
        assert_eq!(
            skill.triggers(),
            &[
                "Dockerfile",
                "docker-compose.yml",
                "compose.yaml",
                "container"
            ]
        );
        assert_eq!(skill.tags(), &["devops", "containerization"]);
        assert!(skill.instructions().contains("multi-stage builds"));
    }

    #[test]
    fn test_parse_markdown_header_structure() {
        let md = r#"# Skill: React Optimization
> Performance patterns for modern React 19 apps.

## Triggers
- react
- useMemo
- useCallback
- rerender

## Guidelines
1. Don't prematurely memoize primitive calculations.
2. Use React compiler when possible.
"#;

        let skill = Skill::parse_markdown(md, None, SkillSource::Builtin, None).unwrap();
        assert_eq!(skill.name(), "React Optimization");
        assert_eq!(
            skill.description(),
            "Performance patterns for modern React 19 apps."
        );
        assert_eq!(
            skill.triggers(),
            &["react", "useMemo", "useCallback", "rerender"]
        );
        assert!(skill.instructions().contains("Don't prematurely memoize"));
    }

    #[test]
    fn test_parse_fallback_plain_markdown() {
        let md = "Just simple instructions without frontmatter or special headers.";
        let skill =
            Skill::parse_markdown(md, Some("my-fallback-skill"), SkillSource::Builtin, None)
                .unwrap();
        assert_eq!(skill.name(), "my-fallback-skill");
        assert_eq!(skill.instructions(), md);
        assert!(skill.is_enabled());
    }

    #[test]
    fn test_skill_scanning_and_overriding() {
        let temp_global = tempdir().unwrap();
        let temp_project = tempdir().unwrap();

        // 1. Create global skill in temp_global/skills/rust-perf/SKILL.md
        let global_skills_dir = temp_global.path().join("skills");
        let global_rust_dir = global_skills_dir.join("rust-perf");
        fs::create_dir_all(&global_rust_dir).unwrap();
        fs::write(
            global_rust_dir.join("SKILL.md"),
            r#"---
name: rust-perf
description: Global rust performance guidelines
triggers: ["rust", "benchmark", "criterion"]
---
Global instructions: Use Criterion.
"#,
        )
        .unwrap();

        // 2. Create project skill overriding rust-perf in temp_project/.fusion/skills/rust-perf/SKILL.md
        let proj_skills_dir = temp_project.path().join(".fusion").join("skills");
        let proj_rust_dir = proj_skills_dir.join("rust-perf");
        fs::create_dir_all(&proj_rust_dir).unwrap();
        fs::write(
            proj_rust_dir.join("SKILL.md"),
            r#"---
name: rust-perf
description: Project specific rust performance guidelines
triggers: ["rust", "perf", "simd"]
---
Project instructions: Use SIMD where appropriate.
"#,
        )
        .unwrap();

        // 3. Create a unique project skill in temp_project/.fusion/skills/sql-queries.md
        fs::write(
            proj_skills_dir.join("sql-queries.md"),
            r#"---
name: sql-queries
description: Database query optimization
triggers: ["sql", "postgres", "query"]
---
Always use prepared statements.
"#,
        )
        .unwrap();

        let loader = SkillLoader::new();

        let global_skills =
            loader.scan_directory(&global_skills_dir, |p| SkillSource::Global(p.to_path_buf()));
        assert_eq!(global_skills.len(), 1);
        assert_eq!(global_skills[0].name(), "rust-perf");
        assert!(global_skills[0]
            .instructions()
            .contains("Global instructions"));

        let proj_skills =
            loader.scan_directory(&proj_skills_dir, |p| SkillSource::Project(p.to_path_buf()));
        assert_eq!(proj_skills.len(), 2);

        // Test registry overriding
        let mut registry = SkillRegistry::new();
        for s in global_skills {
            registry.register(s);
        }
        for s in proj_skills {
            registry.register(s);
        }

        assert_eq!(registry.len(), 2);
        let overridden_rust = registry.get("rust-perf").unwrap();
        assert!(overridden_rust
            .instructions()
            .contains("Project instructions: Use SIMD"));
        assert!(matches!(overridden_rust.source, SkillSource::Project(_)));

        let sql = registry.get("sql-queries").unwrap();
        assert_eq!(sql.name(), "sql-queries");
    }

    #[test]
    fn test_find_relevant_skills_and_injection() {
        let mut registry = SkillRegistry::new();

        let cf_skill = Skill::new(
            "cloudflare-workers",
            "Cloudflare Workers & Durable Objects",
            vec![
                "wrangler.toml".to_string(),
                "cloudflare".to_string(),
                "worker".to_string(),
            ],
            "1. Use Web Standard APIs.\n2. Avoid unhandled rejections.",
            SkillSource::Builtin,
        );

        let docker_skill = Skill::new(
            "docker",
            "Docker containerization guidelines",
            vec![
                "dockerfile".to_string(),
                "docker".to_string(),
                "container".to_string(),
            ],
            "1. Use multi-stage builds.",
            SkillSource::Builtin,
        );

        let always_skill = Skill::new(
            "code-style",
            "General clean code standards",
            vec!["style".to_string()],
            "Write readable, self-documenting code.",
            SkillSource::Builtin,
        )
        .with_always_active(true);

        registry.register(cf_skill);
        registry.register(docker_skill);
        registry.register(always_skill);

        // 1. Query matching cloudflare trigger
        let matches = registry.find_relevant("How do I deploy this cloudflare worker?", None);
        assert!(!matches.is_empty());
        let matched_names: Vec<&str> = matches.iter().map(|m| m.skill.name()).collect();
        assert!(matched_names.contains(&"cloudflare-workers"));
        assert!(matched_names.contains(&"code-style")); // because always active

        // 2. Query matching explicit @docker or @skill:docker
        let explicit_matches =
            registry.find_relevant("Please check @skill:docker configuration", None);
        assert_eq!(explicit_matches[0].skill.name(), "docker");
        assert_eq!(explicit_matches[0].score, 1.0);

        // 3. Inject relevant skills prompt block
        let injection =
            registry.inject_relevant_skills("Need help with cloudflare worker", None, Some(2));
        assert!(injection.is_some());
        let text = injection.unwrap();
        assert!(text.contains("Active Domain Skills:"));
        assert!(text.contains("### Skill: cloudflare-workers"));
        assert!(text.contains("1. Use Web Standard APIs."));
    }

    #[test]
    fn test_enable_disable_skill() {
        let mut registry = SkillRegistry::new();
        let skill = Skill::new(
            "test-skill",
            "Test description",
            vec!["test".to_string()],
            "Test instructions",
            SkillSource::Builtin,
        );
        registry.register(skill);

        assert!(registry.get("test-skill").unwrap().is_enabled());
        assert_eq!(registry.list_enabled().len(), 1);

        registry.set_enabled("test-skill", false);
        assert!(!registry.get("test-skill").unwrap().is_enabled());
        assert_eq!(registry.list_enabled().len(), 0);

        let matches = registry.find_relevant("test query", None);
        assert!(matches.is_empty());
    }

    #[test]
    fn test_format_catalog_markdown() {
        let mut registry = SkillRegistry::new();
        registry.register(Skill::new(
            "rust-async",
            "Tokio async best practices",
            vec!["tokio".to_string(), "async".to_string()],
            "Avoid blocking calls in async context.",
            SkillSource::Builtin,
        ));

        let table = registry.format_catalog_markdown();
        assert!(table
            .contains("| `rust-async` | Builtin | tokio, async | Tokio async best practices |"));
    }
}
